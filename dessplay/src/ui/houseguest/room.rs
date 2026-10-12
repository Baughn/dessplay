//! Her room: furniture she owns, placed around the panes.
//!
//! Props are stored **pane-relative** — a quiet pane's floor and a
//! position along it — and resolved against each frame, so resizes and
//! layout changes carry her room along. A prop stands on its pane's
//! bottom border over blank cells only; when it doesn't fit (the pane
//! is gone, too small, or has text where the prop would stand), it is
//! in the closet for that frame: hidden, still owned.

use std::cell::OnceCell;

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Color;

use super::Rng;
use super::cells::{untouchable, width};
use super::door::Keep;
use super::graphics::strokes;
use super::ledger::Dated;
use super::sprite::{self, Facing};
use super::structures::{
    self, Drawn, EDGE, HIGH_RISE, KeepOff, LOW_ABOVE, LOW_RISE, MARGIN, SHELF_STAGES, SHELF_WIDTH,
    Shelf, Structure,
};

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
    /// A flat-pack shelf, boxed (phase 6a D7b): never unpacked into a
    /// piece; opened, it goes up on her wall as her shelf
    /// ([`Home::raise`]).
    ShelfKit,
    /// A gremlin figurine (phase 6a D9): small decor, glazed, grinning;
    /// he wants up high, on her shelf ([`Furniture::wants_up`]).
    Gremlin,
}

impl Furniture {
    /// Every piece, in the order they were added (their serde names are
    /// stable; the first eight are the oldest builds', see
    /// [`Furniture::legacy`]). What the channel sells, in order, is
    /// `CATALOGUE`.
    pub const ALL: [Self; 14] = [
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
        Self::ShelfKit,
        Self::Gremlin,
    ];

    /// Every piece before her kits (phase 6a step 5b): today's `ALL` as
    /// it was, which every older test's strategy, proptest body and kind
    /// lint draws from, so their seeds keep meaning what they meant
    /// (appending to a strategy's domain changes it). New kinds get new
    /// tests.
    #[cfg(test)]
    pub(super) const PRE_SHELF: [Self; 12] = [
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

    /// Whether it's a kit (phase 6a D7b): always its box, opened only to
    /// go up as a structure.
    pub(super) fn is_kit(self) -> bool {
        self.spec().kit.is_some()
    }

    /// Whether she gazes up at it standing on her shelf (phase 6a D8):
    /// her plant, and her gremlin (D9). Not her clock: a mantel clock is glanced up at for the
    /// time, as on her wall (phase 5b), never gazed at; nor anything that
    /// never goes up there. Wildcard-free, so a new kind says.
    pub(super) fn gazed_up_at(self) -> bool {
        match self {
            Self::Plant | Self::Gremlin => true,
            Self::Sofa
            | Self::Tv
            | Self::Bed
            | Self::Desk
            | Self::Lamp
            | Self::Bookshelf
            | Self::Fridge
            | Self::CatBed
            | Self::Poster
            | Self::Clock
            | Self::Window
            | Self::ShelfKit => false,
        }
    }

    /// Whether it wants up high, on a shelf (phase 6a D9: her gremlin):
    /// the rule [`super::rules::Rule::High`] names it, it's sold only
    /// where a shelf stands or could go up, and a shelf goes up by it.
    /// Wildcard-free, so a new kind says.
    pub(super) fn wants_up(self) -> bool {
        match self {
            Self::Gremlin => true,
            Self::Sofa
            | Self::Tv
            | Self::Bed
            | Self::Desk
            | Self::Lamp
            | Self::Bookshelf
            | Self::Fridge
            | Self::CatBed
            | Self::Plant
            | Self::Poster
            | Self::Clock
            | Self::Window
            | Self::ShelfKit => false,
        }
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
            Self::Plant
            | Self::Poster
            | Self::Clock
            | Self::Window
            | Self::ShelfKit
            | Self::Gremlin => false,
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
            Self::ShelfKit => &SHELF_KIT,
            Self::Gremlin => &GREMLIN,
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
    /// Small decor: it may stand on a shelf (phase 6a; see
    /// [`Strip::takes`]). Never furniture.
    pub shelf: bool,
    /// A kit (phase 6a D7b): what it puts up, never unpacked into a piece
    /// ([`Furniture::is_kit`]).
    pub kit: Option<Kit>,
}

/// What a kit puts up (phase 6a D7b; 6b and 6c add theirs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kit {
    /// Her shelf ([`Shelf`]).
    Shelf,
}

impl Kit {
    /// Whether `structure` is what it puts up.
    fn puts_up(self, structure: Structure) -> bool {
        match (self, structure) {
            (Self::Shelf, Structure::Shelf(_)) => true,
        }
    }
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
    shelf: false,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: true,
    kit: None,
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
    shelf: false,
    kit: None,
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
    shelf: true,
    kit: None,
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
    shelf: false,
    kit: None,
};
const SHELF_KIT: Spec = Spec {
    name: "shelf kit",
    pitch: line!("A shelf! Up high!"),
    // As wide as a parcel and as tall, so its box shows whole.
    footprint: (5, 2),
    // A flat pack (only ever drawn as its box; the noise her goodbye
    // bursts it into).
    ascii: &["_____", "[===]"],
    ink: (Color::Rgb(192, 150, 100), Color::Yellow),
    uses: &[],
    offers: &[],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 0.0,
    symmetric: false,
    shelf: false,
    kit: Some(Kit::Shelf),
};
const GREMLIN: Spec = Spec {
    name: "gremlin",
    pitch: line!("Aww, a little gremlin!"),
    // A figurine on a plinth (art/props.svg, `gremlin`: variant A of its
    // model sheet).
    footprint: (2, 2),
    // His eyes over his grin (mirror-safe).
    ascii: &["oo", "ww"],
    ink: (Color::Rgb(178, 154, 224), Color::LightMagenta),
    // Gazed up at on her shelf (`Use::GazeUp`), which no kind's uses
    // name: it's offered by where he stands.
    uses: &[],
    offers: &[Offer::Decor],
    sit: None,
    comfort: 1.0,
    hang: None,
    beauty: 1.0,
    symmetric: false,
    shelf: true,
    kit: None,
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
    /// Stand on her shelf's floor beside a piece up on it, gazing up at
    /// it (phase 6a D8): what [`Furniture::gazed_up_at`] says, on a
    /// shelf.
    GazeUp,
}

impl Use {
    /// Every use.
    #[cfg(test)]
    pub const ALL: [Self; 12] = [
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
        Self::GazeUp,
    ];

