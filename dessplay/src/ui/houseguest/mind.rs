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
use super::rules::Trials;
use super::scenes::{Build, Job, Lift, SetDown};
use super::script::ScriptId;
use super::terrain::{Link, Route, Terrain};

/// Her mind's stream is seeded from the body's first draw, salted.
pub(super) const MIND_SALT: u64 = 0x6d69_6e64_5f6f_6661;
/// Her whims before her first decision are her mind's seed, salted.
pub(super) const WHIMS_SALT: u64 = 0x7768_696d_735f_3021;

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

    /// The whims of the `salt`th of a series of choices made under
    /// `label` (her night's sleep-talk, each line its own: see
    /// [`SLEEP_TALK`]), from which that choice draws as from a
    /// decision's.
    pub fn series(self, label: &str, salt: u64) -> Whims {
        Whims(self.roll(label, salt))
    }

    /// With probability `p` (0 never, 1 always).
    pub fn odds(self, label: &str, salt: u64, p: f64) -> bool {
        const D: u64 = 1 << 20;
        let n = (p.clamp(0.0, 1.0) * D as f64).round() as u64;
        self.chance(label, salt, n, D)
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
    /// Space out musing: saying one of her musings first, or now and
    /// then telling a riddle (or nothing, if every musing is cooling).
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

/// Space out musing (saying a musing, or now and then telling a
/// riddle): one time in three.
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

/// The piece she means to move, to lift: the one she set off for (or
/// will try in another spot), while the last frame judged its move
/// still holds; or, setting about it, one of the cheapest ways to put
/// the rule right (the first piece of the rule's order among them; a
/// whim among its places as good as the cheapest, the others kept to
/// try it in after: [`Trials`]).
fn lift(c: &Ctx, w: Whims, _: Want) -> Option<Bind> {
    let (repair, trials, (x, y), side) = match c.episode {
        Some(ep) if !ep.pocket && !ep.trying => {
            let judged = c.chances.judged.filter(|j| j.of(&ep) && j.holds)?;
            let (spot, side) = judged.spot?;
            (ep.repair, ep.trials, spot, side)
        }
        Some(_) => return None,
        None => {
            if !c.may_arrange {
                return None;
            }
            // The order she'd try them in: a whim's.
            let mut ties = Trials::ties(&c.chances.repairs);
            for i in 0..ties.len() {
                let j = i + w.below_at("repair", i as u64, (ties.len() - i) as u64) as usize;
                ties.swap(i, j);
            }
            let repair = *ties.first()?;
            let &(_, spot, side) = c
                .chances
                .lift_at
                .iter()
                .find(|(p, ..)| *p == repair.piece)?;
            (repair, Trials::of(&ties), spot, side)
        }
    };
    Some(Bind::Job(Job::Lift(Lift {
        repair,
        trials,
        x,
        y,
        side,
    })))
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
    /// What she might say: lines from the beat pool, and how often.
    pub fn says(self) -> Option<Pool> {
        match self {
            Self::Piece(Furniture::Bed) => Some(MY_BED),
            Self::Piece(_) => Some(MY_SOFA),
            Self::Tear => Some(NEVER_MIND),
            Self::Heading => None,
            Self::LetBe => Some(NAH),
            Self::Moved(_) => Some(OH_WELL),
        }
    }

    /// Which kind of loss it is: its place among the kinds
    /// [`Loss::all`] lists. Wildcard-free, so a new loss doesn't compile
    /// until it's given a kind here, and `every_loss_is_listed` holds
    /// [`Loss::all`] to listing every kind (a kind never given a place
    /// here can't be caught: Rust can't list an enum's variants).
    #[cfg(test)]
    fn kind(self) -> usize {
        match self {
            Self::Piece(_) => 0,
            Self::Tear => 1,
            Self::Heading => 2,
            Self::LetBe => 3,
            Self::Moved(_) => 4,
        }
    }

    /// How many kinds of loss there are (see [`Loss::kind`]).
    #[cfg(test)]
    const KINDS: usize = 5;

    /// Every loss there is, each piece of furniture for those that
    /// name one.
    #[cfg(test)]
    fn all() -> Vec<Loss> {
        Furniture::ALL
            .into_iter()
            .flat_map(|f| [Self::Piece(f), Self::Moved(f)])
            .chain([Self::Tear, Self::Heading, Self::LetBe])
            .collect()
    }
}

/// Her bed, gone before she slept in it.
const MY_BED: Pool = Pool::beat(&[line!("...my bed.")], 1, 1);
/// Her sofa (or any other piece), gone before she used it.
const MY_SOFA: Pool = Pool::beat(&[line!("...my sofa.")], 1, 1);
/// Text she was tearing off, put back.
const NEVER_MIND: Pool = Pool::beat(&[line!("...never mind.")], 1, 4);
/// A piece she made, let be.
const NAH: Pool = Pool::beat(&[line!("Nah.")], 1, 2);
/// A piece she was moving, put back.
const OH_WELL: Pool = Pool::beat(&[line!("Oh well...")], 1, 2);

/// What she sometimes says, going back to a piece she made after an
/// interruption.
pub(super) const AH_RIGHT: Pool = Pool::beat(&[line!("Ah, right!")], 1, 3);

/// What she sometimes says, out of a door somewhere new (one door in
/// three: the rest she comes through quietly).
pub(super) const DOOR: Pool = Pool {
    id: PoolId::Door,
    lines: &[
        line!("Where was I?"),
        line!("Huh? How'd I get here?"),
        line!("...What was I doin'?"),
        line!("I forgot what I forgot."),
        line!("Handy, these doors."),
    ],
    n: 1,
    d: 3,
};

/// Things she says when spacing out (when she isn't telling a riddle).
/// What grew into her rare things (the escalator, the Dream's "Oh my
/// gah!", Chiyo-chan's dad in it) isn't here: she says those only as
/// they're rare (phase 5b D6).
pub(super) const MUSINGS: Pool = Pool {
    id: PoolId::Musing,
    lines: &[
        line!("I wish I were a bird."),
        line!("Why is the sky blue?"),
        line!("Melon bread..."),
        line!("Black spots on white?"),
        line!("Or white on black..."),
        line!("Feels like I could fly."),
        line!("Which hand's left..."),
        line!("Nanja-kora."),
    ],
    n: 1,
    d: 1,
};

/// Her sleep-talk's lines, every night.
const SLEEP_TALK_LINES: [&str; 7] = [
    line!("Mm... melon bread..."),
    line!("...no, the other hand..."),
    line!("Nanja-kora..."),
    line!("...penguins..."),
    line!("...it's floatin'..."),
    line!("Mm... Chiyo-chan..."),
    line!("...not a sponge..."),
];

/// What she says in her sleep, now and then through the night (phase 5b
/// A14; see [`LAST_HOUR_TALK`] for the night's last game hour, and
/// [`NEW_YEAR_TALK`] for the New Year's).
pub(super) const SLEEP_TALK: Pool = Pool {
    id: PoolId::SleepTalk,
    lines: &SLEEP_TALK_LINES,
    n: 1,
    d: 1,
};

/// `lines`, and `line` after them (`M` is one more than `N`).
const fn and<const N: usize, const M: usize>(
    lines: [&'static str; N],
    line: &'static str,
) -> [&'static str; M] {
    assert!(M == N + 1, "one line more");
    let mut out = [line; M];
    let mut i = 0;
    while i < N {
        out[i] = lines[i];
        i += 1;
    }
    out
}

/// The New Year's sleep-talk (Jan 1-3, her calendar's tint): every
/// night's lines, and a dream of the first of the year.
const NEW_YEAR_TALK_LINES: [&str; 8] = and(SLEEP_TALK_LINES, line!("Pigtails... flying..."));

/// What she says in her sleep through the New Year (see [`SLEEP_TALK`]).
pub(super) const NEW_YEAR_TALK: Pool = Pool {
    lines: &NEW_YEAR_TALK_LINES,
    ..SLEEP_TALK
};

/// What she says in her sleep in the last game hour before she wakes.
pub(super) const LAST_HOUR_TALK: Pool = Pool {
    id: PoolId::SleepTalk,
    lines: &[line!("...five more minutes")],
    n: 1,
    d: 1,
};

/// What she says as she leaves for school through her door (round 1:
/// pooled).
pub(super) const OFF: Pool = Pool {
    id: PoolId::Routine,
    lines: &[line!("I'm off!"), line!("Off to school!")],
    n: 1,
    d: 1,
};

/// What she says as she comes home from school out of her door (round
/// 1: pooled; from work it's "I'm home!" with her shopping, on its own).
pub(super) const HOME: Pool = Pool {
    id: PoolId::Routine,
    lines: &[line!("I'm home!"), line!("Tadaima!")],
    n: 1,
    d: 1,
};

/// Her first snack of a morning (by her routine).
pub(super) const BREAKFAST: Pool = Pool {
    id: PoolId::Routine,
    lines: &[line!("Breakfast!")],
    n: 1,
    d: 1,
};

/// Her first snack of an evening (by her routine).
pub(super) const DINNER: Pool = Pool {
    id: PoolId::Routine,
    lines: &[line!("Dinner time~")],
    n: 1,
    d: 1,
};

/// Up on a morning with no school (a weekend, a vacation).
pub(super) const NO_SCHOOL: Pool = Pool {
    id: PoolId::Routine,
    lines: &[line!("No school today!")],
    n: 1,
    d: 1,
};

/// The lamp off for the night.
pub(super) const NIGHT_NIGHT: Pool = Pool {
    id: PoolId::Routine,
    lines: &[line!("Night-night...")],
    n: 1,
    d: 1,
};

/// What her calendar has her say, owed once a day (see
/// [`super::calendar::Owed`]): said as it's owed, never drawn (so only the
/// tests list it: each counts as said from its pool).
#[cfg(test)]
pub(super) const CALENDAR: Pool = Pool {
    id: PoolId::Calendar,
    lines: &super::calendar::GREETINGS,
    n: 1,
    d: 1,
};

/// What she muses on in December (her calendar's tint), one musing in
/// three.
pub(super) const DECEMBER: Pool = Pool {
    id: PoolId::December,
    lines: &[
        line!("Rudolph's nose... why?"),
        line!("Snow tastes of nothing."),
        line!("Kotatsu... kotatsu..."),
    ],
    n: 1,
    d: 3,
};

/// What she muses on in summer's last week (her calendar's tint), one
/// musing in two.
pub(super) const PANIC: Pool = Pool {
    id: PoolId::Panic,
    lines: &[line!("Homework! Homework!")],
    n: 1,
    d: 2,
};

/// The riddles she tells, spacing out: each question, and the answer
/// she gives at once herself. Drawn whole (by its question).
pub(super) const RIDDLES: [(&str, &str); 6] = [
    (line!("Bread ya can't eat?"), line!("A fryin' pan!")),
    (line!("What has keys, no locks?"), line!("A keyboard!")),
    (line!("Has a neck but no head?"), line!("A bottle!")),
    (line!("All holes, holds water?"), line!("A sponge!")),
    (line!("Goes up, never down?"), line!("Yer age!")),
    (line!("What never comes today?"), line!("Tomorrow!")),
];

/// One half of each of `pairs`: the first, or the second.
const fn halves<const N: usize>(
    pairs: [(&'static str, &'static str); N],
    second: bool,
) -> [&'static str; N] {
    let mut out = [""; N];
    let mut i = 0;
    while i < N {
        out[i] = if second { pairs[i].1 } else { pairs[i].0 };
        i += 1;
    }
    out
}

/// The riddles' questions.
const QUESTIONS: [&str; RIDDLES.len()] = halves(RIDDLES, false);
/// The riddles' answers.
#[cfg(test)]
const ANSWERS: [&str; RIDDLES.len()] = halves(RIDDLES, true);

/// A riddle (by its question), on one musing in three.
pub(super) const RIDDLE: Pool = Pool {
    id: PoolId::Riddle,
    lines: &QUESTIONS,
    n: 1,
    d: 3,
};

/// Riddle `question`'s place in [`RIDDLES`].
pub(super) fn riddle_of(question: &str) -> Option<usize> {
    RIDDLES.iter().position(|&(q, _)| q == question)
}

/// A beat she owes: a glance toward where `loss` happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Beat {
    pub loss: Loss,
    pub toward: (i32, i32),
}

/// A line waits this long before she says it again.
const LINE_COOLDOWN_MS: u64 = 10 * 60_000;
/// Beat lines a game day (a visit's, her routine unfed), at most (lines
/// from other pools aren't counted).
const LINE_BUDGET: usize = 8;
/// A script waits this long before she plays it again.
const SCRIPT_COOLDOWN_MS: u64 = 10 * 60_000;

/// Which pool a line is drawn from. Its id salts the rolls drawing from
/// it, so two picks in one decision don't share a roll; only beat lines
/// count toward a game day's (a visit's, unfed) [`LINE_BUDGET`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PoolId {
    /// What she says over a beat she owes, or going back to something
    /// she made (id 0: these were the only lines drawn before pools had
    /// ids, so they keep their rolls).
    Beat,
    /// What she says out of a door ([`DOOR`]).
    Door,
    /// What she says spacing out ([`MUSINGS`]).
    Musing,
    /// The riddles she tells spacing out ([`RIDDLES`]): questions and
    /// answers both.
    Riddle,
    /// What she says in her sleep ([`SLEEP_TALK`], [`LAST_HOUR_TALK`]).
    SleepTalk,
    /// What she says leaving for school and coming home ([`OFF`],
    /// [`HOME`]), and at the times of her day ([`BREAKFAST`], [`DINNER`],
    /// [`NO_SCHOOL`], [`NIGHT_NIGHT`]).
    Routine,
    /// What her calendar owes her to say (its greetings).
    Calendar,
    /// Her December musings ([`DECEMBER`]).
    December,
    /// Her musing in summer's panic week ([`PANIC`]).
    Panic,
    /// A test's pool, with no budget: not a pool she draws from.
    #[cfg(test)]
    Test,
}

