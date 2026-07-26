//! The AI commentary engine (design.md, AI Commentary). Just for fun.
//!
//! On a settings-driven interval — and only while connected, playing,
//! and holding the now-playing file — the engine asks an Anthropic
//! model to react to the episode **in character**: a persistent
//! "commentator" chosen from the show's cast (re-rolled with 5%
//! probability per tick, reset on a series change), given the series,
//! episode, recent subtitles, and an mpv screenshot when one can be
//! taken. The reply (`<Amu> Whaaaat?`) is written to the synced
//! [`marquee register`](dessplay_core::state::CrdtState::marquee), so
//! every client scrolls it — including this one, via the ordinary sync
//! echo, which keeps all replicas showing identical text.
//!
//! Nothing here blocks the bridge loop: the HTTP calls run under
//! [`tokio::task::spawn_blocking`], results come back through a
//! channel, and an in-flight guard keeps slow calls from stacking.
//! Every failure (HTTP, refusal, empty cast, malformed reply) is a
//! `tracing::warn!` and a skipped tick — never a chat line, never a
//! marquee write. The screenshot is best-effort on top of best-effort:
//! its absence is not a failure.

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
const MODEL: &str = "claude-opus-5";
/// Thinking effort for both calls — the task is short and low-stakes.
const EFFORT: &str = "low";
/// Caps thinking *plus* text on this model, so it must not be lowballed.
const MAX_TOKENS: u32 = 2048;
const API_URL: &str = "https://api.anthropic.com/v1/messages";
/// Vision calls are slow; the nyaa agent's 30s would spuriously abort.
const HTTP_TIMEOUT: Duration = Duration::from_secs(120);
/// Chance per tick of re-rolling the commentator once one exists.
const REROLL: f64 = 0.05;
/// Subtitle tail handed to the character-list call (context, not script).
const CHARACTER_SUBTITLES: usize = 20;
/// Hard cap on the marquee line, chars (the slot scrolls, but an essay
/// would take a minute to cross the screen).
const MAX_COMMENT_CHARS: usize = 220;
/// How long to wait for mpv to finish writing the screenshot.
const SCREENSHOT_POLLS: u32 = 20;
const SCREENSHOT_POLL_INTERVAL: Duration = Duration::from_millis(100);
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
}

impl std::fmt::Display for CommentaryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommentaryError::Http(e) => write!(f, "http: {e}"),
            CommentaryError::Api(e) => write!(f, "api: {e}"),
            CommentaryError::Refused => write!(f, "model refused"),
            CommentaryError::NoCharacters => write!(f, "empty character list"),
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

/// Inputs to the comment call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentRequest {
    /// The in-character voice.
    pub commentator: String,
    /// Series title.
    pub series: String,
    /// Episode label, when metadata knows it.
    pub episode: Option<String>,
    /// The last ≤100 deduped subtitle lines, oldest first.
    pub subtitles: Vec<String>,
    /// The current video frame (JPEG bytes), when mpv delivered one in
    /// time.
    pub screenshot: Option<Vec<u8>>,
}

