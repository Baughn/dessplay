//! The IRC bridge actor: mirrors the local user's chat into an IRC
//! channel and surfaces messages from external IRC users back into the
//! dessplay chat pane.
//!
//! It exists so the conversation survives the app being closed: dessplay
//! logs are unavailable when the program isn't running, but an IRC
//! channel that others keep open (or a bouncer) is a durable record, and
//! plain-IRC users can join in. See design.md (IRC bridge).
//!
//! Each dessplay user runs their own bridge, connecting as
//! `[Username]Dess` (e.g. `BaughnDess`). The actor sends only the local
//! user's own chat (tapped at the `Mutation::Chat` site in the session
//! loop — events, subtitles and narrator lines never reach here), and
//! displays incoming messages from nicks that do *not* end in `Dess`.
//! Messages from `*Dess` nicks are other bridges' echoes of dessplay
//! users who are already present via CRDT sync, so they are dropped to
//! avoid double display.
//!
//! The bridge is interactive-only: it is spawned from `run_interactive`,
//! never from the shared `spawn_client` (seeders are headless and have
//! no chat).
//!
//! The IRC protocol surface is tiny — NICK/USER registration, JOIN,
//! PRIVMSG, PING/PONG, 433 nick-collision fallback, and CTCP ACTION —
//! so it is hand-rolled over `tokio-rustls` (reusing the project's
//! pinned rustls, no native-tls) rather than pulling a heavier crate.
//! The line parsing/formatting lives in pure functions that are unit
//! tested in isolation; the actor loop is driven over an in-memory
//! `tokio::io::duplex` pipe in tests.

use std::sync::Arc;
use std::time::Duration;

use dessplay_core::types::{UserId, decode_action};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::config::Settings;

/// First delay between reconnection attempts; doubles up to [`MAX_BACKOFF`].
const INITIAL_BACKOFF: Duration = Duration::from_secs(2);
/// Cap on the reconnection backoff.
const MAX_BACKOFF: Duration = Duration::from_secs(60);
/// Conservative per-PRIVMSG text budget. The 512-byte IRC line limit
/// includes `CRLF` and the server-prepended `:nick!user@host ` we can't
/// see, so leave generous headroom.
const MAX_PRIVMSG_TEXT: usize = 400;
/// The nick suffix that marks a dessplay bridge.
const BRIDGE_SUFFIX: &str = "Dess";
/// Maximum nick length (Rizon allows 30); base is truncated to fit while
/// the [`BRIDGE_SUFFIX`] is always preserved.
const MAX_NICK: usize = 30;
/// Cap on 433 nick-collision retries before giving up on a connection.
const MAX_NICK_TRIES: u32 = 5;

/// Static configuration for the bridge, derived from [`Settings`] plus
/// the local user's name.
#[derive(Clone, Debug)]
pub struct IrcConfig {
    /// When false the actor stays idle (no socket) until reconfigured.
    pub enabled: bool,
    /// IRC server hostname.
    pub server: String,
    /// TCP port (6697 for TLS, 6667 plaintext).
    pub port: u16,
    /// Connect over TLS.
    pub tls: bool,
    /// Channel to join (e.g. `#dess`).
    pub channel: String,
    /// Our nick (`[Username]Dess`).
    pub nick: String,
}

impl IrcConfig {
    /// Derive the bridge config from persisted settings and the local
    /// user. The port follows the TLS toggle; the nick is the username
    /// with the `Dess` suffix (sanitized for IRC).
    pub fn from_settings(settings: &Settings, me: &UserId) -> Self {
        Self {
            enabled: settings.irc_enabled,
            server: settings.irc_server.clone(),
            port: if settings.irc_tls { 6697 } else { 6667 },
            tls: settings.irc_tls,
            channel: settings.irc_channel.clone(),
            nick: derive_nick(&me.0),
        }
    }
}

/// Commands to the bridge actor.
#[derive(Debug)]
pub enum IrcCommand {
    /// Forward one of the local user's chat messages (raw text, possibly
    /// CTCP-encoded for `/me`) to the channel.
    SendChat(String),
    /// Replace the live config (settings changed); reconnects cleanly.
    Reconfigure(Box<IrcConfig>),
    /// QUIT and exit the actor.
    Shutdown,
}

