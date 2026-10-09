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
use super::film::{FilmId, TvPicture};
use super::placement::InSight;
use super::room::{Furniture, Side};
use super::scrap;
use super::sprite::{Face, Facing, HEIGHT, Pose, WIDTH};

/// Her outline colour: dark line art, read against the fills.
const LINE: &str = "#1d1714";
/// Distinct frames kept; past it, the one shown longest ago is dropped
/// (never the lot: her poses in use stay, while what she showed once
/// over passing text goes).
///
/// Sized for a long visit by `image_census` (tests/census.rs: every mood,
/// 16 two-hour visits each, in four rooms). On a still screen (the
/// stage, her furnished home and the resident's room, 100×20 and 100×30
/// cells) the most images a visit showed was 421 by 20 minutes, 591 by
/// an hour and 664 by two hours, and the largest working set (the
/// smallest cache that drops nothing she shows again) 603: 1024 keeps
/// every image of those visits, with some 70% to spare. A live chat
/// (200×50, a line every 45 seconds scrolling the rest up) has no
/// working set to cover: each line puts new text under her, so she
/// gains some ten images a minute for as long as it runs, and those
/// one-off images push the ones she shows again down the list. There
/// 1024 drops nothing she needs again for the first hour, and by two
/// hours a visit has encoded at most 196 again, where the busiest showed
/// 1669 new ones that no cache could spare the terminal; 2048 would hold
/// two hours and then do the same.
///
/// Each kept image holds its kitty transmission (raw RGBA in base64,
/// which ratatui-image keeps for the image's life): 30 to 43 KB by room
/// at 10×20-pixel cells, so a full cache is some 30 to 43 MB of client
/// memory. The limit counts images, not bytes, so that grows with the
/// cell's area: perhaps four times as much if a terminal reports cells
/// in device pixels on a 2× display (unmeasured). The terminal keeps
/// what it was sent until its own limit evicts it (Ghostty 1.3:
/// `image-storage-limit`, 320 MB a screen by default), so an image never
/// encoded twice is never stored twice.
pub(super) const CACHE_LIMIT: usize = 1024;

/// What to draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Look {
    /// Her in a pose with a face, made only where she's in sight.
    Pose(Pose, Face, InSight),
    /// The goodbye wave, arm up or down, made only where she's in sight.
    Wave(bool, InSight),
    /// A door in space, in her box.
    Door(art::DoorFrame),
    /// A piece of her furniture, or the part of it behind or in front of
    /// her.
    Prop(Furniture, art::Layer),
    /// A piece still in its delivery box (open while she unpacks it).
    Parcel(Furniture, bool),
    /// Her TV, switched on.
    Tv(art::Channel),
    /// Her TV holding a still of the film (phase 5c D7): which still,
    /// and the programme it stands in for (drawn if the still is gone).
    Film(FilmId, art::Programme),
    /// A piece in some state (the lamp off, the cat in his bed…).
    Piece(Furniture, art::PieceState),
    /// A makeshift piece she made of text, or part of it.
    Scrap(Furniture, scrap::Scrap, scrap::Part),
    /// Her front door side-on in the wall, under the sky outside, its
    /// frame cropped to the `cols` columns nearest the wall (facing
    /// Right: a right wall, the wall's column last; Left, first). Build
    /// it with [`Look::wall_door`]; whatever it holds, it is drawn and
    /// keyed as that builds it (its size, and `Graphics::resolve`), so
    /// one image never has two keys.
    WallDoor {
        door: art::WallDoor,
        sky: art::Sky,
        cols: u8,
    },
}

impl Look {
    /// Her front door in `door`'s state under `sky`, cropped to its own
    /// columns ([`art::WallDoor::cols`]) or `crop` of them (the Away cue
    /// over text, 2): `sky` is `Day` for a state that shows nothing of
    /// outside (it draws the same under every sky), and the crop keeps
    /// to 1 ..= its own.
    pub(super) fn wall_door(door: art::WallDoor, sky: art::Sky, crop: Option<u8>) -> Self {
        let own = door.cols();
        Self::WallDoor {
            door,
            sky: if door.shows_sky() { sky } else { art::Sky::Day },
            cols: crop.unwrap_or(own).clamp(1, own),
        }
    }

    /// The look as drawn: a [`Look::WallDoor`] as [`Look::wall_door`]
    /// builds it, every other look itself.
    fn normal(self) -> Self {
        match self {
            Self::WallDoor { door, sky, cols } => Self::wall_door(door, sky, Some(cols)),
            _ => self,
        }
    }

