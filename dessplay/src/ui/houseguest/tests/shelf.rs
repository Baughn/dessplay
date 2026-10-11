//! Her shelf in the frame (phase 6a, step 2b): written once a paint,
//! before anything reads the frame; kept as her home's bottom layer, or
//! undone by the arm that ends the frame without her home; hidden whole
//! where the frame can't hold it; raining out with what she showed. It's
//! put up by hand here (nothing makes one before step 5c), and it's no
//! strip yet (step 3), so nothing stands on it. Both modes; visiting and
//! with her home standing empty.

use super::*;
use crate::ui::houseguest::sprite::Facing;
use crate::ui::houseguest::structures::{SHELF_STAGES, Shelf, Structure, Written};

/// Her shelf on [`rooms`] at 100×30, in Users (`50, 0, 50, 13`): `Left`
/// 12, rise 7, so its run is columns `63..73` of row 5, its margin two
/// cells past each end.
const RUN: std::ops::Range<u16> = 63..73;
const ROW: u16 = 5;
const GLYPHS: &str = "┬────────┬";

fn users_shelf() -> Structure {
    Structure::Shelf(Shelf {
        nook: Nook::Users,
        anchor: room::Anchor {
            side: room::Side::Left,
            offset: 12,
        },
        rise: 7,
        built: 2,
    })
}

/// Her pieces, on the floors, well under her shelf.
const PIECES: [(Furniture, Nook, u16); 2] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Bed, Nook::Playlist, 300),
];

/// [`home_at`] with [`PIECES`] and her shelf up.
fn shelved(seed: u64, at: routine::GameTime, graphics: bool) -> Guest {
    let mut guest = home_at(seed, at, &PIECES, graphics);
    guest.ledger.home.structures.push(users_shelf());
    guest
}

/// Her shelf's row as `frame` shows it.
fn run_of(frame: &Buffer) -> String {
    RUN.map(|x| frame[(x, ROW)].symbol().to_owned()).collect()
}

/// What her home painted last, visiting or standing empty.
fn painted(guest: &Guest) -> &[Frozen] {
    match &guest.state {
        State::Visiting(visit) => &visit.painted,
        State::Away(empty) => &empty.painted,
        _ => &[],
    }
}

/// The frame shows her shelf, kept first in what her home painted (the
/// bottom layer), each cell over the real one.
fn shows(guest: &Guest, frame: &Buffer, real: &Buffer, at: &str) {
    assert_eq!(run_of(frame), GLYPHS, "{at}: her shelf shown");
    let kept = painted(guest);
    assert!(kept.len() >= RUN.len(), "{at}: kept");
    for (cell, x) in kept.iter().zip(RUN) {
        assert_eq!((cell.x, cell.y), (x, ROW), "{at}: kept first");
        assert_eq!(cell.under, real[(x, ROW)], "{at}: over the real cell");
        assert!(!cell.burst && cell.face.is_none(), "{at}");
    }
}

/// Neither the frame nor what her home painted has any of her shelf.
fn hides(guest: &Guest, frame: &Buffer, real: &Buffer, at: &str) {
    let shelf = |glyph: &str| glyph == "┬" || glyph == "─";
    for x in RUN {
        let (got, want) = (frame[(x, ROW)].symbol(), real[(x, ROW)].symbol());
        assert!(!shelf(got) || got == want, "{at}: x {x} shows {got:?}");
    }
    assert!(
        !painted(guest)
            .iter()
            .any(|c| !c.burst && matches!(c.glyph, '┬' | '─') && c.under.symbol() == " "),
        "{at}: her shelf kept"
    );
}

/// A tick and a paint, `step` ms on.
fn next(guest: &mut Guest, real: &Buffer, view: &IdleView, now: &mut u64, step: u64) -> Buffer {
    *now += step;
    guest.advance(*now);
    paint(guest, real, view, *now)
}

/// A resident's view of [`rooms`] at `w` wide ([`resident_view`]: the
/// chat on List, Users and the Playlist hers): with `focus`, the client
/// in use (the video playing), so the focused pane is kept clear; else
/// left alone.
fn in_use(w: u16, focus: Option<Rect>) -> IdleView {
    IdleView {
        busy: focus.map(|_| Busy::Playing),
        ..resident_view(w, 30, focus)
    }
}

/// Her shelf shows only where the frame holds it, all of it or none, and
/// comes back as soon as it does again (no hysteresis): text over its
/// margin or its run, one cell of it (a single glyph, as a cursor's
/// block), toggles all ten; a focused pane in use rains it out and keeps
/// it out; too narrow a frame or too short a pane hides it, and the size
/// back restores it; its pane hidden hides it. Her shelf's record never
/// moves, nor, until its pane is hidden (her pieces move off a hidden
/// pane, as ever), her pieces or her door's wall. Both modes, visiting
/// and with her home standing empty.
#[test]
fn her_shelf_shows_only_where_the_frame_holds_it() {
    for graphics in [false, true] {
        for away in [false, true] {
            let at = format!("graphics={graphics} away={away}");
            let real = rooms(100, 30);
            let quiet = in_use(100, None);
            let start = if away { tue(9, 0) } else { sat(14, 0) };
            let mut guest = shelved(4, start, graphics);
            let mut now = if away {
                // Out at school: her home stands empty.
                let mut now = 0;
                paint(&mut guest, &real, &quiet, now);
                while !matches!(guest.state, State::Away(_)) {
                    assert!(now < 200_000, "{at}: her home never stood empty");
                    next(&mut guest, &real, &quiet, &mut now, 250);
                }
                now
            } else {
                until_visiting(&mut guest, &real, &quiet, 0)
            };
            let state = |guest: &Guest| match guest.state {
                State::Visiting(_) => !away,
                State::Away(_) => away,
                _ => false,
            };
            let frame = next(&mut guest, &real, &quiet, &mut now, 250);
            assert!(state(&guest), "{at}: her home");
            shows(&guest, &frame, &real, &format!("{at}: quiet"));
            // Her record as it stands (anchors pinned by the frames so far).
            let record = guest.ledger.home.clone();
            // Text over its margin, then over a cell of its run.
            for (x, text) in [(74, "x"), (61, "so"), (66, "▌")] {
                let mut texty = real.clone();
                texty.set_string(x, ROW, text, Style::new());
                let phase = format!("{at}: {text:?} at {x}");
                let frame = next(&mut guest, &texty, &quiet, &mut now, 250);
                hides(&guest, &frame, &texty, &phase);
                let frame = next(&mut guest, &real, &quiet, &mut now, 250);
                shows(&guest, &frame, &real, &format!("{phase}: gone again"));
            }
            // A styled blank is as good as text (the user's fifth round):
            // a selection bar in reverse video across its row (a list's
            // highlighted entry past its text), then one blank of its
            // margin in a colour unlike its pane's.
            use tuirealm::ratatui::style::{Color, Modifier};
            for (rect, style, what) in [
                (
                    Rect::new(51, ROW, 48, 1),
                    Style::new().add_modifier(Modifier::REVERSED),
                    "a selection bar",
                ),
                (
                    Rect::new(74, ROW, 1, 1),
                    Style::new().bg(Color::Blue),
                    "a tinted blank",
                ),
            ] {
                let mut styled = real.clone();
                styled.set_style(rect, style);
                let phase = format!("{at}: {what}");
                let frame = next(&mut guest, &styled, &quiet, &mut now, 250);
                hides(&guest, &frame, &styled, &phase);
                let frame = next(&mut guest, &real, &quiet, &mut now, 250);
                shows(&guest, &frame, &real, &format!("{phase}: gone again"));
            }
            // Users in a theme's background of its own (a layout's
            // `background-color`), border and inside: her shelf shows on
            // its blanks; one cell of its run in the terminal's background
            // is a blank unlike its pane's, and hides it.
            let mut themed = real.clone();
            themed.set_style(
                Rect::new(50, 0, 50, 13),
                Style::new().bg(Color::Indexed(236)),
            );
            let phase = format!("{at}: a themed pane");
            let frame = next(&mut guest, &themed, &quiet, &mut now, 250);
            shows(&guest, &frame, &themed, &phase);
            let mut unthemed = themed.clone();
            unthemed.set_style(Rect::new(68, ROW, 1, 1), Style::new().bg(Color::Reset));
            let frame = next(&mut guest, &unthemed, &quiet, &mut now, 250);
            hides(
                &guest,
                &frame,
                &unthemed,
                &format!("{phase}, a terminal's blank"),
            );
            let frame = next(&mut guest, &themed, &quiet, &mut now, 250);
            shows(&guest, &frame, &themed, &format!("{phase} again"));
            let frame = next(&mut guest, &real, &quiet, &mut now, 250);
            shows(&guest, &frame, &real, &format!("{phase}: gone again"));
            // Its pane focused, the client in use: it rains out at once,
            // and stays out while it's focused.
            let users = nooks(100, 30)[1].1;
            let focused = in_use(100, Some(users));
            let landed = now + 1;
            now = landed;
            guest.advance(now);
            paint(&mut guest, &real, &focused, now);
            for x in RUN {
                assert!(
                    guest.fades.iter().any(|fade| fade.painting(x, ROW)),
                    "{at}: x {x} rains out of the focused pane"
                );
            }
            while now < landed + 6000 {
                let frame = next(&mut guest, &real, &focused, &mut now, 100);
                assert!(state(&guest), "{at}: her home, focused");
                if now >= landed + dissolve::DURATION_MS {
                    hides(&guest, &frame, &real, &format!("{at}: focused at {now}"));
                    assert_eq!(run_of(&frame), run_of(&real), "{at}: focused at {now}");
                }
            }
            let frame = next(&mut guest, &real, &quiet, &mut now, 250);
            shows(&guest, &frame, &real, &format!("{at}: focus gone"));
            // Too narrow for it (at 60 wide Users is 30: its run would end
            // within a door's space of the wall), then wide again.
            let (narrow, narrow_view) = (rooms(60, 30), in_use(60, None));
            for _ in 0..4 {
                let frame = next(&mut guest, &narrow, &narrow_view, &mut now, 250);
                assert!(state(&guest), "{at}: her home at 60 wide");
                assert!(
                    !frame.content.iter().any(|c| c.symbol() == "┬"),
                    "{at}: at 60 wide"
                );
                assert!(painted(&guest).iter().all(|c| c.glyph != '┬'), "{at}");
            }
            let frame = next(&mut guest, &real, &quiet, &mut now, 250);
            assert!(state(&guest), "{at}: her home, wide again");
            shows(&guest, &frame, &real, &format!("{at}: wide again"));
            // Too short for it (at 24 tall Users is 10, 8 rows inside: its
            // rise of 7 leaves fewer than `LOW_ABOVE` over it), then tall
            // again.
            let short_view = IdleView {
                busy: None,
                ..resident_view(100, 24, None)
            };
            let short = rooms(100, 24);
            for _ in 0..4 {
                let frame = next(&mut guest, &short, &short_view, &mut now, 250);
                assert!(state(&guest), "{at}: her home at 24 tall");
                assert!(
                    !frame.content.iter().any(|c| c.symbol() == "┬"),
                    "{at}: at 24 tall"
                );
                assert!(painted(&guest).iter().all(|c| c.glyph != '┬'), "{at}");
            }
            let frame = next(&mut guest, &real, &quiet, &mut now, 250);
            assert!(state(&guest), "{at}: her home, tall again");
            shows(&guest, &frame, &real, &format!("{at}: tall again"));
            // Her pieces where they were (a delivery may have come since).
            let home = &guest.ledger.home;
            for was in &record.props {
                assert!(
                    home.props
                        .iter()
                        .any(|p| (p.item, p.strip, p.anchor) == (was.item, was.strip, was.anchor)),
                    "{at}: {was:?} moved"
                );
            }
            assert_eq!(home.door, record.door, "{at}: her door's wall");
            // Its pane hidden.
            let hidden = IdleView {
                nooks: quiet.nooks[1..].to_vec(),
                ..quiet.clone()
            };
            for _ in 0..4 {
                let frame = next(&mut guest, &real, &hidden, &mut now, 250);
                assert!(state(&guest), "{at}: her home, its pane hidden");
                hides(&guest, &frame, &real, &format!("{at}: its pane hidden"));
            }
            let frame = next(&mut guest, &real, &quiet, &mut now, 250);
            shows(&guest, &frame, &real, &format!("{at}: its pane back"));
            assert_eq!(
                guest.ledger.home.structures, record.structures,
                "{at}: her shelf's record"
            );
        }
    }
}

