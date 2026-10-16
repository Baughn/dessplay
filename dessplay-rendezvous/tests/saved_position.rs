//! The server-written per-file saved position (design.md, Playback
//! Rules: Saved position): the median of the loaded users' positions,
//! written when the group stops watching a file, 0 past the watched
//! fraction.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay::client::ClientHandle;
use dessplay_core::types::{Ed2kHash, FileAvailability};

/// `entry(i)` lasts 1_440_000ms; 85% of it is 1_224_000.
const PAST_WATCHED: u64 = 1_300_000;

/// A two-entry playlist with entry 1 now-playing, seen by every handle.
async fn playlist(kim: &ClientHandle, everyone: &[&ClientHandle]) {
    mutate(kim, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(kim, Mutation::PushPlaylist { new: entry(2) }).await;
    mutate(
        kim,
        Mutation::SetNowPlaying {
            file: Some(hash(1)),
        },
    )
    .await;
    eventually(everyone, Duration::from_secs(30), |snaps| {
        snaps
            .iter()
            .all(|s| s.view.playlist.len() == 2 && s.view.now_playing == Some(hash(1)))
    })
    .await;
}

/// `client` has `file` loaded at `millis`.
async fn loaded_at(client: &ClientHandle, file: Ed2kHash, millis: u64) {
    mutate(
        client,
        Mutation::SetFileAvailability {
            file,
            availability: FileAvailability::Ready,
        },
    )
    .await;
    mutate(
        client,
        Mutation::SetPlaybackPosition {
            position_millis: millis,
            file,
        },
    )
    .await;
}

/// Wait until every handle sees all `n` users' positions for `file`.
async fn positions_seen(everyone: &[&ClientHandle], file: Ed2kHash, n: usize) {
    eventually(everyone, Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.view
                .playback_position
                .values()
                .filter(|p| p.file == file)
                .count()
                == n
        })
    })
    .await;
}

async fn switch_to(client: &ClientHandle, file: Ed2kHash) {
    mutate(client, Mutation::SetNowPlaying { file: Some(file) }).await;
}

