//! The rules of her home: how her pieces ought to stand, as a table.
//!
//! A rule is broken or not, judged on where her pieces are laid out
//! ([`super::room::Home::layout`]), never on what text closets this
//! frame; a satisfied rule never moves anything. A rule applies only to
//! pieces laid out and out of their boxes (a parcel isn't furniture
//! yet). Each has a grievance: what she says while using a piece it
//! involves, while it's broken.

use super::room::{
    self, Anchor, Extent, Furniture, Home, Nook, Prop, Role, Shown, Side, Strip, Use,
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
    /// A piece she hasn't settled doesn't spoil its room: without it the
    /// room is something (not a den), and it's something else with it
    /// (a bed in a living room; a TV or a fridge in a bedroom).
    Belongs,
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
];

/// The rules, in the order they're judged.
pub(super) const RULES: [RuleRow; 6] = [
    RuleRow {
        rule: Rule::Faces {
            seat: Furniture::Sofa,
            screen: Furniture::Tv,
        },
        felt_on: &[Use::Lounge, Use::Nap],
        grievance: "Can't see the telly...",
    },
    RuleRow {
        rule: Rule::Near {
            a: Furniture::Lamp,
            b: &[Furniture::Bed, Furniture::Desk],
            gap: 3,
        },
        felt_on: &[Use::Sleep, Use::Homework],
        grievance: "Too dark in here...",
    },
    RuleRow {
        rule: Rule::AgainstWall(Furniture::Fridge),
        felt_on: &[Use::Snack],
        grievance: "This wants a wall...",
    },
    RuleRow {
        rule: Rule::AgainstWall(Furniture::Bookshelf),
        felt_on: &[Use::Read],
        grievance: "Wobbly... needs a wall.",
    },
    RuleRow {
        rule: Rule::Apart {
            a: Furniture::Bed,
            b: Furniture::Tv,
        },
        felt_on: &[Use::Sleep],
        grievance: "Too noisy to sleep...",
    },
    RuleRow {
        rule: Rule::Belongs,
        felt_on: ANY_USE,
        grievance: "Hm... not in here.",
    },
];

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
        None => "?",
    }
}

/// Every rule broken by her pieces as `layout` has them on `strips`,
/// with what `home` knows of them (whether she has settled each), in
/// [`RULES`] order (and [`Rule::Belongs`] piece by piece in layout
/// order).
pub(super) fn broken(layout: &[Shown], strips: &[(Strip, Extent)], home: &Home) -> Vec<Broken> {
    let mut out: Vec<Broken> = Vec::new();
    for row in 0..RULES.len() {
        judge(row, layout, strips, home, &mut out);
    }
    out
}

/// Push how the rule on `row` is broken by her pieces as `layout` has
/// them on `strips` (see [`broken`]).
fn judge(
    row: usize,
    layout: &[Shown],
    strips: &[(Strip, Extent)],
    home: &Home,
    out: &mut Vec<Broken>,
) {
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
                && let Some(&(_, e)) = strips.iter().find(|(s, _)| Some(*s) == at.strip)
                && !against_wall(at, e)
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
    }
}

/// Whether `piece` spoils `strip`'s room as `layout` has it: without it
/// the room is something (not a den), and it's something else with it.
fn spoils(layout: &[Shown], strip: Strip, piece: Furniture) -> bool {
    let without = room::role_without(layout, strip, piece);
    without != Role::Den && without != room::role_of(layout, strip)
}

/// The cells between two pieces along a floor (0 when they touch).
fn between(a: &Shown, b: &Shown) -> i32 {
    let (ra, rb) = (a.rect(), b.rect());
    (i32::from(rb.x) - i32::from(ra.right())).max(i32::from(ra.x) - i32::from(rb.right()))
}

/// Whether `at` stands within [`WALL_GAP`] of one of `e`'s walls.
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
/// cells nothing of hers may cover, her pieces that show, and the
/// makeshift pieces' footprints (which nothing moves onto).
pub(super) struct Frame<'a> {
    pub buf: &'a Buffer,
    pub nooks: &'a [(Nook, Rect)],
    pub blocked: &'a dyn Fn(i32, i32) -> bool,
    pub shown: &'a [Shown],
    pub made: &'a [Rect],
}

