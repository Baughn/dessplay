//! The advisor: the session-layer seam behind the health row's
//! suggestion slot (design.md, Connection Health Line).
//!
//! Providers are handed an [`AdvisorContext`] — link/sync health plus
//! what the group is watching (series, episode, the last ~50 deduped
//! subtitle lines) — and deliver [`SuggestionUpdate`]s through a
//! channel. The rule-based provider ships today ("high latency —
//! disable BitTorrent"); the context/channel shape is deliberately
//! everything a future LLM commentary provider needs (an async task
//! owning a `Sender` clone), so that lands as a second provider with no
//! new plumbing.
//!
//! Nothing here blocks: `advise` is called from the bridge loop (the
//! liveness rule), so providers either compute synchronously and
//! `try_send`, or spawn.

use std::collections::VecDeque;
use std::time::Duration;

use dessplay_core::StateView;
use tokio::sync::mpsc;

use crate::player::SpeakerName;
use crate::session::SubtitleLine;
use crate::ui::props::{self, HealthLevel, HealthSample, LinkStatus};

/// How urgent a suggestion is; the UI maps this to a display tone
/// (dim / yellow / red).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Severity {
    /// Background commentary or FYI.
    Info,
    /// Something the user probably wants to act on.
    Warning,
    /// Actively broken; act now.
    Critical,
}

/// One suggestion for the health row's right-aligned slot.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Suggestion {
    /// Stable identity for change-detection (providers re-emit only
    /// when the winning suggestion changes).
    pub id: &'static str,
    /// The display text.
    pub text: String,
    /// Display urgency.
    pub severity: Severity,
}

/// A provider's output: `Some` replaces the slot, `None` clears it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SuggestionUpdate(pub Option<Suggestion>);

/// One deduped line in the subtitle context ring. The speaker (the ASS
/// Name field, when the cue carried one) is kept separately from the
/// text so the reveal/overlap collapse still classifies against mpv's
/// raw re-emissions; consumers that want attribution render it via
/// [`RingLine::attributed`].
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RingLine {
    /// Monotonic sequence number of the line's last mutation (push or
    /// in-place growth) — the commentary engine's non-overlap cursor
    /// keys on it, so consecutive comments never resend dialogue the
    /// model has already seen.
    pub seq: u64,
    /// The ASS speaker/actor, if the cue carried one. Tracks the latest
    /// speaker across in-place growth, like the UI's subtitle log.
    pub speaker: Option<SpeakerName>,
    /// The collapsed subtitle text (newlines already flattened).
    pub text: String,
}

impl RingLine {
    /// The line as prompt text: `Name: text` when the cue named a
    /// speaker — the commentary model can't see the video, so dialogue
    /// goes out properly attributed (same `Name: ` form as the subtitle
    /// pane's speaker-name prefix).
    pub fn attributed(&self) -> String {
        match &self.speaker {
            Some(name) => format!("{name}: {}", self.text),
            None => self.text.clone(),
        }
    }
}

/// Everything a provider may reason over. Built by the session loop
/// from its own state — no storage or network access needed.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct AdvisorContext {
    /// Now-playing series name, when metadata knows it.
    pub series_name: Option<String>,
    /// Now-playing episode label, when metadata knows it.
    pub episode: Option<String>,
    /// Now-playing filename.
    pub filename: Option<String>,
    /// The last ≤100 subtitle lines, deduped with the same
    /// reveal/overlap collapse as the UI's subtitle log, oldest first.
    /// Each carries its sequence stamp and the ASS speaker when known —
    /// see [`RingLine`].
    pub subtitles: Vec<RingLine>,
    /// Server-link state.
    pub link: LinkStatus,
    /// Displayed (hysteresis-filtered) health level.
    pub level: HealthLevel,
    /// The latest merged health sample.
    pub sample: Option<HealthSample>,
    /// The divergence alarm fired and has not yet been followed by a
    /// healthy sample.
    pub diverged: bool,
    /// The BitTorrent setting (as currently saved).
    pub torrent_enabled: bool,
    /// The torrent engine is actually moving or holding torrents.
    pub torrent_active: bool,
}

/// A source of suggestions. Must not block — called from the bridge
/// loop. Synchronous providers `try_send` on `out`; an async provider
/// (the future commentary path) clones `out` into a spawned task and
/// delivers whenever it finishes.
pub trait AdvisorProvider: Send {
    /// The provider's name, for logs.
    fn name(&self) -> &'static str;
    /// Consider the context; deliver any change through `out`.
    fn advise(&mut self, ctx: &AdvisorContext, out: &mpsc::Sender<SuggestionUpdate>);
}

