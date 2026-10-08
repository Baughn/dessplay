//! Scripts: what she does over an act, as data. A script is a run of
//! keys, each a look (pose, face, what she says) and what it shows on
//! her furniture, lasting to a cumulative end; one player plays any of
//! them. Every use of a piece of her furniture plays one (its own, or
//! the shopping channel's on a watch), and the use's [`Play`] holds all
//! that was chosen for it when it began, so how she looks at any instant
//! is a pure function of the act.

use super::art::{Channel, Programme, Sky};
use super::calendar::Tints;
use super::mind::{Lines, RIDDLES, Whims};
use super::osaka::{Bubble, SCRUNCH, THERE, USE_FRAME_MS};
use super::rarity::{Rares, Rarity};
use super::room::{Furniture, Use};
use super::routine::{DayTime, Slot};
use super::sprite::{Face, Pose};

/// When a key ends: cumulative, from the start of the body it plays
/// in, and never past the body's end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Span {
    /// This many ms into the body.
    Ms(u64),
    /// `n`/`d` of the way through the body (rounded down).
    Upto(u64, u64),
    /// The rest of the body.
    Rest,
}

impl Span {
    /// Where this span ends, in a body `body` ms long (`None`: a body
    /// with no set length, where only [`Span::Ms`] ends).
    pub fn end(self, body: Option<u64>) -> u64 {
        match (self, body) {
            (Self::Ms(ms), Some(body)) => ms.min(body),
            (Self::Ms(ms), None) => ms,
            (Self::Upto(n, d), Some(body)) => (body.saturating_mul(n) / d.max(1)).min(body),
            (Self::Rest, Some(body)) => body,
            (Self::Upto(..) | Self::Rest, None) => u64::MAX,
        }
    }
}

/// Her pose through a key.
#[derive(Clone, Copy, Debug)]
pub(super) enum Posed {
    /// Whatever pose the act hosting the script has her in (watching
    /// the TV: sitting, or lounging on a sofa).
    Host,
    /// Held still.
    Still(Pose),
    /// Bobbing between frames 0 and 1 of a pose every so many ms,
    /// counted from the start of the part it plays in (the prelude, the
    /// body or the coda; not of the key), so a bob running across keys
    /// keeps its beat; but a period clear of its key's start and end
    /// ([`bob_frame`]), so its flips never come within a frame of the
    /// changes as its key starts and ends. The period is a whole number
    /// of frames ([`USE_FRAME_MS`]), so her wakeups on the frame grid
    /// catch every bob.
    Bob(fn(u8) -> Pose, u64),
}

/// What she says through a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Say {
    /// A bubble of her own.
    Bubble(Bubble),
    /// The shopping channel's pitch for what it's selling her.
    Pitch,
    /// The question of the riddle drawn for her (its place in
    /// [`RIDDLES`] the first of [`Play::drawn`]).
    Riddle,
    /// That riddle's answer.
    Answer,
}

/// What her script shows on her furniture at a moment: on every shown
/// piece of its kind ([`Prop::item`]). A key names it as [`Shows`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Prop {
    /// The TV on, showing a channel (on its frame, if it moves).
    Tv(Channel),
    /// The lamp switched off.
    LampOff,
    /// The fridge door open.
    FridgeOpen,
    /// The cat, if he's in his bed, biting.
    CatBiting,
}

/// How long each frame of what's on TV lasts, while it moves (static,
/// Chiyo-chichi's hook).
pub(super) const CHANNEL_FRAME_MS: u64 = 400;

/// How long the static lasts as she switches the TV on to watch (phase
/// 5c D7): three of its frames, ending on a frame's edge, then the
/// programme she drew holds.
pub(super) const STATIC_MS: u64 = 3 * CHANNEL_FRAME_MS;

/// What a key shows on her furniture, as its script has it: a prop held
/// as it is, or her TV on static or Chiyo-chichi's hook (each moving
/// every [`CHANNEL_FRAME_MS`]), or holding the programme drawn for the
/// act ([`Play::card`]). Only static and the hook move: whatever else
/// is on holds still for as long as its key plays (phase 5c D7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Shows {
    /// Held as it is: the lamp off, the fridge open, the cat biting; the
    /// TV on a still channel (colour bars, the sunrise, Chiyo-chichi
    /// after his hook).
    Still(Prop),
    /// The TV on static, as she switches it on and between channels.
    Static,
    /// The shopping channel's hook: Chiyo-chichi bobbing as he talks.
    Hook,
    /// The TV holding the programme she drew for the act.
    Programme,
}

impl Shows {
    /// What it shows `elapsed` ms into the part its key plays in, on an
    /// act that drew `card`.
    pub fn at(self, elapsed: u64, card: Programme) -> Prop {
        let frame = (elapsed / CHANNEL_FRAME_MS % 2) as u8;
        match self {
            Self::Still(prop) => prop,
            Self::Static => Prop::Tv(Channel::Snow(frame)),
            Self::Hook => Prop::Tv(Channel::Shopping(frame)),
            Self::Programme => Prop::Tv(Channel::Programme(card)),
        }
    }
}

impl Prop {
    /// The kind of piece it shows on.
    pub fn item(self) -> Furniture {
        match self {
            Self::Tv(_) => Furniture::Tv,
            Self::LampOff => Furniture::Lamp,
            Self::FridgeOpen => Furniture::Fridge,
            Self::CatBiting => Furniture::CatBed,
        }
    }

    /// What's on TV, if it's the TV.
    pub fn channel(self) -> Option<Channel> {
        match self {
            Self::Tv(channel) => Some(channel),
            Self::LampOff | Self::FridgeOpen | Self::CatBiting => None,
        }
    }
}

/// One key of a script.
#[derive(Clone, Copy, Debug)]
pub(super) struct Key {
    pub span: Span,
    pub pose: Posed,
    pub face: Face,
    pub say: Option<Say>,
    pub prop: Option<Shows>,
}

impl Key {
    /// How often what this key shows moves (ms), if she's to wake for
    /// each move: a bob's period, or static's frame
    /// ([`CHANNEL_FRAME_MS`], so all three of the switch-on's frames are
    /// painted). Counted from the start of the part it plays in, as
    /// [`Play::next_frame`] counts. Chiyo-chichi's hook moves at paint
    /// time instead: his bob is talk, not a picture to see whole.
    pub fn frame_ms(&self) -> Option<u64> {
        let bob = match self.pose {
            Posed::Bob(_, period) => Some(period),
            Posed::Host | Posed::Still(_) => None,
        };
        let shows = match self.prop {
            Some(Shows::Static) => Some(CHANNEL_FRAME_MS),
            Some(Shows::Still(_) | Shows::Hook | Shows::Programme) | None => None,
        };
        match (bob, shows) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Its bob's frame at `time`, if it bobs.
    pub fn bob(&self, time: KeyTime) -> Option<u8> {
        match self.pose {
            Posed::Bob(_, period) => Some(time.bob(period)),
            Posed::Host | Posed::Still(_) => None,
        }
    }

    /// How she looks at `time` in the part this key plays in: in
    /// `host`'s pose where the key leaves it to the host, saying what
    /// `play` drew for her or pitching what it sold her.
    pub fn look(&self, time: KeyTime, host: Pose, play: &Play) -> Look {
        let pose = match self.pose {
            Posed::Host => host,
            Posed::Still(pose) => pose,
            Posed::Bob(pose, period) => pose(time.bob(period)),
        };
        let bubble = self.say.and_then(|say| match say {
            Say::Bubble(bubble) => Some(bubble),
            Say::Pitch => play.bought.map(|item| Bubble::Say(item.spec().pitch)),
            Say::Riddle | Say::Answer => {
                let (question, answer) = *RIDDLES.get(usize::from(play.drawn[0]))?;
                Some(Bubble::Say(if say == Say::Riddle {
                    question
                } else {
                    answer
                }))
            }
        });
        (pose, self.face, bubble)
    }
}

/// When a key plays in its part (the prelude, the body or the coda), in
/// ms from the part's start: how far into the part it is (`elapsed`),
/// and the key's own `start` and `end` there (the last key's end the
/// part's, or `u64::MAX` in a part with no set length). What it shows is
/// timed by `elapsed`, as everything in a part is; a bob also keeps clear
/// of `start` and `end` ([`bob_frame`]), and of `held`: a moment in the
/// part (ms) where something else of hers changed in place, and the
/// bob's frame then ([`bob_frame`]; her player sets it, the script
/// never does).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeyTime {
    pub elapsed: u64,
    pub start: u64,
    pub end: u64,
    pub held: Option<(u64, u8)>,
}

impl KeyTime {
    /// The frame of a bob flipping every `period` ms, at this time.
    pub fn bob(self, period: u64) -> u8 {
        bob_frame(self.elapsed, self.start, self.end, period, self.held)
    }
}

/// A bob's frame (0 or 1) `elapsed` ms into its part, flipping every
/// `period` ms on the part's grid, but only a whole period clear of what
/// bobs' `start` and `end` (ms into the part: a key's, or an idle act's
/// own): its first frame held until the first flip on the grid a period
/// or more after its start, its last held through the part of a period
/// before its end (Round 8, the user). So its flips and the changes as
/// it starts and ends (another key's look, her getting up) are never
/// within a period of each other, wherever a share of the use puts them.
/// It starts on the grid's frame at `start`, so a bob starting on the
/// grid (every part's first key; every idle act) is the plain beat but
/// at its end, and one running on across keys keeps its beat as near as
/// it can. A period of 0 holds frame 0.
///
/// And it's held where something of hers changed in place off its grid
/// (`held`: the moment, ms into the part, and the frame shown then): her
/// act's end moved (phase 5c's tail, T4). From that moment the frame it
/// showed holds until the first flip on the grid a whole period on, as
/// from a start, so the bob neither changes as the moment comes (an end
/// moved within its last period would release or take back a flip
/// there, off the grid) nor flips within a period after it; its end
/// holds as ever. A moment before `start` (a key that started since) or
/// after `elapsed` holds nothing.
pub(super) fn bob_frame(
    elapsed: u64,
    start: u64,
    end: u64,
    period: u64,
    held: Option<(u64, u8)>,
) -> u8 {
    if period == 0 {
        return 0;
    }
    // Where its beat runs from, and its frame there: its start, on the
    // grid's frame, or the moment held, on the frame shown then.
    let (from, frame) = match held {
        Some((at, frame)) if (start..=elapsed).contains(&at) => (at, u64::from(frame)),
        _ => (start, start / period),
    };
    // The grid's flips it makes: from the first a period after that to
    // the last a period before its end, none past `elapsed`.
    let first = from.saturating_add(period).div_ceil(period);
    let last = end.saturating_sub(period).min(elapsed) / period;
    let flips = (last + 1).saturating_sub(first);
    ((frame + flips) % 2) as u8
}

/// Where a key plays in a use (for tests telling keys apart): when
/// its part (the prelude, the body or the coda) starts, its index in
/// that part's keys, and whether it bobs ([`Posed::Bob`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeyPlace {
    pub part: u64,
    pub index: usize,
    pub bobs: bool,
}

/// Her pose, face and bubble.
pub(super) type Look = (Pose, Face, Option<Bubble>);

/// The first of `items` whose end (`end_of`, called on each in turn,
/// so it may sum durations or read cumulative ends) is past `elapsed`:
/// its index, it, and its end. `None` once all are over.
pub(super) fn at<T>(
    items: &[T],
    elapsed: u64,
    mut end_of: impl FnMut(&T) -> u64,
) -> Option<(usize, &T, u64)> {
    items.iter().enumerate().find_map(|(i, item)| {
        let end = end_of(item);
        (elapsed < end).then_some((i, item, end))
    })
}

/// The key of `keys` playing `elapsed` ms into a body `body` ms long
/// (`None`: no set length), its index, and when it plays ([`KeyTime`]:
/// `elapsed`, and when it starts and ends). Past the end of the body,
/// its last moment's key holds.
pub(super) fn key_at(
    keys: &[Key],
    elapsed: u64,
    body: Option<u64>,
) -> Option<(usize, &Key, KeyTime)> {
    let held = body.map_or(elapsed, |body| elapsed.min(body.saturating_sub(1)));
    let (index, key, end) = at(keys, held, |key| key.span.end(body)).or_else(|| {
        let last = keys.len().checked_sub(1)?;
        Some((last, keys.get(last)?, body.unwrap_or(u64::MAX)))
    })?;
    // It starts as the last of those before it ends (ends are where in
    // the body each key ends, so a key that ends no later than one
    // before it never plays).
    let start = keys[..index]
        .iter()
        .map(|key| key.span.end(body))
        .max()
        .unwrap_or(0);
    Some((
        index,
        key,
        KeyTime {
            elapsed,
            start,
            end,
            held: None,
        },
    ))
}

/// A riddle's question shows this long, from the start of her spacing
/// out.
pub(super) const RIDDLE_ASKED_MS: u64 = 3000;
/// Her answer shows from the question's end to this far in.
pub(super) const RIDDLE_ANSWERED_MS: u64 = 5500;

/// A script: each of its branches a run of keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ScriptId {
    Lounge,
    Nap,
    Sleep,
    /// Writing, then nodding off onto the paper.
    Homework,
    /// Static as she switches it on, then the programme she drew, held.
    Watch,
    /// Watching the shopping channel: hooked, then sold.
    Shopping,
    Read,
    /// A look in the fridge, then the melon bread.
    Snack,
    /// Petting the cat, who has had quite enough.
    Pet,
    /// Scrunching torn text into shape, pleased with it at the end.
    Crumple,
    /// Bent over the box, rummaging.
    Unpack,
    /// Spacing out, she tells a riddle and answers it herself at once,
    /// pleased with it.
    Riddle,
    /// Flicking through the channels: snow, colour bars, snow, a
    /// sunrise (ooh!), then back on snow, pleased with herself.
    Surf,
    /// Splitting disposable chopsticks before her homework: cleanly
    /// (a twinkle, hehe) or badly (let down, and told off by herself).
    Chopsticks,
    /// After a snack, a sata andagi from the fridge: "Sata andagi." a
    /// few times over, happier each time, then she eats it.
    Andagi,
    /// Her night's sleep (phase 5b A4): one act from bedtime to the wake
    /// time, on her bed, her sofa, a makeshift heap or the floor (each
    /// its pose: see [`Surface`]), the lamp on a moment as she settles
    /// (#70) or off from the start. A chat line only stirs her.
    Night,
    /// Dashed home from school for her lunch (phase 5b D3a): a look in
    /// the fridge, then "Forgot my lunch!". Built directly on a use of
    /// her fridge, never chosen (see `Osaka::dash_on`).
    DashLunch,
    /// Dashed home from school with no fridge to get to: "Forgot
    /// somethin'..." then "...what was it?", spacing out.
    DashForgot,
    /// Setsubun (Feb 3, owed by her calendar): beans thrown on the spot,
    /// jumping from foot to foot, "Oni wa soto!", then "Fuku wa uchi!".
    Setsubun,
    /// New Year's Day (owed by her calendar): the first sunrise on her
    /// TV, "Ooh... first sunrise.", then watching it, humming.
    FirstSunrise,
    /// The Dream (rare, step 7): once a night, 30 game minutes after she
    /// first slept, her night's sleep turns to a dream of a certain
    /// orange cat, talked in her sleep: "Hello everynyan...", "Fine
    /// sankyu...", "Oh my gah!". Her night's act from then on, on
    /// whatever she sleeps on (each surface its branch: see
    /// [`Surface::dream_branch`]); a chat line only stirs her.
    Dream,
    /// After a snack (rare): back in the fridge, let down, "That was the
    /// last one." (the melon bread she just ate).
    NoMelon,
    /// Spacing out (rare): "The box one's the..." then "...escalator?
    /// No?".
    Escalator,
    /// After lounging or reading, from 22:00 to bedtime (rare): hugging
    /// her knees, "Scary story time...", then "A fart. Not mine.".
    Scary,
    /// Looking out of the window (phase 5b D7), gazing up: a line from
    /// the sky as it is (see [`LOOK_OUT_LINES`]), each its branch.
    LookOut,
    /// A glance up at her wall clock (phase 5b D7), spacing out a moment:
    /// "Oh! It's late!" at bedtime, "Time for school!" as she leaves,
    /// or the hour, roughly, of an afternoon ("Three-ish."), each its
    /// branch (see [`ClockGlance`]).
    ClockGlance,
    /// Her homework on the floor (phase 5c D5), where no desk stands: on
    /// her front writing (or on her back reading the set text), then
    /// nodding off as at her desk, each a branch (see
    /// [`floor_homework_branch`]). Played on [`Activity::FloorHomework`],
    /// never chosen as a use's.
    ///
    /// [`Activity::FloorHomework`]: super::osaka::Activity::FloorHomework
    FloorHomework,
}

impl ScriptId {
    #[cfg(test)]
    pub const ALL: [ScriptId; 27] = [
        Self::Lounge,
        Self::Nap,
        Self::Sleep,
        Self::Homework,
        Self::Watch,
        Self::Shopping,
        Self::Read,
        Self::Snack,
        Self::Pet,
        Self::Crumple,
        Self::Unpack,
        Self::Riddle,
        Self::Surf,
        Self::Chopsticks,
        Self::Andagi,
        Self::Night,
        Self::DashLunch,
        Self::DashForgot,
        Self::Setsubun,
        Self::FirstSunrise,
        Self::Dream,
        Self::NoMelon,
        Self::Escalator,
        Self::Scary,
        Self::LookOut,
        Self::ClockGlance,
        Self::FloorHomework,
    ];

    /// How rare it is (phase 5b D6): only a Rare or Legendary script
    /// passes a gate (see [`super::rarity::Rares`]); the rest play on
    /// their own chances, as ever. Wildcard-free, so a new script says.
    pub fn rarity(self) -> Rarity {
        match self {
            Self::Dream | Self::NoMelon | Self::Escalator | Self::Scary => Rarity::Rare,
            Self::Riddle
            | Self::Surf
            | Self::Chopsticks
            | Self::Andagi
            | Self::Shopping
            | Self::Setsubun
            | Self::FirstSunrise => Rarity::Uncommon,
            Self::Lounge
            | Self::Nap
            | Self::Sleep
            | Self::Homework
            | Self::Watch
            | Self::Read
            | Self::Snack
            | Self::Pet
            | Self::Crumple
            | Self::Unpack
            | Self::Night
            | Self::DashLunch
            | Self::DashForgot
            | Self::LookOut
            | Self::ClockGlance
            | Self::FloorHomework => Rarity::Common,
        }
    }