impl Frame<'_> {
    /// Whether `(x, y)` is free for `who` with her pieces where `laid`
    /// has them: not blocked, not under a makeshift piece or another of
    /// hers.
    fn clear(&self, laid: &[Shown], who: Furniture, x: i32, y: i32) -> bool {
        let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        let cell = (ux, uy).into();
        !(self.blocked)(x, y)
            && !self.made.iter().any(|r| r.contains(cell))
            && !laid
                .iter()
                .any(|s| s.item != who && s.rect().contains(cell))
    }
}

/// Her home as it stands before a repair.
struct Before {
    /// Anchored wherever its strip is here.
    home: Home,
    strips: Vec<(Strip, Extent)>,
    laid: Vec<Shown>,
    broken: Vec<Broken>,
    /// Every strip with a piece out of its box on it, and its role.
    roles: Vec<(Strip, Role)>,
    /// Her pieces that show, and whether she'd fit to use each.
    showing: Vec<(Shown, bool)>,
}

impl Before {
    fn new(home: &Home, frame: &Frame) -> Self {
        let mut home = home.clone();
        let laid = home.layout(frame.nooks);
        let strips = room::strips(frame.nooks);
        let broken = broken(&laid, &strips, &home);
        let roles = strips
            .iter()
            .filter(|&&(strip, _)| has_room(&laid, strip))
            .map(|&(strip, _)| (strip, room::role_of(&laid, strip)))
            .collect();
        let showing = frame
            .shown
            .iter()
            .filter(|s| s.scrap.is_none())
            .map(|s| {
                let clear = |x: i32, y: i32| frame.clear(&laid, s.item, x, y);
                (*s, room::roomy(frame.buf, s, &clear))
            })
            .collect();
        Self {
            home,
            strips,
            laid,
            broken,
            roles,
            showing,
        }
    }
}

/// Whether `strip` has a piece out of its box on it (a room).
fn has_room(laid: &[Shown], strip: Strip) -> bool {
    laid.iter()
        .any(|s| s.strip == Some(strip) && !s.boxed && s.scrap.is_none())
}

/// `piece` set down at `to`: the repair of `key` it would be and her
/// pieces' layout then, if it passes the geometry (no frame needed):
/// `key` is broken, every piece laid out before still is, `key` holds,
/// no room is worse for it (and a piece she hasn't settled spoils
/// none), and no rule that held is broken. `scratch` is `before`'s
/// home, and is left so.
fn evaluate(
    before: &Before,
    scratch: &mut Home,
    key: Grievance,
    piece: Furniture,
    to: Placement,
) -> Option<(Repair, Vec<Shown>)> {
    // A rule that holds has nothing to mend (another move mended it,
    // or a resize did).
    let mending = before.broken.iter().find(|b| b.key == key)?;
    let i = before.home.props.iter().position(|p| p.item == piece)?;
    let old = *before.home.props.get(i)?;
    let &(_, e) = before.strips.iter().find(|(s, _)| *s == to.strip)?;
    if old.boxed || (old.strip, old.anchor, old.facing) == (to.strip, Some(to.anchor), to.facing) {
        return None;
    }
    let cols = piece.spec().footprint.0;
    let moved = Prop {
        strip: to.strip,
        anchor: Some(to.anchor),
        at: e.pin(e.left(to.anchor, cols), cols).1,
        facing: to.facing,
        ..old
    };
    let slot = scratch.props.get_mut(i)?;
    *slot = moved;
    let laid = scratch.laid_on(&before.strips);
    if let Some(slot) = scratch.props.get_mut(i) {
        *slot = old;
    }
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
    // It mends the rule (the settled flags are unchanged by a move).
    let mut target = Vec::new();
    judge(key.row, &laid, &before.strips, &before.home, &mut target);
    if target.iter().any(|b| b.key == key) {
        return None;
    }
    // No room is worse for it; set down, a piece is settled, so one she
    // hasn't settled may not spoil where it goes.
    if !old.settled && spoils(&laid, to.strip, piece) {
        return None;
    }
    let worse = before.roles.iter().any(|&(strip, role)| {
        role != Role::Den && has_room(&laid, strip) && room::role_of(&laid, strip) != role
    });
    if worse {
        return None;
    }
    // No rule that held is broken.
    if broken(&laid, &before.strips, &before.home)
        .iter()
        .any(|b| !before.broken.iter().any(|w| w.key == b.key))
    {
        return None;
    }
    // Of a piece she has settled and one she hasn't, she moves the one
    // she never chose a place for, wherever it can go; but turning the
    // settled one round where it stands is the cheapest of all.
    let was = before.laid.iter().find(|s| s.item == piece)?;
    let in_place = (at.strip, at.left, at.floor) == (was.strip, was.left, was.floor);
    let rival = mending
        .pieces
        .iter()
        .any(|&p| p != piece && before.home.props.iter().any(|q| q.item == p && !q.settled));
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
    } else if room::role_without(&laid, to.strip, piece) == Role::Den
        && room::role_of(&laid, to.strip) != Role::Den
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
    Some((repair, laid))
}

