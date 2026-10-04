//! Her routine: what part of her day it is at a moment of her game
//! clock, and on which real dates school is out (phase 5b, design D2
//! with A12). Pure: every answer is a function of game time and each
//! game day's own vacation flag, from the const tables below.
//!
//! Game time here is what [`GameClock::at`] reads: game millis since
//! [`START`], Monday 16:00 of game day 0. Every routine question (the
//! slot now, the next boundary, "is this a school day") goes through
//! this module; the mind, the guest and the art read slots, never raw
//! minutes.

use chrono::{Datelike, NaiveDate, Weekday};

use super::GameClock;

/// A game minute, in game millis.
const MINUTE_MS: u64 = 60_000;
/// Minutes in a day.
const DAY_MIN: u64 = 1440;

/// Her clock's start, in minutes from midnight of game day 0 (a Monday):
/// 16:00. Part of the saved format (`Ledger.clock` counts from it), so it
/// never changes.
pub const START: u64 = 16 * 60;

/// `h:m` as minutes from midnight.
const fn hm(h: u16, m: u16) -> u16 {
    h * 60 + m
}

/// She gets up on a school day.
const WAKE_SCHOOL: u16 = hm(7, 0);
/// She gets up on any other day.
const WAKE_OFF: u16 = hm(9, 0);
/// She leaves for school, and comes back (Q1: school runs 08:30-12:30).
const AWAY: (u16, u16) = (hm(8, 15), hm(12, 45));
/// The (slow) morning of a day off ends; the afternoon starts on every day.
const AFTERNOON: u16 = hm(12, 45);
/// Dinner, TV.
const EVENING: u16 = hm(18, 0);
/// Homework, on a school night.
const HOMEWORK: u16 = hm(20, 0);
/// She goes to bed the evening before a school day.
const BED_SCHOOL_NIGHT: u16 = hm(22, 30);
/// She goes to bed on any other evening.
const BED_OFF: u16 = hm(23, 30);
/// The part-time job is open on a day off, from and until.
const WORK: (u16, u16) = (hm(10, 0), hm(17, 0));

/// Every minute of a day at which a slot may begin. Slots change only at
/// these (a test scans the tape to prove it): never at midnight, which
/// is deep inside every night.
const BOUNDARIES: [u16; 8] = [
    WAKE_SCHOOL,
    AWAY.0,
    WAKE_OFF,
    AFTERNOON,
    EVENING,
    HOMEWORK,
    BED_SCHOOL_NIGHT,
    BED_OFF,
];

/// A real-date window, inclusive at both ends, as (month, day); one whose
/// end comes before its start runs over the new year.
pub type DateWindow = ((u32, u32), (u32, u32));

/// School's out (Q4): summer, the year's end, spring.
const VACATIONS: [DateWindow; 3] = [((7, 20), (8, 31)), ((12, 25), (1, 7)), ((3, 25), (4, 5))];

/// Summer's last week: homework panic.
pub const PANIC_WEEK: DateWindow = ((8, 25), (8, 31));

/// Exam season (D5): homework weighs on her.
pub const EXAMS: DateWindow = ((1, 20), (3, 10));

/// Hay fever (D5): she sneezes more.
pub const HAY_FEVER: DateWindow = ((3, 1), (4, 30));

/// What part of her day it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Slot {
    /// In bed, from bedtime to the wake time.
    Asleep,
    /// Up: breakfast before school, or a slow morning on a day off.
    Morning,
    /// At school.
    Away,
    /// Snack, sofa, TV (on a day off, the part-time job may happen).
    Afternoon,
    /// Dinner, TV; reading on an evening before a day off.
    Evening,
    /// Homework, on a school night until bedtime.
    Homework,
}

impl Slot {
    /// Every slot.
    pub const ALL: [Slot; 6] = [
        Slot::Asleep,
        Slot::Morning,
        Slot::Away,
        Slot::Afternoon,
        Slot::Evening,
        Slot::Homework,
    ];

    /// Whether the routine cuts into what she's doing as this slot
    /// begins: she leaves for school, or goes to bed. Every other change
    /// waits for her next decision.
    pub fn cuts(self) -> bool {
        matches!(self, Slot::Away | Slot::Asleep)
    }
}

/// A set of slots, written as a const (`SlotSet::of(&[Slot::Evening])`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SlotSet(u8);

impl SlotSet {
    /// The set of `slots`.
    pub const fn of(slots: &[Slot]) -> SlotSet {
        let mut bits = 0;
        let mut i = 0;
        while i < slots.len() {
            bits |= 1 << slots[i] as u8;
            i += 1;
        }
        SlotSet(bits)
    }

    /// Whether `slot` is in the set.
    pub fn contains(self, slot: Slot) -> bool {
        self.0 & (1 << slot as u8) != 0
    }
}

