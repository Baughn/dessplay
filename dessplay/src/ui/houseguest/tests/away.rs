//! Away (phase 5b D3, step 5b): on a school morning she goes out through
//! her door at 08:15, the door stands closed at its space by her door's
//! wall (the door batch), her home stands empty with the lamp off, and at
//! 12:45 she comes home out of it. Each in both drawing modes; the
//! screens quiet and text-dense (`home_screens`, or text scattered over
//! `rooms`).

use super::*;
use crate::ui::houseguest::art::PieceState;
use crate::ui::houseguest::osaka::Bubble;

/// Her home on [`home_screen`]: the sofa and TV in the Users pane, the
/// bed and lamp in the Playlist pane.
const HOME: [(Furniture, Nook, u16); 4] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Tv, Nook::Users, 800),
    (Furniture::Bed, Nook::Playlist, 300),
    (Furniture::Lamp, Nook::Playlist, 800),
];

/// Whether `bubble` says a line of `pool`.
fn says(bubble: Option<Bubble>, pool: mind::Pool) -> bool {
    matches!(bubble, Some(Bubble::Say(text)) if pool.lines.contains(&text))
}

/// Her empty home, if it stands.
fn empty_of(guest: &Guest) -> Option<&Empty> {
    match &guest.state {
        State::Away(empty) => Some(empty),
        _ => None,
    }
}

/// The cells of her box standing at `(x, y)` that `frame` changed.
fn box_changed(frame: &Buffer, real: &Buffer, (x, y): (i32, i32)) -> usize {
    (y - sprite::HEIGHT..=y)
        .flat_map(|cy| (x - sprite::WIDTH / 2..=x + sprite::WIDTH / 2).map(move |cx| (cx, cy)))
        .filter_map(|(cx, cy)| Some((u16::try_from(cx).ok()?, u16::try_from(cy).ok()?)))
        .filter(|&at| frame.cell(at) != real.cell(at))
        .count()
}

/// Her home with room in it, quiet and text-dense: [`home_screen`], and
/// it with text over the panes, where she climbs and stands (the panes
/// themselves are clear, so her pieces, a parcel and a cued use fit).
fn roomy_screens() -> [(&'static str, (Buffer, IdleView)); 2] {
    let (mut texty, view) = home_screen();
    for row in 0..8u16 {
        let line = format!("{row} so what did you think of it, honestly? it was fine");
        texty.set_string(row * 3 % 11, row, line, Style::new());
    }
    [("quiet", home_screen()), ("texty", (texty, view))]
}

/// Her closed door standing at `door`, as it's drawn while she's out
/// (her slippers before it, side-on in its wall; face-on, shut).
fn closed(door: door::DoorSpot) -> Front {
    Front::at(door, AWAY, art::Sky::Day, None, 0)
}

/// Whether `frame` shows her closed door at `door` in her empty home
/// (`empty`, just painted), strictly: none of its cells under any of her
/// pieces (that's a failure, not a skip); in ASCII, every glyph of it
/// drawn, over text too (only a wide glyph's cells, which it can't take,
/// are passed over; its wall's column over the wall's line), and at
/// least one; in line art, her door's image, closed, there, taking in no
/// piece, with cells of it drawn.
fn door_shows(empty: &Empty, frame: &Buffer, real: &Buffer, door: door::DoorSpot) -> bool {
    let covers: Vec<Rect> = empty.shown.iter().map(Shown::cover).collect();
    let front = closed(door);
    let (left, top, right, bottom) = front.bounds();
    let under_a_piece = (top..bottom - 1).any(|y| {
        (left..right).any(|x| {
            let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                return false;
            };
            covers.iter().any(|r| r.contains((x, y).into()))
        })
    });
    if under_a_piece {
        return false;
    }
    match &empty.image {
        Some(image) => {
            let drawn = (top..bottom).any(|y| (left..right).any(|x| differs(frame, real, (x, y))));
            image.figure.front().map(Front::spot) == Some(door)
                && image.figure.her().is_none()
                && image.with.is_empty()
                && drawn
        }
        None => {
            let mut seen = 0;
            let mut after_wide = false;
            for (x, y, glyph, _) in front.cells() {
                let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                    continue;
                };
                let Some(cell) = real.cell((x, y)) else {
                    continue;
                };
                let wide = std::mem::replace(&mut after_wide, cells::width(cell) > 1);
                if wide || cells::width(cell) > 1 {
                    continue;
                }
                seen += 1;
                if frame.cell((x, y)).map(|c| c.symbol().to_owned()) != Some(glyph.to_string()) {
                    return false;
                }
            }
            seen > 0
        }
    }
}

/// Where her door stands on `view` of `real` when it stands in its space
/// by her door's wall on `side` of `nook`'s strip: worked out from the
/// nook's rect and the side alone (`walls_of`), never from the code that
/// stands it there. The wall must have a space (asserted).
fn space_spot(real: &Buffer, view: &IdleView, nook: Nook, side: room::Side) -> (i32, i32) {
    let wall = walls_of(&view.nooks, real.area, &[])
        .into_iter()
        .find(|w| w.nook == nook && w.side == side)
        .expect("that wall");
    assert!(wall.space.is_some(), "{wall:?}: a space");
    wall.spot
}

/// From `from` on `real`, until her home stands empty; a few steps more
/// so a frame has painted it. Returns when.
fn until_away(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64) -> u64 {
    let mut now = from;
    paint(guest, real, view, now);
    while empty_of(guest).is_none() {
        // Her walk to her door included.
        assert!(now < from + 120_000, "her home never stood empty");
        shell_step(guest, real, view, &mut now, true);
    }
    shell_step(guest, real, view, &mut now, true);
    now
}

/// Seed 4's school morning on `real`: visiting from 08:10, out through
/// her door at 08:15. Returns her home standing empty, when, and where
/// her door stands.
fn out_to_school(real: &Buffer, view: &IdleView, graphics: bool) -> (Guest, u64, door::DoorSpot) {
    let mut guest = home_at(4, tue(8, 10), &HOME, graphics);
    let now = until_visiting(&mut guest, real, view, 0);
    let now = until_away(&mut guest, real, view, now);
    let door = guest.closed_door().expect("her door");
    (guest, now, door)
}

/// One school morning (Tuesday, fed), in skips of her clock: she leaves
/// at 08:15 through her door in place, saying so; her door stands closed
/// in its space by her door's wall (on [`home_screen`], Users' left, at
/// the screen's edge: drawn from the very frame she's out, with no tick
/// between; on the wordy screen, over the users listed there); her home
/// stands empty, the TV and the lamp off; and at 12:45 she comes home out
/// of that door saying she's home, with no hello (the day's mood goes
/// on).
#[test]
fn a_school_morning_out_through_her_door_and_home_again() {
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(4, tue(8, 10), &HOME, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let school = real_of(&guest, now, tue(8, 15));
            // Until her home stands empty: where and when she set off,
            // and what she said.
            let mut set_off: Option<(u64, (i32, i32), Option<Bubble>)> = None;
            let (mut spot, mut pieces) = (None, Vec::new());
            while empty_of(&guest).is_none() {
                assert!(now < school + 30_000, "{at}: never out");
                let next = now
                    + guest
                        .next_tick(now)
                        .map_or(1000, |d| d.as_millis() as u64)
                        .clamp(1, 1000);
                if guest.gone_out(next) {
                    // Out between ticks: the paint itself ends the visit,
                    // her door in that frame.
                    now = next;
                    let frame = paint(&mut guest, &real, &view, now);
                    let empty = empty_of(&guest)
                        .unwrap_or_else(|| panic!("{at}: a frame of her home without her door"));
                    let door = guest.closed_door().expect("her door");
                    assert!(door_shows(empty, &frame, &real, door), "{at}: no door");
                    break;
                }
                shell_step(&mut guest, &real, &view, &mut now, true);
                if let State::Visiting(visit) = &guest.state {
                    let osaka = &visit.osaka;
                    if set_off.is_none()
                        && osaka.decisions.last().map(|d| d.method) == Some("routine/away")
                    {
                        set_off = Some((now, (osaka.x, osaka.y), osaka.appearance(now).2));
                    }
                    spot = Some((osaka.x, osaka.y));
                    pieces = items(&visit.shown);
                }
            }
            let (left, from, said) = set_off.unwrap_or_else(|| panic!("{at}: no routine/away"));
            assert!(
                left >= school,
                "{at}: out at {left}, before 08:15 ({school})"
            );
            assert!(
                says(said, mind::OFF) || said == Some(Bubble::Say(osaka::LATE)),
                "{at}: {said:?}"
            );
            let door = guest.closed_door().expect("her door");
            let space = space_spot(&real, &view, Nook::Users, room::Side::Left);
            // She set off from where she was and walked to her door's
            // spot (door batch, step 4a): out through it there.
            assert_eq!(spot, Some(space), "{at}: out at her door's spot");
            assert_ne!(
                from, space,
                "{at}: she walked there (set off from {from:?})"
            );
            assert_eq!(door.spot(), space, "{at}: her door in its space");
            assert_eq!(
                door.set(),
                door::Set::Wall {
                    side: room::Side::Left,
                    wall: 0
                },
                "{at}"
            );
            // A while out: her door stands closed there, her things with
            // the lamp off and nothing on TV, and nothing else of hers.
            let out = now;
            while now < out + 30_000 {
                shell_step(&mut guest, &real, &view, &mut now, false);
                let frame = paint(&mut guest, &real, &view, now);
                let empty = empty_of(&guest).unwrap_or_else(|| panic!("{at}: still out"));
                assert_eq!(guest.closed_door(), Some(door), "{at}: her door stays put");
                assert!(
                    door_shows(empty, &frame, &real, door),
                    "{at}: her door shows"
                );
                assert_eq!(empty.looks.state(Furniture::Lamp), PieceState::LampOff);
                assert_eq!(empty.looks.tv, None, "{at}");
                let standing = items(&empty.shown);
                assert!(
                    pieces.iter().all(|item| standing.contains(item)) && !pieces.is_empty(),
                    "{at}: her things stand: {pieces:?} {standing:?}"
                );
            }
            // On to 12:45: out of the door she comes, never dropping in.
            guest.skip_clock(now);
            assert_eq!(
                guest.clock_label(now).as_deref(),
                Some("Tue 12:45 Afternoon"),
                "{at}"
            );
            let mood = visit_mood(&guest);
            let mut home = None;
            let back = now;
            while home.is_none() {
                assert!(now < back + 20_000, "{at}: never home");
                shell_step(&mut guest, &real, &view, &mut now, true);
                if empty_of(&guest).is_some() {
                    assert_eq!(guest.closed_door(), Some(door), "{at}");
                    continue;
                }
                let osaka = &visit_of(&guest).osaka;
                assert_eq!((osaka.x, osaka.y), space, "{at}: out of her door");
                // Her door never blinks away as she comes out of it.
                assert!(
                    osaka.door(now).is_some() || !osaka.hidden(now),
                    "{at}: her door gone at {now}"
                );
                let bubble = osaka.appearance(now).2;
                if says(bubble, mind::HOME) {
                    home = Some(now);
                }
            }
            let osaka = &visit_of(&guest).osaka;
            assert!(osaka.greeted(), "{at}: no hello, it's the same day");
            assert_eq!(osaka.mood(), mood, "{at}: the morning's mood goes on");
            assert!(osaka.decisions.is_empty(), "{at}: {:?}", osaka.decisions);
            let greeting = Bubble::Say(mood.greeting());
            let end = now + 20_000;
            while now < end {
                shell_step(&mut guest, &real, &view, &mut now, true);
                let osaka = &visit_of(&guest).osaka;
                assert_ne!(osaka.appearance(now).2, Some(greeting), "{at}: a hello");
            }
        }
    }
}

/// What she moved as she goes out rains out (A22): the frames the shell
/// paints (only those the tick says could change) show the rain falling,
/// and the last of them shows the real text again.
#[test]
fn what_she_moved_rains_out_as_she_goes() {
    let (real, view) = wordy_home_screen();
    // The first letters of the users pane's lines.
    let moved: Vec<(u16, u16)> = (9..12).map(|y| (2, y)).collect();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(4, tue(8, 14), &HOME, graphics);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        assert!(
            visit.layer.tear(&real, &view.protected, &moved),
            "{at}: torn off"
        );
        let mut last = paint(&mut guest, &real, &view, now);
        while empty_of(&guest).is_none() {
            assert!(now < 60_000, "{at}: never out");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let raining = empty_of(&guest).is_some() && !guest.fades.is_empty();
        assert!(raining, "{at}: what she moved goes as a rain");
        // As the shell has it: painted only when the tick says so.
        let out = now;
        let mut frames = std::collections::HashSet::new();
        while now < out + dissolve::DURATION_MS + 1000 {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            if guest.advance(now) {
                last = paint(&mut guest, &real, &view, now);
                let cells: Vec<String> = moved
                    .iter()
                    .map(|&c| {
                        last.cell(c)
                            .map_or(String::new(), |c| c.symbol().to_owned())
                    })
                    .collect();
                frames.insert(cells);
            }
        }
        assert!(frames.len() > 1, "{at}: the rain falls: {frames:?}");
        for &cell in &moved {
            assert_eq!(last.cell(cell), real.cell(cell), "{at}: {cell:?} back");
        }
    }
}

/// Her real pieces among `shown` (not what she made), in order.
fn items(shown: &[Shown]) -> Vec<Furniture> {
    let mut items: Vec<Furniture> = shown
        .iter()
        .filter(|s| s.scrap.is_none())
        .map(|s| s.item)
        .collect();
    items.sort_by_key(|&item| item as usize);
    items
}

/// Her mood as the day's (what a visit now would bring).
fn visit_mood(guest: &Guest) -> brain::Mood {
    let day = guest.day(0).map(|d| d.day).expect("fed");
    brain::Mood::of(brain::day_seed(guest.ledger.master_seed, day))
}

/// A visitor at school time finds her home standing empty, her door
/// closed and nobody there; a key rains it all out with no wave, the
/// lamp off to the last, her door drawn until the rain (never blinking
/// out first), and the frame is the real one again.
#[test]
fn a_visitor_at_school_time_sees_her_home_and_a_key_rains_it_out() {
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(4, tue(9, 0), &HOME, graphics);
            let mut now = 0;
            paint(&mut guest, &real, &view, now);
            while empty_of(&guest).is_none() {
                assert!(now < 30_000, "{at}: her home never shows");
                assert!(!guest.present(), "{at}");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let shown = now;
            let mut frame = real.clone();
            while now < shown + 10_000 {
                shell_step(&mut guest, &real, &view, &mut now, false);
                frame = paint(&mut guest, &real, &view, now);
                assert!(!guest.present(), "{at}: she isn't here");
            }
            let door = guest.closed_door().expect("her door");
            let empty = empty_of(&guest).expect("away");
            assert!(door_shows(empty, &frame, &real, door), "{at}");
            assert_eq!(empty.image.is_some(), graphics, "{at}: its image");
            guest.activity(now);
            let State::Leaving(leaving) = &guest.state else {
                panic!("{at}: her home goes");
            };
            assert!(
                leaving
                    .image
                    .as_ref()
                    .is_none_or(|i| i.figure.her().is_none()),
                "{at}: no wave"
            );
            assert_eq!(leaving.looks.state(Furniture::Lamp), PieceState::LampOff);
            // Her door is drawn until the rain, in its image in line art.
            let key = now;
            for t in [0, 16, 100, 300, 600, dissolve::RAIN_FROM_MS - 1] {
                guest.advance(key + t);
                let frame = paint(&mut guest, &real, &view, key + t);
                assert!(
                    box_changed(&frame, &real, door.spot()) > 0,
                    "{at}: her door blinked out {t} ms in"
                );
            }
            let end = run(&mut guest, &real, &view, now, now + dissolve::DURATION_MS);
            assert_eq!(end, real, "{at}");
            assert!(matches!(guest.state, State::Absent), "{at}");
        }
    }
}

/// Whatever takes her empty home off the screen while she's out — a
/// visitor's key, an overlay, a frame too small, visits switched off —
/// when it shows again, her door stands in its space as before (on the
/// wordy screen, drawn over the users listed there). A frame too small
/// doesn't spin the shell (it waits for the next quiet, as a visit does);
/// visits switched off take it at once with no rain; an overlay rains it
/// out.
#[test]
fn her_door_stays_in_its_space_whatever_takes_her_home_away() {
    let (w, h) = (50, 15);
    let small = rooms(w, h);
    let small_view = IdleView {
        nooks: nooks(w, h),
        ..view(bottom_strip(w, h))
    };
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let (mut guest, mut now, door) = out_to_school(&real, &view, graphics);
            let space = space_spot(&real, &view, Nook::Users, room::Side::Left);
            assert_eq!(door.spot(), space, "{name}: in its space");
            assert!(door.wall().is_some(), "{name}: in its wall");
            for how in ["key", "overlay", "too small", "visits off"] {
                let at = format!("{name} graphics={graphics} {how}");
                now += 100;
                guest.advance(now);
                match how {
                    "key" => guest.activity(now),
                    "overlay" => {
                        let covered = IdleView {
                            busy: Some(Busy::Overlay),
                            ..view.clone()
                        };
                        paint(&mut guest, &real, &covered, now);
                        let State::Leaving(leaving) = &guest.state else {
                            panic!("{at}: it rains out");
                        };
                        assert!(
                            leaving
                                .image
                                .as_ref()
                                .is_none_or(|i| i.figure.her().is_none()),
                            "{at}: no wave"
                        );
                        let end = now + dissolve::DURATION_MS + 500;
                        let frame = run(&mut guest, &real, &covered, now, end);
                        assert_eq!(frame, real, "{at}");
                        now = end;
                    }
                    "too small" => {
                        paint(&mut guest, &small, &small_view, now);
                        assert!(matches!(guest.state, State::Absent), "{at}");
                        // No busy loop: nothing until another quiet.
                        let wait = guest.next_tick(now).expect("a wake");
                        assert!(wait >= DELAY, "{at}: woken in {wait:?}");
                        for _ in 0..5 {
                            now += 1;
                            assert!(!guest.advance(now), "{at}: redrawn for nothing");
                            let frame = paint(&mut guest, &small, &small_view, now);
                            assert_eq!(frame, small, "{at}");
                        }
                    }
                    _ => {
                        let off = IdleView {
                            delay: None,
                            ..view.clone()
                        };
                        let frame = paint(&mut guest, &real, &off, now);
                        assert!(matches!(guest.state, State::Absent), "{at}");
                        assert_eq!(frame, real, "{at}: gone at once");
                        now += 1000;
                        paint(&mut guest, &real, &off, now);
                    }
                }
                assert!(empty_of(&guest).is_none(), "{at}: gone");
                now = until_away(&mut guest, &real, &view, now);
                let frame = paint(&mut guest, &real, &view, now);
                assert_eq!(guest.closed_door(), Some(door), "{at}: in its space again");
                let empty = empty_of(&guest).expect("away");
                assert!(door_shows(empty, &frame, &real, door), "{at}");
            }
        }
    }
}

/// A resize moves her door to its space on the new frame (on
/// [`wordy_rooms`], Users' left wall, saved on [`home_screen`]); a
/// protected strip over that space then moves it to the strict
/// fallback's spot (the nearest floor spot where her box meets no piece,
/// checked against the predicate written out again, `door_floor`), and
/// it stays there after, over the text as much as anywhere.
#[test]
fn a_resize_moves_her_door_to_its_space_or_the_nearest_clear_spot() {
    let (w, h) = (100, 30);
    let other = wordy_rooms(w, h);
    let other_view = IdleView {
        nooks: nooks(w, h),
        ..view(bottom_strip(w, h))
    };
    let space = space_spot(&other, &other_view, Nook::Users, room::Side::Left);
    let guard = Rect::new(51, 8, 6, 5);
    let mut guarded = other_view.clone();
    guarded.protected.push(guard);
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now, door) = out_to_school(&real, &view, graphics);
        now += 100;
        guest.advance(now);
        let frame = paint(&mut guest, &other, &other_view, now);
        let moved = guest.closed_door().expect("her door");
        assert_ne!(moved.spot(), door.spot(), "{at}: the screen changed");
        assert_eq!(moved.spot(), space, "{at}: in its space on the new frame");
        assert!(moved.wall().is_some(), "{at}");
        let empty = empty_of(&guest).expect("away");
        assert!(door_shows(empty, &frame, &other, moved), "{at}");
        // Its space protected: the strict fallback.
        now += 1000;
        guest.advance(now);
        let frame = paint(&mut guest, &other, &guarded, now);
        let fell = guest.closed_door().expect("her door");
        assert_eq!(
            fell.set(),
            door::Set::Floor(door::Fallback::Protected),
            "{at}"
        );
        let covers: Vec<Rect> = guest
            .ledger
            .home
            .laid_and_shifted(&guarded.nooks)
            .shown
            .iter()
            .map(Shown::cover)
            .collect();
        assert!(
            door_floor(
                &other,
                &guarded.protected,
                &covers,
                guarded.chat,
                fell.spot()
            ),
            "{at}: {fell:?}"
        );
        assert_eq!(
            Some(fell.spot()),
            nearest_door_floor(&other, &guarded.protected, &covers, guarded.chat, space),
            "{at}: the nearest clear spot"
        );
        let empty = empty_of(&guest).expect("away");
        assert!(door_shows(empty, &frame, &other, fell), "{at}");
        now += 1000;
        guest.advance(now);
        paint(&mut guest, &other, &guarded, now);
        assert_eq!(guest.closed_door(), Some(fell), "{at}: and stays");
    }
}

/// While a resident's client is in use, the focused pane is hers no
/// more even with her out: what of her home stands in it (her pieces,
/// her closed door) rains out at once, and comes back, her door where
/// it was, once the pane is left alone. The panes have text in them.
#[test]
fn a_residents_focused_pane_keeps_her_home_out_while_shes_away() {
    let (w, h) = (100, 30);
    let real = wordy_rooms(w, h);
    let panes = nooks(w, h);
    for graphics in [false, true] {
        let mut guest = resident_at(graphics);
        let quiet = resident_view(w, h, None);
        let mut now = until_away(&mut guest, &real, &quiet, 0);
        let door = guest.closed_door().expect("her door");
        let mut doors_pane = false;
        for &(nook, pane) in &panes {
            let at = format!("{nook:?} graphics={graphics}");
            let hers = |frame: &Buffer| {
                (pane.top()..pane.bottom())
                    .flat_map(|y| (pane.left()..pane.right()).map(move |x| (x, y)))
                    .filter(|&at| frame.cell(at) != real.cell(at))
                    .count()
            };
            let before = paint(&mut guest, &real, &quiet, now);
            let there = hers(&before);
            doors_pane |= osaka::box_meets(pane, door.spot());
            let focused = resident_view(w, h, Some(pane));
            let landed = now + 1;
            now = landed;
            guest.advance(now);
            let first = paint(&mut guest, &real, &focused, now);
            if there > 0 {
                assert!(hers(&first) > 0, "{at}: the rain starts at once");
            }
            while now < landed + dissolve::DURATION_MS + 1000 {
                now += 50;
                guest.advance(now);
                let frame = paint(&mut guest, &real, &focused, now);
                assert!(empty_of(&guest).is_some(), "{at}: a resident's home stays");
                if now >= landed + dissolve::DURATION_MS {
                    assert_eq!(hers(&frame), 0, "{at}: the pane is itself again");
                }
            }
            // Left alone, it's all back, her door where it was.
            now += 1000;
            guest.advance(now);
            let again = paint(&mut guest, &real, &quiet, now);
            assert_eq!(hers(&again), there, "{at}: back as it was");
            assert_eq!(guest.closed_door(), Some(door), "{at}");
        }
        assert!(doors_pane, "graphics={graphics}: her door stood in a pane");
    }
}

/// A resident's home at Tuesday 09:00, her sofa, bed and lamp in it.
fn resident_at(graphics: bool) -> Guest {
    let mut ledger = Ledger::new_at(6, tue(9, 0));
    for (item, nook, x) in [
        (Furniture::Sofa, Nook::Users, 300),
        (Furniture::Bed, Nook::Playlist, 300),
        (Furniture::Lamp, Nook::Playlist, 800),
    ] {
        assert!(
            ledger
                .home
                .add(room::Prop::new(item, nook, x, sprite::Facing::Right))
        );
    }
    let mut guest = Guest::restore(ledger);
    guest.set_date(date(2026, 6, 17));
    if graphics {
        guest.set_picker(kitty());
    }
    guest
}

/// A focused pane that takes in only part of her door (its drawn cells
/// in its wall's own column: it stands side-on in the wall) keeps all of
/// it away: no half a door outside the pane, and it never moves
/// meanwhile; left alone, it's back in its space.
#[test]
fn a_focused_pane_across_her_door_keeps_all_of_it_away() {
    let (w, h) = (100, 30);
    let real = wordy_rooms(w, h);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = resident_at(graphics);
        let quiet = resident_view(w, h, None);
        let mut now = until_away(&mut guest, &real, &quiet, 0);
        let door = guest.closed_door().expect("her door");
        let (_, wall) = door.wall().unwrap_or_else(|| panic!("{at}: in its space"));
        // Her door's drawn cells in its wall's own column.
        let drawn = &empty_of(&guest).expect("away").door.as_ref().unwrap().cells;
        let right: Vec<(u16, u16)> = drawn
            .iter()
            .copied()
            .filter(|&(x, _)| i32::from(x) == wall)
            .collect();
        assert!(!right.is_empty(), "{at}: drawn");
        let focus = right
            .iter()
            .map(|&(x, y)| Rect::new(x, y, 1, 1))
            .reduce(|a, b| a.union(b))
            .unwrap();
        let focused = resident_view(w, h, Some(focus));
        let covers: Vec<Rect> = empty_of(&guest)
            .expect("away")
            .shown
            .iter()
            .map(Shown::cover)
            .collect();
        let outside: Vec<(u16, u16)> = closed(door)
            .cells()
            .into_iter()
            .filter_map(|(x, y, ..)| Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?)))
            .filter(|&c| !focus.contains(c.into()) && !covers.iter().any(|r| r.contains(c.into())))
            .collect();
        assert!(!outside.is_empty(), "{at}: some of it outside");
        let end = now + dissolve::DURATION_MS + 500;
        while now < end {
            now += 50;
            guest.advance(now);
            let frame = paint(&mut guest, &real, &focused, now);
            for &cell in &outside {
                assert_eq!(frame.cell(cell), real.cell(cell), "{at}: {cell:?} at {now}");
            }
            assert_eq!(guest.closed_door(), Some(door), "{at}: it never moves");
        }
        now += 1000;
        guest.advance(now);
        let frame = paint(&mut guest, &real, &quiet, now);
        assert_eq!(guest.closed_door(), Some(door), "{at}: back in its space");
        let empty = empty_of(&guest).expect("away");
        assert!(door_shows(empty, &frame, &real, door), "{at}: drawn again");
    }
}

/// A client started in school hours: with a home, it stands empty once
/// the idle gate opens (and its first frame's projection, settling her
/// pieces, is saved); with none, there's nothing at all until 12:45, when
/// she comes in out of a door (never dropping in), home from school.
/// With no home, nothing wakes her before 12:45 (no spinning on the open
/// gate).
#[test]
fn a_cold_start_in_school_hours() {
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            for furnished in [true, false] {
                let at = format!("{name} furnished={furnished} graphics={graphics}");
                let pieces: &[(Furniture, Nook, u16)] = if furnished { &HOME } else { &[] };
                // Furnished, her door stands in Users' left space (worked
                // out from the nook alone), and she comes home out of it.
                let space = space_spot(&real, &view, Nook::Users, room::Side::Left);
                let mut guest = home_at(4, tue(12, 30), pieces, graphics);
                let mut now = 0;
                paint(&mut guest, &real, &view, now);
                // Her clock runs from the first tick.
                shell_step(&mut guest, &real, &view, &mut now, true);
                let home_time = real_of(&guest, now, tue(12, 45));
                let gate = DELAY.as_millis() as u64;
                while now < home_time {
                    let hint = guest.next_tick(now).map(|d| now + d.as_millis() as u64);
                    assert!(
                        hint.is_some_and(|t| t <= home_time),
                        "{at}: no wakeup for 12:45 ({hint:?})"
                    );
                    if !furnished && now > gate {
                        assert_eq!(hint, Some(home_time), "{at}: woken for nothing");
                    }
                    now += guest
                        .next_tick(now)
                        .map_or(1000, |d| d.as_millis() as u64)
                        .clamp(1, 1000);
                    let changed = guest.advance(now);
                    if furnished && empty_of(&guest).is_some_and(|e| e.size == (0, 0)) {
                        // Its first frame: projecting her home pins it.
                        let _ = guest.ledger_to_save();
                        paint(&mut guest, &real, &view, now);
                        assert!(guest.unsaved, "{at}: the projection is hers to save");
                        continue;
                    }
                    if !changed {
                        continue;
                    }
                    let frame = paint(&mut guest, &real, &view, now);
                    if now < home_time {
                        if furnished && now > gate + 1000 {
                            let empty = empty_of(&guest).expect("her home, empty");
                            let door = guest.closed_door().expect("her door");
                            assert!(door_shows(empty, &frame, &real, door), "{at}");
                            assert_eq!(door.spot(), space, "{at}: in its space");
                            assert!(
                                matches!(
                                    door.set(),
                                    door::Set::Wall {
                                        side: room::Side::Left,
                                        ..
                                    }
                                ),
                                "{at}: {door:?}"
                            );
                        } else if !furnished {
                            assert!(matches!(guest.state, State::Absent), "{at}");
                            assert_eq!(frame, real, "{at}: nothing at all");
                        }
                    }
                }
                let mut home = false;
                let mut first = None;
                while !home {
                    assert!(now < home_time + 20_000, "{at}: never home");
                    shell_step(&mut guest, &real, &view, &mut now, true);
                    let osaka = &visit_of(&guest).osaka;
                    first.get_or_insert((osaka.x, osaka.y));
                    assert!(
                        osaka.door(now).is_some() || !osaka.hidden(now),
                        "{at}: her door gone at {now}"
                    );
                    assert_ne!(osaka.act_name(), "Fall", "{at}: no drop from the sky");
                    home = says(osaka.appearance(now).2, mind::HOME);
                }
                if furnished {
                    assert_eq!(first, Some(space), "{at}: home out of her door");
                }
                assert_eq!(guest.ledger.visits, 2, "{at}: her coming home is a visit");
            }
        }
    }
}