/// Her goodbye, frame by frame (both modes), with her plant up on her
/// shelf and with none: before the rain her shelf shows as it was frozen
/// (in line art, but for the cells her plant's image takes in), and her
/// plant shows; once the rain has settled, the real frame; no cell of
/// the shelf the rain has settled shows it again (the goodbye's frame is
/// never written into); and every cell of the shelf, and every cell her
/// plant was drawn on, rains.
#[test]
fn her_shelf_rains_out_with_her_goodbye() {
    // Her plant on the run at 66 (`Left` 3), rows 2..5, on row 5's cells.
    let plant_at = 66..69u16;
    let plant_cells: Vec<(u16, u16)> = (2..ROW)
        .flat_map(|y| plant_at.clone().map(move |x| (x, y)))
        .collect();
    // Without her plant too: then every cell of the run shows its glyph
    // before the rain, in line art as well.
    for plant in [false, true] {
        for graphics in [false, true] {
            let at = format!("plant={plant} graphics={graphics}");
            let (real, view) = rooms_frame(100, 30);
            let mut guest = shelved(7, sat(14, 0), graphics);
            if plant {
                assert!(guest.ledger.home.add(room::Prop {
                    item: Furniture::Plant,
                    ..mantel_clock()
                }));
            }
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            next(&mut guest, &real, &view, &mut now, 250);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            assert_eq!(
                visit.shown.iter().any(|s| s.item == Furniture::Plant
                    && s.strip == Some(room::Strip::Shelf(Nook::Users))),
                plant,
                "{at}: her plant up"
            );
            guest.activity(now);
            assert!(matches!(guest.state, State::Leaving(_)), "{at}: leaving");
            let start = now;
            let mut settled = [false; 10];
            let mut rained = [false; 10];
            // Each cell of her plant: drawn before the rain (not the real
            // cell), and painted by the rain.
            let mut plant_drawn = vec![false; plant_cells.len()];
            let mut plant_rained = vec![false; plant_cells.len()];
            while now < start + dissolve::DURATION_MS {
                let frame = paint(&mut guest, &real, &view, now);
                let t = now - start;
                let State::Leaving(leaving) = &guest.state else {
                    panic!("{at}: leaving at {t}");
                };
                for (i, x) in RUN.enumerate() {
                    let shown = frame[(x, ROW)].symbol().to_owned();
                    let under_plant = plant && graphics && plant_at.contains(&x);
                    if t < dissolve::RAIN_FROM_MS && !under_plant {
                        assert_eq!(
                            shown,
                            GLYPHS.chars().nth(i).unwrap().to_string(),
                            "{at}: t {t} x {x}"
                        );
                    }
                    let painting = leaving.dissolve.painting(x, ROW);
                    rained[i] |= painting && t >= dissolve::RAIN_FROM_MS;
                    if settled[i] || t >= dissolve::SETTLE_BY_MS {
                        assert_eq!(frame[(x, ROW)], real[(x, ROW)], "{at}: t {t} x {x} settled");
                    }
                    settled[i] |= !painting;
                }
                for (i, &(x, y)) in plant_cells.iter().enumerate().filter(|_| plant) {
                    let painting = leaving.dissolve.painting(x, y);
                    plant_rained[i] |= painting && t >= dissolve::RAIN_FROM_MS;
                    if t < dissolve::RAIN_FROM_MS {
                        plant_drawn[i] |= frame[(x, y)] != real[(x, y)];
                        if !graphics {
                            assert_ne!(
                                frame[(x, y)],
                                real[(x, y)],
                                "{at}: t {t} her plant at {x},{y}"
                            );
                        }
                    }
                    if t >= dissolve::SETTLE_BY_MS {
                        assert_eq!(frame[(x, y)], real[(x, y)], "{at}: t {t} {x},{y} settled");
                    }
                }
                now += dissolve::FRAME_MS;
            }
            assert_eq!(rained, [true; 10], "{at}: each cell rained");
            // Every cell her plant was drawn on rains (in line art its
            // image is frozen whole: some of its cells may be blank in
            // it; in ASCII every cell has a glyph).
            if plant {
                assert!(plant_drawn.iter().any(|&d| d), "{at}: her plant drawn");
                for (i, cell) in plant_cells.iter().enumerate() {
                    assert!(
                        !plant_drawn[i] || plant_rained[i],
                        "{at}: her plant at {cell:?} rained"
                    );
                }
            }
            assert_eq!(paint(&mut guest, &real, &view, now), real, "{at}: the end");
            gone_for_good(&mut guest, &real, &view, now, &at);
        }
    }
}

/// Her goodbye over, she's out of the house with her shelf on record:
/// the frames after show nothing of it.
fn gone_for_good(guest: &mut Guest, real: &Buffer, view: &IdleView, mut now: u64, at: &str) {
    for _ in 0..3 {
        now += dissolve::FRAME_MS;
        guest.advance(now);
        let frame = paint(guest, real, view, now);
        assert!(matches!(guest.state, State::Absent), "{at}: out at {now}");
        assert_eq!(frame, *real, "{at}: out at {now}");
    }
}

/// Each cell of her shelf rains out with the goodbye begun in a frame her
/// shelf was written into (`started`: the paint her state changed in):
/// in that frame it shows still frozen, and the rain reaches it later.
fn rains_from_its_first_frame(guest: &Guest, at: &str) -> [bool; 10] {
    let State::Leaving(leaving) = &guest.state else {
        panic!("{at}: leaving");
    };
    let mut frozen = [false; 10];
    for (i, x) in RUN.enumerate() {
        frozen[i] = leaving.dissolve.painting(x, ROW);
    }
    frozen
}