/// When in her day something holds (a boost of her mind's, D4 lever 1),
/// as the routine answers it: the mind asks, the routine reads the
/// minute.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// In any of these slots.
    In(SlotSet),
    /// The evening before a day off: the evening slot of a day whose
    /// next isn't a school day (a weekend's or a vacation's, the reading
    /// evening of D2's table, homework-free).
    EveningOff,
    /// From the first game time of day (minutes since midnight) until
    /// the second, running over midnight when it's earlier: the sky as
    /// the window shows it (dusk and night: 17:00 until 05:00).
    Hours(u16, u16),
}

impl When {
    /// Whether it holds at `day`.
    pub fn holds(self, day: &DayTime) -> bool {
        match self {
            When::In(slots) => slots.contains(day.slot),
            When::EveningOff => day.slot == Slot::Evening && !day.night_before_school,
            When::Hours(from, until) => {
                if from <= until {
                    (from..until).contains(&day.minute)
                } else {
                    day.minute >= from || day.minute < until
                }
            }
        }
    }
}

/// Dusk, the evening and the night (D7's sky phases from 17:00 until
/// 05:00), as a [`When`].
pub const DUSK_TO_DAWN: When = When::Hours(hm(17, 0), hm(5, 0));

/// The vacation flag of one game day, read from the real date at that
/// day's first read and held for the rest of it (A12): a real-09:00
/// date flip mid-day changes nothing until the next game day.
///
/// Held per process: a restart is a cold start, which re-derives
/// everything (her state included) from game time and the date as it is
/// then, so no slot changes under her without a wakeup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Latch {
    /// The game day it was read on.
    pub day: u64,
    /// School is out.
    pub vacation: bool,
    /// The day before's flag, when that day was latched too (a catch-up
    /// decision just before midnight is judged by its own day's flag).
    pub eve: Option<bool>,
}

impl Latch {
    /// The latch for game `day`: `held` if it's that day's, else read
    /// afresh from `date` (`None`: no calendar, no vacation), keeping
    /// `held`'s flag as the eve's when it was the day before's.
    pub fn at(held: Option<Latch>, day: u64, date: Option<NaiveDate>) -> Latch {
        match held {
            Some(latch) if latch.day == day => latch,
            _ => Latch {
                day,
                vacation: date.is_some_and(vacation),
                eve: held
                    .filter(|h| h.day.checked_add(1) == Some(day))
                    .map(|h| h.vacation),
            },
        }
    }
}

/// Her clock, the day's vacation latch, and the date's flag as it reads
/// now: all the routine needs to answer for any moment near the
/// reading. Every game day is judged by its own flag ([`Clock::vacation_on`]).
///
/// An answer about a later day than the latch's (tomorrow's wake, its
/// school start) is **provisional**: that day latches when it comes,
/// from the date as it is then. Anything predicted into it (a wake
/// time, a cutting boundary) is recomputed whenever the latch changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clock {
    /// Her game clock.
    pub game: GameClock,
    /// The vacation flag of the day it was read on.
    pub latch: Latch,
    /// School is out by the date as it reads now: the provisional flag
    /// of every later day.
    pub ahead: bool,
    /// The real date as it reads now (`None`: unknown): her seasons
    /// (exam season, hay fever) read it, the slots never do.
    pub date: Option<NaiveDate>,
}

impl Clock {
    /// Her clock with the day at `now`'s latch (`held` if it's that
    /// day's, see [`Latch::at`]) and the flag `date` gives later days.
    pub fn read(game: GameClock, now: u64, held: Option<Latch>, date: Option<NaiveDate>) -> Clock {
        let (day, _) = split(game.at(now));
        Clock {
            game,
            latch: Latch::at(held, day, date),
            ahead: date.is_some_and(vacation),
            date,
        }
    }

    /// The vacation flag that judges game `day`: the latch's on its own
    /// day, the eve's the day before (the latch's when that's unknown),
    /// and the date's as it reads now on a later day (provisional).
    pub fn vacation_on(&self, day: u64) -> bool {
        match day.cmp(&self.latch.day) {
            std::cmp::Ordering::Equal => self.latch.vacation,
            std::cmp::Ordering::Greater => self.ahead,
            std::cmp::Ordering::Less => self
                .latch
                .eve
                .filter(|_| day.checked_add(1) == Some(self.latch.day))
                .unwrap_or(self.latch.vacation),
        }
    }

    /// The routine at the monotonic millis `at`.
    pub fn day(&self, at: u64) -> DayTime {
        self.day_of(self.game.at(at))
    }

    /// The routine at `game` (game millis since [`START`]), its day
    /// judged by its own flag.
    pub fn day_of(&self, game: u64) -> DayTime {
        let (day, _) = split(game);
        day_time(game, self.vacation_on(day))
    }

    /// When she wakes from a night asleep at `game` (game millis since
    /// [`START`]): the first boundary after it that ends the Asleep slot
    /// ([`next_wake`]).
    pub fn next_wake(&self, game: u64) -> u64 {
        next_wake(game, |day| self.vacation_on(day))
    }

