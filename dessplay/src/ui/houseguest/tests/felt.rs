//! Her felt rules `DoorClear` and `InChat`, and the per-frame closet
//! (door batch, step 6; design D7): a piece in her door's space she
//! bumps into coming home, and one in the chat pane she sees, are felt
//! without a use, said once she's quiet after her hello (if her mood
//! would mend it, at most once a game day per rule), and put right; a
//! piece in the chat that no room would take is hidden, never lost.
//! Each in both drawing modes.

use super::*;
use crate::ui::houseguest::brain::Mood;
use crate::ui::houseguest::door::{Fallback, Set};
use crate::ui::houseguest::osaka::Bubble;
use crate::ui::houseguest::room::{Anchor, DoorWall, Prop, Side, Strip};
use sprite::Facing;

/// A piece anchored `offset` from `side`'s wall of `nook`.
type Anchored = (Furniture, Nook, Side, u16, Facing);

/// Her door's wall in the tests here: Users' right, at the screen's edge.
const USERS_RIGHT: DoorWall = DoorWall {
    strip: Strip::Bottom(Nook::Users),
    side: Side::Right,
};

/// [`LAMP_HOME`] with her door by Users' right wall, of a Tuesday
/// afternoon (her door's space yielding to the lamp): the door batch's
/// step 9 sends it a parcel.
pub(super) fn lamp_home(graphics: bool) -> Guest {
    anchored_home(4, tue(13, 0), &LAMP_HOME, Some(USERS_RIGHT), graphics)
}

/// The DoorClear row of the table.
fn door_row() -> usize {
    rules::RULES
        .iter()
        .position(|r| r.rule == rules::Rule::DoorClear)
        .unwrap()
}

/// The InChat row of the table.
fn chat_row() -> usize {
    rules::RULES
        .iter()
        .position(|r| r.rule == rules::Rule::InChat)
        .unwrap()
}

/// What she says, felt on sight, for `row`.
fn line_of(row: usize) -> &'static str {
    rules::RULES[row].grievance
}

/// An older record (her door's wall saved) she has visited once, her
/// clock at `at`, `pieces` anchored as given (settled), on a mid-June
/// school day with her wall clock long since sent: she's absent and
/// arrives as soon as the idle gate opens.
fn anchored_home(
    seed: u64,
    at: routine::GameTime,
    pieces: &[Anchored],
    door: Option<DoorWall>,
    graphics: bool,
) -> Guest {
    let mut ledger = Ledger::new_at(seed, at);
    ledger.clock_sent = true;
    for &(item, nook, side, offset, facing) in pieces {
        assert!(
            ledger.home.add(Prop {
                anchor: Some(Anchor { side, offset }),
                ..Prop::new(item, nook, 0, facing)
            }),
            "{item:?}"
        );
    }
    ledger.home.door = door;
    let mut guest = Guest::restore(ledger);
    guest.set_date(date(2026, 6, 17));
    if graphics {
        guest.set_picker(kitty());
    }
    guest
}

/// A living room on Users ([`chat_by_a_full_space`]'s screen: List
/// beside it, the chat pane below it) whose pieces fill 25 of the
/// strip's 28 columns: the fridge, sofa, cat bed and plant against the
/// left wall, and the lamp in the corner by her door (Users' right, the
/// screen's edge), in her door's space (columns 93..=98). Without the
/// lamp they'd pack beside the space (22 columns); with it, the space
/// yields. The plant (no use either) stands beside the space, outside
/// it.
const LAMP_HOME: [Anchored; 5] = [
    (Furniture::Fridge, Nook::Users, Side::Left, 0, Facing::Right),
    (Furniture::Sofa, Nook::Users, Side::Left, 4, Facing::Right),
    (
        Furniture::CatBed,
        Nook::Users,
        Side::Left,
        13,
        Facing::Right,
    ),
    (Furniture::Plant, Nook::Users, Side::Left, 19, Facing::Right),
    (Furniture::Lamp, Nook::Users, Side::Right, 0, Facing::Right),
];

/// List and Users as [`chat_by_a_full_space`] has them, the chat pane
/// laid over Users' left end instead (columns 70..=81, a custom layout's
/// grid overlapping the pane; not drawn), so a piece against Users' left
/// wall stands in it. With `list` false, List is no nook of hers (a pane
/// she can't furnish).
fn chat_over_users(list: bool, chat_cols: u16) -> (Buffer, IdleView) {
    let list_rect = Rect::new(0, 0, 70, 27);
    let users = Rect::new(70, 0, 30, 14);
    let mut nooks = vec![(Nook::Users, users)];
    if list {
        nooks.insert(0, (Nook::List, list_rect));
    }
    let view = IdleView {
        chat: Rect::new(70, 0, chat_cols, 14),
        nooks,
        ..view(bottom_strip(100, 30))
    };
    (boxes_screen(&[list_rect, users]), view)
}

