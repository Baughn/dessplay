//! Her calendar (phase 5b D5 with A23; step 6): what the real date owes
//! her is owed once a day, checked as she decides against the date she's
//! given, and delivered as it shows (its line in her drawn bubble, its
//! script playing), not as it's set; recorded in her ledger, so a second
//! visit that date has none of it and the next date's has its own. With
//! no date, nothing. Each in both drawing modes.

use super::*;
use crate::ui::houseguest::calendar::{CHRISTMAS, HALLOWEEN, HAPPY_NEW_YEAR};
use crate::ui::houseguest::osaka::Bubble;
use crate::ui::houseguest::script::{FIRST_SUNRISE_LINE, FUKU_WA_UCHI, ONI_WA_SOTO, ScriptId};

/// Her home on [`home_screen`]: a sofa and a TV, and her bed.
const HOME: [(Furniture, Nook, u16); 3] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Tv, Nook::Users, 800),
    (Furniture::Bed, Nook::Playlist, 300),
];

/// Her home with no TV.
const NO_TV: [(Furniture, Nook, u16); 2] = [
    (Furniture::Sofa, Nook::Users, 300),
    (Furniture::Bed, Nook::Playlist, 300),
];

/// Steps as the shell takes them from `now` for `ms`, painting each:
/// every line of hers on screen, in order (once while it stays), with
/// when it first showed.
fn watch(
    guest: &mut Guest,
    real: &Buffer,
    view: &IdleView,
    now: &mut u64,
    ms: u64,
) -> Vec<(u64, &'static str)> {
    let until = *now + ms;
    let mut shown: Vec<(u64, &'static str)> = Vec::new();
    while *now < until {
        // As the shell steps (`shell_step`), never past `until`.
        *now += guest
            .next_tick(*now)
            .map_or(1000, |d| d.as_millis() as u64)
            .clamp(1, 1000)
            .min(until - *now);
        guest.advance(*now);
        paint(guest, real, view, *now);
        if let State::Visiting(visit) = &guest.state
            && !visit.osaka.hidden(*now)
            && let (_, _, Some(Bubble::Say(line))) = visit.osaka.appearance(*now)
            && shown.last().map(|&(_, l)| l) != Some(line)
        {
            shown.push((*now, line));
        }
    }
    shown
}

/// How many times `line` showed.
fn times(shown: &[(u64, &str)], line: &str) -> usize {
    shown.iter().filter(|&&(_, l)| l == line).count()
}

/// Ends her visit with a key, and waits for her to come again (the idle
/// gate reopening): when she's back.
fn again(guest: &mut Guest, real: &Buffer, view: &IdleView, now: u64) -> u64 {
    guest.activity(now);
    until_visiting(guest, real, view, now)
}

/// Owed once a day: on her first visit that date its greeting shows in
/// her mood's stead, and her ledger records the date; a second visit
/// that date has none of it; the next date's visit has its own (Dec 24,
/// then Dec 25: Christmas both days); a date owing nothing, and no date
/// at all, have none, and leave her record as it was.
#[test]
fn owed_once_a_day_over_visits_and_dates() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(5, mon(16, 0), &HOME, graphics);
        let oct31 = date(2026, 10, 31);
        guest.set_date(oct31);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let mood = visit_of(&guest).osaka.mood();
        let shown = watch(&mut guest, &real, &view, &mut now, 40_000);
        assert_eq!(times(&shown, HALLOWEEN), 1, "{at}: {shown:?}");
        assert_eq!(times(&shown, mood.greeting()), 0, "{at}: in its stead");
        assert_eq!(guest.ledger.calendar_on, oct31, "{at}");
        // Again the same date: nothing of it.
        now = again(&mut guest, &real, &view, now);
        let shown = watch(&mut guest, &real, &view, &mut now, 40_000);
        assert_eq!(times(&shown, HALLOWEEN), 0, "{at}: {shown:?}");
        assert_eq!(guest.ledger.calendar_on, oct31, "{at}");
        // Christmas Eve, then Christmas Day: each its own.
        for day in [24, 25] {
            guest.set_date(date(2026, 12, day));
            now = again(&mut guest, &real, &view, now);
            let shown = watch(&mut guest, &real, &view, &mut now, 40_000);
            assert_eq!(times(&shown, CHRISTMAS), 1, "{at}: Dec {day}: {shown:?}");
            assert_eq!(guest.ledger.calendar_on, date(2026, 12, day), "{at}");
        }
        // Nothing owed, no date: nothing, and her record stands.
        for none in [date(2026, 12, 26), None] {
            guest.set_date(none);
            now = again(&mut guest, &real, &view, now);
            let shown = watch(&mut guest, &real, &view, &mut now, 40_000);
            assert!(
                shown.iter().all(|&(_, l)| !calendar_line(l)),
                "{at}: {none:?}: {shown:?}"
            );
            assert_eq!(guest.ledger.calendar_on, date(2026, 12, 25), "{at}");
        }
        // As saved: the date she last delivered it.
        let saved = guest.final_ledger(now).expect("touched");
        assert_eq!(saved.calendar_on, date(2026, 12, 25), "{at}");
    }
}

