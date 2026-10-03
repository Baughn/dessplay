//! How she sets about what she wants. Each want has named methods with
//! pure guards, tried in order; the first that binds says what her body
//! does. A want is offered only when one of its methods binds, so
//! offering and planning can't disagree. Her mind has its own random
//! stream, one draw a decision, hashed with a label for each choice made
//! from it ([`Whims`]): guards stay pure, every decision draws the same,
//! and the body's stream (durations, how she looks) is its own.

use super::brain::{Spot, Want};
use super::osaka::{Activity, CHAT_FACTOR, Chances, Episode, landing, middle, pick};
use super::room::{Furniture, PieceRef, Seat, Use};
use super::scenes::{Build, Job, Lift, SetDown};
use super::terrain::{Link, Route, Terrain};

/// Her mind's stream is seeded from the body's first draw, salted.
pub(super) const MIND_SALT: u64 = 0x6d69_6e64_5f6f_6661;

/// A makeshift piece, when there's a real one she could use instead: one
/// time in this many.
const MAKESHIFT_ODDS: u64 = 20;

/// How much likelier a makeshift sofa is made where she could also
/// watch the TV from it.
const FACING_TV: f64 = 5.0;

/// The decision's one draw from her mind's stream. Each choice made from
/// it hashes it with its own label (and a salt, for one of many), so no
/// two choices share a number.
#[derive(Clone, Copy, Debug)]
pub(super) struct Whims(pub u64);

impl Whims {
    fn roll(self, label: &str, salt: u64) -> u64 {
        // FNV-1a over the label, then splitmix: stable across Rust
        // versions (the golden trajectories depend on it).
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in label.bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut z = self.0 ^ h ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (0 when `n` is 0).
    pub fn below(self, label: &str, n: u64) -> u64 {
        self.below_at(label, 0, n)
    }

    /// [`Whims::below`], for the `salt`th of several such choices.
    pub fn below_at(self, label: &str, salt: u64, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.roll(label, salt) % n
        }
    }

    /// `n` times in `d`.
    pub fn chance(self, label: &str, salt: u64, n: u64, d: u64) -> bool {
        self.below_at(label, salt, d) < n
    }
}

/// What a decision sees: where she is, and what the frame offers.
pub(super) struct Ctx<'a> {
    pub x: i32,
    pub y: i32,
    /// The floor she's on.
    pub here: usize,
    /// The ways off it.
    pub links: Vec<Link>,
    pub terrain: &'a Terrain,
    pub chances: &'a Chances,
    /// She may go to work now (a home to leave, a while in, once).
    pub may_work: bool,
    /// Mischief she still owes an undo for (one at a time).
    pub owes: bool,
    /// The piece of her home she's moving, if she is.
    pub episode: Option<Episode>,
    /// The piece she just set down where it's right, and the use she
    /// felt was wrong on it (she sits back down to it).
    pub just_set: Option<(Furniture, Use)>,
    /// She'd put a rule of her home right: one she felt is broken
    /// still, and her mood leaves her something to do about her home.
    pub may_arrange: bool,
}

/// What she does on the spot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Here {
    Stand,
    SpaceOut,
    /// Space out, saying one of her musings first.
    Muse,
    Sneeze,
    Idle(Activity),
}

/// What a method binds: what her body does next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Bind {
    Here(Here),
    /// Walk to this column of her floor.
    WalkTo(i32),
    /// Take this link off her floor.
    Take(Link),
    /// A door in space to this spot.
    Door((i32, i32)),
    /// Off to work, out by this link if her floor reaches a screen edge.
    Work(Option<Link>),
    /// A job: on her floor she walks to it, elsewhere she heads for it.
    Job(Job),
}

impl Bind {
    /// Where it takes her, if anywhere in particular.
    fn spot(&self, terrain: &Terrain) -> Option<(i32, i32)> {
        match self {
            Self::Take(link) => landing(link, terrain),
            Self::Door(spot) => Some(*spot),
            Self::Job(job) => Some(job.spot()),
            Self::Here(_) | Self::WalkTo(_) | Self::Work(_) => None,
        }
    }

