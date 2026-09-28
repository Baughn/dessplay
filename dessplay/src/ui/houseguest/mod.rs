//! The houseguest: when the client is fully idle, a stick-figure Osaka
//! (*Azumanga Daioh*) wanders in and makes herself at home among the
//! panes (proposal 2026-09-28-houseguest; design.md, Houseguest).
//!
//! She is a **post-render overlay** owned by the shell loop, not a
//! tui-realm component: after `Ui` has painted a frame, [`Guest::paint`]
//! reads that frame and an [`IdleView`] and paints over it. Nothing flows
//! back into `Ui`. Local input sends her away with a ~2.5 s rain
//! dissolve; a friend's chat message only makes her stop and look.
//!
//! All timing is in the shell's monotonic millis and all randomness
//! comes from a seeded generator, so tests reproduce exactly.

mod art;
mod cells;
mod dissolve;
mod graphics;
mod idle;
mod layer;
mod osaka;
mod scenes;
mod sprite;
mod terrain;

use std::time::Duration;

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Color, Modifier};

use cells::{Ink, put};
use dissolve::{Dissolve, Frozen};
use graphics::{Graphics, Look};
pub use idle::{Busy, ChatMark, IdleView, grow};
use osaka::Osaka;
use sprite::Part;
use terrain::Terrain;

/// Smallest terminal she visits.
const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 18;

/// SplitMix64: tiny, seedable, and stable across dependency upgrades.
#[derive(Clone, Debug)]
pub(crate) struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (0 when `n` is 0).
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next() % n }
    }

    /// Uniform in `lo..hi`.
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi.saturating_sub(lo))
    }
}

/// Where her line art was placed in the last frame.
#[derive(Clone, Copy, Debug)]
struct Placement {
    x: i32,
    y: i32,
    facing: sprite::Facing,
    standing: bool,
}

struct Visit {
    osaka: Osaka,
    terrain: Terrain,
    /// What she painted in the last frame, with the real cells beneath —
    /// the dissolve's frozen composite if activity arrives now.
    painted: Vec<Frozen>,
    /// Her line art in the last frame, if she was drawn as an image.
    image: Option<Placement>,
    /// Text she has moved.
    layer: layer::TextLayer,
    /// What the last frame offered her (lines to pull).
    chances: osaka::Chances,
    size: (u16, u16),
}

struct Leaving {
    dissolve: Dissolve,
    /// Line art for the startled-and-wave beat before she bursts into
    /// letters.
    image: Option<Placement>,
}

enum State {
    Absent,
    /// The idle delay elapsed; she enters on the next paint.
    Arriving,
    Visiting(Box<Visit>),
    Leaving(Box<Leaving>),
}

/// The idle houseguest.
pub struct Guest {
    rng: Rng,
    state: State,
    /// The client was idle and the setting on at the last paint.
    open: bool,
    delay: Option<Duration>,
    truecolor: bool,
    /// Monotonic millis since which the client has been idle.
    quiet_since: u64,
    chat_mark: Option<ChatMark>,
    /// Line art through the kitty protocol, when the terminal has it.
    graphics: Option<Graphics>,
}

impl Guest {
    /// A guest whose behaviour is fully determined by `seed`.
    pub fn new(seed: u64) -> Self {
        Self {
            rng: Rng(seed),
            state: State::Absent,
            open: false,
            delay: None,
            truecolor: false,
            quiet_since: 0,
            chat_mark: None,
            graphics: None,
        }
    }

    /// Draw her as line art through this terminal's image protocol
    /// (kitty only); without it she stays an ASCII sprite.
    pub fn set_picker(&mut self, picker: ratatui_image::picker::Picker) {
        self.graphics = Graphics::new(picker);
        tracing::debug!(graphics = self.graphics.is_some(), "houseguest renderer");
    }

    /// Whether she is on screen (visiting or leaving).
    pub fn present(&self) -> bool {
        matches!(self.state, State::Visiting(_) | State::Leaving(_))
    }

    /// Local input (key, mouse, paste): she leaves, and the idle timer
    /// restarts.
    pub fn activity(&mut self, now: u64) {
        self.quiet_since = now;
        self.leave(now);
    }

