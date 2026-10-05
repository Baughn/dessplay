//! Her wall clock and her window (phase 5b D7, step 8a): pieces that
//! tell her time of day (the clock's dial to the quarter-hour, the
//! window's sky by its phase), from her game clock, never her script;
//! never mirrored; changing on the quarter-hour, with a wakeup for it
//! only while one is shown; and the clock a gift that comes once. Each
//! in both drawing modes, under quiet panes and text-dense ones.

use super::*;
use crate::ui::houseguest::art::{Dial, PieceState, Sky};

/// Her home on [`home_screen`]: the sofa and TV in the Users pane, her
/// bed in the Playlist pane.
const HOME: [(Furniture, Nook, u16); 3] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Tv, Nook::Users, 800),
    (Furniture::Bed, Nook::Playlist, 300),
];

/// The two-pane home, quiet and with chat text above (the wall over her
/// floor clear, so what hangs there shows), by name.
fn screens() -> [(&'static str, (Buffer, IdleView)); 2] {
    [("quiet", home_screen()), ("busy", busy_home_screen())]
}

/// [`HOME`] at `at` with her wall clock and window hung on the Users
/// wall, facing `facing` (her clock long since sent).
fn timed_home(seed: u64, at: routine::GameTime, facing: sprite::Facing, graphics: bool) -> Guest {
    let mut guest = home_at(seed, at, &HOME, graphics);
    for (item, x) in [(Furniture::Clock, 150), (Furniture::Window, 550)] {
        assert!(
            guest
                .ledger
                .home
                .add(room::Prop::new(item, Nook::Users, x, facing))
        );
    }
    guest
}

/// What `item` shows in the last frame (as the visit, or her empty home,
/// painted it), if it's shown.
fn looks_of(guest: &Guest, item: Furniture) -> Option<PieceState> {
    let (shown, looks) = match &guest.state {
        State::Visiting(visit) => (&visit.shown, &visit.looks),
        State::Away(empty) => (&empty.shown, &empty.looks),
        _ => return None,
    };
    shown
        .iter()
        .any(|s| s.item == item && !s.boxed)
        .then(|| looks.state(item))
}

/// Her game minute of the day at `now`.
fn minute_at(guest: &Guest, now: u64) -> u16 {
    let clock = guest.game_clock(now).expect("met");
    routine::split(clock.at(now)).1
}

/// The cells of `item`'s footprint in `frame`, as shown in the last
/// frame.
fn cells_of(guest: &Guest, frame: &Buffer, item: Furniture) -> Vec<String> {
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    let piece = visit
        .shown
        .iter()
        .find(|s| s.item == item)
        .unwrap_or_else(|| panic!("{item:?} shown"));
    let r = piece.rect();
    (r.y..r.bottom())
        .flat_map(|y| (r.x..r.right()).map(move |x| (x, y)))
        .map(|at| frame[at].symbol().to_owned())
        .collect()
}

/// Her clock and window are drawn the same whichever way they were hung
/// (a delivery through the right wall faces left): a mirrored dial
/// would read 3:00 as 9:00. In ASCII the same glyphs; in line art the
/// same image, never encoded again for the other way round.
#[test]
fn her_clock_and_window_are_never_mirrored() {
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = timed_home(3, mon(16, 50), sprite::Facing::Left, graphics);
            let now = until_visiting(&mut guest, &real, &view, 0);
            let left = paint(&mut guest, &real, &view, now);
            let encoded = guest.graphics.as_ref().map(|g| g.counts().encoded);
            let before: Vec<Vec<String>> = [Furniture::Clock, Furniture::Window]
                .map(|item| cells_of(&guest, &left, item))
                .to_vec();
            assert_eq!(
                looks_of(&guest, Furniture::Clock),
                Some(PieceState::Dial(Dial::at(minute_at(&guest, now)))),
                "{at}"
            );
            for prop in &mut guest.ledger.home.props {
                prop.facing = sprite::Facing::Right;
            }
            let right = paint(&mut guest, &real, &view, now);
            let after: Vec<Vec<String>> = [Furniture::Clock, Furniture::Window]
                .map(|item| cells_of(&guest, &right, item))
                .to_vec();
            assert_eq!(before, after, "{at}: mirrored");
            assert_eq!(
                guest.graphics.as_ref().map(|g| g.counts().encoded),
                encoded,
                "{at}: drawn again the other way round"
            );
            if !graphics {
                // The face: rim, then the hand at :45 between its sides.
                assert_eq!(before[0].concat(), ".-.(<)", "{at}");
                // The window: its frame, then the day's sky (a strip of
                // blue, the sun) between its sides.
                assert_eq!(before[1].concat(), ".--.|-o|", "{at}");
            }
        }
    }
}

