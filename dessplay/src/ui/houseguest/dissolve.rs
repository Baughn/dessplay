//! The exit: ~3.75 s of "matrix rain" over every cell she changed,
//! settling each back to the real UI (proposal 2026-09-28-houseguest,
//! The exit dissolve). A pure function of the frozen composite, the live
//! frame, and the time since activity — the spoiler tease's discipline:
//! frames derive from wall time, never per-tick counters.

use std::collections::BTreeMap;

use tuirealm::ratatui::buffer::{Buffer, Cell};
use tuirealm::ratatui::style::{Color, Modifier};

use super::cells::{Ink, put, width};
use super::sprite::Face;
use crate::ui::theme;

/// The whole dissolve; the overlay is gone after this.
pub(super) const DURATION_MS: u64 = 3750;
/// Every rain column has settled by this point (then a safety band).
const SETTLE_BY_MS: u64 = 3600;
/// Rain begins after her startled beat.
pub(super) const RAIN_FROM_MS: u64 = 675;
/// Latest any column may start.
const RAIN_LATEST_MS: u64 = 1950;
/// Noise re-rolls at ~16 fps.
pub(super) const FRAME_MS: u64 = 60;
/// Her face turns from startled to a goodbye smile.
pub(super) const SMILE_FROM_MS: u64 = 375;
/// Slowest rain, in rows per millisecond (8 rows/s).
const MIN_SPEED: f64 = 0.008;
/// Her wave alternates arm up/down this often (line art).
pub(super) const WAVE_MS: u64 = 150;
/// How much later each column's drop starts, per column from her.
const RIPPLE_MS: u64 = 90;
/// Per-column start jitter.
const JITTER_MS: u64 = 180;

/// One cell of her world at the moment activity began.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Frozen {
    pub x: u16,
    pub y: u16,
    pub glyph: char,
    pub ink: Ink,
    /// The real UI cell underneath when she painted it.
    pub under: Cell,
    /// Index into her face, for the startled/smile beat.
    pub face: Option<usize>,
    /// Part of her line-art box: hidden until the rain begins (her image
    /// shows), then noise at once. Other cells hold their frozen glyph
    /// until a drop reaches them.
    pub burst: bool,
}

impl Frozen {
    /// A hole she left: shows blank until the rain re-knits `under`.
    pub fn hole(x: u16, y: u16, under: Cell) -> Self {
        Self {
            x,
            y,
            glyph: ' ',
            ink: Ink::new(Color::Reset, Modifier::empty()),
            under,
            face: None,
            burst: false,
        }
    }