/// Her pieces of `items` as her record has them (a first TV, ordered
/// for her on her second visit, comes in on its own).
fn record_of(guest: &Guest, items: &[Anchored]) -> Vec<Prop> {
    guest
        .ledger
        .home
        .props
        .iter()
        .filter(|p| items.iter().any(|a| a.0 == p.item))
        .copied()
        .collect()
}

/// The plan `view` lays her home out on, over `real`.
fn plan_on<'a>(real: &Buffer, view: &'a IdleView) -> room::Plan<'a> {
    room::Plan {
        nooks: &view.nooks,
        chat: view.chat,
        screen: real.area,
    }
}

/// The rules her home breaks on `view`, as a frame judges them.
fn broken_on(guest: &Guest, real: &Buffer, view: &IdleView) -> Vec<rules::Grievance> {
    let mut home = guest.ledger.home.clone();
    let laid = home.laid_out(&view.nooks);
    let keep = door::Keep::of(&home, plan_on(real, view));
    rules::broken(&laid, &home, &keep)
        .into_iter()
        .map(|b| b.key)
        .collect()
}

/// Her door's space on Users, as her home lays it out on `view`.
pub(super) fn users_space(guest: &Guest, view: &IdleView) -> room::Space {
    guest
        .ledger
        .home
        .extents(&view.nooks)
        .into_iter()
        .find(|p| p.strip == Strip::Bottom(Nook::Users))
        .and_then(|p| p.space)
        .expect("a space on Users")
}

/// From `from`, until her home stands empty (she's out at school); a
/// step more so a frame has painted it. When.
fn until_out(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64) -> u64 {
    let mut now = from;
    paint(guest, real, view, now);
    while !matches!(guest.state, State::Away(_)) {
        assert!(now < from + 120_000, "her home never stood empty");
        shell_step(guest, real, view, &mut now, true);
    }
    shell_step(guest, real, view, &mut now, true);
    now
}

/// From `now` (she's out at school), on to 12:45 and her coming home:
/// when she's first visiting, her mood set to `mood` at once.
fn home_at_1245_in(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    mut now: u64,
    mood: Mood,
) -> u64 {
    let home = real_of(guest, now, tue(12, 45));
    guest.skip_clock(now);
    while !matches!(guest.state, State::Visiting(_)) {
        assert!(now < home + 20_000, "she never came home");
        shell_step(guest, real, view, &mut now, true);
    }
    let State::Visiting(visit) = &mut guest.state else {
        unreachable!()
    };
    visit.osaka.set_mood(mood);
    now
}

/// Until she's visiting from the idle gate, her mood set to `mood` at
/// once. When.
fn visiting_in(guest: &mut Guest, real: &Buffer, view: &IdleView, from: u64, mood: Mood) -> u64 {
    let now = until_visiting(guest, real, view, from);
    let State::Visiting(visit) = &mut guest.state else {
        unreachable!()
    };
    visit.osaka.set_mood(mood);
    now
}

