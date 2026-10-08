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
    /// A round wall clock showing her time of day (it comes as a gift).
    Clock,
    /// A window on the wall, with the sky of her time of day outside.
    Window,
}

impl Furniture {
    /// Every piece, in the order they were added (their serde names are
    /// stable; the first eight are the oldest builds', see
    /// [`Furniture::legacy`]). What the channel sells, in order, is
    /// `CATALOGUE`.
    pub const ALL: [Self; 12] = [
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
        Self::Clock,
        Self::Window,
    ];

    /// Whether it's just for looks.
    pub(super) fn decor(self) -> bool {
        self.spec().offers.contains(&Offer::Decor)
    }

    /// Whether the oldest builds that read her record know it (the first
    /// eight kinds): they skip every later one, so it claims no room's
    /// pane in what's written for them.
    pub(super) fn legacy(self) -> bool {
        match self {
            Self::Sofa
            | Self::Tv
            | Self::Bed
            | Self::Desk
            | Self::Lamp
            | Self::Bookshelf
            | Self::Fridge
            | Self::CatBed => true,
            Self::Plant | Self::Poster | Self::Clock | Self::Window => false,
        }
    }

    /// Whether a piece of its kind and one of `other`'s may ever share
    /// cells, one in front of the other: only her window and a sofa
    /// (phase 5c D6). The window hangs low enough for her to lean on its
    /// sill, so it would meet anything that stands under it; the sofa's
    /// back may cover its lower corner (the window drawn behind it), but
    /// nothing else may stand under it. Only the kinds: the rule every
    /// placement reads, for the pieces as they stand (real, out of their
    /// boxes, the corner only), is [`Shown::may_overlap`].
    pub(super) fn may_overlap(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Window, Self::Sofa) | (Self::Sofa, Self::Window)
        )
    }

    /// Whether it shows her time of day (the clock's dial, the window's
    /// sky), so it changes as her clock runs, on the quarter-hour.
    pub(super) fn tells_time(self) -> bool {
        matches!(self, Self::Clock | Self::Window)
    }

    /// The way it's drawn facing `facing`: a symmetric piece always as
    /// facing right (a mirrored dial would read 3:00 as 9:00).
    pub(super) fn drawn_facing(self, facing: Facing) -> Facing {
        if self.spec().symmetric {
            Facing::Right
        } else {
            facing
        }
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
            Self::Clock => &CLOCK,
            Self::Window => &WINDOW,
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
    /// tallest piece that stands, it hangs over any of them; hung lower
    /// (her window), it hangs clear of each it may not overlap (see
    /// [`Furniture::may_overlap`]).
    pub hang: Option<u16>,
    /// How much prettier it makes a room (decor; nothing else does).
    pub beauty: f64,
    /// It reads the same either way round, and is always drawn facing
    /// right (as line art and in ASCII): the clock's dial and the
    /// window's sky would read wrong mirrored.
    pub symmetric: bool,
}

const SOFA: Spec = Spec {
    name: "sofa",
    pitch: line!("A sofa! I'll take it!"),
    footprint: (9, 3),
    ascii: &[" .-----. ", "(|_____|)", " '     ' "],
    ink: (Color::Rgb(111, 161, 156), Color::Cyan),
    uses: &[Use::Lounge, Use::Nap],
    offers: &[Offer::Seat],
    sit: Some((4, false)),
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
    symmetric: false,
};
const TV: Spec = Spec {
    name: "TV",
    pitch: line!("A TV! I'll take it!"),
    footprint: (6, 4),
    ascii: &["  \\/  ", ".----.", "|[  ]|", "|_::_|"],
    ink: (Color::Rgb(203, 191, 168), Color::Gray),
    uses: &[Use::Watch],
    offers: &[Offer::Screen],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
    symmetric: false,
};
const BED: Spec = Spec {
    name: "bed",
    pitch: line!("A bed... yes please!"),
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
    symmetric: false,
};
const DESK: Spec = Spec {
    name: "desk",
    pitch: line!("A desk. For homework."),
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
    symmetric: false,
};
const LAMP: Spec = Spec {
    name: "lamp",
    pitch: line!("Ooh, a lamp!"),
    footprint: (3, 4),
    ascii: &[" _ ", "/_\\", " | ", "_|_"],
    ink: (Color::Rgb(232, 195, 74), Color::LightYellow),
    uses: &[],
    offers: &[Offer::Light],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
    symmetric: false,
};
const BOOKSHELF: Spec = Spec {
    name: "bookshelf",
    pitch: line!("Books! I'll take it!"),
    footprint: (5, 4),
    ascii: &["_____", "|IlI|", "|lII|", "|___|"],
    ink: (Color::Rgb(160, 120, 79), Color::Yellow),
    uses: &[Use::Read],
    offers: &[Offer::Books],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
    symmetric: false,
};
const FRIDGE: Spec = Spec {
    name: "fridge",
    pitch: line!("A fridge... for snacks!"),
    footprint: (4, 4),
    ascii: &["____", "| .|", "|--|", "|_.|"],
    ink: (Color::Rgb(231, 236, 239), Color::White),
    uses: &[Use::Snack],
    offers: &[Offer::Cold],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
    symmetric: false,
};
const CAT_BED: Spec = Spec {
    name: "cat bed",
    pitch: line!("A cat bed! For a cat!"),
    footprint: (4, 2),
    ascii: &["    ", "\\__/"],
    ink: (Color::Rgb(201, 69, 63), Color::Red),
    uses: &[Use::Pet],
    offers: &[Offer::Cat],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
    symmetric: false,
};
const PLANT: Spec = Spec {
    name: "potted plant",
    pitch: line!("So green and leafy!"),
    footprint: (3, 3),
    ascii: &["\\|/", "~Y~", "\\_/"],
    ink: (Color::Rgb(122, 166, 106), Color::Green),
    uses: &[],
    offers: &[Offer::Decor],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 1.0,
    symmetric: false,
};
const POSTER: Spec = Spec {
    name: "poster",
    pitch: line!("It'd look nice up!"),
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
    symmetric: false,
};
const CLOCK: Spec = Spec {
    name: "wall clock",
    pitch: line!("Ooh, a clock!"),
    footprint: (3, 2),
    // The hand's cell is drawn over (see the guest's `overrides`).
    ascii: &[".-.", "(o)"],
    ink: (Color::Rgb(217, 101, 91), Color::LightRed),
    uses: &[],
    offers: &[Offer::Decor],
    sit: None,
    comfort: 1.0,
    // Beside the poster, as high.
    hang: Some(4),
    beauty: 0.5,
    symmetric: true,
};
const WINDOW: Spec = Spec {
    name: "window",
    pitch: line!("A window! A view!"),
    footprint: (4, 2),
    // The sky's two cells are drawn over (see the guest's `overrides`).
    ascii: &[".--.", "|  |"],
    ink: (Color::Rgb(243, 234, 216), Color::White),
    // She leans on its sill, gazing out (phase 5b D7, 5c D6).
    uses: &[Use::LookOut],
    offers: &[],
    sit: None,
    comfort: 1.0,
    // Low, so its sill is at her chest and she can lean on it, chin in
    // her hands (phase 5c D6): among the pieces that stand, so it hangs
    // clear of every one but a sofa (see [`Furniture::may_overlap`]).
    hang: Some(1),
    beauty: 0.0,
    symmetric: true,
};

/// The glyph at `(dx, dy)` of `item`'s ASCII drawing facing `facing`, if
/// that cell is drawn (spaces are not).
pub(super) fn glyph(item: Furniture, facing: Facing, dx: u16, dy: u16) -> Option<char> {
    let spec = item.spec();
    let facing = item.drawn_facing(facing);
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
    /// The List pane.
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
    /// Stand under the window (or beside it), gazing out at the sky.
    LookOut,
}

impl Use {
    /// Every use.
    #[cfg(test)]
    pub const ALL: [Self; 11] = [
        Self::Lounge,
        Self::Nap,
        Self::Sleep,
        Self::Homework,
        Self::Watch,
        Self::Unpack,
        Self::Read,
        Self::Snack,
        Self::Pet,
        Self::Crumple,
        Self::LookOut,
    ];

    /// Whether she uses it from in it (sits on it, lies in it), rather
    /// than from beside it (or, looking out of the window, from under
    /// it).
    pub fn inside(self) -> bool {
        !matches!(
            self,
            Use::Watch | Use::Read | Use::Snack | Use::Pet | Use::LookOut
        )
    }

    /// Whether a piece is set down only where she'd fit to use it so
    /// (see [`roomy`]). Not to look out of the window: it's hung where
    /// the wall has room for it, and she looks out of it if she can get
    /// under it or beside it (it's offered only then: see
    /// [`Shown::look_out_spots`]). Asking room for that too would keep a
    /// window over a sofa in the closet (and refuse her any move that
    /// set a piece down beneath one). A new one comes in where she could
    /// look out of it first, if anywhere (see [`room_to_look`]).
    pub fn asks_room(self) -> bool {
        self != Use::LookOut
    }

    /// Whether it's a still use, whose length her mood lingers over
    /// (phase 5c M8: see [`Stillness::linger`]): lounging, napping,
    /// sleeping (by day: her night has its own wake), reading, looking
    /// out, and watching TV, now that it holds a picture after its
    /// switch-on (phase 5c D7). Not chores (unpacking, crumpling, a
    /// snack, petting the cat), nor homework, whose nod-off her mood
    /// moves instead.
    ///
    /// [`Stillness::linger`]: super::stillness::Stillness::linger
    pub fn lingers(self) -> bool {
        match self {
            Use::Lounge | Use::Nap | Use::Sleep | Use::Read | Use::LookOut | Use::Watch => true,
            Use::Homework | Use::Unpack | Use::Snack | Use::Pet | Use::Crumple => false,
        }
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
/// hang over a standing one), but a hung piece low enough to meet one
/// that stands hangs clear of it, unless it may overlap it (her window
/// and a sofa: see [`Furniture::may_overlap`], [`Home::laid_and_shifted`]); the
/// floor lane is the room's, and only it moves the room (see
/// [`Home::project_with`]).
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
    pub(super) fn share(&self, at: u16, cols: u16) -> i32 {
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

/// The strips of the quiet panes this frame, each between its walls.
/// Raw: a strip's floor lane is narrowed by her door's space (see
/// [`Home::extents`]), which every placement reads; only pinning (an
/// anchor's share of the way along, as older builds read it) and the
/// two readers of "which room is she in" (`clock_on`, `beauty_at` in
/// mod.rs, where the space is as much the room as the rest) read this.
pub(super) fn raw_strips(nooks: &[(Nook, Rect)]) -> Vec<(Strip, Extent)> {
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

/// The columns her door's space takes along a strip, against its wall.
pub(super) const SPACE: i32 = 6;

/// Her external door's wall: a side of one strip (saved).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) struct DoorWall {
    pub strip: Strip,
    pub side: Side,
}

/// A strip as this frame lays it out: `raw` between its walls; `floor`,
/// what its floor lane packs on (raw less her door's space, when it's
/// kept); `space`, her door's reserved rect on this strip, if it can
/// exist at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StripPlan {
    pub strip: Strip,
    pub raw: Extent,
    pub floor: Extent,
    pub space: Option<Space>,
}

impl StripPlan {
    /// What a piece in `lane` is laid along: standing, the floor; hung,
    /// the raw wall.
    pub(super) fn along(&self, lane: Lane) -> Extent {
        match lane {
            Lane::Floor => self.floor,
            Lane::Wall => self.raw,
        }
    }

    /// The anchor that keeps a piece in `lane`, `cols` wide, at `left`
    /// (laid along [`StripPlan::along`] it), and its share of the way
    /// along `raw` (as older builds read it: shares are always of the raw
    /// strip).
    pub(super) fn pin(&self, lane: Lane, left: i32, cols: u16) -> (Anchor, u16) {
        (
            self.along(lane).pin(left, cols).0,
            self.raw.pin(left, cols).1,
        )
    }
}

/// Her door's space on its strip: against the wall on `side` (column
/// `wall`), `rect` its [`SPACE`] columns by her height and the floor
/// row. `kept` when keeping it costs the strip nothing (see
/// [`Home::extents`]); else it yields this frame: the strip lays out as
/// with no door (its pieces on the raw strip, some maybe in it).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Space {
    pub side: Side,
    pub wall: i32,
    pub rect: Rect,
    pub kept: bool,
}

/// What one layout pass produced, with the plans it was laid on (never
/// handed in separately, so nothing is judged on another home's plans).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LaidOut {
    pub shown: Vec<Shown>,
    pub shifts: Vec<(Furniture, Option<u32>)>,
    pub plans: Vec<StripPlan>,
}

impl LaidOut {
    /// The plan of `strip`, if it's here.
    pub(super) fn plan(&self, strip: Strip) -> Option<&StripPlan> {
        self.plans.iter().find(|p| p.strip == strip)
    }
}

/// The frame's geometry every door and keep-out function reads: her
/// quiet panes, the chat pane, and the screen.
#[derive(Clone, Copy, Debug)]
pub(super) struct Plan<'a> {
    pub nooks: &'a [(Nook, Rect)],
    pub chat: Rect,
    pub screen: Rect,
}

