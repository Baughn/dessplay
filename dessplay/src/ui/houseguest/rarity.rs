//! Rarity and pity (phase 5b D6, step 7): what's rare of hers turns up
//! now and then, and certainly in the end. Pure: a draw is a function of
//! its seed, what she has seen, her pity counters and the window of her
//! day it's for, and draws from neither random stream.
//!
//! Only her Rare and Legendary scripts pass a gate. Common and Uncommon
//! ones are ungated: the chance each already rolls *is* its rarity, so no
//! second system stacks on it.
//!
//! The gate is [`Rares`], drawn once a game day while her routine is fed
//! (as her visit begins, and as she wakes into a new day: "the day is the
//! unit") and once a visit while it isn't. Each tier rolls its base chance
//! plus a ramp in her pity counter (real idle minutes since she last
//! showed something new of that tier), certain at the tier's bound: the
//! seen scripts of a tier that passes are open, and at most one unseen
//! script is new.

use super::routine::{Slot, SlotSet};
use super::script::ScriptId;

/// How rare a script of hers is (wildcard-free on [`ScriptId::rarity`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Rarity {
    /// Whatever she does: ungated.
    Common,
    /// Now and then, by its own chance roll: ungated.
    Uncommon,
    /// A day in seven or so, and certain after six idle hours without
    /// anything new.
    Rare,
    /// A day in fifty or so, and certain after forty idle hours without
    /// anything new.
    Legendary,
}

impl Rarity {
    /// Whether it passes the day's gate ([`Rares`]): Rare and Legendary
    /// only.
    pub fn gated(self) -> bool {
        match self {
            Self::Common | Self::Uncommon => false,
            Self::Rare | Self::Legendary => true,
        }
    }

    /// A gated tier's base chance a draw, and the pity (real idle minutes)
    /// at which it's certain (HG's bounds). `None`: ungated.
    fn odds(self) -> Option<(f64, u64)> {
        match self {
            Self::Common | Self::Uncommon => None,
            Self::Rare => Some((0.15, 360)),
            Self::Legendary => Some((0.02, 2400)),
        }
    }

    /// Its own salt for its roll, so the two tiers roll apart.
    fn salt(self) -> u64 {
        match self {
            Self::Common => 0,
            Self::Uncommon => 1,
            Self::Rare => 2,
            Self::Legendary => 3,
        }
    }
}

/// Her pity counters as a draw reads them: real idle minutes since she
/// last showed something rare (and something legendary) for the first
/// time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Pity {
    pub rare: u64,
    pub legend: u64,
}

impl Pity {
    /// From her ledger's counters: `idle_min` now, and its value when she
    /// last showed something new of each tier (a counter read ahead of
    /// `idle_min`, as a garbled record might have it, is no pity yet).
    pub fn of(idle_min: u64, rare_at: u64, legend_at: u64) -> Self {
        Self {
            rare: idle_min.saturating_sub(rare_at),
            legend: idle_min.saturating_sub(legend_at),
        }
    }

    /// The counter `tier` rolls on (none for an ungated tier).
    fn of_tier(self, tier: Rarity) -> u64 {
        match tier {
            Rarity::Common | Rarity::Uncommon => 0,
            Rarity::Rare => self.rare,
            Rarity::Legendary => self.legend,
        }
    }

    /// She has just shown something new of `tier`: its counter starts
    /// again.
    pub fn reset(&mut self, tier: Rarity) {
        match tier {
            Rarity::Common | Rarity::Uncommon => {}
            Rarity::Rare => self.rare = 0,
            Rarity::Legendary => self.legend = 0,
        }
    }
}

/// The chance `tier` passes a draw with its pity counter at `counter`:
/// its base chance, rising in a straight line to certain at its bound
/// (an ungated tier always passes). Monotone in the counter.
pub(super) fn chance(tier: Rarity, counter: u64) -> f64 {
    match tier.odds() {
        None => 1.0,
        Some((_, bound)) if counter >= bound => 1.0,
        Some((base, bound)) => base + (1.0 - base) * counter as f64 / bound as f64,
    }
}

