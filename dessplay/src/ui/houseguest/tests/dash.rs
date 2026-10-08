//! Dashes home (phase 5b D3a, A16, Q2; step 5c): now and then on a
//! school day she dashes home through her closed door for something she
//! forgot (her lunch, from her fridge; with none, she can't think what),
//! and goes out again by her door. An errand at school is a dash too. A
//! dash never counts as a visit, and brings no parcel. Each in both
//! drawing modes, on quiet and text-dense screens where drawing matters.

use super::*;
use crate::ui::houseguest::art::PieceState;
use crate::ui::houseguest::osaka::Bubble;
use crate::ui::houseguest::script::{FORGOT_LUNCH, FORGOT_SOMETHING, ScriptId, WHAT_WAS_IT};

/// Her home on [`home_screen`] with a fridge: the sofa and TV in the
/// Users pane, the bed and fridge in the Playlist pane.
const FRIDGE_HOME: [(Furniture, Nook, u16); 4] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Tv, Nook::Users, 800),
    (Furniture::Bed, Nook::Playlist, 200),
    (Furniture::Fridge, Nook::Playlist, 800),
];

/// Her home on [`home_screen`] with no fridge.
const BARE_HOME: [(Furniture, Nook, u16); 3] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Tv, Nook::Users, 800),
    (Furniture::Bed, Nook::Playlist, 300),
];

/// Tuesday, game day 1: a school day.
const TUESDAY: u64 = 1;

/// A master seed from `from` on whose Tuesday has a dash home, and its
/// minute.
fn dash_seed(from: u64) -> (u64, u16) {
    (from..from + 10_000)
        .find_map(|seed| brain::dash(seed, TUESDAY).map(|m| (seed, m)))
        .expect("a Tuesday with a dash home")
}

/// `minute` (since midnight) of Tuesday.
fn tue_at(minute: u16) -> routine::GameTime {
    routine::GameTime {
        day: TUESDAY,
        h: minute / 60,
        m: minute % 60,
    }
}

/// Her empty home, if it stands.
fn empty_of(guest: &Guest) -> Option<&Empty> {
    match &guest.state {
        State::Away(empty) => Some(empty),
        _ => None,
    }
}

/// Whether she's on a dash home.
fn dashing(guest: &Guest) -> bool {
    matches!(guest.state, State::Arriving(How::Dash, _))
        || matches!(&guest.state, State::Visiting(visit) if visit.kind == Kind::Dash)
}

/// One step as the shell takes it from `now`, sleeping exactly as long
/// as she asks (a minute when she asks nothing): a tick, and a paint if
/// it says the screen could change. Whatever she must wake for (her
/// dash home among them) is hers to ask for: no shorter wait covers for
/// a wake she forgot.
fn long_step(guest: &mut Guest, real: &Buffer, view: &IdleView, now: &mut u64) {
    *now += guest
        .next_tick(*now)
        .map_or(60_000, |d| d.as_millis() as u64)
        .max(1);
    if guest.advance(*now) {
        paint(guest, real, view, *now);
    }
}

/// Whether a goodbye shows her (a dissolve of her, waving or not; not
/// only of her things, as when they rain out with her out of sight).
fn waving(guest: &Guest) -> bool {
    let State::Leaving(leaving) = &guest.state else {
        return false;
    };
    leaving.dissolve.has_face()
        || leaving
            .image
            .as_ref()
            .is_some_and(|image| image.figure.her().is_some())
}

/// What a dash home looked like, frame by frame.
#[derive(Debug, Default)]
struct Seen {
    /// When she dashed in, and where her closed door stood then.
    came: Option<(u64, Option<door::DoorSpot>)>,
    /// Where she first stood, in.
    first: Option<(i32, i32)>,
    /// Every line she said.
    said: Vec<&'static str>,
    /// Her fridge stood open.
    fridge_open: bool,
    /// What she decided, in order.
    methods: Vec<&'static str>,
    /// Her home stood empty again after it, and when.
    back: Option<u64>,
}

/// Run `guest` (out at school, her home standing empty or not) from
/// `now` until she has dashed home and is out again, at most `limit`
/// ms. A dash never counts as a visit, never ends in a goodbye of her
/// (what it showed may rain out, with no home to stand), and brings no
/// parcel while it lasts.
fn watch_dash(guest: &mut Guest, real: &Buffer, view: &IdleView, mut now: u64, limit: u64) -> Seen {
    let visits = guest.ledger.visits;
    let ordered = guest.ledger.ordered;
    let mut seen = Seen::default();
    let end = now + limit;
    while now < end && seen.back.is_none() {
        let door = guest.closed_door();
        shell_step(guest, real, view, &mut now, true);
        assert!(!waving(guest), "a goodbye at {now}");
        assert_eq!(guest.ledger.visits, visits, "a dash counted at {now}");
        if dashing(guest) && seen.came.is_none() {
            seen.came = Some((now, door));
        }
        if let State::Visiting(visit) = &guest.state {
            assert_eq!(visit.kind, Kind::Dash, "a visit at {now}");
            assert_eq!(guest.ledger.ordered, ordered, "a parcel at {now}");
            let osaka = &visit.osaka;
            if seen.first.is_none() && !osaka.hidden(now) {
                seen.first = Some((osaka.x, osaka.y));
            }
            if let Some(Bubble::Say(line)) = osaka.appearance(now).2
                && seen.said.last() != Some(&line)
            {
                seen.said.push(line);
            }
            seen.fridge_open |= visit.looks.state(Furniture::Fridge) == PieceState::FridgeOpen;
            seen.methods = osaka.decisions.iter().map(|d| d.method).collect();
        } else if seen.came.is_some() && !dashing(guest) {
            seen.back = Some(now);
        }
    }
    seen
}

