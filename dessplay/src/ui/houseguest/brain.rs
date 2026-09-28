//! Her needs, and choosing what to do next from what's on offer.
//!
//! Needs rise slowly while she's around and fall when something serves
//! them; they only *weight* her choices — she never sickens, starves or
//! sulks. Each decision scores every offer as `base × fit × cooldown` and
//! picks at random, weighted by score, among the top few: never the
//! single best (robotic), never flat random (a slot machine). An offer's
//! `fit` is a floor plus its need squared — a need weighs little until
//! it's pressing — and the floor keeps every offer possible whatever her
//! needs say.

use super::Rng;
use super::osaka::Activity;
use super::room::Use;

/// Something she can want.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Need {
    /// Rises over a visit; a doze on her back eases it a little.
    Sleepy,
    /// Rises steadily; moving about answers it.
    Restless,
    /// Rises while there's text to tidy; tidying answers it.
    Tidy,
    /// Rises slowly; a swapped letter answers it.
    Mischief,
}

/// Her needs, each 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Needs {
    pub sleepy: f64,
    pub restless: f64,
    pub tidy: f64,
    pub mischief: f64,
}

/// Milliseconds for each need to rise from 0 to 1.
const SLEEPY_MS: f64 = 15.0 * 60_000.0;
const RESTLESS_MS: f64 = 90_000.0;
const TIDY_MS: f64 = 60_000.0;
const MISCHIEF_MS: f64 = 4.0 * 60_000.0;

impl Default for Needs {
    /// Arriving, she's wide awake, keen to move and to look around.
    fn default() -> Self {
        Self {
            sleepy: 0.0,
            restless: 0.7,
            tidy: 0.5,
            mischief: 0.2,
        }
    }
}

impl Needs {
    fn get_mut(&mut self, need: Need) -> &mut f64 {
        match need {
            Need::Sleepy => &mut self.sleepy,
            Need::Restless => &mut self.restless,
            Need::Tidy => &mut self.tidy,
            Need::Mischief => &mut self.mischief,
        }
    }

    pub fn get(&self, need: Need) -> f64 {
        match need {
            Need::Sleepy => self.sleepy,
            Need::Restless => self.restless,
            Need::Tidy => self.tidy,
            Need::Mischief => self.mischief,
        }
    }

    /// `ms` have passed; `mess` whether there was text on offer to tidy.
    pub fn pass(&mut self, ms: u64, mess: bool) {
        let ms = ms as f64;
        self.sleepy += ms / SLEEPY_MS;
        self.restless += ms / RESTLESS_MS;
        self.mischief += ms / MISCHIEF_MS;
        if mess {
            self.tidy += ms / TIDY_MS;
        }
        self.clamp();
    }

    /// She did something that serves `need` by `amount`.
    pub fn serve(&mut self, need: Need, amount: f64) {
        *self.get_mut(need) -= amount;
        self.clamp();
    }

    fn clamp(&mut self) {
        for need in [Need::Sleepy, Need::Restless, Need::Tidy, Need::Mischief] {
            let v = self.get_mut(need);
            *v = v.clamp(0.0, 1.0);
        }
    }

    /// A compact readout for the stage and logs.
    pub fn summary(&self) -> String {
        format!(
            "sleepy {:.1} restless {:.1} tidy {:.1} mischief {:.1}",
            self.sleepy, self.restless, self.tidy, self.mischief
        )
    }
}

/// What she could do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    /// Stand facing the viewer.
    Stand,
    /// Space out (sometimes musing aloud).
    SpaceOut,
    Sneeze,
    Idle(Activity),
    /// Walk somewhere on this floor.
    Walk,
    /// Climb or hop to another floor.
    Travel,
    Pull,
    Swap,
    /// Use a piece of her furniture.
    Use(Use),
}

impl Kind {
    /// How much she likes it, all else equal.
    fn base(self) -> f64 {
        match self {
            Self::Stand => 4.0,
            Self::SpaceOut => 6.0,
            Self::Sneeze => 2.0,
            Self::Idle(Activity::Stretch) => 4.0,
            // Weak while she's awake, a real contender once she's sleepy.
            Self::Idle(Activity::LieBack) => 6.0,
            Self::Idle(_) => 6.0,
            Self::Walk => 14.0,
            Self::Travel => 10.0,
            Self::Pull => 16.0,
            Self::Swap => 8.0,
            // Her own things are what home is for.
            Self::Use(Use::Watch) => 10.0,
            Self::Use(Use::Sleep) => 10.0,
            Self::Use(_) => 8.0,
        }
    }

    /// The need it answers, and by how much.
    pub fn serves(self) -> Option<(Need, f64)> {
        match self {
            // A doze only takes the edge off: over a visit she gets
            // sleepier, and dozes more often.
            Self::Idle(Activity::LieBack) => Some((Need::Sleepy, 0.15)),
            Self::Idle(Activity::Sit) => Some((Need::Sleepy, 0.1)),
            Self::Idle(Activity::Jacks | Activity::ToeTouch | Activity::Stretch) => {
                Some((Need::Restless, 0.5))
            }
            Self::Walk | Self::Travel => Some((Need::Restless, 0.4)),
            Self::Pull => Some((Need::Tidy, 0.6)),
            Self::Swap => Some((Need::Mischief, 0.8)),
            // A proper bed answers sleepiness far better than a border.
            Self::Use(Use::Sleep) => Some((Need::Sleepy, 0.7)),
            Self::Use(Use::Nap) => Some((Need::Sleepy, 0.3)),
            Self::Use(Use::Lounge | Use::Homework | Use::Watch) => None,
            Self::Stand
            | Self::SpaceOut
            | Self::Sneeze
            | Self::Idle(Activity::LieFront | Activity::Gaze) => None,
        }
    }
}