/// Whether `line` is one her calendar has her say.
fn calendar_line(line: &str) -> bool {
    [
        HALLOWEEN,
        CHRISTMAS,
        HAPPY_NEW_YEAR,
        FIRST_SUNRISE_LINE,
        ONI_WA_SOTO,
        FUKU_WA_UCHI,
    ]
    .contains(&line)
}

/// A resident on screen as the date changes (from Oct 30, owing
/// nothing, to Oct 31) gets the new day's as she next decides: an owed
/// beat, spacing out with its line; then never again that visit.
#[test]
fn a_resident_across_the_dates_change_gets_the_new_days() {
    let (real, view) = home_screen();
    let view = IdleView {
        resident: true,
        ..view
    };
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(9, mon(16, 0), &HOME, graphics);
        guest.set_date(date(2026, 10, 30));
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let shown = watch(&mut guest, &real, &view, &mut now, 30_000);
        assert_eq!(times(&shown, HALLOWEEN), 0, "{at}: {shown:?}");
        assert_eq!(guest.ledger.calendar_on, None, "{at}");
        guest.set_date(date(2026, 10, 31));
        let changed = now;
        let shown = watch(&mut guest, &real, &view, &mut now, 90_000);
        assert_eq!(times(&shown, HALLOWEEN), 1, "{at}: {shown:?}");
        assert_eq!(guest.ledger.calendar_on, date(2026, 10, 31), "{at}");
        let osaka = &visit_of(&guest).osaka;
        let beat = osaka
            .decisions
            .iter()
            .find(|d| d.at >= changed && d.method == "calendar")
            .unwrap_or_else(|| panic!("{at}: no calendar beat"));
        assert_eq!(beat.bucket, osaka::Bucket::Owed, "{at}");
        assert_eq!(beat.act, "SpaceOut", "{at}");
        assert!(
            matches!(guest.state, State::Visiting(_)),
            "{at}: still here"
        );
    }
}