    fn leave(&mut self, now: u64) {
        match std::mem::replace(&mut self.state, State::Absent) {
            State::Arriving | State::Absent => {}
            State::Visiting(visit) => {
                tracing::info!("houseguest leaving");
                if !visit.painted.is_empty() {
                    let dissolve = Dissolve::new(
                        now,
                        visit.painted,
                        visit.osaka.x,
                        self.truecolor,
                        visit.size,
                    );
                    self.state = State::Leaving(Box::new(Leaving {
                        dissolve,
                        image: visit.image,
                    }));
                }
            }
            leaving @ State::Leaving(_) => self.state = leaving,
        }
    }

    /// Advance to `now` on a timer tick. Returns whether the screen
    /// could change (the shell redraws only then).
    pub fn advance(&mut self, now: u64) -> bool {
        match &mut self.state {
            State::Absent => {
                let due = self
                    .delay
                    .is_some_and(|d| now >= self.quiet_since + d.as_millis() as u64);
                if self.open && due {
                    tracing::trace!("houseguest arriving");
                    self.state = State::Arriving;
                    return true;
                }
                false
            }
            State::Arriving => true,
            State::Visiting(visit) => {
                visit
                    .osaka
                    .tick(now, &visit.terrain, &visit.chances, &mut self.rng)
            }
            State::Leaving(leaving) => {
                if leaving.dissolve.done(now) {
                    tracing::trace!("houseguest gone");
                    self.state = State::Absent;
                }
                true
            }
        }
    }

    /// How soon she next needs a tick; `None` when nothing is pending.
    pub fn next_tick(&self, now: u64) -> Option<Duration> {
        let due = match &self.state {
            State::Absent => {
                let delay = self.delay.filter(|_| self.open)?;
                self.quiet_since + delay.as_millis() as u64
            }
            State::Arriving => now,
            State::Visiting(visit) => visit.osaka.due(),
            State::Leaving(leaving) => leaving.dissolve.next_frame(now),
        };
        Some(Duration::from_millis(due.saturating_sub(now)))
    }

    /// Paint her over the finished frame. `view` must describe the frame
    /// just drawn (pane rectangles are measured during the draw).
    pub fn paint(&mut self, buf: &mut Buffer, view: &IdleView, now: u64) {
        self.observe(view, now);
        let size = (buf.area.width, buf.area.height);
        if matches!(self.state, State::Arriving) {
            self.state = State::Absent;
            if size.0 >= MIN_WIDTH && size.1 >= MIN_HEIGHT {
                let terrain = Terrain::read(buf, &view.protected, self.graphics.is_some());
                if let Some(osaka) = Osaka::arrive(now, &terrain, i32::from(size.0), &mut self.rng)
                {
                    tracing::info!("houseguest arrived");
                    self.state = State::Visiting(Box::new(Visit {
                        osaka,
                        terrain,
                        painted: Vec::new(),
                        image: None,
                        layer: layer::TextLayer::default(),
                        chances: osaka::Chances::default(),
                        size,
                    }));
                }
            }
            if !self.present() {
                // Nowhere to stand: try again after another idle delay.
                self.quiet_since = now;
            }
        }
        match &mut self.state {
            State::Absent | State::Arriving => {}
            State::Leaving(leaving) => {
                if leaving.dissolve.size() != size {
                    // The geometry she froze against is gone.
                    self.state = State::Absent;
                    return;
                }
                // Everything that compares against the real frame reads it
                // before any of her pixels or glyphs go on: her own image
                // must never look like "the UI changed here".
                let t = now.saturating_sub(leaving.dissolve.started());
                let untouched = leaving.dissolve.unchanged(buf);
                let terrain = Terrain::read(buf, &view.protected, true);
                leaving.dissolve.paint(buf, now);
                if let (Some(image), Some(graphics)) = (leaving.image, &mut self.graphics)
                    && t < dissolve::RAIN_FROM_MS
                    && untouched
                {
                    // Startled, then a wave; then she bursts into letters.
                    let look = if t < dissolve::SMILE_FROM_MS {
                        Look::Pose(sprite::Pose::Stand, sprite::Face::Surprised)
                    } else {
                        Look::Wave((t / dissolve::WAVE_MS).is_multiple_of(2))
                    };
                    graphics.paint(
                        buf,
                        look,
                        image.facing,
                        (image.x, image.y),
                        image.standing,
                        &terrain,
                    );
                }
            }
            State::Visiting(visit) => {
                // Read before paint: the layer validates against the real
                // frame, then its cells join the protected set so she
                // never stands over moved text or the holes it left.
                visit.layer.validate(buf, &view.protected);
                let mut gripped = true;
                for op in visit.osaka.take_ops() {
                    if !scenes::apply(&op, &mut visit.layer, buf, &view.protected) {
                        tracing::trace!(?op, "houseguest: the frame refused a layer change");
                        gripped &= !op.grips();
                        visit.osaka.refused(now, op);
                    }
                }
                if !gripped {
                    visit.osaka.lost_grip(now);
                }
                let mut protected = view.protected.clone();
                protected.extend(visit.layer.cells().map(|(x, y)| Rect::new(x, y, 1, 1)));
                visit.terrain = Terrain::read(buf, &protected, self.graphics.is_some());
                visit.size = size;
                if size.0 < MIN_WIDTH
                    || size.1 < MIN_HEIGHT
                    || !visit.osaka.settle(now, &visit.terrain)
                {
                    tracing::info!("houseguest left (no room)");
                    self.state = State::Absent;
                    self.quiet_since = now;
                    return;
                }
                let pulls = scenes::pulls(buf, &visit.terrain, &protected);
                let swaps = scenes::swaps(buf, &visit.terrain, &protected);
                if pulls.len() != visit.chances.pulls.len()
                    || swaps.len() != visit.chances.swaps.len()
                {
                    tracing::debug!(
                        lines = pulls.len(),
                        words = swaps.len(),
                        "houseguest: text she could tidy or play with"
                    );
                }
                let loose = scenes::loose(buf, &protected, visit.osaka.x, visit.osaka.y);
                visit.chances = osaka::Chances {
                    pulls,
                    swaps,
                    loose,
                };
                let mut layer = visit.layer.paint(buf);
                match &mut self.graphics {
                    Some(graphics) => {
                        let (painted, image) = draw_art(
                            buf,
                            graphics,
                            &visit.osaka,
                            &visit.terrain,
                            now,
                            self.truecolor,
                        );
                        layer.extend(painted);
                        visit.image = image;
                    }
                    None => {
                        layer.extend(draw(
                            buf,
                            &visit.osaka,
                            &visit.terrain,
                            now,
                            self.truecolor,
                            true,
                        ));
                        visit.image = None;
                    }
                }
                visit.painted = layer;
            }
        }
    }

