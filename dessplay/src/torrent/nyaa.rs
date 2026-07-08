//! nyaa.si RSS search: find a torrent for an exact release filename.
//!
//! The query is the playlist entry's filename (release names embed a CRC
//! tag, so an exact-title hit is near-certainly the right payload); the
//! result still gets ed2k-verified after download before it counts as a
//! local copy. Blocking by design — call from `spawn_blocking`, exactly
//! like the server's anime-titles fetch.

use std::collections::HashSet;

use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use quick_xml::Reader;
use quick_xml::events::Event;

/// One `<item>` from the nyaa RSS feed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NyaaItem {
    /// Release title (usually the exact filename for single-file torrents).
    pub title: String,
    /// The `.torrent` download URL (`https://nyaa.si/download/N.torrent`).
    pub torrent_url: String,
    /// BitTorrent info hash, lowercase hex.
    pub info_hash: String,
    /// Payload size parsed from nyaa's human-formatted `<nyaa:size>`,
    /// `None` if the field was missing or unparseable.
    pub size_bytes: Option<u64>,
    /// Current seeder count.
    pub seeders: u32,
}

/// An accepted search result, ready to hand to the torrent engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NyaaMatch {
    /// Release title.
    pub title: String,
    /// The `.torrent` download URL.
    pub torrent_url: String,
    /// Info hash, lowercase hex (magnet fallback, ban bookkeeping).
    pub info_hash: String,
}

/// Source of raw RSS for a filename query. Mocked in tests; HTTP in
/// production. Blocking by design — call from `spawn_blocking`.
pub trait NyaaSource: Send + Sync + 'static {
    /// Fetch the RSS results for searching `filename`.
    fn search(&self, filename: &str) -> std::io::Result<String>;
}

/// The real thing: one GET against nyaa.si.
pub struct HttpNyaaSource;

/// Build the RSS search URL for an exact filename query.
pub fn search_url(filename: &str) -> String {
    format!(
        "https://nyaa.si/?page=rss&q={}&c=0_0&f=0",
        utf8_percent_encode(filename, NON_ALPHANUMERIC)
    )
}

impl NyaaSource for HttpNyaaSource {
    fn search(&self, filename: &str) -> std::io::Result<String> {
        let response = ureq::get(search_url(filename))
            .header("User-Agent", "dessplay/1")
            .call()
            .map_err(std::io::Error::other)?;
        let body = response
            .into_body()
            .with_config()
            // A results feed is a few KB; anything past 8MB is not it.
            .limit(8 * 1024 * 1024)
            .read_to_vec()
            .map_err(std::io::Error::other)?;
        String::from_utf8(body).map_err(std::io::Error::other)
    }
}

/// Parse a nyaa RSS feed into items. Malformed items are skipped — the
/// feed is machine-generated; a bad item means format drift we'd rather
/// survive than die on.
pub fn parse_rss(xml: &str) -> Vec<NyaaItem> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut items = Vec::new();
    let mut in_item = false;
    let mut field: Option<Field> = None;
    let mut current = PartialItem::default();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let name = name.as_ref();
                if name == b"item" {
                    in_item = true;
                    current = PartialItem::default();
                } else if in_item {
                    field = Field::from_tag(name);
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let name = name.as_ref();
                if name == b"item" {
                    in_item = false;
                    if let Some(item) = current.take() {
                        items.push(item);
                    }
                } else {
                    field = None;
                }
            }
            Ok(Event::Text(t)) => {
                if let (true, Some(field)) = (in_item, field)
                    && let Ok(text) = t.unescape()
                {
                    current.set(field, &text);
                }
            }
            Ok(Event::CData(t)) => {
                if let (true, Some(field)) = (in_item, field) {
                    let text = String::from_utf8_lossy(&t.into_inner()).into_owned();
                    current.set(field, &text);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            // Malformed XML past this point: keep what parsed cleanly.
            Err(_) => break,
        }
    }
    items
}

/// The `<item>` children we care about.
#[derive(Clone, Copy, Debug)]
enum Field {
    Title,
    Link,
    InfoHash,
    Size,
    Seeders,
}

