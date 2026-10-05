//! Away (phase 5b D3, step 5b): on a school morning she goes out through
//! her door at 08:15, the door stands closed where she left, her home
//! stands empty with the lamp off, and at 12:45 she comes home out of
//! it. Each in both drawing modes; the screens quiet and text-dense
//! (`home_screens`, or text scattered over `rooms`).

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
fn closed(door: DoorAt) -> Door {
    Door::closed(door.x, door.y, door.facing)
}

/// Whether `frame` shows her closed door at `door` in her empty home
/// (`empty`, just painted): in ASCII, its glyphs, each where it's free to
/// stand (on blank cells clear of her pieces) and at least one; in line
/// art, her door's image, closed, there, with cells of her box drawn.
fn door_shows(empty: &Empty, frame: &Buffer, real: &Buffer, door: DoorAt) -> bool {
    match &empty.image {
        Some(image) => {
            let drawn = image.figure.door().map(|d| d.cells().collect::<Vec<_>>());
            drawn == Some(closed(door).cells().collect())
                && image.figure.her().is_none()
                && box_changed(frame, real, (door.x, door.y)) > 0
        }
        None => {
            let covers: Vec<Rect> = empty.shown.iter().map(Shown::cover).collect();
            let mut seen = 0;
            for (x, y, glyph) in closed(door).cells() {
                let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                    continue;
                };
                let blank = real
                    .cell((x, y))
                    .is_some_and(|c| c.symbol().trim().is_empty());
                if !blank || covers.iter().any(|r| r.contains((x, y).into())) {
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

/// From `from` on `real`, until her home stands empty; a few steps more
/// so a frame has painted it. Returns when.
fn until_away(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64) -> u64 {
    let mut now = from;
    paint(guest, real, view, now);
    while empty_of(guest).is_none() {
        assert!(now < from + 60_000, "her home never stood empty");
        shell_step(guest, real, view, &mut now, true);
    }
    shell_step(guest, real, view, &mut now, true);
    now
}

/// Seed 4's school morning on `real`: visiting from 08:10, out through
/// her door at 08:15. Returns her home standing empty, when, and where
/// her door stands.
fn out_to_school(real: &Buffer, view: &IdleView, graphics: bool) -> (Guest, u64, DoorAt) {
    let mut guest = home_at(4, tue(8, 10), &HOME, graphics);
    let now = until_visiting(&mut guest, real, view, 0);
    let now = until_away(&mut guest, real, view, now);
    let door = guest.closed_door().expect("her door");
    (guest, now, door)
}

/// One school morning (Tuesday, fed), in skips of her clock: she leaves
/// at 08:15 through her door in place, saying so; her door stays closed
/// where she left (drawn from the very frame she's out, with no tick
/// between); her home stands empty, the TV and the lamp off; and at
/// 12:45 she comes home out of that door saying she's home, with no
/// hello (the day's mood goes on).
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
            assert_eq!(spot, Some(from), "{at}: through her door where she stood");
            let door = guest.closed_door().expect("her door");
            assert_eq!((door.x, door.y), from, "{at}: her door where she went out");
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
                assert_eq!((osaka.x, osaka.y), from, "{at}: out of her door");
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
                    box_changed(&frame, &real, (door.x, door.y)) > 0,
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
/// her door's spot is kept: when it shows again, her door stands where
/// she went out. A frame too small doesn't spin the shell (it waits for
/// the next quiet, as a visit does); visits switched off take it at once
/// with no rain; an overlay rains it out.
#[test]
fn her_door_stays_where_she_left_whatever_takes_her_home_away() {
    let (w, h) = (50, 15);
    let small = rooms(w, h);
    let small_view = IdleView {
        nooks: nooks(w, h),
        ..view(bottom_strip(w, h))
    };
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let (mut guest, mut now, door) = out_to_school(&real, &view, graphics);
            let middle = {
                let terrain = Terrain::read(&real, &view.protected, graphics);
                door_spot(&terrain, middle((real.area.width, real.area.height)))
            };
            assert_ne!(
                Some((door.x, door.y)),
                middle,
                "{name}: a door off the middle"
            );
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
                assert_eq!(
                    guest.closed_door(),
                    Some(door),
                    "{at}: her door's spot kept"
                );
                now = until_away(&mut guest, &real, &view, now);
                let frame = paint(&mut guest, &real, &view, now);
                assert_eq!(guest.closed_door(), Some(door), "{at}: where she went out");
                let empty = empty_of(&guest).expect("away");
                assert!(door_shows(empty, &frame, &real, door), "{at}");
            }
        }
    }
}

/// A resize that leaves her door's spot off every floor moves it to the
/// nearest spot where she'd fit; it stays there after.
#[test]
fn a_resize_moves_her_door_to_the_nearest_spot_it_fits() {
    let (w, h) = (100, 30);
    let other = wordy_rooms(w, h);
    let other_view = IdleView {
        nooks: nooks(w, h),
        ..view(bottom_strip(w, h))
    };
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, mut now, door) = out_to_school(&real, &view, graphics);
        now += 100;
        guest.advance(now);
        let frame = paint(&mut guest, &other, &other_view, now);
        let moved = guest.closed_door().expect("her door");
        let empty = empty_of(&guest).expect("away");
        let mut fit = Terrain::read(&other, &other_view.protected, graphics);
        fit.furnish(empty.shown.iter().map(Shown::cover));
        assert!(!door_fits(&fit, (door.x, door.y)), "{at}: the spot went");
        assert_eq!(
            Some((moved.x, moved.y)),
            door_spot(&fit, (door.x, door.y)),
            "{at}: the nearest spot that fits"
        );
        assert_eq!(moved.facing, door.facing, "{at}");
        assert!(door_shows(empty, &frame, &other, moved), "{at}");
        now += 1000;
        guest.advance(now);
        paint(&mut guest, &other, &other_view, now);
        assert_eq!(guest.closed_door(), Some(moved), "{at}: and stays");
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
            doors_pane |= osaka::box_meets(pane, (door.x, door.y));
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

/// A focused pane that takes in only part of her door's box keeps all
/// of it away: no half a door outside the pane.
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
        // Her box from its middle column rightwards.
        let focus = Rect::new(
            door.x as u16,
            (door.y - sprite::HEIGHT) as u16,
            sprite::WIDTH as u16,
            sprite::HEIGHT as u16 + 1,
        );
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
        }
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
                        } else if !furnished {
                            assert!(matches!(guest.state, State::Absent), "{at}");
                            assert_eq!(frame, real, "{at}: nothing at all");
                        }
                    }
                }
                let mut home = false;
                while !home {
                    assert!(now < home_time + 20_000, "{at}: never home");
                    shell_step(&mut guest, &real, &view, &mut now, true);
                    let osaka = &visit_of(&guest).osaka;
                    assert!(
                        osaka.door(now).is_some() || !osaka.hidden(now),
                        "{at}: her door gone at {now}"
                    );
                    assert_ne!(osaka.act_name(), "Fall", "{at}: no drop from the sky");
                    home = says(osaka.appearance(now).2, mind::HOME);
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
                    matches!(guest.state, State::Arriving(How::Return(_)))
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
                        if matches!(guest.state, State::Arriving(How::Return(_))) {
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
                    !matches!(guest.state, State::Arriving(How::Return(_))),
                    "{at}: home out of her door with a visitor at the keys"
                );
                if matches!(guest.state, State::Arriving(How::Idle)) {
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
            // Until she sets off through her door, in sight still.
            loop {
                assert!(now < school + 30_000, "{at}: never set off");
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
            let door = guest.closed_door().expect("her door");
            assert_eq!((door.x, door.y), (x, y), "{at}: where she went out");
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
        assert!(matches!(guest.state, State::Arriving(How::Idle)), "{at}");
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
        let mut guest = Guest::restore(Ledger::new_at(seed, start));
        guest.set_date(date(2026, 6, 17));
        long_visit_of(guest, graphics, &sizes, &text, &skips, &chats, protect, &owned, 120_000)?;
    }
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
        &[(64, 33)],
        &text,
        &[(53, 0)],
        &[],
        (0, 0, 1, 1),
        &owned,
        120_000,
    )
    .unwrap_or_else(|e| panic!("{e}"));
}