impl<'a> Plan<'a> {
    /// `nooks` alone: no chat pane, the screen their bounding box.
    #[cfg(test)]
    pub(super) fn bare(nooks: &'a [(Nook, Rect)]) -> Self {
        let screen = nooks
            .iter()
            .map(|&(_, r)| r)
            .reduce(|a, b| a.union(b))
            .unwrap_or_default();
        Self {
            nooks,
            chat: Rect::default(),
            screen,
        }
    }
}

/// Whether a wall on `side` of a nook drawn at `rect` is at the
/// screen's edge (read from the nook's rect, never a space's).
pub(super) fn at_edge(rect: Rect, side: Side, screen: Rect) -> bool {
    match side {
        Side::Right => rect.right() == screen.right(),
        Side::Left => rect.x == screen.x,
    }
}

/// `raw` less her door's space against its wall on `side`.
fn narrowed(raw: Extent, side: Side) -> Extent {
    match side {
        Side::Right => Extent {
            to: raw.to - SPACE,
            ..raw
        },
        Side::Left => Extent {
            from: raw.from + SPACE,
            ..raw
        },
    }
}

/// Her door's space against the wall on `side` of `raw`, as its wall
/// column and rect, if it can exist there: her height clear above the
/// floor, and room for the space beside her (or the widest piece that
/// stands on the strip, `widest`). Else none: nothing to keep.
fn space_of(raw: Extent, side: Side, widest: u16) -> Option<(i32, Rect)> {
    use super::sprite::{HEIGHT, WIDTH};
    let wide = WIDTH.max(i32::from(widest));
    if i32::from(raw.rows) < HEIGHT || raw.to - raw.from < SPACE + wide {
        return None;
    }
    let (wall, left) = match side {
        Side::Right => (raw.to, raw.to - SPACE),
        Side::Left => (raw.from - 1, raw.from),
    };
    let rect = Rect::new(
        u16::try_from(left).ok()?,
        u16::try_from(raw.floor - HEIGHT).ok()?,
        SPACE as u16,
        (HEIGHT + 1) as u16,
    );
    Some((wall, rect))
}

/// Which wall her door would be in, chosen from the geometry of `plan`
/// for `home` (see [`Home::wall`]), among `strips` (all, when `None`). A
/// wall qualifies when its strip is here, has room for her door's space
/// (see [`Home::extents`]) and that space misses the chat pane. Of
/// those: at the screen's edge first; then on a strip her pieces stand
/// on; then where the space would be kept (her floor pieces pack beside
/// it and no hung piece is lost to it, as [`Home::extents`] judges it;
/// only a tie-break: a crowded edge wall of her own strip still wins,
/// and its space yields until she clears it); then in pane order, right
/// before left.
fn choose(home: &Home, plan: Plan, only: Option<Strip>) -> Option<DoorWall> {
    let mine = home.furnished();
    // Not at the screen's edge, not her pieces' strip, its space not kept,
    // the pane's place, the left side: least first.
    type Key = (bool, bool, bool, usize, bool);
    let mut best: Option<(Key, DoorWall)> = None;
    for (index, (&(_, rect), (strip, raw))) in
        plan.nooks.iter().zip(raw_strips(plan.nooks)).enumerate()
    {
        if only.is_some_and(|s| s != strip) {
            continue;
        }
        let widest = home.widest_standing(strip);
        for side in [Side::Right, Side::Left] {
            let Some((_, space)) = space_of(raw, side, widest) else {
                continue;
            };
            if space.intersects(plan.chat) {
                continue;
            }
            // Kept as [`Home::extents`] judges it: her floor pieces pack
            // beside it and no hung piece is lost to it.
            let wall = DoorWall { strip, side };
            let kept = Home {
                door: Some(wall),
                ..home.clone()
            }
            .plan_strip(strip, raw)
            .0
            .space
            .is_some_and(|s| s.kept);
            let key = (
                !at_edge(rect, side, plan.screen),
                !mine.contains(&strip),
                !kept,
                index,
                side == Side::Left,
            );
            if best.is_none_or(|(k, _)| key < k) {
                best = Some((key, wall));
            }
        }
    }
    best.map(|(_, wall)| wall)
}

