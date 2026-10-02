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
}

impl Furniture {
    /// Every piece, in catalogue order.
    pub const ALL: [Self; 8] = [
        Self::Sofa,
        Self::Tv,
        Self::Bed,
        Self::Desk,
        Self::Lamp,
        Self::Bookshelf,
        Self::Fridge,
        Self::CatBed,
    ];

    /// Its size in cells (columns, rows), standing on a floor.
    pub fn footprint(self) -> (u16, u16) {
        match self {
            Self::Sofa => (9, 3),
            Self::Tv => (6, 4),
            Self::Bed => (10, 3),
            Self::Desk => (7, 3),
            Self::Lamp => (3, 4),
            Self::Bookshelf => (5, 4),
            Self::Fridge => (4, 4),
            Self::CatBed => (4, 2),
        }
    }

    /// The room it belongs in.
    pub(super) fn room(self) -> RoomKind {
        match self {
            Self::Sofa | Self::Tv | Self::CatBed => RoomKind::Living,
            Self::Bed | Self::Desk | Self::Lamp | Self::Bookshelf => RoomKind::Bedroom,
            Self::Fridge => RoomKind::Kitchen,
        }
    }

    /// What she says buying it off the shopping channel.
    pub fn pitch(self) -> &'static str {
        match self {
            Self::Sofa => "A sofa! I'll take it!",
            Self::Tv => "A TV! I'll take it!",
            Self::Bed => "A bed... yes please!",
            Self::Desk => "A desk. For homework.",
            Self::Lamp => "Ooh, a lamp!",
            Self::Bookshelf => "Books! I'll take it!",
            Self::Fridge => "A fridge... for snacks!",
            Self::CatBed => "A cat bed! For a cat!",
        }
    }

    /// A short name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sofa => "sofa",
            Self::Tv => "TV",
            Self::Bed => "bed",
            Self::Desk => "desk",
            Self::Lamp => "lamp",
            Self::Bookshelf => "bookshelf",
            Self::Fridge => "fridge",
            Self::CatBed => "cat bed",
        }
    }

    /// Its part in `art/props.svg`.
    pub fn art_id(self) -> &'static str {
        match self {
            Self::Sofa => "sofa",
            Self::Tv => "tv",
            Self::Bed => "bed",
            Self::Desk => "desk",
            Self::Lamp => "lamp",
            Self::Bookshelf => "bookshelf",
            Self::Fridge => "fridge",
            Self::CatBed => "cat-bed",
        }
    }

    /// The ASCII drawing, facing right: one string per row, each exactly
    /// as wide as the footprint. Also the noise her goodbye bursts the
    /// line art into.
    fn ascii(self) -> &'static [&'static str] {
        match self {
            Self::Sofa => &[" .-----. ", "(|_____|)", " '     ' "],
            Self::Tv => &["  \\/  ", ".----.", "|[  ]|", "|_::_|"],
            Self::Bed => &["|__       ", "|oo~~~~~~|", "|========|"],
            Self::Desk => &[" = u _/", "_______", "|   |=|"],
            Self::Lamp => &[" _ ", "/_\\", " | ", "_|_"],
            Self::Bookshelf => &["_____", "|IlI|", "|lII|", "|___|"],
            Self::Fridge => &["____", "| .|", "|--|", "|_.|"],
            Self::CatBed => &["    ", "\\__/"],
        }
    }
}

/// The glyph at `(dx, dy)` of `item`'s ASCII drawing facing `facing`, if
/// that cell is drawn (spaces are not).
pub(super) fn glyph(item: Furniture, facing: Facing, dx: u16, dy: u16) -> Option<char> {
    let (cols, _) = item.footprint();
    let row = item.ascii().get(usize::from(dy))?;
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
    /// What each piece is for.
    pub fn of(item: Furniture) -> &'static [Use] {
        match item {
            Furniture::Sofa => &[Use::Lounge, Use::Nap],
            Furniture::Tv => &[Use::Watch],
            Furniture::Bed => &[Use::Sleep],
            Furniture::Desk => &[Use::Homework],
            Furniture::Lamp => &[],
            Furniture::Bookshelf => &[Use::Read],
            Furniture::Fridge => &[Use::Snack],
            Furniture::CatBed => &[Use::Pet],
        }
    }

    /// Whether she uses it from in it (sits on it, lies in it), rather
    /// than from beside it.
    pub fn inside(self) -> bool {
        !matches!(self, Use::Watch | Use::Read | Use::Snack | Use::Pet)
    }
}