    /// Whether it's her night's sleep (her night, or the Dream it turns
    /// to): the one test of it, wherever a night act is told apart.
    pub fn is_night(self) -> bool {
        matches!(self, Self::Night | Self::Dream)
    }

    /// Its branches, each a run of keys.
    pub fn branches(self) -> &'static [&'static [Key]] {
        match self {
            Self::Lounge => LOUNGE,
            Self::Nap => NAP,
            Self::Sleep => SLEEP,
            Self::Homework => HOMEWORK,
            Self::Watch => WATCH,
            Self::Shopping => SHOPPING,
            Self::Read => READ,
            Self::Snack => SNACK,
            Self::Pet => PET,
            Self::Crumple => CRUMPLE,
            Self::Unpack => UNPACK,
            Self::Riddle => RIDDLE,
            Self::Surf => SURF,
            Self::Chopsticks => CHOPSTICKS,
            Self::Andagi => ANDAGI,
            Self::Night => NIGHT,
            Self::DashLunch => DASH_LUNCH,
            Self::DashForgot => DASH_FORGOT,
            Self::Setsubun => SETSUBUN,
            Self::FirstSunrise => FIRST_SUNRISE,
            Self::Dream => DREAM,
            Self::NoMelon => NO_MELON,
            Self::Escalator => ESCALATOR,
            Self::Scary => SCARY,
            Self::LookOut => LOOK_OUT,
            Self::ClockGlance => CLOCK_GLANCE,
            Self::FloorHomework => FLOOR_HOMEWORK,
        }
    }

    /// What a chat line arriving while it plays does: what it does to
    /// the act hosting it (she looks, up from it where she is or
    /// standing, by the act's own class); or it stops the script though
    /// its act is still; or, a line asking her something, gets this
    /// answer said toward the chat while she plays on; or it only stirs
    /// her. Only a splice's script answers, and only her night's stirs
    /// (see [`Osaka::look`]). This layer says what's particular to a
    /// script; whether the act hosting it is still enough to look up
    /// from in place is the act's class (`OnChat` in osaka.rs, phase 5c
    /// B1), and a dozing pose stirs whatever the script ([`Pose::dozes`]).
    /// Wildcard-free.
    ///
    /// [`Osaka::look`]: super::osaka::Osaka::look
    /// [`Pose::dozes`]: super::sprite::Pose::dozes
    pub fn on_chat(self) -> Chat {
        match self {
            Self::Andagi => Chat::Answer(SATA_ANDAGI),
            Self::Night | Self::Dream => Chat::Stir,
            // Thrown on the spot, but thrown; and dashing home is a dash.
            Self::Setsubun | Self::DashForgot => Chat::Stop,
            Self::Lounge
            | Self::Nap
            | Self::Sleep
            | Self::Homework
            | Self::Watch
            | Self::Shopping
            | Self::Read
            | Self::Snack
            | Self::Pet
            | Self::Crumple
            | Self::Unpack
            | Self::Riddle
            | Self::Surf
            | Self::Chopsticks
            | Self::DashLunch
            | Self::FirstSunrise
            | Self::NoMelon
            | Self::Escalator
            | Self::Scary
            | Self::LookOut
            | Self::ClockGlance
            | Self::FloorHomework => Chat::Look,
        }
    }

    /// The branch it plays on a trial sit (a moment on a piece she's
    /// trying where it stands): in bed, the lamp goes off at once, or it
    /// would flicker. Wildcard-free, so a new script says.
    pub fn trial_branch(self) -> u8 {
        match self {
            Self::Sleep => 1,
            Self::Lounge
            | Self::Nap
            | Self::Homework
            | Self::Watch
            | Self::Shopping
            | Self::Read
            | Self::Snack
            | Self::Pet
            | Self::Crumple
            | Self::Unpack
            | Self::Riddle
            | Self::Surf
            | Self::Chopsticks
            | Self::Andagi
            | Self::Night
            | Self::DashLunch
            | Self::DashForgot
            | Self::Setsubun
            | Self::FirstSunrise
            | Self::Dream
            | Self::NoMelon
            | Self::Escalator
            | Self::Scary
            | Self::LookOut
            | Self::ClockGlance
            | Self::FloorHomework => 0,
        }
    }

    /// The use it plays on as that use's own script (`None`: it plays
    /// spacing out, spliced round a use, or on the floor as an activity's
    /// own). Wildcard-free, so a new
    /// script doesn't compile until it says (the lints hold each to its
    /// host). Her night's plays on whatever she sleeps on, a sofa's nap
    /// and the floor too; as the stage cues it, on her bed.
    pub fn played_on(self) -> Option<Use> {
        Some(match self {
            Self::Lounge => Use::Lounge,
            Self::Nap => Use::Nap,
            Self::Sleep | Self::Night | Self::Dream => Use::Sleep,
            Self::Homework => Use::Homework,
            Self::Watch | Self::Shopping | Self::Surf | Self::FirstSunrise => Use::Watch,
            Self::Read => Use::Read,
            Self::Snack | Self::DashLunch => Use::Snack,
            Self::Pet => Use::Pet,
            Self::Crumple => Use::Crumple,
            Self::Unpack => Use::Unpack,
            Self::LookOut => Use::LookOut,
            Self::Riddle
            | Self::Chopsticks
            | Self::Andagi
            | Self::DashForgot
            | Self::Setsubun
            | Self::NoMelon
            | Self::Escalator
            | Self::Scary
            | Self::ClockGlance
            | Self::FloorHomework => {
                return None;
            }
        })
    }

    /// The act it plays on. Wildcard-free.
    #[cfg(test)]
    pub fn host(self) -> Host {
        match self {
            Self::Riddle
            | Self::DashForgot
            | Self::Setsubun
            | Self::Escalator
            | Self::ClockGlance => Host::SpaceOut,
            Self::Night | Self::Dream => Host::Night,
            Self::Chopsticks | Self::Andagi | Self::NoMelon | Self::Scary => Host::Splice,
            Self::FloorHomework => Host::Floor,
            Self::Lounge
            | Self::Nap
            | Self::Sleep
            | Self::Homework
            | Self::Watch
            | Self::Shopping
            | Self::Read
            | Self::Snack
            | Self::Pet
            | Self::Crumple
            | Self::Unpack
            | Self::Surf
            | Self::DashLunch
            | Self::FirstSunrise
            | Self::LookOut => Host::Use,
        }
    }

    /// The shortest body it can play over: a musing's, spacing out; a
    /// watch's, for what plays only on a watch she isn't trying (the
    /// shopping channel, surfing); else the shortest use there is (a
    /// trial sit). `None`: a splice's, each of whose branches is as long
    /// as its row says (the splice lint holds its keys to that).
    /// Wildcard-free.
    #[cfg(test)]
    pub fn shortest_body(self) -> Option<u64> {
        use super::osaka::{
            CLOCK_GLANCE_MS, DASH_FORGOT_MS, DASH_LUNCH_MS, SETSUBUN_MS, SPACE_OUT_MS,
            shortest_use_ms, use_range,
        };
        Some(match self {
            Self::Chopsticks | Self::Andagi | Self::NoMelon | Self::Scary => return None,
            Self::Riddle | Self::Escalator => SPACE_OUT_MS.0,
            Self::FloorHomework => super::osaka::Activity::FloorHomework.duration().0,
            // Begun only with room for it all before she wakes.
            Self::Dream => DREAM_MS,
            // Built directly, always as long.
            Self::DashLunch => DASH_LUNCH_MS,
            Self::DashForgot => DASH_FORGOT_MS,
            Self::Setsubun => SETSUBUN_MS,
            Self::ClockGlance => CLOCK_GLANCE_MS,
            Self::Shopping | Self::Surf | Self::FirstSunrise => use_range(Use::Watch).0,
            Self::Lounge
            | Self::Nap
            | Self::Sleep
            | Self::Homework
            | Self::Watch
            | Self::Read
            | Self::Snack
            | Self::Pet
            | Self::Crumple
            | Self::Unpack
            | Self::Night
            | Self::LookOut => shortest_use_ms(),
        })
    }

    /// The stage scene that cues it. Wildcard-free, so a new script
    /// doesn't compile until it can be cued.
    #[cfg(test)]
    pub fn scene(self) -> super::stage::Scene {
        use super::stage::Scene;
        match self {
            Self::Lounge => Scene::Lounge,
            Self::Nap => Scene::Nap,
            Self::Sleep => Scene::Sleep,
            Self::Homework => Scene::Homework,
            Self::Watch => Scene::Watch,
            Self::Shopping => Scene::Shopping,
            Self::Read => Scene::Read,
            Self::Snack => Scene::Snack,
            Self::Pet => Scene::Pet,
            Self::Crumple => Scene::MakeSofa,
            Self::Unpack => Scene::Parcel,
            Self::Riddle => Scene::Riddle,
            Self::Surf => Scene::Surf,
            Self::Chopsticks => Scene::ChopsticksClean,
            Self::Andagi => Scene::Andagi,
            Self::Night => Scene::Night,
            Self::DashLunch => Scene::DashIn,
            Self::DashForgot => Scene::DashForgot,
            Self::Setsubun => Scene::Setsubun,
            Self::FirstSunrise => Scene::FirstSunrise,
            Self::Dream => Scene::Dream,
            Self::NoMelon => Scene::NoMelon,
            Self::Escalator => Scene::Escalator,
            Self::Scary => Scene::Scary,
            Self::LookOut => Scene::LookOut,
            Self::ClockGlance => Scene::ClockGlance,
            Self::FloorHomework => Scene::FloorHomework,
        }
    }

    /// Branch `branch`'s keys (the first branch's, if it has no such
    /// branch).
    pub fn keys(self, branch: u8) -> &'static [Key] {
        let branches = self.branches();
        branches
            .get(usize::from(branch))
            .or(branches.first())
            .copied()
            .unwrap_or_default()
    }
}

/// The act a script plays on.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Host {
    /// Using a piece of her furniture (a use's own script, or one
    /// spliced around it).
    Use,
    /// Spacing out, musing: only a musing carries a script, never the
    /// short space-out after a swap, so its shortest body is a
    /// musing's.
    SpaceOut,
    /// Spliced round a use, as its prelude or coda (see [`SpliceId`]).
    Splice,
    /// Her night's sleep: a use of her bed, her sofa or a makeshift
    /// heap, or lying on the floor (see [`Surface`]).
    Night,
    /// Something she does on the floor, as its own script (her homework
    /// there: phase 5c D5).
    Floor,
}

/// What a chat line arriving while a script plays does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Chat {
    /// As at any act: she looks, up from it where she is if it's still
    /// (on calm floor), or stopping it and standing to look (by the act's
    /// class, `OnChat` in osaka.rs).
    Look,
    /// She stops it and stands to look, though the act hosting it is
    /// still (a spacing out that isn't: Setsubun's beans, a dash home).
    Stop,
    /// A line asking her something gets this said toward the chat, and
    /// she plays on; any other, she stops and looks.
    Answer(&'static str),
    /// Any line only stirs her: a murmur, a turn, and she sleeps on (her
    /// night's sleep: a chat at night doesn't wake her).
    Stir,
}

/// Where she spends her night (phase 5b A4), and so how her night's
/// script poses her: in bed, curled on a sofa, or flat on the floor,
/// held still (a makeshift heap is a bed's or a sofa's, by what it was
/// made for).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Surface {
    Bed,
    Sofa,
    Floor,
}

impl Surface {
    /// Every surface.
    #[cfg(test)]
    pub const ALL: [Surface; 3] = [Self::Bed, Self::Sofa, Self::Floor];

    /// What she sleeps on using a piece for `what`: a bed's sleep, a
    /// sofa's nap (`None`: nothing she spends the night on).
    pub fn of(what: Use) -> Option<Self> {
        match what {
            Use::Sleep => Some(Self::Bed),
            Use::Nap => Some(Self::Sofa),
            Use::Lounge
            | Use::Homework
            | Use::Watch
            | Use::Unpack
            | Use::Read
            | Use::Snack
            | Use::Pet
            | Use::Crumple
            | Use::LookOut => None,
        }
    }

    /// The branch of [`ScriptId::Night`] she sleeps on it on: the lamp on
    /// a moment as she settles (#70), or (`at_once`) off from the start.
    pub fn branch(self, at_once: bool) -> u8 {
        let surface = match self {
            Self::Bed => 0,
            Self::Sofa => 1,
            Self::Floor => 2,
        };
        2 * surface + u8::from(at_once)
    }

    /// The surface her night's branch `branch` is on.
    pub fn of_branch(branch: u8) -> Self {
        match branch / 2 {
            0 => Self::Bed,
            1 => Self::Sofa,
            _ => Self::Floor,
        }
    }

    /// The branch of [`ScriptId::Dream`] she dreams on it on (the lamp
    /// off: she's been asleep a while).
    pub fn dream_branch(self) -> u8 {
        match self {
            Self::Bed => 0,
            Self::Sofa => 1,
            Self::Floor => 2,
        }
    }
}

/// A script spliced before or after a use (a prelude or a coda).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum SpliceId {
    /// Before homework, about one in three: she splits a pair of
    /// disposable chopsticks, cleanly or badly (even odds).
    Chopsticks,
    /// After a snack, about one in four: a sata andagi, named a few
    /// times over (four to six, its branch), then eaten.
    Andagi,
    /// After a snack, on a day it's open (rare), one in three (of those
    /// the andagi leaves it): that was the last melon bread.
    NoMelon,
    /// After lounging or reading from 22:00 to bedtime, on a day it's
    /// open (rare), one in two: a scary story.
    Scary,
    /// A test's prelude (the snack's look in the fridge, then eating):
    /// not a row.
    #[cfg(test)]
    TestSnack,
    /// A test's coda (the lamp on a moment, then off as she sleeps): not
    /// a row.
    #[cfg(test)]
    TestSleep,
    /// A test's second prelude (settling as if for bed, the lamp on a
    /// moment or off at once, each its own length): not a row.
    #[cfg(test)]
    TestBedtime,
}

impl SpliceId {
    /// Every row: what a use may be wrapped in (the tests' splices
    /// aren't rows, and are never rolled).
    /// The rare rows last: on a day one's open, a commoner one that rolls
    /// still wraps the use as ever (the melon bread is missed after an
    /// andagi), so being open takes nothing from the rest.
    pub const ALL: [SpliceId; 4] = [Self::Chopsticks, Self::Andagi, Self::NoMelon, Self::Scary];

    /// Its row.
    pub fn row(self) -> Splice {
        match self {
            Self::Chopsticks => Splice {
                name: "chopsticks",
                salt: 1,
                around: &[Use::Homework],
                made: false,
                at: Part::Before,
                // In exam season, before every homework (A23): still
                // only once in its ten minutes' cooling.
                chance: |ctx| if ctx.tints.exams { (1, 1) } else { (1, 3) },
                when: |_| true,
                // Clean, bad.
                lens: &[CHOPSTICKS_MS, CHOPSTICKS_MS],
            },
            Self::Andagi => Splice {
                name: "sata andagi",
                salt: 2,
                around: &[Use::Snack],
                made: true,
                at: Part::After,
                chance: |_| (1, 4),
                when: |_| true,
                lens: ANDAGI_LENS,
            },
            Self::NoMelon => Splice {
                name: "no melon",
                salt: 3,
                around: &[Use::Snack],
                made: true,
                at: Part::After,
                chance: |_| (1, 3),
                when: |_| true,
                lens: &[NO_MELON_MS],
            },
            Self::Scary => Splice {
                name: "scary story",
                salt: 4,
                around: &[Use::Lounge, Use::Read],
                made: true,
                at: Part::After,
                chance: |_| (1, 2),
                // From 22:00 until her bedtime (by her routine: never
                // without it).
                when: |ctx| {
                    ctx.day
                        .is_some_and(|day| day.minute >= SCARY_FROM && day.slot != Slot::Asleep)
                },
                lens: &[SCARY_MS],
            },
            #[cfg(test)]
            Self::TestSnack => Splice {
                name: "test snack",
                salt: 100,
                around: TEST_AROUND,
                made: true,
                at: Part::Before,
                chance: |_| (1, 1),
                when: |_| true,
                lens: &[4000],
            },
            #[cfg(test)]
            Self::TestSleep => Splice {
                name: "test sleep",
                salt: 101,
                around: TEST_AROUND,
                made: true,
                at: Part::After,
                chance: |_| (1, 2),
                when: |_| true,
                lens: &[5000],
            },
            #[cfg(test)]
            Self::TestBedtime => Splice {
                name: "test bedtime",
                salt: 102,
                around: TEST_AROUND,
                made: true,
                at: Part::Before,
                chance: |_| (1, 2),
                when: |_| true,
                lens: &[3000, 4500],
            },
        }
    }

    /// The script it plays.
    pub fn script(self) -> ScriptId {
        match self {
            Self::Chopsticks => ScriptId::Chopsticks,
            Self::Andagi => ScriptId::Andagi,
            Self::NoMelon => ScriptId::NoMelon,
            Self::Scary => ScriptId::Scary,
            #[cfg(test)]
            Self::TestSnack => ScriptId::Snack,
            #[cfg(test)]
            Self::TestSleep | Self::TestBedtime => ScriptId::Sleep,
        }
    }

    /// The stage scenes that cue it, each forcing one of its branches
    /// or leaving it to her whims (none: the tests' splices, which
    /// aren't rows). Wildcard-free, so a new splice doesn't compile until
    /// it says.
    #[cfg(test)]
    pub fn scenes(self) -> &'static [super::stage::Scene] {
        use super::stage::Scene;
        match self {
            Self::Chopsticks => &[Scene::ChopsticksClean, Scene::ChopsticksBad],
            Self::Andagi => &[Scene::Andagi],
            Self::NoMelon => &[Scene::NoMelon],
            Self::Scary => &[Scene::Scary],
            Self::TestSnack | Self::TestSleep | Self::TestBedtime => &[],
        }
    }
}

/// What the tests' splices wrap: every use a splice may (not unpacking
/// or crumpling: see [`Splice::around`]).
#[cfg(test)]
const TEST_AROUND: &[Use] = &[
    Use::Lounge,
    Use::Nap,
    Use::Sleep,
    Use::Homework,
    Use::Watch,
    Use::Read,
    Use::Snack,
    Use::Pet,
];

/// Before the use it wraps (a prelude) or after it (a coda).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Before,
    After,
}

