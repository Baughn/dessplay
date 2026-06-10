//! Phase 5: presence transitions (Present -> Lost -> Departed ->
//! return), graceful quit, and their playback consequences — all with
//! full clients against the real server over the sim, paused time.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay_core::net::{Presence, Role};
use dessplay_core::types::{PlaybackIntent, SeekAuthority, UserId};

/// Get a two-user session playing: now-playing set, intent Playing,
/// both clients agreeing playback is active.
async fn playing_session(
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
        snaps.iter().all(|s| s.playing())
    })
    .await;
    (kim, baughn)
}

/// The full presence ladder: Lost pauses everyone, Departed leaves
/// gating but playback stays paused, return does not auto-resume.
#[tokio::test(start_paused = true)]
async fn presence_ladder_lost_departed_return() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = playing_session(&harness).await;

    // Kim's connection dies (and reconnects are blocked).
    harness.isolate("kim");

    // Baughn sees: kim Lost, intent forced Paused, playback inactive.
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        let s = &snaps[0];
        s.peer("kim").is_some_and(|p| p.presence == Presence::Lost)
            && s.view.playback_intent == PlaybackIntent::Paused
            && !s.playing()
    })
    .await;

    // 30 more silent seconds: kim becomes Departed — out of gating, but
    // the intent latch keeps playback stopped (no auto-unpause).
    eventually(&[&baughn], Duration::from_secs(60), |snaps| {
        let s = &snaps[0];
        s.peer("kim")
            .is_some_and(|p| p.presence == Presence::Departed)
    })
    .await;
    let snap = snapshot_of(&baughn).await;
    assert!(
        dessplay_core::derive::playback_blockers(&snap.view, &snap.peers).is_empty(),
        "a departed user must not gate"
    );
    assert!(!snap.playing(), "playback must stay paused after departure");

    // Kim returns: Present again, still no auto-resume.
    harness.heal("kim");
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.peer("kim")
                .is_some_and(|p| p.presence == Presence::Present)
        })
    })
    .await;
    let snap = snapshot_of(&baughn).await;
    assert!(!snap.playing(), "return must not auto-resume");

    // A human presses play: now it runs again.
    mutate(
        &baughn,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;
}

/// Graceful quit: removed immediately (no Lost stage), playback pauses.
#[tokio::test(start_paused = true)]
async fn graceful_quit_pauses_and_removes() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = playing_session(&harness).await;

    quit(&kim).await;

    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        let s = &snaps[0];
        s.peer("kim").is_none() && s.view.playback_intent == PlaybackIntent::Paused && !s.playing()
    })
    .await;
}

/// A lost (or quitting) seeder never pauses anyone, and shows up with
/// its role in the peer list.
#[tokio::test(start_paused = true)]
async fn seeders_never_pause_playback() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = playing_session(&harness).await;
    let nas = harness.seeder("nas", 3);

    eventually(&[&kim, &baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.peer("nas")
                .is_some_and(|p| p.role == Role::Seeder && p.presence == Presence::Present)
        })
    })
    .await;
    // Keep the handle alive — dropping it would close the command
    // channel, which the network actor reads as a graceful shutdown
    // (Goodbye). We want a *death*, not a quit.
    let _nas = nas;
    harness.isolate("nas");

    // The seeder goes Lost, then Departed; playback never stops.
    eventually(&[&baughn], Duration::from_secs(90), |snaps| {
        let s = &snaps[0];
        s.peer("nas")
            .is_some_and(|p| p.presence == Presence::Departed)
    })
    .await;
    let snap = snapshot_of(&baughn).await;
    assert_eq!(snap.view.playback_intent, PlaybackIntent::Playing);
    assert!(snap.playing(), "a seeder must never pause the party");
}

/// Seek authority follows departures: if the authority's user departs
/// (or quits), the server takes authority so nobody syncs to a ghost.
#[tokio::test(start_paused = true)]
async fn authority_rescued_from_departed_user() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = playing_session(&harness).await;

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

    harness.isolate("kim");
    // Lost alone doesn't move authority; departure does.
    eventually(&[&baughn], Duration::from_secs(90), |snaps| {
        let s = &snaps[0];
        s.peer("kim")
            .is_some_and(|p| p.presence == Presence::Departed)
            && s.view.seek_authority == Some(SeekAuthority::Server)
    })
    .await;
    let _kim = kim; // keep alive: a drop reads as graceful shutdown
}

#[tokio::test(start_paused = true)]
async fn authority_rescued_from_quitting_user() {
    let harness = Harness::new(0x5EED);
    let (kim, baughn) = playing_session(&harness).await;

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

    quit(&kim).await;
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        snaps[0].peer("kim").is_none()
            && snaps[0].view.seek_authority == Some(SeekAuthority::Server)
    })
    .await;
}