/// Tucked in on Christmas Eve's night, her calendar waits for her to
/// wake: nothing of it while she sleeps; at her wake (Tuesday 07:00) a
/// plain "Mornin'." (her greeting is her wake line's mood part), then
/// its greeting as the first beat she owes; delivered then, not before.
#[test]
fn tucked_in_it_waits_for_her_wake() {
    let (real, view) = home_screen();
    let mornin = brain::Mood::Ordinary.wake_line();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(6, tue(6, 30), &HOME, graphics);
        guest.set_date(date(2026, 12, 24));
        guest.advance(0);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        assert!(asleep(&guest), "{at}: tucked in");
        let wake = real_of(&guest, now, tue(7, 0));
        let ms = wake - 1 - now;
        let shown = watch(&mut guest, &real, &view, &mut now, ms);
        assert_eq!(times(&shown, CHRISTMAS), 0, "{at}: {shown:?}");
        assert_eq!(guest.ledger.calendar_on, None, "{at}: asleep");
        let mood = visit_of(&guest).osaka.mood();
        let ms = wake + 30_000 - now;
        let shown = watch(&mut guest, &real, &view, &mut now, ms);
        let lines: Vec<&str> = shown.iter().take(2).map(|&(_, l)| l).collect();
        assert_eq!(lines, [mornin, CHRISTMAS], "{at}: {shown:?}");
        for other in brain::Mood::ALL.map(brain::Mood::wake_line) {
            if other != mornin {
                assert_eq!(times(&shown, other), 0, "{at}: {other:?}: {shown:?}");
            }
        }
        assert_eq!(times(&shown, mood.greeting()), 0, "{at}: no hello");
        assert_eq!(guest.ledger.calendar_on, date(2026, 12, 24), "{at}");
    }
}

/// Delivered when shown, not when said: stepped with no frame painted,
/// she says it (again as she moves: never seen, it's owed still), and
/// her record has nothing; painted, it shows, and is delivered.
#[test]
fn delivered_only_when_shown() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(5, mon(16, 0), &HOME, graphics);
        guest.set_date(date(2026, 10, 31));
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let said = |guest: &Guest| {
            visit_of(guest)
                .osaka
                .said_lines()
                .iter()
                .filter(|&&(pool, ..)| pool == mind::PoolId::Calendar)
                .count()
        };
        // No frames: said, and not delivered, however long.
        let end = now + 60_000;
        while now < end {
            now += guest
                .next_tick(now)
                .map_or(1000, |d| d.as_millis() as u64)
                .clamp(1, 1000);
            guest.advance(now);
        }
        assert!(said(&guest) > 0, "{at}: never said");
        assert_eq!(guest.ledger.calendar_on, None, "{at}: never shown");
        // Frames again: it shows, and is delivered.
        let shown = watch(&mut guest, &real, &view, &mut now, 90_000);
        assert_eq!(times(&shown, HALLOWEEN), 1, "{at}: {shown:?}");
        assert_eq!(guest.ledger.calendar_on, date(2026, 10, 31), "{at}");
    }
}

/// Setsubun (Feb 3) has no greeting (her mood's hello as ever, or a
/// drop-in's "...I'm OK."): its beans, owed, spacing out on the spot,
/// jumping foot to foot (each throw on the other foot), "Oni wa soto!"
/// then "Fuku wa uchi!"; delivered as they play; and not again that
/// date.
#[test]
fn setsubuns_beans_are_owed_once() {
    use crate::ui::houseguest::script::Posed;
    // Foot to foot: no two throws in a row on one.
    let feet: Vec<u8> = ScriptId::Setsubun
        .keys(0)
        .iter()
        .map(|key| match key.pose {
            Posed::Still(sprite::Pose::Jack(foot)) => foot,
            other => panic!("not a throw: {other:?}"),
        })
        .collect();
    assert_eq!(feet.len(), 8);
    assert!(feet.windows(2).all(|w| w[0] != w[1]), "{feet:?}");
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(5, mon(16, 0), &HOME, graphics);
        let feb3 = date(2027, 2, 3);
        guest.set_date(feb3);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        let mood = visit_of(&guest).osaka.mood();
        let (mut played, mut jumped) = (false, [false; 2]);
        let mut shown: Vec<(u64, &str)> = Vec::new();
        let mut hello = None;
        let end = now + 60_000;
        while now < end {
            shell_step(&mut guest, &real, &view, &mut now, true);
            let osaka = &visit_of(&guest).osaka;
            let (pose, _, bubble) = osaka.appearance(now);
            if hello.is_none()
                && let Some(Bubble::Say(line)) = bubble
            {
                hello = Some(line);
            }
            if osaka.plays().is_some_and(|p| p.own == ScriptId::Setsubun) {
                played = true;
                if let sprite::Pose::Jack(foot) = pose {
                    jumped[usize::from(foot)] = true;
                }
                if let Some(Bubble::Say(line)) = bubble
                    && shown.last().map(|&(_, l)| l) != Some(line)
                {
                    shown.push((now, line));
                }
                // Delivered as it plays, not before.
                assert_eq!(guest.ledger.calendar_on, feb3, "{at}");
            }
        }
        assert!(played && jumped == [true; 2], "{at}: no beans");
        let lines: Vec<&str> = shown.iter().map(|&(_, l)| l).collect();
        assert_eq!(lines, [ONI_WA_SOTO, FUKU_WA_UCHI], "{at}");
        let hello = hello.unwrap_or_else(|| panic!("{at}: no hello"));
        assert!(
            hello == mood.greeting() || hello == osaka::OK,
            "{at}: {hello:?}"
        );
        let decisions = &visit_of(&guest).osaka.decisions;
        assert!(
            decisions
                .iter()
                .any(|d| d.method == "calendar" && d.bucket == osaka::Bucket::Owed),
            "{at}"
        );
        // Again the same date: no beans.
        now = again(&mut guest, &real, &view, now);
        let end = now + 60_000;
        while now < end {
            shell_step(&mut guest, &real, &view, &mut now, true);
            let plays = visit_of(&guest).osaka.plays();
            assert!(
                plays.is_none_or(|p| p.own != ScriptId::Setsubun),
                "{at}: again"
            );
        }
    }
}

