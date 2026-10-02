//! Osaka herself: where she is, what she's doing, and when that next
//! changes. Every activity is a finite, wall-clock-timed step, so any
//! instant is a safe place to cut the visit short.

use super::Rng;
use super::art::DoorFrame;
use super::brain::{self, Factor, Mood, Need, Needs, Spot, Want};
use super::layer::Placed;
use super::mind::{self, Beat, Bind, Ctx, Heading, Here, Lines, Loss, Whims};
use super::room::{Furniture, MadeId, PieceRef, Seat, Use};
use super::scenes::{Build, Job, JobRef, LayerOp, Pull, Side, Swap};
use super::sprite::{self, Face, Facing, HEIGHT, Pose, SpriteCell};
use super::terrain::{Link, Platform, Route, Terrain};
use tuirealm::ratatui::layout::Rect;

/// What the current frame offers her beyond walking around.
#[derive(Clone, Debug, Default)]
pub(super) struct Chances {
    /// Lines she could pull.
    pub pulls: Vec<super::scenes::Pull>,
    /// Letters she could swap.
    pub swaps: Vec<super::scenes::Swap>,
    /// Glyphs a sneeze where she stands would knock loose.
    pub loose: Vec<(u16, u16)>,
    /// Her furniture, and where she'd go to use it.
    pub seats: Vec<Seat>,
    /// Makeshift furniture she could make of text, for each use.
    pub builds: Vec<super::scenes::Build>,
    /// The makeshift pieces she has made this visit.
    pub mine: Vec<Mine>,
    /// What the shopping channel would sell her, were she to watch now.
    pub advert: Option<Furniture>,
    /// She has a home (to leave for work, and come back to).
    pub furnished: bool,
    /// The chat pane, when she's resident: people read there, so what
    /// would take her into it is [`CHAT_FACTOR`] as likely.
    pub chat: Option<Rect>,
}

/// A makeshift piece she made this visit, and what she made it for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Mine {
    pub id: MadeId,
    pub purpose: Use,
    /// Crumpled into shape.
    pub done: bool,
    /// She has started using it.
    pub used: bool,
    /// Where it stands (its middle, on its floor).
    pub at: (i32, i32),
}

/// Beats she keeps owing at most.
const OWED: usize = 3;
/// A glance at something lost.
const GLANCE_MS: u64 = 900;

/// Times she sets off to finish or use a piece she made before she lets
/// it be: each interruption on the way, or each time she can't get to it,
/// is one.
const TRIES: u8 = 3;

/// How much less likely a resident is to do what takes her into the
/// chat pane.
pub(super) const CHAT_FACTOR: f64 = 0.1;

/// Whether `rect` holds the cell `(x, y)`.
fn holds(rect: Rect, (x, y): (i32, i32)) -> bool {
    match (u16::try_from(x), u16::try_from(y)) {
        (Ok(x), Ok(y)) => rect.contains((x, y).into()),
        _ => false,
    }
}

/// Her box standing at `(x, y)` overlaps `rect`. Only her body counts:
/// the floor under her feet may be another pane's border (and nothing
/// of hers is painted on a protected cell anyway).
pub(super) fn box_meets(rect: Rect, (x, y): (i32, i32)) -> bool {
    let half = sprite::WIDTH / 2;
    let (left, right) = (i32::from(rect.x), i32::from(rect.right()));
    let (top, bottom) = (i32::from(rect.y), i32::from(rect.bottom()));
    x - half < right && x + half >= left && y - HEIGHT < bottom && y > top
}

/// Where taking `link` puts her down.
pub(super) fn landing(link: &Link, terrain: &Terrain) -> Option<(i32, i32)> {
    let p = terrain.platforms.get(link.to)?;
    let x = match link.route {
        Route::Climb => link.x,
        Route::Drop { over } => over,
        Route::Clamber { column } => column,
        Route::Around { enter, .. } => {
            if enter < 0 {
                p.x0
            } else {
                p.x1
            }
        }
    };
    Some((p.clamp(x), p.y))
}

/// A spot on `p`'s floor, midway.
pub(super) fn middle(p: &Platform) -> (i32, i32) {
    ((p.x0 + p.x1) / 2, p.y)
}

/// Pick one of `n` things, those `in_chat` weighing [`CHAT_FACTOR`],
/// with `below(k)` uniform in `0..k`. With none in the chat it's a plain
/// uniform pick.
pub(super) fn pick(
    n: usize,
    in_chat: impl Fn(usize) -> bool,
    below: impl FnOnce(u64) -> u64,
) -> Option<usize> {
    pick_weighted(n, |i| if in_chat(i) { CHAT_FACTOR } else { 1.0 }, below)
}

/// Pick one of `n` things by `weight`, with `below(k)` uniform in
/// `0..k`; with all weighing 1, a plain uniform pick.
pub(super) fn pick_weighted(
    n: usize,
    weight: impl Fn(usize) -> f64,
    below: impl FnOnce(u64) -> u64,
) -> Option<usize> {
    if n == 0 {
        return None;
    }
    if (0..n).all(|i| weight(i) == 1.0) {
        return Some(below(n as u64) as usize);
    }
    let total: f64 = (0..n).map(&weight).sum();
    let mut roll = below(1_000_000) as f64 / 1_000_000.0 * total;
    for i in 0..n {
        if roll < weight(i) {
            return Some(i);
        }
        roll -= weight(i);
    }
    Some(n - 1)
}

/// Something she did to her home (the guest keeps the record).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum HomeEvent {
    /// Bought off the shopping channel: on order from this moment (the
    /// scene's start), whatever interrupts it.
    Bought(Furniture),
    /// Out of its box.
    Unpacked(Furniture),
    /// A makeshift piece crumpled into shape.
    Crumpled(MadeId),
    /// A makeshift piece she has started using.
    Used(MadeId),
}

impl Chances {
    /// Whether `spot` is in the chat pane (and she's resident).
    pub(super) fn in_chat(&self, spot: (i32, i32)) -> bool {
        self.chat.is_some_and(|chat| holds(chat, spot))
    }

    /// Where she'd use a piece she made for its next step: crumpling it
    /// while it's a heap, then what she made it for.
    pub(super) fn next_for(&self, mine: &Mine) -> Option<Seat> {
        let what = if mine.done {
            mine.purpose
        } else {
            Use::Crumple
        };
        self.seats
            .iter()
            .find(|s| s.piece == PieceRef::Made(mine.id) && s.what == what)
            .copied()
    }
}

/// One brace-and-heave cycle, stretched for longer lines.
fn heave_ms(glyphs: usize) -> u64 {
    600 * (40 + glyphs as u64) / 40
}

/// Milliseconds per cell walked (3 cells/s: dreamy, not brisk).
const WALK_MS: u64 = 333;
/// Milliseconds per row climbed.
const CLIMB_MS: u64 = 500;
/// Gravity in rows/s² — honest for a four-row-tall person.
const GRAVITY: f64 = 28.0;
/// How long she lies dazed after a real fall.
const DAZED_MS: u64 = 1500;
/// How long she peers over an edge.
const PEER_MS: u64 = 1200;
/// "!" then "?" when a chat message arrives.
const SURPRISED_MS: u64 = 1200;
const LOOK_MS: u64 = 4000;
/// A conversation keeps her watching until it's been quiet this long.
const WATCH_MS: u64 = 15_000;
const BLINK_MS: u64 = 150;
/// Reaching for two letters (and back again).
const FIDDLE_MS: u64 = 700;
/// How long a swap stays before she swaps it back.
const SWAP_KEPT_MS: (u64, u64) = (7000, 14_000);
/// "a... a..." before the sneeze, then the recoil.
const WINDUP_MS: u64 = 1400;
const RECOIL_MS: u64 = 600;
/// Knocked glyphs drop a row this often, at most `FALL_ROWS` rows.
const DROP_MS: u64 = 90;
const FALL_ROWS: u64 = 4;
/// After a sneeze: a moment's "...", then one glyph back per beat.
const OOPS_MS: u64 = 900;
const PUT_BACK_MS: u64 = 400;
/// Choices remembered for the cooldown.
const RECENT: usize = 3;
/// A refused put-back is retried this many times.
const RETRIES: u8 = 5;

/// Her fixed lines (the lints check each fits a bubble).
#[cfg(test)]
pub(super) const LINES: [&str; 7] = [OK, RIP, SCRUNCH, THERE, HOME, THROUGH, POKE];

/// After a hard landing.
const OK: &str = "...I'm OK.";
/// Things she says when spacing out (each ≤ 24 characters).
pub(super) const MUSINGS: [&str; 12] = [
    "I wish I were a bird.",
    "Why is the sky blue?",
    "Sata andagi!",
    "Melon bread...",
    "Black spots on white?",
    "Or white on black...",
    "Escalator? Elevator?",
    "Feels like I could fly.",
    "Which hand's left...",
    "Chiyo-chan's dad...",
    "Nanja-kora.",
    "Oh my gah.",
];

/// Tearing text off a line for furniture: bracing, then the rip.
const BRACE_MS: u64 = 700;
/// Each step of reeling the torn text in to her hands.
const REEL_MS: u64 = 220;
const RIP: &str = "Rrrip!";
const SCRUNCH: &str = "scrunch...";
const THERE: &str = "There!";
/// How long she keeps saying `text`.
fn speech_ms(text: &str) -> u64 {
    1200 + 60 * text.chars().count() as u64
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    Stand {
        until: u64,
    },
    SpaceOut {
        until: u64,
    },
    Walk {
        to: i32,
        then: Then,
    },
    Peer {
        until: u64,
        then: Option<Link>,
    },
    Climb {
        to_y: i32,
    },
    Fall {
        from_y: i32,
        since: u64,
        to_y: i32,
    },
    Dazed {
        until: u64,
    },
    Look {
        surprised_until: u64,
        until: u64,
    },
    /// Pulling `pull`: bracing, or heaving (stepping back with the line).
    Pull {
        pull: Pull,
        offset: u16,
        goal: u16,
        heaving: bool,
    },
    /// Done pulling: a pleased moment.
    Admire {
        until: u64,
    },
    /// A glance at something she lost (a beat she owes; see
    /// [`Osaka::owe`]).
    Glance {
        until: u64,
    },
    /// Reaching for the letters of `swap` (`back`: to undo it).
    Swap {
        swap: Swap,
        until: u64,
        back: bool,
    },
    /// Giggling at `swap`, which she made; she undoes it at `revert`.
    Giggle {
        swap: Swap,
        until: u64,
        revert: u64,
    },
    /// Whistling, looking anywhere but at the letters of `swap`.
    Innocent {
        swap: Swap,
        until: u64,
        revert: u64,
    },
    /// A sneeze: the windup, then (`knocked`) the recoil.
    Sneeze {
        since: u64,
        knocked: bool,
    },
    /// Picking up what the sneeze knocked loose.
    PutBack {
        since: u64,
        until: u64,
    },
    /// An activity on the spot.
    Idle {
        what: Activity,
        since: u64,
        until: u64,
    },
    /// Clambering over a divider: over to `column`, along the pole to
    /// `to_y`, then over to `to_x` on the new floor.
    Clamber {
        column: i32,
        to_y: i32,
        to_x: i32,
    },
    /// Walking off the screen to `to`, to come back in at `enter`.
    Out {
        to: i32,
        enter: i32,
        to_y: i32,
        to_x: i32,
    },
    /// Off screen until `until`; then in from `enter`, to `to_x`.
    Away {
        until: u64,
        enter: i32,
        to_y: i32,
        to_x: i32,
    },
    /// Through a door in space from where she stands to `to` (see
    /// [`DOOR`]), away for `gap` ms between the doors.
    Door {
        since: u64,
        to: (i32, i32),
        gap: u64,
    },
    /// Back from work with her shopping.
    Home {
        until: u64,
    },
    /// Poking the chat's scrollback accordion under her feet (her
    /// errand's end).
    Poke {
        since: u64,
        until: u64,
    },
    /// Tearing text off a line to `build` furniture: bracing, then
    /// (`ripped`) reeling it in to her hands, `step` cells so far,
    /// crumpling each glyph that gets there.
    Tear {
        build: Build,
        since: u64,
        ripped: bool,
        step: u16,
    },
    /// Using a piece of her furniture, at `seat`. Watching the TV,
    /// `advert` is what the shopping channel is selling her.
    Use {
        seat: Seat,
        since: u64,
        until: u64,
        advert: Option<Furniture>,
    },
}

/// What she does on getting where she walks.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Then {
    /// Nothing in particular: she chooses again (or peers over the edge
    /// she has come to).
    Nothing,
    /// Takes the link to another floor.
    Link(Link),
    /// Does the job, at its spot.
    Job(Job),
}

/// What kind of thing an act is, for whatever asks: one exhaustive
/// match ([`Act::props`]), so a new act doesn't compile until it's
/// classified.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActProps {
    stays: Stays,
    on_chat: OnChat,
}

/// Where she stays while at an act, which [`Osaka::recheck`] keeps calm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stays {
    /// At its job's spot, chosen calm.
    Job,
    /// Where she settled choosing, if it was calm then.
    Rest,
    /// Nowhere: she's passing, or it's a moment after a sneeze, a fall
    /// or a door, and she carries on.
    Pass,
}