/// A chat line while she's out changes nothing, with a home shown or
/// none: at 12:45 she still comes home out of her door (A6; Away and
/// Absent alike).
#[test]
fn a_chat_line_while_shes_out_doesnt_stop_her_coming_home() {
    for (name, (real, mut view)) in home_screens() {
        for graphics in [false, true] {
            for furnished in [true, false] {
                let at = format!("{name} furnished={furnished} graphics={graphics}");
                let pieces: &[(Furniture, Nook, u16)] = if furnished { &HOME } else { &[] };
                let mut guest = home_at(4, tue(12, 40), pieces, graphics);
                let mut now = 0;
                paint(&mut guest, &real, &view, now);
                shell_step(&mut guest, &real, &view, &mut now, true);
                let home_time = real_of(&guest, now, tue(12, 45));
                let mut chatted = false;
                loop {
                    shell_step(&mut guest, &real, &view, &mut now, true);
                    if !chatted && now + 2000 >= home_time {
                        chatted = true;
                        view.chat_mark.synced += 1;
                        paint(&mut guest, &real, &view, now);
                    }
                    if now >= home_time {
                        break;
                    }
                }
                assert!(chatted, "{at}");
                assert!(
                    matches!(guest.state, State::Arriving(How::Return, _))
                        || matches!(&guest.state, State::Visiting(v) if v.osaka.act_name() == "Door"),
                    "{at}: she comes home out of her door"
                );
            }
        }
    }
}

/// Her coming home (decided as school ends) isn't called off by a key
/// press or a chat line arriving in the same frame (A6), for a visitor
/// or a resident: she comes in, and the next key ends a visitor's visit
/// as ever (a resident stays).
#[test]
fn her_coming_home_isnt_called_off_by_a_key_or_a_chat_line() {
    for (name, (real, quiet)) in home_screens() {
        for resident in [false, true] {
            for graphics in [false, true] {
                for (input, chat) in [(true, false), (false, true), (true, true)] {
                    let at = format!(
                        "{name} resident={resident} input={input} chat={chat} graphics={graphics}"
                    );
                    let mut view = IdleView {
                        resident,
                        ..quiet.clone()
                    };
                    let mut guest = home_at(4, tue(12, 44), &HOME, graphics);
                    let mut now = 0;
                    paint(&mut guest, &real, &view, now);
                    while empty_of(&guest).is_none() {
                        assert!(now < 30_000, "{at}: her home never shows");
                        shell_step(&mut guest, &real, &view, &mut now, true);
                    }
                    let home_time = real_of(&guest, now, tue(12, 45));
                    loop {
                        now += guest
                            .next_tick(now)
                            .map_or(1000, |d| d.as_millis() as u64)
                            .clamp(1, 1000);
                        assert!(now <= home_time, "{at}: no wakeup at 12:45");
                        guest.advance(now);
                        if matches!(guest.state, State::Arriving(How::Return, _)) {
                            break;
                        }
                        paint(&mut guest, &real, &view, now);
                    }
                    assert_eq!(now, home_time, "{at}");
                    if input {
                        guest.activity(now);
                    }
                    if chat {
                        view.chat_mark.synced += 1;
                    }
                    paint(&mut guest, &real, &view, now);
                    let osaka = &visit_of(&guest).osaka;
                    assert!(osaka.door(now).is_some(), "{at}: out of her door");
                    // The next key ends it, as for any visitor; a resident
                    // stays.
                    now += 500;
                    guest.advance(now);
                    guest.activity(now);
                    if resident {
                        assert!(matches!(guest.state, State::Visiting(_)), "{at}");
                    } else {
                        assert!(
                            matches!(guest.state, State::Leaving(_) | State::Absent),
                            "{at}"
                        );
                    }
                }
            }
        }
    }
}

/// A visitor at the keys as school ends: no coming home out of her door
/// (the client isn't idle), and later an ordinary arrival once the gate
/// opens again.
#[test]
fn busy_as_school_ends_she_comes_in_later_on_the_idle_gate() {
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(4, tue(12, 44), &HOME, graphics);
            let mut now = until_away(&mut guest, &real, &view, 0);
            let home_time = real_of(&guest, now, tue(12, 45));
            let visits = guest.ledger.visits;
            // A key a moment before 12:45.
            now = home_time - 1000;
            guest.advance(now);
            guest.activity(now);
            let key = now;
            let mut arrived = None;
            while arrived.is_none() {
                assert!(now < key + 60_000, "{at}: she never came");
                now += guest
                    .next_tick(now)
                    .map_or(1000, |d| d.as_millis() as u64)
                    .clamp(1, 1000);
                guest.advance(now);
                assert!(
                    !matches!(guest.state, State::Arriving(How::Return, _)),
                    "{at}: home out of her door with a visitor at the keys"
                );
                if matches!(guest.state, State::Arriving(How::Idle, _)) {
                    arrived = Some(now);
                }
                paint(&mut guest, &real, &view, now);
            }
            assert!(
                arrived.is_some_and(|t| t >= key + DELAY.as_millis() as u64),
                "{at}: on the idle gate"
            );
            assert_eq!(guest.ledger.visits, visits + 1, "{at}");
            assert!(guest.out.is_none(), "{at}: she's not out");
        }
    }
}

/// Nothing is delivered while she's out: a parcel waits for her, and
/// comes once she's said she's home (its line after hers).
#[test]
fn a_parcel_waits_for_her_to_come_home() {
    for (name, (real, view)) in roomy_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(4, tue(12, 40), &HOME, graphics);
            guest.ledger.ordered = Some(Furniture::Desk);
            let mut now = 0;
            paint(&mut guest, &real, &view, now);
            // Her clock runs from the first tick.
            shell_step(&mut guest, &real, &view, &mut now, true);
            let home_time = real_of(&guest, now, tue(12, 45));
            while now < home_time {
                shell_step(&mut guest, &real, &view, &mut now, true);
                assert!(
                    !guest.ledger.home.owns(Furniture::Desk),
                    "{at}: delivered while out"
                );
                assert_eq!(guest.ledger.ordered, Some(Furniture::Desk), "{at}");
            }
            let (mut home, mut parcel) = (None, None);
            while parcel.is_none() {
                assert!(now < home_time + 30_000, "{at}: no parcel");
                shell_step(&mut guest, &real, &view, &mut now, true);
                let State::Visiting(visit) = &guest.state else {
                    continue;
                };
                match visit.osaka.appearance(now).2 {
                    Some(Bubble::Say(text)) if mind::HOME.lines.contains(&text) => {
                        home.get_or_insert((now, text));
                    }
                    Some(Bubble::Say(PARCEL)) => parcel = Some(now),
                    _ => {}
                }
            }
            let (said, line) = home.unwrap_or_else(|| panic!("{at}: never said she's home"));
            let parcel = parcel.unwrap_or_default();
            assert!(
                parcel >= said + osaka::speech_ms(line),
                "{at}: the parcel at {parcel}, over her {line:?} at {said}"
            );
            assert!(guest.ledger.home.owns(Furniture::Desk), "{at}");
        }
    }
}

/// Out of sight between her doors on an ordinary trip (no routine, no
/// work), a parcel waits until she's in sight again: she'd say so.
#[test]
fn a_parcel_waits_while_shes_between_her_doors() {
    for (name, (real, view)) in roomy_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            // A Saturday afternoon: no routine to send her anywhere.
            let mut guest = home_at(
                4,
                routine::GameTime {
                    day: 5,
                    h: 15,
                    m: 0,
                },
                &HOME,
                graphics,
            );
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            let to = (visit.osaka.x, visit.osaka.y);
            visit.osaka.through_door(to, now);
            while !visit_of(&guest).osaka.hidden(now) {
                assert!(now < 120_000, "{at}: never through");
                now += 50;
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
            }
            guest.ledger.ordered = Some(Furniture::Desk);
            guest.ledger.bought_on = 0;
            assert!(guest.ledger.visits > 0, "{at}");
            let mut hidden = 0;
            while visit_of(&guest).osaka.hidden(now) {
                assert!(now < 240_000, "{at}: never back in sight");
                paint(&mut guest, &real, &view, now);
                hidden += 1;
                assert!(
                    !guest.ledger.home.owns(Furniture::Desk),
                    "{at}: delivered out of sight"
                );
                now += 50;
                guest.advance(now);
            }
            assert!(hidden > 0, "{at}");
            let seen = now;
            while !guest.ledger.home.owns(Furniture::Desk) {
                assert!(now < seen + 10_000, "{at}: never delivered");
                paint(&mut guest, &real, &view, now);
                now += 50;
                guest.advance(now);
            }
        }
    }
}

/// The cat in his bed while she's out is the coming visit's: he's there
/// or not as she comes home, and stays so.
#[test]
fn the_cat_doesnt_change_as_she_comes_home() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let mut seen = [false; 2];
        for seed in (0..8u64).map(|i| i.wrapping_mul(0x9E37_79B9_7F4A_7C15)) {
            let at = format!("seed {seed} graphics={graphics}");
            let pieces = [
                (Furniture::Sofa, Nook::Users, 300),
                (Furniture::CatBed, Nook::Playlist, 500),
            ];
            let mut guest = home_at(seed, tue(12, 44), &pieces, graphics);
            let mut now = until_away(&mut guest, &real, &view, 0);
            let away = empty_of(&guest)
                .map(|e| e.looks.state(Furniture::CatBed))
                .expect("away");
            while !matches!(guest.state, State::Visiting(_)) {
                assert!(now < 120_000, "{at}: never home");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            shell_step(&mut guest, &real, &view, &mut now, true);
            let home = visit_of(&guest).looks.state(Furniture::CatBed);
            assert_eq!(away, home, "{at}");
            seen[usize::from(home == PieceState::Cat)] = true;
        }
        assert_eq!(
            seen,
            [true, true],
            "graphics={graphics}: with and without him"
        );
    }
}

/// Off to work as school begins (the stage's doing: her job is open only
/// on days off), she goes on to school through her door: the visit ends
/// once it has closed behind her, her home stands empty, and she never
/// comes home with her shopping.
#[test]
fn out_at_work_as_school_begins_she_goes_on_to_school() {
    use super::sprite::Pose;
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(4, tue(8, 14), &HOME, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let school = real_of(&guest, now, tue(8, 15));
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            let (chances, terrain) = (visit.chances.clone(), visit.terrain.clone());
            visit.osaka.go_to_work(&terrain, &chances, now, &mut Rng(1));
            while empty_of(&guest).is_none() {
                // Her walk to her door included.
                assert!(now < school + 60_000, "{at}: still out at work past 08:15");
                shell_step(&mut guest, &real, &view, &mut now, true);
                if let State::Visiting(visit) = &guest.state {
                    let (pose, ..) = visit.osaka.appearance(now);
                    assert!(!matches!(pose, Pose::Carry(_)), "{at}: home with leeks");
                }
            }
            assert!(now >= school, "{at}: gone before 08:15");
        }
    }
}

/// Going out by her routine, rained out of a resident's newly focused
/// pane: she's through her door already, out for good, and nothing is
/// drawn for it (no spot elsewhere, no shift).
#[test]
fn rained_out_on_her_way_to_school_she_is_simply_gone() {
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(4, tue(8, 14), &HOME, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let school = real_of(&guest, now, tue(8, 15));
            // Until she goes through her door (having walked to it), in
            // sight still.
            loop {
                assert!(now < school + 60_000, "{at}: never set off");
                shell_step(&mut guest, &real, &view, &mut now, true);
                let State::Visiting(visit) = &guest.state else {
                    continue;
                };
                let osaka = &visit.osaka;
                if osaka.act_name() == "Door" && !osaka.hidden(now) {
                    break;
                }
            }
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            let (x, y) = (visit.osaka.x, visit.osaka.y);
            let focus = Rect::new((x - 3).max(0) as u16, (y - 6).max(0) as u16, 7, 7);
            let mut rng = Rng(77);
            let terrain = visit.terrain.clone();
            assert!(visit.osaka.evict(focus, &terrain, None, now, &mut rng));
            assert_eq!(rng.0, Rng(77).0, "{at}: nothing drawn");
            assert!(visit.osaka.gone_out(now).is_some(), "{at}: out at once");
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            let door = guest.closed_door().expect("her door");
            let space = space_spot(&real, &view, Nook::Users, room::Side::Left);
            assert_eq!(door.spot(), space, "{at}: in its space");
            assert_eq!((x, y), space, "{at}: she went through it at its spot");
        }
    }
}

/// Set down by the stage as she sets off, she isn't leaving any more
/// (what she was up to is forgotten); her routine sends her out again
/// at her next decision.
#[test]
fn placed_as_she_sets_off_she_isnt_leaving() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(4, tue(8, 14), &HOME, graphics);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        while visit_of(&guest).osaka.leaving().is_none() {
            assert!(now < 60_000, "{at}: never set off");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        let (x, y) = (visit.osaka.x, visit.osaka.y);
        visit.osaka.place(x, y, now);
        assert_eq!(visit.osaka.leaving(), None, "{at}");
        assert_eq!(visit.osaka.act_name(), "Stand", "{at}");
    }
}

/// Her breakfast cut short by school (on her way to the fridge, or at
/// it): "Late, late, late!" as she goes. Anything else cut short (her
/// sofa): one of her ordinary lines for going.
#[test]
fn late_for_school_only_when_it_cuts_her_breakfast() {
    let pieces = [
        (Furniture::Fridge, Nook::Users, 700),
        (Furniture::Sofa, Nook::Playlist, 300),
    ];
    for (name, (real, view)) in roomy_screens() {
        for graphics in [false, true] {
            for (scene, what, late) in [
                (Scene::Snack, room::Use::Snack, true),
                (Scene::Lounge, room::Use::Lounge, false),
            ] {
                let at = format!("{name} {scene:?} graphics={graphics}");
                let mut guest = home_at(4, tue(8, 13), &pieces, graphics);
                guest.cue(scene);
                let mut now = until_visiting(&mut guest, &real, &view, 0);
                while !visit_of(&guest)
                    .osaka
                    .seat()
                    .is_some_and(|seat| seat.what == what)
                {
                    assert!(now < 60_000, "{at}: never at it");
                    shell_step(&mut guest, &real, &view, &mut now, true);
                }
                guest.skip_clock(now);
                let mut said = None;
                while said.is_none() {
                    assert!(now < 120_000, "{at}: never off");
                    shell_step(&mut guest, &real, &view, &mut now, true);
                    let State::Visiting(visit) = &guest.state else {
                        panic!("{at}: gone without a word");
                    };
                    let osaka = &visit.osaka;
                    if osaka.decisions.last().map(|d| d.method) == Some("routine/away") {
                        said = Some(osaka.appearance(now).2);
                    }
                }
                let said = said.flatten();
                if late {
                    assert_eq!(said, Some(Bubble::Say(osaka::LATE)), "{at}");
                } else {
                    assert!(says(said, mind::OFF), "{at}: {said:?}");
                }
            }
        }
    }
}

/// The stage's cue to arrive, with her home standing empty at school
/// time: an ordinary arrival (she's no longer out), and at her first
/// decision her routine sends her back out through her door.
#[test]
fn cued_in_at_school_time_her_routine_sends_her_out_again() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(4, tue(9, 0), &HOME, graphics);
        let mut now = until_away(&mut guest, &real, &view, 0);
        let visits = guest.ledger.visits;
        guest.cue(Scene::Arrive);
        assert!(matches!(guest.state, State::Arriving(How::Idle, _)), "{at}");
        now += 1;
        paint(&mut guest, &real, &view, now);
        assert!(matches!(guest.state, State::Visiting(_)), "{at}");
        assert_eq!(guest.ledger.visits, visits + 1, "{at}");
        assert!(guest.out.is_none(), "{at}: here, she isn't out");
        while visit_of(&guest).osaka.decisions.is_empty() {
            assert!(now < 120_000, "{at}: never decided");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let first = visit_of(&guest).osaka.decisions[0].method;
        assert_eq!(first, "routine/away", "{at}");
        while empty_of(&guest).is_none() {
            assert!(now < 180_000, "{at}: never out again");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        assert!(guest.closed_door().is_some(), "{at}");
    }
}

/// An errand while she's out (the accordion nudged): she comes for it,
/// and her closed door rains out rather than vanishing.
#[test]
fn an_errand_from_her_empty_home_rains_her_door_out() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now, door) = out_to_school(&real, &view, graphics);
        now += 100;
        guest.advance(now);
        let accordion = Rect::new(10, 8, 20, 1);
        assert!(guest.send(&real, &view, accordion, now), "{at}: sent");
        let State::Visiting(_) = &guest.state else {
            panic!("{at}: visiting");
        };
        let raining = closed(door).cells().into_iter().any(|(x, y, ..)| {
            let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                return false;
            };
            guest.fades.iter().any(|fade| fade.painting(x, y))
        });
        assert!(raining, "{at}: her door rains out");
    }
}

/// A start of her clock where she goes out or comes home (school's start
/// or end, give or take three minutes, on a school day), where she goes
/// to bed (her bedtime, give or take three minutes, any night), or
/// anywhere in her week.
fn somewhen() -> impl Strategy<Value = routine::GameTime> {
    let near = |h: u16, m: u16| {
        (1u64..5, 0u16..7).prop_map(move |(day, d)| {
            let minute = h * 60 + m - 3 + d;
            routine::GameTime {
                day,
                h: minute / 60,
                m: minute % 60,
            }
        })
    };
    // 22:30 the night before school (Sunday to Thursday), else 23:30.
    let bedtime = (0u64..7, 0u16..7).prop_map(|(day, d)| {
        let bed = if matches!(day, 4 | 5) {
            23 * 60 + 30
        } else {
            22 * 60 + 30
        };
        let minute = bed - 3 + d;
        routine::GameTime {
            day,
            h: minute / 60,
            m: minute % 60,
        }
    });
    prop_oneof![
        near(8, 15),
        near(12, 45),
        bedtime,
        (1u64..8, 0u16..1440).prop_map(|(day, minute)| routine::GameTime {
            day,
            h: minute / 60,
            m: minute % 60,
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(16)))]

    /// [`long_visits_never_touch_what_is_protected`] over her week, as
    /// her clock runs: she goes out and comes home through her door, her
    /// home stands empty between; protected cells are never painted, her
    /// door's image never hides text, and a key rains it all back to the
    /// real frame. Some sizes are too small for her.
    #[test]
    fn her_days_never_touch_what_is_protected(
        seed in any::<u64>(),
        start in somewhen(),
        graphics in any::<bool>(),
        sizes in proptest::collection::vec((48u16..130, 14u16..45), 1..3),
        text in proptest::collection::vec((0u16..60, 0u16..18, "[a-z漢─│ ]{1,6}"), 0..20),
        skips in proptest::collection::vec((0u16..60, 0u16..18), 0..6),
        chats in proptest::collection::vec(0u64..120_000, 0..3),
        protect in (0u16..40, 0u16..10, 1u16..20, 1u16..6),
        owned in proptest::collection::vec((0usize..4, 0usize..3, 0u16..=1000, any::<bool>()), 0..4),
    ) {
        let owned: Vec<(Furniture, usize, u16, bool)> = owned
            .into_iter()
            .map(|(item, at, along, left)| (Furniture::ALL[item], at, along, left))
            .collect();
        her_days(
            seed,
            start,
            graphics,
            rooms_frame,
            &sizes,
            &text,
            &skips,
            &chats,
            protect,
            &owned,
            Run::default(),
        )?;
    }

    /// [`her_days_never_touch_what_is_protected`] with the chat pane
    /// apart from her nooks ([`chat_apart`]: her pieces on the two right
    /// panes), so the chat rule's properties can be added here.
    #[test]
    fn her_days_with_the_chat_apart_never_touch_what_is_protected(
        seed in any::<u64>(),
        start in somewhen(),
        graphics in any::<bool>(),
        sizes in proptest::collection::vec((48u16..130, 14u16..45), 1..3),
        // Over the whole screen (clipped by `scatter` and `long_visit_of`),
        // so it reaches her nooks on the right as often as the chat.
        text in proptest::collection::vec((0u16..130, 0u16..45, "[a-z漢─│ ]{1,6}"), 0..20),
        skips in proptest::collection::vec((0u16..130, 0u16..45), 0..6),
        chats in proptest::collection::vec(0u64..120_000, 0..3),
        protect in (0u16..130, 0u16..45, 1u16..20, 1u16..6),
        // Users or Playlist, the nooks of `chat_apart`.
        owned in proptest::collection::vec((0usize..4, 1usize..3, 0u16..=1000, any::<bool>()), 0..4),
    ) {
        let owned: Vec<(Furniture, usize, u16, bool)> = owned
            .into_iter()
            .map(|(item, at, along, left)| (Furniture::ALL[item], at, along, left))
            .collect();
        her_days(
            seed,
            start,
            graphics,
            chat_apart,
            &sizes,
            &text,
            &skips,
            &chats,
            protect,
            &owned,
            Run {
                out_every: Some(1000),
                ..Run::default()
            },
        )?;
    }
}

/// [`her_days_with_the_chat_apart_never_touch_what_is_protected`]'s deep
/// draw pinned (door batch, step 6d): her door open at the right wall of
/// a 79-wide frame, the frame resized to 97. Before the fix the open
/// door kept the 79-wide frame's spot (wall 78), on her sofa laid at the
/// wider frame's wall.
#[test]
fn a_resize_while_her_door_is_open_never_leaves_it_on_her_sofa() {
    for graphics in [false, true] {
        her_days(
            2473510391964127931,
            routine::GameTime {
                day: 5,
                h: 9,
                m: 59,
            },
            graphics,
            chat_apart,
            &[(79, 18), (97, 18)],
            &[],
            &[],
            &[],
            (0, 0, 1, 1),
            &[(Furniture::Sofa, 1, 518, false)],
            Run {
                out_every: Some(1000),
                ..Run::default()
            },
        )
        .unwrap_or_else(|e| panic!("graphics {graphics}: {e}"));
    }
}

/// The stage's school scene (door batch D10) with the frame resized
/// (73 to 90 wide, her bed and sofa laid by the wider frame's right
/// wall) as her door stands open: as she goes out by it, and as she
/// comes back in (door batch, step 6d). Each first frame at the new size
/// is checked by [`long_visit_of`] as every frame is: her door drawn at
/// the act's spot (she and it move together), on none of her pieces.
#[test]
fn a_resize_as_her_stage_school_door_stands_open_moves_her_with_it() {
    let owned = [
        (Furniture::Bed, 1, 120, false),
        (Furniture::Sofa, 1, 116, true),
    ];
    let whens: [(&str, SizeWhen); 2] = [("going out", going_out), ("coming in", coming_in)];
    for (what, when) in whens {
        for graphics in [false, true] {
            let mut guest = Guest::restore(Ledger::new_at(4, tue(15, 0)));
            guest.set_date(date(2026, 6, 17));
            long_visit_of(
                guest,
                graphics,
                rooms_frame,
                &[(73, 20), (90, 18)],
                &[],
                &[],
                &[],
                (0, 0, 1, 1),
                &owned,
                Run {
                    cue: Some(Scene::School),
                    out_every: Some(1000),
                    next_size_when: Some(when),
                },
                120_000,
            )
            .unwrap_or_else(|e| panic!("{what}, graphics {graphics}: {e}"));
        }
    }
}

/// One of her days' runs: from `start` by her clock, mid-June, 120 s
/// over `sizes` of `frame`, run as `how` says.
#[allow(clippy::too_many_arguments)]
fn her_days(
    seed: u64,
    start: routine::GameTime,
    graphics: bool,
    frame: fn(u16, u16) -> (Buffer, IdleView),
    sizes: &[(u16, u16)],
    text: &[(u16, u16, String)],
    skips: &[(u16, u16)],
    chats: &[u64],
    protect: (u16, u16, u16, u16),
    owned: &[(Furniture, usize, u16, bool)],
    how: Run,
) -> Result<(), TestCaseError> {
    let mut guest = Guest::restore(Ledger::new_at(seed, start));
    guest.set_date(date(2026, 6, 17));
    long_visit_of(
        guest, graphics, frame, sizes, text, skips, chats, protect, owned, how, 120_000,
    )
    .map(|_| ())
}

/// [`her_days_never_touch_what_is_protected`]'s case where she was pulling
/// text as school began: the text she moved rains out in her empty home
/// (its holes showing a moment), which is no text hidden by her.
#[test]
fn her_days_case_pulling_text_as_school_begins() {
    let mut guest = Guest::restore(Ledger::new_at(12_452_213_708_426_597_514, tue(8, 12)));
    guest.set_date(date(2026, 6, 17));
    let text = [(28, 11, "漢漢a".to_owned()), (0, 0, "漢".to_owned())];
    let owned = [(Furniture::ALL[1], 0, 0, false)];
    long_visit_of(
        guest,
        true,
        rooms_frame,
        &[(64, 33)],
        &text,
        &[(53, 0)],
        &[],
        (0, 0, 1, 1),
        &owned,
        Run::default(),
        120_000,
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

/// [`her_days_never_touch_what_is_protected`]'s case from before her
/// door was placed from her home (the door batch, step 3): on Tuesday at
/// 08:12, 62×21, her bed on List, she went out of it as school began and
/// her closed door stood on her bed. Both modes.
#[test]
fn her_days_case_out_of_her_bed_as_school_begins() {
    for graphics in [false, true] {
        let mut guest = Guest::restore(Ledger::new_at(61_518_700_694_217, tue(8, 12)));
        guest.set_date(date(2026, 6, 17));
        let owned = [(Furniture::ALL[2], 0, 369, false)];
        assert_eq!(owned[0].0, Furniture::Bed);
        let visited = long_visit_of(
            guest,
            graphics,
            rooms_frame,
            &[(62, 21)],
            &[],
            &[(28, 13)],
            &[],
            (0, 0, 1, 1),
            &owned,
            Run::default(),
            120_000,
        )
        .unwrap_or_else(|e| panic!("graphics={graphics}: {e}"));
        assert!(visited.away > 0, "graphics={graphics}: {visited:?}");
    }
}

/// Whether a rain of hers paints any cell of her closed door at `door`.
fn door_raining(guest: &Guest, door: door::DoorSpot) -> bool {
    closed(door).cells().into_iter().any(|(x, y, ..)| {
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        guest.fades.iter().any(|fade| fade.painting(x, y))
    })
}

/// The stage's cue to arrive from her empty home, called off before the
/// paint she'd come in on: what calls it off takes her empty home as it
/// takes it with no arrival under way. A resident's key and a chat line
/// leave it standing (she's still out, it drawn on the next frame); a
/// visitor's key rains it out. It never just vanishes.
#[test]
fn a_cued_arrival_called_off_leaves_her_empty_home_as_the_cause_would() {
    let (real, quiet) = home_screen();
    for resident in [false, true] {
        for chat in [false, true] {
            for graphics in [false, true] {
                let at = format!("resident={resident} chat={chat} graphics={graphics}");
                let mut view = IdleView {
                    resident,
                    ..quiet.clone()
                };
                let mut guest = home_at(4, tue(9, 0), &HOME, graphics);
                let mut now = until_away(&mut guest, &real, &view, 0);
                assert!(
                    empty_of(&guest).is_some_and(|empty| !empty.painted.is_empty()),
                    "{at}: her home shown"
                );
                guest.cue(Scene::Arrive);
                assert!(
                    matches!(guest.state, State::Arriving(How::Idle, Some(_))),
                    "{at}: cued in from her home"
                );
                now += 1;
                if chat {
                    view.chat_mark.synced += 1;
                } else {
                    guest.activity(now);
                }
                paint(&mut guest, &real, &view, now);
                if resident || chat {
                    assert!(
                        empty_of(&guest).is_some_and(|empty| !empty.painted.is_empty()),
                        "{at}: her home stands on, drawn"
                    );
                    assert!(guest.closed_door().is_some(), "{at}: her door kept");
                } else {
                    assert!(
                        matches!(guest.state, State::Leaving(_)),
                        "{at}: her home rains out"
                    );
                }
            }
        }
    }
}

/// An errand due on the paint she'd come in on from her empty home (cued
/// in, or home from school as it ends): her closed door, as the home she
/// carries in has it, rains out rather than vanishing.
#[test]
fn an_errand_as_she_comes_in_from_her_empty_home_rains_her_door_out() {
    let (real, view) = home_screen();
    for school_ends in [false, true] {
        for graphics in [false, true] {
            let at = format!("school_ends={school_ends} graphics={graphics}");
            let start = if school_ends { tue(12, 44) } else { tue(9, 0) };
            let mut guest = home_at(4, start, &HOME, graphics);
            let mut now = until_away(&mut guest, &real, &view, 0);
            let door = guest.closed_door().expect("her door");
            if school_ends {
                let home = real_of(&guest, now, tue(12, 45));
                assert!(now < home, "{at}: school ended already");
                now = home;
                guest.advance(now);
                assert!(
                    matches!(guest.state, State::Arriving(How::Return, Some(_))),
                    "{at}: home from school, from her home"
                );
            } else {
                guest.cue(Scene::Arrive);
                now += 1;
                assert!(
                    matches!(guest.state, State::Arriving(How::Idle, Some(_))),
                    "{at}: cued in from her home"
                );
            }
            let accordion = Rect::new(10, 8, 20, 1);
            assert!(guest.send(&real, &view, accordion, now), "{at}: sent");
            assert!(matches!(guest.state, State::Visiting(_)), "{at}: visiting");
            assert!(door_raining(&guest, door), "{at}: her door rains out");
        }
    }
}

/// School ends with the client busy, her home standing empty: she
/// doesn't come home out of her door (A6), and her home rains out rather
/// than vanishing (step 1's hand-off).
#[test]
fn school_ending_on_a_busy_client_rains_her_empty_home_out() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(4, tue(12, 44), &HOME, graphics);
        let now = until_away(&mut guest, &real, &view, 0);
        let home = real_of(&guest, now, tue(12, 45));
        assert!(now < home, "{at}: school ended already");
        // Busy as school ends with her home still standing: in the app,
        // only a visitor's key on her errand's last frames does this (it
        // waits for her poke, and leaves the idle timer set, as here),
        // too narrow a window to set up honestly.
        guest.quiet_since = home - 1;
        guest.advance(home);
        assert!(
            matches!(guest.state, State::Leaving(_)),
            "{at}: her home rains out"
        );
        paint(&mut guest, &real, &view, home);
        assert!(
            matches!(guest.state, State::Leaving(_)),
            "{at}: her home's rain drawn"
        );
    }
}

