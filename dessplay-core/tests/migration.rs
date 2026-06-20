//! On-disk / wire forward-compatibility for the watch-state change.
//!
//! Two guarantees underpin the coordinated upgrade (docs/sync-state.md):
//!
//! 1. Appending `SeriesWatchState::Maybe` as a *trailing* enum variant
//!    keeps the existing discriminants byte-identical, so values written
//!    before `Maybe` existed still decode.
//! 2. Appending `acknowledged_absent` to `CrdtState` is migrated by
//!    `CrdtState::decode_snapshot`: an older blob (the field-less prefix)
//!    decodes via the `CrdtStateV1` fallback with an empty set, so the
//!    authoritative server never silently loses its List/playlist.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dessplay_core::types::{
    ActorId, ChatMessage, Ed2kHash, PlaybackPosition, SeriesWatchState, SharedTimestamp, UserId,
};
use dessplay_core::{CrdtState, wire};

const A: ActorId = ActorId(1);

fn ts(t: u64) -> SharedTimestamp {
    SharedTimestamp(t)
}

fn hash(i: u8) -> Ed2kHash {
    Ed2kHash([i; 16])
}

/// A state populated across a spread of fields, with an **empty**
/// `acknowledged_absent` — so its postcard encoding ends with exactly the
/// one-byte empty-GSet that the v1 layout lacks.
fn sample_state() -> CrdtState {
    let mut state = CrdtState::new();
    state.set_now_playing(A, ts(1), Some(hash(1)));
    state.set_watched(A, ts(2), hash(1), true);
    // All three watch states, including the new trailing variant.
    state.set_series_preference(
        A,
        ts(3),
        UserId::new("baughn"),
        dessplay_core::types::AniDbSeriesId(7),
        SeriesWatchState::Watching,
    );
    state.set_series_preference(
        A,
        ts(4),
        UserId::new("kim"),
        dessplay_core::types::AniDbSeriesId(8),
        SeriesWatchState::Maybe,
    );
    state.set_series_preference(
        A,
        ts(5),
        UserId::new("nero"),
        dessplay_core::types::AniDbSeriesId(9),
        SeriesWatchState::NotWatching,
    );
    state.append_chat(ChatMessage {
        timestamp: ts(6),
        sender: UserId::new("baughn"),
        text: "hi".into(),
    });
    state.set_playback_position(
        A,
        ts(7),
        UserId::new("kim"),
        PlaybackPosition {
            position_millis: 123,
            timestamp: ts(7),
        },
    );
    state
}

#[test]
fn series_watch_state_discriminants_are_stable() {
    // The two old variants must keep discriminants 0 and 1; Maybe is the
    // new trailing 2. An old binary wrote 0/1; the new enum still reads them.
    assert_eq!(wire::encode(&SeriesWatchState::Watching).unwrap(), vec![0]);
    assert_eq!(
        wire::encode(&SeriesWatchState::NotWatching).unwrap(),
        vec![1]
    );
    assert_eq!(wire::encode(&SeriesWatchState::Maybe).unwrap(), vec![2]);

    assert_eq!(
        wire::decode::<SeriesWatchState>(&[0]).unwrap(),
        SeriesWatchState::Watching
    );
    assert_eq!(
        wire::decode::<SeriesWatchState>(&[1]).unwrap(),
        SeriesWatchState::NotWatching
    );
}

#[test]
fn v1_snapshot_without_acknowledged_absent_upgrades() {
    let state = sample_state();
    let bytes = wire::encode(&state).unwrap();

    // The empty trailing GSet is exactly one length-0 byte. Stripping it
    // reproduces the pre-`acknowledged_absent` (v1) on-disk layout.
    assert_eq!(
        *bytes.last().unwrap(),
        0u8,
        "an empty acknowledged_absent must serialize to a single 0x00"
    );
    let v1_bytes = &bytes[..bytes.len() - 1];

    let upgraded =
        CrdtState::decode_snapshot(v1_bytes).expect("v1 blob must decode via the fallback");
    // Everything else survives; the new field comes up empty.
    assert_eq!(upgraded.view(), state.view());
    assert!(upgraded.view().acknowledged_absent.is_empty());
}

#[test]
fn current_snapshot_round_trips_through_decode_snapshot() {
    // The Ok path: a current-layout blob (with a populated set) decodes
    // directly, no fallback.
    let mut state = sample_state();
    state.acknowledge_absent(hash(1), UserId::new("baughn"));
    let bytes = wire::encode(&state).unwrap();

    let decoded = CrdtState::decode_snapshot(&bytes).unwrap();
    assert_eq!(decoded.view(), state.view());
    assert!(
        decoded
            .view()
            .acknowledged_absent
            .contains(&(hash(1), UserId::new("baughn")))
    );
}

#[test]
fn genuinely_corrupt_blob_still_errors() {
    // Neither the current layout nor the v1 fallback can read garbage, so
    // a real codec error surfaces (the client's tolerant loader relies on
    // this to drop-and-resync).
    assert!(CrdtState::decode_snapshot(b"not a valid postcard CrdtState").is_err());
    assert!(CrdtState::decode_snapshot(&[]).is_err());
}
