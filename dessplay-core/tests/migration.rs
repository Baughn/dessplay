//! Snapshot storage-format compatibility.
//!
//! Storage blobs carry a tagged envelope — [`SNAPSHOT_MAGIC`] plus the
//! protocol version, see `CrdtState::encode_snapshot` — so a blob names
//! its own layout instead of being identified by trial decode. Exactly
//! one **untagged** legacy layout (protocol v6, pre-envelope) is still
//! decoded and migrated forward: every database deployed at the envelope
//! change held it. A tagged blob with any *other* version is refused
//! outright (the server's refuse-to-start posture) rather than guessed
//! at; a deliberate migration adds an explicit decode arm instead.
//!
//! These tests pin the envelope format, the legacy fallback (via the
//! test-support fixture encoder, which stays faithful to the frozen v6
//! layout even as `CrdtState` changes), the version-mismatch error, and
//! the corrupt-blob error the client's tolerant loader relies on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use dessplay_core::net::message::PROTOCOL_VERSION;
use dessplay_core::state::SNAPSHOT_MAGIC;
use dessplay_core::types::{
    ActorId, ChatMessage, Ed2kHash, SeriesWatchState, SharedTimestamp, UserId,
};
use dessplay_core::{CrdtState, wire};

const A: ActorId = ActorId(1);

fn ts(t: u64) -> SharedTimestamp {
    SharedTimestamp(t)
}

fn hash(i: u8) -> Ed2kHash {
    Ed2kHash([i; 16])
}

/// A state populated across a spread of field kinds (register, map,
/// GList, GSet).
fn sample_state() -> CrdtState {
    let mut state = CrdtState::new();
    state.set_now_playing(A, ts(1), Some(hash(1)));
    state.set_watched(A, ts(2), hash(1), true);
    state.acknowledge_absent(hash(1), UserId::new("baughn"));
    state.append_chat(ChatMessage {
        timestamp: ts(6),
        sender: UserId::new("baughn"),
        text: "hi".into(),
    });
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

/// A `PROTOCOL_VERSION` bump must fail HERE, not at server deploy.
/// Layout compatibility is deliberately keyed on `PROTOCOL_VERSION`
/// (one version, no second constant to keep in step), which means a
/// **wire-only** bump also moves the storage tag past the newest
/// handled snapshot version — exactly what bricked the tsugumi server
/// on 2026-07-28 (it refused to start on its own authoritative v7
/// snapshot after the v7 → v9 bump, because nobody had updated the
/// compat list). This test turns that forgotten-entry failure into a
/// red build: every tagged version below the current one must be
/// handled somewhere, or the bump does not pass the suite.
#[test]
fn every_tagged_snapshot_version_is_deliberately_handled() {
    for version in CrdtState::FIRST_TAGGED_SNAPSHOT_VERSION..PROTOCOL_VERSION {
        let compatible = CrdtState::LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS.contains(&version);
        let frozen = CrdtState::FROZEN_LAYOUT_SNAPSHOT_VERSIONS.contains(&version);
        assert!(
            !(compatible && frozen),
            "v{version} is listed as both layout-compatible and frozen-layout; pick one"
        );
        assert!(
            compatible || frozen,
            "PROTOCOL_VERSION moved past v{version}, but nothing says how a \
             v{version}-tagged storage snapshot decodes — on the server that is a \
             refuse-to-start at deploy (the 2026-07-28 outage). Decide now:\n\
             - persisted CrdtState layout UNCHANGED since v{version} (check the \
               diff!): append {version} to LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS and \
               capture its fixture blob (`cargo test -p dessplay-core --test \
               migration -- --ignored capture_missing_snapshot_fixtures`; policy \
               in tests/fixtures/README.md, pin in \
               layout_compatible_fixture_blobs_decode_to_the_expected_view);\n\
             - layout CHANGED: add a frozen-layout decode arm for v{version} in \
               decode_snapshot_flagged and list it in \
               FROZEN_LAYOUT_SNAPSHOT_VERSIONS."
        );
    }

    // Sanity on the lists themselves: every entry names a real, older
    // tagged version (a typo'd or never-removed entry is also a wrong
    // corruption decision on the authoritative store).
    for v in CrdtState::LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS
        .into_iter()
        .chain(CrdtState::FROZEN_LAYOUT_SNAPSHOT_VERSIONS)
    {
        assert!(
            (CrdtState::FIRST_TAGGED_SNAPSHOT_VERSION..PROTOCOL_VERSION).contains(&v),
            "v{v} in a handled-versions list is not an older tagged version"
        );
    }
}

#[test]
fn tagged_snapshot_round_trips() {
    let state = sample_state();
    let blob = state.encode_snapshot().unwrap();

    // The envelope: magic, then the protocol version as u32 LE, then the
    // raw postcard body.
    assert_eq!(blob[..4], SNAPSHOT_MAGIC);
    assert_eq!(
        u32::from_le_bytes(blob[4..8].try_into().unwrap()),
        dessplay_core::net::message::PROTOCOL_VERSION
    );

    let (decoded, migrated) = CrdtState::decode_snapshot_flagged(&blob).unwrap();
    assert!(!migrated, "a tagged current blob is not a migration");
    assert_eq!(decoded.view(), state.view());
}

#[test]
fn untagged_legacy_blob_decodes_flagged() {
    // The one legacy fallback: an untagged v6 blob (fabricated via the
    // test-support fixture encoder, which is frozen to the v6 layout).
    let state = sample_state();
    let blob = state.encode_untagged_v6_for_tests().unwrap();
    assert_ne!(
        blob[0], SNAPSHOT_MAGIC[0],
        "legacy blobs must not collide with the envelope magic"
    );

    let (decoded, migrated) = CrdtState::decode_snapshot_flagged(&blob).unwrap();
    assert!(migrated, "an untagged blob must report the fallback");
    assert_eq!(decoded.view(), state.view());
}

#[test]
fn tagged_blob_with_a_different_version_is_refused() {
    // Refuse-to-guess: a valid body under a wrong version tag must error,
    // not decode — layout changes between versions are exactly what the
    // tag exists to catch.
    let state = sample_state();
    let mut blob = state.encode_snapshot().unwrap();
    blob[4..8].copy_from_slice(&999u32.to_le_bytes());
    assert!(CrdtState::decode_snapshot(&blob).is_err());

    // A truncated envelope (magic but no version) errors too.
    assert!(CrdtState::decode_snapshot(&SNAPSHOT_MAGIC).is_err());
}

#[test]
fn genuinely_corrupt_blob_still_errors() {
    // Neither the envelope nor the v6 fallback can read garbage, so a
    // real codec error surfaces (the client's tolerant loader relies on
    // this to drop-and-resync).
    assert!(CrdtState::decode_snapshot(b"not a valid postcard CrdtState").is_err());
    assert!(CrdtState::decode_snapshot(&[]).is_err());
}
