//! Rolling subtitle-speaker membership and stable color-slot allocation.
//!
//! This stays local to one UI: subtitle speakers are release-specific and
//! never synchronized. A slot is stable while a speaker has appeared within
//! the last five minutes; expired slots are recycled for new speakers.

use std::collections::{BTreeMap, BTreeSet};

use super::theme::SPEAKER_WINDOW_MILLIS;

use crate::player::SpeakerName;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveSpeaker {
    last_seen: u64,
    slot: usize,
}

/// The distinct subtitle speakers seen in the rolling activity window.
#[derive(Clone, Debug, Default)]
pub struct SpeakerColors {
    active: BTreeMap<String, ActiveSpeaker>,
    now: u64,
}

impl SpeakerColors {
    /// Observe a cue and return its stable color slot, when it names a
    /// speaker. The cue arrival clock is explicit so tests remain fully
    /// deterministic. Backward clock corrections never rewind the window.
    pub fn observe(&mut self, speaker: Option<&SpeakerName>, arrival_millis: u64) -> Option<usize> {
        self.advance(arrival_millis);
        let speaker: &str = speaker?;
        if let Some(active) = self.active.get_mut(speaker) {
            active.last_seen = self.now;
            return Some(active.slot);
        }

        let used: BTreeSet<usize> = self.active.values().map(|active| active.slot).collect();
        // There are `active.len()` used slots, so one of 0..=len must be
        // free. This avoids an unbounded counter and naturally recycles the
        // earliest hole after a speaker expires.
        let slot = (0..=self.active.len())
            .find(|candidate| !used.contains(candidate))
            .unwrap_or(self.active.len());
        self.active.insert(
            speaker.to_owned(),
            ActiveSpeaker {
                last_seen: self.now,
                slot,
            },
        );
        Some(slot)
    }

    /// Move the rolling window forward even when no named subtitle arrives.
    /// Returns whether the active set changed, so the UI can avoid redrawing
    /// on quiet-scene clock ticks that expire nothing.
    pub fn advance(&mut self, now_millis: u64) -> bool {
        let previous_len = self.active.len();
        self.now = self.now.max(now_millis);
        self.expire();
        self.active.len() != previous_len
    }

    /// Move the rolling window forward by locally *elapsed* millis — the
    /// shell's monotonic ticks. The window's absolute domain is the
    /// shared-clock arrival stamps (see [`Self::advance`]); elapsed time
    /// is domain-free, so quiet scenes expire leases without importing a
    /// rewindable clock into the window (2026-08-20 review).
    pub fn tick(&mut self, elapsed_millis: u64) -> bool {
        let previous_len = self.active.len();
        self.now = self.now.saturating_add(elapsed_millis);
        self.expire();
        self.active.len() != previous_len
    }

    fn expire(&mut self) {
        let now = self.now;
        self.active
            .retain(|_, active| now.saturating_sub(active.last_seen) <= SPEAKER_WINDOW_MILLIS);
    }

    /// Number of speakers still active in the five-minute window.
    pub fn len(&self) -> usize {
        self.active.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_are_stable_and_distinct_while_active() {
        let mut colors = SpeakerColors::default();
        assert_eq!(
            colors.observe(SpeakerName::new("Frieren").as_ref(), 10),
            Some(0)
        );
        assert_eq!(
            colors.observe(SpeakerName::new("Fern").as_ref(), 20),
            Some(1)
        );
        assert_eq!(
            colors.observe(SpeakerName::new("Frieren").as_ref(), 30),
            Some(0)
        );
        assert_eq!(colors.len(), 2);
    }

    #[test]
    fn five_minute_boundary_is_inclusive_then_slot_is_reused() {
        let mut colors = SpeakerColors::default();
        assert_eq!(
            colors.observe(SpeakerName::new("old").as_ref(), 1_000),
            Some(0)
        );
        colors.advance(1_000 + SPEAKER_WINDOW_MILLIS);
        assert_eq!(colors.len(), 1, "exactly five minutes is still active");
        colors.advance(1_001 + SPEAKER_WINDOW_MILLIS);
        assert_eq!(colors.len(), 0);
        assert_eq!(
            colors.observe(
                SpeakerName::new("new").as_ref(),
                1_002 + SPEAKER_WINDOW_MILLIS
            ),
            Some(0)
        );
    }

    #[test]
    fn repeat_refreshes_the_lease() {
        let mut colors = SpeakerColors::default();
        colors.observe(SpeakerName::new("Frieren").as_ref(), 0);
        colors.observe(
            SpeakerName::new("Frieren").as_ref(),
            SPEAKER_WINDOW_MILLIS - 1,
        );
        colors.advance(SPEAKER_WINDOW_MILLIS + 1);
        assert_eq!(colors.len(), 1);
        colors.advance(2 * SPEAKER_WINDOW_MILLIS);
        assert_eq!(colors.len(), 0);
    }

    #[test]
    fn unnamed_cues_advance_time_without_consuming_a_slot() {
        let mut colors = SpeakerColors::default();
        colors.observe(SpeakerName::new("old").as_ref(), 0);
        assert_eq!(colors.observe(None, SPEAKER_WINDOW_MILLIS + 1), None);
        assert_eq!(colors.len(), 0);
    }

    #[test]
    fn backwards_arrival_does_not_rewind_or_resurrect() {
        let mut colors = SpeakerColors::default();
        colors.observe(SpeakerName::new("old").as_ref(), 0);
        colors.advance(SPEAKER_WINDOW_MILLIS + 1);
        assert_eq!(colors.len(), 0);
        colors.observe(None, 1);
        assert_eq!(colors.len(), 0);

        // A genuinely new cue with a corrected-backward timestamp is an
        // observation *now*, not an already-expired lease.
        assert_eq!(colors.observe(SpeakerName::new("new").as_ref(), 1), Some(0));
        assert_eq!(colors.len(), 1);
    }
}