/// Watch a goodbye begun at `now` on to its end: each cell of her shelf
/// painted at least once after the rain begins. When it ended.
fn rains_on(guest: &mut Guest, real: &Buffer, view: &IdleView, mut now: u64, at: &str) -> u64 {
    let start = match &guest.state {
        State::Leaving(leaving) => leaving.dissolve.started(),
        _ => panic!("{at}: leaving"),
    };
    let mut rained = [false; 10];
    while now < start + dissolve::DURATION_MS {
        now += dissolve::FRAME_MS;
        guest.advance(now);
        paint(guest, real, view, now);
        let State::Leaving(leaving) = &guest.state else {
            break;
        };
        if now - start >= dissolve::RAIN_FROM_MS {
            for (i, x) in RUN.enumerate() {
                rained[i] |= leaving.dissolve.painting(x, ROW);
            }
        }
    }
    assert_eq!(
        rained, [true; 10],
        "{at}: each cell rained after the rain began"
    );
    now
}

/// A visitor's errand (she comes from nowhere for the accordion and
/// leaves once she has poked it, the video playing) with her shelf up:
/// her shelf shows from the errand's first frame (the visit begun by
/// the errand keeps it), and the goodbye begun in the paint where she's
/// done rains it out (it's never settled in that frame: the frame's own
/// write is undone before the goodbye reads it). Both modes.
#[test]
fn an_errand_goodbye_rains_her_shelf_out() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, accordion) = accordion_room(100, 30, 2);
        let panes = nooks(100, 30);
        let base = IdleView {
            busy: Some(Busy::Playing),
            chat: panes[0].1,
            nooks: panes[1..].to_vec(),
            ..view(bottom_strip(100, 30))
        };
        let view = scrolled_back(base, accordion, 2);
        let mut guest = Guest::new(5);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.ledger.home.structures.push(users_shelf());
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        let mut first = true;
        loop {
            assert!(now < 120_000, "{at}: she never left");
            let was_visiting = matches!(guest.state, State::Visiting(_));
            let frame = next(&mut guest, &real, &view, &mut now, 50);
            match &guest.state {
                State::Visiting(_) if first => {
                    first = false;
                    shows(
                        &guest,
                        &frame,
                        &real,
                        &format!("{at}: her errand's first frame"),
                    );
                }
                State::Leaving(_) if was_visiting => break,
                State::Leaving(_) => panic!("{at}: leaving, not from her visit"),
                _ => {}
            }
        }
        assert!(!first, "{at}: she came");
        assert!(guest.errand.is_none(), "{at}: done");
        assert_eq!(
            rains_from_its_first_frame(&guest, &at),
            [true; 10],
            "{at}: frozen in the goodbye's first frame"
        );
        let end = rains_on(&mut guest, &real, &view, now, &at);
        gone_for_good(&mut guest, &real, &view, end, &at);
    }
}

/// Her coming in from her empty home called off in the paint she'd come
/// in on (nowhere on the frame to stand, though her shelf's row is
/// clear): her empty home rains out, her shelf with it, frozen in that
/// frame (its write there undone before the goodbye reads it) and rained
/// on. Both modes.
#[test]
fn a_called_off_arrival_rains_her_shelf_out() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let real = rooms(100, 30);
        let quiet = in_use(100, None);
        let mut guest = shelved(4, tue(9, 0), graphics);
        let mut now = 0;
        paint(&mut guest, &real, &quiet, now);
        while !matches!(guest.state, State::Away(_)) {
            assert!(now < 200_000, "{at}: her home never stood empty");
            next(&mut guest, &real, &quiet, &mut now, 250);
        }
        let frame = next(&mut guest, &real, &quiet, &mut now, 250);
        shows(&guest, &frame, &real, &format!("{at}: her empty home"));
        // Every floor of the frame kept clear: none she could stand on,
        // her shelf's row and the rows over it left free.
        let users = nooks(100, 30)[1].1;
        let nowhere = IdleView {
            protected: vec![
                nooks(100, 30)[0].1,
                Rect::new(users.x, ROW + 1, users.width, users.height - ROW - 1),
                nooks(100, 30)[2].1,
                Rect::new(0, 27, 100, 3),
            ],
            ..quiet.clone()
        };
        guest.cue(Scene::Arrive);
        assert!(
            matches!(guest.state, State::Arriving(How::Idle, Some(_))),
            "{at}: cued in from her home"
        );
        now += 1;
        guest.advance(now);
        paint(&mut guest, &real, &nowhere, now);
        assert!(
            matches!(guest.state, State::Leaving(_)),
            "{at}: her home rains out, not {}",
            match &guest.state {
                State::Absent => "absent",
                State::Arriving(..) => "arriving",
                State::Visiting(_) => "visiting",
                State::Away(_) => "away",
                State::Leaving(_) => "leaving",
            }
        );
        assert_eq!(
            rains_from_its_first_frame(&guest, &at),
            [true; 10],
            "{at}: frozen in the goodbye's first frame"
        );
        let end = rains_on(&mut guest, &real, &nowhere, now, &at);
        gone_for_good(&mut guest, &real, &nowhere, end, &at);
    }
}

/// Smoke: standing on a text floor under her shelf (her box meets its
/// row), she waves goodbye in line art: her image's cells recorded the
/// real frame under it, so the goodbye's frame (her shelf undone) still
/// matches them. Not the guard: see
/// [`every_cell_drawn_over_her_shelf_records_the_real_frame`].
#[test]
fn she_waves_goodbye_under_her_shelf() {
    let placeholder = |buf: &Buffer| {
        buf.content
            .iter()
            .any(|cell| cell.symbol().contains('\u{10EEEE}'))
    };
    let mut real = rooms(100, 30);
    real.set_string(52, 9, "─".repeat(28), Style::new());
    let view = in_use(100, None);
    let mut guest = Guest::new(7);
    guest.set_picker(kitty());
    guest.ledger.home.structures.push(users_shelf());
    visiting_at(&mut guest, &real, &view, (68, 9));
    let frame = paint(&mut guest, &real, &view, 1);
    let State::Visiting(visit) = &guest.state else {
        panic!("visiting");
    };
    assert_eq!((visit.osaka.x, visit.osaka.y), (68, 9), "standing under it");
    assert!(placeholder(&frame), "line art");
    assert!(
        visit
            .painted
            .iter()
            .take(RUN.len())
            .all(|c| c.y == ROW && !c.burst),
        "her shelf kept"
    );
    assert!(
        visit
            .painted
            .iter()
            .any(|c| c.burst && c.y == ROW && RUN.contains(&c.x)),
        "her image over her shelf"
    );
    guest.activity(2);
    let beat = paint(&mut guest, &real, &view, 100);
    assert!(placeholder(&beat), "startled, still line art: she waves");
}