/// Over an hour of her clock (ten real minutes) from 16:50, her wall
/// clock's dial is her time of day to the quarter-hour in every frame,
/// and changes only on the quarter-hour, in a frame painted just then
/// (she wakes for it); the window's sky turns to dusk at 17:00. In both
/// drawing modes; in ASCII, the hand's glyph moves with the dial.
#[test]
fn the_dial_changes_only_on_the_quarter_hour() {
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = timed_home(5, mon(16, 50), sprite::Facing::Right, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let start = now;
            let mut frames: Vec<(u64, PieceState, PieceState, Option<String>)> = Vec::new();
            let mut skies: Vec<(Sky, String)> = Vec::new();
            while now < start + 10 * 60_000 {
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                if guest.advance(now) {
                    let frame = paint(&mut guest, &real, &view, now);
                    let (Some(dial), Some(sky)) = (
                        looks_of(&guest, Furniture::Clock),
                        looks_of(&guest, Furniture::Window),
                    ) else {
                        panic!("{at} {now}: closeted");
                    };
                    let hand =
                        (!graphics).then(|| cells_of(&guest, &frame, Furniture::Clock)[4].clone());
                    if !graphics && let PieceState::Sky(phase) = sky {
                        // Its lower row, between the frame's sides.
                        let window = cells_of(&guest, &frame, Furniture::Window);
                        skies.push((phase, window[5..7].concat()));
                    }
                    frames.push((now, dial, sky, hand));
                }
            }
            for &(t, dial, sky, _) in &frames {
                let minute = minute_at(&guest, t);
                assert_eq!(dial, PieceState::Dial(Dial::at(minute)), "{at} {t}");
                assert_eq!(sky, PieceState::Sky(Sky::at(minute)), "{at} {t}");
            }
            let mut changes = 0;
            for pair in frames.windows(2) {
                let ((_, d0, _, h0), (t1, d1, _, h1)) = (&pair[0], &pair[1]);
                if d0 != d1 {
                    changes += 1;
                    // Painted the moment it turned (the first real millisecond
                    // her clock reads the quarter).
                    let clock = guest.game_clock(*t1).expect("met");
                    let quarter = routine::next_quarter(clock.at(*t1 - 1));
                    assert_eq!(clock.when(quarter), *t1, "{at}: late to turn");
                    if !graphics {
                        assert_ne!(h0, h1, "{at} {t1}: the hand stood still");
                    }
                } else {
                    assert_eq!(h0, h1, "{at} {t1}: the hand moved");
                }
            }
            // 17:00, 17:15, 17:30, 17:45 (an hour from 16:50 or so).
            assert!(changes >= 4, "{at}: the dial changed {changes} times");
            assert!(
                frames
                    .iter()
                    .any(|&(_, _, sky, _)| sky == PieceState::Sky(Sky::Dusk)),
                "{at}: no dusk"
            );
            // In ASCII, the sky drawn turns with it: the sun, then the
            // dusk's glow.
            if !graphics {
                skies.dedup();
                assert_eq!(
                    skies,
                    [(Sky::Day, "-o".to_owned()), (Sky::Dusk, "~~".to_owned())],
                    "{at}"
                );
            }
        }
    }
}