/// On a school day with a dash, she dashes home as its minute passes:
/// out of her closed door, to her fridge, which opens ("Forgot my
/// lunch!"), and out again through her door, her home standing empty
/// after. Not a visit (her record's count stands), no parcel (the one
/// on order waits for her return), no hello, no goodbye.
#[test]
fn a_dash_home_for_her_lunch_and_out_again() {
    let (seed, minute) = dash_seed(0);
    for (name, (real, view)) in home_screens() {
        for graphics in [false, true] {
            let at = format!("{name} graphics={graphics}");
            let mut guest = home_at(seed, tue_at(minute - 3), &FRIDGE_HOME, graphics);
            // A parcel on order, due at her next visit.
            guest.ledger.ordered = Some(Furniture::Desk);
            guest.ledger.bought_on = 0;
            let mut now = 0;
            paint(&mut guest, &real, &view, now);
            while empty_of(&guest).is_none() {
                assert!(now < 60_000, "{at}: her home never stood empty");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let fridge = empty_of(&guest)
                .is_some_and(|e| e.shown.iter().any(|s| s.item == Furniture::Fridge));
            assert!(fridge, "{at}: her fridge is shown");
            let due = real_of(&guest, now, tue_at(minute));
            let seen = watch_dash(&mut guest, &real, &view, now, 120_000);
            let (came, door) = seen
                .came
                .unwrap_or_else(|| panic!("{at}: no dash: {seen:?}"));
            // At its minute, as the step crossing it ends (a second's
            // step, here).
            assert!(
                (due..=due + 1000).contains(&came),
                "{at}: dashed home at {came}, due {due}"
            );
            let door = door.unwrap_or_else(|| panic!("{at}: no door"));
            assert_eq!(seen.first, Some(door.spot()), "{at}: out of her door");
            assert!(seen.fridge_open, "{at}: her fridge never opened");
            assert!(seen.said.contains(&FORGOT_LUNCH), "{at}: {:?}", seen.said);
            assert!(
                seen.said
                    .iter()
                    .all(|line| !mind::DOOR.lines.contains(line)),
                "{at}: a door's line: {:?}",
                seen.said
            );
            assert!(
                seen.methods.contains(&"dash/lunch"),
                "{at}: {:?}",
                seen.methods
            );
            assert_eq!(
                seen.methods.last(),
                Some(&"routine/away"),
                "{at}: {:?}",
                seen.methods
            );
            seen.back
                .unwrap_or_else(|| panic!("{at}: never out again: {seen:?}"));
            assert!(empty_of(&guest).is_some(), "{at}: her home stands empty");
            assert!(guest.closed_door().is_some(), "{at}: her door stands");
            assert_eq!(guest.ledger.ordered, Some(Furniture::Desk), "{at}");
        }
    }
}

/// Something done to her screen on a dash home for her lunch.
#[derive(Clone, Copy, Debug)]
enum Poke {
    /// A chat line, `into` ms after her first lunch at her fridge began,
    /// drawn `lag` ms after the tick before it (the shell reads its clock
    /// for each).
    Chat { into: u64, lag: u64 },
    /// A chat line as her first lunch ends: her ticks up to a moment
    /// before its end, then the line drawn `late` ms after it, before her
    /// tick for it.
    ChatAsItEnds { late: u64 },
    /// Text over her fridge (into the closet it goes), `into` ms after
    /// she sets off for it (`walking`) or after her first lunch began.
    Closet { walking: bool, into: u64 },
}

/// A dash home for her lunch, as poked (see [`lunch_run`]).
#[derive(Debug, Default)]
struct LunchRun {
    /// Every line she said, each time it showed.
    said: Vec<&'static str>,
    /// Each lunch at her fridge: when it began, and when it would end.
    lunches: Vec<(u64, u64)>,
    /// Each chat line that landed on a lunch before its end: whether she
    /// looked at it, and how many lines she had said by then.
    cuts: Vec<(bool, usize)>,
    /// Chat lines delivered.
    chats: usize,
    /// When she first set off for her fridge.
    set_off: Option<u64>,
    /// When her fridge went into the closet.
    closeted: Option<u64>,
    /// When she was out again, her home standing empty.
    out: Option<u64>,
}

/// Her lunch at her fridge, if she's at it: when it began, and when it
/// would end.
fn at_lunch(guest: &Guest) -> Option<(u64, u64)> {
    let State::Visiting(visit) = &guest.state else {
        return None;
    };
    let osaka = &visit.osaka;
    osaka
        .plays()
        .filter(|p| p.own == ScriptId::DashLunch)
        .and(osaka.use_span())
        .map(|(_, since, until)| (since, until))
}

/// Her dash home for her lunch on a Tuesday (`seed`'s, at `minute`), on
/// `screen` (with her fridge among her things), stepped as the shell
/// steps, with `pokes` done to it as they come due, until she's out
/// again (at most `limit` ms).
fn lunch_run(
    (seed, minute): (u64, u16),
    (real, view): &(Buffer, IdleView),
    graphics: bool,
    pokes: &[Poke],
    limit: u64,
) -> LunchRun {
    let mut guest = home_at(seed, tue_at(minute - 3), &FRIDGE_HOME, graphics);
    let (mut real, mut view) = (real.clone(), view.clone());
    let mut pokes = pokes.to_vec();
    let mut run = LunchRun::default();
    let (mut came, mut bubble) = (false, None);
    let mut now = 0;
    paint(&mut guest, &real, &view, now);
    while now < limit && run.out.is_none() {
        let mut to = now
            + guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
        let lunch = at_lunch(&guest);
        let first = run.lunches.first().map(|l| l.0);
        let poke = pokes.iter().position(|poke| match *poke {
            Poke::Chat { into, .. } => first.is_some_and(|b| b + into <= to),
            Poke::ChatAsItEnds { .. } => {
                lunch.is_some_and(|(since, until)| Some(since) == first && until <= to)
            }
            Poke::Closet { walking, into } => {
                let from = if walking { run.set_off } else { first };
                from.is_some_and(|b| b + into <= to)
            }
        });
        let poke = poke.map(|i| pokes.remove(i));
        let mut draw = to;
        match poke {
            Some(Poke::Chat { lag, .. }) => draw = to + lag,
            Some(Poke::ChatAsItEnds { late }) => {
                let until = lunch.map_or(to, |l| l.1);
                to = (until - 1).max(now);
                draw = until + late;
            }
            Some(Poke::Closet { .. }) => {
                if let State::Visiting(visit) = &guest.state
                    && let Some(fridge) = visit.shown.iter().find(|s| s.item == Furniture::Fridge)
                {
                    let cover = fridge.cover();
                    for y in cover.y..cover.y + cover.height {
                        let text = "x".repeat(usize::from(cover.width));
                        real.set_string(cover.x, y, text, Style::new());
                    }
                    run.closeted = Some(to);
                }
            }
            None => {}
        }
        let chat = matches!(poke, Some(Poke::Chat { .. } | Poke::ChatAsItEnds { .. }));
        guest.advance(to);
        let lunch = at_lunch(&guest);
        now = draw;
        if chat {
            view.chat_mark.synced += 1;
            run.chats += 1;
        }
        paint(&mut guest, &real, &view, now);
        if let State::Visiting(visit) = &guest.state {
            came = true;
            let osaka = &visit.osaka;
            if chat
                && let Some((_, until)) = lunch
                && now < until
            {
                run.cuts.push((osaka.act_name() == "Look", run.said.len()));
            }
            if let Some(lunch) = at_lunch(&guest)
                && run.lunches.last() != Some(&lunch)
            {
                run.lunches.push(lunch);
            }
            if run.set_off.is_none()
                && osaka.act_name() == "Walk"
                && osaka.decisions.iter().any(|d| d.method == "dash/lunch")
            {
                run.set_off = Some(now);
            }
            let line = match osaka.appearance(now).2 {
                Some(Bubble::Say(line)) => Some(line),
                _ => None,
            };
            if let Some(line) = line
                && bubble != Some(line)
            {
                run.said.push(line);
            }
            bubble = line;
        } else if came && !dashing(&guest) {
            run.out = Some(now);
        }
    }
    run
}

/// A chat line as she gets to her fridge doesn't cost her the lunch she
/// dashed home for: arriving as its look in the fridge begins, or a
/// moment into it (before "Forgot my lunch!"), it has her look at the
/// chat, and then she has her lunch after all, line and all, before
/// she's out again by her door. As one that stops her on her way does.
/// Quiet panes or text-dense, in both drawing modes.
#[test]
fn a_chat_line_at_her_fridge_doesnt_cost_her_her_lunch() {
    let dash = dash_seed(0);
    for (name, screen) in home_screens() {
        for graphics in [false, true] {
            for into in [0u64, 700, 1400] {
                let at = format!("{name} graphics={graphics} {into} ms in");
                let pokes = [Poke::Chat { into, lag: 0 }];
                let run = lunch_run(dash, &screen, graphics, &pokes, 300_000);
                assert!(run.out.is_some(), "{at}: never out again: {run:?}");
                let [(looked, said)] = run.cuts[..] else {
                    panic!("{at}: the line never landed on her lunch: {run:?}");
                };
                assert!(looked, "{at}: she didn't look at the chat: {run:?}");
                assert!(
                    run.said[said..].contains(&FORGOT_LUNCH),
                    "{at}: out without her lunch: {run:?}"
                );
            }
        }
    }
}

/// A chat line drawn as her lunch ends (at its end, or a moment after,
/// before her tick for it: the shell reads its clock for her tick and
/// for its frame apart) finds her lunch had: she looks at the chat, and
/// goes out again with no second lunch. Quiet panes or text-dense, in
/// both drawing modes.
#[test]
fn a_chat_line_as_her_lunch_ends_doesnt_have_her_have_it_again() {
    let dash = dash_seed(0);
    for (name, screen) in home_screens() {
        for graphics in [false, true] {
            for late in [0u64, 1, 300] {
                let at = format!("{name} graphics={graphics} {late} ms late");
                let pokes = [Poke::ChatAsItEnds { late }];
                let run = lunch_run(dash, &screen, graphics, &pokes, 300_000);
                assert_eq!(run.chats, 1, "{at}: no line: {run:?}");
                assert!(run.out.is_some(), "{at}: never out again: {run:?}");
                assert_eq!(run.lunches.len(), 1, "{at}: had again: {run:?}");
                let lines = run.said.iter().filter(|&&l| l == FORGOT_LUNCH).count();
                assert_eq!(lines, 1, "{at}: {run:?}");
            }
        }
    }
}

/// Her fridge going into the closet (text over it) as she has her
/// lunch, or on her way to it, ends her dash: she gives up on her lunch
/// and goes out again by her door, never back and forth to where it
/// stood. Quiet panes or text-dense, in both drawing modes.
#[test]
fn a_fridge_gone_from_her_lunch_doesnt_keep_her_home() {
    let dash = dash_seed(0);
    for (name, screen) in home_screens() {
        for graphics in [false, true] {
            for (walking, into) in [(true, 0u64), (true, 300), (false, 0), (false, 700)] {
                let at = format!("{name} graphics={graphics} walking={walking} {into} ms in");
                let pokes = [Poke::Closet { walking, into }];
                let run = lunch_run(dash, &screen, graphics, &pokes, 300_000);
                let closeted = run
                    .closeted
                    .unwrap_or_else(|| panic!("{at}: never closeted: {run:?}"));
                let out = run
                    .out
                    .unwrap_or_else(|| panic!("{at}: never out again: {run:?}"));
                // Her door is at its space by the screen's edge (the door
                // batch), a long walk from her fridge: on her way, she
                // gives up only where it stood (a walk isn't cut short
                // when its piece goes; door-notes, step 3).
                assert!(
                    out < closeted + 60_000,
                    "{at}: out at {out}, closeted at {closeted}: {run:?}"
                );
                let after = run.lunches.iter().filter(|l| l.0 > closeted).count();
                assert!(after <= 1, "{at}: back to where it stood: {run:?}");
            }
        }
    }
}

/// With no fridge, she dashes home and can't think what for: "Forgot
/// somethin'..." then "...what was it?", and out again by her door.
#[test]
fn dashed_home_with_no_fridge_she_cant_think_what_for() {
    let (real, view) = home_screen();
    let (seed, minute) = dash_seed(100);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(seed, tue_at(minute - 2), &BARE_HOME, graphics);
        let now = until_visiting_or_away(&mut guest, &real, &view);
        let seen = watch_dash(&mut guest, &real, &view, now, 120_000);
        assert!(seen.came.is_some(), "{at}: no dash");
        let forgot = seen
            .said
            .iter()
            .position(|&line| line == FORGOT_SOMETHING)
            .unwrap_or_else(|| panic!("{at}: {:?}", seen.said));
        assert_eq!(
            seen.said.get(forgot + 1),
            Some(&WHAT_WAS_IT),
            "{at}: {:?}",
            seen.said
        );
        assert!(!seen.said.contains(&FORGOT_LUNCH), "{at}");
        assert_eq!(seen.methods.first(), Some(&"dash/forgot"), "{at}");
        assert_eq!(seen.methods.last(), Some(&"routine/away"), "{at}");
        assert!(seen.back.is_some(), "{at}: never out again");
        assert!(empty_of(&guest).is_some(), "{at}: her home stands empty");
    }
}