/// Minimum spacing between advise passes; health samples arrive at 1Hz
/// and the rules don't need that resolution.
const ADVISE_INTERVAL: Duration = Duration::from_secs(5);

/// Subtitle-context ring size (design.md: "the last ~100 deduped
/// subtitle lines").
const SUBTITLE_RING: usize = 100;

/// Owns the providers, the suggestion channel, and the subtitle ring.
/// Lives on the [`crate::run::SessionLoop`]; the loop select-arms the
/// receiver and copies delivered updates into the UI snapshot.
pub struct Advisor {
    providers: Vec<Box<dyn AdvisorProvider>>,
    tx: mpsc::Sender<SuggestionUpdate>,
    /// Drained by the session loop's select arm.
    pub suggestions: mpsc::Receiver<SuggestionUpdate>,
    subtitles: VecDeque<RingLine>,
    /// Monotonic counter behind the ring's per-line sequence numbers;
    /// bumped on every push *and* every in-place growth, so a line that
    /// grew after being consumed reads as new again.
    subtitle_seq: u64,
    last_advise: Option<tokio::time::Instant>,
    diverged: bool,
}

impl Default for Advisor {
    /// No providers — the seam idles (tests, seeders).
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl Advisor {
    /// An advisor with the given providers.
    pub fn new(providers: Vec<Box<dyn AdvisorProvider>>) -> Self {
        let (tx, suggestions) = mpsc::channel(16);
        Self {
            providers,
            tx,
            suggestions,
            subtitles: VecDeque::new(),
            subtitle_seq: 0,
            last_advise: None,
            diverged: false,
        }
    }

    /// The production provider set.
    pub fn with_rules() -> Self {
        Self::new(vec![Box::new(RuleProvider::default())])
    }

    /// Feed subtitle lines into the context ring (called wherever the
    /// session forwards them to the UI), applying the shared
    /// reveal/overlap collapse so the ring holds lines, not mpv's
    /// re-emissions.
    pub fn observe_subtitles(&mut self, lines: &[SubtitleLine]) {
        for line in lines {
            let text = line.text.replace('\r', "").replace('\n', " ");
            if text.is_empty() {
                continue;
            }
            match props::subtitle_collapse(
                self.subtitles.back().map(|line| line.text.as_str()),
                &text,
            ) {
                props::SubtitleCollapse::Extends => {
                    if let Some(last) = self.subtitles.back_mut() {
                        // A grown line is new material: re-stamp it so a
                        // cursor that consumed the shorter form picks up
                        // the full one. Track the latest speaker, like
                        // the UI's subtitle log.
                        self.subtitle_seq += 1;
                        *last = RingLine {
                            seq: self.subtitle_seq,
                            speaker: line.speaker.clone(),
                            text,
                        };
                    }
                }
                props::SubtitleCollapse::Contained => {}
                props::SubtitleCollapse::Distinct => {
                    self.subtitle_seq += 1;
                    self.subtitles.push_back(RingLine {
                        seq: self.subtitle_seq,
                        speaker: line.speaker.clone(),
                        text,
                    });
                    while self.subtitles.len() > SUBTITLE_RING {
                        self.subtitles.pop_front();
                    }
                }
            }
        }
    }

    /// The divergence alarm fired; the flag rides the next context and
    /// clears once a healthy sample arrives.
    pub fn on_diverged(&mut self) {
        self.diverged = true;
        // Rare and newsworthy: bypass the throttle so the next health
        // sample advises immediately.
        self.last_advise = None;
    }

    /// A fresh health sample (called at most 1Hz): build the full
    /// context and let every provider consider it, throttled to
    /// [`ADVISE_INTERVAL`].
    #[allow(clippy::too_many_arguments)]
    pub fn on_health(
        &mut self,
        view: &StateView,
        link: LinkStatus,
        level: HealthLevel,
        sample: Option<HealthSample>,
        torrent_enabled: bool,
        torrent_active: bool,
    ) {
        let now = tokio::time::Instant::now();
        if self
            .last_advise
            .is_some_and(|last| now.duration_since(last) < ADVISE_INTERVAL)
        {
            return;
        }
        self.last_advise = Some(now);
        if self.diverged && level == HealthLevel::Ok {
            self.diverged = false;
        }
        let ctx = self.context(view, link, level, sample, torrent_enabled, torrent_active);
        for provider in &mut self.providers {
            provider.advise(&ctx, &self.tx);
        }
    }

    /// Assemble the context — the same now-playing lookups the status
    /// bar's props use. `pub(crate)` so the commentary engine's tick can
    /// reuse the assembled series/episode/subtitle context verbatim.
    pub(crate) fn context(
        &self,
        view: &StateView,
        link: LinkStatus,
        level: HealthLevel,
        sample: Option<HealthSample>,
        torrent_enabled: bool,
        torrent_active: bool,
    ) -> AdvisorContext {
        let entry = view
            .now_playing
            .and_then(|hash| view.playlist.iter().find(|entry| entry.hash == hash));
        let metadata = view
            .now_playing
            .and_then(|hash| view.anidb_metadata.get(&hash))
            .and_then(|m| m.as_ref());
        AdvisorContext {
            series_name: metadata.map(|m| m.series_name.clone()),
            episode: metadata.and_then(|m| m.episode_number.clone()),
            filename: entry.map(|entry| entry.state.filename.clone()),
            subtitles: self.subtitles.iter().cloned().collect(),
            link,
            level,
            sample,
            diverged: self.diverged,
            torrent_enabled,
            torrent_active,
        }
    }
}

/// How long a rule must stay quiet before its suggestion is cleared —
/// a single calm advise pass must not flicker the slot away.
const CLEAR_HOLD: Duration = Duration::from_secs(30);

/// The rule-based provider: deterministic link-health advice.
#[derive(Default)]
pub struct RuleProvider {
    /// The currently displayed suggestion (id + text, for change
    /// detection).
    active: Option<(&'static str, String)>,
    /// When the rules first went quiet while a suggestion was showing.
    quiet_since: Option<tokio::time::Instant>,
}

impl RuleProvider {
    /// The winning rule for this context, most actionable first.
    fn pick(ctx: &AdvisorContext) -> Option<Suggestion> {
        if ctx.level >= HealthLevel::Degraded && ctx.torrent_enabled && ctx.torrent_active {
            // The Starlink scenario: BitTorrent saturating the uplink
            // degrades (or kills) sync while QUIC stays up. The setting
            // applies immediately (design.md, BitTorrent Downloads).
            return Some(Suggestion {
                id: "torrent-uplink",
                text: "high latency — disable BitTorrent (F3, applies immediately)".into(),
                severity: Severity::Warning,
            });
        }
        if ctx.level == HealthLevel::Stalled {
            let silence = ctx
                .sample
                .map(|s| s.server_silence_millis / 1000)
                .unwrap_or(0);
            return Some(Suggestion {
                id: "sync-stalled",
                text: format!("sync stalled — server silent {silence}s"),
                severity: Severity::Warning,
            });
        }
        if ctx.diverged {
            return Some(Suggestion {
                id: "diverged",
                text: "state diverged — resyncing".into(),
                severity: Severity::Info,
            });
        }
        None
    }
}

impl AdvisorProvider for RuleProvider {
    fn name(&self) -> &'static str {
        "rules"
    }

