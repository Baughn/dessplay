//! The AI commentary engine (design.md, AI Commentary). Just for fun.
//!
//! On a settings-driven interval — jittered ±15s so it isn't on the
//! dot, and only while connected, playing, and holding the now-playing
//! file — the engine asks an Anthropic model to react to the episode
//! **in character**: a persistent "commentator" chosen from the show's
//! cast, re-rolled with 5% probability per tick and **never** reset on
//! a series change — the voice follows the group to the next show
//! (Hinamori Amu commenting on Grave of the Fireflies is a feature)
//! until the dice or a restart retire it. Each commentator is a real
//! **chat thread**: the character
//! card lives in the system prompt, and every tick appends a user turn
//! carrying only the dialogue that arrived *since the last comment*
//! (never resent — the advisor ring's per-line sequence numbers are the
//! cursor) plus the current video frame, so the model remembers what it
//! already said. A fresh commentator starts a fresh thread, seeded with
//! the *text* of this episode's earlier comments — not the images or
//! subtitles — so the voice changes but the conversation doesn't reset
//! to zero. The 5% re-roll keeps threads young in expectation; because
//! its tail is geometric, a hard cap backs it up — a thread that
//! reaches [`MAX_THREAD_TURNS`] turns force-re-rolls on the next tick,
//! through the same fresh-thread path the dice take. Sent history is
//! **append-only**: a turn, once sent, is never rewritten or trimmed
//! (the prompt cache below matches on a byte-stable prefix, so the only
//! way to shed old turns — and their heavy frames — is to end the
//! thread), and [`MAX_THREAD_FRAME_BYTES`] sends a turn frameless
//! rather than let the accumulated screenshots outgrow the API's
//! request-size cap.
//!
//! Requests opt into Anthropic's ephemeral prompt cache whenever the
//! interval (jitter included) fits inside the cache's 5-minute TTL —
//! which is why the settings ladder offers 4:00 rather than 5:00 — so
//! the growing thread re-bills at cache-read rates instead of full
//! price. The reply (`<Amu> Whaaaat?`) is written to the synced
//! [`marquee register`](dessplay_core::state::CrdtState::marquee), so
//! every client scrolls it — including this one, via the ordinary sync
//! echo, which keeps all replicas showing identical text.
//!
//! The feature narrates itself at **info**: whether it is enabled (at
//! startup and on every settings change, with the reason when it is
//! not), each outgoing request, the commentator it picked, each call's
//! token usage (cached and fresh), and the comment that came back. A
//! gimmick that only speaks once every few minutes is otherwise
//! indistinguishable from a broken token; skipped ticks (paused, no
//! file, gated) log their reason at debug.
//!
//! Nothing here blocks the bridge loop: the HTTP calls run under
//! [`tokio::task::spawn_blocking`], results come back through a
//! channel, and an in-flight guard keeps slow calls from stacking.
//! Every failure (HTTP, refusal, empty cast, malformed reply) is a
//! `tracing::warn!` and a skipped tick — never a chat line, never a
//! marquee write. A failed tick also leaves the subtitle cursor and
//! thread untouched, so the next attempt resends what the model never
//! saw. The screenshot is best-effort on top of best-effort: its
//! absence is not a failure.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tokio::sync::mpsc;

use crate::advisor::AdvisorContext;
use crate::config::CommentaryInterval;

/// The model every call uses. Hardcoded on purpose: this is a
/// single-user gimmick, not a configuration surface.
const MODEL: &str = "claude-opus-4-6";
/// Thinking effort for both calls — the task is short and low-stakes.
/// Paired with `thinking: {type: "adaptive"}` (the recommended shape on
/// the pinned model and the only accepted on-mode on newer ones — the
/// old fixed-budget `{type: "enabled", budget_tokens}` shape is
/// deprecated on claude-opus-4-6 and a 400 on anything newer, so a
/// routine model bump must never resurrect it).
const EFFORT: &str = "low";
/// Caps thinking *plus* text: adaptive thinking spends out of the same
/// `max_tokens` budget as the reply, so it must not be lowballed.
const MAX_TOKENS: u32 = 3000;
const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// Vision calls are slow; the nyaa agent's 30s would spuriously abort.
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);
/// Chance per tick of re-rolling the commentator once one exists. The
/// geometric expectation is ≈ 20 turns, but the tail is unbounded —
/// [`MAX_THREAD_TURNS`] is the hard governor.
const REROLL: f64 = 0.05;
/// Half-width of the per-comment cadence jitter.
const JITTER: Duration = Duration::from_secs(15);
/// Anthropic's default ephemeral prompt-cache TTL. Intervals short
/// enough that the next (jittered) request still lands inside it opt
/// into `cache_control`; longer ones skip the write surcharge for a
/// cache that would be cold anyway.
const CACHE_TTL: Duration = Duration::from_secs(300);
/// Subtitle tail handed to the character-list call (context, not script).
const CHARACTER_SUBTITLES: usize = 20;
/// Hard cap on the marquee line, chars (the slot scrolls, but an essay
/// would take a minute to cross the screen).
const MAX_COMMENT_CHARS: usize = 220;
/// How long to wait for mpv to finish writing the screenshot.
const SCREENSHOT_POLLS: u32 = 20;
const SCREENSHOT_POLL_INTERVAL: Duration = Duration::from_millis(100);
/// Hard cap on a thread's length, in completed turns: at the cap the
/// next tick force-re-rolls the commentator (the same fresh-thread
/// path the 5% dice take, comment-text seeding included). This — not a
/// trim — is what bounds request size, because sent history must stay
/// **append-only**: the prompt cache is strict-prefix over the request
/// bytes, and an earlier design that stripped old turns' screenshots
/// rewrote the cached prefix every tick, collapsing
/// `cache_read_input_tokens` to ~0 whenever frames flowed (2026-08-20
/// review). The arithmetic behind 10: mpv JPEG frames run ~0.3–0.8 MB
/// (×4/3 as base64), so the top-of-thread request is ~4–11 MB — a few
/// seconds of uplink once per ≥2-minute tick, the same order as the
/// old two-frame trim's bodies, where the uncapped geometric tail
/// (60+ turns) meant tens of MB. Token-wise the capped prefix is
/// ~18K tokens (~1600/image + ~200/turn of text), re-billed at
/// cache-read rates (10% of input price) precisely because the prefix
/// no longer changes. And 0.95^10 ≈ 0.60, so the dice still retire
/// ~40% of threads before the cap ever fires — the cap kills the
/// tail, not the gimmick's feel.
const MAX_THREAD_TURNS: usize = 10;
/// Budget for the *total* screenshot bytes a thread may accumulate
/// (history plus the new turn): a frame, once sent, rides every later
/// request until the thread cap ends the thread, so the sum — not the
/// per-frame size — is what must stay inside the API's 32 MB request
/// cap. Two worst-case frames (the allowance the old trim gave):
/// 15 MB raw → 20 MB base64, comfortably under the cap even in the
/// format-surprise case [`MAX_SCREENSHOT_BYTES`] exists for; typical
/// JPEG threads (≤ [`MAX_THREAD_TURNS`] × ~0.5 MB ≈ 5 MB) never touch
/// it. A turn over budget goes out frameless — losing an image beats
/// losing the comment, and beats mutating history to make room.
const MAX_THREAD_FRAME_BYTES: u64 = 2 * MAX_SCREENSHOT_BYTES;
/// Ceiling on screenshot bytes attached to a request. The API rejects
/// any image whose *base64* exceeds 10MiB (observed 2026-07-26: mpv
/// writes 16-bit PNGs for 10-bit video, ~8MB raw → 11MB base64 → HTTP
/// 400 on every busy frame); 7.5MB raw stays under the cap after the
/// 4/3 expansion. JPEG frames never come near this — the guard covers
/// format surprises.
const MAX_SCREENSHOT_BYTES: u64 = 7_500_000;

/// Why a commentary attempt produced nothing. All variants are logged
/// and skipped; none are user-visible.
#[derive(Debug)]
pub enum CommentaryError {
    /// Transport or non-2xx response.
    Http(String),
    /// A 2xx response we could not interpret.
    Api(String),
    /// The model declined (`stop_reason: refusal`).
    Refused,
    /// The character list came back empty.
    NoCharacters,
    /// The blocking job panicked. Mapped to an ordinary failure so the
    /// in-flight guard always releases (a dropped, unsent result sender
    /// used to latch it forever — 2026-08-12 review).
    Panicked(String),
}

impl std::fmt::Display for CommentaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommentaryError::Http(e) => write!(f, "http: {e}"),
            CommentaryError::Api(e) => write!(f, "api: {e}"),
            CommentaryError::Refused => write!(f, "model refused"),
            CommentaryError::NoCharacters => write!(f, "empty character list"),
            CommentaryError::Panicked(msg) => write!(f, "job panicked: {msg}"),
        }
    }
}

/// Inputs to the character-list call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharacterRequest {
    /// Series title (AniDB name, or the filename-derived fallback).
    pub series: String,
    /// Episode label, when metadata knows it.
    pub episode: Option<String>,
    /// A short subtitle tail, to disambiguate the series.
    pub recent_subtitles: Vec<String>,
}

/// One completed exchange in a commentator's thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadTurn {
    /// The user-side text (header, seeded comments, new dialogue).
    pub user_text: String,
    /// The frame attached to that turn, if any.
    pub screenshot: Option<Vec<u8>>,
    /// The model's raw reply, echoed back verbatim as the assistant
    /// turn (the API's multi-turn contract).
    pub assistant: String,
}

/// Inputs to one comment call: the whole conversation so far plus the
/// new user turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentRequest {
    /// The stable per-thread system prompt (character card + rules).
    pub system: String,
    /// Prior turns of this commentator's thread, oldest first. Empty on
    /// a fresh thread.
    pub history: Vec<ThreadTurn>,
    /// This turn's user text.
    pub user_text: String,
    /// The current video frame (JPEG bytes), when mpv delivered one in
    /// time.
    pub screenshot: Option<Vec<u8>>,
    /// Attach a `cache_control` breakpoint (the interval is short
    /// enough for the ephemeral cache to survive to the next tick).
    pub cache: bool,
}

/// The model seam. Blocking — implementations are always called under
/// `spawn_blocking`; tests inject a scripted fake.
pub trait CommentaryModel: Send + Sync {
    /// Major characters through the current episode, spoiler-bounded.
    fn list_characters(&self, req: &CharacterRequest) -> Result<Vec<String>, CommentaryError>;
    /// A 1–3 sentence in-character reaction, given the thread so far.
    fn write_comment(&self, req: &CommentRequest) -> Result<String, CommentaryError>;
}

