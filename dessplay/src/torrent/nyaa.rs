//! nyaa.si RSS search: find a torrent for an exact release filename.
//!
//! The query is the playlist entry's filename (release names embed a CRC
//! tag, so an exact-title hit is near-certainly the right payload); the
//! result still gets ed2k-verified after download before it counts as a
//! local copy. Blocking by design — call from `spawn_blocking`, exactly
//! like the server's anime-titles fetch.

use std::collections::HashSet;
use std::path::{Component, Path};

use librqbit::{TorrentMetaV1Owned, torrent_from_bytes};
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
    /// Bounds on the payload size, parsed from nyaa's human-formatted
    /// `<nyaa:size>`: the range of actual byte sizes that would display
    /// as that (rounded) string. `None` if the field was missing or
    /// unparseable.
    pub size_bounds: Option<(u64, u64)>,
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

/// A user-selected search result whose `.torrent` metadata proves it has
/// exactly one safe payload file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NyaaBrowseResult {
    /// Release title from the RSS feed.
    pub title: String,
    /// Actual single-file payload name from the torrent metainfo.
    pub filename: String,
    /// Exact payload size from the torrent metainfo.
    pub size_bytes: u64,
    /// Current seeder count from the RSS feed.
    pub seeders: u32,
    /// Torrent identity and download location.
    pub chosen: NyaaMatch,
}

/// Nyaa search category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NyaaCategory {
    /// Every category, used by the automatic exact-filename lookup.
    All,
    /// Anime, all subcategories (`c=1_0`), used by the playlist browser.
    Anime,
}

impl NyaaCategory {
    fn query_value(self) -> &'static str {
        match self {
            Self::All => "0_0",
            Self::Anime => "1_0",
        }
    }
}

/// Source of raw RSS for a filename query. Mocked in tests; HTTP in
/// production. Blocking by design — call from `spawn_blocking`.
pub trait NyaaSource: Send + Sync + 'static {
    /// Fetch the RSS results for searching `filename`.
    fn search(&self, filename: &str) -> std::io::Result<String>;

    /// Fetch an RSS feed in a specific category. Existing test sources only
    /// need exact lookup, so the default preserves the old behavior.
    fn search_category(&self, query: &str, _category: NyaaCategory) -> std::io::Result<String> {
        self.search(query)
    }

    /// Fetch raw `.torrent` metadata for browse-result inspection.
    fn fetch_torrent(&self, _url: &str) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "torrent metadata fetching is unsupported",
        ))
    }
}

/// The real thing: one GET against nyaa.si.
pub struct HttpNyaaSource;

/// Build the RSS search URL for an exact filename query.
pub fn search_url(filename: &str) -> String {
    search_url_for(filename, NyaaCategory::All)
}

/// Build an RSS search URL for a query and category.
pub fn search_url_for(query: &str, category: NyaaCategory) -> String {
    format!(
        "https://nyaa.si/?page=rss&q={}&c={}&f=0",
        utf8_percent_encode(query, NON_ALPHANUMERIC),
        category.query_value(),
    )
}

impl NyaaSource for HttpNyaaSource {
    fn search(&self, filename: &str) -> std::io::Result<String> {
        self.search_category(filename, NyaaCategory::All)
    }

    fn search_category(&self, query: &str, category: NyaaCategory) -> std::io::Result<String> {
        let response = ureq::get(search_url_for(query, category))
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

    fn fetch_torrent(&self, url: &str) -> std::io::Result<Vec<u8>> {
        let response = ureq::get(url)
            .header("User-Agent", "dessplay/1")
            .call()
            .map_err(std::io::Error::other)?;
        response
            .into_body()
            .with_config()
            .limit(16 * 1024 * 1024)
            .read_to_vec()
            .map_err(std::io::Error::other)
    }
}

/// Search the anime category and inspect at most `limit` RSS entries,
/// returning only safe, single-file torrents in feed order. A bad individual
/// result is skipped so one removed or malformed torrent does not fail the
/// whole search.
pub fn browse_single_file_results(
    source: &dyn NyaaSource,
    query: &str,
    limit: usize,
) -> std::io::Result<Vec<NyaaBrowseResult>> {
    let xml = source.search_category(query, NyaaCategory::Anime)?;
    let mut results = Vec::new();
    for item in parse_rss(&xml).into_iter().take(limit) {
        if item.seeders == 0 {
            continue;
        }
        let Ok(bytes) = source.fetch_torrent(&item.torrent_url) else {
            continue;
        };
        let Some((filename, size_bytes)) = single_file_payload(&bytes) else {
            continue;
        };
        results.push(NyaaBrowseResult {
            title: item.title.clone(),
            filename,
            size_bytes,
            seeders: item.seeders,
            chosen: NyaaMatch {
                title: item.title,
                torrent_url: item.torrent_url,
                info_hash: item.info_hash,
            },
        });
    }
    Ok(results)
}

/// Extract a safe single payload from `.torrent` bytes.
fn single_file_payload(bytes: &[u8]) -> Option<(String, u64)> {
    let torrent: TorrentMetaV1Owned = torrent_from_bytes(bytes).ok()?;
    let length = torrent.info.length?;
    if length == 0 {
        return None;
    }
    if torrent.info.files.is_some() {
        return None;
    }
    let raw_name = torrent.info.name?;
    let raw = raw_name.as_ref();
    let name = std::str::from_utf8(raw).ok()?.trim();
    if name.is_empty() || name.contains(['/', '\\']) {
        return None;
    }
    let mut components = Path::new(name).components();
    if !matches!(components.next(), Some(Component::Normal(_))) || components.next().is_some() {
        return None;
    }
    Some((name.to_string(), length))
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
    size_bounds: Option<(u64, u64)>,
    seeders: Option<u32>,
}

impl PartialItem {
    fn set(&mut self, field: Field, text: &str) {
        match field {
            Field::Title => self.title = Some(text.to_string()),
            Field::Link => self.link = Some(text.to_string()),
            Field::InfoHash => self.info_hash = Some(text.to_ascii_lowercase()),
            Field::Size => self.size_bounds = parse_size(text),
            Field::Seeders => self.seeders = text.trim().parse().ok(),
        }
    }

