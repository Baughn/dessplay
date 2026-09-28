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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Furniture {
    /// A two-seat sofa (naps, TV from the sofa).
    Sofa,
    /// A CRT on a cabinet; the shopping channel plays on it.
    Tv,
    /// A single bed (proper sleep).
    Bed,
    /// A study desk (homework).
    Desk,
}

impl Furniture {
    /// Every piece, in catalogue order.
    pub const ALL: [Self; 4] = [Self::Sofa, Self::Tv, Self::Bed, Self::Desk];

    /// Its size in cells (columns, rows), standing on a floor.
    pub fn footprint(self) -> (u16, u16) {
        match self {
            Self::Sofa => (9, 3),
            Self::Tv => (6, 4),
            Self::Bed => (10, 3),
            Self::Desk => (7, 3),
        }
    }

    /// A short name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sofa => "sofa",
            Self::Tv => "TV",
            Self::Bed => "bed",
            Self::Desk => "desk",
        }
    }

    /// Its part in `art/props.svg`.
    pub fn art_id(self) -> &'static str {
        match self {
            Self::Sofa => "sofa",
            Self::Tv => "tv",
            Self::Bed => "bed",
            Self::Desk => "desk",
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Nook {
    /// The List (series), when short.
    List,
    /// The Users pane.
    Users,
    /// The Playlist pane.
    Playlist,
}

/// Where a prop stands: on `nook`'s floor, `at` thousandths of the way
/// along it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Anchor {
    pub nook: Nook,
    pub at: u16,
    pub facing: Facing,
}

/// A piece she owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Prop {
    pub item: Furniture,
    pub anchor: Anchor,
}

/// A prop as placed in this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Shown {
    pub item: Furniture,
    pub facing: Facing,
    /// Its leftmost column.
    pub left: i32,
    /// The floor row it stands on (just below its footprint).
    pub floor: i32,
}

impl Shown {
    /// Its footprint, above the floor.
    pub fn rect(&self) -> Rect {
        let (cols, rows) = self.item.footprint();
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

    /// Every footprint cell, with its ASCII glyph if drawn.
    pub fn cells(&self) -> impl Iterator<Item = (i32, i32, Option<char>)> + '_ {
        let (cols, rows) = self.item.footprint();
        (0..rows).flat_map(move |dy| {
            (0..cols).map(move |dx| {
                (
                    self.left + i32::from(dx),
                    self.floor - i32::from(rows) + i32::from(dy),
                    glyph(self.item, self.facing, dx, dy),
                )
            })
        })
    }
}

/// Everything she owns.
#[derive(Clone, Debug, Default)]
pub(super) struct Room {
    pub props: Vec<Prop>,
}

impl Room {
    /// Whether she owns `item`.
    pub fn owns(&self, item: Furniture) -> bool {
        self.props.iter().any(|p| p.item == item)
    }

    /// Place every prop that fits this frame, in order; the rest are in
    /// the closet. `blocked` cells are never covered (protected
    /// rectangles, moved text, her own box).
    pub fn resolve(
        &self,
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        blocked: &dyn Fn(i32, i32) -> bool,
    ) -> Vec<Shown> {
        let mut shown: Vec<Shown> = Vec::new();
        for prop in &self.props {
            let Some(at) = place(prop.item, prop.anchor, nooks) else {
                continue;
            };
            let clear = |x: i32, y: i32| {
                !blocked(x, y)
                    && !shown
                        .iter()
                        .any(|s| s.rect().contains((x as u16, y as u16).into()))
            };
            if fits(buf, &at, &clear) {
                shown.push(at);
            }
        }
        shown
    }