    /// Where it would have her, as far as her needs care.
    pub fn on(&self) -> Spot {
        match self {
            Self::Job(Job::Use(seat)) if seat.makeshift() => Spot::Made,
            Self::Job(Job::Use(seat)) => Spot::Real(seat.item),
            Self::Job(Job::Build(_)) => Spot::Made,
            Self::Here(Here::Idle(_)) => Spot::Floor,
            _ => Spot::Any,
        }
    }

    /// The want's chat factor applies: it would take her into the chat.
    pub fn in_chat(&self, ctx: &Ctx) -> bool {
        self.spot(ctx.terrain)
            .is_some_and(|spot| ctx.chances.in_chat(spot))
    }
}

/// A way to set about a want: its name (for the explain log) and its
/// guard, which binds or doesn't, from what the decision sees alone.
pub(super) struct Method {
    pub name: &'static str,
    guard: fn(&Ctx, Whims, Want) -> Option<Bind>,
}

const fn m(name: &'static str, guard: fn(&Ctx, Whims, Want) -> Option<Bind>) -> Method {
    Method { name, guard }
}

const STAND: &[Method] = &[m("stand", |_, _, _| Some(Bind::Here(Here::Stand)))];
const SPACE_OUT: &[Method] = &[m("space-out/muse", muse), m("space-out", space_out)];
const SNEEZE: &[Method] = &[m("sneeze", |_, _, _| Some(Bind::Here(Here::Sneeze)))];
const IDLE: &[Method] = &[m("idle", idle)];
const WALK: &[Method] = &[m("walk/along", walk)];
const TRAVEL: &[Method] = &[m("travel/link", take_link), m("travel/door", door_away)];
const WORK: &[Method] = &[m("work", work)];
const PULL: &[Method] = &[m("pull", pull)];
const SWAP: &[Method] = &[m("swap", swap)];
/// The most progressed step first: what she's carrying comes before
/// lifting anything.
const ARRANGE: &[Method] = &[
    m("arrange/use-it", use_it),
    m("arrange/carry", carry),
    m("arrange/lift", lift),
];
const USE: &[Method] = &[
    m("use/finish-my-heap", finish_my_heap),
    m("use/mine", use_mine),
    m("use/real", use_real),
    m("use/made", use_made),
    m("use/make", use_make),
];

/// Each want's methods, in the order she tries them.
pub(super) fn methods(want: Want) -> &'static [Method] {
    match want {
        Want::Stand => STAND,
        Want::SpaceOut => SPACE_OUT,
        Want::Sneeze => SNEEZE,
        Want::Idle(_) => IDLE,
        Want::Walk => WALK,
        Want::Travel => TRAVEL,
        Want::Work => WORK,
        Want::Pull => PULL,
        Want::Swap => SWAP,
        // A heap is crumpled as a step of what she made it for.
        Want::Use(Use::Crumple) => &[],
        Want::Use(_) => USE,
        Want::Arrange => ARRANGE,
    }
}

/// Space out, saying one of her musings first: one time in three.
fn muse(_: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    w.chance("muse", 0, 1, 3).then_some(Bind::Here(Here::Muse))
}

fn space_out(_: &Ctx, _: Whims, _: Want) -> Option<Bind> {
    Some(Bind::Here(Here::SpaceOut))
}

fn idle(_: &Ctx, _: Whims, want: Want) -> Option<Bind> {
    match want {
        Want::Idle(what) => Some(Bind::Here(Here::Idle(what))),
        _ => None,
    }
}

/// The first of `want`'s methods that binds, by name, with what it
/// binds. A job must stand on a floor (or there's no way to it).
pub(super) fn bind(ctx: &Ctx, whims: Whims, want: Want) -> Option<(&'static str, Bind)> {
    methods(want).iter().find_map(|method| {
        let bound = (method.guard)(ctx, whims, want)?;
        if let Bind::Job(job) = &bound {
            let (x, y) = job.spot();
            ctx.terrain.platform_at(x, y)?;
        }
        Some((method.name, bound))
    })
}

