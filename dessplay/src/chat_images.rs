//! Inline chat images (design.md, Inline chat images): fetch an image
//! URL posted in chat, enforce the caps that make auto-fetching from a
//! public, unauthenticated IRC channel tolerable, cache the wire bytes
//! on disk, and decode into a pre-scaled [`image::DynamicImage`] for
//! the chat pane to render.
//!
//! Everything here is blocking (ureq, filesystem) and runs on the
//! blocking pool — see the `FetchChatImage` arm in `run.rs`. The
//! validation and decode steps are pure functions so the caps are
//! testable without a network.

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// Hard cap on the downloaded byte stream. A chat image is a
/// screenshot or a reaction picture; anything larger is either a
/// mistake or an attack on our disk/memory.
const MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Per-axis pixel cap for the decoder. The wire cap can't stop a
/// decompression bomb (a few KB of PNG can claim gigapixels), so the
/// decoder enforces its own limits before allocating.
const MAX_DIMENSION: u32 = 8192;

/// Decoder allocation cap. 64MB comfortably fits any real chat image
/// (~4000×4000 RGBA) while bounding what a hostile file can make us
/// allocate.
const MAX_DECODE_ALLOC: u64 = 64 * 1024 * 1024;

/// Total size the on-disk cache may reach before old entries are
/// evicted, oldest first.
const CACHE_LIMIT_BYTES: u64 = 64 * 1024 * 1024;

/// Longest edge after pre-scaling. The chat pane re-encodes the image
/// on the render thread every time its cell size changes; keeping the
/// source small bounds that cost. A terminal cell column is far
/// coarser than 1280px anyway.
const PRESCALE_MAX: u32 = 1280;

/// Fetch, validate, cache, and decode one chat image URL. Blocking —
/// call from the blocking pool. Errors are user-invisible (the URL
/// simply stays plain text) but logged by the caller at debug.
pub fn fetch(url: &str, cache_dir: &Path) -> Result<image::DynamicImage, String> {
    // Defense in depth: detection already requires https, but this
    // function is the last gate before a request leaves the process.
    if !url.starts_with("https://") {
        return Err(format!("not an https url: {url}"));
    }
    let cache_path = cache_file(cache_dir, url);
    let bytes = match std::fs::read(&cache_path) {
        Ok(bytes) => bytes,
        Err(_) => {
            let bytes = download(url)?;
            store_in_cache(&cache_path, &bytes);
            bytes
        }
    };
    let image = decode_capped(&bytes)?;
    Ok(prescale(image))
}

/// Where `url`'s wire bytes are cached: a sha256-named file under
/// `<cache_dir>/images/`. A cryptographic hash on purpose — the URL is
/// remote-authored, so name collisions must be infeasible, and the
/// flat hex name can't traverse anywhere.
fn cache_file(cache_dir: &Path, url: &str) -> PathBuf {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(url.as_bytes());
    let mut name = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    cache_dir.join("images").join(name)
}

/// One capped GET. An agent with a hard 30s per-call timeout (ureq's
/// default has *none* — a connected-but-silent host would park the
/// blocking thread forever), pinned to https so a redirect can't
/// downgrade the connection.
fn download(url: &str) -> Result<Vec<u8>, String> {
    let agent = ureq::Agent::from(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .https_only(true)
            .build(),
    );
    let response = agent
        .get(url)
        .header("User-Agent", "dessplay/1")
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?;
    validate_headers(
        response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        response
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok()),
    )?;
    // The declared length was checked above, but the cap must hold for
    // chunked/lying responses too: read at most MAX_BYTES + 1 and
    // reject the overflow byte.
    let mut bytes = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("reading {url}: {e}"))?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(format!("{url}: body exceeds {MAX_BYTES} bytes"));
    }
    Ok(bytes)
}

/// Reject a response before buffering its body: a declared type that
/// isn't an image, or a declared length over the cap. Both headers are
/// optional — absence is tolerated (the byte cap and the decoder's
/// format sniffing still stand behind them).
fn validate_headers(content_type: Option<&str>, content_length: Option<u64>) -> Result<(), String> {
    if let Some(kind) = content_type {
        // Parameters (e.g. "image/png; charset=binary") don't matter.
        if !kind.trim_start().starts_with("image/") {
            return Err(format!("content-type {kind:?} is not an image"));
        }
    }
    if let Some(length) = content_length
        && length > MAX_BYTES
    {
        return Err(format!("content-length {length} exceeds {MAX_BYTES}"));
    }
    Ok(())
}

/// Decode with hard limits, sniffing the format from the magic bytes
/// (never the URL's extension — the two need not agree, and the
/// extension is remote-authored). An animated GIF decodes to its first
/// frame.
fn decode_capped(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC);
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("sniffing image format: {e}"))?;
    reader.limits(limits);
    reader.decode().map_err(|e| format!("decoding image: {e}"))
}