/// Her lamp in her door's space (an older record: the space yields for
/// it, [`LAMP_HOME`]): coming home from school through her door, which
/// stands aside for it (`Fallback::Yield`), she has bumped into it, feels
/// DoorClear without a use of the lamp, says "Can't get to the door!"
/// after her "I'm home!", and lifts the lamp to List; the next frame her
/// door's space is kept and her door stands in it (face-on until step 8).
/// Industrious (she mends). Red before step 6: no rule.
#[test]
fn a_lamp_in_her_door_space_is_felt_and_moved() {
    let (real, view) = chat_by_a_full_space();
    let door = door_row();
    let lamp = rules::Grievance {
        row: door,
        piece: Furniture::Lamp,
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(4, tue(9, 0), &LAMP_HOME, Some(USERS_RIGHT), graphics);
        // The record packs on the raw strip, not beside the space; with
        // the lamp gone it would.
        let raw = room::raw_strips(&view.nooks)
            .into_iter()
            .find(|(s, _)| *s == Strip::Bottom(Nook::Users))
            .unwrap()
            .1;
        let cols: i32 = LAMP_HOME
            .iter()
            .map(|p| i32::from(p.0.spec().footprint.0))
            .sum();
        let width = raw.to - raw.from;
        assert!(
            (width - 5..=width - 3).contains(&cols),
            "{at}: {cols} of {width}"
        );
        let space = users_space(&guest, &view);
        assert!(!space.kept, "{at}: the space yields");
        assert_eq!(
            broken_on(&guest, &real, &view),
            vec![lamp],
            "{at}: only the lamp"
        );
        let now = until_out(&mut guest, &real, &view, 0);
        let now = home_at_1245_in(&mut guest, &real, &view, now, Mood::Industrious);
        let arrival = visit_of(&guest).door.expect("her door");
        assert_eq!(
            arrival.set(),
            Set::Floor(Fallback::Yield),
            "{at}: {arrival:?}"
        );
        // Until the lamp has gone from her door's strip.
        let moved = run_until(&mut guest, &real, &view, now, now + 240_000, |guest, _| {
            guest
                .ledger
                .home
                .props
                .iter()
                .any(|p| p.item == Furniture::Lamp && p.strip != Strip::Bottom(Nook::Users))
        });
        let moved = moved.unwrap_or_else(|| panic!("{at}: the lamp never moved"));
        let osaka = &visit_of(&guest).osaka;
        assert!(
            osaka.felt().contains(&lamp),
            "{at}: felt {:?}",
            osaka.felt()
        );
        let home_line = osaka
            .said_lines()
            .iter()
            .find(|(_, line, _)| mind::HOME.lines.contains(line))
            .copied()
            .unwrap_or_else(|| panic!("{at}: never said she's home"));
        let said = osaka.said_aloud().to_vec();
        assert_eq!(said.len(), 1, "{at}: {said:?}");
        assert_eq!(said[0].0, line_of(door), "{at}");
        assert!(
            said[0].1 >= home_line.2 + osaka::speech_ms(home_line.1),
            "{at}: said at {} over her hello at {}",
            said[0].1,
            home_line.2
        );
        assert!(said[0].1 < moved, "{at}: said before she moved it");
        // The next frame: the space kept, her door in it.
        let next = moved + 100;
        guest.advance(next);
        paint(&mut guest, &real, &view, next);
        assert!(users_space(&guest, &view).kept, "{at}: the space kept");
        let door_now = visit_of(&guest).door.expect("her door");
        assert_eq!(
            door_now.set(),
            Set::Wall {
                side: Side::Right,
                wall: space.wall
            },
            "{at}: {door_now:?}"
        );
        assert!(
            !broken_on(&guest, &real, &view).contains(&lamp),
            "{at}: mended"
        );
    }
}

/// [`LAMP_HOME`] with a TV, desk and lamp against the left wall and her
/// sofa in the corner by her door, facing the TV (13 cells apart): the
/// sofa fills her door's space.
const SOFA_HOME: [Anchored; 4] = [
    (Furniture::Tv, Nook::Users, Side::Left, 0, Facing::Right),
    (Furniture::Desk, Nook::Users, Side::Left, 6, Facing::Right),
    (Furniture::Lamp, Nook::Users, Side::Left, 13, Facing::Right),
    (Furniture::Sofa, Nook::Users, Side::Right, 0, Facing::Left),
];

/// Her sofa in her door's space: she needn't have bumped into it; using
/// it (lounging), she feels DoorClear as any rule felt on a use, saying
/// "Can't get to the door!" from the sofa (the `ANY_USE` path).
/// Industrious. Red before step 6: no rule.
#[test]
fn a_sofa_in_her_door_space_is_felt_on_use() {
    let (real, view) = chat_by_a_full_space();
    let sofa = rules::Grievance {
        row: door_row(),
        piece: Furniture::Sofa,
    };
    let line = line_of(door_row());
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(2, tue(14, 0), &SOFA_HOME, Some(USERS_RIGHT), graphics);
        assert!(!users_space(&guest, &view).kept, "{at}: the space yields");
        assert_eq!(broken_on(&guest, &real, &view), vec![sofa], "{at}");
        let now = visiting_in(&mut guest, &real, &view, 0, Mood::Industrious);
        assert!(
            !visit_of(&guest).osaka.bumped(),
            "{at}: came in on the idle gate"
        );
        guest.cue(Scene::Lounge);
        let said = run_until(&mut guest, &real, &view, now, now + 60_000, |guest, now| {
            let osaka = &visit_of(guest).osaka;
            osaka
                .use_span()
                .is_some_and(|(seat, ..)| seat.item == Furniture::Sofa)
                && look_now(guest, now).1 == Some(Bubble::Say(line))
        });
        let said = said.unwrap_or_else(|| panic!("{at}: never said it on the sofa"));
        let _ = run(
            &mut guest,
            &real,
            &view,
            said,
            said + osaka::GRIEVANCE_MS + 100,
        );
        assert!(felt(&guest).contains(&sofa), "{at}: {:?}", felt(&guest));
    }
}

