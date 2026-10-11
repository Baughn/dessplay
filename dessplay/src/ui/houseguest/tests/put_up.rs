//! Putting her shelf up (phase 6a, step 5c; design D7c): a stage a game
//! day (one a visit, unfed), counted in her record so a restart doesn't
//! forget it; the first straight after she opens the kit; up only where
//! the frame would show it; arms up, and on a high shelf a little hop as
//! it goes up. On offers, not choices, where the pacing is concerned.
//! Both modes.

use super::kit::{all_but_the_kit, arrived};
use super::*;
use crate::ui::houseguest::ledger::{Dated, Paced};
use crate::ui::houseguest::osaka::{Bucket, HomeEvent};
use crate::ui::houseguest::structures::{Shelf, Structure};

/// The stage of her shelf she's offered to put up this frame (its pane
/// and what it brings it to), if she is.
fn offered(guest: &Guest) -> Option<(Nook, u8)> {
    match &guest.state {
        State::Visiting(visit) => visit.chances.put_up.map(|p| (p.nook, p.to)),
        _ => None,
    }
}

/// Her shelf as her record has it (one in all, in 6a).
fn shelf_of(guest: &Guest) -> Option<Shelf> {
    match guest.ledger.home.structures[..] {
        [Structure::Shelf(shelf)] => Some(shelf),
        _ => None,
    }
}

/// How much of her shelf is up (none: no shelf).
fn built(guest: &Guest) -> Option<u8> {
    shelf_of(guest).map(|s| s.built)
}

/// As if she had just done `event` (the events her acts end on), and the
/// frame after it.
fn did(guest: &mut Guest, real: &Buffer, view: &IdleView, now: &mut u64, event: HomeEvent) {
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.did(event);
    shell_step(guest, real, view, now, true);
}

/// Her kit, boxed on [`rooms`]' List floor beside her TV, at `at` of her
/// week: the kit's precondition asserted.
fn with_her_kit(at: routine::GameTime, graphics: bool) -> Guest {
    let guest = home_at(
        2,
        at,
        &[
            (Furniture::Tv, Nook::Users, 100),
            (Furniture::ShelfKit, Nook::List, 300),
        ],
        graphics,
    );
    assert!(
        guest
            .ledger
            .home
            .props
            .iter()
            .any(|p| p.item == Furniture::ShelfKit && p.boxed()),
        "her kit, boxed"
    );
    guest
}

/// `guest` restarted: what her record says, as a new run of the client
/// reads it (her date and drawing mode as before), with `edit` made to
/// the record first.
fn restarted(guest: &Guest, graphics: bool, edit: impl FnOnce(&mut Ledger)) -> Guest {
    let mut ledger = Ledger::from_json(&guest.ledger.to_json()).expect("reads");
    edit(&mut ledger);
    let mut again = Guest::restore(ledger);
    again.set_date(date(2026, 6, 17));
    if graphics {
        again.set_picker(kitty());
    }
    again
}

