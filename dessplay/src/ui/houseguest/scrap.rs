//! Makeshift furniture: text she tore off a line and crumpled into a
//! sofa or a bed, for when she has no real one.
//!
//! The torn glyphs leave holes in their line (the text layer keeps them)
//! and live on in the piece, drawn as shreds of the alien glyphs her
//! image derezzes text into — the same letter always the same shred —
//! in the text's own colours. It is her furnishing, so her image may
//! cover it and composite it with her, like any real piece.

use tuirealm::ratatui::style::Color;

use super::art::{CELL_UNITS, rasterize};
use super::graphics::{alien_bits, rgb};
use super::room::Furniture;
use super::sprite::Facing;

/// Most glyphs a piece is made of.
pub(super) const GLYPHS: usize = 10;
/// Fewest glyphs worth tearing off for one.
pub(super) const MIN_GLYPHS: usize = 5;
/// Crumpling goes through this many stages; at `STAGES` it's done.
pub(super) const STAGES: u8 = 4;

/// A makeshift piece: what it's made of, and how far along.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Scrap {
    /// The torn glyphs and their colours (the first `len`).
    pub glyphs: [(char, Color); GLYPHS],
    pub len: u8,
    /// 0 (a loose heap) to [`STAGES`] (done).
    pub stage: u8,
    /// Settles how it's crumpled.
    pub seed: u32,
}

impl Scrap {
    /// A piece of `glyphs` (up to [`GLYPHS`]), not yet crumpled.
    pub fn new(glyphs: &[(char, Color)], seed: u32) -> Self {
        let mut out = [(' ', Color::Reset); GLYPHS];
        for (slot, &g) in out.iter_mut().zip(glyphs) {
            *slot = g;
        }
        Self {
            glyphs: out,
            len: glyphs.len().min(GLYPHS) as u8,
            stage: 0,
            seed,
        }
    }

    pub fn done(&self) -> bool {
        self.stage >= STAGES
    }

    fn glyph(&self, i: usize) -> (char, Color) {
        let len = usize::from(self.len.max(1));
        self.glyphs
            .get(i % len)
            .copied()
            .unwrap_or((' ', Color::Reset))
    }

    /// The ASCII drawing's glyph at `(dx, dy)` of `item`'s footprint,
    /// facing `facing`: its own letters, jumbled.
    pub fn cell(&self, item: Furniture, facing: Facing, dx: u16, dy: u16) -> Option<(char, Color)> {
        let (cols, rows) = footprint(item);
        let dx = match facing {
            Facing::Right => dx,
            Facing::Left => cols - 1 - dx,
        };
        let drawn = match item {
            // A pillow at the head end, the mattress below.
            Furniture::Bed => dy == rows - 1 || (dy == rows - 2 && dx == 0),
            // The backrest, arms, and seat.
            _ => match rows - 1 - dy {
                0 => true,
                1 => dx == 0 || dx == cols - 1,
                _ => (1..cols - 1).contains(&dx),
            },
        };
        // Loose, it's only a heap in the middle of the bottom row.
        let loose = !self.done() && (dy != rows - 1 || !(2..cols - 2).contains(&dx));
        if !drawn || loose || self.len == 0 {
            return None;
        }
        let i = usize::from(dy * cols + dx) * 7 + self.seed as usize;
        Some(self.glyph(i))
    }
}

/// The pieces she can make.
pub(super) const MAKES: [Furniture; 2] = [Furniture::Sofa, Furniture::Bed];

/// A makeshift piece's size in cells (columns, rows): the sofa 7×3, the
/// bed 7×2. Using one, her five-column box is centred on column
/// [`SEAT`].
pub(super) fn footprint(item: Furniture) -> (u16, u16) {
    match item {
        Furniture::Bed => (7, 2),
        _ => (7, 3),
    }
}

/// The column (facing right) her box is centred on, using one.
pub(super) const SEAT: i32 = 3;

/// Which part of a piece to draw (see [`super::art::Layer`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Part {
    Whole,
    /// All but what covers her.
    Back,
    /// What covers her (the bed's blanket).
    Front,
}

/// One shred: a glyph's pattern (rows `from..to` of it), centred at
/// `(x, y)` units, turned `angle` degrees, at `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Shred {
    x: f32,
    y: f32,
    angle: f32,
    scale: f32,
    glyph: usize,
    from: u32,
    to: u32,
}