/// What a chat line arriving does to an act.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OnChat {
    /// She looks up at once, letting go of what she was at.
    Look,
    /// On a pole or in the air: whatever comes up runs its course
    /// first, and she looks once she's on a floor.
    Landed,
    /// Out of sight, or between doors: she sees it when she's back
    /// (and there's no floor here for her to fall off).
    Back,
}

/// What stops her short (see [`Osaka::interrupt`]). Each is detected
/// where it arises.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cause {
    /// A chat line arrived ([`Osaka::look`]).
    Chat,
    /// Text came up where she stays ([`Osaka::recheck`]).
    Restless,
    /// The piece she was using went into the closet
    /// ([`Osaka::lost_seat`]).
    SeatGone,
    /// The text she was pulling or tearing changed under her
    /// ([`Osaka::lost_grip`]).
    LostGrip,
    /// Someone at the keys put back what she was at in the chat
    /// ([`Osaka::shaken`]).
    Shaken,
    /// The letters moved before she could swap them
    /// ([`Osaka::refused`]).
    Refused,
}

/// Why she lets go of where she was heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Letting {
    /// It's gone (the text changed, the piece went away).
    Gone,
    /// She chose something else.
    Other,
    /// A chat line, on her way to a piece she made: she comes back to it
    /// as another try (see [`Osaka::leftover`]).
    Chat,
    /// The stage put her somewhere.
    Placed,
}

/// Which kind of decision it was: a pre-empt, carrying on with what she
/// was about, or a roll among what's on offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bucket {
    Reflex,
    /// A beat she owed.
    Owed,
    Continuation,
    Normal,
}

/// Her latest decisions kept, for the stage.
const LOG: usize = 16;

/// One decision, as the explain log keeps it: what kind, by which
/// reflex or method, what she chose and among what, and what she set
/// about.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Decision {
    pub at: u64,
    pub bucket: Bucket,
    pub method: &'static str,
    /// What she chose, rolling.
    pub want: Option<Want>,
    /// What she rolled among, best first, with their scores.
    pub top: Vec<(Want, f64)>,
    /// Her act, and where it takes her.
    pub act: String,
    /// What she was heading for, deciding.
    pub heading: Option<Want>,
}

impl Decision {
    fn of(bucket: Bucket, method: &'static str) -> Self {
        Self {
            at: 0,
            bucket,
            method,
            want: None,
            top: Vec::new(),
            act: String::new(),
            heading: None,
        }
    }

    fn reflex(method: &'static str) -> Self {
        Self::of(Bucket::Reflex, method)
    }
}

impl std::fmt::Display for Decision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:.1}s {:?}/{}",
            self.at as f64 / 1000.0,
            self.bucket,
            self.method
        )?;
        if let Some(want) = self.want {
            write!(f, " {want:?}")?;
        }
        if !self.top.is_empty() {
            let top: Vec<String> = self
                .top
                .iter()
                .map(|(want, score)| format!("{want:?} {score:.1}"))
                .collect();
            write!(f, " of [{}]", top.join(", "))?;
        }
        if let Some(heading) = self.heading {
            write!(f, " (heading for {heading:?})")?;
        }
        write!(f, " → {}", self.act)
    }
}

impl Act {
    /// The job she's at, at its spot.
    fn at_job(&self) -> Option<JobRef<'_>> {
        match self {
            Self::Pull { pull, .. } => Some(JobRef::Pull(pull)),
            Self::Swap { swap, .. } | Self::Giggle { swap, .. } | Self::Innocent { swap, .. } => {
                Some(JobRef::Swap(swap))
            }
            Self::Tear { build, .. } => Some(JobRef::Build(build)),
            Self::Use { seat, .. } => Some(JobRef::Use(seat)),
            _ => None,
        }
    }

    /// The job she's at, or walking to.
    fn job(&self) -> Option<JobRef<'_>> {
        match self {
            Self::Walk {
                then: Then::Job(job),
                ..
            } => Some(job.by_ref()),
            _ => self.at_job(),
        }
    }

    fn props(&self) -> ActProps {
        let (stays, on_chat) = match self {
            Self::Use { .. }
            | Self::Pull { .. }
            | Self::Tear { .. }
            | Self::Swap { .. }
            | Self::Giggle { .. }
            | Self::Innocent { .. } => (Stays::Job, OnChat::Look),
            Self::Stand { .. } | Self::SpaceOut { .. } | Self::Idle { .. } => {
                (Stays::Rest, OnChat::Look)
            }
            Self::Walk { .. }
            | Self::Peer { .. }
            | Self::Dazed { .. }
            | Self::Look { .. }
            | Self::Admire { .. }
            | Self::Glance { .. }
            | Self::Sneeze { .. }
            | Self::PutBack { .. }
            | Self::Home { .. }
            | Self::Poke { .. } => (Stays::Pass, OnChat::Look),
            Self::Climb { .. } | Self::Fall { .. } | Self::Clamber { .. } => {
                (Stays::Pass, OnChat::Landed)
            }
            Self::Out { .. } | Self::Away { .. } | Self::Door { .. } => (Stays::Pass, OnChat::Back),
        };
        ActProps { stays, on_chat }
    }
}

/// One beat of going through a door: the door (if shown), whether she
/// is, whether it's the far end yet, and for how long.
struct DoorBeat {
    door: Option<DoorFrame>,
    her: bool,
    there: bool,
    ms: u64,
}

/// A door appears, she steps through, it shuts and goes; a door appears
/// where she's going, she steps out, it shuts and goes.
const DOOR: [DoorBeat; 13] = [
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: true,
        there: false,
        ms: 600,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: true,
        there: false,
        ms: 300,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: true,
        there: false,
        ms: 700,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: false,
        there: false,
        ms: 400,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: false,
        there: false,
        ms: 250,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: false,
        there: false,
        ms: 350,
    },
    DoorBeat {
        door: None,
        her: false,
        there: false,
        ms: 600,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: false,
        there: true,
        ms: 400,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: false,
        there: true,
        ms: 250,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: false,
        there: true,
        ms: 350,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: true,
        there: true,
        ms: 600,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: true,
        there: true,
        ms: 300,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: true,
        there: true,
        ms: 400,
    },
];

/// The door beat `elapsed` ms in, and when the next begins; `None` once
/// it's over. `gap` stretches the time between the doors (she's away).
fn door_beat(elapsed: u64, gap: u64) -> Option<(&'static DoorBeat, u64)> {
    let mut end = 0;
    DOOR.iter().find_map(|beat| {
        end += if beat.door.is_none() {
            beat.ms.max(gap)
        } else {
            beat.ms
        };
        (elapsed < end).then_some((beat, end))
    })
}

/// How long a door takes to let her through and close behind her: the
/// beats before the gap.
const DOOR_THROUGH_MS: u64 = {
    let mut ms = 0;
    let mut i = 0;
    while i < DOOR.len() && DOOR[i].door.is_some() {
        ms += DOOR[i].ms;
        i += 1;
    }
    ms
};

/// A spot on a floor where her box is `clear` (of the focused pane),
/// chosen at random, the chat's floors [`CHAT_FACTOR`] as likely.
fn elsewhere(
    terrain: &Terrain,
    clear: &dyn Fn((i32, i32)) -> bool,
    chat: Option<Rect>,
    rng: &mut Rng,
) -> Option<(i32, i32)> {
    // On each floor, one of its spots that are: a single random spot
    // per floor could miss every one, and leave her standing over text
    // with calm floor to spare.
    let spots: Vec<(i32, i32)> = terrain
        .platforms
        .iter()
        .filter_map(|p| {
            let xs: Vec<i32> = (p.x0..=p.x1).filter(|&x| clear((x, p.y))).collect();
            let x = *xs.get(rng.below(xs.len() as u64) as usize)?;
            Some((x, p.y))
        })
        .collect();
    let in_chat = |i: usize| {
        spots
            .get(i)
            .is_some_and(|&spot| chat.is_some_and(|chat| holds(chat, spot)))
    };
    pick(spots.len(), in_chat, |n| rng.below(n)).and_then(|i| spots.get(i).copied())
}

/// [`elsewhere`], somewhere she can stay (clear of text) if there's such
/// a spot, else anywhere `clear`.
fn calm_elsewhere(
    terrain: &Terrain,
    clear: &dyn Fn((i32, i32)) -> bool,
    chat: Option<Rect>,
    rng: &mut Rng,
) -> Option<(i32, i32)> {
    let calm = |spot: (i32, i32)| clear(spot) && terrain.restful(spot.0, spot.1);
    elsewhere(terrain, &calm, chat, rng).or_else(|| elsewhere(terrain, clear, chat, rng))
}

/// She goes to work after this long into a visit, at the earliest.
const WORK_AFTER_MS: u64 = 3 * 60_000;
/// How long a shift lasts (ms range).
const SHIFT_MS: (u64, u64) = (60_000, 180_000);
/// Back from work, showing what she brought.
const HOME_MS: u64 = 3000;
/// What she says, back from work.
const HOME: &str = "I'm home!";

/// What she says stepping out of a door.
const THROUGH: &str = "Where was I?";

/// How long she pokes the scrollback accordion, and each poke.
const POKE_MS: u64 = 2000;
const POKE_FRAME_MS: u64 = 250;
/// Farther than this along her floor, she takes a door to the accordion.
const ERRAND_WALK: i32 = 2 * sprite::WIDTH;
/// What she says, poking it.
pub(super) const POKE: &str = "Somebody said something.";

/// The fridge stands open this long at the start of a snack.
pub(super) const FRIDGE_OPEN_MS: u64 = 1500;

/// When the cat bites, into a petting that lasts `length` ms.
pub(super) fn bite_at(length: u64) -> u64 {
    length * 7 / 10
}

/// How long she keeps at `what` (ms range).
fn use_duration(what: Use) -> (u64, u64) {
    match what {
        Use::Lounge => (15_000, 30_000),
        Use::Nap => (30_000, 60_000),
        Use::Sleep => (60_000, 180_000),
        Use::Homework => (30_000, 60_000),
        Use::Watch => (20_000, 45_000),
        Use::Unpack => (4_000, 6_000),
        Use::Read => (20_000, 40_000),
        Use::Snack => (6_000, 9_000),
        Use::Pet => (6_000, 9_000),
        Use::Crumple => (4_000, 6_000),
    }
}

/// Animation frame period for `what`.
const USE_FRAME_MS: u64 = 1400;

/// How she looks `elapsed` ms into `what`, which lasts `length` ms
/// (with `advert` on the TV; `sofa` when she's on one).
fn use_look(
    what: Use,
    advert: Option<Furniture>,
    sofa: bool,
    elapsed: u64,
    length: u64,
) -> (Pose, Face, Option<Bubble>) {
    let frame = (elapsed / USE_FRAME_MS % 2) as u8;
    // Watching from a sofa, she sits on it.
    let watching = if sofa { Pose::Lounge } else { Pose::Sit };
    match what {
        Use::Lounge => (Pose::Lounge, Face::Vacant, None),
        Use::Nap => (Pose::Nap(frame), Face::Blink, Some(Bubble::Zzz)),
        Use::Sleep => (Pose::Sleep(frame), Face::Blink, Some(Bubble::Zzz)),
        // Writing for the first half, then nodding off onto the paper.
        Use::Homework => {
            if elapsed < length / 2 {
                (Pose::Homework(frame), Face::Vacant, None)
            } else if elapsed < length * 3 / 4 {
                (Pose::Homework(2), Face::Blink, Some(Bubble::Dots))
            } else {
                (Pose::Homework(3), Face::Blink, Some(Bubble::Zzz))
            }
        }
        Use::Watch => match advert {
            // Hooked, then sold.
            Some(_) if elapsed < length * 2 / 5 => (watching, Face::Curious, Some(Bubble::Ooh)),
            Some(item) if elapsed < length * 3 / 5 => {
                (watching, Face::Happy, Some(Bubble::Say(item.spec().pitch)))
            }
            _ => (watching, Face::Curious, None),
        },
        Use::Read => (Pose::Read(frame), Face::Vacant, None),
        // A look in the fridge, then the melon bread.
        Use::Snack if elapsed < FRIDGE_OPEN_MS => (Pose::Side, Face::Curious, None),
        Use::Snack => (Pose::Eat(frame), Face::Happy, None),
        // Petting the cat, who has had quite enough.
        Use::Pet if elapsed < bite_at(length) => (Pose::Pet(0), Face::Happy, Some(Bubble::Hum)),
        Use::Pet => (Pose::Pet(1), Face::Surprised, Some(Bubble::Say("Ow!"))),
        // Scrunching the torn text into shape, pleased with it at the end.
        Use::Crumple => {
            let bubble = if elapsed > length * 4 / 5 {
                Some(Bubble::Say(THERE))
            } else {
                Some(Bubble::Say(SCRUNCH))
            };
            (Pose::ToeTouch(frame), Face::Happy, bubble)
        }
        // Bent over the box, rummaging.
        Use::Unpack => {
            let bubble = (elapsed > length * 3 / 5).then_some(Bubble::Ooh);
            (Pose::ToeTouch(frame), Face::Happy, bubble)
        }
    }
}