    fn take(&mut self) -> Option<NyaaItem> {
        Some(NyaaItem {
            title: self.title.take()?,
            torrent_url: self.link.take()?,
            info_hash: self.info_hash.take()?,
            size_bounds: self.size_bounds,
            seeders: self.seeders.unwrap_or(0),
        })
    }
}

/// Parse nyaa's human-formatted size ("1.4 GiB") into the bounds of
/// actual byte sizes that would display as that string. The display is
/// rounded to the shown decimals, and the rounding quantum dwarfs any
/// percentage tolerance at GiB scale (±0.05 GiB ≈ ±3.8% of "1.3 GiB"),
/// so the bounds — value ± half the last decimal's step — are what a
/// sanity check must compare against, never the midpoint alone.
pub fn parse_size(text: &str) -> Option<(u64, u64)> {
    let text = text.trim();
    let split = text.find(|c: char| c != '.' && !c.is_ascii_digit())?;
    let (number, unit) = text.split_at(split);
    let number = number.trim();
    let value: f64 = number.parse().ok()?;
    let scale: f64 = match unit.trim() {
        "B" | "Bytes" => 1.0,
        "KiB" => 1024.0,
        "MiB" => 1024.0 * 1024.0,
        "GiB" => 1024.0 * 1024.0 * 1024.0,
        "TiB" => 1024.0f64.powi(4),
        _ => return None,
    };
    let decimals = number
        .rsplit_once('.')
        .map_or(0, |(_, frac)| frac.len() as i32);
    let half_step = 10f64.powi(-decimals) * scale / 2.0;
    let bytes = value * scale;
    (bytes.is_finite() && bytes >= 0.0).then_some((
        (bytes - half_step).max(0.0) as u64,
        (bytes + half_step) as u64,
    ))
}

/// Extra slack beyond the display-rounding bounds: how far the entry's
/// exact byte size may sit outside the range nyaa's rounded size string
/// covers and still count as the same payload.
const SIZE_TOLERANCE: f64 = 0.03;

/// Pick the acceptable result for `filename`, or `None`.
///
/// Accepts an item whose title equals the filename (whitespace-normalized,
/// case-insensitive, extension optional), with at least one seeder, whose
/// size bounds (the display-rounding range, ±3% slack) contain
/// `size_bytes`, and whose info hash isn't banned (a previous download of
/// it failed ed2k verification). Ties break to the most seeders.
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
                && item.size_bounds.is_some_and(|(lo, hi)| {
                    let want = size_bytes as f64;
                    let slack = want * SIZE_TOLERANCE;
                    want >= lo as f64 - slack && want <= hi as f64 + slack
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
        assert_eq!(items[0].size_bounds, parse_size("1.4 GiB"));
        assert_eq!(items[0].seeders, 412);
        // Entity-escaped title round-trips.
        assert_eq!(
            items[2].title,
            "[Judas] Foo & Bar - 01 <v2> (1080p) [ABCD1234].mkv"
        );
    }

    #[test]
    fn parse_size_units() {
        let mib = 1024.0 * 1024.0;
        assert_eq!(
            parse_size("711.7 MiB"),
            Some((((711.7 - 0.05) * mib) as u64, ((711.7 + 0.05) * mib) as u64))
        );
        let gib = 1024.0f64.powi(3);
        assert_eq!(
            parse_size("1.4 GiB"),
            Some((((1.4 - 0.05) * gib) as u64, ((1.4 + 0.05) * gib) as u64))
        );
        // No decimals shown: half a unit either way.
        assert_eq!(parse_size("512 B"), Some((511, 512)));
        assert_eq!(parse_size("3 KiB"), Some((2560, 3584)));
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
            size_bounds: Some((1000, 1000)),
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
    fn accepts_a_size_hidden_by_display_rounding() {
        // Regression (2026-07-09, live): the real Clevatess S2-01 is
        // 1,447,979,541 bytes (1.348 GiB) but nyaa displays "1.3 GiB"
        // (1,395,864,371) — a 3.6% apparent deviation from rounding
        // alone, which the plain ±3% check rejected.
        let items = vec![NyaaItem {
            title: "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv".into(),
            torrent_url: "https://nyaa.si/download/2129710.torrent".into(),
            info_hash: "123051cef95247353e061c58ee1cb713691f72b4".into(),
            size_bounds: parse_size("1.3 GiB"),
            seeders: 1716,
        }];
        let m = pick_match(
            &items,
            "[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv",
            1_447_979_541,
            &HashSet::new(),
        );
        assert!(m.is_some());
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
            size_bounds: Some((size, size)),
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
            size_bounds: Some((1000, 1000)),
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

    #[test]
    fn anime_search_url_uses_anime_category() {
        let url = search_url_for("karen", NyaaCategory::Anime);
        assert_eq!(url, "https://nyaa.si/?page=rss&q=karen&c=1_0&f=0");
    }

    fn single_torrent(name: &str, length: u64) -> Vec<u8> {
        format!(
            "d4:infod6:lengthi{length}e4:name{}:{name}12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee",
            name.len()
        )
        .into_bytes()
    }

    fn multi_torrent() -> Vec<u8> {
        b"d4:infod5:filesld6:lengthi123e4:pathl8:file.mkveee4:name3:dir12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee".to_vec()
    }

    #[test]
    fn single_file_metadata_is_accepted_and_unsafe_names_are_rejected() {
        assert_eq!(
            single_file_payload(&single_torrent("file.mkv", 123)),
            Some(("file.mkv".to_string(), 123))
        );
        assert_eq!(
            single_file_payload(&single_torrent("../file.mkv", 123)),
            None
        );
        assert_eq!(
            single_file_payload(&single_torrent("dir\\file.mkv", 123)),
            None
        );
        assert_eq!(single_file_payload(&multi_torrent()), None);
    }

    struct BrowseSource {
        rss: String,
        torrents: std::collections::HashMap<String, Vec<u8>>,
    }

    impl NyaaSource for BrowseSource {
        fn search(&self, _filename: &str) -> std::io::Result<String> {
            Ok(self.rss.clone())
        }

        fn search_category(&self, _query: &str, category: NyaaCategory) -> std::io::Result<String> {
            assert_eq!(category, NyaaCategory::Anime);
            Ok(self.rss.clone())
        }

        fn fetch_torrent(&self, url: &str) -> std::io::Result<Vec<u8>> {
            self.torrents
                .get(url)
                .cloned()
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "missing"))
        }
    }

    #[test]
    fn browse_keeps_feed_order_and_only_single_file_results() {
        let item = |id: u8, seeders: u32| {
            format!(
                "<item><title>Release {id}</title><link>https://nyaa.si/download/{id}.torrent</link><nyaa:infoHash>{id:040}</nyaa:infoHash><nyaa:size>123 B</nyaa:size><nyaa:seeders>{seeders}</nyaa:seeders></item>"
            )
        };
        let rss = format!(
            r#"<rss xmlns:nyaa="https://nyaa.si/xmlns/nyaa"><channel>{}{}{}</channel></rss>"#,
            item(1, 10),
            item(2, 20),
            item(3, 0),
        );
        let source = BrowseSource {
            rss,
            torrents: [
                (
                    "https://nyaa.si/download/1.torrent".to_string(),
                    single_torrent("one.mkv", 123),
                ),
                (
                    "https://nyaa.si/download/2.torrent".to_string(),
                    multi_torrent(),
                ),
            ]
            .into(),
        };
        let results = browse_single_file_results(&source, "release", 20).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].filename, "one.mkv");
        assert_eq!(results[0].title, "Release 1");
    }

    #[test]
    fn browse_inspects_only_the_requested_prefix() {
        let mut items = String::new();
        let mut torrents = std::collections::HashMap::new();
        for id in 0..25 {
            let url = format!("https://nyaa.si/download/{id}.torrent");
            items.push_str(&format!(
                "<item><title>R{id}</title><link>{url}</link><nyaa:infoHash>{id:040}</nyaa:infoHash><nyaa:size>1 B</nyaa:size><nyaa:seeders>1</nyaa:seeders></item>"
            ));
            torrents.insert(url, single_torrent(&format!("{id}.mkv"), 1));
        }
        let source = BrowseSource {
            rss: format!(
                r#"<rss xmlns:nyaa="https://nyaa.si/xmlns/nyaa"><channel>{items}</channel></rss>"#
            ),
            torrents,
        };
        let results = browse_single_file_results(&source, "r", 20).unwrap();
        assert_eq!(results.len(), 20);
        assert_eq!(results.first().unwrap().filename, "0.mkv");
        assert_eq!(results.last().unwrap().filename, "19.mkv");
    }
}
