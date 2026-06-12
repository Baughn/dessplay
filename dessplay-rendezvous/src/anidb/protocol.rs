//! Pure codec for the AniDB UDP API (wiki.anidb.net/UDP_API_Definition).
//!
//! Requests are `COMMAND key=value&key=value` ASCII lines; responses are
//! `[tag ]code TEXT` followed by pipe-separated data lines. Everything
//! here is offline-testable: no sockets, no clocks.
//!
//! The fmask/amask bit tables were cross-checked against two independent
//! client implementations (adbb, anidbcli) because the official wiki is
//! behind an interactive challenge. Each mask constant has a test
//! spelling out the named bits it is built from.

use dessplay_core::types::{AniDbSeriesId, Ed2kHash, RelationKind};

/// Registered client name. Do not change without re-registering on
/// anidb.net.
pub const CLIENT_NAME: &str = "dessplay";
/// Registered client version.
pub const CLIENT_VERSION: u32 = 1;
/// UDP API protocol version.
pub const PROTOVER: u32 = 3;

// ---- Return codes (the subset we react to).

/// AUTH succeeded.
pub const LOGIN_ACCEPTED: u16 = 200;
/// AUTH succeeded; a newer client version is registered.
pub const LOGIN_ACCEPTED_NEW_VERSION: u16 = 201;
/// LOGOUT succeeded.
pub const LOGGED_OUT: u16 = 203;
/// FILE hit.
pub const FILE: u16 = 220;
/// PING reply.
pub const PONG: u16 = 300;
/// ANIME hit.
pub const ANIME: u16 = 230;
/// FILE miss: AniDB does not know this (size, ed2k).
pub const NO_SUCH_FILE: u16 = 320;
/// FILE by non-hash identifiers matched several files. Cannot happen
/// for size+ed2k lookups; treated as a miss if it somehow does.
pub const MULTIPLE_FILES_FOUND: u16 = 322;
/// ANIME miss.
pub const NO_SUCH_ANIME: u16 = 330;
/// LOGOUT without a session.
pub const NOT_LOGGED_IN: u16 = 403;
/// Bad credentials. Fatal: do not retry with the same credentials.
pub const LOGIN_FAILED: u16 = 500;
/// Session-bearing command without a valid session: re-AUTH.
pub const LOGIN_FIRST: u16 = 501;
/// Access denied.
pub const ACCESS_DENIED: u16 = 502;
/// Client protocol version too old. Fatal until the code is updated.
pub const CLIENT_VERSION_OUTDATED: u16 = 503;
/// This client (name) is banned. Fatal.
pub const CLIENT_BANNED: u16 = 504;
/// Malformed command. A bug on our side.
pub const ILLEGAL_INPUT: u16 = 505;
/// Session expired or unknown: re-AUTH.
pub const INVALID_SESSION: u16 = 506;
/// This user is banned (usually flood protection). Long backoff.
pub const BANNED: u16 = 555;
/// Command unknown to the server.
pub const UNKNOWN_COMMAND: u16 = 598;
/// Server-side error. Back off.
pub const INTERNAL_SERVER_ERROR: u16 = 600;
/// API disabled for maintenance. Back off (the wiki suggests 30 min).
pub const OUT_OF_SERVICE: u16 = 601;
/// Server overloaded — the "throttled" reply. Back off; these still
/// count against the rate limit.
pub const SERVER_BUSY: u16 = 602;

// ---- Masks.
//
// FILE fmask, 5 bytes, fields returned in bit order (MSB of byte 1
// first), after the always-present fid:
//   byte 1: -, aid, eid, gid, lid, other-eps, is-deprecated, state
//   byte 2: size, ed2k, md5, sha1, crc32, -, video-depth, -
//   byte 3: quality, source, audio-codecs, audio-bitrates, video-codec,
//           video-bitrate, resolution, filetype
//   byte 4: dub-langs, sub-langs, length, description, aired, -, -, filename
//   byte 5: mylist fields
//
// FILE amask, 4 bytes:
//   byte 1: total-eps, highest-ep, year, type, related-aids,
//           related-types, categories, -
//   byte 2: romaji, kanji, english, other, short-names, synonyms, -, -
//   byte 3: epno, ep-name, ep-romaji, ep-kanji, ep-rating, ep-votes, -, -
//   byte 4: group-name, group-short, -, -, -, -, -, updated
//
// ANIME amask, 7 bytes:
//   byte 1: aid, dateflags, year, type, related-aids, related-types, -, -
//   byte 2: romaji, kanji, english, other, short-names, synonyms, -, -
//   byte 3: episodes, highest-ep, special-count, air-date, end-date,
//           url, picname, -
//   bytes 4-7: ratings / external ids / character ids / counts (unused)

