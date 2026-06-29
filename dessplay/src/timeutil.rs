//! Small, UI-free time helpers shared between the chat pane's day
//! separators (`ui::props`) and daily log rotation (`logging`). Kept out
//! of `ui` so non-UI code can depend on the "biblical day" boundary
//! without pulling in the UI module.

/// The "biblical" calendar day for `millis`: the local date after
/// shifting the boundary to 09:00 (the small hours belong to the prior
/// evening's session — design.md, System Messages). Two timestamps are
/// the same day iff this is equal. `None` for an out-of-range timestamp.
pub fn biblical_date(millis: u64) -> Option<chrono::NaiveDate> {
    use chrono::{Local, TimeZone};
    let dt = Local.timestamp_millis_opt(millis as i64).single()?;
    Some((dt - chrono::Duration::hours(9)).date_naive())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use chrono::{Local, NaiveDate, TimeZone};

    fn local_millis(y: i32, m: u32, d: u32, hour: u32, min: u32) -> u64 {
        Local
            .with_ymd_and_hms(y, m, d, hour, min, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
    }

    #[test]
    fn boundary_is_local_0900() {
        // 08:00 local belongs to the previous biblical day; 10:00 to the
        // current one. TZ-independent: we build the local times and check
        // the shifted date.
        let eight = local_millis(2026, 6, 18, 8, 0);
        let ten = local_millis(2026, 6, 18, 10, 0);
        assert_eq!(biblical_date(eight), NaiveDate::from_ymd_opt(2026, 6, 17));
        assert_eq!(biblical_date(ten), NaiveDate::from_ymd_opt(2026, 6, 18));
    }

    #[test]
    fn late_night_belongs_to_prior_session() {
        // 23:00 and the following 02:00 are the same biblical day; 10:00
        // the next morning starts a new one.
        let evening = local_millis(2026, 6, 18, 23, 0);
        let small_hours = local_millis(2026, 6, 19, 2, 0);
        let next_morning = local_millis(2026, 6, 19, 10, 0);
        assert_eq!(biblical_date(evening), biblical_date(small_hours));
        assert_ne!(biblical_date(evening), biblical_date(next_morning));
    }
}
