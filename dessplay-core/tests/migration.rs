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
//! These tests pin the envelope format, the exhaustiveness contract on
//! the handled-versions lists (a `PROTOCOL_VERSION` bump is a red build
//! until the storage decision is made), the version-mismatch error, and
//! the corrupt-blob error the client's tolerant loader relies on. The
//! decode paths for **older** layouts — the compat-listed tagged
//! versions and the untagged v6 fallback — are pinned by checked-in
//! binary fixture blobs (tests/fixtures/, captured once and never
//! regenerated), because a fixture fabricated by a live encoder drifts
//! with the very code it is supposed to check.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::arb_step;
use dessplay_core::test_support::run_script;
use proptest::prelude::*;

use dessplay_core::net::message::PROTOCOL_VERSION;
use dessplay_core::state::SNAPSHOT_MAGIC;
use dessplay_core::types::{
    ActorId, AniDbMetadata, AniDbSeriesId, ChatMessage, Ed2kHash, FileAvailability,
    FileCatalogEntry, FileHashInfo, ListEntryId, ListStatus, ManualState, MarqueeMessage,
    MetadataSource, NextEpState, PlaybackIntent, PlaybackPosition, RelationKind, SeekAuthority,
    SeriesListEntry, SeriesRelation, SeriesRelations, SeriesWatchState, SharedTimestamp, UserId,
    UserSeek,
};
use dessplay_core::{CrdtState, NewPlaylistEntry, wire};

const A: ActorId = ActorId(1);
const A2: ActorId = ActorId(2);

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

