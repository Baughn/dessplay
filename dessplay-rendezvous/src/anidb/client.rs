//! AniDB UDP API client with rate limiting and session management.
//!
//! Protocol reference: <https://wiki.anidb.net/UDP_API_Definition>

use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::net::UdpSocket;
use tokio::time::Instant;

use dessplay_core::types::{AniDbMetadata, FileId};

// ---------------------------------------------------------------------------
// Rate limiter
// ---------------------------------------------------------------------------

/// Enforces AniDB's rate limits: minimum 4s between packets,
/// 5s penalty on throttle (550) response.
pub struct RateLimiter {
    next_allowed: Instant,
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self {
            next_allowed: Instant::now(),
        }
    }
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Wait until the next packet is allowed to be sent.
    pub async fn wait_until_ready(&self) {
        tokio::time::sleep_until(self.next_allowed).await;
    }

    /// Record that a packet was just sent.
    pub fn record_sent(&mut self) {
        self.next_allowed = Instant::now() + Duration::from_secs(4);
    }

    /// Record a throttle response — apply 5s penalty from now.
    pub fn record_throttle(&mut self) {
        self.next_allowed = Instant::now() + Duration::from_secs(5);
    }
}

// ---------------------------------------------------------------------------
// AniDB session
// ---------------------------------------------------------------------------

const ANIDB_API_HOST: &str = "api.anidb.net:9000";
const CLIENT_NAME: &str = "dessplay";
const CLIENT_VERSION: u32 = 1;
const RECV_TIMEOUT: Duration = Duration::from_secs(10);
/// Send a keepalive if no activity for 25 minutes.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(25 * 60);

/// Active session with the AniDB UDP API.
pub struct AniDbSession {
    socket: UdpSocket,
    session_key: Option<String>,
    username: String,
    password: String,
    rate_limiter: RateLimiter,
    last_activity: Instant,
    banned: bool,
}

/// Result of a file lookup.
#[derive(Debug)]
pub enum LookupResult {
    /// File found with metadata.
    Found(AniDbMetadata),
    /// File not in AniDB database.
    NotFound,
    /// Server banned us — stop all queries.
    Banned,
}

impl AniDbSession {
    /// Create a new session (does not log in yet).
    pub async fn new(username: String, password: String) -> Result<Self> {
        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .context("failed to bind UDP socket for AniDB")?;
        socket
            .connect(ANIDB_API_HOST)
            .await
            .context("failed to connect UDP socket to AniDB")?;
        Ok(Self {
            socket,
            session_key: None,
            username,
            password,
            rate_limiter: RateLimiter::new(),
            last_activity: Instant::now(),
            banned: false,
        })
    }

    /// Ensure we have an active session (login if needed, keepalive if stale).
    pub async fn ensure_session(&mut self) -> Result<()> {
        if self.banned {
            bail!("AniDB session is banned; cannot send commands");
        }

        if self.session_key.is_none() {
            return self.login().await;
        }

        // Send keepalive if approaching session timeout
        if self.last_activity.elapsed() >= KEEPALIVE_INTERVAL {
            tracing::debug!("AniDB session stale, sending keepalive (UPTIME)");
            let response = self.send_command("UPTIME").await?;
            let code = parse_response_code(&response);
            tracing::debug!(code, "AniDB keepalive response");
        }

        Ok(())
    }

    /// Log in to AniDB.
    async fn login(&mut self) -> Result<()> {
        let cmd = format!(
            "AUTH user={}&pass={}&protover=3&client={}&clientver={}&enc=UTF-8",
            self.username, self.password, CLIENT_NAME, CLIENT_VERSION,
        );
        let response = self.send_command(&cmd).await?;
        let code = parse_response_code(&response);

        match code {
            200 | 201 => {
                // "200 {session_key} LOGIN ACCEPTED" or "201 ... NEW VERSION AVAILABLE"
                let session_key = response
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                tracing::info!("AniDB login successful");
                self.session_key = Some(session_key);
                Ok(())
            }
            500 => bail!("AniDB login failed: bad credentials"),
            503 => bail!("AniDB: client version outdated"),
            504 => bail!("AniDB: client banned"),
            _ => bail!("AniDB login unexpected response code {code}: {response}"),
        }
    }

    /// Log out from AniDB (best-effort).
    pub async fn logout(&mut self) {
        if self.session_key.is_some() {
            let _ = self.send_command("LOGOUT").await;
            self.session_key = None;
        }
    }