/// Somewhere along her floor: the far end of it a tenth as likely where
/// it runs into the chat (from outside it).
fn walk(c: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    let p = c.terrain.platforms.get(c.here)?;
    let from_chat = c.chances.in_chat((c.x, c.y));
    let into_chat = |x: i32| !from_chat && c.chances.in_chat((x, p.y));
    let i = pick(
        (p.x1 - p.x0 + 1) as usize,
        |i| into_chat(p.x0 + i as i32),
        |n| w.below("walk", n),
    )?;
    let to = p.x0 + i as i32;
    (to != c.x).then_some(Bind::WalkTo(to))
}

/// One of the ways off her floor, those into the chat a tenth as likely.
fn take_link(c: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    let into_chat = |l: &Link| landing(l, c.terrain).is_some_and(|s| c.chances.in_chat(s));
    let i = pick(
        c.links.len(),
        |i| c.links.get(i).is_some_and(into_chat),
        |n| w.below("link", n),
    )?;
    c.links.get(i).copied().map(Bind::Take)
}

/// With no way off her floor, a door in space to anywhere else.
fn door_away(c: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    if !c.links.is_empty() {
        return None;
    }
    let others: Vec<_> = c
        .terrain
        .platforms
        .iter()
        .enumerate()
        .filter(|&(i, _)| i != c.here)
        .map(|(_, p)| p)
        .collect();
    let i = pick(
        others.len(),
        |i| others.get(i).is_some_and(|p| c.chances.in_chat(middle(p))),
        |n| w.below("door", n),
    )?;
    let p = others.get(i)?;
    let x = p.x0 + w.below("door-x", (p.x1 - p.x0 + 1) as u64) as i32;
    Some(Bind::Door((x, p.y)))
}

/// Off to work, home by a way that doesn't come in through the chat when
/// there is one.
fn work(c: &Ctx, _: Whims, _: Want) -> Option<Bind> {
    if !c.may_work {
        return None;
    }
    let around = || {
        c.links
            .iter()
            .filter(|l| matches!(l.route, Route::Around { .. }))
    };
    let out = around()
        .find(|l| !landing(l, c.terrain).is_some_and(|s| c.chances.in_chat(s)))
        .or_else(|| around().next())
        .copied();
    Some(Bind::Work(out))
}

fn pull(c: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    let pulls = &c.chances.pulls;
    let i = pick(
        pulls.len(),
        |i| pulls.get(i).is_some_and(|p| c.chances.in_chat((p.x, p.y))),
        |n| w.below("pull", n),
    )?;
    pulls.get(i).cloned().map(|p| Bind::Job(Job::Pull(p)))
}

/// One piece of mischief at a time: no new swap while one is owed.
fn swap(c: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    if c.owes {
        return None;
    }
    let swaps = &c.chances.swaps;
    let i = pick(
        swaps.len(),
        |i| swaps.get(i).is_some_and(|s| c.chances.in_chat((s.x, s.y))),
        |n| w.below("swap", n),
    )?;
    swaps.get(i).cloned().map(|s| Bind::Job(Job::Swap(s)))
}

/// A heap she made for this, not yet crumpled into shape (the step
/// before using it).
fn finish_my_heap(c: &Ctx, _: Whims, want: Want) -> Option<Bind> {
    let Want::Use(what) = want else {
        return None;
    };
    c.chances
        .mine
        .iter()
        .filter(|m| m.purpose == what && !m.done)
        .find_map(|m| c.chances.next_for(m))
        .map(|seat| Bind::Job(Job::Use(seat)))
}