/// A stage a game day (phase 6a D7c, the user's pacing), on her offers:
/// her kit opened on a Tuesday afternoon (15:50) on [`rooms`] at 100×30,
/// its first stage is on offer; put up, it's counted in her record for
/// the day, and the next isn't offered for the rest of Tuesday, nor
/// across a restart; at 23:58 still not (restarted then), and once her
/// clock turns past midnight, it is. Unfed, it's one a visit: not again
/// that visit, and on the next, it is. Both modes. Her acts' events are
/// injected (`Osaka::did`) on purpose: the pacing is on offers, and the
/// commit still goes through the paint's re-check; the other tests here
/// run her acts end to end.
#[test]
fn she_puts_up_one_stage_a_game_day() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = with_her_kit(tue(15, 50), graphics);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        did(
            &mut guest,
            &real,
            &view,
            &mut now,
            HomeEvent::Unpacked(Furniture::ShelfKit),
        );
        assert_eq!(built(&guest), Some(0), "{at}: opened, nothing up");
        let nook = shelf_of(&guest).map(|s| s.nook).expect("her shelf");
        assert_eq!(offered(&guest), Some((nook, 1)), "{at}: its first stage");
        did(
            &mut guest,
            &real,
            &view,
            &mut now,
            HomeEvent::PutUp { nook, to: 1 },
        );
        assert_eq!(built(&guest), Some(1), "{at}: up");
        assert_eq!(
            guest.ledger.paced,
            Some(Paced {
                on: Dated::Day(1),
                stages: 1
            }),
            "{at}: counted for Tuesday"
        );
        // The rest of the afternoon: not again.
        let until = now + 60_000;
        while now < until {
            shell_step(&mut guest, &real, &view, &mut now, true);
            assert_eq!(offered(&guest), None, "{at}: again at {now}");
        }
        assert_eq!(built(&guest), Some(1), "{at}");
        // A restart remembers.
        let mut again = restarted(&guest, graphics, |_| {});
        let mut now = until_visiting(&mut again, &real, &view, 0);
        for _ in 0..20 {
            shell_step(&mut again, &real, &view, &mut now, true);
            assert_eq!(offered(&again), None, "{at}: after a restart, at {now}");
        }
        // Late on Tuesday, restarted: not yet. Past midnight: Wednesday's.
        let mut late = restarted(&guest, graphics, |ledger| {
            ledger.clock = tue(23, 58).minutes();
        });
        let mut now = until_visiting(&mut late, &real, &view, 0);
        let mut wednesday = None;
        while wednesday.is_none() {
            assert!(now < 120_000, "{at}: never Wednesday");
            shell_step(&mut late, &real, &view, &mut now, true);
            let day = visit_of(&late).osaka.day(now).map(|d| d.day);
            match day {
                Some(1) => assert_eq!(offered(&late), None, "{at}: Tuesday, at {now}"),
                Some(2) => wednesday = offered(&late),
                other => panic!("{at}: {other:?}"),
            }
        }
        assert_eq!(wednesday, Some((nook, 2)), "{at}: Wednesday's stage");
        // Unfed: one a visit.
        let mut unfed = with_her_kit(tue(15, 50), graphics).unfed();
        let mut now = until_visiting(&mut unfed, &real, &view, 0);
        did(
            &mut unfed,
            &real,
            &view,
            &mut now,
            HomeEvent::Unpacked(Furniture::ShelfKit),
        );
        let visits = unfed.ledger.visits;
        did(
            &mut unfed,
            &real,
            &view,
            &mut now,
            HomeEvent::PutUp { nook, to: 1 },
        );
        assert_eq!(built(&unfed), Some(1), "{at}: unfed, up");
        assert_eq!(
            unfed.ledger.paced,
            Some(Paced {
                on: Dated::Visit(visits),
                stages: 1
            }),
            "{at}: unfed, counted for the visit"
        );
        for _ in 0..20 {
            shell_step(&mut unfed, &real, &view, &mut now, true);
            assert_eq!(offered(&unfed), None, "{at}: unfed, again at {now}");
        }
        unfed.activity(now);
        let mut now = until_visiting(&mut unfed, &real, &view, now + 1);
        shell_step(&mut unfed, &real, &view, &mut now, true);
        assert_eq!(unfed.ledger.visits, visits + 1, "{at}: her next visit");
        assert_eq!(offered(&unfed), Some((nook, 2)), "{at}: unfed, next visit");
    }
}

