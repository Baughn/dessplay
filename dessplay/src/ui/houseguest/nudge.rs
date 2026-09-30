//! The scrollback nudge: when the chat log is scrolled back and
//! messages have gone unseen below it for a minute, its accordion is
//! poked (by Osaka, when she visits at all) and shakes — motion catches
//! the eye where a static pattern doesn't (design.md, Chat scrollback
//! accordion).

use tuirealm::ratatui::buffer::{Buffer, Cell};
use tuirealm::ratatui::layout::Rect;

use super::idle::Scrollback;

/// Messages go unseen this long before the first poke; pokes are at
/// least this far apart.
pub(super) const NUDGE_MS: u64 = 60_000;
/// How long the accordion shakes.
pub(super) const SHAKE_MS: u64 = 1500;
/// One step of the shake (the row slides a column right, back, left,
/// back).
const SHAKE_STEP_MS: u64 = 100;
const SHAKE: [i32; 4] = [0, 1, 0, -1];
/// A due poke that can't happen yet (a modal is up, or she's mid-exit)
/// is looked at again after this long.
pub(super) const RETRY_MS: u64 = 1000;

#[derive(Clone, Debug, Default)]
pub(super) struct Nudge {
    /// The log as of the last paint (`None` when it follows the newest
    /// line).
    back: Option<Scrollback>,
    /// When a message first went unseen, this time scrolled back.
    since: Option<u64>,
    /// The last poke: when, and how many messages were unseen then.
    poked: Option<(u64, usize)>,
    /// Not before this (a due poke that had to wait).
    not_before: u64,
    /// Shaking since.
    shaking: Option<u64>,
}

impl Nudge {
    /// The log as the frame just drawn shows it. Back at the newest
    /// line, everything starts over.
    pub fn observe(&mut self, back: Option<Scrollback>, now: u64) {
        match back {
            None => *self = Self::default(),
            Some(back) => {
                if back.unseen == 0 {
                    self.since = None;
                } else {
                    self.since.get_or_insert(now);
                }
                if self.back.is_some_and(|old| old.accordion != back.accordion) {
                    self.shaking = None;
                }
                self.back = Some(back);
            }
        }
    }

    /// The accordion drawn in the last frame.
    pub fn accordion(&self) -> Option<Rect> {
        self.back.map(|back| back.accordion)
    }

    /// When the next poke is due: a minute after a message first went
    /// unseen, then a minute after the last poke if more have arrived
    /// since.
    pub fn due_at(&self) -> Option<u64> {
        let back = self.back?;
        let first = self.since? + NUDGE_MS;
        let due = match self.poked {
            None => first,
            Some((_, unseen)) if back.unseen <= unseen => return None,
            Some((at, _)) => first.max(at + NUDGE_MS),
        };
        Some(due.max(self.not_before))
    }

    pub fn due(&self, now: u64) -> bool {
        self.due_at().is_some_and(|due| now >= due)
    }

    /// A due poke has to wait a moment.
    pub fn wait(&mut self, now: u64) {
        self.not_before = now + RETRY_MS;
    }

    /// The poke is under way (whoever does it): the next one needs more
    /// unseen messages, and another minute.
    pub fn poked(&mut self, now: u64) {
        let unseen = self.back.map_or(0, |back| back.unseen);
        tracing::debug!(unseen, "chat accordion: poked");
        self.poked = Some((now, unseen));
    }

    /// Start shaking the accordion.
    pub fn shake(&mut self, now: u64) {
        if self.back.is_some() {
            self.shaking = Some(now);
        }
    }

    pub fn shaking(&self, now: u64) -> bool {
        self.shaking.is_some_and(|since| now < since + SHAKE_MS)
    }

    /// When the picture next changes (a shake step, or a poke falling
    /// due).
    pub fn next_at(&self, now: u64) -> Option<u64> {
        let shake = self
            .shaking
            .filter(|&since| now < since + SHAKE_MS)
            .map(|since| since + (now.saturating_sub(since) / SHAKE_STEP_MS + 1) * SHAKE_STEP_MS);
        match (shake, self.due_at()) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Advance to `now`: whether the screen could change.
    pub fn advance(&mut self, now: u64) -> bool {
        let shaking = self.shaking.is_some();
        if !self.shaking(now) {
            self.shaking = None;
        }
        shaking || self.due(now)
    }

    /// Shake the accordion in `buf`: its cells slide a column to and fro
    /// (the zigzag flips, the count jiggles).
    pub fn paint(&self, buf: &mut Buffer, now: u64) {
        let (Some(since), Some(area)) = (self.shaking, self.accordion()) else {
            return;
        };
        if now >= since + SHAKE_MS {
            return;
        }
        let step = SHAKE[(now.saturating_sub(since) / SHAKE_STEP_MS) as usize % SHAKE.len()];
        if step == 0 {
            return;
        }
        let area = area.intersection(buf.area);
        let row: Vec<Cell> = (area.x..area.right())
            .filter_map(|x| buf.cell((x, area.y)).cloned())
            .collect();
        let n = row.len() as i32;
        for (i, x) in (area.x..area.right()).enumerate() {
            let from = (i as i32 - step).rem_euclid(n.max(1)) as usize;
            if let (Some(cell), Some(src)) = (buf.cell_mut((x, area.y)), row.get(from)) {
                *cell = src.clone();
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn back(unseen: usize) -> Option<Scrollback> {
        Some(Scrollback {
            accordion: Rect::new(1, 9, 10, 1),
            unseen,
        })
    }

    #[test]
    fn a_minute_after_the_first_unseen_message_then_a_minute_per_poke_with_more() {
        let mut nudge = Nudge::default();
        nudge.observe(back(0), 0);
        assert_eq!(nudge.due_at(), None, "nothing missed yet");
        nudge.observe(back(1), 5_000);
        assert_eq!(nudge.due_at(), Some(65_000));
        // More arriving doesn't restart the clock.
        nudge.observe(back(3), 30_000);
        assert_eq!(nudge.due_at(), Some(65_000));
        nudge.poked(65_000);
        assert_eq!(nudge.due_at(), None, "nothing new since the poke");
        nudge.observe(back(4), 70_000);
        assert_eq!(nudge.due_at(), Some(125_000));
        // Back at the newest line: all forgotten.
        nudge.observe(None, 80_000);
        nudge.observe(back(0), 81_000);
        assert_eq!(nudge.due_at(), None);
    }

    #[test]
    fn the_shake_slides_the_row_and_settles() {
        let mut nudge = Nudge::default();
        nudge.observe(back(1), 0);
        nudge.shake(1000);
        let mut buf = Buffer::with_lines(["", "", "", "", "", "", "", "", "", "└╱╲╱╲ab╱╲╱╲┘"]);
        let before = buf.clone();
        nudge.paint(&mut buf, 1000 + SHAKE_STEP_MS);
        let row: String = (0..12).map(|x| buf[(x, 9)].symbol().to_owned()).collect();
        assert_eq!(row, "└╲╱╲╱╲ab╱╲╱┘");
        let mut buf = before.clone();
        nudge.paint(&mut buf, 1000 + SHAKE_MS);
        assert_eq!(buf, before);
        assert!(nudge.advance(1000 + SHAKE_MS));
        assert!(!nudge.advance(1000 + SHAKE_MS));
    }
}
