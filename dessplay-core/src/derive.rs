//! Derived state: pure functions from (resolved view, peer list) to the
//! product-level facts the UI, the player layer, and the server all
//! need — a user's effective state, and whether video actually plays.
//!
//! Everything here quantifies over the **peer list** (the server's
//! presence view), not over the CRDT maps: a user with no peer entry does
//! not exist for gating purposes, and seeders never gate anything. A
//! departed user's replicated state is ignored **unless** they are
//! *committed* (Watching) to the now-playing series — a commitment gates
//! across absence ("wait for me even if I've been gone a week"), cleared
//! only by their return or a per-file [acknowledge](StateView::acknowledged_absent).
//!
//! See docs/design.md (User States, Playback Rules, Presence).

use crate::net::{PeerInfo, Presence, Role};
use crate::state::StateView;
use crate::types::{FileAvailability, ManualState, PlaybackIntent, SeriesWatchState, UserId};

/// A user's effective state, derived from their manual override and
/// their watch preference for the now-playing file's series.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DerivedUserState {
    /// No override, **committed** (Watching) to the current series: gates
    /// playback, and keeps gating even while absent.
    Ready,
    /// No override, **Maybe** (the default) on the current series: gates
    /// only while present.
    Maybe,
    /// Manual override: blocks playback.
    Paused,
    /// Marked away by someone else: does not block playback.
    Away {
        /// Who set it, for display.
        set_by: UserId,
    },
    /// The now-playing file's series is marked NotWatching: does not
    /// block playback.
    NotWatching,
}

/// Why playback is blocked, per blocking user.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BlockReason {
    /// Manual pause.
    Paused,
    /// File absent or hash-mismatched.
    FileMissing,
    /// Still downloading, not yet complete enough to play.
    Downloading,
    /// A **committed** (Watching) user is absent (Lost or Departed) and
    /// has not been acknowledged for the current file. Replaces the old
    /// blanket "Lost" reason: an absent Maybe user no longer blocks, so
    /// the only absent-and-blocking case is a committed one.
    CommittedAbsent,
}

/// One user blocking playback, with the reason — feeds the OSD summary
/// and the Users pane.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Blocker {
    /// Who.
    pub user: UserId,
    /// Why.
    pub reason: BlockReason,
}

/// A user's effective commitment to the series of an arbitrary `file`
/// (not necessarily now-playing): the stored preference, or the default
/// [`SeriesWatchState::Maybe`] when there is no preference or no List
/// entry claims the file yet (design.md, Series Identity -- AniDB linking
/// is enrichment only, never required for this). Used by prefetch gating
/// and the playlist watch tag as well as [`effective_watch`].
pub fn series_watch_for_file(
    view: &StateView,
    user: &UserId,
    file: crate::types::Ed2kHash,
) -> SeriesWatchState {
    let Some(entry) = crate::series_identity::resolve_series_entry_for_file(view, file) else {
        return SeriesWatchState::Maybe;
    };
    view.series_preference
        .get(&(user.clone(), entry))
        .map(|pref| pref.state)
        .unwrap_or(SeriesWatchState::Maybe)
}

/// A user's effective commitment to the **now-playing** file's series
/// (the gating-relevant one). Maybe when nothing is playing.
fn effective_watch(view: &StateView, user: &UserId) -> SeriesWatchState {
    match view.now_playing {
        Some(file) => series_watch_for_file(view, user, file),
        None => SeriesWatchState::Maybe,
    }
}

/// Derive a user's effective state. The manual override wins; otherwise
/// the now-playing file's series commitment decides (default Maybe).
pub fn user_state(view: &StateView, user: &UserId) -> DerivedUserState {
    match view.manual_override.get(user) {
        Some(Some(ManualState::Paused)) => return DerivedUserState::Paused,
        Some(Some(ManualState::Away { set_by })) => {
            return DerivedUserState::Away {
                set_by: set_by.clone(),
            };
        }
        _ => {}
    }
    match effective_watch(view, user) {
        SeriesWatchState::Watching => DerivedUserState::Ready,
        SeriesWatchState::Maybe => DerivedUserState::Maybe,
        SeriesWatchState::NotWatching => DerivedUserState::NotWatching,
    }
}