    /// A spot where `item` fits this frame, chosen at random among the
    /// quiet panes' floors (clear of what's already shown).
    pub fn spot(
        buf: &Buffer,
        nooks: &[(Nook, Rect)],
        shown: &[Shown],
        blocked: &dyn Fn(i32, i32) -> bool,
        item: Furniture,
        rng: &mut Rng,
    ) -> Option<Anchor> {
        let clear = |x: i32, y: i32| {
            !blocked(x, y)
                && !shown
                    .iter()
                    .any(|s| s.rect().contains((x as u16, y as u16).into()))
        };
        let facing = if rng.below(2) == 0 {
            Facing::Right
        } else {
            Facing::Left
        };
        let spots: Vec<Anchor> = nooks
            .iter()
            .flat_map(|&(nook, _)| {
                (0..=10).map(move |step| Anchor {
                    nook,
                    at: step * 100,
                    facing,
                })
            })
            .filter(|&anchor| place(item, anchor, nooks).is_some_and(|at| fits(buf, &at, &clear)))
            .collect();
        spots.get(rng.below(spots.len() as u64) as usize).copied()
    }
}

/// Where `item` at `anchor` would stand, if its pane is there and wide
/// and tall enough to hold it inside its border.
fn place(item: Furniture, anchor: Anchor, nooks: &[(Nook, Rect)]) -> Option<Shown> {
    let &(_, rect) = nooks.iter().find(|(nook, _)| *nook == anchor.nook)?;
    let (cols, rows) = item.footprint();
    if rect.width < cols + 2 || rect.height < rows + 2 {
        return None;
    }
    let span = i32::from(rect.width - 2 - cols);
    Some(Shown {
        item,
        facing: anchor.facing,
        left: i32::from(rect.x) + 1 + span * i32::from(anchor.at.min(1000)) / 1000,
        floor: i32::from(rect.bottom()) - 1,
    })
}

/// Whether `at` stands on unbroken line glyphs with only blank, `clear`
/// cells in its footprint (image cells and wide glyphs' halves are
/// never blank).
fn fits(buf: &Buffer, at: &Shown, clear: &dyn Fn(i32, i32) -> bool) -> bool {
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
    let (cols, _) = at.item.footprint();
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

    #[test]
    fn a_prop_stands_on_its_panes_floor() {
        let buf = pane(&USERS);
        let nooks = [(Nook::Users, buf.area)];
        let room = Room {
            props: vec![Prop {
                item: Furniture::Sofa,
                anchor: Anchor {
                    nook: Nook::Users,
                    at: 0,
                    facing: Facing::Right,
                },
            }],
        };
        let shown = room.resolve(&buf, &nooks, &|_, _| false);
        assert_eq!(shown.len(), 1);
        assert_eq!(shown[0].rect(), Rect::new(1, 2, 9, 3));
        let far = Room {
            props: vec![Prop {
                anchor: Anchor {
                    at: 1000,
                    ..room.props[0].anchor
                },
                ..room.props[0]
            }],
        };
        assert_eq!(
            far.resolve(&buf, &nooks, &|_, _| false)[0].rect().right(),
            17
        );
    }

    #[test]
    fn text_or_a_blocked_cell_sends_it_to_the_closet() {
        let mut rows = USERS;
        rows[3] = "│   hi           │";
        let buf = pane(&rows);
        let nooks = [(Nook::Users, buf.area)];
        let prop = Prop {
            item: Furniture::Sofa,
            anchor: Anchor {
                nook: Nook::Users,
                at: 0,
                facing: Facing::Right,
            },
        };
        let room = Room { props: vec![prop] };
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
    }

    #[test]
    fn props_never_overlap() {
        let buf = pane(&USERS);
        let nooks = [(Nook::Users, buf.area)];
        let at = |at| Prop {
            item: Furniture::Sofa,
            anchor: Anchor {
                nook: Nook::Users,
                at,
                facing: Facing::Right,
            },
        };
        let room = Room {
            props: vec![at(0), at(500)],
        };
        assert_eq!(room.resolve(&buf, &nooks, &|_, _| false).len(), 1);
    }

    #[test]
    fn a_left_facing_drawing_is_mirrored() {
        assert_eq!(glyph(Furniture::Desk, Facing::Right, 6, 0), Some('/'));
        assert_eq!(glyph(Furniture::Desk, Facing::Left, 0, 0), Some('\\'));
        assert_eq!(glyph(Furniture::Desk, Facing::Left, 6, 0), None);
    }
}
