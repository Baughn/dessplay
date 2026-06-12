//! The rate-limited AniDB UDP client: session management, request
//! retries, and backoff classification.
//!
//! The worker consumes this through the [`AniDbApi`] trait (dyn-safe,
//! boxed futures — the one deviation from the project's RPITIT style,
//! needed so `ServerConfig` can carry `Arc<dyn AniDbApi>` without
//! making the whole server generic over it). Tests implement the trait
//! directly with canned data; this module's own tests drive the real
//! client over a scripted [`Wire`].
//!
//! Rate limits (docs/design.md): never more than 1 packet per 2
//! seconds, and no more than 1 per 4 seconds sustained, with a burst
//! of 60. Server-throttled replies count against the limit. A missing
//! response costs a 5-second penalty before the next send.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use dessplay_core::types::{AniDbSeriesId, Ed2kHash};
use tokio::time::Instant;

use super::protocol::{self, AnimeResult, FileResult, Response};

/// Hard floor between packets.
const MIN_GAP: Duration = Duration::from_secs(2);
/// Sustained rate: one packet per this, with [`BURST`] of slack.
const SUSTAINED_GAP: Duration = Duration::from_secs(4);
/// Token-bucket capacity for the sustained limiter.
const BURST: f64 = 60.0;
/// How long to wait for a reply before declaring it missing.
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(20);
/// Penalty before the next send after a missing response.
const TIMEOUT_PENALTY: Duration = Duration::from_secs(5);
/// Backoff after 602 SERVER BUSY.
const BUSY_BACKOFF: Duration = Duration::from_secs(5 * 60);
/// Backoff after 555 BANNED / 600 / 601 — flood bans are typically
/// 30 minutes; hammering during one extends it.
const BAN_BACKOFF: Duration = Duration::from_secs(30 * 60);

/// How a lookup failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LookupError {
    /// No reply (or an undecodable one). The 5s penalty is already
    /// applied internally; the caller may retry on its own schedule.
    Timeout,
    /// The server told us to go away for a while. The worker should
    /// not send anything for `millis`.
    Backoff {
        /// How long to stay quiet.
        millis: u64,
    },
    /// Unrecoverable: bad credentials, banned client, outdated
    /// protocol. The worker disables itself and logs loudly.
    Fatal(String),
}

impl std::fmt::Display for LookupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LookupError::Timeout => write!(f, "no response from AniDB"),
            LookupError::Backoff { millis } => write!(f, "AniDB asked for a {millis}ms backoff"),
            LookupError::Fatal(reason) => write!(f, "fatal AniDB error: {reason}"),
        }
    }
}

impl std::error::Error for LookupError {}

/// A boxed future, for the dyn-safe [`AniDbApi`].
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What the worker needs from AniDB. `Ok(None)` is a definitive miss
/// (AniDB does not know the file/anime); errors are transient or
/// fatal per [`LookupError`].
pub trait AniDbApi: Send + Sync + 'static {
    /// FILE lookup by (size, ed2k hash).
    fn file_by_hash(
        &self,
        size: u64,
        hash: Ed2kHash,
    ) -> BoxFuture<'_, Result<Option<FileResult>, LookupError>>;

    /// ANIME lookup by aid.
    fn anime_by_id(
        &self,
        aid: AniDbSeriesId,
    ) -> BoxFuture<'_, Result<Option<AnimeResult>, LookupError>>;
}

/// Datagram transport, mockable for tests. The real implementation is
/// [`UdpWire`].
pub trait Wire: Send + Sync + 'static {
    /// Send one request datagram.
    fn send(&self, payload: &str) -> impl Future<Output = std::io::Result<()>> + Send;
    /// Receive the next reply datagram.
    fn recv(&self) -> impl Future<Output = std::io::Result<String>> + Send;
}

/// A connected UDP socket to the real API.
pub struct UdpWire {
    socket: tokio::net::UdpSocket,
}

impl UdpWire {
    /// Bind an ephemeral local port and connect it to `server`
    /// (normally `api.anidb.net:9000`).
    pub async fn connect(server: &str) -> std::io::Result<Self> {
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(server).await?;
        Ok(Self { socket })
    }
}