/// FILE fmask: `aid` only (fid is always returned first).
pub const FILE_FMASK: &str = "4000000000";
/// FILE amask: romaji name, english name, epno.
pub const FILE_AMASK: &str = "00A08000";
/// Fields of a [`FILE`] data line under [`FILE_FMASK`]/[`FILE_AMASK`]:
/// fid, aid, romaji, english, epno.
pub const FILE_FIELDS: usize = 5;

/// ANIME amask: aid, year, related aid list, related aid types,
/// romaji name, english name, episode count.
pub const ANIME_AMASK: &str = "ACA08000000000";
/// Fields of an [`ANIME`] data line under [`ANIME_AMASK`], in order:
/// aid, year, related aid list, related aid types, romaji, english,
/// episode count.
pub const ANIME_FIELDS: usize = 7;

/// Codec errors. Always a sign of a parser bug, a protocol change, or a
/// mangled datagram — never user error.
#[derive(Debug, PartialEq, Eq)]
pub enum ProtocolError {
    /// The response didn't start with `[tag ]<3-digit code>`.
    MalformedHeader(String),
    /// A data line had the wrong number of fields for the command.
    WrongFieldCount {
        /// What the mask promised.
        expected: usize,
        /// What arrived.
        got: usize,
    },
    /// A field that must be numeric wasn't.
    BadNumber(String),
    /// A response that needs a data line didn't have one.
    MissingDataLine,
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::MalformedHeader(line) => write!(f, "malformed response header {line:?}"),
            ProtocolError::WrongFieldCount { expected, got } => {
                write!(f, "expected {expected} fields, got {got}")
            }
            ProtocolError::BadNumber(field) => write!(f, "expected a number, got {field:?}"),
            ProtocolError::MissingDataLine => write!(f, "response is missing its data line"),
        }
    }
}

impl std::error::Error for ProtocolError {}

// ---- Requests.

/// Escape an outgoing parameter value. `&` would terminate the value;
/// newlines would terminate the command.
fn escape_value(value: &str) -> String {
    value.replace('&', "&amp;").replace('\n', "<br />")
}

