//! The houseguest: when the client is fully idle, a stick-figure Osaka
//! (*Azumanga Daioh*) wanders in and makes herself at home among the
//! panes (proposal 2026-09-28-houseguest; design.md, Houseguest).
//!
//! She is a **post-render overlay** owned by the shell loop, not a
//! tui-realm component: after `Ui` has painted a frame, [`Guest::paint`]
//! reads that frame and an [`IdleView`] and paints over it. Nothing flows
//! back into `Ui`. Local input sends her away with a ~3.75 s rain
//! dissolve — unless she's resident, when she stays and only keeps out
//! of the focused pane; a friend's chat message only makes her stop and
//! look. When the chat log is scrolled back and messages go unseen,
//! she comes to poke its accordion (see [`nudge`]).
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
mod ledger;
mod nudge;
mod osaka;
mod room;
mod scenes;
mod scrap;
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
pub use idle::{Busy, ChatMark, IdleView, Scrollback, grow};
pub use ledger::Ledger;
use osaka::Osaka;
use room::Shown;
pub use room::{Furniture, Nook};
use sprite::{Part, Pose};
use terrain::Terrain;

/// What the shopping channel sells, in order (the TV comes first, on
/// its own).
const CATALOGUE: [Furniture; 7] = [
    Furniture::Sofa,
    Furniture::Bed,
    Furniture::Desk,
    Furniture::Lamp,
    Furniture::Bookshelf,
    Furniture::Fridge,
    Furniture::CatBed,
];
/// She buys at most once every this many visits.
const SHOP_EVERY: u64 = 3;
/// The visit her TV arrives on (the first is a first meeting).
const FIRST_TV_VISIT: u64 = 2;
/// How long each frame of what's on TV lasts.
const CHANNEL_FRAME_MS: u64 = 400;
/// What she says when a parcel arrives.
const PARCEL: &str = "A parcel!";

/// What the shopping channel sells her if she watches now: the next
/// piece she lacks, at most once every [`SHOP_EVERY`] visits (any time
/// with the stage's `shop_now`), and never while something's on order
/// or still boxed.
fn advert(ledger: &Ledger, shop_now: bool) -> Option<Furniture> {
    let due = shop_now || ledger.visits >= ledger.bought_on + SHOP_EVERY;
    let idle = ledger.ordered.is_none() && !ledger.home.boxed();
    let item = CATALOGUE
        .into_iter()
        .find(|&item| !ledger.home.owns(item))?;
    (due && idle && ledger.home.owns(Furniture::Tv)).then_some(item)
}

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
    /// What of hers is raining out of a pane that was just focused.
    fades: Vec<Dissolve>,
    /// Makeshift furniture she has made of text this visit.
    made: Vec<Made>,
    /// The text she was reeling in at the last paint.
    reel: Option<scenes::Build>,
    size: (u16, u16),
}

/// `view` with every protected rectangle widened to take in both halves
/// of a wide glyph straddling its left or right edge: a wide glyph is
/// one brick, and writing beside half of one blanks all of it.
fn whole_glyphs(buf: &Buffer, mut view: IdleView) -> IdleView {
    let wide = |x: u16, y: u16| buf.cell((x, y)).is_some_and(|c| cells::width(c) > 1);
    for rect in &mut view.protected {
        let rows = rect.top()..rect.bottom();
        let left = rect
            .x
            .checked_sub(1)
            .is_some_and(|x| rows.clone().any(|y| wide(x, y)));
        let right = rect.width > 0 && rows.clone().any(|y| wide(rect.right() - 1, y));
        if left {
            rect.x -= 1;
            rect.width += 1;
        }
        if right && rect.right() < buf.area.right() {
            rect.width += 1;
        }
    }
    view
}

/// `cells` as rectangles, a run of neighbours along a row making one:
/// protected sets are scanned per cell, and moved text comes in runs.
fn runs(cells: impl Iterator<Item = (u16, u16)>) -> Vec<Rect> {
    let mut cells: Vec<(u16, u16)> = cells.map(|(x, y)| (y, x)).collect();
    cells.sort_unstable();
    cells.dedup();
    let mut out: Vec<Rect> = Vec::new();
    for (y, x) in cells {
        match out.last_mut() {
            Some(run) if run.y == y && run.right() == x => run.width += 1,
            _ => out.push(Rect::new(x, y, 1, 1)),
        }
    }
    out
}

impl Visit {
    /// What her body keeps out of: `protected`, and the text she moved
    /// and the holes it left. Every terrain she's placed on is read with
    /// this, so a spot chosen on one is a spot on the other.
    fn solid(&self, protected: &[Rect]) -> Vec<Rect> {
        let mut out = protected.to_vec();
        out.extend(runs(self.layer.cells()));
        out
    }
}

/// A makeshift piece, and the glyphs torn off to make it (their holes
/// stay while it does).
#[derive(Clone, Debug)]
struct Made {
    piece: Shown,
    torn: Vec<(u16, u16)>,
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

/// She's on her way to the chat's scrollback accordion, or poking it.
struct Errand {
    /// The accordion she was sent to (moved or gone, the errand's off).
    accordion: Rect,
    /// She got as far as poking it.
    poked: bool,
    /// Someone was at the keys while she was at it (a visitor leaves
    /// once she's done).
    leave_after: bool,
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
    /// Resident Osaka, as of the last paint.
    resident: bool,
    /// Local input since the last paint (resident): what she moved in
    /// the chat goes back.
    shake: bool,
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
    /// Her record: her home, and what her visits are seeded from.
    ledger: Ledger,
    /// The ledger changed since it was last handed out for saving.
    unsaved: bool,
    /// Whether it's saved at all (not when the stored one couldn't be
    /// read: that one is left alone).
    persist: bool,
    /// A piece the stage gave her, placed at the next paint.
    gift: Option<Furniture>,
    /// The stage: the shopping channel is on whenever she watches.
    shop_now: bool,
    /// The stage: the cat is home this visit.
    cat_now: bool,
    /// When to poke the scrollback accordion, and its shake.
    nudge: nudge::Nudge,
    errand: Option<Errand>,
}

impl Guest {
    /// A guest whose behaviour is fully determined by `seed`, meeting
    /// her for the first time.
    pub fn new(seed: u64) -> Self {
        Self::restore(Ledger::new(seed))
    }

