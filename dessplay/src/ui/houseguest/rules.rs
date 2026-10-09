//! The rules of her home: how her pieces ought to stand, as a table.
//!
//! A rule is broken or not, judged on where her pieces are laid out
//! ([`super::room::Home::layout`]), never on what text closets this
//! frame; a satisfied rule never moves anything. A rule applies only to
//! pieces laid out and out of their boxes (a parcel isn't furniture
//! yet). Each has a grievance: what she says while using a piece it
//! involves, while it's broken.

use super::door::Keep;
use super::room::{
    self, Anchor, Extent, Furniture, Home, LaidOut, Nook, Prop, Role, Shown, Side, Strip,
    StripLaid, StripPlan, Use,
};
use super::sprite::Facing;
use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::Rect;

/// How some of her pieces ought to stand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rule {
    /// She can watch `screen` from `seat` (see [`room::faces`]).
    Faces { seat: Furniture, screen: Furniture },
    /// `a` stands on one strip with one of `b`, at most `gap` cells
    /// between them.
    Near {
        a: Furniture,
        b: &'static [Furniture],
        gap: i32,
    },
    /// The piece stands within a cell of one of its strip's walls.
    AgainstWall(Furniture),
    /// `a` and `b` aren't in one room (a room is a strip, for now).
    Apart { a: Furniture, b: Furniture },
    /// A piece she hasn't settled doesn't spoil its room: what the room
    /// is without it doesn't forbid anything it brings (a bed in a
    /// living room or a kitchen; a TV or a fridge in a bedroom).
    Belongs,
    /// Nothing stands in her door's space (door batch, D7): judged on
    /// the would-be space of her door's wall, kept or yielding, piece by
    /// piece (any piece out of its box whose cover meets it). Never on a
    /// strip too short or narrow for the space to exist.
    DoorClear,
    /// No piece stands in the chat pane (door batch, D7): piece by
    /// piece, any out of its box whose cover meets it, unless no room
    /// would take it (then it's in the closet: see [`room::stranded`]).
    InChat,
}

/// A rule, the uses of a piece it involves that she feels it on, and
/// what she says then.
#[derive(Debug)]
pub(super) struct RuleRow {
    pub rule: Rule,
    pub felt_on: &'static [Use],
    pub grievance: &'static str,
}

/// Every use she makes of a piece, once it's out of its box and in
/// shape: [`Rule::Belongs`] is felt on any.
pub(super) const ANY_USE: &[Use] = &[
    Use::Lounge,
    Use::Nap,
    Use::Sleep,
    Use::Homework,
    Use::Watch,
    Use::Read,
    Use::Snack,
    Use::Pet,
    Use::LookOut,
];

/// The rules, in the order they're judged.
pub(super) const RULES: [RuleRow; 8] = [
    RuleRow {
        rule: Rule::Faces {
            seat: Furniture::Sofa,
            screen: Furniture::Tv,
        },
        felt_on: &[Use::Lounge, Use::Nap],
        grievance: line!("Can't see the telly..."),
    },
    RuleRow {
        rule: Rule::Near {
            a: Furniture::Lamp,
            b: &[Furniture::Bed, Furniture::Desk],
            gap: 3,
        },
        felt_on: &[Use::Sleep, Use::Homework],
        grievance: line!("Too dark in here..."),
    },
    RuleRow {
        rule: Rule::AgainstWall(Furniture::Fridge),
        felt_on: &[Use::Snack],
        grievance: line!("This wants a wall..."),
    },
    RuleRow {
        rule: Rule::AgainstWall(Furniture::Bookshelf),
        felt_on: &[Use::Read],
        grievance: line!("Wobbly... needs a wall."),
    },
    RuleRow {
        rule: Rule::Apart {
            a: Furniture::Bed,
            b: Furniture::Tv,
        },
        felt_on: &[Use::Sleep],
        grievance: line!("Too noisy to sleep..."),
    },
    RuleRow {
        rule: Rule::Belongs,
        felt_on: ANY_USE,
        grievance: line!("Hm... not in here."),
    },
    RuleRow {
        rule: Rule::DoorClear,
        felt_on: ANY_USE,
        grievance: line!("Can't get to the door!"),
    },
    RuleRow {
        rule: Rule::InChat,
        felt_on: ANY_USE,
        grievance: line!("People are talking here…"),
    },
];

/// The row of [`RULES`] for a sofa facing the TV.
pub(super) const FACES_ROW: usize = 0;

/// How far from a wall a piece against it may stand, in cells.
const WALL_GAP: i32 = 1;

/// Which broken rule she feels: its row, and the piece it's judged on
/// ([`Rule::Belongs`]: the piece that doesn't belong; otherwise the
/// rule's first piece). It stays the same while the rule stays broken,
/// whatever else changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Grievance {
    pub row: usize,
    pub piece: Furniture,
}

impl Grievance {
    /// Its row of the table.
    pub fn rule(self) -> Option<&'static RuleRow> {
        RULES.get(self.row)
    }

    /// A few words for logs: the rule, and the piece it's judged on.
    pub fn label(self) -> String {
        format!("{}({})", kind(self.row), self.piece.spec().name)
    }
}

/// A rule broken this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Broken {
    /// Its row of [`RULES`].
    pub row: usize,
    /// The pieces moving would mend it, in the rule's order (for
    /// [`Rule::Apart`], one she hasn't settled first).
    pub pieces: Vec<Furniture>,
    /// Every piece it involves (using one, she may feel it), the piece
    /// it's judged on first.
    pub involved: Vec<Furniture>,
    /// What she feels, when she does.
    pub key: Grievance,
}

impl Broken {
    /// Its row of the table.
    pub fn rule(&self) -> Option<&'static RuleRow> {
        RULES.get(self.row)
    }

    /// A few words for the stage: the rule, and the pieces it would move.
    pub fn label(&self) -> String {
        let pieces: Vec<&str> = self.pieces.iter().map(|p| p.spec().name).collect();
        format!("{}({})", kind(self.row), pieces.join(","))
    }

    /// Whether using `piece` for `what` she'd feel it.
    pub fn felt_using(&self, piece: Furniture, what: Use) -> bool {
        self.involved.contains(&piece) && self.rule().is_some_and(|r| r.felt_on.contains(&what))
    }
}

/// A word for the kind of rule on `row`.
fn kind(row: usize) -> &'static str {
    match RULES.get(row).map(|r| r.rule) {
        Some(Rule::Faces { .. }) => "faces",
        Some(Rule::Near { .. }) => "near",
        Some(Rule::AgainstWall(_)) => "wall",
        Some(Rule::Apart { .. }) => "apart",
        Some(Rule::Belongs) => "belongs",
        Some(Rule::DoorClear) => "door",
        Some(Rule::InChat) => "chat",
        None => "?",
    }
}

/// Every rule broken by her pieces as `laid` has them on the plans it
/// was laid on, with what `home` knows of them (whether she has settled
/// each), and what the frame keeps pieces out of (`keep`: the chat
/// pane, for [`Rule::InChat`]), in [`RULES`] order (and the rules
/// judged piece by piece in layout order).
pub(super) fn broken(laid: &LaidOut, home: &Home, keep: &Keep) -> Vec<Broken> {
    broken_with(laid, home, keep, &closeted(laid, home, keep))
}

/// [`broken`], with the pieces `laid` has in the closet (`closet`, as
/// [`closeted`] finds them) known.
fn broken_with(laid: &LaidOut, home: &Home, keep: &Keep, closet: &[Furniture]) -> Vec<Broken> {
    let mut out: Vec<Broken> = Vec::new();
    for row in 0..RULES.len() {
        judge(row, laid, home, keep, closet, &mut out);
    }
    out
}

/// Her pieces `laid` has in the chat pane `keep` keeps them out of that
/// no room would take: in the closet ([`room::stranded`]). None unless a
/// piece meets the chat (the search is costly; the closet seldom used).
fn closeted(laid: &LaidOut, home: &Home, keep: &Keep) -> Vec<Furniture> {
    let chat = keep.chat();
    if !laid.shown.iter().any(|s| room::closetable(chat, s)) {
        return Vec::new();
    }
    room::stranded(home, laid, chat)
        .into_iter()
        .filter_map(|i| laid.shown.get(i).map(|s| s.item))
        .collect()
}

/// Push how the rule on `row` is broken by her pieces as `laid` has
/// them (see [`broken`]), those in `closet` hidden (see [`closeted`]).
fn judge(
    row: usize,
    laid: &LaidOut,
    home: &Home,
    keep: &Keep,
    closet: &[Furniture],
    out: &mut Vec<Broken>,
) {
    let layout = &laid.shown;
    let Some(line) = RULES.get(row) else { return };
    let here = |item: Furniture| {
        layout
            .iter()
            .find(|s| s.item == item && s.scrap.is_none() && !s.boxed && s.strip.is_some())
    };
    let settled = |item: Furniture| {
        home.props
            .iter()
            .find(|p| p.item == item)
            .is_none_or(|p| p.settled)
    };
    // `involved` starts with the piece the rule is judged on.
    let mut push = |pieces: Vec<Furniture>, involved: Vec<Furniture>| {
        if let Some(&piece) = involved.first() {
            out.push(Broken {
                row,
                pieces,
                key: Grievance { row, piece },
                involved,
            });
        }
    };
    match line.rule {
        Rule::Faces { seat, screen } => {
            if let (Some(a), Some(b)) = (here(seat), here(screen))
                && !room::faces(a, b)
            {
                push(vec![seat, screen], vec![seat, screen]);
            }
        }
        Rule::Near { a, b, gap } => {
            let Some(at) = here(a) else { return };
            let partners: Vec<&Shown> = b.iter().filter_map(|&item| here(item)).collect();
            if !partners.is_empty()
                && !partners
                    .iter()
                    .any(|p| p.strip == at.strip && between(at, p) <= gap)
            {
                let mut involved = vec![a];
                involved.extend(partners.iter().map(|p| p.item));
                push(vec![a], involved);
            }
        }
        Rule::AgainstWall(item) => {
            if let Some(at) = here(item)
                && let Some(plan) = at.strip.and_then(|s| laid.plan(s))
                && !against_wall(at, plan.floor)
            {
                push(vec![item], vec![item]);
            }
        }
        Rule::Apart { a, b } => {
            if let (Some(x), Some(y)) = (here(a), here(b))
                && x.strip == y.strip
            {
                let mut pieces = vec![a, b];
                // Unsettled first; otherwise in the rule's order.
                pieces.sort_by_key(|&p| settled(p));
                push(pieces, vec![a, b]);
            }
        }
        Rule::Belongs => {
            for s in layout {
                let Some(strip) = s.strip else { continue };
                if s.scrap.is_some() || s.boxed || settled(s.item) {
                    continue;
                }
                if spoils(layout, strip, s.item) {
                    push(vec![s.item], vec![s.item]);
                }
            }
        }
        // Her door's would-be space, kept or yielding (only its strip has
        // one, and only where it can exist at all: never on a short room).
        Rule::DoorClear => {
            for plan in &laid.plans {
                let Some(space) = plan.space else { continue };
                for s in layout {
                    if s.strip == Some(plan.strip)
                        && s.scrap.is_none()
                        && !s.boxed
                        && s.cover().intersects(space.rect)
                    {
                        push(vec![s.item], vec![s.item]);
                    }
                }
            }
        }
        // A piece no room would take is in the closet: not judged.
        Rule::InChat => {
            for s in layout {
                if room::closetable(keep.chat(), s) && !closet.contains(&s.item) {
                    push(vec![s.item], vec![s.item]);
                }
            }
        }
    }
}

/// Whether `piece` spoils `strip`'s room as `layout` has it: what the
/// room is without it forbids something it brings (the role table's
/// `forbids`; a den forbids nothing).
fn spoils(layout: &[Shown], strip: Strip, piece: Furniture) -> bool {
    let forbidden = room::forbids(room::role_without(layout, strip, piece));
    piece.spec().offers.iter().any(|o| forbidden.contains(o))
}

/// Every piece out of its box that spoils its room as `layout` has it,
/// and the room (whether a piece spoils a room hangs only on what's in
/// it, so the same pair before and after a move is the same spoiling).
fn spoilt(layout: &[Shown]) -> Vec<(Strip, Furniture)> {
    layout
        .iter()
        .filter(|s| !s.boxed && s.scrap.is_none())
        .filter_map(|s| Some((s.strip?, s.item)))
        .filter(|&(strip, item)| spoils(layout, strip, item))
        .collect()
}

/// Whether, her pieces laid out as `after` with one moved from `from`
/// to `to`, a piece spoils a room it didn't (`was`, [`spoilt`] before
/// the move): the moved piece the room it comes into, or another the
/// room it stands in as that one comes or goes. Only those two rooms
/// hold other pieces than before; a move along its own strip changes
/// none.
fn newly_spoilt(
    was: &[(Strip, Furniture)],
    after: &[Shown],
    from: Option<Strip>,
    to: Strip,
) -> bool {
    from != Some(to)
        && after
            .iter()
            .filter(|s| !s.boxed && s.scrap.is_none())
            .filter_map(|s| Some((s.strip?, s.item)))
            .filter(|&(strip, _)| strip == to || Some(strip) == from)
            .any(|pair| !was.contains(&pair) && spoils(after, pair.0, pair.1))
}

/// The cells between two pieces along a floor (0 when they touch).
fn between(a: &Shown, b: &Shown) -> i32 {
    let (ra, rb) = (a.rect(), b.rect());
    (i32::from(rb.x) - i32::from(ra.right())).max(i32::from(ra.x) - i32::from(rb.right()))
}

/// Whether `at` stands within [`WALL_GAP`] of one of `e`'s walls (its
/// floor's: beside her door's space is against the wall).
fn against_wall(at: &Shown, e: Extent) -> bool {
    let (cols, _) = at.size();
    at.left - e.from <= WALL_GAP || e.to - (at.left + i32::from(cols)) <= WALL_GAP
}

/// Where a piece would stand: on `strip`, `anchor` along it, turned
/// `facing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Placement {
    pub strip: Strip,
    pub anchor: Anchor,
    pub facing: Facing,
}

/// One of her pieces moved, or turned, so a broken rule holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Repair {
    /// The rule it mends.
    pub key: Grievance,
    /// The piece she moves.
    pub piece: Furniture,
    /// Where she sets it down.
    pub to: Placement,
    /// Where it would stand this frame.
    pub at: Shown,
    /// The cells her pieces would move (the piece and any it pushes
    /// along: across and up or down), and one for a turn.
    pub cost: u32,
    /// For a piece she hasn't settled: 0 a room it completes, 1 a room
    /// it doesn't spoil, 2 a strip with nothing on it. For one she has:
    /// 0, or [`SETTLED_BEHIND`] when the rule would move one she hasn't
    /// too and this isn't a turn where it stands.
    pub tier: u8,
}

impl Repair {
    /// A few words for the stage: what she'd move where.
    pub fn label(&self) -> String {
        let Strip::Bottom(nook) = self.to.strip;
        let side = match self.to.anchor.side {
            Side::Left => "L",
            Side::Right => "R",
        };
        let facing = match self.to.facing {
            Facing::Left => "<",
            Facing::Right => ">",
        };
        format!(
            "{} to {nook:?} {side}{} {facing} (tier {}, {} cells)",
            self.piece.spec().name,
            self.to.anchor.offset,
            self.tier,
            self.cost
        )
    }
}

/// What a repair search found: up to [`REPAIRS`], cheapest first, and
/// how many moves it weighed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Search {
    pub repairs: Vec<Repair>,
    pub examined: usize,
}

/// The tier of moving a piece she has settled when the rule would move
/// one she hasn't as well: after anything that one can do (she never
/// chose where a delivery stands; she did choose this). A turn where it
/// stands is exempt (tier 0): it's the cheapest repair of all.
pub(super) const SETTLED_BEHIND: u8 = 3;

