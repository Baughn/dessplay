//! She gazes up at what's on her shelf (phase 6a D8, step 6): a use of
//! her plant up there (`Use::GazeUp`), offered by where it stands, never
//! by its kind; taken from beside it on its room's floor, facing it,
//! "hehe" a moment and then a long, curious look up; only from that
//! floor (no climb for it). Both modes.

use super::shelf::{PIECES, mantel_clock, users_shelf};
use super::*;
use crate::ui::houseguest::brain::Want;
use crate::ui::houseguest::room::{PieceRef, Seat, Use};
use crate::ui::houseguest::script::GAZE_UP_PLEASED_MS;
use crate::ui::houseguest::sprite::{Face, Facing, Pose};
use crate::ui::houseguest::stage::{NO_ROOM_BESIDE, NOTHING_UP};

/// Users' floor on [`rooms_frame`] at 100×30 (`50, 0, 50, 13`).
const USERS_FLOOR: i32 = 12;
/// The Playlist's floor (`50, 13, 50, 14`).
const PLAYLIST_FLOOR: i32 = 26;
/// Her plant up on her Users shelf (its run `63..73` on row 5), `Left`
/// 3: columns `66..69`.
const PLANT: std::ops::Range<i32> = 66..69;

/// Her plant up on her Users shelf, turned `facing` (and her sofa and
/// bed on the floors), visiting on a Saturday afternoon, each of `marks`
/// a cell kept from her (protected, as a pane in use is: on Users' floor
/// rows, it makes a spot there unrestful in either mode, where text
/// would only in line art), put at `at` and offered only her gaze up (by
/// her real piece): the frame, the view, and when.
fn plant_up(
    seed: u64,
    graphics: bool,
    facing: Facing,
    marks: &[(u16, u16)],
    at: (i32, i32),
) -> (Guest, Buffer, IdleView, u64) {
    let (real, mut view) = rooms_frame(100, 30);
    view.protected
        .extend(marks.iter().map(|&(x, y)| Rect::new(x, y, 1, 1)));
    let mut guest = home_at(seed, sat(14, 0), &PIECES, graphics);
    guest.ledger.home.structures.push(users_shelf());
    assert!(guest.ledger.home.add(room::Prop {
        item: Furniture::Plant,
        facing,
        ..mantel_clock()
    }));
    let mut now = until_visiting(&mut guest, &real, &view, 0);
    now += 250;
    guest.advance(now);
    put(&mut guest, at, now);
    paint(&mut guest, &real, &view, now);
    (guest, real, view, now)
}

/// Put her at `at`, standing, offered only her gaze up.
fn put(guest: &mut Guest, (x, y): (i32, i32), now: u64) {
    let State::Visiting(visit) = &mut guest.state else {
        panic!("visiting");
    };
    visit.osaka.place(x, y, now);
    visit.osaka.offer_only = Some((Want::Use(Use::GazeUp), "use/real"));
}

/// The frame's seat to gaze up at her plant from, if it has one.
fn gaze_seat(guest: &Guest) -> Option<Seat> {
    visit_of(guest)
        .chances
        .seats
        .iter()
        .find(|s| s.what == Use::GazeUp)
        .copied()
}

/// Whether she has chosen to gaze up since decision `from`.
fn chose_to_gaze(guest: &Guest, from: usize) -> bool {
    visit_of(guest).osaka.decisions[from..]
        .iter()
        .any(|d| d.want == Some(Want::Use(Use::GazeUp)))
}

/// Her plant's run on her Users shelf: columns `63..73` on row 5.
const RUN: std::ops::Range<i32> = 63..73;

/// Her seat to gaze up at her plant is at `x`, beside it (her box one or
/// two columns clear of its columns), under its run, on its room's floor,
/// and she faces it: her gaze is in profile, so she looks up at it.
fn beside_and_facing(seat: &Seat, x: i32, at: &str) {
    assert_eq!(seat.piece, PieceRef::Real(Furniture::Plant), "{at}");
    assert_eq!(seat.y, USERS_FLOOR, "{at}: on its room's floor");
    assert_eq!(seat.x, x, "{at}");
    assert!(RUN.contains(&seat.x), "{at}: under its run {RUN:?}");
    let half = sprite::WIDTH / 2;
    let (left, right) = (seat.x - half, seat.x + half);
    let gap = if seat.x < PLANT.start {
        PLANT.start - right
    } else {
        left - PLANT.end + 1
    };
    assert!(
        (1..=2).contains(&gap),
        "{at}: her box {left}..={right} not just beside {PLANT:?}"
    );
    let toward = if seat.x < PLANT.start {
        Facing::Right
    } else {
        Facing::Left
    };
    assert_eq!(seat.facing, toward, "{at}: facing it");
}

