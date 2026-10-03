//! Her room: furniture she owns, placed around the panes.
//!
//! Props are stored **pane-relative** — a quiet pane's floor and a
//! position along it — and resolved against each frame, so resizes and
//! layout changes carry her room along. A prop stands on its pane's
//! bottom border over blank cells only; when it doesn't fit (the pane
//! is gone, too small, or has text where the prop would stand), it is
//! in the closet for that frame: hidden, still owned.

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;

use super::Rng;
use super::cells::{untouchable, width};
use super::graphics::strokes;
use super::sprite::Facing;

/// A piece of furniture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Furniture {
    /// A two-seat sofa (naps, TV from the sofa).
    Sofa,
    /// A CRT on a cabinet; the shopping channel plays on it.
    Tv,
    /// A single bed (proper sleep).
    Bed,
    /// A study desk (homework).
    Desk,
    /// A floor lamp (dark while she sleeps).
    Lamp,
    /// A bookshelf (reading).
    Bookshelf,
    /// A small fridge (snacks).
    Fridge,
    /// A cat bed (sometimes a cat is in it).
    CatBed,
    /// A potted plant (just for looks).
    Plant,
    /// A poster on the wall (just for looks).
    Poster,
}

impl Furniture {
    /// Every piece, in catalogue order.
    pub const ALL: [Self; 10] = [
        Self::Sofa,
        Self::Tv,
        Self::Bed,
        Self::Desk,
        Self::Lamp,
        Self::Bookshelf,
        Self::Fridge,
        Self::CatBed,
        Self::Plant,
        Self::Poster,
    ];

    /// Whether it's just for looks.
    pub(super) fn decor(self) -> bool {
        self.spec().offers.contains(&Offer::Decor)
    }

    /// Its row of the catalogue.
    pub(super) fn spec(self) -> &'static Spec {
        match self {
            Self::Sofa => &SOFA,
            Self::Tv => &TV,
            Self::Bed => &BED,
            Self::Desk => &DESK,
            Self::Lamp => &LAMP,
            Self::Bookshelf => &BOOKSHELF,
            Self::Fridge => &FRIDGE,
            Self::CatBed => &CAT_BED,
            Self::Plant => &PLANT,
            Self::Poster => &POSTER,
        }
    }
}

/// What a kind of piece is: one row of the catalogue.
#[derive(Debug)]
pub(super) struct Spec {
    /// A short name.
    pub name: &'static str,
    /// What she says buying it off the shopping channel.
    pub pitch: &'static str,
    /// Its size in cells (columns, rows), standing on a floor.
    pub footprint: (u16, u16),
    /// The ASCII drawing, facing right: one string per row, each exactly
    /// as wide as the footprint. Also the noise her goodbye bursts the
    /// line art into.
    pub ascii: &'static [&'static str],
    /// Its colour as text: in truecolor, and in the 16 colours.
    pub ink: (Color, Color),
    /// What it's for.
    pub uses: &'static [Use],
    /// What it brings to a room (which, with the rest of the room's
    /// pieces, says what the room is).
    pub offers: &'static [Offer],
    /// Where she is, using it from in it: the column (facing right) her
    /// box is centred on, and whether she faces away from the piece's
    /// own facing (at the desk, on a stool past its front).
    pub sit: Option<(i32, bool)>,
    /// How well it rests her (sitting, napping, sleeping on it), against
    /// a makeshift piece's or the floor's.
    pub comfort: f64,
    /// Out of its box, it hangs on the wall: the rows between the floor
    /// and its bottom row (none: it stands on the floor). Hung above the
    /// tallest piece that stands, it hangs over any of them.
    pub hang: Option<u16>,
    /// How much prettier it makes a room (decor; nothing else does).
    pub beauty: f64,
}

const SOFA: Spec = Spec {
    name: "sofa",
    pitch: "A sofa! I'll take it!",
    footprint: (9, 3),
    ascii: &[" .-----. ", "(|_____|)", " '     ' "],
    ink: (Color::Rgb(111, 161, 156), Color::Cyan),
    uses: &[Use::Lounge, Use::Nap],
    offers: &[Offer::Seat],
    sit: Some((4, false)),
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const TV: Spec = Spec {
    name: "TV",
    pitch: "A TV! I'll take it!",
    footprint: (6, 4),
    ascii: &["  \\/  ", ".----.", "|[  ]|", "|_::_|"],
    ink: (Color::Rgb(203, 191, 168), Color::Gray),
    uses: &[Use::Watch],
    offers: &[Offer::Screen],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const BED: Spec = Spec {
    name: "bed",
    pitch: "A bed... yes please!",
    footprint: (10, 3),
    ascii: &["|__       ", "|oo~~~~~~|", "|========|"],
    ink: (Color::Rgb(143, 179, 217), Color::LightBlue),
    uses: &[Use::Sleep],
    // Head at the headboard end.
    offers: &[Offer::Bed],
    sit: Some((3, false)),
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const DESK: Spec = Spec {
    name: "desk",
    pitch: "A desk. For homework.",
    footprint: (7, 3),
    ascii: &[" = u _/", "_______", "|   |=|"],
    ink: (Color::Rgb(192, 150, 100), Color::Yellow),
    uses: &[Use::Homework],
    // On a stool just past the desk's front, facing it.
    offers: &[Offer::Desk],
    sit: Some((7, true)),
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const LAMP: Spec = Spec {
    name: "lamp",
    pitch: "Ooh, a lamp!",
    footprint: (3, 4),
    ascii: &[" _ ", "/_\\", " | ", "_|_"],
    ink: (Color::Rgb(232, 195, 74), Color::LightYellow),
    uses: &[],
    offers: &[Offer::Light],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const BOOKSHELF: Spec = Spec {
    name: "bookshelf",
    pitch: "Books! I'll take it!",
    footprint: (5, 4),
    ascii: &["_____", "|IlI|", "|lII|", "|___|"],
    ink: (Color::Rgb(160, 120, 79), Color::Yellow),
    uses: &[Use::Read],
    offers: &[Offer::Books],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const FRIDGE: Spec = Spec {
    name: "fridge",
    pitch: "A fridge... for snacks!",
    footprint: (4, 4),
    ascii: &["____", "| .|", "|--|", "|_.|"],
    ink: (Color::Rgb(231, 236, 239), Color::White),
    uses: &[Use::Snack],
    offers: &[Offer::Cold],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const CAT_BED: Spec = Spec {
    name: "cat bed",
    pitch: "A cat bed! For a cat!",
    footprint: (4, 2),
    ascii: &["    ", "\\__/"],
    ink: (Color::Rgb(201, 69, 63), Color::Red),
    uses: &[Use::Pet],
    offers: &[Offer::Cat],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
};
const PLANT: Spec = Spec {
    name: "potted plant",
    pitch: "So green and leafy!",
    footprint: (3, 3),
    ascii: &["\\|/", "~Y~", "\\_/"],
    ink: (Color::Rgb(122, 166, 106), Color::Green),
    uses: &[],
    offers: &[Offer::Decor],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 1.0,
};
const POSTER: Spec = Spec {
    name: "poster",
    pitch: "It'd look nice up!",
    footprint: (4, 2),
    ascii: &[".--.", "|~~|"],
    ink: (Color::Rgb(247, 184, 154), Color::LightRed),
    uses: &[],
    offers: &[Offer::Decor],
    sit: None,
    comfort: 1.0,
    // Above her head and the tallest piece that stands (4 rows).
    hang: Some(4),
    beauty: 1.0,
};

/// The glyph at `(dx, dy)` of `item`'s ASCII drawing facing `facing`, if
/// that cell is drawn (spaces are not).
pub(super) fn glyph(item: Furniture, facing: Facing, dx: u16, dy: u16) -> Option<char> {
    let spec = item.spec();
    let (cols, _) = spec.footprint;
    let row = spec.ascii.get(usize::from(dy))?;
    let c = match facing {
        Facing::Right => row.chars().nth(usize::from(dx))?,
        Facing::Left => mirror(row.chars().nth(usize::from(cols - 1 - dx))?),
    };
    (c != ' ').then_some(c)
}

/// A delivery box, five wide and two tall, on the floor in the middle of
/// a `cols × rows` footprint.
const PARCEL: [&str; 2] = [" ___ ", "|_#_|"];

fn parcel_glyph(cols: u16, rows: u16, dx: u16, dy: u16) -> Option<char> {
    let left = (cols.saturating_sub(5)) / 2;
    let row = PARCEL.get(usize::from(dy.checked_sub(rows.checked_sub(2)?)?))?;
    let c = row.chars().nth(usize::from(dx.checked_sub(left)?))?;
    (c != ' ').then_some(c)
}

fn mirror(c: char) -> char {
    match c {
        '/' => '\\',
        '\\' => '/',
        '(' => ')',
        ')' => '(',
        '[' => ']',
        ']' => '[',
        other => other,
    }
}

/// A quiet pane she may furnish (the chat is too busy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Nook {
    /// The List (series), when short.
    List,
    /// The Users pane.
    Users,
    /// The Playlist pane.
    Playlist,
}

/// Something she does with a piece of furniture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Use {
    /// Sit on the sofa.
    Lounge,
    /// Nap on the sofa, hugging the cushion.
    Nap,
    /// Sleep in bed.
    Sleep,
    /// Homework at the desk (she nods off onto it).
    Homework,
    /// Sit beside the TV and watch it.
    Watch,
    /// Unpack it from its delivery box.
    Unpack,
    /// Sit beside the bookshelf reading.
    Read,
    /// A snack from the fridge.
    Snack,
    /// Pet the cat in the cat bed (who bites).
    Pet,
    /// Crumple torn-off text into a makeshift piece.
    Crumple,
}

impl Use {
    /// Whether she uses it from in it (sits on it, lies in it), rather
    /// than from beside it.
    pub fn inside(self) -> bool {
        !matches!(self, Use::Watch | Use::Read | Use::Snack | Use::Pet)
    }
}

/// A makeshift piece, among those she makes in a visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct MadeId(pub u32);

/// Which piece: one she owns (one of each kind), or one she made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum PieceRef {
    Real(Furniture),
    Made(MadeId),
}

/// Where she goes to use a piece: she walks to `x` on its floor (in
/// front of it, or beside the TV) and faces `facing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Seat {
    pub what: Use,
    pub item: Furniture,
    pub piece: PieceRef,
    pub x: i32,
    pub y: i32,
    pub facing: Facing,
}

impl Seat {
    /// It's makeshift furniture she made of text.
    pub fn makeshift(&self) -> bool {
        matches!(self.piece, PieceRef::Made(_))
    }
}

/// What a piece brings to a room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Offer {
    Seat,
    Screen,
    Bed,
    Desk,
    Light,
    Books,
    Cold,
    Cat,
    /// Just for looks (no room is named by it).
    Decor,
}

/// What a room is, from what's in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Role {
    Living,
    Bedroom,
    Study,
    Kitchen,
    /// Whatever's left.
    Den,
}

/// A room is `role` when its pieces offer everything `requires` and
/// nothing `forbids`.
struct RoleRule {
    role: Role,
    requires: &'static [Offer],
    forbids: &'static [Offer],
}

/// Which role a room has: the first row it meets (RimWorld's and
/// Oxygen Not Included's rooms, as a table).
const ROLES: [RoleRule; 5] = [
    RoleRule {
        role: Role::Living,
        requires: &[Offer::Screen, Offer::Seat],
        forbids: &[Offer::Bed],
    },
    RoleRule {
        role: Role::Bedroom,
        requires: &[Offer::Bed],
        forbids: &[Offer::Screen, Offer::Cold],
    },
    RoleRule {
        role: Role::Study,
        requires: &[Offer::Desk],
        forbids: &[],
    },
    RoleRule {
        role: Role::Kitchen,
        requires: &[Offer::Cold],
        forbids: &[Offer::Bed],
    },
    RoleRule {
        role: Role::Den,
        requires: &[],
        forbids: &[],
    },
];

/// The role of a room whose pieces offer `offers`.
pub(super) fn role(offers: &[Offer]) -> Role {
    ROLES
        .iter()
        .find(|rule| {
            rule.requires.iter().all(|o| offers.contains(o))
                && !rule.forbids.iter().any(|o| offers.contains(o))
        })
        .map_or(Role::Den, |rule| rule.role)
}

/// What a room that is `role` may not have in it (a den: anything).
pub(super) fn forbids(role: Role) -> &'static [Offer] {
    ROLES
        .iter()
        .find(|rule| rule.role == role)
        .map_or(&[], |rule| rule.forbids)
}

