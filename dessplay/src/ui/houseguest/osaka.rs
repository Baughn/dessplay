//! Osaka herself: where she is, what she's doing, and when that next
//! changes. Every activity is a finite, wall-clock-timed step, so any
//! instant is a safe place to cut the visit short.

use super::Rng;
use super::art::DoorFrame;
use super::brain::{self, Factor, Mood, Need, Needs, Rising, Spot, Want};
use super::layer::Placed;
use super::mind::{self, Beat, Bind, Ctx, Heading, Here, Lines, Loss, PoolId, RIDDLES, Whims};
use super::room::{Furniture, MadeId, PieceRef, Seat, Use};
use super::rules::{Grievance, Placement, Repair, TIE_CELLS, Trials};
use super::scenes::{Build, Job, JobRef, LayerOp, Lift, Pull, SetDown, Side, Swap};
use super::script::{self, CHANNEL_FRAME_MS, Chat, Cue, Play, Prop, ScriptId, SpliceCtx, SpliceId};
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
    /// The rules of her home broken now.
    pub broken: Vec<super::rules::Broken>,
    /// How she would put right the first rule of her home she would mend
    /// that any move mends (see [`Osaka::to_mend`]), cheapest first.
    pub repairs: Vec<super::rules::Repair>,
    /// Where she'd stand to lift each piece those repairs move (beside
    /// it, on its floor), and which way she'd face it.
    pub lift_at: Vec<LiftAt>,
    /// What the frame made of the piece she's moving, if she is.
    pub judged: Option<Judged>,
    /// How pretty the room she's in is: what's pretty on the strip she
    /// stands on (0 off any strip).
    pub beauty_here: f64,
}

/// Where she'd stand to lift a piece (beside it, on its floor), and which
/// way she'd face it.
pub(super) type LiftAt = (Furniture, (i32, i32), Side);

/// Moving a piece of her home to put a rule of it right: from the moment
/// she sets off to lift it until it's set down where it's right, or she
/// lets it go (see [`Osaka::arrange_next`]). It's hers, not her act's:
/// whatever she's startled into, sent off to or carried through her door
/// by, the piece stays in her pocket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Episode {
    /// The move she means to make.
    pub repair: Repair,
    /// Lifted: the piece is in her pocket (shown nowhere, its place kept).
    pub pocket: bool,
    /// Set down, and not yet taken (or refused) by the frame.
    pub set_down: bool,
    /// The times setting off for this step of it has come to nothing
    /// (no way there, or her walk toward it ended short of it).
    /// Interruptions on the way cost none.
    pub tries: u8,
    /// The other spots she means to try it in, if the one she sets it
    /// down in doesn't feel right.
    pub trials: Trials,
    /// The spots she has set it down in (the first was her one thing
    /// about her home: the rule's been right since).
    pub tried: u8,
    /// Set down in a spot she's trying it in: she sits on it a moment,
    /// then keeps it there or tries the next (see [`Osaka::arrange_next`]).
    pub trying: bool,
}

/// What the last frame made of the move she's making: the move it
/// judged (lifted or not), whether it still puts the rule right and
/// fits, where she'd stand for its next step (beside the piece to lift
/// it; beside where it goes to set it down) and which way she'd face,
/// and where the piece stands at its anchor (what she glances back at,
/// letting it go).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Judged {
    pub piece: Furniture,
    pub to: Placement,
    pub pocket: bool,
    pub holds: bool,
    pub spot: Option<((i32, i32), Side)>,
    pub home: Option<(i32, i32)>,
}

impl Judged {
    /// Whether it's the step `ep` is at.
    pub fn of(&self, ep: &Episode) -> bool {
        (self.piece, self.to, self.pocket) == (ep.repair.piece, ep.repair.to, ep.pocket)
    }
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
/// is one. And the times setting off for a step of moving a piece of her
/// home may come to nothing (interruptions aren't counted there: the
/// piece is hers through them) before she lets the move go.
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
    /// A piece of hers set down at `to`: hers once the frame takes it
    /// (it must fit there then; see `Guest::paint`).
    SetDown { piece: Furniture, to: Placement },
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
pub(super) const LOOK_MS: u64 = 4000;
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
/// Lifting a piece into her pocket, and setting it down.
const LIFT_MS: u64 = 900;
const SET_DOWN_MS: u64 = 900;
/// Each bob, bent to the piece.
const LIFT_FRAME_MS: u64 = 450;
/// A moment after lifting or setting down (the frame judges it before
/// she chooses again).
const AFTER_CARRY_MS: u64 = 500;
/// Waiting for the frame to judge a step of moving a piece.
const WAIT_MS: u64 = 300;
/// Waiting when she can't get to the next step of moving a piece just
/// now (a try spent), before she sets off again.
const UNREACHED_MS: u64 = 2000;
/// Lifting a piece.
const HUP: &str = line!("Hup!");
/// Trying a piece where she has set it down.
const HMM: &str = line!("hmm...");
/// Using a piece she's trying where it stands: a moment.
const TRIAL_USE_MS: (u64, u64) = (3500, 5000);
/// The lowest her restlessness counts for, weighing whether to keep a
/// piece where she's trying it (see [`Osaka::arrange_next`]).
const KEEP_T_MIN: f64 = 0.05;

/// Choices remembered for the cooldown.
const RECENT: usize = 3;
/// A refused put-back is retried this many times.
const RETRIES: u8 = 5;

/// After a hard landing.
const OK: &str = line!("...I'm OK.");
/// Spacing out, musing or not (ms range).
pub(super) const SPACE_OUT_MS: (u64, u64) = (6000, 14_000);

/// Tearing text off a line for furniture: bracing, then the rip.
const BRACE_MS: u64 = 700;
/// Each step of reeling the torn text in to her hands.
const REEL_MS: u64 = 220;
const RIP: &str = line!("Rrrip!");
pub(super) const SCRUNCH: &str = line!("scrunch...");
pub(super) const THERE: &str = line!("There!");
/// How long she keeps saying `text`.
pub(super) fn speech_ms(text: &str) -> u64 {
    1200 + 60 * text.chars().count() as u64
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    Stand {
        until: u64,
    },
    /// Spacing out, maybe telling a riddle (a musing only).
    SpaceOut {
        since: u64,
        until: u64,
        play: Option<Play>,
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
    /// Using a piece of her furniture, at `seat`, playing `play` (its
    /// own script, or on a watch the shopping channel's, with what that
    /// sold her, or surfing; and any prelude and coda spliced round it).
    /// `since..until` spans it all, prelude, body and coda: the body
    /// runs from [`Play::body_start`] to [`Play::body_end`], and its
    /// look, wakeups and grievance are timed from its start.
    /// `grievance`: a rule of her home she feels is broken using it,
    /// and when she starts to say so (see [`GRIEVANCE_MS`]).
    Use {
        seat: Seat,
        since: u64,
        until: u64,
        /// How long a whole use of it is (ms): the body's length (a
        /// trial sit's, a moment of a whole use's). What she's eased by
        /// is the share of this she did, counted from the body's start:
        /// none in a prelude, all of it in a coda.
        whole: u64,
        play: Play,
        grievance: Option<(Grievance, u64)>,
    },
    /// Bent to a piece of her furniture, lifting it into her pocket
    /// ("Hup!").
    Lift {
        lift: Lift,
        since: u64,
        until: u64,
    },
    /// Setting the piece in her pocket down where it's right.
    SetDown {
        set: SetDown,
        since: u64,
        until: u64,
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
    /// A chat line arrived where her image hides text: she passes on
    /// without stopping ([`Osaka::look`]).
    ChatPassing,
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
    /// A step of moving a piece of her home: the piece she's moving owns
    /// what's lost (see [`Osaka::drop_episode`]).
    Carry,
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
            Self::Lift { lift, .. } => Some(JobRef::Lift(lift)),
            Self::SetDown { set, .. } => Some(JobRef::SetDown(set)),
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
            | Self::Lift { .. }
            | Self::SetDown { .. }
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
    script::at(&DOOR, elapsed, |beat| {
        end += if beat.door.is_none() {
            beat.ms.max(gap)
        } else {
            beat.ms
        };
        end
    })
    .map(|(_, beat, end)| (beat, end))
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
const HOME: &str = line!("I'm home!");

/// How long she pokes the scrollback accordion, and each poke.
const POKE_MS: u64 = 2000;
const POKE_FRAME_MS: u64 = 250;
/// Farther than this along her floor, she takes a door to the accordion.
const ERRAND_WALK: i32 = 2 * sprite::WIDTH;
/// What she says, poking it.
pub(super) const POKE: &str = line!("Somebody said something.");

/// The shortest use there is: a trial sit, or the shortest use of
/// any piece.
#[cfg(test)]
pub(super) fn shortest_use_ms() -> u64 {
    Use::ALL
        .iter()
        .map(|&u| use_duration(u).0)
        .fold(TRIAL_USE_MS.0, u64::min)
}

/// How long she keeps at `what` (ms range).
#[cfg(test)]
pub(super) fn use_range(what: Use) -> (u64, u64) {
    use_duration(what)
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
pub(super) const USE_FRAME_MS: u64 = 1400;

/// `n` watches in `d` (with nothing on the shopping channel, her home
/// not on her mind, and not trying the piece) she flicks through the
/// channels, unless she has lately.
const SURF: (u64, u64) = (1, 5);

/// How long she says what's wrong with her home, using a piece: two
/// frames, from the first frame after anything she was saying (see
/// [`grievance_from`]). Felt once it has all shown.
pub(super) const GRIEVANCE_MS: u64 = 2 * USE_FRAME_MS;

/// When a use begun at `since` has her say what's wrong with her home:
/// on a frame, the first one after she's done saying anything else
/// (`quiet`), and never on the first.
fn grievance_from(since: u64, quiet: u64) -> u64 {
    let frames = quiet.saturating_sub(since).div_ceil(USE_FRAME_MS).max(1);
    since + frames * USE_FRAME_MS
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

    /// Whether it's rest (sitting, lying, gazing), not exercise: what
    /// doesn't answer restlessness. Resting in a pretty room eases her
    /// want of beauty.
    pub fn restful(self) -> bool {
        match self {
            Self::Sit | Self::LieBack | Self::LieFront | Self::Gaze => true,
            Self::Jacks | Self::ToeTouch | Self::Stretch => false,
        }
    }

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
    /// A twinkle: something came out just right.
    Sparkle,
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
            Self::Sparkle => "*'*",
            Self::Say(text) => text,
        }
    }
}

/// A rule of her home she has felt broken, and the piece and use she
/// felt it on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Felt {
    key: Grievance,
    on: (Furniture, Use),
    /// She set about putting it right and let it go: it stays as it is
    /// this visit.
    let_go: bool,
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
    /// Her latest decision's whims: what she says or plays starting an
    /// act outside a decision (out of a door, or a musing) is drawn from
    /// them, each choice with its own label. Before her first decision,
    /// her mind's seed, salted (no draw).
    whims: Whims,
    /// What the stage cued her to play next, forced rather than rolled:
    /// it waits through anything else she does, and the first use it
    /// plays on ([`Cue::plays_on`]), or musing (a riddle), takes it.
    cued: Option<Cue>,
    /// The splice rows a use may be wrapped in: [`SpliceId::ALL`], or a
    /// test's own.
    #[cfg(test)]
    pub(super) splice_rows: &'static [SpliceId],
    /// Every splice row that may wrap a use she starts does (a test's
    /// busy harness, with her vignettes as often as they may).
    #[cfg(test)]
    pub(super) splices_sure: bool,
    /// The use she answered the chat in as a splice played on (its
    /// seat, start and end), and until when she beams (Happy) saying so
    /// (see [`Osaka::look`]). Tied to that use: any other act has its
    /// own look, however soon after.
    answering: Option<((Seat, u64, u64), u64)>,
    /// What she chose and hasn't done yet: it eases her needs by how
    /// much of it she does (see [`Osaka::credit_done`]).
    credit: Option<Want>,
    /// How pretty the room she was in at the last tick is (see
    /// [`Chances::beauty_here`]).
    beauty_here: f64,
    /// Beats she owes, oldest first (see [`Osaka::owe`]).
    owed: Vec<Beat>,
    /// The pooled lines she has said this visit, and the scripts she
    /// has played.
    lines: Lines,
    /// The rules of her home she has felt broken this visit (each felt
    /// once; see [`GRIEVANCE_MS`]), and the piece and use she felt each
    /// on.
    felt: Vec<Felt>,
    /// The piece of her home she's moving, if she is.
    episode: Option<Episode>,
    /// The piece she just set down, and what she felt was wrong using
    /// it: she sits back down to it (once).
    just_set: Option<(Furniture, Use)>,
    /// The things she has done about her home this visit (see
    /// [`Mood::home_acts`]).
    home_acts: u8,
    /// Every beat she was owed (tests read it).
    #[cfg(test)]
    pub beats: Vec<Beat>,
    /// The times she set off to try a piece in another spot (tests read
    /// it).
    #[cfg(test)]
    pub retried: u32,
    /// The times the frame took a piece she set down (tests read it).
    #[cfg(test)]
    pub set_downs: u32,
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
    /// Each share of what she chose she was credited with as she left
    /// doing it (see [`Osaka::credit_done`]), and when (tests read it).
    #[cfg(test)]
    pub credited: Vec<(Want, f64, u64)>,
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
            // Set from her mind's seed below.
            whims: Whims(0),
            cued: None,
            #[cfg(test)]
            splice_rows: &SpliceId::ALL,
            #[cfg(test)]
            splices_sure: false,
            answering: None,
            credit: None,
            beauty_here: 0.0,
            owed: Vec::new(),
            lines: Lines::default(),
            felt: Vec::new(),
            episode: None,
            just_set: None,
            home_acts: 0,
            #[cfg(test)]
            beats: Vec::new(),
            #[cfg(test)]
            retried: 0,
            #[cfg(test)]
            set_downs: 0,
            #[cfg(test)]
            headings: Vec::new(),
            log: std::collections::VecDeque::new(),
            #[cfg(test)]
            choices: Vec::new(),
            #[cfg(test)]
            decisions: Vec::new(),
            #[cfg(test)]
            credited: Vec::new(),
        };
        osaka.whims = Whims(osaka.mind.0 ^ mind::WHIMS_SALT);
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
                        JobRef::Lift(l) => format!("lifting the {}", l.repair.piece.spec().name),
                        JobRef::SetDown(s) => format!("setting the {} down", s.piece.spec().name),
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

    /// The stage: play `cue` (if any) the next time it can, forced
    /// rather than rolled; `None` lets her roll again.
    pub fn cue(&mut self, cue: Option<Cue>) {
        self.cued = cue;
    }

    /// The splice rows a use she starts may be wrapped in.
    fn splice_rows(&self) -> &'static [SpliceId] {
        #[cfg(test)]
        {
            self.splice_rows
        }
        #[cfg(not(test))]
        {
            &SpliceId::ALL
        }
    }