/// The state behind every checked-in fixture blob under tests/fixtures/.
/// Populated across every `CrdtState` field — playlist entries, series
/// preferences (including a `set_by` attribution), every
/// `FileAvailability` variant including `DownloadingPlayable`, a List
/// entry with watchers/aliases/manual files, chat (plain and CTCP
/// action), a marquee — so a *misaligned* decode of a fixture cannot
/// accidentally reproduce the expected view.
///
/// **Do not change what this writes.** The fixture blobs freeze this
/// function's encoding as of their capture date; the decode tests
/// compare a fixture against this function's view at test time, so a
/// semantic change here breaks every already-captured fixture. New
/// fixture content belongs in a new builder.
fn rich_sample_state() -> CrdtState {
    let baughn = || UserId::new("baughn");
    let kim = || UserId::new("kim");
    let nero = || UserId::new("nero");
    let entry_id = ListEntryId(7);

    let mut state = CrdtState::new();
    state.push_playlist_entry(
        A,
        ts(10),
        NewPlaylistEntry {
            hash: hash(1),
            added_by: baughn(),
            filename: "[Judas] Sousou no Frieren - 03.mkv".into(),
            size_bytes: 730_000_000,
            duration_millis: Some(1_420_000),
        },
    );
    state.push_playlist_entry(
        A2,
        ts(11),
        NewPlaylistEntry {
            hash: hash(2),
            added_by: kim(),
            filename: "RahXephon - 05.mkv".into(),
            size_bytes: 350_000_000,
            duration_millis: None,
        },
    );
    state.set_watched(A, ts(12), hash(1), true);
    state.set_now_playing(A, ts(13), Some(hash(2)));
    state.set_seek_authority(
        A,
        ts(14),
        SeekAuthority::User(UserSeek {
            user: kim(),
            file: hash(2),
            event_at: ts(14),
            from_millis: 492_000,
            to_millis: 754_000,
        }),
    );
    state.set_playback_intent(A, ts(15), PlaybackIntent::Playing);
    state.set_series_preference(
        A,
        ts(16),
        baughn(),
        entry_id,
        SeriesWatchState::Watching,
        None,
    );
    state.set_series_preference(
        A2,
        ts(17),
        kim(),
        entry_id,
        SeriesWatchState::NotWatching,
        Some(nero()),
    );
    state.set_manual_override(A, ts(18), baughn(), Some(ManualState::Paused));
    state.set_manual_override(
        A2,
        ts(19),
        nero(),
        Some(ManualState::Away { set_by: kim() }),
    );
    state.set_file_availability(A, ts(20), baughn(), hash(1), FileAvailability::Ready);
    state.set_file_availability(
        A,
        ts(21),
        baughn(),
        hash(2),
        FileAvailability::Downloading { progress_bps: 3400 },
    );
    state.set_file_availability(
        A2,
        ts(22),
        kim(),
        hash(2),
        FileAvailability::DownloadingPlayable { progress_bps: 8200 },
    );
    state.set_file_availability(A2, ts(23), nero(), hash(2), FileAvailability::Missing);
    state.set_anidb_metadata(
        A,
        ts(24),
        hash(1),
        Some(AniDbMetadata {
            source: MetadataSource::AniDb,
            series_name: "Sousou no Frieren".into(),
            series_id: Some(AniDbSeriesId(17617)),
            episode_number: Some("3".into()),
        }),
    );
    state.set_anidb_metadata(
        A,
        ts(25),
        hash(2),
        Some(AniDbMetadata {
            source: MetadataSource::FilenameDerived,
            series_name: "RahXephon".into(),
            series_id: None,
            episode_number: None,
        }),
    );
    state.set_series_relations(
        A,
        ts(26),
        AniDbSeriesId(17617),
        SeriesRelations {
            title: "Sousou no Frieren".into(),
            year: Some(2023),
            episode_count: Some(28),
            relations: BTreeSet::from([SeriesRelation {
                kind: RelationKind::Sequel,
                target: AniDbSeriesId(18886),
            }]),
            // Kept empty: every checked-in fixture predates the field
            // (v6–v10), decoding through the frozen arm to an empty vec —
            // this view is what those fixtures are compared against. New
            // fixture content belongs in a new builder (fixtures README).
            short_titles: vec![],
        },
    );
    state.set_file_catalog(
        A,
        ts(27),
        hash(2),
        FileCatalogEntry {
            filename: "RahXephon - 05.mkv".into(),
            size_bytes: 350_000_000,
            duration_millis: None,
        },
    );
    state.put_list_entry(
        A,
        ts(28),
        entry_id,
        SeriesListEntry {
            name: "Sousou no Frieren".into(),
            nero_name: Some("Funeral Frieren".into()),
            genre: Some("fantasy".into()),
            notes: vec!["movie when?".into()],
            recommender: Some("Baughn".into()),
            status: ListStatus::Active,
            status_note: None,
            source: Some("SubsPlease".into()),
            watchers: BTreeSet::from([baughn(), kim()]),
            anidb_series_id: Some(AniDbSeriesId(17617)),
            local_aliases: BTreeSet::from(["Frieren".to_owned()]),
            manual_files: BTreeSet::from([hash(1)]),
            anidb_unavailable: false,
        },
    );
    state.set_next_ep(
        A,
        ts(29),
        entry_id,
        NextEpState {
            next_ep: Some("4".into()),
            available: true,
        },
    );
    state.request_lookup(FileHashInfo {
        hash: hash(2),
        size: 350_000_000,
        filename: "RahXephon - 05.mkv".into(),
        mtime: Some(1_726_000_000_000),
        series_hint: Some("RahXephon".into()),
    });
    state.append_chat(ChatMessage {
        timestamp: ts(30),
        sender: baughn(),
        text: "spoilers: ||the fern wins||".into(),
    });
    state.append_chat(ChatMessage {
        timestamp: ts(31),
        sender: kim(),
        text: "\u{1}ACTION waves\u{1}".into(),
    });
    state.set_playback_position(
        A,
        ts(32),
        baughn(),
        PlaybackPosition {
            position_millis: 754_321,
            timestamp: ts(32),
            file: hash(2),
        },
    );
    state.acknowledge_absent(hash(2), nero());
    state.set_marquee(
        A,
        ts(33),
        Some(MarqueeMessage {
            text: "<Amu> Whaaaat?".into(),
            set_by: Some(baughn()),
        }),
    );
    state
}

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read_fixture(name: &str) -> Vec<u8> {
    let path = fixtures_dir().join(name);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "missing fixture {}: {e}\n\
             A new compat version's fixture is captured ONCE, at the moment of \
             the bump: `cargo test -p dessplay-core --test migration -- --ignored \
             capture_missing_snapshot_fixtures`. Existing fixtures are never \
             regenerated — see tests/fixtures/README.md.",
            path.display()
        )
    })
}

/// A fixture blob for `version`: today's snapshot encoding, re-tagged.
/// Honest by the compat-list assertion — a version enters
/// `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` only when its persisted layout
/// is byte-identical to the current one, so capturing with the current
/// encoder *at the moment of the bump* reproduces that version's real
/// bytes. Once written the file is frozen: any later drift of the
/// current type fails the decode test instead of re-baking the fixture.
fn tagged_fixture_bytes(version: u32) -> Vec<u8> {
    let mut blob = rich_sample_state().encode_snapshot().unwrap();
    blob[SNAPSHOT_MAGIC.len()..SNAPSHOT_MAGIC.len() + 4].copy_from_slice(&version.to_le_bytes());
    blob
}

