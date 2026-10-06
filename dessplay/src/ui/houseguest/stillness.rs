//! Her stillness levers (phase 5c, D4 as amended by M6–M8 and B6): how a
//! mood stretches the still acts she chooses (lingering), how often a
//! still act that ran its course settles further where she is (settling
//! in), how many musings a daydream (and her window's sill) holds, where
//! her homework nods off,
//! and whether nearer spots draw her. One table holds them all, so the
//! tuning that pins the stillness band changes values here and nowhere
//! else.
//!
//! What ships is [`SHIPPED`]: [`Stillness::TUNED`] (phase 5c step 8c).
//! The mechanisms landed under [`Stillness::NEUTRAL`], every lever as it
//! was before it existed (each linger 1, no settling, one musing a
//! daydream, homework nodding off halfway, no nearness), which tests
//! still use to pin what a lever alone does. Tests turn a lever on by
//! setting [`Osaka`]'s table.
//!
//! [`Osaka`]: super::osaka::Osaka

use super::brain::Mood;

/// One value per mood.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct ByMood<T> {
    pub ordinary: T,
    pub lazy: T,
    pub industrious: T,
    pub dreamy: T,
}

impl<T: Copy> ByMood<T> {
    /// The same in every mood.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn all(value: T) -> Self {
        Self {
            ordinary: value,
            lazy: value,
            industrious: value,
            dreamy: value,
        }
    }

    pub fn of(&self, mood: Mood) -> T {
        match mood {
            Mood::Ordinary => self.ordinary,
            Mood::Lazy => self.lazy,
            Mood::Industrious => self.industrious,
            Mood::Dreamy => self.dreamy,
        }
    }

    #[cfg(test)]
    fn each(&self) -> [T; 4] {
        [self.ordinary, self.lazy, self.industrious, self.dreamy]
    }
}

/// Where her homework nods off: the branch of its script that writes
/// for this share of its body, nods for half what's left, then sleeps on
/// the paper (see `script::HOMEWORK`, whose branches are in this order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NodOff {
    /// Writing for the first half (as homework always has).
    Half,
    /// A third.
    Third,
    /// Five sixths.
    FiveSixths,
}

impl NodOff {
    /// Its branch of the homework script.
    pub fn branch(self) -> u8 {
        match self {
            Self::Half => 0,
            Self::Third => 1,
            Self::FiveSixths => 2,
        }
    }
}

/// The stillness levers, by mood where a mood bears on one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Stillness {
    /// Times the length she draws for a still act she chose: sitting,
    /// lying back, gazing; spacing out (musing, or her rare musing); and
    /// the still uses (lounging, napping, reading, looking out, a day's
    /// sleep). Never exercise, chores, lying on her front kicking her
    /// feet (it draws the eye), homework (its nod-off moves instead),
    /// her night, a trial sit, a glance, Setsubun or the moment after a
    /// swap; nor watching until the TV holds a picture (phase 5c D7).
    /// See [`Activity::lingers`] and [`Use::lingers`].
    ///
    /// [`Activity::lingers`]: super::osaka::Activity::lingers
    /// [`Use::lingers`]: super::room::Use::lingers
    pub linger: ByMood<f64>,
    /// The odds, as a still act she chose (or settled into) runs its
    /// course, that she settles further where she is: spacing out or
    /// gazing to sitting, sitting to lying back (or dozing where she
    /// sits), lounging to a nap on the same sofa.
    pub settle: ByMood<f64>,
    /// Settling from sitting, the odds she dozes off sitting up rather
    /// than lying back.
    pub sit_doze: f64,
    /// The musings a daydream session holds (from, to: both included),
    /// the first as her musing has always been (or a riddle, her rare
    /// musing, a glance at her clock, which hold no more); each next one
    /// said [`Stillness::musing_gap`] after the last while the session
    /// lasts. None: she only spaces out.
    pub musings: ByMood<(u8, u8)>,
    /// The gap between one musing of a session and the next (ms, from
    /// and to).
    pub musing_gap: (u64, u64),
    /// The musings on the sky she has leaning on her window's sill, after
    /// its first line (from, to: both included; three at most, phase 5c
    /// D6), each [`Stillness::musing_gap`] after the last.
    pub sill: ByMood<(u8, u8)>,
    /// Where her homework nods off.
    pub nod_off: ByMood<NodOff>,
    /// Nearer spots: a line to pull, letters to swap or text to make a
    /// piece of on her own floor strictly first when it has any, then
    /// nearer ones likelier; a seat likelier the nearer it is (see
    /// `mind::Near`).
    pub near: bool,
}