/// Events from the bridge actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IrcEvent {
    /// Registered and joined the channel.
    Connected,
    /// The connection dropped (after having been established).
    Disconnected {
        /// Human-readable reason.
        reason: String,
    },
    /// A message from an external IRC user (a non-`Dess` nick).
    Message {
        /// Sender nick.
        from: String,
        /// Message text (decoded if it was a CTCP ACTION).
        text: String,
        /// True if the message was a CTCP ACTION (an emote / `/me`).
        action: bool,
    },
}

/// Run the bridge until [`IrcCommand::Shutdown`] (or the command channel
/// closes). Owns the reconnection loop and the live config.
pub async fn run(
    mut config: IrcConfig,
    mut commands: mpsc::Receiver<IrcCommand>,
    events: mpsc::Sender<IrcEvent>,
) {
    let mut backoff = INITIAL_BACKOFF;
    // The current nick persists across reconnects so a 433 disambiguation
    // sticks; reset to the configured nick whenever the config changes.
    let mut nick = config.nick.clone();
    loop {
        // Disabled: idle until a command arrives. Drop any SendChat.
        while !config.enabled {
            match commands.recv().await {
                None | Some(IrcCommand::Shutdown) => return,
                Some(IrcCommand::Reconfigure(c)) => {
                    config = *c;
                    nick = config.nick.clone();
                    backoff = INITIAL_BACKOFF;
                }
                Some(IrcCommand::SendChat(_)) => {}
            }
        }

        tracing::info!(server = %config.server, port = config.port, nick = %nick, "connecting to IRC");
        match connect(&config).await {
            Ok(stream) => {
                match run_session(stream, &config, &mut nick, &mut commands, &events).await {
                    SessionEnd::Shutdown => return,
                    SessionEnd::Reconfigure(c) => {
                        config = *c;
                        nick = config.nick.clone();
                        backoff = INITIAL_BACKOFF;
                        continue;
                    }
                    SessionEnd::Lost { reason, registered } => {
                        tracing::info!(reason = %reason, registered, "IRC connection lost");
                        // Only surface drops of an established session;
                        // failed retries while a server is down would
                        // otherwise spam the chat with system lines.
                        if registered {
                            let _ = events.send(IrcEvent::Disconnected { reason }).await;
                            // A clean reconnect — try again promptly.
                            backoff = INITIAL_BACKOFF;
                        }
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "IRC connection attempt failed"),
        }

        // Wait before reconnecting, but stay responsive to commands.
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            cmd = commands.recv() => match cmd {
                None | Some(IrcCommand::Shutdown) => return,
                Some(IrcCommand::Reconfigure(c)) => {
                    config = *c;
                    nick = config.nick.clone();
                    backoff = INITIAL_BACKOFF;
                    continue;
                }
                Some(IrcCommand::SendChat(_)) => {}
            },
        }
        backoff = grow_backoff(backoff);
    }
}

/// How a single connection ended.
pub(crate) enum SessionEnd {
    /// Asked to shut down; the actor should exit.
    Shutdown,
    /// Config changed; reconnect with the new config.
    Reconfigure(Box<IrcConfig>),
    /// The connection was lost. `registered` is true if we had reached
    /// `001` (so the next attempt should be prompt and the drop is
    /// worth surfacing).
    Lost {
        /// Human-readable reason.
        reason: String,
        /// Whether registration (`001`) completed.
        registered: bool,
    },
}

/// Open a TCP (and optionally TLS) connection to the IRC server, boxed
/// behind a stream trait object so the TLS and plaintext paths unify.
async fn connect(config: &IrcConfig) -> Result<Box<dyn StreamIo>, String> {
    let tcp = TcpStream::connect((config.server.as_str(), config.port))
        .await
        .map_err(|e| format!("tcp connect: {e}"))?;
    let _ = tcp.set_nodelay(true);
    if config.tls {
        let tls_config = build_tls_config()?;
        let connector = tokio_rustls::TlsConnector::from(tls_config);
        let domain = rustls::pki_types::ServerName::try_from(config.server.clone())
            .map_err(|e| format!("invalid server name: {e}"))?;
        let stream = connector
            .connect(domain, tcp)
            .await
            .map_err(|e| format!("tls handshake: {e}"))?;
        Ok(Box::new(stream))
    } else {
        Ok(Box::new(tcp))
    }
}

/// Anything we can speak IRC over (real TLS/TCP streams in production, an
/// in-memory duplex pipe in tests).
trait StreamIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> StreamIo for T {}

/// Build the rustls client config with the Mozilla root store. The
/// project never installs a process-default `CryptoProvider`, so the ring
/// provider is passed explicitly (matching the QUIC connector).
fn build_tls_config() -> Result<Arc<rustls::ClientConfig>, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| format!("tls versions: {e}"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

/// Drive one IRC connection: register, join, then pump lines in and
/// commands out until the connection ends. Generic over the stream so
/// tests can inject a duplex pipe.
pub(crate) async fn run_session<S: AsyncRead + AsyncWrite + Unpin + Send>(
    stream: S,
    config: &IrcConfig,
    nick: &mut String,
    commands: &mut mpsc::Receiver<IrcCommand>,
    events: &mpsc::Sender<IrcEvent>,
) -> SessionEnd {
    let (read, mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();

    if let Err(e) = register(&mut write, nick).await {
        return SessionEnd::Lost {
            reason: e,
            registered: false,
        };
    }
    let mut registered = false;
    let mut nick_tries: u32 = 0;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let line = match line {
                    Ok(Some(line)) => line,
                    Ok(None) => return SessionEnd::Lost { reason: "connection closed".into(), registered },
                    Err(e) => return SessionEnd::Lost { reason: e.to_string(), registered },
                };
                tracing::trace!(line = %line, "IRC <<");
                let Some(parsed) = parse_line(&line) else { continue };
                match parsed.command.to_ascii_uppercase().as_str() {
                    "PING" => {
                        let token = parsed.params.last().map(String::as_str).unwrap_or("");
                        if let Err(e) = send_line(&mut write, &format!("PONG :{token}")).await {
                            return SessionEnd::Lost { reason: e, registered };
                        }
                    }
                    // RPL_WELCOME: registration done — join the channel.
                    "001" => {
                        registered = true;
                        if let Err(e) = send_line(&mut write, &format!("JOIN {}", config.channel)).await {
                            return SessionEnd::Lost { reason: e, registered };
                        }
                        tracing::info!(channel = %config.channel, "joined IRC channel");
                        let _ = events.send(IrcEvent::Connected).await;
                    }
                    // ERR_NICKNAMEINUSE / ERR_NICKCOLLISION before registration.
                    "433" | "436" if !registered => {
                        nick_tries += 1;
                        if nick_tries > MAX_NICK_TRIES {
                            return SessionEnd::Lost { reason: "nick exhausted".into(), registered };
                        }
                        *nick = next_nick_on_collision(nick);
                        tracing::info!(nick = %nick, "IRC nick in use, retrying");
                        if let Err(e) = send_line(&mut write, &format!("NICK {nick}")).await {
                            return SessionEnd::Lost { reason: e, registered };
                        }
                    }
                    "PRIVMSG" => {
                        if let Some(event) = privmsg_event(&parsed, &config.channel) {
                            let _ = events.send(event).await;
                        }
                    }
                    "ERROR" => {
                        let reason = parsed.params.last().cloned().unwrap_or_else(|| "ERROR".into());
                        return SessionEnd::Lost { reason, registered };
                    }
                    // Channel-join failures (banned, invite-only, +R, key, full).
                    "473" | "474" | "475" | "471" | "477" => {
                        let reason = parsed.params.last().cloned()
                            .unwrap_or_else(|| format!("cannot join {}", config.channel));
                        return SessionEnd::Lost { reason, registered };
                    }
                    _ => {}
                }
            }
            cmd = commands.recv() => match cmd {
                None | Some(IrcCommand::Shutdown) => {
                    let _ = send_line(&mut write, "QUIT :leaving").await;
                    return SessionEnd::Shutdown;
                }
                Some(IrcCommand::Reconfigure(c)) => {
                    let _ = send_line(&mut write, "QUIT :reconnecting").await;
                    return SessionEnd::Reconfigure(c);
                }
                Some(IrcCommand::SendChat(text)) => {
                    for msg in format_privmsg(&config.channel, &text) {
                        if let Err(e) = send_line(&mut write, &msg).await {
                            return SessionEnd::Lost { reason: e, registered };
                        }
                    }
                }
            },
        }
    }
}