    /// The box it fills, in cells (columns, rows above the floor).
    pub(super) fn size(self) -> (i32, i32) {
        match self {
            Self::Tv(_) | Self::Film(..) => {
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
            Self::Pose(..) | Self::Wave(..) | Self::Door(_) => (WIDTH, HEIGHT),
            Self::WallDoor { door, cols, .. } => (i32::from(cols.clamp(1, door.cols())), HEIGHT),
        }
    }

    fn render(self, facing: Facing, width: u32, height: u32) -> Option<RgbaImage> {
        match self {
            Self::Pose(pose, face, _) => {
                art::render(&Rig::for_pose(pose, face), facing, LINE, width, height)
            }
            Self::Wave(raised, _) => art::render(&Rig::waving(raised), facing, LINE, width, height),
            Self::Door(frame) => art::render_door(frame, facing, LINE, width, height),
            Self::Prop(item, layer) => {
                art::render_prop_layer(item, layer, facing, LINE, width, height)
            }
            Self::Parcel(item, open) => art::render_parcel(item, open, facing, LINE, width, height),
            Self::Tv(channel) => art::render_tv(channel, facing, LINE, width, height),
            // With no still to hand (see `Graphics::compose`), the card.
            Self::Film(_, card) => {
                art::render_tv(art::Channel::Programme(card), facing, LINE, width, height)
            }
            Self::Piece(item, state) => art::render_piece(item, state, facing, LINE, width, height),
            Self::Scrap(item, made, part) => {
                scrap::render(item, &made, part, facing, LINE, width, height)
            }
            Self::WallDoor { door, sky, cols } => {
                let cols = cols.clamp(1, door.cols());
                art::render_wall_door(door, sky, cols, facing, LINE, width, height)
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

/// A layer as painted: offset `dx` half columns (positive right) from
/// its box, and cut at a wall's line if `clip` holds one (`(side, w)`:
/// nothing drawn past the middle of column `w` on that side, as she
/// steps through her door). Its cells are its box's, widened a column
/// when the offset is odd and cut at column `w` (inclusive), so a
/// doorway at an inner wall never claims the next pane's cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct Cut {
    pub layer: Layer,
    pub clip: Option<(Side, i32)>,
    pub dx: i8,
}

impl From<Layer> for Cut {
    fn from(layer: Layer) -> Self {
        Self {
            layer,
            clip: None,
            dx: 0,
        }
    }
}

impl Cut {
    /// Its cells' left column, top row, columns and rows, or `None` when
    /// the clip leaves nothing of it.
    fn bounds(&self) -> Option<(i32, i32, i32, i32)> {
        let (x, y, width, rows) = self.layer.bounds();
        // In half columns: the image's left edge floors, its right edge
        // rounds up (`compose` puts it `dx * cw / 2` pixels over,
        // flooring).
        let dx = i32::from(self.dx);
        let mut left = (2 * x + dx).div_euclid(2);
        let mut right = (2 * (x + width) + dx + 1).div_euclid(2);
        match self.clip {
            Some((Side::Right, w)) => right = right.min(w + 1),
            Some((Side::Left, w)) => left = left.max(w),
            None => {}
        }
        (left < right).then_some((left, y, right - left, rows))
    }
}

/// One layer of a [`Key`]: what, facing, its box origin within the
/// image (negative when cut at a left wall), whether it stands on the
/// floor row, its wall's column within the image if cut, and its offset
/// in half columns.
type KeyLayer = (Look, Facing, (i16, i16), bool, Option<(Side, i16)>, i8);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    /// Back to front.
    layers: Vec<KeyLayer>,
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

/// The film's stills for her TV (phase 5c D7), in two slots: the one
/// her TV shows, and the latest delivered, which takes its place at the
/// next paint ([`Graphics::latch_film`]). An image keyed by a still is
/// only ever composed while that still is in a slot.
#[derive(Default)]
struct Film {
    showing: Option<TvPicture>,
    next: Option<TvPicture>,
}

/// Her image source: the picker, the line geometry, and a frame cache.
pub(super) struct Graphics {
    picker: Picker,
    line: LineGeometry,
    /// The film's stills her TV shows in place of its programme.
    film: Film,
    /// Each image, and when it was last shown.
    cache: HashMap<Key, (Protocol, u64)>,
    /// The cache's images by when they were last shown, oldest first.
    shown: BTreeMap<u64, Key>,
    /// Images shown so far (each showing's stamp).
    clock: u64,
    /// Distinct images kept ([`CACHE_LIMIT`]; tests change it).
    limit: usize,
    /// While measuring, each showing of a kept image's reuse distance
    /// (see [`Graphics::reuses`]).
    #[cfg(test)]
    reuses: Option<Vec<usize>>,
    /// What her images have cost so far.
    #[cfg(test)]
    counts: Counts,
    /// Every image encoded so far (to tell one encoded again).
    #[cfg(test)]
    seen: std::collections::HashSet<Key>,
    /// While recording, the looks of every layer painted since the last
    /// take (see [`Graphics::take_looks`]).
    #[cfg(test)]
    looks: Option<Vec<Look>>,
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
            film: Film::default(),
            cache: HashMap::new(),
            shown: BTreeMap::new(),
            clock: 0,
            limit: CACHE_LIMIT,
            #[cfg(test)]
            reuses: None,
            #[cfg(test)]
            counts: Counts::default(),
            #[cfg(test)]
            seen: std::collections::HashSet::new(),
            #[cfg(test)]
            looks: None,
        })
    }

    /// A still of the film for her TV, or `None` for none (the film
    /// changed or stopped: both slots go). A still waits in `next` for
    /// the next paint.
    pub fn set_film(&mut self, picture: Option<TvPicture>) {
        match picture {
            Some(picture) => self.film.next = Some(picture),
            None => self.film = Film::default(),
        }
    }

    /// At a paint: a still delivered since the last one cuts in now.
    pub fn latch_film(&mut self) {
        if let Some(next) = self.film.next.take() {
            self.film.showing = Some(next);
        }
    }

    /// `look` as drawn: as [`Look::normal`] has it (her front door
    /// keyed as [`Look::wall_door`] builds it), and her TV's programme
    /// the film's still while there's one to show.
    fn resolve(&self, look: Look) -> Look {
        let look = look.normal();
        match (look, &self.film.showing) {
            (Look::Tv(art::Channel::Programme(card)), Some(still)) => Look::Film(still.id(), card),
            _ => look,
        }
    }

    /// The still `id`'s pixels, while it's in a slot.
    fn still(&self, id: FilmId) -> Option<&image::RgbaImage> {
        [&self.film.showing, &self.film.next]
            .into_iter()
            .flatten()
            .find(|still| still.id() == id)
            .map(TvPicture::image)
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
        let cuts: Vec<Cut> = layers.iter().copied().map(Cut::from).collect();
        self.paint_cuts(buf, &cuts, open)
    }

    /// [`Graphics::paint_layers`] for layers that may be offset or cut
    /// at a wall's line ([`Cut`]).
    pub fn paint_cuts(
        &mut self,
        buf: &mut Buffer,
        cuts: &[Cut],
        open: &dyn Fn(i32, i32) -> bool,
    ) -> Option<Rect> {
        let (key, (vx0, vy0)) = self.key(buf, cuts, open)?;
        let clip = key.clip;
        self.clock += 1;
        let stamp = self.clock;
        if let Some((_, last)) = self.cache.get_mut(&key) {
            let last = std::mem::replace(last, stamp);
            #[cfg(test)]
            if let Some(reuses) = self.reuses.as_mut() {
                reuses.push(self.shown.range(last + 1..).count());
            }
            if let Some(key) = self.shown.remove(&last) {
                self.shown.insert(stamp, key);
            }
        } else {
            let protocol = self.frame(&key)?;
            if self.cache.len() >= self.limit
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
        #[cfg(test)]
        if let Some(looks) = self.looks.as_mut() {
            looks.extend(key.layers.iter().map(|layer| layer.0));
        }
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
        let cuts: Vec<Cut> = layers.iter().copied().map(Cut::from).collect();
        self.canvas_cuts(buf, &cuts, open)
    }

    /// [`Graphics::canvas`] for [`Cut`]s.
    #[cfg(test)]
    pub fn canvas_cuts(
        &self,
        buf: &Buffer,
        cuts: &[Cut],
        open: &dyn Fn(i32, i32) -> bool,
    ) -> Option<RgbaImage> {
        let (key, _) = self.key(buf, cuts, open)?;
        self.compose(&key)
    }

    /// What [`Graphics::paint_layers`] would paint: the image's key, and
    /// its top-left cell on screen.
    fn key(
        &self,
        buf: &Buffer,
        cuts: &[Cut],
        open: &dyn Fn(i32, i32) -> bool,
    ) -> Option<(Key, (i32, i32))> {
        let (cw, ch) = self.cell();
        if cw == 0 || ch == 0 {
            return None;
        }
        // A cut that leaves nothing isn't painted.
        let (cuts, bounds): (Vec<&Cut>, Vec<_>) = cuts
            .iter()
            .filter_map(|cut| Some((cut, cut.bounds()?)))
            .unzip();
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
            cuts.iter().zip(&bounds).any(|(cut, b)| {
                cut.layer.standing && cy == cut.layer.at.1 && (b.0..b.0 + b.2).contains(&cx)
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
            layers: cuts
                .iter()
                .map(|cut| {
                    let layer = &cut.layer;
                    let (bx, by, width, _) = layer.bounds();
                    // A whole column of offset is a box a column over,
                    // and a cut that doesn't reach its pixels is none:
                    // keyed so, an image has one key however it's cut.
                    let (bx, dx) = (bx + i32::from(cut.dx).div_euclid(2), cut.dx.rem_euclid(2));
                    let (cw, t) = (cw as i32, self.line.thickness as i32);
                    let x0 = bx * cw + (i32::from(dx) * cw).div_euclid(2);
                    let clip = cut.clip.filter(|&(side, w)| {
                        let line = w * cw + (cw - t).div_euclid(2);
                        match side {
                            Side::Right => x0 + width * cw > line,
                            Side::Left => x0 < line + t,
                        }
                    });
                    (
                        self.resolve(layer.look),
                        layer.facing,
                        ((bx - left) as i16, (by - top) as i16),
                        layer.standing,
                        clip.map(|(side, w)| (side, (w - left) as i16)),
                        dx,
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

    /// Distinct images in the cache showing the film on her TV.
    #[cfg(test)]
    pub fn films_cached(&self) -> usize {
        self.cache
            .keys()
            .filter(|key| key.layers.iter().any(|l| matches!(l.0, Look::Film(..))))
            .count()
    }

    /// What her images have cost so far.
    #[cfg(test)]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// Keep at most `limit` distinct images from now on.
    #[cfg(test)]
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    /// The looks of every layer painted since the last take (recording
    /// from the first): only images actually placed, so a paint that bails
    /// (nothing to key, or an encode that fails) records nothing, and an
    /// empty take proves nothing was drawn only beside a look that was.
    #[cfg(test)]
    pub fn take_looks(&mut self) -> Vec<Look> {
        self.looks.replace(Vec::new()).unwrap_or_default()
    }

    /// Measure her working set from now on (see [`Graphics::reuses`];
    /// each showing then costs a walk of the images shown since).
    #[cfg(test)]
    pub fn measure(&mut self) {
        self.reuses = Some(Vec::new());
    }

    /// While measuring, for each showing of a kept image, how many other
    /// images were shown since it last was: its reuse distance. With no
    /// limit (so nothing dropped), a cache of `n` would have dropped and
    /// encoded again just the showings whose distance is `n` or more, so
    /// one more than the largest is her working set, the smallest cache
    /// that drops nothing she shows again.
    #[cfg(test)]
    pub fn reuses(&self) -> &[usize] {
        self.reuses.as_deref().unwrap_or_default()
    }

    /// What the cached images hold on the client: each one's kitty
    /// transmission (raw RGBA in base64, in escape-wrapped chunks of 4096
    /// characters), which ratatui-image keeps for the image's life. This
    /// is the string's allocation as ratatui-image 11 reserves it outside
    /// tmux (whose wrapping escapes are longer): 4109 bytes a chunk of
    /// 3072 raw bytes, and 46 for the first chunk's header. Each entry's
    /// key (held twice, with the text under her) isn't counted: it is
    /// small beside the transmission.
    #[cfg(test)]
    pub fn cached_bytes(&self) -> usize {
        self.cache
            .keys()
            .map(|key| {
                let (cw, ch) = (usize::from(key.cell.0), usize::from(key.cell.1));
                let raw = usize::from(key.clip.2) * cw * usize::from(key.clip.3) * ch * 4;
                raw.div_ceil(3072) * 4109 + 46
            })
            .sum()
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

    /// `body`, placed with its left edge at pixel `x` of the image,
    /// with nothing past the wall's line in the image's column `w` on
    /// `side`: the line is where the terminal draws `│` in that cell
    /// (`draw_glyph`'s `(cw - t) / 2`), and the line itself is cut too,
    /// so the wall stays drawn in front of her.
    fn cut_at_wall(&self, mut body: RgbaImage, x: i64, side: Side, w: i64) -> RgbaImage {
        let (cw, _) = self.cell();
        let t = i64::from(self.line.thickness);
        let line = w * i64::from(cw) + (i64::from(cw) - t).div_euclid(2);
        for (px, _, p) in body.enumerate_pixels_mut() {
            let sx = x + i64::from(px);
            let past = match side {
                Side::Right => sx >= line,
                Side::Left => sx < line + t,
            };
            if past {
                p.0[3] = 0;
            }
        }
        body
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
        for &(look, facing, (ox, oy), standing, clip, dx) in &key.layers {
            let (width, height) = look.size();
            // Feet rest on the line when standing, on the box floor
            // otherwise.
            let feet = if standing {
                ch * height as u32 + self.line.offset + self.line.thickness
            } else {
                ch * height as u32
            };
            let body = match look {
                Look::Film(id, _) if let Some(still) = self.still(id) => {
                    art::render_film(still, facing, LINE, cw * width as u32, feet)
                }
                _ => look.render(facing, cw * width as u32, feet),
            }?;
            // Offset by half columns, flooring (as `Cut::bounds` counts).
            let x = i64::from(ox) * i64::from(cw) + (i64::from(dx) * i64::from(cw)).div_euclid(2);
            let body = match clip {
                Some((side, w)) => self.cut_at_wall(body, x, side, i64::from(w)),
                None => body,
            };
            image::imageops::overlay(&mut canvas, &body, x, i64::from(oy) * i64::from(ch));
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
    use tuirealm::ratatui::style::Style;

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

    /// Her over a screen blank but for the letter `c` in her box's corner
    /// (derezzed in her image): one distinct image a letter.
    fn her_over(c: char) -> (Buffer, [Layer; 1]) {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 10));
        buf.set_string(0, 0, c.to_string(), Style::new());
        let her = Layer {
            look: Look::Pose(Pose::Stand, Face::Vacant, InSight::assumed()),
            facing: Facing::Right,
            at: (WIDTH / 2, HEIGHT),
            standing: false,
        };
        (buf, [her])
    }

    const FILE: dessplay_core::types::Ed2kHash = dessplay_core::types::Ed2kHash([3; 16]);

    #[allow(deprecated)] // the fixed font size picker is the deterministic one
    fn graphics_at(cell: (u16, u16)) -> Graphics {
        let mut picker = Picker::from_fontsize(cell.into());
        picker.set_protocol_type(ProtocolType::Kitty);
        Graphics::new(picker).unwrap()
    }

    /// Her TV alone, its box at the screen's corner (6×4 cells), showing
    /// `look`.
    fn tv_layer(look: Look, facing: Facing) -> (Buffer, [Layer; 1]) {
        let buf = Buffer::empty(Rect::new(0, 0, 20, 8));
        let tv = Layer {
            look,
            facing,
            at: (3, 4),
            standing: false,
        };
        (buf, [tv])
    }

    const NEWS: Look = Look::Tv(art::Channel::Programme(art::Programme::News));

    fn close(a: &Rgba<u8>, b: &Rgba<u8>) -> bool {
        a.0.iter().zip(b.0).all(|(x, y)| x.abs_diff(y) <= 2)
    }

    /// The film's still on her TV's glass (phase 5c D7), at two cell
    /// sizes, the set facing either way: the still fills the glass's
    /// whole-pixel rectangle (about 27×22 pixels at 9×19 cells) through
    /// its rounded corners, unmirrored (the still's marker, left of its
    /// centre, stays left on a set facing left), its middle the still's
    /// own colour scaled down; outside the glass the set is as drawn
    /// with its programme.
    #[test]
    fn the_still_fills_the_glass_unmirrored() {
        let still = crate::ui::houseguest::film::test_picture(FILE, 0);
        for cell in [(9, 19), (10, 20)] {
            for facing in [Facing::Right, Facing::Left] {
                let at = format!("{cell:?} {facing:?}");
                let mut graphics = graphics_at(cell);
                let (buf, card) = tv_layer(NEWS, facing);
                let programme = graphics.canvas(&buf, &card, &|_, _| true).unwrap();
                graphics.set_film(Some(still.clone()));
                graphics.latch_film();
                let film = graphics.canvas(&buf, &card, &|_, _| true).unwrap();
                assert_eq!(film.dimensions(), programme.dimensions(), "{at}");
                let (w, h) = film.dimensions();
                let (left, top, gw, gh) = art::glass_rect(facing, w, h);
                let glass = match cell {
                    (9, 19) => (27, 22),
                    (10, 20) => (29, 23),
                    _ => unreachable!("{at}"),
                };
                assert_eq!((gw, gh), glass, "{at}");
                let inside =
                    |x: u32, y: u32| (left..left + gw).contains(&x) && (top..top + gh).contains(&y);
                for (x, y, p) in film.enumerate_pixels() {
                    if !inside(x, y) {
                        assert!(close(p, programme.get_pixel(x, y)), "{at}: ({x}, {y})");
                    }
                }
                // The rounded corner is the set's, not the film's.
                assert!(
                    close(film.get_pixel(left, top), programme.get_pixel(left, top)),
                    "{at}: the corner"
                );
                let scaled = crate::ui::houseguest::film::box_scale(still.image(), gw, gh);
                let middle = film.get_pixel(left + gw / 2, top + gh / 2);
                assert!(
                    close(middle, scaled.get_pixel(gw / 2, gh / 2)),
                    "{at}: {middle:?}"
                );
                // The marker: red, left of the glass's middle either way.
                let red: Vec<u32> = (top..top + gh)
                    .flat_map(|y| (left..left + gw).map(move |x| (x, y)))
                    .filter(|&(x, y)| {
                        let p = film.get_pixel(x, y);
                        p[0] > 170 && p[1] < 100 && p[2] < 100
                    })
                    .map(|(x, _)| x)
                    .collect();
                assert!(!red.is_empty(), "{at}: the marker shows");
                let mean = red.iter().sum::<u32>() as f32 / red.len() as f32;
                assert!(mean < (left + gw / 2) as f32, "{at}: marker at {mean}");
            }
        }
    }

    /// An image keyed by a still no slot holds draws the programme the
    /// still stood in for.
    #[test]
    fn a_still_gone_falls_back_to_the_programme() {
        let graphics = graphics_at((9, 19));
        let gone = crate::ui::houseguest::film::test_picture(FILE, 1).id();
        for facing in [Facing::Right, Facing::Left] {
            let (buf, card) = tv_layer(NEWS, facing);
            let (_, film) = tv_layer(Look::Film(gone, art::Programme::News), facing);
            assert_eq!(
                graphics.canvas(&buf, &film, &|_, _| true),
                graphics.canvas(&buf, &card, &|_, _| true),
                "{facing:?}"
            );
        }
    }

    /// A still delivered shows from the next paint on (her TV's
    /// programme drawn as the film until then, the still before it), the
    /// same still again is the same image (a paused film costs nothing),
    /// and none (the film changed) puts the programme back at once.
    #[test]
    fn a_still_cuts_in_at_the_next_paint() {
        let mut graphics = graphics_at((9, 19));
        graphics.take_looks();
        let paint = |graphics: &mut Graphics| {
            let (mut buf, tv) = tv_layer(NEWS, Facing::Right);
            assert!(graphics.paint_layers(&mut buf, &tv, &|_, _| true).is_some());
            graphics.take_looks()
        };
        let (a, b) = (
            crate::ui::houseguest::film::test_picture(FILE, 1),
            crate::ui::houseguest::film::test_picture(FILE, 2),
        );
        let film = |p: &TvPicture| vec![Look::Film(p.id(), art::Programme::News)];
        assert_eq!(paint(&mut graphics), vec![NEWS]);
        graphics.set_film(Some(a.clone()));
        assert_eq!(
            paint(&mut graphics),
            vec![NEWS],
            "not until the paint latches it"
        );
        graphics.latch_film();
        assert_eq!(paint(&mut graphics), film(&a));
        graphics.set_film(Some(b.clone()));
        assert_eq!(paint(&mut graphics), film(&a));
        graphics.latch_film();
        assert_eq!(paint(&mut graphics), film(&b));
        let encoded = graphics.counts().encoded;
        graphics.set_film(Some(crate::ui::houseguest::film::test_picture(FILE, 2)));
        graphics.latch_film();
        assert_eq!(paint(&mut graphics), film(&b), "the same still");
        assert_eq!(graphics.counts().encoded, encoded, "no new image");
        graphics.set_film(None);
        assert_eq!(paint(&mut graphics), vec![NEWS], "cleared at once");
    }

    /// A blank screen with a wall's `│` down column `w` (in an odd
    /// colour, to tell its pixels), her standing centred on it.
    fn her_at_a_wall(w: i32) -> (Buffer, Layer) {
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 8));
        for y in 0..8 {
            buf.set_string(w as u16, y, "│", Style::new().fg(Color::Rgb(1, 2, 3)));
        }
        let her = Layer {
            look: Look::Pose(Pose::Stand, Face::Vacant, InSight::assumed()),
            facing: Facing::Right,
            at: (w, HEIGHT + 1),
            standing: false,
        };
        (buf, her)
    }

    /// Cut at a wall's line (step 7, D8), her image keeps to her side of
    /// it at both walls, still or offset a half column either way (her
    /// stepping through): its cells stop at the wall's column (so a
    /// doorway at an inner wall never claims the next pane's), the
    /// line's own pixels there are the wall's colour (redrawn, she isn't
    /// over them), past it nothing is drawn, and on her side she is (her
    /// middle stands on the line, so she's cut through). Painted, at a
    /// left wall (her box's origin left of the image), the image is
    /// placed and kept like any other.
    #[test]
    fn a_clipped_layer_paints_nothing_past_the_wall() {
        let w = 10;
        for cell in [(9u16, 19u16), (10, 20)] {
            let mut graphics = graphics_at(cell);
            let (cw, ch) = (u32::from(cell.0), u32::from(cell.1));
            let t = LineGeometry::for_cell(ch).thickness;
            for side in [Side::Right, Side::Left] {
                // Her box is columns w-2 ..= w+2, a half column over with
                // an odd offset: the columns of it on her side of w.
                for (dx, cols) in [(-1i8, [4, 3]), (0, [3, 3]), (1, [3, 4])] {
                    let cols = cols[usize::from(side == Side::Left)];
                    let at = format!("{cell:?} {side:?} {dx}");
                    let (buf, her) = her_at_a_wall(w);
                    let cut = Cut {
                        layer: her,
                        clip: Some((side, w)),
                        dx,
                    };
                    let mine = |x: i32, _| match side {
                        Side::Right => x <= w,
                        Side::Left => x >= w,
                    };
                    let image = graphics.canvas_cuts(&buf, &[cut], &mine).unwrap();
                    assert_eq!(
                        image.dimensions(),
                        (cols * cw, u32::from(HEIGHT as u16) * ch),
                        "{at}"
                    );
                    let line = match side {
                        Side::Right => (cols - 1) * cw + (cw - t) / 2,
                        Side::Left => (cw - t) / 2,
                    };
                    let mut hers = 0;
                    for (x, y, p) in image.enumerate_pixels() {
                        let on_line = (line..line + t).contains(&x);
                        let past = match side {
                            Side::Right => x >= line + t,
                            Side::Left => x < line,
                        };
                        if on_line {
                            assert_eq!(p.0, [1, 2, 3, 255], "{at}: the line at ({x}, {y})");
                        } else if past {
                            assert_eq!(p.0[3], 0, "{at}: drawn past the line at ({x}, {y})");
                        } else if p.0[3] > 0 {
                            hers += 1;
                        }
                    }
                    let beside = match side {
                        Side::Right => line - 1,
                        Side::Left => line + t,
                    };
                    assert!(
                        (0..image.height()).any(|y| image.get_pixel(beside, y).0[3] > 0),
                        "{at}: she's drawn up to the line"
                    );
                    assert!(hers > 100, "{at}: {hers} pixels of her");
                    let encoded = graphics.counts().encoded;
                    for again in [0, 1] {
                        // Painting fills the buffer: a fresh one each time.
                        let (mut buf, _) = her_at_a_wall(w);
                        let rect = graphics.paint_cuts(&mut buf, &[cut], &mine).unwrap();
                        let left = match side {
                            Side::Right => w + 1 - cols as i32,
                            Side::Left => w,
                        };
                        assert_eq!(
                            (i32::from(rect.x), u32::from(rect.width)),
                            (left, cols),
                            "{at}: placed"
                        );
                        assert_eq!(graphics.counts().encoded, encoded + 1, "{at}: {again}");
                    }
                }
            }
        }
    }

    /// Offset by a half column either way, her image is the same pixels
    /// moved `cw / 2` (flooring) over, in a box a column wider; and a cut
    /// that leaves nothing of her paints nothing.
    #[test]
    fn an_offset_layer_moves_by_half_columns() {
        let graphics = graphics_at((9, 19));
        let buf = Buffer::empty(Rect::new(0, 0, 20, 8));
        let her = Layer {
            look: Look::Pose(Pose::Walk(1), Face::Vacant, InSight::assumed()),
            facing: Facing::Right,
            at: (8, HEIGHT + 1),
            standing: false,
        };
        let base = graphics.canvas(&buf, &[her], &|_, _| true).unwrap();
        for (dx, shift) in [(1i8, 4u32), (-1, 4)] {
            let cut = Cut {
                layer: her,
                clip: None,
                dx,
            };
            let moved = graphics.canvas_cuts(&buf, &[cut], &|_, _| true).unwrap();
            assert_eq!(
                moved.dimensions(),
                (base.width() + 9, base.height()),
                "{dx}"
            );
            for (x, y, p) in moved.enumerate_pixels() {
                let want = x
                    .checked_sub(shift)
                    .filter(|&bx| bx < base.width())
                    .map_or(0, |bx| base.get_pixel(bx, y).0[3]);
                assert_eq!(p.0[3], want, "{dx}: ({x}, {y})");
            }
        }
        // Wholly past a right wall at column 5 (her box is 6 ..= 10).
        let gone = Cut {
            layer: her,
            clip: Some((Side::Right, 5)),
            dx: 0,
        };
        assert!(graphics.canvas_cuts(&buf, &[gone], &|_, _| true).is_none());
    }

    /// Her front door as a look, shut (2 columns), open (4) and its flap
    /// swung up (6): standing on the floor in the wall's column, cropped
    /// to its state's columns, each of them inked.
    #[test]
    fn a_wall_door_look_stands_in_its_columns() {
        let graphics = graphics_at((9, 19));
        let (w, f) = (12, 6);
        let mut buf = Buffer::empty(Rect::new(0, 0, 20, 8));
        buf.set_string(0, f as u16, "─".repeat(20), Style::new());
        let doors = [
            (
                art::WallDoor::Shut {
                    flap: 0,
                    away: false,
                },
                2,
            ),
            (art::WallDoor::Open, 4),
            (
                art::WallDoor::Shut {
                    flap: 90,
                    away: false,
                },
                6,
            ),
        ];
        for (door, cols) in doors {
            for facing in [Facing::Right, Facing::Left] {
                let at = format!("{door:?} {facing:?}");
                let look = Look::wall_door(door, art::Sky::Day, None);
                assert_eq!(look.size(), (cols, HEIGHT), "{at}");
                // Centred on its middle column, its last (a right wall)
                // or first (a left wall) in the wall's.
                let centre = match facing {
                    Facing::Right => w - cols / 2 + 1,
                    Facing::Left => w + cols / 2,
                };
                let door_layer = Layer {
                    look,
                    facing,
                    at: (centre, f),
                    standing: true,
                };
                let (left, ..) = door_layer.bounds();
                let want = match facing {
                    Facing::Right => w - cols + 1,
                    Facing::Left => w,
                };
                assert_eq!(left, want, "{at}");
                let image = graphics.canvas(&buf, &[door_layer], &|_, _| true).unwrap();
                assert_eq!(image.width(), 9 * cols as u32, "{at}");
                for col in 0..cols as u32 {
                    let inked = (0..19 * 4)
                        .any(|y| (col * 9..col * 9 + 9).any(|x| image.get_pixel(x, y).0[3] > 64));
                    assert!(inked, "{at}: column {col}");
                }
            }
        }
    }

    /// One image, one key: her door showing nothing of outside is the
    /// same image under every sky, and a crop past its own columns its
    /// own (one that shows outside, one image a sky); her cut at a wall
    /// whose line she doesn't reach, or offset whole columns, is the
    /// image of her plainly there.
    #[test]
    fn an_image_is_keyed_once_however_it_is_built() {
        let (w, f) = (12, 6);
        // Painting fills the buffer: a fresh one each time.
        let floor = || {
            let mut buf = Buffer::empty(Rect::new(0, 0, 24, 8));
            buf.set_string(0, f as u16, "─".repeat(24), Style::new());
            buf
        };
        for (door, skies) in [
            (
                art::WallDoor::Shut {
                    flap: 0,
                    away: false,
                },
                1,
            ),
            (art::WallDoor::Post, 1),
            (art::WallDoor::Open, 5),
        ] {
            let mut graphics = graphics_at((9, 19));
            for sky in art::Sky::ALL {
                for cols in [door.cols(), 6, 9] {
                    let look = Look::WallDoor { door, sky, cols };
                    let layer = Layer {
                        look,
                        facing: Facing::Right,
                        at: (w - i32::from(door.cols()) / 2 + 1, f),
                        standing: true,
                    };
                    graphics
                        .paint_layers(&mut floor(), &[layer], &|_, _| true)
                        .unwrap();
                }
            }
            assert_eq!(graphics.cached(), skies, "{door:?}");
        }
        let mut graphics = graphics_at((9, 19));
        let her = |x| Layer {
            look: Look::Pose(Pose::Walk(1), Face::Vacant, InSight::assumed()),
            facing: Facing::Right,
            at: (x, HEIGHT + 1),
            standing: false,
        };
        let mut paint = |cut: Cut| {
            let mut buf = Buffer::empty(Rect::new(0, 0, 24, 8));
            graphics.paint_cuts(&mut buf, &[cut], &|_, _| true).unwrap();
            graphics.cached()
        };
        // Her box is columns 8 ..= 12; its pixels end at 13 · 9.
        assert_eq!(paint(her(10).into()), 1);
        for clip in [(Side::Right, 13), (Side::Right, 20), (Side::Left, 7)] {
            let cut = Cut {
                layer: her(10),
                clip: Some(clip),
                dx: 0,
            };
            assert_eq!(paint(cut), 1, "{clip:?}: cuts nothing");
        }
        for dx in [-4i8, -2, 2, 4] {
            let cut = Cut {
                layer: her(10 - i32::from(dx) / 2),
                clip: None,
                dx,
            };
            assert_eq!(paint(cut), 1, "{dx}: whole columns");
        }
        let half = paint(Cut {
            layer: her(10),
            clip: None,
            dx: 1,
        });
        assert_eq!(half, 2, "a half column is another image");
        for (x, dx) in [(11, -1i8), (9, 3)] {
            let cut = Cut {
                layer: her(x),
                clip: None,
                dx,
            };
            assert_eq!(paint(cut), 2, "{x} {dx}: the same half column");
        }
        let cut = Cut {
            layer: her(10),
            clip: Some((Side::Right, 12)),
            dx: 0,
        };
        assert_eq!(paint(cut), 3, "a cut that bites");
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(
            dessplay_core::test_support::proptest_cases(64)
        ))]

        /// Whatever she shows, in whatever order, a full cache drops the
        /// image shown longest ago, one at a time, as a plain list in
        /// order of last showing would: each image is encoded when it
        /// isn't kept, and only then; it counts as encoded again when it
        /// was kept once before; and each reuse distance measured is how
        /// far down the list the image shown again was.
        #[test]
        #[allow(deprecated)] // the fixed font size picker is the deterministic one
        fn a_full_cache_drops_what_she_showed_longest_ago(
            limit in 1usize..6,
            shows in proptest::collection::vec(proptest::char::range('a', 'j'), 0..80),
        ) {
            let mut picker = Picker::from_fontsize((9, 19).into());
            picker.set_protocol_type(ProtocolType::Kitty);
            let mut graphics = Graphics::new(picker).unwrap();
            graphics.set_limit(limit);
            graphics.measure();
            // Most recently shown last.
            let mut model: Vec<char> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            let (mut encoded, mut evicted, mut reencoded) = (0, 0, 0);
            let mut reuses = Vec::new();
            for &c in &shows {
                // Each frame over the screen afresh, as the client paints.
                let (mut buf, her) = her_over(c);
                let painted = graphics.paint_layers(&mut buf, &her, &|_, _| true);
                proptest::prop_assert!(painted.is_some());
                if let Some(i) = model.iter().position(|&kept| kept == c) {
                    // The images shown since this one last was.
                    reuses.push(model.len() - 1 - i);
                    model.remove(i);
                } else {
                    encoded += 1;
                    if !seen.insert(c) {
                        reencoded += 1;
                    }
                    if model.len() >= limit {
                        model.remove(0);
                        evicted += 1;
                    }
                }
                model.push(c);
                let counts = graphics.counts();
                proptest::prop_assert_eq!(
                    (counts.encoded, counts.evicted, counts.reencoded),
                    (encoded, evicted, reencoded)
                );
                proptest::prop_assert_eq!(graphics.reuses(), &reuses[..]);
                proptest::prop_assert_eq!(graphics.cached(), model.len());
            }
        }
    }
}
