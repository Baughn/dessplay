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

use super::mind::Whims;
use super::osaka::Activity;
use super::room::Use;

/// Something she can want.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Need {
    /// Rises over a visit; a bed answers it best, a nap on a sofa well,
    /// a doze on the floor a little.
    Sleepy,
    /// Rises steadily; moving about answers it.
    Restless,
    /// Rises while there's text to tidy; tidying answers it.
    Tidy,
    /// Rises slowly; a swapped letter answers it.
    Mischief,
    /// Rises over a visit; a snack from her fridge answers it.
    Hungry,
    /// Rises steadily; sitting or lying on furniture answers it (a real
    /// piece best, one she made well, the floor barely).
    Comfort,
    /// Rises steadily; the TV, a book, the cat, a parcel and mischief
    /// answer it.
    Fun,
    /// Rises slowly; spacing out, gazing and drifting off answer it.
    Daydreams,
}

impl Need {
    pub const ALL: [Need; 8] = [
        Self::Sleepy,
        Self::Restless,
        Self::Tidy,
        Self::Mischief,
        Self::Hungry,
        Self::Comfort,
        Self::Fun,
        Self::Daydreams,
    ];

    /// Milliseconds to rise from 0 to 1 (tidy only while there's text on
    /// offer to tidy).
    fn rise_ms(self) -> f64 {
        match self {
            Self::Sleepy => 15.0 * 60_000.0,
            Self::Restless => 90_000.0,
            Self::Tidy => 60_000.0,
            Self::Mischief => 4.0 * 60_000.0,
            Self::Hungry => 20.0 * 60_000.0,
            Self::Comfort => 8.0 * 60_000.0,
            Self::Fun => 6.0 * 60_000.0,
            Self::Daydreams => 10.0 * 60_000.0,
        }
    }

    /// Where she starts a visit: wide awake, keen to move and to look
    /// around, the rest about halfway, so no want is starved at arrival.
    fn arriving(self) -> f64 {
        match self {
            Self::Sleepy => 0.0,
            Self::Restless => 0.7,
            Self::Tidy | Self::Comfort | Self::Fun | Self::Daydreams => 0.5,
            Self::Mischief | Self::Hungry => 0.2,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Sleepy => "sleepy",
            Self::Restless => "restless",
            Self::Tidy => "tidy",
            Self::Mischief => "mischief",
            Self::Hungry => "hungry",
            Self::Comfort => "comfort",
            Self::Fun => "fun",
            Self::Daydreams => "daydreams",
        }
    }
}

/// Her mood for the visit: she has a life outside dessplay. A mood is how
/// fast her needs rise, and how she says hello.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mood {
    Ordinary,
    /// Comfort and sleep come quicker, restlessness slower.
    Lazy,
    /// Tidying and moving about come quicker, comfort slower.
    Industrious,
    /// Daydreams come quickly.
    Dreamy,
}

impl Mood {
    #[cfg(test)]
    pub const ALL: [Mood; 4] = [Self::Ordinary, Self::Lazy, Self::Industrious, Self::Dreamy];

    /// The visit's mood, from its seed: ordinary half the time, lazy and
    /// industrious a fifth each, dreamy a tenth.
    pub fn of(seed: u64) -> Self {
        let mut z = seed ^ 0x6d6f_6f64;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        match (z ^ (z >> 31)) % 10 {
            0..=4 => Self::Ordinary,
            5 | 6 => Self::Lazy,
            7 | 8 => Self::Industrious,
            _ => Self::Dreamy,
        }
    }