    /// Her first wake time after `game` ([`next_morning`]).
    pub fn next_morning(&self, game: u64) -> u64 {
        next_morning(game, |day| self.vacation_on(day))
    }

    /// The next slot boundary after `game` (game millis since
    /// [`START`]), every day judged by its own flag ([`next_boundary`]).
    pub fn next_boundary(&self, game: u64) -> u64 {
        next_boundary(game, |day| self.vacation_on(day))
    }

    /// The next cutting boundary after `game` ([`next_cutting`]).
    pub fn next_cutting(&self, game: u64) -> u64 {
        next_cutting(game, |day| self.vacation_on(day))
    }
}

/// The routine at a moment: everything the mind, the guest and the art
/// ask of the clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DayTime {
    /// The game day (day 0 is the Monday her clock started on).
    pub day: u64,
    /// Minutes since that day's midnight.
    pub minute: u16,
    /// What part of her day it is.
    pub slot: Slot,
    /// She has school today.
    pub school_day: bool,
    /// She has school tomorrow (homework tonight, bed at 22:30).
    pub night_before_school: bool,
    /// School's out (the day's latch).
    pub vacation: bool,
    /// The game day's weekday.
    pub weekday: Weekday,
}

impl DayTime {
    /// Whether the part-time job is open now ([`work_open`]): on a day
    /// off, 10:00 to 17:00.
    pub fn work_open(&self) -> bool {
        work_open(self.day, self.minute, self.vacation)
    }
}

impl std::fmt::Display for DayTime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {:02}:{:02} {:?}",
            self.weekday,
            self.minute / 60,
            self.minute % 60,
            self.slot
        )?;
        if self.vacation {
            write!(f, " (vacation)")?;
        }
        Ok(())
    }
}

/// Absolute game minutes (since midnight of game day 0) at `game` game
/// millis since [`START`].
fn absolute(game: u64) -> u64 {
    (game / MINUTE_MS).saturating_add(START)
}

/// The game day and minute of the day at `game` game millis since
/// [`START`].
pub fn split(game: u64) -> (u64, u16) {
    let abs = absolute(game);
    // Below 1440: fits.
    (abs / DAY_MIN, (abs % DAY_MIN) as u16)
}