/// An ellipse of shreds in the piece's frame (units, facing right), and
/// whether it covers her.
struct Heap {
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    front: bool,
}

const fn heap(cx: f32, cy: f32, rx: f32, ry: f32, front: bool) -> Heap {
    Heap {
        cx,
        cy,
        rx,
        ry,
        front,
    }
}

// Frames are 140 × 84 (bed) and 140 × 126 (sofa); the floor is the
// bottom edge. Her seat is 46 units up, like the real pieces'.
/// Mattress, pillow, then the blanket over her.
static BED: [Heap; 3] = [
    heap(70.0, 64.0, 68.0, 20.0, false),
    heap(26.0, 42.0, 18.0, 10.0, false),
    heap(96.0, 48.0, 42.0, 18.0, true),
];
/// Backrest, seat, arms.
static SOFA: [Heap; 4] = [
    heap(70.0, 58.0, 58.0, 26.0, false),
    heap(70.0, 102.0, 68.0, 22.0, false),
    heap(10.0, 94.0, 11.0, 28.0, false),
    heap(130.0, 94.0, 11.0, 28.0, false),
];

fn heaps(item: Furniture) -> &'static [Heap] {
    match item {
        Furniture::Bed => &BED,
        _ => &SOFA,
    }
}

/// A small, fixed generator for laying shreds out.
struct Mix(u64);

impl Mix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `lo..hi`.
    fn float(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (self.next() % 10_000) as f32 / 10_000.0 * (hi - lo)
    }
}

/// Grid spacing between shreds, in units.
const STEP: (f32, f32) = (9.0, 11.0);

/// Every shred of `item`, back to front, each with whether it covers her.
fn shreds(item: Furniture, scrap: &Scrap) -> Vec<(Shred, bool)> {
    let mut mix = Mix(u64::from(scrap.seed));
    let mut out = Vec::new();
    for heap in heaps(item) {
        let mut y = heap.cy - heap.ry;
        while y <= heap.cy + heap.ry {
            let mut x = heap.cx - heap.rx;
            while x <= heap.cx + heap.rx {
                let (dx, dy) = ((x - heap.cx) / heap.rx, (y - heap.cy) / heap.ry);
                if dx * dx + dy * dy <= 1.0 {
                    let from = (mix.next() % 3) as u32;
                    out.push((
                        Shred {
                            x: x + mix.float(-3.0, 3.0),
                            y: y + mix.float(-3.0, 3.0),
                            angle: mix.float(-50.0, 50.0),
                            scale: mix.float(0.75, 1.05),
                            glyph: (mix.next() % 64) as usize,
                            from,
                            to: from + 2 + (mix.next() % 2) as u32,
                        },
                        heap.front,
                    ));
                }
                x += STEP.0;
            }
            y += STEP.1;
        }
    }
    out
}

/// The piece's frame in units.
fn frame(item: Furniture) -> (f32, f32) {
    let (cols, rows) = footprint(item);
    (
        f32::from(cols) * CELL_UNITS.0,
        f32::from(rows) * CELL_UNITS.1,
    )
}

