//! State -> Props: the pure mapping from (resolved view, peer list,
//! local context) to what each pane displays. Components render these
//! verbatim; keeping the mapping pure makes the display rules testable
//! without a terminal (ui-architecture.md, State to Props Mapping).

use std::collections::{BTreeMap, BTreeSet};

use dessplay_core::derive::{self, DerivedUserState};
use dessplay_core::net::{PeerInfo, Presence, Role};
use dessplay_core::types::{
    AniDbSeriesId, Ed2kHash, FileAvailability, ListEntryId, ListStatus, ManualState, NextEpState,
    SeriesListEntry, SeriesWatchState, UserId, decode_action,
};
use dessplay_core::{StateView, franchise, series_identity};

use crate::player::SpeakerName;
use crate::storage::SeriesKey;

/// Semantic display tone; the theme maps these to colors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Green: ready / downloading-but-playable.
    Good,
    /// Red: blocking playback (missing file, committed-absent).
    Blocked,
    /// Yellow: manually paused. Blocks like red, but a paused friend is
    /// a different situation from a missing file (#18) — the attribution
    /// stays, the colour stops crying wolf.
    Paused,
    /// Blue: transferring, not yet playable.
    Transfer,
    /// Gray: away / not watching.
    Idle,
    /// Dim: history, departed, decoration.
    Muted,
    /// Default foreground.
    Normal,
}

// ---- Users pane ------------------------------------------------------

/// One row in the users pane.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct UserRow {
    /// Username.
    pub name: String,
    /// State label ("ready", "away, set by Baughn", "downloading 34%").
    pub label: String,
    /// Display tone (the design's ready-state color table).
    pub tone: Tone,
}

/// One row in the dim+italic "known offline" list (design.md #15):
/// this-session departures and never-connected-today users alike,
/// selectable as `n` / `/skip <name>` targets.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KnownOfflineRow {
    /// Username.
    pub name: String,
    /// "3d ago" / "5h ago" / "just now", relative to the snapshot's `now`.
    pub last_seen_label: String,
}

/// Users pane props: active rows plus the dim known-offline/seeder lines.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct UsersProps {
    /// Present and Lost interactive users.
    pub rows: Vec<UserRow>,
    /// Known usernames not represented in `rows` -- this-session
    /// departures and never-connected-today users alike (design.md #15).
    /// Dim + italic, selectable.
    pub known_offline: Vec<KnownOfflineRow>,
    /// Seeders (dim line), with a marker when not present.
    pub seeders: Vec<String>,
}

/// Render an elapsed duration as "Nd ago" / "Nh ago" / "Nm ago" / "just
/// now" -- whole units, largest that divides at least 1.
fn humanize_ago(elapsed_millis: u64) -> String {
    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;
    if elapsed_millis >= DAY {
        format!("{}d ago", elapsed_millis / DAY)
    } else if elapsed_millis >= HOUR {
        format!("{}h ago", elapsed_millis / HOUR)
    } else if elapsed_millis >= MINUTE {
        format!("{}m ago", elapsed_millis / MINUTE)
    } else {
        "just now".to_string()
    }
}

/// Names of chat participants: interactive peers that are present or lost
/// (seeders and departed users excluded). Used for chat tab-completion and
/// mention highlighting.
pub fn chat_usernames(peers: &[PeerInfo]) -> Vec<String> {
    peers
        .iter()
        .filter(|p| p.role != Role::Seeder && p.presence != Presence::Departed)
        .map(|p| p.username.to_string())
        .collect()
}

/// Build the users pane from the design's ready-state table. `known_offline`
/// is the server's persisted registry (design.md #15) of usernames not
/// currently Present; `now` is the snapshot's shared-clock millis, for the
/// "last seen Nd ago" labels.
pub fn users_props(
    view: &StateView,
    peers: &[PeerInfo],
    known_offline: &[dessplay_core::net::KnownUser],
    now: u64,
) -> UsersProps {
    let mut props = UsersProps::default();
    for peer in peers {
        let name = peer.username.to_string();
        if peer.role == Role::Seeder {
            props.seeders.push(match peer.presence {
                Presence::Present => name,
                Presence::Lost | Presence::Departed => format!("{name} (offline)"),
            });
            continue;
        }
        match peer.presence {
            // A committed (Watching) absent user keeps gating across
            // absence (until acknowledged), so they must read as a blocker
            // here too — never hidden on the dim known-offline line —
            // matching the `CommittedAbsent` blocker the status bar
            // surfaces. A Maybe/NotWatching absent user does not block:
            // Departed falls through to `known_offline` below, Lost shows
            // greyed (a dropped connection, not something we are waiting
            // on).
            Presence::Departed | Presence::Lost
                if committed_absent_blocker(view, &peer.username) =>
            {
                props.rows.push(UserRow {
                    name,
                    label: "committed, away".into(),
                    tone: Tone::Blocked,
                });
            }
            // A plain Departed peer gets no row here — they're represented
            // by `known_offline` below instead (design.md #15 unifies
            // this-session departures with the persisted registry).
            Presence::Departed => {}
            Presence::Lost => props.rows.push(UserRow {
                name,
                label: "lost".into(),
                tone: Tone::Idle,
            }),
            Presence::Present => {
                let state = derive::user_state(view, &peer.username);
                let avail = availability(view, &peer.username);
                let downloading = match avail {
                    Some(FileAvailability::Downloading { progress_bps }) => {
                        Some((progress_bps, false))
                    }
                    Some(FileAvailability::DownloadingPlayable { progress_bps }) => {
                        Some((progress_bps, true))
                    }
                    _ => None,
                };
                // An in-progress download is *always* shown (design.md
                // Ready States: "Downloading | Any & Downloading") — it
                // must never be shadowed by a paused/away/not-watching
                // label. Green once it can play and the user is Ready,
                // blue while a Ready user is still fetching, and red
                // otherwise: the download is visible, the colour says
                // they still won't be watching right now.
                // A present Maybe user displays exactly like Ready (the
                // per-series distinction lives in the playlist watch tag,
                // not here): both gate on their file state while present.
                let (label, tone) = match (downloading, &state) {
                    (Some((bps, playable)), DerivedUserState::Ready | DerivedUserState::Maybe) => {
                        let label = format!("downloading {}%", bps / 100);
                        // Green once the downloader says it can play from
                        // here (the synced playable verdict — the same
                        // signal gating uses), blue while still fetching.
                        if playable {
                            (label, Tone::Good)
                        } else {
                            (label, Tone::Transfer)
                        }
                    }
                    (Some((bps, _)), _) => (format!("downloading {}%", bps / 100), Tone::Blocked),
                    (None, DerivedUserState::Paused) => ("paused".to_string(), Tone::Paused),
                    (None, DerivedUserState::Away { set_by }) => {
                        (format!("away, set by {set_by}"), Tone::Idle)
                    }
                    (None, DerivedUserState::NotWatching) => {
                        ("not watching".to_string(), Tone::Idle)
                    }
                    // Ready/Maybe, not downloading: Downloading is
                    // impossible here (it would be `Some` above).
                    (None, DerivedUserState::Ready | DerivedUserState::Maybe) => match avail {
                        Some(FileAvailability::Missing) => {
                            ("missing file".to_string(), Tone::Blocked)
                        }
                        _ => ("ready".to_string(), Tone::Good),
                    },
                };
                props.rows.push(UserRow { name, label, tone });
            }
        }
    }
    // Anyone already given a row (Present, Lost, or a committed-absent
    // blocker) is fully represented there; `known_offline` covers the rest.
    let already_shown: std::collections::BTreeSet<&str> =
        props.rows.iter().map(|row| row.name.as_str()).collect();
    for user in known_offline {
        let name = user.username.to_string();
        if already_shown.contains(name.as_str()) {
            continue;
        }
        props.known_offline.push(KnownOfflineRow {
            name,
            last_seen_label: humanize_ago(now.saturating_sub(user.last_seen)),
        });
    }
    props
}

fn availability(view: &StateView, user: &UserId) -> Option<FileAvailability> {
    let file = view.now_playing?;
    view.file_availability.get(&(user.clone(), file)).copied()
}

/// Whether an absent (Lost/Departed) peer still gates playback: committed
/// (Watching) to the now-playing series and not yet acknowledged for that
/// file. Mirrors the `Watching + Lost|Departed` arm of
/// [`derive::playback_blockers`] so the Users pane and status bar agree.
fn committed_absent_blocker(view: &StateView, user: &UserId) -> bool {
    let Some(file) = view.now_playing else {
        return false;
    };
    // Away is the per-user escape hatch: it excuses a committed-absent user
    // from gating, so `derive::playback_blockers` early-`continue`s on it.
    // Mirror that here, or the Users pane would keep drawing a red
    // "committed, away" blocker after playback has already been allowed to
    // proceed — contradicting both gating and this helper's own contract.
    if matches!(
        view.manual_override.get(user).and_then(|m| m.as_ref()),
        Some(ManualState::Away { .. })
    ) {
        return false;
    }
    derive::series_watch_for_file(view, user, file) == SeriesWatchState::Watching
        && !view.acknowledged_absent.contains(&(file, user.clone()))
}

// ---- Playlist pane ---------------------------------------------------

/// One playlist row.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PlaylistRow {
    /// The file.
    pub hash: Ed2kHash,
    /// Display title (the original filename).
    pub title: String,
    /// Now-playing highlight / watched muting / missing red.
    pub tone: Tone,
    /// This row is now-playing.
    pub is_now: bool,
    /// The local copy lives only in the download cache, not a media root
    /// (drives the dim "temporary" marker and gates the archive action).
    pub temporary: bool,
    /// Our own in-flight download of this entry, as progress basis
    /// points — downloads mostly happen in the background (prefetch), so
    /// the playlist is where their progress is visible without selecting
    /// the file. `None` once complete (Ready) or when not downloading.
    pub download: Option<u16>,
    /// The local user's effective watch state for this entry's series,
    /// always shown as a right-aligned tag. Maybe is the default.
    pub watch: SeriesWatchState,
}

/// Playlist pane props.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct PlaylistProps {
    /// Rows in playlist order.
    pub rows: Vec<PlaylistRow>,
    /// Index of the now-playing row.
    pub now_index: Option<usize>,
}

/// Build the playlist pane: now-playing highlighted, group-watched
/// entries muted (play history), files *we* lack in red. `cache_hashes`
/// are the files we hold only in the download cache (rendered with a
/// dim "temporary" marker).
pub fn playlist_props(
    view: &StateView,
    me: &UserId,
    cache_hashes: &BTreeSet<Ed2kHash>,
) -> PlaylistProps {
    let mut props = PlaylistProps::default();
    for (index, entry) in view.playlist.iter().enumerate() {
        let is_now = view.now_playing == Some(entry.hash);
        let avail = view
            .file_availability
            .get(&(me.clone(), entry.hash))
            .copied();
        let missing = avail == Some(FileAvailability::Missing);
        let download = match avail {
            Some(
                FileAvailability::Downloading { progress_bps }
                | FileAvailability::DownloadingPlayable { progress_bps },
            ) => Some(progress_bps),
            _ => None,
        };
        let watched = view.watched.get(&entry.hash) == Some(&true);
        let temporary = cache_hashes.contains(&entry.hash) && !missing;
        let tone = if missing {
            Tone::Blocked
        } else if is_now {
            Tone::Good
        } else if watched {
            Tone::Muted
        } else {
            Tone::Normal
        };
        if is_now {
            props.now_index = Some(index);
        }
        props.rows.push(PlaylistRow {
            hash: entry.hash,
            title: entry.state.filename.clone(),
            tone,
            is_now,
            temporary,
            download,
            watch: derive::series_watch_for_file(view, me, entry.hash),
        });
    }
    props
}

// ---- Chat pane -------------------------------------------------------

/// One formatted chat line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChatLine {
    /// Display time. For chat/system lines this is "HH:MM" in the
    /// machine's local timezone; for subtitle lines it is the in-video
    /// position "MM:SS" (see `subtitle`) — a different clock domain from
    /// `millis`.
    pub time: String,
    /// Sender name (empty for system and subtitle lines).
    pub sender: String,
    /// Message body.
    pub text: String,
    /// A local system line (archive result, etc.): rendered dim with no
    /// sender, never synced.
    pub system: bool,
    /// A local subtitle line folded into the chat log (Intermixed mode):
    /// rendered dim with a `»` marker and an in-video `time`, never
    /// synced.
    pub subtitle: bool,
    /// A render-time day separator (the biblical-day divider): `text`
    /// holds the date label, rendered centered between dashes. Not a
    /// message — computed from timestamps, never stored or synced.
    pub separator: bool,
    /// An IRC-style action ("/me waves"): rendered "* sender text" with no
    /// colon. `text` holds the decoded action phrase (the CTCP wrapper is
    /// stripped). UI-only flag derived from the message text.
    pub action: bool,
    /// A local-only line from an external IRC user (the IRC bridge):
    /// rendered like normal chat (colored sender, mention highlight) but
    /// prefixed with a dim `irc` tag so it isn't mistaken for a dessplay
    /// peer. Never synced.
    pub irc: bool,
    /// Shared-clock millis, the interleave key across synced messages,
    /// local system lines, and subtitle arrivals. For subtitle lines
    /// this is wall-clock *arrival*, not the in-video `time`.
    pub millis: u64,
}

/// Strip control characters from remote-authored text before display:
/// ratatui writes cell symbols through to the terminal, so a raw escape
/// byte in a hostile or malformed synced/IRC message would land in the
/// user's emulator. Display boundary only — the synced state keeps the
/// raw text (the CTCP-action rule: only the display sites decode and
/// sanitize).
fn strip_control_chars(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Format the chat log.
pub fn chat_lines(view: &StateView) -> Vec<ChatLine> {
    view.chat
        .iter()
        .map(|message| {
            // An IRC-style action carries its phrase CTCP-encoded in the
            // text; decode it here so the renderer sees a plain phrase plus
            // the `action` flag.
            let (text, action) = match decode_action(&message.text) {
                Some(phrase) => (strip_control_chars(phrase), true),
                None => (strip_control_chars(&message.text), false),
            };
            ChatLine {
                time: hhmm(message.timestamp.0),
                sender: message.sender.to_string(),
                text,
                system: false,
                subtitle: false,
                separator: false,
                action,
                irc: false,
                millis: message.timestamp.0,
            }
        })
        .collect()
}

/// Build a local system chat line (dim, no sender).
pub fn system_line(timestamp: u64, text: String) -> ChatLine {
    ChatLine {
        time: hhmm(timestamp),
        sender: String::new(),
        text,
        system: true,
        subtitle: false,
        separator: false,
        action: false,
        irc: false,
        millis: timestamp,
    }
}

/// Build a local chat line from an external IRC user. Rendered like
/// normal chat (colored sender, mention highlight) but flagged `irc` so
/// the renderer tags it; never synced.
pub fn irc_line(timestamp: u64, sender: String, text: String, action: bool) -> ChatLine {
    ChatLine {
        time: hhmm(timestamp),
        sender,
        text: strip_control_chars(&text),
        system: false,
        subtitle: false,
        separator: false,
        action,
        irc: true,
        millis: timestamp,
    }
}

/// Build a day-separator line for `millis`. `text` is the date label
/// (e.g. "Thursday, June 18"); the chat pane renders it centered between
/// dashes. The displayed-time field is unused.
pub fn day_separator(millis: u64) -> ChatLine {
    ChatLine {
        time: String::new(),
        sender: String::new(),
        text: biblical_date(millis).map_or_else(String::new, |d| {
            d.format("%A, %B ").to_string() + &d.format("%-d").to_string()
        }),
        system: false,
        subtitle: false,
        separator: true,
        action: false,
        irc: false,
        millis,
    }
}

/// The "biblical" calendar day for a timestamp (09:00-local boundary).
/// Defined in [`crate::timeutil`] so non-UI code (daily log rotation)
/// can share it; re-exported here for the chat day separators.
pub use crate::timeutil::biblical_date;

/// Format the body shared by both subtitle text modes, optionally prefixing
/// the ASS speaker name. Unnamed cues remain byte-for-byte unchanged.
/// [`SpeakerName`] is non-empty by construction, so `Some` always prefixes
/// when names are shown.
pub fn subtitle_text(text: &str, speaker: Option<&SpeakerName>, show_speaker: bool) -> String {
    match speaker.filter(|_| show_speaker) {
        Some(speaker) => format!("{speaker}: {text}"),
        None => text.to_owned(),
    }
}

/// Build a local subtitle chat line for Intermixed mode. The displayed time
/// is the in-video position, while interleaving uses wall-clock arrival.
/// Speaker names use the same helper as Separate mode so the modes cannot
/// drift.
pub fn subtitle_line(
    video_millis: u64,
    arrival_millis: u64,
    text: String,
    speaker: Option<&SpeakerName>,
    show_speaker: bool,
) -> ChatLine {
    ChatLine {
        time: mmss(video_millis),
        sender: String::new(),
        text: subtitle_text(&text, speaker, show_speaker),
        system: false,
        subtitle: true,
        separator: false,
        action: false,
        irc: false,
        millis: arrival_millis,
    }
}

/// Unix millis -> "HH:MM:SS" in the machine's local timezone. The
/// irccloud-style chat-selection copy format; the on-screen log shows
/// only HH:MM.
pub(crate) fn hhmmss(millis: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(millis as i64).single() {
        Some(dt) => dt.format("%H:%M:%S").to_string(),
        // Out-of-range timestamp; fall back to naive UTC math.
        None => {
            let secs = (millis / 1_000) % (24 * 3600);
            format!(
                "{:02}:{:02}:{:02}",
                secs / 3600,
                (secs / 60) % 60,
                secs % 60
            )
        }
    }
}

/// Unix millis -> "HH:MM" in the machine's local timezone.
fn hhmm(millis: u64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(millis as i64).single() {
        Some(dt) => dt.format("%H:%M").to_string(),
        // Out-of-range timestamp; fall back to naive UTC math.
        None => {
            let minutes = (millis / 60_000) % (24 * 60);
            format!("{:02}:{:02}", minutes / 60, minutes % 60)
        }
    }
}

/// In-video position millis -> "MM:SS" (or "H:MM:SS" past an hour). Used
/// for subtitle timestamps, which are video-relative, not wall-clock.
pub fn mmss(millis: u64) -> String {
    let total = millis / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

// ---- Player status ---------------------------------------------------

/// The server-link state shown on the status bar. Without it a dead
/// handshake (which can run the full per-address timeout ladder) is
/// indistinguishable from a hang — the 2026-07-06 post-wake IPv6 black
/// hole read as "dessplay froze" (design.md UI principles: no silent
/// long-running work).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LinkStatus {
    /// Dialing the server (initial connect or a retry).
    Connecting {
        /// 1-based attempt counter from the network actor.
        attempt: u64,
    },
    /// Authenticated and syncing.
    Connected,
    /// Connection lost; the network actor is between retries.
    Down,
}

impl Default for LinkStatus {
    /// The state at startup, before the first network event arrives.
    fn default() -> Self {
        LinkStatus::Connecting { attempt: 1 }
    }
}

/// Player status bar props.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StatusProps {
    /// Server-link state; anything but `Connected` replaces the
    /// play-state text (stale gating info is noise while offline).
    pub link: LinkStatus,
    /// Now-playing filename.
    pub title: Option<String>,
    /// The derived playback state.
    pub playing: bool,
    /// Who blocks playback, formatted ("kim (paused)").
    pub blockers: Vec<String>,
    /// Our playback position in millis, if known.
    pub position_millis: Option<u64>,
    /// The file's duration in millis, if known.
    pub duration_millis: Option<u64>,
}

/// Build the status bar.
pub fn status_props(
    view: &StateView,
    peers: &[PeerInfo],
    me: &UserId,
    link: LinkStatus,
) -> StatusProps {
    let title = view.now_playing.and_then(|hash| {
        view.playlist
            .iter()
            .find(|entry| entry.hash == hash)
            .map(|entry| entry.state.filename.clone())
    });
    let duration_millis = view.now_playing.and_then(|hash| {
        view.playlist
            .iter()
            .find(|entry| entry.hash == hash)
            .and_then(|entry| entry.state.duration_millis)
    });
    let blockers = derive::playback_blockers(view, peers)
        .into_iter()
        .map(|blocker| {
            let reason = match blocker.reason {
                derive::BlockReason::Paused => "paused",
                derive::BlockReason::FileMissing => "missing file",
                derive::BlockReason::Downloading => "downloading",
                derive::BlockReason::CommittedAbsent => "committed, away",
            };
            format!("{} ({reason})", blocker.user)
        })
        .collect();
    StatusProps {
        link,
        title,
        playing: derive::playback_active(view, peers),
        blockers,
        position_millis: view
            .playback_position
            .get(me)
            .map(|position| position.position_millis),
        duration_millis,
    }
}

// ---- Health line -----------------------------------------------------

/// One sample of connection/sync health, merged by the session loop
/// from the network actor's `LinkHealth` report and the torrent
/// engine's live speeds. Raw numbers only; classification, colors, and
/// formatting are pure functions over this.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HealthSample {
    /// Round-trip estimate, milliseconds (time-sync probes, falling
    /// back to the QUIC path estimate before any probe is answered).
    pub rtt_millis: Option<u64>,
    /// Consecutive steady-state time-sync probes without an answer.
    pub unanswered_probes: u32,
    /// Milliseconds since anything arrived from the server. The server
    /// broadcasts a StateHash at least every 30s, so a large value on a
    /// live connection means sync is stalled.
    pub server_silence_millis: u64,
    /// Upload bytes/sec: the QUIC plane plus the torrent engine.
    pub up_bps: u64,
    /// Download bytes/sec: the QUIC plane plus the torrent engine.
    pub down_bps: u64,
}

/// Health classification for the status field: dim / yellow / red.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum HealthLevel {
    /// Everything nominal — the row renders dim.
    #[default]
    Ok,
    /// The link is struggling (bufferbloat, lost probes): yellow on the
    /// offending fields.
    Degraded,
    /// Sync is effectively dead even though QUIC is up: red.
    Stalled,
}