/// One of [`SPOTS`]: her start, the cells kept from her, her spot.
type Spot = (i32, &'static [(u16, u16)], Option<i32>);

/// Where she stands to gaze up at her plant (`66..69` under its run
/// `63..73`), as the floor's text leaves her room: her box one column
/// clear of it, left (`63`), else right (`71`); else two columns clear,
/// where that's still under its run (`72`, not `62`: the run's first cell
/// is `63`); else nowhere. A cell kept from her on her box's rows (row
/// 10) makes a spot unrestful: at `65` only `63`'s box (`61..=65`), at
/// `69` only `71`'s (`69..=73`), at `74` only `72`'s (`70..=74`). Her
/// start, then the cells, then the spot.
const SPOTS: [Spot; 4] = [
    (56, &[], Some(63)),
    (80, &[(65, 10)], Some(71)),
    (80, &[(65, 10), (69, 10)], Some(72)),
    (80, &[(65, 10), (69, 10), (74, 10)], None),
];

/// Her plant up on her shelf, she on its room's floor and offered only
/// her gaze up: the frame offers her a seat beside it, facing it
/// (whichever way it was set down), on the side and as near as the
/// floor's text leaves her room ([`SPOTS`]); she chooses it, walks
/// there, and gazes up: pleased ("hehe", as admiring what she's made)
/// for its first moment, then a long curious look up at it, in profile,
/// 20 to 45 s before her mood lingers over it. Where no spot under its
/// run is restful, none is offered. Both modes.
#[test]
fn she_gazes_up_at_what_is_on_her_shelf() {
    for graphics in [false, true] {
        for facing in [Facing::Right, Facing::Left] {
            for (start, marks, spot) in SPOTS {
                let at = format!("graphics={graphics} {facing:?} from {start} {marks:?}");
                let (mut guest, real, view, mut now) =
                    plant_up(5, graphics, facing, marks, (start, USERS_FLOOR));
                // Offered: the frame's seat, and she's on its room's
                // floor.
                let Some(spot) = spot else {
                    assert!(
                        visit_of(&guest)
                            .shown
                            .iter()
                            .any(|s| s.item == Furniture::Plant && s.on_shelf()),
                        "{at}: her plant up"
                    );
                    assert_eq!(gaze_seat(&guest), None, "{at}");
                    continue;
                };
                let seat =
                    gaze_seat(&guest).unwrap_or_else(|| panic!("{at}: nowhere to gaze up from"));
                beside_and_facing(&seat, spot, &at);
                let from = visit_of(&guest).osaka.decisions.len();
                let begun = now;
                let (span, mood) = loop {
                    assert!(now < begun + 30_000, "{at}: she never gazed up");
                    shell_step(&mut guest, &real, &view, &mut now, true);
                    let osaka = &visit_of(&guest).osaka;
                    if let Some(span) = osaka.use_span()
                        && span.0.what == Use::GazeUp
                    {
                        break (span, osaka.mood());
                    }
                };
                assert!(chose_to_gaze(&guest, from), "{at}: chosen");
                let (seat, since, until) = span;
                beside_and_facing(&seat, spot, &at);
                let osaka = &visit_of(&guest).osaka;
                assert_eq!((osaka.x, osaka.y), (seat.x, seat.y), "{at}: at her seat");
                // 20-45 s (D8), drawn before her mood lingers over it.
                let whole = osaka.use_whole().unwrap();
                assert!(
                    whole <= until - since,
                    "{at}: {whole} ms of {}",
                    until - since
                );
                let unit = 1_000_000;
                let times =
                    osaka.stillness.lingered_use(mood, Use::GazeUp, unit) as f64 / unit as f64;
                let drawn = (whole as f64 / times).round() as u64;
                assert!(
                    (20_000..=45_000).contains(&drawn),
                    "{at}: {drawn} ms ({whole} ms lingered ×{times} as {mood:?})"
                );
                // Pleased a moment, then gazing up.
                for t in [since, since + GAZE_UP_PLEASED_MS - 1] {
                    assert_eq!(
                        osaka.appearance(t),
                        (Pose::Stand, Face::Pleased, Some(osaka::Bubble::Hehe)),
                        "{at}: {} ms in",
                        t - since
                    );
                }
                for t in [since + GAZE_UP_PLEASED_MS, (since + until) / 2, until - 1] {
                    assert_eq!(
                        osaka.appearance(t),
                        (Pose::Gaze, Face::Curious, None),
                        "{at}: {} ms in",
                        t - since
                    );
                }
            }
        }
    }
}

/// She gazes up at her shelf only from its room's floor (phase 6a D8: no
/// climb for it): on the Playlist's floor, the frame still offers her the
/// seat on Users' (it's where her plant stands), but she never chooses
/// it; put on Users' floor, she does. Both modes.
#[test]
fn she_gazes_up_only_from_her_shelf_s_room() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, real, view, mut now) =
            plant_up(7, graphics, Facing::Right, &[], (60, PLAYLIST_FLOOR));
        let seat = gaze_seat(&guest).unwrap_or_else(|| panic!("{at}: nowhere to gaze up from"));
        assert_eq!(seat.y, USERS_FLOOR, "{at}");
        let from = visit_of(&guest).osaka.decisions.len();
        let until = now + 20_000;
        while now < until {
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            assert_eq!(osaka.y, PLAYLIST_FLOOR, "{at}: she stays put");
            assert!(gaze_seat(&guest).is_some(), "{at}: still offered");
        }
        assert!(!chose_to_gaze(&guest, from), "{at}: from the Playlist");
        assert!(
            visit_of(&guest).osaka.decisions.len() > from,
            "{at}: she decided meanwhile"
        );
        put(&mut guest, (56, USERS_FLOOR), now);
        let from = visit_of(&guest).osaka.decisions.len();
        let until = now + 20_000;
        while now < until && !chose_to_gaze(&guest, from) {
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        assert!(chose_to_gaze(&guest, from), "{at}: from Users' floor");
    }
}

