//! nyaa.si RSS search backing the Playlist pane's explicit browse
//! import (`n`).
//!
//! The query is free-form user text against the Anime category; each
//! candidate's `.torrent` metadata is inspected before display so only
//! safe single-file payloads are offered, and the selected payload is
//! still ed2k-hashed after download before it becomes a playlist entry.
//! Blocking by design — call from `spawn_blocking`, exactly like the
//! server's anime-titles fetch.

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
    /// Current seeder count.
    pub seeders: u32,
}

/// A selected search result, ready to hand to the torrent engine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NyaaMatch {
    /// Release title.
    pub title: String,
    /// The `.torrent` download URL.
    pub torrent_url: String,
    /// Info hash, lowercase hex (magnet fallback, dedup).
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

/// Source of raw RSS and torrent metadata for a browse search. Mocked in
/// tests; HTTP in production. Blocking by design — call from
/// `spawn_blocking`.
pub trait NyaaSource: Send + Sync + 'static {
    /// Fetch the RSS results for `query` in the Anime category (`c=1_0`).
    fn search_anime(&self, query: &str) -> std::io::Result<String>;

    /// Fetch raw `.torrent` metadata for browse-result inspection.
    fn fetch_torrent(&self, url: &str) -> std::io::Result<Vec<u8>>;
}

/// The real thing: one GET against nyaa.si.
pub struct HttpNyaaSource;

/// An agent with a hard 30s per-call timeout. ureq's default has *no*
/// timeout, so a connected-but-silent host would park the blocking
/// thread until TCP gives up, and leaked threads accumulate across
/// repeated searches.
pub(super) fn http_agent() -> ureq::Agent {
    ureq::Agent::from(
        ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build(),
    )
}

/// Build the RSS search URL for an Anime-category query.
pub fn anime_search_url(query: &str) -> String {
    format!(
        "https://nyaa.si/?page=rss&q={}&c=1_0&f=0",
        utf8_percent_encode(query, NON_ALPHANUMERIC),
    )
}

impl NyaaSource for HttpNyaaSource {
    fn search_anime(&self, query: &str) -> std::io::Result<String> {
        let response = http_agent()
            .get(anime_search_url(query))
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
        let response = http_agent()
            .get(url)
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
    let xml = source.search_anime(query)?;
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
    Seeders,
}

impl Field {
    fn from_tag(tag: &[u8]) -> Option<Self> {
        match tag {
            b"title" => Some(Self::Title),
            b"link" => Some(Self::Link),
            b"nyaa:infoHash" => Some(Self::InfoHash),
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
    seeders: Option<u32>,
}

impl PartialItem {
    fn set(&mut self, field: Field, text: &str) {
        match field {
            Field::Title => self.title = Some(text.to_string()),
            Field::Link => self.link = Some(text.to_string()),
            Field::InfoHash => self.info_hash = Some(text.to_ascii_lowercase()),
            Field::Seeders => self.seeders = text.trim().parse().ok(),
        }
    }

    fn take(&mut self) -> Option<NyaaItem> {
        Some(NyaaItem {
            title: self.title.take()?,
            torrent_url: self.link.take()?,
            info_hash: self.info_hash.take()?,
            seeders: self.seeders.unwrap_or(0),
        })
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
        assert_eq!(items[0].seeders, 412);
        // Entity-escaped title round-trips.
        assert_eq!(
            items[2].title,
            "[Judas] Foo & Bar - 01 <v2> (1080p) [ABCD1234].mkv"
        );
    }

    #[test]
    fn anime_search_url_percent_encodes() {
        let url = anime_search_url("[SubsPlease] Clevatess S2 - 01 (1080p) [6855D1F4].mkv");
        assert!(url.starts_with("https://nyaa.si/?page=rss&q="));
        assert!(url.ends_with("&c=1_0&f=0"));
        assert!(!url.contains('['));
        assert!(!url.contains(' '));
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
        fn search_anime(&self, _query: &str) -> std::io::Result<String> {
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