/// Home from school with nowhere for her door at the paint she'd come in
/// on (the whole screen kept clear): she doesn't come in, and her empty
/// home rains out rather than vanishing; at another size than it was
/// drawn at, it goes at once (the geometry it froze against is gone).
#[test]
fn home_from_school_with_nowhere_to_come_in_her_empty_home_rains_out() {
    let (real, view) = home_screen();
    for resized in [false, true] {
        for graphics in [false, true] {
            let at = format!("resized={resized} graphics={graphics}");
            let mut guest = home_at(4, tue(12, 44), &HOME, graphics);
            let now = until_away(&mut guest, &real, &view, 0);
            let home = real_of(&guest, now, tue(12, 45));
            assert!(now < home, "{at}: school ended already");
            guest.advance(home);
            assert!(
                matches!(guest.state, State::Arriving(How::Return, Some(_))),
                "{at}: home from school, from her home"
            );
            let area = if resized {
                Rect::new(0, 0, real.area.width + 4, real.area.height)
            } else {
                real.area
            };
            let mut screen = Buffer::empty(area);
            screen.merge(&real);
            let kept = IdleView {
                protected: vec![area],
                ..view.clone()
            };
            paint(&mut guest, &screen, &kept, home);
            assert!(
                !matches!(guest.state, State::Visiting(_)),
                "{at}: she came in"
            );
            if resized {
                assert!(
                    matches!(guest.state, State::Absent),
                    "{at}: her home goes at once"
                );
                assert!(guest.fades.is_empty(), "{at}: nothing rains on");
            } else {
                assert!(
                    matches!(guest.state, State::Leaving(_)),
                    "{at}: her home rains out"
                );
            }
        }
    }
}

// ---- Her door at its space (door batch, step 3) ----

/// Where her closed door stands while she's out, if it does: its floor
/// spot (where she'd stand to go through it).
fn door_spot_of(guest: &Guest) -> Option<(i32, i32)> {
    guest.closed_door().map(door::DoorSpot::spot)
}

/// The box her closed door stands in while she's out, if it does.
fn door_box_of(guest: &Guest) -> Option<Rect> {
    door_spot_of(guest).and_then(|(x, y)| room::her_box(x, y))
}

/// Her one `item` on Users of [`rooms_frame`] at 100×30, a Tuesday at
/// 08:12, she cued to each of `scenes` with it: in it as school begins
/// (asserted), she goes out, and her closed door's box meets none of her
/// pieces. Returns nothing; each case's own asserts say what failed.
fn out_of_a_piece_at_school_time(item: Furniture, scenes: &[Scene]) {
    let (real, view) = rooms_frame(100, 30);
    for &scene in scenes {
        for graphics in [false, true] {
            let at = format!("{item:?} {scene:?} graphics={graphics}");
            let mut guest = home_at(4, tue(8, 12), &[(item, Nook::Users, 300)], graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            guest.cue(scene);
            let school = real_of(&guest, now, tue(8, 15));
            // At the last frame before 08:15: whether she's using it, and
            // whether her box meets its cover.
            let mut last_in = None;
            // Where she sat, and where she went out through her door
            // (step 4a: she walks to it from her seat).
            let (mut seat, mut opened) = (None, None);
            while empty_of(&guest).is_none() {
                assert!(now < school + 60_000, "{at}: never out");
                shell_step(&mut guest, &real, &view, &mut now, true);
                if let State::Visiting(visit) = &guest.state
                    && let Some(osaka::Through::Home(door)) = visit.osaka.through()
                {
                    opened.get_or_insert((door.spot(), (visit.osaka.x, visit.osaka.y)));
                }
                if let State::Visiting(visit) = &guest.state
                    && now < school
                {
                    let osaka = &visit.osaka;
                    seat = Some((osaka.x, osaka.y));
                    let using = osaka
                        .use_span()
                        .is_some_and(|(seat, ..)| seat.piece == room::PieceRef::Real(item));
                    let meets = visit
                        .shown
                        .iter()
                        .find(|s| s.item == item && s.scrap.is_none())
                        .is_some_and(|piece| {
                            room::her_box(osaka.x, osaka.y)
                                .is_some_and(|b| b.intersects(piece.cover()))
                        });
                    last_in = Some((using, meets));
                }
            }
            assert_eq!(last_in, Some((true, true)), "{at}: in it as school began");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let empty = empty_of(&guest).expect("out");
            let door = door_box_of(&guest).unwrap_or_else(|| panic!("{at}: her door"));
            for piece in &empty.shown {
                assert!(
                    !piece.cover().intersects(door),
                    "{at}: her door {door:?} on her {:?} {:?}",
                    piece.item,
                    piece.cover()
                );
            }
            // In its space: Users' right wall, at the screen's edge (her
            // piece's strip), worked out from the nook alone.
            let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
            let spot = guest.closed_door().expect("her door");
            assert_eq!(spot.spot(), space, "{at}: in its space");
            assert_eq!(opened, Some((space, space)), "{at}: out through it there");
            assert_ne!(seat, Some(space), "{at}: she walked there from her seat");
            assert_eq!(
                spot.set(),
                door::Set::Wall {
                    side: room::Side::Right,
                    wall: 99
                },
                "{at}"
            );
            // At 12:45 she comes out of it.
            guest.skip_clock(now);
            let back = now;
            loop {
                assert!(now < back + 20_000, "{at}: never home");
                shell_step(&mut guest, &real, &view, &mut now, true);
                if let State::Visiting(visit) = &guest.state {
                    let osaka = &visit.osaka;
                    assert_eq!((osaka.x, osaka.y), space, "{at}: out of her door");
                    break;
                }
            }
        }
    }
}

#[test]
fn out_of_her_bed_at_school_time_her_door_misses_it() {
    out_of_a_piece_at_school_time(Furniture::Bed, &[Scene::Sleep]);
}

#[test]
fn out_of_her_sofa_at_school_time_her_door_misses_it() {
    out_of_a_piece_at_school_time(Furniture::Sofa, &[Scene::Lounge, Scene::Nap]);
}

#[test]
fn out_of_her_desk_at_school_time_her_door_misses_it() {
    out_of_a_piece_at_school_time(Furniture::Desk, &[Scene::Homework]);
}

/// Her door is never in the chat pane: set down on the chat pane's floor
/// ([`chat_apart`]: the tall left box) a moment before 08:15, she goes
/// out, and her closed door stands clear of it. A guard against a door
/// placed from her feet (red before the door batch's step 3, when the
/// door stood where she set off); her door no longer reads her feet, so
/// no mutant of `door_place` turns it red.
#[test]
fn her_door_is_never_in_the_chat() {
    let (real, view) = chat_apart(100, 30);
    let chat_floor = i32::from(view.chat.bottom()) - 1;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(8, 14),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let school = real_of(&guest, now, tue(8, 15));
        // 08:14:59 by her clock.
        let placed = school - 1000 / CLOCK_SPEED;
        while now < placed {
            now += (placed - now).min(500);
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
        }
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.place(25, chat_floor, now);
        let her = room::her_box(25, chat_floor).unwrap();
        assert!(her.intersects(view.chat), "{at}: she stands in the chat");
        paint(&mut guest, &real, &view, now);
        // Where she stood at the last visiting frame before 08:15.
        let mut last = Some(her);
        while empty_of(&guest).is_none() {
            assert!(now < school + 60_000, "{at}: never out");
            shell_step(&mut guest, &real, &view, &mut now, true);
            if let State::Visiting(visit) = &guest.state
                && now < school
            {
                last = room::her_box(visit.osaka.x, visit.osaka.y);
            }
        }
        assert!(
            last.is_some_and(|b| b.intersects(view.chat)),
            "{at}: in the chat as school began: {last:?}"
        );
        shell_step(&mut guest, &real, &view, &mut now, true);
        let door = door_box_of(&guest).unwrap_or_else(|| panic!("{at}: her door"));
        assert!(
            !door.intersects(view.chat),
            "{at}: her door {door:?} in the chat {:?}",
            view.chat
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(16)))]

    /// Whatever she's in as school begins (a bed, a sofa, a desk, cued),
    /// with a lamp, a fridge or a window besides, on her rooms or with
    /// the chat apart: while she's out, her closed door meets none of her
    /// pieces (checked in [`long_visit_of`] on every frame her home
    /// stands empty), and she does go out.
    #[test]
    fn her_school_mornings_never_put_her_door_on_a_piece(
        seed in any::<u64>(),
        day in 1u64..5,
        minute in 12u16..=14,
        graphics in any::<bool>(),
        main in 0usize..4,
        pane in 1usize..3,
        along in 0u16..=1000,
        left in any::<bool>(),
        extra in proptest::option::of((0usize..3, 1usize..3, 0u16..=1000)),
        apart in any::<bool>(),
        size in (70u16..130, 20u16..45),
        span in 90_000u64..=120_000,
    ) {
        let (item, scene) = [
            (Furniture::Bed, Scene::Sleep),
            (Furniture::Sofa, Scene::Lounge),
            (Furniture::Sofa, Scene::Nap),
            (Furniture::Desk, Scene::Homework),
        ][main];
        let mut owned = vec![(item, pane, along, left)];
        if let Some((more, at, along)) = extra {
            let more = [Furniture::Lamp, Furniture::Fridge, Furniture::Window][more];
            owned.push((more, at, along, false));
        }
        let mut guest = Guest::restore(Ledger::new_at(seed, routine::GameTime { day, h: 8, m: minute }));
        guest.set_date(date(2026, 6, 17));
        let frame = if apart { chat_apart } else { rooms_frame };
        let visited = long_visit_of(
            guest,
            graphics,
            frame,
            &[size],
            &[],
            &[],
            &[],
            (0, 0, 1, 1),
            &owned,
            Run {
                cue: Some(scene),
                out_every: Some(1000),
                ..Run::default()
            },
            span,
        )?;
        prop_assert!(visited.away > 0, "never out: {:?}", visited);
    }
}

/// [`her_school_mornings_never_put_her_door_on_a_piece`]'s deterministic
/// companions, where its random homes seldom reach: (a) on her rooms,
/// out of her sofa, her door drawn in its space; (b) on chat case (i),
/// her pieces filling the space ([`FULL_SPACE_HOME`]), her door drawn at
/// the fallback (the space yields), meeting none of them (both checked
/// on every frame by [`long_visit_of`]). Both modes.
#[test]
fn her_school_mornings_put_her_door_in_its_space_or_clear_of_her_pieces() {
    let full: Vec<(Furniture, usize, u16, bool)> = FULL_SPACE_HOME
        .iter()
        .map(|&(item, _, along)| (item, 1, along, false))
        .collect();
    for graphics in [false, true] {
        for (name, full_space) in [("rooms", false), ("(i)", true)] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = Guest::restore(Ledger::new_at(
                4,
                routine::GameTime {
                    day: 2,
                    h: 8,
                    m: 13,
                },
            ));
            guest.set_date(date(2026, 6, 17));
            let how = Run {
                cue: Some(Scene::Lounge),
                out_every: Some(1000),
                ..Run::default()
            };
            let visited = if full_space {
                long_visit_of(
                    guest,
                    graphics,
                    |_, _| chat_by_a_full_space(),
                    &[(100, 30)],
                    &[],
                    &[],
                    &[],
                    (0, 0, 1, 1),
                    &full,
                    how,
                    90_000,
                )
            } else {
                long_visit_of(
                    guest,
                    graphics,
                    rooms_frame,
                    &[(100, 30)],
                    &[],
                    &[],
                    &[],
                    (0, 0, 1, 1),
                    &[(Furniture::Sofa, 1, 300, false)],
                    how,
                    90_000,
                )
            }
            .unwrap_or_else(|e| panic!("{at}: {e}"));
            if full_space {
                assert!(visited.floors > 0, "{at}: {visited:?}");
            } else {
                assert!(visited.walls > 0, "{at}: {visited:?}");
            }
        }
    }
}

/// Her home (`pieces`) standing empty at Tuesday 09:00 on `view` of
/// `real`, painted a few frames: the guest, and when.
fn away_on(
    real: &Buffer,
    view: &IdleView,
    pieces: &[(Furniture, Nook, u16)],
    graphics: bool,
) -> (Guest, u64) {
    let mut guest = home_at(4, tue(9, 0), pieces, graphics);
    let now = until_away(&mut guest, real, view, 0);
    (guest, now)
}

/// From `now`, on to 12:45 and her coming home: where she first stands,
/// in, if she comes in within 20 s of it.
fn home_at_1245(guest: &mut Guest, real: &Buffer, view: &IdleView, now: u64) -> Option<(i32, i32)> {
    came_home_at_1245(guest, real, view, now).map(|(spot, _)| spot)
}

/// [`home_at_1245`], and which way she faces as she first stands there.
fn came_home_at_1245(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    mut now: u64,
) -> Option<((i32, i32), sprite::Facing)> {
    let home = real_of(guest, now, tue(12, 45));
    guest.skip_clock(now);
    while now < home + 20_000 {
        shell_step(guest, real, view, &mut now, true);
        if let State::Visiting(visit) = &guest.state {
            return Some(((visit.osaka.x, visit.osaka.y), visit.osaka.facing));
        }
    }
    None
}

/// The laid covers of her home on `view`'s nooks: what her door must
/// meet none of.
fn laid_covers(guest: &Guest, view: &IdleView) -> Vec<Rect> {
    guest
        .ledger
        .home
        .laid_and_shifted(&view.nooks)
        .shown
        .iter()
        .map(Shown::cover)
        .collect()
}

/// Her door is never in the chat pane, on the layouts built for it
/// (door batch, T6), each precondition asserted by the fixture's own
/// test in tests.rs: (i) her pieces fill her door's space, and the
/// nearest floor to it is the chat's: the fallback passes it by for
/// List's; (ii) every edge wall's space meets the chat: an inner wall's
/// space; (iii) no wall of hers outside the chat: the face-on fallback
/// outside it. While she's out, and as she comes home at 12:45 (out of
/// that door). Both modes.
#[test]
fn her_door_is_never_in_the_chat_on_the_chat_cases() {
    let sofa = [(Furniture::Sofa, Nook::Users, 500)];
    type Case<'a> = (&'a str, (Buffer, IdleView), &'a [(Furniture, Nook, u16)]);
    let cases: [Case; 3] = [
        ("(i)", chat_by_a_full_space(), &FULL_SPACE_HOME),
        ("(ii)", chat_over_every_edge_space(), &sofa),
        ("(iii)", chat_over_every_space(), &sofa),
    ];
    for (name, (real, view), pieces) in cases {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let (mut guest, mut now) = away_on(&real, &view, pieces, graphics);
            now += 500;
            guest.advance(now);
            let frame = paint(&mut guest, &real, &view, now);
            let door = guest
                .closed_door()
                .unwrap_or_else(|| panic!("{at}: her door"));
            let rect = door.rect().unwrap();
            assert!(!rect.intersects(view.chat), "{at}: {door:?} in the chat");
            let covers = laid_covers(&guest, &view);
            match name {
                "(i)" => {
                    // The user's answer (i), 2026-10-08: a face-on door
                    // never straddles a pane's border. (67, 26) would put
                    // her box over List's right border (column 69).
                    assert_eq!(door.spot(), (66, 26), "{at}: clear of List's wall");
                    assert_eq!(door.set(), door::Set::Floor(door::Fallback::Yield), "{at}");
                    let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
                    assert_eq!(
                        Some(door.spot()),
                        nearest_door_floor(&real, &view.protected, &covers, view.chat, space),
                        "{at}: the nearest clear floor outside the chat"
                    );
                }
                "(ii)" => {
                    let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
                    assert_eq!(door.spot(), space, "{at}: an inner wall's space");
                    assert!(door.wall().is_some(), "{at}");
                }
                _ => {
                    assert_eq!(door.set(), door::Set::Floor(door::Fallback::NoWall), "{at}");
                    assert!(
                        door_floor(&real, &view.protected, &covers, view.chat, door.spot()),
                        "{at}: {door:?}"
                    );
                }
            }
            let empty = empty_of(&guest).expect("away");
            assert!(door_shows(empty, &frame, &real, door), "{at}: drawn");
            let came = home_at_1245(&mut guest, &real, &view, now);
            assert_eq!(came, Some(door.spot()), "{at}: home out of it");
        }
    }
}

/// A saved wall whose space comes to lie under the chat pane (a pane
/// dragged over it) isn't chosen again: while the chat meets it, her
/// door falls back clear of the chat (`Fallback::Chat`); with the chat
/// gone again, it's back in its space.
#[test]
fn her_doors_space_under_the_chat_falls_back_and_comes_back() {
    let (real, view) = rooms_frame(100, 30);
    let pieces = [(Furniture::Sofa, Nook::Users, 300)];
    let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
    let over = IdleView {
        chat: Rect::new(80, 0, 20, 13),
        ..view.clone()
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = away_on(&real, &view, &pieces, graphics);
        let wall = guest.ledger.home.door;
        assert_eq!(
            guest.closed_door().map(door::DoorSpot::spot),
            Some(space),
            "{at}"
        );
        now += 500;
        guest.advance(now);
        paint(&mut guest, &real, &over, now);
        let fell = guest.closed_door().expect("her door");
        assert_eq!(fell.set(), door::Set::Floor(door::Fallback::Chat), "{at}");
        assert!(
            !fell.rect().unwrap().intersects(over.chat),
            "{at}: {fell:?}"
        );
        assert_eq!(guest.ledger.home.door, wall, "{at}: not chosen again");
        now += 500;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        assert_eq!(
            guest.closed_door().map(door::DoorSpot::spot),
            Some(space),
            "{at}: back"
        );
    }
}

/// With no door anywhere outside the chat (chat case (iv): her one nook
/// is the chat pane, and there's no other floor), no door is drawn while
/// she's out, and at 12:45 she doesn't come in inside the chat: there's
/// nowhere calm outside it, so she waits (D5 as written; where she
/// should come home here is the user's call, door-notes step 1).
#[test]
fn with_no_door_anywhere_she_never_comes_in_inside_the_chat() {
    let (real, view) = chat_over_every_floor();
    let pieces = [(Furniture::Sofa, Nook::Users, 500)];
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = away_on(&real, &view, &pieces, graphics);
        now += 500;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        let empty = empty_of(&guest).expect("away");
        assert!(empty.door.is_none(), "{at}: {:?}", empty.door);
        assert!(empty.image.is_none(), "{at}: no door's image");
        // School's out: with nowhere to come in, her coming home is
        // called off (she comes in later on the idle gate, as any
        // visitor, wherever she lands).
        guest.skip_clock(now);
        shell_step(&mut guest, &real, &view, &mut now, true);
        assert!(
            !matches!(guest.state, State::Visiting(_)),
            "{at}: home out of a door at {:?}",
            visit_of(&guest).osaka.act_name()
        );
    }
}

/// On a short terminal (the bundled layout at 18, 19, 20 or 22 rows,
/// her door's strip too short for its space: the precondition, every
/// frame) her door stands at the strict fallback meeting none of her
/// pieces and clear of the chat, or nowhere (and her coming home waits);
/// at 12:45 she comes home out of it. The fallback is `Short`, or
/// `NoWall` once her pieces, not fitting her door's strip, have moved
/// and taken her door's wall with them to a strip with no wall that
/// qualifies (step 2's `move_off`). A face-on door faces out toward the
/// nearer edge of the screen, and she comes in facing into the room.
/// Returns how many frames showed her door at the fallback with her
/// door's wall short (the property's own non-vacuity).
fn short_terminal(
    width: u16,
    height: u16,
    graphics: bool,
    owned: &[(Furniture, Nook, u16)],
) -> usize {
    let at = format!("{width}x{height} graphics={graphics} {owned:?}");
    let (real, view) = real_frame(&mut real_ui(), width, height);
    let mut guest = home_at(4, tue(9, 0), owned, graphics);
    guest.ledger.home.door = Some(room::DoorWall {
        strip: room::Strip::Bottom(Nook::Users),
        side: room::Side::Right,
    });
    let mut now = until_away(&mut guest, &real, &view, 0);
    let mut door = None;
    let mut fell = 0;
    for _ in 0..10 {
        now += 1000;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        let plan = room::Plan {
            nooks: &view.nooks,
            chat: view.chat,
            screen: real.area,
        };
        // Short: her door's wall (wherever her pieces took it) has no
        // space it keeps. (At 22 rows the List pane has: her pieces,
        // not fitting her door's strip, may move there with her door.)
        let home = &guest.ledger.home;
        let short = home.wall(plan).is_none_or(|wall| {
            let doored = room::Home {
                door: Some(wall),
                ..home.clone()
            };
            let strips = doored.extents(&view.nooks);
            let strip = strips.iter().find(|p| p.strip == wall.strip);
            strip.is_none_or(|p| p.space.is_none_or(|s| !s.kept))
        });
        let covers = laid_covers(&guest, &view);
        door = guest.closed_door();
        if let Some(door) = door {
            if short {
                assert!(
                    matches!(
                        door.set(),
                        door::Set::Floor(door::Fallback::Short | door::Fallback::NoWall)
                    ),
                    "{at}: {door:?}"
                );
                fell += 1;
            }
            if door.wall().is_none() {
                let (x, _) = door.spot();
                let nearer = if 2 * x < i32::from(width) {
                    sprite::Facing::Left
                } else {
                    sprite::Facing::Right
                };
                assert_eq!(door.out(), nearer, "{at}: {door:?} out to the nearer edge");
            }
            let rect = door.rect().unwrap();
            assert!(!covers.iter().any(|c| c.intersects(rect)), "{at}: {door:?}");
            assert!(!rect.intersects(view.chat), "{at}: {door:?}");
        }
    }
    let came = came_home_at_1245(&mut guest, &real, &view, now);
    match door {
        Some(door) => assert_eq!(
            came,
            Some((door.spot(), door.into_room())),
            "{at}: home out of it, into the room"
        ),
        // No door: if she comes home at all, it isn't out of a door in
        // a wall.
        None => {
            if came.is_some() {
                let visit = visit_of(&guest);
                assert!(
                    visit.door.is_none_or(|d| d.wall().is_none()),
                    "{at}: {:?}",
                    visit.door
                );
            }
        }
    }
    fell
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(16)))]

    #[test]
    fn on_a_short_terminal_her_door_falls_back_clear_of_every_piece(
        height in prop::sample::select(vec![18u16, 19, 20, 22]),
        width in 80u16..=129,
        graphics in any::<bool>(),
        items in proptest::sample::subsequence(
            vec![
                Furniture::Sofa,
                Furniture::Tv,
                Furniture::Lamp,
                Furniture::Fridge,
                Furniture::Bed,
                Furniture::Desk,
            ],
            1..4,
        ),
        places in proptest::collection::vec((0usize..2, 0u16..=1000), 3),
    ) {
        let owned: Vec<(Furniture, Nook, u16)> = items
            .into_iter()
            .zip(places)
            .map(|(item, (nook, x))| (item, [Nook::Users, Nook::Playlist][nook], x))
            .collect();
        // Only a case with her door at the fallback for a short wall
        // says anything (at 22 rows List may take her pieces and door).
        prop_assume!(short_terminal(width, height, graphics, &owned) > 0);
    }
}

/// [`on_a_short_terminal_her_door_falls_back_clear_of_every_piece`]'s
/// non-vacuity: on the bundled layout at 100×20 her door does stand
/// somewhere while she's out (the fallback, `Short`).
#[test]
fn on_a_short_terminal_her_door_does_stand() {
    let (real, view) = real_frame(&mut real_ui(), 100, 20);
    for graphics in [false, true] {
        let mut guest = home_at(
            4,
            tue(9, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        guest.ledger.home.door = Some(room::DoorWall {
            strip: room::Strip::Bottom(Nook::Users),
            side: room::Side::Right,
        });
        let now = until_away(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        let door = guest.closed_door().expect("her door");
        assert_eq!(
            door.set(),
            door::Set::Floor(door::Fallback::Short),
            "graphics={graphics}"
        );
        let fell = short_terminal(100, 20, graphics, &[(Furniture::Sofa, Nook::Users, 300)]);
        assert!(fell > 0, "graphics={graphics}: {fell}");
    }
}

/// Her fallback door (on a short terminal, as above) never hops with the
/// text in a pane: text written into its box and out again, the spot is
/// the same every frame, and it's drawn over the text. Both modes.
#[test]
fn her_fallback_door_doesnt_hop_with_pane_text() {
    let (quiet, view) = real_frame(&mut real_ui(), 100, 20);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(9, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        guest.ledger.home.door = Some(room::DoorWall {
            strip: room::Strip::Bottom(Nook::Users),
            side: room::Side::Right,
        });
        let mut now = until_away(&mut guest, &quiet, &view, 0);
        paint(&mut guest, &quiet, &view, now);
        let door = guest.closed_door().expect("her door");
        assert!(door.wall().is_none(), "{at}: the fallback");
        let rect = door.rect().unwrap();
        let mut wordy = quiet.clone();
        for y in rect.y..rect.bottom() - 1 {
            wordy.set_string(rect.x, y, "words", Style::new());
        }
        for (i, real) in [&wordy, &quiet, &wordy, &quiet].into_iter().enumerate() {
            now += 1000;
            guest.advance(now);
            let frame = paint(&mut guest, real, &view, now);
            assert_eq!(guest.closed_door(), Some(door), "{at}: frame {i}");
            let empty = empty_of(&guest).expect("away");
            assert!(
                door_shows(empty, &frame, real, door),
                "{at}: drawn, frame {i}"
            );
        }
    }
}

/// Text written into her door's space while she's out: her door stands
/// at the same spot, drawn over it (the user's answer, door batch). On
/// [`home_screen`], both modes.
#[test]
fn her_door_stands_over_text_and_keeps_its_spot() {
    let (quiet, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now, door) = out_to_school(&quiet, &view, graphics);
        assert!(door.wall().is_some(), "{at}: in its space");
        let rect = door.rect().unwrap();
        let mut wordy = quiet.clone();
        for y in rect.y..rect.bottom() - 1 {
            wordy.set_string(rect.x, y, "texts", Style::new());
        }
        now += 1000;
        guest.advance(now);
        let frame = paint(&mut guest, &wordy, &view, now);
        assert_eq!(guest.closed_door(), Some(door), "{at}: the same spot");
        let empty = empty_of(&guest).expect("away");
        assert!(
            !empty.door.as_ref().unwrap().cells.is_empty(),
            "{at}: drawn"
        );
        assert!(
            door_shows(empty, &frame, &wordy, door),
            "{at}: over the text"
        );
    }
}

/// With her door's pane focused and in use (a resident's), at 12:45 she
/// neither comes out inside it nor walks into it: her door falls back
/// outside the pane (`Fallback::Protected`), and she comes home out of
/// that; the visit's own door is the same fallback.
#[test]
fn with_her_doors_pane_focused_she_neither_comes_out_nor_walks_into_it() {
    let (w, h) = (100, 30);
    let real = wordy_rooms(w, h);
    let users = nooks(w, h)[1].1;
    let focused = resident_view(w, h, Some(users));
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = resident_at(graphics);
        let quiet = resident_view(w, h, None);
        let now = until_away(&mut guest, &real, &quiet, 0);
        let space = guest.closed_door().expect("her door");
        assert!(space.wall().is_some(), "{at}: in its space");
        assert!(space.rect().unwrap().intersects(users), "{at}: in Users");
        let came = home_at_1245(&mut guest, &real, &focused, now).expect("home");
        assert!(
            !osaka::box_meets(users, came),
            "{at}: came out in it at {came:?}"
        );
        let visit = visit_of(&guest);
        let door = visit.door.expect("her door");
        assert_eq!(
            door.set(),
            door::Set::Floor(door::Fallback::Protected),
            "{at}"
        );
        assert!(!door.rect().unwrap().intersects(users), "{at}: {door:?}");
        assert_eq!(came, door.spot(), "{at}: out of her door");
    }
}

/// What she made (a scrap) or the place kept for the piece in her pocket
/// in her door's space: it falls back (`Fallback::Blocked`). She never
/// builds anything in the space, and a made piece the space comes to
/// meet falls apart (its stand refused). On [`rooms`] at 100×30, her
/// sofa on Users (her door's space at its right wall, 93..=98).
#[test]
fn a_scrap_in_her_door_space_makes_it_fall_back() {
    let (real, view) = rooms_frame(100, 30);
    let mut home = room::Home::default();
    assert!(home.add(room::Prop::new(
        Furniture::Sofa,
        Nook::Users,
        300,
        sprite::Facing::Right
    )));
    let plan = room::Plan {
        nooks: &view.nooks,
        chat: view.chat,
        screen: real.area,
    };
    home.layout(&view.nooks);
    home.settle_door(plan);
    let laid = home.laid_and_shifted(&view.nooks).shown;
    let space = Rect::new(93, 8, 6, 5);
    let scrap = Rect::new(93, 10, 3, 3);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let ground = Terrain::read(&real, &view.protected, graphics);
        let clear = door::door_place(
            &home,
            plan,
            &door::obstacles(&laid, [], None),
            &ground,
            None,
        )
        .expect("a door");
        assert_eq!(clear.rect(), room::her_box(96, 12), "{at}: in its space");
        assert!(clear.rect().unwrap().intersects(space), "{at}");
        for (what, made, ghost) in [("made", Some(scrap), None), ("ghost", None, Some(scrap))] {
            let obstacles = door::obstacles(&laid, made, ghost);
            let fell = door::door_place(&home, plan, &obstacles, &ground, None).expect("a door");
            assert_eq!(
                fell.set(),
                door::Set::Floor(door::Fallback::Blocked),
                "{at} {what}"
            );
            assert!(!fell.rect().unwrap().intersects(scrap), "{at} {what}");
        }
    }
    let keep = door::Keep::of(&home, plan);
    assert!(keep.refuses(scrap), "the space is kept clear");
    assert!(
        !keep.refuses(Rect::new(70, 10, 3, 3)),
        "the rest of the strip isn't"
    );
    // She builds nothing there: the keybar's line, as though she pulled
    // it standing on Users' floor, at its middle (made) and in the space
    // (refused).
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            3,
            tue(14, 0),
            &[(Furniture::Sofa, Nook::Users, 0)],
            graphics,
        );
        tv_held_back(&mut guest);
        let now = until_visiting(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        let visit = visit_of(&guest);
        let keep = door::Keep::of(&guest.ledger.home, plan);
        let row = real.area.height - 2;
        let pull = |x: i32| scenes::Pull {
            x,
            y: 12,
            row,
            side: scenes::Side::Right,
            cells: (0..26).collect(),
            glyphs: "Tab Next pane | Enter Send".to_owned(),
            gap: 0,
        };
        let middle = builds(&real, visit, &[pull(80)], &[], keep.for_made());
        assert!(!middle.is_empty(), "{at}: a piece mid-strip");
        let free = builds(
            &real,
            visit,
            &[pull(95)],
            &[],
            door::Keep::default().for_made(),
        );
        assert!(!free.is_empty(), "{at}: one in the space, nothing kept");
        let kept = builds(&real, visit, &[pull(95)], &[], keep.for_made());
        assert!(
            kept.is_empty(),
            "{at}: made in her door's space: {:?}",
            kept.iter().map(|b| b.piece).collect::<Vec<_>>()
        );
    }
}

