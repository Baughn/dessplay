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

use std::collections::{BTreeMap, HashMap};

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
use super::scrap;
use super::sprite::{Face, Facing, HEIGHT, Pose, WIDTH};

/// Her outline colour: dark line art, read against the fills.
const LINE: &str = "#1d1714";
/// Distinct frames kept; past it, the one shown longest ago is dropped
/// (never the lot: her poses in use stay, while what she showed once
/// over passing text goes).
pub(super) const CACHE_LIMIT: usize = 256;

/// What to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Look {
    Pose(Pose, Face),
    /// The goodbye wave, arm up or down.
    Wave(bool),
    /// A door in space, in her box.
    Door(art::DoorFrame),
    /// A piece of her furniture, or the part of it behind or in front of
    /// her.
    Prop(Furniture, art::Layer),
    /// A piece still in its delivery box (open while she unpacks it).
    Parcel(Furniture, bool),
    /// Her TV, switched on.
    Tv(art::Channel),
    /// A piece in some state (the lamp off, the cat in his bed…).
    Piece(Furniture, art::PieceState),
    /// A makeshift piece she made of text, or part of it.
    Scrap(Furniture, scrap::Scrap, scrap::Part),
}

impl Look {
    /// The box it fills, in cells (columns, rows above the floor).
    fn size(self) -> (i32, i32) {
        match self {
            Self::Tv(_) => {
                let (cols, rows) = Furniture::Tv.spec().footprint;
                (i32::from(cols), i32::from(rows))
            }
            Self::Prop(item, _) | Self::Parcel(item, _) | Self::Piece(item, _) => {
                let (cols, rows) = item.spec().footprint;
                (i32::from(cols), i32::from(rows))
            }
            Self::Scrap(item, ..) => {
                let (cols, rows) = scrap::footprint(item);
                (i32::from(cols), i32::from(rows))
            }
            Self::Pose(..) | Self::Wave(_) | Self::Door(_) => (WIDTH, HEIGHT),
        }
    }

    fn render(self, facing: Facing, width: u32, height: u32) -> Option<RgbaImage> {
        match self {
            Self::Pose(pose, face) => {
                art::render(&Rig::for_pose(pose, face), facing, LINE, width, height)
            }
            Self::Wave(raised) => art::render(&Rig::waving(raised), facing, LINE, width, height),
            Self::Door(frame) => art::render_door(frame, facing, LINE, width, height),
            Self::Prop(item, layer) => {
                art::render_prop_layer(item, layer, facing, LINE, width, height)
            }
            Self::Parcel(item, open) => art::render_parcel(item, open, facing, LINE, width, height),
            Self::Tv(channel) => art::render_tv(channel, facing, LINE, width, height),
            Self::Piece(item, state) => art::render_piece(item, state, facing, LINE, width, height),
            Self::Scrap(item, made, part) => {
                scrap::render(item, &made, part, facing, LINE, width, height)
            }
        }
    }
}

/// One thing in an image: `look` in its box, centred on column `at.0`
/// (rounding left) with its feet on row `at.1`; `standing` adds the floor
/// row beneath, whose line it redraws so feet rest on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Layer {
    pub look: Look,
    pub facing: Facing,
    pub at: (i32, i32),
    pub standing: bool,
}

