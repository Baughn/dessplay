//! design.md #15: the persisted known-users registry survives a server
//! restart, unlike the in-memory peer registry — so a user who connected
//! yesterday (but hasn't shown up today) can still be named and acted on.

mod common;

use std::time::Duration;

use common::*;
use dessplay_core::types::UserId;

/// Poll `handle.known_offline` (a plain `watch::Receiver`, not part of
/// `ClientSnapshot`/`SnapshotSource`) until `pred` holds, mirroring
/// `eventually`'s budget/retry shape.
async fn eventually_known_offline(
    handle: &dessplay::client::ClientHandle,
    budget: Duration,
    mut pred: impl FnMut(&[dessplay_core::net::KnownUser]) -> bool,
) -> Vec<dessplay_core::net::KnownUser> {
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        let known = handle.known_offline.borrow().clone();
        if pred(&known) {
            return known;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("condition not reached; final known_offline: {known:#?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A user seen on a server that later restarts (same on-disk storage, a
/// fresh in-memory registry) still shows up as known-offline to a peer
/// who connects afterward — and disappears once they reconnect.
#[tokio::test(start_paused = true)]
async fn known_user_survives_a_server_restart() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("rendezvous.db");

    // First "session": kim connects, then quits. Her connect/disconnect
    // both record_seen.
    {
        let storage = dessplay_rendezvous::storage::ServerStorage::open(&db_path).unwrap();
        let harness = Harness::with_config_and_storage(
            0x5EED,
            dessplay_rendezvous::server::ServerConfig::new(PASSWORD),
            Some(storage),
        );
        let kim = harness.client("kim", 1);
        // Wait for kim to actually be registered (her own first PeerList).
        eventually(&[&kim], Duration::from_secs(30), |snaps| {
            snaps[0].peer("kim").is_some()
        })
        .await;
        quit(&kim).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // "Restart": a fresh in-memory registry (new Harness/SimNetwork) but
    // the same on-disk known_users table.
    let storage = dessplay_rendezvous::storage::ServerStorage::open(&db_path).unwrap();
    let harness = Harness::with_config_and_storage(
        0xF00D,
        dessplay_rendezvous::server::ServerConfig::new(PASSWORD),
        Some(storage),
    );
    let baughn = harness.client("baughn", 2);

    // Kim never connected to *this* server process, yet baughn learns she
    // was seen before.
    eventually_known_offline(&baughn, Duration::from_secs(30), |known| {
        known.iter().any(|k| k.username == UserId::new("kim"))
    })
    .await;

    // Once kim actually reconnects, she's Present (not known-offline).
    let kim = harness.client("kim", 3);
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.presence == dessplay_core::net::Presence::Present)
    })
    .await;
    eventually_known_offline(&baughn, Duration::from_secs(30), |known| {
        !known.iter().any(|k| k.username == UserId::new("kim"))
    })
    .await;
    drop(kim);
}