/// The model seam. Blocking — implementations are always called under
/// `spawn_blocking`; tests inject a scripted fake.
pub trait CommentaryModel: Send + Sync {
    /// Major characters through the current episode, spoiler-bounded.
    fn list_characters(&self, req: &CharacterRequest) -> Result<Vec<String>, CommentaryError>;
    /// A 1–3 sentence in-character reaction.
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

/// The comment prompt: in character, spoiler-bounded, IRC-style output.
fn comment_prompt(req: &CommentRequest) -> String {
    let mut prompt = format!(
        "You are {name}, a character from \"{series}\". You are watching \
         {episode} of your own show together with friends at a watch \
         party. Hard rule: you know nothing beyond this episode — no \
         future events, no meta-knowledge, no winking at the audience.\n\n\
         The most recent subtitle lines, oldest first:\n",
        name = req.commentator,
        series = req.series,
        episode = episode_label(req.episode.as_deref()),
    );
    for line in &req.subtitles {
        prompt.push_str(line);
        prompt.push('\n');
    }
    if req.screenshot.is_some() {
        prompt.push_str("\nThe attached image is the current video frame.\n");
    }
    prompt.push_str(&format!(
        "\nReact in character to what is happening right now: 1-3 short \
         sentences, IRC style. Output exactly one line in the form \
         `<{name}> your comment` and nothing else.",
        name = req.commentator,
    ));
    prompt
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

    /// One Messages call; returns the first text block.
    fn call(&self, content: serde_json::Value) -> Result<String, CommentaryError> {
        let body = serde_json::json!({
            "model": MODEL,
            "max_tokens": MAX_TOKENS,
            "output_config": { "effort": EFFORT },
            "messages": [{ "role": "user", "content": content }],
        });
        let bytes = serde_json::to_vec(&body)
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
        let reply = self.call(serde_json::Value::String(character_prompt(req)))?;
        let names = parse_characters(&reply);
        if names.is_empty() {
            return Err(CommentaryError::NoCharacters);
        }
        Ok(names)
    }

    fn write_comment(&self, req: &CommentRequest) -> Result<String, CommentaryError> {
        let mut content = Vec::new();
        if let Some(frame) = &req.screenshot {
            content.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": "image/jpeg",
                    "data": base64::engine::general_purpose::STANDARD.encode(frame),
                },
            }));
        }
        content.push(serde_json::json!({
            "type": "text",
            "text": comment_prompt(req),
        }));
        self.call(serde_json::Value::Array(content))
    }
}

// ---- The engine ----------------------------------------------------------

/// The persistent voice: kept across ticks (and API failures), reset
/// when the series changes, re-rolled with [`REROLL`] probability.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Commentator {
    name: String,
    series: String,
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
    /// Ask for the cast and pick a fresh commentator first.
    pub pick_commentator: bool,
}

/// A finished job's payload. Opaque outside this module — the run loop
/// receives it from [`CommentaryEngine::results`] and hands it straight
/// to [`CommentaryEngine::finish`].
pub struct JobOutcome {
    /// The voice that spoke (persisted for the next tick).
    commentator: Commentator,
    /// The normalized marquee line.
    text: String,
}

/// Owns the cadence, the commentator, the RNG, and the in-flight job.
/// Lives on the [`crate::run::SessionLoop`]; the loop select-arms
/// [`Self::ticker`] and [`Self::results`].
pub struct CommentaryEngine {
    model: Option<Arc<dyn CommentaryModel>>,
    interval: Option<Duration>,
    /// Ticks at the configured interval; only armed when enabled.
    pub ticker: tokio::time::Interval,
    commentator: Option<Commentator>,
    rng: StdRng,
    in_flight: bool,
    results_tx: mpsc::Sender<Result<JobOutcome, CommentaryError>>,
    /// Finished jobs; the run loop select-arms this (a separate field
    /// from [`Self::ticker`] so the two arms borrow disjointly) and
    /// feeds each into [`Self::finish`].
    pub results: mpsc::Receiver<Result<JobOutcome, CommentaryError>>,
    /// Where the player writes screenshots (one path, overwritten).
    screenshot_path: PathBuf,
}

