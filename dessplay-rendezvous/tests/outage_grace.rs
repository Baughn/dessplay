//! Outage grace (design.md, Presence): when someone's connection drops
//! while the video plays, the group plays on without them for
//! `OUTAGE_GRACE_MILLIS`, with pause, seek and file changes locked, so a
//! short outage can be dead-reckoned through. Then everyone pauses,
//! including the dropped client. Full clients against the real server
//! over the sim, paused time.

mod common;

use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay_core::derive::{self, OUTAGE_GRACE_MILLIS};
use dessplay_core::net::Presence;
use dessplay_core::types::{
    AniDbMetadata, AniDbSeriesId, ListEntryId, ListStatus, ManualState, MetadataSource,
    PlaybackIntent, SeekAuthority, SeriesListEntry, SeriesWatchState, UserId,
};

const GRACE: Duration = Duration::from_millis(OUTAGE_GRACE_MILLIS);

/// Start playing `hash(1)` (with `hash(2)` queued after it) for `names`,
/// and wait until every client agrees playback is active.
async fn playing(harness: &Harness, names: &[&str]) -> Vec<dessplay::client::ClientHandle> {
    let clients: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, name)| harness.client(name, i as u128 + 1))
        .collect();
    let first = &clients[0];
    mutate(first, Mutation::PushPlaylist { new: entry(1) }).await;
    mutate(first, Mutation::PushPlaylist { new: entry(2) }).await;
    mutate(
        first,
        Mutation::SetNowPlaying {
            file: Some(hash(1)),
        },
    )
    .await;
    mutate(
        first,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    let refs: Vec<_> = clients.iter().collect();
    eventually(&refs, Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;
    clients
}

/// Commit `user` (Watching) to the series of `hash(1)`.
async fn commit(clients: &[dessplay::client::ClientHandle], user: &str) {
    let series = AniDbSeriesId(42);
    let id = ListEntryId(series.0 as u128);
    mutate(
        &clients[0],
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
        &clients[0],
        Mutation::PutListEntry {
            id,
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
        &clients[0],
        Mutation::SetSeriesPreference {
            user: UserId::new(user),
            entry: id,
            pref: SeriesWatchState::Watching,
            set_by: None,
        },
    )
    .await;
    let refs: Vec<_> = clients.iter().collect();
    eventually(&refs, Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.view
                .series_preference
                .get(&(UserId::new(user), id))
                .map(|p| p.state)
                == Some(SeriesWatchState::Watching)
        })
    })
    .await;
}

/// The headline behaviour, seen from both sides of the outage: the
/// present user plays on with the dropped user listed inside a grace
/// window; the dropped client plays on alone through its own window; when
/// the window ends everyone pauses, the dropped client included.
#[tokio::test(start_paused = true)]
async fn a_drop_mid_play_plays_on_then_pauses_everyone() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "baughn"]).await;
    let (kim, baughn) = (&clients[0], &clients[1]);

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.presence == Presence::Lost && p.grace_until.is_some())
    })
    .await;
    // The dropped client lists *itself* inside its own window.
    eventually(&[kim], Duration::from_secs(30), |snaps| {
        derive::outage_grace(&snaps[0].peers).is_some_and(|g| g.user == UserId::new("kim"))
    })
    .await;

    // Most of the window later, both sides are still playing.
    tokio::time::sleep(GRACE - Duration::from_secs(20)).await;
    for (name, client) in [("kim", kim), ("baughn", baughn)] {
        let snap = snapshot_of(client).await;
        assert!(
            snap.playing(),
            "{name} must still be playing inside the window"
        );
        assert_eq!(snap.view.playback_intent, PlaybackIntent::Playing);
    }

    // The window ends: the server pauses everyone and clears it...
    eventually(&[baughn], Duration::from_secs(60), |snaps| {
        let s = &snaps[0];
        s.view.playback_intent == PlaybackIntent::Paused
            && !s.playing()
            && s.peer("kim").is_some_and(|p| p.grace_until.is_none())
    })
    .await;
    // ...and the dropped client pauses on its own, still offline.
    eventually(&[kim], Duration::from_secs(60), |snaps| {
        !snaps[0].playing() && derive::outage_grace(&snaps[0].peers).is_none()
    })
    .await;

    // On return, kim adopts the server's paused state; a play press then
    // runs for both.
    harness.heal("kim");
    eventually(&[kim, baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.peer("kim")
                .is_some_and(|p| p.presence == Presence::Present)
                && !s.playing()
        })
    })
    .await;
    mutate(
        baughn,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    eventually(&[kim, baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;
}

/// A return inside the window is invisible: nobody ever pauses, and the
/// window closes for everyone.
#[tokio::test(start_paused = true)]
async fn a_return_inside_the_window_never_pauses() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "baughn"]).await;
    let (kim, baughn) = (&clients[0], &clients[1]);

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.grace_until.is_some())
    })
    .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    harness.heal("kim");
    eventually(&[kim, baughn], Duration::from_secs(30), |snaps| {
        snaps.iter().all(|s| {
            s.peer("kim")
                .is_some_and(|p| p.presence == Presence::Present)
                && derive::outage_grace(&s.peers).is_none()
        })
    })
    .await;

    // Well past where the window would have ended, still playing.
    tokio::time::sleep(GRACE + Duration::from_secs(30)).await;
    for client in [kim, baughn] {
        let snap = snapshot_of(client).await;
        assert_eq!(snap.view.playback_intent, PlaybackIntent::Playing);
        assert!(snap.playing());
    }
}

