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
use super::room::{Furniture, Use};
use super::routine::{self, DateWindow, DayTime, Slot, SlotSet, When};
use chrono::NaiveDate;

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
    /// Rises only while a rule of her home she has felt is broken (see
    /// [`Rising::grieved`]); putting it right will answer it.
    Nesting,
    /// Rises only while she's in a plain room (see [`Rising::plain`]);
    /// resting and using her things in a pretty one ease it.
    Beauty,
}

impl Need {
    pub const ALL: [Need; 10] = [
        Self::Sleepy,
        Self::Restless,
        Self::Tidy,
        Self::Mischief,
        Self::Hungry,
        Self::Comfort,
        Self::Fun,
        Self::Daydreams,
        Self::Nesting,
        Self::Beauty,
    ];

    /// Milliseconds to rise from 0 to 1 (tidy only while there's text on
    /// offer to tidy, nesting only while a felt rule is broken, beauty
    /// only in a plain room).
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
            // A starting value, to tune in the visit census.
            Self::Nesting => 3.0 * 60_000.0,
            Self::Beauty => 20.0 * 60_000.0,
        }
    }

    /// Where she starts a visit: wide awake, keen to move and to look
    /// around, the rest about halfway, so no want is starved at arrival.
    /// Nothing about her home bothers her yet, though a plain room a
    /// little. (The afternoon's levels: see [`Need::arriving_in`].)
    fn arriving(self) -> f64 {
        match self {
            Self::Sleepy | Self::Nesting => 0.0,
            Self::Restless => 0.7,
            Self::Tidy | Self::Comfort | Self::Fun | Self::Daydreams => 0.5,
            Self::Mischief | Self::Hungry => 0.2,
            Self::Beauty => 0.3,
        }
    }

    /// Where she starts a visit in `slot` of her day (D4 lever 3): the
    /// afternoon's are [`Need::arriving`]'s exactly; later in the day
    /// she comes sleepier, at mealtimes (breakfast, dinner) hungrier.
    fn arriving_in(self, slot: Slot) -> f64 {
        match (self, slot) {
            (Self::Sleepy, Slot::Evening) => 0.3,
            (Self::Sleepy, Slot::Homework) => 0.5,
            (Self::Sleepy, Slot::Asleep) => 0.8,
            (Self::Hungry, Slot::Morning | Slot::Evening) => 0.6,
            _ => self.arriving(),
        }
    }

    /// How much faster than usual `self` rises in `slot` of her day (D4
    /// lever 2): sleepiness slowly by day, quickly at homework and past
    /// bedtime; hunger quickly at breakfast and dinner, so meals emerge
    /// without a want of their own.
    fn rate_in(self, slot: Slot) -> f64 {
        match (self, slot) {
            (Self::Sleepy, Slot::Morning | Slot::Away | Slot::Afternoon) => SLEEPY_BY_DAY,
            (Self::Sleepy, Slot::Homework | Slot::Asleep) => SLEEPY_AT_NIGHT,
            (Self::Hungry, Slot::Morning | Slot::Evening) => HUNGRY_AT_MEALS,
            _ => 1.0,
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
            Self::Nesting => "nesting",
            Self::Beauty => "beauty",
        }
    }
}

/// Sleepiness rises this much as fast by day (the morning, school, the
/// afternoon).
const SLEEPY_BY_DAY: f64 = 0.3;
/// Sleepiness rises this much as fast at homework and past bedtime.
const SLEEPY_AT_NIGHT: f64 = 3.0;
/// Hunger rises this much as fast at breakfast and dinner.
const HUNGRY_AT_MEALS: f64 = 2.0;
/// A need never rises more than this much faster than usual, her mood's
/// rate and her day's together.
const RATE_MAX: f64 = 4.0;

/// How much faster than usual `need` rises at her routine's `slot`
/// (`None`: no routine reaches her, and exactly 1).
pub(super) fn clock_rate(slot: Option<Slot>, need: Need) -> f64 {
    slot.map_or(1.0, |slot| need.rate_in(slot))
}