    /// How fast `need` rises, against an ordinary visit.
    pub fn rate(self, need: Need) -> f64 {
        match (self, need) {
            (Self::Lazy, Need::Comfort) => 2.0,
            (Self::Lazy, Need::Sleepy) => 1.5,
            (Self::Lazy, Need::Restless) => 0.4,
            (Self::Lazy, Need::Tidy) => 0.7,
            (Self::Industrious, Need::Tidy) => 1.6,
            (Self::Industrious, Need::Restless) => 1.5,
            (Self::Industrious, Need::Comfort) => 0.5,
            (Self::Industrious, Need::Sleepy) => 0.8,
            (Self::Industrious, Need::Daydreams) => 0.6,
            (Self::Dreamy, Need::Daydreams) => 2.5,
            (Self::Dreamy, Need::Restless) => 0.7,
            _ => 1.0,
        }
    }

    /// What she says on first finding her feet: a hint at her mood.
    pub fn greeting(self) -> &'static str {
        match self {
            Self::Ordinary => "Nice to meet you.",
            Self::Lazy => "Mm... lazy day.",
            Self::Industrious => "Okay! Let's tidy up!",
            Self::Dreamy => "...hm? Oh, hello.",
        }
    }
}

/// Where a want would have her, as far as her needs care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Spot {
    /// On (or at) a real piece of her furniture.
    Real,
    /// On a piece she made of text.
    Made,
    /// On the floor.
    Floor,
    /// Nowhere in particular.
    Any,
}

/// How well `spot` answers `need`: comfort and sleep are better on
/// furniture than the floor, and on the real thing than a makeshift one.
pub(super) fn quality(need: Need, spot: Spot) -> f64 {
    match (need, spot) {
        (Need::Comfort, Spot::Made) => 0.6,
        (Need::Comfort, Spot::Floor) => 0.1,
        (Need::Sleepy, Spot::Made) => 0.7,
        (Need::Sleepy, Spot::Floor) => 0.3,
        _ => 1.0,
    }
}

/// Where her fun comes from: each wears thin with use (its tolerance
/// rises) and fresh again with time, so she varies what she enjoys.
const FUN_SOURCES: [Want; 6] = [
    Want::Use(Use::Watch),
    Want::Use(Use::Read),
    Want::Use(Use::Pet),
    Want::Use(Use::Unpack),
    Want::Use(Use::Snack),
    Want::Swap,
];
/// A whole use of a fun source raises its tolerance by this.
const TOLERANCE_PER_USE: f64 = 0.6;
/// Milliseconds for a tolerance to wear off from 1 to 0.
const TOLERANCE_MS: f64 = 10.0 * 60_000.0;

/// Her needs, each 0..=1, and how used she is to each source of fun.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Needs {
    levels: [f64; Need::ALL.len()],
    tolerance: [f64; FUN_SOURCES.len()],
}

impl Default for Needs {
    fn default() -> Self {
        Self {
            levels: Need::ALL.map(Need::arriving),
            tolerance: [0.0; FUN_SOURCES.len()],
        }
    }
}

impl Needs {
    /// All at zero but `set` (tests and the stage).
    #[cfg(test)]
    pub fn with(set: &[(Need, f64)]) -> Self {
        let mut needs = Self {
            levels: [0.0; Need::ALL.len()],
            tolerance: [0.0; FUN_SOURCES.len()],
        };
        for &(need, level) in set {
            needs.levels[need as usize] = level;
        }
        needs
    }

    pub fn get(&self, need: Need) -> f64 {
        self.levels[need as usize]
    }

    /// `ms` have passed in `mood`; `mess` whether there was text on offer
    /// to tidy.
    pub fn pass(&mut self, ms: u64, mess: bool, mood: Mood) {
        for need in Need::ALL {
            if need != Need::Tidy || mess {
                self.levels[need as usize] += ms as f64 * mood.rate(need) / need.rise_ms();
            }
        }
        for tolerance in &mut self.tolerance {
            *tolerance = (*tolerance - ms as f64 / TOLERANCE_MS).max(0.0);
        }
        self.clamp();
    }

    /// How much fun `want` still is: 1 fresh, 0 worn out.
    pub fn fresh(&self, want: Want) -> f64 {
        FUN_SOURCES
            .iter()
            .position(|&w| w == want)
            .map_or(1.0, |i| 1.0 - self.tolerance[i])
    }