/// Every cell of hers drawn over a written cell records the real frame
/// under it, not the structure (D4): her ASCII sprite, her line-art box
/// and the pieces in it, her pieces drawn on their own (both modes), her
/// door standing on its own (both modes). The structure's cells are
/// written by hand here over her box, the pieces and the door, as
/// `read_ground` writes them.
#[test]
fn every_cell_drawn_over_her_shelf_records_the_real_frame() {
    use crate::ui::houseguest::cells::{Ink, put};
    let real = rooms(100, 30);
    let nooks = nooks(100, 30);
    let mut home = room::Home::default();
    assert!(home.add(room::Prop::new(
        Furniture::Sofa,
        Nook::Users,
        300,
        sprite::Facing::Right
    )));
    let shown = home.layout(&nooks);
    let sofa = shown[0];
    let (x, y) = (
        i32::from(sofa.cover().x) + 2,
        i32::from(sofa.cover().bottom()) - 1,
    );
    let ink = Ink::new(Color::Cyan, Modifier::empty());
    // Written over `cells` of `real`: the frame and what was written.
    let written_over = |cells: &[(i32, i32)]| {
        let mut buf = real.clone();
        let mut frozen = Vec::new();
        for &(cx, cy) in cells {
            let (ux, uy) = (u16::try_from(cx).unwrap(), u16::try_from(cy).unwrap());
            let under = buf[(ux, uy)].clone();
            if under.symbol() == " " && put(&mut buf, cx, cy, '─', ink) {
                frozen.push(Frozen {
                    x: ux,
                    y: uy,
                    glyph: '─',
                    ink,
                    under,
                    face: None,
                    burst: false,
                });
            }
        }
        assert!(!frozen.is_empty());
        (buf, Written::of(frozen))
    };
    let around = |left: i32, top: i32, right: i32, bottom: i32| -> Vec<(i32, i32)> {
        (top..bottom)
            .flat_map(|cy| (left..right).map(move |cx| (cx, cy)))
            .collect()
    };
    // Each painted cell at a written place carries the real cell.
    let check = |painted: &[Frozen], written: Written, burst_only: bool, what: &str| {
        let mut over = 0;
        for cell in painted {
            if !written.contains(i32::from(cell.x), i32::from(cell.y))
                || (burst_only && !cell.burst)
            {
                continue;
            }
            over += 1;
            assert_eq!(
                cell.under,
                real[(cell.x, cell.y)],
                "{what}: ({}, {})",
                cell.x,
                cell.y
            );
        }
        assert!(over > 0, "{what}: drawn over what was written");
        written.keep(&mut Vec::new());
    };
    let mut rng = Rng(1);
    let osaka = Osaka::standing_at(x, y, 0, &mut rng);
    let her_box = around(x - 2, y - 4, x + 3, y);
    let footprint = {
        let c = sofa.cover();
        around(
            i32::from(c.x),
            i32::from(c.y),
            i32::from(c.right()),
            i32::from(c.bottom()) - 1,
        )
    };
    // Her ASCII sprite.
    {
        let (mut buf, written) = written_over(&her_box);
        let terrain = Terrain::read_with(&buf, &[], false, &written);
        let sprite = Sprite {
            front: None,
            doorway: None,
            protected: &[],
        };
        let (painted, _) = draw(
            &mut buf,
            &written,
            &osaka,
            &terrain,
            &[],
            Some(sprite),
            0,
            true,
        );
        check(&painted, written, false, "her ASCII sprite");
    }
    // Her line-art box and the sofa in it.
    {
        let mut graphics = Graphics::new(kitty()).expect("kitty");
        let cells: Vec<(i32, i32)> = her_box.iter().chain(&footprint).copied().collect();
        let (mut buf, written) = written_over(&cells);
        let mut terrain = Terrain::read_with(&buf, &[], true, &written);
        terrain.furnish([sofa.cover()]);
        let figure = Figure::of(&osaka, None, None, 0);
        let (painted, image, _) = draw_art(
            &mut buf,
            &written,
            &mut graphics,
            &osaka,
            &terrain,
            &shown,
            figure,
            vec![sofa],
            &Looks::default(),
            0,
            true,
        );
        assert!(
            image.is_some_and(|image| !image.with.is_empty()),
            "the sofa in her image"
        );
        check(&painted, written, true, "her line-art box");
    }
    // Her pieces on their own.
    for graphics in [false, true] {
        let mut art = graphics.then(|| Graphics::new(kitty()).expect("kitty"));
        let (mut buf, written) = written_over(&footprint);
        let painted = draw_props(
            &mut buf,
            &written,
            art.as_mut(),
            &shown,
            &Looks::default(),
            true,
        );
        check(
            &painted,
            written,
            graphics,
            &format!("her pieces, graphics={graphics}"),
        );
    }
    // Her door on its own, in its wall by Users' left wall.
    let spot = door::DoorSpot::at(
        53,
        12,
        door::Set::Wall {
            side: room::Side::Left,
            wall: 50,
        },
    );
    for graphics in [false, true] {
        let mut art = graphics.then(|| Graphics::new(kitty()).expect("kitty"));
        let front = Front::at(spot, AWAY, art::Sky::Day, None, 0);
        let (left, top, right, bottom) = front.bounds();
        let (mut buf, written) = written_over(&around(left.max(51), top, right, bottom - 1));
        let terrain = Terrain::read_with(&buf, &[], graphics, &written);
        let (painted, _) = draw_front(&mut buf, &written, art.as_mut(), front, &terrain, &[]);
        check(
            &painted,
            written,
            graphics,
            &format!("her door, graphics={graphics}"),
        );
    }
}

/// Out of the house with her shelf on record, a paint writes it only
/// when it could bring her in (an errand: visits on, a poke due), so the
/// commonest frames (her out, the video playing, the houseguest off)
/// read nothing for it and write nothing to undo; and when it was
/// written and she doesn't come (the poke held off while someone
/// selects), that frame is the app's own all the same. Both modes.
#[test]
fn out_of_the_house_her_shelf_is_written_only_when_she_could_come() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, accordion) = accordion_room(100, 30, 2);
        let panes = nooks(100, 30);
        let base = IdleView {
            busy: Some(Busy::Selection),
            chat: panes[0].1,
            nooks: panes[1..].to_vec(),
            ..view(bottom_strip(100, 30))
        };
        let view = scrolled_back(base, accordion, 2);
        let off = IdleView {
            delay: None,
            ..view.clone()
        };
        let mut guest = Guest::new(5);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.ledger.home.structures.push(users_shelf());
        assert_eq!(paint(&mut guest, &real, &view, 0), real, "{at}: first");
        assert!(matches!(guest.state, State::Absent), "{at}: out");
        let size = (real.area.width, real.area.height);
        // What a paint at `now` on `view` writes, undone again.
        let writes = |guest: &mut Guest, view: &IdleView, now: u64| {
            let mut buf = real.clone();
            let written = guest.read_ground(&mut buf, view, size, now);
            let wrote = written.meets(buf.area);
            written.undo(&mut buf);
            assert_eq!(buf, real, "{at}: undone");
            wrote
        };
        let due = nudge::NUDGE_MS;
        assert!(!guest.nudge.due(due - 1), "{at}: no poke due yet");
        assert!(!writes(&mut guest, &view, due - 1), "{at}: no poke due");
        assert!(guest.nudge.due(due), "{at}: a poke due");
        assert!(!writes(&mut guest, &off, due), "{at}: visits off");
        assert!(writes(&mut guest, &view, due), "{at}: she could come");
        // The poke held off: she stays out, and the frame is the app's.
        let frame = paint(&mut guest, &real, &view, due);
        assert!(matches!(guest.state, State::Absent), "{at}: still out");
        assert_eq!(frame, real, "{at}: the frame her shelf was written into");
    }
}

/// A visit with nowhere left to stand ends at once (no goodbye), and the
/// frame it ends in shows nothing of her shelf, though it was written
/// there first. Both modes.
#[test]
fn a_visit_with_nowhere_to_stand_takes_her_shelf_with_her() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let real = rooms(100, 30);
        let quiet = in_use(100, None);
        let mut guest = shelved(4, sat(14, 0), graphics);
        let mut now = until_visiting(&mut guest, &real, &quiet, 0);
        let frame = next(&mut guest, &real, &quiet, &mut now, 250);
        shows(&guest, &frame, &real, &at);
        // Every floor kept clear, her shelf's row left free.
        let users = nooks(100, 30)[1].1;
        let nowhere = IdleView {
            protected: vec![
                nooks(100, 30)[0].1,
                Rect::new(users.x, ROW + 1, users.width, users.height - ROW - 1),
                nooks(100, 30)[2].1,
                Rect::new(0, 27, 100, 3),
            ],
            ..quiet.clone()
        };
        let mut buf = real.clone();
        let size = (real.area.width, real.area.height);
        let written = guest.read_ground(&mut buf, &nowhere, size, now + 1);
        assert!(written.meets(buf.area), "{at}: written there");
        written.undo(&mut buf);
        now += 1;
        let frame = paint(&mut guest, &real, &nowhere, now);
        assert!(matches!(guest.state, State::Absent), "{at}: gone");
        assert_eq!(run_of(&frame), run_of(&real), "{at}: her shelf with her");
    }
}

/// Her shelf keeps off what of hers the frame doesn't show yet at the
/// write: a glyph she moved onto its margin (her text layer, before it's
/// validated) hides it that frame, and it's back once the glyph is; a
/// piece she made standing on its margin hides it that frame (the piece
/// falls apart: it may not stand there), and it's back the next. Both
/// modes.
#[test]
fn her_shelf_keeps_off_her_text_and_what_she_made() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut real = rooms(100, 30);
        real.set_string(80, 2, "abc", Style::new());
        let quiet = in_use(100, None);
        let mut guest = shelved(4, sat(14, 0), graphics);
        let mut now = until_visiting(&mut guest, &real, &quiet, 0);
        let frame = next(&mut guest, &real, &quiet, &mut now, 250);
        shows(&guest, &frame, &real, &format!("{at}: before"));
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        assert!(
            visit.layer.take(&real, &[], (80, 2), (74, ROW)),
            "{at}: a glyph moved onto its margin"
        );
        now += 1;
        let frame = paint(&mut guest, &real, &quiet, now);
        hides(
            &guest,
            &frame,
            &real,
            &format!("{at}: her text on its margin"),
        );
        if let State::Visiting(visit) = &mut guest.state {
            visit.layer = layer::TextLayer::default();
        }
        now += 1;
        let frame = paint(&mut guest, &real, &quiet, now);
        shows(&guest, &frame, &real, &format!("{at}: her text gone"));
    }
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now, real, view, _) =
            super::away::made_by_playlists_right_wall(graphics);
        guest.ledger.home.structures.push(users_shelf());
        now += 100;
        let frame = paint(&mut guest, &real, &view, now);
        shows(&guest, &frame, &real, &format!("{at}: before"));
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        let [made] = visit.made.as_mut_slice() else {
            panic!("{at}: one piece");
        };
        // Its cover's left column on her shelf's last margin cell.
        let cover = made.piece.cover();
        made.piece.left += 74 - i32::from(cover.x);
        made.piece.floor += i32::from(ROW) - i32::from(cover.bottom() - 1);
        let cover = made.piece.cover();
        assert!(
            cover.contains((74, ROW).into()) && !cover.intersects(Rect::new(63, ROW, 10, 1)),
            "{at}: {cover:?} on the margin alone"
        );
        now += 100;
        let frame = paint(&mut guest, &real, &view, now);
        hides(
            &guest,
            &frame,
            &real,
            &format!("{at}: a made piece on its margin"),
        );
        now += 100;
        let frame = paint(&mut guest, &real, &view, now);
        shows(&guest, &frame, &real, &format!("{at}: after"));
        assert!(visit_of(&guest).made.is_empty(), "{at}: it fell apart");
    }
}

