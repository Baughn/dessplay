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
/// evening, the dial turning every 2.5 real minutes), with her looking
/// out of the window and glancing up at the clock (the stage cues each
/// once, at 17:35 and 17:45; she looks out of her own accord too), cost
/// scarcely any images: the frame cache drops nothing, encodes nothing
/// twice, and the visit stays within [`VISIT_IMAGES`]. Over seeds, quiet
/// panes or text-dense; never hiding text.
#[test]
fn her_clock_and_window_stay_cheap() {
    use super::brain::Mood;
    use super::script::ScriptId;
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
        let mut cues = vec![
            (real_of(&guest, start, mon(17, 45)), Scene::ClockGlance),
            (real_of(&guest, start, mon(17, 35)), Scene::LookOut),
        ];
        let (mut looked, mut glanced) = (0, 0);
        let mut playing = None;
        let mut now = start;
        while now < start + 20 * 60_000 {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if cues.last().is_some_and(|&(t, _)| t <= now)
                && let Some((_, scene)) = cues.pop()
            {
                guest.cue(scene);
            }
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
                let plays = visit.osaka.plays_since();
                if plays != playing {
                    match plays.map(|(_, p)| p.own) {
                        Some(ScriptId::LookOut) => looked += 1,
                        Some(ScriptId::ClockGlance) => glanced += 1,
                        _ => {}
                    }
                    playing = plays;
                }
            }
        }
        assert!(cues.is_empty(), "{seed_at}: cues left");
        assert!(
            looked >= 1 && glanced >= 1,
            "{seed_at}: looked {looked}, glanced {glanced}"
        );
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

/// The LookOut seats the frame offers her (her window's, if any).
fn look_out_seats(guest: &Guest) -> Vec<room::Seat> {
    visit_of(guest)
        .chances
        .seats
        .iter()
        .filter(|s| s.what == room::Use::LookOut)
        .copied()
        .collect()
}