/// The production regression behind "Known-offline users gate too"
/// (design.md, Presence; 2026-07-18): the server restarted, its in-memory
/// registry forgot every absent user, and an unpause played past a
/// committed, offline Nero. The commitment must gate end-to-end across
/// the restart — server persistence → `known_offline` push → the client
/// router's merge into the gating peer list (`client.rs`) → derivation —
/// and the durable `/skip <name>` escape hatch must clear it. The unit
/// tests cover `merge_known_offline` and the display side separately;
/// this is the only place the router-merge wiring itself is exercised.
#[tokio::test(start_paused = true)]
async fn committed_known_offline_user_blocks_playback_across_server_restart() {
    use dessplay::actors::sync::Mutation;
    use dessplay_core::derive::{BlockReason, playback_blockers};
    use dessplay_core::types::{
        AniDbMetadata, AniDbSeriesId, ListEntryId, ListStatus, MetadataSource, PlaybackIntent,
        SeriesListEntry, SeriesWatchState,
    };

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("rendezvous.db");
    let series = AniDbSeriesId(42);
    let entry_id = ListEntryId(42);
    let kim_id = UserId::new("kim");

    // Session one: kim queues ep1, commits (Watching) to its series, and
    // quits. The server flushes its snapshot and records her last-seen.
    {
        let storage = dessplay_rendezvous::storage::ServerStorage::open(&db_path).unwrap();
        let harness = Harness::with_config_and_storage(
            0x5EED,
            dessplay_rendezvous::server::ServerConfig::new(PASSWORD),
            Some(storage),
        );
        let kim = harness.client("kim", 1);
        mutate(&kim, Mutation::PushPlaylist { new: entry(1) }).await;
        mutate(
            &kim,
            Mutation::SetNowPlaying {
                file: Some(hash(1)),
            },
        )
        .await;
        mutate(
            &kim,
            Mutation::SetAniDbMetadata {
                hash: hash(1),
                metadata: Some(AniDbMetadata {
                    source: MetadataSource::AniDb,
                    series_name: "Frieren".into(),
                    series_id: Some(series),
                    episode_number: Some("1".into()),
                }),
            },
        )
        .await;
        mutate(
            &kim,
            Mutation::PutListEntry {
                id: entry_id,
                entry: SeriesListEntry {
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
            },
        )
        .await;
        mutate(
            &kim,
            Mutation::SetSeriesPreference {
                user: kim_id.clone(),
                entry: entry_id,
                pref: SeriesWatchState::Watching,
                set_by: None,
            },
        )
        .await;
        eventually(&[&kim], Duration::from_secs(30), |snaps| {
            snaps[0]
                .view
                .series_preference
                .get(&(kim_id.clone(), entry_id))
                .map(|p| p.state)
                == Some(SeriesWatchState::Watching)
        })
        .await;
        // Outlive a server FLUSH_INTERVAL so the snapshot is persisted.
        tokio::time::sleep(Duration::from_secs(31)).await;
        quit(&kim).await;
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // "Restart": a fresh in-memory registry, the same on-disk state.
    let storage = dessplay_rendezvous::storage::ServerStorage::open(&db_path).unwrap();
    let harness = Harness::with_config_and_storage(
        0xF00D,
        dessplay_rendezvous::server::ServerConfig::new(PASSWORD),
        Some(storage),
    );
    let baughn = harness.client("baughn", 2);

    // Baughn syncs the restored state and learns kim was seen before.
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        snaps[0].view.now_playing == Some(hash(1))
            && snaps[0]
                .view
                .series_preference
                .get(&(kim_id.clone(), entry_id))
                .map(|p| p.state)
                == Some(SeriesWatchState::Watching)
    })
    .await;
    eventually_known_offline(&baughn, Duration::from_secs(30), |known| {
        known.iter().any(|k| k.username == kim_id)
    })
    .await;

    // He presses play: the committed kim must still block, even though
    // she exists only as a `known_offline` row on this server process.
    mutate(
        &baughn,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snap = snapshot_of(&baughn).await;
    assert!(
        !snap.playing(),
        "a committed known-offline user must block playback across a server restart"
    );
    assert!(
        playback_blockers(&snap.view, &snap.peers)
            .iter()
            .any(|b| b.user == kim_id && b.reason == BlockReason::CommittedAbsent),
        "kim must surface as the committed-absent blocker; got {:?}",
        playback_blockers(&snap.view, &snap.peers)
    );

    // The durable escape hatch (`/skip kim`): playback unblocks for the
    // whole series.
    mutate(
        &baughn,
        Mutation::SetSeriesPreference {
            user: kim_id.clone(),
            entry: entry_id,
            pref: SeriesWatchState::NotWatching,
            set_by: Some(UserId::new("baughn")),
        },
    )
    .await;
    mutate(
        &baughn,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        snaps[0].playing()
    })
    .await;
}

/// design.md (Client Roles): "Seeders are not listed as users" and are
/// "excluded from every presence-derived line." A seeder that connects and
/// later disconnects (e.g. a service restart) must never appear in
/// `known_offline` -- it would otherwise render as a selectable
/// known-offline user in the Users pane, a meaningless `n`/`/skip <name>`
/// target for something that should never gate or be listed at all.
#[tokio::test(start_paused = true)]
async fn seeder_never_appears_in_known_offline() {
    let dir = tempfile::tempdir().unwrap();
    let storage =
        dessplay_rendezvous::storage::ServerStorage::open(&dir.path().join("rendezvous.db"))
            .unwrap();
    let harness = Harness::with_config_and_storage(
        0x5EED,
        dessplay_rendezvous::server::ServerConfig::new(PASSWORD),
        Some(storage),
    );
    let baughn = harness.client("baughn", 1);
    let nas = harness.seeder("nas", 2);

    // Wait for the seeder to actually register (present on the shared
    // registry) before disconnecting it.
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        snaps[0].peer("nas").is_some()
    })
    .await;
    quit(&nas).await;

    // Give the disconnect time to be processed (record_seen, if it were
    // going to run, happens synchronously on the disconnect path).
    tokio::time::sleep(Duration::from_secs(1)).await;

    // A fresh peer connecting afterward must never see "nas" as
    // known-offline.
    let kim = harness.client("kim", 3);
    eventually(&[&kim], Duration::from_secs(30), |snaps| {
        snaps[0].peer("kim").is_some()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert!(
        !kim.known_offline
            .borrow()
            .iter()
            .any(|k| k.username == UserId::new("nas")),
        "seeder leaked into known_offline: {:?}",
        kim.known_offline.borrow()
    );
}