/// Why this user's file state blocks playback of now-playing, if it
/// does.
///
/// An *unreported* availability permits: until the file actor (Phase 9)
/// writes `Missing` promptly on a failed match, blocking on absence-of-
/// data would deadlock every session. `Downloading` permits at >= 20%;
/// the design's "download speed exceeds bitrate" half of the rule is
/// only knowable by the downloading client and arrives with the
/// transfer machinery in Phase 9.
fn file_block_reason(view: &StateView, user: &UserId) -> Option<BlockReason> {
    let file = view.now_playing?;
    match view.file_availability.get(&(user.clone(), file)) {
        None | Some(FileAvailability::Ready) => None,
        Some(FileAvailability::Missing) => Some(BlockReason::FileMissing),
        Some(FileAvailability::Downloading { progress_bps }) => {
            (*progress_bps < 2_000).then_some(BlockReason::Downloading)
        }
    }
}

/// Does this user's file state permit playback of now-playing?
/// See [`file_block_reason`] for the rules.
pub fn file_permits(view: &StateView, user: &UserId) -> bool {
    file_block_reason(view, user).is_none()
}

/// Why a *present* user (committed or Maybe) blocks now-playing: a manual
/// pause beats their file state, otherwise the file state decides.
fn present_block_reason(
    view: &StateView,
    user: &UserId,
    manual: Option<&ManualState>,
) -> Option<BlockReason> {
    if matches!(manual, Some(ManualState::Paused)) {
        Some(BlockReason::Paused)
    } else {
        file_block_reason(view, user)
    }
}

/// Everyone currently blocking playback, with reasons. Empty means
/// gating permits (intent and now-playing are checked separately by
/// [`playback_active`]).
///
/// Quantifies over interactive peers only (seeders never gate; a user
/// with no peer entry is ignored). The matrix over (commitment ×
/// presence), with the manual override mixed in:
///
/// - **Away** (any presence) never blocks — also the manual escape hatch.
/// - **NotWatching** (any presence) never blocks.
/// - **Watching** (committed): present → manual-pause / file state;
///   absent (Lost/Departed) → `CommittedAbsent`, unless `(now-playing,
///   user)` is in `acknowledged_absent`.
/// - **Maybe** (default): present → manual-pause / file state; absent →
///   never blocks.
pub fn playback_blockers(view: &StateView, peers: &[PeerInfo]) -> Vec<Blocker> {
    let now_playing = view.now_playing;
    let mut blockers = Vec::new();
    for peer in peers {
        if peer.role != Role::Interactive {
            continue;
        }
        let user = &peer.username;
        let manual = view.manual_override.get(user).and_then(|m| m.as_ref());

        // Away never blocks, at any presence (it is how an absent person
        // is excused, and how a committed-absent block is acknowledged via
        // the per-user route — the per-file route is below).
        if matches!(manual, Some(ManualState::Away { .. })) {
            continue;
        }

        let reason = match effective_watch(view, user) {
            SeriesWatchState::NotWatching => None,
            SeriesWatchState::Maybe => match peer.presence {
                Presence::Present => present_block_reason(view, user, manual),
                Presence::Lost | Presence::Departed => None,
            },
            SeriesWatchState::Watching => match peer.presence {
                Presence::Present => present_block_reason(view, user, manual),
                Presence::Lost | Presence::Departed => {
                    let acknowledged = now_playing.is_some_and(|file| {
                        view.acknowledged_absent.contains(&(file, user.clone()))
                    });
                    (!acknowledged).then_some(BlockReason::CommittedAbsent)
                }
            },
        };
        if let Some(reason) = reason {
            blockers.push(Blocker {
                user: user.clone(),
                reason,
            });
        }
    }
    blockers
}