/// New Year's Day: "Happy New Year!" in her hello's stead, delivered as
/// it shows (the day's owed entry is hers then); then, with a TV, to it
/// for the first sunrise, "Ooh... first sunrise.", the sunrise on it,
/// and not the shopping channel's advert, though it's on (nothing
/// bought). With no TV, the greeting is all of it.
#[test]
fn new_years_day_brings_her_first_sunrise() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        for tv in [true, false] {
            let at = format!("graphics={graphics} tv={tv}");
            let pieces: &[(Furniture, Nook, u16)] = if tv { &HOME } else { &NO_TV };
            let mut guest = home_at(5, mon(16, 0), pieces, graphics);
            guest.shop_now = true;
            let jan1 = date(2027, 1, 1);
            guest.set_date(jan1);
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let (mut greeted, mut sunrise) = (None, None);
            let end = now + 120_000;
            while now < end {
                let was = guest.ledger.calendar_on;
                shell_step(&mut guest, &real, &view, &mut now, true);
                let osaka = &visit_of(&guest).osaka;
                let bubble = osaka.appearance(now).2;
                if greeted.is_none() && bubble == Some(Bubble::Say(HAPPY_NEW_YEAR)) {
                    greeted = Some(now);
                    assert_eq!(was, None, "{at}: not before it shows");
                    assert_eq!(guest.ledger.calendar_on, jan1, "{at}: as it shows");
                }
                if sunrise.is_none()
                    && osaka
                        .plays()
                        .is_some_and(|p| p.own == ScriptId::FirstSunrise)
                {
                    sunrise = Some(now);
                    assert_eq!(bubble, Some(Bubble::Say(FIRST_SUNRISE_LINE)), "{at}");
                    assert_eq!(
                        osaka.prop(now),
                        Some(script::Prop::Tv(art::Channel::Sunrise)),
                        "{at}"
                    );
                    assert!(visit_of(&guest).chances.advert.is_some(), "{at}: on");
                }
                assert_eq!(guest.ledger.ordered, None, "{at}: bought");
            }
            let greeted = greeted.unwrap_or_else(|| panic!("{at}: no greeting"));
            assert_eq!(guest.ledger.calendar_on, jan1, "{at}");
            match sunrise {
                Some(sunrise) => assert!(tv && sunrise > greeted, "{at}"),
                None => assert!(!tv, "{at}: no sunrise"),
            }
        }
    }
}