/// Something to do on the spot that isn't staring at the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Activity {
    Sit,
    LieBack,
    LieFront,
    Jacks,
    ToeTouch,
    Stretch,
    Gaze,
}

impl Activity {
    #[cfg(test)]
    pub const ALL: [Activity; 7] = [
        Self::Sit,
        Self::LieBack,
        Self::LieFront,
        Self::Jacks,
        Self::ToeTouch,
        Self::Stretch,
        Self::Gaze,
    ];

    /// How long she keeps at it (ms range).
    fn duration(self) -> (u64, u64) {
        match self {
            Self::Sit => (10_000, 25_000),
            Self::LieBack => (15_000, 40_000),
            Self::LieFront => (10_000, 25_000),
            Self::Jacks => (4_000, 8_000),
            Self::ToeTouch => (5_000, 9_000),
            Self::Stretch => (2_000, 4_000),
            Self::Gaze => (4_000, 10_000),
        }
    }

    /// Animation frame period; 0 for a held pose.
    fn period(self) -> u64 {
        match self {
            Self::LieBack => 1400,
            Self::LieFront => 500,
            Self::Jacks => 450,
            Self::ToeTouch => 900,
            Self::Sit | Self::Stretch | Self::Gaze => 0,
        }
    }

    fn look(self, frame: u8) -> (Pose, Face, Option<Bubble>) {
        match self {
            Self::Sit => (Pose::Sit, Face::Vacant, None),
            Self::LieBack => (Pose::LieBack(frame), Face::Blink, Some(Bubble::Zzz)),
            Self::LieFront => (Pose::LieFront(frame), Face::Happy, Some(Bubble::Hum)),
            Self::Jacks => (Pose::Jack(frame), Face::Happy, Some(Bubble::Count)),
            Self::ToeTouch => (Pose::ToeTouch(frame), Face::Vacant, None),
            Self::Stretch => (Pose::Stretch, Face::Blink, Some(Bubble::Stretch)),
            Self::Gaze => (Pose::Gaze, Face::Curious, Some(Bubble::Ooh)),
        }
    }
}

/// A speech or thought bubble.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Bubble {
    Dots,
    Bang,
    Huh,
    Hehe,
    Zzz,
    Hum,
    Count,
    Stretch,
    Ooh,
    Achoo,
    Chu,
    /// Something she says (≤ 24 characters).
    Say(&'static str),
}

impl Bubble {
    pub fn text(self) -> &'static str {
        match self {
            Self::Dots => "...",
            Self::Bang => "!",
            Self::Huh => "?",
            Self::Hehe => "hehe",
            Self::Zzz => "zzz",
            Self::Hum => "~",
            Self::Count => "1, 2!",
            Self::Stretch => "nnn~",
            Self::Ooh => "ooh",
            Self::Achoo => "a...",
            Self::Chu => "chu!",
            Self::Say(text) => text,
        }
    }
}

/// The houseguest.
#[derive(Clone, Debug)]
pub(super) struct Osaka {
    /// Anchor column (her middle).
    pub x: i32,
    /// The floor row she stands on; her body is the rows above it.
    pub y: i32,
    pub facing: Facing,
    act: Act,
    /// When the current act next changes her pose.
    act_due: u64,
    next_blink: u64,
    blink_until: u64,
    /// While a chat conversation continues she stands watching it.
    watch_until: u64,
    watch_x: i32,
    /// A job on another floor she's making her way towards, and what
    /// she wants there (see [`Osaka::drop_heading`]).
    heading: Option<Heading>,
    /// She's on a hop of her way there, uninterrupted: landing, she
    /// carries on without choosing anew.
    hopping: bool,
    /// The pole she's climbing (column).
    pole: i32,
    /// Layer changes for the next paint to apply.
    ops: Vec<LayerOp>,
    /// Changes to her home for the guest to record.
    events: Vec<HomeEvent>,
    /// Layer changes due later, whatever she's doing by then: undoing
    /// mischief is scheduled when it's made, so nothing can strand it.
    pending: Vec<(u64, LayerOp)>,
    /// What she's saying, and until when (over her act's own bubble).
    speech: Option<(&'static str, u64)>,
    /// She has said hello (or "I'm OK", which does as well).
    greeted: bool,
    /// When this visit began.
    arrived: u64,
    /// Out at her part-time job (or on her way there or back).
    at_work: bool,
    /// She's been to work this visit (once is plenty).
    worked: bool,
    needs: Needs,
    /// Her mood this visit.
    mood: Mood,
    /// Her last few choices (repeating herself is discouraged).
    recent: Vec<Want>,
    /// When she last chose.
    decided: u64,
    /// Where she's going to poke the scrollback accordion (standing on
    /// it); it comes before anything else she'd choose.
    errand: Option<(i32, i32)>,
    /// She has just started poking (the guest takes it, and shakes the
    /// accordion).
    poked: bool,
    /// How often she has set off to finish or use each piece she made
    /// (what she made it for is kept on the piece).
    tries: Vec<(MadeId, u8)>,
    /// Where she settled, choosing what to do, if it was calm then: while
    /// she rests there, she keeps checking it is (see [`Osaka::recheck`]).
    rest: Option<(i32, i32)>,
    /// Her mind's own random stream: one draw a decision (see
    /// [`Whims`]).
    mind: Rng,
    /// What she chose and hasn't done yet: it eases her needs by how
    /// much of it she does (see [`Osaka::credit_done`]).
    credit: Option<Want>,
    /// Beats she owes, oldest first (see [`Osaka::owe`]).
    owed: Vec<Beat>,
    /// The beat lines she has said this visit.
    lines: Lines,
    /// Every beat she was owed (tests read it).
    #[cfg(test)]
    pub beats: Vec<Beat>,
    /// How each heading went: set off, arrived, or let go and why
    /// (tests read it).
    #[cfg(test)]
    pub headings: Vec<String>,
    /// Her last few decisions, and why (the stage shows them).
    log: std::collections::VecDeque<Decision>,
    /// Every choice she made (tests read it).
    #[cfg(test)]
    pub choices: Vec<Want>,
    /// Every decision she made (tests read it).
    #[cfg(test)]
    pub decisions: Vec<Decision>,
}

impl Osaka {
    fn new(x: i32, y: i32, facing: Facing, act: Act, now: u64, rng: &mut Rng) -> Self {
        let mut osaka = Self {
            x,
            y,
            facing,
            act,
            act_due: now,
            next_blink: now + rng.range(4000, 9000),
            blink_until: 0,
            watch_until: 0,
            watch_x: x,
            heading: None,
            hopping: false,
            pole: x,
            ops: Vec::new(),
            events: Vec::new(),
            pending: Vec::new(),
            speech: None,
            greeted: false,
            arrived: now,
            at_work: false,
            worked: false,
            needs: Needs::default(),
            mood: Mood::Ordinary,
            recent: Vec::new(),
            decided: now,
            errand: None,
            poked: false,
            tries: Vec::new(),
            rest: None,
            mind: Rng(rng.next() ^ mind::MIND_SALT),
            credit: None,
            owed: Vec::new(),
            lines: Lines::default(),
            #[cfg(test)]
            beats: Vec::new(),
            #[cfg(test)]
            headings: Vec::new(),
            log: std::collections::VecDeque::new(),
            #[cfg(test)]
            choices: Vec::new(),
            #[cfg(test)]
            decisions: Vec::new(),
        };
        osaka.act_due = osaka.first_due(now);
        osaka
    }

    /// She enters: walking in from a screen edge a floor reaches, or
    /// dropping in from above onto a platform with a clear fall. `None`
    /// when there's nowhere to be.
    pub fn arrive(now: u64, terrain: &Terrain, width: i32, rng: &mut Rng) -> Option<Self> {
        let entries: Vec<(i32, i32, Facing, i32)> =
            terrain
                .platforms
                .iter()
                .flat_map(|p| {
                    let target = p.x0 + rng.below((p.x1 - p.x0 + 1) as u64) as i32;
                    let left = (p.edge_left && p.x0 <= sprite::WIDTH / 2).then_some((
                        -sprite::WIDTH,
                        p.y,
                        Facing::Right,
                        target,
                    ));
                    let right = (p.edge_right && p.x1 >= width - 1 - sprite::WIDTH / 2)
                        .then_some((width + sprite::WIDTH, p.y, Facing::Left, target));
                    [left, right]
                })
                .flatten()
                .collect();
        if !entries.is_empty() && rng.below(2) == 0 {
            let pick = rng.below(entries.len() as u64) as usize;
            let (x, y, facing, to) = *entries.get(pick)?;
            tracing::trace!(x, y, to, "houseguest walks in");
            return Some(Self::new(
                x,
                y,
                facing,
                Act::Walk {
                    to,
                    then: Then::Nothing,
                },
                now,
                rng,
            ));
        }
        for _ in 0..16 {
            let pick = rng.below(terrain.platforms.len().max(1) as u64) as usize;
            let platform = terrain.platforms.get(pick)?;
            let x = platform.x0 + rng.below((platform.x1 - platform.x0 + 1) as u64) as i32;
            if terrain.landing(x, 0) == Some(pick) {
                tracing::trace!(x, y = platform.y, "houseguest drops in");
                let act = Act::Fall {
                    from_y: 0,
                    since: now,
                    to_y: platform.y,
                };
                return Some(Self::new(x, 0, Facing::Right, act, now, rng));
            }
        }
        if let Some(&(x, y, facing, to)) = entries.first() {
            return Some(Self::new(
                x,
                y,
                facing,
                Act::Walk {
                    to,
                    then: Then::Nothing,
                },
                now,
                rng,
            ));
        }
        // Nowhere to walk in from or drop onto: she's simply there,
        // blinking, as if she'd been home all along.
        let pick = rng.below(terrain.platforms.len() as u64) as usize;
        let platform = terrain.platforms.get(pick)?;
        let x = platform.x0 + rng.below((platform.x1 - platform.x0 + 1) as u64) as i32;
        let until = now + rng.range(2000, 5000);
        Some(Self::new(
            x,
            platform.y,
            Facing::Right,
            Act::Stand { until },
            now,
            rng,
        ))
    }

    /// Test fixture: standing at `(x, y)`, about to decide.
    #[cfg(test)]
    pub fn standing_at(x: i32, y: i32, now: u64, rng: &mut Rng) -> Self {
        Self::new(
            x,
            y,
            Facing::Right,
            Act::Stand { until: now + 100 },
            now,
            rng,
        )
    }

    /// Test fixture: reach for `swap` now (she must stand at its spot).
    #[cfg(test)]
    pub fn swap_now(&mut self, swap: super::scenes::Swap, now: u64) {
        self.facing = side_facing(swap.side);
        self.set(
            Act::Swap {
                swap,
                until: now + FIDDLE_MS,
                back: false,
            },
            now,
        );
    }

    /// The name of what she's doing (the golden trajectories hash it).
    pub fn act_name(&self) -> String {
        let debug = format!("{:?}", self.act);
        debug
            .split([' ', '{', '('])
            .next()
            .unwrap_or_default()
            .to_owned()
    }

    /// What she's set about, for the explain log: her act, and where
    /// it takes her.
    fn act_summary(&self) -> String {
        let name = self.act_name();
        match &self.act {
            Act::Walk { to, then } => match then {
                Then::Job(job) => {
                    let what = match job.by_ref() {
                        JobRef::Pull(_) => "a pull".to_owned(),
                        JobRef::Swap(_) => "a swap".to_owned(),
                        JobRef::Build(build) => format!("making a {:?}", build.piece.item),
                        JobRef::Use(seat) => format!("{:?} ({:?})", seat.what, seat.item),
                    };
                    format!("{name} to {:?} for {what}", job.spot())
                }
                Then::Link(link) => format!("{name} to {to}, then {:?}", link.route),
                Then::Nothing => format!("{name} to {to}"),
            },
            Act::Door { to, .. } => format!("{name} to {to:?}"),
            _ => name,
        }
    }

    /// Her latest decision, and why.
    pub fn explain(&self) -> Option<&Decision> {
        self.log.back()
    }

    /// Whether any layer change is still queued.
    #[cfg(test)]
    pub fn owes_anything(&self) -> bool {
        self.owes()
    }

    /// Start a sneeze now.
    pub fn sneeze_now(&mut self, now: u64) {
        self.set(
            Act::Sneeze {
                since: now,
                knocked: false,
            },
            now,
        );
    }