/// Her mood for the visit: she has a life outside dessplay. A mood is how
/// fast her needs rise, and how she says hello.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mood {
    Ordinary,
    /// Comfort and sleep come quicker, restlessness slower.
    Lazy,
    /// Tidying, moving about and putting her home right come quicker,
    /// comfort slower.
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
            (Self::Industrious, Need::Nesting) => 1.6,
            (Self::Dreamy, Need::Daydreams) => 2.5,
            (Self::Dreamy, Need::Restless) => 0.7,
            _ => 1.0,
        }
    }

    /// How many things about her home she sets right a visit, at most
    /// (the user's call: lazy none, industrious a few).
    pub fn home_acts(self) -> u8 {
        match self {
            Self::Lazy => 0,
            Self::Ordinary | Self::Dreamy => 1,
            Self::Industrious => 3,
        }
    }

    /// The next mood round (the stage cycles them).
    pub fn next(self) -> Self {
        match self {
            Self::Ordinary => Self::Lazy,
            Self::Lazy => Self::Industrious,
            Self::Industrious => Self::Dreamy,
            Self::Dreamy => Self::Ordinary,
        }
    }

    /// What she says on first finding her feet: a hint at her mood.
    pub fn greeting(self) -> &'static str {
        match self {
            Self::Ordinary => line!("Nice to meet you."),
            Self::Lazy => line!("Mm... lazy day."),
            Self::Industrious => line!("Okay! Let's tidy up!"),
            Self::Dreamy => line!("...hm? Oh, hello."),
        }
    }
}

/// Where a want would have her, as far as her needs care.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Spot {
    /// On (or at) a real piece of her furniture.
    Real(Furniture),
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
        (Need::Comfort | Need::Sleepy, Spot::Real(item)) => item.spec().comfort,
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

/// What there was, over a stretch of her visit, for the needs that rise
/// only with something to rise for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Rising {
    /// There was text on offer to tidy ([`Need::Tidy`]).
    pub mess: bool,
    /// A rule of her home she has felt this visit was still broken
    /// ([`Need::Nesting`]).
    pub grieved: bool,
    /// She was in a plain room: on a strip with nothing pretty on it, or
    /// on none ([`Need::Beauty`]).
    pub plain: bool,
    /// Of the stretch, how long she slept the night (A11): sleepiness
    /// doesn't rise for it, and the rest at a quarter of their pace.
    pub slept_ms: u64,
}

impl Rising {
    /// Everything rising (tests).
    #[cfg(test)]
    pub const ALL: Self = Self {
        mess: true,
        grieved: true,
        plain: true,
        slept_ms: 0,
    };
}

/// Asleep, her needs (but sleepiness) rise at this pace.
const ASLEEP_PACE: f64 = 0.25;

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

    /// Her needs as she arrives at `slot` of her day (see
    /// [`Need::arriving_in`]); nothing worn thin.
    pub fn arriving_in(slot: Slot) -> Self {
        Self {
            levels: Need::ALL.map(|need| need.arriving_in(slot)),
            tolerance: [0.0; FUN_SOURCES.len()],
        }
    }

    /// `ms` have passed in `mood`, with what was `rising` meanwhile, each
    /// need rising `rate(need)` times as fast as usual (her routine's:
    /// see [`clock_rate`]); with her mood's, never more than
    /// [`RATE_MAX`] times.
    pub fn pass(&mut self, ms: u64, rising: Rising, mood: Mood, rate: impl Fn(Need) -> f64) {
        // Asleep, sleepiness doesn't rise, and the rest at a quarter of
        // their pace. (For every stretch but the night's, nothing is
        // slept, and the span is exactly `ms`.)
        let slept = rising.slept_ms.min(ms);
        let awake = ms - slept;
        for need in Need::ALL {
            let rises = match need {
                Need::Tidy => rising.mess,
                Need::Nesting => rising.grieved,
                Need::Beauty => rising.plain,
                _ => true,
            };
            if rises {
                let span = if need == Need::Sleepy {
                    awake as f64
                } else {
                    awake as f64 + slept as f64 * ASLEEP_PACE
                };
                let pace = (mood.rate(need) * rate(need)).min(RATE_MAX);
                self.levels[need as usize] += span * pace / need.rise_ms();
            }
        }
        for tolerance in &mut self.tolerance {
            *tolerance = (*tolerance - ms as f64 / TOLERANCE_MS).max(0.0);
        }
        self.clamp();
    }

    /// Whether `need` is the most pressing of her needs: felt, and none
    /// higher (a tie with another at the top counts).
    pub fn pressing(&self, need: Need) -> bool {
        let level = self.get(need);
        level > 0.0 && Need::ALL.iter().all(|&other| self.get(other) <= level)
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
    /// Put right a rule of her home she has felt broken: lift a piece,
    /// carry it, and set it down where it's right.
    Arrange,
}

