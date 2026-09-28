//! The screen as terrain, read from the finished frame: horizontal
//! box-drawing runs are floors, vertical borders are poles, and the
//! protected rectangles are solid. Deriving it from cells rather than
//! pane rectangles makes any user layout walkable — including chat day
//! separators, which scroll away under her.

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::{Position, Rect};

use super::sprite::{HEIGHT, WIDTH};

const HALF: i32 = WIDTH / 2;
/// Platforms narrower than this many standing positions are ignored.
const MIN_PLATFORM: i32 = 3;

fn box_drawing(symbol: &str) -> Option<char> {
    let mut chars = symbol.chars();
    let c = chars.next()?;
    (chars.next().is_none() && ('\u{2500}'..='\u{2570}').contains(&c)).then_some(c)
}

/// A glyph with a vertical stroke only.
fn vertical_only(c: char) -> bool {
    matches!(
        c,
        '│' | '┃'
            | '║'
            | '╎'
            | '╏'
            | '┆'
            | '┇'
            | '┊'
            | '┋'
            | '╵'
            | '╷'
            | '╹'
            | '╻'
            | '╽'
            | '╿'
    )
}

/// A glyph with a horizontal stroke only.
fn horizontal_only(c: char) -> bool {
    matches!(
        c,
        '─' | '━'
            | '═'
            | '┄'
            | '┅'
            | '┈'
            | '┉'
            | '╌'
            | '╍'
            | '╴'
            | '╶'
            | '╸'
            | '╺'
            | '╼'
            | '╾'
    )
}

fn blank(symbol: &str) -> bool {
    symbol.trim().is_empty()
}

fn ledge(symbol: &str) -> bool {
    box_drawing(symbol).is_some_and(|c| !vertical_only(c))
}

fn pole(symbol: &str) -> bool {
    box_drawing(symbol).is_some_and(|c| !horizontal_only(c))
}

/// A walkable stretch: she can stand centred on any `x` in `x0..=x1`
/// with her feet on row `y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Platform {
    pub y: i32,
    pub x0: i32,
    pub x1: i32,
    /// The ledge itself ends just past `x0` / `x1` (a drop-off she can
    /// peer over), rather than headroom running out (a wall).
    pub edge_left: bool,
    pub edge_right: bool,
}

impl Platform {
    pub fn contains(&self, x: i32) -> bool {
        (self.x0..=self.x1).contains(&x)
    }

    pub fn clamp(&self, x: i32) -> i32 {
        x.clamp(self.x0, self.x1)
    }
}

/// How she gets from one platform to another.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Route {
    /// Climb straight up or down at `x`, beside a pole.
    Climb,
    /// Walk to the edge at `x`, hop off to `over`, and fall.
    Drop { over: i32 },
}

/// A connection from platform `from` (standing at `x`) to platform `to`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Link {
    pub from: usize,
    pub to: usize,
    pub x: i32,
    pub route: Route,
    /// The pole's column, for a climb (her hands go on it).
    pub pole: i32,
}

/// The walkable world of one frame.
#[derive(Clone, Debug, Default)]
pub(super) struct Terrain {
    width: i32,
    height: i32,
    /// Cells her body may occupy.
    open: Vec<bool>,
    pub platforms: Vec<Platform>,
    pub links: Vec<Link>,
}