// ---- Prompts (pure; the load-bearing phrases are unit-tested) ----------

fn episode_label(episode: Option<&str>) -> String {
    match episode {
        Some(ep) => format!("episode {ep}"),
        None => "the current episode".into(),
    }
}

/// The character-list prompt. The spoiler bound is the load-bearing
/// clause: the model knows the whole series and must not leak it.
fn character_prompt(req: &CharacterRequest) -> String {
    let mut prompt = format!(
        "You are helping run a watch-party gimmick for the anime series \
         \"{series}\". The group is currently watching {episode}. List the \
         major characters who have appeared in the series up to and \
         including this episode ONLY — do not include anyone introduced \
         later, and reveal nothing about later events. If you are unsure \
         whether a character has appeared yet, leave them out.",
        series = req.series,
        episode = episode_label(req.episode.as_deref()),
    );
    if !req.recent_subtitles.is_empty() {
        prompt.push_str("\n\nRecent dialogue, for context:\n");
        for line in &req.recent_subtitles {
            prompt.push_str(line);
            prompt.push('\n');
        }
    }
    prompt.push_str("\nOutput one character name per line, nothing else.");
    prompt
}

/// Parse the character-list reply: one name per line, bullets and
/// numbering tolerated, capped to a sane cast size.
fn parse_characters(reply: &str) -> Vec<String> {
    reply
        .lines()
        .map(|line| {
            line.trim()
                .trim_start_matches(['-', '*', '•'])
                .trim_start_matches(|c: char| c.is_ascii_digit())
                .trim_start_matches(['.', ')'])
                .trim()
                .to_string()
        })
        .filter(|name| !name.is_empty())
        .take(20)
        .collect()
}

/// The per-thread system prompt: the character card, the spoiler bound,
/// and the output format. Stable for the thread's lifetime — the
/// episode changes ride the user turns as headers, so the cached prefix
/// never moves.
fn system_prompt(name: &str, series: &str) -> String {
    format!(
        "You are {name}, a character from \"{series}\". You are (initially) watching \
         your own show together with friends at a watch party, reacting in \
         an IRC channel as the episodes play. Hard rule: you know nothing \
         beyond the episode currently being watched — no future events, no \
         meta-knowledge, no winking at the audience.\n\n\
         Each message brings the newest subtitle lines since your last \
         comment, and usually the current video frame as an image. React in \
         character to what is happening right now: 1-3 short sentences, IRC \
         style. Always output exactly one line in the form `<{name}> your \
         comment` and nothing else, in English. 2-3 sentences max."
    )
}

/// Pieces of one user turn, assembled by the engine on the loop thread.
struct TurnInput<'a> {
    /// `Some` on a thread's first turn and on an episode change: the
    /// model is told what is now playing.
    header: Option<(&'a str, Option<&'a str>)>,
    /// Seeded on a fresh thread mid-episode: what earlier commentators
    /// already said (text only — their subtitles and frames stay
    /// behind).
    previous_comments: &'a [String],
    /// Only the dialogue that arrived since the last successful
    /// comment.
    subtitles: &'a [String],
}

/// Render one user turn's text block.
fn turn_text(input: &TurnInput<'_>) -> String {
    let mut text = String::new();
    if let Some((series, episode)) = input.header {
        text.push_str(&format!(
            "Now playing: \"{series}\", {}.\n\n",
            episode_label(episode)
        ));
    }
    if !input.previous_comments.is_empty() {
        text.push_str("Comments so far this episode, from earlier commentators:\n");
        for comment in input.previous_comments {
            text.push_str(comment);
            text.push('\n');
        }
        text.push('\n');
    }
    if input.subtitles.is_empty() {
        text.push_str("(No new dialogue since your last comment.)");
    } else {
        text.push_str("New dialogue, oldest first:\n");
        for line in input.subtitles {
            text.push_str(line);
            text.push('\n');
        }
    }
    text
}

/// Normalize a comment reply into a single marquee-safe line: newlines
/// collapse to spaces, the `<Name>` prefix is repaired if the model
/// dropped it, and the length is capped on a char boundary.
fn normalize_comment(name: &str, raw: &str) -> String {
    let flattened = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    let prefix = format!("<{name}>");
    let mut line = if flattened
        .get(..prefix.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(&prefix))
    {
        flattened
    } else {
        format!("{prefix} {flattened}")
    };
    if line.chars().count() > MAX_COMMENT_CHARS {
        line = line.chars().take(MAX_COMMENT_CHARS - 1).collect::<String>() + "…";
    }
    line
}

/// Pull the human-readable message out of an Anthropic error body
/// (`{"type":"error","error":{"message":…}}`), falling back to a
/// truncated raw snippet — a 4xx must always say *why*, not just that
/// it happened.
fn api_error_detail(body: &[u8]) -> String {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body)
        && let Some(msg) = v["error"]["message"].as_str()
    {
        return msg.to_string();
    }
    String::from_utf8_lossy(body).chars().take(200).collect()
}

// ---- Request bodies (pure; the shapes are unit-tested) ------------------

/// One user message's content blocks: the frame (if any), then the
/// text. `cache` puts the ephemeral breakpoint on the text block —
/// only ever set on the *final* message, the standard incremental
/// multi-turn caching pattern (the marker moves forward each tick; the
/// prior prefix stays readable).
fn user_content(text: &str, screenshot: Option<&[u8]>, cache: bool) -> serde_json::Value {
    let mut blocks = Vec::new();
    if let Some(frame) = screenshot {
        blocks.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": "image/jpeg",
                "data": base64::engine::general_purpose::STANDARD.encode(frame),
            },
        }));
    }
    let mut text_block = serde_json::json!({ "type": "text", "text": text });
    if cache {
        text_block["cache_control"] = serde_json::json!({ "type": "ephemeral" });
    }
    blocks.push(text_block);
    serde_json::Value::Array(blocks)
}

/// The full Messages body for a comment call: system prompt, the
/// thread's history verbatim (append-only, so the rendered prefix is
/// byte-stable across ticks — the caching invariant), and the new turn.
fn build_comment_body(req: &CommentRequest) -> serde_json::Value {
    let mut messages = Vec::new();
    for turn in &req.history {
        messages.push(serde_json::json!({
            "role": "user",
            "content": user_content(&turn.user_text, turn.screenshot.as_deref(), false),
        }));
        messages.push(serde_json::json!({
            "role": "assistant",
            "content": turn.assistant,
        }));
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": user_content(&req.user_text, req.screenshot.as_deref(), req.cache),
    }));
    serde_json::json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "thinking": {"type": "adaptive"},
        "output_config": { "effort": EFFORT },
        "system": [{ "type": "text", "text": req.system }],
        "messages": messages,
    })
}

/// The (single-turn, uncached) body for the character-list call. Same
/// deliberate thinking depth as the comment call ([`EFFORT`]) — one
/// documented setting for the whole feature, not two divergent shapes.
fn build_character_body(req: &CharacterRequest) -> serde_json::Value {
    serde_json::json!({
        "model": MODEL,
        "max_tokens": MAX_TOKENS,
        "thinking": {"type": "adaptive"},
        "output_config": { "effort": EFFORT },
        "messages": [{ "role": "user", "content": character_prompt(req) }],
    })
}

// ---- The Anthropic implementation ---------------------------------------

/// The real model client. Deliberately no `Debug` impl — it holds the
/// API token.
pub struct AnthropicModel {
    agent: ureq::Agent,
    token: String,
}

impl AnthropicModel {
    /// A client for the given API token.
    pub fn new(token: String) -> Self {
        Self {
            agent: ureq::Agent::from(
                ureq::config::Config::builder()
                    .timeout_global(Some(HTTP_TIMEOUT))
                    // A 4xx must reach us as a response, not an error:
                    // the body names the offending field, and ureq's
                    // status-as-error discards it (a bare "http status:
                    // 400" was undiagnosable, 2026-07-26).
                    .http_status_as_error(false)
                    .build(),
            ),
            token,
        }
    }

    /// One Messages call; returns the first text block. `what` labels
    /// the token-usage log line ("cast" / "comment").
    fn call(&self, body: &serde_json::Value, what: &str) -> Result<String, CommentaryError> {
        let bytes = serde_json::to_vec(body)
            .map_err(|e| CommentaryError::Api(format!("encoding request: {e}")))?;
        let response = self
            .agent
            .post(API_URL)
            .header("x-api-key", &self.token)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .send(&bytes[..])
            .map_err(|e| CommentaryError::Http(e.to_string()))?;
        let status = response.status();
        let reply = response
            .into_body()
            .with_config()
            .limit(4 * 1024 * 1024)
            .read_to_vec()
            .map_err(|e| CommentaryError::Http(format!("reading response: {e}")))?;
        if !status.is_success() {
            return Err(CommentaryError::Http(format!(
                "status {}: {}",
                status.as_u16(),
                api_error_detail(&reply)
            )));
        }
        let reply: serde_json::Value = serde_json::from_slice(&reply)
            .map_err(|e| CommentaryError::Api(format!("parsing response: {e}")))?;
        // Token accounting at info: the caching setup is invisible
        // otherwise, and "is the cache hitting?" is one grep away.
        let usage = &reply["usage"];
        tracing::info!(
            call = what,
            input_tokens = usage["input_tokens"].as_u64().unwrap_or(0),
            output_tokens = usage["output_tokens"].as_u64().unwrap_or(0),
            cache_read_tokens = usage["cache_read_input_tokens"].as_u64().unwrap_or(0),
            cache_write_tokens = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0),
            "commentary: token usage"
        );
        if reply["stop_reason"].as_str() == Some("refusal") {
            return Err(CommentaryError::Refused);
        }
        reply["content"]
            .as_array()
            .and_then(|blocks| {
                blocks
                    .iter()
                    .find(|block| block["type"].as_str() == Some("text"))
            })
            .and_then(|block| block["text"].as_str())
            .map(str::to_string)
            .ok_or_else(|| CommentaryError::Api("no text block in response".into()))
    }
}

impl CommentaryModel for AnthropicModel {
    fn list_characters(&self, req: &CharacterRequest) -> Result<Vec<String>, CommentaryError> {
        let reply = self.call(&build_character_body(req), "cast")?;
        let names = parse_characters(&reply);
        if names.is_empty() {
            return Err(CommentaryError::NoCharacters);
        }
        Ok(names)
    }

