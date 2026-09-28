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

// Line-art Osaka, under review; wired into painting once approved.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "art awaits review before wiring")
)]
mod art;
mod cells;
mod dissolve;
mod idle;
mod osaka;
mod sprite;
mod terrain;

use std::time::Duration;

use tuirealm::ratatui::buffer::Buffer;
use tuirealm::ratatui::style::{Color, Modifier};

use cells::{Ink, put};
use dissolve::{Dissolve, Frozen};
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

struct Visit {
    osaka: Osaka,
    terrain: Terrain,
    /// What she painted in the last frame, with the real cells beneath —
    /// the dissolve's frozen composite if activity arrives now.
    painted: Vec<Frozen>,
    size: (u16, u16),
}

enum State {
    Absent,
    /// The idle delay elapsed; she enters on the next paint.
    Arriving,
    Visiting(Box<Visit>),
    Leaving(Box<Dissolve>),
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
        }
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
                    self.state = State::Leaving(Box::new(dissolve));
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
            State::Visiting(visit) => visit.osaka.tick(now, &visit.terrain, &mut self.rng),
            State::Leaving(dissolve) => {
                if dissolve.done(now) {
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
            State::Leaving(dissolve) => dissolve.next_frame(now),
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
                let terrain = Terrain::read(buf, &view.protected);
                if let Some(osaka) = Osaka::arrive(now, &terrain, i32::from(size.0), &mut self.rng)
                {
                    tracing::info!("houseguest arrived");
                    self.state = State::Visiting(Box::new(Visit {
                        osaka,
                        terrain,
                        painted: Vec::new(),
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
            State::Leaving(dissolve) => {
                if dissolve.size() != size {
                    // The geometry she froze against is gone.
                    self.state = State::Absent;
                } else {
                    dissolve.paint(buf, now);
                }
            }
            State::Visiting(visit) => {
                visit.terrain = Terrain::read(buf, &view.protected);
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
                visit.painted = draw(buf, &visit.osaka, &visit.terrain, now, self.truecolor);
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
) -> Vec<Frozen> {
    let (sprite, bubble) = osaka.picture(now);
    let mut wanted: Vec<(i32, i32, char, Ink, Option<usize>)> = sprite
        .iter()
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
        let fits = |start: i32| (start..start + len).all(|x| terrain.open(x, row));
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
            });
        }
    }
    painted
}

#[cfg(test)]
mod tests;