    /// She did `share` of `want`: if it's a source of fun, it wears a
    /// little thinner.
    pub fn enjoyed(&mut self, want: Want, share: f64) {
        if let Some(i) = FUN_SOURCES.iter().position(|&w| w == want) {
            self.tolerance[i] = (self.tolerance[i] + TOLERANCE_PER_USE * share).min(1.0);
        }
    }

    /// She did something that serves `need` by `amount`.
    pub fn serve(&mut self, need: Need, amount: f64) {
        self.levels[need as usize] -= amount;
        self.clamp();
    }

    fn clamp(&mut self) {
        for level in &mut self.levels {
            *level = level.clamp(0.0, 1.0);
        }
    }

    /// A compact readout for the stage and logs.
    pub fn summary(&self) -> String {
        Need::ALL
            .iter()
            .map(|&need| format!("{} {:.1}", need.name(), self.get(need)))
            .collect::<Vec<_>>()
            .join(" ")
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
    /// Every want there is (the order she considers them in).
    pub const ALL: [Want; 25] = [
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
        Self::Work,
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
            // The one filler: what she does when nothing else binds.
            Self::Stand => row(4.0, &[]),
            Self::SpaceOut => row(6.0, &[(Need::Daydreams, 0.6)]),
            // An accident, not a want of anything (mischief would pin it
            // where there's nothing to swap).
            Self::Sneeze => row(2.0, &[]),
            Self::Idle(Activity::Stretch) => row(4.0, &[(Need::Restless, 0.5)]),
            // A doze only takes the edge off: over a visit she gets
            // sleepier, and dozes more often. On the floor, it's a poor
            // answer to sleep or comfort, but an answer.
            Self::Idle(Activity::LieBack) => {
                row(4.0, &[(Need::Sleepy, 0.15), (Need::Comfort, 0.3)])
            }
            Self::Idle(Activity::Sit) => row(4.0, &[(Need::Sleepy, 0.1), (Need::Comfort, 0.3)]),
            Self::Idle(Activity::Jacks | Activity::ToeTouch) => row(6.0, &[(Need::Restless, 0.5)]),
            // Kicking her feet, humming.
            Self::Idle(Activity::LieFront) => {
                row(6.0, &[(Need::Daydreams, 0.3), (Need::Comfort, 0.2)])
            }
            Self::Idle(Activity::Gaze) => row(6.0, &[(Need::Daydreams, 0.5)]),
            Self::Walk => row(14.0, &[(Need::Restless, 0.4)]),
            Self::Travel => in_chat(row(10.0, &[(Need::Restless, 0.4)])),
            Self::Pull => in_chat(row(16.0, &[(Need::Tidy, 0.6)])),
            Self::Swap => in_chat(row(8.0, &[(Need::Mischief, 0.8), (Need::Fun, 0.3)])),
            // Once a visit at most (osaka.rs), so it can afford to compete;
            // out and about, and worth doing whatever she feels.
            Self::Work => DesireDef {
                own_sake: true,
                ..row(9.0, &[(Need::Restless, 0.5)])
            },
            // Her own things are what home is for (made of text in the
            // chat, a tenth as likely too).
            Self::Use(Use::Watch) => in_chat(row(10.0, &[(Need::Fun, 0.6)])),
            // A proper bed answers sleepiness far better than a border.
            Self::Use(Use::Sleep) => {
                in_chat(row(10.0, &[(Need::Sleepy, 0.7), (Need::Comfort, 0.3)]))
            }
            // A parcel! Nothing comes close, whatever she feels.
            Self::Use(Use::Unpack) => DesireDef {
                own_sake: true,
                ..in_chat(row(40.0, &[(Need::Fun, 0.8)]))
            },
            // A heap of torn text is crumpled as a step of what she made
            // it for (no method offers it).
            Self::Use(Use::Crumple) => in_chat(row(12.0, &[])),
            // A nap on a sofa (even one of her own making) draws her more
            // than a border: it's comfortable, and it takes the edge off
            // her sleepiness (each a little, or she'd never get to bed).
            Self::Use(Use::Nap) => in_chat(row(8.0, &[(Need::Sleepy, 0.1), (Need::Comfort, 0.4)])),
            Self::Use(Use::Lounge) => in_chat(row(8.0, &[(Need::Comfort, 0.5)])),
            // At her desk she drifts off.
            Self::Use(Use::Homework) => {
                in_chat(row(8.0, &[(Need::Daydreams, 0.3), (Need::Comfort, 0.2)]))
            }
            Self::Use(Use::Read) => in_chat(row(8.0, &[(Need::Fun, 0.5), (Need::Daydreams, 0.2)])),
            Self::Use(Use::Snack) => in_chat(row(8.0, &[(Need::Hungry, 0.8), (Need::Fun, 0.1)])),
            Self::Use(Use::Pet) => in_chat(row(8.0, &[(Need::Fun, 0.6)])),
        }
    }
}