/// A home without a clock or a window shown wakes for nothing more: no
/// quarter-hour is asked for (unshown, boxed, unfed, or not there at
/// all); with one shown, the next quarter-hour is. In both drawing
/// modes, quiet panes or text-dense.
#[test]
fn no_wakeups_for_the_time_without_a_clock_or_window() {
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(5, mon(16, 50), &HOME, graphics);
            let now = until_visiting(&mut guest, &real, &view, 0);
            paint(&mut guest, &real, &view, now);
            let shown = visit_of(&guest).shown.clone();
            assert_eq!(guest.next_quarter(&shown, now), None, "{at}: no clock");
            let window = |boxed: bool| Shown {
                item: Furniture::Window,
                facing: sprite::Facing::Right,
                boxed,
                strip: None,
                left: 10,
                floor: 16,
                scrap: None,
            };
            let clock = guest.game_clock(now).expect("met");
            let due = clock.when(routine::next_quarter(clock.at(now)));
            assert_eq!(
                guest.next_quarter(&[window(true)], now),
                None,
                "{at}: boxed"
            );
            assert_eq!(guest.next_quarter(&[window(false)], now), Some(due), "{at}");
            assert!(
                due > now && due <= now + 15 * 60_000 / CLOCK_SPEED + 1,
                "{at}"
            );
            guest.set_feed_clock(false);
            assert_eq!(
                guest.next_quarter(&[window(false)], now),
                None,
                "{at}: unfed"
            );
        }
    }
}

/// Her empty home tells the time too while she's at school (the lamp
/// off): the dial and the sky as her clock reads, turning on the
/// quarter-hour, when she wakes for it (and the screen changes).
#[test]
fn her_empty_home_tells_the_time() {
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = timed_home(4, tue(9, 5), sprite::Facing::Left, graphics);
            let mut now = 0;
            paint(&mut guest, &real, &view, now);
            while !matches!(guest.state, State::Away(_)) {
                assert!(now < 60_000, "{at}: her home never stood empty");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            paint(&mut guest, &real, &view, now);
            let minute = minute_at(&guest, now);
            assert_eq!(
                looks_of(&guest, Furniture::Clock),
                Some(PieceState::Dial(Dial::at(minute))),
                "{at}"
            );
            assert_eq!(
                looks_of(&guest, Furniture::Window),
                Some(PieceState::Sky(Sky::Day)),
                "{at}"
            );
            let due = real_of(&guest, now, tue(9, 15));
            let wake = now
                + guest
                    .next_tick(now)
                    .map_or(u64::MAX, |d| d.as_millis() as u64);
            assert!(wake <= due, "{at}: no wakeup for 09:15 ({wake} > {due})");
            while now < due {
                shell_step(&mut guest, &real, &view, &mut now, false);
            }
            assert_eq!(now, due, "{at}: woke past it");
            assert_eq!(
                looks_of(&guest, Furniture::Clock),
                Some(PieceState::Dial(Dial::at(9 * 60 + 15))),
                "{at}: not repainted at 09:15"
            );
        }
    }
}

/// A goodbye holds the dial and the sky it froze: a quarter-hour passing
/// as she goes doesn't move the hands. In line art, every clock painted
/// through the goodbye (up to her rain) is the dial she froze, painted
/// past the quarter-hour too; in ASCII, its glyphs stay as they were.
/// Quiet panes or text-dense.
#[test]
fn a_goodbye_holds_the_dial_it_froze() {
    use crate::ui::houseguest::graphics::Look;
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = timed_home(6, mon(16, 59), sprite::Facing::Right, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            // A few game seconds (half a real one) before a quarter-hour.
            let quarter = loop {
                let clock = guest.game_clock(now).expect("met");
                let next = routine::next_quarter(clock.at(now));
                if next - clock.at(now) <= 3_000 {
                    break clock.when(next);
                }
                if next - clock.at(now) <= 30_000 {
                    now += 100;
                    if guest.advance(now) {
                        paint(&mut guest, &real, &view, now);
                    }
                } else {
                    shell_step(&mut guest, &real, &view, &mut now, true);
                }
            };
            let frame = paint(&mut guest, &real, &view, now);
            let dial = looks_of(&guest, Furniture::Clock).unwrap_or_else(|| panic!("{at}: shown"));
            let glyphs = cells_of(&guest, &frame, Furniture::Clock);
            let rect = visit_of(&guest)
                .shown
                .iter()
                .find(|s| s.item == Furniture::Clock)
                .map(Shown::rect)
                .unwrap_or_else(|| panic!("{at}: shown"));
            if let Some(graphics) = &mut guest.graphics {
                graphics.take_looks();
            }
            guest.activity(now);
            let start = now;
            let (mut painted, mut past) = (0, false);
            // Every 25 ms of her goodbye, until she rains out.
            while now < start + dissolve::RAIN_FROM_MS {
                let frame = paint(&mut guest, &real, &view, now);
                assert!(
                    matches!(guest.state, State::Leaving(_)),
                    "{at} {now}: leaving"
                );
                match &mut guest.graphics {
                    Some(graphics) => {
                        for look in graphics.take_looks() {
                            if let Look::Piece(Furniture::Clock, state) = look {
                                assert_eq!(state, dial, "{at} {now}: the hands moved");
                                painted += 1;
                                past |= now >= quarter;
                            }
                        }
                    }
                    None => {
                        let held: Vec<String> = (rect.y..rect.bottom())
                            .flat_map(|y| (rect.x..rect.right()).map(move |x| (x, y)))
                            .map(|at| frame[at].symbol().to_owned())
                            .collect();
                        assert_eq!(held, glyphs, "{at} {now}: the glyphs moved");
                        past |= now >= quarter;
                    }
                }
                now += 25;
            }
            assert!(past, "{at}: gone before the quarter-hour turned");
            assert!(!graphics || painted > 0, "{at}: never painted");
        }
    }
}