/// Dashed home with her wall clock hanging (in either room: by her
/// fridge, where she has her lunch and goes out again from, or in the
/// other), she goes out again with no glance at it: "Time for school!"
/// is for her morning's leaving, not for heading back after what she
/// forgot.
#[test]
fn dashed_home_she_doesnt_glance_at_her_clock() {
    let (real, view) = home_screen();
    let (seed, minute) = dash_seed(0);
    for graphics in [false, true] {
        for nook in [Nook::Playlist, Nook::Users] {
            let at = format!("graphics={graphics} clock in {nook:?}");
            let mut guest = home_at(seed, tue_at(minute - 3), &FRIDGE_HOME, graphics);
            assert!(guest.ledger.home.add(room::Prop::new(
                Furniture::Clock,
                nook,
                500,
                sprite::Facing::Right
            )));
            let mut now = 0;
            paint(&mut guest, &real, &view, now);
            while empty_of(&guest).is_none() {
                assert!(now < 60_000, "{at}: her home never stood empty");
                shell_step(&mut guest, &real, &view, &mut now, true);
            }
            let seen = watch_dash(&mut guest, &real, &view, now, 120_000);
            assert!(seen.came.is_some(), "{at}: no dash");
            assert!(seen.methods.contains(&"dash/lunch"), "{at}");
            assert_eq!(seen.methods.last(), Some(&"routine/away"), "{at}");
            assert!(
                !seen.methods.contains(&"routine/glance"),
                "{at}: {:?}",
                seen.methods
            );
            assert!(
                !seen
                    .said
                    .contains(&crate::ui::houseguest::script::SCHOOL_TIME),
                "{at}: {:?}",
                seen.said
            );
        }
    }
}