/// Whether, her pieces laid out as `laid` with `piece` moved, `piece`
/// stands on blank, free cells where she'd fit to use it, and every
/// other piece that showed still fits (and she'd still fit to use those
/// she would before).
fn fits_now(frame: &Frame, before: &Before, laid: &[Shown], piece: Furniture) -> bool {
    let fit = |at: &Shown| {
        let clear = |x: i32, y: i32| frame.clear(laid, at.item, x, y);
        room::fits(frame.buf, at, &clear)
    };
    let room = |at: &Shown| {
        let clear = |x: i32, y: i32| frame.clear(laid, at.item, x, y);
        room::roomy(frame.buf, at, &clear)
    };
    let Some(at) = laid.iter().find(|s| s.item == piece) else {
        return false;
    };
    fit(at)
        && room(at)
        && before
            .showing
            .iter()
            .filter(|(was, _)| was.item != piece)
            .all(|(was, roomy)| {
                laid.iter().find(|s| s.item == was.item).is_some_and(|now| {
                    // Where it stood, it still fits: nothing moved onto it.
                    (now == was || fit(now)) && (!roomy || room(now))
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
    e: Extent,
    /// Its leftmost place (against the far wall).
    hi: i32,
    /// Where it stands now, if on this strip.
    here: Option<i32>,
    /// The pieces it keeps company with there.
    partners: Vec<Shown>,
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
        let (cols, rows) = piece.spec().footprint;
        let with = partners(rule, piece);
        for &(strip, e) in &before.strips {
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
                    e,
                    hi: e.to - i32::from(cols),
                    here: (now.strip == Some(strip)).then_some(now.left),
                    partners: before.laid.iter().filter(beside).copied().collect(),
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
            .map(|p| 2 * lefts(stride, p).len())
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
            let flip = match old.facing {
                Facing::Left => Facing::Right,
                Facing::Right => Facing::Left,
            };
            for facing in [old.facing, flip] {
                examined += 1;
                let to = Placement {
                    strip: p.strip,
                    anchor,
                    facing,
                };
                let Some((repair, laid)) = evaluate(&before, &mut scratch, target.key, p.piece, to)
                else {
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

/// Whether `repair` still mends its rule on `frame`, as [`search`] would
/// judge it there (where the piece would stand may have moved with a
/// resize).
pub(super) fn check(home: &Home, frame: &Frame, repair: &Repair) -> bool {
    let before = Before::new(home, frame);
    let mut scratch = before.home.clone();
    evaluate(&before, &mut scratch, repair.key, repair.piece, repair.to)
        .is_some_and(|(_, laid)| fits_now(frame, &before, &laid, repair.piece))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::super::room::{Anchor, Nook, Prop, Side, strips};
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
        let laid = home.layout(&nooks);
        broken(&laid, &strips(&nooks), &home)
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
        let laid = home.layout(&nooks);
        let got = broken(&laid, &strips(&nooks), &home);
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
                | Use::Pet => true,
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
        let laid = home.layout(nooks);
        broken(&laid, &strips(nooks), home)
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
            let mut home = Home::default();
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
                        "{:?} in {:?}", piece, b
                    );
                }
                prop_assert!(b.pieces.iter().all(|p| b.involved.contains(p)), "{:?}", b);
            }
            // A resize, and back (where it moved nothing off its strip).
            let _ = home.project(&frame(narrow, &[]), &small, &|_, _| false);
            let _ = judged(&mut home, &small);
            if home == before {
                let _ = home.project(&clean, &big, &|_, _| false);
                prop_assert_eq!(&judged(&mut home, &big), &first);
            }
            // Text closets what it covers; the rules don't see it.
            let mut home = before;
            let _ = home.project(&frame(wide, &text), &big, &|_, _| false);
            prop_assert_eq!(&judged(&mut home, &big), &first);
        }
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
        let nooks = panes(width);
        let buf = frame(width, text);
        let (home, shown, broken) = broken_at(props, &buf, &nooks, rule);
        let frame = Frame {
            buf: &buf,
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &shown,
            made: &[],
        };
        let found = search(&home, &frame, &broken);
        assert!(found.examined <= CANDIDATES, "{}", found.examined);
        for r in &found.repairs {
            assert!(check(&home, &frame, r), "{r:?}");
        }
        (home, found.repairs)
    }

    /// `home` with `repair` made, laid out on `nooks`.
    fn made(home: &Home, repair: &Repair, nooks: &[(Nook, Rect)]) -> Vec<Shown> {
        let mut after = home.clone();
        let prop = after
            .props
            .iter_mut()
            .find(|p| p.item == repair.piece)
            .unwrap();
        prop.strip = repair.to.strip;
        prop.anchor = Some(repair.to.anchor);
        prop.facing = repair.to.facing;
        after.layout(nooks)
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
    /// sofa going to it would make the kitchen a living room, besides
    /// being a settled piece; see `a_delivered_tv_comes_to_the_sofa`
    /// for the plain case).
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

    /// The fridge goes to the nearer wall; with text along that one, to
    /// the other; with text everywhere it isn't, nowhere.
    #[test]
    fn the_fridge_goes_to_a_wall_clear_of_text() {
        let rule = Rule::AgainstWall(Fridge);
        let fridge = at(Fridge, Nook::Users, Side::Left, 15, Facing::Right);
        let e = strips(&panes(40))[0].1;
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
        };
        check(&home, &frame, r)
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

    /// Searching the worst case — every piece, three strips 200 wide,
    /// and a sofa and a TV that can each move to the other — weighs no
    /// more than [`CANDIDATES`] moves, in under a millisecond (in a
    /// release build).
    #[test]
    fn repair_search_is_cheap() {
        let nooks = three(200, 20, 3);
        let buf = screen(&nooks, 200, 60, &[]);
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
        let (home, shown, broken) = broken_at(&props, &buf, &nooks, FACES);
        assert_eq!(broken.pieces, [Sofa, Tv]);
        let frame = Frame {
            buf: &buf,
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &shown,
            made: &[],
        };
        let found = search(&home, &frame, &broken);
        assert!(found.examined <= CANDIDATES, "{}", found.examined);
        assert!(found.examined > 500, "{}", found.examined);
        assert_eq!(found.repairs.len(), REPAIRS);
        let started = std::time::Instant::now();
        for _ in 0..20 {
            std::hint::black_box(search(&home, &frame, &broken));
        }
        let per_search = started.elapsed() / 20;
        eprintln!("repair search: {per_search:?}, {} moves", found.examined);
        if !cfg!(debug_assertions) {
            assert!(
                per_search < std::time::Duration::from_millis(1),
                "{per_search:?}"
            );
        }
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
    /// as `shown` and makeshift pieces at `made`, mends a rule broken
    /// there and breaks none that held; every strip that held its
    /// pieces still does; no room is worse for it (and a piece she
    /// hasn't settled spoils none); the piece stands on blank cells
    /// clear of everything, where she'd fit to use it; and every other
    /// piece that showed still fits (where she'd fit to use it, if she
    /// did). Returns where the piece would stand.
    fn mends(
        home: &Home,
        nooks: &[(Nook, Rect)],
        buf: &Buffer,
        shown: &[Shown],
        made: &[Rect],
        r: &Repair,
    ) -> Result<Shown, TestCaseError> {
        let all = strips(nooks);
        let laid = home.clone().layout(nooks);
        let keys: Vec<Grievance> = broken(&laid, &all, home).iter().map(|b| b.key).collect();
        prop_assert!(keys.contains(&r.key), "{:?} mends what holds", r);
        let clear = |at: &[Shown], who: Furniture| {
            let at = at.to_vec();
            move |x: i32, y: i32| {
                let cell = (x as u16, y as u16).into();
                x >= 0
                    && y >= 0
                    && !made.iter().any(|r| r.contains(cell))
                    && !at.iter().any(|s| s.item != who && s.rect().contains(cell))
            }
        };
        let after = made_on(home, r, nooks);
        // The same pieces laid out: every strip still packs.
        prop_assert_eq!(after.len(), laid.len());
        for s in &laid {
            prop_assert!(after.iter().any(|a| a.item == s.item), "{:?}", s);
        }
        let at = laid_of(&after, r.piece);
        // Mends it; breaks nothing that held.
        let now = broken(&after, &all, home);
        prop_assert!(!now.iter().any(|b| b.key == r.key), "{:?} {:?}", r, now);
        for b in &now {
            prop_assert!(keys.contains(&b.key), "{:?} newly broken by {:?}", b, r);
        }
        // No room worse; an unsettled piece spoils none.
        for &(strip, _) in &all {
            let (old, new) = (room::role_of(&laid, strip), room::role_of(&after, strip));
            if has_room(&laid, strip) && old != Role::Den && has_room(&after, strip) {
                prop_assert_eq!(old, new, "{:?} by {:?}", strip, r);
            }
        }
        let settled = home.props.iter().any(|p| p.item == r.piece && p.settled);
        prop_assert!(settled || !spoils(&after, r.to.strip, r.piece), "{:?}", r);
        // Fits, clear of everything, where she'd use it.
        let mine = clear(&after, r.piece);
        prop_assert!(room::fits(buf, &at, &mine), "{:?}", r);
        prop_assert!(room::roomy(buf, &at, &mine), "{:?}", r);
        prop_assert!(!made.iter().any(|m| m.intersects(at.rect())), "{:?}", r);
        // Every other piece that showed still fits.
        for s in shown
            .iter()
            .filter(|s| s.item != r.piece && s.scrap.is_none())
        {
            let now = laid_of(&after, s.item);
            let theirs = clear(&after, s.item);
            prop_assert!(room::fits(buf, &now, &theirs), "{:?} by {:?}", s, r);
            let before = clear(&laid, s.item);
            if room::roomy(buf, s, &before) {
                prop_assert!(room::roomy(buf, &now, &theirs), "{:?} by {:?}", s, r);
            }
        }
        Ok(at)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]

        /// Every repair found mends its rule and breaks nothing (see
        /// [`mends`]); a turn keeps its anchor; a piece she has settled
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
        ) {
            let nooks = three(width, 8, n);
            let buf = screen(&nooks, width, 24, &text);
            let mut home = Home::default();
            for p in props {
                prop_assert!(home.add(p));
            }
            let shown = home.project(&buf, &nooks, &|_, _| false);
            let made = makeshift_on(&buf, &shown, &makeshift);
            let frame = Frame {
                buf: &buf,
                nooks: &nooks,
                blocked: &|_, _| false,
                shown: &shown,
                made: &made,
            };
            // The same home, later: resized, with more text and more
            // makeshift pieces.
            let later_nooks = three(later_width, 8, n);
            let mut later_text = later_text;
            later_text.extend(text.iter().copied());
            let later_buf = screen(&later_nooks, later_width, 24, &later_text);
            let mut later_home = home.clone();
            let later_shown = later_home.project(&later_buf, &later_nooks, &|_, _| false);
            let later_made = makeshift_on(&later_buf, &later_shown, &later_makeshift);
            let later = Frame {
                buf: &later_buf,
                nooks: &later_nooks,
                blocked: &|_, _| false,
                shown: &later_shown,
                made: &later_made,
            };
            let laid = home.clone().layout(&nooks);
            let was = broken(&laid, &strips(&nooks), &home);
            for target in &was {
                let found = search(&home, &frame, target);
                prop_assert!(found.examined <= CANDIDATES, "{}", found.examined);
                prop_assert!(found.repairs.len() <= REPAIRS);
                let order: Vec<(u8, u32)> = found.repairs.iter().map(|r| (r.tier, r.cost)).collect();
                prop_assert!(order.is_sorted(), "{:?}", order);
                let settled = |piece: Furniture| home.props.iter().any(|p| p.item == piece && p.settled);
                let mut delivered = false;
                for r in found.repairs.iter().rev() {
                    prop_assert_eq!(r.key, target.key);
                    prop_assert!(target.pieces.contains(&r.piece));
                    prop_assert!(check(&home, &frame, r), "{:?}", r);
                    let at = mends(&home, &nooks, &buf, &shown, &made, r)?;
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
                    if check(&later_home, &later, r) {
                        mends(&later_home, &later_nooks, &later_buf, &later_shown, &later_made, r)?;
                    }
                }
            }
        }
    }

    fn made_on(home: &Home, repair: &Repair, nooks: &[(Nook, Rect)]) -> Vec<Shown> {
        made(home, repair, nooks)
    }

    fn laid_of(at: &[Shown], item: Furniture) -> Shown {
        laid(at, item)
    }
}