/// Send the NICK + USER registration pair.
async fn register<W: AsyncWrite + Unpin>(write: &mut W, nick: &str) -> Result<(), String> {
    send_line(write, &format!("NICK {nick}")).await?;
    send_line(write, &format!("USER {nick} 0 * :{nick}")).await?;
    Ok(())
}

/// Write one IRC line, appending CRLF.
async fn send_line<W: AsyncWrite + Unpin>(write: &mut W, line: &str) -> Result<(), String> {
    tracing::trace!(line = %line, "IRC >>");
    write
        .write_all(format!("{line}\r\n").as_bytes())
        .await
        .map_err(|e| e.to_string())
}

/// Grow the backoff geometrically, capped.
fn grow_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

// --- Pure protocol helpers (unit-tested in isolation) ---

/// A parsed IRC line: optional prefix, command, and parameters (the
/// trailing `:`-param is the last element).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedLine {
    /// The source prefix (`nick!user@host` or a server name), if present.
    pub prefix: Option<String>,
    /// The command or numeric reply.
    pub command: String,
    /// The parameters, trailing `:`-param included as the last entry.
    pub params: Vec<String>,
}

/// Parse a raw IRC line. Returns `None` for blank/malformed lines.
pub(crate) fn parse_line(raw: &str) -> Option<ParsedLine> {
    let mut rest = raw.trim_end_matches(['\r', '\n']).trim_start();
    if rest.is_empty() {
        return None;
    }
    let prefix = if let Some(stripped) = rest.strip_prefix(':') {
        let end = stripped.find(' ')?;
        let prefix = stripped[..end].to_string();
        rest = stripped[end + 1..].trim_start();
        Some(prefix)
    } else {
        None
    };
    let (command, mut rest) = match rest.find(' ') {
        Some(i) => (rest[..i].to_string(), rest[i + 1..].trim_start()),
        None => (rest.to_string(), ""),
    };
    if command.is_empty() {
        return None;
    }
    let mut params = Vec::new();
    while !rest.is_empty() {
        if let Some(trailing) = rest.strip_prefix(':') {
            params.push(trailing.to_string());
            break;
        }
        match rest.find(' ') {
            Some(i) => {
                params.push(rest[..i].to_string());
                rest = rest[i + 1..].trim_start();
            }
            None => {
                params.push(rest.to_string());
                break;
            }
        }
    }
    Some(ParsedLine {
        prefix,
        command,
        params,
    })
}