/// RTT at which the link counts as degraded. Ordinary paths run tens of
/// milliseconds; a saturated uplink (the Starlink incident) shows
/// seconds of bufferbloat.
const RTT_DEGRADED_MILLIS: u64 = 1_500;
/// Server silence beyond one missed 30s StateHash interval (plus
/// margin): degraded.
const SILENCE_DEGRADED_MILLIS: u64 = 40_000;
/// Server silence beyond 2.5 missed StateHash intervals: sync is dead.
const SILENCE_STALLED_MILLIS: u64 = 75_000;
/// With this many consecutive lost probes, a shorter silence already
/// counts as stalled — two independent signals agree.
const SILENCE_WITH_PROBES_STALLED_MILLIS: u64 = 45_000;
/// During group playback (another interactive peer present), server
/// silence beyond this shows as an age instead of "sync ok" — peers'
/// position datagrams normally arrive continuously, so even a few
/// seconds of quiet is worth an eye. Display only; classification
/// keeps the heartbeat-anchored thresholds above.
const SILENCE_SHOW_PLAYBACK_MILLIS: u64 = 5_000;
/// Consecutive unanswered 30s probes for degraded / stalled.
const PROBES_DEGRADED: u32 = 2;
const PROBES_STALLED: u32 = 3;

/// Classify one sample. Only meaningful while `Connected` — any other
/// link state renders its own text and returns `Ok` here (the status
/// bar carries the connecting/lost story).
pub fn classify_health(link: LinkStatus, sample: Option<&HealthSample>) -> HealthLevel {
    let (LinkStatus::Connected, Some(sample)) = (link, sample) else {
        return HealthLevel::Ok;
    };
    let silence = sample.server_silence_millis;
    if silence > SILENCE_STALLED_MILLIS
        || (sample.unanswered_probes >= PROBES_STALLED
            && silence > SILENCE_WITH_PROBES_STALLED_MILLIS)
    {
        return HealthLevel::Stalled;
    }
    if sample
        .rtt_millis
        .is_some_and(|rtt| rtt >= RTT_DEGRADED_MILLIS)
        || silence > SILENCE_DEGRADED_MILLIS
        || sample.unanswered_probes >= PROBES_DEGRADED
    {
        return HealthLevel::Degraded;
    }
    HealthLevel::Ok
}

/// Anti-flap for the displayed level: worse levels apply immediately,
/// better ones only after five consecutive calmer samples (~5s) — a
/// single quiet second must not flicker a red row back to dim.
#[derive(Clone, Copy, Debug, Default)]
pub struct HealthHysteresis {
    current: HealthLevel,
    calmer_streak: u8,
    /// Worst raw level seen during the current calmer streak: the
    /// downgrade lands there, so Stalled steps down through an
    /// intervening Degraded instead of jumping past it.
    streak_max: HealthLevel,
}

/// Consecutive calmer samples required before the display improves.
const CALM_SAMPLES_TO_DOWNGRADE: u8 = 5;

impl HealthHysteresis {
    /// Feed one raw sample; returns the level to display.
    pub fn observe(&mut self, raw: HealthLevel) -> HealthLevel {
        if raw >= self.current {
            self.current = raw;
            self.calmer_streak = 0;
            self.streak_max = HealthLevel::Ok;
        } else {
            self.calmer_streak += 1;
            self.streak_max = self.streak_max.max(raw);
            if self.calmer_streak >= CALM_SAMPLES_TO_DOWNGRADE {
                self.current = self.streak_max;
                self.calmer_streak = 0;
                self.streak_max = HealthLevel::Ok;
            }
        }
        self.current
    }

    /// The currently displayed level.
    pub fn current(&self) -> HealthLevel {
        self.current
    }

    /// Reset (connection went away — stale trouble must not outlive it).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// A suggestion for the health row's right-aligned slot, already
/// reduced to display terms (the advisor's severity became a tone).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SuggestionProps {
    /// The suggestion text.
    pub text: String,
    /// Display tone (Muted / Paused / Blocked).
    pub tone: Tone,
}

/// Everything the health line renders.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct HealthProps {
    /// Server-link state; anything but `Connected` replaces the metrics
    /// with a short link notice.
    pub link: LinkStatus,
    /// Displayed (hysteresis-filtered) health level.
    pub level: HealthLevel,
    /// The latest sample; `None` before the first report (or after a
    /// disconnect cleared it).
    pub sample: Option<HealthSample>,
    /// Group playback is actually running (the derived state).
    pub playing: bool,
    /// Another *interactive* peer is Present. With company the wire is
    /// chatty (their ops and position datagrams arrive constantly), so
    /// the sync field's "worth showing" bar tightens; alone, the only
    /// incoming traffic is two interleaved 30s heartbeats and the age
    /// legitimately sawtooths toward 30s.
    pub company: bool,
    /// The advisor's current suggestion, right-aligned.
    pub suggestion: Option<SuggestionProps>,
}

/// How a new subtitle observation relates to the previously logged
/// line — the incremental-reveal / overlapping-cue collapse (design.md,
/// Subtitle Display). mpv re-emits the whole joined cue-set on every
/// change, so consecutive observations are often the *same* utterance
/// growing or shrinking; the length test makes the two cases mutually
/// exclusive, and either end of the join may carry the change, so both
/// prefix and suffix count. Shared by the UI's display buffer and the
/// advisor's context ring, so the two logs can never disagree on what
/// counts as one line.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SubtitleCollapse {
    /// Same cue, fuller now (a reveal, or an overlapping neighbour
    /// appearing): replace the previous line in place.
    Extends,
    /// Same cue receding (an overlapping neighbour ended): the fuller
    /// text is already logged — drop the redundant re-show. (An exact
    /// repeat has equal length, so it classifies as `Extends` — an
    /// in-place no-op replacement; same outcome, one line.)
    Contained,
    /// Strictly distinct text: a new line.
    Distinct,
}

/// Classify `next` against the previous logged line (`None` = empty log).
pub fn subtitle_collapse(prev: Option<&str>, next: &str) -> SubtitleCollapse {
    let Some(last) = prev else {
        return SubtitleCollapse::Distinct;
    };
    if next.len() >= last.len() && (next.starts_with(last) || next.ends_with(last)) {
        SubtitleCollapse::Extends
    } else if next.len() < last.len() && (last.starts_with(next) || last.ends_with(next)) {
        SubtitleCollapse::Contained
    } else {
        SubtitleCollapse::Distinct
    }
}

/// Compact byte rate for the one-line health row: `0B`, `340K`, `1.2M`
/// (decimal units, one decimal place from M up). Deliberately terse —
/// the row shares 50 columns with a suggestion slot at common terminal
/// widths.
pub fn fmt_rate(bps: u64) -> String {
    if bps < 1_000 {
        format!("{bps}B")
    } else if bps < 1_000_000 {
        format!("{}K", bps / 1_000)
    } else {
        format!("{:.1}M", bps as f64 / 1_000_000.0)
    }
}

/// Marquee scroll rate, display cells per second. Wall-millis-derived
/// (offset = elapsed × rate), so tick jitter never changes trajectory.
pub const MARQUEE_CELLS_PER_SEC: u64 = 15;

/// The visible window of a marquee line at a given scroll offset.
///
/// The line enters entirely off-screen right (offset 0 shows nothing —
/// the whole point: motion draws the eye *before* the sentence starts
/// leaving) and exits entirely off-screen left (nothing again at
/// `free + text_width`). Returns the visible substring and its left
/// padding within the `free`-cell slot, or `None` when no cell of the
/// text is on screen. Display-cell aware: wide (CJK) chars count two
/// cells, and one straddling the left cut is dropped, leaving its
/// half-cell as padding.
pub fn marquee_window(text: &str, free: usize, offset: usize) -> Option<(String, usize)> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    if free == 0 {
        return None;
    }
    let text_width = text.width();
    if offset == 0 || offset >= free + text_width {
        return None;
    }
    let left_edge = free as isize - offset as isize;
    if left_edge >= 0 {
        // Entering / fully visible: pad, then the prefix that fits.
        let pad = left_edge as usize;
        let avail = free - pad;
        let mut out = String::new();
        let mut used = 0;
        for ch in text.chars() {
            let w = ch.width().unwrap_or(0);
            if used + w > avail {
                break;
            }
            out.push(ch);
            used += w;
        }
        Some((out, pad))
    } else {
        // Exiting: skip the cells already gone past the left edge.
        let skip = (-left_edge) as usize;
        let mut skipped = 0;
        let mut out = String::new();
        let mut used = 0;
        for ch in text.chars() {
            let w = ch.width().unwrap_or(0);
            if skipped < skip {
                skipped += w;
                continue;
            }
            if used + w > free {
                break;
            }
            out.push(ch);
            used += w;
        }
        let pad = skipped.saturating_sub(skip);
        if out.is_empty() {
            return None;
        }
        Some((out, pad))
    }
}

/// The health row's left-hand metric fragments, in display order, as
/// (text, tone) pairs — the component joins them with dim `·`
/// separators. Pure, and the per-field warning tones live here with the
/// thresholds they mirror, so the row's story is testable without a
/// terminal. Only the *offending* field warns; the rest stay dim.
pub fn health_fragments(props: &HealthProps) -> Vec<(String, Tone)> {
    match props.link {
        // The status bar carries the full "⚡ connecting (attempt N)"
        // story; the health row just avoids showing stale metrics.
        LinkStatus::Connecting { .. } => vec![("link: connecting…".into(), Tone::Paused)],
        LinkStatus::Down => vec![("link: down — retrying".into(), Tone::Paused)],
        LinkStatus::Connected => {
            let Some(sample) = &props.sample else {
                return vec![("link: measuring…".into(), Tone::Muted)];
            };
            let mut fragments = vec![(
                format!(
                    "▲{} ▼{}",
                    fmt_rate(sample.up_bps),
                    fmt_rate(sample.down_bps)
                ),
                Tone::Muted,
            )];
            if let Some(rtt) = sample.rtt_millis {
                let tone = if rtt >= RTT_DEGRADED_MILLIS {
                    Tone::Paused
                } else {
                    Tone::Muted
                };
                fragments.push((format!("rtt {rtt}ms"), tone));
            }
            let silence = sample.server_silence_millis;
            let sync_tone = if silence > SILENCE_STALLED_MILLIS {
                Tone::Blocked
            } else if silence > SILENCE_DEGRADED_MILLIS {
                Tone::Paused
            } else {
                Tone::Muted
            };
            // A static "sync ok" until the age is worth attention — a
            // counting number draws the eye, and what counts as normal
            // depends on how chatty the wire should be: during group
            // playback peers' position updates arrive constantly, so a
            // few seconds of silence is already news; alone or idle,
            // only the 30s heartbeats arrive and the age is shown only
            // once it would warn anyway.
            let show_from = if props.playing && props.company {
                SILENCE_SHOW_PLAYBACK_MILLIS
            } else {
                SILENCE_DEGRADED_MILLIS
            };
            let sync_text = if silence > show_from {
                format!("sync {}s", silence / 1000)
            } else {
                "sync ok".to_string()
            };
            fragments.push((sync_text, sync_tone));
            if sample.unanswered_probes > 0 {
                let tone = if sample.unanswered_probes >= PROBES_STALLED {
                    Tone::Blocked
                } else if sample.unanswered_probes >= PROBES_DEGRADED {
                    Tone::Paused
                } else {
                    Tone::Muted
                };
                fragments.push((format!("{} probes lost", sample.unanswered_probes), tone));
            }
            fragments
        }
    }
}

// ---- Series pane -----------------------------------------------------

/// Sort order for the All Series mode.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SeriesSort {
    /// Alphabetical.
    #[default]
    Title,
    /// By first air year, then title.
    Year,
}

impl SeriesSort {
    /// Stable string for persistence in the settings table.
    pub fn as_str(self) -> &'static str {
        match self {
            SeriesSort::Title => "title",
            SeriesSort::Year => "year",
        }
    }

    /// Parse a persisted value; `None` for an unrecognized string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "title" => Some(SeriesSort::Title),
            "year" => Some(SeriesSort::Year),
            _ => None,
        }
    }
}

// ---- File browser ------------------------------------------------------

/// Sort order for the add/map file browser (design.md #8). `Newest`
/// overrides both the plain alphabetical listing and the Map browser's
/// edit-distance-to-target ranking -- it's an explicit "show me what just
/// landed" toggle, not a tiebreaker layered on top of either.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BrowserSort {
    /// The browser's normal order: alphabetical, or (Map purpose)
    /// edit-distance to the target filename.
    #[default]
    Alphabetical,
    /// Newest mtime first (from the library index, or a live stat for a
    /// not-yet-indexed file); directories stay first, alphabetical.
    Newest,
}

impl BrowserSort {
    /// Stable string for persistence in the settings table.
    pub fn as_str(self) -> &'static str {
        match self {
            BrowserSort::Alphabetical => "alphabetical",
            BrowserSort::Newest => "newest",
        }
    }

    /// Parse a persisted value; `None` for an unrecognized string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "alphabetical" => Some(BrowserSort::Alphabetical),
            "newest" => Some(BrowserSort::Newest),
            _ => None,
        }
    }

    /// Cycle to the other value (the file browser only has two).
    pub fn toggled(self) -> Self {
        match self {
            BrowserSort::Alphabetical => BrowserSort::Newest,
            BrowserSort::Newest => BrowserSort::Alphabetical,
        }
    }
}

/// One franchise row (Recent / All modes).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FranchiseRow {
    /// Selection key.
    pub key: franchise::FranchiseKey,
    /// Display title.
    pub title: String,
    /// First air year, if known.
    pub year: Option<u16>,
}

/// Sort order for The List mode (design.md, The List: UI Integration).
/// `Hash` because the sort is one of [`ListGroupsCache`]'s fingerprint
/// inputs.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum ListSort {
    /// Watchable entries first (this week's episode out, or unwatched
    /// files held), most recently watched first within each partition.
    /// The default: the nightly "what's next" order.
    #[default]
    Recency,
    /// By entry name.
    Alphabetical,
}

impl ListSort {
    /// Stable string for persistence in the settings table.
    pub fn as_str(self) -> &'static str {
        match self {
            ListSort::Recency => "recency",
            ListSort::Alphabetical => "alphabetical",
        }
    }

    /// Parse a persisted value; `None` for an unrecognized string.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "recency" => Some(ListSort::Recency),
            "alphabetical" => Some(ListSort::Alphabetical),
            _ => None,
        }
    }

    /// Cycle to the other value (The List only has two).
    pub fn toggled(self) -> Self {
        match self {
            ListSort::Recency => ListSort::Alphabetical,
            ListSort::Alphabetical => ListSort::Recency,
        }
    }
}

/// One row of The List.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListRow {
    /// Entry id (selection / edit target).
    pub id: ListEntryId,
    /// Display title: the preferred community short title for a linked
    /// entry that has one (`SeriesRelations::short_titles`), else the
    /// entry's own name. Also the alphabetical sort key — the row sorts
    /// where it reads.
    pub name: String,
    /// Nero's title.
    pub nero_name: Option<String>,
    /// Next episode display text: `SnEnn` for a linked entry whose
    /// free-text `next_ep` parses as a plain episode number (the season
    /// ordinal counted along the prequel chain), verbatim otherwise.
    pub next_ep: Option<String>,
    /// This week's episode is out.
    pub available: bool,
    /// Live commitment initials ("BN"): the users whose
    /// `series_preference` for this entry is Watching — not the
    /// import-time `watchers` seed, which records intent once and never
    /// tracks later `/watch`/`/skip` changes.
    pub watchers: String,
    /// The linked series, if any.
    pub series_id: Option<AniDbSeriesId>,
    /// An AniDB name search for this (unlinked) entry came back with zero
    /// hits (design.md, Series Identity) -- confirmed not on AniDB, not
    /// just never checked.
    pub anidb_unavailable: bool,
    /// Nothing to watch right now: the weekly `available` flag is off
    /// *and* no still-held, unwatched file (by the group flag or
    /// personal watch history — see [`entries_with_unwatched_files`])
    /// maps to this entry. Renders dim, and is exactly the bottom
    /// partition of the Recency sort.
    pub dimmed: bool,
}

/// A status group of List rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListGroup {
    /// Group heading ("Watching — Baughn", "Short List", ...).
    pub heading: String,
    /// Rows, in [`ListSort`] order.
    pub rows: Vec<ListRow>,
    /// Render collapsed by default (Finished / Dropped).
    pub collapsed: bool,
}

/// Recent Series recency: [`SeriesKey`] -> newest local watch time
/// (shared-clock millis), built from personal watch history.
///
/// The series is resolved from the *current* metadata `view` (keyed by
/// file hash), **not** the values frozen into the watch record: a file is
/// often watched (85%) before its AniDB metadata arrives. An AniDB id keys
/// the entry when known; otherwise the filename-derived series *name*
/// does, so files AniDB doesn't recognise still surface (matching the
/// name-keyed franchises in [`franchise::franchises`]). The record's own
/// frozen id/name is the last-ditch fallback. A file with no series
/// identity anywhere is skipped.
pub fn watch_recency(
    records: &[crate::storage::WatchRecord],
    view: &StateView,
) -> BTreeMap<SeriesKey, u64> {
    let mut recency: BTreeMap<SeriesKey, u64> = BTreeMap::new();
    for record in records {
        let meta = view
            .anidb_metadata
            .get(&record.hash)
            .and_then(|m| m.as_ref());
        let key = meta
            .and_then(|m| m.series_id)
            .or(record.series_id)
            .map(SeriesKey::AniDb)
            .or_else(|| {
                meta.map(|m| m.series_name.clone())
                    .or_else(|| record.series_name.clone())
                    .map(SeriesKey::Name)
            });
        let Some(key) = key else { continue };
        let watched_at = record.watched_at as u64;
        recency
            .entry(key)
            .and_modify(|t| *t = (*t).max(watched_at))
            .or_insert(watched_at);
    }
    recency
}

/// Franchise rows for the Recent / All modes. `recency` is `Some` only in
/// Recent mode and maps series to last-watched shared-clock millis (from
/// local watch history); a franchise's recency is the newest watch among
/// its members.
///
/// Visibility:
/// - **Recent mode, no filter**: only *watched* franchises (those with a
///   recency entry), newest first then title. Unwatched shows are hidden.
/// - **A non-empty `filter`** (either mode): case-insensitive substring on
///   the title, which *removes* the watched-only restriction so any series
///   can be found by typing. Recent still orders watched matches first.
/// - **All mode, no filter**: every franchise, by title or year-then-title.
pub fn franchise_rows_from(
    franchises: &[franchise::Franchise],
    sort: SeriesSort,
    recency: Option<&BTreeMap<SeriesKey, u64>>,
    filter: &str,
) -> Vec<FranchiseRow> {
    let mut rows: Vec<(Option<u64>, FranchiseRow)> = franchises
        .iter()
        .map(|franchise| {
            let last_watched = recency.and_then(|map| {
                // Match by any AniDB id the franchise spans, and — for a
                // name-keyed (filename-derived) franchise — by its title.
                let by_id = franchise
                    .series
                    .iter()
                    .filter_map(|id| map.get(&SeriesKey::AniDb(*id)).copied());
                let by_name = map.get(&SeriesKey::Name(franchise.title.clone())).copied();
                by_id.chain(by_name).max()
            });
            (
                last_watched,
                FranchiseRow {
                    key: franchise.key.clone(),
                    title: franchise.title.clone(),
                    year: franchise.year,
                },
            )
        })
        .collect();

    let needle = filter.trim().to_lowercase();
    if !needle.is_empty() {
        rows.retain(|(_, row)| row.title.to_lowercase().contains(&needle));
    } else if recency.is_some() {
        // Recent mode default: watched franchises only.
        rows.retain(|(watched, _)| watched.is_some());
    }

    match (recency, sort) {
        (Some(_), _) => rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.title.cmp(&b.1.title))),
        (None, SeriesSort::Title) => rows.sort_by(|a, b| a.1.title.cmp(&b.1.title)),
        (None, SeriesSort::Year) => rows.sort_by(|a, b| {
            a.1.year
                .unwrap_or(u16::MAX)
                .cmp(&b.1.year.unwrap_or(u16::MAX))
                .then_with(|| a.1.title.cmp(&b.1.title))
        }),
    }
    rows.into_iter().map(|(_, row)| row).collect()
}

/// Convenience for callers without a [`franchise::FranchiseCache`] (tests,
/// one-shots): computes the grouping fresh, then [`franchise_rows_from`].
/// The hot path (the UI snapshot) goes through the cache instead.
pub fn franchise_rows(
    view: &StateView,
    sort: SeriesSort,
    recency: Option<&BTreeMap<SeriesKey, u64>>,
    filter: &str,
) -> Vec<FranchiseRow> {
    franchise_rows_from(&franchise::franchises(view), sort, recency, filter)
}

/// A human-readable label for a file in the episode browser. In order of
/// preference:
/// 1. the playlist entry's filename (the real on-disk name);
/// 2. the file catalog's per-file filename — a real filename beats the
///    cosmetic "series — episode" form below whenever one is known, which
///    is what lets [`episode_rows`] tell same-episode copies apart by
///    filename instead of rendering N identical rows; it also distinguishes
///    episodes when metadata is filename-derived, whose `series_name` is a
///    directory hint shared by every episode of the series (so that name
///    must *not* be used as a per-episode label);
/// 3. AniDB's "series — episode" when the file has a real episode number
///    but, unusually, no catalog entry;
/// 4. the bare `series_name` (better than a raw hash when nothing else);
/// 5. the raw hash.
pub fn episode_label(view: &StateView, hash: &Ed2kHash) -> String {
    if let Some(entry) = view.playlist.iter().find(|entry| entry.hash == *hash) {
        return entry.state.filename.clone();
    }
    if let Some(entry) = view.file_catalog.get(hash) {
        return entry.filename.clone();
    }
    if let Some(Some(metadata)) = view.anidb_metadata.get(hash)
        && let Some(ep) = &metadata.episode_number
    {
        return format!("{} — {}", metadata.series_name, ep);
    }
    if let Some(Some(metadata)) = view.anidb_metadata.get(hash) {
        return metadata.series_name.clone();
    }
    hash.to_string()
}

/// One token of a natural-order parse: numeric runs compare as numbers, text
/// runs case-insensitively. Deriving `Ord` puts `Num` before `Text`, the
/// conventional "numbers sort first" behaviour.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum NatToken {
    Num(u64),
    Text(String),
}

/// Split a string into natural-order tokens: maximal ASCII-digit runs become
/// `Num` (saturating on overflow), everything else lowercased `Text`. This is
/// what makes "ep 2" sort before "ep 10".
fn natural_tokens(s: &str) -> Vec<NatToken> {
    let mut tokens = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            let mut n: u64 = 0;
            while let Some(&d) = chars.peek() {
                let Some(digit) = d.to_digit(10) else { break };
                n = n.saturating_mul(10).saturating_add(u64::from(digit));
                chars.next();
            }
            tokens.push(NatToken::Num(n));
        } else {
            let mut text = String::new();
            while let Some(&t) = chars.peek() {
                if t.is_ascii_digit() {
                    break;
                }
                text.extend(t.to_lowercase());
                chars.next();
            }
            tokens.push(NatToken::Text(text));
        }
    }
    tokens
}