/// Her kit opened, she puts its first stage up straight after (phase 6a
/// D7c, chained as a crumple is to its use): the decision after it is
/// that, carrying on, not a choice among her offers; she reaches up
/// ("Up it goes...", arms up) and the half nearer its wall is up. The
/// channel's kit delivered to her (everything else hers), on [`rooms`]
/// at 100×30. Both modes.
#[test]
fn unpacking_a_kit_puts_its_first_half_up() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = all_but_the_kit(4, graphics);
        guest.ledger.ordered = Some(Furniture::ShelfKit);
        arrived(&mut guest, &real, &view);
        let mut now = 0;
        // Delivered, and opened.
        while built(&guest).is_none() {
            assert!(now < 10 * 60_000, "{at}: never opened");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let opened = now;
        assert_eq!(built(&guest), Some(0), "{at}: opened at {opened}");
        // The decision after it, and her putting it up.
        let (mut stretched, mut said) = (false, false);
        while built(&guest) != Some(1) {
            assert!(now < opened + 60_000, "{at}: never put it up");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if osaka.act_name() == "PutUp" {
                let (pose, _, bubble) = osaka.appearance(now);
                stretched |= pose == Pose::Stretch;
                said |= bubble == Some(osaka::Bubble::Say("Up it goes..."));
            }
        }
        let osaka = &visit_of(&guest).osaka;
        let next = osaka
            .decisions
            .iter()
            .find(|d| d.at >= opened)
            .unwrap_or_else(|| panic!("{at}: no decision since {opened}"));
        assert_eq!(
            (next.bucket, next.method),
            (Bucket::Continuation, "put-up/stage"),
            "{at}: {next:?}"
        );
        assert!(stretched, "{at}: never reached up");
        assert!(said, "{at}: never said it");
        // The half nearer its wall: its bracket and four of its run.
        let shelf = shelf_of(&guest).expect("her shelf");
        let pane = view
            .nooks
            .iter()
            .find(|&&(n, _)| n == shelf.nook)
            .expect("its pane")
            .1;
        let run = shelf.run(room::pane_raw(pane)).expect("it hangs");
        let frame = paint(&mut guest, &real, &view, now + 1);
        let y = u16::try_from(run.y).expect("on screen");
        let row: String = (run.from..run.to)
            .map(|x| {
                frame[(u16::try_from(x).expect("on screen"), y)]
                    .symbol()
                    .to_owned()
            })
            .collect();
        let half = match shelf.anchor.side {
            room::Side::Left => "┬────     ",
            room::Side::Right => "     ────┬",
        };
        assert_eq!(row, half, "{at}");
        // Once: the chain is that decision's alone. Her next stage (today's
        // forgotten) is a choice of hers like any.
        let up = now;
        guest.ledger.paced = None;
        let second = loop {
            assert!(now < up + 10 * 60_000, "{at}: never chose the second");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if let Some(d) = osaka
                .decisions
                .iter()
                .find(|d| d.at > up && d.method == "put-up/stage")
            {
                break d.clone();
            }
        };
        assert_ne!(second.bucket, Bucket::Continuation, "{at}: {second:?}");
    }
}

