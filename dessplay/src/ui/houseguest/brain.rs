//! Her needs, what she wants (a table: [`Want::def`]), and choosing what
//! to do next from what's on offer.
//!
//! Needs rise slowly while she's around and fall when something serves
//! them; they only *weight* her choices — she never sickens, starves or
//! sulks. Each decision scores every offer as `base × fit × cooldown` and
//! picks at random, weighted by score, among the top few: never the
//! single best (robotic), never flat random (a slot machine). An offer's
//! `fit` is a floor plus its need squared — a need weighs little until
//! it's pressing — and the floor keeps every offer possible whatever her
//! needs say. One she'd do for its own sake too (a nap on the sofa)
//! never fits worse than one that answers no need.

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
    /// Rises over a visit; a snack from her fridge answers it.
    Hungry,
}

/// Her needs, each 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Needs {
    pub sleepy: f64,
    pub restless: f64,
    pub tidy: f64,
    pub mischief: f64,
    pub hungry: f64,
}

/// Milliseconds for each need to rise from 0 to 1.
const SLEEPY_MS: f64 = 15.0 * 60_000.0;
const RESTLESS_MS: f64 = 90_000.0;
const TIDY_MS: f64 = 60_000.0;
const MISCHIEF_MS: f64 = 4.0 * 60_000.0;
const HUNGRY_MS: f64 = 20.0 * 60_000.0;

impl Default for Needs {
    /// Arriving, she's wide awake, keen to move and to look around.
    fn default() -> Self {
        Self {
            sleepy: 0.0,
            restless: 0.7,
            tidy: 0.5,
            mischief: 0.2,
            hungry: 0.2,
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
            Need::Hungry => &mut self.hungry,
        }
    }

    pub fn get(&self, need: Need) -> f64 {
        match need {
            Need::Sleepy => self.sleepy,
            Need::Restless => self.restless,
            Need::Tidy => self.tidy,
            Need::Mischief => self.mischief,
            Need::Hungry => self.hungry,
        }
    }

    /// `ms` have passed; `mess` whether there was text on offer to tidy.
    pub fn pass(&mut self, ms: u64, mess: bool) {
        let ms = ms as f64;
        self.sleepy += ms / SLEEPY_MS;
        self.restless += ms / RESTLESS_MS;
        self.mischief += ms / MISCHIEF_MS;
        self.hungry += ms / HUNGRY_MS;
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
        for need in [
            Need::Sleepy,
            Need::Restless,
            Need::Tidy,
            Need::Mischief,
            Need::Hungry,
        ] {
            let v = self.get_mut(need);
            *v = v.clamp(0.0, 1.0);
        }
    }

    /// A compact readout for the stage and logs.
    pub fn summary(&self) -> String {
        format!(
            "sleepy {:.1} restless {:.1} tidy {:.1} mischief {:.1} hungry {:.1}",
            self.sleepy, self.restless, self.tidy, self.mischief, self.hungry
        )
    }
}

/// What she could want to do next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Want {
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
    /// Off to her part-time job for a while.
    Work,
}

/// Something that makes a want more or less likely, beyond her needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Factor {
    /// Where it would take her is in the chat pane (she's resident, and
    /// people read there): this much as likely.
    InChat(f64),
}

/// A want, as data: how much she likes it all else equal, the needs it
/// answers (and by how much), and what else weighs on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct DesireDef {
    pub base: f64,
    pub serves: &'static [(Need, f64)],
    /// She does it for its own sake as well as for its needs: its fit
    /// never drops below that of a want answering none. A nap on the
    /// sofa is lounging when she's awake, and a doze when she's sleepy.
    pub own_sake: bool,
    pub factors: &'static [Factor],
}

/// What takes her into the chat is a tenth as likely.
const IN_CHAT: &[Factor] = &[Factor::InChat(super::osaka::CHAT_FACTOR)];

const fn row(base: f64, serves: &'static [(Need, f64)]) -> DesireDef {
    DesireDef {
        base,
        serves,
        own_sake: false,
        factors: &[],
    }
}

const fn in_chat(def: DesireDef) -> DesireDef {
    DesireDef {
        factors: IN_CHAT,
        ..def
    }
}