/// The role `strip` has as laid out: from what's out of its box there.
pub(super) fn role_of(layout: &[Shown], strip: Strip) -> Role {
    role_among(layout, strip, None)
}

/// The role `strip` would have as laid out without `piece` (whether a
/// piece completes its room, or spoils it).
pub(super) fn role_without(layout: &[Shown], strip: Strip, piece: Furniture) -> Role {
    role_among(layout, strip, Some(piece))
}

fn role_among(layout: &[Shown], strip: Strip, without: Option<Furniture>) -> Role {
    let offers: Vec<Offer> = layout
        .iter()
        .filter(|s| s.strip == Some(strip) && !s.boxed && s.scrap.is_none())
        .filter(|s| Some(s.item) != without)
        .flat_map(|s| s.item.spec().offers.iter().copied())
        .collect();
    role(&offers)
}

/// A floor her home can stand on: a quiet pane's bottom border, between
/// its walls. Its identity holds across frames, unlike the live
/// platforms, which text splits and renumbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum Strip {
    Bottom(Nook),
}

/// Which wall of its strip an anchor counts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum Side {
    Left,
    Right,
}

/// Where a piece stands along its strip: its near edge `offset` cells
/// from `side`'s wall. A resize keeps it that far from that wall.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) struct Anchor {
    pub side: Side,
    pub offset: u16,
}

/// The flap a delivery comes in through: wall column `x`, the rows
/// `rows.0..rows.1` just above the floor, in the wall on `side` of the
/// strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Flap {
    pub x: i32,
    pub rows: (i32, i32),
    pub side: Side,
}

/// A piece she owns, standing on `strip`; `boxed` until she unpacks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Prop {
    pub item: Furniture,
    pub strip: Strip,
    /// Its place along the strip, once it has stood there (an older
    /// record's piece has only `at` until its strip is first seen).
    pub anchor: Option<Anchor>,
    /// Thousandths of the way along its strip, as older builds read it;
    /// set whenever the anchor is.
    pub at: u16,
    pub facing: Facing,
    pub boxed: bool,
    /// She has set it where it stands (a delivery hasn't: it stands
    /// where it came in, until she moves it).
    pub settled: bool,
}

/// Where along its strip a piece is: standing on the floor, or hung on
/// the wall above it. Each lane is packed on its own (a hung piece may
/// hang over a standing one); the floor lane is the room's, and only it
/// moves the room (see [`Home::project`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Lane {
    Floor,
    Wall,
}

/// The rows between the floor and the bottom row of `item`, out of its
/// box or not: its hang, once it's out of a box (a parcel stands on the
/// floor); 0 for a piece that stands.
fn lift(item: Furniture, boxed: bool) -> u16 {
    if boxed {
        0
    } else {
        item.spec().hang.unwrap_or(0)
    }
}

impl Prop {
    /// Which lane of its strip it's in: a piece that hangs is on the
    /// wall once it's out of its box.
    pub fn lane(&self) -> Lane {
        if lift(self.item, self.boxed) > 0 {
            Lane::Wall
        } else {
            Lane::Floor
        }
    }

    /// The columns it takes along its strip, and the rows above the floor
    /// it needs clear (up to its top, hung or standing).
    fn needs(&self) -> (u16, u16) {
        let (cols, rows) = self.item.spec().footprint;
        (cols, rows + lift(self.item, self.boxed))
    }

    /// `item` `at` thousandths of the way along `nook`'s floor, not yet
    /// anchored.
    pub fn new(item: Furniture, nook: Nook, at: u16, facing: Facing) -> Self {
        Self {
            item,
            strip: Strip::Bottom(nook),
            anchor: None,
            at: at.min(1000),
            facing,
            boxed: false,
            settled: true,
        }
    }
}

/// A strip as it stands this frame: columns `from..to` between its
/// walls, the floor row, and the rows clear above the floor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Extent {
    pub from: i32,
    pub to: i32,
    pub floor: i32,
    pub rows: u16,
}

impl Extent {
    /// Whether a piece `cols` wide that needs `rows` clear above the
    /// floor (its own, and any it hangs above the floor) fits between
    /// its walls at all.
    pub(super) fn holds(&self, (cols, rows): (u16, u16)) -> bool {
        self.to - self.from >= i32::from(cols) && self.rows >= rows
    }

    /// The leftmost column a piece `cols` wide anchored at `anchor` would
    /// like, kept between the walls.
    pub(super) fn left(&self, anchor: Anchor, cols: u16) -> i32 {
        let cols = i32::from(cols);
        let offset = i32::from(anchor.offset);
        let want = match anchor.side {
            Side::Left => self.from + offset,
            Side::Right => self.to - offset - cols,
        };
        want.min(self.to - cols).max(self.from)
    }

    /// Where `at` thousandths of the way along puts a piece `cols` wide.
    fn share(&self, at: u16, cols: u16) -> i32 {
        let span = (self.to - self.from - i32::from(cols)).max(0);
        self.from + span * i32::from(at.min(1000)) / 1000
    }

    /// The anchor that keeps a piece `cols` wide at `left`: from the
    /// nearer wall, and its share of the way along.
    pub(super) fn pin(&self, left: i32, cols: u16) -> (Anchor, u16) {
        let span = (self.to - self.from - i32::from(cols)).max(0);
        let from_left = (left - self.from).clamp(0, span);
        let at = if span == 0 {
            0
        } else {
            (from_left * 1000 / span) as u16
        };
        let anchor = if 2 * from_left <= span {
            Anchor {
                side: Side::Left,
                offset: from_left as u16,
            }
        } else {
            Anchor {
                side: Side::Right,
                offset: (span - from_left) as u16,
            }
        };
        (anchor, at)
    }
}

/// The strips of the quiet panes this frame.
pub(super) fn strips(nooks: &[(Nook, Rect)]) -> Vec<(Strip, Extent)> {
    nooks
        .iter()
        .map(|&(nook, rect)| {
            (
                Strip::Bottom(nook),
                Extent {
                    from: i32::from(rect.x) + 1,
                    to: i32::from(rect.right()) - 1,
                    floor: i32::from(rect.bottom()) - 1,
                    rows: rect.height.saturating_sub(2),
                },
            )
        })
        .collect()
}

/// Where a piece anchored at `anchor`, `index`th among her pieces,
/// comes along its strip: those from the left wall, nearest first, then
/// those from the right, farthest first; of two the same distance from
/// one wall, the newer is nearer it (it came in last, through that
/// wall's flap, and pushed the other along).
pub(super) fn order_key(anchor: Anchor, index: usize) -> (u8, i32, i64) {
    let index = index as i64;
    match anchor.side {
        Side::Left => (0, i32::from(anchor.offset), -index),
        Side::Right => (1, -i32::from(anchor.offset), index),
    }
}

