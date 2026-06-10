//! State -> Props: the pure mapping from (resolved view, peer list,
//! local context) to what each pane displays. Components render these
//! verbatim; keeping the mapping pure makes the display rules testable
//! without a terminal (ui-architecture.md, State to Props Mapping).

use std::collections::BTreeMap;

use dessplay_core::derive::{self, DerivedUserState};
use dessplay_core::net::{PeerInfo, Presence, Role};
use dessplay_core::types::{
    AniDbSeriesId, Ed2kHash, FileAvailability, ListEntryId, ListStatus, UserId,
};
use dessplay_core::{StateView, franchise};

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
                let (label, tone) = match derive::user_state(view, &peer.username) {
                    DerivedUserState::Paused => ("paused".to_string(), Tone::Blocked),
                    DerivedUserState::Away { set_by } => {
                        (format!("away, set by {set_by}"), Tone::Idle)
                    }
                    DerivedUserState::NotWatching => ("not watching".to_string(), Tone::Idle),
                    DerivedUserState::Ready => match availability(view, &peer.username) {
                        None | Some(FileAvailability::Ready) => ("ready".to_string(), Tone::Good),
                        Some(FileAvailability::Missing) => {
                            ("missing file".to_string(), Tone::Blocked)
                        }
                        Some(FileAvailability::Downloading { progress_bps }) => {
                            let label = format!("downloading {}%", progress_bps / 100);
                            if progress_bps >= 2_000 {
                                (label, Tone::Good)
                            } else {
                                (label, Tone::Transfer)
                            }
                        }
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
/// entries muted (play history), files *we* lack in red.
pub fn playlist_props(view: &StateView, me: &UserId) -> PlaylistProps {
    let mut props = PlaylistProps::default();
    for (index, entry) in view.playlist.iter().enumerate() {
        let is_now = view.now_playing == Some(entry.hash);
        let missing = view.file_availability.get(&(me.clone(), entry.hash))
            == Some(&FileAvailability::Missing);
        let watched = view.watched.get(&entry.hash) == Some(&true);
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
    /// Sender name.
    pub sender: String,
    /// Message body.
    pub text: String,
}

/// Format the chat log.
pub fn chat_lines(view: &StateView) -> Vec<ChatLine> {
    view.chat
        .iter()
        .map(|message| ChatLine {
            time: hhmm(message.timestamp.0),
            sender: message.sender.to_string(),
            text: message.text.clone(),
        })
        .collect()
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

/// Franchise rows for the Recent / All modes. `recency` maps series to
/// last-watched shared-clock millis (from local watch history); Recent
/// sorts unwatched-franchise-first... — for Phase 6, most recently
/// watched first, then title (unwatched-first needs Phase 9's local
/// file knowledge).
pub fn franchise_rows(
    view: &StateView,
    sort: SeriesSort,
    recency: Option<&BTreeMap<AniDbSeriesId, u64>>,
) -> Vec<FranchiseRow> {
    let mut rows: Vec<(Option<u64>, FranchiseRow)> = franchise::franchises(view)
        .into_iter()
        .map(|franchise| {
            let last_watched = recency.and_then(|map| {
                franchise
                    .series
                    .iter()
                    .filter_map(|id| map.get(id).copied())
                    .max()
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
        ActorId, ManualState, PlaybackIntent, SeriesListEntry, SharedTimestamp,
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
        let props = playlist_props(&state.view(), &UserId::new("kim"));
        assert_eq!(props.now_index, Some(1));
        assert_eq!(
            props.rows.iter().map(|r| r.tone).collect::<Vec<_>>(),
            vec![Tone::Muted, Tone::Good, Tone::Blocked]
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
}