/// Where she goes to use a piece: she walks to `x` on its floor (in
/// front of it, or beside the TV) and faces `facing`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Seat {
    pub what: Use,
    pub item: Furniture,
    pub x: i32,
    pub y: i32,
    pub facing: Facing,
    /// It's makeshift furniture she made of text.
    pub makeshift: bool,
}

/// Which room a piece belongs to. Panes are her rooms: all the pieces of
/// one room stand in the same pane, and each pane holds one room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub(super) enum RoomKind {
    Living,
    Bedroom,
    Kitchen,
}

/// A piece she owns, `at` thousandths of the way along its room's
/// floor; `boxed` until she unpacks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Prop {
    pub item: Furniture,
    pub at: u16,
    pub facing: Facing,
    pub boxed: bool,
}

/// A prop as placed in this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub item: Furniture,
    pub facing: Facing,
    /// Still in its delivery box (drawn as the box).
    pub boxed: bool,
    /// The pane its room is in (none for makeshift pieces, which belong
    /// to no room).
    pub nook: Option<Nook>,
    /// Its leftmost column.
    pub left: i32,
    /// The floor row it stands on (just below its footprint).
    pub floor: i32,
    /// Makeshift, crumpled out of torn-off text (never boxed).
    pub scrap: Option<super::scrap::Scrap>,
}

impl Shown {
    /// Its size in cells (columns, rows).
    pub fn size(&self) -> (u16, u16) {
        match self.scrap {
            Some(_) => super::scrap::footprint(self.item),
            None => self.item.footprint(),
        }
    }

