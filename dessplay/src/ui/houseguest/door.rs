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
    #[cfg(test)]
    pub(super) fn wall(self) -> Option<(Side, i32)> {
        match self.at {
            Set::Wall { side, wall } => Some((side, wall)),
            Set::Floor(_) => None,
        }
    }

    /// Whether it stands where it does because her pieces fill its space.
    #[allow(dead_code)] // DoorClear's trigger (the door batch, step 6)
    pub(super) fn bumped(self) -> bool {
        self.at == Set::Floor(Fallback::Yield)
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
}

/// Every cover her door's box mustn't meet: her pieces as laid out
/// (`laid`: before any hiding by text or a focused pane, so a piece
/// under a released pane counts), what she made this visit (`made`), and
/// the place kept for the piece in her pocket (`ghost`).
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
/// box meeting none of `obstacles`, read off `ground` (see the module
/// doc). `prev`, the spot it stood at last: a fallback spot that still
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
    let fits = |(x, y): (i32, i32)| on_floor((x, y), screen, plan.chat, obstacles, ground);
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
/// protected, meeting no obstacle and missing the chat pane. Text-blind.
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
/// or not, while it exists), and the chat pane.
#[derive(Clone, Debug, Default)]
pub(super) struct Keep {
    chat: Rect,
    spaces: Vec<Rect>,
}

impl Keep {
    /// [`Keep`] on `plan` for `home`: her door's space on its wall (the
    /// saved one, else the one chosen this frame), and `plan`'s chat.
    pub(super) fn of(home: &Home, plan: Plan) -> Self {
        Self {
            chat: plan.chat,
            spaces: space(home, plan)
                .ok()
                .map(|(space, _)| space.rect)
                .into_iter()
                .collect(),
        }
    }

    /// Whether a piece covering `cover` may not be put there.
    pub(super) fn refuses(&self, cover: Rect) -> bool {
        room::in_chat(self.chat, cover) || self.spaces.iter().any(|s| s.intersects(cover))
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