impl PoolId {
    /// Every pool there is.
    #[cfg(test)]
    pub const ALL: [PoolId; 10] = [
        Self::Beat,
        Self::Door,
        Self::Musing,
        Self::Riddle,
        Self::SleepTalk,
        Self::Routine,
        Self::Calendar,
        Self::December,
        Self::Panic,
        Self::Test,
    ];

    /// The salt of its rolls.
    fn id(self) -> u64 {
        match self {
            Self::Beat => 0,
            Self::Door => 1,
            Self::Musing => 2,
            Self::Riddle => 3,
            Self::SleepTalk => 4,
            Self::Routine => 5,
            Self::Calendar => 6,
            Self::December => 7,
            Self::Panic => 8,
            // Out of the way of every pool she draws from.
            #[cfg(test)]
            Self::Test => u64::MAX,
        }
    }

    /// Whether its lines count toward a game day's (a visit's, unfed)
    /// [`LINE_BUDGET`].
    fn budgeted(self) -> bool {
        match self {
            Self::Beat => true,
            Self::Door
            | Self::Musing
            | Self::Riddle
            | Self::SleepTalk
            | Self::Routine
            | Self::Calendar
            | Self::December
            | Self::Panic => false,
            #[cfg(test)]
            Self::Test => false,
        }
    }