    fn advise(&mut self, ctx: &AdvisorContext, out: &mpsc::Sender<SuggestionUpdate>) {
        match Self::pick(ctx) {
            Some(suggestion) => {
                self.quiet_since = None;
                let key = (suggestion.id, suggestion.text.clone());
                if self.active.as_ref() != Some(&key) {
                    self.active = Some(key);
                    let _ = out.try_send(SuggestionUpdate(Some(suggestion)));
                }
            }
            None if self.active.is_some() => {
                // Hold the cleared slot for a while: rules flickering
                // at a threshold edge must not strobe the suggestion.
                let now = tokio::time::Instant::now();
                let since = *self.quiet_since.get_or_insert(now);
                if now.duration_since(since) >= CLEAR_HOLD {
                    self.active = None;
                    self.quiet_since = None;
                    let _ = out.try_send(SuggestionUpdate(None));
                }
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn ctx(level: HealthLevel) -> AdvisorContext {
        AdvisorContext {
            link: LinkStatus::Connected,
            level,
            sample: Some(HealthSample {
                rtt_millis: Some(2_000),
                unanswered_probes: 0,
                server_silence_millis: 80_000,
                up_bps: 500_000,
                down_bps: 0,
            }),
            torrent_enabled: true,
            torrent_active: true,
            ..AdvisorContext::default()
        }
    }

    fn drain(rx: &mut mpsc::Receiver<SuggestionUpdate>) -> Vec<SuggestionUpdate> {
        let mut updates = Vec::new();
        while let Ok(update) = rx.try_recv() {
            updates.push(update);
        }
        updates
    }

    /// Rule 1 (the Starlink scenario) wins over rule 2, emits once, and
    /// does not re-emit on an unchanged context.
    #[tokio::test(start_paused = true)]
    async fn torrent_rule_fires_once_per_change() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut rules = RuleProvider::default();
        rules.advise(&ctx(HealthLevel::Degraded), &tx);
        rules.advise(&ctx(HealthLevel::Degraded), &tx);
        let updates = drain(&mut rx);
        assert_eq!(updates.len(), 1, "no re-emission on unchanged context");
        let suggestion = updates[0].0.clone().unwrap();
        assert_eq!(suggestion.id, "torrent-uplink");
        assert_eq!(suggestion.severity, Severity::Warning);
    }

    /// Without an active torrent the stalled rule fires instead, and its
    /// text updates as the silence grows.
    #[tokio::test(start_paused = true)]
    async fn stalled_rule_fires_and_updates_text() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut rules = RuleProvider::default();
        let mut stalled = ctx(HealthLevel::Stalled);
        stalled.torrent_active = false;
        rules.advise(&stalled, &tx);
        if let Some(sample) = &mut stalled.sample {
            sample.server_silence_millis = 120_000;
        }
        rules.advise(&stalled, &tx);
        let updates = drain(&mut rx);
        let texts: Vec<String> = updates.into_iter().map(|u| u.0.unwrap().text).collect();
        assert_eq!(
            texts,
            [
                "sync stalled — server silent 80s",
                "sync stalled — server silent 120s"
            ]
        );
    }

    /// A cleared condition holds the suggestion for CLEAR_HOLD before
    /// clearing the slot — no strobing at a threshold edge.
    #[tokio::test(start_paused = true)]
    async fn clearing_is_held_for_thirty_seconds() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut rules = RuleProvider::default();
        rules.advise(&ctx(HealthLevel::Degraded), &tx);
        assert_eq!(drain(&mut rx).len(), 1);

        let calm = AdvisorContext {
            level: HealthLevel::Ok,
            torrent_active: false,
            ..ctx(HealthLevel::Ok)
        };
        rules.advise(&calm, &tx);
        assert!(drain(&mut rx).is_empty(), "held, not cleared yet");
        tokio::time::advance(Duration::from_secs(31)).await;
        rules.advise(&calm, &tx);
        assert_eq!(drain(&mut rx), vec![SuggestionUpdate(None)]);

        // A re-fire during the hold cancels the pending clear.
        rules.advise(&ctx(HealthLevel::Degraded), &tx);
        assert_eq!(drain(&mut rx).len(), 1);
        rules.advise(&calm, &tx);
        tokio::time::advance(Duration::from_secs(20)).await;
        rules.advise(&ctx(HealthLevel::Degraded), &tx);
        tokio::time::advance(Duration::from_secs(20)).await;
        rules.advise(&ctx(HealthLevel::Degraded), &tx);
        assert!(
            drain(&mut rx).is_empty(),
            "same suggestion still active; no clear, no re-emit"
        );
    }