/// The repairs a search keeps at most.
pub(super) const REPAIRS: usize = 3;
/// The moves a search weighs at most (the places along a very wide
/// strip are strided to keep within it).
pub(super) const CANDIDATES: usize = 2_000;

/// The frame a repair is judged on: the screen, her quiet panes, the
/// cells nothing of hers may cover, her pieces that show, the makeshift
/// pieces' footprints (which nothing moves onto), and what no piece may
/// be put on (the chat pane and her door's space: door batch, D3).
pub(super) struct Frame<'a> {
    pub buf: &'a Buffer,
    pub nooks: &'a [(Nook, Rect)],
    pub blocked: &'a dyn Fn(i32, i32) -> bool,
    pub shown: &'a [Shown],
    pub made: &'a [Rect],
    pub keep: &'a Keep,
}

impl Frame<'_> {
    /// Whether `(x, y)` is free for `at`, one of her pieces as `laid`
    /// has them: not blocked, not under a makeshift piece or another of
    /// hers it may not overlap (her window and a sofa covering its
    /// corner may: see [`Shown::may_overlap`]).
    fn clear(&self, laid: &[Shown], at: &Shown, x: i32, y: i32) -> bool {
        self.free(laid, at.item, Some(at), x, y)
    }

    /// Whether `(x, y)` is free for her, using `who` with her pieces where
    /// `laid` has them: as [`Frame::clear`], but her box overlaps no
    /// other piece of hers at all.
    fn room(&self, laid: &[Shown], who: Furniture, x: i32, y: i32) -> bool {
        self.free(laid, who, None, x, y)
    }

    fn free(&self, laid: &[Shown], who: Furniture, over: Option<&Shown>, x: i32, y: i32) -> bool {
        let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        let cell = (ux, uy).into();
        !(self.blocked)(x, y)
            && !self.made.iter().any(|r| r.contains(cell))
            && !laid.iter().any(|s| {
                s.item != who
                    && s.rect().contains(cell)
                    && !over.is_some_and(|at| at.may_overlap(s))
            })
    }
}

/// Her home as it stands before a repair.
struct Before {
    /// Anchored wherever its strip is here.
    home: Home,
    /// Each strip laid out alone, with its plan: a move lays out again
    /// only the strips it touches (see [`Home::relaid`]).
    strips: Vec<StripLaid>,
    laid: Vec<Shown>,
    broken: Vec<Broken>,
    /// Every strip with a piece out of its box on it, and its role.
    roles: Vec<(Strip, Role)>,
    /// Every piece that spoils its room, and the room.
    spoilt: Vec<(Strip, Furniture)>,
    /// How far each hung piece hangs from where its wall alone would hang
    /// it (see [`Home::laid_and_shifted`]).
    shifts: Vec<(Furniture, Option<u32>)>,
    /// Her pieces that show, and whether she'd fit to use each.
    showing: Vec<(Shown, bool)>,
    /// What the frame keeps pieces out of (the chat pane, for
    /// [`Rule::InChat`]: judged on every layout a move would make).
    keep: Keep,
    /// Her pieces in the closet (see [`closeted`]).
    closet: Vec<Furniture>,
}

impl Before {
    fn new(home: &Home, frame: &Frame) -> Self {
        let mut home = home.clone();
        let strips = home.strips_laid_out(frame.nooks);
        let out = home.relaid(&strips);
        let closet = closeted(&out, &home, frame.keep);
        let broken = broken_with(&out, &home, frame.keep, &closet);
        let LaidOut {
            shown: laid,
            shifts,
            plans,
        } = out;
        let roles = plans
            .iter()
            .filter(|p| has_room(&laid, p.strip))
            .map(|p| (p.strip, room::role_of(&laid, p.strip)))
            .collect();
        let spoilt = spoilt(&laid);
        let showing = frame
            .shown
            .iter()
            .filter(|s| s.scrap.is_none())
            .map(|s| {
                let clear = |x: i32, y: i32| frame.room(&laid, s.item, x, y);
                (*s, room::roomy(frame.buf, s, &clear))
            })
            .collect();
        Self {
            home,
            strips,
            laid,
            broken,
            roles,
            spoilt,
            shifts,
            showing,
            keep: frame.keep.clone(),
            closet,
        }
    }
}

/// Whether, her pieces laid out as `laid`, a room that was something
/// (in `roles`, as it was) is a den (one that's gone is no worse).
fn becomes_den(roles: &[(Strip, Role)], laid: &[Shown]) -> bool {
    roles.iter().any(|&(strip, role)| {
        role != Role::Den && has_room(laid, strip) && room::role_of(laid, strip) == Role::Den
    })
}

/// Whether `strip` has a piece out of its box on it (a room).
fn has_room(laid: &[Shown], strip: Strip) -> bool {
    laid.iter()
        .any(|s| s.strip == Some(strip) && !s.boxed && s.scrap.is_none())
}

/// `piece` set down at `to`: the repair of `key` it would be and her
/// pieces' layout then, if it passes the geometry (no frame needed):
/// `key` is broken, every piece laid out before still is, `key` holds,
/// no room that was something becomes a den, no piece spoils a room it
/// didn't (nor the piece, one she hasn't settled, the room it's set
/// down in), her door's space that was kept is kept, no rule that held
/// is broken, and no piece that showed goes into the chat's closet.
/// `scratch` is `before`'s home, and is left so.
fn evaluate(
    before: &Before,
    scratch: &mut Home,
    key: Grievance,
    piece: Furniture,
    to: Placement,
    plan: &StripPlan,
    again: bool,
) -> Option<(Repair, Vec<Shown>)> {
    // A rule that holds has nothing to mend (another move mended it,
    // or a resize did); unless she's trying the piece in another spot
    // (`again`): she mended it herself, setting it down where it stands.
    let mending = before.broken.iter().find(|b| b.key == key);
    if mending.is_none() && !again {
        return None;
    }
    let i = before.home.props.iter().position(|p| p.item == piece)?;
    let old = *before.home.props.get(i)?;
    if plan.strip != to.strip {
        return None;
    }
    let e = plan.along(old.lane());
    if old.boxed || (old.strip, old.anchor, old.facing) == (to.strip, Some(to.anchor), to.facing) {
        return None;
    }
    let cols = piece.spec().footprint.0;
    let moved = Prop {
        strip: to.strip,
        anchor: Some(to.anchor),
        at: plan.pin(old.lane(), e.left(to.anchor, cols), cols).1,
        facing: to.facing,
        ..old
    };
    let slot = scratch.props.get_mut(i)?;
    *slot = moved;
    let out = scratch.relaid(&before.strips);
    if let Some(slot) = scratch.props.get_mut(i) {
        *slot = old;
    }
    let (laid, shifts) = (&out.shown, &out.shifts);
    // Every strip that packed still packs: the same pieces laid out.
    if laid.len() != before.laid.len()
        || !before
            .laid
            .iter()
            .all(|b| laid.iter().any(|a| a.item == b.item))
    {
        return None;
    }
    let at = *laid.iter().find(|s| s.item == piece)?;
    // Nothing goes under a piece hung low that it may not overlap (her
    // window: phase 5c D6), nor is a hung piece set down where it can't
    // hang: no other hung piece hangs further from where its wall alone
    // would hang it than it did (one set down under it pushes it aside;
    // one moved off from under it may let it back), and the piece hangs
    // just where it's set down.
    if pushes_hung(&before.shifts, shifts, piece) {
        return None;
    }
    // It mends the rule (the settled flags are unchanged by a move). The
    // pieces in the closet are judged as they were: a piece in the chat
    // that wasn't in the closet is in the chat still, wherever a room
    // would take it (an InChat repair moves it out, or mends nothing).
    let mut target = Vec::new();
    judge(
        key.row,
        &out,
        &before.home,
        &before.keep,
        &before.closet,
        &mut target,
    );
    if target.iter().any(|b| b.key == key) {
        return None;
    }
    // No room is worse for it: none that was something becomes a den (a
    // room may become something else: a study a living room, when the
    // sofa joins a TV that stands with a desk). Nor does any piece spoil
    // a room it didn't: the piece the room it comes into, nor another
    // its own room when this one comes or goes (a fridge with a desk
    // spoilt by a bed joining them, as by its joining a bed and a desk).
    // And set down, a piece is settled, so one she hasn't settled may
    // not spoil the room it's set down in either.
    let was = before.laid.iter().find(|s| s.item == piece)?;
    if (!old.settled && spoils(laid, to.strip, piece))
        || newly_spoilt(&before.spoilt, laid, was.strip, to.strip)
    {
        return None;
    }
    if becomes_den(&before.roles, laid) {
        return None;
    }
    // Her door's space that was kept is kept: a move neither makes it
    // yield nor takes it away (a piece wider than it leaves room for),
    // which no rule would see (DoorClear is judged only on a space, and
    // only on what stands in it), as for a delivery ([`room::admits`]).
    let kept = |plans: &[StripPlan], strip: Strip| {
        plans
            .iter()
            .find(|p| p.strip == strip)
            .and_then(|p| p.space)
            .is_some_and(|s| s.kept)
    };
    if before.strips.iter().any(|b| {
        kept(std::slice::from_ref(&b.plan), b.plan.strip) && !kept(&out.plans, b.plan.strip)
    }) {
        return None;
    }
    // No rule that held is broken: judged with the closet as it was, so
    // a piece newly in the chat breaks InChat (none is put there: see
    // [`fits_now`]), and one that was in the closet isn't judged (a room
    // taking it now, it isn't newly in the chat: it was there).
    if broken_with(&out, &before.home, &before.keep, &before.closet)
        .iter()
        .any(|b| !before.broken.iter().any(|w| w.key == b.key))
    {
        return None;
    }
    // No piece that showed in the chat goes into the closet for it (it
    // would vanish for her moving something else). Judged last, and only
    // while one shows there: the closet is costly to weigh, and weighed
    // for the move's own layout (the piece where it's set down).
    let chat = before.keep.chat();
    if laid
        .iter()
        .any(|s| room::closetable(chat, s) && !before.closet.contains(&s.item))
    {
        let slot = scratch.props.get_mut(i)?;
        *slot = moved;
        let closet = closeted(&out, scratch, &before.keep);
        if let Some(slot) = scratch.props.get_mut(i) {
            *slot = old;
        }
        if closet.iter().any(|c| !before.closet.contains(c)) {
            return None;
        }
    }
    // Of a piece she has settled and one she hasn't, she moves the one
    // she never chose a place for, wherever it can go; but turning the
    // settled one round where it stands is the cheapest of all.
    let in_place = (at.strip, at.left, at.floor) == (was.strip, was.left, was.floor);
    let rival = mending.is_some_and(|m| {
        m.pieces
            .iter()
            .any(|&p| p != piece && before.home.props.iter().any(|q| q.item == p && !q.settled))
    });
    let tier = if old.settled {
        if rival && !in_place {
            SETTLED_BEHIND
        } else {
            0
        }
    } else if !laid
        .iter()
        .any(|s| s.strip == Some(to.strip) && s.item != piece)
    {
        2
    } else if room::role_without(laid, to.strip, piece) == Role::Den
        && room::role_of(laid, to.strip) != Role::Den
    {
        0
    } else {
        1
    };
    let shifted: u32 = laid
        .iter()
        .filter_map(|a| {
            let b = before.laid.iter().find(|b| b.item == a.item)?;
            ((a.strip, a.left, a.floor) != (b.strip, b.left, b.floor))
                .then(|| (a.left - b.left).unsigned_abs() + (a.floor - b.floor).unsigned_abs())
        })
        .sum();
    let cost = shifted + u32::from(to.facing != old.facing);
    let repair = Repair {
        key,
        piece,
        to,
        at,
        cost,
        tier,
    };
    Some((repair, out.shown))
}

/// The plan of `strip` as her home stands before the move: the grid
/// [`search`] lays its places along, and what [`evaluate`] pins a place
/// on. A move that changes it is refused: [`evaluate`] lays out the
/// real home and refuses one that makes her door's kept space yield or
/// go (a piece moved onto the strip can only crowd it), and a yielding
/// space or none lays the strip out raw either way. So the plan before
/// is the plan after for every move she could make. (It was the plan
/// with the piece moved onto the strip, anchored as it was, F7, until
/// her door's space came to be kept by the move rule above, in the door
/// batch's step 6 review; that one could only be wrong where the
/// piece's place decides whether the space is kept.)
fn plan_of(before: &Before, strip: Strip) -> Option<StripPlan> {
    before
        .strips
        .iter()
        .find(|s| s.plan.strip == strip)
        .map(|s| s.plan)
}

/// Whether a move of `piece` pushes a hung piece aside, its wall's
/// pieces `before` and `after` it hung as far from where their walls
/// alone would hang them as each list says (see [`Home::laid_and_shifted`]):
/// another further than it was (left out: furthest of all), or `piece`
/// itself at all.
fn pushes_hung(
    before: &[(Furniture, Option<u32>)],
    after: &[(Furniture, Option<u32>)],
    piece: Furniture,
) -> bool {
    let far = |shifts: &[(Furniture, Option<u32>)], item: Furniture| {
        shifts
            .iter()
            .find(|(i, _)| *i == item)
            .map_or(0, |(_, d)| d.unwrap_or(u32::MAX))
    };
    after
        .iter()
        .any(|&(item, _)| item == piece || far(after, item) > far(before, item))
}

/// Whether, her pieces laid out as `laid` with `piece` moved, `piece`
/// stands on blank, free cells where she'd fit to use it, and every
/// other piece that showed still fits (and she'd still fit to use those
/// she would before). None is put where `frame` keeps pieces out (the
/// chat pane, her door's space): `piece` unless it stays where it stood
/// (turned), and none pushed along newly into it. Judged here, after
/// the layout, once per candidate that passed: a strip's layout reads
/// only her pieces on it and her door (see [`Home::relaid`]).
fn fits_now(frame: &Frame, before: &Before, laid: &[Shown], piece: Furniture) -> bool {
    let fit = |at: &Shown| {
        let clear = |x: i32, y: i32| frame.clear(laid, at, x, y);
        room::fits(frame.buf, at, &clear)
    };
    let room = |at: &Shown| {
        let clear = |x: i32, y: i32| frame.room(laid, at.item, x, y);
        room::roomy(frame.buf, at, &clear)
    };
    let Some(at) = laid.iter().find(|s| s.item == piece) else {
        return false;
    };
    let stood = before.laid.iter().find(|s| s.item == piece);
    let put_in = frame.keep.refuses(at.cover()) && stood.is_none_or(|s| s.cover() != at.cover());
    !put_in
        && fit(at)
        && room(at)
        && before
            .showing
            .iter()
            .filter(|(was, _)| was.item != piece)
            .all(|(was, roomy)| {
                laid.iter().find(|s| s.item == was.item).is_some_and(|now| {
                    // Where it stood, it still fits: nothing moved onto it.
                    (now == was || (fit(now) && !frame.keep.takes_in(&[*was], &[*now])))
                        && (!roomy || room(now))
                })
            })
}

/// The pieces `piece` keeps company with under `rule` (to stand near or
/// face): a move to a strip without one of them can't mend it.
fn partners(rule: Rule, piece: Furniture) -> Vec<Furniture> {
    match rule {
        Rule::Faces { seat, screen } if seat == piece => vec![screen],
        Rule::Faces { seat, screen } if screen == piece => vec![seat],
        Rule::Near { a, b, .. } if a == piece => b.to_vec(),
        _ => Vec::new(),
    }
}

/// Where on one strip a search would set one piece down.
struct Places {
    /// The piece's place among the rule's pieces.
    order: usize,
    piece: Furniture,
    strip: Strip,
    /// The strip's plan with the piece on it.
    plan: StripPlan,
    /// Its floor there.
    e: Extent,
    /// Its leftmost place (against the far wall).
    hi: i32,
    /// Where it stands now, if on this strip.
    here: Option<i32>,
    /// The pieces it keeps company with there.
    partners: Vec<Shown>,
    /// The ways round it could be set down: either, where the rule
    /// judges which way it faces (a sofa, to watch the TV from); else
    /// only as it is (turning it would mend nothing, and only crowd out
    /// other places).
    facings: Vec<Facing>,
}