    /// Its place in [`PoolId::ALL`], and every pool it draws lines from.
    /// Wildcard-free, so a new pool doesn't compile until it's listed
    /// and says what it holds (for [`all_lines`]).
    #[cfg(test)]
    fn listed(self) -> (usize, Vec<Pool>) {
        match self {
            Self::Beat => (
                0,
                Loss::all()
                    .into_iter()
                    .filter_map(Loss::says)
                    .chain([AH_RIGHT])
                    .collect(),
            ),
            Self::Door => (1, vec![DOOR]),
            Self::Musing => (2, vec![MUSINGS]),
            // The answers aren't drawn (each comes with its question),
            // but they're said, so they're listed.
            Self::Riddle => (
                3,
                vec![
                    RIDDLE,
                    Pool {
                        lines: &ANSWERS,
                        ..RIDDLE
                    },
                ],
            ),
            Self::SleepTalk => (4, vec![SLEEP_TALK, LAST_HOUR_TALK, NEW_YEAR_TALK]),
            Self::Routine => (
                5,
                vec![OFF, HOME, BREAKFAST, DINNER, NO_SCHOOL, NIGHT_NIGHT],
            ),
            Self::Calendar => (6, vec![CALENDAR]),
            Self::December => (7, vec![DECEMBER]),
            Self::Panic => (8, vec![PANIC]),
            Self::Test => (9, Vec::new()),
        }
    }
}