    /// Look up a file by its ed2k hash and file size.
    ///
    /// FILE command with:
    ///   fmask=4000000000 (aid)
    ///   amask=2CA0C040 (year, related_aid_list, related_aid_type, romaji_name,
    ///                    english_name, epno, ep_name, group_short_name)
    pub async fn lookup_file(
        &mut self,
        file_id: &FileId,
        file_size: u64,
    ) -> Result<LookupResult> {
        self.ensure_session().await?;

        let ed2k_hex = format!("{file_id}");
        let session = self
            .session_key
            .as_ref()
            .context("no session key")?
            .clone();
        let cmd = format!(
            "FILE size={file_size}&ed2k={ed2k_hex}&fmask=4000000000&amask=2CA0C040&s={session}"
        );

        let response = self.send_command(&cmd).await?;
        let code = parse_response_code(&response);

        match code {
            220 => {
                // FILE response: "220 FILE\n{fields}"
                let data_line = response
                    .lines()
                    .nth(1)
                    .context("missing data line in FILE response")?;
                let metadata = parse_file_response(data_line)?;
                Ok(LookupResult::Found(metadata))
            }
            320 => {
                // NO SUCH FILE
                Ok(LookupResult::NotFound)
            }
            501 => {
                // LOGIN FIRST or session expired — re-login and retry once
                tracing::info!("AniDB session expired (501), re-logging in");
                self.session_key = None;
                self.login().await?;
                // Retry the lookup
                let session = self
                    .session_key
                    .as_ref()
                    .context("no session key after re-login")?
                    .clone();
                let cmd = format!(
                    "FILE size={file_size}&ed2k={ed2k_hex}&fmask=4000000000&amask=2CA0C040&s={session}"
                );
                let response = self.send_command(&cmd).await?;
                let code = parse_response_code(&response);
                match code {
                    220 => {
                        let data_line = response
                            .lines()
                            .nth(1)
                            .context("missing data line in FILE response")?;
                        let metadata = parse_file_response(data_line)?;
                        Ok(LookupResult::Found(metadata))
                    }
                    320 => Ok(LookupResult::NotFound),
                    _ => bail!("AniDB FILE unexpected response after re-login: {code}: {response}"),
                }
            }
            550 => {
                tracing::warn!("AniDB throttled (550)");
                self.rate_limiter.record_throttle();
                bail!("AniDB throttled");
            }
            555 => {
                tracing::error!("AniDB BANNED (555) — stopping all queries");
                self.banned = true;
                Ok(LookupResult::Banned)
            }
            _ => bail!("AniDB FILE unexpected response code {code}: {response}"),
        }
    }

    /// Send a raw command to AniDB and return the response.
    async fn send_command(&mut self, cmd: &str) -> Result<String> {
        self.rate_limiter.wait_until_ready().await;

        tracing::debug!(cmd = cmd.split('&').next().unwrap_or(cmd), "AniDB send");
        self.socket
            .send(cmd.as_bytes())
            .await
            .context("failed to send UDP packet to AniDB")?;
        self.rate_limiter.record_sent();
        self.last_activity = Instant::now();

        let mut buf = vec![0u8; 4096];
        let n = tokio::time::timeout(RECV_TIMEOUT, self.socket.recv(&mut buf))
            .await
            .context("AniDB recv timeout")?
            .context("AniDB recv error")?;

        let response = String::from_utf8_lossy(&buf[..n]).into_owned();
        tracing::debug!(
            code = parse_response_code(&response),
            "AniDB recv"
        );
        Ok(response)
    }

    /// Returns true if the server has banned us.
    pub fn is_banned(&self) -> bool {
        self.banned
    }
}

// ---------------------------------------------------------------------------
// Response parsing
// ---------------------------------------------------------------------------