/// Up to [`REPAIRS`] ways to mend `target` on `frame`, cheapest first
/// (by tier, then cells, then the rule's order of pieces): each moves
/// one of the pieces it would move (or turns it) to a place on a strip
/// that passes the geometry ([`evaluate`]) and fits there this frame.
pub(super) fn search(home: &Home, frame: &Frame, target: &Broken) -> Search {
    let before = Before::new(home, frame);
    let Some(rule) = target.rule().map(|r| r.rule) else {
        return Search::default();
    };
    // Each piece it would move, each strip it could mend it on, and the
    // places along that strip (with where it stands now, and the far
    // wall).
    let mut places: Vec<Places> = Vec::new();
    for (order, &piece) in target.pieces.iter().enumerate() {
        let Some(now) = before.laid.iter().find(|s| s.item == piece) else {
            continue;
        };
        let Some(old) = before.home.props.iter().find(|q| q.item == piece) else {
            continue;
        };
        let flip = match old.facing {
            Facing::Left => Facing::Right,
            Facing::Right => Facing::Left,
        };
        let facings = match rule {
            Rule::Faces { seat, .. } if seat == piece => vec![old.facing, flip],
            _ => vec![old.facing],
        };
        let (cols, rows) = piece.spec().footprint;
        let with = partners(rule, piece);
        for strip in before.strips.iter().map(|s| s.plan.strip) {
            let Some(plan) = plan_of(&before, strip) else {
                continue;
            };
            let e = plan.along(old.lane());
            let beside = |s: &&Shown| with.contains(&s.item) && s.strip == Some(strip) && !s.boxed;
            let keeps_company = with.is_empty() || before.laid.iter().any(|s| beside(&s));
            // Apart: never on the strip the other one stands on.
            let apart_from = match rule {
                Rule::Apart { a, b } => before.laid.iter().any(|s| {
                    (s.item == a || s.item == b) && s.item != piece && s.strip == Some(strip)
                }),
                _ => false,
            };
            if e.holds((cols, rows)) && keeps_company && !apart_from {
                places.push(Places {
                    order,
                    piece,
                    strip,
                    plan,
                    e,
                    hi: e.to - i32::from(cols),
                    here: (now.strip == Some(strip)).then_some(now.left),
                    partners: before.laid.iter().filter(beside).copied().collect(),
                    facings: facings.clone(),
                });
            }
        }
    }
    let lefts = |stride: usize, p: &Places| {
        let mut out: Vec<i32> = (p.e.from..=p.hi).step_by(stride).collect();
        out.push(p.hi);
        out.extend(p.here);
        out.sort_unstable();
        out.dedup();
        out
    };
    let count = |stride: usize| {
        places
            .iter()
            .map(|p| p.facings.len() * lefts(stride, p).len())
            .sum::<usize>()
    };
    let mut stride = 1;
    while count(stride) > CANDIDATES {
        stride += 1;
    }
    let mut scratch = before.home.clone();
    let mut examined = 0;
    let mut passed: Vec<(u8, u32, usize, usize, Repair, Vec<Shown>)> = Vec::new();
    for p in &places {
        let Some(old) = before.home.props.iter().find(|q| q.item == p.piece) else {
            continue;
        };
        let cols = p.piece.spec().footprint.0;
        for left in lefts(stride, p) {
            // Where it stands now, it keeps its anchor (a turn); beside a
            // partner, it's anchored from that partner's wall, so the
            // two keep together across a resize; elsewhere, from the
            // nearer wall.
            let partner = p
                .partners
                .iter()
                .min_by_key(|s| (s.left - left).abs())
                .and_then(|s| before.home.props.iter().find(|q| q.item == s.item))
                .and_then(|q| q.anchor);
            let anchor = match (p.here == Some(left), old.anchor, partner) {
                (true, Some(anchor), _) => anchor,
                (_, _, Some(Anchor { side, .. })) => Anchor {
                    side,
                    offset: match side {
                        Side::Left => left - p.e.from,
                        Side::Right => p.e.to - left - i32::from(cols),
                    }
                    .clamp(0, i32::from(u16::MAX)) as u16,
                },
                _ => p.e.pin(left, cols).0,
            };
            for &facing in &p.facings {
                examined += 1;
                let to = Placement {
                    strip: p.strip,
                    anchor,
                    facing,
                };
                let Some((repair, laid)) = evaluate(
                    &before,
                    &mut scratch,
                    target.key,
                    p.piece,
                    to,
                    &p.plan,
                    false,
                ) else {
                    continue;
                };
                // Pushed along by its neighbours, it would stand where
                // another place puts it: that one is weighed there.
                let same = |(.., r, _): &(u8, u32, usize, usize, Repair, Vec<Shown>)| {
                    r.piece == repair.piece && r.at == repair.at && r.to == repair.to
                };
                if repair.at.left == left && !passed.iter().any(same) {
                    let generated = passed.len();
                    passed.push((repair.tier, repair.cost, p.order, generated, repair, laid));
                }
            }
        }
    }
    passed.sort_by_key(|&(tier, cost, order, generated, ..)| (tier, cost, order, generated));
    let repairs = passed
        .into_iter()
        .filter(|(.., repair, laid)| fits_now(frame, &before, laid, repair.piece))
        .map(|(.., repair, _)| repair)
        .take(REPAIRS)
        .collect();
    Search { repairs, examined }
}

/// Where `repair`'s piece would stand on `frame`, if the move still
/// mends its rule there as [`search`] would judge it (where it would
/// stand may have moved with a resize). Judged with the move applied:
/// the piece's own place, which it keeps until set down, is free.
pub(super) fn check(home: &Home, frame: &Frame, repair: &Repair) -> Option<Shown> {
    judge_move(home, frame, repair, false)
}

/// [`check`], for a spot she'd try a piece in that she has already set
/// down where it mends `repair`'s rule: the rule may hold now (by her
/// own doing), and must still hold with the piece moved, breaking none
/// that holds.
pub(super) fn check_again(home: &Home, frame: &Frame, repair: &Repair) -> Option<Shown> {
    judge_move(home, frame, repair, true)
}

fn judge_move(home: &Home, frame: &Frame, repair: &Repair, again: bool) -> Option<Shown> {
    let before = Before::new(home, frame);
    let mut scratch = before.home.clone();
    let plan = plan_of(&before, repair.to.strip)?;
    let (moved, laid) = evaluate(
        &before,
        &mut scratch,
        repair.key,
        repair.piece,
        repair.to,
        &plan,
        again,
    )?;
    fits_now(frame, &before, &laid, repair.piece).then_some(moved.at)
}

/// Repairs this many cells dearer than the cheapest (at its tier, of
/// its piece) are as good to her: she may try the piece in each.
pub(super) const TIE_CELLS: u32 = 4;

/// The spots she'd try a piece in after the one she sets it down in
/// first, in order, and the cost of the cheapest of them all (what each
/// is weighed against). Empty when there's only the one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Trials {
    rest: [Option<Spot>; REPAIRS - 1],
    pub min: u32,
}

/// One more spot to try a piece in: where it's set down, what that
/// costs, and where it would stand (all a [`Repair`] of the same piece
/// differs by).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Spot {
    to: Placement,
    cost: u32,
    left: i32,
    floor: i32,
}

impl Trials {
    /// Of `repairs` (a search's, cheapest first), those as good as the
    /// first: of its piece, at its tier, within [`TIE_CELLS`] of it; one
    /// a spot (the cheaper way round: a piece turned where another
    /// repair sets it isn't another spot to try).
    pub fn ties(repairs: &[Repair]) -> Vec<Repair> {
        let Some(first) = repairs.first() else {
            return Vec::new();
        };
        let mut ties: Vec<Repair> = Vec::new();
        for r in repairs {
            let spot = |t: &Repair| (t.at.strip, t.at.left, t.at.floor);
            if (r.piece, r.tier) == (first.piece, first.tier)
                && r.cost <= first.cost.saturating_add(TIE_CELLS)
                && !ties.iter().any(|t| spot(t) == spot(r))
            {
                ties.push(*r);
            }
        }
        ties
    }

    /// The spots after the first of `ties` (in the order she'd try
    /// them), as many as she'd try.
    pub fn of(ties: &[Repair]) -> Self {
        let mut trials = Self {
            min: ties.iter().map(|r| r.cost).min().unwrap_or(0),
            ..Self::default()
        };
        for (slot, r) in trials.rest.iter_mut().zip(ties.iter().skip(1)) {
            *slot = Some(Spot {
                to: r.to,
                cost: r.cost,
                left: r.at.left,
                floor: r.at.floor,
            });
        }
        trials
    }

    /// Whether a spot is left to try.
    pub fn remain(&self) -> bool {
        self.rest.iter().any(Option::is_some)
    }