/// Whether `tier` passes the draw seeded `seed` with its counter at
/// `counter`: a pure function of the two (the seed's uniform number
/// below the tier's [`chance`]), so for any seed it passes from some
/// counter on and never stops passing as the counter rises.
pub(super) fn passes(seed: u64, tier: Rarity, counter: u64) -> bool {
    let unit = (mix(seed ^ tier.salt().wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 11) as f64
        / (1u64 << 53) as f64;
    unit < chance(tier, counter)
}

/// Salts a draw's seed (the game day's [`super::brain::day_seed`], or
/// unfed the visit's): keeps its hash apart from every other use of
/// either (her mood's, her midnight snack's, her dash home's), so what's
/// rare today says nothing of her mood.
const RARE_SALT: u64 = 0x7261_7265_5f6f_6e21;

/// Salts the choice of the new one among those it could be.
const WHICH_SALT: u64 = 0x7768_6963_685f_3f21;

/// splitmix64's finaliser.
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// One of her rare scripts: its tier, the stable id her ledger records it
/// by once she has shown it (part of the format: it never changes), and
/// the parts of her day it can play in (a draw makes it new only for a
/// window it can happen in).
#[derive(Clone, Copy, Debug)]
pub(super) struct RareRow {
    pub id: ScriptId,
    pub rarity: Rarity,
    pub key: &'static str,
    pub slots: SlotSet,
}

/// Every rare script of hers (a lint holds it to [`ScriptId::rarity`]:
/// every gated script is here once, at its tier).
pub(super) const RARES: [RareRow; 4] = [
    RareRow {
        id: ScriptId::Dream,
        rarity: Rarity::Rare,
        key: "dream",
        slots: SlotSet::of(&[Slot::Asleep]),
    },
    RareRow {
        id: ScriptId::NoMelon,
        rarity: Rarity::Rare,
        key: "no-melon",
        slots: SlotSet::of(&[Slot::Morning, Slot::Afternoon, Slot::Evening]),
    },
    RareRow {
        id: ScriptId::Escalator,
        rarity: Rarity::Rare,
        key: "escalator",
        slots: SlotSet::of(&[
            Slot::Morning,
            Slot::Afternoon,
            Slot::Evening,
            Slot::Homework,
        ]),
    },
    RareRow {
        id: ScriptId::Scary,
        rarity: Rarity::Rare,
        key: "scary",
        slots: SlotSet::of(&[Slot::Evening, Slot::Homework]),
    },
];

/// The stable id her ledger records `id` by, if it's one of her rare
/// scripts.
pub(super) fn key(id: ScriptId) -> Option<&'static str> {
    RARES.iter().find(|r| r.id == id).map(|r| r.key)
}

/// The rare script her ledger recorded as `key` (`None`: not one this
/// build knows).
pub(super) fn by_key(key: &str) -> Option<ScriptId> {
    RARES.iter().find(|r| r.key == key).map(|r| r.id)
}

/// The window of her day a visit's draw is for (her routine fed): the
/// part of her day it begins in, and the next (a visit is short).
pub(super) fn visit_window(start: Slot, next: Slot) -> SlotSet {
    SlotSet::of(&[start, next])
}

/// The window of her day a new day's draw is for: all of it, to the next
/// morning (the day is the unit; a resident's whole day hangs on it).
pub(super) const DAY_WINDOW: SlotSet = SlotSet::of(&Slot::ALL);

/// The window of a visit's draw with her routine not reaching her: she
/// lives an endless afternoon (the afternoon is what an unfed mind is,
/// arriving), so nothing that needs the evening or the night is new.
pub(super) const UNFED_WINDOW: SlotSet = SlotSet::of(&[Slot::Afternoon]);

/// What of hers that's rare is open now (her day's, or unfed her
/// visit's): the seen rare scripts whose tier passed the draw, and at
/// most one she hasn't shown yet (the `Option` makes two unrepresentable).
/// Built only by [`Rares::draw`] and [`Rares::none`] (its fields private,
/// no `Default`), so it's always a draw over [`RARES`] or nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Rares {
    open: Vec<ScriptId>,
    new: Option<ScriptId>,
}