/// A committed (Watching) user's drop no longer stops the video at once:
/// inside the window they do not block; after it they block as a
/// committed-absent user until acknowledged, exactly as before.
#[tokio::test(start_paused = true)]
async fn a_committed_users_drop_plays_on_until_the_window_ends() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "baughn"]).await;
    commit(&clients, "kim").await;
    let baughn = &clients[1];

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.grace_until.is_some())
    })
    .await;
    tokio::time::sleep(Duration::from_secs(90)).await;
    assert!(
        snapshot_of(baughn).await.playing(),
        "a committed user inside the window must not block"
    );

    eventually(&[baughn], GRACE, |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.grace_until.is_none())
    })
    .await;
    mutate(
        baughn,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    let snap = snapshot_of(baughn).await;
    assert!(!snap.playing(), "after the window kim blocks again");
    // A play press into a block is not an unpause: the rejoin anchor waits
    // for playback to actually resume, or a returning kim who is behind
    // would follow it forward and skip video.
    assert_eq!(
        snap.view.seek_authority,
        Some(SeekAuthority::Laggard(hash(1)))
    );
    assert!(
        derive::playback_blockers(&snap.view, &snap.peers)
            .iter()
            .any(|b| b.reason == derive::BlockReason::CommittedAbsent)
    );
    drop(clients);
}

/// Already paused when someone drops: nothing to dead-reckon, so no
/// window opens and nothing is locked.
#[tokio::test(start_paused = true)]
async fn a_drop_while_paused_opens_no_window() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "baughn"]).await;
    let baughn = &clients[1];
    mutate(
        baughn,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Paused,
        },
    )
    .await;
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        !snaps[0].playing()
    })
    .await;

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.presence == Presence::Lost)
    })
    .await;
    // Give a late window every chance to show up.
    tokio::time::sleep(Duration::from_secs(5)).await;
    let snap = snapshot_of(baughn).await;
    assert_eq!(snap.peer("kim").and_then(|p| p.grace_until), None);
    assert_eq!(snap.view.playback_intent, PlaybackIntent::Paused);
    // The dropped client opens no window of its own either.
    let kim = snapshot_of(&clients[0]).await;
    assert_eq!(derive::outage_grace(&kim.peers), None);
}