impl Field {
    fn from_tag(tag: &[u8]) -> Option<Self> {
        match tag {
            b"title" => Some(Self::Title),
            b"link" => Some(Self::Link),
            b"nyaa:infoHash" => Some(Self::InfoHash),
            b"nyaa:size" => Some(Self::Size),
            b"nyaa:seeders" => Some(Self::Seeders),
            _ => None,
        }
    }
}

#[derive(Default)]
struct PartialItem {
    title: Option<String>,
    link: Option<String>,
    info_hash: Option<String>,
    size_bytes: Option<u64>,
    seeders: Option<u32>,
}

impl PartialItem {
    fn set(&mut self, field: Field, text: &str) {
        match field {
            Field::Title => self.title = Some(text.to_string()),
            Field::Link => self.link = Some(text.to_string()),
            Field::InfoHash => self.info_hash = Some(text.to_ascii_lowercase()),
            Field::Size => self.size_bytes = parse_size(text),
            Field::Seeders => self.seeders = text.trim().parse().ok(),
        }
    }

    fn take(&mut self) -> Option<NyaaItem> {
        Some(NyaaItem {
            title: self.title.take()?,
            torrent_url: self.link.take()?,
            info_hash: self.info_hash.take()?,
            size_bytes: self.size_bytes,
            seeders: self.seeders.unwrap_or(0),
        })
    }
}

/// Parse nyaa's human-formatted size ("1.4 GiB") into bytes. Best-effort:
/// it only feeds a ±tolerance sanity check, never an exact comparison.
pub fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let split = text.find(|c: char| c != '.' && !c.is_ascii_digit())?;
    let (number, unit) = text.split_at(split);
    let value: f64 = number.trim().parse().ok()?;
    let scale: f64 = match unit.trim() {
        "B" | "Bytes" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0f64.powi(4),
        _ => return None,
    };
    let bytes = value * scale;
    (bytes.is_finite() && bytes >= 0.0).then_some(bytes as u64)
}

/// How far nyaa's (rounded, human-formatted) size may deviate from the
/// playlist entry's exact byte size and still count as the same payload.
const SIZE_TOLERANCE: f64 = 0.03;

/// Pick the acceptable result for `filename`, or `None`.
///
/// Accepts an item whose title equals the filename (whitespace-normalized,
/// case-insensitive, extension optional), with at least one seeder, whose
/// size is within ±3% of `size_bytes`, and whose info hash isn't banned
/// (a previous download of it failed ed2k verification). Ties break to
/// the most seeders.
pub fn pick_match(
    items: &[NyaaItem],
    filename: &str,
    size_bytes: u64,
    banned: &HashSet<String>,
) -> Option<NyaaMatch> {
    let want_full = normalize_title(filename);
    let want_stem = normalize_title(stem(filename));
    items
        .iter()
        .filter(|item| {
            let title = normalize_title(&item.title);
            (title == want_full || title == want_stem)
                && item.seeders >= 1
                && !banned.contains(&item.info_hash)
                && item.size_bytes.is_some_and(|s| {
                    let want = size_bytes as f64;
                    (s as f64 - want).abs() <= want * SIZE_TOLERANCE
                })
        })
        .max_by_key(|item| item.seeders)
        .map(|item| NyaaMatch {
            title: item.title.clone(),
            torrent_url: item.torrent_url.clone(),
            info_hash: item.info_hash.clone(),
        })
}

