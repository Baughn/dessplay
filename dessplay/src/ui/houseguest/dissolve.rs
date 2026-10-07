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
use unicode_width::UnicodeWidthChar;

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
    /// Frozen cell index by position: the last painted there (on top).
    at: BTreeMap<(u16, u16), usize>,
    /// The first painted there (lowest), whose `under` is the real UI's.
    first_at: BTreeMap<(u16, u16), usize>,
    /// For each frozen cell, the wide glyph it is the second half of, if
    /// it's one: a cell she froze just after a frozen wide glyph, and
    /// painted before it (under its second half), is that glyph's, not
    /// its own (see [`Dissolve::paint`]).
    half_of: Vec<Option<usize>>,
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
        let at: BTreeMap<(u16, u16), usize> = cells
            .iter()
            .enumerate()
            .map(|(index, cell)| ((cell.x, cell.y), index))
            .collect();
        let mut first_at: BTreeMap<(u16, u16), usize> = BTreeMap::new();
        for (index, cell) in cells.iter().enumerate() {
            first_at.entry((cell.x, cell.y)).or_insert(index);
        }
        // Left to right, so a wide glyph's second half is known before
        // the cell after it is looked at (one that is a second half has
        // none of its own). Only what was painted before the glyph (every
        // cell frozen there: the last is on top) is under its second half;
        // a cell painted after it knocked it out, as in the frame she
        // painted, and paint order alone shows that.
        let mut half_of = vec![None; cells.len()];
        for (&(x, y), &index) in &at {
            let lead = x
                .checked_sub(1)
                .and_then(|left| at.get(&(left, y)).copied())
                .filter(|&lead| {
                    index < lead
                        && half_of.get(lead).is_some_and(Option::is_none)
                        && cells.get(lead).is_some_and(|cell| wide(cell.glyph))
                });
            if let Some(slot) = half_of.get_mut(index) {
                *slot = lead;
            }
        }
        Self {
            t0,
            cells,
            settled,
            at,
            first_at,
            half_of,
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

    /// When it ends: the frame without it is due then.
    #[cfg(test)]
    pub fn ends_at(&self) -> u64 {
        self.t0 + DURATION_MS
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

    /// Whether her face is in it (her ASCII sprite in sight as it froze:
    /// a goodbye of her, not only of her things).
    #[cfg(test)]
    pub fn has_face(&self) -> bool {
        self.cells.iter().any(|cell| cell.face.is_some())
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

    /// Whether writing `cell` (the `index`th) would blank the other half
    /// of a live wide glyph that is not itself still animating
    /// (`settling`: settled, or settling this frame, whatever order the
    /// two are painted in). Wide pairs settle together: once one half
    /// shows the real glyph, the other must not knock it out again. What
    /// shows at the partner's cell is judged by the first cell of hers
    /// painted there (its `under` is the real UI's; one over it settles
    /// once it does). A frozen wide glyph and its own second half are
    /// one brick, never each other's partner: writing the glyph covers
    /// both.
    fn strands_partner(
        &self,
        buf: &Buffer,
        index: usize,
        cell: &Frozen,
        settling: &[bool],
    ) -> bool {
        let wide = |x: u16| buf.cell((x, cell.y)).is_some_and(|c| width(c) > 1);
        let partner = match cell.x.checked_sub(1) {
            Some(left) if wide(left) => Some(left),
            _ if wide(cell.x) => cell.x.checked_add(1),
            _ => None,
        };
        let brick = |other: usize| {
            self.half_of.get(other).copied().flatten() == Some(index)
                || self.half_of.get(index).copied().flatten() == Some(other)
        };
        partner.is_some_and(|x| match self.at.get(&(x, cell.y)) {
            None => true,
            Some(&top) if brick(top) => false,
            Some(_) => self
                .first_at
                .get(&(x, cell.y))
                .is_none_or(|&first| settling.get(first).copied().unwrap_or(true)),
        })
    }

    /// Whether the `index`th cell, a wide glyph with no second half of
    /// its own, is knocked out this frame: a cell of hers painted after
    /// it stands in its second half (it knocked the glyph out in the
    /// frame she painted) and settles (the real UI shows there), which
    /// writing the glyph whole would cover again. It settles with it.
    fn knocked(&self, index: usize, settling: &[bool]) -> bool {
        let Some(cell) = self.cells.get(index) else {
            return false;
        };
        if !wide(cell.glyph) || self.half_of.get(index).copied().flatten().is_some() {
            return false;
        }
        let Some(right) = cell.x.checked_add(1) else {
            return false;
        };
        self.at
            .get(&(right, cell.y))
            .is_some_and(|&top| top > index)
            && self
                .first_at
                .get(&(right, cell.y))
                .is_some_and(|&first| settling.get(first).copied().unwrap_or(true))
    }

    /// Whether `buf` still shows what was under `cell` when she froze it.
    fn holds(&self, buf: &Buffer, cell: &Frozen) -> bool {
        matches!(buf.cell((cell.x, cell.y)), Some(live) if *live == cell.under)
    }

    /// Whether the cell after the wide glyph `cell` still shows what her
    /// glyph's second half covered: what was under it, where she froze
    /// it too, else the blank a moved glyph only ever lands on.
    fn second_half_holds(&self, buf: &Buffer, cell: &Frozen) -> bool {
        let Some(x) = cell.x.checked_add(1) else {
            return false;
        };
        let live = buf.cell((x, cell.y));
        match self.at.get(&(x, cell.y)).and_then(|&i| self.cells.get(i)) {
            Some(half) => live == Some(&half.under),
            None => {
                live.is_some_and(|live| live.symbol() == " " && !super::cells::untouchable(live))
            }
        }
    }

    /// What `cell` shows at `t`, the real UI under it unchanged: `None`
    /// while her image shows there (the caller painted it).
    fn rain_phase(&self, cell: &Frozen, t: u64) -> Option<Phase> {
        Some(match self.phase(cell, t) {
            // Her image is showing; the caller painted it.
            _ if cell.burst && t < RAIN_FROM_MS => return None,
            Phase::Frozen if cell.burst => Phase::Trail { faded: false },
            phase => phase,
        })
    }

    /// This frame's phase for each frozen wide glyph and the cell after
    /// it she froze too (its second half), judged on the live frame
    /// before anything is written: the glyph's own, settled if the real
    /// UI changed under either half; the second half shows nothing of
    /// its own while the glyph shows whole (`Some(None)`), and rains and
    /// settles with it. `None` for every other cell.
    fn pairs(&self, buf: &Buffer, t: u64) -> Vec<Option<Option<Phase>>> {
        let mut pairs: Vec<Option<Option<Phase>>> = vec![None; self.cells.len()];
        for (index, cell) in self.cells.iter().enumerate() {
            if !wide(cell.glyph) || self.half_of.get(index).copied().flatten().is_some() {
                continue;
            }
            let phase = if self.settled.get(index).copied().unwrap_or(true)
                || !self.holds(buf, cell)
                || !self.second_half_holds(buf, cell)
            {
                Some(Phase::Settled)
            } else {
                self.rain_phase(cell, t)
            };
            if let Some(slot) = pairs.get_mut(index) {
                *slot = Some(phase);
            }
        }
        for (index, cell) in self.cells.iter().enumerate() {
            let Some(lead) = self.half_of.get(index).copied().flatten() else {
                continue;
            };
            let phase = match pairs.get(lead).copied().flatten().flatten() {
                // The glyph shows whole (or her image does): nothing of
                // its own here.
                Some(Phase::Frozen) | None => None,
                Some(Phase::Settled) => Some(Phase::Settled),
                Some(_) if !self.holds(buf, cell) => Some(Phase::Settled),
                rain => rain,
            };
            if let Some(slot) = pairs.get_mut(index) {
                *slot = Some(phase);
            }
        }
        pairs
    }

    /// Paint the frame for `now` over the live frame in `buf`.
    ///
    /// Each cell in turn over what's painted so far, so a cell she froze
    /// over another of her cells shows while that one shows what it
    /// showed then. A frozen wide glyph is one brick with the cell after
    /// it: while it holds, its second half's own frozen cell (a hole
    /// another glyph left, say) shows nothing of its own; when a drop
    /// reaches it, both rain as two narrow cells, and they settle
    /// together (see [`Dissolve::pairs`]). So no frame holds half of one,
    /// and none knocks out the other half.
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
        let pairs = self.pairs(buf, t);
        // The live frame under each cell, before any is written: a cell
        // is judged on it, unless another of hers was painted there this
        // frame (it was frozen over that one). Writing beside a wide
        // glyph blanks it, which is no change of the real UI's.
        let live: Vec<Option<Cell>> = self
            .cells
            .iter()
            .map(|cell| buf.cell((cell.x, cell.y)).cloned())
            .collect();
        // Which cells settle this frame (or have): by the clock, by the
        // real UI changing under them, or with their wide glyph; a wide
        // glyph's second half only with it (its own clock and cell are
        // the glyph's). Asked only of the first cell painted at a place
        // (see [`Dissolve::strands_partner`]), so the live frame is what
        // was under it.
        let mut settling: Vec<bool> = self
            .cells
            .iter()
            .enumerate()
            .map(|(index, cell)| {
                let pair = pairs.get(index).copied().flatten();
                self.settled.get(index).copied().unwrap_or(true)
                    || pair == Some(Some(Phase::Settled))
                    || self.half_of.get(index).copied().flatten().is_none()
                        && (self.phase(cell, t) == Phase::Settled
                            || live.get(index).and_then(Option::as_ref) != Some(&cell.under))
            })
            .collect();
        // A wide glyph knocked out by a cell painted after it settles with
        // that cell (right to left, so the cell after it is judged first).
        let mut order: Vec<usize> = (0..self.cells.len()).collect();
        order.sort_by_key(|&index| {
            std::cmp::Reverse(self.cells.get(index).map_or(0, |cell| cell.x))
        });
        for index in order {
            if self.knocked(index, &settling)
                && let Some(slot) = settling.get_mut(index)
            {
                *slot = true;
            }
        }
        let mut written = std::collections::HashSet::new();
        for index in 0..self.cells.len() {
            if self.settled.get(index).copied().unwrap_or(true) {
                continue;
            }
            let Some(cell) = self.cells.get(index) else {
                continue;
            };
            let under = match written.contains(&(cell.x, cell.y)) {
                true => buf.cell((cell.x, cell.y)),
                false => live.get(index).and_then(Option::as_ref),
            };
            // Fast lane: the real UI changed here since she froze —
            // whatever the user is doing is never scrambled.
            let unchanged = under == Some(&cell.under)
                && !self.strands_partner(buf, index, cell, &settling)
                && !self.knocked(index, &settling);
            let phase = match pairs.get(index).copied().flatten() {
                // Her wide glyph shows whole over this second half.
                Some(None) => continue,
                Some(Some(phase)) if unchanged => phase,
                None if unchanged => match self.rain_phase(cell, t) {
                    Some(phase) => phase,
                    None => continue,
                },
                _ => Phase::Settled,
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
                    if put(buf, x, y, glyph, cell.ink) {
                        written.insert((cell.x, cell.y));
                    }
                }
                Phase::Head => {
                    if put(buf, x, y, noise(), self.head_ink()) {
                        written.insert((cell.x, cell.y));
                    }
                }
                Phase::Trail { faded } => {
                    if put(buf, x, y, noise(), self.trail_ink(faded)) {
                        written.insert((cell.x, cell.y));
                    }
                }
            }
        }
    }
}