    /// The advisor throttles advise passes, dedupes the subtitle ring
    /// with the shared collapse, and caps it at [`SUBTITLE_RING`] lines.
    #[tokio::test(start_paused = true)]
    async fn advisor_ring_dedupes_and_caps() {
        let mut advisor = Advisor::default();
        let line = |text: &str| SubtitleLine {
            text: text.into(),
            speaker: None,
            video_millis: 0,
            arrival_millis: 0,
        };
        // A reveal grows in place; the shrink-back is dropped.
        advisor.observe_subtitles(&[
            line("Com"),
            line("Coming!"),
            line("Coming! For glory."),
            line("For glory."),
        ]);
        assert_eq!(advisor.subtitles.len(), 1);
        assert_eq!(advisor.subtitles[0].text, "Coming! For glory.");
        // Multi-line cues arrive newline-separated; newlines become
        // spaces (the "you demons" rule).
        advisor.observe_subtitles(&[line("you\ndemons")]);
        assert_eq!(advisor.subtitles.back().unwrap().text, "you demons");
        for i in 0..120 {
            advisor.observe_subtitles(&[line(&format!("distinct line {i}"))]);
        }
        assert_eq!(advisor.subtitles.len(), SUBTITLE_RING);
    }

    /// Every line carries a monotonic sequence number, and an in-place
    /// growth re-stamps its line: a consumer that has already read the
    /// short form of a reveal must see the full one as new.
    #[tokio::test(start_paused = true)]
    async fn subtitle_lines_are_sequence_stamped_and_growth_restamps() {
        let mut advisor = Advisor::default();
        let line = |text: &str| SubtitleLine {
            text: text.into(),
            speaker: None,
            video_millis: 0,
            arrival_millis: 0,
        };
        advisor.observe_subtitles(&[line("First."), line("Com")]);
        assert_eq!(
            (advisor.subtitles[0].seq, &*advisor.subtitles[0].text),
            (1, "First.")
        );
        assert_eq!(
            (advisor.subtitles[1].seq, &*advisor.subtitles[1].text),
            (2, "Com")
        );
        // Growth re-stamps; the shrink-back changes nothing.
        advisor.observe_subtitles(&[line("Coming!"), line("Com")]);
        assert_eq!(
            (advisor.subtitles[1].seq, &*advisor.subtitles[1].text),
            (3, "Coming!")
        );
        assert_eq!(advisor.subtitles.len(), 2);
    }

