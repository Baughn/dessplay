//! Re-validation scheduling for the AniDB lookup queue (pure functions;
//! the rules live in docs/design.md, "Parsing files to series/season/
//! episode").
//!
//! All times are shared-clock unix milliseconds, caller-supplied — this
//! module never reads a clock, which keeps it trivially testable and
//! the queue deterministic.

/// Milliseconds per minute/hour/day/week, for the ladder below.
const MINUTE: i64 = 60 * 1000;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;

/// Wait after a missing response before retrying; server throttling is
/// unpredictable and a retry storm reads as flooding.
pub const TIMEOUT_RETRY_MILLIS: i64 = 5_000;

/// What an attempt produced, as far as scheduling cares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// AniDB returned data for the file.
    Data,
    /// AniDB answered, and doesn't know the file (320 NO SUCH FILE).
    NoData,
    /// No response arrived in time.
    Timeout,
}

/// The age anchor for the unknown-file ladder: the *older* of when we
/// first saw the file (`first_seen`) and the file's own mtime, when the
/// client supplied one.
///
/// Using the minimum is the safe choice. It survives a missing mtime
/// (falls back to `first_seen`), a `first_seen` reset after a queue wipe
/// (mtime keeps a long-owned file looking old, so it isn't re-polled on
/// the aggressive new-file cadence), and a touched file (`first_seen`
/// keeps it from looking newer than it really is to us).
pub fn effective_anchor(first_seen: i64, mtime: Option<i64>) -> i64 {
    mtime.map_or(first_seen, |m| first_seen.min(m))
}

