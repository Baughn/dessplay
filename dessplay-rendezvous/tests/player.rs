//! Phase 7 milestone scenarios: full player clients (session shell +
//! MockPlayer) against the real server. The mock control plays the
//! user's role inside mpv — pressing space, scrubbing, reaching EOF —
//! and the assertions watch both the synced state and what every
//! *player* was actually told to do.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay::player::PlayerEvent;
use dessplay::player::mock::MockCommand;
use dessplay_core::types::{FileAvailability, ManualState, PlaybackIntent, UserId};

const BUDGET: Duration = Duration::from_secs(20);

/// Both clients have the file, kim adds it and presses play in their
/// player; both players are told to unpause. Then baughn pauses in
/// *their* player: everyone's player pauses, and the state shows who.
#[tokio::test(start_paused = true)]
async fn pause_in_one_player_pauses_everyone() {
    let harness = Harness::new(701);
    let mut kim = harness.player_client("kim", 1);
    let mut baughn = harness.player_client("baughn", 2);
    let file = media_file(1);
    kim.install(&file);
    baughn.install(&file);

    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&file, "kim"),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(file.hash),
        },
    )
    .await;

    // Both matchers verify their local copies; both players load.
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            s.view
                .file_availability
                .values()
                .filter(|a| **a == FileAvailability::Ready)
                .count()
                == 2
        })
    })
    .await;
    kim.expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;
    baughn
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;

    // Kim presses play in mpv. The auto-mock acked the unpause, so this
    // arrives as a user unpause: intent latches Playing, gating passes,
    // and *baughn's* player is told to unpause.
    kim.user(PlayerEvent::PauseChanged(false));
    baughn
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::SetPause(false)))
        .await;
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;

    // Baughn pauses. Kim's player is re-paused, and the override shows
    // who is blocking.
    baughn.user(PlayerEvent::PauseChanged(true));
    kim.expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::SetPause(true)))
        .await;
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            !s.playing()
                && s.view.manual_override.get(&UserId::new("baughn"))
                    == Some(&Some(ManualState::Paused))
        })
    })
    .await;
}

/// A user seek on kim's player takes seek authority and drags baughn's
/// player to the position; kim is never told to follow themself.
#[tokio::test(start_paused = true)]
async fn seek_follows_the_authority() {
    let harness = Harness::new(702);
    let mut kim = harness.player_client("kim", 1);
    let mut baughn = harness.player_client("baughn", 2);
    let file = media_file(1);
    kim.install(&file);
    baughn.install(&file);

    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&file, "kim"),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(file.hash),
        },
    )
    .await;
    kim.expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;
    baughn
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;
    // Baughn's player needs a known position before drift correction
    // can act (a fresh load has none until the player reports one).
    baughn.user(PlayerEvent::Position { position_millis: 0 });

    // Kim scrubs to the minute mark; the debounce (1.5s) coalesces it.
    kim.user(PlayerEvent::Seeked {
        position_millis: 60_000,
    });

    // Baughn's player is dragged there (a hard seek: 60s >> the 3s
    // band). Paused playback means no extrapolation drift beyond the
    // sim-time delivery lag.
    let cmd = baughn
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Seek(_)))
        .await;
    let MockCommand::Seek(position) = cmd else {
        unreachable!()
    };
    assert!(
        (60_000..65_000).contains(&position),
        "baughn dragged to {position}, expected ~60000"
    );

    // The state agrees about who has authority.
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            s.view.seek_authority
                == Some(dessplay_core::types::SeekAuthority::User(UserId::new(
                    "kim",
                )))
        })
    })
    .await;

    // Kim must never have been told to follow their own authority.
    let kim_cmds = kim.control.drain_commands();
    assert!(
        !kim_cmds.iter().any(|c| matches!(c, MockCommand::Seek(_))),
        "kim was told to follow themself: {kim_cmds:#?}"
    );
}

