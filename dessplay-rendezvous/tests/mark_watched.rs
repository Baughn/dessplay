//! Phase 15 (#10): manual mark-watched from the episode browser. Unlike
//! `EofReached` this is not scoped to now-playing and touches no playback
//! register — just the watched flag, plus the same List `next_ep`
//! auto-advance the EOF path gets when marking `true`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay_core::types::{
    AniDbMetadata, AniDbSeriesId, ListEntryId, ListStatus, MetadataSource, NextEpState,
    SeriesListEntry,
};

const FRIEREN: AniDbSeriesId = AniDbSeriesId(8692);

fn frieren_entry() -> SeriesListEntry {
    SeriesListEntry {
        name: "Frieren".into(),
        nero_name: None,
        genre: None,
        notes: vec![],
        recommender: None,
        status: ListStatus::Active,
        status_note: None,
        source: None,
        watchers: Default::default(),
        anidb_series_id: Some(FRIEREN),
        local_aliases: Default::default(),
        manual_files: Default::default(),
        anidb_unavailable: false,
    }
}

/// Two clients, one playlist entry with linked AniDB metadata (episode 1).
async fn session(
    harness: &Harness,
) -> (
    dessplay::client::ClientHandle,
    dessplay::client::ClientHandle,
) {
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    mutate(&kim, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(
        &kim,
        Mutation::SetAniDbMetadata {
            hash: hash(1),
            metadata: Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Frieren".into(),
                series_id: Some(FRIEREN),
                episode_number: Some("1".into()),
            }),
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.anidb_metadata.contains_key(&hash(1)))
    })
    .await;
    (kim, baughn)
}

/// Marking watched sets the flag (replicated to both clients) and marking
/// it again is a no-op; unmarking clears it and is likewise idempotent.
#[tokio::test(start_paused = true)]
async fn mark_watched_toggles_and_is_idempotent() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = session(&harness).await;

    mark_watched(&kim, hash(1), true).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.watched.get(&hash(1)) == Some(&true))
    })
    .await;

    // Repeat: no-op (nothing to observe breaking, but must not panic or
    // otherwise desync the replicas).
    mark_watched(&baughn, hash(1), true).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snap = snapshot_of(&kim).await;
    assert_eq!(snap.view.watched.get(&hash(1)), Some(&true));

    // Unmark: the flag clears.
    mark_watched(&kim, hash(1), false).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.watched.get(&hash(1)) == Some(&false))
    })
    .await;
}

/// A file the AniDB-miss fallback already labeled (`series_id: None`,
/// filename-derived name) -- the "genuinely unknown to AniDB" case
/// design.md's Series Identity work is about.
async fn unknown_series_session(
    harness: &Harness,
) -> (
    dessplay::client::ClientHandle,
    dessplay::client::ClientHandle,
) {
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    mutate(&kim, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(
        &kim,
        Mutation::SetAniDbMetadata {
            hash: hash(1),
            metadata: Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Some Obscure Show".into(),
                series_id: None,
                episode_number: None,
            }),
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.anidb_metadata.contains_key(&hash(1)))
    })
    .await;
    (kim, baughn)
}

/// Marking watched auto-advances an *unlinked* List entry's `next_ep` too
/// (design.md, Series Identity): the file resolves to the entry through
/// `manual_files` rather than an AniDB link, and the episode number comes
/// from parsing the file's own name (`entry(1)`'s filename, "ep1.mkv"),
/// since there is no AniDB episode number to fall back on.
#[tokio::test(start_paused = true)]
async fn mark_watched_advances_unlinked_list_entry_via_filename_parse() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = unknown_series_session(&harness).await;

    let id = ListEntryId(43);
    mutate(
        &kim,
        Mutation::PutListEntry {
            id,
            entry: SeriesListEntry {
                name: "Some Obscure Show".into(),
                nero_name: None,
                genre: None,
                notes: vec![],
                recommender: None,
                status: ListStatus::Active,
                status_note: None,
                source: None,
                watchers: Default::default(),
                anidb_series_id: None,
                local_aliases: Default::default(),
                manual_files: [hash(1)].into_iter().collect(),
                anidb_unavailable: false,
            },
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNextEp {
            id,
            next_ep: NextEpState {
                next_ep: Some("1".into()),
                available: true,
            },
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.view.list_next_ep.len() == 1)
    })
    .await;

    mark_watched(&kim, hash(1), true).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.view
                .list_next_ep
                .get(&id)
                .is_some_and(|n| n.next_ep.as_deref() == Some("2") && !n.available)
        })
    })
    .await;
}

/// Regression (2026-07-05 review): a **linked** entry bumps `next_ep`
/// only from the file's own AniDB episode number — the authoritative
/// source (design.md, Advancing next_ep); the filename parse is the
/// *unlinked* entry's mechanism. A linked special ("S1", non-numeric)
/// whose filename happens to parse to the current `next_ep` must not
/// advance past an episode the group never watched.
#[tokio::test(start_paused = true)]
async fn mark_watched_never_advances_a_linked_entry_from_the_filename() {
    let harness = Harness::new(0x5EED);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    // The playlist filename ("ep1.mkv") parses to episode 1, but AniDB
    // says this file is the special "S1".
    mutate(&kim, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(
        &kim,
        Mutation::SetAniDbMetadata {
            hash: hash(1),
            metadata: Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Frieren".into(),
                series_id: Some(FRIEREN),
                episode_number: Some("S1".into()),
            }),
        },
    )
    .await;
    let id = ListEntryId(42);
    mutate(
        &kim,
        Mutation::PutListEntry {
            id,
            entry: frieren_entry(),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNextEp {
            id,
            next_ep: NextEpState {
                next_ep: Some("1".into()),
                available: true,
            },
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.view.list_next_ep.len() == 1)
    })
    .await;

    mark_watched(&kim, hash(1), true).await;
    // The watched flag replicates (proving the mark round-tripped) …
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.watched.get(&hash(1)) == Some(&true))
    })
    .await;
    // … but next_ep must not have moved: the linked entry has no
    // authoritative (numeric) AniDB episode for this file.
    let snap = snapshot_of(&baughn).await;
    let progress = &snap.view.list_next_ep[&id];
    assert_eq!(
        progress.next_ep.as_deref(),
        Some("1"),
        "a linked special must not bump next_ep from a filename parse"
    );
    assert!(progress.available, "available must be untouched too");
}

/// Marking watched auto-advances a linked List entry's `next_ep`, exactly
/// like the EOF transition — but never on unmark (the design only auto-
/// advances forward, never rewinds a next_ep on a manual undo).
#[tokio::test(start_paused = true)]
async fn mark_watched_advances_linked_list_entry() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = session(&harness).await;

    let id = ListEntryId(42);
    mutate(
        &kim,
        Mutation::PutListEntry {
            id,
            entry: frieren_entry(),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNextEp {
            id,
            next_ep: NextEpState {
                next_ep: Some("1".into()),
                available: true,
            },
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.view.list_next_ep.len() == 1)
    })
    .await;

    mark_watched(&kim, hash(1), true).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.view
                .list_next_ep
                .get(&id)
                .is_some_and(|n| n.next_ep.as_deref() == Some("2") && !n.available)
        })
    })
    .await;

    // Unmarking the same file does not rewind next_ep.
    mark_watched(&kim, hash(1), false).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.watched.get(&hash(1)) == Some(&false))
    })
    .await;
    let snap = snapshot_of(&baughn).await;
    assert_eq!(snap.view.list_next_ep[&id].next_ep.as_deref(), Some("2"));
}