// ---- Her door at its space: the step 3 review's tests ----

/// [`rooms_frame`] at 100×30 with one line of text in Playlist ("abcdefghij"
/// at columns 80..=89 on row 23, over its floor at 26) and Playlist kept
/// out of left of it (columns 51..=79 above its floor): the only pulls are
/// from her standing right of the line, so every makeshift sofa she
/// could make stands by Playlist's right wall, in that wall's space
/// (93..=98 × 22..=26).
fn by_playlists_right_wall() -> (Buffer, IdleView, Rect) {
    let (mut real, mut view) = rooms_frame(100, 30);
    real.set_string(80, 23, "abcdefghij", Style::new());
    view.protected.push(Rect::new(51, 14, 29, 12));
    (real, view, Rect::new(93, 22, 6, 5))
}

/// Her door on Playlist's right wall (at the screen's edge).
const PLAYLIST_RIGHT: room::DoorWall = room::DoorWall {
    strip: room::Strip::Bottom(Nook::Playlist),
    side: room::Side::Right,
};

/// The makeshift pieces of `guest`'s visit meeting `space`.
fn made_in(guest: &Guest, space: Rect) -> Vec<Rect> {
    visit_of(guest)
        .made
        .iter()
        .map(|m| m.piece.cover())
        .filter(|c| c.intersects(space))
        .collect()
}

/// She never makes a piece in her door's space, cued to make a sofa
/// where the only sofa she could make would stand there: the cue finds
/// no room, and nothing she makes meets the space. (With nothing kept,
/// that sofa is on offer: asserted.) Through the visiting frame, so its
/// own `Keep` is what keeps it out.
#[test]
fn she_never_makes_a_piece_in_her_door_space() {
    let (real, view, space) = by_playlists_right_wall();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            3,
            tue(14, 0),
            &[(Furniture::Sofa, Nook::Playlist, 0)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        assert_eq!(guest.ledger.home.door, Some(PLAYLIST_RIGHT), "{at}");
        // Non-vacuous: with nothing kept, she'd make a sofa there.
        let visit = visit_of(&guest);
        let mut solid = view.protected.clone();
        solid.extend(visit.shown.iter().map(Shown::cover));
        let pulls = scenes::pulls(&real, &visit.terrain, &solid);
        let offered = builds(
            &real,
            visit,
            &pulls,
            &solid,
            door::Keep::default().for_made(),
        );
        assert!(
            offered
                .iter()
                .any(|b| b.piece.item == Furniture::Sofa && b.piece.cover().intersects(space)),
            "{at}: a sofa on offer in the space"
        );
        guest.cue(Scene::MakeSofa);
        let end = now + 30_000;
        while now < end {
            shell_step(&mut guest, &real, &view, &mut now, true);
            assert!(
                made_in(&guest, space).is_empty(),
                "{at}: made in her door's space at {now}"
            );
        }
        assert!(
            matches!(guest.cue_note(), Some(Err(_))),
            "{at}: {:?}",
            guest.cue_note()
        );
    }
}

/// [`by_playlists_right_wall`] with her sofa on Users (her door at Users'
/// right wall): she makes a sofa by Playlist's right wall, out of the
/// way of her door. Returns when it's made, and the frame.
fn made_by_playlists_right_wall(graphics: bool) -> (Guest, u64, Buffer, IdleView, Rect) {
    let (real, view, space) = by_playlists_right_wall();
    let mut guest = home_at(
        3,
        tue(14, 0),
        &[(Furniture::Sofa, Nook::Users, 0)],
        graphics,
    );
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    paint(&mut guest, &real, &view, now);
    assert_ne!(guest.ledger.home.door, Some(PLAYLIST_RIGHT));
    guest.cue(Scene::MakeSofa);
    let end = now + 30_000;
    while visit_of(&guest).made.is_empty() {
        assert!(now < end, "graphics={graphics}: never made");
        shell_step(&mut guest, &real, &view, &mut now, true);
    }
    (guest, now, real, view, space)
}

/// A made piece her door's space comes to meet falls apart (D3's
/// symmetric path): her sofa of text by Playlist's right wall, then her
/// door's wall moved there (as a hidden pane can move it with her
/// pieces): at the next frame it's gone, its text back in its line.
#[test]
fn a_made_piece_her_door_space_comes_to_meet_falls_apart() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now, real, view, space) = made_by_playlists_right_wall(graphics);
        let [made] = visit_of(&guest).made.as_slice() else {
            panic!("{at}: one piece");
        };
        assert!(made.piece.cover().intersects(space), "{at}: by the wall");
        let torn = made.torn.clone();
        // It stands a frame first.
        now += 100;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        assert_eq!(made_in(&guest, space).len(), 1, "{at}: it stands");
        guest.ledger.home.door = Some(PLAYLIST_RIGHT);
        now += 100;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        assert!(
            visit_of(&guest).made.is_empty(),
            "{at}: it fell apart: {:?}",
            visit_of(&guest).made
        );
        let frame = paint(&mut guest, &real, &view, now);
        for &(x, y) in &torn {
            assert_eq!(frame[(x, y)], real[(x, y)], "{at}: {:?} back", (x, y));
        }
    }
}

/// A piece she set out to make where her door's space comes meanwhile
/// is never made: her door's wall moved onto Playlist's right as she
/// reels in the text for a sofa there, nothing she makes meets the
/// space (it isn't made to fall apart the next frame).
#[test]
fn a_piece_she_sets_out_to_make_where_her_door_space_comes_isnt_made() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, view, space) = by_playlists_right_wall();
        let mut guest = home_at(
            3,
            tue(14, 0),
            &[(Furniture::Sofa, Nook::Users, 0)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        guest.cue(Scene::MakeSofa);
        let end = now + 30_000;
        while visit_of(&guest).osaka.reeling().is_none() {
            assert!(now < end, "{at}: never reeled");
            assert!(visit_of(&guest).made.is_empty(), "{at}: made already");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let build = visit_of(&guest).osaka.reeling().unwrap().piece.cover();
        assert!(build.intersects(space), "{at}: {build:?} by the wall");
        guest.ledger.home.door = Some(PLAYLIST_RIGHT);
        while now < end {
            shell_step(&mut guest, &real, &view, &mut now, true);
            assert!(
                made_in(&guest, space).is_empty(),
                "{at}: made in her door's space at {now}"
            );
        }
    }
}

/// [`chat_over_every_space`] (her one nook the chat pane) with List's and
/// Playlist's floors protected (the floor row alone: she may still stand
/// on a protected ledge, her body clear above it): no door at all (no
/// wall outside the chat, and no fallback floor, the fallback refusing a
/// protected cell under her), yet floor outside the chat to stand on.
fn no_door_but_floor_outside_the_chat() -> (Buffer, IdleView) {
    let (real, mut view) = chat_over_every_space();
    view.protected.push(Rect::new(0, 26, 100, 1));
    (real, view)
}

/// With no door of hers anywhere but floor to stand on outside the chat
/// (M22's case), no door stands while she's out, and at 12:45 she comes
/// home by a door in space there: out of the chat and her pieces, by a
/// door (not dropping in), the visit with no door of hers.
#[test]
fn with_no_door_anywhere_she_comes_home_by_a_door_in_space_outside_the_chat() {
    let (real, view) = no_door_but_floor_outside_the_chat();
    let pieces = [(Furniture::Sofa, Nook::Users, 500)];
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = away_on(&real, &view, &pieces, graphics);
        now += 500;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        let empty = empty_of(&guest).expect("away");
        assert!(empty.door.is_none(), "{at}: {:?}", empty.door);
        let covers = laid_covers(&guest, &view);
        assert!(
            !every_spot(real.area).any(|spot| door_floor(
                &real,
                &view.protected,
                &covers,
                view.chat,
                spot
            )),
            "{at}: no fallback floor"
        );
        let came = home_at_1245(&mut guest, &real, &view, now).expect("home");
        let visit = visit_of(&guest);
        let her = room::her_box(came.0, came.1).unwrap();
        assert!(!her.intersects(view.chat), "{at}: in the chat at {came:?}");
        assert!(
            !covers.iter().any(|c| c.intersects(her)),
            "{at}: on a piece at {came:?}"
        );
        assert_eq!(visit.osaka.act_name(), "Door", "{at}: by a door");
        assert_eq!(visit.door, None, "{at}");
    }
}

/// Her fallback door stays where it stood while that spot still passes,
/// even with a nearer one free again (M12): on a short terminal (the
/// `Short` fallback), a protected rect over its spot moves it on; with
/// the rect gone, it stays where it went, not back at the nearer spot.
#[test]
fn her_fallback_door_stays_where_it_went() {
    let (real, view) = real_frame(&mut real_ui(), 100, 20);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(9, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        guest.ledger.home.door = Some(room::DoorWall {
            strip: room::Strip::Bottom(Nook::Users),
            side: room::Side::Right,
        });
        let mut now = until_away(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        let first = guest.closed_door().expect("her door");
        assert_eq!(first.set(), door::Set::Floor(door::Fallback::Short), "{at}");
        let mut kept = view.clone();
        kept.protected.push(first.rect().unwrap());
        now += 1000;
        guest.advance(now);
        paint(&mut guest, &real, &kept, now);
        let moved = guest.closed_door().expect("her door");
        assert_ne!(moved.spot(), first.spot(), "{at}: moved on");
        for _ in 0..3 {
            now += 1000;
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            assert_eq!(
                guest.closed_door().map(door::DoorSpot::spot),
                Some(moved.spot()),
                "{at}: stays, not back at {:?}",
                first.spot()
            );
        }
    }
}

/// Her door stands `Unmarked` when the frame doesn't draw its wall (or
/// the floor under its space) as lines: a custom layout, a pane drawn
/// otherwise. On [`rooms`] at 100×30, her sofa on Users (her door's
/// space at its right wall, column 99, floor row 12).
#[test]
fn an_unmarked_wall_puts_her_door_at_the_fallback() {
    let (real, view) = rooms_frame(100, 30);
    let mut home = room::Home::default();
    assert!(home.add(room::Prop::new(
        Furniture::Sofa,
        Nook::Users,
        300,
        sprite::Facing::Right
    )));
    let plan = room::Plan {
        nooks: &view.nooks,
        chat: view.chat,
        screen: real.area,
    };
    home.layout(&view.nooks);
    home.settle_door(plan);
    let laid = home.laid_and_shifted(&view.nooks).shown;
    let obstacles = door::obstacles(&laid, [], None);
    let blank = |cells: &[(u16, u16)]| {
        let mut buf = real.clone();
        for &(x, y) in cells {
            buf.set_string(x, y, " ", Style::new());
        }
        buf
    };
    let no_wall: Vec<(u16, u16)> = (8..12).map(|y| (99, y)).collect();
    let no_floor: Vec<(u16, u16)> = (94..=98).map(|x| (x, 12)).collect();
    for graphics in [false, true] {
        let ground = Terrain::read(&real, &view.protected, graphics);
        let drawn = door::door_place(&home, plan, &obstacles, &ground, None).expect("a door");
        assert!(drawn.wall().is_some(), "graphics={graphics}: in its wall");
        for (what, cells) in [("wall", &no_wall), ("floor", &no_floor)] {
            let at = format!("{what} graphics={graphics}");
            let buf = blank(cells);
            let ground = Terrain::read(&buf, &view.protected, graphics);
            let fell = door::door_place(&home, plan, &obstacles, &ground, None).expect("a door");
            assert_eq!(
                fell.set(),
                door::Set::Floor(door::Fallback::Unmarked),
                "{at}"
            );
        }
    }
}

/// The visit's own door reads the text she moved as text, never as a
/// protected pane: text she tore off in her door's space leaves it in
/// its wall (text never moves her door, D4).
#[test]
fn text_she_moved_in_her_door_space_never_moves_her_visits_door() {
    let (mut real, view) = rooms_frame(100, 30);
    // Words in the space (93..=98 × 8..=12), over Users' floor.
    real.set_string(93, 10, "words", Style::new());
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            3,
            tue(14, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        let now = until_visiting(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        let before = visit_of(&guest).door.expect("her door");
        assert!(before.wall().is_some(), "{at}: in its wall: {before:?}");
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        assert!(
            visit
                .layer
                .tear(&real, &view.protected, &[(94, 10), (95, 10)]),
            "{at}: torn off"
        );
        paint(&mut guest, &real, &view, now + 100);
        assert_eq!(visit_of(&guest).door, Some(before), "{at}: unmoved");
    }
}

// ---- Her walk to her door (door batch, step 4a) ----

/// What her way out to school showed, frame by frame (door batch, step
/// 4a), until her home stood empty.
#[derive(Debug, Default)]
struct WayOut {
    /// Her set-off lines said this visit (OFF, from her pool's record;
    /// "Late, late, late!" isn't pooled and is counted apart).
    off_lines: usize,
    late: bool,
    /// Her routine's away reflex taken: the set-off and each re-entry.
    reentries: usize,
    /// Frames on her walk to her door.
    walking: usize,
    /// Through a door in space on her way (no route between floors).
    spaced: bool,
    /// Off the screen on her way (an `Around` route).
    around: bool,
    /// Her external door as it first opened: its spot, where she stood,
    /// and her pieces' covers then.
    opened: Option<(door::DoorSpot, (i32, i32), Vec<Rect>)>,
    /// The visit ended (her home stood empty) before her door opened.
    early: bool,
}

/// From `now`, frame by frame on `real`/`view`, until her home stands
/// empty (asserted within `limit` ms): what her way out showed.
fn way_out(guest: &mut Guest, real: &Buffer, view: &IdleView, now: &mut u64, limit: u64) -> WayOut {
    let mut seen = WayOut::default();
    let end = *now + limit;
    while empty_of(guest).is_none() {
        assert!(*now < end, "never out: {seen:?}");
        shell_step(guest, real, view, now, true);
        let State::Visiting(visit) = &guest.state else {
            if empty_of(guest).is_some() && seen.opened.is_none() {
                seen.early = true;
            }
            continue;
        };
        let osaka = &visit.osaka;
        seen.off_lines = osaka
            .said_lines()
            .iter()
            .filter(|(pool, ..)| *pool == mind::OFF.id)
            .count();
        seen.late |= osaka.appearance(*now).2 == Some(Bubble::Say(osaka::LATE));
        seen.reentries = osaka
            .decisions
            .iter()
            .filter(|d| d.method == "routine/away")
            .count();
        seen.walking += usize::from(osaka.act_summary().contains("her door"));
        match osaka.through() {
            Some(osaka::Through::Space(_)) if seen.opened.is_none() && seen.reentries > 0 => {
                seen.spaced = true;
            }
            Some(osaka::Through::Home(door)) if seen.opened.is_none() => {
                let covers = visit.shown.iter().map(Shown::cover).collect();
                seen.opened = Some((door, (osaka.x, osaka.y), covers));
            }
            _ => {}
        }
        seen.around |= seen.reentries > 0 && matches!(osaka.act_name().as_str(), "Out" | "Away");
    }
    seen
}

/// A Tuesday at 08:14 on `real`/`view` with her sofa on Users, set down
/// at `at` a moment before 08:15 (08:14:59): returns the guest and when.
fn set_down_before_school(
    real: &Buffer,
    view: &IdleView,
    at: (i32, i32),
    graphics: bool,
) -> (Guest, u64) {
    let mut guest = home_at(
        4,
        tue(8, 14),
        &[(Furniture::Sofa, Nook::Users, 300)],
        graphics,
    );
    let mut now = until_visiting(&mut guest, real, view, 0);
    let school = real_of(&guest, now, tue(8, 15));
    let placed = school - 1000 / CLOCK_SPEED;
    while now < placed {
        now += (placed - now).min(500);
        guest.advance(now);
        paint(&mut guest, real, view, now);
    }
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.place(at.0, at.1, now);
    paint(&mut guest, real, view, now);
    (guest, now)
}

/// Out of her bed at school time (cued to sleep in it at 08:12, on
/// [`rooms_frame`] at 100×30, her bed on Users): she gets up, says one
/// set-off line, walks to her door's spot (Users' right wall, worked out
/// from the nook) and opens it there, its box meeting none of her
/// pieces. Before step 4a, the door opened at her seat.
#[test]
fn out_from_her_bed_she_walks_to_her_door() {
    let (real, view) = rooms_frame(100, 30);
    let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(8, 12),
            &[(Furniture::Bed, Nook::Users, 300)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        guest.cue(Scene::Sleep);
        let school = real_of(&guest, now, tue(8, 15));
        // In her bed as school begins (the precondition).
        let mut seat = None;
        while now < school {
            shell_step(&mut guest, &real, &view, &mut now, true);
            if let State::Visiting(visit) = &guest.state
                && now < school
            {
                seat = visit
                    .osaka
                    .use_span()
                    .filter(|(seat, ..)| seat.piece == room::PieceRef::Real(Furniture::Bed))
                    .map(|_| (visit.osaka.x, visit.osaka.y));
            }
        }
        let seat = seat.unwrap_or_else(|| panic!("{at}: in her bed as school began"));
        let seen = way_out(&mut guest, &real, &view, &mut now, 90_000);
        let (door, stood, covers) = seen
            .opened
            .clone()
            .unwrap_or_else(|| panic!("{at}: her door never opened: {seen:?}"));
        assert_eq!(door.spot(), space, "{at}: her door in its space");
        assert_eq!(stood, space, "{at}: she stood at its spot");
        assert_ne!(seat, space, "{at}: she was elsewhere (in her bed)");
        assert!(seen.walking > 0, "{at}: she walked there: {seen:?}");
        let her = room::her_box(space.0, space.1).unwrap();
        for cover in covers {
            assert!(
                !cover.intersects(her),
                "{at}: her door {her:?} on {cover:?}"
            );
        }
        assert_eq!(
            seen.off_lines + usize::from(seen.late),
            1,
            "{at}: one set-off line: {seen:?}"
        );
        assert!(!seen.early, "{at}");
    }
}

/// Two boxes on a 100×30 screen: List low on the left, Users high on the
/// right at the screen's right edge (her door's wall). `around`: List
/// reaches the left edge, so her way between them is round the screen's
/// edge; else no route at all (a door in space).
fn two_floors(around: bool) -> (Buffer, IdleView, (i32, i32)) {
    let list = if around {
        Rect::new(0, 10, 45, 17)
    } else {
        Rect::new(5, 14, 35, 13)
    };
    let users = Rect::new(55, 0, 45, 12);
    let real = boxes_screen(&[list, users]);
    let view = IdleView {
        nooks: vec![(Nook::List, list), (Nook::Users, users)],
        ..view(bottom_strip(100, 30))
    };
    (real, view, (20, i32::from(list.bottom()) - 1))
}

/// Her walk to her door across floors (an `Around` route, and no route
/// at all) never ends the visit before her external door opens (M1):
/// she's visiting until then, through the door in space or the trip off
/// the screen on her way. Red against a build that sets `leaving` as
/// she sets off. Both modes.
#[test]
fn a_leave_walk_across_floors_never_ends_the_visit_before_her_door() {
    for around in [false, true] {
        let (real, view, start) = two_floors(around);
        let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
        for graphics in [false, true] {
            let at = format!("around={around} graphics={graphics}");
            let (mut guest, mut now) = set_down_before_school(&real, &view, start, graphics);
            let seen = way_out(&mut guest, &real, &view, &mut now, 90_000);
            if around {
                assert!(seen.around, "{at}: round the edge: {seen:?}");
            } else {
                assert!(seen.spaced, "{at}: a door in space: {seen:?}");
            }
            assert!(!seen.early, "{at}: out before her door: {seen:?}");
            let (door, stood, _) = seen
                .opened
                .clone()
                .unwrap_or_else(|| panic!("{at}: her door: {seen:?}"));
            assert_eq!((door.spot(), stood), (space, space), "{at}");
        }
    }
}

/// Her line as she sets off is said once (T11; a guard): re-entering her
/// way out (a landing, the door in space between floors) says nothing.
/// The precondition: her away reflex was taken at least twice (the
/// set-off, and at least one re-entry).
#[test]
fn her_line_as_she_sets_off_is_said_once() {
    for around in [false, true] {
        let (real, view, start) = two_floors(around);
        for graphics in [false, true] {
            let at = format!("around={around} graphics={graphics}");
            let (mut guest, mut now) = set_down_before_school(&real, &view, start, graphics);
            let seen = way_out(&mut guest, &real, &view, &mut now, 90_000);
            // `reentries` counts the set-off itself too: at least one
            // re-entry (the door in space between floors gives exactly
            // one).
            assert!(seen.reentries >= 2, "{at}: re-entered: {seen:?}");
            assert_eq!(seen.off_lines + usize::from(seen.late), 1, "{at}: {seen:?}");
        }
    }
}

/// A resize on her walk to her door that moves its space (Users' right
/// wall, 100 wide → 90 wide, [`rooms_frame`]): her door opens at the new
/// spot (M8).
#[test]
fn a_resize_mid_walk_moves_where_she_goes_out() {
    let (real, view) = rooms_frame(100, 30);
    let (small, small_view) = rooms_frame(90, 30);
    let old = space_spot(&real, &view, Nook::Users, room::Side::Right);
    let new = space_spot(&small, &small_view, Nook::Users, room::Side::Right);
    assert_ne!(old, new, "the spot moves");
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = set_down_before_school(&real, &view, (55, 12), graphics);
        // On her way, halfway.
        loop {
            assert!(now < 120_000, "{at}: never on her way");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if osaka.act_summary().contains("her door") && osaka.x >= 70 {
                break;
            }
        }
        let seen = way_out(&mut guest, &small, &small_view, &mut now, 90_000);
        let (door, stood, _) = seen.opened.unwrap_or_else(|| panic!("{at}: her door"));
        assert_eq!((door.spot(), stood), (new, new), "{at}");
        assert_eq!(
            guest.closed_door().map(door::DoorSpot::spot),
            Some(new),
            "{at}"
        );
    }
}

/// The stage's school scene (D10): visiting throughout, she walks to her
/// door, out through it at its space, and back in out of it with "I'm
/// home!" (the visit never ends).
#[test]
fn a_school_scene_comes_back_after_its_gap() {
    let (real, view) = rooms_frame(100, 30);
    let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(15, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        guest.cue(Scene::School);
        let (mut opened, mut hidden, mut home) = (None, false, false);
        let end = now + 90_000;
        while !home {
            assert!(now < end, "{at}: never home: {opened:?} {hidden}");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting throughout");
            };
            let osaka = &visit.osaka;
            if let Some(osaka::Through::Home(door)) = osaka.through() {
                opened.get_or_insert(door.spot());
            }
            hidden |= opened.is_some() && osaka.hidden(now);
            if hidden && !osaka.hidden(now) {
                assert_eq!((osaka.x, osaka.y), space, "{at}: out of her door");
            }
            home = hidden && says(osaka.appearance(now).2, mind::HOME);
        }
        assert_eq!(opened, Some(space), "{at}");
    }
}

/// Her door moves mid-gap (a resize, the stage's school scene): she
/// comes back out where it stands now (C5, T20; the precondition: the
/// spot moves).
#[test]
fn her_door_follows_a_resize_mid_gap() {
    let (real, view) = rooms_frame(100, 30);
    let (small, small_view) = rooms_frame(90, 30);
    let old = space_spot(&real, &view, Nook::Users, room::Side::Right);
    let new = space_spot(&small, &small_view, Nook::Users, room::Side::Right);
    assert_ne!(old, new, "the spot moves");
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(15, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        guest.cue(Scene::School);
        // Through her door, out of sight.
        loop {
            assert!(now < 120_000, "{at}: never through");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if matches!(osaka.through(), Some(osaka::Through::Home(_))) && osaka.hidden(now) {
                break;
            }
        }
        let back = now;
        loop {
            assert!(now < back + 30_000, "{at}: never back");
            shell_step(&mut guest, &small, &small_view, &mut now, true);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting throughout");
            };
            if !visit.osaka.hidden(now) {
                assert_eq!((visit.osaka.x, visit.osaka.y), new, "{at}: out of it");
                break;
            }
        }
    }
}

/// Evicted on her way out (a resident's focused pane; F21): a pane not
/// over her leaves her walk be; one over her moves her out of it, and
/// her routine sends her on, silently (one set-off line in all), out
/// through her door in the end.
#[test]
fn evicted_on_her_way_out_she_walks_on() {
    let (w, h) = (100, 30);
    let real = rooms(w, h);
    let quiet = resident_view(w, h, None);
    let panes = nooks(w, h);
    let (users, playlist) = (panes[1].1, panes[2].1);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = set_down_before_school(&real, &quiet, (55, 12), graphics);
        loop {
            assert!(now < 120_000, "{at}: never on her way");
            shell_step(&mut guest, &real, &quiet, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if osaka.act_summary().contains("her door") && osaka.x >= 60 {
                break;
            }
        }
        // Playlist focused, under her: she walks on.
        let elsewhere = resident_view(w, h, Some(playlist));
        let was = visit_of(&guest).osaka.x;
        for _ in 0..3 {
            shell_step(&mut guest, &real, &elsewhere, &mut now, true);
        }
        let osaka = &visit_of(&guest).osaka;
        assert!(
            osaka.act_summary().contains("her door"),
            "{at}: {}",
            osaka.act_summary()
        );
        assert!(osaka.x > was, "{at}: on she walks");
        // Users focused, over her: out of it by a door in space, then on
        // to her door (now outside the focused pane).
        let over = resident_view(w, h, Some(users));
        shell_step(&mut guest, &real, &over, &mut now, true);
        let osaka = &visit_of(&guest).osaka;
        assert!(
            matches!(osaka.through(), Some(osaka::Through::Space(_))),
            "{at}: out of the pane: {}",
            osaka.act_summary()
        );
        let seen = way_out(&mut guest, &real, &over, &mut now, 90_000);
        let (door, ..) = seen
            .opened
            .clone()
            .unwrap_or_else(|| panic!("{at}: her door: {seen:?}"));
        assert!(!osaka::box_meets(users, door.spot()), "{at}: {door:?}");
        assert_eq!(seen.off_lines + usize::from(seen.late), 1, "{at}: {seen:?}");
    }
}