    /// What she can do with it: unpack it while it's boxed, crumple it
    /// into shape while it's makeshift and still a heap; once it's in
    /// shape, a makeshift piece is for what the real one is.
    pub fn uses(&self) -> &'static [Use] {
        match (self.boxed, self.scrap) {
            (true, _) => &[Use::Unpack],
            (false, Some(scrap)) if !scrap.done() => &[Use::Crumple],
            (false, _) => Use::of(self.item),
        }
    }

    /// Its footprint, above the floor.
    pub fn rect(&self) -> Rect {
        let (cols, rows) = self.size();
        Rect::new(
            self.left as u16,
            (self.floor - i32::from(rows)) as u16,
            cols,
            rows,
        )
    }

    /// Its footprint and the floor beneath it: nothing else may cover
    /// either.
    pub fn cover(&self) -> Rect {
        let rect = self.rect();
        Rect::new(rect.x, rect.y, rect.width, rect.height + 1)
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
            Use::Lounge | Use::Nap => (mirrored(4), self.facing),
            // Head at the headboard end.
            Use::Sleep => (mirrored(3), self.facing),
            // On a stool just past the desk's front, facing it.
            Use::Homework => (mirrored(7), flip(self.facing)),
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
            x,
            y: self.floor,
            facing,
            makeshift: self.scrap.is_some(),
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
        (0..rows).flat_map(move |dy| {
            (0..cols).map(move |dx| {
                let glyph = if let Some(scrap) = self.scrap {
                    scrap.cell(self.item, self.facing, dx, dy).map(|(c, _)| c)
                } else if self.boxed {
                    parcel_glyph(cols, rows, dx, dy)
                } else {
                    glyph(self.item, self.facing, dx, dy)
                };
                (
                    self.left + i32::from(dx),
                    self.floor - i32::from(rows) + i32::from(dy),
                    glyph,
                )
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

/// Everything she owns, and which pane each room is in.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Home {
    pub props: Vec<Prop>,
    pub rooms: Vec<(RoomKind, Nook)>,
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

    /// The pane `kind` is in, if she has furnished it.
    pub fn nook_of(&self, kind: RoomKind) -> Option<Nook> {
        self.rooms
            .iter()
            .find(|&&(k, _)| k == kind)
            .map(|&(_, nook)| nook)
    }

    /// Place her rooms in this frame. A room whose pane still holds all
    /// its pieces stays, each piece showing if its cells are free (else
    /// it's in the closet this frame). A room whose pane is gone or too
    /// small moves, whole, to a free pane where every piece fits; with
    /// none, the whole room is in the closet. `blocked` cells are never
    /// covered by a piece (protected rectangles, moved text).
    pub fn resolve(
        &mut self,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> Vec<Shown> {
        let mut shown: Vec<Shown> = Vec::new();
        for index in 0..self.rooms.len() {
            let Some(&(kind, home)) = self.rooms.get(index) else {
                continue;
            };
            let pieces: Vec<Prop> = self
                .props
                .iter()
                .filter(|p| p.item.room() == kind)
                .copied()
                .collect();
            if let Some(layout) = layout(&pieces, home, nooks) {
                for at in layout {
                    if fits(buf, &at, &|x, y| free(&shown, blocked, x, y)) {
                        shown.push(at);
                    }
                }
                continue;
            }
            let moved = nooks
                .iter()
                .filter(|&&(nook, _)| !self.rooms.iter().any(|&(_, n)| n == nook))
                .find_map(|&(nook, _)| {
                    let layout = layout(&pieces, nook, nooks)?;
                    layout
                        .iter()
                        .all(|at| fits(buf, at, &|x, y| free(&shown, blocked, x, y)))
                        .then_some((nook, layout))
                });
            if let Some((nook, layout)) = moved {
                tracing::info!(?kind, from = ?home, to = ?nook, "houseguest: a room moved");
                if let Some(room) = self.rooms.get_mut(index) {
                    room.1 = nook;
                }
                shown.extend(layout);
            }
        }
        shown
    }

    /// Where `item` could go this frame: in its room's pane, or, for a
    /// room she hasn't furnished yet, any pane no other room is in —
    /// chosen at random among spots that fit, clear of what's `shown`.
    pub fn spot(
        &self,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        item: Furniture,
        rng: &mut Rng,
    ) -> Option<(Nook, Prop)> {
        let facing = if rng.below(2) == 0 {
            Facing::Right
        } else {
            Facing::Left
        };
        let bound = self.nook_of(item.room());
        let spots: Vec<(Nook, Prop)> = nooks
            .iter()
            .filter(|&&(nook, _)| match bound {
                Some(home) => nook == home,
                None => !self.rooms.iter().any(|&(_, n)| n == nook),
            })
            .flat_map(|&(nook, _)| {
                (0..=10).map(move |step| {
                    (
                        nook,
                        Prop {
                            item,
                            at: step * 100,
                            facing,
                            boxed: false,
                        },
                    )
                })
            })
            .filter(|&(nook, prop)| {
                place(prop, nook, nooks).is_some_and(|at| {
                    let clear = |x: i32, y: i32| free(shown, blocked, x, y);
                    fits(buf, &at, &clear) && roomy(buf, &at, &clear)
                })
            })
            .collect();
        spots.get(rng.below(spots.len() as u64) as usize).copied()
    }

    /// Take ownership of `prop`: into its room, or, for a new room, into
    /// `nook` — unless another room is already there (then she doesn't
    /// take it, and this returns false).
    pub fn add(&mut self, nook: Nook, prop: Prop) -> bool {
        let kind = prop.item.room();
        if self.nook_of(kind).is_none() {
            if self.rooms.iter().any(|&(_, n)| n == nook) {
                return false;
            }
            self.rooms.push((kind, nook));
        }
        self.props.push(prop);
        true
    }
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

/// Where `pieces` would stand in `nook`, if its pane is there and big
/// enough to hold every one of them without overlapping.
fn layout(pieces: &[Prop], nook: Nook, nooks: &[(Nook, Rect)]) -> Option<Vec<Shown>> {
    let placed: Vec<Shown> = pieces
        .iter()
        .map(|&prop| place(prop, nook, nooks))
        .collect::<Option<_>>()?;
    let apart = placed.iter().enumerate().all(|(i, a)| {
        placed
            .iter()
            .skip(i + 1)
            .all(|b| !a.rect().intersects(b.rect()))
    });
    apart.then_some(placed)
}

/// Where `prop` would stand in `nook`, if its pane is there and wide and
/// tall enough to hold it inside its border.
fn place(prop: Prop, nook: Nook, nooks: &[(Nook, Rect)]) -> Option<Shown> {
    let &(_, rect) = nooks.iter().find(|(n, _)| *n == nook)?;
    let (cols, rows) = prop.item.footprint();
    if rect.width < cols + 2 || rect.height < rows + 2 {
        return None;
    }
    let span = i32::from(rect.width - 2 - cols);
    Some(Shown {
        item: prop.item,
        facing: prop.facing,
        boxed: prop.boxed,
        nook: Some(nook),
        left: i32::from(rect.x) + 1 + span * i32::from(prop.at.min(1000)) / 1000,
        floor: i32::from(rect.bottom()) - 1,
        scrap: None,
    })
}

/// Whether `at` stands on unbroken line glyphs with only blank, `clear`
/// cells in its footprint (image cells and wide glyphs' halves are
/// never blank).
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
    let floor = (at.left..at.left + i32::from(cols)).all(|x| {
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
            let (cols, rows) = item.footprint();
            let art = item.ascii();
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
        Prop {
            item,
            at,
            facing: Facing::Right,
            boxed: false,
        }
    }

    fn home(nook: Nook, props: &[Prop]) -> Home {
        let mut home = Home::default();
        for &p in props {
            assert!(home.add(nook, p));
        }
        home
    }

    #[test]
    fn a_prop_stands_on_its_panes_floor() {
        let buf = pane(&USERS);
        let nooks = [(Nook::Users, buf.area)];
        let shown =
            home(Nook::Users, &[prop(Furniture::Sofa, 0)]).resolve(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].rect(), Rect::new(1, 2, 9, 3));
        let far =
            home(Nook::Users, &[prop(Furniture::Sofa, 1000)]).resolve(&buf, &nooks, &|_, _| false);
        assert_eq!(far[0].rect().right(), 17);
    }

    #[test]
    fn text_or_a_blocked_cell_sends_it_to_the_closet() {
        let mut rows = USERS;
        rows[3] = "│   hi           │";
        let buf = pane(&rows);
        let nooks = [(Nook::Users, buf.area)];
        let mut room = home(Nook::Users, &[prop(Furniture::Sofa, 0)]);
        assert!(room.resolve(&buf, &nooks, &|_, _| false).is_empty());
        let clean = pane(&USERS);
        assert!(
            room.resolve(&clean, &nooks, &|x, y| (x, y) == (5, 4))
                .is_empty()
        );
        assert!(
            room.resolve(&clean, &[], &|_, _| false).is_empty(),
            "no pane"
        );
        assert_eq!(
            room.nook_of(RoomKind::Living),
            Some(Nook::Users),
            "text doesn't move a room"
        );
    }

    #[test]
    fn a_room_whose_pane_is_too_small_moves_whole_or_not_at_all() {
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
        let big = pane(&[
            "┌Users─────────────────────────────────────────┐",
            "│                                              │",
            "│                                              │",
            "│                                              │",
            "│                                              │",
            "└──────────────────────────────────────────────┘",
        ]);
        assert_eq!(
            room.resolve(&big, &[(Nook::Users, wide)], &|_, _| false)
                .len(),
            2
        );
        // Users shrinks: both no longer fit there, so the living room
        // moves to the Playlist, together.
        let nooks = [(Nook::Users, users), (Nook::Playlist, playlist)];
        let shown = room.resolve(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 2, "{shown:?}");
        assert!(shown.iter().all(|s| s.nook == Some(Nook::Playlist)));
        assert_eq!(room.nook_of(RoomKind::Living), Some(Nook::Playlist));
        // Nowhere holds both: the whole room is in the closet.
        let shown = room.resolve(&buf, &[(Nook::Users, users)], &|_, _| false);
        assert!(shown.is_empty(), "{shown:?}");
    }

    #[test]
    fn a_new_piece_joins_its_room_and_rooms_keep_to_their_own_panes() {
        let big = |title: &str| {
            let mut rows = vec![format!(
                "┌{title}{}┐",
                "─".repeat(38 - title.chars().count())
            )];
            rows.extend((0..5).map(|_| format!("│{}│", " ".repeat(38))));
            rows.push(format!("└{}┘", "─".repeat(38)));
            rows
        };
        let mut rows = big("Users");
        for (row, other) in rows.iter_mut().zip(big("Playlist")) {
            row.push_str(&other);
        }
        let buf = Buffer::with_lines(rows.iter().map(String::as_str));
        let nooks = [
            (Nook::Users, Rect::new(0, 0, 40, 7)),
            (Nook::Playlist, Rect::new(40, 0, 40, 7)),
        ];
        let mut room = home(Nook::Users, &[prop(Furniture::Sofa, 0)]);
        let mut rng = Rng(5);
        for seed in 0..20 {
            rng.0 = seed;
            let shown = room.resolve(&buf, &nooks, &|_, _| false);
            let (nook, _) = room
                .spot(&buf, &nooks, &shown, &|_, _| false, Furniture::Tv, &mut rng)
                .expect("room for a TV");
            assert_eq!(nook, Nook::Users, "the TV goes with the sofa");
            let (nook, _) = room
                .spot(
                    &buf,
                    &nooks,
                    &shown,
                    &|_, _| false,
                    Furniture::Bed,
                    &mut rng,
                )
                .expect("room for a bed");
            assert_eq!(nook, Nook::Playlist, "the bedroom gets a pane of its own");
        }
    }

    #[test]
    fn a_left_facing_drawing_is_mirrored() {
        assert_eq!(glyph(Furniture::Desk, Facing::Right, 6, 0), Some('/'));
        assert_eq!(glyph(Furniture::Desk, Facing::Left, 0, 0), Some('\\'));
        assert_eq!(glyph(Furniture::Desk, Facing::Left, 6, 0), None);
    }
}
