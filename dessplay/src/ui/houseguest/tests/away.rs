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

/// Her closed door standing at `door`, as it's drawn.
fn closed(door: door::DoorSpot) -> Door {
    let (x, y) = door.spot();
    Door::closed(x, y, door.out())
}

/// Whether `frame` shows her closed door at `door` in her empty home
/// (`empty`, just painted), strictly: none of its cells under any of her
/// pieces (that's a failure, not a skip); in ASCII, every glyph of it
/// drawn, over text too (only a wide glyph's cells, which it can't take,
/// are passed over), and at least one; in line art, her door's image,
/// closed, there, taking in no piece, with cells of her box drawn.
fn door_shows(empty: &Empty, frame: &Buffer, real: &Buffer, door: door::DoorSpot) -> bool {
    let covers: Vec<Rect> = empty.shown.iter().map(Shown::cover).collect();
    let under_a_piece = closed(door).cells().any(|(x, y, _)| {
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        covers.iter().any(|r| r.contains((x, y).into()))
    });
    if under_a_piece {
        return false;
    }
    match &empty.image {
        Some(image) => {
            let drawn = image.figure.door().map(|d| d.cells().collect::<Vec<_>>());
            drawn == Some(closed(door).cells().collect())
                && image.figure.her().is_none()
                && image.with.is_empty()
                && box_changed(frame, real, door.spot()) > 0
        }
        None => {
            let mut seen = 0;
            let mut after_wide = false;
            for (x, y, glyph) in closed(door).cells() {
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
/// from its middle column rightwards) keeps all of it away: no half a
/// door outside the pane, and it never moves meanwhile; left alone, it's
/// back in its space.
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
        assert!(door.wall().is_some(), "{at}: in its space");
        // Her door's drawn cells from its middle column rightwards.
        let drawn = &empty_of(&guest).expect("away").door.as_ref().unwrap().cells;
        let right: Vec<(u16, u16)> = drawn
            .iter()
            .copied()
            .filter(|&(x, _)| i32::from(x) >= door.spot().0)
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
            .filter_map(|(x, y, _)| Some((u16::try_from(x).ok()?, u16::try_from(y).ok()?)))
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

/// Out at work as school begins (the stage's doing: her job is open only
/// on days off), she goes on to school: the visit ends at once, her home
/// stands empty, and she never comes home with her shopping.
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
            visit.osaka.go_to_work(None, now, &mut Rng(1));
            while empty_of(&guest).is_none() {
                assert!(now < school + 2_000, "{at}: still out at work past 08:15");
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
        let raining = closed(door).cells().any(|(x, y, _)| {
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
    closed(door).cells().any(|(x, y, _)| {
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
        let middle = builds(&real, visit, &[pull(80)], &[], &keep);
        assert!(!middle.is_empty(), "{at}: a piece mid-strip");
        let free = builds(&real, visit, &[pull(95)], &[], &door::Keep::default());
        assert!(!free.is_empty(), "{at}: one in the space, nothing kept");
        let kept = builds(&real, visit, &[pull(95)], &[], &keep);
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
        let offered = builds(&real, visit, &pulls, &solid, &door::Keep::default());
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