/// Her wall clock never comes on a dash home from school (it waits for
/// her to be home), nor while she's tucked in for the night. Quiet panes
/// or text-dense, in both drawing modes.
#[test]
fn her_wall_clock_waits_out_a_dash_and_her_night() {
    let seed = (0..10_000)
        .find(|&seed| brain::dash(seed, 1).is_some())
        .expect("a Tuesday with a dash");
    let minute = brain::dash(seed, 1).unwrap_or_default();
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let dash_at = routine::GameTime {
                day: 1,
                h: (minute - 2) / 60,
                m: (minute - 2) % 60,
            };
            let mut guest = home_at(seed, dash_at, &HOME, graphics);
            guest.ledger.clock_sent = false;
            let mut now = 0;
            let mut dashed = false;
            paint(&mut guest, &real, &view, now);
            // Until she has dashed home and her home stands empty again.
            while !(dashed && matches!(guest.state, State::Away(_))) {
                assert!(now < 4 * 60_000, "{at}: no dash, or never out again");
                shell_step(&mut guest, &real, &view, &mut now, true);
                if let State::Visiting(visit) = &guest.state {
                    assert_eq!(visit.kind, Kind::Dash, "{at}: home at {now}");
                    dashed = true;
                }
                assert!(!guest.ledger.clock_sent, "{at}: on a dash, at {now}");
            }
            // Tucked in at 23:00, five minutes of her night (never greeted:
            // see the next test for a night she's greeted before).
            let mut guest = home_at(seed, mon(23, 0), &HOME, graphics);
            guest.ledger.clock_sent = false;
            let now = until_visiting(&mut guest, &real, &view, 0);
            assert!(visit_of(&guest).osaka.sleeping(), "{at}: tucked in");
            run(&mut guest, &real, &view, now, now + 5 * 60_000);
            assert!(
                !guest.ledger.clock_sent && !guest.ledger.home.owns(Furniture::Clock),
                "{at}: delivered in the night"
            );
        }
    }
}