    fn write_comment(&self, req: &CommentRequest) -> Result<String, CommentaryError> {
        self.call(&build_comment_body(req), "comment")
    }
}

// ---- The engine ----------------------------------------------------------

/// The persistent voice: kept across ticks (and API failures, and even
/// series changes), re-rolled with [`REROLL`] probability.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Commentator {
    name: String,
    /// The character's *home* series — the show they were picked from,
    /// which stays fixed even when the group moves on to another one.
    series: String,
}

/// A commentator's conversation: the voice plus every exchange so far.
/// Lives exactly as long as the voice does — only a re-roll starts a
/// fresh one; episode and series changes stay in-thread.
#[derive(Clone, Debug)]
struct Thread {
    commentator: Commentator,
    turns: Vec<ThreadTurn>,
}

/// What the bridge loop knows at tick time; the engine turns it into a
/// go/no-go decision. All gates must hold — commentary about a paused
/// screen, or from a client without the file, would be nonsense.
#[derive(Clone, Debug, Default)]
pub struct TickGates {
    /// The server link is up (the marquee write must sync).
    pub connected: bool,
    /// Video is actually running for the group.
    pub playing: bool,
    /// This client holds the now-playing file (its screenshots and
    /// subtitle ring describe the real video, not a placeholder).
    pub holds_file: bool,
    /// The now-playing series name, when known (filename-derived
    /// fallbacks qualify — the model is told what we know).
    pub series: Option<String>,
}

/// The decision for one tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TickPlan {
    /// Ask for the cast and pick a fresh commentator (a fresh thread)
    /// first.
    pub pick_commentator: bool,
}

/// Identity of "what is playing" for the episode-change header and the
/// comment-seed scope: series name, episode label, and the now-playing
/// **filename**. The filename is the load-bearing part for unlinked
/// series — AniDB-unknown files all get `episode: None` and share one
/// hint-derived series name, so without it the key never changed
/// across such a series' episodes (2026-08-12 review).
type EpisodeKey = (String, Option<String>, Option<String>);

/// Everything the blocking job needs, gathered on the loop thread.
struct JobSpec {
    /// The voice to keep, or `None` to pick one (a fresh thread).
    keep: Option<Commentator>,
    /// Pre-drawn randomness for the cast pick.
    pick_index: u64,
    series: String,
    episode: Option<String>,
    /// The now-playing filename (the file half of [`EpisodeKey`]).
    filename: Option<String>,
    /// Ring tail for the character-list call (context, not the turn).
    cast_subtitles: Vec<String>,
    /// The already-rendered user turn.
    user_text: String,
    /// Prior turns; empty on a fresh thread.
    history: Vec<ThreadTurn>,
    cache: bool,
    /// Subtitle cursor value this turn covers; committed on success.
    seq: u64,
}

/// A finished job's payload. Opaque outside this module — the run loop
/// receives it from [`CommentaryEngine::results`] and hands it straight
/// to [`CommentaryEngine::finish`].
pub struct JobOutcome {
    /// The voice that spoke.
    commentator: Commentator,
    /// The job started a fresh thread (picked its voice).
    fresh: bool,
    /// The now-playing series the comment was made during — not
    /// necessarily the commentator's home series (a kept voice outlives
    /// a series change). Keys the episode-comment seed.
    series: String,
    episode: Option<String>,
    /// The now-playing filename (the file half of [`EpisodeKey`]).
    filename: Option<String>,
    /// The turn as sent, echoed back so the engine can append it to the
    /// thread only once it actually succeeded.
    user_text: String,
    screenshot: Option<Vec<u8>>,
    /// The raw reply (stored verbatim as the assistant turn).
    raw: String,
    /// The normalized marquee line.
    text: String,
    /// Subtitle cursor to commit.
    seq: u64,
}

/// Owns the cadence, the thread, the RNG, and the in-flight job.
/// Lives on the [`crate::run::SessionLoop`]; the loop select-arms
/// [`Self::ticker`] and [`Self::results`].
pub struct CommentaryEngine {
    model: Option<Arc<dyn CommentaryModel>>,
    interval: Option<Duration>,
    /// Ticks at the configured interval (re-jittered ±[`JITTER`] per
    /// tick); only armed when enabled.
    pub ticker: tokio::time::Interval,
    thread: Option<Thread>,
    /// Normalized comments for the currently playing episode, any
    /// voice — the seed for a fresh thread's first turn.
    episode_comments: Vec<String>,
    /// [`EpisodeKey`] of the last successful comment; a change resets
    /// [`Self::episode_comments`] and headers the next turn.
    episode_key: Option<EpisodeKey>,
    /// Subtitle cursor: highest ring sequence number already delivered.
    sent_seq: u64,
    rng: StdRng,
    in_flight: bool,
    results_tx: mpsc::Sender<Result<JobOutcome, CommentaryError>>,
    /// Finished jobs; the run loop select-arms this (a separate field
    /// from [`Self::ticker`] so the two arms borrow disjointly) and
    /// feeds each into [`Self::finish`].
    pub results: mpsc::Receiver<Result<JobOutcome, CommentaryError>>,
    /// Where the player writes screenshots (one stable path inside a
    /// private per-process directory, overwritten every tick). `None`
    /// when the directory could not be created — screenshots are
    /// disabled, commentary itself still runs.
    screenshots: Option<ScreenshotSlot>,
}

/// The screenshot drop point: one stable path (`frame.jpg`) inside an
/// engine-owned temporary directory, mode 0700 on Unix. A predictable
/// name in the shared, world-writable `$TMPDIR` was a symlink-following
/// exfiltration hazard (2026-08-12 review): `poll_screenshot` reads
/// whatever the path resolves to and ships it to the API, so the path
/// must live where only this user can plant anything. The path stays
/// stable across ticks within a session — mpv just overwrites it — and
/// the directory (and any leftover frame) is removed on drop.
struct ScreenshotSlot {
    path: PathBuf,
    /// Owns the directory; kept alive for the engine's lifetime.
    _dir: tempfile::TempDir,
}

impl ScreenshotSlot {
    fn create() -> std::io::Result<Self> {
        let mut builder = tempfile::Builder::new();
        builder.prefix("dessplay-commentary-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            builder.permissions(std::fs::Permissions::from_mode(0o700));
        }
        let dir = builder.tempdir()?;
        // .jpg drives mpv's format inference: a PNG of a 10-bit source
        // is 16-bit and ~8MB — past the API's image cap — where a JPEG
        // frame is a few hundred KB.
        let path = dir.path().join("frame.jpg");
        Ok(Self { path, _dir: dir })
    }
}

impl CommentaryEngine {
    /// An engine from the saved settings, seeded from entropy.
    pub fn from_settings(settings: &crate::config::Settings) -> Self {
        let engine = Self::new(
            settings
                .anthropic_token
                .clone()
                .map(|token| Arc::new(AnthropicModel::new(token)) as Arc<dyn CommentaryModel>),
            settings.commentary_interval,
            StdRng::from_os_rng(),
        );
        engine.log_state("configured");
        engine
    }

    /// Say, at info, whether commentary will run at all and why — the
    /// feature is otherwise silent until a comment lands a whole
    /// interval later, which reads exactly like a broken token.
    fn log_state(&self, when: &str) {
        match (self.model.is_some(), self.interval) {
            (true, Some(interval)) => tracing::info!(
                interval_secs = interval.as_secs(),
                "AI commentary {when}: enabled"
            ),
            (false, _) => tracing::info!("AI commentary {when}: disabled (no Anthropic token)"),
            (true, None) => tracing::info!("AI commentary {when}: disabled (interval off)"),
        }
    }

    /// A disabled engine (seeders, tests that don't exercise it).
    pub fn disabled() -> Self {
        Self::new(None, CommentaryInterval::Off, StdRng::seed_from_u64(0))
    }

    /// The general constructor; tests inject a fake model and a seeded
    /// RNG here.
    pub fn new(
        model: Option<Arc<dyn CommentaryModel>>,
        interval: CommentaryInterval,
        rng: StdRng,
    ) -> Self {
        let (results_tx, results) = mpsc::channel(4);
        let interval = interval.duration();
        Self {
            model,
            interval,
            ticker: Self::make_ticker(interval),
            thread: None,
            episode_comments: Vec::new(),
            episode_key: None,
            sent_seq: 0,
            rng,
            in_flight: false,
            results_tx,
            results,
            screenshots: ScreenshotSlot::create()
                .inspect_err(|e| {
                    // Never a reason to break commentary — the frame is
                    // best-effort on top of best-effort.
                    tracing::warn!(
                        "commentary: no private screenshot dir ({e}); screenshots disabled"
                    );
                })
                .ok(),
        }
    }