/// Extract the numeric response code from the first line.
fn parse_response_code(response: &str) -> u16 {
    response
        .split_whitespace()
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Parse AniDB pipe-delimited FILE response data line.
///
/// Expected fields (from our fmask/amask):
///   fid | aid | year | related_aid_list | related_aid_type | romaji_name | english_name | epno | ep_name | group_short_name
///
/// AniDB uses backtick (`) to escape literal pipe characters in field values.
fn parse_file_response(data: &str) -> Result<AniDbMetadata> {
    let fields = split_anidb_fields(data);
    if fields.len() < 10 {
        bail!(
            "AniDB FILE response has {} fields, expected 10: {data}",
            fields.len()
        );
    }

    // fid is fields[0] — we don't need it
    let aid: u64 = fields[1]
        .parse()
        .with_context(|| format!("failed to parse aid: '{}'", fields[1]))?;
    let year_str = &fields[2];
    let related_aid_list = &fields[3];
    let related_aid_type = &fields[4];
    let romaji_name = &fields[5];
    let english_name = &fields[6];
    let epno = &fields[7];
    let ep_name = &fields[8];
    let group_short = &fields[9];

    // anime_name: prefer romaji, fallback to english
    let anime_name = if romaji_name.is_empty() {
        english_name.to_string()
    } else {
        romaji_name.to_string()
    };

    // Parse year: AniDB returns strings like "2023" or "2023-2024"; parse leading digits.
    let year = parse_year(year_str);

    // Parse related aids: comma-separated aid list + comma-separated type list, zipped.
    let related_aids = parse_related_aids(related_aid_list, related_aid_type);

    Ok(AniDbMetadata {
        anime_id: aid,
        anime_name,
        episode_number: epno.to_string(),
        episode_name: ep_name.to_string(),
        group_name: group_short.to_string(),
        source: dessplay_core::types::MetadataSource::AniDb,
        year,
        related_aids,
    })
}

/// Parse a year string from AniDB. Returns None if not a valid year in 1970-2070.
/// AniDB may return "2023", "2023-2024", or empty strings.
fn parse_year(s: &str) -> Option<u32> {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    let year: u32 = digits.parse().ok()?;
    if (1970..=2070).contains(&year) {
        Some(year)
    } else {
        None
    }
}

/// Parse related aid lists from AniDB.
/// Both lists are comma-separated and must be the same length.
/// Returns Vec<(anime_id, relation_type)>.
fn parse_related_aids(aid_list: &str, type_list: &str) -> Vec<(u64, u16)> {
    if aid_list.is_empty() || type_list.is_empty() {
        return Vec::new();
    }
    let aids: Vec<u64> = aid_list.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let types: Vec<u16> = type_list.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    aids.into_iter().zip(types).collect()
}

/// Split AniDB pipe-delimited fields, handling backtick-escaped pipes.
///
/// In AniDB's encoding, a backtick before a pipe (`|) means the pipe is literal
/// and should not be treated as a field separator.
fn split_anidb_fields(data: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = data.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '`' && i + 1 < chars.len() && chars[i + 1] == '|' {
            // Escaped pipe — include literal pipe
            current.push('|');
            i += 2;
        } else if chars[i] == '|' {
            // Field separator
            fields.push(current.clone());
            current.clear();
            i += 1;
        } else {
            current.push(chars[i]);
            i += 1;
        }
    }
    // Trim trailing newline from last field
    fields.push(current.trim_end().to_string());
    fields
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_code_basic() {
        assert_eq!(parse_response_code("200 abc LOGIN ACCEPTED"), 200);
        assert_eq!(parse_response_code("220 FILE\ndata"), 220);
        assert_eq!(parse_response_code("320 NO SUCH FILE"), 320);
        assert_eq!(parse_response_code("550 BANNED"), 550);
        assert_eq!(parse_response_code(""), 0);
        assert_eq!(parse_response_code("not-a-number blah"), 0);
    }

    #[test]
    fn split_fields_basic() {
        let fields = split_anidb_fields("a|b|c");
        assert_eq!(fields, vec!["a", "b", "c"]);
    }

    #[test]
    fn split_fields_escaped_pipe() {
        // backtick-pipe should produce a literal pipe in the field
        let fields = split_anidb_fields("hello`|world|next");
        assert_eq!(fields, vec!["hello|world", "next"]);
    }

    #[test]
    fn split_fields_empty() {
        let fields = split_anidb_fields("a||c");
        assert_eq!(fields, vec!["a", "", "c"]);
    }

    #[test]
    fn split_fields_trailing_newline() {
        let fields = split_anidb_fields("a|b|c\n");
        assert_eq!(fields, vec!["a", "b", "c"]);
    }

    #[test]
    fn parse_file_response_normal() {
        // Fields: fid|aid|year|related_aids|related_types|romaji|english|epno|ep_name|group
        let line = "12345|6789|2023|6790,6791|1,2|Sousou no Frieren|Frieren: Beyond Journey's End|1|The Journey's End|SubsPlease";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.anime_id, 6789);
        assert_eq!(meta.anime_name, "Sousou no Frieren");
        assert_eq!(meta.episode_number, "1");
        assert_eq!(meta.episode_name, "The Journey's End");
        assert_eq!(meta.group_name, "SubsPlease");
        assert_eq!(meta.year, Some(2023));
        assert_eq!(meta.related_aids, vec![(6790, 1), (6791, 2)]);
    }

    #[test]
    fn parse_file_response_special_episode() {
        let line = "100|200|2020|||TestAnime||S1|Special 1|TestGroup";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.episode_number, "S1");
        assert_eq!(meta.anime_name, "TestAnime");
    }

    #[test]
    fn parse_file_response_credit_episode() {
        let line = "100|200|2020|||TestAnime||C1|Opening 1|TestGroup";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.episode_number, "C1");
    }

    #[test]
    fn parse_file_response_romaji_empty_fallback() {
        let line = "100|200|2021||||English Name|5|Episode Five|Grp";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.anime_name, "English Name");
    }

    #[test]
    fn parse_file_response_escaped_pipe_in_name() {
        // Anime name contains a literal pipe (escaped with backtick)
        let line = "100|200|2022|||Name`|With`|Pipes|English|3|Ep Three|Grp";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.anime_name, "Name|With|Pipes");
        assert_eq!(meta.episode_number, "3");
    }

    #[test]
    fn parse_file_response_too_few_fields() {
        let line = "100|200|Name";
        assert!(parse_file_response(line).is_err());
    }

    #[test]
    fn parse_file_response_empty_group() {
        let line = "100|200|2023|||Anime|English|1|Episode 1|";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.group_name, "");
    }

    #[test]
    fn parse_file_response_no_related_aids() {
        let line = "100|200|2023|||Anime|English|1|Episode 1|Grp";
        let meta = parse_file_response(line).unwrap();
        assert!(meta.related_aids.is_empty());
    }

    #[test]
    fn parse_file_response_year_range() {
        // AniDB returns "2023-2024" for multi-year series; we parse the first year.
        let line = "100|200|2023-2024|||Anime||1|Ep1|Grp";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.year, Some(2023));
    }

    #[test]
    fn parse_file_response_no_year() {
        let line = "100|200||||Anime||1|Ep1|Grp";
        let meta = parse_file_response(line).unwrap();
        assert_eq!(meta.year, None);
    }

    #[test]
    fn parse_year_valid() {
        assert_eq!(parse_year("2023"), Some(2023));
        assert_eq!(parse_year("2023-2024"), Some(2023));
        assert_eq!(parse_year("1990"), Some(1990));
    }

    #[test]
    fn parse_year_invalid() {
        assert_eq!(parse_year(""), None);
        assert_eq!(parse_year("1900"), None); // outside 1970-2070
        assert_eq!(parse_year("abc"), None);
    }

    #[test]
    fn parse_related_aids_basic() {
        let aids = parse_related_aids("100,200,300", "1,2,51");
        assert_eq!(aids, vec![(100, 1), (200, 2), (300, 51)]);
    }

    #[test]
    fn parse_related_aids_empty() {
        let aids = parse_related_aids("", "");
        assert!(aids.is_empty());
    }

    #[test]
    fn parse_related_aids_mismatched_lengths() {
        // zip stops at the shorter list
        let aids = parse_related_aids("100,200", "1");
        assert_eq!(aids, vec![(100, 1)]);
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limiter_first_request_immediate() {
        let limiter = RateLimiter::new();
        // First request should be immediate (no wait)
        let before = Instant::now();
        limiter.wait_until_ready().await;
        let elapsed = before.elapsed();
        assert!(elapsed < Duration::from_millis(100));
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limiter_enforces_interval() {
        let mut limiter = RateLimiter::new();
        limiter.record_sent();

        let before = Instant::now();
        limiter.wait_until_ready().await;
        let elapsed = before.elapsed();
        assert!(elapsed >= Duration::from_secs(4));
    }

    #[tokio::test(start_paused = true)]
    async fn rate_limiter_throttle_penalty() {
        let mut limiter = RateLimiter::new();
        limiter.record_throttle();

        let before = Instant::now();
        limiter.wait_until_ready().await;
        let elapsed = before.elapsed();
        assert!(elapsed >= Duration::from_secs(5));
    }
}