/// From a cold start until her home stands empty: when.
fn until_visiting_or_away(guest: &mut Guest, real: &Buffer, view: &IdleView) -> u64 {
    let mut now = 0;
    paint(guest, real, view, now);
    while empty_of(guest).is_none() {
        assert!(now < 60_000, "her home never stood empty");
        shell_step(guest, real, view, &mut now, true);
    }
    now
}

/// A dash is an edge of her clock: one step of it must cross its minute.
/// Started at or after it (a restart), she never dashes that day; nor on
/// a school day without one, nor on a day of the school holidays; and
/// she wakes for none of them (the shell's waits are as long as her
/// routine allows).
#[test]
fn a_restart_after_her_dash_never_dashes() {
    let (real, view) = home_screen();
    let (seed, minute) = dash_seed(0);
    let none = (0..10_000)
        .find(|&seed| brain::dash(seed, TUESDAY).is_none())
        .expect("a Tuesday without one");
    let june = date(2026, 6, 17);
    let summer = date(2026, 7, 28);
    for (what, seed, from, date) in [
        ("at its minute", seed, minute, june),
        ("a minute after", seed, minute + 1, june),
        ("no dash that day", none, 8 * 60 + 20, june),
        ("the holidays", seed, 8 * 60 + 20, summer),
    ] {
        let mut guest = home_at(seed, tue_at(from), &FRIDGE_HOME, false);
        guest.set_date(date);
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        let end = real_of(&guest, now, tue_at(12 * 60 + 40));
        while now < end {
            long_step(&mut guest, &real, &view, &mut now);
            assert!(!dashing(&guest), "{what}: dashed at {now}");
        }
    }
    // Started before it, with the client running across it, she does.
    let mut guest = home_at(seed, tue_at(minute - 1), &FRIDGE_HOME, false);
    let mut now = 0;
    paint(&mut guest, &real, &view, now);
    let end = real_of(&guest, now, tue_at(minute + 1));
    let mut dashed = false;
    while now < end && !dashed {
        long_step(&mut guest, &real, &view, &mut now);
        dashed = dashing(&guest);
    }
    assert!(dashed, "no dash across its minute");
}

/// At most one dash a school day, at its minute: over a whole school
/// morning for many homes, each that has one that day dashes home once,
/// and no other.
#[test]
fn at_most_one_dash_a_school_day() {
    let (real, view) = home_screen();
    for seed in 0..12 {
        let mut guest = home_at(seed, tue_at(8 * 60), &BARE_HOME, false);
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        let end = real_of(&guest, now, tue_at(12 * 60 + 40));
        let mut dashes = 0;
        let mut was = false;
        while now < end {
            long_step(&mut guest, &real, &view, &mut now);
            let is = dashing(&guest);
            dashes += usize::from(is && !was);
            was = is;
        }
        let want = usize::from(brain::dash(seed, TUESDAY).is_some());
        assert_eq!(dashes, want, "seed {seed}");
    }
}

/// The errand at school (Q2): scrolled back with messages unseen while
/// she's out, she dashes in by a door, pokes the accordion, and goes
/// straight back out by her door: no goodbye of her, even for a visitor
/// whose video plays (she's out on her routine's way, not his); not
/// counted; then her home stands empty again (a resident's), or she's
/// simply out (a visitor at the video: what the dash showed rains out,
/// nobody waving, and her home shows when he's idle). With no home she
/// still comes and goes by her door.
#[test]
fn the_errand_at_school_is_a_dash() {
    let (w, h) = (100, 30);
    for (resident, home) in [(false, true), (true, true), (false, false)] {
        for graphics in [false, true] {
            let at = format!("resident={resident} home={home} graphics={graphics}");
            let (real, accordion) = accordion_room(w, h, 2);
            let chat = nooks(w, h)[0].1;
            let base = IdleView {
                busy: Some(Busy::Playing),
                resident,
                focus: resident.then_some(chat),
                chat,
                nooks: nooks(w, h)[1..].to_vec(),
                ..view(bottom_strip(w, h))
            };
            let view = scrolled_back(base, accordion, 2);
            let pieces: &[(Furniture, Nook, u16)] = if home { &BARE_HOME } else { &[] };
            let (seed, _) = (0..10_000)
                .map(|seed| (seed, brain::dash(seed, TUESDAY)))
                .find(|(_, dash)| dash.is_none())
                .expect("a Tuesday without one");
            let mut guest = home_at(seed, tue_at(9 * 60), pieces, graphics);
            let visits = guest.ledger.visits;
            let mut now = 0;
            paint(&mut guest, &real, &view, now);
            let (mut came, mut poked, mut out) = (None, None, None);
            while now < 150_000 && out.is_none() {
                now += guest
                    .next_tick(now)
                    .map_or(100, |d| d.as_millis() as u64)
                    .clamp(1, 100);
                guest.advance(now);
                paint(&mut guest, &real, &view, now);
                assert!(!waving(&guest), "{at}: a goodbye at {now}");
                assert_eq!(guest.ledger.visits, visits, "{at}: counted at {now}");
                match &guest.state {
                    State::Visiting(visit) => {
                        assert_eq!(visit.kind, Kind::Dash, "{at}");
                        let osaka = &visit.osaka;
                        if came.is_none() {
                            came = Some(now);
                            assert_eq!(osaka.act_name(), "Door", "{at}: not by a door");
                        }
                        if guest.errand.as_ref().is_some_and(|e| e.poked) {
                            poked.get_or_insert(now);
                        }
                    }
                    State::Absent | State::Away(_) if came.is_some() => out = Some(now),
                    _ => {}
                }
            }
            assert!(came.is_some(), "{at}: never came");
            let poked = poked.unwrap_or_else(|| panic!("{at}: never poked"));
            let out = out.unwrap_or_else(|| panic!("{at}: never out again"));
            assert!(out > poked, "{at}");
            assert!(guest.out.is_some(), "{at}: out by her door");
            assert_eq!(
                matches!(guest.state, State::Away(_)),
                resident && home,
                "{at}: her home empty, or simply out"
            );
        }
    }
}