    /// Update the idle gate and chat arrivals from the frame's view.
    fn observe(&mut self, view: &IdleView, now: u64) {
        self.open = view.open();
        self.delay = view.delay;
        self.truecolor = view.truecolor;
        if !self.open {
            self.quiet_since = now;
            match self.state {
                State::Arriving => self.state = State::Absent,
                State::Visiting(_) if view.delay.is_none() => {
                    // Switched off: no goodbye.
                    self.state = State::Absent;
                }
                State::Visiting(_) => self.leave(now),
                State::Absent | State::Leaving(_) => {}
            }
        }
        let arrived = self.chat_mark.is_some_and(|mark| mark != view.chat_mark);
        self.chat_mark = Some(view.chat_mark);
        if arrived {
            match &mut self.state {
                State::Visiting(visit) => {
                    tracing::trace!("houseguest looks at chat");
                    let chat = view.chat;
                    visit
                        .osaka
                        .look(now, i32::from(chat.x) + i32::from(chat.width) / 2);
                }
                State::Absent | State::Arriving => {
                    self.quiet_since = now;
                    self.state = State::Absent;
                }
                State::Leaving(_) => {}
            }
        }
    }
}

fn ink(part: Part, truecolor: bool) -> Ink {
    let fg = if truecolor {
        crate::ui::theme::TRUECOLOR_FOREGROUND
    } else {
        Color::Reset
    };
    match part {
        Part::Head => Ink::new(fg, Modifier::BOLD),
        Part::Ribbon => Ink::new(Color::LightRed, Modifier::empty()),
        Part::Body => Ink::new(fg, Modifier::empty()),
    }
}

