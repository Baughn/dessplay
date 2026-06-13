//! On-disk assembly of a downloaded file (Phase 9B), with ed2k
//! per-block verification.
//!
//! A download is a single file at the cache path, written at 256 KiB
//! chunk offsets. Two bitfields track progress:
//!
//! - `written` — chunks whose bytes are on disk (not yet trusted).
//! - `verified` — *blocks* (9,728,000 B) whose MD4 matched the
//!   peer-supplied block-hash list (which is itself validated against
//!   the file's ed2k root, the playlist key, before use).
//!
//! Chunks align to ed2k blocks: [`CHUNK_SIZE`] divides the block size
//! exactly ([`CHUNKS_PER_BLOCK`] = 38), so each block is a contiguous
//! group of whole chunks and no chunk straddles a boundary.
//! Verification is per block: once all of a block's chunks are written,
//! the block's byte range is hashed and compared. A mismatch clears
//! exactly that block's chunks for re-fetch — no shared-chunk
//! bookkeeping.
//!
//! **Resume** needs no sidecar: re-open the partial file, mark every
//! chunk `written` (the bytes are there), and call [`ChunkStore::verify`]
//! — blocks that hash correctly become verified, incomplete/corrupt
//! ones have their chunks cleared and re-fetched.

use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use dessplay_core::hash::{Ed2kBlockHash, ED2K_BLOCK_SIZE, block_hash, root_from_blocks};
use dessplay_core::net::{Bitfield, CHUNKS_PER_BLOCK, chunk_count, chunk_range};
use dessplay_core::types::Ed2kHash;

/// Chunks per ed2k block (the alignment invariant; `u32` for indexing).
const CPB: u32 = CHUNKS_PER_BLOCK as u32;

/// Number of ed2k blocks covering `size_bytes`.
fn block_count(size_bytes: u64) -> u32 {
    size_bytes.div_ceil(ED2K_BLOCK_SIZE) as u32
}

/// Byte range of block `b` within a file of `size_bytes`.
fn block_byte_range(b: u32, size_bytes: u64) -> std::ops::Range<u64> {
    let start = (b as u64) * ED2K_BLOCK_SIZE;
    let end = (start + ED2K_BLOCK_SIZE).min(size_bytes);
    start..end
}

/// The chunk indices making up block `b` (contiguous, since chunks align
/// to blocks): `[b*38, (b+1)*38)`, clamped to the file's chunk count.
fn chunks_in_block(b: u32, total_chunks: u32) -> std::ops::Range<u32> {
    let first = b * CPB;
    let last = (first + CPB).min(total_chunks);
    first..last
}

/// The single block a chunk belongs to (no straddling under alignment).
fn block_of_chunk(index: u32) -> u32 {
    index / CPB
}

/// What a [`ChunkStore::verify`] pass changed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct VerifySummary {
    /// Blocks that verified this pass.
    pub newly_verified: u32,
    /// Blocks that failed (their chunks were cleared for re-fetch).
    pub mismatched: Vec<u32>,
}

/// A partially-downloaded file on disk.
pub struct ChunkStore {
    file: std::fs::File,
    path: PathBuf,
    size_bytes: u64,
    chunks: u32,
    blocks: u32,
    /// Chunks whose bytes are on disk.
    written: Bitfield,
    /// Blocks whose hash matched (trusted).
    verified: Vec<bool>,
}