/// Where the pieces `(index, anchor, size)` stand on `extent`, as
/// `(index, left)`, if it holds them all: in anchor order along it (see
/// [`order_key`]), each where its anchor puts it unless that collides,
/// in which case the colliding ones stand side by side, still in order.
/// The order doesn't depend on the strip's width, so a resize never
/// reorders them, and a resize and back puts each where it was.
fn pack(pieces: &[(usize, Anchor, (u16, u16))], extent: Extent) -> Option<Vec<(usize, i32)>> {
    if !pieces.iter().all(|&(.., size)| extent.holds(size)) {
        return None;
    }
    let mut order: Vec<(usize, Anchor, (u16, u16))> = pieces.to_vec();
    order.sort_by_key(|&(index, anchor, _)| order_key(anchor, index));
    let mut placed: Vec<(usize, i32)> = order
        .iter()
        .map(|&(index, anchor, (cols, _))| (index, extent.left(anchor, cols)))
        .collect();
    let cols = |index: usize| {
        pieces
            .iter()
            .find(|&&(i, ..)| i == index)
            .map_or(0, |&(.., (cols, _))| i32::from(cols))
    };
    // Left to right, each clear of the one before; then right to left,
    // each back inside the far wall and clear of the one after.
    let mut edge = extent.from;
    for (index, left) in &mut placed {
        *left = (*left).max(edge);
        edge = *left + cols(*index);
    }
    let mut edge = extent.to;
    for (index, left) in placed.iter_mut().rev() {
        *left = (*left).min(edge - cols(*index));
        edge = *left;
    }
    (edge >= extent.from).then_some(placed)
}

/// Where those of `pieces` that `extent` holds stand on it, as [`pack`]
/// puts them: in anchor order, each that packs beside the ones kept
/// before it; one that doesn't (too tall, or no room left beside them)
/// is left out alone. For a wall, which never moves a room (see
/// [`Home::project`]).
fn pack_each(pieces: &[(usize, Anchor, (u16, u16))], extent: Extent) -> Vec<(usize, i32)> {
    let mut order: Vec<(usize, Anchor, (u16, u16))> = pieces
        .iter()
        .filter(|&&(.., needs)| extent.holds(needs))
        .copied()
        .collect();
    order.sort_by_key(|&(index, anchor, _)| order_key(anchor, index));
    let mut kept: Vec<(usize, Anchor, (u16, u16))> = Vec::new();
    let mut placed: Vec<(usize, i32)> = Vec::new();
    for piece in order {
        kept.push(piece);
        match pack(&kept, extent) {
            Some(at) => placed = at,
            None => {
                kept.pop();
            }
        }
    }
    placed
}

/// A prop as placed in this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub item: Furniture,
    pub facing: Facing,
    /// Still in its delivery box (drawn as the box).
    pub boxed: bool,
    /// The strip it stands on (none for makeshift pieces, which stand
    /// where she made them).
    pub strip: Option<Strip>,
    /// Its leftmost column.
    pub left: i32,
    /// The floor row it stands on (just below its footprint).
    pub floor: i32,
    /// Makeshift, crumpled out of torn-off text (never boxed).
    pub scrap: Option<super::scrap::Scrap>,
}

impl Shown {
    /// Which piece it is.
    pub fn piece(&self) -> PieceRef {
        match self.scrap {
            Some(scrap) => PieceRef::Made(scrap.id),
            None => PieceRef::Real(self.item),
        }
    }

    /// Its size in cells (columns, rows).
    pub fn size(&self) -> (u16, u16) {
        match self.scrap {
            Some(_) => super::scrap::footprint(self.item),
            None => self.item.spec().footprint,
        }
    }

    /// What she can do with it: unpack it while it's boxed, crumple it
    /// into shape while it's makeshift and still a heap; once it's in
    /// shape, a makeshift piece is for what the real one is.
    pub fn uses(&self) -> &'static [Use] {
        match (self.boxed, self.scrap) {
            (true, _) => &[Use::Unpack],
            (false, Some(scrap)) if !scrap.done() => &[Use::Crumple],
            (false, _) => self.item.spec().uses,
        }
    }

    /// The rows between the floor and its bottom row: its hang, while
    /// it's hung on the wall; 0 standing on the floor (and while it's
    /// boxed, or makeshift).
    pub fn lift(&self) -> u16 {
        match self.scrap {
            Some(_) => 0,
            None => lift(self.item, self.boxed),
        }
    }

    /// Which lane of its strip it's in.
    pub fn lane(&self) -> Lane {
        if self.lift() > 0 {
            Lane::Wall
        } else {
            Lane::Floor
        }
    }

    /// Its top row.
    fn top(&self) -> i32 {
        self.floor - i32::from(self.lift()) - i32::from(self.size().1)
    }

    /// Its footprint, above the floor (hung, as high as it hangs).
    pub fn rect(&self) -> Rect {
        let (cols, rows) = self.size();
        Rect::new(self.left as u16, self.top() as u16, cols, rows)
    }

    /// Its footprint and the floor beneath it, if it stands on the floor:
    /// nothing else may cover either. A hung piece's is its footprint.
    pub fn cover(&self) -> Rect {
        let rect = self.rect();
        match self.lane() {
            Lane::Floor => Rect::new(rect.x, rect.y, rect.width, rect.height + 1),
            Lane::Wall => rect,
        }
    }

    /// Where she'd be to use this piece for `what`: her anchor column and
    /// the way she faces. In the piece for most uses; for the TV, at
    /// `beside` (a standing spot next to it), facing it.
    pub fn seat(&self, what: Use, beside: i32) -> Seat {
        let (cols, _) = self.size();
        let cols = i32::from(cols);
        let mirrored = |col: i32| match self.facing {
            Facing::Right => self.left + col,
            Facing::Left => self.left + cols - 1 - col,
        };
        let flip = |f: Facing| match f {
            Facing::Right => Facing::Left,
            Facing::Left => Facing::Right,
        };
        let (x, facing) = match what {
            _ if self.scrap.is_some() => (mirrored(super::scrap::SEAT), self.facing),
            Use::Lounge | Use::Nap | Use::Sleep | Use::Homework => match self.item.spec().sit {
                Some((col, false)) => (mirrored(col), self.facing),
                Some((col, true)) => (mirrored(col), flip(self.facing)),
                None => (self.left + cols / 2, self.facing),
            },
            // In front of the box, bending over it.
            Use::Unpack | Use::Crumple => (self.left + cols / 2, self.facing),
            Use::Watch | Use::Read | Use::Snack | Use::Pet => (
                beside,
                if beside < self.left {
                    Facing::Right
                } else {
                    Facing::Left
                },
            ),
        };
        Seat {
            what,
            item: self.item,
            piece: self.piece(),
            x,
            y: self.floor,
            facing,
        }
    }

    /// The standing spots just beside it on its floor, her box clear of
    /// its footprint: left of it, then right.
    pub fn beside(&self) -> [i32; 2] {
        let half = super::sprite::WIDTH / 2;
        let rect = self.rect();
        [i32::from(rect.x) - half - 1, i32::from(rect.right()) + half]
    }

    /// Every footprint cell, with its ASCII glyph if drawn (the parcel's,
    /// while it's boxed).
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32, Option<char>)> + '_ {
        let (cols, rows) = self.size();
        let top = self.top();
        (0..rows).flat_map(move |dy| {
            (0..cols).map(move |dx| {
                let glyph = if let Some(scrap) = self.scrap {
                    scrap.cell(self.item, self.facing, dx, dy).map(|(c, _)| c)
                } else if self.boxed {
                    parcel_glyph(cols, rows, dx, dy)
                } else {
                    glyph(self.item, self.facing, dx, dy)
                };
                (self.left + i32::from(dx), top + i32::from(dy), glyph)
            })
        })
    }

    /// The TV's screen: the two cells inside its brackets.
    pub fn screen(&self) -> Option<[(i32, i32); 2]> {
        (self.item == Furniture::Tv && !self.boxed).then(|| {
            let y = self.floor - 2;
            [(self.left + 2, y), (self.left + 3, y)]
        })
    }
}

/// Everything she owns.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub(super) struct Home {
    pub props: Vec<Prop>,
}

impl Home {
    /// Whether she owns `item`.
    pub fn owns(&self, item: Furniture) -> bool {
        self.props.iter().any(|p| p.item == item)
    }

    /// Unpack `item`: it's out of its box. False if it wasn't boxed.
    pub fn unbox(&mut self, item: Furniture) -> bool {
        match self.props.iter_mut().find(|p| p.item == item && p.boxed) {
            Some(prop) => {
                prop.boxed = false;
                true
            }
            None => false,
        }
    }

    /// Whether anything is still in its box.
    pub fn boxed(&self) -> bool {
        self.props.iter().any(|p| p.boxed)
    }

    /// Her rooms: each strip her pieces stand on, and what it is from
    /// what's out of its box there.
    pub fn rooms(&self) -> Vec<(Strip, Role)> {
        self.furnished()
            .into_iter()
            .map(|strip| {
                let offers: Vec<Offer> = self
                    .props
                    .iter()
                    .filter(|p| p.strip == strip && !p.boxed)
                    .flat_map(|p| p.item.spec().offers.iter().copied())
                    .collect();
                (strip, role(&offers))
            })
            .collect()
    }

    /// The strips her pieces stand on, in the order she furnished them.
    fn furnished(&self) -> Vec<Strip> {
        let mut out: Vec<Strip> = Vec::new();
        for prop in &self.props {
            if !out.contains(&prop.strip) {
                out.push(prop.strip);
            }
        }
        out
    }

    /// Her pieces in `lane` of `strip`, as [`pack`] takes them (with the
    /// rows each needs clear above the floor), those that are anchored.
    fn on(&self, strip: Strip, lane: Lane) -> Vec<(usize, Anchor, (u16, u16))> {
        self.props
            .iter()
            .enumerate()
            .filter(|(_, p)| p.strip == strip && p.lane() == lane)
            .filter_map(|(i, p)| Some((i, p.anchor?, p.needs())))
            .collect()
    }

