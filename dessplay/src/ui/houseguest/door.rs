//! Her external door (the door batch, D4): where it stands this frame,
//! worked out from her home and the frame, never from where she stands.
//! In its space by her door's wall ([`room::Space`]) when it's free;
//! else at the nearest floor spot where her box meets none of her pieces
//! (the strict fallback); else nowhere. Text never moves it.
//!
//! Also what no new piece may be put on ([`Keep`]): her door's space
//! (and, from the door batch's step 5, the chat pane).

use tuirealm::ratatui::layout::Rect;

use super::room::{self, Home, Plan, Shown, Side, Space};
use super::sprite::{Facing, HEIGHT, WIDTH};
use super::structures;

/// Where her external door stands this frame. Only [`door_place`] makes
/// one (and the tests, [`DoorSpot::at`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct DoorSpot {
    x: i32,
    y: i32,
    at: Set,
    /// Which way she goes out through it.
    out: Facing,
}

/// How her door stands: in its wall (in its space), or face-on at the
/// strict fallback, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Set {
    /// In the wall on `side` of its strip, wall column `wall`.
    Wall { side: Side, wall: i32 },
    /// Face-on on a floor, its space out of reach for the reason given.
    Floor(Fallback),
}

/// Why her door isn't in its space this frame (logged; `Yield` alone is
/// "her pieces fill it").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Fallback {
    /// Her pieces fill the space: it yields this frame.
    Yield,
    /// Something of hers that isn't laid out (a makeshift piece, the
    /// place of the piece in her pocket) is in it.
    Blocked,
    /// Her door's strip is too short or too narrow for a space.
    Short,
    /// The space meets the chat pane.
    Chat,
    /// A protected cell (a focused pane in use, the status bar) is in the
    /// space or its wall.
    Protected,
    /// The frame doesn't draw the wall or the floor where the space
    /// would be (a pane drawn otherwise, a custom layout).
    Unmarked,
    /// No wall of hers: no strip of hers here, none with room.
    NoWall,
}

impl DoorSpot {
    /// Where she stands to go through it (her box's floor cell).
    pub(super) fn spot(self) -> (i32, i32) {
        (self.x, self.y)
    }

    /// How it stands, and why.
    pub(super) fn set(self) -> Set {
        self.at
    }

    /// The way out through it: toward its wall (face-on, toward the
    /// nearer edge of the screen).
    pub(super) fn out(self) -> Facing {
        self.out
    }

    /// The way into the room from it ([`DoorSpot::out`] turned round).
    pub(super) fn into_room(self) -> Facing {
        match self.out {
            Facing::Left => Facing::Right,
            Facing::Right => Facing::Left,
        }
    }

    /// Its wall, if it stands in one.
    pub(super) fn wall(self) -> Option<(Side, i32)> {
        match self.at {
            Set::Wall { side, wall } => Some((side, wall)),
            Set::Floor(_) => None,
        }
    }

    /// Whether it stands where it does because her pieces fill its space.
    pub(super) fn bumped(self) -> bool {
        self.at == Set::Floor(Fallback::Yield)
    }

    /// The cells its beats take, which her box meets while she's at it
    /// (door batch C3): in its wall, its space (the six columns before
    /// the wall, her height and the floor row); face-on, her box at its
    /// spot.
    pub(super) fn room(self) -> Option<Rect> {
        match self.at {
            Set::Wall { side, wall } => {
                let left = match side {
                    Side::Right => wall - room::SPACE,
                    Side::Left => wall + 1,
                };
                let top = self.y - HEIGHT;
                Some(Rect::new(
                    u16::try_from(left).ok()?,
                    u16::try_from(top).ok()?,
                    u16::try_from(room::SPACE).ok()?,
                    u16::try_from(HEIGHT + 1).ok()?,
                ))
            }
            Set::Floor(_) => room::her_box(self.x, self.y),
        }
    }

    /// The box her door takes (her box, standing at its spot).
    #[cfg(test)]
    pub(super) fn rect(self) -> Option<Rect> {
        room::her_box(self.x, self.y)
    }

