//! State -> Props: the pure mapping from (resolved view, peer list,
//! local context) to what each pane displays. Components render these
//! verbatim; keeping the mapping pure makes the display rules testable
//! without a terminal (ui-architecture.md, State to Props Mapping).

use std::collections::{BTreeMap, BTreeSet};

use dessplay_core::derive::{self, DerivedUserState};
use dessplay_core::net::{PeerInfo, Presence, Role};
use dessplay_core::types::{
    AniDbSeriesId, Ed2kHash, FileAvailability, ListEntryId, ListStatus, UserId,
};
use dessplay_core::{StateView, franchise};

use crate::storage::SeriesKey;

/// Semantic display tone; the theme maps these to colors.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Green: ready / downloading-but-playable.
    Good,
    /// Red: blocking playback (paused, missing, lost).
    Blocked,
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

/// Users pane props: active rows plus the dim departed/seeder lines.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct UsersProps {
    /// Present and Lost interactive users.
    pub rows: Vec<UserRow>,
    /// Departed usernames (dim line).
    pub departed: Vec<String>,
    /// Seeders (dim line), with a marker when not present.
    pub seeders: Vec<String>,
}

/// Build the users pane from the design's ready-state table.
pub fn users_props(view: &StateView, peers: &[PeerInfo]) -> UsersProps {
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
            Presence::Departed => props.departed.push(name),
            Presence::Lost => props.rows.push(UserRow {
                name,
                label: "lost".into(),
                tone: Tone::Blocked,
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
                let (label, tone) = match (downloading, &state) {
                    (Some(bps), DerivedUserState::Ready) => {
                        let label = format!("downloading {}%", bps / 100);
                        if bps >= 2_000 {
                            (label, Tone::Good)
                        } else {
                            (label, Tone::Transfer)
                        }
                    }
                    (Some(bps), _) => (format!("downloading {}%", bps / 100), Tone::Blocked),
                    (None, DerivedUserState::Paused) => ("paused".to_string(), Tone::Blocked),
                    (None, DerivedUserState::Away { set_by }) => {
                        (format!("away, set by {set_by}"), Tone::Idle)
                    }
                    (None, DerivedUserState::NotWatching) => {
                        ("not watching".to_string(), Tone::Idle)
                    }
                    // Ready, not downloading: Downloading is impossible
                    // here (it would be `Some` above), so `_` is ready.
                    (None, DerivedUserState::Ready) => match avail {
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
    props
}

fn availability(view: &StateView, user: &UserId) -> Option<FileAvailability> {
    let file = view.now_playing?;
    view.file_availability.get(&(user.clone(), file)).copied()
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
        });
    }
    props
}

// ---- Chat pane -------------------------------------------------------

/// One formatted chat line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ChatLine {
    /// "HH:MM" on the shared clock (UTC).
    pub time: String,
    /// Sender name (empty for system lines).
    pub sender: String,
    /// Message body.
    pub text: String,
    /// A local system line (archive result, etc.): rendered dim with no
    /// sender, never synced.
    pub system: bool,
    /// Shared-clock millis, used only to interleave local system lines
    /// with synced messages.
    pub millis: u64,
}