    /// The next spot to try, if one is left: `like` (a repair of the same
    /// rule, by the same piece) set down there.
    pub fn next(&mut self, like: &Repair) -> Option<Repair> {
        let spot = self.rest.iter_mut().find_map(Option::take)?;
        Some(Repair {
            to: spot.to,
            cost: spot.cost,
            at: Shown {
                strip: Some(spot.to.strip),
                facing: spot.to.facing,
                left: spot.left,
                floor: spot.floor,
                ..like.at
            },
            ..*like
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::room::{Anchor, DoorWall, Nook, Prop, Side, raw_strips};
    use super::super::sprite::Facing;
    use super::*;
    use Furniture::*;
    use tuirealm::ratatui::buffer::Buffer;
    use tuirealm::ratatui::layout::Rect;

    /// A piece anchored `offset` cells from `side`'s wall of `nook`.
    fn at(item: Furniture, nook: Nook, side: Side, offset: u16, facing: Facing) -> Prop {
        Prop {
            anchor: Some(Anchor { side, offset }),
            ..Prop::new(item, nook, 0, facing)
        }
    }

    fn unsettled(prop: Prop) -> Prop {
        Prop {
            settled: false,
            ..prop
        }
    }

    fn boxed(prop: Prop) -> Prop {
        Prop {
            boxed: true,
            ..prop
        }
    }

    /// The Users pane above the Playlist pane, each `width` wide and
    /// eight tall.
    fn panes(width: u16) -> [(Nook, Rect); 2] {
        [
            (Nook::Users, Rect::new(0, 0, width, 8)),
            (Nook::Playlist, Rect::new(0, 8, width, 8)),
        ]
    }

    /// The rules `props` break laid out on [`panes`] `width` wide, as
    /// `(rule's row, the pieces it would move)`.
    fn judge(width: u16, props: &[Prop]) -> Vec<(usize, Vec<Furniture>)> {
        let mut home = Home::default();
        for &p in props {
            assert!(home.add(p));
        }
        let nooks = panes(width);
        let laid = home.laid_out(&nooks);
        broken(&laid, &home, &Keep::default())
            .into_iter()
            .map(|b| (b.row, b.pieces))
            .collect()
    }

    fn row(rule: Rule) -> usize {
        RULES.iter().position(|r| r.rule == rule).unwrap()
    }

    const FACES: Rule = Rule::Faces {
        seat: Sofa,
        screen: Tv,
    };
    const APART: Rule = Rule::Apart { a: Bed, b: Tv };
    const INCHAT: Rule = Rule::InChat;

    #[test]
    fn the_faces_row_is_the_sofa_and_the_tv() {
        assert_eq!(RULES[FACES_ROW].rule, FACES);
    }

    #[test]
    fn a_sofa_turned_toward_the_tv_faces_it() {
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        // The TV against the right wall of a 40-wide pane (columns
        // 33..39); the sofa 4 cells left of it.
        let sofa = |facing| at(Sofa, Nook::Users, Side::Right, 6 + 4, facing);
        assert_eq!(judge(40, &[tv, sofa(Facing::Right)]), []);
        assert_eq!(
            judge(40, &[tv, sofa(Facing::Left)]),
            [(row(FACES), vec![Sofa, Tv])],
            "turned away"
        );
        // Too close, too far, on another strip.
        let near = at(Sofa, Nook::Users, Side::Right, 6 + 1, Facing::Right);
        let far = at(Sofa, Nook::Users, Side::Left, 0, Facing::Right);
        let other = at(Sofa, Nook::Playlist, Side::Right, 10, Facing::Right);
        for sofa in [near, far, other] {
            assert_eq!(
                judge(40, &[tv, sofa]),
                [(row(FACES), vec![Sofa, Tv])],
                "{sofa:?}"
            );
        }
        // A rule needs every piece it names, out of its box.
        assert_eq!(judge(40, &[sofa(Facing::Left)]), []);
        assert_eq!(judge(40, &[boxed(tv), sofa(Facing::Left)]), []);
        // Nor does a strip too small to lay them out judge them.
        assert_eq!(judge(14, &[tv, sofa(Facing::Left)]), []);
    }

    #[test]
    fn the_lamp_stands_by_the_bed_or_the_desk() {
        let near = row(Rule::Near {
            a: Lamp,
            b: &[Bed, Desk],
            gap: 3,
        });
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let lamp = |offset| at(Lamp, Nook::Users, Side::Left, offset, Facing::Right);
        // The bed's columns 1..11; the lamp 0 and 3 cells past it.
        assert_eq!(judge(40, &[bed, lamp(10)]), []);
        assert_eq!(judge(40, &[bed, lamp(13)]), []);
        assert_eq!(judge(40, &[bed, lamp(14)]), [(near, vec![Lamp])]);
        // A desk will do as well, on another strip; a lamp alone is no
        // matter.
        let desk = at(Desk, Nook::Playlist, Side::Right, 0, Facing::Left);
        let by_desk = at(Lamp, Nook::Playlist, Side::Right, 9, Facing::Right);
        assert_eq!(judge(40, &[bed, desk, by_desk]), []);
        assert_eq!(judge(40, &[lamp(14)]), []);
        // Each partner it could stand by is involved.
        let mut home = Home::default();
        for p in [bed, desk, lamp(30)] {
            assert!(home.add(p));
        }
        let nooks = panes(60);
        let laid = home.laid_out(&nooks);
        let got = broken(&laid, &home, &Keep::default());
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].involved, [Lamp, Bed, Desk]);
    }

    #[test]
    fn the_fridge_and_the_bookshelf_want_a_wall() {
        for item in [Fridge, Bookshelf] {
            let rule = row(Rule::AgainstWall(item));
            for (side, offset, ok) in [
                (Side::Left, 0, true),
                (Side::Left, 1, true),
                (Side::Left, 2, false),
                (Side::Right, 1, true),
                (Side::Right, 5, false),
            ] {
                let want = if ok { vec![] } else { vec![(rule, vec![item])] };
                assert_eq!(
                    judge(40, &[at(item, Nook::Users, side, offset, Facing::Right)]),
                    want,
                    "{item:?} {side:?} {offset}"
                );
            }
        }
        // Pushed off its wall by a piece packed against it.
        // (Of two the same distance from a wall, the newer is nearer it.)
        let fridge = at(Fridge, Nook::Users, Side::Left, 0, Facing::Right);
        let sofa = at(Sofa, Nook::Users, Side::Left, 0, Facing::Right);
        assert_eq!(
            judge(40, &[fridge, sofa]),
            [(row(Rule::AgainstWall(Fridge)), vec![Fridge])]
        );
        // Beside her door's space, it's against the wall: the space is
        // the wall's (the door batch, step 2). Her door in Users' left
        // wall, the fridge and the bookshelf anchored from it stand 6
        // columns along, and want nothing; a column further, the same.
        for item in [Fridge, Bookshelf] {
            for (offset, ok) in [(0, true), (1, true), (2, false)] {
                let mut home = Home {
                    door: Some(DoorWall {
                        strip: Strip::Bottom(Nook::Users),
                        side: Side::Left,
                    }),
                    ..Home::default()
                };
                assert!(home.add(at(item, Nook::Users, Side::Left, offset, Facing::Right)));
                let laid = home.laid_out(&panes(40));
                let here = laid.shown.iter().find(|s| s.item == item).unwrap();
                assert_eq!(here.left, 1 + 6 + i32::from(offset), "{item:?} {offset}");
                let want = if ok {
                    vec![]
                } else {
                    vec![row(Rule::AgainstWall(item))]
                };
                let got: Vec<usize> = broken(&laid, &home, &Keep::default())
                    .iter()
                    .map(|b| b.row)
                    .collect();
                assert_eq!(got, want, "{item:?} {offset}");
            }
        }
    }

    #[test]
    fn the_bed_and_the_tv_are_apart() {
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let elsewhere = at(Tv, Nook::Playlist, Side::Right, 0, Facing::Left);
        assert_eq!(judge(40, &[bed, tv]), [(row(APART), vec![Bed, Tv])]);
        assert_eq!(judge(40, &[bed, elsewhere]), []);
        // The one she hasn't settled first.
        assert_eq!(
            judge(40, &[bed, unsettled(tv)]),
            [(row(APART), vec![Tv, Bed]), (row(Rule::Belongs), vec![Tv]),]
        );
        assert_eq!(judge(40, &[boxed(bed), tv]), []);
    }

    #[test]
    fn a_piece_she_has_not_settled_belongs_where_it_does_not_spoil_the_room() {
        let belongs = row(Rule::Belongs);
        let sofa = at(Sofa, Nook::Users, Side::Left, 0, Facing::Right);
        let tv = at(Tv, Nook::Users, Side::Left, 14, Facing::Left);
        let bed = at(Bed, Nook::Users, Side::Right, 0, Facing::Left);
        let in_bedroom = |item| at(item, Nook::Users, Side::Left, 0, Facing::Right);
        // A bed in the living room: settled, it's only too noisy.
        assert_eq!(judge(60, &[sofa, tv, bed]), [(row(APART), vec![Bed, Tv])]);
        assert_eq!(
            judge(60, &[sofa, tv, unsettled(bed)]),
            [(row(APART), vec![Bed, Tv]), (belongs, vec![Bed])]
        );
        // A fridge in a bedroom spoils it; a desk doesn't.
        assert_eq!(
            judge(60, &[bed, unsettled(in_bedroom(Fridge))]),
            [(belongs, vec![Fridge])]
        );
        assert_eq!(judge(60, &[bed, unsettled(in_bedroom(Desk))]), []);
        // A bed spoils a kitchen; a desk doesn't (a kitchen forbids only
        // a bed, though a fridge and a desk read as a study), nor a
        // fridge a study (a study forbids nothing).
        let fridge = at(Fridge, Nook::Users, Side::Right, 0, Facing::Left);
        let desk = at(Desk, Nook::Users, Side::Right, 0, Facing::Left);
        assert_eq!(
            judge(60, &[fridge, unsettled(in_bedroom(Bed))]),
            [(belongs, vec![Bed])]
        );
        assert_eq!(judge(60, &[fridge, unsettled(in_bedroom(Desk))]), []);
        assert_eq!(judge(60, &[desk, unsettled(in_bedroom(Fridge))]), []);
        // A TV that makes a living room of a sofa's room completes it.
        assert_eq!(judge(60, &[sofa, unsettled(tv)]), []);
        // Alone on its strip, or still boxed, it spoils nothing.
        assert_eq!(judge(60, &[unsettled(bed)]), []);
        assert_eq!(judge(60, &[sofa, tv, boxed(unsettled(bed))]), []);
        // Only her pieces on the same strip make the room.
        let away = at(Bed, Nook::Playlist, Side::Left, 0, Facing::Right);
        assert_eq!(judge(60, &[sofa, tv, unsettled(away)]), []);
    }

    /// Belongs is felt on every use of a piece once it's out of its box.
    #[test]
    fn belongs_is_felt_on_every_use() {
        let all = [
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
        ];
        for what in all {
            // Exhaustive: a new use is placed here, then in ANY_USE.
            let real = match what {
                Use::Lounge
                | Use::Nap
                | Use::Sleep
                | Use::Homework
                | Use::Watch
                | Use::Read
                | Use::Snack
                | Use::Pet
                | Use::LookOut => true,
                Use::Unpack | Use::Crumple => false,
            };
            assert_eq!(ANY_USE.contains(&what), real, "{what:?}");
        }
        for item in Furniture::ALL {
            for what in item.spec().uses {
                assert!(ANY_USE.contains(what), "{item:?} {what:?}");
            }
        }
    }

    use proptest::prelude::*;

    /// Some of her pieces, each anchored on one of the two strips, either
    /// way round, settled or not, boxed or not.
    fn pieces() -> impl Strategy<Value = Vec<Prop>> {
        proptest::sample::subsequence(Furniture::ALL.to_vec(), 1..=6).prop_flat_map(|items| {
            let n = items.len();
            (
                Just(items),
                proptest::collection::vec(
                    (
                        any::<bool>(),
                        any::<bool>(),
                        0u16..30,
                        any::<bool>(),
                        any::<bool>(),
                        proptest::bool::weighted(0.15),
                    ),
                    n,
                ),
            )
                .prop_map(|(items, places)| {
                    items
                        .into_iter()
                        .zip(places)
                        .map(|(item, (lower, right, offset, left, settled, boxed))| {
                            let nook = if lower { Nook::Playlist } else { Nook::Users };
                            let side = if right { Side::Right } else { Side::Left };
                            let facing = if left { Facing::Left } else { Facing::Right };
                            Prop {
                                settled,
                                boxed,
                                ..at(item, nook, side, offset, facing)
                            }
                        })
                        .collect()
                })
        })
    }

    /// [`panes`] `width` wide, with text cells at `text` along the row
    /// above each floor.
    fn frame(width: u16, text: &[u16]) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, 16));
        for (_, rect) in panes(width) {
            let w = usize::from(width);
            let rows: Vec<String> = std::iter::once(format!("┌{}┐", "─".repeat(w - 2)))
                .chain((0..6).map(|_| format!("│{}│", " ".repeat(w - 2))))
                .chain(std::iter::once(format!("└{}┘", "─".repeat(w - 2))))
                .collect();
            for (dy, line) in rows.iter().enumerate() {
                buf.set_string(
                    rect.x,
                    rect.y + dy as u16,
                    line,
                    tuirealm::ratatui::style::Style::default(),
                );
            }
            for &x in text {
                if x > 0 && x + 1 < width {
                    buf[(x, rect.bottom() - 2)].set_symbol("x");
                }
            }
        }
        buf
    }

    fn judged(home: &mut Home, nooks: &[(Nook, Rect)]) -> Vec<Broken> {
        let laid = home.laid_out(nooks);
        broken(&laid, home, &Keep::default())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        /// A resize and back breaks and mends nothing; text over a piece
        /// (which closets it) neither; and what's broken names only
        /// pieces laid out, out of their boxes.
        #[test]
        fn rules_hold_across_a_resize_and_text(
            props in pieces(),
            wide in 30u16..90,
            narrow in 16u16..90,
            text in proptest::collection::vec(1u16..90, 0..6),
        ) {
            rules_hold(props, wide, narrow, text, None)?;
        }

        /// [`rules_hold_across_a_resize_and_text`] with her door in a wall
        /// of either strip (the door batch, step 2): her door's space
        /// breaks and mends nothing across a resize and text either.
        #[test]
        fn rules_hold_across_a_resize_and_text_with_her_door(
            props in pieces(),
            wide in 30u16..90,
            narrow in 16u16..90,
            text in proptest::collection::vec(1u16..90, 0..6),
            on_users in any::<bool>(),
            right in any::<bool>(),
        ) {
            let door = DoorWall {
                strip: Strip::Bottom(if on_users { Nook::Users } else { Nook::Playlist }),
                side: if right { Side::Right } else { Side::Left },
            };
            rules_hold(props, wide, narrow, text, Some(door))?;
        }
    }

    /// The body of [`rules_hold_across_a_resize_and_text`], her door in
    /// `door`'s wall, if any.
    fn rules_hold(
        props: Vec<Prop>,
        wide: u16,
        narrow: u16,
        text: Vec<u16>,
        door: Option<DoorWall>,
    ) -> Result<(), TestCaseError> {
        let mut home = Home {
            door,
            ..Home::default()
        };
        for p in props {
            prop_assert!(home.add(p));
        }
        let (big, small) = (panes(wide), panes(narrow));
        let clean = frame(wide, &[]);
        let _ = home.project(&clean, &big, &|_, _| false);
        let before = home.clone();
        let first = judged(&mut home, &big);
        let laid = home.layout(&big);
        for b in &first {
            for piece in b.involved.iter().chain(&b.pieces) {
                prop_assert!(
                    laid.iter().any(|s| s.item == *piece && !s.boxed),
                    "{:?} in {:?}",
                    piece,
                    b
                );
            }
            prop_assert!(b.pieces.iter().all(|p| b.involved.contains(p)), "{:?}", b);
        }
        // A resize, and back. Where every piece that stands is laid
        // out at the narrower size too (no strip too small for its
        // pieces), nothing moves, and back at the wider size the
        // same rules are broken; otherwise a strip's pieces moved to
        // another floor that holds them, or nothing moved.
        let standing = |at: &[Shown]| -> Vec<usize> {
            let mut v: Vec<usize> = at
                .iter()
                .filter(|s| s.lane() == room::Lane::Floor)
                .map(|s| s.item as usize)
                .collect();
            v.sort_unstable();
            v
        };
        let narrow_laid = before.clone().layout(&small);
        let holds = standing(&narrow_laid) == standing(&laid);
        let _ = home.project(&frame(narrow, &[]), &small, &|_, _| false);
        if holds {
            prop_assert_eq!(&home, &before, "a resize that holds them moved a piece");
            let _ = home.project(&clean, &big, &|_, _| false);
            prop_assert_eq!(&judged(&mut home, &big), &first);
        } else if home != before {
            let moved = home.clone().layout(&small);
            prop_assert!(
                standing(&moved).len() > standing(&narrow_laid).len(),
                "moved, and no more laid out: {:?} / {:?}",
                moved,
                narrow_laid
            );
        }
        // Text closets what it covers; the rules don't see it.
        let mut home = before;
        let _ = home.project(&frame(wide, &text), &big, &|_, _| false);
        prop_assert_eq!(&judged(&mut home, &big), &first);
        Ok(())
    }

    // The repair search.

    /// Bordered panes at `nooks` on a `width × height` screen, with text
    /// at `text`.
    fn screen(nooks: &[(Nook, Rect)], width: u16, height: u16, text: &[(u16, u16)]) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        for &(_, rect) in nooks {
            tuirealm::ratatui::widgets::Widget::render(
                tuirealm::ratatui::widgets::Block::bordered(),
                rect,
                &mut buf,
            );
        }
        for &(x, y) in text {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol("x");
            }
        }
        buf
    }

    /// Her home of `props` on `nooks` over `buf`, projected; what shows,
    /// and how `rule` is broken (it must be).
    fn broken_at(
        props: &[Prop],
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        rule: Rule,
    ) -> (Home, Vec<Shown>, Broken) {
        let mut home = Home::default();
        for &p in props {
            assert!(home.add(p));
        }
        let shown = home.project(buf, nooks, &|_, _| false);
        let broken = judged(&mut home, nooks)
            .into_iter()
            .find(|b| b.row == row(rule))
            .unwrap_or_else(|| panic!("{rule:?} isn't broken"));
        (home, shown, broken)
    }

    /// The repairs of `rule`, broken by `props` on [`panes`] `width`
    /// wide with text at `text` along the row above each floor.
    fn mend(width: u16, props: &[Prop], text: &[u16], rule: Rule) -> (Home, Vec<Repair>) {
        mend_on(&frame(width, text), width, props, rule)
    }

    /// [`mend`] over `buf`, [`panes`] `width` wide.
    fn mend_on(buf: &Buffer, width: u16, props: &[Prop], rule: Rule) -> (Home, Vec<Repair>) {
        let nooks = panes(width);
        let buf = buf.clone();
        let (home, shown, broken) = broken_at(props, &buf, &nooks, rule);
        let frame = Frame {
            buf: &buf,
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &shown,
            made: &[],
            keep: &Keep::default(),
        };
        let found = search(&home, &frame, &broken);
        assert!(found.examined <= CANDIDATES, "{}", found.examined);
        for r in &found.repairs {
            assert!(check(&home, &frame, r).is_some(), "{r:?}");
        }
        (home, found.repairs)
    }

    /// `home` with `repair` made, laid out on `nooks`.
    fn made(home: &Home, repair: &Repair, nooks: &[(Nook, Rect)]) -> Vec<Shown> {
        made_home(home, repair).layout(nooks)
    }

    /// `home` with `repair` made.
    fn made_home(home: &Home, repair: &Repair) -> Home {
        let mut after = home.clone();
        let prop = after
            .props
            .iter_mut()
            .find(|p| p.item == repair.piece)
            .unwrap();
        prop.strip = repair.to.strip;
        prop.anchor = Some(repair.to.anchor);
        prop.facing = repair.to.facing;
        after
    }

    fn laid(at: &[Shown], item: Furniture) -> Shown {
        *at.iter().find(|s| s.item == item).unwrap()
    }

    /// A sofa turned away from the TV it's in range of is turned round,
    /// where it stands: one cell's worth, its anchor kept. Also in a
    /// study the sofa makes a living room of (it's settled there: the
    /// room is no worse for a turn).
    #[test]
    fn a_sofa_turned_away_is_turned_round() {
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let sofa = at(Sofa, Nook::Users, Side::Right, 10, Facing::Left);
        let desk = at(Desk, Nook::Users, Side::Left, 0, Facing::Right);
        for (width, props) in [(40, vec![tv, sofa]), (60, vec![tv, sofa, desk])] {
            let (_, repairs) = mend(width, &props, &[], FACES);
            let best = repairs.first().unwrap_or_else(|| panic!("{props:?}"));
            assert_eq!(best.piece, Sofa);
            assert_eq!(best.cost, 1, "{best:?}");
            assert_eq!(best.tier, 0);
            assert_eq!(best.to.anchor, sofa.anchor.unwrap(), "same anchor");
            assert_eq!(best.to.facing, Facing::Right);
            assert_eq!(best.to.strip, sofa.strip);
        }
    }

    /// A TV delivered to a kitchen goes to the sofa's strip, where she
    /// can watch it from the sofa, anchored from the sofa's wall (the
    /// sofa she has settled could go to it, making the kitchen a living
    /// room, but only behind every move of the TV; see
    /// `a_delivered_tv_comes_to_the_sofa` for the plain case).
    #[test]
    fn an_unsettled_tv_joins_the_sofa() {
        let sofa = at(Sofa, Nook::Users, Side::Right, 0, Facing::Left);
        let fridge = at(Fridge, Nook::Playlist, Side::Right, 0, Facing::Left);
        let tv = unsettled(at(Tv, Nook::Playlist, Side::Left, 0, Facing::Right));
        let (home, repairs) = mend(40, &[sofa, fridge, tv], &[], FACES);
        assert!(!repairs.is_empty());
        for r in &repairs {
            assert_eq!((r.piece, r.to.strip, r.tier), (Tv, sofa.strip, 0), "{r:?}");
            assert_eq!(r.to.anchor.side, Side::Right, "from the sofa's wall");
            let after = made(&home, r, &panes(40));
            assert!(room::faces(&laid(&after, Sofa), &laid(&after, Tv)));
        }
        let costs: Vec<u32> = repairs.iter().map(|r| r.cost).collect();
        assert!(costs.is_sorted(), "{costs:?}");
    }

    /// A sofa in the bedroom joins the TV that stands with a desk, and
    /// the study becomes a living room (a room changing what it is isn't
    /// worse for it, so long as it's something; the TV can't come to the
    /// sofa, where a bed and a TV make a den).
    #[test]
    fn a_sofa_joins_a_tv_that_stands_with_a_desk() {
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let desk = at(Desk, Nook::Users, Side::Left, 0, Facing::Right);
        let bed = at(Bed, Nook::Playlist, Side::Left, 0, Facing::Right);
        let sofa = at(Sofa, Nook::Playlist, Side::Right, 0, Facing::Left);
        let (home, repairs) = mend(60, &[tv, desk, bed, sofa], &[], FACES);
        let best = repairs.first().expect("a repair");
        assert_eq!((best.piece, best.to.strip), (Sofa, tv.strip), "{best:?}");
        let after = made(&home, best, &panes(60));
        assert!(room::faces(&laid(&after, Sofa), &laid(&after, Tv)));
        assert_eq!(room::role_of(&after, tv.strip), Role::Living);
        assert_eq!(room::role_of(&after, bed.strip), Role::Bedroom);
    }

    /// A fridge she has settled in the middle of the floor, with text
    /// along both walls of its room (and its neighbours' cells clear):
    /// the other room's wall is the only place for it.
    fn fridge_walled_in(width: u16, with: &[Prop], downstairs: &[Prop]) -> Vec<Repair> {
        let fridge = at(Fridge, Nook::Users, Side::Left, 15, Facing::Right);
        let mut buf = frame(width, &[]);
        let mut clear: Vec<(i32, i32)> = vec![(16, 20)];
        if with.iter().any(|p| p.item == CatBed) {
            clear.push((1, 5));
        }
        for x in 1..width - 1 {
            let x32 = i32::from(x);
            if !clear.iter().any(|&(a, b)| (a..b).contains(&x32)) {
                buf[(x, 6)].set_symbol("x");
            }
        }
        let mut props = vec![fridge];
        props.extend_from_slice(with);
        props.extend_from_slice(downstairs);
        let (_, repairs) = mend_on(&buf, width, &props, Rule::AgainstWall(Fridge));
        repairs
    }

    /// A move may leave a room something else, but never a den: a
    /// fridge leaving its kitchen with a cat bed in it leaves a den, so
    /// it stays (alone, it goes downstairs). Nor may a piece she has
    /// settled spoil the room it comes into: a fridge in a bedroom with
    /// a desk in it (a study, then, but spoilt), though into a study
    /// it may go.
    #[test]
    fn a_move_may_leave_no_den_and_spoil_no_room() {
        let downstairs = Strip::Bottom(Nook::Playlist);
        let alone = fridge_walled_in(40, &[], &[]);
        assert!(
            alone.iter().all(|r| r.to.strip == downstairs) && !alone.is_empty(),
            "{alone:?}"
        );
        let cat = at(CatBed, Nook::Users, Side::Left, 0, Facing::Right);
        assert_eq!(fridge_walled_in(40, &[cat], &[]), []);
        let bed = at(Bed, Nook::Playlist, Side::Left, 0, Facing::Right);
        let desk = at(Desk, Nook::Playlist, Side::Left, 11, Facing::Right);
        assert_eq!(fridge_walled_in(40, &[], &[bed, desk]), []);
        let study = fridge_walled_in(40, &[], &[desk]);
        assert!(
            study.iter().all(|r| r.to.strip == downstairs) && !study.is_empty(),
            "{study:?}"
        );
    }

    /// Nor may a move leave a piece she has settled spoiling its room
    /// where it didn't: the bed may not join a fridge and a desk (the
    /// same room as the fridge joining a bed and a desk), nor the TV
    /// leave a bed, a fridge and a desk (whose fridge, the TV gone,
    /// stands in a bedroom). The other piece goes instead.
    #[test]
    fn a_move_leaves_no_settled_piece_spoiling_its_room() {
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let fridge = at(Fridge, Nook::Playlist, Side::Left, 0, Facing::Right);
        let desk = at(Desk, Nook::Playlist, Side::Right, 0, Facing::Right);
        let joins = [tv, bed, fridge, desk];
        let fridge_up = at(Fridge, Nook::Users, Side::Left, 12, Facing::Right);
        let desk_up = at(Desk, Nook::Users, Side::Right, 8, Facing::Right);
        let leaves = [tv, bed, fridge_up, desk_up];
        for (props, piece) in [(&joins[..], Bed), (&leaves[..], Tv)] {
            let (home, repairs) = mend(60, props, &[], APART);
            assert!(!repairs.is_empty(), "{props:?}");
            for r in &repairs {
                assert_ne!(r.piece, piece, "{r:?}");
                let after = made(&home, r, &panes(60));
                let at = laid(&after, Fridge);
                assert!(!spoils(&after, at.strip.unwrap(), Fridge), "{r:?}");
            }
        }
        // As laid out before, the fridge doesn't spoil its room.
        for props in [joins, leaves] {
            let mut home = Home::default();
            for p in props {
                assert!(home.add(p));
            }
            let laid = home.layout(&panes(60));
            assert_eq!(laid.len(), props.len());
            assert!(spoilt(&laid).iter().all(|&(_, p)| p != Fridge), "{laid:?}");
        }
    }

    /// Only a rule that looks at which way a piece faces turns it: the
    /// sofa (to face the TV) may be set down either way round; the lamp,
    /// the fridge and a TV that joins the sofa (seen from the front) keep
    /// theirs, so their repairs are each another place, not the same
    /// place turned.
    #[test]
    fn a_piece_is_turned_only_where_its_rule_looks_at_that() {
        let near = Rule::Near {
            a: Lamp,
            b: &[Bed, Desk],
            gap: 3,
        };
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let lamp = at(Lamp, Nook::Users, Side::Right, 0, Facing::Right);
        let fridge = at(Fridge, Nook::Users, Side::Left, 15, Facing::Right);
        let sofa = at(Sofa, Nook::Users, Side::Right, 0, Facing::Left);
        let tv = unsettled(at(Tv, Nook::Playlist, Side::Left, 0, Facing::Right));
        for (props, rule, piece) in [
            (vec![bed, lamp], near, Lamp),
            (vec![fridge], Rule::AgainstWall(Fridge), Fridge),
            (vec![sofa, tv], FACES, Tv),
        ] {
            let (_, repairs) = mend(40, &props, &[], rule);
            assert_eq!(repairs.len(), REPAIRS, "{rule:?}: {repairs:?}");
            let facing = props.iter().find(|p| p.item == piece).unwrap().facing;
            for (i, r) in repairs.iter().enumerate() {
                assert_eq!((r.piece, r.to.facing), (piece, facing), "{r:?}");
                assert!(
                    repairs[..i].iter().all(|q| q.at.left != r.at.left),
                    "the same place twice: {repairs:?}"
                );
            }
        }
        // The sofa turned away is turned round.
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let away = at(Sofa, Nook::Users, Side::Right, 10, Facing::Left);
        let (_, repairs) = mend(40, &[tv, away], &[], FACES);
        assert!(
            repairs
                .iter()
                .any(|r| r.piece == Sofa && r.to.facing == Facing::Right)
        );
    }

    /// The lamp goes to stand beside the bed.
    #[test]
    fn the_lamp_moves_beside_the_bed() {
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let lamp = at(Lamp, Nook::Users, Side::Right, 0, Facing::Right);
        let near = Rule::Near {
            a: Lamp,
            b: &[Bed, Desk],
            gap: 3,
        };
        let (home, repairs) = mend(40, &[bed, lamp], &[], near);
        let best = repairs.first().expect("a repair");
        assert_eq!(best.piece, Lamp);
        // From column 36 to 3 cells past the bed's end (column 11).
        assert_eq!(best.cost, 36 - 14, "{best:?}");
        assert_eq!(best.to.anchor.side, Side::Left, "from the bed's wall");
        let after = made(&home, best, &panes(40));
        assert!(between(&laid(&after, Lamp), &laid(&after, Bed)) <= 3);
    }

    /// The spots she'd try a piece in: the cheapest repair's piece, at
    /// its tier, within [`TIE_CELLS`] of it, one a spot (the sofa turned
    /// the other way where it would stand is no other spot). Set down in
    /// the first, each other still holds as a move from there
    /// ([`check_again`]), though there's nothing left to mend
    /// ([`check`]).
    #[test]
    fn trials_are_spots_as_good_as_the_cheapest() {
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let sofa = at(Sofa, Nook::Users, Side::Right, 10, Facing::Left);
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let lamp = at(Lamp, Nook::Users, Side::Right, 0, Facing::Right);
        let near = Rule::Near {
            a: Lamp,
            b: &[Bed, Desk],
            gap: 3,
        };
        for (props, rule, spots) in [(vec![tv, sofa], FACES, 3), (vec![bed, lamp], near, 3)] {
            let (home, repairs) = mend(40, &props, &[], rule);
            let ties = Trials::ties(&repairs);
            assert_eq!(ties.len(), spots, "{repairs:?}");
            let first = ties[0];
            assert_eq!(first, repairs[0]);
            for (i, t) in ties.iter().enumerate() {
                assert_eq!((t.piece, t.tier), (first.piece, first.tier));
                assert!(t.cost <= first.cost + TIE_CELLS, "{t:?}");
                assert!(
                    ties[..i].iter().all(|u| u.at.left != t.at.left),
                    "one a spot: {ties:?}"
                );
            }
            let mut trials = Trials::of(&ties);
            assert_eq!(trials.min, first.cost);
            let mut rest = Vec::new();
            while let Some(r) = trials.next(&first) {
                rest.push(r);
            }
            assert_eq!(rest, ties[1..]);
            assert!(!trials.remain());
            // Set down in the first spot: the rule holds, and each other
            // spot keeps it so.
            let mut after = home.clone();
            let p = after
                .props
                .iter_mut()
                .find(|p| p.item == first.piece)
                .unwrap();
            (p.strip, p.anchor, p.facing) =
                (first.to.strip, Some(first.to.anchor), first.to.facing);
            let nooks = panes(40);
            let buf = frame(40, &[]);
            let shown = after.clone().project(&buf, &nooks, &|_, _| false);
            let frame = Frame {
                buf: &buf,
                nooks: &nooks,
                blocked: &|_, _| false,
                shown: &shown,
                made: &[],
                keep: &Keep::default(),
            };
            for r in &ties[1..] {
                assert!(check(&after, &frame, r).is_none(), "nothing to mend: {r:?}");
                let at = check_again(&after, &frame, r).unwrap_or_else(|| panic!("{r:?}"));
                assert_eq!(at.left, r.at.left);
            }
        }
    }

    /// The fridge goes to the nearer wall; with text along that one, to
    /// the other; with text everywhere it isn't, nowhere.
    #[test]
    fn the_fridge_goes_to_a_wall_clear_of_text() {
        let rule = Rule::AgainstWall(Fridge);
        let fridge = at(Fridge, Nook::Users, Side::Left, 15, Facing::Right);
        let e = raw_strips(&panes(40))[0].1;
        let (_, repairs) = mend(40, &[fridge], &[], rule);
        let best = repairs.first().expect("a repair");
        assert_eq!(best.cost, 16 - 2, "{best:?}");
        assert!(against_wall(&best.at, e));
        // Text by the left wall (on both strips).
        let (_, repairs) = mend(40, &[fridge], &[2, 3], rule);
        let best = repairs.first().expect("a repair");
        assert!(best.at.left > 20, "{best:?}");
        assert!(against_wall(&best.at, e));
        // Text everywhere but under the fridge.
        let dense: Vec<u16> = (1..39).filter(|x| !(16..20).contains(x)).collect();
        let (_, repairs) = mend(40, &[fridge], &dense, rule);
        assert_eq!(repairs, []);
    }

    /// Of a bed and a TV in one room, the TV she hasn't settled moves,
    /// to make a living room of the sofa's when there's one (see
    /// `a_delivered_tv_leaves_the_bedroom_and_the_bed_stays` for an
    /// empty strip).
    #[test]
    fn of_the_bed_and_the_tv_the_unsettled_one_moves() {
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let tv = unsettled(at(Tv, Nook::Users, Side::Left, 12, Facing::Left));
        let sofa = at(Sofa, Nook::Playlist, Side::Left, 0, Facing::Right);
        let (home, repairs) = mend(40, &[bed, tv, sofa], &[], APART);
        let best = repairs.first().expect("a repair");
        assert_eq!((best.piece, best.tier), (Tv, 0), "{best:?}");
        assert_eq!(best.to.strip, sofa.strip);
        let after = made(&home, best, &panes(40));
        assert_eq!(room::role_of(&after, sofa.strip), Role::Living, "{after:?}");
    }

    /// A TV just delivered to her bedroom is what goes, to the empty
    /// strip, not the bed she chose a place for (though the bed's move
    /// is as cheap).
    #[test]
    fn a_delivered_tv_leaves_the_bedroom_and_the_bed_stays() {
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let tv = unsettled(at(Tv, Nook::Users, Side::Left, 12, Facing::Left));
        let (_, repairs) = mend(40, &[bed, tv], &[], APART);
        assert!(!repairs.is_empty());
        for r in &repairs {
            assert_eq!(
                (r.piece, r.to.strip, r.tier),
                (Tv, Strip::Bottom(Nook::Playlist), 2)
            );
        }
    }

    /// A TV that stands where her window was hung (the window hung aside,
    /// clear of it: phase 5c D6) may still go: moving it off lets the
    /// window back to its own place, which pushes nothing aside.
    #[test]
    fn a_tv_under_her_window_may_go_and_the_window_hangs_back() {
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let tv = unsettled(at(Tv, Nook::Users, Side::Left, 12, Facing::Left));
        let window = at(Window, Nook::Users, Side::Left, 13, Facing::Right);
        let mut home = Home::default();
        for p in [bed, tv, window] {
            assert!(home.add(p));
        }
        let laid = home.clone().layout(&panes(40));
        assert_ne!(laid_left(&laid, Window), 14, "hung aside: {laid:?}");
        let (home, repairs) = mend(40, &[bed, tv, window], &[], APART);
        assert!(!repairs.is_empty());
        for r in &repairs {
            assert_eq!((r.piece, r.to.strip), (Tv, Strip::Bottom(Nook::Playlist)));
            let after = made(&home, r, &panes(40));
            assert_eq!(laid_left(&after, Window), 14, "{r:?}: back: {after:?}");
        }
    }

    /// Where her window and a sofa covering its corner meet, a cell is
    /// free for either piece (each may overlap the other) but not for her
    /// box, which overlaps no piece of hers ([`Frame::clear`] against
    /// [`Frame::room`]).
    #[test]
    fn her_box_shares_no_cell_with_a_piece_that_its_neighbour_may() {
        let sofa = at(Sofa, Nook::Users, Side::Left, 0, Facing::Right);
        let window = at(Window, Nook::Users, Side::Left, 7, Facing::Right);
        let mut home = Home::default();
        for p in [sofa, window] {
            assert!(home.add(p));
        }
        let nooks = panes(40);
        let buf = frame(40, &[]);
        let pieces = home.layout(&nooks);
        let frame = Frame {
            buf: &buf,
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &pieces,
            made: &[],
            keep: &Keep::default(),
        };
        let (s, w) = (laid(&pieces, Sofa), laid(&pieces, Window));
        let shared = w.rect().intersection(s.rect());
        assert!(!shared.is_empty(), "{pieces:?}");
        let (x, y) = (i32::from(shared.x), i32::from(shared.y));
        assert!(frame.clear(&pieces, &s, x, y) && frame.clear(&pieces, &w, x, y));
        assert!(!frame.room(&pieces, Sofa, x, y) && !frame.room(&pieces, Window, x, y));
    }

    fn laid_left(at: &[Shown], item: Furniture) -> i32 {
        laid(at, item).left
    }

    /// A TV delivered to a strip of its own comes to the sofa she has
    /// settled, not the sofa to it, anchored from the sofa's wall.
    #[test]
    fn a_delivered_tv_comes_to_the_sofa() {
        let sofa = at(Sofa, Nook::Users, Side::Right, 0, Facing::Left);
        let tv = unsettled(at(Tv, Nook::Playlist, Side::Left, 0, Facing::Right));
        let (home, repairs) = mend(40, &[sofa, tv], &[], FACES);
        assert!(!repairs.is_empty());
        for r in &repairs {
            assert_eq!((r.piece, r.to.strip, r.tier), (Tv, sofa.strip, 0), "{r:?}");
            assert_eq!(r.to.anchor.side, Side::Right, "from the sofa's wall");
            let after = made(&home, r, &panes(40));
            assert!(room::faces(&laid(&after, Sofa), &laid(&after, Tv)));
        }
    }

    /// With a delivered TV in the room, turning the settled sofa round
    /// where it stands is still the cheapest repair.
    #[test]
    fn a_turn_in_place_beats_moving_a_delivered_tv() {
        let tv = unsettled(at(Tv, Nook::Users, Side::Right, 0, Facing::Left));
        let sofa = at(Sofa, Nook::Users, Side::Right, 10, Facing::Left);
        let (_, repairs) = mend(40, &[tv, sofa], &[], FACES);
        let best = repairs.first().expect("a repair");
        assert_eq!((best.piece, best.cost, best.tier), (Sofa, 1, 0), "{best:?}");
        assert_eq!(best.to.anchor, sofa.anchor.unwrap());
    }

    /// `props` with `repair` made.
    fn moved(props: &[Prop], repair: &Repair) -> Vec<Prop> {
        props
            .iter()
            .map(|&p| {
                if p.item == repair.piece {
                    Prop {
                        strip: repair.to.strip,
                        anchor: Some(repair.to.anchor),
                        facing: repair.to.facing,
                        settled: true,
                        ..p
                    }
                } else {
                    p
                }
            })
            .collect()
    }

    /// Whether `repair` checks out with `props` on [`panes`] `width`
    /// wide, text at `text` and makeshift pieces at `made`.
    fn checks(width: u16, props: &[Prop], text: &[(u16, u16)], made: &[Rect], r: &Repair) -> bool {
        let nooks = panes(width);
        let buf = screen(&nooks, width, 16, text);
        let mut home = Home::default();
        for &p in props {
            assert!(home.add(p));
        }
        let shown = home.project(&buf, &nooks, &|_, _| false);
        let frame = Frame {
            buf: &buf,
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &shown,
            made,
            keep: &Keep::default(),
        };
        check(&home, &frame, r).is_some()
    }

    /// The cells under `at`.
    fn under(at: &Shown) -> Vec<(u16, u16)> {
        let rect = at.rect();
        rect.positions().map(|p| (p.x, p.y)).collect()
    }

    /// A repair found on one frame stops checking out when text comes
    /// where the piece would stand, a makeshift piece is built there,
    /// the rule is mended another way, or the panes shrink so the move
    /// no longer packs; it still does on a wider frame, and there the
    /// pair it keeps together still are.
    #[test]
    fn a_repair_checks_out_only_while_it_would_still_mend() {
        let bed = at(Bed, Nook::Users, Side::Left, 0, Facing::Right);
        let lamp = at(Lamp, Nook::Users, Side::Right, 0, Facing::Right);
        let near = Rule::Near {
            a: Lamp,
            b: &[Bed, Desk],
            gap: 3,
        };
        let props = [bed, lamp];
        let (_, repairs) = mend(40, &props, &[], near);
        let [first, second, ..] = repairs.as_slice() else {
            panic!("two repairs: {repairs:?}");
        };
        assert!(
            checks(40, &props, &[], &[], first),
            "the frame it was found on"
        );
        assert!(
            !checks(40, &props, &under(&first.at), &[], first),
            "text there"
        );
        assert!(
            !checks(40, &props, &[], &[first.at.rect()], first),
            "built there"
        );
        assert!(
            !checks(40, &moved(&props, second), &[], &[], first),
            "mended"
        );
        assert!(!checks(40, &moved(&props, first), &[], &[], first), "made");
        for width in [60, 120] {
            assert!(checks(width, &props, &[], &[], first), "{width} wide");
            let after = moved(&props, first);
            let mut home = Home::default();
            for p in after {
                assert!(home.add(p));
            }
            let laid = home.layout(&panes(width));
            assert!(
                between(&laid_of(&laid, Lamp), &laid_of(&laid, Bed)) <= 3,
                "{laid:?}"
            );
        }
        // A TV that would join the sofa: on panes too narrow for both on
        // one strip, it can't.
        let sofa = at(Sofa, Nook::Users, Side::Right, 0, Facing::Left);
        let tv = unsettled(at(Tv, Nook::Playlist, Side::Left, 0, Facing::Right));
        let props = [sofa, tv];
        let (_, repairs) = mend(40, &props, &[], FACES);
        let first = repairs.first().expect("a repair");
        assert!(checks(40, &props, &[], &[], first));
        assert!(!checks(16, &props, &[], &[], first), "too narrow");
        for width in [60, 120] {
            assert!(checks(width, &props, &[], &[], first), "{width} wide");
            let mut home = Home::default();
            for p in moved(&props, first) {
                assert!(home.add(p));
            }
            let laid = home.layout(&panes(width));
            assert!(
                room::faces(&laid_of(&laid, Sofa), &laid_of(&laid, Tv)),
                "{laid:?}"
            );
        }
    }

    /// A rule that holds offers nothing to mend; and nothing she does to
    /// mend one breaks it.
    #[test]
    fn a_home_whose_rules_hold_has_nothing_to_mend() {
        let tv = at(Tv, Nook::Users, Side::Right, 0, Facing::Left);
        let sofa = at(Sofa, Nook::Users, Side::Right, 10, Facing::Right);
        let bed = at(Bed, Nook::Playlist, Side::Left, 0, Facing::Right);
        let lamp = at(Lamp, Nook::Playlist, Side::Left, 11, Facing::Right);
        assert_eq!(judge(40, &[tv, sofa, bed, lamp]), []);
    }

    /// Three strips `width` wide, each `rows` tall, one above another;
    /// the first `n` of them.
    fn three(width: u16, rows: u16, n: usize) -> Vec<(Nook, Rect)> {
        [Nook::Users, Nook::Playlist, Nook::List]
            .into_iter()
            .enumerate()
            .take(n)
            .map(|(i, nook)| (nook, Rect::new(0, i as u16 * rows, width, rows)))
            .collect()
    }

    /// Searching the worst case — every piece, three strips so wide the
    /// places along them are strided, a sofa and a TV that can each move
    /// to the other, and text along every floor so that no move fits (so
    /// every one that passes the geometry is fit-checked) — weighs no
    /// more than [`CANDIDATES`] moves (and more than half as many: the
    /// stride is no coarser than it needs to be), in under a millisecond
    /// (the fastest of 20, in a release build). Without the text, it
    /// finds its repairs.
    #[test]
    fn repair_search_is_cheap() {
        let (width, rows) = (400, 20);
        let nooks = three(width, rows, 3);
        let place = |item, nook, side, offset| at(item, nook, side, offset, Facing::Right);
        let props = [
            place(Sofa, Nook::Users, Side::Left, 40),
            place(Tv, Nook::Playlist, Side::Right, 60),
            place(Bed, Nook::List, Side::Left, 0),
            place(Desk, Nook::Users, Side::Right, 0),
            place(Lamp, Nook::List, Side::Right, 0),
            place(Bookshelf, Nook::Playlist, Side::Left, 0),
            place(Fridge, Nook::Users, Side::Left, 0),
            place(CatBed, Nook::List, Side::Left, 80),
        ];
        let floors: Vec<(u16, u16)> = (0..3)
            .flat_map(|i| (1..width - 1).map(move |x| (x, i * rows + rows - 2)))
            .collect();
        // Without her door, and with it in the wall of the sofa's strip
        // (each place on a strip is judged on that strip's plan with the
        // piece on it: the door batch, step 2).
        let doors = [
            None,
            Some(DoorWall {
                strip: Strip::Bottom(Nook::Users),
                side: Side::Right,
            }),
        ];
        // And with a chat pane over Playlist and List (a custom layout's
        // grid over her panes): five of her pieces in it, so every move
        // weighed judges the closet (the door batch, step 6 review).
        let chats = [Rect::default(), Rect::new(0, rows, width, 2 * rows)];
        let cases = doors
            .into_iter()
            .flat_map(|d| [(d, false), (d, true)])
            .flat_map(|(d, dense)| chats.map(|chat| (d, dense, chat)));
        for (door, dense, chat) in cases {
            let text = if dense { floors.clone() } else { Vec::new() };
            let buf = screen(&nooks, width, 3 * rows, &text);
            let mut props = props.to_vec();
            let mut home = Home {
                door,
                ..Home::default()
            };
            for p in props.drain(..) {
                assert!(home.add(p));
            }
            let shown = home.project(&buf, &nooks, &|_, _| false);
            let keep = Keep::of(
                &home,
                room::Plan {
                    nooks: &nooks,
                    chat,
                    screen: buf.area,
                },
            );
            let laid = home.laid_out(&nooks);
            let in_chat = laid
                .shown
                .iter()
                .filter(|s| room::closetable(chat, s))
                .count();
            assert_eq!(in_chat, if chat.is_empty() { 0 } else { 5 });
            let broken = broken(&laid, &home, &keep)
                .into_iter()
                .find(|b| b.row == row(FACES))
                .expect("its sofa turned from its TV");
            assert_eq!(broken.pieces, [Sofa, Tv]);
            let frame = Frame {
                buf: &buf,
                nooks: &nooks,
                blocked: &|_, _| false,
                shown: &shown,
                made: &[],
                keep: &keep,
            };
            let found = search(&home, &frame, &broken);
            assert!(found.examined <= CANDIDATES, "{}", found.examined);
            assert!(found.examined > CANDIDATES / 2, "{}", found.examined);
            let want = if dense { 0 } else { REPAIRS };
            assert_eq!(found.repairs.len(), want, "dense {dense}, chat {chat:?}");
            // The fastest of 20: what a search costs. Tests running
            // alongside (a deep pass loads every core) only ever make
            // a run slower, so a mean would measure the machine.
            let per_search = (0..20)
                .map(|_| {
                    let started = std::time::Instant::now();
                    std::hint::black_box(search(&home, &frame, &broken));
                    started.elapsed()
                })
                .min()
                .unwrap_or_default();
            eprintln!(
                "repair search (dense {dense}, door {door:?}, chat {chat:?}): {per_search:?}, \
                 {} moves (release baseline 2026-10-08: 0.43-0.48 ms; bound 1 ms)",
                found.examined
            );
            // The bound stays loose (a deep pass's load has pushed a run
            // past 1 ms), but alone in release a search took 0.43-0.48
            // ms on 2026-10-08 (the door batch, step 3p), door or not:
            // a step that adds work per move re-measures against that
            // (`--release -E 'test(repair_search_is_cheap)' --no-capture`).
            if !cfg!(debug_assertions) {
                assert!(
                    per_search < std::time::Duration::from_millis(1),
                    "dense {dense}, door {door:?}, chat {chat:?}: {per_search:?}"
                );
            }
        }
    }

    /// A move that crowds her door's kept space is refused (F7's case,
    /// the door batch): her sofa joining her TV on a 22-wide Users pane,
    /// her door in its left wall, crowds the floor (15 columns of 20, 14
    /// beside the space), so the space would yield. Set down at the
    /// first place past the space (the plan before the move's grid), or
    /// against the wall inside it (where it would face the TV with room
    /// for her: once a repair, the plan with the sofa there, F7), it's
    /// refused, so no repair sets the sofa on Users at all.
    #[test]
    fn a_move_that_crowds_her_kept_door_space_is_refused() {
        let nooks = three(22, 12, 2);
        let buf = screen(&nooks, 22, 24, &[]);
        let mut home = Home {
            door: Some(DoorWall {
                strip: Strip::Bottom(Nook::Users),
                side: Side::Left,
            }),
            ..Home::default()
        };
        assert!(home.add(at(Tv, Nook::Users, Side::Right, 0, Facing::Left)));
        assert!(home.add(at(Sofa, Nook::Playlist, Side::Left, 0, Facing::Right)));
        let users = Strip::Bottom(Nook::Users);
        let kept = |home: &Home| home.extents(&nooks)[0].space.expect("a space").kept;
        assert!(kept(&home), "the TV alone packs beside the space");
        let mut joined = home.clone();
        joined.props[1].strip = users;
        assert!(!kept(&joined), "with the sofa there, it yields");
        let shown = home.project(&buf, &nooks, &|_, _| false);
        let broken = judged(&mut home, &nooks)
            .into_iter()
            .find(|b| b.row == row(FACES))
            .expect("her sofa away from her TV");
        let frame = Frame {
            buf: &buf,
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &shown,
            made: &[],
            keep: &Keep::default(),
        };
        // The first place along the strip, and the place by the wall as
        // the plan with the sofa there has it: the space yields for either.
        let before = Before::new(&home, &frame);
        let raw = raw_strips(&nooks)[0].1;
        let first = Placement {
            strip: users,
            anchor: Anchor {
                side: Side::Left,
                offset: 0,
            },
            facing: Facing::Right,
        };
        let plan = plan_of(&before, users).unwrap();
        assert_eq!(plan.floor.left(first.anchor, 9), raw.from + room::SPACE);
        let mut wide = plan;
        wide.floor = raw;
        for (plan, what) in [(plan, "past the space"), (wide, "by the wall")] {
            let mut scratch = before.home.clone();
            assert_eq!(
                evaluate(&before, &mut scratch, broken.key, Sofa, first, &plan, false),
                None,
                "{what}: it would crowd her door's space"
            );
        }
        let found = search(&home, &frame, &broken);
        assert!(
            !found.repairs.iter().any(|r| r.at.strip == Some(users)),
            "{:?} {}",
            found.repairs,
            found.examined
        );
    }

    /// Her home of `props` on `nooks` (over a bare screen `width` ×
    /// `height`), the chat at `chat`: how `rule` is broken (it must be),
    /// what a search of it weighs, and a frame to search on.
    fn on_frame<'a>(
        home: &mut Home,
        nooks: &'a [(Nook, Rect)],
        buf: &'a Buffer,
        shown: &'a mut Vec<Shown>,
        keep: &'a Keep,
        rule: usize,
    ) -> (Broken, Frame<'a>) {
        *shown = home.project(buf, nooks, &|_, _| false);
        let laid = home.laid_out(nooks);
        let broken = broken(&laid, home, keep)
            .into_iter()
            .find(|b| b.row == rule)
            .expect("the rule broken");
        let frame = Frame {
            buf,
            nooks,
            blocked: &|_, _| false,
            shown,
            made: &[],
            keep,
        };
        (broken, frame)
    }

    /// No repair takes her door's space away (the door batch, step 6
    /// review): her bed and TV share Playlist (Apart broken), and Users,
    /// 14 columns, keeps her door's space beside a lamp; the bed moved to
    /// Users is wider than the space leaves room for, so there'd be no
    /// space at all (and DoorClear, judged only where one exists, would
    /// never see it). Refused, as a delivery that would do so is
    /// ([`room::admits`]). Red before: weighed and passed.
    #[test]
    fn a_repair_never_takes_her_door_space_away() {
        let nooks = vec![
            (Nook::Users, Rect::new(0, 0, 16, 12)),
            (Nook::Playlist, Rect::new(0, 12, 40, 12)),
        ];
        let buf = screen(&nooks, 40, 24, &[]);
        let users = Strip::Bottom(Nook::Users);
        let mut home = Home {
            door: Some(DoorWall {
                strip: users,
                side: Side::Right,
            }),
            ..Home::default()
        };
        assert!(home.add(at(Lamp, Nook::Users, Side::Left, 0, Facing::Right)));
        assert!(home.add(at(Bed, Nook::Playlist, Side::Left, 0, Facing::Right)));
        assert!(home.add(at(Tv, Nook::Playlist, Side::Left, 14, Facing::Left)));
        let space = |home: &Home| home.extents(&nooks)[0].space;
        assert!(space(&home).is_some_and(|s| s.kept), "kept beside the lamp");
        let to = Placement {
            strip: users,
            anchor: Anchor {
                side: Side::Left,
                offset: 3,
            },
            facing: Facing::Right,
        };
        let mut moved = home.clone();
        moved.props[1].strip = users;
        moved.props[1].anchor = Some(to.anchor);
        assert_eq!(space(&moved), None, "no room for a space beside the bed");
        assert_eq!(
            moved.clone().laid_out(&nooks).shown.len(),
            3,
            "the bed stands"
        );
        let (keep, mut shown) = (Keep::default(), Vec::new());
        let (broken, frame) = on_frame(&mut home, &nooks, &buf, &mut shown, &keep, row(APART));
        let before = Before::new(&home, &frame);
        let plan = before.strips[0].plan;
        let mut scratch = before.home.clone();
        assert_eq!(
            evaluate(&before, &mut scratch, broken.key, Bed, to, &plan, false).map(|(r, _)| r),
            None
        );
        let found = search(&home, &frame, &broken);
        assert!(
            !found
                .repairs
                .iter()
                .any(|r| r.at.strip == Some(users) && r.piece == Bed),
            "{:?}",
            found.repairs
        );
    }

    /// [`a_repair_never_takes_her_door_space_away`] for a hung piece: her
    /// window in the chat pane (InChat broken) moved to Users, against
    /// its right wall, hangs where it's set down, clear of her door's
    /// space; but laid out with the space kept her lamp and sofa leave it
    /// nowhere, so the space would yield, with nothing standing in it to
    /// blame (no DoorClear). Refused. Red before: weighed and passed.
    #[test]
    fn a_repair_never_makes_her_kept_door_space_yield() {
        let nooks = vec![
            (Nook::Users, Rect::new(0, 0, 22, 12)),
            (Nook::Playlist, Rect::new(0, 12, 40, 12)),
        ];
        let buf = screen(&nooks, 40, 24, &[]);
        let users = Strip::Bottom(Nook::Users);
        let mut home = Home {
            door: Some(DoorWall {
                strip: users,
                side: Side::Left,
            }),
            ..Home::default()
        };
        assert!(home.add(at(Lamp, Nook::Users, Side::Left, 6, Facing::Right)));
        assert!(home.add(at(Sofa, Nook::Users, Side::Left, 7, Facing::Right)));
        assert!(home.add(at(Window, Nook::Playlist, Side::Left, 0, Facing::Right)));
        let chat = Rect::new(0, 12, 8, 12);
        let keep = Keep::of(
            &home,
            room::Plan {
                nooks: &nooks,
                chat,
                screen: buf.area,
            },
        );
        let space = |home: &Home| home.extents(&nooks)[0].space;
        assert!(space(&home).is_some_and(|s| s.kept), "kept");
        let to = Placement {
            strip: users,
            anchor: Anchor {
                side: Side::Right,
                offset: 0,
            },
            facing: Facing::Right,
        };
        let mut moved = home.clone();
        moved.props[2].strip = users;
        moved.props[2].anchor = Some(to.anchor);
        let yielded = space(&moved).expect("a space");
        assert!(!yielded.kept, "it yields for the window");
        let laid = moved.clone().laid_out(&nooks).shown;
        assert_eq!(laid.len(), 3);
        assert!(
            !laid.iter().any(|s| s.cover().intersects(yielded.rect)),
            "nothing in the space: {laid:?}"
        );
        let mut shown = Vec::new();
        let (broken, frame) = on_frame(&mut home, &nooks, &buf, &mut shown, &keep, row(INCHAT));
        let before = Before::new(&home, &frame);
        let plan = before.strips[0].plan;
        let mut scratch = before.home.clone();
        assert_eq!(
            evaluate(&before, &mut scratch, broken.key, Window, to, &plan, false).map(|(r, _)| r),
            None
        );
    }

    /// The closet and a repair (the door batch, step 6 review): Users
    /// and Playlist 30 wide, a chat pane over Users but its last six
    /// columns, her desk in it (too wide for what's left). With `crowded`,
    /// her TV, sofa, bookshelf (away from a wall: AgainstWall) and fridge
    /// leave Playlist 4 columns, and the desk is in the closet. Else her
    /// TV, sofa and plant leave it 10, so the desk shows in the chat
    /// (InChat), and her fridge, two cells from Users' right wall, stands
    /// away from it (AgainstWall).
    fn closet_home(crowded: bool) -> (Vec<(Nook, Rect)>, Buffer, Home, Keep) {
        let nooks = three(30, 12, 2);
        let buf = screen(&nooks, 30, 24, &[]);
        let mut home = Home::default();
        let (users, playlist) = (Nook::Users, Nook::Playlist);
        assert!(home.add(at(Desk, users, Side::Left, 0, Facing::Right)));
        assert!(home.add(at(Tv, playlist, Side::Left, 0, Facing::Right)));
        assert!(home.add(at(Sofa, playlist, Side::Left, 6, Facing::Left)));
        if crowded {
            assert!(home.add(at(Bookshelf, playlist, Side::Left, 15, Facing::Right)));
            assert!(home.add(at(Fridge, playlist, Side::Right, 0, Facing::Right)));
        } else {
            assert!(home.add(at(Plant, playlist, Side::Left, 15, Facing::Right)));
            assert!(home.add(at(Fridge, users, Side::Right, 2, Facing::Right)));
        }
        let keep = Keep::of(
            &home,
            room::Plan {
                nooks: &nooks,
                chat: Rect::new(0, 0, 23, 12),
                screen: buf.area,
            },
        );
        (nooks, buf, home, keep)
    }

    /// A piece in the closet never stops a repair that makes room for it
    /// (the door batch, step 6 review): her bookshelf moved off crowded
    /// Playlist to Users' right wall leaves room for the desk there, so
    /// it comes out of the closet, in the chat; it was there all along,
    /// so that's no rule newly broken. Red before: refused (InChat newly
    /// broken for the desk).
    #[test]
    fn a_piece_in_the_closet_never_blocks_a_repair_that_frees_room() {
        let (nooks, buf, mut home, keep) = closet_home(true);
        let laid = home.laid_out(&nooks);
        assert_eq!(closeted(&laid, &home, &keep), vec![Desk], "in the closet");
        let mut shown = Vec::new();
        let wall = row(Rule::AgainstWall(Bookshelf));
        let (broken, frame) = on_frame(&mut home, &nooks, &buf, &mut shown, &keep, wall);
        let before = Before::new(&home, &frame);
        let users = Strip::Bottom(Nook::Users);
        let to = Placement {
            strip: users,
            anchor: Anchor {
                side: Side::Right,
                offset: 0,
            },
            facing: Facing::Right,
        };
        let mut after = home.clone();
        let shelf = after
            .props
            .iter_mut()
            .find(|p| p.item == Bookshelf)
            .unwrap();
        shelf.strip = users;
        shelf.anchor = Some(to.anchor);
        let laid = after.clone().laid_out(&nooks);
        assert!(
            closeted(&laid, &after, &keep).is_empty(),
            "out: {:?}",
            laid.shown
        );
        let plan = before.strips[0].plan;
        let mut scratch = before.home.clone();
        let repair = evaluate(
            &before,
            &mut scratch,
            broken.key,
            Bookshelf,
            to,
            &plan,
            false,
        );
        assert!(repair.is_some(), "mends it, and frees room for the desk");
    }

    /// No repair puts a piece that shows in the closet (the door batch,
    /// step 6 review): her desk shows in the chat (Playlist has room for
    /// it); her fridge moved from beside Users' wall to Playlist's free
    /// right wall would mend its rule, but fill the room the desk had,
    /// and the desk would vanish for it. Refused.
    /// Red before: weighed and passed.
    #[test]
    fn no_repair_puts_a_piece_that_shows_in_the_closet() {
        let (nooks, buf, mut home, keep) = closet_home(false);
        let laid = home.laid_out(&nooks);
        assert!(closeted(&laid, &home, &keep).is_empty(), "the desk shows");
        let playlist = Strip::Bottom(Nook::Playlist);
        let to = Placement {
            strip: playlist,
            anchor: Anchor {
                side: Side::Right,
                offset: 0,
            },
            facing: Facing::Right,
        };
        let mut moved = home.clone();
        let fridge = moved.props.iter_mut().find(|p| p.item == Fridge).unwrap();
        fridge.strip = playlist;
        fridge.anchor = Some(to.anchor);
        assert_eq!(
            closeted(&moved.clone().laid_out(&nooks), &moved, &keep),
            vec![Desk],
            "with the fridge there, no room for the desk"
        );
        let mut shown = Vec::new();
        let wall = row(Rule::AgainstWall(Fridge));
        let (broken, frame) = on_frame(&mut home, &nooks, &buf, &mut shown, &keep, wall);
        let before = Before::new(&home, &frame);
        let plan = before.strips[1].plan;
        let mut scratch = before.home.clone();
        assert_eq!(
            evaluate(&before, &mut scratch, broken.key, Fridge, to, &plan, false).map(|(r, _)| r),
            None
        );
    }

    /// A crowded strip at the screen's edge is still where her door goes
    /// (the door batch, M5: whether its space is kept is only a
    /// tie-break): her TV and sofa crowd Users (15 columns of 20, 14
    /// beside the space), so its space yields, and the TV standing in it
    /// breaks [`Rule::DoorClear`] (step 2's test 6, carried to step 6).
    #[test]
    fn a_crowded_edge_strip_keeps_her_door_and_breaks_door_clear() {
        let nooks = three(22, 12, 2);
        let mut home = Home::default();
        assert!(home.add(at(Sofa, Nook::Users, Side::Left, 0, Facing::Right)));
        assert!(home.add(at(Tv, Nook::Users, Side::Right, 0, Facing::Left)));
        let wall = room::choose_wall(&home, room::Plan::bare(&nooks));
        let users = Strip::Bottom(Nook::Users);
        assert_eq!(
            wall,
            Some(DoorWall {
                strip: users,
                side: Side::Right
            })
        );
        home.door = wall;
        let space = home.extents(&nooks)[0].space.expect("a space");
        assert!(!space.kept, "it yields");
        let keys: Vec<Grievance> = judged(&mut home, &nooks).iter().map(|b| b.key).collect();
        assert_eq!(
            keys,
            vec![Grievance {
                row: row(Rule::DoorClear),
                piece: Tv
            }]
        );
    }

    /// Some of her pieces, each anchored on one of three strips, either
    /// way round, settled or not, boxed or not.
    fn pieces3() -> impl Strategy<Value = Vec<Prop>> {
        proptest::sample::subsequence(Furniture::ALL.to_vec(), 2..=8).prop_flat_map(|items| {
            let n = items.len();
            (
                Just(items),
                proptest::collection::vec(
                    (
                        0usize..3,
                        any::<bool>(),
                        0u16..60,
                        any::<bool>(),
                        any::<bool>(),
                        proptest::bool::weighted(0.1),
                    ),
                    n,
                ),
            )
                .prop_map(|(items, places)| {
                    items
                        .into_iter()
                        .zip(places)
                        .map(|(item, (nook, right, offset, left, settled, boxed))| {
                            let nook = [Nook::Users, Nook::Playlist, Nook::List][nook];
                            let side = if right { Side::Right } else { Side::Left };
                            let facing = if left { Facing::Left } else { Facing::Right };
                            Prop {
                                settled,
                                boxed,
                                ..at(item, nook, side, offset, facing)
                            }
                        })
                        .collect()
                })
        })
    }

    /// Makeshift pieces at `makeshift` (x, y, w, h) on `buf`, clear of
    /// hers (see `tend_made`).
    fn makeshift_on(
        buf: &Buffer,
        shown: &[Shown],
        makeshift: &[(u16, u16, u16, u16)],
    ) -> Vec<Rect> {
        makeshift
            .iter()
            .map(|&(x, y, w, h)| Rect::new(x, y, w, h).intersection(buf.area))
            .filter(|r| !shown.iter().any(|s| s.cover().intersects(*r)))
            .collect()
    }

    /// That `r`, judged on `buf` over `nooks` with `home`'s pieces showing
    /// as `shown`, `solid` what nothing of hers covers (makeshift pieces,
    /// protected or moved text) and `keep` what nothing is set down in
    /// (but where it stood), mends a rule broken
    /// there and breaks none that held; every strip that held its
    /// pieces still does; no room that was something becomes a den, and
    /// no piece spoils a room it didn't (nor the piece, unsettled, the
    /// one it's set down in); the piece stands on blank cells
    /// clear of everything, where she'd fit to use it; and every other
    /// piece that showed still fits (where she'd fit to use it, if she
    /// did). Returns where the piece would stand.
    fn mends(
        home: &Home,
        nooks: &[(Nook, Rect)],
        buf: &Buffer,
        shown: &[Shown],
        solid: &[Rect],
        keep: &Keep,
        r: &Repair,
    ) -> Result<Shown, TestCaseError> {
        let laid_out = home.clone().laid_out(nooks);
        let keys: Vec<Grievance> = broken(&laid_out, home, keep)
            .iter()
            .map(|b| b.key)
            .collect();
        let laid = laid_out.shown.clone();
        prop_assert!(keys.contains(&r.key), "{:?} mends what holds", r);
        // Clear for the piece `who` (of what it may not overlap: her
        // window and a sofa may), or for her box using it (`her`: of all).
        let free = |at: &[Shown], who: Furniture, her: bool| {
            let at = at.to_vec();
            let piece = at.iter().find(|s| s.item == who).copied();
            move |x: i32, y: i32| {
                let cell = (x as u16, y as u16).into();
                x >= 0
                    && y >= 0
                    && !solid.iter().any(|r| r.contains(cell))
                    && !at.iter().any(|s| {
                        s.item != who
                            && s.rect().contains(cell)
                            && (her || !piece.is_some_and(|p| p.may_overlap(s)))
                    })
            }
        };
        let clear = |at: &[Shown], who: Furniture| free(at, who, false);
        let room_for = |at: &[Shown], who: Furniture| free(at, who, true);
        let after = made_on(home, r, nooks);
        // The same pieces laid out: every strip still packs.
        prop_assert_eq!(after.len(), laid.len());
        for s in &laid {
            prop_assert!(after.iter().any(|a| a.item == s.item), "{:?}", s);
        }
        let at = laid_of(&after, r.piece);
        // Mends it; breaks nothing that held.
        let now = broken(&made_home(home, r).laid_out(nooks), home, keep);
        prop_assert!(!now.iter().any(|b| b.key == r.key), "{:?} {:?}", r, now);
        for b in &now {
            prop_assert!(keys.contains(&b.key), "{:?} newly broken by {:?}", b, r);
        }
        // No room that was something becomes a den; no piece spoils a
        // room it didn't, nor the piece, unsettled, the one it's set
        // down in.
        for strip in laid_out.plans.iter().map(|p| p.strip) {
            let (old, new) = (room::role_of(&laid, strip), room::role_of(&after, strip));
            if has_room(&laid, strip) && old != Role::Den && has_room(&after, strip) {
                prop_assert_ne!(new, Role::Den, "{:?} {:?} by {:?}", strip, old, r);
            }
        }
        let settled = home.props.iter().any(|p| p.item == r.piece && p.settled);
        prop_assert!(settled || !spoils(&after, r.to.strip, r.piece), "{:?}", r);
        let was = spoilt(&laid);
        for s in spoilt(&after) {
            prop_assert!(was.contains(&s), "{:?} newly spoilt by {:?}", s, r);
        }
        // Fits, clear of everything (but what it may overlap), where
        // she'd use it.
        let mine = clear(&after, r.piece);
        prop_assert!(room::fits(buf, &at, &mine), "{:?}", r);
        prop_assert!(room::roomy(buf, &at, &room_for(&after, r.piece)), "{:?}", r);
        // Nothing went under her window, low: every other hung piece
        // hangs no further from where its wall alone would hang it than
        // it did, and the piece just where it's set down.
        let shifts = |home: &Home| home.clone().laid_out(nooks).shifts;
        let (was_shifted, now_shifted) = (shifts(home), shifts(&made_home(home, r)));
        let far = |list: &[(Furniture, Option<u32>)], item: Furniture| {
            list.iter()
                .find(|(i, _)| *i == item)
                .map_or(0, |(_, d)| d.unwrap_or(u32::MAX))
        };
        prop_assert_eq!(far(&now_shifted, r.piece), 0, "{:?}", r);
        for &(item, _) in &now_shifted {
            prop_assert!(
                far(&now_shifted, item) <= far(&was_shifted, item),
                "{:?} pushed by {:?}",
                item,
                r
            );
        }
        prop_assert!(!solid.iter().any(|m| m.intersects(at.rect())), "{:?}", r);
        // Never set down where the frame keeps pieces out (the chat pane,
        // her door's space), but where it stood (turned).
        let stood = laid_of(&laid, r.piece);
        prop_assert!(
            !keep.refuses(at.cover()) || stood.cover() == at.cover(),
            "{:?} into {:?}",
            r,
            keep
        );
        // Every other piece that showed still fits.
        for s in shown
            .iter()
            .filter(|s| s.item != r.piece && s.scrap.is_none())
        {
            let now = laid_of(&after, s.item);
            let theirs = clear(&after, s.item);
            prop_assert!(room::fits(buf, &now, &theirs), "{:?} by {:?}", s, r);
            let before = room_for(&laid, s.item);
            if room::roomy(buf, s, &before) {
                prop_assert!(
                    room::roomy(buf, &now, &room_for(&after, s.item)),
                    "{:?} by {:?}",
                    s,
                    r
                );
            }
        }
        Ok(at)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]

        /// Every repair found mends its rule and breaks nothing (see
        /// [`mends`]: nor covers a blocked cell, protected or moved
        /// text); a turn keeps its anchor; a piece she has settled
        /// moves only after every move of one she hasn't (but for a turn
        /// where it stands); no search weighs more than [`CANDIDATES`]
        /// moves. And on another frame — wider or narrower, more text,
        /// more makeshift pieces — a repair that still checks out mends
        /// and breaks nothing there.
        #[test]
        fn every_repair_mends_and_breaks_nothing(
            props in pieces3(),
            width in 24u16..120,
            n in 2usize..=3,
            text in proptest::collection::vec((1u16..120, 0u16..24), 0..40),
            makeshift in proptest::collection::vec((1u16..120, 0u16..24, 2u16..6, 1u16..3), 0..3),
            later_width in 24u16..120,
            later_text in proptest::collection::vec((1u16..120, 0u16..24), 0..40),
            later_makeshift in proptest::collection::vec((1u16..120, 0u16..24, 2u16..6, 1u16..3), 0..3),
            blocked in proptest::collection::vec((1u16..120, 0u16..24, 1u16..8, 1u16..4), 0..3),
        ) {
            repairs_mend(Draw { props, width, n, text, makeshift, later_width, later_text, later_makeshift, blocked }, None, None)?;
        }

        /// [`every_repair_mends_and_breaks_nothing`] with her door in a
        /// wall of one of the strips (the door batch, step 2): every
        /// repair still mends and breaks nothing, and stands where it's
        /// re-laid. For a move crowding her door's kept space (refused),
        /// see `a_move_that_crowds_her_kept_door_space_is_refused` and
        /// `a_repair_never_takes_her_door_space_away`.
        #[test]
        fn every_repair_mends_and_breaks_nothing_with_her_door(
            props in pieces3(),
            width in 24u16..120,
            n in 2usize..=3,
            text in proptest::collection::vec((1u16..120, 0u16..24), 0..40),
            makeshift in proptest::collection::vec((1u16..120, 0u16..24, 2u16..6, 1u16..3), 0..3),
            later_width in 24u16..120,
            later_text in proptest::collection::vec((1u16..120, 0u16..24), 0..40),
            later_makeshift in proptest::collection::vec((1u16..120, 0u16..24, 2u16..6, 1u16..3), 0..3),
            blocked in proptest::collection::vec((1u16..120, 0u16..24, 1u16..8, 1u16..4), 0..3),
            strip in 0usize..3,
            right in any::<bool>(),
        ) {
            let nook = [Nook::Users, Nook::Playlist, Nook::List][strip % n];
            let door = DoorWall {
                strip: Strip::Bottom(nook),
                side: if right { Side::Right } else { Side::Left },
            };
            repairs_mend(Draw { props, width, n, text, makeshift, later_width, later_text, later_makeshift, blocked }, Some(door), None)?;
        }

        /// [`every_repair_mends_and_breaks_nothing_with_her_door`] with a
        /// chat pane laid over her strips (a custom layout's grid; the
        /// door batch, step 6), maybe her door's wall in none: the felt
        /// rules [`Rule::DoorClear`] and [`Rule::InChat`] broken, every
        /// repair still mends its rule and breaks nothing, and is never
        /// set down in the chat or her door's space.
        #[test]
        fn every_repair_mends_and_breaks_nothing_with_the_chat_over_her_pieces(
            props in pieces3(),
            width in 24u16..120,
            n in 2usize..=3,
            text in proptest::collection::vec((1u16..120, 0u16..24), 0..40),
            makeshift in proptest::collection::vec((1u16..120, 0u16..24, 2u16..6, 1u16..3), 0..3),
            later_width in 24u16..120,
            later_text in proptest::collection::vec((1u16..120, 0u16..24), 0..40),
            later_makeshift in proptest::collection::vec((1u16..120, 0u16..24, 2u16..6, 1u16..3), 0..3),
            blocked in proptest::collection::vec((1u16..120, 0u16..24, 1u16..8, 1u16..4), 0..3),
            door in proptest::option::of((0usize..3, any::<bool>())),
            chat in (0u16..120, 0u16..24, 1u16..40, 1u16..12),
        ) {
            let door = door.map(|(strip, right)| DoorWall {
                strip: Strip::Bottom([Nook::Users, Nook::Playlist, Nook::List][strip % n]),
                side: if right { Side::Right } else { Side::Left },
            });
            let chat = Rect::new(chat.0, chat.1, chat.2, chat.3);
            repairs_mend(Draw { props, width, n, text, makeshift, later_width, later_text, later_makeshift, blocked }, door, Some(chat))?;
        }
    }

    /// What [`every_repair_mends_and_breaks_nothing`] draws.
    struct Draw {
        props: Vec<Prop>,
        width: u16,
        n: usize,
        text: Vec<(u16, u16)>,
        makeshift: Vec<(u16, u16, u16, u16)>,
        later_width: u16,
        later_text: Vec<(u16, u16)>,
        later_makeshift: Vec<(u16, u16, u16, u16)>,
        blocked: Vec<(u16, u16, u16, u16)>,
    }

    /// The body of [`every_repair_mends_and_breaks_nothing`], her door in
    /// `door`'s wall, if any; with a `chat` pane, what the frame keeps
    /// pieces out of is its ([`Keep::of`]), else nothing.
    /// The body of [`every_repair_mends_and_breaks_nothing`] and its
    /// kin: each rule broken, by row, and how many repairs of it were
    /// found and checked (for non-vacuity: see the deterministic
    /// companions, e.g. `a_lamp_in_her_door_space_is_repaired_and_checked`).
    fn repairs_mend(
        draw: Draw,
        door: Option<DoorWall>,
        chat: Option<Rect>,
    ) -> Result<Vec<(usize, usize)>, TestCaseError> {
        let Draw {
            props,
            width,
            n,
            text,
            makeshift,
            later_width,
            later_text,
            later_makeshift,
            blocked,
        } = draw;
        let nooks = three(width, 8, n);
        let buf = screen(&nooks, width, 24, &text);
        let mut home = Home {
            door,
            ..Home::default()
        };
        for p in props {
            prop_assert!(home.add(p));
        }
        // Protected cells, and text she has moved: nothing of hers
        // covers them, now or later.
        let blocked: Vec<Rect> = blocked
            .iter()
            .map(|&(x, y, w, h)| Rect::new(x, y, w, h))
            .collect();
        let block = |x: i32, y: i32| {
            let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                return true;
            };
            blocked.iter().any(|r| r.contains((x, y).into()))
        };
        let shown = home.project(&buf, &nooks, &block);
        let made = makeshift_on(&buf, &shown, &makeshift);
        let keep_on = |home: &Home, nooks: &[(Nook, Rect)], buf: &Buffer| {
            chat.map_or_else(Keep::default, |chat| {
                Keep::of(
                    home,
                    room::Plan {
                        nooks,
                        chat,
                        screen: buf.area,
                    },
                )
            })
        };
        let keep = keep_on(&home, &nooks, &buf);
        let frame = Frame {
            buf: &buf,
            nooks: &nooks,
            blocked: &block,
            shown: &shown,
            made: &made,
            keep: &keep,
        };
        // The same home, later: resized, with more text and more
        // makeshift pieces.
        let later_nooks = three(later_width, 8, n);
        let mut later_text = later_text;
        later_text.extend(text.iter().copied());
        let later_buf = screen(&later_nooks, later_width, 24, &later_text);
        let mut later_home = home.clone();
        let later_shown = later_home.project(&later_buf, &later_nooks, &block);
        let later_made = makeshift_on(&later_buf, &later_shown, &later_makeshift);
        let later_keep = keep_on(&later_home, &later_nooks, &later_buf);
        let later = Frame {
            buf: &later_buf,
            nooks: &later_nooks,
            blocked: &block,
            shown: &later_shown,
            made: &later_made,
            keep: &later_keep,
        };
        let laid_out = home.clone().laid_out(&nooks);
        let was = broken(&laid_out, &home, &keep);
        let laid = laid_out.shown;
        let mut tally = Vec::new();
        for target in &was {
            let found = search(&home, &frame, target);
            tally.push((target.row, found.repairs.len()));
            prop_assert!(found.examined <= CANDIDATES, "{}", found.examined);
            prop_assert!(found.repairs.len() <= REPAIRS);
            let order: Vec<(u8, u32)> = found.repairs.iter().map(|r| (r.tier, r.cost)).collect();
            prop_assert!(order.is_sorted(), "{:?}", order);
            let settled =
                |piece: Furniture| home.props.iter().any(|p| p.item == piece && p.settled);
            let mut delivered = false;
            for r in found.repairs.iter().rev() {
                prop_assert_eq!(r.key, target.key);
                prop_assert!(target.pieces.contains(&r.piece));
                prop_assert!(check(&home, &frame, r).is_some(), "{:?}", r);
                let solid: Vec<Rect> = made.iter().chain(&blocked).copied().collect();
                let at = mends(&home, &nooks, &buf, &shown, &solid, &keep, r)?;
                prop_assert_eq!(at, r.at);
                // A turn where it stands keeps its anchor.
                let old = home.props.iter().find(|p| p.item == r.piece).unwrap();
                let then = laid_of(&laid, r.piece);
                let in_place = (then.strip, then.left) == (at.strip, at.left);
                if in_place {
                    prop_assert_eq!(Some(r.to.anchor), old.anchor, "{:?}", r);
                    prop_assert_eq!(r.cost, 1, "{:?}", r);
                }
                // (Walking from the dearest:) a settled piece moved
                // ahead of a delivery only by a turn.
                if !settled(r.piece) {
                    delivered = true;
                } else if delivered {
                    prop_assert!(in_place, "{:?} ahead of a delivery's move", r);
                }
                if check(&later_home, &later, r).is_some() {
                    mends(
                        &later_home,
                        &later_nooks,
                        &later_buf,
                        &later_shown,
                        &later_made
                            .iter()
                            .chain(&blocked)
                            .copied()
                            .collect::<Vec<_>>(),
                        &later_keep,
                        r,
                    )?;
                }
            }
        }
        Ok(tally)
    }

    /// [`repairs_mend`] on a drawn home with no text, nothing made or
    /// blocked, the same later: its tally.
    fn plain_repairs(
        props: Vec<Prop>,
        width: u16,
        door: Option<DoorWall>,
        chat: Option<Rect>,
    ) -> Vec<(usize, usize)> {
        let draw = Draw {
            props,
            width,
            n: 2,
            text: Vec::new(),
            makeshift: Vec::new(),
            later_width: width,
            later_text: Vec::new(),
            later_makeshift: Vec::new(),
            blocked: Vec::new(),
        };
        repairs_mend(draw, door, chat).unwrap_or_else(|e| panic!("{e}"))
    }

    /// The deterministic companion of the repair properties for
    /// [`Rule::DoorClear`] (T3; they seldom draw a space that yields
    /// with a piece in it): Users 28 columns, her door in its right
    /// wall, her fridge, sofa, cat bed and plant against the left (20
    /// columns) and her lamp in the corner by the door, so the space
    /// (22 columns beside it) yields, the lamp in it. DoorClear is
    /// broken, and repairs of it are found, each checked to mend it and
    /// break nothing ([`repairs_mend`]).
    #[test]
    fn a_lamp_in_her_door_space_is_repaired_and_checked() {
        let props = vec![
            at(Fridge, Nook::Users, Side::Left, 0, Facing::Right),
            at(Sofa, Nook::Users, Side::Left, 4, Facing::Right),
            at(CatBed, Nook::Users, Side::Left, 13, Facing::Right),
            at(Plant, Nook::Users, Side::Left, 17, Facing::Right),
            at(Lamp, Nook::Users, Side::Right, 0, Facing::Right),
        ];
        let door = DoorWall {
            strip: Strip::Bottom(Nook::Users),
            side: Side::Right,
        };
        let tally = plain_repairs(props, 30, Some(door), None);
        let door_clear = row(Rule::DoorClear);
        assert!(
            tally.iter().any(|&(r, found)| r == door_clear && found > 0),
            "{tally:?}"
        );
    }

    /// A case of [`every_repair_mends_and_breaks_nothing_with_the_chat_over_her_pieces`]
    /// (its regressions file has the seed), pinned: a clock and a window
    /// on Playlist, the chat over part of it. A repair search judging its
    /// moves with no chat pane (`Before`'s keep left empty, a mutant of
    /// the door batch's step 6 review) breaks InChat for the window.
    #[test]
    fn a_clock_and_a_window_by_the_chat_case() {
        let props = vec![
            unsettled(at(Clock, Nook::Playlist, Side::Left, 49, Facing::Right)),
            unsettled(at(Window, Nook::Playlist, Side::Left, 44, Facing::Right)),
        ];
        let draw = Draw {
            props,
            width: 70,
            n: 2,
            text: Vec::new(),
            makeshift: Vec::new(),
            later_width: 36,
            later_text: Vec::new(),
            later_makeshift: Vec::new(),
            blocked: Vec::new(),
        };
        let tally = repairs_mend(draw, None, Some(Rect::new(19, 1, 32, 9)))
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!tally.is_empty(), "something broken to repair");
    }

    /// [`a_lamp_in_her_door_space_is_repaired_and_checked`] for
    /// [`Rule::InChat`]: her plant against Users' left wall under a chat
    /// pane over Users' left end, Playlist bare. InChat is broken, and
    /// repairs of it are found and checked.
    #[test]
    fn a_plant_in_the_chat_is_repaired_and_checked() {
        let props = vec![at(Plant, Nook::Users, Side::Left, 0, Facing::Right)];
        let tally = plain_repairs(props, 30, None, Some(Rect::new(0, 0, 10, 8)));
        let in_chat = row(Rule::InChat);
        assert!(
            tally.iter().any(|&(r, found)| r == in_chat && found > 0),
            "{tally:?}"
        );
    }

    fn made_on(home: &Home, repair: &Repair, nooks: &[(Nook, Rect)]) -> Vec<Shown> {
        made(home, repair, nooks)
    }

    fn laid_of(at: &[Shown], item: Furniture) -> Shown {
        laid(at, item)
    }
}
