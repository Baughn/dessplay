//! Her rains (a focused pane's, what she moved as she went out, her
//! closed door broken in on) and the redraw (phase 5c D0a): a rain live
//! before a tick always gets that tick's frame, its last (the frame
//! without it) included, whatever state she's in; a rain outlives her
//! state changing under it (her coming home from school, a goodbye);
//! and only her going at once ends it early. Each in both drawing
//! modes, the panes quiet or with text in them.

use super::*;
use crate::ui::houseguest::room::Use;

const W: u16 = 100;
const H: u16 = 30;

/// Her home on [`rooms`]: the sofa in the Users pane, the bed and lamp
/// in the Playlist pane (the one these tests focus).
const PIECES: [(Furniture, Nook, u16); 3] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Bed, Nook::Playlist, 300),
    (Furniture::Lamp, Nook::Playlist, 800),
];

/// Whether any rain of hers is live (its frames, or the frame without
/// it, still to draw).
fn fading(guest: &Guest) -> bool {
    !guest.fades.is_empty()
}

/// The screen: [`rooms`], or [`wordy_rooms`] (text in the chat, a line
/// at the top of each quiet pane).
fn screen(texty: bool) -> Buffer {
    if texty {
        wordy_rooms(W, H)
    } else {
        rooms(W, H)
    }
}

/// The pane these tests focus: the Playlist pane, her bed and lamp in it.
fn pane() -> Rect {
    nooks(W, H)[2].1
}

/// How many cells of the focused pane `frame` changed.
fn in_pane(frame: &Buffer, real: &Buffer) -> usize {
    let pane = pane();
    (pane.top()..pane.bottom())
        .flat_map(|y| (pane.left()..pane.right()).map(move |x| (x, y)))
        .filter(|&at| frame.cell(at) != real.cell(at))
        .count()
}

/// Whether she's lounging on her sofa.
fn lounging(guest: &Guest) -> bool {
    matches!(&guest.state, State::Visiting(visit)
        if visit.osaka.use_span().is_some_and(|(seat, ..)| seat.what == Use::Lounge))
}

/// A resident's home (`seed`, at `at`), her routine fed to her or not.
fn resident_home(seed: u64, at: routine::GameTime, graphics: bool, fed: bool) -> Guest {
    let guest = home_at(seed, at, &PIECES, graphics);
    if fed { guest } else { guest.unfed() }
}

/// From `from`, as the shell steps (painting every step), until her
/// home stands empty, and a step more so a frame has painted it.
fn until_away(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64) -> u64 {
    let mut now = from;
    paint(guest, real, view, now);
    while !matches!(guest.state, State::Away(_)) {
        assert!(now < from + 60_000, "her home never stood empty");
        shell_step(guest, real, view, &mut now, true);
    }
    shell_step(guest, real, view, &mut now, true);
    now
}

/// From `from`, cued onto her sofa, as the shell steps until she's
/// lounging on it (a minute at most, then she's left as she is).
fn onto_the_sofa(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64) -> u64 {
    let mut now = from;
    guest.cue(Scene::Lounge);
    paint(guest, real, view, now);
    while !lounging(guest) && now < from + 60_000 {
        shell_step(guest, real, view, &mut now, true);
    }
    now
}

/// Focus `on` at `now` while she's at the keys: what of hers stood in
/// it rains out. Returns when the last of her rains ends, as they say.
fn rain_on(guest: &mut Guest, real: &Buffer, on: Rect, now: u64) -> u64 {
    guest.advance(now);
    paint(guest, real, &resident_view(W, H, Some(on)), now);
    assert!(fading(guest), "her pieces in the pane rain out");
    guest
        .fades
        .iter()
        .map(Dissolve::ends_at)
        .max()
        .expect("a rain")
}

/// Focus the Playlist pane at `now`: [`rain_on`] it.
fn focus_rain(guest: &mut Guest, real: &Buffer, now: u64) -> u64 {
    rain_on(guest, real, pane(), now)
}