    /// A spot made by hand, for unit tests with no frame.
    #[cfg(test)]
    pub(super) fn at(x: i32, y: i32, at: Set) -> Self {
        let out = match at {
            Set::Wall {
                side: Side::Left, ..
            } => Facing::Left,
            _ => Facing::Right,
        };
        Self { x, y, at, out }
    }
}

/// What the caller's terrain says of a cell, text-blind: lines and
/// protected cells only (see `Terrain`'s impl).
pub(super) trait Ground {
    /// A floor's line, one she could stand on (protected or not).
    fn ledge(&self, x: i32, y: i32) -> bool;
    /// A plain wall: `│` or `┃`.
    fn wall(&self, x: i32, y: i32) -> bool;
    /// Any box-drawing line.
    fn stroke(&self, x: i32, y: i32) -> bool;
    /// Protected (or off the screen): nothing of hers goes there.
    fn protected(&self, x: i32, y: i32) -> bool;
    /// One of her structures' cells (phase 6a D4): her door never stands
    /// over one.
    fn structure(&self, x: i32, y: i32) -> bool;
}

/// `ground` with the focused pane (`focus`, if the frame protects one)
/// no longer protected: where her door would stand but for the focus.
/// The door she's through follows the frame's door read on this (door
/// batch D6), so a focused pane is never a reason to move it: not over
/// its space, nor over the fallback spot it stood at.
pub(super) struct Ungated<'a, G> {
    pub ground: &'a G,
    pub focus: Option<Rect>,
}

impl<G: Ground> Ground for Ungated<'_, G> {
    fn ledge(&self, x: i32, y: i32) -> bool {
        self.ground.ledge(x, y)
    }

    fn wall(&self, x: i32, y: i32) -> bool {
        self.ground.wall(x, y)
    }

    fn stroke(&self, x: i32, y: i32) -> bool {
        self.ground.stroke(x, y)
    }

    fn protected(&self, x: i32, y: i32) -> bool {
        let focused = self.focus.is_some_and(|focus| {
            let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
                return false;
            };
            focus.contains((ux, uy).into())
        });
        !focused && self.ground.protected(x, y)
    }

    fn structure(&self, x: i32, y: i32) -> bool {
        self.ground.structure(x, y)
    }
}

/// Every cover her door's box mustn't meet: her pieces as laid out
/// (`laid`: before any hiding by text or a focused pane, so a piece
/// under a released pane counts), what she made this visit (`made`), and
/// the place kept for the piece in her pocket (`ghost`). A piece hidden
/// in the chat pane's closet counts too (door batch, D7; the step 6
/// review's choice): it's back where it's laid the moment a room would
/// take it, so her door never stands where a piece may reappear (and a
/// piece straddling the chat's edge is never under her door).
pub(super) fn obstacles(
    laid: &[Shown],
    made: impl IntoIterator<Item = Rect>,
    ghost: Option<Rect>,
) -> Vec<Rect> {
    laid.iter()
        .map(Shown::cover)
        .chain(made)
        .chain(ghost)
        .collect()
}