/// Lines she might say, from a pool, `n` times in `d`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Pool {
    pub id: PoolId,
    pub lines: &'static [&'static str],
    pub n: u64,
    pub d: u64,
}

impl Pool {
    /// `lines`, from the beat pool.
    const fn beat(lines: &'static [&'static str], n: u64, d: u64) -> Self {
        Self {
            id: PoolId::Beat,
            lines,
            n,
            d,
        }
    }
}

/// Every pooled line she might say, with its pool (each is checked as
/// it's written, by `line!`).
#[cfg(test)]
pub(super) fn all_lines() -> Vec<(PoolId, &'static str)> {
    PoolId::ALL
        .into_iter()
        .flat_map(|id| id.listed().1)
        .flat_map(|pool| pool.lines.iter().map(move |&line| (pool.id, line)))
        .collect()
}

/// The lines she has said this visit (from which pool, and when), and
/// the scripts she has played.
#[derive(Clone, Debug, Default)]
pub(super) struct Lines {
    said: Vec<(PoolId, &'static str, u64)>,
    played: Vec<(ScriptId, u64)>,
    /// Since when her [`LINE_BUDGET`] counts what she says: the visit's
    /// start, or (her routine fed) the morning she woke this visit.
    budget_since: u64,
    /// Beat lines she said earlier the same game day, on an earlier
    /// visit (her routine fed: the budget is the day's, not the visit's).
    carried: usize,
}

impl Lines {
    /// One of `pool`'s lines, `n` times in `d`, unless she said it lately
    /// or (a beat line) has said enough today. Said is said, whether
    /// or not it shows, unless it's spoken over the instant it's said
    /// (see [`Lines::unsay`]): then it never could show.
    pub fn pick(&mut self, pool: Pool, w: Whims, at: u64) -> Option<&'static str> {
        let spent = || self.spent() >= LINE_BUDGET;
        if pool.id.budgeted() && spent() || !w.chance("line", pool.id.id(), pool.n, pool.d) {
            return None;
        }
        let fresh: Vec<&'static str> = pool
            .lines
            .iter()
            .copied()
            .filter(|line| {
                !self
                    .said
                    .iter()
                    .any(|&(_, said, when)| said == *line && at < when + LINE_COOLDOWN_MS)
            })
            .collect();
        let which = w.below_at("which-line", pool.id.id(), fresh.len() as u64);
        let line = *fresh.get(which as usize)?;
        self.said.push((pool.id, line, at));
        Some(line)
    }