/// A stage of her shelf is up only where the frame would show it (phase
/// 6a D7c), each stage and its margin. A word comes over where its first
/// stage goes while she's on her way: she never sets about it while the
/// word's there. Gone, she sets about it on a clear wall, the word comes
/// back as she works, and her act runs to its end but nothing is up (nor
/// offered again while the word's there); the word gone, it's offered,
/// and she puts it up. The next day's stage likewise, with a word over
/// the half it puts up, then on the far end's margin: never offered, and
/// put up there it isn't up; gone, it's offered, and it goes up. The
/// stage's shelf on [`rooms`] at 100×30. Both modes.
#[test]
fn a_stage_is_never_committed_over_text() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(6);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::PutUp);
        paint(&mut guest, &real, &view, 0);
        let note = guest.cue_note().cloned();
        assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
        let shelf = shelf_of(&guest).expect("her shelf");
        assert_eq!(shelf.built, 0, "{at}");
        let pane = view
            .nooks
            .iter()
            .find(|&&(n, _)| n == shelf.nook)
            .expect("its pane")
            .1;
        let run = shelf.run(room::pane_raw(pane)).expect("it hangs");
        // The frame with a letter of text at `x` on its run's row.
        let worded = |x: i32| {
            let mut worded = real.clone();
            worded.set_string(
                u16::try_from(x).expect("on screen"),
                u16::try_from(run.y).expect("on screen"),
                "a",
                Style::new(),
            );
            worded
        };
        let first = Shelf { built: 1, ..shelf }.built_on(run);
        let over_first = worded(first[first.len() / 2].0);
        // On her way to it, the word comes: she never sets about it.
        assert!(
            visit_of(&guest)
                .osaka
                .act_summary()
                .contains("her shelf's stage 1"),
            "{at}: on her way: {}",
            visit_of(&guest).osaka.act_summary()
        );
        let mut now = 0;
        while now < 12_000 {
            shell_step(&mut guest, &over_first, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            assert_ne!(osaka.act_name(), "PutUp", "{at}: set about it at {now}");
            assert_ne!(
                osaka.appearance(now).2,
                Some(osaka::Bubble::Say("Up it goes...")),
                "{at}: said it at {now}"
            );
        }
        assert_eq!(built(&guest), Some(0), "{at}");
        // At it on a clear wall, arms up; the word comes back.
        shell_step(&mut guest, &real, &view, &mut now, true);
        assert_eq!(offered(&guest), Some((shelf.nook, 1)), "{at}: on offer");
        guest.cue(Scene::PutUp);
        while visit_of(&guest).osaka.act_name() != "PutUp" {
            assert!(now < 60_000, "{at}: never set about it");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let began = visit_of(&guest).osaka.act_started();
        let until = now + 12_000;
        let mut ended = None;
        while now < until {
            shell_step(&mut guest, &over_first, &view, &mut now, true);
            if ended.is_none() && visit_of(&guest).osaka.act_name() != "PutUp" {
                ended = Some(now);
            }
            assert_eq!(built(&guest), Some(0), "{at}: up over the word at {now}");
            assert_eq!(
                offered(&guest),
                None,
                "{at}: offered over the word at {now}"
            );
        }
        // Her act ran to its end under the word (the frame refused it,
        // not her).
        let ended = ended.unwrap_or_else(|| panic!("{at}: still at it"));
        assert!(
            ended >= began + osaka::PUT_UP_MS,
            "{at}: cut short at {ended} (from {began})"
        );
        // Gone: offered, and up.
        shell_step(&mut guest, &real, &view, &mut now, true);
        assert_eq!(
            offered(&guest),
            Some((shelf.nook, 1)),
            "{at}: offered again"
        );
        guest.cue(Scene::PutUp);
        let until = now + 60_000;
        while built(&guest) != Some(1) {
            assert!(now < until, "{at}: never put it up");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        // The next day's stage (today's forgotten): a word over the half
        // it puts up, then on the margin past its far end.
        guest.ledger.paced = None;
        let whole = Shelf { built: 2, ..shelf }.built_on(run);
        let second: Vec<i32> = whole
            .iter()
            .map(|&(x, _)| x)
            .filter(|x| !first.iter().any(|(at, _)| at == x))
            .collect();
        let far = match shelf.anchor.side {
            room::Side::Left => whole.last().expect("a run").0 + structures::MARGIN,
            room::Side::Right => whole.first().expect("a run").0 - structures::MARGIN,
        };
        for (what, x) in [("its half", second[second.len() / 2]), ("its margin", far)] {
            let over = worded(x);
            for _ in 0..20 {
                shell_step(&mut guest, &over, &view, &mut now, true);
                assert_eq!(
                    offered(&guest),
                    None,
                    "{at}: the second, a word over {what} at {now}"
                );
            }
            did(
                &mut guest,
                &over,
                &view,
                &mut now,
                HomeEvent::PutUp {
                    nook: shelf.nook,
                    to: 2,
                },
            );
            assert_eq!(
                built(&guest),
                Some(1),
                "{at}: the second up, a word over {what}"
            );
        }
        shell_step(&mut guest, &real, &view, &mut now, true);
        assert_eq!(
            offered(&guest),
            Some((shelf.nook, 2)),
            "{at}: the second, offered"
        );
        did(
            &mut guest,
            &real,
            &view,
            &mut now,
            HomeEvent::PutUp {
                nook: shelf.nook,
                to: 2,
            },
        );
        assert_eq!(built(&guest), Some(2), "{at}: the second, up");
    }
}

/// What she put up is the frame's to take before it's hers to choose
/// again (phase 6a D7c): put up with no paint for a while after (her
/// mind runs on, the frame doesn't come), she never chooses that stage
/// again meanwhile; then the frame takes it, and she has lost nothing
/// (no glance owed at where she was heading). And where the frame
/// refuses it (text come over where it goes), she glances at where it
/// would have gone. The
/// stage's shelf on [`rooms`] at 100×30. Both modes.
#[test]
fn she_waits_for_the_frame_to_take_what_she_put_up() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        for refused in [false, true] {
            let at = format!("graphics={graphics} refused={refused}");
            let mut guest = Guest::new(6);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.cue(Scene::PutUp);
            paint(&mut guest, &real, &view, 0);
            let shelf = shelf_of(&guest).expect("her shelf");
            let pane = view
                .nooks
                .iter()
                .find(|&&(n, _)| n == shelf.nook)
                .expect("its pane")
                .1;
            let run = shelf.run(room::pane_raw(pane)).expect("it hangs");
            let mut now = 0;
            while visit_of(&guest).osaka.act_name() != "PutUp" {
                assert!(now < 30_000, "{at}: never set about it");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let done = visit_of(&guest).osaka.act_started() + osaka::PUT_UP_MS;
            // Her mind runs on, unpainted, 5 s past her act's end.
            while now < done + 5_000 {
                now += 50;
                guest.advance(now);
            }
            let osaka = &visit_of(&guest).osaka;
            let again: Vec<_> = osaka
                .decisions
                .iter()
                .filter(|d| d.at >= done && d.method == "put-up/stage")
                .collect();
            assert!(again.is_empty(), "{at}: chose it again: {again:?}");
            let beats = osaka.beats.len();
            let frame = if refused {
                let half = Shelf { built: 1, ..shelf }.built_on(run);
                let mut worded = real.clone();
                worded.set_string(
                    u16::try_from(half[half.len() / 2].0).expect("on screen"),
                    u16::try_from(run.y).expect("on screen"),
                    "a",
                    Style::new(),
                );
                worded
            } else {
                real.clone()
            };
            paint(&mut guest, &frame, &view, now);
            assert_eq!(
                built(&guest),
                Some(u8::from(!refused)),
                "{at}: as the frame took it"
            );
            let owed = &visit_of(&guest).osaka.beats[beats..];
            if refused {
                // At the middle of the half it would have put up.
                let half = Shelf { built: 1, ..shelf }.built_on(run);
                let middle = (half[0].0 + half[half.len() - 1].0 + 1) / 2;
                assert_eq!(owed.len(), 1, "{at}: {owed:?}");
                assert_eq!(owed[0].toward, (middle, run.y), "{at}: {owed:?}");
            } else {
                assert!(owed.is_empty(), "{at}: owes {owed:?}");
            }
        }
    }
}