/// She looks out of her window only with it shown and somewhere to stand
/// for it (step 8b): under it, a cell off its middle toward the side she
/// faces from (her gaze goes up the way she faces), on its floor; with a
/// piece standing beneath it, never in front of that: beside it, clear,
/// facing it (her lamp beneath), or, with nowhere clear to stand (her
/// sofa beneath), not at all: nothing binds. Without a window, or with it
/// still in its box, there's nothing to look out of (only the box to
/// unpack), and nothing binds. Her wall clock is offered as where it
/// hangs only out of its box. Quiet panes or text-dense, in both drawing
/// modes.
#[test]
fn she_looks_out_only_of_a_window_she_can_reach() {
    use crate::ui::houseguest::mind::{Place, Whims, places};
    let offered = |guest: &Guest| {
        (0..32u64)
            .filter(|&w| !places(room::Use::LookOut, &visit_of(guest).chances, Whims(w)).is_empty())
            .count()
    };
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            // Hung, with nothing beneath it: under it.
            let mut guest = timed_home(3, tue(14, 0), sprite::Facing::Right, graphics);
            let now = until_visiting(&mut guest, &real, &view, 0);
            paint(&mut guest, &real, &view, now);
            let window = *visit_of(&guest)
                .shown
                .iter()
                .find(|s| s.item == Furniture::Window)
                .unwrap_or_else(|| panic!("{at}: window shown"));
            let rect = window.rect();
            let middle = i32::from(rect.x) + i32::from(rect.width) / 2;
            let seats = look_out_seats(&guest);
            assert_eq!(seats.len(), 1, "{at}: {seats:?}");
            let seat = seats[0];
            assert_eq!(seat.item, Furniture::Window, "{at}");
            assert_eq!(
                (seat.x, seat.y),
                (middle - 1, window.floor),
                "{at}: under it"
            );
            assert_eq!(seat.facing, sprite::Facing::Right, "{at}");
            assert_eq!(offered(&guest), 32, "{at}");
            assert!(
                places(room::Use::LookOut, &visit_of(&guest).chances, Whims(0))
                    .iter()
                    .all(|p| matches!(p, Place::Seat(s) if *s == seat)),
                "{at}"
            );
            // Her clock, where it hangs.
            let clock = visit_of(&guest)
                .shown
                .iter()
                .find(|s| s.item == Furniture::Clock)
                .map(Shown::rect)
                .unwrap_or_else(|| panic!("{at}: clock shown"));
            let on = visit_of(&guest).chances.clock.expect("her clock offered");
            assert_eq!(
                on.x,
                i32::from(clock.x) + i32::from(clock.width) / 2,
                "{at}"
            );
            assert_eq!(on.floor, window.floor, "{at}: the same strip");
            assert_eq!(on.seen_from((seat.x, seat.y)), Some(on.x), "{at}");
            assert_eq!(on.seen_from((seat.x, seat.y - 3)), None, "{at}");
            // Hung the other way: under it, the other side of its middle.
            let mut guest = timed_home(3, tue(14, 0), sprite::Facing::Left, graphics);
            let now = until_visiting(&mut guest, &real, &view, 0);
            paint(&mut guest, &real, &view, now);
            let seats = look_out_seats(&guest);
            assert_eq!(
                seats.iter().map(|s| (s.x, s.facing)).collect::<Vec<_>>(),
                [(middle, sprite::Facing::Left)],
                "{at}: hung facing left"
            );
            // Over a piece that stands: never in front of it. Over her
            // lamp, beside it (clear of the lamp), facing it; over her
            // sofa, wider, nowhere clear to stand (her box beside the
            // window would be in front of the sofa too), so she can't
            // reach it to look out, and nothing binds.
            // Beside it, she stands first on the side she'd face it from
            // the way it was hung (the other is clear too).
            for (piece, reachable, facing) in [
                (Furniture::Lamp, true, sprite::Facing::Right),
                (Furniture::Lamp, true, sprite::Facing::Left),
                (Furniture::Sofa, false, sprite::Facing::Right),
            ] {
                let at = format!("{at} over her {piece:?} hung {facing:?}");
                let mut guest = home_at(
                    3,
                    tue(14, 0),
                    &[(piece, Nook::Users, 550), (Furniture::Tv, Nook::Users, 900)],
                    graphics,
                );
                assert!(guest.ledger.home.add(room::Prop::new(
                    Furniture::Window,
                    Nook::Users,
                    550,
                    facing
                )));
                let now = until_visiting(&mut guest, &real, &view, 0);
                paint(&mut guest, &real, &view, now);
                let shown = &visit_of(&guest).shown;
                let rect_of = |item: Furniture| {
                    shown
                        .iter()
                        .find(|s| s.item == item)
                        .map(Shown::rect)
                        .unwrap_or_else(|| panic!("{at}: {item:?} shown"))
                };
                let (window, below) = (rect_of(Furniture::Window), rect_of(piece));
                let under = |x: i32| (i32::from(below.x)..i32::from(below.right())).contains(&x);
                let middle = i32::from(window.x) + i32::from(window.width) / 2;
                assert!(
                    under(middle - 1) || under(middle),
                    "{at}: {window:?} over {below:?}"
                );
                let seats = look_out_seats(&guest);
                if !reachable {
                    assert!(seats.is_empty(), "{at}: {seats:?}");
                    assert_eq!(offered(&guest), 0, "{at}");
                    continue;
                }
                assert_eq!(seats.len(), 1, "{at}: {seats:?} {window:?} {below:?}");
                assert_eq!(offered(&guest), 32, "{at}");
                let seat = seats[0];
                let half = sprite::WIDTH / 2;
                assert!(
                    seat.x + half < i32::from(below.x) || seat.x - half >= i32::from(below.right()),
                    "{at}: in front of it: {seat:?}, {below:?}"
                );
                assert_eq!(
                    seat.facing,
                    if seat.x < middle {
                        sprite::Facing::Right
                    } else {
                        sprite::Facing::Left
                    },
                    "{at}: facing it"
                );
                assert_eq!(seat.facing, facing, "{at}: from the side it was hung to");
                // The other side would do as well: she'd fit there too.
                let terrain = &visit_of(&guest).terrain;
                let window = *shown
                    .iter()
                    .find(|s| s.item == Furniture::Window)
                    .unwrap_or_else(|| panic!("{at}: window"));
                let [left, right] = window.beside();
                let other = if seat.x == left { right } else { left };
                assert_ne!(seat.x, other, "{at}");
                assert!(
                    seat_spot(terrain, other, window.floor) && clear_of(shown, other, window.floor),
                    "{at}: the other side at {other} is blocked"
                );
            }
            // No window, or one still boxed: nothing to look out of; no
            // clock offered.
            for boxed in [None, Some(())] {
                let mut guest = home_at(3, tue(14, 0), &HOME, graphics);
                if boxed.is_some() {
                    let mut window =
                        room::Prop::new(Furniture::Window, Nook::Users, 550, sprite::Facing::Right);
                    window.boxed = true;
                    assert!(guest.ledger.home.add(window));
                    let mut clock =
                        room::Prop::new(Furniture::Clock, Nook::Users, 150, sprite::Facing::Right);
                    clock.boxed = true;
                    assert!(guest.ledger.home.add(clock));
                }
                let now = until_visiting(&mut guest, &real, &view, 0);
                paint(&mut guest, &real, &view, now);
                assert!(look_out_seats(&guest).is_empty(), "{at} boxed={boxed:?}");
                assert_eq!(offered(&guest), 0, "{at} boxed={boxed:?}");
                assert_eq!(visit_of(&guest).chances.clock, None, "{at} boxed={boxed:?}");
            }
        }
    }
}