/// EOF on a watching client: the server marks the file watched,
/// advances now-playing, and everyone's player loads the next file —
/// paused, awaiting a human.
#[tokio::test(start_paused = true)]
async fn eof_advances_and_everyone_loads_the_next_file() {
    let harness = Harness::new(703);
    let mut kim = harness.player_client("kim", 1);
    let mut baughn = harness.player_client("baughn", 2);
    let ep1 = media_file(1);
    let ep2 = media_file(2);
    for client in [&kim, &baughn] {
        client.install(&ep1);
        client.install(&ep2);
    }

    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&ep1, "kim"),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&ep2, "kim"),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(ep1.hash),
        },
    )
    .await;
    kim.expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;
    baughn
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;
    kim.user(PlayerEvent::PauseChanged(false));
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;

    // Kim's player hits the end of the episode.
    kim.user(PlayerEvent::Eof);

    // Server owns the transition: ep1 watched, ep2 now-playing, intent
    // paused. Both players load ep2.
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            s.view.now_playing == Some(ep2.hash)
                && s.view.watched.get(&ep1.hash) == Some(&true)
                && s.view.playback_intent == PlaybackIntent::Paused
        })
    })
    .await;
    let loaded = kim
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;
    assert!(
        matches!(&loaded, MockCommand::Load(path, _) if path.ends_with("ep2.mkv")),
        "kim loaded {loaded:?}"
    );
    let loaded = baughn
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;
    assert!(
        matches!(&loaded, MockCommand::Load(path, _) if path.ends_with("ep2.mkv")),
        "baughn loaded {loaded:?}"
    );
}

/// The optimist re-pause (design rule 2): a peer blocks (here, Paused),
/// and kim's attempt to play is immediately re-paused via the observe-
/// and-correct round trip, with the intent latched Playing so playback
/// resumes the moment the blocker clears. (Pre-9B this used a
/// permanently-missing file; with downloading as the default a missing
/// file is now fetched instead — see `a_missing_file_is_downloaded_from_a_peer`.)
#[tokio::test(start_paused = true)]
async fn optimist_is_repaused_while_a_peer_blocks() {
    let harness = Harness::new(704);
    let mut kim = harness.player_client("kim", 1);
    let baughn = harness.player_client("baughn", 2);
    let file = media_file(1);
    kim.install(&file);
    baughn.install(&file); // both have it, so nothing downloads

    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&file, "kim"),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(file.hash),
        },
    )
    .await;
    // Baughn is paused (stepped away): a deterministic blocker.
    mutate(
        &baughn,
        Mutation::SetManualOverride {
            user: UserId::new("baughn"),
            state: Some(ManualState::Paused),
        },
    )
    .await;
    kim.expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;

    // Kim presses play anyway.
    kim.user(PlayerEvent::PauseChanged(false));

    // Rule 2: kim's player is immediately re-paused, kim is marked ready
    // (intent latched Playing), and baughn is named as the blocker.
    kim.expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::SetPause(true)))
        .await;
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            !s.playing()
                && s.view.playback_intent == PlaybackIntent::Playing
                && dessplay_core::derive::playback_blockers(&s.view, &s.peers)
                    .iter()
                    .any(|b| {
                        b.user == UserId::new("baughn")
                            && b.reason == dessplay_core::derive::BlockReason::Paused
                    })
        })
    })
    .await;
}

/// A peer without the file downloads it from a peer who has it, through
/// the real relay, and ends up Ready — the full Phase 9B path wired
/// through the session (resolve Missing -> StartDownload -> relayed
/// chunk transfer -> verified -> Ready).
#[tokio::test(start_paused = true)]
async fn a_missing_file_is_downloaded_from_a_peer() {
    let harness = Harness::new(706);
    let seed = harness.player_client("seed", 1);
    let leech = harness.player_client("leech", 2);
    let file = media_file(1);
    seed.install(&file); // only the seed has it

    mutate(
        &seed,
        Mutation::PushPlaylist {
            new: file_entry(&file, "seed"),
        },
    )
    .await;
    mutate(
        &seed,
        Mutation::SetNowPlaying {
            file: Some(file.hash),
        },
    )
    .await;

    // The seed verifies its copy (Ready); the leech finds it missing,
    // downloads it from the seed through the relay, and becomes Ready.
    eventually(&[&seed, &leech], BUDGET, |snaps| {
        let ready = |s: &ClientSnapshot, who: &str| {
            s.view.file_availability.get(&(UserId::new(who), file.hash))
                == Some(&FileAvailability::Ready)
        };
        snaps.iter().all(|s| ready(s, "seed") && ready(s, "leech"))
    })
    .await;
}

