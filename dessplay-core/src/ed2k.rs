use std::io::{self, Read};

use digest::Digest;

use crate::types::FileId;

/// Compute the ed2k hash (Blue variant — the standard one AniDB uses) of a reader's contents.
pub fn compute_ed2k(mut reader: impl Read) -> io::Result<FileId> {
    let mut hasher = ed2k::Ed2kBlue::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let hash = hasher.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&hash);
    Ok(FileId(id))
}

/// Like [`compute_ed2k`], but calls `progress(bytes_read_so_far)` after each chunk.
pub fn compute_ed2k_with_progress(
    mut reader: impl Read,
    progress: impl Fn(u64),
) -> io::Result<FileId> {
    let mut hasher = ed2k::Ed2kBlue::new();
    let mut buf = [0u8; 65536];
    let mut bytes_read: u64 = 0;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        bytes_read += n as u64;
        progress(bytes_read);
    }
    let hash = hasher.finalize();
    let mut id = [0u8; 16];
    id.copy_from_slice(&hash);
    Ok(FileId(id))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn empty_file() {
        let result = compute_ed2k(io::empty()).unwrap();
        // MD4 of empty input
        assert_eq!(
            result,
            compute_ed2k(&b""[..]).unwrap(),
        );
        // Verify it's a valid 16-byte hash
        assert_eq!(result.0.len(), 16);
    }

    #[test]
    fn small_file() {
        let data = b"hello world";
        let result = compute_ed2k(&data[..]).unwrap();
        // Same data should produce the same hash
        let result2 = compute_ed2k(&data[..]).unwrap();
        assert_eq!(result, result2);
    }

    #[test]
    fn exactly_one_chunk() {
        // ed2k chunk size is 9728000 bytes (9500 KiB)
        let data = vec![0xABu8; 9_728_000];
        let result = compute_ed2k(&data[..]).unwrap();
        assert_eq!(result.0.len(), 16);
    }

    #[test]
    fn multi_chunk_file() {
        // Slightly more than one chunk
        let data = vec![0xCDu8; 9_728_001];
        let result = compute_ed2k(&data[..]).unwrap();
        assert_eq!(result.0.len(), 16);
        // Should differ from single-chunk
        let single = compute_ed2k(&vec![0xCDu8; 9_728_000][..]).unwrap();
        assert_ne!(result, single);
    }

    #[test]
    fn different_data_different_hash() {
        let a = compute_ed2k(&b"hello"[..]).unwrap();
        let b = compute_ed2k(&b"world"[..]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn with_progress_matches_without() {
        use std::sync::atomic::{AtomicU64, Ordering};

        let data = vec![0xABu8; 100_000];
        let expected = compute_ed2k(&data[..]).unwrap();

        let progress_bytes = AtomicU64::new(0);
        let result =
            compute_ed2k_with_progress(&data[..], |b| progress_bytes.store(b, Ordering::Relaxed))
                .unwrap();

        assert_eq!(result, expected);
        assert_eq!(progress_bytes.load(Ordering::Relaxed), 100_000);
    }
}