/// New Year's Day's greeting is owed once that day, sunrise or no: a
/// visit cut short as it shows (before she's at her TV) has had it, and
/// her next visits that day say nothing of it.
#[test]
fn new_years_greeting_is_once_a_day_sunrise_or_no() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(5, mon(16, 0), &HOME, graphics);
        let jan1 = date(2027, 1, 1);
        guest.set_date(jan1);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        // Cut short the moment it shows.
        while guest.ledger.calendar_on.is_none() {
            assert!(now < 60_000, "{at}: never shown");
            shell_step(&mut guest, &real, &view, &mut now, true);
        }
        for _ in 0..2 {
            now = again(&mut guest, &real, &view, now);
            let shown = watch(&mut guest, &real, &view, &mut now, 30_000);
            assert_eq!(times(&shown, HAPPY_NEW_YEAR), 0, "{at}: {shown:?}");
            assert_eq!(guest.ledger.calendar_on, jan1, "{at}");
        }
    }
}

/// Home from school on Halloween: "I'm home!" (no hello: it's the same
/// day), then her calendar's line as the first beat she owes.
#[test]
fn home_from_school_its_the_first_beat_she_owes() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let at = format!("graphics={graphics}");
        let mut guest = home_at(4, tue(12, 30), &HOME, graphics);
        guest.set_date(date(2026, 10, 31));
        let mut now = 0;
        guest.advance(now);
        let back = real_of(&guest, now, tue(12, 45));
        let ms = back + 60_000 - now;
        let shown = watch(&mut guest, &real, &view, &mut now, ms);
        let home = shown
            .iter()
            .position(|&(_, l)| mind::HOME.lines.contains(&l))
            .unwrap_or_else(|| panic!("{at}: never home: {shown:?}"));
        let owed = shown
            .iter()
            .position(|&(_, l)| l == HALLOWEEN)
            .unwrap_or_else(|| panic!("{at}: never owed: {shown:?}"));
        assert_eq!(owed, home + 1, "{at}: {shown:?}");
        assert_eq!(guest.ledger.calendar_on, date(2026, 10, 31), "{at}");
    }
}

/// On a visit that owes it, her calendar's greeting is her hello, in
/// her mood's stead: the first thing she says as she comes in by door
/// or edge; come in by falling ("...I'm OK.", which does for a hello),
/// it's the next thing she says. Over seeds, so both kinds come; never
/// her mood's hello.
#[test]
fn on_an_owed_visit_its_greeting_is_her_hello() {
    let (real, view) = home_screen();
    for graphics in [false, true] {
        let (mut walked, mut fell) = (0, 0);
        for seed in 0..12 {
            let at = format!("graphics={graphics} seed={seed}");
            let mut guest = home_at(seed, mon(16, 0), &HOME, graphics);
            guest.set_date(date(2026, 10, 31));
            let mut now = until_visiting(&mut guest, &real, &view, 0);
            let mood = visit_of(&guest).osaka.mood();
            let shown = watch(&mut guest, &real, &view, &mut now, 60_000);
            let lines: Vec<&str> = shown.iter().map(|&(_, l)| l).collect();
            match lines.as_slice() {
                [osaka::OK, HALLOWEEN, ..] => fell += 1,
                [HALLOWEEN, ..] => walked += 1,
                _ => panic!("{at}: {lines:?}"),
            }
            assert_eq!(times(&shown, mood.greeting()), 0, "{at}: {lines:?}");
            assert_eq!(times(&shown, HALLOWEEN), 1, "{at}: {lines:?}");
        }
        assert!(
            walked > 0 && fell > 0,
            "graphics={graphics}: {walked} {fell}"
        );
    }
}

/// Whether `line` is on screen in `buf`.
fn on_screen(buf: &Buffer, line: &str) -> bool {
    let area = buf.area;
    (area.top()..area.bottom()).any(|y| row_text(buf, y, area.left()..area.right()).contains(line))
}