/// Assemble `COMMAND k=v&k=v`. Parameter order is preserved (AniDB
/// doesn't care, but deterministic output keeps tests simple).
fn encode(command: &str, params: &[(&str, &str)]) -> String {
    let joined = params
        .iter()
        .map(|(k, v)| format!("{k}={}", escape_value(v)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{command} {joined}")
}

/// AUTH. The one command that carries credentials and no session.
pub fn auth(user: &str, pass: &str, tag: &str) -> String {
    let protover = PROTOVER.to_string();
    let clientver = CLIENT_VERSION.to_string();
    encode(
        "AUTH",
        &[
            ("user", user),
            ("pass", pass),
            ("protover", &protover),
            ("client", CLIENT_NAME),
            ("clientver", &clientver),
            ("enc", "UTF8"),
            ("tag", tag),
        ],
    )
}

/// LOGOUT the given session.
pub fn logout(session: &str, tag: &str) -> String {
    encode("LOGOUT", &[("s", session), ("tag", tag)])
}

/// PING (works without a session).
pub fn ping(tag: &str) -> String {
    encode("PING", &[("tag", tag)])
}

/// FILE lookup by (size, ed2k) with our masks.
pub fn file_by_hash(size: u64, hash: Ed2kHash, session: &str, tag: &str) -> String {
    let size = size.to_string();
    let ed2k = hash.to_string();
    encode(
        "FILE",
        &[
            ("size", &size),
            ("ed2k", &ed2k),
            ("fmask", FILE_FMASK),
            ("amask", FILE_AMASK),
            ("s", session),
            ("tag", tag),
        ],
    )
}

/// ANIME lookup by aid with our mask.
pub fn anime_by_id(aid: AniDbSeriesId, session: &str, tag: &str) -> String {
    let aid = aid.0.to_string();
    encode(
        "ANIME",
        &[
            ("aid", &aid),
            ("amask", ANIME_AMASK),
            ("s", session),
            ("tag", tag),
        ],
    )
}

// ---- Responses.

/// A parsed response. Data-line fields are raw (still escaped): list
/// fields use `'` as a separator and escape content apostrophes as
/// backticks, so unescaping must happen *after* list splitting. Use
/// [`unescape`] on scalar fields.
#[derive(Debug, PartialEq, Eq)]
pub struct Response {
    /// Echo of the request's `tag` parameter, if it was tagged.
    pub tag: Option<String>,
    /// The 3-digit return code.
    pub code: u16,
    /// The rest of the header line ("LOGIN ACCEPTED", "FILE", ...).
    /// For AUTH replies this starts with the session key.
    pub text: String,
    /// Data lines, split on `|`, fields raw.
    pub lines: Vec<Vec<String>>,
}

/// Parse a raw UDP reply.
pub fn parse_response(raw: &str) -> Result<Response, ProtocolError> {
    let mut lines = raw.lines();
    let header = lines
        .next()
        .ok_or_else(|| ProtocolError::MalformedHeader(String::new()))?;

    // `tag code TEXT` or `code TEXT`. The tag is whatever we sent; we
    // distinguish by the code being exactly three digits.
    let (first, rest) = header
        .split_once(' ')
        .ok_or_else(|| ProtocolError::MalformedHeader(header.into()))?;
    let (tag, code_str, text) = if is_code(first) {
        (None, first, rest)
    } else {
        let (code_str, text) = rest.split_once(' ').unwrap_or((rest, ""));
        (Some(first.to_string()), code_str, text)
    };
    if !is_code(code_str) {
        return Err(ProtocolError::MalformedHeader(header.into()));
    }
    let code: u16 = code_str
        .parse()
        .map_err(|_| ProtocolError::MalformedHeader(header.into()))?;

    let data = lines
        .filter(|line| !line.is_empty())
        .map(|line| line.split('|').map(str::to_string).collect())
        .collect();
    Ok(Response {
        tag,
        code,
        text: text.to_string(),
        lines: data,
    })
}

fn is_code(token: &str) -> bool {
    token.len() == 3 && token.bytes().all(|b| b.is_ascii_digit())
}

/// Unescape a scalar content field: `<br />` is a newline, a backtick
/// is an escaped apostrophe (`'` itself is the list separator).
///
/// Content pipes arrive as `/` — the server's escaping is lossy and is
/// deliberately *not* reversed: "Fate/stay night" must keep its slash.
pub fn unescape(field: &str) -> String {
    field.replace("<br />", "\n").replace('`', "'")
}

/// The session key from a [`LOGIN_ACCEPTED`] / [`LOGIN_ACCEPTED_NEW_VERSION`]
/// header text (`"xK3fp LOGIN ACCEPTED"`).
pub fn session_key(text: &str) -> Option<String> {
    let key = text.split(' ').next()?;
    (!key.is_empty() && key.bytes().all(|b| b.is_ascii_alphanumeric()))
        .then(|| key.to_string())
}

/// A [`FILE`] hit under our masks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResult {
    /// AniDB file id (always first in a FILE reply).
    pub fid: u32,
    /// The anime the file belongs to.
    pub aid: AniDbSeriesId,
    /// Series romaji title (the group's display convention).
    pub romaji: String,
    /// Series english title.
    pub english: String,
    /// Episode number — a string by design: "01", "S1", "C2", "T1"...
    pub epno: String,
}

impl FileResult {
    /// The display name: romaji, falling back to english.
    pub fn series_name(&self) -> &str {
        if self.romaji.is_empty() {
            &self.english
        } else {
            &self.romaji
        }
    }
}

/// Parse the data line of a [`FILE`] response.
pub fn parse_file_data(response: &Response) -> Result<FileResult, ProtocolError> {
    let line = response.lines.first().ok_or(ProtocolError::MissingDataLine)?;
    if line.len() != FILE_FIELDS {
        return Err(ProtocolError::WrongFieldCount {
            expected: FILE_FIELDS,
            got: line.len(),
        });
    }
    Ok(FileResult {
        fid: parse_number(&line[0])?,
        aid: AniDbSeriesId(parse_number(&line[1])?),
        romaji: unescape(&line[2]),
        english: unescape(&line[3]),
        epno: unescape(&line[4]),
    })
}

/// An [`ANIME`] hit under our mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimeResult {
    /// The anime id (echoed back; trust it over the request's).
    pub aid: AniDbSeriesId,
    /// First air year, if AniDB knows it ("1999-2000" yields 1999).
    pub year: Option<u16>,
    /// Related anime: (raw relation code, target aid) pairs.
    pub relations: Vec<(u16, AniDbSeriesId)>,
    /// Romaji title.
    pub romaji: String,
    /// English title.
    pub english: String,
    /// Episode count; `None` when unknown or 0 (still airing).
    pub episode_count: Option<u32>,
}