/// Her closed door, standing on its own while she's out, never hops for
/// her shelf: placed (as a fallback: her door's space and every pane
/// floor kept clear, the only floors two runs of text `─` on row 9, one
/// under her shelf, nearer her door's wall, and one past it) while her
/// shelf is hidden, by text on its margin or its pane focused and in
/// use, it stands where it stood once the shelf shows again, never
/// meeting its run or margin. Both modes.
#[test]
fn her_door_standing_alone_never_hops_for_her_shelf() {
    let users = nooks(100, 30)[1].1;
    let mut real = rooms(100, 30);
    real.set_string(60, 9, "─".repeat(17), Style::new());
    real.set_string(80, 9, "─".repeat(15), Style::new());
    let mut texty = real.clone();
    texty.set_string(74, ROW, "x", Style::new());
    let kept = Rect::new(61, ROW, 14, 1);
    let shut = |view: IdleView| IdleView {
        protected: [
            view.protected.clone(),
            vec![Rect::new(users.x, 10, users.width, 3), nooks(100, 30)[2].1],
        ]
        .concat(),
        ..view
    };
    for graphics in [false, true] {
        for focused in [false, true] {
            let at = format!("graphics={graphics} focused={focused}");
            let mut guest = home_at(
                4,
                tue(9, 0),
                &[(Furniture::Bed, Nook::Playlist, 300)],
                graphics,
            );
            guest.ledger.home.structures.push(users_shelf());
            guest.ledger.home.door = Some(room::DoorWall {
                strip: room::Strip::Bottom(Nook::Playlist),
                side: room::Side::Left,
            });
            let quiet = in_use(100, None);
            let mut now = 0;
            paint(&mut guest, &real, &quiet, now);
            while !matches!(guest.state, State::Away(_)) {
                assert!(now < 200_000, "{at}: her home never stood empty");
                next(&mut guest, &real, &quiet, &mut now, 250);
            }
            let door = |guest: &Guest| match &guest.state {
                State::Away(empty) => empty.door.as_ref().map(|d| d.spot),
                _ => panic!("{at}: her home stands empty"),
            };
            // Hidden (a focused pane's rain over).
            let (frame, view) = if focused {
                (&real, shut(in_use(100, Some(users))))
            } else {
                (&texty, shut(quiet.clone()))
            };
            let from = now;
            let mut shown = next(&mut guest, frame, &view, &mut now, 250);
            while now < from + dissolve::DURATION_MS + 250 {
                shown = next(&mut guest, frame, &view, &mut now, 250);
            }
            hides(&guest, &shown, frame, &format!("{at}: hidden"));
            let first = door(&guest).expect("a door");
            assert!(
                matches!(first.set(), door::Set::Floor(_)),
                "{at}: {first:?}"
            );
            // Shown.
            let quiet = shut(quiet);
            for _ in 0..3 {
                let shown = next(&mut guest, &real, &quiet, &mut now, 250);
                shows(&guest, &shown, &real, &format!("{at}: shown"));
                assert_eq!(door(&guest), Some(first), "{at}: it stays");
            }
            let (x, y) = first.spot();
            let her = room::her_box(x, y).expect("on the screen");
            assert!(!her.intersects(kept), "{at}: {first:?} meets her shelf");
        }
    }
}

/// No pull is offered across her shelf, shown or not: standing on a text
/// floor under it, hidden by a glyph on its margin, she's offered no
/// line on its row whose glyphs or the gap to them meet its run or
/// margin (the frame refuses moving text there: she'd walk to it only to
/// lose her grip), though read without the shelf there is one (not
/// vacuous). Both modes.
#[test]
fn no_pull_is_offered_across_her_shelf() {
    let mut real = rooms(100, 30);
    // A floor whose box rows reach her shelf's row with the rows she
    // pulls from (`scenes::PULL_ROWS`).
    real.set_string(52, 8, "─".repeat(40), Style::new());
    real.set_string(74, ROW, "xyz", Style::new());
    let view = in_use(100, None);
    let kept = 61..=74;
    let across = |pulls: &[scenes::Pull]| {
        pulls.iter().any(|p| {
            let half = sprite::WIDTH / 2;
            let edge = match p.side {
                scenes::Side::Left => p.x - half - 1,
                scenes::Side::Right => p.x + half + 1,
            };
            let cells = p.cells.iter().map(|&c| i32::from(c));
            let lo = cells.clone().min().unwrap_or(edge).min(edge);
            let hi = cells.max().unwrap_or(edge).max(edge);
            p.row == ROW && lo <= *kept.end() && hi >= *kept.start()
        })
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(7);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.ledger.home.structures.push(users_shelf());
        visiting_at(&mut guest, &real, &view, (68, 8));
        let frame = paint(&mut guest, &real, &view, 1);
        hides(&guest, &frame, &real, &at);
        let visit = visit_of(&guest);
        let bare = scenes::pulls(&real, &visit.terrain, &view.protected);
        assert!(across(&bare), "{at}: a pull across it, but for her shelf");
        assert!(!across(&visit.chances.pulls), "{at}: offered across it");
    }
}

/// No swap is offered across her shelf, shown or not, nor made there
/// (phase 6a D4: what she moves or swaps keeps off its run and margin):
/// standing on a text floor under it, a word on its row with letters on
/// its margin (hiding it), she's offered no swap of a letter there,
/// though read without the shelf there is one (not vacuous); cued to
/// swap letters, nothing of her layer ever lands on the run or margin.
/// The swaps read the keep-out the frame applies her layer by (`base`),
/// not her pulls' (`protected`). Both modes.
#[test]
fn no_swap_is_offered_across_her_shelf() {
    let mut real = rooms(100, 30);
    // A floor whose box rows reach her shelf's row (`scenes::PULL_ROWS`),
    // and a word from its margin's last cell (74) on.
    real.set_string(52, 8, "─".repeat(40), Style::new());
    real.set_string(74, ROW, "wxyz", Style::new());
    let view = in_use(100, None);
    let kept = 61..=74u16;
    let on_it = |(x, y): (u16, u16)| y == ROW && kept.contains(&x);
    let across = |swaps: &[scenes::Swap]| {
        swaps.iter().any(|s| {
            [s.a.at, s.a.source, s.b.at, s.b.source]
                .into_iter()
                .any(on_it)
        })
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(7);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.ledger.home.structures.push(users_shelf());
        visiting_at(&mut guest, &real, &view, (68, 8));
        let frame = paint(&mut guest, &real, &view, 1);
        hides(&guest, &frame, &real, &at);
        let visit = visit_of(&guest);
        let bare = scenes::swaps(&real, &visit.terrain, &view.protected, &visit.layer);
        assert!(across(&bare), "{at}: a swap across it, but for her shelf");
        assert!(!across(&visit.chances.swaps), "{at}: offered across it");
        // Cued to swap letters: whatever she swaps, none of it there.
        guest.cue(Scene::Swap);
        let mut now = 1;
        for _ in 0..120 {
            now += 250;
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                break;
            };
            assert!(
                !visit.layer.cells().any(on_it),
                "{at}: her layer on her shelf at {now}"
            );
        }
    }
}

/// [`rooms`] at 100×30 with the chat pane laid over all of it below row
/// 6 (a custom layout's grid: List and the Playlist in it, Users' lower
/// rows too), her three panes hers: no floor of hers takes a piece
/// anew, and her shelf in Users (run `63..73` on row 5) is clear of it.
fn chat_below_her_shelf() -> (Buffer, IdleView) {
    let (real, view) = rooms_frame(100, 30);
    let view = IdleView {
        chat: Rect::new(0, 6, 100, 24),
        ..view
    };
    (real, view)
}

