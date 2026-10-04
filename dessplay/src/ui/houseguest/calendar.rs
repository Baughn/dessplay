//! Her calendar: what the real date brings her (phase 5b D5, with A23).
//! Pure: every answer is a function of the date alone.
//!
//! Two kinds of entry:
//! - **Owed**: at most one a day, the most specific owed window winning
//!   (New Year's Day over the New Year). Inside a season it stacks with
//!   the season's tints (Setsubun in exam season, Christmas in
//!   December). It's owed once a day: she delivers it on her first visit
//!   that day where it shows, and the date it was delivered on is kept
//!   in her ledger (see `Osaka::calendar_owed`).
//! - **Tints**: boosts and line pools that stack, every day of their
//!   season (exam season, hay fever, December, summer's panic week).
//!
//! The date is the real one, as `timeutil::biblical_date` reads it (the
//! day starts at 09:00); never a time of day. Without a date, nothing.

use chrono::{Datelike, NaiveDate};

use super::routine::{self, DateWindow};

/// "Happy New Year!": her greeting through the New Year (Jan 1-3).
pub(super) const HAPPY_NEW_YEAR: &str = line!("Happy New Year!");
/// Apr 8: the day she first came on.
pub(super) const DEBUT: &str = line!("It's my debut day!");
/// Jul 7, Tanabata.
pub(super) const TANABATA: &str = line!("Wrote my wish. Secret!");
/// Sep 30, the finale.
pub(super) const GRADUATED: &str = line!("We graduated, huh...");
/// Oct 31.
pub(super) const HALLOWEEN: &str = line!("Trick or treat!");
/// Dec 24-25.
pub(super) const CHRISTMAS: &str = line!("Merry Christmas!");
/// Dec 31.
pub(super) const YEAR_END: &str = line!("Year's almost over...");

/// Every greeting a date owes (the pool lint lists them).
#[cfg(test)]
pub(super) const GREETINGS: [&str; 7] = [
    HAPPY_NEW_YEAR,
    DEBUT,
    TANABATA,
    GRADUATED,
    HALLOWEEN,
    CHRISTMAS,
    YEAR_END,
];

/// What a date owes her, once that day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Owed {
    /// A greeting, said in place of her mood's (or her wake line), or on
    /// its own as she spaces out.
    Greet(&'static str),
    /// New Year's Day: "Happy New Year!", delivered as it shows; then,
    /// that visit, the first sunrise on her TV if she can get to it
    /// ("Ooh... first sunrise."), best effort.
    FirstSunrise,
    /// Setsubun (Feb 3): beans thrown on the spot, "Oni wa soto!", then
    /// "Fuku wa uchi!".
    Setsubun,
}

impl Owed {
    /// The greeting it brings, if any.
    pub fn greeting(self) -> Option<&'static str> {
        match self {
            Self::Greet(line) => Some(line),
            Self::FirstSunrise => Some(HAPPY_NEW_YEAR),
            Self::Setsubun => None,
        }
    }
}

/// The seasons a date is in: each tints what she does, and they stack.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Tints {
    /// Exam season (Jan 20 - Mar 10): her chopsticks before every
    /// homework (her homework's boost is the brain's, on the same
    /// window: [`routine::EXAMS`]).
    pub exams: bool,
    /// December: her seasonal musings.
    pub december: bool,
    /// The New Year (Jan 1-3): a dream joins her sleep-talk.
    pub new_year: bool,
    /// Summer's last week (Aug 25-31): "Homework! Homework!" (her
    /// homework's boost is the brain's, on [`routine::PANIC_WEEK`]).
    pub panic: bool,
}

/// The New Year.
const NEW_YEAR: DateWindow = ((1, 1), (1, 3));

/// One day's window.
const fn on(month: u32, day: u32) -> DateWindow {
    ((month, day), (month, day))
}

/// What each window owes, most specific first: the first that holds is
/// the day's (a lint holds the order to its windows' lengths).
const OWED: [(DateWindow, Owed); 9] = [
    (on(1, 1), Owed::FirstSunrise),
    (NEW_YEAR, Owed::Greet(HAPPY_NEW_YEAR)),
    (on(2, 3), Owed::Setsubun),
    (on(4, 8), Owed::Greet(DEBUT)),
    (on(7, 7), Owed::Greet(TANABATA)),
    (on(9, 30), Owed::Greet(GRADUATED)),
    (on(10, 31), Owed::Greet(HALLOWEEN)),
    (((12, 24), (12, 25)), Owed::Greet(CHRISTMAS)),
    (on(12, 31), Owed::Greet(YEAR_END)),
];

/// What `date` owes her (at most one thing) and the seasons it's in.
pub(super) fn entries(date: NaiveDate) -> (Option<Owed>, Tints) {
    (owed(date), tints(date))
}

/// What `date` owes her: the most specific window's.
fn owed(date: NaiveDate) -> Option<Owed> {
    OWED.iter()
        .find(|&&(window, _)| routine::within(date, window))
        .map(|&(_, owed)| owed)
}

