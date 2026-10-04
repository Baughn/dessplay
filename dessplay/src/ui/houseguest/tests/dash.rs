//! Dashes home (phase 5b D3a, A16, Q2; step 5c): now and then on a
//! school day she dashes home through her closed door for something she
//! forgot (her lunch, from her fridge; with none, she can't think what),
//! and goes out again by her door. An errand at school is a dash too. A
//! dash never counts as a visit, and brings no parcel. Each in both
//! drawing modes, on quiet and text-dense screens where drawing matters.

use super::*;
use crate::ui::houseguest::art::PieceState;
use crate::ui::houseguest::osaka::Bubble;
use crate::ui::houseguest::script::{FORGOT_LUNCH, FORGOT_SOMETHING, WHAT_WAS_IT};

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
    (from..)
        .find_map(|seed| brain::dash(seed, TUESDAY).map(|m| (seed, m)))
        .unwrap()
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
    matches!(guest.state, State::Arriving(How::Dash))
        || matches!(&guest.state, State::Visiting(visit) if visit.kind == Kind::Dash)
}

/// One step as the shell takes it from `now`, waking as late as she
/// asks (up to a minute): a tick, and a paint if it says the screen could
/// change.
fn long_step(guest: &mut Guest, real: &Buffer, view: &IdleView, now: &mut u64) {
    *now += guest
        .next_tick(*now)
        .map_or(60_000, |d| d.as_millis() as u64)
        .clamp(1, 60_000);
    if guest.advance(*now) {
        paint(guest, real, view, *now);
    }
}

/// What a dash home looked like, frame by frame.
#[derive(Debug, Default)]
struct Seen {
    /// When she dashed in, and where her closed door stood then.
    came: Option<(u64, Option<DoorAt>)>,
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
/// ms. A dash never counts as a visit, never ends in a goodbye, and
/// brings no parcel while it lasts.
fn watch_dash(guest: &mut Guest, real: &Buffer, view: &IdleView, mut now: u64, limit: u64) -> Seen {
    let visits = guest.ledger.visits;
    let ordered = guest.ledger.ordered;
    let mut seen = Seen::default();
    let end = now + limit;
    while now < end && seen.back.is_none() {
        let door = guest.closed_door();
        shell_step(guest, real, view, &mut now, true);
        assert!(
            !matches!(guest.state, State::Leaving(_)),
            "a goodbye at {now}"
        );
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
            assert!(came >= due, "{at}: dashed home at {came}, before {due}");
            let door = door.unwrap_or_else(|| panic!("{at}: no door"));
            assert_eq!(seen.first, Some((door.x, door.y)), "{at}: out of her door");
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
    let none = (0..)
        .find(|&seed| brain::dash(seed, TUESDAY).is_none())
        .unwrap();
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
/// straight back out by her door: no dissolve, even for a visitor whose
/// video plays (she's out on her routine's way, not his); not counted;
/// then her home stands empty again (a resident's), or she's simply out
/// (a visitor at the video: it shows when he's idle). With no home she
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
            let (seed, _) = (0..)
                .map(|seed| (seed, brain::dash(seed, TUESDAY)))
                .find(|(_, dash)| dash.is_none())
                .unwrap();
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
                assert!(
                    !matches!(guest.state, State::Leaving(_)),
                    "{at}: a dissolve at {now}"
                );
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
            assert!(
                guest.out.is_some_and(|o| o.door.is_some()),
                "{at}: out by her door"
            );
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
    let (seed, _) = (0..)
        .map(|seed| (seed, brain::dash(seed, TUESDAY)))
        .find(|(_, dash)| dash.is_none())
        .unwrap();
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

/// Her clock two game minutes before `seed`'s first dash home on a
/// school day from game day `from` on.
fn before_a_dash(seed: u64, from: u64) -> routine::GameTime {
    let (day, minute) = (from..)
        .find_map(|day| {
            let school = routine::weekday(day).num_days_from_monday() < 5;
            brain::dash(seed, day)
                .filter(|_| school)
                .map(|minute| (day, minute))
        })
        .unwrap();
    routine::GameTime {
        day,
        h: (minute - 2) / 60,
        m: (minute - 2) % 60,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(16)))]

    /// [`her_days_never_touch_what_is_protected`] across a dash home:
    /// from two game minutes before one, a fridge among her things, she
    /// dashes in through her door and out again (for her lunch, or for
    /// nothing she can remember), and nothing protected is painted, no
    /// image of hers hides text, and a key rains it all back.
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
        chats in proptest::collection::vec(0u64..60_000, 0..3),
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
        long_visit_of(guest, graphics, &sizes, &text, &skips, &chats, protect, &owned, 60_000)?;
    }
}