/// Where her external door stands this frame on `plan`, for `home`, her
/// box meeting none of `obstacles` nor her shelves' runs and margins,
/// read off `ground` (see the module doc). `prev`, the spot it stood at last: a fallback spot that still
/// passes stays (so it never hops). `None`: nowhere at all.
pub(super) fn door_place(
    home: &Home,
    plan: Plan,
    obstacles: &[Rect],
    ground: &impl Ground,
    prev: Option<DoorSpot>,
) -> Option<DoorSpot> {
    let screen = plan.screen;
    let mut target = middle(screen);
    let why = match space(home, plan) {
        Err(why) => why,
        Ok((space, floor)) => {
            let (x, out) = match space.side {
                Side::Right => (space.wall - 3, Facing::Right),
                Side::Left => (space.wall + 3, Facing::Left),
            };
            target = (x, floor);
            match in_space(space, floor, plan, obstacles, ground) {
                Ok(()) => {
                    return Some(DoorSpot {
                        x,
                        y: floor,
                        at: Set::Wall {
                            side: space.side,
                            wall: space.wall,
                        },
                        out,
                    });
                }
                Err(why) => why,
            }
        }
    };
    // Her shelves' runs and margins, shown this frame or not (phase 6a
    // D4's keep-out, by geometry, as for the rest of hers): a fallback
    // spot chosen while a shelf is hidden (text on its row, its pane
    // focused) never meets it when it shows again, so the door never
    // hops for it. The terrain's own refusal (`Ground::structure`) is
    // the second check, for what was written.
    let shelves = structures::kept_out(home, plan.nooks);
    let fits = |(x, y): (i32, i32)| {
        on_floor((x, y), screen, plan.chat, obstacles, ground)
            && room::her_box(x, y).is_some_and(|her| !shelves.iter().any(|s| s.intersects(her)))
    };
    let face = |x: i32| {
        if 2 * (x - i32::from(screen.x)) < i32::from(screen.width) {
            Facing::Left
        } else {
            Facing::Right
        }
    };
    if let Some(prev) = prev
        && matches!(prev.at, Set::Floor(_))
        && fits((prev.x, prev.y))
    {
        return Some(DoorSpot {
            at: Set::Floor(why),
            ..prev
        });
    }
    let (left, top) = (i32::from(screen.x), i32::from(screen.y));
    let (right, bottom) = (i32::from(screen.right()), i32::from(screen.bottom()));
    (top..bottom)
        .flat_map(|y| (left..right).map(move |x| (x, y)))
        .filter(|&spot| fits(spot))
        .min_by_key(|&(x, y)| ((x - target.0).abs() + (y - target.1).abs(), y, x))
        .map(|(x, y)| DoorSpot {
            x,
            y,
            at: Set::Floor(why),
            out: face(x),
        })
}

/// Her door's space on `plan` (the saved wall's, else the one chosen
/// this frame, unsaved) and its floor row, or why there's none.
fn space(home: &Home, plan: Plan) -> Result<(Space, i32), Fallback> {
    let wall = home.wall(plan).ok_or(Fallback::NoWall)?;
    let plans = if home.door == Some(wall) {
        home.extents(plan.nooks)
    } else {
        Home {
            door: Some(wall),
            ..home.clone()
        }
        .extents(plan.nooks)
    };
    let strip = plans
        .into_iter()
        .find(|p| p.strip == wall.strip)
        .ok_or(Fallback::NoWall)?;
    let space = strip.space.ok_or(Fallback::Short)?;
    Ok((space, strip.raw.floor))
}

/// Her door's space's rect on `plan` (see [`space`]), if there is one:
/// what no piece of hers, makeshift or not, may be put on, and no parcel
/// slides through her in.
pub(super) fn space_rect(home: &Home, plan: Plan) -> Option<Rect> {
    space(home, plan).ok().map(|(space, _)| space.rect)
}

/// Whether her door may stand in its `space` (its floor row `floor`)
/// this frame, or why not, in that order: the space kept, nothing of
/// hers in it, clear of the chat, its wall and floor drawn as lines, and
/// nothing in it or its wall protected. Text never fails it.
fn in_space(
    space: Space,
    floor: i32,
    plan: Plan,
    obstacles: &[Rect],
    ground: &impl Ground,
) -> Result<(), Fallback> {
    if !space.kept {
        return Err(Fallback::Yield);
    }
    if obstacles.iter().any(|o| o.intersects(space.rect)) {
        return Err(Fallback::Blocked);
    }
    if room::in_chat(plan.chat, space.rect) {
        return Err(Fallback::Chat);
    }
    let x = match space.side {
        Side::Right => space.wall - 3,
        Side::Left => space.wall + 3,
    };
    let half = WIDTH / 2;
    let marked = (x - half..=x + half).all(|cx| ground.ledge(cx, floor))
        && (floor - HEIGHT..floor).all(|y| ground.wall(space.wall, y))
        && ground.stroke(space.wall, floor);
    if !marked {
        return Err(Fallback::Unmarked);
    }
    let cells = |r: Rect| {
        (i32::from(r.y)..i32::from(r.bottom()))
            .flat_map(move |y| (i32::from(r.x)..i32::from(r.right())).map(move |x| (x, y)))
    };
    let guarded = cells(space.rect).any(|(cx, cy)| ground.protected(cx, cy))
        || (floor - HEIGHT..=floor).any(|y| ground.protected(space.wall, y));
    if guarded {
        return Err(Fallback::Protected);
    }
    Ok(())
}