/// Switching away stores the median of the loaded interactive users. A
/// seeder's position never counts. Switching away from a file nobody
/// had loaded writes nothing.
#[tokio::test(start_paused = true)]
async fn switching_away_saves_the_loaded_median() {
    let harness = Harness::new(0x5A7E);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    let chiaki = harness.client("chiaki", 3);
    let nas = harness.seeder("nas", 4);
    let everyone = [&kim, &baughn, &chiaki, &nas];
    playlist(&kim, &everyone).await;

    loaded_at(&kim, hash(1), 300_000).await;
    loaded_at(&baughn, hash(1), 500_000).await;
    loaded_at(&chiaki, hash(1), 400_000).await;
    loaded_at(&nas, hash(1), 1_000_000).await;
    positions_seen(&everyone, hash(1), 4).await;

    switch_to(&kim, hash(2)).await;
    let views = eventually_views(&everyone, Duration::from_secs(30), |views| {
        views
            .iter()
            .all(|v| v.saved_position.get(&hash(1)) == Some(&400_000))
    })
    .await;
    assert_eq!(views[0].now_playing, Some(hash(2)));

    // Back to file 1: nobody has file 2 loaded (every sample is still
    // tagged file 1), so file 2 gets no saved position.
    switch_to(&baughn, hash(1)).await;
    eventually_views(&everyone, Duration::from_secs(30), |views| {
        views.iter().all(|v| v.now_playing == Some(hash(1)))
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let view = view_of(&chiaki).await;
    assert_eq!(view.saved_position.get(&hash(2)), None);
    assert_eq!(view.saved_position.get(&hash(1)), Some(&400_000));
}

/// An even count averages the two middle positions.
#[tokio::test(start_paused = true)]
async fn an_even_count_averages_the_middle_pair() {
    let harness = Harness::new(0x5A7F);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    let everyone = [&kim, &baughn];
    playlist(&kim, &everyone).await;

    loaded_at(&kim, hash(1), 300_000).await;
    loaded_at(&baughn, hash(1), 500_000).await;
    positions_seen(&everyone, hash(1), 2).await;

    switch_to(&kim, hash(2)).await;
    eventually_views(&everyone, Duration::from_secs(30), |views| {
        views
            .iter()
            .all(|v| v.saved_position.get(&hash(1)) == Some(&400_000))
    })
    .await;
}

/// Past the watched fraction, switching away stores 0, and so does an
/// EOF advance.
#[tokio::test(start_paused = true)]
async fn past_the_watched_fraction_saves_zero() {
    let harness = Harness::new(0x5A80);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    let everyone = [&kim, &baughn];
    playlist(&kim, &everyone).await;

    // An earlier abandonment left a mid-file saved position.
    loaded_at(&kim, hash(1), 600_000).await;
    loaded_at(&baughn, hash(1), 600_000).await;
    positions_seen(&everyone, hash(1), 2).await;
    switch_to(&kim, hash(2)).await;
    eventually_views(&everyone, Duration::from_secs(30), |views| {
        views
            .iter()
            .all(|v| v.saved_position.get(&hash(1)) == Some(&600_000))
    })
    .await;

    // Back, watched into the credits, then abandoned again.
    switch_to(&kim, hash(1)).await;
    loaded_at(&kim, hash(1), PAST_WATCHED).await;
    loaded_at(&baughn, hash(1), PAST_WATCHED).await;
    eventually_views(&everyone, Duration::from_secs(30), |views| {
        views.iter().all(|v| {
            v.now_playing == Some(hash(1))
                && v.playback_position
                    .values()
                    .all(|p| p.position_millis == PAST_WATCHED)
        })
    })
    .await;
    switch_to(&kim, hash(2)).await;
    eventually_views(&everyone, Duration::from_secs(30), |views| {
        views
            .iter()
            .all(|v| v.saved_position.get(&hash(1)) == Some(&0))
    })
    .await;

    // File 2 played to its end: the EOF advance saves 0 for it.
    loaded_at(&kim, hash(2), 1_439_000).await;
    loaded_at(&baughn, hash(2), 1_439_000).await;
    positions_seen(&everyone, hash(2), 2).await;
    report_eof(&kim, hash(2)).await;
    eventually_views(&everyone, Duration::from_secs(30), |views| {
        views
            .iter()
            .all(|v| v.now_playing.is_none() && v.saved_position.get(&hash(2)) == Some(&0))
    })
    .await;
}

/// The last present viewer leaving mid-file saves the position, counting
/// only the users present just before (the leaver included). An earlier
/// leaver's sample no longer counts, and their leaving writes nothing
/// while someone is still watching.
#[tokio::test(start_paused = true)]
async fn the_last_viewer_quitting_saves_their_position() {
    let harness = Harness::new(0x5A81);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);
    let nas = harness.seeder("nas", 3);
    let everyone = [&kim, &baughn, &nas];
    playlist(&kim, &everyone).await;

    loaded_at(&kim, hash(1), 600_000).await;
    loaded_at(&baughn, hash(1), 900_000).await;
    positions_seen(&everyone, hash(1), 2).await;

    quit(&baughn).await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        view_of(&nas).await.saved_position.get(&hash(1)),
        None,
        "kim is still watching: no write"
    );

    quit(&kim).await;
    eventually_views(&[&nas], Duration::from_secs(30), |views| {
        views[0].saved_position.get(&hash(1)) == Some(&600_000)
    })
    .await;
}

/// A last viewer who is Lost (not a graceful quit) saves too.
#[tokio::test(start_paused = true)]
async fn the_last_viewer_lost_saves_their_position() {
    let harness = Harness::new(0x5A82);
    let kim = harness.client("kim", 1);
    let nas = harness.seeder("nas", 2);
    let everyone = [&kim, &nas];
    playlist(&kim, &everyone).await;

    loaded_at(&kim, hash(1), 700_000).await;
    positions_seen(&everyone, hash(1), 1).await;

    harness.isolate("kim");
    eventually_views(&[&nas], Duration::from_secs(60), |views| {
        views[0].saved_position.get(&hash(1)) == Some(&700_000)
    })
    .await;
}
