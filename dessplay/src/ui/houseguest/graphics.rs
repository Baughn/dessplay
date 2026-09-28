//! Osaka through the kitty graphics protocol (Ghostty): her line art
//! placed over her 5×4-cell box with unicode placeholders.
//!
//! Placeholders *replace* the cells they cover, so the terrain only lets
//! her stand where her box is blank. When she stands on a floor, the
//! image grows one row to include it and redraws that stretch of the
//! border line itself — matched to the terminal's box-drawing geometry —
//! so her feet rest on the line instead of hovering half a cell above.
//! `DESSPLAY_HOUSEGUEST_LINE=thickness[,offset]` (pixels) tunes the
//! match without a rebuild.

use std::collections::HashMap;

use image::{DynamicImage, Rgba, RgbaImage};
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};
use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::{Rect, Size};
use tuirealm::ratatui::style::Color;
use tuirealm::ratatui::widgets::Widget;

use super::art::{self, Rig};
use super::room::Furniture;
use super::sprite::{Face, Facing, HEIGHT, Pose, WIDTH};

/// Her outline colour: dark line art, read against the fills.
const LINE: &str = "#1d1714";
/// Distinct frames kept before the cache starts over.
const CACHE_LIMIT: usize = 256;

/// What to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Look {
    Pose(Pose, Face),
    /// The goodbye wave, arm up or down.
    Wave(bool),
    /// A piece of her furniture.
    Prop(Furniture),
}

impl Look {
    /// The box it fills, in cells (columns, rows above the floor).
    fn size(self) -> (i32, i32) {
        match self {
            Self::Prop(item) => {
                let (cols, rows) = item.footprint();
                (i32::from(cols), i32::from(rows))
            }
            Self::Pose(..) | Self::Wave(_) => (WIDTH, HEIGHT),
        }
    }

    fn render(self, facing: Facing, width: u32, height: u32) -> Option<RgbaImage> {
        match self {
            Self::Pose(pose, face) => {
                art::render(&Rig::for_pose(pose, face), facing, LINE, width, height)
            }
            Self::Wave(raised) => art::render(&Rig::waving(raised), facing, LINE, width, height),
            Self::Prop(item) => art::render_prop(item, facing, LINE, width, height),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    look: Look,
    facing: Facing,
    /// Rows in the image: her four, plus the floor row when standing.
    rows: u16,
    /// The line glyphs her image covers and redraws, with their colours,
    /// at (column, row) within her box.
    lines: Vec<(u16, u16, char, [u8; 3])>,
    /// The visible part of the image, in cells relative to its origin.
    clip: (u16, u16, u16, u16),
    cell: (u16, u16),
}

/// Where the box-drawing line sits in a cell, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LineGeometry {
    pub thickness: u32,
    pub offset: u32,
}

impl LineGeometry {
    /// Ghostty draws a light line `thickness` pixels thick (its underline
    /// thickness, ~1 px per 16 px of cell height), centred with integer
    /// division. The environment override wins.
    fn for_cell(height: u32) -> Self {
        let thickness = (height / 16).max(1);
        let mut line = Self {
            thickness,
            offset: height.saturating_sub(thickness) / 2,
        };
        if let Ok(spec) = std::env::var("DESSPLAY_HOUSEGUEST_LINE") {
            let mut parts = spec.split(',').map(|p| p.trim().parse::<u32>().ok());
            if let Some(Some(t)) = parts.next() {
                line.thickness = t.max(1);
                line.offset = height.saturating_sub(line.thickness) / 2;
            }
            if let Some(Some(offset)) = parts.next() {
                line.offset = offset.min(height.saturating_sub(1));
            }
        }
        line
    }
}

/// Resolve a cell colour to RGB as the truecolor theme paints it. With
/// truecolor the frame already holds RGB; named colours (limited depth,
/// tests) go through the theme's own mapping.
fn rgb(color: Color) -> [u8; 3] {
    crate::ui::theme::truecolor_rgb(color)
}

/// The strokes of a box-drawing glyph her image can redraw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Strokes {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    heavy: bool,
}