/// A piece she made for this and hasn't used yet.
fn use_mine(c: &Ctx, _: Whims, want: Want) -> Option<Bind> {
    let Want::Use(what) = want else {
        return None;
    };
    c.chances
        .mine
        .iter()
        .filter(|m| m.purpose == what && m.done && !m.used)
        .find_map(|m| c.chances.next_for(m))
        .map(|seat| Bind::Job(Job::Use(seat)))
}

/// Somewhere she could go to use something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Place {
    Seat(Seat),
    /// Make a makeshift piece like this first.
    Make(Furniture),
}

/// Where she might go to `what`: every seat for it, except that a
/// makeshift piece is only for when there's no real one of its kind on
/// offer — else just one time in [`MAKESHIFT_ODDS`], and then it's the
/// makeshift one she goes for. A piece she'd have to make is one place,
/// and only when she hasn't made one of its kind already.
pub(super) fn places(what: Use, chances: &Chances, w: Whims) -> Vec<Place> {
    let seats = || chances.seats.iter().filter(|s| s.what == what);
    let mut makeshift: Vec<Furniture> = seats()
        .filter(|s| s.makeshift())
        .map(|s| s.item)
        .chain(
            chances
                .builds
                .iter()
                .filter(|b| b.then == what)
                .map(|b| b.piece.item),
        )
        .collect();
    makeshift.sort_by_key(|&item| item as u8);
    makeshift.dedup();
    // For each kind with a makeshift option, whether she goes for it:
    // always without a real one, on a whim with.
    let mut whims = Vec::new();
    let mut allowed = Vec::new();
    for item in makeshift {
        let real = seats().any(|s| !s.makeshift() && s.item == item);
        if !real {
            allowed.push(item);
        } else if w.chance("makeshift", item as u64, 1, MAKESHIFT_ODDS) {
            allowed.push(item);
            whims.push(item);
        }
    }
    let mut out: Vec<Place> = seats()
        .filter(|s| {
            if s.makeshift() {
                allowed.contains(&s.item)
            } else {
                !whims.contains(&s.item)
            }
        })
        .map(|&s| Place::Seat(s))
        .collect();
    for item in allowed {
        let made = seats().any(|s| s.makeshift() && s.item == item);
        let can = chances
            .builds
            .iter()
            .any(|b| b.then == what && b.piece.item == item);
        if !made && can {
            out.push(Place::Make(item));
        }
    }
    out
}

/// The place she'd go to `want`, any of them as likely.
fn place(c: &Ctx, w: Whims, want: Want) -> Option<(Use, Place)> {
    let Want::Use(what) = want else {
        return None;
    };
    let places = places(what, c.chances, w);
    let i = w.below("place", places.len() as u64) as usize;
    places.get(i).map(|&place| (what, place))
}

fn use_real(c: &Ctx, w: Whims, want: Want) -> Option<Bind> {
    match place(c, w, want)? {
        (_, Place::Seat(seat)) if !seat.makeshift() => Some(Bind::Job(Job::Use(seat))),
        _ => None,
    }
}

fn use_made(c: &Ctx, w: Whims, want: Want) -> Option<Bind> {
    match place(c, w, want)? {
        (_, Place::Seat(seat)) if seat.makeshift() => Some(Bind::Job(Job::Use(seat))),
        _ => None,
    }
}

fn use_make(c: &Ctx, w: Whims, want: Want) -> Option<Bind> {
    match place(c, w, want)? {
        (what, Place::Make(item)) => {
            pick_build(what, item, c.chances, w).map(|b| Bind::Job(Job::Build(b.clone())))
        }
        _ => None,
    }
}

/// Just set down where it's right: back to where she felt it was wrong
/// (a sofa she'd watch from, if it faces the TV now; else lounge on).
fn use_it(c: &Ctx, _: Whims, _: Want) -> Option<Bind> {
    let (piece, what) = c.just_set?;
    let wants: &[Use] = if piece == Furniture::Sofa {
        &[Use::Watch, Use::Lounge, what]
    } else {
        &[what]
    };
    wants
        .iter()
        .find_map(|&w| {
            c.chances
                .seats
                .iter()
                .find(|s| s.piece == PieceRef::Real(piece) && s.what == w)
        })
        .map(|&seat| Bind::Job(Job::Use(seat)))
}