impl Rares {
    /// Nothing rare open (before any draw; tests).
    pub fn none() -> Self {
        Self {
            open: Vec::new(),
            new: None,
        }
    }

    /// The draw seeded `seed` (the game day's, or unfed the visit's: it's
    /// salted here), with `seen` what she has shown, her `pity` counters,
    /// for `window` of her day.
    pub fn draw(seed: u64, seen: &[ScriptId], pity: Pity, window: SlotSet) -> Self {
        Self::draw_from(&RARES, seed, seen, pity, window)
    }

    /// [`Rares::draw`] over `rows` (tests try Legendary rows of their own:
    /// none ships). Each tier rolls once: its seen scripts are open if it
    /// passes. The new one is chosen from the first tier that passes,
    /// Legendary before Rare, with an unseen script whose slots meet the
    /// window, evenly among those. Private: a row whose tier isn't its
    /// script's would make [`Rares::allows`] disagree with the draw.
    fn draw_from(
        rows: &[RareRow],
        seed: u64,
        seen: &[ScriptId],
        pity: Pity,
        window: SlotSet,
    ) -> Self {
        let z = mix(seed ^ RARE_SALT);
        let pass = |tier: Rarity| passes(z, tier, pity.of_tier(tier));
        let open = rows
            .iter()
            .filter(|r| seen.contains(&r.id) && pass(r.rarity))
            .map(|r| r.id)
            .collect();
        let new = [Rarity::Legendary, Rarity::Rare]
            .into_iter()
            .filter(|&tier| pass(tier))
            .find_map(|tier| {
                let fresh: Vec<ScriptId> = rows
                    .iter()
                    .filter(|r| r.rarity == tier && !seen.contains(&r.id) && r.slots.meets(window))
                    .map(|r| r.id)
                    .collect();
                let n = fresh.len() as u64;
                let which = mix(z ^ WHICH_SALT ^ tier.salt()).checked_rem(n)?;
                fresh.get(usize::try_from(which).ok()?).copied()
            });
        Self { open, new }
    }

    /// Whether `id` may play: anything ungated; a gated script only open
    /// or new.
    pub fn allows(&self, id: ScriptId) -> bool {
        !id.rarity().gated() || self.open.contains(&id) || self.new == Some(id)
    }

    /// The seen rare scripts open (tests).
    #[cfg(test)]
    pub fn open(&self) -> &[ScriptId] {
        &self.open
    }

