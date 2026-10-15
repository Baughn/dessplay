//! Throttling of failed password attempts, per source address.
//!
//! Each address has a budget of `burst` failures that refills at one
//! per `refill`. A failure spends one; a correct password spends
//! nothing. Once the budget is spent, the address's attempts are
//! refused without the password being looked at, so a guess made while
//! throttled learns nothing — not even when it happens to be right.
//!
//! The budget is kept as a GCRA "theoretical arrival time": one
//! `Instant` per address, the moment its budget will be full again.
//! An address whose moment has passed carries no information and is
//! dropped.
//!
//! IPv6 addresses are grouped by their /64 — one subscriber's
//! allocation — so rotating through it buys no extra guesses.
//! IPv4-mapped IPv6 addresses count as the IPv4 address they carry.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::time::Duration;

use tokio::time::Instant;

/// Failed-auth budget per source address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthLimit {
    /// Failures an address may make back to back. At least 1 is used
    /// even if 0 is given (0 would refuse every attempt forever).
    pub burst: u32,
    /// Time for one spent failure to come back.
    pub refill: Duration,
}

impl Default for AuthLimit {
    /// Five failures, then one a minute.
    fn default() -> Self {
        Self {
            burst: 5,
            refill: Duration::from_secs(60),
        }
    }
}

/// The outcome of one password attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The password was right and the address was not throttled.
    Accept,
    /// The password was wrong; one failure was spent.
    Reject,
    /// The address's budget is spent; the password was not judged.
    Throttled,
}

/// Addresses tracked before refilled entries are swept out.
const SWEEP_AT: usize = 1024;
/// Hard cap on tracked addresses. Past it (after a sweep), a new
/// address goes untracked: an attacker holding that many addresses is
/// the distributed case this limiter does not try to stop, and refusing
/// unknown addresses instead would lock everyone else out.
const MAX_TRACKED: usize = 65_536;

/// Per-address failed-auth bookkeeping. See the module docs.
#[derive(Debug)]
pub struct AuthLimiter {
    limit: AuthLimit,
    /// Per address key: when its budget is full again.
    full_at: HashMap<IpAddr, Instant>,
}

impl AuthLimiter {
    /// An empty limiter with the given budget.
    pub fn new(limit: AuthLimit) -> Self {
        Self {
            limit,
            full_at: HashMap::new(),
        }
    }

    /// Judge an attempt from `ip` at `now`; `correct` checks its
    /// password, and is not called while the address is throttled.
    /// Atomic: the throttle check and the spend happen in one call, so
    /// concurrent attempts from one address cannot all slip past a
    /// nearly-spent budget.
    pub fn judge(&mut self, ip: IpAddr, now: Instant, correct: impl FnOnce() -> bool) -> Verdict {
        let key = address_key(ip);
        let full_at = self.full_at.get(&key).copied();
        let debt = full_at.map_or(Duration::ZERO, |at| at.saturating_duration_since(now));
        let capacity = self.limit.refill * self.limit.burst.max(1);
        if debt + self.limit.refill > capacity {
            return Verdict::Throttled;
        }
        if correct() {
            return Verdict::Accept;
        }
        let new_full_at = now + debt + self.limit.refill;
        if full_at.is_none() && !self.make_room(now) {
            tracing::warn!(%ip, "auth limiter full; not tracking this address");
            return Verdict::Reject;
        }
        self.full_at.insert(key, new_full_at);
        Verdict::Reject
    }

    /// Make room for one more address; false if the table is at its cap
    /// even after sweeping out the refilled ones.
    fn make_room(&mut self, now: Instant) -> bool {
        if self.full_at.len() >= SWEEP_AT {
            self.full_at.retain(|_, full_at| *full_at > now);
        }
        self.full_at.len() < MAX_TRACKED
    }

    #[cfg(test)]
    fn tracked(&self) -> usize {
        self.full_at.len()
    }
}