    fn make_ticker(interval: Option<Duration>) -> tokio::time::Interval {
        // A disarmed ticker still needs *some* period; the select arm is
        // gated on `armed()` so it never fires observable work.
        let mut ticker = tokio::time::interval(interval.unwrap_or(Duration::from_secs(3600)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Skip the interval's immediate first tick: the first comment
        // should come one full interval into playback, not at startup.
        ticker.reset();
        ticker
    }

    /// Whether the select arm should listen to the ticker at all.
    pub fn armed(&self) -> bool {
        self.model.is_some() && self.interval.is_some()
    }

    /// Apply a settings change live: swap the model on a token change,
    /// re-cadence on an interval change. Clearing the token drops the
    /// thread too (a fresh token starts fresh); an in-flight call
    /// finishes and its result is discarded if the engine is off by then.
    /// `in_flight` is deliberately *not* reset here: the job always
    /// delivers an outcome (panics included — see `spawn_job`'s
    /// catch_unwind) and [`Self::finish`] clears the guard, while
    /// resetting it under a live job would let calls stack — the very
    /// thing the guard exists to prevent.
    pub fn reconfigure(&mut self, token: Option<&str>, interval: CommentaryInterval) {
        self.model = token.map(|token| {
            Arc::new(AnthropicModel::new(token.to_string())) as Arc<dyn CommentaryModel>
        });
        if self.model.is_none() {
            self.thread = None;
            self.episode_comments.clear();
            self.episode_key = None;
            self.sent_seq = 0;
        }
        self.interval = interval.duration();
        self.ticker = Self::make_ticker(self.interval);
        self.log_state("reconfigured");
    }

    /// Where the player should write the screenshot for the next job;
    /// `None` when the private directory could not be created (the
    /// tick then simply goes out frameless).
    pub fn screenshot_path(&self) -> Option<PathBuf> {
        self.screenshots.as_ref().map(|slot| slot.path.clone())
    }

    /// The current voice, if any (tests peek at it).
    #[cfg(test)]
    fn commentator(&self) -> Option<&Commentator> {
        self.thread.as_ref().map(|t| &t.commentator)
    }

    /// Whether requests should carry the prompt-cache breakpoint: only
    /// when the next tick (worst-case jitter included) still lands
    /// inside the ephemeral cache's TTL.
    fn cache_worthwhile(&self) -> bool {
        self.interval.is_some_and(|i| i + JITTER < CACHE_TTL)
    }

    /// Re-arm the ticker with fresh per-comment jitter: the next fire
    /// lands `interval ± JITTER` from now, so comments drift off the
    /// dot instead of landing metronomically.
    fn rejitter(&mut self) {
        let Some(interval) = self.interval else {
            return;
        };
        let jitter = Duration::from_secs(self.rng.random_range(0..=2 * JITTER.as_secs()));
        self.ticker
            .reset_after((interval + jitter).saturating_sub(JITTER));
    }

    /// Decide what (if anything) this tick does. `None` = skip quietly.
    pub fn plan_tick(&mut self, gates: &TickGates) -> Option<TickPlan> {
        if !self.armed() {
            return None;
        }
        // Every fired tick re-jitters the next, comment or not.
        self.rejitter();
        if self.in_flight {
            tracing::debug!("commentary tick skipped: a call is still in flight");
            return None;
        }
        // Named so "why is nothing showing up?" is one RUST_LOG away; a
        // gated tick is the common case (paused, not holding the file),
        // so it stays at debug rather than info.
        if !gates.connected || !gates.playing || !gates.holds_file || gates.series.is_none() {
            tracing::debug!(
                connected = gates.connected,
                playing = gates.playing,
                holds_file = gates.holds_file,
                have_series = gates.series.is_some(),
                "commentary tick skipped: gated"
            );
            return None;
        }
        // A series change does NOT retire the voice: the commentator
        // follows the group to the next show (deliberate — see the
        // module docs) until the re-roll dice or a restart replaces
        // them. The episode-key machinery headers the new series and
        // resets the comment seed on its own.
        //
        // A thread at [`MAX_THREAD_TURNS`] force-re-rolls instead:
        // sent history is append-only (the caching invariant), so
        // ending the thread is the only way to shed its accumulated
        // turns. Checked before the dice, which are simply not rolled
        // on a capped tick.
        let at_cap = self
            .thread
            .as_ref()
            .is_some_and(|t| t.turns.len() >= MAX_THREAD_TURNS);
        let pick_commentator = self.thread.is_none() || at_cap || self.rng.random_bool(REROLL);
        Some(TickPlan { pick_commentator })
    }

    /// Launch the blocking job for a planned tick. `screenshot` is the
    /// path the player was asked to write plus the instant the request
    /// was issued (a frame whose mtime predates it is a stale leftover
    /// and is never attached), or `None` when no player is running
    /// (skip the poll entirely).
    pub fn spawn_job(
        &mut self,
        plan: TickPlan,
        ctx: &AdvisorContext,
        screenshot: Option<(PathBuf, std::time::SystemTime)>,
    ) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let Some(series) = ctx.series_name.clone() else {
            return;
        };
        self.in_flight = true;
        let keep = (!plan.pick_commentator)
            .then(|| self.thread.as_ref().map(|t| t.commentator.clone()))
            .flatten();
        let fresh = keep.is_none();
        // Drawn on the loop thread so the RNG (and its seed-determinism
        // in tests) never crosses into the blocking task.
        let pick_index: u64 = self.rng.random();
        let episode = ctx.episode.clone();
        let filename = ctx.filename.clone();
        let same_episode =
            self.episode_key.as_ref() == Some(&(series.clone(), episode.clone(), filename.clone()));
        // Only dialogue the model hasn't seen; the cursor commits when
        // the job succeeds, so a failed attempt resends. Rendered
        // speaker-attributed (`Name: line`) — the model can't see the
        // video, so attribution is all it gets.
        let new_subtitles: Vec<String> = ctx
            .subtitles
            .iter()
            .filter(|line| line.seq > self.sent_seq)
            .map(|line| line.attributed())
            .collect();
        let seq = ctx
            .subtitles
            .iter()
            .map(|line| line.seq)
            .max()
            .unwrap_or(0)
            .max(self.sent_seq);
        // A fresh thread mid-episode inherits what was already said —
        // the words only, never the frames or dialogue behind them.
        let previous_comments = if fresh && same_episode {
            self.episode_comments.clone()
        } else {
            Vec::new()
        };
        let user_text = turn_text(&TurnInput {
            header: (fresh || !same_episode).then_some((series.as_str(), episode.as_deref())),
            previous_comments: &previous_comments,
            subtitles: &new_subtitles,
        });
        let history = if fresh {
            Vec::new()
        } else {
            self.thread
                .as_ref()
                .map(|t| t.turns.clone())
                .unwrap_or_default()
        };
        let spec = JobSpec {
            keep,
            pick_index,
            series: series.clone(),
            episode: episode.clone(),
            filename,
            cast_subtitles: ctx
                .subtitles
                .iter()
                .rev()
                .take(CHARACTER_SUBTITLES)
                .rev()
                .map(|line| line.attributed())
                .collect(),
            user_text,
            history,
            cache: self.cache_worthwhile(),
            seq,
        };
        let tx = self.results_tx.clone();
        tracing::info!(
            series = %series,
            episode = episode.as_deref().unwrap_or("?"),
            commentator = spec.keep.as_ref().map_or("(picking one)", |c| c.name.as_str()),
            new_subtitles = new_subtitles.len(),
            thread_turns = spec.history.len(),
            screenshot = screenshot.is_some(),
            cache = spec.cache,
            "commentary: requesting a comment"
        );
        tokio::task::spawn_blocking(move || {
            // The outcome must reach `finish` even when the job panics
            // (fs read, base64, the HTTP client, a future model impl):
            // a dropped, unsent sender used to latch `in_flight` for
            // the rest of the session (2026-08-12 review). A panic is
            // just another failure — a warn line and a skipped tick,
            // per the design.md failure policy.
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let screenshot = screenshot
                    .as_ref()
                    .map(|(path, requested_at)| (path.as_path(), *requested_at));
                run_job(model.as_ref(), spec, screenshot)
            }))
            .unwrap_or_else(|panic| Err(CommentaryError::Panicked(panic_message(&*panic))));
            let _ = tx.blocking_send(outcome);
        });
    }

    /// Consume one finished job (from [`Self::results`]). Returns the
    /// marquee text on success; failures are logged and skipped —
    /// leaving the thread and subtitle cursor untouched, so the next
    /// attempt covers the same ground. Always clears the in-flight
    /// guard.
    pub fn finish(&mut self, outcome: Result<JobOutcome, CommentaryError>) -> Option<String> {
        self.in_flight = false;
        match outcome {
            Ok(outcome) => {
                if !self.armed() {
                    // Disabled while the call was in flight: discard.
                    tracing::info!(
                        "commentary: disabled mid-call — discarding {}",
                        outcome.text
                    );
                    return None;
                }
                tracing::info!(
                    commentator = %outcome.commentator.name,
                    series = %outcome.commentator.series,
                    "commentary: {}", outcome.text
                );
                let key = (
                    outcome.series.clone(),
                    outcome.episode.clone(),
                    outcome.filename.clone(),
                );
                if self.episode_key.as_ref() != Some(&key) {
                    self.episode_comments.clear();
                    self.episode_key = Some(key);
                }
                if outcome.fresh
                    || self
                        .thread
                        .as_ref()
                        .is_none_or(|t| t.commentator != outcome.commentator)
                {
                    self.thread = Some(Thread {
                        commentator: outcome.commentator,
                        turns: Vec::new(),
                    });
                }
                if let Some(thread) = self.thread.as_mut() {
                    // Append, and only ever append: the turn is stored
                    // exactly as it went over the wire, because the next
                    // request replays it byte-for-byte as the cached
                    // prefix. Length is governed at plan time
                    // ([`MAX_THREAD_TURNS`]), frame weight at send time
                    // ([`MAX_THREAD_FRAME_BYTES`]) — never by rewriting
                    // what the model already saw.
                    thread.turns.push(ThreadTurn {
                        user_text: outcome.user_text,
                        screenshot: outcome.screenshot,
                        assistant: outcome.raw,
                    });
                }
                self.episode_comments.push(outcome.text.clone());
                self.sent_seq = self.sent_seq.max(outcome.seq);
                Some(outcome.text)
            }
            Err(e) => {
                tracing::warn!("commentary attempt skipped: {e}");
                None
            }
        }
    }
}

/// Best-effort text of a panic payload (`&str` / `String` from
/// `panic!`; anything else is opaque).
fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".into()
    }
}

/// The blocking job body: poll the screenshot, resolve the voice, ask
/// for the comment. Runs entirely on a blocking thread.
fn run_job(
    model: &dyn CommentaryModel,
    spec: JobSpec,
    screenshot: Option<(&Path, std::time::SystemTime)>,
) -> Result<JobOutcome, CommentaryError> {
    let screenshot_bytes = screenshot
        .and_then(|(path, requested_at)| poll_screenshot(path, requested_at))
        .filter(|frame| {
            // Append-only history means this frame, once sent, rides
            // every later request until the thread cap — so the budget
            // is on the thread's *total* frame bytes, and an
            // over-budget turn goes out frameless rather than either
            // blowing the API's request cap or rewriting sent turns to
            // make room. See [`MAX_THREAD_FRAME_BYTES`].
            let carried: u64 = spec
                .history
                .iter()
                .filter_map(|turn| turn.screenshot.as_deref())
                .map(|bytes| bytes.len() as u64)
                .sum();
            let fits = carried + frame.len() as u64 <= MAX_THREAD_FRAME_BYTES;
            if !fits {
                tracing::debug!(
                    carried_bytes = carried,
                    frame_bytes = frame.len(),
                    "thread frame budget exhausted; commenting frameless"
                );
            }
            fits
        });
    let fresh = spec.keep.is_none();
    let commentator = match spec.keep {
        Some(commentator) => commentator,
        None => {
            let names = model.list_characters(&CharacterRequest {
                series: spec.series.clone(),
                episode: spec.episode.clone(),
                recent_subtitles: spec.cast_subtitles,
            })?;
            if names.is_empty() {
                return Err(CommentaryError::NoCharacters);
            }
            let name = names[(spec.pick_index % names.len() as u64) as usize].clone();
            tracing::info!(
                commentator = %name,
                cast = names.len(),
                series = %spec.series,
                "commentary: picked a commentator"
            );
            Commentator {
                name,
                series: spec.series.clone(),
            }
        }
    };
    // The card names the commentator's *home* series, not the
    // now-playing one — a kept voice may have outlived its show, and
    // the card must stay byte-stable for the thread's lifetime anyway
    // (the caching invariant).
    let raw = model.write_comment(&CommentRequest {
        system: system_prompt(&commentator.name, &commentator.series),
        history: spec.history,
        user_text: spec.user_text.clone(),
        screenshot: screenshot_bytes.clone(),
        cache: spec.cache,
    })?;
    let text = normalize_comment(&commentator.name, &raw);
    Ok(JobOutcome {
        commentator,
        fresh,
        series: spec.series,
        episode: spec.episode,
        filename: spec.filename,
        user_text: spec.user_text,
        screenshot: screenshot_bytes,
        raw,
        text,
        seq: spec.seq,
    })
}