/// Inside the window every local pause, seek and file change is refused
/// at the write path, whatever surface issued it; unpausing, chat and the
/// rest of the state still go through.
#[tokio::test(start_paused = true)]
async fn pause_seek_and_file_changes_are_refused_inside_the_window() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "baughn"]).await;
    let baughn = &clients[1];
    let me = UserId::new("baughn");

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.grace_until.is_some())
    })
    .await;
    let before = snapshot_of(baughn).await.view;

    for mutation in [
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Paused,
        },
        Mutation::SetManualOverride {
            user: me.clone(),
            state: Some(ManualState::Paused),
        },
        Mutation::SetUserSeek {
            file: hash(1),
            from_millis: 1_000,
            to_millis: 60_000,
        },
        Mutation::SetSeekAuthority {
            authority: user_seek_authority("baughn", 7),
        },
        Mutation::SetNowPlaying {
            file: Some(hash(2)),
        },
        Mutation::SetNowPlaying { file: None },
    ] {
        mutate(baughn, mutation).await;
    }
    // Not refused: re-selecting the current file, and ordinary writes.
    mutate(
        baughn,
        Mutation::SetNowPlaying {
            file: Some(hash(1)),
        },
    )
    .await;
    mutate(
        baughn,
        Mutation::Chat {
            text: "still here".into(),
        },
    )
    .await;

    let after = snapshot_of(baughn).await;
    assert_eq!(after.view.playback_intent, PlaybackIntent::Playing);
    assert_eq!(
        after.view.manual_override.get(&me),
        before.manual_override.get(&me)
    );
    assert_eq!(after.view.seek_authority, before.seek_authority);
    assert_eq!(after.view.seek_authority, Some(SeekAuthority::Server));
    assert_eq!(after.view.now_playing, Some(hash(1)));
    assert!(after.playing());
    assert!(after.view.chat.iter().any(|m| m.text == "still here"));

    // Once the window is over, a pause goes through again.
    eventually(&[baughn], GRACE, |snaps| {
        derive::outage_grace(&snaps[0].peers).is_none()
    })
    .await;
    mutate(
        baughn,
        Mutation::SetManualOverride {
            user: me.clone(),
            state: Some(ManualState::Paused),
        },
    )
    .await;
    eventually(&[baughn], Duration::from_secs(10), |snaps| {
        snaps[0].view.manual_override.get(&me) == Some(&Some(ManualState::Paused))
    })
    .await;
}

/// EOF inside the window: the next episode loads paused as usual and the
/// window closes — otherwise a dropped committed user could be played past
/// on the next episode.
#[tokio::test(start_paused = true)]
async fn eof_inside_the_window_closes_it() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "baughn"]).await;
    let baughn = &clients[1];

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.grace_until.is_some())
    })
    .await;
    report_eof(baughn, hash(1)).await;
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        let s = &snaps[0];
        s.view.now_playing == Some(hash(2))
            && s.view.playback_intent == PlaybackIntent::Paused
            && derive::outage_grace(&s.peers).is_none()
    })
    .await;
    drop(clients);
}

/// Overlapping drops: the first window to end pauses everyone and closes
/// every window, the later drop's included.
#[tokio::test(start_paused = true)]
async fn the_first_window_to_end_pauses_and_closes_them_all() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "amy", "baughn"]).await;
    let baughn = &clients[2];

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.grace_until.is_some())
    })
    .await;
    tokio::time::sleep(Duration::from_secs(60)).await;
    harness.isolate("amy");
    let snaps = eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("amy")
            .is_some_and(|p| p.grace_until.is_some())
    })
    .await;
    let kim_until = snaps[0].peer("kim").and_then(|p| p.grace_until).unwrap();
    let amy_until = snaps[0].peer("amy").and_then(|p| p.grace_until).unwrap();
    assert!(kim_until < amy_until);
    assert!(
        snaps[0].playing(),
        "a second drop inside a window still plays on"
    );

    eventually(&[baughn], GRACE, |snaps| {
        let s = &snaps[0];
        s.view.playback_intent == PlaybackIntent::Paused && derive::outage_grace(&s.peers).is_none()
    })
    .await;
    drop(clients);
}

/// A deliberate quit is not an outage: it still pauses everyone at once
/// and opens no window.
#[tokio::test(start_paused = true)]
async fn a_graceful_quit_still_pauses_at_once() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "baughn"]).await;
    let baughn = &clients[1];
    quit(&clients[0]).await;
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        let s = &snaps[0];
        s.peer("kim")
            .is_some_and(|p| p.presence == Presence::Departed && p.grace_until.is_none())
            && s.view.playback_intent == PlaybackIntent::Paused
    })
    .await;
}

/// What the dropped client's *player* is told, not just its derived
/// state: mpv keeps playing through the client's own window and is
/// commanded to pause when it runs out. Swept over seeds, how long the
/// group played before the drop, and whether the user presses pause in
/// mpv mid-window (undone at once, and the window still ends on time).
#[tokio::test(start_paused = true)]
async fn the_dropped_players_mpv_pauses_when_its_window_runs_out() {
    for (seed, played_for, presses_pause) in [
        (801, 0, false),
        (802, 7, true),
        (803, 41, false),
        (804, 95, true),
    ] {
        dropped_player_pauses_after_grace(seed, Duration::from_secs(played_for), presses_pause)
            .await;
    }
}