/// Her plant in the chat with only her shelf to go to (phase 6a, step
/// 3): industrious, she feels it, works out the one move there is (the
/// precondition: up on the shelf, centred on the run at 66), carries it
/// up from the floor below and sets it down arms up (`Pose::Stretch`,
/// never bending to it), and it stands on her shelf, mended. Both
/// modes.
#[test]
fn a_plant_in_the_chat_goes_up_on_her_shelf() {
    let (real, view) = chat_below_her_shelf();
    let shelf = room::Strip::Shelf(Nook::Users);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            5,
            tue(14, 0),
            &[(Furniture::Plant, Nook::List, 400)],
            graphics,
        );
        guest.ledger.home.structures.push(users_shelf());
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &mut guest.state else {
            unreachable!()
        };
        visit.osaka.set_mood(brain::Mood::Industrious);
        // The precondition: the one move she works out is up there.
        let found = run_until(
            &mut guest,
            &real,
            &view,
            now,
            now + 60_000,
            |guest, _| matches!(&guest.state, State::Visiting(v) if !v.repairs.is_empty()),
        );
        now = found.unwrap_or_else(|| panic!("{at}: never worked out a move"));
        let State::Visiting(visit) = &guest.state else {
            unreachable!()
        };
        let moves: Vec<(Furniture, room::Strip, i32)> = visit
            .repairs
            .iter()
            .map(|r| (r.piece, r.to.strip, r.at.left))
            .collect();
        assert_eq!(moves, [(Furniture::Plant, shelf, 66)], "{at}");
        // Carried up and set down, arms up.
        let mut stretched = false;
        let up = run_until(
            &mut guest,
            &real,
            &view,
            now,
            now + 240_000,
            |guest, now| {
                let State::Visiting(visit) = &guest.state else {
                    return false;
                };
                if visit.osaka.carrying() == Some(Furniture::Plant) {
                    let pose = visit.osaka.appearance(now).0;
                    assert!(
                        !matches!(pose, sprite::Pose::ToeTouch(_)),
                        "{at}: bent to it at {now}"
                    );
                    stretched |= pose == sprite::Pose::Stretch;
                }
                guest
                    .ledger
                    .home
                    .props
                    .iter()
                    .any(|p| p.item == Furniture::Plant && p.strip == shelf)
            },
        );
        now = up.unwrap_or_else(|| panic!("{at}: never put it up"));
        assert!(stretched, "{at}: never reached up");
        let frame = next(&mut guest, &real, &view, &mut now, 250);
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting")
        };
        let plant = visit.shown.iter().find(|s| s.item == Furniture::Plant);
        assert_eq!(
            plant.map(|s| (s.strip, s.left, s.floor, s.stood)),
            Some((Some(shelf), 66, i32::from(ROW), 12)),
            "{at}"
        );
        // Her shelf under it, written (in line art the plant's image takes
        // in the run's cells it stands on, and redraws them).
        if !graphics {
            assert_eq!(run_of(&frame), GLYPHS, "{at}: on her shelf");
        }
        let kept: Vec<u16> = painted(&guest)
            .iter()
            .filter(|c| c.y == ROW && !c.burst && matches!(c.glyph, '┬' | '─'))
            .map(|c| c.x)
            .collect();
        assert!(RUN.clone().all(|x| kept.contains(&x)), "{at}: {kept:?}");
        let State::Visiting(visit) = &guest.state else {
            unreachable!()
        };
        assert!(
            !visit
                .broken
                .iter()
                .any(|b| b.rule().map(|r| r.rule) == Some(rules::Rule::InChat)),
            "{at}: mended"
        );
    }
}

/// Her clock up on her Users shelf (a mantel clock), standing at 66
/// along its run.
fn mantel_clock() -> room::Prop {
    room::Prop {
        strip: room::Strip::Shelf(Nook::Users),
        anchor: Some(room::Anchor {
            side: room::Side::Left,
            offset: 3,
        }),
        ..room::Prop::new(Furniture::Clock, Nook::Users, 0, sprite::Facing::Right)
    }
}

/// A clock on a shelf is read from its room's floor (phase 6a D6): where
/// it's offered to her, it's over Users' floor (row 12, wall to wall),
/// at its middle; and as she leaves for school (08:15), standing on that
/// floor, she glances up at it ("Time for school!"). Both modes.
#[test]
fn she_reads_the_time_off_her_mantel_clock_from_the_floor_below() {
    use super::super::script::{ClockGlance, SCHOOL_TIME, ScriptId};
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let mut glanced = 0;
        for seed in 0..3u64 {
            let at = format!("graphics={graphics} {seed}");
            let mut guest = home_at(seed, tue(8, 10), &PIECES, graphics);
            guest.ledger.home.structures.push(users_shelf());
            assert!(guest.ledger.home.add(mantel_clock()));
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let due = real_of(&guest, now, tue(8, 15));
            while now + 3000 < due {
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            visit.osaka.place(60, 12, now);
            visit.osaka.offer_only = Some((brain::Want::Stand, "stand"));
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
            let visit = visit_of(&guest);
            assert_eq!(
                visit.chances.clock,
                Some(osaka::ClockOn {
                    x: 67,
                    floor: 12,
                    from: 51,
                    to: 99
                }),
                "{at}"
            );
            if first(&guest).is_none() {
                assert!(visit.osaka.hidden(now), "{at}: no routine by {now}");
                continue;
            }
            let osaka = &visit.osaka;
            if visit
                .chances
                .clock
                .and_then(|c| c.seen_from((osaka.x, osaka.y)))
                .is_none()
            {
                continue;
            }
            assert_eq!(first(&guest), Some("routine/glance"), "{at}");
            assert_eq!(
                osaka.plays().map(|p| (p.own, p.branch)),
                Some((ScriptId::ClockGlance, ClockGlance::School.branch())),
                "{at}"
            );
            assert_eq!(
                osaka.appearance(now).2,
                Some(osaka::Bubble::Say(SCHOOL_TIME)),
                "{at}"
            );
            glanced += 1;
        }
        assert!(glanced >= 1, "graphics={graphics}: never glanced up");
    }
}

/// Decor on her shelf counts for the room below (phase 6a D10), only
/// while it shows: standing on Users' floor in a visit (put there, and
/// staying), she feels her plant up on the shelf in the room's beauty
/// (what her chances hold, as the paint works it out where she stands);
/// text over the shelf's margin hides it (and the plant), and the beauty
/// she feels drops by the plant's; the text gone, it's back. Both modes.
#[test]
fn her_shelf_decor_is_pretty_only_while_it_shows() {
    let (real, view) = rooms_frame(100, 30);
    let plant = Furniture::Plant.spec().beauty;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = shelved(6, sat(14, 0), graphics);
        assert!(guest.ledger.home.add(room::Prop {
            item: Furniture::Plant,
            ..mantel_clock()
        }));
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        // A frame of `buf` with her on Users' floor (`60, 12`): the beauty
        // she feels there, and the room's as `beauty_at` sums it.
        let mut felt = |guest: &mut Guest, buf: &Buffer| {
            now += 250;
            guest.advance(now);
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            visit.osaka.place(60, 12, now);
            visit.osaka.offer_only = Some((brain::Want::Stand, "stand"));
            paint(guest, buf, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            assert_eq!((visit.osaka.x, visit.osaka.y), (60, 12), "{at}");
            let summed = beauty_at(&visit.shown, &view.nooks, (60, 12));
            assert!((visit.chances.beauty_here - summed).abs() < 1e-9, "{at}");
            visit.chances.beauty_here
        };
        let shown = felt(&mut guest, &real);
        let mut texty = real.clone();
        texty.set_string(74, ROW, "x", Style::new());
        let hidden = felt(&mut guest, &texty);
        assert!(
            (shown - hidden - plant).abs() < 1e-9,
            "{at}: {shown} then {hidden}"
        );
        let back = felt(&mut guest, &real);
        assert!((back - shown).abs() < 1e-9, "{at}: back");
    }
}