/// Wait for mpv to finish writing the screenshot: the file must exist,
/// be non-empty, and hold the same size across two polls. A miss is
/// `None` — the comment goes out without the frame. A frame over
/// [`MAX_SCREENSHOT_BYTES`] is likewise dropped: the API rejects it
/// outright, and losing the image beats losing the whole comment. A
/// file whose mtime predates `requested_at` is a leftover an earlier
/// tick's slow mpv finished late (the caller deletes the path before
/// each request, but a late write can still race in behind that) — it
/// is deleted and never attached.
fn poll_screenshot(path: &Path, requested_at: std::time::SystemTime) -> Option<Vec<u8>> {
    let mut last_len = None;
    for _ in 0..SCREENSHOT_POLLS {
        std::thread::sleep(SCREENSHOT_POLL_INTERVAL);
        if let Ok(meta) = std::fs::metadata(path) {
            let len = meta.len();
            if len > 0 && last_len == Some(len) {
                if meta
                    .modified()
                    .ok()
                    .is_some_and(|written| written < requested_at)
                {
                    // Written before this tick asked for it: a previous
                    // tick's late frame, minutes old by now.
                    let _ = std::fs::remove_file(path);
                    tracing::debug!("screenshot predates the request; dropped");
                    return None;
                }
                let frame = std::fs::read(path).ok();
                let _ = std::fs::remove_file(path);
                if len > MAX_SCREENSHOT_BYTES {
                    tracing::debug!(bytes = len, "screenshot too large for the API; dropped");
                    return None;
                }
                return frame;
            }
            last_len = Some(len);
        }
    }
    let _ = std::fs::remove_file(path);
    None
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// A scripted model that records every request. Comment replies are
    /// consumed front-to-back; the last one repeats.
    struct FakeModel {
        characters: Vec<String>,
        replies: Mutex<VecDeque<Result<&'static str, ()>>>,
        character_requests: Mutex<Vec<CharacterRequest>>,
        comment_requests: Mutex<Vec<CommentRequest>>,
    }

    impl FakeModel {
        fn scripted(characters: &[&str], replies: &[Result<&'static str, ()>]) -> Arc<Self> {
            Arc::new(Self {
                characters: characters.iter().map(|s| s.to_string()).collect(),
                replies: Mutex::new(replies.iter().copied().collect()),
                character_requests: Mutex::new(Vec::new()),
                comment_requests: Mutex::new(Vec::new()),
            })
        }

        fn new(characters: &[&str], comment: &'static str) -> Arc<Self> {
            Self::scripted(characters, &[Ok(comment)])
        }

        fn failing() -> Arc<Self> {
            Self::scripted(&["Amu"], &[Err(())])
        }
    }

    impl CommentaryModel for FakeModel {
        fn list_characters(&self, req: &CharacterRequest) -> Result<Vec<String>, CommentaryError> {
            self.character_requests.lock().unwrap().push(req.clone());
            if self.characters.is_empty() {
                return Err(CommentaryError::NoCharacters);
            }
            Ok(self.characters.clone())
        }

        fn write_comment(&self, req: &CommentRequest) -> Result<String, CommentaryError> {
            self.comment_requests.lock().unwrap().push(req.clone());
            let mut replies = self.replies.lock().unwrap();
            let reply = if replies.len() > 1 {
                replies.pop_front().unwrap()
            } else {
                *replies.front().expect("script has at least one reply")
            };
            reply
                .map(str::to_string)
                .map_err(|()| CommentaryError::Http("scripted failure".into()))
        }
    }

    fn gates(series: &str) -> TickGates {
        TickGates {
            connected: true,
            playing: true,
            holds_file: true,
            series: Some(series.into()),
        }
    }

    fn ring(seq: u64, speaker: Option<&str>, text: &str) -> crate::advisor::RingLine {
        crate::advisor::RingLine {
            seq,
            speaker: speaker.and_then(crate::player::SpeakerName::new),
            text: text.into(),
        }
    }

    fn ctx(series: &str) -> AdvisorContext {
        AdvisorContext {
            series_name: Some(series.into()),
            episode: Some("03".into()),
            subtitles: vec![
                ring(1, Some("Stark"), "Coming!"),
                ring(2, None, "For glory."),
            ],
            ..AdvisorContext::default()
        }
    }

    /// Await the in-flight job and run it through `finish`.
    async fn take(engine: &mut CommentaryEngine) -> Option<String> {
        let outcome = engine
            .results
            .recv()
            .await
            .expect("engine holds the sender");
        engine.finish(outcome)
    }

    fn engine(model: Arc<dyn CommentaryModel>, seed: u64) -> CommentaryEngine {
        CommentaryEngine::new(
            Some(model),
            CommentaryInterval::Every(Duration::from_secs(120)),
            StdRng::seed_from_u64(seed),
        )
    }

    /// A seed whose second `plan_tick` does not re-roll — found by
    /// trying seeds, asserted where used so a rand upgrade that shifts
    /// the stream fails loudly instead of silently testing nothing.
    const NO_REROLL_SEED: u64 = 1;

    /// The whole first-tick flow: pick a commentator from the cast,
    /// comment, remember the voice; the next tick (non-reroll seed)
    /// reuses it without a second cast call — and continues the same
    /// thread, carrying the first exchange as history.
    #[tokio::test]
    async fn first_tick_picks_then_reuses_the_commentator() {
        let fake = FakeModel::new(&["Amu", "Ikuto", "Tadase"], "Whaaaat?");
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);

        let plan = engine.plan_tick(&gates("Shugo Chara!")).expect("eligible");
        assert!(plan.pick_commentator, "no commentator yet — must pick");
        engine.spawn_job(plan, &ctx("Shugo Chara!"), None);
        let text = take(&mut engine).await.unwrap();
        let picked = engine.commentator().cloned().expect("voice retained");
        assert!(
            ["Amu", "Ikuto", "Tadase"].contains(&picked.name.as_str()),
            "picked from the cast: {picked:?}"
        );
        assert_eq!(text, format!("<{}> Whaaaat?", picked.name));

        let plan = engine.plan_tick(&gates("Shugo Chara!")).expect("eligible");
        assert!(!plan.pick_commentator, "NO_REROLL_SEED does not re-roll");
        engine.spawn_job(plan, &ctx("Shugo Chara!"), None);
        take(&mut engine).await.unwrap();
        assert_eq!(fake.character_requests.lock().unwrap().len(), 1);
        let requests = fake.comment_requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        // The second call continues the thread: one prior turn, echoed
        // verbatim (user text and raw assistant reply).
        assert!(requests[0].history.is_empty());
        assert_eq!(requests[1].history.len(), 1);
        assert_eq!(requests[1].history[0].user_text, requests[0].user_text);
        assert_eq!(requests[1].history[0].assistant, "Whaaaat?");
        assert_eq!(
            requests[0].system, requests[1].system,
            "the system prompt is stable across the thread (the caching invariant)"
        );
        drop(requests);
        assert_eq!(engine.commentator().cloned().unwrap(), picked);
    }

    /// Consecutive comments never resend dialogue: the second turn
    /// carries only lines stamped after the first turn's cursor, and a
    /// quiet stretch says so instead of repeating the ring.
    #[tokio::test]
    async fn subtitle_windows_do_not_overlap() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);

        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();

        // The ring grew by two lines (and kept the old ones).
        let mut grown = ctx("s");
        grown
            .subtitles
            .extend([ring(3, None, "Line three."), ring(4, None, "Line four.")]);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &grown, None);
        take(&mut engine).await.unwrap();

        // No new lines at all: the turn says so.
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &grown, None);
        take(&mut engine).await.unwrap();

        let requests = fake.comment_requests.lock().unwrap();
        assert!(requests[0].user_text.contains("Coming!"));
        assert!(requests[0].user_text.contains("For glory."));
        assert!(requests[1].user_text.contains("Line three."));
        assert!(requests[1].user_text.contains("Line four."));
        assert!(
            !requests[1].user_text.contains("Coming!"),
            "already-sent dialogue must not repeat: {}",
            requests[1].user_text
        );
        assert!(requests[2].user_text.contains("No new dialogue"));
    }

    /// A failed call leaves the cursor (and thread) untouched: the next
    /// attempt resends the dialogue the model never saw.
    #[tokio::test]
    async fn failed_tick_does_not_advance_the_subtitle_cursor() {
        let fake = FakeModel::scripted(&["Amu"], &[Err(()), Ok("<Amu> hi")]);
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);

        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        assert_eq!(take(&mut engine).await, None, "scripted failure");

        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();

        let requests = fake.comment_requests.lock().unwrap();
        assert!(
            requests[1].user_text.contains("Coming!"),
            "unseen dialogue is resent after a failure"
        );
        assert!(
            requests[1].history.is_empty(),
            "the failed exchange never entered the thread"
        );
    }

    /// A fresh commentator starts a fresh thread: no history, but the
    /// first turn is seeded with the episode's earlier comments (text
    /// only) and re-headers what is playing.
    #[tokio::test]
    async fn fresh_commentator_is_seeded_with_episode_comments_only() {
        let fake = FakeModel::scripted(&["Amu", "Ikuto"], &[Ok("First take!"), Ok("Second.")]);
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);

        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        let first = take(&mut engine).await.unwrap();

        // Force a re-roll (the 5% dice, taken deliberately).
        engine.spawn_job(
            TickPlan {
                pick_commentator: true,
            },
            &ctx("s"),
            None,
        );
        take(&mut engine).await.unwrap();

        assert_eq!(
            fake.character_requests.lock().unwrap().len(),
            2,
            "a fresh voice re-asks for the cast"
        );
        let requests = fake.comment_requests.lock().unwrap();
        assert!(requests[1].history.is_empty(), "fresh thread");
        assert!(
            requests[1].user_text.contains(&first),
            "seeded with the episode's comments: {}",
            requests[1].user_text
        );
        assert!(
            requests[1].user_text.contains("Now playing"),
            "a fresh thread re-headers the episode"
        );
        assert!(
            !requests[1].user_text.contains("Coming!"),
            "already-consumed dialogue is not replayed to the new voice"
        );
    }

    /// An episode change headers the next turn and resets the comment
    /// seed: a commentator picked during episode 4 hears episode 4's
    /// comments, not episode 3's.
    #[tokio::test]
    async fn episode_change_headers_the_turn_and_resets_the_seed() {
        let fake = FakeModel::scripted(
            &["Amu"],
            &[Ok("From ep three."), Ok("From ep four."), Ok("Later.")],
        );
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);

        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();

        let mut ep4 = ctx("s");
        ep4.episode = Some("04".into());
        ep4.subtitles.push(ring(3, None, "New ep line."));
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ep4, None);
        take(&mut engine).await.unwrap();

        // Fresh voice mid-episode-4: seeded with ep 4's comment only.
        engine.spawn_job(
            TickPlan {
                pick_commentator: true,
            },
            &ep4,
            None,
        );
        take(&mut engine).await.unwrap();

        let requests = fake.comment_requests.lock().unwrap();
        assert!(
            requests[1].user_text.contains("episode 04"),
            "the episode change is announced mid-thread: {}",
            requests[1].user_text
        );
        assert_eq!(
            requests[1].history.len(),
            1,
            "same voice, same thread across the episode boundary"
        );
        assert!(requests[2].user_text.contains("From ep four."));
        assert!(
            !requests[2].user_text.contains("From ep three."),
            "the seed is scoped to the current episode: {}",
            requests[2].user_text
        );
    }

    /// Regression (2026-08-12 review): AniDB-unknown files all get
    /// `episode: None` and share one hint-derived series name, so an
    /// episode key of `(series, episode)` never changed across an
    /// unlinked series' episodes — the "Now playing" header never
    /// re-fired (undermining the spoiler bound) and the comment seed
    /// grew across every episode all night. The key must include the
    /// file identity.
    #[tokio::test]
    async fn unlinked_episode_change_is_keyed_by_the_file() {
        let fake = FakeModel::scripted(
            &["Amu"],
            &[Ok("From file one."), Ok("From file two."), Ok("Later.")],
        );
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);

        let mut ep1 = ctx("s");
        ep1.episode = None;
        ep1.filename = Some("[Judas] Show - 01.mkv".into());
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ep1, None);
        take(&mut engine).await.unwrap();

        let mut ep2 = ctx("s");
        ep2.episode = None;
        ep2.filename = Some("[Judas] Show - 02.mkv".into());
        ep2.subtitles.push(ring(3, None, "New ep line."));
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ep2, None);
        take(&mut engine).await.unwrap();

        // Fresh voice mid-file-two: seeded with file two's comment only.
        engine.spawn_job(
            TickPlan {
                pick_commentator: true,
            },
            &ep2,
            None,
        );
        take(&mut engine).await.unwrap();

        let requests = fake.comment_requests.lock().unwrap();
        assert!(
            requests[1].user_text.contains("Now playing"),
            "a file change on an unlinked series re-headers the turn: {}",
            requests[1].user_text
        );
        assert!(requests[2].user_text.contains("From file two."));
        assert!(
            !requests[2].user_text.contains("From file one."),
            "the seed is scoped to the current file: {}",
            requests[2].user_text
        );
    }

    /// The thread cap: at [`MAX_THREAD_TURNS`] completed turns the
    /// next tick force-re-rolls — fresh voice, empty history, a request
    /// body shrunk back to a single turn. Append-only history (the
    /// caching invariant) means ending the thread is the only way to
    /// shed its accumulated turns, so the cap is what bounds request
    /// size where the old trim used to (and broke the cache doing it).
    #[tokio::test]
    async fn a_thread_at_the_cap_rerolls_into_a_fresh_thread() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);
        // First tick picks the voice; then drive to the cap with the
        // dice bypassed (TickPlan built directly, as the re-roll tests
        // do) so the test pins the cap, not a seed's luck.
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();
        for _ in 1..MAX_THREAD_TURNS {
            engine.spawn_job(
                TickPlan {
                    pick_commentator: false,
                },
                &ctx("s"),
                None,
            );
            take(&mut engine).await.unwrap();
        }
        let plan = engine.plan_tick(&gates("s")).unwrap();
        assert!(plan.pick_commentator, "a thread at the cap must re-roll");
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();

        let requests = fake.comment_requests.lock().unwrap();
        assert_eq!(requests.len(), MAX_THREAD_TURNS + 1);
        assert_eq!(
            requests[MAX_THREAD_TURNS - 1].history.len(),
            MAX_THREAD_TURNS - 1,
            "the last in-thread request carried the whole thread"
        );
        let capped = requests.last().unwrap();
        assert!(capped.history.is_empty(), "the capped thread is cut");
        assert_eq!(
            build_comment_body(capped)["messages"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "the request body shrinks back to a single user turn"
        );
    }

    /// The frame-byte budget: once a thread's accumulated screenshots
    /// reach [`MAX_THREAD_FRAME_BYTES`], further turns go out frameless
    /// — never by stripping frames from already-sent turns (the caching
    /// invariant), and never by letting the request outgrow the API's
    /// size cap.
    #[tokio::test]
    async fn frame_budget_sends_frameless_once_the_thread_is_heavy() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);
        // Three worst-case frames; the budget admits exactly two.
        for _ in 0..3 {
            let path = dir.path().join("frame.jpg");
            std::fs::write(&path, vec![0u8; MAX_SCREENSHOT_BYTES as usize]).unwrap();
            let plan = engine.plan_tick(&gates("s")).unwrap();
            engine.spawn_job(
                plan,
                &ctx("s"),
                Some((path, std::time::SystemTime::UNIX_EPOCH)),
            );
            take(&mut engine).await.unwrap();
        }
        let requests = fake.comment_requests.lock().unwrap();
        assert!(requests[0].screenshot.is_some());
        assert!(
            requests[1].screenshot.is_some(),
            "two worst-case frames fit"
        );
        assert_eq!(
            requests[2].screenshot, None,
            "budget spent: the turn goes out frameless"
        );
        assert!(
            requests[2]
                .history
                .iter()
                .all(|turn| turn.screenshot.is_some()),
            "sent turns keep their frames — the budget never rewrites history"
        );
    }

    /// Regression (2026-08-20 review): the old RETAINED_FRAMES trim
    /// rewrote already-sent turns (stripping their screenshots), so
    /// each tick changed the bytes of a prefix the previous request had
    /// already cached. Anthropic's prompt cache is strict-prefix — past
    /// a few turns every reachable entry diverged and the thread
    /// re-billed at full price plus the write surcharge, exactly
    /// inverting the documented cost model. Sent history must be
    /// append-only: every request's rendered message array must be a
    /// byte-equal prefix-extension of the previous request's.
    #[tokio::test]
    async fn consecutive_tick_bodies_extend_a_byte_stable_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);
        for i in 0..4 {
            let path = dir.path().join("frame.jpg");
            std::fs::write(&path, format!("frame {i}")).unwrap();
            let plan = engine.plan_tick(&gates("s")).unwrap();
            assert_eq!(
                plan.pick_commentator,
                i == 0,
                "NO_REROLL_SEED keeps one thread after the first pick"
            );
            engine.spawn_job(
                plan,
                &ctx("s"),
                Some((path, std::time::SystemTime::UNIX_EPOCH)),
            );
            take(&mut engine).await.unwrap();
        }
        let requests = fake.comment_requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        // Render as the wire would, minus the moving cache marker: the
        // API matches the prefix on content, and the breakpoint riding
        // the final block is the one legitimate per-tick difference.
        let rendered: Vec<Vec<serde_json::Value>> = requests
            .iter()
            .map(|req| {
                build_comment_body(&CommentRequest {
                    cache: false,
                    ..req.clone()
                })["messages"]
                    .as_array()
                    .unwrap()
                    .clone()
            })
            .collect();
        for (i, pair) in rendered.windows(2).enumerate() {
            let (prev, next) = (&pair[0], &pair[1]);
            assert_eq!(
                next.len(),
                prev.len() + 2,
                "tick {} appends exactly one exchange",
                i + 1
            );
            assert_eq!(
                &next[..prev.len()],
                &prev[..],
                "tick {}'s rendered messages must extend tick {}'s byte-for-byte — \
                 mutating sent history invalidates the prompt cache",
                i + 1,
                i
            );
        }
        // Append-only means every sent frame rides along verbatim.
        let images = rendered[3]
            .iter()
            .filter_map(|m| m["content"].as_array())
            .flatten()
            .filter(|block| block["type"] == "image")
            .count();
        assert_eq!(images, 4, "all four frames are still in the request");
    }

    /// The cache flag follows the interval: on when a jittered tick
    /// still lands inside the 5-minute TTL (2:00, 4:00), off past it
    /// (10:00).
    #[tokio::test]
    async fn cache_flag_follows_the_interval() {
        for (secs, expected) in [(120, true), (240, true), (600, false)] {
            let fake = FakeModel::new(&["Amu"], "<Amu> hi");
            let mut engine = CommentaryEngine::new(
                Some(fake.clone()),
                CommentaryInterval::Every(Duration::from_secs(secs)),
                StdRng::seed_from_u64(NO_REROLL_SEED),
            );
            let plan = engine.plan_tick(&gates("s")).unwrap();
            engine.spawn_job(plan, &ctx("s"), None);
            take(&mut engine).await.unwrap();
            assert_eq!(
                fake.comment_requests.lock().unwrap()[0].cache,
                expected,
                "interval {secs}s"
            );
        }
    }

    /// Both request bodies use the same deliberate thinking depth:
    /// `thinking: {type: "adaptive"}` with `output_config.effort` (the
    /// recommended shape on the pinned claude-opus-4-6 and the only
    /// accepted on-mode on newer models). The deprecated
    /// `{type: "enabled", budget_tokens}` shape must never reappear —
    /// it turns a routine model bump into a 400 on every call
    /// (2026-08-12 review).
    #[test]
    fn both_bodies_use_adaptive_thinking_at_the_shared_effort() {
        let comment = build_comment_body(&CommentRequest {
            system: "card".into(),
            history: Vec::new(),
            user_text: "turn".into(),
            screenshot: None,
            cache: false,
        });
        let character = build_character_body(&CharacterRequest {
            series: "s".into(),
            episode: None,
            recent_subtitles: Vec::new(),
        });
        for (what, body) in [("comment", &comment), ("cast", &character)] {
            assert_eq!(
                body["thinking"],
                serde_json::json!({"type": "adaptive"}),
                "{what} call uses adaptive thinking"
            );
            assert_eq!(
                body["output_config"]["effort"], EFFORT,
                "{what} call carries the shared effort"
            );
            assert!(
                !serde_json::to_string(body)
                    .unwrap()
                    .contains("budget_tokens"),
                "{what} call must not use the deprecated fixed-budget shape"
            );
        }
    }

    /// The rendered request body: history echoed as alternating
    /// user/assistant turns (images in place, never re-marked), the
    /// system prompt as a block, and — when caching — exactly one
    /// ephemeral breakpoint, on the final text block.
    #[test]
    fn comment_body_shapes_history_and_cache_breakpoint() {
        let req = CommentRequest {
            system: "card".into(),
            history: vec![ThreadTurn {
                user_text: "turn one".into(),
                screenshot: Some(b"frame".to_vec()),
                assistant: "<Amu> old".into(),
            }],
            user_text: "turn two".into(),
            screenshot: None,
            cache: true,
        };
        let body = build_comment_body(&req);
        assert_eq!(body["system"][0]["text"], "card");
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(messages[0]["content"][0]["type"], "image");
        assert_eq!(messages[0]["content"][1]["text"], "turn one");
        assert!(
            messages[0]["content"][1].get("cache_control").is_none(),
            "history blocks carry no breakpoint"
        );
        assert_eq!(messages[1]["role"], "assistant");
        assert_eq!(messages[1]["content"], "<Amu> old");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(
            messages[2]["content"][0]["cache_control"]["type"], "ephemeral",
            "the breakpoint rides the final block"
        );

        let uncached = build_comment_body(&CommentRequest {
            cache: false,
            ..req
        });
        assert!(
            !serde_json::to_string(&uncached)
                .unwrap()
                .contains("cache_control"),
            "long intervals skip the cache-write surcharge entirely"
        );
    }

    /// The jittered cadence stays within ±JITTER of the interval and
    /// actually varies to both sides — measured on the real path: every
    /// fired tick runs `plan_tick` (as the run loop does), and the
    /// *observed* time until the ticker's next fire is the assertion,
    /// so a regression in `rejitter` or in plan_tick's re-arm shows up
    /// here rather than in a copy of the formula.
    #[tokio::test(start_paused = true)]
    async fn jitter_stays_within_bounds_and_varies() {
        let mut engine = engine(FakeModel::new(&["Amu"], "<Amu> hi"), 42);
        // Gated (not playing): plan_tick re-jitters but never spawns a
        // job, so the cadence is observable without model plumbing.
        let mut gated = gates("Frieren");
        gated.playing = false;
        let interval = Duration::from_secs(120);
        // The startup fire is deliberately un-jittered; the first
        // plan_tick arms the jittered cadence under test.
        engine.ticker.tick().await;
        assert!(engine.plan_tick(&gated).is_none());
        let mut early = false;
        let mut late = false;
        for _ in 0..50 {
            let before = tokio::time::Instant::now();
            engine.ticker.tick().await;
            let elapsed = before.elapsed();
            assert!(
                elapsed >= interval - JITTER && elapsed <= interval + JITTER,
                "observed fire at {elapsed:?}, outside {interval:?} ± {JITTER:?}"
            );
            early |= elapsed < interval;
            late |= elapsed > interval;
            assert!(engine.plan_tick(&gated).is_none());
        }
        assert!(early && late, "the dice must move both ways");
    }

    /// Some seed re-rolls within a bounded number of ticks — the 5%
    /// dice is real — while the commentator persists between rolls.
    #[tokio::test]
    async fn reroll_fires_eventually_with_a_seeded_rng() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake, 7);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();

        let mut rerolled = false;
        for _ in 0..200 {
            let plan = engine.plan_tick(&gates("s")).unwrap();
            if plan.pick_commentator {
                rerolled = true;
                break;
            }
        }
        assert!(
            rerolled,
            "200 ticks at 5% never re-rolling means a dead dice"
        );
    }

    /// A series change does NOT retire the voice: the commentator (and
    /// their thread) follows the group to the next show — only the 5%
    /// dice or a restart replaces them. The turn headers the new series
    /// exactly like an episode change, and the character card stays
    /// pinned to the commentator's home series.
    #[tokio::test]
    async fn series_change_keeps_the_commentator() {
        let fake = FakeModel::scripted(&["Amu"], &[Ok("On First."), Ok("On Second.")]);
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);
        let plan = engine.plan_tick(&gates("First")).unwrap();
        engine.spawn_job(plan, &ctx("First"), None);
        take(&mut engine).await.unwrap();
        let picked = engine.commentator().cloned().expect("voice retained");

        let plan = engine.plan_tick(&gates("Second")).expect("eligible");
        assert!(
            !plan.pick_commentator,
            "a series change must not retire the voice"
        );
        let mut second = ctx("Second");
        second.subtitles.push(ring(3, None, "Fresh line."));
        engine.spawn_job(plan, &second, None);
        take(&mut engine).await.unwrap();
        assert_eq!(
            engine.commentator().cloned().unwrap(),
            picked,
            "same voice across the series boundary"
        );

        {
            let requests = fake.comment_requests.lock().unwrap();
            assert_eq!(requests.len(), 2);
            assert_eq!(
                requests[1].history.len(),
                1,
                "same thread across the series boundary"
            );
            assert!(
                requests[1].user_text.contains("Now playing: \"Second\""),
                "the series change is announced mid-thread: {}",
                requests[1].user_text
            );
            assert_eq!(
                requests[0].system, requests[1].system,
                "the character card is stable across the series change"
            );
            assert!(
                requests[1].system.contains("\"First\""),
                "the card pins the commentator to their home series: {}",
                requests[1].system
            );
        }

        // A fresh voice picked mid-Second seeds from Second's comments
        // only — the old series' comments never cross over.
        engine.spawn_job(
            TickPlan {
                pick_commentator: true,
            },
            &second,
            None,
        );
        take(&mut engine).await.unwrap();
        let requests = fake.comment_requests.lock().unwrap();
        assert!(
            requests[2].user_text.contains("On Second."),
            "seeded with the current episode's comments: {}",
            requests[2].user_text
        );
        assert!(
            !requests[2].user_text.contains("On First."),
            "no cross-series seeding: {}",
            requests[2].user_text
        );
    }

    /// Every gate suppresses the tick on its own.
    #[tokio::test]
    async fn gates_suppress_ticks_individually() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake, 1);
        let cases: Vec<(&str, TickGates)> = vec![
            (
                "disconnected",
                TickGates {
                    connected: false,
                    ..gates("s")
                },
            ),
            (
                "paused",
                TickGates {
                    playing: false,
                    ..gates("s")
                },
            ),
            (
                "missing file",
                TickGates {
                    holds_file: false,
                    ..gates("s")
                },
            ),
            (
                "unknown series",
                TickGates {
                    series: None,
                    ..gates("s")
                },
            ),
        ];
        for (label, gates) in cases {
            assert!(engine.plan_tick(&gates).is_none(), "{label} must gate");
        }
        assert!(engine.plan_tick(&gates("s")).is_some(), "all-clear ticks");

        let mut off = CommentaryEngine::disabled();
        assert!(off.plan_tick(&gates("s")).is_none(), "disabled must gate");
    }

    /// A slow call never stacks: while a job is in flight, plan_tick
    /// declines; after the result is consumed it plans again.
    #[tokio::test]
    async fn in_flight_guard_prevents_stacking() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake, 1);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        assert!(engine.plan_tick(&gates("s")).is_none(), "in flight");
        take(&mut engine).await.unwrap();
        assert!(engine.plan_tick(&gates("s")).is_some(), "cleared");
    }

    /// The prompts carry their load-bearing phrases: the spoiler bound,
    /// the subtitle context, and the output-format instruction.
    #[tokio::test]
    async fn prompts_are_spoiler_bounded_and_formatted() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);
        let plan = engine.plan_tick(&gates("Shugo Chara!")).unwrap();
        engine.spawn_job(plan, &ctx("Shugo Chara!"), None);
        take(&mut engine).await.unwrap();

        let char_req = fake.character_requests.lock().unwrap()[0].clone();
        let char_prompt = character_prompt(&char_req);
        assert!(char_prompt.contains("up to and including this episode ONLY"));
        assert!(char_prompt.contains("episode 03"));
        assert!(char_prompt.contains("For glory."));
        assert!(char_prompt.contains("one character name per line"));
        assert!(
            char_prompt.contains("Stark: Coming!"),
            "dialogue with a known ASS speaker goes out attributed: {char_prompt}"
        );

        let comment_req = fake.comment_requests.lock().unwrap()[0].clone();
        assert!(
            comment_req
                .system
                .contains("you know nothing beyond the episode currently being watched")
        );
        assert!(comment_req.system.contains("<Amu> your comment"));
        assert!(comment_req.user_text.contains("Now playing"));
        assert!(comment_req.user_text.contains("episode 03"));
        assert!(
            comment_req.user_text.contains("Stark: Coming!"),
            "the comment turn's dialogue is speaker-attributed: {}",
            comment_req.user_text
        );
        assert!(
            comment_req.user_text.contains("\nFor glory."),
            "a cue with no ASS speaker stays bare: {}",
            comment_req.user_text
        );
    }

    /// Failures skip quietly (no text) but keep the engine alive and
    /// the commentator (if any) in place.
    #[tokio::test]
    async fn failures_skip_and_the_engine_keeps_ticking() {
        let fake = FakeModel::failing();
        let mut engine = engine(fake, 1);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        assert_eq!(take(&mut engine).await, None, "failure yields no text");
        assert!(
            engine.plan_tick(&gates("s")).is_some(),
            "guard cleared; next tick proceeds"
        );

        // An empty cast is also a quiet skip.
        let empty = Arc::new(FakeModel {
            characters: Vec::new(),
            replies: Mutex::new([Ok("<x> y")].into_iter().collect()),
            character_requests: Mutex::new(Vec::new()),
            comment_requests: Mutex::new(Vec::new()),
        });
        let mut second = CommentaryEngine::new(
            Some(empty),
            CommentaryInterval::Every(Duration::from_secs(120)),
            StdRng::seed_from_u64(1),
        );
        let plan = second.plan_tick(&gates("s")).unwrap();
        second.spawn_job(plan, &ctx("s"), None);
        assert_eq!(take(&mut second).await, None);
    }

    /// A screenshot file that exists and holds still is attached — and
    /// remembered in the thread, so the next request's history carries
    /// it; no path means no attachment (and no polling delay).
    #[tokio::test]
    async fn screenshot_attaches_when_present_and_skips_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.jpg");
        std::fs::write(&path, b"jpeg bytes").unwrap();

        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), NO_REROLL_SEED);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(
            plan,
            &ctx("s"),
            Some((path.clone(), std::time::SystemTime::UNIX_EPOCH)),
        );
        take(&mut engine).await.unwrap();
        assert_eq!(
            fake.comment_requests.lock().unwrap()[0].screenshot,
            Some(b"jpeg bytes".to_vec())
        );
        assert!(!path.exists(), "consumed screenshot is cleaned up");

        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();
        let requests = fake.comment_requests.lock().unwrap();
        assert_eq!(requests[1].screenshot, None);
        assert_eq!(
            requests[1].history[0].screenshot,
            Some(b"jpeg bytes".to_vec()),
            "the thread keeps its frames"
        );
    }

    /// 2026-07-26 regression: a 10-bit source made mpv write an ~8MB
    /// 16-bit PNG whose base64 blew the API's 10MiB per-image cap
    /// (HTTP 400, every busy frame). The frame must be requested as
    /// JPEG — format follows the path extension — which keeps a 1080p
    /// frame in the hundreds of kilobytes.
    #[tokio::test]
    async fn screenshot_path_requests_a_jpeg_frame() {
        let engine = engine(FakeModel::new(&["Amu"], "hi"), 1);
        assert_eq!(
            engine
                .screenshot_path()
                .expect("private dir created")
                .extension()
                .and_then(|e| e.to_str()),
            Some("jpg"),
            "PNG frames from 10-bit video exceed the API image cap"
        );
    }

    /// The screenshot path lives in a private, engine-owned directory
    /// (0700 on Unix), never at a predictable name in the shared
    /// world-writable $TMPDIR — `poll_screenshot` reads whatever the
    /// path resolves to and ships it to the API, so a pre-planted
    /// symlink there meant local-file exfiltration (2026-08-12
    /// review). The path stays stable across ticks (mpv overwrites the
    /// same file).
    #[tokio::test]
    async fn screenshot_path_is_private_and_stable() {
        let engine = engine(FakeModel::new(&["Amu"], "hi"), 1);
        let path = engine.screenshot_path().expect("private dir created");
        let dir = path.parent().unwrap();
        assert!(dir.is_dir(), "the directory exists up front");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "only this user can reach the frame");
        }
        assert_eq!(
            engine.screenshot_path(),
            Some(path.clone()),
            "one stable path per session — mpv overwrites it"
        );
        drop(engine);
        assert!(!dir.exists(), "the engine cleans up its directory");
    }

    /// Regression (2026-08-12 review): a screenshot mpv finished
    /// writing *after* a tick's poll window used to survive on disk and
    /// get attached to the next tick, minutes later. Any file whose
    /// mtime predates the request instant is a stale leftover and must
    /// be rejected (and cleaned up).
    #[test]
    fn stale_screenshot_predating_the_request_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.jpg");
        std::fs::write(&path, b"stale frame").unwrap();
        // The request "happens" well after the file was written, so the
        // pre-planted frame is unambiguously stale.
        let requested_at = std::time::SystemTime::now() + Duration::from_secs(3600);
        assert_eq!(
            poll_screenshot(&path, requested_at),
            None,
            "a frame older than the request is never attached"
        );
        assert!(!path.exists(), "the stale frame is cleaned up");
    }

    /// Belt and braces for the same regression: a frame that is still
    /// too large is dropped (the comment goes out without it) instead
    /// of being sent to certain rejection.
    #[test]
    fn oversized_screenshot_is_dropped_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.jpg");
        std::fs::write(&path, vec![0u8; MAX_SCREENSHOT_BYTES as usize + 1]).unwrap();
        assert_eq!(
            poll_screenshot(&path, std::time::SystemTime::UNIX_EPOCH),
            None
        );
        assert!(!path.exists(), "the oversized frame is still cleaned up");
    }

    /// An Anthropic 4xx body names the offending field; the logged
    /// error must carry that message, not a bare "http status: 400"
    /// (which is what made the original failure undiagnosable).
    #[test]
    fn api_error_details_are_extracted_from_the_body() {
        let body = br#"{"type":"error","error":{"type":"invalid_request_error","message":"image exceeds 10 MB maximum"}}"#;
        assert_eq!(api_error_detail(body), "image exceeds 10 MB maximum");
        assert_eq!(api_error_detail(b"not json at all"), "not json at all");
    }

    /// Regression (2026-08-12 review): a panic inside the blocking job
    /// used to drop the result sender unsent — `in_flight` latched
    /// forever and the feature died silently for the session. The
    /// outcome must always be delivered (design.md failure policy:
    /// every failure is a log line and a skipped tick), releasing the
    /// guard so the next tick proceeds.
    #[tokio::test]
    async fn a_panicking_job_still_delivers_a_failure_and_releases_the_guard() {
        struct PanickingModel;
        impl CommentaryModel for PanickingModel {
            fn list_characters(
                &self,
                _req: &CharacterRequest,
            ) -> Result<Vec<String>, CommentaryError> {
                Ok(vec!["Amu".into()])
            }

            fn write_comment(&self, _req: &CommentRequest) -> Result<String, CommentaryError> {
                panic!("scripted panic");
            }
        }

        let mut engine = engine(Arc::new(PanickingModel), NO_REROLL_SEED);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        let outcome = tokio::time::timeout(Duration::from_secs(5), engine.results.recv())
            .await
            .expect("a panicking job must still deliver an outcome")
            .expect("engine holds the sender");
        match &outcome {
            Err(CommentaryError::Panicked(msg)) => {
                assert!(msg.contains("scripted panic"), "carries the payload: {msg}");
            }
            Err(e) => panic!("expected a Panicked outcome, got: {e}"),
            Ok(_) => panic!("a panicking job must not succeed"),
        }
        assert_eq!(engine.finish(outcome), None, "failures skip quietly");
        assert!(
            engine.plan_tick(&gates("s")).is_some(),
            "the in-flight guard is released; the next tick proceeds"
        );
    }

    /// Reconfiguring off mid-flight discards the late result; a token
    /// change drops the remembered voice.
    #[tokio::test]
    async fn disabling_mid_flight_discards_the_result() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake, 1);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        engine.reconfigure(None, CommentaryInterval::Off);
        assert_eq!(take(&mut engine).await, None, "late result discarded");
        assert!(
            engine.commentator().is_none(),
            "token cleared drops the voice"
        );
        assert!(!engine.armed());
    }

    /// The gimmick must narrate itself at info: enabled-or-why-not, the
    /// outgoing request, and — the point of the exercise — the comment
    /// that came back. Without these a silent interval is
    /// indistinguishable from a dead token.
    #[test]
    fn a_tick_is_logged_at_info_including_the_generated_comment() {
        use std::sync::Arc as StdArc;

        #[derive(Clone)]
        struct LogWriter(StdArc<Mutex<Vec<u8>>>);

        impl std::io::Write for LogWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let captured = StdArc::new(Mutex::new(Vec::new()));
        let writer = LogWriter(StdArc::clone(&captured));
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_max_level(tracing::Level::INFO)
            .with_writer(move || writer.clone())
            .finish();

        // The blocking job runs off-thread (a thread-local subscriber
        // does not reach it), so drive the flow on this thread: the
        // engine's own logs are what we assert.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(async {
                let fake = FakeModel::new(&["Amu"], "Whaaaat?");
                let mut engine = engine(fake, 1);
                async fn tick(engine: &mut CommentaryEngine) -> Option<String> {
                    engine.log_state("configured");
                    let plan = engine.plan_tick(&gates("Shugo Chara!")).unwrap();
                    engine.spawn_job(plan, &ctx("Shugo Chara!"), None);
                    take(engine).await
                }

                // tracing caches each callsite's interest the first time
                // it is hit, and a sibling test hitting these lines first
                // (on a thread with no subscriber) caches "never" for the
                // whole process. Warm the callsites, then rebuild the
                // cache under *this* thread's subscriber, so the
                // assertions below do not depend on test ordering.
                tick(&mut engine).await;
                tracing::callsite::rebuild_interest_cache();
                captured.lock().unwrap().clear();

                assert_eq!(tick(&mut engine).await.unwrap(), "<Amu> Whaaaat?");
            });
        });

        let logs = String::from_utf8(captured.lock().unwrap().clone()).unwrap();
        assert!(logs.contains("AI commentary configured: enabled"), "{logs}");
        assert!(logs.contains("interval_secs=120"), "{logs}");
        assert!(logs.contains("commentary: requesting a comment"), "{logs}");
        assert!(logs.contains("series=Shugo Chara!"), "{logs}");
        assert!(logs.contains("commentary: <Amu> Whaaaat?"), "{logs}");
    }

    #[test]
    fn normalize_repairs_prefix_flattens_and_truncates() {
        assert_eq!(normalize_comment("Amu", "Whaaaat?"), "<Amu> Whaaaat?");
        assert_eq!(
            normalize_comment("Amu", "<Amu> already\nprefixed"),
            "<Amu> already prefixed"
        );
        assert_eq!(
            normalize_comment("Amu", "<amu> case-insensitive"),
            "<amu> case-insensitive"
        );
        let long = "x".repeat(500);
        let normalized = normalize_comment("Amu", &long);
        assert_eq!(normalized.chars().count(), MAX_COMMENT_CHARS);
        assert!(normalized.ends_with('…'));
    }

    #[test]
    fn character_parsing_tolerates_bullets_and_numbering() {
        let reply = "- Amu\n2. Ikuto\n  • Tadase\n\n* Rima\n";
        assert_eq!(parse_characters(reply), ["Amu", "Ikuto", "Tadase", "Rima"]);
    }
}
