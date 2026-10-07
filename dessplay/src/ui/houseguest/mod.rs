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

/// The most characters a line she says may have: what a bubble holds.
const BUBBLE_CHARS: usize = 24;

/// How many characters `text` has (its UTF-8 lead bytes), at compile
/// time.
const fn chars(text: &str) -> usize {
    let bytes = text.as_bytes();
    let (mut i, mut n) = (0, 0);
    while i < bytes.len() {
        if bytes[i] & 0xC0 != 0x80 {
            n += 1;
        }
        i += 1;
    }
    n
}

/// Whether `text` fits a bubble ([`BUBBLE_CHARS`]).
const fn fits_a_bubble(text: &str) -> bool {
    chars(text) <= BUBBLE_CHARS
}

/// A fixed line she says: a compile error if it doesn't fit a bubble.
/// Every line she says is written with it, pooled or not
/// (`mind::all_lines` lists the pooled ones).
///
/// It shadows std's `line!()` in the houseguest, so with no argument it
/// is std's: a bare `line!()` here, or one a macro expands to, still
/// gives the source line.
macro_rules! line {
    () => {
        ::core::line!()
    };
    ($text:expr) => {{
        // Items, not an inline `const { }`: those are checked only once
        // the function holding them is built, these by `cargo check` too.
        const TEXT: &str = $text;
        const _: () = assert!(
            $crate::ui::houseguest::fits_a_bubble(TEXT),
            "a line longer than a bubble"
        );
        TEXT
    }};
}

mod art;
mod brain;
mod calendar;
mod cells;
mod dissolve;
pub mod film;
mod graphics;
mod idle;
mod layer;
mod ledger;
mod mind;
mod nudge;
mod osaka;
mod rarity;
mod room;
pub mod routine;
mod rules;
mod scenes;
mod scrap;
mod script;
mod sprite;
pub mod stage;
mod stillness;
mod terrain;

use std::time::Duration;

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::{Color, Modifier};

use cells::{Ink, put};
use dissolve::{Dissolve, Frozen};
pub use film::TvPicture;
use graphics::{Graphics, Look};
pub use idle::{Busy, ChatMark, IdleView, Scrollback, asks, grow};
pub use ledger::{Ledger, Owned, Pities, Summary, TierPity};
use osaka::{Bubble, Osaka};
use room::Shown;
pub use room::{Furniture, Nook};
use sprite::{Part, Pose};
use terrain::Terrain;

/// What the shopping channel sells, in order (the TV comes first, on
/// its own): furniture, then decor. Not her wall clock: that's a gift,
/// once (see [`furnish`]).
const CATALOGUE: [Furniture; 10] = [
    Furniture::Sofa,
    Furniture::Bed,
    Furniture::Desk,
    Furniture::Lamp,
    Furniture::Bookshelf,
    Furniture::Fridge,
    Furniture::CatBed,
    Furniture::Window,
    Furniture::Plant,
    Furniture::Poster,
];
/// She buys at most once every this many visits.
const SHOP_EVERY: u64 = 3;
/// The visit her TV arrives on (the first is a first meeting).
const FIRST_TV_VISIT: u64 = 2;
/// What she says when a parcel arrives.
const PARCEL: &str = line!("A parcel!");

/// What the shopping channel sells her if she watches now: the next
/// piece of furniture she lacks, or the next piece of decor when her
/// room feeling bare is what she `needs` most (or there's no furniture
/// left to sell), at most once every [`SHOP_EVERY`] visits (any time
/// with the stage's `shop_now`), and never while something's on order
/// or still boxed.
fn advert(ledger: &Ledger, shop_now: bool, needs: &brain::Needs) -> Option<Furniture> {
    let due = shop_now || ledger.visits >= ledger.bought_on + SHOP_EVERY;
    let idle = ledger.ordered.is_none() && !ledger.home.boxed();
    let next = |decor: bool| {
        CATALOGUE
            .into_iter()
            .find(|&item| item.decor() == decor && !ledger.home.owns(item))
    };
    let item = if needs.pressing(brain::Need::Beauty) {
        next(true).or_else(|| next(false))
    } else {
        next(false).or_else(|| next(true))
    }?;
    (due && idle && ledger.home.owns(Furniture::Tv)).then_some(item)
}

/// Her wall clock as `shown` on the strips of `nooks`, if it's out of
/// its box on one (see [`osaka::Chances::clock`]).
fn clock_on(shown: &[Shown], nooks: &[(Nook, Rect)]) -> Option<osaka::ClockOn> {
    let clock = shown
        .iter()
        .find(|s| s.item == Furniture::Clock && !s.boxed && s.scrap.is_none())?;
    let strip = clock.strip?;
    let (_, extent) = room::strips(nooks).into_iter().find(|&(s, _)| s == strip)?;
    let (cols, _) = clock.size();
    Some(osaka::ClockOn {
        x: clock.left + i32::from(cols) / 2,
        floor: extent.floor,
        from: extent.from,
        to: extent.to,
    })
}

/// How pretty the room she stands in at `(x, y)` is: the beauty of what
/// shows on the strip whose floor that is (0 off any strip).
fn beauty_at(shown: &[Shown], nooks: &[(Nook, Rect)], (x, y): (i32, i32)) -> f64 {
    room::strips(nooks)
        .into_iter()
        .find(|(_, e)| e.floor == y && (e.from..e.to).contains(&x))
        .map_or(0.0, |(strip, _)| {
            shown
                .iter()
                .filter(|s| s.strip == Some(strip) && !s.boxed && s.scrap.is_none())
                .map(|s| s.item.spec().beauty)
                .sum()
        })
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

use placement::{BoxArt, Door, Figure};

/// What of hers stands in her box in a frame, as line art: her, made
/// only of her in sight, and her door.
mod placement {
    use super::art::DoorFrame;
    use super::graphics::{Layer, Look};
    use super::osaka::Osaka;
    use super::room::Shown;
    use super::sprite::{self, Face, Pose, SpriteCell};

    /// That a layer showing her (posed or waving) was made of her in
    /// sight: [`Look::Pose`] and [`Look::Wave`] carry one, and only a
    /// [`Placement`] makes it, so no layer draws her where she isn't.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub(super) struct InSight(());

    impl InSight {
        /// One for a test's own layers, with no Osaka to place.
        #[cfg(test)]
        pub(super) fn assumed() -> Self {
            Self(())
        }
    }

    /// Where her line art was placed in a frame. [`Placement::of`] alone
    /// makes one, and never of a hidden Osaka; and her looks are made
    /// only of one (see [`InSight`]): nothing of her is drawn, posed,
    /// startled or waving, where she isn't (through her door, out at
    /// work), and her goodbye shows her only where she was.
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Placement {
        x: i32,
        y: i32,
        facing: sprite::Facing,
        standing: bool,
    }

    impl Placement {
        /// Hers at `now`, if she's in sight.
        pub(super) fn of(osaka: &Osaka, now: u64) -> Option<Self> {
            (!osaka.hidden(now)).then(|| Self {
                x: osaka.x,
                y: osaka.y,
                facing: osaka.facing,
                standing: osaka.standing(),
            })
        }

        /// Her in `pose` with `face`, where she stood.
        pub(super) fn pose(self, pose: Pose, face: Face) -> Layer {
            self.layer(Look::Pose(pose, face, InSight(())))
        }

        /// Her waving goodbye (arm up, or down), where she stood.
        pub(super) fn wave(self, raised: bool) -> Layer {
            self.layer(Look::Wave(raised, InSight(())))
        }

        fn layer(self, look: Look) -> Layer {
            Layer {
                look,
                facing: self.facing,
                at: (self.x, self.y),
                standing: self.standing,
            }
        }

        /// The floor cell under her feet, standing.
        #[cfg(test)]
        fn feet(self) -> Option<(i32, i32)> {
            self.standing.then_some((self.x, self.y))
        }
    }

    /// Her door in space where it stood in a frame (behind her, or alone
    /// once she's through it).
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Door {
        frame: DoorFrame,
        x: i32,
        y: i32,
        facing: sprite::Facing,
        standing: bool,
    }

    impl Door {
        /// Hers at `now`, if one stands.
        pub(super) fn of(osaka: &Osaka, now: u64) -> Option<Self> {
            osaka.door(now).map(|frame| Self {
                frame,
                x: osaka.x,
                y: osaka.y,
                facing: osaka.facing,
                standing: osaka.standing(),
            })
        }

        /// Her door standing closed on the floor at `(x, y)`, facing
        /// `facing`, with her out of sight behind it (she's out: see
        /// `State::Away`).
        pub(super) fn closed(x: i32, y: i32, facing: sprite::Facing) -> Self {
            Self {
                frame: DoorFrame::Closed,
                x,
                y,
                facing,
                standing: true,
            }
        }

        /// The door as it stood.
        pub(super) fn layer(self) -> Layer {
            Layer {
                look: Look::Door(self.frame),
                facing: self.facing,
                at: (self.x, self.y),
                standing: self.standing,
            }
        }

        /// Its glyphs (as ASCII draws it), at their columns and rows.
        pub(super) fn cells(self) -> impl Iterator<Item = (i32, i32, char)> {
            sprite::door_cells(self.frame as usize, self.facing)
                .into_iter()
                .map(move |SpriteCell { dx, dy, glyph, .. }| (self.x + dx, self.y + dy, glyph))
        }

        /// The floor cell under it, standing.
        #[cfg(test)]
        fn feet(self) -> Option<(i32, i32)> {
            self.standing.then_some((self.x, self.y))
        }
    }

    /// What of hers stands in her box at `now`: her if she's in sight,
    /// her door if one stands, and never neither ([`Figure::of`] is
    /// `None` then: out of sight with her door shut, nothing of hers is
    /// there).
    #[derive(Clone, Copy, Debug)]
    pub(super) struct Figure {
        her: Option<Placement>,
        door: Option<Door>,
    }

    impl Figure {
        /// What stands in her box at `now`, if anything does.
        pub(super) fn of(osaka: &Osaka, now: u64) -> Option<Self> {
            let (her, door) = (Placement::of(osaka, now), Door::of(osaka, now));
            (her.is_some() || door.is_some()).then_some(Self { her, door })
        }

        /// Her door standing alone, with her out (her closed door in her
        /// empty home): nothing of her in it.
        pub(super) fn door_alone(door: Door) -> Self {
            Self {
                her: None,
                door: Some(door),
            }
        }

        /// Her, if she's in sight.
        pub(super) fn her(self) -> Option<Placement> {
            self.her
        }

        /// Her door, if one stands.
        pub(super) fn door(self) -> Option<Door> {
            self.door
        }

        /// The floor cell under it, if it stands (her, else her door:
        /// both stand where she does).
        #[cfg(test)]
        fn feet(self) -> Option<(i32, i32)> {
            self.her
                .and_then(Placement::feet)
                .or_else(|| self.door.and_then(Door::feet))
        }
    }

    /// The line art of her box as a frame placed it: what stood in it,
    /// and the pieces it overlapped, drawn in the same image (two images
    /// would cut each other out).
    #[derive(Clone, Debug)]
    pub(super) struct BoxArt {
        pub(super) figure: Figure,
        pub(super) with: Vec<Shown>,
    }

    impl BoxArt {
        /// The floor cell under what stood in her box, if it stood: the
        /// image redraws that floor's line.
        #[cfg(test)]
        pub(super) fn feet(&self) -> Option<(i32, i32)> {
            self.figure.feet()
        }
    }
}

struct Visit {
    osaka: Osaka,
    /// Counted, or a dash home while she's out at school (A16).
    kind: Kind,
    terrain: Terrain,
    /// What she painted in the last frame, with the real cells beneath —
    /// the dissolve's frozen composite if activity arrives now.
    painted: Vec<Frozen>,
    /// Her box's line art in the last frame, if it was placed: her (in
    /// sight) and her door (standing), with the pieces they overlapped.
    /// Out of sight with her door shut, there's nothing in her box to
    /// draw, and none.
    image: Option<BoxArt>,
    /// Text she has moved.
    layer: layer::TextLayer,
    /// What the last frame offered her (lines to pull).
    chances: osaka::Chances,
    /// Her furniture as placed in the last frame.
    shown: Vec<Shown>,
    /// The pieces drawn on their own in the last frame: all of them in
    /// ASCII; in line art, those not in her box's image (it overlapped
    /// the rest, which show only if it was placed).
    apart: Vec<Shown>,
    /// Makeshift furniture she has made of text this visit.
    made: Vec<Made>,
    /// The next piece she makes.
    next_made: room::MadeId,
    /// The text she held torn off its line at the last paint (see
    /// `Osaka::holding`).
    reel: Option<scenes::Held>,
    /// The flap a parcel just came in through, and when.
    flap: Option<(room::Flap, u64)>,
    /// The rules of her home broken in the last frame.
    broken: Vec<rules::Broken>,
    /// How she'd put right the rule she would mend, as last worked out.
    repairs: Vec<rules::Repair>,
    /// What that was worked out from (a hash of her home, the panes,
    /// the rule, and the text she's moved), and when.
    mending: Option<(u64, u64)>,
    /// A piece she set down, for the next paint to take where it goes
    /// (if it still fits there) or refuse.
    set_down: Option<(Furniture, rules::Placement)>,
    /// What the last paint made of the move she's making.
    judging: Option<Judging>,
    /// Where the piece in her pocket stood (its place is kept for it:
    /// nothing she makes or moves goes there).
    ghost: Option<Rect>,
    size: (u16, u16),
    /// The visit began at night by her routine: at its first paint she's
    /// tucked in, if her bed or sofa is shown ([`Osaka::tuck_in`]).
    tuck: bool,
    /// How her furniture looked in the last frame (a goodbye keeps it).
    looks: Looks,
}

/// What a paint makes of the move she's making (the `osaka::Judged`
/// she sees, once the terrain is read): where the piece would stand set
/// down, if the move still puts its rule right and fits; where it shows
/// now (to lift it); and where it stands at its anchor, shown or not.
#[derive(Clone, Copy, Debug)]
struct Judging {
    piece: Furniture,
    to: rules::Placement,
    pocket: bool,
    target: Option<Shown>,
    showing: Option<Shown>,
    laid: Option<Shown>,
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

/// A makeshift piece, the glyphs torn off to make it (their holes stay
/// while it does), and what she made it for: what she means to do with
/// it is kept on it, where nothing that interrupts her can lose it.
#[derive(Clone, Debug)]
struct Made {
    piece: Shown,
    torn: Vec<(u16, u16)>,
    purpose: room::Use,
    /// She has started using it (crumpling it into shape doesn't count).
    used: bool,
}

impl Made {
    /// As she sees it, choosing what to do.
    fn mine(&self) -> Option<osaka::Mine> {
        let scrap = self.piece.scrap?;
        Some(osaka::Mine {
            id: scrap.id,
            item: self.piece.item,
            purpose: self.purpose,
            done: scrap.done(),
            used: self.used,
            at: (
                self.piece.left + i32::from(self.piece.size().0) / 2,
                self.piece.floor,
            ),
        })
    }
}

struct Leaving {
    dissolve: Dissolve,
    /// Her box's line art from the last frame, held until she bursts
    /// into letters: her (startled, then waving) if she was in sight,
    /// her door as it stood, and the pieces they overlapped.
    image: Option<BoxArt>,
    /// Her face for that first beat: startled, or (woken in the night) a
    /// sleepy blink.
    startled: sprite::Face,
    /// Her furniture's line art drawn on its own in the last frame, held
    /// until the rain.
    props: Vec<Shown>,
    /// How it looked then (A22): the TV on, the lamp off, as they were.
    looks: Looks,
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
    /// She enters on the next paint, as `How` says.
    Arriving(How),
    Visiting(Box<Visit>),
    Leaving(Box<Leaving>),
    /// She's out by her routine (at school): her home stands empty, the
    /// lamp off, her closed door where she went out (phase 5b D3, A22).
    Away(Box<Empty>),
}

/// How she comes in (A6), which says what calls it off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum How {
    /// The idle gate opened (or the stage cued her): local input or a chat
    /// line calls it off.
    Idle,
    /// Home by her routine at the end of school, out of her closed door
    /// (where it stood, if it did): only the gate shutting (`!open`)
    /// calls it off.
    Return(Option<DoorAt>),
    /// A dash home while she's out at school (D3a, A16), out of her
    /// closed door (where it stands, if it does): only the gate shutting
    /// calls it off.
    Dash,
}

/// What a visit is (A16).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A visit: counted (`Ledger::visits`), and her body's stream seeded
    /// by its number.
    Normal,
    /// A dash home while she's out at school (D3a), for something she
    /// forgot or for the chat's accordion: not counted (she's still out),
    /// her body's stream salted apart from the coming visit's, no parcel
    /// delivered, and her routine sends her out again by her door.
    Dash,
}

/// Where her door stands: its floor spot, and which way it faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DoorAt {
    x: i32,
    y: i32,
    facing: sprite::Facing,
}

/// She's out by her routine (at school), her coming home not yet
/// decided (`Guest::out`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Out {
    /// Where her closed door stands: where she went out, or the nearest
    /// floor spot that fits it once that doesn't (a resize); `None` until
    /// a frame finds it one (a cold start). Kept while she's out, her
    /// empty home shown or not (a visitor's key, an overlay, a frame too
    /// small), so it stands where she left until she comes out of it.
    door: Option<DoorAt>,
}