/// The stage's gaze up (phase 6a D11): on a frame where her shelf goes up
/// with her plant on it ([`rooms_frame`] at 100×30: List takes a high
/// shelf), she's set off to gaze up at it from beside it, and does; on
/// one whose shelf has no room over it for her plant but has for her
/// gremlin (the stage room at 100×30: a low shelf in a 9-row pane, two
/// rows over it; step 7), at her gremlin; on one that takes none
/// (80×24), the stage says nothing she gazes up at is up there, and
/// changes nothing of her home (no shelf, nor her clock up for it); with
/// her plant up but no spot beside it under its run restful, that
/// there's no room beside it. Both modes.
#[test]
fn the_stage_gazes_up_where_her_plant_or_gremlin_goes_up() {
    let stage_room = |w, h| {
        let mut ui = super::super::stage::stage_ui();
        real_frame(&mut ui, w, h)
    };
    let frames: [(&str, (Buffer, IdleView), Option<Furniture>); 3] = [
        ("rooms", rooms_frame(100, 30), Some(Furniture::Plant)),
        ("stage", stage_room(100, 30), Some(Furniture::Gremlin)),
        ("small", stage_room(80, 24), None),
    ];
    for (name, (real, view), up) in frames {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = Guest::new(1);
            if graphics {
                guest.set_picker(kitty());
            }
            guest.cue(Scene::GazeUp);
            paint(&mut guest, &real, &view, 0);
            let note = guest.cue_note().cloned();
            let Some(up) = up else {
                // Nothing she gazes up at goes up there (her clock
                // isn't put up for it either).
                assert!(
                    matches!(&note, Some(Err(e)) if e.contains(NOTHING_UP)),
                    "{at}: {note:?}"
                );
                assert!(
                    !guest.ledger.home.props.iter().any(|p| !p.strip.is_floor()),
                    "{at}: {:?}",
                    guest.ledger.home.props
                );
                assert!(guest.ledger.home.structures.is_empty(), "{at}: a shelf up");
                let anything_up = visit_of(&guest).shown.iter().any(|s| s.on_shelf());
                assert!(!anything_up, "{at}");
                continue;
            };
            assert!(matches!(note, Some(Ok(_))), "{at}: {note:?}");
            let mut now = 0;
            let gazed = loop {
                assert!(now < 15_000, "{at}: she never gazed up");
                now += guest
                    .next_tick(now)
                    .map_or(100, |d| d.as_millis() as u64)
                    .clamp(1, 100);
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
                let osaka = &visit_of(&guest).osaka;
                if osaka.appearance(now).0 == Pose::Gaze {
                    break osaka.use_span();
                }
            };
            let (seat, ..) = gazed.unwrap_or_else(|| panic!("{at}: gazing, not at her shelf"));
            assert_eq!(seat.what, Use::GazeUp, "{at}");
            assert_eq!(seat.piece, PieceRef::Real(up), "{at}");
            let piece = visit_of(&guest)
                .shown
                .iter()
                .find(|s| s.item == up)
                .cloned()
                .unwrap_or_else(|| panic!("{at}: her {up:?}"));
            assert!(piece.on_shelf(), "{at}");
            assert_eq!(seat.y, piece.stood, "{at}: from its room's floor");
        }
    }
    // Her plant up on her Users shelf (as `stage_shelf` keeps a shelf
    // she has: centred, `66..69` under `63..73`), but every spot under its
    // run beside it kept from her ([`SPOTS`]' last): the stage says there's
    // no room beside it.
    for graphics in [false, true] {
        let at = format!("blocked graphics={graphics}");
        let (real, mut view) = rooms_frame(100, 30);
        view.protected
            .extend([(65, 10), (69, 10), (74, 10)].map(|(x, y)| Rect::new(x, y, 1, 1)));
        let mut guest = Guest::new(1);
        if graphics {
            guest.set_picker(kitty());
        }
        guest.ledger.home.structures.push(users_shelf());
        guest.cue(Scene::GazeUp);
        paint(&mut guest, &real, &view, 0);
        let note = guest.cue_note().cloned();
        assert!(
            matches!(&note, Some(Err(e)) if e.contains(NO_ROOM_BESIDE)),
            "{at}: {note:?}"
        );
        let plant = visit_of(&guest)
            .shown
            .iter()
            .find(|s| s.item == Furniture::Plant)
            .cloned()
            .unwrap_or_else(|| panic!("{at}: her plant"));
        assert!(plant.on_shelf(), "{at}");
        assert_eq!(plant.left, PLANT.start, "{at}");
    }
}