    /// Whether every splice row that may wrap a use she starts does (only
    /// ever in a test).
    fn splices_sure(&self) -> bool {
        #[cfg(test)]
        {
            self.splices_sure
        }
        #[cfg(not(test))]
        {
            false
        }
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
            // As each key of a riddle ends.
            Act::SpaceOut { since, until, play } => play
                .and_then(|play| play.next_end(since, until, now))
                .unwrap_or(until)
                .min(until),
            // On the frame grid from the start of the part playing (the
            // prelude, the body or the coda: what bobs and what's on TV
            // move on, timed as the part times them), as each key ends,
            // so the next one's look, line and prop come on on time, and
            // as she starts and stops saying what's wrong with her home
            // (felt the moment it has all shown).
            Act::Use {
                since,
                until,
                play,
                grievance,
                ..
            } => {
                let grid = play.next_frame(since, until, now, USE_FRAME_MS);
                let key_end = play.next_end(since, until, now).unwrap_or(until);
                let grumble = grievance
                    .into_iter()
                    .flat_map(|(_, from)| [from, from + GRIEVANCE_MS])
                    .find(|&t| t > now)
                    .unwrap_or(until);
                grid.min(key_end).min(grumble).min(until)
            }
            Act::Poke { since, until } => {
                (since + (now.saturating_sub(since) / POKE_FRAME_MS + 1) * POKE_FRAME_MS).min(until)
            }
            Act::Lift { since, until, .. } | Act::SetDown { since, until, .. } => {
                (since + (now.saturating_sub(since) / LIFT_FRAME_MS + 1) * LIFT_FRAME_MS).min(until)
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

    /// Say `text` for a while, over whatever bubble her act shows (and
    /// over what she was saying: see [`Osaka::hush`]).
    pub fn say(&mut self, text: &'static str, now: u64) {
        tracing::debug!(text, "houseguest says");
        self.hush(now);
        self.speech = Some((text, now + speech_ms(text)));
    }

    /// Stop saying what she's saying. A pooled line she started saying
    /// this very instant never showed (nothing is drawn between), so it
    /// isn't said: its record goes, and it doesn't cool. (A door line
    /// the decision after the door speaks over, say.)
    fn hush(&mut self, now: u64) {
        if let Some((text, until)) = self.speech.take()
            && until == now + speech_ms(text)
        {
            self.lines.unsay(text, now);
        }
    }

    /// Space out, telling a riddle (one musing in three) or saying one
    /// of her musings, each drawn from her latest decision's whims. A
    /// riddle only when she isn't saying something already, which would
    /// hide its question.
    pub fn muse(&mut self, now: u64, rng: &mut Rng) {
        // Cued, a riddle: whatever she was saying stops for it.
        let cued = self.cued == Some(Cue::Script(ScriptId::Riddle));
        if cued {
            self.cued = None;
            self.hush(now);
        }
        let quiet = self.speech.is_none_or(|(_, until)| until <= now);
        let pool = if cued {
            mind::Pool {
                n: 1,
                d: 1,
                ..mind::RIDDLE
            }
        } else {
            mind::RIDDLE
        };
        let riddle = quiet
            .then(|| self.lines.pick(pool, self.whims, now))
            .flatten()
            .and_then(mind::riddle_of);
        let play = match riddle.and_then(|i| Some((u8::try_from(i).ok()?, RIDDLES.get(i)?))) {
            Some((which, &(question, answer))) => {
                tracing::debug!(question, answer, "houseguest tells a riddle");
                // Said as it shows, after the question.
                self.lines
                    .note(PoolId::Riddle, answer, now + script::RIDDLE_ASKED_MS);
                Some(Play::riddle(which))
            }
            None => {
                if let Some(line) = self.lines.pick(mind::MUSINGS, self.whims, now) {
                    self.say(line, now);
                }
                None
            }
        };
        self.set(
            Act::SpaceOut {
                since: now,
                until: now + rng.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1),
                play,
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
        self.beauty_here = chances.beauty_here;
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
            Act::Idle { until, .. } => {
                if at >= until {
                    self.decide(at, terrain, chances, rng);
                } else {
                    self.act_due = self.first_due(at);
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
                    self.act_due = self.first_due(at);
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
                Some((beat, _)) => {
                    if beat.there && (self.x, self.y) != to {
                        (self.x, self.y) = to;
                    }
                    self.act_due = self.first_due(at);
                }
                None => {
                    (self.x, self.y) = to;
                    if self.errand == Some(to) {
                        self.poke(at);
                    } else if !self.home_from_work(at) {
                        // Drawn from the decision that sent her through
                        // (deciding draws anew); most doors, she says
                        // nothing. Said before deciding, so what she
                        // starts waits for it (a riddle, a grievance);
                        // if the decision speaks over it at once, it was
                        // never said (see `hush`).
                        if let Some(line) = self.lines.pick(mind::DOOR, self.whims, at) {
                            self.say(line, at);
                        }
                        self.decide(at, terrain, chances, rng);
                    }
                }
            },
            Act::Home { .. } => self.decide(at, terrain, chances, rng),
            Act::Poke { until, .. } => {
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
                    self.act_due = self.first_due(at);
                }
            }
            Act::Use {
                seat,
                until,
                grievance,
                ..
            } => {
                // What she said about her home has all shown: she's felt
                // it.
                if let Some((grievance, from)) = grievance
                    && at >= from + GRIEVANCE_MS
                    && !self.has_felt(grievance)
                {
                    tracing::info!(
                        rule = %grievance.label(),
                        "houseguest: she felt {}",
                        grievance.label()
                    );
                    self.felt.push(Felt {
                        key: grievance,
                        on: (seat.item, seat.what),
                        let_go: false,
                    });
                }
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
                    self.act_due = self.first_due(at);
                }
            }
            Act::Lift { lift, until, .. } => {
                if at < until {
                    self.act_due = self.first_due(at);
                    return;
                }
                if let Some(ep) = &mut self.episode
                    && !ep.pocket
                    && ep.repair.piece == lift.repair.piece
                {
                    tracing::info!(
                        piece = ?ep.repair.piece,
                        "houseguest: her {} in her pocket",
                        ep.repair.piece.spec().name
                    );
                    ep.pocket = true;
                    // Progress: getting it where it goes starts afresh.
                    ep.tries = 0;
                }
                self.set(
                    Act::Stand {
                        until: at + AFTER_CARRY_MS,
                    },
                    at,
                );
            }
            Act::SetDown { set, until, .. } => {
                if at < until {
                    self.act_due = self.first_due(at);
                    return;
                }
                if let Some(ep) = &mut self.episode
                    && ep.pocket
                    && !ep.set_down
                    && (ep.repair.piece, ep.repair.to) == (set.piece, set.to)
                {
                    tracing::debug!(piece = ?set.piece, to = ?set.to, "houseguest: set it down");
                    ep.set_down = true;
                    self.events.push(HomeEvent::SetDown {
                        piece: set.piece,
                        to: set.to,
                    });
                }
                self.set(
                    Act::Stand {
                        until: at + AFTER_CARRY_MS,
                    },
                    at,
                );
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
            // On to the next key of her riddle.
            Act::SpaceOut { until, .. } if at < until => self.act_due = self.first_due(at),
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
                        since: at,
                        until: at + rng.range(1500, 3000),
                        play: None,
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
                    // Not `first_due` (the startle's end): over, she
                    // looks on until `until`.
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
                        self.came_to_nothing(&then);
                        return self.decide(at, terrain, chances, rng);
                    }
                    self.x = next;
                }
                if self.x != to {
                    self.act_due = self.first_due(at);
                    return;
                }
                if self.errand == Some((self.x, self.y)) {
                    return self.poke(at);
                }
                let then = match then {
                    Then::Job(job) if job.spot() == (self.x, self.y) => {
                        return self.start_job(job, at, chances, rng);
                    }
                    Then::Job(job) => {
                        // Arrived somewhere else (the floor changed under
                        // her on the way).
                        self.came_to_nothing(&Then::Job(job));
                        None
                    }
                    Then::Nothing => None,
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
                            self.hop_came_to_nothing();
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
                            self.hop_came_to_nothing();
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
                    self.act_due = self.first_due(at);
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
                    self.act_due = self.first_due(at);
                }
            }
        }
    }

    /// Her walk for `then` ended short of it. If it was for a step of the
    /// move she's making, that setting off came to nothing: a try spent.
    fn came_to_nothing(&mut self, then: &Then) {
        match then {
            Then::Job(job) if job.carry() => self.spend_try(),
            Then::Link(_) => self.hop_came_to_nothing(),
            Then::Job(_) | Then::Nothing => {}
        }
    }

    /// A hop of her way somewhere came to nothing: a try spent, if it was
    /// toward a step of the move she's making.
    fn hop_came_to_nothing(&mut self) {
        if self.heading.as_ref().is_some_and(|h| h.job.carry()) {
            self.spend_try();
        }
    }

    /// Setting off for the step of the move she's at came to nothing.
    /// Only that costs a try: an interruption on the way (chat, a
    /// startle, an errand, her door) costs none, and she sets off again.
    fn spend_try(&mut self) {
        if let Some(ep) = &mut self.episode {
            ep.tries = ep.tries.saturating_add(1);
            tracing::debug!(
                piece = ?ep.repair.piece,
                tries = ep.tries,
                "houseguest: couldn't get to the next step of moving a piece"
            );
        }
    }

    /// At `job`'s spot: set about it.
    fn start_job(&mut self, job: Job, at: u64, chances: &Chances, rng: &mut Rng) {
        self.facing = side_facing(job.side());
        let act = match job {
            Job::Use(seat) => {
                self.facing = seat.facing;
                // Trying a piece where she has just set it down: a moment
                // on it, thoughtful.
                let trying = self.episode.is_some_and(|e| e.trying);
                let (lo, hi) = if trying {
                    TRIAL_USE_MS
                } else {
                    use_duration(seat.what)
                };
                if trying {
                    self.say(HMM, at);
                }
                tracing::debug!(?seat, trying, "houseguest: using her furniture");
                // Using a real piece, she may feel a rule of her home it
                // breaks: the first she hasn't felt this visit.
                let grievance = match seat.piece {
                    PieceRef::Real(piece) => chances
                        .broken
                        .iter()
                        .find(|b| b.felt_using(piece, seat.what) && !self.has_felt(b.key))
                        .map(|b| b.key),
                    PieceRef::Made(_) => None,
                };
                // What the stage cued, if it plays on this use (else it
                // waits for one it does).
                let cued = self
                    .cued
                    .take_if(|cue| cue.plays_on(seat.what, trying, grievance.is_some()));
                let mut quiet = self.speech.map_or(at, |(_, until)| until);
                // A prelude, a coda: drawn from her latest decision's
                // whims (never round a trial), so the body is drawn as
                // it would be without them; it starts after the prelude,
                // and everything in it is timed from there.
                let forced = match cued {
                    Some(Cue::Splice(id, branch)) => Some((id, branch)),
                    Some(Cue::Script(_)) | None => None,
                };
                let ctx = SpliceCtx {
                    what: seat.what,
                    trying,
                    quiet: quiet <= at,
                };
                let (before, after) = script::splices(
                    self.splice_rows(),
                    &ctx,
                    forced,
                    self.splices_sure(),
                    self.whims,
                    &mut self.lines,
                    at,
                );
                // Cued, a prelude plays whether she's quiet or not:
                // whatever she was saying stops for it, or it would hide
                // its first key.
                if forced.is_some() && before.is_some() && quiet > at {
                    self.hush(at);
                    quiet = at;
                }
                let body_start = at + before.map_or(0, |s| s.len);
                let grievance = grievance.map(|g| (g, grievance_from(body_start, quiet)));
                // The shopping channel: she's bought it the moment it
                // comes on (unless she has her home on her mind, or the
                // stage has her flick through the channels).
                let surf_cued = cued == Some(Cue::Script(ScriptId::Surf));
                let watching = seat.what == Use::Watch && grievance.is_none() && !trying;
                let bought = chances.advert.filter(|_| watching && !surf_cued);
                // Else, now and then, she flicks through the channels:
                // cued to, always (and it cools as if rolled); cued to
                // play anything else on the watch, never; else by the
                // chance first, so a surf that doesn't roll doesn't
                // cool.
                let surf = bought.is_none()
                    && watching
                    && match cued {
                        Some(Cue::Script(ScriptId::Surf)) => {
                            self.lines.try_play(ScriptId::Surf, at);
                            true
                        }
                        Some(Cue::Script(_)) => false,
                        Some(Cue::Splice(..)) | None => {
                            self.whims.chance("surf", 0, SURF.0, SURF.1)
                                && self.lines.try_play(ScriptId::Surf, at)
                        }
                    };
                if surf {
                    tracing::info!("houseguest: flicking through the channels");
                }
                if let Some(item) = bought {
                    tracing::info!(?item, "houseguest: bought off the shopping channel");
                    self.events.push(HomeEvent::Bought(item));
                }
                if let PieceRef::Made(id) = seat.piece
                    && seat.what != Use::Crumple
                {
                    self.events.push(HomeEvent::Used(id));
                }
                let length = rng.range(lo, hi);
                let whole = if trying {
                    let (lo, hi) = use_duration(seat.what);
                    lo.midpoint(hi)
                } else {
                    length
                };
                let plain = Play::of(seat.what, bought);
                let own = if surf { ScriptId::Surf } else { plain.own };
                let play = Play {
                    own,
                    branch: if trying { own.trial_branch() } else { 0 },
                    before,
                    after,
                    ..plain
                };
                Act::Use {
                    seat,
                    since: at,
                    until: body_start + length + after.map_or(0, |s| s.len),
                    whole,
                    play,
                    grievance,
                }
            }
            // Beside the piece she means to move: "Hup!", into her
            // pocket.
            Job::Lift(lift) => {
                if self
                    .episode
                    .is_some_and(|e| !e.pocket && e.repair.piece == lift.repair.piece)
                {
                    tracing::debug!(piece = ?lift.repair.piece, "houseguest: lifting a piece");
                    self.say(HUP, at);
                    Act::Lift {
                        lift,
                        since: at,
                        until: at + LIFT_MS,
                    }
                } else {
                    Act::Stand {
                        until: at + WAIT_MS,
                    }
                }
            }
            // Where it goes: down it goes.
            Job::SetDown(set) => {
                if self.episode.is_some_and(|e| {
                    e.pocket && !e.set_down && (e.repair.piece, e.repair.to) == (set.piece, set.to)
                }) {
                    Act::SetDown {
                        set,
                        since: at,
                        until: at + SET_DOWN_MS,
                    }
                } else {
                    Act::Stand {
                        until: at + WAIT_MS,
                    }
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
    /// as she leaves doing it, by the share done (of a use, the share of
    /// a whole one: a moment's trial sit is a little of one). Before she
    /// gets to it, nothing is settled. Resting (not exercising: see
    /// [`Activity::restful`]) or using her things in a pretty room eases
    /// her want of beauty too, by the share done (as much as one pretty
    /// thing does, at most).
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
            // By the share of its body done: none of it in a prelude,
            // all of it in a coda.
            (
                Act::Use {
                    seat,
                    since,
                    whole,
                    play,
                    ..
                },
                Want::Use(chose),
            ) if seat.what == chose => {
                let spot = if seat.makeshift() {
                    Spot::Made
                } else {
                    Spot::Real(seat.item)
                };
                let start = play.body_start(*since);
                (span(start, start + whole), spot)
            }
            (Act::Pull { offset, goal, .. }, Want::Pull) => {
                (f64::from(*offset) / f64::from((*goal).max(1)), Spot::Any)
            }
            _ => return,
        };
        self.credit = None;
        #[cfg(test)]
        self.credited.push((want, done, at));
        self.serve(want, done, spot);
        let restful = match self.act {
            Act::Idle { what, .. } => what.restful(),
            // Unpacking a parcel and crumpling text are chores, not using
            // her things.
            Act::Use { seat, .. } => !matches!(seat.what, Use::Unpack | Use::Crumple),
            _ => false,
        };
        if restful {
            self.needs
                .serve(Need::Beauty, done * self.beauty_here.min(1.0));
        }
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
        let piece = match self.act {
            Act::Use { seat, .. } => seat.piece,
            Act::Lift { lift, .. } => PieceRef::Real(lift.repair.piece),
            _ => return,
        };
        tracing::debug!(?piece, "houseguest: her furniture went away under her");
        self.interrupt(Cause::SeatGone, now);
    }

    /// The piece she's lifting, while she is.
    pub fn lifting(&self) -> Option<Furniture> {
        match self.act {
            Act::Lift { lift, .. } => Some(lift.repair.piece),
            _ => None,
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

    /// Using a piece: where, and since and until when, across any
    /// prelude and coda as well as the body (safe for what reads it: no
    /// splice wraps crumpling or unpacking, whose scraps and parcels
    /// follow the whole span).
    pub fn use_span(&self) -> Option<(Seat, u64, u64)> {
        match self.act {
            Act::Use {
                seat, since, until, ..
            } => Some((seat, since, until)),
            _ => None,
        }
    }

    /// What the script she's playing shows on her furniture at `now`
    /// (what's on TV moving every [`CHANNEL_FRAME_MS`] from the start of
    /// the part it plays in: the prelude, the body or the coda, so a
    /// splice never shifts the body's frames).
    pub fn prop(&self, now: u64) -> Option<Prop> {
        let Act::Use {
            since, until, play, ..
        } = self.act
        else {
            return None;
        };
        let (key, elapsed) = play.key(since, until, now)?;
        let frame = (elapsed / CHANNEL_FRAME_MS % 2) as u8;
        key.prop.map(|prop| prop.framed(frame))
    }

    /// What she's playing at `now`, for the stage: each part of it by
    /// name, the one playing with which of its keys (see [`Play::note`]).
    pub fn playing_note(&self, now: u64) -> Option<String> {
        match self.act {
            Act::Use {
                since, until, play, ..
            }
            | Act::SpaceOut {
                since,
                until,
                play: Some(play),
            } => Some(play.note(since, until, now)),
            _ => None,
        }
    }

    /// Using a piece: where, since when, and what it plays.
    #[cfg(test)]
    pub fn playing(&self) -> Option<(Seat, u64, Play)> {
        match self.act {
            Act::Use {
                seat, since, play, ..
            } => Some((seat, since, play)),
            _ => None,
        }
    }

    /// The census group what she's doing counts in. Wildcard-free, so a
    /// new act doesn't compile until it's put in one (nothing falls into
    /// "standing" unseen).
    #[cfg(test)]
    pub fn census_group(&self) -> &'static str {
        match self.act {
            Act::Use { .. } => "furniture",
            Act::Idle { what, .. } => match what {
                Activity::Sit | Activity::LieBack | Activity::LieFront => "floor rest",
                Activity::Gaze => "spacing out",
                Activity::Jacks | Activity::ToeTouch | Activity::Stretch => "exercise",
            },
            Act::SpaceOut { .. } => "spacing out",
            Act::Walk { .. }
            | Act::Climb { .. }
            | Act::Clamber { .. }
            | Act::Out { .. }
            | Act::Away { .. }
            | Act::Door { .. }
            | Act::Fall { .. }
            | Act::Peer { .. }
            | Act::Dazed { .. } => "moving",
            Act::Pull { .. }
            | Act::Swap { .. }
            | Act::Giggle { .. }
            | Act::Innocent { .. }
            | Act::Tear { .. }
            | Act::Sneeze { .. }
            | Act::PutBack { .. }
            | Act::Admire { .. } => "mischief",
            Act::Lift { .. } | Act::SetDown { .. } => "home",
            Act::Stand { .. }
            | Act::Look { .. }
            | Act::Glance { .. }
            | Act::Home { .. }
            | Act::Poke { .. } => "standing",
        }
    }

    /// What she's playing: using a piece (its own script, or the
    /// shopping channel, or surfing, any prelude or coda), or spacing out
    /// (a riddle).
    #[cfg(test)]
    pub fn plays(&self) -> Option<Play> {
        match self.act {
            Act::Use { play, .. } => Some(play),
            Act::SpaceOut { play, .. } => play,
            _ => None,
        }
    }

    /// Whether she's stopped to look (at the chat, or startled).
    #[cfg(test)]
    pub fn looking(&self) -> bool {
        matches!(self.act, Act::Look { .. })
    }

    /// Every pooled line she has said this visit: from which pool, and
    /// from when (tests read it).
    #[cfg(test)]
    pub fn said_lines(&self) -> &[(mind::PoolId, &'static str, u64)] {
        self.lines.said()
    }

    /// Whether she's using `item` (inside it or beside it).
    #[cfg(test)]
    pub fn using(&self) -> Option<Furniture> {
        match self.act {
            Act::Use { seat, .. } => Some(seat.item),
            _ => None,
        }
    }

    /// Using a piece: the rule of her home she'll say it breaks.
    #[cfg(test)]
    pub fn grievance(&self) -> Option<Grievance> {
        match self.act {
            Act::Use { grievance, .. } => grievance.map(|(g, _)| g),
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
        self.whims = whims;
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
        let rising = Rising {
            mess: !chances.pulls.is_empty(),
            grieved: self.grieved(chances),
            plain: chances.beauty_here <= 0.0,
        };
        self.needs
            .pass(at.saturating_sub(self.decided), rising, self.mood);
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
        let mut ctx = Ctx {
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
            may_work: chances.furnished
                && !self.worked
                && self.episode.is_none()
                && at >= self.arrived + WORK_AFTER_MS,
            owes: self.owes(),
            episode: self.episode,
            just_set: self.just_set,
            may_arrange: !self.to_mend(&chances.broken).is_empty(),
        };
        // Landed from a hop of her way somewhere, or making for a piece
        // she made: the next hop of the same trip, without choosing anew.
        // (Not a step of a move whose tries are spent: that's let go below.)
        let spent = self.episode.is_some_and(|e| e.tries >= TRIES);
        if let Some(going) = self
            .heading
            .clone()
            .filter(|h| (hopped || h.mine()) && !(h.job.carry() && spent))
        {
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
            // A step of the move, judged and still holding, and no way
            // there: a try spent. (Not judged yet, or no longer holding,
            // is for `arrange_next`: a wait, or another way.)
            let judged = self
                .episode
                .is_some_and(|ep| chances.judged.is_some_and(|j| j.of(&ep) && j.holds));
            if going.job.carry() && judged {
                self.spend_try();
            }
            self.drop_heading(Letting::Gone);
        }
        // The piece she's moving (or has just set down) comes before
        // anything new.
        if self.episode.is_some() || self.just_set.is_some() {
            if let Some(decision) = self.arrange_next(&ctx, whims, here, terrain, at, rng) {
                return Decision {
                    heading,
                    ..decision
                };
            }
            // She let it go: what's on offer is as if she never had it.
            ctx.episode = self.episode;
            ctx.just_set = self.just_set;
            ctx.may_work = chances.furnished
                && !self.worked
                && self.episode.is_none()
                && at >= self.arrived + WORK_AFTER_MS;
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
                since: at,
                until: at + rng.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1),
                play: None,
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
            Bind::Job(job) => {
                let lift = match &job {
                    Job::Lift(lift) => Some(*lift),
                    _ => None,
                };
                let went = self.go_to(want, job, here, terrain, at);
                // Setting off to lift a piece, she's moving it.
                if went
                    && let Some(lift) = lift
                    && self.episode.is_none()
                {
                    self.begin_episode(lift.repair, lift.trials);
                }
                return went;
            }
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
                    if again && let Some(line) = self.lines.pick(mind::AH_RIGHT, whims, at) {
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

    /// She lets go of putting `key` right, as when she can't get to it
    /// (tests use it).
    #[cfg(test)]
    pub fn let_go_of(&mut self, key: Grievance) {
        for felt in self.felt.iter_mut().filter(|f| f.key == key) {
            felt.let_go = true;
        }
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
            // A step of moving a piece: the piece owns what's lost.
            let why = if heading.job.carry() {
                Letting::Carry
            } else {
                why
            };
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

    /// Her needs, to set (tests).
    #[cfg(test)]
    pub fn needs_mut(&mut self) -> &mut Needs {
        &mut self.needs
    }

    /// The rules of her home she has felt broken this visit.
    pub fn felt(&self) -> Vec<Grievance> {
        self.felt.iter().map(|f| f.key).collect()
    }

    /// Whether she has felt `key` broken this visit.
    fn has_felt(&self, key: Grievance) -> bool {
        self.felt.iter().any(|f| f.key == key)
    }

    /// The stage: she has felt `key` broken, using `on` (a piece, for a
    /// use).
    pub fn feel(&mut self, key: Grievance, on: (Furniture, Use)) {
        if !self.has_felt(key) {
            tracing::info!(rule = %key.label(), "houseguest: she felt {} (cued)", key.label());
            self.felt.push(Felt {
                key,
                on,
                let_go: false,
            });
        }
    }

    /// The piece of her home she's moving, if she is.
    pub fn episode(&self) -> Option<&Episode> {
        self.episode.as_ref()
    }

    /// The piece in her pocket, if one is: it shows nowhere.
    pub fn carrying(&self) -> Option<Furniture> {
        self.episode.filter(|e| e.pocket).map(|e| e.repair.piece)
    }

    /// The things she has done about her home this visit.
    #[cfg(test)]
    pub fn home_acts(&self) -> u8 {
        self.home_acts
    }

    /// She sets about moving a piece, as `repair` says (and to try it in
    /// `trials`' spots after, if that one doesn't feel right).
    fn begin_episode(&mut self, repair: Repair, trials: Trials) {
        tracing::info!(
            rule = %repair.key.label(),
            repair = %repair.label(),
            trials = trials.remain(),
            "houseguest: she means to put her home right"
        );
        self.episode = Some(Episode {
            repair,
            pocket: false,
            set_down: false,
            tries: 0,
            trials,
            tried: 0,
            trying: false,
        });
    }

    /// The stage: lift `lift`'s piece (she must be on its floor), to move
    /// it as its repair says.
    pub fn lift(&mut self, lift: Lift, at: u64) {
        self.begin_episode(lift.repair, lift.trials);
        self.credit = Some(Want::Arrange);
        self.pursue(Job::Lift(lift), at);
    }

    /// The frame took the piece she set down, `piece`: it stands where
    /// it's right. The first time, it's one thing done about her home,
    /// and nesting eases. Next she sits back down where she felt it was
    /// wrong: pleased ("There!"), or, with another spot she means to try
    /// it in, a moment's thought first (see [`Osaka::arrange_next`]).
    pub fn set_down_done(&mut self, piece: Furniture, now: u64) {
        let Some(mut ep) = self
            .episode
            .take_if(|e| e.pocket && e.repair.piece == piece)
        else {
            return;
        };
        tracing::info!(
            ?piece,
            repair = %ep.repair.label(),
            trying = ep.trials.remain(),
            "houseguest: set her {} down where it's right",
            piece.spec().name
        );
        #[cfg(test)]
        {
            self.set_downs += 1;
        }
        if self.heading.as_ref().is_some_and(|h| h.job.carry()) {
            self.drop_heading(Letting::Carry);
        }
        // However many spots she tries it in, it's the one thing.
        if ep.tried == 0 {
            self.home_acts = self.home_acts.saturating_add(1);
            self.serve(Want::Arrange, 1.0, Spot::Any);
        }
        ep.tried = ep.tried.saturating_add(1);
        self.just_set = self
            .felt
            .iter()
            .find(|f| f.key == ep.repair.key)
            .map(|f| f.on);
        // Done: nesting eased, whatever she was at when the frame took it.
        if self.credit == Some(Want::Arrange) {
            self.credit = None;
        }
        if ep.trials.remain() {
            self.episode = Some(Episode {
                pocket: false,
                set_down: false,
                tries: 0,
                trying: true,
                ..ep
            });
        } else {
            self.say(THERE, now);
        }
    }

    /// The frame wouldn't take the piece she set down (it no longer fits
    /// where it goes, or the rule's right without it): she lets it go,
    /// it's back where it stood, and she glances at it there (`toward`).
    pub fn set_down_refused(&mut self, toward: (i32, i32), now: u64) {
        if self.episode.is_some_and(|e| e.set_down) {
            tracing::debug!("houseguest: the frame refused what she set down");
            self.drop_episode(Some(toward), now);
        }
    }

    /// She lets go of moving the piece: it's back where it stood (if she
    /// had lifted it) and she owes a glance at it there (`toward`, else
    /// where she is); or, still on her way to lift it, at where she was
    /// heading. Set down where it's right already (trying it in another
    /// spot), and not lifted again, she keeps it there, as pleased as if
    /// she'd chosen to ("There!").
    fn drop_episode(&mut self, toward: Option<(i32, i32)>, at: u64) {
        let Some(ep) = self.episode.take() else {
            return;
        };
        if self.heading.as_ref().is_some_and(|h| h.job.carry()) {
            self.drop_heading(Letting::Carry);
        }
        // Set down where it's right already, and not lifted again: it
        // stays there, as if she'd kept it there.
        if ep.tried > 0 && !ep.pocket {
            tracing::info!(
                piece = ?ep.repair.piece,
                "houseguest: leaves her {} where she tried it",
                ep.repair.piece.spec().name
            );
            self.say(THERE, at);
            return;
        }
        tracing::info!(
            piece = ?ep.repair.piece,
            lifted = ep.pocket,
            "houseguest: let go of moving her {}",
            ep.repair.piece.spec().name
        );
        // One go at each rule she felt: let go, it stays as it is.
        for felt in self.felt.iter_mut().filter(|f| f.key == ep.repair.key) {
            felt.let_go = true;
        }
        let toward = toward.unwrap_or((self.x, self.y));
        let loss = if ep.pocket {
            Loss::Moved(ep.repair.piece)
        } else {
            Loss::Heading
        };
        self.owe(loss, toward);
    }

    /// Moving a piece of her home comes before anything new: lifting the
    /// one she set off for, carrying it to where it goes, then (once the
    /// frame has taken it) sitting back down where she felt it was wrong.
    /// While the frame has yet to judge the step she's at (she has just
    /// lifted it, or set it down), she waits a moment. A move the frame
    /// no longer allows is made another way if there's one for the same
    /// piece, else she lets it go; so too once setting off for a step has
    /// come to nothing [`TRIES`] times (interruptions cost none). `None`
    /// when she's let go of it (she chooses anew).
    ///
    /// Trying it in a spot (one of a few as good as each other), once
    /// she has sat on it a moment she keeps it there with probability
    /// e^(−Δ/T): Δ how much dearer the spot is than the cheapest, in
    /// [`TIE_CELLS`], and T her restlessness (at least [`KEEP_T_MIN`]).
    /// Else she lifts it again, for the next spot she'd try that still
    /// puts the rule right; the last she keeps.
    fn arrange_next(
        &mut self,
        ctx: &Ctx,
        whims: Whims,
        here: usize,
        terrain: &Terrain,
        at: u64,
        rng: &mut Rng,
    ) -> Option<Decision> {
        if self.just_set.take().is_some() {
            if let Some(("arrange/use-it", Bind::Job(Job::Use(seat)))) =
                mind::bind(ctx, whims, Want::Arrange)
                && self.go_to(Want::Use(seat.what), Job::Use(seat), here, terrain, at)
            {
                self.credit = Some(Want::Use(seat.what));
                return Some(Decision::of(Bucket::Continuation, "arrange/use-it"));
            }
            // Nowhere to sit on it: trying it, she makes up her mind
            // now.
            if !self.episode.is_some_and(|e| e.trying) {
                return None;
            }
        }
        let mut ep = self.episode?;
        let wait = |osaka: &mut Self| {
            osaka.set(
                Act::Stand {
                    until: at + WAIT_MS,
                },
                at,
            );
            Some(Decision::reflex("waiting"))
        };
        if ep.trying {
            let delta =
                f64::from(ep.repair.cost.saturating_sub(ep.trials.min)) / f64::from(TIE_CELLS);
            let t = self.needs.get(Need::Restless).max(KEEP_T_MIN);
            let keep = whims.odds("keep", u64::from(ep.tried), (-delta / t).exp());
            let next = if keep {
                None
            } else {
                ep.trials.next(&ep.repair)
            };
            let Some(next) = next else {
                tracing::info!(
                    piece = ?ep.repair.piece,
                    repair = %ep.repair.label(),
                    "houseguest: keeps her {} where it is",
                    ep.repair.piece.spec().name
                );
                self.episode = None;
                self.say(THERE, at);
                return None;
            };
            tracing::info!(
                piece = ?ep.repair.piece,
                repair = %next.label(),
                "houseguest: tries her {} somewhere else",
                ep.repair.piece.spec().name
            );
            self.episode = Some(Episode {
                repair: next,
                trying: false,
                tries: 0,
                ..ep
            });
            #[cfg(test)]
            {
                self.retried += 1;
            }
            return wait(self);
        }
        let Some(judged) = ctx.chances.judged.filter(|j| j.of(&ep) && !ep.set_down) else {
            return wait(self);
        };
        if !judged.holds {
            // The next spot she means to try it in, else another way to
            // set it down, if the last frame had one. (Set down where
            // it's right already, the rule holds: no other way is
            // worked out, and it stays where she tried it.)
            let other = ep.trials.next(&ep.repair).or_else(|| {
                ctx.chances
                    .repairs
                    .iter()
                    .find(|r| {
                        ep.pocket
                            && (r.key, r.piece) == (ep.repair.key, ep.repair.piece)
                            && r.to != ep.repair.to
                    })
                    .copied()
            });
            if let Some(repair) = other {
                tracing::debug!(repair = %repair.label(), "houseguest: somewhere else for it");
                ep.repair = repair;
                self.episode = Some(ep);
                return wait(self);
            }
            self.drop_episode(judged.home, at);
            return None;
        }
        if ep.tries >= TRIES {
            self.drop_episode(judged.home, at);
            return None;
        }
        let bound = mind::bind(ctx, whims, Want::Arrange)
            .filter(|(_, bind)| matches!(bind, Bind::Job(job) if job.carry()));
        if let Some((method, bind)) = bound
            && self.plan(Want::Arrange, bind, here, terrain, at, rng)
        {
            self.credit = Some(Want::Arrange);
            return Some(Decision::of(Bucket::Continuation, method));
        }
        // She can't get to it just now: a try spent, and a while before
        // she sets off again.
        self.spend_try();
        self.set(
            Act::Stand {
                until: at + UNREACHED_MS,
            },
            at,
        );
        Some(Decision::reflex("can't get to it"))
    }

    /// The rules of her home she would put right, of those `broken`, in
    /// the order she'd go about them: those she felt that are broken
    /// still (and she hasn't let go of putting right), in the order she
    /// felt them, while her mood leaves her more to do about her home this
    /// visit; moving a piece, only the rule she's moving it for. The frame
    /// works out how she'd mend each in turn and keeps the first it finds
    /// a way for, so a rule no move mends doesn't keep her from the next.
    pub fn to_mend(&self, broken: &[super::rules::Broken]) -> Vec<Grievance> {
        if self.home_acts >= self.mood.home_acts() {
            return Vec::new();
        }
        let felt: Vec<Grievance> = self
            .felt
            .iter()
            .filter(|f| !f.let_go)
            .map(|f| f.key)
            .filter(|&key| broken.iter().any(|b| b.key == key))
            .collect();
        match self.episode {
            Some(ep) if felt.contains(&ep.repair.key) => vec![ep.repair.key],
            _ => felt,
        }
    }

    /// She's saying what's wrong with her home.
    fn grumbling(&self, now: u64) -> bool {
        matches!(self.act, Act::Use { grievance: Some((_, from)), .. }
            if (from..from + GRIEVANCE_MS).contains(&now))
    }

    /// A rule of her home she has felt is broken still (and she hasn't
    /// let it go).
    fn grieved(&self, chances: &Chances) -> bool {
        chances
            .broken
            .iter()
            .any(|b| self.felt.iter().any(|f| f.key == b.key && !f.let_go))
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
        // A piece she was moving is back where it stood, unremarked.
        if let Some(ep) = self.episode.take() {
            tracing::debug!(piece = ?ep.repair.piece, "houseguest: placed; the piece she was moving is put back");
        }
        self.just_set = None;
        self.watch_until = 0;
        self.hush(at);
        self.at_work = false;
        self.set(Act::Stand { until: at + 1000 }, at);
    }

    /// A chat message arrived: stop and look at it — unless where she is
    /// her image hides text (`terrain`, as last read), where she only
    /// passes: there the chat still interrupts what she was at, but she
    /// doesn't stop; she chooses at once, which takes her on to somewhere
    /// calm, and watches it from there. (Stopping for each line of a
    /// lively chat would keep her over text for as long as it went on.)
    /// A line that `asks` her something, arriving as a splice that
    /// answers plays (the andagi), gets its answer instead: she turns to
    /// the chat and says it, beaming, and plays on, the splice neither
    /// stopped nor cut short (and she watches the chat a while after it,
    /// as after any line).
    pub fn look(&mut self, now: u64, chat_x: i32, asks: bool, terrain: &Terrain) {
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
        if asks && let Some(answer) = self.answer(now) {
            tracing::info!(answer, "houseguest: answers the chat, playing on");
            self.say(answer, now);
            self.answering = self.use_span().map(|span| (span, now + speech_ms(answer)));
            return;
        }
        if terrain.restful(self.x, self.y) {
            self.interrupt(Cause::Chat, now);
        } else {
            tracing::trace!(
                x = self.x,
                y = self.y,
                "houseguest: chat over text; on somewhere calm"
            );
            self.interrupt(Cause::ChatPassing, now);
        }
    }

    /// What she'd answer a line asking her something at `now`: the
    /// answer of the splice playing then, if it has one.
    fn answer(&self, now: u64) -> Option<&'static str> {
        let Act::Use {
            since, until, play, ..
        } = self.act
        else {
            return None;
        };
        match play
            .spliced_at(since, until, now)?
            .splice
            .script()
            .on_chat()
        {
            Chat::Answer(answer) => Some(answer),
            Chat::Look => None,
        }
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
            Cause::ChatPassing => (0, 0),
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
            .map(|(text, _)| Bubble::Say(text))
            // What she says about her home isn't cut short.
            .filter(|_| !self.grumbling(now));
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
                play,
                grievance,
                ..
            } => {
                // Watching from a sofa, she sits on it.
                let host = if seat.item == Furniture::Sofa {
                    Pose::Lounge
                } else {
                    Pose::Sit
                };
                let (pose, face, bubble) = play
                    .key(since, until, now)
                    .map_or((host, Face::Vacant, None), |(key, elapsed)| {
                        key.look(elapsed, host, &play)
                    });
                // Answering the chat, she beams.
                let answering = self
                    .answering
                    .is_some_and(|(span, till)| span == (seat, since, until) && now < till);
                let face = if answering { Face::Happy } else { face };
                // Something isn't right: she cranes round at it, at what
                // she's doing, and says so.
                match grievance.and_then(|(g, from)| Some((g.rule()?.grievance, from))) {
                    Some((line, from)) if (from..from + GRIEVANCE_MS).contains(&now) => {
                        (pose, Face::Curious, Some(Bubble::Say(line)))
                    }
                    _ => (pose, face, bubble),
                }
            }
            // Bent to the piece, bobbing.
            Act::Lift { since, .. } | Act::SetDown { since, .. } => {
                let frame = (now.saturating_sub(since) / LIFT_FRAME_MS % 2) as u8;
                (Pose::ToeTouch(frame), Face::Happy, None)
            }
            Act::Tear { ripped, step, .. } => (
                Pose::Pull {
                    heaving: ripped && step % 2 == 1,
                    row: self.hands_row(),
                },
                if ripped { Face::Happy } else { Face::Curious },
                None,
            ),
            Act::SpaceOut {
                since,
                until,
                play: Some(play),
            } => play.key(since, until, now).map_or(
                (Pose::Stand, Face::Vacant, Some(Bubble::Dots)),
                |(key, elapsed)| key.look(elapsed, Pose::Stand, &play),
            ),
            Act::SpaceOut { play: None, .. } => (Pose::Stand, Face::Vacant, Some(Bubble::Dots)),
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

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    /// Musing, she tells a riddle about one time in three, drawn from
    /// her latest decision's whims, and never while she's saying
    /// something (it would hide the question): then she says a musing.
    /// A riddle's question is said as she starts, its answer as it
    /// shows; she says nothing over it. The body's only draw is how long
    /// she spaces out.
    /// A pooled line spoken over in the instant it's said (another line
    /// said, or she's put somewhere) never showed: it isn't said, so it
    /// doesn't cool. Spoken over any later, it showed, and is.
    #[test]
    fn a_line_spoken_over_as_it_is_said_was_never_said() {
        let line = mind::DOOR.lines[0];
        // Drawn (so recorded), then spoken over in the same instant
        // (another line, or put somewhere): it never showed, so it
        // isn't said and doesn't cool.
        for placed in [false, true] {
            let mut rng = Rng(1);
            let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
            let w = (0..)
                .map(Whims)
                .find(|&w| Lines::default().pick(mind::DOOR, w, 0) == Some(line))
                .unwrap();
            assert_eq!(osaka.lines.pick(mind::DOOR, w, 1000), Some(line));
            osaka.say(line, 1000);
            if placed {
                osaka.place(5, 5, 1000);
            } else {
                osaka.say(OK, 1000);
            }
            assert_eq!(osaka.lines.said(), [], "placed {placed}");
            assert_eq!(osaka.lines.pick(mind::DOOR, w, 1000), Some(line));
            // Spoken over a moment later: it showed, so it's said.
            osaka.say(line, 1000);
            if placed {
                osaka.place(5, 5, 1001);
            } else {
                osaka.say(OK, 1001);
            }
            assert_eq!(
                osaka.lines.said(),
                [(PoolId::Door, line, 1000)],
                "placed {placed}"
            );
        }
    }

    #[test]
    fn a_riddle_is_told_one_musing_in_three_and_only_when_quiet() {
        let mut riddles = 0;
        for seed in 0..120 {
            for talking in [false, true] {
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.whims = Whims(seed);
                if talking {
                    osaka.say(OK, 0);
                }
                let mut alone = rng.clone();
                osaka.muse(0, &mut rng);
                let span = alone.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1);
                assert_eq!(rng.0, alone.0, "seed {seed}: one body draw");
                let Act::SpaceOut { since, until, play } = osaka.act else {
                    panic!("seed {seed}: spacing out");
                };
                assert_eq!((since, until), (0, span), "seed {seed}");
                match play {
                    Some(play) => {
                        assert!(!talking, "seed {seed}: a riddle over speech");
                        riddles += 1;
                        let (question, answer) = RIDDLES[usize::from(play.drawn[0])];
                        assert_eq!(osaka.speech, None, "seed {seed}");
                        assert_eq!(
                            osaka.lines.said(),
                            [
                                (PoolId::Riddle, question, 0),
                                (PoolId::Riddle, answer, script::RIDDLE_ASKED_MS),
                            ],
                            "seed {seed}"
                        );
                        assert_eq!(osaka.act_due, script::RIDDLE_ASKED_MS, "seed {seed}");
                    }
                    None => {
                        let (said, _) = osaka.speech.unwrap();
                        assert!(mind::MUSINGS.lines.contains(&said), "seed {seed}: {said}");
                        assert_eq!(osaka.act_due, until, "seed {seed}");
                    }
                }
            }
        }
        assert!(
            (25..=55).contains(&riddles),
            "{riddles} riddles in 120 musings"
        );
    }

    /// Her, standing, with `need` pressing.
    fn pressed(need: Need) -> (Osaka, Rng) {
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        osaka.needs.serve(need, -1.0);
        (osaka, rng)
    }

    /// In a pretty room, resting (sitting, lying, gazing) and using her
    /// things ease her want of beauty; exercising (jumping jacks, touching
    /// her toes, stretching: what answers restlessness) and chores
    /// (unpacking a parcel, crumpling text) don't.
    #[test]
    fn only_rest_in_a_pretty_room_eases_her_want_of_beauty() {
        for what in Activity::ALL {
            let (mut osaka, _) = pressed(Need::Beauty);
            let before = osaka.needs.get(Need::Beauty);
            osaka.beauty_here = 1.0;
            osaka.credit = Some(Want::Idle(what));
            osaka.act = Act::Idle {
                what,
                since: 0,
                until: 1000,
            };
            osaka.credit_done(1000);
            let eased = osaka.needs.get(Need::Beauty) < before;
            let exercise = Want::Idle(what)
                .def()
                .serves
                .iter()
                .any(|&(need, _)| need == Need::Restless);
            assert_eq!(eased, !exercise, "{what:?}");
        }
        for what in [
            Use::Lounge,
            Use::Nap,
            Use::Sleep,
            Use::Homework,
            Use::Watch,
            Use::Unpack,
            Use::Read,
            Use::Snack,
            Use::Pet,
            Use::Crumple,
        ] {
            let (mut osaka, _) = pressed(Need::Beauty);
            let before = osaka.needs.get(Need::Beauty);
            osaka.beauty_here = 1.0;
            osaka.credit = Some(Want::Use(what));
            osaka.act = Act::Use {
                seat: Seat {
                    what,
                    item: Furniture::Sofa,
                    piece: PieceRef::Real(Furniture::Sofa),
                    x: 10,
                    y: 10,
                    facing: Facing::Left,
                },
                since: 0,
                until: 1000,
                whole: 1000,
                play: Play::plain(what.script()),
                grievance: None,
            };
            osaka.credit_done(1000);
            let eased = osaka.needs.get(Need::Beauty) < before;
            let chore = matches!(what, Use::Unpack | Use::Crumple);
            assert_eq!(eased, !chore, "{what:?}");
        }
    }

    /// Her, trying the sofa where she's set it down (sitting on it a
    /// moment, to make up her mind).
    fn trial_episode() -> Episode {
        use super::super::room::Strip;
        let at = super::super::room::Shown {
            item: Furniture::Sofa,
            facing: Facing::Left,
            boxed: false,
            strip: Some(Strip::Bottom(super::super::room::Nook::Users)),
            left: 6,
            floor: 10,
            scrap: None,
        };
        Episode {
            repair: Repair {
                key: Grievance {
                    row: 0,
                    piece: Furniture::Sofa,
                },
                piece: Furniture::Sofa,
                to: Placement {
                    strip: Strip::Bottom(super::super::room::Nook::Users),
                    anchor: super::super::room::Anchor {
                        side: super::super::room::Side::Left,
                        offset: 5,
                    },
                    facing: Facing::Left,
                },
                at,
                cost: 1,
                tier: 0,
            },
            pocket: false,
            set_down: false,
            tries: 0,
            trials: Trials::default(),
            tried: 1,
            trying: true,
        }
    }

    /// Sitting on a piece a moment to try it where she's set it down
    /// eases her by its share of a whole use of it, not as a whole use.
    #[test]
    fn a_trial_sit_eases_her_by_its_share_of_a_use() {
        let eased = |trying: bool| {
            let (mut osaka, mut rng) = pressed(Need::Comfort);
            let before = osaka.needs.get(Need::Comfort);
            if trying {
                osaka.episode = Some(trial_episode());
            }
            let seat = Seat {
                what: Use::Lounge,
                item: Furniture::Sofa,
                piece: PieceRef::Real(Furniture::Sofa),
                x: 10,
                y: 10,
                facing: Facing::Left,
            };
            osaka.credit = Some(Want::Use(Use::Lounge));
            osaka.start_job(Job::Use(seat), 0, &Chances::default(), &mut rng);
            let Act::Use { until, .. } = osaka.act else {
                panic!("{:?}", osaka.act);
            };
            osaka.credit_done(until);
            before - osaka.needs.get(Need::Comfort)
        };
        let (whole, trial) = (eased(false), eased(true));
        assert!(whole > 0.1, "{whole}");
        // A trial sit is at most 5 s; a lounge at least 15.
        assert!(trial > 0.0 && trial <= whole / 3.0, "{trial} of {whole}");
    }

    /// Every look a use switches between holds a half-open span,
    /// `[start, end)`: it is on at its first ms and still on at its last,
    /// and the next look takes over exactly at its share (Crumple's
    /// "There!" at ⅘ and Unpack's `Ooh` at ⅗ included), as played: her
    /// pose (bobbing on its frames), face and bubble, and what's on her
    /// furniture (the TV's frames, the lamp, the fridge, the cat), on a
    /// sofa or not, for every use.
    #[test]
    fn every_use_look_span_is_half_open() {
        use super::super::art::Channel;
        /// Her bob's frame, `t` ms in.
        fn bob(t: u64) -> u8 {
            (t / USE_FRAME_MS % 2) as u8
        }
        /// The TV's frame, `t` ms in.
        fn tv(t: u64) -> u8 {
            (t / CHANNEL_FRAME_MS % 2) as u8
        }
        /// Her pose and what's on her furniture, `t` ms in.
        type Body = fn(u64) -> (Pose, Option<Prop>);
        /// A span's start, her face and bubble, and her body.
        type Span = (u64, Face, Option<Bubble>, Body);
        /// A use, on a sofa or not, its advert, and its spans.
        type Case = (Use, bool, Option<Furniture>, Vec<Span>);
        let pitch = Furniture::Lamp.spec().pitch;
        let snow: Body = |t| (Pose::Sit, Some(Prop::Tv(Channel::Snow(tv(t)))));
        let snow_on_sofa: Body = |t| (Pose::Lounge, Some(Prop::Tv(Channel::Snow(tv(t)))));
        let selling: Body = |t| (Pose::Sit, Some(Prop::Tv(Channel::Shopping(tv(t)))));
        let selling_on_sofa: Body = |t| (Pose::Lounge, Some(Prop::Tv(Channel::Shopping(tv(t)))));
        let sold = |body: Body, length: u64| {
            vec![
                (0, Face::Curious, Some(Bubble::Ooh), body),
                (length * 2 / 5, Face::Happy, Some(Bubble::Say(pitch)), body),
                (length * 3 / 5, Face::Curious, None, body),
            ]
        };
        for length in [7, 5000, 5250, 12_345] {
            let cases: Vec<Case> = vec![
                (
                    Use::Lounge,
                    false,
                    None,
                    vec![(0, Face::Vacant, None, |_| (Pose::Lounge, None))],
                ),
                (
                    Use::Lounge,
                    true,
                    None,
                    vec![(0, Face::Vacant, None, |_| (Pose::Lounge, None))],
                ),
                (
                    Use::Nap,
                    true,
                    None,
                    vec![(0, Face::Blink, Some(Bubble::Zzz), |t| {
                        (Pose::Nap(bob(t)), None)
                    })],
                ),
                (
                    Use::Sleep,
                    false,
                    None,
                    vec![
                        (0, Face::Blink, Some(Bubble::Dots), |t| {
                            (Pose::Sleep(bob(t)), None)
                        }),
                        (script::LAMP_ON_MS, Face::Blink, Some(Bubble::Zzz), |t| {
                            (Pose::Sleep(bob(t)), Some(Prop::LampOff))
                        }),
                    ],
                ),
                (
                    Use::Homework,
                    false,
                    None,
                    vec![
                        (0, Face::Vacant, None, |t| (Pose::Homework(bob(t)), None)),
                        (length / 2, Face::Blink, Some(Bubble::Dots), |_| {
                            (Pose::Homework(2), None)
                        }),
                        (length * 3 / 4, Face::Blink, Some(Bubble::Zzz), |_| {
                            (Pose::Homework(3), None)
                        }),
                    ],
                ),
                (
                    Use::Watch,
                    false,
                    None,
                    vec![(0, Face::Curious, None, snow)],
                ),
                (
                    Use::Watch,
                    true,
                    None,
                    vec![(0, Face::Curious, None, snow_on_sofa)],
                ),
                (
                    Use::Watch,
                    false,
                    Some(Furniture::Lamp),
                    sold(selling, length),
                ),
                (
                    Use::Watch,
                    true,
                    Some(Furniture::Lamp),
                    sold(selling_on_sofa, length),
                ),
                (
                    Use::Read,
                    false,
                    None,
                    vec![(0, Face::Vacant, None, |t| (Pose::Read(bob(t)), None))],
                ),
                (
                    Use::Snack,
                    false,
                    None,
                    vec![
                        (0, Face::Curious, None, |_| {
                            (Pose::Side, Some(Prop::FridgeOpen))
                        }),
                        (1500, Face::Happy, None, |t| (Pose::Eat(bob(t)), None)),
                    ],
                ),
                (
                    Use::Pet,
                    false,
                    None,
                    vec![
                        (0, Face::Happy, Some(Bubble::Hum), |_| (Pose::Pet(0), None)),
                        (
                            length * 7 / 10,
                            Face::Surprised,
                            Some(Bubble::Say("Ow!")),
                            |_| (Pose::Pet(1), Some(Prop::CatBiting)),
                        ),
                    ],
                ),
                (
                    Use::Crumple,
                    false,
                    None,
                    vec![
                        (0, Face::Happy, Some(Bubble::Say(SCRUNCH)), |t| {
                            (Pose::ToeTouch(bob(t)), None)
                        }),
                        (length * 4 / 5, Face::Happy, Some(Bubble::Say(THERE)), |t| {
                            (Pose::ToeTouch(bob(t)), None)
                        }),
                    ],
                ),
                (
                    Use::Unpack,
                    false,
                    None,
                    vec![
                        (0, Face::Happy, None, |t| (Pose::ToeTouch(bob(t)), None)),
                        (length * 3 / 5, Face::Happy, Some(Bubble::Ooh), |t| {
                            (Pose::ToeTouch(bob(t)), None)
                        }),
                    ],
                ),
            ];
            // Every use, and every use a sofa changes, has its case.
            for u in Use::ALL {
                assert!(cases.iter().any(|c| c.0 == u), "{u:?}");
            }
            for (what, sofa, advert, spans) in cases {
                let starts: Vec<u64> = spans.iter().map(|&(start, ..)| start).collect();
                // A use too short for every span to have a ms of its own
                // (a snack shorter than the fridge) has nothing to check.
                if !starts.windows(2).all(|w| w[0] < w[1]) || starts.last() >= Some(&length) {
                    continue;
                }
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
                let mut look = |elapsed| {
                    played(
                        &mut osaka,
                        Played {
                            what,
                            sofa,
                            bought: advert,
                            grievance: None,
                            length,
                        },
                        elapsed,
                    )
                };
                for (i, &(start, face, bubble, body)) in spans.iter().enumerate() {
                    let end = spans.get(i + 1).map_or(length, |&(next, ..)| next);
                    for elapsed in [start, (start + end) / 2, end - 1] {
                        let (pose, prop) = body(elapsed);
                        assert_eq!(
                            look(elapsed),
                            ((pose, face, bubble), prop),
                            "{what:?} sofa {sofa} advert {advert:?}: {elapsed}/{length}"
                        );
                    }
                }
            }
        }
    }

    /// When the uses these tests play start: not 0, so a look timed from
    /// the wrong start shows.
    const SINCE: u64 = 10_000;

    /// A use to play: of `what`, on a sofa or not, with what the shopping
    /// channel `bought` her, a grievance she starts saying how far in,
    /// lasting `length` ms.
    #[derive(Clone, Copy)]
    struct Played {
        what: Use,
        sofa: bool,
        bought: Option<Furniture>,
        grievance: Option<u64>,
        length: u64,
    }

    /// The grievance these tests have her say.
    const FELT: Grievance = Grievance {
        row: 5,
        piece: Furniture::Tv,
    };

    /// How she looks, and what's on her furniture, `elapsed` ms into
    /// `use_` (begun at [`SINCE`]), as `start_job` would set it up.
    fn played(osaka: &mut Osaka, use_: Played, elapsed: u64) -> (script::Look, Option<Prop>) {
        played_as(osaka, use_, Play::of(use_.what, use_.bought), elapsed)
    }

    /// [`played`], playing `play` (`use_.length` long in all, any
    /// prelude and coda included).
    fn played_as(
        osaka: &mut Osaka,
        use_: Played,
        play: Play,
        elapsed: u64,
    ) -> (script::Look, Option<Prop>) {
        let item = if use_.sofa {
            Furniture::Sofa
        } else {
            Furniture::Tv
        };
        osaka.act = Act::Use {
            seat: Seat {
                what: use_.what,
                item,
                piece: PieceRef::Real(item),
                x: 10,
                y: 10,
                facing: Facing::Left,
            },
            since: SINCE,
            until: SINCE + use_.length,
            // The body, as `start_job` has it.
            whole: play.body_end(SINCE, SINCE + use_.length) - play.body_start(SINCE),
            play,
            grievance: use_.grievance.map(|from| (FELT, SINCE + from)),
        };
        let now = SINCE + elapsed;
        (osaka.acting(now), osaka.prop(now))
    }

    /// A prelude shifts the body whole: what's on TV `t` into the body
    /// (its frame too) is the same with one or without, whatever the
    /// prelude's length.
    #[test]
    fn a_prelude_never_shifts_what_is_on_tv() {
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
        let mut on_tv = |play: Play, length: u64, t: u64| {
            played(
                &mut osaka,
                Played {
                    what: Use::Watch,
                    sofa: false,
                    bought: play.bought,
                    grievance: None,
                    length,
                },
                0,
            );
            let Act::Use { play: p, .. } = &mut osaka.act else {
                panic!("using");
            };
            *p = play;
            osaka.prop(play.body_start(SINCE) + t)
        };
        for bought in [None, Some(Furniture::Lamp)] {
            let plain = Play::of(Use::Watch, bought);
            for len in [1, 399, 400, 401, 1234] {
                let before = script::Spliced {
                    splice: script::SpliceId::TestSnack,
                    len,
                    branch: 0,
                };
                let wrapped = Play {
                    before: Some(before),
                    ..plain
                };
                for t in (0..6000).step_by(97) {
                    assert_eq!(
                        on_tv(wrapped, 6000 + len, t),
                        on_tv(plain, 6000, t),
                        "bought {bought:?} prelude {len}: {t}"
                    );
                }
            }
        }
    }

    /// What's wrong with her home overlays whatever key is playing, for
    /// [`GRIEVANCE_MS`] from when she starts saying it: her pose and
    /// what's on her furniture stay the key's, her face is Curious and
    /// she says the rule's line; before and after, the key's own look.
    #[test]
    fn a_grievance_overlays_the_playing_key() {
        let line = FELT.rule().map(|r| r.grievance).unwrap();
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
        let length = 20_000;
        for what in Use::ALL {
            let boughts: &[Option<Furniture>] = if what == Use::Watch {
                &[None, Some(Furniture::Lamp)]
            } else {
                &[None]
            };
            for (&bought, sofa) in boughts.iter().flat_map(|b| [(b, false), (b, true)]) {
                for from in [USE_FRAME_MS, 3 * USE_FRAME_MS, 11 * USE_FRAME_MS] {
                    let plain = Played {
                        what,
                        sofa,
                        bought,
                        grievance: None,
                        length,
                    };
                    let felt = Played {
                        grievance: Some(from),
                        ..plain
                    };
                    let edges = [from - 1, from, from + GRIEVANCE_MS - 1, from + GRIEVANCE_MS];
                    for elapsed in (0..length).step_by(97).chain(edges) {
                        let (look, prop) = played(&mut osaka, plain, elapsed);
                        let over = (from..from + GRIEVANCE_MS).contains(&elapsed);
                        let expected = if over {
                            (look.0, Face::Curious, Some(Bubble::Say(line)))
                        } else {
                            look
                        };
                        assert_eq!(
                            played(&mut osaka, felt, elapsed),
                            (expected, prop),
                            "{what:?} bought {bought:?} sofa {sofa} from {from}: {elapsed}"
                        );
                    }
                }
            }
        }
    }

    /// A use wrapped in a prelude and a coda is credited by the share of
    /// its body done: nothing at all if she's interrupted in the prelude
    /// (before the body starts), the whole of it if in the coda (the body
    /// done), and the share of the body between. Every use, prelude,
    /// coda or both.
    #[test]
    fn a_splice_is_never_credited() {
        let before = script::Spliced {
            splice: script::SpliceId::TestSnack,
            len: 4000,
            branch: 0,
        };
        let after = script::Spliced {
            splice: script::SpliceId::TestSleep,
            len: 5000,
            branch: 0,
        };
        let body = 20_000;
        for what in Use::ALL {
            for (before, after) in [
                (Some(before), None),
                (None, Some(after)),
                (Some(before), Some(after)),
            ] {
                let play = Play {
                    before,
                    after,
                    ..Play::of(what, None)
                };
                let start = play.body_start(SINCE);
                let length = body + before.map_or(0, |s| s.len) + after.map_or(0, |s| s.len);
                let end = play.body_end(SINCE, SINCE + length);
                assert_eq!(end - start, body);
                let mut times = vec![start + body / 4, start + body / 2];
                if before.is_some() {
                    times.extend([SINCE, SINCE + 1, start - 1]);
                }
                if after.is_some() {
                    times.extend([end, end + 1, SINCE + length - 1]);
                }
                for t in times {
                    let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
                    played_as(
                        &mut osaka,
                        Played {
                            what,
                            sofa: false,
                            bought: None,
                            grievance: None,
                            length,
                        },
                        play,
                        0,
                    );
                    osaka.credit = Some(Want::Use(what));
                    osaka.interrupt(Cause::Chat, t);
                    let want = if t < start {
                        0.0
                    } else if t >= end {
                        1.0
                    } else {
                        (t - start) as f64 / body as f64
                    };
                    assert_eq!(
                        osaka.credited,
                        [(Want::Use(what), want, t)],
                        "{what:?} before {before:?} after {after:?} at {t}"
                    );
                }
            }
        }
    }

    /// A seat for `what` on `item`, as the frame would offer it.
    fn seat_for(what: Use, item: Furniture) -> Seat {
        Seat {
            what,
            item,
            piece: PieceRef::Real(item),
            x: 10,
            y: 10,
            facing: Facing::Left,
        }
    }

    /// A broken rule of her home she'd feel using `item` for `what`, if
    /// there's one to feel there.
    fn broken_for(what: Use, item: Furniture) -> Option<super::super::rules::Broken> {
        use super::super::rules::{Broken, RULES};
        let row = RULES.iter().position(|r| r.felt_on.contains(&what))?;
        Some(Broken {
            row,
            pieces: vec![item],
            involved: vec![item],
            key: Grievance { row, piece: item },
        })
    }

    /// Her, starting a use of `seat` at `at` with `chances` (with `cue`
    /// cued, saying something till `talking` if set), from `rng`'s state:
    /// her after, and the stream's state after. No splice row is rolled:
    /// only what's cued wraps it.
    fn started(
        seat: Seat,
        at: u64,
        chances: &Chances,
        cue: Option<Cue>,
        talking: Option<u64>,
        seed: u64,
    ) -> (Osaka, u64) {
        let mut rng = Rng(seed);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        osaka.splice_rows = &[];
        osaka.whims = Whims(seed ^ 0x5eed);
        if let Some(until) = talking {
            osaka.speech = Some((OK, until));
        }
        osaka.credit = Some(Want::Use(seat.what));
        osaka.cue(cue);
        osaka.start_job(Job::Use(seat), at, chances, &mut rng);
        (osaka, rng.0)
    }

    /// The use she's started: since and until when, how much of a whole
    /// use it is, and what it plays.
    fn begun(osaka: &Osaka) -> (u64, u64, u64, Play) {
        let Act::Use {
            since,
            until,
            whole,
            play,
            ..
        } = osaka.act
        else {
            panic!("using: {:?}", osaka.act);
        };
        (since, until, whole, play)
    }

    /// Her wakeups from `from` until `to` (each strictly after the last),
    /// as offsets from `from`.
    fn schedule(osaka: &Osaka, from: u64, to: u64) -> Vec<u64> {
        let mut woke = Vec::new();
        let mut now = from;
        while now < to {
            let due = osaka.first_due(now).min(to);
            assert!(due > now, "due {due} at {now}");
            woke.push(due - from);
            now = due;
        }
        woke
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(
            dessplay_core::test_support::proptest_cases(64)
        ))]

        /// Splices never change what they wrap: a use started with a
        /// splice forced on (a prelude or a coda) and the same use started
        /// without, from the same state, leave her body's stream in the
        /// same state; are as much of a whole use and have bodies as
        /// long; buy the same and record the same; look the same and show
        /// the same on her furniture, wake her the same and credit her the
        /// same share, through the body, timed from its start. Every use a
        /// splice wraps, on a sofa or not, with a grievance or not, quiet
        /// or (before a coda) talking, with the shopping channel on or not;
        /// the tests' splices, and the rows.
        #[test]
        fn a_splice_never_changes_what_it_wraps(
            seed in proptest::prelude::any::<u64>(),
            row in proptest::prelude::any::<bool>(),
            what in 0usize..8,
            sofa in proptest::prelude::any::<bool>(),
            grieve in proptest::prelude::any::<bool>(),
            talking in proptest::option::of(0u64..6000),
            advert in proptest::prelude::any::<bool>(),
            coda in proptest::prelude::any::<bool>(),
            at in 1000u64..100_000,
        ) {
            let splice = match (coda, row) {
                (true, false) => SpliceId::TestSleep,
                (false, false) => SpliceId::TestSnack,
                (true, true) => SpliceId::Andagi,
                (false, true) => SpliceId::Chopsticks,
            };
            let around = splice.row().around;
            let what = around[what % around.len()];
            let item = if sofa { Furniture::Sofa } else { Furniture::Tv };
            let seat = seat_for(what, item);
            let chances = Chances {
                advert: advert.then_some(Furniture::Lamp),
                broken: broken_for(what, item).filter(|_| grieve).into_iter().collect(),
                ..Chances::default()
            };
            // (Talking as a cued prelude starts, she's hushed for it, and
            // so quiet as the body starts: what she says about her home
            // comes on its first frame, not the first she's quiet on.)
            let talking = talking.filter(|_| coda).map(|t| at + t);
            let (mut plain, plain_rng) = started(seat, at, &chances, None, talking, seed);
            let (mut wrapped, wrapped_rng) =
                started(seat, at, &chances, Some(Cue::Splice(splice, None)), talking, seed);
            proptest::prop_assert_eq!(plain_rng, wrapped_rng, "the body's stream");
            let (p_since, p_until, p_whole, p_play) = begun(&plain);
            let (w_since, w_until, w_whole, w_play) = begun(&wrapped);
            proptest::prop_assert!(p_play.before.is_none() && p_play.after.is_none());
            let spliced = if coda { w_play.after } else { w_play.before };
            proptest::prop_assert_eq!(spliced.map(|s| s.splice), Some(splice));
            proptest::prop_assert_eq!(p_since, w_since);
            proptest::prop_assert_eq!(p_whole, w_whole);
            let (p_start, p_end) = (p_play.body_start(p_since), p_play.body_end(p_since, p_until));
            let (w_start, w_end) = (w_play.body_start(w_since), w_play.body_end(w_since, w_until));
            let body = p_end - p_start;
            proptest::prop_assert_eq!(body, w_end - w_start);
            proptest::prop_assert_eq!(
                Play { before: None, after: None, ..w_play },
                p_play,
                "what it plays"
            );
            let events = |osaka: &mut Osaka| {
                let mut e: Vec<String> =
                    osaka.take_events().iter().map(|e| format!("{e:?}")).collect();
                e.sort();
                e
            };
            proptest::prop_assert_eq!(events(&mut plain), events(&mut wrapped));
            let grievance = |osaka: &Osaka, start: u64| match osaka.act {
                Act::Use { grievance, .. } => grievance.map(|(g, from)| (g, from - start)),
                _ => None,
            };
            proptest::prop_assert_eq!(grievance(&plain, p_start), grievance(&wrapped, w_start));
            // Wakeups through the body, from its start (its end the
            // coda's start, or the use's).
            let woke = schedule(&plain, p_start, p_end);
            proptest::prop_assert_eq!(&woke, &schedule(&wrapped, w_start, w_end));
            let times: Vec<u64> = (0..body)
                .step_by(97)
                .chain(woke.iter().flat_map(|&w| [w.saturating_sub(1), w]))
                .filter(|&t| t < body)
                .collect();
            for t in times {
                proptest::prop_assert_eq!(
                    plain.acting(p_start + t),
                    wrapped.acting(w_start + t),
                    "{} into the body",
                    t
                );
                proptest::prop_assert_eq!(
                    plain.prop(p_start + t),
                    wrapped.prop(w_start + t),
                    "{} into the body",
                    t
                );
            }
            for t in [0, 1, body / 3, body / 2, body - 1] {
                let credit = |osaka: &Osaka, at: u64| {
                    let mut osaka = osaka.clone();
                    osaka.credit_done(at);
                    osaka.credited.last().map(|&(want, share, _)| (want, share))
                };
                proptest::prop_assert_eq!(
                    credit(&plain, p_start + t),
                    credit(&wrapped, w_start + t),
                    "credited {} into the body",
                    t
                );
            }
        }
    }

    /// A trial sit is never wrapped in a splice, cued or not: a moment on
    /// a piece she's trying where it stands is all it is.
    #[test]
    fn a_trial_sit_is_never_spliced() {
        for splice in [SpliceId::TestSnack, SpliceId::TestSleep]
            .into_iter()
            .chain(SpliceId::ALL)
        {
            for &what in splice.row().around {
                let mut rng = Rng(3);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.episode = Some(trial_episode());
                osaka.cue(Some(Cue::Splice(splice, None)));
                osaka.start_job(
                    Job::Use(seat_for(what, Furniture::Sofa)),
                    1000,
                    &Chances::default(),
                    &mut rng,
                );
                let (.., play) = begun(&osaka);
                assert_eq!(
                    (play.before, play.after),
                    (None, None),
                    "{splice:?} {what:?}"
                );
            }
        }
    }

    /// Cued, a splice wraps only what it may: never unpacking or
    /// crumpling (what comes of those happens as the use ends).
    #[test]
    fn a_cued_splice_wraps_only_what_it_may() {
        for splice in [SpliceId::TestSnack, SpliceId::TestSleep]
            .into_iter()
            .chain(SpliceId::ALL)
        {
            for what in Use::ALL {
                let (osaka, _) = started(
                    seat_for(what, Furniture::Sofa),
                    1000,
                    &Chances::default(),
                    Some(Cue::Splice(splice, None)),
                    None,
                    9,
                );
                let (.., play) = begun(&osaka);
                let wrapped = play.before.or(play.after).map(|s| s.splice);
                let may = splice.row().around.contains(&what);
                assert_eq!(wrapped, may.then_some(splice), "{splice:?} {what:?}");
                assert!(!may || !matches!(what, Use::Unpack | Use::Crumple));
            }
        }
    }

    /// Asked something as the andagi plays after her snack, she turns
    /// to the chat and answers "Sata andagi.", beaming while she says it,
    /// and plays on: the use neither stopped nor cut short, the coda's
    /// own look back once she's said it. Told anything else then, or
    /// asked anything in the snack itself, she stops and looks. Either
    /// way she watches the chat a while, and mischief she owes goes back
    /// at once.
    #[test]
    fn asked_as_the_andagi_plays_she_answers_and_plays_on() {
        use super::super::scenes::LayerOp;
        use super::super::script::{ANDAGI_FOUND_MS, SATA_ANDAGI};
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        let terrain = Terrain::read(&Buffer::empty(Rect::new(0, 0, 40, 20)), &[], false);
        let seat = seat_for(Use::Snack, Furniture::Fridge);
        for asks in [false, true] {
            for in_coda in [false, true] {
                let case = format!("asks {asks}, in the coda {in_coda}");
                let (mut osaka, _) = started(
                    seat,
                    1000,
                    &Chances::default(),
                    Some(Cue::Splice(SpliceId::Andagi, Some(0))),
                    None,
                    5,
                );
                let owed = LayerOp::Pull {
                    row: 3,
                    cells: vec![4],
                    offset: 1,
                };
                osaka.pending.push((900_000, owed));
                let (_, since, until) = osaka.use_span().unwrap();
                let play = begun(&osaka).3;
                let end = play.body_end(since, until);
                assert!(end < until, "a coda");
                // In the coda: the first "Sata andagi." (blank), half
                // a second in.
                let now = if in_coda {
                    end + ANDAGI_FOUND_MS + 500
                } else {
                    since + 500
                };
                assert_eq!(osaka.facing, Facing::Left, "{case}");
                osaka.look(now, 30, asks, &terrain);
                assert_eq!(osaka.facing, Facing::Right, "{case}: turned to the chat");
                assert_eq!(osaka.watch_until, now + WATCH_MS, "{case}");
                assert!(
                    osaka.pending.iter().all(|&(due, _)| due == now),
                    "{case}: the mischief goes back"
                );
                if !(asks && in_coda) {
                    assert!(!matches!(osaka.act, Act::Use { .. }), "{case}: stopped");
                    continue;
                }
                assert_eq!(osaka.use_span(), Some((seat, since, until)), "{case}");
                let said = now + speech_ms(SATA_ANDAGI);
                assert_eq!(
                    osaka.appearance(now),
                    (
                        Pose::EatAndagi(0),
                        Face::Happy,
                        Some(Bubble::Say(SATA_ANDAGI))
                    ),
                    "{case}"
                );
                assert_eq!(osaka.appearance(said - 1).1, Face::Happy, "{case}");
                // Said: the coda's own look again (its quiet beat).
                assert_eq!(
                    osaka.appearance(said),
                    (Pose::EatAndagi(0), Face::Vacant, None),
                    "{case}"
                );
                assert!(
                    matches!(osaka.appearance(until - 1).0, Pose::EatAndagi(_)),
                    "{case}: eating it at the end"
                );
                // Asked again as she finishes it, then on to her
                // homework while she'd still be saying so: the homework
                // has its own face, the beaming left with the andagi.
                let late = until - 500;
                osaka.look(late, 30, true, &terrain);
                assert_eq!(osaka.use_span(), Some((seat, since, until)), "{case}");
                assert!(late + speech_ms(SATA_ANDAGI) > until + 100);
                let homework = seat_for(Use::Homework, Furniture::Desk);
                osaka.start_job(Job::Use(homework), until, &Chances::default(), &mut Rng(1));
                let (.., play) = begun(&osaka);
                assert_eq!((play.own, play.before), (ScriptId::Homework, None));
                assert_eq!(osaka.appearance(until + 100).1, Face::Vacant, "{case}");
            }
        }
    }

    /// Cued on a branch, a splice plays that branch, at its length,
    /// whatever her whims would have drawn.
    #[test]
    fn a_cued_branch_is_the_branch_played() {
        for splice in SpliceId::ALL {
            let row = splice.row();
            let what = row.around[0];
            for (branch, &len) in row.lens.iter().enumerate() {
                let branch = u8::try_from(branch).unwrap();
                for seed in 0..16 {
                    let (osaka, _) = started(
                        seat_for(what, Furniture::Sofa),
                        1000,
                        &Chances::default(),
                        Some(Cue::Splice(splice, Some(branch))),
                        None,
                        seed,
                    );
                    let (.., play) = begun(&osaka);
                    let spliced = play.before.or(play.after).unwrap();
                    assert_eq!(
                        (spliced.splice, spliced.branch, spliced.len),
                        (splice, branch, len),
                        "seed {seed}"
                    );
                }
            }
        }
    }

    /// A cue waits for a use it plays on: a splice, through a use it
    /// can't wrap and a trial sit, then round the next it can; surfing,
    /// through a use of another piece, a watch with her home on her mind
    /// (which would show over the channels) and a trial, then the next
    /// watch; a plain watch, through the others, then the next watch,
    /// her home on her mind or not. Each is taken by the use it plays
    /// on, and no other; a cued surf cools as a rolled one does.
    #[test]
    fn a_cue_waits_for_a_use_it_plays_on() {
        let mut rng = Rng(5);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        let plain = Chances::default();
        let broken = Chances {
            broken: broken_for(Use::Watch, Furniture::Tv).into_iter().collect(),
            ..Chances::default()
        };
        let mut at = 1000;
        let mut start = |osaka: &mut Osaka, what: Use, trying: bool, chances: &Chances| {
            osaka.episode = trying.then(trial_episode);
            at += 60_000;
            osaka.start_job(
                Job::Use(seat_for(what, Furniture::Tv)),
                at,
                chances,
                &mut rng,
            );
            begun(osaka).3
        };
        let splice = Cue::Splice(SpliceId::TestSnack, None);
        osaka.cue(Some(splice));
        for (what, trying) in [(Use::Unpack, false), (Use::Homework, true)] {
            let play = start(&mut osaka, what, trying, &plain);
            assert_eq!((play.before, play.after), (None, None), "{what:?}");
            assert_eq!(osaka.cued, Some(splice), "kept through {what:?}");
        }
        let play = start(&mut osaka, Use::Homework, false, &plain);
        assert_eq!(play.before.map(|s| s.splice), Some(SpliceId::TestSnack));
        assert_eq!(osaka.cued, None, "taken");
        for cue in [ScriptId::Surf, ScriptId::Watch] {
            osaka.cue(Some(Cue::Script(cue)));
            start(&mut osaka, Use::Lounge, false, &plain);
            assert_eq!(
                osaka.cued,
                Some(Cue::Script(cue)),
                "{cue:?}: kept through a lounge"
            );
            // A plain watch plays with her home on her mind; surfing
            // waits for the next.
            let play = start(&mut osaka, Use::Watch, false, &broken);
            if cue == ScriptId::Surf {
                assert_eq!(play.own, ScriptId::Watch);
                assert_eq!(
                    osaka.cued,
                    Some(Cue::Script(cue)),
                    "surf: kept through a grievance"
                );
            } else {
                assert_eq!(osaka.cued, None, "watch: taken");
                osaka.cue(Some(Cue::Script(cue)));
            }
            start(&mut osaka, Use::Watch, true, &plain);
            assert_eq!(
                osaka.cued,
                Some(Cue::Script(cue)),
                "{cue:?}: kept through a trial"
            );
            let play = start(&mut osaka, Use::Watch, false, &plain);
            assert_eq!(play.own, cue, "{cue:?}");
            assert_eq!(osaka.cued, None, "{cue:?}: taken");
        }
        // A cued surf cools as a rolled one does: not again on its own
        // within ten minutes, whatever the whims.
        let surfing = (0..)
            .find(|&seed| {
                let seat = seat_for(Use::Watch, Furniture::Tv);
                begun(&started(seat, 1000, &plain, None, None, seed).0)
                    .3
                    .own
                    == ScriptId::Surf
            })
            .unwrap_or_default();
        osaka.cue(Some(Cue::Script(ScriptId::Surf)));
        assert_eq!(
            start(&mut osaka, Use::Watch, false, &plain).own,
            ScriptId::Surf
        );
        osaka.whims = Whims(surfing ^ 0x5eed);
        // A minute on.
        assert_eq!(
            start(&mut osaka, Use::Watch, false, &plain).own,
            ScriptId::Watch
        );
    }

    /// Cued, a prelude plays though she's talking: what she was saying
    /// stops for it, so its first key shows (the lamp on a moment,
    /// thoughtful), and she looks as she would have, quiet. Rolled, a
    /// prelude waits for her to be quiet, and she goes on talking.
    #[test]
    fn a_cued_prelude_hushes_her() {
        let seat = seat_for(Use::Homework, Furniture::Sofa);
        let cue = Some(Cue::Splice(SpliceId::TestBedtime, None));
        let chances = Chances::default();
        for seed in 0..16 {
            let (quiet, _) = started(seat, 1000, &chances, cue, None, seed);
            let (talking, _) = started(seat, 1000, &chances, cue, Some(5000), seed);
            assert_eq!(talking.speech, None, "seed {seed}");
            // Settling for bed with the lamp on a moment, or off at once.
            let first = match begun(&talking).3.before.map(|s| s.branch) {
                Some(0) => Bubble::Dots,
                Some(_) => Bubble::Zzz,
                None => panic!("seed {seed}: no prelude"),
            };
            assert_eq!(talking.appearance(1000).2, Some(first), "seed {seed}");
            for t in [1000, 2000, 4999, 5000, 9000] {
                assert_eq!(
                    talking.appearance(t),
                    quiet.appearance(t),
                    "seed {seed} at {t}"
                );
            }
        }
    }

    /// Rolled from her rows as a use starts: a prelude only when she's
    /// quiet (talking, she goes on and the use plays bare in front), a
    /// coda by its chance, and neither round a trial sit.
    #[test]
    fn a_use_rolls_its_splices_as_it_starts() {
        let seat = seat_for(Use::Homework, Furniture::Sofa);
        let mut codas = 0;
        for seed in 0..64 {
            let start = |talking: bool, trying: bool| {
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.splice_rows = &[SpliceId::TestSnack, SpliceId::TestSleep];
                osaka.whims = Whims(seed ^ 0x5eed);
                if talking {
                    osaka.speech = Some((OK, 5000));
                }
                osaka.episode = trying.then(trial_episode);
                osaka.start_job(Job::Use(seat), 1000, &Chances::default(), &mut rng);
                let play = begun(&osaka).3;
                (
                    play.before.map(|s| s.splice),
                    play.after.map(|s| s.splice),
                    osaka.speech.is_some(),
                )
            };
            let (before, after, _) = start(false, false);
            assert_eq!(before, Some(SpliceId::TestSnack), "seed {seed}");
            codas += usize::from(after.is_some());
            assert_eq!(start(true, false), (None, after, true), "seed {seed}");
            let (before, after, _) = start(false, true);
            assert_eq!((before, after), (None, None), "seed {seed}");
        }
        assert!((16..48).contains(&codas), "{codas} codas in 64");
    }

    /// About one plain watch in five she flicks through the channels,
    /// drawn from her latest decision's whims (her body's stream draws
    /// the same either way): never on the shopping channel, with her
    /// home on her mind or trying the piece, nor twice in ten minutes
    /// (a watch she didn't flick through doesn't count: the next whims
    /// to roll it, she does); cued, always (the shopping channel too
    /// waits), and never cued to watch plainly.
    #[test]
    fn she_flicks_through_the_channels_one_watch_in_five() {
        let seat = seat_for(Use::Watch, Furniture::Tv);
        let own = |osaka: &Osaka| begun(osaka).3.own;
        let surfs_on = |seed: u64| {
            own(&started(seat, 1000, &Chances::default(), None, None, seed).0) == ScriptId::Surf
        };
        let surfing = (0..).find(|&seed| surfs_on(seed)).unwrap_or_default();
        let mut surfs = 0u64;
        let n = 2400u64;
        for seed in 0..n {
            let plain = Chances::default();
            let (osaka, rng) = started(seat, 1000, &plain, None, None, seed);
            let mut alone = Rng(seed);
            let _ = Osaka::standing_at(10, 10, 0, &mut alone);
            let span = alone.range(use_duration(Use::Watch).0, use_duration(Use::Watch).1);
            assert_eq!(rng, alone.0, "seed {seed}: one body draw");
            assert_eq!(begun(&osaka).1 - begun(&osaka).0, span, "seed {seed}");
            let surfed = own(&osaka) == ScriptId::Surf;
            surfs += u64::from(surfed);
            if surfed {
                // Not again within ten minutes.
                let mut again = osaka.clone();
                for (at, may) in [(600_999, false), (601_000, true)] {
                    again.start_job(Job::Use(seat), at, &plain, &mut Rng(seed));
                    assert_eq!(own(&again) == ScriptId::Surf, may, "seed {seed} at {at}");
                }
            } else {
                // Didn't, so it didn't cool: whims that roll it, a few
                // minutes on, and she does.
                let mut again = osaka.clone();
                again.whims = Whims(surfing ^ 0x5eed);
                again.start_job(Job::Use(seat), 300_000, &plain, &mut Rng(seed));
                assert_eq!(own(&again), ScriptId::Surf, "seed {seed}, then {surfing}");
            }
            // Never on the shopping channel, her home on her mind, or a
            // trial; cued to watch plainly, never; cued to surf, always.
            let advert = Chances {
                advert: Some(Furniture::Lamp),
                ..Chances::default()
            };
            let (osaka, _) = started(seat, 1000, &advert, None, None, seed);
            assert_eq!(own(&osaka), ScriptId::Shopping, "seed {seed}");
            let broken = Chances {
                broken: broken_for(Use::Watch, Furniture::Tv).into_iter().collect(),
                ..Chances::default()
            };
            let (osaka, _) = started(seat, 1000, &broken, None, None, seed);
            assert_eq!(own(&osaka), ScriptId::Watch, "seed {seed}");
            let mut rng = Rng(seed);
            let mut trying = Osaka::standing_at(10, 10, 0, &mut rng);
            trying.whims = Whims(seed ^ 0x5eed);
            trying.episode = Some(trial_episode());
            trying.start_job(Job::Use(seat), 1000, &plain, &mut rng);
            assert_eq!(own(&trying), ScriptId::Watch, "seed {seed}");
            let cue = Some(Cue::Script(ScriptId::Watch));
            let (osaka, _) = started(seat, 1000, &plain, cue, None, seed);
            assert_eq!(own(&osaka), ScriptId::Watch, "seed {seed}");
            let cue = Some(Cue::Script(ScriptId::Surf));
            let (osaka, _) = started(seat, 1000, &advert, cue, None, seed);
            assert_eq!(own(&osaka), ScriptId::Surf, "seed {seed}");
            assert_eq!(osaka.events, [], "seed {seed}: nothing bought");
        }
        // One in five: 480 ± 20 or so (one in four or six falls out).
        assert!(
            (n * 7 / 40..n * 9 / 40).contains(&surfs),
            "{surfs} surfs in {n} watches"
        );
    }

    /// Surfing: snow, colour bars, snow, the sunrise (ooh!), then snow
    /// to the end, humming, pleased; in her watching pose throughout.
    #[test]
    fn surfing_flicks_through_the_channels_in_turn() {
        use super::super::art::Channel;
        use script::SURF_MS;
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
        let length = use_duration(Use::Watch).0;
        let play = Play::plain(ScriptId::Surf);
        for sofa in [false, true] {
            let host = if sofa { Pose::Lounge } else { Pose::Sit };
            let mut at = |t| {
                let use_ = Played {
                    what: Use::Watch,
                    sofa,
                    bought: None,
                    grievance: None,
                    length,
                };
                let ((pose, face, bubble), prop) = played_as(&mut osaka, use_, play, t);
                assert_eq!(pose, host, "{t}");
                (
                    face,
                    bubble,
                    prop.and_then(Prop::channel)
                        .map(|c| c != Channel::Snow(0) && c != Channel::Snow(1)),
                    prop.map(|p| p.framed(0)),
                )
            };
            let snow = Some(Prop::Tv(Channel::Snow(0)));
            assert_eq!(at(0), (Face::Curious, None, Some(false), snow));
            assert_eq!(at(SURF_MS - 1).3, snow);
            assert_eq!(
                at(SURF_MS),
                (
                    Face::Vacant,
                    None,
                    Some(true),
                    Some(Prop::Tv(Channel::ColourBars))
                )
            );
            assert_eq!(at(2 * SURF_MS).3, snow);
            assert_eq!(
                at(3 * SURF_MS),
                (
                    Face::Curious,
                    Some(Bubble::Ooh),
                    Some(true),
                    Some(Prop::Tv(Channel::Sunrise))
                )
            );
            assert_eq!(
                at(4 * SURF_MS),
                (Face::Happy, Some(Bubble::Hum), Some(false), snow)
            );
            assert_eq!(
                at(length - 1),
                (Face::Happy, Some(Bubble::Hum), Some(false), snow)
            );
        }
    }

    /// The lamp stays on a moment as she settles into bed (blinking,
    /// thoughtful), then goes off while she sleeps; trying the bed where
    /// she's set it down, off at once (it would flicker).
    #[test]
    fn the_lamp_goes_off_a_moment_after_she_lies_down() {
        let seat = seat_for(Use::Sleep, Furniture::Bed);
        let (osaka, _) = started(seat, 1000, &Chances::default(), None, None, 4);
        let (since, until, ..) = begun(&osaka);
        let shown = |osaka: &Osaka, t| (osaka.acting(t).2, osaka.prop(t));
        assert_eq!(shown(&osaka, since), (Some(Bubble::Dots), None));
        let lamp_on = since + script::LAMP_ON_MS;
        assert_eq!(shown(&osaka, lamp_on - 1), (Some(Bubble::Dots), None));
        assert_eq!(
            shown(&osaka, lamp_on),
            (Some(Bubble::Zzz), Some(Prop::LampOff))
        );
        assert_eq!(
            shown(&osaka, until - 1),
            (Some(Bubble::Zzz), Some(Prop::LampOff))
        );
        let mut rng = Rng(4);
        let mut trying = Osaka::standing_at(10, 10, 0, &mut rng);
        trying.episode = Some(trial_episode());
        trying.start_job(Job::Use(seat), 1000, &Chances::default(), &mut rng);
        let (since, until, ..) = begun(&trying);
        let lamp_on = since + script::LAMP_ON_MS;
        for t in [since, since + 1, lamp_on - 1, lamp_on, until - 1] {
            assert_eq!(trying.prop(t), Some(Prop::LampOff), "trying, {t}");
            assert_eq!(trying.acting(t).2, Some(Bubble::Zzz), "trying, {t}");
        }
    }

    /// Whether a key of `keys` ends on the frame grid strictly inside a
    /// body `length` ms long (where a wakeup on the grid and at the key's
    /// end coincide).
    fn a_key_ends_on_the_grid(keys: &[script::Key], length: u64) -> bool {
        keys.iter()
            .map(|k| k.span.end(Some(length)))
            .any(|end| end > 0 && end < length && end % USE_FRAME_MS == 0)
    }

    /// When a use from [`SINCE`] playing `play`, `length` ms in all,
    /// should wake her, from the parts it plays in turn: on every frame
    /// of each part (the prelude, the body, the coda), counted from that
    /// part's start; as each of its keys ends (the part's end with its
    /// last); as she starts and stops saying what's wrong with her home
    /// (from `grievance` ms in); and at its end. Ascending, each once.
    fn wakeups(play: Play, length: u64, grievance: Option<u64>) -> Vec<u64> {
        let until = SINCE + length;
        let start = play.body_start(SINCE);
        let end = play.body_end(SINCE, until);
        let parts = [
            play.before
                .map(|s| (s.splice.script().keys(s.branch), SINCE, start)),
            Some((play.own.keys(play.branch), start, end)),
            play.after
                .map(|s| (s.splice.script().keys(s.branch), end, until)),
        ];
        let mut want: Vec<u64> = parts
            .into_iter()
            .flatten()
            .flat_map(|(keys, from, to)| {
                let frames = (1..)
                    .map(move |k| from + k * USE_FRAME_MS)
                    .take_while(move |&t| t < to);
                let ends = keys.iter().map(move |k| from + k.span.end(Some(to - from)));
                frames.chain(ends)
            })
            .chain(
                grievance
                    .into_iter()
                    .flat_map(|g| [SINCE + g, SINCE + g + GRIEVANCE_MS]),
            )
            .chain([until])
            .filter(|&t| t > SINCE && t <= until)
            .collect();
        want.sort_unstable();
        want.dedup();
        want
    }

    /// Keys change on time: a use wakes her on every frame of the part
    /// playing (counted from its start), as each key ends and as a
    /// grievance comes and goes, and at nothing else, each wakeup
    /// strictly after the last; and how she looks, and what's on her
    /// furniture (what's on TV aside, which moves at paint time), only
    /// ever changes at a wakeup. Every use, the shopping channel too,
    /// with no grievance, one on the frame grid and one off it, plain and
    /// with every splice as a prelude, a coda or both (off the grid's
    /// beat, so a part timed from the wrong start shows), at trial
    /// lengths, through each use's lengths and where a key ends on the
    /// frame grid.
    #[test]
    fn a_use_wakes_her_as_each_key_ends() {
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
        let splices: Vec<script::SpliceId> =
            [script::SpliceId::TestSnack, script::SpliceId::TestSleep]
                .into_iter()
                .chain(script::SpliceId::ALL)
                .collect();
        // Long enough to bob, and not a whole number of frames.
        let spliced = |splice, len| script::Spliced {
            splice,
            len,
            branch: 0,
        };
        let befores: Vec<_> = std::iter::once(None)
            .chain(splices.iter().map(|&s| Some(spliced(s, 6123))))
            .collect();
        let afters: Vec<_> = std::iter::once(None)
            .chain(splices.iter().map(|&s| Some(spliced(s, 6789))))
            .collect();
        let mut wrapped = 0;
        for what in Use::ALL {
            let (lo, hi) = use_duration(what);
            let boughts: &[Option<Furniture>] = if what == Use::Watch {
                &[None, Some(Furniture::Lamp)]
            } else {
                &[None]
            };
            for &bought in boughts {
                let plain = Play::of(what, bought);
                let keys = plain.own.keys(plain.branch);
                let on_grid = (lo..=hi).find(|&l| a_key_ends_on_the_grid(keys, l));
                let lengths: Vec<u64> = [TRIAL_USE_MS.0, TRIAL_USE_MS.1 - 1, lo, lo + 97, hi]
                    .into_iter()
                    .chain(on_grid)
                    .collect();
                for &length in &lengths {
                    for &before in &befores {
                        for &after in &afters {
                            let play = Play {
                                before,
                                after,
                                ..plain
                            };
                            // The body `length` long, the splices round it.
                            let length =
                                length + before.map_or(0, |s| s.len) + after.map_or(0, |s| s.len);
                            wrapped += usize::from(before.is_some() || after.is_some());
                            for grievance in [None, Some(USE_FRAME_MS), Some(USE_FRAME_MS + 333)] {
                                let use_ = Played {
                                    what,
                                    sofa: false,
                                    bought,
                                    grievance,
                                    length,
                                };
                                let at = format!(
                                    "{what:?} bought {bought:?} before {before:?} after \
                                     {after:?} grievance {grievance:?}: {length}"
                                );
                                played_as(&mut osaka, use_, play, 0);
                                let until = SINCE + length;
                                let mut woke = Vec::new();
                                let mut now = SINCE;
                                while now < until {
                                    let due = osaka.first_due(now);
                                    assert!(due > now, "{at}: due {due} at {now}");
                                    woke.push(due);
                                    now = due;
                                }
                                assert_eq!(woke, wakeups(play, length, grievance), "{at}");
                                // Between wakeups, nothing she shows changes.
                                let mut shown = |t: u64| {
                                    let (look, prop) = played_as(&mut osaka, use_, play, t - SINCE);
                                    (look, prop.map(|p| p.framed(0)))
                                };
                                let times = (SINCE..until)
                                    .step_by(37)
                                    .chain(woke.iter().map(|&w| w - 1));
                                for t in times {
                                    let last = woke.iter().rev().find(|&&w| w <= t).copied();
                                    let last = last.unwrap_or(SINCE);
                                    assert_eq!(shown(t), shown(last), "{at}: at {t}, woke {last}");
                                }
                            }
                        }
                    }
                }
            }
        }
        assert!(wrapped > 100, "{wrapped}");
    }
}