/// Greeted and free for it, then through a door in space (out of sight
/// between its two doors), or gone to bed for the night: her wall clock,
/// owed, never comes while she's going through it, hidden, or asleep;
/// out of the door again, it comes.
/// Quiet panes or text-dense, in both drawing modes.
#[test]
fn her_wall_clock_waits_while_shes_out_of_sight_or_asleep() {
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let free = |guest: &Guest, now: u64| {
                let visit = visit_of(guest);
                visit
                    .osaka
                    .free_for_a_gift(now, &visit.chances, &visit.terrain)
            };
            // Through a door the moment she's free for it.
            let mut guest = home_at(5, mon(16, 0), &HOME, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            while !free(&guest, now) {
                assert!(now < 120_000, "{at}: never free");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            guest.ledger.clock_sent = false;
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            let spot = (visit.osaka.x, visit.osaka.y);
            visit.osaka.through_door(spot, now);
            let (mut hidden, mut out) = (0, None);
            while !guest.ledger.clock_sent {
                assert!(now < 240_000, "{at}: it never came");
                now += 20;
                let osaka = &visit_of(&guest).osaka;
                if osaka.hidden(now) {
                    hidden += 1;
                } else if hidden > 0 {
                    out.get_or_insert(now);
                }
                if guest.advance(now) {
                    paint(&mut guest, &real, &view, now);
                }
                let osaka = &visit_of(&guest).osaka;
                if osaka.hidden(now) || osaka.door(now).is_some() {
                    assert!(
                        !guest.ledger.clock_sent,
                        "{at}: came at {now}, she at her door or hidden"
                    );
                }
            }
            assert!(hidden > 0 && out.is_some(), "{at}: never out of sight");
            // Greeted, then to bed at 22:30: not while she sleeps.
            let mut guest = home_at(5, mon(22, 20), &HOME, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            while !asleep(&guest) {
                assert!(now < 240_000, "{at}: never to bed");
                shell_step(&mut guest, &real, &view, &mut now, false);
            }
            assert!(visit_of(&guest).osaka.greeted(), "{at}: greeted");
            guest.ledger.clock_sent = false;
            let end = now + 3 * 60_000;
            while now < end {
                shell_step(&mut guest, &real, &view, &mut now, false);
                assert!(!guest.ledger.clock_sent, "{at}: came at {now}, she asleep");
            }
        }
    }
}

/// Both shown through a busy fed visit from 17:30 to 19:30 (her dusk and
/// evening, the dial turning every 2.5 real minutes) cost scarcely any
/// images: the frame cache drops nothing, encodes nothing twice, and the
/// visit stays within [`VISIT_IMAGES`]. Over seeds, quiet panes or
/// text-dense; never hiding text.
#[test]
fn her_clock_and_window_stay_cheap() {
    use super::brain::Mood;
    for ((name, (real, view)), seed) in screens()
        .into_iter()
        .flat_map(|screen| (0..3u64).map(move |seed| (screen.clone(), seed)))
    {
        let seed_at = format!("{name} seed {seed}");
        let mut guest = timed_home(seed, mon(17, 30), sprite::Facing::Left, true);
        assert!(guest.ledger.home.add(room::Prop::new(
            Furniture::Desk,
            Nook::Playlist,
            800,
            sprite::Facing::Right
        )));
        let start = until_visiting(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("visiting");
        };
        visit.osaka.set_mood(Mood::Industrious);
        let mut hidden = Hidden::default();
        let (mut dials, mut skies) = (Vec::new(), Vec::new());
        let mut now = start;
        while now < start + 20 * 60_000 {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if guest.advance(now) {
                let frame = paint(&mut guest, &real, &view, now);
                let State::Visiting(visit) = &guest.state else {
                    panic!("{seed_at}: still visiting");
                };
                let layer: Vec<(u16, u16)> = visit.layer.cells().collect();
                hidden
                    .check(&frame, &real, &layer, &open_flap(&guest, now), now)
                    .unwrap_or_else(|e| panic!("{seed_at} at {now}: {e}"));
                for (item, seen) in [
                    (Furniture::Clock, &mut dials),
                    (Furniture::Window, &mut skies),
                ] {
                    if let Some(state) = looks_of(&guest, item)
                        && !seen.contains(&state)
                    {
                        seen.push(state);
                    }
                }
            }
        }
        assert!(dials.len() >= 8, "{seed_at}: dials {dials:?}");
        assert_eq!(
            skies,
            [PieceState::Sky(Sky::Dusk), PieceState::Sky(Sky::Evening)],
            "{seed_at}"
        );
        let counts = guest.graphics.as_ref().unwrap().counts();
        eprintln!("{seed_at}: {counts:?}");
        assert_eq!(
            (counts.evicted, counts.reencoded),
            (0, 0),
            "{seed_at}: {counts:?}"
        );
        assert!(
            counts.encoded <= VISIT_IMAGES,
            "{seed_at}: her images cost more: {counts:?}"
        );
    }
}