/// The piece in her pocket, to where it goes, as the last frame judged
/// it (and it still holds).
fn carry(c: &Ctx, _: Whims, _: Want) -> Option<Bind> {
    let ep = c.episode.filter(|e| e.pocket && !e.set_down)?;
    let judged = c.chances.judged.filter(|j| j.of(&ep) && j.holds)?;
    let ((x, y), side) = judged.spot?;
    Some(Bind::Job(Job::SetDown(SetDown {
        piece: ep.repair.piece,
        to: ep.repair.to,
        x,
        y,
        side,
    })))
}

/// The piece she means to move, to lift: the one she set off for, while
/// the last frame judged its move still holds; or, setting about it, one
/// of the cheapest ways to put the rule right (the first piece of the
/// rule's order among them; a whim among its places).
fn lift(c: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    let (repair, (x, y), side) = match c.episode {
        Some(ep) if !ep.pocket => {
            let judged = c.chances.judged.filter(|j| j.of(&ep) && j.holds)?;
            let (spot, side) = judged.spot?;
            (ep.repair, spot, side)
        }
        Some(_) => return None,
        None => {
            if !c.may_arrange {
                return None;
            }
            let first = c.chances.repairs.first()?;
            let tied: Vec<_> = c
                .chances
                .repairs
                .iter()
                .filter(|r| (r.tier, r.cost, r.piece) == (first.tier, first.cost, first.piece))
                .collect();
            let repair = **tied.get(w.below("repair", tied.len() as u64) as usize)?;
            let &(_, spot, side) = c
                .chances
                .lift_at
                .iter()
                .find(|(p, ..)| *p == repair.piece)?;
            (repair, spot, side)
        }
    };
    Some(Bind::Job(Job::Lift(Lift { repair, x, y, side })))
}

/// Where she makes a makeshift `item` for `what`: anywhere it can be
/// made, the chat a tenth as likely, and a sofa [`FACING_TV`] times as
/// likely where it would face the TV too.
pub(super) fn pick_build(
    what: Use,
    item: Furniture,
    chances: &Chances,
    w: Whims,
) -> Option<&Build> {
    let builds: Vec<&Build> = chances
        .builds
        .iter()
        .filter(|b| b.then == what && b.piece.item == item)
        .collect();
    let watches = |b: &Build| {
        chances
            .builds
            .iter()
            .any(|o| o.then == Use::Watch && (o.x, o.y, o.piece) == (b.x, b.y, b.piece))
    };
    let weight = |i: usize| {
        builds.get(i).map_or(1.0, |b| {
            let chat = if chances.in_chat((b.x, b.y)) {
                CHAT_FACTOR
            } else {
                1.0
            };
            let tv = if watches(b) { FACING_TV } else { 1.0 };
            chat * tv
        })
    };
    super::osaka::pick_weighted(builds.len(), weight, |n| w.below("build", n))
        .and_then(|i| builds.get(i).copied())
}

/// What she loses, and owes a beat for: a glance toward it, and maybe
/// a word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Loss {
    /// A piece she made, gone before she used it.
    Piece(Furniture),
    /// Text she was tearing off, put back.
    Tear,
    /// Where she was heading.
    Heading,
    /// A piece she made, let be after her tries.
    LetBe,
    /// A piece she was moving, put back where it stood.
    Moved(Furniture),
}

impl Loss {
    /// What she might say, from which pool, and how often (n in d).
    pub fn says(self) -> Option<(&'static [&'static str], u64, u64)> {
        match self {
            Self::Piece(Furniture::Bed) => Some((&["...my bed."], 1, 1)),
            Self::Piece(_) => Some((&["...my sofa."], 1, 1)),
            Self::Tear => Some((&["...never mind."], 1, 4)),
            Self::Heading => None,
            Self::LetBe => Some((&["Nah."], 1, 2)),
            Self::Moved(_) => Some((&["Oh well..."], 1, 2)),
        }
    }
}