/// A dash's way out crossing 12:45 (Open choice 5): on her walk to her
/// door as school ends, she reaches her door's spot and, school being
/// over, stays in, silently (her door never opens, no set-off line, no
/// "Late, late, late!", no homecoming), visiting on. This proves the re-check at her door; that a boundary
/// never cuts her way out (T11) is `a_later_boundary_never_cuts_her_way_out`'s
/// (osaka.rs): 12:45 reached by skipping her clock cuts no walk either way.
#[test]
fn a_dash_out_as_school_ends_stays_in_at_her_door() {
    let (real, view) = home_screen();
    let space = space_spot(&real, &view, Nook::Users, room::Side::Left);
    let seed = (0..10_000)
        .find(|&seed| brain::dash(seed, 1).is_none())
        .expect("a Tuesday without one");
    let pieces = [
        (Furniture::Fridge, Nook::Playlist, 800),
        (Furniture::Sofa, Nook::Users, 600),
    ];
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(seed, tue(12, 30), &pieces, graphics);
        let mut now = until_visiting_or_away_here(&mut guest, &real, &view);
        // For her lunch, at her fridge across the room from her door.
        guest.cue(Scene::DashIn);
        now += 1;
        paint(&mut guest, &real, &view, now);
        // On her way out again, a way from her door still.
        loop {
            assert!(now < 120_000, "{at}: never on her way out");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: out before school ended");
            };
            let osaka = &visit.osaka;
            if osaka.act_summary().contains("her door") && (osaka.x - space.0).abs() > 10 {
                break;
            }
        }
        let said = visit_of(&guest).osaka.said_lines().len();
        guest.skip_clock(now);
        assert_eq!(
            guest.clock_label(now).as_deref(),
            Some("Tue 12:45 Afternoon")
        );
        let mut there = false;
        let end = now + 30_000;
        while now < end {
            shell_step(&mut guest, &real, &view, &mut now, true);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting on");
            };
            let osaka = &visit.osaka;
            there |= (osaka.x, osaka.y) == space;
            assert!(
                !matches!(osaka.through(), Some(osaka::Through::Home(_))),
                "{at}: her door opened"
            );
            // "Late, late, late!" isn't pooled (`said_lines` below).
            assert_ne!(
                osaka.appearance(now).2,
                Some(Bubble::Say(osaka::LATE)),
                "{at}: late"
            );
            assert_eq!(osaka.leaving(), None, "{at}");
            if !there {
                assert!(
                    osaka.act_summary().contains("her door"),
                    "{at}: cut: {}",
                    osaka.act_summary()
                );
            }
        }
        assert!(there, "{at}: she reached her door");
        let osaka = &visit_of(&guest).osaka;
        assert!(
            !osaka.said_lines()[said..]
                .iter()
                .any(|(pool, ..)| *pool == mind::OFF.id || *pool == mind::HOME.id),
            "{at}: silently: {:?}",
            &osaka.said_lines()[said..]
        );
    }
}

/// From a cold start until she's visiting or her home stands empty.
fn until_visiting_or_away_here(guest: &mut Guest, real: &Buffer, view: &IdleView) -> u64 {
    let mut now = 0;
    paint(guest, real, view, now);
    while !matches!(guest.state, State::Visiting(_) | State::Away(_)) {
        assert!(now < 60_000, "never visiting nor away");
        shell_step(guest, real, view, &mut now, true);
    }
    now
}

// ---- Her way out: the step 4a review's tests ----

/// A dash's way out cut short after 12:45 (a chat line on her walk, the
/// step 4a review): her set-off is let go with school, so she's not on
/// her way out (a parcel may come), and the next school morning she says
/// her set-off line again, once. Red with the latch kept past school.
#[test]
fn a_way_out_cut_after_school_says_its_line_next_morning() {
    let (real, view) = home_screen();
    let space = space_spot(&real, &view, Nook::Users, room::Side::Left);
    let seed = (0..10_000)
        .find(|&seed| brain::dash(seed, 1).is_none())
        .expect("a Tuesday without one");
    let pieces = [
        (Furniture::Fridge, Nook::Playlist, 800),
        (Furniture::Sofa, Nook::Users, 600),
        (Furniture::Bed, Nook::Playlist, 200),
    ];
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(seed, tue(12, 30), &pieces, graphics);
        let mut now = until_visiting_or_away_here(&mut guest, &real, &view);
        guest.cue(Scene::DashIn);
        now += 1;
        paint(&mut guest, &real, &view, now);
        loop {
            assert!(now < 120_000, "{at}: never on her way out");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if osaka.act_summary().contains("her door") && (osaka.x - space.0).abs() > 10 {
                break;
            }
        }
        guest.skip_clock(now);
        // A chat line on her way: she looks, and decides afresh.
        let mut chatty = view.clone();
        chatty.chat_mark.synced += 1;
        shell_step(&mut guest, &real, &chatty, &mut now, true);
        let cut = now;
        loop {
            assert!(now < cut + 30_000, "{at}: never off her way");
            shell_step(&mut guest, &real, &chatty, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if !osaka.act_summary().contains("her door") && osaka.act_name() != "Look" {
                break;
            }
        }
        let osaka = &visit_of(&guest).osaka;
        assert!(!osaka.on_her_way_out(), "{at}: {}", osaka.act_summary());
        let offs = |guest: &Guest| {
            visit_of(guest)
                .osaka
                .said_lines()
                .iter()
                .filter(|(pool, ..)| *pool == mind::OFF.id)
                .count()
        };
        let before = offs(&guest);
        // On to the next school morning, visiting on (the dash home is a
        // visit after all), then out: her lines as she sets off.
        let (mut said, mut late) = (before, false);
        while matches!(guest.state, State::Visiting(_)) {
            assert!(now < cut + 600_000, "{at}: {:?}", guest.clock_label(now));
            // Skipped boundary by boundary to Wednesday's school time,
            // then on until she's out.
            if guest.clock_label(now).is_some_and(|l| !l.ends_with("Away")) {
                guest.skip_clock(now);
            }
            let step = now;
            while now < step + 10_000 && matches!(guest.state, State::Visiting(_)) {
                shell_step(&mut guest, &real, &chatty, &mut now, true);
                if let State::Visiting(visit) = &guest.state {
                    said = offs(&guest);
                    late |= visit.osaka.appearance(now).2 == Some(Bubble::Say(osaka::LATE));
                }
            }
        }
        let out = guest.clock_label(now).unwrap_or_default();
        assert!(out.starts_with("Wed 08:"), "{at}: out at {out}");
        assert_eq!(
            said - before + usize::from(late),
            1,
            "{at}: one set-off line"
        );
    }
}

/// Nothing is delivered on her way out to school (the step 4a review):
/// a parcel ordered once she has set off for her door doesn't come (no
/// flap, no "A parcel!") before she's out; it waits for her, and comes
/// once she's home at 12:45 (the precondition: it could come here).
#[test]
fn no_parcel_comes_on_her_way_out() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = set_down_before_school(&real, &view, (55, 12), graphics);
        let mut ordered = false;
        while matches!(guest.state, State::Visiting(_)) {
            assert!(now < 120_000, "{at}: never out");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let State::Visiting(visit) = &guest.state else {
                break;
            };
            assert!(visit.flap.is_none(), "{at}: a flap on her way out");
            assert_ne!(
                visit.osaka.appearance(now).2,
                Some(Bubble::Say(PARCEL)),
                "{at}: on her way out"
            );
            if !ordered && visit.osaka.act_summary().contains("her door") {
                assert!(visit.osaka.on_her_way_out(), "{at}");
                guest.send_parcel();
                ordered = true;
            }
        }
        assert!(ordered, "{at}: she set off");
        assert!(guest.ledger.ordered.is_some(), "{at}: it waits");
        guest.skip_clock(now);
        let home = now;
        let mut parcel = false;
        while !parcel {
            assert!(now < home + 60_000, "{at}: no parcel once home");
            shell_step(&mut guest, &real, &view, &mut now, true);
            if let State::Visiting(visit) = &guest.state {
                parcel = visit.osaka.appearance(now).2 == Some(Bubble::Say(PARCEL));
            }
        }
    }
}

/// A focused pane over her door's spot on her walk to it (not over her;
/// the step 4a review): she goes where the frame stands it now, outside
/// the pane, and never walks into the pane (no eviction on her way). A
/// guard: the pane's cells are no floor to her, so even a walk on to the
/// old spot would end at its edge (and re-enter her routine); each step
/// following the frame's door is `her_walk_to_her_door_follows_the_frames_door`'s.
#[test]
fn a_pane_focused_over_her_door_on_her_way_turns_her_aside() {
    let (w, h) = (100, 30);
    let real = rooms(w, h);
    let quiet = resident_view(w, h, None);
    let focus = Rect::new(84, 0, 16, 13);
    let focused = resident_view(w, h, Some(focus));
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = set_down_before_school(&real, &quiet, (55, 12), graphics);
        loop {
            assert!(now < 120_000, "{at}: never on her way");
            shell_step(&mut guest, &real, &quiet, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if osaka.act_summary().contains("her door") && osaka.x >= 60 {
                assert!(!osaka::box_meets(focus, (osaka.x, osaka.y)), "{at}");
                break;
            }
        }
        let mut opened = None;
        while opened.is_none() {
            assert!(now < 240_000, "{at}: her door never opened");
            shell_step(&mut guest, &real, &focused, &mut now, true);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: out before her door opened");
            };
            let osaka = &visit.osaka;
            assert!(
                !osaka::box_meets(focus, (osaka.x, osaka.y)),
                "{at}: in the focused pane at {:?}: {}",
                (osaka.x, osaka.y),
                osaka.act_summary()
            );
            // Never moved out of it (an eviction is a door in space).
            assert!(
                !matches!(osaka.through(), Some(osaka::Through::Space(_))),
                "{at}: evicted: {}",
                osaka.act_summary()
            );
            if let Some(osaka::Through::Home(door)) = osaka.through() {
                opened = Some(door.spot());
            }
        }
        let spot = opened.unwrap();
        assert!(!osaka::box_meets(focus, spot), "{at}: her door at {spot:?}");
    }
}

/// An errand on her walk to her door (the step 4a review, D6): it lets
/// her set-off go, so setting off again after the poke she says her line
/// again (two in all), and goes out through her door.
#[test]
fn an_errand_on_her_way_out_has_her_set_off_again() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now) = set_down_before_school(&real, &view, (55, 12), graphics);
        loop {
            assert!(now < 120_000, "{at}: never on her way");
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if osaka.act_summary().contains("her door") && osaka.x >= 60 {
                break;
            }
        }
        // On List's floor.
        let accordion = Rect::new(10, 26, 20, 1);
        assert!(guest.send(&real, &view, accordion, now), "{at}: sent");
        let seen = way_out(&mut guest, &real, &view, &mut now, 120_000);
        assert!(seen.opened.is_some(), "{at}: {seen:?}");
        assert_eq!(seen.off_lines + usize::from(seen.late), 2, "{at}: {seen:?}");
    }
}

/// With no door anywhere and her in her bed as school begins (the door
/// batch's M25 through the guest; the step 4a review): she steps out of
/// her bed and goes out by a door in space where her box meets none of
/// her pieces ([`long_visit_of`] checks every frame of it), for good.
#[test]
fn with_no_door_anywhere_she_steps_out_of_her_bed_to_go_out() {
    let (real, view) = no_door_but_floor_outside_the_chat();
    for graphics in [false, true] {
        let mut guest = Guest::restore(Ledger::new_at(4, tue(8, 12)));
        guest.set_date(date(2026, 6, 17));
        let visited = long_visit_of(
            guest,
            graphics,
            |_, _| (real.clone(), view.clone()),
            &[(100, 30)],
            &[],
            &[],
            &[],
            (0, 0, 1, 1),
            &[(Furniture::Bed, 1, 300, false)],
            Run {
                cue: Some(Scene::Sleep),
                out_every: Some(500),
                ..Run::default()
            },
            90_000,
        )
        .unwrap_or_else(|e| panic!("graphics={graphics}: {e}"));
        assert!(visited.space_doors > 0, "graphics={graphics}: {visited:?}");
        assert!(visited.away > 0, "graphics={graphics}: {visited:?}");
        assert_eq!(visited.out_doors, 0, "graphics={graphics}: no door of hers");
    }
}

/// A focused pane is never a reason to move the door she's through
/// (D6; the step 4a review): mid-gap on the stage's school scene, a
/// pane focused over her door's wall (not over her box, so she isn't
/// moved out of it) puts the frame's door at the fallback (the
/// precondition; face-on, here at the same spot), but she comes back
/// out of the door she went in by, in its wall.
#[test]
fn a_focused_pane_never_moves_the_door_shes_through() {
    let (w, h) = (100, 30);
    let real = rooms(w, h);
    let quiet = resident_view(w, h, None);
    let wall = Rect::new(99, 0, 1, 13);
    let focused = resident_view(w, h, Some(wall));
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(15, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &quiet, 0);
        guest.cue(Scene::School);
        let door = loop {
            assert!(now < 120_000, "{at}: never through");
            shell_step(&mut guest, &real, &quiet, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if let Some(osaka::Through::Home(door)) = osaka.through()
                && osaka.hidden(now)
            {
                break door;
            }
        };
        assert!(door.wall().is_some(), "{at}: in its space: {door:?}");
        assert!(!osaka::box_meets(wall, door.spot()), "{at}");
        let back = now;
        loop {
            assert!(now < back + 30_000, "{at}: never back");
            shell_step(&mut guest, &real, &focused, &mut now, true);
            let visit = visit_of(&guest);
            assert_ne!(
                visit.door,
                Some(door),
                "{at}: the frame's door, off its space"
            );
            if !visit.osaka.hidden(now) {
                assert_eq!(
                    visit.osaka.through(),
                    Some(osaka::Through::Home(door)),
                    "{at}"
                );
                assert_eq!((visit.osaka.x, visit.osaka.y), door.spot(), "{at}");
                break;
            }
        }
    }
}

// ---- Work through her door (door batch, step 4b) ----

/// From `now`, stepped as the shell would on `real`/`view` until she has
/// come back out of the door she's through and it has run its course
/// (at most `bound` ms on), checking each frame she's seen at its far
/// side with `there`: when, and how many such frames there were.
fn back_out_of_her_door(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    mut now: u64,
    bound: u64,
    mut there: impl FnMut(&Visit, u64),
) -> (u64, usize) {
    let from = now;
    let (mut out, mut seen) = (false, 0);
    while visit_of(guest).osaka.through().is_some() {
        assert!(now < from + bound, "never back");
        shell_step(guest, real, view, &mut now, true);
        let visit = visit_of(guest);
        let hidden = visit.osaka.hidden(now);
        out |= hidden;
        if out && !hidden && visit.osaka.through().is_some() {
            seen += 1;
            there(visit, now);
        }
    }
    (now, seen)
}

/// Her part-time job goes out through her door, at its space, and comes
/// home out of it (door batch, step 4b): she walks from her sofa to it,
/// goes through it for her shift (a minute or more out of sight), and
/// comes back out of it facing the room, carrying her shopping (C13),
/// home from work. In both drawing modes, with and without text in her
/// panes (her door stands over text).
#[test]
fn work_goes_out_by_her_door() {
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let sofa = [(Furniture::Sofa, Nook::Users, 300)];
            let mut guest = home_at(3, sat(11, 0), &sofa, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            paint(&mut guest, &real, &view, now);
            let space = space_spot(&real, &view, Nook::Users, room::Side::Left);
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            let door = visit.door.expect("her door");
            assert_eq!(door.spot(), space, "{at}: her door at its space");
            // Along her floor from it (by her sofa).
            let (x, y) = (space.0 + 25, space.1);
            assert!(
                visit.terrain.platform_at(x, y).is_some(),
                "{at}: on her floor"
            );
            visit.osaka.place(x, y, now);
            let (chances, terrain) = (visit.chances.clone(), visit.terrain.clone());
            assert_eq!(chances.door, Some(door), "{at}");
            visit.osaka.go_to_work(&terrain, &chances, now, &mut Rng(1));
            let from = now;
            let mut walked = false;
            while visit_of(&guest).osaka.through().is_none() {
                assert!(now < from + 60_000, "{at}: never at her door");
                walked |= visit_of(&guest).osaka.act_name() == "Walk";
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            assert!(walked, "{at}: she walked to it");
            let osaka = &visit_of(&guest).osaka;
            assert_eq!(osaka.through(), Some(osaka::Through::Home(door)), "{at}");
            assert_eq!((osaka.x, osaka.y), space, "{at}: through it at its spot");
            let went = now;
            let (back, seen) =
                back_out_of_her_door(&mut guest, &real, &view, now, 200_000, |visit, t| {
                    let osaka = &visit.osaka;
                    assert_eq!((osaka.x, osaka.y), space, "{at} t={t}: out of it");
                    assert_eq!(osaka.facing, door.into_room(), "{at} t={t}: into the room");
                    assert!(
                        matches!(osaka.appearance(t).0, sprite::Pose::Carry(_)),
                        "{at} t={t}: her shopping"
                    );
                });
            assert!(seen > 0, "{at}: seen coming out");
            assert!(back >= went + 60_000, "{at}: a shift: {}", back - went);
            assert_eq!(
                visit_of(&guest).osaka.act_name(),
                "Home",
                "{at}: home from work"
            );
        }
    }
}

/// Her door's pane focused while she's out at work (door batch, step 4b,
/// test 3): the frame's door goes to the fallback for the focus (the
/// precondition), but the door she's through stays where it stood, in
/// its space, hidden (none opens anywhere else); the focus gone, she
/// comes home out of it there.
#[test]
fn focusing_her_doors_pane_in_a_work_gap_never_moves_it() {
    let (w, h) = (100, 30);
    let real = rooms(w, h);
    let quiet = resident_view(w, h, None);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let sofa = [(Furniture::Sofa, Nook::Users, 300)];
        let mut guest = home_at(4, sat(11, 0), &sofa, graphics);
        let mut now = until_visiting(&mut guest, &real, &quiet, 0);
        guest.cue(Scene::Work);
        let from = now;
        let door = loop {
            assert!(now < from + 60_000, "{at}: never out");
            shell_step(&mut guest, &real, &quiet, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            if let Some(osaka::Through::Home(door)) = osaka.through()
                && osaka.hidden(now)
                && osaka.door(now).is_none()
            {
                break door;
            }
        };
        assert!(door.wall().is_some(), "{at}: in its space: {door:?}");
        let pane = quiet
            .nooks
            .iter()
            .map(|&(_, r)| r)
            .find(|r| {
                r.contains(Position::new(
                    door.spot().0 as u16,
                    door.spot().1 as u16 - 1,
                ))
            })
            .expect("her door's pane");
        let focused = resident_view(w, h, Some(pane));
        let until = now + 5_000;
        let mut fallbacks = 0;
        while now < until {
            shell_step(&mut guest, &real, &focused, &mut now, true);
            let frame = paint(&mut guest, &real, &focused, now);
            let visit = visit_of(&guest);
            assert_ne!(
                visit.door,
                Some(door),
                "{at}: the frame's door, off its space"
            );
            // No door of hers appears at the frame's fallback while she's
            // out through the one in its space: nothing of it is drawn in
            // the fallback's box (step 8 draws the shut door at the door
            // she's through).
            if let Some(fallback) = visit.door {
                fallbacks += 1;
                let (fx, fy) = fallback.spot();
                for y in fy - sprite::HEIGHT..fy {
                    for x in fx - sprite::WIDTH / 2..=fx + sprite::WIDTH / 2 {
                        let cell = (x as u16, y as u16);
                        assert_eq!(
                            frame[cell].symbol(),
                            real[cell].symbol(),
                            "{at} t={now}: drawn at the fallback {cell:?}"
                        );
                    }
                }
            }
            assert_eq!(
                visit.osaka.through(),
                Some(osaka::Through::Home(door)),
                "{at}: the door she's through never moves"
            );
            assert!(visit.osaka.hidden(now), "{at}: out");
        }
        assert!(fallbacks > 0, "{at}: the frame's door stood at a fallback");
        let (_, seen) =
            back_out_of_her_door(&mut guest, &real, &quiet, now, 200_000, |visit, t| {
                assert_eq!(
                    visit.osaka.through(),
                    Some(osaka::Through::Home(door)),
                    "{at} t={t}"
                );
                assert_eq!((visit.osaka.x, visit.osaka.y), door.spot(), "{at} t={t}");
            });
        assert!(seen > 0, "{at}: seen coming home");
    }
}

/// A lamp (with her sofa, TV and desk) filling her door's space, so her
/// door stands at the nearest floor that meets no piece (the space
/// yields): coming home from work out of it, she has bumped into it
/// (door batch M13, for step 6's DoorClear), from the moment she's back
/// out, never before. Through her door in its space, never.
#[test]
fn a_lamp_in_the_space_is_bumped_into_coming_home_from_work() {
    use crate::ui::houseguest::door::{Fallback, Set};
    let sofa = [(Furniture::Sofa, Nook::Users, 300)];
    let full: &[(Furniture, Nook, u16)] = &FULL_SPACE_HOME;
    type Case<'a> = (
        &'a str,
        (Buffer, IdleView),
        &'a [(Furniture, Nook, u16)],
        bool,
    );
    let cases: [Case; 2] = [
        ("yielded", chat_by_a_full_space(), full, true),
        (
            "kept",
            (rooms(100, 30), resident_view(100, 30, None)),
            &sofa,
            false,
        ),
    ];
    for (name, (real, view), pieces, bumps) in cases {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(3, sat(11, 0), pieces, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            paint(&mut guest, &real, &view, now);
            let door = visit_of(&guest).door.expect("her door");
            assert_eq!(
                door.set() == Set::Floor(Fallback::Yield),
                bumps,
                "{at}: {door:?}"
            );
            guest.cue(Scene::Work);
            let from = now;
            while visit_of(&guest).osaka.through().is_none() {
                assert!(now < from + 60_000, "{at}: never out");
                assert!(!visit_of(&guest).osaka.bumped(), "{at}: on her way");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let (mut out, mut seen, mut t) = (false, 0, now);
            while visit_of(&guest).osaka.through().is_some() {
                assert!(t < now + 200_000, "{at}: never back");
                shell_step(&mut guest, &real, &view, &mut t, true);
                let osaka = &visit_of(&guest).osaka;
                let hidden = osaka.hidden(t);
                out |= hidden;
                if out && !hidden && osaka.through().is_some() {
                    seen += 1;
                    assert_eq!(osaka.bumped(), bumps, "{at} t={t}: back out");
                } else if osaka.through().is_some() && (!out || osaka.door(t).is_none()) {
                    // On her way through, or out of sight between its
                    // doors.
                    assert!(!osaka.bumped(), "{at} t={t}: before she's back");
                }
            }
            assert!(seen > 0, "{at}: seen coming home");
            assert_eq!(visit_of(&guest).osaka.bumped(), bumps, "{at}: home");
        }
    }
}

// ---- No furniture in the chat pane (door batch, step 5), but her
// makeshift pieces may stand there (step 6m) ----

/// [`by_playlists_right_wall`] with the chat pane laid over Playlist's
/// right end (columns 90..=99, a custom layout's grid overlapping it),
/// where every makeshift sofa she could make there would stand.
fn chat_by_playlists_right_wall() -> (Buffer, IdleView, Rect) {
    let (real, mut view, _) = by_playlists_right_wall();
    let chat = Rect::new(90, 13, 10, 14);
    view.chat = chat;
    (real, view, chat)
}

/// [`by_playlists_right_wall`] with the chat pane laid over Playlist's
/// right half, its one line of text and all (columns 78..=99): as in the
/// bundled layout, the only text she could tear is chat text.
fn chat_text_by_playlists_right_wall() -> (Buffer, IdleView, Rect) {
    let (real, mut view, _) = by_playlists_right_wall();
    let chat = Rect::new(78, 13, 22, 14);
    view.chat = chat;
    (real, view, chat)
}

/// The makeshift pieces of `guest`'s visit meeting the chat pane `chat`.
fn made_in_chat(guest: &Guest, chat: Rect) -> Vec<Rect> {
    visit_of(guest)
        .made
        .iter()
        .map(|m| m.piece.cover())
        .filter(|&c| room::in_chat(chat, c))
        .collect()
}

/// She makes furniture of chat text, and it may stand in the chat pane
/// (the user, 2026-10-09: "Allow makeshift in chat": it lasts one visit,
/// like a carried prop): cued to make a sofa where the only text she
/// could tear is in the chat, she makes one there, and it stands, its
/// text torn off and drawn as hers (the chat pane's closet never hides
/// one: by construction, InChat and the closet read only the home's laid
/// pieces, and a made piece has no strip and a scrap). Her door's space
/// still refuses one: here it's on Users' right wall, far from the
/// text, so the per-frame check is only a guard; the space's own cases
/// are [`she_never_makes_a_piece_in_her_door_space`] and
/// [`a_scrap_in_her_door_space_makes_it_fall_back`]. Through the
/// visiting frame.
#[test]
fn she_makes_furniture_from_chat_text() {
    let (real, view, chat) = chat_text_by_playlists_right_wall();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            3,
            tue(14, 0),
            &[(Furniture::Sofa, Nook::Users, 0)],
            graphics,
        );
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        assert_ne!(guest.ledger.home.door, Some(PLAYLIST_RIGHT), "{at}");
        guest.cue(Scene::MakeSofa);
        let end = now + 30_000;
        let mut made = false;
        while now < end && !made {
            shell_step(&mut guest, &real, &view, &mut now, true);
            let space = door_space_of(&guest, &real, &view);
            for m in &visit_of(&guest).made {
                assert!(
                    space.is_none_or(|s| !s.intersects(m.piece.cover())),
                    "{at}: made {:?} in her door's space {space:?} at {now}",
                    m.piece.cover()
                );
            }
            made = !made_in_chat(&guest, chat).is_empty();
        }
        assert!(
            matches!(guest.cue_note(), Some(Ok(_))),
            "{at}: {:?}",
            guest.cue_note()
        );
        assert!(made, "{at}: nothing made in the chat");
        // It stands: on later frames it's still there, its text still
        // torn off its line.
        for _ in 0..20 {
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let frame = paint(&mut guest, &real, &view, now);
        let standing = visit_of(&guest)
            .made
            .iter()
            .filter(|m| room::in_chat(chat, m.piece.cover()))
            .collect::<Vec<_>>();
        assert!(!standing.is_empty(), "{at}: it fell apart in the chat");
        for m in standing {
            for &(x, y) in &m.torn {
                assert_ne!(frame[(x, y)], real[(x, y)], "{at}: {:?} torn off", (x, y));
            }
        }
    }
}

/// A made piece the chat pane comes to meet stands (D3 as amended by
/// the user, 2026-10-09: makeshift pieces may stand in the chat): her
/// sofa of text by Playlist's right wall, then the chat laid over it (a
/// layout changed under her): at the next frame it still stands, its
/// text still torn off. (Her door's space coming to meet one is
/// [`a_made_piece_her_door_space_comes_to_meet_falls_apart`].)
#[test]
fn a_made_piece_the_chat_comes_to_meet_stands() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now, real, view, _) = made_by_playlists_right_wall(graphics);
        let [made] = visit_of(&guest).made.as_slice() else {
            panic!("{at}: one piece");
        };
        let torn = made.torn.clone();
        let (_, chatted, chat) = chat_by_playlists_right_wall();
        assert!(room::in_chat(chat, made.piece.cover()), "{at}: under it");
        // It stands a frame first.
        now += 100;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        assert_eq!(visit_of(&guest).made.len(), 1, "{at}: it stands");
        now += 100;
        guest.advance(now);
        paint(&mut guest, &real, &chatted, now);
        assert_eq!(
            made_in_chat(&guest, chat).len(),
            1,
            "{at}: it stands in the chat: {:?}",
            visit_of(&guest).made
        );
        let frame = paint(&mut guest, &real, &chatted, now);
        for &(x, y) in &torn {
            assert_ne!(frame[(x, y)], real[(x, y)], "{at}: {:?} torn off", (x, y));
        }
    }
}

/// What she made stands, for a delivery's `seats`, as the home will be
/// once the parcel is in (`made_standing_then`): one in the chat pane
/// still stands (step 6m: the chat is no bar to it), and one the
/// delivery's new door space would meet doesn't (D3: judged on the new
/// space, not this frame's). Her sofa of text by Playlist's right wall,
/// her door on Users': the chat laid over it, then her home as a
/// delivery that put her door on Playlist's right wall would leave it.
/// (End to end through a doorstep this can't be told apart: a parcel's
/// box never meets a made piece's torn cells, `blocked` refuses them.)
#[test]
fn a_delivery_judges_what_she_made_on_its_new_door_space_not_the_chat() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (guest, _, real, view, space) = made_by_playlists_right_wall(graphics);
        let visit = visit_of(&guest);
        let [made] = visit.made.as_slice() else {
            panic!("{at}: one piece");
        };
        assert!(made.piece.cover().intersects(space), "{at}: by the wall");
        let (_, chatted, chat) = chat_by_playlists_right_wall();
        assert!(
            room::in_chat(chat, made.piece.cover()),
            "{at}: under the chat"
        );
        let moved: std::collections::HashSet<(u16, u16)> = visit.layer.cells().collect();
        let home = &guest.ledger.home;
        let standing = |then: &room::Home, view: &IdleView| {
            made_standing_then(
                &visit.made,
                &visit.layer,
                &real,
                &visit.shown,
                &moved,
                &view.protected,
                then,
                plan_of(view, &real),
            )
        };
        assert_eq!(
            standing(home, &chatted),
            vec![made.piece],
            "{at}: it stands in the chat"
        );
        let moved_door = room::Home {
            door: Some(PLAYLIST_RIGHT),
            ..home.clone()
        };
        assert_eq!(standing(home, &view), vec![made.piece], "{at}: it stands");
        assert!(
            standing(&moved_door, &view).is_empty(),
            "{at}: her door's new space takes it"
        );
    }
}