/// Her home while she's out by her routine (`State::Away`): painted as
/// a visit paints it, without her (and without anything a visit's paint
/// would set going: no gift, no order, no parcel), her closed door
/// standing where she went out (kept in `Guest::out`).
struct Empty {
    /// The cat's in his bed while she's out, as for the coming visit (so
    /// he doesn't change as she comes home).
    cat: bool,
    /// Her furniture as projected in the last frame.
    shown: Vec<Shown>,
    /// The pieces drawn on their own in the last frame: all of them in
    /// ASCII; in line art, those not in her door's image.
    apart: Vec<Shown>,
    /// Her door's line art in the last frame, if it was placed, with the
    /// pieces it overlapped (a goodbye holds it until the rain).
    image: Option<BoxArt>,
    /// What the last frame painted, with the real cells beneath: a
    /// goodbye's frozen composite, a focused pane's rain.
    painted: Vec<Frozen>,
    /// How her furniture looked in the last frame (a goodbye keeps it).
    looks: Looks,
    size: (u16, u16),
}

impl Empty {
    /// Her home about to stand empty, before its first frame: the cat
    /// as the coming visit has him (or the stage's).
    fn new(ledger: &Ledger, cat_now: bool) -> Self {
        Self {
            cat: cat_now || cat_home_of(ledger, ledger.visits),
            shown: Vec::new(),
            apart: Vec::new(),
            image: None,
            painted: Vec::new(),
            looks: Looks::default(),
            size: (0, 0),
        }
    }
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
    /// The ledger as restored at startup: the exit save writes hers only
    /// when it differs ([`Guest::final_ledger`]). `None` when the stored
    /// one couldn't be read ([`Guest::keep_unsaved`]): whatever she may
    /// save over it differs from it.
    loaded: Option<Ledger>,
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
    /// The stage: at the next paint, a sofa turned away from her TV (see
    /// [`stage::Scene::Arrange`]).
    arranging: bool,
    /// When to poke the scrollback accordion, and its shake.
    nudge: nudge::Nudge,
    /// What of hers is raining out: what stood in a pane just focused,
    /// what she'd moved and made as she went out by her routine, her
    /// closed door broken in on. Hers, not her state's: it rains on to
    /// its end whatever comes meanwhile (she comes home, a goodbye, she
    /// goes with nothing of hers shown, school ends with the client
    /// busy), tended once a tick ([`Guest::advance`]: the tick it ends on
    /// is drawn too), painted once a frame over every state
    /// ([`Guest::paint`]), and woken for. Only her being sent away
    /// (switched off, moved out) or her room going (no room, a resize)
    /// ends it early: the user's say, or the geometry it froze against
    /// gone ([`Guest::vanish`]).
    fades: Vec<Dissolve>,
    errand: Option<Errand>,
    /// Monotonic millis of the last accrual of her clock (`None` before
    /// the first: it only latches). See [`Guest::accrue`].
    clock_at: Option<u64>,
    /// Game millis below the whole minute `ledger.clock` holds.
    clock_rem: u64,
    /// Idle real millis below the whole minute `ledger.idle_min` holds.
    idle_rem: u64,
    /// `ledger.clock` when the ledger was last handed out for saving:
    /// time alone dirties it again once [`CLOCK_BATCH`] more has passed.
    clock_saved: u64,
    /// The most one accrual may add, in real millis (`None`: no cap).
    cap: Option<u64>,
    /// The real date, as the shell last gave it ([`Guest::set_date`]):
    /// only ever a date, never a time of day.
    date: Option<chrono::NaiveDate>,
    /// The vacation flag of the game day of the last tick, read from
    /// `date` at that day's first tick and held through it (A12). Held
    /// per process, never saved: a restart is a cold start, which
    /// re-derives her state from game time and the date as it is then.
    day_latch: Option<routine::Latch>,
    /// The routine reaches her mind: always, but in the tests that turn
    /// it off ([`Guest::unfed`]), where `Osaka::day` is `None`
    /// everywhere and she behaves exactly as before the clock.
    feed_clock: bool,
    /// Her routine's slot at the last tick, while the clock is fed: a
    /// change dirties the ledger, so the record is consistent at
    /// departures and bedtimes.
    slot_was: Option<routine::Slot>,
    /// She's out by her routine (at school), and her coming home isn't
    /// decided yet: set while it's school time and she isn't visiting,
    /// and as a visit ends with her out through her door; decided (and
    /// cleared) as school ends, and cleared as a visit begins. Held per
    /// process: a cold start in school hours sets it at its first tick.
    /// Her closed door's spot lives here, not in `State::Away`, so it
    /// outlasts her empty home going and coming back.
    out: Option<Out>,
    /// Her line budget as her last visit left it, while the clock is fed:
    /// the game day, and the beat lines she said that day. A visit later
    /// the same game day carries them on (the budget is the day's, not
    /// the visit's). Held per process.
    lines_today: Option<(u64, usize)>,
    /// The game day and slot whose first snack had her meal's line
    /// ("Breakfast!", "Dinner time~"), as her last visit left it: a visit
    /// later in the same slot doesn't say it again. Held per process.
    meal_today: Option<(u64, routine::Slot)>,
    /// What's rare and open on a game day, as her last visit left it
    /// (drawn as the day's first visit began, or as she woke into it): a
    /// visit later that day carries it on, so a day is drawn once and
    /// has at most one new rare thing. Held per process.
    rares_today: Option<(u64, rarity::Rares)>,
    /// Her night as her last visit left it (keyed on its morning): a
    /// visit later that night carries it on, so what's once a night (the
    /// Dream, her midnight snack) is once a night, not once a visit.
    /// Held per process.
    night_today: Option<osaka::Night>,
}

/// Her clock runs this many times faster than real time.
pub const CLOCK_SPEED: u64 = 6;
/// A game minute, in game millis.
const GAME_MINUTE_MS: u64 = 60_000;
/// Game minutes of her clock that dirty the ledger on their own (5 real
/// minutes): time is saved in batches, events at once (phase 5b D1,
/// Saving). A crash loses less than this.
const CLOCK_BATCH: u64 = 30;

/// Her clock at a moment, to read at any other: `game` game millis since
/// Monday 16:00 of game day 0 at the monotonic millis `at`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GameClock {
    /// Monotonic millis of the reading.
    pub at: u64,
    /// Game millis since the start of her clock, at `at`.
    pub game: u64,
}

impl GameClock {
    /// Her game millis at the monotonic millis `t`, earlier than `at` or
    /// later (a catch-up decision is earlier); never below zero.
    pub fn at(&self, t: u64) -> u64 {
        let game =
            i128::from(self.game) + i128::from(CLOCK_SPEED) * (i128::from(t) - i128::from(self.at));
        u64::try_from(game.max(0)).unwrap_or(u64::MAX)
    }

    /// The earliest monotonic millis at which her clock reads `game` or
    /// later (for a `game` above zero: below her start it reads 0):
    /// [`GameClock::at`]'s inverse, rounded up (so `at(when(g)) >= g`),
    /// earlier than `at` for a time already past; never below zero.
    pub fn when(&self, game: u64) -> u64 {
        let speed = i128::from(CLOCK_SPEED);
        let ahead = i128::from(game) - i128::from(self.game);
        // Ceiling division, for either sign.
        let real = ahead.div_euclid(speed) + i128::from(ahead.rem_euclid(speed) != 0);
        u64::try_from((i128::from(self.at) + real).max(0)).unwrap_or(u64::MAX)
    }
}

impl Guest {
    /// A guest whose behaviour is fully determined by `seed`, meeting
    /// her for the first time.
    pub fn new(seed: u64) -> Self {
        Self::restore(Ledger::new(seed))
    }

    /// The guest `ledger` records.
    pub fn restore(ledger: Ledger) -> Self {
        let ledger_clock = ledger.clock;
        Self {
            rng: Rng(ledger.visit_seed(ledger.visits)),
            loaded: Some(ledger.clone()),
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
            arranging: false,
            nudge: nudge::Nudge::default(),
            fades: Vec::new(),
            errand: None,
            clock_at: None,
            clock_rem: 0,
            idle_rem: 0,
            clock_saved: ledger_clock,
            cap: None,
            date: None,
            // A cold start: the day latches afresh (see the field).
            day_latch: None,
            feed_clock: true,
            slot_was: None,
            out: None,
            lines_today: None,
            meal_today: None,
            rares_today: None,
            night_today: None,
        }
    }

    /// Never hand out the ledger for saving (the stored one couldn't be
    /// read, and is kept as it is).
    pub fn keep_unsaved(&mut self) {
        self.persist = false;
        self.loaded = None;
    }

    /// Her ledger, when it changed since last asked and is to be saved.
    pub fn ledger_to_save(&mut self) -> Option<Ledger> {
        if !std::mem::take(&mut self.unsaved) {
            return None;
        }
        // Whatever dirtied it, the clock's batch starts again here.
        self.clock_saved = self.ledger.clock;
        self.persist.then(|| self.ledger.clone())
    }

    /// Her ledger as it stands, for the save at exit: when it's saved
    /// at all, and differs from the one restored at startup (so a client
    /// that never met her writes no record, and a moved-out one always
    /// writes over a record that couldn't be read). Deliberately not
    /// [`Guest::ledger_to_save`]'s flag: a handout parked on a full
    /// action queue, or queued and dropped at shutdown, already cleared
    /// it, and this is its only way out.
    ///
    /// Her clock is brought up to `now` first: the one accrual outside
    /// [`Guest::advance`], so a clean exit loses none of her time.
    pub fn final_ledger(&mut self, now: u64) -> Option<Ledger> {
        self.accrue(now);
        (self.persist && self.loaded.as_ref() != Some(&self.ledger)).then(|| self.ledger.clone())
    }

    /// "Osaka moved out": her home and record are gone; the next visit
    /// is a first meeting, drawn from `seed`. She leaves at once.
    pub fn move_out(&mut self, seed: u64) {
        tracing::info!("houseguest moved out");
        self.ledger = Ledger::new(seed);
        // Her clock starts again at Monday 16:00 (`clock_at` stands: it's
        // when time was last counted, not whose).
        self.clock_rem = 0;
        self.idle_rem = 0;
        // The next take resets this anyway (the move-out is unsaved);
        // here it keeps `clock_saved <= ledger.clock` in between.
        self.clock_saved = 0;
        // Monday 16:00 again: the day's latch and slot are another day's.
        self.day_latch = None;
        self.slot_was = None;
        self.lines_today = None;
        self.meal_today = None;
        self.rares_today = None;
        self.night_today = None;
        self.unsaved = true;
        self.persist = true;
        self.gift = None;
        self.shop_now = false;
        self.out = None;
        // Gone at once; a goodbye under way rains to its end.
        if !matches!(self.state, State::Leaving(_)) {
            self.vanish();
        }
    }