    /// The ring keeps each cue's ASS speaker, tracks the latest speaker
    /// across an in-place growth (like the UI's subtitle log), and
    /// renders attribution as `Name: text` — the commentary model can't
    /// see the video, so its dialogue must arrive attributed.
    #[tokio::test(start_paused = true)]
    async fn ring_carries_the_speaker_for_attribution() {
        let mut advisor = Advisor::default();
        let line = |speaker: Option<&str>, text: &str| SubtitleLine {
            text: text.into(),
            speaker: speaker.and_then(SpeakerName::new),
            video_millis: 0,
            arrival_millis: 0,
        };
        advisor.observe_subtitles(&[line(Some("Amu"), "Whaaaat?"), line(None, "(gasps)")]);
        assert_eq!(advisor.subtitles[0].attributed(), "Amu: Whaaaat?");
        assert_eq!(advisor.subtitles[1].attributed(), "(gasps)");
        // An overlap join grows the line in place; the joined cue's
        // speaker wins, matching the UI log's latest-speaker rule.
        advisor.observe_subtitles(&[line(Some("Ikuto"), "(gasps) Yo.")]);
        assert_eq!(advisor.subtitles[1].attributed(), "Ikuto: (gasps) Yo.");
        assert_eq!(advisor.subtitles.len(), 2);
    }

    /// A provider that spawns a task owning a Sender clone (the future
    /// LLM shape) delivers asynchronously through the same channel.
    #[tokio::test(start_paused = true)]
    async fn async_provider_delivery_reaches_the_channel() {
        struct SlowProvider;
        impl AdvisorProvider for SlowProvider {
            fn name(&self) -> &'static str {
                "slow"
            }
            fn advise(&mut self, _ctx: &AdvisorContext, out: &mpsc::Sender<SuggestionUpdate>) {
                let out = out.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    let _ = out
                        .send(SuggestionUpdate(Some(Suggestion {
                            id: "commentary",
                            text: "what a scene".into(),
                            severity: Severity::Info,
                        })))
                        .await;
                });
            }
        }
        let mut advisor = Advisor::new(vec![Box::new(SlowProvider)]);
        advisor.on_health(
            &StateView::default(),
            LinkStatus::Connected,
            HealthLevel::Ok,
            None,
            false,
            false,
        );
        tokio::time::sleep(Duration::from_secs(3)).await;
        let update = advisor.suggestions.try_recv().expect("delivered");
        assert_eq!(update.0.unwrap().id, "commentary");
    }

    /// The 5s advise throttle: 1Hz health samples reach providers at
    /// most every five seconds, and a divergence bypasses the throttle.
    #[tokio::test(start_paused = true)]
    async fn advise_is_throttled_and_divergence_bypasses() {
        struct Counter(std::sync::Arc<std::sync::atomic::AtomicU32>);
        impl AdvisorProvider for Counter {
            fn name(&self) -> &'static str {
                "counter"
            }
            fn advise(&mut self, _ctx: &AdvisorContext, _out: &mpsc::Sender<SuggestionUpdate>) {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
        let count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let mut advisor = Advisor::new(vec![Box::new(Counter(count.clone()))]);
        let tick = |advisor: &mut Advisor| {
            advisor.on_health(
                &StateView::default(),
                LinkStatus::Connected,
                HealthLevel::Ok,
                None,
                false,
                false,
            );
        };
        for _ in 0..3 {
            tick(&mut advisor);
            tokio::time::advance(Duration::from_secs(1)).await;
        }
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
        advisor.on_diverged();
        tick(&mut advisor);
        assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 2);
    }
}