/// The derived playback state: video plays iff the intent latch is
/// `Playing`, something is queued as now-playing, and nobody blocks.
pub fn playback_active(view: &StateView, peers: &[PeerInfo]) -> bool {
    view.playback_intent == PlaybackIntent::Playing
        && view.now_playing.is_some()
        && playback_blockers(view, peers).is_empty()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use crate::state::CrdtState;
    use crate::types::{
        ActorId, AniDbMetadata, AniDbSeriesId, Ed2kHash, ListEntryId, ListStatus, MetadataSource,
        SeriesListEntry, SharedTimestamp,
    };

    const SERVER: ActorId = ActorId::SERVER;

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    /// Link a List entry to `series` so `series_watch_for_file`'s
    /// resolution (design.md, Series Identity) can find preferences
    /// written against it. Tests key preferences on the returned
    /// `ListEntryId`, not the raw `AniDbSeriesId`, mirroring how a real
    /// AniDB-linked series always has a backing entry.
    fn link_series(
        state: &mut CrdtState,
        ts: SharedTimestamp,
        series: AniDbSeriesId,
    ) -> ListEntryId {
        let id = ListEntryId(series.0 as u128);
        state.put_list_entry(
            SERVER,
            ts,
            id,
            SeriesListEntry {
                name: "Frieren".into(),
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

    fn peer(name: &str, role: Role, presence: Presence) -> PeerInfo {
        PeerInfo {
            username: UserId::new(name),
            role,
            presence,
            addresses: vec![],
            connected_since: 0,
        }
    }

    fn present(name: &str) -> PeerInfo {
        peer(name, Role::Interactive, Presence::Present)
    }

    /// A state with now-playing set and intent Playing: the baseline
    /// where everything else permits playback.
    fn playing_state() -> CrdtState {
        let mut state = CrdtState::new();
        state.set_now_playing(SERVER, ts(1), Some(hash(1)));
        state.set_playback_intent(SERVER, ts(2), PlaybackIntent::Playing);
        state
    }

    #[test]
    fn fresh_state_defaults_to_paused() {
        let view = CrdtState::new().view();
        assert_eq!(view.playback_intent, PlaybackIntent::Paused);
        assert!(!playback_active(&view, &[present("kim")]));
    }

    #[test]
    fn all_ready_and_intent_playing_plays() {
        let view = playing_state().view();
        let peers = [present("kim"), present("baughn")];
        assert!(playback_blockers(&view, &peers).is_empty());
        assert!(playback_active(&view, &peers));
    }

    #[test]
    fn intent_paused_blocks_even_when_everyone_is_ready() {
        let mut state = playing_state();
        state.set_playback_intent(SERVER, ts(3), PlaybackIntent::Paused);
        let view = state.view();
        assert!(playback_blockers(&view, &[present("kim")]).is_empty());
        assert!(!playback_active(&view, &[present("kim")]));
    }

    #[test]
    fn nothing_queued_means_nothing_plays() {
        let mut state = CrdtState::new();
        state.set_playback_intent(SERVER, ts(1), PlaybackIntent::Playing);
        assert!(!playback_active(&state.view(), &[present("kim")]));
    }

    #[test]
    fn manual_pause_blocks_with_attribution() {
        let mut state = playing_state();
        state.set_manual_override(SERVER, ts(3), UserId::new("kim"), Some(ManualState::Paused));
        let view = state.view();
        assert_eq!(
            user_state(&view, &UserId::new("kim")),
            DerivedUserState::Paused
        );
        assert_eq!(
            playback_blockers(&view, &[present("kim"), present("baughn")]),
            vec![Blocker {
                user: UserId::new("kim"),
                reason: BlockReason::Paused,
            }]
        );
        assert!(!playback_active(
            &view,
            &[present("kim"), present("baughn")]
        ));
    }

    #[test]
    fn away_does_not_block_and_carries_attribution() {
        let mut state = playing_state();
        state.set_manual_override(
            SERVER,
            ts(3),
            UserId::new("kim"),
            Some(ManualState::Away {
                set_by: UserId::new("baughn"),
            }),
        );
        let view = state.view();
        assert_eq!(
            user_state(&view, &UserId::new("kim")),
            DerivedUserState::Away {
                set_by: UserId::new("baughn")
            }
        );
        assert!(playback_active(&view, &[present("kim"), present("baughn")]));
    }

    #[test]
    fn not_watching_series_does_not_block() {
        let series = AniDbSeriesId(42);
        let mut state = playing_state();
        state.set_anidb_metadata(
            SERVER,
            ts(3),
            hash(1),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Frieren".into(),
                series_id: Some(series),
                episode_number: Some("1".into()),
            }),
        );
        let entry = link_series(&mut state, ts(3), series);
        state.set_series_preference(
            SERVER,
            ts(4),
            UserId::new("kim"),
            entry,
            SeriesWatchState::NotWatching,
            None,
        );
        // Kim is even Missing the file — irrelevant, she isn't watching.
        state.set_file_availability(
            SERVER,
            ts(5),
            UserId::new("kim"),
            hash(1),
            FileAvailability::Missing,
        );
        let view = state.view();
        assert_eq!(
            user_state(&view, &UserId::new("kim")),
            DerivedUserState::NotWatching
        );
        assert!(playback_active(&view, &[present("kim"), present("baughn")]));
    }

    #[test]
    fn manual_pause_overrides_not_watching() {
        let series = AniDbSeriesId(42);
        let mut state = playing_state();
        state.set_anidb_metadata(
            SERVER,
            ts(3),
            hash(1),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Frieren".into(),
                series_id: Some(series),
                episode_number: None,
            }),
        );
        let entry = link_series(&mut state, ts(3), series);
        state.set_series_preference(
            SERVER,
            ts(4),
            UserId::new("kim"),
            entry,
            SeriesWatchState::NotWatching,
            None,
        );
        state.set_manual_override(SERVER, ts(5), UserId::new("kim"), Some(ManualState::Paused));
        let view = state.view();
        // Display side: the manual Paused override wins for `user_state`.
        assert_eq!(
            user_state(&view, &UserId::new("kim")),
            DerivedUserState::Paused
        );
        // Gating side (the subtle precedence inversion): `playback_blockers`
        // short-circuits NotWatching to None *before* the manual-pause check,
        // so a *present* NotWatching user never gates playback even when also
        // manually Paused (design.md User States: "NotWatching … never gates
        // playback on it, present or absent"). This is the opposite of the
        // display side above, and was previously unasserted.
        assert!(
            playback_blockers(&view, &[present("kim")]).is_empty(),
            "a present NotWatching user must not block, even when manually Paused"
        );
        assert!(playback_active(&view, &[present("kim")]));
    }

    #[test]
    fn file_states_gate_watching_users() {
        for (availability, expected) in [
            (FileAvailability::Ready, None),
            (FileAvailability::Missing, Some(BlockReason::FileMissing)),
            (
                FileAvailability::Downloading {
                    progress_bps: 1_999,
                },
                Some(BlockReason::Downloading),
            ),
            (
                FileAvailability::Downloading {
                    progress_bps: 2_000,
                },
                None,
            ),
        ] {
            let mut state = playing_state();
            state.set_file_availability(SERVER, ts(3), UserId::new("kim"), hash(1), availability);
            let blockers = playback_blockers(&state.view(), &[present("kim")]);
            match expected {
                None => assert!(blockers.is_empty(), "{availability:?} should permit"),
                Some(reason) => {
                    assert_eq!(blockers.len(), 1, "{availability:?} should block");
                    assert_eq!(blockers[0].reason, reason);
                }
            }
        }
    }

    #[test]
    fn unreported_availability_permits() {
        // Nobody has written availability at all (pre-Phase-9 reality):
        // playback must not deadlock on absent data.
        let view = playing_state().view();
        assert!(file_permits(&view, &UserId::new("kim")));
        assert!(playback_active(&view, &[present("kim")]));
    }

    const SERIES: AniDbSeriesId = AniDbSeriesId(42);

    /// `playing_state` with now-playing hash(1) carrying AniDB metadata for
    /// [`SERIES`], then each `(user, pref)` written. Lets a test put a user
    /// at a specific commitment to the now-playing series.
    fn committed_state(prefs: &[(&str, SeriesWatchState)]) -> CrdtState {
        let mut state = playing_state();
        state.set_anidb_metadata(
            SERVER,
            ts(3),
            hash(1),
            Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Frieren".into(),
                series_id: Some(SERIES),
                episode_number: Some("1".into()),
            }),
        );
        let entry = link_series(&mut state, ts(3), SERIES);
        for (i, (user, pref)) in prefs.iter().enumerate() {
            state.set_series_preference(
                SERVER,
                ts(10 + i as u64),
                UserId::new(*user),
                entry,
                *pref,
                None,
            );
        }
        state
    }

    #[test]
    fn user_state_defaults_to_maybe() {
        // No preference, no metadata — the opportunistic default.
        let view = playing_state().view();
        assert_eq!(
            user_state(&view, &UserId::new("kim")),
            DerivedUserState::Maybe
        );
    }

    #[test]
    fn absent_maybe_user_does_not_block() {
        // The new rule: a Lost/Departed Maybe (default) user never blocks.
        let view = playing_state().view();
        for presence in [Presence::Lost, Presence::Departed] {
            let peers = [present("kim"), peer("baughn", Role::Interactive, presence)];
            assert!(
                playback_blockers(&view, &peers).is_empty(),
                "absent Maybe ({presence:?}) must not block"
            );
            assert!(playback_active(&view, &peers));
        }
    }

    #[test]
    fn absent_committed_user_blocks() {
        // A committed (Watching) user gates across absence.
        let view = committed_state(&[("baughn", SeriesWatchState::Watching)]).view();
        for presence in [Presence::Lost, Presence::Departed] {
            let peers = [present("kim"), peer("baughn", Role::Interactive, presence)];
            assert_eq!(
                playback_blockers(&view, &peers),
                vec![Blocker {
                    user: UserId::new("baughn"),
                    reason: BlockReason::CommittedAbsent,
                }],
                "committed absent ({presence:?}) must block"
            );
            assert!(!playback_active(&view, &peers));
        }
    }

    #[test]
    fn present_committed_and_maybe_users_gate_on_file_state_alike() {
        // Present commitment doesn't change the present-user rule: a
        // Missing file blocks whether you're committed or Maybe.
        for pref in [SeriesWatchState::Watching, SeriesWatchState::Maybe] {
            let mut state = committed_state(&[("baughn", pref)]);
            state.set_file_availability(
                SERVER,
                ts(20),
                UserId::new("baughn"),
                hash(1),
                FileAvailability::Missing,
            );
            let blockers = playback_blockers(&state.view(), &[present("baughn")]);
            assert_eq!(
                blockers,
                vec![Blocker {
                    user: UserId::new("baughn"),
                    reason: BlockReason::FileMissing,
                }],
                "present {pref:?} with a Missing file must block"
            );
        }
    }

    #[test]
    fn acknowledging_a_committed_absent_user_unblocks_the_current_file() {
        let mut state = committed_state(&[("baughn", SeriesWatchState::Watching)]);
        let peers = [
            present("kim"),
            peer("baughn", Role::Interactive, Presence::Departed),
        ];
        assert!(!playback_active(&state.view(), &peers));

        state.acknowledge_absent(hash(1), UserId::new("baughn"));
        let view = state.view();
        assert!(playback_blockers(&view, &peers).is_empty());
        assert!(playback_active(&view, &peers));
    }

    #[test]
    fn acknowledge_is_scoped_to_the_now_playing_file() {
        // An acknowledge for a *different* file does not unblock now-playing.
        let mut state = committed_state(&[("baughn", SeriesWatchState::Watching)]);
        state.acknowledge_absent(hash(2), UserId::new("baughn"));
        let peers = [
            present("kim"),
            peer("baughn", Role::Interactive, Presence::Departed),
        ];
        assert_eq!(
            playback_blockers(&state.view(), &peers),
            vec![Blocker {
                user: UserId::new("baughn"),
                reason: BlockReason::CommittedAbsent,
            }]
        );
    }

    #[test]
    fn away_excuses_a_committed_absent_user() {
        // The per-user escape: marking the committed-absent user Away also
        // clears the block (does not block at any presence).
        let mut state = committed_state(&[("baughn", SeriesWatchState::Watching)]);
        state.set_manual_override(
            SERVER,
            ts(20),
            UserId::new("baughn"),
            Some(ManualState::Away {
                set_by: UserId::new("kim"),
            }),
        );
        let peers = [
            present("kim"),
            peer("baughn", Role::Interactive, Presence::Departed),
        ];
        assert!(playback_active(&state.view(), &peers));
    }

    #[test]
    fn departed_user_is_removed_from_gating() {
        // A departed user who was Paused no longer blocks — the intent
        // latch (forced Paused by the server on departure) is what
        // keeps playback stopped, not their gating entry.
        let mut state = playing_state();
        state.set_manual_override(SERVER, ts(3), UserId::new("kim"), Some(ManualState::Paused));
        let view = state.view();
        let peers = [
            present("baughn"),
            peer("kim", Role::Interactive, Presence::Departed),
        ];
        assert!(playback_blockers(&view, &peers).is_empty());
        assert!(playback_active(&view, &peers));
    }

    #[test]
    fn seeders_never_gate() {
        let mut state = playing_state();
        // A paused, lost seeder with a missing file: maximally blocked,
        // if it could block.
        state.set_manual_override(SERVER, ts(3), UserId::new("nas"), Some(ManualState::Paused));
        state.set_file_availability(
            SERVER,
            ts(4),
            UserId::new("nas"),
            hash(1),
            FileAvailability::Missing,
        );
        let peers = [present("kim"), peer("nas", Role::Seeder, Presence::Lost)];
        assert!(playback_active(&state.view(), &peers));
    }

    #[test]
    fn users_absent_from_the_peer_list_are_ignored() {
        // CRDT state for a user with no peer entry (long gone): no gate.
        let mut state = playing_state();
        state.set_manual_override(
            SERVER,
            ts(3),
            UserId::new("ghost"),
            Some(ManualState::Paused),
        );
        assert!(playback_active(&state.view(), &[present("kim")]));
    }

    // ---- Feature-request #12 closure: "allow starting when someone who
    // is away / not watching doesn't have the file". The spec (design.md,
    // Playback Rules) says Away and NotWatching users never block, at any
    // presence, whatever their file state — this property pins it against
    // every combination, so the request closes as verified-fixed.
    mod excused_users_never_block {
        use proptest::prelude::*;

        use super::*;

        /// One user's full gating-relevant configuration.
        #[derive(Clone, Debug)]
        struct UserSpec {
            presence: Presence,
            seeder: bool,
            manual: Option<ManualState>,
            /// `None` = no map entry (the implicit Maybe).
            pref: Option<SeriesWatchState>,
            avail: Option<FileAvailability>,
        }

        fn arb_user() -> impl Strategy<Value = UserSpec> {
            let presence = prop_oneof![
                Just(Presence::Present),
                Just(Presence::Lost),
                Just(Presence::Departed),
            ];
            let manual = prop_oneof![
                Just(None),
                Just(Some(ManualState::Paused)),
                Just(Some(ManualState::Away {
                    set_by: UserId::new("setter"),
                })),
            ];
            let pref = prop_oneof![
                Just(None),
                Just(Some(SeriesWatchState::Watching)),
                Just(Some(SeriesWatchState::NotWatching)),
                Just(Some(SeriesWatchState::Maybe)),
            ];
            let avail = prop_oneof![
                Just(None),
                Just(Some(FileAvailability::Ready)),
                Just(Some(FileAvailability::Missing)),
                (0u16..=10_000)
                    .prop_map(|progress_bps| Some(FileAvailability::Downloading { progress_bps })),
            ];
            (presence, any::<bool>(), manual, pref, avail).prop_map(
                |(presence, seeder, manual, pref, avail)| UserSpec {
                    presence,
                    seeder,
                    manual,
                    pref,
                    avail,
                },
            )
        }

        proptest! {
            #[test]
            fn away_notwatching_and_seeders_never_block(
                specs in proptest::collection::vec(arb_user(), 1..6),
            ) {
                let series = AniDbSeriesId(42);
                let mut state = playing_state();
                state.set_anidb_metadata(
                    SERVER,
                    ts(3),
                    hash(1),
                    Some(AniDbMetadata {
                        source: MetadataSource::AniDb,
                        series_name: "Frieren".into(),
                        series_id: Some(series),
                        episode_number: Some("1".into()),
                    }),
                );
                let entry = link_series(&mut state, ts(3), series);
                let mut peers = Vec::new();
                for (i, spec) in specs.iter().enumerate() {
                    let user = UserId::new(format!("user{i}"));
                    let t = 10 + i as u64 * 10;
                    if let Some(manual) = &spec.manual {
                        state.set_manual_override(SERVER, ts(t), user.clone(), Some(manual.clone()));
                    }
                    if let Some(pref) = spec.pref {
                        state.set_series_preference(SERVER, ts(t + 1), user.clone(), entry, pref, None);
                    }
                    if let Some(avail) = spec.avail {
                        state.set_file_availability(SERVER, ts(t + 2), user.clone(), hash(1), avail);
                    }
                    let role = if spec.seeder { Role::Seeder } else { Role::Interactive };
                    peers.push(peer(&format!("user{i}"), role, spec.presence));
                }
                let blockers = playback_blockers(&state.view(), &peers);
                for (i, spec) in specs.iter().enumerate() {
                    let user = UserId::new(format!("user{i}"));
                    let excused = spec.seeder
                        || matches!(spec.manual, Some(ManualState::Away { .. }))
                        || spec.pref == Some(SeriesWatchState::NotWatching);
                    if excused {
                        prop_assert!(
                            !blockers.iter().any(|b| b.user == user),
                            "excused user blocked: {spec:?}, blockers: {blockers:?}",
                        );
                    }
                }
            }
        }
    }
}