/// Something that makes a want more or less likely, beyond her needs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Factor {
    /// Where it would take her is in the chat pane (she's resident, and
    /// people read there): this much as likely (below 1).
    InChat(f64),
    /// At this time of her day (D4 lever 1): this much as likely (above
    /// 1: a boost, never a gate; below 1 would delete the want, since
    /// [`choose`] keeps only the top few).
    Clock(When, f64),
    /// In this real-date window: this much as likely (above 1).
    Season(DateWindow, f64),
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
/// The chat factor, alone or with others.
const CHAT: Factor = Factor::InChat(super::osaka::CHAT_FACTOR);

/// A boost by her day: the usual one.
pub(super) const BOOST: f64 = 2.0;
/// A boost by her day: what the time is for (homework at homework time,
/// sneezing in hay fever).
pub(super) const BOOST_STRONG: f64 = 3.0;

/// A snack, the sofa: the afternoon's.
const AFTERNOON_FACTORS: &[Factor] = &[
    CHAT,
    Factor::Clock(When::In(SlotSet::of(&[Slot::Afternoon])), BOOST),
];
/// The evening is the TV's.
const WATCH_FACTORS: &[Factor] = &[
    CHAT,
    Factor::Clock(When::In(SlotSet::of(&[Slot::Evening])), BOOST),
];
/// Homework on a school night (its slot), in exam season, and in
/// summer's panic week.
const HOMEWORK_FACTORS: &[Factor] = &[
    CHAT,
    Factor::Clock(When::In(SlotSet::of(&[Slot::Homework])), BOOST_STRONG),
    Factor::Season(routine::EXAMS, BOOST),
    Factor::Season(routine::PANIC_WEEK, BOOST),
];
/// A book on an evening before a day off.
const READ_FACTORS: &[Factor] = &[CHAT, Factor::Clock(When::EveningOff, BOOST)];
/// A stretch in the morning.
const STRETCH_FACTORS: &[Factor] = &[Factor::Clock(
    When::In(SlotSet::of(&[Slot::Morning])),
    BOOST,
)];
/// Gazing at dusk and through the night.
const GAZE_FACTORS: &[Factor] = &[Factor::Clock(routine::DUSK_TO_DAWN, BOOST)];
/// Hay fever.
const SNEEZE_FACTORS: &[Factor] = &[Factor::Season(routine::HAY_FEVER, BOOST_STRONG)];

