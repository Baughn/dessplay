//! Her shelf in the frame (phase 6a, step 2b): written once a paint,
//! before anything reads the frame; kept as her home's bottom layer, or
//! undone by the arm that ends the frame without her home; hidden whole
//! where the frame can't hold it; raining out with what she showed. It's
//! put up by hand here (nothing makes one before step 5c), and it's no
//! strip yet (step 3), so nothing stands on it. Both modes; visiting and
//! with her home standing empty.

use super::*;
use crate::ui::houseguest::structures::{Shelf, Structure, Written};

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

/// Her goodbye, frame by frame (both modes): before the rain her shelf
/// shows as it was frozen; once the rain has settled, the real frame; and
/// no cell of it the rain has settled shows it again (the goodbye's frame
/// is never written into).
#[test]
fn her_shelf_rains_out_with_her_goodbye() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view) = rooms_frame(100, 30);
        let mut guest = shelved(7, sat(14, 0), graphics);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let frame = next(&mut guest, &real, &view, &mut now, 250);
        shows(&guest, &frame, &real, &at);
        guest.activity(now);
        assert!(matches!(guest.state, State::Leaving(_)), "{at}: leaving");
        let start = now;
        let mut settled = [false; 10];
        let mut rained = [false; 10];
        while now < start + dissolve::DURATION_MS {
            let frame = paint(&mut guest, &real, &view, now);
            let t = now - start;
            let State::Leaving(leaving) = &guest.state else {
                panic!("{at}: leaving at {t}");
            };
            for (i, x) in RUN.enumerate() {
                let shown = frame[(x, ROW)].symbol().to_owned();
                if t < dissolve::RAIN_FROM_MS {
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
            now += dissolve::FRAME_MS;
        }
        assert_eq!(rained, [true; 10], "{at}: each cell rained");
        assert_eq!(paint(&mut guest, &real, &view, now), real, "{at}: the end");
        gone_for_good(&mut guest, &real, &view, now, &at);
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