impl Terrain {
    /// Read the terrain from a finished frame. `protected` rectangles
    /// (and image cells) are solid: her body never enters them, though
    /// she may stand on a ledge that lies inside one.
    ///
    /// `graphics` is the line-art mode: her image replaces the cells it
    /// covers, so her body only enters cells that are blank or lines her
    /// image redraws (text is never hidden behind her), and she only
    /// stands on lines it can redraw.
    pub fn read(buf: &Buffer, protected: &[Rect], graphics: bool) -> Self {
        let area = buf.area;
        let (width, height) = (i32::from(area.width), i32::from(area.height));
        let mut open = Vec::with_capacity((width * height).max(0) as usize);
        let mut ledges = Vec::with_capacity(open.capacity());
        let mut poles = Vec::with_capacity(open.capacity());
        for y in 0..area.height {
            // The cell after a wide glyph looks blank but is half of it.
            let mut after_wide = false;
            for x in 0..area.width {
                let position = Position::new(area.x + x, area.y + y);
                let cell = buf.cell(position);
                let trailing = std::mem::replace(
                    &mut after_wide,
                    cell.is_some_and(|c| super::cells::width(c) > 1),
                );
                let skip = cell.is_none_or(super::cells::untouchable);
                let symbol = cell.map_or(" ", |cell| cell.symbol());
                let redrawable = || {
                    symbol
                        .chars()
                        .next()
                        .is_some_and(|c| super::graphics::strokes(c).is_some())
                };
                let text_ok = !graphics || (blank(symbol) && !trailing) || redrawable();
                open.push(!skip && text_ok && !protected.iter().any(|r| r.contains(position)));
                let floor = ledge(symbol) && (!graphics || redrawable());
                ledges.push(!skip && floor);
                poles.push(!skip && pole(symbol));
            }
        }
        let mut terrain = Self {
            width,
            height,
            open,
            platforms: Vec::new(),
            links: Vec::new(),
        };
        let at = |grid: &[bool], x: i32, y: i32| {
            (0..width).contains(&x)
                && (0..height).contains(&y)
                && grid.get((y * width + x) as usize).copied().unwrap_or(false)
        };
        for y in 0..height {
            let mut x = 0;
            while x < width {
                if !at(&ledges, x, y) {
                    x += 1;
                    continue;
                }
                let start = x;
                while at(&ledges, x, y) {
                    x += 1;
                }
                terrain.add_platforms(y, start, x - 1);
            }
        }
        terrain.links = terrain.find_links(|x, y| at(&poles, x, y));
        terrain
    }

    /// Split the ledge run `run0..=run1` on row `y` into platforms where
    /// her whole body fits above it.
    fn add_platforms(&mut self, y: i32, run0: i32, run1: i32) {
        let mut x = run0 + HALF;
        while x <= run1 - HALF {
            if !self.clear(x, y) {
                x += 1;
                continue;
            }
            let x0 = x;
            while x < run1 - HALF && self.clear(x + 1, y) {
                x += 1;
            }
            if x - x0 + 1 >= MIN_PLATFORM {
                self.platforms.push(Platform {
                    y,
                    x0,
                    x1: x,
                    edge_left: x0 - HALF == run0,
                    edge_right: x + HALF == run1,
                });
            }
            x += 1;
        }
    }

    /// Screen width in cells.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Whether she fits standing centred on `x` with her feet on row `y`:
    /// every body cell on screen and open.
    pub fn clear(&self, x: i32, y: i32) -> bool {
        (1..=HEIGHT).all(|dy| (-HALF..=HALF).all(|dx| self.open(x + dx, y - dy)))
    }

    /// Whether her body may occupy cell `(x, y)`.
    pub fn open(&self, x: i32, y: i32) -> bool {
        (0..self.width).contains(&x)
            && (0..self.height).contains(&y)
            && self
                .open
                .get((y * self.width + x) as usize)
                .copied()
                .unwrap_or(false)
    }

    /// Whether she is standing somewhere valid.
    pub fn platform_at(&self, x: i32, y: i32) -> Option<usize> {
        self.platforms
            .iter()
            .position(|p| p.y == y && p.contains(x))
    }

    /// The first platform below `(x, y)` she can fall onto without her
    /// body passing through anything solid on the way.
    pub fn landing(&self, x: i32, y: i32) -> Option<usize> {
        let mut best: Option<usize> = None;
        for (index, platform) in self.platforms.iter().enumerate() {
            if platform.y > y
                && platform.contains(x)
                && best.is_none_or(|b| self.platforms.get(b).is_none_or(|b| platform.y < b.y))
            {
                best = Some(index);
            }
        }
        let landing = best?;
        let to = self.platforms.get(landing)?.y;
        // The path: every standing row from just below `y` to the landing.
        ((y + 1).max(HEIGHT)..=to)
            .all(|row| self.clear(x, row))
            .then_some(landing)
    }