    /// She's gone at once, with no goodbye, sent away (switched off,
    /// moved out) or out of room (no room, a resize): absent, and nothing
    /// of hers rains on. The one place her rains are cut short; every
    /// other way she goes (or doesn't come) lets them fall to their end
    /// ([`Guest::fades`]).
    fn vanish(&mut self) {
        self.state = State::Absent;
        self.fades.clear();
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
            // A sofa and a TV, set up at the next paint (it needs the
            // frame).
            stage::Scene::Arrange => self.arranging = true,
            _ => {
                if let Some(what) = scene.furniture()
                    && let Some(&item) = Furniture::ALL
                        .iter()
                        .find(|&&item| item.spec().uses.contains(&what))
                    && !self.ledger.home.owns(item)
                {
                    self.gift = Some(item);
                }
            }
        }
        let dash = matches!(scene, stage::Scene::DashIn | stage::Scene::DashForgot);
        let visiting = matches!(self.state, State::Visiting(_));
        if dash && !visiting {
            // A dash home, forced: in through her door (at school time,
            // out again after). On a visit under way, as any scene but
            // an arrival, the visit goes on (the stage puts her through
            // a door).
            self.state = State::Arriving(How::Dash);
        } else if scene == stage::Scene::Arrive || !visiting {
            // From her empty home too: an arrival like any (at school
            // time her routine sends her out again at her first
            // decision).
            self.state = State::Arriving(How::Idle);
        }
        self.cue = Some(scene);
    }

    /// The stage: make `want` pressing (it weighs on her next choice).
    pub fn press(&mut self, want: stage::Want) {
        if let State::Visiting(visit) = &mut self.state {
            visit.osaka.press(want.need());
        }
    }

    /// The stage: her next mood, for the rest of this visit.
    pub fn next_mood(&mut self) {
        if let State::Visiting(visit) = &mut self.state {
            let mood = visit.osaka.mood().next();
            visit.osaka.set_mood(mood);
        }
    }

    /// What she's playing at `now`, while she's visiting (for the
    /// stage): any prelude, her own script and any coda, the one playing
    /// with which of its keys.
    pub fn playing(&self, now: u64) -> Option<String> {
        match &self.state {
            State::Visiting(visit) => visit.osaka.playing_note(now),
            State::Absent | State::Arriving(_) | State::Leaving(_) | State::Away(_) => None,
        }
    }

    /// Her needs, while she's visiting (for the stage).
    pub fn mood(&self) -> Option<String> {
        match &self.state {
            State::Visiting(visit) => Some(format!(
                "{:?}: {}",
                visit.osaka.mood(),
                visit.osaka.needs().summary()
            )),
            State::Absent | State::Arriving(_) | State::Leaving(_) | State::Away(_) => None,
        }
    }

    /// Her latest decision, and why, while she's visiting (for the
    /// stage).
    pub fn explain(&self) -> Option<String> {
        match &self.state {
            State::Visiting(visit) => visit.osaka.explain().map(ToString::to_string),
            State::Absent | State::Arriving(_) | State::Leaving(_) | State::Away(_) => None,
        }
    }

    /// Her rooms, and what each is from what's in it (for the stage).
    pub fn rooms(&self) -> String {
        self.ledger
            .home
            .rooms()
            .iter()
            .map(|&(room::Strip::Bottom(nook), role)| format!("{nook:?} {role:?}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The rules of her home broken now, while she's visiting (for the
    /// stage): those she has felt marked `*`.
    pub fn broken(&self) -> String {
        match &self.state {
            State::Visiting(visit) => visit
                .broken
                .iter()
                .map(|b| {
                    let felt = if visit.osaka.felt().contains(&b.key) {
                        "*"
                    } else {
                        ""
                    };
                    format!("{}{felt}", b.label())
                })
                .collect::<Vec<_>>()
                .join(", "),
            State::Absent | State::Arriving(_) | State::Leaving(_) | State::Away(_) => {
                String::new()
            }
        }
    }

    /// How she'd put right the rule of her home she would mend, while
    /// she's visiting (for the stage): the cheapest way, if any.
    pub fn repair(&self) -> String {
        match &self.state {
            State::Visiting(visit) => visit
                .chances
                .repairs
                .first()
                .map(|r| format!("{}: {}", r.key.label(), r.label()))
                .unwrap_or_default(),
            State::Absent | State::Arriving(_) | State::Leaving(_) | State::Away(_) => {
                String::new()
            }
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

    /// A still of the film for her TV (phase 5c D7), shown in place of
    /// its programme from the next paint on; `None` when the film changed
    /// or stopped (any still goes: a failed fetch is no reason to call
    /// this). Line art only: without it there's nothing to show it on.
    /// Only her drawing reads it, never what she does (the purity rule:
    /// her trace and her ASCII frames are the same with or without it).
    pub fn set_tv_picture(&mut self, picture: Option<TvPicture>) {
        if let Some(graphics) = &mut self.graphics {
            tracing::trace!(picture = picture.is_some(), "houseguest: TV picture");
            graphics.set_film(picture);
        }
    }

    /// Whether her TV wants a still of the film (phase 5c D7): drawn in
    /// line art, she's on her way to her TV's programme or watching it
    /// ([`Osaka::tv_bound`]), and her TV is the real one (not one made of
    /// text). The shell asks the player for one by it, at most once a
    /// minute. Output only.
    pub fn tv_wants_picture(&self) -> bool {
        let State::Visiting(visit) = &self.state else {
            return false;
        };
        self.graphics.is_some()
            && visit.osaka.tv_bound()
            && visit
                .shown
                .iter()
                .any(|s| s.item == Furniture::Tv && s.scrap.is_none() && !s.boxed)
    }

    /// Whether she is on screen (visiting or leaving): not while she's
    /// out, her home standing empty.
    pub fn present(&self) -> bool {
        match self.state {
            State::Visiting(_) | State::Leaving(_) => true,
            State::Absent | State::Arriving(_) | State::Away(_) => false,
        }
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
        // Resident: an arrival on the idle gate waits for the next quiet;
        // her coming home doesn't (A6), nor does her empty home go (her
        // focused pane is kept clear, as ever).
        match self.state {
            State::Arriving(How::Idle) => self.state = State::Absent,
            State::Visiting(_) => self.shake = true,
            State::Arriving(How::Return(_) | How::Dash)
            | State::Absent
            | State::Leaving(_)
            | State::Away(_) => {}
        }
    }

    fn leave(&mut self, now: u64) {
        match std::mem::replace(&mut self.state, State::Absent) {
            State::Arriving(How::Idle) | State::Absent => {}
            // Her coming home: only the gate shutting calls it off (A6).
            arriving @ State::Arriving(How::Return(_) | How::Dash) => self.state = arriving,
            // Her empty home rains out: her furniture and her door, with
            // no wave (she isn't there). She's still out, her door's spot
            // kept for when it shows again.
            State::Away(empty) => {
                tracing::info!("houseguest: her empty home goes");
                if !empty.painted.is_empty() {
                    let origin = self
                        .closed_door()
                        .map_or(i32::from(empty.size.0) / 2, |door| door.x);
                    let dissolve =
                        Dissolve::new(now, empty.painted, origin, self.truecolor, empty.size);
                    // What the last frame showed, as it showed it, until
                    // the rain: the pieces on their own, and her door's
                    // image (with the pieces it took in, and nobody in
                    // it to wave).
                    self.state = State::Leaving(Box::new(Leaving {
                        dissolve,
                        image: empty.image,
                        startled: sprite::Face::Surprised,
                        props: empty.apart,
                        looks: empty.looks,
                    }));
                }
            }
            State::Visiting(visit) => {
                tracing::info!("houseguest leaving");
                // In the night she isn't startled: she blinks, half
                // asleep, then waves.
                let startled = if visit.osaka.drowsy(now) {
                    sprite::Face::Blink
                } else {
                    sprite::Face::Surprised
                };
                if !visit.painted.is_empty() {
                    let dissolve = Dissolve::new(
                        now,
                        visit.painted,
                        visit.osaka.x,
                        self.truecolor,
                        visit.size,
                    )
                    .startled(startled);
                    // What the last frame showed, as it showed it.
                    self.state = State::Leaving(Box::new(Leaving {
                        dissolve,
                        image: visit.image,
                        startled,
                        props: visit.apart,
                        looks: visit.looks,
                    }));
                }
            }
            leaving @ State::Leaving(_) => self.state = leaving,
        }
    }

    /// The most one accrual of her clock may add (`None`, the default:
    /// no cap). The shell caps it against a suspend that `Instant` might
    /// count (std leaves that unspecified); tests and censuses jump
    /// hours on purpose and leave it off.
    pub fn cap_steps(&mut self, cap: Option<u64>) {
        self.cap = cap;
    }

    /// Her clock, to read at any moment: as of the last accrual, or as of
    /// `now` before the first. `None` until she has met you (and again
    /// after she moves out, until she next does): her clock stands still
    /// at Monday 16:00 then, which a reading, always running, can't say.
    /// Pure.
    pub fn game_clock(&self, now: u64) -> Option<GameClock> {
        (self.ledger.visits > 0).then(|| GameClock {
            at: self.clock_at.unwrap_or(now),
            game: self.game_ms(),
        })
    }

    /// The real date, from the shell before every [`Guest::advance`]
    /// (`None`: unknown, and no calendar or vacation). Only a date
    /// crosses over, never a time of day: her routine runs on her own
    /// clock.
    pub fn set_date(&mut self, date: Option<chrono::NaiveDate>) {
        if self.date != date {
            tracing::trace!(?date, "houseguest date");
        }
        self.date = date;
    }

    /// The real date as last set: the one place it's held, where
    /// whatever of hers needs the date reads it (the [`IdleView`]
    /// carries none).
    pub fn date(&self) -> Option<chrono::NaiveDate> {
        self.date
    }

    /// Feed her routine to her mind, or not (tests: fed is the default).
    #[cfg(test)]
    pub(crate) fn set_feed_clock(&mut self, on: bool) {
        self.feed_clock = on;
    }

    /// This guest with her routine never reaching her (A5): her clock
    /// still accrues into the ledger, but `tick` gets no clock, no
    /// boundary cuts in, and no visit or morning is keyed on the game
    /// day. She behaves exactly as before the clock: the `UNFED_*`
    /// golden tables hold her to it.
    #[cfg(test)]
    pub(crate) fn unfed(mut self) -> Self {
        self.feed_clock = false;
        self
    }

    /// Everything derived from her clock, brought up to `now` after any
    /// change to it (by [`Guest::advance`] and [`Guest::skip_clock`],
    /// the only two): the day's latch (A12), and, while the clock is fed,
    /// the slot (a change dirties the ledger) and the visiting Osaka's
    /// clock. Returns her routine as her mind gets it (`None` unless
    /// fed). Draws nothing: unfed, she behaves as before the clock.
    fn sync_clock(&mut self, now: u64) -> Option<routine::Clock> {
        let clock = self.routine_clock(now);
        if let Some(clock) = clock
            && self.day_latch != Some(clock.latch)
        {
            tracing::trace!(
                day = clock.latch.day,
                vacation = clock.latch.vacation,
                "houseguest day latched"
            );
            self.day_latch = Some(clock.latch);
        }
        let fed = clock.filter(|_| self.feed_clock);
        if let Some(clock) = fed {
            // A slot change saves her record (in the batch's stead).
            let slot = clock.day(now).slot;
            if self.slot_was.is_some_and(|was| was != slot) {
                tracing::trace!(?slot, "houseguest routine slot");
                self.unsaved = true;
            }
            self.slot_was = Some(slot);
        }
        if let State::Visiting(visit) = &mut self.state {
            visit.osaka.read_clock(fed);
        }
        fed
    }

    /// Her clock with the day's latch, whatever the feed: what the stage
    /// shows and the routine questions read.
    fn routine_clock(&self, now: u64) -> Option<routine::Clock> {
        let game = self.game_clock(now)?;
        Some(routine::Clock::read(game, now, self.day_latch, self.date))
    }

    /// The routine at `now`, as the mind sees it: `None` unless the
    /// clock is fed (and running).
    fn fed_day(&self, now: u64) -> Option<routine::DayTime> {
        self.routine_clock(now)
            .filter(|_| self.feed_clock)
            .map(|clock| clock.day(now))
    }

    /// The routine at `now`, as the mind sees it (tests): see
    /// [`Guest::fed_day`].
    #[cfg(test)]
    pub(crate) fn day(&self, now: u64) -> Option<routine::DayTime> {
        self.fed_day(now)
    }

    /// Her game time and the part of her day it is, for the stage:
    /// "Mon 16:05 Afternoon". `None` until she has met you.
    pub fn clock_label(&self, now: u64) -> Option<String> {
        self.routine_clock(now)
            .map(|clock| clock.day(now).to_string())
    }

    /// The stage: skip her clock forward to the next boundary of her
    /// routine (never back). Nothing before she has met you.
    pub fn skip_clock(&mut self, now: u64) {
        // Brought up to `now` first: the skip starts from her time.
        self.accrue(now);
        let Some(clock) = self.routine_clock(now) else {
            return;
        };
        let to = clock.next_boundary(clock.game.at(now));
        let minutes = (to / GAME_MINUTE_MS).min(ledger::MINUTES_MAX);
        // Forward only: `to` is always later, but clamped at the clock's
        // end it may not be, and dropping `clock_rem` would step back.
        if minutes <= self.ledger.clock {
            return;
        }
        self.ledger.clock = minutes;
        self.clock_rem = 0;
        self.unsaved = true;
        let _ = self.sync_clock(now);
        tracing::debug!(clock = ?self.clock_label(now), "houseguest clock skipped");
    }

    /// When the client has been quiet for `delay`: the one sum every
    /// idle check reads (each with its own word on the setting).
    fn quiet_until(&self, delay: Duration) -> u64 {
        self.quiet_since
            .saturating_add(u64::try_from(delay.as_millis()).unwrap_or(u64::MAX))
    }

    /// When the idle gate opens: the client idle for the delay, with the
    /// setting on (as of the last paint). `None` while it's shut.
    fn gate_from(&self) -> Option<u64> {
        self.delay
            .filter(|_| self.open)
            .map(|delay| self.quiet_until(delay))
    }

    /// Whether the idle gate is open at `now` (her arrival, and her pity
    /// counter).
    fn gate_open(&self, now: u64) -> bool {
        self.gate_from().is_some_and(|from| now >= from)
    }

    /// Count the time since the last accrual: her clock, at
    /// [`CLOCK_SPEED`], and the idle minutes since the gate opened. Only
    /// once she has met you; the latch moves to `now` either way, so her
    /// first meeting is at Monday 16:00, give or take the tick it falls
    /// in (she meets you on a paint, between ticks). Time alone dirties
    /// the ledger once [`CLOCK_BATCH`] game minutes have passed since it
    /// was last handed out.
    ///
    /// However the time is cut into ticks, it counts the same, as long
    /// as nothing between them moves the idle gate. `activity` and
    /// `observe` (a paint) do, without an accrual first: the gate's open
    /// time since the last tick isn't counted then, at most a tick (a
    /// second) each time it shuts. The clock itself loses nothing.
    fn accrue(&mut self, now: u64) {
        let Some(prev) = self.clock_at.replace(now.max(self.clock_at.unwrap_or(0))) else {
            return;
        };
        let mut from = prev.min(now);
        if let Some(cap) = self.cap
            && now - from > cap
        {
            tracing::debug!(
                step_ms = now - from,
                cap_ms = cap,
                "houseguest clock step capped"
            );
            from = now - cap;
        }
        if self.ledger.visits == 0 {
            return;
        }
        let game = self
            .clock_rem
            .saturating_add(CLOCK_SPEED.saturating_mul(now - from));
        self.ledger.clock = self
            .ledger
            .clock
            .saturating_add(game / GAME_MINUTE_MS)
            .min(ledger::MINUTES_MAX);
        self.clock_rem = game % GAME_MINUTE_MS;
        if let Some(gate) = self.gate_from()
            && now > gate.max(from)
        {
            let idle = self.idle_rem.saturating_add(now - gate.max(from));
            self.ledger.idle_min = self
                .ledger
                .idle_min
                .saturating_add(idle / 60_000)
                .min(ledger::MINUTES_MAX);
            self.idle_rem = idle % 60_000;
        }
        if self.ledger.clock.saturating_sub(self.clock_saved) >= CLOCK_BATCH {
            self.unsaved = true;
        }
    }

    /// Advance to `now` on a timer tick. Returns whether the screen
    /// could change (the shell redraws only then).
    pub fn advance(&mut self, now: u64) -> bool {
        // Her clock first, and on its own it changes nothing on screen.
        let before = self.game_ms();
        self.accrue(now);
        let fed = self.sync_clock(now);
        let nudge = self.nudge.advance(now);
        // School time, by her routine as fed to her (unfed, never).
        let school = fed.is_some_and(|clock| clock.day(now).slot == routine::Slot::Away);
        // Her dash home (D3a), if this step of her clock crossed it: an
        // edge, so a cold start or a restart after it never dashes.
        let dash = fed.is_some_and(|clock| self.dash_crossed(&clock, before, self.game_ms()));
        // A quarter-hour passed: a clock's dial or a window's sky shown
        // may have changed.
        let quarter = fed.is_some() && routine::quarter_crossed(before, self.game_ms());
        // Her rains, whatever her state: one ending this tick needs its
        // last frame too (the frame without it).
        let fading = !self.fades.is_empty();
        self.fades.retain(|fade| !fade.done(now));
        let mut changed = match &mut self.state {
            State::Absent => self.absent(now, school, dash),
            State::Arriving(_) => true,
            State::Away(empty) => {
                let ticked = quarter && tells_time(&empty.shown);
                if !school {
                    self.school_out(now);
                    true
                } else if dash && self.comes_in(now) {
                    tracing::info!("houseguest: dashing home from school");
                    self.state = State::Arriving(How::Dash);
                    true
                } else {
                    ticked
                }
            }
            State::Visiting(visit) => {
                let flapped = visit
                    .flap
                    .take_if(|&mut (_, since)| now >= since + FLAP_MS)
                    .is_some();
                // Her pity as it stands, for a new day's draw.
                visit.osaka.set_pity(pity_of(&self.ledger));
                let changed =
                    visit
                        .osaka
                        .tick(now, fed, &visit.terrain, &visit.chances, &mut self.rng);
                if let Some(budget) = visit.osaka.line_budget() {
                    self.lines_today = Some(budget);
                }
                if let Some((day, rares)) = visit.osaka.rares_today()
                    && self.rares_today.as_ref().is_none_or(|&(on, _)| on != day)
                {
                    self.rares_today = Some((day, rares.clone()));
                }
                if let Some(meal) = visit.osaka.meal_said() {
                    self.meal_today = Some(meal);
                }
                if let Some(night) = visit.osaka.night() {
                    self.night_today = Some(night);
                }
                // Hers the moment she does it: whatever ends the visit
                // before the next paint can't lose it.
                self.unsaved |= record(&mut self.ledger, &mut self.shop_now, visit);
                // Dashed home and still here as school ends (in on an
                // errand at 12:44, say), not on her way out again: she's
                // home, and it's a visit after all.
                if visit.kind == Kind::Dash && !school && visit.osaka.leaving().is_none() {
                    visit.kind = Kind::Normal;
                    self.ledger.visits += 1;
                    self.unsaved = true;
                    self.out = None;
                    tracing::info!(visit = self.ledger.visits, "houseguest: home early");
                }
                changed || flapped || quarter && tells_time(&visit.shown)
            }
            State::Leaving(leaving) => {
                if leaving.dissolve.done(now) {
                    tracing::trace!("houseguest gone");
                    self.state = State::Absent;
                }
                true
            }
        };
        if self.gone_out(now) {
            self.out_by_door(now);
            changed = true;
        }
        changed || fading || nudge
    }

    /// Absent at `now` (`school`: it's school time by her routine): she
    /// comes in as the idle gate opens, or, at school time, her home
    /// stands empty (she has one; with none, there's nothing to show),
    /// or she dashes home (`dash`: its moment just passed); as school
    /// ends, her coming home is decided. Returns whether the screen could
    /// change.
    fn absent(&mut self, now: u64, school: bool, dash: bool) -> bool {
        if school {
            self.out.get_or_insert_default();
            if dash && self.comes_in(now) {
                tracing::info!("houseguest: dashing home from school");
                self.state = State::Arriving(How::Dash);
                return true;
            }
        } else if self.out.is_some() {
            self.school_out(now);
            return !matches!(self.state, State::Absent);
        }
        if !self.gate_open(now) {
            return false;
        }
        if !school {
            tracing::trace!("houseguest arriving");
            self.state = State::Arriving(How::Idle);
            return true;
        }
        if !self.furnished() {
            return false;
        }
        tracing::info!("houseguest: her home, empty (she's at school)");
        self.state = State::Away(Box::new(Empty::new(&self.ledger, self.cat_now)));
        true
    }

    /// Whether it's school time at `now` by her routine as fed to her
    /// (unfed, never).
    fn at_school(&self, now: u64) -> bool {
        self.fed_day(now)
            .is_some_and(|day| day.slot == routine::Slot::Away)
    }

    /// Whether she has a home: furniture of her own.
    fn furnished(&self) -> bool {
        !self.ledger.home.props.is_empty()
    }

    /// School's out at `now` while she's out: she comes home out of her
    /// closed door (where it stands, if it does) if the client is idle
    /// (or she lives here, and visits are on). Its own question (A6),
    /// asked once, here: input or chat since doesn't call it off. Else
    /// she's absent, and comes in later on the idle gate.
    fn school_out(&mut self, now: u64) {
        let door = self.out.take().and_then(|out| out.door);
        if self.comes_in(now) {
            tracing::info!("houseguest: home from school");
            self.state = State::Arriving(How::Return(door));
        } else {
            tracing::debug!("houseguest: school's out, but the client is busy");
            self.state = State::Absent;
        }
    }

    /// Whether she comes in at `now` by her routine while she's out (home
    /// from school, or dashing home): the client idle, or she lives here,
    /// and visits on. Asked once, as it's due: input or chat since
    /// doesn't call it off (A6).
    fn comes_in(&self, now: u64) -> bool {
        self.open && (self.resident || self.gate_open(now))
    }

    /// Her clock, in game millis since its start, as of the last accrual
    /// (0 before she has met you: it stands still then). The one reading
    /// of her clock's units ([`Guest::game_clock`] reads it too).
    fn game_ms(&self) -> u64 {
        self.ledger
            .clock
            .saturating_mul(GAME_MINUTE_MS)
            .saturating_add(self.clock_rem)
    }

    /// Whether a dash home (D3a) falls in her clock's step from game
    /// millis `from` (not included) to `to`: on a game day of the step,
    /// at its minute ([`brain::dash`]), while that day is at school by
    /// her routine (`clock`). At most one a day.
    fn dash_crossed(&self, clock: &routine::Clock, from: u64, to: u64) -> bool {
        let days = routine::split(from).0..=routine::split(to).0;
        from < to
            && days.into_iter().any(|day| {
                brain::dash(self.ledger.master_seed, day).is_some_and(|minute| {
                    let at = routine::game_of(day, minute);
                    from < at && at <= to && clock.day_of(at).slot == routine::Slot::Away
                })
            })
    }

    /// When her next dash home (D3a) is due, in monotonic millis, while
    /// her routine is fed: today's, if it's still to come (a later day's
    /// comes after a boundary she wakes for anyway).
    fn next_dash(&self, now: u64) -> Option<u64> {
        let clock = self.routine_clock(now).filter(|_| self.feed_clock)?;
        let game = clock.game.at(now);
        let day = routine::split(game).0;
        let at = routine::game_of(day, brain::dash(self.ledger.master_seed, day)?);
        (at > game && clock.day_of(at).slot == routine::Slot::Away).then(|| clock.game.when(at))
    }

    /// Where her closed door stands while she's out, if it does.
    fn closed_door(&self) -> Option<DoorAt> {
        self.out.and_then(|out| out.door)
    }

    /// Whether she's visiting and gone out by her routine at `now`, her
    /// door shut behind her ([`Osaka::gone_out`]).
    fn gone_out(&self, now: u64) -> bool {
        matches!(&self.state, State::Visiting(visit) if visit.osaka.gone_out(now).is_some())
    }

    /// Her visit ends at `now` with her out through her door by her
    /// routine (A9): a new end, with no goodbye and no wave of her. With
    /// a home to show, it stands empty, her closed door where she went
    /// out (`State::Away`), and what she'd moved and made rains out.
    /// With none (no home; or out again from a dash home, the client in
    /// use), what the visit showed rains out as her empty home's does
    /// for a visitor's key: her door, her pieces, what she'd moved and
    /// made, nobody in it to wave (she's out of sight); and she's absent.
    /// Nothing of hers just vanishes. Either way she's out until school
    /// ends.
    fn out_by_door(&mut self, now: u64) {
        let State::Visiting(visit) = std::mem::replace(&mut self.state, State::Absent) else {
            return;
        };
        tracing::info!(kind = ?visit.kind, "houseguest: out, her door shut behind her");
        let door = DoorAt {
            x: visit.osaka.x,
            y: visit.osaka.y,
            facing: visit.osaka.facing,
        };
        // Where she went out (with no home to show, it's where she comes
        // back in).
        self.out = Some(Out { door: Some(door) });
        // Out again from a dash home, her home stands empty only for a
        // client still idle (or hers); else it shows when the gate next
        // opens.
        let stands = self.furnished()
            && match visit.kind {
                Kind::Normal => true,
                Kind::Dash => self.comes_in(now),
            };
        if !stands {
            tracing::info!("houseguest: what she showed rains out, her home not shown");
            self.state = State::Visiting(visit);
            self.leave(now);
            return;
        }
        // What she moved and made goes (the real text shows again), as
        // letters raining out where it stood.
        let moved: std::collections::HashSet<(u16, u16)> =
            visit.layer.cells().chain(visit.layer.holes()).collect();
        let made: Vec<Rect> = visit.made.iter().map(|m| m.piece.cover()).collect();
        let mine = |cell: &&Frozen| {
            moved.contains(&(cell.x, cell.y))
                || made.iter().any(|r| r.contains((cell.x, cell.y).into()))
        };
        let out: Vec<Frozen> = visit.painted.iter().filter(mine).cloned().collect();
        if !out.is_empty() {
            self.fades.push(Dissolve::new(
                now.saturating_sub(dissolve::RAIN_FROM_MS),
                out,
                door.x,
                self.truecolor,
                visit.size,
            ));
        }
        let mut empty = Empty::new(&self.ledger, self.cat_now);
        empty.size = visit.size;
        self.state = State::Away(Box::new(empty));
    }

    /// The next boundary of her routine after `now` (monotonic millis,
    /// rounded up), while it's fed to her: absent or out, she wakes for
    /// it (a census, or the tests' run, never skips one).
    fn next_boundary(&self, now: u64) -> Option<u64> {
        let clock = self.routine_clock(now).filter(|_| self.feed_clock)?;
        Some(clock.game.when(clock.next_boundary(clock.game.at(now))))
    }

    /// Her minute of the day at `now`, as her wall clock and window show
    /// it: `None` unless her routine is fed to her (and her clock runs),
    /// when they show a plain face and a day sky (A5).
    fn time_of_day(&self, now: u64) -> Option<u16> {
        let clock = self.game_clock(now).filter(|_| self.feed_clock)?;
        Some(routine::split(clock.at(now)).1)
    }

    /// The next quarter-hour of her clock after `now` (monotonic millis,
    /// rounded up), while `shown` has a piece that tells the time and her
    /// routine is fed: its dial or sky may change then. A home without
    /// one wakes for nothing more.
    fn next_quarter(&self, shown: &[Shown], now: u64) -> Option<u64> {
        if !tells_time(shown) {
            return None;
        }
        let clock = self.game_clock(now).filter(|_| self.feed_clock)?;
        Some(clock.when(routine::next_quarter(clock.at(now))))
    }

    /// How soon she next needs a tick; `None` when nothing is pending.
    pub fn next_tick(&self, now: u64) -> Option<Duration> {
        let due = match &self.state {
            // The idle gate (at school time, only for a home to show),
            // and her routine's next boundary.
            State::Absent => {
                let school = self
                    .fed_day(now)
                    .is_some_and(|day| day.slot == routine::Slot::Away);
                self.gate_from()
                    .filter(|_| !school || self.furnished())
                    .into_iter()
                    .chain(self.next_boundary(now))
                    .chain(self.next_dash(now))
                    .min()
            }
            State::Arriving(_) => Some(now),
            State::Away(empty) => self
                .next_boundary(now)
                .into_iter()
                .chain(self.next_dash(now))
                .chain(self.next_quarter(&empty.shown, now))
                .min(),
            State::Visiting(visit) => Some(
                visit
                    .flap
                    .map(|(_, since)| since + FLAP_MS)
                    .into_iter()
                    .chain(self.next_quarter(&visit.shown, now))
                    .fold(visit.osaka.wakes_at(), u64::min),
            ),
            State::Leaving(leaving) => Some(leaving.dissolve.next_frame(now)),
        };
        // Her rains' frames, whatever her state.
        let due = due
            .into_iter()
            .chain(self.fades.iter().map(|fade| fade.next_frame(now)))
            .chain(self.nudge.next_at(now))
            .min()?;
        Some(Duration::from_millis(due.saturating_sub(now)))
    }

    /// Paint her over the finished frame. `view` must describe the frame
    /// just drawn (pane rectangles are measured during the draw).
    pub fn paint(&mut self, buf: &mut Buffer, view: &IdleView, now: u64) {
        // A still of the film delivered since the last paint cuts in.
        if let Some(graphics) = &mut self.graphics {
            graphics.latch_film();
        }
        // Her rains, last, over the real frame in a focused pane (where
        // nothing else of hers goes), whatever her state now; once a
        // frame (a second pass would take the first's glyphs for the UI
        // changing, and settle them), so only when the arm that painted
        // her state didn't already (a goodbye paints them under its
        // image).
        if self.paint_state(buf, view, now) == Rains::ToPaint {
            let size = (buf.area.width, buf.area.height);
            paint_fades(&mut self.fades, buf, size, now);
        }
    }

    /// Paint her state over the finished frame (see [`Guest::paint`]).
    /// Returns whether that painted her rains too: each arm says, so
    /// they're painted once whatever state she ends the frame in.
    fn paint_state(&mut self, buf: &mut Buffer, view: &IdleView, now: u64) -> Rains {
        self.observe(view, now);
        let as_drawn = view;
        let view = &whole_glyphs(buf, self.gate(view, now));
        let size = (buf.area.width, buf.area.height);
        self.errand_progress(view, now);
        self.nudge_due(buf, view, now);
        if let State::Arriving(how) = self.state {
            self.state = State::Absent;
            if size.0 >= MIN_WIDTH && size.1 >= MIN_HEIGHT {
                let terrain = Terrain::read(buf, &view.protected, self.graphics.is_some());
                let kind = self.kind_of(how == How::Dash);
                // Each visit draws from its own seed.
                self.rng = Rng(self.seed_of(kind));
                // Out of her door where it stood (or the nearest spot it
                // fits, or one near the middle with no home), never
                // dropping in from the sky.
                let through = |door: Option<DoorAt>| {
                    let near = door.map_or(middle(size), |d| (d.x, d.y));
                    let facing = door.map_or(sprite::Facing::Right, |d| d.facing);
                    door_spot(&terrain, near).map(|spot| (spot, facing))
                };
                let osaka = match how {
                    How::Idle => Osaka::arrive(now, &terrain, i32::from(size.0), &mut self.rng),
                    How::Return(door) => through(door).map(|(spot, facing)| {
                        Osaka::back_through_door(
                            spot,
                            facing,
                            osaka::Routine::School,
                            now,
                            &mut self.rng,
                        )
                    }),
                    How::Dash => through(self.closed_door()).map(|(spot, facing)| {
                        let met = kind == Kind::Dash;
                        Osaka::dash_in(spot, facing, met, now, &mut self.rng)
                    }),
                };
                if let Some(osaka) = osaka {
                    self.begin_visit(osaka, terrain, size, kind, now);
                }
            }
            if !self.present() {
                // Nowhere to stand: try again after another idle delay.
                self.quiet_since = now;
            }
        }
        // Out through her door since the last tick: her home stands
        // empty from this frame on (no frame of it without her door).
        if self.gone_out(now) {
            self.out_by_door(now);
        }
        // Her time of day, as her clock and window show it.
        let time = self.time_of_day(now);
        // The accordion's shake is hers, painted like the rest of her:
        // after everything that reads the real frame, and under her.
        let nudge = &self.nudge;
        match &mut self.state {
            State::Absent | State::Arriving(_) => {
                nudge.paint(buf, now);
                Rains::ToPaint
            }
            State::Away(empty) => {
                if size.0 < MIN_WIDTH || size.1 < MIN_HEIGHT {
                    // As a visit leaves for no room: shown again only
                    // after another idle delay (the gate, still open,
                    // would show it at once, to go again at this paint).
                    // She's still out, her door's spot kept.
                    tracing::info!("houseguest: her empty home goes (no room)");
                    self.vanish();
                    self.quiet_since = now;
                    return Rains::ToPaint;
                }
                // As the frame has it, before she keeps out of a focused
                // pane: her closed door moves only for what would keep
                // her out of its spot for good (a resize), never for a
                // pane kept clear a while.
                let unkept = whole_glyphs(buf, as_drawn.clone()).protected;
                let door = &mut self.out.get_or_insert_default().door;
                let changed = paint_empty(
                    empty,
                    &mut self.fades,
                    door,
                    &mut self.ledger,
                    self.graphics.as_mut(),
                    buf,
                    view,
                    &unkept,
                    nudge,
                    time,
                    now,
                    self.truecolor,
                );
                self.unsaved |= changed;
                Rains::ToPaint
            }
            State::Leaving(leaving) => {
                if leaving.dissolve.size() != size {
                    // The geometry she froze against is gone.
                    self.vanish();
                    return Rains::ToPaint;
                }
                // Everything that compares against the real frame reads it
                // before any of her pixels or glyphs go on: her own image
                // must never look like "the UI changed here".
                let t = now.saturating_sub(leaving.dissolve.started());
                let untouched = leaving.dissolve.unchanged(buf);
                let terrain = Terrain::read(buf, &view.protected, true);
                nudge.paint(buf, now);
                leaving.dissolve.paint(buf, now);
                // Her rains on to their ends, before the goodbye's own
                // (they all began before it), under its image (over it,
                // the rain's cells there would differ from the real frame
                // and settle for good).
                paint_fades(&mut self.fades, buf, size, now);
                if let Some(graphics) = &mut self.graphics
                    && t < dissolve::RAIN_FROM_MS
                    && untouched
                {
                    for prop in &leaving.props {
                        paint_prop_art(buf, graphics, prop, &leaving.looks);
                    }
                }
                if let (Some(image), Some(graphics)) = (&leaving.image, &mut self.graphics)
                    && t < dissolve::RAIN_FROM_MS
                    && untouched
                {
                    // Startled, then a wave; then she bursts into letters.
                    // She jumps up out of anything she was in; what she
                    // overlapped is still drawn in her image, and her
                    // door stands as it stood. Out of sight behind it,
                    // she isn't there to wave.
                    let her = image.figure.her().map(|at| {
                        if t < dissolve::SMILE_FROM_MS {
                            at.pose(sprite::Pose::Stand, leaving.startled)
                        } else {
                            at.wave((t / dissolve::WAVE_MS).is_multiple_of(2))
                        }
                    });
                    let door = image.figure.door().map(Door::layer);
                    // The pieces in it as they looked (A22): the lamp
                    // stays off, the TV on, to the rain.
                    let looks = &leaving.looks;
                    let layers: Vec<graphics::Layer> = image
                        .with
                        .iter()
                        .map(|p| {
                            let look = piece_look(
                                p,
                                art::Layer::Whole,
                                looks.tv,
                                false,
                                looks.state(p.item),
                            );
                            prop_layer(p, look)
                        })
                        .chain(door)
                        .chain(her)
                        .collect();
                    graphics.paint_layers(buf, &layers, &|x, y| terrain.open(x, y));
                }
                Rains::Painted
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
                        self.fades.push(Dissolve::new(
                            now.saturating_sub(dissolve::RAIN_FROM_MS),
                            out,
                            visit.osaka.x,
                            self.truecolor,
                            size,
                        ));
                    }
                }
                // Read before paint: the layer validates against the real
                // frame, then its cells join the protected set so she
                // never stands over moved text or the holes it left.
                visit.layer.validate(buf, &view.protected);
                // Her furniture stands on blank cells, clear of protected
                // ones, moved text, and her; what doesn't fit is in the
                // closet this frame. Placed, it's solid to text; she walks
                // in front of it.
                let before = self.ledger.clone();
                self.unsaved |= record(&mut self.ledger, &mut self.shop_now, visit);
                let shown = furnish(
                    &mut self.ledger,
                    &mut self.gift,
                    &mut self.arranging,
                    &mut self.note,
                    buf,
                    view,
                    visit,
                    time.is_some(),
                    now,
                    &mut self.rng,
                );
                visit.shown = shown;
                if self.ledger != before {
                    self.unsaved = true;
                }
                // The place of the piece in her pocket is kept for it.
                let mut kept = view.protected.clone();
                kept.extend(visit.ghost);
                tend_made(visit, buf, &kept, size, now);
                visit.shown.extend(visit.made.iter().map(|made| made.piece));
                let gone = |piece: room::PieceRef| !visit.shown.iter().any(|s| s.piece() == piece);
                let using = visit.osaka.use_span().map(|(seat, ..)| seat.piece);
                let lifting = visit.osaka.lifting().map(room::PieceRef::Real);
                if using.or(lifting).is_some_and(gone) {
                    visit.osaka.lost_seat(now);
                }
                let mut base = view.protected.clone();
                base.extend(visit.shown.iter().map(Shown::cover));
                base.extend(visit.ghost);
                let mut gripped = true;
                for op in visit.osaka.take_ops() {
                    if !scenes::apply(&op, &mut visit.layer, buf, &base) {
                        tracing::trace!(?op, "houseguest: the frame refused a layer change");
                        gripped &= !op.grips();
                        visit.osaka.refused(now, op);
                    } else if let scenes::LayerOp::Make {
                        row,
                        cells,
                        piece,
                        purpose,
                    } = &op
                    {
                        if let Some(scrap) = piece.scrap {
                            visit.next_made = room::MadeId(scrap.id.0 + 1);
                        }
                        visit.made.push(made_of(buf, *row, cells, piece, *purpose));
                    }
                }
                // The text she's at changed or scrolled under her (the
                // line she's taking hold of, or what of it she has out,
                // gone back to its line): she's lost it, this frame.
                let slipped = visit
                    .osaka
                    .grip()
                    .is_some_and(|grip| !grip.holds(buf, &visit.layer));
                if !gripped || slipped {
                    visit.osaka.lost_grip(now);
                }
                let_go(visit);
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
                    self.vanish();
                    self.quiet_since = now;
                    return Rains::ToPaint;
                }
                // Moved on just now (text came up where she stays, her
                // pane was focused): what she held goes back before the
                // frame shows it.
                let_go(visit);
                // A line she moved isn't hers to pull again (it's
                // protected), but its letters are hers to swap where
                // they now sit: swaps read the frame with her layer on.
                // Text reads keep every piece solid.
                protected.extend(visit.shown.iter().map(Shown::cover));
                let pulls = scenes::pulls(buf, &visit.terrain, &protected);
                // The stage places her after her text is drawn: the
                // frame as it was, to draw again if that let go of text.
                let unpainted = self.cue.is_some().then(|| buf.clone());
                let mut layer = visit.layer.paint(buf);
                let mut holes = base;
                holes.extend(runs(visit.layer.holes()));
                let swaps = scenes::swaps(buf, &visit.terrain, &holes, &visit.layer);
                let mut solid = protected.clone();
                solid.extend(visit.ghost);
                let builds = builds(buf, visit, &pulls, &solid);
                let borrows = borrows(visit, &pulls);
                let (lift_at, judged) = arranging(visit);
                let beauty_here =
                    beauty_at(&visit.shown, &view.nooks, (visit.osaka.x, visit.osaka.y));
                let clock = clock_on(&visit.shown, &view.nooks);
                if let Some(scene) = self.cue.take() {
                    // A scene cued for this visit's start is played, not
                    // slept through (an arrival is tucked in, as any).
                    if scene != stage::Scene::Arrive {
                        visit.tuck = false;
                    }
                    let offered = osaka::Chances {
                        pulls: pulls.clone(),
                        swaps: swaps.clone(),
                        loose: Vec::new(),
                        seats: seats(
                            &visit.shown,
                            &visit.terrain,
                            self.cat_now || cat_home(&self.ledger, visit.kind),
                        ),
                        real: real_kinds(&visit.shown),
                        builds: builds.clone(),
                        borrows: borrows.clone(),
                        mine: visit.made.iter().filter_map(Made::mine).collect(),
                        advert: advert(&self.ledger, self.shop_now, visit.osaka.needs()),
                        furnished: !self.ledger.home.props.is_empty(),
                        chat,
                        broken: visit.broken.clone(),
                        repairs: visit.repairs.clone(),
                        lift_at: lift_at.clone(),
                        judged,
                        beauty_here,
                        clock,
                    };
                    let note =
                        stage::direct(scene, buf, &protected, visit, &offered, now, &mut self.rng);
                    tracing::info!(?note, "houseguest cued");
                    self.note = Some(note);
                    // Placed, she let go of what she held: back before
                    // the frame shows it, as for anything that moves her
                    // on (what she could do with the text refreshes on
                    // the next frame).
                    let held = visit.reel.clone();
                    let_go(visit);
                    if visit.reel != held
                        && let Some(unpainted) = unpainted
                    {
                        *buf = unpainted;
                        layer = visit.layer.paint(buf);
                    }
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
                        self.cat_now || cat_home(&self.ledger, visit.kind),
                    ),
                    real: real_kinds(&visit.shown),
                    builds,
                    borrows,
                    mine: visit.made.iter().filter_map(Made::mine).collect(),
                    advert: advert(&self.ledger, self.shop_now, visit.osaka.needs()),
                    furnished: !self.ledger.home.props.is_empty(),
                    chat,
                    broken: visit.broken.clone(),
                    repairs: visit.repairs.clone(),
                    lift_at,
                    judged,
                    beauty_here,
                    clock,
                };
                // A visit beginning at night: in her bed (else on her
                // sofa) from the first frame, if either is shown; else she
                // arrives as ever, and her routine sends her to bed. (It
                // needs only the piece: an arrival with nowhere to come in
                // is one with no floor at all, where no piece could
                // stand.)
                if std::mem::take(&mut visit.tuck) {
                    visit.osaka.tuck_in(&visit.chances, &visit.terrain, now);
                }
                // In line art, pieces her box overlaps go in its image
                // (two images would cut each other out), if anything of
                // hers stands in it: out of sight, her door shut, every
                // piece is drawn on its own.
                let figure = Figure::of(&visit.osaka, now).filter(|_| self.graphics.is_some());
                let covers: Vec<Rect> = visit.shown.iter().map(Shown::cover).collect();
                let drawn = match figure {
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
                let world = World {
                    cat: self.cat_now || cat_home(&self.ledger, visit.kind),
                    time,
                };
                let prop = visit.osaka.prop(now);
                let dark = visit.osaka.dark(now);
                let looks = Looks {
                    tv: prop.and_then(script::Prop::channel),
                    states: visit
                        .shown
                        .iter()
                        .map(|p| (p.item, piece_state(p, prop, dark, world)))
                        .collect(),
                };
                nudge.paint(buf, now);
                if let Some((flap, since)) = visit.flap {
                    layer.extend(draw_flap(
                        buf,
                        flap,
                        now.saturating_sub(since),
                        &view.protected,
                    ));
                }
                layer.extend(draw_props(
                    buf,
                    self.graphics.as_mut(),
                    &apart,
                    &looks,
                    self.truecolor,
                ));
                let drawn = match &mut self.graphics {
                    Some(graphics) => {
                        let (painted, image, drawn) = draw_art(
                            buf,
                            graphics,
                            &visit.osaka,
                            &visit.terrain,
                            &visit.shown,
                            figure,
                            with,
                            &looks,
                            now,
                            self.truecolor,
                        );
                        layer.extend(painted);
                        visit.image = image;
                        drawn
                    }
                    None => {
                        let (painted, drawn) = draw(
                            buf,
                            &visit.osaka,
                            &visit.terrain,
                            &visit.shown,
                            now,
                            self.truecolor,
                            true,
                        );
                        layer.extend(painted);
                        visit.image = None;
                        drawn
                    }
                };
                visit.apart = apart;
                visit.looks = looks;
                visit.painted = layer;
                // Her calendar's entry, delivered as it shows (its line the
                // bubble drawn): hers the moment it does, whatever ends
                // the visit before the next tick.
                if visit.osaka.shown(now, drawn).is_some() {
                    self.unsaved |= record(&mut self.ledger, &mut self.shop_now, visit);
                }
                Rains::ToPaint
            }
        }
    }

    /// What a visit beginning now is: a dash home (`dash`, A16) once
    /// she has met you; a first meeting is always counted (her clock
    /// starts with it).
    fn kind_of(&self, dash: bool) -> Kind {
        if dash && self.ledger.visits > 0 {
            Kind::Dash
        } else {
            Kind::Normal
        }
    }

    /// The seed of her body's stream for a visit of `kind` beginning
    /// now: the coming visit's, salted apart for a dash home (so the
    /// visit after it draws as it would have).
    fn seed_of(&self, kind: Kind) -> u64 {
        let seed = self.ledger.visit_seed(self.ledger.visits);
        match kind {
            Kind::Normal => seed,
            Kind::Dash => seed ^ brain::DASH_SALT,
        }
    }

    /// The window of her day a visit beginning at `now` (`day`, her
    /// routine fed) draws what's rare for: the part of her day it begins
    /// in, and the next.
    fn rare_window(&self, now: u64, day: routine::DayTime) -> routine::SlotSet {
        let next = self.routine_clock(now).map_or(day.slot, |clock| {
            clock.day_of(clock.next_boundary(clock.game.at(now))).slot
        });
        rarity::visit_window(day.slot, next)
    }

    /// A new visit of `kind`, with `osaka` just arrived.
    fn begin_visit(
        &mut self,
        mut osaka: Osaka,
        terrain: Terrain,
        size: (u16, u16),
        kind: Kind,
        now: u64,
    ) {
        match kind {
            Kind::Normal => {
                self.ledger.visits += 1;
                self.unsaved = true;
                // Here, she isn't out (whatever brought her: her coming
                // home, an errand, the stage), and her closed door is
                // hers again.
                self.out = None;
                tracing::info!(visit = self.ledger.visits, "houseguest arrived");
            }
            // Still out at school: her door's spot is kept, for her empty
            // home if this is cut short.
            Kind::Dash => tracing::info!(visit = self.ledger.visits, "houseguest dashed home"),
        }
        let day = self.fed_day(now);
        // Her mood (neither random stream): the game day's, her routine
        // fed (a second visit the same day comes in the same mood, saying
        // the same hello), else this visit's.
        let seed = match day {
            Some(day) => brain::day_seed(self.ledger.master_seed, day.day),
            None => self.ledger.visit_seed(self.ledger.visits.saturating_sub(1)),
        };
        osaka.set_mood(brain::Mood::of(seed));
        osaka.key_days(self.ledger.master_seed, day.map(|day| day.day));
        // What's rare and open (neither random stream): the game day's,
        // her routine fed, drawn for the day's first visit and carried
        // on to its others; else this visit's (her endless afternoon).
        let seen = self.ledger.seen_scripts();
        let rares = match day {
            Some(day) => match &self.rares_today {
                Some((on, rares)) if *on == day.day => rares.clone(),
                _ => rarity::Rares::draw(
                    brain::day_seed(self.ledger.master_seed, day.day),
                    &seen,
                    pity_of(&self.ledger),
                    self.rare_window(now, day),
                ),
            },
            None => rarity::Rares::draw(
                self.ledger.visit_seed(self.ledger.visits.saturating_sub(1)),
                &seen,
                pity_of(&self.ledger),
                rarity::UNFED_WINDOW,
            ),
        };
        if let Some(day) = day {
            self.rares_today = Some((day.day, rares.clone()));
        }
        osaka.set_rares(rares, day.map(|day| day.day), seen);
        osaka.set_pity(pity_of(&self.ledger));
        // Her calendar: owed once a day, so the date she last delivered
        // it on.
        osaka.set_calendar(self.ledger.calendar_on);
        // Her meal's line is the slot's, not the visit's.
        if let Some(day) = day {
            osaka.carry_meal(self.meal_today.filter(|&(on, _)| on == day.day));
        }
        // Her night is the night's: what an earlier visit had of it
        // counts (its morning keys it).
        if day.is_some() {
            osaka.carry_night(self.night_today);
        }
        // Her line budget is the day's: what she said earlier today
        // counts.
        if let (Some(day), Some((on, spent))) = (day, self.lines_today)
            && on == day.day
        {
            osaka.carry_lines(spent);
        }
        // Her needs, from the time of her day (unfed: as ever).
        if let Some(day) = day {
            osaka.set_clock(day);
        }
        // Her routine reaches her from her first moment (her first paint
        // may tuck her in), not only from her first tick.
        osaka.read_clock(self.routine_clock(now).filter(|_| self.feed_clock));
        // Come in the night for the accordion, she was asleep: groggy.
        osaka.groggy_if_night(now);
        // At night she's tucked in at the first paint (not come on an
        // errand: that's for the accordion; nor dashed home: that's for
        // what she forgot).
        let tuck = day.is_some_and(|day| day.slot == routine::Slot::Asleep)
            && !osaka.on_errand()
            && !osaka.dashing();
        self.state = State::Visiting(Box::new(Visit {
            osaka,
            kind,
            terrain,
            painted: Vec::new(),
            image: None,
            layer: layer::TextLayer::default(),
            chances: osaka::Chances::default(),
            shown: Vec::new(),
            apart: Vec::new(),
            flap: None,
            made: Vec::new(),
            next_made: room::MadeId(0),
            reel: None,
            broken: Vec::new(),
            repairs: Vec::new(),
            mending: None,
            set_down: None,
            judging: None,
            ghost: None,
            size,
            tuck,
            looks: Looks::default(),
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
        // At school time she's off again by her door (her routine's away
        // reflex), not in a dissolve (Q2).
        if self.at_school(now) {
            tracing::debug!("houseguest: errand done, back to school");
            return;
        }
        let quiet = view
            .delay
            .is_some_and(|delay| now >= self.quiet_until(delay));
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
            // Out at school, she comes for it too, out of a door: a dash
            // home (Q2), and after the poke her routine sends her out
            // again by her door.
            State::Absent | State::Arriving(_) | State::Away(_) => {
                // Her closed door, as it stood in her empty home, rains
                // out as she comes (her pieces stand on in the visit).
                let door = self.door_rain(now);
                let kind = self.kind_of(self.at_school(now));
                self.rng = Rng(self.seed_of(kind));
                let osaka = Osaka::arrive_for_errand(spot, now, &mut self.rng);
                let size = (buf.area.width, buf.area.height);
                self.begin_visit(osaka, terrain, size, kind, now);
                self.fades.extend(door);
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

    /// Her closed door as her empty home last painted it (`State::Away`),
    /// raining out from `now`: what a visit breaking in on it (an errand)
    /// keeps of it, so it never just vanishes. The pieces it took in
    /// aren't in it: the visit paints them on.
    fn door_rain(&self, now: u64) -> Option<Dissolve> {
        let (State::Away(empty), Some(door)) = (&self.state, self.closed_door()) else {
            return None;
        };
        let cells: std::collections::HashSet<(i32, i32)> =
            Door::closed(door.x, door.y, door.facing)
                .cells()
                .map(|(x, y, _)| (x, y))
                .collect();
        let out: Vec<Frozen> = empty
            .painted
            .iter()
            .filter(|cell| cells.contains(&(i32::from(cell.x), i32::from(cell.y))))
            .cloned()
            .collect();
        (!out.is_empty()).then(|| {
            Dissolve::new(
                now.saturating_sub(dissolve::RAIN_FROM_MS),
                out,
                door.x,
                self.truecolor,
                empty.size,
            )
        })
    }

    /// `view` as she keeps to it now: while the client is in use —
    /// playing, a held selection, or local input within the idle delay —
    /// a resident keeps out of the focused pane (it's protected); left
    /// alone, the whole screen is hers again.
    fn gate(&self, view: &IdleView, now: u64) -> IdleView {
        let mut view = view.clone();
        let quiet = view
            .delay
            .is_none_or(|delay| now >= self.quiet_until(delay));
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
                // Switched off: no goodbye, and nothing of hers rains on
                // (a goodbye under way rains to its end).
                State::Absent | State::Arriving(_) | State::Visiting(_) | State::Away(_)
                    if view.delay.is_none() =>
                {
                    self.vanish();
                }
                State::Arriving(_) => self.state = State::Absent,
                // A visitor busy (his video, a selection): on an errand,
                // she stays till it's done; on a dash home from school
                // (an errand's too, Q2), till she's out again by her door
                // (it's brief, and on her routine's way). Never under an
                // overlay: nothing of hers goes on over a modal (the
                // accordion shakes by itself if she hadn't poked it).
                State::Visiting(ref visit)
                    if view.busy != Some(Busy::Overlay)
                        && (self.errand.is_some() || visit.kind == Kind::Dash) => {}
                // Covered over (an overlay), or a visitor busy: her home
                // rains out, her too if she's here.
                State::Visiting(_) | State::Away(_) => self.leave(now),
                State::Absent | State::Leaving(_) => {}
            }
        }
        let before = self.chat_mark.replace(view.chat_mark);
        let arrived = before.is_some_and(|mark| mark != view.chat_mark);
        if let Some(before) = before.filter(|_| arrived) {
            match &mut self.state {
                State::Visiting(visit) => {
                    let asks = view.chat_mark.asks_since(&before);
                    tracing::trace!(asks, "houseguest looks at chat");
                    let chat = view.chat;
                    visit.osaka.look(
                        now,
                        i32::from(chat.x) + i32::from(chat.width) / 2,
                        asks,
                        &visit.terrain,
                    );
                }
                // Not idle: an arrival on the idle gate waits for the next
                // quiet. Her coming home doesn't (A6); her empty home
                // doesn't mind.
                // Out at school, no home shown (none, or not yet): as in
                // Away, it changes nothing, so her coming home asks the
                // same of a chat line with a home or without (A6).
                State::Absent if self.out.is_some() => {}
                State::Absent | State::Arriving(How::Idle) => {
                    self.quiet_since = now;
                    self.state = State::Absent;
                }
                State::Arriving(How::Return(_) | How::Dash) => self.quiet_since = now,
                State::Away(_) | State::Leaving(_) => {}
            }
        }
    }
}

/// Her pity counters as `ledger` has them (phase 5b D6).
fn pity_of(ledger: &Ledger) -> rarity::Pity {
    rarity::Pity::of(ledger.idle_min, ledger.rare_at, ledger.legend_at)
}

/// Record what she did to her home since last asked: the ledger's part
/// (an order, an unpacking) and the visit's (a makeshift piece crumpled
/// into shape, or used). Returns whether the ledger changed.
fn record(ledger: &mut Ledger, shop_now: &mut bool, visit: &mut Visit) -> bool {
    let mut changed = false;
    for event in visit.osaka.take_events() {
        match event {
            osaka::HomeEvent::Bought(item) => {
                ledger.ordered = Some(item);
                ledger.bought_on = ledger.visits;
                *shop_now = false;
                changed = true;
            }
            osaka::HomeEvent::Unpacked(item) => {
                changed |= ledger.home.unbox(item);
            }
            osaka::HomeEvent::Crumpled(id) => {
                for made in &mut visit.made {
                    if let Some(scrap) = &mut made.piece.scrap
                        && scrap.id == id
                    {
                        scrap.stage = scrap::STAGES;
                    }
                }
            }
            osaka::HomeEvent::Used(id) => {
                for made in &mut visit.made {
                    made.used |= made.piece.piece() == room::PieceRef::Made(id);
                }
            }
            // Hers once the frame takes it: at the paint (it must fit
            // where it goes then).
            osaka::HomeEvent::SetDown { piece, to } => visit.set_down = Some((piece, to)),
            // Owed once a day: not again this date.
            osaka::HomeEvent::Calendar(date) => {
                if ledger.calendar_on != Some(date) {
                    tracing::debug!(%date, "houseguest: her calendar's date recorded");
                    ledger.calendar_on = Some(date);
                    changed = true;
                }
            }
            // Seen from now on; something new, so her pity for its tier
            // starts again (not for one she'd seen already).
            osaka::HomeEvent::Seen(id) => {
                if let Some(key) = rarity::key(id)
                    && ledger.mark_seen(key, id.rarity())
                {
                    tracing::debug!(key, "houseguest: something rare recorded as seen");
                    changed = true;
                }
            }
        }
    }
    changed
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

/// Her door's glyphs' ink, and the letters they rain as.
const DOOR_INK: Ink = Ink::new(Color::LightMagenta, Modifier::empty());

/// Paint her (and any bubble) into `buf`, returning what was painted
/// over what, and the bubble drawn, if one was (it shows only where
/// there's room for it).
#[allow(clippy::too_many_arguments)]
fn draw(
    buf: &mut Buffer,
    osaka: &Osaka,
    terrain: &Terrain,
    shown: &[Shown],
    now: u64,
    truecolor: bool,
    with_sprite: bool,
) -> (Vec<Frozen>, Option<Bubble>) {
    let (sprite, bubble) = osaka.picture(now);
    let hidden = osaka.hidden(now);
    let door = osaka
        .door(now)
        .filter(|_| with_sprite)
        .map(|frame| sprite::door_cells(frame as usize, osaka.facing))
        .unwrap_or_default();
    let mut wanted: Vec<(i32, i32, char, Ink, Option<usize>)> = door
        .iter()
        .map(|cell| {
            (
                osaka.x + cell.dx,
                osaka.y + cell.dy,
                cell.glyph,
                DOOR_INK,
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
    let mut drawn = None;
    if let Some(bubble) = bubble.filter(|_| !hidden) {
        let text = bubble.text();
        let len = text.chars().count() as i32;
        let (pose, _, _) = osaka.appearance(now);
        if let Some((start, row)) = bubble_spot(buf, terrain, shown, osaka, pose, len) {
            drawn = Some(bubble);
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
    (painted, drawn)
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

/// Where her furniture stands this frame: the piece she set down taken
/// where it goes (if it fits there), then any delivery, then the stage's
/// gift; the rules of her home judged, how she'd put one right worked
/// out, and the move she's making judged on what's final. The piece in
/// her pocket shows nowhere (its place is kept). Nothing covers
/// protected cells or moved text.
#[allow(clippy::too_many_arguments)]
fn furnish(
    ledger: &mut Ledger,
    gift: &mut Option<Furniture>,
    arranging: &mut bool,
    note: &mut Option<Result<String, String>>,
    buf: &Buffer,
    view: &IdleView,
    visit: &mut Visit,
    fed: bool,
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
    let mut shown = home.project(buf, &view.nooks, &blocked);
    // The stage: a sofa turned away from her TV.
    let cued = std::mem::take(arranging) && stage_arrange(home, buf, view, &blocked);
    if cued {
        shown = home.project(buf, &view.nooks, &blocked);
    }
    let made: Vec<Rect> = visit.made.iter().map(|m| m.piece.cover()).collect();
    // The piece she set down: where it goes if it fits there and puts
    // its rule right still (judged with the move made: its own place is
    // free), else she lets it go.
    if let Some((piece, to)) = visit.set_down.take() {
        let frame = rules::Frame {
            buf,
            nooks: &view.nooks,
            blocked: &blocked,
            shown: &shown,
            made: &made,
        };
        let episode = visit
            .osaka
            .episode()
            .filter(|e| e.set_down && (e.repair.piece, e.repair.to) == (piece, to))
            .copied()
            .filter(|_| visit.osaka.carrying() == Some(piece));
        match episode.map(|ep| judge_move(home, &frame, &ep)) {
            Some(Some(_)) => {
                let strips = room::strips(&view.nooks);
                if let Some(&(_, e)) = strips.iter().find(|(s, _)| *s == to.strip)
                    && let Some(prop) = home.props.iter_mut().find(|p| p.item == piece)
                {
                    let cols = piece.spec().footprint.0;
                    prop.strip = to.strip;
                    prop.anchor = Some(to.anchor);
                    prop.at = e.pin(e.left(to.anchor, cols), cols).1;
                    prop.facing = to.facing;
                    prop.settled = true;
                }
                visit.osaka.set_down_done(piece, now);
                shown = home.project(buf, &view.nooks, &blocked);
            }
            Some(None) => {
                let back = home
                    .layout(&view.nooks)
                    .into_iter()
                    .find(|s| s.item == piece)
                    .map_or((visit.osaka.x, visit.osaka.y), |s| middle_of(&s));
                visit.osaka.set_down_refused(back, now);
            }
            // Not hers to set down any more (the stage put her somewhere).
            None => {}
        }
    }
    // A delivery: what she ordered on an earlier visit arrives boxed,
    // wherever it will fit. Her first TV is ordered for her, to arrive
    // on her second visit.
    if ledger.ordered.is_none() && !home.owns(Furniture::Tv) && ledger.visits >= FIRST_TV_VISIT {
        ledger.ordered = Some(Furniture::Tv);
        ledger.bought_on = ledger.visits - 1;
    }
    // Not while she's asleep for the night, or about to be tucked in:
    // she'd say so. It waits for her to wake, and to say good morning.
    // Nor on a dash home from school (A16): it waits for her return.
    let awake = !visit.tuck && visit.kind != Kind::Dash && visit.osaka.awake(now);
    // A parcel comes in only where she can unpack it (her seats' test).
    let terrain = &visit.terrain;
    let stands = |x: i32, y: i32| seat_spot(terrain, x, y);
    if let Some(item) = ledger.ordered
        && ledger.bought_on < ledger.visits
        && awake
        && let Some((prop, flap)) = home.doorstep(buf, &view.nooks, &shown, &blocked, &stands, item)
        && home.add(prop)
    {
        tracing::info!(?item, strip = ?prop.strip, "houseguest: a parcel arrived");
        visit.flap = Some((flap, now));
        ledger.ordered = None;
        visit.osaka.say(PARCEL, now);
        shown = home.project(buf, &view.nooks, &blocked);
    }
    // Her wall clock (phase 5b D7, Q3): a gift, once ever, on her
    // doorstep, the first time she's up and about with her clock fed to
    // her and her TV out of its box (so it's there for her first bedtime
    // and her first morning off to school). Never in the CATALOGUE, so
    // her shopping is untouched; never with another parcel at the door
    // (a flap still open), nor while she's out of sight, asleep, or on a
    // dash home (`awake`), nor before her hello (and her calendar's
    // entry) is said.
    if fed
        && !ledger.clock_sent
        && awake
        && visit
            .osaka
            .free_for_a_gift(now, &visit.chances, &visit.terrain)
        && visit.flap.is_none()
        && !home.owns(Furniture::Clock)
        && home
            .props
            .iter()
            .any(|p| p.item == Furniture::Tv && !p.boxed)
        && let Some((prop, flap)) = home.doorstep(
            buf,
            &view.nooks,
            &shown,
            &blocked,
            &stands,
            Furniture::Clock,
        )
        && home.add(prop)
    {
        tracing::info!(strip = ?prop.strip, "houseguest: her wall clock arrived");
        visit.flap = Some((flap, now));
        ledger.clock_sent = true;
        visit.osaka.say(PARCEL, now);
        shown = home.project(buf, &view.nooks, &blocked);
    }
    if let Some(item) = gift.take() {
        shown = place_gift(home, item, note, buf, view, &shown, &blocked, rng);
    }
    // The rules of her home, judged where her pieces are laid out (text
    // closeting one doesn't count), once the frame's home is final.
    let laid = home.layout(&view.nooks);
    let broken = rules::broken(&laid, &room::strips(&view.nooks), home);
    if broken != visit.broken {
        tracing::trace!(
            broken = ?broken.iter().map(rules::Broken::label).collect::<Vec<_>>(),
            "houseguest: the rules of her home"
        );
        visit.broken = broken;
    }
    // The stage: she has felt the sofa's wrong way round, and works out
    // at once how to put it right, whatever her mood.
    let faces = rules::Grievance {
        row: rules::FACES_ROW,
        piece: Furniture::Sofa,
    };
    let force = (cued && visit.broken.iter().any(|b| b.key == faces)).then_some(faces);
    if let Some(key) = force {
        visit.osaka.feel(key, (Furniture::Sofa, room::Use::Lounge));
    }
    mend(home, buf, view, visit, &shown, &blocked, force, now);
    let frame = rules::Frame {
        buf,
        nooks: &view.nooks,
        blocked: &blocked,
        shown: &shown,
        made: &made,
    };
    visit.judging = visit.osaka.episode().map(|ep| Judging {
        piece: ep.repair.piece,
        to: ep.repair.to,
        pocket: ep.pocket,
        target: judge_move(home, &frame, ep),
        showing: shown.iter().find(|s| s.item == ep.repair.piece).copied(),
        laid: laid.iter().find(|s| s.item == ep.repair.piece).copied(),
    });
    // The piece in her pocket shows nowhere; its place is kept.
    let carried = visit.osaka.carrying();
    visit.ghost = carried
        .and_then(|piece| laid.iter().find(|s| s.item == piece))
        .map(Shown::cover);
    shown.retain(|s| Some(s.item) != carried);
    shown
}

/// Where the piece of the move she's making would stand on `frame`, if
/// the move still puts its rule right ([`rules::check`]); once she has
/// set it down where it does and is trying it elsewhere, if the rule
/// still holds with it moved ([`rules::check_again`]).
fn judge_move(home: &room::Home, frame: &rules::Frame, ep: &osaka::Episode) -> Option<Shown> {
    if ep.tried > 0 {
        rules::check_again(home, frame, &ep.repair)
    } else {
        rules::check(home, frame, &ep.repair)
    }
}

/// The middle of `at`, on its floor.
fn middle_of(at: &Shown) -> (i32, i32) {
    (at.left + i32::from(at.size().0) / 2, at.floor)
}

/// Where she'd stand to lift `at`, or set it down there: beside it on
/// its floor where she may stay (the side nearer `near` first), else
/// over its middle; and which way she'd face it.
fn reach(at: &Shown, terrain: &Terrain, near: i32) -> Option<((i32, i32), scenes::Side)> {
    let (middle, floor) = middle_of(at);
    let mut xs = at.beside().to_vec();
    xs.sort_by_key(|&x| ((x - near).abs(), x));
    xs.push(middle);
    let x = xs
        .into_iter()
        .find(|&x| terrain.platform_at(x, floor).is_some() && terrain.restful(x, floor))?;
    let side = if x > middle {
        scenes::Side::Left
    } else {
        scenes::Side::Right
    };
    Some(((x, floor), side))
}

/// What she sees of moving her pieces this frame (her terrain read):
/// where she'd stand to lift each piece the repairs move, and what the
/// paint made of the move she's making.
fn arranging(visit: &Visit) -> (Vec<osaka::LiftAt>, Option<osaka::Judged>) {
    let near = visit.osaka.x;
    let mut lift_at: Vec<osaka::LiftAt> = Vec::new();
    for repair in &visit.repairs {
        if lift_at.iter().any(|(p, ..)| *p == repair.piece) {
            continue;
        }
        if let Some(at) = visit
            .shown
            .iter()
            .find(|s| s.item == repair.piece && s.scrap.is_none())
            && let Some((spot, side)) = reach(at, &visit.terrain, near)
        {
            lift_at.push((repair.piece, spot, side));
        }
    }
    let judged = visit.judging.map(|j| {
        let to = if j.pocket { j.target } else { j.showing };
        osaka::Judged {
            piece: j.piece,
            to: j.to,
            pocket: j.pocket,
            holds: j.target.is_some(),
            spot: to.and_then(|at| reach(&at, &visit.terrain, near)),
            home: j.laid.as_ref().map(middle_of),
        }
    });
    (lift_at, judged)
}

/// The stage: her sofa turned away from her TV, both settled on one
/// strip a few cells apart (the TV against a wall, the sofa further in,
/// facing on into the room): a turn where it stands puts it right.
/// Pieces she owns are moved (and unpacked); others are added. Tried on
/// each strip, from each wall, until both show (and everything that
/// showed still does). False when there's nowhere for them.
fn stage_arrange(
    home: &mut room::Home,
    buf: &Buffer,
    view: &IdleView,
    blocked: &dyn Fn(i32, i32) -> bool,
) -> bool {
    use room::{Anchor, Side};
    use sprite::Facing;
    let before = home.clone().project(buf, &view.nooks, blocked);
    let tv_cols = Furniture::Tv.spec().footprint.0;
    for (strip, e) in room::strips(&view.nooks) {
        let room::Strip::Bottom(nook) = strip;
        for side in [Side::Left, Side::Right] {
            // Both face into the room: the sofa, past the TV, away from
            // it.
            let facing = match side {
                Side::Left => Facing::Right,
                Side::Right => Facing::Left,
            };
            let mut tried = home.clone();
            for (item, offset, facing) in [
                (Furniture::Tv, 0, facing),
                (Furniture::Sofa, tv_cols + 4, facing),
            ] {
                let anchor = Anchor { side, offset };
                let cols = item.spec().footprint.0;
                let at = e.pin(e.left(anchor, cols), cols).1;
                let prop = room::Prop {
                    anchor: Some(anchor),
                    ..room::Prop::new(item, nook, at, facing)
                };
                match tried.props.iter_mut().find(|p| p.item == item) {
                    Some(owned) => *owned = prop,
                    None => {
                        tried.add(prop);
                    }
                }
            }
            let shown = tried.project(buf, &view.nooks, blocked);
            let shows = |item: Furniture| shown.iter().any(|s| s.item == item);
            if shows(Furniture::Tv)
                && shows(Furniture::Sofa)
                && before.iter().all(|b| shows(b.item))
            {
                tracing::info!(
                    ?strip,
                    ?side,
                    "houseguest: (stage) a sofa turned from the TV"
                );
                *home = tried;
                return true;
            }
        }
    }
    false
}

/// How often she works out again how to put her home right, when
/// nothing it depends on has changed.
const MEND_MS: u64 = 1000;

/// Work out how she'd put right a rule of her home she would mend (see
/// [`Osaka::to_mend`]; the stage may `force` one), on this frame: each in
/// turn, keeping the first a move mends (one no move mends doesn't keep
/// her from the next). Again when her home, the panes, those rules or
/// the text she has moved changes, and at most every [`MEND_MS`]
/// otherwise (text comes and goes).
#[allow(clippy::too_many_arguments)]
fn mend(
    home: &room::Home,
    buf: &Buffer,
    view: &IdleView,
    visit: &mut Visit,
    shown: &[Shown],
    blocked: &dyn Fn(i32, i32) -> bool,
    force: Option<rules::Grievance>,
    now: u64,
) {
    use std::hash::{Hash, Hasher};
    let wanted = match force {
        Some(key) => vec![key],
        None => visit.osaka.to_mend(&visit.broken),
    };
    let targets: Vec<rules::Broken> = wanted
        .iter()
        .filter_map(|&key| visit.broken.iter().find(|b| b.key == key).cloned())
        .collect();
    if targets.is_empty() {
        visit.repairs.clear();
        visit.mending = None;
        return;
    }
    let mut moved: Vec<(u16, u16)> = visit.layer.cells().collect();
    moved.sort_unstable();
    moved.dedup();
    let mut hasher = std::hash::DefaultHasher::new();
    (home, &view.nooks, &wanted, &moved).hash(&mut hasher);
    let basis = hasher.finish();
    let due = force.is_some()
        || visit
            .mending
            .is_none_or(|(was, at)| was != basis || now >= at.saturating_add(MEND_MS));
    if !due {
        return;
    }
    let made: Vec<Rect> = visit.made.iter().map(|m| m.piece.cover()).collect();
    let frame = rules::Frame {
        buf,
        nooks: &view.nooks,
        blocked,
        shown,
        made: &made,
    };
    let mut examined = 0;
    let found = targets
        .iter()
        .map(|target| {
            let found = rules::search(home, &frame, target);
            examined += found.examined;
            found
        })
        .find(|found| !found.repairs.is_empty())
        .unwrap_or_default();
    debug_assert!(
        found
            .repairs
            .iter()
            .all(|r| rules::check(home, &frame, r).is_some()),
        "a repair found that doesn't hold up: {:?}",
        found.repairs
    );
    if found.repairs != visit.repairs {
        tracing::trace!(
            rules = ?wanted.iter().map(|k| k.label()).collect::<Vec<_>>(),
            examined,
            repairs = ?found.repairs.iter().map(rules::Repair::label).collect::<Vec<_>>(),
            "houseguest: how she'd put her home right"
        );
    }
    visit.repairs = found.repairs;
    visit.mending = Some((basis, now));
}

/// Place the stage's gift `item` where it fits, noting how that went;
/// where her furniture then stands.
#[allow(clippy::too_many_arguments)]
fn place_gift(
    home: &mut room::Home,
    item: Furniture,
    note: &mut Option<Result<String, String>>,
    buf: &Buffer,
    view: &IdleView,
    shown: &[Shown],
    blocked: &dyn Fn(i32, i32) -> bool,
    rng: &mut Rng,
) -> Vec<Shown> {
    let result = if home.owns(item) {
        Err(format!("she already has a {}", item.spec().name))
    } else {
        match home.spot(buf, &view.nooks, shown, blocked, item, rng) {
            Some(prop) => {
                let strip = prop.strip;
                tracing::info!(?item, ?strip, at = prop.at, "houseguest: new furniture");
                if home.add(prop) {
                    Ok(format!("a {} on {strip:?}", item.spec().name))
                } else {
                    Err(format!("{strip:?} is another room's"))
                }
            }
            None => Err(format!("no room for a {}", item.spec().name)),
        }
    };
    *note = Some(result);
    home.project(buf, &view.nooks, blocked)
}

/// The kinds of her real pieces `shown` (boxed or not): what stands in
/// her room ([`osaka::Chances::real`]).
fn real_kinds(shown: &[Shown]) -> Vec<Furniture> {
    shown
        .iter()
        .filter(|s| s.scrap.is_none())
        .map(|s| s.item)
        .collect()
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

/// Whether she could be seated at `(x, y)` to use something: where she
/// may stay, on a floor she stands on. (A parcel comes in only where
/// this holds of its unpack spot: [`room::Home::doorstep`].)
fn seat_spot(terrain: &Terrain, x: i32, y: i32) -> bool {
    terrain.restful(x, y) && terrain.platform_at(x, y).is_some()
}

/// Whether her box standing at `(x, y)` is clear of every shown piece
/// (and the floor beneath one) but `of`, the window she'd lean on (it
/// hangs low: phase 5c D6): where she stands to look out of it, not in
/// front of a sofa it hangs behind, nor anything beside it.
fn clear_of(shown: &[Shown], of: &Shown, x: i32, y: i32) -> bool {
    room::her_box(x, y).is_some_and(|her| {
        shown
            .iter()
            .filter(|s| *s != of)
            .all(|s| !s.cover().intersects(her))
    })
}

fn spots_for(piece: &Shown, shown: &[Shown], terrain: &Terrain, cat: bool) -> Vec<room::Seat> {
    let mut out = Vec::new();
    for &what in piece.uses() {
        // No cat, no petting (but his bed, boxed, is hers to unpack: the
        // cat comes only to a bed out of its box).
        if what == room::Use::Pet && !cat {
            continue;
        }
        // Looking out of the window: leaning on its sill at either end.
        let spots: &[i32] = match what {
            room::Use::LookOut => &piece.look_out_spots(),
            _ if what.inside() => &[0],
            _ => &piece.beside(),
        };
        let seat = spots
            .iter()
            .map(|&beside| piece.seat(what, beside))
            .find(|seat| {
                seat_spot(terrain, seat.x, seat.y)
                    && (what != room::Use::LookOut || clear_of(shown, piece, seat.x, seat.y))
            });
        out.extend(seat);
    }
    // A sofa that faces the TV (see [`room::faces`]) is where to watch
    // it from, turned the way the sofa is (toward the TV). A makeshift
    // one stands on no strip and has no way round: for it, the TV on
    // its floor, where she'd stand beside it to watch (its own columns
    // may run up to a wall, past the floor's last standing spot), and
    // she turns toward it.
    if piece.uses().contains(&room::Use::Lounge) {
        let sofa = piece.seat(room::Use::Lounge, 0);
        let floor = terrain.platform_at(sofa.x, sofa.y);
        let tv = shown.iter().find(|tv| {
            tv.item == Furniture::Tv
                && !tv.boxed
                && match piece.strip {
                    Some(_) => room::faces(piece, tv),
                    None => {
                        floor.is_some()
                            && tv
                                .beside()
                                .into_iter()
                                .any(|x| terrain.platform_at(x, tv.floor) == floor)
                    }
                }
        });
        if let Some(tv) = tv {
            let facing = match piece.strip {
                Some(_) => piece.facing,
                None if tv.left > sofa.x => sprite::Facing::Right,
                None => sprite::Facing::Left,
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

/// Whether `(x, y)` is clear of every real piece `shown` (and the floor
/// beneath one), as a makeshift piece must be to be made there and to
/// stay (see [`builds`], [`tend_made`]).
fn clear_of_real(shown: &[Shown], x: i32, y: i32) -> bool {
    let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
        return false;
    };
    !shown
        .iter()
        .any(|s| s.scrap.is_none() && s.cover().contains((ux, uy).into()))
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
            && clear_of_real(&real, x, y)
    };
    let mut gone = Vec::new();
    let layer = &visit.layer;
    visit.made.retain(|made| {
        let stands =
            !resized && layer.torn_intact(&made.torn) && room::fits(buf, &made.piece, &clear);
        if !stands {
            gone.push((made.torn.clone(), made.used, made.mine(), made.piece.item));
        }
        stands
    });
    for (torn, used, mine, item) in gone {
        tracing::debug!("houseguest: a makeshift piece fell apart");
        visit.layer.mend(&torn);
        // Before she used it: "...my sofa."
        if let Some(mine) = mine
            && !used
        {
            visit.osaka.owe(mind::Loss::Piece(item), mine.at);
        }
    }
    if let Some((seat, since, until)) = visit.osaka.use_span()
        && seat.what == room::Use::Crumple
    {
        let length = until.saturating_sub(since).max(1);
        let stage = 1 + now.saturating_sub(since) * u64::from(scrap::STAGES - 1) / length;
        for made in &mut visit.made {
            if made.piece.piece() == seat.piece
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
/// `cells` (read before anything of hers is painted), for `purpose`.
fn made_of(buf: &Buffer, row: u16, cells: &[u16], piece: &Shown, purpose: room::Use) -> Made {
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
            scrap: piece.scrap.map(|s| scrap::Scrap::new(s.id, &glyphs, seed)),
            ..*piece
        },
        torn: cells.iter().map(|&c| (c, row)).collect(),
        purpose,
        used: false,
    }
}

/// Text she held torn off its line at the last look and doesn't now
/// (see `Osaka::holding`), however she came to let go of it
/// (interrupted, her grip lost, moved on, put somewhere), goes back:
/// what she was reeling in and never made anything of, and what of a
/// strip she borrowed she hadn't slid back yet (once she has, there's
/// nothing to put back). She's sorry about what she dropped. Called as
/// the paint reads the frame (for what moved her on between paints) and
/// again after the paint's own checks (for what moved her on there), so
/// no frame shows text out that she isn't holding.
fn let_go(visit: &mut Visit) {
    let holding = visit.osaka.holding();
    if let Some(old) = visit.reel.take()
        && holding.as_ref() != Some(&old)
    {
        let sources = old.sources();
        let dropped = match &old {
            scenes::Held::Build(_) => !visit.made.iter().any(|m| m.torn == sources),
            scenes::Held::Strip(_) => visit.layer.holds_any(&sources),
        };
        if dropped {
            tracing::info!("houseguest: dropped the text she held; it's back in its line");
            visit.layer.unreel(&sources);
            visit.osaka.owe(mind::Loss::Tear, old.middle());
        }
    }
    visit.reel = holding;
}

/// Lines she could borrow a strip of to read this frame (phase 5c D5):
/// those long enough, while the text layer has room for the strip.
fn borrows(visit: &Visit, pulls: &[scenes::Pull]) -> Vec<scenes::Pull> {
    if visit.layer.cells().count() + scenes::STRIP_GLYPHS > layer::CAP {
        return Vec::new();
    }
    pulls.iter().filter(|p| p.lends()).cloned().collect()
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
    // Clear of her real pieces too, as a made piece must stay (see
    // `tend_made`), or it would fall apart as it's made: her window hangs
    // low enough to meet one built beneath it (phase 5c D6).
    let clear = |x: i32, y: i32| {
        let (Ok(ux), Ok(uy)) = (u16::try_from(x), u16::try_from(y)) else {
            return false;
        };
        !protected.iter().any(|r| r.contains((ux, uy).into())) && clear_of_real(&visit.shown, x, y)
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
            .map(|seat| seat.what)
            .filter(|&what| what != room::Use::Crumple)
            .collect()
    };
    scenes::builds(buf, pulls, &wanted, visit.next_made, &clear, &then)
}

/// Her furniture's colour as text (the ASCII drawings).
fn prop_ink(item: Furniture, truecolor: bool) -> Ink {
    let (rgb, plain) = item.spec().ink;
    let fg = if truecolor { rgb } else { plain };
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

/// Whether the cat is in his bed this visit, of `kind` (about half of
/// them): read off the visit's seed, so it's settled for the whole visit
/// and draws nothing from her generator. A dash home has the coming
/// visit's (`ledger.visits`: it isn't counted), as her empty home does
/// (A22), so he's there or not as she dashes in and out again; a counted
/// visit, its own (`visits - 1`), which a dash promoted as school ends
/// keeps (the count goes up as it becomes one).
fn cat_home(ledger: &Ledger, kind: Kind) -> bool {
    let visit = match kind {
        Kind::Normal => ledger.visits.saturating_sub(1),
        Kind::Dash => ledger.visits,
    };
    cat_home_of(ledger, visit)
}

/// Whether the cat is in his bed on visit number `visit` (from 0): while
/// she's out, it's the coming visit's (`ledger.visits`), so he's there
/// or not as she comes home, and stays so.
fn cat_home_of(ledger: &Ledger, visit: u64) -> bool {
    let owns = ledger
        .home
        .props
        .iter()
        .any(|p| p.item == Furniture::CatBed && !p.boxed);
    owns && ledger.visit_seed(visit) >> 17 & 1 == 1
}

/// What of the world beyond her shows on her furniture: whether the cat
/// is home, and her minute of the day (`None`: her clock isn't fed to
/// her, or isn't running) for her wall clock's dial and her window's
/// sky. Game time, never her script's ([`script::Prop`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct World {
    cat: bool,
    time: Option<u16>,
}

/// Whether any of `shown` tells the time as it's drawn (a clock or a
/// window out of its box): it changes as her clock runs. (Neither is
/// ever made of text: [`scrap::MAKES`] has no piece that tells the
/// time.)
fn tells_time(shown: &[Shown]) -> bool {
    shown.iter().any(|s| s.item.tells_time() && !s.boxed)
}

/// What state `piece` is in, given what the script she's playing shows
/// on her furniture (`prop`; see [`Osaka::prop`]), whether her lamp is
/// dark for the night (`dark`; see [`Osaka::dark`]) and the `world`,
/// each piece from its own: the lamp off while she sleeps or in the
/// night, the fridge open as she looks in, the cat in his bed (`cat`),
/// biting at the end of a petting; the clock's dial and the window's sky
/// at her time of day (plain without one).
fn piece_state(
    piece: &Shown,
    prop: Option<script::Prop>,
    dark: bool,
    world: World,
) -> art::PieceState {
    use art::PieceState;
    use script::Prop;
    if dark && piece.item == Furniture::Lamp {
        return PieceState::LampOff;
    }
    match (piece.item, world.time) {
        (Furniture::Clock, Some(minute)) => return PieceState::Dial(art::Dial::at(minute)),
        (Furniture::Window, Some(minute)) => return PieceState::Sky(art::Sky::at(minute)),
        _ => {}
    }
    let cat = world.cat;
    let state = match prop.filter(|p| p.item() == piece.item) {
        Some(Prop::LampOff) => Some(PieceState::LampOff),
        Some(Prop::FridgeOpen) => Some(PieceState::FridgeOpen),
        Some(Prop::CatBiting) => Some(PieceState::CatBiting),
        // What's on TV is on its screen (see `Looks::tv`).
        Some(Prop::Tv(_)) | None => None,
    };
    match (piece.item, state) {
        // Nobody's home to bite.
        (Furniture::CatBed, _) if !cat => PieceState::Plain,
        (Furniture::CatBed, None) => PieceState::Cat,
        (_, state) => state.unwrap_or(PieceState::Plain),
    }
}

/// A piece of furniture as an image layer that `look`s so, standing on
/// its floor, or hung on the wall above it (its box ends on the row
/// above `at`'s).
fn prop_layer(prop: &Shown, look: Look) -> graphics::Layer {
    let (cols, _) = prop.size();
    let lift = i32::from(prop.lift());
    graphics::Layer {
        look,
        // A symmetric piece is drawn one way (and cached once).
        facing: prop.item.drawn_facing(prop.facing),
        at: (prop.left + i32::from(cols) / 2, prop.floor - lift),
        standing: lift == 0,
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

/// How long a delivery's flap stands open.
const FLAP_MS: u64 = 800;

/// The flap a parcel came in through, `age` ms ago: open, swung in on
/// its hinge at the top, wherever the wall is a plain vertical line
/// clear of protected cells.
fn draw_flap(buf: &mut Buffer, flap: room::Flap, age: u64, protected: &[Rect]) -> Vec<Frozen> {
    let mut painted = Vec::new();
    if age >= FLAP_MS {
        return painted;
    }
    let glyph = match flap.side {
        room::Side::Left => '╲',
        room::Side::Right => '╱',
    };
    for y in flap.rows.0..flap.rows.1 {
        let (Ok(x), Ok(y)) = (u16::try_from(flap.x), u16::try_from(y)) else {
            continue;
        };
        if protected.iter().any(|r| r.contains((x, y).into())) {
            continue;
        }
        let Some(cell) = buf.cell_mut((x, y)) else {
            continue;
        };
        if !matches!(cell.symbol(), "│" | "┃") {
            continue;
        }
        let under = cell.clone();
        let ink = Ink::new(cell.fg, Modifier::empty());
        cell.set_char(glyph);
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
    painted
}

/// The cat in his bed in ASCII, if he's there (its inside row), curled
/// up or biting. Line art only: in ASCII the lamp and the fridge look
/// the same off or on, shut or open. Wildcard-free, so a new state
/// says.
fn cat_glyphs(state: art::PieceState) -> Option<[char; 4]> {
    match state {
        art::PieceState::Cat => Some([' ', '^', '^', ' ']),
        art::PieceState::CatBiting => Some(['!', '^', '^', '!']),
        art::PieceState::Plain
        | art::PieceState::LampOff
        | art::PieceState::FridgeOpen
        | art::PieceState::Dial(_)
        | art::PieceState::Sky(_) => None,
    }
}

/// The wall clock's hand in ASCII, in its face's cell: by the minute
/// hand, so it moves on the quarter-hour as the dial does.
fn dial_glyph(dial: art::Dial) -> char {
    match dial.quarter % 4 {
        0 => '\'',
        1 => '>',
        2 => '.',
        _ => '<',
    }
}

/// The window's two panes of sky in ASCII.
fn sky_glyphs(sky: art::Sky) -> [char; 2] {
    match sky {
        art::Sky::Night => ['*', '.'],
        art::Sky::Dawn => ['_', 'o'],
        art::Sky::Day => ['-', 'o'],
        art::Sky::Dusk => ['~', '~'],
        art::Sky::Evening => ['.', '*'],
    }
}

/// What's drawn over `prop`'s ASCII drawing this frame, cell by cell
/// (never mirrored): what's on the TV's screen, the cat in his bed, the
/// clock's hand, the window's sky. Placed from the piece's footprint as
/// it stands or hangs (`Shown::rect`), never from its floor alone.
fn overrides(prop: &Shown, looks: &Looks) -> Vec<((i32, i32), char)> {
    let mut out = Vec::new();
    // On, the TV's screen shows what's on.
    if let (Some(cells), Some(channel)) = (prop.screen(), looks.tv) {
        out.extend(cells.into_iter().zip(screen_glyphs(channel)));
    }
    if prop.boxed {
        return out;
    }
    let rect = prop.rect();
    let (left, top) = (prop.left, i32::from(rect.y));
    let state = looks.state(prop.item);
    // The cat curls in his bed (its top row).
    if let Some(glyphs) = cat_glyphs(state) {
        out.extend(
            glyphs
                .into_iter()
                .enumerate()
                .map(|(dx, glyph)| ((left + dx as i32, top), glyph)),
        );
    }
    // (Never a makeshift piece's: none tells the time, [`scrap::MAKES`].)
    match state {
        art::PieceState::Dial(dial) => out.push(((left + 1, top + 1), dial_glyph(dial))),
        art::PieceState::Sky(sky) => {
            let [a, b] = sky_glyphs(sky);
            out.extend([((left + 1, top + 1), a), ((left + 2, top + 1), b)]);
        }
        _ => {}
    }
    out
}

/// What's on the TV's two-cell screen in ASCII: static, the shopping
/// channel's sunburst, colour bars, a sunrise, a programme.
fn screen_glyphs(channel: art::Channel) -> [char; 2] {
    match channel {
        art::Channel::Snow(0) => [':', '.'],
        art::Channel::Snow(_) => ['.', ':'],
        art::Channel::Shopping(_) => ['^', '^'],
        art::Channel::ColourBars => ['|', '|'],
        art::Channel::Sunrise => ['o', '_'],
        // The programmes she watches (phase 5c D7): the anchor over the
        // ticker, the sun and a cloud, a penguin on the sea, a bowl under
        // its steam.
        art::Channel::Programme(programme) => match programme {
            art::Programme::News => ['o', '='],
            art::Programme::Weather => ['*', 'c'],
            art::Programme::Penguins => ['i', '~'],
            art::Programme::Cooking => ['~', 'u'],
        },
    }
}

/// `pieces` back to front: each hung piece that a piece standing
/// overlaps (her window, low, behind her sofa: phase 5c D6) first, so
/// the sofa is drawn over it; the rest as they are (no other two share a
/// cell).
fn back_to_front(pieces: &[Shown]) -> Vec<Shown> {
    let behind = |hung: &Shown| {
        hung.lane() == room::Lane::Wall
            && pieces
                .iter()
                .any(|s| s.lane() == room::Lane::Floor && hung.rect().intersects(s.rect()))
    };
    let (mut out, rest): (Vec<Shown>, Vec<Shown>) = pieces.iter().partition(|p| behind(p));
    out.extend(rest);
    out
}

/// `pieces` in groups that overlap (each a piece, or pieces whose
/// footprints meet, as her window and the sofa it hangs behind), in the
/// order their first pieces come: in line art each group is one image,
/// since two images over the same cells would cut each other out.
fn overlapping(pieces: &[Shown]) -> Vec<Vec<Shown>> {
    let mut group: Vec<usize> = (0..pieces.len()).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while let Some(&up) = group.get(i)
            && up != i
        {
            i = up;
        }
        i
    }
    for (i, a) in pieces.iter().enumerate() {
        for (j, b) in pieces.iter().enumerate().skip(i + 1) {
            if a.rect().intersects(b.rect()) {
                let (ra, rb) = (root(&mut group, i), root(&mut group, j));
                if let Some(slot) = group.get_mut(rb.max(ra)) {
                    *slot = ra.min(rb);
                }
            }
        }
    }
    let mut out: Vec<(usize, Vec<Shown>)> = Vec::new();
    for (i, &piece) in pieces.iter().enumerate() {
        let r = root(&mut group, i);
        match out.iter_mut().find(|(at, _)| *at == r) {
            Some((_, members)) => members.push(piece),
            None => out.push((r, vec![piece])),
        }
    }
    out.into_iter().map(|(_, members)| members).collect()
}

/// Paint her furniture (as line art, or as ASCII without graphics),
/// returning what was painted over what. Back to front ([`back_to_front`]:
/// her window behind her sofa); in line art, pieces that overlap as one
/// image ([`overlapping`]).
fn draw_props(
    buf: &mut Buffer,
    graphics: Option<&mut Graphics>,
    shown: &[Shown],
    looks: &Looks,
    truecolor: bool,
) -> Vec<Frozen> {
    /// A footprint cell: where, its glyph this frame, what's under it.
    type Under = (u16, u16, Option<char>, tuirealm::ratatui::buffer::Cell);
    let shown = back_to_front(shown);
    let mut painted = Vec::new();
    // Each footprint cell of `prop` with its glyph this frame and what's
    // under it now.
    let unders_of = |buf: &Buffer,
                     prop: &Shown|
     -> Vec<(u16, u16, Option<char>, tuirealm::ratatui::buffer::Cell)> {
        let over = overrides(prop, looks);
        prop.cells()
            .filter_map(|(x, y, glyph)| {
                let (ux, uy) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
                let glyph = over
                    .iter()
                    .find(|&&(at, _)| at == (x, y))
                    .map(|&(_, over)| over)
                    .or(glyph);
                Some((ux, uy, glyph, buf.cell((ux, uy))?.clone()))
            })
            .collect()
    };
    match graphics {
        Some(graphics) => {
            for group in overlapping(&shown) {
                let unders: Vec<(Shown, Vec<Under>)> =
                    group.iter().map(|p| (*p, unders_of(buf, p))).collect();
                let placed: Vec<bool> = match group.as_slice() {
                    [prop] => vec![paint_prop_art(buf, graphics, prop, looks)],
                    _ => {
                        let layers: Vec<graphics::Layer> = group
                            .iter()
                            .map(|prop| {
                                let look = piece_look(
                                    prop,
                                    art::Layer::Whole,
                                    looks.tv,
                                    false,
                                    looks.state(prop.item),
                                );
                                prop_layer(prop, look)
                            })
                            .collect();
                        // Only its pieces' own cells, and what's blank or
                        // a line it redraws between them, are the image's
                        // to cover: text there (under the window beside
                        // the sofa's end) is no one's to derez.
                        let bounds = group
                            .iter()
                            .map(Shown::cover)
                            .reduce(|a, b| a.union(b))
                            .unwrap_or_default();
                        let theirs = |x: u16, y: u16| {
                            group.iter().any(|p| p.cover().contains((x, y).into()))
                        };
                        let text: Vec<(i32, i32)> = bounds
                            .positions()
                            .filter(|&at| {
                                !theirs(at.x, at.y)
                                    && buf.cell(at).is_some_and(|c| {
                                        c.symbol().chars().next().is_some_and(|c| {
                                            !c.is_whitespace() && graphics::strokes(c).is_none()
                                        })
                                    })
                            })
                            .map(|at| (i32::from(at.x), i32::from(at.y)))
                            .collect();
                        let open = |x: i32, y: i32| !text.contains(&(x, y));
                        if graphics.paint_layers(buf, &layers, &open).is_some() {
                            vec![true; group.len()]
                        } else {
                            // Not all of it hers to cover: each alone.
                            group
                                .iter()
                                .map(|prop| paint_prop_art(buf, graphics, prop, looks))
                                .collect()
                        }
                    }
                };
                for ((prop, unders), placed) in unders.into_iter().zip(placed) {
                    if !placed {
                        continue;
                    }
                    let ink = prop_ink(prop.item, truecolor);
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
        }
        None => {
            // What's under each, before any is painted: where her sofa
            // covers her window's corner, what's under both is the screen's.
            let unders: Vec<(Shown, Vec<Under>)> =
                shown.iter().map(|p| (*p, unders_of(buf, p))).collect();
            for (prop, cells) in unders {
                let prop = &prop;
                let ink = prop_ink(prop.item, truecolor);
                for (x, y, glyph, under) in cells {
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
    // A cell a piece in front painted over is that piece's alone.
    let mut seen = std::collections::HashSet::new();
    painted.reverse();
    painted.retain(|f: &Frozen| seen.insert((f.x, f.y)));
    painted.reverse();
    painted
}

/// Paint her box as line art (what of hers stands in it, `figure`, and
/// the pieces it overlaps, `with`, in one image), plus any bubble as
/// text: her box's art if it was placed. The frozen cells for a dissolve
/// are what the image covers (blank), carrying the glyphs it bursts into:
/// her box's, as her ASCII sprite (she's in sight), else her door's; and
/// each piece's.
/// The pieces are behind her, bar the quilt of a bed she's asleep in
/// (and the sofa's cushion is in her arms when she naps).
#[allow(clippy::too_many_arguments)]
fn draw_art(
    buf: &mut Buffer,
    graphics: &mut Graphics,
    osaka: &Osaka,
    terrain: &Terrain,
    shown: &[Shown],
    figure: Option<Figure>,
    with: Vec<Shown>,
    looks: &Looks,
    now: u64,
    truecolor: bool,
) -> (Vec<Frozen>, Option<BoxArt>, Option<Bubble>) {
    let (pose, face, _) = osaka.appearance(now);
    let (sprite, _) = osaka.picture(now);
    // Her, if she's in sight; and a door in space, standing behind her
    // (through it, she's gone: it's the door alone).
    let her = figure.and_then(Figure::her);
    let door = figure.and_then(Figure::door);
    let under = |buf: &Buffer, x: i32, y: i32| {
        // Only what the image can cover (it's clipped to the same cells):
        // the rest it never drew, and mustn't rain over.
        if !terrain.open(x, y) {
            return None;
        }
        let (ux, uy) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
        Some((ux, uy, buf.cell((ux, uy))?.clone()))
    };
    let mut body: Vec<Frozen> = Vec::new();
    if her.is_some() {
        // Her box bursts into her letters.
        for dy in -sprite::HEIGHT..0 {
            for dx in -(sprite::WIDTH / 2)..=(sprite::WIDTH / 2) {
                let Some((x, y, under)) = under(buf, osaka.x + dx, osaka.y + dy) else {
                    continue;
                };
                let cell = sprite.iter().find(|c| c.dx == dx && c.dy == dy);
                body.push(Frozen {
                    x,
                    y,
                    glyph: cell.map_or('a', |c| c.glyph),
                    ink: ink(cell.map_or(Part::Body, |c| c.part), truecolor),
                    under,
                    face: None,
                    burst: true,
                });
            }
        }
    } else if let Some(door) = door {
        // Out of sight behind her door, it bursts into its own glyphs,
        // as in ASCII.
        body.extend(door.cells().filter_map(|(x, y, glyph)| {
            let (x, y, under) = under(buf, x, y)?;
            Some(Frozen {
                x,
                y,
                glyph,
                ink: DOOR_INK,
                under,
                face: None,
                burst: true,
            })
        }));
    }
    let using = osaka.seat().filter(|seat| seat.what.inside());
    let part = |piece: &Shown, front: bool| {
        let what = using
            .filter(|seat| seat.piece == piece.piece())
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
    let mut layers = Vec::with_capacity(with.len() * 2 + 2);
    for piece in &back_to_front(&with) {
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
    layers.extend(door.map(Door::layer));
    layers.extend(her.map(|at| at.pose(pose, face)));
    for piece in &back_to_front(&with) {
        layers.extend(part(piece, true).map(|layer| prop_layer(piece, layer)));
    }
    let placed = graphics
        .paint_layers(buf, &layers, &|x, y| terrain.open(x, y))
        .is_some();
    let mut painted = if placed { body } else { Vec::new() };
    let (bubble, drawn) = draw(buf, osaka, terrain, shown, now, truecolor, false);
    painted.extend(bubble);
    let image = figure
        .filter(|_| placed)
        .map(|figure| BoxArt { figure, with });
    (painted, image, drawn)
}

/// The middle of a screen `size`, at its foot: where her door is looked
/// for with nowhere else to start from.
fn middle(size: (u16, u16)) -> (i32, i32) {
    (i32::from(size.0) / 2, i32::from(size.1))
}

/// Whether her door may stand at `(x, y)` on `terrain`: where she'd stay
/// (on a floor, and nothing in her box, or the image she'd be drawn in,
/// that she may not stay over).
fn door_fits(terrain: &Terrain, (x, y): (i32, i32)) -> bool {
    terrain.platform_at(x, y).is_some() && terrain.restful(x, y)
}

/// The floor spot nearest `near` where her door may stand
/// ([`door_fits`]), `near` itself if it does; `None` with nowhere.
fn door_spot(terrain: &Terrain, near: (i32, i32)) -> Option<(i32, i32)> {
    if door_fits(terrain, near) {
        return Some(near);
    }
    terrain
        .platforms
        .iter()
        .flat_map(|p| (p.x0..=p.x1).map(move |x| (x, p.y)))
        .filter(|&spot| door_fits(terrain, spot))
        .min_by_key(|&(x, y)| ((x - near.0).abs() + (y - near.1).abs(), y, x))
}

/// Whether the arm of [`Guest::paint_state`] that painted her state
/// painted her rains too, or left them to [`Guest::paint`].
#[must_use]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rains {
    /// Painted under the goodbye's image.
    Painted,
    /// Still to paint, last, over everything.
    ToPaint,
}

/// Paint her rains (`fades`, [`Guest::fades`]) over the frame `buf`
/// (`size`), dropping those done at `now` or made at another size (the
/// geometry they froze against is gone).
fn paint_fades(fades: &mut Vec<Dissolve>, buf: &mut Buffer, size: (u16, u16), now: u64) {
    fades.retain(|fade| fade.size() == size && !fade.done(now));
    for fade in fades {
        fade.paint(buf, now);
    }
}

/// Paint her home standing empty while she's out (`State::Away`) over
/// the finished frame: her furniture as a visit projects it (and nothing
/// a visit's paint sets going: no gift, no order, no parcel), the TV off
/// and the lamp off, the cat as the coming visit has him, her clock and
/// window at her time of day (`time`, when it's fed); her closed
/// door where she went out (moved to the nearest spot it fits only once
/// it doesn't on the frame as drawn, `unkept`); what was in a pane just
/// focused raining out (her door too: it comes back when the pane's left
/// alone). Returns whether her record changed (projecting may settle a
/// piece), as a visit's paint diffs it.
#[allow(clippy::too_many_arguments)]
fn paint_empty(
    empty: &mut Empty,
    fades: &mut Vec<Dissolve>,
    door: &mut Option<DoorAt>,
    ledger: &mut Ledger,
    mut graphics: Option<&mut Graphics>,
    buf: &mut Buffer,
    view: &IdleView,
    unkept: &[Rect],
    nudge: &nudge::Nudge,
    time: Option<u16>,
    now: u64,
    truecolor: bool,
) -> bool {
    let size = (buf.area.width, buf.area.height);
    let origin = door.map_or(i32::from(size.0) / 2, |door| door.x);
    // A pane was just focused: what of hers was in it rains away at once.
    if let Some(focus) = view.focus {
        let out: Vec<Frozen> = empty
            .painted
            .iter()
            .filter(|cell| focus.contains((cell.x, cell.y).into()))
            .cloned()
            .collect();
        if !out.is_empty() && empty.size == size {
            tracing::debug!(
                cells = out.len(),
                "houseguest: her empty home rains out of the focused pane"
            );
            fades.push(Dissolve::new(
                now.saturating_sub(dissolve::RAIN_FROM_MS),
                out,
                origin,
                truecolor,
                size,
            ));
        }
    }
    // Her furniture where a visit's paint would stand it, clear of
    // protected cells.
    let blocked = |cx: i32, cy: i32| {
        let (Ok(ux), Ok(uy)) = (u16::try_from(cx), u16::try_from(cy)) else {
            return true;
        };
        view.protected.iter().any(|r| r.contains((ux, uy).into()))
    };
    let before = ledger.clone();
    let shown = ledger.home.project(buf, &view.nooks, &blocked);
    let changed = *ledger != before;
    let covers: Vec<Rect> = shown.iter().map(Shown::cover).collect();
    let line_art = graphics.is_some();
    // Her door where she went out, unless that no longer fits on the
    // frame as drawn: then the nearest spot that does.
    let mut fit = Terrain::read(buf, unkept, line_art);
    fit.furnish(covers.iter().copied());
    let fits = |door: DoorAt| door_fits(&fit, (door.x, door.y));
    if !door.is_some_and(fits) {
        let near = door.map_or(middle(size), |door| (door.x, door.y));
        let facing = door.map_or(sprite::Facing::Right, |door| door.facing);
        if let Some((x, y)) = door_spot(&fit, near) {
            tracing::debug!(x, y, "houseguest: her closed door stands");
            *door = Some(DoorAt { x, y, facing });
        }
    }
    // Shown where it fits, and out of the pane she keeps clear.
    let standing = door.filter(|&door| {
        fits(door)
            && !view
                .protected
                .iter()
                .any(|&rect| osaka::box_meets(rect, (door.x, door.y)))
    });
    let terrain = if view.protected == unkept {
        fit
    } else {
        let mut terrain = Terrain::read(buf, &view.protected, line_art);
        terrain.furnish(covers.iter().copied());
        terrain
    };
    // Nobody's watching, reading by the lamp or petting him; her clock
    // and window tell the time all the same.
    let world = World {
        cat: empty.cat,
        time,
    };
    let looks = Looks {
        tv: None,
        states: shown
            .iter()
            .map(|p| (p.item, piece_state(p, None, true, world)))
            .collect(),
    };
    nudge.paint(buf, now);
    // In line art, the pieces her door overlaps go in its image (two
    // images would cut each other out).
    let drawn = match standing.filter(|_| line_art) {
        Some(door) => terrain::image(door.x, door.y, &covers).with,
        None => vec![false; covers.len()],
    };
    let (mut with, mut apart) = (Vec::new(), Vec::new());
    for (&piece, drawn) in shown.iter().zip(drawn) {
        if drawn {
            with.push(piece);
        } else {
            apart.push(piece);
        }
    }
    let mut painted = draw_props(buf, graphics.as_deref_mut(), &apart, &looks, truecolor);
    let mut image = None;
    if let Some(door) = standing {
        let door = Door::closed(door.x, door.y, door.facing);
        let (cells, art) = draw_door(buf, graphics, door, with, &looks, &terrain, truecolor);
        painted.extend(cells);
        image = art;
    }
    empty.shown = shown;
    empty.apart = apart;
    empty.image = image;
    empty.painted = painted;
    empty.looks = looks;
    empty.size = size;
    changed
}

/// Paint her door standing on its own (she's out, see [`paint_empty`]):
/// as glyphs, or as line art in one image with the pieces it overlaps
/// (`with`). Returns what was painted over what (the cells it covers,
/// bursting into its glyphs, and each piece's), and in line art the
/// image, if it was placed (a goodbye holds it until the rain).
fn draw_door(
    buf: &mut Buffer,
    graphics: Option<&mut Graphics>,
    door: Door,
    with: Vec<Shown>,
    looks: &Looks,
    terrain: &Terrain,
    truecolor: bool,
) -> (Vec<Frozen>, Option<BoxArt>) {
    // Only what it may cover (clipped to the same cells): the rest it
    // never drew, and mustn't rain over.
    let under = |buf: &Buffer, x: i32, y: i32| {
        if !terrain.open(x, y) {
            return None;
        }
        let (ux, uy) = (u16::try_from(x).ok()?, u16::try_from(y).ok()?);
        Some((ux, uy, buf.cell((ux, uy))?.clone()))
    };
    let Some(graphics) = graphics else {
        let mut painted = Vec::new();
        for (x, y, glyph) in door.cells() {
            if let Some((ux, uy, under)) = under(buf, x, y)
                && put(buf, x, y, glyph, DOOR_INK)
            {
                painted.push(Frozen {
                    x: ux,
                    y: uy,
                    glyph,
                    ink: DOOR_INK,
                    under,
                    face: None,
                    burst: false,
                });
            }
        }
        return (painted, None);
    };
    let mut body: Vec<Frozen> = door
        .cells()
        .filter_map(|(x, y, glyph)| {
            let (x, y, under) = under(buf, x, y)?;
            Some(Frozen {
                x,
                y,
                glyph,
                ink: DOOR_INK,
                under,
                face: None,
                burst: true,
            })
        })
        .collect();
    let mut layers = Vec::with_capacity(with.len() + 1);
    for piece in &back_to_front(&with) {
        let look = piece_look(
            piece,
            art::Layer::Whole,
            looks.tv,
            false,
            looks.state(piece.item),
        );
        layers.push(prop_layer(piece, look));
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
    layers.push(door.layer());
    if graphics
        .paint_layers(buf, &layers, &|x, y| terrain.open(x, y))
        .is_some()
    {
        let figure = Figure::door_alone(door);
        (body, Some(BoxArt { figure, with }))
    } else {
        (Vec::new(), None)
    }
}

#[cfg(test)]
mod tests;