impl ChunkStore {
    /// Create a fresh download file of `size_bytes` at `path` (parent
    /// directories created), all chunks absent.
    pub fn create(path: &Path, size_bytes: u64) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        file.set_len(size_bytes)?;
        Ok(Self::with_file(file, path.to_path_buf(), size_bytes, false))
    }

    /// Re-open an existing partial file for resume. Every chunk is
    /// marked written (the bytes are present); the caller verifies to
    /// rebuild a trustworthy `verified` set. If the file is missing or
    /// the wrong size, falls back to a fresh [`Self::create`].
    pub fn open(path: &Path, size_bytes: u64) -> io::Result<Self> {
        let file = match std::fs::OpenOptions::new().read(true).write(true).open(path) {
            Ok(file) if file.metadata().map(|m| m.len()).unwrap_or(0) == size_bytes => file,
            _ => return Self::create(path, size_bytes),
        };
        Ok(Self::with_file(file, path.to_path_buf(), size_bytes, true))
    }

    fn with_file(file: std::fs::File, path: PathBuf, size_bytes: u64, all_written: bool) -> Self {
        let chunks = chunk_count(size_bytes);
        let blocks = block_count(size_bytes);
        let mut written = Bitfield::new(chunks);
        if all_written {
            for i in 0..chunks {
                written.set(i);
            }
        }
        ChunkStore {
            file,
            path,
            size_bytes,
            chunks,
            blocks,
            written,
            verified: vec![false; blocks as usize],
        }
    }

    /// The download's path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// File size in bytes.
    pub fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    /// Total chunk count.
    pub fn chunk_count(&self) -> u32 {
        self.chunks
    }

    /// Whether a chunk's bytes are on disk (not necessarily verified).
    pub fn is_written(&self, index: u32) -> bool {
        self.written.get(index)
    }

    /// Byte length of block `b` (the last block is short).
    pub fn block_size(&self, b: u32) -> u64 {
        let r = block_byte_range(b, self.size_bytes);
        r.end - r.start
    }

    /// Validate a peer-supplied block-hash list against `root` (the file
    /// id) and the expected block count. Hashes must be trusted before
    /// they can verify chunks.
    pub fn block_hashes_match(
        &self,
        root: Ed2kHash,
        hashes: &[Ed2kBlockHash],
    ) -> bool {
        hashes.len() as u32 == self.blocks && root_from_blocks(hashes, self.size_bytes) == root
    }

    /// Write a chunk's bytes at its offset and mark it present. The data
    /// length must match the chunk's true size (short last chunk).
    pub fn write_chunk(&mut self, index: u32, data: &[u8]) -> io::Result<()> {
        let range = chunk_range(index, self.size_bytes);
        if index >= self.chunks || data.len() as u64 != (range.end - range.start) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("chunk {index} has wrong size {}", data.len()),
            ));
        }
        self.file.seek(SeekFrom::Start(range.start))?;
        self.file.write_all(data)?;
        self.written.set(index);
        Ok(())
    }

    /// Read a chunk's bytes (e.g. to serve it). Returns an error for an
    /// absent chunk.
    pub fn read_chunk(&mut self, index: u32) -> io::Result<Vec<u8>> {
        if !self.written.get(index) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "chunk not present"));
        }
        let range = chunk_range(index, self.size_bytes);
        let mut buf = vec![0u8; (range.end - range.start) as usize];
        self.file.seek(SeekFrom::Start(range.start))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Verify every fully-written, not-yet-verified block against
    /// `block_hashes` (already validated via [`Self::block_hashes_match`]).
    /// A match marks the block verified; a mismatch clears its chunks'
    /// written bits so they are re-fetched.
    pub fn verify(&mut self, block_hashes: &[Ed2kBlockHash]) -> io::Result<VerifySummary> {
        let mut summary = VerifySummary::default();
        for b in 0..self.blocks {
            if self.verified[b as usize] {
                continue;
            }
            let chunks = chunks_in_block(b, self.chunks);
            if !chunks.clone().all(|c| self.written.get(c)) {
                continue;
            }
            let bytes = block_byte_range(b, self.size_bytes);
            let mut buf = vec![0u8; (bytes.end - bytes.start) as usize];
            self.file.seek(SeekFrom::Start(bytes.start))?;
            self.file.read_exact(&mut buf)?;
            if block_hash(&buf) == block_hashes[b as usize] {
                self.verified[b as usize] = true;
                summary.newly_verified += 1;
            } else {
                for c in chunks {
                    self.written.unset(c);
                }
                summary.mismatched.push(b);
            }
        }
        Ok(summary)
    }

    /// Are all blocks verified?
    pub fn is_complete(&self) -> bool {
        self.blocks > 0 && self.verified.iter().all(|&v| v)
    }

    /// Progress in basis points (0–10000), by verified bytes. Drives the
    /// `Downloading { progress_bps }` availability the group sees.
    pub fn progress_bps(&self) -> u16 {
        if self.blocks == 0 {
            return 10_000;
        }
        let verified = (0..self.blocks)
            .filter(|&b| self.verified[b as usize])
            .map(|b| {
                let r = block_byte_range(b, self.size_bytes);
                r.end - r.start
            })
            .sum::<u64>();
        ((verified.saturating_mul(10_000)) / self.size_bytes.max(1)) as u16
    }

    /// The bitfield to advertise: a chunk is available iff its block is
    /// verified (we never serve unverified bytes).
    pub fn available(&self) -> Bitfield {
        let mut bf = Bitfield::new(self.chunks);
        for i in 0..self.chunks {
            if self.verified[block_of_chunk(i) as usize] {
                bf.set(i);
            }
        }
        bf
    }

    /// Chunks still needed: those in an unverified block and not
    /// currently written. The download scheduler requests these.
    pub fn needed_chunks(&self) -> Vec<u32> {
        (0..self.chunks)
            .filter(|&i| !self.written.get(i) && !self.verified[block_of_chunk(i) as usize])
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::hash::ed2k_hash_bytes;

    use super::*;

    /// Deterministic bytes.
    fn data(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i.wrapping_mul(31) % 251) as u8).collect()
    }

    /// Feed a whole file into a store chunk by chunk (in order), then
    /// verify. Returns the store.
    fn fill(path: &Path, bytes: &[u8]) -> ChunkStore {
        let size = bytes.len() as u64;
        let mut store = ChunkStore::create(path, size).unwrap();
        for i in 0..store.chunk_count() {
            let r = chunk_range(i, size);
            store
                .write_chunk(i, &bytes[r.start as usize..r.end as usize])
                .unwrap();
        }
        store
    }

    #[test]
    fn geometry_is_aligned_no_straddling() {
        let size = 3 * ED2K_BLOCK_SIZE + 12345;
        let total = chunk_count(size);
        // Every block's chunks map back to that block, and the groups
        // partition the chunk space (no overlap, no gaps).
        let mut seen = 0u32;
        for b in 0..block_count(size) {
            let chunks = chunks_in_block(b, total);
            for c in chunks.clone() {
                assert_eq!(block_of_chunk(c), b, "chunk {c} should belong to block {b}");
            }
            seen += chunks.end - chunks.start;
        }
        assert_eq!(seen, total, "blocks must partition all chunks exactly");
        // The chunk at the first block boundary belongs to exactly one
        // block (alignment): chunk 38 starts block 1.
        assert_eq!(block_of_chunk(CPB), 1);
        assert_eq!(block_of_chunk(CPB - 1), 0);
    }

    #[test]
    fn fill_verify_completes_a_multi_block_file() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = data(2 * ED2K_BLOCK_SIZE as usize + 5000);
        let full = ed2k_hash_bytes(&bytes);
        let path = dir.path().join("dl.bin");

        let mut store = fill(&path, &bytes);
        assert!(!store.is_complete());
        // Block hashes validate against the root.
        assert!(store.block_hashes_match(full.root, &full.blocks));

        let summary = store.verify(&full.blocks).unwrap();
        assert_eq!(summary.newly_verified, full.blocks.len() as u32);
        assert!(summary.mismatched.is_empty());
        assert!(store.is_complete());
        assert_eq!(store.progress_bps(), 10_000);
        assert!(store.available().is_complete());
        assert!(store.needed_chunks().is_empty());

        // The assembled file on disk equals the original.
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn a_corrupt_chunk_fails_its_block_and_is_refetched() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = data(ED2K_BLOCK_SIZE as usize + 1000); // 2 blocks
        let full = ed2k_hash_bytes(&bytes);
        let path = dir.path().join("dl.bin");
        let size = bytes.len() as u64;

        let mut store = ChunkStore::create(&path, size).unwrap();
        // Write everything correctly except corrupt chunk 0 (in block 0).
        for i in 0..store.chunk_count() {
            let r = chunk_range(i, size);
            let mut slice = bytes[r.start as usize..r.end as usize].to_vec();
            if i == 0 {
                slice[0] ^= 0xff;
            }
            store.write_chunk(i, &slice).unwrap();
        }
        let summary = store.verify(&full.blocks).unwrap();
        // Block 0 mismatched; block 1 (and any block 0 doesn't touch)
        // verified.
        assert!(summary.mismatched.contains(&0));
        assert!(!store.is_complete());
        // The bad block's chunks are queued for re-fetch.
        let needed = store.needed_chunks();
        assert!(needed.contains(&0), "corrupt chunk 0 should be re-needed");

        // Re-fetch correctly and re-verify: now complete.
        for i in needed {
            let r = chunk_range(i, size);
            store
                .write_chunk(i, &bytes[r.start as usize..r.end as usize])
                .unwrap();
        }
        let summary = store.verify(&full.blocks).unwrap();
        assert!(summary.mismatched.is_empty());
        assert!(store.is_complete());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }

    #[test]
    fn resume_rebuilds_verified_blocks_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = data(2 * ED2K_BLOCK_SIZE as usize + 5000);
        let full = ed2k_hash_bytes(&bytes);
        let path = dir.path().join("dl.bin");

        // First session: complete the file, then drop the store.
        {
            let mut store = fill(&path, &bytes);
            store.verify(&full.blocks).unwrap();
            assert!(store.is_complete());
        }

        // Resume: re-open, verify against the (validated) hashes — every
        // block on disk is good, so it comes back complete with no
        // re-fetch and no sidecar.
        let mut resumed = ChunkStore::open(&path, bytes.len() as u64).unwrap();
        let summary = resumed.verify(&full.blocks).unwrap();
        assert_eq!(summary.newly_verified, full.blocks.len() as u32);
        assert!(resumed.is_complete());
        assert!(resumed.needed_chunks().is_empty());
    }

    #[test]
    fn resume_refetches_a_partial_trailing_block() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = data(2 * ED2K_BLOCK_SIZE as usize + 5000);
        let full = ed2k_hash_bytes(&bytes);
        let path = dir.path().join("dl.bin");
        let size = bytes.len() as u64;

        // Write only block 0's chunks (truncated download).
        {
            let mut store = ChunkStore::create(&path, size).unwrap();
            for c in chunks_in_block(0, chunk_count(size)) {
                let r = chunk_range(c, size);
                store
                    .write_chunk(c, &bytes[r.start as usize..r.end as usize])
                    .unwrap();
            }
            // The rest of the file is zero-filled (set_len), so blocks
            // 1+ are present-but-wrong on disk.
        }

        // Resume marks everything written, then verify clears the bad
        // (zero-filled) blocks for re-fetch while keeping block 0.
        let mut resumed = ChunkStore::open(&path, size).unwrap();
        let summary = resumed.verify(&full.blocks).unwrap();
        assert!(summary.newly_verified >= 1, "block 0 should survive resume");
        assert!(!resumed.is_complete());
        assert!(
            !resumed.needed_chunks().is_empty(),
            "the zero-filled blocks must be re-fetched"
        );
        // Block 0's chunks are NOT re-needed.
        let needed = resumed.needed_chunks();
        assert!(!needed.contains(&0));
    }
}