    /// The unseen one, if any (tests).
    #[cfg(test)]
    pub fn new_one(&self) -> Option<ScriptId> {
        self.new
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Pity is a pure function of (seed, counter): for every seed, once
    /// a tier passes it keeps passing as the counter rises; certain at
    /// the bound, and its chance below 1 short of it.
    #[test]
    fn pity_is_monotone_and_certain_at_its_bound() {
        for tier in [Rarity::Rare, Rarity::Legendary] {
            let (_, bound) = tier.odds().unwrap();
            for seed in 0..500u64 {
                let seed = mix(seed);
                for counter in [0, 1, bound / 2, bound - 1, bound, bound + 1, u64::MAX] {
                    let now = passes(seed, tier, counter);
                    assert_eq!(now, passes(seed, tier, counter), "pure");
                    if counter >= bound {
                        assert!(now, "{tier:?} seed {seed}: not certain at {counter}");
                    }
                }
                // Monotone: from the first counter it passes at, it
                // passes at every higher one.
                let first = (0..=bound).find(|&c| passes(seed, tier, c)).unwrap();
                for c in first..=bound + 5 {
                    assert!(
                        passes(seed, tier, c),
                        "{tier:?} seed {seed}: {first} then {c}"
                    );
                }
            }
            // The chance rises, and is certain from the bound on.
            for c in 0..bound {
                assert!(chance(tier, c) <= chance(tier, c + 1), "{tier:?} at {c}");
                assert!(chance(tier, c) < 1.0, "{tier:?} at {c}");
            }
            assert_eq!(chance(tier, bound), 1.0);
            assert_eq!(chance(tier, u64::MAX), 1.0);
        }
        // Ungated tiers always pass.
        for tier in [Rarity::Common, Rarity::Uncommon] {
            assert!(passes(1, tier, 0) && chance(tier, 0) == 1.0);
        }
    }

    /// Her pity reads a counter set ahead of her idle minutes (a garbled
    /// record's) as no pity yet, never underflowing; and the far end of
    /// either as it is.
    #[test]
    fn pity_saturates() {
        assert_eq!(Pity::of(5, 10, 3000), Pity { rare: 0, legend: 0 });
        assert_eq!(
            Pity::of(u64::MAX, 0, u64::MAX),
            Pity {
                rare: u64::MAX,
                legend: 0
            }
        );
        assert_eq!(Pity::of(0, u64::MAX, u64::MAX), Pity::default());
        assert_eq!(
            Pity::of(400, 40, 0),
            Pity {
                rare: 360,
                legend: 400
            }
        );
    }

    /// With no pity, each tier passes about its base rate over fixed
    /// seeds (a Rare day in seven or so, a Legendary in fifty); at half
    /// the bound, about halfway to certain.
    #[test]
    fn pity_passes_at_its_base_rate() {
        const N: u64 = 20_000;
        let rate = |tier, counter| {
            (0..N).filter(|&s| passes(mix(s), tier, counter)).count() as f64 / N as f64
        };
        let near = |got: f64, want: f64, tol: f64| (got - want).abs() < tol;
        let rare = rate(Rarity::Rare, 0);
        assert!(near(rare, 0.15, 0.01), "Rare: {rare}");
        let legend = rate(Rarity::Legendary, 0);
        assert!(near(legend, 0.02, 0.004), "Legendary: {legend}");
        let half = rate(Rarity::Rare, 180);
        assert!(near(half, 0.575, 0.015), "Rare at 180: {half}");
    }

    /// Every seen subset of her rares, as a list in table order.
    fn subsets(rows: &[RareRow]) -> Vec<Vec<ScriptId>> {
        (0..1u32 << rows.len())
            .map(|bits| {
                rows.iter()
                    .enumerate()
                    .filter(|&(i, _)| bits & 1 << i != 0)
                    .map(|(_, r)| r.id)
                    .collect()
            })
            .collect()
    }

    /// Over a thousand seeds and every set of seen rares, pity and
    /// window: the new one is never one she has seen, is one the window
    /// can hold, and the open ones are all seen; with no pity the draw
    /// opens something on about the Rare tier's base rate of days, and
    /// with full pity on every one.
    #[test]
    fn a_draw_opens_only_the_seen_and_news_only_the_unseen() {
        let windows = [
            DAY_WINDOW,
            UNFED_WINDOW,
            visit_window(Slot::Afternoon, Slot::Evening),
            visit_window(Slot::Homework, Slot::Asleep),
            visit_window(Slot::Asleep, Slot::Morning),
        ];
        let mut news = 0;
        for seen in subsets(&RARES) {
            for pity in [
                Pity::default(),
                Pity {
                    rare: 200,
                    legend: 0,
                },
                Pity {
                    rare: 360,
                    legend: 2400,
                },
            ] {
                for &window in &windows {
                    for seed in 0..1000u64 {
                        let rares = Rares::draw(seed, &seen, pity, window);
                        assert!(rares.open.iter().all(|id| seen.contains(id)), "{rares:?}");
                        if let Some(new) = rares.new {
                            assert!(!seen.contains(&new), "{new:?} seen: {seen:?}");
                            let row = RARES.iter().find(|r| r.id == new).unwrap();
                            assert!(row.slots.meets(window), "{new:?} out of its window");
                            news += 1;
                        }
                        // Certain at the bound: something unseen that
                        // fits is new, and every seen one is open.
                        if pity.rare >= 360 {
                            assert_eq!(rares.open.len(), seen.len(), "{seed}");
                            let fits = RARES
                                .iter()
                                .any(|r| !seen.contains(&r.id) && r.slots.meets(window));
                            assert_eq!(rares.new.is_some(), fits, "{seed} {seen:?}");
                        }
                        // Ungated scripts are always allowed; gated ones
                        // only as drawn.
                        assert!(rares.allows(ScriptId::Snack));
                        for r in RARES {
                            assert_eq!(
                                rares.allows(r.id),
                                rares.open.contains(&r.id) || rares.new == Some(r.id)
                            );
                        }
                    }
                }
            }
        }
        assert!(news > 0);
    }

    /// Nothing rare is open with none drawn: not one of her rare scripts
    /// is allowed, and everything else is.
    #[test]
    fn none_opens_nothing() {
        let none = Rares::none();
        for r in RARES {
            assert!(!none.allows(r.id), "{:?}", r.id);
        }
        assert!(none.allows(ScriptId::Andagi) && none.allows(ScriptId::Night));
        assert_eq!(none.new, None);
        assert!(none.open.is_empty());
    }

    /// The Legendary tier (no row ships one: these are the tests' own,
    /// stood in by existing scripts): it rolls apart from Rare, at its own
    /// base rate and bound, its new one chosen before a Rare's, still at
    /// most one; and a seen Legendary opens only when its own tier passes.
    #[test]
    fn the_legendary_tier_rolls_on_its_own() {
        let all = SlotSet::of(&Slot::ALL);
        let rows = [
            RareRow {
                id: ScriptId::Lounge,
                rarity: Rarity::Legendary,
                key: "test-legend",
                slots: all,
            },
            RareRow {
                id: ScriptId::Read,
                rarity: Rarity::Rare,
                key: "test-rare",
                slots: all,
            },
        ];
        let (mut legend_new, mut rare_new) = (0, 0);
        for seed in 0..5000u64 {
            let d = Rares::draw_from(&rows, seed, &[], Pity::default(), all);
            match d.new {
                Some(ScriptId::Lounge) => legend_new += 1,
                Some(ScriptId::Read) => rare_new += 1,
                other => assert_eq!(other, None),
            }
            // Legendary's pity alone makes it certain, and new before the
            // Rare one.
            let sure = Rares::draw_from(
                &rows,
                seed,
                &[],
                Pity {
                    rare: 0,
                    legend: 2400,
                },
                all,
            );
            assert_eq!(sure.new, Some(ScriptId::Lounge), "{seed}");
            // Seen, it opens with its tier's roll alone.
            let seen = [ScriptId::Lounge];
            let open = Rares::draw_from(
                &rows,
                seed,
                &seen,
                Pity {
                    rare: 360,
                    legend: 0,
                },
                all,
            );
            let z = mix(seed ^ RARE_SALT);
            assert_eq!(
                open.open.contains(&ScriptId::Lounge),
                passes(z, Rarity::Legendary, 0),
                "{seed}"
            );
            assert_eq!(
                open.new,
                Some(ScriptId::Read),
                "{seed}: the Rare one, certain"
            );
        }
        assert!(
            (60..150).contains(&legend_new),
            "{legend_new} legendary of 5000"
        );
        assert!((600..900).contains(&rare_new), "{rare_new} rare of 5000");
    }

    /// Lint: every script of hers that's gated is a row here, once, at
    /// its tier; every row's id is a distinct stable id that reads back
    /// as its script, and fits somewhere in her day.
    #[test]
    fn every_gated_script_is_a_row() {
        for id in ScriptId::ALL {
            let rows: Vec<&RareRow> = RARES.iter().filter(|r| r.id == id).collect();
            if id.rarity().gated() {
                assert_eq!(rows.len(), 1, "{id:?}");
                assert_eq!(rows[0].rarity, id.rarity(), "{id:?}");
            } else {
                assert!(rows.is_empty(), "{id:?}: ungated, but a row");
            }
        }
        let mut keys = std::collections::HashSet::new();
        for r in RARES {
            assert!(keys.insert(r.key), "{}: twice", r.key);
            assert_eq!(by_key(r.key), Some(r.id));
            assert_eq!(key(r.id), Some(r.key));
            assert!(r.slots.meets(DAY_WINDOW), "{:?}: never", r.id);
        }
        assert_eq!(by_key("melon"), None);
        assert_eq!(key(ScriptId::Snack), None);
    }
}