/// Format the chat log.
pub fn chat_lines(view: &StateView) -> Vec<ChatLine> {
    view.chat
        .iter()
        .map(|message| ChatLine {
            time: hhmm(message.timestamp.0),
            sender: message.sender.to_string(),
            text: message.text.clone(),
            system: false,
            millis: message.timestamp.0,
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
        millis: timestamp,
    }
}

/// Unix millis -> "HH:MM" (UTC; good enough until a tz dependency is
/// justified).
fn hhmm(millis: u64) -> String {
    let minutes = (millis / 60_000) % (24 * 60);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

// ---- Player status ---------------------------------------------------

/// Player status bar props.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct StatusProps {
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
pub fn status_props(view: &StateView, peers: &[PeerInfo], me: &UserId) -> StatusProps {
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
                derive::BlockReason::Lost => "lost",
            };
            format!("{} ({reason})", blocker.user)
        })
        .collect();
    StatusProps {
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
        let meta = view.anidb_metadata.get(&record.hash).and_then(|m| m.as_ref());
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
pub fn franchise_rows(
    view: &StateView,
    sort: SeriesSort,
    recency: Option<&BTreeMap<SeriesKey, u64>>,
    filter: &str,
) -> Vec<FranchiseRow> {
    let mut rows: Vec<(Option<u64>, FranchiseRow)> = franchise::franchises(view)
        .into_iter()
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
                    key: franchise.key,
                    title: franchise.title,
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

/// A human-readable label for a file in the episode browser. Prefers the
/// playlist entry's filename (the real on-disk name); falls back to the
/// AniDB metadata's "series — episode" when the file is known to AniDB
/// but not in the playlist; then to the file catalog's filename (a
/// library file we don't hold, before metadata arrives); only then to the
/// raw hash.
pub fn episode_label(view: &StateView, hash: &Ed2kHash) -> String {
    if let Some(entry) = view.playlist.iter().find(|entry| entry.hash == *hash) {
        return entry.state.filename.clone();
    }
    if let Some(Some(metadata)) = view.anidb_metadata.get(hash) {
        return match &metadata.episode_number {
            Some(ep) => format!("{} — {}", metadata.series_name, ep),
            None => metadata.series_name.clone(),
        };
    }
    if let Some(entry) = view.file_catalog.get(hash) {
        return entry.filename.clone();
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
    let parsed = episode_number.and_then(|epno| {
        let epno = epno.trim();
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
            .fold(0u64, |n, d| n.saturating_mul(10).saturating_add(u64::from(d)));
        let category = match prefix.to_ascii_uppercase().as_str() {
            "" => 0,
            "S" => 1,
            "C" => 2,
            "T" => 3,
            "P" => 4,
            _ => 5,
        };
        Some((category, number))
    });
    match parsed {
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
        ActorId, AniDbMetadata, AniDbSeriesId, ManualState, MetadataSource, PlaybackIntent,
        SeriesListEntry, SeriesWatchState, SharedTimestamp,
    };

    const A: ActorId = ActorId::SERVER;

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
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
        let props = users_props(&state.view(), &peers);

        let by_name: BTreeMap<&str, &UserRow> = props
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row))
            .collect();
        assert_eq!(by_name["kim"].tone, Tone::Good);
        assert_eq!(by_name["paused"].tone, Tone::Blocked);
        assert_eq!(by_name["afk"].label, "away, set by kim");
        assert_eq!(by_name["afk"].tone, Tone::Idle);
        assert_eq!(by_name["downloader"].label, "downloading 15%");
        assert_eq!(by_name["downloader"].tone, Tone::Transfer);
        assert_eq!(by_name["lacking"].tone, Tone::Blocked);
        assert_eq!(by_name["ghost"].label, "lost");
        assert_eq!(props.departed, vec!["gone"]);
        assert_eq!(props.seeders, vec!["nas"]);
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
        state.set_series_preference(
            A,
            ts(3),
            UserId::new("ndl"),
            AniDbSeriesId(7),
            SeriesWatchState::NotWatching,
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
        let props = users_props(&state.view(), &peers);
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
        let props = status_props(&state.view(), &peers, &UserId::new("kim"));
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
    fn hhmm_is_utc() {
        assert_eq!(hhmm(0), "00:00");
        assert_eq!(hhmm(13 * 3_600_000 + 37 * 60_000 + 12_345), "13:37");
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
        // hash(3): metadata with no episode number -> just the name.
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

    /// Sort a list of (episode_number, label) by the episode key and return
    /// the labels in order.
    fn sorted_labels(items: &[(Option<&str>, &str)]) -> Vec<String> {
        let mut items = items.to_vec();
        items.sort_by_key(|a| episode_sort_key(a.0, a.1));
        items.into_iter().map(|(_, label)| label.to_string()).collect()
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
}