/// Whether her door may stand face-on at `(x, y)`: her box on `screen`,
/// a floor's line under all of its floor row, none of its cells
/// protected, no plain wall (`│`) above its floor row (it never
/// straddles a pane's border), meeting no obstacle and missing the chat
/// pane. Otherwise text-blind.
fn on_floor(
    (x, y): (i32, i32),
    screen: Rect,
    chat: Rect,
    obstacles: &[Rect],
    ground: &impl Ground,
) -> bool {
    let half = WIDTH / 2;
    let Some(her) = room::her_box(x, y) else {
        return false;
    };
    her.intersection(screen) == her
        && (x - half..=x + half).all(|cx| ground.ledge(cx, y))
        && !room::in_chat(chat, her)
        && !obstacles.iter().any(|o| o.intersects(her))
        && (y - HEIGHT..=y).all(|cy| (x - half..=x + half).all(|cx| !ground.protected(cx, cy)))
        && (y - HEIGHT..y).all(|cy| (x - half..=x + half).all(|cx| !ground.wall(cx, cy)))
        && (y - HEIGHT..=y).all(|cy| (x - half..=x + half).all(|cx| !ground.structure(cx, cy)))
}

/// The middle of `screen`, at its foot: where her door is looked for
/// with no wall to start from.
fn middle(screen: Rect) -> (i32, i32) {
    (
        i32::from(screen.x) + i32::from(screen.width) / 2,
        i32::from(screen.bottom()),
    )
}

/// What no new piece may be put on this frame: her door's space (kept
/// or not, while it exists), and the chat pane. A makeshift piece is
/// judged by [`Keep::for_made`] instead (the space alone).
#[derive(Clone, Debug, Default)]
pub(super) struct Keep {
    chat: Rect,
    spaces: Vec<Rect>,
    /// Her shelves' runs and margins (phase 6a D4): kept from what she
    /// makes only. No real piece can meet one: one standing tops out
    /// below a shelf's lowest rise, and a hung piece keeps out of a low
    /// shelf's block from step 4.
    shelves: Vec<Rect>,
}

impl Keep {
    /// [`Keep`] on `plan` for `home`: her door's space on its wall (the
    /// saved one, else the one chosen this frame), and `plan`'s chat.
    /// Her shelves' rects are pure geometry (no layout, nothing read off
    /// the frame, nothing allocated with none on record), a sliver of
    /// what finding the space (`Home::extents`) costs, so they're taken
    /// eagerly although only [`MadeKeep`] reads them.
    pub(super) fn of(home: &Home, plan: Plan) -> Self {
        Self {
            chat: plan.chat,
            spaces: space(home, plan)
                .ok()
                .map(|(space, _)| space.rect)
                .into_iter()
                .collect(),
            shelves: structures::kept_out(home, plan.nooks),
        }
    }

    /// The chat pane it keeps pieces out of (empty: none).
    pub(super) fn chat(&self) -> Rect {
        self.chat
    }

    /// Whether a piece covering `cover` may not be put there.
    pub(super) fn refuses(&self, cover: Rect) -> bool {
        room::in_chat(self.chat, cover) || self.in_space(cover)
    }

    /// What a makeshift piece may not stand on: her door's space, never
    /// the chat (the user, 2026-10-09: it lasts one visit, like a carried
    /// prop, and in the bundled layout all text she could tear is chat
    /// text).
    pub(super) fn for_made(&self) -> MadeKeep<'_> {
        MadeKeep(self)
    }

    fn in_space(&self, cover: Rect) -> bool {
        self.spaces.iter().any(|s| s.intersects(cover))
    }

    fn made_refuses(&self, cover: Rect) -> bool {
        self.in_space(cover) || self.shelves.iter().any(|s| s.intersects(cover))
    }

    /// Whether a placement puts a piece that showed (in `was`, outside
    /// this) newly in it, where `now` lays it out: moved, and inside. A
    /// piece already inside stays (no new placement), as does one that
    /// doesn't move, or goes to the closet (it's put nowhere).
    pub(super) fn takes_in(&self, was: &[Shown], now: &[Shown]) -> bool {
        was.iter()
            .filter(|w| w.scrap.is_none() && !self.refuses(w.cover()))
            .any(|w| {
                now.iter()
                    .any(|n| n.item == w.item && n.cover() != w.cover() && self.refuses(n.cover()))
            })
    }
}

