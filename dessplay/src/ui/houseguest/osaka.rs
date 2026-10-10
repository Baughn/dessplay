//! Osaka herself: where she is, what she's doing, and when that next
//! changes. Every activity is a finite, wall-clock-timed step, so any
//! instant is a safe place to cut the visit short.

use super::Rng;
use super::art::{self, DoorFrame};
use super::brain::{self, Mood, Need, Needs, Rising, Spot, Want};
use super::calendar::{self, Owed, Tints};
use super::door::{DoorSpot, Set};
use super::layer::Placed;
use super::mind::{self, Beat, Bind, Ctx, Heading, Here, Lines, Loss, PoolId, RIDDLES, Whims};
use super::rarity::{self, Pity, Rares};
use super::room::{Furniture, MadeId, PieceRef, Seat, Use};
use super::routine::{self, DayTime};
use super::rules::{Grievance, Placement, Repair, TIE_CELLS, Trials};
use super::scenes::{
    Build, Grip, Held, Job, JobRef, LayerOp, Leave, Lift, Pull, SetDown, Side, Swap,
};
use super::script::{
    self, Chat, ClockGlance, Cue, Play, Prop, ScriptId, SpliceCtx, SpliceId, Surface,
};
use super::sprite::{self, Face, Facing, HEIGHT, Pose, SpriteCell};
use super::stillness::{self, Stillness};
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
    /// The kinds of her real furniture shown (boxed or not): what stands
    /// in her room, whether or not she could get to it to use it now (a
    /// seat behind text drops out of `seats`, its piece still stands).
    pub real: Vec<Furniture>,
    /// Makeshift furniture she could make of text, for each use.
    pub builds: Vec<super::scenes::Build>,
    /// Lines she could borrow a strip of to read (long enough, with room
    /// in her text layer for the strip): see `mind::borrow`.
    pub borrows: Vec<Pull>,
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
    /// Her wall clock, if it's shown out of its box on a strip: where it
    /// hangs, and the strip's floor and columns (what she glances at,
    /// on that strip: see [`ClockOn::seen_from`]).
    pub clock: Option<ClockOn>,
    /// Her external door this frame, if it stands anywhere (the door
    /// batch, D4): what a dash in comes through, and what she goes out
    /// by.
    pub door: Option<super::door::DoorSpot>,
    /// What her door's box mustn't meet this frame (her pieces as laid,
    /// what she made, the place kept for the piece in her pocket: see
    /// [`super::door::obstacles`]): with no way to her door, she goes out
    /// at her feet only where her box meets none of them.
    pub obstacles: Vec<Rect>,
    /// Where the door she's through stands this frame (only while she's
    /// through her own door, [`Osaka::opened_door`]; else `None`): the
    /// frame's door read without the focused pane and from where that
    /// door stood ([`super::door::Ungated`]), so a focused pane is never
    /// a reason to move it (door batch D6). `None` while she's through
    /// it: no door anywhere now.
    pub door_through: Option<super::door::DoorSpot>,
}

/// Her wall clock where it hangs (phase 5b D7): its middle column, and
/// the floor and columns of the strip it hangs over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ClockOn {
    /// Its middle column.
    pub x: i32,
    /// The strip's floor.
    pub floor: i32,
    /// The strip's first column.
    pub from: i32,
    /// Just past the strip's last column.
    pub to: i32,
}

impl ClockOn {
    /// Its column, if it hangs on the strip she'd stand on at `(x, y)`
    /// (where she can see it, to glance up at it).
    pub fn seen_from(self, (x, y): (i32, i32)) -> Option<i32> {
        (self.floor == y && (self.from..self.to).contains(&x)).then_some(self.x)
    }
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
    /// The kind of piece it is.
    pub item: Furniture,
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

/// A spot where a door in space she goes out by may open: her box there
/// meets none of `obstacles` (her pieces as laid, what she made, the
/// place kept for the piece in her pocket: [`super::door::obstacles`]),
/// as the frame it was judged on stood them (door batch M25, step 10a).
/// Made only by [`Clear::of`], so no way out of hers in space opens in
/// her bed or on her TV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Clear((i32, i32));

impl Clear {
    /// `spot`, if her box there meets none of `obstacles`.
    pub fn of(spot: (i32, i32), obstacles: &[Rect]) -> Option<Self> {
        (!obstacles.iter().any(|&o| box_meets(o, spot))).then_some(Self(spot))
    }

    pub fn spot(self) -> (i32, i32) {
        self.0
    }
}

/// The place nearest `from` (her floor `here` first, then the floors she
/// can get to from it, then any; one she can stay at before one she only
/// passes; then the fewest columns and rows away) where a door in space
/// may open clear of `obstacles` (door batch M25, step 10a). `None`:
/// nowhere on any floor.
fn nearest_clear(
    terrain: &Terrain,
    here: Option<usize>,
    from: (i32, i32),
    obstacles: &[Rect],
) -> Option<Clear> {
    let mut reach = vec![false; terrain.platforms.len()];
    let mut queue: std::collections::VecDeque<usize> = here.into_iter().collect();
    while let Some(at) = queue.pop_front() {
        if reach.get(at).copied().unwrap_or(true) {
            continue;
        }
        reach[at] = true;
        queue.extend(terrain.links.iter().filter(|l| l.from == at).map(|l| l.to));
    }
    terrain
        .platforms
        .iter()
        .enumerate()
        .flat_map(|(p, floor)| (floor.x0..=floor.x1).map(move |x| (p, (x, floor.y))))
        .filter_map(|(p, spot)| Clear::of(spot, obstacles).map(|clear| (p, clear)))
        .min_by_key(|&(p, clear)| {
            let (x, y) = clear.spot();
            (
                Some(p) != here,
                !reach.get(p).copied().unwrap_or(false),
                !terrain.restful(x, y),
                (x - from.0).abs() + (y - from.1).abs(),
                y,
                x,
            )
        })
        .map(|(_, clear)| clear)
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

/// Where she spends the night, of what's shown (A4): her bed, else her
/// sofa, real ones where she can lie. The one binder of her night's place
/// for her routine's bed reflex and for being tucked in.
pub(super) fn night_seat(chances: &Chances, terrain: &Terrain) -> Option<Seat> {
    night_seats(chances, terrain).next()
}

/// Where she has a snack from `fridge` as it's shown now, if it is: what
/// she got up or came home for (her midnight snack, the lunch she
/// dashed home for) names its piece, and is found afresh here each time
/// she sets off for it, so she never sets off for one gone into the
/// closet, nor for where it stood before.
fn fridge_seat(fridge: PieceRef, chances: &Chances) -> Option<Seat> {
    chances
        .seats
        .iter()
        .find(|s| s.piece == fridge && s.what == Use::Snack)
        .copied()
}

/// Every real piece she could spend the night on, of what's shown, in
/// the order she'd choose them (A4): her beds, then her sofas.
fn night_seats<'a>(chances: &'a Chances, terrain: &'a Terrain) -> impl Iterator<Item = Seat> + 'a {
    [Use::Sleep, Use::Nap].into_iter().flat_map(move |what| {
        chances
            .seats
            .iter()
            .filter(move |s| {
                s.what == what && !s.makeshift() && terrain.platform_at(s.x, s.y).is_some()
            })
            .copied()
    })
}

/// A spot on its floor just beside the piece she's at from `seat` (her
/// bed, her sofa; or a desk she sits beside), where she may stay: the
/// side nearer first.
fn beside(seat: &Seat, terrain: &Terrain) -> Option<(i32, i32)> {
    // Where in the piece (facing right) she's seated, and whether she
    // faces away from the piece's own facing (`Shown::seat`'s inverse).
    let (cols, sit) = match seat.piece {
        PieceRef::Made(_) => (
            i32::from(super::scrap::footprint(seat.item).0),
            Some(super::scrap::sit(seat.item)),
        ),
        PieceRef::Real(item) => (i32::from(item.spec().footprint.0), item.spec().sit),
    };
    let left = match sit {
        Some((col, flip)) => {
            let facing = match (seat.facing, flip) {
                (facing, false) => facing,
                (Facing::Right, true) => Facing::Left,
                (Facing::Left, true) => Facing::Right,
            };
            match facing {
                Facing::Right => seat.x - col,
                Facing::Left => seat.x - (cols - 1 - col),
            }
        }
        // Centred, whichever way it faces.
        None => seat.x - cols / 2,
    };
    let half = sprite::WIDTH / 2;
    let floor = terrain.platform_at(seat.x, seat.y);
    let mut xs = [left - half - 1, left + cols + half];
    xs.sort_by_key(|&x| ((x - seat.x).abs(), x));
    xs.into_iter().map(|x| (x, seat.y)).find(|&(x, y)| {
        floor.is_some() && terrain.platform_at(x, y) == floor && terrain.restful(x, y)
    })
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
    /// What her calendar owed her on this real date has shown: not owed
    /// again that date (phase 5b D5).
    Calendar(chrono::NaiveDate),
    /// A rare script of hers showed for the first time (its first key
    /// played: not as it was planned, or offered): seen from now on, and
    /// her pity for its tier starts again (phase 5b D6).
    Seen(ScriptId),
    /// She said what's wrong with her home that she felt on sight, for
    /// the rule on `row` of the table, on game day `day` (none: no
    /// routine reaches her): not again that day (door batch, D7).
    Grieved { row: usize, day: Option<u64> },
}

impl Chances {
    /// Whether `spot` is in the chat pane (and she's resident).
    pub(super) fn in_chat(&self, spot: (i32, i32)) -> bool {
        self.chat.is_some_and(|chat| holds(chat, spot))
    }

    /// Whether a piece like `item` stands in her room: a real one shown
    /// (boxed or not), or one she made this visit (a heap or in shape).
    /// Read off the pieces, not their seats (phase 5c M10: a desk whose
    /// seat text blocks still stands there).
    pub(super) fn stands(&self, item: Furniture) -> bool {
        self.real.contains(&item) || self.mine.iter().any(|m| m.item == item)
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
pub(super) const WALK_MS: u64 = 333;
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
/// Her look up at a chat line from a still act is startled (`!`) a frame,
/// so it never comes and goes inside one (phase 5c's tail, T4); on her
/// feet, a short act that moves, [`SURPRISED_MS`].
const LOOK_UP_SURPRISED_MS: u64 = USE_FRAME_MS;
/// Her look up's shortest: startled, puzzled to [`LOOK_MS`], then
/// watching plain-faced a frame (never coming and going inside one).
const LOOK_UP_MS: u64 = LOOK_MS + USE_FRAME_MS;
pub(super) const LOOK_MS: u64 = 4000;
/// A conversation keeps her watching until it's been quiet this long
/// (counted from the line, so past her look, `LOOK_MS`, by a second; a
/// lively chat renews it with each line; a look up in place watches a
/// frame past its `?` at least, [`LOOK_UP_MS`], 5.4 s). Counted from the line also when
/// her look is put off (aloft, in a door, passing over text, answering
/// with the andagi): if she can't watch within it, the line is let go.
const WATCH_MS: u64 = 5_000;
// A line's look is followed by a watch: the tests that read "watching
// chat" after a look, and the census's restarts, rely on it.
const _: () = assert!(WATCH_MS > LOOK_MS);
const BLINK_MS: u64 = 150;
/// On a pose she holds (phase 5c Q3, [`Pose::holds`]), a slow blink of
/// [`BLINK_MS`] every this long (ms, from and to).
const HELD_BLINK_GAP_MS: (u64, u64) = (6_000, 12_000);
/// How many slow blinks ahead, unseen, her next wakeup is looked for
/// ([`Osaka::wakes_at`]): six minutes of them at the least.
const HELD_BLINK_LOOKAHEAD: usize = 64;
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
/// Her homework on the floor over, the first time a visit (phase 5c D5).
pub(super) const MY_BACK: &str = line!("My back...");
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
pub(super) const OK: &str = line!("...I'm OK.");
/// Stirring at a chat line in the night.
pub(super) const MM: &str = line!("mm...");
/// Stirring at a chat line dozing by day (phase 5c B1).
pub(super) const STIRRED: &str = line!("Mm?");
/// Spacing out, musing or not (ms range).
pub(super) const SPACE_OUT_MS: (u64, u64) = (10_000, 28_000);
/// After her calendar's greeting ("I'm home!", "Mornin'."), she spaces
/// out a moment at least (ms): its own, not a daydream's (phase 5c step
/// 8c lengthened those, not this).
pub(super) const GREETING_PAUSE_MS: u64 = 6000;
/// A glance up at her wall clock (phase 5b D7): a moment, spacing out.
pub(super) const CLOCK_GLANCE_MS: u64 = 2500;

/// Tearing text off a line for furniture: bracing, then the rip.
const BRACE_MS: u64 = 700;
/// Each step of reeling the torn text in to her hands.
const REEL_MS: u64 = 220;
const RIP: &str = line!("Rrrip!");
pub(super) const SCRUNCH: &str = line!("scrunch...");
pub(super) const THERE: &str = line!("There!");
/// How long she keeps saying `text`: a frame ([`USE_FRAME_MS`]) at
/// least, however short, so a line never comes and goes inside one (and
/// a stir's turn, which lasts as long as its murmur, with it: Round 8,
/// "Mm?" by day lasted 1380 ms).
pub(super) fn speech_ms(text: &str) -> u64 {
    (1200 + 60 * text.chars().count() as u64).max(USE_FRAME_MS)
}

/// How she turns, stirring at a chat line dozing posed `pose` (see
/// [`Osaka::stirring`]): over, or her head up off her knees dozing where
/// she sits; `None` where she doesn't turn.
fn stir_turn(pose: Pose) -> Option<Pose> {
    match pose {
        Pose::Sleep(_) => Some(Pose::Sleep(1)),
        Pose::Nap(_) => Some(Pose::Nap(1)),
        Pose::LieBack(_) => Some(Pose::LieBack(1)),
        Pose::SitDoze(_) => Some(Pose::SitDoze(0)),
        _ => None,
    }
}

/// Whether `pose`, shown stirring, is one she's turned to (so it holds
/// the stir through).
#[cfg(test)]
pub(super) fn turned_stirring(pose: Pose) -> bool {
    stir_turn(pose) == Some(pose)
}

/// What her calendar has for her to do (see [`Osaka::calendar_due`]).
struct CalendarDue {
    greeting: Option<(chrono::NaiveDate, Owed, &'static str)>,
    setsubun: Option<(chrono::NaiveDate, Owed)>,
    tv: Option<Seat>,
}

/// The decision that settles her further where she is (see
/// [`Osaka::settle_in`]): a continuation, not a method she chooses by.
pub(super) const SETTLE_IN: &str = "settle in";

/// What she settles into where she is (see [`Osaka::settling`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Settle {
    /// Something on the spot.
    Idle(Activity),
    /// The same piece, another use from the same seat.
    Use(Seat),
}

/// The musings a daydream session has still to say (phase 5c B6): the
/// next at `next`, and `left` in all; `said` so far (her first, as she
/// set about it, included). Each next one is drawn from her decision's
/// whims for the session's `said`th musing, as is the gap after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Session {
    left: u8,
    next: u64,
    said: u8,
    /// Her whims as she set about it.
    whims: Whims,
}

/// Where she is in borrowing a strip of a line to read (see
/// [`Act::Borrow`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Borrowing {
    /// Taking hold of the line's end.
    Brace,
    /// Reeling the strip in to her hands, `step` cells so far (as she
    /// tears text for furniture).
    Reel(u16),
    /// Sat beside the tear, reading the strip, until `until`.
    Read { until: u64 },
    /// Sliding it back: the strip as it was `step` cells into reeling
    /// it in (0: the line whole).
    Slide(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    Stand {
        until: u64,
    },
    /// Spacing out, maybe telling a riddle (a musing only), or a
    /// daydream session's musings to come (see [`Session`]).
    SpaceOut {
        since: u64,
        until: u64,
        play: Option<Play>,
        session: Option<Session>,
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
    /// An activity on the spot. `play`: her night's script, lying on
    /// the floor for the night (see [`Surface::Floor`]); `None` for any
    /// other activity, which plays none.
    Idle {
        what: Activity,
        since: u64,
        until: u64,
        play: Option<Play>,
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
    /// Through a door from where she stands to `to` (see [`DOOR`]): a
    /// door in space, or her external door at its spot ([`Through`]);
    /// away for `gap` ms between the doors.
    Door {
        since: u64,
        to: Through,
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
    /// Borrowing a strip of the line `pull` to read (phase 5c D5, HG
    /// #72): tearing it off and reeling it in, reading it sat beside the
    /// tear, sliding it back ([`Borrowing`]). `since`: the phase's start.
    /// Whatever ends it, the guest puts back what of the strip is still
    /// out (see [`Osaka::holding`]).
    Borrow {
        pull: Pull,
        since: u64,
        phase: Borrowing,
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

/// How what she did came to ease her needs: the one place that eases
/// them for what she did ([`Osaka::serve`]) is told which, for her trace
/// log and the credit class's test.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Via {
    /// By the share of it done, as she leaves it ([`Osaka::credit_done`]).
    Share,
    /// Whole, as she sets off ([`Osaka::choose_next`]): moving is the
    /// point of moving.
    SetOff,
    /// Whole as she finishes it, or (a pull let go) by the share done.
    Whole,
    /// Her shift: whole, home from it ([`Osaka::come_home`]); cut short,
    /// by the share of it she worked ([`Osaka::cut_shift`]).
    Shift,
    /// Whole, as the frame takes the piece she set down
    /// ([`Osaka::set_down_done`]).
    SetDown,
}

/// How a chosen want comes to ease what it serves (see
/// [`Osaka::credit_path`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CreditPath {
    /// It serves nothing.
    Nothing,
    /// It eases what it serves by this way.
    By(Via),
    /// It's a use of a piece, and eases as the use it is (by the share
    /// of it done: [`Via::Share`]), not as the want that chose it.
    AsUse,
}

/// One easing of her needs for what she did, as [`Osaka::serve`] records
/// it (tests).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Served {
    pub want: Want,
    pub share: f64,
    /// The count of [`Osaka::decisions`] then: a credit as she decides
    /// is that decision's; one between decisions, the next's.
    pub decision: usize,
    pub via: Via,
    /// Where it had her, as her needs care.
    pub spot: Spot,
}

/// What her body does while she's moving, as the census counts it (see
/// [`Osaka::census_motion`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Body {
    /// Walking, walking off the screen's edge, or around off it.
    Walk,
    /// Climbing a pole, or clambering over a divider.
    Climb,
    /// Peering over an edge, falling, dazed after a fall.
    Fall,
    /// A door's beats she's seen in.
    Door,
    /// Heaving a line of text she pulls, stepping back with it.
    Text,
}

#[cfg(test)]
impl Body {
    pub const ALL: [Body; 5] = [Body::Walk, Body::Climb, Body::Fall, Body::Door, Body::Text];

    /// Its column heading.
    pub fn label(self) -> &'static str {
        match self {
            Body::Walk => "walk",
            Body::Climb => "climb",
            Body::Fall => "fall",
            Body::Door => "door",
            Body::Text => "text",
        }
    }
}

/// Her moving, as the census counts it: what her body does, what for
/// (see [`Osaka::census_purpose`]), and the want behind it, if one.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub purpose: &'static str,
    pub body: Body,
    pub want: Option<Want>,
}

/// One time she set off (see [`Osaka::count_set_off`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetOff {
    pub at: u64,
    pub body: Body,
    /// What for ([`Osaka::census_purpose`]); empty until the decision
    /// that set her off has its method (its chain), then filled.
    pub purpose: &'static str,
    /// The first set-off after a chat line stopped her (whether she only
    /// passed on over text, and what she was moving for if she was).
    pub after_chat: Option<(bool, Option<&'static str>)>,
}

/// How an act moves her, as the census counts it (see [`census_moves`]).
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Moves {
    /// What she sets off on: a walk (or off the screen's edge), a climb,
    /// a door.
    Off(Body),
    /// Moving that only carries on from setting off: peering over an
    /// edge, falling, dazed, around off the screen.
    On(Body),
    /// Moving in place: a pull's heave, the beat she steps back with the
    /// line. Moving for the share, but not a trip: what she sets off on
    /// after a pull is a set-off.
    Heave,
    /// Staying put (a pull's bracing and reeling in, a swap, a sneeze,
    /// exercise: whatever she does in place).
    Still,
}

#[cfg(test)]
impl Moves {
    /// On a trip (setting off, or carried on from it): what she sets off
    /// on next isn't a set-off unless a decision starts it.
    fn trip(self) -> bool {
        matches!(self, Moves::Off(_) | Moves::On(_))
    }
}

/// The census's one class of each act: how it moves her. Wildcard-free,
/// so a new act doesn't compile until it's classed; the census's motion,
/// set-offs and chat cuts all read it.
#[cfg(test)]
fn census_moves(act: &Act) -> Moves {
    match act {
        Act::Walk { .. } | Act::Out { .. } => Moves::Off(Body::Walk),
        Act::Climb { .. } | Act::Clamber { .. } => Moves::Off(Body::Climb),
        Act::Door { .. } => Moves::Off(Body::Door),
        Act::Peer { .. } | Act::Fall { .. } | Act::Dazed { .. } => Moves::On(Body::Fall),
        Act::Away { .. } => Moves::On(Body::Walk),
        // She steps back as a bracing beat turns into a heave (`fire`),
        // once the line's slack is in (`offset > gap`); the bracing beat
        // after it is still.
        Act::Pull {
            pull,
            offset,
            heaving: true,
            ..
        } if *offset > pull.gap => Moves::Heave,
        Act::Pull { .. }
        | Act::Stand { .. }
        | Act::SpaceOut { .. }
        | Act::Look { .. }
        | Act::Admire { .. }
        | Act::Glance { .. }
        | Act::Swap { .. }
        | Act::Giggle { .. }
        | Act::Innocent { .. }
        | Act::Sneeze { .. }
        | Act::PutBack { .. }
        | Act::Idle { .. }
        | Act::Home { .. }
        | Act::Poke { .. }
        | Act::Tear { .. }
        // Tearing a strip off, reading it, sliding it back: all in
        // place (the walk to the line is the set-off).
        | Act::Borrow { .. }
        | Act::Use { .. }
        | Act::Lift { .. }
        | Act::SetDown { .. } => Moves::Still,
    }
}

/// Where she is in a shift of her part-time job (see
/// [`Osaka::go_to_work`]). Whatever she decides next ends it (see
/// [`Osaka::choose_next`]) but her way to her door, which she goes on
/// with: it lasts only as long as the acts it set going. Work always
/// goes out and comes home through her door (door batch, step 4b): the
/// door's gap is the shift, and coming back out of it she's home
/// ([`Osaka::back_home`]). Which door that is is told by the shift, never
/// by the door: a door in space she hops by on her way is never it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shift {
    /// On her way out to her door for it: how long it'll last (drawn as
    /// she sets off), the gap of whichever door she goes out by.
    Going { gap: u64 },
    /// Through that door (in its beats, or out at work in its gap):
    /// coming back out of it, she's home from work.
    Out,
}

/// Where her routine takes her out of her home, through her door (phase
/// 5b D3, A9): school, which isn't something she wants (her part-time
/// job is a want, and a [`Shift`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Routine {
    /// School: out at 08:15 on a school day, home at 12:45.
    School,
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
    /// A still act (resting on the spot, spacing out, using a piece to
    /// rest at): on calm floor she looks up where she is and the act
    /// runs on, or, dozing, only stirs (see [`Osaka::look_up`]). Over
    /// text she passes on, as from any act. (A script that stops at a
    /// line, [`Chat::Stop`], makes its still act [`OnChat::Look`].) (Phase 5c B1: looking draws
    /// the eye, which is only worth it for a change; getting up and
    /// settling back down is the biggest change her sprite makes.)
    LooksUp,
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
    /// Her routine: it's time for school, or bed ([`Osaka::cut`]). She
    /// isn't startled; she just decides again.
    Routine,
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
    /// Her routine as she decided (`None`: it didn't reach her).
    pub day: Option<DayTime>,
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
            day: None,
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
        write!(f, " → {}", self.act)?;
        if let Some(day) = self.day {
            write!(f, " @ {day}")?;
        }
        Ok(())
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
            Self::Borrow { pull, .. } => Some(JobRef::Borrow(pull)),
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
        // A script that stops at a chat line though the act hosting it
        // is still (Setsubun's beans, a dash home: see `ScriptId::on_chat`).
        let stops = match self {
            Self::Use { play, .. }
            | Self::SpaceOut {
                play: Some(play), ..
            }
            | Self::Idle {
                play: Some(play), ..
            } => play.own.on_chat() == Chat::Stop,
            _ => false,
        };
        let (stays, on_chat) = match self {
            Self::Use { seat, .. } => {
                let on_chat = match seat.what {
                    Use::Lounge
                    | Use::Nap
                    | Use::Sleep
                    | Use::Homework
                    | Use::Watch
                    | Use::Read
                    | Use::LookOut => OnChat::LooksUp,
                    // Chores: unpacking, crumpling text, a snack, petting
                    // the cat.
                    Use::Unpack | Use::Crumple | Use::Snack | Use::Pet => OnChat::Look,
                };
                (Stays::Job, on_chat)
            }
            // Reading the strip she sits still, on the calm spot she
            // tore it from: she looks up where she is. Tearing it off
            // and sliding it back, she's at the text: she stops.
            Self::Borrow {
                phase: Borrowing::Read { .. },
                ..
            } => (Stays::Job, OnChat::LooksUp),
            Self::Lift { .. }
            | Self::SetDown { .. }
            | Self::Pull { .. }
            | Self::Tear { .. }
            | Self::Borrow { .. }
            | Self::Swap { .. }
            | Self::Giggle { .. }
            | Self::Innocent { .. } => (Stays::Job, OnChat::Look),
            // Resting on the spot, not exercising.
            Self::Idle { what, .. } if what.restful() => (Stays::Rest, OnChat::LooksUp),
            Self::SpaceOut { .. } => (Stays::Rest, OnChat::LooksUp),
            Self::Stand { .. } | Self::Idle { .. } => (Stays::Rest, OnChat::Look),
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
        let on_chat = match on_chat {
            OnChat::LooksUp if stops => OnChat::Look,
            other => other,
        };
        ActProps { stays, on_chat }
    }
}

/// Where a door she goes through lets her out (door batch D6): a door in
/// space onto a spot, or her external door, at its spot by her home
/// (the one way out by her routine, and in again). Which it is is told
/// by type, never by the door's gap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Through {
    Space((i32, i32)),
    Home(DoorSpot),
}

impl Through {
    /// Where she stands at the far door.
    pub fn spot(self) -> (i32, i32) {
        match self {
            Self::Space(spot) => spot,
            Self::Home(door) => door.spot(),
        }
    }

    /// Which way she faces coming in through it: into the room from her
    /// door (door batch C6); a door in space, to the right.
    pub fn into_room(self) -> Facing {
        match self {
            Self::Space(_) => Facing::Right,
            Self::Home(door) => door.into_room(),
        }
    }
}

/// Her front door side-on in its wall in one of her beats through it
/// (door batch D8): the door's state, and her, if she's in sight, posed
/// `d` columns from her spot toward the wall (her image cut at its line).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct WallBeat {
    pub door: art::WallDoor,
    pub her: Option<(Pose, i32)>,
}

/// Her own door in one of her beats through it (see
/// [`Osaka::front_door`]): side-on in its wall, the beat's index and
/// what's drawn; or face-on (a fallback), its frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrontBeat {
    Wall(usize, WallBeat),
    Floor(DoorFrame),
}

/// What her side-on door draws `into` ms into beat `beat` of `DOOR`
/// (door batch D8, the approved table, with C2, C11-C13): shut, ajar,
/// open as she steps out through it a column a [`WALK_MS`] (`d` 1 to
/// 4, toward the wall); open, ajar, shut behind her; her slippers before
/// it and a card on its knob while she's out (the gap and the far door's
/// first beat), gone as it opens; open as she steps in (`d` 3 to 0,
/// carrying her shopping home from work, `shift`); ajar and shut with her
/// held side-on facing the room (or holding her shopping). Draw-only: the
/// beats' timing and what they mean are `DOOR`'s.
fn wall_beat(beat: usize, into: u64, shift: bool) -> WallBeat {
    use art::WallDoor::{Ajar, Open, Shut};
    let shut = Shut {
        flap: 0,
        away: false,
    };
    let away = Shut {
        flap: 0,
        away: true,
    };
    let step = i32::try_from(into / WALK_MS).unwrap_or(i32::MAX);
    let side = (Pose::Side, 0);
    let held = if shift { (Pose::Carry(0), 0) } else { side };
    let walk = |d: i32| Pose::Walk(d.rem_euclid(4) as u8);
    let (door, her) = match beat {
        0 => (shut, Some(side)),
        1 => (Ajar, Some(side)),
        2 => {
            let d = step.saturating_add(1).min(4);
            (Open, Some((walk(d), d)))
        }
        3 | 9 => (Open, None),
        4 | 8 => (Ajar, None),
        5 => (shut, None),
        6 | 7 => (away, None),
        10 => {
            let d = 3 - step.min(3);
            let pose = match d {
                _ if shift => Pose::Carry(d.rem_euclid(2) as u8),
                0 => Pose::Side,
                _ => walk(d),
            };
            (Open, Some((pose, d)))
        }
        11 => (Ajar, Some(held)),
        _ => (shut, Some(held)),
    };
    WallBeat { door, her }
}

/// One beat of going through a door: the door (if shown), whether she
/// is, whether it's the far end yet, and for how long.
struct DoorBeat {
    door: Option<DoorFrame>,
    her: bool,
    there: bool,
    ms: u64,
    /// She steps through the doorway in it (out, or in): through her
    /// own door, at her walking pace ([`STEP_THROUGH_MS`]).
    step: bool,
}

/// A door appears, she steps through, it shuts and goes; a door appears
/// where she's going, she steps out, it shuts and goes.
const DOOR: [DoorBeat; 13] = [
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: true,
        there: false,
        ms: 600,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: true,
        there: false,
        ms: 300,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: true,
        there: false,
        ms: 700,
        step: true,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: false,
        there: false,
        ms: 400,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: false,
        there: false,
        ms: 250,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: false,
        there: false,
        ms: 350,
        step: false,
    },
    DoorBeat {
        door: None,
        her: false,
        there: false,
        ms: 600,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: false,
        there: true,
        ms: 400,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: false,
        there: true,
        ms: 250,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: false,
        there: true,
        ms: 350,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Open),
        her: true,
        there: true,
        ms: 600,
        step: true,
    },
    DoorBeat {
        door: Some(DoorFrame::Ajar),
        her: true,
        there: true,
        ms: 300,
        step: false,
    },
    DoorBeat {
        door: Some(DoorFrame::Closed),
        her: true,
        there: true,
        ms: 400,
        step: false,
    },
];

/// How long she takes over each step through her own door's doorway
/// (beats 2 and 10, door batch C11): her walking pace, a column a
/// [`WALK_MS`] over the four between her spot and through the wall.
const STEP_THROUGH_MS: u64 = 4 * WALK_MS;

/// How long `beat` of a door to `to` lasts: `gap` stretches the time
/// between the doors (she's away), and her own door (`Through::Home`)
/// her steps through it, to walking pace. Every door's timing goes
/// through here, so none reads another door's.
fn beat_ms(beat: &DoorBeat, to: Through, gap: u64) -> u64 {
    if beat.door.is_none() {
        beat.ms.max(gap)
    } else if beat.step && matches!(to, Through::Home(_)) {
        STEP_THROUGH_MS
    } else {
        beat.ms
    }
}

/// The door beat `elapsed` ms into a door to `to` with `gap` between
/// its doors: its index, the beat, when it began and when the next
/// begins; `None` once it's over.
fn door_at(elapsed: u64, gap: u64, to: Through) -> Option<(usize, &'static DoorBeat, u64, u64)> {
    let mut end: u64 = 0;
    let mut start: u64 = 0;
    // Saturating: her routine's door out has a gap that never ends
    // (`u64::MAX`, see `Osaka::leaving`).
    script::at(&DOOR, elapsed, |beat| {
        start = end;
        end = end.saturating_add(beat_ms(beat, to, gap));
        end
    })
    .map(|(index, beat, end)| (index, beat, start, end))
}

/// The door beat `elapsed` ms into a door to `to`, and when the next
/// begins; `None` once it's over (see [`door_at`]).
fn door_beat(elapsed: u64, gap: u64, to: Through) -> Option<(&'static DoorBeat, u64)> {
    door_at(elapsed, gap, to).map(|(_, beat, _, end)| (beat, end))
}

/// What she heads for her door for, on her way out by it for `why`
/// (door batch F13): her shift's want for work (`Want::Work`), a walk
/// (no credit) for school.
fn leave_want(why: Leave) -> Want {
    match why {
        Leave::Work => Want::Work,
        Leave::School | Leave::Stage => Want::Walk,
    }
}

/// The stage's school scene (door batch D10): how long she's out
/// through her door before she comes back in.
const STAGE_GAP_MS: u64 = 5000;

/// When a door begun at `since`, its beats timed for `from` (a door to
/// it with its gap), began as a door `to` (to it, with its gap: its
/// beats timed so) that stands at `now` in the same beat as far in
/// (clamped to the beat): her own door's steps take longer than a door
/// in space's (see [`beat_ms`]), and a gap is beat 6's length, so a door
/// that turns from one to the other mid-way keeps its beat, never
/// jumping back or on (her shown again, or gone at once). Only
/// [`Act::redirect_door`] calls it.
fn retimed(since: u64, now: u64, from: (Through, u64), to: (Through, u64)) -> u64 {
    let Some((index, _, start, _)) = door_at(now.saturating_sub(since), from.1, from.0) else {
        return since;
    };
    let into = now.saturating_sub(since).saturating_sub(start);
    let before: u64 = DOOR
        .iter()
        .take(index)
        .map(|beat| beat_ms(beat, to.0, to.1))
        .fold(0, u64::saturating_add);
    let length = DOOR.get(index).map_or(1, |beat| beat_ms(beat, to.0, to.1));
    now.saturating_sub(before.saturating_add(into.min(length.saturating_sub(1))))
}

/// How long a door to `to` takes to let her through and close behind
/// her: the beats before the gap.
fn through_ms(to: Through) -> u64 {
    DOOR.iter()
        .take_while(|beat| beat.door.is_some())
        .map(|beat| beat_ms(beat, to, 0))
        .sum()
}

/// How far into a door's beats to `to` (with no gap) the far door first
/// shows, closed: a door she comes out of starts there, so a door
/// standing closed where she comes out (her closed door while she's
/// away, see `State::Away`) goes straight on into hers, and never
/// blinks away.
fn there_ms(to: Through) -> u64 {
    DOOR.iter()
        .take_while(|beat| !beat.there)
        .map(|beat| beat_ms(beat, to, 0))
        .sum()
}

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

/// Where she comes in by a door in space with no door of hers anywhere
/// (the door batch, M22): somewhere she can stay if there's such a
/// spot, else anywhere, her box clear of the chat pane and of
/// `obstacles` (her pieces).
pub(super) fn door_in_space_spot(
    terrain: &Terrain,
    chat: Rect,
    obstacles: &[Rect],
    rng: &mut Rng,
) -> Option<(i32, i32)> {
    let clear =
        |spot: (i32, i32)| !box_meets(chat, spot) && !obstacles.iter().any(|&o| box_meets(o, spot));
    calm_elsewhere(terrain, &clear, None, rng)
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

/// Nowhere clear of her pieces to go out by (door batch, step 10a): she
/// stands this long before her routine sends her again.
const OUT_AGAIN_MS: u64 = 3_000;
/// She goes to work after this long into a visit, at the earliest.
const WORK_AFTER_MS: u64 = 3 * 60_000;
/// How long a shift lasts (ms range).
const SHIFT_MS: (u64, u64) = (60_000, 180_000);
/// Back from work, showing what she brought.
const HOME_MS: u64 = 3000;
/// What she says, back from work: her coming-home pool's first line.
const HOME: &str = mind::HOME.lines[0];
/// What she says leaving for school when it cut her breakfast short
/// (round 1; her toast waits for its art).
pub(super) const LATE: &str = line!("Late, late, late!");

/// How long she pokes the scrollback accordion, and each poke.
const POKE_MS: u64 = 2000;
const POKE_FRAME_MS: u64 = 250;
/// Farther than this along her floor, she takes a door to the accordion.
const ERRAND_WALK: i32 = 2 * sprite::WIDTH;
/// What she says, poking it.
pub(super) const POKE: &str = line!("Somebody said something.");
/// What she says poking it, up groggy from her night's sleep.
pub(super) const SLEEPY_POKE: &str = line!("Mm... someone said...");

/// How long after the last (or the night act's start) she talks in her
/// sleep again (ms range, both ends included; phase 5b A14).
pub(super) const TALK_GAP_MS: (u64, u64) = (6 * 60_000, 10 * 60_000);
/// Her clock's last hour before she wakes, in real millis: the hour her
/// sleep-talk is "...five more minutes".
pub(super) const LAST_HOUR_MS: u64 = 60 * 60_000 / super::CLOCK_SPEED;
/// The Dream (step 7) comes this long after her first sleep of the
/// night, by her clock (game millis).
const DREAM_AFTER_MS: u64 = 30 * 60_000;
/// Back in bed after its moment passed while she was up (an errand, her
/// midnight snack, a visit's end), she sleeps this long (real millis)
/// before the Dream comes: not the instant she lies down.
pub(super) const DREAM_SETTLE_MS: u64 = 20_000;

/// How long after an act of hers begins a parcel may still come in
/// ([`Osaka::free_for_a_parcel`]): its slide and its flap (under a
/// second) are over long before the act's first 10 s, which the drawn
/// stillness rule leaves to settling in (phase 5c D7).
const PARCEL_AT_START_MS: u64 = 2_000;

/// How long until her `k`th sleep-talk after the one before it (the
/// 0th: after the night act's start), drawn from the night act's
/// starting decision's `whims`: a pure schedule, no draw from either
/// stream.
pub(super) fn talk_gap(whims: Whims, k: u64) -> u64 {
    let (lo, hi) = TALK_GAP_MS;
    lo + whims.below_at("sleep-talk-gap", k, hi - lo + 1)
}

/// The gap before her `k`th slow blink on a pose she holds (the 0th:
/// after her act's start), drawn from the whims of the decision that set
/// the act: a pure schedule, no draw from either stream.
pub(super) fn held_blink_gap(whims: Whims, k: u64) -> u64 {
    let (lo, hi) = HELD_BLINK_GAP_MS;
    lo + whims.below_at("held blink", k, hi - lo + 1)
}

/// Her night (phase 5b step 4b), from her first night act of it to the
/// morning: what of it carries across whatever gets her out of bed
/// before her wake time (an errand, her midnight snack), and her current
/// night act's sleep-talk, scheduled afresh as each begins (see
/// [`Osaka::arm_night`]). The guest keeps her last one across her visits
/// (see [`Osaka::carry_night`]): a night is hers, not a visit's.
#[derive(Clone, Copy, Debug)]
pub(super) struct Night {
    /// The game day the night ends on (its morning): what keys it.
    morning: u64,
    /// When she first slept this night, by her clock (game millis): the
    /// Dream's count runs from it, whatever got her up since.
    first: u64,
    /// Her midnight snack's moment by her clock (game millis), while
    /// tonight has one she hasn't had (at most one a night).
    snack: Option<u64>,
    /// The current night act's starting decision's whims, which its
    /// sleep-talk is drawn from.
    talk: Whims,
    /// Which sleep-talk of the night act is next (0 first).
    talk_k: u64,
    /// When it's due (monotonic millis).
    next_talk: u64,
    /// Tonight's Dream has come (or its moment passed with no room for
    /// it before she wakes): once a night.
    dreamt: bool,
}

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
        Use::Lounge => (27_000, 60_000),
        Use::Nap => (45_000, 105_000),
        Use::Sleep => (60_000, 180_000),
        Use::Homework => (37_000, 75_000),
        Use::Watch => (32_000, 82_000),
        Use::Unpack => (4_000, 6_000),
        Use::Read => (30_000, 65_000),
        Use::Snack => (6_000, 9_000),
        Use::Pet => (6_000, 9_000),
        Use::Crumple => (4_000, 6_000),
        // Her long daydream at the sill (phase 5c D6).
        Use::LookOut => (60_000, 180_000),
    }
}

/// Homework at homework time (her routine's slot) lasts this long (ms
/// range): the evening reads as homework with breaks.
const HOMEWORK_IN_SLOT_MS: (u64, u64) = (120_000, 240_000);

/// How long she keeps at `what` at her routine's `slot` (ms range;
/// `None`: no routine reaches her). The same one draw over either range:
/// a lengthened use draws no more than a usual one.
fn use_duration_in(what: Use, slot: Option<routine::Slot>) -> (u64, u64) {
    match (what, slot) {
        (Use::Homework, Some(routine::Slot::Homework)) => HOMEWORK_IN_SLOT_MS,
        _ => use_duration(what),
    }
}

/// The longest a still act she chooses can run at her routine's `slot`
/// (ms): any use (her day's sleep, today) with the longest prelude and
/// coda any splice row may wrap it in, anything she does on the spot, or
/// spacing out. The stillness band's visits must outlast it twice over
/// (phase 5c, B5), so a lengthened act shows there.
///
/// It reads the length tables themselves, lingered as the mood that
/// lingers most does in the levers she ships with ([`stillness::SHIPPED`]:
/// see [`longest_still_ms_with`]), so an act lengthened anywhere else
/// must draw its length through these tables, or this must read what it
/// draws through: else the band's guard can't see it. Settling in
/// (phase 5c M7) strings still acts together where she is into one still
/// stretch, so with any mood settling, the longest chain of them counts
/// as one act here.
#[cfg(test)]
pub(super) fn longest_still_ms(slot: Option<routine::Slot>) -> u64 {
    longest_still_ms_with(slot, &stillness::SHIPPED)
}

/// [`longest_still_ms`] at whatever time of day makes it longest
/// (routine or none): how long a test waits at most for her to be done
/// sitting still somewhere.
#[cfg(test)]
pub(super) fn longest_still_any_ms() -> u64 {
    routine::Slot::ALL
        .into_iter()
        .map(Some)
        .chain([None])
        .map(longest_still_ms)
        .max()
        .unwrap_or(0)
}

/// [`longest_still_ms`], with the levers `still`: each act her mood
/// lingers over as long as the most lingering mood's, and, if any mood
/// settles in, each chain she can settle along (spacing out or gazing,
/// then sitting, then lying back or dozing where she sits; lounging,
/// then napping on the same sofa; reading on her back, then dozing under
/// the book; leaning on her window's sill, then sitting in front of it,
/// then dozing there or watching the clouds) as long as its links
/// together.
#[cfg(test)]
pub(super) fn longest_still_ms_with(slot: Option<routine::Slot>, still: &Stillness) -> u64 {
    let linger = |lingers: bool, ms: u64| still.lingered_most(lingers, ms);
    let using = |what: Use| {
        splice_wrap_ms(what, script::Part::Before)
            + linger(what.lingers(), use_duration_in(what, slot).1)
            + splice_wrap_ms(what, script::Part::After)
    };
    let doing = |what: Activity| linger(what.lingers(), what.duration_in(slot).1);
    let space_out = linger(true, SPACE_OUT_MS.1);
    Use::ALL
        .map(using)
        .into_iter()
        .chain(Activity::ALL.map(doing))
        .chain([space_out])
        .chain(settle_chains_ms(slot, still))
        .max()
        .unwrap_or(0)
}

/// The longest chains she can settle along with the levers `still` at
/// her routine's `slot` (ms; none, if no mood settles in): spacing out
/// or gazing, then sitting, then lying back or dozing where she sits;
/// lounging, then napping on the same sofa (each with the longest
/// prelude and coda any splice row may wrap it in); reading on her
/// back, then dozing under the book; leaning on her window's sill, then
/// sitting in front of it, then dozing there or watching the clouds.
/// Each link is as long as its longest, lingered as the most lingering
/// mood lingers.
#[cfg(test)]
pub(super) fn settle_chains_ms(slot: Option<routine::Slot>, still: &Stillness) -> [u64; 4] {
    if !still.settles() {
        return [0; 4];
    }
    let linger = |lingers: bool, ms: u64| still.lingered_most(lingers, ms);
    let doing = |what: Activity| linger(what.lingers(), what.duration_in(slot).1);
    let using = |what: Use| {
        splice_wrap_ms(what, script::Part::Before)
            + linger(what.lingers(), use_duration_in(what, slot).1)
            + splice_wrap_ms(what, script::Part::After)
    };
    let into_sit = linger(true, SPACE_OUT_MS.1).max(doing(Activity::Gaze));
    let from_sit = doing(Activity::LieBack).max(doing(Activity::SitDoze));
    [
        into_sit + doing(Activity::Sit) + from_sit,
        using(Use::Lounge) + using(Use::Nap),
        doing(Activity::LieRead) + doing(Activity::BookDoze),
        using(Use::LookOut)
            + doing(Activity::UnderSill)
            + doing(Activity::SitDoze).max(doing(Activity::CloudWatch)),
    ]
}

/// The longest `part` (a prelude or a coda) any splice row may wrap a
/// use of `what` in (ms).
#[cfg(test)]
fn splice_wrap_ms(what: Use, part: script::Part) -> u64 {
    SpliceId::ALL
        .iter()
        .map(|id| id.row())
        .filter(|row| row.at == part && row.around.contains(&what))
        .flat_map(|row| row.lens.iter().copied())
        .max()
        .unwrap_or(0)
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
    /// Dozing off where she sits, her head sinking onto her knees: only
    /// ever settled into from sitting (phase 5c M7), never chosen.
    SitDoze,
    /// Her homework lying on the floor (phase 5c D5), where no desk
    /// stands: on her front writing on a paper out in front of her, or on
    /// her back reading the set text over her face, then nodding off
    /// where her mood has her (its own script: [`ScriptId::FloorHomework`]).
    FloorHomework,
    /// Reading lying on her back, the book held over her face (phase 5c
    /// D5), where no bookshelf stands.
    LieRead,
    /// Dozing under the book, open over her eyes: only ever settled into
    /// from reading on her back (phase 5c M7), never chosen.
    BookDoze,
    /// Sitting in front of her window, chin in her hands, looking up at
    /// the sky: only ever settled into from leaning on its sill (phase 5c
    /// D6), never chosen.
    UnderSill,
    /// Lying on her back under her window watching the clouds, eyes
    /// open, held: only ever settled into from sitting in front of it
    /// (phase 5c D6), never chosen, so never on a bare floor.
    CloudWatch,
}

impl Activity {
    /// Every activity: those she chooses, and those she only settles
    /// into ([`Activity::chosen`]).
    #[cfg(test)]
    pub const ALL: [Activity; 13] = [
        Self::Sit,
        Self::LieBack,
        Self::LieFront,
        Self::Jacks,
        Self::ToeTouch,
        Self::Stretch,
        Self::Gaze,
        Self::SitDoze,
        Self::FloorHomework,
        Self::LieRead,
        Self::BookDoze,
        Self::UnderSill,
        Self::CloudWatch,
    ];

    /// Whether she chooses it (it's a want of hers: [`Want::ALL`] lists
    /// it), rather than only settling into it as something she chose
    /// runs its course.
    #[cfg(test)]
    pub fn chosen(self) -> bool {
        match self {
            Self::Sit
            | Self::LieBack
            | Self::LieFront
            | Self::Jacks
            | Self::ToeTouch
            | Self::Stretch
            | Self::Gaze
            | Self::FloorHomework
            | Self::LieRead => true,
            Self::SitDoze | Self::BookDoze | Self::UnderSill | Self::CloudWatch => false,
        }
    }

    /// Whether it's rest (sitting, lying, gazing), not exercise: what
    /// doesn't answer restlessness. Resting in a pretty room eases her
    /// want of beauty.
    pub fn restful(self) -> bool {
        match self {
            Self::Sit
            | Self::LieBack
            | Self::LieFront
            | Self::Gaze
            | Self::SitDoze
            | Self::FloorHomework
            | Self::LieRead
            | Self::BookDoze
            | Self::UnderSill
            | Self::CloudWatch => true,
            Self::Jacks | Self::ToeTouch | Self::Stretch => false,
        }
    }

    /// Whether her mood lingers over its length (phase 5c M8: see
    /// [`Stillness::linger`]): sitting, lying back, gazing, dozing where
    /// she sits, reading on her back, dozing under the book. Not
    /// exercise, nor lying on her front kicking her feet (twice a second:
    /// a longer kick would draw the eye longer), nor her homework on the
    /// floor, whose nod-off her mood moves instead (as at her desk).
    pub fn lingers(self) -> bool {
        match self {
            Self::Sit
            | Self::LieBack
            | Self::Gaze
            | Self::SitDoze
            | Self::LieRead
            | Self::BookDoze
            | Self::UnderSill
            | Self::CloudWatch => true,
            Self::LieFront | Self::Jacks | Self::ToeTouch | Self::Stretch | Self::FloorHomework => {
                false
            }
        }
    }

    /// How long she keeps at it (ms range): her homework on the floor as
    /// long as at her desk, reading on her back as long as at her
    /// bookshelf.
    pub(super) fn duration(self) -> (u64, u64) {
        match self {
            Self::Sit => (15_000, 40_000),
            Self::LieBack => (20_000, 60_000),
            Self::LieFront => (10_000, 25_000),
            Self::Jacks => (4_000, 8_000),
            Self::ToeTouch => (5_000, 9_000),
            Self::Stretch => (2_000, 4_000),
            Self::Gaze => (6000, 14_000),
            // A doze, as long as she'd lie back.
            Self::SitDoze | Self::BookDoze => Self::LieBack.duration(),
            Self::FloorHomework => use_duration(Use::Homework),
            Self::LieRead => use_duration(Use::Read),
            // As long as she'd sit, or lie back.
            Self::UnderSill => Self::Sit.duration(),
            Self::CloudWatch => Self::LieBack.duration(),
        }
    }

    /// How long she keeps at it at her routine's `slot` (ms range;
    /// `None`: no routine reaches her): her homework on the floor at
    /// homework time as long as at her desk then.
    pub(super) fn duration_in(self, slot: Option<routine::Slot>) -> (u64, u64) {
        match self {
            Self::FloorHomework => use_duration_in(Use::Homework, slot),
            Self::Sit
            | Self::LieBack
            | Self::LieFront
            | Self::Jacks
            | Self::ToeTouch
            | Self::Stretch
            | Self::Gaze
            | Self::SitDoze
            | Self::LieRead
            | Self::BookDoze
            | Self::UnderSill
            | Self::CloudWatch => self.duration(),
        }
    }

    /// Animation frame period; 0 for a held pose. Reading on her back,
    /// a page turns as often as at her bookshelf; her homework on the
    /// floor is its script's.
    fn period(self) -> u64 {
        match self {
            Self::LieBack => 1400,
            Self::LieFront => 500,
            Self::Jacks => 450,
            Self::ToeTouch => 900,
            Self::LieRead => USE_FRAME_MS,
            Self::Sit
            | Self::Stretch
            | Self::Gaze
            | Self::SitDoze
            | Self::FloorHomework
            | Self::BookDoze
            | Self::UnderSill
            | Self::CloudWatch => 0,
        }
    }

    /// Its frame `elapsed` ms into an act `length` ms long: alternating
    /// on its period, and on a frame's period or slower (lying back,
    /// reading on her back) its last frame held through the part of a
    /// period before the act ends, as a script's bob is
    /// ([`script::bob_frame`]; the act starts on its grid); dozing where
    /// she sits, her head sinking (frame 0) for [`SIT_DOZE_NOD_MS`], then
    /// on her knees (frame 1) and held; gazing, "ooh" (frame 0) for
    /// [`GAZE_OOH_MS`], then quiet (frame 1). What alternates faster
    /// (exercise, kicking her feet) is motion through a short act, not a
    /// still one's change (the stillness rule is for still acts over 30 s;
    /// these are 25 s at most), so it runs to the end.
    /// A bob on the frame is held from `held` (ms into the act, and the
    /// frame then: [`script::bob_frame`]).
    fn frame(self, elapsed: u64, length: u64, held: Option<(u64, u8)>) -> u8 {
        let period = self.period();
        match self {
            Self::SitDoze => u8::from(elapsed >= SIT_DOZE_NOD_MS),
            Self::Gaze => u8::from(elapsed >= GAZE_OOH_MS),
            _ if self.bobs_on_the_frame() => script::bob_frame(elapsed, 0, length, period, held),
            _ => elapsed.checked_div(period).map_or(0, |n| (n % 2) as u8),
        }
    }

    /// Whether it bobs on the frame or slower (lying back, reading on
    /// her back): a still act's change, held as a script's bob is.
    fn bobs_on_the_frame(self) -> bool {
        !matches!(self, Self::SitDoze | Self::Gaze) && self.period() >= USE_FRAME_MS
    }

    fn look(self, frame: u8) -> (Pose, Face, Option<Bubble>) {
        match self {
            Self::Sit => (Pose::Sit, Face::Vacant, None),
            Self::LieBack => (Pose::LieBack(frame), Face::Blink, Some(Bubble::Zzz)),
            Self::LieFront => (Pose::LieFront(frame), Face::Happy, Some(Bubble::Hum)),
            Self::Jacks => (Pose::Jack(frame), Face::Happy, Some(Bubble::Count)),
            Self::ToeTouch => (Pose::ToeTouch(frame), Face::Vacant, None),
            Self::Stretch => (Pose::Stretch, Face::Blink, Some(Bubble::Stretch)),
            Self::Gaze if frame == 0 => (Pose::Gaze, Face::Curious, Some(Bubble::Ooh)),
            Self::Gaze => (Pose::Gaze, Face::Curious, None),
            Self::SitDoze => (Pose::SitDoze(frame), Face::Blink, Some(Bubble::Zzz)),
            // Its script poses her (this is its host: on her front,
            // writing).
            Self::FloorHomework => (Pose::FloorHomework(frame), Face::Vacant, None),
            Self::LieRead => (Pose::LieRead(frame), Face::Vacant, None),
            Self::BookDoze => (Pose::LieRead(2), Face::Blink, Some(Bubble::Zzz)),
            // Looking up at the sky, as she leaned on the sill.
            Self::UnderSill => (Pose::UnderSill, Face::Curious, None),
            // Eyes open, held: what she says of the sky is her one line.
            Self::CloudWatch => (Pose::CloudWatch, Face::Curious, None),
        }
    }
}

/// Dozing off where she sits: how long her head takes to sink onto her
/// knees.
const SIT_DOZE_NOD_MS: u64 = USE_FRAME_MS;
/// Gazing up, how long she says "ooh" before gazing on quietly (phase 5c
/// M8).
const GAZE_OOH_MS: u64 = 3_000;
/// The most musings on the sky she has leaning on her window's sill,
/// after its first line (phase 5c D6), whatever [`Stillness::sill`] says.
const SILL_MUSINGS: u8 = 3;

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
            Self::Dots => line!("..."),
            Self::Bang => line!("!"),
            Self::Huh => line!("?"),
            Self::Hehe => line!("hehe"),
            Self::Zzz => line!("zzz"),
            Self::Hum => line!("~"),
            Self::Count => line!("1, 2!"),
            Self::Stretch => line!("nnn~"),
            Self::Ooh => line!("ooh"),
            Self::Achoo => line!("a..."),
            Self::Chu => line!("chu!"),
            Self::Sparkle => line!("*'*"),
            Self::Say(text) => text,
        }
    }
}

/// A rule of her home she has felt broken, and the piece and use she
/// felt it on (none for one felt on sight: door batch, D7).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Felt {
    key: Grievance,
    on: Option<(Furniture, Use)>,
    /// She set about putting it right and let it go: it stays as it is
    /// this visit.
    let_go: bool,
}

/// Her look up at a chat line where she is, over a still act that runs
/// on (phase 5c B1; see [`Osaka::look_up`]): startled (`!`) for
/// [`LOOK_UP_SURPRISED_MS`] from the line, puzzled (`?`) to [`LOOK_MS`],
/// then watching, plain-faced, until `until` (the line's watch, never
/// before [`LOOK_UP_MS`]: a frame past the `?`). Tied to the
/// act: any other act has its own look ([`Osaka::set`] ends it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LookUp {
    /// When the look began: as the line came (the latest, in a lively
    /// chat), or, a line coming as she says what's wrong with her home,
    /// once she has said it (the look waits for it).
    since: u64,
    /// Since when what her act's script says is hidden and unsaid: the
    /// first look's, while each next comes before the last one's look is
    /// over (what the act said under them all is said after the last).
    hidden: u64,
    /// Her watch's end, as the line set it (never before a frame past
    /// the look's `?`).
    until: u64,
    /// The look's next moment to handle: 0, out of her startle; 1, the
    /// look over (what her act said under it said now); 2, the watch
    /// over.
    step: u8,
    /// Her facing to turn back to once it's over: a piece's seat's (she
    /// sits to it), or none (where she turned, she stays turned).
    back: Option<Facing>,
}

impl LookUp {
    /// When its next moment is.
    fn due(&self) -> u64 {
        match self.step {
            0 => self.since + LOOK_UP_SURPRISED_MS,
            1 => self.since + LOOK_MS,
            _ => self.until,
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
    /// The moment of her last tick: her next slow blink's edge is the
    /// first after it ([`Osaka::wakes_at`]).
    ticked: u64,
    /// Whether a slow blink showed at her last tick, so the next tick
    /// can say whether how she looks changed.
    blink_shown: bool,
    /// When her last look up from her act ended, if she has looked up:
    /// a slow blink begun before then, begun under it, shows none of its
    /// remainder.
    look_ended: Option<u64>,
    /// While a chat conversation continues she stands watching it.
    watch_until: u64,
    watch_x: i32,
    /// Looking up at the chat where she is, over a still act.
    looking_up: Option<LookUp>,
    /// The musings on the sky she has still to say leaning on her
    /// window's sill (phase 5c D6; see [`Osaka::sill_on`]): her look-out's
    /// session, which goes with her act (see [`Osaka::set`]).
    sill: Option<Session>,
    /// What her act's script said under her look, still to say once
    /// what she's saying now is over, in turn (see
    /// [`Osaka::say_what_her_look_hid`]). Anything else she says, or
    /// hushing her, lets it go.
    unsaid: Vec<&'static str>,
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
    /// Her part-time job's shift under way (on her way out to it, or
    /// back from it).
    shift: Option<Shift>,
    /// She's been to work this visit (once is plenty).
    worked: bool,
    /// She came in this visit through her door where it stood out of
    /// its space because her pieces fill it (`Fallback::Yield`: door
    /// batch M13): she bumped into what stands there. Set from the first
    /// moment she's back out of it (home from school, a dash, work, the
    /// stage), never going out; what lets her feel her DoorClear rule on
    /// sight (`on_sight`, mod.rs).
    bumped: bool,
    /// Nowhere clear of her pieces to go out by, since she last went out
    /// ([`Osaka::nowhere_clear`]): her routine's tries again are logged
    /// quieter.
    kept_in: bool,
    /// She's leaving by her routine (A9): through her door, whose gap
    /// never ends (`u64::MAX`, nothing drawn), or out at the screen's
    /// edge for good. Set only once her external door opens (door batch
    /// D6, M1): nothing on her way to it can end the visit. The guest
    /// ends the visit once she's through and it has closed behind her
    /// ([`Osaka::gone_out`]). Never a [`Shift`]; cleared by
    /// [`Osaka::place`] and by an errand.
    leaving: Option<Routine>,
    /// She has set off for her door by her routine (door batch D6): her
    /// line as she sets off is said once (the latch); re-entering her
    /// way out (a hop's landing, a walk come to nothing, a look at the
    /// chat) says nothing. Cleared where `leaving` is, and when school's
    /// over before she's out.
    set_off: Option<Routine>,
    /// She's coming home by her routine, out of her door: at its end,
    /// she says so ([`Osaka::home_from`]).
    returning: Option<Routine>,
    /// Her routine cut her breakfast short: she's late, and says so as
    /// she leaves.
    late: bool,
    needs: Needs,
    /// Her mood this visit.
    mood: Mood,
    /// Her stillness levers (phase 5c): how her mood lingers and settles,
    /// her daydreams' musings, where her homework nods off, whether
    /// nearer spots draw her. [`stillness::SHIPPED`]; tests set others.
    pub(super) stillness: Stillness,
    /// How many times she has settled further into the still act she's
    /// at (0: she chose it): see [`Osaka::settle_in`].
    settled: u8,
    /// Her back aches from her homework on the floor (phase 5c D5): set
    /// for the visit the first time one ends, when she says so; from then
    /// on a desk of her own making draws her the more.
    pub(super) ached: bool,
    /// Her last few choices (repeating herself is discouraged).
    recent: Vec<Want>,
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
    /// What was rising her needs at the last tick (or decision): text on
    /// offer to tidy, a rule of her home felt broken, a plain room. Her
    /// needs rise by it until they're next brought up (see
    /// [`Osaka::rise_to`]); `slept_ms` is kept apart.
    rising: Rising,
    /// The time her needs are current to (see [`Osaka::rise_to`]): when
    /// she last decided.
    decided: u64,
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
    /// The rules (rows of the table) she felt broken on sight and owes
    /// saying so, once she's quiet (door batch, D7), oldest first.
    owed_aloud: Vec<usize>,
    /// What she's saying of what's wrong with her home, felt on sight,
    /// and since when: for [`GRIEVANCE_MS`], standing (the act it's said
    /// in; any other act ends it).
    aloud: Option<(&'static str, u64)>,
    /// Every such line she said, and when (tests).
    #[cfg(test)]
    aloud_said: Vec<(&'static str, u64)>,
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
    /// The times she set off (see [`Osaka::count_set_off`]; the census
    /// reads it).
    #[cfg(test)]
    pub set_offs: u32,
    /// Each time she set off, what for and how (the census reads it).
    #[cfg(test)]
    pub set_off_log: Vec<SetOff>,
    /// The chat lines that stopped her (not those she only stirred at,
    /// answered playing on, or that lapsed while she was out of sight or
    /// aloft), as she was stopped: when, whether she only passed on over
    /// text, and what she was moving for if she was (one that came while
    /// she was out of sight or aloft stops her as she next decides and
    /// watches, and cut what she was on when it came; see
    /// [`Osaka::chat_owed`]). The census reads it.
    #[cfg(test)]
    pub chat_cuts: Vec<(u64, bool, Option<&'static str>)>,
    /// The chain her moving belongs to, for the census: the method of the
    /// decision that set it going, or what set it going without one
    /// ("arrival", "accident"). See [`Osaka::census_purpose`].
    #[cfg(test)]
    chain: &'static str,
    /// Deciding (inside [`Osaka::decide`]), and whether this decision has
    /// set her off yet.
    #[cfg(test)]
    deciding: Option<bool>,
    /// A chat line stopped her (whether she only passed on, what she was
    /// moving for), and she hasn't done anything but look and watch
    /// since: her next set-off is a restart after it.
    #[cfg(test)]
    after_chat: Option<(bool, Option<&'static str>)>,
    /// A chat line came while she was out of sight, in a door or aloft
    /// (what it cut): it stops her as she next decides, if she watches
    /// it then (the census reads it as a look then).
    #[cfg(test)]
    chat_owed: Option<Option<&'static str>>,
    /// How each heading went: set off, arrived, or let go and why
    /// (tests read it).
    #[cfg(test)]
    pub headings: Vec<String>,
    /// Her last few decisions, and why (the stage shows them).
    log: std::collections::VecDeque<Decision>,
    /// Every choice she made (tests read it).
    #[cfg(test)]
    pub choices: Vec<Want>,
    /// Every want she chose and then found no way to after all (tests
    /// read it).
    #[cfg(test)]
    pub stranded: Vec<Want>,
    /// Every factor her choices weighed a want by (the chat's, her
    /// day's, the season's; not her inertia), with whether it would have
    /// taken her into the chat (tests read it).
    #[cfg(test)]
    pub factored: Vec<(Want, bool, f64)>,
    /// Every decision she made (tests read it).
    #[cfg(test)]
    pub decisions: Vec<Decision>,
    /// Each share of what she chose she was credited with as she left
    /// doing it (see [`Osaka::credit_done`]), and when (tests read it).
    #[cfg(test)]
    pub credited: Vec<(Want, f64, u64)>,
    /// Every easing of her needs for what she did ([`Osaka::serve`],
    /// which every credit goes through). Tests read it.
    #[cfg(test)]
    pub served: Vec<Served>,
    /// All the night she has slept that her needs rose by as slept
    /// (`slept_ms`, as each pass took it). Tests read it.
    #[cfg(test)]
    pub slept_total: u64,
    /// Only this want, by this method, is on offer as she chooses (and
    /// what she was heading for, if it's that want): tests drive her
    /// choices with it. Reflexes and continuations are as ever.
    #[cfg(test)]
    pub offer_only: Option<(Want, &'static str)>,
    /// Her routine's clock and the day's vacation latch, as the guest
    /// gave them at the last tick's entry (`None`: the routine doesn't
    /// reach her, and she behaves as before the clock). Every decision
    /// reads it at its own moment ([`Osaka::day`]).
    clock: Option<routine::Clock>,
    /// When her current act began (monotonic millis): set by
    /// [`Osaka::set`], the one way an act begins.
    act_since: u64,
    /// When her current act began by her clock (game millis since her
    /// start), read as it began: from the clock then, never mapped back
    /// through a later reading (a capped step moves every earlier
    /// moment, see [`Osaka::refresh_cut`]). `None` until a clock has
    /// reached her during the act (the first one stamps it).
    act_since_game: Option<u64>,
    /// The next boundary of her routine that cuts into what she's doing
    /// (school, bed: [`routine::Slot::cuts`]), in monotonic millis: the
    /// first after her act began and after the last one handled (see
    /// [`Osaka::refresh_cut`]). `None` while no routine reaches her.
    cut_at: Option<u64>,
    /// That boundary by her clock (game millis), whose handling
    /// [`Osaka::cut`] records in `cut_past`.
    cut_game: u64,
    /// The last cutting boundary handled, by her clock (game millis; 0
    /// before any): handled, it's never found again, cutting or not,
    /// however her clock is read afterwards.
    cut_past: u64,
    /// How long she has slept the night since her needs last moved on
    /// (A11): the next [`Needs::pass`] takes it (see
    /// [`Osaka::credit_done`]).
    slept_ms: u64,
    /// Until when her night act has been counted into `slept_ms` (it's
    /// settled twice as she leaves it, by the decision and by the act
    /// that follows: once is counted).
    slept_to: u64,
    /// The lamp is off for the night (round-1b): turned off by her night
    /// act's lamp key in the night (by her routine), so whatever takes
    /// her out of bed in the night happens in the dark. (Bedtime alone
    /// never turns it off: she does, settling in.) Only ever honoured in
    /// the night (see [`Osaka::dark`]), and cleared as a new day begins
    /// (woken into, or caught up on), so a night missed or cut short
    /// never darkens a day, nor the next night before she settles.
    lamp_off: bool,
    /// Until when she's stirring, turned over, at a chat line in the
    /// night (see [`Osaka::look`]).
    stir_until: u64,
    /// Her bob held where something of hers changed in place off its
    /// grid, in this act: the moment (monotonic) and the frame it showed
    /// then ([`script::bob_frame`], set by [`Osaka::hold_bob`]).
    bob_held: Option<(u64, u8)>,
    /// Until when she's saying good morning (see [`Osaka::begin_day`]),
    /// or that she's home (from school or work), or what's wrong with
    /// her home that she felt on sight ([`Osaka::say_aloud`]): what would
    /// speak over it waits (see [`Osaka::awake`]).
    morning_until: u64,
    /// Her home's master seed: her mornings key her new day's mood on it
    /// and the game day ([`brain::day_seed`]).
    master: u64,
    /// The game day her line budget is that day's (`None`: no routine
    /// reaches her, and it's the visit's).
    budget_day: Option<u64>,
    /// The earliest her clock reads (game millis): its reading before
    /// the last time it lost time (a step the shell's cap clamped: a
    /// suspend). A catch-up decision whose time that step left far behind
    /// would otherwise map back, through the new reading, to before the
    /// old one.
    game_floor: u64,
    /// Up groggy from her night's sleep (Q2): an errand took her out of
    /// bed in the night. She blinks her way about until she's back in it
    /// (read only through [`Osaka::groggy_at`]: never past the night).
    groggy: bool,
    /// When her day began, by her clock (game millis): her clock's first
    /// reading this visit, or the wake she woke into or was up at. Her
    /// next morning after it begins a new day (see
    /// [`Osaka::catch_up_day`]).
    day_from: Option<u64>,
    /// Her night, while it's night by her routine and she has slept
    /// some of it (see [`Night`]); cleared as a new day begins.
    night: Option<Night>,
    /// Her night as an earlier visit left it (the guest's: see
    /// [`Osaka::carry_night`]): her first night act this visit carries it
    /// on if it's the same night, so what's once a night (the Dream, her
    /// midnight snack) stays once, and the Dream's count runs on.
    night_before: Option<Night>,
    /// The fridge she's up for her midnight snack from, from the moment
    /// it gets her up until she has had it (see
    /// [`Osaka::midnight_snack`], [`Osaka::got_what_she_came_for`]). The
    /// piece, not where it stood: she finds it afresh each time she sets
    /// off for it (see [`fridge_seat`]), and gives up on it once it's
    /// gone.
    snacking: Option<PieceRef>,
    /// She dashed home from school for something she forgot (phase 5b
    /// D3a): set as she comes in, kept until she has it (or has stood
    /// wondering what it was, or its fridge has gone); then her routine's
    /// away reflex sends her out again. See [`Osaka::dash_in`].
    dash: Option<Dash>,
    /// Her calendar (phase 5b D5): the real date whose owed entry she
    /// has delivered (it showed), as her ledger had it when her visit
    /// began and as she delivers one since. Owed once a day, it's
    /// checked at each decision against the date she's given
    /// ([`Osaka::calendar_owed`]), so a resident on screen as the date
    /// changes gets the new day's.
    cal_done: Option<chrono::NaiveDate>,
    /// Her calendar's entry under way: said or begun, not yet shown (or
    /// said, and never seen). It's delivered as it shows
    /// ([`Osaka::shown`]), not as it's set: spoken over or cut short
    /// before it showed, it's owed again (said again once she has moved
    /// from where it went unseen: see [`Osaka::calendar_greeting`]).
    cal_showing: Option<CalShowing>,
    /// How many times this visit her calendar's line for a date was on
    /// her, she in sight and painted, but not in her drawn bubble (no
    /// room for it beside her, or what she says of her home over it): at
    /// [`CAL_MISSES`] she lets it be until her next visit, still owed.
    cal_missed: Option<(chrono::NaiveDate, u8)>,
    /// New Year's Day's first sunrise on her TV, after its greeting has
    /// shown: this visit's, best effort (the day was delivered as the
    /// greeting showed). Her calendar's beat sends her to her TV for it
    /// once; any watch of hers that date plays it.
    sunrise: Option<SunriseOwed>,
    /// The game day and slot of her routine whose first snack she has
    /// said her meal's line at ("Breakfast!", "Dinner time~"): this
    /// visit's, or an earlier one's that day (the guest carries it on).
    meal_said: Option<(u64, routine::Slot)>,
    /// Up on a day with no school: "No school today!", once she's quiet,
    /// that morning.
    day_off: bool,
    /// Her wall clock where it hangs, as the frame last offered it (see
    /// [`Chances::clock`]): what she glances up at.
    clock_on: Option<ClockOn>,
    /// The slot of her routine she has had her glance at the clock in
    /// (or her night begun in), by its end (game millis): her bed and
    /// school reflexes glance first only once a slot, however often they
    /// send her (back from her midnight snack, an errand).
    clock_glanced: Option<u64>,
    /// She has glanced up at the clock of an afternoon this visit (once
    /// a visit).
    hour_glanced: bool,
    /// What's rare and open (phase 5b D6): her day's, drawn as the visit
    /// began or as she woke into the day (see [`Osaka::begin_day`]), or
    /// unfed the visit's. Nothing before a draw.
    rares: Rares,
    /// The game day `rares` was drawn for (`None`: unfed, the visit's),
    /// so a day is drawn once, however it began.
    rares_day: Option<u64>,
    /// The rare scripts she has shown (her ledger's as the visit began,
    /// and those she has shown since).
    seen: Vec<ScriptId>,
    /// Her pity counters as the guest last gave them (and reset here as
    /// she shows something new): a new day's draw reads them.
    pity: Pity,
}

/// Her calendar's owed entry for `date`, said or begun, waiting to show
/// (see [`Osaka::shown`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CalShowing {
    date: chrono::NaiveDate,
    owed: Owed,
    what: CalShows,
    /// Where she was as she said or began it.
    spot: (i32, i32),
    /// A frame missed it: she was in sight with it on her, and it wasn't
    /// in her drawn bubble (counted once, in [`Osaka::cal_missed`]).
    missed: bool,
}

/// How many sayings of her calendar's line may go unseen on screen (a
/// frame missing it) before she lets it be for the visit (see
/// [`Osaka::cal_missed`]): a pane too full of text for her bubble
/// wherever she goes mustn't have her say it all visit.
const CAL_MISSES: u8 = 3;

/// New Year's Day's first sunrise, owed on her TV this visit (see
/// [`Osaka::sunrise`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SunriseOwed {
    /// The date it's owed on (none after it: a resident into Jan 2 has
    /// missed it).
    date: chrono::NaiveDate,
    /// Her calendar's beat has sent her to her TV for it (it sends her
    /// once).
    sent: bool,
}

/// How her calendar's entry shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CalShows {
    /// Its greeting, as she says it.
    Line(&'static str),
    /// Its script, as it plays.
    Script(ScriptId),
}

/// Setsubun's beans, on the spot (her calendar's, Feb 3): eight throws.
pub(super) const SETSUBUN_MS: u64 = 8 * script::THROW_MS;

/// Where she is in a dash home from school (phase 5b D3a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dash {
    /// Just in: what she forgot is settled as she first decides (her
    /// lunch, from her fridge if she can get to it; else she can't
    /// remember).
    In,
    /// For her lunch from her fridge (the piece, found afresh each time
    /// she sets off for it: see [`fridge_seat`]): kept until she has had
    /// it (see [`Osaka::got_what_she_came_for`]), so whatever stops her on
    /// the way or at it, she sets off again; gone, she gives up on it.
    Lunch(PieceRef),
}

/// Her lunch from her fridge, dashed home for: the door open a moment,
/// then "Forgot my lunch!" (see [`ScriptId::DashLunch`]).
pub(super) const DASH_LUNCH_MS: u64 = 3800;
/// Dashed home with no fridge to get to: standing, "Forgot somethin'..."
/// then "...what was it?" (see [`ScriptId::DashForgot`]).
pub(super) const DASH_FORGOT_MS: u64 = 3000;

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
            ticked: now,
            blink_shown: false,
            look_ended: None,
            watch_until: 0,
            looking_up: None,
            sill: None,
            unsaid: Vec::new(),
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
            shift: None,
            worked: false,
            bumped: false,
            kept_in: false,
            leaving: None,
            set_off: None,
            returning: None,
            late: false,
            needs: Needs::default(),
            mood: Mood::Ordinary,
            stillness: stillness::SHIPPED,
            settled: 0,
            ached: false,
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
            rising: Rising::default(),
            owed: Vec::new(),
            lines: Lines::default(),
            felt: Vec::new(),
            owed_aloud: Vec::new(),
            aloud: None,
            #[cfg(test)]
            aloud_said: Vec::new(),
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
            set_offs: 0,
            #[cfg(test)]
            set_off_log: Vec::new(),
            #[cfg(test)]
            chat_cuts: Vec::new(),
            #[cfg(test)]
            chain: "arrival",
            #[cfg(test)]
            deciding: None,
            #[cfg(test)]
            after_chat: None,
            #[cfg(test)]
            chat_owed: None,
            #[cfg(test)]
            headings: Vec::new(),
            log: std::collections::VecDeque::new(),
            #[cfg(test)]
            choices: Vec::new(),
            #[cfg(test)]
            stranded: Vec::new(),
            #[cfg(test)]
            factored: Vec::new(),
            #[cfg(test)]
            decisions: Vec::new(),
            #[cfg(test)]
            credited: Vec::new(),
            #[cfg(test)]
            served: Vec::new(),
            #[cfg(test)]
            slept_total: 0,
            #[cfg(test)]
            offer_only: None,
            clock: None,
            act_since: now,
            act_since_game: None,
            cut_at: None,
            cut_game: 0,
            cut_past: 0,
            slept_ms: 0,
            slept_to: 0,
            lamp_off: false,
            stir_until: 0,
            bob_held: None,
            morning_until: 0,
            master: 0,
            budget_day: None,
            game_floor: 0,
            groggy: false,
            day_from: None,
            night: None,
            night_before: None,
            snacking: None,
            dash: None,
            cal_done: None,
            cal_showing: None,
            cal_missed: None,
            sunrise: None,
            meal_said: None,
            day_off: false,
            clock_on: None,
            clock_glanced: None,
            hour_glanced: false,
            rares: Rares::none(),
            rares_day: None,
            seen: Vec::new(),
            pity: Pity::default(),
        };
        osaka.whims = Whims(osaka.mind.0 ^ mind::WHIMS_SALT);
        osaka.act_due = osaka.first_due(now);
        // Her arrival's act (a walk in, a door) is a set-off from
        // nowhere: no `set` starts it. What for is her arrival's, or
        // what the constructor that made her says (see
        // [`Osaka::census_arrived_for`]).
        #[cfg(test)]
        osaka.count_set_off(false, now);
        osaka
    }

    /// The census: her arrival came for what she's flagged with now (an
    /// errand, her routine, a dash home), set after [`Osaka::new`], so
    /// her arrival's set-off is for it too.
    #[cfg(test)]
    fn census_arrived_for(&mut self) {
        let purpose = self.census_purpose();
        for set_off in &mut self.set_off_log {
            set_off.purpose = purpose;
        }
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

    /// Test fixture: stand still where she is from `now` to `until`.
    #[cfg(test)]
    pub fn stand_still(&mut self, until: u64, now: u64) {
        self.set(Act::Stand { until }, now);
    }

    /// Test fixture: walk along her floor to column `to` from `now`,
    /// choosing again once there.
    #[cfg(test)]
    pub fn walk_to(&mut self, to: i32, now: u64) {
        self.set(
            Act::Walk {
                to,
                then: Then::Nothing,
            },
            now,
        );
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

    /// When the act she's at began (a pull or a borrow begins again at
    /// each of its phases).
    #[cfg(test)]
    pub fn act_started(&self) -> u64 {
        self.act_since
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
    pub fn act_summary(&self) -> String {
        let name = self.act_name();
        match &self.act {
            Act::Walk { to, then } => match then {
                Then::Job(job) => {
                    let what = match job.by_ref() {
                        JobRef::Pull(_) => "a pull".to_owned(),
                        JobRef::Swap(_) => "a swap".to_owned(),
                        JobRef::Build(build) => format!("making a {:?}", build.piece.item),
                        JobRef::Borrow(_) => "a strip to read".to_owned(),
                        JobRef::Use(seat) => format!("{:?} ({:?})", seat.what, seat.item),
                        JobRef::Lift(l) => format!("lifting the {}", l.repair.piece.spec().name),
                        JobRef::SetDown(s) => format!("setting the {} down", s.piece.spec().name),
                        JobRef::Leave(_, why) => format!("her door ({why:?})"),
                        JobRef::Out(_, why) => format!("out where it's clear ({why:?})"),
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

    /// What she keeps of the frame's `chances` between frames: how
    /// pretty the room she stands in is, and where her wall clock hangs.
    /// Taken in at each tick, and by the stage before it directs her (a
    /// scene played before her tick has seen the frame sees it as it is).
    pub fn take_in(&mut self, chances: &Chances, terrain: &Terrain, now: u64) {
        self.beauty_here = chances.beauty_here;
        self.clock_on = chances.clock;
        self.rising = self.rising_in(chances);
        self.door_follows(
            chances.door,
            chances.door_through,
            (terrain, &chances.obstacles),
            now,
        );
    }

    /// Her external door, opened and not yet done with (its last beat
    /// not over), follows where the frame stands it now (door batch D6,
    /// C5, M8; `through`, the frame's door read without the focused
    /// pane, from the opened door's spot): a resize mid-gap and she
    /// comes back out where it stands. A focused pane over it is never a
    /// reason to move it (that door is read without the focus; the act
    /// keeps its spot and [`Osaka::evict`] handles the focus). With no
    /// door anywhere now, it's a door in space at her feet: a door at a
    /// stale spot can't be represented.
    ///
    /// Her door is drawn at her feet (`Door::of`), so in every beat it's
    /// shown she moves with it (door batch, step 6d: a resize as it stood
    /// open left it drawn where her pieces now stand). On its near side
    /// (going out) she stands at it facing out of the room, as
    /// [`Osaka::through_her_door`] stood her; let out at its far side
    /// (its `there` beats, her coming in and the door closing behind
    /// her) she's out of it into the room ([`Osaka::out_of`]: facing in,
    /// and bumped if its space is filled). In the gap between (nothing
    /// drawn) she's out of sight; her feet go with it all the same, so a
    /// door gone then is a door in space where it last stood.
    ///
    /// Called by the frame as it reads her door (`seen`, and `through`,
    /// read without the focused pane), so nothing drawn or judged that
    /// frame sees the door where it stood before a resize; and again by
    /// her tick (`take_in`: the same chances, a no-op then).
    ///
    /// Her door (`Act::Door`) turned to `to` with `gap` between its
    /// doors, at `now`, in the beat it stands in (as far in, see
    /// [`retimed`]): the one way a door's far side or gap changes once
    /// it's begun (door batch, step 8 review), so a door of hers turned
    /// into one in space (or back) by whatever turns it never jumps a
    /// beat, and her wakes follow its new beats. Nothing but a door.
    fn redirect_door(&mut self, to: Through, gap: u64, now: u64) {
        let Act::Door {
            since,
            to: was,
            gap: had,
        } = &mut self.act
        else {
            return;
        };
        *since = retimed(*since, now, (*was, *had), (to, gap));
        (*was, *had) = (to, gap);
        self.act_due = self.first_due(now);
    }

    /// Out at work by a door in space (a focused pane moved her door's
    /// far side, or there was no door of hers to go to; door batch M10):
    /// while she's out of sight between its doors, where the frame stands
    /// her door now is where she comes home, if it stands anywhere and
    /// no focused pane is over it (then she comes home where she went:
    /// step 4b review). Only a shift's door (its gap above 0): a hop
    /// between floors on her way there, or an errand's door, is never
    /// one.
    pub(super) fn door_follows(
        &mut self,
        seen: Option<DoorSpot>,
        through: Option<DoorSpot>,
        (terrain, obstacles): (&Terrain, &[Rect]),
        now: u64,
    ) {
        let (x, y) = (self.x, self.y);
        let at_work = self.shift == Some(Shift::Out);
        let Act::Door { since, to, gap } = self.act else {
            return;
        };
        let Some((beat, _)) = door_beat(now.saturating_sub(since), gap, to) else {
            return;
        };
        let there = beat.there;
        let going = !there && beat.door.is_some();
        let door = match to {
            Through::Home(door) => door,
            Through::Space(spot) => {
                let out = !there && beat.door.is_none();
                // Only where the frame stands her door as it is seen: a
                // focused pane over it (the door she'd come through, read
                // with the focus unprotected, isn't the frame's) keeps
                // her coming home where she went, clear of the focus.
                if let Some(fresh) =
                    through.filter(|_| at_work && gap > 0 && out && seen == through)
                {
                    tracing::debug!(from = ?spot, to = ?fresh.spot(), "houseguest: home from work by her door");
                    self.redirect_door(Through::Home(fresh), gap, now);
                    return;
                }
                // A way out of hers (its gap: school's, the stage's, her
                // shift's) was judged clear of her pieces as it opened;
                // a resize or a delivery since can lay one over it, so
                // it's judged again against each frame it's drawn in:
                // going out, and coming in (where she comes home, as its
                // far door first shows; door batch, step 10a review). In
                // its gap nothing of it is drawn. A door between floors
                // or an errand's (no gap) isn't a way out.
                let drawn = beat.door.is_some();
                if gap > 0 && drawn && Clear::of(spot, obstacles).is_none() {
                    self.way_out_in_space(spot, gap, going, (terrain, obstacles), now);
                }
                return;
            }
        };
        // Where the frame stands her door now, focus and all: not stale.
        // Read without the focus it may stand elsewhere (in its space,
        // under the focused pane she opened it beside), but a focused
        // pane is never a reason to move it.
        if seen == Some(door) {
            return;
        }
        match through {
            Some(fresh) if fresh != door => {
                tracing::debug!(from = ?door.spot(), to = ?fresh.spot(), there, "houseguest: her door moved while she's through it");
                self.redirect_door(Through::Home(fresh), gap, now);
                if there {
                    self.out_of(Through::Home(fresh));
                } else {
                    (self.x, self.y) = fresh.spot();
                    self.facing = fresh.out();
                }
            }
            Some(_) => {}
            // A door in space where it stood (door batch, step 10a: a
            // resize can lay a piece of hers there; see
            // `way_out_in_space`).
            None => {
                tracing::debug!(from = ?door.spot(), there, "houseguest: her door's gone while she's through it");
                self.way_out_in_space((x, y), gap, going, (terrain, obstacles), now);
            }
        }
    }

    /// The door she's through (her door gone, or a way out in space a
    /// piece has been laid over) now opens in space from `from`: there if
    /// her pieces (`obstacles`, as the frame lays them) leave it clear,
    /// else at the nearest place they do ([`nearest_clear`]), she with it
    /// in every beat (it's drawn at her feet, and she comes in where it
    /// stands; door batch, step 10a). Kept in its beat, with its `gap`.
    ///
    /// With nowhere clear on any floor: going out by it (`going`, its
    /// near beats), she isn't out after all ([`Osaka::not_out_after_all`]:
    /// no way out opens on a piece). Out of sight, or coming in, it stays
    /// where it stood (she's out, and must come in somewhere), judged
    /// again each frame until there's room.
    fn way_out_in_space(
        &mut self,
        from: (i32, i32),
        gap: u64,
        going: bool,
        (terrain, obstacles): (&Terrain, &[Rect]),
        now: u64,
    ) {
        let clear = Clear::of(from, obstacles).or_else(|| {
            nearest_clear(
                terrain,
                terrain.platform_at(from.0, from.1),
                from,
                obstacles,
            )
        });
        match clear {
            Some(clear) => {
                if clear.spot() != from {
                    tracing::debug!(?from, to = ?clear.spot(), "houseguest: her way out moves clear of her pieces");
                }
                self.redirect_out(clear, gap, now);
                (self.x, self.y) = clear.spot();
            }
            None if going => self.not_out_after_all(now),
            None => self.redirect_door(Through::Space(from), gap, now),
        }
    }

    /// Her door turned to a door in space at `clear` (see
    /// [`Osaka::redirect_door`]): the one way a way out of hers is turned
    /// into one in space, only where her pieces leave it clear.
    fn redirect_out(&mut self, clear: Clear, gap: u64, now: u64) {
        self.redirect_door(Through::Space(clear.spot()), gap, now);
    }

    /// Going out by a door of hers with nowhere clear of her pieces to
    /// open it (door batch, step 10a review): she isn't out after all.
    /// What going out set ([`Osaka::leave_by`]) is undone (school's
    /// `leaving`, the stage's `returning`; her shift is let go), and she
    /// stays in for now ([`Osaka::nowhere_clear`]).
    fn not_out_after_all(&mut self, at: u64) {
        let why = self.not_out_by(at);
        self.nowhere_clear(why, at);
    }

    /// What going out by the door she's through set ([`Osaka::leave_by`])
    /// undone: school's `leaving`, the stage's `returning`, her shift let
    /// go ([`Osaka::let_work_go`]). Returns what she was going out for.
    fn not_out_by(&mut self, at: u64) -> Leave {
        let why = if self.leaving.is_some() {
            Leave::School
        } else if self.returning.is_some() {
            Leave::Stage
        } else {
            Leave::Work
        };
        // Work's first, while she's still through its door: the share
        // she worked of it (none, going out).
        if why == Leave::Work {
            self.let_work_go(at);
        }
        self.leaving = None;
        self.returning = None;
        why
    }

    /// The door of hers she's through at `now`, in any of its beats
    /// (going out, out of sight, or coming back in by it): what the frame
    /// reads her door from for `Chances::door_through`, so the door that
    /// [`Osaka::door_follows`] moves is always read from its own spot
    /// (D6), never from the frame's last door. `None` once its last beat
    /// is over.
    pub fn opened_door(&self, now: u64) -> Option<DoorSpot> {
        match self.act {
            Act::Door {
                since,
                to: Through::Home(door),
                gap,
            } if door_beat(now.saturating_sub(since), gap, Through::Home(door)).is_some() => {
                Some(door)
            }
            _ => None,
        }
    }

    /// What rises her needs in `chances`' frame (nothing slept).
    fn rising_in(&self, chances: &Chances) -> Rising {
        Rising {
            mess: !chances.pulls.is_empty(),
            grieved: self.grieved(chances),
            plain: chances.beauty_here <= 0.0,
            slept_ms: 0,
        }
    }

    /// Her needs brought up to `at`: each rises with the time since they
    /// last were (by what was rising them, her mood and her routine's
    /// pace; the night she slept at its own). Done as she decides, and
    /// before anything eases her ([`Osaka::serve`]), so an easing lands
    /// on her needs as they are.
    fn rise_to(&mut self, at: u64) {
        self.count_sleep(at);
        let rising = Rising {
            slept_ms: std::mem::take(&mut self.slept_ms),
            ..self.rising
        };
        #[cfg(test)]
        {
            self.slept_total += rising.slept_ms.min(at.saturating_sub(self.decided));
        }
        let slot = self.day(at).map(|day| day.slot);
        self.needs
            .pass(at.saturating_sub(self.decided), rising, self.mood, |need| {
                brain::clock_rate(slot, need)
            });
        self.decided = self.decided.max(at);
    }

    /// Asleep for the night until `at`, that time is kept for her needs
    /// (A11), counted once however often it's asked.
    fn count_sleep(&mut self, at: u64) {
        if self.sleeping() {
            let from = self.act_since.max(self.slept_to);
            self.slept_ms = self.slept_ms.saturating_add(at.saturating_sub(from));
            self.slept_to = self.slept_to.max(at);
        }
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
            Act::Borrow { since, phase, .. } => match phase {
                Borrowing::Brace => since + BRACE_MS,
                Borrowing::Reel(_) | Borrowing::Slide(_) => since + REEL_MS,
                // On her reading's frames.
                Borrowing::Read { until } => {
                    let frames = now.saturating_sub(since) / USE_FRAME_MS + 1;
                    (since + frames * USE_FRAME_MS).min(until)
                }
            },
            // An activity's own script wakes her as each key ends, and
            // on a key's frames while it bobs (her homework on the floor,
            // writing); a held one (her night on the floor) only as it
            // ends. Any other activity, on its frames.
            Act::Idle {
                play: Some(play),
                since,
                until,
                ..
            } => {
                let key_end = play.next_end(since, until, now).unwrap_or(until);
                key_end.min(play.next_move(since, until, now)).min(until)
            }
            Act::Idle {
                what, since, until, ..
            } => next_frame(what, since, now).min(until),
            // As each key of a riddle ends.
            // A daydream session's next musing.
            Act::SpaceOut {
                since,
                until,
                play,
                session,
            } => play
                .and_then(|play| play.next_end(since, until, now))
                .unwrap_or(until)
                .min(session.map_or(until, |s| s.next))
                .min(until),
            // On the frame grid from the start of the part playing (the
            // prelude, the body or the coda: what bobs moves on it, timed
            // as the part times them), and on static's quicker frames
            // while it shows (the hook moves at paint time), as each key
            // ends, so the next one's look, line and prop come on on
            // time, and as she starts and stops saying what's wrong with
            // her home (felt the moment it has all shown).
            Act::Use {
                since,
                until,
                play,
                grievance,
                ..
            } => {
                let grid = play
                    .next_frame(since, until, now, USE_FRAME_MS)
                    .min(play.next_move(since, until, now));
                let key_end = play.next_end(since, until, now).unwrap_or(until);
                let grumble = grievance
                    .into_iter()
                    .flat_map(|(_, from)| [from, from + GRIEVANCE_MS])
                    .find(|&t| t > now)
                    .unwrap_or(until);
                // Her next musing at the sill.
                let muse = self.sill.map_or(until, |s| s.next);
                grid.min(key_end).min(grumble).min(muse).min(until)
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
            // As each beat ends; through her own door, at each of her
            // steps through its doorway too (her side-on door draws them,
            // `wall_beat`), or nothing repaints mid-step.
            Act::Door { since, to, gap } => match door_at(now.saturating_sub(since), gap, to) {
                Some((_, beat, start, end)) => {
                    let end = since.saturating_add(end);
                    if beat.step && matches!(to, Through::Home(_)) {
                        let into = now.saturating_sub(since).saturating_sub(start);
                        let step = since
                            .saturating_add(start)
                            .saturating_add((into / WALK_MS + 1) * WALK_MS);
                        step.min(end)
                    } else {
                        end
                    }
                }
                None => now,
            },
            Act::Look {
                surprised_until, ..
            } => surprised_until,
            Act::Walk { .. } => now + WALK_MS,
            Act::Pull { ref pull, .. } => now + heave_ms(pull.cells.len()) / 2,
            Act::Climb { .. } => now + CLIMB_MS,
            Act::Fall { from_y, since, .. } => fall_time(since, (self.y - from_y + 1) as u64),
        }
    }

    /// When she next needs a tick: anything [`Osaka::due`], or the next
    /// edge of a slow blink on a pose she holds (which changes only how
    /// she looks, so it's no event of hers: see [`Osaka::tick`]).
    pub fn wakes_at(&self) -> u64 {
        self.next_held_blink()
            .map_or(self.due(), |blink| blink.min(self.due()))
    }

    /// When her pose, speech, or the text layer next changes, or her
    /// routine next cuts into what she's doing.
    pub fn due(&self) -> u64 {
        let hush = self.speech.map_or(u64::MAX, |(_, until)| until);
        let cut = self.cut_at.unwrap_or(u64::MAX);
        let talk = self.talk_due().unwrap_or(u64::MAX);
        let snack = self.snack_due().unwrap_or(u64::MAX);
        let dream = self.dream_due().unwrap_or(u64::MAX);
        let look_up = self.looking_up.map_or(u64::MAX, |look| look.due());
        self.pose_due()
            .min(look_up)
            .min(self.pending_due())
            .min(hush)
            .min(cut)
            .min(talk)
            .min(snack)
            .min(dream)
    }

    /// When tonight's Dream comes (monotonic millis): 30 game minutes
    /// after she first slept tonight, counted across whatever got her up
    /// since (and, its moment passed while she was up, a little after
    /// she's back asleep: [`DREAM_SETTLE_MS`]), once a night, only while
    /// she's asleep for it, and only on a day it's open.
    fn dream_due(&self) -> Option<u64> {
        self.night
            .filter(|night| !night.dreamt && self.sleeping())?;
        if !self.rares.allows(ScriptId::Dream) {
            return None;
        }
        Some(
            self.dream_at()?
                .max(self.act_since.saturating_add(DREAM_SETTLE_MS)),
        )
    }

    /// When she next talks in her sleep (monotonic millis): only while
    /// she's asleep for the night by her routine.
    pub(super) fn talk_due(&self) -> Option<u64> {
        self.night
            .filter(|_| self.sleeping())
            .map(|night| night.next_talk)
    }

    /// When her midnight snack gets her up (monotonic millis): only
    /// while she's asleep for the night, and tonight has one she hasn't
    /// had.
    fn snack_due(&self) -> Option<u64> {
        let snack = self.night.filter(|_| self.sleeping())?.snack?;
        Some(self.clock?.game.when(snack))
    }

    /// Say `text` for a while, over whatever bubble her act shows (and
    /// over what she was saying: see [`Osaka::hush`]).
    pub fn say(&mut self, text: &'static str, now: u64) {
        tracing::debug!(text, "houseguest says");
        self.hush(now);
        self.hold_bob(now);
        self.speech = Some((text, now + speech_ms(text)));
    }

    /// Stop saying what she's saying. A pooled line she started saying
    /// this very instant never showed (nothing is drawn between), so it
    /// isn't said: its record goes, and it doesn't cool. (A door line
    /// the decision after the door speaks over, say.) A stir's turn ends
    /// with its murmur (phase 5c's tail, T4): it's only ever woken for
    /// and held at the murmur's end, so cut short, it ends here too.
    ///
    /// Her bob isn't held here, for what's cut short: every caller holds
    /// or starts afresh at the same moment. [`Osaka::say`] holds; a new
    /// act ([`Osaka::set`]: a cued Setsubun, riddle or rare musing, a
    /// glance at her clock, a cued prelude's use, being placed) clears
    /// the hold, its bob starting on its own grid; and the Dream
    /// ([`Osaka::dream_from`]) begins its own part.
    fn hush(&mut self, now: u64) {
        self.unsaid.clear();
        self.stir_until = self.stir_until.min(now);
        if let Some((text, until)) = self.speech.take()
            && until == now + speech_ms(text)
        {
            self.lines.unsay(text, now);
        }
    }

    /// Say `line`, from `pool`, at `at`: never drawn (said as it's due),
    /// but said from it, so it cools there.
    fn say_noted(&mut self, pool: PoolId, line: &'static str, at: u64) {
        self.say(line, at);
        self.lines.note(pool, line, at);
    }

    /// Say `pool`'s line (a pool of one, said as it's due) at `at`.
    fn say_pool(&mut self, pool: mind::Pool, at: u64) {
        if let Some(&line) = pool.lines.first() {
            self.say_noted(pool.id, line, at);
        }
    }

    /// Space out, telling a riddle (one musing in three) or saying one
    /// of her musings, each drawn from her latest decision's whims (in
    /// summer's panic week or December, one of the season's first, now
    /// and then). A riddle only when she isn't saying something already,
    /// which would hide its question. On a day it's open, first, now and
    /// then, her rare musing (the escalator), quiet too; then, of an
    /// afternoon, with her wall clock where she can see it, now and then
    /// (once a visit, quiet too) a glance up at it, saying the hour,
    /// roughly. Cued to by the stage, Setsubun's beans, a glance at the
    /// clock, the escalator, or a riddle, instead: nothing rolled takes a
    /// cued one's place.
    pub fn muse(&mut self, now: u64, rng: &mut Rng) {
        // Cued, on the spot, whatever she was saying stopping for it:
        // Setsubun's beans; a glance up at her clock (whatever the hour,
        // three o'clock without her clock to tell her, and wherever her
        // clock: turned toward it if she can see it); her rare musing
        // (it cools as if rolled).
        match self.cued {
            Some(Cue::Script(ScriptId::Setsubun)) => {
                self.cued = None;
                self.hush(now);
                return self.set(Self::setsubun(now), now);
            }
            Some(Cue::Script(ScriptId::ClockGlance)) => {
                self.cued = None;
                if let Some((glance, x)) = self.hour_glance(now, true, true) {
                    return self.glance_up(glance, x, now);
                }
            }
            Some(Cue::Script(ScriptId::Escalator)) => {
                self.cued = None;
                self.hush(now);
                self.lines.try_play(ScriptId::Escalator, now);
                return self.wonder(now, rng);
            }
            _ => {}
        }
        // A riddle cued (below) is what this musing is: nothing she'd
        // roll comes first.
        let rolls = self.cued != Some(Cue::Script(ScriptId::Riddle));
        // Her rare musing: on a day it's open (checked before anything of
        // it rolls), one musing in three while she's quiet, and not again
        // within its ten minutes. It takes the musing's place, as long as
        // one.
        let quiet = self.speech.is_none_or(|(_, until)| until <= now);
        if rolls
            && self.rares.allows(ScriptId::Escalator)
            && quiet
            && self.whims.chance("rare-musing", 0, 1, 3)
            && self.lines.try_play(ScriptId::Escalator, now)
        {
            return self.wonder(now, rng);
        }
        // Of an afternoon, her wall clock where she can see it: the first
        // daydream she starts quiet (once a visit, unrolled) has her
        // glance up at it, saying the hour, roughly ("Three-ish."). It
        // takes the musing's place.
        if rolls && let Some((glance, x)) = self.hour_glance(now, quiet, false) {
            return self.glance_up(glance, x, now);
        }
        // Her daydream session's musings (phase 5c B6): as many as her
        // mood's whim says. With none, she only spaces out: no riddle or
        // musing rolls. (Her rare musing and a glance at her clock, above,
        // aren't a musing of hers: they keep to their own odds in every
        // mood, and hold no more.)
        let (lo, hi) = self.stillness.musings.of(self.mood);
        let musings = lo
            + self
                .whims
                .below("daydream", u64::from(hi.saturating_sub(lo)) + 1) as u8;
        if rolls && musings == 0 {
            tracing::debug!("houseguest: a daydream with nothing to say");
            return self.set(
                Act::SpaceOut {
                    since: now,
                    until: now + self.lingered(true, rng.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1)),
                    play: None,
                    session: None,
                },
                now,
            );
        }
        // Cued, a riddle: whatever she was saying stops for it.
        let cued = !rolls;
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
                let whims = self.whims;
                self.say_musing(whims, now);
                None
            }
        };
        // A riddle is all the session she has; a musing has the rest of
        // hers to come, the next a whim's gap on.
        let session = (play.is_none() && musings > 1).then(|| Session {
            left: musings - 1,
            next: now + self.musing_gap(self.whims, 0),
            said: 1,
            whims: self.whims,
        });
        self.set(
            Act::SpaceOut {
                since: now,
                until: now + self.lingered(true, rng.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1)),
                play,
                session,
            },
            now,
        );
    }

    /// Say a musing at `now`, drawn from `whims`: the season's (her
    /// panic before exams, December's), else one of her musings; nothing
    /// if every one of them is cooling.
    fn say_musing(&mut self, whims: Whims, now: u64) {
        let tints = self.tints();
        let seasonal = [(tints.panic, mind::PANIC), (tints.december, mind::DECEMBER)];
        let line = seasonal
            .into_iter()
            .filter(|&(on, _)| on)
            .find_map(|(_, pool)| self.lines.pick(pool, whims, now))
            .or_else(|| self.lines.pick(mind::MUSINGS, whims, now));
        if let Some(line) = line {
            self.say(line, now);
        }
    }

    /// The gap after a daydream session's `k`th musing (counting from 0),
    /// drawn from its decision's `whims`.
    fn musing_gap(&self, whims: Whims, k: u8) -> u64 {
        let (lo, hi) = self.stillness.musing_gap;
        lo + whims
            .series("daydream", u64::from(k))
            .below("gap", hi.saturating_sub(lo) + 1)
    }

    /// When a musing due at `at` (her daydream's, or at her sill) may be
    /// said: once what she's saying and her look up at the chat are over,
    /// and never within a frame after her look's end (a change of hers
    /// waits a frame from a look's: phase 5c's tail, T4's sibling); at its
    /// very end, it shows with it. `at` itself when nothing holds it.
    fn free_to_muse(&self, at: u64) -> u64 {
        let speaking = self.speech.map_or(0, |(_, until)| until);
        let looking = self.looking_up.map_or(0, |look| look.until);
        // Never looked up: nothing to settle from.
        let settling = match self.look_ended {
            Some(ended) if at > ended && at < ended + USE_FRAME_MS => ended + USE_FRAME_MS,
            _ => 0,
        };
        speaking.max(looking).max(settling)
    }

    /// Her daydream session's next musing is due at `at` (phase 5c B6):
    /// said now, the `said`th, drawn from the session's whims, unless
    /// she's saying something or looking up at the chat, when it waits
    /// for that to be over, or her look ended less than a frame ago
    /// ([`Osaka::free_to_muse`]). The last one said, the session is over:
    /// she spaces out to the end.
    fn muse_on(&mut self, session: Session, at: u64) {
        let busy = self.free_to_muse(at);
        let next = if busy > at {
            Some(Session {
                next: busy,
                ..session
            })
        } else {
            tracing::debug!(said = session.said, "houseguest: muses on");
            self.say_musing(
                session.whims.series("daydream", u64::from(session.said)),
                at,
            );
            (session.left > 1).then(|| Session {
                left: session.left - 1,
                next: at + self.musing_gap(session.whims, session.said),
                said: session.said + 1,
                whims: session.whims,
            })
        };
        if let Act::SpaceOut { session, .. } = &mut self.act {
            *session = next;
        }
    }

    /// Her glance up at her clock of an afternoon (see
    /// [`Osaka::hour_glance`]), at column `x` (where she stands, if she
    /// can't see it), from `now`: once a visit.
    fn glance_up(&mut self, glance: ClockGlance, x: Option<i32>, now: u64) {
        tracing::debug!(?glance, "houseguest: a glance at her clock");
        self.hour_glanced = true;
        self.glance_at_clock(glance, x.unwrap_or(self.x), now);
    }

    /// Her rare musing, spacing out from `now` (see [`Osaka::muse`]):
    /// which one's the escalator?
    fn wonder(&mut self, now: u64, rng: &mut Rng) {
        tracing::debug!("houseguest: which one's the escalator");
        self.set(
            Act::SpaceOut {
                since: now,
                until: now + self.lingered(true, rng.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1)),
                play: Some(Play::plain(ScriptId::Escalator)),
                session: None,
            },
            now,
        );
    }

    /// Whether she glances up at her clock at `now`, spacing out (see
    /// [`Osaka::muse`]; `quiet`: she isn't saying anything; `cued`: the
    /// stage cued it, and she does, saying whatever hour her clock says,
    /// three o'clock without one): the glance, and the clock's column if
    /// it's where she can see it. Unrolled (phase 5c step 8c): of an
    /// afternoon, once a visit, while she's quiet and can see her clock.
    fn hour_glance(&self, now: u64, quiet: bool, cued: bool) -> Option<(ClockGlance, Option<i32>)> {
        let x = self.clock_on.and_then(|c| c.seen_from((self.x, self.y)));
        let day = self.day(now);
        if cued {
            let hour = day.map_or(15, |day| day.minute / 60);
            return Some((ClockGlance::Hour(hour), x));
        }
        let hour = day
            .filter(|day| day.slot == routine::Slot::Afternoon)
            .map(|day| day.minute / 60)?;
        // No roll (phase 5c step 8c, the user's): her daydreams are long
        // and few now, so any she starts quiet glances, once a visit.
        let glance = !self.hour_glanced && quiet && x.is_some();
        glance.then_some((ClockGlance::Hour(hour), x))
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

    /// Her routine's clock from now on (see [`Osaka::day`]): set at
    /// every tick's entry, and when the stage skips her clock. Her next
    /// cutting boundary is found afresh from it, and her night's wake
    /// time (a later day's vacation flag is provisional until that day
    /// latches: what's predicted into it is predicted again each time),
    /// read at the monotonic millis `now`.
    pub fn read_clock(&mut self, clock: Option<routine::Clock>, now: u64) {
        // Time lost between the readings (a step the shell's cap clamped:
        // a suspend): no moment from here on reads earlier than the last
        // reading did, as this one, extended back, would.
        if let (Some(was), Some(now)) = (self.clock, clock)
            && now.game.game < was.game.at(now.game.at)
        {
            self.game_floor = self.game_floor.max(was.game.game);
        }
        self.clock = clock;
        if let Some(clock) = clock
            && self.day_from.is_none()
        {
            self.day_from = Some(clock.game.game.max(self.game_floor));
        }
        if clock.is_some() {
            // An act begun before any clock reached her: when it began
            // by this first reading, held from now on.
            let since = self.act_since;
            if self.act_since_game.is_none() {
                self.act_since_game = self.game_at(since);
            }
        }
        self.refresh_cut();
        self.refresh_night(now);
    }

    /// Her clock at the monotonic millis `at` (game millis since her
    /// start), never earlier than the last tick's reading (see
    /// `game_floor`): `None` while no routine reaches her.
    fn game_at(&self, at: u64) -> Option<u64> {
        self.clock
            .map(|clock| clock.game.at(at).max(self.game_floor))
    }

    /// Find her next cutting boundary ([`routine::Slot::cuts`]) from her
    /// clock: the first after both her act's start and the last one
    /// handled. From the act's start, not from now, so a catch-up that
    /// falls behind still cuts the act that was running at the boundary;
    /// past the last handled, so a handled one is never due again (no
    /// tick spins on it). Always later than both.
    ///
    /// Both floors are her clock's own times, recorded as they happened:
    /// only the boundary found is mapped to monotonic time, forward,
    /// through the clock as read now. Never history backward through it:
    /// a step the shell's cap clamps (a suspend) puts every earlier
    /// monotonic moment further back in her day than it was, and a
    /// boundary handled would be found again and cut again.
    fn refresh_cut(&mut self) {
        let Some(clock) = self.clock else {
            self.cut_at = None;
            return;
        };
        let began = self
            .act_since_game
            .unwrap_or_else(|| clock.game.at(self.act_since));
        let boundary = clock.next_cutting(began.max(self.cut_past));
        self.cut_game = boundary;
        self.cut_at = Some(clock.game.when(boundary));
    }

    /// When her night's sleep begun at `began` (game millis) ends: the
    /// wake time, in monotonic millis, if it began in the night (by her
    /// routine); `None` for one begun by day (the stage's), or with no
    /// routine reaching her.
    fn wake_of(&self, began: u64) -> Option<u64> {
        let clock = self.clock?;
        (clock.day_of(began).slot == routine::Slot::Asleep)
            .then(|| clock.game.when(clock.next_wake(began)))
    }

    /// Her night's sleep lasts until her wake time, found afresh from her
    /// clock as it's read now (from when it began by her clock, never
    /// mapped back): a later day's flag may have changed since it began.
    /// Read at `now`.
    fn refresh_night(&mut self, now: u64) {
        let Some(began) = self.act_since_game.filter(|_| self.sleeping()) else {
            return;
        };
        let Some(wake) = self.wake_of(began) else {
            return;
        };
        // Her breathing holds as the end moves: the frame it shows now,
        // read before the end moves.
        let frame = self.bob_frame_at(now);
        let changed = match &mut self.act {
            Act::Use {
                since,
                until,
                whole,
                play,
                ..
            } => {
                let was = *until;
                *until = wake.max(play.body_start(*since));
                *whole = until.saturating_sub(play.body_start(*since));
                *until != was
            }
            Act::Idle { since, until, .. } => {
                let was = *until;
                *until = wake.max(*since);
                *until != was
            }
            _ => false,
        };
        if changed {
            tracing::trace!(wake, "houseguest: her wake time moved");
            self.act_due = self.act_due.min(wake);
            if let Some(frame) = frame {
                self.hold_frame(now, frame);
            }
        }
    }

    /// Her routine reached a cutting boundary at `at` (school, bed):
    /// what she's resting at or busy with stops (A10), and she decides
    /// again, where her routine sends her (see [`Osaka::send_to_bed`]).
    /// Whatever she's doing, the boundary is handled: the next one is
    /// found, and nothing fires early.
    ///
    /// Only an act she stays at (resting, or at a job: [`Act::props`]'s
    /// table) is cut, and a walk to such a job (its end starts the job
    /// without a decision). Never what she passes through or what runs
    /// its course on its own, which that table says she doesn't stay at:
    /// her errand's poke, a climb, a fall, a door, being out (they end
    /// soon, in a decision). And at bedtime, never her sleep: asleep in her bed
    /// (or already asleep for the night, as the stage put her), she
    /// sleeps on through the night, in place.
    fn cut(&mut self, at: u64) -> bool {
        let bedtime = self
            .clock
            .is_some_and(|clock| clock.day_of(self.cut_game).slot == routine::Slot::Asleep);
        let school = self
            .clock
            .is_some_and(|clock| clock.day_of(self.cut_game).slot == routine::Slot::Away);
        // Handled before anything it sets off: whatever she decides
        // now searches past it.
        self.cut_past = self.cut_game;
        if bedtime && self.sleep_on(at) {
            self.refresh_cut();
            return true;
        }
        // On her way to a job she'd stay at: at its spot she'd start it
        // with no decision (her routine's reflex unasked), so it's cut
        // as the job itself would be.
        // Never her way out by her door (door batch D6, T11): at her
        // door, she re-checks her routine herself.
        let (to_job, snack) = match &self.act {
            Act::Walk {
                then: Then::Job(job),
                ..
            } if job.leave().is_none() => (
                true,
                matches!(job, Job::Use(seat) if seat.what == Use::Snack),
            ),
            Act::Use { seat, .. } => (false, seat.what == Use::Snack),
            _ => (false, false),
        };
        let mut changed = false;
        if school {
            // Her breakfast cut short, or her way to it: she's late (she
            // says so, going).
            self.late = snack;
            // At work (a stage's cue: her job is open only on days off),
            // on her way or back: on to school, never home with her
            // shopping.
            if self.shift.is_some() {
                self.cut_shift(at);
                changed |= self.school_from_work(at);
            }
        }
        let cuttable = matches!(self.act.props().stays, Stays::Rest | Stays::Job) || to_job;
        if cuttable {
            tracing::info!(at, act = %self.act_summary(), "houseguest: her routine cuts in");
            self.interrupt(Cause::Routine, at);
        } else {
            tracing::trace!(at, act = %self.act_summary(), "houseguest: her routine passes");
        }
        self.refresh_cut();
        cuttable || changed
    }

    /// School begins at `at` while she's at work, or on her way there or
    /// back (A9; door batch M9): she goes on to school from where she
    /// is, with no shift. On her way to her door for work (walking or
    /// heading there), her way out is school's now: she goes on out
    /// through it, silently (her set-off latched), and `leaving` is set
    /// only as it opens (never on her way, M1). Through her door for
    /// work, it never opens again (the guest ends the visit at once). In
    /// sight coming back out of it, her next decision sends her out.
    /// Returns whether her act was altered (her door's gap), so the
    /// screen could change.
    fn school_from_work(&mut self, at: u64) -> bool {
        tracing::info!("houseguest: from work, on to school");
        let mut on_her_way = false;
        if let Act::Walk {
            then: Then::Job(Job::Leave { why, .. } | Job::Out { why, .. }),
            ..
        } = &mut self.act
        {
            *why = Leave::School;
            on_her_way = true;
        }
        if let Some(Heading {
            job: Job::Leave { why, .. } | Job::Out { why, .. },
            ..
        }) = &mut self.heading
        {
            *why = Leave::School;
            on_her_way = true;
        }
        if on_her_way {
            self.set_off = Some(Routine::School);
            return false;
        }
        let through = match self.act {
            Act::Door { since, to, gap } => {
                door_beat(at.saturating_sub(since), gap, to).is_some_and(|(beat, _)| !beat.there)
            }
            _ => false,
        };
        match self.act {
            Act::Door { to, .. } if through => self.redirect_door(to, u64::MAX, at),
            _ => return false,
        }
        self.leaving = Some(Routine::School);
        self.act_due = self.first_due(at);
        true
    }

    /// At bedtime (`at`), asleep in her bed, or asleep for the night by
    /// day: that sleep becomes her night's, in place (A10), lasting until
    /// her wake time. No interruption, no new act: the sleep's keys stay
    /// where they were (the night's bed branches are timed as a day's
    /// sleep's), and only the night from bedtime on counts as slept.
    /// False for anything else.
    fn sleep_on(&mut self, at: u64) -> bool {
        let night = match &self.act {
            Act::Use { seat, play, .. } => {
                (seat.what == Use::Sleep && play.own == ScriptId::Sleep) || play.own.is_night()
            }
            Act::Idle { play, .. } => play.is_some_and(|p| p.own.is_night()),
            _ => false,
        };
        let Some(wake) = self
            .clock
            .map(|clock| clock.game.when(clock.next_wake(self.cut_game)))
        else {
            return false;
        };
        if !night {
            return false;
        }
        // Her breathing holds as her sleep's end moves to her wake time
        // (the frame it shows now).
        self.hold_bob(at);
        match &mut self.act {
            Act::Use {
                since,
                until,
                whole,
                play,
                ..
            } => {
                if play.own == ScriptId::Sleep {
                    // Lamp on a moment, or (a trial's) off at once.
                    play.branch = Surface::Bed.branch(play.branch != 0);
                    play.own = ScriptId::Night;
                }
                play.after = None;
                *until = wake.max(play.body_start(*since));
                *whole = until.saturating_sub(play.body_start(*since));
            }
            Act::Idle { since, until, .. } => *until = wake.max(*since),
            _ => {}
        }
        // From bedtime, as her clock read it then: her wake time is found
        // from it from now on.
        self.act_since_game = Some(self.cut_game);
        self.slept_to = at;
        self.act_due = self.first_due(at);
        self.latch_lamp(at);
        self.arm_night(at);
        tracing::info!(at, act = %self.act_summary(), "houseguest: asleep for the night");
        true
    }

    /// Her night's sleep, if that's her act (A4): what it plays, and its
    /// span. Whatever she sleeps on, it's one act.
    fn night_play(&self) -> Option<(Play, u64, u64)> {
        match self.act {
            Act::Use {
                play, since, until, ..
            } if play.own.is_night() => Some((play, since, until)),
            Act::Idle {
                play: Some(play),
                since,
                until,
                ..
            } if play.own.is_night() => Some((play, since, until)),
            _ => None,
        }
    }

    /// Whether she's asleep for the night: her act is her night's sleep
    /// (A4), whatever she sleeps on, whose time asleep her needs keep
    /// apart (see `slept_ms`).
    pub(super) fn sleeping(&self) -> bool {
        self.night_play().is_some()
    }

    /// Her next cutting boundary (tests).
    #[cfg(test)]
    pub(super) fn cut_at(&self) -> Option<u64> {
        self.cut_at
    }

    /// The night's sleep kept for her needs' next pass (tests).
    #[cfg(test)]
    pub(super) fn slept_ms(&self) -> u64 {
        self.slept_ms
    }

    /// All the night's sleep counted for her needs, whether a pass has
    /// taken it yet or not (tests): each moment once.
    #[cfg(test)]
    pub(super) fn slept_counted(&self) -> u64 {
        self.slept_total + self.slept_ms
    }

    /// Where the door she's going through lets her out, if she's going
    /// through one (tests).
    #[cfg(test)]
    pub fn through(&self) -> Option<Through> {
        match self.act {
            Act::Door { to, .. } => Some(to),
            _ => None,
        }
    }

    /// Her own door, if she's going out through it at `now`: standing at
    /// it on its near side (not yet let out at its far side), the act's
    /// door hers (tests: a door in space re-pointed at her door while
    /// she's out, M10, never counts).
    #[cfg(test)]
    pub fn out_by_her_door(&self, now: u64) -> Option<DoorSpot> {
        match self.act {
            Act::Door {
                since,
                to: Through::Home(door),
                gap,
            } if (self.x, self.y) == door.spot()
                && door_beat(now.saturating_sub(since), gap, Through::Home(door))
                    .is_some_and(|(beat, _)| !beat.there) =>
            {
                Some(door)
            }
            _ => None,
        }
    }

    /// Her own door, if she's coming in by it at `now`: let out at its
    /// far side (its `there` beats), the door shown, the act's door hers.
    #[cfg(test)]
    pub fn in_by_her_door(&self, now: u64) -> Option<DoorSpot> {
        match self.act {
            Act::Door {
                since,
                to: Through::Home(door),
                gap,
            } if door_beat(now.saturating_sub(since), gap, Through::Home(door))
                .is_some_and(|(beat, _)| beat.there && beat.door.is_some()) =>
            {
                Some(door)
            }
            _ => None,
        }
    }

    /// The routine she's leaving by, if she is.
    pub(super) fn leaving(&self) -> Option<Routine> {
        self.leaving
    }

    /// Whether she has said hello, or good morning (tests).
    #[cfg(test)]
    pub(super) fn greeted(&self) -> bool {
        self.greeted
    }

    /// A beat line said at `at`, as if she'd drawn it (tests: her line
    /// budget spent).
    #[cfg(test)]
    pub(super) fn note_beat(&mut self, at: u64) {
        self.lines.note(mind::PoolId::Beat, "Hm.", at);
    }

    /// Her routine at the monotonic millis `at`: `None` while the clock
    /// doesn't reach her. The one way anything of hers asks the time.
    pub fn day(&self, at: u64) -> Option<DayTime> {
        let clock = self.clock?;
        self.game_at(at).map(|game| clock.day_of(game))
    }

    /// Run every event due by `now`, with her routine's `clock` (`None`:
    /// none reaches her). Returns whether her pose changed.
    pub fn tick(
        &mut self,
        now: u64,
        clock: Option<routine::Clock>,
        terrain: &Terrain,
        chances: &Chances,
        rng: &mut Rng,
    ) -> bool {
        self.read_clock(clock, now);
        self.take_in(chances, terrain, now);
        let mut changed = false;
        for _ in 0..64 {
            let due = self.due();
            if due > now {
                self.note_seen(now);
                return self.blinks_by(now) || changed;
            }
            // Her routine first: a boundary is handled before anything
            // else due with it, and never fires her act.
            if self.cut_at == Some(due) {
                changed |= self.cut(due);
                continue;
            }
            // Her night's own moments, while she sleeps it: nothing to
            // show if no line came, or no fridge got her up. The Dream
            // first: her sleep-talk waits for it.
            if self.dream_due() == Some(due) {
                changed |= self.dream(due);
                continue;
            }
            if self.talk_due() == Some(due) {
                changed |= self.sleep_talk(due);
                continue;
            }
            if self.snack_due() == Some(due) {
                changed |= self.midnight_snack(due, terrain, chances, rng);
                continue;
            }
            changed = true;
            if let Some((_, until)) = self.speech
                && until == due
            {
                self.speech = None;
                self.hold_bob(due);
                // What her act said under her look, said on in turn.
                if let Some((&next, rest)) = std::mem::take(&mut self.unsaid).split_first() {
                    self.say(next, due);
                    self.unsaid = rest.to_vec();
                }
                continue;
            }
            if self.looking_up.is_some_and(|look| look.due() == due) {
                self.looking_up_moves(due);
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
            // Nodded off under her look: a doze wears none (a line now
            // would only stir her).
            if self.looking_up.is_some() && self.acting(due).0.dozes() {
                tracing::trace!("houseguest: nodded off under her look; it's over");
                self.end_look(due);
            }
            // A rare script's first key, shown as it begins.
            self.note_seen(due);
            if self.latch_lamp(due) {
                self.say_pool(mind::NIGHT_NIGHT, due);
            }
            debug_assert!(self.undoes_its_mischief(), "mischief without its undo");
        }
        // Far behind (a suspended laptop): resume from now.
        self.act_due = self.act_due.max(now);
        self.next_blink = self.next_blink.max(now);
        self.note_seen(now);
        self.blinks_by(now) || changed
    }

    /// Her tick at `now` done: whether a slow blink on a pose she holds
    /// began or ended since her last (how she looks changed, though no
    /// event of hers came: it's drawn from a pure schedule, see
    /// [`Osaka::held_blink`]).
    fn blinks_by(&mut self, now: u64) -> bool {
        let shown = self.held_blink(now);
        let changed = shown != self.blink_shown;
        self.blink_shown = shown;
        self.ticked = self.ticked.max(now);
        changed
    }

    /// Her slow blinks' starts on a pose she holds (phase 5c Q3), as her
    /// act runs: the `k`th the gaps `0..=k` after it began, each drawn
    /// from the whims of the decision that set it (her whims change only
    /// as she decides, which sets her act: so acts one decision chains,
    /// a walk and then the seat, a settling in, restart the same gaps
    /// from their own starts). None but in a still act with an end she
    /// holds a pose in (sitting, gazing, a use), and none past its end.
    fn held_blinks(&self) -> impl Iterator<Item = u64> + use<> {
        let until = match self.act {
            Act::Idle { until, .. } | Act::Use { until, .. } => until,
            _ => 0,
        };
        let whims = self.whims;
        (0..)
            .scan(self.act_since, move |at, k| {
                *at += held_blink_gap(whims, k);
                Some(*at)
            })
            .take_while(move |&at| at < until)
    }

    /// What shows of a slow blink beginning at `start` (from, to): any
    /// only if she holds her pose there with her eyes open
    /// ([`Pose::holds`]), and none of it while she looks up at the chat
    /// (her look is all attention): a look come mid-blink ends it, and
    /// one begun before it hides it all, its end too.
    fn held_window(&self, start: u64) -> Option<(u64, u64)> {
        let (pose, face, _) = self.look_at(start);
        if !pose.holds() || !matches!(face, Face::Vacant | Face::Curious) {
            return None;
        }
        let end = match self.looking_up {
            Some(look) if look.since <= start => return None,
            Some(look) => look.since.min(start + BLINK_MS),
            None => start + BLINK_MS,
        };
        (start >= self.look_ended.unwrap_or(0)).then_some((start, end))
    }

    /// Whether she's in a slow blink at `now` (see [`Osaka::held_blinks`]).
    fn held_blink(&self, now: u64) -> bool {
        self.held_blinks()
            .find(|&start| now < start + BLINK_MS)
            .and_then(|start| self.held_window(start))
            .is_some_and(|(from, to)| from <= now && now < to)
    }

    /// The next edge (a start or an end) of a slow blink that shows, after
    /// her last tick and before her next event ([`Osaka::due`]): an edge
    /// past it would lose to it in [`Osaka::wakes_at`] and is found again
    /// at its tick. So the search is a few blinks long however long her
    /// act (a night's sleep) or its end (none, for an act without one).
    ///
    /// An act with no end and no event coming (none has one yet) would
    /// make that search endless where no blink shows: past
    /// [`HELD_BLINK_LOOKAHEAD`] blinks unseen it wakes her at the last
    /// one's start instead, a tick that changes nothing.
    fn next_held_blink(&self) -> Option<u64> {
        let (after, due) = (self.ticked, self.due());
        let ahead = self
            .held_blinks()
            .take_while(|&start| start < due)
            .skip_while(|&start| start + BLINK_MS <= after);
        for (k, start) in ahead.enumerate() {
            if k == HELD_BLINK_LOOKAHEAD {
                return Some(start);
            }
            if let Some((from, to)) = self.held_window(start)
                && to > after
            {
                return Some(if from > after { from } else { to });
            }
        }
        None
    }

    fn set(&mut self, act: Act, at: u64) {
        tracing::trace!(?act, x = self.x, y = self.y, "houseguest act");
        // Leaving what she chose (pulling on is still pulling; the next
        // phase of a borrow is still the borrow).
        let pulling_on = matches!(
            (&self.act, &act),
            (Act::Pull { .. }, Act::Pull { .. }) | (Act::Borrow { .. }, Act::Borrow { .. })
        );
        if !pulling_on {
            self.credit_done(at);
        }
        // However her act ends (her next decision, a look at the chat, a
        // startle), what she got up or came home for counts as had if it
        // ran its course.
        self.got_what_she_came_for(at);
        #[cfg(test)]
        let was_moving = census_moves(&self.act).trip();
        // A look up from her act, or a stir in it, goes with it: any
        // other act has its own look. (Her night never sets an act
        // mid-sleep, a Dream changing hers in place, so the night's stir
        // isn't cut by this.)
        self.looking_up = None;
        self.stir_until = 0;
        self.bob_held = None;
        self.sill = None;
        self.aloud = None;
        // Anything she's set at is chosen afresh, unless she's settling
        // into it (which counts itself, once set: see `settle_in`).
        self.settled = 0;
        // Standing while a chat watch is live she's drawn side-on
        // watching it (see `acting`), so whatever stood her (her "I'm
        // home!", her back aching, the watch's own stand) she faces it.
        if matches!(act, Act::Stand { .. }) && at < self.watch_until {
            self.facing = toward(self.x, self.watch_x);
        }
        self.act = act;
        self.act_since = at;
        self.act_since_game = self.game_at(at);
        self.act_due = self.first_due(at);
        #[cfg(test)]
        self.count_set_off(was_moving, at);
    }

    /// The census's count of her setting off, as she starts `self.act`
    /// (tests). A set-off is a start of a walk, a climb or a door that
    /// either comes from a still body (standing, looking at the chat, a
    /// pull: anything not on a trip, see [`Moves::trip`]) or is the
    /// first such start of a decision (so each hop of a trip, landed
    /// and chosen again, is one; a second start in one decision counts
    /// only from a still body). Moving on within a trip without a
    /// decision (a walk to a pole, then the climb; off the screen's
    /// edge and back in; a door's far side) is not one, and falling
    /// never is. Its purpose is read as she starts outside a decision,
    /// and once the decision has its method (its chain) inside one.
    #[cfg(test)]
    fn count_set_off(&mut self, was_moving: bool, at: u64) {
        let Moves::Off(body) = census_moves(&self.act) else {
            if !matches!(self.act, Act::Look { .. } | Act::Stand { .. }) {
                self.after_chat = None;
            }
            return;
        };
        let fresh = match self.deciding {
            Some(set_off) => !set_off || !was_moving,
            None => !was_moving,
        };
        if !fresh {
            return;
        }
        if let Some(set_off) = &mut self.deciding {
            *set_off = true;
        }
        self.set_offs += 1;
        let purpose = if self.deciding.is_some() {
            ""
        } else {
            self.census_purpose()
        };
        self.set_off_log.push(SetOff {
            at,
            body,
            purpose,
            after_chat: self.after_chat.take(),
        });
    }

    /// What a chat line arriving now cuts, for the census: what she's
    /// moving for, on a trip; `None` if she isn't on one.
    #[cfg(test)]
    fn census_cut(&self) -> Option<&'static str> {
        census_moves(&self.act)
            .trip()
            .then(|| self.census_purpose())
    }

    fn fire(&mut self, at: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) {
        // Her still act running its course before her look's `?` is
        // over: what it said under the look is said as it ends (she
        // stands to watch the rest). Only here, where it runs out: put
        // elsewhere, or called away, it goes unsaid.
        let runs_out = matches!(
            self.act,
            Act::Idle { until, .. } | Act::Use { until, .. } | Act::SpaceOut { until, .. }
                if at >= until
        );
        if runs_out
            && let Some(look) = self.looking_up
            && look.step < 2
        {
            self.say_what_her_look_hid(look.hidden, at, at);
        }
        match self.act.clone() {
            Act::Idle { until, .. } => {
                if at < until {
                    self.act_due = self.first_due(at);
                } else if self.sleeping() {
                    self.end_night(at, terrain, chances, rng);
                } else {
                    self.decide(at, terrain, chances, rng);
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
                    tracing::debug!("houseguest: stepped out");
                    // Never her shift (door batch, step 4b): work goes out
                    // by her door; round the edge is a moment's trip.
                    let until = if self.leaving.is_some() {
                        // Out by her routine: for good (the guest ends the
                        // visit), nothing drawn.
                        u64::MAX
                    } else {
                        at + rng.range(4000, 12_000)
                    };
                    self.set(
                        Act::Away {
                            until,
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
            Act::Door { since, to, gap } => match door_beat(at.saturating_sub(since), gap, to) {
                Some((beat, _)) => {
                    if beat.there {
                        self.out_of(to);
                    }
                    self.act_due = self.first_due(at);
                }
                None => {
                    self.out_of(to);
                    if self.errand == Some(to.spot()) {
                        self.poke(at);
                    } else if !self.back_home(at) {
                        // Drawn from the decision that sent her through
                        // (deciding draws anew); most doors, she says
                        // nothing. Said before deciding, so what she
                        // starts waits for it (a riddle, a grievance);
                        // if the decision speaks over it at once, it was
                        // never said (see `hush`). Not on a dash home
                        // from school: her routine's doors never say
                        // the door's lines (round 1, "Routine doors").
                        let line = if self.dash.is_none() {
                            self.lines.pick(mind::DOOR, self.whims, at)
                        } else {
                            None
                        };
                        if let Some(line) = line {
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
                    // Her way out is her errand's, for the census.
                    #[cfg(test)]
                    {
                        self.chain = "errand";
                    }
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
                        on: Some((seat.item, seat.what)),
                        let_go: false,
                    });
                }
                if at < until
                    && let Some(session) = self.sill.filter(|s| s.next <= at)
                {
                    self.sill_on(session, at);
                }
                if at >= until && self.sleeping() {
                    self.end_night(at, terrain, chances, rng);
                } else if at >= until {
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
            Act::Borrow { pull, phase, .. } => {
                self.borrowing(pull, phase, at, terrain, chances, rng)
            }
            Act::Dazed { .. } => {
                // Said in place of a hello when it's her entrance.
                if !self.greeted || rng.below(2) == 0 {
                    self.greeted = true;
                    self.say(OK, at);
                }
                self.decide(at, terrain, chances, rng)
            }
            // On to her daydream session's next musing, or the next key
            // of her riddle.
            Act::SpaceOut { until, session, .. } if at < until => {
                if let Some(session) = session.filter(|s| s.next <= at) {
                    self.muse_on(session, at);
                }
                self.act_due = self.first_due(at);
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
                        since: at,
                        until: at + rng.range(1500, 3000),
                        play: None,
                        session: None,
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
                self.credit_whole(Want::Swap, Via::Whole, at);
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
                // On her way to her door: wherever it stands now.
                if let Then::Job(Job::Leave { spot, why }) = then
                    && self.door_moved(spot, why, at, terrain, chances)
                {
                    return;
                }
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
                    // As the frame stands it now (`door_moved` saw the
                    // same spot).
                    Then::Job(Job::Leave { spot, why }) if spot.spot() == (self.x, self.y) => {
                        let spot = chances.door.unwrap_or(spot);
                        return self.at_door(spot, why, at, terrain, chances, rng);
                    }
                    // Where it was clear to go out (door batch M25): out
                    // there if it still is, else on to the nearest place
                    // that is.
                    Then::Job(Job::Out { at: clear, why }) if clear.spot() == (self.x, self.y) => {
                        if self.stays_in(why, at) {
                            return self.decide(at, terrain, chances, rng);
                        }
                        return self.out_where_clear(why, terrain, chances, at);
                    }
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
                    None if self.back_home(at) => {}
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
                // At the fridge for her midnight snack: a snack as it
                // plays, built here, nothing spliced round it, nothing
                // felt, nothing recorded (see `night_snack_at`). Still
                // owed until she has had it (see `got_what_she_came_for`).
                if self.snacking == Some(seat.piece) && seat.what == Use::Snack {
                    let act = self.night_snack_at(seat, at, rng);
                    return self.set(act, at);
                }
                // At the fridge for the lunch she dashed home for: built
                // here too, nothing spliced, felt or recorded, nothing
                // drawn (see `dash_lunch_at`). Still what she's home for
                // until she has had it.
                if self.dash == Some(Dash::Lunch(seat.piece)) && seat.what == Use::Snack {
                    let act = Self::dash_lunch_at(seat, at);
                    return self.set(act, at);
                }
                // Trying a piece where she has just set it down: a moment
                // on it, thoughtful.
                let trying = self.episode.is_some_and(|e| e.trying);
                let day = self.day(at);
                // How long a use lasts now (longer homework at homework
                // time): a trial is a share of it.
                let usual = use_duration_in(seat.what, day.map(|day| day.slot));
                let (lo, hi) = if trying { TRIAL_USE_MS } else { usual };
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
                // Her night's sleep (A4), in bed or on a sofa, real or
                // made: at night, or as the stage cues it. It plays whole,
                // nothing spliced round it and nothing felt: she's asleep.
                // Its length is drawn as a use's is, then (at night) her
                // wake time's.
                let sleeps_here = !trying && Surface::of(seat.what).is_some();
                let at_night = day.is_some_and(|day| day.slot == routine::Slot::Asleep);
                // What the stage cued, if it plays on this use (else it
                // waits for one it does). A splice waits for a use it
                // wraps that isn't her night's sleep (nothing wraps that).
                let cued = self.cued.take_if(|cue| {
                    cue.plays_on(seat.what, trying, grievance.is_some(), seat.makeshift())
                        && !(sleeps_here && at_night && matches!(cue, Cue::Splice(..)))
                });
                let night = sleeps_here
                    && match cued {
                        Some(Cue::Script(id)) => id.is_night(),
                        Some(Cue::Splice(..)) | None => at_night,
                    };
                if night {
                    let drawn = rng.range(lo, hi);
                    let mut act = self.night_in(seat, drawn, at, false);
                    // Cued, the Dream: her night dreamt from the start
                    // (tonight's, if it's night).
                    if cued == Some(Cue::Script(ScriptId::Dream)) {
                        self.dream_from(&mut act, at);
                    }
                    return self.set(act, at);
                }
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
                    makeshift: seat.makeshift(),
                    trying,
                    quiet: quiet <= at,
                    day,
                    tints: self.tints(),
                    rares: &self.rares,
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
                // Her first sunrise (New Year's Day): owed this visit
                // (any watch of hers that date), or cued. Nothing else
                // plays on it.
                let sunrise = watching && self.sunrise_here(cued, at);
                let bought = chances
                    .advert
                    .filter(|_| watching && !surf_cued && !sunrise);
                // Else, now and then, she flicks through the channels:
                // cued to, always (and it cools as if rolled); cued to
                // play anything else on the watch, never; else by the
                // chance first, so a surf that doesn't roll doesn't
                // cool.
                let surf = bought.is_none()
                    && watching
                    && !sunrise
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
                // A still use she chose, as long as her mood lingers over
                // it (not a trial: a moment's sit is a moment's).
                let drawn = rng.range(lo, hi);
                let length = if trying {
                    drawn
                } else {
                    self.stillness.lingered_use(self.mood, seat.what, drawn)
                };
                let whole = if trying {
                    let (lo, hi) = usual;
                    lo.midpoint(hi)
                } else {
                    length
                };
                // Her first snack of a morning or an evening: her meal's
                // line (not over what she'd say of her home).
                if seat.what == Use::Snack
                    && !trying
                    && grievance.is_none()
                    && let Some(day) = day
                {
                    self.meal(day, at);
                }
                let plain = Play::of(seat.what, bought);
                let own = if sunrise {
                    ScriptId::FirstSunrise
                } else if surf {
                    ScriptId::Surf
                } else {
                    plain.own
                };
                // Looking out of the window: a line from the sky as it is.
                let branch = if trying {
                    own.trial_branch()
                } else if own == ScriptId::LookOut {
                    self.look_out_branch(at)
                } else if own == ScriptId::Homework {
                    // Where her homework nods off: her mood's (phase 5c M8).
                    self.stillness.nod_off.of(self.mood).branch()
                } else {
                    0
                };
                // The programme her TV holds as she watches: drawn on every
                // watch, whatever it plays (phase 5c D7).
                let card = if seat.what == Use::Watch {
                    let cards = art::Programme::ALL;
                    cards[self.whims.below("programme", cards.len() as u64) as usize]
                } else {
                    plain.card
                };
                let play = Play {
                    own,
                    branch,
                    before,
                    after,
                    card,
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
            // At the line's end: take hold of it, to borrow a strip.
            Job::Borrow(pull) => Act::Borrow {
                pull,
                since: at,
                phase: Borrowing::Brace,
            },
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
            // At her door's spot: out through it (door batch D6), facing
            // its wall. School's gap never ends (the visit ends once it
            // has closed behind her); the stage's lets her back in.
            Job::Leave { spot, why } => return self.through_her_door(spot, why, at),
            // Never begun here: the walk's arrival takes a `Job::Out` at
            // its spot (it's judged again there, against the frame, with
            // her floor in hand: `Osaka::out_where_clear`), and nothing
            // else starts one. Should one get here, she stays in.
            Job::Out { why, .. } => {
                debug_assert!(false, "Job::Out is begun only at the walk's arrival");
                return self.nowhere_clear(why, at);
            }
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
        self.lean_on_the_sill();
    }

    /// Leaning on her window's sill, a look-out she chose just begun
    /// (phase 5c D6): after the sky's line, up to three musings on
    /// the sky to come (as many as her decision's whim draws from her
    /// mood's [`Stillness::sill`]), the first a whim's gap after the line
    /// (see [`Osaka::sill_on`]). Not on a trial (a moment's look) or a
    /// cued script other than her own.
    fn lean_on_the_sill(&mut self) {
        let Act::Use {
            seat, since, play, ..
        } = self.act
        else {
            return;
        };
        // A trial (3.5-5 s) would end before a first musing could come
        // (15 s in at the least), and `set` drops the session with its
        // act: this only says so.
        let trying = self.episode.is_some_and(|e| e.trying);
        if seat.what != Use::LookOut || play.own != ScriptId::LookOut || trying {
            return;
        }
        let (lo, hi) = self.stillness.sill.of(self.mood);
        let musings = (lo
            + self
                .whims
                .below("sill", u64::from(hi.saturating_sub(lo)) + 1) as u8)
            .min(SILL_MUSINGS);
        if musings == 0 {
            tracing::trace!("houseguest: at her sill with nothing to muse");
            return;
        }
        let next = play.body_start(since) + script::LOOK_OUT_LINE_MS + self.sill_gap(self.whims, 0);
        self.sill = Some(Session {
            left: musings,
            next,
            said: 0,
            whims: self.whims,
        });
        self.act_due = self.act_due.min(next);
    }

    /// The gap before her `k`th musing at the sill (counting from 0: the
    /// first's is after the sky's line), drawn from its decision's
    /// `whims`, as long as a daydream session's ([`Stillness::musing_gap`]).
    fn sill_gap(&self, whims: Whims, k: u8) -> u64 {
        let (lo, hi) = self.stillness.musing_gap;
        lo + whims
            .series("sill", u64::from(k))
            .below("gap", hi.saturating_sub(lo) + 1)
    }

    /// Her next musing at the sill is due at `at` (phase 5c D6): said now,
    /// the `said`th, from the sky as it is now ([`mind::sky_musings`]),
    /// drawn from the session's whims (none, if every line of that sky's
    /// is cooling), unless she's saying something or looking up at the
    /// chat, when it waits for that to be over, or her look ended less
    /// than a frame ago ([`Osaka::free_to_muse`]). The last one said, she
    /// leans on in silence.
    fn sill_on(&mut self, session: Session, at: u64) {
        let busy = self.free_to_muse(at);
        self.sill = if busy > at {
            Some(Session {
                next: busy,
                ..session
            })
        } else {
            let sky = self.sky_at(at);
            let whims = session.whims.series("sill", u64::from(session.said));
            if let Some(line) = self.lines.pick(mind::sky_musings(sky), whims, at) {
                tracing::debug!(said = session.said, line, "houseguest: muses at her sill");
                self.say(line, at);
            }
            (session.left > 1).then(|| Session {
                left: session.left - 1,
                next: at + self.sill_gap(session.whims, session.said + 1),
                said: session.said + 1,
                whims: session.whims,
            })
        };
    }

    /// Her night's sleep (A4) at `seat`, on her bed or her sofa, real or
    /// made, from `at`: until her wake time, if it's night by her routine;
    /// else (the stage's, by day) `drawn` long. The lamp on a moment as
    /// she settles, or (`at_once`) off from the start.
    fn night_in(&mut self, seat: Seat, drawn: u64, at: u64, at_once: bool) -> Act {
        if let PieceRef::Made(id) = seat.piece {
            self.events.push(HomeEvent::Used(id));
        }
        let surface = Surface::of(seat.what).unwrap_or(Surface::Bed);
        let until = self.night_until(at, drawn);
        self.arm_night(at);
        tracing::info!(item = ?seat.item, ?surface, until, "houseguest: asleep for the night");
        Act::Use {
            seat,
            since: at,
            until,
            whole: until - at,
            play: Play {
                branch: surface.branch(at_once),
                ..Play::plain(ScriptId::Night)
            },
            grievance: None,
        }
    }

    /// Her night's sleep on the floor (A4) from `at`, lying still where
    /// she is, the lamp on a moment as she settles: until her wake time
    /// (else, by day, as long as lying back lasts).
    fn night_on_floor(&mut self, at: u64, rng: &mut Rng) -> Act {
        let (lo, hi) = Activity::LieBack.duration();
        let until = self.night_until(at, rng.range(lo, hi));
        self.arm_night(at);
        tracing::info!(until, "houseguest: asleep for the night on the floor");
        Act::Idle {
            what: Activity::LieBack,
            since: at,
            until,
            play: Some(Play {
                branch: Surface::Floor.branch(false),
                ..Play::plain(ScriptId::Night)
            }),
        }
    }

    /// When a night's sleep begun at `at` ends: her wake time, if it's
    /// night by her routine; else `drawn` ms on.
    fn night_until(&self, at: u64, drawn: u64) -> u64 {
        self.game_at(at)
            .and_then(|game| self.wake_of(game))
            .map_or(at + drawn, |wake| wake.max(at))
    }

    /// Tucked in (A4): a visit beginning at night (`now`) has her in her
    /// bed, else on her sofa, asleep from the start, the lamp already
    /// off. False, and nothing changes, with neither shown where she
    /// could lie (she arrives as ever, and her routine sends her to bed),
    /// or when it isn't night by her routine. Her look at the chat and
    /// her hello wait for her to wake.
    pub fn tuck_in(&mut self, chances: &Chances, terrain: &Terrain, now: u64) -> bool {
        let night = self
            .day(now)
            .is_some_and(|day| day.slot == routine::Slot::Asleep);
        let Some(seat) = night_seat(chances, terrain).filter(|_| night) else {
            return false;
        };
        tracing::info!(item = ?seat.item, "houseguest: tucked in");
        (self.x, self.y, self.facing) = (seat.x, seat.y, seat.facing);
        self.heading = None;
        self.credit = Some(Want::Use(seat.what));
        let act = self.night_in(seat, 0, now, true);
        self.set(act, now);
        self.decided = now;
        // Tucked in asleep: no "Night-night..." (she's long since said it).
        self.latch_lamp(now);
        true
    }

    /// The lamp goes off for the night as her night's lamp key shows it
    /// in the night (see `lamp_off`). A night's sleep the stage gives her
    /// by day darkens her lamp by its key alone, while it plays. Returns
    /// whether she turned it off just now, settling for the night (and
    /// so says "Night-night..."): only her night's own lamp key. Dreaming
    /// (the stage's Dream from the start), she's long since said it, and
    /// it goes off without a word.
    fn latch_lamp(&mut self, at: u64) -> bool {
        let night = self.asleep_slot(at);
        let off = self.night_play().and_then(|(play, since, until)| {
            play.key(since, until, at)
                .filter(|(key, _)| key.prop == Some(script::Shows::Still(Prop::LampOff)))
                .map(|_| play.own)
        });
        let Some(own) = off.filter(|_| night && !self.lamp_off) else {
            return false;
        };
        tracing::info!(?own, "houseguest: the lamp off for the night");
        self.lamp_off = true;
        own == ScriptId::Night
    }

    /// Whether it's night by her routine at `at` (her Asleep slot).
    fn asleep_slot(&self, at: u64) -> bool {
        self.day(at)
            .is_some_and(|day| day.slot == routine::Slot::Asleep)
    }

    /// Whether her lamp is dark at `now` for the night, whatever she's
    /// doing (see `lamp_off`): off since she settled in, and still the
    /// night. What her act shows on her furniture is [`Osaka::prop`].
    pub fn dark(&self, now: u64) -> bool {
        self.lamp_off && self.asleep_slot(now)
    }

    /// At bedtime, or as she leaves for school (`at`), whatever moving of
    /// her home is under way is over for the day (it can't wait out the night: trying a piece is a
    /// moment's sit, which her night's sleep isn't). Set down where she
    /// tried it, it stays there; lifted, or not yet, she lets it go, the
    /// piece back where it stood (see [`Osaka::drop_episode`]).
    fn settle_for_night(&mut self, at: u64) {
        if self.episode.is_some() {
            tracing::info!("houseguest: her arranging over for the day");
            self.drop_episode(None, at);
        }
        self.just_set = None;
    }

    /// Her routine's glance up at her wall clock (D7), the first key of
    /// her bed or school reflex at `at`: turned toward the clock, gazing
    /// up at it, `glance`'s line ("Oh! It's late!", "Time for school!"),
    /// whatever she was saying stopping for it. Only with her clock
    /// hanging on the strip she stands on, and once a slot (see
    /// `clock_glanced`): `None` otherwise, and her reflex goes on.
    fn glance_first(&mut self, glance: ClockGlance, at: u64) -> Option<Decision> {
        if !self.pass_glance(at) {
            return None;
        }
        let x = self.clock_on.and_then(|c| c.seen_from((self.x, self.y)))?;
        if self.hidden(at) {
            return None;
        }
        tracing::debug!(?glance, "houseguest: a glance at her clock");
        self.glance_at_clock(glance, x, at);
        Some(Decision::reflex("routine/glance"))
    }

    /// Her routine's glance up at her clock is spent for the slot she's
    /// in at `at` (keyed on its end: see `clock_glanced`): whether it
    /// wasn't yet. Without her routine, never anything to spend.
    fn pass_glance(&mut self, at: u64) -> bool {
        let Some(end) = self
            .clock
            .zip(self.game_at(at))
            .map(|(clock, game)| clock.next_boundary(game))
        else {
            return false;
        };
        self.clock_glanced.replace(end) != Some(end)
    }

    /// A glance up at her clock (at column `x`) from `at`: turned toward
    /// it, spacing out a moment, saying `glance`'s line, whatever she was
    /// saying stopping for it.
    fn glance_at_clock(&mut self, glance: ClockGlance, x: i32, at: u64) {
        if x != self.x {
            self.facing = toward(self.x, x);
        }
        self.hush(at);
        self.credit = None;
        self.set(
            Act::SpaceOut {
                since: at,
                until: at + CLOCK_GLANCE_MS,
                play: Some(Play {
                    branch: glance.branch(),
                    ..Play::plain(ScriptId::ClockGlance)
                }),
                session: None,
            },
            at,
        );
    }

    /// Her routine's bed reflex (D4, A4): at night, to bed. Her bed, else
    /// her sofa (the one way she finds either, tucked in too: see
    /// [`night_seat`]); else a makeshift one, by its own methods (one she
    /// made, or making one); else the floor where she is. Each in turn
    /// until she can set off for it. Her night's sleep begins as she gets
    /// there (see [`Osaka::start_job`]).
    fn send_to_bed(
        &mut self,
        ctx: &Ctx,
        whims: Whims,
        here: usize,
        terrain: &Terrain,
        at: u64,
        rng: &mut Rng,
    ) -> Decision {
        // Watching the chat, she still faces it as she sets off.
        if at < self.watch_until {
            self.facing = toward(self.x, self.watch_x);
        }
        // Up for her midnight snack: the fridge first, hop by hop (each
        // landing decides again, and comes here), then back to bed.
        if let Some(fridge) = self.snacking {
            let want = Want::Use(Use::Snack);
            // Kept until she has had it (see `got_what_she_came_for`).
            match fridge_seat(fridge, ctx.chances) {
                Some(seat) if self.go_to(want, Job::Use(seat), here, terrain, at) => {
                    self.credit = Some(want);
                    return Decision {
                        want: Some(want),
                        ..Decision::reflex("routine/snack")
                    };
                }
                _ => {
                    tracing::debug!("houseguest: no way to the fridge, back to bed");
                    self.snacking = None;
                }
            }
        }
        let real = night_seats(ctx.chances, terrain)
            .map(|seat| (Want::Use(seat.what), "routine/bed", Job::Use(seat)));
        let made = [Use::Sleep, Use::Nap].into_iter().filter_map(|what| {
            let want = Want::Use(what);
            match mind::bind(ctx, whims, want) {
                Some((_, Bind::Job(job))) => Some((want, "routine/bed-made", job)),
                _ => None,
            }
        });
        for (want, method, job) in real.chain(made) {
            if self.go_to(want, job, here, terrain, at) {
                self.credit = Some(want);
                return Decision {
                    want: Some(want),
                    ..Decision::reflex(method)
                };
            }
        }
        let act = self.night_on_floor(at, rng);
        self.set(act, at);
        self.credit = Some(Want::Idle(Activity::LieBack));
        Decision {
            want: self.credit,
            ..Decision::reflex("routine/bed-floor")
        }
    }

    /// Her routine's away reflex (D4, A9; the door batch D6): at school
    /// time (`why`), she sets off for her door where the frame stands it
    /// (`chances.door`), saying so once ("Late, late, late!" if it cut
    /// her breakfast short; the latch, `set_off`), and walks there (a
    /// hop or a door in space between floors, as any walk); at its spot,
    /// out through it ([`Osaka::at_door`]). Re-entered (a landing, a walk
    /// come to nothing, a look at the chat), she says nothing and goes on.
    /// With no door anywhere, or no way to its spot, out by a door in
    /// space where she stands, out of any piece first
    /// ([`Osaka::out_where_clear`]). The visit ends only once her door
    /// has closed behind her ([`Osaka::gone_out`]), and her closed door
    /// stands at its spot until she comes home out of it.
    fn go_out(
        &mut self,
        why: Routine,
        whims: Whims,
        ctx: &Ctx,
        at: u64,
        rng: &mut Rng,
    ) -> Decision {
        let (terrain, chances) = (ctx.terrain, ctx.chances);
        let door = chances.door;
        let at_spot = door.is_some_and(|d| d.spot() == (self.x, self.y));
        let first = self.set_off.is_none();
        if first {
            let line = if std::mem::take(&mut self.late) {
                Some(LATE)
            } else {
                self.lines.pick(mind::OFF, whims, at)
            };
            if let Some(line) = line {
                self.say(line, at);
            }
            match door {
                Some(door) => {
                    tracing::info!(?why, door = ?door.spot(), "houseguest: sets off for her door");
                }
                // `out_where_clear` says where she goes out.
                None => tracing::debug!(?why, "houseguest: sets off, no door anywhere"),
            }
            self.set_off = Some(why);
        }
        self.credit = None;
        let leave = match why {
            Routine::School => Leave::School,
        };
        match door {
            None => self.out_where_clear(leave, terrain, chances, at),
            Some(spot) if at_spot => {
                self.start_job(Job::Leave { spot, why: leave }, at, chances, rng);
                // Watching the chat, at her door already, she says it to
                // the chat (door batch C14).
                if first && at < self.watch_until {
                    self.facing = toward(self.x, self.watch_x);
                }
            }
            Some(spot) => {
                let job = Job::Leave { spot, why: leave };
                if !self.go_to(Want::Walk, job, ctx.here, terrain, at) {
                    self.out_where_clear(leave, terrain, chances, at);
                }
            }
        }
        Decision::reflex("routine/away")
    }

    /// On her walk to her door's `spot` (`why`), each step (door batch
    /// D6): the frame's door is read again. Where it stands now is
    /// where she goes (a resize, or a focused pane over its spot: she
    /// never walks on into the pane for it), silently; with no door
    /// anywhere now, or no way to where it stands, out by a door in space
    /// where she stands ([`Osaka::out_where_clear`]). Returns whether it
    /// set her about something else (the walk she's on is stale).
    fn door_moved(
        &mut self,
        spot: DoorSpot,
        why: Leave,
        at: u64,
        terrain: &Terrain,
        chances: &Chances,
    ) -> bool {
        match chances.door {
            // Where it stood (how it stands may have changed: at its spot
            // she goes out through it as it stands then).
            Some(door) if door.spot() == spot.spot() => false,
            Some(door) => {
                tracing::debug!(from = ?spot.spot(), to = ?door.spot(), "houseguest: her door moved on her way");
                let job = Job::Leave { spot: door, why };
                let went = terrain
                    .platform_at(self.x, self.y)
                    .is_some_and(|here| self.go_to(leave_want(why), job, here, terrain, at));
                if !went {
                    self.out_where_clear(why, terrain, chances, at);
                }
                true
            }
            None => {
                tracing::debug!(from = ?spot.spot(), "houseguest: her door's gone on her way");
                self.out_where_clear(why, terrain, chances, at);
                true
            }
        }
    }

    /// At her door's `spot` on her way out (`why`; door batch D6), where
    /// the frame stands it now (her walk followed it each step,
    /// [`Osaka::door_moved`]). School over on her way (a dash's way out
    /// crossing 12:45): she stays in and decides instead, silently. Else
    /// out through it ([`Osaka::start_job`]).
    fn at_door(
        &mut self,
        spot: DoorSpot,
        why: Leave,
        at: u64,
        terrain: &Terrain,
        chances: &Chances,
        rng: &mut Rng,
    ) {
        if self.stays_in(why, at) {
            return self.decide(at, terrain, chances, rng);
        }
        self.start_job(Job::Leave { spot, why }, at, chances, rng);
    }

    /// Whether, about to go out for `why` at `at`, she stays in: school
    /// over on her way (her routine no longer has her out). She decides
    /// then, silently, and that lets her set-off go (`choose_next`: a
    /// later school morning says its line again).
    fn stays_in(&self, why: Leave, at: u64) -> bool {
        let school = self
            .day(at)
            .is_some_and(|day| day.slot == routine::Slot::Away);
        let stays = why == Leave::School && !school;
        if stays {
            tracing::debug!(at = ?(self.x, self.y), "houseguest: on her way out, school's over: she stays in");
        }
        stays
    }

    /// The stage's school scene (door batch D10): she sets off for her
    /// `door` from where she stands, with no line and nothing latched,
    /// and goes out through it for a short gap (no `leaving`: the visit
    /// never ends), coming back in with "I'm home!". False, and nothing
    /// done, when she isn't on a floor or has no way there.
    pub fn off_to_school_on_stage(
        &mut self,
        door: DoorSpot,
        terrain: &Terrain,
        chances: &Chances,
        now: u64,
        rng: &mut Rng,
    ) -> bool {
        let Some(here) = terrain.platform_at(self.x, self.y) else {
            return false;
        };
        self.credit = None;
        let job = Job::Leave {
            spot: door,
            why: Leave::Stage,
        };
        if door.spot() == (self.x, self.y) {
            self.start_job(job, now, chances, rng);
            return true;
        }
        self.go_to(Want::Walk, job, here, terrain, now)
    }

    /// Out by a door in space where she stands (`why`; door batch M25):
    /// no door of hers to go to, or no way to its spot. Only where her
    /// box meets none of what her door mustn't (`chances.obstacles`:
    /// she's out of the piece she got up from), so a door never opens in
    /// her bed; while it does, she goes to the nearest place where it
    /// doesn't ([`nearest_clear`]: along her floor, a calm one first; with
    /// none there, by a hop or a door in space between floors, as any
    /// walk, door batch step 10a), and out there on arrival
    /// (`Job::Out`). With nowhere clear at all, she stays in for now
    /// ([`Osaka::nowhere_clear`]).
    fn out_where_clear(&mut self, why: Leave, terrain: &Terrain, chances: &Chances, at: u64) {
        if let Some(feet) = Clear::of((self.x, self.y), &chances.obstacles) {
            return self.out_at(feet, why, at);
        }
        let here = terrain.platform_at(self.x, self.y);
        let to = nearest_clear(terrain, here, (self.x, self.y), &chances.obstacles);
        let went = match (here, to) {
            (Some(here), Some(to)) => {
                tracing::debug!(to = ?to.spot(), "houseguest: no door to go to; out of the piece first");
                self.go_to(leave_want(why), Job::Out { at: to, why }, here, terrain, at)
            }
            // On no floor of hers: by a door in space to it, as any walk
            // with no way there.
            (None, Some(to)) => {
                tracing::debug!(to = ?to.spot(), "houseguest: no door to go to, on no floor; out of the piece first");
                self.door_to(leave_want(why), Job::Out { at: to, why }, at);
                true
            }
            (_, None) => false,
        };
        if !went {
            self.nowhere_clear(why, at);
        }
    }

    /// Nowhere she can go out by a door in space clear of her pieces
    /// (door batch, step 10a): she stays in for now, standing a moment,
    /// and goes again as her routine sends her (school's reflex, at her
    /// next decision). Work can wait ([`Osaka::let_work_go`]); the
    /// stage's scene is over. Said at info the first time since she
    /// last went out (a user-visible change: she doesn't go), at debug
    /// as her routine tries again.
    fn nowhere_clear(&mut self, why: Leave, at: u64) {
        let spot = (self.x, self.y);
        if std::mem::replace(&mut self.kept_in, true) {
            tracing::debug!(?why, at = ?spot, "houseguest: still nowhere clear of her pieces to go out by");
        } else {
            tracing::info!(?why, at = ?spot, "houseguest: nowhere clear of her pieces to go out by; she stays in for now");
        }
        if why == Leave::Work {
            self.let_work_go(at);
        }
        self.set(
            Act::Stand {
                until: at + OUT_AGAIN_MS,
            },
            at,
        );
    }

    /// Out through her door at `spot` (she's at it) for `why`, now (door
    /// batch D6): facing its wall, its gap and what it sets by `why`
    /// ([`Osaka::leave_by`]).
    fn through_her_door(&mut self, spot: DoorSpot, why: Leave, at: u64) {
        self.facing = spot.out();
        let gap = self.leave_by(why);
        tracing::info!(spot = ?spot.spot(), ?why, "houseguest: out through her door");
        self.set(
            Act::Door {
                since: at,
                to: Through::Home(spot),
                gap,
            },
            at,
        );
    }

    /// Out by a door in space at `clear`, where she stands (`why`), now:
    /// no door of hers to go to. School's gap never ends; the stage's
    /// lets her back in; work's is her shift's.
    fn out_at(&mut self, clear: Clear, why: Leave, at: u64) {
        debug_assert_eq!(clear.spot(), (self.x, self.y), "out where she stands");
        tracing::info!(?why, at = ?clear.spot(), "houseguest: out by a door in space where she stands");
        let gap = self.leave_by(why);
        self.open_out(clear, at, gap, at);
    }

    /// A way out of hers in space opened (begun at `since`, with `gap`)
    /// at `clear`: with [`Osaka::redirect_out`], the one way a door of
    /// hers that's a way out comes to stand in space, only where her
    /// pieces leave it clear (door batch, step 10a).
    fn open_out(&mut self, clear: Clear, since: u64, gap: u64, at: u64) {
        self.set(
            Act::Door {
                since,
                to: Through::Space(clear.spot()),
                gap,
            },
            at,
        );
    }

    /// What going out by a door for `why` sets (door batch D6, M9): the
    /// one place it's decided, whichever door. School: `leaving` (the
    /// visit ends once it has closed behind her) and a gap that never
    /// ends. The stage's: `returning` (coming back in, she's home; see
    /// `back_home`) and a short gap. Work's: her shift's gap (drawn as
    /// she set off, [`Osaka::go_to_work`]); coming back in, she's home
    /// from work. Returns the gap.
    fn leave_by(&mut self, why: Leave) -> u64 {
        self.kept_in = false;
        match why {
            Leave::School => {
                self.leaving = Some(Routine::School);
                u64::MAX
            }
            Leave::Stage => {
                self.returning = Some(Routine::School);
                STAGE_GAP_MS
            }
            Leave::Work => match self.shift {
                Some(Shift::Going { gap }) => {
                    self.shift = Some(Shift::Out);
                    gap
                }
                // Never: work's way out goes with its shift. Every way a
                // shift is cut lets her way to work go with it
                // ([`Osaka::let_work_go`]: an errand, a decision that
                // doesn't go on; `place`; school rewrites it to school's,
                // `school_from_work`), and she's through one door for it
                // (`Out`). So only `go_to_work`, the "work/on" decision and
                // a walk for it (all with `Going`) come here. Were one
                // missed, a door with no shift is a moment's, as any door
                // in space.
                Some(Shift::Out) | None => {
                    tracing::warn!(shift = ?self.shift, "houseguest: out for work with no shift on her way");
                    0
                }
            },
        }
    }

    /// The routine she's out by, once she's through her door and it has
    /// closed behind her (or she's off the screen for good) at `now`: the
    /// guest ends the visit then (A9).
    pub fn gone_out(&self, now: u64) -> Option<Routine> {
        self.leaving
            .filter(|_| self.hidden(now) && self.door(now).is_none())
    }

    /// Her night's sleep is over at `at` (it ran to its end): at her wake
    /// time, she wakes to a new day; one the stage gave her by day just
    /// ends (the lamp on again), and she decides as ever.
    fn end_night(&mut self, at: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) {
        let night = self
            .day(at)
            .is_some_and(|day| day.slot == routine::Slot::Asleep);
        let morning = !night
            && self
                .act_since_game
                .is_some_and(|began| self.wake_of(began).is_some());
        if morning {
            self.wake(at, terrain);
        } else {
            self.decide(at, terrain, chances, rng);
        }
    }

    /// She wakes at `at`, her night's sleep over (round 1, "each morning
    /// is a new day"): up beside her bed (there's no sitting up), a big
    /// stretch, and good morning in the new day's mood (see
    /// [`Osaka::begin_day`]).
    fn wake(&mut self, at: u64, terrain: &Terrain) {
        let Some(day) = self.day(at) else {
            return;
        };
        let beside = match self.act {
            Act::Use { seat, .. } => beside(&seat, terrain),
            _ => None,
        };
        tracing::info!(%day, "houseguest: she wakes");
        if let Some((x, y)) = beside {
            self.facing = toward(x, self.x);
            (self.x, self.y) = (x, y);
        }
        let line = self.wake_line(day, at);
        let (lo, _) = Activity::Stretch.duration();
        // Leaving the night settles it (her credit, the time slept)
        // before the new day begins.
        self.set(
            Act::Idle {
                what: Activity::Stretch,
                since: at,
                until: at + speech_ms(line).max(lo),
                play: None,
            },
            at,
        );
        self.begin_day(day, at);
    }

    /// A new game day begins for her at `at` (`day`), without a new
    /// visit, whether she wakes into it ([`Osaka::wake`]) or was up at
    /// her wake time ([`Osaka::catch_up_day`]): the one place either
    /// starts it. Its mood, keyed on the day as a visit's is (so it's the
    /// one a visit that day would bring), a fresh line budget and shift;
    /// her needs the time of day's (the night is behind her: none of it
    /// is counted again), the lamp on, and good morning in that mood.
    fn begin_day(&mut self, day: DayTime, at: u64) {
        let mood = Mood::of(brain::day_seed(self.master, day.day));
        tracing::info!(?mood, %day, "houseguest: a new day");
        self.mood = mood;
        // What's rare today: drawn for the whole day, from the day's
        // seed, unless this visit already drew it (begun after
        // midnight).
        if self.rares_day != Some(day.day) {
            self.rares = Rares::draw(
                brain::day_seed(self.master, day.day),
                &self.seen,
                self.pity,
                rarity::DAY_WINDOW,
            );
            self.rares_day = Some(day.day);
            tracing::debug!(rares = ?self.rares, %day, "houseguest: what's rare today");
        }
        self.lines.new_day(at);
        self.budget_day = Some(day.day);
        self.day_from = self.game_at(at);
        self.worked = false;
        self.set_clock(day);
        self.decided = at;
        self.slept_ms = 0;
        self.lamp_off = false;
        self.groggy = false;
        self.night = None;
        self.snacking = None;
        self.greeted = true;
        let line = self.wake_line(day, at);
        self.say(line, at);
        self.morning_until = at + speech_ms(line);
        // A day off: she says so, once she's quiet.
        self.day_off = !day.school_day;
    }

    /// What she says as her day `day` begins at `at`: good morning in the
    /// day's mood; or, with her calendar's greeting owed, a plain
    /// "Mornin'." (the greeting is her mood's part of it, said next, as
    /// the first beat she owes once she's quiet: a night tucked in, it
    /// waits for her to wake; see [`Osaka::calendar_beat`]).
    fn wake_line(&self, day: DayTime, at: u64) -> &'static str {
        let mood = if self.calendar_greeting(at).is_some() {
            Mood::Ordinary
        } else {
            Mood::of(brain::day_seed(self.master, day.day))
        };
        mood.wake_line()
    }

    /// Whether she's up at `now` for what would have her say something
    /// (a parcel at the door): not asleep for the night, nor up in the
    /// middle of it (for the accordion, or her midnight snack: her night
    /// isn't over until her day begins), not saying good morning or that
    /// she's home, which nothing speaks over; and here: not out of sight,
    /// nor on her way out or in, by her routine or to and from work (a
    /// parcel waits for her to have said she's home; on her way out,
    /// [`Osaka::on_her_way_out`], for her next visit).
    pub fn awake(&self, now: u64) -> bool {
        !self.sleeping()
            && self.night.is_none()
            && !self.groggy_at(now)
            && now >= self.morning_until
            && !self.hidden(now)
            && !self.on_her_way_out()
            && self.returning.is_none()
            && self.shift.is_none()
    }

    /// Whether she's on her way out by a door of hers (door batch D6):
    /// set off for it (`set_off`, latched until school's over or she's
    /// moved), walking or heading to it, stepping out of a piece to go
    /// out where she stands, or through it (`leaving`).
    pub fn on_her_way_out(&self) -> bool {
        let leave = |job: &Job| job.leave().is_some();
        self.set_off.is_some()
            || self.leaving.is_some()
            || self.heading.as_ref().is_some_and(|h| leave(&h.job))
            || matches!(&self.act, Act::Walk { then: Then::Job(job), .. } if leave(job))
    }

    /// Whether she's bound for her own door to go out by it: her line
    /// said as she set off (`set_off`, latched), or walking or heading to
    /// it (a `Job::Leave`, work's way and the stage's too). Her door
    /// shows from then (door batch D8, C3).
    pub fn bound_for_her_door(&self) -> bool {
        let leave = |job: &Job| matches!(job, Job::Leave { .. });
        self.set_off.is_some()
            || self.heading.as_ref().is_some_and(|h| leave(&h.job))
            || matches!(&self.act, Act::Walk { then: Then::Job(job), .. } if leave(job))
    }

    /// Whether she's free at `now` for a gift on her doorstep (her wall
    /// clock, phase 5b D7): up and here ([`Osaka::awake`]), not walking
    /// off the screen nor going through a door (out of sight in a
    /// moment, either way), her hello said and nothing else being said, and
    /// her calendar settled ([`Osaka::calendar_settled`]: it comes first,
    /// all of it, but a day she lets be, or a first sunrise she can't get
    /// to her TV for, doesn't hold the gift back). `chances` and
    /// `terrain` are the frame's, as her calendar's beat reads them.
    pub fn free_for_a_gift(&self, now: u64, chances: &Chances, terrain: &Terrain) -> bool {
        self.awake(now)
            && !matches!(self.act, Act::Out { .. } | Act::Door { .. })
            && self.greeted
            && self.speech.is_none_or(|(_, until)| until <= now)
            && self.calendar_settled(now, chances, terrain)
    }

    /// Whether a parcel may come in at `now` (door batch D9, C10 and
    /// Open choice 3): never as she goes through a door of hers or while
    /// she's out (her slippers in her door's space), and only while she's
    /// on her way somewhere (walking, climbing) or an act of hers has
    /// only just begun ([`PARCEL_AT_START_MS`]): its slide through her
    /// door's flap and the flap shutting (a handful of changes in under a
    /// second) never come in the middle of an act that holds her still.
    /// `staged`: the stage's parcel scene, whose cue sends her to unpack
    /// it, whatever she's at. (Up and here, [`Osaka::awake`], is the
    /// caller's.)
    pub fn free_for_a_parcel(&self, staged: bool, now: u64) -> bool {
        match self.act {
            Act::Out { .. } | Act::Door { .. } => false,
            Act::Walk { .. } | Act::Climb { .. } | Act::Clamber { .. } => true,
            _ => staged || now < self.act_since.saturating_add(PARCEL_AT_START_MS),
        }
    }

    /// Where her feet stand now and, on her way somewhere, where they'll
    /// stand within `ms` (door batch C10: what a parcel's slide must keep
    /// clear of): a walk's columns ahead, one a [`WALK_MS`]; a climb's or
    /// a clamber's way to its end, however long it takes her (both rare,
    /// and slower than a walk). Every other act holds her where she
    /// stands until it ends (one that ends within `ms` and walks her off
    /// begins its walk after a slide has begun: a parcel only comes as an
    /// act begins or on her way, [`Osaka::free_for_a_parcel`]).
    pub fn feet_within(&self, ms: u64) -> Vec<(i32, i32)> {
        let row = |from: i32, to: i32, y: i32| {
            let step = if to < from { -1 } else { 1 };
            (0..=(to - from).abs()).map(move |d| (from + d * step, y))
        };
        match self.act {
            Act::Walk { to, .. } => {
                let steps = i32::try_from(ms.div_ceil(WALK_MS)).unwrap_or(i32::MAX);
                let ahead = (to - self.x).clamp(-steps, steps);
                row(self.x, self.x + ahead, self.y).collect()
            }
            Act::Climb { to_y } => row(self.y, to_y, self.x).map(|(y, x)| (x, y)).collect(),
            Act::Clamber { column, to_y, to_x } => row(self.x, column, self.y)
                .chain(row(self.y, to_y, column).map(|(y, x)| (x, y)))
                .chain(row(column, to_x, to_y))
                .collect(),
            _ => vec![(self.x, self.y)],
        }
    }

    /// Whether her calendar has nothing more to do at `now` before
    /// anything else she'd say this visit: no date, or nothing owed
    /// (delivered, or nothing that day); its greeting owed only if she
    /// has let it be for the visit ([`CAL_MISSES`]; it's still owed, but
    /// she won't say it again until her next visit), not if it's waiting
    /// to be said again elsewhere; not under way; not Setsubun's beans,
    /// owed (thrown on the spot, next) or being thrown; not the first
    /// sunrise playing (both delivered at their first key, they play on
    /// to their end first); nor New Year's first sunrise after it, owed
    /// with her TV where she can get to it (she goes next), or on her way
    /// to it.
    fn calendar_settled(&self, now: u64, chances: &Chances, terrain: &Terrain) -> bool {
        let Some(date) = self.real_date() else {
            return true;
        };
        if self.cal_under_way(date, now)
            || self
                .plays()
                .is_some_and(|p| matches!(p.own, ScriptId::Setsubun | ScriptId::FirstSunrise))
            || self.sunrise_tv(chances, terrain).is_some()
            || self.to_her_sunrise()
        {
            return false;
        }
        match self.calendar_owed(now) {
            None => true,
            Some((_, Owed::Setsubun)) => false,
            Some((date, Owed::Greet(_) | Owed::FirstSunrise)) => self.cal_let_be(date),
        }
    }

    /// Her TV's seat for New Year's first sunrise, if it's owed this
    /// visit, she hasn't been sent to it yet, and the seat is where she
    /// can get to it (`chances` and `terrain` the frame's).
    fn sunrise_tv(&self, chances: &Chances, terrain: &Terrain) -> Option<Seat> {
        self.sunrise
            .filter(|s| !s.sent && Some(s.date) == self.real_date())
            .and_then(|_| {
                chances
                    .seats
                    .iter()
                    .find(|s| s.what == Use::Watch && terrain.platform_at(s.x, s.y).is_some())
                    .copied()
            })
    }

    /// Whether she's on her way to her TV for New Year's first sunrise
    /// (sent, and still heading for a watch of it: it's hers once she
    /// watches, [`Osaka::sunrise_here`]).
    fn to_her_sunrise(&self) -> bool {
        let watch = |job: &Job| matches!(job, Job::Use(seat) if seat.what == Use::Watch);
        self.sunrise
            .is_some_and(|s| s.sent && Some(s.date) == self.real_date())
            && (self.heading.as_ref().is_some_and(|h| watch(&h.job))
                || matches!(&self.act, Act::Walk { then: Then::Job(job), .. } if watch(job)))
    }

    /// Whether she has let her calendar's greeting for `date` be this
    /// visit: frames kept missing it ([`CAL_MISSES`]).
    fn cal_let_be(&self, date: chrono::NaiveDate) -> bool {
        self.cal_missed
            .is_some_and(|(on, n)| on == date && n >= CAL_MISSES)
    }

    /// A wake time has passed since her day began (her night behind
    /// her), and she didn't wake into it from her night's sleep (she was
    /// up, or on her way back to bed, at her wake time): it begins as she
    /// next decides, at `at`, as if she'd woken (without the stretch).
    /// Keyed on the wake, not the date: a visit begun after midnight is
    /// on the morning's date already.
    fn catch_up_day(&mut self, at: u64) {
        let (Some(clock), Some(game), Some(from)) = (self.clock, self.game_at(at), self.day_from)
        else {
            return;
        };
        let day = clock.day_of(game);
        if day.slot != routine::Slot::Asleep && clock.next_morning(from) <= game {
            tracing::info!(%day, "houseguest: up at her wake time");
            self.begin_day(day, at);
        }
    }

    /// She stirs at a chat line in the night (A13): a murmur, a turn,
    /// and she sleeps on.
    fn stir(&mut self, now: u64) {
        tracing::info!("houseguest: stirs in her sleep");
        self.stir_saying(MM, now);
    }

    /// She stirs at `now`, murmuring `line`: turned over a moment,
    /// blinking, while she says it (a frame at least: [`speech_ms`]; see
    /// [`Osaka::stirring`]).
    fn stir_saying(&mut self, line: &'static str, now: u64) {
        self.say(line, now);
        self.stir_until = now + speech_ms(line);
    }

    /// A night act begins at `at` (her bed, her sofa, the floor; or a
    /// sleep become her night in place). Back in bed, she's no longer
    /// groggy. If it's night by her routine, her night is armed: her
    /// sleep-talk scheduled afresh from `at`, drawn from her latest
    /// decision's whims (the one that sent her to bed: a pure schedule of
    /// the act's start); and, the night's first, the Dream's count starts
    /// and her midnight snack is found. A later night act the same night
    /// (after an errand, or the snack; or on a later visit, carried by
    /// the guest) keeps both: the count runs on, the Dream come isn't
    /// come again, and a snack had (or missed while she was up) isn't had
    /// again.
    fn arm_night(&mut self, at: u64) {
        self.groggy = false;
        let (Some(clock), Some(game)) = (self.clock, self.game_at(at)) else {
            return;
        };
        if clock.day_of(game).slot != routine::Slot::Asleep {
            return;
        }
        // In for the night: past her glance at the clock (going back to
        // bed after a midnight snack, or an errand, she doesn't again).
        self.pass_glance(at);
        let (morning, _) = routine::split(clock.next_wake(game));
        let whims = self.whims;
        let mut night = match self.night.or(self.night_before) {
            Some(night) if night.morning == morning => night,
            _ => Night {
                morning,
                first: game,
                snack: brain::night_snack(self.master, morning)
                    .map(|minute| routine::game_of(morning, minute)),
                talk: whims,
                talk_k: 0,
                next_talk: at,
                dreamt: false,
            },
        };
        night.snack = night.snack.filter(|&snack| snack > game);
        night.talk = whims;
        night.talk_k = 0;
        night.next_talk = at + talk_gap(whims, 0);
        tracing::trace!(
            morning,
            first = night.first,
            snack = ?night.snack,
            next_talk = night.next_talk,
            "houseguest: her night armed"
        );
        self.night = Some(night);
    }

    /// She talks in her sleep at `at` (A14): her night act's next line,
    /// drawn from its starting decision's whims labelled by the line's
    /// place in the night act, not budgeted, each line cooling as any
    /// does; in the last game hour before she wakes, "...five more
    /// minutes". The next one is scheduled whether or not a line came
    /// (all cooling). Returns whether one did.
    fn sleep_talk(&mut self, at: u64) -> bool {
        let last_hour = self
            .night_play()
            .is_some_and(|(_, _, until)| at.saturating_add(LAST_HOUR_MS) >= until);
        let new_year = self.tints().new_year;
        let Some(night) = &mut self.night else {
            return false;
        };
        let k = night.talk_k;
        let whims = night.talk.series("sleep-talk", k);
        night.talk_k = k + 1;
        night.next_talk = at + talk_gap(night.talk, k + 1);
        let pool = if last_hour {
            mind::LAST_HOUR_TALK
        } else if new_year {
            mind::NEW_YEAR_TALK
        } else {
            mind::SLEEP_TALK
        };
        let line = self.lines.pick(pool, whims, at);
        if let Some(line) = line {
            tracing::info!(line, "houseguest: talks in her sleep");
            self.say(line, at);
        }
        line.is_some()
    }

    /// Her midnight snack's moment, `at` (round 1): with a fridge shown
    /// where she could get to it, she gets up and pads to it with the
    /// lamp off, and her routine's bed reflex takes her back after (see
    /// [`Osaka::send_to_bed`]). Without one, she sleeps on. Either way
    /// it's tonight's one chance. Returns whether she got up.
    fn midnight_snack(
        &mut self,
        at: u64,
        terrain: &Terrain,
        chances: &Chances,
        rng: &mut Rng,
    ) -> bool {
        if let Some(night) = &mut self.night {
            night.snack = None;
        }
        let fridge = chances
            .seats
            .iter()
            .find(|s| {
                s.what == Use::Snack && !s.makeshift() && terrain.platform_at(s.x, s.y).is_some()
            })
            .copied();
        let Some(fridge) = fridge else {
            tracing::trace!("houseguest: no fridge for a midnight snack");
            return false;
        };
        tracing::info!("houseguest: up for a midnight snack");
        self.snacking = Some(fridge.piece);
        self.decide(at, terrain, chances, rng);
        true
    }

    /// Her midnight snack at `seat`, her fridge, from `at`: the snack as
    /// any plays, built directly (no prelude or coda, no grievance felt,
    /// nothing recorded), as long as a snack lasts.
    fn night_snack_at(&mut self, seat: Seat, at: u64, rng: &mut Rng) -> Act {
        let (lo, hi) = use_duration(Use::Snack);
        let length = rng.range(lo, hi);
        tracing::info!(item = ?seat.item, "houseguest: a midnight snack");
        Act::Use {
            seat,
            since: at,
            until: at + length,
            whole: length,
            play: Play::of(Use::Snack, None),
            grievance: None,
        }
    }

    /// Dashed home from school (`dash`, D3a), deciding at `at` on floor
    /// `here`: to her fridge for her lunch, if one's shown where she can
    /// get to it (the one she was on her way to, or at, if she was); else
    /// she stands a moment, unable to remember what it was. Neither eases
    /// a need (no credit). What the stage cued for a dash is taken here (a
    /// cue for the forgetting forgets with a fridge to hand), so it never
    /// waits on for a later snack. Already set on her lunch, its fridge
    /// gone (into the closet) or out of her reach, she gives up on it:
    /// `None`, her dash over, and her routine sends her out again (as her
    /// midnight snack's sends her back to bed: see
    /// [`Osaka::send_to_bed`]).
    fn dash_on(
        &mut self,
        dash: Dash,
        here: usize,
        terrain: &Terrain,
        chances: &Chances,
        at: u64,
    ) -> Option<Decision> {
        let cued = self
            .cued
            .take_if(|cue| matches!(cue, Cue::Script(ScriptId::DashLunch | ScriptId::DashForgot)));
        self.credit = None;
        // Home only for what she forgot: her routine sends her out again
        // after, with no glance at the clock ("Time for school!" is for
        // her morning's leaving).
        self.pass_glance(at);
        let want = Want::Use(Use::Snack);
        if let Dash::Lunch(fridge) = dash {
            if let Some(seat) = fridge_seat(fridge, chances)
                && self.go_to(want, Job::Use(seat), here, terrain, at)
            {
                return Some(Decision::reflex("dash/lunch"));
            }
            tracing::debug!("houseguest: no way to her fridge for her lunch; out again");
            self.dash = None;
            return None;
        }
        let fridge = chances
            .seats
            .iter()
            .find(|s| {
                s.what == Use::Snack && !s.makeshift() && terrain.platform_at(s.x, s.y).is_some()
            })
            .copied()
            .filter(|_| cued != Some(Cue::Script(ScriptId::DashForgot)));
        if let Some(seat) = fridge {
            self.dash = Some(Dash::Lunch(seat.piece));
            if self.go_to(want, Job::Use(seat), here, terrain, at) {
                return Some(Decision::reflex("dash/lunch"));
            }
            tracing::debug!("houseguest: no way to her fridge for her lunch");
        }
        self.dash = None;
        tracing::info!("houseguest: dashed home for something, but what?");
        self.set(
            Act::SpaceOut {
                since: at,
                until: at + DASH_FORGOT_MS,
                play: Some(Play::plain(ScriptId::DashForgot)),
                session: None,
            },
            at,
        );
        Some(Decision::reflex("dash/forgot"))
    }

    /// The seasons of the real date she was last given (none without a
    /// date, or without her routine).
    fn tints(&self) -> Tints {
        self.real_date()
            .map(|date| calendar::entries(date).1)
            .unwrap_or_default()
    }

    /// The real date she was last given (the guest's, at her tick's
    /// entry): `None` without one, and without her routine (unfed, she
    /// knows no date).
    fn real_date(&self) -> Option<chrono::NaiveDate> {
        self.clock?.date
    }

    /// Her calendar as her visit begins: the date she last delivered its
    /// owed entry on (her ledger's).
    pub fn set_calendar(&mut self, done: Option<chrono::NaiveDate>) {
        self.cal_done = done;
    }

    /// What her calendar owes her at `at`, and for which date: that date's
    /// entry, unless she has delivered it, or it's under way (said or
    /// begun, and showing). Checked as she decides, against the date she
    /// was last given. `None` without a date.
    pub(super) fn calendar_owed(&self, at: u64) -> Option<(chrono::NaiveDate, Owed)> {
        let date = self.real_date()?;
        if self.cal_done == Some(date) || self.cal_under_way(date, at) {
            return None;
        }
        Some((date, calendar::entries(date).0?))
    }

    /// Whether her calendar's entry for `date` is under way at `at`: its
    /// greeting still being said, or its script playing.
    fn cal_under_way(&self, date: chrono::NaiveDate, at: u64) -> bool {
        self.cal_showing
            .is_some_and(|s| s.date == date && self.cal_on(s.what, at))
    }

    /// Whether `what` is on her at `at`: the line being said, the script
    /// playing.
    fn cal_on(&self, what: CalShows, at: u64) -> bool {
        match what {
            CalShows::Line(line) => self
                .speech
                .is_some_and(|(said, until)| said == line && at < until),
            CalShows::Script(id) => self.plays().is_some_and(|p| p.own == id),
        }
    }

    /// The greeting her calendar owes her at `at`, if any, with its date
    /// and entry: the owed entry's, unless she has let it be this visit
    /// (frames kept missing it: [`CAL_MISSES`]), or she said it here and
    /// it went unseen (said again once she has moved, never in place
    /// over and over where her bubble can't go).
    fn calendar_greeting(&self, at: u64) -> Option<(chrono::NaiveDate, Owed, &'static str)> {
        let (date, owed) = self.calendar_owed(at)?;
        let line = owed.greeting()?;
        let let_be = self.cal_let_be(date);
        // Under way it isn't owed (above): any saying of it left is one
        // that went unseen.
        let unseen_here = self
            .cal_showing
            .is_some_and(|s| s.date == date && s.spot == (self.x, self.y));
        (!let_be && !unseen_here).then_some((date, owed, line))
    }

    /// Say her calendar's greeting at `at`: delivered as it shows.
    fn say_calendar(
        &mut self,
        (date, owed, line): (chrono::NaiveDate, Owed, &'static str),
        at: u64,
    ) {
        tracing::info!(%date, line, "houseguest: her calendar's greeting");
        self.say_noted(PoolId::Calendar, line, at);
        self.cal_showing = Some(CalShowing {
            date,
            owed,
            what: CalShows::Line(line),
            spot: (self.x, self.y),
            missed: false,
        });
    }

    /// What her calendar owes her at `at`, as the first beat she owes
    /// (after "I'm home!", after a drop-in's "...I'm OK.", after her
    /// plain "Mornin'.", or on screen as the date changes), deciding on
    /// floor `here`: its greeting, spacing out a moment; else Setsubun's
    /// beans on the spot; else New Year's first sunrise, to her TV. Only
    /// what can be done now, and once she's quiet (she stands until what
    /// she's saying is said). Never in the middle of moving a piece of
    /// her home, or on her way somewhere: it waits for that to settle.
    /// `None` when there's nothing it can do now.
    fn calendar_beat(
        &mut self,
        here: usize,
        terrain: &Terrain,
        chances: &Chances,
        at: u64,
    ) -> Option<Decision> {
        let CalendarDue {
            greeting,
            setsubun,
            tv,
        } = self.calendar_due(chances, terrain, at)?;
        // Not over what she's saying ("I'm home!", "...I'm OK.",
        // "Mornin'."): she stands until she's said it, and it's next.
        if let Some((_, until)) = self.speech.filter(|&(_, until)| until > at) {
            self.set(Act::Stand { until }, at);
            return Some(Decision::of(Bucket::Owed, "calendar/wait"));
        }
        if let Some(greeting) = greeting {
            self.say_calendar(greeting, at);
            self.set(
                Act::SpaceOut {
                    since: at,
                    until: at + speech_ms(greeting.2).max(GREETING_PAUSE_MS),
                    play: None,
                    session: None,
                },
                at,
            );
            return Some(Decision::of(Bucket::Owed, "calendar"));
        }
        if let Some((date, owed)) = setsubun {
            tracing::info!(%date, "houseguest: Setsubun's beans");
            self.cal_showing = Some(CalShowing {
                date,
                owed,
                what: CalShows::Script(ScriptId::Setsubun),
                spot: (self.x, self.y),
                missed: false,
            });
            self.set(Self::setsubun(at), at);
            return Some(Decision::of(Bucket::Owed, "calendar"));
        }
        let (tv, sunrise) = tv.zip(self.sunrise)?;
        if !self.go_to(Want::Use(Use::Watch), Job::Use(tv), here, terrain, at) {
            return None;
        }
        tracing::info!(date = %sunrise.date, "houseguest: to her TV, for the first sunrise");
        self.sunrise = Some(SunriseOwed {
            sent: true,
            ..sunrise
        });
        self.credit = Some(Want::Use(Use::Watch));
        Some(Decision::of(Bucket::Owed, "calendar/sunrise"))
    }

    /// What her calendar has for her to do at `at` (see
    /// [`Osaka::calendar_beat`], which does it), if anything: never in
    /// the middle of moving a piece of her home, or on her way somewhere.
    fn calendar_due(&self, chances: &Chances, terrain: &Terrain, at: u64) -> Option<CalendarDue> {
        if self.episode.is_some() || self.just_set.is_some() || self.heading.is_some() {
            return None;
        }
        let greeting = self.calendar_greeting(at);
        let setsubun = self
            .calendar_owed(at)
            .filter(|&(_, owed)| owed == Owed::Setsubun);
        // Her sunrise, if it's owed this visit and her TV is where she
        // can get to it.
        let tv = self.sunrise_tv(chances, terrain);
        (greeting.is_some() || setsubun.is_some() || tv.is_some()).then_some(CalendarDue {
            greeting,
            setsubun,
            tv,
        })
    }

    /// Setsubun's beans from `at`, on the spot: always as long.
    fn setsubun(at: u64) -> Act {
        Act::SpaceOut {
            since: at,
            until: at + SETSUBUN_MS,
            play: Some(Play::plain(ScriptId::Setsubun)),
            session: None,
        }
    }

    /// Whether a watch of hers starting at `at` (`cued` by the stage or
    /// not) is her first sunrise: cued to it; or, uncued, it's owed this
    /// visit, on this date (then it's played, and owed no longer).
    fn sunrise_here(&mut self, cued: Option<Cue>, at: u64) -> bool {
        if cued == Some(Cue::Script(ScriptId::FirstSunrise)) {
            self.lines.try_play(ScriptId::FirstSunrise, at);
            return true;
        }
        let today = self.real_date();
        match self.sunrise {
            Some(sunrise) if cued.is_none() && Some(sunrise.date) == today => {
                tracing::info!(date = %sunrise.date, "houseguest: her first sunrise");
                self.sunrise = None;
                true
            }
            _ => false,
        }
    }

    /// The frame at `now` shows her (the guest calls it as it paints her,
    /// whatever the drawing; `drawn`: the bubble it drew her, if any):
    /// her calendar's entry under way has shown if she's in sight and
    /// it's on her now, its line the bubble drawn (not under something
    /// else, nor squeezed out), or its script playing. Shown, it's
    /// delivered: recorded ([`HomeEvent::Calendar`]), and not owed again
    /// that date; New Year's Day's first sunrise follows, this visit (see
    /// [`Osaka::sunrise`]). A frame that shows her with its line on her
    /// but not drawn counts against it ([`CAL_MISSES`]). Returns the date
    /// delivered.
    pub fn shown(&mut self, now: u64, drawn: Option<Bubble>) -> Option<chrono::NaiveDate> {
        let showing = self.cal_showing?;
        if self.hidden(now) || !self.cal_on(showing.what, now) {
            return None;
        }
        if let CalShows::Line(line) = showing.what
            && drawn != Some(Bubble::Say(line))
        {
            if !showing.missed {
                let missed = match self.cal_missed {
                    Some((on, n)) if on == showing.date => n.saturating_add(1),
                    _ => 1,
                };
                tracing::debug!(line, missed, "houseguest: her calendar's line unseen");
                self.cal_missed = Some((showing.date, missed));
                self.cal_showing = Some(CalShowing {
                    missed: true,
                    ..showing
                });
            }
            return None;
        }
        self.cal_showing = None;
        if showing.owed == Owed::FirstSunrise {
            self.sunrise = Some(SunriseOwed {
                date: showing.date,
                sent: false,
            });
        }
        tracing::info!(date = %showing.date, "houseguest: her calendar's entry delivered");
        self.cal_done = Some(showing.date);
        self.events.push(HomeEvent::Calendar(showing.date));
        Some(showing.date)
    }

    /// The branch she looks out of the window on at `at`: one of the
    /// lines of the sky her clock shows (unfed, the day's: her window
    /// shows a day sky then), drawn from her latest decision's whims.
    fn look_out_branch(&self, at: u64) -> u8 {
        // Six: as even over two lines as over three.
        script::look_out_branch(self.sky_at(at), self.whims.below_at("look-out", 0, 6))
    }

    /// The sky her window shows at `at`, by her clock (unfed, the day's).
    /// The one reading for her look-out's line, her musings at the sill
    /// and her line watching the clouds.
    fn sky_at(&self, at: u64) -> super::art::Sky {
        self.day(at)
            .map_or(super::art::Sky::Day, |day| super::art::Sky::at(day.minute))
    }

    /// Her meal's line, on her first snack of a morning or an evening by
    /// her routine (`day`), at `at`: once a slot.
    fn meal(&mut self, day: DayTime, at: u64) {
        let pool = match day.slot {
            routine::Slot::Morning => mind::BREAKFAST,
            routine::Slot::Evening => mind::DINNER,
            _ => return,
        };
        if self.meal_said == Some((day.day, day.slot)) {
            return;
        }
        self.meal_said = Some((day.day, day.slot));
        self.say_pool(pool, at);
    }

    /// The game day and slot whose first snack had its meal's line (the
    /// guest carries it to her next visit).
    pub fn meal_said(&self) -> Option<(u64, routine::Slot)> {
        self.meal_said
    }

    /// Her night so far, while it's night and she has slept some of it
    /// (the guest keeps it for her next visit: see
    /// [`Osaka::carry_night`]).
    pub fn night(&self) -> Option<Night> {
        self.night
    }

    /// Her night as an earlier visit left it: if this visit's first night
    /// act is the same night (keyed on its morning), it carries on from
    /// it, the Dream's count and what's once a night included, rather
    /// than beginning the night afresh. Held apart from her own night
    /// until then, so arriving in the night isn't sleeping it.
    pub fn carry_night(&mut self, night: Option<Night>) {
        self.night_before = night;
    }

    /// Her meal's line was said on an earlier visit, at `said`.
    pub fn carry_meal(&mut self, said: Option<(u64, routine::Slot)>) {
        self.meal_said = said;
    }

    /// What she got up or came home for, had to its end by `at` (her act
    /// is its use, run its course), is done: the lunch she dashed home
    /// for, her midnight snack. Called as her act ends, however it ends
    /// (see [`Osaka::set`]), and as she decides (the act just finished
    /// still hers), so a use that ran its course is had even when a look
    /// replaces it before her tick for its end has come. Cut short (a
    /// chat line she looks at, a startle), it isn't: deciding next, she
    /// goes back to it, as she does when something stops her on her way
    /// (a lunch cut just after its line has shown is had again, line and
    /// all; never lost), unless its fridge has gone.
    fn got_what_she_came_for(&mut self, at: u64) {
        let Act::Use { seat, until, .. } = self.act else {
            return;
        };
        let had = |fridge: PieceRef| at >= until && fridge == seat.piece && seat.what == Use::Snack;
        if self.snacking.is_some_and(had) {
            tracing::trace!("houseguest: her midnight snack had");
            self.snacking = None;
        }
        if let Some(Dash::Lunch(fridge)) = self.dash
            && had(fridge)
        {
            tracing::trace!("houseguest: the lunch she dashed home for had");
            self.dash = None;
        }
    }

    /// Her lunch at `seat`, her fridge, from `at`, dashed home for (D3a):
    /// built directly (no prelude or coda, no grievance felt, nothing
    /// recorded, nothing drawn), always as long.
    fn dash_lunch_at(seat: Seat, at: u64) -> Act {
        tracing::info!(item = ?seat.item, "houseguest: her lunch, forgotten");
        Act::Use {
            seat,
            since: at,
            until: at + DASH_LUNCH_MS,
            whole: DASH_LUNCH_MS,
            play: Play::plain(ScriptId::DashLunch),
            grievance: None,
        }
    }

    /// When the Dream would come tonight, in monotonic millis: 30 game
    /// minutes after her first sleep of the night, counted across
    /// whatever got her up since. `None` with no night by her routine.
    fn dream_at(&self) -> Option<u64> {
        let first = self.night?.first;
        Some(self.clock?.game.when(first + DREAM_AFTER_MS))
    }

    /// Tonight's Dream at `at` (step 7): her night's sleep turns to it in
    /// place, on whatever she sleeps on, if there's room for all of it
    /// before she wakes; her sleep-talk waits for it to be said. Once a
    /// night either way. Returns whether it came.
    fn dream(&mut self, at: u64) -> bool {
        if let Some(night) = &mut self.night {
            night.dreamt = true;
            night.next_talk = night.next_talk.max(at + script::DREAM_MS);
        }
        let room = self
            .night_play()
            .is_some_and(|(_, _, until)| until.saturating_sub(at) >= script::DREAM_MS);
        if !room {
            tracing::trace!("houseguest: no time left tonight for the Dream");
            return false;
        }
        let mut act = self.act.clone();
        self.dream_from(&mut act, at);
        self.act = act;
        self.act_due = self.first_due(at);
        tracing::info!("houseguest: the Dream");
        self.note_seen(at);
        true
    }

    /// Her night act `act` dreamt from `at` on: the Dream's branch for
    /// the surface it's on, from its first line, until she wakes as
    /// before. In place: no new act (her sleep, her wake time and her
    /// night's count are as they were), only what it plays and from when.
    /// Tonight's Dream has come. Whatever she was saying stops for it
    /// (a murmur, or on the stage what she said on her way to bed), so
    /// it begins on its first line.
    fn dream_from(&mut self, act: &mut Act, at: u64) {
        if let Some(night) = &mut self.night {
            night.dreamt = true;
        }
        self.hush(at);
        // Its part begins at `at`: its bob keeps clear of that, as any
        // key's start, and no hold of the night's carries into it.
        self.bob_held = None;
        let dreamt = |play: Play| Play {
            branch: Surface::of_branch(play.branch).dream_branch(),
            ..Play::plain(ScriptId::Dream)
        };
        match act {
            Act::Use {
                since,
                until,
                whole,
                play,
                ..
            } => {
                *since = at;
                *whole = until.saturating_sub(at);
                *play = dreamt(*play);
            }
            Act::Idle {
                since,
                play: Some(play),
                ..
            } => {
                *since = at;
                *play = dreamt(*play);
            }
            _ => {}
        }
    }

    /// What's rare and open as her visit begins (`rares`, drawn for game
    /// `day`, or unfed the visit's), and the rare scripts she has shown
    /// (her ledger's).
    pub fn set_rares(&mut self, rares: Rares, day: Option<u64>, seen: Vec<ScriptId>) {
        tracing::debug!(?rares, ?day, ?seen, "houseguest: what's rare");
        self.rares = rares;
        self.rares_day = day;
        self.seen = seen;
    }

    /// Her pity counters as her ledger has them now (the guest gives them
    /// before each tick: a new day's draw reads them).
    pub fn set_pity(&mut self, pity: Pity) {
        self.pity = pity;
    }

    /// What's rare and open today, with its game day, her routine fed
    /// (the guest carries it to the day's next visit, so a day is drawn
    /// once).
    pub fn rares_today(&self) -> Option<(u64, &Rares)> {
        self.rares_day.map(|day| (day, &self.rares))
    }

    /// What's rare and open (tests).
    #[cfg(test)]
    pub(super) fn rares(&self) -> &Rares {
        &self.rares
    }

    /// The rare script whose key plays at `at`, if one does: the part of
    /// her act's play then (its prelude, its own script or its coda).
    fn rare_playing(&self, at: u64) -> Option<ScriptId> {
        let (play, since, until) = match self.act {
            Act::Use {
                play, since, until, ..
            } => (play, since, until),
            Act::SpaceOut {
                play: Some(play),
                since,
                until,
                ..
            }
            | Act::Idle {
                play: Some(play),
                since,
                until,
                ..
            } => (play, since, until),
            _ => return None,
        };
        let id = play
            .spliced_at(since, until, at)
            .map_or(play.own, |s| s.splice.script());
        id.rarity().gated().then_some(id)
    }

    /// Seen means first shown: a rare script's key playing at `at` that
    /// she has never shown is seen now ([`HomeEvent::Seen`]), and her
    /// pity for its tier starts again. Not as it's planned, or offered: a
    /// walk to it cut short leaves it unseen. (She's in sight whenever
    /// one plays: [`Osaka::rare_playing`] reads only acts she's seen
    /// doing, never a door's or being away.)
    fn note_seen(&mut self, at: u64) {
        let Some(id) = self.rare_playing(at) else {
            return;
        };
        if self.seen.contains(&id) {
            return;
        }
        tracing::info!(?id, "houseguest: something rare, for the first time");
        self.seen.push(id);
        self.pity.reset(id.rarity());
        self.events.push(HomeEvent::Seen(id));
    }

    /// What she has shown of what's rare (tests).
    #[cfg(test)]
    pub(super) fn seen(&self) -> &[ScriptId] {
        &self.seen
    }

    /// The rare script whose key plays at `now`, if one does (tests).
    #[cfg(test)]
    pub(super) fn rare_showing(&self, now: u64) -> Option<ScriptId> {
        self.rare_playing(now)
    }

    /// Whether she may go to work at `at` (D2): the one place it's
    /// decided. A furnished home; no shift yet this visit, nor since her
    /// day began (`worked` is reset by each new visit and each new game
    /// day, so a second visit the same Saturday may work again); nothing
    /// of her home in hand; and the job open. With her routine, on a day
    /// off from 10:00 to 17:00 ([`DayTime::work_open`]); without it, once
    /// she's been here a while.
    fn may_work(&self, chances: &Chances, at: u64) -> bool {
        chances.furnished
            && !self.worked
            && self.episode.is_none()
            && match self.day(at) {
                Some(day) => day.work_open(),
                None => at >= self.arrived + WORK_AFTER_MS,
            }
    }

    /// Whether she's drowsy at `now`: asleep for the night (the stage's
    /// by day too), or it's night by her routine (up groggy included:
    /// she only ever is in the night). A goodbye then is a sleepy blink,
    /// not startled.
    pub fn drowsy(&self, now: u64) -> bool {
        self.sleeping() || self.asleep_slot(now)
    }

    /// An errand brought her here in the night (`now`): she was asleep
    /// off screen, so she's up groggy, and back to bed after.
    pub fn groggy_if_night(&mut self, now: u64) {
        if self.on_errand() && self.asleep_slot(now) {
            tracing::info!("houseguest: come half asleep for the accordion");
            self.groggy = true;
        }
    }

    /// Whether she's up groggy at `now`: got up in the night, and it's
    /// still the night. The one way anything reads `groggy`, so it never
    /// shows past her wake time, even before her day begins (as she
    /// next decides: see [`Osaka::catch_up_day`]).
    fn groggy_at(&self, now: u64) -> bool {
        self.groggy && self.asleep_slot(now)
    }

    /// Whether she's up groggy, as stored (tests: cleared back in bed, or
    /// as her day begins).
    #[cfg(test)]
    pub(super) fn groggy(&self) -> bool {
        self.groggy
    }

    /// When the Dream would come tonight, open or not (tests; see
    /// `dream_at`).
    #[cfg(test)]
    pub(super) fn dream_moment(&self) -> Option<u64> {
        self.dream_at()
    }

    /// Her home's master seed, for her mornings' new days, and the game
    /// day the visit begins on (`None`: no routine reaches her): set as a
    /// visit begins.
    pub fn key_days(&mut self, master: u64, day: Option<u64>) {
        self.master = master;
        self.budget_day = day;
    }

    /// Her line budget as it stands, with the routine fed: the game day
    /// it's that day's, and the beat lines she has said that day.
    pub fn line_budget(&self) -> Option<(u64, usize)> {
        self.budget_day.map(|day| (day, self.lines.spent()))
    }

    /// She said `spent` beat lines earlier today, on another visit: her
    /// budget is the day's (her routine fed).
    pub fn carry_lines(&mut self, spent: usize) {
        self.lines.carry(spent);
    }

    /// The next beat of borrowing a strip of `pull` to read, in `phase`
    /// now (phase 5c D5, HG #72): hand over hand, the strip comes in
    /// off the line (a cell a step, as she tears text for furniture);
    /// she reads it sat beside the tear, as long as a read at a
    /// bookshelf and lingered by her mood as one is; then she slides it
    /// back the way it came, and once the line is whole, chooses again.
    fn borrowing(
        &mut self,
        pull: Pull,
        phase: Borrowing,
        at: u64,
        terrain: &Terrain,
        chances: &Chances,
        rng: &mut Rng,
    ) {
        let steps = pull.strip_steps();
        let next = match phase {
            Borrowing::Brace => {
                tracing::info!(
                    row = pull.row,
                    glyphs = pull.strip().len(),
                    side = ?pull.side,
                    "houseguest: borrowing a strip of a line to read"
                );
                Borrowing::Reel(1)
            }
            Borrowing::Reel(step) if step < steps => Borrowing::Reel(step + 1),
            Borrowing::Reel(_) => {
                let slot = self.day(at).map(|day| day.slot);
                let (lo, hi) = use_duration_in(Use::Read, slot);
                let until = at
                    + self
                        .stillness
                        .lingered_use(self.mood, Use::Read, rng.range(lo, hi));
                tracing::debug!(until, "houseguest: reading the strip beside the tear");
                Borrowing::Read { until }
            }
            Borrowing::Read { until } if at < until => {
                self.act_due = self.first_due(at);
                return;
            }
            Borrowing::Read { .. } => {
                tracing::debug!("houseguest: sliding the strip back");
                // Nothing turns her from the line while she reads it (a
                // chat line has her look up without turning: phase 5c
                // step 8c).
                debug_assert_eq!(self.facing, side_facing(pull.side), "still facing the line");
                Borrowing::Slide(steps.saturating_sub(1))
            }
            Borrowing::Slide(0) => return self.decide(at, terrain, chances, rng),
            Borrowing::Slide(step) => Borrowing::Slide(step - 1),
        };
        match next {
            Borrowing::Reel(step) => self.ops.push(LayerOp::Reel {
                row: pull.row,
                cells: pull.strip(),
                hand: pull.hand(),
                step,
            }),
            Borrowing::Slide(step) => self.ops.push(LayerOp::Unreel {
                row: pull.row,
                cells: pull.strip(),
                hand: pull.hand(),
                step,
            }),
            Borrowing::Brace | Borrowing::Read { .. } => {}
        }
        self.set(
            Act::Borrow {
                pull,
                since: at,
                phase: next,
            },
            at,
        );
    }

    fn finish_pull(&mut self, at: u64) {
        self.credit_whole(Want::Pull, Via::Whole, at);
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
        // Leaving her night's sleep, the time asleep is kept for her
        // needs (A11), counted once.
        self.count_sleep(at);
        let Some(want) = self.credit else {
            return;
        };
        let span = |since: u64, until: u64| {
            (at.saturating_sub(since) as f64 / until.saturating_sub(since).max(1) as f64)
                .clamp(0.0, 1.0)
        };
        // Her night act runs from when she lay down (`act_since`: the
        // Dream turns its play in place, re-timing it from the Dream on,
        // never her night's start) to her wake.
        let night = self.sleeping();
        let (done, spot) = match (&self.act, want) {
            (
                Act::Idle {
                    what, since, until, ..
                },
                Want::Idle(chose),
            ) if *what == chose => {
                let since = if night { self.act_since } else { *since };
                (span(since, *until), Spot::Floor)
            }
            // By the share of its body done: none of it in a prelude,
            // all of it in a coda.
            (
                Act::Use {
                    seat,
                    since,
                    until,
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
                let (start, end) = if night {
                    (play.body_start(self.act_since), *until)
                } else {
                    let start = play.body_start(*since);
                    (start, start + whole)
                };
                (span(start, end), spot)
            }
            (Act::Pull { offset, goal, .. }, Want::Pull) => {
                (f64::from(*offset) / f64::from((*goal).max(1)), Spot::Any)
            }
            // A strip she borrowed to read: by the share of her reading
            // done (none before she sits down with it, all of it once
            // she's sliding it back), on the floor.
            (Act::Borrow { since, phase, .. }, Want::Use(Use::Read)) => {
                let done = match *phase {
                    Borrowing::Brace | Borrowing::Reel(_) => 0.0,
                    Borrowing::Read { until } => span(*since, until),
                    Borrowing::Slide(_) => 1.0,
                };
                (done, Spot::Floor)
            }
            // A glance up at her clock, though her musing chose it, is a
            // glance: it eases nothing.
            (
                Act::SpaceOut {
                    play: Some(play), ..
                },
                Want::SpaceOut,
            ) if play.own == ScriptId::ClockGlance => {
                self.credit = None;
                return;
            }
            // Spacing out (musing, telling a riddle) answers her
            // daydreams, by the share of it done.
            (Act::SpaceOut { since, until, .. }, Want::SpaceOut) => {
                (span(*since, *until), Spot::Any)
            }
            _ => return,
        };
        self.credit = None;
        #[cfg(test)]
        self.credited.push((want, done, at));
        // A pull's share is of the goal she reached (let go short of it);
        // whole, it's credited as she finishes it.
        let via = if want == Want::Pull {
            Via::Whole
        } else {
            Via::Share
        };
        self.serve(want, done, spot, via, at);
        let restful = match self.act {
            Act::Idle { what, .. } => what.restful(),
            // Unpacking a parcel and crumpling text are chores, not using
            // her things.
            Act::Use { seat, .. } => !matches!(seat.what, Use::Unpack | Use::Crumple),
            // Reading the strip she borrowed, sat on the floor.
            Act::Borrow { .. } => true,
            // Spacing out is done on her feet: not resting.
            _ => false,
        };
        if restful {
            self.needs
                .serve(Need::Beauty, done * self.beauty_here.min(1.0));
        }
    }

    /// She did all of what she chose, `want`, at `at` (eased `via`).
    fn credit_whole(&mut self, want: Want, via: Via, at: u64) {
        self.credit_share(want, 1.0, via, at);
    }

    /// She did `share` of what she chose, `want`, at `at` (eased `via`):
    /// none of it settles it unserved.
    fn credit_share(&mut self, want: Want, share: f64, via: Via, at: u64) {
        if self.credit == Some(want) {
            self.credit = None;
            if share > 0.0 {
                self.serve(want, share, Spot::Any, via, at);
            }
        }
    }

    /// `want`'s needs eased at `at` by `share` of what it answers, as
    /// well as `spot` answers each (eased `via`): on her needs as they
    /// are at `at`, brought up to it first ([`Osaka::rise_to`]), not as
    /// they were when she last chose (where an easing landed on needs
    /// that went on rising over it, and a need that rose to full
    /// meanwhile came back full).
    fn serve(&mut self, want: Want, share: f64, spot: Spot, via: Via, at: u64) {
        self.rise_to(at);
        tracing::trace!(
            ?want,
            share,
            ?spot,
            ?via,
            at,
            "houseguest: eased by what she did"
        );
        #[cfg(test)]
        self.served.push(Served {
            want,
            share,
            decision: self.decisions.len(),
            via,
            spot,
        });
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

    /// How what she chooses, `want` by `method` (a name in
    /// [`mind::methods`], or [`SETTLE_IN`]: settled into where she is,
    /// from a still act she chose), comes to ease what it serves: every
    /// want that serves anything has a way, so none is chosen for
    /// nothing. Matched with no wildcard over wants, so a new one won't
    /// compile without its way; a method this doesn't know is `None` (the
    /// class test fails on it).
    #[cfg(test)]
    pub(super) fn credit_path(want: Want, method: &str) -> Option<CreditPath> {
        match want {
            Want::Stand | Want::Sneeze | Want::Use(Use::Crumple) => Some(CreditPath::Nothing),
            Want::SpaceOut => match method {
                "space-out" | "space-out/muse" => Some(CreditPath::By(Via::Share)),
                _ => None,
            },
            // What she settles into is credited by its share, as
            // chosen.
            Want::Idle(Activity::Sit | Activity::LieBack) => match method {
                "idle" | SETTLE_IN => Some(CreditPath::By(Via::Share)),
                _ => None,
            },
            Want::Idle(
                Activity::LieFront
                | Activity::Jacks
                | Activity::ToeTouch
                | Activity::Stretch
                | Activity::Gaze,
            ) => (method == "idle").then_some(CreditPath::By(Via::Share)),
            // Never chosen: only settled into, from a sit, or from
            // reading on her back.
            Want::Idle(
                Activity::SitDoze | Activity::BookDoze | Activity::UnderSill | Activity::CloudWatch,
            ) => (method == SETTLE_IN).then_some(CreditPath::By(Via::Share)),
            Want::Idle(Activity::FloorHomework) => match method {
                "floor-homework" | "floor-homework/book" => Some(CreditPath::By(Via::Share)),
                _ => None,
            },
            Want::Idle(Activity::LieRead) => {
                (method == "lie-read").then_some(CreditPath::By(Via::Share))
            }
            Want::Walk => (method == "walk/along").then_some(CreditPath::By(Via::SetOff)),
            Want::Travel => match method {
                "travel/link" | "travel/door" => Some(CreditPath::By(Via::SetOff)),
                _ => None,
            },
            Want::Work => (method == "work").then_some(CreditPath::By(Via::Shift)),
            Want::Pull => (method == "pull").then_some(CreditPath::By(Via::Whole)),
            Want::Swap => (method == "swap").then_some(CreditPath::By(Via::Whole)),
            Want::Use(Use::Nap) if method == SETTLE_IN => Some(CreditPath::By(Via::Share)),
            // A strip borrowed off a line, where no bookshelf stands: by
            // the share of her reading done, as she leaves it.
            Want::Use(Use::Read) if method == "use/borrow" => Some(CreditPath::By(Via::Share)),
            Want::Use(
                Use::Lounge
                | Use::Nap
                | Use::Sleep
                | Use::Homework
                | Use::Watch
                | Use::Unpack
                | Use::Read
                | Use::Snack
                | Use::Pet
                | Use::LookOut,
            ) => match method {
                // Making one first: the use of it is credited, as she
                // gets to it (by the leftover reflex).
                "use/finish-my-heap" | "use/mine" | "use/real" | "use/made" | "use/make" => {
                    Some(CreditPath::By(Via::Share))
                }
                _ => None,
            },
            Want::Arrange => match method {
                // Each step of moving the piece is the one thing done
                // about her home, credited as the frame takes it; sitting
                // back down to it is a use, credited as one.
                "arrange/lift" | "arrange/carry" => Some(CreditPath::By(Via::SetDown)),
                "arrange/use-it" => Some(CreditPath::AsUse),
                _ => None,
            },
        }
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

    /// The text she holds torn off its line (or is reeling in, or
    /// sliding back), while she does: text for a makeshift piece, or a
    /// strip she borrowed to read. The same all the while she's at it,
    /// so the guest can tell the moment she isn't, however that came
    /// about, and put back what of it is still out.
    pub fn holding(&self) -> Option<Held> {
        match &self.act {
            Act::Tear { build, .. } => Some(Held::Build(build.clone())),
            Act::Borrow { pull, .. } => Some(Held::Strip(pull.clone())),
            _ => None,
        }
    }

    /// Her grip on the text her act is at, while she has one (see
    /// [`Grip`]): the line as she found it while she takes hold (a pull
    /// not yet heaved, a tear or a borrow braced for), then every glyph
    /// she has out of it (heaving, reeling in, reading the strip, sliding
    /// it back until the last of it is home). The guest checks it each
    /// paint: slipped, she's lost her grip ([`Osaka::lost_grip`]).
    pub fn grip(&self) -> Option<Grip<'_>> {
        let hand =
            |row: u16, cells: &[u16]| Grip::InHand(cells.iter().map(|&c| (c, row)).collect());
        match &self.act {
            Act::Pull {
                pull, offset: 0, ..
            } => Some(Grip::Line {
                row: pull.row,
                cells: &pull.cells,
                glyphs: &pull.glyphs,
            }),
            Act::Pull { pull, .. } => Some(hand(pull.row, &pull.cells)),
            Act::Tear {
                build,
                ripped: false,
                ..
            } => Some(Grip::Line {
                row: build.row,
                cells: &build.cells,
                glyphs: &build.glyphs,
            }),
            Act::Tear { build, .. } => Some(hand(build.row, &build.cells)),
            Act::Borrow {
                pull,
                phase: Borrowing::Brace,
                ..
            } => Some(Grip::Line {
                row: pull.row,
                cells: &pull.cells,
                glyphs: &pull.glyphs,
            }),
            Act::Borrow {
                phase: Borrowing::Slide(0),
                ..
            } => None,
            Act::Borrow { pull, .. } => Some(hand(pull.row, &pull.strip())),
            _ => None,
        }
    }

    /// The makeshift piece she's tearing text off for, while she is.
    #[cfg(test)]
    pub fn reeling(&self) -> Option<&Build> {
        match &self.act {
            Act::Tear { build, .. } => Some(build),
            _ => None,
        }
    }

    /// The line she's reading a strip of, sat beside the tear, while she
    /// is: the whole strip is in her hands now (the guest checks it still
    /// is: see [`Osaka::grip`]).
    #[cfg(test)]
    pub fn reading_strip(&self) -> Option<&Pull> {
        match &self.act {
            Act::Borrow {
                pull,
                phase: Borrowing::Read { .. },
                ..
            } => Some(pull),
            _ => None,
        }
    }

    /// Whether her TV's programme is coming or on (phase 5c D7): she's
    /// on her way to watch (any watch: whether it'll be the shopping
    /// channel or her first sunrise is only chosen as it starts, so a
    /// walk to one asks for a still it won't show), or watching, or
    /// flicking through the channels to rest on it (not the shopping
    /// channel or her first sunrise, which show their own pictures).
    /// Output only: the shell fetches the film's still by it, and
    /// nothing of hers reads it.
    pub fn tv_bound(&self) -> bool {
        let watch = |job: &Job| matches!(job, Job::Use(seat) if seat.what == Use::Watch);
        if self.heading.as_ref().is_some_and(|h| watch(&h.job)) {
            return true;
        }
        match &self.act {
            Act::Walk {
                then: Then::Job(job),
                ..
            } => watch(job),
            Act::Use { seat, play, .. } => {
                seat.what == Use::Watch && matches!(play.own, ScriptId::Watch | ScriptId::Surf)
            }
            _ => false,
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
    /// (static and Chiyo-chichi's hook moving every [`CHANNEL_FRAME_MS`]
    /// from the start of the part it plays in: the prelude, the body or
    /// the coda, so a splice never shifts the body's frames; a programme,
    /// the one the act drew).
    ///
    /// [`CHANNEL_FRAME_MS`]: script::CHANNEL_FRAME_MS
    ///
    /// The lamp off for the night, whatever she's doing, is apart from
    /// it ([`Osaka::dark`]): the two show together (the fridge open in
    /// the dark).
    pub fn prop(&self, now: u64) -> Option<Prop> {
        let (since, until, play) = match self.act {
            Act::Use {
                since, until, play, ..
            }
            | Act::Idle {
                since,
                until,
                play: Some(play),
                ..
            } => (since, until, play),
            _ => return None,
        };
        let (key, time) = play.key(since, until, now)?;
        key.prop.map(|shows| shows.at(time.elapsed, play.card))
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
                ..
            }
            | Act::Idle {
                since,
                until,
                play: Some(play),
                ..
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

    /// Whether the still act she's at is one she settled into (phase 5c
    /// M7), not one she chose.
    #[cfg(test)]
    pub fn settled_in(&self) -> bool {
        self.settled > 0
    }

    /// The census group what she's doing counts in. Wildcard-free, so a
    /// new act doesn't compile until it's put in one (nothing falls into
    /// "standing" unseen).
    #[cfg(test)]
    pub fn census_group(&self) -> &'static str {
        match self.act {
            // Gazing out of the window is spacing out, as gazing up is.
            Act::Use { seat, .. } if seat.what == Use::LookOut => "spacing out",
            Act::Use { .. } => "furniture",
            // Reading a strip she borrowed, sat on the floor; tearing it
            // off and sliding it back, at the text.
            Act::Borrow {
                phase: Borrowing::Read { .. },
                ..
            } => "floor rest",
            Act::Idle { what, .. } => match what {
                // On the floor, at her homework or a book too.
                Activity::Sit
                | Activity::LieBack
                | Activity::LieFront
                | Activity::SitDoze
                | Activity::FloorHomework
                | Activity::LieRead
                | Activity::BookDoze
                | Activity::UnderSill
                | Activity::CloudWatch => "floor rest",
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
            | Act::Borrow { .. }
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

    /// Her moving at `now`, as the census counts it ([`census_moves`]):
    /// what her body does and what for. `None` while she's still
    /// (whatever she does in place: a pull's bracing and reeling in, a
    /// swap, a sneeze, picking up after it, exercise) *or out of sight*
    /// (around off the screen, a door's hidden beats): a caller counting
    /// still time leaves out [`Osaka::hidden`] first, as the census does.
    /// A pull's heave, the beat she steps back with the line, moves her,
    /// for pulling.
    #[cfg(test)]
    pub fn census_motion(&self, now: u64) -> Option<Motion> {
        if self.hidden(now) {
            return None;
        }
        let (body, purpose) = match census_moves(&self.act) {
            Moves::Off(body) | Moves::On(body) => (body, self.census_purpose()),
            Moves::Heave => (Body::Text, "pull"),
            Moves::Still => return None,
        };
        let want = self.heading.as_ref().map(|h| h.want).or(self.credit);
        Some(Motion {
            purpose,
            body,
            want,
        })
    }

    /// What her moving is for, as the census counts it, first that
    /// holds: to a job she's heading for on another floor, or walking to
    /// on hers, by its kind ("to text": a pull, a swap, tearing text to
    /// make a piece; "to seat": a use; "to home": lifting or setting
    /// down a piece); her shift ("work"); her routine out or home
    /// ("routine"); a dash home ("dash"); an errand ("errand"); else by
    /// her chain: a wander ("wander", "travel"), off the text ("off
    /// text"), her arrival ("arrival"), the floor gone under her
    /// ("accident"), her errand's way out ("errand"), or the method of
    /// the decision itself. The climb, the drop's daze and the walk back
    /// in from off the screen that finish a hop keep the hop's chain (a
    /// trip around the screen's edge reads "travel" to its end, not a
    /// return). A walk to a job that a re-anchor ([`Osaka::settle`])
    /// turned into a plain walk reads as its chain.
    #[cfg(test)]
    pub fn census_purpose(&self) -> &'static str {
        let kind = |job: &Job| match job {
            Job::Pull(_) | Job::Swap(_) | Job::Build(_) | Job::Borrow(_) => "to text",
            Job::Use(_) => "to seat",
            Job::Lift(_) | Job::SetDown(_) => "to home",
            // On her way out by her door for her routine (door batch D6).
            Job::Leave { .. } => "routine",
            // Out of a piece, to go out where it's clear (door batch
            // M25).
            Job::Out { .. } => "routine",
        };
        // On her way out for her shift (door batch, step 4b: "work", not
        // D6's "routine", as work's edge walk was).
        if self.work_way() {
            return "work";
        }
        if let Some(heading) = &self.heading {
            return kind(&heading.job);
        }
        if let Act::Walk {
            then: Then::Job(job),
            ..
        } = &self.act
        {
            return kind(job);
        }
        if self.shift.is_some() {
            "work"
        } else if self.leaving.is_some() || self.returning.is_some() {
            "routine"
        } else if self.dash.is_some() {
            "dash"
        } else if self.errand.is_some() {
            "errand"
        } else {
            match self.chain {
                "walk/along" => "wander",
                "travel/link" | "travel/door" => "travel",
                chain => chain,
            }
        }
    }

    /// What she's playing: using a piece (its own script, or the
    /// shopping channel, or surfing, any prelude or coda), or spacing out
    /// (a riddle, Setsubun's beans).
    pub fn plays(&self) -> Option<Play> {
        match self.act {
            Act::Use { play, .. } => Some(play),
            Act::SpaceOut { play, .. } | Act::Idle { play, .. } => play,
            _ => None,
        }
    }

    /// What she's playing, as [`Osaka::plays`], and since when (its act's
    /// start): two plays alike are told apart by it (the census counts
    /// each once).
    #[cfg(test)]
    pub fn plays_since(&self) -> Option<(u64, Play)> {
        match self.act {
            Act::Use { since, play, .. } => Some((since, play)),
            Act::SpaceOut {
                since,
                play: Some(play),
                ..
            }
            | Act::Idle {
                since,
                play: Some(play),
                ..
            } => Some((since, play)),
            _ => None,
        }
    }

    /// Until when she beams saying her answer to the chat, if she has
    /// answered it in the use she's at (each answer its own time).
    #[cfg(test)]
    pub fn answered_until(&self) -> Option<u64> {
        let (span, until) = self.answering?;
        (self.use_span() == Some(span)).then_some(until)
    }

    /// Whether she's looking up at the chat where she is, from a still
    /// act (phase 5c B1). (On her feet, a watch is a decision of its
    /// own, "watching chat".)
    #[cfg(test)]
    pub fn looking_up_at_chat(&self) -> bool {
        self.looking_up.is_some()
    }

    /// Whether she's stirring at a chat line at `now`, dozing (by day or
    /// for the night): her head up a moment, murmuring.
    #[cfg(test)]
    pub fn stirring_at_chat(&self, now: u64) -> bool {
        now < self.stir_until
    }

    /// Where the key of her act's script playing at `now` is, if her act
    /// plays one (a use, or an idle act or spacing out with a script).
    #[cfg(test)]
    pub fn key_at(&self, now: u64) -> Option<script::KeyPlace> {
        let (play, since, until) = match self.act {
            Act::Use {
                play, since, until, ..
            }
            | Act::Idle {
                play: Some(play),
                since,
                until,
                ..
            }
            | Act::SpaceOut {
                play: Some(play),
                since,
                until,
                ..
            } => (play, since, until),
            _ => return None,
        };
        play.key_place(since, until, now)
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

    /// The text she was pulling, tearing or reading a strip of changed
    /// under her (someone scrolled the chat): she lets go and stares.
    pub fn lost_grip(&mut self, now: u64) {
        if matches!(
            self.act.job(),
            Some(JobRef::Pull(_) | JobRef::Build(_) | JobRef::Borrow(_))
        ) {
            tracing::trace!("houseguest lost her grip");
            self.interrupt(Cause::LostGrip, now);
        }
    }

    /// Choose what to do next, standing somewhere valid.
    fn decide(&mut self, at: u64, terrain: &Terrain, chances: &Chances, rng: &mut Rng) {
        #[cfg(test)]
        {
            self.deciding = Some(false);
        }
        let why = self.choose_next(at, terrain, chances, rng);
        // The census's chain: what set her moving is this decision now,
        // and any set-off it made is for what it's for.
        #[cfg(test)]
        {
            self.deciding = None;
            // A line owed a look that this decision didn't watch (bed or
            // school came first, or the watch ran out) never stopped her.
            self.chat_owed = None;
            self.chain = why.method;
            let purpose = self.census_purpose();
            for set_off in self.set_off_log.iter_mut().filter(|s| s.purpose.is_empty()) {
                set_off.purpose = purpose;
            }
        }
        let decision = Decision {
            at,
            act: self.act_summary(),
            day: self.day(at),
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
        // What she just finished, and what she chose it for: what she
        // might settle further from (read before it's credited).
        let ended = (self.act.clone(), self.credit);
        // What she just finished counts before she chooses anew.
        self.credit_done(at);
        self.got_what_she_came_for(at);
        self.catch_up_day(at);
        // Her set-off line is latched for this school time only: whatever
        // cut her way out short (a chat line, a startle, a focused pane),
        // once school's over, so is the latch (the next school morning
        // says its line again), before anything she decides.
        if self.day(at).map(|day| day.slot) != Some(routine::Slot::Away)
            && self.set_off.take().is_some()
        {
            tracing::debug!("houseguest: school's over; her set-off is let go");
        }
        self.rest = None;
        if self.errand.is_some() {
            self.head_for_errand(terrain, at);
            return Decision::reflex("errand");
        }
        let Some(here) = terrain.platform_at(self.x, self.y) else {
            self.set(Act::Stand { until: at + 1000 }, at);
            return Decision::reflex("no floor");
        };
        // Coming home by her routine, whatever cut her door short: she's
        // home.
        if let Some(why) = self.returning.take() {
            self.home_from(why, at);
            return Decision::reflex("routine/home");
        }
        // Deciding ends a shift under way, but her way to her door for it
        // (door batch, step 4b): landed from a hop on the way (or her walk
        // come to nothing under her), she goes on, to where the frame
        // stands her door now. Anything else cut her way out short (a
        // startle, a chat line, her routine's bed): it came to nothing (no
        // shift, no homecoming). Through her door for it (`Out`) she never
        // decides: nothing cuts a door short but an errand (which cuts
        // her shift: [`Osaka::errand`]), `place` (the same) and a focused
        // pane (another door, `evict`), and the end of whichever door
        // brings her home ([`Osaka::back_home`]). (Work's walk-in from the
        // screen's edge, the one way she came back from it by deciding,
        // is gone: review of step 4b.)
        if self.shift.is_some() {
            let bed = self.day(at).map(|day| day.slot) == Some(routine::Slot::Asleep);
            if self.on_her_way_to_work() && !bed {
                tracing::debug!("houseguest: on her way to work, on she goes");
                self.hopping = false;
                self.on_to_work(terrain, chances, at);
                // The same trip, as a hop's next is (`heading/hop`).
                return Decision {
                    heading: Some(Want::Work),
                    ..Decision::of(Bucket::Continuation, "work/on")
                };
            }
            if self.shift == Some(Shift::Out) {
                tracing::warn!(act = %self.act_summary(), "houseguest: deciding through her door for work");
            } else {
                tracing::debug!("houseguest: her way to work came to nothing");
            }
            self.let_work_go(at);
        }
        // Over text she only passes: on to the nearest calm spot, or by
        // door to one elsewhere.
        if !terrain.restful(self.x, self.y) && self.find_rest(here, terrain, chances, at, rng) {
            return Decision::reflex("off text");
        }
        // With nowhere calm to go, she stays as she is, and isn't startled
        // off it again.
        self.rest = terrain.restful(self.x, self.y).then_some((self.x, self.y));
        // Dashed home from school (D3a): for what she forgot, before her
        // routine sends her out again.
        if let Some(dash) = self.dash
            && let Some(decision) = self.dash_on(dash, here, terrain, chances, at)
        {
            return decision;
        }
        // Her routine (D4, A3): at night, to bed; at school time, out
        // through her door; before anything else she'd do (below, once
        // her context is built). Whatever moving of her home was under
        // way is settled first, so her context sees none, and nothing of
        // it is left frozen overnight or while she's out.
        let slot = self.day(at).map(|day| day.slot);
        let to_bed = slot == Some(routine::Slot::Asleep);
        let to_school = slot == Some(routine::Slot::Away);
        if to_bed || to_school {
            self.settle_for_night(at);
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
            may_work: self.may_work(chances, at),
            owes: self.owes(),
            episode: self.episode,
            just_set: self.just_set,
            may_arrange: !self.to_mend(&chances.broken).is_empty(),
            near: self.stillness.near,
        };
        // With her wall clock where she can see it, her routine's first
        // key is a glance up at it (D7): "Oh! It's late!", "Time for
        // school!". Once a slot: not on her way back to bed from her
        // midnight snack or an errand (her night latched it: see
        // [`Osaka::arm_night`]), nor out again after a dash home (see
        // [`Osaka::dash_on`]).
        if to_bed || to_school {
            let glance = if to_bed {
                ClockGlance::Bed
            } else {
                ClockGlance::School
            };
            if let Some(decision) = self.glance_first(glance, at) {
                return decision;
            }
        }
        // Her routine (D4, A3): at night, to bed, before anything else
        // she'd do, a chat she's watching included (or a chat line every
        // few seconds would keep her up all night).
        if to_bed {
            return self.send_to_bed(&ctx, whims, here, terrain, at, rng);
        }
        if to_school {
            return self.go_out(Routine::School, whims, &ctx, at, rng);
        }
        // Her homework on the floor over, the floor was hard on her back
        // (phase 5c D5): the first time a visit, she says so, standing a
        // moment while it shows (after her routine: bed and school come
        // first, and the next one aches instead).
        if !self.ached
            && matches!(
                ended.0,
                Act::Idle {
                    what: Activity::FloorHomework,
                    ..
                }
            )
        {
            self.ached = true;
            tracing::info!("houseguest: her back aches from homework on the floor");
            self.say(MY_BACK, at);
            let until = self.speech.map_or(at, |(_, until)| until);
            self.set(Act::Stand { until }, at);
            return Decision::reflex("ached");
        }
        // A still act she chose that has run its course: she may settle
        // further where she is (phase 5c M7), after her routine and
        // (below) anything she owes, is moving or made, and before
        // anything new. Watching the chat as it ran out, she settles
        // first, and watches on in her new pose, unless something of
        // those waits: then she stands the watch out, and it's next.
        let settle = self.settling(&ended, terrain, chances, whims, at);
        if at < self.watch_until
            && let Some(settle) = settle
            && !self.owes_before_settling(terrain, chances, at)
        {
            return self.settle_in(settle, chances, at, rng);
        }
        if at < self.watch_until {
            // Standing the rest of it out in one, facing it (`set`
            // turns a stand under a live watch): the watch is never more
            // than `WATCH_MS` ahead (each line sets it from its own time),
            // so there's no long watch to break up.
            self.set(
                Act::Stand {
                    until: self.watch_until,
                },
                at,
            );
            // A line that came while she was out of sight or aloft stops
            // her now, for the census (what it cut is what she was on).
            #[cfg(test)]
            if let Some(cut) = self.chat_owed.take() {
                self.chat_cuts.push((at, false, cut));
                self.after_chat = Some((false, cut));
            }
            return Decision::reflex("watching chat");
        }
        // Hello: what her calendar owes her to say, if it does, in her
        // mood's greeting's stead.
        if !self.greeted {
            self.greeted = true;
            match self.calendar_greeting(at) {
                Some(greeting) => self.say_calendar(greeting, at),
                None => self.say(self.mood.greeting(), at),
            }
        }
        // Her needs move on with the time since they last did (as she
        // last chose, or was last eased).
        self.rising = self.rising_in(chances);
        self.rise_to(at);
        let heading = self.heading.as_ref().map(|h| h.want);
        let hopped = std::mem::take(&mut self.hopping);
        // What her calendar owes her (D5): the first beat she owes.
        if let Some(decision) = self.calendar_beat(here, terrain, chances, at) {
            return Decision {
                heading,
                ..decision
            };
        }
        // Up on a day off: she says so once she's quiet, that morning,
        // standing a moment (what she'd do next might speak over it).
        if self.day_off {
            let morning = self
                .day(at)
                .is_some_and(|day| day.slot == routine::Slot::Morning);
            let quiet = self.speech.is_none_or(|(_, until)| until <= at);
            if !morning || quiet {
                self.day_off = false;
            }
            if morning && quiet {
                self.say_pool(mind::NO_SCHOOL, at);
                let until = self.speech.map_or(at, |(_, until)| until);
                self.set(Act::Stand { until }, at);
                return Decision {
                    heading,
                    ..Decision::reflex("routine/day off")
                };
            }
        }
        // What's wrong with her home that she felt on sight (door batch,
        // D7): said once she's quiet (after her "I'm home!"), standing
        // for it, while she has felt it broken still and not let it go.
        if let Some(decision) = self.say_aloud(chances, at) {
            return Decision {
                heading,
                ..decision
            };
        }
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
            ctx.may_work = self.may_work(chances, at);
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
        if let Some(settle) = settle {
            return Decision {
                heading,
                ..self.settle_in(settle, chances, at, rng)
            };
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
        #[cfg(test)]
        if let Some((only, by)) = self.offer_only {
            offers.retain(|&(want, name, _)| want == only && (name == by || name == "heading"));
        }
        let (day, date) = (self.day(at), self.clock.and_then(|clock| clock.date));
        let ached = self.ached;
        #[cfg(test)]
        let factored = std::cell::RefCell::new(Vec::new());
        for attempt in 0.. {
            let wants: Vec<(Want, Spot)> = offers
                .iter()
                .map(|(want, _, bind)| (*want, bind.on()))
                .collect();
            // A resident mostly keeps out of the chat, where people read;
            // her day and the season make some things likelier.
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
                let made = offers
                    .iter()
                    .find(|(w, ..)| *w == want)
                    .map_or(1.0, |(_, _, bind)| making(bind, chances, ached));
                let times = brain::factor(want, into_chat, day.as_ref(), date) * made;
                #[cfg(test)]
                factored.borrow_mut().push((want, into_chat, times));
                times * inertia
            };
            let chosen = brain::choose(&wants, &self.needs, &self.recent, &factor, whims, attempt);
            #[cfg(test)]
            self.factored.append(&mut factored.borrow_mut());
            let Some((i, top)) = chosen else {
                break;
            };
            let (want, method, bind) = offers.remove(i);
            if inert.is_some_and(|w| w != want) {
                inert = None;
                self.drop_heading(Letting::Other);
            }
            if self.plan(want, bind, here, terrain, chances, at, rng) {
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
                    self.serve(want, 1.0, Spot::Any, Via::SetOff, at);
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
            #[cfg(test)]
            self.stranded.push(want);
        }
        self.set(Act::Stand { until: at + 2000 }, at);
        Decision {
            heading,
            ..Decision::of(Bucket::Normal, "nothing bound")
        }
    }

    /// Set about what a method bound, from platform `here`. False when
    /// there's no way there after all. (`chances` for work's door: the
    /// frame's, as her mind bound it.)
    #[allow(clippy::too_many_arguments)]
    fn plan(
        &mut self,
        want: Want,
        bind: Bind,
        here: usize,
        terrain: &Terrain,
        chances: &Chances,
        at: u64,
        rng: &mut Rng,
    ) -> bool {
        let act = match bind {
            Bind::Here(Here::Stand) => Act::Stand {
                until: at + rng.range(3000, 8000),
            },
            Bind::Here(Here::SpaceOut) => Act::SpaceOut {
                since: at,
                until: at + self.lingered(true, rng.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1)),
                play: None,
                session: None,
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
            Bind::Here(Here::FloorHomework { book }) => self.floor_homework_act(book, at, rng),
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
            Bind::Work => {
                self.go_to_work(terrain, chances, at, rng);
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

    /// What she'd settle into where she is (phase 5c M7, D4), `ended`
    /// (her act, and the want she was at it for) having run its course
    /// at `at`, on calm floor: spacing out (musing, a riddle, her rare
    /// musing) or gazing, to sitting; sitting, to lying back, or (on a
    /// whim, [`Stillness::sit_doze`]) dozing off where she sits; lounging,
    /// to a nap on the same sofa, from where she sits (as napping on it
    /// would be offered her now: [`mind::places`]); reading on her back,
    /// to a doze under the book. As often as her
    /// mood's odds ([`Stillness::settle`]) on her decision's `whims`,
    /// salted with how far she has settled already. Nothing else
    /// settles: a lying doze is as far as it goes; nor what she didn't
    /// choose (a glance at her clock, Setsubun, the moment after a swap,
    /// a trial sit) or was cut short of, nor anything over text.
    fn settling(
        &self,
        ended: &(Act, Option<Want>),
        terrain: &Terrain,
        chances: &Chances,
        whims: Whims,
        at: u64,
    ) -> Option<Settle> {
        let depth = u64::from(self.settled);
        let odds = self.stillness.settle.of(self.mood);
        if !terrain.restful(self.x, self.y) || !whims.odds("settle in", depth, odds) {
            return None;
        }
        let trying = self.episode.is_some_and(|e| e.trying);
        match ended {
            (
                Act::SpaceOut {
                    until, play: None, ..
                },
                Some(Want::SpaceOut),
            ) if *until <= at => Some(Settle::Idle(Activity::Sit)),
            (
                Act::SpaceOut {
                    until,
                    play: Some(play),
                    ..
                },
                Some(Want::SpaceOut),
            ) if *until <= at && matches!(play.own, ScriptId::Riddle | ScriptId::Escalator) => {
                Some(Settle::Idle(Activity::Sit))
            }
            (
                Act::Idle {
                    what,
                    until,
                    play: None,
                    ..
                },
                Some(Want::Idle(chose)),
            ) if what == chose && *until <= at => match what {
                Activity::Gaze => Some(Settle::Idle(Activity::Sit)),
                Activity::Sit => {
                    let doze = whims.odds("settle doze", depth, self.stillness.sit_doze);
                    Some(Settle::Idle(if doze {
                        Activity::SitDoze
                    } else {
                        Activity::LieBack
                    }))
                }
                // Reading on her back, she dozes off under the book.
                Activity::LieRead => Some(Settle::Idle(Activity::BookDoze)),
                // In front of her window: dozing off where she sits (on
                // the doze's whim), else lying back under it to watch
                // the clouds, while it's still there (only ever under a
                // window: her spot its look-out seat).
                Activity::UnderSill => {
                    let doze = whims.odds("settle doze", depth, self.stillness.sit_doze);
                    let under = chances
                        .seats
                        .iter()
                        .any(|s| s.what == Use::LookOut && (s.x, s.y) == (self.x, self.y));
                    Some(Settle::Idle(if doze || !under {
                        Activity::SitDoze
                    } else {
                        Activity::CloudWatch
                    }))
                }
                // Her homework on the floor nods off in its own script
                // (as at her desk), never settled from (it plays: no
                // arm here sees it).
                Activity::LieBack
                | Activity::LieFront
                | Activity::Jacks
                | Activity::ToeTouch
                | Activity::Stretch
                | Activity::SitDoze
                | Activity::FloorHomework
                | Activity::BookDoze
                | Activity::CloudWatch => None,
            },
            // Leaning on her window's sill, she sits down in front of it.
            (Act::Use { seat, until, .. }, Some(Want::Use(Use::LookOut)))
                if seat.what == Use::LookOut && *until <= at && !trying =>
            {
                Some(Settle::Idle(Activity::UnderSill))
            }
            (Act::Use { seat, until, .. }, Some(Want::Use(Use::Lounge)))
                if seat.what == Use::Lounge && *until <= at && !trying =>
            {
                let nap = chances.seats.iter().find(|s| {
                    s.what == Use::Nap && s.piece == seat.piece && (s.x, s.y) == (seat.x, seat.y)
                })?;
                mind::places(Use::Nap, chances, whims)
                    .contains(&mind::Place::Seat(*nap))
                    .then_some(Settle::Use(*nap))
            }
            _ => None,
        }
    }

    /// Whether, at `at`, something comes before anything she'd settle
    /// into ([`Osaka::choose_next`]'s order after her watch): what her
    /// calendar has for her, her day off said, a beat she owes, the next
    /// hop of her way somewhere (or to a piece she made), the piece
    /// she's moving or has just set down, or a piece she made and hasn't
    /// finished with that she can get to. Read only as her watch decides
    /// whether she settles first, so none of them waits a whole settled
    /// act behind it; each is the same test as where it's done (a
    /// moving piece let go of, or a way that's gone, she stands the
    /// watch out for nothing: no more).
    fn owes_before_settling(&self, terrain: &Terrain, chances: &Chances, at: u64) -> bool {
        let day_off = self.day_off
            && self
                .day(at)
                .is_some_and(|day| day.slot == routine::Slot::Morning)
            && self.speech.is_none_or(|(_, until)| until <= at);
        let leftover = chances.mine.iter().any(|m| {
            !(m.done && m.used) && self.tries_at(m.id) < TRIES && chances.next_for(m).is_some()
        });
        self.calendar_due(chances, terrain, at).is_some()
            || day_off
            || self.aloud_owed(chances).is_some()
            || !self.owed.is_empty()
            || self
                .heading
                .as_ref()
                .is_some_and(|h| self.hopping || h.mine())
            || self.episode.is_some()
            || self.just_set.is_some()
            || leftover
    }

    /// Settle into `settle` at `at`, where she is and facing as she is
    /// (see [`Osaka::settling`]): a continuation of what she chose, not
    /// a new choice, so it skips the roll and isn't one of her recent
    /// choices; it eases her as its own want. Watching the chat, she
    /// watches on in her new pose (unless she dozes off: a doze wears no
    /// look). On a piece she faces as its seat does, which is how she
    /// faced on it but for a look at the chat; and the one use she
    /// settles into, a nap, is a doze, whose look is over, turning her
    /// back to the seat as any look's end does.
    fn settle_in(&mut self, settle: Settle, chances: &Chances, at: u64, rng: &mut Rng) -> Decision {
        let (look, facing, depth) = (self.looking_up, self.facing, self.settled + 1);
        let want = match settle {
            Settle::Idle(what) => {
                let act = self.idle_from(what, at, rng);
                self.set(act, at);
                self.facing = facing;
                if what == Activity::CloudWatch {
                    // Facing her window as its look-out seat has her,
                    // whichever way she last turned to the chat.
                    let toward = chances
                        .seats
                        .iter()
                        .find(|s| s.what == Use::LookOut && (s.x, s.y) == (self.x, self.y))
                        .map_or(facing, |s| s.facing);
                    self.watch_the_clouds(toward, depth, at);
                }
                Want::Idle(what)
            }
            Settle::Use(seat) => {
                self.start_job(Job::Use(seat), at, chances, rng);
                Want::Use(seat.what)
            }
        };
        self.credit = Some(want);
        self.settled = depth;
        if at < self.watch_until && !self.acting(at).0.dozes() {
            self.looking_up = Some(look.unwrap_or(LookUp {
                since: at.saturating_sub(LOOK_MS),
                hidden: at,
                until: self.watch_until,
                step: 2,
                back: None,
            }));
        }
        tracing::info!(act = %self.act_name(), depth, "houseguest: settles in where she is");
        // Not a roll: no want chosen (a census counts those as choices).
        Decision::of(Bucket::Continuation, SETTLE_IN)
    }

    /// Lying back under her window at `at` to watch the clouds (phase 5c
    /// D6), settled in `depth` deep: turned over end to end from facing
    /// it (`toward`, as its look-out seat has her), so her head's under
    /// the glass, and saying her one line of it, the sky's as it is
    /// (none, if every one of its lines is cooling).
    fn watch_the_clouds(&mut self, toward: Facing, depth: u8, at: u64) {
        self.facing = match toward {
            Facing::Right => Facing::Left,
            Facing::Left => Facing::Right,
        };
        let sky = self.sky_at(at);
        let whims = self.whims.series("clouds", u64::from(depth));
        if let Some(line) = self.lines.pick(mind::sky_musings(sky), whims, at) {
            self.say(line, at);
        }
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
                self.door_to(want, job, at);
                true
            }
            None => false,
        }
    }

    /// Heading for `job` (for `want`) by a door in space straight to its
    /// spot: no way there from where she stands.
    fn door_to(&mut self, want: Want, job: Job, at: u64) {
        let to = job.spot();
        self.hopping = true;
        #[cfg(test)]
        if self.heading.is_none() {
            self.headings.push("set off".to_owned());
        }
        self.heading = Some(Heading { want, job });
        self.through_door(to, at);
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
            // Her way out by her door isn't something she lost: her
            // routine sends her on, or school's over (door batch D6).
            let leave = heading.job.leave().is_some();
            if matches!(why, Letting::Gone | Letting::Other) && !leave {
                self.owe(Loss::Heading, heading.job.spot());
            }
        }
    }

    /// The rule she'd say what's wrong with, felt on sight (see
    /// [`Osaka::say_aloud`]): the first she owes that she has felt broken
    /// still and not let go, while her mood would have her put it right
    /// (her home acts not used up, as [`Osaka::to_mend`] asks). The one
    /// place that rule is held: she comes to owe a line whatever her mood
    /// (`on_sight`, mod.rs), so a rule felt mid-episode is said once that
    /// episode is done, if she'd still mend.
    fn aloud_owed(&self, chances: &Chances) -> Option<usize> {
        if self.home_acts >= self.mood.home_acts() {
            return None;
        }
        self.owed_aloud.iter().copied().find(|&row| {
            chances
                .broken
                .iter()
                .any(|b| b.row == row && self.felt.iter().any(|f| f.key == b.key && !f.let_go))
        })
    }

    /// Say what's wrong with her home that she felt on sight (door batch,
    /// D7), if she owes it ([`Osaka::aloud_owed`]): once she's quiet
    /// (nothing she's saying, nor her "I'm home!" or good morning),
    /// standing a moment, for [`GRIEVANCE_MS`], what she says under
    /// [`Osaka::grumbling`] so a look at the chat waits for it; not yet
    /// quiet, she stands until she is (what she'd do next might take her
    /// off for long). Once a visit; owed but no longer broken (or let
    /// go), it goes unsaid.
    fn say_aloud(&mut self, chances: &Chances, at: u64) -> Option<Decision> {
        let row = self.aloud_owed(chances)?;
        let quiet = self
            .speech
            .map_or(at, |(_, until)| until)
            .max(self.morning_until);
        if quiet > at {
            self.set(Act::Stand { until: quiet }, at);
            return Some(Decision::reflex("home/on sight, once quiet"));
        }
        self.owed_aloud.retain(|&r| r != row);
        let line = super::rules::RULES.get(row)?.grievance;
        tracing::info!(line, "houseguest: says what's wrong with her home");
        self.hush(at);
        self.set(
            Act::Stand {
                until: at + GRIEVANCE_MS,
            },
            at,
        );
        self.aloud = Some((line, at));
        // Nothing speaks over it (a parcel waits, as for her hello).
        self.morning_until = self.morning_until.max(at + GRIEVANCE_MS);
        #[cfg(test)]
        self.aloud_said.push((line, at));
        let day = self.day(at).map(|d| d.day);
        self.events.push(HomeEvent::Grieved { row, day });
        Some(Decision::reflex("home/on sight"))
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

    /// She has felt `key` broken: using `on` (a piece, for a use: the
    /// stage's cue), or on sight with none (door batch, D7: a piece in
    /// her door's space she bumped into coming home, or one in the chat
    /// pane). Whether it's newly felt this visit.
    pub fn feel(&mut self, key: Grievance, on: Option<(Furniture, Use)>) -> bool {
        if self.has_felt(key) {
            return false;
        }
        let how = if on.is_some() { "cued" } else { "on sight" };
        tracing::info!(rule = %key.label(), how, "houseguest: she felt {}", key.label());
        self.felt.push(Felt {
            key,
            on,
            let_go: false,
        });
        true
    }

    /// She'll say what's wrong with her home that she felt on sight, for
    /// the rule on `row` of the table, once she's quiet (door batch, D7:
    /// see [`Osaka::aloud`]): once a visit.
    pub fn owe_aloud(&mut self, row: usize) {
        if !self.owed_aloud.contains(&row) {
            tracing::trace!(row, "houseguest: she'll say what's wrong once she's quiet");
            self.owed_aloud.push(row);
        }
    }

    /// The line she says what's wrong with her home with, felt on sight,
    /// if she's saying one at `now` (standing, for [`GRIEVANCE_MS`]).
    fn aloud_at(&self, now: u64) -> Option<&'static str> {
        let (line, from) = self.aloud?;
        (matches!(self.act, Act::Stand { .. }) && (from..from + GRIEVANCE_MS).contains(&now))
            .then_some(line)
    }

    /// What she has said aloud of what's wrong with her home, felt on
    /// sight, and when (tests).
    #[cfg(test)]
    pub fn said_aloud(&self) -> &[(&'static str, u64)] {
        &self.aloud_said
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
            self.serve(Want::Arrange, 1.0, Spot::Any, Via::SetDown, now);
        }
        ep.tried = ep.tried.saturating_add(1);
        self.just_set = self
            .felt
            .iter()
            .find(|f| f.key == ep.repair.key)
            .and_then(|f| f.on);
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
            && self.plan(Want::Arrange, bind, here, terrain, ctx.chances, at, rng)
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
        self.grievance_span()
            .is_some_and(|(from, to)| (from..to).contains(&now))
    }

    /// When her act has her say what's wrong with her home, if it does:
    /// using a piece, or standing, saying what she felt on sight.
    fn grievance_span(&self) -> Option<(u64, u64)> {
        match self.act {
            Act::Use {
                grievance: Some((_, from)),
                ..
            } => Some((from, from + GRIEVANCE_MS)),
            Act::Stand { .. } => self.aloud.map(|(_, from)| (from, from + GRIEVANCE_MS)),
            _ => None,
        }
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

    /// Her needs as she arrives at `day`'s time of her routine (D4 lever
    /// 3; set as a visit begins, beside her mood): the afternoon's are
    /// as ever, she comes sleepier later in the day, hungrier at
    /// mealtimes.
    pub fn set_clock(&mut self, day: DayTime) {
        tracing::trace!(%day, "houseguest: arriving at this time of her day");
        self.needs = Needs::arriving_in(day.slot);
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
        if what == Activity::FloorHomework {
            return self.floor_homework_act(false, at, rng);
        }
        if matches!(
            what,
            Activity::Sit | Activity::LieBack | Activity::LieFront | Activity::LieRead
        ) {
            self.face_either_way(rng);
        }
        self.idle_from(what, at, rng)
    }

    /// Sitting and lying, she faces either way.
    fn face_either_way(&mut self, rng: &mut Rng) {
        self.facing = if rng.below(2) == 0 {
            Facing::Left
        } else {
            Facing::Right
        };
    }

    /// Her homework on the floor from `at` (phase 5c D5): on her front
    /// writing on the paper out in front of her, or (`book`) on her back
    /// reading the set text over her face; nodding off where her mood
    /// has her, as at her desk (see [`script::floor_homework_branch`]),
    /// and as long as at her desk (at homework time, as long as then).
    /// Facing either way, as lying does.
    fn floor_homework_act(&mut self, book: bool, at: u64, rng: &mut Rng) -> Act {
        self.face_either_way(rng);
        let slot = self.day(at).map(|day| day.slot);
        let (lo, hi) = Activity::FloorHomework.duration_in(slot);
        let nod = self.stillness.nod_off.of(self.mood);
        tracing::debug!(book, "houseguest: homework on the floor");
        Act::Idle {
            what: Activity::FloorHomework,
            since: at,
            until: at + rng.range(lo, hi),
            play: Some(Play {
                branch: script::floor_homework_branch(nod, book),
                ..Play::plain(ScriptId::FloorHomework)
            }),
        }
    }

    /// The stage: her homework on the floor, now (on her back with the
    /// set text, if `book`).
    pub fn floor_homework(&mut self, book: bool, at: u64, rng: &mut Rng) {
        let act = self.floor_homework_act(book, at, rng);
        self.set(act, at);
    }

    /// Doing `what` on the spot from `at`, as long as she draws for it
    /// (lingered by her mood, if it lingers), facing as she is.
    fn idle_from(&self, what: Activity, at: u64, rng: &mut Rng) -> Act {
        let (lo, hi) = what.duration();
        Act::Idle {
            what,
            since: at,
            until: at + self.lingered(what.lingers(), rng.range(lo, hi)),
            play: None,
        }
    }

    /// `ms`, drawn for a still act she chose, as long as her mood
    /// lingers over it if it `lingers` (phase 5c M8: see
    /// [`Stillness::linger`]).
    fn lingered(&self, lingers: bool, ms: u64) -> u64 {
        if lingers {
            self.stillness.lingered(self.mood, ms)
        } else {
            ms
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
        self.cut_shift(at);
        self.kept_in = false;
        self.leaving = None;
        self.set_off = None;
        self.returning = None;
        self.late = false;
        self.dash = None;
        self.set(Act::Stand { until: at + 1000 }, at);
    }

    /// A chat message arrived: stop and look at it — unless where she is
    /// her image hides text (`terrain`, as last read), where she only
    /// passes: there the chat still interrupts what she was at, but she
    /// doesn't stop; she chooses at once, which takes her on to somewhere
    /// calm, and watches it from there if she's there within its watch.
    /// (Stopping for each line of a lively chat would keep her over text
    /// for as long as it went on.) A line that `asks` her something,
    /// arriving as a splice that answers plays (the andagi), gets its
    /// answer instead: she turns to the chat and says it, beaming, and
    /// plays on, the splice neither stopped nor cut short (and she watches
    /// the chat after it if its watch is still running then). The watch
    /// runs from `now` whatever puts it off (`WATCH_MS`).
    pub fn look(&mut self, now: u64, chat_x: i32, asks: bool, terrain: &Terrain) {
        // What the line cuts, for the census: read before anything here
        // lets go of where she was heading.
        #[cfg(test)]
        let cut = self.census_cut();
        // A stir isn't a look: it sets no watch (she never turned to it).
        let watched = self.watch_until;
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
        // Out, or on her way: she'll see it when she's back, if she's
        // back within its watch.
        if self.act.props().on_chat == OnChat::Back {
            #[cfg(test)]
            {
                self.chat_owed = Some(cut);
            }
            return;
        }
        self.watch_x = chat_x;
        if self.aloft() {
            #[cfg(test)]
            {
                self.chat_owed = Some(cut);
            }
            // She watches once she has landed (decide watches), if she
            // lands within the watch.
            return;
        }
        // Asleep for the night, a line only stirs her, whatever it asks
        // (A13): she doesn't even turn to it.
        if self
            .night_play()
            .is_some_and(|(play, ..)| play.own.on_chat() == Chat::Stir)
        {
            self.watch_until = watched;
            return self.stir(now);
        }
        if asks && let Some(answer) = self.answer(now) {
            tracing::info!(answer, "houseguest: answers the chat, playing on");
            self.facing = toward(self.x, chat_x);
            self.say(answer, now);
            self.answering = self.use_span().map(|span| (span, now + speech_ms(answer)));
            return;
        }
        let passing = !terrain.restful(self.x, self.y);
        // In a still act on calm floor, she looks up where she is, and
        // it runs on; dozing, she only stirs (no watch, and not a look
        // for the census: as at night).
        if !passing && self.act.props().on_chat == OnChat::LooksUp {
            if self.acting(now).0.dozes() {
                self.watch_until = watched;
                return self.stir_dozing(now);
            }
            self.look_up(now, chat_x);
            // A look that cut nothing, for the census.
            #[cfg(test)]
            {
                self.chat_owed = None;
                self.chat_cuts.push((now, false, cut));
            }
            return;
        }
        self.facing = toward(self.x, chat_x);
        if passing {
            tracing::trace!(
                x = self.x,
                y = self.y,
                "houseguest: chat over text; on somewhere calm"
            );
            self.interrupt(Cause::ChatPassing, now);
        } else {
            self.interrupt(Cause::Chat, now);
        }
        #[cfg(test)]
        {
            self.chat_owed = None;
            self.chat_cuts.push((now, passing, cut));
            self.after_chat = Some((passing, cut));
        }
    }

    /// Dozing by day in a still act (lying back, napping, asleep, nodding
    /// off over her homework: whatever pose [`Pose::dozes`]), a chat
    /// line only stirs her, as at night: turned over a moment, blinking,
    /// "Mm?" (phase 5c B1). Any look she was under is over (her tick
    /// ends one as she nods off; this is the same end if it hasn't run).
    fn stir_dozing(&mut self, now: u64) {
        tracing::info!("houseguest: stirs at the chat, dozing");
        self.end_look(now);
        self.stir_saying(STIRRED, now);
    }

    /// A chat line at `now` (the chat's middle at column `chat_x`) in a
    /// still act on calm floor ([`OnChat::LooksUp`]), not dozing: she
    /// looks up where she is, and her act runs on, its clock and what it
    /// eases her by as they were (phase 5c B1). In her act's own pose she
    /// turns to the chat where the pose turns ([`Pose::turns`]: not lying,
    /// not at her desk), startled (`!`), then puzzled (`?`), then watches
    /// plain-faced until her watch is over, her act's own bubble hidden
    /// meanwhile (what she's saying shows on over it, as over any look);
    /// then she's at it as before, turned back to a piece she sits to.
    /// What her act's script said under the look (a riddle's question
    /// and answer) she says once it's over, in turn. A line coming as she
    /// says what's wrong with her home, the look waits for her to have
    /// said it. Each line of a lively chat looks again. If her act ends
    /// while she watches, she stands to watch, as after any look.
    fn look_up(&mut self, now: u64, chat_x: i32) {
        let pose = self.acting(now).0;
        tracing::info!(act = %self.act_name(), "houseguest: looks up at the chat where she is");
        let was = self.looking_up;
        // Turned back once it's over only to a piece's seat (and to the
        // seat's facing, however many lines turned her since).
        let back = match (was, &self.act) {
            (Some(was), _) => was.back,
            (None, Act::Use { seat, .. }) => Some(seat.facing),
            (None, _) => None,
        };
        // What she says about her home she says to its end first.
        let since = match self.grievance_span() {
            Some((from, to)) if from < now + LOOK_MS && now < to => to,
            _ => now,
        };
        let hidden = match was {
            Some(was) if was.step < 2 => was.hidden,
            _ => since,
        };
        if pose.turns() {
            self.facing = toward(self.x, chat_x);
        }
        self.hold_bob(now);
        self.looking_up = Some(LookUp {
            since,
            hidden,
            // Watching plain-faced a frame at least once her `?` is over
            // (never coming and going inside one: phase 5c's tail, T4).
            until: self.watch_until.max(since + LOOK_UP_MS),
            step: 0,
            back,
        });
    }

    /// Her look up from her act is over: turned back to the piece she
    /// sits to, if she turned from one. Her bob isn't held here: each
    /// caller holds or starts afresh at the same moment. Her watch over
    /// ([`Osaka::looking_up_moves`]) holds as it moves; a stir dozing
    /// ([`Osaka::stir_dozing`]) says its murmur, which holds; nodding off
    /// under her look (her tick) is her act's next key, whose bob starts
    /// on its own grid; and a new act ([`Osaka::set`]) clears the hold.
    /// A line still showing that would end less than a frame after it
    /// shows on until a frame after it (a change of hers waits a frame
    /// from a look's, as [`Osaka::free_to_muse`] has a musing wait).
    fn end_look(&mut self, at: u64) {
        if let Some(look) = self.looking_up.take() {
            self.look_ended = Some(at);
            if let Some(back) = look.back {
                self.facing = back;
            }
            if let Some((text, until)) = self.speech
                && until > at
                && until < at + USE_FRAME_MS
            {
                tracing::trace!(
                    until,
                    at,
                    "houseguest: her line shows a frame past her look"
                );
                self.speech = Some((text, at + USE_FRAME_MS));
            }
        }
    }

    /// Her look up from her act reached its next moment, `at`: out of
    /// her startle; the look over, what her act's script said under it
    /// said now; the watch over, back to her act (turned back to the
    /// piece she sits to).
    fn looking_up_moves(&mut self, at: u64) {
        let Some(look) = &mut self.looking_up else {
            return;
        };
        look.step += 1;
        let (step, from, to) = (look.step, look.hidden, look.since + LOOK_MS);
        // Her face and bubble change in place: her bob holds a frame.
        self.hold_bob(at);
        match step {
            1 => {}
            2 => self.say_what_her_look_hid(from, to, at),
            _ => {
                self.end_look(at);
                tracing::trace!("houseguest: back to what she was at after the chat");
            }
        }
    }

    /// Say at `at` what her act's script said from `from` to `to`, under
    /// her look: each line in turn, the first now and each next as the
    /// one before is over (a riddle's question, then its answer: never a
    /// punchline without its setup).
    fn say_what_her_look_hid(&mut self, from: u64, to: u64, at: u64) {
        let lines = self.script_lines_in(from, to);
        if let Some((&first, rest)) = lines.split_first() {
            tracing::debug!(?lines, "houseguest: says what her act said under her look");
            self.say(first, at);
            self.unsaid = rest.to_vec();
        }
    }

    /// The lines her act's script says from `from` to `to` (a key's line
    /// showing then, by its keys' own timing), in order, each once.
    fn script_lines_in(&self, from: u64, to: u64) -> Vec<&'static str> {
        let (since, until, play) = match self.act {
            Act::Use {
                since, until, play, ..
            } => (since, until, play),
            Act::SpaceOut {
                since,
                until,
                play: Some(play),
                ..
            }
            | Act::Idle {
                since,
                until,
                play: Some(play),
                ..
            } => (since, until, play),
            _ => return Vec::new(),
        };
        let mut lines: Vec<&'static str> = Vec::new();
        let mut at = from;
        while at < to {
            if let Some((key, time)) = play.key(since, until, at)
                && let (_, _, Some(Bubble::Say(text))) = key.look(time, Pose::Stand, &play)
                && lines.last() != Some(&text)
            {
                lines.push(text);
            }
            match play.next_end(since, until, at) {
                Some(end) if end > at => at = end,
                _ => break,
            }
        }
        lines
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
            Chat::Look | Chat::Stop | Chat::Stir => None,
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
            Cause::ChatPassing | Cause::Routine => (0, 0),
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
        #[cfg(test)]
        {
            self.chain = "accident";
        }
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
            Act::Door { since, to, gap } => {
                door_beat(now.saturating_sub(since), gap, to).is_some_and(|(beat, _)| !beat.her)
            }
            _ => false,
        }
    }

    /// Her own door she's going through (`Through::Home`), as it stands
    /// at `now` in its beats (door batch D8): its spot, and side-on in
    /// its wall the beat and what it draws ([`wall_beat`]), or face-on
    /// (a fallback) its frame, standing closed through the gap (no cue
    /// there). `None` through a door in space, or none.
    pub fn front_door(&self, now: u64) -> Option<(DoorSpot, FrontBeat)> {
        let Act::Door {
            since,
            to: to @ Through::Home(door),
            gap,
        } = self.act
        else {
            return None;
        };
        let (index, beat, start, _) = door_at(now.saturating_sub(since), gap, to)?;
        Some(match door.set() {
            Set::Wall { .. } => {
                let into = now.saturating_sub(since).saturating_sub(start);
                let shift = self.shift == Some(Shift::Out);
                (door, FrontBeat::Wall(index, wall_beat(index, into, shift)))
            }
            Set::Floor(_) => (
                door,
                FrontBeat::Floor(beat.door.unwrap_or(DoorFrame::Closed)),
            ),
        })
    }

    /// Her front door side-on in its wall, as her beat through it draws
    /// it at `now` ([`Osaka::front_door`]): its spot, the beat, and what's
    /// drawn.
    #[cfg(test)]
    pub fn wall_beat(&self, now: u64) -> Option<(DoorSpot, usize, WallBeat)> {
        match self.front_door(now)? {
            (door, FrontBeat::Wall(index, beat)) => Some((door, index, beat)),
            (_, FrontBeat::Floor(_)) => None,
        }
    }

    /// The door in space she's going through, if any, as it looks at
    /// `now` (her own door is [`Osaka::front_door`]'s to draw).
    pub fn door_in_space(&self, now: u64) -> Option<DoorFrame> {
        match self.act {
            Act::Door {
                to: Through::Space(_),
                ..
            } => self.door(now),
            _ => None,
        }
    }

    /// The door she's going through, if any, as it looks at `now`.
    pub fn door(&self, now: u64) -> Option<DoorFrame> {
        match self.act {
            Act::Door { since, to, gap } => door_beat(now.saturating_sub(since), gap, to)?.0.door,
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
                to: Through::Space(to),
                gap: 0,
            },
            at,
        );
    }

    /// Out at work through a door in space where she stands, in its gap
    /// (out of sight, her shift `gap` long), whatever stands there: as a
    /// resize while she's out can leave it over a piece of hers (a way
    /// out is judged again only as its far door shows; door batch, step
    /// 10a review). For the guest's tests of what's drawn of her box
    /// while she's out.
    #[cfg(test)]
    pub(super) fn out_at_work_in_gap(&mut self, gap: u64, now: u64) {
        let to = Through::Space((self.x, self.y));
        self.shift = Some(Shift::Out);
        self.worked = true;
        self.set(
            Act::Door {
                since: now.saturating_sub(through_ms(to) + 100),
                to,
                gap,
            },
            now,
        );
    }

    /// She arrives for an errand: out of a door onto `spot` (the far
    /// door's beats only), to poke the scrollback accordion under it.
    pub fn arrive_for_errand(spot: (i32, i32), now: u64, rng: &mut Rng) -> Self {
        let act = Act::Door {
            since: now.saturating_sub(through_ms(Through::Space(spot))),
            to: Through::Space(spot),
            gap: 0,
        };
        let mut osaka = Self::new(spot.0, spot.1, Facing::Right, act, now, rng);
        osaka.errand = Some(spot);
        #[cfg(test)]
        {
            osaka.chain = "errand";
            osaka.census_arrived_for();
        }
        osaka
    }

    /// She comes home by her routine (`why`): out of her door `to` (her
    /// own at its spot, or with none anywhere a door in space), facing
    /// into the room (the far door's beats only, from its first, so a
    /// door standing closed there goes straight on into hers). At its
    /// end she says she's home ([`Osaka::home_from`]). The day is the
    /// unit: she said good morning already, so no hello.
    pub fn back_through_door(to: Through, why: Routine, now: u64, rng: &mut Rng) -> Self {
        let act = Act::Door {
            since: now.saturating_sub(there_ms(to)),
            to,
            gap: 0,
        };
        let (x, y) = to.spot();
        let mut osaka = Self::new(x, y, to.into_room(), act, now, rng);
        osaka.returning = Some(why);
        osaka.greeted = true;
        #[cfg(test)]
        osaka.census_arrived_for();
        osaka
    }

    /// She dashes home from school for something she forgot (phase 5b
    /// D3a): out of her door `to`, facing into the room (the far door's
    /// beats only, from its first, so a door standing closed there goes
    /// straight on into hers). As she first decides, she goes for it (see
    /// [`Osaka::dash_on`]); then her routine sends her out again. No
    /// hello once she has `met` you (she said good morning already); a
    /// first meeting (the stage's) has hers after.
    pub fn dash_in(to: Through, met: bool, now: u64, rng: &mut Rng) -> Self {
        let act = Act::Door {
            since: now.saturating_sub(there_ms(to)),
            to,
            gap: 0,
        };
        let (x, y) = to.spot();
        let mut osaka = Self::new(x, y, to.into_room(), act, now, rng);
        osaka.dash = Some(Dash::In);
        osaka.greeted = met;
        #[cfg(test)]
        osaka.census_arrived_for();
        osaka
    }

    /// Whether she's on a dash home from school, what she forgot not yet
    /// got (see [`Osaka::dash_in`]).
    pub fn dashing(&self) -> bool {
        self.dash.is_some()
    }

    /// The stage: her dash home comes out of her `door` instead, facing
    /// into the room, from its first far beat at `now`, what she forgot
    /// still to settle. Whatever she was up to is dropped, as
    /// [`Osaka::place`] drops it.
    pub fn dash_through(&mut self, door: DoorSpot, now: u64) {
        let (x, y) = door.spot();
        self.place(x, y, now);
        self.facing = door.into_room();
        self.dash = Some(Dash::In);
        self.set(
            Act::Door {
                since: now.saturating_sub(there_ms(Through::Home(door))),
                to: Through::Home(door),
                gap: 0,
            },
            now,
        );
    }

    /// Off to poke the scrollback accordion, standing at `spot` on it:
    /// whatever she's doing is dropped (a fall, a climb or a door she's
    /// already through runs its course first), and she walks there along
    /// her floor or takes a door. Out of sight, she comes back through
    /// one.
    pub fn errand(&mut self, spot: (i32, i32), terrain: &Terrain, at: u64) {
        tracing::debug!(?spot, "houseguest: off to poke the accordion");
        self.errand = Some(spot);
        // Not out by her routine any more: the accordion first (on her
        // way out through her door, it opens on the accordion instead).
        // Setting off again after, she says so again.
        self.leaving = None;
        self.set_off = None;
        // Out of her night's sleep: up groggy, and back to bed after
        // (her routine's bed reflex). Only in the night: a night's sleep
        // the stage gives her by day has no bed to go back to.
        if self.sleeping() && self.asleep_slot(at) {
            tracing::info!("houseguest: up groggy for the accordion");
            self.groggy = true;
        }
        // On her way to work, however it finds her (walking, heading
        // there across floors, round the screen's edge, up a pole): work
        // can wait, and her way to her door for it goes with it. (Through
        // her door for it, its door's arm below cuts it by the share she
        // worked; back out of it, she comes home first.)
        if matches!(self.shift, Some(Shift::Going { .. })) {
            self.let_work_go(at);
        }
        if self.aloft() {
            return;
        }
        match self.act {
            Act::Door { since, to, gap } => {
                let there =
                    door_beat(at.saturating_sub(since), gap, to).is_none_or(|(beat, _)| beat.there);
                if !there {
                    // Not out of it yet: it opens on the accordion
                    // instead (a shift through it can wait).
                    self.cut_shift(at);
                    self.redirect_door(Through::Space(spot), 0, at);
                }
            }
            Act::Away { .. } | Act::Out { .. } => {
                self.set(
                    Act::Door {
                        since: at.saturating_sub(through_ms(Through::Space(spot))),
                        to: Through::Space(spot),
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
        self.say(
            if self.groggy_at(at) {
                SLEEPY_POKE
            } else {
                POKE
            },
            at,
        );
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
    /// clear of it. A door that's a way out of hers (a gap: her shift's)
    /// opens only clear of her pieces too (`obstacles`, as
    /// [`Chances::obstacles`]; door batch M25, step 10a); a door between
    /// floors, or an errand's, only clear of the pane, as any door in
    /// space between floors.
    pub fn evict(
        &mut self,
        focus: Rect,
        terrain: &Terrain,
        obstacles: &[Rect],
        chat: Option<Rect>,
        now: u64,
        rng: &mut Rng,
    ) -> bool {
        let clear = |spot: (i32, i32)| !box_meets(focus, spot);
        let clear_of_all = |spot: (i32, i32)| clear(spot) && Clear::of(spot, obstacles).is_some();
        // Where a way out of hers opens: clear of the pane and her pieces.
        let place_out = |rng: &mut Rng| {
            calm_elsewhere(terrain, &clear_of_all, chat, rng)
                .and_then(|spot| Clear::of(spot, obstacles))
        };
        // Where a door between floors opens: clear of the pane.
        let place_in = |rng: &mut Rng| calm_elsewhere(terrain, &clear, chat, rng);
        // Leaving by her routine (A9): `leaving` is set only once her
        // external door has opened (door batch D6), so she's in its beats
        // or through it already (or off the screen): rained out of it,
        // out for good, with no shift and nothing drawn. The guest ends
        // the visit, her closed door in the pane until it's left alone.
        // On her walk to her door she isn't leaving yet: the ordinary
        // arms below move her out of the pane, and her routine sends her
        // on from there, silently (her line is latched, `set_off`).
        if self.leaving.is_some() {
            if !clear((self.x, self.y)) {
                tracing::debug!("houseguest: out of the focused pane, out for good");
                match &mut self.act {
                    Act::Door { since, to, gap } => {
                        *since = (*since).min(now.saturating_sub(through_ms(*to)));
                        *gap = u64::MAX;
                        self.act_due = self.first_due(now);
                    }
                    Act::Away { .. } => {}
                    _ => {
                        let (x, y) = (self.x, self.y);
                        self.set(
                            Act::Away {
                                until: u64::MAX,
                                enter: x,
                                to_y: y,
                                to_x: x,
                            },
                            now,
                        );
                    }
                }
            }
            return true;
        }
        match self.act {
            // Out of sight: when she's back in, she'll be moved on.
            Act::Away { .. } => return true,
            // Not through yet: the far door opens somewhere else instead.
            // But her own door while she's out through it (work's gap:
            // nothing of hers or it is drawn): a focused pane is never a
            // reason to move it (door batch, step 4b). Still focused as
            // she comes back out, she's moved on from there (below).
            Act::Door { since, to, gap } => {
                let beat = door_beat(now.saturating_sub(since), gap, to);
                let there = beat.is_none_or(|(beat, _)| beat.there);
                let out = beat.is_some_and(|(beat, _)| beat.door.is_none());
                if out && matches!(to, Through::Home(_)) {
                    return true;
                }
                if there {
                    if clear((self.x, self.y)) {
                        return true;
                    }
                } else {
                    if clear(to.spot()) {
                        return true;
                    }
                    // A way out (its gap): where her pieces leave room
                    // too; with none, she isn't out after all, and it's
                    // a door between floors out of the pane.
                    if gap > 0 {
                        if let Some(clear) = place_out(rng) {
                            tracing::debug!(spot = ?clear.spot(), "houseguest: her door opens elsewhere");
                            self.redirect_out(clear, gap, now);
                            return true;
                        }
                        self.not_out_by(now);
                    }
                    let Some(spot) = place_in(rng) else {
                        return false;
                    };
                    tracing::debug!(?spot, "houseguest: her door opens elsewhere");
                    self.redirect_door(Through::Space(spot), 0, now);
                    return true;
                }
            }
            _ => {
                if clear((self.x, self.y)) {
                    return true;
                }
            }
        }
        // On her way out to work (walking or heading to her door, or
        // stepping out of a piece to go out where she stands: door batch
        // C4), the shift still happens, by this door: its gap is her
        // shift's, never 0. Coming home from it, the door is the way in.
        // With nowhere clear of her pieces too, work can wait, and it's
        // a door between floors out of the pane (as `nowhere_clear`).
        let shift = match self.shift {
            Some(Shift::Going { gap }) if self.work_way() => Some(gap),
            _ => None,
        };
        if let Some(gap) = shift {
            if let Some(clear) = place_out(rng) {
                tracing::debug!(from = ?(self.x, self.y), to = ?clear.spot(), "houseguest: out of the focused pane, to work");
                self.drop_heading(Letting::Other);
                self.shift = Some(Shift::Out);
                self.open_out(
                    clear,
                    now.saturating_sub(through_ms(Through::Space(clear.spot()))),
                    gap,
                    now,
                );
                return true;
            }
            tracing::info!(
                "houseguest: out of the focused pane, nowhere clear of her pieces to go to work by: work can wait"
            );
            self.let_work_go(now);
        }
        let Some(spot) = place_in(rng) else {
            return false;
        };
        tracing::debug!(from = ?(self.x, self.y), to = ?spot, "houseguest: out of the focused pane");
        self.set(
            Act::Door {
                since: now.saturating_sub(through_ms(Through::Space(spot))),
                to: Through::Space(spot),
                gap: 0,
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
        // At text in the chat (pulling, swapping, tearing it off, or a
        // strip of it to read): what she had of it went back. Where the
        // text is counts, as for what went back, not where she stands.
        let busy_there = self.act.at_job().is_some_and(|job| {
            job.text_cells()
                .iter()
                .any(|&(x, y)| chat.contains((x, y).into()))
        });
        if busy_there {
            tracing::debug!("houseguest: shaken off in the chat");
            self.interrupt(Cause::Shaken, now);
        }
    }

    /// Off to her part-time job (door batch, step 4b): through her door,
    /// always, as school is. She walks to where the frame stands it
    /// (`chances.door`; a hop or a door in space between floors on the
    /// way, as any walk), and goes through it for her shift (its gap, 1–3
    /// minutes, drawn now), coming home out of it with her shopping
    /// ([`Osaka::back_home`]). With no door of hers anywhere, or no way
    /// to it, a door in space where she stands, once out of any piece
    /// ([`Osaka::out_where_clear`]).
    pub fn go_to_work(&mut self, terrain: &Terrain, chances: &Chances, at: u64, rng: &mut Rng) {
        let gap = rng.range(SHIFT_MS.0, SHIFT_MS.1);
        tracing::info!(door = ?chances.door.map(DoorSpot::spot), gap, "houseguest: off to work");
        self.shift = Some(Shift::Going { gap });
        self.worked = true;
        self.on_to_work(terrain, chances, at);
    }

    /// On her way to work (her shift set): to her door (`chances.door`)
    /// and through it, or a door in space where she stands, clear of her
    /// pieces (see [`Osaka::go_to_work`]).
    /// Also where her way there goes on from a landing (`choose_next`).
    fn on_to_work(&mut self, terrain: &Terrain, chances: &Chances, at: u64) {
        let Some(spot) = chances.door else {
            return self.out_where_clear(Leave::Work, terrain, chances, at);
        };
        let job = Job::Leave {
            spot,
            why: Leave::Work,
        };
        if spot.spot() == (self.x, self.y) {
            // Where she was heading: there (as `go_to` has it).
            #[cfg(test)]
            if self.heading.is_some() {
                self.headings.push("arrived".to_owned());
            }
            self.heading = None;
            return self.through_her_door(spot, Leave::Work, at);
        }
        let went = terrain
            .platform_at(self.x, self.y)
            .is_some_and(|here| self.go_to(Want::Work, job, here, terrain, at));
        if !went {
            tracing::debug!(door = ?spot.spot(), "houseguest: no way to her door for work");
            self.out_where_clear(Leave::Work, terrain, chances, at);
        }
    }

    /// Heading for her door for work across floors (door batch, step
    /// 4b).
    fn work_heading(&self) -> bool {
        self.heading
            .as_ref()
            .is_some_and(|h| h.job.leave() == Some(Leave::Work))
    }

    /// Walking for work: to her door, or out of a piece to go out where
    /// she stands (door batch, step 4b).
    fn work_walk(&self) -> bool {
        matches!(
            &self.act,
            Act::Walk {
                then: Then::Job(
                    Job::Leave {
                        why: Leave::Work,
                        ..
                    } | Job::Out {
                        why: Leave::Work,
                        ..
                    }
                ),
                ..
            }
        )
    }

    /// On her way out to work, by whatever stands of it (door batch,
    /// step 4b): walking for it, or heading for her door for it. The one
    /// reading of "her way to work" (her shift, census, eviction, an
    /// errand).
    fn work_way(&self) -> bool {
        self.work_heading() || self.work_walk()
    }

    /// On her way to work as she was going, to go on with it at a
    /// decision (door batch, step 4b): heading there across floors
    /// mid-hop (no startle since), or walking for it.
    fn on_her_way_to_work(&self) -> bool {
        self.hopping && self.work_heading() || self.work_walk()
    }

    /// Work can wait (door batch, step 4b): her shift is cut
    /// ([`Osaka::cut_shift`]: on her way to it, with nothing worked), and
    /// where she was heading for it goes with it (her hop's next leg
    /// too), so nothing sends her on to her door for a shift she no
    /// longer has (`leave_by`'s "never").
    fn let_work_go(&mut self, at: u64) {
        self.cut_shift(at);
        if self.work_heading() {
            self.drop_heading(Letting::Other);
            self.hopping = false;
        }
    }

    /// At the far side of the door she's through (`to`): there, and out
    /// of her own door into the room (door batch C6), having bumped into
    /// what fills its space if her pieces pushed it out of it (M13).
    fn out_of(&mut self, to: Through) {
        (self.x, self.y) = to.spot();
        if let Through::Home(door) = to {
            self.facing = door.into_room();
            if door.bumped() && !self.bumped {
                tracing::debug!(door = ?door.spot(), "houseguest: in by her door, out of its space: she bumped into what fills it");
                self.bumped = true;
            }
        }
    }

    /// Whether she has come in this visit through her door where her
    /// pieces filling its space pushed it (see the field).
    pub(super) fn bumped(&self) -> bool {
        self.bumped
    }

    /// Back home out of her door at `at`, if she was out: by her routine
    /// ([`Osaka::home_from`]), or from work ("I'm home!", showing her
    /// shopping). Returns whether she was.
    fn back_home(&mut self, at: u64) -> bool {
        if let Some(why) = self.returning.take() {
            self.home_from(why, at);
            return true;
        }
        if self.shift != Some(Shift::Out) {
            return false;
        }
        self.shift = None;
        self.come_home(at);
        true
    }

    /// Home by her routine (`why`) at `at`: "I'm home!" (pooled), with no
    /// shopping (her leeks are work's), held a moment so it shows; a
    /// parcel at the door waits for it (see [`Osaka::awake`]).
    fn home_from(&mut self, why: Routine, at: u64) {
        tracing::info!(?why, "houseguest: home");
        let line = self.lines.pick(mind::HOME, self.whims, at);
        if let Some(line) = line {
            self.say(line, at);
            self.morning_until = self.morning_until.max(at + speech_ms(line));
        }
        self.set(
            Act::Stand {
                until: at + line.map_or(1000, speech_ms),
            },
            at,
        );
    }

    /// Her shift ends at `at` without her coming home from it: "Work can
    /// wait" (an errand), the stage putting her somewhere, school
    /// beginning, or her next decision on her way out. What she worked of
    /// it eases her restlessness by its share: through her door (or a
    /// door in space for it), of its gap (all of it once she's back out);
    /// on her way out, none (nothing eased, and the credit settled). The
    /// one place a shift is cut short (one run its course is
    /// [`Osaka::come_home`]'s); call it before her act is changed. On her
    /// way out, call [`Osaka::let_work_go`] instead, so her way there
    /// goes too.
    fn cut_shift(&mut self, at: u64) {
        let Some(shift) = self.shift.take() else {
            return;
        };
        let share = match (&self.act, shift) {
            // Through her door to work, whose gap is her shift.
            (Act::Door { since, to, gap }, Shift::Out) if *gap > 0 => {
                at.saturating_sub(since.saturating_add(through_ms(*to))) as f64 / *gap as f64
            }
            // Back out of it.
            (_, Shift::Out) => 1.0,
            // On her way to it: she hasn't started.
            (_, Shift::Going { .. }) => 0.0,
        }
        .clamp(0.0, 1.0);
        tracing::debug!(?shift, share, "houseguest: her shift cut short");
        self.credit_share(Want::Work, share, Via::Shift, at);
        if self.credit == Some(Want::Work) {
            self.credit = None;
        }
    }

    /// Home from work at `at`: "I'm home!", showing her shopping. Always
    /// that line (never "Tadaima!"), but it's her coming-home pool's
    /// line, and noted as said from it, as it cools there too.
    fn come_home(&mut self, at: u64) {
        tracing::info!("houseguest: back from work");
        // Her shift done, out and about: restlessness eased. (A shift cut
        // short, by an errand, the stage or school, is settled by the
        // share she worked: see [`Osaka::cut_shift`].)
        self.credit_whole(Want::Work, Via::Shift, at);
        self.say(HOME, at);
        self.lines.note(PoolId::Routine, HOME, at);
        self.morning_until = self.morning_until.max(at + speech_ms(HOME));
        self.set(
            Act::Home {
                until: at + HOME_MS,
            },
            at,
        );
    }

    /// Her pose, face and bubble at `now`.
    pub fn appearance(&self, now: u64) -> (Pose, Face, Option<Bubble>) {
        let (pose, face, bubble) = self.look_at(now);
        // On a pose she holds, now and then a slow blink.
        let face = if self.held_blink(now) {
            Face::Blink
        } else {
            face
        };
        (pose, face, bubble)
    }

    /// How she looks at `now`, but for her slow blink on a pose she holds
    /// (see [`Osaka::appearance`]).
    fn look_at(&self, now: u64) -> (Pose, Face, Option<Bubble>) {
        let (pose, face, bubble) = self.acting(now);
        // Looking up at the chat from her act, in its pose: startled,
        // puzzled, then watching plain-faced, her act's own bubble hidden
        // (what she's saying shows over it, as over any look). Not before
        // it begins (it waits for what she says about her home, which
        // isn't cut short: begun under the look, it shows over it), and
        // never on a doze.
        let (face, bubble) = match self.looking_up {
            Some(look) if now >= look.since && !self.grumbling(now) && !pose.dozes() => {
                if now < look.since + LOOK_UP_SURPRISED_MS {
                    (Face::Surprised, Some(Bubble::Bang))
                } else if now < look.since + LOOK_MS {
                    (Face::Curious, Some(Bubble::Huh))
                } else {
                    (Face::Vacant, None)
                }
            }
            _ => (face, bubble),
        };
        // Up groggy in the night, she blinks her way about.
        let face = if self.groggy_at(now) {
            Face::Blink
        } else {
            face
        };
        let speech = self
            .speech
            .filter(|&(_, until)| now < until)
            .map(|(text, _)| Bubble::Say(text))
            // What she says about her home isn't cut short.
            .filter(|_| !self.grumbling(now));
        // What she felt on sight, said standing: she cranes round,
        // saying so.
        if let Some(line) = self.aloud_at(now) {
            return (pose, Face::Curious, Some(Bubble::Say(line)));
        }
        (pose, face, speech.or(bubble))
    }

    /// Her bob's hold ([`Osaka::bob_held`]) in a part (or an act) begun
    /// at `from`: the moment held, ms into it, and the frame then.
    fn held_from(&self, from: u64) -> Option<(u64, u8)> {
        self.bob_held
            .and_then(|(at, frame)| Some((at.checked_sub(from)?, frame)))
    }

    /// `time`, a key's at `now`, with her bob's hold.
    fn held_time(&self, time: script::KeyTime, now: u64) -> script::KeyTime {
        script::KeyTime {
            held: self.held_from(now.saturating_sub(time.elapsed)),
            ..time
        }
    }

    /// The frame of what bobs on the frame in her act at `at` (a key's
    /// bob, lying back or reading on her back, reading a borrowed strip),
    /// held as it's drawn; `None` if nothing does.
    fn bob_frame_at(&self, at: u64) -> Option<u8> {
        match self.act {
            Act::Use {
                since, until, play, ..
            }
            | Act::Idle {
                since,
                until,
                play: Some(play),
                ..
            }
            | Act::SpaceOut {
                since,
                until,
                play: Some(play),
                ..
            } => {
                let (key, time) = play.key(since, until, at)?;
                key.bob(self.held_time(time, at))
            }
            Act::Idle {
                what, since, until, ..
            } => what.bobs_on_the_frame().then(|| {
                what.frame(
                    at.saturating_sub(since),
                    until.saturating_sub(since),
                    self.held_from(since),
                )
            }),
            Act::Borrow {
                since,
                phase: Borrowing::Read { until },
                ..
            } => Some(script::bob_frame(
                at.saturating_sub(since),
                0,
                until.saturating_sub(since),
                USE_FRAME_MS,
                self.held_from(since),
            )),
            _ => None,
        }
    }

    /// Something of hers changes in place at `at`, off her bob's grid:
    /// her act's end moved, her look up at the chat beginning, moving on
    /// or ending, or what she says (a stir's murmur too) coming, going or
    /// cut short. Her bob holds the frame it shows now until a frame on
    /// ([`script::bob_frame`]), so it never flips within a frame of that
    /// (phase 5c's tail, T4). Called before an end moves, so the frame is
    /// the one shown.
    fn hold_bob(&mut self, at: u64) {
        if let Some(frame) = self.bob_frame_at(at) {
            self.hold_frame(at, frame);
        }
    }

    /// Her bob held at `at` on `frame` ([`Osaka::hold_bob`]), unless it's
    /// held from later already: a hold never moves back. A tick that
    /// comes late reads her clock at its `now` (her wake time moving
    /// holds there) before it handles what fell due before it (a line
    /// ending): the client paints from `now` on, so the latest hold is
    /// what it shows held.
    fn hold_frame(&mut self, at: u64, frame: u8) {
        if self.bob_held.is_some_and(|(held, _)| held > at) {
            return;
        }
        tracing::trace!(at, frame, "houseguest: her bob held");
        self.bob_held = Some((at, frame));
    }

    /// How she looks dozing (`look`: asleep for the night, or by day),
    /// stirring at a chat line if she is at `now`: turned over, held
    /// there, blinking.
    fn stirring(
        &self,
        look: (Pose, Face, Option<Bubble>),
        now: u64,
    ) -> (Pose, Face, Option<Bubble>) {
        if now >= self.stir_until {
            return look;
        }
        let (pose, _, bubble) = look;
        (stir_turn(pose).unwrap_or(pose), Face::Blink, bubble)
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
            Act::Idle {
                play: Some(play),
                since,
                until,
                what,
            } => {
                let host = what.look(0).0;
                let look = play
                    .key(since, until, now)
                    .map_or(what.look(0), |(key, time)| {
                        key.look(self.held_time(time, now), host, &play)
                    });
                self.stirring(look, now)
            }
            Act::Idle {
                what, since, until, ..
            } => {
                let frame = what.frame(
                    now.saturating_sub(since),
                    until.saturating_sub(since),
                    self.held_from(since),
                );
                self.stirring(what.look(frame), now)
            }
            Act::Use {
                seat,
                since,
                until,
                play,
                grievance,
                ..
            } => {
                // Watching from a sofa, she sits on it; from the floor,
                // cross-legged (phase 5c D5).
                let host = if seat.item == Furniture::Sofa {
                    Pose::Lounge
                } else {
                    Pose::CrossLegged
                };
                let (pose, face, bubble) = play
                    .key(since, until, now)
                    .map_or((host, Face::Vacant, None), |(key, time)| {
                        key.look(self.held_time(time, now), host, &play)
                    });
                let pose = at_seat(pose, seat);
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
                    // Dozing (for the night, or by day), she may stir.
                    _ => self.stirring((pose, face, bubble), now),
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
            // Hand over hand at the line, in and back out; between, sat
            // beside the tear reading the strip, along it now and then.
            Act::Borrow { phase, since, .. } => {
                let pull = |step: u16| Pose::Pull {
                    heaving: step % 2 == 1,
                    row: self.hands_row(),
                };
                match phase {
                    Borrowing::Brace => (pull(0), Face::Curious, None),
                    Borrowing::Reel(step) => (pull(step), Face::Happy, None),
                    // Its last frame held through the part of a frame
                    // before she slides the strip back ([`script::bob_frame`]).
                    Borrowing::Read { until } => {
                        let frame = script::bob_frame(
                            now.saturating_sub(since),
                            0,
                            until.saturating_sub(since),
                            USE_FRAME_MS,
                            self.held_from(since),
                        );
                        (Pose::ReadStrip(frame), Face::Vacant, None)
                    }
                    Borrowing::Slide(step) => (pull(step), Face::Pleased, None),
                }
            }
            Act::SpaceOut {
                since,
                until,
                play: Some(play),
                ..
            } => play.key(since, until, now).map_or(
                (Pose::Stand, Face::Vacant, Some(Bubble::Dots)),
                |(key, time)| key.look(self.held_time(time, now), Pose::Stand, &play),
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
            Act::Door { since, to, gap } => {
                let there =
                    door_beat(now.saturating_sub(since), gap, to).is_some_and(|(b, _)| b.there);
                let face = if there { Face::Pleased } else { Face::Curious };
                // Home from work: her shopping comes in with her (door
                // batch C13), as she holds it once in. Through her own
                // door side-on in its wall, as its beat draws her (D8).
                let wall = match self.front_door(now) {
                    Some((_, FrontBeat::Wall(_, beat))) => beat.her.map(|(pose, _)| pose),
                    _ => None,
                };
                let pose = if let Some(pose) = wall {
                    pose
                } else if there && self.shift == Some(Shift::Out) {
                    Pose::Carry((now.saturating_sub(since) / 700 % 2) as u8)
                } else {
                    Pose::Stand
                };
                (pose, face, None)
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

/// How much likelier an offer is for making what it binds her to (phase
/// 5c D5): a piece of a kind she owns no real one of ([`Chances::real`])
/// [`MAKESHIFT_DRAW`] times as likely; a paper desk with none standing,
/// as much again while her back aches from homework on the floor
/// (`ached`; with a real desk standing, her back is nothing to making
/// another). Anything else, 1.
/// Above 1 always, so it never pushes the want itself out of her best
/// few (`brain::choose`).
fn making(bind: &Bind, chances: &Chances, ached: bool) -> f64 {
    let Bind::Job(Job::Build(build)) = bind else {
        return 1.0;
    };
    let item = build.piece.item;
    if chances.real.contains(&item) {
        return 1.0;
    }
    let back = if ached && item == Furniture::Desk {
        MAKESHIFT_DRAW
    } else {
        1.0
    };
    MAKESHIFT_DRAW * back
}

/// How much likelier making a piece is while she owns no real one of
/// its kind (phase 5c D5), and making a desk again while her back aches.
pub(super) const MAKESHIFT_DRAW: f64 = 3.0;

/// Her pose `pose` (as a use's script poses her) at `seat`: at her paper
/// desk (phase 5c D5), kneeling beside it, where the script has her on
/// her stool at a real desk. The one place a made piece's pose differs
/// from its real kind's.
fn at_seat(pose: Pose, seat: Seat) -> Pose {
    match pose {
        Pose::Homework(frame) if seat.makeshift() && seat.item == Furniture::Desk => {
            Pose::PaperDesk(frame)
        }
        other => other,
    }
}

/// The next animation-frame boundary of `what` after `now`, or far away
/// for a held pose.
fn next_frame(what: Activity, since: u64, now: u64) -> u64 {
    let period = what.period();
    if what == Activity::SitDoze && now < since + SIT_DOZE_NOD_MS {
        return since + SIT_DOZE_NOD_MS;
    }
    if what == Activity::Gaze && now < since + GAZE_OOH_MS {
        return since + GAZE_OOH_MS;
    }
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
            let w = (0..10_000)
                .map(Whims)
                .find(|&w| Lines::default().pick(mind::DOOR, w, 0) == Some(line))
                .expect("a whim that picks it");
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

    /// A daydream with a musing in it (her mood's session has one or
    /// more: phase 5c B6) begins with a riddle one time in three, and only
    /// while she's quiet; one with none says nothing at all. In every
    /// mood, with the levers she ships with, its length her mood's.
    #[test]
    fn a_riddle_is_told_one_musing_in_three_and_only_when_quiet() {
        for mood in Mood::ALL {
            let (mut riddles, mut musing) = (0, 0);
            for seed in 0..120 {
                for talking in [false, true] {
                    let at = format!("{mood:?} seed {seed} talking={talking}");
                    let mut rng = Rng(seed);
                    let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                    osaka.set_mood(mood);
                    osaka.whims = Whims(seed);
                    if talking {
                        osaka.say(OK, 0);
                    }
                    let mut alone = rng.clone();
                    osaka.muse(0, &mut rng);
                    let span = osaka.lingered(true, alone.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1));
                    assert_eq!(rng.0, alone.0, "{at}: one body draw");
                    let Act::SpaceOut {
                        since,
                        until,
                        play,
                        session,
                    } = osaka.act
                    else {
                        panic!("{at}: spacing out");
                    };
                    assert_eq!((since, until), (0, span), "{at}");
                    match play {
                        Some(play) => {
                            assert!(!talking, "{at}: a riddle over speech");
                            riddles += 1;
                            musing += 1;
                            let (question, answer) = RIDDLES[usize::from(play.drawn[0])];
                            assert_eq!(osaka.speech, None, "{at}");
                            assert_eq!(
                                osaka.lines.said(),
                                [
                                    (PoolId::Riddle, question, 0),
                                    (PoolId::Riddle, answer, script::RIDDLE_ASKED_MS),
                                ],
                                "{at}"
                            );
                            assert_eq!(osaka.act_due, script::RIDDLE_ASKED_MS, "{at}");
                        }
                        // Talking, a daydream with nothing to say leaves
                        // her saying it: no session, due at its end.
                        None if talking && osaka.speech.is_some_and(|(said, _)| said == OK) => {
                            assert_eq!(session, None, "{at}");
                            assert_eq!(osaka.act_due, until, "{at}");
                        }
                        // Her first musing is said over what she was
                        // saying (only a riddle waits for quiet), the
                        // rest of her session to come.
                        None => match osaka.speech {
                            Some((said, _)) => {
                                // Over speech, no riddle could be told.
                                musing += usize::from(!talking);
                                assert!(mind::MUSINGS.lines.contains(&said), "{at}: {said}");
                                // Due at her session's next musing, if one
                                // comes before the end.
                                let next = session.map_or(until, |s| s.next.min(until));
                                assert_eq!(osaka.act_due, next, "{at}");
                            }
                            // A daydream with nothing to say: her mood's
                            // sessions may hold none.
                            None => assert_eq!(
                                osaka.stillness.musings.of(mood).0,
                                0,
                                "{at}: a musing due"
                            ),
                        },
                    }
                }
            }
            assert!(
                (musing / 5..=musing / 2).contains(&riddles),
                "{mood:?}: {riddles} riddles in {musing} daydreams with a musing"
            );
        }
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
                play: None,
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
            Use::LookOut,
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
    /// furniture (the TV's static and Chiyo-chichi's hook on their
    /// frames, a programme or Chiyo-chichi held, the lamp, the fridge,
    /// the cat), on a sofa or not, for every use.
    #[test]
    fn every_use_look_span_is_half_open() {
        use super::super::art::Channel;
        /// Her bob's frame, `t` ms in, in a span (a key) from `start` to
        /// `end`: on its frames, but a frame clear of the span's start and
        /// end (Round 8). This takes the frame from `script::bob_frame`
        /// itself, so here it checks the wiring only (each key's span
        /// reaching it as played); `bob_frame`'s values are held against
        /// the rule independently by its own tests in script.rs
        /// (`a_bob_frame_flips_on_the_grid_clear_of_its_ends`, its
        /// property, and `a_bob_flips_a_frame_clear_of_its_keys_start_and_end`
        /// over every script).
        fn bob(t: u64, (start, end): (u64, u64)) -> u8 {
            script::bob_frame(t, start, end, USE_FRAME_MS, None)
        }
        /// The TV's frame, `t` ms in.
        fn tv(t: u64) -> u8 {
            (t / script::CHANNEL_FRAME_MS % 2) as u8
        }
        /// Her pose and what's on her furniture, `t` ms in, in a span
        /// from and to (ms in).
        type Body = fn(u64, (u64, u64)) -> (Pose, Option<Prop>);
        /// A span's start, her face and bubble, and her body.
        type Span = (u64, Face, Option<Bubble>, Body);
        /// A use, on a sofa or not, its advert, and its spans.
        type Case = (Use, bool, Option<Furniture>, Vec<Span>);
        let pitch = Furniture::Lamp.spec().pitch;
        let snow: Body = |t, _| (Pose::CrossLegged, Some(Prop::Tv(Channel::Snow(tv(t)))));
        let snow_on_sofa: Body = |t, _| (Pose::Lounge, Some(Prop::Tv(Channel::Snow(tv(t)))));
        let selling: Body = |t, _| (Pose::CrossLegged, Some(Prop::Tv(Channel::Shopping(tv(t)))));
        let selling_on_sofa: Body = |t, _| (Pose::Lounge, Some(Prop::Tv(Channel::Shopping(tv(t)))));
        // The programme a plain play holds (none drawn: the first), and
        // Chiyo-chichi after his hook.
        const NEWS: Option<Prop> = Some(Prop::Tv(Channel::Programme(art::Programme::News)));
        const SOLD: Option<Prop> = Some(Prop::Tv(Channel::Shopping(0)));
        let held: Body = |_, _| (Pose::CrossLegged, NEWS);
        let held_on_sofa: Body = |_, _| (Pose::Lounge, NEWS);
        let sold_held: Body = |_, _| (Pose::CrossLegged, SOLD);
        let sold_held_on_sofa: Body = |_, _| (Pose::Lounge, SOLD);
        let sold = |hook: Body, held: Body, length: u64| {
            vec![
                (0, Face::Curious, Some(Bubble::Ooh), hook),
                (length * 2 / 5, Face::Happy, Some(Bubble::Say(pitch)), held),
                (length * 3 / 5, Face::Curious, None, held),
            ]
        };
        for length in [7, 5000, 5250, 12_345] {
            let cases: Vec<Case> = vec![
                (
                    Use::Lounge,
                    false,
                    None,
                    vec![(0, Face::Vacant, None, |_, _| (Pose::Lounge, None))],
                ),
                (
                    Use::Lounge,
                    true,
                    None,
                    vec![(0, Face::Vacant, None, |_, _| (Pose::Lounge, None))],
                ),
                (
                    Use::Nap,
                    true,
                    None,
                    vec![(0, Face::Blink, Some(Bubble::Zzz), |t, span| {
                        (Pose::Nap(bob(t, span)), None)
                    })],
                ),
                (
                    Use::Sleep,
                    false,
                    None,
                    vec![
                        (0, Face::Blink, Some(Bubble::Dots), |t, span| {
                            (Pose::Sleep(bob(t, span)), None)
                        }),
                        (
                            script::LAMP_ON_MS,
                            Face::Blink,
                            Some(Bubble::Zzz),
                            |t, span| (Pose::Sleep(bob(t, span)), Some(Prop::LampOff)),
                        ),
                    ],
                ),
                (
                    Use::Homework,
                    false,
                    None,
                    vec![
                        (0, Face::Vacant, None, |t, span| {
                            (Pose::Homework(bob(t, span)), None)
                        }),
                        (length / 2, Face::Blink, Some(Bubble::Dots), |_, _| {
                            (Pose::Homework(2), None)
                        }),
                        (length * 3 / 4, Face::Blink, Some(Bubble::Zzz), |_, _| {
                            (Pose::Homework(3), None)
                        }),
                    ],
                ),
                (
                    Use::Watch,
                    false,
                    None,
                    vec![
                        (0, Face::Curious, None, snow),
                        (script::STATIC_MS, Face::Curious, None, held),
                    ],
                ),
                (
                    Use::Watch,
                    true,
                    None,
                    vec![
                        (0, Face::Curious, None, snow_on_sofa),
                        (script::STATIC_MS, Face::Curious, None, held_on_sofa),
                    ],
                ),
                (
                    Use::Watch,
                    false,
                    Some(Furniture::Lamp),
                    sold(selling, sold_held, length),
                ),
                (
                    Use::Watch,
                    true,
                    Some(Furniture::Lamp),
                    sold(selling_on_sofa, sold_held_on_sofa, length),
                ),
                (
                    Use::Read,
                    false,
                    None,
                    vec![(0, Face::Vacant, None, |t, span| {
                        (Pose::Read(bob(t, span)), None)
                    })],
                ),
                (
                    Use::Snack,
                    false,
                    None,
                    vec![
                        (0, Face::Curious, None, |_, _| {
                            (Pose::Side, Some(Prop::FridgeOpen))
                        }),
                        (1500, Face::Happy, None, |t, span| {
                            (Pose::Eat(bob(t, span)), None)
                        }),
                    ],
                ),
                (
                    Use::Pet,
                    false,
                    None,
                    vec![
                        (0, Face::Happy, Some(Bubble::Hum), |_, _| {
                            (Pose::Pet(0), None)
                        }),
                        (
                            length * 7 / 10,
                            Face::Surprised,
                            Some(Bubble::Say(line!("Ow!"))),
                            |_, _| (Pose::Pet(1), Some(Prop::CatBiting)),
                        ),
                    ],
                ),
                (
                    Use::Crumple,
                    false,
                    None,
                    vec![
                        (0, Face::Happy, Some(Bubble::Say(SCRUNCH)), |t, span| {
                            (Pose::ToeTouch(bob(t, span)), None)
                        }),
                        (
                            length * 4 / 5,
                            Face::Happy,
                            Some(Bubble::Say(THERE)),
                            |t, span| (Pose::ToeTouch(bob(t, span)), None),
                        ),
                    ],
                ),
                (
                    Use::Unpack,
                    false,
                    None,
                    vec![
                        (0, Face::Happy, None, |t, span| {
                            (Pose::ToeTouch(bob(t, span)), None)
                        }),
                        (length * 3 / 5, Face::Happy, Some(Bubble::Ooh), |t, span| {
                            (Pose::ToeTouch(bob(t, span)), None)
                        }),
                    ],
                ),
                (
                    Use::LookOut,
                    false,
                    None,
                    vec![
                        (
                            0,
                            Face::Curious,
                            Some(Bubble::Say(script::LOOK_OUT_LINES[0].1)),
                            |_, _| (Pose::SillLean, None),
                        ),
                        (script::LOOK_OUT_LINE_MS, Face::Curious, None, |_, _| {
                            (Pose::SillLean, None)
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
                        let (pose, prop) = body(elapsed, (start, end));
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
    /// way her watch runs from the line (so after the andagi, only if it
    /// ended within 5 s of it), and mischief she owes goes back at once.
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
                assert_eq!(osaka.answered_until(), None, "{case}: not asked yet");
                osaka.look(now, 30, asks, &terrain);
                assert_eq!(osaka.facing, Facing::Right, "{case}: turned to the chat");
                assert_eq!(osaka.watch_until, now + WATCH_MS, "{case}");
                assert!(
                    osaka.pending.iter().all(|&(due, _)| due == now),
                    "{case}: the mischief goes back"
                );
                if !(asks && in_coda) {
                    assert!(!matches!(osaka.act, Act::Use { .. }), "{case}: stopped");
                    assert_eq!(osaka.answered_until(), None, "{case}: not answered");
                    continue;
                }
                assert_eq!(osaka.use_span(), Some((seat, since, until)), "{case}");
                let said = now + speech_ms(SATA_ANDAGI);
                assert_eq!(osaka.answered_until(), Some(said), "{case}");
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
                assert_eq!(
                    osaka.answered_until(),
                    Some(late + speech_ms(SATA_ANDAGI)),
                    "{case}: answered again, its own time"
                );
                let homework = seat_for(Use::Homework, Furniture::Desk);
                osaka.start_job(Job::Use(homework), until, &Chances::default(), &mut Rng(1));
                let (.., play) = begun(&osaka);
                assert_eq!((play.own, play.before), (ScriptId::Homework, None));
                assert_eq!(osaka.appearance(until + 100).1, Face::Vacant, "{case}");
                assert_eq!(
                    osaka.answered_until(),
                    None,
                    "{case}: the answer left with it"
                );
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

    /// Every watch draws the programme her TV holds as it begins, from
    /// her decision's whims, whatever it plays (phase 5c D7): a plain
    /// watch, the shopping channel, a cued surf, a watch with her home on
    /// her mind; the same card from the same whims, each of the four
    /// from some, and drawing it draws nothing from her generator (what
    /// she does never hangs on whether a picture of the film shows
    /// instead, phase 5c step 12b). Any other use keeps the first.
    #[test]
    fn every_watch_draws_its_programme() {
        use super::super::art::Programme;
        let seat = seat_for(Use::Watch, Furniture::Tv);
        let plain = Chances::default();
        let selling = Chances {
            advert: Some(Furniture::Lamp),
            ..Chances::default()
        };
        let broken = Chances {
            broken: broken_for(Use::Watch, Furniture::Tv).into_iter().collect(),
            ..Chances::default()
        };
        let mut drawn = std::collections::HashSet::new();
        for seed in 0..64 {
            let start = |chances: &Chances, cue: Option<Cue>| {
                let (osaka, rng) = started(seat, 1000, chances, cue, None, seed);
                (begun(&osaka).3, rng)
            };
            let (watch, rng) = start(&plain, None);
            let card = watch.card;
            // From her decision's whims, as `started` seeds them (this
            // pins the helper's derivation too; the spec's purity is the
            // generator check below).
            assert_eq!(
                card,
                Programme::ALL[Whims(seed ^ 0x5eed).below("programme", 4) as usize],
                "seed {seed}"
            );
            for (name, (play, _)) in [
                ("sold", start(&selling, None)),
                ("surfing", start(&plain, Some(Cue::Script(ScriptId::Surf)))),
                ("her home on her mind", start(&broken, None)),
            ] {
                assert_eq!(play.card, card, "seed {seed}: {name} ({:?})", play.own);
            }
            // As much drawn from her generator as before there were
            // programmes: her length, and nothing more.
            let mut alone = Rng(seed);
            Osaka::standing_at(10, 10, 0, &mut alone);
            let (lo, hi) = use_duration(Use::Watch);
            let _ = alone.range(lo, hi);
            assert_eq!(rng, alone.0, "seed {seed}");
            drawn.insert(card);
        }
        assert_eq!(drawn.len(), Programme::ALL.len(), "{drawn:?}");
        let sit = seat_for(Use::Lounge, Furniture::Sofa);
        let (osaka, _) = started(sit, 1000, &plain, None, None, 3);
        assert_eq!(begun(&osaka).3.card, Programme::News);
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
        let surfing = (0..10_000)
            .find(|&seed| {
                let seat = seat_for(Use::Watch, Furniture::Tv);
                begun(&started(seat, 1000, &plain, None, None, seed).0)
                    .3
                    .own
                    == ScriptId::Surf
            })
            .expect("a seed she surfs on");
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
        let surfing = (0..10_000)
            .find(|&seed| surfs_on(seed))
            .expect("a seed she surfs on");
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

    /// Surfing: snow, colour bars, snow, the sunrise (ooh!), then the
    /// programme she drew to the end, humming, pleased; in her watching
    /// pose throughout.
    #[test]
    fn surfing_flicks_through_the_channels_in_turn() {
        use super::super::art::{Channel, Programme};
        use script::SURF_MS;
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
        let length = use_duration(Use::Watch).0;
        for card in Programme::ALL {
            let play = Play {
                card,
                ..Play::plain(ScriptId::Surf)
            };
            for sofa in [false, true] {
                let host = if sofa {
                    Pose::Lounge
                } else {
                    Pose::CrossLegged
                };
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
                    (face, bubble, prop.map(on_frame_0))
                };
                let snow = Some(Prop::Tv(Channel::Snow(0)));
                let held = Some(Prop::Tv(Channel::Programme(card)));
                assert_eq!(at(0), (Face::Curious, None, snow));
                assert_eq!(at(SURF_MS - 1).2, snow);
                assert_eq!(
                    at(SURF_MS),
                    (Face::Vacant, None, Some(Prop::Tv(Channel::ColourBars)))
                );
                assert_eq!(at(2 * SURF_MS).2, snow);
                assert_eq!(
                    at(3 * SURF_MS),
                    (
                        Face::Curious,
                        Some(Bubble::Ooh),
                        Some(Prop::Tv(Channel::Sunrise))
                    )
                );
                assert_eq!(at(4 * SURF_MS), (Face::Happy, Some(Bubble::Hum), held));
                assert_eq!(at(length - 1), (Face::Happy, Some(Bubble::Hum), held));
            }
        }
    }

    /// Watching: static as she switches the TV on, for [`STATIC_MS`]
    /// (three of its frames), then the programme she drew, held to the
    /// end; the shopping channel: Chiyo-chichi bobbing through his hook
    /// (the first two fifths), then held on one frame. On a sofa or not.
    ///
    /// [`STATIC_MS`]: script::STATIC_MS
    #[test]
    fn the_tv_holds_its_picture_after_switching_on() {
        use super::super::art::{Channel, Programme};
        use script::{CHANNEL_FRAME_MS, STATIC_MS};
        assert_eq!(STATIC_MS, 3 * CHANNEL_FRAME_MS);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
        let length = use_duration(Use::Watch).0;
        for sofa in [false, true] {
            for card in Programme::ALL {
                let play = Play {
                    card,
                    ..Play::of(Use::Watch, None)
                };
                let use_ = Played {
                    what: Use::Watch,
                    sofa,
                    bought: None,
                    grievance: None,
                    length,
                };
                let frames: Vec<_> = (0..STATIC_MS)
                    .step_by(CHANNEL_FRAME_MS as usize)
                    .map(|t| played_as(&mut osaka, use_, play, t).1)
                    .collect();
                assert_eq!(
                    frames,
                    [0, 1, 0].map(|f| Some(Prop::Tv(Channel::Snow(f)))),
                    "sofa={sofa}: three frames of static"
                );
                for t in (STATIC_MS..length).step_by(97) {
                    assert_eq!(
                        played_as(&mut osaka, use_, play, t).1,
                        Some(Prop::Tv(Channel::Programme(card))),
                        "sofa={sofa} {card:?}: {t}"
                    );
                }
            }
            let sold = Played {
                what: Use::Watch,
                sofa,
                bought: Some(Furniture::Lamp),
                grievance: None,
                length,
            };
            let hook = length * 2 / 5;
            let bobbed: std::collections::HashSet<_> = (0..hook)
                .step_by(97)
                .map(|t| played(&mut osaka, sold, t).1)
                .collect();
            assert_eq!(bobbed.len(), 2, "sofa={sofa}: he bobs through his hook");
            for t in (hook..length).step_by(97) {
                assert_eq!(
                    played(&mut osaka, sold, t).1,
                    Some(Prop::Tv(Channel::Shopping(0))),
                    "sofa={sofa}: held after his hook, {t}"
                );
            }
        }
    }

    /// A TV act over 30 s holds still after its first 10 s (phase 5c
    /// D7): sampled every 100 ms, as a client painting for any reason
    /// would show it, no two changes of how she looks or of what's on
    /// her furniture come closer than [`USE_FRAME_MS`] once 10 s have
    /// passed, but Chiyo-chichi's bob through his hook (the first two
    /// fifths of the shopping channel). Watching, flicking through the
    /// channels and the shopping channel, on a sofa or not, a plain watch
    /// with a grievance said 20 s in or none, as long as she watches.
    #[test]
    fn a_long_tv_act_holds_still_after_its_first_ten_seconds() {
        use super::super::art::Channel;
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut Rng(1));
        let plays = [
            Play::of(Use::Watch, None),
            Play::plain(ScriptId::Surf),
            Play::of(Use::Watch, Some(Furniture::Lamp)),
        ];
        let (lo, hi) = use_duration(Use::Watch);
        for play in plays {
            for sofa in [false, true] {
                // (Only a plain watch has a grievance: the shopping
                // channel and surfing come on only without one.)
                let grievances: &[Option<u64>] = if play.own == ScriptId::Watch {
                    &[None, Some(20_000)]
                } else {
                    &[None]
                };
                for &grievance in grievances {
                    for length in [30_001, lo, hi, hi * 3 / 2] {
                        let at = format!("{:?} sofa={sofa} {grievance:?} {length}", play.own);
                        let use_ = Played {
                            what: Use::Watch,
                            sofa,
                            bought: play.bought,
                            grievance,
                            length,
                        };
                        let hook = length * 2 / 5;
                        let mut last = None;
                        let mut flipped: Option<u64> = None;
                        for t in (0..length).step_by(100) {
                            let shown = played_as(&mut osaka, use_, play, t);
                            if let Some(was) = last.replace(shown)
                                && was != shown
                            {
                                let bob = play.own == ScriptId::Shopping
                                    && t <= hook
                                    && was.0 == shown.0
                                    && matches!(
                                        (was.1, shown.1),
                                        (
                                            Some(Prop::Tv(Channel::Shopping(_))),
                                            Some(Prop::Tv(Channel::Shopping(_)))
                                        )
                                    );
                                if t >= 10_000 && !bob {
                                    if let Some(before) = flipped {
                                        assert!(
                                            t - before >= USE_FRAME_MS,
                                            "{at}: flipped at {before} and {t}: {was:?} to {shown:?}"
                                        );
                                    }
                                    flipped = Some(t);
                                }
                            }
                        }
                    }
                }
            }
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
    /// part's start, and on each of static's frames
    /// ([`script::CHANNEL_FRAME_MS`], from the part's start too) while a
    /// key showing it plays; as each of its keys ends (the part's end
    /// with its last); as she starts and stops saying what's wrong with
    /// her home (from `grievance` ms in); and at its end. Ascending, each
    /// once.
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
                // Static's frames, through each key showing it.
                let statics = keys
                    .iter()
                    .scan(from, move |start, k| {
                        let (key_from, key_to) = (*start, from + k.span.end(Some(to - from)));
                        *start = key_to;
                        Some((k.prop == Some(script::Shows::Static), key_from, key_to))
                    })
                    .filter(|&(snow, ..)| snow)
                    .flat_map(move |(_, key_from, key_to)| {
                        (1..)
                            .map(move |k| from + k * script::CHANNEL_FRAME_MS)
                            .skip_while(move |&t| t <= key_from)
                            .take_while(move |&t| t < key_to)
                    });
                frames.chain(ends).chain(statics)
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

    /// `prop` on its first frame: Chiyo-chichi's hook moves at paint
    /// time (static, on her wakes).
    fn on_frame_0(prop: Prop) -> Prop {
        use super::super::art::Channel;
        match prop {
            Prop::Tv(Channel::Shopping(_)) => Prop::Tv(Channel::Shopping(0)),
            prop => prop,
        }
    }

    /// Keys change on time: a use wakes her on every frame of the part
    /// playing (counted from its start), as each key ends and as a
    /// grievance comes and goes, and at nothing else, each wakeup
    /// strictly after the last; and how she looks, and what's on her
    /// furniture (static too; Chiyo-chichi's hook aside, which moves at
    /// paint time), only ever changes at a wakeup. Every use, the shopping channel too,
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
                                    (look, prop.map(on_frame_0))
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

    // ---- Her routine's levers (phase 5b step 3b) ----

    /// A blank 40×20 screen as she reads it.
    fn blank_terrain() -> Terrain {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        Terrain::read(&Buffer::empty(Rect::new(0, 0, 40, 20)), &[], false)
    }

    /// Her clock reading Monday `h:m` (a school day) at monotonic 0.
    /// From 22:00, bedtime (22:30) is five real minutes on.
    fn monday_at(h: u64, m: u64) -> routine::Clock {
        clock_at(0, h, m)
    }

    /// Her clock reading `h:m` of game day `day` (day 0 a Monday, from
    /// 16:00) at monotonic 0.
    fn clock_at(day: u64, h: u64, m: u64) -> routine::Clock {
        let game = ((day * 24 + h) * 60 + m - routine::START) * 60_000;
        routine::Clock::read(super::super::GameClock { at: 0, game }, 0, None, None)
    }

    /// Bedtime (Monday 22:30) in monotonic millis, from [`monday_at`]
    /// 22:00.
    const BED: u64 = 300_000;
    /// School (Tuesday 08:15) in monotonic millis, from 22:00: the next
    /// cutting boundary after bedtime.
    const SCHOOL: u64 = (2 * 60 + 8 * 60 + 15) * 10_000;

    /// Her external door in a right-hand wall, standing at `(x, y)`.
    fn wall_door(x: i32, y: i32) -> DoorSpot {
        DoorSpot::at(
            x,
            y,
            super::super::door::Set::Wall {
                side: super::super::room::Side::Right,
                wall: x + 3,
            },
        )
    }

    /// A later boundary never cuts her way out (door batch, step 4a,
    /// T11): at school's end (12:45) on her walk to her door, `cut`
    /// leaves the walk; by the structural match in `cut`'s `to_job`.
    /// A walk to a use, the same boundary cuts (the precondition).
    #[test]
    fn a_later_boundary_never_cuts_her_way_out() {
        let leave = Act::Walk {
            to: 40,
            then: Then::Job(Job::Leave {
                spot: wall_door(40, 10),
                why: Leave::School,
            }),
        };
        let to_use = Act::Walk {
            to: 40,
            then: Then::Job(Job::Use(seat_for(Use::Watch, Furniture::Tv))),
        };
        for (act, cut) in [(leave, false), (to_use, true)] {
            let (mut osaka, _) = at_bedtime(act.clone(), BED + 50);
            assert_eq!(osaka.cut(BED), cut, "{act:?}");
            assert_eq!(osaka.act == act, !cut, "{act:?}");
        }
    }

    /// Her, on a 40-wide floor at row 15 at `(x, 15)`, on a Tuesday at
    /// `h:m` (a school day) from monotonic 0, at `act` due now.
    fn on_a_school_day(x: i32, h: u64, m: u64, act: Act) -> (Osaka, Terrain, Rng) {
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(x, 15, 0, &mut rng);
        osaka.act = act;
        osaka.act_due = 0;
        osaka.read_clock(Some(clock_at(1, h, m)), 0);
        (osaka, floor_at(15), rng)
    }

    /// Her walk to her door, set off at 08:20 (the latch set).
    fn leave_walk(to: i32) -> Act {
        Act::Walk {
            to,
            then: Then::Job(Job::Leave {
                spot: wall_door(to, 15),
                why: Leave::School,
            }),
        }
    }

    /// On her walk to her door, a step from its spot (door batch D6,
    /// step 4a review): the frame's door read each step. Moved (on her
    /// floor): she walks on to where it stands now. Gone: out by a door
    /// in space where she stands, for good. Never out through it at the
    /// stale spot. Where it stood, but standing otherwise: out through
    /// it as it stands now.
    #[test]
    fn her_walk_to_her_door_follows_the_frames_door() {
        use super::super::door::{Fallback, Set};
        let moved = wall_door(30, 15);
        let restood = DoorSpot::at(20, 15, Set::Floor(Fallback::Protected));
        for door in [Some(moved), None, Some(restood)] {
            let (mut osaka, terrain, mut rng) = on_a_school_day(19, 8, 20, leave_walk(20));
            osaka.set_off = Some(Routine::School);
            let chances = Chances {
                door,
                ..Chances::default()
            };
            osaka.tick(1000, Some(clock_at(1, 8, 20)), &terrain, &chances, &mut rng);
            match door {
                Some(d) if d == moved => {
                    assert!((19..30).contains(&osaka.x), "on toward it: {}", osaka.x);
                    assert!(
                        matches!(
                            osaka.act,
                            Act::Walk { to: 30, then: Then::Job(Job::Leave { spot, .. }) } if spot == moved
                        ),
                        "{:?}",
                        osaka.act
                    );
                    assert_eq!(osaka.leaving, None);
                }
                None => {
                    assert_eq!(osaka.through(), Some(Through::Space((19, 15))));
                    assert_eq!(osaka.leaving, Some(Routine::School));
                }
                Some(d) => {
                    assert_eq!(osaka.through(), Some(Through::Home(d)));
                    assert_eq!(osaka.leaving, Some(Routine::School));
                }
            }
        }
    }

    /// With no way to her door's spot (on no floor of hers) or no door
    /// anywhere, she goes out by a door in space where she stands, but
    /// only once her box meets nothing of hers (door batch M25, the
    /// step 4a review): in a piece, she steps along her floor to the
    /// nearest clear spot first (no door, nothing set), and out there
    /// as she arrives, nothing decided between; out of any piece, out
    /// at once.
    #[test]
    fn with_no_way_to_her_door_she_steps_out_of_her_piece_first() {
        let unreachable = wall_door(35, 5);
        let piece = Rect::new(8, 11, 5, 5);
        for door in [Some(unreachable), None] {
            for (obstacles, step) in [(vec![piece], true), (Vec::new(), false)] {
                let at = format!("{door:?} {obstacles:?}");
                let stand = Act::Stand { until: 0 };
                let (mut osaka, terrain, mut rng) = on_a_school_day(10, 8, 20, stand);
                let chances = Chances {
                    door,
                    obstacles,
                    ..Chances::default()
                };
                osaka.decide(1000, &terrain, &chances, &mut rng);
                assert_eq!(
                    osaka.decisions.last().map(|d| d.method),
                    Some("routine/away"),
                    "{at}"
                );
                assert_eq!(osaka.set_off, Some(Routine::School), "{at}");
                if !step {
                    assert_eq!(osaka.through(), Some(Through::Space((10, 15))), "{at}");
                    assert_eq!(osaka.leaving, Some(Routine::School), "{at}");
                    continue;
                }
                // Box x−2..=x+2 clear of 8..=12: 5 or 15, the lower first.
                assert_eq!(
                    osaka.act,
                    Act::Walk {
                        to: 5,
                        then: Then::Job(Job::Out {
                            at: Clear::of((5, 15), &[piece]).unwrap(),
                            why: Leave::School
                        })
                    },
                    "{at}"
                );
                assert_eq!(osaka.leaving, None, "{at}");
                assert_eq!(osaka.census_purpose(), "routine", "{at}");
                assert!(osaka.on_her_way_out(), "{at}");
                let decided = osaka.decisions.len();
                let mut now = 1000;
                while osaka.through().is_none() {
                    assert!(now < 30_000, "{at}: never out: {:?}", osaka.act);
                    now = osaka.due().max(now + 1);
                    osaka.tick(now, Some(clock_at(1, 8, 20)), &terrain, &chances, &mut rng);
                }
                assert_eq!(osaka.through(), Some(Through::Space((5, 15))), "{at}");
                assert_eq!(osaka.leaving, Some(Routine::School), "{at}");
                assert_eq!(osaka.decisions.len(), decided, "{at}: nothing decided");
            }
        }
    }

    /// Her floor full of her pieces, with no way to her door (door batch,
    /// step 10a: the deep run's dash, out by a door in space on her TV):
    /// she makes for the nearest floor with room, by the drop off its
    /// end, and goes out there, never where her box meets a piece. With
    /// no room anywhere, she stays in, standing, and never goes out on a
    /// piece.
    #[test]
    fn with_her_floor_full_she_goes_out_from_another() {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::style::Style;
        // A shelf (row 9, columns 0..16) over the floor (row 15).
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        buf.set_string(0, 15, "─".repeat(40), Style::default());
        buf.set_string(0, 9, "─".repeat(16), Style::default());
        let terrain = Terrain::read(&buf, &[], false);
        let shelf = terrain.platform_at(6, 9).expect("the shelf");
        let floor = terrain.platform_at(30, 15).expect("the floor");
        assert!(route(&terrain, shelf, floor).is_some());
        // Her pieces fill the shelf; the floor has room at its far end.
        let full = vec![Rect::new(0, 5, 18, 5), Rect::new(0, 11, 30, 5)];
        let unreachable = wall_door(35, 5);
        for door in [Some(unreachable), None] {
            let at = format!("{door:?}");
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(6, 9, 0, &mut rng);
            osaka.act = Act::Stand { until: 0 };
            osaka.read_clock(Some(clock_at(1, 8, 20)), 0);
            let chances = Chances {
                door,
                obstacles: full.clone(),
                ..Chances::default()
            };
            osaka.decide(1000, &terrain, &chances, &mut rng);
            assert_eq!(osaka.set_off, Some(Routine::School), "{at}");
            assert!(
                osaka.through().is_none(),
                "{at}: not out on her pieces: {:?}",
                osaka.act
            );
            assert!(osaka.on_her_way_out(), "{at}: {:?}", osaka.act);
            let mut now = 1000;
            while osaka.through().is_none() {
                assert!(now < 60_000, "{at}: never out: {:?}", osaka.act);
                now = osaka.due().max(now + 1);
                osaka.tick(now, Some(clock_at(1, 8, 20)), &terrain, &chances, &mut rng);
            }
            let Some(Through::Space(spot)) = osaka.through() else {
                panic!("{at}: {:?}", osaka.through());
            };
            assert_eq!(spot.1, 15, "{at}: out from the floor");
            assert!(
                full.iter().all(|&piece| !box_meets(piece, spot)),
                "{at}: out at {spot:?}, on a piece"
            );
            assert_eq!(osaka.leaving, Some(Routine::School), "{at}");
        }
        // No room on any floor: she stays in, standing, again and again.
        let everywhere = vec![Rect::new(0, 0, 40, 16)];
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(6, 9, 0, &mut rng);
        osaka.act = Act::Stand { until: 0 };
        osaka.read_clock(Some(clock_at(1, 8, 20)), 0);
        let chances = Chances {
            obstacles: everywhere,
            ..Chances::default()
        };
        osaka.decide(1000, &terrain, &chances, &mut rng);
        let mut now = 1000;
        while now < 30_000 {
            assert!(osaka.through().is_none(), "out at {now}: {:?}", osaka.act);
            now = osaka.due().max(now + 1);
            osaka.tick(now, Some(clock_at(1, 8, 20)), &terrain, &chances, &mut rng);
        }
        assert!(matches!(osaka.act, Act::Stand { .. }), "{:?}", osaka.act);
    }

    /// Her way out by her door isn't something she lost (door batch D6):
    /// a Leave heading let go owes no beat, however it's let go; a walk
    /// to a use let go so does (the precondition).
    #[test]
    fn a_lost_way_out_owes_no_beat() {
        let mut rng = Rng(3);
        let leave = Job::Leave {
            spot: wall_door(30, 15),
            why: Leave::School,
        };
        let to_use = Job::Use(seat_for(Use::Watch, Furniture::Tv));
        for why in [Letting::Gone, Letting::Other] {
            for (job, owes) in [(leave.clone(), false), (to_use.clone(), true)] {
                let mut osaka = Osaka::standing_at(10, 15, 0, &mut rng);
                osaka.heading = Some(Heading {
                    want: Want::Walk,
                    job: job.clone(),
                });
                osaka.drop_heading(why);
                assert_eq!(!osaka.beats.is_empty(), owes, "{why:?} {job:?}");
                assert_eq!(!osaka.owed.is_empty(), owes, "{why:?} {job:?}");
            }
        }
    }

    /// Her set-off line is latched for one school time (door batch D6,
    /// the step 4a review): an errand or being placed lets it go (a new
    /// set-off says its line), and so does school ending: at her door
    /// at 12:45 she stays in, and on any decision once school's over
    /// (whatever cut her way out short), it's gone.
    #[test]
    fn her_set_off_is_let_go() {
        let terrain = floor_at(15);
        // An errand on her way.
        let (mut osaka, _, _) = on_a_school_day(10, 8, 20, leave_walk(30));
        osaka.set_off = Some(Routine::School);
        osaka.errand((5, 15), &terrain, 1000);
        assert_eq!(osaka.set_off, None, "the errand");
        // Placed on her way.
        let (mut osaka, _, _) = on_a_school_day(10, 8, 20, leave_walk(30));
        osaka.set_off = Some(Routine::School);
        osaka.place(12, 15, 1000);
        assert_eq!(osaka.set_off, None, "placed");
        // At her door as school ends.
        let (mut osaka, terrain, mut rng) = on_a_school_day(19, 12, 50, leave_walk(20));
        osaka.set_off = Some(Routine::School);
        let chances = Chances {
            door: Some(wall_door(20, 15)),
            ..Chances::default()
        };
        osaka.tick(
            1000,
            Some(clock_at(1, 12, 50)),
            &terrain,
            &chances,
            &mut rng,
        );
        assert_eq!((osaka.x, osaka.y), (20, 15));
        assert_eq!(osaka.through(), None, "she stays in");
        assert_eq!(osaka.set_off, None, "at her door");
        // Any decision once school's over (a chat look cut her walk).
        let stand = Act::Stand { until: 0 };
        let (mut osaka, terrain, mut rng) = on_a_school_day(10, 12, 50, stand);
        osaka.set_off = Some(Routine::School);
        osaka.decide(1000, &terrain, &Chances::default(), &mut rng);
        assert_eq!(osaka.set_off, None, "decided after school");
        // While school is on, a decision keeps it (a re-entry).
        let stand = Act::Stand { until: 0 };
        let (mut osaka, terrain, mut rng) = on_a_school_day(10, 8, 20, stand);
        osaka.set_off = Some(Routine::School);
        osaka.decide(1000, &terrain, &Chances::default(), &mut rng);
        assert_eq!(osaka.set_off, Some(Routine::School), "re-entered");
        assert!(osaka.said_lines().is_empty(), "silently");
    }

    /// She comes in facing the room (door batch C6): out of her door in
    /// a left wall she faces right, in a right wall left; coming home and
    /// dashing in alike. A guard (step 3 already faced her so).
    #[test]
    fn she_comes_in_facing_the_room() {
        use super::super::door::Set;
        use super::super::room::Side;
        let mut rng = Rng(5);
        // Her spot three in from its wall: a left wall at 7, a right at 13.
        for (side, wall, facing) in [
            (Side::Left, 7, Facing::Right),
            (Side::Right, 13, Facing::Left),
        ] {
            let door = DoorSpot::at(10, 12, Set::Wall { side, wall });
            let to = Through::Home(door);
            let back = Osaka::back_through_door(to, Routine::School, 0, &mut rng);
            let dash = Osaka::dash_in(to, true, 0, &mut rng);
            for osaka in [back, dash] {
                assert_eq!(osaka.facing, facing, "{side:?}");
                assert_eq!((osaka.x, osaka.y), (10, 12), "{side:?}");
                assert_eq!(osaka.through(), Some(to), "{side:?}");
            }
        }
    }

    /// Her door gone while she's through it (no door anywhere now), with
    /// a piece of hers laid where it stood (a resize): the door in space
    /// it becomes opens at the nearest place clear of her pieces, never
    /// on the piece, she with it in its beats (door batch, step 10a).
    #[test]
    fn her_door_gone_over_a_piece_opens_clear_of_it() {
        let terrain = floor_at(10);
        let door = wall_door(20, 10);
        let piece = Rect::new(16, 6, 8, 5);
        let to = Through::Home(door);
        assert_eq!(three_beats(to, 60_000).len(), 3);
        for (what, now) in three_beats(to, 60_000) {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
            osaka.act = Act::Door {
                since: 0,
                to,
                gap: 60_000,
            };
            let chances = Chances {
                obstacles: vec![piece],
                ..Chances::default()
            };
            osaka.take_in(&chances, &terrain, now);
            let Some(Through::Space(spot)) = osaka.through() else {
                panic!("{what}: {:?}", osaka.through());
            };
            assert!(!box_meets(piece, spot), "{what}: at {spot:?}, on her piece");
            assert_eq!(spot.1, 10, "{what}");
            // Her box x−2..=x+2 clear of 16..=23: 13 or 26, the nearer.
            assert_eq!(spot.0, 26, "{what}: the nearest clear column");
            // Drawn at her feet in its beats, where she comes in by it
            // out of sight: she goes with it, never left on the piece.
            assert_eq!((osaka.x, osaka.y), spot, "{what}: she goes with it");
        }
    }

    /// A door's stretches, as `(what, now)` from its `since` of 0 with
    /// `gap`: going out by it (its near beats, drawn at her feet), out of
    /// sight in its gap, and coming in by it (its far beats), each within
    /// a minute and a half of its gap's end (a gap that never ends has
    /// no coming in).
    fn three_beats(to: Through, gap: u64) -> Vec<(&'static str, u64)> {
        let first = |near: bool, drawn: bool| {
            let bound = gap.min(60_000) + 30_000;
            (0..bound).step_by(10).find(|&t| {
                door_beat(t, gap, to)
                    .is_some_and(|(b, _)| b.there != near && b.door.is_some() == drawn)
            })
        };
        [
            ("in its beats", first(true, true)),
            ("out of sight", first(true, false)),
            ("coming in", first(false, true)),
        ]
        .into_iter()
        .filter_map(|(what, t)| t.map(|t| (what, t + 50)))
        .collect()
    }

    /// A way out of hers in space (its gap above 0: school's, the
    /// stage's, her shift's) with a piece of hers laid over its spot
    /// since it opened (a resize, a delivery): it moves to the nearest
    /// place clear of her pieces, she with it, in every stretch of its
    /// beats it's drawn in (going out, and coming in: where she comes
    /// home; door batch, step 10a review). A door between floors (no
    /// gap) isn't a way out and keeps its spot.
    #[test]
    fn a_way_out_in_space_under_a_piece_moves_clear_of_it() {
        let terrain = floor_at(10);
        let piece = Rect::new(16, 6, 8, 5);
        let to = Through::Space((20, 10));
        for gap in [60_000, STAGE_GAP_MS, u64::MAX] {
            for (what, now) in three_beats(to, gap) {
                let what = format!("{what}, gap {gap}");
                let mut rng = Rng(5);
                let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
                osaka.act = Act::Door { since: 0, to, gap };
                let chances = Chances {
                    obstacles: vec![piece],
                    ..Chances::default()
                };
                osaka.take_in(&chances, &terrain, now);
                if what.starts_with("out of sight") {
                    // Nothing of it drawn: judged as its far door shows.
                    assert_eq!(osaka.through(), Some(to), "{what}");
                    continue;
                }
                assert_eq!(
                    osaka.through(),
                    Some(Through::Space((26, 10))),
                    "{what}: the nearest clear column"
                );
                assert_eq!((osaka.x, osaka.y), (26, 10), "{what}: she goes with it");
                // Judged again each frame: clear now, it stays.
                osaka.take_in(&chances, &terrain, now + 10);
                assert_eq!(osaka.through(), Some(Through::Space((26, 10))), "{what}");
            }
        }
        let mut rng = Rng(5);
        let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
        osaka.act = Act::Door {
            since: 0,
            to,
            gap: 0,
        };
        let chances = Chances {
            obstacles: vec![piece],
            ..Chances::default()
        };
        osaka.take_in(&chances, &terrain, 100);
        assert_eq!(osaka.through(), Some(to), "a door between floors");
    }

    /// Her door gone as she goes out by it (its near beats) with nowhere
    /// on any floor clear of her pieces: she isn't out after all (no way
    /// out opens on a piece, door batch step 10a review); what going out
    /// set is undone, and she stands a moment before her routine sends
    /// her again.
    #[test]
    fn her_door_gone_with_nowhere_clear_she_stays_in() {
        let terrain = floor_at(10);
        let door = wall_door(20, 10);
        let everywhere = vec![Rect::new(0, 0, 40, 11)];
        let (mut osaka, _, _) = on_a_school_day(20, 8, 20, Act::Stand { until: 0 });
        osaka.y = 10;
        osaka.through_her_door(door, Leave::School, 0);
        assert_eq!(osaka.leaving, Some(Routine::School));
        let chances = Chances {
            obstacles: everywhere,
            ..Chances::default()
        };
        osaka.take_in(&chances, &terrain, 100);
        assert_eq!(osaka.through(), None, "{:?}", osaka.act);
        assert_eq!(osaka.leaving, None);
        assert!(matches!(osaka.act, Act::Stand { .. }), "{:?}", osaka.act);
    }

    /// A focused pane over her door as she goes out by it for her shift
    /// (its gap): the door in space it becomes opens clear of the pane
    /// and of her pieces, never on one (door batch, step 10a). Wherever
    /// it might open, a piece of hers stands but at one column.
    #[test]
    fn a_shift_door_out_of_a_focused_pane_opens_clear_of_her_pieces() {
        let terrain = floor_at(15);
        let door = wall_door(20, 15);
        let focus = Rect::new(14, 10, 12, 6);
        // Clear of these only at column 4 (her box 2..=6).
        let pieces = [
            Rect::new(0, 11, 2, 5),
            Rect::new(7, 11, 7, 5),
            Rect::new(26, 11, 14, 5),
        ];
        for seed in 0..8 {
            let mut rng = Rng(seed);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.act = Act::Door {
                since: 0,
                to: Through::Home(door),
                gap: 60_000,
            };
            assert!(osaka.evict(focus, &terrain, &pieces, None, 100, &mut rng));
            assert_eq!(
                osaka.through(),
                Some(Through::Space((4, 15))),
                "seed {seed}"
            );
        }
    }

    /// Where it was clear to go out goes stale on her walk there (a
    /// resize or a delivery lays a piece over it): arrived, she never
    /// goes out on the piece; she goes on to a place still clear (door
    /// batch, step 10a review). Heading there across floors, the place is
    /// re-found only while it's still clear.
    #[test]
    fn where_it_was_clear_to_go_out_is_judged_again_on_arrival() {
        let piece = Rect::new(8, 11, 5, 5);
        let stand = Act::Stand { until: 0 };
        let (mut osaka, terrain, mut rng) = on_a_school_day(10, 8, 20, stand);
        let mut chances = Chances {
            obstacles: vec![piece],
            ..Chances::default()
        };
        osaka.decide(1000, &terrain, &chances, &mut rng);
        let Act::Walk {
            to: 5,
            then: Then::Job(Job::Out { at: clear, .. }),
        } = osaka.act
        else {
            panic!("out of the piece first: {:?}", osaka.act);
        };
        // Laid over where she's making for, before she's there.
        let laid = Rect::new(2, 11, 6, 5);
        chances.obstacles.push(laid);
        assert!(Clear::of(clear.spot(), &chances.obstacles).is_none());
        let mut now = 1000;
        while osaka.through().is_none() {
            assert!(now < 60_000, "never out: {:?}", osaka.act);
            now = osaka.due().max(now + 1);
            osaka.tick(now, Some(clock_at(1, 8, 20)), &terrain, &chances, &mut rng);
        }
        let Some(Through::Space(spot)) = osaka.through() else {
            panic!("{:?}", osaka.through());
        };
        assert!(
            chances.obstacles.iter().all(|&o| !box_meets(o, spot)),
            "out at {spot:?}, on a piece"
        );
        // Heading there: re-found only while it's still clear.
        let heading = Heading {
            want: Want::Walk,
            job: Job::Out {
                at: clear,
                why: Leave::School,
            },
        };
        let was = Chances {
            obstacles: vec![piece],
            ..Chances::default()
        };
        assert_eq!(heading.find(&was), Some(heading.job.clone()));
        assert_eq!(heading.find(&chances), None, "stale: let go");
    }

    /// A focused pane over her on her walk to her door for work, with her
    /// pieces clear at one column outside it (door batch, step 10a): the
    /// door in space she goes out to work by opens there, never on a
    /// piece, and her shift goes on.
    #[test]
    fn evicted_on_her_way_to_work_she_goes_out_clear_of_her_pieces() {
        let terrain = floor_at(15);
        let door = wall_door(36, 15);
        // Clear of these only at column 30 (her box 28..=32).
        let pieces = [Rect::new(0, 11, 28, 5), Rect::new(33, 11, 7, 5)];
        for seed in 0..8 {
            let chances = door_chances(door);
            let mut rng = Rng(seed);
            let mut osaka = Osaka::standing_at(3, 15, 0, &mut rng);
            osaka.go_to_work(&terrain, &chances, 0, &mut rng);
            let now = tick_with(&mut osaka, 0, &terrain, &chances, &mut rng, 30_000, |o| {
                o.x >= 10
            });
            assert!(osaka.work_walk(), "seed {seed}: {:?}", osaka.act);
            let focus = Rect::new(osaka.x as u16 - 3, 10, 7, 6);
            assert!(osaka.evict(focus, &terrain, &pieces, None, now, &mut rng));
            assert_eq!(
                osaka.through(),
                Some(Through::Space((30, 15))),
                "seed {seed}"
            );
            assert_eq!(osaka.shift, Some(Shift::Out), "seed {seed}");
        }
    }

    /// Evicted out of a focused pane on her way out (to work, or through
    /// her door for it), with nowhere clear of the pane and her pieces
    /// both: she isn't out after all (no way out opens on a piece; door
    /// batch, step 10a review), her shift let go, and she's moved out of
    /// the pane by a door between floors (no gap), never home from work
    /// for it. The visit goes on.
    #[test]
    fn evicted_with_nowhere_clear_of_her_pieces_she_stays_in() {
        let terrain = floor_at(15);
        let everywhere = [Rect::new(0, 0, 40, 16)];
        // On her walk to her door for work.
        let door = wall_door(36, 15);
        let chances = door_chances(door);
        let mut rng = Rng(4);
        let mut osaka = Osaka::standing_at(3, 15, 0, &mut rng);
        osaka.go_to_work(&terrain, &chances, 0, &mut rng);
        let now = tick_with(&mut osaka, 0, &terrain, &chances, &mut rng, 30_000, |o| {
            o.x >= 10
        });
        let focus = Rect::new(osaka.x as u16 - 3, 10, 7, 6);
        assert!(osaka.evict(focus, &terrain, &everywhere, None, now, &mut rng));
        assert_eq!(osaka.shift, None, "her shift let go");
        let Act::Door {
            to: Through::Space(spot),
            gap: 0,
            ..
        } = osaka.act
        else {
            panic!("out of the pane, no way out: {:?}", osaka.act);
        };
        assert!(!box_meets(focus, spot));
        // Through her door for it, its beats begun.
        let door = wall_door(20, 15);
        let chances = door_chances(door);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.go_to_work(&terrain, &chances, 0, &mut rng);
        assert_eq!(osaka.shift, Some(Shift::Out));
        osaka.tick(300, None, &terrain, &chances, &mut rng);
        let focus = Rect::new(17, 10, 7, 6);
        assert!(osaka.evict(focus, &terrain, &everywhere, None, 300, &mut rng));
        assert_eq!(osaka.shift, None, "her shift let go");
        assert!(
            matches!(osaka.act, Act::Door { to: Through::Space(spot), gap: 0, .. } if !box_meets(focus, spot)),
            "{:?}",
            osaka.act
        );
        let mut now = 300;
        while matches!(osaka.act, Act::Door { .. }) {
            assert!(now < 30_000, "{:?}", osaka.act);
            now += 100;
            osaka.tick(now, None, &terrain, &chances, &mut rng);
        }
        assert_ne!(
            osaka.speech.map(|(line, _)| line),
            Some(HOME),
            "not home from work"
        );
    }

    /// Where she goes to go out with her own spot covered
    /// ([`nearest_clear`]): her floor first, however far; then a floor
    /// she can get to; a floor she can't get to only after those,
    /// however near (door batch, step 10a review).
    #[test]
    fn the_nearest_clear_place_keeps_to_floors_she_can_reach() {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::style::Style;
        // A shelf (row 9, 0..16) over the floor (row 15, 0..40), and a
        // ledge high up (row 5, 20..30) with nothing to climb to it.
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        buf.set_string(0, 15, "─".repeat(40), Style::default());
        buf.set_string(0, 9, "─".repeat(16), Style::default());
        buf.set_string(20, 5, "─".repeat(10), Style::default());
        let terrain = Terrain::read(&buf, &[], false);
        let shelf = terrain.platform_at(6, 9).expect("the shelf");
        let floor = terrain.platform_at(30, 15).expect("the floor");
        let ledge = terrain.platform_at(25, 5).expect("the ledge");
        assert!(route(&terrain, shelf, floor).is_some());
        assert!(route(&terrain, shelf, ledge).is_none());
        // The shelf full, the floor clear only at its far end (36), the
        // ledge nearer (a walk of 14 + 4 against 30 + 6): the floor.
        let shelf_full = vec![Rect::new(0, 5, 18, 5), Rect::new(0, 11, 34, 5)];
        let to = nearest_clear(&terrain, Some(shelf), (6, 9), &shelf_full);
        assert_eq!(
            to.map(Clear::spot),
            Some((36, 15)),
            "a floor she can get to"
        );
        // Driven: she makes for it and goes out there.
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(6, 9, 0, &mut rng);
        osaka.act = Act::Stand { until: 0 };
        osaka.read_clock(Some(clock_at(1, 8, 20)), 0);
        let chances = Chances {
            obstacles: shelf_full,
            ..Chances::default()
        };
        osaka.decide(1000, &terrain, &chances, &mut rng);
        let mut now = 1000;
        while osaka.through().is_none() {
            assert!(now < 60_000, "never out: {:?}", osaka.act);
            now = osaka.due().max(now + 1);
            osaka.tick(now, Some(clock_at(1, 8, 20)), &terrain, &chances, &mut rng);
        }
        assert_eq!(osaka.through(), Some(Through::Space((36, 15))));
        // On the floor, clear only far along it (36), the shelf clear
        // right above her: her own floor.
        let floor_far = vec![Rect::new(0, 11, 34, 5)];
        let to = nearest_clear(&terrain, Some(floor), (6, 15), &floor_far);
        assert_eq!(to.map(Clear::spot), Some((36, 15)), "her own floor first");
    }

    /// Off to work with nowhere clear of her pieces on any floor and no
    /// door of hers (door batch, step 10a): work can wait (her shift let
    /// go), she stands, and never goes out on a piece. The stage's scene
    /// likewise never sends her out (nothing of it set).
    #[test]
    fn with_nowhere_clear_work_can_wait() {
        let terrain = floor_at(15);
        let chances = Chances {
            obstacles: vec![Rect::new(0, 0, 40, 16)],
            ..Chances::default()
        };
        let mut rng = Rng(4);
        let mut osaka = Osaka::standing_at(10, 15, 0, &mut rng);
        osaka.go_to_work(&terrain, &chances, 0, &mut rng);
        assert!(matches!(osaka.act, Act::Stand { .. }), "{:?}", osaka.act);
        assert_eq!(osaka.shift, None, "her shift let go");
        assert!(!osaka.work_way());
        let mut now = 0;
        while now < 30_000 {
            assert!(osaka.through().is_none(), "out at {now}: {:?}", osaka.act);
            now += 100;
            osaka.tick(now, None, &terrain, &chances, &mut rng);
        }
        let mut osaka = Osaka::standing_at(10, 15, 0, &mut rng);
        osaka.out_where_clear(Leave::Stage, &terrain, &chances, 0);
        assert!(matches!(osaka.act, Act::Stand { .. }), "{:?}", osaka.act);
        assert_eq!((osaka.returning, osaka.leaving), (None, None));
    }

    /// Standing on no floor of hers (a ledge she was put on, a hop's
    /// landing gone) inside a piece, set to go out: she goes by a door
    /// between floors (no gap) to the nearest place clear of her pieces,
    /// and out there (door batch, step 10a review), never standing put
    /// as if there were nowhere clear.
    #[test]
    fn off_any_floor_she_goes_out_where_it_is_clear() {
        let terrain = floor_at(15);
        let piece = Rect::new(4, 7, 14, 4);
        let chances = Chances {
            obstacles: vec![piece],
            ..Chances::default()
        };
        let (mut osaka, _, mut rng) = on_a_school_day(10, 8, 20, Act::Stand { until: 0 });
        osaka.y = 12;
        assert_eq!(terrain.platform_at(10, 12), None);
        osaka.out_where_clear(Leave::School, &terrain, &chances, 1000);
        assert!(
            matches!(
                osaka.act,
                Act::Door {
                    to: Through::Space((10, 15)),
                    gap: 0,
                    ..
                }
            ),
            "{:?}",
            osaka.act
        );
        let mut now = 1000;
        while osaka.leaving.is_none() {
            assert!(now < 60_000, "never out: {:?}", osaka.act);
            now = osaka.due().max(now + 1);
            osaka.tick(now, Some(clock_at(1, 8, 20)), &terrain, &chances, &mut rng);
        }
        let Some(Through::Space(spot)) = osaka.through() else {
            panic!("{:?}", osaka.through());
        };
        assert!(!box_meets(piece, spot), "out at {spot:?}, on a piece");
    }

    /// Her external door opened, not yet let her out (door batch, step
    /// 4a, C5): with no door anywhere now, it's a door in space at her
    /// feet; moved, it follows (`Chances::door_through`: the frame's door
    /// read without the focused pane, so a focus never moves it; see
    /// door.rs `a_focused_pane_never_moves_the_door_read_ungated`); the
    /// frame's own door is not what it reads. It's drawn at her feet, so
    /// in every beat she moves with it (step 6d): facing out on its near
    /// side, into the room (and bumped, once, by a door pushed out of its
    /// space) at its far side; its last beat over, it's left. The frame
    /// calls it directly as it reads her door, as her tick does through
    /// `take_in`.
    #[test]
    fn her_door_mid_gap_follows_the_frame() {
        use super::super::door::{Fallback, Set};
        let mut rng = Rng(5);
        let open = |osaka: &mut Osaka, door: DoorSpot| {
            osaka.act = Act::Door {
                since: 0,
                to: Through::Home(door),
                gap: 5_000,
            };
        };
        let door = wall_door(20, 10);
        let moved = wall_door(30, 10);
        let elsewhere = DoorSpot::at(25, 10, Set::Floor(Fallback::Protected));
        let mid = through_ms(Through::Home(door)) + 100;
        for (through, want) in [
            (None, Through::Space((20, 10))),
            (Some(moved), Through::Home(moved)),
            (Some(door), Through::Home(door)),
        ] {
            let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
            open(&mut osaka, door);
            assert_eq!(osaka.opened_door(mid), Some(door));
            let chances = Chances {
                door: Some(elsewhere),
                door_through: through,
                ..Chances::default()
            };
            osaka.take_in(&chances, &Terrain::default(), mid);
            assert_eq!(osaka.through(), Some(want), "{through:?}");
            // Out of sight in the gap, her feet go with it.
            assert_eq!((osaka.x, osaka.y), want.spot(), "{through:?}");
        }
        // On its near side, the door drawn at her feet as it opens (her
        // in its doorway, then gone through it): she stands at it where
        // it stands now, facing out of the room by it, never bumped (the
        // frame calls it directly, as it reads her door).
        let pushed = DoorSpot::at(32, 10, Set::Floor(Fallback::Yield));
        assert!(pushed.bumped());
        for near in [100, 1_700] {
            assert!(
                door_beat(near, 5_000, Through::Home(door))
                    .is_some_and(|(b, _)| !b.there && b.door.is_some())
            );
            for fresh in [moved, pushed] {
                let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
                open(&mut osaka, door);
                osaka.facing = door.out();
                assert_eq!(osaka.opened_door(near), Some(door));
                osaka.door_follows(Some(fresh), Some(fresh), (&Terrain::default(), &[]), near);
                assert_eq!(osaka.through(), Some(Through::Home(fresh)), "{near}");
                assert_eq!((osaka.x, osaka.y), fresh.spot(), "{near}");
                assert_eq!(osaka.facing, fresh.out(), "{near}");
                assert!(!osaka.bumped(), "{near}");
            }
        }
        // Opened beside a focused pane, where the frame stands it with
        // the focus: read without the focus it would stand in its space,
        // under the pane, but the focus never moves it (step 6d: the
        // frame follows the door the same frame it opens).
        let beside = DoorSpot::at(14, 10, Set::Floor(Fallback::Protected));
        let mut osaka = Osaka::standing_at(14, 10, 0, &mut rng);
        open(&mut osaka, beside);
        let chances = Chances {
            door: Some(beside),
            door_through: Some(door),
            ..Chances::default()
        };
        osaka.take_in(&chances, &Terrain::default(), mid);
        assert_eq!(osaka.through(), Some(Through::Home(beside)));
        // At its far side already, coming in by it: she's in its
        // doorway, so she moves with it (door batch, step 6d); with no
        // door now, it's a door in space where she stands.
        // Both with her out of sight behind it (its first far beats) and
        // with her in its doorway.
        for there in [
            through_ms(Through::Home(door)) + 5_000 + 10,
            through_ms(Through::Home(door)) + 5_000 + 1_100,
        ] {
            let her = door_beat(there, 5_000, Through::Home(door)).map(|(b, _)| (b.there, b.her));
            assert!(matches!(her, Some((true, _))), "{there}");
            for (through, want, feet) in [
                (Some(moved), Through::Home(moved), (30, 10)),
                (Some(door), Through::Home(door), (20, 10)),
                (None, Through::Space((20, 10)), (20, 10)),
            ] {
                let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
                open(&mut osaka, door);
                assert_eq!(osaka.opened_door(there), Some(door));
                let chances = Chances {
                    door: through,
                    door_through: through,
                    ..Chances::default()
                };
                osaka.take_in(&chances, &Terrain::default(), there);
                assert_eq!(osaka.through(), Some(want), "{her:?}, {through:?}");
                assert_eq!((osaka.x, osaka.y), feet, "{her:?}, {through:?}");
                if want == Through::Home(moved) {
                    assert_eq!(osaka.facing, want.into_room(), "{her:?}, {through:?}");
                }
                assert!(!osaka.bumped(), "{her:?}, {through:?}");
            }
            // Moved to a spot her pieces pushed it to: she bumped into
            // what fills its space, once; moved on again, still once.
            let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
            open(&mut osaka, door);
            osaka.door_follows(
                Some(pushed),
                Some(pushed),
                (&Terrain::default(), &[]),
                there,
            );
            assert_eq!(osaka.through(), Some(Through::Home(pushed)), "{her:?}");
            assert_eq!((osaka.x, osaka.y), pushed.spot(), "{her:?}");
            assert!(osaka.bumped(), "{her:?}");
            let further = DoorSpot::at(34, 10, Set::Floor(Fallback::Yield));
            osaka.door_follows(
                Some(further),
                Some(further),
                (&Terrain::default(), &[]),
                there,
            );
            assert_eq!((osaka.x, osaka.y), further.spot(), "{her:?}");
            assert!(osaka.bumped(), "{her:?}");
        }
        // Its last beat over: nothing to follow.
        let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
        open(&mut osaka, door);
        let done = through_ms(Through::Home(door)) + 5_000 + 60_000;
        let chances = Chances {
            door: Some(moved),
            door_through: Some(moved),
            ..Chances::default()
        };
        osaka.take_in(&chances, &Terrain::default(), done);
        assert_eq!(osaka.through(), Some(Through::Home(door)));
    }

    /// Her external door opened, not leaving by her routine (the stage's
    /// school scene, work's), not yet let her out, its far side in a
    /// focused pane: going through it, it opens elsewhere, a door in
    /// space, keeping its gap (door batch M10, F21). Out of sight between
    /// its doors (its gap), the focus is no reason to move it (door
    /// batch, step 4b: nothing of hers or it is drawn; she's moved on as
    /// she comes back out, if it's still focused then).
    #[test]
    fn an_opened_door_evicted_keeps_its_gap() {
        let terrain = floor_at(15);
        let door = wall_door(20, 15);
        let focus = Rect::new(15, 8, 12, 9);
        for (now, moved) in [(100, true), (through_ms(Through::Home(door)) + 100, false)] {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.act = Act::Door {
                since: 0,
                to: Through::Home(door),
                gap: 5_000,
            };
            assert!(osaka.evict(focus, &terrain, &[], None, now, &mut rng));
            let Act::Door { to, gap, .. } = osaka.act else {
                panic!("{:?}", osaka.act);
            };
            if moved {
                assert!(matches!(to, Through::Space(_)), "{to:?}");
                assert!(!box_meets(focus, to.spot()), "{to:?}");
            } else {
                assert_eq!(to, Through::Home(door), "in its gap");
            }
            assert_eq!(gap, 5_000);
        }
    }

    /// Her, at `act` (its next pose change at `act_due`), with her clock
    /// at Monday 22:00 from monotonic 0.
    fn at_bedtime(act: Act, act_due: u64) -> (Osaka, Rng) {
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        osaka.act = act;
        osaka.act_due = act_due;
        osaka.read_clock(Some(monday_at(22, 0)), 0);
        (osaka, rng)
    }

    /// What she's resting at or busy with, her routine cuts at a cutting
    /// boundary (bedtime): no startle, and she decides again at once;
    /// the next boundary (school) is due next.
    #[test]
    fn her_routine_cuts_what_she_rests_at_without_a_startle() {
        let far = 10 * BED;
        let watch = Act::Use {
            seat: seat_for(Use::Watch, Furniture::Tv),
            since: 0,
            until: far,
            whole: far,
            play: Play::of(Use::Watch, None),
            grievance: None,
        };
        let acts = [
            watch.clone(),
            Act::Stand { until: far },
            Act::Idle {
                what: Activity::Sit,
                since: 0,
                until: far,
                play: None,
            },
            Act::SpaceOut {
                since: 0,
                until: far,
                play: None,
                session: None,
            },
        ];
        for act in acts {
            let (mut osaka, _) = at_bedtime(act.clone(), far);
            assert_eq!(osaka.cut_at, Some(BED), "{act:?}");
            assert!(osaka.cut(BED), "{act:?}");
            assert_eq!(
                osaka.act,
                Act::Look {
                    surprised_until: BED,
                    until: BED,
                },
                "{act:?}: no startle"
            );
            assert_eq!(osaka.cut_at, Some(SCHOOL), "{act:?}");
        }
        // Through a tick: the cut comes first, at the boundary itself,
        // however late the tick (a catch-up), and she decides there.
        let (mut osaka, mut rng) = at_bedtime(watch, far);
        let terrain = blank_terrain();
        let late = BED + 7_000;
        assert!(osaka.tick(
            late,
            Some(monday_at(22, 0)),
            &terrain,
            &Chances::default(),
            &mut rng
        ));
        assert!(!matches!(osaka.act, Act::Use { .. }), "{:?}", osaka.act);
        assert_eq!(osaka.decisions.first().map(|d| d.at), Some(BED));
        assert_eq!(osaka.cut_at, Some(SCHOOL));
        assert!(osaka.due() > late);
    }

    /// A use ending at bedtime itself: the boundary is handled first (it
    /// cuts the use), and she decides once. Were the use's end fired
    /// first, she'd decide, and what she began at the boundary would be
    /// cut by it too: a second decision.
    #[test]
    fn a_cut_due_with_her_act_comes_first() {
        let watch = Act::Use {
            seat: seat_for(Use::Watch, Furniture::Tv),
            since: 0,
            until: BED,
            whole: BED,
            play: Play::of(Use::Watch, None),
            grievance: None,
        };
        let (mut osaka, mut rng) = at_bedtime(watch, BED);
        assert_eq!(osaka.cut_at, Some(BED));
        let terrain = blank_terrain();
        osaka.tick(
            BED,
            Some(monday_at(22, 0)),
            &terrain,
            &Chances::default(),
            &mut rng,
        );
        assert_eq!(osaka.decisions.len(), 1, "{:?}", osaka.decisions);
        assert_eq!(osaka.cut_at, Some(SCHOOL));
    }

    /// What runs its course on its own (a climb, a fall, a door, being
    /// out, her errand's poke) her routine leaves alone: crossing the
    /// boundary, nothing of hers is stepped (no fire, no draw), the
    /// boundary is handled all the same (the next is due, so the tick
    /// doesn't spin on it), and a tick finds nothing to do. (Each is
    /// spared by `Act::props`'s table, which has her passing through
    /// them: an act moved to one she stays at fails here.)
    #[test]
    fn her_routine_leaves_what_runs_its_course() {
        let step = BED + 50;
        let acts = [
            Act::Climb { to_y: 4 },
            Act::Fall {
                from_y: 4,
                since: BED - 100,
                to_y: 10,
            },
            Act::Door {
                since: BED - 100,
                to: Through::Space((20, 10)),
                gap: 5_000,
            },
            // Her external door (door batch, step 4a), and her walk to
            // it: a boundary never cuts her way out.
            Act::Door {
                since: BED - 100,
                to: Through::Home(wall_door(10, 10)),
                gap: u64::MAX,
            },
            Act::Walk {
                to: 20,
                then: Then::Job(Job::Leave {
                    spot: wall_door(20, 10),
                    why: Leave::School,
                }),
            },
            Act::Poke {
                since: BED - 100,
                until: BED + 2_000,
            },
            Act::Out {
                to: 0,
                enter: 30,
                to_y: 10,
                to_x: 30,
            },
            Act::Away {
                until: BED + 60_000,
                enter: 30,
                to_y: 10,
                to_x: 30,
            },
        ];
        let terrain = blank_terrain();
        // Her door where the frame stands it (where she is).
        let chances = Chances {
            door: Some(wall_door(10, 10)),
            door_through: Some(wall_door(10, 10)),
            ..Chances::default()
        };
        for act in acts {
            let (mut osaka, mut rng) = at_bedtime(act.clone(), step);
            let before = rng.0;
            assert_eq!(osaka.cut_at, Some(BED), "{act:?}");
            let changed = osaka.tick(BED, Some(monday_at(22, 0)), &terrain, &chances, &mut rng);
            assert!(!changed, "{act:?}");
            assert_eq!(osaka.act, act, "not cut, not stepped");
            assert_eq!(osaka.act_due, step, "{act:?}");
            assert_eq!(rng.0, before, "{act:?}: nothing drawn");
            assert!(osaka.decisions.is_empty(), "{act:?}");
            assert_eq!(osaka.cut_at, Some(SCHOOL), "{act:?}");
            assert_eq!(osaka.due(), step, "{act:?}");
            // Again at the same moment, and as the clock is read afresh
            // (the next tick's entry): the boundary handled stays handled.
            assert!(!osaka.tick(BED, Some(monday_at(22, 0)), &terrain, &chances, &mut rng));
            assert_eq!(osaka.cut_at, Some(SCHOOL), "{act:?}");
        }
    }

    /// Without her routine, nothing is ever cut: no boundary is due.
    #[test]
    fn unfed_nothing_is_cut() {
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        osaka.read_clock(None, 0);
        assert_eq!(osaka.cut_at, None);
        let unfed = osaka.clone();
        osaka.read_clock(Some(monday_at(22, 0)), 0);
        assert_eq!(osaka.cut_at, Some(BED));
        osaka.read_clock(None, 0);
        assert_eq!(osaka.cut_at, None);
        assert_eq!(osaka.due(), unfed.due());
    }

    /// An act begun after a boundary is cut only at the next: the
    /// search starts from her act's start.
    #[test]
    fn a_cut_is_the_first_boundary_after_her_act_began() {
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(10, 10, BED + 1_000, &mut rng);
        osaka.read_clock(Some(monday_at(22, 0)), 0);
        assert_eq!(osaka.cut_at, Some(SCHOOL));
        // Begun before: bedtime.
        let mut osaka = Osaka::standing_at(10, 10, BED - 1, &mut rng);
        osaka.read_clock(Some(monday_at(22, 0)), 0);
        assert_eq!(osaka.cut_at, Some(BED));
        // A new act after it moves the search on.
        osaka.set(Act::Stand { until: BED + 5_000 }, BED + 1);
        osaka.read_clock(Some(monday_at(22, 0)), 0);
        assert_eq!(osaka.cut_at, Some(SCHOOL));
    }

    /// A boundary handled stays handled when her clock is read afresh
    /// after a step the shell's cap clamps (a suspend: `Guest::accrue`
    /// counts at most the cap, so the new reading puts every earlier
    /// monotonic moment further back in her day than it was): the act
    /// begun after bedtime isn't cut again at a bedtime found anew, and
    /// school is still her next cut.
    #[test]
    fn a_capped_step_never_brings_a_handled_boundary_back() {
        use super::super::{CLOCK_SPEED, GameClock};
        let far = 100 * SCHOOL;
        let (mut osaka, mut rng) = at_bedtime(Act::Stand { until: far }, far);
        let terrain = blank_terrain();
        let before = monday_at(22, 0);
        osaka.tick(BED, Some(before), &terrain, &Chances::default(), &mut rng);
        assert_eq!(osaka.decisions.len(), 1, "cut at bedtime");
        assert_eq!(osaka.cut_at, Some(SCHOOL));
        // Resting, begun after bedtime.
        osaka.set(Act::Stand { until: far }, BED + 30_000);
        let decided = osaka.decisions.len();
        // Ticked last a minute after bedtime; then an hour's suspend,
        // counted as the ten minutes the shell caps it at.
        let (last, cap) = (BED + 60_000, 600_000);
        let now = last + 3_600_000;
        let game = GameClock {
            at: now,
            game: before.game.at(last) + CLOCK_SPEED * cap,
        };
        let after = routine::Clock::read(game, now, None, None);
        assert_eq!(after.day(now).to_string(), "Mon 23:36 Asleep");
        // The hour's blinks may hold the tick's catch-up back (it gives
        // up after a few dozen events): the next tick, a moment on, goes
        // on from there.
        for now in [now, now + 1_000] {
            osaka.tick(now, Some(after), &terrain, &Chances::default(), &mut rng);
        }
        assert_eq!(osaka.decisions.len(), decided, "not cut again");
        assert_eq!(osaka.act, Act::Stand { until: far });
        let school = before.game.at(SCHOOL);
        assert_eq!(osaka.cut_at, Some(game.when(school)));
        assert!(osaka.due() > now + 1_000);
    }

    /// At homework time, homework lasts 2-4 real minutes instead of 30-60
    /// seconds: the same one draw over the longer range (her stream is
    /// left exactly as unfed), and the whole use is its length, so its
    /// credit is the share truly done. Out of homework time, or another
    /// use, as ever.
    #[test]
    fn homework_lasts_longer_at_homework_time() {
        let length = |what: Use, clock: Option<routine::Clock>, seed: u64| {
            let mut rng = Rng(seed);
            let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
            osaka.splice_rows = &[];
            osaka.whims = Whims(seed ^ 0x5eed);
            osaka.credit = Some(Want::Use(what));
            osaka.read_clock(clock, 0);
            let item = Furniture::ALL
                .into_iter()
                .find(|f| f.spec().uses.contains(&what))
                .unwrap_or(Furniture::Sofa);
            osaka.start_job(
                Job::Use(seat_for(what, item)),
                1000,
                &Chances::default(),
                &mut rng,
            );
            let (since, until, whole, play) = begun(&osaka);
            assert_eq!((play.before, play.after), (None, None));
            assert_eq!(whole, until - since, "the whole is its length");
            (until - since, rng.0)
        };
        let homework_time = Some(monday_at(21, 0));
        let afternoon = Some(monday_at(16, 0));
        let usual = use_duration(Use::Homework);
        assert!(usual.1 < HOMEWORK_IN_SLOT_MS.0, "longer at its time");
        for seed in 0..64 {
            let (unfed, drawn) = length(Use::Homework, None, seed);
            let (fed, fed_drawn) = length(Use::Homework, homework_time, seed);
            assert!((usual.0..usual.1).contains(&unfed), "{unfed}");
            assert!(
                (HOMEWORK_IN_SLOT_MS.0..HOMEWORK_IN_SLOT_MS.1).contains(&fed),
                "{fed}"
            );
            assert_eq!(fed_drawn, drawn, "seed {seed}: the same draws");
            assert_eq!(length(Use::Homework, afternoon, seed), (unfed, drawn));
            for what in [Use::Read, Use::Watch, Use::Lounge] {
                assert_eq!(length(what, homework_time, seed), length(what, None, seed));
            }
            // Her homework on the floor (phase 5c D5), as long as at her
            // desk, at its times.
            for (clock, (lo, hi)) in [
                (None, usual),
                (homework_time, HOMEWORK_IN_SLOT_MS),
                (afternoon, usual),
            ] {
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.read_clock(clock, 0);
                osaka.floor_homework(seed % 2 == 0, 1000, &mut rng);
                let Act::Idle { since, until, .. } = osaka.act else {
                    panic!("homework on the floor: {:?}", osaka.act);
                };
                assert!(
                    (lo..hi).contains(&(until - since)),
                    "seed {seed} {clock:?}: {}",
                    until - since
                );
            }
        }
    }

    /// Her still acts last as phase 5c shipped them (step 8c): what she
    /// settles into lasts as what it grew from (sitting under her window
    /// as long as she'd sit, dozing sitting up, under a book or watching
    /// the clouds as long as she'd lie back), her reading and homework
    /// on the floor as at her bookshelf and desk, and each at its
    /// shipped range. Not her sill's lean: that's a use, its length its
    /// own (`looking_out_lasts_and_counts_as_spacing_out`).
    #[test]
    fn her_still_acts_last_as_shipped() {
        let settled_from = [
            (Activity::UnderSill, Activity::Sit),
            (Activity::SitDoze, Activity::LieBack),
            (Activity::BookDoze, Activity::LieBack),
            (Activity::CloudWatch, Activity::LieBack),
        ];
        for (settled, from) in settled_from {
            assert!(!settled.chosen(), "{settled:?} is settled into");
            assert_eq!(
                settled.duration(),
                from.duration(),
                "{settled:?} as {from:?}"
            );
        }
        assert_eq!(
            Activity::FloorHomework.duration(),
            use_duration(Use::Homework)
        );
        assert_eq!(Activity::LieRead.duration(), use_duration(Use::Read));
        let shipped = |what: Activity| match what {
            Activity::Sit | Activity::UnderSill => (15_000, 40_000),
            Activity::LieBack | Activity::SitDoze | Activity::BookDoze | Activity::CloudWatch => {
                (20_000, 60_000)
            }
            Activity::LieFront => (10_000, 25_000),
            Activity::Jacks => (4_000, 8_000),
            Activity::ToeTouch => (5_000, 9_000),
            Activity::Stretch => (2_000, 4_000),
            Activity::Gaze => (6_000, 14_000),
            Activity::FloorHomework => (37_000, 75_000),
            Activity::LieRead => (30_000, 65_000),
        };
        for what in Activity::ALL {
            assert_eq!(what.duration(), shipped(what), "{what:?}");
        }
        assert_eq!(SPACE_OUT_MS, (10_000, 28_000), "spacing out");
        for (what, range) in [
            (Use::Lounge, (27_000, 60_000)),
            (Use::Nap, (45_000, 105_000)),
            (Use::Watch, (32_000, 82_000)),
            (Use::Read, (30_000, 65_000)),
            (Use::Homework, (37_000, 75_000)),
            (Use::LookOut, (60_000, 180_000)),
        ] {
            assert_eq!(use_duration(what), range, "{what:?}");
        }
    }

    /// Her night's sleep, as she leaves it, is kept for her needs (A11):
    /// counted once, though it's settled twice (by her decision, then by
    /// the act that follows), and taken by her needs at that decision.
    /// Any other act keeps nothing.
    #[test]
    fn a_nights_sleep_is_counted_once() {
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        let lie_back = |play| Act::Idle {
            what: Activity::LieBack,
            since: 1_000,
            until: 90_000,
            play,
        };
        osaka.set(lie_back(None), 1_000);
        osaka.credit_done(50_000);
        assert_eq!(osaka.slept_ms, 0, "not the night act");
        let night = Play {
            branch: Surface::Floor.branch(false),
            ..Play::plain(ScriptId::Night)
        };
        osaka.set(lie_back(Some(night)), 1_000);
        osaka.credit_done(61_000);
        osaka.credit_done(61_000);
        assert_eq!(osaka.slept_counted(), 60_000);
        osaka.set(Act::Stand { until: 70_000 }, 61_000);
        assert_eq!(osaka.slept_counted(), 60_000, "counted once");
        // Her decision takes it: sleepiness rose only for the time awake.
        let terrain = {
            use tuirealm::ratatui::buffer::Buffer;
            use tuirealm::ratatui::layout::Rect;
            use tuirealm::ratatui::style::Style;
            let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
            buf.set_string(0, 15, "─".repeat(40), Style::default());
            Terrain::read(&buf, &[], false)
        };
        let floor = terrain.platforms.first().cloned().expect("a floor");
        osaka.x = (floor.x0 + floor.x1) / 2;
        osaka.y = floor.y;
        osaka.decided = 1_000;
        let sleepy = osaka.needs.get(Need::Sleepy);
        osaka.decide(61_000, &terrain, &Chances::default(), &mut rng);
        let method = osaka.decisions.last().map(|d| d.method);
        assert!(
            !matches!(method, Some("no floor" | "off text")),
            "{method:?}"
        );
        assert_eq!(osaka.slept_ms, 0, "taken");
        assert_eq!(osaka.needs.get(Need::Sleepy), sleepy, "asleep throughout");
    }

    /// Her needs rise by her day as she decides (D4 lever 2, through the
    /// decision's own reading of her clock): a minute ending at
    /// homework time, sleepiness three times as fast as unfed; one
    /// ending at (Tuesday's) breakfast, hunger twice as fast; in the
    /// afternoon, as unfed.
    #[test]
    fn her_decisions_raise_her_needs_by_her_day() {
        let terrain = {
            use tuirealm::ratatui::buffer::Buffer;
            use tuirealm::ratatui::layout::Rect;
            use tuirealm::ratatui::style::Style;
            let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
            buf.set_string(0, 15, "─".repeat(40), Style::default());
            Terrain::read(&buf, &[], false)
        };
        let floor = terrain.platforms.first().cloned().expect("a floor");
        let rose = |clock: Option<routine::Clock>, need: Need| {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at((floor.x0 + floor.x1) / 2, floor.y, 0, &mut rng);
            osaka.read_clock(clock, 0);
            osaka.decided = 0;
            let before = osaka.needs.get(need);
            osaka.decide(60_000, &terrain, &Chances::default(), &mut rng);
            let after = osaka.needs.get(need);
            assert!(after < 1.0, "{need:?} saturated");
            after - before
        };
        for (clock, need, times) in [
            (monday_at(21, 0), Need::Sleepy, 3.0),
            (clock_at(1, 7, 30), Need::Hungry, 2.0),
            (monday_at(16, 0), Need::Hungry, 1.0),
        ] {
            let unfed = rose(None, need);
            assert!(unfed > 0.0, "{need:?}");
            let fed = rose(Some(clock), need);
            assert!(
                (fed - unfed * times).abs() < 1e-9,
                "{need:?} at {}: {fed} vs {unfed}",
                clock.day(60_000)
            );
        }
    }

    /// Fed her routine, she arrives as her day has left her: at 16:00
    /// exactly as ever.
    #[test]
    fn she_arrives_at_four_exactly_as_ever() {
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        let ever = osaka.needs;
        osaka.set_clock(monday_at(16, 0).day(0));
        assert_eq!(osaka.needs, ever);
        osaka.set_clock(monday_at(21, 0).day(0));
        assert!(osaka.needs.get(Need::Sleepy) > ever.get(Need::Sleepy));
    }

    /// A terrain with one floor, at `row`, across 40 columns.
    fn floor_at(row: u16) -> Terrain {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::style::Style;
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        buf.set_string(0, row, "─".repeat(40), Style::default());
        Terrain::read(&buf, &[], false)
    }

    /// Her ticks from `now` on `terrain` with `clock`, a second apart, until
    /// she's asleep for the night (at most `bound` ms on): when she is.
    fn until_asleep(
        osaka: &mut Osaka,
        mut now: u64,
        clock: routine::Clock,
        terrain: &Terrain,
        rng: &mut Rng,
        bound: u64,
    ) -> u64 {
        let from = now;
        while !osaka.sleeping() {
            assert!(now < from + bound, "awake at {now}: {:?}", osaka.act);
            now += 1_000;
            osaka.tick(now, Some(clock), terrain, &Chances::default(), rng);
        }
        now
    }

    /// Awake in the night is out of her reach through every way out of
    /// what she's doing that skips an interruption (D4): put somewhere by
    /// the stage, set musing, her floor gone from under her (a fall, a
    /// dazed moment). Each ends in a decision, and that takes her back to
    /// sleep at once (with nothing shown to lie on, on the floor).
    #[test]
    fn every_way_out_of_her_night_leads_back_to_sleep() {
        let (high, low) = (floor_at(12), floor_at(15));
        let clock = clock_at(1, 2, 0);
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        let mut now = until_asleep(&mut osaka, 0, clock, &low, &mut rng, 5_000);
        assert_eq!(
            osaka.decisions.last().map(|d| d.method),
            Some("routine/bed-floor")
        );
        // Placed.
        osaka.place(30, 15, now);
        assert!(!osaka.sleeping());
        now = until_asleep(&mut osaka, now, clock, &low, &mut rng, 5_000);
        // Musing.
        osaka.muse(now, &mut rng);
        assert!(!osaka.sleeping());
        now = until_asleep(&mut osaka, now, clock, &low, &mut rng, 20_000);
        // Her floor gone: the one above it holds, and she falls onto the
        // one below in the other's stead.
        let mut rng2 = Rng(4);
        let mut up = Osaka::standing_at(20, 12, now, &mut rng2);
        up.read_clock(Some(clock), now);
        let then = until_asleep(&mut up, now, clock, &high, &mut rng2, 5_000);
        assert!(up.settle(then, &low));
        assert!(!up.sleeping(), "fell: {:?}", up.act);
        until_asleep(&mut up, then, clock, &low, &mut rng2, 10_000);
        assert_eq!(up.y, 15);
    }

    /// At bedtime, asleep in her bed (a day's sleep running into it):
    /// she sleeps on, the same act become her night's in place (A10), no
    /// interruption and no decision, lasting until her wake time (07:00);
    /// her credit stays with it, and only the night from bedtime counts
    /// as slept.
    #[test]
    fn a_sleep_running_into_bedtime_becomes_her_night() {
        let sleep = Act::Use {
            seat: seat_for(Use::Sleep, Furniture::Bed),
            since: BED - 30_000,
            until: BED + 90_000,
            whole: 120_000,
            play: Play::of(Use::Sleep, None),
            grievance: None,
        };
        let (mut osaka, mut rng) = at_bedtime(sleep, BED + 400);
        osaka.credit = Some(Want::Use(Use::Sleep));
        assert_eq!(osaka.cut_at, Some(BED));
        osaka.tick(
            BED,
            Some(monday_at(22, 0)),
            &blank_terrain(),
            &Chances::default(),
            &mut rng,
        );
        let wake = (9 * 60 - 30) * 10_000;
        let (since, until, whole, play) = begun(&osaka);
        assert_eq!((since, until), (BED - 30_000, BED + wake));
        assert_eq!(whole, until - play.body_start(since));
        assert_eq!(play.own, ScriptId::Night);
        assert_eq!(play.branch, Surface::Bed.branch(false));
        assert!(osaka.decisions.is_empty(), "{:?}", osaka.decisions);
        assert_eq!(osaka.cut_at, Some(SCHOOL));
        assert_eq!(osaka.credit, Some(Want::Use(Use::Sleep)));
        assert_eq!(osaka.prop(BED), Some(Prop::LampOff));
        // Her night is armed from bedtime: sleep-talk on its schedule, the
        // Dream's count begun.
        assert_eq!(osaka.talk_due(), Some(BED + talk_gap(osaka.whims, 0)));
        assert_eq!(osaka.dream_moment(), Some(BED + DREAM_AFTER_MS / 6));
        osaka.credit_done(BED + 60_000);
        assert_eq!(osaka.slept_counted(), 60_000, "from bedtime");
    }

    /// Her wake time is found afresh as her clock is read (a later day's
    /// flag is provisional until that day latches): a night begun on
    /// Sunday at bedtime, before a school day (up at 07:00), whose
    /// morning turns out a vacation day (the real date crosses into
    /// summer in the night, before Monday latches) lasts until 09:00:
    /// the longest night there is, 105 real minutes. On the floor and in
    /// her bed alike (a use's whole follows its end). And a wake time
    /// moved earlier when her next event is already the old one (the
    /// lamp's off: nothing else happens before her wake on the floor)
    /// still wakes her at the new one.
    #[test]
    fn her_wake_time_follows_her_mornings_flag() {
        use super::super::GameClock;
        let june = chrono::NaiveDate::from_ymd_opt(2026, 6, 17);
        let july = chrono::NaiveDate::from_ymd_opt(2026, 7, 25);
        let game = ((6 * 24 + 22) * 60 + 30 - routine::START) * 60_000;
        // Sunday latched in June (school tomorrow), Monday's flag read
        // from the date as it is now.
        let read = |date| routine::Clock {
            ahead: date == july,
            date,
            ..routine::Clock::read(GameClock { at: 0, game }, 0, None, june)
        };
        assert_eq!(read(july).latch.day, 6);
        let low = floor_at(15);
        // From 22:30 on Sunday, at monotonic 0.
        let school = (8 * 60 + 30) * 10_000;
        let summer = (10 * 60 + 30) * 10_000;
        for chances in [Chances::default(), bed_at(25)] {
            let bed = !chances.seats.is_empty();
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(read(june)), 0);
            let mut now = 0;
            while !osaka.sleeping() {
                assert!(now < 30_000, "bed={bed}: {:?}", osaka.act);
                now += 1_000;
                osaka.tick(now, Some(read(june)), &low, &chances, &mut rng);
            }
            let until = |osaka: &Osaka| match osaka.act {
                Act::Idle { until, .. } => until,
                Act::Use {
                    since,
                    until,
                    whole,
                    play,
                    ..
                } => {
                    assert_eq!(whole, until - play.body_start(since), "bed={bed}");
                    until
                }
                _ => panic!("{:?}", osaka.act),
            };
            assert_eq!(matches!(osaka.act, Act::Use { .. }), bed, "{:?}", osaka.act);
            assert_eq!(until(&osaka), school, "bed={bed}");
            osaka.read_clock(Some(read(july)), 0);
            assert_eq!(until(&osaka), summer, "bed={bed}: 105 real minutes");
            // And back, should the date go back before Monday latches.
            osaka.read_clock(Some(read(june)), 0);
            assert_eq!(until(&osaka), school, "bed={bed}");
        }
        // On the floor, settled in (the lamp off) on a predicted summer
        // morning: her next event is her 09:00 wake. Then the date turns
        // out June after all: up at 07:00, not 09:00.
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(read(july)), 0);
        let chances = Chances::default();
        let settled = 1_000 + script::LAMP_ON_MS + 5_000;
        for now in [1_000, settled] {
            osaka.tick(now, Some(read(july)), &low, &chances, &mut rng);
        }
        assert!(osaka.sleeping());
        assert_eq!(osaka.act_due, summer, "nothing before her wake");
        osaka.tick(school + 1_000, Some(read(june)), &low, &chances, &mut rng);
        assert!(!osaka.sleeping(), "awake at 07:00: {:?}", osaka.act);
    }

    /// Asleep, she stirs at a chat line and sleeps on, whatever the line
    /// asks, not even turning to it: for the night (A13), in her bed, on
    /// her sofa or on the floor, she murmurs ("mm..."), blinks and turns
    /// over; by day (phase 5c B1, a sleep or a nap), she does the same
    /// saying "Mm?".
    #[test]
    fn her_sleep_stirs_by_night_and_by_day() {
        let terrain = floor_at(15);
        for (what, item) in [(Use::Sleep, Furniture::Bed), (Use::Nap, Furniture::Sofa)] {
            for night in [false, true] {
                for asks in [false, true] {
                    let at = format!("{what:?} night={night} asks={asks}");
                    let mut rng = Rng(3);
                    let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                    let seat = Seat {
                        y: 15,
                        x: 20,
                        facing: Facing::Right,
                        ..seat_for(what, item)
                    };
                    let play = if night {
                        Play {
                            branch: Surface::of(what).unwrap().branch(true),
                            ..Play::plain(ScriptId::Night)
                        }
                    } else {
                        Play::of(what, None)
                    };
                    osaka.set(
                        Act::Use {
                            seat,
                            since: 0,
                            until: 600_000,
                            whole: 600_000,
                            play,
                            grievance: None,
                        },
                        0,
                    );
                    // On her own pose's frame 0, so the turn shows.
                    let plain = osaka.clone();
                    let line = (5_000..15_000)
                        .find(|&t| matches!(plain.appearance(t).0, Pose::Sleep(0) | Pose::Nap(0)))
                        .unwrap_or_else(|| panic!("{at}: never on frame 0"));
                    osaka.look(line, 0, asks, &terrain);
                    assert_eq!(osaka.sleeping(), night, "{at}");
                    let murmur = if night { MM } else { STIRRED };
                    assert!(matches!(osaka.act, Act::Use { .. }), "{at}");
                    assert_eq!(osaka.facing, Facing::Right, "{at}: not turned to it");
                    let (pose, face, said) = osaka.appearance(line);
                    assert!(matches!(pose, Pose::Sleep(1) | Pose::Nap(1)), "{at}");
                    assert_eq!(face, Face::Blink, "{at}");
                    assert_eq!(said, Some(Bubble::Say(murmur)), "{at}");
                    // Over, she's asleep as before.
                    let after = line + speech_ms(murmur);
                    let (_, _, said) = osaka.appearance(after);
                    assert_eq!(said, Some(Bubble::Zzz), "{at}");
                }
            }
        }
        // On the floor, lying still: turned over a moment, blinking, and
        // as still as before once it's over.
        for asks in [false, true] {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.set(
                Act::Idle {
                    what: Activity::LieBack,
                    since: 0,
                    until: 600_000,
                    play: Some(Play {
                        branch: Surface::Floor.branch(true),
                        ..Play::plain(ScriptId::Night)
                    }),
                },
                0,
            );
            let unstirred = osaka.clone();
            let (pose, ..) = unstirred.appearance(5_000);
            assert!(matches!(pose, Pose::LieBack(0)), "{pose:?}");
            osaka.look(5_000, 0, asks, &terrain);
            assert!(osaka.sleeping(), "floor asks={asks}");
            assert!(matches!(osaka.act, Act::Idle { .. }));
            assert_eq!(osaka.facing, unstirred.facing, "not turned to it");
            let (pose, face, said) = osaka.appearance(5_000);
            assert_eq!(pose, Pose::LieBack(1), "floor asks={asks}");
            assert_eq!(face, Face::Blink);
            assert_eq!(said, Some(Bubble::Say(MM)));
            let after = 5_000 + speech_ms(MM);
            assert_eq!(osaka.appearance(after), unstirred.appearance(after));
        }
    }

    /// Her bed, `x` along [`floor_at`]`(15)`, as the frame offers it.
    fn bed_at(x: i32) -> Chances {
        Chances {
            seats: vec![Seat {
                x,
                y: 15,
                facing: Facing::Right,
                ..seat_for(Use::Sleep, Furniture::Bed)
            }],
            ..Chances::default()
        }
    }

    /// Bedtime settles whatever moving of her home is under way: trying
    /// a piece where she set it down (kept there, "There!"), or with it
    /// in her pocket (let go, back where it stood). Either way her night
    /// is her night's sleep, not a trial sit over and over, and nothing
    /// of it is left in her pocket overnight.
    #[test]
    fn bedtime_settles_her_arranging() {
        let clock = clock_at(1, 2, 0);
        let terrain = floor_at(15);
        let chances = bed_at(25);
        let pocketed = Episode {
            pocket: true,
            tried: 0,
            trying: false,
            ..trial_episode()
        };
        for episode in [trial_episode(), pocketed] {
            let at = format!("pocket={} trying={}", episode.pocket, episode.trying);
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.episode = Some(episode);
            osaka.read_clock(Some(clock), 0);
            let mut now = 0;
            while !osaka.sleeping() {
                assert!(now < 60_000, "{at}: awake at {now}: {:?}", osaka.act);
                now += 1_000;
                osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
            }
            assert_eq!(osaka.episode, None, "{at}");
            assert_eq!(osaka.carrying(), None, "{at}");
            assert_eq!(osaka.seat().map(|s| s.item), Some(Furniture::Bed), "{at}");
            let methods: Vec<_> = osaka.decisions.iter().map(|d| d.method).collect();
            assert_eq!(methods, ["routine/bed"], "{at}");
            // And she sleeps on.
            for t in (now..now + 600_000).step_by(1_000) {
                osaka.tick(t, Some(clock), &terrain, &chances, &mut rng);
            }
            assert!(osaka.sleeping(), "{at}");
            assert_eq!(osaka.decisions.len(), 1, "{at}");
        }
    }

    /// Up at her wake time (put out of bed a moment before): the new day
    /// begins as she next decides, as if she'd woken, the stretch aside.
    /// Its mood and a fresh budget, good morning, her needs the
    /// morning's with nothing of the night kept, and the lamp on: the
    /// night's dark never reaches the day.
    #[test]
    fn up_at_her_wake_time_she_starts_the_day_all_the_same() {
        use super::super::brain::Needs;
        let clock = clock_at(1, 6, 50);
        let terrain = floor_at(15);
        let chances = bed_at(25);
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.key_days(9, Some(0));
        osaka.worked = true;
        osaka.read_clock(Some(clock), 0);
        let mut now = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
        // Settled in: dark.
        for t in (now..now + script::LAMP_ON_MS + 2_000).step_by(500) {
            osaka.tick(t, Some(clock), &terrain, &chances, &mut rng);
        }
        now += script::LAMP_ON_MS + 2_000;
        assert!(osaka.dark(now));
        // 07:00 is 100 s on: out of bed a second before.
        let wake = 100_000;
        osaka.place(2, 15, wake - 1_000);
        assert!(!osaka.sleeping());
        assert!(osaka.dark(wake - 1_000), "up in the dark");
        assert!(!osaka.dark(wake), "the day is never dark");
        let mood = Mood::of(brain::day_seed(9, 1));
        now = wake - 1_000;
        while osaka.budget_day == Some(0) {
            assert!(now < wake + 60_000, "the day never began");
            now += 500;
            osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
        }
        assert_eq!(osaka.day(now).map(|d| d.slot), Some(routine::Slot::Morning));
        assert_eq!(osaka.line_budget(), Some((1, 0)));
        assert_eq!(osaka.mood(), mood);
        assert!(osaka.greeted);
        assert_eq!(osaka.speech.map(|(line, _)| line), Some(mood.wake_line()));
        assert_eq!(osaka.slept_ms, 0);
        assert!(!osaka.worked);
        assert!(!osaka.lamp_off && !osaka.dark(now));
        assert_eq!(osaka.prop(now), None);
        // Her needs started from the morning's at that decision.
        let decided = osaka.decided;
        let mut morning = Needs::arriving_in(routine::Slot::Morning);
        assert!(decided <= now);
        if decided == now {
            assert_eq!(osaka.needs, morning);
        } else {
            morning = osaka.needs;
        }
        assert!(Need::ALL.iter().all(|&n| morning.get(n) < 0.9));
    }

    /// A night's sleep the stage gives her by day darkens her lamp by
    /// its own key while it plays, and never latches it: once it's over,
    /// the lamp is on (and no dark is left waiting for the night).
    #[test]
    fn a_stage_night_by_day_leaves_the_lamp_on() {
        let clock = monday_at(16, 0);
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        osaka.cue(Some(Cue::Script(ScriptId::Night)));
        osaka.credit = Some(Want::Use(Use::Sleep));
        osaka.start_job(
            Job::Use(seat_for(Use::Sleep, Furniture::Bed)),
            1_000,
            &Chances::default(),
            &mut rng,
        );
        assert!(osaka.sleeping(), "{:?}", osaka.act);
        let (_, until, _, _) = begun(&osaka);
        let terrain = blank_terrain();
        let mut dark_by_key = false;
        let mut now = 1_000;
        while osaka.sleeping() {
            assert!(now <= until + 1_000, "{:?}", osaka.act);
            dark_by_key |= osaka.prop(now) == Some(Prop::LampOff);
            assert!(!osaka.dark(now), "by day, the latch never sets");
            now += 500;
            osaka.tick(now, Some(clock), &terrain, &Chances::default(), &mut rng);
        }
        assert!(dark_by_key, "dark while she sleeps, by its key");
        assert!(!osaka.lamp_off);
        assert_eq!(osaka.prop(now), None);
    }

    /// Her clock read after a step that lost time (a suspend the shell's
    /// cap clamped): a catch-up moment before the new reading never maps
    /// back to before the last one.
    #[test]
    fn a_clock_that_lost_time_never_reads_earlier() {
        use super::super::GameClock;
        let was = monday_at(22, 0);
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
        osaka.read_clock(Some(was), 0);
        let (then, now) = (500_000, 1_000_000);
        let lost = GameClock {
            at: now,
            game: was.game.at(1_000),
        };
        osaka.read_clock(Some(routine::Clock::read(lost, now, None, None)), 0);
        assert!(osaka.game_at(then) >= Some(was.game.game));
        assert!(osaka.game_at(now) >= Some(was.game.game));
    }

    /// A day's sleep in her bed that bedtime made her night's, in place,
    /// is her night through to its end: at 07:00 she wakes (a stretch,
    /// good morning in the new day's mood), a fresh budget and shift,
    /// nothing of the night kept twice, the lamp on.
    #[test]
    fn a_sleep_become_her_night_wakes_her_at_seven() {
        let sleep = Act::Use {
            seat: seat_for(Use::Sleep, Furniture::Bed),
            since: BED - 30_000,
            until: BED + 90_000,
            whole: 120_000,
            play: Play::of(Use::Sleep, None),
            grievance: None,
        };
        let (mut osaka, mut rng) = at_bedtime(sleep, BED + 400);
        osaka.credit = Some(Want::Use(Use::Sleep));
        osaka.key_days(9, Some(0));
        osaka.worked = true;
        let clock = Some(monday_at(22, 0));
        let terrain = blank_terrain();
        osaka.tick(BED, clock, &terrain, &Chances::default(), &mut rng);
        assert!(osaka.sleeping());
        let wake = BED + (9 * 60 - 30) * 10_000;
        let mut now = BED;
        while osaka.sleeping() {
            assert!(now < wake + 2_000, "still asleep at {now}");
            now += 1_000;
            osaka.tick(now, clock, &terrain, &Chances::default(), &mut rng);
        }
        assert!(now >= wake, "woke at {now}, before {wake}");
        assert!(
            matches!(
                osaka.act,
                Act::Idle {
                    what: Activity::Stretch,
                    ..
                }
            ),
            "{:?}",
            osaka.act
        );
        let mood = Mood::of(brain::day_seed(9, 1));
        assert_eq!(osaka.speech.map(|(line, _)| line), Some(mood.wake_line()));
        assert_eq!(osaka.line_budget(), Some((1, 0)));
        assert!(!osaka.worked, "a new shift");
        assert_eq!(osaka.slept_ms, 0, "the night not kept twice");
        assert!(!osaka.lamp_off);
        assert_eq!(osaka.prop(now), None);
    }

    /// Her ticks from `now` a second apart with `chances` until she's
    /// asleep for the night (within a minute): when she is.
    fn until_asleep_with(
        osaka: &mut Osaka,
        mut now: u64,
        clock: routine::Clock,
        terrain: &Terrain,
        chances: &Chances,
        rng: &mut Rng,
    ) -> u64 {
        let from = now;
        while !osaka.sleeping() {
            assert!(now < from + 60_000, "awake at {now}: {:?}", osaka.act);
            now += 1_000;
            osaka.tick(now, Some(clock), terrain, chances, rng);
        }
        now
    }

    /// Watching the chat at night, the bed reflex fires first (A3), and
    /// she still faces the chat as it does: on the floor, where she lies
    /// down where she is, facing it.
    #[test]
    fn to_bed_while_watching_she_still_faces_the_chat() {
        let terrain = floor_at(15);
        for (watch_x, facing) in [(0, Facing::Left), (39, Facing::Right)] {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.facing = if facing == Facing::Left {
                Facing::Right
            } else {
                Facing::Left
            };
            osaka.read_clock(Some(clock_at(1, 2, 0)), 0);
            osaka.watch_until = 10_000;
            osaka.watch_x = watch_x;
            osaka.decide(1_000, &terrain, &Chances::default(), &mut rng);
            let method = osaka.decisions.last().map(|d| d.method);
            assert_eq!(method, Some("routine/bed-floor"));
            assert!(osaka.sleeping());
            assert_eq!(osaka.facing, facing, "watching {watch_x}");
        }
    }

    // ---- Her night's extras (phase 5b step 4b) ----

    /// Her ticks from `now` until `until` (each step to her next event,
    /// at most a second) in `world` (her clock, the terrain and what's on
    /// offer), `watch` seeing her after each.
    fn tick_until(
        osaka: &mut Osaka,
        mut now: u64,
        until: u64,
        (clock, terrain, chances): (routine::Clock, &Terrain, &Chances),
        rng: &mut Rng,
        mut watch: impl FnMut(&Osaka, u64),
    ) -> u64 {
        while now < until {
            now = osaka.due().clamp(now + 1, now + 1_000).min(until);
            osaka.tick(now, Some(clock), terrain, chances, rng);
            watch(osaka, now);
        }
        now
    }

    /// Every line she said in her sleep, and when.
    fn sleep_talk(osaka: &Osaka) -> Vec<(u64, &'static str)> {
        osaka
            .lines
            .said()
            .iter()
            .filter(|&&(pool, ..)| pool == PoolId::SleepTalk)
            .map(|&(_, line, at)| (at, line))
            .collect()
    }

    /// The last game hour's line.
    const FIVE_MORE: &str = "...five more minutes";

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(
            dessplay_core::test_support::proptest_cases(64)
        ))]

        /// Her sleep-talk's spacing is six to ten real minutes, a pure
        /// function of the night act's whims and the line's place.
        #[test]
        fn her_sleep_talk_is_spaced_six_to_ten_minutes(seed: u64, k in 0u64..64) {
            let gap = talk_gap(Whims(seed), k);
            proptest::prop_assert!((6 * 60_000..=10 * 60_000).contains(&gap));
            proptest::prop_assert_eq!(gap, talk_gap(Whims(seed), k));
        }
    }

    /// Through a night (Tuesday 00:00 to her 07:00 wake, 70 real
    /// minutes), she talks in her sleep exactly on the schedule her night
    /// act's start and its starting decision's whims make (A14): every
    /// six to ten minutes, a line from her pool, never budgeted; in the
    /// last game hour "...five more minutes" (once: it cools), and never
    /// before.
    #[test]
    fn she_talks_in_her_sleep_on_a_pure_schedule() {
        let terrain = floor_at(15);
        let chances = bed_at(25);
        let clock = clock_at(1, 0, 0);
        let wake = 7 * 60 * 10_000;
        for seed in 0..6 {
            let mut rng = Rng(seed);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(clock), 0);
            let now = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
            let (start, whims) = (osaka.act_since, osaka.whims);
            let mut schedule = Vec::new();
            let mut t = start;
            for k in 0.. {
                t += talk_gap(whims, k);
                if t >= wake {
                    break;
                }
                schedule.push(t);
            }
            assert_eq!(osaka.talk_due(), schedule.first().copied(), "seed {seed}");
            let mut talked = Vec::new();
            tick_until(
                &mut osaka,
                now,
                wake - 1,
                (clock, &terrain, &chances),
                &mut rng,
                |osaka, now| {
                    assert!(osaka.sleeping(), "seed {seed}: up at {now}");
                    if let Some((line, until)) = osaka.speech
                        && until == now + speech_ms(line)
                    {
                        talked.push((now, line));
                    }
                },
            );
            // Her lamp off as she settles in: "Night-night...", once, as
            // its key comes (her routine's line, not sleep-talk).
            let night_night = mind::NIGHT_NIGHT.lines[0];
            let lamp: Vec<u64> = talked
                .iter()
                .filter(|&&(_, line)| line == night_night)
                .map(|&(t, _)| t)
                .collect();
            assert_eq!(lamp, [start + script::LAMP_ON_MS], "seed {seed}");
            talked.retain(|&(_, line)| line != night_night);
            let said = sleep_talk(&osaka);
            assert_eq!(said, talked, "seed {seed}: each shown as it's said");
            assert!(osaka.lines.spent() == 0, "seed {seed}: budgeted");
            let last_hour = |t: u64| t + LAST_HOUR_MS >= wake;
            // Before the last hour, every time on the schedule has a line
            // (seven, at most one of them cooling); in it, only the one.
            let before: Vec<u64> = schedule
                .iter()
                .copied()
                .filter(|&t| !last_hour(t))
                .collect();
            let said_before: Vec<u64> = said
                .iter()
                .filter(|&&(t, _)| !last_hour(t))
                .map(|&(t, _)| t)
                .collect();
            assert_eq!(said_before, before, "seed {seed}");
            assert!(said.len() >= 6, "seed {seed}: {said:?}");
            for &(t, line) in &said {
                assert!(schedule.contains(&t), "seed {seed}: {t} off the schedule");
                assert_eq!(
                    line == FIVE_MORE,
                    last_hour(t),
                    "seed {seed}: {line} at {t}"
                );
                assert!(
                    mind::SLEEP_TALK.lines.contains(&line) || line == FIVE_MORE,
                    "{line}"
                );
            }
            assert_eq!(
                said.iter().filter(|&&(_, l)| l == FIVE_MORE).count(),
                1,
                "seed {seed}: {said:?}"
            );
            for pair in schedule.windows(2) {
                assert!((6 * 60_000..=10 * 60_000).contains(&(pair[1] - pair[0])));
            }
        }
    }

    /// A fridge, `x` along [`floor_at`]`(15)`, and her bed at 25.
    fn fridge_and_bed(x: i32) -> Chances {
        let mut chances = bed_at(25);
        chances.seats.push(Seat {
            x,
            y: 15,
            facing: Facing::Left,
            ..seat_for(Use::Snack, Furniture::Fridge)
        });
        chances
    }

    /// The groggy errand (Q2): asleep for the night, sent to the
    /// accordion, she gets up blinking, says "Mm... someone said..." as
    /// she pokes it, and her routine takes her back to bed: one night act
    /// again until her wake time (Tuesday 07:00), no longer groggy, the
    /// lamp off all the while, the night she slept kept for her needs and
    /// the Dream's count running on from her first sleep. On a fridge
    /// night too (none due), from her bed and with a walk or a door. In
    /// for the night, she's past her glance at the clock: back to bed,
    /// with it hanging over her floor now, she doesn't glance.
    #[test]
    fn the_groggy_errand_goes_back_to_bed() {
        let terrain = floor_at(15);
        let clock = clock_at(1, 2, 0);
        let wake = 5 * 60 * 10_000;
        for (unlit, spot) in [(bed_at(25), (18, 15)), (fridge_and_bed(4), (2, 15))] {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(clock), 0);
            let asleep = until_asleep_with(&mut osaka, 0, clock, &terrain, &unlit, &mut rng);
            let chances = Chances {
                clock: Some(clock_on(30, 15)),
                ..unlit
            };
            let first = osaka.act_since;
            let dream = osaka.dream_moment();
            assert_eq!(
                dream,
                Some(clock.game.when(clock.game.at(first) + DREAM_AFTER_MS))
            );
            let mut now = tick_until(
                &mut osaka,
                asleep,
                asleep + 300_000,
                (clock, &terrain, &chances),
                &mut rng,
                |_, _| {},
            );
            assert!(osaka.dark(now));
            osaka.errand(spot, &terrain, now);
            assert!(osaka.groggy() && !osaka.sleeping());
            assert!(
                osaka.slept_counted() >= 300_000,
                "{}",
                osaka.slept_counted()
            );
            let mut poked = None;
            while poked.is_none() {
                assert!(now < asleep + 360_000, "never poked: {:?}", osaka.act);
                now += 100;
                osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
                assert_eq!(osaka.appearance(now).1, Face::Blink, "{:?}", osaka.act);
                assert!(osaka.dark(now), "the lamp stays off");
                if osaka.take_poked() {
                    poked = osaka.speech.map(|(line, _)| line);
                }
            }
            assert_eq!(poked, Some(SLEEPY_POKE));
            now = until_asleep_with(&mut osaka, now, clock, &terrain, &chances, &mut rng);
            assert!(!osaka.groggy());
            assert!(osaka.dark(now));
            assert_eq!(osaka.night_play().map(|(.., until)| until), Some(wake));
            assert_eq!(osaka.dream_moment(), dream, "the Dream's count runs on");
            assert!(
                osaka.slept_counted() >= 300_000,
                "kept: {}",
                osaka.slept_counted()
            );
            assert_eq!(
                osaka.decisions.last().map(|d| d.method),
                Some("routine/bed")
            );
            let methods: Vec<&str> = osaka.decisions.iter().map(|d| d.method).collect();
            assert!(!methods.contains(&"routine/glance"), "{methods:?}");
        }
    }

    /// A master seed whose night ending on game `morning` has a midnight
    /// snack, and its minute.
    fn snack_night(morning: u64) -> (u64, u16) {
        (0..10_000)
            .find_map(|master| brain::night_snack(master, morning).map(|m| (master, m)))
            .expect("a night with a snack")
    }

    /// Her midnight snack is a pure function of the home and the night:
    /// one night in four or so, inside the window, the same every time.
    #[test]
    fn the_midnight_snack_is_hashed_from_the_home_and_night() {
        let mut nights = 0;
        let (lo, hi) = routine::SNACK_WINDOW;
        for master in 0..400u64 {
            for morning in 0..10 {
                let snack = brain::night_snack(master, morning);
                assert_eq!(snack, brain::night_snack(master, morning));
                if let Some(minute) = snack {
                    assert!((lo..hi).contains(&minute), "{minute}");
                    nights += 1;
                }
            }
        }
        assert!((800..1200).contains(&nights), "{nights} in 4000");
    }

    /// The midnight snack (round 1): on a night that has one, with a
    /// fridge, she gets up at its moment, pads to the fridge with the
    /// lamp off, has a snack as any plays (nothing spliced round it,
    /// nothing felt, nothing recorded), and goes back to bed: once, the
    /// night act again until her wake time, the night she slept kept and
    /// the Dream's count running on. While she's up, it's still her
    /// night: no parcel, no sleep-talk, no second snack due. The same
    /// night with no fridge, she sleeps through. Deterministic: the same
    /// night twice is the same. (Every splice that may wrap a snack would,
    /// her fridge's grievance is there to feel, and a splice is cued: the
    /// snack is built as it plays all the same, the cue left for later.)
    #[test]
    fn the_midnight_snack_once_a_night_with_a_fridge() {
        use super::script::{ScriptId, SpliceId};
        let cue = Cue::Splice(SpliceId::TestSnack, None);
        let terrain = floor_at(15);
        let clock = clock_at(1, 0, 0);
        let wake = 7 * 60 * 10_000;
        let (master, minute) = snack_night(1);
        let moment = u64::from(minute) * 10_000;
        let run = |chances: &Chances| {
            let mut rng = Rng(9);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.key_days(master, Some(1));
            osaka.read_clock(Some(clock), 0);
            osaka.splices_sure = true;
            osaka.cued = Some(cue);
            let now = until_asleep_with(&mut osaka, 0, clock, &terrain, chances, &mut rng);
            let (asleep, dream) = (osaka.act_since, osaka.dream_moment());
            assert!(dream.is_some());
            // Her wall clock over her floor from now on: in for the
            // night, she's past her glance at it (back from her snack, she
            // doesn't glance).
            let decided = osaka.decisions.len();
            let chances = &Chances {
                clock: Some(clock_on(30, 15)),
                ..chances.clone()
            };
            // Snacks: when each began, and whether she was ever awake
            // other than for one; what she'd slept as she got up.
            let mut snacks: Vec<u64> = Vec::new();
            let mut up = Vec::new();
            let mut slept = None;
            let mut fridge_open = false;
            tick_until(
                &mut osaka,
                now,
                wake - 1,
                (clock, &terrain, chances),
                &mut rng,
                |osaka, now| {
                    if let Act::Use {
                        seat,
                        since,
                        play,
                        grievance,
                        ..
                    } = osaka.act
                        && seat.what == Use::Snack
                    {
                        if snacks.last() != Some(&since) {
                            snacks.push(since);
                        }
                        assert!(play.before.is_none() && play.after.is_none());
                        assert_eq!(play.own, ScriptId::Snack);
                        assert!(grievance.is_none());
                        assert!(osaka.dark(now), "the lamp's off");
                        fridge_open |= osaka.prop(now) == Some(Prop::FridgeOpen);
                    }
                    if !osaka.sleeping() {
                        up.push(now);
                        slept.get_or_insert(osaka.slept_counted());
                        assert!(!osaka.awake(now), "up at {now}: a parcel waits");
                        assert!(osaka.talk_due().is_none() && osaka.snack_due().is_none());
                    }
                },
            );
            assert!(osaka.sleeping(), "back in bed");
            let methods: Vec<&str> = osaka.decisions[decided..]
                .iter()
                .map(|d| d.method)
                .collect();
            assert!(!methods.contains(&"routine/glance"), "{methods:?}");
            assert!(osaka.events.is_empty(), "{:?}", osaka.events);
            assert!(osaka.felt.is_empty(), "{:?}", osaka.felt);
            assert_eq!(osaka.cued, Some(cue), "the cue waits");
            assert_eq!(osaka.dream_moment(), dream, "the Dream's count runs on");
            if let Some(slept) = slept {
                assert!(slept >= moment - asleep, "{slept} of {}", moment - asleep);
                assert!(osaka.slept_counted() >= slept, "kept");
            }
            (snacks, up, fridge_open, osaka)
        };
        let mut chances = fridge_and_bed(4);
        chances
            .broken
            .extend(broken_for(Use::Snack, Furniture::Fridge));
        let (snacks, up, fridge_open, osaka) = run(&chances);
        assert_eq!(snacks.len(), 1, "{snacks:?}");
        assert!(
            snacks[0] > moment && snacks[0] < moment + 60_000,
            "{snacks:?} vs {moment}"
        );
        assert!(
            up.iter().all(|&t| t >= moment && t < moment + 120_000),
            "{up:?}"
        );
        assert!(fridge_open);
        assert_eq!(osaka.night_play().map(|(.., until)| until), Some(wake));
        assert!(osaka.slept_counted() > 0, "kept");
        assert!(
            osaka
                .credited
                .iter()
                .any(|&(want, ..)| want == Want::Use(Use::Snack)),
            "a snack eases her hunger"
        );
        assert_eq!(run(&chances).0, snacks, "deterministic");
        // No fridge: she sleeps through.
        let (snacks, up, ..) = run(&bed_at(25));
        assert!(snacks.is_empty() && up.is_empty(), "{snacks:?} {up:?}");
        // A night without one: none, fridge or no.
        let quiet = (0..10_000)
            .find(|&m| brain::night_snack(m, 1).is_none())
            .expect("a night without one");
        let mut rng = Rng(9);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.key_days(quiet, Some(1));
        osaka.read_clock(Some(clock), 0);
        let chances = fridge_and_bed(4);
        let now = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
        tick_until(
            &mut osaka,
            now,
            wake - 1,
            (clock, &terrain, &chances),
            &mut rng,
            |o, t| {
                assert!(o.sleeping(), "up at {t}: {:?}", o.act);
            },
        );
    }

    /// Her part-time job (D2): `may_work` is the one place it's decided.
    /// Her routine fed, it's open only on a day off (a weekend, or
    /// vacation) from 10:00 to 17:00, however long she's been here;
    /// unfed, as before the clock: three minutes into the visit. Never
    /// twice, never in a home that isn't one.
    #[test]
    fn her_job_is_open_on_days_off_from_ten_to_five() {
        use super::super::GameClock;
        let furnished = Chances {
            furnished: true,
            ..Chances::default()
        };
        let at = |clock: Option<routine::Clock>, now: u64| {
            let mut rng = Rng(1);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(clock, 0);
            osaka.may_work(&furnished, now)
        };
        for (day, h, m, open) in [
            (5, 9, 59, false),
            (5, 10, 0, true),
            (5, 16, 59, true),
            (5, 17, 0, false),
            (6, 12, 0, true),
            (0, 16, 0, false),
            (2, 12, 0, false),
            (7, 11, 0, false),
        ] {
            assert_eq!(at(Some(clock_at(day, h, m)), 0), open, "day {day} {h}:{m}");
        }
        // A Monday in the summer vacation.
        let game = ((7 * 24 + 11) * 60 - routine::START) * 60_000;
        let summer = routine::Clock::read(
            GameClock { at: 0, game },
            0,
            None,
            chrono::NaiveDate::from_ymd_opt(2026, 7, 27),
        );
        assert!(at(Some(summer), 0));
        // Unfed: three minutes in.
        assert!(!at(None, WORK_AFTER_MS - 1));
        assert!(at(None, WORK_AFTER_MS));
        // Worked already, or no home.
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock_at(5, 12, 0)), 0);
        assert!(!osaka.may_work(&Chances::default(), 0));
        osaka.worked = true;
        assert!(!osaka.may_work(&furnished, 0));
    }

    // ---- Her night's end, and her shift's (step 4b, review fixes) ----

    /// Up groggy across her wake time (an errand just before it), the
    /// night is behind her once it passes: her day begins as she next
    /// decides (her night, her snack, the lamp and the groggy blink all
    /// over, a parcel free to come), whether the visit began before
    /// midnight or after it (keyed on that day or the morning's), from a
    /// walk or a door. A poke after the wake is the waking one.
    #[test]
    fn groggy_never_outlasts_her_night() {
        let terrain = floor_at(15);
        let chances = bed_at(25);
        let clock = clock_at(1, 6, 50);
        let wake = 10 * 10_000;
        for (keyed, spot) in [(1, (18, 15)), (1, (2, 15)), (0, (18, 15)), (0, (2, 15))] {
            let what = format!("keyed on day {keyed}, the errand to {spot:?}");
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.key_days(9, Some(keyed));
            osaka.read_clock(Some(clock), 0);
            let asleep = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
            assert!(asleep < wake - 10_000, "{what}");
            let world = (clock, &terrain, &chances);
            let now = tick_until(&mut osaka, asleep, wake - 1_500, world, &mut rng, |_, _| {});
            osaka.errand(spot, &terrain, now);
            assert!(osaka.groggy(), "{what}");
            assert!(!osaka.awake(now), "{what}: a parcel waits");
            let mut poked = None;
            let now = tick_until(&mut osaka, now, wake + 180_000, world, &mut rng, |o, t| {
                if poked.is_none() && o.poked {
                    poked = o.speech.map(|(line, _)| (t, line));
                }
            });
            let (at, line) = poked.unwrap_or_else(|| panic!("{what}: never poked"));
            assert_eq!(line, if at < wake { SLEEPY_POKE } else { POKE }, "{what}");
            assert!(!osaka.groggy(), "{what}: groggy by day");
            assert!(!osaka.drowsy(now), "{what}");
            assert!(osaka.dream_moment().is_none(), "{what}: her night kept");
            assert!(!osaka.lamp_off && osaka.snacking.is_none(), "{what}");
            assert!(osaka.awake(now), "{what}");
            assert_eq!(osaka.line_budget().map(|(day, _)| day), Some(1), "{what}");
        }
    }

    /// Two floors reaching the screen's edges, each linked around to the
    /// other: the left at row 4, columns 0-15; the right at row `right`,
    /// 24-39.
    fn edge_floors(right: u16) -> Terrain {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::style::Style;
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 8));
        buf.set_string(0, 4, "─".repeat(16), Style::default());
        buf.set_string(24, right, "─".repeat(16), Style::default());
        Terrain::read(&buf, &[], false)
    }

    /// Her ticks a tenth of a second apart from `now` on `terrain` with
    /// `chances` (her door), until `done` (at most `bound` ms on): when
    /// it is.
    fn tick_with(
        osaka: &mut Osaka,
        mut now: u64,
        terrain: &Terrain,
        chances: &Chances,
        rng: &mut Rng,
        bound: u64,
        done: impl Fn(&Osaka) -> bool,
    ) -> u64 {
        let from = now;
        while !done(osaka) {
            assert!(now < from + bound, "at {now}: {:?}", osaka.act);
            now += 100;
            osaka.tick(now, None, terrain, chances, rng);
        }
        now
    }

    /// The frame's door standing at `door` (gated and not: no pane
    /// focused).
    fn door_chances(door: DoorSpot) -> Chances {
        Chances {
            door: Some(door),
            door_through: Some(door),
            ..Chances::default()
        }
    }

    /// From where she is in her door's beats (`since`, `gap`) at `now`:
    /// whether she's shown at its far side.
    fn shown_there(osaka: &Osaka, now: u64) -> bool {
        match osaka.act {
            Act::Door { since, to, gap } => door_beat(now.saturating_sub(since), gap, to)
                .is_some_and(|(beat, _)| beat.there && beat.her),
            _ => false,
        }
    }

    /// Her part-time job goes out and comes home through her door (door
    /// batch, step 4b): she walks to it (on her floor, or round the
    /// screen's edge to another, her heading resumed at the landing),
    /// goes through it for her shift (its gap), and comes back out of
    /// it facing the room, carrying her shopping as she shows, home from
    /// work ("I'm home!"). Never off the screen's edge for the shift: a
    /// trip round the edge on her way is an ordinary one. Her way to
    /// work cut short by a startle: no shift, no homecoming.
    #[test]
    fn a_shift_through_her_door_ends_at_home() {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::style::Style;
        // Two floors with no way between them but a door in space (the
        // door's floor is lower, its ends short of the screen's edges).
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 14));
        buf.set_string(2, 4, "─".repeat(14), Style::default());
        buf.set_string(24, 12, "─".repeat(14), Style::default());
        let apart = Terrain::read(&buf, &[], false);
        assert!(apart.links.is_empty(), "{:?}", apart.links);
        let level = edge_floors(4);
        for (terrain, (door_x, door_y), what) in [
            (&level, (12, 4), "on her floor"),
            (&level, (30, 4), "round the screen's edge"),
            (&apart, (30, 12), "by a door in space"),
        ] {
            let level = terrain;
            let door = wall_door(door_x, door_y);
            let chances = door_chances(door);
            let mut rng = Rng(4);
            let mut osaka = Osaka::standing_at(6, 4, 0, &mut rng);
            osaka.go_to_work(level, &chances, 0, &mut rng);
            // On her way by a door in space: a moment's, never her shift.
            if level.links.is_empty() {
                assert!(
                    matches!(
                        osaka.act,
                        Act::Door {
                            to: Through::Space(_),
                            gap: 0,
                            ..
                        }
                    ),
                    "{what}: {:?}",
                    osaka.act
                );
            }
            let now = tick_with(&mut osaka, 0, level, &chances, &mut rng, 60_000, |o| {
                o.through() == Some(Through::Home(door))
            });
            assert_eq!((osaka.x, osaka.y), door.spot(), "{what}: at her door");
            assert_eq!(osaka.facing, door.out(), "{what}: facing out");
            let Act::Door { since, gap, .. } = osaka.act else {
                unreachable!()
            };
            assert!(
                (SHIFT_MS.0..SHIFT_MS.1).contains(&gap),
                "{what}: the shift's gap: {gap}"
            );
            assert_eq!(osaka.leaving, None, "{what}");
            let (mut now, mut shown, mut carried) = (now, 0, 0);
            while matches!(osaka.act, Act::Door { .. }) {
                assert!(now < since + SHIFT_MS.1 + 10_000, "{what}: never back");
                now += 100;
                osaka.tick(now, None, level, &chances, &mut rng);
                assert!(
                    !matches!(osaka.act, Act::Out { .. } | Act::Away { .. }),
                    "{what}: off the edge: {:?}",
                    osaka.act
                );
                if shown_there(&osaka, now) {
                    shown += 1;
                    assert_eq!((osaka.x, osaka.y), door.spot(), "{what}: out of it");
                    assert_eq!(osaka.facing, door.into_room(), "{what}: into the room");
                    carried += usize::from(matches!(osaka.appearance(now).0, Pose::Carry(_)));
                }
            }
            assert!(now >= since + SHIFT_MS.0, "{what}: a shift");
            assert!(shown > 0, "{what}: seen coming out");
            assert_eq!(carried, shown, "{what}: her shopping as she comes out");
            assert!(
                matches!(osaka.act, Act::Home { .. }),
                "{what}: {:?}",
                osaka.act
            );
            assert_eq!(osaka.speech.map(|(line, _)| line), Some(HOME), "{what}");
            assert!(osaka.shift.is_none(), "{what}");
        }
        // Her way to work cut short: no shift, no homecoming.
        let door = wall_door(30, 4);
        let chances = door_chances(door);
        let mut rng = Rng(4);
        let mut osaka = Osaka::standing_at(3, 4, 0, &mut rng);
        osaka.go_to_work(&level, &chances, 0, &mut rng);
        osaka.interrupt(Cause::Shaken, 100);
        tick_with(&mut osaka, 100, &level, &chances, &mut rng, 30_000, |o| {
            !matches!(o.act, Act::Look { .. })
        });
        assert!(osaka.shift.is_none());
        assert!(osaka.speech.is_none_or(|(line, _)| line != HOME));
    }

    /// On her way to work through her door as school begins (the cut
    /// takes her shift: `school_from_work`), she walks on to her door
    /// and out through it to school, for good (door batch M9): the
    /// door's gap never ends, `leaving` set only as it opens (never on
    /// her way), and the guest may end the visit once it has closed
    /// behind her ([`Osaka::gone_out`]).
    #[test]
    fn walking_out_to_work_as_school_begins_she_walks_on_to_school() {
        let (mut osaka, terrain, mut rng) = on_a_school_day(3, 8, 14, Act::Stand { until: 0 });
        let clock = Some(clock_at(1, 8, 14));
        let door = wall_door(36, 15);
        let chances = door_chances(door);
        osaka.go_to_work(&terrain, &chances, 0, &mut rng);
        // 08:15 is 10 s on.
        let mut now = 0;
        let mut walked_on = false;
        while osaka.through().is_none() {
            assert!(now < 30_000, "never at her door: {:?}", osaka.act);
            now += 100;
            osaka.tick(now, clock, &terrain, &chances, &mut rng);
            if osaka.through().is_none() {
                assert_eq!(osaka.leaving, None, "at {now}: leaving on her way");
                walked_on |= now > 10_000;
            }
        }
        assert!(walked_on, "on her way at 08:15");
        assert_eq!(osaka.through(), Some(Through::Home(door)));
        assert!(
            matches!(osaka.act, Act::Door { gap: u64::MAX, .. }),
            "{:?}",
            osaka.act
        );
        assert_eq!(osaka.leaving, Some(Routine::School));
        assert!(osaka.shift.is_none());
        while osaka.gone_out(now).is_none() {
            assert!(now < 60_000, "never out: {:?}", osaka.act);
            now += 100;
            osaka.tick(now, clock, &terrain, &chances, &mut rng);
        }
        assert!(osaka.hidden(now) && osaka.door(now).is_none());
    }

    /// A focused pane over her on her walk to her door for work (door
    /// batch C4): she goes out to work from where she's moved to, by a
    /// door in space whose gap is her shift's (never 0), and comes home
    /// by her door (M10: the frame's door read again before she's out of
    /// it).
    #[test]
    fn evicted_on_her_way_to_work_she_keeps_her_shift() {
        let terrain = floor_at(15);
        let door = wall_door(36, 15);
        let chances = door_chances(door);
        let mut rng = Rng(4);
        let mut osaka = Osaka::standing_at(3, 15, 0, &mut rng);
        osaka.go_to_work(&terrain, &chances, 0, &mut rng);
        let now = tick_with(&mut osaka, 0, &terrain, &chances, &mut rng, 30_000, |o| {
            o.x >= 10
        });
        assert!(
            matches!(
                osaka.act,
                Act::Walk {
                    then: Then::Job(Job::Leave {
                        why: Leave::Work,
                        ..
                    }),
                    ..
                }
            ),
            "on her way: {:?}",
            osaka.act
        );
        let focus = Rect::new(osaka.x as u16 - 3, 10, 7, 6);
        assert!(osaka.evict(focus, &terrain, &[], None, now, &mut rng));
        let Act::Door {
            to: Through::Space(spot),
            gap,
            ..
        } = osaka.act
        else {
            panic!("out by a door in space: {:?}", osaka.act);
        };
        assert!(!box_meets(focus, spot), "{spot:?}");
        assert!((SHIFT_MS.0..SHIFT_MS.1).contains(&gap), "her shift: {gap}");
        assert!(osaka.shift.is_some());
        let (mut now, mut shown) = (now, 0);
        while !matches!(osaka.act, Act::Home { .. }) {
            assert!(now < SHIFT_MS.1 + 30_000, "never home: {:?}", osaka.act);
            now += 100;
            osaka.tick(now, None, &terrain, &chances, &mut rng);
            if shown_there(&osaka, now) {
                shown += 1;
                assert_eq!(osaka.through(), Some(Through::Home(door)), "by her door");
                assert_eq!((osaka.x, osaka.y), door.spot());
            }
        }
        assert!(shown > 0, "seen coming home");
        assert_eq!(osaka.speech.map(|(line, _)| line), Some(HOME));
    }

    /// Evicted at her door on her way to work, its beats begun (door
    /// batch M10): the door she's through opens elsewhere (a door in
    /// space, keeping her shift's gap), and she comes home by her door,
    /// where the frame stands it once she's back.
    #[test]
    fn evicted_at_her_door_on_her_way_to_work_she_comes_home_by_it() {
        let terrain = floor_at(15);
        let door = wall_door(20, 15);
        let chances = door_chances(door);
        let mut rng = Rng(4);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.go_to_work(&terrain, &chances, 0, &mut rng);
        assert_eq!(
            osaka.through(),
            Some(Through::Home(door)),
            "{:?}",
            osaka.act
        );
        let Act::Door { gap: shift, .. } = osaka.act else {
            unreachable!()
        };
        let now = 300;
        osaka.tick(now, None, &terrain, &chances, &mut rng);
        let focus = Rect::new(17, 10, 7, 6);
        assert!(osaka.evict(focus, &terrain, &[], None, now, &mut rng));
        assert!(
            matches!(osaka.act, Act::Door { to: Through::Space(_), gap, .. } if gap == shift),
            "{:?}",
            osaka.act
        );
        let mut now = now;
        while !matches!(osaka.act, Act::Home { .. }) {
            assert!(now < SHIFT_MS.1 + 30_000, "never home: {:?}", osaka.act);
            now += 100;
            osaka.tick(now, None, &terrain, &chances, &mut rng);
            if shown_there(&osaka, now) {
                assert_eq!(osaka.through(), Some(Through::Home(door)), "by her door");
                assert_eq!((osaka.x, osaka.y), door.spot());
            }
        }
        assert_eq!(osaka.speech.map(|(line, _)| line), Some(HOME));
    }

    /// A door of hers turned into a door in space mid-way keeps its
    /// beat, whoever turns it (door batch, step 8 review): her own
    /// door's steps take walking pace, a door in space's don't, so an
    /// errand breaking in, or a focused pane over her door, late in
    /// beat 2 (she's still shown, stepping through) or in beat 5 (out of
    /// sight, the door shutting) must leave her in that beat, shown or
    /// hidden as she was, never a beat on (gone at once) or back.
    #[test]
    fn her_door_turned_into_one_in_space_keeps_its_beat() {
        let terrain = floor_at(15);
        let door = wall_door(20, 15);
        let to = Through::Home(door);
        let gap = 5_000;
        // Late in each beat: past where a door in space's would end.
        let late = |index: usize| {
            let mut start = 0;
            for (i, beat) in DOOR.iter().enumerate() {
                let end = start + beat_ms(beat, to, gap);
                if i == index {
                    return end - 20;
                }
                start = end;
            }
            unreachable!()
        };
        let beat_of = |osaka: &Osaka, now: u64| match osaka.act {
            Act::Door { since, to, gap } => door_at(now - since, gap, to).map(|(i, ..)| i),
            _ => None,
        };
        for index in [2, 5] {
            let now = late(index);
            for how in ["errand", "evict"] {
                let mut rng = Rng(4);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.act = Act::Door { since: 0, to, gap };
                let hidden = osaka.hidden(now);
                assert_eq!(beat_of(&osaka, now), Some(index));
                match how {
                    "errand" => osaka.errand((5, 15), &terrain, now),
                    _ => {
                        assert!(osaka.evict(
                            Rect::new(17, 10, 7, 6),
                            &terrain,
                            &[],
                            None,
                            now,
                            &mut rng
                        ))
                    }
                }
                assert!(
                    matches!(
                        osaka.act,
                        Act::Door {
                            to: Through::Space(_),
                            ..
                        }
                    ),
                    "{how}: {:?}",
                    osaka.act
                );
                // The frame that turned it (as far in, clamped to the
                // beat: the next may go on, as the beat would).
                assert_eq!(beat_of(&osaka, now), Some(index), "{how} in beat {index}");
                assert_eq!(osaka.hidden(now), hidden, "{how} in beat {index}");
            }
        }
    }

    /// Her walk to her door for work follows the frame's door, as
    /// school's does (door batch, step 4a's retarget; step 4b review):
    /// moved to another floor mid-walk, she heads there for work (her
    /// heading's want is `Work`, so the trip is her shift's, as its
    /// credit is), her shift unchanged, and goes through it there for
    /// the shift's gap.
    #[test]
    fn her_walk_to_her_door_for_work_follows_the_frames_door() {
        let level = edge_floors(4);
        let was = wall_door(12, 4);
        let moved = wall_door(30, 4);
        let mut rng = Rng(4);
        let mut osaka = Osaka::standing_at(3, 4, 0, &mut rng);
        osaka.go_to_work(&level, &door_chances(was), 0, &mut rng);
        let Some(Shift::Going { gap }) = osaka.shift else {
            panic!("{:?}", osaka.shift);
        };
        assert!(
            matches!(osaka.act, Act::Walk { then: Then::Job(Job::Leave { spot, why: Leave::Work }), .. } if spot == was),
            "{:?}",
            osaka.act
        );
        let chances = door_chances(moved);
        tick_with(&mut osaka, 0, &level, &chances, &mut rng, 5_000, |o| {
            o.heading.is_some()
        });
        let heading = osaka.heading.as_ref().expect("heading there");
        assert_eq!(heading.want, Want::Work);
        assert!(
            matches!(heading.job, Job::Leave { spot, why: Leave::Work } if spot == moved),
            "{:?}",
            heading.job
        );
        assert_eq!(osaka.shift, Some(Shift::Going { gap }));
        tick_with(&mut osaka, 0, &level, &chances, &mut rng, 60_000, |o| {
            o.through().is_some()
        });
        assert_eq!(osaka.through(), Some(Through::Home(moved)));
        assert!(
            matches!(osaka.act, Act::Door { gap: g, .. } if g == gap),
            "the shift's gap: {:?}",
            osaka.act
        );
    }

    /// Evicted at her door on her way to work, and the pane over her door
    /// stays focused through her shift (door batch, step 4b review, M10):
    /// she comes home where her door in space opened, never at her own
    /// door under the focus (the frame's door, gated, is elsewhere or
    /// nowhere while the door she'd come through, ungated, is under it),
    /// and is never placed in the focused pane coming out.
    #[test]
    fn focused_through_her_shift_she_comes_home_where_she_went() {
        use super::super::door::{Fallback, Set};
        let terrain = floor_at(15);
        let door = wall_door(20, 15);
        let focus = Rect::new(17, 10, 7, 6);
        let fallback = DoorSpot::at(5, 15, Set::Floor(Fallback::Protected));
        for gated in [None, Some(fallback)] {
            let what = format!("gated {:?}", gated.map(DoorSpot::spot));
            let mut rng = Rng(4);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.go_to_work(&terrain, &door_chances(door), 0, &mut rng);
            let mut now = 300;
            osaka.tick(now, None, &terrain, &door_chances(door), &mut rng);
            assert!(
                osaka.evict(focus, &terrain, &[], None, now, &mut rng),
                "{what}"
            );
            let Some(Through::Space(went)) = osaka.through() else {
                panic!("{what}: {:?}", osaka.act);
            };
            assert!(!box_meets(focus, went), "{what}: {went:?}");
            // The frame while the focus stays.
            let focused = Chances {
                door: gated,
                door_through: Some(door),
                ..Chances::default()
            };
            let mut shown = 0;
            while !matches!(osaka.act, Act::Home { .. }) {
                assert!(
                    now < SHIFT_MS.1 + 30_000,
                    "{what}: never home: {:?}",
                    osaka.act
                );
                now += 100;
                osaka.tick(now, None, &terrain, &focused, &mut rng);
                assert!(
                    osaka.evict(focus, &terrain, &[], None, now, &mut rng),
                    "{what}"
                );
                assert_ne!(
                    osaka.through(),
                    Some(Through::Home(door)),
                    "{what}: at {now}"
                );
                // (Going in, she's seen at her door a moment, as any
                // door cut short by the focus.)
                if shown_there(&osaka, now) {
                    shown += 1;
                    assert_eq!((osaka.x, osaka.y), went, "{what}: where she went");
                }
                if matches!(osaka.act, Act::Home { .. }) {
                    assert!(
                        !box_meets(focus, (osaka.x, osaka.y)),
                        "{what}: home in the focus: {:?}",
                        (osaka.x, osaka.y)
                    );
                }
            }
            assert!(shown > 0, "{what}: seen coming home");
            assert_eq!(osaka.speech.map(|(line, _)| line), Some(HOME), "{what}");
        }
    }

    /// An errand on her way to work lets work go, however it finds her
    /// (door batch, step 4b review): walking to her door, round the
    /// screen's edge or through a door in space between floors on her
    /// way, or up a pole. Her shift is cut with nothing worked (nothing
    /// eased), where she was heading for it goes with it, and after the
    /// accordion she never walks on to her door for it (no "work/on",
    /// no door of hers opened with no shift behind it, no homecoming
    /// from work).
    #[test]
    fn an_errand_on_her_way_to_work_lets_work_go() {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::style::Style;
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 14));
        buf.set_string(2, 4, "─".repeat(14), Style::default());
        buf.set_string(24, 12, "─".repeat(14), Style::default());
        let apart = Terrain::read(&buf, &[], false);
        let level = edge_floors(4);
        let floor = floor_at(15);
        type Until = fn(&Osaka) -> bool;
        /// What it is, where, her start, her door, the accordion, and
        /// when on her way the errand comes.
        type Case<'a> = (
            &'a str,
            &'a Terrain,
            (i32, i32),
            (i32, i32),
            (i32, i32),
            Until,
        );
        let walking: Until = |o| {
            o.x >= 8
                && matches!(
                    o.act,
                    Act::Walk {
                        then: Then::Job(Job::Leave {
                            why: Leave::Work,
                            ..
                        }),
                        ..
                    }
                )
        };
        let round: Until = |o| matches!(o.act, Act::Out { .. } | Act::Away { .. });
        let in_space: Until = |o| {
            matches!(
                o.act,
                Act::Door {
                    to: Through::Space(_),
                    gap: 0,
                    ..
                }
            )
        };
        let cases: [Case; 4] = [
            ("walking", &floor, (3, 15), (36, 15), (14, 15), walking),
            ("round the edge", &level, (6, 4), (30, 4), (10, 4), round),
            (
                "a door in space",
                &apart,
                (6, 4),
                (30, 12),
                (10, 4),
                in_space,
            ),
            ("up a pole", &floor, (3, 15), (36, 15), (14, 15), |_| true),
        ];
        for (what, terrain, start, (door_x, door_y), spot, until) in cases {
            let door = wall_door(door_x, door_y);
            let chances = door_chances(door);
            let mut rng = Rng(4);
            let mut osaka = Osaka::standing_at(start.0, start.1, 0, &mut rng);
            osaka.go_to_work(terrain, &chances, 0, &mut rng);
            let mut now = tick_with(&mut osaka, 0, terrain, &chances, &mut rng, 30_000, until);
            if what == "up a pole" {
                // Heading for her door across floors, mid-climb (as
                // `travel` leaves her).
                osaka.heading = Some(Heading {
                    want: Want::Work,
                    job: Job::Leave {
                        spot: door,
                        why: Leave::Work,
                    },
                });
                osaka.hopping = true;
                osaka.y = 12;
                osaka.set(Act::Climb { to_y: 15 }, now);
                assert!(osaka.aloft(), "{what}");
            }
            assert!(
                matches!(osaka.shift, Some(Shift::Going { .. })),
                "{what}: on her way: {:?} {:?}",
                osaka.shift,
                osaka.act
            );
            let (served, decided) = (osaka.served.len(), osaka.decisions.len());
            osaka.errand(spot, terrain, now);
            // At once (not at her next decision: on the way to the
            // accordion she's not still on her way to work, for her
            // census nor for a delivery).
            assert!(osaka.shift.is_none(), "{what}: {:?}", osaka.shift);
            assert!(!osaka.work_way(), "{what}: {:?}", osaka.heading);
            let bound = now + 30_000;
            while now < bound {
                now += 100;
                osaka.tick(now, None, terrain, &chances, &mut rng);
                assert!(
                    !matches!(
                        osaka.act,
                        Act::Door {
                            to: Through::Home(_),
                            ..
                        }
                    ),
                    "{what}: out by her door at {now}: {:?}",
                    osaka.act
                );
                assert!(
                    osaka.speech.is_none_or(|(line, _)| line != HOME),
                    "{what}: home from work"
                );
            }
            assert!(osaka.poked, "{what}: at the accordion");
            assert!(osaka.shift.is_none(), "{what}: {:?}", osaka.shift);
            assert!(
                osaka
                    .heading
                    .as_ref()
                    .is_none_or(|h| h.job.leave() != Some(Leave::Work)),
                "{what}: {:?}",
                osaka.heading
            );
            let methods: Vec<&str> = osaka.decisions[decided..]
                .iter()
                .map(|d| d.method)
                .collect();
            assert!(!methods.contains(&"work/on"), "{what}: {methods:?}");
            let eased: Vec<(Want, f64, Via)> = osaka.served[served..]
                .iter()
                .map(|s| (s.want, s.share, s.via))
                .collect();
            // Nothing worked, nothing eased.
            assert!(
                !eased.iter().any(|&(want, _, _)| want == Want::Work),
                "{what}: {eased:?}"
            );
        }
    }

    /// Off to work with no door of hers to go to, or no way to it, from
    /// inside a piece (her sofa, her bed): she steps out of it first, and
    /// goes by a door in space where she stands once her box meets none
    /// of her pieces, with her shift's gap (door batch, step 4b review:
    /// every door in space she goes out by follows one rule,
    /// [`Osaka::out_where_clear`]; a door never opens in her bed). Her
    /// step out is her way to work: deciding on it, she goes on.
    #[test]
    fn off_to_work_with_no_way_to_her_door_she_steps_out_of_her_piece_first() {
        let terrain = floor_at(15);
        // Her sofa, where she stands.
        let sofa = Rect::new(16, 13, 10, 3);
        for (what, door) in [
            ("no door", None),
            // Her door's spot on no floor of hers: no way to it.
            ("no way", Some(wall_door(30, 9))),
        ] {
            let chances = Chances {
                door,
                door_through: door,
                obstacles: vec![sofa],
                ..Chances::default()
            };
            let mut rng = Rng(4);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            assert!(box_meets(sofa, (osaka.x, osaka.y)), "{what}");
            osaka.go_to_work(&terrain, &chances, 0, &mut rng);
            assert!(
                matches!(
                    osaka.act,
                    Act::Walk {
                        then: Then::Job(Job::Out {
                            why: Leave::Work,
                            ..
                        }),
                        ..
                    }
                ),
                "{what}: out of her sofa first: {:?}",
                osaka.act
            );
            assert!(osaka.work_way(), "{what}");
            // Her step out come to nothing (deciding on it): she goes on
            // to work, as from any walk for it.
            osaka.decide(100, &terrain, &chances, &mut rng);
            assert_eq!(
                osaka.decisions.last().map(|d| d.method),
                Some("work/on"),
                "{what}"
            );
            assert!(matches!(osaka.shift, Some(Shift::Going { .. })), "{what}");
            tick_with(&mut osaka, 100, &terrain, &chances, &mut rng, 10_000, |o| {
                matches!(o.act, Act::Door { .. })
            });
            let Act::Door {
                to: Through::Space(spot),
                gap,
                ..
            } = osaka.act
            else {
                panic!("{what}: {:?}", osaka.act);
            };
            assert_eq!(spot, (osaka.x, osaka.y), "{what}: where she stands");
            assert!(!box_meets(sofa, spot), "{what}: in her sofa at {spot:?}");
            assert!(
                (SHIFT_MS.0..SHIFT_MS.1).contains(&gap),
                "{what}: her shift: {gap}"
            );
            assert_eq!(osaka.shift, Some(Shift::Out), "{what}");
        }
    }

    /// Landed from a hop on her way to work once her bedtime has come
    /// (door batch, step 4b review): she doesn't go on to it; her shift
    /// is cut, and where she was heading for it goes with it. The same
    /// landing by day goes on to work ("work/on"), the precondition.
    #[test]
    fn landed_on_her_way_to_work_at_bedtime_she_lets_work_go() {
        let terrain = floor_at(15);
        let door = wall_door(36, 15);
        let chances = door_chances(door);
        // Tuesday 02:00 (her night) and 16:00 (an afternoon).
        for (bedtime, hour) in [(true, 2), (false, 16)] {
            let what = format!("at {hour}:00");
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(clock_at(1, hour, 0)), 0);
            // Her hop's landing, on her way to her door for work.
            osaka.shift = Some(Shift::Going { gap: 60_000 });
            osaka.heading = Some(Heading {
                want: Want::Work,
                job: Job::Leave {
                    spot: door,
                    why: Leave::Work,
                },
            });
            osaka.hopping = true;
            osaka.decide(1_000, &terrain, &chances, &mut rng);
            let method = osaka.decisions.last().map(|d| d.method);
            if bedtime {
                assert_ne!(method, Some("work/on"), "{what}");
                assert!(osaka.shift.is_none(), "{what}: {:?}", osaka.shift);
                assert!(!osaka.work_way(), "{what}: {:?}", osaka.act);
            } else {
                assert_eq!(method, Some("work/on"), "{what}");
                assert_eq!(osaka.shift, Some(Shift::Going { gap: 60_000 }), "{what}");
            }
        }
    }

    /// With neither bed nor sofa, her night on the floor is her night as
    /// any is: sleep-talk on its schedule from when she lay down, the
    /// Dream's count begun.
    #[test]
    fn her_night_on_the_floor_is_armed() {
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock_at(1, 2, 0)), 0);
        osaka.decide(1_000, &floor_at(15), &Chances::default(), &mut rng);
        assert_eq!(
            osaka.decisions.last().map(|d| d.method),
            Some("routine/bed-floor")
        );
        assert!(osaka.sleeping());
        assert_eq!(osaka.talk_due(), Some(1_000 + talk_gap(osaka.whims, 0)));
        assert_eq!(osaka.dream_moment(), Some(1_000 + DREAM_AFTER_MS / 6));
    }

    /// Up groggy is only ever in the night: come for the accordion just
    /// before her wake, she's groggy until it passes, then her day
    /// begins; a night's sleep the stage gives her by day gets her up for
    /// the accordion wide awake (no bed to go back to), though a goodbye
    /// as she sleeps it is a sleepy one.
    #[test]
    fn groggy_only_in_the_night() {
        let terrain = floor_at(15);
        let chances = bed_at(25);
        // An errand arrival at 06:58 (Tuesday, up at 07:00).
        let clock = clock_at(1, 6, 58);
        let wake = 2 * 10_000;
        let mut rng = Rng(5);
        let mut osaka = Osaka::standing_at(10, 15, 0, &mut rng);
        osaka.key_days(9, Some(1));
        osaka.read_clock(Some(clock), 0);
        osaka.errand((14, 15), &terrain, 0);
        osaka.groggy_if_night(0);
        assert!(osaka.groggy() && osaka.groggy_at(0) && osaka.drowsy(0));
        assert!(!osaka.awake(0));
        let world = (clock, &terrain, &chances);
        let now = tick_until(&mut osaka, 0, wake + 120_000, world, &mut rng, |o, t| {
            assert_eq!(o.groggy_at(t), t < wake && o.groggy, "at {t}");
        });
        assert!(!osaka.groggy() && !osaka.drowsy(now) && osaka.awake(now));
        // The stage's night's sleep by day (Tuesday 14:00).
        let clock = clock_at(1, 14, 0);
        let mut osaka = Osaka::standing_at(25, 15, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        let seat = chances.seats[0];
        let act = osaka.night_in(seat, 120_000, 0, true);
        osaka.set(act, 0);
        assert!(osaka.sleeping() && osaka.drowsy(1_000), "a sleepy goodbye");
        assert!(osaka.dream_moment().is_none(), "no night by day");
        osaka.errand((20, 15), &terrain, 1_000);
        assert!(!osaka.groggy() && !osaka.drowsy(1_000));
    }

    /// A master seed whose night ending on game `morning` has a midnight
    /// snack at a minute in `range`, and that minute.
    fn snack_night_in(morning: u64, range: std::ops::Range<u16>) -> (u64, u16) {
        (0..10_000)
            .find_map(|master| {
                brain::night_snack(master, morning)
                    .filter(|m| range.contains(m))
                    .map(|m| (master, m))
            })
            .unwrap_or_else(|| panic!("no snack night in {range:?}"))
    }

    /// Her clock at `minute` (since midnight) of game `day`, from
    /// monotonic 0.
    fn clock_at_minute(day: u64, minute: u16) -> routine::Clock {
        let minute = u64::from(minute);
        clock_at(day, minute / 60, minute % 60)
    }

    /// Whether her act is a snack.
    fn snacking(osaka: &Osaka) -> bool {
        matches!(osaka.act, Act::Use { seat, .. } if seat.what == Use::Snack)
    }

    /// A chat line on a night with a midnight snack.
    #[derive(Clone, Copy, Debug)]
    enum NightLine {
        /// `into` ms after her first snack began, her ticks `lag` ms
        /// behind it (the shell reads its clock for her tick and for its
        /// frame apart).
        Into { into: u64, lag: u64 },
        /// As her first snack ends: her ticks up to a moment before its
        /// end, the line `late` ms after it, before her tick for it.
        AsItEnds { late: u64 },
    }

    /// A midnight snack, as it went.
    #[derive(Clone, Copy, Debug)]
    struct SnackSeen {
        since: u64,
        until: u64,
        /// When she left it, and whether a chat line cut it short.
        ended: Option<(u64, bool)>,
    }

    impl SnackSeen {
        /// It ran its course.
        fn had(&self) -> bool {
            self.ended.is_some_and(|(at, _)| at >= self.until)
        }
    }

    /// Her night with a midnight snack (the first of [`snack_night`]'s
    /// on Tuesday morning), her `rng` seeded `seed`, her fridge and her
    /// bed on one floor: asleep, then up for her snack, with `lines` as
    /// they come due, until her wake time. Each snack, and her at the
    /// end.
    fn snack_run(seed: u64, lines: &[NightLine]) -> (Vec<SnackSeen>, Osaka) {
        let terrain = floor_at(15);
        let chances = fridge_and_bed(4);
        let clock = clock_at(1, 0, 0);
        let wake = 7 * 60 * 10_000;
        let (master, _) = snack_night(1);
        let mut rng = Rng(seed);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.key_days(master, Some(1));
        osaka.read_clock(Some(clock), 0);
        let mut now = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
        let mut lines = lines.to_vec();
        let mut snacks: Vec<SnackSeen> = Vec::new();
        let observe = |osaka: &Osaka, snacks: &mut Vec<SnackSeen>, t: u64, chat: bool| {
            let at = match osaka.act {
                Act::Use {
                    seat, since, until, ..
                } if seat.what == Use::Snack => Some((since, until)),
                _ => None,
            };
            if let Some(last) = snacks.last_mut()
                && last.ended.is_none()
                && at.map(|a| a.0) != Some(last.since)
            {
                last.ended = Some((t, chat && t < last.until));
            }
            if let Some((since, until)) = at
                && snacks.last().map(|s| s.since) != Some(since)
            {
                snacks.push(SnackSeen {
                    since,
                    until,
                    ended: None,
                });
            }
        };
        while now < wake - 1 {
            let next = osaka.due().clamp(now + 1, now + 1_000).min(wake - 1);
            let first = snacks.first().copied();
            // The next line, if it comes by `next`: when her ticks reach,
            // and when it's drawn.
            let line = lines.iter().position(|line| match (*line, first) {
                (NightLine::Into { into, .. }, Some(s)) => s.since + into <= next,
                (NightLine::AsItEnds { .. }, Some(s)) => s.ended.is_none() && s.until <= next,
                (_, None) => false,
            });
            let Some(line) = line.map(|i| lines.remove(i)) else {
                osaka.tick(next, Some(clock), &terrain, &chances, &mut rng);
                now = next;
                observe(&osaka, &mut snacks, now, false);
                continue;
            };
            let (ticks, drawn) = match (line, first) {
                (NightLine::Into { into, lag }, Some(s)) => {
                    let at = s.since + into;
                    (at.saturating_sub(lag), at)
                }
                (NightLine::AsItEnds { late }, Some(s)) => (s.until - 1, s.until + late),
                (_, None) => (now, now),
            };
            if ticks > now {
                osaka.tick(ticks, Some(clock), &terrain, &chances, &mut rng);
                now = ticks;
                observe(&osaka, &mut snacks, now, false);
            }
            now = now.max(drawn);
            osaka.look(now, 40, false, &terrain);
            observe(&osaka, &mut snacks, now, true);
        }
        (snacks, osaka)
    }

    /// Up for her midnight snack, she has it once, to its end, whatever
    /// chat lines come, and is back in bed by her wake time: each line
    /// that cuts it short (she looks at the chat), she has it again
    /// after; one that comes as it ends finds it had.
    fn snack_had_once(seed: u64, lines: &[NightLine]) -> Result<(), String> {
        let (snacks, osaka) = snack_run(seed, lines);
        let had = snacks.iter().filter(|s| s.had()).count();
        let cut = snacks
            .iter()
            .filter(|s| s.ended.is_some_and(|e| e.1))
            .count();
        if had != 1 {
            return Err(format!("had {had} times: {snacks:?}"));
        }
        if had + cut != snacks.len() {
            return Err(format!("left short, not by a line: {snacks:?}"));
        }
        if !osaka.sleeping() || osaka.snacking.is_some() {
            return Err(format!("not back in bed, or owed it: {snacks:?}"));
        }
        Ok(())
    }

    /// A chat line as her midnight snack begins, or a moment into it,
    /// doesn't cost her the snack: she looks at the chat (awake, at her
    /// fridge), then has it after all, to its end, before she goes back
    /// to bed; as one that stops her on her way to the fridge does.
    #[test]
    fn a_chat_line_at_her_midnight_snack_doesnt_cost_her_it() {
        for into in [0u64, 1_000, 3_000] {
            let lines = [NightLine::Into { into, lag: 0 }];
            let (snacks, _) = snack_run(9, &lines);
            assert!(
                snacks.first().is_some_and(|s| s.ended.is_some_and(|e| e.1)),
                "{into}: she looks at the chat: {snacks:?}"
            );
            if let Err(e) = snack_had_once(9, &lines) {
                panic!("{into}: {e}");
            }
        }
    }

    /// A chat line drawn as her midnight snack ends (at its end, or a
    /// moment after, before her tick for it) finds it had: no second
    /// snack.
    #[test]
    fn a_chat_line_as_her_midnight_snack_ends_finds_it_had() {
        for late in [0u64, 1, 300] {
            if let Err(e) = snack_had_once(9, &[NightLine::AsItEnds { late }]) {
                panic!("{late} ms late: {e}");
            }
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(
            dessplay_core::test_support::proptest_cases(64)
        ))]

        /// [`snack_had_once`], whatever chat lines come, wherever in her
        /// snack or after, her ticks lagging each as the shell's may.
        #[test]
        fn her_midnight_snack_is_had_once_whatever_the_chat(
            seed in proptest::prelude::any::<u64>(),
            into in proptest::collection::vec((0u64..12_000, 0u64..400), 0..4),
            ends in proptest::option::of(0u64..400),
        ) {
            let mut lines: Vec<NightLine> = into
                .into_iter()
                .map(|(into, lag)| NightLine::Into { into, lag })
                .collect();
            lines.extend(ends.map(|late| NightLine::AsItEnds { late }));
            if let Err(e) = snack_had_once(seed, &lines) {
                proptest::prop_assert!(false, "{lines:?}: {e}");
            }
        }
    }

    /// The midnight snack is never had late: tucked in after its moment
    /// (a visit begun later that night), she sleeps through; up for the
    /// accordion as it comes (and so not asleep for it: no snack due, no
    /// sleep-talk, while she's up), back in bed she doesn't have it
    /// after all.
    #[test]
    fn the_midnight_snack_is_never_had_late() {
        let terrain = floor_at(15);
        let chances = fridge_and_bed(4);
        let (master, minute) = snack_night_in(1, 60..240);
        // Tucked in half an hour after it.
        let clock = clock_at_minute(1, minute + 30);
        let mut rng = Rng(9);
        let mut osaka = Osaka::standing_at(10, 15, 0, &mut rng);
        osaka.key_days(master, Some(1));
        osaka.read_clock(Some(clock), 0);
        assert!(osaka.tuck_in(&chances, &terrain, 0));
        assert!(osaka.night.is_some_and(|n| n.snack.is_none()));
        tick_until(
            &mut osaka,
            0,
            600_000,
            (clock, &terrain, &chances),
            &mut rng,
            |o, t| {
                assert!(o.sleeping(), "up at {t}: {:?}", o.act);
            },
        );
        // Up for the accordion as it comes.
        let clock = clock_at_minute(1, minute - 5);
        let moment = 50_000;
        let mut rng = Rng(9);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.key_days(master, Some(1));
        osaka.read_clock(Some(clock), 0);
        let world = (clock, &terrain, &chances);
        let asleep = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
        assert_eq!(osaka.snack_due(), Some(moment));
        let now = tick_until(
            &mut osaka,
            asleep,
            moment - 1_000,
            world,
            &mut rng,
            |_, _| {},
        );
        osaka.errand((2, 15), &terrain, now);
        let mut back = None;
        tick_until(
            &mut osaka,
            now,
            moment + 600_000,
            world,
            &mut rng,
            |o, t| {
                assert!(!snacking(o), "a snack at {t}");
                if o.sleeping() {
                    back.get_or_insert(t);
                } else {
                    assert!(back.is_none(), "up again at {t}");
                    assert!(o.snack_due().is_none() && o.talk_due().is_none(), "at {t}");
                    assert!(!o.awake(t), "at {t}");
                }
            },
        );
        let back = back.unwrap_or_else(|| panic!("never back in bed"));
        assert!(back > moment, "back at {back}, before {moment}");
    }

    /// Her midnight snack is had only at a fridge she could get to: one
    /// off any floor, or one she made (makeshift), isn't one; on a snack
    /// night with only those, she sleeps through.
    #[test]
    fn the_midnight_snack_needs_a_real_fridge_on_a_floor() {
        let terrain = floor_at(15);
        let (master, minute) = snack_night_in(1, 60..240);
        let clock = clock_at_minute(1, minute - 5);
        let mut chances = bed_at(25);
        chances.seats.push(Seat {
            x: 4,
            y: 10,
            ..seat_for(Use::Snack, Furniture::Fridge)
        });
        chances.seats.push(Seat {
            x: 8,
            y: 15,
            piece: PieceRef::Made(MadeId(0)),
            ..seat_for(Use::Snack, Furniture::Fridge)
        });
        let mut rng = Rng(9);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.key_days(master, Some(1));
        osaka.read_clock(Some(clock), 0);
        let asleep = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
        assert_eq!(osaka.snack_due(), Some(50_000));
        let mut changed = Vec::new();
        let mut now = asleep;
        while now < 120_000 {
            now = osaka.due().clamp(now + 1, now + 1_000).min(120_000);
            if osaka.tick(now, Some(clock), &terrain, &chances, &mut rng) {
                changed.push(now);
            }
            assert!(osaka.sleeping(), "up at {now}: {:?}", osaka.act);
        }
        assert!(
            osaka.night.is_some_and(|n| n.snack.is_none()),
            "tonight's chance gone"
        );
        assert!(!changed.contains(&50_000), "nothing to show for it");
    }

    /// Her sleep-talk's lines are each drawn from the night act's
    /// starting whims labelled by the line's place in it (a fresh draw
    /// for each, not one draw for all): replayed from those whims, the
    /// same lines at the same times.
    #[test]
    fn her_sleep_talk_draws_each_line_by_its_place() {
        let terrain = floor_at(15);
        let chances = bed_at(25);
        let clock = clock_at(1, 0, 0);
        let wake = 7 * 60 * 10_000;
        for seed in 0..4 {
            let mut rng = Rng(seed);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(clock), 0);
            let now = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
            let (start, whims) = (osaka.act_since, osaka.whims);
            let world = (clock, &terrain, &chances);
            tick_until(
                &mut osaka,
                now,
                wake - LAST_HOUR_MS - 1,
                world,
                &mut rng,
                |_, _| {},
            );
            let said = sleep_talk(&osaka);
            let mut lines = Lines::default();
            let mut want = Vec::new();
            let mut t = start;
            for k in 0.. {
                t += talk_gap(whims, k);
                if t >= wake - LAST_HOUR_MS {
                    break;
                }
                if let Some(line) = lines.pick(mind::SLEEP_TALK, whims.series("sleep-talk", k), t) {
                    want.push((t, line));
                }
            }
            assert_eq!(said, want, "seed {seed}");
            let distinct: std::collections::HashSet<_> = said.iter().map(|&(_, l)| l).collect();
            assert!(distinct.len() > 2, "seed {seed}: {said:?}");
        }
    }

    // ---- Her calendar, and the times of her day ----

    /// Her clock at `h:m` of game `day` from monotonic 0, on the real
    /// `date`.
    fn clock_dated(day: u64, h: u64, m: u64, date: Option<chrono::NaiveDate>) -> routine::Clock {
        let game = ((day * 24 + h) * 60 + m - routine::START) * 60_000;
        routine::Clock::read(super::super::GameClock { at: 0, game }, 0, None, date)
    }

    fn ymd(y: i32, m: u32, d: u32) -> Option<chrono::NaiveDate> {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
    }

    /// What she said from `pool`.
    fn said_from(osaka: &Osaka, pool: PoolId) -> Vec<&'static str> {
        osaka
            .said_lines()
            .iter()
            .filter(|&&(p, ..)| p == pool)
            .map(|&(_, line, _)| line)
            .collect()
    }

    /// Her on Halloween at Monday 16:00, at (20, 15), her calendar's
    /// greeting owed.
    fn on_halloween(rng: &mut Rng) -> Osaka {
        let mut osaka = Osaka::standing_at(20, 15, 0, rng);
        osaka.read_clock(Some(clock_dated(0, 16, 0, ymd(2026, 10, 31))), 0);
        osaka
    }

    /// Her calendar's line is delivered only by a frame that draws it,
    /// she in sight: not by a frame drawing no bubble (no room for it),
    /// nor another bubble over it (what she says of her home), nor one
    /// with her out of sight; then, drawn, it is, recorded once.
    #[test]
    fn her_calendars_line_is_delivered_only_as_drawn() {
        let mut rng = Rng(1);
        let mut osaka = on_halloween(&mut rng);
        let greeting = osaka.calendar_greeting(0).expect("owed");
        let line = greeting.2;
        assert_eq!(line, calendar::HALLOWEEN);
        osaka.say_calendar(greeting, 0);
        assert_eq!(osaka.shown(100, None), None, "squeezed out");
        assert_eq!(osaka.shown(200, Some(Bubble::Dots)), None, "under another");
        osaka.act = Act::Away {
            until: 10_000,
            enter: 0,
            to_y: 15,
            to_x: 20,
        };
        assert!(osaka.hidden(300));
        assert_eq!(osaka.shown(300, Some(Bubble::Say(line))), None, "hidden");
        osaka.act = Act::Stand { until: 10_000 };
        assert!(osaka.events.is_empty());
        assert_eq!(osaka.cal_done, None);
        let date = ymd(2026, 10, 31);
        assert_eq!(osaka.shown(400, Some(Bubble::Say(line))), date);
        assert_eq!(osaka.events, [HomeEvent::Calendar(date.expect("date"))]);
        assert_eq!(osaka.calendar_owed(500), None, "once a day");
        assert_eq!(osaka.shown(500, Some(Bubble::Say(line))), None);
        assert_eq!(osaka.events.len(), 1);
    }

    /// Said and never seen, her calendar's greeting is said again only
    /// once she has moved from where it went unseen, never over and over
    /// in place; and after [`CAL_MISSES`] sayings that frames missed she
    /// lets it be for the visit (owed still). Unpainted sayings (no frame
    /// at all) don't count against it.
    #[test]
    fn an_unseen_calendar_line_is_said_again_elsewhere_not_forever() {
        let mut rng = Rng(1);
        let mut osaka = on_halloween(&mut rng);
        let mut at = 0;
        // Unpainted: said, then said again elsewhere, and again.
        for x in [20, 24, 28, 32] {
            osaka.x = x;
            let greeting = osaka.calendar_greeting(at).expect("owed");
            osaka.say_calendar(greeting, at);
            at += 10_000;
            assert_eq!(osaka.calendar_greeting(at), None, "{x}: not in place");
        }
        assert_eq!(osaka.cal_missed, None);
        // Painted, and missed: each time elsewhere, until she lets it be.
        for k in 0..CAL_MISSES {
            osaka.x = 40 + i32::from(k);
            let greeting = osaka
                .calendar_greeting(at)
                .unwrap_or_else(|| panic!("{k}: owed"));
            osaka.say_calendar(greeting, at);
            assert_eq!(osaka.shown(at + 100, None), None);
            assert_eq!(osaka.shown(at + 200, None), None, "one miss a saying");
            at += 10_000;
            assert_eq!(osaka.calendar_greeting(at), None, "{k}: not in place");
        }
        assert_eq!(
            osaka.cal_missed,
            Some((
                chrono::NaiveDate::from_ymd_opt(2026, 10, 31).expect("date"),
                CAL_MISSES
            ))
        );
        osaka.x = 10;
        assert_eq!(osaka.calendar_greeting(at), None, "let be");
        assert!(osaka.calendar_owed(at).is_some(), "owed still");
        assert_eq!(osaka.cal_done, None);
    }

    /// A gift on her doorstep (her wall clock) waits for her calendar to
    /// settle, and only that long: not while its greeting is owed, being
    /// said, or waiting to be said again elsewhere (unseen where she
    /// stood); free once it's delivered, or once she lets it be for the
    /// visit ([`CAL_MISSES`] sayings missed, owed still); not while
    /// Setsubun's beans are owed, nor while they're thrown (delivered on
    /// their first frame, they play on to their end first).
    #[test]
    fn a_gift_waits_for_her_calendar_to_settle() {
        let terrain = floor_at(15);
        let none = Chances::default();
        let free = |osaka: &Osaka, at: u64| osaka.free_for_a_gift(at, &none, &terrain);
        let mut rng = Rng(1);
        // Let be: missed CAL_MISSES times, each elsewhere.
        let mut osaka = on_halloween(&mut rng);
        osaka.greeted = true;
        let mut at = 0;
        assert!(!free(&osaka, at), "owed");
        for k in 0..CAL_MISSES {
            osaka.x = 40 + i32::from(k);
            let greeting = osaka
                .calendar_greeting(at)
                .unwrap_or_else(|| panic!("{k}: owed"));
            osaka.say_calendar(greeting, at);
            assert!(!free(&osaka, at + 50), "{k}: being said");
            assert_eq!(osaka.shown(at + 100, None), None);
            at += 10_000;
            // Unseen where she stood: said again once she's moved.
            assert_eq!(osaka.calendar_greeting(at), None);
            assert_eq!(
                free(&osaka, at),
                k + 1 == CAL_MISSES,
                "{k}: let be only after {CAL_MISSES}"
            );
        }
        assert!(osaka.calendar_owed(at).is_some(), "owed still");
        // Delivered.
        let mut osaka = on_halloween(&mut rng);
        osaka.greeted = true;
        let greeting = osaka.calendar_greeting(0).expect("owed");
        osaka.say_calendar(greeting, 0);
        assert!(osaka.shown(100, Some(Bubble::Say(greeting.2))).is_some());
        let quiet = osaka.speech.map_or(0, |(_, until)| until);
        assert!(!free(&osaka, quiet - 1), "still saying it");
        assert!(free(&osaka, quiet), "delivered");
        // Setsubun: owed, then thrown, then done.
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock_dated(0, 16, 0, ymd(2027, 2, 3))), 0);
        osaka.greeted = true;
        assert!(!free(&osaka, 0), "beans owed");
        let here = terrain.platform_at(20, 15).expect("floor");
        let beat = osaka.calendar_beat(here, &terrain, &none, 0);
        assert_eq!(beat.map(|d| d.method), Some("calendar"));
        assert!(osaka.shown(100, None).is_some(), "delivered as they play");
        assert!(!free(&osaka, 200), "thrown");
        osaka.act = Act::Stand { until: u64::MAX };
        assert!(free(&osaka, SETSUBUN_MS + 1), "done");
        // New Year's Day: greeted, then her first sunrise, if she can get
        // to her TV for it; not while she's on her way to it.
        let tv = Chances {
            seats: vec![Seat {
                x: 30,
                y: 15,
                ..seat_for(Use::Watch, Furniture::Tv)
            }],
            ..Chances::default()
        };
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock_dated(0, 16, 0, ymd(2027, 1, 1))), 0);
        osaka.greeted = true;
        let greeting = osaka.calendar_greeting(0).expect("owed");
        osaka.say_calendar(greeting, 0);
        assert!(osaka.shown(100, Some(Bubble::Say(greeting.2))).is_some());
        let quiet = osaka.speech.map_or(0, |(_, until)| until);
        assert!(free(&osaka, quiet), "no TV to get to: the greeting is all");
        assert!(
            !osaka.free_for_a_gift(quiet, &tv, &terrain),
            "to her TV next"
        );
        let go = osaka.calendar_beat(here, &terrain, &tv, quiet);
        assert_eq!(go.map(|d| d.method), Some("calendar/sunrise"));
        assert!(!osaka.free_for_a_gift(quiet, &tv, &terrain), "on her way");
        assert!(!free(&osaka, quiet), "on her way, whatever the frame");
        osaka.act = Act::Stand { until: u64::MAX };
        assert!(free(&osaka, quiet), "she let it go");
        assert!(osaka.sunrise_here(None, quiet + 1), "hers as she watches");
        assert!(osaka.free_for_a_gift(quiet + 1, &tv, &terrain), "seen");
    }

    /// Her calendar's beat never cuts into a move of her home under way
    /// (a piece she's trying, or making for): it waits for it to settle.
    #[test]
    fn the_calendar_beat_waits_for_a_move_to_settle() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(1);
        let mut osaka = on_halloween(&mut rng);
        let here = terrain.platform_at(20, 15).expect("floor");
        osaka.episode = Some(trial_episode());
        assert!(osaka.calendar_beat(here, &terrain, &chances, 0).is_none());
        assert!(osaka.cal_showing.is_none(), "nothing said");
        osaka.episode = None;
        let beat = osaka.calendar_beat(here, &terrain, &chances, 0);
        assert_eq!(beat.map(|d| d.method), Some("calendar"));
        // Its greeting, then a moment spacing out: as long as it's said,
        // or 6 s (its own pause, not a daydream's: phase 5c step 8c's
        // review).
        let said = osaka.speech.map(|(line, _)| line).expect("greeting");
        assert!(speech_ms(said) < GREETING_PAUSE_MS, "{said}");
        let Act::SpaceOut {
            since,
            until,
            play: None,
            session: None,
        } = osaka.act
        else {
            panic!("spacing out: {:?}", osaka.act);
        };
        assert_eq!((since, until), (0, GREETING_PAUSE_MS));
        assert_eq!(GREETING_PAUSE_MS, 6000);
    }

    /// New Year's Day's sunrise follows its greeting shown, to her TV if
    /// it's where she can get to it: with no TV seat, nothing (and no
    /// standing about waiting to go to it while she talks); with one,
    /// she waits out what she's saying, then goes, once.
    #[test]
    fn her_first_sunrise_wants_a_tv_she_can_get_to() {
        let terrain = floor_at(15);
        let tv = Chances {
            seats: vec![Seat {
                x: 30,
                y: 15,
                ..seat_for(Use::Watch, Furniture::Tv)
            }],
            ..Chances::default()
        };
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock_dated(0, 16, 0, ymd(2027, 1, 1))), 0);
        let here = terrain.platform_at(20, 15).expect("floor");
        let greeting = osaka.calendar_greeting(0).expect("owed");
        assert_eq!(greeting.2, calendar::HAPPY_NEW_YEAR);
        osaka.say_calendar(greeting, 0);
        assert!(osaka.shown(100, Some(Bubble::Say(greeting.2))).is_some());
        assert_eq!(osaka.calendar_owed(100), None, "delivered");
        // Talking, with no TV: nothing to wait for.
        osaka.say(mind::HOME.lines[0], 1000);
        let none = Chances::default();
        assert!(osaka.calendar_beat(here, &terrain, &none, 1100).is_none());
        // With one: wait, then to it; then not again.
        let wait = osaka.calendar_beat(here, &terrain, &tv, 1100);
        assert_eq!(wait.map(|d| d.method), Some("calendar/wait"));
        let quiet = osaka.speech.map_or(0, |(_, until)| until);
        let go = osaka.calendar_beat(here, &terrain, &tv, quiet);
        assert_eq!(go.map(|d| d.method), Some("calendar/sunrise"));
        assert!(
            osaka
                .calendar_beat(here, &terrain, &tv, quiet + 1)
                .is_none()
        );
        // Any watch of hers that date plays it, once.
        assert!(osaka.sunrise_here(None, quiet + 2));
        assert!(!osaka.sunrise_here(None, quiet + 3));
    }

    /// Her seasons' musings come only in their seasons (her calendar's
    /// tints): December's in December, and "Homework! Homework!" in
    /// summer's panic week; in season, now and then (a musing's lines
    /// still come the rest of the time); never with no date, nor with
    /// her routine unfed (she knows no date then).
    #[test]
    fn her_seasons_musings_come_only_in_their_seasons() {
        for (date, fed, december, panic) in [
            (ymd(2026, 12, 10), true, true, false),
            (ymd(2027, 12, 31), true, true, false),
            (ymd(2026, 8, 27), true, false, true),
            (ymd(2026, 11, 30), true, false, false),
            (ymd(2027, 1, 1), true, false, false),
            (ymd(2026, 8, 24), true, false, false),
            (None, true, false, false),
            (ymd(2026, 12, 10), false, false, false),
        ] {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(fed.then(|| clock_dated(0, 16, 0, date)), 0);
            for k in 0..240 {
                osaka.whims = Whims(k ^ 0xca1e);
                osaka.speech = None;
                // Ten minutes and more apart: nothing cools.
                osaka.muse(k * 11 * 60_000, &mut rng);
            }
            let at = format!("{date:?} fed {fed}");
            let seasonal = said_from(&osaka, PoolId::December);
            assert_eq!(!seasonal.is_empty(), december, "{at}: {seasonal:?}");
            let panicked = said_from(&osaka, PoolId::Panic);
            assert_eq!(!panicked.is_empty(), panic, "{at}: {panicked:?}");
            assert!(seasonal.iter().all(|l| mind::DECEMBER.lines.contains(l)));
            assert!(!said_from(&osaka, PoolId::Musing).is_empty(), "{at}");
            if december {
                let distinct: std::collections::HashSet<_> = seasonal.iter().collect();
                assert_eq!(distinct.len(), mind::DECEMBER.lines.len(), "{at}");
            }
        }
    }

    /// Through the New Year (Jan 1-3) a dream joins her sleep-talk,
    /// "Pigtails... flying...": some night over a few seeds, and never
    /// on any other date.
    #[test]
    fn the_new_years_dream_joins_her_sleep_talk() {
        const PIGTAILS: &str = "Pigtails... flying...";
        let terrain = floor_at(15);
        let chances = bed_at(25);
        for (date, dreams) in [
            (ymd(2027, 1, 1), true),
            (ymd(2027, 1, 3), true),
            (ymd(2027, 1, 4), false),
            (ymd(2026, 12, 31), false),
            (None, false),
        ] {
            let clock = clock_dated(1, 0, 0, date);
            let mut dreamt = 0;
            for seed in 0..6 {
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.read_clock(Some(clock), 0);
                let now = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
                // Her wake (later on a day of the winter vacation).
                let (.., wake) = osaka.night_play().expect("asleep");
                tick_until(
                    &mut osaka,
                    now,
                    wake - 1,
                    (clock, &terrain, &chances),
                    &mut rng,
                    |_, _| {},
                );
                let said = sleep_talk(&osaka);
                dreamt += said.iter().filter(|&&(_, l)| l == PIGTAILS).count();
                // In the last hour, its lines alone, New Year or no;
                // before it, never them.
                for &(when, line) in &said {
                    let last_hour = when + LAST_HOUR_MS >= wake;
                    assert_eq!(
                        mind::LAST_HOUR_TALK.lines.contains(&line),
                        last_hour,
                        "{date:?} seed {seed} at {when}: {line:?}"
                    );
                }
                assert!(
                    said.iter()
                        .all(|&(_, l)| mind::NEW_YEAR_TALK.lines.contains(&l)
                            || mind::LAST_HOUR_TALK.lines.contains(&l)),
                    "{date:?}: {said:?}"
                );
            }
            assert_eq!(dreamt > 0, dreams, "{date:?}: {dreamt}");
        }
    }

    /// Her first snack of a morning or an evening by her routine has its
    /// meal's line ("Breakfast!", "Dinner time~"), once a slot; no other
    /// snack, no other slot, no trial sit; never with her routine unfed.
    #[test]
    fn her_meal_lines_come_in_their_slots_only() {
        let chances = Chances::default();
        let seat = seat_for(Use::Snack, Furniture::Fridge);
        let (breakfast, dinner) = (mind::BREAKFAST.lines[0], mind::DINNER.lines[0]);
        // Tuesday (a school day), Saturday (a day off).
        for (day, h, m, want) in [
            (1, 7, 30, Some(breakfast)),
            (1, 13, 0, None),
            (1, 18, 30, Some(dinner)),
            (1, 21, 0, None),
            (5, 10, 0, Some(breakfast)),
            (5, 13, 0, None),
            (5, 19, 30, Some(dinner)),
        ] {
            for fed in [true, false] {
                let mut rng = Rng(7);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.splice_rows = &[];
                osaka.read_clock(fed.then(|| clock_dated(day, h, m, None)), 0);
                osaka.start_job(Job::Use(seat), 1000, &chances, &mut rng);
                let at = format!("day {day} {h}:{m:02} fed {fed}");
                let said = osaka.speech.map(|(line, _)| line);
                assert_eq!(said, want.filter(|_| fed), "{at}");
                assert_eq!(
                    said_from(&osaka, PoolId::Routine),
                    want.filter(|_| fed).into_iter().collect::<Vec<_>>(),
                    "{at}"
                );
                // The slot's second snack: nothing.
                osaka.speech = None;
                osaka.start_job(Job::Use(seat), 60_000, &chances, &mut rng);
                assert_eq!(osaka.speech, None, "{at}: again");
                // Trying the fridge where it stands: nothing, ever.
                let mut trying = Osaka::standing_at(10, 10, 0, &mut rng);
                trying.read_clock(fed.then(|| clock_dated(day, h, m, None)), 0);
                trying.episode = Some(trial_episode());
                trying.start_job(Job::Use(seat), 1000, &chances, &mut rng);
                assert!(
                    said_from(&trying, PoolId::Routine).is_empty(),
                    "{at}: a trial sit"
                );
            }
        }
    }

    /// Up on a day with no school (a weekend, a vacation), she says so
    /// once she's quiet after good morning, that morning; on a school
    /// day, or past the morning, never. Her day begins as it does on her
    /// wake ([`Osaka::begin_day`]).
    #[test]
    fn no_school_today_only_on_a_day_off() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let no_school = mind::NO_SCHOOL.lines[0];
        for (day, h, date, off) in [
            (1, 7, ymd(2026, 6, 16), false),
            (5, 9, ymd(2026, 6, 20), true),
            (6, 9, ymd(2026, 6, 21), true),
            // A Tuesday in the summer vacation.
            (1, 9, ymd(2026, 7, 28), true),
            (1, 7, None, false),
            // Her day begun past the morning (caught up at 13:00): the
            // morning is gone, and so is the line.
            (5, 13, ymd(2026, 6, 20), false),
            (1, 13, ymd(2026, 7, 28), false),
        ] {
            let clock = clock_dated(day, h, 0, date);
            let mut rng = Rng(11);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(clock), 0);
            let today = clock.day(0);
            osaka.begin_day(today, 0);
            let mut now = 0;
            while now < 60_000 {
                now += 500;
                osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
            }
            let said = said_from(&osaka, PoolId::Routine);
            let at = format!("day {day} {date:?}");
            assert_eq!(
                said,
                off.then_some(no_school).into_iter().collect::<Vec<_>>(),
                "{at}"
            );
            // Said after good morning, not over it.
            if let Some(&(.., when)) = osaka.said_lines().iter().find(|&&(_, l, _)| l == no_school)
            {
                assert!(when >= osaka.morning_until, "{at}");
            }
        }
    }

    /// Tucked in, she's asleep from the first frame, the lamp off: no
    /// "Night-night..." (she'd have said it settling in).
    #[test]
    fn tucked_in_she_says_no_night_night() {
        let terrain = floor_at(15);
        let chances = bed_at(25);
        let clock = clock_at(1, 0, 0);
        let mut rng = Rng(2);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        assert!(osaka.tuck_in(&chances, &terrain, 0));
        tick_until(
            &mut osaka,
            0,
            60_000,
            (clock, &terrain, &chances),
            &mut rng,
            |_, _| {},
        );
        assert!(osaka.lamp_off);
        assert!(said_from(&osaka, PoolId::Routine).is_empty());
    }

    /// In exam season her homework's chopsticks come every time, cooling
    /// aside (A23): its row's chance reads the season the use starts in.
    #[test]
    fn exam_season_chopsticks_wrap_her_homework() {
        let chances = Chances::default();
        let seat = seat_for(Use::Homework, Furniture::Desk);
        for (date, every) in [
            (ymd(2027, 2, 10), true),
            (ymd(2027, 4, 10), false),
            (None, false),
        ] {
            let mut wrapped = 0;
            for seed in 0..60 {
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.whims = Whims(seed ^ 0xe4a3);
                osaka.read_clock(Some(clock_dated(1, 20, 30, date)), 0);
                osaka.start_job(Job::Use(seat), 1000, &chances, &mut rng);
                wrapped +=
                    usize::from(osaka.plays().is_some_and(|p| {
                        p.before.is_some_and(|s| s.splice == SpliceId::Chopsticks)
                    }));
            }
            if every {
                assert_eq!(wrapped, 60, "{date:?}");
            } else {
                assert!((5..40).contains(&wrapped), "{date:?}: {wrapped}");
            }
        }
    }

    /// What's rare and open with the Dream new tonight (the only rare
    /// thing her night holds: her pity certain).
    fn dream_open() -> Rares {
        let rares = Rares::draw(
            0,
            &[],
            Pity {
                rare: 360,
                legend: 0,
            },
            routine::SlotSet::of(&[routine::Slot::Asleep]),
        );
        assert_eq!(rares.new_one(), Some(ScriptId::Dream));
        rares
    }

    /// The Dream (step 7): on a night it's open, 30 game minutes after
    /// she first slept (five real minutes), counted across the groggy
    /// errand that got her up and back to bed meanwhile, her night's
    /// sleep turns to it in place: in bed, the lamp off, her wake time as
    /// it was, "Hello everynyan...", "Fine sankyu...", "Oh my gah!" in
    /// turn, then asleep as before. Seen as its first line plays, once.
    /// Once a night: another errand after it, and back in bed, no second
    /// one before her wake. On a night it isn't open, none at all.
    #[test]
    fn the_dream_comes_once_a_night_across_a_groggy_errand() {
        use script::{DREAM_LINE_MS, EVERYNYAN, OH_MY_GAH, SANKYU};
        let terrain = floor_at(15);
        let clock = clock_at(1, 2, 0);
        let wake = 5 * 60 * 10_000;
        let chances = bed_at(25);
        for open in [true, false] {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(clock), 0);
            let rares = if open { dream_open() } else { Rares::none() };
            osaka.set_rares(rares, Some(1), Vec::new());
            let asleep = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
            let first = osaka.act_since;
            let dream = clock.game.when(clock.game.at(first) + DREAM_AFTER_MS);
            assert_eq!(dream, first + 300_000, "five real minutes");
            assert_eq!(osaka.dream_moment(), Some(dream));
            // Two minutes asleep, then the accordion.
            let mut now = tick_until(
                &mut osaka,
                asleep,
                asleep + 120_000,
                (clock, &terrain, &chances),
                &mut rng,
                |_, _| {},
            );
            osaka.errand((18, 15), &terrain, now);
            while !osaka.take_poked() {
                assert!(now < asleep + 200_000, "never poked: {:?}", osaka.act);
                now += 100;
                osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
            }
            now = until_asleep_with(&mut osaka, now, clock, &terrain, &chances, &mut rng);
            assert!(now < dream, "back in bed before its moment");
            assert!(osaka.act_since > first, "a night act of its own");
            // Her night through to her wake: every line she says, and
            // each turn to the Dream.
            let mut lines: Vec<(u64, &str)> = Vec::new();
            let mut dreams: Vec<u64> = Vec::new();
            let mut was = osaka.plays().map(|p| p.own);
            let mut errand_again = open;
            let mut seen: Vec<HomeEvent> = osaka.take_events();
            while now < wake - 1 {
                now = osaka.due().clamp(now + 1, now + 1_000).min(wake - 1);
                osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
                seen.extend(osaka.take_events());
                let own = osaka.plays().map(|p| p.own);
                if own == Some(ScriptId::Dream) && was != own {
                    dreams.push(now);
                    assert_eq!(osaka.night_play().map(|(.., until)| until), Some(wake));
                    assert!(osaka.dark(now), "the lamp off");
                }
                was = own;
                if let (_, _, Some(Bubble::Say(line))) = osaka.appearance(now)
                    && lines.last().map(|&(_, l)| l) != Some(line)
                {
                    lines.push((now, line));
                }
                // After the Dream, up for the accordion again.
                if errand_again && !dreams.is_empty() && now > dream + 60_000 && osaka.sleeping() {
                    errand_again = false;
                    osaka.errand((18, 15), &terrain, now);
                }
            }
            let dreamt: Vec<&str> = lines
                .iter()
                .map(|&(_, l)| l)
                .filter(|l| [EVERYNYAN, SANKYU, OH_MY_GAH].contains(l))
                .collect();
            let seen: Vec<&HomeEvent> = seen
                .iter()
                .filter(|e| matches!(e, HomeEvent::Seen(_)))
                .collect();
            if open {
                assert_eq!(dreams, [dream], "once, at its moment");
                assert_eq!(dreamt, [EVERYNYAN, SANKYU, OH_MY_GAH], "{lines:?}");
                let at = |line| lines.iter().find(|&&(_, l)| l == line).map(|&(t, _)| t);
                assert_eq!(at(EVERYNYAN), Some(dream));
                assert_eq!(at(SANKYU), Some(dream + DREAM_LINE_MS));
                assert_eq!(at(OH_MY_GAH), Some(dream + 2 * DREAM_LINE_MS));
                assert_eq!(seen, [&HomeEvent::Seen(ScriptId::Dream)]);
                assert_eq!(osaka.seen(), [ScriptId::Dream]);
                assert!(!errand_again, "up again after it");
                assert_eq!(osaka.plays().map(|p| p.own), Some(ScriptId::Night));
            } else {
                assert!(dreams.is_empty() && dreamt.is_empty(), "{lines:?}");
                assert!(seen.is_empty());
            }
        }
    }

    /// A new day's draw (as she wakes into it, or catches up on it), over
    /// a thousand homes and every set of what she has seen: what's new is
    /// never seen, what's open is all seen, and it's the day's own draw,
    /// keyed on the home and the day (the same home and day draw the
    /// same); a day already drawn (a visit begun after midnight) isn't
    /// drawn again, whatever she has seen since, so a game day has at
    /// most one new rare thing.
    #[test]
    fn a_new_day_draws_once_and_news_only_the_unseen() {
        let day = routine::day_time(routine::game_of(3, 7 * 60), false);
        let rows = rarity::RARES.map(|r| r.id);
        let mut news = 0;
        for master in 0..1000u64 {
            let bits = master % 16;
            let seen: Vec<ScriptId> = rows
                .iter()
                .enumerate()
                .filter(|&(i, _)| bits & 1 << i != 0)
                .map(|(_, &id)| id)
                .collect();
            let mut rng = Rng(master);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.master = master;
            osaka.set_rares(Rares::none(), None, seen.clone());
            osaka.set_pity(Pity {
                rare: master % 400,
                legend: 0,
            });
            osaka.begin_day(day, 1_000);
            let rares = osaka.rares().clone();
            assert_eq!(osaka.rares_today().map(|(d, _)| d), Some(3));
            assert!(rares.open().iter().all(|id| seen.contains(id)), "{master}");
            if let Some(new) = rares.new_one() {
                assert!(!seen.contains(&new), "{master}: {new:?}");
                news += 1;
            }
            assert_eq!(
                rares,
                Rares::draw(
                    brain::day_seed(master, 3),
                    &seen,
                    osaka.pity,
                    rarity::DAY_WINDOW
                ),
                "{master}"
            );
            // The same day again (begun after midnight, then woken into):
            // as drawn, whatever she has seen since.
            osaka.seen = rows.to_vec();
            osaka.begin_day(day, 2_000);
            assert_eq!(osaka.rares(), &rares, "{master}");
        }
        assert!(news > 100, "{news}");
    }

    /// The lines she shows (her speech or her act's bubble) as each
    /// first shows, a new one each time it changes.
    fn note_line(lines: &mut Vec<(u64, &'static str)>, osaka: &Osaka, now: u64) {
        if let (_, _, Some(Bubble::Say(line))) = osaka.appearance(now)
            && lines.last().map(|&(_, l)| l) != Some(line)
        {
            lines.push((now, line));
        }
    }

    /// Her tucked in at monotonic 0 in her bed at 25 on [`floor_at`]`(15)`
    /// by `clock` (it must be night by it), the Dream new tonight: her
    /// night's start is the Dream's count's.
    fn tucked_in_dreamy(clock: routine::Clock) -> (Osaka, Rng, Terrain, Chances) {
        let mut rng = Rng(5);
        let terrain = floor_at(15);
        let chances = bed_at(25);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        osaka.set_rares(dream_open(), Some(1), Vec::new());
        assert!(osaka.tuck_in(&chances, &terrain, 0), "tucked in");
        assert_eq!(osaka.dream_moment(), Some(DREAM_AFTER_MS / 6));
        (osaka, rng, terrain, chances)
    }

    /// The Dream on every surface her night is spent on: her bed, her
    /// sofa (napping on it) and the floor (lying back, no seat): at its
    /// moment, her night's own act turns to it in place, on that
    /// surface's branch, its three lines in turn, seen once as it begins.
    #[test]
    fn the_dream_comes_on_every_surface() {
        use script::{DREAM_LINE_MS, DREAM_MS, EVERYNYAN, OH_MY_GAH, SANKYU};
        let terrain = floor_at(15);
        let clock = clock_at(1, 2, 0);
        let sofa = Chances {
            seats: vec![Seat {
                x: 25,
                y: 15,
                facing: Facing::Right,
                ..seat_for(Use::Nap, Furniture::Sofa)
            }],
            ..Chances::default()
        };
        for (surface, chances) in [
            (Surface::Bed, bed_at(25)),
            (Surface::Sofa, sofa),
            (Surface::Floor, Chances::default()),
        ] {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.read_clock(Some(clock), 0);
            osaka.set_rares(dream_open(), Some(1), Vec::new());
            let asleep = until_asleep_with(&mut osaka, 0, clock, &terrain, &chances, &mut rng);
            let dream = osaka.act_since + DREAM_AFTER_MS / 6;
            assert_eq!(osaka.dream_moment(), Some(dream), "{surface:?}");
            let floor = matches!(osaka.act, Act::Idle { .. });
            assert_eq!(
                floor,
                surface == Surface::Floor,
                "{surface:?}: {:?}",
                osaka.act
            );
            let mut lines = Vec::new();
            let mut turned = Vec::new();
            let mut events = osaka.take_events();
            let mut was = osaka.plays().map(|p| p.own);
            tick_until(
                &mut osaka,
                asleep,
                dream + DREAM_MS + 2_000,
                (clock, &terrain, &chances),
                &mut rng,
                |osaka, now| {
                    note_line(&mut lines, osaka, now);
                    let play = osaka.plays();
                    if play.map(|p| p.own) == Some(ScriptId::Dream) && was != Some(ScriptId::Dream)
                    {
                        turned.push((now, play.map(|p| p.branch)));
                    }
                    was = play.map(|p| p.own);
                },
            );
            events.extend(osaka.take_events());
            assert_eq!(
                turned,
                [(dream, Some(surface.dream_branch()))],
                "{surface:?}"
            );
            let dreamt: Vec<(u64, &str)> = lines
                .into_iter()
                .filter(|&(_, l)| [EVERYNYAN, SANKYU, OH_MY_GAH].contains(&l))
                .collect();
            assert_eq!(
                dreamt,
                [
                    (dream, EVERYNYAN),
                    (dream + DREAM_LINE_MS, SANKYU),
                    (dream + 2 * DREAM_LINE_MS, OH_MY_GAH)
                ],
                "{surface:?}"
            );
            let seen: Vec<&HomeEvent> = events
                .iter()
                .filter(|e| matches!(e, HomeEvent::Seen(_)))
                .collect();
            assert_eq!(seen, [&HomeEvent::Seen(ScriptId::Dream)], "{surface:?}");
            assert!(osaka.sleeping(), "{surface:?}: asleep on");
        }
    }

    /// The Dream comes only with room for all of it before she wakes: its
    /// moment 4.5 s short of her wake, no Dream (not one line, not seen),
    /// and tonight's chance of one is spent.
    #[test]
    fn no_dream_without_room_for_it_before_her_wake() {
        let wake = DREAM_AFTER_MS / 6 + 4_500;
        let game = ((24 + 7) * 60 - routine::START) * 60_000 - super::super::CLOCK_SPEED * wake;
        let clock = routine::Clock::read(super::super::GameClock { at: 0, game }, 0, None, None);
        let (mut osaka, mut rng, terrain, chances) = tucked_in_dreamy(clock);
        assert_eq!(osaka.night_play().map(|(.., until)| until), Some(wake));
        let moment = DREAM_AFTER_MS / 6;
        let mut lines = Vec::new();
        let mut spent = false;
        tick_until(
            &mut osaka,
            0,
            wake + 5_000,
            (clock, &terrain, &chances),
            &mut rng,
            |osaka, now| {
                note_line(&mut lines, osaka, now);
                assert_ne!(osaka.plays().map(|p| p.own), Some(ScriptId::Dream), "{now}");
                if (moment..wake).contains(&now) {
                    assert!(osaka.night.is_some_and(|n| n.dreamt), "{now}: spent");
                    assert_eq!(osaka.dream_due(), None, "{now}");
                    spent = true;
                }
            },
        );
        assert!(spent);
        assert!(
            !lines
                .iter()
                .any(|&(_, l)| l == script::EVERYNYAN || l == script::SANKYU),
            "{lines:?}"
        );
        assert!(osaka.seen().is_empty());
        assert!(
            !osaka
                .take_events()
                .iter()
                .any(|e| matches!(e, HomeEvent::Seen(_)))
        );
    }

    /// Her sleep-talk waits for the Dream: a line due a second into it
    /// comes once its last line has been said, and the Dream's three are
    /// shown unbroken.
    #[test]
    fn her_sleep_talk_waits_for_the_dream() {
        use script::{DREAM_LINE_MS, DREAM_MS, EVERYNYAN, OH_MY_GAH, SANKYU};
        let clock = clock_at(1, 2, 0);
        let (mut osaka, mut rng, terrain, chances) = tucked_in_dreamy(clock);
        let dream = DREAM_AFTER_MS / 6;
        if let Some(night) = &mut osaka.night {
            night.next_talk = dream + 1_000;
        }
        let mut lines = Vec::new();
        tick_until(
            &mut osaka,
            0,
            dream + DREAM_MS + 5_000,
            (clock, &terrain, &chances),
            &mut rng,
            |osaka, now| note_line(&mut lines, osaka, now),
        );
        let during: Vec<(u64, &str)> = lines
            .iter()
            .copied()
            .filter(|&(t, _)| (dream..dream + DREAM_MS).contains(&t))
            .collect();
        assert_eq!(
            during,
            [
                (dream, EVERYNYAN),
                (dream + DREAM_LINE_MS, SANKYU),
                (dream + 2 * DREAM_LINE_MS, OH_MY_GAH)
            ]
        );
        let talk = sleep_talk(&osaka);
        assert!(
            talk.first().is_some_and(|&(t, _)| t >= dream + DREAM_MS),
            "{talk:?}"
        );
    }

    /// Its moment passed while she was up (for the accordion), the Dream
    /// comes a little after she's back asleep, not the instant she lies
    /// down: [`DREAM_SETTLE_MS`] on.
    #[test]
    fn the_dream_missed_while_up_waits_for_her_to_settle() {
        let clock = clock_at(1, 2, 0);
        let (mut osaka, mut rng, terrain, chances) = tucked_in_dreamy(clock);
        let dream = DREAM_AFTER_MS / 6;
        let mut now = tick_until(
            &mut osaka,
            0,
            dream - 2_000,
            (clock, &terrain, &chances),
            &mut rng,
            |_, _| {},
        );
        osaka.errand((18, 15), &terrain, now);
        while !osaka.take_poked() {
            assert!(now < dream + 60_000, "never poked: {:?}", osaka.act);
            now += 100;
            osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
        }
        now = until_asleep_with(&mut osaka, now, clock, &terrain, &chances, &mut rng);
        let back = osaka.act_since;
        assert!(back > dream, "back in bed after its moment: {back}");
        let mut turned = None;
        tick_until(
            &mut osaka,
            now,
            back + DREAM_SETTLE_MS + 5_000,
            (clock, &terrain, &chances),
            &mut rng,
            |osaka, now| {
                if turned.is_none() && osaka.plays().map(|p| p.own) == Some(ScriptId::Dream) {
                    turned = Some(now);
                }
            },
        );
        assert_eq!(turned, Some(back + DREAM_SETTLE_MS));
    }

    /// Dreaming re-times only what her night act plays, not her night:
    /// interrupted after the Dream, what she's done of it is counted from
    /// when she lay down to her wake, not from the Dream.
    #[test]
    fn the_dream_keeps_her_nights_share() {
        let clock = clock_at(1, 2, 0);
        let (mut osaka, mut rng, terrain, chances) = tucked_in_dreamy(clock);
        let wake = osaka.night_play().map(|(.., until)| until).unwrap();
        let now = tick_until(
            &mut osaka,
            0,
            600_000,
            (clock, &terrain, &chances),
            &mut rng,
            |_, _| {},
        );
        assert_eq!(osaka.plays().map(|p| p.own), Some(ScriptId::Dream));
        osaka.credited.clear();
        osaka.errand((18, 15), &terrain, now);
        let &(want, done, at) = osaka.credited.last().expect("credited as she gets up");
        assert_eq!((want, at), (Want::Use(Use::Sleep), now));
        let share = now as f64 / wake as f64;
        assert!((done - share).abs() < 1e-9, "{done} of {share}");
    }

    /// Her night is the night's, not the visit's: carried to a later visit
    /// the same night (keyed on its morning), her first night act there
    /// carries it on, the Dream's count from her first sleep, a Dream come
    /// and a snack had staying so; another night's isn't carried.
    #[test]
    fn a_night_carried_to_a_later_visit_carries_on() {
        let clock = clock_at(1, 2, 0);
        let (first, ..) = tucked_in_dreamy(clock);
        let mut was = first.night.expect("armed");
        was.dreamt = true;
        was.snack = None;
        for (carried, same) in [
            (was, true),
            (
                Night {
                    morning: was.morning + 1,
                    ..was
                },
                false,
            ),
        ] {
            let mut rng = Rng(9);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            // Her clock a real minute on: the same night.
            let later = routine::Clock::read(
                super::super::GameClock {
                    at: 0,
                    game: clock.game.at(60_000),
                },
                0,
                None,
                None,
            );
            osaka.read_clock(Some(later), 0);
            osaka.set_rares(dream_open(), Some(1), Vec::new());
            osaka.carry_night(Some(carried));
            assert!(osaka.night.is_none(), "not asleep for arriving");
            assert!(osaka.tuck_in(&bed_at(25), &floor_at(15), 0));
            let night = osaka.night.expect("armed");
            if same {
                assert_eq!(night.first, was.first);
                assert!(night.dreamt && night.snack.is_none());
                assert_eq!(osaka.dream_due(), None);
            } else {
                assert_eq!(night.first, later.game.at(0));
                assert!(!night.dreamt);
                assert!(osaka.dream_due().is_some());
            }
        }
    }

    /// Her rare musing, the escalator: never on a day it isn't open;
    /// open, one quiet musing in three or so (and never over speech),
    /// and not again within its cooldown.
    #[test]
    fn the_escalator_is_mused_only_on_a_day_its_open() {
        // A day it's open: the first whose Rare roll passes its base
        // chance (pity never opens the seen).
        let open = (0..1000)
            .map(|seed| {
                Rares::draw(
                    seed,
                    &[ScriptId::Escalator],
                    Pity::default(),
                    rarity::DAY_WINDOW,
                )
            })
            .find(|r| r.allows(ScriptId::Escalator))
            .expect("a day in seven or so");
        assert!(open.allows(ScriptId::Escalator));
        let escalator = |osaka: &Osaka| matches!(osaka.act, Act::SpaceOut { play: Some(p), .. } if p.own == ScriptId::Escalator);
        let mut mused = 0;
        let n = 300;
        for seed in 0..n {
            for (rares, talking) in [
                (Rares::none(), false),
                (open.clone(), true),
                (open.clone(), false),
            ] {
                let allowed = rares.allows(ScriptId::Escalator) && !talking;
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.whims = Whims(seed);
                osaka.set_rares(rares, Some(0), vec![ScriptId::Escalator]);
                if talking {
                    osaka.say(OK, 0);
                }
                osaka.muse(0, &mut rng);
                if !allowed {
                    assert!(!escalator(&osaka), "seed {seed} talking={talking}");
                    continue;
                }
                if !escalator(&osaka) {
                    continue;
                }
                mused += 1;
                // Its own whims would muse it again: its cooldown says no,
                // until it's past.
                osaka.muse(60_000, &mut rng);
                assert!(!escalator(&osaka), "seed {seed}: again within its cooldown");
                osaka.speech = None;
                osaka.muse(11 * 60_000, &mut rng);
                assert!(escalator(&osaka), "seed {seed}: its cooldown past");
            }
        }
        assert!((n / 4..n * 5 / 12).contains(&mused), "{mused} of {n}");
    }

    /// Her wall clock hung over the floor at row `floor` (its strip
    /// columns 0 to 40), its middle at column `x`.
    fn clock_on(x: i32, floor: i32) -> ClockOn {
        ClockOn {
            x,
            floor,
            from: 0,
            to: 40,
        }
    }

    /// Her routine's first key is a glance up at her wall clock (D7)
    /// when it hangs on the strip she stands on: turned toward it, gazing
    /// up, "Oh! It's late!" at bedtime, "Time for school!" as she leaves,
    /// whatever she was saying stopping for it (she's mid-sentence); then
    /// her reflex goes on, and doesn't glance again that slot (sent
    /// again, by the stage). Without a clock, or with it on another floor
    /// or over another strip of hers on this floor (the next pane's),
    /// her reflex goes on at once.
    #[test]
    fn her_routine_glances_at_her_clock_first() {
        let terrain = floor_at(15);
        // A game minute (ten real seconds) before bedtime, Monday 22:30
        // (to the floor: nothing to lie on), and before school, Tuesday
        // 08:15.
        let cases = [
            (
                monday_at(22, 29),
                10_000,
                ClockGlance::Bed,
                Face::Surprised,
                script::ITS_LATE,
            ),
            (
                clock_at(1, 8, 14),
                10_000,
                ClockGlance::School,
                Face::Happy,
                script::SCHOOL_TIME,
            ),
        ];
        for (clock, due, glance, face, line) in cases {
            for on in [
                None,
                Some(clock_on(5, 15)),
                Some(clock_on(35, 15)),
                Some(clock_on(5, 10)),
                // Over the pane beside hers (past her floor's end), on
                // the same row.
                Some(ClockOn {
                    x: 45,
                    floor: 15,
                    from: 40,
                    to: 80,
                }),
            ] {
                let at = format!("{glance:?} {on:?}");
                let mut rng = Rng(3);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.read_clock(Some(clock), 0);
                let chances = Chances {
                    clock: on,
                    ..Chances::default()
                };
                let seen = on.is_some_and(|c| c.floor == 15 && c.from == 0);
                let routine = |osaka: &Osaka| -> Vec<&'static str> {
                    osaka
                        .decisions
                        .iter()
                        .map(|d| d.method)
                        .filter(|m| m.starts_with("routine/"))
                        .collect()
                };
                let mut now = 0;
                let mut glanced = None;
                while !routine(&osaka).iter().any(|&m| m != "routine/glance") {
                    assert!(now < due + 30_000, "{at}: no routine by {now}");
                    // Saying something as her routine turns.
                    if now == due - 500 {
                        osaka.say(OK, now);
                    }
                    now += 500;
                    osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
                    if osaka
                        .plays()
                        .is_some_and(|p| p.own == ScriptId::ClockGlance)
                        && glanced.is_none()
                    {
                        let toward = on.map(|c| toward(osaka.x, c.x));
                        glanced = Some((now, osaka.plays(), osaka.appearance(now), toward));
                        assert_eq!(Some(osaka.facing), toward, "{at}: not toward it");
                    }
                }
                let methods = routine(&osaka);
                if !seen {
                    assert_eq!(glanced, None, "{at}");
                    assert!(!methods.contains(&"routine/glance"), "{at}: {methods:?}");
                    continue;
                }
                let Some((when, play, look, _)) = glanced else {
                    panic!("{at}: no glance ({methods:?})");
                };
                assert!(when >= due, "{at}: glanced at {when}, before {due}");
                assert_eq!(play.map(|p| p.branch), Some(glance.branch()), "{at}");
                assert_eq!(look, (Pose::Gaze, face, Some(Bubble::Say(line))), "{at}");
                assert_eq!(methods[0], "routine/glance", "{at}: {methods:?}");
                assert!(methods[1] != "routine/glance", "{at}: {methods:?}");
                // Sent again that slot: no second glance.
                let decided = osaka.decisions.len();
                osaka.place(osaka.x, osaka.y, now);
                osaka.tick(now + 1500, Some(clock), &terrain, &chances, &mut rng);
                let again: Vec<&str> = osaka.decisions[decided..]
                    .iter()
                    .map(|d| d.method)
                    .collect();
                assert!(
                    !again.is_empty() && !again.contains(&"routine/glance"),
                    "{at}: {again:?}"
                );
            }
        }
    }

    /// In for the night by any way (here, tucked in), she's past her
    /// glance at the clock: sent to bed again that night (back from her
    /// midnight snack, an errand), she doesn't glance.
    #[test]
    fn in_for_the_night_she_glances_no_more() {
        let terrain = floor_at(15);
        let clock = clock_at(1, 1, 0);
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        let chances = Chances {
            clock: Some(clock_on(5, 15)),
            ..bed_at(25)
        };
        assert!(osaka.tuck_in(&chances, &terrain, 0));
        let decided = osaka.decisions.len();
        osaka.place(20, 15, 1000);
        osaka.tick(2500, Some(clock), &terrain, &chances, &mut rng);
        let methods: Vec<&str> = osaka.decisions[decided..]
            .iter()
            .map(|d| d.method)
            .collect();
        assert!(methods.contains(&"routine/bed"), "{methods:?}");
        assert!(!methods.contains(&"routine/glance"), "{methods:?}");
    }

    /// Of an afternoon, her wall clock where she can see it, the first
    /// daydream she starts quiet (her rare musing aside) is a glance up at
    /// it instead of musing (phase 5c step 8c: no roll), saying the hour
    /// roughly ("Noon-ish." from 12:45, "Five-ish." up to 18:00), turned
    /// toward it; once a visit. Never out of the afternoon, without a
    /// clock, with it on another floor, unfed, or while she's saying
    /// something.
    #[test]
    fn of_an_afternoon_she_glances_at_the_hour() {
        let glance = |osaka: &Osaka| {
            osaka
                .plays()
                .filter(|p| p.own == ScriptId::ClockGlance)
                .map(|p| p.branch)
        };
        let n = 300u64;
        for (day, h, m, hour) in [
            (1, 12, 50, Some(0)),
            (1, 13, 0, Some(1)),
            (0, 16, 30, Some(4)),
            (5, 17, 59, Some(5)),
            (1, 18, 0, None),
            (5, 10, 0, None),
            (1, 21, 0, None),
        ] {
            let mut glanced = 0;
            for seed in 0..n {
                for (on, talking, fed) in [
                    (Some(clock_on(5, 10)), false, true),
                    (None, false, true),
                    (Some(clock_on(5, 3)), false, true),
                    // Over the pane beside hers, on her row.
                    (
                        Some(ClockOn {
                            x: 3,
                            floor: 10,
                            from: 0,
                            to: 8,
                        }),
                        false,
                        true,
                    ),
                    (Some(clock_on(5, 10)), true, true),
                    (Some(clock_on(5, 10)), false, false),
                ] {
                    let at =
                        format!("{day} {h}:{m} seed {seed} {on:?} talking={talking} fed={fed}");
                    let mut rng = Rng(seed);
                    let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                    osaka.whims = Whims(seed);
                    if fed {
                        osaka.read_clock(Some(clock_at(day, h, m)), 0);
                    }
                    osaka.clock_on = on;
                    if talking {
                        osaka.say(OK, 0);
                    }
                    // What she was doing before, credited already.
                    osaka.credit = Some(Want::SpaceOut);
                    osaka.muse(0, &mut rng);
                    let open =
                        hour.is_some() && on.is_some_and(|c| c.seen_from((10, 10)).is_some());
                    let rare = osaka.plays().is_some_and(|p| p.own == ScriptId::Escalator);
                    let Some(branch) = glance(&osaka) else {
                        assert!(!(open && !talking && fed) || rare, "{at}: no glance");
                        continue;
                    };
                    assert!(open && !talking && fed, "{at}");
                    assert_eq!(osaka.credit, None, "{at}: a glance eases nothing");
                    let hour = hour.unwrap_or_default();
                    assert_eq!(branch, 2 + hour, "{at}");
                    assert_eq!(osaka.facing, Facing::Left, "{at}: toward the clock");
                    assert_eq!(
                        osaka.appearance(0),
                        (
                            Pose::Gaze,
                            Face::Curious,
                            Some(Bubble::Say(script::HOURS[usize::from(hour)]))
                        ),
                        "{at}"
                    );
                    glanced += 1;
                    // Once a visit: never again, whatever her whims.
                    for k in 1..20u64 {
                        osaka.whims = Whims(seed ^ (k << 20));
                        osaka.speech = None;
                        osaka.muse(k * 60_000, &mut rng);
                        assert_eq!(glance(&osaka), None, "{at}: again at {k}");
                    }
                }
            }
            match hour {
                // Her rare musing comes first now and then (a musing in
                // three on a day it's open).
                Some(_) => assert!(glanced >= n / 2, "{h}:{m}: {glanced} of {n}"),
                None => assert_eq!(glanced, 0, "{h}:{m}"),
            }
        }
    }

    /// A glance up at her clock eases no daydream, though she chose it
    /// musing (phase 5c D0b): her musing after it, spacing out, eases
    /// them by its share.
    #[test]
    fn a_glance_at_her_clock_eases_no_daydream() {
        let terrain = floor_at(10);
        let clock = clock_at(1, 15, 0);
        let mut rng = Rng(4);
        let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        osaka.offer_only = Some((Want::SpaceOut, "space-out/muse"));
        osaka.cue(Some(Cue::Script(ScriptId::ClockGlance)));
        let mut glanced = None;
        tick_until(
            &mut osaka,
            0,
            300_000,
            (clock, &terrain, &Chances::default()),
            &mut rng,
            |osaka, _| {
                if glanced.is_none() && osaka.plays().map(|p| p.own) == Some(ScriptId::ClockGlance)
                {
                    glanced = Some(osaka.decisions.len() - 1);
                }
            },
        );
        let glanced = glanced.expect("glanced at her clock");
        let mused: Vec<usize> = (glanced..osaka.decisions.len())
            .filter(|&i| osaka.decisions[i].method == "space-out/muse")
            .collect();
        let [chose, next, after, ..] = mused[..] else {
            panic!("mused {mused:?}");
        };
        assert_eq!(chose, glanced);
        let eased = |from: usize, to: usize| -> Vec<(Want, f64)> {
            osaka
                .served
                .iter()
                .filter(|s| from < s.decision && s.decision <= to)
                .map(|s| (s.want, s.share))
                .collect()
        };
        assert_eq!(eased(chose, next), [], "a glance eases nothing");
        let mused = eased(next, after);
        assert!(
            mused
                .iter()
                .any(|&(want, share)| want == Want::SpaceOut && share > 0.0),
            "{mused:?}"
        );
    }

    /// Spacing out eases her daydreams by the share of it she spent: cut
    /// short a quarter of the way in, a quarter; run out, all of it.
    #[test]
    fn spacing_out_eases_her_by_the_share_done() {
        for (left_at, share) in [(5_000, 0.25), (20_000, 1.0)] {
            let mut rng = Rng(4);
            let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
            osaka.credit = Some(Want::SpaceOut);
            osaka.act = Act::SpaceOut {
                since: 0,
                until: 20_000,
                play: None,
                session: None,
            };
            osaka.set(
                Act::Stand {
                    until: left_at + 1000,
                },
                left_at,
            );
            let served: Vec<(Want, f64)> = osaka.served.iter().map(|s| (s.want, s.share)).collect();
            assert_eq!(served.len(), 1, "{served:?}");
            assert_eq!(served[0].0, Want::SpaceOut);
            assert!((served[0].1 - share).abs() < 1e-9, "{served:?}");
        }
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(
            dessplay_core::test_support::proptest_cases(64)
        ))]

        /// What she did eases her needs as they are when it's credited,
        /// not as they were when she chose it: over any stretch since she
        /// last chose, from any level, an act she's credited for (one
        /// settled as she leaves it, her shift as she comes home, a pull
        /// as she finishes it) leaves every need it serves lower at her
        /// next decision than the same act uncredited, unless that need
        /// is spent either way. A need that rose to full while she was at
        /// it is eased all the same.
        #[test]
        fn a_credit_eases_her_needs_as_they_are(
            level in 0.0f64..=1.0,
            stretch in 0u64..600_000,
            how in 0usize..3,
            what in 0usize..Activity::ALL.len(),
        ) {
            let terrain = floor_at(10);
            let at = 1_000 + stretch;
            let want = match how {
                0 => Want::Idle(Activity::ALL[what]),
                1 => Want::Work,
                _ => Want::Pull,
            };
            let after = |credited: bool| {
                let mut rng = Rng(5);
                let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
                for need in Need::ALL {
                    let by = osaka.needs.get(need) - level;
                    osaka.needs.serve(need, by);
                }
                osaka.decided = 0;
                osaka.credit = credited.then_some(want);
                match want {
                    Want::Idle(what) => {
                        osaka.act = Act::Idle {
                            what,
                            since: 0,
                            until: at,
                            play: None,
                        };
                        osaka.set(Act::Stand { until: at + 1000 }, at);
                    }
                    Want::Work => osaka.come_home(at),
                    _ => osaka.finish_pull(at),
                }
                osaka.decide(at, &terrain, &Chances::default(), &mut rng);
                let bucket = osaka.decisions.last().map(|d| d.bucket);
                assert_ne!(bucket, Some(Bucket::Reflex), "{:?}", osaka.decisions);
                osaka.needs
            };
            let (with, without) = (after(true), after(false));
            for &(need, _) in want.def().serves {
                let (with, without) = (with.get(need), without.get(need));
                proptest::prop_assert!(
                    without == 0.0 || with < without,
                    "{want:?}: {need:?} {with} credited, {without} not"
                );
            }
        }
    }

    /// Looking out of the window, she says what she sees of the sky her
    /// clock shows (unfed, the day's: her window's plain look), each of
    /// that sky's lines over her whims, gazing up curious: the stars at
    /// night, the sunset at dusk.
    #[test]
    fn looking_out_she_says_what_the_sky_is() {
        use super::super::art::Sky;
        for (clock, sky) in [
            (Some(clock_at(1, 6, 0)), Sky::Dawn),
            (Some(clock_at(1, 14, 0)), Sky::Day),
            (Some(clock_at(0, 18, 0)), Sky::Dusk),
            (Some(clock_at(0, 20, 0)), Sky::Evening),
            (Some(clock_at(0, 21, 30)), Sky::Night),
            (None, Sky::Day),
        ] {
            let mut said = std::collections::BTreeSet::new();
            for seed in 0..64 {
                let at = format!("{sky:?} seed {seed}");
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.splice_rows = &[];
                osaka.whims = Whims(seed ^ 0x5eed);
                osaka.read_clock(clock, 0);
                osaka.credit = Some(Want::Use(Use::LookOut));
                osaka.start_job(
                    Job::Use(seat_for(Use::LookOut, Furniture::Window)),
                    1000,
                    &Chances::default(),
                    &mut rng,
                );
                let (_, _, _, play) = begun(&osaka);
                assert_eq!(play.own, ScriptId::LookOut, "{at}");
                let (of, line) = script::LOOK_OUT_LINES[usize::from(play.branch)];
                assert_eq!(of, sky, "{at}: {line}");
                assert_eq!(
                    osaka.appearance(1000),
                    (Pose::SillLean, Face::Curious, Some(Bubble::Say(line))),
                    "{at}: leaning on her sill"
                );
                said.insert(line);
            }
            let lines: std::collections::BTreeSet<&str> = script::LOOK_OUT_LINES
                .iter()
                .filter(|(s, _)| *s == sky)
                .map(|&(_, line)| line)
                .collect();
            assert_eq!(said, lines, "{sky:?}: not every line");
            assert!((2..=3).contains(&lines.len()), "{sky:?}");
        }
    }

    /// Cued by the stage, she glances up at her clock at any hour, and
    /// says the hour her clock shows (not the afternoon's only): "Eight-
    /// ish." at 20:00, "Nine-ish." at 09:00, "Three-ish." at 15:00;
    /// unfed, with no clock to tell her, three o'clock. Turned toward the
    /// clock where she can see it; rolled, never out of the afternoon.
    #[test]
    fn a_cued_glance_says_the_hour_her_clock_shows() {
        for (clock, line) in [
            (Some(clock_at(1, 20, 0)), "Eight-ish."),
            (Some(clock_at(1, 9, 10)), "Nine-ish."),
            (Some(clock_at(1, 15, 30)), "Three-ish."),
            (Some(clock_at(1, 0, 20)), "Midnight-ish."),
            (None, "Three-ish."),
        ] {
            for on in [None, Some(clock_on(30, 10))] {
                let at = format!("{clock:?} {on:?}");
                let mut rng = Rng(4);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.read_clock(clock, 0);
                osaka.clock_on = on;
                osaka.cue(Some(Cue::Script(ScriptId::ClockGlance)));
                osaka.muse(0, &mut rng);
                assert_eq!(
                    osaka.plays().map(|p| p.own),
                    Some(ScriptId::ClockGlance),
                    "{at}"
                );
                assert_eq!(osaka.appearance(0).2, Some(Bubble::Say(line)), "{at}");
                if on.is_some() {
                    assert_eq!(osaka.facing, Facing::Right, "{at}: toward it");
                }
                assert_eq!(osaka.cued, None, "{at}");
            }
        }
        assert_eq!(script::HOURS[3], "Three-ish.");
    }

    /// A riddle the stage cued is what her musing is: a glance at her
    /// clock she'd roll of an afternoon (or her rare musing) never takes
    /// its place.
    #[test]
    fn a_cued_riddle_wins_over_a_rolled_glance() {
        let mut glanced = 0;
        for seed in 0..64u64 {
            let mut rng = Rng(seed);
            let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
            osaka.whims = Whims(seed);
            osaka.read_clock(Some(clock_at(1, 15, 0)), 0);
            osaka.clock_on = Some(clock_on(5, 10));
            // Rolled: now and then a glance.
            let mut rolled = osaka.clone();
            rolled.muse(0, &mut rng.clone());
            if rolled.plays().map(|p| p.own) == Some(ScriptId::ClockGlance) {
                glanced += 1;
            }
            osaka.cue(Some(Cue::Script(ScriptId::Riddle)));
            osaka.muse(0, &mut rng);
            assert_eq!(
                osaka.plays().map(|p| p.own),
                Some(ScriptId::Riddle),
                "seed {seed}"
            );
            assert_eq!(osaka.cued, None, "seed {seed}");
        }
        assert!(glanced > 0, "no glance rolled to stand in for");
    }

    /// Out of sight as her routine turns (through a door, out), she
    /// doesn't glance at her clock: her reflex goes on, and her glance is
    /// spent for the slot.
    #[test]
    fn hidden_she_glances_not() {
        let terrain = floor_at(15);
        let clock = clock_at(1, 8, 15);
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.read_clock(Some(clock), 0);
        let chances = Chances {
            clock: Some(clock_on(5, 15)),
            ..Chances::default()
        };
        osaka.take_in(&chances, &terrain, 0);
        osaka.act = Act::Away {
            until: 0,
            enter: 20,
            to_y: 15,
            to_x: 20,
        };
        assert!(osaka.hidden(0));
        osaka.decide(0, &terrain, &chances, &mut rng);
        let methods: Vec<&str> = osaka.decisions.iter().map(|d| d.method).collect();
        assert!(!methods.contains(&"routine/glance"), "{methods:?}");
        assert!(osaka.clock_glanced.is_some(), "spent for the slot");
        assert!(osaka.plays().is_none_or(|p| p.own != ScriptId::ClockGlance));
    }

    /// A line of text to pull, at her feet on [`floor_at`]'s floor 15.
    fn census_pull() -> Pull {
        Pull {
            x: 20,
            y: 15,
            row: 13,
            side: Side::Right,
            cells: vec![22, 23, 24],
            glyphs: "abc".to_owned(),
            gap: 2,
        }
    }

    /// The census counts her setting off, not her moving on: a walk, a
    /// climb or a door started from a still body (standing, looking at
    /// the chat, pulling) is a set-off, and so is the first a decision
    /// starts, whatever she was doing (a door that ended and chose a
    /// walk, a walk that ended and chose another, a hop landed and on to
    /// the next); a second start in one decision is one only from a
    /// still body. Moving on within a trip without a decision (the walk
    /// to a pole, then the climb; off the screen's edge, and back in)
    /// and falling are not. A set-off outside a decision is for what her
    /// chain says at once; one a decision made is for what that decision
    /// chose (a wander is "wander", whatever set her going before).
    /// (The mechanism, with `deciding` set by hand; the real paths are
    /// the tests after it.)
    #[test]
    fn the_census_counts_setting_off_not_moving_on() {
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        let walk = |to| Act::Walk {
            to,
            then: Then::Nothing,
        };
        let door = Act::Door {
            since: 0,
            to: Through::Space((5, 15)),
            gap: 0,
        };
        let look = Act::Look {
            surprised_until: 0,
            until: 0,
        };
        let fall = Act::Fall {
            from_y: 10,
            since: 0,
            to_y: 15,
        };
        let away = Act::Away {
            until: 0,
            enter: 0,
            to_y: 15,
            to_x: 5,
        };
        let out = Act::Out {
            to: 0,
            enter: 0,
            to_y: 15,
            to_x: 5,
        };
        let heave = Act::Pull {
            pull: census_pull(),
            offset: 4,
            goal: 8,
            heaving: true,
        };
        let stand = Act::Stand { until: 0 };
        // (what, the act, deciding (and whether the decision has set
        // her off already), whether it's a set-off)
        let steps: [(&str, Act, Option<bool>, bool); 22] = [
            ("stand → walk", walk(5), None, true),
            (
                "walk → climb (on along the hop)",
                Act::Climb { to_y: 10 },
                None,
                false,
            ),
            ("climb → stand", stand.clone(), None, false),
            ("stand → door", door.clone(), None, true),
            ("door → walk, its decision", walk(5), Some(false), true),
            ("walk → out (off the edge)", out, None, false),
            ("out → away", away, None, false),
            ("away → walk (back in)", walk(5), None, false),
            ("walk → walk, deciding", walk(8), Some(false), true),
            ("walk → door, the same decision", door, Some(true), false),
            ("door → look (a chat line)", look, None, false),
            ("look → walk", walk(5), None, true),
            ("walk → walk, deciding", walk(9), Some(false), true),
            (
                "walk → stand, the same decision",
                stand.clone(),
                Some(true),
                false,
            ),
            (
                "stand → walk, the same decision (from still)",
                walk(8),
                Some(true),
                true,
            ),
            ("walk → fall (no decision)", fall.clone(), None, false),
            ("fall → stand", stand.clone(), None, false),
            ("stand → fall", fall, None, false),
            ("fall → dazed", Act::Dazed { until: 0 }, None, false),
            ("dazed → walk, deciding", walk(5), Some(false), true),
            ("walk → a pull's heave", heave, None, false),
            ("a pull's heave → walk (not a trip)", walk(5), None, true),
        ];
        let mut want = 0;
        for (i, (what, act, deciding, counts)) in steps.into_iter().enumerate() {
            osaka.deciding = deciding;
            osaka.set(act, i as u64 * 1000);
            osaka.deciding = None;
            want += u32::from(counts);
            assert_eq!(osaka.set_offs, want, "{what}");
            assert_eq!(osaka.set_off_log.len(), want as usize, "{what}");
        }
        let bodies: Vec<Body> = osaka.set_off_log.iter().map(|s| s.body).collect();
        let (w, d) = (Body::Walk, Body::Door);
        assert_eq!(bodies, [w, d, w, w, w, w, w, w, w]);
        // Outside a decision, for her chain at once (her arrival, here);
        // inside one, filled in as the decision has its method.
        assert_eq!(osaka.set_off_log[0].purpose, "arrival");
        assert_eq!(osaka.set_off_log[2].purpose, "");

        // A real decision: a wander from a walk that ended (her chain was
        // her arrival) is a set-off, for wandering.
        let terrain = floor_at(15);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.act = walk(20);
        osaka.offer_only = Some((Want::Walk, "walk/along"));
        osaka.decide(1000, &terrain, &Chances::default(), &mut rng);
        assert!(
            matches!(osaka.act, Act::Walk { .. }),
            "{:?}",
            osaka.decisions
        );
        assert_eq!(osaka.set_offs, 1);
        assert_eq!(osaka.set_off_log[0].purpose, "wander");
        assert_eq!(osaka.census_purpose(), "wander");
    }

    /// Her arrival is a set-off (no `set` starts it), for what she came
    /// for: walking in, her arrival; out of a door, her errand, her
    /// routine home, or her dash home. Her errand's way out is the
    /// errand's too. Standing there from the start, or dropping in, is
    /// none.
    #[test]
    fn her_arrival_is_a_set_off_for_what_she_came_for() {
        let mut rng = Rng(1);
        let terrain = floor_at(15);
        let mut walked_in = false;
        for seed in 0..32 {
            let mut rng = Rng(seed);
            let osaka = Osaka::arrive(0, &terrain, 40, &mut rng).expect("somewhere");
            let log: Vec<_> = osaka
                .set_off_log
                .iter()
                .map(|s| (s.body, s.purpose))
                .collect();
            match osaka.act {
                Act::Walk { .. } => {
                    walked_in = true;
                    assert_eq!(log, [(Body::Walk, "arrival")], "seed {seed}");
                }
                _ => assert_eq!(log, [], "seed {seed}: {:?}", osaka.act),
            }
        }
        assert!(walked_in, "she never walked in");
        let errand = Osaka::arrive_for_errand((20, 15), 0, &mut rng);
        let into = Through::Space((20, 15));
        let back = Osaka::back_through_door(into, Routine::School, 0, &mut rng);
        let dash = Osaka::dash_in(into, true, 0, &mut rng);
        for (osaka, what) in [(&errand, "errand"), (&back, "routine"), (&dash, "dash")] {
            let log: Vec<_> = osaka
                .set_off_log
                .iter()
                .map(|s| (s.body, s.purpose))
                .collect();
            assert_eq!(log, [(Body::Door, what)], "{what}");
        }
        assert_eq!(Osaka::standing_at(20, 15, 0, &mut rng).set_offs, 0);
        // The errand done, over text, she leaves by a door: the errand's.
        let mut osaka = errand;
        osaka.errand = None;
        osaka.chain = "errand";
        osaka.set(Act::Stand { until: 0 }, 1000);
        osaka.through_door((5, 15), 2000);
        assert_eq!(osaka.set_off_log.last().map(|s| s.purpose), Some("errand"));
    }

    /// A chat line that stops her is logged with what it cut (nothing, if
    /// she was still; what she was moving for, a walk to her job read
    /// before the line lets go of where she was heading), and the first
    /// set-off after it, a decision's, carries it: a restart. A look, a
    /// watch and nothing else between, so a glance she owes in between
    /// drops it. A line that comes while she's in a door is owed: it
    /// stops her as she next decides and watches, for what she was on
    /// when it came; one whose watch has run out by then never does.
    #[test]
    fn the_census_counts_her_restarts_after_a_chat_line() {
        let mut rng = Rng(1);
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.offer_only = Some((Want::Walk, "walk/along"));
        // Still: a look, her watch, then a wander.
        osaka.look(1000, 30, false, &terrain);
        assert_eq!(osaka.chat_cuts, [(1000, false, None)]);
        let mut now = 1000;
        while osaka.set_offs == 0 {
            now += 250;
            assert!(now < 60_000, "never set off: {:?}", osaka.act);
            osaka.tick(now, None, &terrain, &chances, &mut rng);
        }
        let first = osaka.set_off_log[0];
        assert!(first.at >= 1000 + WATCH_MS, "watched first: {first:?}");
        assert_eq!(
            (first.purpose, first.after_chat),
            ("wander", Some((false, None)))
        );
        // On her way to a job she's heading for: the line cuts it.
        osaka.heading = Some(Heading {
            want: Want::Pull,
            job: Job::Pull(census_pull()),
        });
        osaka.set(
            Act::Walk {
                to: 25,
                then: Then::Job(Job::Pull(census_pull())),
            },
            now,
        );
        osaka.look(now, 30, false, &terrain);
        assert_eq!(osaka.chat_cuts.last(), Some(&(now, false, Some("to text"))));
        // A glance she owes before she's off again: no restart.
        osaka.set(Act::Glance { until: now + 500 }, now);
        osaka.set(
            Act::Walk {
                to: 5,
                then: Then::Nothing,
            },
            now + 500,
        );
        assert_eq!(osaka.set_off_log.last().map(|s| s.after_chat), Some(None));
        // In a door: owed, not yet a cut.
        osaka.heading = None;
        let cuts = osaka.chat_cuts.len();
        now += 10_000;
        osaka.chain = "travel/door";
        osaka.set(
            Act::Door {
                since: now,
                to: Through::Space((5, 15)),
                gap: 0,
            },
            now,
        );
        osaka.look(now, 30, false, &terrain);
        assert_eq!(osaka.chat_cuts.len(), cuts, "in a door");
        // Back out of it within the watch: she stops and watches now.
        osaka.set(Act::Stand { until: 0 }, now + 2000);
        osaka.decide(now + 2000, &terrain, &chances, &mut rng);
        assert_eq!(
            osaka.chat_cuts.last(),
            Some(&(now + 2000, false, Some("travel")))
        );
        assert!(matches!(osaka.act, Act::Stand { .. }), "{:?}", osaka.act);
        // Owed again, but out past the watch: it lapses.
        let cuts = osaka.chat_cuts.len();
        now += 10_000;
        osaka.set(
            Act::Door {
                since: now,
                to: Through::Space((5, 15)),
                gap: 0,
            },
            now,
        );
        osaka.look(now, 30, false, &terrain);
        osaka.set(Act::Stand { until: 0 }, now + WATCH_MS);
        osaka.decide(now + WATCH_MS, &terrain, &chances, &mut rng);
        assert_eq!(osaka.chat_cuts.len(), cuts, "lapsed");
        assert_eq!(osaka.chat_owed, None);
    }

    /// Her ticks from `now`, 100 ms apart, until she has made a decision
    /// after the `n`th (at most 30 s on): that decision.
    fn next_decision(
        osaka: &mut Osaka,
        mut now: u64,
        n: usize,
        terrain: &Terrain,
        rng: &mut Rng,
    ) -> Decision {
        let chances = Chances::default();
        let bound = now + 30_000;
        while osaka.decisions.len() <= n {
            now += 100;
            assert!(now < bound, "no decision: {:?}", osaka.act);
            osaka.tick(now, None, terrain, &chances, rng);
        }
        osaka.decisions[n].clone()
    }

    /// The watch is counted from the line, whatever puts her look off:
    /// a line that comes while she's on a pole is watched once she has
    /// landed only if she lands within 5 s of it, until 5 s after it;
    /// landing later, she lets it go and never turns to it (looking draws
    /// the eye, which is only worth it while the chat is changing). The
    /// same holds for the line she sees from a door (see
    /// `the_census_counts_her_restarts_after_a_chat_line`) and for the
    /// line the andagi answers.
    #[test]
    fn a_line_put_off_by_a_climb_is_watched_only_inside_its_five_seconds() {
        use super::super::sprite::Pose;
        let terrain = floor_at(15);
        // Twelve rungs land her 6.8 s after the line (500 ms a rung, then
        // her 800 ms stand); three, 2.3 s after it.
        for (rows, watched) in [(12, false), (3, true)] {
            let case = format!("{rows} rungs");
            let mut rng = Rng(1);
            let mut osaka = Osaka::standing_at(20, 15 - rows, 0, &mut rng);
            osaka.offer_only = Some((Want::Walk, "walk/along"));
            osaka.set(Act::Climb { to_y: 15 }, 1000);
            osaka.look(1000, 39, false, &terrain);
            assert!(matches!(osaka.act, Act::Climb { .. }), "{case}: climbs on");
            let n = osaka.decisions.len();
            let landed = next_decision(&mut osaka, 1000, n, &terrain, &mut rng);
            assert_eq!(osaka.y, 15, "{case}: landed");
            assert_eq!(landed.at < 1000 + WATCH_MS, watched, "{case}: {landed:?}");
            if !watched {
                assert_ne!(landed.method, "watching chat", "{case}");
                assert_ne!(osaka.appearance(landed.at).0, Pose::Side, "{case}");
                continue;
            }
            assert_eq!(landed.method, "watching chat", "{case}");
            assert_eq!(osaka.facing, Facing::Right, "{case}: turned to the chat");
            assert_eq!(osaka.appearance(landed.at).0, Pose::Side, "{case}");
            let back = next_decision(&mut osaka, landed.at, n + 1, &terrain, &mut rng);
            assert_eq!(
                (back.at, back.method),
                (1000 + WATCH_MS, "walk/along"),
                "{case}: back to her business 5 s after the line"
            );
        }
    }

    /// Over text (line art) a line doesn't stop her: she goes on to the
    /// nearest calm spot at once, and watches from there only if she gets
    /// there within 5 s of the line, until 5 s after it; from further
    /// away, she lets the line go and carries on.
    #[test]
    fn a_line_over_text_is_watched_from_the_calm_spot_only_inside_its_five_seconds() {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::style::Style;
        // Text over the floor's first so many columns: from x = 4, a calm
        // spot 32 cells on (10.7 s at a walk), or 8 (2.7 s).
        for (text, watched) in [(34u16, false), (10, true)] {
            let case = format!("text over {text} columns");
            let mut buf = Buffer::empty(Rect::new(0, 0, 60, 20));
            buf.set_string(0, 15, "─".repeat(60), Style::default());
            for row in 11..15 {
                buf.set_string(0, row, "x".repeat(text.into()), Style::default());
            }
            let terrain = Terrain::read(&buf, &[], true);
            assert!(!terrain.restful(4, 15), "{case}: on text");
            let mut rng = Rng(1);
            let mut osaka = Osaka::standing_at(4, 15, 0, &mut rng);
            osaka.offer_only = Some((Want::Walk, "walk/along"));
            osaka.set(Act::Stand { until: 60_000 }, 0);
            let n = osaka.decisions.len();
            osaka.look(1000, 59, false, &terrain);
            let off = next_decision(&mut osaka, 1000, n, &terrain, &mut rng);
            assert_eq!(off.method, "off text", "{case}: {off:?}");
            let arrived = next_decision(&mut osaka, off.at, n + 1, &terrain, &mut rng);
            assert!(terrain.restful(osaka.x, 15), "{case}: somewhere calm");
            assert_eq!(arrived.at < 1000 + WATCH_MS, watched, "{case}: {arrived:?}");
            if !watched {
                assert_ne!(arrived.method, "watching chat", "{case}");
                continue;
            }
            assert_eq!(arrived.method, "watching chat", "{case}");
            assert_eq!(osaka.facing, Facing::Right, "{case}: turned to the chat");
            let back = next_decision(&mut osaka, arrived.at, n + 2, &terrain, &mut rng);
            assert_eq!(
                (back.at, back.method),
                (1000 + WATCH_MS, "walk/along"),
                "{case}: back to her business 5 s after the line"
            );
        }
    }

    /// A real hop: she sets off once a hop, the walk to the pole and the
    /// climb one trip (the climb no set-off of its own), for travel; and
    /// landed and chosen again, the next hop is the next set-off.
    #[test]
    fn a_hop_is_one_set_off() {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::style::Style;
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        // A box clear of the screen's edges: a pole between its top and
        // its floor is the only way between them.
        buf.set_string(10, 8, format!("┌{}┐", "─".repeat(18)), Style::default());
        for row in 9..15 {
            buf.set_string(10, row, format!("│{}│", " ".repeat(18)), Style::default());
        }
        buf.set_string(10, 15, format!("└{}┘", "─".repeat(18)), Style::default());
        let terrain = Terrain::read(&buf, &[], false);
        assert!(
            terrain.links.iter().all(|l| l.route == Route::Climb),
            "{:?}",
            terrain.links
        );
        let low = terrain
            .platforms
            .iter()
            .max_by_key(|p| p.y)
            .expect("a floor");
        let mut rng = Rng(3);
        let chances = Chances::default();
        let mut osaka = Osaka::standing_at((low.x0 + low.x1) / 2, low.y, 0, &mut rng);
        osaka.offer_only = Some((Want::Travel, "travel/link"));
        let (mut now, mut climbed) = (0, false);
        while osaka.set_offs < 2 {
            now += 100;
            assert!(now < 120_000, "{:?}", osaka.decisions);
            osaka.tick(now, None, &terrain, &chances, &mut rng);
            if matches!(osaka.act, Act::Climb { .. }) {
                climbed = true;
                assert_eq!(osaka.set_offs, 1, "the climb is the hop's");
            }
        }
        assert!(
            climbed,
            "she never climbed: {:?} {:?} {:?}",
            osaka.set_off_log, osaka.decisions, terrain.links
        );
        let log: Vec<_> = osaka.set_off_log.iter().map(|s| s.purpose).collect();
        assert_eq!(log, ["travel", "travel"]);
    }

    /// The floor gone from under her: her fall is an accident's.
    #[test]
    fn the_floor_gone_is_an_accident() {
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.chain = "walk/along";
        assert!(osaka.settle(1000, &floor_at(18)));
        let motion = osaka.census_motion(1000);
        assert_eq!(
            motion.map(|m| (m.body, m.purpose)),
            Some((Body::Fall, "accident")),
            "{:?}",
            osaka.act
        );
        assert_eq!(osaka.set_offs, 0, "falling is no set-off");
    }

    /// What her moving is for, first that holds: a job she's heading for,
    /// or walking to; her shift; her routine (out or home); a dash home;
    /// an errand; then her chain (named: a wander, travel; or as it is).
    #[test]
    fn the_census_reads_what_her_moving_is_for_first_that_holds() {
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.act = Act::Walk {
            to: 25,
            then: Then::Nothing,
        };
        osaka.heading = Some(Heading {
            want: Want::Pull,
            job: Job::Pull(census_pull()),
        });
        osaka.shift = Some(Shift::Going { gap: 60_000 });
        osaka.leaving = Some(Routine::School);
        osaka.returning = Some(Routine::School);
        osaka.dash = Some(Dash::In);
        osaka.errand = Some((5, 15));
        osaka.chain = "walk/along";
        let mut read = vec![osaka.census_purpose()];
        osaka.heading = None;
        osaka.act = Act::Walk {
            to: 25,
            then: Then::Job(Job::Pull(census_pull())),
        };
        read.push(osaka.census_purpose());
        osaka.act = Act::Walk {
            to: 25,
            then: Then::Nothing,
        };
        read.push(osaka.census_purpose());
        osaka.shift = None;
        read.push(osaka.census_purpose());
        osaka.leaving = None;
        read.push(osaka.census_purpose());
        osaka.returning = None;
        read.push(osaka.census_purpose());
        osaka.dash = None;
        read.push(osaka.census_purpose());
        osaka.errand = None;
        read.push(osaka.census_purpose());
        osaka.chain = "travel/door";
        read.push(osaka.census_purpose());
        osaka.chain = "off-text/rest";
        read.push(osaka.census_purpose());
        assert_eq!(
            read,
            [
                "to text",
                "to text",
                "work",
                "routine",
                "routine",
                "dash",
                "errand",
                "wander",
                "travel",
                "off-text/rest"
            ]
        );
        // Her way to her door: for work, her shift's; else her routine's
        // (door batch, step 4b).
        for (why, purpose) in [
            (Leave::Work, "work"),
            (Leave::School, "routine"),
            (Leave::Stage, "routine"),
        ] {
            let spot = DoorSpot::at(
                30,
                15,
                super::super::door::Set::Floor(super::super::door::Fallback::Short),
            );
            osaka.act = Act::Walk {
                to: 30,
                then: Then::Job(Job::Leave { spot, why }),
            };
            assert_eq!(osaka.census_purpose(), purpose, "{why:?}");
            osaka.act = Act::Walk {
                to: 30,
                then: Then::Job(Job::Out {
                    at: Clear::of((30, 15), &[]).unwrap(),
                    why,
                }),
            };
            assert_eq!(
                osaka.census_purpose(),
                purpose,
                "{why:?}, out where she stands"
            );
            osaka.act = Act::Stand { until: 0 };
            osaka.heading = Some(Heading {
                want: Want::Work,
                job: Job::Leave { spot, why },
            });
            assert_eq!(osaka.census_purpose(), purpose, "{why:?}, heading");
            osaka.heading = None;
        }
    }

    /// The census's motion: a door is moving in the beats she's seen in,
    /// and none between them (out of sight, as off the screen is); a pull
    /// moves her in the beat she steps back with the line (a "text" body,
    /// for pulling: heaving, once its slack is in), not while she braces
    /// or reels the slack in; a walk to a job is for the job's kind, for
    /// the want she chose.
    #[test]
    fn the_census_classes_her_moving() {
        let mut rng = Rng(1);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        assert_eq!(osaka.census_motion(0), None, "standing is still");
        osaka.act = Act::Door {
            since: 0,
            to: Through::Space((5, 15)),
            gap: 0,
        };
        assert_eq!(osaka.census_motion(0).map(|m| m.body), Some(Body::Door));
        let hidden = (0..6000).find(|&t| osaka.hidden(t)).expect("a hidden beat");
        assert_eq!(osaka.census_motion(hidden), None, "out of sight");
        osaka.act = Act::Away {
            until: 0,
            enter: 0,
            to_y: 15,
            to_x: 5,
        };
        assert_eq!(osaka.census_motion(0), None, "off the screen");
        let pull = census_pull();
        for offset in 0..6 {
            for heaving in [false, true] {
                osaka.act = Act::Pull {
                    pull: pull.clone(),
                    offset,
                    goal: 8,
                    heaving,
                };
                let moving = heaving && offset > pull.gap;
                assert_eq!(
                    osaka.census_motion(0).map(|m| (m.body, m.purpose)),
                    moving.then_some((Body::Text, "pull")),
                    "offset {offset} heaving {heaving}"
                );
            }
        }
        osaka.credit = Some(Want::Pull);
        osaka.act = Act::Walk {
            to: 25,
            then: Then::Job(Job::Pull(pull)),
        };
        assert_eq!(
            osaka.census_motion(0),
            Some(Motion {
                purpose: "to text",
                body: Body::Walk,
                want: Some(Want::Pull),
            })
        );
    }

    /// Looking out of the window is her long daydream (phase 5c D6): one
    /// to three minutes, as long again as her mood lingers over it (a
    /// lazy Osaka's half again, an industrious one's less: lingered as
    /// she draws it, so the band's guard sees it), and it's spacing out,
    /// as gazing up is, in the census.
    #[test]
    fn looking_out_lasts_and_counts_as_spacing_out() {
        assert_eq!(use_duration(Use::LookOut), (60_000, 180_000));
        for (still, mood, (lo, hi)) in [
            (Stillness::NEUTRAL, Mood::Ordinary, (60_000, 180_000)),
            (Stillness::STARTING, Mood::Lazy, (90_000, 270_000)),
            (Stillness::STARTING, Mood::Industrious, (42_000, 126_000)),
        ] {
            let mut lengths = Vec::new();
            for seed in 0..32 {
                let mut rng = Rng(seed);
                let mut osaka = Osaka::standing_at(10, 10, 0, &mut rng);
                osaka.splice_rows = &[];
                osaka.set_mood(mood);
                osaka.stillness = still;
                osaka.credit = Some(Want::Use(Use::LookOut));
                osaka.start_job(
                    Job::Use(seat_for(Use::LookOut, Furniture::Window)),
                    1000,
                    &Chances::default(),
                    &mut rng,
                );
                let Act::Use { seat, until, .. } = osaka.act else {
                    panic!("{mood:?}: {:?}", osaka.act);
                };
                assert_eq!(seat.what, Use::LookOut);
                assert_eq!(osaka.census_group(), "spacing out");
                lengths.push(until - 1000);
            }
            assert!(
                lengths.iter().all(|l| (lo..=hi).contains(l)),
                "{mood:?}: {lengths:?}"
            );
            assert!(
                lengths.iter().any(|&l| l > hi - (hi - lo) / 4),
                "{mood:?}: never long: {lengths:?}"
            );
        }
    }

    /// The still acts (phase 5c B1) as tests start them: `set` at 0,
    /// lasting to 60 s, her at `(20, 15)` on [`floor_at`]`(15)` facing
    /// right (a piece's seat facing right too), each with the want that
    /// chose it, and whether she turns her body to the chat in its pose
    /// (not lying, which would turn her over end to end; not at her
    /// desk, where her pose is aimed at it).
    fn still_acts() -> Vec<(&'static str, Act, Want, bool)> {
        let using = |what: Use, item: Furniture| Act::Use {
            seat: Seat {
                x: 20,
                y: 15,
                facing: Facing::Right,
                ..seat_for(what, item)
            },
            since: 0,
            until: 60_000,
            whole: 60_000,
            play: Play::of(what, None),
            grievance: None,
        };
        let idle = |what: Activity| Act::Idle {
            what,
            since: 0,
            until: 60_000,
            play: None,
        };
        vec![
            ("sit", idle(Activity::Sit), Want::Idle(Activity::Sit), true),
            (
                "lie front",
                idle(Activity::LieFront),
                Want::Idle(Activity::LieFront),
                false,
            ),
            (
                "gaze",
                idle(Activity::Gaze),
                Want::Idle(Activity::Gaze),
                true,
            ),
            (
                "space out",
                Act::SpaceOut {
                    since: 0,
                    until: 60_000,
                    play: None,
                    session: None,
                },
                Want::SpaceOut,
                true,
            ),
            (
                "lounge",
                using(Use::Lounge, Furniture::Sofa),
                Want::Use(Use::Lounge),
                true,
            ),
            (
                "watch",
                using(Use::Watch, Furniture::Tv),
                Want::Use(Use::Watch),
                true,
            ),
            (
                "read",
                using(Use::Read, Furniture::Bookshelf),
                Want::Use(Use::Read),
                true,
            ),
            // Writing (the first half of its body).
            (
                "homework",
                using(Use::Homework, Furniture::Desk),
                Want::Use(Use::Homework),
                false,
            ),
            // Leaning on her sill, her pose aimed at it (phase 5c D6).
            (
                "look out",
                using(Use::LookOut, Furniture::Window),
                Want::Use(Use::LookOut),
                false,
            ),
        ]
    }

    /// A chat line in a still act on calm floor (phase 5c B1): she looks
    /// up where she is, the act running on. In its own pose, turned to
    /// the chat (where the pose turns: not lying, not at her desk or sill), `!` startled, then
    /// `?`, then a plain watching face until 5.4 s after the line (a frame
    /// past the `?`, [`LOOK_UP_MS`]); then
    /// her act's own look again (a piece's facing back to it). Its
    /// clock runs on: she decides only at its own end, it eases her as a
    /// whole, and the census logs a look that cut nothing (no restart).
    #[test]
    fn a_still_act_looks_up_where_she_is_through_a_chat_line() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        for (name, act, want, turns) in still_acts() {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.facing = Facing::Right;
            osaka.credit = Some(want);
            osaka.set(act.clone(), 0);
            osaka.tick(4_000, None, &terrain, &chances, &mut rng);
            let plain = osaka.clone();
            // In its own pose (her bob held a frame from each of the look's
            // changes: its frame may differ).
            let own = |at: u64| std::mem::discriminant(&plain.appearance(at).0);
            let looks = |osaka: &Osaka, at: u64| {
                let (pose, face, bubble) = osaka.appearance(at);
                (std::mem::discriminant(&pose), face, bubble)
            };
            let line = 5_000;
            osaka.look(line, 0, false, &terrain);
            assert_eq!(osaka.act, act, "{name}: the act runs on");
            let turned = if turns { Facing::Left } else { Facing::Right };
            assert_eq!(osaka.facing, turned, "{name}: turned to the chat");
            assert_eq!(
                looks(&osaka, line),
                (own(line), Face::Surprised, Some(Bubble::Bang)),
                "{name}"
            );
            let curious = line + LOOK_UP_SURPRISED_MS;
            osaka.tick(curious, None, &terrain, &chances, &mut rng);
            assert_eq!(
                looks(&osaka, curious),
                (own(curious), Face::Curious, Some(Bubble::Huh)),
                "{name}"
            );
            let watching = line + LOOK_MS;
            osaka.tick(watching, None, &terrain, &chances, &mut rng);
            assert_eq!(
                looks(&osaka, watching),
                (own(watching), Face::Vacant, None),
                "{name}: watching"
            );
            let over = line + WATCH_MS.max(LOOK_UP_MS);
            osaka.tick(over, None, &terrain, &chances, &mut rng);
            assert_eq!(osaka.act, act, "{name}: still at it");
            let (_, face, bubble) = plain.appearance(over);
            assert_eq!(looks(&osaka, over), (own(over), face, bubble), "{name}");
            let facing = if matches!(act, Act::Use { .. }) {
                Facing::Right
            } else {
                turned
            };
            assert_eq!(osaka.facing, facing, "{name}: as she was at it");
            assert!(osaka.due() > over, "{name}: the look's moments all handled");
            let n = osaka.decisions.len();
            osaka.tick(59_999, None, &terrain, &chances, &mut rng);
            assert_eq!(osaka.act, act, "{name}: at it to its end");
            assert_eq!(osaka.decisions.len(), n, "{name}");
            osaka.tick(60_000, None, &terrain, &chances, &mut rng);
            assert_eq!(osaka.decisions.len(), n + 1, "{name}: decides at its end");
            assert_eq!(osaka.credited, [(want, 1.0, 60_000)], "{name}: as a whole");
            assert_eq!(osaka.chat_cuts, [(line, false, None)], "{name}");
            assert_eq!(osaka.after_chat, None, "{name}: no restart");
        }
    }

    /// The slow blinks a held act set at 0 with `whims` would have, from
    /// the schedule alone (starts, before `until`).
    fn held_blink_starts(whims: Whims, until: u64) -> Vec<u64> {
        let mut starts = Vec::new();
        let mut at = 0;
        for k in 0.. {
            at += held_blink_gap(whims, k);
            if at >= until {
                break;
            }
            starts.push(at);
        }
        starts
    }

    /// On a pose she holds (phase 5c Q3: sitting, lounging, watching,
    /// reading, writing her homework, a long gaze, looking out), she
    /// blinks slowly: for `BLINK_MS` every 6–12 s of the act, each gap
    /// drawn from the whims of the decision that set it, never from
    /// either stream. Her wakeups land on each blink's start and end and
    /// nowhere else her act doesn't ask for, and a tick there says how
    /// she looks changed. Homework blinks only while she writes (its
    /// first half), never nodding off or asleep on the paper. Other
    /// whims blink at other moments.
    #[test]
    fn a_held_pose_blinks_slowly_on_her_whims() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let held = [
            "sit", "gaze", "lounge", "watch", "read", "homework", "look out",
        ];
        let mut firsts = std::collections::BTreeSet::new();
        for (name, act, _, _) in still_acts() {
            if !held.contains(&name) {
                continue;
            }
            for seed in [1u64, 2, 3] {
                let at = format!("{name} whims {seed}");
                let mut rng = Rng(3);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.facing = Facing::Right;
                osaka.whims = Whims(seed);
                osaka.set(act.clone(), 0);
                let until = if name == "homework" { 30_000 } else { 60_000 };
                let starts = held_blink_starts(Whims(seed), until);
                assert!(starts.len() >= 2, "{at}: {starts:?}");
                assert!((6_000..=12_000).contains(&starts[0]), "{at}: {starts:?}");
                firsts.insert(starts[0]);
                for pair in starts.windows(2) {
                    let gap = pair[1] - pair[0];
                    assert!((6_000..=12_000).contains(&gap), "{at}: {gap}");
                }
                let edges: Vec<u64> = starts.iter().flat_map(|&s| [s, s + 150]).collect();
                let streams = (rng.0, osaka.mind.0);
                let mut now = 0;
                let mut woke = Vec::new();
                while now < 59_000 {
                    let wake = osaka.wakes_at();
                    assert!(wake > now, "{at}: wakes at {wake} by {now}");
                    let event = osaka.due();
                    now = wake.min(59_000);
                    let changed = osaka.tick(now, None, &terrain, &chances, &mut rng);
                    if wake < event && now == wake {
                        assert!(changed, "{at}: a blink's edge at {now} changed nothing");
                        woke.push(now);
                    }
                }
                assert_eq!(woke, edges, "{at}: woken for each blink's edges");
                assert_eq!((rng.0, osaka.mind.0), streams, "{at}: nothing drawn");
                for &start in &starts {
                    // 150 ms (phase 5c Q3), as the spec has it.
                    for (t, blinks) in [
                        (start - 1, false),
                        (start, true),
                        (start + 149, true),
                        (start + 150, false),
                    ] {
                        let face = osaka.appearance(t).1;
                        assert_eq!(face == Face::Blink, blinks, "{at}: {face:?} at {t}");
                    }
                }
            }
        }
        assert!(firsts.len() > 1, "every whims blinks alike: {firsts:?}");
    }

    /// No slow blink where she doesn't hold still with her eyes open:
    /// standing (her own quicker blink), spacing out, kicking her feet,
    /// touching her toes, eating, petting the cat, dozing, asleep over her
    /// homework, smiling pleased with her surfing; nor while she looks up at a
    /// chat line, startled, puzzled and watching. No wakeup comes for
    /// one either.
    #[test]
    fn no_slow_blink_where_she_doesnt_hold_still() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let idle = |what: Activity| Act::Idle {
            what,
            since: 0,
            until: 60_000,
            play: None,
        };
        let playing = |what: Use, item: Furniture, play: Play| Act::Use {
            seat: Seat {
                x: 20,
                y: 15,
                facing: Facing::Right,
                ..seat_for(what, item)
            },
            since: 0,
            until: 60_000,
            whole: 60_000,
            play,
            grievance: None,
        };
        let using = |what: Use, item: Furniture| playing(what, item, Play::of(what, None));
        let homework = still_acts()
            .into_iter()
            .find(|(name, ..)| *name == "homework")
            .map(|(_, act, ..)| act)
            .expect("homework");
        let acts = [
            (
                "space out",
                Act::SpaceOut {
                    since: 0,
                    until: 60_000,
                    play: None,
                    session: None,
                },
                0,
            ),
            ("lie front", idle(Activity::LieFront), 0),
            ("lie back", idle(Activity::LieBack), 0),
            ("sit doze", idle(Activity::SitDoze), 0),
            ("homework asleep", homework, 30_000),
            ("snack", using(Use::Snack, Furniture::Fridge), 0),
            ("pet", using(Use::Pet, Furniture::CatBed), 0),
            ("toe touch", idle(Activity::ToeTouch), 0),
            // Pleased with her surfing, humming (eyes smiling shut).
            (
                "surfed",
                playing(Use::Watch, Furniture::Tv, Play::plain(ScriptId::Surf)),
                4 * script::SURF_MS,
            ),
        ];
        for (name, act, from) in acts {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.whims = Whims(1);
            osaka.set(act, 0);
            let mut now = from;
            osaka.tick(now, None, &terrain, &chances, &mut rng);
            while now < 59_000 {
                assert_eq!(osaka.wakes_at(), osaka.due(), "{name}: a wakeup at {now}");
                // Awake, she never blinks there (a doze's eyes are shut
                // anyway).
                if matches!(
                    name,
                    "space out" | "lie front" | "snack" | "pet" | "toe touch" | "surfed"
                ) {
                    assert_ne!(osaka.appearance(now).1, Face::Blink, "{name}: at {now}");
                }
                now = (now + 50).min(osaka.due()).max(now + 1);
                osaka.tick(now, None, &terrain, &chances, &mut rng);
            }
        }
        // Sitting, a line at each blink's start: she looks up, and her
        // look's faces hold through it, blink or no blink due.
        let starts = held_blink_starts(Whims(1), 60_000);
        for &start in starts.iter().take(3) {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.whims = Whims(1);
            osaka.set(idle(Activity::Sit), 0);
            osaka.tick(start - 10, None, &terrain, &chances, &mut rng);
            osaka.look(start, 0, false, &terrain);
            for (t, face) in [
                (start, Face::Surprised),
                (start + BLINK_MS / 2, Face::Surprised),
            ] {
                osaka.tick(t, None, &terrain, &chances, &mut rng);
                assert_eq!(osaka.appearance(t).1, face, "a line at {start}, at {t}");
            }
        }
        // A line come mid-blink: the blink is over at once, her startle
        // all there is; and no wakeup comes for the blink's end, unseen.
        for &start in starts.iter().take(3) {
            let line = start + 75;
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.whims = Whims(1);
            osaka.set(idle(Activity::Sit), 0);
            osaka.tick(start, None, &terrain, &chances, &mut rng);
            assert_eq!(
                osaka.appearance(start).1,
                Face::Blink,
                "blinking at {start}"
            );
            osaka.look(line, 0, false, &terrain);
            osaka.tick(line, None, &terrain, &chances, &mut rng);
            for t in line..start + 150 {
                assert_eq!(
                    osaka.appearance(t).1,
                    Face::Surprised,
                    "a line at {line}, mid-blink, at {t}"
                );
            }
            assert!(
                osaka.wakes_at() > start + 150,
                "a line at {line}: a wakeup for a blink unseen"
            );
        }
        // Her look over mid-blink (begun under it): no blink's remainder
        // shows after it, nor wakes her.
        for &start in starts.iter().take(3) {
            let line = start + 50 - WATCH_MS.max(LOOK_UP_MS);
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.whims = Whims(1);
            osaka.set(idle(Activity::Sit), 0);
            osaka.tick(line - 10, None, &terrain, &chances, &mut rng);
            osaka.look(line, 0, false, &terrain);
            let mut now = line;
            while osaka.looking_up.is_some() {
                now = osaka.wakes_at();
                osaka.tick(now, None, &terrain, &chances, &mut rng);
            }
            assert!(
                (start..start + 150).contains(&now),
                "her look over at {now}, not mid-blink at {start}"
            );
            assert!(
                osaka.wakes_at() > start + 150,
                "a wakeup at a blink's end unseen"
            );
            for t in now..start + 150 {
                assert_ne!(
                    osaka.appearance(t).1,
                    Face::Blink,
                    "her look over at {now}, at {t}"
                );
            }
        }
        // The look's plain watch, over a blink due in it.
        let start = starts[0];
        let line = start - LOOK_MS - 100;
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.whims = Whims(1);
        osaka.set(idle(Activity::Sit), 0);
        osaka.tick(line - 10, None, &terrain, &chances, &mut rng);
        osaka.look(line, 0, false, &terrain);
        osaka.tick(start, None, &terrain, &chances, &mut rng);
        assert_eq!(
            osaka.appearance(start),
            (Pose::Sit, Face::Vacant, None),
            "watching the chat at {start}"
        );
        assert!(
            osaka.wakes_at() > start + BLINK_MS,
            "no wakeup for a blink unseen"
        );
    }

    /// A gaze says "ooh" as it begins, for its first 3 s, then gazes on
    /// curious and quiet (phase 5c M8): her tick comes as the bubble goes.
    #[test]
    fn a_gaze_says_ooh_then_gazes_on_quietly() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.set(
            Act::Idle {
                what: Activity::Gaze,
                since: 0,
                until: 60_000,
                play: None,
            },
            0,
        );
        for t in [0, 1_000, 2_999] {
            assert_eq!(
                osaka.appearance(t),
                (Pose::Gaze, Face::Curious, Some(Bubble::Ooh)),
                "{t}"
            );
        }
        assert_eq!(osaka.due(), 3_000, "a tick as the bubble goes");
        assert!(osaka.tick(3_000, None, &terrain, &chances, &mut rng));
        for t in [3_000, 5_000, 59_000] {
            let (pose, face, bubble) = osaka.appearance(t);
            assert_eq!((pose, bubble), (Pose::Gaze, None), "{t}");
            assert!(matches!(face, Face::Curious | Face::Blink), "{t}: {face:?}");
        }
    }

    /// Her look up at a chat line from a still act shows each of its
    /// marks for a frame at least ([`USE_FRAME_MS`]; phase 5c's tail, T4,
    /// as whatever she says does since T2): startled (`!`), puzzled
    /// (`?`), and watching plain-faced, each never coming and going
    /// inside a frame. In every still act, a single line: her face and
    /// bubble sampled every 10 ms from the line until a frame past her
    /// look's end hold each look of hers for a frame, but the last (her
    /// act's own again, which runs on).
    #[test]
    fn her_look_up_shows_each_of_its_marks_for_a_frame() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        for (name, act, _, _) in still_acts() {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.facing = Facing::Right;
            osaka.set(act.clone(), 0);
            osaka.tick(5_000, None, &terrain, &chances, &mut rng);
            osaka.look(5_000, 0, false, &terrain);
            assert!(osaka.looking_up_at_chat(), "{name}: looks up in place");
            // Each run of her face and bubble: its start and what it shows.
            let mut runs: Vec<(u64, (Face, Option<Bubble>))> = Vec::new();
            let mut over = None;
            let mut t = 5_000;
            while over.is_none_or(|over| t < over + 2 * USE_FRAME_MS) {
                osaka.tick(t, None, &terrain, &chances, &mut rng);
                let (_, face, bubble) = osaka.appearance(t);
                // Her slow blink is its own exemption: her face as it was.
                let face = match runs.last() {
                    Some(&(_, (was, _))) if face == Face::Blink => was,
                    _ => face,
                };
                if runs.last().is_none_or(|&(_, look)| look != (face, bubble)) {
                    runs.push((t, (face, bubble)));
                }
                if over.is_none() && !osaka.looking_up_at_chat() {
                    over = Some(t);
                }
                t += 10;
                assert!(t < 30_000, "{name}: her look never ends");
            }
            assert_eq!(runs[0].1, (Face::Surprised, Some(Bubble::Bang)), "{name}");
            for pair in runs.windows(2) {
                let ((from, look), (to, _)) = (pair[0], pair[1]);
                assert!(
                    to - from >= USE_FRAME_MS,
                    "{name}: {look:?} from {from} to {to}, under a frame ({runs:?})"
                );
            }
        }
    }

    /// Her still acts and the bobbing one beside them (reading on her
    /// back), for [`her_act_waits_a_frame_from_each_change_of_her_look_up`]
    /// and [`her_act_waits_a_frame_from_what_she_says`]: those whose own
    /// pose changes no faster than a frame (lying on her front kicks her
    /// feet faster, a quick bob the stillness rule leaves to the band).
    fn still_acts_and_a_bob() -> Vec<(&'static str, Act)> {
        let acts: Vec<(&'static str, Act)> = still_acts()
            .into_iter()
            .map(|(name, act, ..)| (name, act))
            .chain([(
                "lie read",
                Act::Idle {
                    what: Activity::LieRead,
                    since: 0,
                    until: 60_000,
                    play: None,
                },
            )])
            .collect();
        let slow = |act: &Act| {
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut Rng(3));
            osaka.set(act.clone(), 0);
            let changes: Vec<u64> = (10..20_000)
                .step_by(10)
                .filter(|&t| osaka.acting(t - 10).0 != osaka.acting(t).0)
                .collect();
            changes.windows(2).all(|w| w[1] - w[0] >= USE_FRAME_MS)
        };
        let (slow, quick): (Vec<_>, Vec<_>) = acts.into_iter().partition(|(_, act)| slow(act));
        let quick: Vec<&str> = quick.iter().map(|(name, _)| *name).collect();
        assert_eq!(quick, ["lie front"]);
        slow
    }

    /// Her whole look (pose, face, bubble, facing) sampled every 10 ms
    /// from `from` to `to`, `event` (a chat line, a line she says) run at
    /// `at` (on a sample) and her tick at each sample (as a client
    /// painting then would), checked against the stillness rule's count
    /// (design.md: an exempt change of hers that isn't periodic still
    /// counts): a change `own` names (the look's or her line's) needn't
    /// wait, but whatever of hers changes next waits a frame from it, and
    /// from any change before; her slow blink alone is exempt and counts
    /// nothing.
    fn check_waits_a_frame(
        case: &str,
        osaka: &mut Osaka,
        (from, to): (u64, u64),
        (at, event): (u64, impl FnOnce(&mut Osaka, u64)),
        own: impl Fn((Pose, Face, Option<Bubble>), (Pose, Face, Option<Bubble>)) -> bool,
    ) {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(3);
        let mut event = Some(event);
        let mut last: Option<u64> = None;
        let mut was: Option<(script::Look, Facing)> = None;
        for t in (from..to).step_by(10) {
            if t == at
                && let Some(event) = event.take()
            {
                event(osaka, t);
            }
            osaka.tick(t, None, &terrain, &chances, &mut rng);
            let now = (osaka.appearance(t), osaka.facing);
            let Some(before) = was.replace(now) else {
                continue;
            };
            if before == now {
                continue;
            }
            let ((pose, face, bubble), facing) = before;
            let ((pose2, face2, bubble2), facing2) = now;
            if (pose, bubble, facing) == (pose2, bubble2, facing2)
                && (face == Face::Blink || face2 == Face::Blink)
            {
                continue;
            }
            if !own(before.0, now.0)
                && let Some(last) = last
            {
                assert!(
                    t - last >= USE_FRAME_MS,
                    "{case}: changed at {last} and {t} ({before:?} to {now:?})"
                );
            }
            last = Some(t);
        }
        assert!(event.is_none(), "{case}: {at} never sampled");
    }

    /// Her act waits a frame from each change of her look up at a chat
    /// line (phase 5c's tail, T4; design.md, an exempt change that isn't
    /// periodic still counts): its `!` beginning, its `?`, its plain watch
    /// and its end each hold her bob a frame ([`Osaka::hold_bob`]), so in
    /// every still act, and reading on her back, with the line at each
    /// 100 ms of her bob's frame, her whole look, from two frames before
    /// the line to well past the look, never changes within a frame of a
    /// change but for the look's own next change (her face, bubble and
    /// facing: [`check_waits_a_frame`]).
    #[test]
    fn her_act_waits_a_frame_from_each_change_of_her_look_up() {
        let terrain = floor_at(15);
        let mut tried = 0;
        for (name, act) in still_acts_and_a_bob() {
            for line in (5_000..5_000 + USE_FRAME_MS).step_by(100) {
                let case = format!("{name}, a line at {line}");
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut Rng(3));
                osaka.facing = Facing::Right;
                osaka.set(act.clone(), 0);
                let end = line + LOOK_UP_MS.max(WATCH_MS) + 3 * USE_FRAME_MS;
                check_waits_a_frame(
                    &case,
                    &mut osaka,
                    (line - 2 * USE_FRAME_MS, end),
                    (line, |osaka: &mut Osaka, t| {
                        osaka.look(t, 0, false, &terrain);
                        assert!(osaka.looking_up_at_chat(), "looks up in place");
                    }),
                    |(pose, ..), (pose2, ..)| pose == pose2,
                );
                tried += 1;
            }
        }
        assert!(tried > 50, "{tried}");
    }

    /// Her act waits a frame from what she says (phase 5c's tail, T4):
    /// a line coming and going, off her bob's grid, holds her bob a frame
    /// from each ([`Osaka::say`], her tick at its end), so in every still
    /// act, and reading on her back, with a line said at each 100 ms of
    /// her bob's frame (short and long), her whole look, from two frames
    /// before the line to well past it, never changes within a frame of a
    /// change but for the line's own bubble ([`check_waits_a_frame`]).
    #[test]
    fn her_act_waits_a_frame_from_what_she_says() {
        let mut tried = 0;
        for (name, act) in still_acts_and_a_bob() {
            for text in [OK, "I wonder what the sea tastes like today"] {
                for said in (5_000..5_000 + USE_FRAME_MS).step_by(100) {
                    let case = format!("{name}, {text:?} said at {said}");
                    let mut osaka = Osaka::standing_at(20, 15, 0, &mut Rng(3));
                    osaka.facing = Facing::Right;
                    osaka.set(act.clone(), 0);
                    let end = said + speech_ms(text) + 3 * USE_FRAME_MS;
                    check_waits_a_frame(
                        &case,
                        &mut osaka,
                        (said - 2 * USE_FRAME_MS, end),
                        (said, |osaka: &mut Osaka, t| osaka.say(text, t)),
                        |(pose, face, _), (pose2, face2, _)| (pose, face) == (pose2, face2),
                    );
                    tried += 1;
                }
            }
        }
        assert!(tried > 100, "{tried}");
    }

    /// Asked something (a line that `asks`) in a still act with nothing
    /// to answer, she looks up in place as at any line; and each line of
    /// a lively chat looks again (`!` from it), watched until 5.4 s after
    /// the last (a frame past the `?`, [`LOOK_UP_MS`]), a piece's facing
    /// back after it.
    #[test]
    fn each_line_of_a_lively_chat_looks_up_again_in_place() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        for (name, act, _, _) in still_acts() {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.facing = Facing::Right;
            osaka.set(act.clone(), 0);
            osaka.look(5_000, 0, true, &terrain);
            osaka.tick(7_000, None, &terrain, &chances, &mut rng);
            osaka.look(7_000, 0, false, &terrain);
            assert_eq!(osaka.act, act, "{name}");
            assert_eq!(osaka.appearance(7_000).2, Some(Bubble::Bang), "{name}");
            osaka.tick(7_000 + LOOK_MS, None, &terrain, &chances, &mut rng);
            assert_eq!(osaka.appearance(7_000 + LOOK_MS).1, Face::Vacant, "{name}");
            osaka.tick(
                7_000 + WATCH_MS.max(LOOK_UP_MS) - 1,
                None,
                &terrain,
                &chances,
                &mut rng,
            );
            assert_eq!(
                osaka.appearance(7_000 + WATCH_MS.max(LOOK_UP_MS) - 1).1,
                Face::Vacant,
                "{name}"
            );
            osaka.tick(
                7_000 + WATCH_MS.max(LOOK_UP_MS),
                None,
                &terrain,
                &chances,
                &mut rng,
            );
            assert_eq!(osaka.act, act, "{name}");
            if matches!(act, Act::Use { .. }) {
                assert_eq!(osaka.facing, Facing::Right, "{name}: back to the piece");
            }
            assert_eq!(osaka.chat_cuts.len(), 2, "{name}");
        }
    }

    /// Whatever she says shows for a frame at least ([`USE_FRAME_MS`]),
    /// however short: a line coming and going inside one would flicker
    /// (Round 8: "Mm?", stirring by day, lasted 1380 ms). Each line she
    /// can say, from one character up, said standing and stirring
    /// asleep: the bubble, and the stir's turn with it, hold through the
    /// frame and end together.
    #[test]
    fn whatever_she_says_shows_for_a_frame() {
        const SHORT: [&str; 6] = ["", "!", "Oh", STIRRED, MM, OK];
        // Every 10 ms through the frame, and its last ms.
        let frame = |from: u64| {
            (from..from + USE_FRAME_MS)
                .step_by(10)
                .chain([from + USE_FRAME_MS - 1])
        };
        for text in SHORT {
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut Rng(3));
            osaka.say(text, 1_000);
            for t in frame(1_000) {
                assert_eq!(
                    osaka.appearance(t).2,
                    Some(Bubble::Say(text)),
                    "{text:?} said at 1000, {t}"
                );
            }
            let mut asleep = Osaka::standing_at(20, 15, 0, &mut Rng(3));
            asleep.set(
                Act::Idle {
                    what: Activity::LieBack,
                    since: 0,
                    until: 600_000,
                    play: None,
                },
                0,
            );
            asleep.stir_saying(text, 5_000);
            let stirred = asleep.appearance(5_000);
            for t in frame(5_000) {
                assert_eq!(
                    asleep.appearance(t),
                    stirred,
                    "{text:?} stirred at 5000, {t}"
                );
            }
            let over = 5_000 + speech_ms(text);
            assert_eq!(
                asleep.stir_until, over,
                "{text:?}: the turn ends with the line"
            );
            assert_ne!(
                asleep.appearance(over).2,
                Some(Bubble::Say(text)),
                "{text:?}"
            );
            assert!(speech_ms(text) >= USE_FRAME_MS, "{text:?}");
        }
    }

    /// A stir's turn ends with its murmur, however the murmur ends (phase
    /// 5c's tail, T4): run to its end it's woken for and held at
    /// (`whatever_she_says_shows_for_a_frame`), and cut short by another
    /// line (her sleep-talk, the Dream's hush, a line by day) the turn
    /// ends there too, so it never runs on unwoken past the line that
    /// went with it. Lying back, stirred at 5000 and saying another line
    /// at each 100 ms of the murmur: stirring until then, not after.
    #[test]
    fn a_stirs_turn_ends_with_its_murmur_cut_short() {
        for said in (5_100..5_000 + speech_ms(MM)).step_by(100) {
            let mut asleep = Osaka::standing_at(20, 15, 0, &mut Rng(3));
            asleep.set(
                Act::Idle {
                    what: Activity::LieBack,
                    since: 0,
                    until: 600_000,
                    play: None,
                },
                0,
            );
            asleep.stir_saying(MM, 5_000);
            assert!(asleep.stirring_at_chat(said - 1), "{said}");
            asleep.say(OK, said);
            for t in [said, said + 100, 5_000 + speech_ms(MM)] {
                assert!(
                    !asleep.stirring_at_chat(t),
                    "murmur cut at {said}: turned at {t}"
                );
            }
        }
    }

    /// A bob on the frame that isn't a script's key holds its last frame
    /// through the part of a period before her act ends, as a key's bob
    /// does (Round 8, the user; `a_bob_flips_a_frame_clear_of_its_keys_start_and_end`
    /// in script.rs): lying back and reading on her back (her idle acts
    /// bobbing on [`USE_FRAME_MS`] or slower), and reading a borrowed
    /// strip beside the tear. Each starts on its own grid, so only its
    /// end can land off it: over acts from 20 s to 90 s (in steps of
    /// 997 ms), no flip comes within a frame of the act's end.
    #[test]
    fn an_idle_bob_holds_its_last_frame_before_her_act_ends() {
        let slow: Vec<Activity> = Activity::ALL
            .into_iter()
            .filter(|what| what.period() >= USE_FRAME_MS)
            .collect();
        assert_eq!(slow, [Activity::LieBack, Activity::LieRead]);
        let mut acts: Vec<(String, Act)> = Vec::new();
        for until in (20_000..90_000).step_by(997) {
            for &what in &slow {
                acts.push((
                    format!("{what:?}"),
                    Act::Idle {
                        what,
                        since: 0,
                        until,
                        play: None,
                    },
                ));
            }
            acts.push((
                "reading a strip".to_owned(),
                Act::Borrow {
                    pull: census_pull(),
                    since: 0,
                    phase: Borrowing::Read { until },
                },
            ));
        }
        let mut flips = 0u32;
        for (at, act) in acts {
            let until = match act {
                Act::Idle { until, .. }
                | Act::Borrow {
                    phase: Borrowing::Read { until },
                    ..
                } => until,
                _ => unreachable!(),
            };
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut Rng(3));
            osaka.set(act, 0);
            for t in (1..).map(|k| k * USE_FRAME_MS).take_while(|&t| t < until) {
                if osaka.acting(t - 1).0 != osaka.acting(t).0 {
                    flips += 1;
                    assert!(
                        t + USE_FRAME_MS <= until,
                        "{at} to {until}: flips at {t}, within a frame of its end"
                    );
                }
            }
        }
        assert!(flips > 1000, "{flips} flips tried");
    }

    /// Her act's end moved in place (phase 5c's tail, T4): a day's sleep
    /// become her night at bedtime (`sleep_on`), and her night's wake time
    /// found afresh as her clock is read (`refresh_night`), later or
    /// earlier. Her breathing's bob holds the frame it shows as the end
    /// moves, until the first flip on its grid a frame on (as from a
    /// key's start), so it neither changes as the end moves nor flips
    /// within a frame of that: the hold was timed from the end known
    /// before, so with the end moved within its last frame her breathing
    /// flipped then, off the grid, and again within a frame. Over every
    /// 50 ms of that last frame, at three phases of her breathing's grid,
    /// her pose sampled every 10 ms either side never changes twice
    /// within a frame.
    #[test]
    fn a_bob_holds_a_frame_when_her_acts_end_moves_in_place() {
        let clock = monday_at(22, 0);
        // Tuesday 07:00 (a school day), in monotonic millis.
        let wake = BED + (9 * 60 - 30) * 10_000;
        // Every change of her pose from `from` to `to`, sampled every 10 ms.
        let changes = |osaka: &Osaka, from: u64, to: u64| -> Vec<(u64, Pose)> {
            (from..to)
                .step_by(10)
                .map(|t| (t, osaka.appearance(t).0))
                .collect()
        };
        let check = |at: &str, poses: Vec<(u64, Pose)>| {
            let mut last: Option<u64> = None;
            for pair in poses.windows(2) {
                let ((_, was), (t, now)) = (pair[0], pair[1]);
                if was == now {
                    continue;
                }
                if let Some(before) = last {
                    assert!(
                        t - before >= USE_FRAME_MS,
                        "{at}: changed at {before} and {t} ({was:?} to {now:?})"
                    );
                }
                last = Some(t);
            }
        };
        let night = |since: u64, until: u64| Act::Use {
            seat: seat_for(Use::Sleep, Furniture::Bed),
            since,
            until,
            whole: until - since,
            play: Play {
                branch: Surface::Bed.branch(true),
                ..Play::plain(ScriptId::Night)
            },
            grievance: None,
        };
        let mut tried = 0;
        for phase in [0, 700, 1_234] {
            for d in (50..USE_FRAME_MS).step_by(50) {
                // At bedtime, a day's sleep that would have ended `d` on.
                let at = format!("sleep_on, phase {phase}, {d} ms before its end");
                let sleep = Act::Use {
                    seat: seat_for(Use::Sleep, Furniture::Bed),
                    since: BED - 30_000 - phase,
                    until: BED + d,
                    whole: 30_000 + phase + d,
                    play: Play::of(Use::Sleep, None),
                    grievance: None,
                };
                let (mut osaka, mut rng) = at_bedtime(sleep, BED + d);
                let mut poses = changes(&osaka, BED - 3 * USE_FRAME_MS, BED);
                osaka.tick(
                    BED,
                    Some(clock),
                    &blank_terrain(),
                    &Chances::default(),
                    &mut rng,
                );
                assert!(osaka.sleeping(), "{at}");
                poses.extend(changes(&osaka, BED, BED + 4 * USE_FRAME_MS));
                check(&at, poses);
                // Asleep for the night, her wake time moves as her clock
                // is read: later (it was a moment off), and earlier (to a
                // moment on).
                for later in [true, false] {
                    let at = format!("refresh_night, later={later}, phase {phase}, {d} ms");
                    let read = if later {
                        BED + 600_000 + phase
                    } else {
                        wake - d
                    };
                    let until = if later { read + d } else { wake + 3_600_000 };
                    let (mut osaka, _) = at_bedtime(night(BED - phase, until), read + d);
                    osaka.act_since_game = Some(clock.game.at(BED));
                    let mut poses = changes(&osaka, read - 3 * USE_FRAME_MS, read);
                    osaka.read_clock(Some(clock), read);
                    let Act::Use { until: moved, .. } = osaka.act else {
                        panic!("{at}: {:?}", osaka.act);
                    };
                    assert_eq!(moved, wake, "{at}: her wake time");
                    poses.extend(changes(&osaka, read, (read + 4 * USE_FRAME_MS).min(wake)));
                    check(&at, poses);
                    tried += 1;
                }
            }
        }
        assert!(tried > 100, "{tried}");
    }

    /// Her wake time moved as her clock is read at a tick that comes late,
    /// after a line she said in her sleep ended (a client whose ticks lag
    /// her wakes): her breathing holds from the moment the end moved, the
    /// latest, not from the line's end handled after it in the same tick
    /// (a hold never moves back: [`Osaka::hold_bob`]). Painted as the
    /// client paints, her pose before the tick (her wake as it was) and
    /// after it never changes twice within a frame. Over lines ending in
    /// each 100 ms of the frame before the late tick, at three phases of
    /// her breathing's grid, her old wake within its last frame.
    #[test]
    fn a_late_tick_holds_her_bob_from_her_wake_moving() {
        let clock = monday_at(22, 0);
        let wake = BED + (9 * 60 - 30) * 10_000;
        let night = |since: u64, until: u64| Act::Use {
            seat: seat_for(Use::Sleep, Furniture::Bed),
            since,
            until,
            whole: until - since,
            play: Play {
                branch: Surface::Bed.branch(true),
                ..Play::plain(ScriptId::Night)
            },
            grievance: None,
        };
        let mut tried = 0;
        for phase in [0, 700, 1_234] {
            for d in [300, 900] {
                for ended in (100..USE_FRAME_MS).step_by(100) {
                    let read = BED + 600_000 + phase;
                    let at = format!("phase {phase}, wake {d} ms on, line over {ended} ms before");
                    let (mut osaka, mut rng) = at_bedtime(night(BED - phase, read + d), read + d);
                    osaka.act_since_game = Some(clock.game.at(BED));
                    osaka.say(MM, read - ended - speech_ms(MM));
                    let mut poses: Vec<(u64, Pose)> = (read - 4 * USE_FRAME_MS..read)
                        .step_by(10)
                        .map(|t| (t, osaka.appearance(t).0))
                        .collect();
                    osaka.tick(
                        read,
                        Some(clock),
                        &blank_terrain(),
                        &Chances::default(),
                        &mut rng,
                    );
                    let Act::Use { until: moved, .. } = osaka.act else {
                        panic!("{at}: {:?}", osaka.act);
                    };
                    assert_eq!(moved, wake, "{at}: her wake time");
                    assert_eq!(
                        osaka.bob_held.map(|(held, _)| held),
                        Some(read),
                        "{at}: held from her wake moving"
                    );
                    poses.extend(
                        (read..read + 4 * USE_FRAME_MS)
                            .step_by(10)
                            .map(|t| (t, osaka.appearance(t).0)),
                    );
                    let mut last: Option<u64> = None;
                    for pair in poses.windows(2) {
                        let ((_, was), (t, now)) = (pair[0], pair[1]);
                        if was == now {
                            continue;
                        }
                        if let Some(before) = last {
                            assert!(
                                t - before >= USE_FRAME_MS,
                                "{at}: changed at {before} and {t} ({was:?} to {now:?})"
                            );
                        }
                        last = Some(t);
                    }
                    tried += 1;
                }
            }
        }
        assert!(tried > 50, "{tried}");
    }

    /// Dozing (lying back on the floor, napping, asleep by day, asleep
    /// over her homework), a chat line only stirs her, as at night: she
    /// blinks and turns over a moment (the line comes as her own pose
    /// is on its frame 0, so the turn shows), saying "Mm?", not turning
    /// to it, and dozes on; her act runs to its own end. A stir isn't a
    /// look: the census logs none, and it sets no watch.
    #[test]
    fn a_doze_stirs_at_a_chat_line_and_dozes_on() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let using = |what: Use, item: Furniture| Act::Use {
            seat: Seat {
                x: 20,
                y: 15,
                facing: Facing::Right,
                ..seat_for(what, item)
            },
            since: 0,
            until: 60_000,
            whole: 60_000,
            play: Play::of(what, None),
            grievance: None,
        };
        let dozes = [
            (
                "lie back",
                Act::Idle {
                    what: Activity::LieBack,
                    since: 0,
                    until: 60_000,
                    play: None,
                },
                5_000,
                Pose::LieBack(1),
            ),
            ("nap", using(Use::Nap, Furniture::Sofa), 5_000, Pose::Nap(1)),
            (
                "sleep",
                using(Use::Sleep, Furniture::Bed),
                5_000,
                Pose::Sleep(1),
            ),
            // Asleep over it (its body's last quarter).
            (
                "homework",
                using(Use::Homework, Furniture::Desk),
                50_000,
                Pose::Homework(3),
            ),
        ];
        for (name, act, from, stirred) in dozes {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.facing = Facing::Right;
            osaka.set(act.clone(), 0);
            let plain = osaka.clone();
            // Her own pose's frame 0 (asleep over her homework has one).
            let line = (from..from + 10_000)
                .find(|&t| {
                    matches!(
                        plain.appearance(t).0,
                        Pose::LieBack(0) | Pose::Nap(0) | Pose::Sleep(0) | Pose::Homework(3)
                    )
                })
                .unwrap_or_else(|| panic!("{name}: never on frame 0"));
            osaka.tick(line - 1, None, &terrain, &chances, &mut rng);
            osaka.look(line, 0, false, &terrain);
            assert_eq!(osaka.act, act, "{name}: dozes on");
            assert_eq!(osaka.facing, Facing::Right, "{name}: not turned to it");
            assert_eq!(osaka.watch_until, 0, "{name}: a stir sets no watch");
            assert_eq!(
                osaka.appearance(line),
                (stirred, Face::Blink, Some(Bubble::Say(STIRRED))),
                "{name}"
            );
            // The stir, its turn and its "Mm?" together, lasts a frame at
            // least (Round 8: by day it was 1380 ms, coming and going
            // inside one).
            for t in (line..line + USE_FRAME_MS)
                .step_by(10)
                .chain([line + USE_FRAME_MS - 1])
            {
                assert_eq!(
                    osaka.appearance(t),
                    (stirred, Face::Blink, Some(Bubble::Say(STIRRED))),
                    "{name}: the stir holds a frame, {t}"
                );
            }
            // Back as she was (her bob held a frame from the stir's end:
            // its frame may differ).
            let after = line + speech_ms(STIRRED);
            osaka.tick(after, None, &terrain, &chances, &mut rng);
            let (pose, face, bubble) = osaka.appearance(after);
            let (own, own_face, own_bubble) = plain.appearance(after);
            assert_eq!(
                (std::mem::discriminant(&pose), face, bubble),
                (std::mem::discriminant(&own), own_face, own_bubble),
                "{name}"
            );
            let n = osaka.decisions.len();
            osaka.tick(59_999, None, &terrain, &chances, &mut rng);
            assert_eq!((&osaka.act, osaka.decisions.len()), (&act, n), "{name}");
            assert!(osaka.chat_cuts.is_empty(), "{name}: {:?}", osaka.chat_cuts);
        }
        // Looking up from her homework as she writes, she nods off under
        // her look; the next line finds her dozing: it stirs her, and her
        // look is over (a stir shows, never hidden under a look).
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.facing = Facing::Right;
        let act = using(Use::Homework, Furniture::Desk);
        osaka.set(act.clone(), 0);
        osaka.look(29_000, 0, false, &terrain);
        assert_eq!(osaka.appearance(29_000).2, Some(Bubble::Bang));
        osaka.tick(31_000, None, &terrain, &chances, &mut rng);
        osaka.look(31_000, 0, false, &terrain);
        assert_eq!(osaka.act, act);
        assert_eq!(
            osaka.appearance(31_000),
            (Pose::Homework(2), Face::Blink, Some(Bubble::Say(STIRRED))),
            "stirred over the look"
        );
        assert_eq!(osaka.facing, Facing::Right, "back to her desk");
        let after = 31_000 + speech_ms(STIRRED);
        osaka.tick(after, None, &terrain, &chances, &mut rng);
        assert_eq!(osaka.appearance(after).1, Face::Blink, "dozing on");
    }

    /// What isn't still is cut by a chat line, as ever (phase 5c B1):
    /// walking, pulling, chores (unpacking, crumpling, a snack, petting
    /// the cat), exercise, standing about, her mischief (swapping
    /// letters, giggling at it), making (tearing text), moving a piece
    /// (lifting it, setting it down), Setsubun's beans (thrown on the
    /// spot, but thrown) and dashing home for what she forgot (a dash);
    /// and a still act over text (in line art, where her image hides
    /// it) passes on at once.
    #[test]
    fn what_isnt_still_or_calm_is_cut_by_a_chat_line() {
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::style::Style;
        let calm = floor_at(15);
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
        buf.set_string(0, 15, "─".repeat(40), Style::default());
        for row in 11..15 {
            buf.set_string(0, row, "x".repeat(40), Style::default());
        }
        let wordy = Terrain::read(&buf, &[], true);
        assert!(!wordy.restful(20, 15));
        let using = |what: Use, item: Furniture| Act::Use {
            seat: Seat {
                x: 20,
                y: 15,
                facing: Facing::Right,
                ..seat_for(what, item)
            },
            since: 0,
            until: 60_000,
            whole: 60_000,
            play: Play::of(what, None),
            grievance: None,
        };
        let idle = |what: Activity| Act::Idle {
            what,
            since: 0,
            until: 60_000,
            play: None,
        };
        let swap = Swap {
            x: 20,
            y: 15,
            row: 13,
            side: Side::Right,
            a: Placed::home((22, 13)),
            b: Placed::home((23, 13)),
            glyphs: "ab".to_owned(),
        };
        let episode = trial_episode();
        let build = Build {
            x: 20,
            y: 15,
            row: 13,
            side: Side::Right,
            cells: vec![22, 23, 24],
            glyphs: "abc".to_owned(),
            piece: super::super::room::Shown {
                item: Furniture::Sofa,
                facing: Facing::Left,
                boxed: false,
                strip: None,
                left: 22,
                floor: 15,
                scrap: None,
            },
            then: Use::Lounge,
        };
        let mut cut: Vec<(&str, Act, &Terrain)> = vec![
            (
                "walk",
                Act::Walk {
                    to: 30,
                    then: Then::Nothing,
                },
                &calm,
            ),
            (
                "pull",
                Act::Pull {
                    pull: census_pull(),
                    offset: 0,
                    goal: 3,
                    heaving: false,
                },
                &calm,
            ),
            ("unpack", using(Use::Unpack, Furniture::Sofa), &calm),
            ("crumple", using(Use::Crumple, Furniture::Sofa), &calm),
            ("snack", using(Use::Snack, Furniture::Fridge), &calm),
            ("pet", using(Use::Pet, Furniture::CatBed), &calm),
            ("jacks", idle(Activity::Jacks), &calm),
            ("toe touch", idle(Activity::ToeTouch), &calm),
            ("stretch", idle(Activity::Stretch), &calm),
            ("stand", Act::Stand { until: 60_000 }, &calm),
            (
                "swap",
                Act::Swap {
                    swap: swap.clone(),
                    until: 60_000,
                    back: false,
                },
                &calm,
            ),
            (
                "giggle",
                Act::Giggle {
                    swap,
                    until: 60_000,
                    revert: 60_000,
                },
                &calm,
            ),
            (
                "tear",
                Act::Tear {
                    build,
                    since: 0,
                    ripped: false,
                    step: 0,
                },
                &calm,
            ),
            (
                "lift",
                Act::Lift {
                    lift: Lift {
                        repair: episode.repair,
                        trials: Trials::default(),
                        x: 20,
                        y: 15,
                        side: Side::Right,
                    },
                    since: 0,
                    until: 60_000,
                },
                &calm,
            ),
            (
                "set down",
                Act::SetDown {
                    set: SetDown {
                        piece: Furniture::Sofa,
                        to: episode.repair.to,
                        x: 20,
                        y: 15,
                        side: Side::Right,
                    },
                    since: 0,
                    until: 60_000,
                },
                &calm,
            ),
            ("setsubun", Osaka::setsubun(0), &calm),
            (
                "dash forgot",
                Act::SpaceOut {
                    since: 0,
                    until: DASH_FORGOT_MS,
                    play: Some(Play::plain(ScriptId::DashForgot)),
                    session: None,
                },
                &calm,
            ),
        ];
        for (name, act, ..) in still_acts() {
            cut.push((name, act, &wordy));
        }
        for (name, act, terrain) in cut {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.set(act, 0);
            osaka.look(5_000, 0, false, terrain);
            assert!(
                matches!(osaka.act, Act::Look { .. }),
                "{name}: {:?}",
                osaka.act
            );
        }
    }

    /// A riddle she's telling when a chat line comes: what it said under
    /// her look is said after it, in order (the question again, then the
    /// answer: never a punchline without its setup); a line after the
    /// answer has shown owes nothing. Across a lively chat, what the act
    /// said under every look is said after the last. And her act ending
    /// under her look (spacing out runs out), what it said under it is
    /// said as it ends.
    #[test]
    fn a_riddles_answer_under_her_look_is_said_after_it() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let (question, answer) = RIDDLES[0];
        // (until, lines, when the first owed line is said, what's owed)
        let cases: [(u64, &[u64], u64, &[&str]); 4] = [
            (14_000, &[1_000], 1_000 + LOOK_MS, &[question, answer]),
            (14_000, &[script::RIDDLE_ANSWERED_MS + 500], 0, &[]),
            (
                14_000,
                &[2_000, 5_800],
                5_800 + LOOK_MS,
                &[question, answer],
            ),
            (7_000, &[4_000], 7_000, &[answer]),
        ];
        for (until, lines, said_at, owed) in cases {
            let case = format!("until {until}, lines {lines:?}");
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            let act = Act::SpaceOut {
                since: 0,
                until,
                play: Some(Play::riddle(0)),
                session: None,
            };
            osaka.set(act.clone(), 0);
            for &line in lines {
                osaka.tick(line, None, &terrain, &chances, &mut rng);
                osaka.look(line, 0, false, &terrain);
            }
            let last = *lines.last().unwrap();
            if owed.is_empty() {
                let after = last + LOOK_MS;
                osaka.tick(after, None, &terrain, &chances, &mut rng);
                assert_eq!(osaka.act, act, "{case}");
                assert_eq!(osaka.appearance(after).2, None, "{case}");
                continue;
            }
            let mut at = said_at;
            for &text in owed {
                osaka.tick(at, None, &terrain, &chances, &mut rng);
                assert_eq!(
                    osaka.appearance(at).2,
                    Some(Bubble::Say(text)),
                    "{case}: at {at}"
                );
                // Said for its length; or, her look ending less than a
                // frame before that, until a frame after her look's end.
                // (A line her next act says over it ends it as it would.)
                let natural = at + speech_ms(text);
                osaka.tick(natural - 1, None, &terrain, &chances, &mut rng);
                at = match osaka.speech {
                    Some((said, until)) if said == text => {
                        let held = osaka.look_ended.map(|ended| ended + USE_FRAME_MS);
                        assert!(
                            until == natural || (until > natural && Some(until) == held),
                            "{case}: {text} said at {at} until {until}"
                        );
                        until
                    }
                    _ => natural,
                };
            }
            osaka.tick(at, None, &terrain, &chances, &mut rng);
            assert!(
                !owed.contains(&match osaka.appearance(at).2 {
                    Some(Bubble::Say(text)) => text,
                    _ => "",
                }),
                "{case}: said once"
            );
        }
    }

    /// Her still act runs out while she watches the chat from it: she
    /// stands to watch the rest of it, side-on, as after any look (step
    /// 7's settle-in will restate this).
    #[test]
    fn a_still_act_ending_under_her_watch_stands_to_watch() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.set(
            Act::SpaceOut {
                since: 0,
                until: 7_000,
                play: None,
                session: None,
            },
            0,
        );
        osaka.look(5_000, 0, false, &terrain);
        assert!(matches!(osaka.act, Act::SpaceOut { .. }));
        osaka.tick(7_000, None, &terrain, &chances, &mut rng);
        assert_eq!(osaka.act, Act::Stand { until: 10_000 });
        assert_eq!(osaka.appearance(7_000), (Pose::Side, Face::Vacant, None));
        assert_eq!(osaka.facing, Facing::Left, "facing the chat");
    }

    /// A stir isn't a look: it sets no watch. Her doze running out just
    /// after one, she gets on with her day, not standing to watch a chat
    /// she never turned to.
    #[test]
    fn a_doze_ending_after_a_stir_doesnt_stand_to_watch() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.set(
            Act::Idle {
                what: Activity::LieBack,
                since: 0,
                until: 7_000,
                play: None,
            },
            0,
        );
        osaka.look(5_000, 0, false, &terrain);
        assert_eq!(osaka.appearance(5_000).2, Some(Bubble::Say(STIRRED)));
        osaka.tick(7_000, None, &terrain, &chances, &mut rng);
        let decided = osaka.decisions.last().expect("decided at its end");
        assert_eq!(decided.at, 7_000);
        assert_ne!(decided.method, "watching chat", "{decided:?}");
    }

    /// What she's saying as a chat line comes shows on over her look
    /// (as over the standing look before it): only her act's own bubble
    /// is hidden under it.
    #[test]
    fn what_shes_saying_shows_over_her_look() {
        let terrain = floor_at(15);
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.set(
            Act::SpaceOut {
                since: 0,
                until: 60_000,
                play: None,
                session: None,
            },
            0,
        );
        // Said just before the line, it's still showing once her startle
        // is over.
        let said = 5_000 + LOOK_UP_SURPRISED_MS + 100 - speech_ms(OK);
        osaka.say(OK, said);
        osaka.look(5_000, 0, false, &terrain);
        assert_eq!(
            osaka.appearance(5_000),
            (Pose::Stand, Face::Surprised, Some(Bubble::Say(OK)))
        );
        let curious = 5_000 + LOOK_UP_SURPRISED_MS;
        assert_eq!(
            osaka.appearance(curious),
            (Pose::Stand, Face::Curious, Some(Bubble::Say(OK)))
        );
        let over = said + speech_ms(OK);
        assert_eq!(
            osaka.appearance(over),
            (Pose::Stand, Face::Curious, Some(Bubble::Huh))
        );
    }

    /// Nodding off over her homework under her look, the look is over:
    /// a doze never wears one. At the nod-off she shows her act's own
    /// look, as if no line had come.
    #[test]
    fn nodding_off_under_her_look_ends_it() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.facing = Facing::Right;
        let seat = Seat {
            x: 20,
            y: 15,
            facing: Facing::Right,
            ..seat_for(Use::Homework, Furniture::Desk)
        };
        osaka.set(
            Act::Use {
                seat,
                since: 0,
                until: 60_000,
                whole: 60_000,
                play: Play::of(Use::Homework, None),
                grievance: None,
            },
            0,
        );
        let plain = osaka.clone();
        let nod = (20_000..50_000)
            .find(|&t| plain.appearance(t).0.dozes())
            .expect("nods off");
        osaka.tick(nod - 2_000, None, &terrain, &chances, &mut rng);
        osaka.look(nod - 2_000, 0, false, &terrain);
        assert_eq!(osaka.appearance(nod - 2_000).2, Some(Bubble::Bang));
        osaka.tick(nod, None, &terrain, &chances, &mut rng);
        assert_eq!(osaka.appearance(nod), plain.appearance(nod));
        assert_eq!(osaka.facing, Facing::Right);
        assert!(osaka.looking_up.is_none(), "the look is over");
    }

    /// A chat line as she says what's wrong with her home: she says it
    /// to its end (and feels it), then looks up, `!` and `?` in full,
    /// then watches; her look waits for it, rather than being lost
    /// under it.
    #[test]
    fn a_look_up_waits_for_her_grievance() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let line = FELT.rule().map(|r| r.grievance).unwrap();
        let mut rng = Rng(3);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        let seat = Seat {
            x: 20,
            y: 15,
            facing: Facing::Right,
            ..seat_for(Use::Lounge, Furniture::Sofa)
        };
        let from = USE_FRAME_MS;
        let act = Act::Use {
            seat,
            since: 0,
            until: 60_000,
            whole: 60_000,
            play: Play::of(Use::Lounge, None),
            grievance: Some((FELT, from)),
        };
        osaka.set(act.clone(), 0);
        let chat = from + 500;
        osaka.tick(chat, None, &terrain, &chances, &mut rng);
        osaka.look(chat, 0, false, &terrain);
        assert_eq!(osaka.appearance(chat).2, Some(Bubble::Say(line)));
        let shown = from + GRIEVANCE_MS;
        osaka.tick(shown, None, &terrain, &chances, &mut rng);
        assert!(osaka.has_felt(FELT), "felt");
        assert_eq!(osaka.appearance(shown).1, Face::Surprised);
        assert_eq!(osaka.appearance(shown).2, Some(Bubble::Bang));
        osaka.tick(
            shown + LOOK_UP_SURPRISED_MS,
            None,
            &terrain,
            &chances,
            &mut rng,
        );
        assert_eq!(
            osaka.appearance(shown + LOOK_UP_SURPRISED_MS).2,
            Some(Bubble::Huh)
        );
        osaka.tick(shown + LOOK_MS, None, &terrain, &chances, &mut rng);
        assert_eq!(osaka.appearance(shown + LOOK_MS).1, Face::Vacant);
        assert_eq!(osaka.act, act, "lounging on");
    }

    // Phase 5c step 7: the stillness levers, each turned on in its test
    // (they ship neutral until the band is tuned).

    use super::super::stillness::{ByMood, NodOff};

    /// The design's starting levers, settling as often as `settle` and
    /// dozing where she sits from a sit as often as `sit_doze`.
    fn levers(settle: f64, sit_doze: f64) -> Stillness {
        Stillness {
            settle: ByMood::all(settle),
            sit_doze,
            ..Stillness::STARTING
        }
    }

    /// Her at `(20, 15)` on [`floor_at`]`(15)`, facing left, in `mood`
    /// with the levers `still`, at `act` (from 0, to 10 s) for `want`.
    fn at_still(still: Stillness, mood: Mood, act: Act, want: Want, rng: &mut Rng) -> Osaka {
        let mut osaka = Osaka::standing_at(20, 15, 0, rng);
        osaka.set_mood(mood);
        osaka.stillness = still;
        osaka.facing = Facing::Left;
        osaka.credit = Some(want);
        osaka.set(act, 0);
        osaka
    }

    fn idle_until(what: Activity, until: u64) -> Act {
        Act::Idle {
            what,
            since: 0,
            until,
            play: None,
        }
    }

    fn spacing_out(until: u64, play: Option<Play>) -> Act {
        Act::SpaceOut {
            since: 0,
            until,
            play,
            session: None,
        }
    }

    /// A sofa's seat at `(20, 15)` facing left, for `what`.
    fn sofa_seat(what: Use) -> Seat {
        Seat {
            x: 20,
            y: 15,
            facing: Facing::Left,
            ..seat_for(what, Furniture::Sofa)
        }
    }

    fn lounging(until: u64) -> Act {
        Act::Use {
            seat: sofa_seat(Use::Lounge),
            since: 0,
            until,
            whole: until,
            play: Play::of(Use::Lounge, None),
            grievance: None,
        }
    }

    /// What she's at, as settling cares: the activity, or the use and
    /// its seat.
    fn settled_at(osaka: &Osaka) -> Option<(Want, Option<Seat>)> {
        match osaka.act {
            Act::Idle {
                what, play: None, ..
            } => Some((Want::Idle(what), None)),
            Act::Use { seat, .. } => Some((Want::Use(seat.what), Some(seat))),
            _ => None,
        }
    }

    /// A still act she chose that runs its course settles further where
    /// she is, as often as her mood's odds (here always, in every mood):
    /// spacing out (musing or a riddle) or gazing, to sitting; sitting,
    /// to lying back or (on its whim) dozing where she sits; lounging, to
    /// a nap from the same seat. She doesn't move or turn, sets off for
    /// nothing, and it's no choice of hers (not one she rolled, nor one
    /// her recent choices remember): a continuation, eased as its own
    /// want, the act she settled from eased whole.
    #[test]
    fn a_still_act_that_runs_its_course_settles_in_where_she_is() {
        let terrain = floor_at(15);
        let chances = Chances {
            seats: vec![sofa_seat(Use::Lounge), sofa_seat(Use::Nap)],
            ..Chances::default()
        };
        let sit = Want::Idle(Activity::Sit);
        let cases: Vec<(&str, Act, Want, f64, Want)> = vec![
            (
                "space out",
                spacing_out(10_000, None),
                Want::SpaceOut,
                0.0,
                sit,
            ),
            (
                "riddle",
                spacing_out(10_000, Some(Play::riddle(0))),
                Want::SpaceOut,
                0.0,
                sit,
            ),
            (
                "gaze",
                idle_until(Activity::Gaze, 10_000),
                Want::Idle(Activity::Gaze),
                0.0,
                sit,
            ),
            (
                "sit, lying back",
                idle_until(Activity::Sit, 10_000),
                sit,
                0.0,
                Want::Idle(Activity::LieBack),
            ),
            (
                "sit, dozing",
                idle_until(Activity::Sit, 10_000),
                sit,
                1.0,
                Want::Idle(Activity::SitDoze),
            ),
            (
                "lounge",
                lounging(10_000),
                Want::Use(Use::Lounge),
                0.0,
                Want::Use(Use::Nap),
            ),
            (
                "reading on her back",
                idle_until(Activity::LieRead, 10_000),
                Want::Idle(Activity::LieRead),
                0.0,
                Want::Idle(Activity::BookDoze),
            ),
        ];
        for mood in Mood::ALL {
            for (name, act, want, sit_doze, into) in &cases {
                let at = format!("{name} {mood:?}");
                let mut rng = Rng(5);
                let mut osaka =
                    at_still(levers(1.0, *sit_doze), mood, act.clone(), *want, &mut rng);
                let (decided, set_offs) = (osaka.decisions.len(), osaka.set_offs);
                osaka.tick(10_000, None, &terrain, &chances, &mut rng);
                let decision = osaka.decisions.last().expect("decided");
                assert_eq!(osaka.decisions.len(), decided + 1, "{at}");
                assert_eq!(
                    (decision.bucket, decision.method, decision.want),
                    (Bucket::Continuation, "settle in", None),
                    "{at}: a continuation, not a roll"
                );
                let (now, seat) =
                    settled_at(&osaka).unwrap_or_else(|| panic!("{at}: {:?}", osaka.act));
                assert_eq!(now, *into, "{at}");
                if let Some(seat) = seat {
                    assert_eq!(seat, sofa_seat(Use::Nap), "{at}: the same sofa's seat");
                    assert!(
                        mind::places(Use::Nap, &chances, osaka.whims)
                            .contains(&mind::Place::Seat(seat)),
                        "{at}: a nap there is on offer"
                    );
                }
                assert_eq!((osaka.x, osaka.y), (20, 15), "{at}: where she was");
                assert_eq!(osaka.facing, Facing::Left, "{at}: facing as she was");
                assert_eq!(osaka.set_offs, set_offs, "{at}: no set-off");
                assert!(osaka.choices.is_empty(), "{at}: no choice");
                assert!(osaka.recent.is_empty(), "{at}: not a recent choice");
                assert_eq!(osaka.credit, Some(*into), "{at}: eased as itself");
                assert_eq!(
                    Osaka::credit_path(*into, SETTLE_IN),
                    Some(CreditPath::By(Via::Share)),
                    "{at}: its credit path"
                );
                assert_eq!(osaka.credited.last(), Some(&(*want, 1.0, 10_000)), "{at}");
                // Run to its end, it eases her whole, as itself.
                let (Act::Idle { until, .. } | Act::Use { until, .. }) = osaka.act else {
                    panic!("{at}");
                };
                // A tick handles at most 64 of her moments, and a nap's
                // frames can be more: tick until nothing at its end is left.
                for _ in 0..8 {
                    osaka.tick(until, None, &terrain, &chances, &mut rng);
                    if osaka.due() > until {
                        break;
                    }
                }
                assert!(
                    osaka.credited.contains(&(*into, 1.0, until)),
                    "{at}: {:?}",
                    osaka.credited
                );
            }
        }
    }

    /// Each mood has its own odds of settling in: here a lazy Osaka
    /// always settles and every other mood never does.
    #[test]
    fn her_mood_has_its_own_odds_of_settling_in() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let still = Stillness {
            settle: ByMood {
                lazy: 1.0,
                ..ByMood::all(0.0)
            },
            ..Stillness::STARTING
        };
        for mood in Mood::ALL {
            for seed in 0..8 {
                let mut rng = Rng(seed);
                let mut osaka = at_still(
                    still,
                    mood,
                    spacing_out(10_000, None),
                    Want::SpaceOut,
                    &mut rng,
                );
                osaka.tick(10_000, None, &terrain, &chances, &mut rng);
                let settled = osaka.decisions.last().map(|d| d.method) == Some(SETTLE_IN);
                assert_eq!(settled, mood == Mood::Lazy, "{mood:?} seed {seed}");
            }
        }
    }

    /// A daydream with no musings in it (a mood's whim of none, phase 5c
    /// B6) says no riddle or musing, but her rare musing and a glance up
    /// at her clock aren't musings of hers: they come exactly as they
    /// would with a musing to say.
    #[test]
    fn a_silent_daydream_keeps_her_rare_musing_and_clock_glance() {
        let open = (0..1000)
            .map(|seed| {
                Rares::draw(
                    seed,
                    &[ScriptId::Escalator],
                    Pity::default(),
                    rarity::DAY_WINDOW,
                )
            })
            .find(|r| r.allows(ScriptId::Escalator))
            .expect("a day in seven or so");
        let silent = Stillness {
            musings: ByMood::all((0, 0)),
            ..Stillness::NEUTRAL
        };
        let mut came = [0; 2];
        for seed in 0..300 {
            for (i, rare) in [ScriptId::Escalator, ScriptId::ClockGlance]
                .into_iter()
                .enumerate()
            {
                let daydream = |still: Stillness| {
                    let mut rng = Rng(seed);
                    let mut osaka = Osaka::standing_at(20, 10, 0, &mut rng);
                    osaka.stillness = still;
                    osaka.whims = Whims(seed);
                    if rare == ScriptId::Escalator {
                        osaka.set_rares(open.clone(), Some(0), vec![ScriptId::Escalator]);
                    } else {
                        osaka.read_clock(Some(clock_at(1, 15, 0)), 0);
                        osaka.clock_on = Some(clock_on(20, 10));
                    }
                    osaka.muse(0, &mut rng);
                    (osaka.plays().map(|p| p.own), osaka.speech.is_some())
                };
                let (with, _) = daydream(Stillness::NEUTRAL);
                let (without, speaks) = daydream(silent);
                assert_eq!(
                    without == Some(rare),
                    with == Some(rare),
                    "seed {seed} {rare:?}: {without:?} silent, {with:?} with a musing"
                );
                if without == Some(rare) {
                    came[i] += 1;
                } else {
                    assert_eq!((without, speaks), (None, false), "seed {seed}: silent");
                }
            }
        }
        assert!(came.iter().all(|&n| n >= 20), "{came:?} in 300");
    }

    /// Settling goes on as far as it goes and stops there: spacing out
    /// to sitting, sitting to lying back, and a lying doze ends with a
    /// fresh choice. Each settled act is drawn as long as its own
    /// activity, lingered as her mood lingers.
    #[test]
    fn settling_in_runs_its_chain_to_its_end() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        for mood in Mood::ALL {
            let mut rng = Rng(9);
            let mut osaka = at_still(
                levers(1.0, 0.0),
                mood,
                spacing_out(10_000, None),
                Want::SpaceOut,
                &mut rng,
            );
            let mut chain = Vec::new();
            let mut now = 10_000;
            for _ in 0..3 {
                osaka.tick(now, None, &terrain, &chances, &mut rng);
                let decision = osaka.decisions.last().expect("decided");
                chain.push((decision.method, osaka.credit));
                let Act::Idle {
                    what, since, until, ..
                } = osaka.act
                else {
                    break;
                };
                let (lo, hi) = what.duration();
                let linger = if what.lingers() {
                    osaka.stillness.linger.of(mood)
                } else {
                    1.0
                };
                let drawn = (until - since) as f64 / linger;
                assert!(
                    (lo as f64 - 1.0..=hi as f64 + 1.0).contains(&drawn),
                    "{mood:?} {what:?}: {} ms at ×{linger}",
                    until - since
                );
                now = until;
            }
            let sit = Want::Idle(Activity::Sit);
            let lie = Want::Idle(Activity::LieBack);
            assert_eq!(
                chain[..2],
                [("settle in", Some(sit)), ("settle in", Some(lie))],
                "{mood:?}"
            );
            assert_ne!(
                chain[2].0, "settle in",
                "{mood:?}: a doze is as far as it goes"
            );
        }
    }

    /// Only a still act she chose, that ran its course, on calm floor,
    /// settles (however sure the odds): not one she was startled out of,
    /// nor what she didn't choose (the moment after a swap, a glance at
    /// her clock, Setsubun, a trial sit), nor exercise or kicking her
    /// feet, nor at odds of nothing (the neutral levers), nor past her
    /// bedtime, nor over text (where she'd only pass).
    #[test]
    fn settling_in_is_only_for_a_still_act_she_chose_that_ran_its_course() {
        let terrain = floor_at(15);
        let chances = Chances {
            seats: vec![sofa_seat(Use::Lounge), sofa_seat(Use::Nap)],
            ..Chances::default()
        };
        let glance = Play {
            branch: ClockGlance::Hour(15).branch(),
            ..Play::plain(ScriptId::ClockGlance)
        };
        let cases: Vec<(&str, Act, Option<Want>, Stillness)> = vec![
            (
                "after a swap",
                spacing_out(10_000, None),
                None,
                levers(1.0, 0.0),
            ),
            (
                "a glance",
                spacing_out(10_000, Some(glance)),
                Some(Want::SpaceOut),
                levers(1.0, 0.0),
            ),
            (
                "setsubun",
                spacing_out(10_000, Some(Play::plain(ScriptId::Setsubun))),
                Some(Want::SpaceOut),
                levers(1.0, 0.0),
            ),
            (
                "lie front",
                idle_until(Activity::LieFront, 10_000),
                Some(Want::Idle(Activity::LieFront)),
                levers(1.0, 0.0),
            ),
            (
                "jacks",
                idle_until(Activity::Jacks, 10_000),
                Some(Want::Idle(Activity::Jacks)),
                levers(1.0, 0.0),
            ),
            (
                "neutral levers",
                spacing_out(10_000, None),
                Some(Want::SpaceOut),
                Stillness::NEUTRAL,
            ),
            (
                "odds of nothing",
                idle_until(Activity::Sit, 10_000),
                Some(Want::Idle(Activity::Sit)),
                levers(0.0, 0.0),
            ),
        ];
        for (name, act, want, still) in cases {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.stillness = still;
            osaka.credit = want;
            osaka.set(act, 0);
            osaka.tick(10_000, None, &terrain, &chances, &mut rng);
            let method = osaka.decisions.last().map(|d| d.method);
            assert_ne!(method, Some("settle in"), "{name}");
        }
        // Deciding before it has run its course (as the stage's tests
        // make her).
        let mut rng = Rng(5);
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Ordinary,
            spacing_out(10_000, None),
            Want::SpaceOut,
            &mut rng,
        );
        osaka.decide(5_000, &terrain, &chances, &mut rng);
        let method = osaka.decisions.last().map(|d| d.method);
        assert_ne!(method, Some("settle in"), "before its end");
        // Startled out of it (it didn't run its course).
        let mut rng = Rng(5);
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Ordinary,
            spacing_out(10_000, None),
            Want::SpaceOut,
            &mut rng,
        );
        osaka.interrupt(Cause::Shaken, 5_000);
        osaka.tick(5_000 + LOOK_MS, None, &terrain, &chances, &mut rng);
        assert!(
            osaka.decisions.iter().all(|d| d.method != "settle in"),
            "startled: {:?}",
            osaka.decisions
        );
        // A trial sit, on the sofa she has set down: she makes up her
        // mind about it (keeping it there, or trying it elsewhere), and
        // doesn't settle in.
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Ordinary,
            lounging(10_000),
            Want::Use(Use::Lounge),
            &mut rng,
        );
        osaka.episode = Some(trial_episode());
        osaka.tick(10_000, None, &terrain, &chances, &mut rng);
        let method = osaka.decisions.last().map(|d| d.method);
        assert_ne!(method, Some("settle in"), "a trial sit: {method:?}");
        // (Asked outright, too.)
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Ordinary,
            lounging(10_000),
            Want::Use(Use::Lounge),
            &mut rng,
        );
        osaka.episode = Some(trial_episode());
        assert!(
            osaka
                .settling(
                    &(lounging(10_000), Some(Want::Use(Use::Lounge))),
                    &terrain,
                    &chances,
                    Whims(1),
                    10_000
                )
                .is_none(),
            "a trial sit"
        );
        // Over text, asked outright (her tick moves her off it first):
        // only on calm floor does the same ask settle.
        let osaka = at_still(
            levers(1.0, 0.0),
            Mood::Ordinary,
            spacing_out(10_000, None),
            Want::SpaceOut,
            &mut rng,
        );
        let ended = (spacing_out(10_000, None), Some(Want::SpaceOut));
        for graphics in [false, true] {
            use tuirealm::ratatui::buffer::Buffer;
            use tuirealm::ratatui::style::Style;
            let mut buf = Buffer::empty(Rect::new(0, 0, 40, 20));
            buf.set_string(0, 15, "─".repeat(40), Style::default());
            for row in 11..15 {
                buf.set_string(0, row, "x".repeat(40), Style::default());
            }
            // Text is no calm floor in line art, where her image hides
            // it; in ASCII it is.
            let busy = Terrain::read(&buf, &[], graphics);
            assert_eq!(busy.restful(20, 15), !graphics);
            assert_eq!(
                osaka
                    .settling(&ended, &busy, &chances, Whims(1), 10_000)
                    .is_some(),
                !graphics,
                "over text, graphics {graphics}"
            );
        }
        assert_eq!(
            osaka.settling(&ended, &terrain, &chances, Whims(1), 10_000),
            Some(Settle::Idle(Activity::Sit)),
            "on calm floor"
        );
        // Past her bedtime: to bed.
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Ordinary,
            spacing_out(10_000, None),
            Want::SpaceOut,
            &mut rng,
        );
        osaka.set_clock(monday_at(23, 0).day(0));
        osaka.tick(10_000, Some(monday_at(23, 0)), &terrain, &chances, &mut rng);
        let method = osaka.decisions.last().map(|d| d.method);
        assert_ne!(method, Some("settle in"), "at bedtime: {method:?}");
    }

    /// Lounging settles into a nap only from the same seat of the same
    /// sofa, as napping there is on offer: with no nap seat, the nap seat
    /// somewhere else, or another piece's there, she chooses afresh.
    #[test]
    fn a_lounge_settles_into_a_nap_only_on_the_same_seat() {
        let terrain = floor_at(15);
        let elsewhere = Seat {
            x: 28,
            ..sofa_seat(Use::Nap)
        };
        let another_piece = Seat {
            piece: PieceRef::Real(Furniture::Bed),
            item: Furniture::Bed,
            ..sofa_seat(Use::Nap)
        };
        for (name, seats) in [
            ("no nap seat", vec![sofa_seat(Use::Lounge)]),
            ("another seat", vec![sofa_seat(Use::Lounge), elsewhere]),
            (
                "another piece's seat there",
                vec![sofa_seat(Use::Lounge), another_piece],
            ),
        ] {
            let chances = Chances {
                seats,
                ..Chances::default()
            };
            let mut rng = Rng(5);
            let mut osaka = at_still(
                levers(1.0, 0.0),
                Mood::Lazy,
                lounging(10_000),
                Want::Use(Use::Lounge),
                &mut rng,
            );
            osaka.tick(10_000, None, &terrain, &chances, &mut rng);
            let method = osaka.decisions.last().map(|d| d.method);
            assert_ne!(method, Some("settle in"), "{name}");
        }
    }

    /// Lounging on a sofa she made, with her real one elsewhere: napping
    /// on hers is on offer only on the whim that has her go for the one
    /// she made ([`mind::places`]), and she settles into it exactly then.
    #[test]
    fn a_lounge_on_a_sofa_she_made_settles_as_napping_there_is_offered() {
        let terrain = floor_at(15);
        let made = |what: Use| Seat {
            piece: PieceRef::Made(MadeId(0)),
            ..sofa_seat(what)
        };
        let real = Seat {
            x: 30,
            ..sofa_seat(Use::Nap)
        };
        let chances = Chances {
            seats: vec![made(Use::Lounge), made(Use::Nap), real],
            ..Chances::default()
        };
        let mut seen = [false; 2];
        for seed in 0..60 {
            let mut rng = Rng(seed);
            let mut osaka = at_still(
                levers(1.0, 0.0),
                Mood::Lazy,
                Act::Use {
                    seat: made(Use::Lounge),
                    since: 0,
                    until: 10_000,
                    whole: 10_000,
                    play: Play::of(Use::Lounge, None),
                    grievance: None,
                },
                Want::Use(Use::Lounge),
                &mut rng,
            );
            osaka.tick(10_000, None, &terrain, &chances, &mut rng);
            let settled = osaka.decisions.last().map(|d| d.method) == Some("settle in");
            let offered = mind::places(Use::Nap, &chances, osaka.whims)
                .contains(&mind::Place::Seat(made(Use::Nap)));
            assert_eq!(settled, offered, "seed {seed}");
            seen[usize::from(settled)] = true;
        }
        assert_eq!(seen, [true; 2], "both ways");
    }

    /// A still act that runs its course while she watches the chat
    /// (looking up where she is) settles first, and the watch goes on in
    /// her new pose: her look carried over (puzzled still, then the
    /// plain watching face) until the watch is over, turned to the chat
    /// throughout. Settling into a doze, she wears no look. With odds of
    /// nothing, she stands to watch the rest, as after any look.
    #[test]
    fn settling_in_during_her_watch_watches_on_in_the_new_pose() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let line = 8_000;
        let end = 10_000;
        let watching = |still: Stillness, act: Act, want: Want| {
            let mut rng = Rng(5);
            let mut osaka = at_still(still, Mood::Ordinary, act, want, &mut rng);
            osaka.tick(line, None, &terrain, &chances, &mut rng);
            osaka.look(line, 39, false, &terrain);
            assert!(osaka.looking_up.is_some(), "looking up");
            osaka.tick(end, None, &terrain, &chances, &mut rng);
            (osaka, rng)
        };
        let (mut osaka, mut rng) =
            watching(levers(1.0, 0.0), spacing_out(end, None), Want::SpaceOut);
        assert_eq!(osaka.decisions.last().map(|d| d.method), Some("settle in"));
        assert!(matches!(
            osaka.act,
            Act::Idle {
                what: Activity::Sit,
                ..
            }
        ));
        assert_eq!(osaka.facing, Facing::Right, "turned to the chat");
        assert_eq!(
            osaka.appearance(end),
            (Pose::Sit, Face::Curious, Some(Bubble::Huh)),
            "puzzled still, sitting"
        );
        let plain = line + LOOK_MS;
        osaka.tick(plain, None, &terrain, &chances, &mut rng);
        assert_eq!(osaka.appearance(plain), (Pose::Sit, Face::Vacant, None));
        assert!(osaka.looking_up.is_some(), "watching on");
        osaka.tick(
            line + WATCH_MS.max(LOOK_UP_MS),
            None,
            &terrain,
            &chances,
            &mut rng,
        );
        assert!(osaka.looking_up.is_none(), "the watch is over");
        assert!(matches!(osaka.act, Act::Idle { .. }), "sitting on");
        assert_eq!(osaka.facing, Facing::Right);
        // Into a doze (lying back, or dozing where she sits): no look,
        // whether her tick decides (ending any look she nods off under)
        // or the stage has her decide outright.
        for (sit_doze, into) in [(0.0, Activity::LieBack), (1.0, Activity::SitDoze)] {
            let (osaka, _) = watching(
                levers(1.0, sit_doze),
                idle_until(Activity::Sit, end),
                Want::Idle(Activity::Sit),
            );
            assert_eq!(osaka.decisions.last().map(|d| d.method), Some("settle in"));
            assert!(
                matches!(osaka.act, Act::Idle { what, .. } if what == into),
                "{into:?}"
            );
            assert!(osaka.appearance(end).0.dozes(), "{into:?}");
            assert!(osaka.looking_up.is_none(), "{into:?}: a doze wears no look");
            let mut rng = Rng(5);
            let mut osaka = at_still(
                levers(1.0, sit_doze),
                Mood::Ordinary,
                idle_until(Activity::Sit, end),
                Want::Idle(Activity::Sit),
                &mut rng,
            );
            osaka.tick(line, None, &terrain, &chances, &mut rng);
            osaka.look(line, 39, false, &terrain);
            osaka.decide(end, &terrain, &chances, &mut rng);
            assert_eq!(osaka.decisions.last().map(|d| d.method), Some("settle in"));
            assert!(
                osaka.looking_up.is_none(),
                "{into:?}, decided: a doze wears no look"
            );
        }
        // Lounging into a nap on the same sofa: turned to the chat as she
        // lounged, then (a nap is a doze, its look over) back to the
        // seat's facing, as any look's end turns her.
        let sofa = Chances {
            seats: vec![sofa_seat(Use::Lounge), sofa_seat(Use::Nap)],
            ..Chances::default()
        };
        let mut rng = Rng(5);
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Ordinary,
            lounging(end),
            Want::Use(Use::Lounge),
            &mut rng,
        );
        osaka.tick(line, None, &terrain, &sofa, &mut rng);
        osaka.look(line, 39, false, &terrain);
        assert_eq!(osaka.facing, Facing::Right, "lounging, turned to the chat");
        osaka.tick(end, None, &terrain, &sofa, &mut rng);
        assert_eq!(osaka.decisions.last().map(|d| d.method), Some("settle in"));
        assert!(matches!(osaka.act, Act::Use { seat, .. } if seat.what == Use::Nap));
        assert!(osaka.looking_up.is_none(), "a nap wears no look");
        assert_eq!(osaka.facing, Facing::Left, "as the seat faces");
        // Odds of nothing: she stands to watch.
        let (osaka, _) = watching(levers(0.0, 0.0), spacing_out(end, None), Want::SpaceOut);
        assert_eq!(
            osaka.decisions.last().map(|d| d.method),
            Some("watching chat")
        );
        assert!(matches!(osaka.act, Act::Stand { .. }));
    }

    /// Settling comes after what she owes, is moving or made (phase 5c
    /// M7), however sure the odds, and whether or not she was watching
    /// the chat as her still act ran out: a beat she owes (a glance
    /// toward what she lost), the piece she's moving (here, waiting to
    /// see where it went) or has just set down (she sits back down to
    /// it), a piece she made and hasn't finished with. Watching, she stands the rest of the
    /// watch out (as she did before settling was), and what she owes
    /// comes next.
    #[test]
    fn settling_in_waits_behind_what_she_owes() {
        let terrain = floor_at(15);
        let line = 8_000;
        let end = 10_000;
        let heap = Mine {
            id: MadeId(0),
            item: Furniture::Sofa,
            purpose: Use::Lounge,
            done: false,
            used: false,
            at: (26, 15),
        };
        let heap_seat = Seat {
            x: 26,
            y: 15,
            piece: PieceRef::Made(MadeId(0)),
            ..seat_for(Use::Crumple, Furniture::Sofa)
        };
        type Setup = fn(&mut Osaka);
        let cases: Vec<(&str, Setup, Chances, &str)> = vec![
            (
                "a beat she owes",
                |o| o.owe(Loss::Tear, (26, 15)),
                Chances::default(),
                "beat",
            ),
            (
                "a piece she made",
                |_| {},
                Chances {
                    mine: vec![heap],
                    seats: vec![heap_seat],
                    ..Chances::default()
                },
                "leftover",
            ),
            (
                "moving a piece",
                |o| {
                    o.episode = Some(Episode {
                        trying: false,
                        ..trial_episode()
                    })
                },
                Chances::default(),
                "waiting",
            ),
            (
                "a piece just set down",
                |o| o.just_set = Some((Furniture::Sofa, Use::Lounge)),
                Chances {
                    seats: vec![Seat {
                        x: 26,
                        ..sofa_seat(Use::Lounge)
                    }],
                    ..Chances::default()
                },
                "arrange/use-it",
            ),
        ];
        for (name, setup, chances, owed) in &cases {
            for watching in [false, true] {
                let at = format!("{name}, watching {watching}");
                let mut rng = Rng(5);
                let mut osaka = at_still(
                    levers(1.0, 0.0),
                    Mood::Ordinary,
                    spacing_out(end, None),
                    Want::SpaceOut,
                    &mut rng,
                );
                if watching {
                    osaka.tick(line, None, &terrain, chances, &mut rng);
                    osaka.look(line, 39, false, &terrain);
                    assert!(osaka.looking_up.is_some(), "{at}: looking up");
                }
                setup(&mut osaka);
                osaka.tick(end, None, &terrain, chances, &mut rng);
                let mut methods: Vec<_> = osaka.decisions.iter().map(|d| d.method).collect();
                if watching {
                    assert_eq!(methods.last(), Some(&"watching chat"), "{at}");
                    osaka.tick(
                        line + WATCH_MS.max(LOOK_UP_MS),
                        None,
                        &terrain,
                        chances,
                        &mut rng,
                    );
                    methods = osaka.decisions.iter().map(|d| d.method).collect();
                }
                assert_eq!(methods.last(), Some(owed), "{at}: {methods:?}");
                assert!(!methods.contains(&"settle in"), "{at}: {methods:?}");
            }
        }
    }

    /// Dozing where she sits: her head sinks (the first frame) and then
    /// rests on her knees, held, eyes shut, zzz; a chat line only stirs
    /// her.
    #[test]
    fn dozing_where_she_sits_nods_then_holds() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(5);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.set(idle_until(Activity::SitDoze, 30_000), 0);
        assert_eq!(
            osaka.appearance(0),
            (Pose::SitDoze(0), Face::Blink, Some(Bubble::Zzz))
        );
        assert_eq!(osaka.first_due(0), SIT_DOZE_NOD_MS);
        osaka.tick(SIT_DOZE_NOD_MS, None, &terrain, &chances, &mut rng);
        for at in (SIT_DOZE_NOD_MS..30_000).step_by(700) {
            assert_eq!(osaka.appearance(at).0, Pose::SitDoze(1), "{at}");
        }
        assert_eq!(osaka.first_due(SIT_DOZE_NOD_MS), 30_000, "held");
        osaka.look(5_000, 0, false, &terrain);
        assert_eq!(osaka.watch_until, 0, "a stir, not a look");
        // Stirring, her head comes up off her knees a moment, "Mm?".
        assert_eq!(
            osaka.appearance(5_000),
            (Pose::SitDoze(0), Face::Blink, Some(Bubble::Say(STIRRED)))
        );
    }

    /// Her mood lingers over the still acts she chooses (each drawn as
    /// before, then times her mood's linger), the whole of the design's
    /// list and nothing else: sitting, lying back, gazing, dozing where
    /// she sits; spacing out (plain, a musing, her rare musing); and the
    /// still uses (lounging, napping, a day's sleep, reading, looking
    /// out, watching TV: it holds a picture, phase 5c D7; a watch by its
    /// own table, `Stillness::watch`). Not lying on her front, exercise,
    /// homework (its nod-off moves instead), chores, a snack, the cat,
    /// nor a trial sit. Without the levers, nothing lingers.
    #[test]
    fn her_mood_lingers_over_the_still_acts_she_chooses() {
        let terrain = floor_at(15);
        let lingering = Stillness {
            linger: ByMood {
                ordinary: 1.25,
                lazy: 2.0,
                industrious: 0.5,
                dreamy: 1.5,
            },
            // A watch by its own table, but where it leaves a mood to
            // the linger (dreamy).
            watch: ByMood {
                ordinary: Some(0.75),
                lazy: Some(1.75),
                industrious: Some(1.1),
                dreamy: None,
            },
            ..Stillness::NEUTRAL
        };
        // How long `start` sets her at, from a fresh Osaka with `still`
        // in `mood` (her body's stream and her whims the same each time).
        let length = |still: Stillness, mood: Mood, start: &dyn Fn(&mut Osaka, &mut Rng)| {
            let mut rng = Rng(11);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.set_mood(mood);
            osaka.stillness = still;
            osaka.splice_rows = &[];
            osaka.whims = Whims(11);
            start(&mut osaka, &mut rng);
            match osaka.act {
                Act::Idle { since, until, .. }
                | Act::SpaceOut { since, until, .. }
                | Act::Use { since, until, .. } => until - since,
                Act::Borrow {
                    since,
                    phase: Borrowing::Read { until },
                    ..
                } => until - since,
                ref other => panic!("{other:?}"),
            }
        };
        type Start<'a> = Box<dyn Fn(&mut Osaka, &mut Rng) + 'a>;
        let mut cases: Vec<(String, Start, bool)> = Vec::new();
        for what in Activity::ALL {
            // Her homework on the floor nods off where her mood has it
            // instead (as at her desk).
            let lingers = matches!(
                what,
                Activity::Sit
                    | Activity::LieBack
                    | Activity::Gaze
                    | Activity::SitDoze
                    | Activity::LieRead
                    | Activity::BookDoze
                    | Activity::UnderSill
                    | Activity::CloudWatch
            );
            cases.push((
                format!("{what:?}"),
                Box::new(move |o: &mut Osaka, rng: &mut Rng| o.idle(what, 0, rng)),
                lingers,
            ));
        }
        let terrain = &terrain;
        for book in [false, true] {
            cases.push((
                format!("floor homework, book {book}"),
                Box::new(move |o: &mut Osaka, rng: &mut Rng| {
                    assert!(o.plan(
                        Want::Idle(Activity::FloorHomework),
                        Bind::Here(Here::FloorHomework { book }),
                        0,
                        terrain,
                        &Chances::default(),
                        0,
                        rng
                    ));
                }),
                false,
            ));
        }
        cases.push((
            "space out".into(),
            Box::new(move |o: &mut Osaka, rng: &mut Rng| {
                assert!(o.plan(
                    Want::SpaceOut,
                    Bind::Here(Here::SpaceOut),
                    0,
                    terrain,
                    &Chances::default(),
                    0,
                    rng
                ));
            }),
            true,
        ));
        cases.push((
            "muse".into(),
            Box::new(|o: &mut Osaka, rng: &mut Rng| o.muse(0, rng)),
            true,
        ));
        cases.push((
            "rare musing".into(),
            Box::new(|o: &mut Osaka, rng: &mut Rng| {
                o.cue(Some(Cue::Script(ScriptId::Escalator)));
                o.muse(0, rng);
                assert_eq!(o.plays().map(|p| p.own), Some(ScriptId::Escalator));
            }),
            true,
        ));
        // A strip borrowed off a line, all in her hands: her read of it.
        cases.push((
            "a borrowed strip".into(),
            Box::new(move |o: &mut Osaka, rng: &mut Rng| {
                let pull = Pull {
                    x: 20,
                    y: 15,
                    row: 13,
                    side: Side::Right,
                    cells: (24..32).collect(),
                    glyphs: "abcdefgh".into(),
                    gap: 0,
                };
                let steps = pull.strip_steps();
                o.borrowing(
                    pull,
                    Borrowing::Reel(steps),
                    0,
                    terrain,
                    &Chances::default(),
                    rng,
                );
            }),
            true,
        ));
        let using = |what: Use, trying: bool| {
            let item = Furniture::ALL
                .into_iter()
                .find(|item| item.spec().uses.contains(&what))
                .unwrap_or(Furniture::Sofa);
            move |o: &mut Osaka, rng: &mut Rng| {
                o.episode = trying.then(trial_episode);
                let seat = Seat {
                    x: 20,
                    y: 15,
                    ..seat_for(what, item)
                };
                o.start_job(Job::Use(seat), 0, &Chances::default(), rng);
            }
        };
        for what in Use::ALL {
            let lingers = matches!(
                what,
                Use::Lounge | Use::Nap | Use::Sleep | Use::Read | Use::LookOut | Use::Watch
            );
            cases.push((format!("{what:?}"), Box::new(using(what, false)), lingers));
        }
        let watch = format!("{:?}", Use::Watch);
        cases.push((
            "trial sit".into(),
            Box::new(using(Use::Lounge, true)),
            false,
        ));
        for mood in Mood::ALL {
            for (name, start, lingers) in &cases {
                let plain = length(Stillness::NEUTRAL, mood, start.as_ref());
                let long = length(lingering, mood, start.as_ref());
                let times = match lingers {
                    false => 1.0,
                    true if *name == watch => lingering.watch_of(mood),
                    true => lingering.linger.of(mood),
                };
                assert_eq!(
                    long,
                    (plain as f64 * times).round() as u64,
                    "{name} {mood:?}"
                );
            }
        }
        // The band's guard reads the lingered tables.
        let slot = Some(routine::Slot::Afternoon);
        let longest = longest_still_ms_with(slot, &lingering);
        assert!(
            longest >= use_duration_in(Use::Sleep, slot).1 * 2,
            "a lazy day's sleep, lingered: {longest}"
        );
        assert!(
            longest <= 2 * longest_still_ms_with(slot, &Stillness::NEUTRAL),
            "{longest}"
        );
    }

    /// With the levers she ships with, an industrious Osaka's watch is as
    /// long as it's drawn (the user, phase 5c step 12c: "Don't shorten
    /// industrious watching"), while her other still uses are shortened
    /// as her mood lingers, and every other mood's watch lingers as that
    /// mood does.
    #[test]
    fn industrious_watching_is_not_shortened() {
        let length = |still: Stillness, mood: Mood, what: Use| {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.set_mood(mood);
            osaka.stillness = still;
            osaka.splice_rows = &[];
            osaka.whims = Whims(5);
            let item = Furniture::ALL
                .into_iter()
                .find(|item| item.spec().uses.contains(&what))
                .unwrap();
            let seat = Seat {
                x: 20,
                y: 15,
                ..seat_for(what, item)
            };
            osaka.start_job(Job::Use(seat), 0, &Chances::default(), &mut rng);
            match osaka.act {
                Act::Use { since, until, .. } => until - since,
                ref other => panic!("{other:?}"),
            }
        };
        let shipped = stillness::SHIPPED;
        for mood in Mood::ALL {
            for what in [Use::Watch, Use::Lounge, Use::Read] {
                let plain = length(Stillness::NEUTRAL, mood, what);
                let times = if what == Use::Watch && mood == Mood::Industrious {
                    1.0
                } else {
                    shipped.linger.of(mood)
                };
                assert_eq!(
                    length(shipped, mood, what),
                    (plain as f64 * times).round() as u64,
                    "{what:?} {mood:?}"
                );
            }
        }
        assert!(
            shipped.linger.of(Mood::Industrious) < 1.0,
            "her other still uses are still shortened"
        );
    }

    /// A strip she borrowed to read (phase 5c D5) eases what reading
    /// does by the share of her reading done: none while she braces or
    /// reels it in, the share of the read so far, all of it once she's
    /// sliding it back; on the floor, and as rest (her room's beauty
    /// eases her too). The census has her at floor rest while she reads,
    /// at mischief at the text.
    #[test]
    fn a_borrowed_read_is_credited_by_the_share_read_as_floor_rest() {
        let pull = Pull {
            x: 20,
            y: 15,
            row: 13,
            side: Side::Right,
            cells: (24..32).collect(),
            glyphs: "abcdefgh".into(),
            gap: 0,
        };
        let read = Want::Use(Use::Read);
        let (since, until, at) = (1_000, 11_000, 6_000);
        let cases = [
            (Borrowing::Brace, 0.0, "mischief"),
            (Borrowing::Reel(2), 0.0, "mischief"),
            (Borrowing::Read { until }, 0.5, "floor rest"),
            (Borrowing::Slide(1), 1.0, "mischief"),
        ];
        for (phase, share, group) in cases {
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.beauty_here = 1.0;
            osaka.credit = Some(read);
            osaka.act = Act::Borrow {
                pull: pull.clone(),
                since,
                phase,
            };
            assert_eq!(osaka.census_group(), group, "{phase:?}");
            let mut want = osaka.clone();
            osaka.credit_done(at);
            assert_eq!(osaka.credited, vec![(read, share, at)], "{phase:?}");
            let spots: Vec<Spot> = osaka.served.iter().map(|s| s.spot).collect();
            assert_eq!(spots, vec![Spot::Floor], "{phase:?}: on the floor");
            want.serve(read, share, Spot::Floor, Via::Share, at);
            want.needs.serve(Need::Beauty, share);
            assert_eq!(osaka.needs, want.needs, "{phase:?}: on the floor, resting");
        }
    }

    /// The band's guard sees a chain she settles along as one still
    /// stretch (phase 5c M7, B5), once any mood settles in: spacing out
    /// or gazing, sitting, then lying back or dozing where she sits;
    /// lounging, then napping on the same sofa; reading on her back,
    /// then dozing under the book; leaning on her window's sill, sitting
    /// in front of it, then dozing there or watching the clouds; each
    /// link as long as its longest, lingered as the most lingering mood
    /// lingers.
    #[test]
    fn the_band_guard_sees_her_settling_chains_whole() {
        let slot = Some(routine::Slot::Afternoon);
        let lingering = Stillness {
            linger: ByMood::all(1.5),
            ..Stillness::NEUTRAL
        };
        assert_eq!(settle_chains_ms(slot, &lingering), [0; 4], "none settle");
        let settling = Stillness {
            settle: ByMood {
                lazy: 0.5,
                ..ByMood::all(0.0)
            },
            ..lingering
        };
        let l = |ms: u64| (ms as f64 * 1.5).round() as u64;
        let longest = |what: Activity| l(what.duration().1);
        let space_out = l(SPACE_OUT_MS.1).max(longest(Activity::Gaze));
        let doze = longest(Activity::LieBack).max(longest(Activity::SitDoze));
        let [here, sofa, book, sill] = settle_chains_ms(slot, &settling);
        assert_eq!(here, space_out + longest(Activity::Sit) + doze);
        assert_eq!(
            book,
            longest(Activity::LieRead) + longest(Activity::BookDoze)
        );
        assert_eq!(
            sill,
            l(use_duration_in(Use::LookOut, slot).1)
                + longest(Activity::UnderSill)
                + longest(Activity::SitDoze).max(longest(Activity::CloudWatch))
        );
        let (lounge, nap) = (
            use_duration_in(Use::Lounge, slot).1,
            use_duration_in(Use::Nap, slot).1,
        );
        assert!(
            sofa >= l(lounge) + l(nap),
            "{sofa}: with their preludes and codas"
        );
        assert!(longest_still_ms_with(slot, &settling) >= here.max(sofa).max(book).max(sill));
    }

    /// Her homework nods off where her mood has it (phase 5c M8): a
    /// third of the way through lazy, halfway ordinary (and dreamy), five
    /// sixths industrious; with the neutral levers, halfway in every
    /// mood. Nodding, then asleep on the paper, for half what's
    /// left each.
    #[test]
    fn her_homework_nods_off_where_her_mood_has_it() {
        for (still, mood, write) in [
            (Stillness::STARTING, Mood::Lazy, (1, 3)),
            (Stillness::STARTING, Mood::Ordinary, (1, 2)),
            (Stillness::STARTING, Mood::Dreamy, (1, 2)),
            (Stillness::STARTING, Mood::Industrious, (5, 6)),
            (Stillness::NEUTRAL, Mood::Lazy, (1, 2)),
            (Stillness::NEUTRAL, Mood::Industrious, (1, 2)),
        ] {
            let at = format!("{mood:?} neutral {}", still == Stillness::NEUTRAL);
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.set_mood(mood);
            osaka.stillness = still;
            osaka.splice_rows = &[];
            let seat = Seat {
                x: 20,
                y: 15,
                ..seat_for(Use::Homework, Furniture::Desk)
            };
            osaka.start_job(Job::Use(seat), 0, &Chances::default(), &mut rng);
            let Act::Use { until, play, .. } = osaka.act else {
                panic!("{at}: homework");
            };
            assert_eq!(play.branch, still.nod_off.of(mood).branch(), "{at}");
            let nods = until * write.0 / write.1;
            let sleeps = nods + (until - nods) / 2;
            let pose = |t: u64| osaka.appearance(t).0;
            assert!(
                matches!(pose(nods - 1), Pose::Homework(0 | 1)),
                "{at}: writing just before {nods}: {:?}",
                pose(nods - 1)
            );
            assert_eq!(pose(nods + 1), Pose::Homework(2), "{at}: nodding");
            assert_eq!(pose(sleeps + 1), Pose::Homework(3), "{at}: asleep");
        }
        assert_eq!(
            NodOff::Half.branch(),
            0,
            "halfway is as homework always was"
        );
    }

    /// A daydream session (phase 5c B6) says its musings in turn: the
    /// first as she sets about it, each next one a gap on, as many as her
    /// mood's whim says while the daydream lasts (a dreamy Osaka's three,
    /// here, an industrious one's none); a musing due while she says
    /// something or looks up at the chat waits for it to be over. Her
    /// body's stream draws only the daydream's length. A riddle is all
    /// its session, and a session of none says nothing.
    #[test]
    fn a_daydream_session_says_its_musings_in_turn() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let levers = Stillness {
            musings: ByMood {
                dreamy: (3, 3),
                industrious: (0, 0),
                ..ByMood::all((1, 1))
            },
            musing_gap: (12_000, 12_000),
            linger: ByMood::all(6.0),
            ..Stillness::NEUTRAL
        };
        // The lines she says from 0 to her daydream's end in `mood`, and
        // when each begins, with a chat line at `chat` if any.
        let said = |mood: Mood, whims: u64, chat: Option<u64>| {
            let mut rng = Rng(whims);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.set_mood(mood);
            osaka.stillness = levers;
            osaka.whims = Whims(whims);
            osaka.credit = Some(Want::SpaceOut);
            osaka.muse(0, &mut rng);
            // One body draw: the daydream's length.
            let mut alone = Rng(whims);
            let _ = Osaka::standing_at(20, 15, 0, &mut alone);
            let _ = alone.range(SPACE_OUT_MS.0, SPACE_OUT_MS.1);
            assert_eq!(rng.0, alone.0, "{whims}: one body draw");
            let Act::SpaceOut {
                until,
                play,
                session,
                ..
            } = osaka.act
            else {
                panic!("spacing out");
            };
            let mut lines: Vec<(u64, &'static str)> = Vec::new();
            let mut now = 0;
            while now < until {
                if chat == Some(now) {
                    osaka.look(now, 0, false, &terrain);
                }
                osaka.tick(now, None, &terrain, &chances, &mut rng);
                if let (_, _, Some(Bubble::Say(text))) = osaka.appearance(now)
                    && lines.last().is_none_or(|&(_, last)| last != text)
                {
                    lines.push((now, text));
                }
                now += 250;
            }
            (lines, play, session, until, osaka)
        };
        let mut sessions = 0;
        for whims in 0..40 {
            let (lines, play, session, until, osaka) = said(Mood::Dreamy, whims, None);
            if play.is_some() {
                assert_eq!(session, None, "{whims}: a riddle is all its session");
                continue;
            }
            sessions += 1;
            assert!(until >= 36_000, "{whims}: lasts long enough ({until})");
            assert_eq!(
                lines.iter().map(|&(at, _)| at).collect::<Vec<_>>(),
                [0, 12_000, 24_000],
                "{whims}: {lines:?}"
            );
            for (_, line) in &lines {
                assert!(mind::MUSINGS.lines.contains(line), "{whims}: {line}");
            }
            let mut distinct: Vec<_> = lines.iter().map(|&(_, l)| l).collect();
            distinct.dedup();
            assert_eq!(distinct.len(), 3, "{whims}: three musings");
            assert!(
                matches!(osaka.act, Act::SpaceOut { session: None, .. }),
                "{whims}: said them all"
            );
            // A chat line just before the second: it waits for her look.
            let (late, ..) = said(Mood::Dreamy, whims, Some(11_000));
            let look = 11_000 + WATCH_MS.max(LOOK_UP_MS);
            // (Sampled every 250 ms.)
            assert_eq!(
                late.iter().map(|&(at, _)| at).collect::<Vec<_>>(),
                [0, look, look + 12_000].map(|at: u64| at.next_multiple_of(250)),
                "{whims}: {late:?}"
            );
            // A chat line whose look is over 600 ms before the second
            // (at 11 400): it waits a frame from the look's end (phase 5c's
            // tail, T4's sibling; the door batch's step 2), and the third a
            // gap after it.
            let (settled, ..) = said(Mood::Dreamy, whims, Some(6_000));
            let ended = 6_000 + WATCH_MS.max(LOOK_UP_MS);
            assert_eq!(ended, 11_400);
            assert_eq!(
                settled.iter().map(|&(at, _)| at).collect::<Vec<_>>(),
                [0, ended + USE_FRAME_MS, ended + USE_FRAME_MS + 12_000]
                    .map(|at: u64| at.next_multiple_of(250)),
                "{whims}: {settled:?}"
            );
            // Industrious, her mood's none: nothing to say.
            let (silent, play, session, ..) = said(Mood::Industrious, whims, None);
            assert!(silent.is_empty(), "{whims}: {silent:?}");
            assert_eq!((play, session), (None, None));
        }
        assert!(sessions >= 10, "{sessions} sessions in 40");
    }

    /// Each musing of a daydream session after the first, and the gap
    /// before it, is drawn from her decision's whims for its place in
    /// the session (phase 5c B6: `whims.series("daydream", k)`): the
    /// gap after the `k`th musing (from 0) by the `k`th of the series,
    /// the `k`th musing by the `k`th; so replayed from those whims, the
    /// same musings at the same times. Those that don't fit before the
    /// daydream's end go unsaid.
    #[test]
    fn each_musing_of_a_session_is_drawn_by_its_place() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let (lo, hi) = (12_000, 20_000);
        let still = Stillness {
            musings: ByMood::all((3, 3)),
            musing_gap: (lo, hi),
            linger: ByMood::all(6.0),
            ..Stillness::NEUTRAL
        };
        let (mut checked, mut gaps) = (0, std::collections::BTreeSet::new());
        for seed in 0..40 {
            let mut rng = Rng(seed);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.stillness = still;
            let w = Whims(seed);
            osaka.whims = w;
            osaka.credit = Some(Want::SpaceOut);
            osaka.muse(0, &mut rng);
            let Act::SpaceOut {
                until, play: None, ..
            } = osaka.act
            else {
                continue;
            };
            let Some((first, _)) = osaka.speech else {
                continue;
            };
            // Replayed: from her lines as they stood after the first.
            let mut lines = osaka.lines.clone();
            let gap = |k: u64| lo + w.series("daydream", k).below("gap", hi - lo + 1);
            let mut want = vec![(0, first)];
            let mut at = 0;
            for k in 1..3 {
                at += gap(k - 1);
                gaps.insert(gap(k - 1));
                if at >= until {
                    break;
                }
                let line = lines
                    .pick(mind::MUSINGS, w.series("daydream", k), at)
                    .expect("a fresh musing");
                want.push((at.div_ceil(250) * 250, line));
            }
            let mut got: Vec<(u64, &'static str)> = Vec::new();
            let mut now = 0;
            while now < until {
                osaka.tick(now, None, &terrain, &chances, &mut rng);
                if let (_, _, Some(Bubble::Say(text))) = osaka.appearance(now)
                    && got.last().is_none_or(|&(_, last)| last != text)
                {
                    got.push((now, text));
                }
                now += 250;
            }
            assert_eq!(got, want, "seed {seed}");
            checked += 1;
        }
        assert!(checked >= 10, "{checked} sessions");
        assert!(gaps.len() >= 10, "the gaps vary: {gaps:?}");
    }

    // Phase 5c D5: her homework and a book on the floor, the paper desk,
    // making while she owns no real piece, watching cross-legged.

    /// A paper desk she made, in shape, to do her homework at: she
    /// kneels beside it at `(20, 15)`, facing left toward it.
    fn paper_desk_seat() -> Seat {
        Seat {
            what: Use::Homework,
            item: Furniture::Desk,
            piece: PieceRef::Made(MadeId(0)),
            x: 20,
            y: 15,
            facing: Facing::Left,
        }
    }

    /// Her homework on the floor, either way (on her front writing, on
    /// her back with the set text), writes (or reads) bobbing on the use
    /// frame, nods off where her mood has it (as at her desk: a third
    /// lazy, half ordinary and dreamy, five sixths industrious; half in
    /// every mood with the neutral levers), dots then asleep: her head
    /// on her arms, or the book open over her eyes. She doesn't linger
    /// over it (its nod-off moves instead). A chat line has her look up
    /// where she is while she writes, and only stirs her dozing.
    #[test]
    fn her_homework_on_the_floor_nods_off_where_her_mood_has_it() {
        let terrain = floor_at(15);
        for book in [false, true] {
            let (at_it, dozed) = if book {
                (Pose::LieRead as fn(u8) -> Pose, Pose::LieRead(2))
            } else {
                (
                    Pose::FloorHomework as fn(u8) -> Pose,
                    Pose::FloorHomework(2),
                )
            };
            for (still, mood, write) in [
                (Stillness::STARTING, Mood::Lazy, (1, 3)),
                (Stillness::STARTING, Mood::Ordinary, (1, 2)),
                (Stillness::STARTING, Mood::Dreamy, (1, 2)),
                (Stillness::STARTING, Mood::Industrious, (5, 6)),
                (Stillness::NEUTRAL, Mood::Lazy, (1, 2)),
                (Stillness::NEUTRAL, Mood::Industrious, (1, 2)),
            ] {
                let at = format!(
                    "book {book} {mood:?} neutral {}",
                    still == Stillness::NEUTRAL
                );
                let mut rng = Rng(3);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.set_mood(mood);
                osaka.stillness = still;
                osaka.floor_homework(book, 0, &mut rng);
                let Act::Idle {
                    what,
                    until,
                    play: Some(play),
                    ..
                } = osaka.act
                else {
                    panic!("{at}: {:?}", osaka.act);
                };
                assert_eq!(what, Activity::FloorHomework, "{at}");
                assert_eq!(play.own, ScriptId::FloorHomework, "{at}");
                assert_eq!(
                    play.branch,
                    script::floor_homework_branch(still.nod_off.of(mood), book),
                    "{at}"
                );
                let (lo, hi) = use_duration(Use::Homework);
                assert!((lo..hi).contains(&until), "{at}: as long as at her desk");
                let nods = until * write.0 / write.1;
                let sleeps = nods + (until - nods) / 2;
                let look = |t: u64| osaka.appearance(t);
                // On its frames, its last held through the part of a
                // frame before she nods off.
                for t in (0..nods).step_by(350) {
                    let frame = (t.min(nods - USE_FRAME_MS) / USE_FRAME_MS % 2) as u8;
                    assert_eq!(look(t).0, at_it(frame), "{at}: at it, {t}");
                }
                assert_eq!(
                    look(nods + 1),
                    (dozed, Face::Blink, Some(Bubble::Dots)),
                    "{at}"
                );
                assert_eq!(
                    look(sleeps + 1),
                    (dozed, Face::Blink, Some(Bubble::Zzz)),
                    "{at}"
                );
                // Her wakeups fall on its frames, then on its keys.
                assert_eq!(osaka.first_due(0), USE_FRAME_MS.min(nods), "{at}");
                // Looking up from writing in place, the act running on.
                let mut writing = osaka.clone();
                writing.look(1_000, 0, false, &terrain);
                assert!(
                    matches!(
                        writing.act,
                        Act::Idle {
                            what: Activity::FloorHomework,
                            ..
                        }
                    ),
                    "{at}: looked up where she was"
                );
                assert!(writing.watch_until > 1_000, "{at}: watching");
                // Dozing, a line only stirs her.
                let mut dozing = osaka.clone();
                dozing.look(sleeps + 1, 0, false, &terrain);
                assert_eq!(dozing.watch_until, 0, "{at}: a stir, not a look");
                assert_eq!(
                    dozing.appearance(sleeps + 1),
                    (dozed, Face::Blink, Some(Bubble::Say(STIRRED))),
                    "{at}"
                );
            }
        }
    }

    /// Reading on her back, a page turns on the use frame (as at her
    /// bookshelf), the book held over her face; settled into from it,
    /// she dozes under it, the book open over her eyes, and a line only
    /// stirs her.
    #[test]
    fn reading_on_her_back_turns_pages_then_dozes_under_the_book() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut rng = Rng(5);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        osaka.set(idle_until(Activity::LieRead, 30_000), 0);
        // On its frames, the last held through the part of a frame
        // before it ends.
        for t in (0..30_000).step_by(350) {
            let frame = (t.min(30_000 - USE_FRAME_MS) / USE_FRAME_MS % 2) as u8;
            assert_eq!(
                osaka.appearance(t),
                (Pose::LieRead(frame), Face::Vacant, None),
                "{t}"
            );
        }
        assert_eq!(osaka.first_due(0), USE_FRAME_MS);
        osaka.set(idle_until(Activity::BookDoze, 30_000), 0);
        osaka.tick(1, None, &terrain, &chances, &mut rng);
        for t in (0..30_000).step_by(700) {
            assert_eq!(
                osaka.appearance(t),
                (Pose::LieRead(2), Face::Blink, Some(Bubble::Zzz)),
                "{t}"
            );
        }
        assert_eq!(osaka.first_due(1), 30_000, "held");
        osaka.look(5_000, 0, false, &terrain);
        assert_eq!(osaka.watch_until, 0, "a stir, not a look");
        assert_eq!(
            osaka.appearance(5_000),
            (Pose::LieRead(2), Face::Blink, Some(Bubble::Say(STIRRED)))
        );
    }

    /// Her homework on the floor over, her back aches, the first time a
    /// visit: she says so ("My back..."), standing while it shows, a
    /// reflex and not a choice; from then on, as long as the visit, it
    /// aches (a second one ends quietly). Cut off by her routine (her
    /// bedtime), it doesn't ache yet.
    #[test]
    fn her_back_aches_once_a_visit_after_homework_on_the_floor() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        for book in [false, true] {
            let mut rng = Rng(7);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.floor_homework(book, 0, &mut rng);
            osaka.credit = Some(Want::Idle(Activity::FloorHomework));
            let Act::Idle { until, .. } = osaka.act else {
                panic!("homework");
            };
            assert!(!osaka.ached);
            osaka.tick(until, None, &terrain, &chances, &mut rng);
            let decision = osaka.decisions.last().expect("decided");
            assert_eq!(decision.method, "ached", "book {book}");
            assert_eq!(decision.want, None, "book {book}: not a choice");
            assert!(osaka.ached, "book {book}");
            let (said, shows) = osaka.speech.expect("said");
            assert_eq!(said, MY_BACK, "book {book}");
            assert_eq!(osaka.act, Act::Stand { until: shows }, "book {book}");
            assert_eq!(osaka.appearance(until).2, Some(Bubble::Say(MY_BACK)));
            // Credited as the homework it was.
            assert!(
                osaka
                    .credited
                    .iter()
                    .any(|&(w, share, _)| w == Want::Idle(Activity::FloorHomework) && share == 1.0),
                "book {book}: {:?}",
                osaka.credited
            );
            // Again, later in the visit: nothing said.
            let again = shows + 1_000;
            osaka.floor_homework(book, again, &mut rng);
            osaka.credit = Some(Want::Idle(Activity::FloorHomework));
            let Act::Idle { until, .. } = osaka.act else {
                panic!("homework again");
            };
            let decided = osaka.decisions.len();
            osaka.tick(until, None, &terrain, &chances, &mut rng);
            assert!(
                osaka.decisions[decided..]
                    .iter()
                    .all(|d| d.method != "ached"),
                "book {book}: {:?}",
                osaka.decisions[decided..]
                    .iter()
                    .map(|d| d.method)
                    .collect::<Vec<_>>()
            );
            assert_ne!(osaka.speech.map(|(said, _)| said), Some(MY_BACK));
        }
        // Her bedtime cuts it: to bed, her back not aching yet.
        let far = 10 * BED;
        let homework = Act::Idle {
            what: Activity::FloorHomework,
            since: 0,
            until: far,
            play: Some(Play::plain(ScriptId::FloorHomework)),
        };
        let (mut osaka, mut rng) = at_bedtime(homework, far);
        osaka.tick(
            BED,
            Some(monday_at(22, 0)),
            &blank_terrain(),
            &chances,
            &mut rng,
        );
        assert!(
            osaka.decisions.iter().all(|d| d.method != "ached"),
            "{:?}",
            osaka.decisions
        );
        assert!(!osaka.ached);
    }

    /// Standing while a chat watch is live she's drawn side-on watching
    /// it, so she faces it, however she came to stand: home from school
    /// ("I'm home!") just after a line she was out of sight for (the
    /// watch set, nothing turned), or her back aching as her homework on
    /// the floor ends under a watch (lying, she'd never turn to a line;
    /// the watch is set here, as her script dozes her before its end),
    /// each a reflex before the watch's own stand, facing away from the
    /// chat as she was.
    #[test]
    fn standing_under_a_live_watch_she_faces_the_chat() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        for case in ["home", "ached paper", "ached book"] {
            let mut rng = Rng(7);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            let at = match case {
                "home" => {
                    osaka.returning = Some(Routine::School);
                    1_000
                }
                _ => {
                    osaka.floor_homework(case == "ached book", 0, &mut rng);
                    let Act::Idle { until, .. } = osaka.act else {
                        panic!("{case}: homework");
                    };
                    until
                }
            };
            osaka.facing = Facing::Right;
            // The chat on her left, its line seen a moment ago.
            osaka.watch_until = at + 3_000;
            osaka.watch_x = 0;
            osaka.tick(at, None, &terrain, &chances, &mut rng);
            let method = osaka.decisions.last().map(|d| d.method);
            let want = if case == "home" {
                "routine/home"
            } else {
                "ached"
            };
            assert_eq!(method, Some(want), "{case}");
            assert!(matches!(osaka.act, Act::Stand { .. }), "{case}");
            assert_eq!(osaka.appearance(at).0, Pose::Side, "{case}");
            assert_eq!(osaka.facing, Facing::Left, "{case}: facing the chat");
        }
    }

    /// The spot just beside a piece, found from a seat at it, is just
    /// beside the piece itself ([`Shown::beside`]), whichever way it
    /// faces: a seat in it (a bed, a sofa) or beside it, turned to it (a
    /// desk, real or paper), real or made.
    #[test]
    fn beside_a_seat_is_beside_its_piece() {
        use super::super::room::Shown;
        use tuirealm::ratatui::buffer::Buffer;
        use tuirealm::ratatui::layout::Rect;
        use tuirealm::ratatui::style::Style;
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 20));
        buf.set_string(0, 15, "─".repeat(100), Style::default());
        let terrain = Terrain::read(&buf, &[], false);
        for (item, what) in [
            (Furniture::Desk, Use::Homework),
            (Furniture::Bed, Use::Sleep),
            (Furniture::Sofa, Use::Lounge),
        ] {
            for made in [false, true] {
                for facing in [Facing::Right, Facing::Left] {
                    let piece = Shown {
                        item,
                        facing,
                        boxed: false,
                        strip: None,
                        left: 40,
                        floor: 15,
                        scrap: made.then(|| super::super::scrap::Scrap::new(MadeId(0), &[], 0)),
                    };
                    let seat = piece.seat(what, 0);
                    let spot = beside(&seat, &terrain);
                    assert!(
                        spot.is_some_and(|(x, y)| y == 15 && piece.beside().contains(&x)),
                        "{item:?} made={made} {facing:?}: {spot:?}, not one of {:?}",
                        piece.beside()
                    );
                }
            }
        }
    }

    /// Making a piece is [`MAKESHIFT_DRAW`] times as likely while she
    /// owns no real one of its kind (boxed counts as owned), and no
    /// likelier once she does; making a paper desk with no real one, as
    /// much again while her back aches from homework on the floor (and
    /// only a desk; with a real desk, her back draws her to that).
    /// Nothing but making is: every other bind is as likely as ever.
    #[test]
    fn making_draws_her_only_while_she_owns_no_real_piece() {
        use super::super::room::Shown;
        use super::super::scenes::Side;
        let build = |item: Furniture, then: Use| {
            Bind::Job(Job::Build(Build {
                x: 20,
                y: 15,
                row: 13,
                side: Side::Left,
                cells: vec![1, 2, 3, 4, 5],
                glyphs: "hello".into(),
                piece: Shown {
                    item,
                    facing: Facing::Right,
                    boxed: false,
                    strip: None,
                    left: 17,
                    floor: 15,
                    scrap: Some(super::super::scrap::Scrap::new(MadeId(0), &[], 0)),
                },
                then,
            }))
        };
        let owning = |real: &[Furniture]| Chances {
            real: real.to_vec(),
            ..Chances::default()
        };
        let none = owning(&[Furniture::Tv]);
        for (item, then) in [
            (Furniture::Sofa, Use::Lounge),
            (Furniture::Bed, Use::Sleep),
            (Furniture::Desk, Use::Homework),
        ] {
            let bind = build(item, then);
            assert_eq!(making(&bind, &none, false), MAKESHIFT_DRAW, "{item:?}");
            assert_eq!(making(&bind, &owning(&[item]), false), 1.0, "{item:?}");
            let ached = if item == Furniture::Desk {
                MAKESHIFT_DRAW
            } else {
                1.0
            };
            assert_eq!(
                making(&bind, &none, true),
                MAKESHIFT_DRAW * ached,
                "{item:?} aching"
            );
            // Owning a real one, her back is nothing to making another.
            assert_eq!(making(&bind, &owning(&[item]), true), 1.0, "{item:?}");
        }
        for bind in [
            Bind::Here(Here::FloorHomework { book: false }),
            Bind::Here(Here::Idle(Activity::Sit)),
            Bind::Job(Job::Use(paper_desk_seat())),
            Bind::WalkTo(3),
        ] {
            assert_eq!(making(&bind, &none, true), 1.0, "{bind:?}");
        }
        // Above 1 always, so it never pushes the want itself out of her
        // best few.
        const _: () = assert!(MAKESHIFT_DRAW > 1.0);
    }

    /// Her choosing weighs making by what she owns and her back (phase 5c
    /// D5, through her decision's own factors): an offer to make a paper
    /// desk for her homework weighs its want's factor [`MAKESHIFT_DRAW`]
    /// times while no real desk stands, as much again while her back
    /// aches; with a real desk standing (boxed counts), its factor alone,
    /// aching or not.
    #[test]
    fn her_choice_weighs_making_a_desk_by_what_she_owns_and_her_back() {
        use super::super::room::Shown;
        use super::super::scenes::Side;
        let terrain = floor_at(15);
        let want = Want::Use(Use::Homework);
        let desk = Build {
            x: 30,
            y: 15,
            row: 13,
            side: Side::Left,
            cells: vec![1, 2, 3, 4, 5],
            glyphs: "hello".into(),
            piece: Shown {
                item: Furniture::Desk,
                facing: Facing::Right,
                boxed: false,
                strip: None,
                left: 28,
                floor: 15,
                scrap: Some(super::super::scrap::Scrap::new(MadeId(0), &[], 0)),
            },
            then: Use::Homework,
        };
        let base = brain::factor(want, false, None, None);
        for (real, ached, times) in [
            (&[][..], false, MAKESHIFT_DRAW),
            (&[][..], true, MAKESHIFT_DRAW * MAKESHIFT_DRAW),
            (&[Furniture::Desk][..], false, 1.0),
            (&[Furniture::Desk][..], true, 1.0),
        ] {
            let at = format!("real {real:?} ached {ached}");
            let chances = Chances {
                real: real.to_vec(),
                builds: vec![desk.clone()],
                ..Chances::default()
            };
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.ached = ached;
            osaka.offer_only = Some((want, "use/make"));
            osaka.tick(100, None, &terrain, &chances, &mut rng);
            let weighed: Vec<f64> = osaka
                .factored
                .iter()
                .filter(|&&(w, into_chat, _)| w == want && !into_chat)
                .map(|&(_, _, times)| times)
                .collect();
            assert!(!weighed.is_empty(), "{at}: never weighed");
            assert!(
                weighed.iter().all(|&t| t == base * times),
                "{at}: {weighed:?}, not {base} × {times}"
            );
        }
    }

    /// At her paper desk she kneels beside it (her homework's script's
    /// stool poses, resolved for it: writing, nodding, asleep on the
    /// cube), and never splits chopsticks there, rolled however surely or
    /// cued (a cue waits for a real desk); at a real desk, on her stool,
    /// and the chopsticks play as ever.
    #[test]
    fn at_her_paper_desk_she_kneels_and_splits_no_chopsticks() {
        let real = Seat {
            x: 20,
            y: 15,
            ..seat_for(Use::Homework, Furniture::Desk)
        };
        for (seat, made) in [(paper_desk_seat(), true), (real, false)] {
            let at = format!("made {made}");
            let mut rng = Rng(3);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.splices_sure = true;
            osaka.whims = Whims(3);
            osaka.start_job(Job::Use(seat), 0, &Chances::default(), &mut rng);
            let Act::Use {
                since, until, play, ..
            } = osaka.act
            else {
                panic!("{at}: {:?}", osaka.act);
            };
            let chopsticks = play
                .before
                .is_some_and(|s| s.splice == SpliceId::Chopsticks);
            assert_eq!(chopsticks, !made, "{at}: {play:?}");
            let body = play.body_start(since);
            for t in (body..until).step_by(500) {
                let pose = osaka.appearance(t).0;
                if made {
                    assert!(matches!(pose, Pose::PaperDesk(_)), "{at} {t}: {pose:?}");
                } else {
                    assert!(matches!(pose, Pose::Homework(_)), "{at} {t}: {pose:?}");
                }
            }
            // Cued, the chopsticks wait for a use they wrap.
            let mut cued = Osaka::standing_at(20, 15, 0, &mut rng);
            let cue = Cue::Splice(SpliceId::Chopsticks, Some(0));
            cued.cue(Some(cue));
            cued.start_job(Job::Use(seat), 0, &Chances::default(), &mut rng);
            let Act::Use { play, .. } = cued.act else {
                panic!("{at}: {:?}", cued.act);
            };
            let played = play
                .before
                .is_some_and(|s| s.splice == SpliceId::Chopsticks);
            assert_eq!(played, !made, "{at}: cued");
            assert_eq!(cued.cued == Some(cue), made, "{at}: the cue waits");
        }
    }

    /// Her window's look-out seat at `(20, 15)`, facing left (into the
    /// window: she leans on its sill from its right end).
    fn sill_seat() -> Seat {
        Seat {
            x: 20,
            y: 15,
            facing: Facing::Left,
            ..seat_for(Use::LookOut, Furniture::Window)
        }
    }

    /// Her at her sill (see [`sill_seat`]) from 0 to `until`, chosen.
    fn leaning(until: u64) -> Act {
        Act::Use {
            seat: sill_seat(),
            since: 0,
            until,
            whole: until,
            play: Play::of(Use::LookOut, None),
            grievance: None,
        }
    }

    /// Her musings at her sill (phase 5c D6): after the sky's first line
    /// (leaning on the sill, curious), up to three musings on the sky, a
    /// whim's gap apart, each from the sky as it is as she says it
    /// (from her afternoon into dusk, the dusk's), none twice; leaning on
    /// the sill all the while. Drawn from her decision's whims, not her
    /// stream: the same whims say the same however her stream runs. Over
    /// whims, every count from none to three comes (the levers as built,
    /// none to three in every mood: what her mood draws is
    /// `her_mood_says_how_much_she_muses_at_her_sill`'s to say).
    #[test]
    fn at_her_sill_she_muses_on_the_sky() {
        use super::super::art::Sky;
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut counts = std::collections::BTreeSet::new();
        for whims in 0..48u64 {
            let mut said_by_stream = Vec::new();
            for stream in [3u64, 99] {
                let at = format!("whims {whims} stream {stream}");
                let mut rng = Rng(stream);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.stillness = Stillness::NEUTRAL;
                osaka.splice_rows = &[];
                osaka.whims = Whims(whims);
                // 16:58 on a Monday: her dusk comes 20 s in (two game
                // minutes at 6x), by her second musing at the latest.
                let clock = clock_at(0, 16, 58);
                osaka.read_clock(Some(clock), 0);
                osaka.credit = Some(Want::Use(Use::LookOut));
                osaka.start_job(Job::Use(sill_seat()), 0, &chances, &mut rng);
                let Act::Use { until, play, .. } = osaka.act else {
                    panic!("{at}: {:?}", osaka.act);
                };
                assert_eq!(play.own, ScriptId::LookOut, "{at}");
                assert_eq!(
                    osaka.appearance(0).0,
                    Pose::SillLean,
                    "{at}: leaning on the sill"
                );
                let mut now = 0;
                while now + 1 < until {
                    now = osaka.due().min(until - 1);
                    osaka.tick(now, Some(clock), &terrain, &chances, &mut rng);
                    let (pose, ..) = osaka.appearance(now);
                    assert_eq!(pose, Pose::SillLean, "{at} {now}");
                }
                let musings: Vec<(&str, u64)> = osaka
                    .said_lines()
                    .iter()
                    .filter(|(pool, ..)| *pool == mind::PoolId::Sky)
                    .map(|&(_, line, when)| (line, when))
                    .collect();
                assert!(musings.len() <= 3, "{at}: {musings:?}");
                for &(line, when) in &musings {
                    let minute = osaka.day(when).expect("fed").minute;
                    let pool = mind::sky_musings(Sky::at(minute));
                    assert!(pool.lines.contains(&line), "{at}: {line} at {minute}");
                    assert!(
                        when >= script::LOOK_OUT_LINE_MS,
                        "{at}: after the sky's line"
                    );
                }
                let distinct: std::collections::BTreeSet<&str> =
                    musings.iter().map(|&(l, _)| l).collect();
                assert_eq!(distinct.len(), musings.len(), "{at}: twice");
                for pair in musings.windows(2) {
                    assert!(pair[1].1 - pair[0].1 >= 12_000, "{at}: {musings:?}");
                }
                counts.insert(musings.len());
                said_by_stream.push(musings);
            }
            assert_eq!(
                said_by_stream[0], said_by_stream[1],
                "whims {whims}: by her stream"
            );
        }
        assert_eq!(counts.into_iter().collect::<Vec<_>>(), [0, 1, 2, 3]);
    }

    /// A line in the chat while she muses at her sill: she looks up where
    /// she leans (no cut), and her next musing waits until her look is
    /// over.
    #[test]
    fn a_chat_line_holds_her_next_musing_at_the_sill() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut checked = 0;
        for whims in 0..32u64 {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.splice_rows = &[];
            osaka.whims = Whims(whims);
            osaka.credit = Some(Want::Use(Use::LookOut));
            osaka.start_job(Job::Use(sill_seat()), 0, &chances, &mut rng);
            // When she'd muse first, undisturbed.
            let mut quiet = osaka.clone();
            let first = loop {
                let now = quiet.due();
                if now >= 60_000 {
                    break None;
                }
                quiet.tick(now, None, &terrain, &chances, &mut rng);
                if let Some(&(_, _, when)) = quiet
                    .said_lines()
                    .iter()
                    .find(|(pool, ..)| *pool == mind::PoolId::Sky)
                {
                    break Some(when);
                }
            };
            let Some(first) = first else { continue };
            checked += 1;
            // A line 1 s before it: she looks up in place.
            let line = first - 1000;
            osaka.tick(line - 1, None, &terrain, &chances, &mut rng);
            osaka.look(line, 0, false, &terrain);
            assert!(matches!(osaka.act, Act::Use { seat, .. } if seat == sill_seat()));
            let look_over = line + LOOK_MS;
            let mut now = line;
            let mut mused = None;
            while now < look_over + 30_000 {
                now = osaka.due();
                osaka.tick(now, None, &terrain, &chances, &mut rng);
                if let Some(&(_, _, when)) = osaka
                    .said_lines()
                    .iter()
                    .find(|(pool, ..)| *pool == mind::PoolId::Sky)
                {
                    assert!(
                        when >= look_over,
                        "whims {whims}: mused at {when} under her look"
                    );
                    mused = Some(when);
                    break;
                }
            }
            // Held, not dropped: it comes once her look is over, a gap
            // after at most, while she still leans there.
            let when = mused.unwrap_or_else(|| panic!("whims {whims}: her musing was dropped"));
            assert!(
                when < look_over + Stillness::NEUTRAL.musing_gap.1,
                "whims {whims}: mused at {when}, her look over at {look_over}"
            );
            assert!(matches!(osaka.act, Act::Use { seat, .. } if seat == sill_seat()));
        }
        assert!(checked >= 8, "only {checked} mused");
    }

    /// A line still showing as her look up ends, due to end less than a
    /// frame after it, shows on until a frame after it: the look's end and
    /// the line's are never within a frame (the door batch's step 2
    /// review, "home, shopping Dreamy seed 2": what her look hid, her
    /// pitch, said at 94000 for 2460 ms, her look over at 95400, the line
    /// gone at 96460). A line ending with the look, or a frame or more
    /// after it, keeps its end.
    #[test]
    fn a_line_never_ends_within_a_frame_of_her_look_ending() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        for after in [0, 1, 1060, USE_FRAME_MS - 1, USE_FRAME_MS, 3000] {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.splice_rows = &[];
            osaka.credit = Some(Want::Use(Use::LookOut));
            osaka.start_job(Job::Use(sill_seat()), 0, &chances, &mut rng);
            osaka.tick(9_999, None, &terrain, &chances, &mut rng);
            osaka.look(10_000, 0, false, &terrain);
            let end = osaka.looking_up.expect("she looks up").until;
            let text = "A cat bed! For a cat!";
            osaka.speech = Some((text, end + after));
            while osaka.looking_up.is_some() {
                let now = osaka.due();
                assert!(now <= end, "after {after}: past her look's end");
                osaka.tick(now, None, &terrain, &chances, &mut rng);
            }
            assert_eq!(osaka.look_ended, Some(end), "after {after}");
            let want = if after == 0 {
                None
            } else if after < USE_FRAME_MS {
                Some((text, end + USE_FRAME_MS))
            } else {
                Some((text, end + after))
            };
            assert_eq!(osaka.speech, want, "after {after}");
        }
    }

    /// A musing due in the first frame of the clock, before she has ever
    /// looked up, isn't held: no look has ended to settle from.
    #[test]
    fn a_musing_before_she_ever_looked_up_waits_for_nothing() {
        let mut rng = Rng(5);
        let osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        assert_eq!(osaka.look_ended, None);
        for at in [1, 500, USE_FRAME_MS - 1] {
            assert_eq!(osaka.free_to_muse(at), 0, "due at {at}");
        }
    }

    /// Her next musing at her sill, due less than a frame after her look
    /// up at the chat is over, waits until a frame after it (phase 5c's
    /// tail, T4's sibling, found when the door batch's step 2 moved her
    /// pieces: "home, clock and window, to dusk Dreamy seed 2, an act from
    /// 503085 flipped at 545400 and 546718", her watch's end and a
    /// musing): a change of hers waits a frame from a look's. Never
    /// dropped: it comes then, while she still leans there.
    #[test]
    fn her_musing_at_the_sill_waits_a_frame_from_her_look_ending() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let mut checked = 0;
        for whims in 0..32u64 {
            for after in [1, 500, USE_FRAME_MS - 1] {
                let at = format!("whims {whims}, due {after} ms after her look");
                let mut rng = Rng(5);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.splice_rows = &[];
                osaka.whims = Whims(whims);
                osaka.credit = Some(Want::Use(Use::LookOut));
                osaka.start_job(Job::Use(sill_seat()), 0, &chances, &mut rng);
                // When she'd muse first, undisturbed.
                let mut quiet = osaka.clone();
                let first = loop {
                    let now = quiet.due();
                    if now >= 60_000 {
                        break None;
                    }
                    quiet.tick(now, None, &terrain, &chances, &mut rng);
                    if let Some(&(_, _, when)) = quiet
                        .said_lines()
                        .iter()
                        .find(|(pool, ..)| *pool == mind::PoolId::Sky)
                    {
                        break Some(when);
                    }
                };
                let Some(first) = first else { continue };
                // A line whose look is over `after` ms before it.
                let Some(line) = first.checked_sub(LOOK_UP_MS + after) else {
                    continue;
                };
                if line <= script::LOOK_OUT_LINE_MS {
                    continue;
                }
                osaka.tick(line - 1, None, &terrain, &chances, &mut rng);
                osaka.look(line, 0, false, &terrain);
                let mut now = line;
                let mut mused = None;
                while now < first + 30_000 {
                    now = osaka.due();
                    osaka.tick(now, None, &terrain, &chances, &mut rng);
                    if let Some(&(_, _, when)) = osaka
                        .said_lines()
                        .iter()
                        .find(|(pool, ..)| *pool == mind::PoolId::Sky)
                    {
                        mused = Some(when);
                        break;
                    }
                }
                let when = mused.unwrap_or_else(|| panic!("{at}: her musing was dropped"));
                let ended = osaka.look_ended.expect("she looked up");
                assert!(ended > line, "{at}: her look is over ({ended})");
                assert!(
                    when == ended || when >= ended + USE_FRAME_MS,
                    "{at}: mused at {when}, her look over at {ended}"
                );
                assert!(matches!(osaka.act, Act::Use { seat, .. } if seat == sill_seat()));
                checked += 1;
            }
        }
        assert!(checked >= 12, "only {checked} mused");
    }

    /// From her sill she settles in where she is (phase 5c D6), as often
    /// as her mood's odds (here always): leaning on it, to sitting in
    /// front of it; from there to dozing where she sits, or (on the
    /// doze's whim's other side, her window still there) lying back
    /// under it watching the clouds, turned so her head's under the
    /// glass. No walk, no set-off, no choice. With her window gone (no
    /// look-out seat where she sits), never the clouds: she dozes.
    #[test]
    fn from_her_sill_she_settles_in_where_she_is() {
        let terrain = floor_at(15);
        let windowed = Chances {
            seats: vec![sill_seat()],
            ..Chances::default()
        };
        let bare = Chances::default();
        let under = Want::Idle(Activity::UnderSill);
        let cases: Vec<(&str, Act, Want, f64, &Chances, Want, Facing)> = vec![
            (
                "the sill",
                leaning(10_000),
                Want::Use(Use::LookOut),
                0.0,
                &windowed,
                under,
                Facing::Left,
            ),
            (
                "in front of it, dozing",
                idle_until(Activity::UnderSill, 10_000),
                under,
                1.0,
                &windowed,
                Want::Idle(Activity::SitDoze),
                Facing::Left,
            ),
            (
                "in front of it, the clouds",
                idle_until(Activity::UnderSill, 10_000),
                under,
                0.0,
                &windowed,
                Want::Idle(Activity::CloudWatch),
                Facing::Right,
            ),
            (
                "its window gone",
                idle_until(Activity::UnderSill, 10_000),
                under,
                0.0,
                &bare,
                Want::Idle(Activity::SitDoze),
                Facing::Left,
            ),
        ];
        for mood in Mood::ALL {
            for (name, act, want, sit_doze, chances, into, facing) in &cases {
                let at = format!("{name} {mood:?}");
                let mut rng = Rng(5);
                let mut osaka =
                    at_still(levers(1.0, *sit_doze), mood, act.clone(), *want, &mut rng);
                let set_offs = osaka.set_offs;
                osaka.tick(10_000, None, &terrain, chances, &mut rng);
                let decision = osaka.decisions.last().expect("decided");
                assert_eq!(
                    (decision.bucket, decision.method),
                    (Bucket::Continuation, "settle in"),
                    "{at}"
                );
                let (now, _) =
                    settled_at(&osaka).unwrap_or_else(|| panic!("{at}: {:?}", osaka.act));
                assert_eq!(now, *into, "{at}");
                assert_eq!((osaka.x, osaka.y), (20, 15), "{at}: where she was");
                assert_eq!(osaka.facing, *facing, "{at}");
                assert_eq!(osaka.set_offs, set_offs, "{at}: no set-off");
                assert!(osaka.choices.is_empty(), "{at}: no choice");
                assert_eq!(osaka.credit, Some(*into), "{at}");
            }
        }
    }

    /// In front of her window (phase 5c D6) a chat line from the far side
    /// doesn't turn her (her face is up at its sky, her chin in her
    /// hands), and lying back from there to watch the clouds she lies
    /// with her head under the glass however she was turned: turned end
    /// to end from facing it as its look-out seat has her.
    #[test]
    fn in_front_of_her_window_she_keeps_to_it_and_lies_back_under_it() {
        let terrain = floor_at(15);
        let windowed = Chances {
            seats: vec![sill_seat()],
            ..Chances::default()
        };
        let mut rng = Rng(5);
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Dreamy,
            idle_until(Activity::UnderSill, 10_000),
            Want::Idle(Activity::UnderSill),
            &mut rng,
        );
        assert_eq!(osaka.facing, sill_seat().facing);
        // A line from the right, the far side from her window.
        osaka.look(2_000, 100, false, &terrain);
        assert_eq!(osaka.facing, sill_seat().facing, "she keeps to it");
        // Turned all the same (as an older visit might have left her).
        for turned in [Facing::Left, Facing::Right] {
            let mut lying = osaka.clone();
            lying.facing = turned;
            lying.tick(10_000, None, &terrain, &windowed, &mut rng);
            assert!(
                matches!(
                    lying.act,
                    Act::Idle {
                        what: Activity::CloudWatch,
                        ..
                    }
                ),
                "{turned:?}: {:?}",
                lying.act
            );
            assert_eq!(
                lying.facing,
                Facing::Right,
                "{turned:?}: head under the glass"
            );
        }
    }

    /// Her sill's session ends with her look-out (phase 5c D6): cut short
    /// by another use before her first musing, she says nothing of the
    /// sky there, and nothing calls her sooner for it.
    #[test]
    fn her_sill_session_ends_with_her_look_out() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let lounge = Seat {
            x: 20,
            y: 15,
            ..seat_for(Use::Lounge, Furniture::Sofa)
        };
        let mut checked = 0;
        for whims in 0..32u64 {
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            osaka.splice_rows = &[];
            osaka.whims = Whims(whims);
            osaka.credit = Some(Want::Use(Use::LookOut));
            osaka.start_job(Job::Use(sill_seat()), 0, &chances, &mut rng);
            let Some(session) = osaka.sill else { continue };
            checked += 1;
            let cut = 5_000;
            assert!(session.next > cut, "whims {whims}: {session:?}");
            osaka.credit = Some(Want::Use(Use::Lounge));
            osaka.start_job(Job::Use(lounge), cut, &chances, &mut rng);
            assert!(osaka.sill.is_none(), "whims {whims}: left behind");
            let Act::Use { until, .. } = osaka.act else {
                panic!("whims {whims}: {:?}", osaka.act);
            };
            let mut now = cut;
            while now + 1 < until.min(cut + 60_000) {
                now = osaka.due().min(until - 1);
                osaka.tick(now, None, &terrain, &chances, &mut rng);
            }
            assert!(
                !osaka
                    .said_lines()
                    .iter()
                    .any(|(pool, ..)| *pool == mind::PoolId::Sky),
                "whims {whims}: the sky from her sofa"
            );
        }
        assert!(checked >= 8, "only {checked} with musings to come");
    }

    /// How many musings she has at her sill is her mood's (phase 5c D6,
    /// [`Stillness::sill`]): exactly as many as a table that fixes them
    /// says, three at most.
    #[test]
    fn her_mood_says_how_much_she_muses_at_her_sill() {
        let terrain = floor_at(15);
        let chances = Chances::default();
        let table = Stillness {
            sill: ByMood {
                ordinary: (0, 0),
                lazy: (1, 1),
                industrious: (2, 2),
                dreamy: (3, 9),
            },
            ..Stillness::NEUTRAL
        };
        for mood in Mood::ALL {
            let want = match mood {
                Mood::Ordinary => 0,
                Mood::Lazy => 1,
                Mood::Industrious => 2,
                Mood::Dreamy => 3,
            };
            for whims in 0..8u64 {
                let at = format!("{mood:?} whims {whims}");
                let mut rng = Rng(5);
                let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
                osaka.set_mood(mood);
                osaka.stillness = table;
                osaka.splice_rows = &[];
                osaka.whims = Whims(whims);
                osaka.credit = Some(Want::Use(Use::LookOut));
                osaka.start_job(Job::Use(sill_seat()), 0, &chances, &mut rng);
                assert_eq!(osaka.sill.map_or(0, |s| s.left), want, "{at}");
                let Act::Use { until, .. } = osaka.act else {
                    panic!("{at}: {:?}", osaka.act);
                };
                let mut now = 0;
                while now + 1 < until {
                    now = osaka.due().min(until - 1);
                    osaka.tick(now, None, &terrain, &chances, &mut rng);
                }
                let said = osaka
                    .said_lines()
                    .iter()
                    .filter(|(pool, ..)| *pool == mind::PoolId::Sky)
                    .count();
                assert!(said <= usize::from(want), "{at}: {said}");
            }
        }
    }

    /// Watching the clouds (phase 5c D6) is lying back with her eyes
    /// open, held: curious, no Zzz, one line at most (the sky's, as she
    /// lies back), never a doze, so a chat line has her look up where she
    /// lies rather than stir. Settled into only under her window: from a
    /// sit on a bare floor she lies back to doze, never to the clouds.
    #[test]
    fn watching_the_clouds_is_eyes_open_and_only_under_her_window() {
        let terrain = floor_at(15);
        let windowed = Chances {
            seats: vec![sill_seat()],
            ..Chances::default()
        };
        // Under her window, from sitting in front of it.
        let mut rng = Rng(5);
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Dreamy,
            idle_until(Activity::UnderSill, 10_000),
            Want::Idle(Activity::UnderSill),
            &mut rng,
        );
        osaka.tick(10_000, None, &terrain, &windowed, &mut rng);
        let Act::Idle {
            what: Activity::CloudWatch,
            until,
            ..
        } = osaka.act
        else {
            panic!("{:?}", osaka.act);
        };
        let mut lines = 0;
        let mut was = None;
        for t in (10_000..until).step_by(250) {
            osaka.tick(t, None, &terrain, &windowed, &mut rng);
            let (pose, face, bubble) = osaka.appearance(t);
            assert_eq!(pose, Pose::CloudWatch, "{t}");
            assert!(!pose.dozes(), "{t}");
            // Her eyes open throughout: shut only for a slow blink's
            // moment, as on any pose she holds.
            assert_eq!(osaka.look_at(t).1, Face::Curious, "{t}");
            assert!(
                face == Face::Curious || (face == Face::Blink && osaka.held_blink(t)),
                "{t}: {face:?}"
            );
            assert_ne!(bubble, Some(Bubble::Zzz), "{t}");
            if bubble.is_some() && was.is_none() {
                lines += 1;
            }
            was = bubble;
        }
        assert!(lines <= 1, "{lines} lines");
        // A chat line: she looks up where she lies.
        let mut rng = Rng(5);
        let mut osaka = at_still(
            levers(1.0, 0.0),
            Mood::Dreamy,
            idle_until(Activity::UnderSill, 10_000),
            Want::Idle(Activity::UnderSill),
            &mut rng,
        );
        osaka.tick(10_000, None, &terrain, &windowed, &mut rng);
        let lying = osaka.act.clone();
        osaka.look(15_000, 0, false, &terrain);
        assert_eq!(osaka.act, lying, "not cut");
        assert_eq!(
            osaka.appearance(15_000),
            (Pose::CloudWatch, Face::Surprised, Some(Bubble::Bang)),
            "she looks up"
        );
        // On a bare floor, from a sit: lying back is the doze.
        for whims in 0..32u64 {
            let mut rng = Rng(5);
            let mut osaka = at_still(
                levers(1.0, 0.0),
                Mood::Dreamy,
                idle_until(Activity::Sit, 10_000),
                Want::Idle(Activity::Sit),
                &mut rng,
            );
            osaka.whims = Whims(whims);
            osaka.tick(10_000, None, &terrain, &Chances::default(), &mut rng);
            assert!(
                matches!(
                    osaka.act,
                    Act::Idle {
                        what: Activity::LieBack,
                        ..
                    }
                ),
                "whims {whims}: {:?}",
                osaka.act
            );
            assert!(
                matches!(osaka.appearance(10_000).0, Pose::LieBack(_)),
                "whims {whims}: the doze"
            );
        }
    }

    /// Her InChat grievance over a plant, felt on sight and owed (door
    /// batch, D7), as a decision would see it: what's broken, the line,
    /// and when she's quiet (Industrious, past her mood's first line,
    /// said on her first decision).
    fn owing_aloud(
        osaka: &mut Osaka,
        terrain: &Terrain,
        rng: &mut Rng,
    ) -> (Chances, &'static str, u64) {
        let row = super::super::rules::RULES
            .iter()
            .position(|r| r.rule == super::super::rules::Rule::InChat)
            .unwrap();
        let key = Grievance {
            row,
            piece: Furniture::Plant,
        };
        let chances = Chances {
            broken: vec![super::super::rules::Broken {
                row,
                pieces: vec![Furniture::Plant],
                involved: vec![Furniture::Plant],
                key,
            }],
            ..Chances::default()
        };
        osaka.set_mood(Mood::Industrious);
        osaka.decide(0, terrain, &Chances::default(), rng);
        let quiet = osaka
            .speech
            .map_or(0, |(_, until)| until)
            .max(osaka.morning_until);
        assert!(osaka.said_aloud().is_empty());
        assert!(osaka.feel(key, None));
        osaka.owe_aloud(row);
        (chances, super::super::rules::RULES[row].grievance, quiet)
    }

    /// What she felt on sight waits for her to be quiet (door batch, C4):
    /// her "I'm home!" or good morning still showing (`morning_until`), or
    /// a line of hers (`speech`), she stands until it's over, saying
    /// nothing, then says it whole. A guard of the door batch's step 6
    /// review: said at once, it went over her hello (mutant C).
    #[test]
    fn a_line_felt_on_sight_waits_for_her_to_be_quiet() {
        let terrain = floor_at(15);
        for hello in [true, false] {
            let at = format!("hello {hello}");
            let mut rng = Rng(5);
            let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
            let (chances, line, quiet) = owing_aloud(&mut osaka, &terrain, &mut rng);
            let (now, until) = (quiet + 1000, quiet + 3000);
            if hello {
                osaka.morning_until = until;
            } else {
                osaka.speech = Some(("Hm.", until));
            }
            // Precondition: not quiet at the decision.
            let quiet = osaka
                .speech
                .map_or(0, |(_, until)| until)
                .max(osaka.morning_until);
            assert!(quiet > now, "{at}");
            osaka.decide(now, &terrain, &chances, &mut rng);
            assert!(
                matches!(osaka.act, Act::Stand { until: u } if u == until),
                "{at}: {:?}",
                osaka.act
            );
            assert!(osaka.said_aloud().is_empty(), "{at}");
            osaka.decide(until, &terrain, &chances, &mut rng);
            assert_eq!(osaka.said_aloud(), [(line, until)], "{at}");
            for now in (until..until + GRIEVANCE_MS).step_by(100) {
                assert_eq!(osaka.look_at(now).2, Some(Bubble::Say(line)), "{at} {now}");
            }
        }
    }

    /// While she says what she felt on sight, nothing speaks over it: a
    /// parcel at the door waits for its end, as for her hello (`awake`).
    /// Red under the door batch's step 6 review's mutant D (the line not
    /// keeping her from saying "A parcel!").
    #[test]
    fn a_parcel_waits_for_her_line_felt_on_sight() {
        let terrain = floor_at(15);
        let mut rng = Rng(5);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        let (chances, line, quiet) = owing_aloud(&mut osaka, &terrain, &mut rng);
        osaka.decide(quiet, &terrain, &chances, &mut rng);
        assert_eq!(osaka.said_aloud(), [(line, quiet)]);
        assert!(!osaka.awake(quiet + 100), "a parcel waits");
        assert!(!osaka.awake(quiet + GRIEVANCE_MS - 1), "a parcel waits");
        assert!(
            osaka.awake(quiet + GRIEVANCE_MS),
            "then she's free to say so"
        );
    }

    /// A chat line as she says what she felt on sight: her look up waits
    /// for her to have said it (`grumbling` covers the line, standing),
    /// so it shows whole, and then she looks up, startled. Red under the
    /// door batch's step 6 review's mutant K (the look begun at once:
    /// past its startle by the line's end).
    #[test]
    fn a_chat_line_waits_for_her_line_felt_on_sight() {
        let terrain = floor_at(15);
        let mut rng = Rng(5);
        let mut osaka = Osaka::standing_at(20, 15, 0, &mut rng);
        let (chances, line, quiet) = owing_aloud(&mut osaka, &terrain, &mut rng);
        osaka.decide(quiet, &terrain, &chances, &mut rng);
        assert_eq!(osaka.said_aloud(), [(line, quiet)]);
        osaka.look_up(quiet + 100, 0);
        for now in (quiet..quiet + GRIEVANCE_MS).step_by(100) {
            assert_eq!(osaka.look_at(now).2, Some(Bubble::Say(line)), "{now}");
        }
        let end = quiet + GRIEVANCE_MS;
        assert_eq!(osaka.looking_up.map(|l| l.since), Some(end));
        assert_eq!(
            osaka.look_at(end).2,
            Some(Bubble::Bang),
            "startled once it's said"
        );
    }
}