impl Wire for UdpWire {
    async fn send(&self, payload: &str) -> std::io::Result<()> {
        self.socket.send(payload.as_bytes()).await.map(|_| ())
    }

    async fn recv(&self) -> std::io::Result<String> {
        let mut buf = vec![0u8; 64 * 1024];
        let len = self.socket.recv(&mut buf).await?;
        Ok(String::from_utf8_lossy(&buf[..len]).into_owned())
    }
}

/// Token-bucket limiter with a hard minimum gap. Time comes from
/// `tokio::time`, so paused-time tests are exact.
struct RateLimiter {
    tokens: f64,
    last_refill: Instant,
    next_allowed: Instant,
}

impl RateLimiter {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            tokens: BURST,
            last_refill: now,
            next_allowed: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let accrued = now.duration_since(self.last_refill).as_secs_f64()
            / SUSTAINED_GAP.as_secs_f64();
        self.tokens = (self.tokens + accrued).min(BURST);
        self.last_refill = now;
    }

    /// Wait until a send is allowed, then account for it.
    async fn acquire(&mut self) {
        let now = Instant::now();
        self.refill(now);
        let mut until = self.next_allowed;
        if self.tokens < 1.0 {
            let deficit = 1.0 - self.tokens;
            until = until.max(now + SUSTAINED_GAP.mul_f64(deficit));
        }
        if until > now {
            tokio::time::sleep_until(until).await;
        }
        let now = Instant::now();
        self.refill(now);
        self.tokens -= 1.0;
        self.next_allowed = now + MIN_GAP;
    }

    /// Push the next allowed send out by at least `extra` from now.
    fn penalize(&mut self, extra: Duration) {
        self.next_allowed = self.next_allowed.max(Instant::now() + extra);
    }
}

struct Inner {
    limiter: RateLimiter,
    session: Option<String>,
    tag_counter: u64,
}

/// The real client: one session, strictly serialized requests.
pub struct UdpClient<W: Wire> {
    wire: W,
    user: String,
    password: String,
    inner: tokio::sync::Mutex<Inner>,
}

impl<W: Wire> UdpClient<W> {
    /// A client that will AUTH lazily on the first request.
    pub fn new(wire: W, user: impl Into<String>, password: impl Into<String>) -> Self {
        Self {
            wire,
            user: user.into(),
            password: password.into(),
            inner: tokio::sync::Mutex::new(Inner {
                limiter: RateLimiter::new(),
                session: None,
                tag_counter: 0,
            }),
        }
    }