/// Whitespace-collapse + lowercase, so cosmetic differences don't reject
/// the right release.
fn normalize_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Filename minus its extension (nyaa titles sometimes drop it).
fn stem(filename: &str) -> &str {
    match filename.rsplit_once('.') {
        // Only strip something extension-shaped; "Show S2 - 01" has no dot
        // and "a.b/weird" never reaches here (filenames, not paths).
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 4 => stem,
        _ => filename,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/nyaa_search.rss");

    fn fixture_items() -> Vec<NyaaItem> {
        parse_rss(FIXTURE)
    }

    #[test]
    fn parses_fixture_items() {
        let items = fixture_items();
        assert_eq!(items.len(), 3);
        assert_eq!(
            items[0].title,
            "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv"
        );
        assert_eq!(
            items[0].torrent_url,
            "https://nyaa.si/download/1846001.torrent"
        );
        assert_eq!(
            items[0].info_hash,
            "0123456789abcdef0123456789abcdef01234567"
        );
        assert_eq!(
            items[0].size_bytes,
            Some((1.4 * 1024.0 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(items[0].seeders, 412);
        // Entity-escaped title round-trips.
        assert_eq!(
            items[2].title,
            "[Judas] Foo & Bar - 01 <v2> (1080p) [ABCD1234].mkv"
        );
    }

    #[test]
    fn parse_size_units() {
        assert_eq!(
            parse_size("711.7 MiB"),
            Some((711.7 * 1024.0 * 1024.0) as u64)
        );
        assert_eq!(
            parse_size("1.4 GiB"),
            Some((1.4 * 1024.0f64.powi(3)) as u64)
        );
        assert_eq!(parse_size("512 B"), Some(512));
        assert_eq!(parse_size("3 KiB"), Some(3072));
        assert_eq!(parse_size("nonsense"), None);
        assert_eq!(parse_size("1.4 GB"), None); // nyaa uses binary units only
    }

    #[test]
    fn picks_exact_title_match() {
        let items = fixture_items();
        let size = (1.4 * 1024.0f64.powi(3)) as u64;
        let m = pick_match(
            &items,
            "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv",
            size,
            &HashSet::new(),
        )
        .unwrap();
        assert_eq!(m.info_hash, "0123456789abcdef0123456789abcdef01234567");
    }

    #[test]
    fn matches_title_without_extension() {
        let items = vec![NyaaItem {
            title: "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4]".into(),
            torrent_url: "https://nyaa.si/download/1.torrent".into(),
            info_hash: "aa".into(),
            size_bytes: Some(1000),
            seeders: 5,
        }];
        let m = pick_match(
            &items,
            "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv",
            1000,
            &HashSet::new(),
        );
        assert!(m.is_some());
    }

    #[test]
    fn rejects_wrong_title() {
        let items = fixture_items();
        assert!(
            pick_match(
                &items,
                "[SubsPlease] Clevatess S2 - 02 (1080p) [11111111].mkv",
                (1.4 * 1024.0f64.powi(3)) as u64,
                &HashSet::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_size_out_of_tolerance() {
        let items = fixture_items();
        // Right title, but claim the file is half the size nyaa reports.
        assert!(
            pick_match(
                &items,
                "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv",
                (0.7 * 1024.0f64.powi(3)) as u64,
                &HashSet::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn rejects_zero_seeders() {
        let mut items = fixture_items();
        items[0].seeders = 0;
        assert!(
            pick_match(
                &items,
                "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv",
                (1.4 * 1024.0f64.powi(3)) as u64,
                &HashSet::new(),
            )
            .is_none()
        );
    }

    #[test]
    fn skips_banned_info_hash_and_falls_to_next() {
        let size = 1000;
        let make = |hash: &str, seeders| NyaaItem {
            title: "Show - 01.mkv".into(),
            torrent_url: format!("https://nyaa.si/download/{hash}.torrent"),
            info_hash: hash.into(),
            size_bytes: Some(size),
            seeders,
        };
        let items = vec![make("aa", 100), make("bb", 10)];
        let banned: HashSet<String> = ["aa".to_string()].into();
        let m = pick_match(&items, "Show - 01.mkv", size, &banned).unwrap();
        assert_eq!(m.info_hash, "bb");
    }

    #[test]
    fn ties_break_to_most_seeders() {
        let make = |hash: &str, seeders| NyaaItem {
            title: "Show - 01.mkv".into(),
            torrent_url: "u".into(),
            info_hash: hash.into(),
            size_bytes: Some(1000),
            seeders,
        };
        let items = vec![make("aa", 3), make("bb", 30), make("cc", 7)];
        let m = pick_match(&items, "Show - 01.mkv", 1000, &HashSet::new()).unwrap();
        assert_eq!(m.info_hash, "bb");
    }

    #[test]
    fn search_url_percent_encodes() {
        let url = search_url("[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv");
        assert!(url.starts_with("https://nyaa.si/?page=rss&q="));
        assert!(url.ends_with("&c=0_0&f=0"));
        assert!(!url.contains('['));
        assert!(!url.contains(' '));
    }
}