/// A splice's row: what it wraps, and how likely.
#[derive(Clone, Copy, Debug)]
pub(super) struct Splice {
    /// What it's called (the log and the stage say).
    pub name: &'static str,
    /// Its own salt for its rolls (all labelled `"splice"`, so no row's
    /// can be another whim's): its chance rolls at twice it, its branch
    /// at one more. Every row's differs (a lint holds it), and stays put
    /// as rows come and go, so no row's rolls change with another's.
    pub salt: u64,
    /// The uses it may wrap. Never unpacking or crumpling: what comes of
    /// those happens as the use ends, after any coda (a lint holds it).
    pub around: &'static [Use],
    /// Whether it may wrap a use of a piece she made: not the
    /// chopsticks, whose drawings are of her on her stool at a real desk
    /// (phase 5c D5: at her paper desk she kneels).
    pub made: bool,
    /// Before or after.
    pub at: Part,
    /// `n` uses in `d` it wraps (of those `when` allows), as the use
    /// starts.
    pub chance: fn(&SpliceCtx<'_>) -> (u64, u64),
    /// Whether it may wrap a use as it starts.
    pub when: fn(&SpliceCtx<'_>) -> bool,
    /// How long it plays in each of its branches (drawn evenly). Its
    /// script's keys end at set times (never a share: it has no body to
    /// take one of), each inside its branch's length.
    pub lens: &'static [u64],
}

impl Splice {
    /// Whether it may wrap a use for `what`, of a piece she made if
    /// `makeshift`: the one test of it, rolled or cued (see
    /// [`splices`], [`Cue::plays_on`]).
    pub fn wraps(&self, what: Use, makeshift: bool) -> bool {
        self.around.contains(&what) && (self.made || !makeshift)
    }
}

/// What a splice row sees as the use it would wrap starts.
#[derive(Clone, Copy, Debug)]
pub(super) struct SpliceCtx<'a> {
    /// What she's using it for.
    pub what: Use,
    /// The piece she's using is one she made.
    pub makeshift: bool,
    /// She's only trying the piece where it stands: no splice.
    pub trying: bool,
    /// She has stopped saying anything (a prelude's first key would be
    /// hidden under it otherwise, so none is rolled).
    pub quiet: bool,
    /// Her routine as the use starts (`None`: it doesn't reach her):
    /// the scary story's late evening.
    pub day: Option<DayTime>,
    /// The seasons of the real date as the use starts (none without a
    /// date, or without her routine): exam season's chopsticks.
    pub tints: Tints,
    /// What's rare and open today (phase 5b D6): a rare row wraps a use
    /// only if its script is.
    pub rares: &'a Rares,
}

/// A cue from the stage: what she's to play, forced rather than rolled,
/// the next time it can be. It waits, through anything else she does,
/// for the first use it plays on ([`Cue::plays_on`]), or (a riddle) the
/// first musing; that takes it, and it plays (and starts its script's
/// cooldown, as if rolled).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Cue {
    /// A script, on the next use of its piece it can play on (or the
    /// next musing, a riddle). A plain watch, surfing and the shopping
    /// channel all play on a watch: cued to one, she plays none of the
    /// others (the advert still decides what's on the shopping
    /// channel).
    Script(ScriptId),
    /// A splice round the next use it wraps: on the branch given, or
    /// (`None`) the one her whims draw.
    Splice(SpliceId, Option<u8>),
}

impl Cue {
    /// Whether it plays on a use of `what` as it starts: never a trial
    /// sit (a moment on a piece she's trying where it stands is all
    /// that is); a splice round a use it wraps (of a piece she made if
    /// `makeshift`: see [`Splice::wraps`]); a script on a use of its
    /// piece, and surfing or the shopping channel only with nothing
    /// about her home on her mind (`grieved`), which would show over
    /// them. A riddle never: it waits for her to muse.
    pub fn plays_on(self, what: Use, trying: bool, grieved: bool, makeshift: bool) -> bool {
        !trying
            && match self {
                Self::Script(id) => {
                    id.played_on() == Some(what)
                        && !(grieved && matches!(id, ScriptId::Surf | ScriptId::Shopping))
                }
                Self::Splice(id, _) => id.row().wraps(what, makeshift),
            }
    }
}

/// The prelude and coda a use is wrapped in as it starts (`ctx`): of
/// `rows`, each that wraps it, is open today if it's rare (a closed one
/// is passed over before anything of it rolls), may (`when`, and a
/// prelude only when she's quiet), rolls its chance from `whims` and
/// hasn't played in the
/// last while (`lines` keeps what she's played), at most one before and
/// one after, its branch (so its length) drawn from `whims` too. Or, cued
/// (`forced`), that splice alone, if it wraps the use, whether she's
/// quiet or not (the caller hushes her for a cued prelude), on the branch
/// cued if any, and starting its cooldown as if rolled. `sure` (the
/// tests': every vignette, as often as it may) has every row that may
/// wrap the use roll as if certain. Never round a trial sit. Draws
/// nothing from her body's stream.
pub(super) fn splices(
    rows: &[SpliceId],
    ctx: &SpliceCtx<'_>,
    forced: Option<(SpliceId, Option<u8>)>,
    sure: bool,
    whims: Whims,
    lines: &mut Lines,
    at: u64,
) -> (Option<Spliced>, Option<Spliced>) {
    let mut wrapped = (None, None);
    if ctx.trying {
        return wrapped;
    }
    let forced_id = forced.map(|(id, _)| id);
    let candidates: &[SpliceId] = match &forced_id {
        Some(id) => std::slice::from_ref(id),
        None => rows,
    };
    for &id in candidates {
        let row = id.row();
        let slot = match row.at {
            Part::Before => &mut wrapped.0,
            Part::After => &mut wrapped.1,
        };
        if slot.is_some() || !row.wraps(ctx.what, ctx.makeshift) {
            continue;
        }
        if forced.is_none() {
            // Rare, and not open today: passed over before it rolls, so
            // a closed day rolls everything else as if it weren't a row.
            if !ctx.rares.allows(id.script()) {
                continue;
            }
            let (n, d) = (row.chance)(ctx);
            let may = (row.at == Part::After || ctx.quiet) && (row.when)(ctx);
            // The chance first: a splice that doesn't roll hasn't
            // played, so it doesn't cool.
            let rolled = sure || whims.chance(SPLICE, 2 * row.salt, n, d);
            if !may || !rolled || !lines.try_play(id.script(), at) {
                continue;
            }
        } else {
            lines.try_play(id.script(), at);
        }
        let cued = forced.and_then(|(_, branch)| branch).map(u64::from);
        let branch =
            cued.unwrap_or_else(|| whims.below_at(SPLICE, 2 * row.salt + 1, row.lens.len() as u64));
        let Some(&len) = row.lens.get(branch as usize) else {
            continue;
        };
        tracing::info!(splice = row.name, what = ?ctx.what, "houseguest: a splice round her use");
        *slot = Some(Spliced {
            splice: id,
            len,
            branch: u8::try_from(branch).unwrap_or(0),
        });
    }
    wrapped
}

/// What every splice's rolls are labelled (each salted by its row).
const SPLICE: &str = "splice";

/// A splice chosen for a use: which, how long it plays, and its branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Spliced {
    pub splice: SpliceId,
    pub len: u64,
    pub branch: u8,
}

impl Spliced {
    /// Its keys, and the body they play in.
    fn keys(self) -> (&'static [Key], Option<u64>) {
        (self.splice.script().keys(self.branch), Some(self.len))
    }
}

/// What a use plays, all chosen when it began: its own script and
/// branch, any prelude (`before`) and coda (`after`), the lines drawn
/// for it, what the shopping channel sold her (on a watch), and the
/// programme her TV holds (on a watch: [`Shows::Programme`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Play {
    pub own: ScriptId,
    pub branch: u8,
    pub before: Option<Spliced>,
    pub after: Option<Spliced>,
    pub drawn: [u8; 2],
    pub bought: Option<Furniture>,
    /// Drawn as every watch begins, from her decision's whims, whatever
    /// it plays (phase 5c D7: so what she does never hangs on whether a
    /// picture of the film came to show instead).
    pub card: Programme,
}

impl Play {
    /// `own`, plainly: no prelude, no coda, nothing drawn or bought.
    pub fn plain(own: ScriptId) -> Self {
        Self {
            own,
            branch: 0,
            before: None,
            after: None,
            drawn: [0; 2],
            bought: None,
            card: Programme::News,
        }
    }

    /// Riddle `which` of [`RIDDLES`], told spacing out.
    pub fn riddle(which: u8) -> Self {
        Self {
            drawn: [which, 0],
            ..Self::plain(ScriptId::Riddle)
        }
    }

    /// What a use of `what` plays plainly: the shopping channel if it
    /// sold her something (`bought`, on a watch), else its own script.
    pub fn of(what: Use, bought: Option<Furniture>) -> Self {
        let own = match bought {
            Some(_) => ScriptId::Shopping,
            None => what.script(),
        };
        Self {
            bought,
            ..Self::plain(own)
        }
    }

    /// When the body of a use from `since` begins (after any prelude).
    pub fn body_start(&self, since: u64) -> u64 {
        since + self.before.map_or(0, |s| s.len)
    }

    /// When the body of a use from `since` to `until` ends (before any
    /// coda).
    pub fn body_end(&self, since: u64, until: u64) -> u64 {
        until
            .saturating_sub(self.after.map_or(0, |s| s.len))
            .max(self.body_start(since))
    }

    /// The first time after `now` that a key of a use from `since` to
    /// `until` ends (so the next one starts): in its prelude, its body
    /// or its coda, each part's last key ending with the part. `None`
    /// once every key is over.
    pub fn next_end(&self, since: u64, until: u64, now: u64) -> Option<u64> {
        let start = self.body_start(since);
        let end = self.body_end(since, until);
        let parts = [
            self.before.map(|s| (s.keys(), since)),
            Some(((self.own.keys(self.branch), Some(end - start)), start)),
            self.after.map(|s| (s.keys(), end)),
        ];
        parts
            .into_iter()
            .flatten()
            .flat_map(|((keys, body), from)| {
                keys.iter()
                    .map(move |key| from.saturating_add(key.span.end(body)))
            })
            .filter(|&end| end > now)
            .min()
    }

    /// The splice playing at `now` in a use from `since` to `until`
    /// (its prelude, or its coda), if one is.
    pub fn spliced_at(&self, since: u64, until: u64, now: u64) -> Option<Spliced> {
        match (self.before, self.after) {
            (Some(before), _) if now < self.body_start(since) => Some(before),
            (_, Some(after)) if (self.body_end(since, until)..until).contains(&now) => Some(after),
            _ => None,
        }
    }

    /// The part of a use from `since` to `until` playing at `now` (the
    /// prelude, the body or the coda): its keys, when it starts, and how
    /// long it is. Everything timed within a part (its keys, a bob, what's
    /// on TV, the frame grid she wakes on) counts from its start.
    fn part(&self, since: u64, until: u64, now: u64) -> (&'static [Key], u64, Option<u64>) {
        let start = self.body_start(since);
        let end = self.body_end(since, until);
        match (self.before, self.after) {
            (Some(before), _) if now < start => {
                let (keys, body) = before.keys();
                (keys, since, body)
            }
            (_, Some(after)) if now >= end => {
                let (keys, body) = after.keys();
                (keys, end, body)
            }
            _ => (self.own.keys(self.branch), start, Some(end - start)),
        }
    }

    /// Where the key playing at `now` in a use from `since` to `until`
    /// is: its part's start, its index there, and whether it bobs.
    #[cfg(test)]
    pub fn key_place(&self, since: u64, until: u64, now: u64) -> Option<KeyPlace> {
        let (keys, from, body) = self.part(since, until, now);
        key_at(keys, now.saturating_sub(from), body).map(|(index, key, _)| KeyPlace {
            part: from,
            index,
            bobs: matches!(key.pose, Posed::Bob(..)),
        })
    }

    /// The key playing at `now` in a use from `since` to `until`, and
    /// when it plays in its part (the prelude, the body or the coda):
    /// how far `now` is into the part, and the key's start and end there.
    pub fn key(&self, since: u64, until: u64, now: u64) -> Option<(&'static Key, KeyTime)> {
        let (keys, from, body) = self.part(since, until, now);
        key_at(keys, now.saturating_sub(from), body).map(|(_, key, time)| (key, time))
    }

    /// What it plays at `now` in a use (or musing) from `since` to
    /// `until`, for the stage: each part by name in turn (any prelude,
    /// its own script, any coda), the one playing with which of its keys
    /// ("test snack 2/2 › homework › test sleep").
    pub fn note(&self, since: u64, until: u64, now: u64) -> String {
        let start = self.body_start(since);
        let end = self.body_end(since, until);
        let playing = if self.before.is_some() && now < start {
            0
        } else if self.after.is_some() && now >= end {
            2
        } else {
            1
        };
        let (keys, from, body) = self.part(since, until, now);
        let key = key_at(keys, now.saturating_sub(from), body).map_or(0, |(i, ..)| i + 1);
        let name = |s: Spliced| s.splice.row().name.to_owned();
        [
            self.before.map(name),
            Some(format!("{:?}", self.own).to_lowercase()),
            self.after.map(name),
        ]
        .into_iter()
        .enumerate()
        .filter_map(|(i, part)| {
            let part = part?;
            Some(if i == playing {
                format!("{part} {key}/{}", keys.len())
            } else {
                part
            })
        })
        .collect::<Vec<_>>()
        .join(" › ")
    }

    /// The next frame after `now`, `period` ms apart, of the part of a
    /// use from `since` to `until` playing at `now`: counted from that
    /// part's start, as [`Play::key`] times it (so a bob, or what's on
    /// TV, in a prelude or a coda moves on a frame too).
    pub fn next_frame(&self, since: u64, until: u64, now: u64, period: u64) -> u64 {
        let (_, from, _) = self.part(since, until, now);
        let period = period.max(1);
        from + (now.saturating_sub(from) / period + 1) * period
    }

    /// The next time after `now` that what the key playing at `now`
    /// shows moves ([`Key::frame_ms`]), in a use from `since` to
    /// `until`; `u64::MAX` if it holds.
    pub fn next_move(&self, since: u64, until: u64, now: u64) -> u64 {
        self.key(since, until, now)
            .and_then(|(key, _)| key.frame_ms())
            .map_or(u64::MAX, |period| {
                self.next_frame(since, until, now, period)
            })
    }
}

impl Use {
    /// The script that is how she does it.
    pub fn script(self) -> ScriptId {
        match self {
            Use::Lounge => ScriptId::Lounge,
            Use::Nap => ScriptId::Nap,
            Use::Sleep => ScriptId::Sleep,
            Use::Homework => ScriptId::Homework,
            Use::Watch => ScriptId::Watch,
            Use::Unpack => ScriptId::Unpack,
            Use::Read => ScriptId::Read,
            Use::Snack => ScriptId::Snack,
            Use::Pet => ScriptId::Pet,
            Use::Crumple => ScriptId::Crumple,
            Use::LookOut => ScriptId::LookOut,
        }
    }
}

/// A key with nothing on her furniture.
const fn key(span: Span, pose: Posed, face: Face, say: Option<Say>) -> Key {
    Key {
        span,
        pose,
        face,
        say,
        prop: None,
    }
}

/// A key with `prop` held on her furniture.
const fn shows(span: Span, pose: Posed, face: Face, say: Option<Say>, prop: Prop) -> Key {
    on(span, pose, face, say, Shows::Still(prop))
}

/// A key with `shows` on her furniture.
const fn on(span: Span, pose: Posed, face: Face, say: Option<Say>, shows: Shows) -> Key {
    Key {
        span,
        pose,
        face,
        say,
        prop: Some(shows),
    }
}

const fn bubble(bubble: Bubble) -> Option<Say> {
    Some(Say::Bubble(bubble))
}

const LOUNGE: &[&[Key]] = &[&[key(
    Span::Rest,
    Posed::Still(Pose::Lounge),
    Face::Vacant,
    None,
)]];

const NAP: &[&[Key]] = &[&[key(
    Span::Rest,
    Posed::Bob(Pose::Nap, USE_FRAME_MS),
    Face::Blink,
    bubble(Bubble::Zzz),
)]];

/// How long the lamp stays on as she settles into bed: lying still
/// (under two frames, so a bob there could never flip clear of its
/// key's start and end: Round 8), breathing once it's off.
pub(super) const LAMP_ON_MS: u64 = 2000;

/// The lamp on a moment as she settles (blinking, thoughtful), then off
/// while she sleeps; trying the bed (the second branch), off at once.
const SLEEP: &[&[Key]] = &[
    &[
        key(
            Span::Ms(LAMP_ON_MS),
            Posed::Still(Pose::Sleep(0)),
            Face::Blink,
            bubble(Bubble::Dots),
        ),
        shows(
            Span::Rest,
            Posed::Bob(Pose::Sleep, USE_FRAME_MS),
            Face::Blink,
            bubble(Bubble::Zzz),
            Prop::LampOff,
        ),
    ],
    &[shows(
        Span::Rest,
        Posed::Bob(Pose::Sleep, USE_FRAME_MS),
        Face::Blink,
        bubble(Bubble::Zzz),
        Prop::LampOff,
    )],
];

/// Her night: each surface's two branches ([`Surface::branch`]), the lamp
/// on a moment as she settles (the bed's, timed as [`SLEEP`]'s, so a sleep
/// running into bedtime becomes the night in place) or off at once. In
/// bed she breathes as she does sleeping by day, on a sofa as napping; on
/// the floor she lies still (the floor's host wakes only as a key ends).
const NIGHT: &[&[Key]] = &[
    &[
        key(
            Span::Ms(LAMP_ON_MS),
            Posed::Still(Pose::Sleep(0)),
            Face::Blink,
            bubble(Bubble::Dots),
        ),
        night(Posed::Bob(Pose::Sleep, USE_FRAME_MS)),
    ],
    &[night(Posed::Bob(Pose::Sleep, USE_FRAME_MS))],
    &[
        key(
            Span::Ms(LAMP_ON_MS),
            Posed::Still(Pose::Nap(0)),
            Face::Blink,
            bubble(Bubble::Dots),
        ),
        night(Posed::Bob(Pose::Nap, USE_FRAME_MS)),
    ],
    &[night(Posed::Bob(Pose::Nap, USE_FRAME_MS))],
    &[
        key(
            Span::Ms(LAMP_ON_MS),
            Posed::Still(Pose::LieBack(0)),
            Face::Blink,
            bubble(Bubble::Dots),
        ),
        night(Posed::Still(Pose::LieBack(0))),
    ],
    &[night(Posed::Still(Pose::LieBack(0)))],
];

/// Asleep for the night, posed `pose`, with the lamp off.
const fn night(pose: Posed) -> Key {
    shows(
        Span::Rest,
        pose,
        Face::Blink,
        bubble(Bubble::Zzz),
        Prop::LampOff,
    )
}