/// The seasons `date` is in.
fn tints(date: NaiveDate) -> Tints {
    Tints {
        exams: routine::within(date, routine::EXAMS),
        december: date.month() == 12,
        new_year: routine::within(date, NEW_YEAR),
        panic: routine::panic_week(date),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// Every date from 2024 through 2040.
    fn every_date() -> impl Iterator<Item = NaiveDate> {
        let (first, last) = (date(2024, 1, 1), date(2040, 12, 31));
        first.iter_days().take_while(move |&d| d <= last)
    }

    /// Days a window spans in a leap year (its specificity).
    fn span(window: DateWindow) -> usize {
        date(2028, 1, 1)
            .iter_days()
            .take(366)
            .filter(|&d| routine::within(d, window))
            .count()
    }

    /// Over every date of 2024-2040: at most one owed entry a day (an
    /// `Option`, by construction; and no two windows of one specificity
    /// overlap, so "the most specific" is never a tie), the same on
    /// every call; each starter date on exactly its days, and the
    /// tints on theirs.
    #[test]
    fn the_calendar_over_2024_to_2040() {
        let mut years_seen = 0;
        for d in every_date() {
            let (owed, tints) = entries(d);
            assert_eq!(entries(d), (owed, tints), "{d}: stable");
            let md = (d.month(), d.day());
            let want = match md {
                (1, 1) => Some(Owed::FirstSunrise),
                (1, 2..=3) => Some(Owed::Greet(HAPPY_NEW_YEAR)),
                (2, 3) => Some(Owed::Setsubun),
                (4, 8) => Some(Owed::Greet(DEBUT)),
                (7, 7) => Some(Owed::Greet(TANABATA)),
                (9, 30) => Some(Owed::Greet(GRADUATED)),
                (10, 31) => Some(Owed::Greet(HALLOWEEN)),
                (12, 24..=25) => Some(Owed::Greet(CHRISTMAS)),
                (12, 31) => Some(Owed::Greet(YEAR_END)),
                _ => None,
            };
            assert_eq!(owed, want, "{d}");
            // Every window that holds that day, beyond the one that won,
            // is wider: the most specific won.
            let holding: Vec<usize> = OWED
                .iter()
                .filter(|&&(w, _)| routine::within(d, w))
                .map(|&(w, _)| span(w))
                .collect();
            if let Some((&won, rest)) = holding.split_first() {
                assert!(rest.iter().all(|&s| s > won), "{d}: {holding:?}");
            }
            let exams = ((1, 20)..=(3, 10)).contains(&md);
            let want = Tints {
                exams,
                december: d.month() == 12,
                new_year: md <= (1, 3),
                panic: ((8, 25)..=(8, 31)).contains(&md),
            };
            assert_eq!(tints, want, "{d}");
            years_seen += usize::from(md == (1, 1));
        }
        assert_eq!(years_seen, 17);
    }

    /// The precedence pairs: an owed entry inside a season keeps the
    /// season's tint (they stack); within the owed, the narrower wins.
    #[test]
    fn the_most_specific_window_wins() {
        for year in [2024, 2027, 2040] {
            let (owed, tints) = entries(date(year, 2, 3));
            assert_eq!(owed, Some(Owed::Setsubun));
            assert!(tints.exams);
            let (owed, _) = entries(date(year, 4, 8));
            assert_eq!(owed, Some(Owed::Greet(DEBUT)));
            assert!(routine::within(date(year, 4, 8), routine::HAY_FEVER));
            for day in [24, 25] {
                let (owed, tints) = entries(date(year, 12, day));
                assert_eq!(owed, Some(Owed::Greet(CHRISTMAS)));
                assert!(tints.december);
            }
            let (owed, tints) = entries(date(year, 12, 31));
            assert_eq!(owed, Some(Owed::Greet(YEAR_END)));
            assert!(tints.december);
            let (owed, tints) = entries(date(year, 1, 1));
            assert_eq!(owed, Some(Owed::FirstSunrise));
            assert_eq!(owed.and_then(Owed::greeting), Some(HAPPY_NEW_YEAR));
            assert!(tints.new_year);
            let (owed, tints) = entries(date(year, 1, 4));
            assert_eq!((owed, tints.new_year), (None, false));
        }
    }

    /// Lint: the windows are listed most specific first, so the first
    /// that holds is the most specific; and every greeting is listed.
    #[test]
    fn the_windows_are_listed_most_specific_first() {
        for (i, &(a, _)) in OWED.iter().enumerate() {
            for &(b, _) in &OWED[i + 1..] {
                let overlap = date(2028, 1, 1)
                    .iter_days()
                    .take(366)
                    .any(|d| routine::within(d, a) && routine::within(d, b));
                if overlap {
                    assert!(span(a) < span(b), "{a:?} before {b:?}");
                }
            }
        }
        for (_, owed) in OWED {
            if let Some(line) = owed.greeting() {
                assert!(GREETINGS.contains(&line), "{line}");
            }
        }
    }
}