/// Sort key for ordering episodes within a season. Compared field by field
/// in declaration order (derived `Ord`).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct EpisodeSortKey {
    /// `false` (a number was parsed) sorts before `true`.
    unnumbered: bool,
    /// Regular=0, special "S"=1, credit "C"=2, trailer "T"=3, parody/promo
    /// "P"=4, anything else=5.
    category: u8,
    /// The numeric part of the episode number.
    number: u64,
    /// Natural-order parse of the display label; final tiebreak, and the sole
    /// ordering for unnumbered episodes.
    fallback: Vec<NatToken>,
}

/// Parse an AniDB `episode_number` string ("03", "S1", "C1", ...) into its
/// `(category, number)` ordering/grouping identity. `None` when
/// unparseable: no digits, or a numeric-leading string with a
/// non-alphabetic prefix.
fn parse_episode_number(episode_number: Option<&str>) -> Option<(u8, u64)> {
    let epno = episode_number?.trim();
    let digits_at = epno.find(|c: char| c.is_ascii_digit())?;
    let (prefix, digits) = epno.split_at(digits_at);
    // Only an alphabetic (or empty) prefix is a recognised episode form;
    // a leading digit means prefix is empty (regular episode).
    if !prefix.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let number: u64 = digits
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .filter_map(|c| c.to_digit(10))
        .fold(0u64, |n, d| {
            n.saturating_mul(10).saturating_add(u64::from(d))
        });
    let category = match prefix.to_ascii_uppercase().as_str() {
        "" => 0,
        "S" => 1,
        "C" => 2,
        "T" => 3,
        "P" => 4,
        _ => 5,
    };
    Some((category, number))
}

/// Sort key for ordering episodes within a season.
///
/// Topological by AniDB episode number when known: regular episodes (numeric,
/// no prefix) first in numeric order, then specials (`S`), credits (`C`),
/// trailers (`T`), parodies/promos (`P`), then anything else -- each group in
/// numeric order. Episodes with no parseable number sort after the numbered
/// ones, ordered by a natural-order parse of the display label (so "ep 2"
/// precedes "ep 10").
pub fn episode_sort_key(episode_number: Option<&str>, label: &str) -> EpisodeSortKey {
    let fallback = natural_tokens(label);
    match parse_episode_number(episode_number) {
        Some((category, number)) => EpisodeSortKey {
            unnumbered: false,
            category,
            number,
            fallback,
        },
        None => EpisodeSortKey {
            unnumbered: true,
            category: 0,
            number: 0,
            fallback,
        },
    }
}

/// One known file for an episode: its display filename, holders (users
/// advertising [`FileAvailability::Ready`] for it), and whether it counts
/// as watched for muting.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct EpisodeCopy {
    /// The file's hash — what `Enter`/mark-watched act on.
    pub hash: Ed2kHash,
    /// Best-effort display name (see [`episode_label`]).
    pub filename: String,
    /// Users advertising [`FileAvailability::Ready`] for this hash,
    /// sorted.
    pub holders: Vec<UserId>,
    /// Group watched flag, or personal 85%-history — either counts.
    pub watched: bool,
}

/// One row in the episode browser's file list (design.md #31): most
/// episodes have exactly one known copy and render as a single line;
/// when several files share the same real AniDB episode number they
/// expand into a header plus one child per copy, so "which copy, who has
/// it" is visible instead of several identical-looking lines.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EpisodeRow {
    /// The only known copy of this episode. `episode` is the AniDB-derived
    /// label ("Episode 03") when the file has a real episode number tying
    /// it to potential siblings; `None` when it doesn't (the filename is
    /// already the whole story, and there's no evidence any other file is
    /// the same episode).
    Single {
        /// `Some("Episode 03")` when numbered.
        episode: Option<String>,
        /// The file.
        copy: EpisodeCopy,
    },
    /// Display-only grouping line for a multi-copy episode — not
    /// selectable, just names the group. `watched` is true when *any*
    /// copy underneath is: one watched encoding means the group saw the
    /// episode (user decision 2026-08-17).
    Header {
        /// "Episode 03".
        episode: String,
        /// Whether every copy in the group is watched.
        watched: bool,
    },
    /// One copy of a multi-copy episode, indented under its [`Header`].
    Child(EpisodeCopy),
}

impl EpisodeRow {
    /// The hash to act on (add to playlist, mark watched) — `None` for a
    /// [`Header`](EpisodeRow::Header), which exists only to name the
    /// group and carries no file of its own.
    pub fn hash(&self) -> Option<Ed2kHash> {
        match self {
            EpisodeRow::Single { copy, .. } | EpisodeRow::Child(copy) => Some(copy.hash),
            EpisodeRow::Header { .. } => None,
        }
    }

    /// Whether this row (and everything under it) is watched, for muting.
    pub fn watched(&self) -> bool {
        match self {
            EpisodeRow::Single { copy, .. } | EpisodeRow::Child(copy) => copy.watched,
            EpisodeRow::Header { watched, .. } => *watched,
        }
    }
}

/// Users advertising [`FileAvailability::Ready`] for `hash` (design.md
/// #31: the episode browser's per-copy "who has it" list). Sorted for a
/// stable display order.
pub fn ready_holders(view: &StateView, hash: Ed2kHash) -> Vec<UserId> {
    let mut holders: Vec<UserId> = view
        .file_availability
        .iter()
        .filter(|((_, h), avail)| *h == hash && **avail == FileAvailability::Ready)
        .map(|((user, _), _)| user.clone())
        .collect();
    holders.sort();
    holders
}