/// A plant against Users' left wall, in the chat pane laid over that end
/// of the pane: seeing it as she visits, she feels InChat (no use of the
/// plant), says "People are talking here…" once she's quiet, and moves
/// it out of the chat. Industrious. Red before step 6: no rule.
#[test]
fn a_piece_in_the_chat_is_moved_out() {
    let (real, view) = chat_over_users(true, 12);
    let chat = view.chat;
    let plant = rules::Grievance {
        row: chat_row(),
        piece: Furniture::Plant,
    };
    let pieces = [(Furniture::Plant, Nook::Users, Side::Left, 0, Facing::Right)];
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(3, tue(14, 0), &pieces, Some(USERS_RIGHT), graphics);
        assert_eq!(broken_on(&guest, &real, &view), vec![plant], "{at}");
        let now = visiting_in(&mut guest, &real, &view, 0, Mood::Industrious);
        let shows = visit_of(&guest)
            .shown
            .iter()
            .any(|s| s.item == Furniture::Plant && room::in_chat(chat, s.cover()));
        assert!(shows, "{at}: shown where it stands");
        let out = run_until(&mut guest, &real, &view, now, now + 180_000, |guest, _| {
            let mut home = guest.ledger.home.clone();
            home.laid_out(&view.nooks)
                .shown
                .iter()
                .any(|s| s.item == Furniture::Plant && !room::in_chat(chat, s.cover()))
        });
        let out = out.unwrap_or_else(|| panic!("{at}: never moved out of the chat"));
        let osaka = &visit_of(&guest).osaka;
        assert!(osaka.felt().contains(&plant), "{at}");
        let said = osaka.said_aloud();
        assert_eq!(said.len(), 1, "{at}: {said:?}");
        assert_eq!(said[0].0, line_of(chat_row()), "{at}");
        assert!(said[0].1 < out, "{at}");
        assert!(!broken_on(&guest, &real, &view).contains(&plant), "{at}");
    }
}

/// A bed against Users' left wall in a chat pane over all but Users'
/// last seven columns, Users her only nook: no room takes it, so it's in
/// the closet (hidden) while she visits and while she's out; List
/// becoming a nook of hers, it shows again where it stands (in the chat:
/// there's room for it now, for her to move it), and hidden again with
/// List gone. Her record never changes (Lazy, so she moves nothing).
/// Red before step 6: shown in the chat.
#[test]
fn a_piece_in_the_chat_with_nowhere_else_is_closeted_never_lost() {
    let (real, hidden) = chat_over_users(false, 22);
    let (_, roomy) = chat_over_users(true, 22);
    let chat = hidden.chat;
    let pieces = [(Furniture::Bed, Nook::Users, Side::Left, 0, Facing::Right)];
    let bed_shown = |shown: &[Shown]| shown.iter().any(|s| s.item == Furniture::Bed);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        // Visiting.
        let mut guest = anchored_home(5, tue(14, 0), &pieces, Some(USERS_RIGHT), graphics);
        let record = record_of(&guest, &pieces);
        let laid = guest.ledger.home.clone().laid_out(&hidden.nooks);
        let bed = laid
            .shown
            .iter()
            .find(|s| s.item == Furniture::Bed)
            .unwrap();
        assert!(room::in_chat(chat, bed.cover()), "{at}: in the chat");
        let mut now = visiting_in(&mut guest, &real, &hidden, 0, Mood::Lazy);
        for (view, shows, what) in [
            (&hidden, false, "no room"),
            (&roomy, true, "List's room"),
            (&hidden, false, "no room again"),
        ] {
            now += 1000;
            guest.advance(now);
            paint(&mut guest, &real, view, now);
            assert_eq!(
                bed_shown(&visit_of(&guest).shown),
                shows,
                "{at} visiting: {what}"
            );
            assert!(
                !visit_of(&guest)
                    .broken
                    .iter()
                    .any(|b| b.row == chat_row() && !shows),
                "{at}: a hidden piece isn't judged in the chat"
            );
            assert_eq!(record_of(&guest, &pieces), record, "{at}: {what}");
        }
        // Away.
        let mut guest = anchored_home(5, tue(9, 0), &pieces, Some(USERS_RIGHT), graphics);
        let mut now = until_out(&mut guest, &real, &hidden, 0);
        for (view, shows, what) in [
            (&hidden, false, "no room"),
            (&roomy, true, "List's room"),
            (&hidden, false, "no room again"),
        ] {
            now += 1000;
            guest.advance(now);
            paint(&mut guest, &real, view, now);
            let State::Away(empty) = &guest.state else {
                panic!("{at}: out");
            };
            assert_eq!(bed_shown(&empty.shown), shows, "{at} away: {what}");
            assert_eq!(record_of(&guest, &pieces), record, "{at}: {what}");
        }
    }
}