async fn dropped_player_pauses_after_grace(seed: u64, played_for: Duration, presses_pause: bool) {
    use dessplay::player::PlayerEvent;
    use dessplay::player::mock::MockCommand;
    use dessplay_core::types::FileAvailability;

    let case = format!("seed {seed}, played {played_for:?}, presses pause: {presses_pause}");
    let harness = Harness::new(seed);
    let mut kim = harness.player_client("kim", 1);
    let baughn = harness.client("baughn", 2);
    let file = media_file(1);
    kim.install(&file);
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
    eventually(&[&kim], Duration::from_secs(20), |snaps| {
        snaps[0]
            .view
            .file_availability
            .values()
            .any(|a| *a == FileAvailability::Ready)
    })
    .await;
    kim.expect_load(Duration::from_secs(20)).await;

    // Kim presses play in mpv; both sides agree it runs.
    kim.user(PlayerEvent::PauseChanged(false));
    eventually(&[&baughn], Duration::from_secs(20), |snaps| {
        snaps[0].playing()
    })
    .await;
    eventually(&[&kim], Duration::from_secs(20), |snaps| snaps[0].playing()).await;
    tokio::time::sleep(played_for).await;
    while kim.control.try_command().is_some() {}

    harness.isolate("kim");
    let dropped_at = tokio::time::Instant::now();

    if presses_pause {
        tokio::time::sleep(Duration::from_secs(30)).await;
        kim.user(PlayerEvent::PauseChanged(true));
        kim.expect_player_command(Duration::from_secs(5), |cmd| {
            matches!(cmd, MockCommand::SetPause(false))
        })
        .await;
        kim.expect_system_line(Duration::from_secs(5), |text| {
            text.starts_with("Can't pause while reconnecting")
        })
        .await;
    }

    // Nothing pauses mpv while the window is open...
    let window_end = dropped_at + GRACE;
    while tokio::time::Instant::now() < window_end - Duration::from_secs(1) {
        while let Some(cmd) = kim.control.try_command() {
            assert!(
                !matches!(cmd, MockCommand::SetPause(true)),
                "{case}: mpv paused {:?} into the window",
                tokio::time::Instant::now() - dropped_at
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // ...and the pause arrives as soon as it runs out, still offline.
    kim.expect_player_command(Duration::from_secs(6), |cmd| {
        matches!(cmd, MockCommand::SetPause(true))
    })
    .await;
    let paused_after = tokio::time::Instant::now() - dropped_at;
    assert!(
        paused_after <= GRACE + Duration::from_secs(5),
        "{case}: mpv paused {paused_after:?} after the drop"
    );
    // It stays paused: no stray unpause while still offline.
    tokio::time::sleep(Duration::from_secs(30)).await;
    while let Some(cmd) = kim.control.try_command() {
        assert!(
            !matches!(cmd, MockCommand::SetPause(false)),
            "{case}: mpv unpaused after the window ran out"
        );
    }
}

/// Any server-forced pause ends the windows, not just their expiry: here
/// someone quits while another's window is open. Left open, the window
/// would keep the paused room locked and excuse the dropped user from
/// gating, so a play press would play past them.
#[tokio::test(start_paused = true)]
async fn a_server_forced_pause_closes_open_windows() {
    let harness = Harness::new(0x6EA5);
    let clients = playing(&harness, &["kim", "amy", "baughn"]).await;
    commit(&clients, "kim").await;
    let baughn = &clients[2];

    harness.isolate("kim");
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.grace_until.is_some())
    })
    .await;
    quit(&clients[1]).await;
    eventually(&[baughn], Duration::from_secs(30), |snaps| {
        let s = &snaps[0];
        s.view.playback_intent == PlaybackIntent::Paused && derive::outage_grace(&s.peers).is_none()
    })
    .await;
    // The window is gone, so committed kim blocks a play press again.
    mutate(
        baughn,
        Mutation::SetPlaybackIntent {
            intent: PlaybackIntent::Playing,
        },
    )
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(!snapshot_of(baughn).await.playing());
    drop(clients);
}