impl Stillness {
    /// Every lever as before it was made: what shipped until the band
    /// was tuned (phase 5c step 8c); tests pin a lever alone against it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const NEUTRAL: Self = Self {
        linger: ByMood::all(1.0),
        settle: ByMood::all(0.0),
        sit_doze: 0.0,
        musings: ByMood::all((1, 1)),
        musing_gap: (12_000, 20_000),
        // As it was built (phase 5c step 11): none to three, by a whim.
        sill: ByMood::all((0, 3)),
        nod_off: ByMood::all(NodOff::Half),
        near: false,
    };

    /// The design's starting values (phase 5c D4, M8, B6), before the
    /// band tunes them: a lazy Osaka lingers and settles most, an
    /// industrious one least; a dreamy one lingers and settles as an
    /// ordinary one does, and differs by her daydreams.
    ///
    /// Its musings need the longer spacing out D4 asks for (with 5b's
    /// 6–14 s, a session's second musing, 12 s or more on, seldom fits
    /// and goes unsaid): `SPACE_OUT_MS` is 10–28 s with it.
    pub const STARTING: Self = Self {
        linger: ByMood {
            ordinary: 1.0,
            lazy: 1.5,
            industrious: 0.7,
            dreamy: 1.0,
        },
        settle: ByMood {
            ordinary: 0.35,
            lazy: 0.6,
            industrious: 0.15,
            dreamy: 0.35,
        },
        sit_doze: 0.5,
        musings: ByMood {
            ordinary: (1, 3),
            lazy: (0, 2),
            industrious: (0, 1),
            dreamy: (2, 4),
        },
        musing_gap: (12_000, 20_000),
        // As her daydreams' musings, three at most.
        sill: ByMood {
            ordinary: (1, 3),
            lazy: (0, 2),
            industrious: (0, 1),
            dreamy: (2, 3),
        },
        nod_off: ByMood {
            ordinary: NodOff::Half,
            lazy: NodOff::Third,
            industrious: NodOff::FiveSixths,
            dreamy: NodOff::Half,
        },
        near: true,
    };

    /// What ships (phase 5c step 8c): [`Stillness::STARTING`], but an
    /// industrious Osaka lingers a little longer (0.85, not 0.7), which
    /// brings the furnished home's industrious afternoon under its
    /// ceiling (step 8b's run c3), and a dreamy one a little less than an
    /// ordinary one (0.85, not 1), which keeps her afternoons where she
    /// has text above her off the band's floor: her long daydreams are
    /// her stillness, not her lingering (phase5c/baseline.md, "Shipped
    /// (step 8c)").
    pub const TUNED: Self = Self {
        linger: ByMood {
            industrious: 0.85,
            dreamy: 0.85,
            ..Self::STARTING.linger
        },
        ..Self::STARTING
    };

    /// `ms` drawn for a still act she chose, lingered as her `mood` does.
    pub fn lingered(&self, mood: Mood, ms: u64) -> u64 {
        (ms as f64 * self.linger.of(mood)).round() as u64
    }

    /// `ms`, as long as the most lingering mood lingers over it if it
    /// `lingers`.
    #[cfg(test)]
    pub fn lingered_most(&self, lingers: bool, ms: u64) -> u64 {
        if lingers {
            (ms as f64 * self.most_linger()).round() as u64
        } else {
            ms
        }
    }

    /// Whether any mood settles in at all.
    #[cfg(test)]
    pub fn settles(&self) -> bool {
        self.settle.each().into_iter().any(|odds| odds > 0.0)
    }

    /// The most any mood lingers (at least 1: a mood that lingers less
    /// shortens no act past its table's longest).
    #[cfg(test)]
    pub fn most_linger(&self) -> f64 {
        self.linger.each().into_iter().fold(1.0, f64::max)
    }
}

/// The levers as she ships with them.
pub(super) const SHIPPED: Stillness = Stillness::TUNED;