/// Where a shelf could go up on `buf` (phase 6a, step 3's measurement):
/// for each of `view`'s panes, how many places a high shelf has (rise
/// `HIGH_RISE` and up, its run and margin free and her height clear
/// over the run) and a low one (rise `LOW_RISE`, `LOW_ABOVE` clear), by
/// D4's per-cell test with nothing of hers on the frame; and the first
/// of each (rise, offset from the left).
fn shelf_places(buf: &Buffer, view: &IdleView) -> Vec<String> {
    use super::super::structures::{EDGE, KeepOff, LOW_ABOVE, LOW_RISE, SHELF_WIDTH, free};
    let plan = room::Plan {
        nooks: &view.nooks,
        chat: view.chat,
        screen: buf.area,
    };
    let keep_off = KeepOff {
        protected: &view.protected,
        layer: &[],
        made: &[],
    };
    let mut out = Vec::new();
    for &(nook, rect) in &view.nooks {
        let raw = room::pane_raw(rect);
        let bg = |from: i32| {
            buf.cell((
                u16::try_from(from + SHELF_WIDTH / 2).unwrap_or(0),
                u16::try_from(raw.floor).unwrap_or(0),
            ))
            .map_or(tuirealm::ratatui::style::Color::Reset, |c| c.bg)
        };
        let fits = |rise: u16, over: u16, from: i32| {
            let y = raw.floor - i32::from(rise);
            (from - 2..from + SHELF_WIDTH + 2)
                .all(|x| free(buf, (x, y), plan, &keep_off, &[], bg(from)))
                && (1..=i32::from(over)).all(|k| {
                    (from..from + SHELF_WIDTH)
                        .all(|x| free(buf, (x, y - k), plan, &keep_off, &[], bg(from)))
                })
        };
        let froms: Vec<i32> = (raw.from + EDGE..=raw.to - EDGE - SHELF_WIDTH).collect();
        let her = sprite::HEIGHT as u16;
        let highs: Vec<(u16, i32)> = (7..=raw.rows.saturating_sub(her))
            .flat_map(|rise| froms.iter().map(move |&f| (rise, f)))
            .filter(|&(rise, f)| fits(rise, her, f))
            .collect();
        let lows: Vec<(u16, i32)> = if raw.rows >= LOW_RISE + LOW_ABOVE {
            froms
                .iter()
                .map(|&f| (LOW_RISE, f))
                .filter(|&(rise, f)| fits(rise, LOW_ABOVE, f))
                .collect()
        } else {
            Vec::new()
        };
        out.push(format!(
            "{nook:?} {}x{} ({} rows inside): high {} (first {:?}), low {} (first {:?})",
            rect.width,
            rect.height,
            raw.rows,
            highs.len(),
            highs.first().map(|&(r, f)| (r, f - raw.from)),
            lows.len(),
            lows.first().map(|&(r, f)| (r, f - raw.from)),
        ));
    }
    out
}

/// The measurement (phase 6a, step 3; the user's third round): on the
/// bundled layout with typical content (an evening's chat, ten playlist
/// rows, four users: `fuller_ui`), at 100×30, 80×24, the user's
/// 191×46, and 100 wide at 32, 35, 40 and 50 tall, which panes take a
/// high shelf and which a low one; and the stage room's (`stage_ui` at
/// 100×30). Printed for plan.md's record. `cargo nextest run -p dessplay
/// --run-ignored only --no-capture -E 'test(/where_a_shelf_could_go/)'`.
#[test]
#[ignore = "a measurement: prints, asserts nothing"]
fn where_a_shelf_could_go() {
    let sizes = [
        (100, 30),
        (80, 24),
        (191, 46),
        (100, 32),
        (100, 35),
        (100, 40),
        (100, 50),
    ];
    for (w, h) in sizes {
        for (name, mut ui) in [
            ("fuller", super::super::stage::fuller_ui()),
            ("empty", real_ui()),
        ] {
            let (buf, view) = real_frame(&mut ui, w, h);
            eprintln!("{w}x{h} {name}: chat {:?}", view.chat);
            for line in shelf_places(&buf, &view) {
                eprintln!("  {line}");
            }
        }
    }
    let (buf, view) = real_frame(&mut stage_ui(), 100, 30);
    eprintln!("stage room 100x30: chat {:?}", view.chat);
    for line in shelf_places(&buf, &view) {
        eprintln!("  {line}");
    }
}

/// She reaches a piece on her shelf arms up, wired from what she'd move
/// (phase 6a D6; `arranging`): a repair that takes her plant off her
/// shelf puts it among the pieces she'd lift as one up there, and a move
/// she's judging onto her shelf is judged as one up there; the same off
/// and onto Users' floor, not. (No rule of 6a moves a piece off a shelf:
/// this is the wiring for step 7's.) Both modes.
#[test]
fn she_lifts_from_her_shelf_and_sets_down_on_it_arms_up() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        for up in [true, false] {
            let at = format!("graphics={graphics} up={up}");
            let mut guest = shelved(4, sat(14, 0), graphics);
            let plant = room::Prop {
                item: Furniture::Plant,
                ..mantel_clock()
            };
            let plant = if up {
                plant
            } else {
                room::Prop {
                    strip: room::Strip::Bottom(Nook::Users),
                    ..plant
                }
            };
            assert!(guest.ledger.home.add(plant));
            let now = until_visiting(&mut guest, &real, &view, 0);
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            let shown = *visit
                .shown
                .iter()
                .find(|s| s.item == Furniture::Plant)
                .unwrap_or_else(|| panic!("{at}: her plant shown"));
            assert_eq!(shown.on_shelf(), up, "{at}");
            let to = rules::Placement {
                strip: if up {
                    room::Strip::Bottom(Nook::Users)
                } else {
                    room::Strip::Shelf(Nook::Users)
                },
                anchor: room::Anchor {
                    side: room::Side::Right,
                    offset: 2,
                },
                facing: sprite::Facing::Right,
            };
            visit.repairs = vec![rules::Repair {
                key: rules::Grievance {
                    row: 0,
                    piece: Furniture::Plant,
                },
                piece: Furniture::Plant,
                to,
                at: shown,
                cost: 1,
                tier: 0,
            }];
            // Judging a move onto where the plant stands now: up there
            // or on the floor.
            visit.judging = Some(Judging {
                piece: Furniture::Plant,
                to,
                pocket: false,
                target: Some(shown),
                showing: Some(shown),
                laid: Some(shown),
            });
            let (lift_at, judged) = arranging(visit);
            assert_eq!(
                lift_at.iter().map(|l| (l.0, l.3)).collect::<Vec<_>>(),
                [(Furniture::Plant, up)],
                "{at}: lifted"
            );
            assert_eq!(judged.map(|j| j.up), Some(up), "{at}: set down");
        }
    }
}

/// The stage's shelf (phase 6a D11, step 5a): cued, it's up finished on
/// the very frame, where `Home::shelf_spot` puts one (as high as fits,
/// mid-wall), with small decor on it, shown. On [`rooms`] at 100×30
/// (List 25 rows inside, first in order) a high one, her plant on it; in
/// the stage room at 100×30 (only its List takes one, 7 rows inside) a
/// low one, with no third row over it for the plant: her clock, a mantel
/// clock there. Both modes.
#[test]
fn the_stage_puts_her_shelf_up_where_one_fits() {
    let stage_room = || {
        let mut ui = super::super::stage::stage_ui();
        real_frame(&mut ui, 100, 30)
    };
    let frames: [(&str, (Buffer, IdleView)); 2] =
        [("rooms", rooms_frame(100, 30)), ("stage", stage_room())];
    for (name, (real, view)) in frames {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.cue(Scene::Shelf);
            paint(&mut guest, &real, &view, 0);
            let note = guest.cue_note().cloned();
            assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
            let home = &guest.ledger.home;
            let [Structure::Shelf(shelf)] = home.structures[..] else {
                panic!("{at}: one shelf: {:?}", home.structures);
            };
            let list = view.nooks.iter().find(|(n, _)| *n == Nook::List).unwrap().1;
            let (left, right) = (i32::from(list.x) + 1, i32::from(list.right()) - 1);
            let floor = i32::from(list.bottom()) - 1;
            // Mid-wall: of two lefts as near its middle, the left.
            let from = (left + right - 10).div_euclid(2);
            let (item, rise, cols) = match name {
                "rooms" => (Furniture::Plant, 7, 3),
                _ => (Furniture::Clock, 5, 3),
            };
            assert_eq!(shelf.nook, Nook::List, "{at}");
            assert_eq!(shelf.rise, rise, "{at}");
            assert_eq!(shelf.built, SHELF_STAGES, "{at}");
            let run = match shelf.anchor.side {
                room::Side::Left => left + i32::from(shelf.anchor.offset),
                room::Side::Right => right - i32::from(shelf.anchor.offset) - 10,
            };
            assert_eq!(run, from, "{at}: {shelf:?}");
            let y = floor - i32::from(rise);
            // Written first, whole, kept as her home's bottom layer.
            let kept: String = painted(&guest)
                .iter()
                .take(10)
                .map(|c| {
                    assert_eq!(i32::from(c.y), y, "{at}");
                    c.glyph
                })
                .collect();
            assert_eq!(kept, GLYPHS, "{at}");
            assert_eq!(i32::from(painted(&guest)[0].x), from, "{at}");
            // On it, centred, shown, reached from the floor below.
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            let on = visit
                .shown
                .iter()
                .find(|s| s.on_shelf())
                .unwrap_or_else(|| panic!("{at}: something on it"));
            assert_eq!(on.item, item, "{at}");
            assert_eq!(on.strip, Some(room::Strip::Shelf(Nook::List)), "{at}");
            assert_eq!(
                (on.left, on.floor, on.stood),
                (from + (10 - cols) / 2, y, floor),
                "{at}"
            );
            assert!(!on.boxed(), "{at}");
        }
    }
}

/// Cue the stage's shelf on `guest` and paint the frame it plays on: that
/// frame, and the cue's note.
fn stage_shelf(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
) -> (Buffer, Option<Result<String, String>>) {
    guest.cue(Scene::Shelf);
    let frame = paint(guest, real, view, 0);
    (frame, guest.cue_note().cloned())
}