/// `def` with `factors` (all of them: [`in_chat`]'s too, if it should
/// keep it).
const fn with(def: DesireDef, factors: &'static [Factor]) -> DesireDef {
    DesireDef { factors, ..def }
}

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
    pub const ALL: [Want; 26] = [
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
        Self::Arrange,
    ];

    /// Its row of the table.
    pub fn def(self) -> DesireDef {
        match self {
            // The one filler: what she does when nothing else binds.
            Self::Stand => row(4.0, &[]),
            Self::SpaceOut => row(6.0, &[(Need::Daydreams, 0.6)]),
            // An accident, not a want of anything (mischief would pin it
            // where there's nothing to swap).
            Self::Sneeze => with(row(2.0, &[]), SNEEZE_FACTORS),
            Self::Idle(Activity::Stretch) => {
                with(row(4.0, &[(Need::Restless, 0.5)]), STRETCH_FACTORS)
            }
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
            Self::Idle(Activity::Gaze) => with(row(6.0, &[(Need::Daydreams, 0.5)]), GAZE_FACTORS),
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
            Self::Use(Use::Watch) => with(row(10.0, &[(Need::Fun, 0.6)]), WATCH_FACTORS),
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
            Self::Use(Use::Lounge) => with(row(8.0, &[(Need::Comfort, 0.5)]), AFTERNOON_FACTORS),
            // At her desk she drifts off.
            Self::Use(Use::Homework) => with(
                row(8.0, &[(Need::Daydreams, 0.3), (Need::Comfort, 0.2)]),
                HOMEWORK_FACTORS,
            ),
            Self::Use(Use::Read) => with(
                row(8.0, &[(Need::Fun, 0.5), (Need::Daydreams, 0.2)]),
                READ_FACTORS,
            ),
            Self::Use(Use::Snack) => with(
                row(8.0, &[(Need::Hungry, 0.8), (Need::Fun, 0.1)]),
                AFTERNOON_FACTORS,
            ),
            Self::Use(Use::Pet) => in_chat(row(8.0, &[(Need::Fun, 0.6)])),
            // Only on offer while a rule she has felt is broken and her
            // mood leaves her something to do about it: nesting is all
            // it answers.
            Self::Arrange => row(6.0, &[(Need::Nesting, 1.0)]),
        }
    }
}