/// What she sometimes says, going back to a piece she made after an
/// interruption.
pub(super) const AH_RIGHT: &[&str] = &["Ah, right!"];

/// A beat she owes: a glance toward where `loss` happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Beat {
    pub loss: Loss,
    pub toward: (i32, i32),
}

/// A line waits this long before she says it again.
const LINE_COOLDOWN_MS: u64 = 10 * 60_000;
/// Beat lines a visit, at most.
const LINE_BUDGET: usize = 8;

/// The beat lines she has said this visit, and when.
#[derive(Clone, Debug, Default)]
pub(super) struct Lines {
    said: Vec<(&'static str, u64)>,
}

impl Lines {
    /// One of `pool`, `n` times in `d`, unless she said it lately or has
    /// said enough this visit.
    pub fn pick(
        &mut self,
        (pool, n, d): (&'static [&'static str], u64, u64),
        w: Whims,
        at: u64,
    ) -> Option<&'static str> {
        if self.said.len() >= LINE_BUDGET || !w.chance("line", 0, n, d) {
            return None;
        }
        let fresh: Vec<&'static str> = pool
            .iter()
            .copied()
            .filter(|line| {
                !self
                    .said
                    .iter()
                    .any(|&(said, when)| said == *line && at < when + LINE_COOLDOWN_MS)
            })
            .collect();
        let line = *fresh.get(w.below("which-line", fresh.len() as u64) as usize)?;
        self.said.push((line, at));
        Some(line)
    }
}

/// Rows a line may have scrolled up (chat scrolls up) and still be the
/// one she set off for.
pub(super) const SCROLLED: u16 = 4;

/// Where she's heading off her floor: what she wants there, and the job
/// as she set off for it, by which she knows it again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Heading {
    pub want: Want,
    pub job: Job,
}

impl Heading {
    /// She's heading for a piece she made this visit (what she made it
    /// for is on the piece, and she comes back to it anyway).
    pub fn mine(&self) -> bool {
        matches!(&self.job, Job::Use(seat) if seat.makeshift())
    }

    /// The job as this frame offers it, found by meaning: a use of the
    /// same piece for the same thing, wherever it now stands; text by its
    /// glyphs in the same columns, on its row or up to [`SCROLLED`] rows
    /// above (the least scrolled first).
    pub fn find(&self, chances: &Chances) -> Option<Job> {
        let near = |row: u16| row <= self.row() && row + SCROLLED >= self.row();
        match &self.job {
            Job::Use(seat) => chances
                .seats
                .iter()
                .find(|s| s.piece == seat.piece && s.what == seat.what)
                .map(|&s| Job::Use(s)),
            Job::Pull(was) => chances
                .pulls
                .iter()
                .filter(|p| near(p.row) && p.cells == was.cells && p.glyphs == was.glyphs)
                .max_by_key(|p| p.row)
                .cloned()
                .map(Job::Pull),
            Job::Swap(was) => chances
                .swaps
                .iter()
                .filter(|s| {
                    near(s.row)
                        && (s.a.at.0, s.b.at.0) == (was.a.at.0, was.b.at.0)
                        && s.glyphs == was.glyphs
                })
                .max_by_key(|s| s.row)
                .cloned()
                .map(Job::Swap),
            Job::Build(was) => chances
                .builds
                .iter()
                .filter(|b| {
                    near(b.row)
                        && b.cells == was.cells
                        && b.glyphs == was.glyphs
                        && b.piece.item == was.piece.item
                        && b.then == was.then
                })
                .max_by_key(|b| b.row)
                .cloned()
                .map(Job::Build),
            // The move she's making, as the last frame judged it.
            Job::Lift(was) => {
                let judged = chances.judged.filter(|j| {
                    !j.pocket && j.holds && (j.piece, j.to) == (was.repair.piece, was.repair.to)
                })?;
                let ((x, y), side) = judged.spot?;
                Some(Job::Lift(Lift { x, y, side, ..*was }))
            }
            Job::SetDown(was) => {
                let judged = chances
                    .judged
                    .filter(|j| j.pocket && j.holds && (j.piece, j.to) == (was.piece, was.to))?;
                let ((x, y), side) = judged.spot?;
                Some(Job::SetDown(SetDown { x, y, side, ..*was }))
            }
        }
    }