/// Prefetch: the leecher fetches not just the now-playing file but the
/// next queued entry too, so it's local before it's needed (design.md,
/// Pre-fetching).
#[tokio::test(start_paused = true)]
async fn queued_entries_are_prefetched_ahead_of_now_playing() {
    let harness = Harness::new(707);
    let seed = harness.player_client("seed", 1);
    let leech = harness.player_client("leech", 2);
    let ep1 = media_file(1);
    let ep2 = media_file(2);
    seed.install(&ep1);
    seed.install(&ep2);

    for ep in [&ep1, &ep2] {
        mutate(
            &seed,
            Mutation::PushPlaylist {
                new: file_entry(ep, "seed"),
            },
        )
        .await;
    }
    mutate(
        &seed,
        Mutation::SetNowPlaying {
            file: Some(ep1.hash),
        },
    )
    .await;

    // The leecher ends up Ready for BOTH — ep1 (now-playing) and ep2
    // (prefetched, never now-playing).
    eventually(&[&leech], BUDGET, |snaps| {
        let ready = |s: &ClientSnapshot, h| {
            s.view.file_availability.get(&(UserId::new("leech"), h))
                == Some(&FileAvailability::Ready)
        };
        snaps
            .iter()
            .all(|s| ready(s, ep1.hash) && ready(s, ep2.hash))
    })
    .await;
}

/// A now-playing file whose series is *unknown* to a user (empty watch
/// history) but carries an AniDB series id, that **no present peer holds**:
/// the missing user is auto-marked NotWatching, sees the placeholder, and
/// stops gating — so an `unpause` actually starts playback rather than
/// deadlocking on a file nobody can supply (design.md, File State,
/// unknown-series branch).
///
/// "No present peer holds it" is load-bearing: the auto-NotWatch is
/// deliberately *suppressed* when the file is obtainable (a present peer
/// advertises it Ready), because then it just downloads — that path is
/// covered by `a_missing_file_is_downloaded_from_a_peer`. So here nobody
/// installs the file: kim adds it by identity (as the catalog allows) and
/// both clients, lacking it and not knowing the series, opt out.
#[tokio::test(start_paused = true)]
async fn missing_unknown_series_auto_not_watching_lets_the_group_play() {
    use dessplay_core::types::{AniDbMetadata, AniDbSeriesId, MetadataSource, SeriesWatchState};

    let harness = Harness::new(705);
    let kim = harness.player_client("kim", 1);
    let mut baughn = harness.player_client("baughn", 2);
    let file = media_file(1);
    // Nobody installs `file`: no present peer can serve it, so the
    // auto-NotWatch path is exercised instead of a download. Neither client
    // has watch history for the series, so it is "unknown" to both.

    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&file, "kim"),
        },
    )
    .await;
    // Metadata with a real series id (as the AniDB worker would write).
    mutate(
        &kim,
        Mutation::SetAniDbMetadata {
            hash: file.hash,
            metadata: Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Some Obscure Show".into(),
                series_id: Some(AniDbSeriesId(4242)),
                episode_number: Some("1".into()),
            }),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(file.hash),
        },
    )
    .await;

    // Both users auto-mark NotWatching for the unknown, unobtainable series.
    for who in ["kim", "baughn"] {
        eventually(&[&kim, &baughn], BUDGET, |snaps| {
            snaps.iter().all(|s| {
                s.view
                    .series_preference
                    .get(&(UserId::new(who), AniDbSeriesId(4242)))
                    == Some(&SeriesWatchState::NotWatching)
            })
        })
        .await;
    }

    // Baughn's player is handed the placeholder (a Load), not the real
    // file (which nobody has).
    baughn
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;

    // An unpause now starts playback: nobody gates (all NotWatching), so
    // the group plays on placeholders instead of deadlocking.
    kim.user(PlayerEvent::PauseChanged(false));
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;
}