/// A resize that moves her plant (the window wider, its height kept: her
/// Users pane, her shelf and her plant shift right, she stays on its
/// floor) moves her off her gaze up: on no frame after it is she gazing
/// up from where the plant was, only from beside where it is (or not at
/// all: startled off it, as when a piece goes away under her). The class
/// is every use whose seat is where a piece stands: none outlives a move
/// of its piece. Both modes.
#[test]
fn a_resize_that_moves_her_plant_moves_her_off_her_gaze() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (mut guest, real, view, mut now) =
            plant_up(5, graphics, Facing::Right, &[], (56, USERS_FLOOR));
        let begun = now;
        while !visit_of(&guest)
            .osaka
            .use_span()
            .is_some_and(|(seat, ..)| seat.what == Use::GazeUp)
        {
            assert!(now < begun + 30_000, "{at}: she never gazed up");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        // A few seconds into it, a wider window.
        let gazing = now;
        while now < gazing + 4_000 {
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let (wide, wide_view) = rooms_frame(110, 30);
        let until = now + 10_000;
        while now < until {
            shell_step(&mut guest, &wide, &wide_view, &mut now, true);
            let visit = visit_of(&guest);
            let Some((seat, ..)) = visit.osaka.use_span() else {
                continue;
            };
            let piece = visit
                .shown
                .iter()
                .find(|s| s.piece() == seat.piece)
                .unwrap_or_else(|| panic!("{at} at {now}: {seat:?} not shown"));
            let half = sprite::WIDTH / 2;
            let (from, to) = (piece.left, piece.left + i32::from(piece.size().0));
            let gap = if seat.x < from {
                from - (seat.x + half)
            } else {
                seat.x - half - to + 1
            };
            assert!(
                seat.what != Use::GazeUp || (1..=2).contains(&gap),
                "{at} at {now}: gazing up from {} at her plant on {from}..{to}",
                seat.x
            );
            assert_eq!(
                (visit.osaka.x, visit.osaka.y),
                (seat.x, seat.y),
                "{at} at {now}: off her seat"
            );
        }
    }
}