    fn find_links(&self, pole: impl Fn(i32, i32) -> bool) -> Vec<Link> {
        let mut links = Vec::new();
        for (a, upper) in self.platforms.iter().enumerate() {
            for (b, lower) in self.platforms.iter().enumerate() {
                if lower.y <= upper.y {
                    continue;
                }
                let lo = upper.x0.max(lower.x0);
                let hi = upper.x1.min(lower.x1);
                if lo > hi {
                    continue;
                }
                // A pole within reach — in her box or just beside it — of
                // a standing spot both platforms share, running unbroken
                // between them.
                let climb = (lo - HALF - 1..=hi + HALF + 1).find_map(|column| {
                    let x = column.clamp(lo, hi);
                    let unbroken = (upper.y + 1..lower.y).all(|row| pole(column, row));
                    (unbroken && (upper.y + 1..=lower.y).all(|row| self.clear(x, row)))
                        .then_some((x, column))
                });
                if let Some((x, pole)) = climb {
                    links.push(Link {
                        from: a,
                        to: b,
                        x,
                        route: Route::Climb,
                        pole,
                    });
                    links.push(Link {
                        from: b,
                        to: a,
                        x,
                        route: Route::Climb,
                        pole,
                    });
                }
            }
            // Drop-offs at true ledge ends: one step past the edge and down.
            for (edge, x, over) in [
                (upper.edge_left, upper.x0, upper.x0 - WIDTH),
                (upper.edge_right, upper.x1, upper.x1 + WIDTH),
            ] {
                if !edge || !self.clear(over, upper.y) {
                    continue;
                }
                if let Some(to) = self.landing(over, upper.y) {
                    links.push(Link {
                        from: a,
                        to,
                        x,
                        route: Route::Drop { over },
                        pole: x,
                    });
                }
            }
        }
        links
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn buffer(rows: &[&str]) -> Buffer {
        Buffer::with_lines(rows.iter().copied())
    }

    #[test]
    fn a_ledge_with_headroom_is_a_platform() {
        let buf = buffer(&[
            "            ",
            "            ",
            "            ",
            "            ",
            "────────────",
        ]);
        let terrain = Terrain::read(&buf, &[], false);
        assert_eq!(
            terrain.platforms,
            [Platform {
                y: 4,
                x0: 2,
                x1: 9,
                edge_left: true,
                edge_right: true,
            }]
        );
    }

    #[test]
    fn protected_headroom_walls_off_a_ledge() {
        let buf = buffer(&[
            "            ",
            "            ",
            "            ",
            "            ",
            "────────────",
        ]);
        let terrain = Terrain::read(&buf, &[Rect::new(8, 0, 4, 4)], false);
        let [platform] = terrain.platforms[..] else {
            panic!("{:?}", terrain.platforms);
        };
        assert_eq!((platform.x0, platform.x1), (2, 5));
        assert!(
            platform.edge_left && !platform.edge_right,
            "a wall, not a drop"
        );
    }

    #[test]
    fn too_little_headroom_is_no_platform() {
        let buf = buffer(&[
            "            ",
            "            ",
            "            ",
            "────────────",
        ]);
        assert!(Terrain::read(&buf, &[], false).platforms.is_empty());
    }

    #[test]
    fn a_pole_links_two_floors_both_ways() {
        let buf = buffer(&[
            "              ",
            "              ",
            "              ",
            "              ",
            "┌────────────┐",
            "│            │",
            "│            │",
            "│            │",
            "│            │",
            "└────────────┘",
        ]);
        let terrain = Terrain::read(&buf, &[], false);
        assert_eq!(terrain.platforms.len(), 2);
        let climbs: Vec<_> = terrain
            .links
            .iter()
            .filter(|l| l.route == Route::Climb)
            .collect();
        assert_eq!(climbs.len(), 2, "{:?}", terrain.links);
        assert!(climbs.iter().all(|l| l.x == 2 || l.x == 11));
    }

    #[test]
    fn a_ledge_end_drops_to_the_floor_below() {
        let buf = buffer(&[
            "              ",
            "              ",
            "              ",
            "              ",
            "───────       ",
            "              ",
            "              ",
            "              ",
            "              ",
            "──────────────",
        ]);
        let terrain = Terrain::read(&buf, &[], false);
        let drop = terrain
            .links
            .iter()
            .find(|l| matches!(l.route, Route::Drop { .. }))
            .expect("a drop-off");
        assert_eq!(terrain.platforms[drop.from].y, 4);
        assert_eq!(terrain.platforms[drop.to].y, 9);
        assert_eq!(drop.route, Route::Drop { over: 9 });
    }
}