/// Fixture capture. Run explicitly (`-- --ignored
/// capture_missing_snapshot_fixtures`) when a version is added to
/// `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS`; commit the new file. Writes
/// only fixtures that do not exist yet — an existing fixture is never
/// rewritten, because its whole value is that its bytes stay frozen at
/// capture time (tests/fixtures/README.md).
#[test]
#[ignore = "writes new fixture blobs; run once when a version enters the compat list"]
fn capture_missing_snapshot_fixtures() {
    std::fs::create_dir_all(fixtures_dir()).unwrap();
    for version in CrdtState::LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS {
        let path = fixtures_dir().join(format!("snapshot-v{version}.bin"));
        if path.exists() {
            continue;
        }
        std::fs::write(&path, tagged_fixture_bytes(version)).unwrap();
        eprintln!("captured {}", path.display());
    }
    let v6 = fixtures_dir().join("snapshot-untagged-v6.bin");
    if !v6.exists() {
        let blob = rich_sample_state().encode_untagged_v6_for_tests().unwrap();
        std::fs::write(&v6, blob).unwrap();
        eprintln!("captured {}", v6.display());
    }
}

/// Every version in `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` decodes from
/// its checked-in fixture blob — **real frozen bytes**, not a re-tagged
/// fresh encoding — to the expected resolved view. This is what makes a
/// compat-list entry mean something: if the current type drifts against
/// a listed version's bytes (postcard is positional, and a misaligned
/// decode can succeed with silently wrong values), this fails, and that
/// version must move to a frozen-layout decode arm.
#[test]
fn layout_compatible_fixture_blobs_decode_to_the_expected_view() {
    let expected = rich_sample_state().view();
    for version in CrdtState::LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS {
        let blob = read_fixture(&format!("snapshot-v{version}.bin"));
        assert_eq!(blob[..4], SNAPSHOT_MAGIC, "fixture v{version}: bad magic");
        assert_eq!(
            u32::from_le_bytes(blob[4..8].try_into().unwrap()),
            version,
            "fixture v{version} is tagged with the wrong version"
        );

        let (decoded, migrated) = CrdtState::decode_snapshot_flagged(&blob).unwrap_or_else(|e| {
            panic!(
                "the checked-in v{version} fixture no longer decodes ({e}): the \
                     current CrdtState layout has drifted from v{version}'s bytes, so \
                     v{version} can no longer sit in LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS \
                     — move it to a frozen-layout decode arm (do NOT regenerate the \
                     fixture; its bytes are the contract)"
            )
        });
        assert!(migrated, "a v{version} tag must report the migration");
        assert_eq!(
            decoded.protocol_version, PROTOCOL_VERSION,
            "the migrated state re-tags itself"
        );
        assert_eq!(
            decoded.view(),
            expected,
            "the v{version} fixture decoded, but to the WRONG view — a misaligned \
             (silently corrupting) decode; v{version} must leave \
             LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS for a frozen-layout decode arm \
             (do NOT regenerate the fixture)"
        );
    }
}

/// Every version in `FROZEN_LAYOUT_SNAPSHOT_VERSIONS` decodes from its
/// checked-in fixture blob — that version's **real frozen bytes** —
/// through the frozen `CrdtStateV10` arm to the expected resolved view,
/// with `short_titles` (which those bodies predate) upgraded to empty.
/// The v7–v9 blobs were captured while those versions sat in
/// `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS`; v10's was captured at the v11
/// bump, the last moment the current encoder still produced its bytes.
/// If the frozen structs ever drift against these bytes, this fails —
/// the fixtures are the contract, never regenerate them.
#[test]
fn frozen_layout_fixture_blobs_decode_to_the_expected_view() {
    let expected = rich_sample_state().view();
    for version in CrdtState::FROZEN_LAYOUT_SNAPSHOT_VERSIONS {
        let blob = read_fixture(&format!("snapshot-v{version}.bin"));
        assert_eq!(blob[..4], SNAPSHOT_MAGIC, "fixture v{version}: bad magic");
        assert_eq!(
            u32::from_le_bytes(blob[4..8].try_into().unwrap()),
            version,
            "fixture v{version} is tagged with the wrong version"
        );

        let (decoded, migrated) = CrdtState::decode_snapshot_flagged(&blob).unwrap_or_else(|e| {
            panic!(
                "the checked-in v{version} fixture no longer decodes ({e}): the \
                 frozen v7–v10 layout structs have drifted from v{version}'s real \
                 bytes (do NOT regenerate the fixture; its bytes are the contract)"
            )
        });
        assert!(migrated, "a v{version} tag must report the migration");
        assert_eq!(
            decoded.protocol_version, PROTOCOL_VERSION,
            "the migrated state re-tags itself"
        );
        assert_eq!(
            decoded.view(),
            expected,
            "the v{version} fixture decoded, but to the WRONG view — a misaligned \
             (silently corrupting) decode through the frozen arm \
             (do NOT regenerate the fixture)"
        );
    }
}