    fn first_due(&self, now: u64) -> u64 {
        match self.act {
            Act::Stand { until }
            | Act::SpaceOut { until }
            | Act::Peer { until, .. }
            | Act::Dazed { until }
            | Act::Admire { until }
            | Act::Glance { until }
            | Act::Swap { until, .. }
            | Act::Giggle { until, .. }
            | Act::Innocent { until, .. }
            | Act::PutBack { until, .. }
            | Act::Home { until } => until,
            Act::Sneeze { since, knocked } => {
                since + WINDUP_MS + if knocked { RECOIL_MS } else { 0 }
            }
            Act::Tear { since, ripped, .. } => since + if ripped { REEL_MS } else { BRACE_MS },
            Act::Idle { what, since, until } => next_frame(what, since, now).min(until),
            Act::Use { since, until, .. } => {
                (since + (now.saturating_sub(since) / USE_FRAME_MS + 1) * USE_FRAME_MS).min(until)
            }
            Act::Poke { since, until } => {
                (since + (now.saturating_sub(since) / POKE_FRAME_MS + 1) * POKE_FRAME_MS).min(until)
            }
            Act::Clamber { column, to_y, .. } => {
                if self.x == column && self.y != to_y {
                    now + CLIMB_MS
                } else {
                    now + WALK_MS
                }
            }
            Act::Out { .. } => now + WALK_MS,
            Act::Away { until, .. } => until,
            Act::Door { since, gap, .. } => {
                door_beat(now.saturating_sub(since), gap).map_or(now, |(_, end)| since + end)
            }
            Act::Look {
                surprised_until, ..
            } => surprised_until,
            Act::Walk { .. } => now + WALK_MS,
            Act::Pull { ref pull, .. } => now + heave_ms(pull.cells.len()) / 2,
            Act::Climb { .. } => now + CLIMB_MS,
            Act::Fall { from_y, since, .. } => fall_time(since, (self.y - from_y + 1) as u64),
        }
    }

    /// When her pose, speech, or the text layer next changes.
    pub fn due(&self) -> u64 {
        let hush = self.speech.map_or(u64::MAX, |(_, until)| until);
        self.pose_due().min(self.pending_due()).min(hush)
    }

    /// Say `text` for a while, over whatever bubble her act shows.
    pub fn say(&mut self, text: &'static str, now: u64) {
        tracing::debug!(text, "houseguest says");
        self.speech = Some((text, now + speech_ms(text)));
    }

    /// Space out, maybe saying one of her musings first.
    pub fn muse(&mut self, now: u64, rng: &mut Rng) {
        let line = MUSINGS.get(rng.below(MUSINGS.len() as u64) as usize);
        if let Some(line) = line {
            self.say(line, now);
        }
        self.set(
            Act::SpaceOut {
                until: now + rng.range(6000, 14_000),
            },
            now,
        );
    }

    fn pose_due(&self) -> u64 {
        let blinking = matches!(self.act, Act::Stand { .. });
        if blinking {
            let blink = if self.blink_until > self.next_blink {
                self.blink_until
            } else {
                self.next_blink
            };
            self.act_due.min(blink)
        } else {
            self.act_due
        }
    }

    fn pending_due(&self) -> u64 {
        self.pending
            .iter()
            .map(|(due, _)| *due)
            .min()
            .unwrap_or(u64::MAX)
    }

    fn schedule(&mut self, due: u64, op: LayerOp) {
        self.pending.push((due, op));
    }

    /// Every piece of mischief queued has its undo scheduled with it
    /// (a lint, checked after every step she takes in debug builds).
    fn undoes_its_mischief(&self) -> bool {
        self.ops
            .iter()
            .filter(|op| matches!(op, LayerOp::Swap { .. } | LayerOp::Knock { .. }))
            .flat_map(LayerOp::sources)
            .all(|source| {
                self.pending.iter().any(|(_, op)| {
                    matches!(op, LayerOp::Restore { .. }) && op.sources().contains(&source)
                })
            })
    }