/// An older box in the chat pane with nowhere else to go (its bed laid
/// against Users' left wall under a chat over most of Users, Users her
/// only nook; a layout change put it there, or she was sent it before
/// the chat moved): a box is never closeted, so it shows where it
/// stands while she's out, and visiting she unpacks it (then it's a bed
/// with no room, in the closet). It never stays boxed, so the shopping
/// channel isn't stopped for good (`advert` sells nothing while a piece
/// is boxed). Red before the fix: the box was closeted, hidden from her
/// and from her seats, boxed for as long as the layout stood.
#[test]
fn a_box_in_the_chat_is_shown_and_unpacked() {
    let (real, view) = chat_over_users(false, 22);
    let chat = view.chat;
    let pieces = [(Furniture::Bed, Nook::Users, Side::Left, 0, Facing::Right)];
    let boxed = |guest: &Guest| prop_of(guest, Furniture::Bed).boxed;
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let boxed_home = |at_time| {
            let mut guest = anchored_home(5, at_time, &pieces, Some(USERS_RIGHT), graphics);
            for p in &mut guest.ledger.home.props {
                p.boxed = true;
                p.settled = false;
            }
            guest
        };
        // Precondition: in the chat, and no room would take it out of
        // its box either.
        let mut probe = boxed_home(tue(14, 0));
        probe.ledger.home.props[0].boxed = false;
        let mut home = probe.ledger.home.clone();
        let laid = home.laid_out(&view.nooks);
        assert!(room::in_chat(chat, laid.shown[0].cover()), "{at}");
        assert_eq!(room::stranded(&home, &laid, chat), vec![0], "{at}");
        // Away: the box shows.
        let mut guest = boxed_home(tue(9, 0));
        let now = until_out(&mut guest, &real, &view, 0);
        paint(&mut guest, &real, &view, now);
        let State::Away(empty) = &guest.state else {
            panic!("{at}: out");
        };
        assert!(
            empty
                .shown
                .iter()
                .any(|s| s.item == Furniture::Bed && s.boxed),
            "{at}: the box shows while she's out"
        );
        // Visiting: she unpacks it.
        let mut guest = boxed_home(tue(14, 0));
        let now = visiting_in(&mut guest, &real, &view, 0, Mood::Ordinary);
        assert!(
            visit_of(&guest)
                .shown
                .iter()
                .any(|s| s.item == Furniture::Bed && s.boxed),
            "{at}: the box shows"
        );
        let unpacked = run_until(&mut guest, &real, &view, now, now + 10 * 60_000, |g, _| {
            !boxed(g)
        });
        assert!(unpacked.is_some(), "{at}: never unpacked");
        let now = unpacked.unwrap() + 1000;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        assert!(
            !visit_of(&guest)
                .shown
                .iter()
                .any(|s| s.item == Furniture::Bed),
            "{at}: out of its box, a bed with no room: in the closet"
        );
    }
}

/// A piece in the chat is felt on sight only once she's in sight (D7):
/// coming home from school through her door, unseen in the doorway, she
/// feels nothing; the first frame she has felt the plant in the chat
/// she shows. Red under the door batch's step 6 review's mutant J
/// (feeling while hidden).
#[test]
fn a_piece_in_the_chat_is_felt_only_once_she_is_in_sight() {
    let (real, view) = chat_over_users(true, 12);
    let plant = rules::Grievance {
        row: chat_row(),
        piece: Furniture::Plant,
    };
    let pieces = [(Furniture::Plant, Nook::Users, Side::Left, 0, Facing::Right)];
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(3, tue(9, 0), &pieces, Some(USERS_RIGHT), graphics);
        assert_eq!(broken_on(&guest, &real, &view), vec![plant], "{at}");
        let now = until_out(&mut guest, &real, &view, 0);
        let mut now = home_at_1245_in(&mut guest, &real, &view, now, Mood::Ordinary);
        let mut unseen = 0;
        loop {
            let osaka = &visit_of(&guest).osaka;
            let hidden = osaka.hidden(now);
            if osaka.felt().contains(&plant) {
                assert!(!hidden, "{at}: felt at {now}, out of sight");
                break;
            }
            if hidden {
                unseen += 1;
            }
            assert!(now < 120_000_000, "{at}: never felt");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        assert!(unseen > 0, "{at}: she came home unseen a while");
    }
}

/// What she said aloud is her home's: moved out, a new tenant's home
/// that breaks the same rule on the same game day says it again (her
/// day's "once" isn't the old home's). Red under the door batch's step 6
/// review's mutant H (`said_aloud` kept across `move_out`).
#[test]
fn a_new_tenant_says_it_on_the_same_day() {
    let (real, view) = chat_over_users(true, 12);
    let pieces = [(Furniture::Plant, Nook::Users, Side::Left, 0, Facing::Right)];
    let line = line_of(chat_row());
    let said = |guest: &Guest| {
        visit_of(guest)
            .osaka
            .said_aloud()
            .iter()
            .map(|&(l, _)| l)
            .collect::<Vec<_>>()
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(3, tue(14, 0), &pieces, Some(USERS_RIGHT), graphics);
        let now = visiting_in(&mut guest, &real, &view, 0, Mood::Industrious);
        let day = visit_of(&guest).osaka.day(now).expect("a routine").day;
        let said_at = run_until(&mut guest, &real, &view, now, now + 60_000, |g, _| {
            !said(g).is_empty()
        });
        assert!(said_at.is_some(), "{at}: said by the first tenant");
        // She moves out; her successor's home (the same, on the same day)
        // comes in.
        guest.move_out(4);
        let next = anchored_home(3, tue(14, 0), &pieces, Some(USERS_RIGHT), graphics);
        guest.ledger = next.ledger;
        let now = visiting_in(
            &mut guest,
            &real,
            &view,
            said_at.unwrap() + 1000,
            Mood::Industrious,
        );
        assert_eq!(
            visit_of(&guest).osaka.day(now).map(|d| d.day),
            Some(day),
            "{at}: the same day"
        );
        let again = run_until(&mut guest, &real, &view, now, now + 60_000, |g, _| {
            said(g) == [line]
        });
        assert!(again.is_some(), "{at}: said again in the new home");
    }
}

