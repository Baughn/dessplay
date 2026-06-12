//! Throughput check for ed2k hashing, run by hand:
//! `cargo test -p dessplay-core --test hash_bench -- --ignored --nocapture`
//! (and again with `--release`). Not a correctness test.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::Write;
use std::time::Instant;

#[test]
#[ignore = "manual benchmark"]
fn hash_throughput() {
    const SIZE: usize = 1_200 * 1024 * 1024;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bench.bin");
    {
        let mut f = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
        let chunk = vec![0xABu8; 8 * 1024 * 1024];
        let mut written = 0;
        while written < SIZE {
            f.write_all(&chunk).unwrap();
            written += chunk.len();
        }
    }
    // Warm the page cache so we measure hashing, not first-read IO.
    let _ = std::fs::read(&path).map(|v| v.len());

    let started = Instant::now();
    let hashed =
        dessplay_core::hash::ed2k_hash_reader(std::fs::File::open(&path).unwrap()).unwrap();
    let elapsed = started.elapsed();
    println!(
        "hashed {} MiB in {:.2?} = {:.0} MiB/s (root {:02x?})",
        SIZE / 1024 / 1024,
        elapsed,
        SIZE as f64 / 1024.0 / 1024.0 / elapsed.as_secs_f64(),
        &hashed.root.0[..4],
    );
}