    /// Whether some mischief is still waiting to be undone.
    fn owes(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Run every event due by `now`. Returns whether her pose changed.
    pub fn tick(&mut self, now: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) -> bool {
        let mut changed = false;
        for _ in 0..64 {
            let due = self.due();
            if due > now {
                return changed;
            }
            changed = true;
            if let Some((_, until)) = self.speech
                && until == due
            {
                self.speech = None;
                continue;
            }
            if self.pending_due() == due {
                // Every op due now, in the order they were scheduled.
                let (now_due, later): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
                    .into_iter()
                    .partition(|(d, _)| *d == due);
                self.pending = later;
                self.ops.extend(now_due.into_iter().map(|(_, op)| op));
                continue;
            }
            if matches!(self.act, Act::Stand { .. }) && due < self.act_due {
                if self.blink_until > self.next_blink {
                    self.blink_until = 0;
                } else {
                    self.blink_until = due + BLINK_MS;
                    self.next_blink = due + rng.range(4000, 9000);
                }
                continue;
            }
            self.fire(due, terrain, chances, rng);
            debug_assert!(self.undoes_its_mischief(), "mischief without its undo");
        }
        // Far behind (a suspended laptop): resume from now.
        self.act_due = self.act_due.max(now);
        self.next_blink = self.next_blink.max(now);
        changed
    }

    fn set(&mut self, act: Act, at: u64) {
        tracing::trace!(?act, x = self.x, y = self.y, "houseguest act");
        // Leaving what she chose (pulling on is still pulling).
        let pulling_on = matches!((&self.act, &act), (Act::Pull { .. }, Act::Pull { .. }));
        if !pulling_on {
            self.credit_done(at);
        }
        self.act = act;
        self.act_due = self.first_due(at);
    }

    fn fire(&mut self, at: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) {
        match self.act.clone() {
            Act::Idle { what, since, until } => {
                if at >= until {
                    self.decide(at, terrain, chances, rng);
                } else {
                    self.act_due = next_frame(what, since, at).min(until);
                }
            }
            Act::Clamber { column, to_y, to_x } => {
                if self.y != to_y && self.x != column {
                    self.x += (column - self.x).signum();
                } else if self.y != to_y {
                    self.y += (to_y - self.y).signum();
                } else if self.x != to_x {
                    self.x += (to_x - self.x).signum();
                }
                if (self.x, self.y) == (to_x, to_y) {
                    self.set(Act::Stand { until: at + 800 }, at);
                } else {
                    self.act_due = self.first_due(at);
                }
            }
            Act::Out {
                to,
                enter,
                to_y,
                to_x,
            } => {
                self.x += (to - self.x).signum();
                if self.x == to {
                    tracing::debug!(work = self.at_work, "houseguest: stepped out");
                    let away = if self.at_work {
                        rng.range(SHIFT_MS.0, SHIFT_MS.1)
                    } else {
                        rng.range(4000, 12_000)
                    };
                    self.set(
                        Act::Away {
                            until: at + away,
                            enter,
                            to_y,
                            to_x,
                        },
                        at,
                    );
                } else {
                    self.act_due = at + WALK_MS;
                }
            }
            Act::Away {
                enter, to_y, to_x, ..
            } => {
                tracing::debug!("houseguest: back");
                self.x = enter;
                self.y = to_y;
                self.facing = toward(enter, to_x);
                self.set(
                    Act::Walk {
                        to: to_x,
                        then: Then::Nothing,
                    },
                    at,
                );
            }
            Act::Door { since, to, gap } => match door_beat(at.saturating_sub(since), gap) {
                Some((beat, end)) => {
                    if beat.there && (self.x, self.y) != to {
                        (self.x, self.y) = to;
                    }
                    self.act_due = since + end;
                }
                None => {
                    (self.x, self.y) = to;
                    if self.errand == Some(to) {
                        self.poke(at);
                    } else if !self.home_from_work(at) {
                        self.say(THROUGH, at);
                        self.decide(at, terrain, chances, rng);
                    }
                }
            },
            Act::Home { .. } => self.decide(at, terrain, chances, rng),
            Act::Poke { since, until } => {
                if at >= until {
                    tracing::debug!("houseguest: errand done");
                    self.errand = None;
                    // Off the log's text the way she came: by door.
                    let calm = |(x, y): (i32, i32)| terrain.restful(x, y);
                    if !terrain.restful(self.x, self.y)
                        && let Some(spot) = elsewhere(terrain, &calm, chances.chat, rng)
                    {
                        return self.through_door(spot, at);
                    }
                    self.decide(at, terrain, chances, rng);
                } else {
                    self.act_due = (since
                        + (at.saturating_sub(since) / POKE_FRAME_MS + 1) * POKE_FRAME_MS)
                        .min(until);
                }
            }
            Act::Use {
                seat, since, until, ..
            } => {
                if at >= until {
                    if seat.what == Use::Unpack {
                        tracing::info!(item = ?seat.item, "houseguest: unpacked");
                        self.events.push(HomeEvent::Unpacked(seat.item));
                    }
                    if seat.what == Use::Crumple
                        && let PieceRef::Made(id) = seat.piece
                    {
                        tracing::info!(item = ?seat.item, "houseguest: made a makeshift piece");
                        self.events.push(HomeEvent::Crumpled(id));
                        // Progress: getting to use it starts afresh. She
                        // admires it, then goes to use it (see `decide`).
                        self.tries.retain(|&(made, _)| made != id);
                        return self.set(Act::Admire { until: at + 1200 }, at);
                    }
                    self.decide(at, terrain, chances, rng);
                } else {
                    let frame =
                        since + (at.saturating_sub(since) / USE_FRAME_MS + 1) * USE_FRAME_MS;
                    self.act_due = frame.min(until);
                }
            }
            Act::Tear {
                build,
                ripped,
                step,
                ..
            } => {
                if !ripped {
                    tracing::debug!(
                        row = build.row,
                        glyphs = build.cells.len(),
                        item = ?build.piece.item,
                        "houseguest: tearing text off for furniture"
                    );
                    self.say(RIP, at);
                }
                let step = if ripped { step + 1 } else { 1 };
                if step <= build.steps() {
                    // Hand over hand: in it comes, a cell at a time.
                    self.ops.push(LayerOp::Reel {
                        row: build.row,
                        cells: build.cells.clone(),
                        hand: build.hand(),
                        step,
                    });
                    return self.set(
                        Act::Tear {
                            build,
                            since: at,
                            ripped: true,
                            step,
                        },
                        at,
                    );
                }
                // All in her hands: the heap, under her, to shape.
                let seat = build.piece.seat(Use::Crumple, 0);
                self.ops.push(LayerOp::Make {
                    row: build.row,
                    cells: build.cells,
                    piece: build.piece,
                    purpose: build.then,
                });
                self.pursue(Job::Use(seat), at);
            }
            Act::Dazed { .. } => {
                // Said in place of a hello when it's her entrance.
                if !self.greeted || rng.below(2) == 0 {
                    self.greeted = true;
                    self.say(OK, at);
                }
                self.decide(at, terrain, chances, rng)
            }
            Act::Stand { .. } | Act::SpaceOut { .. } | Act::Admire { .. } | Act::PutBack { .. } => {
                self.decide(at, terrain, chances, rng)
            }
            Act::Glance { .. } => {
                // Paid (an interrupted glance stays owed).
                if !self.owed.is_empty() {
                    self.owed.remove(0);
                }
                self.decide(at, terrain, chances, rng)
            }
            Act::Swap { back: true, .. } => {
                self.set(
                    Act::SpaceOut {
                        until: at + rng.range(1500, 3000),
                    },
                    at,
                );
            }
            Act::Swap {
                swap, back: false, ..
            } => {
                let (a, b) = (swap.a, swap.b);
                let revert = at + rng.range(SWAP_KEPT_MS.0, SWAP_KEPT_MS.1);
                tracing::debug!(?a, ?b, "houseguest: swapping two letters");
                self.ops.push(LayerOp::Swap { a, b });
                self.credit_whole(Want::Swap);
                // Back where they were shown: home, or along a line she
                // pulled.
                self.schedule(
                    revert,
                    LayerOp::Restore {
                        to: vec![a, b],
                        tries: 0,
                    },
                );
                self.set(
                    Act::Giggle {
                        swap,
                        until: at + 1500,
                        revert,
                    },
                    at,
                );
            }
            Act::Giggle { swap, revert, .. } => {
                let until = revert.saturating_sub(FIDDLE_MS).max(at);
                self.set(
                    Act::Innocent {
                        swap,
                        until,
                        revert,
                    },
                    at,
                );
            }
            Act::Innocent { swap, revert, .. } => {
                // Nobody noticed. She quietly puts it right.
                self.facing = side_facing(swap.side);
                self.set(
                    Act::Swap {
                        swap,
                        until: revert.max(at),
                        back: true,
                    },
                    at,
                );
            }
            Act::Sneeze {
                since,
                knocked: false,
            } => {
                self.sneeze(at, chances, rng);
                self.set(
                    Act::Sneeze {
                        since,
                        knocked: true,
                    },
                    at,
                );
            }
            Act::Sneeze { knocked: true, .. } => {
                let last = self
                    .pending
                    .iter()
                    .filter(|(_, op)| matches!(op, LayerOp::Restore { .. }))
                    .map(|(due, _)| *due)
                    .max();
                match last {
                    Some(until) => self.set(Act::PutBack { since: at, until }, at),
                    None => self.decide(at, terrain, chances, rng),
                }
            }
            Act::Look {
                surprised_until,
                until,
            } => {
                if at <= surprised_until && until > at {
                    self.act_due = until;
                } else {
                    self.decide(at, terrain, chances, rng);
                }
            }
            Act::Walk { to, then } => {
                if self.x != to {
                    let step = (to - self.x).signum();
                    let next = self.x + step;
                    let inside = terrain.platform_at(next, self.y).is_some();
                    let entering = terrain.platform_at(self.x, self.y).is_none() && !inside;
                    if !inside && !entering {
                        return self.decide(at, terrain, chances, rng);
                    }
                    self.x = next;
                }
                if self.x != to {
                    self.act_due = at + WALK_MS;
                    return;
                }
                if self.errand == Some((self.x, self.y)) {
                    return self.poke(at);
                }
                let then = match then {
                    Then::Job(job) if job.spot() == (self.x, self.y) => {
                        return self.start_job(job, at, chances, rng);
                    }
                    Then::Job(_) | Then::Nothing => None,
                    Then::Link(link) => Some(link),
                };
                match then {
                    Some(Link {
                        to,
                        route: Route::Clamber { column },
                        pole,
                        ..
                    }) => {
                        let Some(target) = terrain.platforms.get(to) else {
                            return self.decide(at, terrain, chances, rng);
                        };
                        self.pole = pole;
                        let to_x = target.clamp(column);
                        tracing::debug!(column, to_y = target.y, "houseguest: clambering over");
                        self.set(
                            Act::Clamber {
                                column,
                                to_y: target.y,
                                to_x,
                            },
                            at,
                        );
                    }
                    Some(Link {
                        to,
                        route: Route::Around { out, enter },
                        ..
                    }) => {
                        let Some(target) = terrain.platforms.get(to) else {
                            return self.decide(at, terrain, chances, rng);
                        };
                        // In at the end nearest where she comes back.
                        let to_x = if enter < 0 { target.x0 } else { target.x1 };
                        self.facing = toward(self.x, out);
                        self.set(
                            Act::Out {
                                to: out,
                                enter,
                                to_y: target.y,
                                to_x,
                            },
                            at,
                        );
                    }
                    Some(link) if link.route == Route::Climb => {
                        let to_y = terrain.platforms.get(link.to).map_or(self.y, |p| p.y);
                        // She faces the pole and climbs it.
                        self.pole = link.pole;
                        if link.pole != self.x {
                            self.facing = toward(self.x, link.pole);
                        }
                        self.set(Act::Climb { to_y }, at);
                    }
                    Some(link) => self.set(
                        Act::Peer {
                            until: at + PEER_MS,
                            then: Some(link),
                        },
                        at,
                    ),
                    None if self.home_from_work(at) => {}
                    None => {
                        let at_edge = terrain.platform_at(self.x, self.y).and_then(|i| {
                            let p = terrain.platforms.get(i)?;
                            (p.edge_left && self.x == p.x0 || p.edge_right && self.x == p.x1)
                                .then_some(())
                        });
                        if at_edge.is_some() && rng.below(2) == 0 {
                            self.set(
                                Act::Peer {
                                    until: at + PEER_MS,
                                    then: None,
                                },
                                at,
                            );
                        } else {
                            self.decide(at, terrain, chances, rng);
                        }
                    }
                }
            }
            Act::Peer { then, .. } => match then {
                Some(Link {
                    to,
                    route: Route::Drop { over },
                    ..
                }) => {
                    self.x = over;
                    let to_y = terrain.platforms.get(to).map_or(self.y, |p| p.y);
                    self.set(
                        Act::Fall {
                            from_y: self.y,
                            since: at,
                            to_y,
                        },
                        at,
                    );
                }
                _ => {
                    self.facing = flip(self.facing);
                    self.decide(at, terrain, chances, rng);
                }
            },
            Act::Pull {
                pull,
                offset,
                goal,
                heaving,
            } => {
                if !heaving {
                    // The heave: the line comes to her hands (reeling in
                    // the slack), then she steps back and it follows.
                    let step = pull.side.step();
                    if offset >= pull.gap {
                        let next = self.x + step;
                        let room = terrain.platform_at(next, self.y).is_some()
                            && terrain.clear(next, self.y);
                        if !room {
                            tracing::debug!("houseguest: out of floor, done pulling");
                            return self.finish_pull(at);
                        }
                        self.x = next;
                    }
                    let offset = offset + 1;
                    self.ops.push(LayerOp::Pull {
                        row: pull.row,
                        cells: pull.cells.clone(),
                        offset: (i32::from(offset) * step) as i16,
                    });
                    self.set(
                        Act::Pull {
                            pull,
                            offset,
                            goal,
                            heaving: true,
                        },
                        at,
                    );
                } else if offset >= goal {
                    self.finish_pull(at);
                } else {
                    self.set(
                        Act::Pull {
                            pull,
                            offset,
                            goal,
                            heaving: false,
                        },
                        at,
                    );
                }
            }
            Act::Climb { to_y } => {
                self.y += (to_y - self.y).signum();
                if self.y == to_y {
                    self.set(Act::Stand { until: at + 800 }, at);
                } else {
                    self.act_due = at + CLIMB_MS;
                }
            }
            Act::Fall {
                from_y,
                since,
                to_y,
            } => {
                let fallen = rows_fallen(since, at).max(self.y - from_y + 1);
                self.y = (from_y + fallen).min(to_y);
                if self.y >= to_y {
                    if to_y - from_y >= 3 {
                        self.set(
                            Act::Dazed {
                                until: at + DAZED_MS,
                            },
                            at,
                        );
                    } else {
                        self.set(Act::Stand { until: at + 600 }, at);
                    }
                } else {
                    self.act_due = fall_time(since, (self.y - from_y + 1) as u64);
                }
            }
        }
    }

    /// At `job`'s spot: set about it.
    fn start_job(&mut self, job: Job, at: u64, chances: &Chances, rng: &mut Rng) {
        self.facing = side_facing(job.side());
        let act = match job {
            Job::Use(seat) => {
                self.facing = seat.facing;
                let (lo, hi) = use_duration(seat.what);
                tracing::debug!(?seat, "houseguest: using her furniture");
                // The shopping channel: she's bought it the moment it
                // comes on.
                let advert = chances.advert.filter(|_| seat.what == Use::Watch);
                if let Some(item) = advert {
                    tracing::info!(?item, "houseguest: bought off the shopping channel");
                    self.events.push(HomeEvent::Bought(item));
                }
                if let PieceRef::Made(id) = seat.piece
                    && seat.what != Use::Crumple
                {
                    self.events.push(HomeEvent::Used(id));
                }
                Act::Use {
                    seat,
                    since: at,
                    until: at + rng.range(lo, hi),
                    advert,
                }
            }
            // At the line's end: brace to tear it.
            Job::Build(build) => Act::Tear {
                build,
                since: at,
                ripped: false,
                step: 0,
            },
            // At the word: reach for the letters.
            Job::Swap(swap) => Act::Swap {
                swap,
                until: at + FIDDLE_MS,
                back: false,
            },
            Job::Pull(pull) => {
                // At the line's end: brace. She reels in any slack first,
                // then heaves it 2–7 cells further.
                let goal = pull.gap + rng.range(2, 8) as u16;
                tracing::debug!(
                    row = pull.row,
                    glyphs = pull.cells.len(),
                    side = ?pull.side,
                    cells = goal,
                    "houseguest: pulling a line"
                );
                Act::Pull {
                    pull,
                    offset: 0,
                    goal,
                    heaving: false,
                }
            }
        };
        self.set(act, at);
    }

    fn finish_pull(&mut self, at: u64) {
        self.credit_whole(Want::Pull);
        self.set(Act::Admire { until: at + 2000 }, at);
    }

    /// What she chose eases her needs by how much of it she did: settled
    /// as she leaves doing it, by the share done. Before she gets to it,
    /// nothing is settled.
    fn credit_done(&mut self, at: u64) {
        let Some(want) = self.credit else {
            return;
        };
        let span = |since: u64, until: u64| {
            (at.saturating_sub(since) as f64 / until.saturating_sub(since).max(1) as f64)
                .clamp(0.0, 1.0)
        };
        let (done, spot) = match (&self.act, want) {
            (Act::Idle { what, since, until }, Want::Idle(chose)) if *what == chose => {
                (span(*since, *until), Spot::Floor)
            }
            (
                Act::Use {
                    seat, since, until, ..
                },
                Want::Use(chose),
            ) if seat.what == chose => {
                let spot = if seat.makeshift() {
                    Spot::Made
                } else {
                    Spot::Real(seat.item)
                };
                (span(*since, *until), spot)
            }
            (Act::Pull { offset, goal, .. }, Want::Pull) => {
                (f64::from(*offset) / f64::from((*goal).max(1)), Spot::Any)
            }
            _ => return,
        };
        self.credit = None;
        self.serve(want, done, spot);
    }

    /// She did all of what she chose, `want`.
    fn credit_whole(&mut self, want: Want) {
        if self.credit == Some(want) {
            self.credit = None;
            self.serve(want, 1.0, Spot::Any);
        }
    }

    /// `want`'s needs eased by `share` of what it answers, as well as
    /// `spot` answers each.
    fn serve(&mut self, want: Want, share: f64, spot: Spot) {
        tracing::trace!(?want, share, ?spot, "houseguest: eased by what she did");
        for &(need, amount) in want.def().serves {
            let fresh = if need == Need::Fun {
                self.needs.fresh(want)
            } else {
                1.0
            };
            self.needs
                .serve(need, amount * share * brain::quality(need, spot) * fresh);
        }
        self.needs.enjoyed(want, share);
    }

    /// Layer changes queued since the last paint.
    pub fn take_ops(&mut self) -> Vec<LayerOp> {
        std::mem::take(&mut self.ops)
    }

    /// What she did to her home since last asked.
    pub fn take_events(&mut self) -> Vec<HomeEvent> {
        std::mem::take(&mut self.events)
    }

    /// The paint refused `op` (the frame didn't allow it). A put-back is
    /// tried again shortly. Mischief that never happened owes nothing:
    /// what was queued to follow it is cancelled, and a swap she'd have
    /// giggled at leaves her puzzled instead.
    pub fn refused(&mut self, now: u64, op: LayerOp) {
        match op {
            LayerOp::Restore { to, tries } if tries < RETRIES => self.schedule(
                now + PUT_BACK_MS,
                LayerOp::Restore {
                    to,
                    tries: tries + 1,
                },
            ),
            LayerOp::Swap { .. } | LayerOp::Knock { .. } => {
                let gone = op.sources();
                self.pending
                    .retain(|(_, queued)| !queued.sources().iter().any(|c| gone.contains(c)));
                let giggling = matches!(self.act.at_job(), Some(JobRef::Swap(_)));
                if matches!(op, LayerOp::Swap { .. }) && giggling {
                    tracing::debug!("houseguest: the letters moved before she could swap them");
                    self.interrupt(Cause::Refused, now);
                }
            }
            _ => {}
        }
    }

    /// "chu!": knock 2–4 glyphs beside her loose, let them fall, and
    /// schedule putting each back.
    fn sneeze(&mut self, at: u64, chances: &Chances, rng: &mut Rng) {
        let mut loose = chances.loose.clone();
        let count = (rng.range(2, 5) as usize).min(loose.len());
        let mut knocked = Vec::with_capacity(count);
        for _ in 0..count {
            let pick = rng.below(loose.len() as u64) as usize;
            knocked.push(loose.swap_remove(pick));
        }
        tracing::debug!(knocked = knocked.len(), "houseguest: sneezed");
        let putting_back = at + RECOIL_MS + OOPS_MS;
        for (i, &source) in knocked.iter().enumerate() {
            let dir = if i32::from(source.0) < self.x { -1 } else { 1 };
            self.ops.push(LayerOp::Knock { source, dir });
            for row in 1..=FALL_ROWS {
                self.schedule(at + row * DROP_MS, LayerOp::Fall { source });
            }
            self.schedule(
                putting_back + i as u64 * PUT_BACK_MS,
                LayerOp::Restore {
                    to: vec![Placed::home(source)],
                    tries: 0,
                },
            );
        }
    }

    /// The makeshift piece she's tearing text off for, while she is.
    pub fn reeling(&self) -> Option<&Build> {
        match &self.act {
            Act::Tear { build, .. } => Some(build),
            _ => None,
        }
    }

    /// Where she's using a piece of her furniture, if she is.
    pub fn seat(&self) -> Option<Seat> {
        match self.act {
            Act::Use { seat, .. } => Some(seat),
            _ => None,
        }
    }

    /// The piece she was using went into the closet: she's back on her
    /// feet where it was, blinking.
    pub fn lost_seat(&mut self, now: u64) {
        if let Act::Use { seat, .. } = self.act {
            tracing::debug!(?seat, "houseguest: her furniture went away under her");
            self.interrupt(Cause::SeatGone, now);
        }
    }

    /// Where she's staying stopped being calm — text came up under the
    /// image she's drawn in, or a piece of her furniture joined it — so
    /// she's startled off it, to move on somewhere calm. That's while she
    /// works at a job's spot (pulling, tearing, swapping and hanging
    /// about after) or uses a piece, all chosen calm, or rests (standing,
    /// spacing out, an activity) where she settled calm. Passing
    /// (walking, climbing, falling, a door, a startled look) and the
    /// moments after a sneeze, a fall or a door, she carries on.
    pub fn recheck(&mut self, terrain: &Terrain, now: u64) {
        let here = (self.x, self.y);
        let staying = match self.act.props().stays {
            Stays::Job => self.act.at_job().map(JobRef::spot),
            Stays::Rest => self.rest,
            Stays::Pass => None,
        }
        .filter(|&at| at == here);
        let Some((x, y)) = staying else {
            return;
        };
        if terrain.restful(x, y) {
            return;
        }
        tracing::debug!(x, y, "houseguest: text came up where she stays");
        self.interrupt(Cause::Restless, now);
    }

    /// Using a piece: where, and since and until when.
    pub fn use_span(&self) -> Option<(Seat, u64, u64)> {
        match self.act {
            Act::Use {
                seat, since, until, ..
            } => Some((seat, since, until)),
            _ => None,
        }
    }

    /// Watching the TV: since when, and what the shopping channel is
    /// selling (for what's on screen).
    pub fn watching(&self) -> Option<(u64, Option<Furniture>)> {
        match self.act {
            Act::Use {
                seat,
                since,
                advert,
                ..
            } if seat.what == Use::Watch => Some((since, advert)),
            _ => None,
        }
    }

    /// Whether she's using `item` (inside it or beside it).
    #[cfg(test)]
    pub fn using(&self) -> Option<Furniture> {
        match self.act {
            Act::Use { seat, .. } => Some(seat.item),
            _ => None,
        }
    }

    /// The text she was pulling changed under her (someone scrolled the
    /// chat): she lets go and stares.
    pub fn lost_grip(&mut self, now: u64) {
        if matches!(self.act.job(), Some(JobRef::Pull(_) | JobRef::Build(_))) {
            tracing::trace!("houseguest lost her grip");
            self.interrupt(Cause::LostGrip, now);
        }
    }

    /// Choose what to do next, standing somewhere valid.
    fn decide(&mut self, at: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) {
        let why = self.choose_next(at, terrain, chances, rng);
        let decision = Decision {
            at,
            act: self.act_summary(),
            ..why
        };
        tracing::debug!(%decision, "houseguest: decided");
        #[cfg(test)]
        self.decisions.push(decision.clone());
        if self.log.len() == LOG {
            self.log.pop_front();
        }
        self.log.push_back(decision);
    }

    /// Choose what to do next and set about it; returns why.
    fn choose_next(
        &mut self,
        at: u64,
        terrain: &Terrain,
        chances: &Chances,
        rng: &mut Rng,
    ) -> Decision {
        let whims = Whims(self.mind.next());
        // What she just finished counts before she chooses anew.
        self.credit_done(at);
        self.rest = None;
        if self.errand.is_some() {
            self.head_for_errand(terrain, at);
            return Decision::reflex("errand");
        }
        let Some(here) = terrain.platform_at(self.x, self.y) else {
            self.set(Act::Stand { until: at + 1000 }, at);
            return Decision::reflex("no floor");
        };
        // Over text she only passes: on to the nearest calm spot, or by
        // door to one elsewhere.
        if !terrain.restful(self.x, self.y) && self.find_rest(here, terrain, chances, at, rng) {
            return Decision::reflex("off text");
        }
        // With nowhere calm to go, she stays as she is, and isn't startled
        // off it again.
        self.rest = terrain.restful(self.x, self.y).then_some((self.x, self.y));
        if at < self.watch_until {
            self.facing = toward(self.x, self.watch_x);
            self.set(
                Act::Stand {
                    until: self.watch_until.min(at + 5000),
                },
                at,
            );
            return Decision::reflex("watching chat");
        }
        if !self.greeted {
            self.greeted = true;
            self.say(self.mood.greeting(), at);
        }
        // Her needs move on with the time since she last chose.
        self.needs.pass(
            at.saturating_sub(self.decided),
            !chances.pulls.is_empty(),
            self.mood,
        );
        self.decided = at;
        let heading = self.heading.as_ref().map(|h| h.want);
        let hopped = std::mem::take(&mut self.hopping);
        // Something she lost: a glance toward it (maybe a word) first.
        if let Some(&beat) = self.owed.first() {
            tracing::debug!(?beat, "houseguest: a beat she owed");
            if beat.toward.0 != self.x {
                self.facing = toward(self.x, beat.toward.0);
            }
            if let Some(says) = beat.loss.says()
                && let Some(line) = self.lines.pick(says, whims, at)
            {
                self.say(line, at);
            }
            self.set(
                Act::Glance {
                    until: at + GLANCE_MS,
                },
                at,
            );
            return Decision {
                heading,
                ..Decision::of(Bucket::Owed, "beat")
            };
        }
        // Landed from a hop of her way somewhere, or making for a piece
        // she made: the next hop of the same trip, without choosing anew.
        if let Some(going) = self.heading.clone().filter(|h| hopped || h.mine()) {
            if let Some(job) = going.find(chances)
                && self.go_to(going.want, job, here, terrain, at)
            {
                let method = if hopped {
                    "heading/hop"
                } else {
                    "heading/mine"
                };
                return Decision {
                    heading,
                    ..Decision::of(Bucket::Continuation, method)
                };
            }
            self.drop_heading(Letting::Gone);
        }
        // A piece she made and hasn't finished with: she finishes it, or
        // uses it, before choosing anything new.
        if let Some((want, job)) = self.leftover(chances, terrain, here, whims, at) {
            tracing::debug!(?job, "houseguest: back to what she made");
            if self.go_to(want, job, here, terrain, at) {
                self.credit = Some(want);
                return Decision {
                    heading,
                    ..Decision::of(Bucket::Continuation, "leftover")
                };
            }
        }
        let ctx = Ctx {
            x: self.x,
            y: self.y,
            here,
            links: terrain
                .links
                .iter()
                .filter(|l| l.from == here)
                .copied()
                .collect(),
            terrain,
            chances,
            may_work: chances.furnished && !self.worked && at >= self.arrived + WORK_AFTER_MS,
            owes: self.owes(),
        };
        // What's on offer: each want one of whose methods binds, with
        // what it binds.
        let mut offers: Vec<(Want, &'static str, Bind)> = Want::ALL
            .iter()
            .filter_map(|&want| {
                mind::bind(&ctx, whims, want).map(|(name, bind)| (want, name, bind))
            })
            .collect();
        // Making for a job on another floor: where she finds it again, it's
        // on offer as she set off for it, and [`brain::INERTIA`] times as
        // likely.
        let mut inert = None;
        if let Some(heading) = self.heading.clone() {
            match heading.find(chances) {
                Some(job) => {
                    offers.retain(|(want, ..)| *want != heading.want);
                    offers.push((heading.want, "heading", Bind::Job(job)));
                    inert = Some(heading.want);
                }
                None => self.drop_heading(Letting::Gone),
            }
        }
        for attempt in 0.. {
            let wants: Vec<(Want, Spot)> = offers
                .iter()
                .map(|(want, _, bind)| (*want, bind.on()))
                .collect();
            // A resident mostly keeps out of the chat, where people read.
            let factor = |want: Want| {
                let into_chat = offers
                    .iter()
                    .find(|(w, ..)| *w == want)
                    .is_some_and(|(_, _, bind)| bind.in_chat(&ctx));
                let inertia = if inert == Some(want) {
                    brain::INERTIA
                } else {
                    1.0
                };
                want.def()
                    .factors
                    .iter()
                    .map(|&factor| match factor {
                        Factor::InChat(times) if into_chat => times,
                        Factor::InChat(_) => 1.0,
                    })
                    .product::<f64>()
                    * inertia
            };
            let Some((i, top)) =
                brain::choose(&wants, &self.needs, &self.recent, &factor, whims, attempt)
            else {
                break;
            };
            let (want, method, bind) = offers.remove(i);
            if inert.is_some_and(|w| w != want) {
                inert = None;
                self.drop_heading(Letting::Other);
            }
            if self.plan(want, bind, here, terrain, at, rng) {
                tracing::debug!(needs = %self.needs.summary(), "houseguest: her needs");
                self.recent.push(want);
                #[cfg(test)]
                self.choices.push(want);
                if self.recent.len() > RECENT {
                    self.recent.remove(0);
                }
                // Moving is the point of moving: walking and travel are
                // credited as she sets off. The rest, by what she does.
                if matches!(want, Want::Walk | Want::Travel) {
                    self.credit = None;
                    self.serve(want, 1.0, Spot::Any);
                } else {
                    self.credit = Some(want);
                }
                return Decision {
                    want: Some(want),
                    top,
                    heading,
                    ..Decision::of(Bucket::Normal, method)
                };
            }
            tracing::debug!(?want, "houseguest: couldn't after all");
        }
        self.set(Act::Stand { until: at + 2000 }, at);
        Decision {
            heading,
            ..Decision::of(Bucket::Normal, "nothing bound")
        }
    }

    /// Set about what a method bound, from platform `here`. False when
    /// there's no way there after all.
    fn plan(
        &mut self,
        want: Want,
        bind: Bind,
        here: usize,
        terrain: &Terrain,
        at: u64,
        rng: &mut Rng,
    ) -> bool {
        let act = match bind {
            Bind::Here(Here::Stand) => Act::Stand {
                until: at + rng.range(2000, 5000),
            },
            Bind::Here(Here::SpaceOut) => Act::SpaceOut {
                until: at + rng.range(6000, 14_000),
            },
            Bind::Here(Here::Muse) => {
                self.muse(at, rng);
                return true;
            }
            Bind::Here(Here::Sneeze) => Act::Sneeze {
                since: at,
                knocked: false,
            },
            Bind::Here(Here::Idle(what)) => self.idle_act(what, at, rng),
            Bind::WalkTo(to) => {
                self.facing = toward(self.x, to);
                Act::Walk {
                    to,
                    then: Then::Nothing,
                }
            }
            Bind::Take(link) => {
                self.travel(link, at);
                return true;
            }
            Bind::Door(spot) => {
                self.through_door(spot, at);
                return true;
            }
            Bind::Work(out) => {
                self.go_to_work(out, at, rng);
                return true;
            }
            Bind::Job(job) => return self.go_to(want, job, here, terrain, at),
        };
        self.set(act, at);
        true
    }

    /// The next step for a piece she made this visit and hasn't finished
    /// with, counting a try at it: the nearest first (the one she just
    /// crumpled, or was crumpling, is under her), then any she can't get
    /// to now, which is a try too. After [`TRIES`] she lets it be.
    fn leftover(
        &mut self,
        chances: &Chances,
        terrain: &Terrain,
        here: usize,
        whims: Whims,
        at: u64,
    ) -> Option<(Want, Job)> {
        // Those she has tried enough: she lets them be, with a beat.
        for m in &chances.mine {
            if !(m.done && m.used)
                && let Some((_, tries)) = self.tries.iter_mut().find(|(made, _)| *made == m.id)
                && *tries == TRIES
            {
                *tries = TRIES + 1;
                tracing::debug!(id = ?m.id, "houseguest: lets what she made be");
                self.owe(Loss::LetBe, m.at);
            }
        }
        let near = |seat: &Seat| {
            (
                terrain.platform_at(seat.x, seat.y) != Some(here),
                (seat.x - self.x).abs() + (seat.y - self.y).abs(),
            )
        };
        let mut waiting: Vec<(MadeId, Use, Option<Seat>)> = chances
            .mine
            .iter()
            .filter(|m| !(m.done && m.used) && self.tries_at(m.id) < TRIES)
            .map(|m| (m.id, m.purpose, chances.next_for(m)))
            .collect();
        waiting.sort_by_key(|(_, _, seat)| {
            seat.as_ref()
                .map_or((true, (true, i32::MAX)), |s| (false, near(s)))
        });
        for (id, purpose, seat) in waiting {
            let again = match self.tries.iter_mut().find(|(made, _)| *made == id) {
                Some((_, tries)) => {
                    *tries += 1;
                    true
                }
                None => {
                    self.tries.push((id, 1));
                    false
                }
            };
            match seat {
                Some(seat) => {
                    // Back to it after an interruption.
                    if again && let Some(line) = self.lines.pick((mind::AH_RIGHT, 1, 3), whims, at)
                    {
                        self.say(line, at);
                    }
                    return Some((Want::Use(purpose), Job::Use(seat)));
                }
                None => tracing::debug!(?id, "houseguest: can't get to what she made"),
            }
        }
        None
    }

    /// Times she has set off for piece `id` (since it was crumpled).
    pub(super) fn tries_at(&self, id: MadeId) -> u8 {
        self.tries
            .iter()
            .find(|(made, _)| *made == id)
            .map_or(0, |&(_, tries)| tries)
    }

    /// She has let `id` be, having tried enough times.
    #[cfg(test)]
    pub fn gave_up(&self, id: MadeId) -> bool {
        self.tries_at(id) >= TRIES
    }

    /// Head for `job`, for `want`: straight there on this floor (the
    /// heading is done), or along the first link of a route to its floor,
    /// heading for it. False when there's no way there.
    fn go_to(&mut self, want: Want, job: Job, here: usize, terrain: &Terrain, at: u64) -> bool {
        let (x, y) = job.spot();
        let there = terrain.platform_at(x, y);
        if there == Some(here) {
            tracing::debug!(?job, "houseguest: walking to a job");
            #[cfg(test)]
            if self.heading.is_some() {
                self.headings.push("arrived".to_owned());
            }
            self.heading = None;
            self.pursue(job, at);
            return true;
        }
        match there.map(|there| route(terrain, here, there)) {
            Some(Some(link)) => {
                tracing::debug!(?job, via = ?link.route, "houseguest: heading for a job on another floor");
                self.hopping = true;
                #[cfg(test)]
                if self.heading.is_none() {
                    self.headings.push("set off".to_owned());
                }
                self.heading = Some(Heading { want, job });
                self.travel(link, at);
                true
            }
            // No way there: a door in space, straight to it.
            Some(None) => {
                self.hopping = true;
                #[cfg(test)]
                if self.heading.is_none() {
                    self.headings.push("set off".to_owned());
                }
                self.heading = Some(Heading { want, job });
                self.through_door((x, y), at);
                true
            }
            None => false,
        }
    }

    /// The one way she lets go of where she was heading.
    fn drop_heading(&mut self, why: Letting) {
        if let Some(heading) = self.heading.take() {
            tracing::debug!(?why, want = ?heading.want, "houseguest: lets go of where she was heading");
            #[cfg(test)]
            self.headings.push(format!("let go: {why:?}"));
            if matches!(why, Letting::Gone | Letting::Other) {
                self.owe(Loss::Heading, heading.job.spot());
            }
        }
    }

    /// She lost something (`loss`, at `toward`): she owes a beat, played
    /// once the reflexes let her, before anything else she'd choose. An
    /// interrupted beat stays owed; past [`OWED`], the oldest goes.
    pub fn owe(&mut self, loss: Loss, toward: (i32, i32)) {
        tracing::debug!(?loss, ?toward, "houseguest: owes a beat");
        let beat = Beat { loss, toward };
        #[cfg(test)]
        self.beats.push(beat);
        if self.owed.len() == OWED {
            self.owed.remove(0);
        }
        self.owed.push(beat);
    }

    /// Her needs.
    pub fn needs(&self) -> &Needs {
        &self.needs
    }

    /// Her mood this visit.
    pub fn mood(&self) -> Mood {
        self.mood
    }

    /// Set her mood for the visit (drawn as it begins; the stage and
    /// tests may force one).
    pub fn set_mood(&mut self, mood: Mood) {
        tracing::info!(?mood, "houseguest: her mood this visit");
        self.mood = mood;
    }

    /// Stage: make `need` pressing.
    pub fn press(&mut self, need: Need) {
        self.needs.serve(need, -1.0);
    }

    /// Walk to `job`'s spot on this floor and do it.
    pub fn pursue(&mut self, job: Job, at: u64) {
        let (x, _) = job.spot();
        if x != self.x {
            self.facing = toward(self.x, x);
        }
        self.set(
            Act::Walk {
                to: x,
                then: Then::Job(job),
            },
            at,
        );
    }

    /// Walk to `link` on this floor and take it (climb or drop).
    pub fn travel(&mut self, link: Link, at: u64) {
        if link.x != self.x {
            self.facing = toward(self.x, link.x);
        }
        self.set(
            Act::Walk {
                to: link.x,
                then: Then::Link(link),
            },
            at,
        );
    }

    /// Do `what` on the spot.
    pub fn idle(&mut self, what: Activity, at: u64, rng: &mut Rng) {
        let act = self.idle_act(what, at, rng);
        self.set(act, at);
    }

    fn idle_act(&mut self, what: Activity, at: u64, rng: &mut Rng) -> Act {
        let (lo, hi) = what.duration();
        if matches!(what, Activity::Sit | Activity::LieBack | Activity::LieFront) {
            // Sitting and lying face either way.
            self.facing = if rng.below(2) == 0 {
                Facing::Left
            } else {
                Facing::Right
            };
        }
        Act::Idle {
            what,
            since: at,
            until: at + rng.range(lo, hi),
        }
    }

    /// Put her at `(x, y)`, standing, having forgotten what she was up
    /// to (mischief she owes is still undone on schedule).
    pub fn place(&mut self, x: i32, y: i32, at: u64) {
        self.x = x;
        self.y = y;
        self.drop_heading(Letting::Placed);
        self.watch_until = 0;
        self.speech = None;
        self.at_work = false;
        self.set(Act::Stand { until: at + 1000 }, at);
    }

    /// A chat message arrived: stop and look at it.
    pub fn look(&mut self, now: u64, chat_x: i32) {
        self.watch_until = now + WATCH_MS;
        // Wherever she is on her way, chat interrupts the trip: where she
        // was heading competes again once she's watched it.
        self.hopping = false;
        // On her way to a piece she made: what it's for is kept on the
        // piece, and she comes back to it as another try (`leftover`).
        if self.heading.as_ref().is_some_and(Heading::mine) {
            self.drop_heading(Letting::Chat);
        }
        // Someone's here: whatever she knocked over or swapped goes back
        // at once, in order.
        self.pending.sort_by_key(|(due, _)| *due);
        for (due, _) in &mut self.pending {
            *due = now;
        }
        // Out, or on her way: she'll see it when she's back.
        if self.act.props().on_chat == OnChat::Back {
            return;
        }
        self.watch_x = chat_x;
        if self.aloft() {
            return; // She looks once she has landed (decide watches).
        }
        self.facing = toward(self.x, chat_x);
        self.interrupt(Cause::Chat, now);
    }

    /// Whatever she was at, she stops and looks: startled first unless
    /// it only puzzles her, and a full look only at the chat. The job
    /// she was at goes with the act; what she made it for stays on the
    /// piece (see [`Osaka::leftover`]). She's up from wherever she'd
    /// settled, and settles again when she next chooses.
    fn interrupt(&mut self, cause: Cause, now: u64) {
        let (startled, look) = match cause {
            Cause::Chat => (SURPRISED_MS, LOOK_MS),
            Cause::Restless | Cause::SeatGone | Cause::Shaken => (SURPRISED_MS, LOOK_MS / 2),
            Cause::LostGrip | Cause::Refused => (0, LOOK_MS / 2),
        };
        self.rest = None;
        // Where she was heading now competes with what else she'd do.
        self.hopping = false;
        self.set(
            Act::Look {
                surprised_until: now + startled,
                until: now + look,
            },
            now,
        );
    }

    /// Re-anchor to a freshly read terrain (a resize, a scrolled day
    /// separator). Returns false when there's nowhere left to be.
    pub fn settle(&mut self, now: u64, terrain: &Terrain) -> bool {
        // Off screen, or between doors: nothing here to fall off.
        if self.act.props().on_chat == OnChat::Back {
            return true;
        }
        match self.act {
            Act::Clamber { .. } => {
                if terrain.clear(self.x, self.y) {
                    return true;
                }
            }
            Act::Climb { to_y } => {
                if terrain.platform_at(self.x, to_y).is_some() && terrain.clear(self.x, self.y) {
                    return true;
                }
            }
            Act::Fall { to_y, .. } => match terrain.landing(self.x, self.y) {
                Some(landing) => {
                    let landing_y = terrain.platforms.get(landing).map_or(to_y, |p| p.y);
                    if landing_y != to_y
                        && let Act::Fall { to_y, .. } = &mut self.act
                    {
                        *to_y = landing_y;
                    }
                    return true;
                }
                None if self.y < HEIGHT => return true, // still above the screen
                None => {}
            },
            // Walking in (on arrival, or back from stepping out): until her
            // box is wholly on screen and over the floor, there's no floor
            // under her, and that's fine.
            Act::Walk { to, .. }
                if terrain.platform_at(self.x, self.y).is_none()
                    && self.offscreen(terrain)
                    && terrain.platform_at(to, self.y).is_some() =>
            {
                return true;
            }
            _ => {
                if terrain.platform_at(self.x, self.y).is_some() {
                    if let Act::Walk { to, then } = &mut self.act
                        && let Some(here) = terrain.platform_at(self.x, self.y)
                        && let Some(p) = terrain.platforms.get(here)
                        && !p.contains(*to)
                    {
                        *to = p.clamp(*to);
                        *then = Then::Nothing;
                    }
                    return true;
                }
            }
        }
        // The floor went away under her.
        if let Some(landing) = terrain.landing(self.x, self.y) {
            let to_y = terrain.platforms.get(landing).map_or(self.y, |p| p.y);
            self.set(
                Act::Fall {
                    from_y: self.y,
                    since: now,
                    to_y,
                },
                now,
            );
            return true;
        }
        let nearest = terrain
            .platforms
            .iter()
            .min_by_key(|p| (p.clamp(self.x) - self.x).abs() + (p.y - self.y).abs());
        let Some(p) = nearest else {
            return false;
        };
        self.x = p.clamp(self.x);
        self.y = p.y;
        self.set(
            Act::Dazed {
                until: now + DAZED_MS,
            },
            now,
        );
        true
    }

    /// Whether any of her box is off the screen's sides.
    fn offscreen(&self, terrain: &Terrain) -> bool {
        let half = sprite::WIDTH / 2;
        self.x - half < 0 || self.x + half >= terrain.width()
    }

    /// Her current sprite cells and bubble.
    pub fn picture(&self, now: u64) -> (Vec<SpriteCell>, Option<Bubble>) {
        let (pose, face, bubble) = self.appearance(now);
        (sprite::cells(pose, self.facing, face), bubble)
    }

    /// The box row her hands work at for the current job.
    fn hands_row(&self) -> u8 {
        self.act.at_job().map_or(1, JobRef::box_row)
    }

    fn facing_sign(&self) -> i32 {
        match self.facing {
            Facing::Left => -1,
            Facing::Right => 1,
        }
    }

    /// Whether she is standing on a floor (not climbing or falling), so
    /// her line art includes the floor under her feet.
    pub fn standing(&self) -> bool {
        !self.aloft()
    }

    /// On a pole or in the air (climbing, clambering, falling): whatever
    /// comes up runs its course first, until she's on a floor.
    fn aloft(&self) -> bool {
        self.act.props().on_chat == OnChat::Landed
    }

    /// Whether she's out of sight (through a door, or stepped out).
    pub fn hidden(&self, now: u64) -> bool {
        match self.act {
            Act::Away { .. } => true,
            Act::Door { since, gap, .. } => {
                door_beat(now.saturating_sub(since), gap).is_some_and(|(beat, _)| !beat.her)
            }
            _ => false,
        }
    }

    /// The door she's going through, if any, as it looks at `now`.
    pub fn door(&self, now: u64) -> Option<DoorFrame> {
        match self.act {
            Act::Door { since, gap, .. } => door_beat(now.saturating_sub(since), gap)?.0.door,
            _ => None,
        }
    }

    /// Go through a door in space to `(x, y)`, ignoring whatever lies
    /// between: her way out when there's no other.
    pub fn through_door(&mut self, to: (i32, i32), at: u64) {
        tracing::debug!(from = ?(self.x, self.y), ?to, "houseguest: a door in space");
        self.set(
            Act::Door {
                since: at,
                to,
                gap: 0,
            },
            at,
        );
    }

    /// She arrives for an errand: out of a door onto `spot` (the far
    /// door's beats only), to poke the scrollback accordion under it.
    pub fn arrive_for_errand(spot: (i32, i32), now: u64, rng: &mut Rng) -> Self {
        let act = Act::Door {
            since: now.saturating_sub(DOOR_THROUGH_MS),
            to: spot,
            gap: 0,
        };
        let mut osaka = Self::new(spot.0, spot.1, Facing::Right, act, now, rng);
        osaka.errand = Some(spot);
        osaka
    }

    /// Off to poke the scrollback accordion, standing at `spot` on it:
    /// whatever she's doing is dropped (a fall, a climb or a door she's
    /// already through runs its course first), and she walks there along
    /// her floor or takes a door. Out of sight, she comes back through
    /// one.
    pub fn errand(&mut self, spot: (i32, i32), terrain: &Terrain, at: u64) {
        tracing::debug!(?spot, "houseguest: off to poke the accordion");
        self.errand = Some(spot);
        if self.aloft() {
            return;
        }
        match &mut self.act {
            Act::Door { since, to, gap } => {
                let there =
                    door_beat(at.saturating_sub(*since), *gap).is_none_or(|(beat, _)| beat.there);
                if !there {
                    *to = spot;
                    *gap = 0;
                    self.at_work = false;
                }
            }
            Act::Away { .. } | Act::Out { .. } => {
                // Work can wait.
                self.at_work = false;
                self.set(
                    Act::Door {
                        since: at.saturating_sub(DOOR_THROUGH_MS),
                        to: spot,
                        gap: 0,
                    },
                    at,
                );
            }
            _ => self.head_for_errand(terrain, at),
        }
    }

    /// On her way to the accordion, or poking it.
    pub fn on_errand(&self) -> bool {
        self.errand.is_some()
    }

    /// The accordion's gone (the log follows the newest line again) or
    /// moved: never mind.
    pub fn drop_errand(&mut self, at: u64) {
        if self.errand.take().is_some() {
            tracing::debug!("houseguest: errand dropped");
            if matches!(self.act, Act::Poke { .. }) {
                self.set(Act::Stand { until: at + 800 }, at);
            }
        }
    }

    /// Whether she started poking since last asked.
    pub fn take_poked(&mut self) -> bool {
        std::mem::take(&mut self.poked)
    }

    /// Move off the text she's standing over: walk to the nearest calm
    /// spot on her floor, or take a door to one elsewhere. False when
    /// there's none anywhere (she stays).
    fn find_rest(
        &mut self,
        here: usize,
        terrain: &Terrain,
        chances: &Chances,
        at: u64,
        rng: &mut Rng,
    ) -> bool {
        if let Some(x) = terrain.nearest_rest(here, self.x) {
            tracing::debug!(from = self.x, to = x, "houseguest: off the text");
            self.facing = toward(self.x, x);
            self.set(
                Act::Walk {
                    to: x,
                    then: Then::Nothing,
                },
                at,
            );
            return true;
        }
        let calm = |(x, y): (i32, i32)| terrain.restful(x, y);
        match elsewhere(terrain, &calm, chances.chat, rng) {
            Some(spot) => {
                self.through_door(spot, at);
                true
            }
            None => false,
        }
    }

    fn head_for_errand(&mut self, terrain: &Terrain, at: u64) {
        let Some(spot) = self.errand else {
            return;
        };
        if (self.x, self.y) == spot {
            return self.poke(at);
        }
        // Close by on the same floor she walks; otherwise a door, so she
        // doesn't trail along the log's text.
        let here = terrain.platform_at(self.x, self.y);
        if here.is_some()
            && here == terrain.platform_at(spot.0, spot.1)
            && (spot.0 - self.x).abs() <= ERRAND_WALK
        {
            self.facing = toward(self.x, spot.0);
            self.set(
                Act::Walk {
                    to: spot.0,
                    then: Then::Nothing,
                },
                at,
            );
        } else {
            self.through_door(spot, at);
        }
    }

    fn poke(&mut self, at: u64) {
        tracing::debug!("houseguest: poking the accordion");
        self.poked = true;
        self.say(POKE, at);
        self.set(
            Act::Poke {
                since: at,
                until: at + POKE_MS,
            },
            at,
        );
    }

    /// The focused pane `focus` covers where she is (she's resident): she
    /// has rained out of it, and is already through her door, which
    /// opens on a floor clear of it — the chat's floors [`CHAT_FACTOR`]
    /// as likely — where she steps out. Headed for work, she still
    /// goes; on her way home, she's home. Returns false when no floor is
    /// clear of it.
    pub fn evict(
        &mut self,
        focus: Rect,
        terrain: &Terrain,
        chat: Option<Rect>,
        now: u64,
        rng: &mut Rng,
    ) -> bool {
        let clear = |spot: (i32, i32)| !box_meets(focus, spot);
        match &mut self.act {
            // Out of sight: when she's back in, she'll be moved on.
            Act::Away { .. } => return true,
            // Not through yet: the far door opens somewhere else instead.
            Act::Door { since, to, gap } => {
                let there =
                    door_beat(now.saturating_sub(*since), *gap).is_none_or(|(beat, _)| beat.there);
                if there {
                    if clear((self.x, self.y)) {
                        return true;
                    }
                } else {
                    if clear(*to) {
                        return true;
                    }
                    let Some(spot) = calm_elsewhere(terrain, &clear, chat, rng) else {
                        return false;
                    };
                    tracing::debug!(?spot, "houseguest: her door opens elsewhere");
                    *to = spot;
                    return true;
                }
            }
            _ => {
                if clear((self.x, self.y)) {
                    return true;
                }
            }
        }
        let Some(spot) = calm_elsewhere(terrain, &clear, chat, rng) else {
            return false;
        };
        tracing::debug!(from = ?(self.x, self.y), to = ?spot, "houseguest: out of the focused pane");
        // On her way out to work, the shift still happens; coming home
        // from it, the door is the way in.
        let leaving_for_work = self.at_work
            && matches!(
                self.act,
                Act::Out { .. }
                    | Act::Walk {
                        then: Then::Link(Link {
                            route: Route::Around { .. },
                            ..
                        }),
                        ..
                    }
            );
        let gap = if leaving_for_work {
            rng.range(SHIFT_MS.0, SHIFT_MS.1)
        } else {
            0
        };
        self.set(
            Act::Door {
                since: now.saturating_sub(DOOR_THROUGH_MS),
                to: spot,
                gap,
            },
            now,
        );
        true
    }

    /// Someone's at the keys (she's resident): whatever she'd moved in
    /// the chat has just been put back, so nothing queued for those
    /// letters is owed any more, and if she was at it there, she looks
    /// up, caught out.
    pub fn shaken(&mut self, now: u64, chat: Rect) {
        self.pending.retain(|(_, op)| {
            !op.sources()
                .iter()
                .any(|&(x, y)| chat.contains((x, y).into()))
        });
        let busy_there = self.act.at_job().is_some_and(|job| {
            matches!(job, JobRef::Pull(_) | JobRef::Swap(_)) && holds(chat, job.spot())
        });
        if busy_there {
            tracing::debug!("houseguest: shaken off in the chat");
            self.interrupt(Cause::Shaken, now);
        }
    }

    /// Off to her part-time job: out at a screen edge if her floor
    /// reaches one (`out`), else through a door, and back in 1–3
    /// minutes with her shopping.
    pub fn go_to_work(&mut self, out: Option<Link>, at: u64, rng: &mut Rng) {
        tracing::info!("houseguest: off to work");
        self.at_work = true;
        self.worked = true;
        match out {
            Some(link) => self.travel(link, at),
            None => {
                let gap = rng.range(SHIFT_MS.0, SHIFT_MS.1);
                let here = (self.x, self.y);
                self.set(
                    Act::Door {
                        since: at,
                        to: here,
                        gap,
                    },
                    at,
                );
            }
        }
    }

    /// Back from work (if she was at work): "I'm home!", showing her
    /// shopping. Returns whether she was.
    fn home_from_work(&mut self, at: u64) -> bool {
        if !std::mem::take(&mut self.at_work) {
            return false;
        }
        tracing::info!("houseguest: back from work");
        self.say(HOME, at);
        self.set(
            Act::Home {
                until: at + HOME_MS,
            },
            at,
        );
        true
    }

    /// Her pose, face and bubble at `now`.
    pub fn appearance(&self, now: u64) -> (Pose, Face, Option<Bubble>) {
        let (pose, face, bubble) = self.acting(now);
        let speech = self
            .speech
            .filter(|&(_, until)| now < until)
            .map(|(text, _)| Bubble::Say(text));
        (pose, face, speech.or(bubble))
    }

    fn acting(&self, now: u64) -> (Pose, Face, Option<Bubble>) {
        match self.act {
            Act::Stand { .. } => {
                let blink = now < self.blink_until;
                let face = if blink { Face::Blink } else { Face::Vacant };
                // Watching the chat she stands side-on, facing it.
                let pose = if now < self.watch_until {
                    Pose::Side
                } else {
                    Pose::Stand
                };
                (pose, face, None)
            }
            Act::Idle { what, since, .. } => {
                let frame = now
                    .saturating_sub(since)
                    .checked_div(what.period())
                    .map_or(0, |n| (n % 2) as u8);
                what.look(frame)
            }
            Act::Use {
                seat,
                since,
                until,
                advert,
            } => use_look(
                seat.what,
                advert,
                seat.item == Furniture::Sofa,
                now.saturating_sub(since),
                until.saturating_sub(since),
            ),
            Act::Tear { ripped, step, .. } => (
                Pose::Pull {
                    heaving: ripped && step % 2 == 1,
                    row: self.hands_row(),
                },
                if ripped { Face::Happy } else { Face::Curious },
                None,
            ),
            Act::SpaceOut { .. } => (Pose::Stand, Face::Vacant, Some(Bubble::Dots)),
            Act::Home { until } => {
                let frame = (until.saturating_sub(now) / 700 % 2) as u8;
                (Pose::Carry(frame), Face::Pleased, None)
            }
            Act::Clamber { column, to_y, .. } if self.x == column && self.y != to_y => {
                let pole = ((self.pole - self.x) * self.facing_sign()).clamp(-3, 3) as i8;
                let frame = self.y.rem_euclid(2) as u8;
                (Pose::Climb { frame, pole }, Face::Vacant, None)
            }
            Act::Clamber { .. } | Act::Out { .. } | Act::Away { .. } => {
                (Pose::Walk(self.x.rem_euclid(4) as u8), Face::Vacant, None)
            }
            Act::Door { since, gap, .. } => {
                let there = door_beat(now.saturating_sub(since), gap).is_some_and(|(b, _)| b.there);
                let face = if there { Face::Pleased } else { Face::Curious };
                (Pose::Stand, face, None)
            }
            Act::Walk { .. } => (Pose::Walk(self.x.rem_euclid(4) as u8), Face::Vacant, None),
            Act::Peer { .. } => (Pose::Peer, Face::Vacant, None),
            Act::Climb { .. } => {
                let pole = ((self.pole - self.x) * self.facing_sign()).clamp(-3, 3) as i8;
                let frame = self.y.rem_euclid(2) as u8;
                (Pose::Climb { frame, pole }, Face::Vacant, None)
            }
            Act::Fall { .. } => (Pose::Fall, Face::Surprised, None),
            Act::Dazed { .. } => (Pose::Dazed, Face::Vacant, None),
            Act::Pull { heaving, .. } => (
                Pose::Pull {
                    heaving,
                    row: self.hands_row(),
                },
                Face::Vacant,
                None,
            ),
            Act::Swap { back, .. } => {
                let face = if back { Face::Vacant } else { Face::Curious };
                let pose = Pose::Pull {
                    heaving: false,
                    row: self.hands_row(),
                };
                (pose, face, None)
            }
            Act::Giggle { .. } => (Pose::Stand, Face::Pleased, Some(Bubble::Hehe)),
            Act::Innocent { .. } => (Pose::Gaze, Face::Happy, Some(Bubble::Hum)),
            Act::Sneeze { knocked: false, .. } => (Pose::Gaze, Face::Blink, Some(Bubble::Achoo)),
            Act::Sneeze { knocked: true, .. } => {
                (Pose::ToeTouch(0), Face::Blink, Some(Bubble::Chu))
            }
            Act::PutBack { since, .. } => {
                let oops = now < since + OOPS_MS;
                let frame = (now.saturating_sub(since) / PUT_BACK_MS % 2) as u8;
                let bubble = oops.then_some(Bubble::Dots);
                (Pose::ToeTouch(frame), Face::Vacant, bubble)
            }
            Act::Admire { .. } => (Pose::Stand, Face::Pleased, Some(Bubble::Hehe)),
            Act::Glance { .. } => (Pose::Side, Face::Vacant, None),
            Act::Poke { since, .. } => {
                let frame = (now.saturating_sub(since) / POKE_FRAME_MS % 2) as u8;
                (Pose::ToeTouch(frame), Face::Curious, None)
            }
            Act::Look {
                surprised_until, ..
            } => {
                if now < surprised_until {
                    (Pose::Side, Face::Surprised, Some(Bubble::Bang))
                } else {
                    (Pose::Side, Face::Curious, Some(Bubble::Huh))
                }
            }
        }
    }
}

fn side_facing(side: Side) -> Facing {
    match side {
        Side::Left => Facing::Left,
        Side::Right => Facing::Right,
    }
}

fn flip(facing: Facing) -> Facing {
    match facing {
        Facing::Left => Facing::Right,
        Facing::Right => Facing::Left,
    }
}

fn toward(from: i32, to: i32) -> Facing {
    if to < from {
        Facing::Left
    } else {
        Facing::Right
    }
}

/// Rows fallen `since..at` under gravity.
fn rows_fallen(since: u64, at: u64) -> i32 {
    let t = at.saturating_sub(since) as f64 / 1000.0;
    (0.5 * GRAVITY * t * t).floor() as i32
}

/// When the fall started at `since` crosses its `rows`th row.
fn fall_time(since: u64, rows: u64) -> u64 {
    since + ((2.0 * rows as f64 / GRAVITY).sqrt() * 1000.0).ceil() as u64
}

/// The first link on a shortest route from platform `from` to `to`.
fn route(terrain: &Terrain, from: usize, to: usize) -> Option<Link> {
    let mut first: Vec<Option<Link>> = vec![None; terrain.platforms.len()];
    let mut seen = vec![false; terrain.platforms.len()];
    let mut queue = std::collections::VecDeque::from([from]);
    if let Some(s) = seen.get_mut(from) {
        *s = true;
    }
    while let Some(at) = queue.pop_front() {
        if at == to {
            return first.get(at).copied().flatten();
        }
        for link in terrain.links.iter().filter(|l| l.from == at) {
            if seen.get(link.to).copied().unwrap_or(true) {
                continue;
            }
            if let Some(s) = seen.get_mut(link.to) {
                *s = true;
            }
            let via = first.get(at).copied().flatten().unwrap_or(*link);
            if let Some(slot) = first.get_mut(link.to) {
                *slot = Some(via);
            }
            queue.push_back(link.to);
        }
    }
    None
}

/// The next animation-frame boundary of `what` after `now`, or far away
/// for a held pose.
fn next_frame(what: Activity, since: u64, now: u64) -> u64 {
    let period = what.period();
    if period == 0 {
        return u64::MAX;
    }
    since + (now.saturating_sub(since) / period + 1) * period
}