/// Group and order a season's known files into episode rows (design.md
/// #31/#11).
///
/// Files are sorted by [`episode_sort_key`], then adjacent files sharing
/// the same real, parsed AniDB episode number collapse into one row (or a
/// header + children when there's more than one copy). Files with no
/// parseable episode number never merge with each other, even if
/// adjacent — there is no evidence any two of them are the same episode,
/// so each stays its own singleton group.
///
/// `personally_watched` is the local 85%-history hash set (design.md,
/// Watch Tracking); a copy is muted when the group watched flag *or*
/// personal history says watched, matching the playlist pane's
/// convention.
pub fn episode_rows(
    view: &StateView,
    hashes: &[Ed2kHash],
    personally_watched: &BTreeMap<Ed2kHash, i64>,
) -> Vec<EpisodeRow> {
    struct Entry {
        hash: Ed2kHash,
        label: String,
        episode_number: Option<String>,
        key: Option<(u8, u64)>,
        sort_key: EpisodeSortKey,
    }
    let mut entries: Vec<Entry> = hashes
        .iter()
        .map(|&hash| {
            let label = episode_label(view, &hash);
            let episode_number = view
                .anidb_metadata
                .get(&hash)
                .and_then(|m| m.as_ref())
                .and_then(|m| m.episode_number.clone());
            Entry {
                hash,
                key: parse_episode_number(episode_number.as_deref()),
                sort_key: episode_sort_key(episode_number.as_deref(), &label),
                episode_number,
                label,
            }
        })
        .collect();
    entries.sort_by(|a, b| a.sort_key.cmp(&b.sort_key));

    // Group adjacent entries sharing the same parsed key; an unparseable
    // (`None`) key always starts its own singleton group. `episode` is
    // computed once here so the flat_map below never needs to re-derive
    // "is this really a shared-key group" from `Option` alone.
    struct Group {
        episode: Option<String>,
        members: Vec<Entry>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for entry in entries {
        if let Some(key) = entry.key
            && let Some(last) = groups.last_mut()
            && last
                .members
                .last()
                .is_some_and(|prev| prev.key == Some(key))
        {
            last.members.push(entry);
            continue;
        }
        let episode = entry
            .key
            .and(entry.episode_number.as_deref())
            .map(|epno| format!("Episode {epno}"));
        groups.push(Group {
            episode,
            members: vec![entry],
        });
    }

    let copy_of = |entry: &Entry| EpisodeCopy {
        hash: entry.hash,
        filename: entry.label.clone(),
        holders: ready_holders(view, entry.hash),
        watched: view.watched.get(&entry.hash) == Some(&true)
            || personally_watched.contains_key(&entry.hash),
    };

    groups
        .into_iter()
        .flat_map(|group| {
            if let [entry] = group.members.as_slice() {
                vec![EpisodeRow::Single {
                    episode: group.episode,
                    copy: copy_of(entry),
                }]
            } else {
                let copies: Vec<EpisodeCopy> = group.members.iter().map(copy_of).collect();
                // Any watched copy mutes the episode: the group saw it,
                // whichever encoding carried it. Copies keep their own
                // per-file marks.
                let watched = copies.iter().any(|copy| copy.watched);
                // A multi-member group only ever forms from a shared
                // `Some` key (the loop above never merges `None`-keyed
                // entries), so `episode` is always `Some` here;
                // `unwrap_or_default` keeps this total rather than
                // panicking on that invariant.
                let mut rows = vec![EpisodeRow::Header {
                    episode: group.episode.unwrap_or_default(),
                    watched,
                }];
                rows.extend(copies.into_iter().map(EpisodeRow::Child));
                rows
            }
        })
        .collect()
}

/// The index of the first unwatched row (design.md #11: the browser's
/// `<` marker and initial cursor placement). `None` when everything is
/// watched. An unwatched *copy* under a watched header doesn't count —
/// the episode was seen through another encoding, and the marker means
/// "continue here", never "here's a leftover duplicate".
pub fn first_unwatched(rows: &[EpisodeRow]) -> Option<usize> {
    let mut header_watched = false;
    rows.iter().position(|row| match row {
        EpisodeRow::Header { watched, .. } => {
            header_watched = *watched;
            !watched
        }
        EpisodeRow::Child(copy) => !header_watched && !copy.watched,
        EpisodeRow::Single { copy, .. } => !copy.watched,
    })
}

/// The row the browser's cursor opens on: the first unwatched row
/// ([`first_unwatched`]; row 0 when everything is watched or nothing is
/// known), refined for a multi-copy episode. A `Header` names several
/// files and can't be chosen with Enter, so when the previous episode
/// was actually played from a known copy, the cursor lands on the child
/// whose filename is nearest (Levenshtein) to that copy's — the same
/// release group, resolution and subtitle track the group has been
/// following. "Actually played" is the personal watch history (85%
/// rule, `personal_watched`, newest record wins) or, failing that, the
/// copy sitting in the group playlist — never a bare watched *flag*,
/// which `w` can set on any copy. No such evidence: the header, and
/// the user picks.
pub fn opening_row(
    rows: &[EpisodeRow],
    personal_watched: &BTreeMap<Ed2kHash, i64>,
    view: &StateView,
) -> usize {
    let Some(first) = first_unwatched(rows) else {
        return 0;
    };
    if !matches!(rows[first], EpisodeRow::Header { .. }) {
        return first;
    }
    let Some(reference) = previous_episode_copies(rows, first)
        .into_iter()
        .filter_map(|copy| {
            // Personal history outranks playlist presence outright; the
            // playlist tie-breaks among unrecorded copies by hash only,
            // so the choice is deterministic across redraws.
            let rank = match personal_watched.get(&copy.hash) {
                Some(at) => (1, *at),
                None if view.playlist.iter().any(|entry| entry.hash == copy.hash) => (0, 0),
                None => return None,
            };
            Some((rank, copy))
        })
        .max_by_key(|(rank, copy)| (*rank, std::cmp::Reverse(copy.hash)))
        .map(|(_, copy)| copy)
    else {
        return first;
    };
    rows[first + 1..]
        .iter()
        .take_while(|row| matches!(row, EpisodeRow::Child(_)))
        .enumerate()
        .filter_map(|(offset, row)| match row {
            EpisodeRow::Child(copy) => Some((
                strsim::levenshtein(&copy.filename, &reference.filename),
                first + 1 + offset,
            )),
            _ => None,
        })
        .min()
        .map_or(first, |(_, index)| index)
}

/// The copies of the episode immediately preceding row `index` (which
/// must start an episode: a `Header` or a `Single`): a `Single` on its
/// own, or the child run under the previous `Header`. Empty at the top
/// of the list.
fn previous_episode_copies(rows: &[EpisodeRow], index: usize) -> Vec<&EpisodeCopy> {
    match rows[..index].last() {
        Some(EpisodeRow::Single { copy, .. }) => vec![copy],
        Some(EpisodeRow::Child(_)) => rows[..index]
            .iter()
            .rev()
            .map_while(|row| match row {
                EpisodeRow::Child(copy) => Some(copy),
                _ => None,
            })
            .collect(),
        Some(EpisodeRow::Header { .. }) | None => vec![],
    }
}

/// The row starting the episode after the one containing row `index`:
/// the next `Header` or `Single` past the current episode's copies.
/// `None` when `index` is in the last episode. Where the cursor lands
/// after `w` marks an episode watched (the natural next thing to mark,
/// or to choose).
pub fn next_episode_row(rows: &[EpisodeRow], index: usize) -> Option<usize> {
    rows.iter()
        .enumerate()
        .skip(index + 1)
        .find(|(_, row)| !matches!(row, EpisodeRow::Child(_)))
        .map(|(i, _)| i)
}

/// How far (Levenshtein distance) a file's derived series name may sit
/// from a List entry's `name`/`local_aliases` and still be considered a
/// plausible candidate for its next episode (design.md, Advancing
/// next_ep). Loose enough to catch a differently-hinted file -- the whole
/// reason Series Identity exists -- but tight enough that unrelated
/// library files don't flood the list.
const CANDIDATE_NAME_DISTANCE_THRESHOLD: usize = 6;

/// Rank library files as candidates for `entry`'s next episode (design.md,
/// Advancing next_ep): for a series with no AniDB episode identity to
/// match against, finding the right file is a heuristic, not a lookup.
/// Files already queued are excluded (nothing left to disambiguate for
/// them). Candidates are ranked, best first: an explicit `manual_files`
/// override always wins; then a file whose own name parses to the
/// expected episode number ([`episode_parse::parse_episode_number`]
/// against `next_ep`); then by ascending edit distance from the file's
/// derived name to `entry`'s `name`/`local_aliases`. A file whose name
/// distance exceeds [`CANDIDATE_NAME_DISTANCE_THRESHOLD`] (and isn't a
/// manual override) is dropped, not just ranked low.
///
/// Returns a `Header` (design.md #31's grouping line, generalized from
/// "several files, one confirmed episode" to "several candidates, no
/// confirmed identity") followed by one `Child` per candidate, in rank
/// order -- reusing the Episode Browser's existing tree UI outright.
/// Empty when nothing clears the bar; callers should fall back to the
/// plain edit form when there's nothing to disambiguate.
pub fn candidate_rows(
    view: &StateView,
    entry: &SeriesListEntry,
    next_ep: Option<&NextEpState>,
) -> Vec<EpisodeRow> {
    let expected_episode: Option<u32> = next_ep
        .and_then(|n| n.next_ep.as_deref())
        .and_then(|s| s.trim().parse().ok());
    let queued: BTreeSet<Ed2kHash> = view.playlist.iter().map(|e| e.hash).collect();

    let mut candidates: Vec<((bool, u8, usize, String), EpisodeCopy)> = view
        .anidb_metadata
        .iter()
        .filter_map(|(hash, metadata)| {
            if queued.contains(hash) {
                return None;
            }
            let metadata = metadata.as_ref()?;
            let is_manual = entry.manual_files.contains(hash);
            let name_distance = if is_manual {
                0
            } else {
                let to_name = strsim::levenshtein(&metadata.series_name, &entry.name);
                let to_alias = entry
                    .local_aliases
                    .iter()
                    .map(|alias| strsim::levenshtein(&metadata.series_name, alias))
                    .min()
                    .unwrap_or(usize::MAX);
                to_name.min(to_alias)
            };
            if !is_manual && name_distance > CANDIDATE_NAME_DISTANCE_THRESHOLD {
                return None;
            }
            let parsed_episode = view
                .file_catalog
                .get(hash)
                .and_then(|c| dessplay_core::episode_parse::parse_episode_number(&c.filename))
                .and_then(|s| s.parse::<u32>().ok());
            let episode_rank: u8 = match (parsed_episode, expected_episode) {
                (Some(p), Some(e)) if p == e => 0,
                (Some(_), _) => 1,
                (None, _) => 2,
            };
            let filename = episode_label(view, hash);
            let key = (!is_manual, episode_rank, name_distance, filename.clone());
            Some((
                key,
                EpisodeCopy {
                    hash: *hash,
                    filename,
                    holders: ready_holders(view, *hash),
                    watched: view.watched.get(hash).copied().unwrap_or(false),
                },
            ))
        })
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    candidates.sort_by(|(a, _), (b, _)| a.cmp(b));

    let header = EpisodeRow::Header {
        episode: match expected_episode {
            Some(n) => format!("Next episode ({n}?)"),
            None => "Next episode?".to_string(),
        },
        watched: false,
    };
    std::iter::once(header)
        .chain(
            candidates
                .into_iter()
                .map(|(_, copy)| EpisodeRow::Child(copy)),
        )
        .collect()
}

/// The episode browser's season order for a franchise (proposal
/// 2026-08-28): the main line in prequel-chain order, with each side
/// branch — a member attached to a main-line season by a non-chain
/// structural edge (SideStory / Summary / AlternativeVersion, or the
/// reverse ParentStory / FullStory) — indented one level directly under
/// the season it attaches to. A branch that is itself a chain (shorts
/// with their own seasons) stays together, in its own chain order.
/// Members AniDB chains as Sequel/Prequel — OVAs included — are main
/// line: that is how the group watched them. Only `franchise.series`
/// (members with known files) are placed.
///
/// Returns `(series, depth)` with depth 0 for the main line, 1 for a
/// branch. Deterministic: chains order by their root's year then id,
/// then season ordinal; branches under one parent by ordinal then id.
pub fn season_tree(view: &StateView, franchise: &franchise::Franchise) -> Vec<(AniDbSeriesId, u8)> {
    use dessplay_core::types::RelationKind;
    let members: BTreeSet<AniDbSeriesId> = franchise.series.iter().copied().collect();
    // Walk to the prequel-chain root (lowest-id prequel each step,
    // cycle-guarded — the same walk as `season_ordinal`).
    let chain_root = |series: AniDbSeriesId| {
        let mut seen = BTreeSet::from([series]);
        let mut current = series;
        while let Some(relations) = view.series_relations.get(&current) {
            let Some(prequel) = relations
                .relations
                .iter()
                .filter(|r| r.kind == RelationKind::Prequel)
                .map(|r| r.target)
                .min()
            else {
                break;
            };
            if !seen.insert(prequel) {
                break;
            }
            current = prequel;
        }
        current
    };
    // The main-line member a chain root hangs off, if any: the root
    // names a parent (ParentStory/FullStory), or a member names the
    // root as its SideStory/Summary/AlternativeVersion.
    let attach_parent = |root: AniDbSeriesId| -> Option<AniDbSeriesId> {
        let up = view
            .series_relations
            .get(&root)
            .into_iter()
            .flat_map(|r| r.relations.iter())
            .filter(|r| matches!(r.kind, RelationKind::ParentStory | RelationKind::FullStory))
            .map(|r| r.target);
        let down = members.iter().copied().filter(|member| {
            view.series_relations.get(member).is_some_and(|r| {
                r.relations.iter().any(|r| {
                    r.target == root
                        && matches!(
                            r.kind,
                            RelationKind::SideStory
                                | RelationKind::Summary
                                | RelationKind::AlternativeVersion
                        )
                })
            })
        });
        up.chain(down)
            .filter(|parent| *parent != root && members.contains(parent))
            .min()
    };
    let year = |series: AniDbSeriesId| {
        view.series_relations
            .get(&series)
            .and_then(|r| r.year)
            .unwrap_or(u16::MAX)
    };

    // (parent-or-none, chain root, ordinal, id) per member.
    let mut main: Vec<(u16, AniDbSeriesId, u32, AniDbSeriesId)> = Vec::new();
    let mut branches: BTreeMap<AniDbSeriesId, Vec<(u32, AniDbSeriesId)>> = BTreeMap::new();
    for member in &members {
        let root = chain_root(*member);
        let ordinal = franchise::season_ordinal(view, *member);
        match attach_parent(root) {
            // A branch under a parent that is itself a branch would
            // need a third level; flatten it under the same parent.
            Some(parent) if parent != *member => {
                branches.entry(parent).or_default().push((ordinal, *member));
            }
            _ => main.push((year(root), root, ordinal, *member)),
        }
    }
    // A branch whose parent turned out to be a branch itself (its own
    // root attached elsewhere) still needs a main-line anchor: promote
    // it to the main line rather than lose it.
    let main_ids: BTreeSet<AniDbSeriesId> = main.iter().map(|(.., id)| *id).collect();
    for (parent, children) in std::mem::take(&mut branches) {
        if main_ids.contains(&parent) {
            branches.insert(parent, children);
        } else {
            for (ordinal, member) in children {
                main.push((
                    year(chain_root(member)),
                    chain_root(member),
                    ordinal,
                    member,
                ));
            }
        }
    }
    main.sort();
    let mut out = Vec::with_capacity(members.len());
    for (.., member) in main {
        out.push((member, 0));
        if let Some(mut children) = branches.remove(&member) {
            children.sort();
            out.extend(children.into_iter().map(|(_, child)| (child, 1)));
        }
    }
    out
}

/// The `next_ep` display text: `SnEnn` for a linked entry whose free
/// text parses as a plain episode number, verbatim for everything else
/// ("S3-05", "Sisters", "movie 5?", unlinked entries).
fn next_ep_display(view: &StateView, entry: &SeriesListEntry, next_ep: &str) -> String {
    let (Some(series), Ok(episode)) = (entry.anidb_series_id, next_ep.trim().parse::<u32>()) else {
        return next_ep.to_string();
    };
    format!("S{}E{episode:02}", franchise::season_ordinal(view, series))
}

/// Entries with something to watch tonight: a file some client still
/// advertises (Ready or downloading — the availability map, which
/// survives compaction) that is unwatched by **both** durable records —
/// the group watched flag *and* personal watch history — the episode
/// browser's muting rule (design.md #11). The group flag alone is not
/// enough: compaction drops flags for files off the playlist while
/// metadata rows persist forever, which would resurrect every
/// long-finished episode as "unwatched". Files are resolved to entries
/// by the canonical Series Identity order
/// ([`series_identity::resolve_series_entry_for_file`]).
///
/// Iterating held files (a few hundred availability rows) rather than
/// the whole metadata map (tens of thousands) also keeps this cheap
/// enough for the per-snapshot refresh.
fn entries_with_unwatched_files(
    view: &StateView,
    watched_hashes: &BTreeMap<Ed2kHash, i64>,
) -> BTreeSet<ListEntryId> {
    // Episode identities watched through *any* copy: a duplicate
    // encoding of an episode the group has seen is not "something to
    // watch" (the browser's any-copy muting rule). Identity needs an
    // AniDB link; filename-derived files fall back to per-file marks.
    let watched_episodes: BTreeSet<(AniDbSeriesId, &str)> = view
        .watched
        .iter()
        .filter(|(_, watched)| **watched)
        .map(|(hash, _)| hash)
        .chain(watched_hashes.keys())
        .filter_map(|hash| {
            let metadata = view.anidb_metadata.get(hash)?.as_ref()?;
            Some((metadata.series_id?, metadata.episode_number.as_deref()?))
        })
        .collect();
    let episode_watched = |hash: &Ed2kHash| {
        let Some(Some(metadata)) = view.anidb_metadata.get(hash) else {
            return false;
        };
        let (Some(series), Some(episode)) =
            (metadata.series_id, metadata.episode_number.as_deref())
        else {
            return false;
        };
        watched_episodes.contains(&(series, episode))
    };

    let held: BTreeSet<Ed2kHash> = view
        .file_availability
        .iter()
        .filter(|(_, availability)| !matches!(availability, FileAvailability::Missing))
        .map(|((_, hash), _)| *hash)
        .collect();
    // One resolution index for the whole walk: resolving each held file
    // by the scanning resolver would make this O(held × entries).
    let index = series_identity::SeriesEntryIndex::new(view);
    held.into_iter()
        .filter(|hash| !view.watched.get(hash).copied().unwrap_or(false))
        .filter(|hash| !watched_hashes.contains_key(hash))
        .filter(|hash| !episode_watched(hash))
        .filter_map(|hash| index.resolve(view, hash))
        .collect()
}

/// The List, grouped per design (design.md, The List: UI Integration):
/// one "Watching — ⟨user⟩" group per committed user (`me` first, the
/// rest of `users` alphabetically), a residual shared "Watching" group
/// for Watching-tier entries no rendered group claims, then ShortList,
/// Planned, Waiting, Hiatus, and a collapsed Finished / Dropped tail.
///
/// `users` is the candidate group set (peers + known-offline); `recency`
/// is the local watch history ([`watch_recency`]) — the closest thing to
/// a "when did the group last watch this" timestamp, since group watched
/// flags are plain booleans. Empty groups are dropped.
pub fn list_groups(
    view: &StateView,
    me: &UserId,
    users: &[UserId],
    sort: ListSort,
    recency: &BTreeMap<SeriesKey, u64>,
    watched_hashes: &BTreeMap<Ed2kHash, i64>,
) -> Vec<ListGroup> {
    // Live commitment: who has each entry as Watching, from the resolved
    // series_preference map (absent = Maybe, which never groups).
    let mut committed: BTreeMap<ListEntryId, BTreeSet<&UserId>> = BTreeMap::new();
    for ((user, entry), pref) in &view.series_preference {
        if pref.state == SeriesWatchState::Watching {
            committed.entry(*entry).or_default().insert(user);
        }
    }
    let unwatched = entries_with_unwatched_files(view, watched_hashes);

    // One row per franchise (proposal 2026-08-28): linked entries group by
    // their franchise component, unlinked entries stand alone. The
    // *canonical* entry (`series_identity::canonical_first`) speaks for
    // the row — name, status, next_ep, the edit/Enter target — while
    // commitment, availability and recency aggregate over every member
    // entry and every season in the component.
    let components = franchise::series_components(view);
    let mut seasons_by_root: BTreeMap<AniDbSeriesId, Vec<AniDbSeriesId>> = BTreeMap::new();
    for (series, root) in &components {
        seasons_by_root.entry(*root).or_default().push(*series);
    }
    #[derive(PartialEq, Eq, PartialOrd, Ord)]
    enum RowKey {
        Franchise(AniDbSeriesId),
        Entry(ListEntryId),
    }
    let mut rows_by_key: BTreeMap<RowKey, Vec<ListEntryId>> = BTreeMap::new();
    for (id, entry) in &view.list_entries {
        let key = match entry.anidb_series_id {
            Some(series) => RowKey::Franchise(components.get(&series).copied().unwrap_or(series)),
            None => RowKey::Entry(*id),
        };
        rows_by_key.entry(key).or_default().push(*id);
    }

    // `me` first, the rest alphabetically, deduped.
    let mut ordered_users: Vec<&UserId> = vec![me];
    let mut rest: Vec<&UserId> = users.iter().filter(|user| *user != me).collect();
    rest.sort();
    rest.dedup();
    ordered_users.extend(rest);

    // (heading, statuses, collapsed); per-user groups match on the
    // Watching-tier statuses and additionally require commitment below.
    let watching_tier = [ListStatus::CurrentSeason, ListStatus::Active];
    let mut groups: Vec<(ListGroup, Vec<ListStatus>, Option<&UserId>)> = ordered_users
        .iter()
        .map(|user| {
            (
                format!("Watching — {user}"),
                watching_tier.to_vec(),
                Some(*user),
            )
        })
        .chain([
            ("Watching".to_string(), watching_tier.to_vec(), None),
            ("Short List".to_string(), vec![ListStatus::ShortList], None),
            ("Planned".to_string(), vec![ListStatus::Planned], None),
            ("Waiting".to_string(), vec![ListStatus::Waiting], None),
            ("Hiatus".to_string(), vec![ListStatus::Hiatus], None),
            (
                "Finished / Dropped".to_string(),
                vec![ListStatus::Finished, ListStatus::Dropped],
                None,
            ),
        ])
        .map(|(heading, statuses, user)| {
            let collapsed = heading == "Finished / Dropped";
            (
                ListGroup {
                    heading,
                    rows: Vec::new(),
                    collapsed,
                },
                statuses,
                user,
            )
        })
        .collect();
    // The residual shared Watching group: index of the `None`-user
    // Watching-tier group, for entries no per-user group claimed.
    let residual = ordered_users.len();

    // Sort keys computed alongside each row: the newest watch drives the
    // Recency order but doesn't belong in the rendered row.
    let mut keyed: Vec<Vec<(Option<u64>, usize)>> = vec![Vec::new(); groups.len()];
    let mut rows: Vec<ListRow> = Vec::new();
    for (key, mut members) in rows_by_key {
        series_identity::canonical_first(view, &mut members);
        let id = members[0];
        let entry = &view.list_entries[&id];
        let next = view.list_next_ep.get(&id);
        let available = members
            .iter()
            .any(|member| view.list_next_ep.get(member).is_some_and(|n| n.available));
        let dimmed = !available && !members.iter().any(|member| unwatched.contains(member));
        // Recency: the newest local watch of any member entry's identity
        // — or of *any season in the franchise*, linked entry or not, so
        // "the latest episode is in season three" floats the row.
        let seasons = match key {
            RowKey::Franchise(root) => seasons_by_root.get(&root).cloned().unwrap_or_default(),
            RowKey::Entry(_) => Vec::new(),
        };
        let last_watched = members
            .iter()
            .map(|member| &view.list_entries[member])
            .flat_map(|entry| {
                entry
                    .anidb_series_id
                    .map(SeriesKey::AniDb)
                    .into_iter()
                    .chain(std::iter::once(SeriesKey::Name(entry.name.clone())))
                    .chain(
                        entry
                            .local_aliases
                            .iter()
                            .map(|alias| SeriesKey::Name(alias.clone())),
                    )
            })
            .chain(seasons.iter().copied().map(SeriesKey::AniDb))
            .filter_map(|key| recency.get(&key).copied())
            .max();
        let watchers: BTreeSet<&UserId> = members
            .iter()
            .filter_map(|member| committed.get(member))
            .flatten()
            .copied()
            .collect();
        // A linked entry with a curated community short title displays
        // (and alphabetizes) under it — "GochiUsa", not "Gochuumon wa
        // Usagi Desu ka??" (design.md, The List). Only over the
        // auto-seeded name, though: an entry whose name differs from
        // the official title was named by a human, and a human name
        // always beats the curator (user decision 2026-08-18 — this is
        // also the fix-it path when the AI picks a clunker). The full
        // name still lives in the edit modal and the episode browser.
        let display_name = entry
            .anidb_series_id
            .and_then(|series| view.series_relations.get(&series))
            .filter(|relations| relations.title == entry.name)
            .and_then(|relations| relations.short_titles.first())
            .cloned()
            .unwrap_or_else(|| entry.name.clone());
        let row = ListRow {
            id,
            name: display_name,
            nero_name: entry.nero_name.clone(),
            next_ep: next
                .and_then(|n| n.next_ep.as_deref())
                .map(|text| next_ep_display(view, entry, text)),
            available,
            watchers: watchers
                .iter()
                .filter_map(|user| user.0.chars().next())
                .map(|c| c.to_ascii_uppercase())
                .collect(),
            series_id: entry.anidb_series_id,
            anidb_unavailable: entry.anidb_unavailable,
            dimmed,
        };
        let row_index = rows.len();
        rows.push(row);

        let mut placed = false;
        for (g, (_, statuses, user)) in groups.iter().enumerate() {
            if !statuses.contains(&entry.status) {
                continue;
            }
            match user {
                // Per-user group: needs that user's live commitment to any
                // member. A row lands in every group whose user is
                // committed.
                Some(user) => {
                    if watchers.contains(*user) {
                        keyed[g].push((last_watched, row_index));
                        placed = true;
                    }
                }
                // Shared status groups take every row of their status;
                // the residual Watching group only what nobody claimed
                // (below), so nothing vanishes — a commitment by a user
                // outside `users` still isn't a rendered group.
                None if g != residual => {
                    keyed[g].push((last_watched, row_index));
                }
                None => {}
            }
        }
        if matches!(entry.status, ListStatus::CurrentSeason | ListStatus::Active) && !placed {
            keyed[residual].push((last_watched, row_index));
        }
    }

    for (g, (group, ..)) in groups.iter_mut().enumerate() {
        let mut members = std::mem::take(&mut keyed[g]);
        match sort {
            ListSort::Alphabetical => {
                members.sort_by(|(_, a), (_, b)| rows[*a].name.cmp(&rows[*b].name));
            }
            // Most recently watched first (never-watched last), name as
            // the tiebreak. Dimming is purely visual and never reorders
            // (proposal 2026-08-28): a predictable order beats a
            // partition that shuffles rows as files come and go.
            ListSort::Recency => members.sort_by(|(ra, a), (rb, b)| {
                rb.cmp(ra).then_with(|| rows[*a].name.cmp(&rows[*b].name))
            }),
        }
        group.rows = members
            .into_iter()
            .map(|(_, index)| rows[index].clone())
            .collect();
    }
    groups
        .into_iter()
        .map(|(group, ..)| group)
        .filter(|group| !group.rows.is_empty())
        .collect()
}

/// Memoizes [`list_groups`] — the derivation behind the Series pane's
/// *default* mode — exactly as [`franchise::FranchiseCache`] memoizes the
/// franchise grouping (whose uncached rebuild was ~1/3 of normal-play
/// CPU). Snapshots arrive at ~10 Hz during playback; the derivation is
/// O(held files × entries) computed fresh, while position ticks change
/// none of its inputs — so they must hit the cache. The grouping is
/// reachable only through [`ListGroupsCache::get`], so a caller cannot
/// read a stale grouping without the freshness check running. Guarded by
/// the perf rig (dessplay-rendezvous/tests/perf.rs), which seeds a
/// populated List.
#[derive(Default)]
pub struct ListGroupsCache {
    /// Fingerprint of the inputs the cached grouping was built from;
    /// `None` until the first `get`.
    key: Option<u64>,
    groups: Vec<ListGroup>,
    /// Number of real recomputes; the cache-correctness tests assert
    /// unchanged inputs (position ticks included) recompute nothing.
    #[cfg(test)]
    recomputes: usize,
}

impl ListGroupsCache {
    /// The current grouping, recomputing only when an input
    /// [`list_groups`] reads has changed since the last call.
    pub fn get(
        &mut self,
        view: &StateView,
        me: &UserId,
        users: &[UserId],
        sort: ListSort,
        recency: &BTreeMap<SeriesKey, u64>,
        watched_hashes: &BTreeMap<Ed2kHash, i64>,
    ) -> &[ListGroup] {
        let key = list_inputs_fingerprint(view, me, users, sort, recency, watched_hashes);
        if self.key != Some(key) {
            self.key = Some(key);
            #[cfg(test)]
            {
                self.recomputes += 1;
            }
            self.groups = list_groups(view, me, users, sort, recency, watched_hashes);
        }
        &self.groups
    }
}

/// Fingerprint every input [`list_groups`] (and its helpers) reads:
/// the List maps (`list_entries`, `list_next_ep`), `series_preference`
/// (live commitment), the group `watched` flags, the metadata/relations
/// maps (file→entry resolution, `SnEnn`/short-title display — the same
/// fingerprint the franchise cache uses), which files are *held*, the
/// local watch history (`recency` + `watched_hashes`), the user set,
/// and the sort. Resolved values, not LWW clocks, so a no-op rewrite
/// doesn't recompute; FxHasher for the same profiling reason as
/// [`franchise::metadata_relations_fingerprint`].
///
/// `file_availability` is deliberately reduced to the *held set* (any
/// non-Missing advert per file) before hashing: that bit is all the
/// derivation reads, and hashing the raw values would recompute on
/// every `Downloading` progress tick.
fn list_inputs_fingerprint(
    view: &StateView,
    me: &UserId,
    users: &[UserId],
    sort: ListSort,
    recency: &BTreeMap<SeriesKey, u64>,
    watched_hashes: &BTreeMap<Ed2kHash, i64>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = rustc_hash::FxHasher::default();
    franchise::metadata_relations_fingerprint(view).hash(&mut hasher);
    view.list_entries.hash(&mut hasher);
    view.list_next_ep.hash(&mut hasher);
    view.series_preference.hash(&mut hasher);
    view.watched.hash(&mut hasher);
    let held: BTreeSet<Ed2kHash> = view
        .file_availability
        .iter()
        .filter(|(_, availability)| !matches!(availability, FileAvailability::Missing))
        .map(|((_, hash), _)| *hash)
        .collect();
    held.hash(&mut hasher);
    recency.hash(&mut hasher);
    watched_hashes.hash(&mut hasher);
    me.hash(&mut hasher);
    users.hash(&mut hasher);
    sort.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use dessplay_core::CrdtState;
    use dessplay_core::playlist::NewPlaylistEntry;
    use dessplay_core::types::{
        ActorId, AniDbMetadata, AniDbSeriesId, ListEntryId, ListStatus, ManualState,
        MetadataSource, PlaybackIntent, RelationKind, SeriesListEntry, SeriesRelation,
        SeriesRelations, SeriesWatchState, SharedTimestamp,
    };

    const A: ActorId = ActorId::SERVER;

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    // ---- Health classification ---------------------------------------

    /// A nominal sample; tests override single fields.
    fn healthy() -> HealthSample {
        HealthSample {
            rtt_millis: Some(40),
            unanswered_probes: 0,
            server_silence_millis: 2_000,
            up_bps: 10_000,
            down_bps: 100_000,
        }
    }

    #[test]
    fn health_classification_thresholds() {
        use HealthLevel::*;
        let cases: &[(HealthSample, HealthLevel)] = &[
            (healthy(), Ok),
            // RTT boundary: 1499 fine, 1500 degraded.
            (
                HealthSample {
                    rtt_millis: Some(1_499),
                    ..healthy()
                },
                Ok,
            ),
            (
                HealthSample {
                    rtt_millis: Some(1_500),
                    ..healthy()
                },
                Degraded,
            ),
            // Unknown RTT alone is not trouble (pre-first-probe).
            (
                HealthSample {
                    rtt_millis: None,
                    ..healthy()
                },
                Ok,
            ),
            // Silence ladder: one missed StateHash within margin is Ok,
            // beyond 40s degraded, beyond 75s stalled.
            (
                HealthSample {
                    server_silence_millis: 39_000,
                    ..healthy()
                },
                Ok,
            ),
            (
                HealthSample {
                    server_silence_millis: 41_000,
                    ..healthy()
                },
                Degraded,
            ),
            (
                HealthSample {
                    server_silence_millis: 76_000,
                    ..healthy()
                },
                Stalled,
            ),
            // Lost probes: 2 degrade; 3 with 45s+ silence stall (the
            // Starlink signature); 3 with low silence only degrade.
            (
                HealthSample {
                    unanswered_probes: 2,
                    ..healthy()
                },
                Degraded,
            ),
            (
                HealthSample {
                    unanswered_probes: 3,
                    server_silence_millis: 46_000,
                    ..healthy()
                },
                Stalled,
            ),
            (
                HealthSample {
                    unanswered_probes: 3,
                    server_silence_millis: 10_000,
                    ..healthy()
                },
                Degraded,
            ),
        ];
        for (sample, expected) in cases {
            assert_eq!(
                classify_health(LinkStatus::Connected, Some(sample)),
                *expected,
                "sample {sample:?}"
            );
        }
    }

    #[test]
    fn health_is_ok_while_not_connected() {
        // The row shows the link notice instead; a stale terrible sample
        // must not color it.
        let sample = HealthSample {
            server_silence_millis: 500_000,
            unanswered_probes: 9,
            ..healthy()
        };
        for link in [LinkStatus::Down, LinkStatus::Connecting { attempt: 3 }] {
            assert_eq!(classify_health(link, Some(&sample)), HealthLevel::Ok);
        }
        assert_eq!(
            classify_health(LinkStatus::Connected, None),
            HealthLevel::Ok
        );
    }

    #[test]
    fn health_hysteresis_upgrades_fast_downgrades_slow() {
        use HealthLevel::*;
        let mut h = HealthHysteresis::default();
        assert_eq!(h.observe(Ok), Ok);
        // Trouble shows immediately.
        assert_eq!(h.observe(Stalled), Stalled);
        // Four calm samples do not clear it...
        for _ in 0..4 {
            assert_eq!(h.observe(Ok), Stalled);
        }
        // ...the fifth does.
        assert_eq!(h.observe(Ok), Ok);
        // A Degraded sample inside the calm window: the downgrade lands
        // on Degraded (the worst of the window), not straight on Ok —
        // then a further calm window clears it fully.
        assert_eq!(h.observe(Stalled), Stalled);
        for _ in 0..3 {
            assert_eq!(h.observe(Ok), Stalled);
        }
        assert_eq!(h.observe(Degraded), Stalled);
        assert_eq!(h.observe(Ok), Degraded);
        for _ in 0..4 {
            assert_eq!(h.observe(Ok), Degraded);
        }
        assert_eq!(h.observe(Ok), Ok);
    }

    #[test]
    fn health_fragments_tell_the_connected_story() {
        // Healthy: bandwidth, rtt, a static "sync ok" — all dim, no
        // probe fragment (a counting age would draw the eye for no
        // reason; alone/idle the age legitimately sawtooths toward 30s).
        let props = HealthProps {
            link: LinkStatus::Connected,
            level: HealthLevel::Ok,
            sample: Some(healthy()),
            ..HealthProps::default()
        };
        let fragments = health_fragments(&props);
        let texts: Vec<&str> = fragments.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(texts, ["▲10K ▼100K", "rtt 40ms", "sync ok"]);
        assert!(fragments.iter().all(|(_, tone)| *tone == Tone::Muted));

        // Stalled sync: the sync fragment goes red, the rest stay dim.
        let props = HealthProps {
            sample: Some(HealthSample {
                server_silence_millis: 80_000,
                unanswered_probes: 3,
                ..healthy()
            }),
            ..props
        };
        let fragments = health_fragments(&props);
        assert!(
            fragments
                .iter()
                .any(|(t, tone)| t == "sync 80s" && *tone == Tone::Blocked)
        );
        assert!(
            fragments
                .iter()
                .any(|(t, tone)| t == "3 probes lost" && *tone == Tone::Blocked)
        );
        assert!(
            fragments
                .iter()
                .any(|(t, tone)| t.starts_with("▲") && *tone == Tone::Muted)
        );
    }

    #[test]
    fn health_fragments_show_link_state_when_not_connected() {
        for (link, expected) in [
            (LinkStatus::Connecting { attempt: 2 }, "link: connecting…"),
            (LinkStatus::Down, "link: down — retrying"),
        ] {
            let props = HealthProps {
                link,
                level: HealthLevel::Ok,
                sample: Some(healthy()),
                ..HealthProps::default()
            };
            let fragments = health_fragments(&props);
            assert_eq!(fragments.len(), 1);
            assert_eq!(fragments[0].0, expected);
            assert_eq!(fragments[0].1, Tone::Paused);
        }
        // Connected but nothing measured yet.
        let props = HealthProps::default();
        let props = HealthProps {
            link: LinkStatus::Connected,
            ..props
        };
        assert_eq!(
            health_fragments(&props),
            vec![("link: measuring…".to_string(), Tone::Muted)]
        );
    }

    /// The sync field shows an age only when it is *noteworthy*: during
    /// group playback (playing + another interactive peer present) the
    /// bar is 5s, because position datagrams should be arriving
    /// constantly; alone or idle only the 30s heartbeats arrive, so the
    /// age appears only once it would warn anyway (40s+). Everything
    /// below the bar is a static, dim "sync ok".
    #[test]
    fn marquee_window_enters_crosses_and_exits() {
        let w = |offset| marquee_window("hello", 10, offset);
        assert_eq!(w(0), None, "offset 0 is fully off-screen right");
        assert_eq!(w(1), Some(("h".into(), 9)));
        assert_eq!(w(5), Some(("hello".into(), 5)));
        assert_eq!(w(10), Some(("hello".into(), 0)), "flush left");
        assert_eq!(w(12), Some(("llo".into(), 0)), "exiting");
        assert_eq!(w(14), Some(("o".into(), 0)), "last cell");
        assert_eq!(w(15), None, "fully exited at free + width");
        assert_eq!(marquee_window("hello", 0, 3), None, "no slot, no text");
    }

    #[test]
    fn marquee_window_is_display_cell_aware() {
        // "日本" is 4 display cells wide.
        assert_eq!(marquee_window("日本", 6, 2), Some(("日".into(), 4)));
        // A wide char that does not fit the entering window yet renders
        // nothing (blank cells), not a half char.
        assert_eq!(marquee_window("日本", 6, 1), Some(("".into(), 5)));
        // Exiting with the cut straddling a wide char: the straddled
        // char drops and its residual cell becomes padding.
        assert_eq!(marquee_window("日本", 6, 7), Some(("本".into(), 1)));
        assert_eq!(marquee_window("日本", 6, 10), None);
    }

    #[test]
    fn sync_age_shows_only_when_noteworthy() {
        let with = |silence: u64, playing: bool, company: bool| {
            let props = HealthProps {
                link: LinkStatus::Connected,
                level: HealthLevel::Ok,
                sample: Some(HealthSample {
                    server_silence_millis: silence,
                    ..healthy()
                }),
                playing,
                company,
                ..HealthProps::default()
            };
            health_fragments(&props)
                .into_iter()
                .find(|(t, _)| t.starts_with("sync"))
                .expect("a sync fragment")
        };
        // Group playback: 6s of silence is news (but still dim — the
        // warning thresholds are unchanged); 3s is not.
        assert_eq!(with(6_000, true, true), ("sync 6s".into(), Tone::Muted));
        assert_eq!(with(3_000, true, true), ("sync ok".into(), Tone::Muted));
        // Alone (even during playback) or idle-with-company: the same 6s
        // is normal heartbeat spacing.
        assert_eq!(with(6_000, true, false), ("sync ok".into(), Tone::Muted));
        assert_eq!(with(6_000, false, true), ("sync ok".into(), Tone::Muted));
        // The 20-ish-second sawtooth of a solo session stays "ok"...
        assert_eq!(with(25_000, false, false), ("sync ok".into(), Tone::Muted));
        // ...and past the warning threshold the age appears with its
        // color, regardless of context.
        assert_eq!(
            with(41_000, false, false),
            ("sync 41s".into(), Tone::Paused)
        );
        assert_eq!(
            with(80_000, false, false),
            ("sync 80s".into(), Tone::Blocked)
        );
    }

    #[test]
    fn rates_format_compactly() {
        assert_eq!(fmt_rate(0), "0B");
        assert_eq!(fmt_rate(999), "999B");
        assert_eq!(fmt_rate(1_000), "1K");
        assert_eq!(fmt_rate(340_000), "340K");
        assert_eq!(fmt_rate(1_200_000), "1.2M");
        assert_eq!(fmt_rate(250_000_000), "250.0M");
    }

    /// Link a List entry to `series` so preference writes/gating resolve
    /// through it (design.md, Series Identity).
    fn link_series(
        state: &mut CrdtState,
        ts: SharedTimestamp,
        series: AniDbSeriesId,
    ) -> ListEntryId {
        let id = ListEntryId(series.0 as u128);
        state.put_list_entry(
            A,
            ts,
            id,
            SeriesListEntry {
                name: "Show".into(),
                nero_name: None,
                genre: None,
                notes: Vec::new(),
                recommender: None,
                status: ListStatus::Active,
                status_note: None,
                source: None,
                watchers: Default::default(),
                anidb_series_id: Some(series),
                local_aliases: Default::default(),
                manual_files: Default::default(),
                anidb_unavailable: false,
            },
        );
        id
    }

    #[test]
    fn chat_lines_decode_actions() {
        let mut state = CrdtState::new();
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: ts(1),
            sender: UserId::new("baughn"),
            text: "hello".to_string(),
        });
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: ts(2),
            sender: UserId::new("baughn"),
            text: dessplay_core::types::encode_action("waves"),
        });
        let lines = chat_lines(&state.view());
        assert_eq!((lines[0].text.as_str(), lines[0].action), ("hello", false));
        assert_eq!((lines[1].text.as_str(), lines[1].action), ("waves", true));
    }

    /// Regression: a hostile or malformed remote message must not write
    /// raw escape bytes into the terminal — ratatui passes cell symbols
    /// through. Control characters are stripped at the display boundary
    /// (never from synced state).
    #[test]
    fn chat_lines_strip_control_characters() {
        let mut state = CrdtState::new();
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: ts(1),
            sender: UserId::new("baughn"),
            text: "evil\x1b[2J\x07text".to_string(),
        });
        let lines = chat_lines(&state.view());
        assert_eq!(lines[0].text, "evil[2Jtext");
    }

    /// The same boundary for the IRC bridge's local-only lines.
    #[test]
    fn irc_lines_strip_control_characters() {
        let line = irc_line(0, "nick".into(), "hi\x1b[31m\rthere".into(), false);
        assert_eq!(line.text, "hi[31mthere");
    }

    #[test]
    fn subtitle_text_prefixes_only_named_opted_in_cues() {
        // `SpeakerName` is non-empty by construction, so there is no
        // empty-speaker case to test — that state is unrepresentable.
        let frieren = SpeakerName::new("Frieren");
        assert_eq!(
            subtitle_text("Hello", frieren.as_ref(), true),
            "Frieren: Hello"
        );
        assert_eq!(subtitle_text("Hello", frieren.as_ref(), false), "Hello");
        assert_eq!(subtitle_text("Hello", None, true), "Hello");
    }

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn peer(name: &str, role: Role, presence: Presence) -> PeerInfo {
        PeerInfo {
            username: UserId::new(name),
            role,
            presence,
            addresses: vec![],
            connected_since: 0,
        }
    }

    fn known_user(name: &str, last_seen: u64) -> dessplay_core::net::KnownUser {
        dessplay_core::net::KnownUser {
            username: UserId::new(name),
            last_seen,
        }
    }

    fn entry(i: u8, name: &str) -> NewPlaylistEntry {
        NewPlaylistEntry {
            hash: hash(i),
            added_by: UserId::new("kim"),
            filename: name.into(),
            size_bytes: 1,
            duration_millis: Some(1_440_000),
        }
    }

    #[test]
    fn users_pane_follows_the_color_table() {
        let mut state = CrdtState::new();
        state.set_now_playing(A, ts(1), Some(hash(1)));
        state.set_manual_override(A, ts(2), UserId::new("paused"), Some(ManualState::Paused));
        state.set_manual_override(
            A,
            ts(3),
            UserId::new("afk"),
            Some(ManualState::Away {
                set_by: UserId::new("kim"),
            }),
        );
        state.set_file_availability(
            A,
            ts(4),
            UserId::new("downloader"),
            hash(1),
            FileAvailability::Downloading {
                progress_bps: 1_500,
            },
        );
        state.set_file_availability(
            A,
            ts(5),
            UserId::new("lacking"),
            hash(1),
            FileAvailability::Missing,
        );
        state.set_file_availability(
            A,
            ts(6),
            UserId::new("buffered"),
            hash(1),
            FileAvailability::DownloadingPlayable {
                progress_bps: 3_500,
            },
        );
        let peers = [
            peer("kim", Role::Interactive, Presence::Present),
            peer("paused", Role::Interactive, Presence::Present),
            peer("afk", Role::Interactive, Presence::Present),
            peer("downloader", Role::Interactive, Presence::Present),
            peer("buffered", Role::Interactive, Presence::Present),
            peer("lacking", Role::Interactive, Presence::Present),
            peer("ghost", Role::Interactive, Presence::Lost),
            peer("gone", Role::Interactive, Presence::Departed),
            peer("nas", Role::Seeder, Presence::Present),
        ];
        let known_offline = [known_user("gone", 900)];
        let props = users_props(&state.view(), &peers, &known_offline, 1_000);

        let by_name: BTreeMap<&str, &UserRow> = props
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row))
            .collect();
        assert_eq!(by_name["kim"].tone, Tone::Good);
        // #18: manually Paused is its own (yellow) state, distinct from
        // the red blockers — a paused friend is not a missing file.
        assert_eq!(by_name["paused"].tone, Tone::Paused);
        assert_eq!(by_name["afk"].label, "away, set by kim");
        assert_eq!(by_name["afk"].tone, Tone::Idle);
        assert_eq!(by_name["downloader"].label, "downloading 15%");
        assert_eq!(by_name["downloader"].tone, Tone::Transfer);
        // A playable downloader reads green: the holder's own synced
        // verdict (the same signal gating uses), not a raw percentage.
        assert_eq!(by_name["buffered"].label, "downloading 35%");
        assert_eq!(by_name["buffered"].tone, Tone::Good);
        assert_eq!(by_name["lacking"].tone, Tone::Blocked);
        assert_eq!(by_name["ghost"].label, "lost");
        assert_eq!(
            props
                .known_offline
                .iter()
                .map(|row| row.name.as_str())
                .collect::<Vec<_>>(),
            vec!["gone"]
        );
        assert_eq!(props.seeders, vec!["nas"]);
    }

    #[test]
    fn users_pane_surfaces_committed_absent_blockers() {
        // Regression: a committed (Watching) absent user must read as a
        // blocker in the Users pane — matching the status bar's
        // "committed, away" — not vanish onto the dim departed line. A
        // Maybe (default) absent user must not block: Departed -> dim line,
        // Lost -> greyed (a dropped connection, not something we wait on).
        let series = AniDbSeriesId(7);
        let mut state = CrdtState::new();
        state.set_now_playing(A, ts(1), Some(hash(1)));
        state.set_anidb_metadata(
            A,
            ts(2),
            hash(1),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show".into(),
                series_id: Some(series),
                episode_number: Some("1".into()),
            }),
        );
        let entry = link_series(&mut state, ts(1), series);
        for who in ["clost", "cgone"] {
            state.set_series_preference(
                A,
                ts(3),
                UserId::new(who),
                entry,
                SeriesWatchState::Watching,
                None,
            );
        }
        let peers = [
            peer("clost", Role::Interactive, Presence::Lost),
            peer("cgone", Role::Interactive, Presence::Departed),
            peer("mlost", Role::Interactive, Presence::Lost),
            peer("mgone", Role::Interactive, Presence::Departed),
        ];
        let known_offline = [known_user("cgone", 900), known_user("mgone", 900)];
        let known_offline_names = |props: &UsersProps| {
            props
                .known_offline
                .iter()
                .map(|row| row.name.clone())
                .collect::<Vec<_>>()
        };
        let props = users_props(&state.view(), &peers, &known_offline, 1_000);
        let by_name: BTreeMap<&str, &UserRow> = props
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row))
            .collect();

        // Committed absent users block, on a visible row, Lost or Departed.
        for who in ["clost", "cgone"] {
            assert_eq!(by_name[who].label, "committed, away", "{who}");
            assert_eq!(by_name[who].tone, Tone::Blocked, "{who}");
        }
        assert!(
            !known_offline_names(&props).contains(&"cgone".to_string()),
            "a committed departed user must not hide on the dim line"
        );
        // Maybe absent users do not block.
        assert_eq!(by_name["mlost"].label, "lost");
        assert_eq!(by_name["mlost"].tone, Tone::Idle);
        assert_eq!(known_offline_names(&props), vec!["mgone"]);

        // Acknowledging cgone for this file clears the block: it returns to
        // the dim known-offline line.
        state.acknowledge_absent(hash(1), UserId::new("cgone"));
        let props = users_props(&state.view(), &peers, &known_offline, 1_000);
        assert!(
            known_offline_names(&props).contains(&"cgone".to_string()),
            "an acknowledged committed-absent user no longer blocks: {:?}",
            known_offline_names(&props)
        );
    }

    #[test]
    fn away_excuses_a_committed_absent_user_in_the_users_pane() {
        // Regression: marking a committed (Watching) absent user Away is the
        // per-user escape hatch that lets playback proceed (mirrors derive's
        // `away_excuses_a_committed_absent_user`). The Users pane must follow
        // `derive::playback_blockers` and stop drawing them as a red
        // "committed, away" blocker — otherwise the pane contradicts gating.
        let series = AniDbSeriesId(7);
        for presence in [Presence::Lost, Presence::Departed] {
            let mut state = CrdtState::new();
            state.set_now_playing(A, ts(1), Some(hash(1)));
            state.set_anidb_metadata(
                A,
                ts(2),
                hash(1),
                Some(AniDbMetadata {
                    source: MetadataSource::AniDb,
                    series_name: "Show".into(),
                    series_id: Some(series),
                    episode_number: Some("1".into()),
                }),
            );
            let entry = link_series(&mut state, ts(1), series);
            state.set_series_preference(
                A,
                ts(3),
                UserId::new("cabs"),
                entry,
                SeriesWatchState::Watching,
                None,
            );
            let peers = [peer("cabs", Role::Interactive, presence)];

            // Before Away: shown as a red "committed, away" blocker.
            let props = users_props(&state.view(), &peers, &[], 0);
            assert!(
                props
                    .rows
                    .iter()
                    .any(|r| r.name == "cabs" && r.tone == Tone::Blocked),
                "committed-absent ({presence:?}) should start as a blocker"
            );

            // Marking them Away clears the block (derive::playback_blockers
            // early-continues on Away), so the Users pane must no longer show
            // a red blocker row for them.
            state.set_manual_override(
                A,
                ts(4),
                UserId::new("cabs"),
                Some(ManualState::Away {
                    set_by: UserId::new("kim"),
                }),
            );
            let props = users_props(&state.view(), &peers, &[], 0);
            assert!(
                !props
                    .rows
                    .iter()
                    .any(|r| r.name == "cabs" && r.tone == Tone::Blocked),
                "away-excused committed-absent ({presence:?}) must not be a red blocker: {:?}",
                props.rows
            );
        }
    }

    #[test]
    fn download_progress_is_always_visible_red_when_not_ready() {
        // Bug 1a / follow-up: an in-progress download must never be
        // shadowed by a paused/away/not-watching label (design.md Ready
        // States: "Downloading | Any & Downloading"). When the user is
        // not Ready the progress shows in red.
        let mut state = CrdtState::new();
        state.set_now_playing(A, ts(1), Some(hash(1)));
        state.set_anidb_metadata(
            A,
            ts(2),
            hash(1),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Show".into(),
                series_id: Some(AniDbSeriesId(7)),
                episode_number: Some("1".into()),
            }),
        );
        // A not-watching downloader and a paused downloader.
        let entry = link_series(&mut state, ts(1), AniDbSeriesId(7));
        state.set_series_preference(
            A,
            ts(3),
            UserId::new("ndl"),
            entry,
            SeriesWatchState::NotWatching,
            None,
        );
        state.set_manual_override(A, ts(4), UserId::new("pdl"), Some(ManualState::Paused));
        for (i, name) in [(5, "ndl"), (6, "pdl")] {
            state.set_file_availability(
                A,
                ts(i),
                UserId::new(name),
                hash(1),
                FileAvailability::Downloading {
                    progress_bps: 1_500,
                },
            );
        }
        let peers = [
            peer("ndl", Role::Interactive, Presence::Present),
            peer("pdl", Role::Interactive, Presence::Present),
        ];
        let props = users_props(&state.view(), &peers, &[], 0);
        let by_name: BTreeMap<&str, &UserRow> = props
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row))
            .collect();
        assert_eq!(by_name["ndl"].label, "downloading 15%");
        assert_eq!(by_name["ndl"].tone, Tone::Blocked);
        assert_eq!(by_name["pdl"].label, "downloading 15%");
        assert_eq!(by_name["pdl"].tone, Tone::Blocked);
    }

    #[test]
    fn playlist_rows_highlight_and_mute() {
        let mut state = CrdtState::new();
        state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
        state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
        state.push_playlist_entry(A, ts(3), entry(3, "ep3.mkv"));
        state.set_watched(A, ts(4), hash(1), true);
        state.set_now_playing(A, ts(5), Some(hash(2)));
        state.set_file_availability(
            A,
            ts(6),
            UserId::new("kim"),
            hash(3),
            FileAvailability::Missing,
        );
        let props = playlist_props(&state.view(), &UserId::new("kim"), &BTreeSet::new());
        assert_eq!(props.now_index, Some(1));
        assert_eq!(
            props.rows.iter().map(|r| r.tone).collect::<Vec<_>>(),
            vec![Tone::Muted, Tone::Good, Tone::Blocked]
        );
    }

    #[test]
    fn playlist_rows_show_own_download_progress() {
        // Background downloads (prefetch included) surface their
        // percentage on the playlist row — playable or not — and only
        // *our* downloads count; a peer's never marks our playlist.
        let mut state = CrdtState::new();
        state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
        state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
        state.push_playlist_entry(A, ts(3), entry(3, "ep3.mkv"));
        state.set_file_availability(
            A,
            ts(4),
            UserId::new("kim"),
            hash(1),
            FileAvailability::Downloading {
                progress_bps: 1_500,
            },
        );
        state.set_file_availability(
            A,
            ts(5),
            UserId::new("kim"),
            hash(2),
            FileAvailability::DownloadingPlayable {
                progress_bps: 4_200,
            },
        );
        state.set_file_availability(
            A,
            ts(6),
            UserId::new("baughn"),
            hash(3),
            FileAvailability::Downloading {
                progress_bps: 9_000,
            },
        );
        let props = playlist_props(&state.view(), &UserId::new("kim"), &BTreeSet::new());
        assert_eq!(
            props.rows.iter().map(|r| r.download).collect::<Vec<_>>(),
            vec![Some(1_500), Some(4_200), None]
        );
    }

    #[test]
    fn playlist_props_marks_temporary() {
        // ep2 in cache only -> temporary; ep1 in a media root (absent
        // from the set) -> not; ep3 missing despite a cache row -> not.
        let mut state = CrdtState::new();
        state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
        state.push_playlist_entry(A, ts(2), entry(2, "ep2.mkv"));
        state.push_playlist_entry(A, ts(3), entry(3, "ep3.mkv"));
        state.set_file_availability(
            A,
            ts(4),
            UserId::new("kim"),
            hash(3),
            FileAvailability::Missing,
        );
        let cache: BTreeSet<Ed2kHash> = [hash(2), hash(3)].into_iter().collect();
        let props = playlist_props(&state.view(), &UserId::new("kim"), &cache);
        assert_eq!(
            props.rows.iter().map(|r| r.temporary).collect::<Vec<_>>(),
            vec![false, true, false]
        );
    }

    #[test]
    fn status_props_name_blockers() {
        let mut state = CrdtState::new();
        state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
        state.set_now_playing(A, ts(2), Some(hash(1)));
        state.set_playback_intent(A, ts(3), PlaybackIntent::Playing);
        state.set_manual_override(A, ts(4), UserId::new("kim"), Some(ManualState::Paused));
        let peers = [peer("kim", Role::Interactive, Presence::Present)];
        let props = status_props(
            &state.view(),
            &peers,
            &UserId::new("kim"),
            LinkStatus::Connected,
        );
        assert_eq!(props.title.as_deref(), Some("ep1.mkv"));
        assert!(!props.playing);
        assert_eq!(props.blockers, vec!["kim (paused)"]);
        assert_eq!(props.duration_millis, Some(1_440_000));
    }

    // ---- The List ----------------------------------------------------

    /// A bare List entry; tests override single fields.
    fn list_entry(name: &str, status: ListStatus) -> SeriesListEntry {
        SeriesListEntry {
            name: name.into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        }
    }

    fn commit(state: &mut CrdtState, t: u64, user: &str, entry: u128) {
        state.set_series_preference(
            A,
            ts(t),
            UserId::new(user),
            ListEntryId(entry),
            SeriesWatchState::Watching,
            None,
        );
    }

    /// `list_groups` with the common fixture context: me = "baughn",
    /// users = baughn/kim/nero, no watch history.
    fn groups_of(state: &CrdtState, sort: ListSort) -> Vec<ListGroup> {
        list_groups(
            &state.view(),
            &UserId::new("baughn"),
            &[
                UserId::new("baughn"),
                UserId::new("kim"),
                UserId::new("nero"),
            ],
            sort,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
    }

    fn headings(groups: &[ListGroup]) -> Vec<&str> {
        groups.iter().map(|g| g.heading.as_str()).collect()
    }

    #[test]
    fn list_groups_follow_design_order() {
        let mut state = CrdtState::new();
        let mut put = |id: u128, name: &str, status: ListStatus| {
            state.put_list_entry(A, ts(id as u64), ListEntryId(id), list_entry(name, status));
        };
        put(1, "Airing", ListStatus::CurrentSeason);
        put(2, "Binging", ListStatus::Active);
        put(3, "Done", ListStatus::Finished);
        put(4, "Up next", ListStatus::ShortList);
        // kim is committed to "Airing"; "Binging" has no committed watcher
        // and falls into the residual shared Watching group.
        commit(&mut state, 5, "kim", 1);
        state.set_next_ep(
            A,
            ts(10),
            ListEntryId(1),
            NextEpState {
                next_ep: Some("12".into()),
                available: true,
            },
        );

        let groups = groups_of(&state, ListSort::Alphabetical);
        assert_eq!(
            headings(&groups),
            vec![
                "Watching — kim",
                "Watching",
                "Short List",
                "Finished / Dropped"
            ]
        );
        let kim = &groups[0];
        assert_eq!(kim.rows.len(), 1);
        assert_eq!(kim.rows[0].name, "Airing");
        assert_eq!(kim.rows[0].next_ep.as_deref(), Some("12"));
        assert!(kim.rows[0].available);
        assert_eq!(kim.rows[0].watchers, "K");
        assert_eq!(groups[1].rows[0].name, "Binging");
        assert!(groups.last().unwrap().collapsed);
    }

    /// A linked entry still carrying its auto-seeded name (== the
    /// official title) displays under the curated short title, and
    /// alphabetizes where it reads. A human-named entry keeps its name
    /// even when a short title exists; entries without short titles,
    /// and unlinked entries, keep theirs too.
    #[test]
    fn linked_entries_display_their_short_title() {
        let mut state = CrdtState::new();
        // Auto-seeded: name == official title -> substituted.
        let mut gochiusa = list_entry("Gochuumon wa Usagi Desu ka??", ListStatus::Planned);
        gochiusa.anidb_series_id = Some(AniDbSeriesId(1));
        state.put_list_entry(A, ts(1), ListEntryId(1), gochiusa);
        // Human-named: name differs from the official title -> kept,
        // short title or not.
        let mut renamed = list_entry("Bunny Cafe", ListStatus::Planned);
        renamed.anidb_series_id = Some(AniDbSeriesId(3));
        state.put_list_entry(A, ts(2), ListEntryId(2), renamed);
        // Linked, no curated short title -> kept.
        let mut plain = list_entry("Zetsubou", ListStatus::Planned);
        plain.anidb_series_id = Some(AniDbSeriesId(2));
        state.put_list_entry(A, ts(3), ListEntryId(3), plain);
        state.put_list_entry(
            A,
            ts(4),
            ListEntryId(4),
            list_entry("Unlinked", ListStatus::Planned),
        );
        let relations = |title: &str, short: &[&str]| SeriesRelations {
            title: title.into(),
            year: None,
            episode_count: None,
            relations: Default::default(),
            short_titles: short.iter().map(|s| s.to_string()).collect(),
        };
        state.set_series_relations(
            A,
            ts(5),
            AniDbSeriesId(1),
            relations("Gochuumon wa Usagi Desu ka??", &["GochiUsa"]),
        );
        state.set_series_relations(A, ts(6), AniDbSeriesId(2), relations("Zetsubou", &[]));
        state.set_series_relations(
            A,
            ts(7),
            AniDbSeriesId(3),
            relations("Gochuumon wa Usagi Desu ka?", &["GochiUsa"]),
        );

        let groups = groups_of(&state, ListSort::Alphabetical);
        assert_eq!(headings(&groups), vec!["Planned"]);
        let names: Vec<&str> = groups[0].rows.iter().map(|r| r.name.as_str()).collect();
        // "GochiUsa" (not "Gochuumon...") sorts as G; the human rename
        // "Bunny Cafe" survives its series' short title.
        assert_eq!(
            names,
            vec!["Bunny Cafe", "GochiUsa", "Unlinked", "Zetsubou"]
        );
    }

    /// The users column derives from live `series_preference`, not the
    /// import-time `watchers` seed: the seed says Baughn+Nero, but only
    /// kim has a Watching preference — the column shows kim alone, and
    /// the per-user groups follow the preferences too.
    #[test]
    fn commitment_column_is_live_preference_not_the_watchers_seed() {
        let mut state = CrdtState::new();
        let mut entry = list_entry("Airing", ListStatus::CurrentSeason);
        entry.watchers = [UserId::new("baughn"), UserId::new("nero")]
            .into_iter()
            .collect();
        state.put_list_entry(A, ts(1), ListEntryId(1), entry);
        commit(&mut state, 2, "kim", 1);
        // nero explicitly backed out after the seed.
        state.set_series_preference(
            A,
            ts(3),
            UserId::new("nero"),
            ListEntryId(1),
            SeriesWatchState::NotWatching,
            None,
        );

        let groups = groups_of(&state, ListSort::Alphabetical);
        assert_eq!(headings(&groups), vec!["Watching — kim"]);
        assert_eq!(groups[0].rows[0].watchers, "K");
    }

    /// An entry appears in *every* committed user's group — including a
    /// known-offline user's — with `me` first and the rest alphabetical.
    #[test]
    fn entry_appears_in_every_committed_users_group() {
        let mut state = CrdtState::new();
        state.put_list_entry(
            A,
            ts(1),
            ListEntryId(1),
            list_entry("Airing", ListStatus::CurrentSeason),
        );
        // baughn is me; nero is known-offline in the fixture's user set.
        commit(&mut state, 2, "nero", 1);
        commit(&mut state, 3, "baughn", 1);
        commit(&mut state, 4, "kim", 1);

        let groups = groups_of(&state, ListSort::Alphabetical);
        assert_eq!(
            headings(&groups),
            vec!["Watching — baughn", "Watching — kim", "Watching — nero"]
        );
        for group in &groups {
            assert_eq!(group.rows.len(), 1, "{}", group.heading);
            assert_eq!(group.rows[0].name, "Airing");
        }
    }

    /// Nothing vanishes: a Watching-tier entry whose only committed
    /// watcher is a user the client can't name (not a peer, not in the
    /// known-offline roster — no rendered group) still lands in the
    /// residual shared Watching group.
    #[test]
    fn commitment_by_an_unknown_user_falls_into_the_residual_group() {
        let mut state = CrdtState::new();
        state.put_list_entry(
            A,
            ts(1),
            ListEntryId(1),
            list_entry("Airing", ListStatus::CurrentSeason),
        );
        commit(&mut state, 2, "stranger", 1);

        let groups = groups_of(&state, ListSort::Alphabetical);
        assert_eq!(headings(&groups), vec!["Watching"]);
        assert_eq!(groups[0].rows[0].name, "Airing");
        // The column still shows the commitment even without a group.
        assert_eq!(groups[0].rows[0].watchers, "S");
    }

    /// A non-Watching-tier status keeps an entry out of the per-user
    /// groups even when someone is committed: commitment says who waits,
    /// status says where the entry lives.
    #[test]
    fn committed_but_planned_entries_stay_in_their_status_group() {
        let mut state = CrdtState::new();
        state.put_list_entry(
            A,
            ts(1),
            ListEntryId(1),
            list_entry("Someday", ListStatus::Planned),
        );
        commit(&mut state, 2, "kim", 1);

        let groups = groups_of(&state, ListSort::Alphabetical);
        assert_eq!(headings(&groups), vec!["Planned"]);
        assert_eq!(groups[0].rows[0].watchers, "K");
    }

    /// The season tree (proposal 2026-08-28): the prequel chain is the
    /// main line; side stories — and a whole chain of them — indent
    /// under the season they branch from, in their own chain order.
    #[test]
    fn season_tree_indents_branches_under_their_parent() {
        use dessplay_core::types::{RelationKind, SeriesRelation, SeriesRelations};
        let mut state = CrdtState::new();
        let mut relations = |id: u32, title: &str, year: u16, edges: &[(RelationKind, u32)]| {
            state.set_series_relations(
                A,
                ts(1),
                AniDbSeriesId(id),
                SeriesRelations {
                    title: title.into(),
                    year: Some(year),
                    episode_count: None,
                    relations: edges
                        .iter()
                        .map(|(kind, target)| SeriesRelation {
                            kind: *kind,
                            target: AniDbSeriesId(*target),
                        })
                        .collect(),
                    short_titles: vec![],
                },
            );
        };
        use RelationKind::*;
        relations(
            1,
            "GuP",
            2012,
            &[(Sequel, 3), (SideStory, 2), (SideStory, 10)],
        );
        relations(2, "GuP: Anzio OVA", 2014, &[(ParentStory, 1)]);
        relations(3, "GuP der Film", 2015, &[(Prequel, 1)]);
        relations(10, "GuP shorts", 2013, &[(ParentStory, 1), (Sequel, 11)]);
        relations(11, "GuP shorts 2", 2014, &[(Prequel, 10)]);
        for id in [1u32, 2, 3, 10, 11] {
            state.set_anidb_metadata(
                A,
                ts(2),
                Ed2kHash([id as u8; 16]),
                Some(AniDbMetadata {
                    source: MetadataSource::AniDb,
                    series_name: format!("s{id}"),
                    series_id: Some(AniDbSeriesId(id)),
                    episode_number: Some("1".into()),
                }),
            );
        }
        let view = state.view();
        let franchises = franchise::franchises(&view);
        assert_eq!(franchises.len(), 1);
        let tree: Vec<(u32, u8)> = season_tree(&view, &franchises[0])
            .into_iter()
            .map(|(series, depth)| (series.0, depth))
            .collect();
        assert_eq!(tree, vec![(1, 0), (2, 1), (10, 1), (11, 1), (3, 0)]);
    }

    /// One row per franchise (proposal 2026-08-28): entries linked to
    /// different seasons of one show collapse into a single row — the
    /// canonical entry's name and `next_ep`, the union of the members'
    /// commitments, and a recency taken from *any* season of the
    /// franchise (here a season with no entry of its own at all).
    #[test]
    fn linked_seasons_of_one_franchise_render_as_one_row() {
        use dessplay_core::types::{RelationKind, SeriesRelation, SeriesRelations};
        let mut state = CrdtState::new();
        let relations = |title: &str, edges: &[(RelationKind, u32)]| SeriesRelations {
            title: title.into(),
            year: None,
            episode_count: None,
            relations: edges
                .iter()
                .map(|(kind, target)| SeriesRelation {
                    kind: *kind,
                    target: AniDbSeriesId(*target),
                })
                .collect(),
            short_titles: vec![],
        };
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(1),
            relations("Yuru Yuri", &[(RelationKind::Sequel, 2)]),
        );
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(2),
            relations(
                "Yuru Yuri 2",
                &[(RelationKind::Prequel, 1), (RelationKind::Sequel, 3)],
            ),
        );
        state.set_series_relations(
            A,
            ts(1),
            AniDbSeriesId(3),
            relations("Yuru Yuri San Hai!", &[(RelationKind::Prequel, 2)]),
        );
        state.set_series_relations(A, ts(1), AniDbSeriesId(9), relations("Unrelated", &[]));
        let mut put = |id: u128, name: &str, series: u32| {
            let mut entry = list_entry(name, ListStatus::Active);
            entry.anidb_series_id = Some(AniDbSeriesId(series));
            state.put_list_entry(A, ts(id as u64), ListEntryId(id), entry);
        };
        put(1, "Yuru Yuri", 1);
        put(2, "Yuru Yuri 2", 2);
        put(9, "Unrelated", 9);
        // kim committed on season one, nero on season two: both watch
        // the franchise. Season two carries the live next_ep.
        commit(&mut state, 20, "kim", 1);
        commit(&mut state, 21, "nero", 2);
        state.set_next_ep(
            A,
            ts(30),
            ListEntryId(2),
            NextEpState {
                next_ep: Some("7".into()),
                available: false,
            },
        );
        // The group last watched season *three*, which has no entry.
        let recency: BTreeMap<SeriesKey, u64> = [
            (SeriesKey::AniDb(AniDbSeriesId(3)), 900),
            (SeriesKey::AniDb(AniDbSeriesId(9)), 100),
        ]
        .into_iter()
        .collect();

        let groups = list_groups(
            &state.view(),
            &UserId::new("kim"),
            &[UserId::new("kim"), UserId::new("nero")],
            ListSort::Recency,
            &recency,
            &BTreeMap::new(),
        );
        assert_eq!(
            headings(&groups),
            vec!["Watching — kim", "Watching — nero", "Watching"]
        );
        let kim = &groups[0];
        assert_eq!(
            kim.rows.len(),
            1,
            "one row for the franchise: {:?}",
            kim.rows
        );
        let row = &kim.rows[0];
        // Both entries are human-created (random ids); season two is
        // deeper along the prequel chain, so it is canonical.
        assert_eq!(row.id, ListEntryId(2));
        assert_eq!(row.name, "Yuru Yuri 2");
        assert_eq!(row.next_ep.as_deref(), Some("S2E07"));
        assert_eq!(row.watchers, "KN");
        assert_eq!(
            groups[1].rows[0].id,
            ListEntryId(2),
            "nero's group sees the same row"
        );
        // The residual group holds only the unrelated show; Recency in
        // kim's group is unaffected, but across the shared view the
        // franchise (watched at 900 via season three) outranks it.
        assert_eq!(
            groups[2].rows.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![ListEntryId(9)]
        );
        let all = list_groups(
            &state.view(),
            &UserId::new("x"),
            &[],
            ListSort::Recency,
            &recency,
            &BTreeMap::new(),
        );
        let names: Vec<&str> = all[0].rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["Yuru Yuri 2", "Unrelated"]);
    }

    /// Recency order: most recently watched first, never-watched last,
    /// names as the tiebreak. Having nothing to watch (no weekly
    /// `available`, no unwatched library file) *dims* a row but never
    /// moves it (proposal 2026-08-28): order stays predictable as files
    /// come and go.
    #[test]
    fn recency_sort_orders_by_last_watch_and_dims_without_reordering() {
        let mut state = CrdtState::new();
        let mut put = |id: u128, name: &str| {
            let mut entry = list_entry(name, ListStatus::CurrentSeason);
            entry.anidb_series_id = Some(AniDbSeriesId(id as u32));
            state.put_list_entry(A, ts(id as u64), ListEntryId(id), entry);
            commit(&mut state, 100 + id as u64, "kim", id);
        };
        put(1, "Available, stale");
        put(2, "Available, fresh");
        put(3, "Nothing to watch, fresh");
        put(4, "Unwatched file held");
        for id in [1u128, 2] {
            state.set_next_ep(
                A,
                ts(10 + id as u64),
                ListEntryId(id),
                NextEpState {
                    next_ep: Some("5".into()),
                    available: true,
                },
            );
        }
        // Entry 4's freshness comes from a held unwatched file, not the
        // weekly flag: known to the library *and* advertised by a client.
        state.set_anidb_metadata(
            A,
            ts(20),
            Ed2kHash([4; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Unwatched file held".into(),
                series_id: Some(AniDbSeriesId(4)),
                episode_number: Some("2".into()),
            }),
        );
        state.set_file_availability(
            A,
            ts(21),
            UserId::new("kim"),
            Ed2kHash([4; 16]),
            FileAvailability::Ready,
        );
        let recency: BTreeMap<SeriesKey, u64> = [
            (SeriesKey::AniDb(AniDbSeriesId(1)), 100),
            (SeriesKey::AniDb(AniDbSeriesId(2)), 900),
            (SeriesKey::AniDb(AniDbSeriesId(3)), 950),
        ]
        .into_iter()
        .collect();

        let groups = list_groups(
            &state.view(),
            &UserId::new("kim"),
            &[UserId::new("kim")],
            ListSort::Recency,
            &recency,
            &BTreeMap::new(),
        );
        assert_eq!(headings(&groups), vec!["Watching — kim"]);
        let names: Vec<&str> = groups[0].rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Nothing to watch, fresh", // watched at 950 (dim, but not demoted)
                "Available, fresh",        // watched at 900
                "Available, stale",        // watched at 100
                "Unwatched file held",     // never watched
            ]
        );
        let dimmed: Vec<bool> = groups[0].rows.iter().map(|r| r.dimmed).collect();
        assert_eq!(dimmed, vec![true, false, false, false]);

        // Alphabetical ignores all of that.
        let groups = groups_of(&state, ListSort::Alphabetical);
        let watching = groups
            .iter()
            .find(|g| g.heading == "Watching — kim")
            .unwrap();
        let names: Vec<&str> = watching.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Available, fresh",
                "Available, stale",
                "Nothing to watch, fresh",
                "Unwatched file held"
            ]
        );
    }

    /// Unwatched-file resolution follows the Series Identity order for
    /// unlinked entries too: `manual_files` membership and exact
    /// alias/name matches light an entry up; a group-watched file does
    /// not.
    #[test]
    fn unwatched_files_resolve_via_manual_files_and_aliases() {
        let mut state = CrdtState::new();
        let mut by_alias = list_entry("Alias Show", ListStatus::Active);
        by_alias.local_aliases = ["Alias Hint".to_string()].into_iter().collect();
        state.put_list_entry(A, ts(1), ListEntryId(1), by_alias);
        let mut by_manual = list_entry("Manual Show", ListStatus::Active);
        by_manual.manual_files = [Ed2kHash([2; 16])].into_iter().collect();
        state.put_list_entry(A, ts(2), ListEntryId(2), by_manual);
        state.put_list_entry(
            A,
            ts(3),
            ListEntryId(3),
            list_entry("Watched Show", ListStatus::Active),
        );

        let meta = |name: &str| {
            Some(AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: name.into(),
                series_id: None,
                episode_number: None,
            })
        };
        state.set_anidb_metadata(A, ts(4), Ed2kHash([1; 16]), meta("Alias Hint"));
        state.set_anidb_metadata(A, ts(5), Ed2kHash([2; 16]), meta("No Name Match"));
        state.set_anidb_metadata(A, ts(6), Ed2kHash([3; 16]), meta("Watched Show"));
        state.set_watched(A, ts(7), Ed2kHash([3; 16]), true);
        // All three copies are held; only the watched flag separates them.
        for (t, hash) in [(8, [1; 16]), (9, [2; 16]), (10, [3; 16])] {
            state.set_file_availability(
                A,
                ts(t),
                UserId::new("kim"),
                Ed2kHash(hash),
                FileAvailability::Ready,
            );
        }

        let groups = groups_of(&state, ListSort::Alphabetical);
        let watching = groups.iter().find(|g| g.heading == "Watching").unwrap();
        let dimmed: BTreeMap<&str, bool> = watching
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.dimmed))
            .collect();
        assert_eq!(
            dimmed,
            [
                ("Alias Show", false),
                ("Manual Show", false),
                ("Watched Show", true)
            ]
            .into_iter()
            .collect()
        );
    }

    /// Regression (2026-08-17, "everything floats"): "has unwatched
    /// files" must mean *held and unwatched by the durable record*.
    /// Compaction drops group watched flags for files off the playlist
    /// while their metadata rows live forever, so the group flag alone
    /// resurrects every long-finished episode as "unwatched"; and a
    /// metadata row whose file nobody holds anymore isn't watchable
    /// tonight either. A file counts only when some client advertises a
    /// copy and it is unwatched by both the group flag and personal
    /// watch history (the episode browser's muting rule, design.md #11).
    #[test]
    fn unwatched_needs_a_held_copy_and_survives_compacted_flags() {
        let mut state = CrdtState::new();
        let mut put = |id: u128, name: &str| {
            let mut entry = list_entry(name, ListStatus::Active);
            entry.anidb_series_id = Some(AniDbSeriesId(id as u32));
            state.put_list_entry(A, ts(id as u64), ListEntryId(id), entry);
        };
        put(1, "Finished long ago");
        put(2, "Metadata ghost");
        put(3, "Fresh episode");
        let meta = |series: u32| {
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: format!("series {series}"),
                series_id: Some(AniDbSeriesId(series)),
                episode_number: Some("1".into()),
            })
        };
        // Entry 1: file held and long since watched — but the group flag
        // was compacted away; only personal history remembers.
        state.set_anidb_metadata(A, ts(10), Ed2kHash([1; 16]), meta(1));
        state.set_file_availability(
            A,
            ts(11),
            UserId::new("kim"),
            Ed2kHash([1; 16]),
            FileAvailability::Ready,
        );
        // Entry 2: a metadata row survives but nobody holds the file.
        state.set_anidb_metadata(A, ts(12), Ed2kHash([2; 16]), meta(2));
        // Entry 3: held and genuinely unwatched.
        state.set_anidb_metadata(A, ts(13), Ed2kHash([3; 16]), meta(3));
        state.set_file_availability(
            A,
            ts(14),
            UserId::new("kim"),
            Ed2kHash([3; 16]),
            FileAvailability::Ready,
        );

        let personally_watched: BTreeMap<Ed2kHash, i64> =
            [(Ed2kHash([1; 16]), 1)].into_iter().collect();
        let groups = list_groups(
            &state.view(),
            &UserId::new("kim"),
            &[UserId::new("kim")],
            ListSort::Recency,
            &BTreeMap::new(),
            &personally_watched,
        );
        let watching = groups.iter().find(|g| g.heading == "Watching").unwrap();
        let dimmed: BTreeMap<&str, bool> = watching
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.dimmed))
            .collect();
        assert_eq!(
            dimmed,
            [
                ("Finished long ago", true),
                ("Metadata ghost", true),
                ("Fresh episode", false)
            ]
            .into_iter()
            .collect()
        );
        // Dimming never reorders: with no watch history the Recency
        // order is plain name order (proposal 2026-08-28).
        let names: Vec<&str> = watching.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Finished long ago", "Fresh episode", "Metadata ghost"]
        );
    }

    /// A held duplicate copy of an episode the group already watched
    /// through *any* other copy does not float the series: "unwatched"
    /// means episode identity, not file identity — the same any-copy
    /// rule as the episode browser's muting.
    #[test]
    fn duplicate_copy_of_a_watched_episode_does_not_float() {
        let mut state = CrdtState::new();
        let mut put = |id: u128, name: &str| {
            let mut entry = list_entry(name, ListStatus::Active);
            entry.anidb_series_id = Some(AniDbSeriesId(id as u32));
            state.put_list_entry(A, ts(id as u64), ListEntryId(id), entry);
        };
        put(1, "Rewatched in HEVC");
        put(2, "Genuinely fresh");
        let meta = |series: u32, episode: &str| {
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: format!("series {series}"),
                series_id: Some(AniDbSeriesId(series)),
                episode_number: Some(episode.into()),
            })
        };
        // Series 1, episode 01: the watched copy plus a held duplicate
        // encoding with no watched record of its own.
        state.set_anidb_metadata(A, ts(10), Ed2kHash([1; 16]), meta(1, "01"));
        state.set_watched(A, ts(11), Ed2kHash([1; 16]), true);
        state.set_anidb_metadata(A, ts(12), Ed2kHash([2; 16]), meta(1, "01"));
        // Series 2, episode 01: held and truly unwatched.
        state.set_anidb_metadata(A, ts(13), Ed2kHash([3; 16]), meta(2, "01"));
        for (t, hash) in [(14, [2; 16]), (15, [3; 16])] {
            state.set_file_availability(
                A,
                ts(t),
                UserId::new("kim"),
                Ed2kHash(hash),
                FileAvailability::Ready,
            );
        }

        let groups = groups_of(&state, ListSort::Recency);
        let watching = groups.iter().find(|g| g.heading == "Watching").unwrap();
        let dimmed: BTreeMap<&str, bool> = watching
            .rows
            .iter()
            .map(|r| (r.name.as_str(), r.dimmed))
            .collect();
        assert_eq!(
            dimmed,
            [("Rewatched in HEVC", true), ("Genuinely fresh", false)]
                .into_iter()
                .collect()
        );
    }

    // ---- ListGroupsCache ---------------------------------------------

    /// Cache fixture: three linked Watching-tier entries, all committed
    /// by kim so they share one group; a held unwatched file resolves to
    /// entry 1 ("Zeta Bright" — bright, floats in Recency, sinks
    /// alphabetically), the other two dim. Enough structure that every
    /// invalidation input below visibly changes the derived groups.
    fn cache_state() -> CrdtState {
        let mut state = CrdtState::new();
        for (id, name) in [(1, "Zeta Bright"), (2, "Alpha Other"), (3, "Middle Show")] {
            let mut entry = list_entry(name, ListStatus::Active);
            entry.anidb_series_id = Some(AniDbSeriesId(id as u32));
            state.put_list_entry(A, ts(id), ListEntryId(id as u128), entry);
        }
        for (t, id) in [(11, 1), (12, 2), (13, 3)] {
            commit(&mut state, t, "kim", id);
        }
        state.set_anidb_metadata(
            A,
            ts(4),
            Ed2kHash([1; 16]),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Zeta Bright".into(),
                series_id: Some(AniDbSeriesId(1)),
                episode_number: Some("1".into()),
            }),
        );
        state.set_file_availability(
            A,
            ts(5),
            UserId::new("kim"),
            Ed2kHash([1; 16]),
            FileAvailability::Ready,
        );
        state
    }

    /// A cache hit returns exactly what a fresh compute would, and the
    /// non-inputs — position ticks, download progress on an
    /// already-held file — do not recompute. This is the load the cache
    /// exists for (~10 Hz snapshots during playback); the CPU-side guard
    /// is the perf rig.
    #[test]
    fn list_groups_cache_hits_match_fresh_compute_and_ignore_ticks() {
        let mut state = cache_state();
        let me = UserId::new("kim");
        let users = [UserId::new("kim")];
        let mut cache = ListGroupsCache::default();

        let fresh = groups_of(&state, ListSort::Recency);
        let via_cache = |cache: &mut ListGroupsCache, state: &CrdtState| {
            cache
                .get(
                    &state.view(),
                    &UserId::new("baughn"),
                    &[
                        UserId::new("baughn"),
                        UserId::new("kim"),
                        UserId::new("nero"),
                    ],
                    ListSort::Recency,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                )
                .to_vec()
        };
        assert_eq!(via_cache(&mut cache, &state), fresh);
        assert_eq!(via_cache(&mut cache, &state), fresh);
        assert_eq!(cache.recomputes, 1, "an unchanged view must not recompute");

        // A position tick — the ~10 Hz snapshot driver — is not an input.
        state.set_playback_position(
            A,
            ts(100),
            me.clone(),
            dessplay_core::types::PlaybackPosition {
                position_millis: 90_000,
                timestamp: ts(100),
                file: Ed2kHash([1; 16]),
            },
        );
        // Nor is download *progress* on a file that stays held.
        state.set_file_availability(
            A,
            ts(101),
            users[0].clone(),
            Ed2kHash([9; 16]),
            FileAvailability::Downloading { progress_bps: 1000 },
        );
        via_cache(&mut cache, &state);
        assert_eq!(cache.recomputes, 2, "a new held file must recompute");
        state.set_file_availability(
            A,
            ts(102),
            users[0].clone(),
            Ed2kHash([9; 16]),
            FileAvailability::Downloading { progress_bps: 2000 },
        );
        via_cache(&mut cache, &state);
        assert_eq!(
            cache.recomputes, 2,
            "progress ticks on a held file must hit the cache"
        );
    }

    /// Every input the derivation reads invalidates the cache, and the
    /// recomputed groups equal a fresh compute. A stale List that shrugs
    /// off a watched toggle would be a worse bug than the CPU burn the
    /// cache saves.
    #[test]
    fn list_groups_cache_invalidates_on_each_input() {
        let state = cache_state();
        let me = UserId::new("baughn");
        let users = vec![
            UserId::new("baughn"),
            UserId::new("kim"),
            UserId::new("nero"),
        ];

        // Each step mutates one input; the harness asserts the cache
        // recomputed *and* that the input was semantic (the groups
        // changed), so a step that stops changing the output rots
        // visibly instead of silently passing.
        struct Step {
            name: &'static str,
            state: CrdtState,
            users: Vec<UserId>,
            sort: ListSort,
            recency: BTreeMap<SeriesKey, u64>,
            watched_hashes: BTreeMap<Ed2kHash, i64>,
        }
        // A base watch history, so Recency and Alphabetical differ (with
        // no history the two orders coincide — dimming no longer
        // partitions, proposal 2026-08-28).
        let base_recency: BTreeMap<SeriesKey, u64> = [(SeriesKey::Name("Zeta Bright".into()), 300)]
            .into_iter()
            .collect();
        let base = |state: &CrdtState| Step {
            name: "",
            state: state.clone(),
            users: users.clone(),
            sort: ListSort::Recency,
            recency: base_recency.clone(),
            watched_hashes: BTreeMap::new(),
        };

        let mut steps = Vec::new();
        // Group watched flag flips: the held file stops counting and
        // "Bright Show" dims.
        let mut watched = base(&state);
        watched.name = "watched flip";
        watched
            .state
            .set_watched(A, ts(50), Ed2kHash([1; 16]), true);
        steps.push(watched);
        // Availability flips to Missing: same effect, different input.
        let mut availability = base(&state);
        availability.name = "availability flip";
        availability.state.set_file_availability(
            A,
            ts(50),
            UserId::new("kim"),
            Ed2kHash([1; 16]),
            FileAvailability::Missing,
        );
        steps.push(availability);
        // Sort toggle reorders within groups.
        let mut sort = base(&state);
        sort.name = "sort toggle";
        sort.sort = ListSort::Alphabetical;
        steps.push(sort);
        // A user joining (with a commitment) grows the per-user groups.
        let mut join = base(&state);
        join.name = "user join";
        join.users.push(UserId::new("amu"));
        commit(&mut join.state, 50, "amu", 2);
        steps.push(join);
        // Personal watch history: the same file watched only locally.
        let mut personal = base(&state);
        personal.name = "personal watch history";
        personal.watched_hashes.insert(Ed2kHash([1; 16]), 1);
        steps.push(personal);
        // Recency floats the most recently watched row.
        let mut recency = base(&state);
        recency.name = "recency";
        recency
            .recency
            .insert(SeriesKey::Name("Middle Show".into()), 500);
        steps.push(recency);
        // An entry edit (rename) rewrites the row.
        let mut rename = base(&state);
        rename.name = "entry edit";
        let mut renamed = list_entry("Renamed Show", ListStatus::Active);
        renamed.anidb_series_id = Some(AniDbSeriesId(2));
        rename
            .state
            .put_list_entry(A, ts(50), ListEntryId(2), renamed);
        steps.push(rename);
        // next_ep arrives: episode text + available marker.
        let mut next_ep = base(&state);
        next_ep.name = "next_ep";
        next_ep.state.set_next_ep(
            A,
            ts(50),
            ListEntryId(1),
            NextEpState {
                next_ep: Some("5".into()),
                available: true,
            },
        );
        steps.push(next_ep);
        // Relations arrive: the curated short title takes over display.
        let mut relations = base(&state);
        relations.name = "series relations";
        relations.state.set_series_relations(
            A,
            ts(50),
            AniDbSeriesId(1),
            SeriesRelations {
                title: "Zeta Bright".into(),
                year: None,
                episode_count: None,
                relations: Default::default(),
                short_titles: vec!["Brights".into()],
            },
        );
        steps.push(relations);

        for step in steps {
            let mut cache = ListGroupsCache::default();
            let before = cache
                .get(
                    &state.view(),
                    &me,
                    &users,
                    ListSort::Recency,
                    &base_recency,
                    &BTreeMap::new(),
                )
                .to_vec();
            let after = cache
                .get(
                    &step.state.view(),
                    &me,
                    &step.users,
                    step.sort,
                    &step.recency,
                    &step.watched_hashes,
                )
                .to_vec();
            assert_eq!(cache.recomputes, 2, "{}: must invalidate", step.name);
            assert_ne!(
                after, before,
                "{}: fixture change must be semantic",
                step.name
            );
            let fresh = list_groups(
                &step.state.view(),
                &me,
                &step.users,
                step.sort,
                &step.recency,
                &step.watched_hashes,
            );
            assert_eq!(
                after, fresh,
                "{}: cached result must equal fresh",
                step.name
            );
        }
    }

    /// SnEnn display: a linked entry with a numeric `next_ep` renders as
    /// `S⟨ordinal⟩E⟨nn⟩`, the ordinal counted along the prequel chain.
    /// Free text and unlinked entries render verbatim; a cycle in the
    /// relations data terminates; a chain broken by a missing relations
    /// entry counts the visible prefix.
    #[test]
    fn next_ep_renders_snenn_for_linked_numeric_entries() {
        let mut state = CrdtState::new();
        let relations = |title: &str, prequel: Option<u32>| SeriesRelations {
            title: title.into(),
            year: None,
            episode_count: None,
            relations: prequel
                .map(|target| SeriesRelation {
                    kind: RelationKind::Prequel,
                    target: AniDbSeriesId(target),
                })
                .into_iter()
                .collect(),
            short_titles: vec![],
        };
        // Season 3 (id 30) -> season 2 (id 20) -> season 1 (id 10).
        state.set_series_relations(A, ts(1), AniDbSeriesId(30), relations("S3", Some(20)));
        state.set_series_relations(A, ts(2), AniDbSeriesId(20), relations("S2", Some(10)));
        state.set_series_relations(A, ts(3), AniDbSeriesId(10), relations("S1", None));
        // A mutual-prequel mistake: 41 <-> 40.
        state.set_series_relations(A, ts(4), AniDbSeriesId(41), relations("Cyc B", Some(40)));
        state.set_series_relations(A, ts(5), AniDbSeriesId(40), relations("Cyc A", Some(41)));
        // 50's prequel (49) has no relations entry yet: the walk counts
        // the one visible step and stops.
        state.set_series_relations(A, ts(6), AniDbSeriesId(50), relations("Broken", Some(49)));
        // A standalone first season.
        state.set_series_relations(A, ts(7), AniDbSeriesId(60), relations("Solo", None));

        // (entry id, name, linked series, next_ep free text, expected display)
        type Case = (
            u128,
            &'static str,
            Option<u32>,
            &'static str,
            Option<&'static str>,
        );
        let cases: &[Case] = &[
            // One franchise per case: entries linked into the same chain
            // would collapse into one row (proposal 2026-08-28).
            (1, "Third season", Some(30), "5", Some("S3E05")),
            (2, "First season", Some(60), "12", Some("S1E12")),
            (3, "Cycle", Some(41), "3", Some("S2E03")),
            (4, "Broken chain", Some(50), "7", Some("S2E07")),
            // No relations entry at all: a plain S1.
            (5, "Unknown", Some(99), "9", Some("S1E09")),
            (6, "Free text", Some(70), "movie 5?", Some("movie 5?")),
            (7, "Unlinked", None, "8", Some("8")),
            (8, "No next ep", Some(80), "", None),
        ];
        for (id, name, series, next, _) in cases {
            let mut entry = list_entry(name, ListStatus::Active);
            entry.anidb_series_id = series.map(AniDbSeriesId);
            state.put_list_entry(A, ts(100 + *id as u64), ListEntryId(*id), entry);
            if !next.is_empty() {
                state.set_next_ep(
                    A,
                    ts(200 + *id as u64),
                    ListEntryId(*id),
                    NextEpState {
                        next_ep: Some((*next).into()),
                        available: false,
                    },
                );
            }
        }

        let groups = groups_of(&state, ListSort::Alphabetical);
        let watching = groups.iter().find(|g| g.heading == "Watching").unwrap();
        for (_, name, _, _, expected) in cases {
            let row = watching.rows.iter().find(|r| r.name == *name).unwrap();
            assert_eq!(row.next_ep.as_deref(), *expected, "{name}");
        }
    }

    #[test]
    fn hhmm_is_local() {
        use chrono::{Local, TimeZone};
        // Timestamps render in the machine's local timezone — verify
        // against chrono's own conversion rather than a hardcoded offset,
        // so the test holds wherever it runs.
        for millis in [0u64, 13 * 3_600_000 + 37 * 60_000 + 12_345] {
            let expected = Local
                .timestamp_millis_opt(millis as i64)
                .single()
                .unwrap()
                .format("%H:%M")
                .to_string();
            assert_eq!(hhmm(millis), expected);
        }
    }

    /// The episode browser must not show a bare ed2k hash. A file in the
    /// playlist shows its filename; a file known to AniDB but no longer
    /// in the playlist (its metadata register persists) shows
    /// "series — episode"; only a truly unknown file falls back to the
    /// hash. Regression for the "Niwatori Fighter, named by hash" report.
    #[test]
    fn episode_label_prefers_filename_then_metadata_then_hash() {
        use dessplay_core::types::{AniDbMetadata, AniDbSeriesId, MetadataSource};

        let mut state = CrdtState::new();
        // hash(1): in the playlist -> its on-disk filename.
        state.push_playlist_entry(A, ts(1), entry(1, "Niwatori - 01.mkv"));
        // hash(2): known to AniDB, absent from the playlist -> "name — ep".
        state.set_anidb_metadata(
            A,
            ts(2),
            hash(2),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Niwatori Fighter".into(),
                series_id: Some(AniDbSeriesId(18772)),
                episode_number: Some("01".into()),
            }),
        );
        // hash(3): metadata with no episode number and nothing more specific
        // to go on -> just the name.
        state.set_anidb_metadata(
            A,
            ts(3),
            hash(3),
            Some(AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: "Mystery Show".into(),
                series_id: None,
                episode_number: None,
            }),
        );
        let view = state.view();
        assert_eq!(episode_label(&view, &hash(1)), "Niwatori - 01.mkv");
        assert_eq!(episode_label(&view, &hash(2)), "Niwatori Fighter — 01");
        assert_eq!(episode_label(&view, &hash(3)), "Mystery Show");
        // hash(4): totally unknown -> the raw hash is the only fallback.
        assert_eq!(episode_label(&view, &hash(4)), hash(4).to_string());
    }

    /// Regression: filename-derived metadata shares one `series_name` across
    /// every episode of a series (it's a directory hint since the
    /// group-by-folder change). Such an episode must still be labelled by its
    /// own catalog filename, not by the shared series name — otherwise the
    /// episode browser shows N identical "Cardcaptor Sakura" rows.
    #[test]
    fn episode_label_uses_catalog_filename_when_series_name_is_a_shared_hint() {
        use dessplay_core::types::{AniDbMetadata, FileCatalogEntry, MetadataSource};

        let mut state = CrdtState::new();
        let hint = AniDbMetadata {
            source: MetadataSource::FilenameDerived,
            series_name: "Cardcaptor Sakura".into(),
            series_id: None,
            episode_number: None,
        };
        let catalog = |name: &str| FileCatalogEntry {
            filename: name.into(),
            size_bytes: 1,
            duration_millis: None,
        };
        // Two episodes of the same folder-derived "series": identical
        // metadata, distinct catalog filenames.
        for (i, name) in [
            (5u8, "Cardcaptor Sakura - 01 - The Magic Book.mkv"),
            (6, "Cardcaptor Sakura - 02 - Wonderful Friend.mkv"),
        ] {
            state.set_anidb_metadata(A, ts(i.into()), hash(i), Some(hint.clone()));
            state.set_file_catalog(A, ts(i.into()), hash(i), catalog(name));
        }
        let view = state.view();
        assert_eq!(
            episode_label(&view, &hash(5)),
            "Cardcaptor Sakura - 01 - The Magic Book.mkv"
        );
        assert_eq!(
            episode_label(&view, &hash(6)),
            "Cardcaptor Sakura - 02 - Wonderful Friend.mkv"
        );
    }

    /// Regression: when a file has *real* AniDB metadata (a genuine
    /// episode number, not a filename-derived directory hint) **and** a
    /// catalog entry, the catalog filename must still win. Multiple
    /// releases of the same AniDB-known episode share identical metadata,
    /// so preferring "series — episode" here rendered every copy in the
    /// episode browser as the exact same string (reported 2026-07-03:
    /// four copies of "Yuuki Yuuna wa Yuusha de Aru — 01" with no way to
    /// tell them apart).
    #[test]
    fn episode_label_prefers_catalog_filename_over_series_dash_episode_when_both_known() {
        use dessplay_core::types::{
            AniDbMetadata, AniDbSeriesId, FileCatalogEntry, MetadataSource,
        };

        let mut state = CrdtState::new();
        let metadata = AniDbMetadata {
            source: MetadataSource::AniDb,
            series_name: "Yuuki Yuuna wa Yuusha de Aru".into(),
            series_id: Some(AniDbSeriesId(1)),
            episode_number: Some("01".into()),
        };
        state.set_anidb_metadata(A, ts(1), hash(1), Some(metadata.clone()));
        state.set_file_catalog(
            A,
            ts(2),
            hash(1),
            FileCatalogEntry {
                filename: "[Judas] Yuuki Yuuna - 01 [1080p].mkv".into(),
                size_bytes: 1,
                duration_millis: None,
            },
        );
        // Metadata only, no catalog entry: falls back to "series — ep".
        state.set_anidb_metadata(A, ts(3), hash(2), Some(metadata));
        let view = state.view();
        assert_eq!(
            episode_label(&view, &hash(1)),
            "[Judas] Yuuki Yuuna - 01 [1080p].mkv"
        );
        assert_eq!(
            episode_label(&view, &hash(2)),
            "Yuuki Yuuna wa Yuusha de Aru — 01"
        );
    }

    #[test]
    fn episode_rows_multi_copy_shows_distinct_catalog_filenames() {
        use dessplay_core::types::{AniDbSeriesId, FileCatalogEntry};

        let series = AniDbSeriesId(1);
        let mut state = CrdtState::new();
        let names = [
            "[Judas] Yuuki Yuuna - 01 [1080p].mkv",
            "[SubGroup] Yuuki Yuuna - 01.mkv",
            "Yuuki.Yuuna.S01E01.mkv",
        ];
        for (i, name) in names.iter().enumerate() {
            let h = hash(i as u8 + 1);
            state.set_anidb_metadata(A, ts(i as u64 * 2 + 1), h, Some(metadata(series, "01")));
            state.set_file_catalog(
                A,
                ts(i as u64 * 2 + 2),
                h,
                FileCatalogEntry {
                    filename: (*name).into(),
                    size_bytes: 1,
                    duration_millis: None,
                },
            );
        }
        let view = state.view();
        let hashes: Vec<Ed2kHash> = (1..=3).map(hash).collect();
        let rows = episode_rows(&view, &hashes, &BTreeMap::new());
        let filenames: Vec<&str> = rows
            .iter()
            .filter_map(|row| match row {
                EpisodeRow::Child(copy) => Some(copy.filename.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(filenames, names, "each copy must show its own filename");
    }

    /// Sort a list of (episode_number, label) by the episode key and return
    /// the labels in order.
    fn sorted_labels(items: &[(Option<&str>, &str)]) -> Vec<String> {
        let mut items = items.to_vec();
        items.sort_by_key(|a| episode_sort_key(a.0, a.1));
        items
            .into_iter()
            .map(|(_, label)| label.to_string())
            .collect()
    }

    /// Episodes within a season must order by AniDB episode *number*, not by
    /// lexical string or hash order. Regression for the episode browser
    /// listing files in (effectively random) ed2k-hash order.
    #[test]
    fn episodes_sort_numerically_not_lexically() {
        let items = [
            (Some("10"), "ep10"),
            (Some("2"), "ep2"),
            (Some("01"), "ep01"),
        ];
        assert_eq!(sorted_labels(&items), vec!["ep01", "ep2", "ep10"]);
    }

    /// Regular episodes precede specials, which precede credits; each
    /// category orders numerically within itself.
    #[test]
    fn regular_episodes_precede_specials_and_credits() {
        let items = [
            (Some("C1"), "credit1"),
            (Some("S2"), "special2"),
            (Some("3"), "regular3"),
            (Some("S1"), "special1"),
            (Some("1"), "regular1"),
        ];
        assert_eq!(
            sorted_labels(&items),
            vec!["regular1", "regular3", "special1", "special2", "credit1"]
        );
    }

    /// With no episode numbers, ordering falls back to a natural-order parse
    /// of the label: "ep 2" before "ep 10".
    #[test]
    fn unnumbered_episodes_fall_back_to_natural_label_order() {
        let items = [(None, "ep 10"), (None, "ep 2")];
        assert_eq!(sorted_labels(&items), vec!["ep 2", "ep 10"]);
    }

    /// Numbered episodes sort ahead of unnumbered ones in the same season.
    #[test]
    fn numbered_episodes_precede_unnumbered() {
        let items = [(None, "extra"), (Some("01"), "ep01")];
        assert_eq!(sorted_labels(&items), vec!["ep01", "extra"]);
    }

    // ---- franchise_rows: recency / watched-only / filter ----------------

    /// Build a one-file franchise named `title` for series id `id`, then
    /// register the watch timestamp into a recency map if `watched_at` is set.
    fn add_series(
        state: &mut CrdtState,
        recency: &mut BTreeMap<SeriesKey, u64>,
        id: u32,
        title: &str,
        watched_at: Option<u64>,
    ) {
        use dessplay_core::types::{AniDbMetadata, MetadataSource};
        // A distinct file hash per series (id fits in a byte for our tests).
        state.set_anidb_metadata(
            A,
            ts(id as u64),
            hash(id as u8),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: title.into(),
                series_id: Some(AniDbSeriesId(id)),
                episode_number: Some("1".into()),
            }),
        );
        if let Some(t) = watched_at {
            recency.insert(SeriesKey::AniDb(AniDbSeriesId(id)), t);
        }
    }

    fn titles(rows: &[FranchiseRow]) -> Vec<String> {
        rows.iter().map(|r| r.title.clone()).collect()
    }

    /// Recent mode, no filter: only watched franchises, newest first;
    /// unwatched ones are hidden entirely. Regression for the pane rendering
    /// alphabetically and never reflecting a just-watched episode.
    #[test]
    fn recent_shows_only_watched_newest_first() {
        let mut state = CrdtState::new();
        let mut recency = BTreeMap::new();
        add_series(&mut state, &mut recency, 1, "Zelda", Some(300)); // newest
        add_series(&mut state, &mut recency, 2, "Akira", Some(100)); // oldest
        add_series(&mut state, &mut recency, 3, "Monster", Some(200));
        add_series(&mut state, &mut recency, 4, "Berserk", None); // unwatched
        let rows = franchise_rows(&state.view(), SeriesSort::Title, Some(&recency), "");
        assert_eq!(titles(&rows), vec!["Zelda", "Monster", "Akira"]);
    }

    /// A non-empty filter removes the watched-only default and matches
    /// titles case-insensitively, so an unwatched series can be found by
    /// typing. Watched matches still sort ahead of unwatched ones.
    #[test]
    fn recent_filter_reveals_unwatched_case_insensitive() {
        let mut state = CrdtState::new();
        let mut recency = BTreeMap::new();
        add_series(&mut state, &mut recency, 1, "Berserk", None); // unwatched
        add_series(&mut state, &mut recency, 2, "Bersaga", Some(50)); // watched
        add_series(&mut state, &mut recency, 3, "Akira", Some(99)); // no match
        let rows = franchise_rows(&state.view(), SeriesSort::Title, Some(&recency), "BERS");
        // Both "Bers…" match; watched (Bersaga) first, then unwatched Berserk.
        assert_eq!(titles(&rows), vec!["Bersaga", "Berserk"]);
    }

    /// All mode (recency `None`) shows every franchise; a filter narrows by
    /// substring while the title sort is preserved.
    #[test]
    fn all_mode_filter_narrows_and_keeps_sort() {
        let mut state = CrdtState::new();
        let mut recency = BTreeMap::new();
        add_series(&mut state, &mut recency, 1, "Monster", None);
        add_series(&mut state, &mut recency, 2, "Monogatari", None);
        add_series(&mut state, &mut recency, 3, "Akira", None);
        let all = franchise_rows(&state.view(), SeriesSort::Title, None, "");
        assert_eq!(titles(&all), vec!["Akira", "Monogatari", "Monster"]);
        let mono = franchise_rows(&state.view(), SeriesSort::Title, None, "mon");
        assert_eq!(titles(&mono), vec!["Monogatari", "Monster"]);
    }

    fn watched_meta(state: &mut CrdtState, h: u8, id: u32) {
        use dessplay_core::types::{AniDbMetadata, MetadataSource};
        state.set_anidb_metadata(
            A,
            ts(h as u64),
            hash(h),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: format!("series-{id}"),
                series_id: Some(AniDbSeriesId(id)),
                episode_number: Some("1".into()),
            }),
        );
    }

    fn watch_record(h: u8, watched_at: i64) -> crate::storage::WatchRecord {
        crate::storage::WatchRecord {
            hash: hash(h),
            // Frozen null: the file was watched before metadata arrived.
            series_id: None,
            series_name: None,
            filename: format!("ep{h}.mkv"),
            watched_at,
        }
    }

    /// Regression: a file watched *before* its AniDB metadata arrived is
    /// stored with a null series id. Recency must recover the series from
    /// the current metadata view (by hash), or Recent Series stays empty
    /// even though the episode is clearly watched (2026-06-14).
    #[test]
    fn watch_recency_resolves_series_from_live_metadata() {
        let mut state = CrdtState::new();
        watched_meta(&mut state, 1, 7);
        let recency = watch_recency(&[watch_record(1, 100)], &state.view());
        assert_eq!(recency.get(&SeriesKey::AniDb(AniDbSeriesId(7))), Some(&100));
    }

    /// Multiple episodes of one series collapse to the newest watch time.
    #[test]
    fn watch_recency_keeps_newest_per_series() {
        let mut state = CrdtState::new();
        watched_meta(&mut state, 1, 7);
        watched_meta(&mut state, 2, 7);
        // recent_watched yields newest-first; order must not matter.
        let recency = watch_recency(&[watch_record(2, 250), watch_record(1, 100)], &state.view());
        assert_eq!(recency.get(&SeriesKey::AniDb(AniDbSeriesId(7))), Some(&250));
    }

    /// Still no series id anywhere (metadata absent) -> skipped, not panic.
    #[test]
    fn watch_recency_skips_files_with_no_known_series() {
        let state = CrdtState::new();
        let recency = watch_recency(&[watch_record(1, 100)], &state.view());
        assert!(recency.is_empty());
    }

    /// A show AniDB doesn't recognise gets filename-derived metadata (a
    /// series name, no id). Recency must key it by *name* so it matches the
    /// name-keyed franchise, and Recent must then list it. Regression for
    /// Recent staying empty for not-in-AniDB shows (2026-06-15).
    #[test]
    fn recent_shows_name_keyed_franchise_from_filename_derived_metadata() {
        use dessplay_core::types::{AniDbMetadata, MetadataSource};
        let mut state = CrdtState::new();
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(1),
            Some(AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: "Niwatori Fighter".into(),
                series_id: None,
                episode_number: None,
            }),
        );
        let recency = watch_recency(&[watch_record(1, 100)], &state.view());
        assert_eq!(
            recency.get(&SeriesKey::Name("Niwatori Fighter".into())),
            Some(&100)
        );
        // It surfaces in Recent (watched-only) as a name-keyed franchise.
        let rows = franchise_rows(&state.view(), SeriesSort::Title, Some(&recency), "");
        assert_eq!(titles(&rows), vec!["Niwatori Fighter"]);
    }

    proptest::proptest! {
        /// For any recency assignment, the unfiltered Recent view is exactly
        /// the watched franchises, ordered by descending watch time.
        #[test]
        fn recent_unfiltered_is_watched_set_newest_first(
            // (series id, watched_at?) for a handful of distinct series.
            specs in proptest::collection::vec(
                (1u32..200, proptest::option::of(0u64..10_000)),
                1..12,
            )
        ) {
            let mut state = CrdtState::new();
            let mut recency = BTreeMap::new();
            // Deduplicate ids; each surviving series gets a unique title.
            let mut seen = std::collections::BTreeSet::new();
            let mut watched_count = 0usize;
            let mut title_time: BTreeMap<String, u64> = BTreeMap::new();
            for (i, (id, w)) in specs.iter().enumerate() {
                if !seen.insert(*id) {
                    continue;
                }
                let title = format!("S{i:03}");
                add_series(&mut state, &mut recency, *id, &title, *w);
                if let Some(t) = w {
                    watched_count += 1;
                    title_time.insert(title, *t);
                }
            }
            let rows = franchise_rows(&state.view(), SeriesSort::Title, Some(&recency), "");
            // Exactly the watched franchises appear, none of the unwatched.
            proptest::prop_assert_eq!(rows.len(), watched_count);
            // …in non-increasing watch-time order (title tiebreak aside).
            let times: Vec<u64> = rows.iter().map(|r| title_time[&r.title]).collect();
            for win in times.windows(2) {
                proptest::prop_assert!(win[0] >= win[1]);
            }
        }
    }

    proptest::proptest! {
        /// For any permutation of distinct non-negative integers rendered as
        /// zero-padded episode strings, sorting by the episode key yields
        /// ascending numeric order.
        #[test]
        fn numeric_episodes_always_sort_ascending(
            nums in proptest::collection::hash_set(0u64..10_000, 1..30)
        ) {
            let mut nums: Vec<u64> = nums.into_iter().collect();
            // Start from a shuffled-ish order (reverse) to ensure the sort works.
            nums.reverse();
            let labels: Vec<String> = nums.iter().map(|n| format!("{n:05}")).collect();
            let items: Vec<(Option<&str>, &str)> =
                labels.iter().map(|l| (Some(l.as_str()), l.as_str())).collect();
            let mut sorted = nums.clone();
            sorted.sort_unstable();
            let expected: Vec<String> = sorted.iter().map(|n| format!("{n:05}")).collect();
            proptest::prop_assert_eq!(sorted_labels(&items), expected);
        }
    }

    fn metadata(series: AniDbSeriesId, episode: &str) -> AniDbMetadata {
        AniDbMetadata {
            source: MetadataSource::AniDb,
            series_name: "Frieren".into(),
            series_id: Some(series),
            episode_number: Some(episode.into()),
        }
    }

    #[test]
    fn ready_holders_lists_users_advertising_ready_sorted() {
        let mut state = CrdtState::new();
        state.set_file_availability(
            A,
            ts(1),
            UserId::new("nero"),
            hash(1),
            FileAvailability::Ready,
        );
        state.set_file_availability(
            A,
            ts(2),
            UserId::new("kim"),
            hash(1),
            FileAvailability::Ready,
        );
        state.set_file_availability(
            A,
            ts(3),
            UserId::new("baughn"),
            hash(1),
            FileAvailability::Missing,
        );
        // Different file: must not leak in.
        state.set_file_availability(
            A,
            ts(4),
            UserId::new("dagger"),
            hash(2),
            FileAvailability::Ready,
        );
        assert_eq!(
            ready_holders(&state.view(), hash(1)),
            vec![UserId::new("kim"), UserId::new("nero")]
        );
    }

    /// Hand-built browser rows for the cursor-placement helpers: the
    /// grouping itself is `episode_rows`'s business (tested above).
    fn copy(i: u8, filename: &str, watched: bool) -> EpisodeCopy {
        EpisodeCopy {
            hash: hash(i),
            filename: filename.into(),
            holders: vec![],
            watched,
        }
    }

    fn single_row(i: u8, filename: &str, watched: bool) -> EpisodeRow {
        EpisodeRow::Single {
            episode: None,
            copy: copy(i, filename, watched),
        }
    }

    fn header_row(watched: bool) -> EpisodeRow {
        EpisodeRow::Header {
            episode: "Episode".into(),
            watched,
        }
    }

    #[test]
    fn opening_row_is_the_first_unwatched_row_for_single_copies() {
        let empty = BTreeMap::new();
        let view = CrdtState::new().view();
        // Nothing watched: the top of the list.
        let rows = vec![single_row(1, "ep1", false), single_row(2, "ep2", false)];
        assert_eq!(opening_row(&rows, &empty, &view), 0);
        // Episode 1 seen: episode 2.
        let rows = vec![single_row(1, "ep1", true), single_row(2, "ep2", false)];
        assert_eq!(opening_row(&rows, &empty, &view), 1);
        // Everything seen: the top again (nothing to continue to).
        let rows = vec![single_row(1, "ep1", true), single_row(2, "ep2", true)];
        assert_eq!(opening_row(&rows, &empty, &view), 0);
        assert_eq!(opening_row(&[], &empty, &view), 0);
    }

    #[test]
    fn opening_row_picks_the_copy_nearest_the_previously_played_file() {
        // Episode 1 was actually played (personal history) from group A's
        // 1080p release; episode 2 has a B/720p copy listed first and an
        // A/1080p copy second. The cursor skips the header for the A copy.
        let rows = vec![
            single_row(1, "[A] Show - 01 [1080p].mkv", true),
            header_row(false),
            EpisodeRow::Child(copy(2, "[B] Show - 02 [720p].mkv", false)),
            EpisodeRow::Child(copy(3, "[A] Show - 02 [1080p].mkv", false)),
        ];
        let view = CrdtState::new().view();
        let personal: BTreeMap<Ed2kHash, i64> = [(hash(1), 10)].into_iter().collect();
        assert_eq!(opening_row(&rows, &personal, &view), 3);
        // A watched *flag* alone is not evidence of which file played:
        // the header, and the user chooses.
        assert_eq!(opening_row(&rows, &BTreeMap::new(), &view), 1);
        // At the top of the list there is no previous episode at all.
        assert_eq!(opening_row(&rows[1..], &personal, &view), 0);
    }

    #[test]
    fn opening_row_reference_prefers_personal_history_then_the_playlist() {
        // Episode 1 has two copies; episode 2 has two copies mirroring
        // them by name. Which copy of episode 2 the cursor opens on must
        // follow the copy of episode 1 that was actually played.
        let rows = vec![
            header_row(true),
            EpisodeRow::Child(copy(1, "[A] Show - 01.mkv", true)),
            EpisodeRow::Child(copy(2, "[B] Show - 01.mkv", true)),
            header_row(false),
            EpisodeRow::Child(copy(3, "[A] Show - 02.mkv", false)),
            EpisodeRow::Child(copy(4, "[B] Show - 02.mkv", false)),
        ];
        let empty = BTreeMap::new();
        // Both flagged, neither played anywhere we can see: header.
        assert_eq!(opening_row(&rows, &empty, &CrdtState::new().view()), 3);
        // The B copy sits in the group playlist: follow B.
        let mut state = CrdtState::new();
        state.push_playlist_entry(A, ts(1), entry(2, "[B] Show - 01.mkv"));
        let view = state.view();
        assert_eq!(opening_row(&rows, &empty, &view), 5);
        // But a personal record for A outranks playlist presence...
        let personal: BTreeMap<Ed2kHash, i64> = [(hash(1), 10)].into_iter().collect();
        assert_eq!(opening_row(&rows, &personal, &view), 4);
        // ...and between two personal records, the newest wins.
        let personal: BTreeMap<Ed2kHash, i64> =
            [(hash(1), 10), (hash(2), 20)].into_iter().collect();
        assert_eq!(opening_row(&rows, &personal, &view), 5);
    }

    #[test]
    fn next_episode_row_skips_the_current_episodes_copies() {
        let rows = vec![
            single_row(1, "ep1", false),
            header_row(false),
            EpisodeRow::Child(copy(2, "a", false)),
            EpisodeRow::Child(copy(3, "b", false)),
            single_row(4, "ep3", false),
        ];
        assert_eq!(next_episode_row(&rows, 0), Some(1));
        assert_eq!(next_episode_row(&rows, 1), Some(4));
        assert_eq!(next_episode_row(&rows, 2), Some(4));
        assert_eq!(next_episode_row(&rows, 3), Some(4));
        assert_eq!(next_episode_row(&rows, 4), None);
    }

    #[test]
    fn episode_rows_single_copy_per_episode() {
        // Two distinct AniDB episodes, one file each: two Single rows, in
        // episode order regardless of hash-map iteration order.
        let series = AniDbSeriesId(1);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(A, ts(1), hash(2), Some(metadata(series, "2")));
        state.set_anidb_metadata(A, ts(2), hash(1), Some(metadata(series, "1")));
        let view = state.view();
        let rows = episode_rows(&view, &[hash(2), hash(1)], &BTreeMap::new());
        let hashes: Vec<Ed2kHash> = rows.iter().filter_map(EpisodeRow::hash).collect();
        assert_eq!(hashes, vec![hash(1), hash(2)]);
        assert!(
            matches!(&rows[0], EpisodeRow::Single { episode: Some(e), .. } if e == "Episode 1")
        );
        assert!(
            matches!(&rows[1], EpisodeRow::Single { episode: Some(e), .. } if e == "Episode 2")
        );
    }

    #[test]
    fn episode_rows_multi_copy_becomes_header_and_children() {
        // Two files both claiming AniDB episode 3: one Header + two
        // Children, holders attached per copy.
        let series = AniDbSeriesId(1);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(A, ts(1), hash(1), Some(metadata(series, "3")));
        state.set_anidb_metadata(A, ts(2), hash(2), Some(metadata(series, "3")));
        state.set_file_availability(
            A,
            ts(3),
            UserId::new("kim"),
            hash(1),
            FileAvailability::Ready,
        );
        state.set_file_availability(
            A,
            ts(4),
            UserId::new("nero"),
            hash(2),
            FileAvailability::Ready,
        );
        let view = state.view();
        let rows = episode_rows(&view, &[hash(1), hash(2)], &BTreeMap::new());
        assert_eq!(rows.len(), 3);
        assert!(
            matches!(&rows[0], EpisodeRow::Header { episode, watched: false } if episode == "Episode 3")
        );
        let EpisodeRow::Child(a) = &rows[1] else {
            panic!("expected a Child row")
        };
        let EpisodeRow::Child(b) = &rows[2] else {
            panic!("expected a Child row")
        };
        assert_eq!(a.hash, hash(1));
        assert_eq!(a.holders, vec![UserId::new("kim")]);
        assert_eq!(b.hash, hash(2));
        assert_eq!(b.holders, vec![UserId::new("nero")]);
    }

    #[test]
    fn episode_rows_never_merges_unnumbered_files() {
        // Two files with no AniDB episode number: each is a singleton
        // group, never merged just because they're adjacent after sorting.
        let mut state = CrdtState::new();
        // No metadata at all -- both fall back to their playlist filename.
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: ts(1),
            sender: UserId::new("kim"),
            text: "irrelevant".into(),
        });
        let view = state.view();
        let rows = episode_rows(&view, &[hash(1), hash(2)], &BTreeMap::new());
        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|row| matches!(row, EpisodeRow::Single { episode: None, .. }))
        );
    }

    #[test]
    fn episode_rows_muted_by_group_flag_or_personal_history() {
        let series = AniDbSeriesId(1);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(A, ts(1), hash(1), Some(metadata(series, "1")));
        state.set_anidb_metadata(A, ts(2), hash(2), Some(metadata(series, "2")));
        state.set_watched(A, ts(3), hash(1), true); // group flag
        let personally_watched: BTreeMap<Ed2kHash, i64> = [(hash(2), 1)].into_iter().collect(); // personal history
        let view = state.view();
        let rows = episode_rows(&view, &[hash(1), hash(2)], &personally_watched);
        assert!(rows.iter().all(EpisodeRow::watched));
        assert_eq!(first_unwatched(&rows), None);

        // A third, untouched episode breaks the streak.
        state.set_anidb_metadata(A, ts(4), hash(3), Some(metadata(series, "3")));
        let view = state.view();
        let rows = episode_rows(&view, &[hash(1), hash(2), hash(3)], &personally_watched);
        assert_eq!(first_unwatched(&rows), Some(2));
    }

    /// One watched copy marks the whole *episode* watched — the group
    /// saw it; which encoding carried it doesn't matter (user decision
    /// 2026-08-17, reversing the earlier every-copy rule). Individual
    /// copies keep their own per-file marks, and the first-unwatched
    /// marker skips the whole group rather than landing on a leftover
    /// duplicate.
    #[test]
    fn episode_rows_header_watched_when_any_copy_is() {
        let series = AniDbSeriesId(1);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(A, ts(1), hash(1), Some(metadata(series, "1")));
        state.set_anidb_metadata(A, ts(2), hash(2), Some(metadata(series, "1")));
        state.set_anidb_metadata(A, ts(3), hash(3), Some(metadata(series, "2")));
        state.set_watched(A, ts(4), hash(1), true);
        let view = state.view();
        let rows = episode_rows(&view, &[hash(1), hash(2), hash(3)], &BTreeMap::new());
        let EpisodeRow::Header { watched, .. } = &rows[0] else {
            panic!("expected a Header row")
        };
        assert!(watched, "one watched copy must mute the whole episode");
        // Copies stay truthful per file.
        assert!(rows[1].watched());
        assert!(!rows[2].watched());
        // The marker skips the watched episode's unwatched duplicate and
        // lands on the genuinely unwatched next episode.
        assert_eq!(first_unwatched(&rows), Some(3));
    }

    fn unlinked_entry(name: &str, aliases: &[&str], manual_files: &[Ed2kHash]) -> SeriesListEntry {
        SeriesListEntry {
            name: name.into(),
            nero_name: None,
            genre: None,
            notes: Vec::new(),
            recommender: None,
            status: ListStatus::Active,
            status_note: None,
            source: None,
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: aliases.iter().map(|a| a.to_string()).collect(),
            manual_files: manual_files.iter().copied().collect(),
            anidb_unavailable: false,
        }
    }

    fn unknown_metadata(series_name: &str) -> AniDbMetadata {
        AniDbMetadata {
            source: MetadataSource::FilenameDerived,
            series_name: series_name.into(),
            series_id: None,
            episode_number: None,
        }
    }

    fn catalog(filename: &str) -> dessplay_core::types::FileCatalogEntry {
        dessplay_core::types::FileCatalogEntry {
            filename: filename.into(),
            size_bytes: 1,
            duration_millis: None,
        }
    }

    #[test]
    fn candidate_rows_ranks_manual_files_first() {
        let list_entry = unlinked_entry("Some Obscure Show", &[], &[hash(2)]);
        let mut state = CrdtState::new();
        // hash(1): a close name match, but not a manual override.
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(1),
            Some(unknown_metadata("Some Obscure Show")),
        );
        state.set_file_catalog(A, ts(1), hash(1), catalog("Some Obscure Show - 02.mkv"));
        // hash(2): an unrelated derived name, but manually overridden --
        // must still win over a merely name-similar file.
        state.set_anidb_metadata(
            A,
            ts(2),
            hash(2),
            Some(unknown_metadata("Totally Different Title")),
        );
        state.set_file_catalog(
            A,
            ts(2),
            hash(2),
            catalog("Totally Different Title - 02.mkv"),
        );
        let view = state.view();
        let rows = candidate_rows(&view, &list_entry, None);
        let hashes: Vec<_> = rows.iter().filter_map(EpisodeRow::hash).collect();
        assert_eq!(
            hashes,
            vec![hash(2), hash(1)],
            "manual_files must rank first regardless of name distance"
        );
    }

    #[test]
    fn candidate_rows_prefers_the_matching_parsed_episode_number() {
        let list_entry = unlinked_entry("Some Obscure Show", &[], &[]);
        let next_ep = NextEpState {
            next_ep: Some("13".into()),
            available: false,
        };
        let mut state = CrdtState::new();
        // hash(1): right name, wrong episode.
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(1),
            Some(unknown_metadata("Some Obscure Show")),
        );
        state.set_file_catalog(A, ts(1), hash(1), catalog("Some Obscure Show - 12.mkv"));
        // hash(2): right name, right (expected) episode.
        state.set_anidb_metadata(
            A,
            ts(2),
            hash(2),
            Some(unknown_metadata("Some Obscure Show")),
        );
        state.set_file_catalog(A, ts(2), hash(2), catalog("Some Obscure Show - 13.mkv"));
        let view = state.view();
        let rows = candidate_rows(&view, &list_entry, Some(&next_ep));
        let hashes: Vec<_> = rows.iter().filter_map(EpisodeRow::hash).collect();
        assert_eq!(
            hashes,
            vec![hash(2), hash(1)],
            "the file parsing to the expected episode number must rank first"
        );
    }

    #[test]
    fn candidate_rows_matches_via_local_aliases_too() {
        // The derived name doesn't match `name` at all, only a registered
        // alias -- exactly the differently-hinted-file case Series
        // Identity's local_aliases exist for.
        let list_entry = unlinked_entry("Some Obscure Show", &["Some Obscure Show OVA"], &[]);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(1),
            Some(unknown_metadata("Some Obscure Show OVA")),
        );
        state.set_file_catalog(A, ts(1), hash(1), catalog("Some Obscure Show OVA - 01.mkv"));
        let view = state.view();
        let rows = candidate_rows(&view, &list_entry, None);
        assert_eq!(
            rows.iter().filter_map(EpisodeRow::hash).collect::<Vec<_>>(),
            vec![hash(1)]
        );
    }

    #[test]
    fn candidate_rows_excludes_files_too_dissimilar_in_name() {
        let list_entry = unlinked_entry("Some Obscure Show", &[], &[]);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(1),
            Some(unknown_metadata(
                "A Completely Unrelated Series About Something Else",
            )),
        );
        state.set_file_catalog(A, ts(1), hash(1), catalog("Unrelated - 01.mkv"));
        let view = state.view();
        assert!(
            candidate_rows(&view, &list_entry, None).is_empty(),
            "an unrelated file must not become a candidate"
        );
    }

    #[test]
    fn candidate_rows_excludes_already_queued_files() {
        let list_entry = unlinked_entry("Some Obscure Show", &[], &[]);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(1),
            Some(unknown_metadata("Some Obscure Show")),
        );
        state.set_file_catalog(A, ts(1), hash(1), catalog("Some Obscure Show - 02.mkv"));
        state.push_playlist_entry(A, ts(2), entry(1, "Some Obscure Show - 02.mkv"));
        let view = state.view();
        assert!(
            candidate_rows(&view, &list_entry, None).is_empty(),
            "an already-queued file has nothing left to disambiguate"
        );
    }

    #[test]
    fn candidate_rows_empty_when_nothing_is_known_at_all() {
        let list_entry = unlinked_entry("Some Obscure Show", &[], &[]);
        let view = CrdtState::new().view();
        assert!(candidate_rows(&view, &list_entry, None).is_empty());
    }
}