/// How much likelier `want` is for its factors: `into_chat` (where it
/// would take her is in the chat pane), at her routine's `day`, on the
/// real `date`. Without a day or a date, exactly as likely as without
/// her routine or the calendar: each such factor is 1.
pub(super) fn factor(
    want: Want,
    into_chat: bool,
    day: Option<&DayTime>,
    date: Option<NaiveDate>,
) -> f64 {
    want.def()
        .factors
        .iter()
        .map(|&factor| match factor {
            Factor::InChat(times) if into_chat => times,
            Factor::InChat(_) => 1.0,
            Factor::Clock(when, times) if day.is_some_and(|day| when.holds(day)) => times,
            Factor::Clock(..) => 1.0,
            Factor::Season(window, times)
                if date.is_some_and(|date| routine::within(date, window)) =>
            {
                times
            }
            Factor::Season(..) => 1.0,
        })
        .product()
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
                    Want::Use(what) => Spot::Real(
                        Furniture::ALL
                            .into_iter()
                            .find(|f| f.spec().uses.contains(&what))
                            .unwrap_or(Furniture::Sofa),
                    ),
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
            Want::Arrange => 10,
        };
        for k in 0..11 {
            assert!(Want::ALL.iter().any(|&w| kind(w) == k), "kind {k} missing");
        }
        for a in Activity::ALL {
            assert!(Want::ALL.contains(&Want::Idle(a)), "{a:?}");
        }
        for furniture in Furniture::ALL {
            for &what in furniture.spec().uses {
                assert!(Want::ALL.contains(&Want::Use(what)), "{what:?}");
            }
        }
        assert!(Want::ALL.contains(&Want::Use(Use::Crumple)));
        for (i, a) in Want::ALL.iter().enumerate() {
            assert!(!Want::ALL[..i].contains(a), "{a:?} twice");
        }
    }

    /// A factor the table may hold: the chat's makes a want less likely
    /// (but possible); her day's and the season's make one more likely,
    /// never less (a factor below 1 deletes a want: [`choose`] keeps
    /// only the top few).
    fn sane_factor(factor: Factor) -> bool {
        match factor {
            Factor::InChat(times) => times > 0.0 && times < 1.0,
            Factor::Clock(_, times) | Factor::Season(_, times) => times > 1.0 && times.is_finite(),
        }
    }

    /// The lint's predicate rejects a boost that isn't one: her day's or
    /// the season's at 1 or below, the chat's at 1 or above.
    #[test]
    fn the_lint_rejects_a_boost_that_is_not_one() {
        let evening = When::In(SlotSet::of(&[Slot::Evening]));
        for times in [1.0, 0.5, 0.0, -2.0, f64::NAN, f64::INFINITY] {
            assert!(!sane_factor(Factor::Clock(evening, times)), "{times}");
            assert!(
                !sane_factor(Factor::Season(routine::EXAMS, times)),
                "{times}"
            );
        }
        assert!(sane_factor(Factor::Clock(evening, 1.5)));
        assert!(sane_factor(Factor::Season(routine::EXAMS, 3.0)));
        for times in [1.0, 2.0, 0.0] {
            assert!(!sane_factor(Factor::InChat(times)), "{times}");
        }
        assert!(sane_factor(Factor::InChat(0.1)));
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
            for &factor in def.factors {
                assert!(sane_factor(factor), "{want:?}: {factor:?}");
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
        let fresh = score(tv, Spot::Real(Furniture::Tv), &needs, &[]);
        needs.enjoyed(tv, 1.0);
        needs.enjoyed(tv, 1.0);
        assert!(score(tv, Spot::Real(Furniture::Tv), &needs, &[]) < fresh * 0.5);
        assert_eq!(needs.fresh(book), 1.0);
        needs.pass(10 * 60_000, Rising::default(), Mood::Ordinary, |_| 1.0);
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
    }

    #[test]
    fn needs_stay_in_range() {
        let mut needs = Needs::default();
        needs.pass(10 * 3_600_000, Rising::ALL, Mood::Ordinary, |_| 1.0);
        assert!(Need::ALL.iter().all(|&need| needs.get(need) == 1.0));
        needs.serve(Need::Sleepy, 5.0);
        assert_eq!(needs.get(Need::Sleepy), 0.0);
    }

    /// Nothing about her home bothers her as she arrives; nesting rises
    /// only while a rule she has felt is broken, and quicker when she's
    /// industrious. Tidiness waits for a mess likewise.
    #[test]
    fn nesting_rises_only_while_grieved() {
        let mut needs = Needs::default();
        assert_eq!(needs.get(Need::Nesting), 0.0);
        needs.pass(3_600_000, Rising::default(), Mood::Industrious, |_| 1.0);
        assert_eq!(needs.get(Need::Nesting), 0.0);
        let tidy = needs.get(Need::Tidy);
        let grieved = Rising {
            grieved: true,
            ..Rising::default()
        };
        let minute = |mood: Mood| {
            let mut needs = Needs::default();
            needs.pass(60_000, grieved, mood, |_| 1.0);
            needs.get(Need::Nesting)
        };
        assert!(minute(Mood::Ordinary) > 0.2, "{}", minute(Mood::Ordinary));
        assert!(minute(Mood::Industrious) > minute(Mood::Ordinary) * 1.5);
        needs.pass(60_000, grieved, Mood::Ordinary, |_| 1.0);
        assert!(needs.get(Need::Nesting) > 0.0);
        assert_eq!(needs.get(Need::Tidy), tidy, "no mess, no tidying");
    }

    /// A plain room bothers her a little as she arrives; her want of
    /// beauty rises only while she's in one (over twenty minutes), and
    /// no want answers it (resting in a pretty room does).
    #[test]
    fn beauty_rises_only_in_a_plain_room() {
        let mut needs = Needs::default();
        assert_eq!(needs.get(Need::Beauty), 0.3);
        needs.pass(3_600_000, Rising::default(), Mood::Ordinary, |_| 1.0);
        assert_eq!(needs.get(Need::Beauty), 0.3, "in a pretty room");
        let plain = Rising {
            plain: true,
            ..Rising::default()
        };
        needs.pass(10 * 60_000, plain, Mood::Ordinary, |_| 1.0);
        assert!((needs.get(Need::Beauty) - 0.8).abs() < 1e-9);
        assert!(
            Want::ALL
                .iter()
                .all(|w| w.def().serves.iter().all(|&(need, _)| need != Need::Beauty))
        );
    }

    /// The most pressing need: felt, and none higher (a tie at the top
    /// counts).
    #[test]
    fn the_most_pressing_need() {
        let needs = Needs::with(&[(Need::Beauty, 0.6), (Need::Fun, 0.5)]);
        assert!(needs.pressing(Need::Beauty));
        assert!(!needs.pressing(Need::Fun));
        let tie = Needs::with(&[(Need::Beauty, 1.0), (Need::Restless, 1.0)]);
        assert!(tie.pressing(Need::Beauty) && tie.pressing(Need::Restless));
        assert!(!Needs::with(&[]).pressing(Need::Beauty), "felt at all");
    }

    /// Every want that takes her into the chat a tenth as likely, as
    /// written out before her day's boosts gave some of them rows of
    /// their own (whose slices must keep the chat's: [`in_chat`] would
    /// overwrite them).
    const CHAT_WANTS: [Want; 13] = [
        Want::Travel,
        Want::Pull,
        Want::Swap,
        Want::Use(Use::Watch),
        Want::Use(Use::Sleep),
        Want::Use(Use::Unpack),
        Want::Use(Use::Crumple),
        Want::Use(Use::Nap),
        Want::Use(Use::Lounge),
        Want::Use(Use::Homework),
        Want::Use(Use::Read),
        Want::Use(Use::Snack),
        Want::Use(Use::Pet),
    ];

    /// The chat's factor for `want`, pinned (not read off the table).
    fn chat_of(want: Want) -> f64 {
        if CHAT_WANTS.contains(&want) {
            super::super::osaka::CHAT_FACTOR
        } else {
            1.0
        }
    }

    /// Each want the chat made a tenth as likely still is, whatever her
    /// day and the date boost, and no other: a boosted row keeps the
    /// chat's factor beside its own.
    #[test]
    fn the_chat_keeps_its_factor() {
        let days = [
            None,
            Some(day_at(7, 16 * 60, false)),
            Some(day_at(7, 21 * 60, false)),
        ];
        let dates = [None, ymd(2027, 3, 5)];
        for want in Want::ALL {
            assert_eq!(factor(want, true, None, None), chat_of(want), "{want:?}");
            for day in &days {
                for &date in &dates {
                    let out = factor(want, false, day.as_ref(), date);
                    let into = factor(want, true, day.as_ref(), date);
                    assert_eq!(into, out * chat_of(want), "{want:?} {day:?} {date:?}");
                }
            }
        }
        for want in CHAT_WANTS {
            assert!(Want::ALL.contains(&want), "{want:?}");
        }
    }

    /// Her routine at `minute` (since midnight) of game day `day`, with
    /// the day's vacation flag.
    fn day_at(day: u64, minute: u64, vacation: bool) -> DayTime {
        routine::day_time((day * 1440 + minute - routine::START) * 60_000, vacation)
    }

    fn ymd(y: i32, m: u32, d: u32) -> Option<NaiveDate> {
        NaiveDate::from_ymd_opt(y, m, d)
    }

    /// Her day's boosts (D4 lever 1), each where the table puts it:
    /// above 1 exactly there, exactly 1 everywhere else, and exactly 1
    /// for every want without a day or a date. Checked every quarter
    /// hour of a school week and a vacation week.
    #[test]
    fn a_boost_raises_its_want_only_in_its_time() {
        let homework = Want::Use(Use::Homework);
        let read = Want::Use(Use::Read);
        let expected = |want: Want, day: &DayTime| -> bool {
            match want {
                Want::Use(Use::Snack | Use::Lounge) => day.slot == Slot::Afternoon,
                Want::Use(Use::Watch) => day.slot == Slot::Evening,
                Want::Use(Use::Homework) => day.slot == Slot::Homework,
                Want::Use(Use::Read) => day.slot == Slot::Evening && !day.night_before_school,
                Want::Idle(Activity::Stretch) => day.slot == Slot::Morning,
                Want::Idle(Activity::Gaze) => day.minute >= 17 * 60 || day.minute < 5 * 60,
                _ => false,
            }
        };
        let mut seen = std::collections::HashSet::new();
        for vacation in [false, true] {
            for day in 7..14 {
                for minute in (0..1440).step_by(15) {
                    let now = day_at(day, minute, vacation);
                    for want in Want::ALL {
                        assert_eq!(factor(want, false, None, None), 1.0, "{want:?}");
                        let times = factor(want, false, Some(&now), None);
                        if expected(want, &now) {
                            assert!(times > 1.0, "{want:?} at {now}");
                            seen.insert(want);
                        } else {
                            assert_eq!(times, 1.0, "{want:?} at {now}");
                        }
                        // In the chat, the boost is on top of the chat's
                        // (a row's own slice keeps it: see `chat_of`).
                        let in_chat = factor(want, true, Some(&now), None);
                        assert_eq!(in_chat, chat_of(want) * times, "{want:?} at {now}");
                    }
                }
            }
        }
        assert_eq!(seen.len(), 7, "{seen:?}");
        // Read's evening is before a day off: Friday's, not Thursday's.
        let thursday = day_at(10, 19 * 60, false);
        let friday = day_at(11, 19 * 60, false);
        assert_eq!(factor(read, false, Some(&thursday), None), 1.0);
        assert!(factor(read, false, Some(&friday), None) > 1.0);
        // The seasons: exams and panic week for homework, hay fever for
        // sneezing; each only on its dates, and none without a date.
        let afternoon = day_at(8, 16 * 60, false);
        for (date, homework_up, sneeze_up) in [
            (ymd(2027, 1, 19), false, false),
            (ymd(2027, 1, 20), true, false),
            (ymd(2027, 3, 1), true, true),
            (ymd(2027, 3, 10), true, true),
            (ymd(2027, 3, 11), false, true),
            (ymd(2027, 4, 30), false, true),
            (ymd(2027, 5, 1), false, false),
            (ymd(2027, 8, 24), false, false),
            (ymd(2027, 8, 25), true, false),
            (ymd(2027, 8, 31), true, false),
        ] {
            let up = |want: Want| factor(want, false, Some(&afternoon), date) > 1.0;
            assert_eq!(up(homework), homework_up, "{date:?}");
            assert_eq!(up(Want::Sneeze), sneeze_up, "{date:?}");
            // The date's, whatever the time of day (her mind is given a
            // date only with her day: see `Osaka::choose_next`).
            assert_eq!(factor(homework, false, None, date) > 1.0, homework_up);
        }
    }

    /// Her needs rise by her day (D4 lever 2): sleepiness slowly by day
    /// and quickly at homework and past bedtime, hunger quickly at
    /// breakfast and dinner, the rest as ever; with her mood's never
    /// more than four times as fast; and without her day exactly as
    /// ever.
    #[test]
    fn needs_rise_by_her_day() {
        for need in Need::ALL {
            assert_eq!(clock_rate(None, need), 1.0, "{need:?}");
        }
        let rate = |slot: Slot, need: Need| clock_rate(Some(slot), need);
        for slot in Slot::ALL {
            let sleepy = match slot {
                Slot::Morning | Slot::Away | Slot::Afternoon => 0.3,
                Slot::Evening => 1.0,
                Slot::Homework | Slot::Asleep => 3.0,
            };
            assert_eq!(rate(slot, Need::Sleepy), sleepy, "{slot:?}");
            let hungry = if matches!(slot, Slot::Morning | Slot::Evening) {
                2.0
            } else {
                1.0
            };
            assert_eq!(rate(slot, Need::Hungry), hungry, "{slot:?}");
            for need in Need::ALL {
                if !matches!(need, Need::Sleepy | Need::Hungry) {
                    assert_eq!(rate(slot, need), 1.0, "{slot:?} {need:?}");
                }
            }
        }
        // A minute in the afternoon, as ever bar sleepiness.
        let minute = |slot: Option<Slot>, mood: Mood| {
            let mut needs = Needs::with(&[]);
            needs.pass(60_000, Rising::ALL, mood, |need| clock_rate(slot, need));
            needs
        };
        let ever = minute(None, Mood::Ordinary);
        let afternoon = minute(Some(Slot::Afternoon), Mood::Ordinary);
        for need in Need::ALL {
            let times = if need == Need::Sleepy { 0.3 } else { 1.0 };
            assert!(
                (afternoon.get(need) - ever.get(need) * times).abs() < 1e-12,
                "{need:?}"
            );
        }
        // Lazy at homework: 1.5 × 3 is held at 4.
        let lazy = minute(Some(Slot::Homework), Mood::Lazy);
        assert!((lazy.get(Need::Sleepy) - 4.0 * ever.get(Need::Sleepy)).abs() < 1e-12);
        let lazy_ever = minute(None, Mood::Lazy);
        assert!((lazy_ever.get(Need::Sleepy) - 1.5 * ever.get(Need::Sleepy)).abs() < 1e-12);
    }

    /// Asleep the night (A11), her sleepiness doesn't rise, and the rest
    /// at a quarter of their pace; awake, the rest of the stretch counts
    /// in full. With no time asleep, exactly as ever.
    #[test]
    fn a_nights_sleep_slows_her_needs() {
        let pass = |ms: u64, slept_ms: u64| {
            let mut needs = Needs::with(&[]);
            let rising = Rising {
                slept_ms,
                ..Rising::ALL
            };
            needs.pass(ms, rising, Mood::Ordinary, |_| 1.0);
            needs
        };
        // Short enough that nothing reaches 1 (tidiness rises in 60 s).
        let awake = pass(6_000, 0);
        let asleep = pass(24_000, 24_000);
        let half = pass(12_000, 6_000);
        let longer = pass(6_000, 60_000);
        for need in Need::ALL {
            if need == Need::Sleepy {
                assert_eq!(asleep.get(need), 0.0);
                assert!((half.get(need) - awake.get(need)).abs() < 1e-12);
            } else {
                assert!(
                    (asleep.get(need) - awake.get(need)).abs() < 1e-12,
                    "{need:?}"
                );
                let expected = awake.get(need) * 1.25;
                assert!((half.get(need) - expected).abs() < 1e-12, "{need:?}");
            }
        }
        // Never more asleep than the stretch.
        assert_eq!(longer, pass(6_000, 6_000));
    }

    /// She arrives (D4 lever 3) in the afternoon exactly as ever; later
    /// in the day sleepier, at breakfast and dinner hungrier; every other
    /// need as ever, whatever the time.
    #[test]
    fn she_arrives_as_her_day_has_left_her() {
        assert_eq!(Needs::arriving_in(Slot::Afternoon), Needs::default());
        let ever = Needs::default();
        for slot in Slot::ALL {
            let needs = Needs::arriving_in(slot);
            let sleepier = matches!(slot, Slot::Evening | Slot::Homework | Slot::Asleep);
            let hungrier = matches!(slot, Slot::Morning | Slot::Evening);
            assert_eq!(
                needs.get(Need::Sleepy) > ever.get(Need::Sleepy),
                sleepier,
                "{slot:?}"
            );
            assert_eq!(
                needs.get(Need::Hungry) > ever.get(Need::Hungry),
                hungrier,
                "{slot:?}"
            );
            for need in Need::ALL {
                if !matches!(need, Need::Sleepy | Need::Hungry) {
                    assert_eq!(needs.get(need), ever.get(need), "{slot:?} {need:?}");
                }
            }
        }
        let sleepy = |slot: Slot| Needs::arriving_in(slot).get(Need::Sleepy);
        assert!(sleepy(Slot::Evening) < sleepy(Slot::Homework));
        assert!(sleepy(Slot::Homework) < sleepy(Slot::Asleep));
    }

    /// How much of her home she sets right a visit, by mood (the
    /// user's call): lazy none, ordinary or dreamy one, industrious three.
    #[test]
    fn home_acts_by_mood() {
        let acts = Mood::ALL.map(|m| (m, m.home_acts()));
        assert_eq!(
            acts,
            [
                (Mood::Ordinary, 1),
                (Mood::Lazy, 0),
                (Mood::Industrious, 3),
                (Mood::Dreamy, 1),
            ]
        );
    }
}
