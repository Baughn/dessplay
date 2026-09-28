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
mod brain;
mod cells;
mod dissolve;
mod graphics;
mod idle;
mod layer;
mod osaka;
mod room;
mod scenes;
mod sprite;
pub mod stage;
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
pub use room::{Furniture, Nook};
use room::{Home, Shown};
use sprite::{Part, Pose};
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
    /// Her furniture as placed in the last frame.
    shown: Vec<Shown>,
    /// The pieces drawn in her image in the last frame (she overlapped
    /// them).
    with: Vec<Shown>,
    size: (u16, u16),
}

struct Leaving {
    dissolve: Dissolve,
    /// Line art for the startled-and-wave beat before she bursts into
    /// letters.
    image: Option<Placement>,
    /// Her furniture's line art, held until the rain.
    props: Vec<Shown>,
    /// The pieces she overlapped, drawn in her image.
    with: Vec<Shown>,
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
    /// A scene the stage asked for, applied at the next paint.
    cue: Option<stage::Scene>,
    /// What came of the last cue.
    note: Option<Result<String, String>>,
    /// What she owns; it outlives her visits.
    home: Home,
    /// A piece the stage gave her, placed at the next paint.
    gift: Option<Furniture>,
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
            cue: None,
            note: None,
            home: Home::default(),
            gift: None,
        }
    }

    /// The stage: give her `item`, placed at the next paint on a quiet
    /// pane's floor where it fits (the note says where, or why not).
    pub fn give(&mut self, item: Furniture) {
        self.gift = Some(item);
    }

    /// The stage: the first piece she doesn't own yet.
    pub fn wishlist(&self) -> Option<Furniture> {
        Furniture::ALL
            .into_iter()
            .find(|&item| !self.home.owns(item))
    }

    /// The stage: have her do `scene` at the next paint, somewhere it
    /// works — starting a visit if she isn't here (the idle wait is
    /// skipped). See [`stage`].
    pub fn cue(&mut self, scene: stage::Scene) {
        // A scene with her furniture: she gets the piece if she has none.
        if let Some(what) = scene.furniture()
            && let Some(&item) = Furniture::ALL
                .iter()
                .find(|&&item| room::Use::of(item).contains(&what))
            && !self.home.owns(item)
        {
            self.gift = Some(item);
        }
        if scene == stage::Scene::Arrive || !matches!(self.state, State::Visiting(_)) {
            self.state = State::Arriving;
        }
        self.cue = Some(scene);
    }

    /// The stage: make `want` pressing (it weighs on her next choice).
    pub fn press(&mut self, want: stage::Want) {
        if let State::Visiting(visit) = &mut self.state {
            visit.osaka.press(want.need());
        }
    }

    /// Her needs, while she's visiting (for the stage).
    pub fn mood(&self) -> Option<String> {
        match &self.state {
            State::Visiting(visit) => Some(visit.osaka.needs().summary()),
            _ => None,
        }
    }

    /// What came of the last cue: what she's doing, or why the room
    /// offers no spot for it.
    pub fn cue_note(&self) -> Option<&Result<String, String>> {
        self.note.as_ref()
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
                        props: visit
                            .shown
                            .into_iter()
                            .filter(|s| !visit.with.contains(s))
                            .collect(),
                        with: visit.with,
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
                        shown: Vec::new(),
                        with: Vec::new(),
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
                if let Some(graphics) = &mut self.graphics
                    && t < dissolve::RAIN_FROM_MS
                    && untouched
                {
                    for prop in &leaving.props {
                        paint_prop_art(buf, graphics, prop);
                    }
                }
                if let (Some(image), Some(graphics)) = (leaving.image, &mut self.graphics)
                    && t < dissolve::RAIN_FROM_MS
                    && untouched
                {
                    // Startled, then a wave; then she bursts into letters.
                    // She jumps up out of anything she was in; what she
                    // overlapped is still drawn in her image.
                    let look = if t < dissolve::SMILE_FROM_MS {
                        Look::Pose(sprite::Pose::Stand, sprite::Face::Surprised)
                    } else {
                        Look::Wave((t / dissolve::WAVE_MS).is_multiple_of(2))
                    };
                    let her = graphics::Layer {
                        look,
                        facing: image.facing,
                        at: (image.x, image.y),
                        standing: image.standing,
                    };
                    let layers: Vec<graphics::Layer> = leaving
                        .with
                        .iter()
                        .map(|p| prop_layer(p, art::Layer::Whole))
                        .chain([her])
                        .collect();
                    graphics.paint_layers(buf, &layers, &|x, y| terrain.open(x, y));
                }
            }
            State::Visiting(visit) => {
                // Read before paint: the layer validates against the real
                // frame, then its cells join the protected set so she
                // never stands over moved text or the holes it left.
                visit.layer.validate(buf, &view.protected);
                // Her furniture stands on blank cells, clear of protected
                // ones, moved text, and her; what doesn't fit is in the
                // closet this frame. Placed, it's solid to text; she walks
                // in front of it.
                visit.shown = furnish(
                    &mut self.home,
                    &mut self.gift,
                    &mut self.note,
                    buf,
                    view,
                    visit,
                    &mut self.rng,
                );
                if let Some(item) = visit.osaka.using()
                    && !visit.shown.iter().any(|s| s.item == item)
                {
                    visit.osaka.lost_seat(now);
                }
                let mut base = view.protected.clone();
                base.extend(visit.shown.iter().map(Shown::cover));
                let mut gripped = true;
                for op in visit.osaka.take_ops() {
                    if !scenes::apply(&op, &mut visit.layer, buf, &base) {
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
                // A line she moved isn't hers to pull again (it's
                // protected), but its letters are hers to swap where
                // they now sit: swaps read the frame with her layer on.
                // Text reads keep every piece solid.
                protected.extend(visit.shown.iter().map(Shown::cover));
                let pulls = scenes::pulls(buf, &visit.terrain, &protected);
                let mut layer = visit.layer.paint(buf);
                let mut holes = base;
                holes.extend(visit.layer.holes().map(|(x, y)| Rect::new(x, y, 1, 1)));
                let swaps = scenes::swaps(buf, &visit.terrain, &holes, &visit.layer);
                if let Some(scene) = self.cue.take() {
                    let offered = osaka::Chances {
                        pulls: pulls.clone(),
                        swaps: swaps.clone(),
                        loose: Vec::new(),
                        seats: seats(&visit.shown, &visit.terrain),
                    };
                    let note =
                        stage::direct(scene, buf, &protected, visit, &offered, now, &mut self.rng);
                    tracing::info!(?note, "houseguest cued");
                    self.note = Some(note);
                }
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
                    seats: seats(&visit.shown, &visit.terrain),
                };
                // In line art, pieces she overlaps go in her image: two
                // images would cut each other out.
                let her = Rect::new(
                    (visit.osaka.x - sprite::WIDTH / 2).max(0) as u16,
                    (visit.osaka.y - sprite::HEIGHT).max(0) as u16,
                    sprite::WIDTH as u16,
                    sprite::HEIGHT as u16 + 1,
                );
                let (with, apart): (Vec<Shown>, Vec<Shown>) = visit
                    .shown
                    .iter()
                    .partition(|s| self.graphics.is_some() && s.cover().intersects(her));
                layer.extend(draw_props(
                    buf,
                    self.graphics.as_mut(),
                    &apart,
                    self.truecolor,
                ));
                match &mut self.graphics {
                    Some(graphics) => {
                        let (painted, image) = draw_art(
                            buf,
                            graphics,
                            &visit.osaka,
                            &visit.terrain,
                            &visit.shown,
                            &with,
                            now,
                            self.truecolor,
                        );
                        layer.extend(painted);
                        visit.image = image;
                        visit.with = with;
                    }
                    None => {
                        layer.extend(draw(
                            buf,
                            &visit.osaka,
                            &visit.terrain,
                            &visit.shown,
                            now,
                            self.truecolor,
                            true,
                        ));
                        visit.image = None;
                        visit.with = Vec::new();
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
#[allow(clippy::too_many_arguments)]
fn draw(
    buf: &mut Buffer,
    osaka: &Osaka,
    terrain: &Terrain,
    shown: &[Shown],
    now: u64,
    truecolor: bool,
    with_sprite: bool,
) -> Vec<Frozen> {
    let (sprite, bubble) = osaka.picture(now);
    let hidden = osaka.hidden(now);
    let door = osaka
        .door(now)
        .filter(|_| with_sprite)
        .map(|frame| sprite::door_cells(frame as usize, osaka.facing))
        .unwrap_or_default();
    let door_ink = Ink::new(Color::LightMagenta, Modifier::empty());
    let mut wanted: Vec<(i32, i32, char, Ink, Option<usize>)> = door
        .iter()
        .map(|cell| {
            (
                osaka.x + cell.dx,
                osaka.y + cell.dy,
                cell.glyph,
                door_ink,
                None,
            )
        })
        .collect();
    // Her sprite over the door; she's gone while through it.
    wanted.retain(|&(x, y, ..)| {
        hidden
            || !sprite
                .iter()
                .any(|c| (osaka.x + c.dx, osaka.y + c.dy) == (x, y))
    });
    wanted.extend(
        sprite
            .iter()
            .filter(|_| with_sprite && !hidden)
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
            }),
    );
    if let Some(bubble) = bubble.filter(|_| !hidden) {
        let text = bubble.text();
        let len = text.chars().count() as i32;
        let (pose, _, _) = osaka.appearance(now);
        if let Some((start, row)) = bubble_spot(buf, terrain, shown, osaka, pose, len) {
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

/// Where a bubble of `len` characters goes: the first of several spots
/// around her head in `pose` (the side it's nearer, else the side she
/// faces, first) that is all blank, open cells — bubbles are text, so
/// never over a line or anything else, and never inside her box, which
/// is hers.
fn bubble_spot(
    buf: &Buffer,
    terrain: &Terrain,
    shown: &[Shown],
    osaka: &Osaka,
    pose: Pose,
    len: i32,
) -> Option<(i32, i32)> {
    let half = sprite::WIDTH / 2;
    let (x, y) = (osaka.x, osaka.y);
    let (dx0, dx1, dy) = sprite::head(pose, osaka.facing);
    let (hx0, hx1, head) = (x + dx0, x + dx1, y + dy);
    let in_box = |row: i32| (y - sprite::HEIGHT..y).contains(&row);
    // The start of a bubble on `row` just clear of her head (and her
    // box) on one side, `gap` blank cells further out.
    let right = |row: i32, gap: i32| {
        let edge = if in_box(row) { x + half } else { hx1 };
        (edge + 1 + gap, row)
    };
    let left = |row: i32, gap: i32| {
        let edge = if in_box(row) { x - half } else { hx0 };
        (edge - gap - len, row)
    };
    let right_first = match (dx0 + dx1).signum() {
        1 => true,
        -1 => false,
        _ => osaka.facing == sprite::Facing::Right,
    };
    let side = |near: bool, row: i32, gap: i32| {
        if near == right_first {
            right(row, gap)
        } else {
            left(row, gap)
        }
    };
    // Up and to the side, as a speech bubble sits; then above her head;
    // then level with it; then higher up. When she's down, "above her
    // head" means over her whole box, far from it: the last resort.
    let down = in_box(head - 1);
    let over = (
        (hx0 + hx1) / 2 - len / 2,
        if down {
            y - sprite::HEIGHT - 1
        } else {
            head - 1
        },
    );
    let mut spots = vec![
        side(true, head - 1, 0),
        side(false, head - 1, 0),
        over,
        side(true, head, 1),
        side(false, head, 1),
        side(true, head - 2, 0),
        side(false, head - 2, 0),
    ];
    if down {
        spots.retain(|&spot| spot != over);
        spots.push(over);
    }
    let blank = |x: i32, y: i32| {
        let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        terrain.open(i32::from(x), i32::from(y))
            && !shown.iter().any(|s| s.rect().contains((x, y).into()))
            && buf
                .cell((x, y))
                .is_some_and(|c| c.symbol().trim().is_empty())
    };
    let clear_of_her =
        |(start, row): (i32, i32)| !in_box(row) || start + len <= x - half || start > x + half;
    spots
        .into_iter()
        .filter(|&spot| clear_of_her(spot))
        .find(|&(start, row)| (start..start + len).all(|x| blank(x, row)))
}

/// Where her furniture stands this frame, placing the stage's gift
/// first if there is one. Nothing covers protected cells or moved text.
fn furnish(
    home: &mut Home,
    gift: &mut Option<Furniture>,
    note: &mut Option<Result<String, String>>,
    buf: &Buffer,
    view: &IdleView,
    visit: &Visit,
    rng: &mut Rng,
) -> Vec<Shown> {
    let moved: std::collections::HashSet<(u16, u16)> = visit.layer.cells().collect();
    let blocked = |cx: i32, cy: i32| {
        let (Ok(ux), Ok(uy)) = (u16::try_from(cx), u16::try_from(cy)) else {
            return true;
        };
        moved.contains(&(ux, uy)) || view.protected.iter().any(|r| r.contains((ux, uy).into()))
    };
    let shown = home.resolve(buf, &view.nooks, &blocked);
    let Some(item) = gift.take() else {
        return shown;
    };
    let result = if home.owns(item) {
        Err(format!("she already has a {}", item.name()))
    } else {
        match home.spot(buf, &view.nooks, &shown, &blocked, item, rng) {
            Some((nook, prop)) => {
                tracing::info!(?item, ?nook, at = prop.at, "houseguest: new furniture");
                if home.add(nook, prop) {
                    Ok(format!("a {} in {:?}", item.name(), nook))
                } else {
                    Err(format!("{nook:?} is another room"))
                }
            }
            None => Err(format!("no room for a {}", item.name())),
        }
    };
    *note = Some(result);
    home.resolve(buf, &view.nooks, &blocked)
}

/// Where she could go to use each piece shown: in front of it on its
/// floor, or for the TV, beside it facing it.
fn seats(shown: &[Shown], terrain: &Terrain) -> Vec<room::Seat> {
    let mut out = Vec::new();
    for piece in shown {
        for &what in room::Use::of(piece.item) {
            let spots: &[i32] = if what.inside() { &[0] } else { &piece.beside() };
            let seat = spots
                .iter()
                .map(|&beside| piece.seat(what, beside))
                .find(|seat| terrain.platform_at(seat.x, seat.y).is_some());
            out.extend(seat);
        }
    }
    out
}

/// Her furniture's colour as text (the ASCII drawings).
fn prop_ink(item: Furniture, truecolor: bool) -> Ink {
    let fg = match (item, truecolor) {
        (Furniture::Sofa, true) => Color::Rgb(111, 161, 156),
        (Furniture::Sofa, false) => Color::Cyan,
        (Furniture::Tv, true) => Color::Rgb(203, 191, 168),
        (Furniture::Tv, false) => Color::Gray,
        (Furniture::Bed, true) => Color::Rgb(143, 179, 217),
        (Furniture::Bed, false) => Color::LightBlue,
        (Furniture::Desk, true) => Color::Rgb(192, 150, 100),
        (Furniture::Desk, false) => Color::Yellow,
    };
    Ink::new(fg, Modifier::empty())
}

/// `layer` of a piece of furniture as an image layer, standing on its
/// floor.
fn prop_layer(prop: &Shown, layer: art::Layer) -> graphics::Layer {
    let (cols, _) = prop.item.footprint();
    graphics::Layer {
        look: Look::Prop(prop.item, layer),
        facing: prop.facing,
        at: (prop.left + i32::from(cols) / 2, prop.floor),
        standing: true,
    }
}

/// Paint one piece's line art. Returns whether it went on.
fn paint_prop_art(buf: &mut Buffer, graphics: &mut Graphics, prop: &Shown) -> bool {
    graphics
        .paint_layers(buf, &[prop_layer(prop, art::Layer::Whole)], &|_, _| true)
        .is_some()
}

/// Paint her furniture (as line art, or as ASCII without graphics),
/// returning what was painted over what.
fn draw_props(
    buf: &mut Buffer,
    mut graphics: Option<&mut Graphics>,
    shown: &[Shown],
    truecolor: bool,
) -> Vec<Frozen> {
    let mut painted = Vec::new();
    for prop in shown {
        let ink = prop_ink(prop.item, truecolor);
        let unders: Vec<_> = prop
            .cells()
            .filter_map(|(x, y, glyph)| {
                let (ux, uy) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
                Some((ux, uy, glyph, buf.cell((ux, uy))?.clone()))
            })
            .collect();
        match graphics.as_deref_mut() {
            Some(graphics) => {
                if paint_prop_art(buf, graphics, prop) {
                    painted.extend(unders.into_iter().map(|(x, y, glyph, under)| Frozen {
                        x,
                        y,
                        glyph: glyph.unwrap_or('.'),
                        ink,
                        under,
                        face: None,
                        burst: true,
                    }));
                }
            }
            None => {
                for (x, y, glyph, under) in unders {
                    if let Some(glyph) = glyph
                        && put(buf, i32::from(x), i32::from(y), glyph, ink)
                    {
                        painted.push(Frozen {
                            x,
                            y,
                            glyph,
                            ink,
                            under,
                            face: None,
                            burst: false,
                        });
                    }
                }
            }
        }
    }
    painted
}

/// Paint her as line art, plus any bubble as text. The frozen cells for
/// a dissolve are her box's (blank) cells, carrying the ASCII sprite's
/// glyphs as the noise class she bursts into.
/// The pieces she overlaps (`with`) are drawn in the same image: behind
/// her, bar the quilt of a bed she's asleep in (and the sofa's cushion
/// is in her arms when she naps).
#[allow(clippy::too_many_arguments)]
fn draw_art(
    buf: &mut Buffer,
    graphics: &mut Graphics,
    osaka: &Osaka,
    terrain: &Terrain,
    shown: &[Shown],
    with: &[Shown],
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
    let her = graphics::Layer {
        look: Look::Pose(pose, face),
        facing: osaka.facing,
        at: (osaka.x, osaka.y),
        standing: placement.standing,
    };
    let using = osaka.seat().filter(|seat| seat.what.inside());
    let part = |piece: &Shown, front: bool| {
        let what = using.filter(|seat| seat.item == piece.item).map(|s| s.what);
        match (what, front) {
            (Some(room::Use::Sleep), false) => Some(art::Layer::Back),
            (Some(room::Use::Sleep), true) => Some(art::Layer::Front),
            (Some(room::Use::Nap), false) => Some(art::Layer::Bare),
            (_, false) => Some(art::Layer::Whole),
            (_, true) => None,
        }
    };
    let mut layers = Vec::with_capacity(with.len() * 2 + 1);
    for piece in with {
        layers.extend(part(piece, false).map(|layer| prop_layer(piece, layer)));
        body.extend(piece.cells().filter_map(|(x, y, glyph)| {
            let (ux, uy) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
            Some(Frozen {
                x: ux,
                y: uy,
                glyph: glyph.unwrap_or('.'),
                ink: prop_ink(piece.item, truecolor),
                under: buf.cell((ux, uy))?.clone(),
                face: None,
                burst: true,
            })
        }));
    }
    // A door in space stands behind her; through it, she's gone.
    if let Some(frame) = osaka.door(now) {
        layers.push(graphics::Layer {
            look: Look::Door(frame),
            ..her
        });
    }
    if !osaka.hidden(now) {
        layers.push(her);
    }
    for piece in with {
        layers.extend(part(piece, true).map(|layer| prop_layer(piece, layer)));
    }
    let placed = graphics
        .paint_layers(buf, &layers, &|x, y| terrain.open(x, y))
        .is_some();
    let mut painted = if placed { body } else { Vec::new() };
    painted.extend(draw(buf, osaka, terrain, shown, now, truecolor, false));
    (painted, placed.then_some(placement))
}

#[cfg(test)]
mod tests;