/// When to try this queue entry again. `None` means never — the entry
/// can be dropped from the queue.
///
/// The ladder for unknown files stretches with the entry's age (since
/// `anchor`, see [`effective_anchor`]): new files often appear on AniDB
/// within hours, old ones almost never do. Files AniDB *does* know are
/// re-validated weekly (the design's hard cap of once per week).
pub fn next_attempt(now: i64, anchor: i64, has_data: bool, outcome: Outcome) -> Option<i64> {
    match outcome {
        Outcome::Timeout => Some(now + TIMEOUT_RETRY_MILLIS),
        Outcome::Data => Some(now + WEEK),
        Outcome::NoData if has_data => Some(now + WEEK),
        Outcome::NoData => {
            let age = now.saturating_sub(anchor);
            let interval = match age {
                _ if age < DAY => 30 * MINUTE,
                _ if age < WEEK => 2 * HOUR,
                _ if age < 30 * DAY => 12 * HOUR,
                _ if age < 90 * DAY => 3 * DAY,
                _ => return None,
            };
            Some(now + interval)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn fresh_unknown_files_retry_every_half_hour() {
        assert_eq!(
            next_attempt(1000, 0, false, Outcome::NoData),
            Some(1000 + 30 * MINUTE)
        );
    }

    #[test]
    fn the_ladder_stretches_with_age() {
        let day_old = 2 * DAY;
        assert_eq!(
            next_attempt(day_old, 0, false, Outcome::NoData),
            Some(day_old + 2 * HOUR)
        );
        let week_old = 8 * DAY;
        assert_eq!(
            next_attempt(week_old, 0, false, Outcome::NoData),
            Some(week_old + 12 * HOUR)
        );
        let month_old = 40 * DAY;
        assert_eq!(
            next_attempt(month_old, 0, false, Outcome::NoData),
            Some(month_old + 3 * DAY)
        );
    }

    #[test]
    fn unknown_files_older_than_three_months_stop() {
        assert_eq!(next_attempt(91 * DAY, 0, false, Outcome::NoData), None);
    }

    #[test]
    fn known_files_revalidate_weekly_at_most() {
        assert_eq!(
            next_attempt(1000, 0, false, Outcome::Data),
            Some(1000 + WEEK)
        );
        // A file that had data once but is now missing keeps the weekly
        // cadence — has_data sticks.
        assert_eq!(
            next_attempt(91 * DAY, 0, true, Outcome::NoData),
            Some(91 * DAY + WEEK)
        );
    }

    #[test]
    fn timeouts_retry_after_the_penalty_wait() {
        assert_eq!(
            next_attempt(1000, 0, false, Outcome::Timeout),
            Some(1000 + TIMEOUT_RETRY_MILLIS)
        );
        // Age doesn't matter for timeouts; the server never answered.
        assert_eq!(
            next_attempt(100 * DAY, 0, false, Outcome::Timeout),
            Some(100 * DAY + TIMEOUT_RETRY_MILLIS)
        );
    }

    #[test]
    fn boundaries_are_exact() {
        // At exactly one day, the 2h band applies.
        assert_eq!(
            next_attempt(DAY, 0, false, Outcome::NoData),
            Some(DAY + 2 * HOUR)
        );
        // At exactly 90 days, re-validation stops.
        assert_eq!(next_attempt(90 * DAY, 0, false, Outcome::NoData), None);
    }

    #[test]
    fn effective_anchor_is_the_minimum() {
        // No mtime: fall back to first_seen (today's behaviour).
        assert_eq!(effective_anchor(1000, None), 1000);
        // mtime older than first_seen wins (the file is old to the world).
        assert_eq!(effective_anchor(1000, Some(10)), 10);
        // mtime newer than first_seen: first_seen still keeps it "old" to us.
        assert_eq!(effective_anchor(10, Some(1000)), 10);
    }

    /// Regression: a file we only just enqueued (`first_seen = now`) but
    /// have owned for 200 days (its mtime) must NOT be polled on the
    /// aggressive new-file ladder — the mtime anchors it past the 90-day
    /// cutoff, so it's never re-validated. Before the mtime anchor, a
    /// queue reset made every long-owned unknown file look brand-new and
    /// got re-polled every 30 min forever.
    #[test]
    fn old_mtime_file_never_revalidates_despite_recent_first_seen() {
        let now = 1_000 * DAY;
        let first_seen = now; // just enqueued (e.g. after a DB wipe)
        let mtime = now - 200 * DAY; // but owned for 200 days
        let anchor = effective_anchor(first_seen, Some(mtime));
        assert_eq!(next_attempt(now, anchor, false, Outcome::NoData), None);
        // Sanity: without the mtime it would be back on the 30-min ladder.
        assert_eq!(
            next_attempt(
                now,
                effective_anchor(first_seen, None),
                false,
                Outcome::NoData
            ),
            Some(now + 30 * MINUTE)
        );
    }

    /// Property (deterministic grid): the unknown-file ladder is governed
    /// by `now - min(first_seen, mtime)` — the older timestamp's age —
    /// checked against the design's ladder table restated as an
    /// independent oracle (which never calls `effective_anchor` or
    /// `next_attempt`, so the two derivations can genuinely disagree).
    #[test]
    fn ladder_is_governed_by_the_older_timestamp() {
        let now = 1_000 * DAY;
        for fs_age in [0, DAY, 8 * DAY, 40 * DAY, 89 * DAY, 90 * DAY, 200 * DAY] {
            for mt_age in [0, DAY, 8 * DAY, 40 * DAY, 89 * DAY, 90 * DAY, 200 * DAY] {
                let anchor = effective_anchor(now - fs_age, Some(now - mt_age));
                let got = next_attempt(now, anchor, false, Outcome::NoData);
                let age = fs_age.max(mt_age);
                let expected = if age < DAY {
                    Some(now + 30 * MINUTE)
                } else if age < WEEK {
                    Some(now + 2 * HOUR)
                } else if age < 30 * DAY {
                    Some(now + 12 * HOUR)
                } else if age < 90 * DAY {
                    Some(now + 3 * DAY)
                } else {
                    None
                };
                assert_eq!(got, expected, "fs_age={fs_age} mt_age={mt_age}");
            }
        }
    }
}