/// Downscale to fit [`PRESCALE_MAX`] on the longest edge (aspect
/// preserved); smaller images pass through untouched.
fn prescale(image: image::DynamicImage) -> image::DynamicImage {
    if image.width() <= PRESCALE_MAX && image.height() <= PRESCALE_MAX {
        return image;
    }
    image.thumbnail(PRESCALE_MAX, PRESCALE_MAX)
}

/// Write freshly downloaded bytes into the cache, then evict oldest
/// entries while the cache directory exceeds its budget. Best-effort
/// throughout: a full disk or permission problem must not fail the
/// fetch that already has its bytes in memory.
fn store_in_cache(cache_path: &Path, bytes: &[u8]) {
    let Some(dir) = cache_path.parent() else {
        return;
    };
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::debug!("creating image cache dir: {e}");
        return;
    }
    if let Err(e) = std::fs::write(cache_path, bytes) {
        tracing::debug!("writing image cache entry: {e}");
        return;
    }
    evict_cache(dir, CACHE_LIMIT_BYTES);
}

/// Delete the oldest entries (by mtime) until the directory's regular
/// files total at most `limit` bytes. Only direct children are
/// considered — this touches nothing outside the images directory.
fn evict_cache(dir: &Path, limit: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            Some((meta.modified().ok()?, meta.len(), entry.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    files.sort_by_key(|(mtime, _, _)| *mtime);
    for (_, len, path) in files {
        if total <= limit {
            break;
        }
        match std::fs::remove_file(&path) {
            Ok(()) => total = total.saturating_sub(len),
            Err(e) => tracing::debug!("evicting image cache entry: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn header_validation_rejects_oversize_and_non_image() {
        assert!(validate_headers(Some("image/png"), Some(1000)).is_ok());
        assert!(validate_headers(Some("image/jpeg; charset=binary"), None).is_ok());
        // Absent headers are tolerated — the byte cap and decoder stand behind them.
        assert!(validate_headers(None, None).is_ok());
        assert!(validate_headers(Some("text/html"), Some(1000)).is_err());
        assert!(validate_headers(Some("image/png"), Some(MAX_BYTES + 1)).is_err());
    }

    #[test]
    fn cache_names_are_flat_hex() {
        let path = cache_file(Path::new("/cache"), "https://example.com/../a.png");
        let name = path.file_name().unwrap().to_str().unwrap();
        assert_eq!(name.len(), 64);
        assert!(name.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(path.parent().unwrap(), Path::new("/cache/images"));
    }

    #[test]
    fn decode_rejects_dimension_bomb() {
        // A valid PNG header claiming 40000×40000 — the decoder must
        // refuse before allocating, not after.
        let png = png_bytes(40_000, 40_000, false);
        assert!(decode_capped(&png).is_err());
    }

    #[test]
    fn decode_and_prescale_real_png() {
        let mut img = image::RgbaImage::new(2000, 500);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(
                &mut std::io::Cursor::new(&mut bytes),
                image::ImageFormat::Png,
            )
            .unwrap();
        let decoded = prescale(decode_capped(&bytes).unwrap());
        assert_eq!(decoded.width(), PRESCALE_MAX);
        assert_eq!(decoded.height(), 320);
    }

    #[test]
    fn eviction_deletes_oldest_first_and_only_files() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir(&sub).unwrap();
        for (name, age_secs) in [("old", 300), ("mid", 200), ("new", 100)] {
            let path = dir.path().join(name);
            std::fs::write(&path, vec![0u8; 100]).unwrap();
            let mtime = std::time::SystemTime::now() - std::time::Duration::from_secs(age_secs);
            let file = std::fs::File::open(&path).unwrap();
            file.set_modified(mtime).unwrap();
        }
        evict_cache(dir.path(), 250);
        assert!(!dir.path().join("old").exists(), "oldest entry evicted");
        assert!(dir.path().join("mid").exists());
        assert!(dir.path().join("new").exists());
        assert!(sub.exists(), "directories are never touched");
    }

    /// Minimal PNG bytes with an IHDR claiming the given dimensions.
    /// `valid_body` unused for now — the bomb test only needs a header
    /// good enough for the decoder to read dimensions from.
    fn png_bytes(width: u32, height: u32, _valid_body: bool) -> Vec<u8> {
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(b"IHDR");
        ihdr.extend_from_slice(&width.to_be_bytes());
        ihdr.extend_from_slice(&height.to_be_bytes());
        // bit depth 8, color type 6 (RGBA), compression/filter/interlace 0
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        let mut png = Vec::new();
        png.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]);
        png.extend_from_slice(&((ihdr.len() - 4) as u32).to_be_bytes());
        let crc = crc32(&ihdr);
        png.extend_from_slice(&ihdr);
        png.extend_from_slice(&crc.to_be_bytes());
        png
    }

    /// CRC-32 (PNG flavor) for the synthetic chunk above.
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in data {
            crc ^= byte as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xedb8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
}