/// Whether `glyph` takes two cells.
fn wide(glyph: char) -> bool {
    glyph.width().unwrap_or(0) > 1
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

    /// The few marks her lines may use beyond ASCII ([`NARROW`]) are one
    /// cell wide under the width the renderer uses (unicode-width's,
    /// not its CJK variant), and nothing of hers that keeps wide glyphs
    /// whole takes them for wide: `put` writes one without touching the
    /// cell after it, a cell holding one is a cell wide, and the rain
    /// never pairs it with its neighbour. A line may use them; any other
    /// character beyond ASCII is refused (`line!`, at compile time).
    ///
    /// [`NARROW`]: super::super::NARROW
    #[test]
    fn her_narrow_marks_are_one_cell_wide() {
        use super::super::cells::width;
        use unicode_width::UnicodeWidthStr;
        // The marks the user named, all on the list.
        let named = ['…', '♪', '—', '–', '’', '‘', '“', '”', '·'];
        assert!(named.iter().all(|c| super::super::NARROW.contains(c)));
        for mark in super::super::NARROW {
            let text = mark.to_string();
            assert_eq!(mark.width(), Some(1), "{mark:?}");
            assert_eq!(text.width(), 1, "{mark:?}");
            assert!(!wide(mark), "{mark:?}: the rain takes it for wide");
            let mut buf = live(3, 1, &[(0, 0, "xyz")]);
            assert!(put(&mut buf, 0, 0, mark, INK), "{mark:?}");
            assert_eq!(width(buf.cell((0, 0)).unwrap()), 1, "{mark:?}");
            assert_eq!(buf.cell((1, 0)).unwrap().symbol(), "y", "{mark:?}");
            assert!(
                super::super::narrow(&text),
                "{mark:?}: not allowed in a line"
            );
            assert!(
                super::super::narrow(&format!("Hm{mark} ok{mark}")),
                "{mark:?}"
            );
        }
        // Wide, or beyond the list, refused.
        for other in ["漢", "é", "★", "\u{2028}"] {
            assert!(!super::super::narrow(other), "{other:?}");
        }
    }

    /// Frozen cells at `positions`, with glyphs both narrow and wide (a
    /// wide one is a glyph she moved, shown whole in her composite).
    fn frozen_over(buf: &Buffer, positions: &[(u16, u16)]) -> Vec<Frozen> {
        positions
            .iter()
            .enumerate()
            .map(|(i, &(x, y))| Frozen {
                x,
                y,
                glyph: ['/', 'V', '漢'][i % 3],
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

    /// The text row `    w漢 l漢` (a wide glyph at 5 and at 9), with `w`
    /// moved to 2 and the first 漢 to 3 — its second half on `w`'s hole
    /// — as her layer composes it: the real frame, the composite and its
    /// frozen cells.
    fn a_wide_glyph_moved_onto_a_hole() -> (Buffer, Buffer, Vec<Frozen>) {
        let real = live(12, 1, &[(4, 0, "w漢 l漢")]);
        let mut layer = super::super::layer::TextLayer::default();
        assert!(layer.take(&real, &[], (4, 0), (2, 0)), "w moves");
        assert!(
            layer.take(&real, &[], (5, 0), (3, 0)),
            "漢 moves onto w's hole"
        );
        let mut composite = real.clone();
        let frozen = layer.paint(&mut composite);
        (real, composite, frozen)
    }

    /// No frame of `buf` holds half of a wide glyph.
    fn whole_glyphs(buf: &Buffer) -> Result<(), String> {
        let area = buf.area;
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right().saturating_sub(1) {
                if super::super::cells::width(buf.cell((x, y)).unwrap()) > 1
                    && buf.cell((x + 1, y)).unwrap().symbol() != " "
                {
                    return Err(format!("half a wide glyph at ({x}, {y})"));
                }
            }
        }
        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(256)))]

        /// Text she moved, as her layer composes it, rains out whole: over
        /// arbitrary rows of narrow and wide glyphs, with arbitrary glyphs
        /// moved wherever the layer takes them, no frame of the rain holds
        /// half of a wide glyph, a cell that settled shows the real UI from
        /// then on, and the rain ends on the real frame; and wherever the
        /// real UI changes after she froze (someone typing), it shows
        /// what changed from that frame on.
        #[test]
        fn moved_text_rains_out_whole(
            rows in proptest::collection::vec((0u16..4, "[a-z漢 ]{1,10}"), 1..4),
            moves in proptest::collection::vec((0u16..16, 0u16..4, -4i16..=4, 0u16..4, any::<bool>()), 1..16),
            typed in proptest::collection::vec((0u16..16, 0u16..4), 0..3),
            t0 in 0u64..100_000,
            origin in 0i32..16,
        ) {
            let text: Vec<(u16, u16, &str)> = rows.iter().map(|(y, s)| (0, *y, s.as_str())).collect();
            let real = live(16, 4, &text);
            let mut layer = super::super::layer::TextLayer::default();
            // Slid along its row (as a pull or a tear has them), or
            // carried anywhere.
            for &(sx, sy, dx, ty, slide) in &moves {
                let to = match slide {
                    true => (sx.saturating_add_signed(dx), sy),
                    false => (sx.saturating_add_signed(dx * 4).min(15), ty),
                };
                let _ = layer.take(&real, &[], (sx, sy), to);
            }
            let mut composite = real.clone();
            let frozen = layer.paint(&mut composite);
            prop_assert!(whole_glyphs(&composite).is_ok(), "the composite: {:?}", whole_glyphs(&composite));
            let mut changed = real.clone();
            for &(x, y) in &typed {
                changed.set_string(x, y, "!", Style::new());
            }
            super::super::cells::sanitize(&mut changed);
            for (live_frame, label) in [(&real, "real"), (&changed, "typed")] {
                let mut dissolve = Dissolve::new(t0, frozen.clone(), origin, false, (16, 4));
                for step in 0..=(DURATION_MS / 20) {
                    let now = t0 + step * 20;
                    let at = frame(&mut dissolve, live_frame, now);
                    prop_assert!(whole_glyphs(&at).is_ok(), "{} t={}: {:?}", label, step * 20, whole_glyphs(&at));
                    // Settled (in the frame it settles, too): the live UI.
                    for (i, cell) in frozen.iter().enumerate() {
                        let shows = at.cell((cell.x, cell.y)) == live_frame.cell((cell.x, cell.y));
                        prop_assert!(
                            shows || !dissolve.settled[i],
                            "{} ({}, {}) settled but not shown at t={}", label, cell.x, cell.y, step * 20
                        );
                    }
                    for &(x, y) in &typed {
                        if label == "typed" && changed.cell((x, y)) != real.cell((x, y)) {
                            prop_assert_eq!(at.cell((x, y)), changed.cell((x, y)), "typed ({}, {}) at t={}", x, y, step * 20);
                        }
                    }
                }
                prop_assert_eq!(&frame(&mut dissolve, live_frame, t0 + DURATION_MS), live_frame);
            }
        }
    }

    /// A wide glyph she moved so that its second half covers the hole
    /// another left rains out whole: in every frame of every rain over
    /// it, rippling from anywhere, it shows whole or not at all, and the
    /// rain ends on the real frame. (Phase 5c step 12d: the second half's
    /// hole rained noise beside the frozen glyph, and the rain wrote the
    /// glyph without its second half.)
    #[test]
    fn a_wide_glyph_she_moved_rains_out_whole() {
        let (real, composite, frozen) = a_wide_glyph_moved_onto_a_hole();
        whole_glyphs(&composite).unwrap();
        for t0 in (0..40_000).step_by(997) {
            for origin in 0..12 {
                let mut dissolve = Dissolve::new(t0, frozen.clone(), origin, false, (12, 1));
                for step in 0..=(DURATION_MS / 10) {
                    let at = frame(&mut dissolve, &real, t0 + step * 10);
                    whole_glyphs(&at)
                        .unwrap_or_else(|e| panic!("t0={t0} origin={origin} t={}: {e}", step * 10));
                }
                assert_eq!(frame(&mut dissolve, &real, t0 + DURATION_MS), real);
            }
        }
    }

    /// The text row `a漢漢` on the third row, its first 漢 carried to
    /// the top left, onto blank cells (no frozen cell under its second
    /// half): the real frame and the frozen cells.
    fn a_wide_glyph_moved_onto_blanks() -> (Buffer, Vec<Frozen>) {
        let real = live(16, 4, &[(0, 2, "a漢漢")]);
        let mut layer = super::super::layer::TextLayer::default();
        assert!(layer.take(&real, &[], (1, 2), (0, 0)), "漢 moves");
        let mut composite = real.clone();
        let frozen = layer.paint(&mut composite);
        assert!(
            frozen.iter().all(|f| (f.x, f.y) != (1, 0)),
            "nothing frozen under its second half"
        );
        (real, frozen)
    }

    /// The second half of a wide glyph she moved is the glyph's: where
    /// the real UI changes under it after she froze (someone typed
    /// there), the rain shows what they typed from that frame on, and
    /// never half of the glyph; whether the second half covered a hole
    /// she left (frozen too) or a blank (not).
    #[test]
    fn a_wide_glyph_she_moved_never_hides_what_changed_under_its_half() {
        let (real, _, frozen) = a_wide_glyph_moved_onto_a_hole();
        let (blanks, over_blanks) = a_wide_glyph_moved_onto_blanks();
        for (real, frozen, half, label) in [
            (real, frozen, (4, 0), "onto a hole"),
            (blanks, over_blanks, (1, 0), "onto blanks"),
        ] {
            let size = (real.area.width, real.area.height);
            let mut changed = real.clone();
            changed.set_string(half.0, half.1, "X", Style::new());
            for origin in 0..i32::from(size.0) {
                let mut dissolve = Dissolve::new(0, frozen.clone(), origin, false, size);
                for step in 0..=(DURATION_MS / 10) {
                    let at = frame(&mut dissolve, &changed, step * 10);
                    whole_glyphs(&at)
                        .unwrap_or_else(|e| panic!("{label} origin={origin} t={}: {e}", step * 10));
                    assert_eq!(
                        at.cell(half).unwrap().symbol(),
                        "X",
                        "{label} origin={origin} t={}",
                        step * 10
                    );
                }
            }
        }
    }

    /// The run `漢語` slid two cells left, as a pull has it: 語 lands on
    /// 漢's hole, over the real 漢 (a live wide glyph), its second half
    /// on the hole 漢's second half left. With a letter carried up a tall
    /// screen (a fast rain, short trails) and rippling from anywhere, the moved 語 holds
    /// until its drop reaches it and rains before it settles: its own
    /// second half, settling by its own column's clock, is no partner
    /// it could strand (they're one brick, and settle together).
    #[test]
    fn a_wide_glyph_moved_over_a_live_one_rains_before_it_settles() {
        let (w, h) = (12, 48);
        let y = h - 1;
        let real = live(w, h, &[(2, y, "漢語"), (10, y, "b")]);
        let mut layer = super::super::layer::TextLayer::default();
        // A letter carried to the top: a drop that falls the whole
        // screen in time, so the rain is fast and its trails short.
        assert!(layer.take(&real, &[], (10, y), (10, 0)), "b moves");
        assert!(layer.take(&real, &[], (2, y), (0, y)), "漢 moves");
        assert!(
            layer.take(&real, &[], (4, y), (2, y)),
            "語 moves onto 漢's hole"
        );
        let mut composite = real.clone();
        let frozen = layer.paint(&mut composite);
        whole_glyphs(&composite).unwrap();
        let lead = frozen
            .iter()
            .position(|f| (f.x, f.y) == (2, y))
            .expect("語 frozen");
        assert_eq!(frozen[lead].glyph, '語');
        for t0 in (0..20_000).step_by(331) {
            for origin in 0..i32::from(w) {
                let mut dissolve = Dissolve::new(t0, frozen.clone(), origin, false, (w, h));
                let mut rained = false;
                for step in 0..=(DURATION_MS / 5) {
                    let t = step * 5;
                    let phase = dissolve.phase(&frozen[lead], t);
                    frame(&mut dissolve, &real, t0 + t);
                    rained |= matches!(phase, Phase::Head | Phase::Trail { .. });
                    assert!(
                        !dissolve.settled[lead] || rained || t >= SETTLE_BY_MS,
                        "t0={t0} origin={origin}: 語 settled at t={t} before its drop reached it ({phase:?})"
                    );
                }
            }
        }
    }

    /// Cells of hers frozen in paint order, each with what was under it
    /// as she painted it: `(x, y, glyph)` written in turn over `real`
    /// with [`put`] (a moved glyph's layer cells come first, as the
    /// layer paints before her). Returns the composite too.
    fn painted_in_turn(real: &Buffer, cells: &[(u16, u16, char)]) -> (Buffer, Vec<Frozen>) {
        let mut composite = real.clone();
        let frozen = cells
            .iter()
            .map(|&(x, y, glyph)| {
                let under = composite.cell((x, y)).unwrap().clone();
                assert!(put(&mut composite, i32::from(x), i32::from(y), glyph, INK));
                Frozen {
                    x,
                    y,
                    glyph,
                    ink: INK,
                    under,
                    face: None,
                    burst: false,
                }
            })
            .collect();
        (composite, frozen)
    }

    /// Her cells over a live wide glyph: `(` on its first cell, then a
    /// piece's `#` on its second and her `o` over that. While all of
    /// them hold, the frame is what she painted: the `#` beneath still
    /// shows there (the `o` over it), so the real glyph's second half
    /// is not showing and the `(` beside it strands nothing: it holds
    /// with them.
    #[test]
    fn her_cells_layered_beside_a_live_wide_glyph_hold_together() {
        let real = live(8, 2, &[(4, 1, "漢")]);
        let (composite, frozen) = painted_in_turn(&real, &[(4, 1, '('), (5, 1, '#'), (5, 1, 'o')]);
        for origin in 0..8 {
            let mut dissolve = Dissolve::new(0, frozen.clone(), origin, false, (8, 2));
            for step in 0..=(RAIN_FROM_MS / 10) {
                let at = frame(&mut dissolve, &real, step * 10);
                assert_eq!(at, composite, "origin={origin} t={}", step * 10);
            }
            assert_eq!(frame(&mut dissolve, &real, DURATION_MS), real);
        }
    }

    /// A wide glyph she moved, then one of her other cells (a bubble's
    /// letter, say) painted over its second half, as the frame has them:
    /// the later cell knocked the glyph out. Until the rain, the frame is
    /// what she painted (the cell, beside a blank), never the glyph
    /// whole with the later cell hidden as its second half.
    #[test]
    fn a_cell_painted_over_a_moved_wide_glyphs_half_shows_as_painted() {
        let real = live(8, 2, &[(0, 1, "a漢")]);
        let mut layer = super::super::layer::TextLayer::default();
        assert!(layer.take(&real, &[], (1, 1), (4, 0)), "漢 moves");
        let mut composite = real.clone();
        let mut frozen = layer.paint(&mut composite);
        let (composite, bubble) = painted_in_turn(&composite, &[(5, 0, 'h')]);
        frozen.extend(bubble);
        whole_glyphs(&composite).unwrap();
        assert_eq!(composite.cell((4, 0)).unwrap().symbol(), " ");
        for origin in 0..8 {
            let mut dissolve = Dissolve::new(0, frozen.clone(), origin, false, (8, 2));
            for step in 0..=(RAIN_FROM_MS / 10) {
                let at = frame(&mut dissolve, &real, step * 10);
                assert_eq!(at, composite, "origin={origin} t={}", step * 10);
            }
            assert_eq!(frame(&mut dissolve, &real, DURATION_MS), real);
        }
    }

    /// A cell she froze over another of hers (her sprite over a piece,
    /// say) is judged on the frame as painted, not the live UI: while
    /// both hold, the later one shows; the earlier one is not changed
    /// under it by her own paint.
    #[test]
    fn a_cell_frozen_over_another_of_hers_shows_over_it() {
        let real = live(6, 2, &[]);
        let mut composite = real.clone();
        let lower = Frozen {
            x: 2,
            y: 1,
            glyph: '#',
            ink: INK,
            under: real.cell((2, 1)).unwrap().clone(),
            face: None,
            burst: false,
        };
        assert!(put(&mut composite, 2, 1, '#', INK));
        let upper = Frozen {
            glyph: 'o',
            under: composite.cell((2, 1)).unwrap().clone(),
            ..lower.clone()
        };
        assert!(put(&mut composite, 2, 1, 'o', INK));
        for origin in 0..6 {
            let mut dissolve =
                Dissolve::new(0, vec![lower.clone(), upper.clone()], origin, false, (6, 2));
            for step in 0..=(RAIN_FROM_MS / 10) {
                let at = frame(&mut dissolve, &real, step * 10);
                assert_eq!(
                    at.cell((2, 1)).unwrap().symbol(),
                    "o",
                    "origin={origin} t={}",
                    step * 10
                );
            }
            assert_eq!(frame(&mut dissolve, &real, DURATION_MS), real);
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