/// With no routine reaching her (an unfed clock: no game day), "once a
/// game day" is once a visit (`Said::Visit`): a piece in the chat she
/// says so of on one visit, she says so of again on the next.
#[test]
fn unfed_she_says_it_once_a_visit() {
    let (real, view) = chat_over_users(true, 12);
    let pieces = [(Furniture::Plant, Nook::Users, Side::Left, 0, Facing::Right)];
    let line = line_of(chat_row());
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(3, tue(14, 0), &pieces, Some(USERS_RIGHT), graphics).unfed();
        let mut now = 0;
        for visit in 0..2 {
            now = visiting_in(&mut guest, &real, &view, now, Mood::Industrious);
            assert_eq!(visit_of(&guest).osaka.day(now), None, "{at}: no game day");
            let said = run_until(&mut guest, &real, &view, now, now + 60_000, |g, _| {
                visit_of(g)
                    .osaka
                    .said_aloud()
                    .iter()
                    .any(|&(l, _)| l == line)
            });
            assert!(said.is_some(), "{at}: said on visit {visit}");
            now = said.unwrap();
            // Input: she leaves, to come back on the next quiet.
            guest.activity(now);
            let end = now + 60_000;
            while matches!(guest.state, State::Visiting(_) | State::Leaving(_)) {
                assert!(now < end, "{at}: never left");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
        }
    }
}

/// On a short terminal (the bundled layout, 18 to 22 rows), where her
/// door's strip has no room for its space, she never feels her door
/// blocked: no DoorClear broken while her door stands at the `Short`
/// fallback, though her cat's bed stands where the space would be (at
/// Users' right wall: asserted on some frame), none felt, nothing said,
/// nothing moved for it. A guard (DoorClear is judged only where a space
/// exists): a mutant judging the would-be space on a short strip is red.
#[test]
fn on_a_short_terminal_she_never_feels_her_door_blocked() {
    let owned = [(Furniture::CatBed, Nook::Users, 1000)];
    let (mut shorts, mut where_it_would_be) = (0, 0);
    for height in 18..=22 {
        let (real, view) = real_frame(&mut real_ui(), 100, height);
        for graphics in [false, true] {
            let at = format!("H {height} graphics={graphics}");
            let mut guest = home_at(4, tue(9, 0), &owned, graphics);
            guest.ledger.home.door = Some(USERS_RIGHT);
            let now = until_out(&mut guest, &real, &view, 0);
            let now = home_at_1245_in(&mut guest, &real, &view, now, Mood::Industrious);
            // Her cat's bed (a first TV comes in on its own).
            let bed = |guest: &Guest| {
                guest
                    .ledger
                    .home
                    .props
                    .iter()
                    .find(|p| p.item == Furniture::CatBed)
                    .copied()
            };
            let record = bed(&guest);
            let short = |guest: &Guest| {
                visit_of(guest)
                    .door
                    .is_some_and(|d| d.set() == Set::Floor(Fallback::Short))
            };
            let _ = run_until(&mut guest, &real, &view, now, now + 30_000, |guest, _| {
                let visit = visit_of(guest);
                if short(guest) {
                    shorts += 1;
                    assert!(
                        !visit.broken.iter().any(|b| b.row == door_row()),
                        "{at}: {:?}",
                        visit.broken
                    );
                    // Where the space would stand at Users' right wall.
                    let users = room::raw_strips(&view.nooks)
                        .into_iter()
                        .find(|(s, _)| *s == USERS_RIGHT.strip)
                        .map(|(_, e)| e);
                    let would = users.and_then(|e| room::space_rect(e, Side::Right));
                    let laid = guest.ledger.home.clone().laid_out(&view.nooks).shown;
                    if laid.iter().any(|s| {
                        s.strip == Some(USERS_RIGHT.strip)
                            && would.is_some_and(|r| r.intersects(s.cover()))
                    }) {
                        where_it_would_be += 1;
                    }
                }
                false
            });
            let osaka = &visit_of(&guest).osaka;
            if short(&guest) {
                assert!(
                    !osaka.felt().iter().any(|k| k.row == door_row()),
                    "{at}: {:?}",
                    osaka.felt()
                );
                assert!(osaka.said_aloud().is_empty(), "{at}");
                assert_eq!(bed(&guest), record, "{at}: nothing moved");
            }
        }
    }
    assert!(shorts > 0, "a short terminal's door stood at the fallback");
    assert!(where_it_would_be > 0, "a piece where the space would stand");
}