/// The key an address's budget is kept under: IPv4 as is (including
/// IPv4-mapped IPv6), IPv6 by its /64.
fn address_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V4(_) => ip,
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => IpAddr::V6(Ipv6Addr::from(u128::from(v6) & (!0u128 << 64))),
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use dessplay_core::test_support::proptest_cases;
    use proptest::prelude::*;
    use std::net::Ipv4Addr;

    const LIMIT: AuthLimit = AuthLimit {
        burst: 3,
        refill: Duration::from_secs(10),
    };

    fn v4(last: u8) -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, last))
    }

    #[test]
    fn spent_budget_refuses_even_the_right_password() {
        let mut limiter = AuthLimiter::new(LIMIT);
        let t0 = Instant::now();
        for _ in 0..LIMIT.burst {
            assert_eq!(limiter.judge(v4(1), t0, || false), Verdict::Reject);
        }
        assert_eq!(limiter.judge(v4(1), t0, || true), Verdict::Throttled);
        // Another address is untouched.
        assert_eq!(limiter.judge(v4(2), t0, || true), Verdict::Accept);
        // One refill later, one attempt is allowed again.
        let t1 = t0 + LIMIT.refill;
        assert_eq!(limiter.judge(v4(1), t1, || true), Verdict::Accept);
        assert_eq!(limiter.judge(v4(1), t1, || false), Verdict::Reject);
        assert_eq!(limiter.judge(v4(1), t1, || true), Verdict::Throttled);
    }

    #[test]
    fn ipv6_shares_a_budget_per_64_and_mapped_v4_is_v4() {
        let a: IpAddr = "2001:db8:1:2::1".parse().unwrap();
        let b: IpAddr = "2001:db8:1:2:ffff::9".parse().unwrap();
        let other: IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert_eq!(address_key(a), address_key(b));
        assert_ne!(address_key(a), address_key(other));
        let mapped: IpAddr = "::ffff:192.0.2.7".parse().unwrap();
        assert_eq!(address_key(mapped), v4(7));
    }

    #[test]
    fn refilled_addresses_are_swept() {
        let mut limiter = AuthLimiter::new(LIMIT);
        let t0 = Instant::now();
        for i in 0..SWEEP_AT as u32 {
            let ip = IpAddr::V4(Ipv4Addr::from(0x0a00_0000 + i));
            limiter.judge(ip, t0, || false);
        }
        assert_eq!(limiter.tracked(), SWEEP_AT);
        // Once their budgets are full again, the next new address
        // sweeps them all out.
        limiter.judge(v4(1), t0 + LIMIT.refill, || false);
        assert_eq!(limiter.tracked(), 1);
    }

    /// One attempt: wait `gap_ms`, then try from address `who` with a
    /// right or wrong password.
    fn attempts() -> impl Strategy<Value = Vec<(u64, u8, bool)>> {
        prop::collection::vec((0u64..15_000, 0u8..3, any::<bool>()), 1..80)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(proptest_cases(256)))]

        /// Over any window, an address has at most `burst` failures plus
        /// one per whole `refill` judged; a right password is accepted
        /// whenever the address isn't throttled; a throttled password is
        /// never checked; and an address is
        /// throttled only when it has failed `burst` times within the
        /// last `burst × refill`.
        #[test]
        fn failures_are_rate_bounded(attempts in attempts()) {
            let mut limiter = AuthLimiter::new(LIMIT);
            let t0 = Instant::now();
            let mut now = t0;
            // Per address: the times of its judged failures.
            let mut failures: [Vec<Instant>; 3] = Default::default();
            for (gap_ms, who, correct) in attempts {
                now += Duration::from_millis(gap_ms);
                let mut asked = false;
                let verdict = limiter.judge(v4(who), now, || {
                    asked = true;
                    correct
                });
                let mine = &mut failures[who as usize];
                match verdict {
                    Verdict::Accept => prop_assert!(correct),
                    Verdict::Reject => {
                        prop_assert!(!correct);
                        mine.push(now);
                    }
                    Verdict::Throttled => {
                        prop_assert!(!asked, "a throttled password was checked");
                        let horizon = LIMIT.refill * LIMIT.burst;
                        let recent = mine
                            .iter()
                            .filter(|t| now.duration_since(**t) < horizon)
                            .count();
                        prop_assert!(
                            recent >= LIMIT.burst as usize,
                            "throttled with only {recent} recent failures"
                        );
                    }
                }
                // The rate bound, over every window ending now.
                for (i, start) in mine.iter().enumerate() {
                    let in_window = (mine.len() - i) as u64;
                    let span = now.duration_since(*start);
                    let allowed =
                        LIMIT.burst as u64 + (span.as_millis() / LIMIT.refill.as_millis()) as u64;
                    prop_assert!(
                        in_window <= allowed,
                        "{in_window} failures in {span:?}, allowed {allowed}"
                    );
                }
            }
        }

        /// A fresh address gets exactly `burst` judged failures back to
        /// back, then is throttled.
        #[test]
        fn fresh_address_gets_its_burst(extra in 1u32..10) {
            let mut limiter = AuthLimiter::new(LIMIT);
            let t0 = Instant::now();
            let verdicts: Vec<_> = (0..LIMIT.burst + extra)
                .map(|_| limiter.judge(v4(1), t0, || false))
                .collect();
            let judged = verdicts.iter().filter(|v| **v == Verdict::Reject).count();
            prop_assert_eq!(judged, LIMIT.burst as usize);
            prop_assert!(verdicts[LIMIT.burst as usize..]
                .iter()
                .all(|v| *v == Verdict::Throttled));
        }
    }
}