/// `part` of `item` made of `scrap`, as an SVG document; `outline` is her
/// line colour, edging each shred.
fn scene(item: Furniture, scrap: &Scrap, part: Part, facing: Facing, outline: &str) -> String {
    let (w, h) = frame(item);
    let mirror = match facing {
        Facing::Right => String::new(),
        Facing::Left => format!(r#" transform="translate({w} 0) scale(-1 1)""#),
    };
    let all = shreds(item, scrap);
    // Crumpling it builds it bottom up: a loose heap first.
    let shown = if scrap.done() {
        all.len()
    } else {
        let mut order: Vec<usize> = (0..all.len()).collect();
        order.sort_by(|&a, &b| all[b].0.y.total_cmp(&all[a].0.y));
        let n = all.len() * (usize::from(scrap.stage) + 1) / (usize::from(STAGES) + 2);
        return draw(
            order.iter().take(n).map(|&i| all[i]),
            scrap,
            part,
            (w, h),
            &mirror,
            outline,
        );
    };
    draw(
        all.into_iter().take(shown),
        scrap,
        part,
        (w, h),
        &mirror,
        outline,
    )
}

fn draw(
    shreds: impl Iterator<Item = (Shred, bool)>,
    scrap: &Scrap,
    part: Part,
    (w, h): (f32, f32),
    mirror: &str,
    outline: &str,
) -> String {
    // A block of the 3 × 5 pattern, in units at scale 1.
    const BLOCK: (f32, f32) = (4.4, 6.0);
    let mut body = String::new();
    for (shred, front) in shreds {
        let wanted = match part {
            Part::Whole => true,
            Part::Back => !front,
            Part::Front => front,
        };
        if !wanted {
            continue;
        }
        let (c, color) = scrap.glyph(shred.glyph);
        let [r, g, b] = rgb(color);
        let bits = alien_bits(c);
        let rows = shred.to - shred.from;
        let (ox, oy) = (-1.5 * BLOCK.0, -(rows as f32) / 2.0 * BLOCK.1);
        body.push_str(&format!(
            r#"<g transform="translate({:.1} {:.1}) rotate({:.0}) scale({:.2})">"#,
            shred.x, shred.y, shred.angle, shred.scale
        ));
        // A torn backing, so each shred reads against the others.
        body.push_str(&format!(
            r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{outline}"/>"#,
            ox - 0.8,
            oy - 0.8,
            3.0 * BLOCK.0 + 1.6,
            rows as f32 * BLOCK.1 + 1.6,
        ));
        for row in shred.from..shred.to {
            for col in 0..3 {
                if bits >> (row * 3 + col) & 1 == 0 {
                    continue;
                }
                body.push_str(&format!(
                    r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="rgb({r},{g},{b})"/>"#,
                    ox + col as f32 * BLOCK.0 + 0.4,
                    oy + (row - shred.from) as f32 * BLOCK.1 + 0.4,
                    BLOCK.0 - 0.8,
                    BLOCK.1 - 0.8,
                ));
            }
        }
        body.push_str("</g>");
    }
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {w} {h}"><g{mirror}>{body}</g></svg>"#
    )
}