/// Writing, then nodding off onto the paper: each branch writes for a
/// share of the body (her mood's: see `stillness::NodOff`, whose
/// variants are these branches in order), nods for half what's left,
/// then sleeps on the paper. Branch 0, the first half, is what every mood
/// had before (and has, in the levers she ships with).
const HOMEWORK: &[&[Key]] = &[
    &homework((1, 2), (3, 4)),
    &homework((1, 3), (2, 3)),
    &homework((5, 6), (11, 12)),
];

/// Homework writing up to `write` of the way through the body, nodding
/// up to `nod`, then asleep on the paper.
const fn homework(write: (u64, u64), nod: (u64, u64)) -> [Key; 3] {
    [
        key(
            Span::Upto(write.0, write.1),
            Posed::Bob(Pose::Homework, USE_FRAME_MS),
            Face::Vacant,
            None,
        ),
        key(
            Span::Upto(nod.0, nod.1),
            Posed::Still(Pose::Homework(2)),
            Face::Blink,
            bubble(Bubble::Dots),
        ),
        key(
            Span::Rest,
            Posed::Still(Pose::Homework(3)),
            Face::Blink,
            bubble(Bubble::Zzz),
        ),
    ]
}

/// Her homework on the floor (phase 5c D5): on her front writing on
/// the paper out in front of her (branches 0 to 2), or on her back
/// reading the set text over her face (3 to 5); each nodding off where
/// her mood has her, at [`HOMEWORK`]'s shares in `stillness::NodOff`'s
/// order (see [`floor_homework_branch`]), then asleep: her head down on
/// her arms, or the book open over her eyes.
const FLOOR_HOMEWORK: &[&[Key]] = &[
    &floor_homework(Pose::FloorHomework, Pose::FloorHomework(2), (1, 2), (3, 4)),
    &floor_homework(Pose::FloorHomework, Pose::FloorHomework(2), (1, 3), (2, 3)),
    &floor_homework(
        Pose::FloorHomework,
        Pose::FloorHomework(2),
        (5, 6),
        (11, 12),
    ),
    &floor_homework(Pose::LieRead, Pose::LieRead(2), (1, 2), (3, 4)),
    &floor_homework(Pose::LieRead, Pose::LieRead(2), (1, 3), (2, 3)),
    &floor_homework(Pose::LieRead, Pose::LieRead(2), (5, 6), (11, 12)),
];

/// Her homework on the floor as [`HOMEWORK`] is at her desk: at it in
/// `at`'s frames up to `write` of the way through the body, nodding in
/// `dozed` up to `nod` (her eyes shut), then asleep in it.
const fn floor_homework(
    at: fn(u8) -> Pose,
    dozed: Pose,
    write: (u64, u64),
    nod: (u64, u64),
) -> [Key; 3] {
    [
        key(
            Span::Upto(write.0, write.1),
            Posed::Bob(at, USE_FRAME_MS),
            Face::Vacant,
            None,
        ),
        key(
            Span::Upto(nod.0, nod.1),
            Posed::Still(dozed),
            Face::Blink,
            bubble(Bubble::Dots),
        ),
        key(
            Span::Rest,
            Posed::Still(dozed),
            Face::Blink,
            bubble(Bubble::Zzz),
        ),
    ]
}

/// The branch of [`ScriptId::FloorHomework`] her homework on the floor
/// plays: nodding off at `nod`, on her front writing, or (`book`) on her
/// back reading.
pub(super) fn floor_homework_branch(nod: super::stillness::NodOff, book: bool) -> u8 {
    nod.branch() + if book { 3 } else { 0 }
}

/// Static as she switches it on, then the programme she drew, held.
const WATCH: &[&[Key]] = &[&[
    on(
        Span::Ms(STATIC_MS),
        Posed::Host,
        Face::Curious,
        None,
        Shows::Static,
    ),
    on(
        Span::Rest,
        Posed::Host,
        Face::Curious,
        None,
        Shows::Programme,
    ),
]];

/// How long each channel she flicks to before the sunrise stays on.
pub(super) const SURF_MS: u64 = 2 * USE_FRAME_MS;

/// Snow, colour bars, snow, a sunrise (ooh!), then the programme she
/// drew, held, humming, pleased with herself.
const SURF: &[&[Key]] = &[&[
    on(
        Span::Ms(SURF_MS),
        Posed::Host,
        Face::Curious,
        None,
        Shows::Static,
    ),
    shows(
        Span::Ms(2 * SURF_MS),
        Posed::Host,
        Face::Vacant,
        None,
        Prop::Tv(Channel::ColourBars),
    ),
    on(
        Span::Ms(3 * SURF_MS),
        Posed::Host,
        Face::Curious,
        None,
        Shows::Static,
    ),
    shows(
        Span::Ms(4 * SURF_MS),
        Posed::Host,
        Face::Curious,
        bubble(Bubble::Ooh),
        Prop::Tv(Channel::Sunrise),
    ),
    on(
        Span::Rest,
        Posed::Host,
        Face::Happy,
        bubble(Bubble::Hum),
        Shows::Programme,
    ),
]];

/// Hooked (Chiyo-chichi bobbing as he talks), then sold, then watching
/// on, he holding still.
const SHOPPING: &[&[Key]] = &[&[
    on(
        Span::Upto(2, 5),
        Posed::Host,
        Face::Curious,
        bubble(Bubble::Ooh),
        Shows::Hook,
    ),
    shows(
        Span::Upto(3, 5),
        Posed::Host,
        Face::Happy,
        Some(Say::Pitch),
        Prop::Tv(Channel::Shopping(0)),
    ),
    shows(
        Span::Rest,
        Posed::Host,
        Face::Curious,
        None,
        Prop::Tv(Channel::Shopping(0)),
    ),
]];

const READ: &[&[Key]] = &[&[key(
    Span::Rest,
    Posed::Bob(Pose::Read, USE_FRAME_MS),
    Face::Vacant,
    None,
)]];

/// A look in the fridge (standing open), then the melon bread.
const SNACK: &[&[Key]] = &[&[
    shows(
        Span::Ms(1500),
        Posed::Still(Pose::Side),
        Face::Curious,
        None,
        Prop::FridgeOpen,
    ),
    key(
        Span::Rest,
        Posed::Bob(Pose::Eat, USE_FRAME_MS),
        Face::Happy,
        None,
    ),
]];

/// Dashed home from school for her lunch: a look in the fridge (as a
/// snack's), then, turning with it, the line.
const DASH_LUNCH: &[&[Key]] = &[&[
    shows(
        Span::Ms(1500),
        Posed::Still(Pose::Side),
        Face::Curious,
        None,
        Prop::FridgeOpen,
    ),
    key(
        Span::Rest,
        Posed::Still(Pose::Stand),
        Face::Happy,
        bubble(Bubble::Say(FORGOT_LUNCH)),
    ),
]];

/// What she says as she has the lunch she dashed home for.
pub(super) const FORGOT_LUNCH: &str = line!("Forgot my lunch!");

/// Dashed home with no fridge to get to: she can't think what it was.
const DASH_FORGOT: &[&[Key]] = &[&[
    key(
        Span::Ms(1500),
        Posed::Still(Pose::Stand),
        Face::Curious,
        bubble(Bubble::Say(FORGOT_SOMETHING)),
    ),
    key(
        Span::Rest,
        Posed::Still(Pose::Stand),
        Face::Vacant,
        bubble(Bubble::Say(WHAT_WAS_IT)),
    ),
]];

/// Every line a script's keys have her say in a bubble of her own,
/// with its script (not the shopping channel's pitch, nor a riddle's
/// halves: those are pooled): the bubble-fit lint walks them beside
/// [`super::mind::all_lines`].
#[cfg(test)]
pub(super) fn script_lines() -> Vec<(ScriptId, &'static str)> {
    ScriptId::ALL
        .into_iter()
        .flat_map(|id| {
            id.branches().iter().flat_map(move |keys| {
                keys.iter().filter_map(move |key| match key.say {
                    Some(Say::Bubble(Bubble::Say(line))) => Some((id, line)),
                    _ => None,
                })
            })
        })
        .collect()
}

/// Setsubun's beans, out (each key a throw, foot to foot)...
pub(super) const ONI_WA_SOTO: &str = line!("Oni wa soto!");
/// ...and luck, in.
pub(super) const FUKU_WA_UCHI: &str = line!("Fuku wa uchi!");

/// One throw of Setsubun's beans: jumping on `frame`, saying `line`,
/// until `ms` in.
const fn throw(ms: u64, frame: u8, line: &'static str) -> Key {
    key(
        Span::Ms(ms),
        Posed::Still(Pose::Jack(frame)),
        Face::Happy,
        bubble(Bubble::Say(line)),
    )
}

/// How long each throw of Setsubun's beans lasts.
pub(super) const THROW_MS: u64 = 700;

/// Beans out, four throws, then luck in, four more (the last the rest).
const SETSUBUN: &[&[Key]] = &[&[
    throw(THROW_MS, 0, ONI_WA_SOTO),
    throw(2 * THROW_MS, 1, ONI_WA_SOTO),
    throw(3 * THROW_MS, 0, ONI_WA_SOTO),
    throw(4 * THROW_MS, 1, ONI_WA_SOTO),
    throw(5 * THROW_MS, 0, FUKU_WA_UCHI),
    throw(6 * THROW_MS, 1, FUKU_WA_UCHI),
    throw(7 * THROW_MS, 0, FUKU_WA_UCHI),
    key(
        Span::Rest,
        Posed::Still(Pose::Jack(1)),
        Face::Happy,
        bubble(Bubble::Say(FUKU_WA_UCHI)),
    ),
]];

/// New Year's Day on her TV.
pub(super) const FIRST_SUNRISE_LINE: &str = line!("Ooh... first sunrise.");

/// The sunrise on from the first moment ("Ooh... first sunrise."), then
/// watching it, humming.
const FIRST_SUNRISE: &[&[Key]] = &[&[
    shows(
        Span::Ms(4000),
        Posed::Host,
        Face::Curious,
        bubble(Bubble::Say(FIRST_SUNRISE_LINE)),
        Prop::Tv(Channel::Sunrise),
    ),
    shows(
        Span::Rest,
        Posed::Host,
        Face::Happy,
        bubble(Bubble::Hum),
        Prop::Tv(Channel::Sunrise),
    ),
]];

/// The Dream's lines, talked in her sleep, in turn.
pub(super) const EVERYNYAN: &str = line!("Hello everynyan...");
pub(super) const SANKYU: &str = line!("Fine sankyu...");
pub(super) const OH_MY_GAH: &str = line!("Oh my gah!");

/// Each of the Dream's lines shows this long, the last ending at
/// [`DREAM_MS`]; then she sleeps on. Three frames (the user, phase 5c's
/// tail): each line starts on her breathing's frame grid, and, holding a
/// frame clear of each end ([`bob_frame`]), her breathing flips twice
/// under it (at 3 s, off the grid, the second and third lines held her
/// still).
pub(super) const DREAM_LINE_MS: u64 = 3 * USE_FRAME_MS;
/// The Dream's lines, all said.
pub(super) const DREAM_MS: u64 = 3 * DREAM_LINE_MS;

/// One of the Dream's keys: asleep posed `pose`, talking in her sleep,
/// the lamp off, until `ms` in.
const fn dreaming(ms: u64, pose: Posed, line: &'static str) -> Key {
    shows(
        Span::Ms(ms),
        pose,
        Face::Blink,
        bubble(Bubble::Say(line)),
        Prop::LampOff,
    )
}

/// The Dream on a surface posed `pose`: its three lines, then asleep for
/// the night as before.
const fn dream(pose: Posed) -> [Key; 4] {
    [
        dreaming(DREAM_LINE_MS, pose, EVERYNYAN),
        dreaming(2 * DREAM_LINE_MS, pose, SANKYU),
        dreaming(DREAM_MS, pose, OH_MY_GAH),
        night(pose),
    ]
}

/// The Dream, each surface its branch ([`Surface::dream_branch`]): in
/// bed, on a sofa, on the floor.
const DREAM: &[&[Key]] = &[
    &dream(Posed::Bob(Pose::Sleep, USE_FRAME_MS)),
    &dream(Posed::Bob(Pose::Nap, USE_FRAME_MS)),
    &dream(Posed::Still(Pose::LieBack(0))),
];

/// What she says, the melon bread all gone.
pub(super) const LAST_ONE: &str = line!("That was the last one.");
/// She looks back in the fridge this long...
pub(super) const NO_MELON_LOOK_MS: u64 = 1200;
/// ...and it's all of it.
pub(super) const NO_MELON_MS: u64 = NO_MELON_LOOK_MS + 3000;

/// After the snack, back in the fridge (standing open) for another, then
/// turning from it, let down: none left.
const NO_MELON: &[&[Key]] = &[&[
    shows(
        Span::Ms(NO_MELON_LOOK_MS),
        Posed::Still(Pose::Side),
        Face::Curious,
        None,
        Prop::FridgeOpen,
    ),
    shows(
        Span::Rest,
        Posed::Still(Pose::Stand),
        Face::Droop,
        bubble(Bubble::Say(LAST_ONE)),
        Prop::FridgeOpen,
    ),
]];

/// Spacing out, she starts on something...
pub(super) const BOX_ONE: &str = line!("The box one's the...");
/// ...and isn't sure.
pub(super) const ESCALATOR_NO: &str = line!("...escalator? No?");
/// The first half shows this long (inside the shortest musing).
pub(super) const ESCALATOR_HALF_MS: u64 = 3000;

/// Which moving staircase is which: standing, blank, then looking up,
/// unsure.
const ESCALATOR: &[&[Key]] = &[&[
    key(
        Span::Ms(ESCALATOR_HALF_MS),
        Posed::Still(Pose::Stand),
        Face::Vacant,
        bubble(Bubble::Say(BOX_ONE)),
    ),
    key(
        Span::Rest,
        Posed::Still(Pose::Gaze),
        Face::Curious,
        bubble(Bubble::Say(ESCALATOR_NO)),
    ),
]];

/// Her story's opening...
pub(super) const STORY_TIME: &str = line!("Scary story time...");
/// ...and how it ends.
pub(super) const NOT_MINE: &str = line!("A fart. Not mine.");
/// The opening shows this long...
pub(super) const SCARY_OPEN_MS: u64 = 3000;
/// ...and it's all of it.
pub(super) const SCARY_MS: u64 = SCARY_OPEN_MS + 3000;
/// Her scary story comes from this minute of the day (22:00) until her
/// bedtime.
const SCARY_FROM: u16 = 22 * 60;

/// Hugging her knees for the story, solemn; then the ending, pleased
/// with it.
const SCARY: &[&[Key]] = &[&[
    key(
        Span::Ms(SCARY_OPEN_MS),
        Posed::Still(Pose::Sit),
        Face::Curious,
        bubble(Bubble::Say(STORY_TIME)),
    ),
    key(
        Span::Rest,
        Posed::Still(Pose::Sit),
        Face::Pleased,
        bubble(Bubble::Say(NOT_MINE)),
    ),
]];

/// Dashed home with no fridge to get to, first...
pub(super) const FORGOT_SOMETHING: &str = line!("Forgot somethin'...");
/// ...then.
pub(super) const WHAT_WAS_IT: &str = line!("...what was it?");

/// What she says looking out of the window, by the sky outside (two or
/// three a phase, so it doesn't repeat): each a branch of
/// [`ScriptId::LookOut`], in this order (see [`look_out_branch`]).
pub(super) const LOOK_OUT_LINES: [(Sky, &str); 11] = [
    (Sky::Day, line!("Sunny!")),
    (Sky::Day, line!("A bird!")),
    (Sky::Day, line!("That cloud's a bun.")),
    (Sky::Dawn, line!("Mornin', sky.")),
    (Sky::Dawn, line!("The sun's up!")),
    (Sky::Dusk, line!("Pretty...")),
    (Sky::Dusk, line!("The sky's on fire.")),
    (Sky::Evening, line!("Lights are on...")),
    (Sky::Evening, line!("First star!")),
    (Sky::Night, line!("Stars!")),
    (Sky::Night, line!("So many stars...")),
];

/// Looking out of the window, her line shows this long (inside the
/// shortest use there is).
pub(super) const LOOK_OUT_LINE_MS: u64 = 3000;

/// Leaning on the window's sill, chin in her hands, curious (phase 5c
/// D6): `line` first, then quietly (her musings on the sky come over it:
/// see `Osaka::sill_on`).
const fn looking(line: &'static str) -> [Key; 2] {
    [
        key(
            Span::Ms(LOOK_OUT_LINE_MS),
            Posed::Still(Pose::SillLean),
            Face::Curious,
            bubble(Bubble::Say(line)),
        ),
        key(
            Span::Rest,
            Posed::Still(Pose::SillLean),
            Face::Curious,
            None,
        ),
    ]
}

/// Looking out of the window, each of [`LOOK_OUT_LINES`] its branch.
const LOOK_OUT: &[&[Key]] = &[
    &looking(LOOK_OUT_LINES[0].1),
    &looking(LOOK_OUT_LINES[1].1),
    &looking(LOOK_OUT_LINES[2].1),
    &looking(LOOK_OUT_LINES[3].1),
    &looking(LOOK_OUT_LINES[4].1),
    &looking(LOOK_OUT_LINES[5].1),
    &looking(LOOK_OUT_LINES[6].1),
    &looking(LOOK_OUT_LINES[7].1),
    &looking(LOOK_OUT_LINES[8].1),
    &looking(LOOK_OUT_LINES[9].1),
    &looking(LOOK_OUT_LINES[10].1),
];

/// The branch of [`ScriptId::LookOut`] she plays looking out at `sky`:
/// the `pick`th (wrapping) of that sky's lines.
pub(super) fn look_out_branch(sky: Sky, pick: u64) -> u8 {
    let mine: Vec<usize> = LOOK_OUT_LINES
        .iter()
        .enumerate()
        .filter(|(_, (s, _))| *s == sky)
        .map(|(i, _)| i)
        .collect();
    let i = usize::try_from(pick % mine.len().max(1) as u64).unwrap_or(0);
    mine.get(i)
        .and_then(|&branch| u8::try_from(branch).ok())
        .unwrap_or(0)
}