/// Game day `day`'s weekday: day 0 is a Monday.
pub fn weekday(day: u64) -> Weekday {
    match day % 7 {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

/// Whether game `day` is a school day: Monday to Friday, and not
/// vacation.
pub fn school_day(day: u64, vacation: bool) -> bool {
    !vacation && !matches!(weekday(day), Weekday::Sat | Weekday::Sun)
}

/// Whether game `day`'s evening is a school night: the next day is a
/// school day (judged by `day`'s flag).
pub fn night_before_school(day: u64, vacation: bool) -> bool {
    school_day(day.saturating_add(1), vacation)
}

/// When she gets up on game `day`.
fn wake(day: u64, vacation: bool) -> u16 {
    if school_day(day, vacation) {
        WAKE_SCHOOL
    } else {
        WAKE_OFF
    }
}

/// When she goes to bed on game `day`.
fn bed(day: u64, vacation: bool) -> u16 {
    if night_before_school(day, vacation) {
        BED_SCHOOL_NIGHT
    } else {
        BED_OFF
    }
}

/// What part of her day `minute` (since midnight) of game `day` is.
pub fn slot(day: u64, minute: u16, vacation: bool) -> Slot {
    let school = school_day(day, vacation);
    if minute < wake(day, vacation) || minute >= bed(day, vacation) {
        Slot::Asleep
    } else if school && minute < AWAY.0 {
        Slot::Morning
    } else if school && minute < AWAY.1 {
        Slot::Away
    } else if minute < AFTERNOON {
        Slot::Morning
    } else if minute < EVENING {
        Slot::Afternoon
    } else if minute < HOMEWORK || !night_before_school(day, vacation) {
        Slot::Evening
    } else {
        Slot::Homework
    }
}

/// The routine at `game` game millis since [`START`], with the day's
/// vacation flag.
pub fn day_time(game: u64, vacation: bool) -> DayTime {
    let (day, minute) = split(game);
    DayTime {
        day,
        minute,
        slot: slot(day, minute, vacation),
        school_day: school_day(day, vacation),
        night_before_school: night_before_school(day, vacation),
        vacation,
        weekday: weekday(day),
    }
}

/// The stretch of every night (minutes since midnight, from and until)
/// her midnight snack may fall in: inside every night (each covers 23:30
/// to 07:00, the latest bedtime to the earliest wake), clear of settling
/// in and of waking.
pub const SNACK_WINDOW: (u16, u16) = (hm(0, 30), hm(5, 30));

/// Game millis since [`START`] at `minute` (since midnight) of game
/// `day` (0 for a moment before her start).
pub fn game_of(day: u64, minute: u16) -> u64 {
    day.saturating_mul(DAY_MIN)
        .saturating_add(u64::from(minute))
        .saturating_sub(START)
        .saturating_mul(MINUTE_MS)
}

/// Whether the part-time job is open at `minute` of game `day`: on a day
/// off (a weekend, or vacation), 10:00 to 17:00.
pub fn work_open(day: u64, minute: u16, vacation: bool) -> bool {
    !school_day(day, vacation) && (WORK.0..WORK.1).contains(&minute)
}

/// The first slot boundary after `game` (game millis since [`START`])
/// that `which` accepts (it's given the slot that begins there), in game
/// millis since [`START`]. Each day is judged by its own flag,
/// `vacation(day)`: a day's bedtime by that day's, the next morning's
/// wake by the next day's.
fn next_where(game: u64, vacation: impl Fn(u64) -> bool, which: impl Fn(Slot) -> bool) -> u64 {
    let (today, _) = split(game);
    // Every day has a bedtime, a cutting boundary, so today or tomorrow
    // always holds the answer; the third day is a margin.
    for day in today..=today.saturating_add(2) {
        let vacation = vacation(day);
        for &minute in &BOUNDARIES {
            let abs = day * DAY_MIN + u64::from(minute);
            let Some(at) = abs.checked_sub(START).map(|m| m.saturating_mul(MINUTE_MS)) else {
                continue;
            };
            if at <= game {
                continue;
            }
            let begins = slot(day, minute, vacation);
            if begins != slot(day, minute - 1, vacation) && which(begins) {
                return at;
            }
        }
    }
    // Unreachable (see above); a day later is a safe answer.
    game.saturating_add(DAY_MIN * MINUTE_MS)
}

/// The next slot boundary after `game` (game millis since [`START`]), in
/// game millis since [`START`]: always a whole minute, and strictly
/// later. Each game day is judged by `vacation(day)`.
pub fn next_boundary(game: u64, vacation: impl Fn(u64) -> bool) -> u64 {
    next_where(game, vacation, |_| true)
}

/// The next **cutting** boundary after `game`: the start of school or
/// bedtime ([`Slot::cuts`]), as [`next_boundary`].
pub fn next_cutting(game: u64, vacation: impl Fn(u64) -> bool) -> u64 {
    next_where(game, vacation, Slot::cuts)
}

/// The first boundary after `game` (game millis since [`START`]) where a
/// slot other than Asleep begins: from a moment of the night, the
/// morning's wake time; each day judged by `vacation(day)`, so the
/// morning's by its own day's flag (a school morning at 07:00, a day off
/// at 09:00). Computed from the table, never assumed.
pub fn next_wake(game: u64, vacation: impl Fn(u64) -> bool) -> u64 {
    next_where(game, vacation, |slot| slot != Slot::Asleep)
}

/// Her first wake time after `game` (game millis since [`START`]): from
/// a moment of the night, its morning ([`next_wake`]); from one of the
/// day, the morning after the next bedtime. Each day judged by
/// `vacation(day)`.
pub fn next_morning(game: u64, vacation: impl Fn(u64) -> bool) -> u64 {
    let (day, _) = split(game);
    let night = if day_time(game, vacation(day)).slot == Slot::Asleep {
        game
    } else {
        next_where(game, &vacation, |slot| slot == Slot::Asleep)
    };
    next_wake(night, vacation)
}

/// A moment of her clock as a test names it: `h:m` of game `day`.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameTime {
    /// The game day (day 0 is the Monday her clock started on).
    pub day: u64,
    /// The hour.
    pub h: u16,
    /// The minute.
    pub m: u16,
}

#[cfg(test)]
impl GameTime {
    /// Game minutes since [`START`] (her ledger's clock) at this moment,
    /// which must be no earlier than her start.
    pub fn minutes(self) -> u64 {
        self.day * DAY_MIN + u64::from(hm(self.h, self.m)) - START
    }
}

/// Whether `date` falls in `window`.
pub fn within(date: NaiveDate, ((m0, d0), (m1, d1)): DateWindow) -> bool {
    let md = (date.month(), date.day());
    if (m0, d0) <= (m1, d1) {
        (m0, d0) <= md && md <= (m1, d1)
    } else {
        md >= (m0, d0) || md <= (m1, d1)
    }
}

/// Whether school is out on the real `date` (Q4): summer (Jul 20 - Aug
/// 31), the year's end (Dec 25 - Jan 7), spring (Mar 25 - Apr 5).
pub fn vacation(date: NaiveDate) -> bool {
    VACATIONS.iter().any(|&window| within(date, window))
}

/// Whether the real `date` is in summer's last week (Aug 25-31): homework
/// panic.
pub fn panic_week(date: NaiveDate) -> bool {
    within(date, PANIC_WEEK)
}

/// `game` (game millis since [`START`]) as the stage shows it: "Mon
/// 16:05".
pub fn label(game: u64) -> String {
    let (day, minute) = split(game);
    format!("{} {:02}:{:02}", weekday(day), minute / 60, minute % 60)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    /// Game millis since START at minute `minute` of game `day`.
    fn at(day: u64, minute: u64) -> u64 {
        (day * DAY_MIN + minute - START) * MINUTE_MS
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// The tape's first and last-plus-one absolute minute: one game week
    /// from Monday 00:00 of game day 7 (her clock starts on day 0 at 16:00).
    const TAPE: (u64, u64) = (7 * DAY_MIN, 14 * DAY_MIN);

    /// Each game day's vacation flag, as the latches would hold them: a
    /// school week, a vacation week, a vacation starting or ending on the
    /// tape's Wednesday (game day 9), and every other day.
    type Pattern = (&'static str, fn(u64) -> bool);
    const PATTERNS: [Pattern; 5] = [
        ("school", |_| false),
        ("vacation", |_| true),
        ("vacation from Wednesday", |day| day >= 9),
        ("vacation until Tuesday", |day| day <= 8),
        ("every other day", |day| day % 2 == 0),
    ];

    /// One game week's slots, minute by minute, each day by its flag.
    fn tape(flag: fn(u64) -> bool) -> Vec<(u64, u16, Slot)> {
        (TAPE.0..TAPE.1)
            .map(|abs| {
                let (day, minute) = (abs / DAY_MIN, (abs % DAY_MIN) as u16);
                (day, minute, slot(day, minute, flag(day)))
            })
            .collect()
    }

    /// The tape's runs: (slot, first absolute minute, length in minutes).
    fn runs(tape: &[(u64, u16, Slot)]) -> Vec<(Slot, u64, u64)> {
        let mut runs: Vec<(Slot, u64, u64)> = Vec::new();
        for &(day, minute, slot) in tape {
            let abs = day * DAY_MIN + u64::from(minute);
            match runs.last_mut() {
                Some((s, _, len)) if *s == slot => *len += 1,
                _ => runs.push((slot, abs, 1)),
            }
        }
        runs
    }

    #[test]
    fn her_clock_starts_on_a_monday_afternoon_at_four() {
        let day = day_time(0, false);
        assert_eq!((day.day, day.minute), (0, 16 * 60));
        assert_eq!(day.weekday, Weekday::Mon);
        assert_eq!(day.slot, Slot::Afternoon);
        assert!(day.school_day && day.night_before_school);
        assert_eq!(label(0), "Mon 16:00");
        assert_eq!(label(5 * MINUTE_MS + 59_999), "Mon 16:05");
        // Eight hours on: day 1, a Tuesday, at midnight.
        assert_eq!(at(1, 0), 8 * 60 * MINUTE_MS);
        assert_eq!(split(at(1, 0)), (1, 0));
        assert_eq!(split(at(9, 8 * 60 + 15) + 59_999), (9, 8 * 60 + 15));
        assert_eq!(weekday(1), Weekday::Tue);
        assert_eq!(weekday(6), Weekday::Sun);
        assert_eq!(weekday(7), Weekday::Mon);
    }

    /// What the stage and the explain line show: the vacation flag too.
    #[test]
    fn a_day_time_reads_as_the_stage_shows_it() {
        assert_eq!(day_time(0, false).to_string(), "Mon 16:00 Afternoon");
        assert_eq!(
            day_time(at(1, 9 * 60), true).to_string(),
            "Tue 09:00 Morning (vacation)"
        );
        assert_eq!(day_time(at(1, 9 * 60), false).to_string(), "Tue 09:00 Away");
    }

    /// The routine tape over a game week, in every flag pattern: every
    /// slot in its place, contiguous, with the table's bed and wake
    /// times, each day judged by its own flag.
    #[test]
    fn the_routine_tape() {
        for (name, flag) in PATTERNS {
            let tape = tape(flag);
            let seen: std::collections::HashSet<Slot> = tape.iter().map(|t| t.2).collect();
            if name == "vacation" {
                // No school at all: no Away, no Homework.
                assert!(!seen.contains(&Slot::Away));
                assert!(!seen.contains(&Slot::Homework));
                assert_eq!(seen.len(), 4, "{seen:?}");
            } else {
                assert_eq!(
                    seen.len(),
                    Slot::ALL.len(),
                    "{name}: every slot occurs: {seen:?}"
                );
            }
            for &(day, minute, slot) in &tape {
                let weekend = matches!(weekday(day), Weekday::Sat | Weekday::Sun);
                let school = !flag(day) && !weekend;
                // Weekends and vacations have no school; school days have
                // it exactly 08:15-12:45.
                assert_eq!(
                    slot == Slot::Away,
                    school && (hm(8, 15)..hm(12, 45)).contains(&minute),
                    "{name}: day {day} {minute}"
                );
                // Homework only on school nights (Sun-Thu evenings, by the
                // day's own flag), from 20:00.
                if slot == Slot::Homework {
                    assert!(!flag(day), "{name}: day {day}");
                    assert!(
                        !matches!(weekday(day), Weekday::Fri | Weekday::Sat),
                        "{name}: homework on {:?}",
                        weekday(day)
                    );
                    assert!(minute >= hm(20, 0));
                }
            }
            for day in TAPE.0 / DAY_MIN..TAPE.1 / DAY_MIN {
                let vacation = flag(day);
                let slots: Vec<Slot> = (0..DAY_MIN as u16)
                    .map(|minute| slot(day, minute, vacation))
                    .collect();
                let school = school_day(day, vacation);
                let tonight = night_before_school(day, vacation);
                assert_eq!(
                    school,
                    !vacation && !matches!(weekday(day), Weekday::Sat | Weekday::Sun)
                );
                // School nights are Sun-Thu (by the day's own flag).
                assert_eq!(
                    tonight,
                    !vacation && !matches!(weekday(day), Weekday::Fri | Weekday::Sat),
                    "{name}: day {day}"
                );
                // Contiguous: the day runs through its slots in order,
                // each once, the night wrapping across midnight.
                let mut runs = vec![slots[0]];
                for &s in &slots[1..] {
                    if runs.last() != Some(&s) {
                        runs.push(s);
                    }
                }
                let mut want = vec![Slot::Asleep, Slot::Morning];
                if school {
                    want.push(Slot::Away);
                }
                want.extend([Slot::Afternoon, Slot::Evening]);
                if tonight {
                    want.push(Slot::Homework);
                }
                want.push(Slot::Asleep);
                assert_eq!(runs, want, "{name}: day {day}");
                // Bed and wake times.
                let wake = if school { hm(7, 0) } else { hm(9, 0) };
                let bed = if tonight { hm(22, 30) } else { hm(23, 30) };
                assert_eq!(slots[usize::from(wake) - 1], Slot::Asleep);
                assert_ne!(slots[usize::from(wake)], Slot::Asleep);
                assert_ne!(slots[usize::from(bed) - 1], Slot::Asleep);
                assert_eq!(slots[usize::from(bed)], Slot::Asleep);
            }
            // Nights are computed, not assumed. With one flag all week the
            // longest is 9.5 game hours (95 real minutes at 6x), after a
            // Friday or Saturday in a school week and after every day in
            // a vacation one. Where a vacation starts after a school night
            // (bed at 22:30 by that day's flag, the lie-in until 09:00 by
            // the next's) it is 10.5 game hours: 105 real minutes.
            let nights: Vec<u64> = runs(&tape)
                .into_iter()
                .filter(|&(s, from, len)| {
                    // Whole nights only: begun and ended on the tape.
                    s == Slot::Asleep && from > TAPE.0 && from + len < TAPE.1
                })
                .map(|(_, _, len)| len)
                .collect();
            assert_eq!(nights.len(), 6, "{name}: {nights:?}");
            let longest = nights.iter().copied().max().unwrap();
            let want = match name {
                "vacation from Wednesday" | "every other day" => 10 * 60 + 30,
                _ => 9 * 60 + 30,
            };
            assert_eq!(longest, want, "{name}: {nights:?}");
            match name {
                "school" => {
                    assert_eq!(longest * 60 / super::super::CLOCK_SPEED, 95 * 60);
                    // Mon-Thu nights on the tape: 22:30 to 07:00.
                    assert_eq!(nights.iter().filter(|&&n| n == 8 * 60 + 30).count(), 4);
                }
                "vacation from Wednesday" => {
                    assert_eq!(longest * 60 / super::super::CLOCK_SPEED, 105 * 60);
                    // Tuesday's: a school night's bedtime, a vacation's wake.
                    assert_eq!(nights[1], 10 * 60 + 30, "{nights:?}");
                }
                "vacation until Tuesday" => {
                    // Tuesday's: a vacation's bedtime, a school day's wake.
                    assert_eq!(nights[1], 7 * 60 + 30, "{nights:?}");
                }
                _ => {}
            }
        }
    }

    /// The next boundary is the next minute the slot changes, found by
    /// brute force on the tape, in every flag pattern (each day by its
    /// own flag): the boundary table misses none, and a boundary
    /// tomorrow is judged by tomorrow's flag.
    #[test]
    fn next_boundaries_match_the_tape() {
        for (name, flag) in PATTERNS {
            let at_abs = |abs: u64| {
                let (day, minute) = (abs / DAY_MIN, (abs % DAY_MIN) as u16);
                (minute, slot(day, minute, flag(day)))
            };
            let mut changes = Vec::new();
            for abs in START + 1..START + 16 * DAY_MIN {
                let (minute, begins) = at_abs(abs);
                if begins != at_abs(abs - 1).1 {
                    assert!(BOUNDARIES.contains(&minute), "{name}: a change at {minute}");
                    changes.push(((abs - START) * MINUTE_MS, begins));
                }
            }
            // From every minute, and between minutes, of two weeks.
            for abs in START..START + 14 * DAY_MIN {
                for game in [(abs - START) * MINUTE_MS, (abs - START) * MINUTE_MS + 1] {
                    let want = changes.iter().find(|&&(t, _)| t > game).unwrap();
                    assert_eq!(next_boundary(game, flag), want.0, "{name}: at {game}");
                    let cut = changes
                        .iter()
                        .find(|&&(t, s)| t > game && s.cuts())
                        .unwrap();
                    assert_eq!(next_cutting(game, flag), cut.0, "{name}: at {game}");
                }
            }
        }
    }

    /// A clock judges each day by its own flag: its latch's day by the
    /// latch, the day before by the eve's, and a later day by the date
    /// as it reads now, so tomorrow's wake and school are predicted from
    /// the coming latch, not today's.
    #[test]
    fn a_clock_judges_each_day_by_its_own_flag() {
        let summer = Some(date(2026, 7, 25));
        let june = Some(date(2026, 6, 17));
        let clock = |game: u64, held: Option<Latch>, date| {
            Clock::read(GameClock { at: 0, game }, 0, held, date)
        };
        // Tuesday 23:00, latched a school day; the date has since turned
        // to summer.
        let tue = Latch::at(None, 1, june);
        let late = clock(at(1, 23 * 60), Some(tue), summer);
        assert!(!late.vacation_on(1) && late.vacation_on(2));
        // Bed came at 22:30 by Tuesday's flag; Wednesday lies in to 09:00
        // and has no school.
        assert_eq!(late.day(0).slot, Slot::Asleep);
        assert_eq!(late.next_boundary(at(1, 23 * 60)), at(2, 9 * 60));
        assert_eq!(late.next_cutting(at(1, 23 * 60)), at(2, 23 * 60 + 30));
        // The reverse: a vacation Tuesday before a school Wednesday.
        let tue = Latch::at(None, 1, summer);
        let late = clock(at(1, 23 * 60 + 45), Some(tue), june);
        assert_eq!(late.next_boundary(at(1, 23 * 60 + 45)), at(2, 7 * 60));
        assert_eq!(late.next_cutting(at(1, 23 * 60 + 45)), at(2, 8 * 60 + 15));
        // Just past midnight the new day latches; a catch-up decision
        // in the minute before it is judged by the eve's flag.
        let wed = Latch::at(Some(tue), 2, june);
        assert_eq!(wed.eve, Some(true));
        let read = GameClock {
            at: 60_000,
            game: at(2, 1),
        };
        let early = Clock::read(read, 60_000, Some(wed), june);
        // Two game minutes back: Tuesday 23:59.
        let eve = early.day(60_000 - 20_000);
        assert_eq!((eve.day, eve.minute), (1, 23 * 60 + 59));
        assert!(eve.vacation && !eve.school_day);
        assert!(!early.day(60_000).vacation);
        // An eve that wasn't held (a skip, a restart): the latch's.
        let fresh = clock(at(2, 1), None, june);
        assert_eq!(fresh.latch.eve, None);
        assert!(!fresh.vacation_on(1));
    }

    #[test]
    fn the_work_window() {
        // Saturday.
        assert!(!work_open(5, hm(9, 59), false));
        assert!(work_open(5, hm(10, 0), false));
        assert!(work_open(5, hm(16, 59), false));
        assert!(!work_open(5, hm(17, 0), false));
        // A school Monday never; a vacation Monday does.
        assert!(!work_open(7, hm(12, 0), false));
        assert!(work_open(7, hm(12, 0), true));
    }

    #[test]
    fn vacations_and_panic_week_on_real_dates() {
        for year in [2024, 2025, 2026, 2028, 2100] {
            let on = |m, d| vacation(date(year, m, d));
            // Summer.
            assert!(!on(7, 19));
            assert!(on(7, 20));
            assert!(on(8, 31));
            assert!(!on(9, 1));
            // The year's end, over the new year.
            assert!(!on(12, 24));
            assert!(on(12, 25));
            assert!(on(12, 31));
            assert!(on(1, 1));
            assert!(on(1, 7));
            assert!(!on(1, 8));
            // Spring.
            assert!(!on(3, 24));
            assert!(on(3, 25));
            assert!(on(4, 5));
            assert!(!on(4, 6));
            // Ordinary days.
            assert!(!on(2, 28));
            assert!(!on(6, 17));
            assert!(!on(10, 31));
            // Panic week.
            let panic = |m, d| panic_week(date(year, m, d));
            assert!(!panic(8, 24));
            assert!(panic(8, 25));
            assert!(panic(8, 31));
            assert!(!panic(9, 1));
            assert!(!panic(7, 25));
        }
        // Leap days are school days.
        for year in [2024, 2028, 2000] {
            assert!(!vacation(date(year, 2, 29)));
            assert!(!panic_week(date(year, 2, 29)));
        }
        // Over a whole leap year: exactly the windows' days.
        let days = (0..366)
            .map(|i| date(2024, 1, 1) + chrono::Duration::days(i))
            .filter(|&d| vacation(d))
            .count();
        assert_eq!(days, 43 + 14 + 12);
    }

    #[test]
    fn a_latch_holds_its_day() {
        let summer = Some(date(2026, 7, 25));
        let june = Some(date(2026, 6, 17));
        let held = Latch::at(None, 3, summer);
        assert!(held.vacation);
        assert_eq!(held.eve, None, "no day before was held");
        // The date flips: the same day keeps its flag.
        assert_eq!(Latch::at(Some(held), 3, june), held);
        // The next day reads it afresh.
        assert_eq!(
            Latch::at(Some(held), 4, june),
            Latch {
                day: 4,
                vacation: false,
                eve: Some(true),
            }
        );
        // A day further on holds no eve.
        assert_eq!(Latch::at(Some(held), 5, june).eve, None);
        // No calendar: no vacation.
        assert!(!Latch::at(None, 3, None).vacation);
    }

    /// A set of slots holds exactly its own.
    #[test]
    fn a_slot_set_holds_its_slots() {
        const SET: SlotSet = SlotSet::of(&[Slot::Morning, Slot::Homework]);
        for slot in Slot::ALL {
            assert_eq!(
                SET.contains(slot),
                matches!(slot, Slot::Morning | Slot::Homework),
                "{slot:?}"
            );
            assert!(!SlotSet::of(&[]).contains(slot));
            assert!(SlotSet::of(&Slot::ALL).contains(slot));
        }
    }

    /// When a boost holds: its slots; the evening before a day off (the
    /// reading evening, never a school night's, never past bedtime);
    /// game hours, over midnight too.
    #[test]
    fn when_holds_at_its_times() {
        let flags: [fn(u64) -> bool; 2] = [|_| false, |_| true];
        for flag in flags {
            for abs in TAPE.0..TAPE.1 {
                let (day, minute) = (abs / DAY_MIN, (abs % DAY_MIN) as u16);
                let now = day_time(at(day, u64::from(minute)), flag(day));
                let evening = When::In(SlotSet::of(&[Slot::Evening]));
                assert_eq!(evening.holds(&now), now.slot == Slot::Evening);
                let off = When::EveningOff.holds(&now);
                assert_eq!(
                    off,
                    now.slot == Slot::Evening && !night_before_school(day, flag(day)),
                    "{now}"
                );
                if off {
                    assert!((EVENING..BED_OFF).contains(&minute), "{now}");
                }
                assert_eq!(
                    DUSK_TO_DAWN.holds(&now),
                    !(hm(5, 0)..hm(17, 0)).contains(&minute),
                    "{now}"
                );
                assert_eq!(
                    When::Hours(hm(9, 0), hm(10, 0)).holds(&now),
                    (hm(9, 0)..hm(10, 0)).contains(&minute)
                );
            }
        }
        // A school week's evenings off: Friday's and Saturday's.
        let off: Vec<Weekday> = (7..14)
            .filter(|&day| When::EveningOff.holds(&day_time(at(day, 21 * 60), false)))
            .map(weekday)
            .collect();
        assert_eq!(off, [Weekday::Fri, Weekday::Sat]);
    }

    /// The clock's inverse: the earliest moment it reads a game time or
    /// later, ahead of the reading or behind it (rounded up, never below
    /// zero).
    #[test]
    fn when_is_the_clocks_inverse() {
        for (at, game) in [(0, 0), (1_000, 0), (5_000, 7), (100_000, 3_000_001)] {
            let clock = GameClock { at, game };
            for target in (0..game + 1_000_000)
                .step_by(997)
                .chain([game, game + 6, game + 7])
            {
                let t = clock.when(target);
                assert!(clock.at(t) >= target, "{clock:?} {target}: {t}");
                // (Earliest above zero: below her start the clock reads 0.)
                if t > 0 && target > 0 {
                    assert!(clock.at(t - 1) < target, "{clock:?} {target}: {t}");
                }
            }
        }
    }

    /// Exam season, hay fever and panic week fall on their dates.
    #[test]
    fn the_seasons_fall_on_their_dates() {
        let days = |window: DateWindow| -> Vec<(u32, u32)> {
            (0..366)
                .map(|i| date(2028, 1, 1) + chrono::Duration::days(i))
                .filter(|&d| within(d, window))
                .map(|d| (d.month(), d.day()))
                .collect()
        };
        let exams = days(EXAMS);
        assert_eq!(exams.first(), Some(&(1, 20)));
        assert_eq!(exams.last(), Some(&(3, 10)));
        assert_eq!(exams.len(), 12 + 29 + 10);
        let hay = days(HAY_FEVER);
        assert_eq!(
            (hay.first(), hay.last(), hay.len()),
            (Some(&(3, 1)), Some(&(4, 30)), 61)
        );
        assert_eq!(days(PANIC_WEEK).len(), 7);
    }
}