    /// Whether she uses it from in it (sits on it, lies in it), rather
    /// than from beside it (or, looking out of the window, from under
    /// it).
    pub fn inside(self) -> bool {
        !matches!(
            self,
            Use::Watch | Use::Read | Use::Snack | Use::Pet | Use::LookOut | Use::GazeUp
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
    ///
    /// Nor to gaze up at a piece on her shelf (phase 6a D8): it stands
    /// where her shelf is, over her floor, never in her way.
    pub fn asks_room(self) -> bool {
        !matches!(self, Use::LookOut | Use::GazeUp)
    }

    /// Whether it's a still use, whose length her mood lingers over
    /// (phase 5c M8: see [`Stillness::linger`]): lounging, napping,
    /// sleeping (by day: her night has its own wake), reading, looking
    /// out, gazing up at her shelf (phase 6a D8), and watching TV, now
    /// that it holds a picture after its switch-on (phase 5c D7). Not
    /// chores (unpacking, crumpling, a
    /// snack, petting the cat), nor homework, whose nod-off her mood
    /// moves instead.
    ///
    /// [`Stillness::linger`]: super::stillness::Stillness::linger
    pub fn lingers(self) -> bool {
        match self {
            Use::Lounge
            | Use::Nap
            | Use::Sleep
            | Use::Read
            | Use::LookOut
            | Use::GazeUp
            | Use::Watch => true,
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

/// The role `strip`'s room has as laid out: from what's out of its box
/// there (its pane's floor and any shelf above it: [`Strip::room`]).
pub(super) fn role_of(layout: &[Shown], strip: Strip) -> Role {
    role_among(layout, strip, None)
}

/// The role `strip`'s room would have as laid out without `piece`
/// (whether a piece completes its room, or spoils it).
pub(super) fn role_without(layout: &[Shown], strip: Strip, piece: Furniture) -> Role {
    role_among(layout, strip, Some(piece))
}

/// Whether `s` is in `strip`'s room ([`Strip::room`]): rooms are floors.
pub(super) fn in_room(s: &Shown, strip: Strip) -> bool {
    s.strip.map(Strip::room) == Some(strip.room())
}

fn role_among(layout: &[Shown], strip: Strip, without: Option<Furniture>) -> Role {
    let offers: Vec<Offer> = layout
        .iter()
        .filter(|s| in_room(s, strip) && !s.boxed() && s.scrap.is_none())
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
    /// The shelf she put up in `nook`'s pane (one a pane at most; phase
    /// 6a). Its floor is the shelf's run; it's part of the room below
    /// ([`Strip::room`]), never a room of its own.
    Shelf(Nook),
}

impl Strip {
    /// The pane it's in: every strip has one.
    pub fn nook(self) -> Nook {
        match self {
            Self::Bottom(nook) | Self::Shelf(nook) => nook,
        }
    }

    /// The room it's part of: its pane's floor (a shelf is part of the
    /// room below it, never a room of its own).
    pub fn room(self) -> Strip {
        Self::Bottom(self.nook())
    }

    /// Whether it's a pane's floor (not a shelf): a room of its own.
    pub fn is_floor(self) -> bool {
        matches!(self, Self::Bottom(_))
    }

    /// Whether `item` (`boxed`, or out of its box) may stand on it: a
    /// floor takes anything; a shelf, small decor only ([`Spec::shelf`])
    /// and never a parcel (phase 6a D2: one comes in on a floor, where
    /// she unpacks it). The one admission rule: [`Home::on`] lays out
    /// nothing a strip doesn't take, every enumeration of places asks
    /// [`StripPlan::takes`] first, and her record puts nothing on a shelf
    /// that doesn't take it.
    pub fn takes(self, item: Furniture, boxed: bool) -> bool {
        match self {
            Self::Bottom(_) => true,
            Self::Shelf(_) => item.spec().shelf && !boxed,
        }
    }
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
/// strip; `door`, the flap in her own door, standing in its space by
/// her door's wall (door batch D9), else one in the wall itself (a
/// fallback); `item`, what's in the parcel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Flap {
    pub x: i32,
    pub rows: (i32, i32),
    pub side: Side,
    pub door: bool,
    pub item: Furniture,
}

/// Whether a piece is still in its delivery box, as it was set (a
/// delivery, the record, her unpacking it). The bit is room.rs's own:
/// everything else reads it through [`Prop::boxed`] and
/// [`Shown::boxed`], which say a kit is always its box (phase 6a D2: a
/// kit is never unpacked into a piece; it goes up as a structure), so no
/// path, scratch, stage or record, shows or judges an unboxed kit,
/// whatever was set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub(super) struct Boxed(bool);

impl From<bool> for Boxed {
    fn from(boxed: bool) -> Self {
        Self(boxed)
    }
}

impl std::fmt::Debug for Boxed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// A piece she owns, standing on `strip`; boxed until she unpacks it
/// ([`Prop::boxed`]).
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
    /// As set: read it through [`Prop::boxed`].
    pub boxed: Boxed,
    /// She has set it where it stands (a delivery hasn't: it stands
    /// where it came in, until she moves it).
    pub settled: bool,
}

/// How high a piece of hers stands (phase 6a D7c, step 8): down on a
/// floor; up on a low shelf, in her reach (she reaches up, arms up); or
/// up on a high one (a little hop as it goes up).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Height {
    Floor,
    Low,
    High,
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

/// The rows between the floor of `strip` and the bottom row of `item`,
/// out of its box or not: its hang, once it's out of a box (a parcel
/// stands on the floor); 0 for a piece that stands, and on a shelf,
/// where everything stands (phase 6a D1: a clock on a shelf is a mantel
/// clock). Off any strip (none: as a floor).
fn lift(item: Furniture, boxed: bool, strip: Option<Strip>) -> u16 {
    match strip {
        Some(Strip::Shelf(_)) => 0,
        Some(Strip::Bottom(_)) | None if boxed => 0,
        Some(Strip::Bottom(_)) | None => item.spec().hang.unwrap_or(0),
    }
}

/// The lane a piece lifted `lift` rows off its floor is in.
fn lane_of(lift: u16) -> Lane {
    if lift > 0 { Lane::Wall } else { Lane::Floor }
}

impl Prop {
    /// Whether it's still in its delivery box: a kit always is.
    pub fn boxed(&self) -> bool {
        self.boxed.0 || self.item.is_kit()
    }

    /// Which lane of its strip it's in: a piece that hangs is on the
    /// wall once it's out of its box.
    pub fn lane(&self) -> Lane {
        self.lane_on(self.strip)
    }

    /// Which lane it would be in on `strip` (a place a search weighs for
    /// it: a candidate strip's lane, not its own).
    pub fn lane_on(&self, strip: Strip) -> Lane {
        lane_of(lift(self.item, self.boxed(), Some(strip)))
    }

    /// The columns it takes along its strip, and the rows above the floor
    /// it needs clear (up to its top, hung or standing).
    fn needs(&self) -> (u16, u16) {
        let (cols, rows) = self.item.spec().footprint;
        (cols, rows + lift(self.item, self.boxed(), Some(self.strip)))
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
            boxed: false.into(),
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

/// A quiet pane's floor this frame: the pane `nook`, drawn at `rect`,
/// and its strip (its bottom border) `raw` between its walls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PaneStrip {
    pub nook: Nook,
    pub rect: Rect,
    pub strip: Strip,
    pub raw: Extent,
}

/// The floors of the quiet panes this frame, in `nooks`' order, each
/// between its walls: the panes' own strips, never a shelf (phase 6a
/// D1), so whatever pairs a pane with its floor reads both from here.
/// Raw: a strip's floor lane is narrowed by her door's space (see
/// [`Home::extents`]), which every placement reads; only pinning (an
/// anchor's share of the way along, as older builds read it) and the
/// two readers of "which room is she in" (`clock_on`, `beauty_at` in
/// mod.rs, where the space is as much the room as the rest) read this.
/// Every floors-only path (her door's wall, a delivery's flap, a
/// strip's pieces moving off, a stage gift) iterates these.
pub(super) fn pane_strips(nooks: &[(Nook, Rect)]) -> Vec<PaneStrip> {
    nooks
        .iter()
        .map(|&(nook, rect)| PaneStrip {
            nook,
            rect,
            strip: Strip::Bottom(nook),
            raw: pane_raw(rect),
        })
        .collect()
}

/// A pane's raw floor strip (see [`pane_strips`]): inside its borders,
/// on its bottom border's row.
pub(super) fn pane_raw(rect: Rect) -> Extent {
    Extent {
        from: i32::from(rect.x) + 1,
        to: i32::from(rect.right()) - 1,
        floor: i32::from(rect.bottom()) - 1,
        rows: rect.height.saturating_sub(2),
    }
}

/// A strip's geometry this frame, before her door's space: `raw`, its
/// own extent between its ends; `host`, its pane's floor's (the floor
/// she stands on to reach what's on it: its own, for a pane's floor),
/// what an anchor's share of the way along is of, both ways
/// ([`StripPlan::pin`] and [`RawStrip::anchor_at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RawStrip {
    pub strip: Strip,
    pub raw: Extent,
    pub host: Extent,
}

impl RawStrip {
    /// The anchor on this strip of a piece `cols` wide `at` thousandths
    /// of the way along its `host` (an older record's share, as
    /// [`StripPlan::pin`] writes it): where the share puts it on the
    /// host, kept on the raw strip.
    pub(super) fn anchor_at(&self, at: u16, cols: u16) -> Anchor {
        self.raw.pin(self.host.share(at, cols), cols).0
    }
}

/// The columns her door's space takes along a strip, against its wall.
pub(super) const SPACE: i32 = 6;

/// Her external door's wall: a side of one strip (saved). Always a
/// pane's floor: only [`choose`] makes one, from [`pane_strips`], and
/// a record naming a shelf doesn't read (phase 6a D1; a `Nook` in
/// memory would touch every comparison for a state no code builds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "SavedDoorWall")]
pub(super) struct DoorWall {
    pub strip: Strip,
    pub side: Side,
}

/// [`DoorWall`] as a record has it, before its strip is checked.
#[derive(serde::Deserialize)]
struct SavedDoorWall {
    strip: Strip,
    side: Side,
}

impl TryFrom<SavedDoorWall> for DoorWall {
    type Error = &'static str;

    fn try_from(saved: SavedDoorWall) -> Result<Self, Self::Error> {
        match saved.strip {
            Strip::Bottom(_) => Ok(Self {
                strip: saved.strip,
                side: saved.side,
            }),
            Strip::Shelf(_) => Err("her door is never on a shelf"),
        }
    }
}

/// A strip as this frame lays it out: `raw` between its walls; `floor`,
/// what its floor lane packs on (raw less her door's space, when it's
/// kept); `space`, her door's reserved rect on this strip, if it can
/// exist at all; `host`, its pane's floor's raw extent (its own `raw`,
/// for a pane's floor): where she stands to reach what's on it, and
/// what an anchor's share of the way along is of; `block`, on a pane's
/// floor under a low shelf, the shelf's block, which nothing hung there
/// meets ([`Shelf::block`], phase 6a D3a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StripPlan {
    pub strip: Strip,
    pub raw: Extent,
    pub floor: Extent,
    pub space: Option<Space>,
    pub host: Extent,
    pub block: Option<Rect>,
}

impl StripPlan {
    /// Whether `item` may stand here ([`Strip::takes`]): asked by every
    /// enumeration of places before it weighs one.
    pub(super) fn takes(&self, item: Furniture, boxed: bool) -> bool {
        self.strip.takes(item, boxed)
    }

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
    /// along `host` (as older builds read it: shares are always of the
    /// raw pane floor's).
    pub(super) fn pin(&self, lane: Lane, left: i32, cols: u16) -> (Anchor, u16) {
        (
            self.along(lane).pin(left, cols).0,
            self.host.pin(left, cols).1,
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

/// One strip laid out alone (see [`Home::strips_laid`]): its plan, and
/// what it lays out, if she has a piece on it and what stands there
/// packs; and what it was laid out from (her pieces on it, by index,
/// her door's wall if it's on it, and her shelf in its pane: all its
/// layout reads, see [`Home::lays_from`]), so it's never taken for the
/// layout of a home whose strip differs ([`Home::relaid`]).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct StripLaid {
    pub plan: StripPlan,
    laid: Option<LaidOut>,
    from: Vec<(usize, Prop)>,
    door: Option<DoorWall>,
    shelf: Option<Shelf>,
}

/// One of her pieces in a lane of a strip, as [`pack`] takes it: by
/// index, its anchor, and its columns and the rows it needs clear.
type Laned = (usize, Anchor, (u16, u16));

/// The strips [`Home::relaid_keeping`] has laid out again, each by its
/// place in the `was` it lays out from: the latest of each (`kept`), and
/// which the latest move laid out from other than `was` did (`again`).
#[derive(Debug, Default)]
pub(super) struct LaidAgain {
    kept: Vec<Option<StripLaid>>,
    again: Vec<bool>,
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

/// Whether `rect` meets the chat pane laid out at `chat`. An empty chat
/// rect is no chat, wherever it lies: ratatui's `intersects` alone says
/// a zero-high or zero-wide rect away from the origin meets whatever
/// straddles its line.
pub(super) fn in_chat(chat: Rect, rect: Rect) -> bool {
    !chat.is_empty() && !rect.is_empty() && chat.intersects(rect)
}

/// Her pieces `laid` (her home `home`) has in the chat pane (`chat`)
/// that no room would take (door batch, D7: the closet), as indices into
/// `laid.shown`: see [`placements`]. Pure.
pub(super) fn stranded(home: &Home, laid: &LaidOut, chat: Rect) -> Vec<usize> {
    placements(home, laid, chat)
        .into_iter()
        .filter_map(|(i, to)| to.is_none().then_some(i))
        .collect()
}

/// Whether `s` is one of her pieces in the chat pane (`chat`) that the
/// closet weighs ([`placements`]), and the chat rule judges: on a pane's
/// floor, real, out of its box. A box in the chat is never closeted: it
/// shows where it stands, for her to unpack (then it's a piece like any
/// other). Nor is a piece on a shelf weighed (phase 6a D6): a shelf the
/// chat comes to meet is hidden with what's on it, so no repair carries
/// a piece down off one for the chat.
pub(super) fn closetable(chat: Rect, s: &Shown) -> bool {
    s.strip.is_some_and(Strip::is_floor)
        && s.scrap.is_none()
        && !s.boxed()
        && in_chat(chat, s.cover())
}

/// Where each of her pieces `laid` has in the chat pane (`chat`) could
/// go, if anywhere (door batch, D7), as its index into `laid.shown` and
/// the strip and left: those [`closetable`], taken in layout order, and
/// assigned jointly. On its own strip, any left along its lane where it
/// would miss the chat and her door's space (its neighbours pack round
/// it); else on another strip whose lane holds it (and where it would
/// leave a kept door space room to be), anchored anywhere along that
/// lane, where it [`pack`]s with the pieces there (anchored as `home`
/// has them; those in the chat that are weighed themselves left out)
/// and those already assigned there: it misses the chat and the space
/// (kept or not), and none of the others is pushed newly into them
/// (as [`super::door::Keep::takes_in`]). Own strip first, then the
/// strips in order. A piece with none is stranded: in the closet this
/// frame (hidden, never moved). Only a cheap stand-in for "a repair
/// could move it": one some room takes is shown in the chat until a
/// repair moves it, which needs a mood that mends (the documented gap,
/// M20).
pub(super) fn placements(
    home: &Home,
    laid: &LaidOut,
    chat: Rect,
) -> Vec<(usize, Option<(Strip, i32)>)> {
    // Assigned so far: the piece's index among her props, anchor, needs
    // and where it's taken to.
    let mut taken: Vec<(usize, Anchor, (u16, u16), Shown)> = Vec::new();
    let mut out = Vec::new();
    for (i, piece) in laid.shown.iter().enumerate() {
        if !closetable(chat, piece) {
            continue;
        }
        let Some(own) = piece.strip else { continue };
        let found = laid
            .plans
            .iter()
            .filter(|p| p.strip == own)
            .chain(laid.plans.iter().filter(|p| p.strip != own))
            .filter(|p| p.takes(piece.item, piece.boxed()))
            .find_map(|plan| {
                if plan.strip == own {
                    on_own_strip(plan, piece, chat)
                } else {
                    packed_on(home, laid, &taken, plan, piece, chat)
                }
            });
        if let Some((anchor, at)) = found {
            let index = home
                .props
                .iter()
                .position(|p| p.item == piece.item)
                .unwrap_or(usize::MAX);
            let (cols, rows) = piece.size();
            taken.push((index, anchor, (cols, rows + at.lift()), at));
        }
        out.push((i, found.and_then(|(_, at)| Some((at.strip?, at.left)))));
    }
    out
}

/// `piece` (in the chat pane `chat`) set down on its own strip, `plan`:
/// at any left along its lane missing the chat and her door's space, as
/// the anchor that keeps it there and where it stands then.
fn on_own_strip(plan: &StripPlan, piece: &Shown, chat: Rect) -> Option<(Anchor, Shown)> {
    let (cols, rows) = piece.size();
    let e = plan.along(piece.lane_on(plan.strip));
    if !e.holds((cols, rows + piece.lift_on(plan.strip))) {
        return None;
    }
    let left = first_clear(plan, piece, chat)?;
    let at = at_left(piece, plan, left);
    Some((e.pin(left, cols).0, at))
}

/// The first left along `piece`'s lane of `plan` (one that holds it)
/// where its cover misses the chat pane `chat` and her door's space, if
/// any. Its cover keeps its rows as it slides along, so only the
/// columns where those two meet its rows matter: the first left is the
/// far wall's, or just past one of them.
fn first_clear(plan: &StripPlan, piece: &Shown, chat: Rect) -> Option<i32> {
    let e = plan.along(piece.lane_on(plan.strip));
    let span = e.to - e.from - i32::from(piece.size().0);
    let base = at_left(piece, plan, e.from).cover();
    if base.is_empty() {
        return (span >= 0).then_some(e.from);
    }
    let rows = |r: Rect| r.y < base.bottom() && base.y < r.bottom();
    let keep: Vec<Rect> = [
        Some(chat).filter(|c| !c.is_empty()),
        plan.space.map(|s| s.rect),
        plan.block,
    ]
    .into_iter()
    .flatten()
    .filter(|&r| !r.is_empty() && rows(r))
    .collect();
    let (x, w) = (i32::from(base.x), i32::from(base.width));
    let meets = |d: i32| {
        keep.iter()
            .any(|r| x + d < i32::from(r.right()) && i32::from(r.x) < x + d + w)
    };
    std::iter::once(0)
        .chain(keep.iter().map(|r| i32::from(r.right()) - x))
        .filter(|&d| (0..=span).contains(&d) && !meets(d))
        .min()
        .map(|d| e.from + d)
}

/// `s` at `left` on `plan`'s strip: its strip, floor and where she'd
/// stand to reach it ([`Shown::stood`]) all from the plan, as [`stand`]
/// sets them for a piece laid out.
fn at_left(s: &Shown, plan: &StripPlan, left: i32) -> Shown {
    debug_assert!(
        !plan.strip.is_floor() || plan.host == plan.raw,
        "a floor is its own host: {plan:?}"
    );
    Shown {
        strip: Some(plan.strip),
        left,
        floor: plan.raw.floor,
        stood: plan.host.floor,
        ..*s
    }
}

/// `piece` (in the chat pane `chat`) set down on another strip, `plan`:
/// anchored anywhere along its lane, packed ([`pack`]) with the pieces
/// standing there in that lane (as `home` anchors them; those
/// [`closetable`] left out, unless `taken` there) and those `taken`
/// there, so that it misses the chat and her door's space and pushes
/// none of the others newly into them. Not where it's wider than her
/// door's kept space leaves room for (see [`space_of`]). As the anchor
/// and where it stands then.
fn packed_on(
    home: &Home,
    laid: &LaidOut,
    taken: &[(usize, Anchor, (u16, u16), Shown)],
    plan: &StripPlan,
    piece: &Shown,
    chat: Rect,
) -> Option<(Anchor, Shown)> {
    let (cols, rows) = piece.size();
    let lane = piece.lane_on(plan.strip);
    let e = plan.along(lane);
    let needs = (cols, rows + piece.lift_on(plan.strip));
    if !e.holds(needs) {
        return None;
    }
    if let Some(space) = plan.space.filter(|s| s.kept)
        && lane == Lane::Floor
        && space_of(plan.raw, space.side, cols).is_none()
    {
        return None;
    }
    let refuses = |r: Rect| {
        in_chat(chat, r)
            || plan.space.is_some_and(|s| s.rect.intersects(r))
            || plan.block.is_some_and(|b| b.intersects(r))
    };
    // No left along the lane misses the chat and the space: no packing
    // will find one (the cheap test first: the closet is weighed for
    // every move a repair search weighs while a piece meets the chat).
    first_clear(plan, piece, chat)?;
    let span = e.to - e.from - i32::from(cols);
    // What stands there in that lane: by index, anchor, needs, and as
    // it shows.
    let mut there: Vec<(usize, Anchor, (u16, u16), Shown)> = laid
        .shown
        .iter()
        .filter(|s| s.strip == Some(plan.strip) && s.lane() == lane && s.scrap.is_none())
        .filter(|s| !closetable(chat, s))
        .filter_map(|s| {
            let index = home.props.iter().position(|p| p.item == s.item)?;
            let (c, r) = s.size();
            let anchor = home
                .props
                .get(index)?
                .anchor
                .unwrap_or_else(|| e.pin(s.left, c).0);
            Some((index, anchor, (c, r + s.lift()), *s))
        })
        .collect();
    there.extend(
        taken
            .iter()
            .filter(|(.., at)| at.strip == Some(plan.strip) && at.lane() == lane)
            .copied(),
    );
    let at_left = |s: &Shown, left: i32| at_left(s, plan, left);
    // Where they stand without it: what "pushed newly in" is judged by.
    let packing: Vec<(usize, Anchor, (u16, u16))> =
        there.iter().map(|&(i, a, n, _)| (i, a, n)).collect();
    let widths: i32 = packing.iter().map(|&(.., (c, _))| i32::from(c)).sum();
    if widths + i32::from(cols) > e.to - e.from {
        return None;
    }
    let alone = pack(&packing, e)?;
    let index = home
        .props
        .iter()
        .position(|p| p.item == piece.item)
        .unwrap_or(usize::MAX);
    let anchors = [Side::Left, Side::Right].into_iter().flat_map(|side| {
        (0..=span).filter_map(move |offset| {
            Some(Anchor {
                side,
                offset: u16::try_from(offset).ok()?,
            })
        })
    });
    let mut with = packing.clone();
    with.push((
        index,
        Anchor {
            side: Side::Left,
            offset: 0,
        },
        needs,
    ));
    for anchor in anchors {
        if let Some(last) = with.last_mut() {
            last.1 = anchor;
        }
        let Some(lefts) = pack(&with, e) else {
            continue;
        };
        let left_of = |i: usize, lefts: &[(usize, i32)]| {
            lefts.iter().find(|&&(j, _)| j == i).map(|&(_, l)| l)
        };
        let Some(left) = left_of(index, &lefts) else {
            continue;
        };
        let at = at_left(piece, left);
        if refuses(at.cover()) {
            continue;
        }
        let pushes_in = there.iter().any(|&(i, _, _, s)| {
            let (Some(was), Some(now)) = (left_of(i, &alone), left_of(i, &lefts)) else {
                return true;
            };
            let (was, now) = (at_left(&s, was).cover(), at_left(&s, now).cover());
            was != now && refuses(now) && !refuses(was)
        });
        if !pushes_in {
            return Some((anchor, at));
        }
    }
    None
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
    let wall = match side {
        Side::Right => raw.to,
        Side::Left => raw.from - 1,
    };
    Some((wall, space_rect(raw, side)?))
}

/// Where her door's space would be against the wall on `side` of `raw`
/// (its [`SPACE`] columns by her height and the floor row), whether or
/// not it can be there (see [`space_of`]): none only off the screen.
pub(super) fn space_rect(raw: Extent, side: Side) -> Option<Rect> {
    use super::sprite::HEIGHT;
    let left = match side {
        Side::Right => raw.to - SPACE,
        Side::Left => raw.from,
    };
    Some(Rect::new(
        u16::try_from(left).ok()?,
        u16::try_from(raw.floor - HEIGHT).ok()?,
        SPACE as u16,
        (HEIGHT + 1) as u16,
    ))
}

/// Which wall her door would be in, chosen from the geometry of `plan`
/// for `home` (see [`Home::wall`]), among `strips` (all, when `None`). A
/// wall qualifies when its strip is here, has room for her door's space
/// (see [`Home::extents`]) and that space misses the chat pane. Of
/// those: at the screen's edge first; then in a room her pieces are in
/// (its floor or its shelf); then where the space would be kept (her floor pieces pack beside
/// it and no hung piece is lost to it, as [`Home::extents`] judges it;
/// only a tie-break: a crowded edge wall of her own strip still wins,
/// and its space yields until she clears it); then in pane order, right
/// before left.
fn choose(home: &Home, plan: Plan, only: Option<Strip>) -> Option<DoorWall> {
    // The rooms her pieces are in (a pane whose pieces all stand on its
    // shelf is hers too: rooms are floors).
    let mine: Vec<Strip> = home.furnished().into_iter().map(Strip::room).collect();
    // Not at the screen's edge, not her pieces' strip, its space not kept,
    // the pane's place, the left side: least first.
    type Key = (bool, bool, bool, usize, bool);
    let mut best: Option<(Key, DoorWall)> = None;
    for (
        index,
        PaneStrip {
            rect, strip, raw, ..
        },
    ) in pane_strips(plan.nooks).into_iter().enumerate()
    {
        if only.is_some_and(|s| s != strip) {
            continue;
        }
        let widest = home.widest_standing(strip);
        for side in [Side::Right, Side::Left] {
            let Some((_, space)) = space_of(raw, side, widest) else {
                continue;
            };
            if in_chat(plan.chat, space) {
                continue;
            }
            // Kept as [`Home::extents`] judges it: her floor pieces pack
            // beside it and no hung piece is lost to it.
            let wall = DoorWall { strip, side };
            let kept = Home {
                door: Some(wall),
                ..home.clone()
            }
            .plan_strip(RawStrip {
                strip,
                raw,
                host: raw,
            })
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
    /// Still in its delivery box (drawn as the box), as set: read it
    /// through [`Shown::boxed`].
    pub boxed: Boxed,
    /// The strip it stands on (none for makeshift pieces, which stand
    /// where she made them).
    pub strip: Option<Strip>,
    /// Its leftmost column.
    pub left: i32,
    /// The floor row it stands on (just below its footprint).
    pub floor: i32,
    /// The floor row she stands on to reach it: its own `floor`, but for
    /// a piece on a shelf (phase 6a D1), its pane's floor below. Derived
    /// from the strip's plan by [`stand`] and [`at_left`], the two ways a
    /// piece is placed on a strip; anything else (a makeshift piece, a
    /// test's) stands where it's reached, `stood == floor`. A struct
    /// update that moves `floor` must set it too.
    pub stood: i32,
    /// Makeshift, crumpled out of torn-off text (never boxed).
    pub scrap: Option<super::scrap::Scrap>,
}

impl Shown {
    /// Whether it's still in its delivery box (drawn as the box): a kit
    /// always is.
    pub fn boxed(&self) -> bool {
        self.boxed.0 || self.item.is_kit()
    }

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
    /// shape, a makeshift piece is for what the real one is. Up on her
    /// shelf, what she gazes up at is for that (phase 6a D8): offered by
    /// where it stands, not by its kind (no [`Spec::uses`] names it).
    pub fn uses(&self) -> &'static [Use] {
        match (self.boxed(), self.scrap) {
            (true, _) => &[Use::Unpack],
            (false, Some(scrap)) if !scrap.done() => &[Use::Crumple],
            (false, None) if self.on_shelf() && self.item.gazed_up_at() => &[Use::GazeUp],
            (false, _) => self.item.spec().uses,
        }
    }

    /// The rows between the floor and its bottom row: its hang, while
    /// it's hung on the wall; 0 standing on the floor (and while it's
    /// boxed, or makeshift, or on a shelf).
    pub fn lift(&self) -> u16 {
        self.lifted(self.strip)
    }

    /// [`Shown::lift`] on `strip` (a place weighed for it there: a
    /// candidate strip's, not its own).
    pub fn lift_on(&self, strip: Strip) -> u16 {
        self.lifted(Some(strip))
    }

    fn lifted(&self, strip: Option<Strip>) -> u16 {
        match self.scrap {
            Some(_) => 0,
            None => lift(self.item, self.boxed(), strip),
        }
    }

    /// Which lane of its strip it's in.
    pub fn lane(&self) -> Lane {
        lane_of(self.lift())
    }

    /// Whether it stands on a shelf (phase 6a): she reaches up to it,
    /// and it stands only where her shelf is written ([`fits`]).
    pub fn on_shelf(&self) -> bool {
        self.strip.is_some_and(|s| !s.is_floor())
    }

    /// Which lane it would be in on `strip` (see [`Shown::lift_on`]).
    pub fn lane_on(&self, strip: Strip) -> Lane {
        lane_of(self.lift_on(strip))
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
            // Beside it, facing it (gazing up at it on her shelf: her
            // gaze is in profile, so beside it is where she looks up at
            // it from, phase 6a D8).
            Use::Watch | Use::Read | Use::Snack | Use::Pet | Use::GazeUp => (
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
            y: self.stood,
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

    /// Where she'd stand to gaze up at it on her shelf (phase 6a D8), in
    /// the order she tries them: just beside it (her box a column past
    /// its columns, as [`Shown::beside`]: left, then right), else a
    /// column further out; only those under its shelf's run, `under`
    /// (its columns: D8's "still under the run"; a piece at a run's end
    /// has none past that end). Beside it rather than under it: her gaze
    /// is in profile, so she looks up at it from there. On its room's
    /// floor ([`Shown::stood`]).
    pub fn gaze_up_spots(&self, under: std::ops::Range<i32>) -> Vec<i32> {
        let [left, right] = self.beside();
        [left, right, left - 1, right + 1]
            .into_iter()
            .filter(|x| under.contains(x))
            .collect()
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
        let real = |s: &Shown| !s.boxed() && s.scrap.is_none();
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
                } else if self.boxed() {
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
        (self.item == Furniture::Tv && !self.boxed()).then(|| {
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
    /// What she has put up (phase 6a: a shelf at most).
    pub structures: Vec<Structure>,
}

impl Home {
    /// Whether she owns `item`: a kit, boxed or put up (phase 6a D2: she
    /// has her shelf kit while it's a parcel and once it's her shelf).
    pub fn owns(&self, item: Furniture) -> bool {
        self.props.iter().any(|p| p.item == item)
            || item
                .spec()
                .kit
                .is_some_and(|kit| self.structures.iter().any(|&s| kit.puts_up(s)))
    }

    /// Whether her kit `item` is hers but not yet up (phase 6a, the
    /// user's seventh round): still in its box, or what it puts up going
    /// up. With her order, [`super::ledger::Ledger::kit_coming`].
    pub fn kit_coming(&self, item: Furniture) -> bool {
        let Some(kit) = item.spec().kit else {
            return false;
        };
        self.props.iter().any(|p| p.item == item)
            || self.structures.iter().any(|&s| {
                kit.puts_up(s)
                    && match s {
                        Structure::Shelf(shelf) => !shelf.finished(),
                    }
            })
    }

    /// Her shelf in `nook`'s pane, if she has put one up there.
    pub fn shelf(&self, nook: Nook) -> Option<Shelf> {
        self.shelves().find(|shelf| shelf.nook == nook)
    }

    /// Her shelves, put up or going up (phase 6a D2).
    pub fn shelves(&self) -> impl Iterator<Item = Shelf> + '_ {
        self.structures.iter().map(|s| match *s {
            Structure::Shelf(shelf) => shelf,
        })
    }

    /// Where a shelf of hers would go up on this frame (phase 6a D3), if
    /// she has none yet (one in all, in 6a): highest that fits (the
    /// user's rule), so every pane is tried for a high shelf before any is
    /// tried for a low one. A **high** shelf hangs from `HIGH_RISE` rows
    /// up to her height under the pane's top, its run and margin
    /// ([`structures::free`]) and the `HEIGHT` rows over its run free; a **low** one
    /// at `LOW_RISE`, its run and margin and the `LOW_ABOVE` rows over its
    /// run free, its block ([`Shelf::block`]) meeting no hung piece of
    /// its pane as laid out (so going up it never pushes one aside).
    /// Every cell is judged as [`super::structures::read_ground`] will
    /// judge it, in the background of its pane's floor under the run's
    /// middle, with `keep_off` and her pieces' covers as laid out.
    ///
    /// Within a height: the panes in order (the pane of a piece that
    /// wants up, her gremlin, standing on its floor out of its box
    /// ([`Furniture::wants_up`], step 7); her living room, her sofa's
    /// pane, the sofa out of its box on its floor (the user's eighth
    /// round: where she spends her time, so she sees what's up there);
    /// `kit_at`, the pane she unpacks in; then `plan`'s; never one
    /// meeting the chat pane), the first
    /// that takes one; in that pane, a row with **no text** anywhere in
    /// the pane's interior before one with text (a shelf among a table's
    /// text reads as the UI's own rule), then (high) a run **touching no
    /// hung piece** (no hung piece's top row right under it, its columns
    /// meeting the run's) before one that does, then the lowest rise,
    /// then the left whose run's middle is nearest the gremlin's middle
    /// in his pane, else the pane's middle (the left first of two as
    /// near). Saved with its anchor from the nearer wall and
    /// `built: 0`. No RNG; each cell of a pane is read once (what's kept
    /// off mapped onto its wall once first, so a visit's layer costs its
    /// own cells, not a scan per cell: `her_shelf_is_quick`), and each
    /// place looks up at most its run's and margin's columns of what was
    /// read ([`Home::shelf_spot_visiting`] counts the frame's cells).
    pub(super) fn shelf_spot(
        &self,
        buf: &Buffer,
        plan: Plan,
        keep_off: &KeepOff,
        kit_at: Option<Nook>,
    ) -> Option<Shelf> {
        self.shelf_spot_visiting(buf, plan, keep_off, kit_at, &mut 0)
    }

    /// [`Home::shelf_spot`], counting into `visited` each cell of the
    /// frame it reads and each entry of what it read that it looks up
    /// (its cost: `shelf_spot_is_cheap`).
    pub(super) fn shelf_spot_visiting(
        &self,
        buf: &Buffer,
        plan: Plan,
        keep_off: &KeepOff,
        kit_at: Option<Nook>,
        visited: &mut usize,
    ) -> Option<Shelf> {
        if !self.structures.is_empty() {
            return None;
        }
        let laid = self.laid_and_shifted(plan.nooks).shown;
        let covers: Vec<Rect> = laid.iter().map(Shown::cover).collect();
        // A piece that wants up, standing on a floor (step 7): its pane,
        // and twice its middle's column.
        let wanting = laid
            .iter()
            .filter(|s| s.item.wants_up() && !s.boxed() && s.scrap.is_none())
            .find_map(|s| match s.strip? {
                Strip::Bottom(nook) => Some((nook, 2 * s.left + i32::from(s.size().0))),
                Strip::Shelf(_) => None,
            });
        // Her living room (the user's eighth round): her sofa's pane,
        // out of its box on its floor.
        let living = laid
            .iter()
            .filter(|s| s.item == Furniture::Sofa && !s.boxed() && s.scrap.is_none())
            .find_map(|s| match s.strip? {
                Strip::Bottom(nook) => Some(nook),
                Strip::Shelf(_) => None,
            });
        let mut panes: Vec<(Nook, Rect)> = plan
            .nooks
            .iter()
            .copied()
            .filter(|&(_, rect)| !rect.intersects(plan.chat))
            .collect();
        // Stable: `plan`'s order within each rank.
        panes.sort_by_key(|&(nook, _)| {
            (
                wanting.is_none_or(|(by, _)| by != nook),
                Some(nook) != living,
                Some(nook) != kit_at,
            )
        });
        // Each pane's wall, read once, when first asked (a high shelf in
        // the first pane reads no other).
        let mut walls: Vec<Option<Option<Wall>>> = panes.iter().map(|_| None).collect();
        for high in [true, false] {
            for (&(nook, rect), wall) in panes.iter().zip(&mut walls) {
                let host = pane_raw(rect);
                let read = || Wall::read(buf, host, plan, keep_off, &covers, visited);
                let Some(wall) = wall.get_or_insert_with(read) else {
                    continue;
                };
                let hung: Vec<Rect> = laid
                    .iter()
                    .filter(|s| s.strip == Some(Strip::Bottom(nook)) && s.lane() == Lane::Wall)
                    .map(Shown::rect)
                    .collect();
                let middle = match wanting {
                    Some((by, twice)) if by == nook => twice,
                    _ => host.from + host.to,
                };
                if let Some(shelf) = wall.best(buf, nook, host, high, &hung, middle, visited) {
                    return Some(shelf);
                }
            }
        }
        None
    }

    /// Unpack `item`: it's out of its box. False if it wasn't boxed, or
    /// it's a kit (never out of its box: it goes up as a structure,
    /// [`Home::raise`]).
    pub fn unbox(&mut self, item: Furniture) -> bool {
        if item.is_kit() {
            return false;
        }
        match self.props.iter_mut().find(|p| p.item == item && p.boxed()) {
            Some(prop) => {
                prop.boxed = false.into();
                true
            }
            None => false,
        }
    }

    /// Her kit `item` opened, and up as `structure` (phase 6a D7b), not
    /// yet built (`built: 0`; she puts it up in stages): the kit goes,
    /// the structure comes. False, and nothing changes, unless `item` is
    /// a kit that puts `structure` up, it's here in its box, and nothing
    /// is up yet (one structure in all, in 6a).
    pub fn raise(&mut self, item: Furniture, structure: Structure) -> bool {
        let puts_up = item.spec().kit.is_some_and(|kit| kit.puts_up(structure));
        let kit = self.props.iter().position(|p| p.item == item);
        let Some(i) = kit.filter(|_| puts_up && self.structures.is_empty()) else {
            return false;
        };
        self.props.remove(i);
        self.structures.push(match structure {
            Structure::Shelf(shelf) => Structure::Shelf(Shelf { built: 0, ..shelf }),
        });
        true
    }

    /// How high what's set down on `strip` is: up on a high shelf of
    /// hers ([`Shelf::high`]), which she hops up to (phase 6a D7c); on a
    /// low one, in her reach; else down on a floor (a shelf strip with
    /// no shelf of hers on it has nothing up there).
    pub fn height(&self, strip: Strip) -> Height {
        match strip {
            Strip::Shelf(nook) => match self.shelf(nook) {
                Some(shelf) if shelf.high() => Height::High,
                Some(_) => Height::Low,
                None => Height::Floor,
            },
            Strip::Bottom(_) => Height::Floor,
        }
    }

    /// Something new up on the shelf `strip` is, `today` (phase 6a step
    /// 8: a piece she set on it; a stage up is [`Home::put_up`]'s): her
    /// gaze up there likelier that day and the next. Nothing on a floor.
    pub fn freshen(&mut self, strip: Strip, today: Dated) {
        let Strip::Shelf(nook) = strip else {
            return;
        };
        for structure in &mut self.structures {
            let Structure::Shelf(shelf) = structure;
            if shelf.nook == nook {
                shelf.fresh = Some(today);
            }
        }
    }

    /// Stage `to` of her shelf on `nook`'s wall up (phase 6a D7c): its
    /// next, and no further than finished. False, and nothing changes,
    /// unless she has a shelf there with just the stage before it up.
    pub fn put_up(&mut self, nook: Nook, to: u8, today: Dated) -> bool {
        for structure in &mut self.structures {
            let Structure::Shelf(shelf) = structure;
            if shelf.nook == nook && shelf.built.checked_add(1) == Some(to) && to <= SHELF_STAGES {
                shelf.built = to;
                // Something new up there (phase 6a step 8).
                shelf.fresh = Some(today);
                return true;
            }
        }
        false
    }

    /// Whether anything is still in its box.
    pub fn boxed(&self) -> bool {
        self.props.iter().any(|p| p.boxed())
    }

    /// Her rooms: each pane's floor her pieces stand in (on it, or on a
    /// shelf above it: a shelf is part of the room below, never a room of
    /// its own), in the order she furnished them, and what it is from
    /// what's out of its box there.
    pub fn rooms(&self) -> Vec<(Strip, Role)> {
        let mut rooms: Vec<Strip> = Vec::new();
        for strip in self.furnished() {
            if !rooms.contains(&strip.room()) {
                rooms.push(strip.room());
            }
        }
        rooms
            .into_iter()
            .map(|room| {
                let offers: Vec<Offer> = self
                    .props
                    .iter()
                    .filter(|p| p.strip.room() == room && !p.boxed())
                    .flat_map(|p| p.item.spec().offers.iter().copied())
                    .collect();
                (room, role(&offers))
            })
            .collect()
    }

    /// The strips her pieces stand on, in the order she furnished them.
    fn furnished(&self) -> Vec<Strip> {
        self.each_furnished().collect()
    }

    /// [`Home::furnished`], one by one (nothing gathered: a move's
    /// layout reads it once per move a search weighs).
    fn each_furnished(&self) -> impl Iterator<Item = Strip> + '_ {
        self.props
            .iter()
            .enumerate()
            .filter(|&(k, prop)| !self.props.iter().take(k).any(|q| q.strip == prop.strip))
            .map(|(_, prop)| prop.strip)
    }

    /// The widest of her pieces that stand on `strip`'s floor, anchored
    /// or not (0 with none).
    fn widest_standing(&self, strip: Strip) -> u16 {
        self.lays_from(strip)
            .filter(|(_, p)| p.lane() == Lane::Floor)
            .map(|(_, p)| p.item.spec().footprint.0)
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
        self.raw_extents(nooks)
            .into_iter()
            .map(|raw| self.plan_strip(raw).0)
            .collect()
    }

    /// Her strips' geometry this frame, before her door's space: every
    /// layout path's one source of strips (pinning, laying out, the
    /// plans). The pane strips in `nooks`' order ([`pane_strips`]), then
    /// each shelf she has finished putting up ([`Shelf::finished`], the
    /// one test her record reads too) whose geometry holds this frame
    /// ([`Shelf::run`]: pure geometry, no text): its run, the rows over
    /// it inside its pane, reached from its pane's floor (phase 6a D6).
    /// Panes first, so a pane's plan is where it always was.
    fn raw_extents(&self, nooks: &[(Nook, Rect)]) -> Vec<RawStrip> {
        self.with_shelves(
            pane_strips(nooks)
                .into_iter()
                .map(|p| RawStrip {
                    strip: p.strip,
                    raw: p.raw,
                    host: p.raw,
                })
                .collect(),
        )
    }

    /// `panes` (her panes' floors), then each shelf she has finished
    /// putting up whose geometry holds on one of them this frame, as a
    /// strip (see [`Home::raw_extents`]): from her panes' floors alone,
    /// so [`Home::relaid`] has her shelves just as laying her home out
    /// does (the one way both work them out).
    fn with_shelves(&self, panes: Vec<RawStrip>) -> Vec<RawStrip> {
        let shelves: Vec<RawStrip> = self
            .structures
            .iter()
            .filter_map(|&structure| {
                let Structure::Shelf(shelf) = structure;
                if !shelf.finished() {
                    return None;
                }
                let host = panes
                    .iter()
                    .find(|p| p.strip == Strip::Bottom(shelf.nook))?
                    .raw;
                let run = shelf.run(host)?;
                Some(RawStrip {
                    strip: Strip::Shelf(shelf.nook),
                    raw: Extent {
                        from: run.from,
                        to: run.to,
                        floor: run.y,
                        rows: host.rows.saturating_sub(shelf.rise),
                    },
                    host,
                })
            })
            .collect();
        let mut strips = panes;
        strips.extend(shelves);
        strips
    }

    /// On a pane's floor `at`, her shelf's block there, if it hangs low
    /// (see [`Shelf::block`]): no piece she hangs there meets it. Read by
    /// the strip's layout ([`Home::hung_clear`]) and its plan. A shelf's
    /// own strip has none (nothing hangs on a shelf).
    fn block_on(&self, at: RawStrip) -> Option<Rect> {
        if !at.strip.is_floor() {
            return None;
        }
        self.shelf(at.strip.nook())?.block(at.host)
    }

    /// `strip`'s plan (see [`Home::extents`]), and on her door's strip,
    /// where it has a space, the strip laid out as the plan has it (its
    /// space kept or yielding): judging the one lays out the other.
    fn plan_strip(&self, at: RawStrip) -> (StripPlan, Option<Option<LaidOut>>) {
        let RawStrip { strip, raw, host } = at;
        let mut laid = None;
        let space = self.strip_door(strip).and_then(|d| {
            let (wall, rect) = space_of(raw, d.side, self.widest_standing(strip))?;
            let doored = self.lay_strip(at, narrowed(raw, d.side), Some(rect));
            // Laid out with the space, every piece on the strip that
            // could be laid out is (none left out, as `lay_strip` counts
            // them, whatever left it out): laid out without it, no more
            // could be, and it's kept. Only one left out needs the strip
            // laid out without the space to judge it.
            let doored = match doored {
                Some((doored, 0)) => {
                    laid = Some(Some(doored));
                    return Some(Space {
                        side: d.side,
                        wall,
                        rect,
                        kept: true,
                    });
                }
                doored => doored.map(|(l, _)| l),
            };
            let doorless = self.lay_strip(at, raw, None).map(|(l, _)| l);
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
            host,
            block: self.block_on(at),
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

    /// Her pieces projected ([`Home::project_with`]: kept off the chat
    /// pane where they move, her door's wall going with them if they
    /// move off its strip) and her door's wall settled
    /// ([`Home::settle_door`]) for her home as projected: with none saved
    /// yet, her pieces are projected first (anchored, and moved off a
    /// strip that's gone, each target judged with the wall the home
    /// moved there would choose: [`Home::wall`]), the wall then chosen
    /// for the home they make, and the pieces projected again with it,
    /// so the frame lays them out as they'll stay. A wall chosen before
    /// they moved would be chosen for a home that's never laid out, and
    /// could send them on, or lose a hung piece, where with no door they
    /// all stay. With each move made, from and to. Logs nothing:
    /// [`Home::frame`] is this for her own home, and a scratch home a
    /// placement tries is projected as a frame would have it by this too.
    pub(super) fn settled(
        &mut self,
        drawn: Drawn,
        plan: Plan,
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> (Vec<Shown>, Vec<(Strip, Strip)>) {
        let (shown, mut moves) = self.project_with(drawn, plan, blocked);
        if !self.settle_door(plan) {
            return (shown, moves);
        }
        let (shown, more) = self.project_with(drawn, plan, blocked);
        moves.extend(more);
        (shown, moves)
    }

    /// Where her pieces show this frame, as production asks it
    /// ([`Home::settled`]: her pieces projected, her door's wall settled
    /// for them), the closet taking what no room would. Also whether her
    /// home changed (anchors pinned, pieces moved, her door's wall
    /// saved). The only call that logs a change of her door's wall (a
    /// scratch home's is no change of hers).
    pub(super) fn frame(
        &mut self,
        drawn: Drawn,
        plan: Plan,
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> (Vec<Shown>, bool) {
        let before = self.clone();
        let (mut shown, moves) = self.settled(drawn, plan, blocked);
        // The closet (door batch, D7): a piece in the chat pane that no
        // room would take is hidden this frame (kept where it's laid, in
        // her record), back as soon as one would.
        if shown.iter().any(|s| in_chat(plan.chat, s.cover())) {
            let laid = self.laid_and_shifted(plan.nooks);
            let gone: Vec<Furniture> = stranded(self, &laid, plan.chat)
                .into_iter()
                .filter_map(|i| laid.shown.get(i).map(|s| s.item))
                .collect();
            if !gone.is_empty() {
                tracing::trace!(?gone, "houseguest: in the chat pane with no room for it");
                shown.retain(|s| s.scrap.is_some() || !gone.contains(&s.item));
            }
        }
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
    /// rows each needs clear above the floor), those that are anchored
    /// and that it takes ([`Strip::takes`], a parcel on a shelf
    /// included: every layout reads this, so a piece is never laid out on
    /// a strip that doesn't take it, and every search that judges a place
    /// by its layout refuses it there).
    fn on(&self, strip: Strip, lane: Lane) -> Vec<Laned> {
        let (floor, wall) = self.on_lanes(strip);
        match lane {
            Lane::Floor => floor,
            Lane::Wall => wall,
        }
    }

    /// [`Home::on`] for both lanes of `strip` at once (standing, hung),
    /// in one pass over her pieces: a strip's layout reads both, once
    /// per move a search weighs.
    fn on_lanes(&self, strip: Strip) -> (Vec<Laned>, Vec<Laned>) {
        let (mut floor, mut wall) = (Vec::new(), Vec::new());
        for (i, p) in self.lays_from(strip) {
            let Some(anchor) = p.anchor.filter(|_| strip.takes(p.item, p.boxed())) else {
                continue;
            };
            match p.lane() {
                Lane::Floor => floor.push((i, anchor, p.needs())),
                Lane::Wall => wall.push((i, anchor, p.needs())),
            }
        }
        (floor, wall)
    }

    /// Her pieces on `strip`, by index: with [`Home::strip_door`] and
    /// her shelf in its pane ([`Home::shelf`]: a low one's block on a
    /// pane's floor, the shelf's own geometry on a shelf), all that laying
    /// the strip out reads (its plan, what stands, what hangs).
    /// [`StripLaid`] records them, and [`Home::relaid`] lays a strip out
    /// again whenever they differ, so a new input to a strip's layout
    /// belongs here (or beside the door and the shelf in [`StripLaid`]),
    /// or a strip laid out before it changed would be taken for one
    /// after.
    fn lays_from(&self, strip: Strip) -> impl Iterator<Item = (usize, &Prop)> + '_ {
        self.props
            .iter()
            .enumerate()
            .filter(move |(_, p)| p.strip == strip)
    }

    /// Her door's wall, if it's on `strip` (see [`Home::lays_from`]).
    fn strip_door(&self, strip: Strip) -> Option<DoorWall> {
        self.door.filter(|d| d.strip == strip)
    }

    /// Anchor every piece that has only a share of the way along (an
    /// older record's) where that share puts it, if its strip is here:
    /// the share is of its `host` (its pane's floor, as
    /// [`StripPlan::pin`] takes it), the anchor on its own raw strip
    /// (see [`RawStrip::anchor_at`]).
    fn pin_anchors(&mut self, nooks: &[(Nook, Rect)]) {
        let strips = self.raw_extents(nooks);
        for prop in &mut self.props {
            if prop.anchor.is_none()
                && let Some(at) = strips.iter().find(|s| s.strip == prop.strip)
            {
                let cols = prop.item.spec().footprint.0;
                prop.anchor = Some(at.anchor_at(prop.at, cols));
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
        self.pin_anchors(nooks);
        self.laid_and_shifted(nooks)
    }

    /// [`Home::laid_out`], strip by strip ([`Home::strips_laid`]): what
    /// [`Home::relaid`] lays a move out from.
    pub(super) fn strips_laid_out(&mut self, nooks: &[(Nook, Rect)]) -> Vec<StripLaid> {
        self.pin_anchors(nooks);
        self.strips_laid(nooks)
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
        let strips: Vec<(StripPlan, Option<LaidOut>)> = self
            .raw_extents(nooks)
            .into_iter()
            .map(|raw| self.strip_layout(raw))
            .collect();
        let strips: Vec<_> = strips.iter().map(|(p, l)| (p, l.as_ref())).collect();
        self.gathered(&strips)
    }

    /// Each of `nooks`' strips laid out alone, in their order: its plan
    /// and, where she has a piece on it, what it lays out (see
    /// [`Home::laid_and_shifted`], which gathers them). A strip's layout
    /// reads only her pieces on it, her door's wall there and her shelf
    /// in its pane (a low one's block; [`Home::lays_from`] is the one
    /// list), so a move lays out again only the strips it touches
    /// ([`Home::relaid`]).
    pub(super) fn strips_laid(&self, nooks: &[(Nook, Rect)]) -> Vec<StripLaid> {
        self.raw_extents(nooks)
            .into_iter()
            .map(|raw| self.strip_laid(raw))
            .collect()
    }

    /// `strip` laid out alone (see [`Home::strips_laid`]), with what
    /// it was laid out from.
    fn strip_laid(&self, raw: RawStrip) -> StripLaid {
        self.strip_laid_into(raw, Vec::new())
    }

    /// [`Home::strip_laid`], what it was laid out from written into
    /// `from` (cleared first: a list kept from a strip laid out before,
    /// so laying one out again allocates none).
    fn strip_laid_into(&self, raw: RawStrip, mut from: Vec<(usize, Prop)>) -> StripLaid {
        let strip = raw.strip;
        let (plan, laid) = self.strip_layout(raw);
        from.clear();
        from.extend(self.lays_from(strip).map(|(i, p)| (i, *p)));
        StripLaid {
            plan,
            laid,
            from,
            door: self.strip_door(strip),
            shelf: self.shelf(strip.nook()),
        }
    }

    /// Whether this home lays `s`'s strip out from just what `s` was
    /// laid out from ([`Home::lays_from`]: her pieces on it, by index,
    /// her door's wall there, her shelf in its pane), so it lays it out
    /// as `s` has it.
    fn lays_as(&self, s: &StripLaid) -> bool {
        let strip = s.plan.strip;
        s.door == self.strip_door(strip)
            && s.shelf == self.shelf(strip.nook())
            && self
                .lays_from(strip)
                .eq(s.from.iter().map(|(i, p)| (*i, p)))
    }

    /// `strip`'s plan and, if she has a piece on it, what it lays out
    /// (see [`StripLaid`]).
    fn strip_layout(&self, raw: RawStrip) -> (StripPlan, Option<LaidOut>) {
        let (plan, laid) = self.plan_strip(raw);
        if self.lays_from(raw.strip).next().is_none() {
            return (plan, None);
        }
        // Her door's strip, as its plan was judged; a space that yields
        // refuses nothing: the strip lays out as with no door.
        let laid = laid.unwrap_or_else(|| self.lay_strip(raw, plan.floor, None).map(|(l, _)| l));
        (plan, laid)
    }

    /// [`Home::laid_and_shifted`] of this home, `was` the strips of a
    /// home on the same panes laid out alone ([`Home::strips_laid`]): a
    /// strip laid out from just what this home's lays out from
    /// ([`Home::lays_as`]: its pieces, her door, her shelf in its pane)
    /// is taken as `was` has it, every other is laid out again. A move
    /// lays out only the strips it touches. Where her shelves aren't as
    /// `was` had them (never, within a search: nothing puts one up or
    /// takes one down there), her strips themselves may differ (a shelf
    /// finished or gone is a strip more or less), so they're worked out
    /// again from `was`'s panes' floors ([`Home::with_shelves`]) and all
    /// laid out afresh.
    pub(super) fn relaid(&self, was: &[StripLaid]) -> LaidOut {
        self.relaid_keeping(was, &mut LaidAgain::default())
    }

    /// [`Home::relaid`], keeping in `again` each strip it lays out again
    /// (by its place in `was`), and taking the one kept there instead of
    /// laying it out again where this home lays it out from just what
    /// that one was laid out from ([`Home::lays_as`]): a search weighs
    /// many moves from one `was`, and every move of a piece off its strip
    /// leaves that strip as the others do. Laid out from the same, a
    /// strip lays out the same (the rule `was` itself is taken by), so
    /// this is [`Home::relaid`] exactly, only cheaper.
    pub(super) fn relaid_keeping(&self, was: &[StripLaid], again: &mut LaidAgain) -> LaidOut {
        if !was
            .iter()
            .all(|s| s.shelf == self.shelf(s.plan.strip.nook()))
        {
            return self.laid_on_panes_of(was);
        }
        let LaidAgain { kept, again } = again;
        kept.resize_with(was.len(), || None);
        again.clear();
        for (s, kept) in was.iter().zip(kept.iter_mut()) {
            let as_was = self.lays_as(s);
            again.push(!as_was);
            let on = |k: &StripLaid| {
                (k.plan.strip, k.plan.raw, k.plan.host) == (s.plan.strip, s.plan.raw, s.plan.host)
            };
            if as_was || kept.as_ref().is_some_and(|k| on(k) && self.lays_as(k)) {
                continue;
            }
            let raw = RawStrip {
                strip: s.plan.strip,
                raw: s.plan.raw,
                host: s.plan.host,
            };
            let from = kept.take().map(|k| k.from).unwrap_or_default();
            *kept = Some(self.strip_laid_into(raw, from));
        }
        self.gathered_from(was.iter().zip(kept.iter()).zip(again.iter()).map(
            |((s, kept), &again)| match kept {
                Some(k) if again => (&k.plan, k.laid.as_ref()),
                _ => (&s.plan, s.laid.as_ref()),
            },
        ))
    }

    /// [`Home::laid_and_shifted`] of this home on `was`'s panes: its
    /// strips worked out again from their floors (her shelves on them
    /// where they hang: [`Home::with_shelves`]), every one laid out.
    fn laid_on_panes_of(&self, was: &[StripLaid]) -> LaidOut {
        let panes: Vec<RawStrip> = was
            .iter()
            .filter(|s| s.plan.strip.is_floor())
            .map(|s| RawStrip {
                strip: s.plan.strip,
                raw: s.plan.raw,
                host: s.plan.host,
            })
            .collect();
        let laid: Vec<(StripPlan, Option<LaidOut>)> = self
            .with_shelves(panes)
            .into_iter()
            .map(|at| self.strip_layout(at))
            .collect();
        let strips: Vec<(&StripPlan, Option<&LaidOut>)> = laid
            .iter()
            .map(|(plan, laid)| (plan, laid.as_ref()))
            .collect();
        self.gathered(&strips)
    }

    /// Her strips laid out alone, in their order (each its plan and
    /// what it lays out), gathered as [`Home::laid_and_shifted`] has
    /// them: what they lay out strip by strip in the order she
    /// furnished them, every one's plan.
    fn gathered(&self, strips: &[(&StripPlan, Option<&LaidOut>)]) -> LaidOut {
        self.gathered_from(strips.iter().copied())
    }

    /// [`Home::gathered`], her strips handed in one by one (a move's
    /// layout gathers them without first listing them).
    fn gathered_from<'a>(
        &self,
        strips: impl Iterator<Item = (&'a StripPlan, Option<&'a LaidOut>)> + Clone,
    ) -> LaidOut {
        let laid = || strips.clone().filter_map(|(_, l)| l);
        let mut shown: Vec<Shown> = Vec::with_capacity(laid().map(|l| l.shown.len()).sum());
        let mut shifts = Vec::with_capacity(laid().map(|l| l.shifts.len()).sum());
        for strip in self.each_furnished() {
            if let Some((_, Some(laid))) = strips.clone().find(|(p, _)| p.strip == strip) {
                shown.extend_from_slice(&laid.shown);
                shifts.extend_from_slice(&laid.shifts);
            }
        }
        LaidOut {
            shown,
            shifts,
            plans: strips.map(|(p, _)| *p).collect(),
        }
    }

    /// `strip` laid out alone (see [`Home::laid_and_shifted`]): what
    /// stands packed on `floor`, what hangs on the `raw` wall, clear of
    /// what stands and of `space`; none of it if what stands doesn't
    /// pack. Its plans are left empty. With how many of her anchored
    /// pieces on the strip it leaves out (whatever leaves them out: a
    /// wall too narrow, no clear place to hang).
    fn lay_strip(
        &self,
        at: RawStrip,
        floor: Extent,
        space: Option<Rect>,
    ) -> Option<(LaidOut, usize)> {
        let RawStrip { strip, raw, host } = at;
        let (standing_on, hung_on) = self.on_lanes(strip);
        let mut packed = pack(&standing_on, floor)?;
        let mut shifts = Vec::new();
        // With nothing hung there, there's nothing to hang clear (the
        // usual strip, laid out many times a search).
        if !hung_on.is_empty() {
            let standing: Vec<Shown> = packed
                .iter()
                .filter_map(|&(i, left)| Some(stand(self.props.get(i)?, strip, raw, host, left)))
                .collect();
            let hung = pack_each(&hung_on, raw);
            let clear = self.hung_clear(hung.clone(), at, space, &standing);
            for &(index, left) in &hung {
                let Some(prop) = self.props.get(index) else {
                    continue;
                };
                match clear.iter().find(|&&(i, _)| i == index) {
                    Some(&(_, now)) if now == left => {}
                    Some(&(_, now)) => {
                        shifts.push((prop.item, Some((now - left).unsigned_abs())));
                    }
                    None => shifts.push((prop.item, None)),
                }
            }
            packed.extend(clear);
        }
        packed.sort_unstable();
        let shown: Vec<Shown> = packed
            .into_iter()
            .filter_map(|(index, left)| Some(stand(self.props.get(index)?, strip, raw, host, left)))
            .collect();
        let left_out = (standing_on.len() + hung_on.len()).saturating_sub(shown.len());
        let laid = LaidOut {
            shown,
            shifts,
            plans: Vec::new(),
        };
        Some((laid, left_out))
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
    /// low enough to. Nor does any meet a low shelf's block in its pane
    /// (phase 6a D3a, [`Home::block_on`]): a poster or her clock whose
    /// anchor puts it there hangs at the nearest place beside it the same
    /// way (only her window looks for a place to lean on its sill), so it
    /// never meets the shelf or hides it; with none, it's left out alone.
    /// Whether there's such a place is judged between its neighbours'
    /// places on the wall alone (where [`pack_each`] has them), even a
    /// neighbour's that is about to be moved past the block itself. Left out is logged at trace only: this is
    /// pure layout, weighed many times a frame by the searches, and no
    /// change of hers reaches it (a delivery, a repair or a move judges a
    /// piece that shows left out as no place for it: [`Home::admits`],
    /// `rules::fits_now`, [`Home::move_off`]); only a resize or an older
    /// record crowds a wall so, and a piece hidden by a resize or by
    /// text isn't logged either.
    fn hung_clear(
        &self,
        mut hung: Vec<(usize, i32)>,
        at: RawStrip,
        space: Option<Rect>,
        standing: &[Shown],
    ) -> Vec<(usize, i32)> {
        hung.sort_by_key(|&(index, left)| (left, index));
        let block = self.block_on(at);
        let RawStrip {
            strip,
            raw: e,
            host,
        } = at;
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
                let at = stand(prop, strip, e, host, left);
                space.is_some_and(|r| at.rect().intersects(r))
                    || block.is_some_and(|r| at.rect().intersects(r))
                    || standing
                        .iter()
                        .any(|s| at.rect().intersects(s.cover()) && !at.may_overlap(s))
            };
            if !meets(left) {
                out.push((index, left));
                continue;
            }
            // Where she could lean on its sill (her window) from an end
            // clear of every piece that stands.
            let looks = prop.item.spec().uses.contains(&Use::LookOut);
            let leans = |left: i32| {
                let at = stand(prop, strip, e, host, left);
                looks
                    && at.look_out_spots().iter().any(|&x| {
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
                    let shelf = block
                        .is_some_and(|r| stand(prop, strip, e, host, left).rect().intersects(r));
                    tracing::trace!(
                        item = ?prop.item,
                        shelf,
                        "houseguest: no room on the wall clear of what stands, her door's space and her shelf"
                    );
                }
            }
        }
        out
    }

    /// Set `piece` down where a repair takes it (`to`): on its strip,
    /// anchored there, turned as it says and settled, with its share of
    /// the way along (of the strip's host, where its anchor stands it)
    /// for a frame that lays the strip out no more. All her record keeps
    /// of a piece she set down, whatever strip it's on.
    pub(super) fn set_down(
        &mut self,
        piece: Furniture,
        to: super::rules::Placement,
        nooks: &[(Nook, Rect)],
    ) {
        let Some(prop) = self.props.iter_mut().find(|p| p.item == piece) else {
            return;
        };
        prop.strip = to.strip;
        prop.anchor = Some(to.anchor);
        prop.facing = to.facing;
        prop.settled = true;
        let cols = piece.spec().footprint.0;
        let plans = self.extents(nooks);
        if let Some(plan) = plans.iter().find(|p| p.strip == to.strip)
            && let Some(prop) = self.props.iter_mut().find(|p| p.item == piece)
        {
            let lane = prop.lane();
            prop.at = plan
                .pin(lane, plan.along(lane).left(to.anchor, cols), cols)
                .1;
        }
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
        self.project_with(Drawn::bare(buf), Plan::bare(nooks), blocked)
            .0
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
        drawn: Drawn,
        plan: Plan,
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> (Vec<Shown>, Vec<(Strip, Strip)>) {
        let mut laid = self.laid_out(plan.nooks);
        let mut moves = Vec::new();
        for strip in self.furnished() {
            // Only a pane's floor moves off: a shelf whose plan is gone
            // hides what's on it, never moves it (phase 6a D1).
            let Strip::Bottom(nook) = strip else {
                continue;
            };
            let packs = laid
                .plan(strip)
                .is_some_and(|p| pack(&self.on(strip, Lane::Floor), p.floor).is_some());
            if !packs && let Some(to) = self.move_off(nook, drawn, plan, blocked) {
                moves.push((strip, to));
                laid = self.laid_and_shifted(plan.nooks);
            }
        }
        let mut shown: Vec<Shown> = Vec::new();
        for at in laid.shown {
            if fits(drawn, &at, &|x, y| free(&shown, blocked, Some(&at), x, y)) {
                shown.push(at);
            }
        }
        (shown, moves)
    }

    /// Move the pieces on `from`'s floor, together, to the first other
    /// pane's floor ([`pane_strips`]: never a shelf) that holds those
    /// that stand with its own, each on free cells;
    /// those hung go along, where their share of the way puts them. With
    /// only hung pieces, to the first whose wall holds them all that way.
    /// None of them may meet `plan`'s chat pane, nor the target's door
    /// space (kept or not), where they stand nor where they're laid
    /// out, nor push a piece already there newly into either. Her door
    /// goes with them if it was on `strip`: to the target's wall
    /// [`choose`] picks, else the first that qualifies; each target
    /// judged with her door already there, so the pieces are laid out the
    /// frame they move as they'll stay. Where they went, if they moved.
    fn move_off(
        &mut self,
        from: Nook,
        drawn: Drawn,
        plan: Plan,
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> Option<Strip> {
        let strip = Strip::Bottom(from);
        let leaving: Vec<usize> = (0..self.props.len())
            .filter(|&i| self.props.get(i).is_some_and(|p| p.strip == strip))
            .collect();
        let follows = self.door.is_some_and(|d| d.strip == strip);
        // Where the pieces staying are laid out before any move.
        let was = self.laid_and_shifted(plan.nooks).shown;
        let target = pane_strips(plan.nooks)
            .into_iter()
            .filter(|p| p.strip != strip)
            .find_map(|PaneStrip { strip: to, raw, .. }| {
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
                // Her home there, her door where the frame will have it,
                // and how it's laid out: worked out once, if asked.
                let judged = OnceCell::new();
                let judged = || {
                    judged.get_or_init(|| Home {
                        door: moved.wall(plan),
                        ..moved.clone()
                    })
                };
                let now = OnceCell::new();
                let now = || now.get_or_init(|| judged().laid_and_shifted(plan.nooks).shown);
                // Each judged where packing puts it; but a piece that
                // hangs packed onto a low shelf's block where the layout
                // hangs it beside the block, and none if it's left out
                // there (as [`Home::admits`] judges a delivery: only a
                // block moves a poster or her clock; her window over a
                // piece that stands is judged where packing puts it, as
                // before phase 6a). With pieces that stand moving, the
                // hung ones aren't judged at all: they go along where
                // their share puts them, the closet taking any their wall
                // doesn't hold (her wall never moves a room).
                let all_free =
                    packed
                        .iter()
                        .filter(|(i, _)| leaving.contains(i))
                        .all(|&(i, left)| {
                            moved.props.get(i).is_some_and(|p| {
                                let at = stand(p, to, e, strip_plan.host, left);
                                let at =
                                    if strip_plan.block.is_some_and(|b| at.rect().intersects(b)) {
                                        match now()
                                            .iter()
                                            .find(|s| s.strip == Some(to) && s.item == p.item)
                                        {
                                            Some(&laid) => laid,
                                            None => return false,
                                        }
                                    } else {
                                        at
                                    };
                                let cover = at.cover();
                                fits(drawn, &at, &|x, y| !blocked(x, y))
                                    && !in_chat(plan.chat, cover)
                                    && space.is_none_or(|r| !cover.intersects(r))
                            })
                        });
                // Nor, as they're laid out there (a window hung by its
                // share, where a space that yields doesn't move it), in
                // what the frame keeps new pieces out of (D3), her door
                // where the frame will have it: neither those that move,
                // nor any they push along newly into it (judged against
                // where it's laid out now).
                let kept_out = || {
                    let keep = Keep::of(judged(), plan);
                    let now = now();
                    keep.takes_in(&was, now)
                        || now.iter().any(|s| {
                            keep.refuses(s.cover())
                                && leaving
                                    .iter()
                                    .any(|&i| moved.props.get(i).is_some_and(|p| p.item == s.item))
                        })
                };
                (all_free && !kept_out()).then_some((to, strip_plan, moved, packed))
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
    /// clear of what's `shown`, of her door's space and of the chat pane,
    /// chosen at random (a stage gift: it stands where it's settled).
    pub fn spot(
        &self,
        drawn: Drawn,
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
        let spots: Vec<(Prop, bool)> = pane_strips(plan.nooks)
            .into_iter()
            .filter_map(|PaneStrip { nook, strip, .. }| {
                let mut probe = self.clone();
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
                        boxed: false.into(),
                        settled: true,
                    };
                    if !e.holds(prop.needs()) {
                        return None;
                    }
                    // Where it's laid out once it's there, if at all
                    // (packed with what's on its lane, a window kept
                    // clear of what stands and of her door's space: else
                    // it would only go to the closet); and never where
                    // the frame keeps new pieces out (`Keep`: the chat
                    // pane, and her door's space as the home is with it
                    // there, kept or not: a window hung there would
                    // stand aside, or not at all).
                    // Nor pushing a piece that shows (or hanging one clear
                    // of it) newly into what's kept out.
                    let mut with = home.clone();
                    with.props.push(prop);
                    let laid = with.laid_and_shifted(plan.nooks);
                    let at = *laid.shown.iter().find(|s| s.item == item)?;
                    let keep = Keep::of(&with, plan);
                    (!keep.refuses(at.cover()) && !keep.takes_in(shown, &laid.shown))
                        .then_some((prop, at))
                })
            })
            .filter_map(|(prop, at)| {
                let clear = |x: i32, y: i32| free(shown, blocked, Some(&at), x, y);
                let her = |x: i32, y: i32| free(shown, blocked, None, x, y);
                (fits(drawn, &at, &clear) && roomy(drawn.buf(), &at, &her))
                    .then(|| (prop, room_to_look(drawn.buf(), &at, &her)))
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

    /// Where a delivery of `item` comes in this frame: through the flap
    /// in her own door, by her door's wall, if its space is kept there
    /// ([`Home::through_her_door`]: it stands just past the space; door
    /// batch D9; the flap's `door`); else through a flap in one of her
    /// strips' walls, preferring a wall at the screen's edge; to stand
    /// against it facing into the room, unsettled (she never
    /// chose where it stands). The pieces already on
    /// that strip make way, packed in order, but only where every one of
    /// them that shows still fits, its box fits on blank, free cells, she
    /// can unpack it, and she'd fit to use the piece. Both are asked of
    /// `seats` (her seats at a piece, among the pieces shown: see
    /// `seats_of`), each on the whole room as it would show (the pieces
    /// that made way where they'd be), and her home as it will be then
    /// (what keeps out her makeshift pieces: see `Keep`): with the box
    /// standing there, an `Unpack` seat; with the piece out of its box, a seat for each way
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
        drawn: Drawn,
        plan: Plan,
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        seats: &Seats<'_>,
        item: Furniture,
    ) -> Option<(Prop, Flap)> {
        let mut walls: Vec<(bool, bool, Strip, Extent, Side)> = Vec::new();
        for PaneStrip {
            rect,
            strip,
            raw: e,
            ..
        } in pane_strips(plan.nooks)
        {
            for side in [Side::Right, Side::Left] {
                let wall = DoorWall { strip, side };
                let door = self.through_her_door(plan, wall, item);
                walls.push((door, at_edge(rect, side, plan.screen), strip, e, side));
            }
        }
        // Her door first; then the screen's edge; otherwise in pane order.
        walls.sort_by_key(|&(door, edge, ..)| (!door, !edge));
        let looks: &[bool] = if item.spec().uses.contains(&Use::LookOut) {
            &[true, false]
        } else {
            &[false]
        };
        let tries = looks
            .iter()
            .flat_map(|&look| walls.iter().map(move |&wall| (look, wall)));
        tries
            .into_iter()
            .find_map(|(look, (through, _, strip, e, side))| {
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
                    boxed: false.into(),
                    // She never chose where it stands.
                    settled: false,
                };
                // Its box stands on the floor, where she can get to it to
                // unpack it (judged as her seats are, not as room to use a
                // piece: a box narrower than her at a wall has her take in
                // the wall's line); then the piece, standing there or hung
                // above, must fit with room to use it.
                let parcel = Prop {
                    boxed: true.into(),
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
                let at = doored.admits(drawn, shown, blocked, plan, parcel, Room::None)?;
                let room = if look { Room::ToLook } else { Room::ToUse };
                doored.admits(drawn, shown, blocked, plan, prop, room)?;
                // The room as it will show with the box in it, then with the
                // piece out of it: her seats there are judged on each.
                let offers = |prop: Prop, wants: &[Use]| {
                    let mut with = self.clone();
                    with.door = door;
                    with.props.push(prop);
                    let (after, _) = with.project_with(drawn, plan, blocked);
                    let Some(piece) = after.iter().find(|s| s.item == item) else {
                        return false;
                    };
                    let seats = seats(piece, &after, &with);
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
                        boxed: true.into(),
                        ..prop
                    },
                    Flap {
                        x,
                        rows: (at.floor - 2, at.floor),
                        side,
                        door: through,
                        item,
                    },
                ))
            })
    }

    /// Whether a delivery of `item` against `wall` comes in through her
    /// door's flap (door batch D9): `wall` is her door's wall with it in
    /// (her first piece's arrival chooses it), and her door's space there
    /// is kept (her pieces leave it free; the parcel stands beside it).
    /// Its space yielding to a piece hung there (her window, with its
    /// poster beside it, has nowhere else to hang), the parcel can still
    /// stand on the floor past it: it comes in by the wall's own flap
    /// (`a_parcel_by_a_yielding_space_comes_through_the_walls_flap`).
    fn through_her_door(&self, plan: Plan, wall: DoorWall, item: Furniture) -> bool {
        let mut with = self.clone();
        with.props.push(Prop {
            item,
            strip: wall.strip,
            anchor: Some(Anchor {
                side: wall.side,
                offset: 0,
            }),
            at: match wall.side {
                Side::Left => 0,
                Side::Right => 1000,
            },
            facing: match wall.side {
                Side::Left => Facing::Right,
                Side::Right => Facing::Left,
            },
            boxed: true.into(),
            settled: false,
        });
        if with.wall(plan) != Some(wall) {
            return false;
        }
        Home {
            door: Some(wall),
            ..self.clone()
        }
        .extents(plan.nooks)
        .iter()
        .find(|p| p.strip == wall.strip)
        .and_then(|p| p.space)
        .is_some_and(|space| space.kept)
    }

    /// Where `prop`, new, would stand on its strip this frame (her door
    /// where this home has it: the caller sets it as it will be), if the
    /// pieces in its lane there make way for it, packed in order
    /// (standing, on the floor beside her door's space; hung, on the raw
    /// wall): every one of them that shows still fits, and it fits on
    /// blank, free cells, with the `room` around it she'd need. A piece
    /// that hangs is judged where the layout hangs it beside a low
    /// shelf's block (phase 6a D3a), if packing puts it there. Never
    /// where the frame keeps new pieces out (`Keep`: the chat pane and
    /// her door's space, the piece where it's laid out), nor pushing one
    /// that shows into it, nor where its coming makes her door's kept
    /// space yield (door batch, D3).
    fn admits(
        &self,
        drawn: Drawn,
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        plan: Plan,
        prop: Prop,
        room: Room,
    ) -> Option<Shown> {
        let (strip, lane) = (prop.strip, prop.lane());
        let mut with = self.clone();
        with.props.push(prop);
        let new = with.props.len() - 1;
        let plans = with.extents(plan.nooks);
        let strip_plan = *plans.iter().find(|p| p.strip == strip)?;
        let e = strip_plan.along(lane);
        // Her door's space, kept before it came, is kept with it there.
        let kept = |plans: &[StripPlan], door: Option<DoorWall>| {
            let door = door?;
            plans
                .iter()
                .find(|p| p.strip == door.strip)?
                .space
                .map(|s| s.kept)
        };
        if kept(&self.extents(plan.nooks), self.door) == Some(true)
            && kept(&plans, with.door) != Some(true)
        {
            return None;
        }
        // Where the frame will lay them out (the new piece among them).
        let laid_out = with.laid_and_shifted(plan.nooks).shown;
        // Each where packing in order puts it; but one it puts on a low
        // shelf's block where the layout hangs it, beside the block
        // ([`Home::hung_clear`]), or none if it's left out for want of a
        // clear place: the block has no stand-in among what shows, as
        // what stands has for her window (pushed over a piece that
        // stands, it's judged where packing puts it, meeting that piece,
        // and refused, as before phase 6a). Only a block ever moves a
        // poster or her clock, so a piece that hangs is judged where it
        // will hang, but her window moved off what stands.
        let packed: Vec<(usize, Option<Shown>)> = pack(&with.on(strip, lane), e)?
            .into_iter()
            .filter_map(|(i, left)| {
                let at = stand(with.props.get(i)?, strip, e, strip_plan.host, left);
                if !strip_plan.block.is_some_and(|b| at.rect().intersects(b)) {
                    return Some((i, Some(at)));
                }
                let laid = laid_out
                    .iter()
                    .find(|s| s.strip == Some(strip) && s.item == at.item)
                    .copied();
                Some((i, laid))
            })
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
                    && !packed.iter().any(|(j, s)| {
                        *j != i && s.is_some_and(|s| s.rect().contains((x as u16, y as u16).into()))
                    })
            };
            // The piece clear of what it may not overlap; her, of all.
            let clear = |x: i32, y: i32| clear_for(Some(at), x, y);
            let her = |x: i32, y: i32| clear_for(None, x, y);
            fits(drawn, at, &clear)
                && (i != new
                    || match room {
                        Room::None => true,
                        Room::ToUse => roomy(drawn.buf(), at, &her),
                        Room::ToLook => {
                            roomy(drawn.buf(), at, &her) && room_to_look(drawn.buf(), at, &her)
                        }
                    })
        };
        let all_fit = packed.iter().all(|(i, at)| {
            let was = with.props.get(*i).map(|p| p.item);
            let showing = shown
                .iter()
                .any(|s| Some(s.item) == was && s.scrap.is_none());
            (*i != new && !showing) || at.is_some_and(|at| fits_here(*i, &at))
        });
        let at = packed
            .iter()
            .find(|(i, _)| *i == new)
            .and_then(|(_, at)| *at)?;
        // What the frame keeps out: the piece where it will be laid out
        // (a window hung clear of what stands), and none that shows put
        // newly in it by its coming, judged where the layout puts it (in
        // either lane: what the new piece pushes along, and what hangs
        // clear of it). One that goes to the closet isn't put anywhere.
        let keep = Keep::of(&with, plan);
        let laid = laid_out
            .iter()
            .find(|s| s.item == prop.item)
            .copied()
            .unwrap_or(at);
        (all_fit && !keep.refuses(laid.cover()) && !keep.takes_in(shown, &laid_out)).then_some(at)
    }

    /// Take ownership of `prop`, unless she already has one (then this
    /// returns false: one of each).
    pub fn add(&mut self, prop: Prop) -> bool {
        if self.owns(prop.item) {
            return false;
        }
        // A kit's bit as its accessor reads it (always boxed), so no two
        // kits differ by a bit nothing reads.
        self.props.push(Prop {
            boxed: prop.boxed().into(),
            ..prop
        });
        true
    }
}

/// Her seats at a piece shown among the pieces shown, her home as it
/// will be then (see [`Home::doorstep`]).
pub(super) type Seats<'a> = dyn Fn(&Shown, &[Shown], &Home) -> Vec<Seat> + 'a;

/// One question for [`Home::shelf_spot`], with everything it reads: her
/// home, the frame, its plan, what's kept off and the pane she unpacks
/// in. [`ShelfAsk::basis`] hashes all of it, so a memo keyed on it
/// (`kit_room`'s: phase 6a D9, the user's eighth round) is asked again
/// whenever any of it changes, and holds a `u64`, not the frame. A new
/// argument of `shelf_spot` must come in here as a field for
/// [`ShelfAsk::spot`] to pass it, and `basis` takes this, [`Plan`] and
/// [`KeepOff`] apart whole, so a field added to any of them doesn't
/// build until it's hashed too (the stale-memo trap `Home::lays_as`
/// warns of for her layouts).
pub(super) struct ShelfAsk<'a> {
    pub home: &'a Home,
    pub buf: &'a Buffer,
    pub plan: Plan<'a>,
    pub keep_off: &'a KeepOff<'a>,
    pub kit_at: Option<Nook>,
}

impl ShelfAsk<'_> {
    /// [`Home::shelf_spot`]'s answer.
    pub fn spot(&self) -> Option<Shelf> {
        self.home
            .shelf_spot(self.buf, self.plan, self.keep_off, self.kit_at)
    }

    /// A hash of everything [`ShelfAsk::spot`] reads (the frame's every
    /// cell as `Buffer`'s own `Hash` takes it, which agrees with its
    /// `==`: what the memo compared whole before). FxHash, as the
    /// client's other input fingerprints (`ListGroupsCache`): SipHash
    /// over a frame each paint costs more than the question.
    pub fn basis(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let Self {
            home,
            buf,
            plan,
            keep_off,
            kit_at,
        } = self;
        let Plan {
            nooks,
            chat,
            screen,
        } = plan;
        let KeepOff {
            protected,
            layer,
            made,
        } = keep_off;
        let mut hasher = rustc_hash::FxHasher::default();
        (
            home, buf, nooks, chat, screen, protected, layer, made, kit_at,
        )
            .hash(&mut hasher);
        hasher.finish()
    }
}

/// A pane's back wall as [`Home::shelf_spot`] reads it, in one pass over
/// the pane's interior rows a shelf could hang in: for each cell where a
/// run or its margin could lie, how many cells from it upward, itself
/// first, pass D4's per-cell test ([`structures::free`]) in its background (0 where
/// it fails); and for each row, whether the pane's interior has text on
/// it.
struct Wall {
    /// Its first column (the margin's first of a run `EDGE` in from the
    /// left wall), its columns and its first row (the pane's top inside).
    x0: i32,
    columns: usize,
    top: i32,
    /// By row from `top`, then column from `x0`.
    up: Vec<u16>,
    bg: Vec<Color>,
    /// By row from `top`.
    text: Vec<bool>,
}

impl Wall {
    /// The wall of the pane whose floor is `host`, or `None` where no
    /// shelf could hang by geometry (too narrow, or too short for a low
    /// one).
    fn read(
        buf: &Buffer,
        host: Extent,
        plan: Plan,
        keep_off: &KeepOff,
        covers: &[Rect],
        visited: &mut usize,
    ) -> Option<Self> {
        let x0 = host.from + EDGE - MARGIN;
        let x1 = host.to - EDGE + MARGIN;
        if x1 - x0 < SHELF_WIDTH + 2 * MARGIN || host.rows < LOW_RISE + LOW_ABOVE {
            return None;
        }
        let top = host.floor - i32::from(host.rows);
        let bottom = host.floor - i32::from(LOW_RISE);
        let columns = usize::try_from(x1 - x0).ok()?;
        // What `keep_off` and her pieces' covers take of the cells the
        // per-cell test is asked of, mapped once: the test then asks the
        // frame and the chat pane alone, so the wall costs its cells plus
        // what's kept off (clipped to it), never their product (a visit's
        // layer can hold hundreds of cells).
        let rows = usize::try_from(bottom - top + 1).ok()?;
        let mut kept = vec![false; rows * columns];
        let mut keep = |x: i32, y: i32| {
            if (x0..x1).contains(&x) && (top..=bottom).contains(&y) {
                let (Ok(row), Ok(column)) = (usize::try_from(y - top), usize::try_from(x - x0))
                else {
                    return;
                };
                kept[row * columns + column] = true;
            }
        };
        for &(x, y) in keep_off.layer {
            keep(i32::from(x), i32::from(y));
        }
        for r in keep_off.protected.iter().chain(keep_off.made).chain(covers) {
            let (from, to) = (i32::from(r.x).max(x0), i32::from(r.right()).min(x1));
            let (up, down) = (
                i32::from(r.y).max(top),
                i32::from(r.bottom()).min(bottom + 1),
            );
            for y in up..down {
                for x in from..to {
                    keep(x, y);
                }
            }
        }
        let nothing = KeepOff {
            protected: &[],
            layer: &[],
            made: &[],
        };
        let mut wall = Self {
            x0,
            columns,
            top,
            up: Vec::new(),
            bg: Vec::new(),
            text: Vec::new(),
        };
        for y in top..=bottom {
            let mut text = false;
            for x in host.from..host.to {
                *visited += 1;
                let cell = u16::try_from(x)
                    .ok()
                    .zip(u16::try_from(y).ok())
                    .and_then(|at| buf.cell(at));
                text |= cell.is_some_and(|c| !c.symbol().trim().is_empty());
                if !(x0..x1).contains(&x) {
                    continue;
                }
                // The per-cell test in the cell's own background; a place
                // asks its pane's (`best`).
                let bg = cell.map_or(Color::Reset, |c| c.bg);
                let free =
                    !kept[wall.up.len()] && structures::free(buf, (x, y), plan, &nothing, &[], bg);
                let above = match wall.at(x, y - 1) {
                    Some(i) if y > top && wall.bg[i] == bg => wall.up[i],
                    _ => 0,
                };
                wall.up.push(if free { above + 1 } else { 0 });
                wall.bg.push(bg);
            }
            wall.text.push(text);
        }
        Some(wall)
    }

    /// The index of `(x, y)` in what was read, if it was.
    fn at(&self, x: i32, y: i32) -> Option<usize> {
        let column = usize::try_from(x - self.x0).ok()?;
        let row = usize::try_from(y - self.top).ok()?;
        (column < self.columns).then_some(row * self.columns + column)
    }

    /// The best place on it for a shelf of `high`'s height (see
    /// [`Home::shelf_spot`] for the order), `hung` the hung pieces of its
    /// pane as laid out, the run's middle nearest `middle` (twice the
    /// column: the pane's, or her gremlin's in his pane).
    #[allow(clippy::too_many_arguments)]
    fn best(
        &self,
        buf: &Buffer,
        nook: Nook,
        host: Extent,
        high: bool,
        hung: &[Rect],
        middle: i32,
        visited: &mut usize,
    ) -> Option<Shelf> {
        let her = u16::try_from(sprite::HEIGHT).unwrap_or(u16::MAX);
        let (rises, over) = if high {
            (HIGH_RISE..=host.rows.saturating_sub(her), her)
        } else {
            (LOW_RISE..=LOW_RISE, LOW_ABOVE)
        };
        let width = u16::try_from(SHELF_WIDTH).unwrap_or(u16::MAX);
        // Text on its row, touching a hung piece, its rise, twice its
        // middle's distance from the pane's, its left.
        type Key = (bool, bool, u16, i32, i32);
        let mut best: Option<(Key, Shelf)> = None;
        for rise in rises {
            if host.rows < rise.saturating_add(over) {
                continue;
            }
            let y = host.floor - i32::from(rise);
            let Some(text) = usize::try_from(y - self.top)
                .ok()
                .and_then(|row| self.text.get(row).copied())
            else {
                continue;
            };
            for from in host.from + EDGE..=host.to - EDGE - SHELF_WIDTH {
                let to = from + SHELF_WIDTH;
                // Its pane's background, as `read_ground` reads it.
                *visited += 1;
                let bg = u16::try_from(from + SHELF_WIDTH / 2)
                    .ok()
                    .zip(u16::try_from(host.floor).ok())
                    .and_then(|at| buf.cell(at))
                    .map_or(Color::Reset, |c| c.bg);
                let clear = (from - MARGIN..to + MARGIN).all(|x| {
                    *visited += 1;
                    let need = if (from..to).contains(&x) { over } else { 0 };
                    self.at(x, y)
                        .is_some_and(|i| self.bg[i] == bg && self.up[i] > need)
                });
                if !clear {
                    continue;
                }
                let (anchor, _) = host.pin(from, width);
                let shelf = Shelf {
                    nook,
                    anchor,
                    rise,
                    built: 0,
                    fresh: None,
                };
                // Its block clear of what hangs: with today's hung kinds
                // (the poster and the clock fill both of its rows) the
                // covers the cells were judged against refuse the same
                // places, and each check alone stands in for the other;
                // this one stands for a hung kind that doesn't.
                if !high
                    && let Some(block) = shelf.block(host)
                    && hung.iter().any(|r| r.intersects(block))
                {
                    continue;
                }
                let touches = high
                    && hung.iter().any(|r| {
                        i32::from(r.y) == y + 1
                            && i32::from(r.x) < to
                            && i32::from(r.right()) > from
                    });
                let away = (2 * from + SHELF_WIDTH - middle).abs();
                let key = (text, touches, rise, away, from);
                if best.is_none_or(|(was, _)| key < was) {
                    best = Some((key, shelf));
                }
            }
        }
        best.map(|(_, shelf)| shelf)
    }
}

/// `prop` as it stands at `left` on `strip` (between its ends
/// `extent`), reached from `host`'s floor (its pane's: see
/// [`Shown::stood`]).
fn stand(prop: &Prop, strip: Strip, extent: Extent, host: Extent, left: i32) -> Shown {
    debug_assert!(
        !strip.is_floor() || extent.floor == host.floor,
        "a floor's pieces are reached from it: {strip:?} {extent:?} {host:?}"
    );
    Shown {
        item: prop.item,
        facing: prop.facing,
        boxed: prop.boxed,
        strip: Some(strip),
        left,
        floor: extent.floor,
        stood: host.floor,
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
            // Never asked (not `asks_room`, not a look out): a piece goes
            // up on her shelf whether or not she could stand beside it,
            // and its spots need its shelf's run, which the frame knows
            // (`spots` in the paint), not this. Before the `inside()`
            // arm, as everywhere a use's spots are matched.
            Use::GazeUp => return true,
            _ if what.inside() => {
                let seat = at.seat(what, 0);
                return fits(seat.x, seat.y);
            }
            _ => &at.beside(),
        };
        spots
            .iter()
            .any(|&x| fits(x, at.stood) && floor(x, at.stood))
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
/// glyphs' halves are never blank). A shelf is a floor only for what's
/// on it (phase 6a D6): a piece on a shelf stands only where every cell
/// of its floor row is one her shelf was `written` in this frame (the
/// app's own `────` across a hidden run holds nothing up), and no other
/// piece stands on a written cell.
pub(super) fn fits(drawn: Drawn, at: &Shown, clear: &dyn Fn(i32, i32) -> bool) -> bool {
    let (buf, written) = (drawn.buf(), drawn.written());
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
    let on_shelf = at.on_shelf();
    // Hung, it needs no floor beneath it.
    let floor = at.lane() == Lane::Wall
        || (at.left..at.left + i32::from(cols)).all(|x| {
            clear(x, at.floor)
                && if on_shelf {
                    written.contains(x, at.floor)
                } else {
                    !written.contains(x, at.floor)
                        && cell(x, at.floor)
                            .and_then(|c| c.symbol().chars().next())
                            .is_some_and(|c| strokes(c).is_some())
                }
        });
    body && floor
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::test_runner::TestCaseError;

    /// Her seats at `piece` (every way she'd use it) as if she could stand
    /// anywhere: for a doorstep that judges only the room's cells.
    pub(super) fn anywhere(piece: &Shown, _: &[Shown], _: &Home) -> Vec<Seat> {
        piece
            .uses()
            .iter()
            .map(|&what| piece.seat(what, piece.beside()[0]))
            .collect()
    }

    /// A strip's extent: columns `from..to`, its floor row, the rows
    /// clear above it.
    fn extent(from: i32, to: i32, floor: i32, rows: u16) -> Extent {
        Extent {
            from,
            to,
            floor,
            rows,
        }
    }

    /// Her seats at `piece` as if she could stand nowhere.
    fn nowhere(_: &Shown, _: &[Shown], _: &Home) -> Vec<Seat> {
        Vec::new()
    }

    /// What each strip takes (phase 6a D1): a floor, every kind, boxed
    /// or not; a shelf, small decor only (the plant and the clock) out of
    /// its box, never furniture, a poster, her window or a parcel. Every
    /// pane's strips alike; a strip's pane and room are its own pane's
    /// floor.
    #[test]
    fn a_floor_takes_anything_and_a_shelf_small_decor_only() {
        use Furniture::*;
        let small = [Plant, Clock, Gremlin];
        for nook in [Nook::List, Nook::Users, Nook::Playlist] {
            for (strip, boxed, wants) in [
                (Strip::Bottom(nook), false, Furniture::ALL.to_vec()),
                (Strip::Bottom(nook), true, Furniture::ALL.to_vec()),
                (Strip::Shelf(nook), false, small.to_vec()),
                (Strip::Shelf(nook), true, Vec::new()),
            ] {
                assert_eq!(strip.nook(), nook);
                assert_eq!(strip.room(), Strip::Bottom(nook));
                let takes: Vec<Furniture> = Furniture::ALL
                    .into_iter()
                    .filter(|&item| strip.takes(item, boxed))
                    .collect();
                assert_eq!(takes, wants, "{strip:?}, boxed {boxed}");
            }
        }
        // The table, row by row: nothing but small decor says it may.
        for item in Furniture::ALL {
            assert_eq!(item.spec().shelf, small.contains(&item), "{item:?}");
        }
    }

    /// On a shelf everything stands (phase 6a D1): the wall clock there
    /// is a mantel clock, laned and lifted as standing whether it's
    /// weighed for the shelf (a candidate) or stands on it; on a floor
    /// it hangs. Boxed, it's a parcel on either.
    #[test]
    fn on_a_shelf_everything_stands() {
        let hang = Furniture::Clock.spec().hang.unwrap();
        let rows = Furniture::Clock.spec().footprint.1;
        let (floor, shelf) = (Strip::Bottom(Nook::Users), Strip::Shelf(Nook::Users));
        for boxed in [false, true] {
            let clock = Prop {
                boxed: boxed.into(),
                ..Prop::new(Furniture::Clock, Nook::Users, 0, Facing::Right)
            };
            let hung = if boxed { 0 } else { hang };
            assert_eq!(clock.lane_on(shelf), Lane::Floor, "boxed={boxed}");
            assert_eq!(clock.lane_on(floor), lane_of(hung), "boxed={boxed}");
            let on_shelf = Prop {
                strip: shelf,
                ..clock
            };
            assert_eq!((on_shelf.lane(), on_shelf.needs().1), (Lane::Floor, rows));
            assert_eq!(clock.needs().1, rows + hung, "boxed={boxed}");
            let shown = stand(&clock, floor, extent(0, 20, 10, 9), extent(0, 20, 10, 9), 2);
            assert_eq!(
                (shown.lift_on(shelf), shown.lane_on(shelf)),
                (0, Lane::Floor)
            );
            assert_eq!(shown.lift_on(floor), hung, "boxed={boxed}");
            let up = Shown {
                strip: Some(shelf),
                ..shown
            };
            assert_eq!((up.lift(), up.lane()), (0, Lane::Floor), "boxed={boxed}");
        }
    }

    /// Rooms are floors (phase 6a D1): a piece on a pane's shelf is in
    /// that pane's room, asked by its floor or its shelf, and in no other
    /// pane's; a room's role reads what's on its shelf with what's on its
    /// floor, asked either way.
    #[test]
    fn a_shelf_is_part_of_the_room_below() {
        let users = extent(0, 30, 10, 9);
        let at = |item: Furniture, strip: Strip, left: i32| {
            let prop = Prop {
                strip,
                ..Prop::new(item, strip.nook(), 0, Facing::Right)
            };
            stand(&prop, strip, users, users, left)
        };
        let (floor, shelf) = (Strip::Bottom(Nook::Users), Strip::Shelf(Nook::Users));
        let plant = at(Furniture::Plant, shelf, 2);
        assert!(in_room(&plant, floor) && in_room(&plant, shelf));
        assert!(!in_room(&plant, Strip::Bottom(Nook::List)));
        assert!(!in_room(&plant, Strip::Shelf(Nook::List)));
        let bed = at(Furniture::Bed, floor, 10);
        assert!(in_room(&bed, shelf));
        let laid = [plant, bed];
        assert_eq!(role_of(&laid, floor), Role::Bedroom);
        assert_eq!(role_of(&laid, shelf), Role::Bedroom);
        assert_eq!(role_without(&laid, shelf, Furniture::Bed), Role::Den);
        assert_eq!(role_of(&laid, Strip::Shelf(Nook::List)), Role::Den);
    }

    /// The admission rule as every layout reads it (phase 6a D1):
    /// [`Home::on`] lists, of a strip's anchored pieces, only those it
    /// takes. A sofa an odd record put on a shelf is never laid out
    /// there; a plant is.
    #[test]
    fn a_shelf_lays_out_only_what_it_takes() {
        let shelf = Strip::Shelf(Nook::Users);
        let anchor = Anchor {
            side: Side::Left,
            offset: 0,
        };
        let mut home = Home::default();
        for item in [Furniture::Sofa, Furniture::Plant] {
            assert!(home.add(Prop {
                strip: shelf,
                anchor: Some(anchor),
                ..Prop::new(item, Nook::Users, 0, Facing::Right)
            }));
        }
        let on: Vec<Furniture> = home
            .on(shelf, Lane::Floor)
            .into_iter()
            .map(|(i, ..)| home.props[i].item)
            .collect();
        assert_eq!(on, [Furniture::Plant]);
    }

    /// Where she stands to reach a piece is [`Shown::stood`], not its own
    /// floor (phase 6a D1: the pane's floor under a shelf): her seats at
    /// it are on that floor, and a piece placed by its plan takes it
    /// from the plan's host.
    #[test]
    fn she_reaches_a_shelf_piece_from_the_floor_below() {
        let (host, run) = (extent(0, 30, 12, 11), extent(8, 18, 6, 5));
        let shelf = Strip::Shelf(Nook::Users);
        let prop = Prop {
            strip: shelf,
            ..Prop::new(Furniture::Plant, Nook::Users, 0, Facing::Right)
        };
        let up = stand(&prop, shelf, run, host, 9);
        assert_eq!((up.floor, up.stood), (6, 12));
        for what in Use::ALL {
            assert_eq!(up.seat(what, 3).y, 12, "{what:?}");
        }
        let plan = StripPlan {
            strip: shelf,
            raw: run,
            floor: run,
            space: None,
            host,
            block: None,
        };
        let moved = at_left(&Shown { stood: 6, ..up }, &plan, 10);
        assert_eq!(
            (moved.strip, moved.floor, moved.stood),
            (Some(shelf), 6, 12)
        );
    }

    /// An older record's share of the way along is of the host both ways
    /// (phase 6a D1): [`RawStrip::anchor_at`] reads it back where
    /// [`StripPlan::pin`] wrote it, on a shelf as on a floor, give or take
    /// the column a thousandth rounds away; and a share the run can't
    /// hold puts the piece at the run's nearer end.
    #[test]
    fn a_share_is_of_the_host_both_ways() {
        let host = extent(1, 61, 12, 11);
        let cols = Furniture::Plant.spec().footprint.0;
        for (strip, raw) in [
            (Strip::Bottom(Nook::Users), host),
            (Strip::Shelf(Nook::Users), extent(20, 30, 6, 5)),
        ] {
            let plan = StripPlan {
                strip,
                raw,
                floor: raw,
                space: None,
                host,
                block: None,
            };
            let at = RawStrip { strip, raw, host };
            for left in raw.from..=raw.to - i32::from(cols) {
                let (_, share) = plan.pin(Lane::Floor, left, cols);
                let back = raw.left(at.anchor_at(share, cols), cols);
                assert!(
                    (back - left).abs() <= 1,
                    "{strip:?}: {left} came back {back}"
                );
            }
        }
        let shelf = RawStrip {
            strip: Strip::Shelf(Nook::Users),
            raw: extent(20, 30, 6, 5),
            host,
        };
        assert_eq!(shelf.raw.left(shelf.anchor_at(0, cols), cols), 20);
        assert_eq!(shelf.raw.left(shelf.anchor_at(1000, cols), cols), 27);
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
                    Drawn::bare(&buf),
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
                    Drawn::bare(&buf),
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
            boxed: true.into(),
            ..prop(Tv, 1000)
        }));
        assert_eq!(boxed.rooms(), [(Strip::Bottom(Nook::Users), Role::Den)]);
        assert!(boxed.unbox(Tv));
        assert_eq!(boxed.rooms(), [(Strip::Bottom(Nook::Users), Role::Living)]);
    }

    use proptest::prelude::*;

    /// The pieces that stand on the floor.
    fn standing() -> Vec<Furniture> {
        Furniture::PRE_SHELF
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
    pub(super) fn placed(items: Vec<Furniture>) -> impl Strategy<Value = Vec<Prop>> {
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
                prop_assert!(fits(Drawn::bare(&noisy), s, &|x, y| !block(x, y)), "{:?}", s);
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
    pub(super) fn two_panes(
        width: u16,
        height: u16,
        text: &[(u16, u16)],
    ) -> (Buffer, [(Nook, Rect); 2]) {
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
            boxed: boxed.into(),
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
            boxed: true.into(),
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
                    Drawn::bare(&clean),
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
                Drawn::bare(&clean),
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Poster,
            )
            .expect("room for it");
        assert!(prop.boxed());
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
                Drawn::bare(&clean),
                Plan::bare(&nooks),
                &shown,
                &|_, _| false,
                &|piece, _, _| {
                    let unpack = piece.seat(Use::Unpack, 0);
                    match unpack.x != 21 || !piece.boxed() {
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
                Drawn::bare(&clean),
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
                Drawn::bare(&hung_over),
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
                Drawn::bare(&hung_over),
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
                Drawn::bare(&floored),
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
                Drawn::bare(&low),
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
                    Drawn::bare(&low),
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
                Drawn::bare(buf),
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
                boxed: false.into(),
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
                let e = pane_strips(&nooks).into_iter().find(|p| Some(p.strip) == s.strip).unwrap().raw;
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
            let small_users = pane_strips(&at_small)
                .into_iter()
                .find(|p| p.strip == users_strip)
                .map(|p| p.raw)
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
    pub(super) fn door_on(nook: Nook, side: Side) -> Option<DoorWall> {
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
            structures: Vec::new(),
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
        let mut room = Home {
            props,
            door,
            structures: Vec::new(),
        };
        let (buf, nooks) = two_panes(wide, 12, &[]);
        let plan = Plan {
            nooks: &nooks,
            chat: Rect::default(),
            screen: buf.area,
        };
        let (first, _) = room.frame(Drawn::bare(&buf), plan, &|_, _| false);
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
                let raw = pane_strips(&nooks)
                    .into_iter()
                    .find(|p| p.strip == wall.strip)
                    .unwrap()
                    .raw;
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
        if pane_strips(&at_small)
            .iter()
            .any(|p| pack(&pinned.on(p.strip, Lane::Floor), p.raw).is_none())
        {
            // Too narrow for what stands: they may move off.
            return Ok(());
        }
        let small_plan = Plan {
            nooks: &at_small,
            chat: Rect::default(),
            screen: small.area,
        };
        let _ = room.frame(Drawn::bare(&small), small_plan, &|_, _| false);
        prop_assert_eq!(&room, &pinned, "a resize moved a piece, or her door");
        let (again, changed) = room.frame(Drawn::bare(&buf), plan, &|_, _| false);
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
                structures: Vec::new(),
            };
            let (shown, _) = room.frame(Drawn::bare(&buf), plan, &|_, _| false);
            let gifts: Vec<Furniture> = Furniture::PRE_SHELF.into_iter().filter(|&f| !room.owns(f)).collect();
            prop_assume!(!gifts.is_empty());
            let item = gifts[gift.index(gifts.len())];
            let Some(prop) = room.spot(Drawn::bare(&buf), plan, &shown, &|_, _| false, item, &mut Rng(seed)) else {
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
            let mut room = Home { props, door: door_on(Nook::Users, side), structures: Vec::new() };
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

        /// Her door's space is kept just when, laid out beside it, her
        /// strip lays out every piece it lays out without it: the plan
        /// that lays it out without the space only when a piece is left
        /// out beside it (her window, behind a crowded floor) judges as
        /// laying out both would. Her pieces on another strip change
        /// nothing of it.
        #[test]
        fn her_door_space_is_kept_as_laying_out_both_ways_judges(
            props in decorated(),
            elsewhere in proptest::collection::vec(proptest::bool::weighted(0.25), 6),
            wide in 16u16..60,
            tall in 6u16..14,
            right in any::<bool>(),
        ) {
            let side = if right { Side::Right } else { Side::Left };
            let (_, nooks) = two_panes(wide, tall, &[]);
            let users = Strip::Bottom(Nook::Users);
            let props = props
                .into_iter()
                .zip(&elsewhere)
                .map(|(p, &away)| Prop {
                    strip: if away { Strip::Bottom(Nook::Playlist) } else { users },
                    ..p
                })
                .collect();
            let mut room = Home { props, door: door_on(Nook::Users, side), structures: Vec::new() };
            room.layout(&nooks);
            let PaneStrip { strip, raw, .. } = pane_strips(&nooks)[0];
            prop_assert_eq!(strip, users);
            let plan = room.extents(&nooks)[0];
            let Some((wall, rect)) = space_of(raw, side, room.widest_standing(users)) else {
                prop_assert_eq!(plan.space, None);
                return Ok(());
            };
            let shown = |l: Option<(LaidOut, usize)>| l.map(|(l, _)| l.shown);
            let at = RawStrip { strip: users, raw, host: raw };
            let doored = shown(room.lay_strip(at, narrowed(raw, side), Some(rect)));
            let doorless = shown(room.lay_strip(at, raw, None));
            let kept = match (&doored, &doorless) {
                (Some(doored), Some(doorless)) => doorless
                    .iter()
                    .all(|s| doored.iter().any(|t| t.item == s.item)),
                (doored, None) => doored.is_some(),
                (None, Some(_)) => false,
            };
            prop_assert_eq!(plan.space, Some(Space { side, wall, rect, kept }));
            let laid = room.laid_and_shifted(&nooks);
            let want = if kept { doored } else { doorless };
            let want = want.unwrap_or_default();
            let got: Vec<Shown> = laid.shown.into_iter().filter(|s| s.strip == Some(users)).collect();
            prop_assert_eq!(got, want);
        }

        /// A home laid out again from another's strips laid out alone
        /// (only those whose pieces or door differ laid out afresh) lays
        /// out just as laying it all out does (shown, shifts and plans,
        /// in order), whatever moves where: one piece or two, across
        /// strips, along one, turned, boxed or unboxed, settled or not,
        /// off a strip that isn't here or onto one, her door moved or
        /// gone, its space kept or yielding (her door often on a strip a
        /// piece leaves or comes to, and sometimes on a narrow strip
        /// crowded with her pieces). Her repair search weighs every move
        /// this way, keeping each strip one move laid out again for the
        /// next ([`Home::relaid_keeping`]): so too every home on the way
        /// there and back, from one `LaidAgain`. The oracle is `laid_and_shifted` as it is now (the
        /// same strip-by-strip layout, gathered); that it lays out as the
        /// old whole-home pass did is
        /// `her_door_space_is_kept_as_laying_out_both_ways_judges`' and
        /// the goldens' to show.
        #[test]
        fn a_move_relaid_strip_by_strip_is_laid_out_as_a_whole(
            props in decorated(),
            on in proptest::collection::vec(0usize..3, 6),
            panes in 2usize..=3,
            sizes in proptest::collection::vec((14u16..60, 6u16..14), 3),
            crowded in proptest::option::of(14u16..24),
            door in (0usize..4, 0usize..3, any::<bool>()),
            redoor in proptest::option::of(proptest::option::of((0usize..3, any::<bool>()))),
            moves in proptest::collection::vec(
                (
                    any::<proptest::sample::Index>(),
                    (0usize..3, any::<bool>(), 0u16..40, any::<bool>(), 0u16..=1000),
                    any::<bool>(),
                    any::<bool>(),
                ),
                1..=2,
            ),
        ) {
            let order = [Nook::Users, Nook::Playlist, Nook::List];
            let side = |right: bool| if right { Side::Right } else { Side::Left };
            let first = moves[0].0.index(props.len());
            let (old, new) = (on[first % on.len()], moves[0].1 .0);
            // Her door: none, anywhere, or on the strip her first moved
            // piece leaves or comes to.
            let (how, anywhere, right) = door;
            let door_at = match how {
                0 => None,
                1 => Some(anywhere),
                2 => Some(old),
                _ => Some(new),
            };
            // A crowded door strip: narrow, every piece on it.
            let crowd = crowded.zip(door_at);
            let mut x = 0;
            let nooks: Vec<(Nook, Rect)> = order
                .iter()
                .zip(&sizes)
                .enumerate()
                .take(panes)
                .map(|(k, (&nook, &(w, h)))| {
                    let w = match crowd {
                        Some((narrow, at)) if at == k => narrow,
                        _ => w,
                    };
                    let r = Rect::new(x, 0, w, h);
                    x += w;
                    (nook, r)
                })
                .collect();
            let props: Vec<Prop> = props
                .into_iter()
                .zip(on.iter().cycle())
                .map(|(p, &i)| {
                    let i = crowd.map_or(i, |(_, at)| at);
                    Prop { strip: Strip::Bottom(order[i]), ..p }
                })
                .collect();
            let mut home = Home {
                props,
                door: door_at.and_then(|i| door_on(order[i], side(right))),
                structures: Vec::new(),
            };
            let was = home.strips_laid_out(&nooks);
            prop_assert_eq!(home.relaid(&was), home.laid_and_shifted(&nooks));
            // And kept from move to move (as a search keeps them), each
            // home on the way and back again laid out as a whole too.
            let mut kept = LaidAgain::default();
            let mut homes = vec![home.clone()];
            for (moved, to, boxed, settled) in &moves {
                let i = moved.index(home.props.len());
                let &(strip, right, offset, left, at) = to;
                home.props[i] = Prop {
                    strip: Strip::Bottom(order[strip]),
                    anchor: Some(Anchor { side: side(right), offset }),
                    at,
                    facing: if left { Facing::Left } else { Facing::Right },
                    boxed: (*boxed).into(),
                    settled: *settled,
                    ..home.props[i]
                };
                prop_assert_eq!(home.relaid_keeping(&was, &mut kept), home.laid_and_shifted(&nooks));
                homes.push(home.clone());
            }
            if let Some(redoor) = redoor {
                home.door = redoor.and_then(|(i, right)| door_on(order[i], side(right)));
                homes.push(home.clone());
            }
            for home in homes.iter().rev() {
                prop_assert_eq!(home.relaid_keeping(&was, &mut kept), home.laid_and_shifted(&nooks));
            }
            let relaid = home.relaid(&was);
            prop_assert_eq!(&relaid, &home.laid_and_shifted(&nooks));
            // Strip by strip in the order she furnished them (her first
            // piece on each, by index), whichever strips it laid again.
            let mut strips: Vec<Option<Strip>> = relaid.shown.iter().map(|s| s.strip).collect();
            strips.dedup();
            let furnished: Vec<Option<Strip>> = home
                .furnished()
                .into_iter()
                .map(Some)
                .filter(|s| strips.contains(s))
                .collect();
            prop_assert_eq!(strips, furnished);
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
                    let at = stand(&prop, Strip::Bottom(Nook::Users), raw, raw, x);
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
        // A band across them laid out empty (a collapsed chat slot) is no
        // chat: the first edge wall, as with none.
        let collapsed = Rect::new(0, 7, 100, 0);
        assert_eq!(
            choose_wall(&mine, plan_on(&nooks, collapsed, screen)),
            door_on(Nook::Users, Side::Left)
        );
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
        let _ = moving.frame(Drawn::bare(&buf), full, &|_, _| false);
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
        let _ = moving.frame(Drawn::bare(&buf), hide, &|_, _| false);
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
        let _ = moving.frame(Drawn::bare(&buf), away, &|_, _| false);
        assert_eq!(moving.props[0].strip, Strip::Bottom(Nook::Playlist));
        assert_eq!(moving.door, door_on(Nook::Playlist, Side::Right));
        // Shown again: her door stays where it went (with her pieces).
        let _ = moving.frame(Drawn::bare(&buf), full, &|_, _| false);
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
        let _ = home.frame(Drawn::bare(&buf), full, &|_, _| false);
        assert_eq!(home.door, door_on(Nook::Users, Side::Left));
        // Users hidden: her pieces go to Playlist, anchored from its
        // right wall, the screen's edge, where her door goes too.
        let playlist = [nooks[1]];
        let hidden = plan_on(&playlist, Rect::default(), buf.area);
        let (moved, _) = home.frame(Drawn::bare(&buf), hidden, &|_, _| false);
        assert_eq!(home.door, door_on(Nook::Playlist, Side::Right));
        assert_eq!(moved.len(), 2, "{moved:?}");
        let space = home.extents(&playlist)[0].space.expect("a space");
        assert!(space.kept);
        for s in &moved {
            assert!(!s.cover().intersects(space.rect), "{s:?} in {space:?}");
        }
        let (next, changed) = home.frame(Drawn::bare(&buf), hidden, &|_, _| false);
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
        let _ = stays.frame(
            Drawn::bare(&buf),
            plan_on(&playlist, over, buf.area),
            &|_, _| false,
        );
        assert_eq!(stays, home, "moved under the chat");
        // The chat over the rows above it: they move.
        let above = Rect::new(30, 0, 40, 3);
        let mut moves = home.clone();
        let (shown, _) = moves.frame(
            Drawn::bare(&buf),
            plan_on(&playlist, above, buf.area),
            &|_, _| false,
        );
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
            structures: Vec::new(),
        };
        let (shown, _) = room.frame(Drawn::bare(&buf), plan, &|_, _| false);
        let lamp = room
            .spot(
                Drawn::bare(&buf),
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
        let _ = home.frame(
            Drawn::bare(&buf),
            plan_on(&rest, Rect::default(), screen),
            &|_, _| false,
        );
        assert_eq!(home.props[0].strip, Strip::Bottom(Nook::List));
        assert_eq!(home.door, door_on(Nook::List, Side::Right));
    }

    // ---- Keep-out for new placements (door batch, step 5) ----

    /// Whether her door's space, on the wall `home` has (or would choose)
    /// on `plan`, is kept: `None` with no wall, or no space on it.
    fn kept_on(home: &Home, plan: Plan) -> Option<bool> {
        let wall = home.wall(plan)?;
        let scratch = Home {
            door: Some(wall),
            ..home.clone()
        };
        scratch
            .extents(plan.nooks)
            .into_iter()
            .find(|p| p.strip == wall.strip)?
            .space
            .map(|s| s.kept)
    }

    /// Where `item` is laid out in `home` on `plan` as it comes in (her
    /// door where it will be with it there; what the next frame moves
    /// off a strip is `move_off`'s, checked apart), and what that frame
    /// keeps out.
    fn framed(home: &mut Home, plan: Plan, item: Furniture) -> Option<(Shown, Keep)> {
        home.door = home.wall(plan);
        let laid = home.laid_and_shifted(plan.nooks);
        let at = *laid.shown.iter().find(|s| s.item == item)?;
        Some((at, Keep::of(home, plan)))
    }

    /// A piece of `was` (showing) put newly in `keep` where `now` lays
    /// it out: outside it before, inside it now, moved.
    fn pushed_in(was: &[Shown], now: &[Shown], keep: &Keep) -> Option<(Shown, Shown)> {
        was.iter()
            .filter(|w| w.scrap.is_none() && !keep.refuses(w.cover()))
            .find_map(|w| {
                now.iter()
                    .find(|n| n.item == w.item && n.cover() != w.cover() && keep.refuses(n.cover()))
                    .map(|n| (*w, *n))
            })
    }

    /// The body of [`no_placement_meets_the_keep`]: `props` on
    /// [`two_panes`] (Users `wide` wide), a chat pane drawn at `chat`
    /// over part of the screen (a custom layout's grid overlapping her
    /// panes), her door saved at `door` or chosen. No new placement (a
    /// delivery boxed and unpacked, a stage gift, a repair and the pieces
    /// it pushes along, her pieces moved off a pane that's gone) meets
    /// the chat or her door's space; a delivery never makes a kept space
    /// yield.
    fn placements_keep_out(
        props: Vec<Prop>,
        wide: u16,
        chat: (u16, u16, u16, u16),
        door: Option<(bool, bool)>,
        which: proptest::sample::Index,
        seed: u64,
    ) -> Result<(), TestCaseError> {
        let (buf, nooks) = two_panes(wide, 12, &[]);
        let (cx, cy, cw, ch) = chat;
        let chat = Rect::new(cx, cy, cw, ch).intersection(buf.area);
        let plan = plan_on(&nooks, chat, buf.area);
        let mut room = Home {
            props,
            door: door.map(|(users, right)| DoorWall {
                strip: Strip::Bottom(if users { Nook::Users } else { Nook::Playlist }),
                side: if right { Side::Right } else { Side::Left },
            }),
            structures: Vec::new(),
        };
        let (shown, _) = room.frame(Drawn::bare(&buf), plan, &|_, _| false);
        let keep = Keep::of(&room, plan);
        let items: Vec<Furniture> = Furniture::PRE_SHELF
            .into_iter()
            .filter(|&f| !room.owns(f))
            .collect();
        // A delivery of one she hasn't got: boxed where it comes in, and
        // out of its box there.
        if let Some(&item) = items.get(which.index(items.len().max(1))) {
            let was = kept_on(&room, plan);
            if let Some((parcel, _)) = room.doorstep(
                Drawn::bare(&buf),
                plan,
                &shown,
                &|_, _| false,
                &anywhere,
                item,
            ) {
                for boxed in [true, false] {
                    let mut with = room.clone();
                    let added = with.add(Prop {
                        boxed: boxed.into(),
                        ..parcel
                    });
                    prop_assert!(added);
                    if was == Some(true) {
                        prop_assert_eq!(
                            kept_on(&with, plan),
                            Some(true),
                            "{:?} (boxed {}) made her door's space yield",
                            item,
                            boxed
                        );
                    }
                    if let Some((at, keep)) = framed(&mut with, plan, item) {
                        prop_assert!(
                            !keep.refuses(at.cover()),
                            "{:?} delivered (boxed {}) into {:?}",
                            at,
                            boxed,
                            keep
                        );
                        let now = with.laid_and_shifted(plan.nooks).shown;
                        let pushed = pushed_in(&shown, &now, &keep);
                        prop_assert!(
                            pushed.is_none(),
                            "{:?} (boxed {}) pushed {:?} into {:?}",
                            item,
                            boxed,
                            pushed,
                            keep
                        );
                    }
                }
            }
            // A stage gift of it.
            if let Some(gift) = room.spot(
                Drawn::bare(&buf),
                plan,
                &shown,
                &|_, _| false,
                item,
                &mut Rng(seed),
            ) {
                let mut with = room.clone();
                let added = with.add(gift);
                prop_assert!(added);
                if let Some((at, keep)) = framed(&mut with, plan, item) {
                    prop_assert!(!keep.refuses(at.cover()), "gift {:?} in {:?}", at, keep);
                    let now = with.laid_and_shifted(plan.nooks).shown;
                    let pushed = pushed_in(&shown, &now, &keep);
                    prop_assert!(
                        pushed.is_none(),
                        "gift {:?} pushed {:?} into {:?}",
                        item,
                        pushed,
                        keep
                    );
                }
            }
        }
        // Every repair of every rule broken: the piece where it's set
        // down, unless it stays where it stood (turned round: an older
        // piece in the chat is shown where it stands).
        let laid = room.laid_out(&nooks);
        let frame = super::super::rules::Frame {
            drawn: Drawn::bare(&buf),
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &shown,
            made: &[],
            keep: &keep,
        };
        for target in super::super::rules::broken(&laid, &room, &keep) {
            for repair in super::super::rules::search(&room, &frame, &target).repairs {
                let stood = laid.shown.iter().find(|s| s.item == repair.piece);
                prop_assert!(
                    !keep.refuses(repair.at.cover())
                        || stood.is_some_and(|s| s.cover() == repair.at.cover()),
                    "{:?} mended into {:?}",
                    repair.at,
                    keep
                );
                // Nor any piece it pushes along (or hangs clear of it)
                // put newly in it.
                let mut mended = room.clone();
                if let Some(p) = mended.props.iter_mut().find(|p| p.item == repair.piece) {
                    p.strip = repair.to.strip;
                    p.anchor = Some(repair.to.anchor);
                    p.facing = repair.to.facing;
                }
                let now = mended.laid_and_shifted(&nooks).shown;
                let others: Vec<Shown> = shown
                    .iter()
                    .filter(|s| s.item != repair.piece)
                    .copied()
                    .collect();
                let pushed = pushed_in(&others, &now, &keep);
                prop_assert!(
                    pushed.is_none(),
                    "{:?} mended pushed {:?} into {:?}",
                    repair.at,
                    pushed,
                    keep
                );
            }
        }
        // Her pieces moved off a pane that's gone (a guard: step 2's
        // `move_off` refuses both).
        for gone in [Nook::Users, Nook::Playlist] {
            let rest: Vec<(Nook, Rect)> =
                nooks.iter().copied().filter(|&(n, _)| n != gone).collect();
            let less = plan_on(&rest, chat, buf.area);
            let mut moved = room.clone();
            let (after, _) = moved.frame(Drawn::bare(&buf), less, &|_, _| false);
            let keep = Keep::of(&moved, less);
            // Nor any piece already where they went pushed newly into
            // it (judged against where it's laid out before the move).
            let before = room.laid_and_shifted(&rest).shown;
            let pushed = pushed_in(&before, &after, &keep);
            prop_assert!(
                pushed.is_none(),
                "moving off {:?} pushed {:?} into {:?}",
                gone,
                pushed,
                keep
            );
            for s in after
                .iter()
                .filter(|s| s.strip != Some(Strip::Bottom(gone)))
            {
                let was = room
                    .props
                    .iter()
                    .find(|p| p.item == s.item)
                    .map(|p| p.strip);
                if was == Some(Strip::Bottom(gone)) {
                    prop_assert!(!keep.refuses(s.cover()), "{:?} moved into {:?}", s, keep);
                }
            }
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(512)))]

        /// No new piece is put in the chat pane or her door's space (D3):
        /// see [`placements_keep_out`].
        #[test]
        fn no_placement_meets_the_keep(
            props in decorated(),
            wide in 24u16..60,
            chat in (0u16..100, 0u16..12, 1u16..40, 1u16..13),
            door in proptest::option::of((any::<bool>(), any::<bool>())),
            which in any::<proptest::sample::Index>(),
            seed in 0u64..1000,
        ) {
            placements_keep_out(props, wide, chat, door, which, seed)?;
        }
    }

    /// [`no_placement_meets_the_keep`]'s find: her cat's bed, window and
    /// poster moved off a hidden Users onto Playlist, her door on
    /// Playlist's left wall. The bed packs clear of the space, but the
    /// window, hung by its share of the way, lands in it (the space
    /// yields: no hung piece is shifted out of one that yields), so the
    /// move is refused: nothing is laid out in her door's space.
    #[test]
    fn a_window_moved_off_a_hidden_pane_never_hangs_in_her_door_space() {
        let (buf, nooks) = two_panes(26, 12, &[]);
        let mut home = Home {
            props: vec![
                Prop {
                    anchor: Some(Anchor {
                        side: Side::Right,
                        offset: 0,
                    }),
                    ..prop(Furniture::CatBed, 0)
                },
                Prop {
                    anchor: Some(Anchor {
                        side: Side::Left,
                        offset: 0,
                    }),
                    ..prop(Furniture::Window, 0)
                },
                prop(Furniture::Poster, 30),
            ],
            door: door_on(Nook::Playlist, Side::Left),
            structures: Vec::new(),
        };
        let playlist = [nooks[1]];
        let less = plan_on(&playlist, Rect::default(), buf.area);
        let (shown, _) = home.frame(Drawn::bare(&buf), less, &|_, _| false);
        let keep = Keep::of(&home, less);
        let laid = home.laid_and_shifted(&playlist).shown;
        for s in shown.iter().chain(&laid) {
            assert!(!keep.refuses(s.cover()), "{s:?} in {keep:?}");
        }
    }

    /// Her pieces moved off a pane that's gone push none already where
    /// they go newly into the chat pane (D3): her bed off a hidden Users
    /// onto Playlist, whose TV stands at its right wall; packed against
    /// the bed it would stand where the chat comes over (with no chat,
    /// it stands there: asserted), so the move is refused.
    #[test]
    fn pieces_moved_off_a_hidden_pane_push_none_into_the_chat() {
        let (buf, nooks) = two_panes(30, 12, &[]);
        let right = Some(Anchor {
            side: Side::Right,
            offset: 0,
        });
        let home = Home {
            props: vec![
                Prop {
                    strip: Strip::Bottom(Nook::Playlist),
                    anchor: right,
                    ..prop(Furniture::Tv, 0)
                },
                Prop {
                    anchor: right,
                    ..prop(Furniture::Bed, 0)
                },
            ],
            door: door_on(Nook::Playlist, Side::Left),
            structures: Vec::new(),
        };
        let playlist = [nooks[1]];
        let chat = Rect::new(53, 0, 5, 12);
        let tv_in_chat = |home: &Home| {
            home.laid_and_shifted(&playlist)
                .shown
                .iter()
                .any(|s| s.item == Furniture::Tv && in_chat(chat, s.cover()))
        };
        assert!(!tv_in_chat(&home), "the TV stands clear of it");
        let mut apart = home.clone();
        let to = apart.move_off(
            Nook::Users,
            Drawn::bare(&buf),
            plan_on(&playlist, Rect::default(), buf.area),
            &|_, _| false,
        );
        assert_eq!(to, Some(Strip::Bottom(Nook::Playlist)), "with no chat");
        assert!(tv_in_chat(&apart), "with no chat, pushed where it comes");
        let mut kept = home.clone();
        let to = kept.move_off(
            Nook::Users,
            Drawn::bare(&buf),
            plan_on(&playlist, chat, buf.area),
            &|_, _| false,
        );
        assert_eq!(to, None, "{:?}", kept.laid_and_shifted(&playlist).shown);
        assert_eq!(kept, home);
    }

    /// [`no_placement_meets_the_keep`]'s rarest clause, pinned (found at
    /// 16384 cases with the clause off): her sofa, poster, bed, bookshelf
    /// and fridge from Users' left wall, her door on its right, the chat
    /// over its floor. A lamp delivered, boxed or out of its box, never
    /// makes her door's kept space yield (D3: only a transition is
    /// refused; an older crowded home still gets parcels).
    #[test]
    fn a_lamp_delivered_never_makes_her_kept_door_space_yield() {
        let (buf, nooks) = two_panes(36, 12, &[]);
        let chat = Rect::new(4, 10, 27, 2).intersection(buf.area);
        let plan = plan_on(&nooks, chat, buf.area);
        let left = Some(Anchor {
            side: Side::Left,
            offset: 0,
        });
        let mut room = Home {
            props: [
                Furniture::Sofa,
                Furniture::Poster,
                Furniture::Bed,
                Furniture::Bookshelf,
                Furniture::Fridge,
            ]
            .into_iter()
            .map(|item| Prop {
                anchor: left,
                ..prop(item, 0)
            })
            .collect(),
            door: door_on(Nook::Users, Side::Right),
            structures: Vec::new(),
        };
        let (shown, _) = room.frame(Drawn::bare(&buf), plan, &|_, _| false);
        assert_eq!(kept_on(&room, plan), Some(true), "kept before");
        let (parcel, _) = room
            .doorstep(
                Drawn::bare(&buf),
                plan,
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Lamp,
            )
            .expect("it comes in by another wall");
        for boxed in [true, false] {
            let mut with = room.clone();
            assert!(with.add(Prop {
                boxed: boxed.into(),
                ..parcel
            }));
            assert_eq!(
                kept_on(&with, plan),
                Some(true),
                "boxed {boxed}: {parcel:?}"
            );
        }
    }

    /// A piece in the chat with no gap anywhere wide enough for it is
    /// still taken by another room when that room's pieces would pack
    /// round it (door batch, D7: "where it packs"): Playlist's free
    /// columns split 7 and 8, a 10-wide bed, so it's not in the closet.
    #[test]
    fn a_piece_the_other_room_packs_round_is_never_stranded() {
        let (buf, nooks) = two_panes(30, 12, &[]);
        let at = |item, nook, side, offset| Prop {
            strip: Strip::Bottom(nook),
            anchor: Some(Anchor { side, offset }),
            ..Prop::new(item, nook, 0, Facing::Right)
        };
        let home = Home {
            props: vec![
                at(Furniture::Bed, Nook::Users, Side::Left, 4),
                at(Furniture::Sofa, Nook::Playlist, Side::Left, 0),
                at(Furniture::Tv, Nook::Playlist, Side::Left, 16),
                at(Furniture::Desk, Nook::Playlist, Side::Right, 1),
            ],
            door: None,
            structures: Vec::new(),
        };
        let mut laid_home = home.clone();
        let laid = laid_home.laid_out(&nooks);
        let e = laid.plan(Strip::Bottom(Nook::Playlist)).unwrap().floor;
        let mut free: Vec<i32> = Vec::new();
        let mut run = 0;
        for x in e.from..e.to {
            let taken = laid.shown.iter().any(|s| {
                s.strip == Some(Strip::Bottom(Nook::Playlist))
                    && (s.left..s.left + i32::from(s.size().0)).contains(&x)
            });
            if taken {
                if run > 0 {
                    free.push(run);
                }
                run = 0;
            } else {
                run += 1;
            }
        }
        if run > 0 {
            free.push(run);
        }
        // Precondition: room enough in all, no gap wide enough.
        assert!(free.iter().sum::<i32>() >= 10, "{free:?}");
        assert!(free.iter().all(|&g| g < 10), "{free:?}");
        let chat = nooks[0].1;
        let bed = laid
            .shown
            .iter()
            .position(|s| s.item == Furniture::Bed)
            .unwrap();
        assert!(in_chat(chat, laid.shown[bed].cover()));
        let places = placements(&home, &laid, chat);
        assert!(
            matches!(places.as_slice(), [(i, Some((Strip::Bottom(Nook::Playlist), _)))] if *i == bed),
            "{places:?} (free {free:?})"
        );
        // And a frame shows it, where it stands.
        let plan = plan_on(&nooks, chat, buf.area);
        let mut framed = home.clone();
        let (shown, _) = framed.frame(Drawn::bare(&buf), plan, &|_, _| false);
        assert!(shown.iter().any(|s| s.item == Furniture::Bed), "{shown:?}");
    }

    /// Whether a repair could move `item` (one of `home`'s pieces, in
    /// the chat pane `chat`) out of the chat, as a brute force over every
    /// strip of `nooks` and every anchor along it: the piece moved there,
    /// her home laid out again for real; every piece still laid, the
    /// piece clear of the chat and her door's space (as it would be,
    /// before or after), none of the others newly pushed into them, and
    /// a door space that was kept still kept. The first such strip and
    /// anchor.
    fn a_repair_could_move(
        home: &Home,
        nooks: &[(Nook, Rect)],
        chat: Rect,
        item: Furniture,
    ) -> Option<(Strip, Anchor)> {
        let before = home.clone().laid_out(nooks);
        let kept = |laid: &LaidOut| {
            laid.plans
                .iter()
                .find_map(|p| p.space)
                .is_some_and(|s| s.kept)
        };
        let cells =
            |r: Rect| (r.x..r.right()).flat_map(move |x| (r.y..r.bottom()).map(move |y| (x, y)));
        let meets = |a: Rect, b: Rect| {
            let b: Vec<(u16, u16)> = cells(b).collect();
            cells(a).any(|c| b.contains(&c))
        };
        for plan in &before.plans {
            for side in [Side::Left, Side::Right] {
                for offset in 0..=(plan.raw.to - plan.raw.from).max(0) {
                    let mut moved = home.clone();
                    let p = moved.props.iter_mut().find(|p| p.item == item)?;
                    p.strip = plan.strip;
                    p.anchor = Some(Anchor {
                        side,
                        offset: offset as u16,
                    });
                    let after = moved.clone().laid_out(nooks);
                    if after.shown.len() != before.shown.len() || (kept(&before) && !kept(&after)) {
                        continue;
                    }
                    let spaces: Vec<Rect> = before
                        .plans
                        .iter()
                        .chain(&after.plans)
                        .filter_map(|p| p.space.map(|s| s.rect))
                        .collect();
                    let keeps = |r: Rect| {
                        (!chat.is_empty() && meets(r, chat)) || spaces.iter().any(|&s| meets(r, s))
                    };
                    let Some(at) = after.shown.iter().find(|s| s.item == item) else {
                        continue;
                    };
                    if keeps(at.cover()) {
                        continue;
                    }
                    let pushed = before.shown.iter().filter(|b| b.item != item).any(|b| {
                        after
                            .shown
                            .iter()
                            .find(|a| a.item == b.item)
                            .is_none_or(|a| {
                                a.cover() != b.cover() && keeps(a.cover()) && !keeps(b.cover())
                            })
                    });
                    if !pushed {
                        return Some((
                            plan.strip,
                            Anchor {
                                side,
                                offset: offset as u16,
                            },
                        ));
                    }
                }
            }
        }
        None
    }

    /// The body of the closet's properties (door batch, D7; T25):
    /// `props` on Users (or Playlist, where `playlist` says so and both
    /// panes are here), her door where `door` says, the chat at `chat`.
    /// Of her pieces in the chat pane, each [`placements`] assigns goes
    /// somewhere its strip's lane holds and clear of the chat and her
    /// door's space (cell by cell); with one in the chat, it's stranded
    /// only if no repair could move it ([`a_repair_could_move`]: a piece
    /// some room could take is never hidden). A box is never weighed. And
    /// a frame hides just those stranded: none it shows, every other it
    /// would show. How many it strands.
    fn closet_case(
        props: Vec<Prop>,
        wide: u16,
        playlist: &[bool],
        both: bool,
        chat: Rect,
        door: Option<(bool, bool)>,
    ) -> Result<usize, TestCaseError> {
        closet_case_on(props, wide, playlist, both, chat, door, Vec::new())
    }

    /// [`closet_case`] with her `structures` up (phase 6a, step 3): a
    /// piece of `props` on a shelf stays on its pane's shelf (Users', or
    /// the Playlist's where `playlist` says), and none on a shelf is
    /// weighed ([`closetable`]).
    pub(super) fn closet_case_on(
        props: Vec<Prop>,
        wide: u16,
        playlist: &[bool],
        both: bool,
        chat: Rect,
        door: Option<(bool, bool)>,
        structures: Vec<Structure>,
    ) -> Result<usize, TestCaseError> {
        let (buf, panes) = two_panes(wide, 12, &[]);
        let nooks: Vec<(Nook, Rect)> = if both { panes.to_vec() } else { vec![panes[0]] };
        let mut home = Home {
            structures,
            door: door.and_then(|(users, right)| {
                door_on(
                    if users || !both {
                        Nook::Users
                    } else {
                        Nook::Playlist
                    },
                    if right { Side::Right } else { Side::Left },
                )
            }),
            ..Home::default()
        };
        for (i, p) in props.into_iter().enumerate() {
            let nook = if both && playlist.get(i).copied().unwrap_or(false) {
                Nook::Playlist
            } else {
                Nook::Users
            };
            let strip = if p.strip.is_floor() {
                Strip::Bottom(nook)
            } else {
                Strip::Shelf(nook)
            };
            let p = Prop { strip, ..p };
            prop_assert!(home.add(p), "{:?}", p);
        }
        let laid = home.laid_out(&nooks);
        let inside = |s: &Shown| {
            s.strip.is_some_and(Strip::is_floor) && s.scrap.is_none() && in_chat(chat, s.cover())
        };
        let places = placements(&home, &laid, chat);
        let weighed: Vec<usize> = (0..laid.shown.len())
            .filter(|&i| inside(&laid.shown[i]) && !laid.shown[i].boxed())
            .collect();
        prop_assert_eq!(
            places.iter().map(|&(i, _)| i).collect::<Vec<_>>(),
            weighed.clone()
        );
        let mut strands = 0;
        for &(i, to) in &places {
            let piece = laid.shown[i];
            match to {
                Some((strip, left)) => {
                    let plan = laid.plan(strip).unwrap();
                    // Laned and lifted as it would be there (a candidate
                    // strip's lane, not its own: phase 6a D1), and where
                    // she'd stand to reach it read from the plan.
                    prop_assert!(
                        plan.takes(piece.item, piece.boxed()),
                        "{:?} to {:?}",
                        piece,
                        strip
                    );
                    let e = plan.along(piece.lane_on(strip));
                    let (cols, rows) = piece.size();
                    let at = at_left(&piece, plan, left);
                    prop_assert_eq!(at.stood, plan.host.floor);
                    prop_assert!(
                        left >= e.from
                            && left + i32::from(cols) <= e.to
                            && e.rows >= rows + piece.lift_on(strip)
                    );
                    prop_assert!(!in_chat(chat, at.cover()), "{:?} to {:?}", piece, at);
                    prop_assert!(
                        plan.space.is_none_or(|s| !s.rect.intersects(at.cover())),
                        "{:?} to {:?}",
                        piece,
                        at
                    );
                }
                None => {
                    strands += 1;
                    if weighed.len() == 1 {
                        let could = a_repair_could_move(&home, &nooks, chat, piece.item);
                        prop_assert!(
                            could.is_none(),
                            "{:?} stranded, but a repair could move it: {:?}",
                            piece,
                            could
                        );
                    }
                }
            }
        }
        // A frame hides just the stranded.
        let plan = plan_on(&nooks, chat, buf.area);
        let mut framed = home.clone();
        let (shown, _) = framed.frame(Drawn::bare(&buf), plan, &|_, _| false);
        let mut projected = home.clone();
        projected.settle_door(plan);
        let (all, _) = projected.project_with(Drawn::bare(&buf), plan, &|_, _| false);
        let laid = projected.laid_and_shifted(&nooks);
        let gone: Vec<Furniture> = stranded(&projected, &laid, chat)
            .iter()
            .map(|&i| laid.shown[i].item)
            .collect();
        let expected: Vec<Shown> = all
            .into_iter()
            .filter(|s| !gone.contains(&s.item))
            .collect();
        prop_assert_eq!(shown, expected);
        Ok(strands)
    }

    /// The pieces of a closet case that strand most often: some boxed,
    /// three wide floor pieces and a lamp.
    fn crowded() -> impl Strategy<Value = Vec<Prop>> {
        (
            proptest::sample::subsequence(
                vec![
                    Furniture::Bed,
                    Furniture::Sofa,
                    Furniture::Desk,
                    Furniture::Tv,
                    Furniture::Lamp,
                ],
                2..=5,
            ),
            proptest::collection::vec(any::<bool>(), 5),
        )
            .prop_flat_map(|(items, boxed)| {
                placed(items).prop_map(move |props| {
                    props
                        .into_iter()
                        .zip(&boxed)
                        .map(|(p, &b)| Prop {
                            boxed: (b && p.item != Furniture::Lamp).into(),
                            ..p
                        })
                        .collect()
                })
            })
    }

    /// The closet puts small decor up on her shelf (phase 6a D6; the
    /// deterministic companion of `stranded_iff_no_room_takes_it_with_a_
    /// shelf`, where it's rare): Users and the Playlist 40 wide and 12
    /// tall, a chat over both from row 6 (every floor in it), her shelf up
    /// in Users (run `13..23` on row 4, clear of the chat), her plant on
    /// Users' floor. The one place for it is up on the shelf: the closet
    /// assigns it there, within the run, so it's not stranded, and a
    /// repair could move it there (it stands alone). With no shelf, it's
    /// stranded, and no repair could move it. (Pure layout: no drawing
    /// mode.)
    #[test]
    fn the_closet_puts_small_decor_up_on_her_shelf() {
        use super::shelves::{on, shelf_on};
        let (_, panes) = two_panes(40, 12, &[]);
        let nooks = panes.to_vec();
        let chat = Rect::new(0, 6, 80, 6);
        for shelved in [true, false] {
            let mut home = Home {
                structures: if shelved {
                    vec![shelf_on(Nook::Users, 12, 7, 2)]
                } else {
                    Vec::new()
                },
                ..Home::default()
            };
            assert!(home.add(on(Furniture::Plant, Strip::Bottom(Nook::Users), 20)));
            let laid = home.laid_out(&nooks);
            let places = placements(&home, &laid, chat);
            let plant = laid.shown.iter().position(|s| s.item == Furniture::Plant);
            assert_eq!(
                places.iter().map(|&(i, _)| Some(i)).collect::<Vec<_>>(),
                [plant]
            );
            let could = a_repair_could_move(&home, &nooks, chat, Furniture::Plant);
            if shelved {
                let Some((strip, left)) = places[0].1 else {
                    panic!("her plant stranded: {places:?}");
                };
                assert_eq!(strip, Strip::Shelf(Nook::Users));
                assert!((13..=20).contains(&left), "{left}");
                assert!(stranded(&home, &laid, chat).is_empty());
                assert_eq!(
                    could.map(|(strip, _)| strip),
                    Some(Strip::Shelf(Nook::Users))
                );
            } else {
                assert_eq!(places[0].1, None);
                assert_eq!(stranded(&home, &laid, chat), [plant.unwrap()]);
                assert_eq!(could, None);
            }
        }
    }

    /// A deterministic companion to the closet's properties: a bed in a
    /// chat over all of Users, Playlist too full to take it, is hidden
    /// by a frame (and only it), and back when the chat moves off.
    #[test]
    fn a_frame_hides_a_piece_no_room_takes() {
        let (buf, nooks) = two_panes(30, 12, &[]);
        let at = |item, nook, side, offset| Prop {
            strip: Strip::Bottom(nook),
            anchor: Some(Anchor { side, offset }),
            ..Prop::new(item, nook, 0, Facing::Right)
        };
        let home = Home {
            props: vec![
                at(Furniture::Bed, Nook::Users, Side::Left, 4),
                at(Furniture::Sofa, Nook::Playlist, Side::Left, 0),
                at(Furniture::Tv, Nook::Playlist, Side::Left, 10),
                at(Furniture::Desk, Nook::Playlist, Side::Right, 1),
                at(Furniture::Fridge, Nook::Playlist, Side::Right, 9),
                at(Furniture::Bookshelf, Nook::Playlist, Side::Left, 18),
            ],
            door: None,
            structures: Vec::new(),
        };
        let chat = nooks[0].1;
        let laid = home.clone().laid_out(&nooks);
        assert_eq!(laid.shown.len(), 6, "{:?}", laid.shown);
        let bed = laid
            .shown
            .iter()
            .position(|s| s.item == Furniture::Bed)
            .unwrap();
        assert_eq!(stranded(&home, &laid, chat), vec![bed]);
        assert_eq!(
            a_repair_could_move(&home, &nooks, chat, Furniture::Bed),
            None
        );
        let mut framed = home.clone();
        let (shown, _) = framed.frame(
            Drawn::bare(&buf),
            plan_on(&nooks, chat, buf.area),
            &|_, _| false,
        );
        let items: Vec<Furniture> = shown.iter().map(|s| s.item).collect();
        assert!(!items.contains(&Furniture::Bed), "{items:?}");
        assert_eq!(items.len(), 5, "{items:?}");
        assert_eq!(framed.props, home.props, "never moved");
        let (shown, _) = framed.frame(
            Drawn::bare(&buf),
            plan_on(&nooks, Rect::default(), buf.area),
            &|_, _| false,
        );
        assert!(shown.iter().any(|s| s.item == Furniture::Bed));
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(256)))]

        /// The closet, over any chat (see [`closet_case`]).
        #[test]
        fn stranded_iff_no_room_takes_it(
            props in decorated(),
            wide in 24u16..60,
            playlist in proptest::collection::vec(any::<bool>(), 6),
            both in any::<bool>(),
            chat in (0u16..100, 0u16..12, 1u16..40, 1u16..13),
            door in proptest::option::of((any::<bool>(), any::<bool>())),
        ) {
            let chat = Rect::new(chat.0, chat.1, chat.2, chat.3);
            closet_case(props, wide, &playlist, both, chat, door)?;
        }

        /// The closet where it strands most (see [`closet_case`]): wide
        /// pieces, some boxed, under a chat over most of Users' floor.
        #[test]
        fn stranded_iff_no_room_takes_it_with_the_chat_over_her_floor(
            props in crowded(),
            wide in 24u16..44,
            playlist in proptest::collection::vec(any::<bool>(), 6),
            both in any::<bool>(),
            chat in (0u16..8, 4u16..10, 16u16..48),
            door in proptest::option::of((any::<bool>(), any::<bool>())),
        ) {
            let chat = Rect::new(chat.0, chat.1, chat.2, 12 - chat.1);
            // It strands about 23 pieces in 43 cases at 32 (measured
            // 2026-10-09; the plain one, 2 in 43).
            closet_case(props, wide, &playlist, both, chat, door)?;
        }
    }
}

/// Shelves as strips (phase 6a, step 3): a finished shelf is a strip
/// after the panes; what stands on it stands only on its written cells;
/// the floors-only paths never take one; the closet and the chat rule
/// never weigh what's on one. (Pure layout and search: no drawing mode
/// to loop over.)
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
pub(in crate::ui::houseguest) mod shelves {
    use super::super::structures::{KeepOff, Shelf, Structure, read_ground};
    use super::*;
    use proptest::prelude::*;
    use tuirealm::ratatui::style::Style;
    use tuirealm::ratatui::widgets::{Block, Widget};

    /// A 100×30 frame of three bordered panes, as the tests' `rooms`:
    /// List `0, 0, 50, 27`, Users `50, 0, 50, 13`, Playlist `50, 13, 50,
    /// 14`.
    pub(in crate::ui::houseguest) fn three_panes() -> (Buffer, Vec<(Nook, Rect)>) {
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        let nooks = vec![
            (Nook::List, Rect::new(0, 0, 50, 27)),
            (Nook::Users, Rect::new(50, 0, 50, 13)),
            (Nook::Playlist, Rect::new(50, 13, 50, 14)),
        ];
        for &(_, pane) in &nooks {
            Block::bordered().render(pane, &mut buf);
        }
        (buf, nooks)
    }

    /// Her shelf in Users, `Left` `offset`, `rise` rows up, `built`.
    pub(in crate::ui::houseguest) fn shelf_on(
        nook: Nook,
        offset: u16,
        rise: u16,
        built: u8,
    ) -> Structure {
        Structure::Shelf(Shelf {
            nook,
            anchor: Anchor {
                side: Side::Left,
                offset,
            },
            rise,
            built,
            fresh: None,
        })
    }

    /// `item` anchored on `strip`, `Left` `offset`.
    pub(in crate::ui::houseguest) fn on(item: Furniture, strip: Strip, offset: u16) -> Prop {
        Prop {
            strip,
            anchor: Some(Anchor {
                side: Side::Left,
                offset,
            }),
            ..Prop::new(item, strip.nook(), 0, Facing::Right)
        }
    }

    /// Write `home`'s shelves into `buf` as a paint does (nothing kept
    /// off), returning what was written.
    pub(in crate::ui::houseguest) fn written(
        buf: &mut Buffer,
        home: &Home,
        plan: Plan,
    ) -> super::super::structures::Written {
        let keep_off = KeepOff {
            protected: &[],
            layer: &[],
            made: &[],
        };
        read_ground(buf, home, plan, &keep_off)
    }

    /// A finished shelf whose geometry holds is a strip after the panes
    /// (phase 6a D6), its run its raw extent and floor (no door space on
    /// a shelf), its pane's floor its host, the rows over it inside the
    /// pane its rows: worked out from the record and the pane's rect
    /// alone. One not finished, or whose geometry doesn't hold, is none.
    /// Both sides, at and past the edges.
    #[test]
    fn a_finished_shelf_is_a_strip() {
        let (_, nooks) = three_panes();
        // Users: inside columns 51..99, floor 12, 11 rows.
        for side in [Side::Left, Side::Right] {
            for offset in [7u16, 8, 30, 31] {
                for rise in [4u16, 5, 9, 10] {
                    for built in 0..=2u8 {
                        let at = format!("{side:?} {offset} {rise} {built}");
                        let shelf = Shelf {
                            nook: Nook::Users,
                            anchor: Anchor { side, offset },
                            rise,
                            built,
                            fresh: None,
                        };
                        let home = Home {
                            structures: vec![Structure::Shelf(shelf)],
                            ..Home::default()
                        };
                        let plans = home.extents(&nooks);
                        let panes: Vec<Strip> = plans.iter().take(3).map(|p| p.strip).collect();
                        assert_eq!(
                            panes,
                            [Nook::List, Nook::Users, Nook::Playlist].map(Strip::Bottom),
                            "{at}"
                        );
                        let from = match side {
                            Side::Left => 51 + i32::from(offset),
                            Side::Right => 99 - i32::from(offset) - 10,
                        };
                        let holds = from >= 59 && from + 10 <= 91 && rise >= 5 && 11 - rise >= 2;
                        if built == 2 && holds {
                            let host = extent_of(51, 99, 12, 11);
                            let run = extent_of(from, from + 10, 12 - i32::from(rise), 11 - rise);
                            assert_eq!(
                                plans.get(3),
                                Some(&StripPlan {
                                    strip: Strip::Shelf(Nook::Users),
                                    raw: run,
                                    floor: run,
                                    space: None,
                                    host,
                                    block: None,
                                }),
                                "{at}"
                            );
                            assert_eq!(plans.len(), 4, "{at}");
                        } else {
                            assert_eq!(plans.len(), 3, "{at}: {plans:?}");
                        }
                    }
                }
            }
        }
    }

    fn extent_of(from: i32, to: i32, floor: i32, rows: u16) -> Extent {
        Extent {
            from,
            to,
            floor,
            rows,
        }
    }

    /// A piece on a shelf stands only on the cells her shelf was written
    /// in this frame (phase 6a D6): written, the plant shows there (its
    /// floor the run's row, reached from the floor below); with the
    /// app's own `────` across the run's row instead (the shelf hidden by
    /// it, nothing written), it holds nothing up and the plant is hidden,
    /// never moved. And a piece on no shelf never stands on a written
    /// cell: a makeshift piece whose floor row is the run stands on the
    /// app's line there, not on her shelf.
    #[test]
    fn a_piece_stands_on_its_shelf_only_where_it_was_written() {
        let (real, nooks) = three_panes();
        let plan = Plan::bare(&nooks);
        let shelf = Strip::Shelf(Nook::Users);
        let mut home = Home {
            structures: vec![shelf_on(Nook::Users, 12, 7, 2)],
            ..Home::default()
        };
        assert!(home.add(on(Furniture::Plant, shelf, 3)));
        let record = home.clone();
        // Written: the run `63..73` on row 5.
        let mut buf = real.clone();
        let mine = written(&mut buf, &home, plan);
        assert!((63..73).all(|x| mine.contains(x, 5)));
        let (shown, _) = home.frame(mine.over(&buf), plan, &|_, _| false);
        let plant = shown.iter().find(|s| s.item == Furniture::Plant);
        assert_eq!(
            plant.map(|s| (s.strip, s.left, s.floor, s.stood)),
            Some((Some(shelf), 66, 5, 12))
        );
        // The app's own line across the row, nothing written.
        let mut lined = real.clone();
        lined.set_string(55, 5, "─".repeat(40), Style::new());
        let (shown, _) = home.frame(Drawn::bare(&lined), plan, &|_, _| false);
        assert!(
            !shown.iter().any(|s| s.item == Furniture::Plant),
            "{shown:?}"
        );
        assert_eq!(home.props, record.props, "never moved");
        // A makeshift piece on the run's row: on her shelf, no; on the
        // app's line there, yes.
        let made = Shown {
            item: Furniture::Sofa,
            facing: Facing::Right,
            boxed: false.into(),
            strip: None,
            left: 64,
            floor: 5,
            stood: 5,
            scrap: Some(super::super::scrap::Scrap::new(MadeId(0), &[], 0)),
        };
        let clear = |_: i32, _: i32| true;
        assert!(!fits(mine.over(&buf), &made, &clear), "on her shelf");
        let mut over = buf.clone();
        mine.undo(&mut over);
        over.set_string(60, 5, "─".repeat(20), Style::new());
        assert!(fits(Drawn::bare(&over), &made, &clear), "on a line");
    }

    /// An older record's share of the way along is pinned on a shelf
    /// where it says ([`Home::pin_anchors`]), of its host, as
    /// [`StripPlan::pin`] writes it: a plant with only a share on the
    /// Users shelf comes to stand within a column of where the share was
    /// taken, anywhere along the run.
    #[test]
    fn a_share_on_a_shelf_is_pinned_where_it_says() {
        let (_, nooks) = three_panes();
        let shelf = Strip::Shelf(Nook::Users);
        let base = Home {
            structures: vec![shelf_on(Nook::Users, 12, 7, 2)],
            ..Home::default()
        };
        let plan = *base
            .extents(&nooks)
            .iter()
            .find(|p| p.strip == shelf)
            .expect("the shelf's plan");
        let cols = Furniture::Plant.spec().footprint.0;
        for left in plan.raw.from..=plan.raw.to - i32::from(cols) {
            let (_, share) = plan.pin(Lane::Floor, left, cols);
            let mut home = base.clone();
            assert!(home.add(Prop {
                strip: shelf,
                anchor: None,
                ..Prop::new(Furniture::Plant, Nook::Users, share, Facing::Right)
            }));
            let laid = home.laid_out(&nooks);
            let plant = laid.shown.iter().find(|s| s.item == Furniture::Plant);
            let got = plant.map(|s| (s.strip, s.left));
            assert!(
                got.is_some_and(|(strip, at)| strip == Some(shelf) && (at - left).abs() <= 1),
                "{left}: {got:?}"
            );
            assert!(home.props[0].anchor.is_some(), "{left}: pinned");
        }
    }

    /// What stands on a shelf is never weighed for the chat (phase 6a
    /// D6): a shelf the chat comes to meet is hidden (its pieces with
    /// it), so neither the closet ([`closetable`]) nor the chat rule
    /// judges what's on one. Its twin on the floor below is.
    #[test]
    fn a_shelf_piece_is_never_weighed_for_the_chat() {
        let users = extent_of(51, 99, 12, 11);
        let run = extent_of(63, 73, 5, 4);
        let shelf = Strip::Shelf(Nook::Users);
        let up = stand(&on(Furniture::Plant, shelf, 3), shelf, run, users, 66);
        let floor = Strip::Bottom(Nook::Users);
        let down = stand(&on(Furniture::Plant, floor, 15), floor, users, users, 66);
        let chat = Rect::new(50, 0, 50, 13);
        assert!(in_chat(chat, up.cover()) && in_chat(chat, down.cover()));
        assert!(!closetable(chat, &up));
        assert!(closetable(chat, &down));
    }

    /// A parcel never stands on a shelf (phase 6a D2: one comes in only
    /// on a floor, where she unpacks it): the layout, which every search
    /// reads, lays a boxed piece anchored on a shelf nowhere, and a
    /// shelf is never a place for one in the closet; out of its box, the
    /// same piece stands there.
    #[test]
    fn a_parcel_never_stands_on_a_shelf() {
        let (_, nooks) = three_panes();
        let shelf = Strip::Shelf(Nook::Users);
        for boxed in [false, true] {
            let mut home = Home {
                structures: vec![shelf_on(Nook::Users, 12, 7, 2)],
                ..Home::default()
            };
            assert!(home.add(Prop {
                boxed: boxed.into(),
                ..on(Furniture::Plant, shelf, 3)
            }));
            let laid = home.laid_out(&nooks);
            let up = laid.shown.iter().any(|s| s.strip == Some(shelf));
            assert_eq!(up, !boxed, "boxed={boxed}: {laid:?}");
            assert_eq!(
                home.on(shelf, Lane::Floor).is_empty(),
                boxed,
                "boxed={boxed}"
            );
        }
    }

    /// What she does with small decor up on her shelf (phase 6a D8): she
    /// gazes up at her plant and her gremlin there (D9; from its room's
    /// floor: the seat is the pane's floor, `stood`); at nothing else of
    /// hers up there (her clock, a mantel clock, she glances up at for
    /// the time, as on her wall), and at none of them on a floor; boxed,
    /// a parcel is to unpack. Laid out as her home lays them, each small-decor kind on
    /// her Users shelf and on Users' floor.
    #[test]
    fn she_gazes_up_at_her_plant_on_her_shelf() {
        let (_, nooks) = three_panes();
        let shelf = Strip::Shelf(Nook::Users);
        for item in Furniture::ALL.into_iter().filter(|f| f.spec().shelf) {
            for strip in [shelf, Strip::Bottom(Nook::Users)] {
                let at = format!("{item:?} on {strip:?}");
                let mut home = Home {
                    structures: vec![shelf_on(Nook::Users, 12, 7, 2)],
                    ..Home::default()
                };
                assert!(home.add(on(item, strip, 3)), "{at}");
                let laid = home.laid_out(&nooks);
                let s = *laid
                    .shown
                    .iter()
                    .find(|s| s.item == item)
                    .unwrap_or_else(|| panic!("{at}: laid out"));
                assert_eq!(s.strip, Some(strip), "{at}");
                let gazes = matches!(item, Furniture::Plant | Furniture::Gremlin) && strip == shelf;
                let uses: &[Use] = if gazes {
                    &[Use::GazeUp]
                } else {
                    item.spec().uses
                };
                assert_eq!(s.uses(), uses, "{at}");
                assert_eq!(s.stood, 12, "{at}: reached from Users' floor");
                let boxed = Shown {
                    boxed: true.into(),
                    ..s
                };
                assert_eq!(boxed.uses(), [Use::Unpack], "{at}: boxed");
            }
        }
    }

    /// [`three_panes`] with a line of text just over Users' and the
    /// Playlist's floors (row 11 and row 25, wall to wall: no piece
    /// stands on either floor), List's floor clear, and her finished
    /// shelf in Users (run `63..73` on row 5) the one other run free.
    fn floors_taken() -> (Buffer, Vec<(Nook, Rect)>) {
        let (mut buf, nooks) = three_panes();
        buf.set_string(51, 11, "x".repeat(48), Style::new());
        buf.set_string(51, 25, "x".repeat(48), Style::new());
        (buf, nooks)
    }

    fn shelved(props: &[Prop]) -> Home {
        let mut home = Home {
            structures: vec![shelf_on(Nook::Users, 12, 7, 2)],
            ..Home::default()
        };
        for &p in props {
            assert!(home.add(p), "{p:?}");
        }
        home
    }

    /// Her seats at a piece as if she could stand anywhere.
    fn anywhere(piece: &Shown, _: &[Shown], _: &Home) -> Vec<Seat> {
        piece
            .uses()
            .iter()
            .map(|&what| piece.seat(what, piece.beside()[0]))
            .collect()
    }

    /// Her pieces moving off a pane that's gone go to another pane's
    /// floor, never onto a shelf (phase 6a D1: [`Home::move_off`] takes
    /// [`pane_strips`]), even where the shelf is the one free run: List
    /// hidden, its plant goes nowhere (no floor takes it) and keeps its
    /// place on List's floor. With the Playlist's floor clear, it moves
    /// there.
    #[test]
    fn pieces_moving_off_go_to_a_floor_never_a_shelf() {
        let plant = on(Furniture::Plant, Strip::Bottom(Nook::List), 20);
        for clear in [false, true] {
            let (mut buf, nooks) = floors_taken();
            if clear {
                buf.set_string(51, 25, " ".repeat(48), Style::new());
            }
            let rest = &nooks[1..];
            let plan = Plan::bare(rest);
            let mut home = shelved(&[plant]);
            let mine = written(&mut buf, &home, plan);
            let (_, moves) = home.project_with(mine.over(&buf), plan, &|_, _| false);
            let want = if clear {
                Strip::Bottom(Nook::Playlist)
            } else {
                Strip::Bottom(Nook::List)
            };
            assert_eq!(home.props[0].strip, want, "clear={clear}: {moves:?}");
            assert!(
                moves.iter().all(|&(_, to)| to.is_floor()),
                "clear={clear}: {moves:?}"
            );
            mine.keep(&mut Vec::new());
        }
    }

    /// A shelf whose pane is gone hides what's on it, never moves it
    /// (phase 6a D1: [`Home::project_with`] moves only a pane's floor):
    /// Users hidden, its sofa moves to a floor, its plant on the shelf
    /// stays there (anchored as it was), shown nowhere; Users back, it
    /// shows on its shelf again.
    #[test]
    fn a_shelf_gone_with_its_pane_hides_what_is_on_it() {
        let (real, nooks) = three_panes();
        let shelf = Strip::Shelf(Nook::Users);
        let plant = on(Furniture::Plant, shelf, 3);
        let sofa = on(Furniture::Sofa, Strip::Bottom(Nook::Users), 20);
        let mut home = shelved(&[sofa, plant]);
        let hidden = [nooks[0], nooks[2]];
        let (shown, moves) =
            home.project_with(Drawn::bare(&real), Plan::bare(&hidden), &|_, _| false);
        assert_eq!(moves.len(), 1, "{moves:?}");
        assert!(moves[0].1.is_floor(), "{moves:?}");
        assert!(
            !shown.iter().any(|s| s.item == Furniture::Plant),
            "{shown:?}"
        );
        assert_eq!(home.props[1], plant, "never moved");
        let mut buf = real.clone();
        let plan = Plan::bare(&nooks);
        let mine = written(&mut buf, &home, plan);
        let (shown, _) = home.project_with(mine.over(&buf), plan, &|_, _| false);
        let up = shown.iter().find(|s| s.item == Furniture::Plant);
        assert_eq!(up.map(|s| (s.strip, s.left)), Some((Some(shelf), 66)));
        mine.keep(&mut Vec::new());
    }

    /// A delivery comes in on a floor, never on a shelf, through a pane's
    /// wall (phase 6a D1: [`Home::doorstep`] takes [`pane_strips`]); and
    /// a stage gift lands on a floor ([`Home::spot`]): with every floor
    /// taken but List's, a plant comes in on List and is given there;
    /// with List's taken too (its own line of text), neither has
    /// anywhere, her shelf the one free run.
    #[test]
    fn a_delivery_and_a_gift_come_to_a_floor_never_a_shelf() {
        for list in [true, false] {
            let (mut buf, nooks) = floors_taken();
            if !list {
                buf.set_string(1, 25, "x".repeat(48), Style::new());
            }
            let plan = Plan::bare(&nooks);
            let home = shelved(&[on(Furniture::Sofa, Strip::Bottom(Nook::List), 30)]);
            let mine = written(&mut buf, &home, plan);
            let mut framed = home.clone();
            let (shown, _) = framed.frame(mine.over(&buf), plan, &|_, _| false);
            let came = home.doorstep(
                mine.over(&buf),
                plan,
                &shown,
                &|_, _| false,
                &anywhere,
                Furniture::Plant,
            );
            let given = home.spot(
                mine.over(&buf),
                plan,
                &shown,
                &|_, _| false,
                Furniture::Plant,
                &mut Rng(3),
            );
            if list {
                let (prop, flap) = came.expect("a delivery");
                assert_eq!(prop.strip, Strip::Bottom(Nook::List));
                assert!(flap.x == 0 || flap.x == 49, "{flap:?}: a pane's wall");
                assert_eq!(given.map(|p| p.strip), Some(Strip::Bottom(Nook::List)));
            } else {
                assert_eq!(came.map(|(p, _)| p.strip), None);
                assert_eq!(given.map(|p| p.strip), None);
            }
            mine.keep(&mut Vec::new());
        }
    }

    /// Her door's wall is a pane's floor's, never a shelf's (phase 6a
    /// D1: [`choose`] takes [`pane_strips`]), and a pane whose only piece
    /// of hers stands on its shelf is hers: of the walls at the screen's
    /// edge (List's left, Users' and the Playlist's right), her plant on
    /// Users' shelf puts her door on Users' right; with it on List's
    /// floor, List's left.
    #[test]
    fn her_door_is_on_a_floor_and_a_shelf_makes_its_pane_hers() {
        let (_, nooks) = floors_taken();
        let plan = Plan::bare(&nooks);
        for (strip, want) in [
            (Strip::Shelf(Nook::Users), Strip::Bottom(Nook::Users)),
            (Strip::Bottom(Nook::List), Strip::Bottom(Nook::List)),
        ] {
            let home = shelved(&[on(Furniture::Plant, strip, 3)]);
            let wall = choose_wall(&home, plan).expect("a wall");
            assert_eq!(wall.strip, want, "{strip:?}");
            assert!(wall.strip.is_floor());
        }
    }

    /// Every kind there is, written out (this module's strategies' own
    /// list: a strategy's domain is never appended to).
    const EVERY_KIND: [Furniture; 12] = [
        Furniture::Sofa,
        Furniture::Tv,
        Furniture::Bed,
        Furniture::Desk,
        Furniture::Lamp,
        Furniture::Bookshelf,
        Furniture::Fridge,
        Furniture::CatBed,
        Furniture::Plant,
        Furniture::Poster,
        Furniture::Clock,
        Furniture::Window,
    ];

    /// Pieces drawn by [`anywhere_up`]: `(kind, pane, shelf, offset,
    /// boxed)`.
    type Up = Vec<(usize, usize, bool, u16, bool)>;

    /// Pieces of any kind, each anchored on a pane's floor or its shelf
    /// (`(kind, pane, shelf, offset, boxed)`).
    fn anywhere_up() -> impl Strategy<Value = Up> {
        proptest::collection::vec(
            (
                0..EVERY_KIND.len(),
                0usize..3,
                any::<bool>(),
                0u16..40,
                proptest::bool::weighted(0.2),
            ),
            1..8,
        )
    }

    /// The body of [`only_small_decor_stands_on_a_shelf`].
    fn small_decor_only(pieces: Up, chat: Rect) -> Result<(usize, usize), TestCaseError> {
        use super::super::door::Keep;
        use super::super::rules::{Frame, broken, search};
        let (mut buf, nooks) = three_panes();
        let plan = Plan {
            nooks: &nooks,
            chat,
            screen: buf.area,
        };
        let mut home = Home {
            structures: vec![
                shelf_on(Nook::List, 12, 7, 2),
                shelf_on(Nook::Users, 12, 7, 2),
                shelf_on(Nook::Playlist, 20, 7, 2),
            ],
            ..Home::default()
        };
        for (kind, pane, up, offset, boxed) in pieces {
            let nook = [Nook::List, Nook::Users, Nook::Playlist][pane];
            let strip = if up {
                Strip::Shelf(nook)
            } else {
                Strip::Bottom(nook)
            };
            let _ = home.add(Prop {
                boxed: boxed.into(),
                ..on(EVERY_KIND[kind], strip, offset)
            });
        }
        let ok = |item: Furniture, boxed: bool| item.spec().shelf && !boxed;
        // Laid out: on a shelf, only small decor, out of its box.
        let laid = home.laid_out(&nooks);
        let mut up = 0;
        for s in laid.shown.iter().filter(|s| s.on_shelf()) {
            prop_assert!(ok(s.item, s.boxed()), "{:?} on a shelf", s);
            up += 1;
        }
        // Searched: no repair puts anything else up there.
        let mine = written(&mut buf, &home, plan);
        let keep = Keep::of(&home, plan);
        let shown = home.frame(mine.over(&buf), plan, &|_, _| false).0;
        let frame = Frame {
            drawn: mine.over(&buf),
            nooks: &nooks,
            blocked: &|_, _| false,
            shown: &shown,
            made: &[],
            keep: &keep,
        };
        let mut moves = 0;
        let mut bad = None;
        for target in broken(&home.laid_out(&nooks), &home, &keep) {
            for r in search(&home, &frame, &target).repairs {
                moves += usize::from(!r.to.strip.is_floor());
                if !r.to.strip.is_floor() && !ok(r.piece, false) {
                    bad = Some(r);
                }
            }
        }
        mine.keep(&mut Vec::new());
        prop_assert!(bad.is_none(), "{:?}", bad);
        // Its record read back: what a shelf doesn't take, boxed or not,
        // stands on that pane's floor; what it takes stays up, on a shelf
        // the record keeps (one in all in 6a: the first, List's).
        let mut ledger = super::super::ledger::Ledger::new(1);
        ledger.home = home.clone();
        let read = super::super::ledger::Ledger::from_json(&ledger.to_json())
            .map_err(TestCaseError::fail)?;
        for p in &home.props {
            let back = read.home.props.iter().find(|q| q.item == p.item);
            let kept = read.home.shelf(p.strip.nook()).is_some();
            let want = if p.strip.is_floor() || (kept && ok(p.item, p.boxed())) {
                p.strip
            } else {
                p.strip.room()
            };
            prop_assert_eq!(back.map(|q| q.strip), Some(want), "{:?}", p);
        }
        Ok((up, moves))
    }

    /// Pieces of every kind laid on her shelves, from the record as from
    /// a search: three high shelves up, pieces of any kind anchored on a
    /// floor or a shelf, some boxed, a chat anywhere, or half the time
    /// over every floor from row 6 down (Users' shelf, on row 5, the one
    /// out of it: what's in the chat goes up there).
    fn shelf_pieces_and_chat() -> impl Strategy<Value = (Up, Rect)> {
        (
            anywhere_up(),
            (0u16..100, 0u16..30, 1u16..60, 1u16..30),
            any::<bool>(),
        )
            .prop_map(|(pieces, (x, y, w, h), band)| {
                let chat = if band {
                    Rect::new(0, 6, 100, 24)
                } else {
                    Rect::new(x, y, w, h)
                };
                (pieces, chat)
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]

        /// Only small decor stands on a shelf (phase 6a D1, D2, D6): of
        /// pieces of every kind anchored on floors and shelves, boxed or
        /// not, the layout lays none on a shelf that a shelf doesn't
        /// take (nor a parcel); no repair puts one up there; and her
        /// record read back puts each on its pane's floor. ("No repair
        /// puts one up" holds through the layout as well as through
        /// the search's own `takes` clause: `Home::on` lays no such
        /// piece there, so `evaluate` refuses it. This doesn't pin that
        /// clause alone.)
        #[test]
        fn only_small_decor_stands_on_a_shelf(case in shelf_pieces_and_chat()) {
            small_decor_only(case.0, case.1)?;
        }

        /// The closet with her shelves up (see [`tests::closet_case_on`]):
        /// small decor on a shelf is never weighed, a shelf is a place
        /// the closet assigns small decor to, and what it strands no
        /// repair could move.
        #[test]
        fn stranded_iff_no_room_takes_it_with_a_shelf(
            props in shelf_decorated(),
            wide in 28u16..60,
            playlist in proptest::collection::vec(any::<bool>(), 6),
            both in any::<bool>(),
            chat in (0u16..100, 0u16..12, 1u16..40, 1u16..13),
            door in proptest::option::of((any::<bool>(), any::<bool>())),
            shelf in (0u16..30, 5u16..=8, 0u16..30, 5u16..=8),
        ) {
            let chat = Rect::new(chat.0, chat.1, chat.2, chat.3);
            let room = wide.saturating_sub(2 + 16 + 10) + 1;
            let shelves = vec![
                shelf_on(Nook::Users, 8 + shelf.0 % room, shelf.1, 2),
                shelf_on(Nook::Playlist, 8 + shelf.2 % 13, shelf.3, 2),
            ];
            tests::closet_case_on(props, wide, &playlist, both, chat, door, shelves)?;
        }
    }

    /// Small decor and furniture for the closet with a shelf: each
    /// anchored on Users' floor (the body moves it by pane), small decor
    /// maybe on the shelf (`Left` up to 7 along), some boxed.
    fn shelf_decorated() -> impl Strategy<Value = Vec<Prop>> {
        const KINDS: [Furniture; 7] = [
            Furniture::Sofa,
            Furniture::Bed,
            Furniture::Lamp,
            Furniture::Plant,
            Furniture::Clock,
            Furniture::Poster,
            Furniture::Window,
        ];
        proptest::sample::subsequence(KINDS.to_vec(), 1..=5).prop_flat_map(|items| {
            let n = items.len();
            (
                Just(items),
                proptest::collection::vec(
                    (
                        any::<bool>(),
                        0u16..40,
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
                        .map(|(item, (right, offset, up, boxed))| {
                            let side = if right { Side::Right } else { Side::Left };
                            if up && item.spec().shelf {
                                Prop {
                                    boxed: boxed.into(),
                                    ..on(item, Strip::Shelf(Nook::Users), offset % 8)
                                }
                            } else {
                                Prop {
                                    boxed: boxed.into(),
                                    anchor: Some(Anchor { side, offset }),
                                    ..Prop::new(item, Nook::Users, 0, Facing::Right)
                                }
                            }
                        })
                        .collect()
                })
        })
    }

    /// [`only_small_decor_stands_on_a_shelf`]'s non-vacuity, over a fixed
    /// run of its strategy: pieces stand on shelves, and repairs put
    /// pieces up there (16 and 9 as the strategy stands; each threshold
    /// about half).
    #[test]
    fn small_decor_goes_up_on_her_shelves() {
        use proptest::strategy::ValueTree;
        use proptest::test_runner::{Config, TestRng, TestRunner};
        let mut runner = TestRunner::new_with_rng(
            Config::default(),
            TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
        );
        let (mut up, mut moves) = (0, 0);
        for _ in 0..64 {
            let (pieces, chat) = shelf_pieces_and_chat()
                .new_tree(&mut runner)
                .unwrap()
                .current();
            let (u, m) = small_decor_only(pieces, chat).unwrap();
            up += u;
            moves += m;
        }
        assert!(up >= 8 && moves >= 4, "{up} up, {moves} moves up");
    }

    /// A stage of her shelf goes up only onto the stage before it, no
    /// further than finished (phase 6a D7c): from nothing up, stage 1 and
    /// then 2; never a stage already up, one ahead, past the last, on a
    /// pane with no shelf of hers, or on a finished shelf. Refused,
    /// nothing changes. (A guard: the paint only puts up the stage
    /// `next_stage` names.)
    #[test]
    fn a_stage_goes_up_only_onto_the_stage_before_it() {
        use super::super::structures::SHELF_STAGES;
        let home = |built| Home {
            structures: vec![shelf_on(Nook::Users, 12, 7, built)],
            ..Home::default()
        };
        let refused = |built: u8, nook: Nook, to: u8| {
            let mut at = home(built);
            assert!(
                !at.put_up(nook, to, Dated::Day(0)),
                "{built} → {nook:?} {to}"
            );
            assert_eq!(at, home(built), "{built} → {nook:?} {to}");
        };
        let mut building = home(0);
        // Up, and fresh that day (phase 6a step 8).
        let fresh = |built| {
            let mut home = home(built);
            home.freshen(Strip::Shelf(Nook::Users), Dated::Day(0));
            home
        };
        assert!(building.put_up(Nook::Users, 1, Dated::Day(0)));
        assert_eq!(building, fresh(1));
        assert!(building.put_up(Nook::Users, 2, Dated::Day(0)));
        assert_eq!(building, fresh(2));
        assert_eq!(SHELF_STAGES, 2);
        for (built, nook, to) in [
            (0, Nook::Users, 0),
            (0, Nook::Users, 2),
            (1, Nook::Users, 1),
            (1, Nook::Users, 3),
            (0, Nook::List, 1),
            (1, Nook::Playlist, 2),
            (2, Nook::Users, 2),
            (2, Nook::Users, 3),
        ] {
            refused(built, nook, to);
        }
        let mut none = Home::default();
        assert!(!none.put_up(Nook::Users, 1, Dated::Day(0)));
        assert_eq!(none, Home::default());
    }

    /// What's set down up on a high shelf of hers goes up high (she hops
    /// to it: phase 6a D7c); on a low one it's in her reach, and on a
    /// floor (a high shelf's pane's too) it's down low; a shelf strip
    /// with no shelf of hers on it is no higher than a floor.
    #[test]
    fn only_a_high_shelf_is_up_high() {
        let home = |rise| Home {
            structures: vec![shelf_on(Nook::Users, 12, rise, 2)],
            ..Home::default()
        };
        let high = home(7);
        assert_eq!(high.height(Strip::Shelf(Nook::Users)), Height::High);
        assert_eq!(high.height(Strip::Bottom(Nook::Users)), Height::Floor);
        assert_eq!(high.height(Strip::Shelf(Nook::List)), Height::Floor);
        assert_eq!(high.height(Strip::Bottom(Nook::List)), Height::Floor);
        let low = home(5);
        assert_eq!(low.height(Strip::Shelf(Nook::Users)), Height::Low);
        assert_eq!(low.height(Strip::Bottom(Nook::Users)), Height::Floor);
    }

    /// Something new up on her shelf (phase 6a step 8): a stage up, or a
    /// piece she set on it, makes it fresh that day of hers; a piece set
    /// on a floor, or on a shelf strip with no shelf of hers, nothing.
    #[test]
    fn what_goes_up_on_her_shelf_is_fresh() {
        let mut home = Home {
            structures: vec![shelf_on(Nook::Users, 12, 7, 0)],
            ..Home::default()
        };
        let fresh = |home: &Home| home.shelf(Nook::Users).and_then(|s| s.fresh);
        home.freshen(Strip::Bottom(Nook::Users), Dated::Day(3));
        home.freshen(Strip::Shelf(Nook::List), Dated::Day(3));
        assert_eq!(fresh(&home), None);
        assert!(home.put_up(Nook::Users, 1, Dated::Day(3)));
        assert_eq!(fresh(&home), Some(Dated::Day(3)));
        assert!(!home.put_up(Nook::Users, 1, Dated::Day(4)), "not again");
        assert_eq!(fresh(&home), Some(Dated::Day(3)));
        home.freshen(Strip::Shelf(Nook::Users), Dated::Visit(9));
        assert_eq!(fresh(&home), Some(Dated::Visit(9)));
    }
}

/// The low shelf (phase 6a, step 4, D3a): a shelf hung under the top of
/// the wall lane blocks its run's columns (and its margin's) across the
/// lane's rows from its own row up, and every piece that hangs keeps out
/// of that block, hanging beside it (or in the closet alone, on a wall
/// too crowded for that), so it never hides her shelf, whatever is built
/// of it. A high shelf, above every hung piece, changes no hung place.
/// The oracles work the shelf out from the record and the pane's rect
/// alone ([`super::tests::shelf_runs`], [`super::tests::ShelfRun::hangs_over`]).
/// Independent of the drawing mode: no loop over it (where a frame is
/// written, `read_ground` writes the same glyphs in both).
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod low_shelf {
    use super::super::structures::{EDGE, HIGH_RISE, LOW_RISE, SHELF_WIDTH, Shelf, Structure};
    use super::super::tests::shelf_runs;
    use super::shelves::written;
    use super::tests::{anywhere, door_on, placed, two_panes};
    use super::*;
    use proptest::prelude::*;
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestCaseError, TestRng, TestRunner};

    /// Her shelf in `nooks`' pane `pane` (Users, the Playlist), against
    /// its `right` wall or left, `along` its room past `EDGE` (wrapped,
    /// so its geometry holds where the pane is wide enough), `rise` rows
    /// up, `built`.
    fn shelf_in(
        nooks: &[(Nook, Rect)],
        (pane, right, along, rise, built): (usize, bool, u16, u16, u8),
    ) -> Structure {
        let (nook, rect) = nooks[pane % nooks.len()];
        let room = i32::from(rect.width) - 2 - 2 * EDGE - SHELF_WIDTH + 1;
        let offset = EDGE + i32::from(along) % room.max(1);
        Structure::Shelf(Shelf {
            nook,
            anchor: Anchor {
                side: if right { Side::Right } else { Side::Left },
                offset: u16::try_from(offset).unwrap(),
            },
            rise,
            built,
            fresh: None,
        })
    }

    /// A shelf's rise: low (`LOW_RISE`, or one row higher, its block
    /// then that one row) or high (`HIGH_RISE` and up to 5 rows more).
    fn a_rise() -> impl Strategy<Value = u16> {
        prop_oneof![Just(LOW_RISE), Just(LOW_RISE + 1), HIGH_RISE..HIGH_RISE + 6]
    }

    /// What a shelf is drawn from (see [`shelf_in`]).
    fn a_shelf() -> impl Strategy<Value = (usize, bool, u16, u16, u8)> {
        (0usize..2, any::<bool>(), 0u16..60, a_rise(), 0u8..=2)
    }

    /// Her pieces for the low shelf's properties (a list of their own:
    /// never appended to): what stands, and every piece that hangs (her
    /// clock among them), each placed as `placed` places them.
    fn hung_among() -> impl Strategy<Value = Vec<Prop>> {
        use Furniture::*;
        const KINDS: [Furniture; 7] = [Sofa, Tv, Lamp, Desk, Poster, Clock, Window];
        proptest::sample::subsequence(KINDS.to_vec(), 1..=6).prop_flat_map(placed)
    }

    /// What [`no_hung_piece_meets_a_low_shelf`] draws: her pieces on
    /// Users, Users `wide × tall` (beside a 40 × 12 Playlist), her shelf
    /// on Users, maybe her door in one of its walls, and maybe a chat
    /// over Users' first columns.
    type LowShelfDraw = (
        Vec<Prop>,
        u16,
        u16,
        (bool, u16, u16, u8),
        Option<bool>,
        Option<u16>,
    );

    fn low_shelf_draw() -> impl Strategy<Value = LowShelfDraw> {
        (
            hung_among(),
            28u16..70,
            9u16..=22,
            (any::<bool>(), 0u16..60, a_rise(), 0u8..=2),
            proptest::option::of(any::<bool>()),
            proptest::option::of(1u16..40),
        )
    }

    /// How often each case of [`no_hung_piece_meets_a_low_shelf`] came
    /// up: shelves written high and low (and of the low, one row above
    /// `LOW_RISE`), pieces a low shelf moved (posters and clocks: only a
    /// block moves those), and hung pieces the closet placed on a wall
    /// with a low shelf.
    #[derive(Debug, Default)]
    struct Seen {
        high: usize,
        low: usize,
        six: usize,
        moved: usize,
        placed: usize,
    }

    /// The body of [`no_hung_piece_meets_a_low_shelf`].
    fn low_shelf_case(
        (props, wide, tall, (right, along, rise, built), door, chat): &LowShelfDraw,
    ) -> Result<Seen, TestCaseError> {
        let (mut buf, nooks) = two_panes(*wide, *tall, &[]);
        let mut home = Home {
            door: door.and_then(|right| {
                door_on(Nook::Users, if right { Side::Right } else { Side::Left })
            }),
            structures: vec![shelf_in(&nooks, (0, *right, *along, *rise, *built))],
            ..Home::default()
        };
        for &p in props {
            prop_assert!(home.add(p));
        }
        let laid = home.laid_out(&nooks);
        let runs = shelf_runs(&home, &nooks);
        let mut seen = Seen::default();
        for run in &runs {
            for s in laid.shown.iter().filter(|s| {
                s.strip == Some(Strip::Bottom(run.shelf.nook)) && s.lane() == Lane::Wall
            }) {
                prop_assert!(!run.hangs_over(s.rect()), "{:?} meets {:?}", s, run);
            }
            if !run.shelf.high() {
                seen.moved += laid
                    .shifts
                    .iter()
                    .filter(|(item, _)| *item != Furniture::Window)
                    .count();
            }
        }
        // The closet hangs a piece in the chat clear of the block too,
        // where it takes it to (as the layout would hang it there).
        if let Some(cols) = chat {
            let chat = Rect::new(0, 0, (*cols).min(*wide - 1), *tall);
            for (i, to) in placements(&home, &laid, chat) {
                let (Some((strip, left)), Some(s)) = (to, laid.shown.get(i)) else {
                    continue;
                };
                let Some(plan) = laid.plan(strip).filter(|_| strip.is_floor()) else {
                    continue;
                };
                if s.lane() != Lane::Wall {
                    continue;
                }
                let at = Shown {
                    strip: Some(strip),
                    left,
                    floor: plan.raw.floor,
                    stood: plan.host.floor,
                    ..*s
                };
                for run in runs.iter().filter(|r| Strip::Bottom(r.shelf.nook) == strip) {
                    prop_assert!(!run.hangs_over(at.rect()), "{:?} placed on {:?}", at, run);
                    if !run.shelf.high() {
                        seen.placed += 1;
                    }
                }
            }
        }
        // On a frame with nothing else on it, nothing of hers hides it.
        let w = written(&mut buf, &home, Plan::bare(&nooks));
        let shown_on = w.shown_on().to_vec();
        w.keep(&mut Vec::new());
        let built: Vec<Nook> = runs
            .iter()
            .filter(|r| !r.built.is_empty())
            .map(|r| r.shelf.nook)
            .collect();
        prop_assert_eq!(&shown_on, &built, "{:?}", laid.shown);
        for run in runs.iter().filter(|r| !r.built.is_empty()) {
            if !run.shelf.high() {
                seen.low += 1;
                seen.six += usize::from(run.shelf.rise == LOW_RISE + 1);
            } else {
                seen.high += 1;
            }
        }
        Ok(seen)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(256)))]

        /// No piece that hangs meets a low shelf (D3a): on a pane up to
        /// 20 rows inside, with her clock, poster and window hung among
        /// what stands, her door maybe in a wall, and her shelf high or
        /// low (at `LOW_RISE` or a row higher), built or not, no hung
        /// piece meets its run or margin from its row up, laid out or
        /// placed by the closet out of a chat over the pane's first
        /// columns; and on a frame with nothing else on it her shelf
        /// shows, whatever of it is built (nothing of hers hides it).
        #[test]
        fn no_hung_piece_meets_a_low_shelf(draw in low_shelf_draw()) {
            low_shelf_case(&draw)?;
        }

        /// A high shelf (above every piece that hangs) changes no hung
        /// place: her home (her poster, clock and window among what
        /// stands) lays out on its panes just as with no shelf. Only
        /// shelves whose geometry holds count (the rest are rejected,
        /// so a change of pane sizes that made them rare shows).
        #[test]
        fn a_high_shelf_changes_no_hung_place(
            props in hung_among(),
            wide in 28u16..70,
            tall in 13u16..=22,
            shelf in (any::<bool>(), 0u16..60, 0u16..6, 0u8..=2),
            door in proptest::option::of(any::<bool>()),
        ) {
            let (_, nooks) = two_panes(wide, tall, &[]);
            let (right, along, extra, built) = shelf;
            let mut home = Home {
                door: door.and_then(|right| {
                    door_on(Nook::Users, if right { Side::Right } else { Side::Left })
                }),
                ..Home::default()
            };
            for p in props {
                prop_assert!(home.add(p));
            }
            let mut shelved = home.clone();
            shelved.structures = vec![shelf_in(&nooks, (0, right, along, HIGH_RISE + extra, built))];
            prop_assume!(!shelf_runs(&shelved, &nooks).is_empty());
            let (bare, with) = (home.laid_out(&nooks), shelved.laid_out(&nooks));
            prop_assert_eq!(&bare.shown, &with.shown);
            prop_assert_eq!(&bare.shifts, &with.shifts);
            prop_assert_eq!(&bare.plans[..], &with.plans[..bare.plans.len()]);
        }

        /// [`super::tests::a_move_relaid_strip_by_strip_is_laid_out_as_a_whole`]
        /// with her shelf (phase 6a, step 4): a home laid out again from
        /// another's strips laid out alone lays out just as laying it all
        /// out does, whatever moves where and whatever becomes of her
        /// shelf meanwhile (put up, taken down, moved, raised or lowered,
        /// built further): a pane's layout reads its shelf (a low one's
        /// block), and a shelf's own layout reads it too (her clock may
        /// be put on either pane's shelf strip, there or not). Kept from
        /// move to move as a search keeps them too (and on past her shelf
        /// changing, which lays every strip out afresh).
        #[test]
        fn a_move_relaid_strip_by_strip_is_laid_out_as_a_whole_with_a_shelf(
            props in hung_among(),
            on in proptest::collection::vec(0usize..4, 6),
            sizes in proptest::collection::vec((28u16..70, 9u16..=20), 2),
            before in proptest::option::of(a_shelf()),
            after in proptest::option::of(proptest::option::of(a_shelf())),
            door in proptest::option::of((0usize..2, any::<bool>())),
            moves in proptest::collection::vec(
                (
                    any::<proptest::sample::Index>(),
                    (0usize..4, any::<bool>(), 0u16..40, any::<bool>()),
                    any::<bool>(),
                ),
                0..=2,
            ),
        ) {
            let order = [Nook::Users, Nook::Playlist];
            // A pane's floor, or (2 and 3) its shelf.
            let strip_of = |i: usize| {
                if i < 2 {
                    Strip::Bottom(order[i])
                } else {
                    Strip::Shelf(order[i - 2])
                }
            };
            let side = |right: bool| if right { Side::Right } else { Side::Left };
            let mut x = 0;
            let nooks: Vec<(Nook, Rect)> = order
                .iter()
                .zip(&sizes)
                .map(|(&nook, &(w, h))| {
                    let r = Rect::new(x, 0, w, h);
                    x += w;
                    (nook, r)
                })
                .collect();
            let mut home = Home {
                door: door.and_then(|(i, right)| door_on(order[i], side(right))),
                structures: before.map(|s| shelf_in(&nooks, s)).into_iter().collect(),
                ..Home::default()
            };
            for (p, &i) in props.into_iter().zip(on.iter().cycle()) {
                let p = Prop { strip: strip_of(i), ..p };
                prop_assert!(home.add(p));
            }
            let was = home.strips_laid_out(&nooks);
            prop_assert_eq!(home.relaid(&was), home.laid_and_shifted(&nooks));
            // And kept from move to move (as a search keeps them), each
            // home on the way and back again laid out as a whole too.
            let mut kept = LaidAgain::default();
            let mut homes = vec![home.clone()];
            for (moved, (strip, right, offset, left), boxed) in &moves {
                let i = moved.index(home.props.len());
                home.props[i] = Prop {
                    strip: strip_of(*strip),
                    anchor: Some(Anchor { side: side(*right), offset: *offset }),
                    facing: if *left { Facing::Left } else { Facing::Right },
                    boxed: (*boxed).into(),
                    ..home.props[i]
                };
                prop_assert_eq!(home.relaid_keeping(&was, &mut kept), home.laid_and_shifted(&nooks));
                homes.push(home.clone());
            }
            if let Some(after) = after {
                home.structures = after.map(|s| shelf_in(&nooks, s)).into_iter().collect();
                homes.push(home.clone());
            }
            for home in homes.iter().rev() {
                prop_assert_eq!(home.relaid_keeping(&was, &mut kept), home.laid_and_shifted(&nooks));
            }
            prop_assert_eq!(home.relaid(&was), home.laid_and_shifted(&nooks));
        }
    }

    /// The non-vacuity of [`no_hung_piece_meets_a_low_shelf`], over a
    /// fixed run of its strategy: shelves shown high and low (and one
    /// row above `LOW_RISE`), hung pieces a low shelf moved aside, and
    /// hung pieces the closet placed on a wall with a low shelf. Measured
    /// (2026-10-11): `Seen { high: 27, low: 114, six: 57, moved: 64,
    /// placed: 37 }`; each threshold is about half of that.
    #[test]
    fn low_shelves_move_what_hangs_aside() {
        let mut runner = TestRunner::new_with_rng(
            Config::default(),
            TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
        );
        let mut seen = Seen::default();
        for _ in 0..256 {
            let draw = low_shelf_draw().new_tree(&mut runner).unwrap().current();
            let once = low_shelf_case(&draw).unwrap_or_else(|e| panic!("{e}"));
            seen.high += once.high;
            seen.low += once.low;
            seen.six += once.six;
            seen.moved += once.moved;
            seen.placed += once.placed;
        }
        eprintln!("{seen:?}");
        assert!(
            seen.high >= 13
                && seen.low >= 57
                && seen.six >= 28
                && seen.moved >= 32
                && seen.placed >= 18,
            "{seen:?}"
        );
    }

    /// A piece that hangs where a low shelf's block is hangs at the
    /// nearest place clear of it (D3a), not where her box would be clear
    /// of what stands (only her window looks for a place to lean on its
    /// sill): Users 60 wide (inside columns 1..59), her shelf `Left` 8,
    /// low (its block 7..21), her bed against the left wall (1..11) and
    /// her poster `Left` 10 (11..15, on the block). It hangs at 3..7, two
    /// columns nearer than 21..25 (which her box would be clear at, both
    /// ends: at 3..7, the bed is under both), whatever of the shelf is up.
    #[test]
    fn a_poster_hangs_at_the_nearest_place_clear_of_a_low_shelf() {
        for built in [0u8, 1, 2] {
            let (_, nooks) = two_panes(60, 14, &[]);
            let mut home = Home {
                structures: vec![Structure::Shelf(Shelf {
                    nook: Nook::Users,
                    anchor: Anchor {
                        side: Side::Left,
                        offset: 8,
                    },
                    rise: LOW_RISE,
                    built,
                    fresh: None,
                })],
                ..Home::default()
            };
            for (item, offset) in [(Furniture::Bed, 0), (Furniture::Poster, 10)] {
                assert!(home.add(Prop {
                    anchor: Some(Anchor {
                        side: Side::Left,
                        offset,
                    }),
                    ..Prop::new(item, Nook::Users, 0, Facing::Right)
                }));
            }
            let laid = home.laid_out(&nooks);
            let poster = laid
                .shown
                .iter()
                .find(|s| s.item == Furniture::Poster)
                .expect("hung");
            assert_eq!(poster.left, 3, "built {built}");
            assert_eq!(laid.shifts, [(Furniture::Poster, Some(8))], "built {built}");
        }
    }

    /// The closet hangs a piece in the chat clear of a low shelf's block
    /// as the layout does (D3a; door batch D7): Users 60 wide (inside
    /// 1..59) with her shelf `Left` 8, low (its block 7..21), her poster
    /// against its left wall under a chat over its first 6 columns: on
    /// its own strip, it would go past the block (21), not just past the
    /// chat (6). With the chat over all of Users, it's taken to the
    /// Playlist (inside 61..99), where her clock hangs against the left
    /// wall (61..64) and her shelf hangs low `Left` 8 too (its block
    /// 67..81): packed there it would push her clock into the block, or
    /// meet it itself, at every place before 81.
    #[test]
    fn the_closet_hangs_a_piece_clear_of_a_low_shelf() {
        let (_, nooks) = two_panes(60, 14, &[]);
        let shelf = |nook| {
            Structure::Shelf(Shelf {
                nook,
                anchor: Anchor {
                    side: Side::Left,
                    offset: 8,
                },
                rise: LOW_RISE,
                built: 2,
                fresh: None,
            })
        };
        let hung = |item, nook| Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset: 0,
            }),
            ..Prop::new(item, nook, 0, Facing::Right)
        };
        let mut home = Home {
            structures: vec![shelf(Nook::Users)],
            ..Home::default()
        };
        assert!(home.add(hung(Furniture::Poster, Nook::Users)));
        let laid = home.laid_out(&nooks);
        assert_eq!(
            placements(&home, &laid, Rect::new(0, 0, 6, 14)),
            [(0, Some((Strip::Bottom(Nook::Users), 21)))]
        );
        let mut home = Home {
            structures: vec![shelf(Nook::Playlist)],
            ..Home::default()
        };
        assert!(home.add(hung(Furniture::Clock, Nook::Playlist)));
        assert!(home.add(hung(Furniture::Poster, Nook::Users)));
        let laid = home.laid_out(&nooks);
        let poster = laid
            .shown
            .iter()
            .position(|s| s.item == Furniture::Poster)
            .unwrap();
        assert_eq!(
            placements(&home, &laid, Rect::new(0, 0, 60, 14)),
            [(poster, Some((Strip::Bottom(Nook::Playlist), 81)))]
        );
    }

    /// A poster delivered to a wall with a low shelf hangs beside it,
    /// and so does what it pushes along (D3a): Users 42 wide (inside
    /// columns 1..41), her shelf `Left` 8, low (its run 9..19 on row 5,
    /// its margin 7..21), her clock hung against Users' left wall (1..4)
    /// and her window against its right (37..41). The poster comes in
    /// through the left wall, at the screen's edge, and hangs against it
    /// (1..5: the newest piece is nearest its wall), pushing her clock
    /// along into the shelf's margin (5..8): the clock hangs past the
    /// shelf instead (21..24), and the shelf shows, whether it's up yet
    /// or not.
    #[test]
    fn a_poster_delivered_hangs_beside_a_low_shelf() {
        for built in [0u8, 1, 2] {
            let (mut buf, nooks) = two_panes(42, 12, &[]);
            let plan = Plan::bare(&nooks);
            let mut home = Home {
                structures: vec![Structure::Shelf(Shelf {
                    nook: Nook::Users,
                    anchor: Anchor {
                        side: Side::Left,
                        offset: 8,
                    },
                    rise: LOW_RISE,
                    built,
                    fresh: None,
                })],
                ..Home::default()
            };
            for (item, side) in [
                (Furniture::Clock, Side::Left),
                (Furniture::Window, Side::Right),
            ] {
                assert!(home.add(Prop {
                    anchor: Some(Anchor { side, offset: 0 }),
                    ..Prop::new(item, Nook::Users, 0, Facing::Right)
                }));
            }
            let w = written(&mut buf, &home, plan);
            let shown = home.project_with(w.over(&buf), plan, &|_, _| false).0;
            let (poster, _) = home
                .doorstep(
                    w.over(&buf),
                    plan,
                    &shown,
                    &|_, _| false,
                    &anywhere,
                    Furniture::Poster,
                )
                .expect("delivered");
            w.undo(&mut buf);
            assert_eq!(
                (poster.strip, poster.anchor),
                (
                    Strip::Bottom(Nook::Users),
                    Some(Anchor {
                        side: Side::Left,
                        offset: 0
                    })
                ),
                "built {built}"
            );
            assert!(home.add(poster));
            assert!(home.unbox(Furniture::Poster));
            let laid = home.laid_out(&nooks);
            let at = laid
                .shown
                .iter()
                .find(|s| s.item == Furniture::Poster)
                .expect("hung");
            assert_eq!(at.left, 1, "built {built}");
            let clock = laid
                .shown
                .iter()
                .find(|s| s.item == Furniture::Clock)
                .expect("hung");
            assert_eq!(clock.left, 21, "built {built}");
            assert_eq!(laid.shifts, [(Furniture::Clock, Some(16))], "built {built}");
            let w = written(&mut buf, &home, plan);
            let shown_on = w.shown_on().to_vec();
            let shown = home.project_with(w.over(&buf), plan, &|_, _| false).0;
            w.keep(&mut Vec::new());
            let want: &[Nook] = if built == 0 { &[] } else { &[Nook::Users] };
            assert_eq!(shown_on, want, "built {built}");
            assert_eq!(shown.len(), 3, "built {built}: {shown:?}");
        }
    }

    /// Her shelf on Users, `Left 8`, low (its run 9..19 on row 6, its
    /// block 7..21 over rows 5..6), `built`.
    fn low_on_users(built: u8) -> Structure {
        Structure::Shelf(Shelf {
            nook: Nook::Users,
            anchor: Anchor {
                side: Side::Left,
                offset: 8,
            },
            rise: LOW_RISE,
            built,
            fresh: None,
        })
    }

    /// `item` hung on `nook`'s wall, `side` `offset`.
    fn hung_at(item: Furniture, nook: Nook, side: Side, offset: u16) -> Prop {
        Prop {
            anchor: Some(Anchor { side, offset }),
            ..Prop::new(item, nook, 0, Facing::Right)
        }
    }

    /// A delivery is judged where the layout hangs what it pushes along
    /// (D3a): [`a_poster_delivered_hangs_beside_a_low_shelf`]'s wall,
    /// with text where her clock would hang past the shelf (22, 5). A
    /// poster through Users' left wall would push her clock onto the
    /// shelf's blank margin, and the layout hangs it past the shelf, on
    /// the text: so it never comes in there, and every piece that showed
    /// still shows with it in.
    #[test]
    fn a_delivery_never_pushes_a_piece_past_a_low_shelf_under_text() {
        for built in [0u8, 1, 2] {
            let (mut buf, nooks) = two_panes(42, 12, &[(22, 5)]);
            let plan = Plan::bare(&nooks);
            let mut home = Home {
                structures: vec![low_on_users(built)],
                ..Home::default()
            };
            assert!(home.add(hung_at(Furniture::Clock, Nook::Users, Side::Left, 0)));
            assert!(home.add(hung_at(Furniture::Window, Nook::Users, Side::Right, 0)));
            let w = written(&mut buf, &home, plan);
            let before = home.project_with(w.over(&buf), plan, &|_, _| false).0;
            assert_eq!(before.len(), 2, "built {built}: {before:?}");
            let (poster, _) = home
                .doorstep(
                    w.over(&buf),
                    plan,
                    &before,
                    &|_, _| false,
                    &anywhere,
                    Furniture::Poster,
                )
                .expect("delivered somewhere");
            w.undo(&mut buf);
            assert_ne!(
                (poster.strip, poster.anchor),
                (
                    Strip::Bottom(Nook::Users),
                    Some(Anchor {
                        side: Side::Left,
                        offset: 0
                    })
                ),
                "built {built}"
            );
            assert!(home.add(poster));
            assert!(home.unbox(Furniture::Poster));
            let w = written(&mut buf, &home, plan);
            let after = home.project_with(w.over(&buf), plan, &|_, _| false).0;
            w.undo(&mut buf);
            for s in &before {
                assert!(
                    after.iter().any(|t| t.item == s.item),
                    "built {built}: {:?} hidden: {after:?}",
                    s.item
                );
            }
        }
    }

    /// A piece the layout already hangs aside from a low shelf is judged
    /// there when a delivery packs it along (D3a: later deliveries hang
    /// beside it): her shelf up (its run written on row 6), her clock
    /// anchored `Left 10` (over the run, 11..14), hung aside at 4..7. A
    /// poster through Users' left wall (1..5) packs it on at its anchor,
    /// on the written run; the layout hangs it past the shelf (21..24),
    /// clear. So the poster comes in there, and all show.
    #[test]
    fn a_delivery_hangs_beside_a_piece_already_hung_aside() {
        let (mut buf, nooks) = two_panes(42, 12, &[]);
        let plan = Plan::bare(&nooks);
        let mut home = Home {
            structures: vec![low_on_users(2)],
            ..Home::default()
        };
        assert!(home.add(hung_at(Furniture::Clock, Nook::Users, Side::Left, 10)));
        let w = written(&mut buf, &home, plan);
        assert_eq!(w.shown_on(), [Nook::Users]);
        let before = home.project_with(w.over(&buf), plan, &|_, _| false).0;
        assert_eq!(before.iter().map(|s| s.left).collect::<Vec<_>>(), [4]);
        let (poster, _) = home
            .doorstep(
                w.over(&buf),
                plan,
                &before,
                &|_, _| false,
                &anywhere,
                Furniture::Poster,
            )
            .expect("delivered");
        w.undo(&mut buf);
        assert_eq!(
            (poster.strip, poster.anchor),
            (
                Strip::Bottom(Nook::Users),
                Some(Anchor {
                    side: Side::Left,
                    offset: 0
                })
            )
        );
        assert!(home.add(poster));
        assert!(home.unbox(Furniture::Poster));
        let w = written(&mut buf, &home, plan);
        let after = home.project_with(w.over(&buf), plan, &|_, _| false).0;
        assert_eq!(w.shown_on(), [Nook::Users]);
        w.undo(&mut buf);
        let lefts: Vec<(Furniture, i32)> = after.iter().map(|s| (s.item, s.left)).collect();
        assert_eq!(
            lefts,
            [(Furniture::Clock, 21), (Furniture::Poster, 1)],
            "{after:?}"
        );
    }

    /// Hung pieces moving off a pane that's gone are judged where the
    /// layout hangs them on the wall they'd move to (D3a), as a delivery
    /// is: her clock alone on The List (not on this frame), Users 42 wide
    /// with her shelf low `Left 8` (its block 7..21), the Playlist bare.
    /// Its share puts it at 19 on Users' wall (on the block's blank
    /// margin), where the layout hangs it at 21, on text at (22, 5): it
    /// goes to the Playlist instead, and shows. Its share at 12 (on the
    /// written run), the layout hangs it at 4, clear: it goes to Users.
    #[test]
    fn hung_pieces_moving_off_hang_clear_of_a_low_shelf() {
        for (left, text, to, hangs) in [
            (19, &[(22u16, 5u16)][..], Nook::Playlist, None),
            (12, &[][..], Nook::Users, Some(4)),
        ] {
            let (mut buf, nooks) = two_panes(42, 12, text);
            let plan = Plan::bare(&nooks);
            let users = pane_strips(&nooks)[0].raw;
            let at = (0..=1000).find(|&at| users.share(at, 3) == left).unwrap();
            let mut home = Home {
                structures: vec![low_on_users(2)],
                ..Home::default()
            };
            assert!(home.add(Prop {
                at,
                ..Prop::new(Furniture::Clock, Nook::List, 0, Facing::Right)
            }));
            let w = written(&mut buf, &home, plan);
            let (shown, moves) = home.project_with(w.over(&buf), plan, &|_, _| false);
            w.undo(&mut buf);
            assert_eq!(
                moves,
                [(Strip::Bottom(Nook::List), Strip::Bottom(to))],
                "share at {left}"
            );
            let clock = shown.iter().find(|s| s.item == Furniture::Clock);
            assert!(clock.is_some(), "share at {left}: hidden");
            if let Some(hangs) = hangs {
                assert_eq!(clock.map(|s| s.left), Some(hangs));
            }
        }
    }

    /// Of two places beside a low shelf's block equally near, a piece
    /// hangs at the left one (D3a): Users 60 wide, her shelf low `Left
    /// 8` (its block 7..21), her poster `Left 11` (12..16): 9 columns
    /// from 3 (3..7) and from 21. It hangs at 3.
    #[test]
    fn a_poster_hangs_left_of_two_places_as_near() {
        let (_, nooks) = two_panes(60, 14, &[]);
        let mut home = Home {
            structures: vec![low_on_users(2)],
            ..Home::default()
        };
        assert!(home.add(hung_at(Furniture::Poster, Nook::Users, Side::Left, 11)));
        let laid = home.laid_out(&nooks);
        assert_eq!(laid.shown.iter().map(|s| s.left).collect::<Vec<_>>(), [3]);
        assert_eq!(laid.shifts, [(Furniture::Poster, Some(9))]);
    }

    /// A shelf one row higher than `LOW_RISE` (rise 6, in the poster's
    /// top row) is low too: its block is that one row, and a poster
    /// whose anchor puts it there hangs beside it (D3a). At `HIGH_RISE`
    /// it stays where its anchor puts it.
    #[test]
    fn a_poster_hangs_beside_a_shelf_one_row_higher() {
        for (rise, left) in [(LOW_RISE + 1, 3), (HIGH_RISE, 11)] {
            let (_, nooks) = two_panes(60, 14, &[]);
            let mut home = Home {
                structures: vec![Structure::Shelf(Shelf {
                    rise,
                    ..match low_on_users(2) {
                        Structure::Shelf(shelf) => shelf,
                    }
                })],
                ..Home::default()
            };
            assert!(home.add(hung_at(Furniture::Poster, Nook::Users, Side::Left, 10)));
            let laid = home.laid_out(&nooks);
            let at = laid.shown[0];
            assert_eq!(at.left, left, "rise {rise}");
            let runs = shelf_runs(&home, &nooks);
            assert_eq!(runs.len(), 1, "rise {rise}");
            assert!(!runs[0].hangs_over(at.rect()), "rise {rise}");
        }
    }
}

/// Where a shelf goes up (phase 6a D3, step 5a): [`Home::shelf_spot`]
/// against a brute-force oracle that tries every pane, rise and left,
/// judging each cell by D4's rules written out again (never
/// [`structures::free`]), its pane's background as the draw declares it
/// and a low shelf's block as [`super::tests::ShelfRun::hangs_over`]
/// works it out from the record and the pane's rect; and its cost.
/// Independent of the drawing mode: no loop over it (the frame's cells
/// are the app's).
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod spot {
    use super::super::cells::sanitize;
    use super::super::cells::untouchable as image_cell;
    use super::super::structures::{KeepOff, Shelf, Structure};
    use super::super::tests::shelf_runs;
    use super::*;
    use proptest::prelude::*;
    use proptest::strategy::ValueTree;
    use proptest::test_runner::{Config, TestCaseError, TestRng, TestRunner};
    use tuirealm::ratatui::style::{Modifier, Style};
    use tuirealm::ratatui::widgets::{Block, Widget};

    const NOOKS: [Nook; 3] = [Nook::List, Nook::Users, Nook::Playlist];
    /// The kinds laid out under a shelf's place: what stands and what
    /// hangs (this strategy's own list).
    const KINDS: [Furniture; 7] = [
        Furniture::Sofa,
        Furniture::Tv,
        Furniture::Lamp,
        Furniture::Plant,
        Furniture::Poster,
        Furniture::Clock,
        Furniture::Window,
    ];
    /// A theme's background for a whole pane (border and inside).
    const THEME: Color = Color::Indexed(236);
    /// The rises a tint is drawn near: a low shelf's, a high one's, a row
    /// either side.
    const RISES: [u16; 5] = [5, 6, 7, 8, 9];

    /// A pane: `tall` 0, 9 rows (a low shelf at most); 1, 13 (a high one
    /// at rise 7 exactly); else `height`; from `top`.
    #[derive(Clone, Debug)]
    struct PaneDraw {
        tall: u8,
        height: u16,
        top: u16,
        theme: bool,
    }

    #[derive(Clone, Debug)]
    struct SpotDraw {
        width: u16,
        panes: Vec<PaneDraw>,
        /// `(x, y, len, wide)`, anywhere.
        text: Vec<(u16, u16, u16, bool)>,
        /// `(pane, rise, right, short)`: a word against a wall inside a
        /// pane, on the row of a low shelf (`rise` 0), or of a high one 7
        /// or 8 rows up (1, 2): the row has text, but not where a run or
        /// its margin could lie. `short`, one glyph (in the column right
        /// against the wall); else two.
        walls: Vec<(usize, u16, bool, bool)>,
        images: Vec<(u16, u16)>,
        /// `(pane, row)`: a selection bar across the row inside.
        bars: Vec<(usize, u16)>,
        /// `(pane, column, row)`: a blank tinted unlike its pane,
        /// anywhere inside it (`column` and `row` wrap).
        tints: Vec<(usize, u16, u16)>,
        /// `(pane, rise, dx, over)`: a blank tinted unlike its pane near
        /// where a shelf goes: `over` (0..4) rows above the row of the
        /// rise [`RISES`]`[rise]`, `dx` columns from the pane's middle.
        near: Vec<(usize, usize, i16, u16)>,
        protected: Vec<(u16, u16, u16, u16)>,
        focused: Option<usize>,
        /// 0..=2, that pane is the chat; else a rect, or none.
        chat: Option<(u8, (u16, u16, u16, u16))>,
        layer: Vec<(u16, u16)>,
        made: Vec<(u16, u16, u16, u16)>,
        pieces: Vec<(usize, usize, u16)>,
        /// `(pane, clock)`: her poster (or her clock) hung mid-wall in
        /// that pane, before `pieces`: under the middle run of rise 7, so
        /// the touching rule has a run beside it to choose.
        centred: Option<(usize, bool)>,
        door: Option<(usize, bool)>,
        kit_at: Option<usize>,
        /// She has a shelf already.
        has: bool,
        /// `(pane, at)`: her gremlin on that pane's floor (phase 6a step
        /// 7: [`gremlin_spot_draw`] only; [`spot_draw`] draws none).
        gremlin: Option<(usize, u16)>,
    }

    fn rect_draw() -> impl Strategy<Value = (u16, u16, u16, u16)> {
        (0u16..170, 0u16..36, 1u16..30, 1u16..6)
    }

    fn spot_draw() -> impl Strategy<Value = SpotDraw> {
        let pane = (0u8..4, 5u16..=30, 0u16..4, proptest::bool::weighted(0.15)).prop_map(
            |(tall, height, top, theme)| PaneDraw {
                tall,
                height,
                top,
                theme,
            },
        );
        (
            (
                60u16..=170,
                proptest::collection::vec(pane, 1..=3),
                proptest::collection::vec((0u16..170, 0u16..36, 1u16..40, any::<bool>()), 0..12),
                proptest::collection::vec((0usize..3, 0u16..3, any::<bool>(), any::<bool>()), 0..6),
                proptest::collection::vec((0u16..170, 0u16..36), 0..3),
                proptest::collection::vec((0usize..3, 0u16..30), 0..2),
                proptest::collection::vec((0usize..3, 0u16..170, 0u16..30), 0..3),
                proptest::collection::vec((0usize..3, 0..RISES.len(), -10i16..10, 0u16..4), 0..4),
            ),
            (
                proptest::collection::vec(rect_draw(), 0..3),
                proptest::option::weighted(0.1, 0usize..3),
                proptest::option::weighted(0.3, (0u8..6, rect_draw())),
                proptest::collection::vec((0u16..170, 0u16..36), 0..12),
                proptest::collection::vec(rect_draw(), 0..3),
                proptest::collection::vec((0..KINDS.len(), 0usize..3, 0u16..=1000), 0..6),
                proptest::option::weighted(0.25, (0usize..3, any::<bool>())),
                proptest::option::weighted(0.4, (0usize..3, any::<bool>())),
                proptest::option::weighted(0.4, 0usize..3),
                proptest::bool::weighted(0.05),
            ),
        )
            .prop_map(
                |(
                    (width, panes, text, walls, images, bars, tints, near),
                    (protected, focused, chat, layer, made, pieces, centred, door, kit_at, has),
                )| SpotDraw {
                    width,
                    panes,
                    text,
                    walls,
                    images,
                    bars,
                    tints,
                    near,
                    protected,
                    focused,
                    chat,
                    layer,
                    made,
                    pieces,
                    centred,
                    door,
                    kit_at,
                    has,
                    gremlin: None,
                },
            )
    }

    /// [`spot_draw`] with her gremlin on a floor most of the time (phase
    /// 6a step 7: its own strategy; `spot_draw`'s draws are unchanged).
    fn gremlin_spot_draw() -> impl Strategy<Value = SpotDraw> {
        (
            spot_draw(),
            proptest::option::weighted(0.8, (0usize..3, 0u16..=1000)),
        )
            .prop_map(|(draw, gremlin)| SpotDraw { gremlin, ..draw })
    }

    /// A draw made a frame: the buffer, her panes, each pane's declared
    /// background, her home (its pieces anchored), what's kept off, and
    /// the pane she unpacks in.
    struct Spot {
        buf: Buffer,
        nooks: Vec<(Nook, Rect)>,
        themes: Vec<(Nook, Color)>,
        home: Home,
        protected: Vec<Rect>,
        chat: Rect,
        layer: Vec<(u16, u16)>,
        made: Vec<Rect>,
        kit_at: Option<Nook>,
    }

    fn spot(draw: &SpotDraw) -> Spot {
        let n = u16::try_from(draw.panes.len()).unwrap();
        let rects: Vec<Rect> = draw
            .panes
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let i = u16::try_from(i).unwrap();
                let x = i * draw.width / n;
                let right = (i + 1) * draw.width / n;
                let height = match p.tall {
                    0 => 9,
                    1 => 13,
                    _ => p.height,
                };
                Rect::new(x, p.top, right - x, height)
            })
            .collect();
        let height = rects.iter().map(|r| r.bottom()).max().unwrap_or(0) + 2;
        let mut buf = Buffer::empty(Rect::new(0, 0, draw.width, height));
        let nooks: Vec<(Nook, Rect)> = NOOKS.iter().copied().zip(rects.iter().copied()).collect();
        for &(_, pane) in &nooks {
            Block::bordered().render(pane, &mut buf);
        }
        let inside = |pane: usize, column: u16, row: u16| {
            let rect = nooks.get(pane)?.1;
            let rows = rect.height.checked_sub(2)?;
            let width = rect.width.checked_sub(2)?;
            (rows > 0 && width > 0).then(|| (rect.x + 1 + column % width, rect.y + 1 + row % rows))
        };
        for &(x, y, len, wide) in &draw.text {
            if !buf.area.contains((x, y).into()) {
                continue;
            }
            let text: String = if wide {
                "漢字".repeat(usize::from(len / 2 + 1))
            } else {
                "so what did you think ".repeat(2)
            };
            buf.set_stringn(x, y, &text, usize::from(len), Style::new());
        }
        for &(pane, rise, right, short) in &draw.walls {
            let Some(&(_, rect)) = nooks.get(pane) else {
                continue;
            };
            let rise = [5, 7, 8][usize::from(rise)];
            let Some(y) = (rect.bottom() - 1)
                .checked_sub(rise)
                .filter(|&y| y > rect.y)
            else {
                continue;
            };
            let word = if short { "o" } else { "ok" };
            let len = u16::try_from(word.len()).unwrap();
            let x = if right {
                rect.right() - 1 - len
            } else {
                rect.x + 1
            };
            buf.set_string(x, y, word, Style::new());
        }
        sanitize(&mut buf);
        for &(x, y) in &draw.images {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ")
                    .set_diff_option(tuirealm::ratatui::buffer::CellDiffOption::Skip);
            }
        }
        let mut themes = Vec::new();
        for (i, p) in draw.panes.iter().enumerate() {
            if p.theme {
                themes.push((NOOKS[i], THEME));
                buf.set_style(rects[i], Style::new().bg(THEME));
            }
        }
        for &(pane, row) in &draw.bars {
            if let Some((x, y)) = inside(pane, 0, row) {
                let width = rects[pane].width - 2;
                buf.set_style(
                    Rect::new(x, y, width, 1),
                    Style::new().add_modifier(Modifier::REVERSED),
                );
            }
        }
        for &(pane, column, row) in &draw.tints {
            if let Some((x, y)) = inside(pane, column, row) {
                buf.set_style(Rect::new(x, y, 1, 1), Style::new().bg(Color::Blue));
            }
        }
        for &(pane, rise, dx, over) in &draw.near {
            let Some(&rect) = rects.get(pane) else {
                continue;
            };
            let x = i32::from(rect.x) + i32::from(rect.width) / 2 + i32::from(dx);
            let y = i32::from(rect.bottom()) - 1 - i32::from(RISES[rise]) - i32::from(over);
            let inside_x = i32::from(rect.x) < x && x < i32::from(rect.right()) - 1;
            if inside_x && i32::from(rect.y) < y {
                let at = Rect::new(u16::try_from(x).unwrap(), u16::try_from(y).unwrap(), 1, 1);
                buf.set_style(at, Style::new().bg(Color::Blue));
            }
        }
        let rect = |(x, y, w, h): (u16, u16, u16, u16)| Rect::new(x, y, w, h);
        let mut protected: Vec<Rect> = draw.protected.iter().copied().map(rect).collect();
        if let Some(pane) = draw.focused.filter(|&p| p < rects.len()) {
            protected.push(rects[pane]);
        }
        let chat = match draw.chat {
            Some((pane @ 0..=2, _)) if usize::from(pane) < rects.len() => rects[usize::from(pane)],
            Some((_, r)) => rect(r),
            None => Rect::default(),
        };
        let mut home = Home {
            door: draw
                .door
                .filter(|&(pane, _)| pane < rects.len())
                .map(|(pane, right)| DoorWall {
                    strip: Strip::Bottom(NOOKS[pane]),
                    side: if right { Side::Right } else { Side::Left },
                }),
            ..Home::default()
        };
        if let Some((pane, clock)) = draw.centred.filter(|&(p, _)| p < rects.len()) {
            let item = if clock {
                Furniture::Clock
            } else {
                Furniture::Poster
            };
            home.add(Prop::new(item, NOOKS[pane], 500, Facing::Right));
        }
        for &(kind, pane, at) in &draw.pieces {
            if pane < rects.len() {
                home.add(Prop::new(KINDS[kind], NOOKS[pane], at, Facing::Right));
            }
        }
        if let Some((pane, at)) = draw.gremlin.filter(|&(p, _)| p < rects.len()) {
            home.add(Prop::new(
                Furniture::Gremlin,
                NOOKS[pane],
                at,
                Facing::Right,
            ));
        }
        if draw.has {
            home.structures.push(Structure::Shelf(Shelf {
                nook: NOOKS[0],
                anchor: Anchor {
                    side: Side::Left,
                    offset: 9,
                },
                rise: 30,
                built: 0,
                fresh: None,
            }));
        }
        // Anchored where their shares put them, as a frame's projection
        // pins them before a spot is chosen.
        home.layout(&nooks);
        Spot {
            buf,
            kit_at: draw.kit_at.filter(|&p| p < rects.len()).map(|p| NOOKS[p]),
            nooks,
            themes,
            home,
            protected,
            chat,
            layer: draw.layer.clone(),
            made: draw.made.iter().copied().map(rect).collect(),
        }
    }

    impl Spot {
        fn plan(&self) -> Plan<'_> {
            Plan {
                nooks: &self.nooks,
                chat: self.chat,
                screen: self.buf.area,
            }
        }

        fn chosen(&self) -> Option<Shelf> {
            let keep_off = KeepOff {
                protected: &self.protected,
                layer: &self.layer,
                made: &self.made,
            };
            self.home
                .shelf_spot(&self.buf, self.plan(), &keep_off, self.kit_at)
        }
    }

    /// A place a shelf could go, as the oracle orders them: high (0)
    /// before low (1), its pane's rank, text on its row, touching a hung
    /// piece (high), its rise, twice its middle's distance from its
    /// pane's, its left; and the shelf.
    type Place = (Key, Shelf);
    type Key = (u8, usize, bool, bool, u16, i32, i32);

    /// Which of D3's pane preferences [`places`] orders the panes by
    /// (each left out in turn to see whether it decided a choice).
    #[derive(Clone, Copy)]
    struct Prefer {
        /// Her gremlin standing on a floor puts his pane first, and in
        /// it the places nearest his middle first (step 7); else he's
        /// only a piece.
        gremlin: bool,
        /// Her living room (her sofa's pane, the sofa out of its box on
        /// its floor) next (the user's eighth round); else it's only a
        /// pane.
        living: bool,
        /// The pane she unpacks in next.
        kit: bool,
    }

    const PREFER_ALL: Prefer = Prefer {
        gremlin: true,
        living: true,
        kit: true,
    };

    /// Every place a shelf could go on `spot`'s frame, worked out from
    /// D3's words and the panes' rects alone, its panes ranked as
    /// `prefer` says; and how many low places were refused for their
    /// block alone.
    fn places(spot: &Spot, prefer: Prefer) -> (Vec<Place>, usize) {
        let mut blocked = 0;
        if !spot.home.structures.is_empty() {
            return (Vec::new(), blocked);
        }
        let laid = spot.home.laid_and_shifted(&spot.nooks).shown;
        let covers: Vec<Rect> = laid.iter().map(Shown::cover).collect();
        let buf = &spot.buf;
        // Her gremlin out of his box on a floor: his pane, and twice his
        // middle's column.
        let gremlin = laid
            .iter()
            .find(|s| s.item == Furniture::Gremlin && !s.boxed() && s.scrap.is_none())
            .and_then(|s| match s.strip? {
                Strip::Bottom(nook) => Some((nook, 2 * s.left + i32::from(s.size().0))),
                Strip::Shelf(_) => None,
            })
            .filter(|_| prefer.gremlin);
        // Her sofa out of its box on a floor: her living room's pane.
        let living = laid
            .iter()
            .find(|s| s.item == Furniture::Sofa && !s.boxed() && s.scrap.is_none())
            .and_then(|s| match s.strip? {
                Strip::Bottom(nook) => Some(nook),
                Strip::Shelf(_) => None,
            })
            .filter(|_| prefer.living);
        let mut order: Vec<Nook> = gremlin.map(|(n, _)| n).into_iter().collect();
        if let Some(sofa) = living.filter(|n| !order.contains(n)) {
            order.push(sofa);
        }
        if let Some(kit) = spot.kit_at.filter(|k| prefer.kit && !order.contains(k)) {
            order.push(kit);
        }
        let rest: Vec<Nook> = spot
            .nooks
            .iter()
            .map(|&(n, _)| n)
            .filter(|n| !order.contains(n))
            .collect();
        order.extend(rest);
        let mut out = Vec::new();
        for (rank, nook) in order.into_iter().enumerate() {
            let pane = spot.nooks.iter().find(|(n, _)| *n == nook).unwrap().1;
            let c = spot.chat;
            let meets_chat = c.width > 0
                && c.height > 0
                && pane.x < c.x + c.width
                && c.x < pane.x + pane.width
                && pane.y < c.y + c.height
                && c.y < pane.y + pane.height;
            if meets_chat {
                continue;
            }
            let (left, right) = (i32::from(pane.x) + 1, i32::from(pane.right()) - 1);
            let floor = i32::from(pane.bottom()) - 1;
            let rows = i32::from(pane.height) - 2;
            let bg = spot
                .themes
                .iter()
                .find(|(n, _)| *n == nook)
                .map_or(Color::Reset, |&(_, bg)| bg);
            let ok = |x: i32, y: i32| {
                let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
                    return false;
                };
                let Some(cell) = buf.cell((ux, uy)) else {
                    return false;
                };
                let at = (ux, uy).into();
                let trailer = ux > 0
                    && buf
                        .cell((ux - 1, uy))
                        .is_some_and(|c| unicode_width::UnicodeWidthStr::width(c.symbol()) > 1);
                cell.symbol().trim().is_empty()
                    && !cell.modifier.contains(Modifier::REVERSED)
                    && cell.bg == bg
                    && !trailer
                    && !image_cell(cell)
                    && !spot.protected.iter().any(|r| r.contains(at))
                    && !spot.chat.contains(at)
                    && !spot.layer.contains(&(ux, uy))
                    && !spot.made.iter().any(|r| r.contains(at))
                    && !covers.iter().any(|r| r.contains(at))
            };
            // What hangs on this pane's wall, as laid out.
            let hung: Vec<Rect> = laid
                .iter()
                .filter(|s| {
                    s.strip == Some(Strip::Bottom(nook))
                        && !s.boxed()
                        && s.item.spec().hang.is_some()
                })
                .map(Shown::rect)
                .collect();
            for rise in 5..=rows {
                let (class, over) = match rise {
                    5 if rows - rise >= 2 => (1, 2),
                    7.. if rows - rise >= 4 => (0, 4),
                    _ => continue,
                };
                let y = floor - rise;
                let text = (left..right).any(|x| {
                    buf.cell((u16::try_from(x).unwrap(), u16::try_from(y).unwrap()))
                        .is_some_and(|c| !c.symbol().trim().is_empty())
                });
                for from in left + 8..=right - 18 {
                    let to = from + 10;
                    if !(from - 2..to + 2).all(|x| ok(x, y))
                        || !(1..=over).all(|k| (from..to).all(|x| ok(x, y - k)))
                    {
                        continue;
                    }
                    let anchor = if from - left <= right - to {
                        Anchor {
                            side: Side::Left,
                            offset: u16::try_from(from - left).unwrap(),
                        }
                    } else {
                        Anchor {
                            side: Side::Right,
                            offset: u16::try_from(right - to).unwrap(),
                        }
                    };
                    let shelf = Shelf {
                        nook,
                        anchor,
                        rise: u16::try_from(rise).unwrap(),
                        built: 0,
                        fresh: None,
                    };
                    let alone = Home {
                        structures: vec![Structure::Shelf(shelf)],
                        ..Home::default()
                    };
                    let runs = shelf_runs(&alone, &spot.nooks);
                    assert_eq!(runs.len(), 1, "{shelf:?}: its geometry holds");
                    if class == 1 && hung.iter().any(|&r| runs[0].hangs_over(r)) {
                        blocked += 1;
                        continue;
                    }
                    let touches = class == 0
                        && hung.iter().any(|r| {
                            i32::from(r.y) == y + 1
                                && i32::from(r.x) < to
                                && from < i32::from(r.right())
                        });
                    let middle = match gremlin {
                        Some((n, twice)) if n == nook => twice,
                        _ => left + right,
                    };
                    let away = (2 * from + 10 - middle).abs();
                    let key = (class, rank, text, touches, shelf.rise, away, from);
                    out.push((key, shelf));
                }
            }
        }
        (out, blocked)
    }

    /// How often each case came up (non-vacuity): a high, low or no
    /// place chosen; one in a pane after the first in order; a choice
    /// the text rule, the touching rule, her living room or the kit's
    /// pane decided (the first place differs with it left out of the
    /// order); frames where a
    /// low place was refused for its block alone (printed, not asserted:
    /// with today's hung kinds, the poster and the clock 2 rows tall from
    /// the block's top row, a piece meeting a block always meets the run's
    /// row or its margin too, which the cells refuse first; the block rule
    /// stands for a hung kind that doesn't).
    #[derive(Debug, Default)]
    struct Seen {
        high: usize,
        low: usize,
        none: usize,
        later: usize,
        texted: usize,
        touching: usize,
        kit: usize,
        blocked: usize,
        /// A choice her gremlin decided (step 7): his pane first, or the
        /// place nearest him in it (the first place differs with him
        /// only a piece).
        gremlin: usize,
        /// A choice her living room decided (the user's eighth round:
        /// the first place differs with her sofa's pane only a pane).
        living: usize,
    }

    impl Seen {
        fn add(&mut self, one: &Seen) {
            self.high += one.high;
            self.low += one.low;
            self.none += one.none;
            self.later += one.later;
            self.texted += one.texted;
            self.touching += one.touching;
            self.kit += one.kit;
            self.blocked += one.blocked;
            self.gremlin += one.gremlin;
            self.living += one.living;
        }
    }

    fn hangs_as_high_as_fits(draw: &SpotDraw) -> Result<Seen, TestCaseError> {
        let spot = spot(draw);
        let chosen = spot.chosen();
        let (all, blocked) = places(&spot, PREFER_ALL);
        let first = all.iter().min_by_key(|&&(key, _)| key).copied();
        prop_assert_eq!(chosen.is_some(), first.is_some(), "{:?}", first);
        prop_assert_eq!(chosen, first.map(|(_, shelf)| shelf));
        let mut seen = Seen {
            blocked: usize::from(blocked > 0),
            ..Seen::default()
        };
        let Some(((class, rank, ..), shelf)) = first else {
            seen.none += 1;
            return Ok(seen);
        };
        if class == 0 {
            seen.high += 1;
        } else {
            seen.low += 1;
        }
        seen.later += usize::from(rank > 0);
        // The first place with a rule left out of the order.
        let without = |blind: fn(&mut Key)| {
            all.iter()
                .map(|&(mut key, shelf)| {
                    blind(&mut key);
                    (key, shelf)
                })
                .min_by_key(|&(key, _)| key)
                .map(|(_, s)| s)
        };
        seen.texted += usize::from(without(|k| k.2 = false) != Some(shelf));
        seen.touching += usize::from(without(|k| k.3 = false) != Some(shelf));
        // The first place with one pane preference left out.
        let decided = |prefer: Prefer| {
            let (plain, _) = places(&spot, prefer);
            let first = plain.iter().min_by_key(|&&(key, _)| key).map(|&(_, s)| s);
            usize::from(first != Some(shelf))
        };
        seen.gremlin += decided(Prefer {
            gremlin: false,
            ..PREFER_ALL
        });
        seen.living += decided(Prefer {
            living: false,
            ..PREFER_ALL
        });
        seen.kit += decided(Prefer {
            kit: false,
            ..PREFER_ALL
        });
        Ok(seen)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(256)))]