    /// Send one packet and wait for the matching (tagged) reply.
    async fn exchange(&self, inner: &mut Inner, payload: &str, tag: &str) -> Result<Response, LookupError> {
        inner.limiter.acquire().await;
        tracing::trace!(command = payload.split(' ').next().unwrap_or(""), "anidb send");
        if self.wire.send(payload).await.is_err() {
            inner.limiter.penalize(TIMEOUT_PENALTY);
            return Err(LookupError::Timeout);
        }
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let raw = match tokio::time::timeout(remaining, self.wire.recv()).await {
                Ok(Ok(raw)) => raw,
                Ok(Err(_)) | Err(_) => {
                    inner.limiter.penalize(TIMEOUT_PENALTY);
                    return Err(LookupError::Timeout);
                }
            };
            match protocol::parse_response(&raw) {
                // Untagged error replies (the server drops the tag on
                // some global errors like 555) still end this request.
                Ok(response) if response.tag.as_deref() == Some(tag) || response.tag.is_none() => {
                    tracing::trace!(code = response.code, "anidb recv");
                    return Ok(response);
                }
                Ok(stale) => {
                    tracing::debug!(code = stale.code, tag = ?stale.tag, "dropping stale anidb reply");
                }
                Err(e) => {
                    tracing::warn!("undecodable anidb reply: {e}");
                }
            }
        }
    }

    /// Classify the global error codes every command can produce.
    /// Returns `None` if the response is command-specific.
    fn global_error(&self, inner: &mut Inner, response: &Response) -> Option<LookupError> {
        let backoff = |inner: &mut Inner, duration: Duration| {
            inner.limiter.penalize(duration);
            Some(LookupError::Backoff {
                millis: duration.as_millis() as u64,
            })
        };
        match response.code {
            protocol::SERVER_BUSY => backoff(inner, BUSY_BACKOFF),
            protocol::BANNED | protocol::INTERNAL_SERVER_ERROR | protocol::OUT_OF_SERVICE => {
                backoff(inner, BAN_BACKOFF)
            }
            protocol::LOGIN_FAILED => Some(LookupError::Fatal("login failed (bad credentials)".into())),
            protocol::CLIENT_VERSION_OUTDATED => {
                Some(LookupError::Fatal("client version outdated".into()))
            }
            protocol::CLIENT_BANNED => Some(LookupError::Fatal(format!(
                "client banned: {}",
                response.text
            ))),
            protocol::ACCESS_DENIED => Some(LookupError::Fatal("access denied".into())),
            _ => None,
        }
    }

    async fn ensure_session(&self, inner: &mut Inner) -> Result<String, LookupError> {
        if let Some(session) = &inner.session {
            return Ok(session.clone());
        }
        inner.tag_counter += 1;
        let tag = format!("t{}", inner.tag_counter);
        let payload = protocol::auth(&self.user, &self.password, &tag);
        let response = self.exchange(inner, &payload, &tag).await?;
        if let Some(error) = self.global_error(inner, &response) {
            return Err(error);
        }
        match response.code {
            protocol::LOGIN_ACCEPTED | protocol::LOGIN_ACCEPTED_NEW_VERSION => {
                let session = protocol::session_key(&response.text).ok_or_else(|| {
                    LookupError::Fatal(format!("no session key in AUTH reply {:?}", response.text))
                })?;
                if response.code == protocol::LOGIN_ACCEPTED_NEW_VERSION {
                    tracing::info!("AniDB reports a newer registered client version");
                }
                tracing::debug!("anidb session established");
                inner.session = Some(session.clone());
                Ok(session)
            }
            other => Err(LookupError::Fatal(format!(
                "unexpected AUTH reply {other} {}",
                response.text
            ))),
        }
    }

    /// Run a session-bearing command, re-authenticating once if the
    /// session has expired.
    async fn call(
        &self,
        build: impl Fn(&str, &str) -> String,
    ) -> Result<Response, LookupError> {
        let mut inner = self.inner.lock().await;
        for attempt in 0..2 {
            let session = self.ensure_session(&mut inner).await?;
            inner.tag_counter += 1;
            let tag = format!("t{}", inner.tag_counter);
            let payload = build(&session, &tag);
            let response = self.exchange(&mut inner, &payload, &tag).await?;
            if let Some(error) = self.global_error(&mut inner, &response) {
                return Err(error);
            }
            match response.code {
                protocol::LOGIN_FIRST | protocol::INVALID_SESSION => {
                    tracing::debug!("anidb session expired; re-authenticating");
                    inner.session = None;
                    if attempt == 1 {
                        return Err(LookupError::Backoff {
                            millis: BUSY_BACKOFF.as_millis() as u64,
                        });
                    }
                }
                _ => return Ok(response),
            }
        }
        // Unreachable: the loop returns on every path of its second pass.
        Err(LookupError::Timeout)
    }

    /// LOGOUT, dropping the session. Best-effort; used by the probe
    /// binary and clean shutdown.
    pub async fn logout(&self) -> Result<(), LookupError> {
        let mut inner = self.inner.lock().await;
        let Some(session) = inner.session.take() else {
            return Ok(());
        };
        inner.tag_counter += 1;
        let tag = format!("t{}", inner.tag_counter);
        let payload = protocol::logout(&session, &tag);
        self.exchange(&mut inner, &payload, &tag).await.map(|_| ())
    }
}