/// The nick portion of a prefix (`nick!user@host` → `nick`).
pub(crate) fn nick_of_prefix(prefix: &str) -> &str {
    let end = prefix
        .find('!')
        .or_else(|| prefix.find('@'))
        .unwrap_or(prefix.len());
    &prefix[..end]
}

/// True if a char is legal in an IRC nick body.
fn is_nick_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "-[]\\`_^{}|".contains(c)
}

/// Derive the bridge nick from a username: keep IRC-legal characters,
/// ensure a letter leads, append the `Dess` suffix, and cap the length
/// while always preserving the suffix.
pub(crate) fn derive_nick(username: &str) -> String {
    let mut base: String = username.chars().filter(|c| is_nick_char(*c)).collect();
    // Nicks must start with a letter; drop any leading digits/specials.
    while base
        .chars()
        .next()
        .is_some_and(|c| !c.is_ascii_alphabetic())
    {
        base.remove(0);
    }
    let max_base = MAX_NICK.saturating_sub(BRIDGE_SUFFIX.len());
    if base.len() > max_base {
        base.truncate(max_base);
    }
    format!("{base}{BRIDGE_SUFFIX}")
}

/// True if a nick belongs to a dessplay bridge (ends in `Dess`,
/// case-insensitively). Such nicks are dropped on receive to avoid
/// double-displaying dessplay users already present via CRDT sync.
///
/// Heuristic: a genuine IRC user whose nick ends in "dess" (e.g.
/// "Goddess") is also dropped. Accepted — refining it would require the
/// dessplay roster, which this actor deliberately does not hold.
pub(crate) fn is_bridge_nick(nick: &str) -> bool {
    let n = nick.len();
    n >= BRIDGE_SUFFIX.len() && nick[n - BRIDGE_SUFFIX.len()..].eq_ignore_ascii_case(BRIDGE_SUFFIX)
}