/// After an outage pause the group follows the **furthest-behind** viewer,
/// so nobody skips video. Each side paused on its own clock, so the
/// dropped client comes back ahead of the group (it learned of the drop
/// later) or behind it (sooner). Either way only the player that is ahead
/// seeks, back to the one behind; nobody seeks forward. Pressing play then
/// anchors the group on whoever is furthest behind.
#[tokio::test(start_paused = true)]
async fn after_an_outage_pause_the_furthest_behind_viewer_leads() {
    // The dropped client comes back 30s ahead, then (fresh run) 30s behind.
    for skew in [30_000i64, -30_000] {
        rejoin_follows_the_laggard(skew).await;
    }
}

async fn rejoin_follows_the_laggard(skew: i64) {
    use dessplay::player::PlayerEvent;
    use dessplay::player::mock::MockCommand;
    use dessplay_core::types::FileAvailability;

    let harness = Harness::new(0x1A66);
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
    eventually(&[&kim, &baughn], Duration::from_secs(20), |snaps| {
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
    kim.expect_load(Duration::from_secs(20)).await;
    baughn.expect_load(Duration::from_secs(20)).await;
    kim.user(PlayerEvent::PauseChanged(false));
    eventually(&[&kim, &baughn], Duration::from_secs(20), |snaps| {
        snaps.iter().all(|s| s.playing())
    })
    .await;
    tokio::time::sleep(Duration::from_secs(60)).await;

    // Kim drops; both sides play on, and both pause when the window ends.
    harness.isolate("kim");
    eventually(&[&baughn], GRACE + Duration::from_secs(30), |snaps| {
        snaps[0].view.seek_authority == Some(SeekAuthority::Laggard(file.hash))
            && !snaps[0].playing()
    })
    .await;
    eventually(&[&kim], Duration::from_secs(30), |snaps| {
        !snaps[0].playing()
    })
    .await;
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Kim's player paused `skew` away from the group's.
    let group_at =
        snapshot_of(&baughn).await.view.playback_position[&UserId::new("baughn")].position_millis;
    let kim_at = group_at.saturating_add_signed(skew);
    kim.user(PlayerEvent::Position {
        position_millis: kim_at,
    });
    let (behind, behind_at) = if skew > 0 {
        ("baughn", group_at)
    } else {
        ("kim", kim_at)
    };
    while kim.control.try_command().is_some() {}
    while baughn.control.try_command().is_some() {}

    // Kim comes back. The one ahead seeks back to the one behind...
    harness.heal("kim");
    eventually(&[&baughn], Duration::from_secs(30), |snaps| {
        snaps[0]
            .peer("kim")
            .is_some_and(|p| p.presence == Presence::Present)
    })
    .await;
    let (ahead_client, behind_client) = if skew > 0 {
        (&mut kim, &mut baughn)
    } else {
        (&mut baughn, &mut kim)
    };
    let seek = ahead_client
        .expect_player_command(Duration::from_secs(20), |cmd| {
            matches!(cmd, MockCommand::Seek(_))
        })
        .await;
    let MockCommand::Seek(to) = seek else {
        unreachable!()
    };
    assert!(
        to.abs_diff(behind_at) < 1_000,
        "skew {skew}: the ahead player sought to {to}, not back to {behind}'s {behind_at}"
    );
    // ...and the one behind is never pulled forward.
    tokio::time::sleep(Duration::from_secs(10)).await;
    while let Some(cmd) = behind_client.control.try_command() {
        assert!(
            !matches!(cmd, MockCommand::Seek(to) if to > behind_at + 1_000),
            "skew {skew}: {behind} was pulled forward ({cmd:?}), skipping video"
        );
    }

    // Play: the group anchors on the furthest-behind viewer. By now the
    // two are level (the ahead one sought back), so the anchor may be
    // either; what matters is that it sits where the one behind was.
    baughn.user(PlayerEvent::PauseChanged(false));
    let snaps = eventually(&[&kim, &baughn], Duration::from_secs(20), |snaps| {
        snaps.iter().all(|s| {
            matches!(s.view.seek_authority, Some(SeekAuthority::Anchor { file: f, .. }) if f == file.hash)
                && s.playing()
        })
    })
    .await;
    let Some(SeekAuthority::Anchor { user, .. }) = &snaps[0].view.seek_authority else {
        unreachable!()
    };
    let anchored_at = snaps[0].view.playback_position[user].position_millis;
    assert!(
        anchored_at.abs_diff(behind_at) < 3_000,
        "skew {skew}: anchored on {user} at {anchored_at}, not near {behind}'s {behind_at}"
    );
}