/// [`rooms_frame`] at 100×30 with the chat pane over List's floor by its
/// left wall (columns 0..=19, rows 14..=26: a custom layout's grid
/// overlapping List), the wall a parcel comes in by first (her door's,
/// [`LIST_LEFT`]).
fn chat_by_lists_left_wall() -> (Buffer, IdleView, Rect) {
    let (real, mut view) = rooms_frame(100, 30);
    let chat = Rect::new(0, 14, 20, 13);
    view.chat = chat;
    (real, view, chat)
}

/// Her door on List's left wall (the screen's edge).
const LIST_LEFT: room::DoorWall = room::DoorWall {
    strip: room::Strip::Bottom(Nook::List),
    side: room::Side::Left,
};

/// Her lamp on order, delivered on `view`: where its box stands once
/// it's in (her TV on Users, her door by List's left wall: a parcel
/// comes in through her door first).
fn a_lamp_delivered(real: &Buffer, view: &IdleView, graphics: bool) -> Shown {
    let mut guest = home_at(
        3,
        tue(14, 0),
        &[(Furniture::Tv, Nook::Users, 500)],
        graphics,
    );
    guest.ledger.home.door = Some(LIST_LEFT);
    guest.ledger.ordered = Some(Furniture::Lamp);
    guest.ledger.bought_on = 0;
    let mut now = until_visiting(&mut guest, real, view, 0);
    let end = now + 180_000;
    while !guest.ledger.home.owns(Furniture::Lamp) {
        assert!(now < end, "graphics={graphics}: never delivered");
        shell_step(&mut guest, real, view, &mut now, true);
    }
    paint(&mut guest, real, view, now);
    *visit_of(&guest)
        .shown
        .iter()
        .find(|s| s.item == Furniture::Lamp)
        .expect("its box shows")
}

/// A parcel is never left on a doorstep in the chat pane (D3): with the
/// chat over the floor by List's left wall (where it comes in with no
/// chat there: asserted), her lamp comes in by another wall, its box
/// clear of the chat.
#[test]
fn a_parcel_is_never_left_on_a_doorstep_in_the_chat() {
    let (real, view, chat) = chat_by_lists_left_wall();
    let apart = IdleView {
        chat: Rect::default(),
        ..view.clone()
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let there = a_lamp_delivered(&real, &apart, graphics);
        assert!(
            room::in_chat(chat, there.cover()),
            "{at}: with no chat it comes in there: {there:?}"
        );
        let elsewhere = a_lamp_delivered(&real, &view, graphics);
        assert!(
            !room::in_chat(chat, elsewhere.cover()),
            "{at}: left in the chat: {elsewhere:?}"
        );
    }
}

// ---- Her door side-on in its wall (door batch, step 8) ----

/// The wall door looks among `looks` (what was painted, in order).
fn wall_doors(looks: &[Look]) -> Vec<(art::WallDoor, art::Sky, u8)> {
    looks
        .iter()
        .filter_map(|look| match *look {
            Look::WallDoor { door, sky, cols } => Some((door, sky, cols)),
            _ => None,
        })
        .collect()
}

/// Her door's wall and floor row, standing in its space (asserted).
fn wall_of(door: door::DoorSpot) -> (room::Side, i32, i32) {
    let door::Set::Wall { side, wall } = door.set() else {
        panic!("not at its wall: {door:?}");
    };
    (side, wall, door.spot().1)
}

/// The columns `cols` wide at `wall` on `side`, toward the room (the
/// wall's own column one of them).
fn toward_room(side: room::Side, wall: i32, cols: i32) -> std::ops::RangeInclusive<i32> {
    match side {
        room::Side::Right => wall - cols + 1..=wall,
        room::Side::Left => wall..=wall + cols - 1,
    }
}

/// Whether `frame` changed the cell at `(x, y)` from `real`.
fn differs(frame: &Buffer, real: &Buffer, (x, y): (i32, i32)) -> bool {
    let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
        return false;
    };
    frame.cell((x, y)) != real.cell((x, y))
}

/// One wall's case: its name, the frame, its view, her pieces, and the
/// side her door's wall is on.
type WallCase = (
    &'static str,
    Buffer,
    IdleView,
    Vec<(Furniture, Nook, u16)>,
    room::Side,
);

/// Her empty home with her door on each wall: on [`home_screen`] by
/// Users' left wall (the screen's edge, column 0), on [`rooms_frame`]
/// by Users' right (column 99).
fn both_walls() -> [WallCase; 2] {
    let (left, left_view) = home_screen();
    let (right, right_view) = rooms_frame(100, 30);
    [
        ("left", left, left_view, HOME.to_vec(), room::Side::Left),
        (
            "right",
            right,
            right_view,
            vec![(Furniture::Sofa, Nook::Users, 300)],
            room::Side::Right,
        ),
    ]
}

/// While she's out her door stands side-on in its wall (door batch D8),
/// not face-on in her box: in line art one image of `Look::WallDoor`
/// shut with her slippers before it, over its own columns at the wall
/// (four, on a quiet space) and nothing else of the space; in ASCII the
/// shut door's two columns of glyphs, the wall's own `|` over the
/// border's line. A key rains it out, the wall's line back as it was.
/// Both walls, both modes.
#[test]
fn her_door_shows_side_on_in_the_wall() {
    for (name, real, view, pieces, side) in both_walls() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let (mut guest, now) = away_on(&real, &view, &pieces, graphics);
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.take_looks();
            }
            let frame = paint(&mut guest, &real, &view, now);
            let door = guest.closed_door().expect("her door");
            let (wall_side, wall, floor) = wall_of(door);
            assert_eq!(wall_side, side, "{at}: {door:?}");
            let shut = art::WallDoor::Shut {
                flap: 0,
                away: true,
            };
            let cols = if graphics {
                let looks = guest.graphics.as_mut().expect("line art").take_looks();
                assert!(
                    !looks.iter().any(|l| matches!(l, Look::Door(_))),
                    "{at}: no face-on door: {looks:?}"
                );
                let doors = wall_doors(&looks);
                assert_eq!(doors.len(), 1, "{at}: {looks:?}");
                assert_eq!(doors[0].0, shut, "{at}");
                i32::from(doors[0].2)
            } else {
                for (dx, dy, glyph) in sprite::wall_door_cells(shut, side) {
                    let (x, y) = (wall + dx, floor + dy);
                    let got = frame
                        .cell((x as u16, y as u16))
                        .map(|c| c.symbol().to_owned());
                    assert_eq!(got, Some(glyph.to_string()), "{at}: at ({x}, {y})");
                }
                2
            };
            assert_eq!(cols, if graphics { 4 } else { 2 }, "{at}: a quiet space");
            // Nothing else of her space is drawn over.
            let drawn = toward_room(side, wall, cols);
            let space = toward_room(side, wall, room::SPACE + 1);
            for y in floor - sprite::HEIGHT..=floor {
                for x in space.clone() {
                    let changed = differs(&frame, &real, (x, y));
                    if !drawn.contains(&x) {
                        assert!(!changed, "{at}: ({x}, {y}) outside its columns");
                    } else if y < floor && x != wall {
                        assert!(changed, "{at}: ({x}, {y}) of the door not drawn");
                    }
                }
            }
            // What it painted over its wall's line is kept for the rain
            // to put back: each of the door's cells in the wall's column,
            // over the border's own line.
            let empty = empty_of(&guest).expect("her home empty");
            for (dx, dy, glyph) in sprite::wall_door_cells(shut, side) {
                if dx != 0 {
                    continue;
                }
                let (x, y) = (wall as u16, (floor + dy) as u16);
                let kept = empty
                    .painted
                    .iter()
                    .find(|c| (c.x, c.y) == (x, y))
                    .unwrap_or_else(|| panic!("{at}: ({x}, {y}) not kept for the rain"));
                assert_eq!(
                    kept.under.symbol(),
                    real[(x, y)].symbol(),
                    "{at}: ({x}, {y})"
                );
                assert!(matches!(kept.under.symbol(), "│" | "┃"), "{at}: ({x}, {y})");
                if !graphics {
                    assert_eq!(kept.glyph, glyph, "{at}: ({x}, {y})");
                    assert!(
                        empty
                            .door
                            .as_ref()
                            .is_some_and(|d| d.cells.contains(&(x, y))),
                        "{at}: ({x}, {y}) not among its drawn cells"
                    );
                }
            }
            // A key: her home rains out, the wall's line back; every
            // frame of the rain has her door's glyph or the wall's line
            // in the wall's column, never a hole.
            guest.activity(now);
            let end = now + dissolve::DURATION_MS;
            let mut t = now;
            while t < end {
                t += 50;
                let frame = paint(&mut guest, &real, &view, t);
                for dy in -sprite::HEIGHT..0 {
                    let cell = (wall as u16, (floor + dy) as u16);
                    assert!(
                        !frame[cell].symbol().trim().is_empty(),
                        "{at}: at {t}: a hole in the wall at {cell:?}"
                    );
                }
            }
            let end = run(&mut guest, &real, &view, t, t + 1000);
            assert_eq!(end, real, "{at}: the rain restores the frame");
        }
    }
}

/// The one rule for a wall's own column (her door's glyphs there, a
/// parcel's flap): drawn only over a plain vertical line (`│`/`┃`),
/// never over anything else there (a corner, a title's letter), and
/// never in a protected cell (a flap may come in by a pane in use; her
/// door is hidden whole there before it gets here); what it drew over is
/// kept for the rain.
#[test]
fn a_wall_glyph_goes_only_over_a_plain_wall_line() {
    let area = Rect::new(0, 0, 3, 4);
    let mut buf = Buffer::empty(area);
    buf.set_string(1, 0, "│", Style::new());
    buf.set_string(1, 1, "┃", Style::new());
    buf.set_string(1, 2, "┤", Style::new());
    buf.set_string(1, 3, "T", Style::new());
    let before = buf.clone();
    let protected = [Rect::new(1, 1, 1, 1)];
    let drawn: Vec<Option<char>> = (0..4)
        .map(|y| super::wall_glyph(&mut buf, 1, y, '|', &protected).map(|f| f.glyph))
        .collect();
    assert_eq!(drawn, vec![Some('|'), None, None, None]);
    assert_eq!(buf[(1, 0)].symbol(), "|");
    for y in 1..4 {
        assert_eq!(buf[(1, y)], before[(1, y)], "row {y} untouched");
    }
    let mut again = before.clone();
    let kept = super::wall_glyph(&mut again, 1, 0, '|', &[]).expect("over the line");
    assert_eq!(kept.under, before[(1, 0)], "the line kept for the rain");
}

/// One frame of the stage's school scene: when, her door's beat (if
/// she's in one of her front door's), the looks painted, the frame, and
/// her (where, pose, what she says, in sight).
struct SchoolFrame {
    now: u64,
    beat: Option<(door::DoorSpot, usize, osaka::WallBeat)>,
    looks: Vec<Look>,
    frame: Buffer,
    at: (i32, i32),
    pose: Pose,
    said: Option<Bubble>,
    hidden: bool,
    /// Her own door as the frame stood it, drawn or not (C3's latch).
    front: Option<Front>,
}

/// The stage's school scene cued on `view` of `real` (her `pieces`, her
/// clock at `start`, fed or not): she walks to her door, goes out through
/// it, is out 5 s, and comes back in, saying she's home; then 10 s more.
/// Every frame painted, at most 50 ms apart.
fn school_scene_frames(
    real: &Buffer,
    view: &IdleView,
    pieces: &[(Furniture, Nook, u16)],
    start: routine::GameTime,
    graphics: bool,
    fed: bool,
) -> (Guest, Vec<SchoolFrame>) {
    let mut guest = home_at(4, start, pieces, graphics);
    if !fed {
        guest = guest.unfed();
    }
    school_scene_of(guest, real, view, graphics)
}

/// [`school_scene_frames`] for `guest` as made (her clock at its start).
fn school_scene_of(
    mut guest: Guest,
    real: &Buffer,
    view: &IdleView,
    graphics: bool,
) -> (Guest, Vec<SchoolFrame>) {
    let mut now = until_visiting(&mut guest, real, view, 0);
    guest.cue(Scene::School);
    let mut frames = Vec::new();
    let (mut hidden, mut home) = (false, None);
    let limit = now + 120_000;
    while home.is_none_or(|t| now < t + 10_000) {
        assert!(now < limit, "graphics={graphics}: never home again");
        now += guest
            .next_tick(now)
            .map_or(50, |d| d.as_millis() as u64)
            .clamp(1, 50);
        guest.advance(now);
        if let Some(graphics) = guest.graphics.as_mut() {
            graphics.take_looks();
        }
        let frame = paint(&mut guest, real, view, now);
        let looks = guest
            .graphics
            .as_mut()
            .map(Graphics::take_looks)
            .unwrap_or_default();
        let State::Visiting(visit) = &guest.state else {
            panic!("graphics={graphics}: visiting throughout");
        };
        let osaka = &visit.osaka;
        let (pose, _, said) = osaka.appearance(now);
        hidden |= osaka.hidden(now);
        if hidden && home.is_none() && says(said, mind::HOME) {
            home = Some(now);
        }
        frames.push(SchoolFrame {
            now,
            beat: osaka.wall_beat(now),
            looks,
            frame,
            at: (osaka.x, osaka.y),
            pose,
            said,
            hidden: osaka.hidden(now),
            front: visit.front.map(|(front, _)| front),
        });
    }
    (guest, frames)
}

/// Going out through her door in an inner wall and coming back in, she
/// steps through it a column every `WALK_MS` (beats 2 and 10), each step
/// held one `WALK_MS`, the last too (beat 3 and 11 begin one after it),
/// and nothing of her is drawn past the wall: no cell beyond it changes,
/// and in ASCII none of her glyphs stand in it. Both walls: a right one
/// (chat case (ii): Users' right wall at column 49, Playlist's pane
/// beyond it) and a left one (her door in Playlist's left wall at column
/// 50 on [`home_screen`], Users' border and pane beyond it). In line art
/// the door's front post is drawn after her while she's in the doorway.
/// Both modes.
#[test]
fn going_out_she_is_clipped_at_the_wall() {
    let (right, right_view) = chat_over_every_edge_space();
    let (left, left_view) = home_screen();
    let cases = [
        (right, right_view, None, (room::Side::Right, 49)),
        (
            left,
            left_view,
            Some(room::DoorWall {
                strip: room::Strip::Bottom(Nook::Playlist),
                side: room::Side::Left,
            }),
            (room::Side::Left, 50),
        ),
    ];
    for (real, view, wall_of_door, want) in cases {
        for graphics in [false, true] {
            let at = format!("{want:?} graphics={graphics}");
            let mut guest = home_at(
                4,
                tue(15, 0),
                &[(Furniture::Sofa, Nook::Users, 500)],
                graphics,
            );
            if wall_of_door.is_some() {
                guest.ledger.home.door = wall_of_door;
            }
            let (_, frames) = school_scene_of(guest, &real, &view, graphics);
            clipped_at_the_wall(&at, &real, &frames, want);
        }
    }
}

/// [`going_out_she_is_clipped_at_the_wall`]'s checks on one scene's
/// `frames` over `real`, her door in the wall `want`.
fn clipped_at_the_wall(at: &str, real: &Buffer, frames: &[SchoolFrame], want: (room::Side, i32)) {
    let mut steps: Vec<(usize, u64, i32)> = Vec::new();
    let mut begins: std::collections::BTreeMap<usize, u64> = std::collections::BTreeMap::new();
    for f in frames {
        let Some((door, beat, wall_beat)) = f.beat else {
            continue;
        };
        begins.entry(beat).or_insert(f.now);
        let (side, wall, floor) = wall_of(door);
        assert_eq!((side, wall), want, "{at}");
        // Nothing of her or her door past the wall's column.
        let past = match side {
            room::Side::Right => wall + 1..wall + 1 + sprite::WIDTH,
            room::Side::Left => wall - sprite::WIDTH..wall,
        };
        for y in floor - sprite::HEIGHT..=floor {
            for x in past.clone() {
                assert!(
                    !differs(&f.frame, real, (x, y)),
                    "{at}: beat {beat} at {}: ({x}, {y}) past the wall",
                    f.now
                );
            }
        }
        let Some((_, d)) = wall_beat.her.filter(|_| beat == 2 || beat == 10) else {
            continue;
        };
        steps.push((beat, f.now, d));
        if f.looks.is_empty() {
            // ASCII: the wall's own column is her door's glyph or the
            // wall's line, never hers.
            for y in floor - sprite::HEIGHT..floor {
                let cell = f.frame.cell((wall as u16, y as u16)).map(|c| c.symbol());
                let door_glyph = sprite::wall_door_cells(wall_beat.door, side)
                    .into_iter()
                    .find(|&(dx, dy, _)| dx == 0 && floor + dy == y)
                    .map(|(_, _, g)| g.to_string());
                let real_glyph = real.cell((wall as u16, y as u16)).map(|c| c.symbol());
                assert!(
                    cell == real_glyph || cell.map(str::to_owned) == door_glyph,
                    "{at}: beat {beat} d={d} at {}: {cell:?} in the wall at ({wall}, {y})",
                    f.now
                );
            }
        } else if d > 0 {
            let her = f.looks.iter().position(|l| matches!(l, Look::Pose(..)));
            let post = f.looks.iter().position(|l| {
                matches!(
                    l,
                    Look::WallDoor {
                        door: art::WallDoor::Post,
                        ..
                    }
                )
            });
            assert!(
                her.is_some() && post > her,
                "{at}: the post over her at {}: {:?}",
                f.now,
                f.looks
            );
        }
    }
    for (beat, ds) in [(2, [1, 2, 3, 4]), (10, [3, 2, 1, 0])] {
        let seen: Vec<(u64, i32)> = steps
            .iter()
            .filter(|s| s.0 == beat)
            .map(|s| (s.1, s.2))
            .collect();
        let mut order: Vec<i32> = seen.iter().map(|s| s.1).collect();
        order.dedup();
        assert_eq!(order, ds, "{at}: her steps in beat {beat}");
        // A column per WALK_MS: each step begins WALK_MS after the last
        // (to the frame's 50 ms), and the next beat WALK_MS after the
        // last step: each held one WALK_MS.
        let mut starts: Vec<u64> = ds
            .iter()
            .map(|&d| seen.iter().find(|s| s.1 == d).expect("a step").0)
            .collect();
        starts.push(*begins.get(&(beat + 1)).expect("the next beat"));
        for pair in starts.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                (osaka::WALK_MS - 50..=osaka::WALK_MS + 50).contains(&gap),
                "{at}: beat {beat} steps then the next beat {starts:?}"
            );
        }
    }
}

/// While she's out through her door, her slippers stand before it with a
/// card on its knob (`Shut { away: true }`): in her empty home, on a cold
/// start in school hours, and in a visit's gap (beats 6 and 7); they go
/// as the door opens (`Ajar` at 8, `Open` at 9), and never show while
/// she's in. Line art (ASCII has no cue).
#[test]
fn while_shes_out_her_slippers_stand_before_it() {
    let away = art::WallDoor::Shut {
        flap: 0,
        away: true,
    };
    // Her empty home, and a cold start in school hours.
    let (real, view) = home_screen();
    for start in [tue(9, 0), tue(12, 30)] {
        let mut guest = home_at(4, start, &HOME, true);
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        while empty_of(&guest).is_none_or(|e| e.size == (0, 0)) {
            assert!(now < 120_000, "{start:?}: her home never stood empty");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        guest.graphics.as_mut().expect("line art").take_looks();
        paint(&mut guest, &real, &view, now);
        let looks = guest.graphics.as_mut().expect("line art").take_looks();
        let doors = wall_doors(&looks);
        assert!(
            doors.iter().any(|d| d.0 == away),
            "{start:?}: her slippers before it: {looks:?}"
        );
    }
    // A visit's gap: the stage's school scene.
    let (real, view) = rooms_frame(100, 30);
    let (_, frames) = school_scene_frames(
        &real,
        &view,
        &[(Furniture::Sofa, Nook::Users, 300)],
        tue(15, 0),
        true,
        true,
    );
    let mut seen = std::collections::BTreeSet::new();
    for f in &frames {
        let doors = wall_doors(&f.looks);
        let slippers = doors.iter().any(|d| d.0 == away);
        match f.beat {
            Some((_, beat @ (6 | 7), _)) => {
                assert!(slippers, "beat {beat} at {}: {:?}", f.now, f.looks);
                seen.insert(beat);
            }
            Some((_, 8, _)) => {
                assert!(!slippers, "beat 8 at {}", f.now);
                assert!(doors.iter().any(|d| d.0 == art::WallDoor::Ajar));
                seen.insert(8);
            }
            Some((_, 9, _)) => {
                assert!(!slippers, "beat 9 at {}", f.now);
                assert!(doors.iter().any(|d| d.0 == art::WallDoor::Open));
                seen.insert(9);
            }
            _ => assert!(!slippers, "at {}: slippers while she's in", f.now),
        }
    }
    assert_eq!(
        seen.into_iter().collect::<Vec<_>>(),
        vec![6, 7, 8, 9],
        "every beat of her coming back seen"
    );
}

/// Out to school, the frame of beat 6 (her slippers' first) is the one
/// that ends the visit: she's gone out by her routine (`gone_out`) just
/// as before the side-on door, and her empty home shows it. Both modes.
#[test]
fn a_school_exit_still_ends_the_visit() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(4, tue(8, 10), &HOME, graphics);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let school = real_of(&guest, now, tue(8, 15));
        loop {
            assert!(now < school + 120_000, "{at}: never out");
            let next = now
                + guest
                    .next_tick(now)
                    .map_or(50, |d| d.as_millis() as u64)
                    .clamp(1, 50);
            if let State::Visiting(visit) = &guest.state
                && let Some((_, beat, wall_beat)) = visit.osaka.wall_beat(next)
                && beat == 6
            {
                assert_eq!(
                    wall_beat.door,
                    art::WallDoor::Shut {
                        flap: 0,
                        away: true
                    },
                    "{at}"
                );
                assert!(visit.osaka.gone_out(next).is_some(), "{at}: out at beat 6");
                now = next;
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
                assert!(empty_of(&guest).is_some(), "{at}: her home empty");
                break;
            }
            now = next;
            guest.advance(now);
            paint(&mut guest, &real, &view, now);
            assert!(
                !matches!(guest.state, State::Away(_)),
                "{at}: out before her door's beat 6"
            );
        }
    }
}

/// Whether a frame shows her front door side-on at `wall` (`floor` its
/// floor row): in line art a wall door look (every frame paints some
/// look: her pieces at least, asserted), in ASCII its wall column's `|`.
fn shows_wall_door(f: &SchoolFrame, real: &Buffer, wall: i32, floor: i32, graphics: bool) -> bool {
    if graphics {
        assert!(!f.looks.is_empty(), "at {}: no looks taken", f.now);
        !wall_doors(&f.looks).is_empty()
    } else {
        let cell = f.frame.cell((wall as u16, (floor - 1) as u16));
        cell.is_some_and(|c| c.symbol() == "|") && differs(&f.frame, real, (wall, floor - 1))
    }
}

/// Her door never pops in or out beside her (C3): from the first frame
/// that shows it, no frame shows it or stops showing it while her box
/// meets her door's space. The stage's school scene on a quiet space (no
/// text under it, so nothing hides it in passing); it shows while she's
/// in her door's space (not vacuous). The design's one exception (it may
/// appear beside her as she sets off, if she sets off in its space) isn't
/// reached here: she's cued from outside the space, so its appearing is
/// asserted away from her too. Both modes.
#[test]
fn her_door_never_pops_beside_her() {
    let (real, view) = rooms_frame(100, 30);
    let space = space_spot(&real, &view, Nook::Users, room::Side::Right);
    let (wall, floor) = (99, space.1);
    let rect = Rect::new(
        (wall - room::SPACE) as u16,
        (floor - sprite::HEIGHT) as u16,
        room::SPACE as u16,
        sprite::HEIGHT as u16 + 1,
    );
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (_, frames) = school_scene_frames(
            &real,
            &view,
            &[(Furniture::Sofa, Nook::Users, 300)],
            tue(15, 0),
            graphics,
            true,
        );
        let mut shown_beside = 0;
        let mut last: Option<bool> = None;
        for f in &frames {
            let shows = shows_wall_door(f, &real, wall, floor, graphics);
            let meets =
                !f.hidden && room::her_box(f.at.0, f.at.1).is_some_and(|b| b.intersects(rect));
            if meets && shows {
                shown_beside += 1;
            }
            if let Some(was) = last
                && was != shows
            {
                assert!(
                    !meets,
                    "{at}: her door {} beside her at {} ({:?})",
                    if shows { "popped in" } else { "went" },
                    f.now,
                    f.at
                );
            }
            last = Some(shows);
        }
        assert!(shown_beside > 0, "{at}: never shown beside her");
    }
}

/// The doorway shows the window's sky (D8): every look of her door that
/// shows outside (ajar or open) is under `Sky::at` her time of day; unfed,
/// a day sky, as her window's. Line art.
#[test]
fn the_doorway_shows_the_windows_sky() {
    let (real, view) = rooms_frame(100, 30);
    let pieces = [(Furniture::Sofa, Nook::Users, 300)];
    for (start, fed) in [(tue(18, 0), true), (tue(21, 30), true), (tue(15, 0), false)] {
        let at = format!("{start:?} fed={fed}");
        let (guest, frames) = school_scene_frames(&real, &view, &pieces, start, true, fed);
        let mut outside = 0;
        for f in &frames {
            let want = guest.time_of_day(f.now).map_or(art::Sky::Day, art::Sky::at);
            for (door, sky, _) in wall_doors(&f.looks) {
                if door.shows_sky() {
                    outside += 1;
                    assert_eq!(sky, want, "{at}: {door:?} at {}", f.now);
                }
            }
        }
        assert!(outside > 0, "{at}: the doorway never showed");
        if fed {
            let sky = art::Sky::at((start.minutes() % (24 * 60)) as u16);
            assert_ne!(sky, art::Sky::Day, "{at}: a sky not the day's");
        }
    }
}

/// Coming in, she holds side-on facing the room from her last step in
/// (beat 10's last) through her door shutting (12), and her one turn to
/// the viewer is the line she says as she's home (C12): the next pose
/// after them is that line's. Both modes.
#[test]
fn she_turns_to_the_viewer_once_coming_in() {
    let (real, view) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (_, frames) = school_scene_frames(
            &real,
            &view,
            &[(Furniture::Sofa, Nook::Users, 300)],
            tue(15, 0),
            graphics,
            true,
        );
        let last_step = frames
            .iter()
            .position(|f| matches!(f.beat, Some((_, 10, b)) if b.her.is_some_and(|h| h.1 == 0)))
            .unwrap_or_else(|| panic!("{at}: no last step in"));
        let mut held = 0;
        let mut turned = None;
        for f in &frames[last_step..] {
            if f.beat.is_some() {
                assert_eq!(f.pose, Pose::Side, "{at}: side-on at {}", f.now);
                held += 1;
                continue;
            }
            if f.pose != Pose::Side {
                turned = Some(f);
                break;
            }
        }
        assert!(held > 0, "{at}");
        let turned = turned.unwrap_or_else(|| panic!("{at}: never turned"));
        assert_eq!(turned.pose, Pose::Stand, "{at}: to the viewer");
        assert!(
            says(turned.said, mind::HOME),
            "{at}: as she says she's home: {:?}",
            turned.said
        );
    }
}

/// Her slippers never stand over text (door batch D8; the user's answer
/// lets her door's own two columns stand over it): on the wordy screen
/// the users listed run under the slippers' columns by her door (Users'
/// left wall), so it's drawn cropped to the shut door's two columns,
/// and the text there shows the whole while she's out. Line art (ASCII
/// draws no slippers).
#[test]
fn her_slippers_never_stand_over_text() {
    let (real, view) = wordy_home_screen();
    let (mut guest, mut now) = away_on(&real, &view, &HOME, true);
    let door = guest.closed_door().expect("her door");
    let (side, wall, floor) = wall_of(door);
    assert_eq!((side, wall), (room::Side::Left, 0));
    let slippers: Vec<(i32, i32)> = (floor - sprite::HEIGHT..floor)
        .flat_map(|y| (wall + 2..=wall + 3).map(move |x| (x, y)))
        .collect();
    let texty = slippers
        .iter()
        .filter(|&&(x, y)| {
            real.cell((x as u16, y as u16))
                .is_some_and(|c| !c.symbol().trim().is_empty())
        })
        .count();
    assert!(texty > 0, "the precondition: text under her slippers");
    let end = now + 30_000;
    let mut frames = 0;
    while now < end {
        now += 1000;
        guest.advance(now);
        guest.graphics.as_mut().expect("line art").take_looks();
        let frame = paint(&mut guest, &real, &view, now);
        let looks = guest.graphics.as_mut().expect("line art").take_looks();
        let doors = wall_doors(&looks);
        assert_eq!(
            doors,
            vec![(
                art::WallDoor::Shut {
                    flap: 0,
                    away: true
                },
                art::Sky::Day,
                2
            )],
            "at {now}: cropped to its own two columns"
        );
        for &at in &slippers {
            assert!(!differs(&frame, &real, at), "at {now}: {at:?} drawn over");
        }
        frames += 1;
    }
    assert!(frames > 0);
}