impl Want {
    /// Every want there is.
    #[cfg(test)]
    pub const ALL: [Want; 24] = [
        Self::Stand,
        Self::SpaceOut,
        Self::Sneeze,
        Self::Idle(Activity::Sit),
        Self::Idle(Activity::LieBack),
        Self::Idle(Activity::LieFront),
        Self::Idle(Activity::Jacks),
        Self::Idle(Activity::ToeTouch),
        Self::Idle(Activity::Stretch),
        Self::Idle(Activity::Gaze),
        Self::Walk,
        Self::Travel,
        Self::Pull,
        Self::Swap,
        Self::Use(Use::Lounge),
        Self::Use(Use::Nap),
        Self::Use(Use::Sleep),
        Self::Use(Use::Homework),
        Self::Use(Use::Watch),
        Self::Use(Use::Unpack),
        Self::Use(Use::Read),
        Self::Use(Use::Snack),
        Self::Use(Use::Pet),
        Self::Use(Use::Crumple),
    ];

    /// Its row of the table.
    pub fn def(self) -> DesireDef {
        match self {
            Self::Stand => row(4.0, &[]),
            Self::SpaceOut => row(6.0, &[]),
            Self::Sneeze => row(2.0, &[]),
            Self::Idle(Activity::Stretch) => row(4.0, &[(Need::Restless, 0.5)]),
            // A doze only takes the edge off: over a visit she gets
            // sleepier, and dozes more often. Weak while she's awake, a
            // real contender once she's sleepy.
            Self::Idle(Activity::LieBack) => row(6.0, &[(Need::Sleepy, 0.15)]),
            Self::Idle(Activity::Sit) => row(6.0, &[(Need::Sleepy, 0.1)]),
            Self::Idle(Activity::Jacks | Activity::ToeTouch) => row(6.0, &[(Need::Restless, 0.5)]),
            Self::Idle(Activity::LieFront | Activity::Gaze) => row(6.0, &[]),
            Self::Walk => row(14.0, &[(Need::Restless, 0.4)]),
            Self::Travel => in_chat(row(10.0, &[(Need::Restless, 0.4)])),
            Self::Pull => in_chat(row(16.0, &[(Need::Tidy, 0.6)])),
            Self::Swap => in_chat(row(8.0, &[(Need::Mischief, 0.8)])),
            // Once a visit at most (osaka.rs), so it can afford to compete.
            Self::Work => row(9.0, &[]),
            // Her own things are what home is for (made of text in the
            // chat, a tenth as likely too).
            Self::Use(Use::Watch) => in_chat(row(10.0, &[])),
            // A proper bed answers sleepiness far better than a border.
            Self::Use(Use::Sleep) => in_chat(row(10.0, &[(Need::Sleepy, 0.7)])),
            // A parcel! Nothing comes close.
            Self::Use(Use::Unpack) => in_chat(row(40.0, &[])),
            // A heap of torn text she meant to make something of.
            Self::Use(Use::Crumple) => in_chat(row(12.0, &[])),
            // A nap on a sofa (even one of her own making) draws her more
            // than a border when she's sleepy, but she naps awake too:
            // each takes a little off, or she'd never get to bed.
            Self::Use(Use::Nap) => DesireDef {
                own_sake: true,
                ..in_chat(row(8.0, &[(Need::Sleepy, 0.1)]))
            },
            Self::Use(Use::Snack) => in_chat(row(8.0, &[(Need::Hungry, 0.8)])),
            Self::Use(Use::Lounge | Use::Homework | Use::Read | Use::Pet) => in_chat(row(8.0, &[])),
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
pub(super) fn score(want: Want, needs: &Needs, recent: &[Want]) -> f64 {
    let def = want.def();
    // Squared: a need weighs little until it's pressing.
    let fit = if def.serves.is_empty() {
        NEUTRAL
    } else {
        let fit = FLOOR
            + def
                .serves
                .iter()
                .map(|&(need, _)| needs.get(need).powi(2))
                .sum::<f64>();
        if def.own_sake { NEUTRAL.max(fit) } else { fit }
    };
    let repeats = recent.iter().filter(|&&w| w == want).count();
    def.base * fit * COOLDOWN.powi(repeats as i32)
}

/// Choose among `offers`: weighted by score, times the offer's `factor`
/// (where it would take her), among the top few. Returns the chosen
/// index and the scored top offers (for the log).
pub(super) fn choose(
    offers: &[Want],
    needs: &Needs,
    recent: &[Want],
    factor: &dyn Fn(Want) -> f64,
    rng: &mut Rng,
) -> Option<(usize, Vec<(Want, f64)>)> {
    let mut scored: Vec<(usize, f64)> = offers
        .iter()
        .enumerate()
        .map(|(i, &k)| (i, score(k, needs, recent) * factor(k)))
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

    fn all() -> Vec<Want> {
        let mut offers = vec![
            Want::Stand,
            Want::SpaceOut,
            Want::Sneeze,
            Want::Walk,
            Want::Travel,
            Want::Pull,
            Want::Swap,
        ];
        offers.extend(Activity::ALL.iter().map(|&a| Want::Idle(a)));
        offers
    }

    fn tally(needs: Needs, offers: &[Want]) -> std::collections::HashMap<Want, usize> {
        let mut rng = Rng(9);
        let mut counts = std::collections::HashMap::new();
        for _ in 0..2000 {
            let (i, _) = choose(offers, &needs, &[], &|_| 1.0, &mut rng).unwrap();
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
            hungry: 0.0,
        };
        let counts = tally(needs, &all());
        let lie = counts[&Want::Idle(Activity::LieBack)];
        assert!(counts.values().all(|&n| n <= lie), "{counts:?}");
    }

    /// Over a visit she gets sleepier and lies down more: with the same
    /// offers, a settled Osaka at a visit's end (sleepy 0.9) lies down at
    /// least twice as often as ten minutes in (sleepy 0.3), choosing as she
    /// does — her last few choices cooling repeats.
    #[test]
    fn sleepiness_draws_her_to_lie_down() {
        let share = |sleepy: f64| {
            let needs = Needs {
                sleepy,
                restless: 0.1,
                tidy: 0.0,
                mischief: 0.2,
                hungry: 0.0,
            };
            let offers = all();
            let mut rng = Rng(3);
            let mut recent: Vec<Want> = Vec::new();
            let mut lie = 0;
            for _ in 0..4000 {
                let (i, _) = choose(&offers, &needs, &recent, &|_| 1.0, &mut rng).unwrap();
                let kind = offers[i];
                lie += usize::from(kind == Want::Idle(Activity::LieBack));
                recent.push(kind);
                if recent.len() > 3 {
                    recent.remove(0);
                }
            }
            lie
        };
        let (awake, sleepy) = (share(0.3), share(0.9));
        assert!(
            sleepy > 2 * awake,
            "lay down {awake} times awake, {sleepy} sleepy"
        );
    }

    /// Hungry, with a fridge on offer, a snack is her likeliest choice.
    #[test]
    fn a_hungry_osaka_has_a_snack() {
        let needs = Needs {
            sleepy: 0.0,
            restless: 0.0,
            tidy: 0.0,
            mischief: 0.0,
            hungry: 1.0,
        };
        let mut offers = all();
        offers.push(Want::Use(Use::Snack));
        let counts = tally(needs, &offers);
        let snack = counts[&Want::Use(Use::Snack)];
        assert!(counts.values().all(|&n| n <= snack), "{counts:?}");
    }

    #[test]
    fn a_restless_osaka_mostly_moves() {
        let needs = Needs {
            sleepy: 0.0,
            restless: 1.0,
            tidy: 0.0,
            mischief: 0.0,
            hungry: 0.0,
        };
        let counts = tally(needs, &all());
        let moving: usize = [Want::Walk, Want::Travel]
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
                hungry: 0.0,
            },
            Needs {
                sleepy: 1.0,
                restless: 1.0,
                tidy: 1.0,
                mischief: 1.0,
                hungry: 0.0,
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
        let fresh = score(Want::Walk, &needs, &[]);
        let again = score(Want::Walk, &needs, &[Want::Walk, Want::Walk]);
        assert!(again < fresh * 0.2);
    }

    /// Every row is sane: she likes everything a little, and what it
    /// answers takes something off without wiping the need out.
    #[test]
    fn every_want_has_a_sane_row() {
        for want in Want::ALL {
            let def = want.def();
            assert!(def.base > 0.0, "{want:?}");
            for &(need, amount) in def.serves {
                assert!(amount > 0.0 && amount <= 1.0, "{want:?} {need:?}");
            }
            for &Factor::InChat(times) in def.factors {
                assert!(times > 0.0 && times < 1.0, "{want:?}");
            }
            assert!(!def.own_sake || !def.serves.is_empty(), "{want:?}");
        }
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