/// Dashed in for the accordion as school ends (12:44, say), she's still
/// here when it does: home early, and counted then (and a parcel may
/// come), rather than a dash that never ends.
#[test]
fn still_in_from_a_dash_as_school_ends_she_is_home() {
    let (real, view) = home_screen();
    let (seed, _) = (0..10_000)
        .map(|seed| (seed, brain::dash(seed, TUESDAY)))
        .find(|(_, dash)| dash.is_none())
        .expect("a Tuesday without one");
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(seed, tue_at(12 * 60 + 40), &FRIDGE_HOME, graphics);
        let mut now = until_visiting_or_away(&mut guest, &real, &view);
        let visits = guest.ledger.visits;
        guest.cue(Scene::DashForgot);
        now += 1;
        paint(&mut guest, &real, &view, now);
        assert!(dashing(&guest), "{at}");
        assert_eq!(guest.ledger.visits, visits, "{at}");
        // School ends before she's off again.
        guest.skip_clock(now);
        assert_eq!(
            guest.clock_label(now).as_deref(),
            Some("Tue 12:45 Afternoon")
        );
        shell_step(&mut guest, &real, &view, &mut now, true);
        let State::Visiting(visit) = &guest.state else {
            panic!("{at}: visiting");
        };
        assert_eq!(visit.kind, Kind::Normal, "{at}: home early");
        assert_eq!(guest.ledger.visits, visits + 1, "{at}: counted");
        assert!(guest.out.is_none(), "{at}: not out");
    }
}

/// A master seed from `from` on whose Tuesday has no dash home.
fn no_dash_seed(from: u64) -> u64 {
    (from..from + 10_000)
        .find(|&seed| brain::dash(seed, TUESDAY).is_none())
        .expect("a Tuesday without one")
}

/// From a cold start until her home stands empty, then a dash cued
/// (`scene`): she's in through her door, dashing.
fn cue_a_dash(guest: &mut Guest, real: &Buffer, view: &IdleView, scene: Scene) -> u64 {
    let mut now = until_visiting_or_away(guest, real, view);
    guest.cue(scene);
    now += 1;
    paint(guest, real, view, now);
    assert!(dashing(guest), "{scene:?}: dashing");
    now
}

/// Dashed home near school's end and already through her door on her
/// way out again when it ends: she goes on out (the dash isn't made a
/// visit then: it's over), and comes home as school ends, out of her
/// door, counted once.
#[test]
fn on_her_way_out_from_a_dash_as_school_ends_she_comes_home_once() {
    let (real, view) = home_screen();
    let seed = no_dash_seed(0);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(seed, tue_at(12 * 60 + 40), &FRIDGE_HOME, graphics);
        let mut now = cue_a_dash(&mut guest, &real, &view, Scene::DashForgot);
        let visits = guest.ledger.visits;
        // Through her door, on her way out, before school ends.
        let through = |guest: &Guest, now: u64| {
            matches!(&guest.state, State::Visiting(visit)
                if visit.osaka.leaving().is_some() && visit.osaka.hidden(now))
        };
        while !through(&guest, now) {
            assert!(now < 60_000, "{at}: never on her way out");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        guest.skip_clock(now);
        assert_eq!(
            guest.clock_label(now).as_deref(),
            Some("Tue 12:45 Afternoon")
        );
        let (mut out, mut home) = (false, false);
        let end = now + 60_000;
        while now < end && !home {
            shell_step(&mut guest, &real, &view, &mut now, true);
            assert!(guest.ledger.visits <= visits + 1, "{at}: counted twice");
            match &guest.state {
                State::Visiting(visit) if visit.kind == Kind::Normal => {
                    assert!(out, "{at}: made a visit before she was out");
                    home = true;
                }
                State::Visiting(_) => {}
                _ => out = true,
            }
        }
        assert!(home, "{at}: never home");
        assert_eq!(guest.ledger.visits, visits + 1, "{at}: counted once");
    }
}

/// A first meeting is always counted (her clock starts with it): a dash
/// home cued on a guest never met is a visit, and she says hello after
/// her lunch.
#[test]
fn a_dash_cued_at_a_first_meeting_is_a_visit() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = Guest::new(5);
        if graphics {
            guest.set_picker(kitty());
        }
        assert_eq!(guest.ledger.visits, 0);
        guest.cue(Scene::DashIn);
        paint(&mut guest, &real, &view, 0);
        let visit = visit_of(&guest);
        assert_eq!(visit.kind, Kind::Normal, "{at}");
        assert!(!visit.osaka.greeted(), "{at}: her hello is to come");
        assert_eq!(guest.ledger.visits, 1, "{at}: counted");
        let mut now = 0;
        let mut said = Vec::new();
        while now < 60_000 && !visit_of(&guest).osaka.greeted() {
            shell_step(&mut guest, &real, &view, &mut now, true);
            if let Some(Bubble::Say(line)) = visit_of(&guest).osaka.appearance(now).2
                && said.last() != Some(&line)
            {
                said.push(line);
            }
        }
        let greeting = visit_of(&guest).osaka.mood().greeting();
        assert!(said.contains(&FORGOT_LUNCH), "{at}: {said:?}");
        while now < 60_000 && said.last() != Some(&greeting) {
            shell_step(&mut guest, &real, &view, &mut now, true);
            if let Some(Bubble::Say(line)) = visit_of(&guest).osaka.appearance(now).2
                && said.last() != Some(&line)
            {
                said.push(line);
            }
        }
        assert_eq!(said.last(), Some(&greeting), "{at}: no hello: {said:?}");
    }
}

/// The forgetful dash cued with her fridge to hand: she still can't
/// think what she came for ("Forgot somethin'..." then "...what was
/// it?"), and her fridge stays shut.
#[test]
fn the_forgetful_dash_forgets_with_a_fridge_to_hand() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(no_dash_seed(0), tue_at(9 * 60), &FRIDGE_HOME, graphics);
        let now = cue_a_dash(&mut guest, &real, &view, Scene::DashForgot);
        let seen = watch_dash(&mut guest, &real, &view, now, 120_000);
        assert!(!seen.fridge_open, "{at}: her fridge opened");
        assert!(!seen.said.contains(&FORGOT_LUNCH), "{at}: {:?}", seen.said);
        let forgot = seen
            .said
            .iter()
            .position(|&line| line == FORGOT_SOMETHING)
            .unwrap_or_else(|| panic!("{at}: {:?}", seen.said));
        assert_eq!(seen.said.get(forgot + 1), Some(&WHAT_WAS_IT), "{at}");
        assert_eq!(seen.methods.first(), Some(&"dash/forgot"), "{at}");
        assert!(seen.back.is_some(), "{at}: never out again");
    }
}