impl AnimeResult {
    /// The display title: romaji, falling back to english.
    pub fn title(&self) -> &str {
        if self.romaji.is_empty() {
            &self.english
        } else {
            &self.romaji
        }
    }
}

/// Parse the data line of an [`ANIME`] response.
pub fn parse_anime_data(response: &Response) -> Result<AnimeResult, ProtocolError> {
    let line = response.lines.first().ok_or(ProtocolError::MissingDataLine)?;
    if line.len() != ANIME_FIELDS {
        return Err(ProtocolError::WrongFieldCount {
            expected: ANIME_FIELDS,
            got: line.len(),
        });
    }
    // List fields: `'`-separated, before any unescaping. The two lists
    // are parallel arrays; a length mismatch keeps the shorter prefix.
    let targets: Vec<u32> = split_list(&line[2])
        .map(parse_number)
        .collect::<Result<_, _>>()?;
    let kinds: Vec<u16> = split_list(&line[3])
        .map(parse_number)
        .collect::<Result<_, _>>()?;
    let relations = kinds
        .into_iter()
        .zip(targets)
        .map(|(kind, aid)| (kind, AniDbSeriesId(aid)))
        .collect();
    let episodes: u32 = parse_number(&line[6]).unwrap_or(0);
    Ok(AnimeResult {
        aid: AniDbSeriesId(parse_number(&line[0])?),
        year: parse_year(&line[1]),
        relations,
        romaji: unescape(&line[4]),
        english: unescape(&line[5]),
        episode_count: (episodes > 0).then_some(episodes),
    })
}

/// Split an apostrophe-separated list field; empty fields are empty
/// lists, not a list of one empty item.
fn split_list(field: &str) -> impl Iterator<Item = &str> {
    field.split('\'').filter(|item| !item.is_empty())
}

fn parse_number<N: std::str::FromStr>(field: &str) -> Result<N, ProtocolError> {
    field
        .parse()
        .map_err(|_| ProtocolError::BadNumber(field.to_string()))
}

/// First year of a year field: "2004", "1999-2000", "?"...
fn parse_year(field: &str) -> Option<u16> {
    let digits: String = field.chars().take_while(|c| c.is_ascii_digit()).collect();
    (digits.len() == 4).then(|| digits.parse().ok()).flatten()
}