/// The first seed of `0..32` that sets the scene: `scene` returns
/// `None` when its setup didn't take (she wasn't where the test needs
/// her), and asserts the rule under test itself (a broken rule is never
/// "try another seed"). A scene no seed sets fails loudly; tuning her
/// elsewhere moves the seed, not the test.
fn first_seed<T>(what: &str, mut scene: impl FnMut(u64) -> Option<T>) -> T {
    (0..32)
        .find_map(&mut scene)
        .unwrap_or_else(|| panic!("{what}: no seed of 0..32 sets the scene"))
}

/// Whether the goodbye under way paints anything in the focused pane
/// (it mustn't: nothing of hers but her rains goes there).
fn goodbye_in_pane(guest: &Guest) -> bool {
    let State::Leaving(leaving) = &guest.state else {
        return false;
    };
    let pane = pane();
    (pane.top()..pane.bottom())
        .any(|y| (pane.left()..pane.right()).any(|x| leaving.dissolve.painting(x, y)))
}

/// Whether some rain of hers painted a cell of the focused pane in the
/// last frame.
fn raining_in_pane(guest: &Guest) -> bool {
    let pane = pane();
    guest.fades.iter().any(|fade| {
        (pane.top()..pane.bottom())
            .any(|y| (pane.left()..pane.right()).any(|x| fade.painting(x, y)))
    })
}

/// Where the rain falls while the property watches.
#[derive(Clone, Copy, Debug)]
enum Where {
    /// Visiting (on her sofa, if she got there).
    Visiting,
    /// Out at school, her home standing empty.
    Away,
    /// Her goodbye from a visit (an overlay came up), mid-rain.
    LeavingVisit,
    /// Her empty home's goodbye (an overlay came up), mid-rain.
    LeavingAway,
}

/// One run of the property: a rain begins `wait` ms after she's settled
/// (`Where` says how), then a shell that may be late (each of `jumps`
/// not 0 is a jump of that many ms; 0 a step on her next tick) paints
/// only when a tick says the screen could change. Every tick taken
/// while a rain was live must say so.
fn live_rains_redraw(
    seed: u64,
    graphics: bool,
    texty: bool,
    at: Where,
    wait: u64,
    jumps: &[u64],
) -> Result<(), TestCaseError> {
    let real = screen(texty);
    let quiet = resident_view(W, H, None);
    let focused = resident_view(W, H, Some(pane()));
    let covered = IdleView {
        busy: Some(Busy::Overlay),
        ..focused.clone()
    };
    let (mut guest, mut now) = match at {
        Where::Visiting | Where::LeavingVisit => {
            let mut guest = resident_home(seed, tue(16, 0), graphics, false);
            let now = until_visiting(&mut guest, &real, &quiet, 0);
            let now = onto_the_sofa(&mut guest, &real, &quiet, now);
            (guest, now)
        }
        Where::Away | Where::LeavingAway => {
            let mut guest = resident_home(seed, tue(9, 0), graphics, true);
            let now = until_away(&mut guest, &real, &quiet, 0);
            (guest, now)
        }
    };
    let end = now + wait;
    while now < end {
        shell_step(&mut guest, &real, &quiet, &mut now, true);
    }
    now += 1;
    let rain_end = focus_rain(&mut guest, &real, now);
    let view = match at {
        Where::Visiting | Where::Away => focused,
        Where::LeavingVisit | Where::LeavingAway => {
            now += 1 + wait % 1500;
            let live = fading(&guest);
            let redraw = guest.advance(now);
            prop_assert!(
                !live || redraw,
                "{at:?}: a rain was live as the overlay came"
            );
            paint(&mut guest, &real, &covered, now);
            prop_assert!(
                matches!(guest.state, State::Leaving(_)),
                "her goodbye under the overlay"
            );
            covered
        }
    };
    let mut jumps = jumps.iter().copied();
    while now < rain_end + 2_000 {
        let step = match jumps.next() {
            Some(jump) if jump > 0 => jump,
            _ => guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000),
        };
        now += step;
        let live = fading(&guest);
        let redraw = guest.advance(now);
        prop_assert!(
            !live || redraw,
            "{at:?} graphics={graphics} texty={texty}: a rain was live, \
             yet the tick at {now} (the rain ends at {rain_end}) said nothing changed"
        );
        if redraw {
            paint(&mut guest, &real, &view, now);
        }
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(16)))]

    /// Whenever a rain of hers was live before a tick, the tick says the
    /// screen could change, its last frame (the frame without the rain)
    /// included: visiting (seated, mostly, so she's quiet), out at school
    /// with her home empty, and in either goodbye; with a shell on time
    /// or late. Both drawing modes, the panes quiet or with text.
    #[test]
    fn a_live_rain_always_gets_its_frame(
        seed in 0u64..1_000_000,
        texty in any::<bool>(),
        at in prop_oneof![
            Just(Where::Visiting),
            Just(Where::Away),
            Just(Where::LeavingVisit),
            Just(Where::LeavingAway),
        ],
        wait in 0u64..6_000,
        jumps in proptest::collection::vec(prop_oneof![Just(0u64), 1u64..1_500], 0..40),
    ) {
        for graphics in [false, true] {
            live_rains_redraw(seed, graphics, texty, at, wait, &jumps)?;
        }
    }
}