/// Whatever stops her on her way to her fridge for her lunch (here a
/// chat line: she stops and looks), she sets off for it again, and has
/// her lunch.
#[test]
fn stopped_on_her_way_to_her_lunch_she_sets_off_again() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(no_dash_seed(0), tue_at(9 * 60), &FRIDGE_HOME, graphics);
        let mut now = until_visiting_or_away(&mut guest, &real, &view);
        // Dashing home as her clock would have her (not cued: the stage
        // puts her door beside her fridge), out of her closed door, a
        // walk from her fridge.
        let home = guest.take_home();
        guest.state = State::Arriving(How::Dash, home);
        let walking = |guest: &Guest| {
            matches!(&guest.state, State::Visiting(visit)
                if visit.osaka.act_name() == "Walk"
                    && visit.osaka.decisions.iter().any(|d| d.method == "dash/lunch"))
        };
        while !walking(&guest) {
            assert!(now < 60_000, "{at}: never on her way to her fridge");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        // A chat line: she stops to look.
        let chatty = IdleView {
            chat_mark: ChatMark {
                synced: 1,
                ..ChatMark::default()
            },
            ..view.clone()
        };
        now += 1;
        guest.advance(now);
        paint(&mut guest, &real, &chatty, now);
        assert!(!walking(&guest), "{at}: she didn't stop");
        let seen = watch_dash(&mut guest, &real, &chatty, now, 120_000);
        assert!(seen.fridge_open, "{at}: no lunch: {:?}", seen.methods);
        assert!(seen.said.contains(&FORGOT_LUNCH), "{at}: {:?}", seen.said);
        assert!(seen.back.is_some(), "{at}: never out again");
    }
}

/// With no home (nothing to show while she's out: she's simply absent),
/// she still dashes home as its minute passes, the shell asleep until
/// she asks to wake for it: in by her door (with no home, at the space
/// of the wall the chooser picks, unsaved: on [`home_screen`] Users'
/// left), can't think what for, and out by her door again; what showed
/// (her door) rains out, nobody waving, and she's absent, out at school;
/// at 12:45 she comes home by the same wall. Not counted. Her client not
/// idle at the minute, the day's dash is simply missed.
#[test]
fn with_no_home_she_comes_in_by_the_same_wall_each_time() {
    let (real, view) = home_screen();
    let (seed, minute) = dash_seed(0);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(seed, tue_at(minute - 3), &[], graphics);
        let visits = guest.ledger.visits;
        // The shell's first tick latches her clock; then it sleeps as
        // long as she asks.
        let mut now = 0;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        let due = real_of(&guest, now, tue_at(minute));
        while !dashing(&guest) {
            assert!(
                matches!(guest.state, State::Absent),
                "{at}: absent till she dashes"
            );
            assert!(now <= due, "{at}: no dash by {now} (due {due})");
            long_step(&mut guest, &real, &view, &mut now);
        }
        assert!((due..=due + 10).contains(&now), "{at}: at {now}, due {due}");
        let seen = watch_dash(&mut guest, &real, &view, now - 1, 120_000);
        assert!(seen.first.is_some(), "{at}: never in: {seen:?}");
        let forgot = seen
            .said
            .iter()
            .position(|&line| line == FORGOT_SOMETHING)
            .unwrap_or_else(|| panic!("{at}: {:?}", seen.said));
        assert_eq!(seen.said.get(forgot + 1), Some(&WHAT_WAS_IT), "{at}");
        assert_eq!(seen.methods.last(), Some(&"routine/away"), "{at}");
        let back = seen.back.unwrap_or_else(|| panic!("{at}: never out again"));
        now = back;
        while !matches!(guest.state, State::Absent) {
            assert!(now < back + 10_000, "{at}: never absent");
            assert!(!waving(&guest), "{at}: a goodbye at {now}");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        let wall = walls_of(&view.nooks, real.area, &[])
            .into_iter()
            .find(|w| w.nook == Nook::Users && w.side == room::Side::Left)
            .expect("Users' left wall");
        assert!(wall.edge && wall.space.is_some(), "{at}: {wall:?}");
        assert_eq!(seen.first, Some(wall.spot), "{at}: in by her door's space");
        assert_eq!(guest.ledger.visits, visits, "{at}: counted");
        assert_eq!(guest.ledger.home.door, None, "{at}: no wall saved, no home");
        // Home from school: by the same wall.
        let home_time = real_of(&guest, now, tue(12, 45));
        while now < home_time {
            long_step(&mut guest, &real, &view, &mut now);
        }
        let mut first = None;
        while first.is_none() {
            assert!(now < home_time + 20_000, "{at}: never home");
            shell_step(&mut guest, &real, &view, &mut now, true);
            if let State::Visiting(visit) = &guest.state {
                first = Some((visit.osaka.x, visit.osaka.y));
            }
        }
        assert_eq!(first, Some(wall.spot), "{at}: home by the same wall");
    }
    // At the keys a moment before the minute (the client open, the idle
    // gate shut): no dash that day.
    for resident in [false, true] {
        let view = IdleView {
            resident,
            ..view.clone()
        };
        let mut guest = home_at(seed, tue_at(minute - 3), &[], false);
        let mut now = 0;
        guest.advance(now);
        paint(&mut guest, &real, &view, now);
        let due = real_of(&guest, now, tue_at(minute));
        now = due - 1000;
        guest.advance(now);
        guest.activity(now);
        paint(&mut guest, &real, &view, now);
        let end = real_of(&guest, now, tue_at(12 * 60 + 40));
        let mut dashed = false;
        while now < end {
            long_step(&mut guest, &real, &view, &mut now);
            dashed |= dashing(&guest);
        }
        assert_eq!(dashed, resident, "resident={resident}: hers, or missed");
    }
}

/// The errand at school for a visitor at the keys (nothing playing, but
/// not idle long enough for her to come): in by a door, the poke, out by
/// her door; and her home isn't left standing for him after (it shows
/// only to an idle client): what the dash showed rains out, nobody
/// waving, and she's simply out.
#[test]
fn the_errand_at_school_for_a_visitor_at_the_keys() {
    let (w, h) = (100, 30);
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (real, accordion) = accordion_room(w, h, 2);
        let chat = nooks(w, h)[0].1;
        let base = IdleView {
            // Typed a moment ago: the idle gate stays shut throughout.
            delay: Some(Duration::from_secs(600)),
            chat,
            nooks: nooks(w, h)[1..].to_vec(),
            ..view(bottom_strip(w, h))
        };
        let view = scrolled_back(base, accordion, 2);
        let mut guest = home_at(no_dash_seed(0), tue_at(9 * 60), &BARE_HOME, graphics);
        let visits = guest.ledger.visits;
        let mut now = 0;
        paint(&mut guest, &real, &view, now);
        let (mut came, mut poked, mut rained) = (false, false, false);
        while now < 150_000 && !(came && matches!(guest.state, State::Absent)) {
            shell_step(&mut guest, &real, &view, &mut now, true);
            assert!(empty_of(&guest).is_none(), "{at}: her home shown at {now}");
            assert!(!waving(&guest), "{at}: a goodbye at {now}");
            assert_eq!(guest.ledger.visits, visits, "{at}: counted at {now}");
            match &guest.state {
                State::Visiting(visit) => {
                    assert_eq!(visit.kind, Kind::Dash, "{at}");
                    came = true;
                    poked |= guest.errand.as_ref().is_some_and(|e| e.poked);
                }
                State::Leaving(_) => rained = true,
                _ => {}
            }
        }
        assert!(came && poked, "{at}: came {came}, poked {poked}");
        assert!(matches!(guest.state, State::Absent), "{at}: out");
        assert!(rained, "{at}: her things just vanished");
        assert!(guest.out.is_some(), "{at}: out by her door");
    }
}