/// Looking out of her window, she says what her sky shows, gazing up
/// curious: the sun by day, the sunset at dusk, the lights coming on of
/// an evening, the stars at night. Cued at each of those times, in both
/// drawing modes, quiet panes or text-dense; drawn, the bubble is her
/// line (nothing else she says is over it at first).
#[test]
fn she_looks_out_at_the_sky_she_sees() {
    use super::script::{LOOK_OUT_LINES, ScriptId};
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            for (when, sky) in [
                (tue(14, 0), Sky::Day),
                (mon(17, 30), Sky::Dusk),
                (mon(19, 30), Sky::Evening),
                (mon(21, 30), Sky::Night),
            ] {
                let at = format!("{name} graphics={graphics} {sky:?}");
                let mut guest = timed_home(2, when, sprite::Facing::Right, graphics);
                let mut now = until_visiting(&mut guest, &real, &view, 0);
                guest.cue(Scene::LookOut);
                let start = now;
                let play = loop {
                    assert!(now < start + 30_000, "{at}: never looked out");
                    shell_step(&mut guest, &real, &view, &mut now, true);
                    if let Some(play) = visit_of(&guest)
                        .osaka
                        .plays()
                        .filter(|p| p.own == ScriptId::LookOut)
                    {
                        break play;
                    }
                };
                let (of, line) = LOOK_OUT_LINES[usize::from(play.branch)];
                assert_eq!(of, sky, "{at}: {line}");
                assert_eq!(
                    Sky::at(minute_at(&guest, now)),
                    sky,
                    "{at}: the sky turned meanwhile"
                );
                let (pose, face, bubble) = visit_of(&guest).osaka.appearance(now);
                assert_eq!(
                    (pose, face),
                    (sprite::Pose::Gaze, sprite::Face::Curious),
                    "{at}"
                );
                assert_eq!(bubble, Some(osaka::Bubble::Say(line)), "{at}");
            }
        }
    }
}