/// A Lazy Osaka lives with it (design D7, M20): coming home through her
/// door where it stood aside for the lamp, the space keeps yielding;
/// a piece in the chat pane she sees keeps showing there. She feels both
/// (on sight), but says nothing, and nothing's lost or moved.
#[test]
fn a_lazy_osaka_lives_with_it() {
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        // The lamp in her door's space.
        let (real, view) = chat_by_a_full_space();
        let mut guest = anchored_home(4, tue(9, 0), &LAMP_HOME, Some(USERS_RIGHT), graphics);
        let record = record_of(&guest, &LAMP_HOME);
        let now = until_out(&mut guest, &real, &view, 0);
        let now = home_at_1245_in(&mut guest, &real, &view, now, Mood::Lazy);
        let _ = run_until(&mut guest, &real, &view, now, now + 60_000, |guest, _| {
            assert!(!users_space(guest, &view).kept, "{at}: the space yields");
            false
        });
        let osaka = &visit_of(&guest).osaka;
        assert!(osaka.bumped(), "{at}: bumped into it");
        assert!(
            osaka.felt().iter().any(|k| k.row == door_row()),
            "{at}: {:?}",
            osaka.felt()
        );
        assert!(osaka.said_aloud().is_empty(), "{at}");
        assert_eq!(record_of(&guest, &LAMP_HOME), record, "{at}");
        // The plant in the chat.
        let (real, view) = chat_over_users(true, 12);
        let chat = view.chat;
        let pieces = [(Furniture::Plant, Nook::Users, Side::Left, 0, Facing::Right)];
        let mut guest = anchored_home(3, tue(14, 0), &pieces, Some(USERS_RIGHT), graphics);
        let record = record_of(&guest, &pieces);
        let now = visiting_in(&mut guest, &real, &view, 0, Mood::Lazy);
        let _ = run_until(&mut guest, &real, &view, now, now + 60_000, |guest, _| {
            let shows = visit_of(guest)
                .shown
                .iter()
                .any(|s| s.item == Furniture::Plant && room::in_chat(chat, s.cover()));
            assert!(shows, "{at}: shown in the chat");
            false
        });
        let osaka = &visit_of(&guest).osaka;
        assert!(
            osaka.felt().iter().any(|k| k.row == chat_row()),
            "{at}: {:?}",
            osaka.felt()
        );
        assert!(osaka.said_aloud().is_empty(), "{at}");
        assert_eq!(record_of(&guest, &pieces), record, "{at}");
    }
}