/// Pick the next nick after a 433 collision, keeping the `Dess` suffix
/// terminal (e.g. `BaughnDess` → `Baughn2Dess` → `Baughn3Dess`). The
/// terminal suffix is load-bearing: it is how *other* bridges recognize
/// and drop this one, so a `BaughnDess_` form would cause double display.
pub(crate) fn next_nick_on_collision(current: &str) -> String {
    let base = current.strip_suffix(BRIDGE_SUFFIX).unwrap_or(current);
    let stem_len = base.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (stem, digits) = base.split_at(stem_len);
    let next = digits.parse::<u64>().unwrap_or(1) + 1;
    let next = next.to_string();
    let max_stem = MAX_NICK.saturating_sub(BRIDGE_SUFFIX.len() + next.len());
    let mut stem = stem.to_string();
    if stem.len() > max_stem {
        stem.truncate(max_stem);
    }
    format!("{stem}{next}{BRIDGE_SUFFIX}")
}

/// Build the PRIVMSG line(s) for a chat message: split on newlines into
/// separate messages, byte-chunk long lines on char boundaries, and keep
/// CTCP-wrapped messages (`\x01…\x01`) intact.
pub(crate) fn format_privmsg(channel: &str, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for segment in text.split('\n') {
        let segment = segment.trim_end_matches('\r');
        if segment.is_empty() {
            continue;
        }
        // A CTCP message must not be split mid-control.
        if segment.starts_with('\u{1}') || segment.len() <= MAX_PRIVMSG_TEXT {
            out.push(format!("PRIVMSG {channel} :{segment}"));
        } else {
            for chunk in chunk_str(segment, MAX_PRIVMSG_TEXT) {
                out.push(format!("PRIVMSG {channel} :{chunk}"));
            }
        }
    }
    out
}

/// Split a string into substrings of at most `max` bytes, on char
/// boundaries.
fn chunk_str(s: &str, max: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() - start > max && i > start {
            chunks.push(&s[start..i]);
            start = i;
        }
    }
    if start < s.len() {
        chunks.push(&s[start..]);
    }
    chunks
}

/// Turn a parsed PRIVMSG into an [`IrcEvent::Message`], or `None` if it
/// isn't a channel message we should display (wrong target, or a bridge
/// nick we drop to avoid double display).
fn privmsg_event(parsed: &ParsedLine, channel: &str) -> Option<IrcEvent> {
    let from = nick_of_prefix(parsed.prefix.as_deref()?).to_string();
    let target = parsed.params.first()?;
    let body = parsed.params.get(1)?;
    if !target.eq_ignore_ascii_case(channel) || is_bridge_nick(&from) {
        return None;
    }
    let (text, action) = match decode_action(body) {
        Some(phrase) => (strip_controls(phrase), true),
        None => (strip_controls(body), false),
    };
    Some(IrcEvent::Message { from, text, action })
}