/// No offer's fit drops below this, whatever her needs.
const FLOOR: f64 = 0.1;
/// The fit of an offer that answers no need.
const NEUTRAL: f64 = 0.5;
/// Each of the last few choices of the same kind multiplies by this.
const COOLDOWN: f64 = 0.4;
/// She picks among this many best offers.
const TOP: usize = 4;

/// An offer's score given her needs and what she did lately.
pub(super) fn score(kind: Kind, needs: &Needs, recent: &[Kind]) -> f64 {
    // Squared: a need weighs little until it's pressing.
    let fit = kind
        .serves()
        .map_or(NEUTRAL, |(need, _)| FLOOR + needs.get(need).powi(2));
    let repeats = recent.iter().filter(|&&k| k == kind).count();
    kind.base() * fit * COOLDOWN.powi(repeats as i32)
}

/// Choose among `offers`: weighted by score among the top few. Returns
/// the chosen index and the scored top offers (for the log).
pub(super) fn choose(
    offers: &[Kind],
    needs: &Needs,
    recent: &[Kind],
    rng: &mut Rng,
) -> Option<(usize, Vec<(Kind, f64)>)> {
    let mut scored: Vec<(usize, f64)> = offers
        .iter()
        .enumerate()
        .map(|(i, &k)| (i, score(k, needs, recent)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(TOP);
    let total: f64 = scored.iter().map(|(_, s)| s).sum();
    if scored.is_empty() || total <= 0.0 {
        return None;
    }
    // A 1/1000 grid is plenty for weighting a handful of offers.
    let mut roll = rng.below(1000) as f64 / 1000.0 * total;
    let mut pick = scored.first().map(|(i, _)| *i)?;
    for &(i, s) in &scored {
        if roll < s {
            pick = i;
            break;
        }
        roll -= s;
    }
    let top = scored
        .iter()
        .filter_map(|&(i, s)| offers.get(i).map(|&k| (k, s)))
        .collect();
    Some((pick, top))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn all() -> Vec<Kind> {
        let mut offers = vec![
            Kind::Stand,
            Kind::SpaceOut,
            Kind::Sneeze,
            Kind::Walk,
            Kind::Travel,
            Kind::Pull,
            Kind::Swap,
        ];
        offers.extend(Activity::ALL.iter().map(|&a| Kind::Idle(a)));
        offers
    }

    fn tally(needs: Needs, offers: &[Kind]) -> std::collections::HashMap<Kind, usize> {
        let mut rng = Rng(9);
        let mut counts = std::collections::HashMap::new();
        for _ in 0..2000 {
            let (i, _) = choose(offers, &needs, &[], &mut rng).unwrap();
            *counts.entry(offers[i]).or_default() += 1;
        }
        counts
    }

    #[test]
    fn a_sleepy_osaka_mostly_lies_down() {
        let needs = Needs {
            sleepy: 1.0,
            restless: 0.0,
            tidy: 0.0,
            mischief: 0.0,
        };
        let counts = tally(needs, &all());
        let lie = counts[&Kind::Idle(Activity::LieBack)];
        assert!(counts.values().all(|&n| n <= lie), "{counts:?}");
    }

    #[test]
    fn a_restless_osaka_mostly_moves() {
        let needs = Needs {
            sleepy: 0.0,
            restless: 1.0,
            tidy: 0.0,
            mischief: 0.0,
        };
        let counts = tally(needs, &all());
        let moving: usize = [Kind::Walk, Kind::Travel]
            .iter()
            .map(|k| counts.get(k).copied().unwrap_or(0))
            .sum();
        assert!(moving * 2 > 2000, "{counts:?}");
    }

    /// Never argmax: even with nothing pressing, several offers come up.
    #[test]
    fn choices_vary_whatever_the_needs() {
        for needs in [
            Needs::default(),
            Needs {
                sleepy: 0.0,
                restless: 0.0,
                tidy: 0.0,
                mischief: 0.0,
            },
            Needs {
                sleepy: 1.0,
                restless: 1.0,
                tidy: 1.0,
                mischief: 1.0,
            },
        ] {
            let counts = tally(needs, &all());
            assert!(counts.len() >= 3, "{needs:?}: {counts:?}");
            assert!(counts.values().all(|&n| n < 1500), "{needs:?}: {counts:?}");
        }
    }

    #[test]
    fn repeating_herself_is_discouraged() {
        let needs = Needs::default();
        let fresh = score(Kind::Walk, &needs, &[]);
        let again = score(Kind::Walk, &needs, &[Kind::Walk, Kind::Walk]);
        assert!(again < fresh * 0.2);
    }

    #[test]
    fn needs_stay_in_range() {
        let mut needs = Needs::default();
        needs.pass(10 * 3_600_000, true);
        assert_eq!(needs.sleepy, 1.0);
        needs.serve(Need::Sleepy, 5.0);
        assert_eq!(needs.sleepy, 0.0);
    }
}