/// Her glance up at the clock at bedtime...
pub(super) const ITS_LATE: &str = line!("Oh! It's late!");
/// ...and as she leaves for school.
pub(super) const SCHOOL_TIME: &str = line!("Time for school!");
/// The hour, roughly: noon, one to eleven (morning or evening), and
/// midnight (see [`hour_line`]). Of an afternoon, noon to five.
pub(super) const HOURS: [&str; 13] = [
    line!("Noon-ish."),
    line!("One-ish."),
    line!("Two-ish."),
    line!("Three-ish."),
    line!("Four-ish."),
    line!("Five-ish."),
    line!("Six-ish."),
    line!("Seven-ish."),
    line!("Eight-ish."),
    line!("Nine-ish."),
    line!("Ten-ish."),
    line!("Eleven-ish."),
    line!("Midnight-ish."),
];

/// Which of [`HOURS`] says `hour` (0 to 23), roughly.
pub(super) fn hour_line(hour: u16) -> usize {
    match hour % 24 {
        0 => 12,
        h => usize::from(h % 12),
    }
}

/// Which glance up at the clock: a branch of [`ScriptId::ClockGlance`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClockGlance {
    /// Bedtime: "Oh! It's late!".
    Bed,
    /// Off to school: "Time for school!".
    School,
    /// The game `hour` (0 to 23): it, roughly (of an afternoon, the
    /// hours 12 to 17; cued by the stage, any).
    Hour(u16),
}

impl ClockGlance {
    /// Its branch.
    pub fn branch(self) -> u8 {
        match self {
            Self::Bed => 0,
            Self::School => 1,
            // From noon (the first of the afternoon's, 12:45) on, as
            // [`HOURS`] has them.
            Self::Hour(hour) => 2 + u8::try_from(hour_line(hour)).unwrap_or(0),
        }
    }
}

/// A glance up at the clock, saying `line` with `face`.
const fn glancing(face: Face, line: &'static str) -> [Key; 1] {
    [key(
        Span::Rest,
        Posed::Still(Pose::Gaze),
        face,
        bubble(Bubble::Say(line)),
    )]
}

/// Her glances up at the clock, each its branch ([`ClockGlance::branch`]).
const CLOCK_GLANCE: &[&[Key]] = &[
    &glancing(Face::Surprised, ITS_LATE),
    &glancing(Face::Happy, SCHOOL_TIME),
    &glancing(Face::Curious, HOURS[0]),
    &glancing(Face::Curious, HOURS[1]),
    &glancing(Face::Curious, HOURS[2]),
    &glancing(Face::Curious, HOURS[3]),
    &glancing(Face::Curious, HOURS[4]),
    &glancing(Face::Curious, HOURS[5]),
    &glancing(Face::Curious, HOURS[6]),
    &glancing(Face::Curious, HOURS[7]),
    &glancing(Face::Curious, HOURS[8]),
    &glancing(Face::Curious, HOURS[9]),
    &glancing(Face::Curious, HOURS[10]),
    &glancing(Face::Curious, HOURS[11]),
    &glancing(Face::Curious, HOURS[12]),
];

/// Petting the cat, until he bites.
const PET: &[&[Key]] = &[&[
    key(
        Span::Upto(7, 10),
        Posed::Still(Pose::Pet(0)),
        Face::Happy,
        bubble(Bubble::Hum),
    ),
    shows(
        Span::Rest,
        Posed::Still(Pose::Pet(1)),
        Face::Surprised,
        bubble(Bubble::Say(line!("Ow!"))),
        Prop::CatBiting,
    ),
]];

const CRUMPLE: &[&[Key]] = &[&[
    key(
        Span::Upto(4, 5),
        Posed::Bob(Pose::ToeTouch, USE_FRAME_MS),
        Face::Happy,
        bubble(Bubble::Say(SCRUNCH)),
    ),
    key(
        Span::Rest,
        Posed::Bob(Pose::ToeTouch, USE_FRAME_MS),
        Face::Happy,
        bubble(Bubble::Say(THERE)),
    ),
]];

const UNPACK: &[&[Key]] = &[&[
    key(
        Span::Upto(3, 5),
        Posed::Bob(Pose::ToeTouch, USE_FRAME_MS),
        Face::Happy,
        None,
    ),
    key(
        Span::Rest,
        Posed::Bob(Pose::ToeTouch, USE_FRAME_MS),
        Face::Happy,
        bubble(Bubble::Ooh),
    ),
]];

/// The question (Curious), her answer at once (Happy), then pleased
/// with it.
const RIDDLE: &[&[Key]] = &[&[
    key(
        Span::Ms(RIDDLE_ASKED_MS),
        Posed::Still(Pose::Stand),
        Face::Curious,
        Some(Say::Riddle),
    ),
    key(
        Span::Ms(RIDDLE_ANSWERED_MS),
        Posed::Still(Pose::Stand),
        Face::Happy,
        Some(Say::Answer),
    ),
    key(
        Span::Rest,
        Posed::Still(Pose::Stand),
        Face::Happy,
        bubble(Bubble::Hehe),
    ),
]];

/// The chopsticks stay joined this long, she splits them by this far
/// in, and what came of it shows to this far (as long as she takes to
/// say "Hold 'em by the ends!"); then she's pleased, or trails off, to
/// the end.
pub(super) const CHOPSTICKS_JOINED_MS: u64 = 1000;
pub(super) const CHOPSTICKS_SPLIT_MS: u64 = CHOPSTICKS_JOINED_MS + 800;
pub(super) const CHOPSTICKS_SHOWN_MS: u64 = CHOPSTICKS_SPLIT_MS + 2500;
/// The whole of it, either way.
pub(super) const CHOPSTICKS_MS: u64 = CHOPSTICKS_SHOWN_MS + 1400;

/// What she says splitting them badly.
pub(super) const HOLD_EM: &str = line!("Hold 'em by the ends!");

/// Splitting a pair of disposable chopsticks: held joined (blank),
/// pulled apart, then (clean) a twinkle and a giggle, or (bad) let down,
/// telling herself off, then trailing off. Each split frame bakes in its
/// face in ASCII, so the face matches it from the split on.
const CHOPSTICKS: &[&[Key]] = &[
    &[
        key(
            Span::Ms(CHOPSTICKS_JOINED_MS),
            Posed::Still(Pose::Chopsticks(0)),
            Face::Vacant,
            None,
        ),
        key(
            Span::Ms(CHOPSTICKS_SPLIT_MS),
            Posed::Still(Pose::Chopsticks(1)),
            Face::Happy,
            None,
        ),
        key(
            Span::Ms(CHOPSTICKS_SHOWN_MS),
            Posed::Still(Pose::Chopsticks(1)),
            Face::Happy,
            bubble(Bubble::Sparkle),
        ),
        key(
            Span::Rest,
            Posed::Still(Pose::Chopsticks(1)),
            Face::Happy,
            bubble(Bubble::Hehe),
        ),
    ],
    &[
        key(
            Span::Ms(CHOPSTICKS_JOINED_MS),
            Posed::Still(Pose::Chopsticks(0)),
            Face::Vacant,
            None,
        ),
        key(
            Span::Ms(CHOPSTICKS_SPLIT_MS),
            Posed::Still(Pose::Chopsticks(2)),
            Face::Droop,
            None,
        ),
        key(
            Span::Ms(CHOPSTICKS_SHOWN_MS),
            Posed::Still(Pose::Chopsticks(2)),
            Face::Droop,
            bubble(Bubble::Say(HOLD_EM)),
        ),
        key(
            Span::Rest,
            Posed::Still(Pose::Chopsticks(2)),
            Face::Droop,
            bubble(Bubble::Dots),
        ),
    ],
];

/// What she says of the andagi (and to anyone asking her anything
/// meanwhile).
pub(super) const SATA_ANDAGI: &str = line!("Sata andagi.");
/// She finds it in the fridge (standing open) this long.
pub(super) const ANDAGI_FOUND_MS: u64 = 1000;
/// Each "Sata andagi." shows this long (about as long as she takes to
/// say it)...
pub(super) const ANDAGI_SAID_MS: u64 = 1900;
/// ...then a quiet beat, so the next reads as said again.
pub(super) const ANDAGI_BEAT_MS: u64 = 600;
/// The first bite.
pub(super) const ANDAGI_BITE_MS: u64 = USE_FRAME_MS;
/// Then eating it, two frames: chewing, then biting again, a frame each
/// (two keys, not a bob: a bob's first and last frames are held a frame
/// clear of its key's ends, so off the grid as this is, a bob two frames
/// long would never flip).
pub(super) const ANDAGI_EAT_MS: u64 = 2 * USE_FRAME_MS;

/// How long the andagi plays, said `count` times.
pub(super) const fn andagi_ms(count: u64) -> u64 {
    ANDAGI_FOUND_MS + count * (ANDAGI_SAID_MS + ANDAGI_BEAT_MS) + ANDAGI_BITE_MS + ANDAGI_EAT_MS
}

/// The andagi said `(N - 4) / 2` times: the fridge open as she finds
/// it, then holding it up, "Sata andagi." and a quiet beat each time,
/// blank, then pleased, then happy; then a bite, and she eats it,
/// chewing what's left of it in her hand, then biting again.
const fn andagi<const N: usize>() -> [Key; N] {
    let count = (N - 4) / 2;
    let held = Posed::Still(Pose::EatAndagi(0));
    let mut keys = [key(Span::Rest, held, Face::Happy, None); N];
    keys[0] = shows(
        Span::Ms(ANDAGI_FOUND_MS),
        Posed::Still(Pose::Side),
        Face::Curious,
        None,
        Prop::FridgeOpen,
    );
    let mut i = 0;
    while i < count {
        let face = match i * 3 / count {
            0 => Face::Vacant,
            1 => Face::Pleased,
            _ => Face::Happy,
        };
        let from = ANDAGI_FOUND_MS + i as u64 * (ANDAGI_SAID_MS + ANDAGI_BEAT_MS);
        keys[1 + 2 * i] = key(
            Span::Ms(from + ANDAGI_SAID_MS),
            held,
            face,
            bubble(Bubble::Say(SATA_ANDAGI)),
        );
        keys[2 + 2 * i] = key(
            Span::Ms(from + ANDAGI_SAID_MS + ANDAGI_BEAT_MS),
            held,
            face,
            None,
        );
        i += 1;
    }
    keys[N - 3] = key(
        Span::Ms(andagi_ms(count as u64) - ANDAGI_EAT_MS),
        Posed::Still(Pose::EatAndagi(1)),
        Face::Happy,
        None,
    );
    keys[N - 2] = key(
        Span::Ms(andagi_ms(count as u64) - ANDAGI_EAT_MS + USE_FRAME_MS),
        Posed::Still(Pose::EatAndagi(2)),
        Face::Happy,
        None,
    );
    keys[N - 1] = key(
        Span::Rest,
        Posed::Still(Pose::EatAndagi(1)),
        Face::Happy,
        None,
    );
    keys
}

/// How many times she names it, by branch: four, five or six.
pub(super) const ANDAGI_COUNTS: [u64; 3] = [4, 5, 6];

/// How long it plays, by branch.
const ANDAGI_LENS: &[u64] = &[
    andagi_ms(ANDAGI_COUNTS[0]),
    andagi_ms(ANDAGI_COUNTS[1]),
    andagi_ms(ANDAGI_COUNTS[2]),
];