    /// The beat lines she has said today: on this visit since the budget
    /// last started afresh, and on earlier visits the same game day.
    pub fn spent(&self) -> usize {
        let since = self.budget_since;
        self.carried
            + self
                .said
                .iter()
                .filter(|&&(id, _, at)| id.budgeted() && at >= since)
                .count()
    }

    /// A new game day began at `at` (she woke): her budget starts afresh.
    /// What she said lately still cools.
    pub fn new_day(&mut self, at: u64) {
        self.budget_since = at;
        self.carried = 0;
    }

    /// She said `spent` beat lines earlier today, on another visit.
    pub fn carry(&mut self, spent: usize) {
        self.carried = spent;
    }

    /// `line`, from `pool`, said at `at` (drawn with another of the
    /// pool's lines, as a riddle's answer is with its question, and
    /// recorded as said from when it's shown).
    pub fn note(&mut self, pool: PoolId, line: &'static str, at: u64) {
        self.said.push((pool, line, at));
    }

    /// `line`, said at `at`, was spoken over in that same instant: it
    /// never showed, so it isn't said after all.
    pub fn unsay(&mut self, line: &str, at: u64) {
        self.said
            .retain(|&(_, said, when)| !(said == line && when == at));
    }

    /// Every line she has said: from which pool, and from when.
    #[cfg(test)]
    pub fn said(&self) -> &[(PoolId, &'static str, u64)] {
        &self.said
    }

    /// Whether she may play `script` at `at` (she hasn't in the last ten
    /// minutes), and if so, she does.
    pub fn try_play(&mut self, script: ScriptId, at: u64) -> bool {
        let lately = self
            .played
            .iter()
            .any(|&(played, when)| played == script && at < when + SCRIPT_COOLDOWN_MS);
        if !lately {
            self.played.push((script, at));
        }
        !lately
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

    /// Lint: every pooled line fits a bubble (`line!` already makes an
    /// over-long one a compile error), and so does every line a script
    /// says; and every pool has a line to say.
    /// Every pool lists its own lines, all of them: the beat pool's six.
    #[test]
    fn every_pooled_line_fits_a_bubble() {
        for (pool, line) in all_lines() {
            assert!(super::super::fits_a_bubble(line), "{pool:?}: {line:?}");
            assert!(!line.is_empty(), "{pool:?}");
        }
        // And every line her scripts' keys say, pooled or not.
        use super::super::script::{self, ScriptId};
        let scripted = script::script_lines();
        for &(id, line) in &scripted {
            assert!(super::super::fits_a_bubble(line), "{id:?}: {line:?}");
            assert!(!line.is_empty(), "{id:?}");
        }
        for (id, line) in [
            (ScriptId::Setsubun, script::ONI_WA_SOTO),
            (ScriptId::Setsubun, script::FUKU_WA_UCHI),
            (ScriptId::FirstSunrise, script::FIRST_SUNRISE_LINE),
            (ScriptId::DashForgot, script::FORGOT_SOMETHING),
            (ScriptId::DashForgot, script::WHAT_WAS_IT),
            (ScriptId::Dream, script::EVERYNYAN),
            (ScriptId::Dream, script::SANKYU),
            (ScriptId::Dream, script::OH_MY_GAH),
            (ScriptId::NoMelon, script::LAST_ONE),
            (ScriptId::Escalator, script::BOX_ONE),
            (ScriptId::Escalator, script::ESCALATOR_NO),
            (ScriptId::Scary, script::STORY_TIME),
            (ScriptId::Scary, script::NOT_MINE),
        ] {
            assert!(scripted.contains(&(id, line)), "{id:?}: {line:?}");
        }
        for (i, id) in PoolId::ALL.into_iter().enumerate() {
            let (at, pools) = id.listed();
            assert_eq!(at, i, "{id:?}");
            for pool in pools {
                assert_eq!(pool.id, id, "{pool:?}");
                assert!(!pool.lines.is_empty(), "{pool:?}");
            }
        }
        let distinct = |id: PoolId| {
            let mut lines: Vec<&str> = all_lines()
                .into_iter()
                .filter(|&(pool, _)| pool == id)
                .map(|(_, line)| line)
                .collect();
            lines.sort_unstable();
            lines.dedup();
            lines
        };
        assert_eq!(
            distinct(PoolId::Beat),
            [
                "...my bed.",
                "...my sofa.",
                "...never mind.",
                "Ah, right!",
                "Nah.",
                "Oh well...",
            ]
        );
        assert_eq!(distinct(PoolId::Door).len(), DOOR.lines.len());
        assert_eq!(distinct(PoolId::Musing).len(), MUSINGS.lines.len());
        // The andagi is a vignette now, not a musing.
        assert!(!MUSINGS.lines.iter().any(|l| l.contains("andagi")));
        // Both halves of every riddle, each once.
        assert_eq!(distinct(PoolId::Riddle).len(), 2 * RIDDLES.len());
        for (question, answer) in RIDDLES {
            assert!(distinct(PoolId::Riddle).contains(&question));
            assert!(distinct(PoolId::Riddle).contains(&answer));
        }
        // Her sleep-talk: some eight lines, the last hour's apart, and
        // the New Year's dream.
        assert_eq!(
            distinct(PoolId::SleepTalk).len(),
            SLEEP_TALK.lines.len() + LAST_HOUR_TALK.lines.len() + 1
        );
        assert_eq!(distinct(PoolId::SleepTalk).len(), 9);
        assert!(
            SLEEP_TALK
                .lines
                .iter()
                .all(|line| NEW_YEAR_TALK.lines.contains(line))
        );
        // Her routine's: off and home, two each; a line each for the
        // times of her day.
        assert_eq!(distinct(PoolId::Routine).len(), 2 + 2 + 4);
        // Her calendar's: every greeting a date owes, each once.
        assert_eq!(
            distinct(PoolId::Calendar).len(),
            super::super::calendar::GREETINGS.len()
        );
        // Her seasons' musings: December's three, panic week's one, none
        // a musing she says the year round.
        assert_eq!(distinct(PoolId::December).len(), 3);
        assert_eq!(distinct(PoolId::Panic).len(), 1);
        assert!(distinct(PoolId::Test).is_empty());
        // No line is in two pools, so a line said is said from one.
        let mut pool_of: std::collections::BTreeMap<&str, PoolId> = Default::default();
        for (pool, line) in all_lines() {
            if let Some(other) = pool_of.insert(line, pool) {
                assert_eq!(other, pool, "{line:?} in two pools");
            }
        }
        // Characters, not bytes.
        assert!(super::super::fits_a_bubble(&"…".repeat(24)));
        assert!(!super::super::fits_a_bubble(&"a".repeat(25)));
    }

    /// [`Loss::all`] lists every kind of loss (see [`Loss::kind`]), each
    /// piece of furniture for those that name one, and nothing twice.
    #[test]
    fn every_loss_is_listed() {
        let all = Loss::all();
        for kind in 0..Loss::KINDS {
            assert!(all.iter().any(|l| l.kind() == kind), "kind {kind}");
        }
        assert!(all.iter().all(|l| l.kind() < Loss::KINDS));
        for f in Furniture::ALL {
            assert!(all.contains(&Loss::Piece(f)), "{f:?}");
            assert!(all.contains(&Loss::Moved(f)), "{f:?}");
        }
        for (i, loss) in all.iter().enumerate() {
            assert!(!all[..i].contains(loss), "{loss:?} twice");
        }
    }

    /// A pool of the lines `lines`, `id`, always said.
    fn pool(id: PoolId, lines: &'static [&'static str]) -> Pool {
        Pool {
            id,
            lines,
            n: 1,
            d: 1,
        }
    }

    /// The beat pool keeps the salt its lines were drawn with before
    /// pools had ids (0), so the existing beats keep their rolls; every
    /// pool has a salt of its own, so two picks in one decision don't
    /// share a roll.
    #[test]
    fn every_pool_has_its_own_salt() {
        assert_eq!(PoolId::Beat.id(), 0);
        for (i, a) in PoolId::ALL.into_iter().enumerate() {
            for b in &PoolId::ALL[..i] {
                assert_ne!(a.id(), b.id(), "{a:?} and {b:?}");
            }
        }
    }

    /// Going back to a piece she made, she says "Ah, right!" one time in
    /// three, as the beat pool rolls (the roll it always had).
    #[test]
    fn ah_right_is_said_one_time_in_three() {
        assert_eq!(AH_RIGHT.id, PoolId::Beat);
        let mut said = 0;
        for seed in 0..300_u64 {
            let w = Whims(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
            let line = Lines::default().pick(AH_RIGHT, w, 0);
            let rolled = w.below_at("line", 0, 3) < 1;
            assert_eq!(line, rolled.then_some("Ah, right!"), "{seed}");
            said += usize::from(rolled);
        }
        assert!((60..140).contains(&said), "{said}");
    }

    /// Each pool's rolls (whether she says anything, and which line) are
    /// salted by its id: a pick is the line its own salt chooses, and two
    /// pools in one decision don't share a roll.
    #[test]
    fn a_pool_rolls_with_its_own_salt() {
        const LINES: &[&str] = &["a", "b", "c", "d", "e", "f", "g"];
        let mut differ = (0, 0);
        for id in [PoolId::Beat, PoolId::Test] {
            for seed in 0..200_u64 {
                let w = Whims(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
                // Which line: the one its salt chooses.
                let mut lines = Lines::default();
                let which = w.below_at("which-line", id.id(), LINES.len() as u64);
                assert_eq!(
                    lines.pick(pool(id, LINES), w, 0),
                    Some(LINES[which as usize]),
                    "{id:?} {seed}"
                );
                // Whether she says one at all: as its salt rolls.
                let mut lines = Lines::default();
                let sometimes = Pool {
                    n: 1,
                    d: 3,
                    ..pool(id, LINES)
                };
                assert_eq!(
                    lines.pick(sometimes, w, 0).is_some(),
                    w.chance("line", id.id(), 1, 3),
                    "{id:?} {seed}"
                );
            }
        }
        for seed in 0..200 {
            let w = Whims(seed);
            let pick = |id| Lines::default().pick(pool(id, LINES), w, 0);
            differ.0 += usize::from(pick(PoolId::Beat) != pick(PoolId::Test));
            differ.1 += 1;
        }
        // Seven lines: the same pick by chance about one time in seven.
        assert!(differ.0 * 2 > differ.1, "{differ:?}");
    }

    /// A line isn't said twice within ten minutes (from any pool), and a
    /// visit has a budget of beat lines, which other pools neither spend
    /// nor are held to.
    #[test]
    fn lines_cool_down_and_only_beats_run_out() {
        let beat = |lines: &'static [&'static str]| pool(PoolId::Beat, lines);
        let mut lines = Lines::default();
        assert_eq!(lines.pick(beat(&["a"]), Whims(1), 0), Some("a"));
        assert_eq!(lines.pick(beat(&["a"]), Whims(2), 60_000), None);
        // Cooling down whichever pool said it.
        assert_eq!(
            lines.pick(pool(PoolId::Test, &["a"]), Whims(2), 60_000),
            None
        );
        assert_eq!(
            lines.pick(beat(&["a"]), Whims(3), LINE_COOLDOWN_MS),
            Some("a")
        );
        let mut said = 2;
        let mut at = 2 * LINE_COOLDOWN_MS;
        while lines.pick(beat(&["a"]), Whims(at), at).is_some() {
            said += 1;
            at += LINE_COOLDOWN_MS;
        }
        assert_eq!(said, LINE_BUDGET);
        // Every pool but the beats', each by its own id.
        for id in PoolId::ALL.into_iter().filter(|&id| id != PoolId::Beat) {
            // Beats spent, another pool still has its say.
            let mut spent = lines.clone();
            assert_eq!(
                spent.pick(pool(id, &["b"]), Whims(at), at),
                Some("b"),
                "{id:?}"
            );
            // And another pool's lines don't spend the beats'.
            let mut other = Lines::default();
            for i in 0..2 * LINE_BUDGET as u64 {
                let at = i * LINE_COOLDOWN_MS;
                assert_eq!(
                    other.pick(pool(id, &["b"]), Whims(i), at),
                    Some("b"),
                    "{id:?}"
                );
            }
            let at = 2 * LINE_BUDGET as u64 * LINE_COOLDOWN_MS;
            assert_eq!(other.pick(beat(&["a"]), Whims(at), at), Some("a"), "{id:?}");
        }
    }

    /// Each pool she draws from speaks as often as it's meant to (of the
    /// times she might): the door one time in three (most doors are
    /// quiet), a musing always, a riddle on one musing in three, "Ah,
    /// right!" one time in three. Each as its own salt rolls.
    #[test]
    fn each_pool_speaks_as_often_as_it_should() {
        for (pool, n, d) in [
            (DOOR, 1, 3),
            (MUSINGS, 1, 1),
            (RIDDLE, 1, 3),
            (AH_RIGHT, 1, 3),
        ] {
            let mut said = 0_u64;
            let tries = 600_u64;
            for seed in 0..tries {
                let w = Whims(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
                let line = Lines::default().pick(pool, w, 0);
                assert_eq!(
                    line.is_some(),
                    w.chance("line", pool.id.id(), n, d),
                    "{:?} {seed}",
                    pool.id
                );
                said += u64::from(line.is_some());
            }
            // Within a fifth of the rate either way.
            let expected = tries * n / d;
            assert!(
                said * 5 >= expected * 4 && said * 5 <= expected * 6,
                "{:?}: {said} of {tries}, {n} in {d}",
                pool.id
            );
        }
    }

    /// Which line she says is drawn from the lines she hasn't said
    /// lately: with four of five cooling, always the fifth; with all
    /// five, none.
    #[test]
    fn a_line_is_drawn_from_those_not_cooling() {
        const FIVE: &[&str] = &["a", "b", "c", "d", "e"];
        let at = LINE_COOLDOWN_MS / 2;
        let mut lines = Lines::default();
        for (i, &line) in FIVE.iter().enumerate().filter(|&(i, _)| i != 2) {
            lines.said.push((PoolId::Test, line, i as u64 * 1000));
        }
        for seed in 0..200 {
            assert_eq!(
                lines
                    .clone()
                    .pick(pool(PoolId::Test, FIVE), Whims(seed), at),
                Some("c"),
                "{seed}"
            );
        }
        lines.said.push((PoolId::Test, "c", 0));
        for seed in 0..200 {
            assert_eq!(
                lines
                    .clone()
                    .pick(pool(PoolId::Test, FIVE), Whims(seed), at),
                None,
                "{seed}"
            );
        }
    }

    /// No script twice in ten minutes; each script cools down on its own.
    #[test]
    fn scripts_cool_down() {
        let mut lines = Lines::default();
        assert!(lines.try_play(ScriptId::Shopping, 0));
        assert!(!lines.try_play(ScriptId::Shopping, 5 * 60_000));
        assert!(lines.try_play(ScriptId::Snack, 5 * 60_000));
        assert!(!lines.try_play(ScriptId::Shopping, SCRIPT_COOLDOWN_MS - 1));
        assert!(lines.try_play(ScriptId::Shopping, SCRIPT_COOLDOWN_MS));
        assert!(!lines.try_play(ScriptId::Snack, SCRIPT_COOLDOWN_MS));
        assert!(lines.try_play(ScriptId::Snack, 15 * 60_000));
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