    /// The text row she set off for (0 for a piece).
    fn row(&self) -> u16 {
        match &self.job {
            Job::Pull(p) => p.row,
            Job::Swap(s) => s.row,
            Job::Build(b) => b.row,
            Job::Use(_) | Job::Lift(_) | Job::SetDown(_) => 0,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::scenes::{Pull, Side};
    use super::*;

    fn pull(row: u16, glyphs: &str) -> Pull {
        Pull {
            x: 10,
            y: i32::from(row) + 2,
            row,
            side: Side::Left,
            cells: vec![2, 3, 4],
            glyphs: glyphs.to_owned(),
            gap: 0,
        }
    }

    fn heading(job: Pull) -> Heading {
        Heading {
            want: Want::Pull,
            job: Job::Pull(job),
        }
    }

    fn offering(pulls: Vec<Pull>) -> Chances {
        Chances {
            pulls,
            ..Chances::default()
        }
    }

    /// Every line she says fits a bubble (24 characters).
    #[test]
    fn beat_lines_fit_a_bubble() {
        let losses = [
            Loss::Piece(Furniture::Sofa),
            Loss::Piece(Furniture::Bed),
            Loss::Tear,
            Loss::Heading,
            Loss::LetBe,
            Loss::Moved(Furniture::Sofa),
        ];
        let pools = losses
            .iter()
            .filter_map(|l| l.says())
            .map(|(pool, ..)| pool);
        for line in pools.flatten().chain(AH_RIGHT) {
            assert!(line.chars().count() <= 24, "{line}");
        }
    }

    /// A line isn't said twice within ten minutes, and a visit has a
    /// budget of them.
    #[test]
    fn lines_cool_down_and_run_out() {
        let mut lines = Lines::default();
        let pool: (&'static [&'static str], u64, u64) = (&["a"], 1, 1);
        assert_eq!(lines.pick(pool, Whims(1), 0), Some("a"));
        assert_eq!(lines.pick(pool, Whims(2), 60_000), None);
        assert_eq!(lines.pick(pool, Whims(3), LINE_COOLDOWN_MS), Some("a"));
        let mut said = 2;
        let mut at = 2 * LINE_COOLDOWN_MS;
        while lines.pick(pool, Whims(at), at).is_some() {
            said += 1;
            at += LINE_COOLDOWN_MS;
        }
        assert_eq!(said, LINE_BUDGET);
    }

    /// A line she set off for is the same line when it has scrolled up a
    /// row or few (chat scrolls up): the same glyphs in the same columns.
    /// Changed text, or a line that moved down or scrolled too far, isn't.
    #[test]
    fn a_heading_finds_its_line_after_a_scroll() {
        let set_off = heading(pull(20, "abc"));
        let found = |pulls| set_off.find(&offering(pulls));
        assert_eq!(
            found(vec![pull(20, "abc")]),
            Some(Job::Pull(pull(20, "abc")))
        );
        assert_eq!(
            found(vec![pull(18, "abc")]),
            Some(Job::Pull(pull(18, "abc")))
        );
        assert_eq!(found(vec![pull(20, "abd")]), None);
        assert_eq!(found(vec![pull(21, "abc")]), None);
        assert_eq!(found(vec![pull(20 - SCROLLED - 1, "abc")]), None);
        // Of two that would do, the least scrolled.
        assert_eq!(
            found(vec![pull(17, "abc"), pull(19, "abc")]),
            Some(Job::Pull(pull(19, "abc")))
        );
    }
}
