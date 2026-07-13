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
use dessplay_core::{StateView, franchise};

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
                    Some(FileAvailability::Downloading { progress_bps }) => Some(progress_bps),
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
                    (Some(bps), DerivedUserState::Ready | DerivedUserState::Maybe) => {
                        let label = format!("downloading {}%", bps / 100);
                        if bps >= 2_000 {
                            (label, Tone::Good)
                        } else {
                            (label, Tone::Transfer)
                        }
                    }
                    (Some(bps), _) => (format!("downloading {}%", bps / 100), Tone::Blocked),
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
        let missing = view.file_availability.get(&(me.clone(), entry.hash))
            == Some(&FileAvailability::Missing);
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

/// Format the chat log.
pub fn chat_lines(view: &StateView) -> Vec<ChatLine> {
    view.chat
        .iter()
        .map(|message| {
            // An IRC-style action carries its phrase CTCP-encoded in the
            // text; decode it here so the renderer sees a plain phrase plus
            // the `action` flag.
            let (text, action) = match decode_action(&message.text) {
                Some(phrase) => (phrase.to_string(), true),
                None => (message.text.clone(), false),
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
        text,
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
pub fn subtitle_text(text: &str, speaker: Option<&str>, show_speaker: bool) -> String {
    match speaker.filter(|name| show_speaker && !name.is_empty()) {
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
    speaker: Option<&str>,
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

/// One row of The List.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListRow {
    /// Entry id (selection / edit target).
    pub id: ListEntryId,
    /// Primary title.
    pub name: String,
    /// Nero's title.
    pub nero_name: Option<String>,
    /// Next episode text, with availability marker.
    pub next_ep: Option<String>,
    /// This week's episode is out.
    pub available: bool,
    /// Watcher initials ("BNQ").
    pub watchers: String,
    /// The linked series, if any.
    pub series_id: Option<AniDbSeriesId>,
    /// An AniDB name search for this (unlinked) entry came back with zero
    /// hits (design.md, Series Identity) -- confirmed not on AniDB, not
    /// just never checked.
    pub anidb_unavailable: bool,
}

/// A status group of List rows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ListGroup {
    /// Group heading ("Watching", "Short List", ...).
    pub heading: &'static str,
    /// Rows, in entry-name order.
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
    /// selectable, just names the group. `watched` is true only when
    /// every copy underneath is.
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
    personally_watched: &BTreeSet<Ed2kHash>,
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
            || personally_watched.contains(&entry.hash),
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
                let watched = copies.iter().all(|copy| copy.watched);
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

/// The index of the first not-fully-watched row (design.md #11: the
/// browser's `<` marker and initial cursor placement). `None` when
/// everything is watched.
pub fn first_unwatched(rows: &[EpisodeRow]) -> Option<usize> {
    rows.iter().position(|row| !row.watched())
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

/// The List, grouped per design: Watching (CurrentSeason + Active)
/// first, then ShortList, Planned, Waiting, Hiatus, and a collapsed
/// Finished / Dropped tail.
pub fn list_groups(view: &StateView) -> Vec<ListGroup> {
    let mut groups: Vec<(ListGroup, Vec<ListStatus>)> = [
        (
            "Watching",
            vec![ListStatus::CurrentSeason, ListStatus::Active],
            false,
        ),
        ("Short List", vec![ListStatus::ShortList], false),
        ("Planned", vec![ListStatus::Planned], false),
        ("Waiting", vec![ListStatus::Waiting], false),
        ("Hiatus", vec![ListStatus::Hiatus], false),
        (
            "Finished / Dropped",
            vec![ListStatus::Finished, ListStatus::Dropped],
            true,
        ),
    ]
    .into_iter()
    .map(|(heading, statuses, collapsed)| {
        (
            ListGroup {
                heading,
                rows: Vec::new(),
                collapsed,
            },
            statuses,
        )
    })
    .collect();

    for (id, entry) in &view.list_entries {
        let Some((group, _)) = groups
            .iter_mut()
            .find(|(_, statuses)| statuses.contains(&entry.status))
        else {
            continue;
        };
        let next = view.list_next_ep.get(id);
        group.rows.push(ListRow {
            id: *id,
            name: entry.name.clone(),
            nero_name: entry.nero_name.clone(),
            next_ep: next.and_then(|n| n.next_ep.clone()),
            available: next.is_some_and(|n| n.available),
            watchers: entry
                .watchers
                .iter()
                .filter_map(|user| user.0.chars().next())
                .map(|c| c.to_ascii_uppercase())
                .collect(),
            series_id: entry.anidb_series_id,
            anidb_unavailable: entry.anidb_unavailable,
        });
    }
    for (group, _) in &mut groups {
        group.rows.sort_by(|a, b| a.name.cmp(&b.name));
    }
    groups
        .into_iter()
        .map(|(group, _)| group)
        .filter(|group| !group.rows.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use dessplay_core::CrdtState;
    use dessplay_core::playlist::NewPlaylistEntry;
    use dessplay_core::types::{
        ActorId, AniDbMetadata, AniDbSeriesId, ListEntryId, ListStatus, ManualState,
        MetadataSource, PlaybackIntent, SeriesListEntry, SeriesWatchState, SharedTimestamp,
    };

    const A: ActorId = ActorId::SERVER;

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
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

    #[test]
    fn subtitle_text_prefixes_only_named_opted_in_cues() {
        assert_eq!(
            subtitle_text("Hello", Some("Frieren"), true),
            "Frieren: Hello"
        );
        assert_eq!(subtitle_text("Hello", Some("Frieren"), false), "Hello");
        assert_eq!(subtitle_text("Hello", None, true), "Hello");
        assert_eq!(subtitle_text("Hello", Some(""), true), "Hello");
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
        let peers = [
            peer("kim", Role::Interactive, Presence::Present),
            peer("paused", Role::Interactive, Presence::Present),
            peer("afk", Role::Interactive, Presence::Present),
            peer("downloader", Role::Interactive, Presence::Present),
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

    #[test]
    fn list_groups_follow_design_order() {
        let mut state = CrdtState::new();
        let mut put = |id: u128, name: &str, status: ListStatus| {
            state.put_list_entry(
                A,
                ts(id as u64),
                ListEntryId(id),
                SeriesListEntry {
                    name: name.into(),
                    nero_name: None,
                    genre: None,
                    notes: vec![],
                    recommender: None,
                    status,
                    status_note: None,
                    source: None,
                    watchers: [UserId::new("Baughn"), UserId::new("Nero")]
                        .into_iter()
                        .collect(),
                    anidb_series_id: None,
                    local_aliases: Default::default(),
                    manual_files: Default::default(),
                    anidb_unavailable: false,
                },
            );
        };
        put(1, "Airing", ListStatus::CurrentSeason);
        put(2, "Binging", ListStatus::Active);
        put(3, "Done", ListStatus::Finished);
        put(4, "Up next", ListStatus::ShortList);
        state.set_next_ep(
            A,
            ts(10),
            ListEntryId(1),
            dessplay_core::types::NextEpState {
                next_ep: Some("12".into()),
                available: true,
            },
        );

        let groups = list_groups(&state.view());
        let headings: Vec<&str> = groups.iter().map(|g| g.heading).collect();
        assert_eq!(
            headings,
            vec!["Watching", "Short List", "Finished / Dropped"]
        );
        let watching = &groups[0];
        assert_eq!(watching.rows.len(), 2);
        assert_eq!(watching.rows[0].name, "Airing");
        assert_eq!(watching.rows[0].next_ep.as_deref(), Some("12"));
        assert!(watching.rows[0].available);
        assert_eq!(watching.rows[0].watchers, "BN");
        assert!(groups.last().unwrap().collapsed);
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
        let rows = episode_rows(&view, &hashes, &BTreeSet::new());
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

    #[test]
    fn episode_rows_single_copy_per_episode() {
        // Two distinct AniDB episodes, one file each: two Single rows, in
        // episode order regardless of hash-map iteration order.
        let series = AniDbSeriesId(1);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(A, ts(1), hash(2), Some(metadata(series, "2")));
        state.set_anidb_metadata(A, ts(2), hash(1), Some(metadata(series, "1")));
        let view = state.view();
        let rows = episode_rows(&view, &[hash(2), hash(1)], &BTreeSet::new());
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
        let rows = episode_rows(&view, &[hash(1), hash(2)], &BTreeSet::new());
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
        let rows = episode_rows(&view, &[hash(1), hash(2)], &BTreeSet::new());
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
        let personally_watched: BTreeSet<Ed2kHash> = [hash(2)].into_iter().collect(); // personal history
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

    #[test]
    fn episode_rows_header_watched_only_when_every_copy_is() {
        let series = AniDbSeriesId(1);
        let mut state = CrdtState::new();
        state.set_anidb_metadata(A, ts(1), hash(1), Some(metadata(series, "1")));
        state.set_anidb_metadata(A, ts(2), hash(2), Some(metadata(series, "1")));
        state.set_watched(A, ts(3), hash(1), true);
        let view = state.view();
        let rows = episode_rows(&view, &[hash(1), hash(2)], &BTreeSet::new());
        let EpisodeRow::Header { watched, .. } = &rows[0] else {
            panic!("expected a Header row")
        };
        assert!(!watched, "one unwatched copy must keep the group unwatched");
        assert_eq!(first_unwatched(&rows), Some(0));
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