/// Delivered only on a frame that shows its line: quiet panes or text
/// dense, in both drawing modes, the frame her record takes the date on
/// has the line on screen, painted (not merely hers to say).
#[test]
fn delivered_on_a_frame_with_its_line_on_screen() {
    for (screen, (real, view)) in home_screens() {
        for graphics in [false, true] {
            for seed in [4, 5, 9] {
                let at = format!("{screen} graphics={graphics} seed={seed}");
                let mut guest = home_at(seed, mon(16, 0), &HOME, graphics);
                guest.set_date(date(2026, 12, 25));
                let mut now = until_visiting(&mut guest, &real, &view, 0);
                let end = now + 120_000;
                while guest.ledger.calendar_on.is_none() {
                    assert!(now < end, "{at}: never shown");
                    now += guest
                        .next_tick(now)
                        .map_or(1000, |d| d.as_millis() as u64)
                        .clamp(1, 1000);
                    guest.advance(now);
                    let buf = paint(&mut guest, &real, &view, now);
                    if guest.ledger.calendar_on.is_some() {
                        assert!(on_screen(&buf, CHRISTMAS), "{at}: delivered unseen");
                    }
                }
            }
        }
    }
}

/// Her home with a fridge (for her meals) and her bed.
const FRIDGE: [(Furniture, Nook, u16); 2] = [
    (Furniture::Fridge, Nook::Users, 0),
    (Furniture::Bed, Nook::Playlist, 300),
];

/// Her meal's line is the slot's, not the visit's (the guest carries it
/// to her next visit): her first snack of a Saturday morning says
/// "Breakfast!"; her next visit that morning, her snack says nothing;
/// that evening's first, "Dinner time~"; and Sunday morning's first,
/// "Breakfast!" again.
#[test]
fn her_meal_line_is_once_a_slot_across_visits() {
    let (real, view) = home_screen();
    let (breakfast, dinner) = (mind::BREAKFAST.lines[0], mind::DINNER.lines[0]);
    for graphics in [false, true] {
        let mut guest = home_at(5, sat(9, 0), &FRIDGE, graphics);
        let mut now = until_visiting(&mut guest, &real, &view, 0);
        // A snack, cued: the slot she has it in, whether she did, and
        // her lines over it.
        let snack = |guest: &mut Guest, now: &mut u64| {
            guest.cue(Scene::Snack);
            let slot = guest.routine_clock(*now).map(|clock| clock.day(*now).slot);
            let (mut snacked, mut shown) = (false, Vec::new());
            let until = *now + 20_000;
            while *now < until {
                shell_step(guest, &real, &view, now, true);
                let osaka = &visit_of(guest).osaka;
                snacked |= osaka
                    .playing()
                    .is_some_and(|(seat, ..)| seat.what == room::Use::Snack);
                if let (_, _, Some(Bubble::Say(line))) = osaka.appearance(*now)
                    && shown.last() != Some(&line)
                {
                    shown.push(line);
                }
            }
            (slot, snacked, shown)
        };
        let mut meals = Vec::new();
        for skips in [0, 0, 2, 2] {
            let at = format!("graphics={graphics} skips={skips}");
            if !meals.is_empty() {
                guest.activity(now);
                for _ in 0..skips {
                    guest.skip_clock(now);
                }
                now = until_visiting(&mut guest, &real, &view, now);
            }
            let (slot, snacked, shown) = snack(&mut guest, &mut now);
            assert!(snacked, "{at}: no snack: {shown:?}");
            let times = |line| shown.iter().filter(|&&l| l == line).count();
            meals.push((slot, times(breakfast), times(dinner)));
        }
        use routine::Slot::{Evening, Morning};
        assert_eq!(
            meals,
            [
                (Some(Morning), 1, 0),
                (Some(Morning), 0, 0),
                (Some(Evening), 0, 1),
                (Some(Morning), 1, 0),
            ],
            "graphics={graphics}"
        );
    }
}