/// A frozen-layout blob whose `series_relations` carried data survives
/// the upgrade with LWW timestamps intact: a later write with an older
/// stamp must still lose against the migrated entry, exactly as it
/// would have against the original.
#[test]
fn frozen_upgrade_preserves_relations_lww_timestamps() {
    let blob = read_fixture("snapshot-v10.bin");
    let (mut decoded, _) = CrdtState::decode_snapshot_flagged(&blob).unwrap();

    let series = AniDbSeriesId(17617);
    let original = decoded.view().series_relations[&series].clone();
    assert_eq!(original.title, "Sousou no Frieren");
    assert_eq!(original.short_titles, Vec::<String>::new());

    // rich_sample_state wrote this entry at ts(26); an older write must lose.
    let stale = SeriesRelations {
        title: "Stale".into(),
        year: None,
        episode_count: None,
        relations: BTreeSet::new(),
        short_titles: vec!["stale".into()],
    };
    decoded.set_series_relations(A2, ts(25), series, stale);
    assert_eq!(
        decoded.view().series_relations[&series],
        original,
        "an older-stamped write must not beat the migrated entry"
    );
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
    // The fallback plumbing, via the test-support fixture encoder. Note
    // this encoder freezes only the v6 *top-level field list* — nested
    // value shapes ride the live types, so encoder and decoder drift
    // together and this test alone cannot catch that drift. The real
    // pin is the checked-in binary fixture below.
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

/// The untagged-v6 fallback, pinned by **frozen bytes**: the checked-in
/// blob (captured 2026-08-13, never regenerated — see
/// tests/fixtures/README.md) decodes and migrates forward to the
/// expected view. `CrdtStateUntaggedV6` freezes only its top-level
/// field list; its nested value types are the live `types` shapes, and
/// only append-only drift keeps old bytes decodable — this fixture is
/// what fails when a nested change breaks that, where the live-encoder
/// test above would drift along and stay green.
#[test]
fn untagged_v6_fixture_blob_decodes_via_the_legacy_fallback() {
    let blob = read_fixture("snapshot-untagged-v6.bin");
    assert_ne!(
        blob[0], SNAPSHOT_MAGIC[0],
        "legacy blobs must not collide with the envelope magic"
    );

    let (decoded, migrated) = CrdtState::decode_snapshot_flagged(&blob).unwrap_or_else(|e| {
        panic!(
            "the checked-in untagged-v6 fixture no longer decodes ({e}): a nested \
             type change broke the append-only drift the fallback depends on — \
             pre-envelope databases are now unreadable (on the server, a fatal \
             refusal BEFORE the pre-migration backup runs). Do not regenerate the \
             fixture; either revert the shape change or freeze the nested types \
             into CrdtStateUntaggedV6."
        )
    });
    assert!(migrated, "an untagged blob must report the fallback");

    // The v6 layout predates the marquee register; everything else must
    // survive the round trip through the frozen bytes.
    let mut expected = rich_sample_state().view();
    expected.marquee = None;
    assert_eq!(decoded.view(), expected);
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

proptest! {
    /// `SNAPSHOT_MAGIC`'s discriminator claim — no untagged postcard
    /// state begins with 0xFF (the first field is the playlist map's
    /// vclock length varint, and 0xFF there would claim a
    /// continuation-varint size no real state reaches) — checked over
    /// generated states, in both the current untagged encoding and the
    /// legacy v6 one, rather than asserted on a single handpicked
    /// fixture's first byte.
    #[test]
    fn untagged_encodings_never_begin_with_the_magic(
        steps in proptest::collection::vec(arb_step(), 0..40),
    ) {
        let (state, _) = run_script(&steps);
        let current = wire::encode(&state)
            .map_err(|e| TestCaseError::fail(format!("encode failed: {e}")))?;
        prop_assert_ne!(current[0], SNAPSHOT_MAGIC[0]);
        let v6 = state
            .encode_untagged_v6_for_tests()
            .map_err(|e| TestCaseError::fail(format!("v6 encode failed: {e}")))?;
        prop_assert_ne!(v6[0], SNAPSHOT_MAGIC[0]);
    }
}
