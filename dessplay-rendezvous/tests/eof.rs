//! Phase 5: the server-owned EOF -> next-file transition.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay_core::types::{
    AniDbMetadata, AniDbSeriesId, ManualState, MetadataSource, PlaybackIntent, SeekAuthority,
    SeriesWatchState, UserId,
};

/// Two clients, a two-entry playlist, entry 1 playing.
async fn session(
    harness: &Harness,
) -> (
    dessplay::client::ClientHandle,
    dessplay::client::ClientHandle,
) {
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    mutate(&kim, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(&kim, Mutation::PushPlaylist { new: entry(2) }).await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(hash(1)),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.playlist.len() == 2 && s.playing())
    })
    .await;
    (kim, baughn)
}

/// EOF: watched flag, advance, pause, server authority — and duplicate
/// reports are no-ops.
#[tokio::test(start_paused = true)]
async fn eof_advances_pauses_and_is_idempotent() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = session(&harness).await;

    report_eof(&kim, hash(1)).await;
    let snaps = eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.view.watched.get(&hash(1)) == Some(&true)
                && s.view.now_playing == Some(hash(2))
                && s.view.playback_intent == PlaybackIntent::Paused
                && s.view.seek_authority == Some(SeekAuthority::Server)
        })
    })
    .await;
    // The finished file stays on the playlist (history), unwatched next
    // entry untouched.
    assert_eq!(snaps[0].view.playlist.len(), 2);
    assert_eq!(snaps[0].view.watched.get(&hash(2)), None);

    // Duplicate reports from both clients: now-playing no longer
    // matches, so nothing changes.
    report_eof(&baughn, hash(1)).await;
    report_eof(&kim, hash(1)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snap = snapshot_of(&baughn).await;
    assert_eq!(snap.view.now_playing, Some(hash(2)));
    assert_eq!(snap.view.watched.get(&hash(2)), None);

    // EOF on the last entry: now-playing clears.
    report_eof(&kim, hash(2)).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.now_playing.is_none() && s.view.watched.get(&hash(2)) == Some(&true))
    })
    .await;
}

/// Reports from seeders and from users not watching the series are
/// ignored; a watching user's report still works afterwards.
#[tokio::test(start_paused = true)]
async fn eof_ignores_non_watching_reporters() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = session(&harness).await;
    let nas = harness.seeder("nas", 3);

    // Kim marks the series NotWatching (metadata links the file).
    let series = AniDbSeriesId(42);
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
        Mutation::SetSeriesPreference {
            user: UserId::new("kim"),
            series,
            pref: SeriesWatchState::NotWatching,
            set_by: None,
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| !s.view.series_preference.is_empty() && !s.view.anidb_metadata.is_empty())
    })
    .await;

    // A seeder report and a not-watching report: both ignored.
    report_eof(&nas, hash(1)).await;
    report_eof(&kim, hash(1)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snap = snapshot_of(&baughn).await;
    assert_eq!(
        snap.view.now_playing,
        Some(hash(1)),
        "ignored reporters advanced the file"
    );
    assert_eq!(snap.view.watched.get(&hash(1)), None);

    // Baughn (present, watching) reports: the transition runs.
    report_eof(&baughn, hash(1)).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.view.now_playing == Some(hash(2)))
    })
    .await;
}

/// A present but manually-Paused reporter does not advance the group: the
/// EOF transition admits only a present *watching* reporter — Ready
/// (committed) or Maybe — per docs/design.md, Playback Rules. A Maybe
/// reporter still advances afterwards. (Pre-fix `handle_eof` also accepted
/// Paused, so baughn's report below advanced the file.)
#[tokio::test(start_paused = true)]
async fn eof_ignores_a_manually_paused_reporter() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = session(&harness).await;

    // baughn manually pauses -> derived state Paused.
    mutate(
        &baughn,
        Mutation::SetManualOverride {
            user: UserId::new("baughn"),
            state: Some(ManualState::Paused),
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.view.manual_override.get(&UserId::new("baughn")) == Some(&Some(ManualState::Paused))
        })
    })
    .await;

    // baughn (Paused) reports EOF: ignored, the file does not advance.
    report_eof(&baughn, hash(1)).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snap = snapshot_of(&kim).await;
    assert_eq!(
        snap.view.now_playing,
        Some(hash(1)),
        "a manually-paused reporter must not advance now-playing"
    );
    assert_eq!(snap.view.watched.get(&hash(1)), None);

    // kim (present, Maybe) reports: the transition runs.
    report_eof(&kim, hash(1)).await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.view.now_playing == Some(hash(2)))
    })
    .await;
}

/// A client changing now-playing by hand hands seek authority to the
/// server (one position source during the transition).
#[tokio::test(start_paused = true)]
async fn now_playing_change_gives_server_authority() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = session(&harness).await;

    // Kim grabs authority, then baughn switches files.
    mutate(
        &kim,
        Mutation::SetSeekAuthority {
            authority: SeekAuthority::User(UserId::new("kim")),
        },
    )
    .await;
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        snaps[0].view.seek_authority == Some(SeekAuthority::User(UserId::new("kim")))
    })
    .await;

    mutate(
        &baughn,
        Mutation::SetNowPlaying {
            file: Some(hash(2)),
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.view.now_playing == Some(hash(2))
                && s.view.seek_authority == Some(SeekAuthority::Server)
        })
    })
    .await;
}