/// Seated on her sofa, the pane with her bed and lamp focused: they rain
/// out, and the tick landing exactly at the rain's end (she's quiet
/// then, lounging on) says the screen changes, so its last frame (the
/// pane as it really is) is drawn.
#[test]
fn the_tick_a_rain_ends_on_redraws() {
    for graphics in [false, true] {
        for texty in [false, true] {
            let at = format!("graphics={graphics} texty={texty}");
            let real = screen(texty);
            let quiet = resident_view(W, H, None);
            let focused = resident_view(W, H, Some(pane()));
            first_seed(&at, |seed| {
                let at = format!("{at} seed={seed}");
                let mut guest = resident_home(seed, tue(16, 0), graphics, false);
                let now = until_visiting(&mut guest, &real, &quiet, 0);
                let mut now = onto_the_sofa(&mut guest, &real, &quiet, now);
                if !lounging(&guest) {
                    return None;
                }
                now += 1;
                let end = focus_rain(&mut guest, &real, now);
                let mut last = None;
                while now < end {
                    now += guest
                        .next_tick(now)
                        .map_or(1000, |d| d.as_millis() as u64)
                        .clamp(1, 1000);
                    assert!(now <= end, "{at}: a tick lands on the rain's end");
                    // The scene: she's quiet then, so only the rain could
                    // change the screen.
                    if now == end && !(lounging(&guest) && visit_of(&guest).osaka.due() > end) {
                        return None;
                    }
                    if guest.advance(now) {
                        last = Some((now, paint(&mut guest, &real, &focused, now)));
                    } else {
                        assert_ne!(now, end, "{at}: the rain's last frame never drawn");
                    }
                }
                let (drawn, frame) = last.expect("frames drawn");
                assert_eq!(drawn, end, "{at}");
                assert_eq!(in_pane(&frame, &real), 0, "{at}: the pane as it really is");
                Some(())
            });
        }
    }
}

/// A rain in her empty home as school ends outlives her coming home: it
/// falls on into the visit's frames to its end, never cut off (the pane
/// snapping back to the real UI), and the pane is itself again after.
#[test]
fn a_rain_outlives_her_coming_home() {
    for graphics in [false, true] {
        for texty in [false, true] {
            let at = format!("graphics={graphics} texty={texty}");
            let real = screen(texty);
            let quiet = resident_view(W, H, None);
            let focused = resident_view(W, H, Some(pane()));
            first_seed(&at, |seed| {
                let at = format!("{at} seed={seed}");
                let mut guest = resident_home(seed, tue(12, 44), graphics, true);
                let mut now = until_away(&mut guest, &real, &quiet, 0);
                let home = real_of(&guest, now, tue(12, 45));
                while now + 1_500 < home {
                    shell_step(&mut guest, &real, &quiet, &mut now, true);
                }
                now += 1;
                let end = focus_rain(&mut guest, &real, now);
                // The scene: school ends well inside the rain.
                if home + 1_000 >= end {
                    return None;
                }
                let mut came = None;
                while now < end + 1_000 {
                    now += guest
                        .next_tick(now)
                        .map_or(1000, |d| d.as_millis() as u64)
                        .clamp(1, 1000);
                    guest.advance(now);
                    let frame = paint(&mut guest, &real, &focused, now);
                    if came.is_none() && matches!(guest.state, State::Visiting(_)) {
                        // The scene: she's in before the rain is done.
                        if now >= end {
                            return None;
                        }
                        came = Some(now);
                        assert!(fading(&guest), "{at}: the rain falls on as she comes in");
                        assert!(in_pane(&frame, &real) > 0, "{at}: still raining at {now}");
                    }
                    if now >= end {
                        assert_eq!(in_pane(&frame, &real), 0, "{at}: the pane itself again");
                    }
                }
                came.map(|_| ())
            });
        }
    }
}