/// The wall [`choose`] picks among all of `plan`'s.
pub(super) fn choose_wall(home: &Home, plan: Plan) -> Option<DoorWall> {
    choose(home, plan, None)
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
/// [`Home::project_with`]).
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
    /// `beside` (a standing spot next to it), facing it. Looking out of
    /// the window, `beside` is the spot she stands on (under it or
    /// beside it: see [`Shown::look_out_spots`]), and she faces the
    /// window's middle from it, gazing up the way she faces.
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
            // A heap she crumples, bending over its middle.
            Use::Crumple if self.scrap.is_some() => (mirrored(cols / 2), self.facing),
            // In a makeshift piece, or beside it, as its kind has her.
            _ if self.scrap.is_some() => match super::scrap::sit(self.item) {
                (col, false) => (mirrored(col), self.facing),
                (col, true) => (mirrored(col), flip(self.facing)),
            },
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
            Use::LookOut => (
                beside,
                if beside < self.left + cols / 2 {
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

    /// Whether it and `other` may share cells, one in front of the other
    /// (phase 5c D6): only her window, hung, and her sofa, standing, both
    /// real and out of their boxes (see [`Furniture::may_overlap`]), and
    /// only while the sofa's back covers no more than the window's
    /// corner: she could still lean on its sill from an end clear of the
    /// sofa (her box at one of [`Shown::look_out_spots`] meets none of
    /// it). The one rule every placement of either reads, both ways
    /// round: packing the wall, a delivery, her moving a piece.
    pub(super) fn may_overlap(&self, other: &Shown) -> bool {
        if !self.item.may_overlap(other.item) {
            return false;
        }
        let (window, sofa) = if self.lane() == Lane::Wall {
            (self, other)
        } else {
            (other, self)
        };
        let real = |s: &Shown| !s.boxed && s.scrap.is_none();
        real(window)
            && real(sofa)
            && window.item == Furniture::Window
            && window.lane() == Lane::Wall
            && sofa.lane() == Lane::Floor
            && window
                .look_out_spots()
                .iter()
                .any(|&x| her_box(x, window.floor).is_some_and(|b| !b.intersects(sofa.cover())))
    }

    /// Where she'd stand to look out of it (a window), leaning on its
    /// sill (phase 5c D6), in the order she tries them: at its end, her
    /// face over the glass, facing into it: first at the end she faces it
    /// from the way it was hung (its left end, hung facing right), then at
    /// the other. As the approved sheet stands her: facing right, her box
    /// centred on its first column; facing left, a column outside its
    /// last (the lean's figure sits a column toward her back in its box,
    /// so the two aren't mirror images: phase 5c step 8c). Her box takes
    /// in the window's end columns (it hangs low), and nothing else may be
    /// in it: a sofa the window hangs behind blocks that end, so she
    /// leans at it from its free one.
    pub fn look_out_spots(&self) -> [i32; 2] {
        let (cols, _) = self.size();
        let (left, right) = (self.left, self.left + i32::from(cols));
        match self.facing {
            Facing::Right => [left, right],
            Facing::Left => [right, left],
        }
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

/// Everything she owns, and the wall her external door is in.
#[derive(Clone, Debug, Default, PartialEq, Hash)]
pub(super) struct Home {
    pub props: Vec<Prop>,
    /// Her door's wall, once she has a piece and a wall qualified (see
    /// [`Home::settle_door`]); `None`, chosen afresh each frame (see
    /// [`Home::wall`]).
    pub door: Option<DoorWall>,
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

    /// The widest of her pieces that stand on `strip`'s floor, anchored
    /// or not (0 with none).
    fn widest_standing(&self, strip: Strip) -> u16 {
        self.props
            .iter()
            .filter(|p| p.strip == strip && p.lane() == Lane::Floor)
            .map(|p| p.item.spec().footprint.0)
            .max()
            .unwrap_or(0)
    }

    /// Her strips as this frame lays them out (see [`StripPlan`]): each
    /// quiet pane's, and on her door's wall's strip ([`Home::door`]) her
    /// door's space, where it can exist. The space is kept when keeping
    /// it costs the strip nothing: her floor pieces there pack beside it,
    /// and every piece hung there that the strip lays out without her
    /// door is still laid out with the floor beside the space and the
    /// space refused it (a piece pushed along by the narrower floor may
    /// meet her window, which would have nowhere else to hang). Else it
    /// yields this frame, and the strip lays out as with no door at all.
    /// With only what stands, whether it's kept never depends on anchors,
    /// only on what stands there; a hung piece's place may decide it.
    /// Needs no screen and no chat.
    pub(super) fn extents(&self, nooks: &[(Nook, Rect)]) -> Vec<StripPlan> {
        raw_strips(nooks)
            .into_iter()
            .map(|(strip, raw)| self.plan_strip(strip, raw).0)
            .collect()
    }

    /// `strip`'s plan (see [`Home::extents`]), and on her door's strip,
    /// where it has a space, the strip laid out as the plan has it (its
    /// space kept or yielding): judging the one lays out the other.
    fn plan_strip(&self, strip: Strip, raw: Extent) -> (StripPlan, Option<Option<LaidOut>>) {
        let mut laid = None;
        let space = self.door.filter(|d| d.strip == strip).and_then(|d| {
            let (wall, rect) = space_of(raw, d.side, self.widest_standing(strip))?;
            let doored = self.lay_strip(strip, raw, narrowed(raw, d.side), Some(rect));
            let doorless = self.lay_strip(strip, raw, raw, None);
            let kept = match (&doored, &doorless) {
                (Some(doored), Some(doorless)) => doorless
                    .shown
                    .iter()
                    .all(|s| doored.shown.iter().any(|t| t.item == s.item)),
                (doored, None) => doored.is_some(),
                (None, Some(_)) => false,
            };
            laid = Some(if kept { doored } else { doorless });
            Some(Space {
                side: d.side,
                wall,
                rect,
                kept,
            })
        });
        let floor = match space {
            Some(space) if space.kept => narrowed(raw, space.side),
            _ => raw,
        };
        let plan = StripPlan {
            strip,
            raw,
            floor,
            space,
        };
        (plan, laid)
    }

    /// Her door's wall: the saved one, else the one [`choose_wall`]
    /// picks this frame (unsaved). Every reader of where her door is
    /// asks this.
    pub(super) fn wall(&self, plan: Plan) -> Option<DoorWall> {
        self.door.or_else(|| choose_wall(self, plan))
    }

    /// Save her door's wall when it's due, returning whether it changed:
    /// once she has a piece and a wall qualifies, if none is saved. Never
    /// forgets a saved wall, nor chooses again for a resize or a frame
    /// too short for it: a saved wall whose space meets the chat is
    /// refused frame by frame (where her door stands), so going back
    /// restores it. (Her pieces moving off its strip take it with them:
    /// see [`Home::move_off`].)
    pub(super) fn settle_door(&mut self, plan: Plan) -> bool {
        if self.door.is_some() || self.props.is_empty() {
            return false;
        }
        let wall = choose_wall(self, plan);
        tracing::trace!(?wall, "houseguest: her door's wall settled");
        self.door = wall;
        wall.is_some()
    }

    /// Where her pieces show this frame, as production asks it: her
    /// door's wall settled ([`Home::settle_door`]), her pieces projected
    /// ([`Home::project_with`]: kept off the chat pane where they move,
    /// her door's wall going with them if they move off its strip). Also
    /// whether her home changed (anchors pinned, pieces moved, her door's
    /// wall saved). The only call that logs a change of her door's wall
    /// (a scratch home's is no change of hers).
    pub(super) fn frame(
        &mut self,
        buf: &Buffer,
        plan: Plan,
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> (Vec<Shown>, bool) {
        let before = self.clone();
        self.settle_door(plan);
        let (shown, moves) = self.project_with(buf, plan, blocked);
        for (from, to) in moves {
            tracing::info!(?from, ?to, "houseguest: her pieces moved");
        }
        if self.door != before.door {
            tracing::info!(wall = ?self.door, "houseguest: her door's wall");
        }
        let changed = *self != before;
        (shown, changed)
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
    /// Always on the raw strip (a share is of the way between its walls).
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
    /// anything; [`Home::project_with`] does); a piece its wall doesn't hold
    /// (too low, or too narrow beside the others hung there) is left out
    /// alone. Pure
    /// geometry: text over a piece doesn't take it out of the layout.
    /// An older record's piece is anchored where its share of the way
    /// along puts it, the first time its strip is here.
    pub fn layout(&mut self, nooks: &[(Nook, Rect)]) -> Vec<Shown> {
        self.laid_out(nooks).shown
    }

    /// [`Home::layout`], with what [`Home::laid_and_shifted`] tells of it.
    pub(super) fn laid_out(&mut self, nooks: &[(Nook, Rect)]) -> LaidOut {
        self.pin_anchors(&raw_strips(nooks));
        self.laid_and_shifted(nooks)
    }

    /// Where her pieces stand on `nooks`, once every piece whose strip is
    /// there is anchored (a piece that isn't is left out), and how far
    /// each hung piece hangs from where its wall alone would hang it (see
    /// [`Home::hung_clear`]: the columns, or none where it's left out for
    /// want of a clear place; only the pieces so moved), with the plans
    /// they were laid on ([`Home::extents`]): those that stand
    /// packed on each strip's floor, those hung on the raw wall, both
    /// clear of her door's space while it's kept.
    pub(super) fn laid_and_shifted(&self, nooks: &[(Nook, Rect)]) -> LaidOut {
        let (plans, mut lays): (Vec<StripPlan>, Vec<Option<Option<LaidOut>>>) = raw_strips(nooks)
            .into_iter()
            .map(|(strip, raw)| self.plan_strip(strip, raw))
            .unzip();
        let mut shown: Vec<Shown> = Vec::new();
        let mut shifts = Vec::new();
        for strip in self.furnished() {
            let Some(at) = plans.iter().position(|p| p.strip == strip) else {
                continue;
            };
            let (Some(&plan), Some(laid)) = (plans.get(at), lays.get_mut(at)) else {
                continue;
            };
            // Her door's strip, as its plan was judged; a space that
            // yields refuses nothing: the strip lays out as with no door.
            let laid = laid
                .take()
                .unwrap_or_else(|| self.lay_strip(strip, plan.raw, plan.floor, None));
            if let Some(laid) = laid {
                shown.extend(laid.shown);
                shifts.extend(laid.shifts);
            }
        }
        LaidOut {
            shown,
            shifts,
            plans,
        }
    }

    /// `strip` laid out alone (see [`Home::laid_and_shifted`]): what
    /// stands packed on `floor`, what hangs on the `raw` wall, clear of
    /// what stands and of `space`; none of it if what stands doesn't
    /// pack. Its plans are left empty.
    fn lay_strip(
        &self,
        strip: Strip,
        raw: Extent,
        floor: Extent,
        space: Option<Rect>,
    ) -> Option<LaidOut> {
        let mut packed = pack(&self.on(strip, Lane::Floor), floor)?;
        let standing: Vec<Shown> = packed
            .iter()
            .filter_map(|&(i, left)| Some(stand(self.props.get(i)?, strip, raw, left)))
            .collect();
        let hung = pack_each(&self.on(strip, Lane::Wall), raw);
        let clear = self.hung_clear(hung.clone(), strip, raw, space, &standing);
        let mut shifts = Vec::new();
        for &(index, left) in &hung {
            let Some(prop) = self.props.get(index) else {
                continue;
            };
            match clear.iter().find(|&&(i, _)| i == index) {
                Some(&(_, now)) if now == left => {}
                Some(&(_, now)) => shifts.push((prop.item, Some((now - left).unsigned_abs()))),
                None => shifts.push((prop.item, None)),
            }
        }
        packed.extend(clear);
        packed.sort_unstable();
        let shown = packed
            .into_iter()
            .filter_map(|(index, left)| Some(stand(self.props.get(index)?, strip, raw, left)))
            .collect();
        Some(LaidOut {
            shown,
            shifts,
            plans: Vec::new(),
        })
    }

    /// `hung`, her pieces on `strip`'s wall as [`pack_each`] has them on
    /// `e`, each clear of every piece `standing` there it may not overlap
    /// (see [`Shown::may_overlap`]: only her window hangs low enough to
    /// meet one, phase 5c D6). One that meets such a piece hangs at the
    /// nearest place between its neighbours on the wall that meets none,
    /// keeping its order along the wall: where she could lean on its sill
    /// from an end clear of what stands, if there's any such place, then
    /// the nearest (to the left, of two as near); with none, it's left
    /// out alone (a wall never moves a room). So an older record's
    /// window, hung over her TV or wholly behind her sofa, shows beside
    /// the TV, or with only its corner behind the sofa. Nor does one meet
    /// her door's `space` (while it's kept), which only her window hangs
    /// low enough to.
    fn hung_clear(
        &self,
        mut hung: Vec<(usize, i32)>,
        strip: Strip,
        e: Extent,
        space: Option<Rect>,
        standing: &[Shown],
    ) -> Vec<(usize, i32)> {
        hung.sort_by_key(|&(index, left)| (left, index));
        let cols = |index: usize| {
            self.props
                .get(index)
                .map_or(0, |p| i32::from(p.item.spec().footprint.0))
        };
        let mut out: Vec<(usize, i32)> = Vec::with_capacity(hung.len());
        for (k, &(index, left)) in hung.iter().enumerate() {
            let Some(prop) = self.props.get(index) else {
                continue;
            };
            let meets = |left: i32| {
                let at = stand(prop, strip, e, left);
                space.is_some_and(|r| at.rect().intersects(r))
                    || standing
                        .iter()
                        .any(|s| at.rect().intersects(s.cover()) && !at.may_overlap(s))
            };
            if !meets(left) {
                out.push((index, left));
                continue;
            }
            // Where she could lean on its sill (a window) from an end
            // clear of every piece that stands.
            let leans = |left: i32| {
                let at = stand(prop, strip, e, left);
                at.look_out_spots().iter().any(|&x| {
                    her_box(x, at.floor)
                        .is_some_and(|b| !standing.iter().any(|s| b.intersects(s.cover())))
                })
            };
            let from = out.last().map_or(e.from, |&(j, l)| l + cols(j));
            let to = hung.get(k + 1).map_or(e.to, |&(_, l)| l) - cols(index);
            match (from..=to)
                .filter(|&l| !meets(l))
                .min_by_key(|&l| (!leans(l), (l - left).abs(), l))
            {
                Some(clear) => out.push((index, clear)),
                None => {
                    tracing::trace!(item = ?prop.item, "houseguest: no room on the wall clear of what stands")
                }
            }
        }
        out
    }

    /// [`Home::project_with`] on `nooks` alone (no chat pane, the
    /// nooks' union the screen): the tests' way in.
    #[cfg(test)]
    pub fn project(
        &mut self,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> Vec<Shown> {
        self.project_with(buf, Plan::bare(nooks), blocked).0
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
    /// Nothing moves onto `plan`'s chat pane, and her door's wall goes
    /// with her pieces if they move off its strip (see
    /// [`Home::move_off`]). With each move of a strip's pieces it made,
    /// from and to. Laid out once, again only after a move.
    pub(super) fn project_with(
        &mut self,
        buf: &Buffer,
        plan: Plan,
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> (Vec<Shown>, Vec<(Strip, Strip)>) {
        let mut laid = self.laid_out(plan.nooks);
        let mut moves = Vec::new();
        for strip in self.furnished() {
            let packs = laid
                .plan(strip)
                .is_some_and(|p| pack(&self.on(strip, Lane::Floor), p.floor).is_some());
            if !packs && let Some(to) = self.move_off(strip, buf, plan, blocked) {
                moves.push((strip, to));
                laid = self.laid_and_shifted(plan.nooks);
            }
        }
        let mut shown: Vec<Shown> = Vec::new();
        for at in laid.shown {
            if fits(buf, &at, &|x, y| free(&shown, blocked, Some(&at), x, y)) {
                shown.push(at);
            }
        }
        (shown, moves)
    }

    /// Move `strip`'s pieces, together, to the first other strip whose
    /// floor holds those that stand with its own, each on free cells;
    /// those hung go along, where their share of the way puts them. With
    /// only hung pieces, to the first whose wall holds them all that way.
    /// None of them may meet `plan`'s chat pane, nor the target's door
    /// space (kept or not). Her door goes with them if it was on `strip`:
    /// to the target's wall [`choose`] picks, else the first that
    /// qualifies; each target judged with her door already there, so the
    /// pieces are laid out the frame they move as they'll stay. Where
    /// they went, if they moved.
    fn move_off(
        &mut self,
        strip: Strip,
        buf: &Buffer,
        plan: Plan,
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> Option<Strip> {
        let leaving: Vec<usize> = (0..self.props.len())
            .filter(|&i| self.props.get(i).is_some_and(|p| p.strip == strip))
            .collect();
        let follows = self.door.is_some_and(|d| d.strip == strip);
        let target = raw_strips(plan.nooks)
            .into_iter()
            .filter(|&(s, _)| s != strip)
            .find_map(|(to, raw)| {
                // Pieces never anchored, and hung ones, are anchored by
                // their share there (of the raw strip, as every share).
                let mut moved = self.clone();
                for &i in &leaving {
                    let prop = moved.props.get_mut(i)?;
                    let cols = prop.item.spec().footprint.0;
                    let share = Some(raw.pin(raw.share(prop.at, cols), cols).0);
                    prop.anchor = match prop.lane() {
                        Lane::Floor => prop.anchor.or(share),
                        Lane::Wall => share,
                    };
                    prop.strip = to;
                }
                if follows {
                    moved.door =
                        choose(&moved, plan, Some(to)).or_else(|| choose_wall(&moved, plan));
                }
                let strip_plan = *moved.extents(plan.nooks).iter().find(|p| p.strip == to)?;
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
                let e = strip_plan.along(lane);
                let packed = pack(&moved.on(to, lane), e)?;
                let space = strip_plan.space.map(|s| s.rect);
                let all_free =
                    packed
                        .iter()
                        .filter(|(i, _)| leaving.contains(i))
                        .all(|&(i, left)| {
                            moved.props.get(i).is_some_and(|p| {
                                let at = stand(p, to, e, left);
                                let cover = at.cover();
                                fits(buf, &at, &|x, y| !blocked(x, y))
                                    && !cover.intersects(plan.chat)
                                    && space.is_none_or(|r| !cover.intersects(r))
                            })
                        });
                all_free.then_some((to, strip_plan, moved, packed))
            });
        let (to, strip_plan, mut moved, packed) = target?;
        tracing::trace!(from = ?strip, ?to, door = ?moved.door, "houseguest: her pieces move off");
        for (i, left) in packed {
            if leaving.contains(&i)
                && let Some(prop) = moved.props.get_mut(i)
            {
                prop.at = strip_plan.raw.pin(left, prop.item.spec().footprint.0).1;
            }
        }
        *self = moved;
        Some(to)
    }

    /// Where `item` could go this frame: anywhere on a strip it fits,
    /// clear of what's `shown` and of her door's space, chosen at random
    /// (a stage gift: it stands where it's settled).
    pub fn spot(
        &self,
        buf: &Buffer,
        plan: Plan,
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
        let lane = Prop::new(item, Nook::Users, 0, facing).lane();
        // Along each strip, her door where it will be with the piece
        // there (her first piece chooses it), and the places along the
        // strip as its plan with the piece there has them: standing, on
        // the floor (beside her door's space); hung, along the wall.
        let spots: Vec<(Prop, bool)> = raw_strips(plan.nooks)
            .into_iter()
            .filter_map(|(strip, _)| {
                let mut probe = self.clone();
                let Strip::Bottom(nook) = strip;
                probe.props.push(Prop {
                    anchor: Some(Anchor {
                        side: Side::Left,
                        offset: 0,
                    }),
                    ..Prop::new(item, nook, 0, facing)
                });
                probe.door = probe.wall(plan);
                let home = Home {
                    door: probe.door,
                    ..self.clone()
                };
                let strip_plan = *probe
                    .extents(plan.nooks)
                    .iter()
                    .find(|p| p.strip == strip)?;
                Some((home, strip_plan))
            })
            .flat_map(|(home, strip_plan)| {
                let (strip, e) = (strip_plan.strip, strip_plan.along(lane));
                (0..=10).filter_map(move |step| {
                    let left = e.share(step * 100, cols);
                    let (anchor, at) = strip_plan.pin(lane, left, cols);
                    let prop = Prop {
                        item,
                        strip,
                        anchor: Some(anchor),
                        at,
                        facing,
                        boxed: false,
                        settled: true,
                    };
                    if !e.holds(prop.needs()) {
                        return None;
                    }
                    // Where it's laid out once it's there, if at all
                    // (packed with what's on its lane, a window kept
                    // clear of what stands and of her door's space: else
                    // it would only go to the closet); and never in her
                    // door's space as the strip is with it there, kept or
                    // not (a window hung there would stand aside, or not
                    // at all).
                    let mut with = home.clone();
                    with.props.push(prop);
                    let laid = with.laid_and_shifted(plan.nooks);
                    let at = *laid.shown.iter().find(|s| s.item == item)?;
                    let space = laid.plan(strip).and_then(|p| p.space).map(|s| s.rect);
                    space
                        .is_none_or(|r| !at.cover().intersects(r))
                        .then_some((prop, at))
                })
            })
            .filter_map(|(prop, at)| {
                let clear = |x: i32, y: i32| free(shown, blocked, Some(&at), x, y);
                let her = |x: i32, y: i32| free(shown, blocked, None, x, y);
                (fits(buf, &at, &clear) && roomy(buf, &at, &her))
                    .then(|| (prop, room_to_look(buf, &at, &her)))
            })
            .collect();
        // A window where she could stand to look out of it, if there's
        // anywhere (see [`Home::doorstep`]).
        let looking: Vec<Prop> = spots.iter().filter(|(_, l)| *l).map(|&(p, _)| p).collect();
        let spots: Vec<Prop> = if looking.is_empty() {
            spots.into_iter().map(|(p, _)| p).collect()
        } else {
            looking
        };
        spots.get(rng.below(spots.len() as u64) as usize).copied()
    }

    /// Where a delivery of `item` comes in this frame: through a flap in
    /// one of her strips' walls, preferring a wall at the screen's edge,
    /// to stand against it facing into the room, unsettled (she never
    /// chose where it stands). The pieces already on
    /// that strip make way, packed in order, but only where every one of
    /// them that shows still fits, its box fits on blank, free cells, she
    /// can unpack it, and she'd fit to use the piece. Both are asked of
    /// `seats` (her seats at a piece, among the pieces shown: see
    /// `seats_of`), each on the whole room as it would show (the pieces
    /// that made way where they'd be): with the box standing there, an
    /// `Unpack` seat; with the piece out of its box, a seat for each way
    /// she uses it that asks room (see [`Use::asks_room`]), and to look
    /// out of a window first coming in where she could. So it's judged
    /// as her seats will be once it's there: in line art the image she'd
    /// be drawn in takes in the box (or the piece) and whatever it meets;
    /// in either mode she stands on a floor. A piece that
    /// hangs must fit both ways: boxed, standing on the floor to be
    /// unpacked, and hung on the wall above. A window comes in first
    /// through a wall where she could stand to look out of it (not over
    /// a piece standing beneath it: see [`room_to_look`]), if any; else
    /// wherever it fits.
    pub fn doorstep(
        &self,
        buf: &Buffer,
        plan: Plan,
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        seats: &dyn Fn(&Shown, &[Shown]) -> Vec<Seat>,
        item: Furniture,
    ) -> Option<(Prop, Flap)> {
        let mut walls: Vec<(bool, Strip, Extent, Side)> = Vec::new();
        for (&(_, rect), (strip, e)) in plan.nooks.iter().zip(raw_strips(plan.nooks)) {
            for side in [Side::Right, Side::Left] {
                walls.push((at_edge(rect, side, plan.screen), strip, e, side));
            }
        }
        // The screen's edge first; otherwise in pane order.
        walls.sort_by_key(|&(edge, ..)| !edge);
        let looks: &[bool] = if item.spec().uses.contains(&Use::LookOut) {
            &[true, false]
        } else {
            &[false]
        };
        let tries = looks
            .iter()
            .flat_map(|&look| walls.iter().map(move |&wall| (look, wall)));
        tries.into_iter().find_map(|(look, (_, strip, e, side))| {
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
            // Its box stands on the floor, where she can get to it to
            // unpack it (judged as her seats are, not as room to use a
            // piece: a box narrower than her at a wall has her take in
            // the wall's line); then the piece, standing there or hung
            // above, must fit with room to use it.
            let parcel = Prop {
                boxed: true,
                ..prop
            };
            // Judged with her door where it will be once it's in (her
            // first piece's arrival chooses it): it stands past her
            // door's space from the frame it arrives.
            let door = {
                let mut with = self.clone();
                with.props.push(parcel);
                with.wall(plan)
            };
            let doored = Home {
                door,
                ..self.clone()
            };
            let at = doored.admits(buf, shown, blocked, plan.nooks, parcel, Room::None)?;
            let room = if look { Room::ToLook } else { Room::ToUse };
            doored.admits(buf, shown, blocked, plan.nooks, prop, room)?;
            // The room as it will show with the box in it, then with the
            // piece out of it: her seats there are judged on each.
            let offers = |prop: Prop, wants: &[Use]| {
                let mut with = self.clone();
                with.door = door;
                with.props.push(prop);
                let (after, _) = with.project_with(buf, plan, blocked);
                let Some(piece) = after.iter().find(|s| s.item == item) else {
                    return false;
                };
                let seats = seats(piece, &after);
                wants
                    .iter()
                    .all(|&what| seats.iter().any(|seat| seat.what == what))
            };
            let uses: Vec<Use> = item
                .spec()
                .uses
                .iter()
                .copied()
                .filter(|&what| what.asks_room() || look && what == Use::LookOut)
                .collect();
            if !offers(parcel, &[Use::Unpack]) || !offers(prop, &uses) {
                return None;
            }
            // The flap is in the wall itself (the raw strip's).
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

    /// Where `prop`, new, would stand on its strip this frame (her door
    /// where this home has it: the caller sets it as it will be), if the
    /// pieces in its lane there make way for it, packed in order
    /// (standing, on the floor beside her door's space; hung, on the raw
    /// wall): every one of them that shows still fits, and it fits on
    /// blank, free cells, with the `room` around it she'd need.
    fn admits(
        &self,
        buf: &Buffer,
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        nooks: &[(Nook, Rect)],
        prop: Prop,
        room: Room,
    ) -> Option<Shown> {
        let (strip, lane) = (prop.strip, prop.lane());
        let mut with = self.clone();
        with.props.push(prop);
        let new = with.props.len() - 1;
        let e = with
            .extents(nooks)
            .iter()
            .find(|p| p.strip == strip)?
            .along(lane);
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
            let clear_for = |piece: Option<&Shown>, x: i32, y: i32| {
                free(&others, blocked, piece, x, y)
                    && !packed
                        .iter()
                        .any(|(j, s)| *j != i && s.rect().contains((x as u16, y as u16).into()))
            };
            // The piece clear of what it may not overlap; her, of all.
            let clear = |x: i32, y: i32| clear_for(Some(at), x, y);
            let her = |x: i32, y: i32| clear_for(None, x, y);
            fits(buf, at, &clear)
                && (i != new
                    || match room {
                        Room::None => true,
                        Room::ToUse => roomy(buf, at, &her),
                        Room::ToLook => roomy(buf, at, &her) && room_to_look(buf, at, &her),
                    })
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

/// Whether she'd fit to use `at` every way it's used that asks room of
/// it (a new piece is never set down where she couldn't; see
/// [`Use::asks_room`]): for a use in it, her box at its
/// seat, beyond the piece itself; for a use beside it, her box at one of
/// the spots beside it, standing on a line. Her box must be blank and
/// `clear`.
pub(super) fn roomy(buf: &Buffer, at: &Shown, clear: &dyn Fn(i32, i32) -> bool) -> bool {
    room_for(buf, at, clear, Use::asks_room)
}

/// Whether she'd fit to look out of `at` (a window), standing at one of
/// [`Shown::look_out_spots`], as [`roomy`] judges room (true of a piece
/// she doesn't look out of). Not asked of a window (see
/// [`Use::asks_room`]), only preferred where a new one comes in.
pub(super) fn room_to_look(buf: &Buffer, at: &Shown, clear: &dyn Fn(i32, i32) -> bool) -> bool {
    room_for(buf, at, clear, |what| what == Use::LookOut)
}

/// Whether she'd fit to use `at` every way it's used that `asks` room of
/// (see [`roomy`]).
fn room_for(
    buf: &Buffer,
    at: &Shown,
    clear: &dyn Fn(i32, i32) -> bool,
    asks: impl Fn(Use) -> bool,
) -> bool {
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
    at.uses().iter().filter(|&&what| asks(what)).all(|&what| {
        let spots: &[i32] = match what {
            Use::LookOut => &at.look_out_spots(),
            _ if what.inside() => {
                let seat = at.seat(what, 0);
                return fits(seat.x, seat.y);
            }
            _ => &at.beside(),
        };
        spots
            .iter()
            .any(|&x| fits(x, at.floor) && floor(x, at.floor))
    })
}

/// The room a new piece asks around it (see [`Home::admits`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Room {
    /// None: its parcel, which she only unpacks.
    None,
    /// Room to use it (see [`roomy`]).
    ToUse,
    /// That, and room to look out of it, a window (see [`room_to_look`]).
    ToLook,
}

/// Her box standing at `(x, y)` (her anchor column, the floor row she
/// stands on): the cells her sprite takes, and the floor beneath them.
/// None where it would leave the screen's top or left.
pub(super) fn her_box(x: i32, y: i32) -> Option<Rect> {
    let half = super::sprite::WIDTH / 2;
    let (Ok(left), Ok(top)) = (
        u16::try_from(x - half),
        u16::try_from(y - super::sprite::HEIGHT),
    ) else {
        return None;
    };
    let width = u16::try_from(super::sprite::WIDTH).ok()?;
    let height = u16::try_from(super::sprite::HEIGHT + 1).ok()?;
    Some(Rect::new(left, top, width, height))
}

/// Whether `(x, y)` is free for `at` (a piece being placed; `None`:
/// her, as room to use one): not `blocked`, nor under a shown piece it
/// may not overlap (see [`Shown::may_overlap`]; her box may overlap
/// none). The one test both lanes' pieces are placed by, so a piece that
/// stands never goes under her window, nor her window over it, but for a
/// sofa covering its corner.
fn free(
    shown: &[Shown],
    blocked: &dyn Fn(i32, i32) -> bool,
    at: Option<&Shown>,
    x: i32,
    y: i32,
) -> bool {
    !blocked(x, y)
        && !shown.iter().any(|s| {
            s.rect().contains((x as u16, y as u16).into())
                && !at.is_some_and(|at| at.may_overlap(s))
        })
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

    /// Her seats at `piece` (every way she'd use it) as if she could stand
    /// anywhere: for a doorstep that judges only the room's cells.
    fn anywhere(piece: &Shown, _: &[Shown]) -> Vec<Seat> {
        piece
            .uses()
            .iter()
            .map(|&what| piece.seat(what, piece.beside()[0]))
            .collect()
    }

    /// Her seats at `piece` as if she could stand nowhere.
    fn nowhere(_: &Shown, _: &[Shown]) -> Vec<Seat> {
        Vec::new()
    }

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
                    Plan::bare(&nooks),
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

    /// A stage gift of her window may go with its corner behind her sofa
    /// (phase 5c D6), as anything placing it reads [`Shown::may_overlap`]:
    /// with the wall past the sofa's end taken, that's the one place it
    /// goes (never wholly behind it).
    #[test]
    fn a_gifted_window_may_go_with_its_corner_behind_her_sofa() {
        // 76 wide: the gift's places along the wall are 7 apart, from 1.
        let rows = empty("Users", 76, 9);
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, buf.area)];
        let blocked = |x: i32, _: i32| x >= 12;
        let mut room = home(Nook::Users, &[prop(Furniture::Sofa, 0)]);
        let shown = room.project(&buf, &nooks, &blocked);
        assert_eq!(shown.len(), 1);
        for seed in 0..8 {
            let window = room
                .spot(
                    &buf,
                    Plan::bare(&nooks),
                    &shown,
                    &blocked,
                    Furniture::Window,
                    &mut Rng(seed),
                )
                .expect("its corner behind the sofa");
            let mut with = room.clone();
            assert!(with.add(window));
            let both = with.project(&buf, &nooks, &blocked);
            let w = *both
                .iter()
                .find(|s| s.item == Furniture::Window)
                .expect("shown");
            assert_eq!(w.left, 8, "seed {seed}");
            assert!(w.rect().intersects(shown[0].rect()));
        }
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
    /// (newer or older than any of them), and maybe her window hung low
    /// among them too, each placed as [`pieces`].
    fn decorated() -> impl Strategy<Value = Vec<Prop>> {
        (
            proptest::sample::subsequence(standing(), 0..=4),
            any::<proptest::sample::Index>(),
            proptest::option::of(any::<proptest::sample::Index>()),
        )
            .prop_flat_map(|(mut items, at, window)| {
                items.insert(at.index(items.len() + 1), Furniture::Poster);
                if let Some(window) = window {
                    items.insert(window.index(items.len() + 1), Furniture::Window);
                }
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
            let mut room = Home { props, ..Default::default() };
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
    /// (and so above any it hangs over), and above her head; all but her
    /// window, hung low for her to lean on its sill (its bottom row the
    /// floor less 2, the sill at her chest: phase 5c D6). It hangs among
    /// the pieces that stand, so it may share columns only with what it
    /// may overlap: a sofa (drawn in front of it), and nothing else.
    /// [`Furniture::may_overlap`] says so of that pair, both ways round,
    /// and of no other. Each is used from beneath or beside it, never
    /// from in it (out of reach).
    #[test]
    fn hung_pieces_clear_every_standing_piece() {
        let tallest = standing()
            .into_iter()
            .map(|f| f.spec().footprint.1)
            .max()
            .unwrap();
        assert!(tallest >= super::super::sprite::HEIGHT as u16);
        for a in Furniture::ALL {
            for b in Furniture::ALL {
                assert_eq!(a.may_overlap(b), b.may_overlap(a), "{a:?} {b:?}");
                assert_eq!(
                    a.may_overlap(b),
                    matches!(
                        (a, b),
                        (Furniture::Window, Furniture::Sofa) | (Furniture::Sofa, Furniture::Window)
                    ),
                    "{a:?} {b:?}"
                );
            }
        }
        assert_eq!(
            Furniture::Window.spec().hang,
            Some(1),
            "her sill at her chest"
        );
        for item in Furniture::ALL {
            if let Some(hang) = item.spec().hang {
                if item != Furniture::Window {
                    assert!(hang >= tallest, "{item:?} hangs {hang}, under {tallest}");
                }
                // Hung lower than a piece that stands, it may overlap one
                // (the sofa: so the sofa's back can cover its corner).
                if hang < tallest {
                    assert!(
                        standing().into_iter().any(|s| item.may_overlap(s)),
                        "{item:?} hangs {hang}, under {tallest}, over nothing it may overlap"
                    );
                }
                // Used from the floor beneath or beside it, never from in
                // it (out of reach).
                for &what in item.spec().uses {
                    assert!(!what.inside(), "{item:?} is out of reach for {what:?}");
                }
            }
        }
    }

    /// Behind her sofa, her window shows only its corner covered (phase 5c
    /// D6, "the sofa covers the window's lower corner"): wherever it was
    /// hung along the wall, against either wall, whichever came first, it
    /// shows, and she could still lean on its sill from an end clear of
    /// the sofa (her box at one of its look-out spots meets no part of
    /// it). A sofa still in its box is no sofa: the window never shares a
    /// cell with the parcel.
    #[test]
    fn a_window_never_hangs_wholly_behind_a_sofa() {
        let rows = empty("Users", 40, 9);
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, buf.area)];
        let her = |x: i32, y: i32| {
            let half = super::super::sprite::WIDTH / 2;
            Rect::new(
                (x - half).max(0) as u16,
                (y - super::super::sprite::HEIGHT) as u16,
                super::super::sprite::WIDTH as u16,
                super::super::sprite::HEIGHT as u16 + 1,
            )
        };
        let anchored = |item: Furniture, side: Side, offset: u16, boxed: bool| Prop {
            anchor: Some(Anchor { side, offset }),
            boxed,
            ..prop(item, 0)
        };
        for side in [Side::Left, Side::Right] {
            for boxed in [false, true] {
                for offset in 0..12 {
                    for window_first in [false, true] {
                        let at = format!(
                            "{side:?} offset {offset} boxed {boxed} window first {window_first}"
                        );
                        let sofa = anchored(Furniture::Sofa, side, 0, boxed);
                        let window = anchored(Furniture::Window, side, offset, false);
                        let props = if window_first {
                            [window, sofa]
                        } else {
                            [sofa, window]
                        };
                        let mut room = home(Nook::Users, &props);
                        let shown = room.project(&buf, &nooks, &|_, _| false);
                        assert_eq!(shown.len(), 2, "{at}: {shown:?}");
                        let get = |item: Furniture| *shown.iter().find(|s| s.item == item).unwrap();
                        let (w, sofa) = (get(Furniture::Window), get(Furniture::Sofa));
                        if boxed {
                            assert!(!w.rect().intersects(sofa.cover()), "{at}: over the parcel");
                            continue;
                        }
                        if w.rect().intersects(sofa.cover()) {
                            assert!(
                                w.look_out_spots()
                                    .iter()
                                    .any(|&x| x >= 2 && !her(x, w.floor).intersects(sofa.cover())),
                                "{at}: {w:?} behind {sofa:?}, no end clear"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Her window hangs low (phase 5c D6), and it may hang behind a sofa:
    /// against the same wall, the sofa's back covers its lower corner (two
    /// of its columns), whichever came first, and nothing moves for it;
    /// she leans on its sill from its free end.
    #[test]
    fn a_window_may_hang_behind_a_sofa() {
        let rows = empty("Users", 30, 9);
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, buf.area)];
        for window_first in [false, true] {
            let sofa = prop(Furniture::Sofa, 0);
            let window = Prop {
                anchor: Some(Anchor {
                    side: Side::Left,
                    offset: 7,
                }),
                ..prop(Furniture::Window, 0)
            };
            let props = if window_first {
                [window, sofa]
            } else {
                [sofa, window]
            };
            let mut room = home(Nook::Users, &props);
            room.layout(&nooks);
            let before = room.clone();
            let shown = room.project(&buf, &nooks, &|_, _| false);
            let at = format!("window first: {window_first}");
            assert_eq!(shown.len(), 2, "{at}: {shown:?}");
            let get = |item: Furniture| *shown.iter().find(|s| s.item == item).unwrap();
            let (w, sofa) = (get(Furniture::Window), get(Furniture::Sofa));
            assert_eq!((w.left, sofa.left), (8, 1), "{at}: each where it was put");
            assert_eq!(w.rect(), Rect::new(8, 5, 4, 2), "{at}: hung low");
            assert_eq!(
                w.rect().intersection(sofa.rect()).width,
                2,
                "{at}: its corner"
            );
            assert!(w.may_overlap(&sofa) && sofa.may_overlap(&w), "{at}");
            assert_eq!(room, before, "{at}: nothing moved");
        }
    }

    /// Over any other piece that stands, her window never hangs (phase 5c
    /// D6): hung where it would meet one, it hangs beside it instead, on
    /// the nearest stretch of its wall that's clear; with none, it's in
    /// the closet alone, and the room never moves for it. Whichever came
    /// first.
    #[test]
    fn a_window_hangs_clear_of_every_other_standing_piece() {
        for item in standing().into_iter().filter(|&f| f != Furniture::Sofa) {
            for window_first in [false, true] {
                let at = format!("{item:?}, window first: {window_first}");
                let (piece, window) = (prop(item, 0), prop(Furniture::Window, 0));
                let props = if window_first {
                    [window, piece]
                } else {
                    [piece, window]
                };
                let rows = empty("Users", 40, 9);
                let buf = Buffer::with_lines(rows.iter().map(String::as_str));
                let nooks = [(Nook::Users, buf.area)];
                let mut room = home(Nook::Users, &props);
                room.layout(&nooks);
                let before = room.clone();
                let shown = room.project(&buf, &nooks, &|_, _| false);
                assert_eq!(shown.len(), 2, "{at}: {shown:?}");
                let get = |f: Furniture| *shown.iter().find(|s| s.item == f).unwrap();
                let (w, standing) = (get(Furniture::Window), get(item));
                assert_eq!(standing.left, 1, "{at}: it stays against the wall");
                assert_eq!(w.rect().y, 5, "{at}: hung low");
                assert!(
                    !w.rect().intersects(standing.cover()),
                    "{at}: {w:?} over {standing:?}"
                );
                assert_eq!(
                    w.left,
                    i32::from(standing.rect().right()),
                    "{at}: beside it, as near as it can"
                );
                assert_eq!(room, before, "{at}: nothing moved");
                // No room beside it: in the closet alone.
                let cols = item.spec().footprint.0;
                let rows = empty("Users", usize::from(cols) + 2 + 3, 9);
                let buf = Buffer::with_lines(rows.iter().map(String::as_str));
                let nooks = [(Nook::Users, buf.area)];
                let shown = room.project(&buf, &nooks, &|_, _| false);
                assert_eq!(
                    shown.iter().map(|s| s.item).collect::<Vec<_>>(),
                    [item],
                    "{at}: narrow"
                );
                assert_eq!(room, before, "{at}: narrow, nothing moved");
            }
        }
    }

    /// A window that can't hang where it was hung (over her TV) hangs
    /// where she could lean on its sill, if its wall has such a place,
    /// before the nearest: not in the gap between her TV and her lamp,
    /// where either end of it is up against one of them, but past the
    /// lamp, where its far end is clear.
    #[test]
    fn a_window_moved_off_a_piece_hangs_where_she_can_lean() {
        let rows = empty("Users", 40, 9);
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, buf.area)];
        let anchored = |item: Furniture, offset: u16| Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset,
            }),
            ..prop(item, 0)
        };
        let room = home(
            Nook::Users,
            &[
                anchored(Furniture::Tv, 0),
                anchored(Furniture::Lamp, 11),
                anchored(Furniture::Window, 0),
            ],
        );
        let shown = room.clone().project(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 3, "{shown:?}");
        let get = |item: Furniture| *shown.iter().find(|s| s.item == item).unwrap();
        let (tv, lamp, w) = (
            get(Furniture::Tv),
            get(Furniture::Lamp),
            get(Furniture::Window),
        );
        assert_eq!((tv.left, lamp.left), (1, 12));
        assert_eq!(w.left, 15, "past the lamp: {w:?}");
        assert!(!w.rect().intersects(lamp.cover()) && !w.rect().intersects(tv.cover()));
    }

    /// A saved home whose window hung at the old height (four rows up,
    /// over her TV, or over her sofa) loads as it was saved, and shows the
    /// window hung low and valid: beside the TV (never over it), the TV
    /// where it stood; over the sofa, as near as it can be with only its
    /// corner behind it, so she can lean on its sill from its free end.
    #[test]
    fn a_window_saved_at_the_old_hang_hangs_low_and_valid() {
        use super::super::ledger::Ledger;
        let record = |under: &str| {
            format!(
                concat!(
                    r#"{{"version":1,"master_seed":3,"visits":9,"rooms":[["Living","Users"]],"#,
                    r#""props":[{{"item":"{under}","at":0,"facing":"Right","boxed":false}},"#,
                    r#"{{"item":"Window","at":20,"facing":"Right","boxed":false}}],"#,
                    r#""anchors":[{{"item":"{under}","strip":{{"Bottom":"Users"}},"anchor":{{"side":"Left","offset":0}}}},"#,
                    r#"{{"item":"Window","strip":{{"Bottom":"Users"}},"anchor":{{"side":"Left","offset":1}}}}]}}"#
                ),
                under = under
            )
        };
        let rows = empty("Users", 40, 9);
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, buf.area)];
        for (under, overlaps) in [("Tv", false), ("Sofa", true)] {
            let ledger = Ledger::from_json(&record(under)).expect("it reads");
            let mut home = ledger.home.clone();
            let shown = home.project(&buf, &nooks, &|_, _| false);
            assert_eq!(home, ledger.home, "{under}: nothing moved");
            assert_eq!(shown.len(), 2, "{under}: {shown:?}");
            let get = |f: Furniture| *shown.iter().find(|s| s.item == f).unwrap();
            let w = get(Furniture::Window);
            let below = *shown.iter().find(|s| s.item != Furniture::Window).unwrap();
            assert_eq!(below.left, 1, "{under}");
            assert_eq!(w.rect().y, 5, "{under}: hung low");
            assert_eq!(
                w.rect().intersects(below.cover()),
                overlaps,
                "{under}: {w:?} {below:?}"
            );
            if !overlaps {
                assert_eq!(
                    w.left,
                    i32::from(below.rect().right()),
                    "{under}: beside it"
                );
            } else {
                // Hung at 2 (wholly behind it), it hangs at 8: two of its
                // columns behind the sofa's last two.
                assert_eq!(w.left, 8, "{under}: its corner behind it");
                assert!(w.may_overlap(&below), "{under}");
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
            ..Default::default()
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

    /// Nothing that stands comes in under her window, nor does her window
    /// come in over anything that stands but a sofa, and then only with
    /// its corner behind it (phase 5c D6, [`Shown::may_overlap`]): a
    /// delivery through a wall where the one would meet the other comes
    /// in through the other wall instead.
    #[test]
    fn a_delivery_never_meets_her_window_but_for_a_sofa() {
        let rows = empty("Users", 40, 9);
        let clean = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, clean.area)];
        let against_right = |item: Furniture| Prop {
            anchor: Some(Anchor {
                side: Side::Right,
                offset: 0,
            }),
            at: 1000,
            facing: Facing::Left,
            ..prop(item, 1000)
        };
        // Her door in a pane that isn't here: no space on this strip, so
        // both walls are the window's to hang against (her door's space
        // would keep it off one; step 5 of the door batch refuses a
        // delivery there).
        let comes_in = |room: &Home, item: Furniture| {
            let mut projected = room.clone();
            projected.door = Some(DoorWall {
                strip: Strip::Bottom(Nook::List),
                side: Side::Left,
            });
            let shown = projected.project(&clean, &nooks, &|_, _| false);
            projected
                .doorstep(
                    &clean,
                    Plan::bare(&nooks),
                    &shown,
                    &|_, _| false,
                    &anywhere,
                    item,
                )
                .map(|(prop, _)| prop.anchor.map_or(Side::Left, |a| a.side))
        };
        // Her window against the right wall (the first a delivery
        // tries): a TV comes in at the left; a sofa too, as its parcel
        // (a box, no sofa yet) may share no cell with the window.
        let windowed = home(Nook::Users, &[against_right(Furniture::Window)]);
        assert_eq!(comes_in(&windowed, Furniture::Tv), Some(Side::Left));
        assert_eq!(comes_in(&windowed, Furniture::Lamp), Some(Side::Left));
        assert_eq!(comes_in(&windowed, Furniture::Sofa), Some(Side::Left));
        // Her bed against the left wall, and against the right her TV
        // (or bookshelf, or sofa: against the wall, it would hide the
        // window wholly): a window comes in through neither; nothing
        // against the right, and there.
        for (under, side) in [
            (Some(Furniture::Tv), None),
            (Some(Furniture::Bookshelf), None),
            (Some(Furniture::Sofa), None),
            (None, Some(Side::Right)),
        ] {
            let mut props = vec![prop(Furniture::Bed, 0)];
            props.extend(under.map(against_right));
            let room = home(Nook::Users, &props);
            assert_eq!(
                comes_in(&room, Furniture::Window),
                side,
                "over her {under:?}"
            );
        }
    }

    /// A delivery that hangs comes in only where it fits both ways: its
    /// parcel on the floor (where she can stand to unpack it: `stands`),
    /// and hung above.
    #[test]
    fn a_poster_is_delivered_where_it_fits_boxed_and_hung() {
        let room = home(Nook::Users, &[prop(Furniture::Sofa, 500)]);
        let rows = empty("Users", 30, 9);
        let clean = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [(Nook::Users, clean.area)];
        let mut projected = room.clone();
        let shown = projected.project(&clean, &nooks, &|_, _| false);
        let (prop, flap) = projected
            .doorstep(
                &clean,
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Poster,
            )
            .expect("room for it");
        assert!(prop.boxed);
        assert_eq!(prop.lane(), Lane::Floor);
        assert_eq!(flap.rows, (6, 8), "the flap is at the floor");
        assert_eq!(prop.anchor.map(|a| a.side), Some(Side::Right));
        // Her door's wall is the right one (the screen's edge, her
        // sofa's strip): the flap is in the wall itself, column 29, and
        // the box stands past her door's space (columns 23-28), on 19-22.
        assert_eq!(flap.x, 29, "the flap is in the wall");
        let mut with = projected.clone();
        assert!(with.add(prop));
        with.door = with.wall(Plan::bare(&nooks));
        let boxed = with
            .layout(&nooks)
            .into_iter()
            .find(|s| s.item == Furniture::Poster)
            .expect("its box stands");
        assert_eq!((boxed.left, boxed.size().0), (19, 4), "{boxed:?}");
        // Nowhere for her to stand to unpack it at the right wall (its
        // box's middle column, 21): the left wall; at neither, none.
        let (prop, _) = projected
            .doorstep(
                &clean,
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &|piece, _| {
                    let unpack = piece.seat(Use::Unpack, 0);
                    match unpack.x != 21 || !piece.boxed {
                        true => piece
                            .uses()
                            .iter()
                            .map(|&what| piece.seat(what, 0))
                            .collect(),
                        false => Vec::new(),
                    }
                },
                Furniture::Poster,
            )
            .expect("the other wall");
        assert_eq!(prop.anchor.map(|a| a.side), Some(Side::Left));
        assert_eq!(
            projected.doorstep(
                &clean,
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &nowhere,
                Furniture::Poster
            ),
            None
        );
        // Text where it would hang, at the screen's edge (the right
        // wall): it comes in at the left wall instead.
        let mut hung_over = clean.clone();
        hung_over[(27, 2)].set_symbol("x");
        let (prop, _) = projected
            .doorstep(
                &hung_over,
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Poster,
            )
            .expect("the other wall");
        assert_eq!(prop.anchor.map(|a| a.side), Some(Side::Left));
        // Text where it would hang at either wall: no delivery.
        hung_over[(2, 2)].set_symbol("x");
        assert_eq!(
            projected.doorstep(
                &hung_over,
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Poster
            ),
            None
        );
        // Text where the parcel would stand at either wall: none either.
        let mut floored = clean.clone();
        floored[(21, 7)].set_symbol("x");
        floored[(2, 7)].set_symbol("x");
        assert_eq!(
            projected.doorstep(
                &floored,
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Poster
            ),
            None
        );
        // Too low to hang it: none.
        let low = Buffer::with_lines(empty("Users", 30, 7).iter().map(String::as_str));
        let low_nooks = [(Nook::Users, low.area)];
        let shown = projected.project(&low, &low_nooks, &|_, _| false);
        assert_eq!(
            projected.doorstep(
                &low,
                Plan::bare(&low_nooks),
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Poster
            ),
            None
        );
        assert!(
            projected
                .doorstep(
                    &low,
                    Plan::bare(&low_nooks),
                    &shown,
                    &|_, _| false,
                    &anywhere,
                    Furniture::Plant
                )
                .is_some(),
            "a plant stands"
        );
    }

    /// A window comes in first through a wall where she could stand to
    /// look out of it: with her sofa against the right wall (the first
    /// tried), through the left. With text where she'd stand at either
    /// wall it comes in all the same, through the right (looking out of
    /// it asks no room of where it hangs: see [`Use::asks_room`]), where
    /// she can't. With her bed against the left wall and her sofa against
    /// the right, it comes in through neither: over the bed it may not
    /// hang, and against the right wall the sofa would hide it wholly.
    #[test]
    fn a_window_comes_in_where_she_can_look_out() {
        let rows = empty("Users", 30, 9);
        let clean = Buffer::with_lines(rows.iter().map(String::as_str));
        // A letter where her box would be, leaning at a window against
        // either wall (its free end's spot). Against the left wall, just
        // above the floor. Against the right wall (columns 25-28, hung on
        // rows 5-6) she leans facing right with her box centred on its
        // first column (23-27, rows 4-7, as the approved sheet stands
        // her): the letter at (27, 4) is in that box, above the window
        // and clear of the parcel it comes in, and not in a box centred
        // a column further out (22-26).
        let mut busy = rows.clone();
        busy[7] = format!("│      x{}│", " ".repeat(21));
        busy[4] = format!("│{}x │", " ".repeat(26));
        let busy = Buffer::with_lines(busy.iter().map(String::as_str));
        let nooks = [(Nook::Users, clean.area)];
        for (props, buf, side, looks) in [
            (
                vec![prop(Furniture::Sofa, 1000)],
                &clean,
                Some(Side::Left),
                true,
            ),
            (vec![], &busy, Some(Side::Right), false),
            (
                vec![prop(Furniture::Sofa, 1000), prop(Furniture::Bed, 0)],
                &clean,
                None,
                false,
            ),
        ] {
            let at = format!("{props:?}");
            let mut room = home(Nook::Users, &props);
            let shown = room.project(buf, &nooks, &|_, _| false);
            assert_eq!(shown.len(), props.len(), "{at}");
            let delivered = room.doorstep(
                buf,
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Window,
            );
            assert_eq!(
                delivered.map(|(window, _)| window.anchor.map(|a| a.side)),
                side.map(Some),
                "{at}"
            );
            let Some((window, _)) = delivered else {
                continue;
            };
            assert!(room.add(Prop {
                boxed: false,
                ..window
            }));
            let shown = room.project(buf, &nooks, &|_, _| false);
            let hung = *shown
                .iter()
                .find(|s| s.item == Furniture::Window)
                .unwrap_or_else(|| panic!("{at}: shown"));
            let others: Vec<Shown> = shown
                .iter()
                .filter(|s| s.item != Furniture::Window)
                .copied()
                .collect();
            let clear = |x: i32, y: i32| free(&others, &|_, _| false, None, x, y);
            assert!(roomy(buf, &hung, &clear), "{at}");
            assert_eq!(room_to_look(buf, &hung, &clear), looks, "{at}");
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        /// With a poster hung among her pieces (and maybe her window,
        /// hung low): each lane keeps its anchor order and its walls, and
        /// never overlaps; what hangs never overlaps what stands, but
        /// what it may (her window, a sofa); a resize and back puts every
        /// piece where it was; a wall too low for what hangs never moves
        /// the room; and text closets only the pieces it would cover.
        #[test]
        fn the_wall_lane_keeps_order_and_never_moves_the_room(
            props in decorated(),
            wide in 30u16..90,
            tall in 8u16..14,
            narrow in 20u16..90,
            low in 4u16..8,
            text in proptest::collection::vec((1u16..90, 1u16..13), 0..5),
        ) {
            let mut room = Home { props, ..Default::default() };
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
                let &(_, e) = raw_strips(&nooks).iter().find(|(t, _)| Some(*t) == s.strip).unwrap();
                let r = s.rect();
                prop_assert!(i32::from(r.x) >= e.from && i32::from(r.right()) <= e.to, "{:?}", s);
                prop_assert!(i32::from(r.y) > e.floor - i32::from(e.rows) - 1, "inside: {:?}", s);
                for t in &laid {
                    if s.lane() == Lane::Wall && t.lane() == Lane::Floor {
                        prop_assert!(
                            !s.cover().intersects(t.cover()) || s.may_overlap(t),
                            "{:?} over {:?}", s, t
                        );
                        // Over anything, only her window over her sofa,
                        // and only its corner: she could lean on its sill
                        // from an end clear of the sofa.
                        if s.cover().intersects(t.cover()) {
                            prop_assert!((s.item, t.item) == (Furniture::Window, Furniture::Sofa), "{:?} over {:?}", s, t);
                            prop_assert!(
                                s.look_out_spots().iter().any(|&x| {
                                    her_box(x, s.floor).is_some_and(|b| !b.intersects(t.cover()))
                                }),
                                "{:?} wholly behind {:?}", s, t
                            );
                        }
                    }
                }
            }
            // A resize and back: where the narrower floor still holds
            // what stands on it, nothing moves and every piece shows
            // where it did; where it doesn't, they all go to the other
            // strip, or (with no room there) nothing moves.
            let (small, at_small) = two_panes(narrow, tall, &[]);
            let users_strip = Strip::Bottom(Nook::Users);
            let small_users = raw_strips(&at_small)
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
                // Only what that wall holds shows on it.
                let held = |s: &&Shown| s.lane() != Lane::Wall || s.size().1 + s.lift() + 2 <= low;
                prop_assert!(shown.iter().filter(users).all(|s| held(&s)), "{:?}", shown);
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

    // ---- Her door's space (the door batch, step 2) ----

    /// Her door's space as the geometry words have it, worked out from the
    /// nook's rect and the side alone (never read from [`Space::rect`]):
    /// the wall's own column `w` (right: the nook's last column; left:
    /// its first), the 6 columns inside it, her height and the floor row.
    fn space_by_hand(nook: Rect, side: Side) -> Rect {
        let floor = nook.bottom() - 1;
        let x = match side {
            Side::Right => nook.right() - 1 - 6,
            Side::Left => nook.x + 1,
        };
        Rect::new(x, floor - 4, 6, 5)
    }

    /// The frame's geometry: `nooks`, `chat` and `screen`.
    fn plan_on(nooks: &[(Nook, Rect)], chat: Rect, screen: Rect) -> Plan<'_> {
        Plan {
            nooks,
            chat,
            screen,
        }
    }

    /// Her door on `strip`'s `side`.
    fn door_on(nook: Nook, side: Side) -> Option<DoorWall> {
        Some(DoorWall {
            strip: Strip::Bottom(nook),
            side,
        })
    }

    /// The body of [`no_floor_piece_or_window_meets_her_door_space`]:
    /// `props` on two panes, her door on `nook`'s wall (`right` or left).
    fn door_space_is_clear(
        props: Vec<Prop>,
        wide: u16,
        tall: u16,
        nook: Nook,
        right: bool,
    ) -> Result<(), TestCaseError> {
        let (_, nooks) = two_panes(wide, tall, &[]);
        let side = if right { Side::Right } else { Side::Left };
        let mut room = Home {
            props,
            door: door_on(nook, side),
        };
        let laid = room.layout(&nooks);
        let rect = nooks.iter().find(|(n, _)| *n == nook).unwrap().1;
        let plan = room
            .extents(&nooks)
            .into_iter()
            .find(|p| p.strip == Strip::Bottom(nook))
            .unwrap();
        if let Some(space) = plan.space.filter(|s| s.kept) {
            let by_hand = space_by_hand(rect, side);
            prop_assert_eq!(space.rect, by_hand);
            for s in laid.iter().filter(|s| s.strip == Some(Strip::Bottom(nook))) {
                if s.lane() == Lane::Floor {
                    prop_assert!(!s.cover().intersects(by_hand), "{:?} in {:?}", s, by_hand);
                }
                if s.item == Furniture::Window {
                    prop_assert!(!s.rect().intersects(by_hand), "{:?} in {:?}", s, by_hand);
                }
            }
        }
        Ok(())
    }

    /// The body of [`a_home_packing_both_ways_keeps_its_anchors_and_order`]:
    /// `props` on two panes, her door given (or chosen by the first frame).
    fn packs_both_ways(
        props: Vec<Prop>,
        wide: u16,
        narrow: u16,
        door: Option<DoorWall>,
    ) -> Result<(), TestCaseError> {
        let mut room = Home { props, door };
        let (buf, nooks) = two_panes(wide, 12, &[]);
        let plan = Plan {
            nooks: &nooks,
            chat: Rect::default(),
            screen: buf.area,
        };
        let (first, _) = room.frame(&buf, plan, &|_, _| false);
        let pinned = room.clone();
        prop_assert!(room.props.iter().all(|p| p.anchor.is_some()));
        prop_assert!(room.door.is_some(), "a door chosen");
        let laid = room.layout(&nooks);
        prop_assert_eq!(&room, &pinned, "laying out moves nothing");
        let doorless = Home {
            door: None,
            ..pinned.clone()
        }
        .layout(&nooks);
        // In each lane, the same order along it.
        let items = |laid: &[Shown], lane: Lane| {
            let mut along: Vec<(i32, Furniture)> = laid
                .iter()
                .filter(|s| s.lane() == lane)
                .map(|s| (s.left, s.item))
                .collect();
            along.sort_by_key(|&(left, _)| left);
            along.into_iter().map(|(_, item)| item).collect::<Vec<_>>()
        };
        for lane in [Lane::Floor, Lane::Wall] {
            // Nothing laid out without her door is left out with it
            // (a space that would cost the strip a piece yields; one
            // may let her window hang where it found no place).
            let (with, without) = (items(&laid, lane), items(&doorless, lane));
            let both: Vec<Furniture> = with
                .iter()
                .copied()
                .filter(|i| without.contains(i))
                .collect();
            prop_assert_eq!(both, without);
        }
        let wall = room.door.unwrap();
        let strip_plan = room
            .extents(&nooks)
            .into_iter()
            .find(|p| p.strip == wall.strip)
            .unwrap();
        match strip_plan.space {
            Some(space) if !space.kept => prop_assert_eq!(&laid, &doorless, "the yield"),
            Some(_) => {
                let raw = raw_strips(&nooks)
                    .into_iter()
                    .find(|(s, _)| *s == wall.strip)
                    .unwrap()
                    .1;
                let (from, to) = match wall.side {
                    Side::Left => (raw.from + 6, raw.to),
                    Side::Right => (raw.from, raw.to - 6),
                };
                for s in laid
                    .iter()
                    .filter(|s| s.lane() == Lane::Floor && s.strip == Some(wall.strip))
                {
                    prop_assert!(
                        s.left >= from && i32::from(s.rect().right()) <= to,
                        "{:?} not in {}..{}",
                        s,
                        from,
                        to
                    );
                }
            }
            None => prop_assert_eq!(&laid, &doorless, "no space"),
        }
        let (small, at_small) = two_panes(narrow, 12, &[]);
        if raw_strips(&at_small)
            .iter()
            .any(|&(strip, e)| pack(&pinned.on(strip, Lane::Floor), e).is_none())
        {
            // Too narrow for what stands: they may move off.
            return Ok(());
        }
        let small_plan = Plan {
            nooks: &at_small,
            chat: Rect::default(),
            screen: small.area,
        };
        let _ = room.frame(&small, small_plan, &|_, _| false);
        prop_assert_eq!(&room, &pinned, "a resize moved a piece, or her door");
        let (again, changed) = room.frame(&buf, plan, &|_, _| false);
        prop_assert!(!changed);
        prop_assert_eq!(&again, &first);
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(128)))]

        /// Wherever her door's space is kept, nothing that stands meets
        /// it, nor does her window (hung low); a poster or her clock may
        /// hang above it. Her door drawn on either side of either strip;
        /// the space worked out by hand from the nook (T14).
        #[test]
        fn no_floor_piece_or_window_meets_her_door_space(
            props in decorated(),
            wide in 30u16..90,
            tall in 6u16..14,
            on_users in any::<bool>(),
            right in any::<bool>(),
        ) {
            let nook = if on_users { Nook::Users } else { Nook::Playlist };
            door_space_is_clear(props, wide, tall, nook, right)?;
        }

        /// [`no_floor_piece_or_window_meets_her_door_space`] with her
        /// pieces on the strip her door is drawn for, whichever it is (the
        /// other draws only half the time have any on it).
        #[test]
        fn no_floor_piece_or_window_meets_her_door_space_on_its_strip(
            props in decorated(),
            wide in 30u16..90,
            tall in 6u16..14,
            on_users in any::<bool>(),
            right in any::<bool>(),
        ) {
            let nook = if on_users { Nook::Users } else { Nook::Playlist };
            let props = props.into_iter().map(|p| Prop { strip: Strip::Bottom(nook), ..p }).collect();
            door_space_is_clear(props, wide, tall, nook, right)?;
        }

        /// A stage gift is laid out where it's set down, and never in her
        /// door's space as the strip is with it there, kept or not (the
        /// space judged with the gift on its strip, not before it).
        #[test]
        fn a_stage_gift_is_laid_out_clear_of_her_door_space(
            props in decorated(),
            wide in 24u16..40,
            gift in any::<proptest::sample::Index>(),
            seed in 0u64..1000,
            door in proptest::option::of((any::<bool>(), any::<bool>())),
        ) {
            let (buf, nooks) = two_panes(wide, 12, &[]);
            let plan = Plan { nooks: &nooks, chat: Rect::default(), screen: buf.area };
            let mut room = Home {
                props,
                door: door.map(|(users, right)| DoorWall {
                    strip: Strip::Bottom(if users { Nook::Users } else { Nook::Playlist }),
                    side: if right { Side::Right } else { Side::Left },
                }),
            };
            let (shown, _) = room.frame(&buf, plan, &|_, _| false);
            let gifts: Vec<Furniture> = Furniture::ALL.into_iter().filter(|&f| !room.owns(f)).collect();
            prop_assume!(!gifts.is_empty());
            let item = gifts[gift.index(gifts.len())];
            let Some(prop) = room.spot(&buf, plan, &shown, &|_, _| false, item, &mut Rng(seed)) else {
                return Ok(());
            };
            let mut with = room.clone();
            prop_assert!(with.add(prop));
            with.door = with.wall(plan);
            let laid = with.laid_and_shifted(&nooks);
            let at = laid.shown.iter().find(|s| s.item == item);
            prop_assert!(at.is_some(), "{:?} not laid out: {:?}", item, laid.shown);
            let space = laid.plan(prop.strip).and_then(|p| p.space);
            if let (Some(at), Some(space)) = (at, space) {
                prop_assert!(!at.cover().intersects(space.rect), "{:?} in {:?}", at, space);
            }
        }

        /// Her door's space never reorders her pieces or moves an anchor:
        /// laid out with it, they come in the order they do between the
        /// walls; a resize and back, frame by frame, restores her home,
        /// her door's wall with it; where it's kept, what stands on her
        /// door's strip stands between the far wall and the space (the
        /// strip less 6 columns on her door's side, worked out by hand);
        /// and where the floor's pieces pack only between the walls (the
        /// space yields), the layout is the doorless one, byte for byte
        /// (T15's guard; F2: no door means none).
        #[test]
        fn a_home_packing_both_ways_keeps_its_anchors_and_order(
            props in decorated(),
            wide in 30u16..90,
            narrow in 20u16..90,
            right in any::<bool>(),
        ) {
            let side = if right { Side::Right } else { Side::Left };
            packs_both_ways(props, wide, narrow, door_on(Nook::Users, side))?;
        }

        /// [`a_home_packing_both_ways_keeps_its_anchors_and_order`] with
        /// her door's wall chosen by her first frame, then kept through
        /// the resize and back.
        #[test]
        fn a_home_packing_both_ways_with_her_door_chosen(
            props in decorated(),
            wide in 30u16..90,
            narrow in 20u16..90,
        ) {
            packs_both_ways(props, wide, narrow, None)?;
        }

        /// With only what stands on its strip, whether her door's space
        /// is kept depends on what stands there, never on where along it
        /// each piece is anchored (T15): a layout computing it from where
        /// they stand would fail this. (A hung piece's place may decide
        /// it: see [`Home::extents`].)
        #[test]
        fn whether_the_space_is_kept_never_depends_on_anchors(
            props in pieces(),
            wide in 20u16..90,
            right in any::<bool>(),
            anchors in proptest::collection::vec((any::<bool>(), 0u16..60), 5),
        ) {
            let side = if right { Side::Right } else { Side::Left };
            let mut room = Home { props, door: door_on(Nook::Users, side) };
            let (_, nooks) = users(wide, &[]);
            room.layout(&nooks);
            let kept = |room: &Home| room.extents(&nooks)[0].space.map(|s| s.kept);
            let was = kept(&room);
            for (prop, &(right, offset)) in room.props.iter_mut().zip(&anchors) {
                prop.anchor = Some(Anchor {
                    side: if right { Side::Right } else { Side::Left },
                    offset,
                });
            }
            prop_assert_eq!(kept(&room), was);
        }
    }

    /// Of what hangs, only her window hangs low enough to meet her door's
    /// space (its rows are her height and the floor row): a poster or her
    /// clock hung over it clears it, an off-by-one's row above (the poster
    /// at hang 4) would not (T16).
    #[test]
    fn only_the_window_hangs_into_her_door_space() {
        let hung: Vec<Furniture> = Furniture::ALL
            .into_iter()
            .filter(|f| f.spec().hang.is_some())
            .collect();
        let mut checked = std::collections::HashSet::new();
        for rows in [5u16, 7, 11] {
            for side in [Side::Left, Side::Right] {
                let raw = Extent {
                    from: 1,
                    to: 49,
                    floor: i32::from(rows) + 1,
                    rows,
                };
                // By the geometry words: the 6 columns inside the wall
                // (the left wall is column `from - 1`, the right `to`),
                // the floor row and the 4 above it.
                let x = match side {
                    Side::Left => raw.from,
                    Side::Right => raw.to - 6,
                };
                let space = Rect::new(x as u16, (raw.floor - 4) as u16, 6, 5);
                for &item in &hung {
                    let prop = Prop::new(item, Nook::Users, 0, Facing::Right);
                    if !raw.holds(prop.needs()) {
                        continue;
                    }
                    checked.insert(item);
                    let at = stand(&prop, Strip::Bottom(Nook::Users), raw, x);
                    assert_eq!(
                        at.rect().intersects(space),
                        item == Furniture::Window,
                        "{item:?} {side:?} rows {rows}: {:?} {space:?}",
                        at.rect()
                    );
                }
            }
        }
        assert_eq!(checked.len(), hung.len(), "{checked:?} of {hung:?}");
    }

    /// Her door's wall is chosen once, from the panes' geometry: a wall at
    /// the screen's edge first (a layout with only a left edge too), then
    /// her pieces' strip among those, then where its space would be kept
    /// (only a tie-break: a crowded edge wall of her own strip is still
    /// hers, its space yielding; see `her_door_goes_where_its_space_is_kept`
    /// for the tie it breaks), right before left. Never a wall whose space
    /// meets the chat; an inner wall when no edge wall qualifies. Saved,
    /// it stays: a resize, her pane hidden while her pieces have nowhere
    /// to go and shown again, a frame too short, the chat coming to meet
    /// its space; only her pieces moving off its strip (her pane gone, or
    /// hidden while another strip takes them) takes her door with them,
    /// and it stays there when the pane shows again. A short frame saves
    /// nothing.
    #[test]
    fn the_door_wall_is_chosen_once() {
        let users = |props: &[(Furniture, Nook)]| {
            let mut home = Home::default();
            for &(item, nook) in props {
                assert!(home.add(Prop {
                    anchor: Some(Anchor {
                        side: Side::Left,
                        offset: 0,
                    }),
                    ..Prop::new(item, nook, 0, Facing::Right)
                }));
            }
            home
        };
        let side_by_side = |width: u16| {
            [
                (Nook::Users, Rect::new(0, 0, 50, 10)),
                (Nook::Playlist, Rect::new(50, 0, width, 10)),
            ]
        };
        let screen = Rect::new(0, 0, 100, 20);
        let nooks = side_by_side(50);
        // At the screen's edge first, and of those, her pieces' strip.
        let mine = users(&[(Furniture::Sofa, Nook::Playlist)]);
        assert_eq!(
            choose_wall(&mine, plan_on(&nooks, Rect::default(), screen)),
            door_on(Nook::Playlist, Side::Right)
        );
        let mine = users(&[(Furniture::Sofa, Nook::Users)]);
        assert_eq!(
            choose_wall(&mine, plan_on(&nooks, Rect::default(), screen)),
            door_on(Nook::Users, Side::Left)
        );
        // Only a left edge: it, before her pieces' strip's inner walls.
        let wider = Rect::new(0, 0, 120, 20);
        let theirs = users(&[(Furniture::Sofa, Nook::Playlist)]);
        assert_eq!(
            choose_wall(&theirs, plan_on(&nooks, Rect::default(), wider)),
            door_on(Nook::Users, Side::Left)
        );
        // No piece: edges first, in the panes' order.
        assert_eq!(
            choose_wall(&Home::default(), plan_on(&nooks, Rect::default(), screen)),
            door_on(Nook::Users, Side::Left)
        );
        // Crowded: her bed, sofa and TV (25 columns) on a 30-wide Users
        // pack between its walls (28), not beside a space (22); its edge
        // wall is still hers, and its space yields.
        let narrow = [
            (Nook::Users, Rect::new(0, 0, 30, 10)),
            (Nook::Playlist, Rect::new(30, 0, 70, 10)),
        ];
        let mut crowded = Home::default();
        for (item, at) in [
            (Furniture::Bed, 0),
            (Furniture::Sofa, 500),
            (Furniture::Tv, 1000),
        ] {
            assert!(crowded.add(Prop::new(item, Nook::Users, at, Facing::Right)));
        }
        crowded.layout(&narrow);
        let wall = choose_wall(&crowded, plan_on(&narrow, Rect::default(), screen));
        assert_eq!(wall, door_on(Nook::Users, Side::Left));
        crowded.door = wall;
        let space = crowded.extents(&narrow)[0].space.expect("room for a space");
        assert!(!space.kept, "it yields");
        // The chat over Users' left space: refused, the next edge wall.
        let chat = Rect::new(0, 0, 10, 10);
        assert_eq!(
            choose_wall(&mine, plan_on(&nooks, chat, screen)),
            door_on(Nook::Playlist, Side::Right)
        );
        // Only a left edge, the chat over its space: an inner wall, of her
        // pieces' strip, the right before the left.
        assert_eq!(
            choose_wall(&mine, plan_on(&nooks, chat, wider)),
            door_on(Nook::Users, Side::Right)
        );
        // A chat across every space: none.
        let band = Rect::new(0, 5, 100, 1);
        for (nook, rect) in nooks {
            for side in [Side::Left, Side::Right] {
                assert!(
                    space_by_hand(rect, side).intersects(band),
                    "{nook:?} {side:?}"
                );
            }
        }
        assert_eq!(choose_wall(&mine, plan_on(&nooks, band, screen)), None);
        // Saved once she has a piece; then kept through a resize, a plan
        // without her pane (settling alone moves nothing), a short frame
        // and the chat coming over its space.
        let mut home = users(&[(Furniture::Sofa, Nook::Users)]);
        let mut empty = Home::default();
        assert!(!empty.settle_door(plan_on(&nooks, Rect::default(), screen)));
        assert_eq!(empty.door, None, "nothing to have a door for");
        assert!(home.settle_door(plan_on(&nooks, Rect::default(), screen)));
        let saved = home.door;
        assert_eq!(saved, door_on(Nook::Users, Side::Left));
        let resized = side_by_side(30);
        assert!(!home.settle_door(plan_on(&resized, Rect::default(), Rect::new(0, 0, 80, 20))));
        let hidden = [(Nook::Playlist, Rect::new(50, 0, 50, 10))];
        assert!(!home.settle_door(plan_on(&hidden, Rect::default(), screen)));
        assert!(!home.settle_door(plan_on(&nooks, chat, screen)));
        assert_eq!(
            home.wall(plan_on(&nooks, chat, screen)),
            saved,
            "refused where it stands, not here"
        );
        assert!(!home.settle_door(plan_on(&nooks, Rect::default(), screen)));
        assert_eq!(home.door, saved);
        // A short frame (3 rows over the floor) saves nothing.
        let short = [
            (Nook::Users, Rect::new(0, 0, 50, 5)),
            (Nook::Playlist, Rect::new(50, 0, 50, 5)),
        ];
        let mut fresh = users(&[(Furniture::Sofa, Nook::Users)]);
        assert!(!fresh.settle_door(plan_on(&short, Rect::default(), screen)));
        assert_eq!(fresh.door, None);
        // Her pieces moving off its strip take her door with them: Users
        // gone, they go to Playlist, and her door to its edge wall.
        let (buf, two) = two_panes(30, 10, &[]);
        let mut moving = users(&[(Furniture::Sofa, Nook::Users)]);
        let full = Plan {
            nooks: &two,
            chat: Rect::default(),
            screen: buf.area,
        };
        let _ = moving.frame(&buf, full, &|_, _| false);
        assert_eq!(moving.door, door_on(Nook::Users, Side::Left));
        // Hidden, with nowhere to go (the Playlist pane too narrow for
        // her sofa, 8 columns between its walls): nothing moves, her
        // door stays.
        let tiny = [(Nook::Playlist, Rect::new(30, 0, 10, 12))];
        let hide = Plan {
            nooks: &tiny,
            chat: Rect::default(),
            screen: buf.area,
        };
        let _ = moving.frame(&buf, hide, &|_, _| false);
        assert_eq!(
            moving.door,
            door_on(Nook::Users, Side::Left),
            "nothing moved"
        );
        assert_eq!(moving.props[0].strip, Strip::Bottom(Nook::Users));
        // Gone, with room on Playlist: moved, and her door with them.
        let gone = [(Nook::Playlist, Rect::new(30, 0, 40, 12))];
        let away = Plan {
            nooks: &gone,
            chat: Rect::default(),
            screen: buf.area,
        };
        let _ = moving.frame(&buf, away, &|_, _| false);
        assert_eq!(moving.props[0].strip, Strip::Bottom(Nook::Playlist));
        assert_eq!(moving.door, door_on(Nook::Playlist, Side::Right));
        // Shown again: her door stays where it went (with her pieces).
        let _ = moving.frame(&buf, full, &|_, _| false);
        assert_eq!(moving.door, door_on(Nook::Playlist, Side::Right));
    }

    /// Panes side by side, `height` tall, each `(nook, width)`, drawn
    /// empty on one buffer; with their rects.
    fn side_panes(panes: &[(Nook, u16)], height: u16) -> (Buffer, Vec<(Nook, Rect)>) {
        let mut rows = vec![String::new(); usize::from(height)];
        let mut nooks = Vec::new();
        let mut x = 0;
        for &(nook, width) in panes {
            for (row, line) in
                rows.iter_mut()
                    .zip(empty("P", usize::from(width), usize::from(height)))
            {
                row.push_str(&line);
            }
            nooks.push((nook, Rect::new(x, 0, width, height)));
            x += width;
        }
        (Buffer::with_lines(rows.iter().map(String::as_str)), nooks)
    }

    /// Her pieces moved off her door's strip are laid out, the frame
    /// they move, with her door where it went: the next frame shows them
    /// where this one did (no hop of the space's width a frame later),
    /// and none stands in her door's new space (finding: the door was
    /// re-chosen after the layout, judged on the old one).
    #[test]
    fn her_pieces_moving_with_her_door_stand_once_where_they_stay() {
        let (buf, nooks) = side_panes(&[(Nook::Users, 30), (Nook::Playlist, 40)], 12);
        let mut home = Home::default();
        for (item, offset) in [(Furniture::Tv, 0), (Furniture::Sofa, 8)] {
            assert!(home.add(Prop {
                anchor: Some(Anchor {
                    side: Side::Right,
                    offset,
                }),
                ..Prop::new(item, Nook::Users, 0, Facing::Left)
            }));
        }
        let full = plan_on(&nooks, Rect::default(), buf.area);
        let _ = home.frame(&buf, full, &|_, _| false);
        assert_eq!(home.door, door_on(Nook::Users, Side::Left));
        // Users hidden: her pieces go to Playlist, anchored from its
        // right wall, the screen's edge, where her door goes too.
        let playlist = [nooks[1]];
        let hidden = plan_on(&playlist, Rect::default(), buf.area);
        let (moved, _) = home.frame(&buf, hidden, &|_, _| false);
        assert_eq!(home.door, door_on(Nook::Playlist, Side::Right));
        assert_eq!(moved.len(), 2, "{moved:?}");
        let space = home.extents(&playlist)[0].space.expect("a space");
        assert!(space.kept);
        for s in &moved {
            assert!(!s.cover().intersects(space.rect), "{s:?} in {space:?}");
        }
        let (next, changed) = home.frame(&buf, hidden, &|_, _| false);
        assert!(!changed);
        assert_eq!(next, moved, "they hop a frame later");
    }

    /// Of two edge walls, both on her pieces' strips, her door goes where
    /// keeping its space costs the strip nothing, against the panes'
    /// order: her bed, sofa and TV (25 columns) crowd a 30-wide pane (28
    /// between its walls, 22 beside a space), her lamp alone on the other.
    /// Either way round (D2's third key).
    #[test]
    fn her_door_goes_where_its_space_is_kept() {
        for crowded_first in [true, false] {
            let (crowd, roomy) = if crowded_first {
                (Nook::Users, Nook::Playlist)
            } else {
                (Nook::Playlist, Nook::Users)
            };
            let (buf, nooks) = side_panes(&[(crowd, 30), (roomy, 30)], 12);
            let mut home = Home::default();
            for (item, nook, at) in [
                (Furniture::Bed, crowd, 0),
                (Furniture::Sofa, crowd, 500),
                (Furniture::Tv, crowd, 1000),
                (Furniture::Lamp, roomy, 500),
            ] {
                assert!(home.add(Prop::new(item, nook, at, Facing::Right)));
            }
            home.layout(&nooks);
            let wall = choose_wall(&home, plan_on(&nooks, Rect::default(), buf.area));
            assert_eq!(wall, door_on(roomy, Side::Right), "{crowd:?} crowded");
            // The other one would yield.
            let yields = Home {
                door: door_on(crowd, Side::Left),
                ..home.clone()
            };
            assert!(!yields.extents(&nooks)[0].space.expect("a space").kept);
        }
    }

    /// Her pieces never move onto the chat pane: with her door's pane gone
    /// and the only other strip's floor, where they'd stand, under the
    /// chat, nothing moves and her door stays; with the chat clear of
    /// where they'd stand, they move, her door with them.
    #[test]
    fn her_pieces_never_move_under_the_chat() {
        let (buf, nooks) = side_panes(&[(Nook::Users, 30), (Nook::Playlist, 40)], 12);
        let mut home = Home::default();
        assert!(home.add(Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset: 0,
            }),
            ..Prop::new(Furniture::Sofa, Nook::Users, 0, Facing::Right)
        }));
        home.door = door_on(Nook::Users, Side::Left);
        let playlist = [nooks[1]];
        // The chat over Playlist's left end, where her sofa would stand.
        let over = Rect::new(30, 0, 15, 12);
        let mut stays = home.clone();
        let _ = stays.frame(&buf, plan_on(&playlist, over, buf.area), &|_, _| false);
        assert_eq!(stays, home, "moved under the chat");
        // The chat over the rows above it: they move.
        let above = Rect::new(30, 0, 40, 3);
        let mut moves = home.clone();
        let (shown, _) = moves.frame(&buf, plan_on(&playlist, above, buf.area), &|_, _| false);
        assert_eq!(moves.props[0].strip, Strip::Bottom(Nook::Playlist));
        assert_eq!(moves.door, door_on(Nook::Playlist, Side::Right));
        assert!(
            shown.iter().all(|s| !s.cover().intersects(above)),
            "{shown:?}"
        );
    }

    /// [`a_stage_gift_is_laid_out_clear_of_her_door_space`]'s find (its
    /// regressions file has it): on a 29-wide Users pane, her poster,
    /// desk, fridge and window there, her door coming to Users' left wall
    /// with a gifted lamp. Judged on the strip before the lamp, its places
    /// start past the space; with it there, the floor no longer packs
    /// beside the space, which yields, and the lamp anchored from the
    /// space's edge stands six columns nearer the wall, in the space. It
    /// goes where it's laid clear of it.
    #[test]
    fn a_gifted_lamp_never_stands_in_a_space_its_coming_yields() {
        let (buf, nooks) = two_panes(29, 12, &[]);
        let plan = Plan {
            nooks: &nooks,
            chat: Rect::default(),
            screen: buf.area,
        };
        let anchored = |item, side, offset| Prop {
            anchor: Some(Anchor { side, offset }),
            ..Prop::new(item, Nook::Users, 0, Facing::Right)
        };
        let mut room = Home {
            props: vec![
                anchored(Furniture::Poster, Side::Left, 8),
                Prop::new(Furniture::Desk, Nook::Users, 700, Facing::Right),
                anchored(Furniture::Fridge, Side::Right, 16),
                anchored(Furniture::Window, Side::Left, 9),
            ],
            door: None,
        };
        let (shown, _) = room.frame(&buf, plan, &|_, _| false);
        let lamp = room
            .spot(
                &buf,
                plan,
                &shown,
                &|_, _| false,
                Furniture::Lamp,
                &mut Rng(893),
            )
            .expect("a place for it");
        let mut with = room.clone();
        assert!(with.add(lamp));
        with.door = with.wall(plan);
        let laid = with.laid_and_shifted(&nooks);
        let at = laid
            .shown
            .iter()
            .find(|s| s.item == Furniture::Lamp)
            .expect("laid out");
        if let Some(space) = laid.plan(lamp.strip).and_then(|p| p.space) {
            assert!(!at.cover().intersects(space.rect), "{at:?} in {space:?}");
        }
    }

    /// Her door follows her pieces to the strip they move to, even when
    /// another strip has a better wall (one at the screen's edge): the
    /// door goes with the room (D2).
    #[test]
    fn her_door_follows_her_pieces_not_the_best_wall() {
        // Playlist at the screen's left edge; Users (her door's) hidden;
        // List first among the panes, where her pieces go, with only
        // inner walls (the screen runs on past it).
        let (buf, nooks) = side_panes(
            &[(Nook::Playlist, 40), (Nook::Users, 40), (Nook::List, 40)],
            12,
        );
        let screen = Rect::new(0, 0, 200, 12);
        let mut home = Home::default();
        assert!(home.add(Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset: 0,
            }),
            ..Prop::new(Furniture::Sofa, Nook::Users, 0, Facing::Right)
        }));
        home.door = door_on(Nook::Users, Side::Right);
        let rest = [nooks[2], nooks[0]];
        let _ = home.frame(&buf, plan_on(&rest, Rect::default(), screen), &|_, _| false);
        assert_eq!(home.props[0].strip, Strip::Bottom(Nook::List));
        assert_eq!(home.door, door_on(Nook::List, Side::Right));
    }
}