/// [`Keep`] as a makeshift piece is judged by (see [`Keep::for_made`]):
/// her door's space, kept or not, and not the chat pane. Its own type, so
/// a made piece's site can't be handed the whole [`Keep`].
#[derive(Clone, Copy, Debug)]
pub(super) struct MadeKeep<'a>(&'a Keep);

impl MadeKeep<'_> {
    /// Whether a makeshift piece covering `cover` may not stand there.
    pub(super) fn refuses(self, cover: Rect) -> bool {
        self.0.made_refuses(cover)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A door in its wall goes out toward the wall and in away from it;
    /// only a fallback forced by her pieces filling the space is bumped.
    #[test]
    fn a_door_faces_its_wall_and_only_a_full_space_bumps() {
        let right = DoorSpot::at(
            96,
            12,
            Set::Wall {
                side: Side::Right,
                wall: 99,
            },
        );
        assert_eq!(
            (right.out(), right.into_room()),
            (Facing::Right, Facing::Left)
        );
        let left = DoorSpot::at(
            3,
            16,
            Set::Wall {
                side: Side::Left,
                wall: 0,
            },
        );
        assert_eq!(
            (left.out(), left.into_room()),
            (Facing::Left, Facing::Right)
        );
        assert_eq!(right.wall(), Some((Side::Right, 99)));
        assert_eq!(right.rect(), Some(Rect::new(94, 8, 5, 5)));
        for (why, bumped) in [
            (Fallback::Yield, true),
            (Fallback::Blocked, false),
            (Fallback::Short, false),
            (Fallback::Chat, false),
            (Fallback::Protected, false),
            (Fallback::Unmarked, false),
            (Fallback::NoWall, false),
        ] {
            let at = DoorSpot::at(10, 12, Set::Floor(why));
            assert_eq!(at.bumped(), bumped, "{why:?}");
            assert_eq!(at.wall(), None, "{why:?}");
        }
        assert!(!right.bumped());
    }

    /// A makeshift piece is refused her door's space only (step 6m): the
    /// chat, which refuses every other piece, is no bar to it.
    #[test]
    fn a_made_piece_is_kept_out_of_her_door_space_but_not_the_chat() {
        let keep = Keep {
            chat: Rect::new(70, 0, 30, 14),
            spaces: vec![Rect::new(94, 8, 5, 5)],
            shelves: Vec::new(),
        };
        let in_chat = Rect::new(72, 8, 9, 5);
        let in_space = Rect::new(92, 8, 6, 5);
        let elsewhere = Rect::new(10, 20, 9, 5);
        assert!(keep.refuses(in_chat));
        assert!(keep.refuses(in_space));
        assert!(!keep.refuses(elsewhere));
        assert!(!keep.for_made().refuses(in_chat));
        assert!(keep.for_made().refuses(in_space));
        assert!(!keep.for_made().refuses(elsewhere));
    }

    /// Her shelf's run and margin (phase 6a D4) keep out what she makes,
    /// as her door's space does, whatever of it shows; a real piece
    /// isn't judged by it (none can meet one).
    #[test]
    fn a_made_piece_is_kept_off_her_shelf() {
        let home = Home {
            structures: vec![structures::Structure::Shelf(structures::Shelf {
                nook: room::Nook::Users,
                anchor: room::Anchor {
                    side: Side::Left,
                    offset: 12,
                },
                rise: 7,
                built: 1,
                fresh: None,
            })],
            ..Home::default()
        };
        let nooks = [(room::Nook::Users, Rect::new(50, 0, 50, 13))];
        let plan = Plan {
            nooks: &nooks,
            chat: Rect::default(),
            screen: Rect::new(0, 0, 100, 30),
        };
        let keep = Keep::of(&home, plan);
        // Its run is 63..73 on row 5, its margin two cells past each end.
        for (x, kept) in [(60, false), (61, true), (66, true), (74, true), (75, false)] {
            let cover = Rect::new(x, 3, 1, 3);
            assert_eq!(keep.for_made().refuses(cover), kept, "x {x}");
            assert!(!keep.refuses(cover), "x {x}: a real piece");
        }
        assert!(
            !keep.for_made().refuses(Rect::new(63, 6, 10, 2)),
            "under it"
        );
    }

    /// Ground with a floor's line on every cell of row `floor`, a plain
    /// wall in column `wall` (stroked on the floor row too), nothing
    /// protected but what's off `area`.
    struct Lines {
        area: Rect,
        floor: i32,
        wall: i32,
    }

    impl Ground for Lines {
        fn ledge(&self, _: i32, y: i32) -> bool {
            y == self.floor
        }
        fn wall(&self, x: i32, y: i32) -> bool {
            x == self.wall && y < self.floor
        }
        fn stroke(&self, x: i32, y: i32) -> bool {
            x == self.wall || y == self.floor
        }
        fn protected(&self, x: i32, y: i32) -> bool {
            let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                return true;
            };
            !self.area.contains((x, y).into())
        }
        fn structure(&self, _: i32, _: i32) -> bool {
            false
        }
    }

    /// Her fallback door never stands over her shelf (phase 6a D4): no
    /// cell of her box, its floor row included, is one her structures
    /// wrote, though a shelf is a line and she could stand under it.
    #[test]
    fn her_fallback_door_never_stands_over_her_shelf() {
        struct Shelved {
            lines: Lines,
            /// Row, and columns `from..to`.
            shelf: (i32, i32, i32),
        }
        impl Ground for Shelved {
            fn ledge(&self, x: i32, y: i32) -> bool {
                self.lines.ledge(x, y)
            }
            fn wall(&self, x: i32, y: i32) -> bool {
                self.lines.wall(x, y)
            }
            fn stroke(&self, x: i32, y: i32) -> bool {
                self.lines.stroke(x, y) || self.structure(x, y)
            }
            fn protected(&self, x: i32, y: i32) -> bool {
                self.lines.protected(x, y)
            }
            fn structure(&self, x: i32, y: i32) -> bool {
                let (row, from, to) = self.shelf;
                y == row && (from..to).contains(&x)
            }
        }
        let area = Rect::new(0, 0, 100, 30);
        let half = WIDTH / 2;
        // Rows of her box (`8..=12`), and just over and under it.
        for row in [7, 8, 10, 12, 13] {
            let ground = Shelved {
                lines: Lines {
                    area,
                    floor: 12,
                    wall: 99,
                },
                shelf: (row, 30, 40),
            };
            for x in 20..=50 {
                let meets = (12 - HEIGHT..=12).contains(&row)
                    && (x - half..=x + half).any(|cx| (30..40).contains(&cx));
                assert_eq!(
                    on_floor((x, 12), area, Rect::default(), &[], &ground),
                    !meets,
                    "row {row} x {x}"
                );
            }
        }
    }

    /// Her fallback door never stands where her shelf would be, shown
    /// or not (phase 6a D4's keep-out, by geometry), so it never hops
    /// when a hidden shelf shows: chosen while text is on its margin, or
    /// while its pane is focused (read [`Ungated`], as the door she's
    /// through is), it stays where it stood once the shelf shows, its box
    /// meeting neither the run nor the margin. The only floors are two
    /// runs of text `─` on row 9, under the shelf (nearer where her door
    /// is looked for) and past it; her door's spaces are taken by her
    /// pieces. Through the frame's own terrain, both modes.
    #[test]
    #[allow(clippy::expect_used)]
    fn her_fallback_door_never_hops_for_her_shelf() {
        use super::super::terrain::Terrain;
        let home = Home {
            structures: vec![structures::Structure::Shelf(structures::Shelf {
                nook: room::Nook::Users,
                anchor: room::Anchor {
                    side: Side::Left,
                    offset: 12,
                },
                rise: 7,
                built: 2,
                fresh: None,
            })],
            door: Some(room::DoorWall {
                strip: room::Strip::Bottom(room::Nook::Users),
                side: Side::Left,
            }),
            ..Home::default()
        };
        let users = Rect::new(50, 0, 50, 13);
        let nooks = [(room::Nook::Users, users)];
        let plan = Plan {
            nooks: &nooks,
            chat: Rect::default(),
            screen: Rect::new(0, 0, 100, 30),
        };
        // Its run `63..73` on row 5, and its margin.
        let kept = Rect::new(61, 5, 14, 1);
        let mut real = tuirealm::ratatui::buffer::Buffer::empty(plan.screen);
        tuirealm::ratatui::widgets::Widget::render(
            tuirealm::ratatui::widgets::Block::bordered(),
            users,
            &mut real,
        );
        real.set_string(
            60,
            9,
            "─".repeat(17),
            tuirealm::ratatui::style::Style::new(),
        );
        real.set_string(
            80,
            9,
            "─".repeat(15),
            tuirealm::ratatui::style::Style::new(),
        );
        // Her pieces over every spot on Users' floor, its spaces among them.
        let obstacles = [Rect::new(50, 10, 50, 3)];
        let none = structures::KeepOff {
            protected: &[],
            layer: &[],
            made: &[],
        };
        let texty = {
            let mut texty = real.clone();
            texty.set_string(74, 5, "x", tuirealm::ratatui::style::Style::new());
            texty
        };
        for graphics in [false, true] {
            for focused in [false, true] {
                let at = format!("graphics={graphics} focused={focused}");
                // Hidden: text on its margin, or its pane focused (kept
                // clear: the gated view's, which the shelf is written by).
                let (mut hidden, protected) = if focused {
                    (real.clone(), vec![users])
                } else {
                    (texty.clone(), Vec::new())
                };
                let keep_off = structures::KeepOff {
                    protected: &protected,
                    ..none
                };
                let written = structures::read_ground(&mut hidden, &home, plan, &keep_off);
                assert!(!written.meets(kept), "{at}: hidden");
                let terrain = Terrain::read_with(&hidden, &protected, graphics, &written);
                let ungated = Ungated {
                    ground: &terrain,
                    focus: focused.then_some(users),
                };
                let first = door_place(&home, plan, &obstacles, &ungated, None);
                written.undo(&mut hidden);
                let first = first.expect("a door");
                assert!(matches!(first.set(), Set::Floor(_)), "{at}: {first:?}");
                // Shown.
                let mut shown = real.clone();
                let written = structures::read_ground(&mut shown, &home, plan, &none);
                assert!(written.meets(kept), "{at}: shown");
                let terrain = Terrain::read_with(&shown, &[], graphics, &written);
                let again = door_place(&home, plan, &obstacles, &terrain, Some(first));
                let ungated = Ungated {
                    ground: &terrain,
                    focus: None,
                };
                let through = door_place(&home, plan, &obstacles, &ungated, Some(first));
                written.undo(&mut shown);
                assert_eq!(again, Some(first), "{at}: it stays");
                assert_eq!(through, Some(first), "{at}: read ungated, it stays");
                let (x, y) = first.spot();
                let her = room::her_box(x, y).expect("on the screen");
                assert!(!her.intersects(kept), "{at}: {first:?} meets her shelf");
            }
        }
    }

    /// A face-on fallback door never straddles a pane's border (the
    /// user's answer, 2026-10-08): no cell of her box above the floor row
    /// is a plain wall. The wall's own stroke on the floor row is floor.
    #[test]
    fn a_fallback_door_never_straddles_a_pane_border() {
        let area = Rect::new(0, 0, 100, 30);
        let ground = Lines {
            area,
            floor: 12,
            wall: 50,
        };
        let half = WIDTH / 2;
        for x in 40..=60 {
            let across = (x - half..=x + half).contains(&50);
            assert_eq!(
                on_floor((x, 12), area, Rect::default(), &[], &ground),
                !across,
                "x {x}"
            );
        }
    }

    /// A focused pane over her face-on door's spot moves the frame's
    /// door off it (nothing of hers stands in a focused pane), but read
    /// without the focus ([`Ungated`]) it stays where it stood: what the
    /// door she's through follows, so a focused pane never moves it
    /// (door batch D6), whatever her door fell back for.
    #[test]
    fn a_focused_pane_never_moves_the_door_read_ungated() {
        struct Focused {
            lines: Lines,
            focus: Rect,
        }
        impl Ground for Focused {
            fn ledge(&self, x: i32, y: i32) -> bool {
                self.lines.ledge(x, y)
            }
            fn wall(&self, x: i32, y: i32) -> bool {
                self.lines.wall(x, y)
            }
            fn stroke(&self, x: i32, y: i32) -> bool {
                self.lines.stroke(x, y)
            }
            fn protected(&self, x: i32, y: i32) -> bool {
                self.lines.protected(x, y)
                    || u16::try_from(x)
                        .ok()
                        .zip(u16::try_from(y).ok())
                        .is_some_and(|(x, y)| self.focus.contains((x, y).into()))
            }
            fn structure(&self, x: i32, y: i32) -> bool {
                self.lines.structure(x, y)
            }
        }
        let area = Rect::new(0, 0, 100, 30);
        let focus = Rect::new(40, 0, 20, 30);
        let gated = Focused {
            lines: Lines {
                area,
                floor: 12,
                wall: 99,
            },
            focus,
        };
        let plan = Plan {
            nooks: &[],
            chat: Rect::default(),
            screen: area,
        };
        let home = Home::default();
        let prev = DoorSpot::at(50, 12, Set::Floor(Fallback::NoWall));
        let moved = door_place(&home, plan, &[], &gated, Some(prev));
        assert!(
            moved.is_some_and(|m| m.spot() != prev.spot()),
            "out of the focused pane: {moved:?}"
        );
        assert_eq!(moved.map(DoorSpot::set), Some(Set::Floor(Fallback::NoWall)));
        let ungated = Ungated {
            ground: &gated,
            focus: Some(focus),
        };
        assert_eq!(
            door_place(&home, plan, &[], &ungated, Some(prev)),
            Some(prev)
        );
        // Off the screen still counts.
        let off = DoorSpot::at(1, 12, Set::Floor(Fallback::NoWall));
        assert_ne!(door_place(&home, plan, &[], &ungated, Some(off)), Some(off));
    }

    /// An empty chat pane is no chat pane, wherever it's laid out: a
    /// zero-high (or zero-wide) rect away from the origin refuses no
    /// space, no fallback spot and no piece across its line (ratatui's
    /// `intersects` alone says such a rect meets whatever straddles it).
    #[test]
    fn an_empty_chat_pane_refuses_nothing() {
        let area = Rect::new(0, 0, 100, 30);
        let ground = Lines {
            area,
            floor: 12,
            wall: 99,
        };
        let space = Space {
            side: Side::Right,
            wall: 99,
            rect: Rect::new(93, 8, 6, 5),
            kept: true,
        };
        for chat in [Rect::new(10, 10, 89, 0), Rect::new(95, 0, 0, 30)] {
            let plan = Plan {
                nooks: &[],
                chat,
                screen: area,
            };
            assert_eq!(in_space(space, 12, plan, &[], &ground), Ok(()), "{chat:?}");
            assert!(on_floor((50, 12), area, chat, &[], &ground), "{chat:?}");
            let keep = Keep {
                chat,
                spaces: Vec::new(),
                shelves: Vec::new(),
            };
            assert!(!keep.refuses(Rect::new(48, 8, 9, 5)), "{chat:?}");
            assert!(!keep.refuses(Rect::new(93, 8, 6, 5)), "{chat:?}");
        }
        // A chat that is there still refuses.
        let chat = Rect::new(90, 0, 10, 10);
        let plan = Plan {
            nooks: &[],
            chat,
            screen: area,
        };
        assert_eq!(in_space(space, 12, plan, &[], &ground), Err(Fallback::Chat));
    }
}
