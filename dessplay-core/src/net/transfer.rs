//! File-transfer wire types: peer messages, the relay envelope, and the
//! chunk bitfield (docs/network-design.md, File Transfer).
//!
//! All peer traffic is **relayed** through the server: a downloader
//! addresses a logical peer by [`PeerId`], the server forwards the
//! opaque inner bytes to that peer's relay stream. The server never
//! decodes a [`PeerMessage`] — it only reads the [`RelayEnvelope`]
//! around it — so the transfer protocol is peer-to-peer in semantics and
//! relay-transported in mechanics.

use serde::{Deserialize, Serialize};

use crate::hash::Ed2kBlockHash;
use crate::types::{Ed2kHash, UserId};

/// How peers are addressed for relay. Users are identified by username
/// (the threat model accepts that names are not cryptographic); the
/// server forwards to the live connection registered under this name.
pub type PeerId = UserId;

/// Transfer chunks per ed2k block. 38 divides the 9,728,000-byte ed2k
/// block exactly, so a chunk never straddles a block boundary — block
/// verification maps to a contiguous group of whole chunks, with no
/// shared-chunk bookkeeping.
pub const CHUNKS_PER_BLOCK: u64 = 38;

/// A file's chunk size: 256,000 bytes (250 KiB), chosen as
/// `ED2K_BLOCK_SIZE / 38` so chunks align to ed2k block boundaries (the
/// block size is fixed by the AniDB-compatible root hash; the chunk
/// size is ours). The last chunk of a file may be smaller. Chunk count
/// is derived from the file's `size_bytes`.
pub const CHUNK_SIZE: u64 = crate::hash::ED2K_BLOCK_SIZE / CHUNKS_PER_BLOCK;

// The alignment the rest of transfer relies on.
const _: () = assert!(crate::hash::ED2K_BLOCK_SIZE.is_multiple_of(CHUNK_SIZE));

/// Number of 256 KiB chunks covering `size_bytes` (the last is short).
pub fn chunk_count(size_bytes: u64) -> u32 {
    // size 0 still has no chunks; every nonzero size has at least one.
    size_bytes.div_ceil(CHUNK_SIZE) as u32
}

/// The byte range a chunk covers within a file of `size_bytes`
/// (`[start, end)`; the last chunk is clamped to the file size).
pub fn chunk_range(index: u32, size_bytes: u64) -> std::ops::Range<u64> {
    let start = (index as u64) * CHUNK_SIZE;
    let end = (start + CHUNK_SIZE).min(size_bytes);
    start..end
}

/// A compact set of "which chunks do I have" bits (1 = present),
/// LSB-first within each byte. Replaces a `bitvec` dependency — the only
/// operations transfer needs are set/get/count and whole-field merge.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bitfield {
    bits: Vec<u8>,
    /// Logical length in bits (the trailing bits of the last byte are
    /// padding and must stay zero).
    len: u32,
}

impl std::fmt::Debug for Bitfield {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bitfield({}/{} set)", self.count_ones(), self.len)
    }
}

impl Bitfield {
    /// An all-zero bitfield for `len` chunks.
    pub fn new(len: u32) -> Self {
        let bytes = (len as usize).div_ceil(8);
        Bitfield {
            bits: vec![0; bytes],
            len,
        }
    }

    /// Logical length in bits (chunk count).
    pub fn len(&self) -> u32 {
        self.len
    }

    /// Whether the bitfield covers zero chunks.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Is chunk `index` present? Out-of-range is `false`.
    pub fn get(&self, index: u32) -> bool {
        if index >= self.len {
            return false;
        }
        let (byte, bit) = (index as usize / 8, index % 8);
        self.bits[byte] & (1 << bit) != 0
    }

    /// Mark chunk `index` present. Out-of-range is ignored.
    pub fn set(&mut self, index: u32) {
        if index >= self.len {
            return;
        }
        let (byte, bit) = (index as usize / 8, index % 8);
        self.bits[byte] |= 1 << bit;
    }

    /// Mark chunk `index` absent. Out-of-range is ignored.
    pub fn unset(&mut self, index: u32) {
        if index >= self.len {
            return;
        }
        let (byte, bit) = (index as usize / 8, index % 8);
        self.bits[byte] &= !(1 << bit);
    }

    /// How many chunks are present.
    pub fn count_ones(&self) -> u32 {
        self.bits.iter().map(|b| b.count_ones()).sum()
    }

    /// Are all `len` chunks present?
    pub fn is_complete(&self) -> bool {
        self.count_ones() == self.len
    }

    /// Reject a bitfield whose declared length or padding bits don't
    /// match `expected_len` — a peer must not be able to claim chunks
    /// outside the file or smuggle bits into the padding.
    pub fn is_valid_for(&self, expected_len: u32) -> bool {
        if self.len != expected_len || self.bits.len() != (expected_len as usize).div_ceil(8) {
            return false;
        }
        // Trailing padding bits in the final byte must be zero.
        let used = expected_len % 8;
        if used != 0
            && let Some(last) = self.bits.last()
        {
            let mask = !((1u8 << used) - 1);
            if last & mask != 0 {
                return false;
            }
        }
        true
    }
}

