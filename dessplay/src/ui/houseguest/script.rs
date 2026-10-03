//! Scripts: what she does over an act, as data. A script is a run of
//! keys, each a look (pose, face, what she says) and what it shows on
//! her furniture, lasting to a cumulative end; one player plays any of
//! them. Every use of a piece of her furniture plays one (its own, or
//! the shopping channel's on a watch), and the use's [`Play`] holds all
//! that was chosen for it when it began, so how she looks at any instant
//! is a pure function of the act.

use super::art::Channel;
use super::osaka::{Bubble, SCRUNCH, THERE, USE_FRAME_MS};
use super::room::{Furniture, Use};
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
    /// keeps its beat. The period is a whole number of frames
    /// ([`USE_FRAME_MS`]), so her wakeups on the frame grid catch every
    /// bob.
    Bob(fn(u8) -> Pose, u64),
}

/// What she says through a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Say {
    /// A bubble of her own.
    Bubble(Bubble),
    /// The shopping channel's pitch for what it's selling her.
    Pitch,
}

/// What a key shows on her furniture: on every shown piece of its kind
/// ([`Prop::item`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Prop {
    /// The TV on, showing a channel (each key names it on frame 0; see
    /// [`Prop::framed`]).
    Tv(Channel),
    /// The lamp switched off.
    LampOff,
    /// The fridge door open.
    FridgeOpen,
    /// The cat, if he's in his bed, biting.
    CatBiting,
}