/// A need's term in the fit is its amount times this: an answer of 0.5
/// weighs what a whole need did when fit ignored amounts.
const WEIGHT: f64 = 2.0;
/// Where she's heading for is this much likelier to stay what she wants.
pub(super) const INERTIA: f64 = 3.0;
/// No offer's fit drops below this, whatever her needs.
const FLOOR: f64 = 0.1;
/// The fit of an offer that answers no need.
const NEUTRAL: f64 = 0.5;
/// Each of the last few choices of the same kind multiplies by this.
const COOLDOWN: f64 = 0.4;
/// She picks among this many best offers.
const TOP: usize = 4;

/// An offer's score given her needs, where it would have her, and what
/// she did lately.
pub(super) fn score(want: Want, spot: Spot, needs: &Needs, recent: &[Want]) -> f64 {
    let def = want.def();
    // Squared: a need weighs little until it's pressing. Each need it
    // answers counts by how much it answers, and how well the spot does.
    let fit = if def.serves.is_empty() {
        NEUTRAL
    } else {
        let fit = FLOOR
            + def
                .serves
                .iter()
                .map(|&(need, amount)| {
                    let fresh = if need == Need::Fun {
                        needs.fresh(want)
                    } else {
                        1.0
                    };
                    needs.get(need).powi(2) * quality(need, spot) * fresh * amount * WEIGHT
                })
                .sum::<f64>();
        if def.own_sake { NEUTRAL.max(fit) } else { fit }
    };
    let repeats = recent.iter().filter(|&&w| w == want).count();
    def.base * fit * COOLDOWN.powi(repeats as i32)
}