    /// The guest `ledger` records.
    pub fn restore(ledger: Ledger) -> Self {
        Self {
            rng: Rng(ledger.visit_seed(ledger.visits)),
            ledger,
            unsaved: false,
            persist: true,
            state: State::Absent,
            open: false,
            resident: false,
            shake: false,
            delay: None,
            truecolor: false,
            quiet_since: 0,
            chat_mark: None,
            graphics: None,
            cue: None,
            note: None,
            gift: None,
            shop_now: false,
            cat_now: false,
            nudge: nudge::Nudge::default(),
            errand: None,
        }
    }

    /// Never hand out the ledger for saving (the stored one couldn't be
    /// read, and is kept as it is).
    pub fn keep_unsaved(&mut self) {
        self.persist = false;
    }

    /// Her ledger, when it changed since last asked and is to be saved.
    pub fn ledger_to_save(&mut self) -> Option<Ledger> {
        (std::mem::take(&mut self.unsaved) && self.persist).then(|| self.ledger.clone())
    }

    /// "Osaka moved out": her home and record are gone; the next visit
    /// is a first meeting, drawn from `seed`. She leaves at once.
    pub fn move_out(&mut self, seed: u64) {
        tracing::info!("houseguest moved out");
        self.ledger = Ledger::new(seed);
        self.unsaved = true;
        self.persist = true;
        self.gift = None;
        self.shop_now = false;
        if matches!(self.state, State::Visiting(_) | State::Arriving) {
            self.state = State::Absent;
        }
    }

    /// The stage: give her `item`, placed at the next paint on a quiet
    /// pane's floor where it fits (the note says where, or why not).
    pub fn give(&mut self, item: Furniture) {
        self.gift = Some(item);
    }

    /// The stage: a parcel with the next piece she doesn't own (her TV
    /// first), delivered at the next paint.
    pub fn send_parcel(&mut self) {
        let next = std::iter::once(Furniture::Tv)
            .chain(CATALOGUE)
            .find(|&item| !self.ledger.home.owns(item));
        if let Some(item) = next {
            self.ledger.ordered = Some(item);
            self.ledger.bought_on = self.ledger.visits.saturating_sub(1);
            self.unsaved = true;
        }
    }

    /// The stage: put the shopping channel on next time she watches.
    pub fn shop(&mut self) {
        self.shop_now = true;
    }

    /// The stage: the first piece she doesn't own yet.
    pub fn wishlist(&self) -> Option<Furniture> {
        Furniture::ALL
            .into_iter()
            .find(|&item| !self.ledger.home.owns(item))
    }

