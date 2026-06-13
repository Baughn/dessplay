//! ed2k hashing: the root hash (DessPlay's `FileId`) plus per-block MD4
//! hashes kept for transfer verification.
//!
//! We compute the eMule/AniDB ("red") variant: a file whose size is an
//! exact non-zero multiple of the block size contributes a trailing
//! empty-block hash to the root computation. AniDB's FILE lookups expect
//! this variant; it differs from the "blue" variant only for such files.
//!
//! Block hashes cover the *real* blocks only (the trailing empty block is
//! a root-computation artifact, not a transfer chunk).

use std::io::{self, Read};

use digest::Digest;
use md4::Md4;
use serde::{Deserialize, Serialize};

use crate::types::Ed2kHash;

/// ed2k block size: 9500 KiB.
pub const ED2K_BLOCK_SIZE: u64 = 9_728_000;

/// MD4 hash of one ed2k block, used for per-block transfer verification.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Ed2kBlockHash(pub [u8; 16]);

/// The full hashing result for one file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Ed2kFileHash {
    /// The root hash — DessPlay's `FileId`.
    pub root: Ed2kHash,
    /// MD4 of each `ED2K_BLOCK_SIZE` block (last may be short). Always at
    /// least one entry; a zero-byte file has the hash of an empty block.
    pub blocks: Vec<Ed2kBlockHash>,
    /// Total bytes hashed.
    pub size_bytes: u64,
}

/// Hash a complete in-memory buffer.
pub fn ed2k_hash_bytes(data: &[u8]) -> Ed2kFileHash {
    // Reading from a slice cannot fail.
    match ed2k_hash_reader(data) {
        Ok(hash) => hash,
        Err(_) => unreachable!("reading from a byte slice is infallible"),
    }
}

/// Hash a stream incrementally; memory use is one 64 KiB buffer plus the
/// block hash list (16 bytes per ~9.3 MiB of input).
pub fn ed2k_hash_reader<R: Read>(mut reader: R) -> io::Result<Ed2kFileHash> {
    let mut blocks = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut block_hasher = Md4::new();
    let mut block_len: u64 = 0;
    let mut size_bytes: u64 = 0;

    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        size_bytes += n as u64;
        let mut rest = buf.get(..n).unwrap_or_default();
        while !rest.is_empty() {
            let space = usize::try_from(ED2K_BLOCK_SIZE - block_len).unwrap_or(usize::MAX);
            let take = rest.len().min(space);
            let (head, tail) = rest.split_at(take);
            block_hasher.update(head);
            block_len += take as u64;
            rest = tail;
            if block_len == ED2K_BLOCK_SIZE {
                blocks.push(Ed2kBlockHash(block_hasher.finalize_reset().into()));
                block_len = 0;
            }
        }
    }

    // Flush the final partial block; a zero-byte file gets the hash of an
    // empty block so `blocks` is never empty.
    if block_len > 0 || blocks.is_empty() {
        blocks.push(Ed2kBlockHash(block_hasher.finalize_reset().into()));
    }

    let root = match blocks.as_slice() {
        [single] if size_bytes < ED2K_BLOCK_SIZE => Ed2kHash(single.0),
        all => {
            let mut root_hasher = Md4::new();
            for block in all {
                root_hasher.update(block.0);
            }
            if size_bytes.is_multiple_of(ED2K_BLOCK_SIZE) {
                // The "red" variant: exact multiples get an empty block.
                root_hasher.update(Md4::digest([]));
            }
            Ed2kHash(root_hasher.finalize().into())
        }
    };

    Ok(Ed2kFileHash {
        root,
        blocks,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use digest::Digest;

    use super::*;

    /// Deterministic non-trivial test data.
    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// Reference root hash from the ed2k crate (red = eMule/AniDB variant).
    fn reference_root(data: &[u8]) -> Ed2kHash {
        Ed2kHash(ed2k::Ed2kRed::digest(data).into())
    }

    #[test]
    fn empty_file_has_known_hash() {
        let hash = ed2k_hash_bytes(&[]);
        // MD4 of the empty string — the canonical ed2k hash of a 0-byte file.
        assert_eq!(hash.root.to_string(), "31d6cfe0d16ae931b73c59d7e0c089c0");
        assert_eq!(hash.blocks.len(), 1);
        assert_eq!(hash.size_bytes, 0);
    }

    #[test]
    fn matches_reference_at_boundary_sizes() {
        let block = ED2K_BLOCK_SIZE as usize;
        for len in [
            0,
            1,
            1000,
            block - 1,
            block,
            block + 1,
            2 * block - 1,
            2 * block,
            2 * block + 7,
        ] {
            let bytes = data(len);
            let ours = ed2k_hash_bytes(&bytes);
            assert_eq!(
                ours.root,
                reference_root(&bytes),
                "root mismatch at len {len}"
            );
            assert_eq!(ours.size_bytes, len as u64);
            assert_eq!(ours.blocks.len(), len.div_ceil(block).max(1));
        }
    }

    #[test]
    fn block_hashes_are_plain_md4_of_each_block() {
        let block = ED2K_BLOCK_SIZE as usize;
        let bytes = data(block + 1234);
        let ours = ed2k_hash_bytes(&bytes);
        assert_eq!(ours.blocks.len(), 2);
        assert_eq!(
            ours.blocks[0].0,
            <[u8; 16]>::from(md4::Md4::digest(&bytes[..block]))
        );
        assert_eq!(
            ours.blocks[1].0,
            <[u8; 16]>::from(md4::Md4::digest(&bytes[block..]))
        );
    }

    #[test]
    fn reader_and_bytes_agree() {
        let bytes = data(3 * 64 * 1024 + 17);
        let from_bytes = ed2k_hash_bytes(&bytes);
        let from_reader = ed2k_hash_reader(std::io::Cursor::new(&bytes)).unwrap();
        assert_eq!(from_bytes, from_reader);
    }

    #[test]
    fn small_file_root_is_its_single_block_hash() {
        let bytes = data(1000);
        let ours = ed2k_hash_bytes(&bytes);
        assert_eq!(ours.root.0, ours.blocks[0].0);
    }
}