/// An overlay (a modal, F3) opened while she's dashed home ends the dash
/// at once, as it ends any visit: nothing of hers goes on over it. So
/// too her errand at school, before she has poked (the accordion shakes
/// by itself).
#[test]
fn an_overlay_ends_a_dash_home_at_once() {
    let (real, home) = home_screen();
    let home = IdleView {
        resident: true,
        ..home
    };
    let covered = IdleView {
        busy: Some(Busy::Overlay),
        ..home.clone()
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(no_dash_seed(0), tue_at(9 * 60), &FRIDGE_HOME, graphics);
        let mut now = cue_a_dash(&mut guest, &real, &home, Scene::DashIn);
        for _ in 0..3 {
            shell_step(&mut guest, &real, &home, &mut now, true);
        }
        assert!(dashing(&guest), "{at}");
        now += 1;
        guest.advance(now);
        paint(&mut guest, &real, &covered, now);
        assert!(!guest.present() || !dashing(&guest), "{at}: dashing on");
        assert!(
            !matches!(guest.state, State::Visiting(_)),
            "{at}: still visiting under the overlay"
        );
    }
    // The errand at school, under way when the overlay opens.
    let (w, h) = (100, 30);
    for graphics in [false, true] {
        let at = format!("errand graphics={graphics}");
        let (real, accordion) = accordion_room(w, h, 2);
        let chat = nooks(w, h)[0].1;
        let base = IdleView {
            resident: true,
            chat,
            nooks: nooks(w, h)[1..].to_vec(),
            ..view(bottom_strip(w, h))
        };
        let open = scrolled_back(base, accordion, 2);
        let covered = IdleView {
            busy: Some(Busy::Overlay),
            ..open.clone()
        };
        let mut guest = home_at(no_dash_seed(0), tue_at(9 * 60), &BARE_HOME, graphics);
        let mut now = 0;
        paint(&mut guest, &real, &open, now);
        while !(guest.errand.is_some() && matches!(guest.state, State::Visiting(_))) {
            assert!(now < 60_000, "{at}: no errand");
            shell_step(&mut guest, &real, &open, &mut now, true);
        }
        assert!(guest.errand.as_ref().is_some_and(|e| !e.poked), "{at}");
        now += 1;
        guest.advance(now);
        paint(&mut guest, &real, &covered, now);
        assert!(
            !matches!(guest.state, State::Visiting(_)),
            "{at}: still on her errand under the overlay"
        );
        paint(&mut guest, &real, &covered, now + 1);
        assert!(guest.errand.is_none(), "{at}: the errand's off");
    }
}

/// The cat stays as he is through her dash home: in his bed or not as
/// her empty home has him (the coming visit's), not the last visit's,
/// over homes where the two differ.
#[test]
fn the_cat_stays_put_through_a_dash_home() {
    let (real, view) = home_screen();
    let pieces = [
        (Furniture::Sofa, Nook::Users, 200),
        (Furniture::CatBed, Nook::Users, 700),
        (Furniture::Bed, Nook::Playlist, 200),
        (Furniture::Fridge, Nook::Playlist, 800),
    ];
    let mut differ = 0;
    for seed in 0..40 {
        let guest = home_at(seed, tue_at(9 * 60), &pieces, false);
        let ledger = &guest.ledger;
        if cat_home_of(ledger, ledger.visits) == cat_home_of(ledger, ledger.visits - 1) {
            continue;
        }
        differ += 1;
        for graphics in [false, true] {
            let at = format!("seed {seed} graphics={graphics}");
            let mut guest = home_at(seed, tue_at(9 * 60), &pieces, graphics);
            let mut now = until_visiting_or_away(&mut guest, &real, &view);
            let cat = |guest: &Guest| match &guest.state {
                State::Away(empty) => Some(empty.looks.state(Furniture::CatBed)),
                State::Visiting(visit) => Some(visit.looks.state(Furniture::CatBed)),
                _ => None,
            };
            let home = cat(&guest).unwrap_or_else(|| panic!("{at}: away"));
            guest.cue(Scene::DashForgot);
            let end = now + 60_000;
            let mut dashed = false;
            while now < end && !(dashed && empty_of(&guest).is_some()) {
                shell_step(&mut guest, &real, &view, &mut now, true);
                dashed |= dashing(&guest);
                if let Some(state) = cat(&guest) {
                    assert_eq!(state, home, "{at}: the cat changed at {now}");
                }
            }
            assert!(dashed && empty_of(&guest).is_some(), "{at}: dash done");
        }
    }
    assert!(differ >= 5, "{differ} homes");
}

/// The stage's dash cued on a visit under way (after school): as any
/// scene but an arrival, the visit goes on (not ended, not a new one),
/// and she's put through a door where one may stand, beside her fridge
/// for her lunch, or where she stood, forgetful.
#[test]
fn a_dash_cued_on_a_visit_keeps_the_visit() {
    let (real, view) = home_screen();
    for scene in [Scene::DashIn, Scene::DashForgot] {
        for graphics in [false, true] {
            let at = format!("{scene:?} graphics={graphics}");
            let mut guest = home_at(
                no_dash_seed(0),
                tue_at(16 * 60 + 30),
                &FRIDGE_HOME,
                graphics,
            );
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let visits = guest.ledger.visits;
            guest.cue(scene);
            now += 1;
            paint(&mut guest, &real, &view, now);
            assert!(
                matches!(guest.cue_note(), Some(Ok(_))),
                "{at}: {:?}",
                guest.cue_note()
            );
            let visit = visit_of(&guest);
            assert_eq!(visit.kind, Kind::Normal, "{at}: the same visit");
            assert_eq!(guest.ledger.visits, visits, "{at}: not a new one");
            assert!(visit.osaka.dashing(), "{at}: dashing in");
            let spot = (visit.osaka.x, visit.osaka.y);
            assert_eq!(
                visit.door.map(door::DoorSpot::spot),
                Some(spot),
                "{at}: through her door"
            );
            assert!(
                visit.door.is_some_and(|d| d.wall().is_some()),
                "{at}: in its space"
            );
            let line = if scene == Scene::DashIn {
                FORGOT_LUNCH
            } else {
                WHAT_WAS_IT
            };
            // Her door is at its space, by the screen's edge: she walks
            // the room to her fridge.
            let mut said = Vec::new();
            let cued = now;
            while now < cued + 60_000 && !said.contains(&line) {
                shell_step(&mut guest, &real, &view, &mut now, true);
                if let Some(Bubble::Say(line)) = visit_of(&guest).osaka.appearance(now).2 {
                    said.push(line);
                }
            }
            assert!(said.contains(&line), "{at}: {said:?}");
        }
    }
}