/// A rain falls on through her goodbye (an overlay came up mid-rain), from
/// a visit or her empty home: every frame of the goodbye's rain still
/// shows it in the focused pane (painted once a frame: twice, and the
/// second pass would take the first's glyphs for the UI changing and
/// settle every cell), each tick it's live redraws, a tick lands on its
/// end, and the pane is itself again then. Nothing of the goodbye's own
/// goes in the focused pane.
#[test]
fn a_rain_falls_on_through_her_goodbye() {
    for graphics in [false, true] {
        for texty in [false, true] {
            for away in [false, true] {
                let at = format!("graphics={graphics} texty={texty} away={away}");
                let real = screen(texty);
                let quiet = resident_view(W, H, None);
                let covered = IdleView {
                    busy: Some(Busy::Overlay),
                    ..resident_view(W, H, Some(pane()))
                };
                first_seed(&at, |seed| {
                    let at = format!("{at} seed={seed}");
                    let (mut guest, now) = if away {
                        let mut guest = resident_home(seed, tue(9, 0), graphics, true);
                        let now = until_away(&mut guest, &real, &quiet, 0);
                        (guest, now)
                    } else {
                        let mut guest = resident_home(seed, tue(16, 0), graphics, false);
                        let now = until_visiting(&mut guest, &real, &quiet, 0);
                        (guest, now)
                    };
                    let mut now = now + 1;
                    let end = focus_rain(&mut guest, &real, now);
                    let rain_from = end - dissolve::DURATION_MS;
                    // A tick on, the overlay: her goodbye, mid-rain.
                    now += 1;
                    assert!(guest.advance(now), "{at}: a rain is live");
                    paint(&mut guest, &real, &covered, now);
                    let State::Leaving(leaving) = &guest.state else {
                        // The scene: nothing of hers left to wave goodbye.
                        return None;
                    };
                    let bye = leaving.dissolve.started();
                    let mut checked = 0;
                    while now < end {
                        let live = fading(&guest);
                        now += guest
                            .next_tick(now)
                            .map_or(1000, |d| d.as_millis() as u64)
                            .clamp(1, 1000);
                        assert!(now <= end, "{at}: a tick lands on the rain's end");
                        let redraw = guest.advance(now);
                        assert!(!live || redraw, "{at}: the rain live at {now}, no redraw");
                        let frame = paint(&mut guest, &real, &covered, now);
                        assert!(!goodbye_in_pane(&guest), "{at}: her goodbye in the pane");
                        // Past the goodbye's own image (it may stand over
                        // the pane's edge in line art), till late in the rain.
                        if now >= bye + dissolve::RAIN_FROM_MS && now < rain_from + 2_500 {
                            checked += 1;
                            assert!(raining_in_pane(&guest), "{at}: the rain cut off at {now}");
                            assert!(in_pane(&frame, &real) > 0, "{at}: no rain drawn at {now}");
                        }
                        if now == end {
                            assert!(!fading(&guest), "{at}: the rain over");
                            assert_eq!(in_pane(&frame, &real), 0, "{at}: the pane itself again");
                        }
                    }
                    assert!(
                        checked >= 2,
                        "{at}: only {checked} frames of rain in the goodbye"
                    );
                    Some(())
                });
            }
        }
    }
}