impl CommentaryEngine {
    /// An engine from the saved settings, seeded from entropy.
    pub fn from_settings(settings: &crate::config::Settings) -> Self {
        Self::new(
            settings
                .anthropic_token
                .clone()
                .map(|token| Arc::new(AnthropicModel::new(token)) as Arc<dyn CommentaryModel>),
            settings.commentary_interval,
            StdRng::from_os_rng(),
        )
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
            commentator: None,
            rng,
            in_flight: false,
            results_tx,
            results,
            // .jpg drives mpv's format inference: a PNG of a 10-bit
            // source is 16-bit and ~8MB — past the API's image cap —
            // where a JPEG frame is a few hundred KB.
            screenshot_path: std::env::temp_dir()
                .join(format!("dessplay-commentary-{}.jpg", std::process::id())),
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
    /// commentator too (a fresh token starts fresh); an in-flight call
    /// finishes and its result is discarded if the engine is off by then.
    pub fn reconfigure(&mut self, token: Option<&str>, interval: CommentaryInterval) {
        self.model = token.map(|token| {
            Arc::new(AnthropicModel::new(token.to_string())) as Arc<dyn CommentaryModel>
        });
        if self.model.is_none() {
            self.commentator = None;
        }
        self.interval = interval.duration();
        self.ticker = Self::make_ticker(self.interval);
    }

    /// Where the player should write the screenshot for the next job.
    pub fn screenshot_path(&self) -> PathBuf {
        self.screenshot_path.clone()
    }

    /// Decide what (if anything) this tick does. `None` = skip quietly.
    pub fn plan_tick(&mut self, gates: &TickGates) -> Option<TickPlan> {
        if !self.armed() || self.in_flight {
            return None;
        }
        if !gates.connected || !gates.playing || !gates.holds_file {
            return None;
        }
        let series = gates.series.as_deref()?;
        // A series change retires the old voice before the roll below.
        if self
            .commentator
            .as_ref()
            .is_some_and(|c| c.series != series)
        {
            self.commentator = None;
        }
        let pick_commentator = self.commentator.is_none() || self.rng.random_bool(REROLL);
        Some(TickPlan { pick_commentator })
    }

    /// Launch the blocking job for a planned tick. `screenshot` is the
    /// path the player was asked to write, or `None` when no player is
    /// running (skip the poll entirely).
    pub fn spawn_job(&mut self, plan: TickPlan, ctx: &AdvisorContext, screenshot: Option<PathBuf>) {
        let Some(model) = self.model.clone() else {
            return;
        };
        let Some(series) = ctx.series_name.clone() else {
            return;
        };
        self.in_flight = true;
        let keep = (!plan.pick_commentator)
            .then(|| self.commentator.clone())
            .flatten();
        // Drawn on the loop thread so the RNG (and its seed-determinism
        // in tests) never crosses into the blocking task.
        let pick_index: u64 = self.rng.random();
        let episode = ctx.episode.clone();
        let subtitles = ctx.subtitles.clone();
        let tx = self.results_tx.clone();
        tokio::task::spawn_blocking(move || {
            let outcome = run_job(
                model.as_ref(),
                keep,
                pick_index,
                series,
                episode,
                subtitles,
                screenshot.as_deref(),
            );
            let _ = tx.blocking_send(outcome);
        });
    }

    /// Consume one finished job (from [`Self::results`]). Returns the
    /// marquee text on success; failures are logged and skipped. Always
    /// clears the in-flight guard.
    pub fn finish(&mut self, outcome: Result<JobOutcome, CommentaryError>) -> Option<String> {
        self.in_flight = false;
        match outcome {
            Ok(JobOutcome { commentator, text }) => {
                if !self.armed() {
                    // Disabled while the call was in flight: discard.
                    return None;
                }
                tracing::info!(
                    commentator = %commentator.name,
                    "commentary: {text}"
                );
                self.commentator = Some(commentator);
                Some(text)
            }
            Err(e) => {
                tracing::warn!("commentary attempt skipped: {e}");
                None
            }
        }
    }
}

/// The blocking job body: poll the screenshot, resolve the voice, ask
/// for the comment. Runs entirely on a blocking thread.
fn run_job(
    model: &dyn CommentaryModel,
    keep: Option<Commentator>,
    pick_index: u64,
    series: String,
    episode: Option<String>,
    subtitles: Vec<String>,
    screenshot: Option<&Path>,
) -> Result<JobOutcome, CommentaryError> {
    let screenshot_bytes = screenshot.and_then(poll_screenshot);
    let commentator = match keep {
        Some(commentator) => commentator,
        None => {
            let names = model.list_characters(&CharacterRequest {
                series: series.clone(),
                episode: episode.clone(),
                recent_subtitles: subtitles
                    .iter()
                    .rev()
                    .take(CHARACTER_SUBTITLES)
                    .rev()
                    .cloned()
                    .collect(),
            })?;
            if names.is_empty() {
                return Err(CommentaryError::NoCharacters);
            }
            let name = names[(pick_index % names.len() as u64) as usize].clone();
            Commentator {
                name,
                series: series.clone(),
            }
        }
    };
    let raw = model.write_comment(&CommentRequest {
        commentator: commentator.name.clone(),
        series,
        episode,
        subtitles,
        screenshot: screenshot_bytes,
    })?;
    let text = normalize_comment(&commentator.name, &raw);
    Ok(JobOutcome { commentator, text })
}

/// Wait for mpv to finish writing the screenshot: the file must exist,
/// be non-empty, and hold the same size across two polls. A miss is
/// `None` — the comment goes out without the frame. A frame over
/// [`MAX_SCREENSHOT_BYTES`] is likewise dropped: the API rejects it
/// outright, and losing the image beats losing the whole comment.
fn poll_screenshot(path: &Path) -> Option<Vec<u8>> {
    let mut last_len = None;
    for _ in 0..SCREENSHOT_POLLS {
        std::thread::sleep(SCREENSHOT_POLL_INTERVAL);
        if let Ok(meta) = std::fs::metadata(path) {
            let len = meta.len();
            if len > 0 && last_len == Some(len) {
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

    use std::sync::Mutex;

    use super::*;

    /// A scripted model that records every request.
    struct FakeModel {
        characters: Vec<String>,
        comment: Result<&'static str, ()>,
        character_requests: Mutex<Vec<CharacterRequest>>,
        comment_requests: Mutex<Vec<CommentRequest>>,
    }

    impl FakeModel {
        fn new(characters: &[&str], comment: &'static str) -> Arc<Self> {
            Arc::new(Self {
                characters: characters.iter().map(|s| s.to_string()).collect(),
                comment: Ok(comment),
                character_requests: Mutex::new(Vec::new()),
                comment_requests: Mutex::new(Vec::new()),
            })
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self {
                characters: vec!["Amu".into()],
                comment: Err(()),
                character_requests: Mutex::new(Vec::new()),
                comment_requests: Mutex::new(Vec::new()),
            })
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
            self.comment
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

    fn ctx(series: &str) -> AdvisorContext {
        AdvisorContext {
            series_name: Some(series.into()),
            episode: Some("03".into()),
            subtitles: vec!["Coming!".into(), "For glory.".into()],
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

    /// The whole first-tick flow: pick a commentator from the cast,
    /// comment, remember the voice; the next tick (non-reroll seed)
    /// reuses it without a second cast call.
    #[tokio::test]
    async fn first_tick_picks_then_reuses_the_commentator() {
        let fake = FakeModel::new(&["Amu", "Ikuto", "Tadase"], "Whaaaat?");
        // Seed 1: the post-pick rolls stay under 95%, so no re-roll.
        let mut engine = engine(fake.clone(), 1);

        let plan = engine.plan_tick(&gates("Shugo Chara!")).expect("eligible");
        assert!(plan.pick_commentator, "no commentator yet — must pick");
        engine.spawn_job(plan, &ctx("Shugo Chara!"), None);
        let text = take(&mut engine).await.unwrap();
        let picked = engine.commentator.clone().expect("voice retained");
        assert!(
            ["Amu", "Ikuto", "Tadase"].contains(&picked.name.as_str()),
            "picked from the cast: {picked:?}"
        );
        assert_eq!(text, format!("<{}> Whaaaat?", picked.name));

        let plan = engine.plan_tick(&gates("Shugo Chara!")).expect("eligible");
        assert!(!plan.pick_commentator, "seed 1 does not re-roll");
        engine.spawn_job(plan, &ctx("Shugo Chara!"), None);
        take(&mut engine).await.unwrap();
        assert_eq!(fake.character_requests.lock().unwrap().len(), 1);
        assert_eq!(fake.comment_requests.lock().unwrap().len(), 2);
        assert_eq!(engine.commentator.clone().unwrap(), picked);
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

    /// A series change retires the voice: the next tick picks fresh.
    #[tokio::test]
    async fn series_change_resets_the_commentator() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake, 1);
        let plan = engine.plan_tick(&gates("First")).unwrap();
        engine.spawn_job(plan, &ctx("First"), None);
        take(&mut engine).await.unwrap();
        assert!(engine.commentator.is_some());

        let plan = engine.plan_tick(&gates("Second")).expect("eligible");
        assert!(plan.pick_commentator, "new series — new voice");
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
    /// the subtitle tail, and the output-format instruction.
    #[tokio::test]
    async fn prompts_are_spoiler_bounded_and_formatted() {
        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), 1);
        let plan = engine.plan_tick(&gates("Shugo Chara!")).unwrap();
        engine.spawn_job(plan, &ctx("Shugo Chara!"), None);
        take(&mut engine).await.unwrap();

        let char_req = fake.character_requests.lock().unwrap()[0].clone();
        let char_prompt = character_prompt(&char_req);
        assert!(char_prompt.contains("up to and including this episode ONLY"));
        assert!(char_prompt.contains("episode 03"));
        assert!(char_prompt.contains("For glory."));
        assert!(char_prompt.contains("one character name per line"));

        let comment_req = fake.comment_requests.lock().unwrap()[0].clone();
        let prompt = comment_prompt(&comment_req);
        assert!(prompt.contains("you know nothing beyond this episode"));
        assert!(prompt.contains("Coming!"));
        assert!(prompt.contains(&format!("<{}> your comment", comment_req.commentator)));
        assert!(
            !prompt.contains("attached image"),
            "no screenshot — no image sentence"
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
            comment: Ok("<x> y"),
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

    /// A screenshot file that exists and holds still is attached; no
    /// path means no attachment (and no polling delay).
    #[tokio::test]
    async fn screenshot_attaches_when_present_and_skips_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.jpg");
        std::fs::write(&path, b"jpeg bytes").unwrap();

        let fake = FakeModel::new(&["Amu"], "<Amu> hi");
        let mut engine = engine(fake.clone(), 1);
        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), Some(path.clone()));
        take(&mut engine).await.unwrap();
        assert_eq!(
            fake.comment_requests.lock().unwrap()[0].screenshot,
            Some(b"jpeg bytes".to_vec())
        );
        assert!(!path.exists(), "consumed screenshot is cleaned up");

        let plan = engine.plan_tick(&gates("s")).unwrap();
        engine.spawn_job(plan, &ctx("s"), None);
        take(&mut engine).await.unwrap();
        assert_eq!(fake.comment_requests.lock().unwrap()[1].screenshot, None);
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
                .extension()
                .and_then(|e| e.to_str()),
            Some("jpg"),
            "PNG frames from 10-bit video exceed the API image cap"
        );
    }

    /// Belt and braces for the same regression: a frame that is still
    /// too large is dropped (the comment goes out without it) instead
    /// of being sent to certain rejection.
    #[test]
    fn oversized_screenshot_is_dropped_not_sent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.jpg");
        std::fs::write(&path, vec![0u8; MAX_SCREENSHOT_BYTES as usize + 1]).unwrap();
        assert_eq!(poll_screenshot(&path), None);
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
            engine.commentator.is_none(),
            "token cleared drops the voice"
        );
        assert!(!engine.armed());
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