/// Its keys, by branch (two a naming, and four more).
const ANDAGI: &[&[Key]] = &[
    &andagi::<{ 2 * ANDAGI_COUNTS[0] as usize + 4 }>(),
    &andagi::<{ 2 * ANDAGI_COUNTS[1] as usize + 4 }>(),
    &andagi::<{ 2 * ANDAGI_COUNTS[2] as usize + 4 }>(),
];

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::ui::houseguest::mind::Lines;

    /// The bodies the lints try spans in: the shortest use there is (a
    /// trial sit), and lengths either side of every share's rounding.
    const BODIES: [u64; 6] = [3500, 3501, 5000, 6003, 20_004, 180_000];

    /// The lints' shortest body is the shortest use there is.
    #[test]
    fn the_lints_try_the_shortest_use() {
        assert_eq!(BODIES[0], super::super::osaka::shortest_use_ms());
    }

    /// Where `id` is in [`ScriptId::ALL`]. Wildcard-free, so a new
    /// script doesn't compile until it's given its place here, and
    /// [`every_script_is_listed`] holds it to that place: listed, the
    /// lints see it.
    fn listed_at(id: ScriptId) -> usize {
        match id {
            ScriptId::Lounge => 0,
            ScriptId::Nap => 1,
            ScriptId::Sleep => 2,
            ScriptId::Homework => 3,
            ScriptId::Watch => 4,
            ScriptId::Shopping => 5,
            ScriptId::Read => 6,
            ScriptId::Snack => 7,
            ScriptId::Pet => 8,
            ScriptId::Crumple => 9,
            ScriptId::Unpack => 10,
            ScriptId::Riddle => 11,
            ScriptId::Surf => 12,
            ScriptId::Chopsticks => 13,
            ScriptId::Andagi => 14,
            ScriptId::Night => 15,
            ScriptId::DashLunch => 16,
            ScriptId::DashForgot => 17,
            ScriptId::Setsubun => 18,
            ScriptId::FirstSunrise => 19,
            ScriptId::Dream => 20,
            ScriptId::NoMelon => 21,
            ScriptId::Escalator => 22,
            ScriptId::Scary => 23,
            ScriptId::LookOut => 24,
            ScriptId::ClockGlance => 25,
            ScriptId::FloorHomework => 26,
        }
    }

    /// Lint: every script is listed in [`ScriptId::ALL`] (see
    /// [`listed_at`]), every splice that is a row in [`SpliceId::ALL`],
    /// and every use in [`Use::ALL`], each once; and every script is
    /// some use's, the shopping channel's, a musing's or a splice row's:
    /// none goes unplayed.
    #[test]
    fn every_script_is_listed() {
        for (i, id) in ScriptId::ALL.into_iter().enumerate() {
            assert_eq!(listed_at(id), i, "{id:?}");
        }
        // Wildcard-free: a new splice doesn't compile until it's said
        // here whether it's a row (and so listed).
        let row = |id: SpliceId| match id {
            SpliceId::Chopsticks | SpliceId::Andagi | SpliceId::NoMelon | SpliceId::Scary => true,
            SpliceId::TestSnack | SpliceId::TestSleep | SpliceId::TestBedtime => false,
        };
        for id in SPLICES {
            assert_eq!(row(id), SpliceId::ALL.contains(&id), "{id:?}");
        }
        assert!(SpliceId::ALL.iter().all(|&id| row(id)));
        // Wildcard-free: a new use doesn't compile until it's listed.
        let use_at = |u: Use| match u {
            Use::Lounge => 0,
            Use::Nap => 1,
            Use::Sleep => 2,
            Use::Homework => 3,
            Use::Watch => 4,
            Use::Unpack => 5,
            Use::Read => 6,
            Use::Snack => 7,
            Use::Pet => 8,
            Use::Crumple => 9,
            Use::LookOut => 10,
        };
        for (i, u) in Use::ALL.into_iter().enumerate() {
            assert_eq!(use_at(u), i, "{u:?}");
            assert!(ScriptId::ALL.contains(&u.script()), "{u:?}");
        }
        // And every script is played by something. Wildcard-free: a new
        // script doesn't compile until it says what plays it.
        let spliced = |splice: SpliceId| Play {
            after: Some(Spliced {
                splice,
                len: 1,
                branch: 0,
            }),
            ..Play::of(Use::Homework, None)
        };
        let player = |id: ScriptId| match id {
            ScriptId::Chopsticks => spliced(SpliceId::Chopsticks),
            ScriptId::Andagi => spliced(SpliceId::Andagi),
            ScriptId::Lounge => Play::of(Use::Lounge, None),
            ScriptId::Nap => Play::of(Use::Nap, None),
            ScriptId::Sleep => Play::of(Use::Sleep, None),
            ScriptId::Homework => Play::of(Use::Homework, None),
            ScriptId::Watch => Play::of(Use::Watch, None),
            ScriptId::Shopping => Play::of(Use::Watch, Some(Furniture::Lamp)),
            ScriptId::Read => Play::of(Use::Read, None),
            ScriptId::Snack => Play::of(Use::Snack, None),
            ScriptId::Pet => Play::of(Use::Pet, None),
            ScriptId::Crumple => Play::of(Use::Crumple, None),
            ScriptId::Unpack => Play::of(Use::Unpack, None),
            ScriptId::Riddle => Play::riddle(0),
            ScriptId::Surf => Play::plain(ScriptId::Surf),
            ScriptId::Night => Play::plain(ScriptId::Night),
            ScriptId::DashLunch => Play::plain(ScriptId::DashLunch),
            ScriptId::DashForgot => Play::plain(ScriptId::DashForgot),
            ScriptId::Setsubun => Play::plain(ScriptId::Setsubun),
            ScriptId::FirstSunrise => Play::plain(ScriptId::FirstSunrise),
            ScriptId::Dream => Play::plain(ScriptId::Dream),
            ScriptId::NoMelon => spliced(SpliceId::NoMelon),
            ScriptId::Escalator => Play::plain(ScriptId::Escalator),
            ScriptId::Scary => spliced(SpliceId::Scary),
            ScriptId::LookOut => Play::of(Use::LookOut, None),
            ScriptId::ClockGlance => Play::plain(ScriptId::ClockGlance),
            ScriptId::FloorHomework => Play::plain(ScriptId::FloorHomework),
        };
        for id in ScriptId::ALL {
            let play = player(id);
            let plays = match id.host() {
                Host::Use | Host::SpaceOut | Host::Night | Host::Floor => play.own,
                Host::Splice => {
                    let splice = play.after.unwrap().splice;
                    assert!(SpliceId::ALL.contains(&splice), "{id:?}: not a row's");
                    splice.script()
                }
            };
            assert_eq!(plays, id, "{id:?} isn't what plays it");
        }
    }

    /// Every key's end is at or past the one before, and each run ends
    /// with the rest of the body, so the player never skips a key that
    /// should play and the body is filled.
    #[test]
    fn every_script_fills_its_body_in_order() {
        for id in ScriptId::ALL {
            for (branch, keys) in id.branches().iter().enumerate() {
                assert!(!keys.is_empty(), "{id:?}/{branch}: no keys");
                assert!(
                    matches!(keys.last().map(|k| k.span), Some(Span::Rest)),
                    "{id:?}/{branch}: the last key isn't the rest"
                );
                for body in BODIES {
                    let ends: Vec<u64> = keys.iter().map(|k| k.span.end(Some(body))).collect();
                    assert!(
                        ends.windows(2).all(|w| w[0] <= w[1]),
                        "{id:?}/{branch} in {body}: {ends:?}"
                    );
                }
            }
        }
    }

    /// A key that ends a set time in ends inside the shortest body its
    /// host can have (a trial sit, for a use; a musing, spacing out), or
    /// it would be cut short there. (A splice's keys end inside its
    /// branch's length: see the splice lint.)
    #[test]
    fn every_set_time_fits_its_shortest_host() {
        for id in ScriptId::ALL {
            let Some(shortest) = id.shortest_body() else {
                assert_eq!(id.host(), Host::Splice, "{id:?}");
                continue;
            };
            for keys in id.branches() {
                for key in *keys {
                    if let Span::Ms(ms) = key.span {
                        assert!(ms <= shortest, "{id:?}: {ms} > {shortest}");
                    }
                }
            }
        }
    }

    /// Only static and the hook move, and a programme is the act's
    /// (phase 5c D7): no key holds a frame of static still, or names a
    /// programme of its own instead of the one the act drew. (Wildcard-
    /// free, so a new channel says which it is.)
    #[test]
    fn no_key_holds_static_or_a_programme_of_its_own() {
        use crate::ui::houseguest::art::Channel;
        for id in ScriptId::ALL {
            for (branch, keys) in id.branches().iter().enumerate() {
                for key in *keys {
                    let Some(Shows::Still(Prop::Tv(channel))) = key.prop else {
                        continue;
                    };
                    let held = match channel {
                        Channel::ColourBars | Channel::Sunrise | Channel::Shopping(_) => true,
                        Channel::Snow(_) | Channel::Programme(_) => false,
                    };
                    assert!(held, "{id:?}/{branch}: holds {channel:?} still");
                }
            }
        }
    }

    /// Spacing out, she wakes only as a key ends (no frame grid, as a
    /// use has), and nothing shows on her furniture: so a script played
    /// spacing out neither bobs (it would freeze) nor shows a prop.
    #[test]
    fn spacing_out_neither_bobs_nor_shows_a_prop() {
        for id in ScriptId::ALL {
            if id.host() != Host::SpaceOut {
                continue;
            }
            for key in id.branches().iter().flat_map(|keys| keys.iter()) {
                assert!(matches!(key.pose, Posed::Still(_)), "{id:?}: {key:?}");
                assert_eq!(key.prop, None, "{id:?}");
            }
        }
    }

    /// A riddle says its question (Curious), then its answer (Happy),
    /// then is pleased with it (Happy, Hehe), standing throughout: each
    /// riddle's own, and both ends of a key half-open.
    #[test]
    fn a_riddle_asks_then_answers() {
        for (i, &(question, answer)) in RIDDLES.iter().enumerate() {
            let play = Play::riddle(u8::try_from(i).unwrap());
            let (since, until) = (1000, 1000 + super::super::osaka::SPACE_OUT_MS.0);
            let look = |now| {
                let (key, time) = play.key(since, until, now).unwrap();
                key.look(time, Pose::Stand, &play)
            };
            let said = |now| look(now).2;
            // Standing throughout.
            for now in (since..until).step_by(250).chain([until - 1]) {
                assert_eq!(look(now).0, Pose::Stand, "{now}");
            }
            assert_eq!(look(since).1, Face::Curious);
            assert_eq!(said(since), Some(Bubble::Say(question)));
            assert_eq!(
                said(since + RIDDLE_ASKED_MS - 1),
                Some(Bubble::Say(question))
            );
            assert_eq!(look(since + RIDDLE_ASKED_MS).1, Face::Happy);
            assert_eq!(said(since + RIDDLE_ASKED_MS), Some(Bubble::Say(answer)));
            assert_eq!(
                said(since + RIDDLE_ANSWERED_MS - 1),
                Some(Bubble::Say(answer))
            );
            assert_eq!(said(since + RIDDLE_ANSWERED_MS), Some(Bubble::Hehe));
            assert_eq!(look(since + RIDDLE_ANSWERED_MS).1, Face::Happy);
            assert_eq!(said(until - 1), Some(Bubble::Hehe));
            assert_eq!(look(until - 1).1, Face::Happy);
            let ends: Vec<u64> =
                std::iter::successors(Some(since), |&now| play.next_end(since, until, now))
                    .collect();
            assert_eq!(
                ends,
                [
                    since,
                    since + RIDDLE_ASKED_MS,
                    since + RIDDLE_ANSWERED_MS,
                    until
                ]
            );
        }
    }

    /// Only a watch leaves her pose to its host (sitting, or lounging on
    /// a sofa): anywhere else the host's pose is meaningless.
    #[test]
    fn only_a_watch_leaves_the_pose_to_its_host() {
        for id in ScriptId::ALL {
            let hosted = id
                .branches()
                .iter()
                .flat_map(|keys| keys.iter())
                .any(|k| matches!(k.pose, Posed::Host));
            let watch = matches!(
                id,
                ScriptId::Watch | ScriptId::Shopping | ScriptId::Surf | ScriptId::FirstSunrise
            );
            assert!(!hosted || watch, "{id:?}");
        }
    }

    /// Every bob's period is a whole number of frames, so her wakeups on
    /// the frame grid (counted from the same start) catch every change of
    /// a bob: none is left frozen between wakeups, or shown late.
    #[test]
    fn every_bob_moves_on_the_frame_grid() {
        for id in ScriptId::ALL {
            for key in id.branches().iter().flat_map(|keys| keys.iter()) {
                if let Posed::Bob(_, period) = key.pose {
                    assert!(
                        period > 0 && period % USE_FRAME_MS == 0,
                        "{id:?}: a bob every {period} ms"
                    );
                }
            }
        }
    }

    /// Her pose `t` ms into a part playing `keys` over a body `body` ms
    /// long, and the key playing then (its index; whether it bobs).
    fn pose_in(keys: &[Key], body: u64, t: u64) -> Option<(Pose, usize, bool)> {
        let play = Play::plain(ScriptId::Lounge);
        let (index, key, time) = key_at(keys, t, Some(body))?;
        let (pose, ..) = key.look(time, Pose::Stand, &play);
        Some((pose, index, matches!(key.pose, Posed::Bob(..))))
    }

    /// A bob's flips keep a frame ([`USE_FRAME_MS`]) clear of its key's
    /// start and its end (Round 8, the user: hold a bob's last frame
    /// through the part of a period before its key ends, and its first
    /// flip until a frame after its key starts), so a bobbing key's
    /// flips and the changes as it starts and ends are never within a
    /// frame of each other: for every bobbing key of every script's every
    /// branch and every splice, over bodies of every length from 2 s to
    /// 200 s (in steps of 997 ms, so its ends at a share of the body fall
    /// all about the frame grid) and each splice's own. Her pose can only
    /// change on the grid or as a key ends, so those moments are all it
    /// looks at.
    #[test]
    fn a_bob_flips_a_frame_clear_of_its_keys_start_and_end() {
        let mut parts: Vec<(String, &'static [Key], u64)> = Vec::new();
        for id in ScriptId::ALL {
            for (branch, keys) in id.branches().iter().enumerate() {
                for body in (2_000..200_000).step_by(997) {
                    parts.push((format!("{id:?} branch {branch}"), keys, body));
                }
            }
        }
        for splice in SpliceId::ALL {
            for (branch, &len) in splice.row().lens.iter().enumerate() {
                let keys = splice.script().keys(branch as u8);
                parts.push((format!("{splice:?} branch {branch}"), keys, len));
            }
        }
        let mut flips = 0u32;
        for (at, keys, body) in parts {
            let ends: Vec<u64> = keys.iter().map(|k| k.span.end(Some(body))).collect();
            // Each key's start and end, in the body.
            let bounds = |index: usize| {
                let start = ends[..index].iter().copied().max().unwrap_or(0);
                (start, ends[index])
            };
            let mut moments: Vec<u64> = (1..)
                .map(|k| k * USE_FRAME_MS)
                .take_while(|&t| t < body)
                .chain(ends.iter().copied().filter(|&t| t > 0 && t < body))
                .collect();
            moments.sort_unstable();
            moments.dedup();
            for t in moments {
                let (Some(was), Some(now)) = (pose_in(keys, body, t - 1), pose_in(keys, body, t))
                else {
                    continue;
                };
                let ((pose, index, bobs), (pose2, index2, _)) = (was, now);
                if index != index2 || !bobs || pose == pose2 {
                    continue;
                }
                flips += 1;
                let (start, end) = bounds(index);
                assert!(
                    t >= start + USE_FRAME_MS && t + USE_FRAME_MS <= end.min(body),
                    "{at}, body {body}: key {index} ({start} to {end}) flips at {t}"
                );
            }
        }
        assert!(flips > 1000, "{flips} flips tried");
    }

    /// [`bob_frame`] against its rule, at every ms from a bob's start to
    /// a period past its end: it starts on the grid's frame at its start
    /// (`start / period`, odd or even), flips on the grid and only there,
    /// never less than a period after its start nor less than a period
    /// before its end (nor after it), and on every grid point between:
    /// so its flips are exactly the grid's in `[start + period, end -
    /// period]`.
    fn check_bob_frame(start: u64, end: u64, period: u64) {
        let case = format!("{start} to {end}, every {period}");
        let frame = |t| bob_frame(t, start, end, period, None);
        assert_eq!(u64::from(frame(start)), start / period % 2, "{case}");
        let mut flips = 0;
        for t in start + 1..end + period {
            if frame(t) == frame(t - 1) {
                continue;
            }
            flips += 1;
            assert!(
                t.is_multiple_of(period) && t >= start + period && t + period <= end,
                "{case}: flips at {t}"
            );
        }
        let allowed = (start + period..=end.saturating_sub(period))
            .filter(|t| t.is_multiple_of(period))
            .count();
        assert_eq!(
            flips, allowed,
            "{case}: flips on every grid point clear of its ends"
        );
    }

    /// [`bob_frame`] at the edges: bobs starting and ending a ms either
    /// side of the grid, on it, and half a period off, for bobs a period
    /// long (or a ms less or more) to several, so a flip a ms within a
    /// period of either end shows (Round 8: holding the first and last
    /// frame). Then any bob ([`check_bob_frame`]).
    #[test]
    fn a_bob_frame_flips_on_the_grid_clear_of_its_ends() {
        for period in [3, USE_FRAME_MS, 2 * USE_FRAME_MS] {
            for g in 0..4 {
                for start in [g * period, g * period + 1, g * period + period / 2]
                    .into_iter()
                    .chain((g * period).checked_sub(1))
                {
                    for len in [1, 2, 3, 5]
                        .into_iter()
                        .flat_map(|k| [k * period - 1, k * period, k * period + 1])
                        .chain([period / 2, 5 * period + 7])
                    {
                        check_bob_frame(start, start + len, period);
                    }
                }
            }
        }
        // A key from a ms past the grid holds its first frame through the
        // grid point a ms under a period on, flipping at the next.
        let frame = |t| bob_frame(t, 1401, 100_000, USE_FRAME_MS, None);
        assert_eq!((frame(1401), frame(2799), frame(2800)), (1, 1, 1));
        assert_eq!((frame(4199), frame(4200)), (1, 0));
        // One ending a ms short of a period past the grid holds its last
        // through it.
        let frame = |t| bob_frame(t, 0, 86_799, USE_FRAME_MS, None);
        assert_eq!(
            (frame(84_000), frame(85_399), frame(85_400), frame(86_798)),
            (0, 0, 0, 0)
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(
            dessplay_core::test_support::proptest_cases(64)
        ))]

        /// Any bob, at any start and length, keeps [`check_bob_frame`]'s
        /// rule.
        #[test]
        fn any_bob_frame_flips_on_the_grid_clear_of_its_ends(
            start in 0u64..20_000,
            len in 1u64..12_000,
            period in proptest::sample::select(vec![700u64, USE_FRAME_MS, 2 * USE_FRAME_MS]),
        ) {
            check_bob_frame(start, start + len, period);
        }

        /// Any bob held at any moment of it, on either frame (phase 5c's
        /// tail, T4): it shows that frame at the moment, and from there
        /// flips on the grid and only there, never less than a period
        /// after the moment nor less than a period before its end, and on
        /// every grid point between. Before the moment it's the bob
        /// unheld. A hold from before its start (a part or key begun
        /// since) holds nothing: it's the bob unheld throughout.
        #[test]
        fn any_held_bob_frame_holds_from_its_moment(
            start in 0u64..20_000,
            len in 1u64..12_000,
            at in 0u64..12_000,
            shown in 0u8..2,
            early in 1u64..20_000,
            period in proptest::sample::select(vec![700u64, USE_FRAME_MS, 2 * USE_FRAME_MS]),
        ) {
            let (end, at) = (start + len, start + at % len);
            if let Some(before) = start.checked_sub(early) {
                for t in start..end + period {
                    proptest::prop_assert_eq!(
                        bob_frame(t, start, end, period, Some((before, shown))),
                        bob_frame(t, start, end, period, None),
                        "{} to {}, held at {} before it, every {}, at {}", start, end, before, period, t
                    );
                }
            }
            let case = format!("{start} to {end}, held at {at} on {shown}, every {period}");
            let held = |t| bob_frame(t, start, end, period, Some((at, shown)));
            for t in start..at {
                proptest::prop_assert_eq!(held(t), bob_frame(t, start, end, period, None), "{} at {}", case, t);
            }
            proptest::prop_assert_eq!(held(at), shown, "{}", case);
            let mut flips = 0;
            for t in at + 1..end + period {
                if held(t) == held(t - 1) {
                    continue;
                }
                flips += 1;
                proptest::prop_assert!(
                    t.is_multiple_of(period) && t >= at + period && t + period <= end,
                    "{}: flips at {}", case, t
                );
            }
            let allowed = (at + period..=end.saturating_sub(period))
                .filter(|t| t.is_multiple_of(period))
                .count();
            proptest::prop_assert_eq!(flips, allowed, "{}", case);
        }
    }

    /// Every bobbing key that always plays at the same moments (each of a
    /// splice's at its branches' lengths; a script's keys set in ms from
    /// its body's start, played over its shortest body) flips at least
    /// once: holding a bob's first and last frame a frame clear of its
    /// key's start and end (Round 8) leaves a key of two frames off the
    /// grid none, a still pose drawn as a bob (the andagi's chewing, T2's
    /// review). The Dream's lines are three frames each so that each
    /// flips (twice; at 3 s, off the grid, its second and third never
    /// did: phase 5c's tail).
    #[test]
    fn every_bob_at_a_set_time_flips() {
        let mut parts: Vec<(String, &'static [Key], u64, usize)> = Vec::new();
        for splice in SpliceId::ALL {
            for (branch, &len) in splice.row().lens.iter().enumerate() {
                let keys = splice.script().keys(branch as u8);
                parts.push((format!("{splice:?} branch {branch}"), keys, len, keys.len()));
            }
        }
        for id in ScriptId::ALL {
            let Some(shortest) = id.shortest_body() else {
                continue;
            };
            for (branch, keys) in id.branches().iter().enumerate() {
                // Its keys set in ms, from the first.
                let set = keys
                    .iter()
                    .take_while(|k| matches!(k.span, Span::Ms(ms) if ms <= shortest))
                    .count();
                parts.push((format!("{id:?} branch {branch}"), keys, shortest, set));
            }
        }
        let mut checked = 0;
        for (at, keys, body, set) in parts {
            let ends: Vec<u64> = keys.iter().map(|k| k.span.end(Some(body))).collect();
            for index in 0..set {
                if !matches!(keys[index].pose, Posed::Bob(..)) {
                    continue;
                }
                let start = ends[..index].iter().copied().max().unwrap_or(0);
                let end = ends[index].min(body);
                if end <= start {
                    continue;
                }
                checked += 1;
                let flips = (start + 1..end)
                    .filter(|&t| pose_in(keys, body, t) != pose_in(keys, body, t - 1))
                    .count();
                assert!(
                    flips > 0,
                    "{at}: key {index} ({start} to {end}) never flips"
                );
            }
        }
        assert!(checked >= 2, "{checked} keys checked");
    }

    /// The player: half-open keys, cumulative ends clamped to the body,
    /// and the last key held past it.
    #[test]
    fn keys_play_half_open_and_hold_past_the_end() {
        let keys = ScriptId::Homework.keys(0);
        let index = |elapsed, body| key_at(keys, elapsed, Some(body)).map(|(i, ..)| i);
        assert_eq!(index(0, 1000), Some(0));
        assert_eq!(index(499, 1000), Some(0));
        assert_eq!(index(500, 1000), Some(1));
        assert_eq!(index(749, 1000), Some(1));
        assert_eq!(index(750, 1000), Some(2));
        assert_eq!(index(999, 1000), Some(2));
        assert_eq!(index(5000, 1000), Some(2), "held past the end");
        // A set time longer than the body is clamped to it.
        let snack = ScriptId::Snack.keys(0);
        assert_eq!(key_at(snack, 999, Some(1000)).map(|(i, ..)| i), Some(0));
        assert_eq!(key_at(snack, 1000, Some(1000)).map(|(i, ..)| i), Some(0));
    }

    /// The tests' own splices (not rows).
    const SPLICES: [SpliceId; 3] = [
        SpliceId::TestSnack,
        SpliceId::TestSleep,
        SpliceId::TestBedtime,
    ];

    /// Whether every key of `keys` ends at a set time, the last with the
    /// rest of its part: what a splice's script must do (a share of a
    /// splice would be a share of nothing in particular).
    fn at_set_times(keys: &[Key]) -> bool {
        let (last, rest) = keys
            .split_last()
            .map_or((None, keys), |(l, r)| (Some(l), r));
        rest.iter().all(|k| matches!(k.span, Span::Ms(_)))
            && last.is_some_and(|k| k.span == Span::Rest)
    }

    /// Lint, over every splice: its script's keys end at set times (the
    /// lint bites: a petting's bite comes at a share), each inside the
    /// shortest of its lengths; it has a length per branch it plays
    /// (some); it never leaves her pose to a host (it has none); it
    /// never wraps unpacking or crumpling; and its rolls are salted apart
    /// from every other's.
    #[test]
    fn every_splice_plays_at_set_times_and_wraps_what_it_may() {
        assert!(!at_set_times(ScriptId::Pet.keys(0)), "the lint bites");
        let mut salts = std::collections::HashSet::new();
        for id in SPLICES.into_iter().chain(SpliceId::ALL) {
            assert!(salts.insert(id.row().salt), "{id:?}: its salt is another's");
            let row = id.row();
            let branches = id.script().branches();
            assert!(
                !row.lens.is_empty() && row.lens.len() <= branches.len(),
                "{id:?}"
            );
            for (branch, &len) in row.lens.iter().enumerate() {
                let keys = id.script().keys(branch as u8);
                assert!(at_set_times(keys), "{id:?}/{branch}");
                for key in keys {
                    if let Span::Ms(ms) = key.span {
                        assert!(ms <= len, "{id:?}/{branch}: {ms} > {len}");
                    }
                    assert!(!matches!(key.pose, Posed::Host), "{id:?}/{branch}");
                }
            }
            assert!(!row.around.is_empty(), "{id:?}");
            for what in row.around {
                assert!(
                    !matches!(what, Use::Unpack | Use::Crumple),
                    "{id:?}: {what:?}"
                );
            }
            for exams in [false, true] {
                let none = Rares::none();
                let ctx = SpliceCtx {
                    what: Use::Homework,
                    makeshift: false,
                    trying: false,
                    quiet: true,
                    day: None,
                    tints: Tints {
                        exams,
                        ..Tints::default()
                    },
                    rares: &none,
                };
                let (n, d) = (row.chance)(&ctx);
                assert!(n <= d && d > 0, "{id:?}");
            }
        }
    }

    /// What `rows` wrap a use of `what` in, from `whims` (quiet or not,
    /// trying it or not), with `lines` keeping what she's played.
    fn rolled(
        rows: &[SpliceId],
        what: Use,
        quiet: bool,
        trying: bool,
        whims: u64,
        lines: &mut Lines,
        at: u64,
    ) -> (Option<SpliceId>, Option<SpliceId>) {
        let none = Rares::none();
        let ctx = SpliceCtx {
            what,
            makeshift: false,
            trying,
            quiet,
            day: None,
            tints: Tints::default(),
            rares: &none,
        };
        let (before, after) = splices(rows, &ctx, None, false, Whims(whims), lines, at);
        (before.map(|s| s.splice), after.map(|s| s.splice))
    }

    /// Rolled: a splice wraps only a use it may (and never a trial sit),
    /// a prelude only when she's quiet; each by its chance, and not
    /// again within ten minutes of playing (a splice that didn't roll
    /// didn't play, and doesn't cool: one that rolls a moment later
    /// plays); each its row's length.
    #[test]
    fn splices_roll_by_their_rows() {
        let rows = [SpliceId::TestSnack, SpliceId::TestSleep];
        // Whims the coda rolls on, round a homework (the coda's roll
        // doesn't depend on the use).
        let rolls = |w: u64| {
            rolled(
                &rows,
                Use::Homework,
                true,
                false,
                w,
                &mut Lines::default(),
                0,
            )
            .1 == Some(SpliceId::TestSleep)
        };
        let coda = (0..10_000).find(|&w| rolls(w)).expect("a coda rolls");
        let mut afters = 0;
        for w in 0..400 {
            for &what in &Use::ALL {
                let may = TEST_AROUND.contains(&what);
                let mut lines = Lines::default();
                let (before, after) = rolled(&rows, what, true, false, w, &mut lines, 0);
                assert_eq!(before, may.then_some(SpliceId::TestSnack), "{what:?}");
                assert!(after.is_none() || may, "{what:?}");
                afters += usize::from(after.is_some());
                // Not a trial sit, nor (a prelude) while she talks.
                let mut fresh = Lines::default();
                assert_eq!(
                    rolled(&rows, what, true, true, w, &mut fresh, 0),
                    (None, None)
                );
                let (before, _) = rolled(&rows, what, false, false, w, &mut fresh, 0);
                assert_eq!(before, None, "{what:?}: a prelude over her talking");
                if !may {
                    continue;
                }
                // Cooling: not again within ten minutes of playing; and a
                // coda that didn't roll didn't play, so one that rolls a
                // moment later does.
                let (before, again) = rolled(&rows, what, true, false, coda, &mut lines, 599_999);
                assert_eq!(before, None, "{what:?}");
                let cooling = after.is_some();
                assert_eq!(
                    again,
                    (!cooling).then_some(SpliceId::TestSleep),
                    "{what:?}, whims {w}"
                );
                let (before, _) = rolled(&rows, what, true, false, w, &mut lines, 600_000);
                assert_eq!(before, Some(SpliceId::TestSnack), "{what:?}");
            }
        }
        // The coda, one in two.
        let n = 400 * TEST_AROUND.len();
        assert!((n * 2 / 5..n * 3 / 5).contains(&afters), "{afters} of {n}");
        // Each its row's length.
        let none = Rares::none();
        let ctx = SpliceCtx {
            what: Use::Homework,
            makeshift: false,
            trying: false,
            quiet: true,
            day: None,
            tints: Tints::default(),
            rares: &none,
        };
        for id in SPLICES {
            let (before, after) = splices(
                &[],
                &ctx,
                Some((id, None)),
                false,
                Whims(1),
                &mut Lines::default(),
                0,
            );
            let s = before.or(after).unwrap();
            assert_eq!(s.splice, id);
            assert_eq!(Some(&s.len), id.row().lens.get(usize::from(s.branch)));
        }
    }

    /// The rows: the chopsticks before about one homework in three (only
    /// when she's quiet), split cleanly or badly evenly; the andagi
    /// after about one snack in four, said four, five or six times
    /// evenly; neither round anything else. (Whether a branch is drawn
    /// apart from the chance is
    /// [`the_first_splice_to_roll_wraps_it_on_a_branch_of_its_own`]'s to
    /// tell: at these rows' odds, a branch drawn from the chance's own
    /// roll would still come out even.)
    #[test]
    fn the_rows_wrap_their_uses_at_their_odds() {
        let n = 3000u64;
        let rolled = |what, quiet, w| {
            let none = Rares::none();
            let ctx = SpliceCtx {
                what,
                makeshift: false,
                trying: false,
                quiet,
                day: None,
                tints: Tints::default(),
                rares: &none,
            };
            splices(
                &SpliceId::ALL,
                &ctx,
                None,
                false,
                Whims(w),
                &mut Lines::default(),
                0,
            )
        };
        for (what, id, (num, den)) in [
            (Use::Homework, SpliceId::Chopsticks, (1, 3)),
            (Use::Snack, SpliceId::Andagi, (1, 4)),
        ] {
            let branches = id.row().lens.len();
            let mut counts = vec![0u64; branches];
            for w in 0..n {
                let (before, after) = rolled(what, true, w);
                assert!(before.is_none() || after.is_none(), "{what:?}");
                if let Some(s) = before.or(after) {
                    assert_eq!(s.splice, id);
                    assert_eq!(Some(&s.len), id.row().lens.get(usize::from(s.branch)));
                    counts[usize::from(s.branch)] += 1;
                }
            }
            let total: u64 = counts.iter().sum();
            let want = n * num / den;
            assert!(
                (want * 4 / 5..want * 6 / 5).contains(&total),
                "{id:?}: {total} of {n}"
            );
            for &count in &counts {
                let even = total / branches as u64;
                assert!(
                    (even * 3 / 4..even * 5 / 4).contains(&count),
                    "{id:?}: {counts:?}"
                );
            }
        }
        for w in 0..400 {
            // Talking, no chopsticks (they'd hide under what she says).
            assert_eq!(rolled(Use::Homework, false, w).0, None);
            for what in Use::ALL {
                if !matches!(what, Use::Homework | Use::Snack) {
                    assert_eq!(rolled(what, true, w), (None, None), "{what:?}");
                }
            }
        }
    }

    /// The chopsticks: held joined (blank), then split, cleanly (happy, a
    /// twinkle, then a giggle) or badly (let down, telling herself off
    /// for as long as that takes to say, then trailing off), at the desk
    /// throughout; the face the split frame bakes in (ASCII) from the
    /// split on. Asked anything meanwhile, she only looks.
    #[test]
    fn the_chopsticks_split_cleanly_or_badly() {
        use super::super::osaka::speech_ms;
        let len = SpliceId::Chopsticks.row().lens;
        assert_eq!(len, [CHOPSTICKS_MS, CHOPSTICKS_MS]);
        assert!(CHOPSTICKS_SHOWN_MS - CHOPSTICKS_SPLIT_MS >= speech_ms(HOLD_EM));
        let looks = |branch: u8| -> Vec<(Span, Pose, Face, Option<Say>)> {
            ScriptId::Chopsticks
                .keys(branch)
                .iter()
                .map(|k| match k.pose {
                    Posed::Still(pose) => (k.span, pose, k.face, k.say),
                    other => panic!("{other:?}"),
                })
                .collect()
        };
        let joined = (
            Span::Ms(CHOPSTICKS_JOINED_MS),
            Pose::Chopsticks(0),
            Face::Vacant,
            None,
        );
        let clean = |span, say| (span, Pose::Chopsticks(1), Face::Happy, say);
        let bad = |span, say| (span, Pose::Chopsticks(2), Face::Droop, say);
        let (split, shown) = (Span::Ms(CHOPSTICKS_SPLIT_MS), Span::Ms(CHOPSTICKS_SHOWN_MS));
        assert_eq!(
            looks(0),
            [
                joined,
                clean(split, None),
                clean(shown, bubble(Bubble::Sparkle)),
                clean(Span::Rest, bubble(Bubble::Hehe))
            ]
        );
        assert_eq!(
            looks(1),
            [
                joined,
                bad(split, None),
                bad(shown, bubble(Bubble::Say(HOLD_EM))),
                bad(Span::Rest, bubble(Bubble::Dots))
            ]
        );
        // Joined about a second, split in under one, what came of it
        // shown for as long as telling herself off takes (2.5 s or so),
        // then a beat more.
        let (joined, split, shown) = (
            CHOPSTICKS_JOINED_MS,
            CHOPSTICKS_SPLIT_MS - CHOPSTICKS_JOINED_MS,
            CHOPSTICKS_SHOWN_MS - CHOPSTICKS_SPLIT_MS,
        );
        assert!((800..=1200).contains(&joined), "{joined}");
        assert!((600..=1000).contains(&split), "{split}");
        assert!((2000..=3000).contains(&shown), "{shown}");
        assert_eq!(ScriptId::Chopsticks.on_chat(), Chat::Look);
    }

    /// Lint: a script that changes her face while her pose holds still
    /// shows the change in both drawing modes (an ASCII profile keeps a
    /// face of its own, so a ramp of faces there would go unseen).
    #[test]
    fn every_change_of_face_shows_in_both_modes() {
        use super::super::art::Rig;
        use super::super::sprite::{Facing, cells};
        for id in ScriptId::ALL {
            for (branch, keys) in id.branches().iter().enumerate() {
                for pair in keys.windows(2) {
                    let (Posed::Still(a), Posed::Still(b)) = (pair[0].pose, pair[1].pose) else {
                        continue;
                    };
                    let (f, g) = (pair[0].face, pair[1].face);
                    if a != b || f == g {
                        continue;
                    }
                    let at = format!("{id:?} branch {branch}: {a:?}, {f:?} to {g:?}");
                    for facing in [Facing::Left, Facing::Right] {
                        assert_ne!(cells(a, facing, f), cells(a, facing, g), "ASCII, {at}");
                    }
                    assert_ne!(Rig::for_pose(a, f), Rig::for_pose(a, g), "line art, {at}");
                }
            }
        }
    }

    /// Lint: only a splice's script answers a question (its own use's
    /// script, or a musing's, is played where no answer is looked for),
    /// and only her night's stirs (and every surface's branch of it, the
    /// Dream's too).
    #[test]
    fn only_a_splice_answers_and_only_the_night_stirs() {
        for id in ScriptId::ALL {
            match id.on_chat() {
                Chat::Answer(_) => assert_eq!(id.host(), Host::Splice, "{id:?}"),
                Chat::Stir => assert!(id.is_night(), "{id:?}"),
                Chat::Look | Chat::Stop => assert!(!id.is_night(), "{id:?}"),
            }
            assert_eq!(id.is_night(), id.host() == Host::Night, "{id:?}");
        }
    }

    /// The Dream: a branch for each surface her night is on, posed as
    /// her night's is there (the floor's held still), the lamp off
    /// throughout; its three lines in turn, each a line of her sleep-talk
    /// shown a while, then asleep as before until she wakes.
    #[test]
    fn the_dream_is_talked_in_her_sleep_on_every_surface() {
        let dream = ScriptId::Dream;
        assert_eq!(dream.branches().len(), Surface::ALL.len());
        for surface in Surface::ALL {
            for at_once in [false, true] {
                assert_eq!(Surface::of_branch(surface.branch(at_once)), surface);
            }
            let keys = dream.keys(surface.dream_branch());
            let night = ScriptId::Night.keys(surface.branch(true));
            let said: Vec<Option<Say>> = keys.iter().map(|k| k.say).collect();
            assert_eq!(
                said,
                [
                    bubble(Bubble::Say(EVERYNYAN)),
                    bubble(Bubble::Say(SANKYU)),
                    bubble(Bubble::Say(OH_MY_GAH)),
                    bubble(Bubble::Zzz),
                ],
                "{surface:?}"
            );
            let posed = |pose: Posed| match pose {
                Posed::Bob(pose, period) => (pose(0), Some(period)),
                Posed::Still(pose) => (pose, None),
                Posed::Host => panic!("{surface:?}: posed by its host"),
            };
            for key in keys {
                assert_eq!(key.prop, Some(Shows::Still(Prop::LampOff)), "{surface:?}");
                assert_eq!(key.face, Face::Blink, "{surface:?}");
                assert_eq!(
                    posed(key.pose),
                    posed(night[0].pose),
                    "{surface:?}: posed as her night"
                );
            }
            let ends: Vec<u64> = keys
                .iter()
                .map(|k| k.span.end(Some(DREAM_MS * 10)))
                .collect();
            assert_eq!(
                ends,
                [DREAM_LINE_MS, 2 * DREAM_LINE_MS, DREAM_MS, DREAM_MS * 10],
                "{surface:?}"
            );
        }
    }

    /// Her night: a branch for each surface and lamp, each posed as its
    /// surface is slept on (in bed, napping, flat on the floor), the lamp
    /// off by its last key; the floor's held still (its host wakes only
    /// as a key ends, so a bob would freeze), the bed's lamp moment timed
    /// as a day's sleep's (so one running into bedtime becomes the night
    /// in place, its keys where they were).
    #[test]
    fn the_night_poses_her_as_her_surface_with_the_lamp_off() {
        let night = ScriptId::Night;
        assert_eq!(night.branches().len(), 2 * Surface::ALL.len());
        for surface in Surface::ALL {
            for at_once in [false, true] {
                let keys = night.keys(surface.branch(at_once));
                assert_eq!(keys.len(), if at_once { 1 } else { 2 }, "{surface:?}");
                assert_eq!(keys.last().unwrap().prop, Some(Shows::Still(Prop::LampOff)));
                assert_eq!(keys[0].prop.is_some(), at_once, "{surface:?}");
                for key in keys {
                    let pose = match key.pose {
                        Posed::Bob(pose, _) => pose(0),
                        Posed::Still(pose) => pose,
                        Posed::Host => panic!("{surface:?}: posed by its host"),
                    };
                    let want = match surface {
                        Surface::Bed => Pose::Sleep(0),
                        Surface::Sofa => Pose::Nap(0),
                        Surface::Floor => Pose::LieBack(0),
                    };
                    assert_eq!(pose, want, "{surface:?}");
                    if surface == Surface::Floor {
                        assert!(matches!(key.pose, Posed::Still(_)), "{key:?}");
                    }
                }
            }
        }
        let (sleep, bed) = (
            ScriptId::Sleep.keys(0),
            night.keys(Surface::Bed.branch(false)),
        );
        assert_eq!(sleep[0].span, bed[0].span);
        assert_eq!(Surface::of(Use::Sleep), Some(Surface::Bed));
        assert_eq!(Surface::of(Use::Nap), Some(Surface::Sofa));
        assert_eq!(Surface::of(Use::Lounge), None);
    }

    /// The andagi: the fridge open a moment as she finds it, then held
    /// up: "Sata andagi." (about as long as it takes to say) and a quiet
    /// beat, four to six times (its branch), blank at first, then
    /// pleased, then happy; then a bite, and she eats it. Asked anything
    /// meanwhile, she answers "Sata andagi.".
    #[test]
    fn the_andagi_is_named_happier_each_time_then_eaten() {
        use super::super::osaka::speech_ms;
        assert!(ANDAGI_SAID_MS.abs_diff(speech_ms(SATA_ANDAGI)) <= 100);
        assert_eq!(ScriptId::Andagi.on_chat(), Chat::Answer(SATA_ANDAGI));
        let lens = SpliceId::Andagi.row().lens;
        assert_eq!(lens.len(), ANDAGI_COUNTS.len());
        assert_eq!(ANDAGI_COUNTS, [4, 5, 6], "a few times");
        for (branch, &len) in lens.iter().enumerate() {
            let count = ANDAGI_COUNTS[branch] as usize;
            let keys = ScriptId::Andagi.keys(branch as u8);
            assert_eq!(keys.len(), 2 * count + 4, "{count}");
            assert_eq!(len, andagi_ms(count as u64));
            let found = &keys[0];
            assert_eq!(found.span, Span::Ms(ANDAGI_FOUND_MS));
            assert_eq!(found.prop, Some(Shows::Still(Prop::FridgeOpen)));
            let rank = |face| match face {
                Face::Vacant => 0,
                Face::Pleased => 1,
                Face::Happy => 2,
                other => panic!("{other:?}"),
            };
            let mut faces = Vec::new();
            for i in 0..count {
                let (said, beat) = (&keys[1 + 2 * i], &keys[2 + 2 * i]);
                let from = ANDAGI_FOUND_MS + i as u64 * (ANDAGI_SAID_MS + ANDAGI_BEAT_MS);
                assert_eq!(said.span, Span::Ms(from + ANDAGI_SAID_MS));
                assert_eq!(said.say, bubble(Bubble::Say(SATA_ANDAGI)));
                assert_eq!(beat.span, Span::Ms(from + ANDAGI_SAID_MS + ANDAGI_BEAT_MS));
                assert_eq!(beat.say, None, "a quiet beat");
                assert_eq!(said.face, beat.face);
                for key in [said, beat] {
                    assert!(matches!(key.pose, Posed::Still(Pose::EatAndagi(0))));
                    assert_eq!(key.prop, None);
                }
                faces.push(rank(said.face));
            }
            assert!((500..=800).contains(&ANDAGI_BEAT_MS));
            assert!(faces.windows(2).all(|w| w[0] <= w[1]), "{faces:?}");
            assert_eq!((faces[0], faces[count - 1]), (0, 2), "{faces:?}");
            assert!(faces.contains(&1), "{faces:?}");
            let (bite, eat) = (&keys[2 * count + 1], &keys[2 * count + 3]);
            assert!(matches!(bite.pose, Posed::Still(Pose::EatAndagi(1))));
            // Bitten, it stays bitten.
            let poses = |key: &Key| match key.pose {
                Posed::Still(pose) => vec![pose],
                Posed::Bob(pose, _) => vec![pose(0), pose(1)],
                Posed::Host => vec![],
            };
            let bitten = keys
                .iter()
                .position(|k| poses(k).contains(&Pose::EatAndagi(1)))
                .unwrap();
            for key in &keys[bitten..] {
                assert!(
                    !poses(key).contains(&Pose::EatAndagi(0)),
                    "{count}: whole again after the bite"
                );
            }
            assert_eq!(eat.span, Span::Rest);
            assert_eq!(bite.span, Span::Ms(len - ANDAGI_EAT_MS));
            // Bitten, she eats it over its last two frames: chewing,
            // then biting again, a frame each.
            let mut eaten: Vec<(Pose, u64)> = Vec::new();
            for t in len - ANDAGI_EAT_MS..len {
                let (pose, ..) = pose_in(keys, len, t).unwrap();
                match eaten.last_mut() {
                    Some((was, ms)) if *was == pose => *ms += 1,
                    _ => eaten.push((pose, 1)),
                }
            }
            assert_eq!(
                eaten,
                [
                    (Pose::EatAndagi(2), USE_FRAME_MS),
                    (Pose::EatAndagi(1), USE_FRAME_MS)
                ],
                "{count}: eating it"
            );
        }
    }

    /// In exam season (A23) her chopsticks come before every homework
    /// she starts quiet, whatever her whims, still never again within
    /// their ten minutes' cooling; out of it, about one in three (the
    /// rows' odds, above), and the season changes nothing else.
    #[test]
    fn exam_season_brings_her_chopsticks_to_every_homework() {
        let none = Rares::none();
        let ctx = |what, exams| SpliceCtx {
            what,
            makeshift: false,
            trying: false,
            quiet: true,
            day: None,
            tints: Tints {
                exams,
                ..Tints::default()
            },
            rares: &none,
        };
        let roll = |what, exams, w, lines: &mut Lines, at| {
            let (before, after) = splices(
                &SpliceId::ALL,
                &ctx(what, exams),
                None,
                false,
                Whims(w),
                lines,
                at,
            );
            (before.map(|s| s.splice), after.map(|s| s.splice))
        };
        let mut outside = 0;
        for w in 0..300 {
            let mut lines = Lines::default();
            let (before, _) = roll(Use::Homework, true, w, &mut lines, 0);
            assert_eq!(before, Some(SpliceId::Chopsticks), "whims {w}");
            let (again, _) = roll(Use::Homework, true, w + 1, &mut lines, 599_999);
            assert_eq!(again, None, "whims {w}: cooling");
            let (cooled, _) = roll(Use::Homework, true, w + 2, &mut lines, 600_000);
            assert_eq!(cooled, Some(SpliceId::Chopsticks), "whims {w}: cooled");
            outside += usize::from(
                roll(Use::Homework, false, w, &mut Lines::default(), 0).0
                    == Some(SpliceId::Chopsticks),
            );
            // Nothing else is tinted: a snack's andagi rolls as ever.
            for what in [Use::Snack, Use::Watch] {
                assert_eq!(
                    roll(what, true, w, &mut Lines::default(), 0),
                    roll(what, false, w, &mut Lines::default(), 0),
                    "{what:?}, whims {w}"
                );
            }
        }
        assert!((60..140).contains(&outside), "{outside} of 300");
    }

    /// Of several preludes (or codas) that would wrap a use, only the
    /// first that rolls does; each's branch, and so its length, is drawn
    /// evenly from the whims.
    #[test]
    fn the_first_splice_to_roll_wraps_it_on_a_branch_of_its_own() {
        let rows = [SpliceId::TestBedtime, SpliceId::TestSnack];
        let none = Rares::none();
        let ctx = SpliceCtx {
            what: Use::Homework,
            makeshift: false,
            trying: false,
            quiet: true,
            day: None,
            tints: Tints::default(),
            rares: &none,
        };
        let lens = SpliceId::TestBedtime.row().lens;
        let mut seen = [0u64; 2];
        let n = 400;
        for w in 0..n {
            let (before, after) =
                splices(&rows, &ctx, None, false, Whims(w), &mut Lines::default(), 0);
            assert_eq!(after, None);
            let before = before.unwrap();
            // The bedtime one, one in two; else the snack (which always
            // rolls), never both.
            match before.splice {
                SpliceId::TestBedtime => {
                    let branch = usize::from(before.branch);
                    assert_eq!(Some(&before.len), lens.get(branch), "whims {w}");
                    seen[branch] += 1;
                }
                other => assert_eq!(other, SpliceId::TestSnack, "whims {w}"),
            }
        }
        let bedtimes = seen[0] + seen[1];
        assert!((n * 2 / 5..n * 3 / 5).contains(&bedtimes), "{seen:?}");
        for count in seen {
            assert!(
                (bedtimes * 2 / 5..bedtimes * 3 / 5).contains(&count),
                "{seen:?}"
            );
        }
    }

    /// Which key it is, as far as the tests can tell them apart.
    fn which(key: &Key) -> (Span, Face, Option<Say>, Option<Shows>) {
        (key.span, key.face, key.say, key.prop)
    }

    /// A use played through its prelude, body and coda: each part's keys
    /// half-open at its boundaries, timed from the part's own start, and
    /// the coda's last key held past the end.
    #[test]
    fn a_use_plays_its_prelude_body_and_coda_in_turn() {
        const SINCE: u64 = 10_000;
        const UNTIL: u64 = SINCE + 10_000;
        let before = Spliced {
            splice: SpliceId::TestSnack,
            len: 2000,
            branch: 0,
        };
        let after = Spliced {
            splice: SpliceId::TestSleep,
            len: 3000,
            branch: 0,
        };
        let snack = ScriptId::Snack.keys(0);
        let sleep = ScriptId::Sleep.keys(0);
        let homework = ScriptId::Homework.keys(0);
        for (before, after) in [
            (None, None),
            (Some(before), None),
            (None, Some(after)),
            (Some(before), Some(after)),
        ] {
            let play = Play {
                before,
                after,
                ..Play::plain(ScriptId::Homework)
            };
            let start = play.body_start(SINCE);
            let end = play.body_end(SINCE, UNTIL);
            assert_eq!(start, SINCE + before.map_or(0, |s| s.len));
            assert_eq!(end, UNTIL - after.map_or(0, |s| s.len));
            let body = end - start;
            let at = |now| {
                let (key, time) = play.key(SINCE, UNTIL, now).unwrap();
                (which(key), time.elapsed)
            };
            let case = format!("before {before:?} after {after:?}");
            if before.is_some() {
                assert_eq!(at(SINCE), (which(&snack[0]), 0), "{case}");
                assert_eq!(at(SINCE + 1499), (which(&snack[0]), 1499), "{case}");
                assert_eq!(at(SINCE + 1500), (which(&snack[1]), 1500), "{case}");
                assert_eq!(at(start - 1), (which(&snack[1]), 1999), "{case}");
            }
            assert_eq!(at(start), (which(&homework[0]), 0), "{case}");
            assert_eq!(
                at(start + body / 2),
                (which(&homework[1]), body / 2),
                "{case}"
            );
            assert_eq!(at(end - 1), (which(&homework[2]), body - 1), "{case}");
            if after.is_some() {
                assert_eq!(at(end), (which(&sleep[0]), 0), "{case}");
                assert_eq!(at(end + 1999), (which(&sleep[0]), 1999), "{case}");
                assert_eq!(at(end + 2000), (which(&sleep[1]), 2000), "{case}");
                assert_eq!(at(UNTIL - 1), (which(&sleep[1]), 2999), "{case}");
                assert_eq!(at(UNTIL + 500), (which(&sleep[1]), 3500), "{case}");
            } else {
                assert_eq!(at(UNTIL + 500), (which(&homework[2]), body + 500), "{case}");
            }
        }
        // Splices longer than the use: the body is empty, the prelude
        // plays to its end and the coda from where the body would be.
        let crowded = Play {
            before: Some(Spliced {
                len: 6000,
                ..before
            }),
            after: Some(Spliced { len: 6000, ..after }),
            ..Play::plain(ScriptId::Homework)
        };
        let start = crowded.body_start(SINCE);
        assert_eq!(crowded.body_end(SINCE, UNTIL), start);
        let (key, time) = crowded.key(SINCE, UNTIL, start - 1).unwrap();
        assert_eq!((which(key), time.elapsed), (which(&snack[1]), 5999));
        let (key, time) = crowded.key(SINCE, UNTIL, start).unwrap();
        assert_eq!((which(key), time.elapsed), (which(&sleep[0]), 0));
    }

    /// The stage's note names each part in turn, the one playing with
    /// which of its keys.
    #[test]
    fn the_stage_says_what_part_and_key_she_plays() {
        const SINCE: u64 = 10_000;
        const UNTIL: u64 = SINCE + 10_000;
        let plain = Play::plain(ScriptId::Homework);
        assert_eq!(plain.note(SINCE, UNTIL, SINCE), "homework 1/3");
        assert_eq!(plain.note(SINCE, UNTIL, UNTIL + 500), "homework 3/3");
        let wrapped = Play {
            before: Some(Spliced {
                splice: SpliceId::TestSnack,
                len: 2000,
                branch: 0,
            }),
            after: Some(Spliced {
                splice: SpliceId::TestSleep,
                len: 3000,
                branch: 0,
            }),
            ..plain
        };
        let note = |now| wrapped.note(SINCE, UNTIL, now);
        assert_eq!(note(SINCE), "test snack 1/2 › homework › test sleep");
        assert_eq!(note(SINCE + 1500), "test snack 2/2 › homework › test sleep");
        assert_eq!(note(SINCE + 2000), "test snack › homework 1/3 › test sleep");
        assert_eq!(note(UNTIL - 3001), "test snack › homework 3/3 › test sleep");
        assert_eq!(note(UNTIL - 3000), "test snack › homework › test sleep 1/2");
        assert_eq!(note(UNTIL), "test snack › homework › test sleep 2/2");
    }

    /// The next key end is the first one strictly after `now` (at a key's
    /// end, the one after it), through the prelude, the body and the
    /// coda in turn, and none once every key is over.
    #[test]
    fn the_next_key_end_is_strictly_ahead() {
        const SINCE: u64 = 10_000;
        const UNTIL: u64 = SINCE + 10_000;
        let next = |play: Play, now| play.next_end(SINCE, UNTIL, now);
        // Homework, 10 s: ends at ½, ¾ and the end.
        let plain = Play::plain(ScriptId::Homework);
        assert_eq!(next(plain, 0), Some(SINCE + 5000));
        assert_eq!(next(plain, SINCE), Some(SINCE + 5000));
        assert_eq!(next(plain, SINCE + 4999), Some(SINCE + 5000));
        assert_eq!(next(plain, SINCE + 5000), Some(SINCE + 7500));
        assert_eq!(next(plain, SINCE + 7500), Some(UNTIL));
        assert_eq!(next(plain, UNTIL - 1), Some(UNTIL));
        assert_eq!(next(plain, UNTIL), None);
        // A snack's look in the fridge before, a sleep after: its keys
        // end where they would alone, from where each part starts.
        let wrapped = Play {
            before: Some(Spliced {
                splice: SpliceId::TestSnack,
                len: 2000,
                branch: 0,
            }),
            after: Some(Spliced {
                splice: SpliceId::TestSleep,
                len: 3000,
                branch: 0,
            }),
            ..plain
        };
        let mut ends = Vec::new();
        let mut now = SINCE;
        while let Some(end) = next(wrapped, now) {
            assert!(end > now, "{end} after {now}");
            ends.push(end - SINCE);
            now = end;
        }
        // Snack: 1500, the prelude's end; Homework over a 5 s body: ½,
        // ¾, its end; Sleep: the lamp off at 2 s, the coda's end.
        assert_eq!(ends, [1500, 2000, 4500, 5750, 7000, 9000, 10_000]);
    }

    /// A plain use of anything but a watch plays its own script; a watch
    /// plays the shopping channel if it sold her something.
    #[test]
    fn a_use_plays_the_channel_only_when_it_sold_her_something() {
        for u in Use::ALL {
            assert_eq!(Play::of(u, None), Play::plain(u.script()), "{u:?}");
        }
        let sold = Play::of(Use::Watch, Some(Furniture::Lamp));
        assert_eq!(sold.own, ScriptId::Shopping);
        assert_eq!(sold.bought, Some(Furniture::Lamp));
    }

    /// A rare splice never wraps a use on a day it isn't open, however
    /// sure her whims (not even the tests' "every vignette" roll), and
    /// leaves everything else to roll exactly as if it weren't a row; on a
    /// day it's open it rolls its own odds (in its own window: the scary
    /// story only from 22:00 to bedtime, never without her routine); and
    /// cued, it plays whether open or not.
    #[test]
    fn a_rare_splice_plays_only_on_a_day_its_open() {
        use super::super::rarity::{DAY_WINDOW, Pity, Rares};
        use super::super::routine::{self, Slot};
        let none = Rares::none();
        // A day both are open: the first whose Rare roll passes its base
        // chance (pity never opens the seen).
        let seen = [ScriptId::NoMelon, ScriptId::Scary];
        let open = (0..1000)
            .map(|seed| Rares::draw(seed, &seen, Pity::default(), DAY_WINDOW))
            .find(|r| r.allows(ScriptId::NoMelon))
            .expect("a day in seven or so");
        assert!(open.allows(ScriptId::NoMelon) && open.allows(ScriptId::Scary));
        let late = routine::day_time(routine::game_of(0, 22 * 60 + 10), false);
        let early = routine::day_time(routine::game_of(0, 21 * 60 + 50), false);
        // Past her bedtime (22:30 on a school night), and after midnight.
        let past_bed = [
            routine::day_time(routine::game_of(0, 22 * 60 + 45), false),
            routine::day_time(routine::game_of(1, 60 + 15), false),
        ];
        assert_eq!(late.slot, Slot::Homework);
        assert!(past_bed.iter().all(|day| day.slot == Slot::Asleep));
        let rare = [SpliceId::NoMelon, SpliceId::Scary];
        let roll = |rares: &Rares, what, day, sure, w| {
            let ctx = SpliceCtx {
                what,
                makeshift: false,
                trying: false,
                quiet: true,
                day,
                tints: Tints::default(),
                rares,
            };
            let (before, after) = splices(
                &SpliceId::ALL,
                &ctx,
                None,
                sure,
                Whims(w),
                &mut Lines::default(),
                0,
            );
            (before.map(|s| s.splice), after.map(|s| s.splice))
        };
        let mut counts = [0u64; 2];
        let n = 3000;
        for w in 0..n {
            for what in Use::ALL {
                for day in [None, Some(early), Some(late)] {
                    for sure in [false, true] {
                        let (before, after) = roll(&none, what, day, sure, w);
                        assert!(
                            !rare.iter().any(|r| [before, after].contains(&Some(*r))),
                            "closed: {what:?} {day:?} sure={sure} whims {w}"
                        );
                        // Closed, they're as if not rows at all.
                        let commons = [SpliceId::Chopsticks, SpliceId::Andagi];
                        let ctx = SpliceCtx {
                            what,
                            makeshift: false,
                            trying: false,
                            quiet: true,
                            day,
                            tints: Tints::default(),
                            rares: &none,
                        };
                        let (b, a) = splices(
                            &commons,
                            &ctx,
                            None,
                            sure,
                            Whims(w),
                            &mut Lines::default(),
                            0,
                        );
                        assert_eq!(
                            (b.map(|s| s.splice), a.map(|s| s.splice)),
                            (before, after),
                            "{what:?} whims {w}"
                        );
                    }
                }
            }
            // Open: the melon bread after a snack, one in three of those
            // the andagi (one in four) leaves it...
            let (_, after) = roll(&open, Use::Snack, None, false, w);
            counts[0] += u64::from(after == Some(SpliceId::NoMelon));
            // ...the story after lounging or reading late, one in two,
            // and never earlier, nor past her bedtime, nor unfed.
            for what in [Use::Lounge, Use::Read] {
                let (_, after) = roll(&open, what, Some(late), false, w);
                counts[1] += u64::from(after == Some(SpliceId::Scary));
                for day in [None, Some(early), Some(past_bed[0]), Some(past_bed[1])] {
                    let (_, after) = roll(&open, what, day, true, w);
                    assert_ne!(after, Some(SpliceId::Scary), "{what:?} {day:?}");
                }
            }
        }
        assert!(
            (n * 22 / 100..n * 28 / 100).contains(&counts[0]),
            "{counts:?}"
        );
        assert!(
            (2 * n * 45 / 100..2 * n * 55 / 100).contains(&counts[1]),
            "{counts:?}"
        );
        // Cued, each plays closed or not, at any hour.
        for id in rare {
            for what in id.row().around {
                let ctx = SpliceCtx {
                    what: *what,
                    makeshift: false,
                    trying: false,
                    quiet: true,
                    day: None,
                    tints: Tints::default(),
                    rares: &none,
                };
                let (_, after) = splices(
                    &[],
                    &ctx,
                    Some((id, None)),
                    false,
                    Whims(1),
                    &mut Lines::default(),
                    0,
                );
                assert_eq!(after.map(|s| s.splice), Some(id), "{id:?} cued");
            }
        }
    }

    /// A glance up at the clock says the hour, roughly, whatever the
    /// hour: its branch is its line's, "Noon-ish." at 12, "Five-ish."
    /// at 17, "Midnight-ish." at 0, "Eight-ish." at 8 and at 20; the
    /// afternoon's (12 to 17) on the branches they always were (2 to 7).
    #[test]
    fn a_glance_says_the_hour_roughly() {
        let said = |hour: u16| {
            let branch = usize::from(ClockGlance::Hour(hour).branch());
            let keys = CLOCK_GLANCE.get(branch).expect("a branch");
            keys.iter().find_map(|k| match k.say {
                Some(Say::Bubble(Bubble::Say(line))) => Some(line),
                _ => None,
            })
        };
        for (hour, line) in [
            (0, "Midnight-ish."),
            (1, "One-ish."),
            (8, "Eight-ish."),
            (11, "Eleven-ish."),
            (12, "Noon-ish."),
            (15, "Three-ish."),
            (17, "Five-ish."),
            (20, "Eight-ish."),
            (23, "Eleven-ish."),
        ] {
            assert_eq!(said(hour), Some(line), "{hour}");
        }
        for hour in 12..18 {
            assert_eq!(ClockGlance::Hour(hour).branch(), 2 + (hour - 12) as u8);
        }
        for hour in 0..24 {
            assert!(said(hour).is_some(), "{hour}");
        }
    }
}