/// A window delivered to a furnished home (her sofa, TV, bed, desk, lamp
/// and fridge wherever, across her two rooms) comes in through a wall
/// where she can stand to look out of it, wherever one lets her: unboxed
/// where its parcel stood, it offers her a seat in nearly every home.
/// Without that preference about a third of them had a window she could
/// never look out of. In both drawing modes.
#[test]
fn a_delivered_window_is_one_she_can_look_out_of() {
    const PIECES: [Furniture; 6] = [
        Furniture::Sofa,
        Furniture::Tv,
        Furniture::Bed,
        Furniture::Desk,
        Furniture::Lamp,
        Furniture::Fridge,
    ];
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let (mut delivered, mut usable) = (0, 0);
        for seed in 0..40u64 {
            let at = format!("seed {seed} graphics={graphics}");
            let mut rng = Rng(seed ^ 0x77_1d0e);
            let pieces: Vec<(Furniture, Nook, u16)> = PIECES
                .iter()
                .map(|&item| {
                    let nook = if rng.below(2) == 0 {
                        Nook::Users
                    } else {
                        Nook::Playlist
                    };
                    (item, nook, rng.below(1001) as u16)
                })
                .collect();
            let mut guest = home_at(seed, tue(14, 0), &pieces, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            guest.ledger.ordered = Some(Furniture::Window);
            guest.ledger.bought_on = 0;
            let start = now;
            while !guest.ledger.home.owns(Furniture::Window) && now < start + 10_000 {
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let Some(window) = guest
                .ledger
                .home
                .props
                .iter_mut()
                .find(|p| p.item == Furniture::Window)
            else {
                continue;
            };
            delivered += 1;
            window.boxed = false;
            paint(&mut guest, &real, &view, now);
            if look_out_seats(&guest).is_empty() {
                tracing::debug!("{at}: no window seat");
            } else {
                usable += 1;
            }
        }
        // 26 of 40 without the preference (2026-10-05); 40 of 40 with.
        assert!(
            delivered >= 30,
            "graphics={graphics}: {delivered} delivered"
        );
        assert!(
            usable * 20 >= delivered * 19,
            "graphics={graphics}: {usable} of {delivered} she can look out of"
        );
    }
}

/// As her routine turns, at bedtime (22:30) and as she leaves for school
/// (08:15), with her wall clock hung over the strip she stands on, she
/// first glances up at it, through the frame's own chain (where it
/// hangs, offered to her each frame): turned toward it, gazing up,
/// "Oh! It's late!" or "Time for school!", and then her routine goes on.
/// With it hung in her other room (or her standing between rooms), or
/// with none, her routine goes on at once. (Just before the turn, she's
/// put in her living room, and it's hung there or in her
/// bedroom.) In both drawing modes, quiet panes or text-dense.
#[test]
fn she_glances_at_her_clock_as_her_routine_turns() {
    use super::script::{ClockGlance, ITS_LATE, SCHOOL_TIME, ScriptId};
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            for (start, turn, glance, line) in [
                (mon(22, 25), mon(22, 30), ClockGlance::Bed, ITS_LATE),
                (tue(8, 10), tue(8, 15), ClockGlance::School, SCHOOL_TIME),
            ] {
                let mut glanced = 0;
                for (mine, hung) in [(true, true), (false, true), (false, false)] {
                    for seed in 0..3u64 {
                        let at = format!(
                            "{name} graphics={graphics} {glance:?} mine={mine} hung={hung} {seed}"
                        );
                        let mut guest = home_at(seed, start, &HOME, graphics);
                        let mut now = until_visiting(&mut guest, &real, &view, 0);
                        let due = real_of(&guest, now, turn);
                        assert!(now + 3000 < due, "{at}: arrived after the turn");
                        // (Each step at most a second: none crosses the
                        // turn.)
                        while now + 3000 < due {
                            shell_step(&mut guest, &real, &view, &mut now, true);
                        }
                        // In her living room as the turn comes (the stage
                        // puts her there), her clock hung there or in her
                        // bedroom.
                        let State::Visiting(visit) = &mut guest.state else {
                            panic!("{at}: visiting");
                        };
                        let (_, users) = view.nooks[0];
                        visit.osaka.place(25, i32::from(users.bottom()) - 1, now);
                        // Standing there until the turn (not setting off
                        // anywhere in its last seconds: what's measured
                        // is the glance from where she's put).
                        visit.osaka.offer_only = Some((brain::Want::Stand, "stand"));
                        if hung {
                            let nook = if mine { Nook::Users } else { Nook::Playlist };
                            assert!(guest.ledger.home.add(room::Prop::new(
                                Furniture::Clock,
                                nook,
                                500,
                                sprite::Facing::Right
                            )));
                        }
                        paint(&mut guest, &real, &view, now);
                        let decided = visit_of(&guest).osaka.decisions.len();
                        let first = |guest: &Guest| {
                            visit_of(guest).osaka.decisions[decided..]
                                .iter()
                                .find(|d| d.at >= due && d.method.starts_with("routine/"))
                                .map(|d| d.method)
                        };
                        while first(&guest).is_none() && now < due + 30_000 {
                            shell_step(&mut guest, &real, &view, &mut now, true);
                        }
                        // Off out of sight as the turn came (her routine
                        // waits for her), or asleep in her bed already as
                        // bedtime came (her night begun in place): no
                        // routine to glance first.
                        if first(&guest).is_none() {
                            let osaka = &visit_of(&guest).osaka;
                            assert!(
                                osaka.hidden(now) || glance == ClockGlance::Bed && osaka.sleeping(),
                                "{at}: no routine by {now}: {:?}",
                                osaka.act_name()
                            );
                            continue;
                        }
                        let visit = visit_of(&guest);
                        let osaka = &visit.osaka;
                        let clock = visit.chances.clock;
                        assert_eq!(clock.is_some(), hung, "{at}: {clock:?}");
                        let seen = clock.and_then(|c| c.seen_from((osaka.x, osaka.y)));
                        let method = first(&guest);
                        let Some(x) = seen else {
                            assert_ne!(method, Some("routine/glance"), "{at}");
                            continue;
                        };
                        assert_eq!(method, Some("routine/glance"), "{at}");
                        glanced += 1;
                        assert_eq!(
                            osaka.plays().map(|p| (p.own, p.branch)),
                            Some((ScriptId::ClockGlance, glance.branch())),
                            "{at}"
                        );
                        let (pose, _, bubble) = osaka.appearance(now);
                        assert_eq!(pose, sprite::Pose::Gaze, "{at}");
                        assert_eq!(bubble, Some(osaka::Bubble::Say(line)), "{at}");
                        if x != osaka.x {
                            let toward = if x > osaka.x {
                                sprite::Facing::Right
                            } else {
                                sprite::Facing::Left
                            };
                            assert_eq!(osaka.facing, toward, "{at}: toward it at {x}");
                        }
                    }
                }
                assert!(
                    glanced >= 1,
                    "{name} graphics={graphics} {glance:?}: {glanced} on her strip"
                );
            }
        }
    }
}