/// Her empty home standing wholly in a pane just focused (the whole
/// screen, here) rains out of it; an overlay then has nothing of hers to
/// wave goodbye to, so she goes at once, but her rain falls on to its end
/// (only her being sent away or her room going cuts it short): each tick
/// it's live redraws, its frames still show it, a tick lands on its end,
/// and the screen is itself again then.
#[test]
fn her_rain_falls_on_after_she_goes_with_nothing_shown() {
    for graphics in [false, true] {
        for texty in [false, true] {
            let at = format!("graphics={graphics} texty={texty}");
            let real = screen(texty);
            let quiet = resident_view(W, H, None);
            let all = Rect::new(0, 0, W, H);
            let covered = IdleView {
                busy: Some(Busy::Overlay),
                ..resident_view(W, H, Some(all))
            };
            let mut guest = resident_home(6, tue(9, 0), graphics, true);
            let mut now = until_away(&mut guest, &real, &quiet, 0) + 1;
            let end = rain_on(&mut guest, &real, all, now);
            let rain_from = end - dissolve::DURATION_MS;
            let State::Away(empty) = &guest.state else {
                panic!("{at}: her home still stands empty");
            };
            assert!(empty.painted.is_empty(), "{at}: nothing of hers shown");
            now += 1;
            assert!(guest.advance(now), "{at}: a rain is live");
            let frame = paint(&mut guest, &real, &covered, now);
            assert!(matches!(guest.state, State::Absent), "{at}: gone at once");
            assert!(fading(&guest), "{at}: her rain falls on");
            assert_ne!(frame, real, "{at}: her rain drawn");
            while now < end {
                let live = fading(&guest);
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                assert!(now <= end, "{at}: a tick lands on the rain's end");
                let redraw = guest.advance(now);
                assert!(!live || redraw, "{at}: the rain live at {now}, no redraw");
                let frame = paint(&mut guest, &real, &covered, now);
                if now < rain_from + 2_500 {
                    assert_ne!(frame, real, "{at}: no rain drawn at {now}");
                }
                if now == end {
                    assert!(!fading(&guest), "{at}: the rain over");
                    assert_eq!(frame, real, "{at}: the screen itself again");
                }
            }
        }
    }
}

/// Switched off or moved out mid-rain, she's gone at once, her rains
/// with her (no goodbye, nothing of hers on screen; switched off,
/// nothing of the rain to wake for), visiting or out with her home
/// empty.
#[test]
fn switched_off_her_rains_go_with_her() {
    for graphics in [false, true] {
        for away in [false, true] {
            for moved_out in [false, true] {
                let at = format!("graphics={graphics} away={away} moved_out={moved_out}");
                let real = screen(true);
                let quiet = resident_view(W, H, None);
                let (mut guest, now) = if away {
                    let mut guest = resident_home(6, tue(9, 0), graphics, true);
                    let now = until_away(&mut guest, &real, &quiet, 0);
                    (guest, now)
                } else {
                    let mut guest = resident_home(3, tue(16, 0), graphics, false);
                    let now = until_visiting(&mut guest, &real, &quiet, 0);
                    (guest, now)
                };
                let mut now = now + 1;
                let end = focus_rain(&mut guest, &real, now);
                now += 500;
                guest.advance(now);
                let view = if moved_out {
                    guest.move_out(99);
                    resident_view(W, H, Some(pane()))
                } else {
                    IdleView {
                        delay: None,
                        ..resident_view(W, H, Some(pane()))
                    }
                };
                assert!(!fading(&guest) || !moved_out, "{at}: her rains with her");
                let frame = paint(&mut guest, &real, &view, now);
                assert!(matches!(guest.state, State::Absent), "{at}: gone");
                assert!(!fading(&guest), "{at}: her rains with her");
                assert_eq!(frame, real, "{at}: nothing of hers");
                // Moved out, the idle gate stays open: she may come again
                // soon, a first meeting, so only switched off is quiet.
                if !moved_out {
                    let wake = guest.next_tick(now).map(|d| now + d.as_millis() as u64);
                    assert!(
                        wake.is_none_or(|wake| wake > end),
                        "{at}: nothing to wake for in the rain's time: {wake:?}"
                    );
                }
            }
        }
    }
}