/// Her door stands over text only in passing while she's in (door batch
/// C9): with words in its own columns, as she sets off from the far end
/// of her home and walks to it (a walk far longer than that), then goes
/// out through it, no text is hidden behind it
/// (or her) for longer than her image may hide it, and it's drawn over
/// the words at least once (in her beats). In line art by the hidden-text
/// check (her home empty after, the door's own columns stand over the
/// words: exempt); in ASCII, its glyphs over the words outside her beats
/// for no longer than [`super::FRONT_PASSING_MS`] (and a frame) at a
/// time. Both modes.
#[test]
fn her_door_stands_over_text_only_in_passing_as_she_goes() {
    let (mut real, view) = home_screen();
    for row in 12..16u16 {
        real.set_string(1, row, "zz", Style::new());
    }
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        // At Playlist's right end as school begins: a long walk to her door.
        let (mut guest, mut now) = set_down_before_school(&real, &view, (95, 16), graphics);
        let school = now;
        let mut hidden = Hidden::default();
        let (mut over, mut walked) = (0, 0);
        let mut apart_since: Option<u64> = None;
        while !matches!(guest.state, State::Away(_)) {
            assert!(now < school + 120_000, "{at}: never out");
            now += guest
                .next_tick(now)
                .map_or(50, |d| d.as_millis() as u64)
                .clamp(1, 250);
            guest.advance(now);
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.take_looks();
            }
            let frame = paint(&mut guest, &real, &view, now);
            let looks = guest
                .graphics
                .as_mut()
                .map(Graphics::take_looks)
                .unwrap_or_default();
            let (layer, in_beats): (Vec<(u16, u16)>, bool) = match &guest.state {
                State::Visiting(visit) => {
                    if visit.osaka.bound_for_her_door() {
                        walked += 1;
                    }
                    (
                        visit.layer.cells().collect(),
                        visit.osaka.front_door(now).is_some(),
                    )
                }
                _ => (Vec::new(), true),
            };
            let drawn = door_over_words(graphics, &looks, &frame, &real);
            if drawn {
                over += 1;
            }
            if graphics {
                let shut = out_over_text(&guest, now);
                hidden
                    .check_raining(
                        &frame,
                        &real,
                        &layer,
                        &open_flap(&guest, now),
                        |at| raining(&guest, at) || shut.is_some_and(|r| r.contains(at.into())),
                        now,
                    )
                    .unwrap_or_else(|e| panic!("{e}"));
            } else if drawn && !in_beats {
                let since = *apart_since.get_or_insert(now);
                assert!(
                    now - since <= super::FRONT_PASSING_MS + 250,
                    "{at}: over the words from {since} to {now}"
                );
            } else {
                apart_since = None;
            }
        }
        assert!(walked > 0, "{at}: she walked to her door");
        assert!(over > 0, "{at}: her door drawn over the words");
    }
}

/// Whether her shut door (in [`home_screen`]'s left wall, floor row 16)
/// is drawn over the words in its own column (1): in line art a wall
/// door look with that cell drawn over; in ASCII its glyphs there.
fn door_over_words(graphics: bool, looks: &[Look], frame: &Buffer, real: &Buffer) -> bool {
    if graphics {
        return !wall_doors(looks).is_empty() && differs(frame, real, (1, 13));
    }
    let shut = art::WallDoor::Shut {
        flap: 0,
        away: false,
    };
    sprite::wall_door_cells(shut, room::Side::Left)
        .into_iter()
        .filter(|&(dx, _, _)| dx == 1)
        .any(|(dx, dy, glyph)| {
            let cell = (dx as u16, (16 + dy) as u16);
            frame[cell].symbol() == glyph.to_string() && differs(frame, real, (dx, 16 + dy))
        })
}

/// Each time she goes to her door, it stands over text in passing
/// afresh (door batch C9): home from school through it at 12:45 over the
/// words in its own columns (its time over them run out as she walks
/// off), later sent out again from the far end of her home (the stage's
/// school scene), it's drawn over the words as she walks to it, before
/// her beats through it begin. Both modes.
#[test]
fn her_door_over_text_shows_in_passing_each_time_she_goes() {
    let (mut real, view) = home_screen();
    for row in 12..16u16 {
        real.set_string(1, row, "zz", Style::new());
    }
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, now) = away_on(&real, &view, &HOME, graphics);
        let mut now = now;
        assert!(
            home_at_1245(&mut guest, &real, &view, now).is_some(),
            "{at}: home"
        );
        // A while in, then at Playlist's right end, sent out again.
        let later = now + 40_000;
        while now < later {
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        while now < real_of(&guest, now, tue(12, 46)) {
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.place(95, 16, now);
        paint(&mut guest, &real, &view, now);
        guest.cue(Scene::School);
        let mut walked_over = 0;
        let limit = now + 120_000;
        loop {
            assert!(now < limit, "{at}: never through her door");
            now += guest
                .next_tick(now)
                .map_or(50, |d| d.as_millis() as u64)
                .clamp(1, 250);
            guest.advance(now);
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.take_looks();
            }
            let frame = paint(&mut guest, &real, &view, now);
            let looks = guest
                .graphics
                .as_mut()
                .map(Graphics::take_looks)
                .unwrap_or_default();
            let osaka = &visit_of(&guest).osaka;
            if osaka.wall_beat(now).is_some() {
                break;
            }
            if osaka.bound_for_her_door() && door_over_words(graphics, &looks, &frame, &real) {
                walked_over += 1;
            }
        }
        assert!(
            walked_over > 0,
            "{at}: her door over the words as she walked to it"
        );
    }
}

/// A pane in use across her door's wall column hides her door whole
/// (door batch, step 8, deviation 6: no half a door beside a pane in
/// use), never her: going out through it (beats 2 to 4) with such a
/// pane focused, no cell of her door is drawn, while she's still drawn
/// stepping through its doorway (`d` columns toward the wall, as
/// though it stood); the pane left alone again mid-way, her door comes
/// back whole at once. The pane is the wall's own column only, so it
/// never meets her box (she's never turned aside). Both modes.
#[test]
fn a_pane_in_use_across_her_doors_wall_hides_it_whole_not_her() {
    let (real, quiet) = rooms_frame(100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let sofa = [(Furniture::Sofa, Nook::Users, 300)];
        let mut guest = home_at(4, tue(15, 0), &sofa, graphics);
        let mut now = until_visiting(&mut guest, &real, &quiet, 0);
        guest.cue(Scene::School);
        let limit = now + 120_000;
        let step = |guest: &mut Guest, view: &IdleView, now: &mut u64| {
            *now += guest
                .next_tick(*now)
                .map_or(50, |d| d.as_millis() as u64)
                .clamp(1, 50);
            guest.advance(*now);
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.take_looks();
            }
            let frame = paint(guest, &real, view, *now);
            let looks = guest
                .graphics
                .as_mut()
                .map(Graphics::take_looks)
                .unwrap_or_default();
            let beat = match &guest.state {
                State::Visiting(visit) => visit.osaka.wall_beat(*now),
                _ => None,
            };
            (frame, looks, beat)
        };
        // To her first step through it.
        let (door, wall, floor) = loop {
            assert!(now < limit, "{at}: never through her door");
            let (_, _, beat) = step(&mut guest, &quiet, &mut now);
            if let Some((door, 2, _)) = beat {
                let (side, wall, floor) = wall_of(door);
                assert_eq!(side, room::Side::Right, "{at}");
                break (door, wall, floor);
            }
        };
        let x = door.spot().0;
        let strip = Rect::new(wall as u16, 0, 1, 30);
        // In use (else no pane is kept from her: `Guest::gate`), by a
        // resident (else she'd say goodbye).
        let focused = IdleView {
            busy: Some(Busy::Playing),
            resident: true,
            focus: Some(strip),
            ..quiet.clone()
        };
        // Focused to beat 3's middle.
        let (mut stepping, mut out) = (0, 0);
        let leave = loop {
            assert!(now < limit, "{at}: never past beat 3");
            let (frame, looks, beat) = step(&mut guest, &focused, &mut now);
            let Some((_, index, wall_beat)) = beat else {
                panic!("{at}: out of her door's beats at {now}");
            };
            assert!(
                wall_doors(&looks).is_empty(),
                "{at}: beat {index} at {now}: her door drawn by a pane in use: {looks:?}"
            );
            if index == 3 && out > 3 {
                break now;
            }
            if index >= 3 {
                // Out of sight behind it: nothing of it, nor of her.
                out += 1;
                for y in floor - sprite::HEIGHT..floor {
                    for cx in x - sprite::WIDTH / 2..wall {
                        assert!(
                            !differs(&frame, &real, (cx, y)),
                            "{at}: beat {index} at {now}: ({cx}, {y}) drawn"
                        );
                    }
                }
                continue;
            }
            // (At 4 she's through: none of her in ASCII, C19.)
            let Some((_, d)) = wall_beat.her.filter(|&(_, d)| d == 3) else {
                continue;
            };
            // Stepped `d` toward the wall: nothing of her left of her
            // box moved on (its first column `x - 2 + d`), something in it.
            stepping += 1;
            let left = x - sprite::WIDTH / 2 + d - 1;
            for y in floor - sprite::HEIGHT..floor {
                assert!(
                    !differs(&frame, &real, (left, y)),
                    "{at}: d={d} at {now}: ({left}, {y}) drawn: she's not in the doorway"
                );
            }
            assert!(
                (floor - sprite::HEIGHT..floor).any(|y| (left + 1..wall).any(|cx| differs(
                    &frame,
                    &real,
                    (cx, y)
                ))),
                "{at}: d={d} at {now}: she's not drawn"
            );
        };
        assert!(stepping > 0, "{at}: never seen stepping through");
        // Left alone: her door whole from the next frame, to beat 6.
        let mut back = 0;
        loop {
            assert!(now < leave + 5_000, "{at}: never out");
            let (frame, looks, beat) = step(&mut guest, &quiet, &mut now);
            let Some((_, index @ 3..=5, wall_beat)) = beat else {
                break;
            };
            back += 1;
            if graphics {
                let doors: Vec<(art::WallDoor, u8)> =
                    wall_doors(&looks).iter().map(|d| (d.0, d.2)).collect();
                assert_eq!(
                    doors,
                    vec![(wall_beat.door, wall_beat.door.cols())],
                    "{at}: beat {index} at {now}: her door whole"
                );
            } else {
                for (dx, dy, glyph) in sprite::wall_door_cells(wall_beat.door, room::Side::Right) {
                    let cell = frame.cell(((wall + dx) as u16, (floor + dy) as u16));
                    assert_eq!(
                        cell.map(|c| c.symbol().to_owned()),
                        Some(glyph.to_string()),
                        "{at}: beat {index} at {now}: ({}, {}) of her door",
                        wall + dx,
                        floor + dy
                    );
                }
            }
        }
        assert!(back > 0, "{at}: never seen back");
    }
}

/// A pane in use over her slippers' columns alone only crops them (the
/// user's answer: only a protected pane hides her door, and only the
/// door's own cells count; the slippers are cropped wherever they
/// can't stand): while she's out, her door still stands, its own two
/// columns, as a visit's gap stands it. Line art (ASCII draws no
/// slippers).
#[test]
fn a_pane_in_use_over_her_slippers_only_crops_them() {
    let (real, quiet) = home_screen();
    let (mut guest, mut now) = away_on(&real, &quiet, &HOME, true);
    let door = guest.closed_door().expect("her door");
    let (side, wall, floor) = wall_of(door);
    assert_eq!((side, wall), (room::Side::Left, 0));
    let slippers = Rect::new(
        (wall + 2) as u16,
        (floor - sprite::HEIGHT) as u16,
        2,
        sprite::HEIGHT as u16,
    );
    let focused = IdleView {
        busy: Some(Busy::Playing),
        resident: true,
        focus: Some(slippers),
        ..quiet.clone()
    };
    for _ in 0..5 {
        now += 1000;
        guest.advance(now);
        guest.graphics.as_mut().expect("line art").take_looks();
        let frame = paint(&mut guest, &real, &focused, now);
        let looks = guest.graphics.as_mut().expect("line art").take_looks();
        assert_eq!(guest.closed_door(), Some(door), "at {now}: never moved");
        assert_eq!(
            wall_doors(&looks),
            vec![(super::AWAY, art::Sky::Day, 2)],
            "at {now}: her door, its own two columns"
        );
        for y in floor - sprite::HEIGHT..floor {
            for x in wall + 2..=wall + 3 {
                assert!(
                    !differs(&frame, &real, (x, y)),
                    "at {now}: ({x}, {y}) drawn"
                );
            }
        }
    }
}

/// Her face-on fallback door (a short terminal: its wall's strip too
/// short for its space) never pops in or out beside her, and never
/// turns (C3, step 8 review): from the frame that stands it, it comes
/// and goes only while her box is clear of it, and faces out the one
/// way its spot does (`DoorSpot::out`) throughout, through her beats,
/// her coming in and its staying after, never with her. Both modes.
#[test]
fn her_face_on_door_never_pops_or_turns_beside_her() {
    let (real, view) = real_frame(&mut real_ui(), 100, 20);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(
            4,
            tue(15, 0),
            &[(Furniture::Sofa, Nook::Users, 300)],
            graphics,
        );
        guest.ledger.home.door = Some(room::DoorWall {
            strip: room::Strip::Bottom(Nook::Users),
            side: room::Side::Right,
        });
        let (_, frames) = school_scene_of(guest, &real, &view, graphics);
        let (mut beside, mut faced) = (0, 0);
        let mut last: Option<bool> = None;
        let mut spot = None;
        for f in &frames {
            spot = f.front.map(Front::spot).or(spot);
            if let Some(front) = f.front {
                let spot = front.spot();
                assert!(spot.wall().is_none(), "{at}: face-on: {spot:?}");
                assert_eq!(
                    front.facing(),
                    Some(spot.out()),
                    "{at}: at {} it faces its spot's way out",
                    f.now
                );
                faced += 1;
            }
            let shows = f.front.is_some();
            let meets = !f.hidden
                && spot
                    .and_then(|spot: door::DoorSpot| spot.room())
                    .zip(room::her_box(f.at.0, f.at.1))
                    .is_some_and(|(room, her)| room.intersects(her));
            if meets && shows {
                beside += 1;
            }
            if let Some(was) = last
                && was != shows
            {
                assert!(
                    !meets,
                    "{at}: her door {} beside her at {} ({:?})",
                    if shows { "popped in" } else { "went" },
                    f.now,
                    f.at
                );
            }
            last = Some(shows);
        }
        assert!(faced > 0 && beside > 0, "{at}: never stood beside her");
    }
}

/// Her door's time over text in passing ends on time (step 8 review):
/// home through it at 12:45 with words in its own columns, while it
/// stands over them after her beats, her guest's next tick is never
/// later than when its time there runs out
/// ([`super::FRONT_PASSING_MS`]), and the frame painted then has it
/// gone from over the words, however still she stands. Driven by her
/// own ticks alone. Both modes.
#[test]
fn her_door_over_text_goes_on_time_however_still_she_stands() {
    let (mut real, view) = home_screen();
    for row in 12..16u16 {
        real.set_string(1, row, "zz", Style::new());
    }
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, now) = away_on(&real, &view, &HOME, graphics);
        let mut now = now;
        assert!(
            home_at_1245(&mut guest, &real, &view, now).is_some(),
            "{at}: home"
        );
        let limit = now + 60_000;
        let (mut timed, mut ended) = (0, 0);
        let mut due_end: Option<u64> = None;
        while now < limit {
            let tick = guest
                .next_tick(now)
                .map_or(60_000, |d| d.as_millis() as u64)
                .max(1);
            let next = now + tick;
            if let Some(end) = due_end.filter(|&end| end > now) {
                assert!(
                    next <= end,
                    "{at}: at {now} the next tick {next} is past {end}"
                );
                timed += 1;
            }
            now = next;
            guest.advance(now);
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.take_looks();
            }
            let frame = paint(&mut guest, &real, &view, now);
            let looks = guest
                .graphics
                .as_mut()
                .map(Graphics::take_looks)
                .unwrap_or_default();
            if due_end.is_some_and(|end| now >= end) {
                // Its time over the words is out: none of it is drawn
                // (she may stand over them still: her own image may).
                assert!(wall_doors(&looks).is_empty(), "{at}: at {now}: {looks:?}");
                if !graphics {
                    let shut = art::WallDoor::Shut {
                        flap: 0,
                        away: false,
                    };
                    for (dx, dy, glyph) in sprite::wall_door_cells(shut, room::Side::Left) {
                        let cell = frame.cell((dx as u16, (16 + dy) as u16));
                        assert_ne!(
                            cell.map(|c| c.symbol().to_owned()),
                            Some(glyph.to_string()),
                            "{at}: at {now}: ({dx}, {}) of her door",
                            16 + dy
                        );
                    }
                }
                ended += 1;
                due_end = None;
            }
            let State::Visiting(visit) = &guest.state else {
                break;
            };
            let end = visit
                .front
                .and_then(|(_, since)| since)
                .map(|since| since + super::FRONT_PASSING_MS);
            if due_end.is_none() && end.is_some() && ended == 0 {
                // Still beside it, a good while (nothing of hers wakes
                // the frame).
                let State::Visiting(visit) = &mut guest.state else {
                    unreachable!()
                };
                visit.osaka.stand_still(now + 20_000, now);
            }
            due_end = end;
        }
        assert!(
            timed > 0 && ended > 0,
            "{at}: its time over the words never ran out"
        );
    }
}

/// A goodbye as she steps through her door (beat 2, a step or more
/// into its doorway): she jumps up out of it, startled then waving at
/// her spot, uncut (no post of her door over her), her door behind her
/// as it stood, open. Line art (ASCII's goodbye is her letters).
#[test]
fn a_goodbye_mid_doorway_has_her_jump_out_of_it() {
    let (real, view) = rooms_frame(100, 30);
    let mut guest = home_at(4, tue(15, 0), &[(Furniture::Sofa, Nook::Users, 300)], true);
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    guest.cue(Scene::School);
    let limit = now + 120_000;
    loop {
        assert!(now < limit, "never in her doorway");
        now += guest
            .next_tick(now)
            .map_or(50, |d| d.as_millis() as u64)
            .clamp(1, 50);
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        if visit_of(&guest)
            .osaka
            .wall_beat(now)
            .is_some_and(|(_, beat, b)| beat == 2 && b.her.is_some_and(|(_, d)| d >= 2))
        {
            break;
        }
    }
    guest.activity(now);
    assert!(matches!(guest.state, State::Leaving(_)), "a goodbye");
    let start = now;
    let (mut her, mut doors) = (0, 0);
    while now < start + dissolve::RAIN_FROM_MS {
        now += 50;
        guest.graphics.as_mut().expect("line art").take_looks();
        paint(&mut guest, &real, &view, now);
        let looks = guest.graphics.as_mut().expect("line art").take_looks();
        let walls = wall_doors(&looks);
        assert!(
            !walls.iter().any(|d| d.0 == art::WallDoor::Post),
            "at {now}: her door's post over her: {looks:?}"
        );
        if walls.iter().any(|d| d.0 == art::WallDoor::Open) {
            doors += 1;
        }
        if looks
            .iter()
            .any(|l| matches!(l, Look::Pose(..) | Look::Wave(..)))
        {
            her += 1;
        }
    }
    assert!(
        her > 0 && doors > 0,
        "her ({her}) and her door ({doors}) drawn"
    );
}

/// A goodbye while her door stands apart from her (her walk to it after
/// her set-off, its own image) holds it as it stood until the rain, and
/// no longer: a wall door look in the goodbye's frames before
/// `RAIN_FROM_MS`, none after. Line art.
#[test]
fn a_goodbye_on_her_way_to_her_door_holds_it_to_the_rain() {
    let (real, view) = home_screen();
    let (mut guest, mut now) = set_down_before_school(&real, &view, (95, 16), true);
    let limit = now + 120_000;
    loop {
        assert!(now < limit, "never on her way with her door apart");
        now += guest
            .next_tick(now)
            .map_or(50, |d| d.as_millis() as u64)
            .clamp(1, 250);
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        let visit = visit_of(&guest);
        if visit.osaka.bound_for_her_door() && visit.front_apart.is_some() {
            break;
        }
    }
    guest.activity(now);
    assert!(matches!(guest.state, State::Leaving(_)), "a goodbye");
    let start = now;
    let (mut held, mut after) = (0, 0);
    while now < start + dissolve::DURATION_MS {
        now += 50;
        guest.graphics.as_mut().expect("line art").take_looks();
        paint(&mut guest, &real, &view, now);
        let looks = guest.graphics.as_mut().expect("line art").take_looks();
        let doors = wall_doors(&looks);
        if now < start + dissolve::RAIN_FROM_MS {
            assert_eq!(
                doors.iter().map(|d| d.0).collect::<Vec<_>>(),
                vec![art::WallDoor::Shut {
                    flap: 0,
                    away: false
                }],
                "at {now}: her door as it stood"
            );
            held += 1;
        } else {
            assert!(doors.is_empty(), "at {now}: past the rain: {looks:?}");
            after += 1;
        }
    }
    assert!(held > 0 && after > 0);
}

/// In a visit's gap (beats 6 and 7: the stage's school scene, out a few
/// seconds and back), her slippers stand only on calm cells, as while
/// she's out: on the wordy screen, with the users listed under the
/// slippers' columns by her door, it's drawn cropped to the shut door's
/// own two columns, and nothing of those columns is drawn over. Line
/// art (ASCII draws no slippers).
#[test]
fn in_a_visits_gap_her_slippers_never_stand_over_text() {
    let (real, view) = wordy_home_screen();
    let guest = home_at(4, tue(15, 0), &HOME, true);
    let (_, frames) = school_scene_of(guest, &real, &view, true);
    let mut gap = 0;
    for f in &frames {
        let Some((door, beat @ (6 | 7), _)) = f.beat else {
            continue;
        };
        let (side, wall, floor) = wall_of(door);
        assert_eq!((side, wall), (room::Side::Left, 0));
        let texty = (floor - sprite::HEIGHT..floor)
            .flat_map(|y| (wall + 2..=wall + 3).map(move |x| (x, y)))
            .any(|(x, y)| {
                real.cell((x as u16, y as u16))
                    .is_some_and(|c| !c.symbol().trim().is_empty())
            });
        assert!(texty, "the precondition: text under her slippers");
        let doors: Vec<(art::WallDoor, u8)> =
            wall_doors(&f.looks).iter().map(|d| (d.0, d.2)).collect();
        assert_eq!(
            doors,
            vec![(super::AWAY, 2)],
            "beat {beat} at {}: cropped to its own two columns",
            f.now
        );
        for y in floor - sprite::HEIGHT..floor {
            for x in wall + 2..=wall + 3 {
                assert!(
                    !differs(&f.frame, &real, (x, y)),
                    "beat {beat} at {}: ({x}, {y}) drawn over",
                    f.now
                );
            }
        }
        gap += 1;
    }
    assert!(gap > 0, "never in her door's gap");
}

// ---- Parcels through her door's flap (door batch, step 9) ----

/// The ends of a parcel's slide's beats through her door's flap, ms
/// from its coming (D9; snippets.md, "The flap's beats"): nosing out,
/// half out, out, settling; at rest from the last.
const SLIDE_BEATS: [u64; 4] = [150, 350, 500, 650];

/// One frame as a parcel came in: its age, the frame, the looks painted
/// (line art), her own door as the frame stood it, and her feet.
struct FlapFrame {
    age: u64,
    frame: Buffer,
    looks: Vec<Look>,
    front: Option<Front>,
    her: (i32, i32),
    /// The parcel as it shows (where it rests).
    parcel: Option<Shown>,
}

/// Sends `guest` (visiting on `view` of `real`) a parcel at `from`, and
/// paints at most 50 ms apart until it has come in (within two minutes)
/// and for [`FLAP_MS`] and a little more after (or until another
/// comes): when it came, its flap, the frames from then, and when they
/// ended.
fn a_parcel_comes(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    from: u64,
) -> (u64, room::Flap, Vec<FlapFrame>, u64) {
    guest.send_parcel();
    let mut now = from;
    let mut came: Option<(u64, room::Flap)> = None;
    let mut frames = Vec::new();
    loop {
        if let Some(graphics) = guest.graphics.as_mut() {
            graphics.take_looks();
        }
        let frame = paint(guest, real, view, now);
        let looks = guest
            .graphics
            .as_mut()
            .map(Graphics::take_looks)
            .unwrap_or_default();
        let State::Visiting(visit) = &guest.state else {
            panic!("visiting throughout");
        };
        if came.is_none() {
            came = visit
                .flap
                .filter(|&(_, since)| since >= from)
                .map(|(flap, since)| (since, flap));
        }
        match came {
            Some((since, _)) if visit.flap.is_some_and(|(_, at)| at != since) => break,
            Some((since, flap)) => {
                frames.push(FlapFrame {
                    age: now - since,
                    frame,
                    looks,
                    front: visit.front.map(|(front, _)| front),
                    her: (visit.osaka.x, visit.osaka.y),
                    parcel: visit
                        .shown
                        .iter()
                        .find(|s| s.item == flap.item && s.boxed)
                        .copied(),
                });
                if now >= since + FLAP_MS + 200 {
                    break;
                }
            }
            None => assert!(now < from + 120_000, "the parcel never came"),
        }
        now += guest
            .next_tick(now)
            .map_or(50, |d| d.as_millis() as u64)
            .clamp(1, 50);
        guest.advance(now);
    }
    let (since, flap) = came.expect("it came");
    (since, flap, frames, now)
}

/// Whether the cell `(x, y)` is in her box standing at `her`.
fn in_her_box(her: (i32, i32), (x, y): (i32, i32)) -> bool {
    room::her_box(her.0, her.1).is_some_and(|b| {
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        b.contains((x, y).into())
    })
}