/// Render `part` of `item` made of `scrap` into a `width × height` image
/// (feet on its bottom edge, like the real pieces). `None` when the part
/// is empty.
pub(super) fn render(
    item: Furniture,
    scrap: &Scrap,
    part: Part,
    facing: Facing,
    outline: &str,
    width: u32,
    height: u32,
) -> Option<image::RgbaImage> {
    if part == Part::Front && item != Furniture::Bed {
        return None;
    }
    rasterize(
        &scene(item, scrap, part, facing, outline),
        frame(item),
        width,
        height,
    )
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::ui::houseguest::art::{Rig, render as render_her};
    use crate::ui::houseguest::sprite::{Face, Pose};

    const LINE: &str = "#1d1714";

    /// A title's worth of letters in a playlist's colours.
    fn sample(seed: u32) -> Scrap {
        let glyphs: Vec<(char, Color)> = "Frieren 12"
            .chars()
            .zip(
                [
                    Color::Rgb(201, 209, 217),
                    Color::Rgb(201, 209, 217),
                    Color::Rgb(121, 192, 255),
                    Color::Rgb(201, 209, 217),
                ]
                .into_iter()
                .cycle(),
            )
            .collect();
        let mut scrap = Scrap::new(&glyphs, seed);
        scrap.stage = STAGES;
        scrap
    }

    #[test]
    fn a_piece_renders_inside_its_footprint_at_every_stage() {
        for item in MAKES {
            let (cols, rows) = footprint(item);
            let (w, h) = (u32::from(cols) * 9, u32::from(rows) * 19);
            let mut last = 0;
            for stage in 0..=STAGES {
                let scrap = Scrap { stage, ..sample(3) };
                let image =
                    render(item, &scrap, Part::Whole, Facing::Right, LINE, w, h).expect("renders");
                let inked = image.pixels().filter(|p| p.0[3] > 0).count();
                assert!(inked > last, "{item:?} stage {stage}: {inked} after {last}");
                last = inked;
            }
            assert!(last as u32 > w * h / 4, "{item:?}: only {last} pixels");
        }
    }

    #[test]
    fn every_ascii_cell_is_one_of_its_letters() {
        let scrap = sample(5);
        let letters: Vec<char> = scrap.glyphs[..usize::from(scrap.len)]
            .iter()
            .map(|g| g.0)
            .collect();
        for item in MAKES {
            let (cols, rows) = footprint(item);
            let mut drawn = 0;
            for dy in 0..rows {
                for dx in 0..cols {
                    if let Some((c, _)) = scrap.cell(item, Facing::Left, dx, dy) {
                        assert!(letters.contains(&c));
                        drawn += 1;
                    }
                }
            }
            assert!(drawn >= usize::from(cols), "{item:?}: {drawn}");
        }
    }

    /// The makeshift pieces for review, alone at each stage and in use,
    /// at 1×, 2× and 4× over the dark theme, with the cell grid and floor:
    /// `HOUSEGUEST_SCRAPS=/tmp/scraps.png cargo test scraps_sheet -- --ignored`.
    #[test]
    #[ignore = "writes a PNG for review"]
    fn scraps_sheet() {
        let path = std::env::var("HOUSEGUEST_SCRAPS").expect("HOUSEGUEST_SCRAPS");
        let (cw, ch) = (9u32, 19u32);
        let scales = [1u32, 2, 4];
        let span = |s: u32| cw * s * 10;
        let row_h = ch * 4 * 6;
        // (piece, stage, her pose if she's using it, facing)
        let mut rows: Vec<(Furniture, u8, Option<Pose>, Facing)> = Vec::new();
        for item in MAKES {
            for stage in 0..=STAGES {
                rows.push((item, stage, None, Facing::Right));
            }
        }
        rows.push((Furniture::Sofa, STAGES, Some(Pose::Lounge), Facing::Right));
        rows.push((Furniture::Sofa, STAGES, Some(Pose::Lounge), Facing::Left));
        rows.push((Furniture::Bed, STAGES, Some(Pose::Sleep(0)), Facing::Right));
        rows.push((Furniture::Bed, STAGES, Some(Pose::Sleep(1)), Facing::Left));
        let mut sheet = image::RgbaImage::from_pixel(
            scales.iter().map(|&s| span(s)).sum(),
            row_h * rows.len() as u32,
            image::Rgba([13, 17, 23, 255]),
        );
        for (row, &(item, stage, pose, facing)) in rows.iter().enumerate() {
            let scrap = Scrap {
                stage,
                ..sample(row as u32 + 1)
            };
            let (cols, prows) = footprint(item);
            let (cols, prows) = (u32::from(cols), u32::from(prows));
            let mut x0 = 0;
            for &s in &scales {
                let (w, h) = (cw * s, ch * s);
                let floor = row as u32 * row_h + row_h - h;
                let line = floor + h / 2;
                for gx in 0..span(s) - w {
                    for t in 0..s {
                        sheet.put_pixel(x0 + gx, line + t, image::Rgba([139, 148, 158, 255]));
                    }
                }
                let px = x0 + w;
                for gy in 0..=prows.max(4) {
                    for gx in 0..w * cols {
                        sheet.put_pixel(px + gx, floor - gy * h, image::Rgba([40, 46, 56, 255]));
                    }
                }
                let (pw, ph) = (w * cols, h * prows + h / 2);
                let top = i64::from(floor + h / 2 - ph);
                let part = if pose.is_some() {
                    Part::Back
                } else {
                    Part::Whole
                };
                let mut layers = Vec::new();
                layers.extend(render(item, &scrap, part, facing, LINE, pw, ph).map(|i| (i, false)));
                if let Some(pose) = pose {
                    let rig = Rig::for_pose(pose, Face::Blink);
                    let her_facing = match (item, facing) {
                        (Furniture::Bed, f) => f,
                        (_, f) => f,
                    };
                    layers.push((
                        render_her(&rig, her_facing, LINE, w * 5, h * 4 + h / 2).unwrap(),
                        true,
                    ));
                    layers.extend(
                        render(item, &scrap, Part::Front, facing, LINE, pw, ph).map(|i| (i, false)),
                    );
                }
                for (image, her) in layers {
                    let (x, y) = if her {
                        let c = match facing {
                            Facing::Right => SEAT,
                            Facing::Left => cols as i32 - 1 - SEAT,
                        };
                        (
                            i64::from(px) + i64::from(w) * i64::from(c - 2),
                            i64::from(floor - h * 4),
                        )
                    } else {
                        (i64::from(px), top)
                    };
                    image::imageops::overlay(&mut sheet, &image, x, y);
                }
                x0 += span(s);
            }
        }
        sheet.save(path).unwrap();
    }
}