impl Layer {
    /// Its box's left column, top row, columns and rows (with the floor
    /// row when standing).
    fn bounds(&self) -> (i32, i32, i32, i32) {
        let (width, height) = self.look.size();
        let rows = if self.standing { height + 1 } else { height };
        (self.at.0 - width / 2, self.at.1 - height, width, rows)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    /// Back to front: what, facing, its box origin within the image, and
    /// whether it stands on the floor row.
    layers: Vec<(Look, Facing, (u16, u16), bool)>,
    /// The image in cells.
    size: (u16, u16),
    /// The line glyphs the image covers and redraws, with their colours,
    /// at (column, row) within it.
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
pub(super) fn rgb(color: Color) -> [u8; 3] {
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
    /// A diagonal: `╱` (rising) or `╲` (falling).
    rise: bool,
    fall: bool,
}

/// Solid light and heavy lines, corners and tees, and the diagonals (the
/// chat's scrollback accordion). Double and dashed lines aren't redrawn,
/// so she treats them like text.
pub(super) fn strokes(c: char) -> Option<Strokes> {
    let diagonal = |rise| Strokes {
        up: false,
        down: false,
        left: false,
        right: false,
        heavy: false,
        rise,
        fall: !rise,
    };
    match c {
        '╱' => return Some(diagonal(true)),
        '╲' => return Some(diagonal(false)),
        _ => {}
    }
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
        rise: false,
        fall: false,
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
    if s.rise || s.fall {
        // Corner to corner, `t` thick, as a terminal draws it.
        for py in 0..h {
            let along = if s.rise { h - 1 - py } else { py };
            let px = along * w.max(1) / h.max(1);
            fill(px.saturating_sub(t / 2), py, t, 1);
        }
    }
}

/// The alien glyph `c` derezzes into: a 3×5 block pattern, bit
/// `row * 3 + col` set where a block is. The same character always gives
/// the same pattern.
pub(super) fn alien_bits(c: char) -> u32 {
    let mut z = u64::from(c as u32).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // At least a spine, so no glyph is a near-blank speck.
    (z as u32 & 0x7FFF) | 0b010_000_010_000_010
}

/// Derez a text glyph her image covers: an alien glyph in its colour, a
/// block pattern on a 3×5 grid picked by the character (the same letter
/// always turns into the same glyph, so text reads as a cipher).
fn draw_alien(
    image: &mut RgbaImage,
    c: char,
    color: [u8; 3],
    (x0, y0): (u32, u32),
    (w, h): (u32, u32),
) {
    let bits = alien_bits(c);
    let (cols, rows) = (3u32, 5u32);
    let (mx, my) = (w / 6, h / 8);
    let (iw, ih) = (w.saturating_sub(2 * mx), h.saturating_sub(2 * my));
    let (bw, bh) = ((iw / cols).max(1), (ih / rows).max(1));
    let pixel = Rgba([color[0], color[1], color[2], 255]);
    for row in 0..rows {
        for col in 0..cols {
            if bits >> (row * cols + col) & 1 == 0 {
                continue;
            }
            let (bx, by) = (x0 + mx + col * bw, y0 + my + row * bh);
            // A pixel's gap between big blocks keeps them glyph-like;
            // small ones would crumble into dots.
            let gap = |b: u32| if b >= 4 { b - 1 } else { b };
            for py in by..by + gap(bh) {
                for px in bx..bx + gap(bw) {
                    if px < image.width() && py < image.height() {
                        image.put_pixel(px, py, pixel);
                    }
                }
            }
        }
    }
}

/// Her image source: the picker, the line geometry, and a frame cache.
pub(super) struct Graphics {
    picker: Picker,
    line: LineGeometry,
    /// Each image, and when it was last shown.
    cache: HashMap<Key, (Protocol, u64)>,
    /// The cache's images by when they were last shown, oldest first.
    shown: BTreeMap<u64, Key>,
    /// Images shown so far (each showing's stamp).
    clock: u64,
    /// What her images have cost so far.
    #[cfg(test)]
    counts: Counts,
    /// Every image encoded so far (to tell one encoded again).
    #[cfg(test)]
    seen: std::collections::HashSet<Key>,
}

/// What her images have cost so far (tests measure the budget by it).
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Counts {
    /// Images composed and encoded.
    pub encoded: usize,
    /// Images dropped from the full frame cache.
    pub evicted: usize,
    /// Images encoded again, having been dropped (her working set
    /// thrashing).
    pub reencoded: usize,
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
            shown: BTreeMap::new(),
            clock: 0,
            #[cfg(test)]
            counts: Counts::default(),
            #[cfg(test)]
            seen: std::collections::HashSet::new(),
        })
    }

    fn cell(&self) -> (u32, u32) {
        let size = self.picker.font_size();
        (u32::from(size.width), u32::from(size.height))
    }

    /// Paint `layers` (back to front) as **one** image over the union of
    /// their boxes: two images would each hide the other's cells behind
    /// their placeholders. Every cell of the union must be hers to cover:
    /// floor-row cells lines, the rest `open` and blank or a line the
    /// image redraws. Returns the cells covered, or `None`.
    pub fn paint_layers(
        &mut self,
        buf: &mut Buffer,
        layers: &[Layer],
        open: &dyn Fn(i32, i32) -> bool,
    ) -> Option<Rect> {
        let (key, (vx0, vy0)) = self.key(buf, layers, open)?;
        let clip = key.clip;
        self.clock += 1;
        let stamp = self.clock;
        if let Some((_, last)) = self.cache.get_mut(&key) {
            let last = std::mem::replace(last, stamp);
            if let Some(key) = self.shown.remove(&last) {
                self.shown.insert(stamp, key);
            }
        } else {
            let protocol = self.frame(&key)?;
            if self.cache.len() >= CACHE_LIMIT
                && let Some((_, stale)) = self.shown.pop_first()
            {
                self.cache.remove(&stale);
                #[cfg(test)]
                {
                    self.counts.evicted += 1;
                }
            }
            #[cfg(test)]
            {
                self.counts.encoded += 1;
                if !self.seen.insert(key.clone()) {
                    self.counts.reencoded += 1;
                }
            }
            self.shown.insert(stamp, key.clone());
            self.cache.insert(key.clone(), (protocol, stamp));
        }
        let (protocol, _) = self.cache.get(&key)?;
        let rect = Rect::new(vx0 as u16, vy0 as u16, clip.2, clip.3);
        Image::new(protocol).render(rect, buf);
        Some(rect)
    }

    /// The image of `layers` over `buf` this frame, as composed (cropped
    /// to the screen), for review.
    #[cfg(test)]
    pub fn canvas(
        &self,
        buf: &Buffer,
        layers: &[Layer],
        open: &dyn Fn(i32, i32) -> bool,
    ) -> Option<RgbaImage> {
        let (key, _) = self.key(buf, layers, open)?;
        self.compose(&key)
    }

    /// What [`Graphics::paint_layers`] would paint: the image's key, and
    /// its top-left cell on screen.
    fn key(
        &self,
        buf: &Buffer,
        layers: &[Layer],
        open: &dyn Fn(i32, i32) -> bool,
    ) -> Option<(Key, (i32, i32))> {
        let (cw, ch) = self.cell();
        if cw == 0 || ch == 0 {
            return None;
        }
        let bounds: Vec<_> = layers.iter().map(Layer::bounds).collect();
        let left = bounds.iter().map(|b| b.0).min()?;
        let top = bounds.iter().map(|b| b.1).min()?;
        let right = bounds.iter().map(|b| b.0 + b.2).max()?;
        let bottom = bounds.iter().map(|b| b.1 + b.3).max()?;
        let area = buf.area;
        let vx0 = left.max(i32::from(area.left()));
        let vy0 = top.max(i32::from(area.top()));
        let vx1 = right.min(i32::from(area.right()));
        let vy1 = bottom.min(i32::from(area.bottom()));
        if vx0 >= vx1 || vy0 >= vy1 {
            return None;
        }
        let floor = |cx: i32, cy: i32| {
            layers.iter().zip(&bounds).any(|(layer, b)| {
                layer.standing && cy == layer.at.1 && (b.0..b.0 + b.2).contains(&cx)
            })
        };
        let mut lines = Vec::new();
        for cy in vy0..vy1 {
            for cx in vx0..vx1 {
                let floor = floor(cx, cy);
                if !floor && !open(cx, cy) {
                    return None;
                }
                let cell = buf.cell((cx as u16, cy as u16))?;
                let symbol = cell.symbol();
                if symbol.trim().is_empty() {
                    if floor {
                        return None;
                    }
                    continue;
                }
                // Lines are redrawn; the text she's passing is derezzed.
                // What she stands on must be a line.
                let c = symbol
                    .chars()
                    .next()
                    .filter(|&c| !floor || strokes(c).is_some())?;
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
            layers: layers
                .iter()
                .zip(&bounds)
                .map(|(layer, b)| {
                    (
                        layer.look,
                        layer.facing,
                        ((b.0 - left) as u16, (b.1 - top) as u16),
                        layer.standing,
                    )
                })
                .collect(),
            size: ((right - left) as u16, (bottom - top) as u16),
            lines,
            clip,
            cell: (cw as u16, ch as u16),
        };
        Some((key, (vx0, vy0)))
    }

    /// Distinct images in the cache.
    #[cfg(test)]
    pub fn cached(&self) -> usize {
        self.cache.len()
    }

    /// What her images have cost so far.
    #[cfg(test)]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Compose and encode one frame.
    fn frame(&self, key: &Key) -> Option<Protocol> {
        let image = self.compose(key)?;
        self.picker
            .new_protocol(
                DynamicImage::ImageRgba8(image),
                Size::new(key.clip.2, key.clip.3),
                Resize::Fit(None),
            )
            .ok()
    }

    /// Compose one frame: its lines, then its layers, cropped to what
    /// shows.
    fn compose(&self, key: &Key) -> Option<RgbaImage> {
        let (cw, ch) = self.cell();
        let (w, h) = (cw * u32::from(key.size.0), ch * u32::from(key.size.1));
        let mut canvas = RgbaImage::new(w, h);
        for &(col, row, c, color) in &key.lines {
            let origin = (u32::from(col) * cw, u32::from(row) * ch);
            if strokes(c).is_some() {
                draw_glyph(&mut canvas, c, color, origin, (cw, ch), self.line);
            } else {
                draw_alien(&mut canvas, c, color, origin, (cw, ch));
            }
        }
        for &(look, facing, (ox, oy), standing) in &key.layers {
            let (width, height) = look.size();
            // Feet rest on the line when standing, on the box floor
            // otherwise.
            let feet = if standing {
                ch * height as u32 + self.line.offset + self.line.thickness
            } else {
                ch * height as u32
            };
            let body = look.render(facing, cw * width as u32, feet)?;
            image::imageops::overlay(
                &mut canvas,
                &body,
                i64::from(u32::from(ox) * cw),
                i64::from(u32::from(oy) * ch),
            );
        }
        let (cx, cy, cwn, chn) = key.clip;
        Some(
            image::imageops::crop_imm(
                &canvas,
                u32::from(cx) * cw,
                u32::from(cy) * ch,
                u32::from(cwn) * cw,
                u32::from(chn) * ch,
            )
            .to_image(),
        )
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