/// Her clock a game minute before `seed`'s first dash home on a school
/// day from game day `from` on.
fn before_a_dash(seed: u64, from: u64) -> routine::GameTime {
    let (day, minute) = (from..from + 1000)
        .find_map(|day| {
            let school = routine::weekday(day).num_days_from_monday() < 5;
            brain::dash(seed, day)
                .filter(|_| school)
                .map(|minute| (day, minute))
        })
        .expect("a school day with a dash home");
    routine::GameTime {
        day,
        h: (minute - 1) / 60,
        m: (minute - 1) % 60,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(16)))]

    /// Dashed home for her lunch, she has it whatever chat lines come
    /// (wherever in it or after, drawn a moment after her tick as the
    /// shell's may be), and is out again by her door: a line before its
    /// end cuts it short (she looks at the chat) and she has it again
    /// after, line and all; one as it ends finds it had; no lunch is had
    /// again without a line cutting the one before. Her fridge going
    /// into the closet (as she sets off for it, or has it) ends her dash,
    /// and she's out again all the same, never back to where it stood.
    /// Quiet panes or text-dense, in both drawing modes.
    #[test]
    fn a_dash_home_has_her_lunch_whatever_the_chat(
        from in 0u64..2000,
        wordy in any::<bool>(),
        graphics in any::<bool>(),
        chats in proptest::collection::vec((0u64..6000, 0u64..400), 0..4),
        ends in proptest::option::of(0u64..400),
        closet in proptest::option::weighted(0.25, (any::<bool>(), 0u64..5000)),
    ) {
        let dash = dash_seed(from);
        let screen = if wordy { wordy_home_screen() } else { home_screen() };
        let mut pokes: Vec<Poke> = chats
            .into_iter()
            .map(|(into, lag)| Poke::Chat { into, lag })
            .collect();
        pokes.extend(ends.map(|late| Poke::ChatAsItEnds { late }));
        pokes.extend(closet.map(|(walking, into)| Poke::Closet { walking, into }));
        let run = lunch_run(dash, &screen, graphics, &pokes, 300_000);
        let out = run.out;
        prop_assert!(out.is_some(), "never out again: {run:?}");
        prop_assert!(run.cuts.iter().all(|c| c.0), "a line she didn't look at: {run:?}");
        match run.closeted {
            None => {
                // Each cut, she has it again after: its line too.
                let since = run.cuts.last().map_or(0, |c| c.1);
                prop_assert!(
                    run.said[since..].contains(&FORGOT_LUNCH),
                    "out without her lunch: {run:?}"
                );
                prop_assert!(
                    run.lunches.len() <= 1 + run.cuts.len(),
                    "had again, uncut: {run:?}"
                );
            }
            Some(closeted) => {
                let after = run.lunches.iter().filter(|l| l.0 > closeted).count();
                prop_assert!(after <= 1, "back to where it stood: {run:?}");
            }
        }
    }

    /// [`her_days_never_touch_what_is_protected`] across a dash home:
    /// from a game minute before one, a fridge among her things, she
    /// dashes in through her door and out again (for her lunch, or for
    /// nothing she can remember), and nothing protected is painted, no
    /// image of hers hides text, and a key rains it all back. Long
    /// enough that nearly every dash reaches its end (her way out, and
    /// what it showed raining out), not only its start.
    ///
    /// [`her_days_never_touch_what_is_protected`]: super::away
    #[test]
    fn a_dash_home_never_touches_what_is_protected(
        seed in any::<u64>(),
        from in 1u64..8,
        graphics in any::<bool>(),
        sizes in proptest::collection::vec((48u16..130, 14u16..45), 1..3),
        text in proptest::collection::vec((0u16..60, 0u16..18, "[a-z漢─│ ]{1,6}"), 0..20),
        skips in proptest::collection::vec((0u16..60, 0u16..18), 0..6),
        chats in proptest::collection::vec(0u64..120_000, 0..3),
        protect in (0u16..40, 0u16..10, 1u16..20, 1u16..6),
        owned in proptest::collection::vec((0usize..4, 0usize..3, 0u16..=1000, any::<bool>()), 0..3),
        fridge in (0usize..3, 0u16..=1000),
    ) {
        let mut owned: Vec<(Furniture, usize, u16, bool)> = owned
            .into_iter()
            .map(|(item, at, along, left)| (Furniture::ALL[item], at, along, left))
            .collect();
        owned.push((Furniture::Fridge, fridge.0, fridge.1, false));
        let mut guest = Guest::restore(Ledger::new_at(seed, before_a_dash(seed, from)));
        guest.set_date(date(2026, 6, 17));
        long_visit_of(
            guest,
            graphics,
            rooms_frame,
            &sizes,
            &text,
            &skips,
            &chats,
            protect,
            &owned,
            Run::default(),
            120_000,
        )?;
    }
}

/// An overlay coming up on the paint she'd dash home on (her dash
/// decided at the tick before), her home standing empty: the dash is
/// off, and her home rains out, never just vanishing.
#[test]
fn an_overlay_as_she_dashes_home_rains_her_empty_home_out() {
    let (real, home) = home_screen();
    let home = IdleView {
        resident: true,
        ..home
    };
    let covered = IdleView {
        busy: Some(Busy::Overlay),
        ..home.clone()
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let (seed, minute) = dash_seed(0);
        let mut guest = home_at(seed, tue_at(minute - 2), &FRIDGE_HOME, graphics);
        let mut now = until_visiting_or_away(&mut guest, &real, &home);
        shell_step(&mut guest, &real, &home, &mut now, true);
        assert!(
            empty_of(&guest).is_some_and(|empty| !empty.painted.is_empty()),
            "{at}: her home shown"
        );
        let dash = real_of(&guest, now, tue_at(minute));
        loop {
            assert!(now < dash + 30_000, "{at}: never dashed");
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            guest.advance(now);
            if dashing(&guest) {
                break;
            }
            paint(&mut guest, &real, &home, now);
        }
        assert!(
            matches!(guest.state, State::Arriving(How::Dash, Some(_))),
            "{at}: dashing in from her home"
        );
        paint(&mut guest, &real, &covered, now);
        assert!(
            matches!(guest.state, State::Leaving(_)),
            "{at}: her home rains out"
        );
        assert!(!waving(&guest), "{at}: nobody in it to wave");
    }
}