    /// A glyph she moved here, in its own colours.
    pub fn glyph(x: u16, y: u16, moved: &Cell, under: Cell) -> Self {
        Self {
            x,
            y,
            glyph: moved.symbol().chars().next().unwrap_or(' '),
            ink: Ink::new(moved.fg, moved.modifier),
            under,
            face: None,
            burst: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Column {
    start: u64,
    top: i32,
    trail: f64,
}

/// What a frozen cell shows at some instant.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Frozen,
    Head,
    Trail { faded: bool },
    Settled,
}

/// A running exit.
#[derive(Clone, Debug)]
pub(super) struct Dissolve {
    t0: u64,
    cells: Vec<Frozen>,
    /// Sticky: a settled cell never shows noise again.
    settled: Vec<bool>,
    /// Frozen cell index by position.
    at: BTreeMap<(u16, u16), usize>,
    columns: BTreeMap<u16, Column>,
    speed: f64,
    seed: u64,
    truecolor: bool,
    size: (u16, u16),
    /// Her face for the first beat, before the goodbye smile: startled,
    /// unless told otherwise ([`Dissolve::startled`]).
    startled: Face,
}

fn hash(seed: u64, x: u16) -> u64 {
    dessplay_core::spoiler::seed(seed, "houseguest-rain", usize::from(x))
}

impl Dissolve {
    /// Freeze `cells` at `t0`, rippling out from column `origin`.
    pub fn new(
        t0: u64,
        cells: Vec<Frozen>,
        origin: i32,
        truecolor: bool,
        size: (u16, u16),
    ) -> Self {
        let mut columns: BTreeMap<u16, Column> = BTreeMap::new();
        for cell in &cells {
            let h = hash(t0, cell.x);
            let start = (RAIN_FROM_MS
                + RIPPLE_MS * (i32::from(cell.x) - origin).unsigned_abs() as u64
                + h % JITTER_MS)
                .min(RAIN_LATEST_MS);
            let column = columns.entry(cell.x).or_insert(Column {
                start,
                top: i32::from(cell.y) - 1,
                trail: (3 + (h >> 8) % 4) as f64,
            });
            column.top = column.top.min(i32::from(cell.y) - 1);
        }
        let mut speed = MIN_SPEED;
        for cell in &cells {
            if let Some(column) = columns.get(&cell.x) {
                let rows = f64::from(i32::from(cell.y) - column.top) + column.trail;
                let window = (SETTLE_BY_MS - column.start) as f64;
                speed = speed.max(rows / window);
            }
        }
        let settled = vec![false; cells.len()];
        let at = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| ((cell.x, cell.y), index))
            .collect();
        Self {
            t0,
            cells,
            settled,
            at,
            columns,
            speed,
            seed: t0,
            truecolor,
            size,
            startled: Face::Surprised,
        }
    }

    /// This exit with `face` for her first beat instead of the startled
    /// one (a sleepy blink, woken in the night).
    pub fn startled(mut self, face: Face) -> Self {
        self.startled = face;
        self
    }

    /// When activity began.
    pub fn started(&self) -> u64 {
        self.t0
    }

    /// Whether her line-art box still shows what she froze over (no
    /// modal, typing or new text has landed in her space).
    pub fn unchanged(&self, buf: &Buffer) -> bool {
        self.cells
            .iter()
            .filter(|cell| cell.burst)
            .all(|cell| buf.cell((cell.x, cell.y)) == Some(&cell.under))
    }

    /// Whether the overlay is gone.
    pub fn done(&self, now: u64) -> bool {
        now.saturating_sub(self.t0) >= DURATION_MS
    }

    /// When the next frame is due.
    pub fn next_frame(&self, now: u64) -> u64 {
        let t = now.saturating_sub(self.t0);
        (self.t0 + (t / FRAME_MS + 1) * FRAME_MS).min(self.t0 + DURATION_MS)
    }

    /// The terminal size the dissolve was frozen against.
    pub fn size(&self) -> (u16, u16) {
        self.size
    }

    /// Whether the last paint drew this cell from the frozen frame (its
    /// glyph or rain over it): not yet settled, and the real UI beneath
    /// unchanged (a changed cell settles at once).
    #[cfg(test)]
    pub fn painting(&self, x: u16, y: u16) -> bool {
        self.at
            .get(&(x, y))
            .is_some_and(|&index| !self.settled.get(index).copied().unwrap_or(true))
    }

    fn phase(&self, cell: &Frozen, t: u64) -> Phase {
        if t >= SETTLE_BY_MS {
            return Phase::Settled;
        }
        let Some(column) = self.columns.get(&cell.x) else {
            return Phase::Settled;
        };
        if t < column.start {
            return Phase::Frozen;
        }
        let d = (t - column.start) as f64 * self.speed - f64::from(i32::from(cell.y) - column.top);
        if d < 0.0 {
            Phase::Frozen
        } else if d < 1.0 {
            Phase::Head
        } else if d < column.trail {
            Phase::Trail {
                faded: d >= column.trail / 2.0,
            }
        } else {
            Phase::Settled
        }
    }

    fn head_ink(&self) -> Ink {
        if self.truecolor {
            Ink::new(Color::Rgb(210, 255, 215), Modifier::BOLD)
        } else {
            Ink::new(Color::Reset, Modifier::BOLD)
        }
    }

    fn trail_ink(&self, faded: bool) -> Ink {
        match (self.truecolor, faded) {
            (true, false) => Ink::new(Color::Rgb(90, 200, 120), Modifier::empty()),
            (true, true) => Ink::new(theme::TRUECOLOR_MUTED_FOREGROUND, Modifier::empty()),
            (false, false) => Ink::new(Color::LightGreen, Modifier::empty()),
            (false, true) => Ink::new(Color::LightGreen, Modifier::DIM),
        }
    }

    /// Whether writing `cell` would blank the other half of a live wide
    /// glyph that is not itself still animating. Wide pairs settle
    /// together: once one half shows the real glyph, the other must not
    /// knock it out again.
    fn strands_partner(&self, buf: &Buffer, cell: &Frozen) -> bool {
        let wide = |x: u16| buf.cell((x, cell.y)).is_some_and(|c| width(c) > 1);
        let partner = match cell.x.checked_sub(1) {
            Some(left) if wide(left) => Some(left),
            _ if wide(cell.x) => cell.x.checked_add(1),
            _ => None,
        };
        partner.is_some_and(|x| {
            self.at
                .get(&(x, cell.y))
                .is_none_or(|&index| self.settled.get(index).copied().unwrap_or(true))
        })
    }

    /// Paint the frame for `now` over the live frame in `buf`.
    pub fn paint(&mut self, buf: &mut Buffer, now: u64) {
        let t = now.saturating_sub(self.t0);
        let generation = (t / FRAME_MS) as u32;
        let face = if t < SMILE_FROM_MS {
            self.startled
        } else {
            Face::Pleased
        }
        .glyphs();
        let width = usize::from(self.size.0);
        for index in 0..self.cells.len() {
            if self.settled.get(index).copied().unwrap_or(true) {
                continue;
            }
            let Some(cell) = self.cells.get(index) else {
                continue;
            };
            // Fast lane: the real UI changed here since she froze —
            // whatever the user is doing is never scrambled.
            let unchanged = matches!(buf.cell((cell.x, cell.y)), Some(live) if *live == cell.under);
            let phase = if unchanged && !self.strands_partner(buf, cell) {
                match self.phase(cell, t) {
                    // Her image is showing; the caller painted it.
                    _ if cell.burst && t < RAIN_FROM_MS => continue,
                    Phase::Frozen if cell.burst => Phase::Trail { faded: false },
                    phase => phase,
                }
            } else {
                Phase::Settled
            };
            let (x, y) = (i32::from(cell.x), i32::from(cell.y));
            let noise = || {
                let target = cell.under.symbol().chars().next().unwrap_or(' ');
                let seed = dessplay_core::spoiler::seed(
                    self.seed,
                    "houseguest",
                    usize::from(cell.y) * width + usize::from(cell.x),
                );
                dessplay_core::spoiler::rain_glyph(target, cell.glyph, seed, generation)
            };
            match phase {
                Phase::Settled => {
                    if let Some(settled) = self.settled.get_mut(index) {
                        *settled = true;
                    }
                }
                Phase::Frozen => {
                    let glyph = cell
                        .face
                        .and_then(|i| face.get(i).copied())
                        .unwrap_or(cell.glyph);
                    put(buf, x, y, glyph, cell.ink);
                }
                Phase::Head => {
                    put(buf, x, y, noise(), self.head_ink());
                }
                Phase::Trail { faded } => {
                    put(buf, x, y, noise(), self.trail_ink(faded));
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tuirealm::ratatui::layout::Rect;
    use tuirealm::ratatui::style::Style;

    const INK: Ink = Ink {
        fg: Color::Reset,
        modifier: Modifier::BOLD,
    };

    fn live(width: u16, height: u16, text: &[(u16, u16, &str)]) -> Buffer {
        let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
        for &(x, y, s) in text {
            buf.set_string(x, y, s, Style::new());
        }
        super::super::cells::sanitize(&mut buf);
        buf
    }

    fn frozen_over(buf: &Buffer, positions: &[(u16, u16)]) -> Vec<Frozen> {
        positions
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| Frozen {
                x,
                y,
                glyph: if i % 2 == 0 { '/' } else { 'V' },
                ink: INK,
                under: buf.cell((x, y)).cloned().unwrap_or_default(),
                face: (i % 5 == 1).then_some(i % 3),
                burst: i % 3 == 0,
            })
            .collect()
    }

    fn frame(dissolve: &mut Dissolve, real: &Buffer, now: u64) -> Buffer {
        let mut buf = real.clone();
        dissolve.paint(&mut buf, now);
        buf
    }

    #[allow(clippy::type_complexity)]
    fn arbitrary_scene()
    -> impl Strategy<Value = (u16, u16, Vec<(u16, u16)>, Vec<(u16, u16, String)>, u64, i32)> {
        (8u16..40, 6u16..20).prop_flat_map(|(w, h)| {
            (
                Just(w),
                Just(h),
                proptest::collection::vec((0..w, 0..h), 1..40),
                proptest::collection::vec((0..w, 0..h, "[a-z漢─│ ]{1,4}"), 0..12),
                0u64..1_000_000,
                0..i32::from(w),
            )
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(64)))]

        /// At T0 + 2.5 s the frame is exactly the real UI, the dissolve
        /// is deterministic, settled cells never show noise again, and no
        /// frame ever holds half of a wide glyph.
        #[test]
        fn dissolve_ends_on_the_real_frame((w, h, positions, text, t0, origin) in arbitrary_scene()) {
            let text: Vec<(u16, u16, &str)> = text.iter().map(|(x, y, s)| (*x, *y, s.as_str())).collect();
            let real = live(w, h, &text);
            let mut positions = positions;
            positions.sort_unstable();
            positions.dedup();
            let cells = frozen_over(&real, &positions);
            let mut a = Dissolve::new(t0, cells.clone(), origin, false, (w, h));
            let mut b = Dissolve::new(t0, cells, origin, false, (w, h));
            let mut ever_settled = vec![false; positions.len()];
            for step in 0..=(DURATION_MS / 20) {
                let now = t0 + step * 20;
                let fa = frame(&mut a, &real, now);
                let fb = frame(&mut b, &real, now);
                prop_assert_eq!(&fa, &fb, "deterministic at t={}", step * 20);
                for (i, &(x, y)) in positions.iter().enumerate() {
                    let shows_real = fa.cell((x, y)) == real.cell((x, y));
                    if ever_settled[i] {
                        prop_assert!(shows_real, "({x},{y}) un-settled at t={}", step * 20);
                    }
                    ever_settled[i] |= a.settled[i];
                }
                for y in 0..h {
                    for x in 0..w.saturating_sub(1) {
                        let cell = fa.cell((x, y)).unwrap();
                        if super::super::cells::width(cell) > 1 {
                            prop_assert_eq!(fa.cell((x + 1, y)).unwrap().symbol(), " ");
                        }
                    }
                }
            }
            prop_assert_eq!(frame(&mut a, &real, t0 + DURATION_MS), real);
        }

        /// A cell the real UI changed after she froze shows the real UI
        /// from that frame on.
        #[test]
        fn changed_cells_are_never_scrambled((w, h, positions, _text, t0, origin) in arbitrary_scene()) {
            let real = live(w, h, &[]);
            let mut positions = positions;
            positions.sort_unstable();
            positions.dedup();
            let cells = frozen_over(&real, &positions);
            let mut dissolve = Dissolve::new(t0, cells, origin, true, (w, h));
            let mut changed = real.clone();
            for &(x, y) in &positions {
                changed.set_string(x, y, "!", Style::new());
            }
            for step in 0..=(DURATION_MS / 100) {
                let now = t0 + step * 100;
                prop_assert_eq!(frame(&mut dissolve, &changed, now), changed.clone());
            }
        }
    }

    #[test]
    fn she_is_startled_then_smiles_before_the_rain() {
        let real = live(10, 6, &[]);
        let cells: Vec<Frozen> = "(._.)"
            .chars()
            .enumerate()
            .map(|(i, glyph)| Frozen {
                x: 3 + i as u16,
                y: 2,
                glyph,
                ink: INK,
                under: Cell::default(),
                face: (1..=3).contains(&i).then(|| i - 1),
                burst: false,
            })
            .collect();
        let mut dissolve = Dissolve::new(1000, cells, 5, false, (10, 6));
        let row = |buf: &Buffer| {
            (3..8)
                .map(|x| buf.cell((x, 2)).unwrap().symbol().to_string())
                .collect::<String>()
        };
        assert_eq!(row(&frame(&mut dissolve, &real, 1000)), "(o_o)");
        assert_eq!(row(&frame(&mut dissolve, &real, 1400)), "(^_^)");
        assert_eq!(row(&frame(&mut dissolve, &real, 4800)), "     ");
    }
}