/// Regression (the "Kill Ao" incident): a client showing a placeholder
/// (it does not hold the real now-playing video) must never seize seek
/// authority. Pre-fix, a seek in such a client — a scrub of the
/// placeholder, or the side effect of dragging a file into mpv — took
/// authority and froze the whole group on that client's bogus position
/// (everyone hard-seeking back every couple of seconds). Reuses the
/// unobtainable-unknown-series setup so both clients sit on placeholders
/// deterministically (no downloads to race).
#[tokio::test(start_paused = true)]
async fn placeholder_client_cannot_take_seek_authority() {
    use dessplay_core::types::{
        AniDbMetadata, AniDbSeriesId, MetadataSource, SeekAuthority, SeriesWatchState,
    };

    let harness = Harness::new(707);
    let kim = harness.player_client("kim", 1);
    let baughn = harness.player_client("baughn", 2);
    let file = media_file(1);
    // Nobody installs it: unobtainable unknown series → both NotWatching,
    // both on placeholders (neither holds the real video).

    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&file, "kim"),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetAniDbMetadata {
            hash: file.hash,
            metadata: Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Some Obscure Show".into(),
                series_id: Some(AniDbSeriesId(4242)),
                episode_number: Some("1".into()),
            }),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(file.hash),
        },
    )
    .await;

    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            s.view
                .series_preference
                .get(&(UserId::new("baughn"), AniDbSeriesId(4242)))
                == Some(&SeriesWatchState::NotWatching)
        })
    })
    .await;

    // The group plays on placeholders (nobody gates).
    kim.user(PlayerEvent::PauseChanged(false));
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;

    // baughn scrubs / drags against their placeholder — a Seeked for a file
    // they do not hold.
    baughn.user(PlayerEvent::Seeked {
        position_millis: 990_000,
    });
    // Past the 1.5s seek debounce: a pre-fix client would have published a
    // UserSeeked and taken authority by now.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let snap = snapshot_of(&baughn).await;
    assert_ne!(
        snap.view.seek_authority,
        Some(SeekAuthority::User(UserId::new("baughn"))),
        "a placeholder client must never take seek authority"
    );
    // The group is still playing, not frozen on a bogus position.
    eventually(&[&kim, &baughn], BUDGET, |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;
}

/// Regression: a user who lacks the now-playing file in any media root can
/// clear the Missing state by loading the correctly-named file directly
/// into their player (drag-and-drop), even from outside the media roots.
/// dessplay observes mpv's `path`, matches the basename to the now-playing
/// entry, and adopts it as a manual mapping → Ready + a real load.
#[tokio::test(start_paused = true)]
async fn dragging_the_right_file_clears_missing() {
    use dessplay_core::types::{AniDbMetadata, AniDbSeriesId, MetadataSource, SeriesWatchState};

    let harness = Harness::new(708);
    let kim = harness.player_client("kim", 1);
    let mut dagger = harness.player_client("dagger", 2);
    let file = media_file(1); // filename ep1.mkv
    // dagger does NOT install it: no filename match under their media root.

    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: file_entry(&file, "kim"),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetAniDbMetadata {
            hash: file.hash,
            metadata: Some(AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Some Obscure Show".into(),
                series_id: Some(AniDbSeriesId(4242)),
                episode_number: Some("1".into()),
            }),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(file.hash),
        },
    )
    .await;

    // dagger lacks the unobtainable unknown-series file → placeholder.
    eventually(&[&dagger], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            s.view
                .series_preference
                .get(&(UserId::new("dagger"), AniDbSeriesId(4242)))
                == Some(&SeriesWatchState::NotWatching)
        })
    })
    .await;
    dagger
        .expect_player_command(BUDGET, |cmd| matches!(cmd, MockCommand::Load(..)))
        .await;

    // The user drags the correctly-named file into mpv, from outside any
    // media root.
    let drop_dir = tempfile::tempdir().unwrap();
    let dropped = drop_dir.path().join(&file.filename);
    std::fs::write(&dropped, &file.contents).unwrap();
    dagger.user(PlayerEvent::PathChanged {
        path: dropped.to_string_lossy().into_owned(),
    });

    // dagger now holds the file: availability flips to Ready...
    eventually(&[&dagger], BUDGET, |snaps| {
        snaps.iter().all(|s| {
            s.view
                .file_availability
                .get(&(UserId::new("dagger"), file.hash))
                == Some(&FileAvailability::Ready)
        })
    })
    .await;
    // ...and the real file (the dropped path) is loaded into their player.
    let want = dropped.clone();
    dagger
        .expect_player_command(
            BUDGET,
            |cmd| matches!(cmd, MockCommand::Load(p, _) if *p == want),
        )
        .await;
}