impl<W: Wire> AniDbApi for UdpClient<W> {
    fn file_by_hash(
        &self,
        size: u64,
        hash: Ed2kHash,
    ) -> BoxFuture<'_, Result<Option<FileResult>, LookupError>> {
        Box::pin(async move {
            let response = self
                .call(|session, tag| protocol::file_by_hash(size, hash, session, tag))
                .await?;
            match response.code {
                protocol::FILE => protocol::parse_file_data(&response)
                    .map(Some)
                    .map_err(|e| LookupError::Fatal(format!("FILE parse error: {e}"))),
                protocol::NO_SUCH_FILE | protocol::MULTIPLE_FILES_FOUND => Ok(None),
                other => Err(LookupError::Backoff {
                    millis: BUSY_BACKOFF.as_millis() as u64,
                })
                .inspect_err(|_| {
                    tracing::warn!(code = other, text = %response.text, "unexpected FILE reply");
                }),
            }
        })
    }

    fn anime_by_id(
        &self,
        aid: AniDbSeriesId,
    ) -> BoxFuture<'_, Result<Option<AnimeResult>, LookupError>> {
        Box::pin(async move {
            let response = self
                .call(|session, tag| protocol::anime_by_id(aid, session, tag))
                .await?;
            match response.code {
                protocol::ANIME => protocol::parse_anime_data(&response)
                    .map(Some)
                    .map_err(|e| LookupError::Fatal(format!("ANIME parse error: {e}"))),
                protocol::NO_SUCH_ANIME => Ok(None),
                other => Err(LookupError::Backoff {
                    millis: BUSY_BACKOFF.as_millis() as u64,
                })
                .inspect_err(|_| {
                    tracing::warn!(code = other, text = %response.text, "unexpected ANIME reply");
                }),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Mutex;

    use super::*;

    /// A scripted wire: each send consults the script for a reply (or
    /// silence). Requests are logged with their tokio-time arrival.
    struct MockWire {
        state: Mutex<MockState>,
        replies: tokio::sync::Mutex<tokio::sync::mpsc::UnboundedReceiver<String>>,
        reply_tx: tokio::sync::mpsc::UnboundedSender<String>,
    }

    struct MockState {
        script: Vec<Script>,
        sent: Vec<(String, Instant)>,
    }

    /// One scripted exchange, matched in order.
    enum Script {
        /// Reply with `{tag} {body}` (tag extracted from the request).
        Reply(&'static str),
        /// Don't answer at all.
        Silence,
        /// Reply without echoing the tag.
        Untagged(&'static str),
    }

    fn tag_of(request: &str) -> String {
        request
            .split(['&', ' '])
            .find_map(|kv| kv.strip_prefix("tag="))
            .unwrap_or("")
            .to_string()
    }

    impl MockWire {
        fn new(script: Vec<Script>) -> Self {
            let (reply_tx, replies) = tokio::sync::mpsc::unbounded_channel();
            Self {
                state: Mutex::new(MockState {
                    script,
                    sent: Vec::new(),
                }),
                replies: tokio::sync::Mutex::new(replies),
                reply_tx,
            }
        }

        fn sent(&self) -> Vec<(String, Instant)> {
            self.state.lock().unwrap().sent.clone()
        }
    }

    impl Wire for &'static MockWire {
        async fn send(&self, payload: &str) -> std::io::Result<()> {
            let mut state = self.state.lock().unwrap();
            state.sent.push((payload.to_string(), Instant::now()));
            let step = if state.script.is_empty() {
                Script::Silence
            } else {
                state.script.remove(0)
            };
            match step {
                Script::Reply(body) => {
                    let tag = tag_of(payload);
                    let _ = self.reply_tx.send(format!("{tag} {body}"));
                }
                Script::Untagged(body) => {
                    let _ = self.reply_tx.send(body.to_string());
                }
                Script::Silence => {}
            }
            Ok(())
        }

        async fn recv(&self) -> std::io::Result<String> {
            let mut replies = self.replies.lock().await;
            replies
                .recv()
                .await
                .ok_or_else(|| std::io::Error::other("wire closed"))
        }
    }

    fn client(script: Vec<Script>) -> (UdpClient<&'static MockWire>, &'static MockWire) {
        let wire: &'static MockWire = Box::leak(Box::new(MockWire::new(script)));
        (UdpClient::new(wire, "user", "pass"), wire)
    }

    fn hash() -> Ed2kHash {
        Ed2kHash([7; 16])
    }

    const AUTH_OK: &str = "200 abc12 LOGIN ACCEPTED";
    const FILE_HIT: &str = "220 FILE\n1|8692|Sousou no Frieren|Frieren|01";

    #[tokio::test(start_paused = true)]
    async fn authenticates_lazily_then_looks_up() {
        let (client, wire) = client(vec![Script::Reply(AUTH_OK), Script::Reply(FILE_HIT)]);
        let result = client.file_by_hash(1234, hash()).await.unwrap().unwrap();
        assert_eq!(result.aid, AniDbSeriesId(8692));
        assert_eq!(result.epno, "01");

        let sent = wire.sent();
        assert_eq!(sent.len(), 2);
        assert!(sent[0].0.starts_with("AUTH "));
        assert!(sent[0].0.contains("client=dessplay"));
        assert!(sent[1].0.starts_with("FILE "));
        assert!(sent[1].0.contains("s=abc12"));
    }

    #[tokio::test(start_paused = true)]
    async fn no_such_file_is_a_definitive_miss() {
        let (client, _) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Reply("320 NO SUCH FILE"),
        ]);
        assert_eq!(client.file_by_hash(1234, hash()).await.unwrap(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn packets_keep_the_minimum_gap() {
        let (client, wire) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Reply(FILE_HIT),
            Script::Reply(FILE_HIT),
        ]);
        client.file_by_hash(1, hash()).await.unwrap();
        client.file_by_hash(2, hash()).await.unwrap();
        let sent = wire.sent();
        assert_eq!(sent.len(), 3);
        // AUTH -> FILE and FILE -> FILE both >= 2s apart.
        assert!(sent[1].1.duration_since(sent[0].1) >= MIN_GAP);
        assert!(sent[2].1.duration_since(sent[1].1) >= MIN_GAP);
    }

    #[tokio::test(start_paused = true)]
    async fn sustained_rate_drops_to_one_per_four_seconds() {
        // At the 2s floor each packet drains a net half token, so the
        // 60-token burst carries ~120 packets before the sustained
        // rate takes over. 130 lookups must cross that point.
        const LOOKUPS: u64 = 130;
        let mut script = vec![Script::Reply(AUTH_OK)];
        script.extend((0..LOOKUPS).map(|_| Script::Reply(FILE_HIT)));
        let (client, wire) = client(script);
        for i in 0..LOOKUPS {
            client.file_by_hash(i, hash()).await.unwrap();
        }
        let sent = wire.sent();
        assert_eq!(sent.len() as u64, LOOKUPS + 1);
        // Early packets ride the burst at the 2s floor...
        assert!(sent[2].1.duration_since(sent[1].1) < Duration::from_millis(2100));
        // ...but the tail has fallen to the sustained one-per-4s rate.
        let last = sent.len() - 1;
        let tail = sent[last].1.duration_since(sent[last - 1].1);
        assert!(tail >= Duration::from_millis(3900), "tail gap was {tail:?}");
        // Total wall time respects the sustained budget: n packets
        // cost at least (n - burst) * 4s in refills.
        let total = sent[last].1.duration_since(sent[0].1);
        let budget = Duration::from_secs((LOOKUPS + 1 - 60) * 4);
        assert!(total >= budget, "total was {total:?}, budget {budget:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn missing_response_times_out_and_penalizes() {
        let (client, wire) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Silence,
            Script::Reply(FILE_HIT),
        ]);
        let start = Instant::now();
        assert_eq!(
            client.file_by_hash(1, hash()).await,
            Err(LookupError::Timeout)
        );
        assert!(Instant::now().duration_since(start) >= RESPONSE_TIMEOUT);

        // The next send waits the 5s penalty from the timeout.
        client.file_by_hash(2, hash()).await.unwrap();
        let sent = wire.sent();
        assert_eq!(sent.len(), 3);
        assert!(sent[2].1.duration_since(sent[1].1) >= RESPONSE_TIMEOUT + TIMEOUT_PENALTY);
    }

    #[tokio::test(start_paused = true)]
    async fn expired_session_reauths_once() {
        let (client, wire) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Reply("501 LOGIN FIRST"),
            Script::Reply("200 xyz99 LOGIN ACCEPTED"),
            Script::Reply(FILE_HIT),
        ]);
        let result = client.file_by_hash(1, hash()).await.unwrap();
        assert!(result.is_some());
        let sent = wire.sent();
        assert_eq!(sent.len(), 4);
        assert!(sent[2].0.starts_with("AUTH "));
        assert!(sent[3].0.contains("s=xyz99"));
    }

    #[tokio::test(start_paused = true)]
    async fn bad_credentials_are_fatal() {
        let (client, _) = client(vec![Script::Reply("500 LOGIN FAILED")]);
        assert!(matches!(
            client.file_by_hash(1, hash()).await,
            Err(LookupError::Fatal(_))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn server_busy_backs_off_five_minutes() {
        let (client, wire) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Untagged("602 SERVER BUSY - PLEASE TRY AGAIN LATER"),
            Script::Reply(FILE_HIT),
        ]);
        assert_eq!(
            client.file_by_hash(1, hash()).await,
            Err(LookupError::Backoff {
                millis: BUSY_BACKOFF.as_millis() as u64
            })
        );
        // The backoff is enforced internally too.
        client.file_by_hash(2, hash()).await.unwrap();
        let sent = wire.sent();
        assert!(sent[2].1.duration_since(sent[1].1) >= BUSY_BACKOFF);
    }

    #[tokio::test(start_paused = true)]
    async fn ban_backs_off_thirty_minutes() {
        let (client, _) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Untagged("555 BANNED\nflooding"),
        ]);
        assert_eq!(
            client.file_by_hash(1, hash()).await,
            Err(LookupError::Backoff {
                millis: BAN_BACKOFF.as_millis() as u64
            })
        );
    }

    #[tokio::test(start_paused = true)]
    async fn stale_replies_are_skipped() {
        // The reply to a previous (timed-out) tag arrives before ours.
        let (client, _) = client(vec![Script::Reply(AUTH_OK), Script::Silence]);
        // Run a lookup that times out, leaving t2 unanswered.
        assert_eq!(
            client.file_by_hash(1, hash()).await,
            Err(LookupError::Timeout)
        );
        // Now the wire delivers the stale t2 reply followed by the real
        // t3 reply.
        let _ = client.wire.reply_tx.send("t2 220 FILE\n9|9|stale|stale|99".into());
        let _ = client
            .wire
            .reply_tx
            .send("t3 220 FILE\n1|8692|Sousou no Frieren|Frieren|01".into());
        let result = client.file_by_hash(2, hash()).await.unwrap().unwrap();
        assert_eq!(result.aid, AniDbSeriesId(8692));
    }

    #[tokio::test(start_paused = true)]
    async fn anime_lookup_parses_relations() {
        let (client, _) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Reply("230 ANIME\n8692|2023|13310|2|Sousou no Frieren|Frieren|28"),
        ]);
        let anime = client
            .anime_by_id(AniDbSeriesId(8692))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(anime.relations, vec![(2, AniDbSeriesId(13310))]);
        assert_eq!(anime.episode_count, Some(28));
    }

    #[tokio::test(start_paused = true)]
    async fn logout_uses_the_session() {
        let (client, wire) = client(vec![
            Script::Reply(AUTH_OK),
            Script::Reply(FILE_HIT),
            Script::Reply("203 LOGGED OUT"),
        ]);
        client.file_by_hash(1, hash()).await.unwrap();
        client.logout().await.unwrap();
        let sent = wire.sent();
        assert_eq!(sent.len(), 3);
        assert!(sent[2].0.starts_with("LOGOUT "));
        assert!(sent[2].0.contains("s=abc12"));
        // Logging out twice is a no-op (no session).
        client.logout().await.unwrap();
        assert_eq!(wire.sent().len(), 3);
    }
}