/// Paint her (and any bubble) into `buf`, returning what was painted
/// over what.
fn draw(
    buf: &mut Buffer,
    osaka: &Osaka,
    terrain: &Terrain,
    now: u64,
    truecolor: bool,
    with_sprite: bool,
) -> Vec<Frozen> {
    let (sprite, bubble) = osaka.picture(now);
    let mut wanted: Vec<(i32, i32, char, Ink, Option<usize>)> = sprite
        .iter()
        .filter(|_| with_sprite)
        .map(|cell| {
            let face = (cell.part == Part::Head && (-1..=1).contains(&cell.dx))
                .then(|| (cell.dx + 1) as usize);
            (
                osaka.x + cell.dx,
                osaka.y + cell.dy,
                cell.glyph,
                ink(cell.part, truecolor),
                face,
            )
        })
        .collect();
    if let Some(bubble) = bubble {
        let text = bubble.text();
        let len = text.chars().count() as i32;
        let row = osaka.y - sprite::HEIGHT - 1;
        let reach = sprite::WIDTH / 2 + 1;
        let right = osaka.x + reach;
        let left = osaka.x - reach - len + 1;
        let sides = match osaka.facing {
            sprite::Facing::Right => [right, left],
            sprite::Facing::Left => [left, right],
        };
        // Bubbles are text: only over blank cells, never over a line.
        let blank = |x: i32| {
            let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(row)) else {
                return false;
            };
            buf.cell((x, y))
                .is_some_and(|c| c.symbol().trim().is_empty())
        };
        let fits = |start: i32| (start..start + len).all(|x| terrain.open(x, row) && blank(x));
        if let Some(start) = sides.into_iter().find(|&start| fits(start)) {
            let bold = Ink::new(ink(Part::Body, truecolor).fg, Modifier::BOLD);
            for (i, glyph) in text.chars().enumerate() {
                wanted.push((start + i as i32, row, glyph, bold, None));
            }
        }
    }
    // Never inside protected rectangles or images, whatever the state.
    wanted.retain(|&(x, y, ..)| terrain.open(x, y));
    let unders: Vec<_> = wanted
        .iter()
        .map(|&(x, y, ..)| {
            let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
                return None;
            };
            buf.cell((x, y)).cloned().map(|under| (x, y, under))
        })
        .collect();
    let mut painted = Vec::with_capacity(wanted.len());
    for ((x, y, glyph, ink, face), under) in wanted.into_iter().zip(unders) {
        if let Some((ux, uy, under)) = under
            && put(buf, x, y, glyph, ink)
        {
            painted.push(Frozen {
                x: ux,
                y: uy,
                glyph,
                ink,
                under,
                face,
                burst: false,
            });
        }
    }
    painted
}

/// Paint her as line art, plus any bubble as text. The frozen cells for
/// a dissolve are her box's (blank) cells, carrying the ASCII sprite's
/// glyphs as the noise class she bursts into.
fn draw_art(
    buf: &mut Buffer,
    graphics: &mut Graphics,
    osaka: &Osaka,
    terrain: &Terrain,
    now: u64,
    truecolor: bool,
) -> (Vec<Frozen>, Option<Placement>) {
    let (pose, face, _) = osaka.appearance(now);
    let (sprite, _) = osaka.picture(now);
    let placement = Placement {
        x: osaka.x,
        y: osaka.y,
        facing: osaka.facing,
        standing: osaka.standing(),
    };
    let mut body: Vec<Frozen> = Vec::new();
    for dy in -sprite::HEIGHT..0 {
        for dx in -(sprite::WIDTH / 2)..=(sprite::WIDTH / 2) {
            let (x, y) = (osaka.x + dx, osaka.y + dy);
            let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
                continue;
            };
            let Some(under) = buf.cell((ux, uy)).cloned() else {
                continue;
            };
            let cell = sprite.iter().find(|c| c.dx == dx && c.dy == dy);
            body.push(Frozen {
                x: ux,
                y: uy,
                glyph: cell.map_or('a', |c| c.glyph),
                ink: ink(cell.map_or(Part::Body, |c| c.part), truecolor),
                under,
                face: None,
                burst: true,
            });
        }
    }
    let placed = graphics
        .paint(
            buf,
            Look::Pose(pose, face),
            osaka.facing,
            (osaka.x, osaka.y),
            placement.standing,
            terrain,
        )
        .is_some();
    let mut painted = if placed { body } else { Vec::new() };
    painted.extend(draw(buf, osaka, terrain, now, truecolor, false));
    (painted, placed.then_some(placement))
}

#[cfg(test)]
mod tests;