/// Map an AniDB relation code to our [`RelationKind`]. Codes verified
/// against adbb's table; unknown codes are preserved as `Other`.
pub fn relation_kind(code: u16) -> RelationKind {
    match code {
        1 => RelationKind::Sequel,
        2 => RelationKind::Prequel,
        11 => RelationKind::SameSetting,
        12 | 21 | 22 => RelationKind::AlternativeSetting,
        31 | 32 => RelationKind::AlternativeVersion,
        41 => RelationKind::MusicVideo,
        42 => RelationKind::Character,
        51 => RelationKind::SideStory,
        52 => RelationKind::ParentStory,
        61 => RelationKind::Summary,
        62 => RelationKind::FullStory,
        other => RelationKind::Other(other),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Build a mask string from MSB-first bit positions, documenting
    /// exactly which named bits each constant is made of.
    fn mask(bytes: usize, bits: &[usize]) -> String {
        let mut value: u64 = 0;
        for &bit in bits {
            value |= 1 << (bytes * 8 - 1 - bit);
        }
        format!("{value:0width$X}", width = bytes * 2)
    }

    #[test]
    fn file_fmask_is_aid_only() {
        // fmask byte 1: [unused, aid, eid, gid, lid, other-eps,
        // deprecated, state] — aid is bit index 1 from the MSB.
        assert_eq!(FILE_FMASK, mask(5, &[1]));
    }

    #[test]
    fn file_amask_is_romaji_english_epno() {
        // amask byte 2 starts at index 8: [romaji, kanji, english, ...];
        // byte 3 starts at index 16: [epno, ...].
        assert_eq!(FILE_AMASK, mask(4, &[8, 10, 16]));
    }

    #[test]
    fn anime_amask_fields() {
        // byte 1: [aid, dateflags, year, type, related-aids,
        //          related-types, -, -]
        // byte 2: [romaji, kanji, english, ...]
        // byte 3: [episodes, ...]
        assert_eq!(ANIME_AMASK, mask(7, &[0, 2, 4, 5, 8, 10, 16]));
    }

    #[test]
    fn auth_includes_registration_and_encoding() {
        let cmd = auth("baughn", "hunter2", "t1");
        assert_eq!(
            cmd,
            "AUTH user=baughn&pass=hunter2&protover=3&client=dessplay&clientver=1&enc=UTF8&tag=t1"
        );
    }

    #[test]
    fn outgoing_ampersands_are_escaped() {
        let cmd = auth("a&b", "p&q", "t");
        assert!(cmd.contains("user=a&amp;b"));
        assert!(cmd.contains("pass=p&amp;q"));
    }

    #[test]
    fn file_command_format() {
        let hash = Ed2kHash([0xAB; 16]);
        let cmd = file_by_hash(734003200, hash, "sess1", "t42");
        assert_eq!(
            cmd,
            format!(
                "FILE size=734003200&ed2k={}&fmask={FILE_FMASK}&amask={FILE_AMASK}&s=sess1&tag=t42",
                "ab".repeat(16)
            )
        );
    }

    #[test]
    fn anime_command_format() {
        let cmd = anime_by_id(AniDbSeriesId(8692), "sess1", "t7");
        assert_eq!(cmd, format!("ANIME aid=8692&amask={ANIME_AMASK}&s=sess1&tag=t7"));
    }

    #[test]
    fn logout_and_ping() {
        assert_eq!(logout("sess1", "t9"), "LOGOUT s=sess1&tag=t9");
        assert_eq!(ping("t0"), "PING tag=t0");
    }

    #[test]
    fn parses_tagged_response() {
        let response = parse_response("t42 220 FILE\n312498|8692|Sousou no Frieren|Frieren: Beyond Journey's End|01\n").unwrap();
        assert_eq!(response.tag.as_deref(), Some("t42"));
        assert_eq!(response.code, FILE);
        assert_eq!(response.text, "FILE");
        assert_eq!(response.lines.len(), 1);
        assert_eq!(response.lines[0].len(), 5);
    }

    #[test]
    fn parses_untagged_response() {
        let response = parse_response("300 PONG").unwrap();
        assert_eq!(response.tag, None);
        assert_eq!(response.code, PONG);
        assert_eq!(response.text, "PONG");
        assert!(response.lines.is_empty());
    }

    #[test]
    fn parses_auth_response_and_session_key() {
        let response = parse_response("t1 200 xK3fp LOGIN ACCEPTED").unwrap();
        assert_eq!(response.code, LOGIN_ACCEPTED);
        assert_eq!(session_key(&response.text).as_deref(), Some("xK3fp"));
    }

    #[test]
    fn session_key_rejects_garbage() {
        assert_eq!(session_key(""), None);
        assert_eq!(session_key("LOGIN|ACCEPTED"), None);
    }

    #[test]
    fn rejects_malformed_headers() {
        assert!(parse_response("").is_err());
        assert!(parse_response("FILE").is_err());
        assert!(parse_response("hello world").is_err());
        assert!(parse_response("t1 t2 220 FILE").is_err());
    }

    #[test]
    fn code_only_header_with_tag() {
        // LOGOUT replies can be bare: "t3 203 LOGGED OUT".
        let response = parse_response("t3 203 LOGGED OUT").unwrap();
        assert_eq!(response.tag.as_deref(), Some("t3"));
        assert_eq!(response.code, LOGGED_OUT);
    }

    #[test]
    fn file_data_round_trip() {
        let response = parse_response(
            "t1 220 FILE\n312498|8692|Sousou no Frieren|Frieren: Beyond Journey`s End|01\n",
        )
        .unwrap();
        let file = parse_file_data(&response).unwrap();
        assert_eq!(file.fid, 312498);
        assert_eq!(file.aid, AniDbSeriesId(8692));
        assert_eq!(file.romaji, "Sousou no Frieren");
        // Backtick unescapes to an apostrophe.
        assert_eq!(file.english, "Frieren: Beyond Journey's End");
        assert_eq!(file.epno, "01");
        assert_eq!(file.series_name(), "Sousou no Frieren");
    }

    #[test]
    fn file_special_episode_numbers_stay_strings() {
        let response = parse_response("t1 220 FILE\n1|2|A|B|S1\n").unwrap();
        assert_eq!(parse_file_data(&response).unwrap().epno, "S1");
    }

    #[test]
    fn file_data_field_count_is_checked() {
        let response = parse_response("t1 220 FILE\n312498|8692|only|four\n").unwrap();
        assert_eq!(
            parse_file_data(&response),
            Err(ProtocolError::WrongFieldCount {
                expected: 5,
                got: 4
            })
        );
        let no_data = parse_response("t1 220 FILE").unwrap();
        assert_eq!(parse_file_data(&no_data), Err(ProtocolError::MissingDataLine));
    }

    #[test]
    fn anime_data_round_trip() {
        let response = parse_response(
            "t2 230 ANIME\n8692|2023-2024|13310'17617|2'1|Sousou no Frieren|Frieren: Beyond Journey`s End|28\n",
        )
        .unwrap();
        let anime = parse_anime_data(&response).unwrap();
        assert_eq!(anime.aid, AniDbSeriesId(8692));
        assert_eq!(anime.year, Some(2023));
        assert_eq!(
            anime.relations,
            vec![
                (2, AniDbSeriesId(13310)),
                (1, AniDbSeriesId(17617)),
            ]
        );
        assert_eq!(anime.romaji, "Sousou no Frieren");
        assert_eq!(anime.english, "Frieren: Beyond Journey's End");
        assert_eq!(anime.episode_count, Some(28));
        assert_eq!(anime.title(), "Sousou no Frieren");
    }

    #[test]
    fn anime_with_no_relations_or_year() {
        let response = parse_response("t2 230 ANIME\n123|?|||Title|English|0\n").unwrap();
        let anime = parse_anime_data(&response).unwrap();
        assert_eq!(anime.year, None);
        assert!(anime.relations.is_empty());
        // Episode count 0 means "unknown / still airing".
        assert_eq!(anime.episode_count, None);
    }

    #[test]
    fn slashes_in_titles_survive() {
        // The server escapes content pipes as `/`, which makes `/`
        // ambiguous; reversing it would corrupt real slashes. We keep
        // them as-is.
        let response = parse_response("t2 220 FILE\n1|2|Fate/stay night|Fate/stay night|01\n").unwrap();
        assert_eq!(parse_file_data(&response).unwrap().romaji, "Fate/stay night");
    }

    #[test]
    fn br_unescapes_to_newline() {
        assert_eq!(unescape("line one<br />line two"), "line one\nline two");
    }

    #[test]
    fn relation_kinds_map() {
        use RelationKind::*;
        assert_eq!(relation_kind(1), Sequel);
        assert_eq!(relation_kind(2), Prequel);
        assert_eq!(relation_kind(11), SameSetting);
        assert_eq!(relation_kind(12), AlternativeSetting);
        assert_eq!(relation_kind(31), AlternativeVersion);
        assert_eq!(relation_kind(41), MusicVideo);
        assert_eq!(relation_kind(42), Character);
        assert_eq!(relation_kind(51), SideStory);
        assert_eq!(relation_kind(52), ParentStory);
        assert_eq!(relation_kind(61), Summary);
        assert_eq!(relation_kind(62), FullStory);
        assert_eq!(relation_kind(100), Other(100));
    }

    #[test]
    fn year_parsing() {
        assert_eq!(parse_year("2004"), Some(2004));
        assert_eq!(parse_year("1999-2000"), Some(1999));
        assert_eq!(parse_year("?"), None);
        assert_eq!(parse_year(""), None);
        assert_eq!(parse_year("204"), None);
    }
}