/// Choose among `offers`: weighted by score, times the offer's `factor`
/// (where it would take her), among the top few, rolling with `whims`
/// (the `attempt`th roll of this decision). Returns the chosen
/// index and the scored top offers (for the log).
pub(super) fn choose(
    offers: &[(Want, Spot)],
    needs: &Needs,
    recent: &[Want],
    factor: &dyn Fn(Want) -> f64,
    whims: Whims,
    attempt: u64,
) -> Option<(usize, Vec<(Want, f64)>)> {
    let mut scored: Vec<(usize, f64)> = offers
        .iter()
        .enumerate()
        .map(|(i, &(want, spot))| (i, score(want, spot, needs, recent) * factor(want)))
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(TOP);
    let total: f64 = scored.iter().map(|(_, s)| s).sum();
    if scored.is_empty() || total <= 0.0 {
        return None;
    }
    // A 1/1000 grid is plenty for weighting a handful of offers.
    let mut roll = whims.below_at("roll", attempt, 1000) as f64 / 1000.0 * total;
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
        .filter_map(|&(i, s)| offers.get(i).map(|&(want, _)| (want, s)))
        .collect();
    Some((pick, top))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::Rng;
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

    /// Where each want would have her in play: an activity on the floor,
    /// a use at a real piece, anything else nowhere in particular.
    fn spotted(offers: &[Want]) -> Vec<(Want, Spot)> {
        offers
            .iter()
            .map(|&want| {
                let spot = match want {
                    Want::Idle(_) => Spot::Floor,
                    Want::Use(_) => Spot::Real,
                    _ => Spot::Any,
                };
                (want, spot)
            })
            .collect()
    }

    fn tally(needs: Needs, offers: &[Want]) -> std::collections::HashMap<Want, usize> {
        let mut rng = Rng(9);
        let mut counts = std::collections::HashMap::new();
        for _ in 0..2000 {
            let (i, _) = choose(
                &spotted(offers),
                &needs,
                &[],
                &|_| 1.0,
                Whims(rng.next()),
                0,
            )
            .unwrap();
            *counts.entry(offers[i]).or_default() += 1;
        }
        counts
    }

    /// Sleepy, with a bed and a sofa, she mostly goes to bed, naps next,
    /// and both beat dozing on the floor (a poor answer to sleep).
    #[test]
    fn a_sleepy_osaka_goes_to_bed() {
        let needs = Needs::with(&[(Need::Sleepy, 1.0), (Need::Comfort, 0.5)]);
        let mut offers = all();
        offers.extend([Want::Use(Use::Sleep), Want::Use(Use::Nap)]);
        let counts = tally(needs, &offers);
        let n = |want: Want| counts.get(&want).copied().unwrap_or(0);
        let (bed, nap) = (n(Want::Use(Use::Sleep)), n(Want::Use(Use::Nap)));
        assert!(counts.values().all(|&c| c <= bed), "{counts:?}");
        for floor in [Want::Idle(Activity::LieBack), Want::Idle(Activity::Sit)] {
            assert!(n(floor) < nap, "{floor:?}: {counts:?}");
        }
    }

    /// Over a visit she gets sleepier and lies down more: with the same
    /// offers, a settled Osaka at a visit's end (sleepy 0.9) lies down at
    /// least twice as often as ten minutes in (sleepy 0.3), choosing as she
    /// does — her last few choices cooling repeats.
    #[test]
    fn sleepiness_draws_her_to_lie_down() {
        let share = |sleepy: f64| {
            let needs = Needs::with(&[
                (Need::Sleepy, sleepy),
                (Need::Restless, 0.1),
                (Need::Mischief, 0.2),
            ]);
            let offers = all();
            let mut rng = Rng(3);
            let mut recent: Vec<Want> = Vec::new();
            let mut lie = 0;
            for _ in 0..4000 {
                let (i, _) = choose(
                    &spotted(&offers),
                    &needs,
                    &recent,
                    &|_| 1.0,
                    Whims(rng.next()),
                    0,
                )
                .unwrap();
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
        let needs = Needs::with(&[(Need::Hungry, 1.0)]);
        let mut offers = all();
        offers.push(Want::Use(Use::Snack));
        let counts = tally(needs, &offers);
        let snack = counts[&Want::Use(Use::Snack)];
        assert!(counts.values().all(|&n| n <= snack), "{counts:?}");
    }

    #[test]
    fn a_restless_osaka_mostly_moves() {
        let needs = Needs::with(&[(Need::Restless, 1.0)]);
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
            Needs::with(&[]),
            Needs::with(&[
                (Need::Sleepy, 1.0),
                (Need::Restless, 1.0),
                (Need::Tidy, 1.0),
                (Need::Mischief, 1.0),
            ]),
        ] {
            let counts = tally(needs, &all());
            assert!(counts.len() >= 3, "{needs:?}: {counts:?}");
            assert!(counts.values().all(|&n| n < 1500), "{needs:?}: {counts:?}");
        }
    }

    #[test]
    fn repeating_herself_is_discouraged() {
        let needs = Needs::default();
        let fresh = score(Want::Walk, Spot::Any, &needs, &[]);
        let again = score(Want::Walk, Spot::Any, &needs, &[Want::Walk, Want::Walk]);
        assert!(again < fresh * 0.2);
    }

    /// [`Want::ALL`] lists every want, once: each kind, every activity,
    /// every use.
    #[test]
    fn all_lists_every_want() {
        let kind = |want: Want| match want {
            Want::Stand => 0,
            Want::SpaceOut => 1,
            Want::Sneeze => 2,
            Want::Idle(_) => 3,
            Want::Walk => 4,
            Want::Travel => 5,
            Want::Pull => 6,
            Want::Swap => 7,
            Want::Use(_) => 8,
            Want::Work => 9,
        };
        for k in 0..10 {
            assert!(Want::ALL.iter().any(|&w| kind(w) == k), "kind {k} missing");
        }
        for a in Activity::ALL {
            assert!(Want::ALL.contains(&Want::Idle(a)), "{a:?}");
        }
        for furniture in super::super::room::Furniture::ALL {
            for &what in Use::of(furniture) {
                assert!(Want::ALL.contains(&Want::Use(what)), "{what:?}");
            }
        }
        assert!(Want::ALL.contains(&Want::Use(Use::Crumple)));
        for (i, a) in Want::ALL.iter().enumerate() {
            assert!(!Want::ALL[..i].contains(a), "{a:?} twice");
        }
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
            // Every want answers a need, but the filler (Stand), the
            // accident (Sneeze), and crumpling (a step of a purpose).
            let exempt = matches!(want, Want::Stand | Want::Sneeze | Want::Use(Use::Crumple));
            assert!(exempt || !def.serves.is_empty(), "{want:?} answers nothing");
        }
    }

    /// Fun wears thin: the same source, used again and again, answers
    /// less and scores less, until time freshens it; other sources don't
    /// mind.
    #[test]
    fn fun_wears_thin_and_freshens() {
        let tv = Want::Use(Use::Watch);
        let book = Want::Use(Use::Read);
        let mut needs = Needs::with(&[(Need::Fun, 1.0)]);
        let fresh = score(tv, Spot::Real, &needs, &[]);
        needs.enjoyed(tv, 1.0);
        needs.enjoyed(tv, 1.0);
        assert!(score(tv, Spot::Real, &needs, &[]) < fresh * 0.5);
        assert_eq!(needs.fresh(book), 1.0);
        needs.pass(10 * 60_000, false, Mood::Ordinary);
        assert_eq!(needs.fresh(tv), 1.0);
    }

    /// Moods come in the shares the table says, from the visit's seed.
    #[test]
    fn moods_come_in_their_shares() {
        let mut counts = [0usize; 4];
        for seed in 0..10_000u64 {
            let mood = Mood::of(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            counts[Mood::ALL.iter().position(|&m| m == mood).unwrap()] += 1;
        }
        let share = |i: usize| counts[i] as f64 / 10_000.0;
        assert!((share(0) - 0.5).abs() < 0.03, "{counts:?}");
        assert!((share(1) - 0.2).abs() < 0.03, "{counts:?}");
        assert!((share(2) - 0.2).abs() < 0.03, "{counts:?}");
        assert!((share(3) - 0.1).abs() < 0.03, "{counts:?}");
        for mood in Mood::ALL {
            assert!(mood.greeting().chars().count() <= 24, "{mood:?}");
        }
    }

    #[test]
    fn needs_stay_in_range() {
        let mut needs = Needs::default();
        needs.pass(10 * 3_600_000, true, Mood::Ordinary);
        assert!(Need::ALL.iter().all(|&need| needs.get(need) == 1.0));
        needs.serve(Need::Sleepy, 5.0);
        assert_eq!(needs.get(Need::Sleepy), 0.0);
    }
}