/// A parcel comes in through the flap in her own door (door batch D9):
/// side-on in her door's wall, the flap riding up on the parcel as it
/// slides out (in line art the door's flap swung up, the parcel cut at
/// the wall's line and the flap's plate over it, at least three angles
/// of it; in ASCII the door's open-flap rows, and the parcel at its spot
/// at once), then the door shut; it rests just past her door's space
/// (its trailing edge 7 columns from the wall), and once the flap's time
/// is up her door is gone again. Never the wall's own flap glyphs. Both
/// walls, both modes. Red before step 9: the wall's `╲`/`╱`, no door.
#[test]
fn a_parcel_comes_in_through_her_door() {
    for (name, real, view, pieces, side) in both_walls() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(4, tue(13, 0), &pieces, graphics);
            let now = until_visiting(&mut guest, &real, &view, 0);
            let (_, flap, frames, now) = a_parcel_comes(&mut guest, &real, &view, now);
            assert!(flap.door, "{at}: through her door: {flap:?}");
            assert_eq!(flap.side, side, "{at}: {flap:?}");
            let (wall, floor) = (flap.x, flap.rows.1);
            let rect = frames[0].parcel.expect("the parcel shows").rect();
            match side {
                room::Side::Right => {
                    assert_eq!(i32::from(rect.right()) - 1, wall - 7, "{at}: {rect:?}");
                }
                room::Side::Left => assert_eq!(i32::from(rect.x), wall + 7, "{at}: {rect:?}"),
            }
            let space = Rect::new(
                match side {
                    room::Side::Right => wall - room::SPACE,
                    room::Side::Left => wall + 1,
                } as u16,
                (floor - sprite::HEIGHT) as u16,
                room::SPACE as u16,
                (sprite::HEIGHT + 1) as u16,
            );
            let shut = |flap| art::WallDoor::Shut { flap, away: false };
            let mut angles = std::collections::BTreeSet::new();
            let mut leads = Vec::new();
            for f in &frames {
                let at = format!("{at} at {}", f.age);
                for y in floor - sprite::HEIGHT..floor {
                    let cell = f.frame[(wall as u16, y as u16)].symbol();
                    assert!(!matches!(cell, "╲" | "╱"), "{at}: the wall's flap at {y}");
                }
                let beside = room::her_box(f.her.0, f.her.1).is_some_and(|b| b.intersects(space));
                let doors = wall_doors(&f.looks);
                if f.age < 650 {
                    if graphics {
                        let up = doors
                            .iter()
                            .find_map(|&(door, ..)| match door {
                                art::WallDoor::Shut { flap, away: false } => Some(flap),
                                _ => None,
                            })
                            .unwrap_or_else(|| panic!("{at}: no door: {:?}", f.looks));
                        if up > 0 {
                            assert!(
                                doors.iter().any(|d| d.0 == art::WallDoor::Plate(up)),
                                "{at}: the plate over the parcel: {doors:?}"
                            );
                        }
                        assert!(
                            f.looks.contains(&Look::Parcel(flap.item, false)),
                            "{at}: the parcel: {:?}",
                            f.looks
                        );
                        let cut = f
                            .front
                            .and_then(Front::parcel)
                            .unwrap_or_else(|| panic!("{at}: no parcel sliding"));
                        assert_eq!(cut.clip, Some((side, wall)), "{at}");
                        let (left, _, cols, _) = cut.bounds().expect("some of it");
                        match side {
                            room::Side::Right => assert!(left + cols <= wall + 1, "{at}"),
                            room::Side::Left => assert!(left >= wall, "{at}"),
                        }
                        angles.insert(up);
                        // Its leading edge, the room's side of it.
                        leads.push(match side {
                            room::Side::Right => left,
                            room::Side::Left => left + cols,
                        });
                    } else {
                        assert!(
                            f.front.and_then(Front::parcel).is_none(),
                            "{at}: ASCII draws the parcel at rest, never sliding"
                        );
                        for (dx, dy, glyph) in sprite::wall_door_cells(shut(45), side) {
                            let (x, y) = (wall + dx, floor + dy);
                            if in_her_box(f.her, (x, y)) {
                                continue;
                            }
                            assert_eq!(
                                f.frame[(x as u16, y as u16)].symbol(),
                                glyph.to_string(),
                                "{at}: the open flap's ({x}, {y})"
                            );
                        }
                        assert!(
                            rect.positions().any(|p| f.frame.cell(p) != real.cell(p)),
                            "{at}: the parcel at its spot at once"
                        );
                    }
                } else if f.age < FLAP_MS {
                    if graphics {
                        assert_eq!(
                            doors.iter().map(|d| d.0).collect::<Vec<_>>(),
                            vec![shut(0)],
                            "{at}: shut"
                        );
                    } else {
                        for (dx, dy, glyph) in sprite::wall_door_cells(shut(0), side) {
                            let (x, y) = (wall + dx, floor + dy);
                            if in_her_box(f.her, (x, y)) {
                                continue;
                            }
                            assert_eq!(
                                f.frame[(x as u16, y as u16)].symbol(),
                                glyph.to_string(),
                                "{at}: the shut door's ({x}, {y})"
                            );
                        }
                    }
                } else if !beside {
                    if graphics {
                        assert!(doors.is_empty(), "{at}: her door gone: {doors:?}");
                    } else {
                        for y in floor - sprite::HEIGHT..floor {
                            assert!(!differs(&f.frame, &real, (wall, y)), "{at}: ({wall}, {y})");
                        }
                    }
                }
            }
            if graphics {
                assert!(angles.len() >= 3, "{at}: the flap's angles {angles:?}");
                // It slides: from by the wall's line into the room, frame
                // by frame, short of where it rests till its last beat.
                leads.dedup();
                assert!(leads.len() >= 3, "{at}: where it slid {leads:?}");
                let (first, last) = (leads[0], leads[leads.len() - 1]);
                let rest = match side {
                    room::Side::Right => i32::from(rect.x),
                    room::Side::Left => i32::from(rect.right()),
                };
                match side {
                    room::Side::Right => {
                        assert!(leads.windows(2).all(|w| w[1] < w[0]), "{at}: {leads:?}");
                        assert!(first >= wall - 3, "{at}: from the wall: {leads:?}");
                        assert!(last > rest, "{at}: short of its rest {rest}: {leads:?}");
                    }
                    room::Side::Left => {
                        assert!(leads.windows(2).all(|w| w[1] > w[0]), "{at}: {leads:?}");
                        assert!(first <= wall + 4, "{at}: from the wall: {leads:?}");
                        assert!(last < rest, "{at}: short of its rest {rest}: {leads:?}");
                    }
                }
            }
            // The next comes in through her door too, to the same place,
            // and pushes the first along.
            let first = flap.item;
            let (_, next, frames, _) = a_parcel_comes(&mut guest, &real, &view, now);
            assert!(next.door && next.item != first, "{at}: {next:?}");
            let rested = frames[0].parcel.expect("the next shows").rect();
            match side {
                room::Side::Right => assert_eq!(rested.right(), rect.right(), "{at}"),
                room::Side::Left => assert_eq!(rested.x, rect.x, "{at}"),
            }
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            let pushed = visit
                .shown
                .iter()
                .find(|s| s.item == first)
                .expect("the first still shows")
                .rect();
            assert!(!pushed.intersects(rested), "{at}: pushed along: {pushed:?}");
        }
    }
}

/// With her door's space yielding (her pieces fill it:
/// [`super::felt::lamp_home`]), a parcel comes in through another wall's flap, today's
/// `╲`/`╱` over the wall's line, not through her door. Both modes. A
/// guard (every flap was the wall's before step 9): a mutant putting
/// every parcel through her door makes it red. (Here the full lane
/// refuses her door's wall whatever the flag says; a space yielding to
/// a hung piece, the floor past it free, is
/// [`a_parcel_by_a_yielding_space_comes_through_the_walls_flap`].)
#[test]
fn a_parcel_with_her_door_space_yielding_comes_through_another_wall() {
    let (real, view) = chat_by_a_full_space();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = super::felt::lamp_home(graphics);
        let space = super::felt::users_space(&guest, &view);
        assert!(!space.kept, "{at}: the precondition: the space yields");
        let now = until_visiting(&mut guest, &real, &view, 0);
        let (_, flap, frames, _) = a_parcel_comes(&mut guest, &real, &view, now);
        assert!(!flap.door, "{at}: {flap:?}");
        assert_ne!(
            (flap.side, flap.x),
            (space.side, space.wall),
            "{at}: not her door's wall"
        );
        let first = &frames[0];
        let glyph = match flap.side {
            room::Side::Right => "╱",
            room::Side::Left => "╲",
        };
        for y in flap.rows.0..flap.rows.1 {
            assert_eq!(
                first.frame[(flap.x as u16, y as u16)].symbol(),
                glyph,
                "{at}: the wall's flap at {y}"
            );
        }
        assert!(
            wall_doors(&first.looks).is_empty(),
            "{at}: no door of hers: {:?}",
            first.looks
        );
    }
}

/// No parcel slides through her or her slippers (door batch C10): one
/// due as she goes out through her door and while she's out (the stage's
/// school and work scenes: her beats and her gap), or while she stands
/// in her door's space, waits; it comes once she's clear of it, through
/// her door, and she stays clear of it every frame its flap is open.
/// Both modes. Red before step 9: it came with her in the space (her
/// beats and gap were already kept out by her being on her way out or
/// coming home: guards; a door's act `awake` lets through is
/// [`a_parcel_waits_out_a_door_in_space`]'s).
#[test]
fn no_parcel_slides_through_her_or_her_slippers() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        for scene in [Scene::School, Scene::Work] {
            let at = format!("{scene:?} graphics={graphics}");
            let mut guest = home_at(4, tue(13, 0), &HOME, graphics);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            // Her arrival over, then the scene, and the parcel due.
            now = run_quiet(&mut guest, &real, &view, now, 20_000);
            guest.cue(scene);
            let (mut went, mut came, mut sent) = (false, None, false);
            // Her shift is up to 3 minutes.
            let limit = now + 420_000;
            while came.is_none_or(|(_, since)| now < since + FLAP_MS) {
                assert!(now < limit, "{at}: the parcel never came");
                now += guest
                    .next_tick(now)
                    .map_or(50, |d| d.as_millis() as u64)
                    .clamp(1, 50);
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
                let State::Visiting(visit) = &guest.state else {
                    panic!("{at}: visiting throughout");
                };
                let osaka = &visit.osaka;
                let through = osaka.front_door(now).is_some();
                went |= through;
                // Due once she's set off for her door.
                if !sent && (osaka.bound_for_her_door() || through) {
                    sent = true;
                    guest.send_parcel();
                    continue;
                }
                // Every frame of its slide and its flap: never in her
                // beats, never through her.
                came = came.or(visit.flap);
                if let Some((flap, since)) = came
                    && now < since + FLAP_MS
                {
                    let age = now - since;
                    assert!(!through, "{at}: a parcel at {age} in her beats");
                    assert!(
                        !in_the_slides_way(visit, flap, now),
                        "{at}: a parcel at {age} through her"
                    );
                }
            }
            assert!(went, "{at}: she went through her door");
            assert!(
                came.is_some_and(|(f, _)| f.door),
                "{at}: through her door: {came:?}"
            );
        }
        // Standing in her door's space, still: it waits; moved out of it,
        // it comes. (A fixture: she's put there and moved out directly.
        // The scenes above bring her there honestly, coming home, and
        // [`a_parcel_waits_for_her_walk_into_her_doors_space`] walks her
        // in.)
        let at = format!("in her space graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        let (_, users) = view.nooks[0];
        let floor = i32::from(users.bottom()) - 1;
        visiting_at(&mut guest, &real, &view, (users.x as i32 + 3, floor));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.stand_still(60_000, 0);
        guest.send_parcel();
        let mut now = 0;
        while now < 20_000 {
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            assert!(visit.flap.is_none(), "{at}: a parcel at {now} through her");
            now += 500;
            guest.advance(now);
        }
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.x = 75;
        visit.osaka.stand_still(now + 60_000, now);
        paint(&mut guest, &real, &view, now);
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting");
        };
        assert!(
            visit.flap.is_some_and(|(flap, _)| flap.door),
            "{at}: it came through her door once she was clear: {:?}",
            visit.flap
        );
    }
}

/// [`run`] from `from` for `ms`, painting every frame the shell would:
/// when it ended.
fn run_quiet(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64, ms: u64) -> u64 {
    run(guest, real, view, from, from + ms);
    from + ms
}

/// The slide repaints at each of its beats (door batch D9): her tick
/// wakes at each beat's end and the shell is told the screen changed
/// there, and at the flap's end, each beat's frame unlike the last (in
/// line art, the looks painted). In ASCII the open flap's glyphs stand
/// till the parcel rests (the parcel stands at its rest at once), so it
/// wakes only as the flap shuts and at the flap's end, not for beats
/// that draw nothing new.
#[test]
fn the_slide_repaints_at_each_beat() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        let (_, users) = view.nooks[0];
        let floor = i32::from(users.bottom()) - 1;
        visiting_at(&mut guest, &real, &view, (75, floor));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        // Still through the flap's time, so only it changes the frame.
        visit.osaka.stand_still(60_000, 0);
        guest.send_parcel();
        let first = paint(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting");
        };
        let (flap, since) = visit.flap.expect("it came at once");
        assert!(flap.door, "{at}: through her door");
        let ends: &[u64] = if graphics {
            &SLIDE_BEATS
        } else {
            &SLIDE_BEATS[3..]
        };
        let mut now = since;
        let mut last = (
            first,
            guest
                .graphics
                .as_mut()
                .map(Graphics::take_looks)
                .unwrap_or_default(),
        );
        for &end in ends.iter().chain([&FLAP_MS]) {
            let due = since + end;
            let mut changed = false;
            while now < due {
                let wake = now
                    + guest
                        .next_tick(now)
                        .map_or(u64::MAX, |d| d.as_millis() as u64);
                assert!(
                    wake <= due,
                    "{at}: no wake by {end} (next at {})",
                    wake - since
                );
                // Still, nothing else wakes her tick meanwhile.
                assert_eq!(wake, due, "{at}: woken at {} for nothing", wake - since);
                now = wake;
                changed = guest.advance(now);
                assert!(
                    !changed || now == due,
                    "{at}: a change told at {} between beats",
                    now - since
                );
            }
            assert!(changed, "{at}: the screen unchanged at {end}");
            let frame = paint(&mut guest, &real, &view, now);
            let looks = guest
                .graphics
                .as_mut()
                .map(Graphics::take_looks)
                .unwrap_or_default();
            if graphics {
                assert_ne!(looks, last.1, "{at}: the same looks at {end}");
            } else {
                assert!(frame != last.0, "{at}: the same frame at {end}");
            }
            last = (frame, looks);
        }
    }
}

/// A parcel due while she goes through a door in space (an ordinary
/// door, not her own: one her being up and about lets through) waits
/// till the door's act is over, then comes, through her door (door
/// batch D9: a delivery waits while a door's act runs). Both modes. A
/// guard: a mutant letting a parcel come in a door's act makes it red.
#[test]
fn a_parcel_waits_out_a_door_in_space() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (40, 16));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.through_door((60, 16), 0);
        guest.send_parcel();
        let (mut now, mut doors) = (0, 0);
        let came = loop {
            assert!(now < 30_000, "{at}: the parcel never came");
            paint(&mut guest, &real, &view, now);
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            let in_door = matches!(visit.osaka.through(), Some(osaka::Through::Space(_)));
            doors += usize::from(in_door);
            if let Some((flap, _)) = visit.flap {
                assert!(!in_door, "{at}: a parcel at {now} in her door's act");
                break flap;
            }
            now += guest
                .next_tick(now)
                .map_or(50, |d| d.as_millis() as u64)
                .clamp(1, 50);
            guest.advance(now);
        };
        assert!(doors > 0, "{at}: the precondition: she went through it");
        assert!(came.door, "{at}: through her door: {came:?}");
    }
}

/// Her door standing face-on (its wall's top protected: it falls back to
/// the nearest floor clear of its space), a parcel waits while she stands
/// where that door stands (the room its beats take, clear of the space),
/// and comes once she's moved off (door batch C10). Both modes. A guard:
/// a mutant judging only the space makes it red.
#[test]
fn a_parcel_waits_for_her_off_her_face_on_door() {
    let (real, mut view) = home_screen();
    view.protected.push(Rect::new(0, 12, 7, 5));
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (40, 16));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.stand_still(600_000, 0);
        paint(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        let spot = visit.door.expect("a door");
        assert!(
            spot.wall().is_none(),
            "{at}: the precondition: face-on: {spot:?}"
        );
        let (x, y) = spot.spot();
        let space = door::space_rect(&guest.ledger.home, plan_of(&view, &real)).expect("a space");
        assert!(
            !room::her_box(x, y).is_some_and(|b| b.intersects(space)),
            "{at}: the precondition: clear of the space: {spot:?} {space:?}"
        );
        (visit.osaka.x, visit.osaka.y) = (x, y);
        visit.osaka.stand_still(600_000, 0);
        guest.send_parcel();
        let (_, since) =
            slide_clear_of_her(&mut guest, &real, &view, (0, 120_000), &at, |guest, now| {
                if now >= 20_000
                    && let State::Visiting(visit) = &mut guest.state
                    && visit.osaka.x == x
                {
                    visit.osaka.x = 40;
                    visit.osaka.stand_still(now + 600_000, now);
                }
            });
        assert!(since >= 20_000, "{at}: it waited for her: {since}");
    }
}

/// A parcel through her door's flap while her door doesn't stand at its
/// wall this frame (the wall's top protected: her door falls back
/// face-on) comes in through the wall's own flap there, `╲` over the
/// wall's line, with no door of hers drawn (D9's fallback look; the
/// review of step 9). Both modes. A guard: a mutant never drawing the
/// wall's flap for her door's makes it red (no flap at all).
#[test]
fn a_parcel_by_her_doors_wall_comes_through_the_walls_flap() {
    let (real, mut view) = home_screen();
    view.protected.push(Rect::new(0, 12, 1, 2));
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (40, 16));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.stand_still(600_000, 0);
        let (_, flap, frames, _) = a_parcel_comes(&mut guest, &real, &view, 0);
        assert!(
            flap.door && (flap.side, flap.x) == (room::Side::Left, 0),
            "{at}: the precondition: her door's flap: {flap:?}"
        );
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting");
        };
        assert!(
            visit.door.is_some_and(|spot| spot.wall().is_none()),
            "{at}: the precondition: her door face-on: {:?}",
            visit.door
        );
        for f in frames.iter().filter(|f| f.age < FLAP_MS) {
            let at = format!("{at} at {}", f.age);
            for y in flap.rows.0..flap.rows.1 {
                assert_eq!(
                    f.frame[(0, y as u16)].symbol(),
                    "╲",
                    "{at}: the wall's flap at {y}"
                );
            }
            assert!(
                wall_doors(&f.looks).is_empty(),
                "{at}: no door of hers: {:?}",
                f.looks
            );
        }
    }
}

/// The stage's parcel cue brings its parcel at the cue's paint, though
/// she's long since begun an act that holds her still; the same parcel
/// ordered with no cue waits (door batch Open choice 3: never mid-act).
/// Both modes. A guard: a mutant giving the cue no exemption makes it
/// red, one giving it to every delivery too.
#[test]
fn only_the_stages_parcel_comes_mid_act() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        for cued in [true, false] {
            let at = format!("cued={cued} graphics={graphics}");
            let mut guest = Guest::new(3);
            if graphics {
                guest.set_picker(kitty());
            }
            visiting_at(&mut guest, &real, &view, (40, 16));
            guest.ledger.visits = 1;
            let State::Visiting(visit) = &mut guest.state else {
                panic!("{at}: visiting");
            };
            visit.osaka.stand_still(600_000, 0);
            paint(&mut guest, &real, &view, 0);
            if cued {
                guest.cue(Scene::Parcel);
            } else {
                guest.send_parcel();
            }
            let mut now = 5_000;
            paint(&mut guest, &real, &view, now);
            let came = |guest: &Guest| match &guest.state {
                State::Visiting(visit) => visit.flap.is_some(),
                _ => panic!("visiting"),
            };
            if cued {
                assert!(came(&guest), "{at}: at the cue's paint");
                continue;
            }
            while now < 15_000 {
                assert!(!came(&guest), "{at}: mid-act at {now}");
                now += 500;
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
            }
        }
    }
}

/// The room a parcel coming in through her door's flap `flap` slides
/// through: her door's space by its wall (the six columns past the wall,
/// her height above its floor row and the floor row).
fn slide_space(flap: room::Flap) -> Rect {
    let left = match flap.side {
        room::Side::Right => flap.x - room::SPACE,
        room::Side::Left => flap.x + 1,
    };
    Rect::new(
        left as u16,
        (flap.rows.1 - sprite::HEIGHT) as u16,
        room::SPACE as u16,
        (sprite::HEIGHT + 1) as u16,
    )
}

/// Whether she, in sight in `visit` at `now`, stands where a parcel
/// coming in through her door's flap `flap` would slide through her:
/// in the space it slides through, or in the room her door's beats take
/// where it stands.
fn in_the_slides_way(visit: &Visit, flap: room::Flap, now: u64) -> bool {
    let osaka = &visit.osaka;
    !osaka.hidden(now)
        && room::her_box(osaka.x, osaka.y).is_some_and(|her| {
            std::iter::once(slide_space(flap))
                .chain(visit.door.and_then(door::DoorSpot::room))
                .any(|room| her.intersects(room))
        })
}

/// Paints `guest` (visiting on `view` of `real`) from `from` as the
/// shell would, at most 50 ms apart, until a parcel that came in from
/// `from` on has had its flap's time ([`FLAP_MS`]), asserting on every
/// frame of a flap through her door that she isn't in its way
/// ([`in_the_slides_way`]): the flap, and when it came. `meanwhile` runs
/// after each paint (the frame's `now`). Panics if none came by `limit`.
fn slide_clear_of_her(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    (from, limit): (u64, u64),
    at: &str,
    mut meanwhile: impl FnMut(&mut Guest, u64),
) -> (room::Flap, u64) {
    let mut now = from;
    let mut came = None;
    loop {
        paint(guest, real, view, now);
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting throughout");
        };
        came = came.or(visit.flap.filter(|&(_, since)| since >= from));
        if let Some((flap, since)) = came {
            if now >= since + FLAP_MS {
                return (flap, since);
            }
            assert!(
                !flap.door || !in_the_slides_way(visit, flap, now),
                "{at}: {flap:?} at {} through her at ({}, {})",
                now - since,
                visit.osaka.x,
                visit.osaka.y
            );
        }
        assert!(now < limit, "{at}: the parcel never came");
        meanwhile(guest, now);
        now += guest
            .next_tick(now)
            .map_or(50, |d| d.as_millis() as u64)
            .clamp(1, 50);
        guest.advance(now);
    }
}

/// A parcel due as she walks toward her door's space (an ordinary walk,
/// not out through her door) waits until no step she takes while its
/// flap is open brings her into the space it slides through (door batch
/// C10; the review of step 9: it was judged on where she stood as it
/// came, and she walked into its slide). Both modes. Red before the fix:
/// it came with her two columns from the space, and she stepped into it
/// mid-slide.
#[test]
fn a_parcel_waits_for_her_walk_into_her_doors_space() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        // Users' floor; her door by its left wall (the space columns 1-6).
        visiting_at(&mut guest, &real, &view, (10, 16));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.walk_to(3, 0);
        guest.send_parcel();
        let (flap, since) =
            slide_clear_of_her(&mut guest, &real, &view, (0, 120_000), &at, |guest, now| {
                // Clear of it after 20 s, still.
                if now >= 20_000
                    && let State::Visiting(visit) = &mut guest.state
                    && visit.osaka.x < 10
                {
                    visit.osaka.x = 40;
                    visit.osaka.stand_still(now + 600_000, now);
                }
            });
        assert!(since >= 20_000, "{at}: it waited for her: {since}");
        assert!(flap.door, "{at}: through her door: {flap:?}");
    }
}

/// Her first parcel (no door of hers saved) waits for her to be clear of
/// the space of the door it actually comes through: here the wall the
/// empty home would choose (Users' left) can't take it (a protected pane
/// past the space), so it comes through Playlist's right, which becomes
/// her door, and she stands in that space. Once she's clear, it comes
/// through there (door batch C10; the review of step 9: only the empty
/// home's choice was judged). Both modes. Red before the fix: it slid
/// through her at once.
#[test]
fn her_first_parcel_waits_for_her_by_the_door_it_takes() {
    let (real, mut view) = home_screen();
    view.protected.push(Rect::new(7, 9, 12, 8));
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (96, 16));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.stand_still(600_000, 0);
        paint(&mut guest, &real, &view, 0);
        let plan = plan_of(&view, &real);
        assert_eq!(
            guest.ledger.home.wall(plan).map(|w| (w.strip, w.side)),
            Some((room::Strip::Bottom(Nook::Users), room::Side::Left)),
            "{at}: the precondition: the empty home's door on Users' left"
        );
        guest.send_parcel();
        let (flap, since) =
            slide_clear_of_her(&mut guest, &real, &view, (0, 120_000), &at, |guest, now| {
                // Clear of it after 20 s, still.
                if now >= 20_000
                    && let State::Visiting(visit) = &mut guest.state
                    && visit.osaka.x == 96
                {
                    visit.osaka.x = 75;
                    visit.osaka.stand_still(now + 600_000, now);
                }
            });
        assert!(since >= 20_000, "{at}: it waited for her: {since}");
        assert!(
            flap.door && (flap.side, flap.x) == (room::Side::Right, 99),
            "{at}: through her door on Playlist's right: {flap:?}"
        );
    }
}

/// A flap through her door is drawn as her door only while her door
/// stands at that wall's own strip: if it moves to another strip whose
/// wall shares the column (List's left over Users' left, column 0) while
/// the flap is open, the flap shows in the wall where the parcel comes
/// in, never as her door on the other strip, apart from its parcel
/// (the review of step 9: the match read only the side and column).
/// Both modes. Red before the fix: her door with its flap open on List.
#[test]
fn a_door_flap_stays_on_its_own_strip() {
    let list = Rect::new(0, 0, 50, 8);
    let users = Rect::new(0, 8, 50, 9);
    let playlist = Rect::new(50, 0, 50, 17);
    let real = {
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
        for area in [list, users, playlist] {
            tuirealm::ratatui::widgets::Widget::render(
                tuirealm::ratatui::widgets::Block::bordered(),
                area,
                &mut buf,
            );
        }
        buf
    };
    let view = IdleView {
        nooks: vec![
            (Nook::List, list),
            (Nook::Users, users),
            (Nook::Playlist, playlist),
        ],
        ..view(bottom_strip(100, 20))
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (75, 16));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.stand_still(600_000, 0);
        guest.send_parcel();
        paint(&mut guest, &real, &view, 0);
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting");
        };
        let (flap, since) = visit.flap.expect("it came at once");
        assert!(
            flap.door && (flap.side, flap.x) == (room::Side::Left, 0),
            "{at}: the precondition: through her door at column 0: {flap:?}"
        );
        let saved = guest.ledger.home.door.expect("her door saved");
        let other = if saved.strip == room::Strip::Bottom(Nook::List) {
            Nook::Users
        } else {
            Nook::List
        };
        guest.ledger.home.door = Some(room::DoorWall {
            strip: room::Strip::Bottom(other),
            side: room::Side::Left,
        });
        let mut now = since;
        let mut moved = false;
        while now < since + SLIDE_BEATS[3] {
            now += 50;
            guest.advance(now);
            if let Some(graphics) = guest.graphics.as_mut() {
                graphics.take_looks();
            }
            let frame = paint(&mut guest, &real, &view, now);
            let looks = guest
                .graphics
                .as_mut()
                .map(Graphics::take_looks)
                .unwrap_or_default();
            let State::Visiting(visit) = &guest.state else {
                panic!("{at}: visiting");
            };
            let Some(spot) = visit.door else { continue };
            if spot.spot().1 == flap.rows.1 {
                continue;
            }
            moved = true;
            let at = format!("{at} at {}", now - since);
            assert!(
                visit
                    .front
                    .is_none_or(|(front, _)| front.spot().spot().1 == flap.rows.1),
                "{at}: her door on the other strip: {:?}",
                visit.front
            );
            assert!(
                wall_doors(&looks)
                    .iter()
                    .all(|(door, ..)| !matches!(door, art::WallDoor::Shut { flap: 1.., .. })),
                "{at}: her door's flap open: {looks:?}"
            );
            for y in flap.rows.0..flap.rows.1 {
                assert_eq!(
                    frame[(0, y as u16)].symbol(),
                    "╲",
                    "{at}: the wall's flap at {y}"
                );
            }
        }
        assert!(moved, "{at}: the precondition: her door moved strips");
    }
}

/// The stage's parcel, cued while she stands in her door's space, comes
/// as soon as she's clear of it, whatever act holds her (the cue's own
/// exemption from the wait for an act's start lasts until it comes; the
/// review of step 9: it held only on the cue's paint). Both modes. Red
/// before the fix: it waited for her act to end.
#[test]
fn the_stages_parcel_comes_once_she_is_clear_of_her_door() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(3);
        if graphics {
            guest.set_picker(kitty());
        }
        visiting_at(&mut guest, &real, &view, (3, 16));
        guest.ledger.visits = 1;
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        visit.osaka.stand_still(600_000, 0);
        paint(&mut guest, &real, &view, 0);
        guest.cue(Scene::Parcel);
        paint(&mut guest, &real, &view, 5_000);
        let State::Visiting(visit) = &mut guest.state else {
            panic!("{at}: visiting");
        };
        assert!(
            visit.flap.is_none(),
            "{at}: the precondition: not through her"
        );
        // Clear of her door, held long since by an act.
        visit.osaka.x = 75;
        visit.osaka.stand_still(600_000, 0);
        paint(&mut guest, &real, &view, 5_050);
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting");
        };
        assert!(
            visit.flap.is_some_and(|(flap, _)| flap.door),
            "{at}: it came once she was clear: {:?}",
            visit.flap
        );
    }
}

/// Her door's space yielding to her window hung by its wall (it has
/// nowhere else to hang, her poster beside it), though the floor past
/// the space is free: a parcel comes in by that wall, her door's, but
/// through the wall's own flap (`╲` over the wall's line, no door of
/// hers drawn), every frame of its flap, never through her door, which
/// didn't stand there as it came (D9: through her door only while its
/// space is kept; the review of step 9). Both modes. A guard: a mutant
/// dropping the `kept` clause of `Home::through_her_door` makes it red
/// (her door, standing there once the parcel's in, slides it through).
#[test]
fn a_parcel_by_a_yielding_space_comes_through_the_walls_flap() {
    use crate::ui::houseguest::room::{Anchor, DoorWall, Prop, Side, Strip};
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut ledger = Ledger::new_at(4, tue(13, 0));
        ledger.clock_sent = true;
        for (item, nook, side, offset) in [
            (Furniture::Window, Nook::Users, Side::Left, 0),
            (Furniture::Poster, Nook::Users, Side::Left, 1),
            (Furniture::Tv, Nook::Playlist, Side::Right, 0),
        ] {
            assert!(ledger.home.add(Prop {
                anchor: Some(Anchor { side, offset }),
                ..Prop::new(item, nook, 0, sprite::Facing::Right)
            }));
        }
        let wall = DoorWall {
            strip: Strip::Bottom(Nook::Users),
            side: Side::Left,
        };
        ledger.home.door = Some(wall);
        let mut guest = Guest::restore(ledger);
        guest.set_date(date(2026, 6, 17));
        if graphics {
            guest.set_picker(kitty());
        }
        let space = guest
            .ledger
            .home
            .extents(&view.nooks)
            .into_iter()
            .find(|p| p.strip == wall.strip)
            .and_then(|p| p.space)
            .expect("a space");
        assert!(!space.kept, "{at}: the precondition: it yields: {space:?}");
        let now = until_visiting(&mut guest, &real, &view, 0);
        let (_, flap, frames, _) = a_parcel_comes(&mut guest, &real, &view, now);
        assert_eq!(
            (flap.side, flap.x),
            (Side::Left, 0),
            "{at}: by her door's wall: {flap:?}"
        );
        assert!(!flap.door, "{at}: not through her door: {flap:?}");
        for f in frames.iter().filter(|f| f.age < FLAP_MS) {
            let at = format!("{at} at {}", f.age);
            for y in flap.rows.0..flap.rows.1 {
                assert_eq!(
                    f.frame[(0, y as u16)].symbol(),
                    "╲",
                    "{at}: the wall's flap at {y}"
                );
            }
            assert!(
                wall_doors(&f.looks).is_empty(),
                "{at}: no door of hers: {:?}",
                f.looks
            );
        }
    }
}