        /// A shelf goes up exactly where D3 says: wherever any place
        /// takes one, and then the first in D3's order (high before low
        /// over every pane; the panes in order, her living room (her
        /// sofa's pane) first, then the pane she unpacks in; a row with
        /// no text; a run
        /// touching no hung piece; the lowest rise; nearest the pane's
        /// middle; the left of two as near), every cell of its run, its
        /// margin and the rows over its run as D4 judges them.
        #[test]
        fn a_shelf_hangs_as_high_as_fits(draw in spot_draw()) {
            hangs_as_high_as_fits(&draw)?;
        }

        /// [`a_shelf_hangs_as_high_as_fits`] with her gremlin on a floor
        /// (phase 6a step 7, D3): his pane before every other (her
        /// living room next, then the pane she unpacks in), and in it the
        /// places nearest his middle first.
        #[test]
        fn a_shelf_hangs_as_high_as_fits_by_her_gremlin(draw in gremlin_spot_draw()) {
            hangs_as_high_as_fits(&draw)?;
        }
    }

    /// Bordered panes `nooks` on a blank screen `width` × `height`.
    fn panes(nooks: &[(Nook, Rect)], width: u16, height: u16) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        for &(_, pane) in nooks {
            Block::bordered().render(pane, &mut buf);
        }
        buf
    }

    /// Where `home` puts a shelf on `buf` with `nooks`, `chat` and
    /// nothing else kept off.
    fn spot_on(
        home: &Home,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        chat: Rect,
        kit_at: Option<Nook>,
    ) -> Option<Shelf> {
        let keep_off = KeepOff {
            protected: &[],
            layer: &[],
            made: &[],
        };
        let plan = Plan {
            nooks,
            chat,
            screen: buf.area,
        };
        home.shelf_spot(buf, plan, &keep_off, kit_at)
    }

    fn shelf(nook: Nook, side: Side, offset: u16, rise: u16) -> Shelf {
        Shelf {
            nook,
            anchor: Anchor { side, offset },
            rise,
            built: 0,
            fresh: None,
        }
    }

    /// D3's order, case by case: high in a later pane before low in the
    /// first; the pane she unpacks in first; never the chat's pane; a row
    /// with no text before one with text; a run touching no hung piece
    /// before one that does; nearest the pane's middle, the left of two as
    /// near; none where she has one already.
    #[test]
    fn a_shelf_goes_where_d3_orders_it() {
        // List 9 rows (a low shelf at most), Users and the Playlist 13 (a
        // high one at rise 7 exactly), 41 wide: inside, columns 1..40 of
        // each, its middle between 20 and 21.
        let nooks = [
            (Nook::List, Rect::new(0, 0, 41, 9)),
            (Nook::Users, Rect::new(41, 0, 41, 13)),
            (Nook::Playlist, Rect::new(82, 0, 41, 13)),
        ];
        let buf = panes(&nooks, 123, 14);
        let home = Home::default();
        // High in Users before low in List; of 15 and 16, as near the
        // middle, the left (Users' inside starts at 42: offset 14).
        let users = shelf(Nook::Users, Side::Left, 14, 7);
        assert_eq!(
            spot_on(&home, &buf, &nooks, Rect::default(), None),
            Some(users)
        );
        // The Playlist first, where she unpacks; Users, the chat's,
        // never.
        let playlist = Shelf {
            nook: Nook::Playlist,
            ..users
        };
        let kit = Some(Nook::Playlist);
        assert_eq!(
            spot_on(&home, &buf, &nooks, Rect::default(), kit),
            Some(playlist)
        );
        assert_eq!(
            spot_on(&home, &buf, &nooks, nooks[1].1, None),
            Some(playlist)
        );
        // Every high place's pane the chat's: List's low one, in the
        // middle of its wall.
        let both = nooks[1].1.union(nooks[2].1);
        let list = shelf(Nook::List, Side::Left, 14, 5);
        assert_eq!(spot_on(&home, &buf, &nooks, both, None), Some(list));
        // And every pane the chat's, or one shelf hers already: none.
        let all = Rect::new(0, 0, 123, 14);
        assert_eq!(spot_on(&home, &buf, &nooks, all, None), None);
        let hers = Home {
            structures: vec![Structure::Shelf(list)],
            ..Home::default()
        };
        assert_eq!(spot_on(&hers, &buf, &nooks, Rect::default(), None), None);

        // A pane 14 rows tall (rises 7 and 8): a glyph against either of
        // its walls on rise 7's row (inside, columns 1..40) sends the
        // shelf a row higher.
        let tall = [(Nook::Users, Rect::new(0, 0, 41, 14))];
        let blank = panes(&tall, 41, 15);
        assert_eq!(
            spot_on(&home, &blank, &tall, Rect::default(), None),
            Some(shelf(Nook::Users, Side::Left, 14, 7))
        );
        for x in [1, 39] {
            let mut worded = blank.clone();
            worded.set_string(x, 13 - 7, "o", Style::new());
            assert_eq!(
                spot_on(&home, &worded, &tall, Rect::default(), None),
                Some(shelf(Nook::Users, Side::Left, 14, 8)),
                "a glyph at {x}"
            );
        }

        // Her poster hung mid-wall, right under rise 7's row: the run
        // nearest the middle that touches it not, beside it (a 13-row
        // pane has rise 7 only).
        let users_only = [nooks[1]];
        let buf = panes(&users_only, 82, 14);
        let mut postered = Home::default();
        postered.add(Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset: 18,
            }),
            ..Prop::new(Furniture::Poster, Nook::Users, 500, Facing::Right)
        });
        let poster = postered.layout(&users_only)[0].rect();
        assert_eq!((poster.x, poster.y, poster.width), (60, 6, 4), "{poster:?}");
        let chosen = spot_on(&postered, &buf, &users_only, Rect::default(), None).unwrap();
        let from = 42 + i32::from(chosen.anchor.offset);
        assert_eq!(chosen.rise, 7);
        // Clear of columns 60..64: ending at 60 (from 50) is two from the
        // middle's 50.5, as near as starting at 64 is not.
        assert_eq!(from, 50, "{chosen:?}");
        // Touching a poster on a row with no text before clear of it on a
        // row with text: a 14-row pane (rises 7 and 8), 30 wide (its lefts
        // 9..=11 all over the poster's columns 13..17), a word against
        // its wall on rise 8's row.
        let short = [(Nook::Users, Rect::new(0, 0, 30, 14))];
        let mut buf = panes(&short, 30, 15);
        buf.set_string(1, 13 - 8, "ok", Style::new());
        let mut touched = Home::default();
        touched.add(Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset: 12,
            }),
            ..Prop::new(Furniture::Poster, Nook::Users, 500, Facing::Right)
        });
        let poster = touched.layout(&short)[0].rect();
        assert_eq!((poster.x, poster.y), (13, 13 - 6), "{poster:?}");
        let chosen = spot_on(&touched, &buf, &short, Rect::default(), None).unwrap();
        assert_eq!(chosen.rise, 7, "{chosen:?}");
        // With nowhere clear of it but beside the wall, it touches.
        let narrow = [(Nook::Users, Rect::new(0, 0, 30, 13))];
        let buf = panes(&narrow, 30, 14);
        let mut squeezed = Home::default();
        squeezed.add(Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset: 12,
            }),
            ..Prop::new(Furniture::Poster, Nook::Users, 500, Facing::Right)
        });
        squeezed.layout(&narrow);
        // The only place (inside 1..29: from 9..=11) meets columns 13..17.
        let chosen = spot_on(&squeezed, &buf, &narrow, Rect::default(), None).unwrap();
        assert_eq!(chosen.rise, 7, "{chosen:?}");
    }

    /// A low shelf goes up only where its block meets no piece she has
    /// hung (it never pushes one aside going up): beside her poster, or
    /// nowhere where the wall has no room beside it.
    #[test]
    fn a_low_shelf_goes_up_beside_what_hangs() {
        // 9 rows, 61 wide: inside 1..60, its middle between 30 and 31.
        let nooks = [(Nook::List, Rect::new(0, 0, 61, 9))];
        let buf = panes(&nooks, 61, 10);
        let postered = |offset: u16| {
            let mut home = Home::default();
            home.add(Prop {
                anchor: Some(Anchor {
                    side: Side::Left,
                    offset,
                }),
                ..Prop::new(Furniture::Poster, Nook::List, 500, Facing::Right)
            });
            let poster = home.layout(&nooks)[0].rect();
            (home, poster)
        };
        // Without it, mid-wall (of 25 and 26, as near, the left).
        assert_eq!(
            spot_on(&Home::default(), &buf, &nooks, Rect::default(), None),
            Some(shelf(Nook::List, Side::Left, 24, 5))
        );
        // Hung mid-wall (columns 26..30): the nearest place whose block
        // (the run and its margin, its row and the one above) is clear of
        // it starts at 32, from the right wall 18.
        let (home, poster) = postered(25);
        assert_eq!((poster.x, poster.y, poster.width), (26, 2, 4), "{poster:?}");
        let chosen = spot_on(&home, &buf, &nooks, Rect::default(), None);
        assert_eq!(chosen, Some(shelf(Nook::List, Side::Right, 18, 5)));
        let block = chosen.unwrap().block(pane_raw(nooks[0].1)).unwrap();
        assert!(!block.intersects(poster), "{block:?} {poster:?}");
        // On a wall 41 wide, a poster mid-wall leaves no place.
        let narrow = [(Nook::List, Rect::new(0, 0, 41, 9))];
        let buf = panes(&narrow, 41, 10);
        let mut home = Home::default();
        home.add(Prop {
            anchor: Some(Anchor {
                side: Side::Left,
                offset: 18,
            }),
            ..Prop::new(Furniture::Poster, Nook::List, 500, Facing::Right)
        });
        home.layout(&narrow);
        assert_eq!(spot_on(&home, &buf, &narrow, Rect::default(), None), None);
    }

    /// Choosing a spot reads each cell of a pane's wall once, and looks
    /// up at most a run's and its margin's columns of what it read (and
    /// its pane's background) for each place by geometry: linear in the
    /// pane's cells, whatever the frame holds (blank, so every place is
    /// looked at whole; or dense, so most are refused at once), at the
    /// stage's sizes and 400 wide.
    #[test]
    fn shelf_spot_is_cheap() {
        use super::super::structures::{EDGE, HIGH_RISE, LOW_ABOVE, LOW_RISE, SHELF_WIDTH};
        for (width, rows, dense, nine) in [
            (100, 13, false, false),
            (100, 13, true, false),
            (400, 20, false, false),
            (400, 20, true, false),
            (400, 9, false, true),
            (200, 9, false, true),
        ] {
            let nooks: Vec<(Nook, Rect)> = NOOKS
                .iter()
                .enumerate()
                .map(|(i, &n)| (n, Rect::new(0, rows * i as u16, width, rows)))
                .collect();
            let mut buf = panes(&nooks, width, 3 * rows);
            if dense {
                // Every other cell of every row inside: no place passes.
                for &(_, pane) in &nooks {
                    for y in pane.y + 1..pane.bottom() - 1 {
                        for x in (pane.x + 1..pane.right() - 1).step_by(2) {
                            buf[(x, y)].set_symbol("x");
                        }
                    }
                }
            }
            let plan = Plan::bare(&nooks);
            let keep_off = KeepOff {
                protected: &[],
                layer: &[],
                made: &[],
            };
            let mut visited = 0;
            let chosen =
                Home::default().shelf_spot_visiting(&buf, plan, &keep_off, None, &mut visited);
            assert_eq!(chosen.is_some(), !dense, "{width}x{rows}");
            // The bound: each pane's wall rows read whole, and 1 + the
            // margined run's columns for each place.
            let her = 4;
            let (mut cells, mut places) = (0, 0);
            for &(_, pane) in &nooks {
                let inside = i32::from(pane.width) - 2;
                let rows = i32::from(pane.height) - 2;
                cells += inside * (rows - i32::from(LOW_RISE) + 1);
                let lefts = inside - 2 * EDGE - SHELF_WIDTH + 1;
                let rises = (rows - her - i32::from(HIGH_RISE) + 1).max(0)
                    + i32::from(rows >= i32::from(LOW_RISE + LOW_ABOVE));
                places += lefts * rises;
            }
            let bound = usize::try_from(cells + (SHELF_WIDTH + 5) * places).unwrap();
            eprintln!("{width}x{rows} dense {dense}: visited {visited}, bound {bound}");
            assert!(
                visited <= bound,
                "{width}x{rows} dense {dense}: {visited} > {bound}"
            );
            if nine {
                // No pane takes a high one: every wall read once, and no
                // more than the first pane's low places each looked at
                // whole (the cost of the first pane taking one, not the
                // bound's of every pane's places).
                let lefts = i32::from(width) - 2 - 2 * EDGE - SHELF_WIDTH + 1;
                let first = usize::try_from(cells + (SHELF_WIDTH + 5) * lefts).unwrap();
                assert!(visited <= first, "{width}x{rows}: {visited} > {first}");
            }
        }
    }

    /// Non-vacuity for [`a_shelf_hangs_as_high_as_fits`], on 256 fixed
    /// draws of its strategy: each case comes up. Measured (2026-10-11,
    /// with `centred` and one-glyph wall words): high 145, low 55, none
    /// 56, later 45, texted 20, touching 15, kit 9 (blocked 0, see
    /// [`Seen`]); the thresholds are about half. With her living room in
    /// the order (2026-10-12, the user's eighth round): later 46, texted
    /// 21, kit 7 (her sofa's pane decides some the kit's did), living 11,
    /// the rest as before. Touching decides a
    /// choice mostly through `centred` (a hung piece under the middle run
    /// of rise 7); `a_shelf_goes_where_d3_orders_it` pins it by hand too,
    /// and is the guard of record for its place in the order after the
    /// text rule (a touching run first on a row with no text over a clear
    /// one on a row with text: the strategy rarely draws it).
    #[test]
    fn shelves_hang_high_low_and_nowhere() {
        let mut runner = TestRunner::new_with_rng(
            Config::default(),
            TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
        );
        let mut seen = Seen::default();
        for _ in 0..256 {
            let draw = spot_draw().new_tree(&mut runner).unwrap().current();
            seen.add(&hangs_as_high_as_fits(&draw).unwrap());
        }
        eprintln!("{seen:?}");
        assert!(
            seen.high >= 72 && seen.low >= 27 && seen.none >= 28,
            "{seen:?}"
        );
        assert!(seen.texted >= 10 && seen.touching >= 7, "{seen:?}");
        assert!(seen.later >= 22 && seen.kit >= 4, "{seen:?}");
        assert!(seen.living >= 5, "{seen:?}");
    }

    /// The deterministic companion of
    /// [`a_shelf_hangs_as_high_as_fits_by_her_gremlin`]: over 256 fixed
    /// draws, her gremlin decides where it goes often (his pane, or the
    /// place nearest him), and every case still comes up.
    #[test]
    fn shelves_go_up_by_her_gremlin() {
        let mut runner = TestRunner::new_with_rng(
            Config::default(),
            TestRng::deterministic_rng(proptest::test_runner::RngAlgorithm::ChaCha),
        );
        let mut seen = Seen::default();
        for _ in 0..256 {
            let draw = gremlin_spot_draw().new_tree(&mut runner).unwrap().current();
            seen.add(&hangs_as_high_as_fits(&draw).unwrap());
        }
        eprintln!("{seen:?}");
        assert!(
            seen.high >= GREMLIN_SEEN.0
                && seen.low >= GREMLIN_SEEN.1
                && seen.none >= GREMLIN_SEEN.2,
            "{seen:?}"
        );
        assert!(seen.gremlin >= GREMLIN_SEEN.3, "{seen:?}");
    }

    /// [`shelves_go_up_by_her_gremlin`]'s thresholds (high, low, none,
    /// gremlin): about half of what it measured (2026-10-11: high 157,
    /// low 45, none 54, by her gremlin 64; later 51, texted 27, touching
    /// 16; with her living room in the order, 2026-10-12: the same, later
    /// 54, by her living room 1, by the kit's pane 1: his pane decides
    /// first).
    const GREMLIN_SEEN: (usize, usize, usize, usize) = (78, 22, 27, 32);

    /// Her gremlin's pane first (D3, step 7): his pane before the one
    /// she unpacks in and before the panes' order, and in it, the run
    /// nearest his middle; a pane of his that takes none passes to the
    /// next as ever; boxed, he decides nothing. (Up on her shelf there's
    /// nothing to decide: with a shelf of hers, none is chosen at all.)
    #[test]
    fn a_shelf_goes_up_by_her_gremlin() {
        // List 27 rows and Users 13 (a high shelf either), the Playlist 9
        // (a low one at most), 50 wide: Users' inside is 51..99.
        let nooks = [
            (Nook::List, Rect::new(0, 0, 50, 27)),
            (Nook::Users, Rect::new(50, 0, 50, 13)),
            (Nook::Playlist, Rect::new(100, 0, 50, 9)),
        ];
        let buf = panes(&nooks, 150, 28);
        let gremlin = |nook: Nook, offset: u16, boxed: bool| {
            let mut home = Home::default();
            assert!(home.add(Prop {
                anchor: Some(Anchor {
                    side: Side::Left,
                    offset,
                }),
                boxed: boxed.into(),
                ..Prop::new(Furniture::Gremlin, nook, 0, Facing::Right)
            }));
            home
        };
        // No gremlin: List, the first in order, mid-wall.
        let list = spot_on(&Home::default(), &buf, &nooks, Rect::default(), None);
        assert_eq!(list.map(|s| s.nook), Some(Nook::List));
        // In Users, 30 along (columns 81..83, his middle 82): Users, the
        // run nearest him (77..87; mid-wall would be 70..80), ahead of
        // List and of the pane she unpacks in.
        for kit in [None, Some(Nook::List)] {
            let got = spot_on(
                &gremlin(Nook::Users, 30, false),
                &buf,
                &nooks,
                Rect::default(),
                kit,
            );
            assert_eq!(
                got,
                Some(shelf(Nook::Users, Side::Right, 12, 7)),
                "kit {kit:?}"
            );
        }
        let mid = spot_on(&Home::default(), &buf, &[nooks[1]], Rect::default(), None);
        assert_eq!(mid, Some(shelf(Nook::Users, Side::Left, 19, 7)), "mid-wall");
        // In the Playlist, which takes no high shelf: List's high one, as
        // with no gremlin (highest that fits first).
        let got = spot_on(
            &gremlin(Nook::Playlist, 10, false),
            &buf,
            &nooks,
            Rect::default(),
            None,
        );
        assert_eq!(got.map(|s| (s.nook, s.rise)), Some((Nook::List, 7)));
        // Boxed, he's a parcel: List, as with none.
        let got = spot_on(
            &gremlin(Nook::Users, 30, true),
            &buf,
            &nooks,
            Rect::default(),
            None,
        );
        assert_eq!(got.map(|s| s.nook), Some(Nook::List));
    }

    /// `kit_room`'s memo asks [`Home::shelf_spot`] again whenever
    /// anything it reads changes (the user's eighth round: a hash, not
    /// the frame): [`ShelfAsk::basis`] differs with each of its inputs
    /// changed alone (her home: a piece moved, unpacked, a shelf hers,
    /// her door; the frame: a glyph, its ink, its background, its
    /// modifier, a skipped cell, its size; the panes, the chat, the
    /// screen, what's protected, moved text, a made piece, the pane she
    /// unpacks in), and is the same for the same question.
    #[test]
    fn a_change_in_anything_it_reads_asks_again() {
        use tuirealm::ratatui::buffer::CellDiffOption;
        let nooks = [
            (Nook::List, Rect::new(0, 0, 50, 27)),
            (Nook::Users, Rect::new(50, 0, 50, 13)),
        ];
        let buf = panes(&nooks, 100, 28);
        let mut home = Home::default();
        assert!(home.add(Prop::new(Furniture::Sofa, Nook::Users, 300, Facing::Right)));
        assert!(home.add(Prop {
            boxed: true.into(),
            ..Prop::new(Furniture::Gremlin, Nook::List, 600, Facing::Right)
        }));
        let protected = [Rect::new(1, 1, 3, 1)];
        let layer = [(2, 2)];
        let made = [Rect::new(5, 5, 2, 2)];
        let keep_off = KeepOff {
            protected: &protected,
            layer: &layer,
            made: &made,
        };
        let plan = Plan {
            nooks: &nooks,
            chat: Rect::new(0, 20, 10, 5),
            screen: buf.area,
        };
        let ask = ShelfAsk {
            home: &home,
            buf: &buf,
            plan,
            keep_off: &keep_off,
            kit_at: None,
        };
        let basis = ask.basis();
        // The same question, built afresh: the same hash.
        let (home2, buf2) = (home.clone(), buf.clone());
        let same = ShelfAsk {
            home: &home2,
            buf: &buf2,
            ..ask
        };
        assert_eq!(same.basis(), basis, "the same question");
        let mut changed: Vec<(&str, u64)> = Vec::new();
        // Her home.
        let homes: [(&str, Home); 4] = [
            ("a piece moved", {
                let mut h = home.clone();
                h.props[0].at = 700;
                h
            }),
            ("a piece unpacked", {
                let mut h = home.clone();
                assert!(h.unbox(Furniture::Gremlin));
                h
            }),
            (
                "a shelf hers",
                Home {
                    structures: vec![Structure::Shelf(shelf(Nook::List, Side::Left, 9, 7))],
                    ..home.clone()
                },
            ),
            (
                "her door",
                Home {
                    door: Some(DoorWall {
                        strip: Strip::Bottom(Nook::List),
                        side: Side::Left,
                    }),
                    ..home.clone()
                },
            ),
        ];
        for (what, h) in &homes {
            changed.push((what, ShelfAsk { home: h, ..ask }.basis()));
        }
        // The frame.
        let cell = (60, 3);
        let frames: [(&str, Buffer); 6] = [
            ("a glyph", {
                let mut b = buf.clone();
                b[cell].set_symbol("x");
                b
            }),
            ("its ink", {
                let mut b = buf.clone();
                b[cell].fg = Color::Red;
                b
            }),
            ("its background", {
                let mut b = buf.clone();
                b[cell].bg = Color::Blue;
                b
            }),
            ("its modifier", {
                let mut b = buf.clone();
                b[cell].modifier = Modifier::REVERSED;
                b
            }),
            ("a skipped cell", {
                let mut b = buf.clone();
                b[cell].set_diff_option(CellDiffOption::Skip);
                b
            }),
            ("its size", panes(&nooks, 100, 29)),
        ];
        for (what, b) in &frames {
            changed.push((what, ShelfAsk { buf: b, ..ask }.basis()));
        }
        // The plan.
        let fewer = [nooks[0]];
        let moved = [nooks[0], (Nook::Users, Rect::new(50, 1, 50, 13))];
        for (what, plan) in [
            (
                "the panes",
                Plan {
                    nooks: &fewer,
                    ..plan
                },
            ),
            (
                "a pane moved",
                Plan {
                    nooks: &moved,
                    ..plan
                },
            ),
            (
                "the chat",
                Plan {
                    chat: Rect::new(0, 20, 11, 5),
                    ..plan
                },
            ),
            (
                "the screen",
                Plan {
                    screen: Rect::new(0, 0, 100, 29),
                    ..plan
                },
            ),
        ] {
            changed.push((what, ShelfAsk { plan, ..ask }.basis()));
        }
        // What's kept off.
        let other_rect = [Rect::new(1, 1, 4, 1)];
        let other_cell = [(2, 3)];
        for (what, keep_off) in [
            (
                "what's protected",
                KeepOff {
                    protected: &other_rect,
                    ..keep_off
                },
            ),
            (
                "moved text",
                KeepOff {
                    layer: &other_cell,
                    ..keep_off
                },
            ),
            (
                "a made piece",
                KeepOff {
                    made: &other_rect,
                    ..keep_off
                },
            ),
        ] {
            changed.push((
                what,
                ShelfAsk {
                    keep_off: &keep_off,
                    ..ask
                }
                .basis(),
            ));
        }
        // The pane she unpacks in.
        for kit_at in [Some(Nook::List), Some(Nook::Users)] {
            changed.push((
                "the pane she unpacks in",
                ShelfAsk { kit_at, ..ask }.basis(),
            ));
        }
        for (what, other) in changed {
            assert_ne!(other, basis, "{what}: asked again");
        }
    }

    /// Her living room next (the user's eighth round, D3): with no
    /// gremlin waiting on a floor, her sofa's pane before the one she
    /// unpacks in and before the panes' order, mid-wall (the sofa
    /// decides the pane, not the place in it); after her gremlin's;
    /// never over "high before low" (a living room that takes only a
    /// low shelf has a high one elsewhere first); the chat's pane never;
    /// boxed, her sofa decides nothing.
    #[test]
    fn a_shelf_goes_up_in_her_living_room() {
        // As `a_shelf_goes_up_by_her_gremlin`'s: List 27 rows and Users
        // 13 (a high shelf either), the Playlist 9 (a low one at most),
        // 50 wide.
        let nooks = [
            (Nook::List, Rect::new(0, 0, 50, 27)),
            (Nook::Users, Rect::new(50, 0, 50, 13)),
            (Nook::Playlist, Rect::new(100, 0, 50, 9)),
        ];
        let buf = panes(&nooks, 150, 28);
        let home = |pieces: &[(Furniture, Nook, bool)]| {
            let mut home = Home::default();
            for &(item, nook, boxed) in pieces {
                assert!(home.add(Prop {
                    anchor: Some(Anchor {
                        side: Side::Left,
                        offset: 2,
                    }),
                    boxed: boxed.into(),
                    ..Prop::new(item, nook, 0, Facing::Right)
                }));
            }
            home
        };
        let sofa = |nook| (Furniture::Sofa, nook, false);
        // Mid-wall in Users (as its own pane alone puts one), not List,
        // the first in order; and before the pane she unpacks in.
        let mid = spot_on(&Home::default(), &buf, &[nooks[1]], Rect::default(), None);
        assert_eq!(mid, Some(shelf(Nook::Users, Side::Left, 19, 7)), "mid-wall");
        for kit in [None, Some(Nook::List)] {
            let got = spot_on(
                &home(&[sofa(Nook::Users)]),
                &buf,
                &nooks,
                Rect::default(),
                kit,
            );
            assert_eq!(got, mid, "kit {kit:?}");
        }
        // Her gremlin's pane first: List, though her sofa's in Users.
        let got = spot_on(
            &home(&[sofa(Nook::Users), (Furniture::Gremlin, Nook::List, false)]),
            &buf,
            &nooks,
            Rect::default(),
            None,
        );
        assert_eq!(got.map(|s| s.nook), Some(Nook::List), "by her gremlin");
        // In the Playlist, which takes no high shelf: List's high one,
        // as with no sofa (highest that fits first); the Playlist's low
        // one only where no pane takes a high one (List and Users the
        // chat's).
        let living = home(&[sofa(Nook::Playlist)]);
        let got = spot_on(&living, &buf, &nooks, Rect::default(), None);
        assert_eq!(got.map(|s| (s.nook, s.rise)), Some((Nook::List, 7)));
        let chat = nooks[0].1.union(nooks[1].1);
        let got = spot_on(&living, &buf, &nooks, chat, None);
        assert_eq!(got.map(|s| (s.nook, s.rise)), Some((Nook::Playlist, 5)));
        // Her living room the chat's pane: as with no sofa (List).
        let got = spot_on(&home(&[sofa(Nook::Users)]), &buf, &nooks, nooks[1].1, None);
        assert_eq!(got.map(|s| s.nook), Some(Nook::List), "the chat's");
        // Boxed, her sofa's a parcel: List, as with none; and the pane
        // she unpacks in comes next as ever.
        let boxed = home(&[(Furniture::Sofa, Nook::Users, true)]);
        let got = spot_on(&boxed, &buf, &nooks, Rect::default(), None);
        assert_eq!(got.map(|s| s.nook), Some(Nook::List), "boxed");
        let got = spot_on(&boxed, &buf, &nooks, Rect::default(), Some(Nook::Users));
        assert_eq!(
            got.map(|s| s.nook),
            Some(Nook::Users),
            "boxed, kit in Users"
        );
    }
}