/// Cued by the stage in the very frame her wall clock first shows
/// (before any tick of hers has seen it), her glance still finds it:
/// the stage hands her the frame as it is. Turned toward it, she says
/// the hour it shows ("Two-ish." at 14:00). In both drawing modes, quiet
/// panes or text-dense.
#[test]
fn a_cued_glance_finds_the_clock_as_it_hangs_now() {
    use super::script::ScriptId;
    for (name, (real, view)) in screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(2, tue(14, 0), &HOME, graphics);
            let now = until_visiting(&mut guest, &real, &view, 0);
            let (_, users) = view.nooks[0];
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            visit.osaka.place(40, i32::from(users.bottom()) - 1, now);
            visit.osaka.facing = sprite::Facing::Right;
            assert!(guest.ledger.home.add(room::Prop::new(
                Furniture::Clock,
                Nook::Users,
                150,
                sprite::Facing::Right
            )));
            guest.cue(Scene::ClockGlance);
            paint(&mut guest, &real, &view, now);
            let visit = visit_of(&guest);
            let osaka = &visit.osaka;
            let x = visit
                .chances
                .clock
                .and_then(|c| c.seen_from((osaka.x, osaka.y)))
                .unwrap_or_else(|| panic!("{at}: her clock in sight"));
            assert!(x < osaka.x, "{at}: {x} vs {}", osaka.x);
            assert_eq!(
                osaka.plays().map(|p| p.own),
                Some(ScriptId::ClockGlance),
                "{at}"
            );
            assert_eq!(osaka.facing, sprite::Facing::Left, "{at}: toward it");
            assert_eq!(
                osaka.appearance(now).2,
                Some(osaka::Bubble::Say("Two-ish.")),
                "{at}"
            );
        }
    }
}