/// Solid light and heavy lines, corners and tees. Double and dashed
/// lines aren't redrawn, so she treats them like text.
pub(super) fn strokes(c: char) -> Option<Strokes> {
    let (u, d, l, r, heavy) = match c {
        '─' => (false, false, true, true, false),
        '│' => (true, true, false, false, false),
        '┌' | '╭' => (false, true, false, true, false),
        '┐' | '╮' => (false, true, true, false, false),
        '└' | '╰' => (true, false, false, true, false),
        '┘' | '╯' => (true, false, true, false, false),
        '├' => (true, true, false, true, false),
        '┤' => (true, true, true, false, false),
        '┬' => (false, true, true, true, false),
        '┴' => (true, false, true, true, false),
        '┼' => (true, true, true, true, false),
        '━' => (false, false, true, true, true),
        '┃' => (true, true, false, false, true),
        '┏' => (false, true, false, true, true),
        '┓' => (false, true, true, false, true),
        '┗' => (true, false, false, true, true),
        '┛' => (true, false, true, false, true),
        '┣' => (true, true, false, true, true),
        '┫' => (true, true, true, false, true),
        '┳' => (false, true, true, true, true),
        '┻' => (true, false, true, true, true),
        '╋' => (true, true, true, true, true),
        _ => return None,
    };
    Some(Strokes {
        up: u,
        down: d,
        left: l,
        right: r,
        heavy,
    })
}

/// Redraw the line glyph `c` into the cell at `(x0, y0)` of `image`, as
/// the terminal draws it: strokes from the cell centre, `thickness` wide
/// (doubled for heavy), centred with integer division.
fn draw_glyph(
    image: &mut RgbaImage,
    c: char,
    color: [u8; 3],
    (x0, y0): (u32, u32),
    (w, h): (u32, u32),
    line: LineGeometry,
) {
    let Some(s) = strokes(c) else {
        return;
    };
    let t = if s.heavy {
        line.thickness * 2
    } else {
        line.thickness
    };
    let hy = if s.heavy {
        h.saturating_sub(t) / 2
    } else {
        line.offset
    };
    let vx = w.saturating_sub(t) / 2;
    let pixel = Rgba([color[0], color[1], color[2], 255]);
    let mut fill = |x: u32, y: u32, fw: u32, fh: u32| {
        for py in y0 + y..(y0 + y + fh).min(y0 + h) {
            for px in x0 + x..(x0 + x + fw).min(x0 + w) {
                if px < image.width() && py < image.height() {
                    image.put_pixel(px, py, pixel);
                }
            }
        }
    };
    if s.left {
        fill(0, hy, vx + t, t);
    }
    if s.right {
        fill(vx, hy, w - vx, t);
    }
    if s.up {
        fill(vx, 0, t, hy + t);
    }
    if s.down {
        fill(vx, hy, t, h - hy);
    }
}

/// Her image source: the picker, the line geometry, and a frame cache.
pub(super) struct Graphics {
    picker: Picker,
    line: LineGeometry,
    cache: HashMap<Key, Protocol>,
}

impl Graphics {
    /// Graphics only through the kitty protocol, whose placeholders are
    /// what her placement rules are built around.
    pub fn new(picker: Picker) -> Option<Self> {
        if picker.protocol_type() != ProtocolType::Kitty {
            return None;
        }
        let height = u32::from(picker.font_size().height);
        Some(Self {
            picker,
            line: LineGeometry::for_cell(height),
            cache: HashMap::new(),
        })
    }

    fn cell(&self) -> (u32, u32) {
        let size = self.picker.font_size();
        (u32::from(size.width), u32::from(size.height))
    }