    /// Anchor every piece that has only a share of the way along (an
    /// older record's) where that share puts it, if its strip is here.
    fn pin_anchors(&mut self, strips: &[(Strip, Extent)]) {
        for prop in &mut self.props {
            if prop.anchor.is_none()
                && let Some(&(_, e)) = strips.iter().find(|(s, _)| *s == prop.strip)
            {
                let cols = prop.item.spec().footprint.0;
                prop.anchor = Some(e.pin(e.share(prop.at, cols), cols).0);
            }
        }
    }

    /// Where her pieces stand this frame, whether or not they show: each
    /// on its strip, in anchor order (see [`pack`]) in its lane, strip by
    /// strip in the order she furnished them and by index within one. A
    /// strip that's gone, or whose floor is too small to hold the pieces
    /// that stand on it, lays out none of them (this never moves
    /// anything; [`Home::project`] does); a piece its wall doesn't hold
    /// (too low, or too narrow beside the others hung there) is left out
    /// alone. Pure
    /// geometry: text over a piece doesn't take it out of the layout.
    /// An older record's piece is anchored where its share of the way
    /// along puts it, the first time its strip is here.
    pub fn layout(&mut self, nooks: &[(Nook, Rect)]) -> Vec<Shown> {
        let strips = strips(nooks);
        self.pin_anchors(&strips);
        self.laid_on(&strips)
    }

    /// [`Home::layout`] on `strips`, once every piece whose strip is
    /// there is anchored (a piece that isn't is left out).
    pub(super) fn laid_on(&self, strips: &[(Strip, Extent)]) -> Vec<Shown> {
        let mut out: Vec<Shown> = Vec::new();
        for strip in self.furnished() {
            let Some(&(_, e)) = strips.iter().find(|(s, _)| *s == strip) else {
                continue;
            };
            let Some(mut packed) = pack(&self.on(strip, Lane::Floor), e) else {
                continue;
            };
            packed.extend(pack_each(&self.on(strip, Lane::Wall), e));
            packed.sort_unstable();
            out.extend(
                packed.into_iter().filter_map(|(index, left)| {
                    Some(stand(self.props.get(index)?, strip, e, left))
                }),
            );
        }
        out
    }

    /// Where her pieces stand this frame. Each stands on its strip, in
    /// anchor order (see [`pack`]), if its cells are free (else it's in
    /// the closet this frame). A strip that's gone, or whose floor is too
    /// small to hold the pieces standing on it, has them all moved,
    /// together and in order, to the first strip whose floor holds them
    /// besides its own, every one of them on free cells, its hung pieces
    /// with them (where their share of the way along puts them there);
    /// with none, they're all in the closet (a strip that's gone with only
    /// hung pieces moves them to the first wall that holds them all, on
    /// free cells). Its wall never moves a room:
    /// a hung piece it doesn't hold is in the closet alone. An older
    /// record's piece is anchored where its share of the way along puts
    /// it, the first time its strip is here. `blocked` cells are never
    /// covered by a piece (protected rectangles, moved text). What shows
    /// is [`Home::layout`] less what doesn't fit.
    pub fn project(
        &mut self,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> Vec<Shown> {
        let strips = strips(nooks);
        self.pin_anchors(&strips);
        for strip in self.furnished() {
            let packs = strips
                .iter()
                .find(|(s, _)| *s == strip)
                .is_some_and(|&(_, e)| pack(&self.on(strip, Lane::Floor), e).is_some());
            if !packs {
                self.move_off(strip, buf, &strips, blocked);
            }
        }
        let mut shown: Vec<Shown> = Vec::new();
        for at in self.layout(nooks) {
            if fits(buf, &at, &|x, y| free(&shown, blocked, x, y)) {
                shown.push(at);
            }
        }
        shown
    }

    /// Move `strip`'s pieces, together, to the first other strip whose
    /// floor holds those that stand with its own, each on free cells;
    /// those hung go along, where their share of the way puts them. With
    /// only hung pieces, to the first whose wall holds them all that way.
    fn move_off(
        &mut self,
        strip: Strip,
        buf: &Buffer,
        strips: &[(Strip, Extent)],
        blocked: &dyn Fn(i32, i32) -> bool,
    ) {
        let leaving: Vec<usize> = (0..self.props.len())
            .filter(|&i| self.props.get(i).is_some_and(|p| p.strip == strip))
            .collect();
        let target = strips
            .iter()
            .filter(|&&(s, _)| s != strip)
            .find_map(|&(to, e)| {
                // Pieces never anchored, and hung ones, are anchored by
                // their share there.
                let mut moved = self.clone();
                for &i in &leaving {
                    let prop = moved.props.get_mut(i)?;
                    let cols = prop.item.spec().footprint.0;
                    let share = Some(e.pin(e.share(prop.at, cols), cols).0);
                    prop.anchor = match prop.lane() {
                        Lane::Floor => prop.anchor.or(share),
                        Lane::Wall => share,
                    };
                    prop.strip = to;
                }
                // Its floor holds those that stand, every one on free
                // cells; with none standing, its wall holds those hung,
                // the same way (else they'd only move into its closet).
                let lane = if leaving
                    .iter()
                    .any(|&i| moved.props.get(i).is_some_and(|p| p.lane() == Lane::Floor))
                {
                    Lane::Floor
                } else {
                    Lane::Wall
                };
                let packed = pack(&moved.on(to, lane), e)?;
                let all_free =
                    packed
                        .iter()
                        .filter(|(i, _)| leaving.contains(i))
                        .all(|&(i, left)| {
                            moved.props.get(i).is_some_and(|p| {
                                fits(buf, &stand(p, to, e, left), &|x, y| !blocked(x, y))
                            })
                        });
                all_free.then_some((to, e, moved, packed))
            });
        let Some((to, e, mut moved, packed)) = target else {
            return;
        };
        tracing::info!(from = ?strip, ?to, "houseguest: her pieces moved");
        for (i, left) in packed {
            if leaving.contains(&i)
                && let Some(prop) = moved.props.get_mut(i)
            {
                prop.at = e.pin(left, prop.item.spec().footprint.0).1;
            }
        }
        *self = moved;
    }

    /// Where `item` could go this frame: anywhere on a strip it fits,
    /// clear of what's `shown`, chosen at random (a stage gift: it
    /// stands where it's settled).
    pub fn spot(
        &self,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        item: Furniture,
        rng: &mut Rng,
    ) -> Option<Prop> {
        let facing = if rng.below(2) == 0 {
            Facing::Right
        } else {
            Facing::Left
        };
        let cols = item.spec().footprint.0;
        let spots: Vec<Prop> = strips(nooks)
            .into_iter()
            .flat_map(|(strip, e)| {
                (0..=10).filter_map(move |step| {
                    let at = step * 100;
                    let prop = Prop {
                        item,
                        strip,
                        anchor: Some(e.pin(e.share(at, cols), cols).0),
                        at,
                        facing,
                        boxed: false,
                        settled: true,
                    };
                    e.holds(prop.needs())
                        .then(|| (prop, stand(&prop, strip, e, e.share(at, cols))))
                })
            })
            .filter(|(_, at)| {
                let clear = |x: i32, y: i32| free(shown, blocked, x, y);
                fits(buf, at, &clear) && roomy(buf, at, &clear)
            })
            .map(|(prop, _)| prop)
            .collect();
        spots.get(rng.below(spots.len() as u64) as usize).copied()
    }

    /// Where a delivery of `item` comes in this frame: through a flap in
    /// one of her strips' walls, preferring a wall at the screen's edge,
    /// to stand against it facing into the room, unsettled (she never
    /// chose where it stands). The pieces already on
    /// that strip make way, packed in order, but only where every one of
    /// them that shows still fits, the piece fits on blank, free cells,
    /// and she'd fit to unpack and use it. A piece that hangs must fit
    /// both ways: boxed, standing on the floor to be unpacked, and
    /// hung on the wall above.
    pub fn doorstep(
        &self,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        item: Furniture,
    ) -> Option<(Prop, Flap)> {
        let screen = buf.area;
        let mut walls: Vec<(bool, Strip, Extent, Side)> = Vec::new();
        for (&(_, rect), (strip, e)) in nooks.iter().zip(strips(nooks)) {
            walls.push((rect.right() == screen.right(), strip, e, Side::Right));
            walls.push((rect.x == screen.x, strip, e, Side::Left));
        }
        // The screen's edge first; otherwise in pane order.
        walls.sort_by_key(|&(edge, ..)| !edge);
        walls.into_iter().find_map(|(_, strip, e, side)| {
            let prop = Prop {
                item,
                strip,
                anchor: Some(Anchor { side, offset: 0 }),
                at: match side {
                    Side::Left => 0,
                    Side::Right => 1000,
                },
                facing: match side {
                    Side::Left => Facing::Right,
                    Side::Right => Facing::Left,
                },
                boxed: false,
                // She never chose where it stands.
                settled: false,
            };
            let at = match item.spec().hang {
                None => self.admits(buf, shown, blocked, e, prop)?,
                Some(_) => {
                    let parcel = Prop {
                        boxed: true,
                        ..prop
                    };
                    let at = self.admits(buf, shown, blocked, e, parcel)?;
                    self.admits(buf, shown, blocked, e, prop)?;
                    at
                }
            };
            let x = match side {
                Side::Left => e.from - 1,
                Side::Right => e.to,
            };
            Some((
                Prop {
                    boxed: true,
                    ..prop
                },
                Flap {
                    x,
                    rows: (at.floor - 2, at.floor),
                    side,
                },
            ))
        })
    }

    /// Where `prop`, new, would stand on its strip (`e` this frame), if
    /// the pieces in its lane there make way for it, packed in order:
    /// every one of them that shows still fits, and it fits on blank,
    /// free cells where she'd fit to use it.
    fn admits(
        &self,
        buf: &Buffer,
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        e: Extent,
        prop: Prop,
    ) -> Option<Shown> {
        let (strip, lane) = (prop.strip, prop.lane());
        let mut with = self.clone();
        with.props.push(prop);
        let new = with.props.len() - 1;
        let packed: Vec<(usize, Shown)> = pack(&with.on(strip, lane), e)?
            .into_iter()
            .filter_map(|(i, left)| Some((i, stand(with.props.get(i)?, strip, e, left))))
            .collect();
        // What shows anywhere else stays where it is: on other strips, and
        // in the strip's other lane.
        let others: Vec<Shown> = shown
            .iter()
            .filter(|s| s.strip != Some(strip) || s.lane() != lane)
            .copied()
            .collect();
        let fits_here = |i: usize, at: &Shown| {
            let clear = |x: i32, y: i32| {
                free(&others, blocked, x, y)
                    && !packed
                        .iter()
                        .any(|(j, s)| *j != i && s.rect().contains((x as u16, y as u16).into()))
            };
            fits(buf, at, &clear) && (i != new || roomy(buf, at, &clear))
        };
        let all_fit = packed.iter().all(|(i, at)| {
            let was = with.props.get(*i).map(|p| p.item);
            let showing = shown
                .iter()
                .any(|s| Some(s.item) == was && s.scrap.is_none());
            (*i != new && !showing) || fits_here(*i, at)
        });
        let at = packed.iter().find(|(i, _)| *i == new).map(|(_, at)| *at)?;
        all_fit.then_some(at)
    }

    /// Take ownership of `prop`, unless she already has one (then this
    /// returns false: one of each).
    pub fn add(&mut self, prop: Prop) -> bool {
        if self.owns(prop.item) {
            return false;
        }
        self.props.push(prop);
        true
    }
}

/// `prop` as it stands at `left` on `strip`.
fn stand(prop: &Prop, strip: Strip, extent: Extent, left: i32) -> Shown {
    Shown {
        item: prop.item,
        facing: prop.facing,
        boxed: prop.boxed,
        strip: Some(strip),
        left,
        floor: extent.floor,
        scrap: None,
    }
}

/// How far apart a seat and a screen may be, in cells between them, for
/// her to watch from the one.
pub(super) const FACING_GAP: std::ops::RangeInclusive<i32> = 2..=14;

/// Whether `seat` faces `screen`, for watching: both on one strip, a
/// [`FACING_GAP`] apart, the seat turned toward the screen. Judged on the
/// strip, not the live floor, so a name that splits the floor between
/// them doesn't stop her. The screen is seen from the front, so which
/// way it is turned doesn't count.
pub(super) fn faces(seat: &Shown, screen: &Shown) -> bool {
    let (a, b) = (seat.rect(), screen.rect());
    let (gap, toward) = if a.x < b.x {
        (i32::from(b.x) - i32::from(a.right()), Facing::Right)
    } else {
        (i32::from(a.x) - i32::from(b.right()), Facing::Left)
    };
    seat.strip.is_some()
        && seat.strip == screen.strip
        && FACING_GAP.contains(&gap)
        && seat.facing == toward
}

/// Whether she'd fit to use `at` every way it's used (a new piece is
/// never set down where she couldn't): for a use in it, her box at its
/// seat, beyond the piece itself; for a use beside it, her box at one of
/// the spots beside it, standing on a line. Her box must be blank and
/// `clear`.
pub(super) fn roomy(buf: &Buffer, at: &Shown, clear: &dyn Fn(i32, i32) -> bool) -> bool {
    let half = super::sprite::WIDTH / 2;
    let rect = at.rect();
    let cell = |x: i32, y: i32| {
        let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
            return None;
        };
        buf.cell((ux, uy))
            .filter(|c| !untouchable(c))
            .map(|c| (ux, uy, c))
    };
    let fits = |x: i32, y: i32| {
        (1..=super::sprite::HEIGHT).all(|dy| {
            (-half..=half).all(|dx| {
                cell(x + dx, y - dy).is_some_and(|(ux, uy, c)| {
                    rect.contains((ux, uy).into())
                        || clear(x + dx, y - dy) && c.symbol().trim().is_empty()
                })
            })
        })
    };
    let floor = |x: i32, y: i32| {
        (-half..=half).all(|dx| {
            cell(x + dx, y).is_some_and(|(.., c)| {
                c.symbol()
                    .chars()
                    .next()
                    .is_some_and(|c| strokes(c).is_some())
            })
        })
    };
    at.uses().iter().all(|&what| {
        if what.inside() {
            let seat = at.seat(what, 0);
            fits(seat.x, seat.y)
        } else {
            at.beside()
                .into_iter()
                .any(|x| fits(x, at.floor) && floor(x, at.floor))
        }
    })
}

