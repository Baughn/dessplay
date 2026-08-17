//! The AniDB anime-titles dump: the sanctioned way to do name search.
//!
//! The UDP API has no multi-result search (`ANIME aname=` is an exact
//! lookup), so the server fetches `anime-titles.dat.gz` — published
//! daily, one download per day allowed — into SQLite and answers the
//! AniDbSearch modal locally. See docs/design.md.
//!
//! The fetch is one blocking GET per day, run on the blocking pool
//! (the short-title curator, `super::curator`, is the server's only
//! other HTTP).

use std::io::Read;

use dessplay_core::types::AniDbSeriesId;

use crate::storage::TitleRow;

/// Where the dump lives.
pub const TITLES_URL: &str = "https://anidb.net/api/anime-titles.dat.gz";
/// How often to refresh (the dump itself updates daily; fetching more
/// often than once a day is against AniDB's rules).
pub const REFRESH_MILLIS: u64 = 24 * 60 * 60 * 1000;
/// Retry delay after a failed fetch.
pub const RETRY_MILLIS: u64 = 60 * 60 * 1000;
/// The kv key recording the last successful fetch (shared-clock millis).
pub const FETCHED_AT_KEY: &str = "titles_fetched_at";

/// Source of the (decompressed) dump text. Mocked in tests; HTTP in
/// production. Blocking by design — call from `spawn_blocking`.
pub trait TitlesSource: Send + Sync + 'static {
    /// Fetch and decompress the dump.
    fn fetch(&self) -> std::io::Result<String>;
}

/// The real thing: GET + gunzip.
pub struct HttpTitlesSource;

impl TitlesSource for HttpTitlesSource {
    fn fetch(&self) -> std::io::Result<String> {
        let response = ureq::get(TITLES_URL)
            .header("User-Agent", "dessplay/1")
            .call()
            .map_err(std::io::Error::other)?;
        let compressed = response
            .into_body()
            .with_config()
            // The dump is ~3MB gzipped; anything past 64MB decompressed
            // is not the file we asked for.
            .limit(64 * 1024 * 1024)
            .read_to_vec()
            .map_err(std::io::Error::other)?;
        let mut text = String::new();
        flate2::read::GzDecoder::new(compressed.as_slice()).read_to_string(&mut text)?;
        Ok(text)
    }
}

/// Parse the dump: `aid|type|language|title` lines, `#` comments.
/// Unparsable lines are skipped (the dump is machine-generated; a bad
/// line means a format drift we'd rather survive than die on).
pub fn parse_dump(text: &str) -> Vec<TitleRow> {
    text.lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.splitn(4, '|');
            let aid: u32 = parts.next()?.parse().ok()?;
            let kind: u8 = parts.next()?.parse().ok()?;
            let lang = parts.next()?;
            let title = parts.next()?;
            (!title.is_empty()).then(|| TitleRow {
                series: AniDbSeriesId(aid),
                kind,
                lang: lang.to_string(),
                title: title.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn parses_the_dump_format() {
        let dump = "\
# anime-titles.dat
# generated on ...
1|1|x-jat|Seikai no Monshou
1|2|en|Crest of the Stars
8692|1|x-jat|Sousou no Frieren
8692|4|en|Frieren: Beyond Journey's End
bogus line
99|notanumber|en|skipped
100|1|ja|
";
        let rows = parse_dump(dump);
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].series, AniDbSeriesId(1));
        assert_eq!(rows[0].kind, 1);
        assert_eq!(rows[0].lang, "x-jat");
        assert_eq!(rows[0].title, "Seikai no Monshou");
        assert_eq!(rows[3].kind, 4);
        assert_eq!(rows[3].title, "Frieren: Beyond Journey's End");
    }

    #[test]
    fn titles_may_contain_pipes() {
        // splitn(4) keeps everything after the third separator.
        let rows = parse_dump("5|1|en|A|B|C");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "A|B|C");
    }
}