/// Strip mIRC/IRC formatting control codes (bold, color, reset, the CTCP
/// `\x01` marker, …) so they don't garble the TUI.
fn strip_controls(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            // mIRC color: \x03[fg[,bg]] — drop the digits too.
            '\u{3}' => {
                skip_color_digits(&mut chars);
                if chars.peek() == Some(&',') {
                    let mut probe = chars.clone();
                    probe.next();
                    if probe.peek().is_some_and(char::is_ascii_digit) {
                        chars.next();
                        skip_color_digits(&mut chars);
                    }
                }
            }
            // Other control bytes (bold/italic/underline/reverse/reset/
            // CTCP marker and anything else below space): drop.
            c if (c as u32) < 0x20 => {}
            c => out.push(c),
        }
    }
    out
}

/// Consume up to two leading ASCII digits from an mIRC color code.
fn skip_color_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    for _ in 0..2 {
        if chars.peek().is_some_and(char::is_ascii_digit) {
            chars.next();
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use dessplay_core::types::encode_action;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    #[test]
    fn parses_privmsg() {
        let parsed = parse_line(":Nero!u@host PRIVMSG #dess :hi there\r\n").unwrap();
        assert_eq!(parsed.prefix.as_deref(), Some("Nero!u@host"));
        assert_eq!(parsed.command, "PRIVMSG");
        assert_eq!(parsed.params, vec!["#dess", "hi there"]);
    }

    #[test]
    fn parses_ping_and_numerics() {
        let ping = parse_line("PING :tantalum.rizon.net").unwrap();
        assert_eq!(ping.command, "PING");
        assert_eq!(ping.params, vec!["tantalum.rizon.net"]);

        let welcome = parse_line(":srv 001 BaughnDess :Welcome").unwrap();
        assert_eq!(welcome.command, "001");

        let inuse = parse_line(":srv 433 * BaughnDess :Nickname is already in use").unwrap();
        assert_eq!(inuse.command, "433");
    }

    #[test]
    fn parses_malformed_as_none() {
        assert!(parse_line("").is_none());
        assert!(parse_line("   ").is_none());
        assert!(parse_line(":only-a-prefix").is_none());
    }

    #[test]
    fn nick_of_prefix_extracts_nick() {
        assert_eq!(nick_of_prefix("Nero!user@host"), "Nero");
        assert_eq!(nick_of_prefix("Nero@host"), "Nero");
        assert_eq!(nick_of_prefix("Nero"), "Nero");
        assert_eq!(nick_of_prefix("irc.server.net"), "irc.server.net");
    }

    #[test]
    fn derive_nick_appends_suffix_and_sanitizes() {
        assert_eq!(derive_nick("Baughn"), "BaughnDess");
        // Spaces and illegal chars dropped.
        assert_eq!(derive_nick("Kim Possible!"), "KimPossibleDess");
        // Leading non-letters dropped.
        assert_eq!(derive_nick("3vil"), "vilDess");
        // Empty / all-illegal base still yields a legal nick.
        assert_eq!(derive_nick("   "), "Dess");
    }

    #[test]
    fn derive_nick_caps_length_keeping_suffix() {
        let nick = derive_nick(&"a".repeat(100));
        assert!(nick.len() <= MAX_NICK);
        assert!(nick.ends_with(BRIDGE_SUFFIX));
    }

    #[test]
    fn is_bridge_nick_matches_dess_suffix() {
        assert!(is_bridge_nick("NeroDess"));
        assert!(is_bridge_nick("nerodess")); // case-insensitive
        assert!(is_bridge_nick("Dess"));
        assert!(!is_bridge_nick("Nero"));
        assert!(!is_bridge_nick("Des"));
        // Documented false positive.
        assert!(is_bridge_nick("Goddess"));
    }

    #[test]
    fn collision_keeps_dess_terminal() {
        let a = next_nick_on_collision("BaughnDess");
        assert_eq!(a, "Baughn2Dess");
        let b = next_nick_on_collision(&a);
        assert_eq!(b, "Baughn3Dess");
        assert!(b.ends_with(BRIDGE_SUFFIX));
    }

    #[test]
    fn format_privmsg_splits_newlines_and_long_lines() {
        let lines = format_privmsg("#dess", "one\ntwo");
        assert_eq!(lines, vec!["PRIVMSG #dess :one", "PRIVMSG #dess :two"]);

        // Empty segments are skipped.
        assert_eq!(format_privmsg("#dess", "\n\n"), Vec::<String>::new());

        let long = "x".repeat(1000);
        let chunks = format_privmsg("#dess", &long);
        assert!(chunks.len() > 1);
        for line in &chunks {
            assert!(line.len() <= "PRIVMSG #dess :".len() + MAX_PRIVMSG_TEXT);
        }
    }

    #[test]
    fn format_privmsg_keeps_ctcp_intact() {
        let action = encode_action("waves enthusiastically");
        let lines = format_privmsg("#dess", &action);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains('\u{1}'));
    }

    #[test]
    fn ctcp_action_round_trips_through_the_wire_form() {
        // Our encode_action produces exactly the IRC CTCP ACTION form.
        let wire = encode_action("waves");
        assert_eq!(decode_action(&wire), Some("waves"));
    }

    #[test]
    fn privmsg_event_filters_and_decodes() {
        let chan = "#dess";
        // External user → event.
        let p = parse_line(":Nero!u@h PRIVMSG #dess :hello").unwrap();
        assert_eq!(
            privmsg_event(&p, chan),
            Some(IrcEvent::Message {
                from: "Nero".into(),
                text: "hello".into(),
                action: false
            })
        );
        // Bridge nick → dropped.
        let p = parse_line(":BaughnDess!u@h PRIVMSG #dess :echo").unwrap();
        assert_eq!(privmsg_event(&p, chan), None);
        // Wrong target → dropped.
        let p = parse_line(":Nero!u@h PRIVMSG BaughnDess :dm").unwrap();
        assert_eq!(privmsg_event(&p, chan), None);
        // CTCP ACTION → decoded with action flag.
        let action = format!(":Nero!u@h PRIVMSG #dess :{}", encode_action("waves"));
        let p = parse_line(&action).unwrap();
        assert_eq!(
            privmsg_event(&p, chan),
            Some(IrcEvent::Message {
                from: "Nero".into(),
                text: "waves".into(),
                action: true
            })
        );
    }

    #[test]
    fn strip_controls_removes_formatting() {
        assert_eq!(strip_controls("\u{2}bold\u{f}"), "bold");
        assert_eq!(strip_controls("\u{3}04red\u{3}"), "red");
        assert_eq!(strip_controls("\u{3}04,01both"), "both");
        assert_eq!(strip_controls("plain"), "plain");
    }

    #[test]
    fn backoff_grows_and_caps() {
        let mut b = INITIAL_BACKOFF;
        b = grow_backoff(b);
        assert_eq!(b, Duration::from_secs(4));
        for _ in 0..20 {
            b = grow_backoff(b);
        }
        assert_eq!(b, MAX_BACKOFF);
    }

    // --- Actor-loop tests over an in-memory duplex pipe ---

    fn test_config() -> IrcConfig {
        IrcConfig {
            enabled: true,
            server: "test".into(),
            port: 6697,
            tls: false,
            channel: "#dess".into(),
            nick: "BaughnDess".into(),
        }
    }

    /// Spawn `run_session` against one end of a duplex pipe; return the
    /// command sender, event receiver, a line reader for the server side,
    /// the server-side writer, and the session task handle.
    #[allow(clippy::type_complexity)]
    fn spawn_session() -> (
        mpsc::Sender<IrcCommand>,
        mpsc::Receiver<IrcEvent>,
        tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
        tokio::io::WriteHalf<tokio::io::DuplexStream>,
        tokio::task::JoinHandle<SessionEnd>,
    ) {
        let (client, server) = tokio::io::duplex(8192);
        let (sr, sw) = tokio::io::split(server);
        let server_lines = BufReader::new(sr).lines();
        let (cmd_tx, mut cmd_rx) = mpsc::channel(8);
        let (ev_tx, ev_rx) = mpsc::channel(8);
        let cfg = test_config();
        let handle = tokio::spawn(async move {
            let mut nick = cfg.nick.clone();
            run_session(client, &cfg, &mut nick, &mut cmd_rx, &ev_tx).await
        });
        (cmd_tx, ev_rx, server_lines, sw, handle)
    }

    async fn next_line(
        lines: &mut tokio::io::Lines<BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>>,
    ) -> String {
        tokio::time::timeout(Duration::from_secs(2), lines.next_line())
            .await
            .expect("timed out reading a line")
            .expect("io error")
            .expect("stream closed")
    }

    async fn write_server(write: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>, line: &str) {
        write
            .write_all(format!("{line}\r\n").as_bytes())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn registers_joins_and_emits_connected() {
        let (_cmd, mut events, mut lines, mut server, _h) = spawn_session();
        assert_eq!(next_line(&mut lines).await, "NICK BaughnDess");
        assert_eq!(
            next_line(&mut lines).await,
            "USER BaughnDess 0 * :BaughnDess"
        );
        write_server(&mut server, ":srv 001 BaughnDess :Welcome").await;
        assert_eq!(next_line(&mut lines).await, "JOIN #dess");
        assert_eq!(events.recv().await, Some(IrcEvent::Connected));
    }

    #[tokio::test]
    async fn forwards_external_messages_and_drops_bridges() {
        let (_cmd, mut events, mut lines, mut server, _h) = spawn_session();
        let _ = next_line(&mut lines).await; // NICK
        let _ = next_line(&mut lines).await; // USER
        write_server(&mut server, ":srv 001 BaughnDess :Welcome").await;
        let _ = next_line(&mut lines).await; // JOIN
        assert_eq!(events.recv().await, Some(IrcEvent::Connected));

        // A bridge nick is dropped; the following external one is the
        // first message event we observe.
        write_server(&mut server, ":NeroDess!u@h PRIVMSG #dess :echo").await;
        write_server(&mut server, ":Tomoko!u@h PRIVMSG #dess :hi all").await;
        assert_eq!(
            events.recv().await,
            Some(IrcEvent::Message {
                from: "Tomoko".into(),
                text: "hi all".into(),
                action: false
            })
        );
    }

    #[tokio::test]
    async fn answers_ping() {
        let (_cmd, _events, mut lines, mut server, _h) = spawn_session();
        let _ = next_line(&mut lines).await; // NICK
        let _ = next_line(&mut lines).await; // USER
        write_server(&mut server, "PING :tantalum.rizon.net").await;
        assert_eq!(next_line(&mut lines).await, "PONG :tantalum.rizon.net");
    }

    #[tokio::test]
    async fn sends_chat_as_privmsg() {
        let (cmd, _events, mut lines, mut server, _h) = spawn_session();
        let _ = next_line(&mut lines).await; // NICK
        let _ = next_line(&mut lines).await; // USER
        write_server(&mut server, ":srv 001 BaughnDess :Welcome").await;
        let _ = next_line(&mut lines).await; // JOIN
        cmd.send(IrcCommand::SendChat("hello world".into()))
            .await
            .unwrap();
        assert_eq!(next_line(&mut lines).await, "PRIVMSG #dess :hello world");
    }

    #[tokio::test]
    async fn retries_nick_on_collision() {
        let (_cmd, _events, mut lines, mut server, _h) = spawn_session();
        assert_eq!(next_line(&mut lines).await, "NICK BaughnDess");
        let _ = next_line(&mut lines).await; // USER
        write_server(
            &mut server,
            ":srv 433 * BaughnDess :Nickname is already in use",
        )
        .await;
        let retry = next_line(&mut lines).await;
        assert_eq!(retry, "NICK Baughn2Dess");
        assert!(retry.ends_with("Dess"));
    }

    #[tokio::test]
    async fn shutdown_quits_and_exits() {
        let (cmd, _events, mut lines, _server, handle) = spawn_session();
        let _ = next_line(&mut lines).await; // NICK
        let _ = next_line(&mut lines).await; // USER
        cmd.send(IrcCommand::Shutdown).await.unwrap();
        assert_eq!(next_line(&mut lines).await, "QUIT :leaving");
        assert!(matches!(handle.await.unwrap(), SessionEnd::Shutdown));
    }
}