/// Whether `(x, y)` is neither `blocked` nor under a shown piece.
fn free(shown: &[Shown], blocked: &dyn Fn(i32, i32) -> bool, x: i32, y: i32) -> bool {
    !blocked(x, y)
        && !shown
            .iter()
            .any(|s| s.rect().contains((x as u16, y as u16).into()))
}

/// Whether `at` stands on unbroken line glyphs (hung, it needs none)
/// with only blank, `clear` cells in its footprint (image cells and wide
/// glyphs' halves are never blank).
pub(super) fn fits(buf: &Buffer, at: &Shown, clear: &dyn Fn(i32, i32) -> bool) -> bool {
    let cell = |x: i32, y: i32| {
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
            return None;
        };
        buf.cell((x, y)).filter(|c| !untouchable(c))
    };
    let body = at.cells().all(|(x, y, _)| {
        clear(x, y)
            && cell(x, y).is_some_and(|c| c.symbol().trim().is_empty())
            && cell(x - 1, y).is_none_or(|c| width(c) < 2)
    });
    let (cols, _) = at.size();
    // Hung, it needs no floor beneath it.
    let floor = at.lane() == Lane::Wall
        || (at.left..at.left + i32::from(cols)).all(|x| {
            clear(x, at.floor)
                && cell(x, at.floor)
                    .and_then(|c| c.symbol().chars().next())
                    .is_some_and(|c| strokes(c).is_some())
        });
    body && floor
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn every_ascii_drawing_fills_its_footprint_exactly() {
        for item in Furniture::ALL {
            let (cols, rows) = item.spec().footprint;
            let art = item.spec().ascii;
            assert_eq!(art.len(), usize::from(rows), "{item:?}");
            for row in art {
                assert_eq!(row.chars().count(), usize::from(cols), "{item:?}: {row:?}");
            }
        }
    }

    fn pane(rows: &[&str]) -> Buffer {
        Buffer::with_lines(rows.iter().copied())
    }

    const USERS: [&str; 6] = [
        "┌Users───────────┐",
        "│                │",
        "│                │",
        "│                │",
        "│                │",
        "└────────────────┘",
    ];

    fn prop(item: Furniture, at: u16) -> Prop {
        Prop::new(item, Nook::Users, at, Facing::Right)
    }

    fn home(nook: Nook, props: &[Prop]) -> Home {
        let mut home = Home::default();
        for &p in props {
            assert!(home.add(Prop {
                strip: Strip::Bottom(nook),
                ..p
            }));
        }
        home
    }

    /// An empty pane `width × height` titled `title`, its rows.
    fn empty(title: &str, width: usize, height: usize) -> Vec<String> {
        let mut rows = vec![format!(
            "┌{title}{}┐",
            "─".repeat(width - 2 - title.chars().count())
        )];
        rows.extend((0..height - 2).map(|_| format!("│{}│", " ".repeat(width - 2))));
        rows.push(format!("└{}┘", "─".repeat(width - 2)));
        rows
    }

    #[test]
    fn a_prop_stands_on_its_panes_floor() {
        let buf = pane(&USERS);
        let nooks = [(Nook::Users, buf.area)];
        let shown =
            home(Nook::Users, &[prop(Furniture::Sofa, 0)]).project(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].rect(), Rect::new(1, 2, 9, 3));
        let far =
            home(Nook::Users, &[prop(Furniture::Sofa, 1000)]).project(&buf, &nooks, &|_, _| false);
        assert_eq!(far[0].rect().right(), 17);
    }

    #[test]
    fn text_or_a_blocked_cell_sends_it_to_the_closet() {
        let mut rows = USERS;
        rows[3] = "│   hi           │";
        let buf = pane(&rows);
        let nooks = [(Nook::Users, buf.area)];
        let mut room = home(Nook::Users, &[prop(Furniture::Sofa, 0)]);
        assert!(room.project(&buf, &nooks, &|_, _| false).is_empty());
        let clean = pane(&USERS);
        assert!(
            room.project(&clean, &nooks, &|x, y| (x, y) == (5, 4))
                .is_empty()
        );
        assert!(
            room.project(&clean, &[], &|_, _| false).is_empty(),
            "no pane"
        );
        assert_eq!(
            room.props[0].strip,
            Strip::Bottom(Nook::Users),
            "text doesn't move a piece"
        );
    }

    #[test]
    fn text_over_a_piece_closets_it_but_leaves_it_laid_out() {
        let mut rows = USERS;
        rows[3] = "│   hi           │";
        let buf = pane(&rows);
        let nooks = [(Nook::Users, buf.area)];
        let mut room = home(
            Nook::Users,
            &[prop(Furniture::Sofa, 0), prop(Furniture::Lamp, 1000)],
        );
        let shown = room.project(&buf, &nooks, &|_, _| false);
        let items: Vec<Furniture> = shown.iter().map(|s| s.item).collect();
        assert_eq!(items, [Furniture::Lamp], "the text closets the sofa");
        let laid = room.layout(&nooks);
        let items: Vec<Furniture> = laid.iter().map(|s| s.item).collect();
        assert_eq!(items, [Furniture::Sofa, Furniture::Lamp]);
        assert_eq!(laid[0].rect(), Rect::new(1, 2, 9, 3), "where it'd stand");
        assert_eq!(laid[1], shown[0]);
        assert!(room.layout(&[]).is_empty(), "no pane, no layout");
    }

    #[test]
    fn a_strip_too_small_moves_its_pieces_together_or_not_at_all() {
        // Two panes side by side: Users 18 wide, Playlist 30 wide.
        let mut rows: Vec<String> = Vec::new();
        for (i, row) in USERS.iter().enumerate() {
            let other = match i {
                0 => "┌Playlist────────────────────┐".to_string(),
                5 => "└────────────────────────────┘".to_string(),
                _ => format!("│{}│", " ".repeat(28)),
            };
            rows.push(format!("{row}{other}"));
        }
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        // Narrower than the sofa and TV side by side.
        let users = Rect::new(0, 0, 14, 6);
        let playlist = Rect::new(18, 0, 30, 6);
        let wide = Rect::new(0, 0, 48, 6);
        // Sofa and TV furnished in a wide Users pane.
        let mut room = home(
            Nook::Users,
            &[prop(Furniture::Sofa, 0), prop(Furniture::Tv, 1000)],
        );
        let big = Buffer::with_lines(empty("Users", 48, 6).iter().map(String::as_str));
        assert_eq!(
            room.project(&big, &[(Nook::Users, wide)], &|_, _| false)
                .len(),
            2
        );
        // Users shrinks: both no longer fit there, so they move to the
        // Playlist, together, the sofa still left of the TV.
        let nooks = [(Nook::Users, users), (Nook::Playlist, playlist)];
        let shown = room.project(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert!(
            shown
                .iter()
                .all(|s| s.strip == Some(Strip::Bottom(Nook::Playlist)))
        );
        assert!(shown[0].left < shown[1].left, "{shown:?}");
        assert!(
            room.props
                .iter()
                .all(|p| p.strip == Strip::Bottom(Nook::Playlist))
        );
        // Nowhere holds both: they're all in the closet.
        let shown = room.project(&buf, &[(Nook::Users, users)], &|_, _| false);
        assert!(shown.is_empty(), "{shown:?}");
    }

    /// A strip that's gone sends its pieces to one where other pieces
    /// already stand, if it holds them all.
    #[test]
    fn moved_pieces_may_join_another_rooms_strip() {
        let rows: Vec<String> = empty("Users", 40, 6)
            .into_iter()
            .zip(empty("Playlist", 40, 6))
            .map(|(a, b)| a + &b)
            .collect();
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [
            (Nook::Users, Rect::new(0, 0, 40, 6)),
            (Nook::Playlist, Rect::new(40, 0, 40, 6)),
        ];
        let mut room = home(Nook::Users, &[prop(Furniture::Sofa, 0)]);
        assert!(room.add(Prop::new(
            Furniture::Bed,
            Nook::Playlist,
            1000,
            Facing::Right
        )));
        assert_eq!(room.project(&buf, &nooks, &|_, _| false).len(), 2);
        let shown = room.project(&buf, &nooks[1..], &|_, _| false);
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert!(
            room.props
                .iter()
                .all(|p| p.strip == Strip::Bottom(Nook::Playlist))
        );
    }

    /// Pieces whose places along the floor collide stand side by side,
    /// in order along it, while the pane holds them all; and keep their
    /// own places where they don't collide.
    #[test]
    fn colliding_pieces_stand_side_by_side() {
        let buf = pane(&[
            "┌Users───────────────────────┐",
            "│                            │",
            "│                            │",
            "│                            │",
            "│                            │",
            "└────────────────────────────┘",
        ]);
        let nooks = [(Nook::Users, buf.area)];
        // Shares of the way along, and whether the sofa comes first: each is
        // anchored from its nearer wall; level with the sofa at a wall, the
        // TV (the newer) is nearer it.
        for (sofa, tv, first) in [
            (500, 500, true),
            (400, 450, true),
            (1000, 900, false),
            (0, 0, false),
        ] {
            let mut room = home(
                Nook::Users,
                &[prop(Furniture::Sofa, sofa), prop(Furniture::Tv, tv)],
            );
            let shown = room.project(&buf, &nooks, &|_, _| false);
            assert_eq!(shown.len(), 2, "sofa {sofa}, tv {tv}: {shown:?}");
            assert!(!shown[0].rect().intersects(shown[1].rect()), "{shown:?}");
            let left = |item| shown.iter().find(|s| s.item == item).map(|s| s.left);
            assert_eq!(
                left(Furniture::Sofa) < left(Furniture::Tv),
                first,
                "in order along the floor: {shown:?}"
            );
        }
        // Apart, each keeps its own place.
        let mut room = home(
            Nook::Users,
            &[prop(Furniture::Sofa, 0), prop(Furniture::Tv, 1000)],
        );
        let shown = room.project(&buf, &nooks, &|_, _| false);
        assert_eq!(shown[0].left, 1);
        assert_eq!(shown[1].rect().right(), 29);
    }

    /// A new piece goes anywhere on a strip it fits, clear of what
    /// stands, and where she'd fit to use it.
    #[test]
    fn a_new_piece_goes_where_it_fits() {
        let mut rows = empty("Users", 40, 7);
        for (row, other) in rows.iter_mut().zip(empty("Playlist", 40, 7)) {
            row.push_str(&other);
        }
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [
            (Nook::Users, Rect::new(0, 0, 40, 7)),
            (Nook::Playlist, Rect::new(40, 0, 40, 7)),
        ];
        let mut room = home(Nook::Users, &[prop(Furniture::Sofa, 0)]);
        let shown = room.project(&buf, &nooks, &|_, _| false);
        let mut strips = std::collections::HashSet::new();
        for seed in 0..40 {
            let bed = room
                .spot(
                    &buf,
                    &nooks,
                    &shown,
                    &|_, _| false,
                    Furniture::Bed,
                    &mut Rng(seed),
                )
                .expect("room for a bed");
            strips.insert(bed.strip);
            let mut with = room.clone();
            assert!(with.add(bed));
            let both = with.project(&buf, &nooks, &|_, _| false);
            assert_eq!(both.len(), 2, "{both:?}");
            assert!(!both[0].rect().intersects(both[1].rect()));
        }
        assert_eq!(strips.len(), 2, "either pane");
        assert!(!room.add(prop(Furniture::Sofa, 500)), "one of each");
    }

    /// A room is what its pieces make it, by the first row of the table
    /// it meets; a piece still in its box doesn't count yet.
    #[test]
    fn roles_come_from_contents() {
        use Furniture::*;
        for (items, want) in [
            (&[Sofa, Tv][..], Role::Living),
            (&[Sofa, Tv, CatBed, Lamp, Desk], Role::Living),
            (&[Bed], Role::Bedroom),
            (&[Bed, Desk, Lamp, Bookshelf], Role::Bedroom),
            (&[Desk, Bookshelf], Role::Study),
            (&[Fridge], Role::Kitchen),
            (&[Fridge, Desk], Role::Study),
            (&[Sofa, Tv, Bed], Role::Den),
            (&[Bed, Fridge], Role::Den),
            (&[Sofa], Role::Den),
            (&[Lamp, CatBed], Role::Den),
            // Decor names no room, and changes none.
            (&[Sofa, Tv, Plant, Poster], Role::Living),
            (&[Bed, Poster], Role::Bedroom),
            (&[Fridge, Plant], Role::Kitchen),
            (&[Plant, Poster], Role::Den),
        ] {
            let pieces: Vec<Prop> = items.iter().map(|&item| prop(item, 0)).collect();
            assert_eq!(
                home(Nook::Users, &pieces).rooms(),
                [(Strip::Bottom(Nook::Users), want)],
                "{items:?}"
            );
        }
        let mut boxed = home(Nook::Users, &[prop(Sofa, 0)]);
        assert!(boxed.add(Prop {
            boxed: true,
            ..prop(Tv, 1000)
        }));
        assert_eq!(boxed.rooms(), [(Strip::Bottom(Nook::Users), Role::Den)]);
        assert!(boxed.unbox(Tv));
        assert_eq!(boxed.rooms(), [(Strip::Bottom(Nook::Users), Role::Living)]);
    }

    use proptest::prelude::*;

    /// The pieces that stand on the floor.
    fn standing() -> Vec<Furniture> {
        Furniture::ALL
            .into_iter()
            .filter(|f| f.spec().hang.is_none())
            .collect()
    }

    /// Some of her pieces that stand on the floor, each anchored
    /// somewhere (or, from an older record, only a share of the way
    /// along), facing either way.
    fn pieces() -> impl Strategy<Value = Vec<Prop>> {
        proptest::sample::subsequence(standing(), 1..=5).prop_flat_map(placed)
    }

    /// Some of her pieces that stand, and a poster hung among them
    /// (newer or older than any of them), each placed as [`pieces`].
    fn decorated() -> impl Strategy<Value = Vec<Prop>> {
        (
            proptest::sample::subsequence(standing(), 0..=4),
            any::<proptest::sample::Index>(),
        )
            .prop_flat_map(|(mut items, at)| {
                items.insert(at.index(items.len() + 1), Furniture::Poster);
                placed(items)
            })
    }

    /// `items`, each anchored somewhere (or only a share of the way
    /// along), facing either way.
    fn placed(items: Vec<Furniture>) -> impl Strategy<Value = Vec<Prop>> {
        {
            let n = items.len();
            (
                Just(items),
                proptest::collection::vec(
                    (
                        any::<bool>(),
                        0u16..40,
                        proptest::option::of(0u16..=1000),
                        any::<bool>(),
                    ),
                    n,
                ),
            )
                .prop_map(|(items, places)| {
                    items
                        .into_iter()
                        .zip(places)
                        .map(|(item, (right, offset, share, left))| {
                            let facing = if left { Facing::Left } else { Facing::Right };
                            let mut prop = Prop::new(item, Nook::Users, share.unwrap_or(0), facing);
                            if share.is_none() {
                                prop.anchor = Some(Anchor {
                                    side: if right { Side::Right } else { Side::Left },
                                    offset,
                                });
                            }
                            prop
                        })
                        .collect()
                })
        }
    }

    /// A Users pane `width` wide, six tall, with `text` cells along its
    /// first row above the floor.
    fn users(width: u16, text: &[u16]) -> (Buffer, [(Nook, Rect); 1]) {
        let mut buf = Buffer::with_lines(
            empty("Users", usize::from(width), 6)
                .iter()
                .map(String::as_str),
        );
        for &x in text {
            if x > 0 && x + 1 < width {
                buf[(x, 4)].set_symbol("x");
            }
        }
        (buf, [(Nook::Users, Rect::new(0, 0, width, 6))])
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(256)))]

        /// Packing keeps anchor order and the walls, and never overlaps;
        /// a resize that still holds them all, and back, puts every
        /// piece where it was; and a piece shows only over blank,
        /// unblocked cells on an unbroken floor.
        #[test]
        fn packing_keeps_order_and_a_resize_and_back_restores(
            props in pieces(),
            wide in 30u16..90,
            narrow in 20u16..90,
            text in proptest::collection::vec(1u16..90, 0..4),
            blocked in proptest::option::of(1u16..90),
        ) {
            let mut room = Home { props };
            let (buf, nooks) = users(wide, &[]);
            let first = room.project(&buf, &nooks, &|_, _| false);
            let pinned = room.clone();
            prop_assert!(room.props.iter().all(|p| p.anchor.is_some()));
            // What shows is the layout, in its order, less what doesn't
            // fit; laying out moves nothing.
            let laid = room.layout(&nooks);
            prop_assert_eq!(&room, &pinned);
            let subsequence = |shown: &[Shown], laid: &[Shown]| {
                let mut rest = laid.iter();
                shown.iter().all(|s| rest.any(|l| l == s))
            };
            prop_assert!(subsequence(&first, &laid), "{:?} / {:?}", first, laid);
            let (small, at_small) = users(narrow, &[]);
            // Shown in anchor order, inside the walls, apart.
            let order = |s: &Shown| {
                let index = room.props.iter().position(|p| p.item == s.item);
                index.and_then(|i| Some(order_key(room.props.get(i)?.anchor?, i)))
            };
            let mut along = first.clone();
            along.sort_by_key(|s| s.left);
            for pair in along.windows(2) {
                prop_assert!(pair[0].rect().right() <= pair[1].rect().x, "{:?}", along);
                prop_assert!(order(&pair[0]) < order(&pair[1]), "{:?}", along);
            }
            for s in &first {
                prop_assert!(s.left >= 1 && s.rect().right() < wide, "{:?}", s);
            }
            // A resize, and back: with no other strip to go to, nothing
            // moves (however small), and every piece shows where it did.
            let _ = room.project(&small, &at_small, &|_, _| false);
            prop_assert_eq!(&room, &pinned, "a resize moved a piece");
            let again = room.project(&buf, &nooks, &|_, _| false);
            prop_assert_eq!(&again, &first);
            // Text and blocked cells only closet what they'd cover.
            let mut room = pinned;
            let (noisy, _) = users(wide, &text);
            let block = |x: i32, _: i32| blocked.is_some_and(|b| i32::from(b) == x);
            let noisy_shown = room.project(&noisy, &nooks, &block);
            for s in &noisy_shown {
                prop_assert!(fits(&noisy, s, &|x, y| !block(x, y)), "{:?}", s);
                prop_assert!(first.contains(s), "text never moves a piece: {:?}", s);
            }
            // Text closets pieces from what shows, never from the layout.
            let noisy_laid = room.layout(&nooks);
            prop_assert_eq!(&noisy_laid, &laid);
            prop_assert!(subsequence(&noisy_shown, &noisy_laid));
        }
    }

    /// Two panes side by side: Users `width × height` and a Playlist 40
    /// wide and 12 tall, with `text` cells inside Users.
    fn two_panes(width: u16, height: u16, text: &[(u16, u16)]) -> (Buffer, [(Nook, Rect); 2]) {
        let tall = height.max(12);
        let mut rows = Vec::new();
        let users = empty("Users", usize::from(width), usize::from(height));
        let playlist = empty("Playlist", 40, 12);
        for y in 0..usize::from(tall) {
            let a = users
                .get(y)
                .cloned()
                .unwrap_or_else(|| " ".repeat(usize::from(width)));
            let b = playlist.get(y).cloned().unwrap_or_else(|| " ".repeat(40));
            rows.push(a + &b);
        }
        let mut buf = Buffer::with_lines(rows.iter().map(String::as_str));
        for &(x, y) in text {
            if x > 0 && x + 1 < width && y > 0 && y + 1 < height {
                buf[(x, y)].set_symbol("x");
            }
        }
        (
            buf,
            [
                (Nook::Users, Rect::new(0, 0, width, height)),
                (Nook::Playlist, Rect::new(width, 0, 40, 12)),
            ],
        )
    }

    /// Every piece that hangs hangs above the tallest piece that stands
    /// (and so above any it hangs over), and above her head.
    #[test]
    fn hung_pieces_clear_every_standing_piece() {
        let tallest = standing()
            .into_iter()
            .map(|f| f.spec().footprint.1)
            .max()
            .unwrap();
        assert!(tallest >= super::super::sprite::HEIGHT as u16);
        for item in Furniture::ALL {
            if let Some(hang) = item.spec().hang {
                assert!(hang >= tallest, "{item:?} hangs {hang}, under {tallest}");
                assert!(item.spec().uses.is_empty(), "{item:?} is out of reach");
            }
        }
    }

    /// A poster hangs on the wall, over the sofa: above it, apart from
    /// it, with nothing beneath it but the wall.
    #[test]
    fn a_poster_hangs_over_the_sofa() {
        let rows = empty("Users", 20, 9);
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, buf.area)];
        let mut room = home(
            Nook::Users,
            &[prop(Furniture::Sofa, 0), prop(Furniture::Poster, 0)],
        );
        let shown = room.project(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 2, "{shown:?}");
        let (sofa, poster) = (shown[0], shown[1]);
        assert_eq!(poster.item, Furniture::Poster);
        assert_eq!((sofa.lane(), poster.lane()), (Lane::Floor, Lane::Wall));
        assert_eq!((sofa.left, poster.left), (1, 1), "each against the wall");
        assert_eq!(poster.rect(), Rect::new(1, 2, 4, 2));
        assert_eq!(poster.cover(), poster.rect(), "no floor beneath it");
        assert!(!poster.cover().intersects(sofa.cover()));
        assert!(
            poster
                .cells()
                .all(|(x, y, _)| poster.rect().contains((x as u16, y as u16).into()))
        );
        // Boxed, it's a parcel on the floor.
        let parcel = Shown {
            boxed: true,
            ..poster
        };
        assert_eq!(
            (parcel.lane(), parcel.rect()),
            (Lane::Floor, Rect::new(1, 6, 4, 2))
        );
        // Too low a wall for it: it's in the closet alone, and nothing
        // moves.
        let low = empty("Users", 20, 7);
        let low = Buffer::with_lines(low.iter().map(String::as_str));
        let before = room.clone();
        let shown = room.project(&low, &[(Nook::Users, low.area)], &|_, _| false);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(shown[0].item, Furniture::Sofa);
        assert_eq!(room, before);
    }

    /// The wall never moves a room, however little of it there is; when
    /// the floor does, what hangs goes along.
    #[test]
    fn what_hangs_goes_where_the_room_goes() {
        let mut room = home(
            Nook::Users,
            &[prop(Furniture::Sofa, 0), prop(Furniture::Poster, 1000)],
        );
        let (buf, nooks) = two_panes(30, 9, &[]);
        assert_eq!(room.project(&buf, &nooks, &|_, _| false).len(), 2);
        // Too low to hang it: it's in the closet, the sofa stays.
        let (buf, nooks) = two_panes(30, 6, &[]);
        let shown = room.project(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert!(
            room.props
                .iter()
                .all(|p| p.strip == Strip::Bottom(Nook::Users))
        );
        // Too low for the sofa: both go to the Playlist, the poster where
        // its share of the way along puts it.
        let (buf, nooks) = two_panes(30, 4, &[]);
        let shown = room.project(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert!(
            room.props
                .iter()
                .all(|p| p.strip == Strip::Bottom(Nook::Playlist))
        );
        let poster = shown.iter().find(|s| s.item == Furniture::Poster).unwrap();
        assert_eq!(poster.rect().right(), 30 + 39, "against the far wall");
        // A hung piece alone on a strip that's gone goes too.
        let mut alone = home(Nook::Users, &[prop(Furniture::Poster, 0)]);
        let (buf, nooks) = two_panes(30, 9, &[]);
        let shown = alone.project(&buf, &nooks[1..], &|_, _| false);
        assert_eq!(shown.len(), 1, "{shown:?}");
        assert_eq!(alone.props[0].strip, Strip::Bottom(Nook::Playlist));
        // But only to a wall that holds it: with the other strip too low
        // to hang it, or text where it would hang, it stays where it was
        // (in the closet).
        let mut alone = home(Nook::Users, &[prop(Furniture::Poster, 0)]);
        let low = [(Nook::Playlist, Rect::new(30, 0, 40, 6))];
        let shown = alone.project(&buf, &low, &|_, _| false);
        assert_eq!(shown, []);
        assert_eq!(alone.props[0].strip, Strip::Bottom(Nook::Users), "too low");
        let mut written = buf.clone();
        for x in 31..69 {
            written[(x, 6)].set_symbol("x");
        }
        let shown = alone.project(&written, &nooks[1..], &|_, _| false);
        assert_eq!(shown, []);
        assert_eq!(alone.props[0].strip, Strip::Bottom(Nook::Users), "text");
    }

    /// A wall that can't hang all its pieces leaves out only those that
    /// don't fit beside the ones before them in anchor order, not every
    /// one of them.
    #[test]
    fn a_crowded_wall_leaves_out_only_what_it_cannot_hold() {
        let rows = empty("Users", 13, 9);
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, buf.area)];
        // (Three of a kind only here: she owns one of each.)
        let poster = |side, offset| Prop {
            anchor: Some(Anchor { side, offset }),
            ..prop(Furniture::Poster, 0)
        };
        let room = Home {
            props: vec![
                poster(Side::Left, 0),
                poster(Side::Right, 0),
                poster(Side::Left, 4),
            ],
        };
        // Eleven columns: room for two of the three. From the left wall
        // in, then from the right: the one against the right wall is
        // left out.
        let laid = room.clone().layout(&nooks);
        let lefts: Vec<i32> = laid.iter().map(|s| s.left).collect();
        assert_eq!(laid.len(), 2, "{laid:?}");
        assert!(laid.iter().all(|s| s.lane() == Lane::Wall));
        assert_eq!(lefts, [1, 5], "{laid:?}");
        let mut shown_room = room.clone();
        assert_eq!(shown_room.project(&buf, &nooks, &|_, _| false), laid);
        assert_eq!(shown_room, room, "nothing moved");
    }

    /// A delivery that hangs comes in only where it fits both ways: its
    /// parcel on the floor (with room to unpack it), and hung above.
    #[test]
    fn a_poster_is_delivered_where_it_fits_boxed_and_hung() {
        let room = home(Nook::Users, &[prop(Furniture::Sofa, 500)]);
        let rows = empty("Users", 30, 9);
        let clean = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, clean.area)];
        let mut projected = room.clone();
        let shown = projected.project(&clean, &nooks, &|_, _| false);
        let (prop, flap) = projected
            .doorstep(&clean, &nooks, &shown, &|_, _| false, Furniture::Poster)
            .expect("room for it");
        assert!(prop.boxed);
        assert_eq!(prop.lane(), Lane::Floor);
        assert_eq!(flap.rows, (6, 8), "the flap is at the floor");
        // Text where it would hang, at the screen's edge (the right
        // wall): it comes in at the left wall instead.
        let mut hung_over = clean.clone();
        hung_over[(27, 2)].set_symbol("x");
        let (prop, _) = projected
            .doorstep(&hung_over, &nooks, &shown, &|_, _| false, Furniture::Poster)
            .expect("the other wall");
        assert_eq!(prop.anchor.map(|a| a.side), Some(Side::Left));
        // Text where it would hang at either wall: no delivery.
        hung_over[(2, 2)].set_symbol("x");
        assert_eq!(
            projected.doorstep(&hung_over, &nooks, &shown, &|_, _| false, Furniture::Poster),
            None
        );
        // Text where the parcel would stand at either wall: none either.
        let mut floored = clean.clone();
        floored[(27, 7)].set_symbol("x");
        floored[(2, 7)].set_symbol("x");
        assert_eq!(
            projected.doorstep(&floored, &nooks, &shown, &|_, _| false, Furniture::Poster),
            None
        );
        // Too low to hang it: none.
        let low = Buffer::with_lines(empty("Users", 30, 7).iter().map(String::as_str));
        let low_nooks = [(Nook::Users, low.area)];
        let shown = projected.project(&low, &low_nooks, &|_, _| false);
        assert_eq!(
            projected.doorstep(&low, &low_nooks, &shown, &|_, _| false, Furniture::Poster),
            None
        );
        assert!(
            projected
                .doorstep(&low, &low_nooks, &shown, &|_, _| false, Furniture::Plant)
                .is_some(),
            "a plant stands"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        /// With a poster hung among her pieces: each lane keeps its
        /// anchor order and its walls, and never overlaps; what hangs
        /// never overlaps what stands; a resize and back puts every piece
        /// where it was; a wall too low for what hangs never moves the
        /// room; and text closets only the pieces it would cover.
        #[test]
        fn the_wall_lane_keeps_order_and_never_moves_the_room(
            props in decorated(),
            wide in 30u16..90,
            tall in 8u16..14,
            narrow in 20u16..90,
            low in 4u16..8,
            text in proptest::collection::vec((1u16..90, 1u16..13), 0..5),
        ) {
            let mut room = Home { props };
            let (buf, nooks) = two_panes(wide, tall, &[]);
            let first = room.project(&buf, &nooks, &|_, _| false);
            let pinned = room.clone();
            let laid = room.layout(&nooks);
            prop_assert_eq!(&room, &pinned);
            let order = |s: &Shown| {
                let index = room.props.iter().position(|p| p.item == s.item);
                index.and_then(|i| Some(order_key(room.props.get(i)?.anchor?, i)))
            };
            for lane in [Lane::Floor, Lane::Wall] {
                for strip in [Strip::Bottom(Nook::Users), Strip::Bottom(Nook::Playlist)] {
                    let mut along: Vec<Shown> = laid
                        .iter()
                        .filter(|s| s.lane() == lane && s.strip == Some(strip))
                        .copied()
                        .collect();
                    along.sort_by_key(|s| s.left);
                    for pair in along.windows(2) {
                        prop_assert!(pair[0].rect().right() <= pair[1].rect().x, "{:?}", along);
                        prop_assert!(order(&pair[0]) < order(&pair[1]), "{:?}", along);
                    }
                }
            }
            for s in &laid {
                let &(_, e) = strips(&nooks).iter().find(|(t, _)| Some(*t) == s.strip).unwrap();
                let r = s.rect();
                prop_assert!(i32::from(r.x) >= e.from && i32::from(r.right()) <= e.to, "{:?}", s);
                prop_assert!(i32::from(r.y) > e.floor - i32::from(e.rows) - 1, "inside: {:?}", s);
                for t in &laid {
                    if s.lane() == Lane::Wall && t.lane() == Lane::Floor {
                        prop_assert!(!s.cover().intersects(t.cover()), "{:?} over {:?}", s, t);
                    }
                }
            }
            // A resize and back: where the narrower floor still holds
            // what stands on it, nothing moves and every piece shows
            // where it did; where it doesn't, they all go to the other
            // strip, or (with no room there) nothing moves.
            let (small, at_small) = two_panes(narrow, tall, &[]);
            let users_strip = Strip::Bottom(Nook::Users);
            let small_users = strips(&at_small)
                .into_iter()
                .find(|(s, _)| *s == users_strip)
                .map(|(_, e)| e)
                .unwrap();
            let holds = pack(&pinned.on(users_strip, Lane::Floor), small_users).is_some();
            let _ = room.project(&small, &at_small, &|_, _| false);
            if holds {
                prop_assert_eq!(&room, &pinned, "a resize that holds them moved a piece");
                prop_assert_eq!(&room.project(&buf, &nooks, &|_, _| false), &first);
            } else if room != pinned {
                for (p, was) in room.props.iter().zip(&pinned.props) {
                    let went = was.strip == users_strip;
                    prop_assert_eq!(
                        p.strip,
                        if went { Strip::Bottom(Nook::Playlist) } else { was.strip },
                        "{:?}", p
                    );
                }
            }
            // A wall too low for what hangs: it's in the closet, and
            // nothing moves unless the floor's too low too.
            let mut lowered = pinned.clone();
            let (short, at_short) = two_panes(wide, low, &[]);
            let shown = lowered.project(&short, &at_short, &|_, _| false);
            let floor_holds = pinned
                .props
                .iter()
                .filter(|p| p.strip == Strip::Bottom(Nook::Users) && p.lane() == Lane::Floor)
                .all(|p| p.item.spec().footprint.1 + 2 <= low);
            let users = |s: &&Shown| s.strip == Some(Strip::Bottom(Nook::Users));
            if floor_holds {
                prop_assert_eq!(&lowered, &pinned, "the wall moved the room");
                prop_assert!(!shown.iter().filter(users).any(|s| s.lane() == Lane::Wall));
                let standing = |v: &[Shown]| -> Vec<(Furniture, i32)> {
                    v.iter().filter(users).filter(|s| s.lane() == Lane::Floor).map(|s| (s.item, s.left)).collect()
                };
                prop_assert_eq!(standing(&shown), standing(&first));
            }
            // Text closets only the pieces it would cover.
            let mut room = pinned.clone();
            let (noisy, _) = two_panes(wide, tall, &text);
            let noisy_shown = room.project(&noisy, &nooks, &|_, _| false);
            prop_assert_eq!(&room, &pinned, "text never moves a piece");
            prop_assert_eq!(&room.layout(&nooks), &laid);
            for s in &first {
                let covered = s.cover().intersection(noisy.area);
                let hit = (covered.x..covered.right()).any(|x| {
                    (covered.y..covered.bottom()).any(|y| noisy[(x, y)] != buf[(x, y)])
                });
                prop_assert_eq!(noisy_shown.contains(s), !hit, "{:?}", s);
            }
        }
    }

    #[test]
    fn a_left_facing_drawing_is_mirrored() {
        assert_eq!(glyph(Furniture::Desk, Facing::Right, 6, 0), Some('/'));
        assert_eq!(glyph(Furniture::Desk, Facing::Left, 0, 0), Some('\\'));
        assert_eq!(glyph(Furniture::Desk, Facing::Left, 6, 0), None);
    }
}