    /// Paint `look` at anchor `(x, y)`: centred on column `x` (rounding
    /// left), feet on row `y`. With `standing`, the floor row joins the
    /// image. Returns the cells covered, or `None` when any body cell
    /// isn't `open` or isn't blank or a line it can redraw.
    pub fn paint(
        &mut self,
        buf: &mut Buffer,
        look: Look,
        facing: Facing,
        (x, y): (i32, i32),
        standing: bool,
        open: &dyn Fn(i32, i32) -> bool,
    ) -> Option<Rect> {
        let (cw, ch) = self.cell();
        if cw == 0 || ch == 0 {
            return None;
        }
        let area = buf.area;
        let (width, height) = look.size();
        let rows = if standing { height + 1 } else { height };
        let (left, top) = (x - width / 2, y - height);
        let vx0 = left.max(i32::from(area.left()));
        let vy0 = top.max(i32::from(area.top()));
        let vx1 = (left + width).min(i32::from(area.right()));
        let vy1 = (top + rows).min(i32::from(area.bottom()));
        if vx0 >= vx1 || vy0 >= vy1 {
            return None;
        }
        // Body cells must be hers to cover (blank, or a line she
        // redraws); the floor row must be lines.
        let mut lines = Vec::new();
        for cy in vy0..vy1 {
            for cx in vx0..vx1 {
                if cy < y && !open(cx, cy) {
                    return None;
                }
                let cell = buf.cell((cx as u16, cy as u16))?;
                let symbol = cell.symbol();
                if symbol.trim().is_empty() {
                    if cy >= y {
                        return None;
                    }
                    continue;
                }
                let c = symbol.chars().next().filter(|&c| strokes(c).is_some())?;
                lines.push(((cx - left) as u16, (cy - top) as u16, c, rgb(cell.fg)));
            }
        }
        let clip = (
            (vx0 - left) as u16,
            (vy0 - top) as u16,
            (vx1 - vx0) as u16,
            (vy1 - vy0) as u16,
        );
        let key = Key {
            look,
            facing,
            rows: rows as u16,
            lines,
            clip,
            cell: (cw as u16, ch as u16),
        };
        if !self.cache.contains_key(&key) {
            if self.cache.len() >= CACHE_LIMIT {
                self.cache.clear();
            }
            let protocol = self.frame(&key)?;
            self.cache.insert(key.clone(), protocol);
        }
        let protocol = self.cache.get(&key)?;
        let rect = Rect::new(vx0 as u16, vy0 as u16, clip.2, clip.3);
        Image::new(protocol).render(rect, buf);
        Some(rect)
    }

    /// Compose and encode one frame.
    fn frame(&self, key: &Key) -> Option<Protocol> {
        let (cw, ch) = self.cell();
        let rows = u32::from(key.rows);
        let (width, height) = key.look.size();
        let standing = i32::from(key.rows) > height;
        let (w, h) = (cw * width as u32, ch * rows);
        let mut canvas = RgbaImage::new(w, h);
        // Her feet rest on the line when standing, on the box floor
        // otherwise.
        let feet = if standing {
            ch * height as u32 + self.line.offset + self.line.thickness
        } else {
            ch * height as u32
        };
        for &(col, row, c, color) in &key.lines {
            let origin = (u32::from(col) * cw, u32::from(row) * ch);
            draw_glyph(&mut canvas, c, color, origin, (cw, ch), self.line);
        }
        let body = key.look.render(key.facing, w, feet)?;
        image::imageops::overlay(&mut canvas, &body, 0, 0);
        let (cx, cy, cwn, chn) = key.clip;
        let cropped = image::imageops::crop_imm(
            &canvas,
            u32::from(cx) * cw,
            u32::from(cy) * ch,
            u32::from(cwn) * cw,
            u32::from(chn) * ch,
        )
        .to_image();
        self.picker
            .new_protocol(
                DynamicImage::ImageRgba8(cropped),
                Size::new(cwn, chn),
                Resize::Fit(None),
            )
            .ok()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn line_geometry_follows_the_cell_height() {
        assert_eq!(
            LineGeometry::for_cell(19),
            LineGeometry {
                thickness: 1,
                offset: 9
            }
        );
        assert_eq!(
            LineGeometry::for_cell(38),
            LineGeometry {
                thickness: 2,
                offset: 18
            }
        );
    }

    #[test]
    fn a_line_glyph_is_redrawn_at_the_offset_in_its_colour() {
        let line = LineGeometry {
            thickness: 1,
            offset: 9,
        };
        let mut image = RgbaImage::new(9, 19);
        draw_glyph(&mut image, '─', [1, 2, 3], (0, 0), (9, 19), line);
        for y in 0..19 {
            for x in 0..9 {
                let alpha = image.get_pixel(x, y).0[3];
                assert_eq!(alpha > 0, y == 9, "({x}, {y})");
            }
        }
        assert_eq!(image.get_pixel(4, 9).0, [1, 2, 3, 255]);
        let mut tee = RgbaImage::new(9, 19);
        draw_glyph(&mut tee, '┬', [1, 2, 3], (0, 0), (9, 19), line);
        assert_eq!(tee.get_pixel(4, 18).0[3], 255, "the stem goes down");
        assert_eq!(tee.get_pixel(4, 5).0[3], 0, "and not up");
    }
}