/// Messages exchanged between peers (always wrapped in a
/// [`RelayEnvelope`] on the wire). The server forwards these without
/// decoding them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PeerMessage {
    /// "Here is which chunks of `file` I have." Sent when a peer begins
    /// serving a file (complete bitfield) and as it completes more.
    FileAvailability {
        /// The file.
        file: Ed2kHash,
        /// Chunks the sender holds.
        bitfield: Bitfield,
    },
    /// "Send me the ed2k per-block hashes for `file`" (for verification).
    BlockHashRequest {
        /// The file.
        file: Ed2kHash,
    },
    /// The ed2k per-block (9,728,000-byte) MD4 hashes for `file`. The
    /// recipient validates them against the file's ed2k root (the
    /// playlist key) before trusting them.
    BlockHashes {
        /// The file.
        file: Ed2kHash,
        /// One MD4 per ed2k block, in order.
        hashes: Vec<Ed2kBlockHash>,
    },
    /// "Send me these chunks of `file`," in preferred order.
    ChunkRequest {
        /// The file.
        file: Ed2kHash,
        /// Chunk indices, most-wanted first.
        chunks: Vec<u32>,
    },
    /// One chunk's bytes (up to [`CHUNK_SIZE`]).
    ChunkData {
        /// The file.
        file: Ed2kHash,
        /// Chunk index.
        index: u32,
        /// The chunk's bytes.
        data: Vec<u8>,
    },
    /// "Drop these outstanding requests for `file`." Sent when a chunk
    /// arrived from another source (endgame) or a source is being
    /// dropped, so the uploader doesn't waste bandwidth on it.
    Cancel {
        /// The file.
        file: Ed2kHash,
        /// Chunk indices to cancel.
        chunks: Vec<u32>,
    },
}

/// The relay wrapper. Carried as length-prefixed frames on a dedicated
/// QUIC stream (separate from the control stream, so bulk transfer
/// never head-of-line-blocks state sync — QUIC isolates streams). The
/// `message` bytes are a postcard-encoded [`PeerMessage`], opaque to the
/// server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelayEnvelope {
    /// Client -> server: the first frame a client writes on its relay
    /// stream, sent immediately on open. QUIC reveals a bidirectional
    /// stream to the peer only when bytes are first written, so a peer
    /// that only ever *receives* (an idle source/seeder) would never
    /// register its relay stream on the server. `Hello` forces that
    /// registration; the server reads and ignores it.
    Hello,
    /// Client -> server: forward `message` to peer `to`.
    Forward {
        /// The destination peer.
        to: PeerId,
        /// Postcard-encoded [`PeerMessage`].
        message: Vec<u8>,
    },
    /// Server -> client: `message` forwarded from peer `from`.
    Forwarded {
        /// The originating peer.
        from: PeerId,
        /// Postcard-encoded [`PeerMessage`].
        message: Vec<u8>,
    },
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn chunk_math() {
        assert_eq!(chunk_count(0), 0);
        assert_eq!(chunk_count(1), 1);
        assert_eq!(chunk_count(CHUNK_SIZE), 1);
        assert_eq!(chunk_count(CHUNK_SIZE + 1), 2);
        // A typical ~1.4 GB file is ~5500 chunks of 250 KiB.
        assert_eq!(chunk_count(1_400_000_000), 5469);
        // Chunks divide the ed2k block exactly (no straddling).
        assert_eq!(crate::hash::ED2K_BLOCK_SIZE / CHUNK_SIZE, CHUNKS_PER_BLOCK);
        assert_eq!(CHUNK_SIZE, 256_000);

        assert_eq!(chunk_range(0, CHUNK_SIZE + 10), 0..CHUNK_SIZE);
        // The last chunk is clamped to the file size.
        assert_eq!(chunk_range(1, CHUNK_SIZE + 10), CHUNK_SIZE..CHUNK_SIZE + 10);
    }

    #[test]
    fn bitfield_set_get_count() {
        let mut bf = Bitfield::new(10);
        assert_eq!(bf.len(), 10);
        assert!(!bf.is_complete());
        assert_eq!(bf.count_ones(), 0);

        bf.set(0);
        bf.set(9);
        bf.set(100); // out of range: ignored
        assert!(bf.get(0));
        assert!(bf.get(9));
        assert!(!bf.get(5));
        assert!(!bf.get(100));
        assert_eq!(bf.count_ones(), 2);

        for i in 0..10 {
            bf.set(i);
        }
        assert!(bf.is_complete());
    }

    #[test]
    fn bitfield_validation_rejects_tampering() {
        let bf = Bitfield::new(10);
        assert!(bf.is_valid_for(10));
        assert!(!bf.is_valid_for(11)); // wrong length

        // Padding bits set: invalid.
        let mut tampered = Bitfield::new(10);
        tampered.bits[1] = 0b1111_1100; // bits 10..16 are padding
        assert!(!tampered.is_valid_for(10));

        // A byte-aligned length has no padding to check.
        let aligned = Bitfield::new(16);
        assert!(aligned.is_valid_for(16));
    }

    #[test]
    fn bitfield_round_trips_through_postcard() {
        let mut bf = Bitfield::new(20);
        bf.set(3);
        bf.set(19);
        let bytes = crate::wire::encode(&bf).unwrap();
        let back: Bitfield = crate::wire::decode(&bytes).unwrap();
        assert_eq!(bf, back);
    }

    #[test]
    fn peer_message_round_trips() {
        let msg = PeerMessage::ChunkData {
            file: Ed2kHash([1; 16]),
            index: 7,
            data: vec![9; 1000],
        };
        let bytes = crate::wire::encode(&msg).unwrap();
        assert_eq!(crate::wire::decode::<PeerMessage>(&bytes).unwrap(), msg);
    }
}