/// What she says never talks over her hello (C4): home from school out
/// of her door where it stood aside for the lamp ([`LAMP_HOME`]), her
/// "I'm home!" shows whole, every frame of it, before "Can't get to the
/// door!", which then shows whole too, every frame of its
/// [`osaka::GRIEVANCE_MS`] (a frame shows one line, so none shows two).
/// And home out of the fallback her door takes with its pane focused
/// (`Fallback::Protected`, which no piece of hers forced: [`LAMP_HOME`]
/// without the lamp, its space kept; with it, the space yields first),
/// she has bumped into nothing: nothing felt, nothing said (a guard:
/// `bumped` is set only for `Fallback::Yield`).
#[test]
fn her_grievance_never_talks_over_her_hello() {
    let (real, view) = chat_by_a_full_space();
    let door_line = line_of(door_row());
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(4, tue(9, 0), &LAMP_HOME, Some(USERS_RIGHT), graphics);
        let now = until_out(&mut guest, &real, &view, 0);
        let now = home_at_1245_in(&mut guest, &real, &view, now, Mood::Industrious);
        let mut frames: Vec<(u64, Option<Bubble>)> = Vec::new();
        let _ = run_until(&mut guest, &real, &view, now, now + 90_000, |guest, now| {
            frames.push((now, look_now(guest, now).1));
            !visit_of(guest).osaka.said_aloud().is_empty()
                && frames.last().is_some_and(|&(t, _)| {
                    t > visit_of(guest).osaka.said_aloud()[0].1 + osaka::GRIEVANCE_MS
                })
        });
        let osaka = &visit_of(&guest).osaka;
        let (home_line, home_from) = osaka
            .said_lines()
            .iter()
            .find(|(_, line, _)| mind::HOME.lines.contains(line))
            .map(|&(_, line, at)| (line, at))
            .unwrap_or_else(|| panic!("{at}: never said she's home"));
        let said = osaka.said_aloud().to_vec();
        assert_eq!(said.len(), 1, "{at}: {said:?}");
        let (_, from) = said[0];
        let home_to = home_from + osaka::speech_ms(home_line);
        for &(t, bubble) in &frames {
            if (home_from..home_to).contains(&t) {
                assert_eq!(
                    bubble,
                    Some(Bubble::Say(home_line)),
                    "{at} t={t}: her hello"
                );
            }
            if (from..from + osaka::GRIEVANCE_MS).contains(&t) {
                assert_eq!(bubble, Some(Bubble::Say(door_line)), "{at} t={t}: whole");
            }
            if t < from {
                assert_ne!(bubble, Some(Bubble::Say(door_line)), "{at} t={t}: early");
            }
        }
        assert!(from >= home_to, "{at}: {from} over her hello to {home_to}");
        let during = frames
            .iter()
            .filter(|(t, _)| (from..from + osaka::GRIEVANCE_MS).contains(t))
            .count();
        assert!(during > 0, "{at}: a frame showed it");
        // Her door's pane focused as she comes home: the fallback no piece
        // of hers forced.
        let users = view.nooks[1].1;
        let focused = IdleView {
            resident: true,
            busy: Some(Busy::Playing),
            focus: Some(users),
            ..view.clone()
        };
        let kept = &LAMP_HOME[..4];
        let mut guest = anchored_home(4, tue(9, 0), kept, Some(USERS_RIGHT), graphics);
        assert!(
            users_space(&guest, &view).kept,
            "{at}: kept without the lamp"
        );
        let now = until_out(&mut guest, &real, &focused, 0);
        let now = home_at_1245_in(&mut guest, &real, &focused, now, Mood::Industrious);
        let arrival = visit_of(&guest).door.expect("her door");
        assert_eq!(
            arrival.set(),
            Set::Floor(Fallback::Protected),
            "{at}: {arrival:?}"
        );
        let _ = run(&mut guest, &real, &focused, now, now + 60_000);
        let osaka = &visit_of(&guest).osaka;
        assert!(!osaka.bumped(), "{at}");
        assert!(osaka.felt().is_empty(), "{at}: {:?}", osaka.felt());
        assert!(osaka.said_aloud().is_empty(), "{at}");
    }
}

/// Her sofa half in the chat pane (laid over Users' left end), facing
/// her TV: no move of hers puts it right (away from the TV it couldn't
/// face it; along the strip, the TV would be pushed into the chat or the
/// sofa into her door's space), though Users takes it (so it shows). She
/// feels InChat on sight on every visit; says so at most once a game
/// day: on the first of two visits on Tuesday, not the second, and
/// again on Wednesday's. Ordinary. Red before step 6: no rule.
#[test]
fn an_on_sight_line_is_said_once_a_day() {
    let (real, view) = chat_over_users(true, 12);
    let pieces = [
        (Furniture::Sofa, Nook::Users, Side::Left, 4, Facing::Right),
        (Furniture::Tv, Nook::Users, Side::Left, 15, Facing::Right),
    ];
    let sofa = rules::Grievance {
        row: chat_row(),
        piece: Furniture::Sofa,
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = anchored_home(7, tue(14, 0), &pieces, Some(USERS_RIGHT), graphics);
        assert_eq!(broken_on(&guest, &real, &view), vec![sofa], "{at}");
        let mut now = 0;
        let mut visits = Vec::new();
        for visit in 0..3 {
            if visit == 2 {
                // On to Wednesday afternoon, while she's away.
                for _ in 0..40 {
                    let label = guest.clock_label(now).unwrap_or_default();
                    if label.starts_with("Wed") && label.ends_with("Afternoon") {
                        break;
                    }
                    guest.skip_clock(now);
                }
                let label = guest.clock_label(now).unwrap_or_default();
                assert!(label.starts_with("Wed"), "{at}: {label}");
            }
            now = visiting_in(&mut guest, &real, &view, now, Mood::Ordinary);
            let day = visit_of(&guest).osaka.day(now).expect("a routine").day;
            let end = now + 60_000;
            let _ = run(&mut guest, &real, &view, now, end);
            now = end;
            let osaka = &visit_of(&guest).osaka;
            assert!(osaka.felt().contains(&sofa), "{at} visit {visit}: felt");
            let lines: Vec<&str> = osaka.said_aloud().iter().map(|&(l, _)| l).collect();
            visits.push((day, lines));
            // Input: she leaves, to come back on the next quiet.
            guest.activity(now);
            while matches!(guest.state, State::Visiting(_) | State::Leaving(_)) {
                assert!(now < end + 60_000, "{at}: never left");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
        }
        let line = line_of(chat_row());
        assert_eq!(
            visits,
            vec![(1, vec![line]), (1, vec![]), (2, vec![line])],
            "{at}"
        );
    }
}