/// What her visit shows on her shelf of `nook` (none if she isn't
/// visiting).
fn up_on(guest: &Guest, nook: Nook) -> Vec<room::Shown> {
    match &guest.state {
        State::Visiting(visit) => visit
            .shown
            .iter()
            .filter(|s| s.strip == Some(room::Strip::Shelf(nook)))
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

/// With a shelf hers already (half built, or not begun), the stage
/// finishes it where it hangs (Users, though a shelf of the stage's own
/// would go up on List's wall) and stands her plant on it, centred.
#[test]
fn the_stage_finishes_the_shelf_she_has_where_it_hangs() {
    let (real, view) = rooms_frame(100, 30);
    let Structure::Shelf(users) = users_shelf();
    let keep_off = structures::KeepOff {
        protected: &view.protected,
        layer: &[],
        made: &[],
    };
    let fresh = room::Home::default().shelf_spot(&real, plan_of(&view, &real), &keep_off, None);
    assert_eq!(fresh.map(|s| s.nook), Some(Nook::List), "{fresh:?}");
    for graphics in [false, true] {
        for built in [0, 1] {
            let at = format!("graphics={graphics} built {built}");
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            let hers = Shelf { built, ..users };
            guest.ledger.home.structures.push(Structure::Shelf(hers));
            let (frame, note) = stage_shelf(&mut guest, &real, &view);
            assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
            let finished = Shelf {
                built: SHELF_STAGES,
                ..hers
            };
            assert_eq!(
                guest.ledger.home.structures,
                [Structure::Shelf(finished)],
                "{at}"
            );
            // Written first, whole, kept as her home's bottom layer (in
            // graphics her plant's image is drawn over the frame's run).
            let kept: Vec<_> = painted(&guest)
                .iter()
                .take(RUN.len())
                .map(|c| (c.x, c.y, c.glyph))
                .collect();
            let want: Vec<_> = RUN.zip(GLYPHS.chars()).map(|(x, g)| (x, ROW, g)).collect();
            assert_eq!(kept, want, "{at}");
            if !graphics {
                assert_eq!(run_of(&frame), GLYPHS, "{at}");
            }
            let on: Vec<_> = up_on(&guest, Nook::Users)
                .iter()
                .map(|s| (s.item, s.left, s.floor, s.stood))
                .collect();
            assert_eq!(on, [(Furniture::Plant, 66, 5, 12)], "{at}");
            assert!(guest.unsaved, "{at}: saved");
        }
    }
}

/// Where her shelf can't show this frame (its anchor too near its wall
/// now, or text over its run), the stage finishes it but stands nothing
/// on it: her plant stays on the floor it shows on, nothing is given her,
/// and the note says why.
#[test]
fn the_stage_moves_nothing_up_on_a_shelf_the_frame_hides() {
    let (real, view) = rooms_frame(100, 30);
    let Structure::Shelf(users) = users_shelf();
    let near_wall = Shelf {
        anchor: room::Anchor {
            side: room::Side::Left,
            offset: 2,
        },
        ..users
    };
    let mut texted = real.clone();
    texted.set_string(66, ROW, "x", Style::new());
    for graphics in [false, true] {
        for (case, shelf, real) in [
            ("near its wall", near_wall, &real),
            ("texted", users, &texted),
        ] {
            let at = format!("graphics={graphics} {case}");
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.ledger.home.structures.push(Structure::Shelf(shelf));
            assert!(guest.ledger.home.add(room::Prop::new(
                Furniture::Plant,
                Nook::Playlist,
                300,
                Facing::Left
            )));
            let (_, note) = stage_shelf(&mut guest, real, &view);
            assert!(matches!(note, Some(Err(_))), "{at}: {note:?}");
            let home = &guest.ledger.home;
            assert_eq!(
                home.structures,
                [Structure::Shelf(Shelf {
                    built: SHELF_STAGES,
                    ..shelf
                })],
                "{at}"
            );
            let plants: Vec<_> = home
                .props
                .iter()
                .filter(|p| p.item == Furniture::Plant)
                .map(|p| (p.strip, p.facing))
                .collect();
            assert_eq!(
                plants,
                [(room::Strip::Bottom(Nook::Playlist), Facing::Left)],
                "{at}"
            );
            assert!(!home.owns(Furniture::Clock), "{at}: nothing given");
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            assert!(
                visit.shown.iter().any(|s| s.item == Furniture::Plant
                    && s.strip == Some(room::Strip::Bottom(Nook::Playlist))),
                "{at}: her plant still shows on its floor"
            );
        }
    }
}

/// Three panes 10 rows tall (a low shelf at most, with three rows over
/// its run: room for the plant by geometry), on a screen 100×30 with
/// [`rooms`]' protected strip along the bottom.
fn low_rooms() -> (Buffer, IdleView) {
    let nooks = vec![
        (Nook::List, Rect::new(0, 0, 50, 10)),
        (Nook::Users, Rect::new(50, 0, 50, 10)),
        (Nook::Playlist, Rect::new(0, 10, 100, 12)),
    ];
    let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
    for &(_, pane) in &nooks {
        tuirealm::ratatui::widgets::Widget::render(
            tuirealm::ratatui::widgets::Block::bordered(),
            pane,
            &mut buf,
        );
    }
    buf.set_string(0, 28, "Tab Next pane | Enter Send", Style::new());
    (
        buf,
        IdleView {
            nooks,
            ..view(bottom_strip(100, 30))
        },
    )
}

/// The stage stands on her shelf the first small decor the frame shows
/// there: on a low shelf with three rows clear over its run by geometry,
/// the plant; with a word on the third row where the plant's top would
/// be, her clock (a mantel clock), and no plant given her.
#[test]
fn the_stage_puts_up_her_clock_where_the_frame_hides_her_plant() {
    let (real, view) = low_rooms();
    // List's low shelf: inside 1..49, its run 20..30 on row 4 (rise 5
    // over the floor at 9); the plant would stand at 23..26, rows 1..=3.
    let mut worded = real.clone();
    worded.set_string(24, 1, "x", Style::new());
    for graphics in [false, true] {
        for (frame, item) in [(&real, Furniture::Plant), (&worded, Furniture::Clock)] {
            let at = format!("graphics={graphics} {item:?}");
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            let (_, note) = stage_shelf(&mut guest, frame, &view);
            assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
            let home = &guest.ledger.home;
            let [Structure::Shelf(shelf)] = home.structures[..] else {
                panic!("{at}: one shelf: {:?}", home.structures);
            };
            assert_eq!((shelf.nook, shelf.rise), (Nook::List, 5), "{at}");
            let on: Vec<_> = up_on(&guest, Nook::List)
                .iter()
                .map(|s| (s.item, s.left, s.floor))
                .collect();
            assert_eq!(on, [(item, 23, 4)], "{at}");
            assert_eq!(home.props.len(), 1, "{at}: only it given: {:?}", home.props);
        }
    }
}

/// The plant she has already, on a floor or still in its box, is what
/// the stage puts up on her shelf: moved there, out of its box, facing as
/// it faced, and no second plant.
#[test]
fn the_stage_puts_up_the_plant_she_has() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        for boxed in [false, true] {
            let at = format!("graphics={graphics} boxed {boxed}");
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            assert!(guest.ledger.home.add(room::Prop {
                boxed: boxed.into(),
                ..room::Prop::new(Furniture::Plant, Nook::Playlist, 300, Facing::Left)
            }));
            let (_, note) = stage_shelf(&mut guest, &real, &view);
            assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
            let plants: Vec<_> = guest
                .ledger
                .home
                .props
                .iter()
                .filter(|p| p.item == Furniture::Plant)
                .map(|p| (p.strip, p.facing, p.boxed()))
                .collect();
            assert_eq!(
                plants,
                [(room::Strip::Shelf(Nook::List), Facing::Left, false)],
                "{at}"
            );
            let on: Vec<_> = up_on(&guest, Nook::List)
                .iter()
                .map(|s| (s.item, s.facing, s.boxed()))
                .collect();
            assert_eq!(on, [(Furniture::Plant, Facing::Left, false)], "{at}");
        }
    }
}

/// Cued on a frame with no room for her, the stage's shelf waits with the
/// cue for the first frame with room, and goes up there.
#[test]
fn the_stage_keeps_its_shelf_for_a_frame_with_room() {
    let (small, small_view) = rooms_frame(50, 15);
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.cue(Scene::Shelf);
        paint(&mut guest, &small, &small_view, 0);
        assert!(!guest.present(), "{at}: nowhere to stand");
        assert!(guest.ledger.home.structures.is_empty(), "{at}: not yet");
        // She comes in on the idle gate, at the full size.
        let mut now = 0;
        while !guest.present() && now < 60_000 {
            now += 500;
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
        }
        assert!(guest.present(), "{at}: she comes in");
        let note = guest.cue_note().cloned();
        assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
        assert_eq!(guest.ledger.home.structures.len(), 1, "{at}");
        assert_eq!(up_on(&guest, Nook::List).len(), 1, "{at}");
    }
}