/// A stage of a high shelf up, she hops (phase 6a D7c, the user's "a
/// little hop"): the frame it's taken on starts one beat of her jumping
/// jack's lifted frame, about 400 ms; a low shelf is in her reach, and
/// she doesn't. The stage's shelf: high on [`rooms`] at 100×30, low on
/// three panes 10 rows tall. Both modes.
#[test]
fn she_hops_up_to_a_high_shelf() {
    let frames = [
        ("high", rooms_frame(100, 30)),
        ("low", super::shelf::low_rooms()),
    ];
    for (name, (real, view)) in frames {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = Guest::new(7);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.cue(Scene::PutUp);
            paint(&mut guest, &real, &view, 0);
            let shelf = shelf_of(&guest).expect("her shelf");
            assert_eq!(shelf.high(), name == "high", "{at}: {shelf:?}");
            let mut now = 0;
            while built(&guest) != Some(1) {
                assert!(now < 60_000, "{at}: never put it up");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            // From the frame it's taken on, each frame for 3 s.
            let up = now;
            let hop = Pose::Jack(1);
            let mut hopped = Vec::new();
            while now < up + 3_000 {
                let osaka = &visit_of(&guest).osaka;
                if osaka.act_name() == "Hop" && osaka.appearance(now).0 == hop {
                    hopped.push(now);
                }
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            if name == "high" {
                assert_eq!(hopped.first(), Some(&up), "{at}: hopped {hopped:?}");
                let last = hopped.last().copied().unwrap_or(up);
                assert!(last < up + 600, "{at}: a beat, not more: {hopped:?}");
            } else {
                assert!(hopped.is_empty(), "{at}: hopped {hopped:?}");
            }
            assert_eq!(built(&guest), Some(1), "{at}");
        }
    }
}