/// How long each frame of what's on TV lasts.
pub(super) const CHANNEL_FRAME_MS: u64 = 400;

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

    /// On animation frame `frame` (what's on TV moves; the rest hold).
    pub fn framed(self, frame: u8) -> Self {
        match self {
            Self::Tv(Channel::Snow(_)) => Self::Tv(Channel::Snow(frame)),
            Self::Tv(Channel::Shopping(_)) => Self::Tv(Channel::Shopping(frame)),
            Self::LampOff | Self::FridgeOpen | Self::CatBiting => self,
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
    pub prop: Option<Prop>,
}

impl Key {
    /// How she looks `elapsed` ms into the body this key plays in: in
    /// `host`'s pose where the key leaves it to the host, pitching what
    /// she `bought`.
    pub fn look(&self, elapsed: u64, host: Pose, bought: Option<Furniture>) -> Look {
        let pose = match self.pose {
            Posed::Host => host,
            Posed::Still(pose) => pose,
            Posed::Bob(pose, period) => {
                pose(elapsed.checked_div(period).map_or(0, |n| (n % 2) as u8))
            }
        };
        let bubble = self.say.and_then(|say| match say {
            Say::Bubble(bubble) => Some(bubble),
            Say::Pitch => bought.map(|item| Bubble::Say(item.spec().pitch)),
        });
        (pose, self.face, bubble)
    }
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
/// (`None`: no set length), its index, and when it ends. Past the end
/// of the body, its last moment's key holds.
pub(super) fn key_at(keys: &[Key], elapsed: u64, body: Option<u64>) -> Option<(usize, &Key, u64)> {
    let elapsed = body.map_or(elapsed, |body| elapsed.min(body.saturating_sub(1)));
    at(keys, elapsed, |key| key.span.end(body)).or_else(|| {
        let last = keys.len().checked_sub(1)?;
        Some((last, keys.get(last)?, body.unwrap_or(u64::MAX)))
    })
}

/// A script: each of its branches a run of keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ScriptId {
    Lounge,
    Nap,
    Sleep,
    /// Writing, then nodding off onto the paper.
    Homework,
    /// Watching static.
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
}

impl ScriptId {
    #[cfg(test)]
    pub const ALL: [ScriptId; 11] = [
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
    ];

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

/// A script spliced before or after a use (a prelude or a coda). No
/// rows yet: each comes with its art.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum SpliceId {
    /// A test's prelude or coda (the snack's look in the fridge, then
    /// eating): not a row.
    #[cfg(test)]
    TestSnack,
    /// A test's prelude or coda (petting the cat, until he bites): not a
    /// row.
    #[cfg(test)]
    TestPet,
}

impl SpliceId {
    /// Every row (the tests' splices aren't rows).
    #[cfg(test)]
    pub const ALL: [SpliceId; 0] = [];

    /// The script it plays.
    pub fn script(self) -> ScriptId {
        match self {
            #[cfg(test)]
            Self::TestSnack => ScriptId::Snack,
            #[cfg(test)]
            Self::TestPet => ScriptId::Pet,
        }
    }
}

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
/// for it, and what the shopping channel sold her (on a watch).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Play {
    pub own: ScriptId,
    pub branch: u8,
    pub before: Option<Spliced>,
    pub after: Option<Spliced>,
    pub drawn: [u8; 2],
    pub bought: Option<Furniture>,
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

    /// The key playing at `now` in a use from `since` to `until`, and
    /// how far `now` is into the part it plays in (the prelude, the
    /// body or the coda).
    pub fn key(&self, since: u64, until: u64, now: u64) -> Option<(&'static Key, u64)> {
        let (keys, from, body) = self.part(since, until, now);
        let elapsed = now.saturating_sub(from);
        key_at(keys, elapsed, body).map(|(_, key, _)| (key, elapsed))
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

/// A key with `prop` on her furniture.
const fn shows(span: Span, pose: Posed, face: Face, say: Option<Say>, prop: Prop) -> Key {
    Key {
        span,
        pose,
        face,
        say,
        prop: Some(prop),
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

/// The lamp off while she sleeps.
const SLEEP: &[&[Key]] = &[&[shows(
    Span::Rest,
    Posed::Bob(Pose::Sleep, USE_FRAME_MS),
    Face::Blink,
    bubble(Bubble::Zzz),
    Prop::LampOff,
)]];

/// Writing for the first half, then nodding off onto the paper.
const HOMEWORK: &[&[Key]] = &[&[
    key(
        Span::Upto(1, 2),
        Posed::Bob(Pose::Homework, USE_FRAME_MS),
        Face::Vacant,
        None,
    ),
    key(
        Span::Upto(3, 4),
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
]];

const WATCH: &[&[Key]] = &[&[shows(
    Span::Rest,
    Posed::Host,
    Face::Curious,
    None,
    Prop::Tv(Channel::Snow(0)),
)]];

/// Hooked, then sold, then watching on.
const SHOPPING: &[&[Key]] = &[&[
    shows(
        Span::Upto(2, 5),
        Posed::Host,
        Face::Curious,
        bubble(Bubble::Ooh),
        Prop::Tv(Channel::Shopping(0)),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

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
        }
    }

    /// Lint: every script is listed in [`ScriptId::ALL`] (see
    /// [`listed_at`]), every splice that is a row in [`SpliceId::ALL`],
    /// and every use in [`Use::ALL`], each once; and every script is
    /// some use's, or the shopping channel's: none goes unplayed.
    #[test]
    fn every_script_is_listed() {
        for (i, id) in ScriptId::ALL.into_iter().enumerate() {
            assert_eq!(listed_at(id), i, "{id:?}");
        }
        // Wildcard-free: a new splice doesn't compile until it's said
        // here whether it's a row (and so listed).
        let row = |id: SpliceId| match id {
            SpliceId::TestSnack | SpliceId::TestPet => false,
        };
        for id in [SpliceId::TestSnack, SpliceId::TestPet] {
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
        };
        for (i, u) in Use::ALL.into_iter().enumerate() {
            assert_eq!(use_at(u), i, "{u:?}");
            assert!(ScriptId::ALL.contains(&u.script()), "{u:?}");
        }
        // And every script is played by something. Wildcard-free: a new
        // script doesn't compile until it says what plays it.
        let player = |id: ScriptId| match id {
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
        };
        for id in ScriptId::ALL {
            assert_eq!(player(id).own, id, "{id:?} isn't what plays it");
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

    /// A key that ends a set time in ends inside the shortest body there
    /// is (a trial sit), or it would be cut short there.
    #[test]
    fn every_set_time_fits_the_shortest_use() {
        for id in ScriptId::ALL {
            for keys in id.branches() {
                for key in *keys {
                    if let Span::Ms(ms) = key.span {
                        let shortest = super::super::osaka::shortest_use_ms();
                        assert!(ms <= shortest, "{id:?}: {ms} > {shortest}");
                    }
                }
            }
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
            let watch = matches!(id, ScriptId::Watch | ScriptId::Shopping);
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
        assert!(SpliceId::ALL.is_empty(), "no splice rows yet");
    }

    /// Which key it is, as far as the tests can tell them apart.
    fn which(key: &Key) -> (Span, Face, Option<Say>, Option<Prop>) {
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
            splice: SpliceId::TestPet,
            len: 3000,
            branch: 0,
        };
        let snack = ScriptId::Snack.keys(0);
        let pet = ScriptId::Pet.keys(0);
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
                let (key, elapsed) = play.key(SINCE, UNTIL, now).unwrap();
                (which(key), elapsed)
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
                assert_eq!(at(end), (which(&pet[0]), 0), "{case}");
                assert_eq!(at(end + 2099), (which(&pet[0]), 2099), "{case}");
                assert_eq!(at(end + 2100), (which(&pet[1]), 2100), "{case}");
                assert_eq!(at(UNTIL - 1), (which(&pet[1]), 2999), "{case}");
                assert_eq!(at(UNTIL + 500), (which(&pet[1]), 3500), "{case}");
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
        let (key, elapsed) = crowded.key(SINCE, UNTIL, start - 1).unwrap();
        assert_eq!((which(key), elapsed), (which(&snack[1]), 5999));
        let (key, elapsed) = crowded.key(SINCE, UNTIL, start).unwrap();
        assert_eq!((which(key), elapsed), (which(&pet[0]), 0));
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
        // A snack's look in the fridge before, a petting after: its keys
        // end where they would alone, from where each part starts.
        let wrapped = Play {
            before: Some(Spliced {
                splice: SpliceId::TestSnack,
                len: 2000,
                branch: 0,
            }),
            after: Some(Spliced {
                splice: SpliceId::TestPet,
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
        // ¾, its end; Pet: 7/10 of 3 s, the coda's end.
        assert_eq!(ends, [1500, 2000, 4500, 5750, 7000, 9100, 10_000]);
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
}