    /// The stage: have her do `scene` at the next paint, somewhere it
    /// works — starting a visit if she isn't here (the idle wait is
    /// skipped). See [`stage`].
    pub fn cue(&mut self, scene: stage::Scene) {
        // A scene with her furniture: she gets the piece if she has none.
        match scene {
            stage::Scene::Parcel => self.send_parcel(),
            stage::Scene::Pet => {
                self.cat_now = true;
                if !self.ledger.home.owns(Furniture::CatBed) {
                    self.gift = Some(Furniture::CatBed);
                }
            }
            stage::Scene::Work => {
                if self.ledger.home.props.is_empty() {
                    self.gift = Some(Furniture::Sofa);
                }
            }
            stage::Scene::Shopping => {
                self.shop();
                if !self.ledger.home.owns(Furniture::Tv) {
                    self.gift = Some(Furniture::Tv);
                }
            }
            _ => {
                if let Some(what) = scene.furniture()
                    && let Some(&item) = Furniture::ALL
                        .iter()
                        .find(|&&item| room::Use::of(item).contains(&what))
                    && !self.ledger.home.owns(item)
                {
                    self.gift = Some(item);
                }
            }
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

    /// Local input (key, mouse, paste): the idle timer restarts, and she
    /// leaves — or, resident, stays, and what she moved in the chat goes
    /// back.
    pub fn activity(&mut self, now: u64) {
        self.quiet_since = now;
        // Mid-errand, a visitor finishes poking first.
        if let Some(errand) = &mut self.errand
            && !self.resident
        {
            errand.leave_after = true;
            return;
        }
        if !self.resident {
            return self.leave(now);
        }
        match self.state {
            State::Arriving => self.state = State::Absent,
            State::Visiting(_) => self.shake = true,
            State::Absent | State::Leaving(_) => {}
        }
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
        let nudge = self.nudge.advance(now);
        let changed = match &mut self.state {
            State::Absent => {
                let due = self
                    .delay
                    .is_some_and(|d| now >= self.quiet_since + d.as_millis() as u64);
                if self.open && due {
                    tracing::trace!("houseguest arriving");
                    self.state = State::Arriving;
                }
                self.open && due
            }
            State::Arriving => true,
            State::Visiting(visit) => {
                visit.fades.retain(|fade| !fade.done(now));
                let fading = !visit.fades.is_empty();
                visit
                    .osaka
                    .tick(now, &visit.terrain, &visit.chances, &mut self.rng)
                    || fading
            }
            State::Leaving(leaving) => {
                if leaving.dissolve.done(now) {
                    tracing::trace!("houseguest gone");
                    self.state = State::Absent;
                }
                true
            }
        };
        changed || nudge
    }

    /// How soon she next needs a tick; `None` when nothing is pending.
    pub fn next_tick(&self, now: u64) -> Option<Duration> {
        let due = match &self.state {
            State::Absent => self
                .delay
                .filter(|_| self.open)
                .map(|delay| self.quiet_since + delay.as_millis() as u64),
            State::Arriving => Some(now),
            State::Visiting(visit) => Some(
                visit
                    .fades
                    .iter()
                    .map(|fade| fade.next_frame(now))
                    .fold(visit.osaka.due(), u64::min),
            ),
            State::Leaving(leaving) => Some(leaving.dissolve.next_frame(now)),
        };
        let due = due.into_iter().chain(self.nudge.next_at(now)).min()?;
        Some(Duration::from_millis(due.saturating_sub(now)))
    }

    /// Paint her over the finished frame. `view` must describe the frame
    /// just drawn (pane rectangles are measured during the draw).
    pub fn paint(&mut self, buf: &mut Buffer, view: &IdleView, now: u64) {
        self.observe(view, now);
        let view = &whole_glyphs(buf, self.gate(view, now));
        let size = (buf.area.width, buf.area.height);
        self.errand_progress(view, now);
        self.nudge_due(buf, view, now);
        if matches!(self.state, State::Arriving) {
            self.state = State::Absent;
            if size.0 >= MIN_WIDTH && size.1 >= MIN_HEIGHT {
                let terrain = Terrain::read(buf, &view.protected, self.graphics.is_some());
                // Each visit draws from its own seed.
                self.rng = Rng(self.ledger.visit_seed(self.ledger.visits));
                if let Some(osaka) = Osaka::arrive(now, &terrain, i32::from(size.0), &mut self.rng)
                {
                    self.begin_visit(osaka, terrain, size);
                }
            }
            if !self.present() {
                // Nowhere to stand: try again after another idle delay.
                self.quiet_since = now;
            }
        }
        // The accordion's shake is hers, painted like the rest of her:
        // after everything that reads the real frame, and under her.
        let nudge = &self.nudge;
        match &mut self.state {
            State::Absent | State::Arriving => nudge.paint(buf, now),
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
                nudge.paint(buf, now);
                leaving.dissolve.paint(buf, now);
                if let Some(graphics) = &mut self.graphics
                    && t < dissolve::RAIN_FROM_MS
                    && untouched
                {
                    for prop in &leaving.props {
                        paint_prop_art(buf, graphics, prop, &Looks::default());
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
                        .map(|p| {
                            let look = piece_look(
                                p,
                                art::Layer::Whole,
                                None,
                                false,
                                art::PieceState::Plain,
                            );
                            prop_layer(p, look)
                        })
                        .chain([her])
                        .collect();
                    graphics.paint_layers(buf, &layers, &|x, y| terrain.open(x, y));
                }
            }
            State::Visiting(visit) => {
                // Someone's at the keys: what she moved in the chat goes
                // back (the holes it leaves settle in the validation).
                if std::mem::take(&mut self.shake) && view.resident {
                    let undone = visit.layer.drop_in(view.chat);
                    if undone > 0 {
                        tracing::debug!(undone, "houseguest: chat mischief shaken off");
                    }
                    visit.osaka.shaken(now, view.chat);
                }
                // A pane was just focused: what of hers was in it rains
                // away at once (no startled beat), while she carries on
                // elsewhere.
                if let Some(focus) = view.focus {
                    let out: Vec<Frozen> = visit
                        .painted
                        .iter()
                        .filter(|cell| focus.contains((cell.x, cell.y).into()))
                        .cloned()
                        .collect();
                    if !out.is_empty() && visit.size == size {
                        tracing::debug!(
                            cells = out.len(),
                            "houseguest: raining out of the focused pane"
                        );
                        visit.fades.push(Dissolve::new(
                            now.saturating_sub(dissolve::RAIN_FROM_MS),
                            out,
                            visit.osaka.x,
                            self.truecolor,
                            size,
                        ));
                    }
                }
                visit
                    .fades
                    .retain(|fade| fade.size() == size && !fade.done(now));
                // Read before paint: the layer validates against the real
                // frame, then its cells join the protected set so she
                // never stands over moved text or the holes it left.
                visit.layer.validate(buf, &view.protected);
                // Her furniture stands on blank cells, clear of protected
                // ones, moved text, and her; what doesn't fit is in the
                // closet this frame. Placed, it's solid to text; she walks
                // in front of it.
                let before = self.ledger.clone();
                for event in visit.osaka.take_events() {
                    match event {
                        osaka::HomeEvent::Bought(item) => {
                            self.ledger.ordered = Some(item);
                            self.ledger.bought_on = self.ledger.visits;
                            self.shop_now = false;
                            self.unsaved = true;
                        }
                        osaka::HomeEvent::Unpacked(item) => {
                            self.unsaved |= self.ledger.home.unbox(item);
                        }
                        osaka::HomeEvent::Crumpled(seat) => {
                            for made in &mut visit.made {
                                if made.piece.seat(room::Use::Crumple, 0) == seat
                                    && let Some(scrap) = &mut made.piece.scrap
                                {
                                    scrap.stage = scrap::STAGES;
                                }
                            }
                        }
                    }
                }
                let shown = furnish(
                    &mut self.ledger,
                    &mut self.gift,
                    &mut self.note,
                    buf,
                    view,
                    visit,
                    now,
                    &mut self.rng,
                );
                visit.shown = shown;
                if self.ledger != before {
                    self.unsaved = true;
                }
                tend_made(visit, buf, &view.protected, size, now);
                visit.shown.extend(visit.made.iter().map(|made| made.piece));
                if let Some((seat, ..)) = visit.osaka.use_span()
                    && !visit
                        .shown
                        .iter()
                        .any(|s| s.item == seat.item && s.scrap.is_some() == seat.makeshift)
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
                    } else if let scenes::LayerOp::Make { row, cells, piece } = &op {
                        visit.made.push(made_of(buf, *row, cells, piece));
                    }
                }
                if !gripped {
                    visit.osaka.lost_grip(now);
                }
                // Text she was reeling in and never made anything of (she
                // was interrupted, or lost her grip) goes back.
                let reeling = visit.osaka.reeling().cloned();
                if let Some(old) = visit.reel.take()
                    && reeling.as_ref() != Some(&old)
                {
                    let sources: Vec<(u16, u16)> =
                        old.cells.iter().map(|&c| (c, old.row)).collect();
                    if !visit.made.iter().any(|m| m.torn == sources) {
                        tracing::debug!("houseguest: dropped the text she was reeling in");
                        visit.layer.unreel(&sources);
                    }
                }
                visit.reel = reeling;
                let mut protected = visit.solid(&view.protected);
                visit.terrain = Terrain::read(buf, &protected, self.graphics.is_some());
                visit.terrain.furnish(visit.shown.iter().map(Shown::cover));
                visit.size = size;
                let chat = view.resident.then_some(view.chat);
                // Out of the focused pane, through her door.
                // Text came up where she stays (under her, or under the
                // image she's drawn in): she gets up.
                visit.osaka.recheck(&visit.terrain, now);
                let evicted = view.focus.is_some_and(|focus| {
                    !visit
                        .osaka
                        .evict(focus, &visit.terrain, chat, now, &mut self.rng)
                });
                if evicted
                    || size.0 < MIN_WIDTH
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
                holes.extend(runs(visit.layer.holes()));
                let swaps = scenes::swaps(buf, &visit.terrain, &holes, &visit.layer);
                let builds = builds(buf, visit, &pulls, &protected);
                if let Some(scene) = self.cue.take() {
                    let offered = osaka::Chances {
                        pulls: pulls.clone(),
                        swaps: swaps.clone(),
                        loose: Vec::new(),
                        seats: seats(
                            &visit.shown,
                            &visit.terrain,
                            self.cat_now || cat_home(&self.ledger),
                        ),
                        builds: builds.clone(),
                        advert: advert(&self.ledger, self.shop_now),
                        furnished: !self.ledger.home.props.is_empty(),
                        chat,
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
                    seats: seats(
                        &visit.shown,
                        &visit.terrain,
                        self.cat_now || cat_home(&self.ledger),
                    ),
                    builds,
                    advert: advert(&self.ledger, self.shop_now),
                    furnished: !self.ledger.home.props.is_empty(),
                    chat,
                };
                // In line art, pieces she overlaps go in her image: two
                // images would cut each other out.
                let covers: Vec<Rect> = visit.shown.iter().map(Shown::cover).collect();
                let drawn = match self.graphics {
                    Some(_) => terrain::image(visit.osaka.x, visit.osaka.y, &covers).with,
                    None => vec![false; covers.len()],
                };
                let (mut with, mut apart) = (Vec::new(), Vec::new());
                for (&piece, drawn) in visit.shown.iter().zip(drawn) {
                    if drawn {
                        with.push(piece);
                    } else {
                        apart.push(piece);
                    }
                }
                // She's watching: the TV is on. And the lamp, the fridge,
                // the cat...
                let cat = self.cat_now || cat_home(&self.ledger);
                let looks = Looks {
                    tv: visit.osaka.watching().map(|(since, advert)| {
                        let frame = (now.saturating_sub(since) / CHANNEL_FRAME_MS % 2) as u8;
                        match advert {
                            Some(_) => art::Channel::Shopping(frame),
                            None => art::Channel::Snow(frame),
                        }
                    }),
                    states: visit
                        .shown
                        .iter()
                        .map(|p| (p.item, piece_state(p, &visit.osaka, cat, now)))
                        .collect(),
                };
                nudge.paint(buf, now);
                layer.extend(draw_props(
                    buf,
                    self.graphics.as_mut(),
                    &apart,
                    &looks,
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
                            &looks,
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
                // Last, over the real frame in the focused pane (where
                // nothing else of hers goes).
                for fade in &mut visit.fades {
                    fade.paint(buf, now);
                }
            }
        }
    }

    /// A new visit, with `osaka` just arrived.
    fn begin_visit(&mut self, osaka: Osaka, terrain: Terrain, size: (u16, u16)) {
        self.ledger.visits += 1;
        self.unsaved = true;
        tracing::info!(visit = self.ledger.visits, "houseguest arrived");
        self.state = State::Visiting(Box::new(Visit {
            osaka,
            terrain,
            painted: Vec::new(),
            image: None,
            layer: layer::TextLayer::default(),
            chances: osaka::Chances::default(),
            shown: Vec::new(),
            with: Vec::new(),
            fades: Vec::new(),
            made: Vec::new(),
            reel: None,
            size,
        }));
    }

    /// Follow her errand: the accordion shakes when she pokes it; done
    /// (or called off), a visitor who shouldn't be here leaves. If she
    /// never got to poke it, it shakes by itself.
    fn errand_progress(&mut self, view: &IdleView, now: u64) {
        let Some(errand) = &mut self.errand else {
            return;
        };
        let accordion = view.scrollback.map(|back| back.accordion);
        let State::Visiting(visit) = &mut self.state else {
            // She's gone (no room, or visits were switched off).
            let poked = errand.poked;
            self.errand = None;
            if !poked {
                self.nudge.shake(now);
            }
            return;
        };
        if accordion != Some(errand.accordion) {
            visit.osaka.drop_errand(now);
        }
        if visit.osaka.take_poked() {
            errand.poked = true;
            self.nudge.shake(now);
        }
        if visit.osaka.on_errand() {
            return;
        }
        let Some(errand) = self.errand.take() else {
            return;
        };
        if !errand.poked && accordion.is_some() {
            self.nudge.shake(now);
        }
        let quiet = view
            .delay
            .is_some_and(|delay| now >= self.quiet_since + delay.as_millis() as u64);
        if !view.resident && (errand.leave_after || !quiet || view.busy.is_some()) {
            self.leave(now);
        }
    }

    /// A poke is due: send her to the accordion (arriving for it if she
    /// isn't here), or — visits off, or nowhere for her to stand on it —
    /// it shakes by itself. Not while something covers the panes or
    /// someone's selecting in the chat, and not mid-goodbye.
    fn nudge_due(&mut self, buf: &Buffer, view: &IdleView, now: u64) {
        if !self.nudge.due(now) {
            return;
        }
        let visits = view.delay.is_some();
        let blocked = matches!(view.busy, Some(Busy::Overlay | Busy::Selection))
            || self.errand.is_some()
            || visits && matches!(self.state, State::Leaving(_));
        let Some(accordion) = self.nudge.accordion().filter(|_| !blocked) else {
            self.nudge.wait(now);
            return;
        };
        self.nudge.poked(now);
        let size = (buf.area.width, buf.area.height);
        let roomy = size.0 >= MIN_WIDTH && size.1 >= MIN_HEIGHT;
        if !(visits && roomy && self.send(buf, view, accordion, now)) {
            self.nudge.shake(now);
        }
    }

    /// Send her to poke `accordion`. False when there's no floor on it.
    fn send(&mut self, buf: &Buffer, view: &IdleView, accordion: Rect, now: u64) -> bool {
        // The errand overrides the focused pane (the accordion's usually
        // in it).
        let mut protected: Vec<Rect> = view
            .protected
            .iter()
            .copied()
            .filter(|&rect| Some(rect) != view.focus)
            .collect();
        // Where she'd stand is read as she'll read it: visiting, the text
        // she moved (and its holes) is solid to her.
        if let State::Visiting(visit) = &self.state {
            protected = visit.solid(&protected);
        }
        let terrain = Terrain::read(buf, &protected, self.graphics.is_some());
        let Some(spot) = accordion_spot(&terrain, accordion) else {
            tracing::debug!("houseguest: nowhere to stand on the accordion");
            return false;
        };
        match &mut self.state {
            State::Visiting(visit) => visit.osaka.errand(spot, &visit.terrain, now),
            State::Absent | State::Arriving => {
                self.rng = Rng(self.ledger.visit_seed(self.ledger.visits));
                let osaka = Osaka::arrive_for_errand(spot, now, &mut self.rng);
                let size = (buf.area.width, buf.area.height);
                self.begin_visit(osaka, terrain, size);
            }
            State::Leaving(_) => return false,
        }
        self.errand = Some(Errand {
            accordion,
            poked: false,
            leave_after: false,
        });
        true
    }

    /// `view` as she keeps to it now: while the client is in use —
    /// playing, a held selection, or local input within the idle delay —
    /// a resident keeps out of the focused pane (it's protected); left
    /// alone, the whole screen is hers again.
    fn gate(&self, view: &IdleView, now: u64) -> IdleView {
        let mut view = view.clone();
        let quiet = view
            .delay
            .is_none_or(|delay| now >= self.quiet_since + delay.as_millis() as u64);
        let in_use = !quiet || view.busy.is_some();
        match view.focus {
            // On her errand, the accordion comes first (it's usually in
            // the focused pane).
            Some(focus) if in_use && self.errand.is_none() => view.protected.push(focus),
            _ => view.focus = None,
        }
        view
    }

    /// Update the idle gate and chat arrivals from the frame's view.
    fn observe(&mut self, view: &IdleView, now: u64) {
        self.open = view.open();
        self.resident = view.resident;
        self.delay = view.delay;
        self.truecolor = view.truecolor;
        self.nudge.observe(view.scrollback, now);
        if !self.open {
            self.quiet_since = now;
            match self.state {
                State::Arriving => self.state = State::Absent,
                State::Visiting(_) if view.delay.is_none() => {
                    // Switched off: no goodbye.
                    self.state = State::Absent;
                }
                // On an errand, she stays till it's done.
                State::Visiting(_) if self.errand.is_some() => {}
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

/// Where she stands to poke `accordion`: the spot on it where her box
/// covers the least text (the log's lines above it; she may stand over
/// them for the poke), rightmost on a tie (short lines leave the right
/// side blank).
fn accordion_spot(terrain: &Terrain, accordion: Rect) -> Option<(i32, i32)> {
    let (left, right) = (i32::from(accordion.x), i32::from(accordion.right()) - 1);
    let y = i32::from(accordion.y);
    let covered = |x: i32| {
        (1..=sprite::HEIGHT)
            .flat_map(|dy| (-sprite::WIDTH / 2..=sprite::WIDTH / 2).map(move |dx| (x + dx, y - dy)))
            .filter(|&(cx, cy)| !terrain.calm(cx, cy))
            .count()
    };
    terrain
        .platforms
        .iter()
        .filter(|p| p.y == y)
        .flat_map(|p| p.x0.max(left)..=p.x1.min(right))
        .min_by_key(|&x| (covered(x), -x))
        .map(|x| (x, y))
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
#[allow(clippy::too_many_arguments)]
fn furnish(
    ledger: &mut Ledger,
    gift: &mut Option<Furniture>,
    note: &mut Option<Result<String, String>>,
    buf: &Buffer,
    view: &IdleView,
    visit: &mut Visit,
    now: u64,
    rng: &mut Rng,
) -> Vec<Shown> {
    let moved: std::collections::HashSet<(u16, u16)> = visit.layer.cells().collect();
    let blocked = |cx: i32, cy: i32| {
        let (Ok(ux), Ok(uy)) = (u16::try_from(cx), u16::try_from(cy)) else {
            return true;
        };
        moved.contains(&(ux, uy)) || view.protected.iter().any(|r| r.contains((ux, uy).into()))
    };
    let home = &mut ledger.home;
    let mut shown = home.resolve(buf, &view.nooks, &blocked);
    // A delivery: what she ordered on an earlier visit arrives boxed,
    // wherever it will fit. Her first TV is ordered for her, to arrive
    // on her second visit.
    if ledger.ordered.is_none() && !home.owns(Furniture::Tv) && ledger.visits >= FIRST_TV_VISIT {
        ledger.ordered = Some(Furniture::Tv);
        ledger.bought_on = ledger.visits - 1;
    }
    if let Some(item) = ledger.ordered
        && ledger.bought_on < ledger.visits
        && let Some((nook, prop)) = home.spot(buf, &view.nooks, &shown, &blocked, item, rng)
        && home.add(
            nook,
            room::Prop {
                boxed: true,
                ..prop
            },
        )
    {
        tracing::info!(?item, ?nook, "houseguest: a parcel arrived");
        ledger.ordered = None;
        visit.osaka.say(PARCEL, now);
        shown = home.resolve(buf, &view.nooks, &blocked);
    }
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
/// floor, or for the TV, beside it facing it — or from a sofa on the
/// TV's floor.
fn seats(shown: &[Shown], terrain: &Terrain, cat: bool) -> Vec<room::Seat> {
    shown
        .iter()
        .flat_map(|piece| seats_of(piece, shown, terrain, cat))
        .collect()
}

/// Where she could go to use `piece`, among the pieces `shown`: only
/// where she may stay (in line art, where the image she'd be drawn in is
/// clear of text).
fn seats_of(piece: &Shown, shown: &[Shown], terrain: &Terrain, cat: bool) -> Vec<room::Seat> {
    let mut out = spots_for(piece, shown, terrain, cat);
    out.retain(|seat| terrain.restful(seat.x, seat.y));
    out
}

fn spots_for(piece: &Shown, shown: &[Shown], terrain: &Terrain, cat: bool) -> Vec<room::Seat> {
    let mut out = Vec::new();
    // No cat, no petting.
    if piece.item == Furniture::CatBed && !cat {
        return out;
    }
    for &what in piece.uses() {
        let spots: &[i32] = if what.inside() { &[0] } else { &piece.beside() };
        let seat = spots
            .iter()
            .map(|&beside| piece.seat(what, beside))
            .find(|seat| {
                terrain.restful(seat.x, seat.y) && terrain.platform_at(seat.x, seat.y).is_some()
            });
        out.extend(seat);
    }
    // A sofa on the same floor as the TV is where to watch it from.
    if piece.uses().contains(&room::Use::Lounge) {
        let sofa = piece.seat(room::Use::Lounge, 0);
        let floor = terrain.platform_at(sofa.x, sofa.y);
        let tv = shown.iter().find(|tv| {
            tv.item == Furniture::Tv
                && !tv.boxed
                && floor.is_some()
                && terrain.platform_at(tv.left, tv.floor) == floor
        });
        if let Some(tv) = tv {
            let facing = if tv.left > sofa.x {
                sprite::Facing::Right
            } else {
                sprite::Facing::Left
            };
            out.push(room::Seat {
                what: room::Use::Watch,
                facing,
                ..sofa
            });
        }
    }
    out
}

/// Keep her makeshift pieces that still stand: each while every glyph
/// torn off for it is still torn off (its line hasn't changed, and its
/// pane isn't protected), and it still fits where she made it, clear of
/// her real furniture and anything `protected`; a resize takes them all.
/// What goes, goes back into its line. One she's crumpling grows as she
/// does.
fn tend_made(visit: &mut Visit, buf: &Buffer, protected: &[Rect], size: (u16, u16), now: u64) {
    let resized = visit.size != size;
    let real = visit.shown.clone();
    let moved: std::collections::HashSet<(u16, u16)> = visit.layer.cells().collect();
    let clear = |x: i32, y: i32| {
        let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        !moved.contains(&(ux, uy))
            && !protected.iter().any(|r| r.contains((ux, uy).into()))
            && !real.iter().any(|s| s.cover().contains((ux, uy).into()))
    };
    let mut gone = Vec::new();
    let layer = &visit.layer;
    visit.made.retain(|made| {
        let stands =
            !resized && layer.torn_intact(&made.torn) && room::fits(buf, &made.piece, &clear);
        if !stands {
            gone.push(made.torn.clone());
        }
        stands
    });
    for torn in gone {
        tracing::debug!("houseguest: a makeshift piece fell apart");
        visit.layer.mend(&torn);
    }
    if let Some((seat, since, until)) = visit.osaka.use_span()
        && seat.what == room::Use::Crumple
    {
        let length = until.saturating_sub(since).max(1);
        let stage = 1 + now.saturating_sub(since) * u64::from(scrap::STAGES - 1) / length;
        for made in &mut visit.made {
            if made.piece.seat(room::Use::Crumple, 0) == seat
                && let Some(scrap) = &mut made.piece.scrap
                && !scrap.done()
            {
                scrap.stage = scrap
                    .stage
                    .max(stage.min(u64::from(scrap::STAGES - 1)) as u8);
            }
        }
    }
}

/// A makeshift piece `piece` of the glyphs just torn off `row` at
/// `cells` (read before anything of hers is painted).
fn made_of(buf: &Buffer, row: u16, cells: &[u16], piece: &Shown) -> Made {
    let glyphs: Vec<(char, Color)> = cells
        .iter()
        .filter_map(|&c| {
            let cell = buf.cell((c, row))?;
            Some((cell.symbol().chars().next()?, cell.fg))
        })
        .collect();
    // Crumpled its own way wherever it's made.
    let seed = (piece.left as u32)
        .wrapping_mul(0x9E37_79B9)
        .wrapping_add(piece.floor as u32)
        .wrapping_add(u32::from(row) << 16);
    tracing::info!(item = ?piece.item, glyphs = glyphs.len(), "houseguest: tore text off for furniture");
    Made {
        piece: Shown {
            scrap: Some(scrap::Scrap::new(&glyphs, seed)),
            ..*piece
        },
        torn: cells.iter().map(|&c| (c, row)).collect(),
    }
}

/// Makeshift pieces she could make this frame: of each kind she hasn't
/// made yet this visit, while the text layer has room for the glyphs.
fn builds(
    buf: &Buffer,
    visit: &Visit,
    pulls: &[scenes::Pull],
    protected: &[Rect],
) -> Vec<scenes::Build> {
    let wanted: Vec<Furniture> = scrap::MAKES
        .into_iter()
        .filter(|&item| !visit.made.iter().any(|m| m.piece.item == item))
        .collect();
    if wanted.is_empty() || visit.layer.cells().count() + scrap::GLYPHS > layer::CAP {
        return Vec::new();
    }
    let clear = |x: i32, y: i32| {
        let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        !protected.iter().any(|r| r.contains((ux, uy).into()))
    };
    // Where she'd stay once it's made, judged with it in her image: to
    // crumple it, and then to use it.
    let then = |piece: &Shown| {
        let mut terrain = visit.terrain.clone();
        terrain.furnish(visit.shown.iter().chain([piece]).map(Shown::cover));
        let crumple = piece.seat(room::Use::Crumple, 0);
        if !terrain.restful(crumple.x, crumple.y) {
            return Vec::new();
        }
        seats_of(piece, &visit.shown, &terrain, false)
            .into_iter()
            .filter(|seat| seat.what != room::Use::Crumple)
            .collect()
    };
    scenes::builds(buf, pulls, &wanted, &clear, &then)
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
        (Furniture::Lamp, true) => Color::Rgb(232, 195, 74),
        (Furniture::Lamp, false) => Color::LightYellow,
        (Furniture::Bookshelf, true) => Color::Rgb(160, 120, 79),
        (Furniture::Bookshelf, false) => Color::Yellow,
        (Furniture::Fridge, true) => Color::Rgb(231, 236, 239),
        (Furniture::Fridge, false) => Color::White,
        (Furniture::CatBed, true) => Color::Rgb(201, 69, 63),
        (Furniture::CatBed, false) => Color::Red,
    };
    Ink::new(fg, Modifier::empty())
}

/// The colour of a makeshift piece's letter at `(x, y)`.
fn scrap_ink(prop: &Shown, x: u16, y: u16) -> Option<Ink> {
    let scrap = prop.scrap?;
    let (_, rows) = prop.size();
    let dx = u16::try_from(i32::from(x) - prop.left).ok()?;
    let dy = u16::try_from(i32::from(y) - (prop.floor - i32::from(rows))).ok()?;
    let (_, color) = scrap.cell(prop.item, prop.facing, dx, dy)?;
    Some(Ink::new(color, Modifier::empty()))
}

/// How a piece looks: `layer` of it, or its box while it's boxed (open
/// while she's `unpacking` it), or with `tv` on its screen, or in
/// `state`.
fn piece_look(
    prop: &Shown,
    layer: art::Layer,
    tv: Option<art::Channel>,
    unpacking: bool,
    state: art::PieceState,
) -> Look {
    if let Some(scrap) = prop.scrap {
        let part = match layer {
            art::Layer::Whole => scrap::Part::Whole,
            art::Layer::Back | art::Layer::Bare => scrap::Part::Back,
            art::Layer::Front => scrap::Part::Front,
        };
        return Look::Scrap(prop.item, scrap, part);
    }
    match (prop.boxed, prop.item, tv) {
        (true, item, _) => Look::Parcel(item, unpacking),
        (false, Furniture::Tv, Some(channel)) => Look::Tv(channel),
        (false, item, _) if layer == art::Layer::Whole && state != art::PieceState::Plain => {
            Look::Piece(item, state)
        }
        (false, item, _) => Look::Prop(item, layer),
    }
}

/// Whether the cat is in his bed this visit (about half of them): read
/// off the visit's seed, so it's settled for the whole visit and draws
/// nothing from her generator.
fn cat_home(ledger: &Ledger) -> bool {
    let owns = ledger
        .home
        .props
        .iter()
        .any(|p| p.item == Furniture::CatBed && !p.boxed);
    owns && ledger.visit_seed(ledger.visits.saturating_sub(1)) >> 17 & 1 == 1
}

/// What state `piece` is in, given what she's doing at `now`: the lamp is
/// off while she sleeps, the fridge open as she looks in, the cat in his
/// bed (`cat`), biting at the end of a petting.
fn piece_state(piece: &Shown, osaka: &Osaka, cat: bool, now: u64) -> art::PieceState {
    use art::PieceState;
    let span = osaka.use_span();
    let doing = |what: room::Use| span.filter(|(seat, ..)| seat.what == what);
    match piece.item {
        Furniture::Lamp if doing(room::Use::Sleep).is_some() => PieceState::LampOff,
        Furniture::Fridge
            if doing(room::Use::Snack)
                .is_some_and(|(_, since, _)| now < since + osaka::FRIDGE_OPEN_MS) =>
        {
            PieceState::FridgeOpen
        }
        Furniture::CatBed if cat => match doing(room::Use::Pet) {
            Some((_, since, until))
                if now.saturating_sub(since) >= osaka::bite_at(until.saturating_sub(since)) =>
            {
                PieceState::CatBiting
            }
            _ => PieceState::Cat,
        },
        _ => PieceState::Plain,
    }
}

/// A piece of furniture as an image layer that `look`s so, standing on
/// its floor.
fn prop_layer(prop: &Shown, look: Look) -> graphics::Layer {
    let (cols, _) = prop.size();
    graphics::Layer {
        look,
        facing: prop.facing,
        at: (prop.left + i32::from(cols) / 2, prop.floor),
        standing: true,
    }
}

/// How her furniture looks this frame: what's on TV, and each piece's
/// state.
#[derive(Clone, Debug, Default)]
struct Looks {
    tv: Option<art::Channel>,
    states: Vec<(Furniture, art::PieceState)>,
}

impl Looks {
    fn state(&self, item: Furniture) -> art::PieceState {
        self.states
            .iter()
            .find(|(i, _)| *i == item)
            .map_or(art::PieceState::Plain, |&(_, state)| state)
    }
}

/// Paint one piece's line art. Returns whether it went on.
fn paint_prop_art(buf: &mut Buffer, graphics: &mut Graphics, prop: &Shown, looks: &Looks) -> bool {
    let look = piece_look(
        prop,
        art::Layer::Whole,
        looks.tv,
        false,
        looks.state(prop.item),
    );
    graphics
        .paint_layers(buf, &[prop_layer(prop, look)], &|_, _| true)
        .is_some()
}

/// Paint her furniture (as line art, or as ASCII without graphics),
/// returning what was painted over what.
fn draw_props(
    buf: &mut Buffer,
    mut graphics: Option<&mut Graphics>,
    shown: &[Shown],
    looks: &Looks,
    truecolor: bool,
) -> Vec<Frozen> {
    let mut painted = Vec::new();
    for prop in shown {
        let ink = prop_ink(prop.item, truecolor);
        // On, the TV's screen shows what's on; the cat curls in his bed.
        let cat = match looks.state(prop.item) {
            art::PieceState::Cat => Some([' ', '^', '^', ' ']),
            art::PieceState::CatBiting => Some(['!', '^', '^', '!']),
            _ => None,
        };
        let screen = prop.screen().zip(looks.tv).map(|(cells, channel)| {
            let glyphs = match channel {
                art::Channel::Snow(0) => [':', '.'],
                art::Channel::Snow(_) => ['.', ':'],
                art::Channel::Shopping(_) => ['^', '^'],
            };
            (cells, glyphs)
        });
        let unders: Vec<_> = prop
            .cells()
            .filter_map(|(x, y, glyph)| {
                let (ux, uy) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
                let glyph = screen
                    .and_then(|(cells, glyphs)| {
                        cells
                            .iter()
                            .position(|&c| c == (x, y))
                            .and_then(|i| glyphs.get(i).copied())
                    })
                    .or_else(|| {
                        let top = y == prop.floor - i32::from(prop.size().1);
                        let glyphs = cat.filter(|_| top && !prop.boxed)?;
                        glyphs.get((x - prop.left) as usize).copied()
                    })
                    .or(glyph);
                Some((ux, uy, glyph, buf.cell((ux, uy))?.clone()))
            })
            .collect();
        match graphics.as_deref_mut() {
            Some(graphics) => {
                if paint_prop_art(buf, graphics, prop, looks) {
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
                    // Makeshift, it's its own letters in their own colours.
                    let ink = prop
                        .scrap
                        .map_or(ink, |_| scrap_ink(prop, x, y).unwrap_or(ink));
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
    looks: &Looks,
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
        let what = using
            .filter(|seat| seat.item == piece.item && seat.makeshift == piece.scrap.is_some())
            .map(|s| s.what);
        let layer = match (what, front) {
            (Some(room::Use::Sleep), false) => art::Layer::Back,
            (Some(room::Use::Sleep), true) => art::Layer::Front,
            (Some(room::Use::Nap), false) => art::Layer::Bare,
            (_, false) => art::Layer::Whole,
            (_, true) => return None,
        };
        Some(piece_look(
            piece,
            layer,
            looks.tv,
            what == Some(room::Use::Unpack),
            looks.state(piece.item),
        ))
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
