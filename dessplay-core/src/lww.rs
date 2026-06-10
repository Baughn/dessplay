//! Last-writer-wins registers.
//!
//! [`LwwCell`] is DessPlay's own register CRDT: a pure max-merge over
//! [`Lww`]-wrapped values. Applying an op and merging a state are the
//! same operation — keep the maximum of `(timestamp, value)` — which is
//! commutative, associative, and idempotent under *any* delivery order,
//! with no causal metadata at all.
//!
//! ## Why not `crdts::MVReg`?
//!
//! The original design nested `MVReg<Lww<V>>` inside `crdts::Map`.
//! Property testing found two view-divergence bugs in that composition,
//! both rooted in the same impedance mismatch: nested put clocks are
//! map-global while the Map's remove/merge machinery reasons
//! entry-scoped, so `Map::rm` and even plain `Map::merge` trim value
//! clocks and corrupt dominance between writes (see
//! tests/regressions.rs and docs/sync-state.md). Since every DessPlay
//! register resolves to an LWW winner anyway, the multi-value register
//! bought nothing but the bug surface. `LwwCell` carries no clocks to
//! corrupt.
//!
//! ## Timestamp discipline
//!
//! A causally-later write with an *older or equal* timestamp can lose
//! under pure LWW (equal stamps fall to the value tiebreak). Writers
//! must therefore issue **Lamport-monotonic** timestamps:
//! `max(shared_now, last_issued + 1)`, where `last_issued` is bumped
//! not only by the writer's own stamps but by every LWW timestamp it
//! *observes* — remote ops, merges, snapshots, and state loaded from
//! storage. Self-monotonicity alone is not enough: two actors writing
//! in the same shared-clock millisecond would tie, and the causally
//! later write (e.g. the server forcing Paused right after seeing a
//! client's Playing) could lose the tiebreak. Found by the Phase 5
//! EOF tests.

use std::convert::Infallible;

use crdts::{CmRDT, CvRDT, ResetRemove, VClock};
use serde::{Deserialize, Serialize};

use crate::types::SharedTimestamp;

/// A timestamped value. The derived `Ord` compares `(timestamp, value)`,
/// which is exactly the LWW resolution order.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Lww<V> {
    /// Shared-clock write time. Compared first.
    pub timestamp: SharedTimestamp,
    /// The wrapped value. Tiebreaker for equal timestamps.
    pub value: V,
}

impl<V> Lww<V> {
    /// Wrap a value with its write timestamp.
    pub fn new(timestamp: SharedTimestamp, value: V) -> Self {
        Self { timestamp, value }
    }
}

/// A last-writer-wins register. The op type *is* the timestamped value;
/// state and op merge identically by `max`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LwwCell<V> {
    current: Option<Lww<V>>,
}

// Manual impl: an empty register needs no `V: Default` (crdts' `Map`
// requires `Val: Default` to materialize entries).
impl<V> Default for LwwCell<V> {
    fn default() -> Self {
        Self { current: None }
    }
}

impl<V: Ord> LwwCell<V> {
    /// An empty register.
    pub fn new() -> Self {
        Self { current: None }
    }

    /// Build the op for writing `value` at `timestamp`. Pure — apply it
    /// (locally and remotely) via [`CmRDT::apply`].
    pub fn write(&self, timestamp: SharedTimestamp, value: V) -> Lww<V> {
        Lww::new(timestamp, value)
    }

    /// The current winner, if ever written.
    pub fn read(&self) -> Option<&Lww<V>> {
        self.current.as_ref()
    }

    /// The current winning value, if ever written.
    pub fn value(&self) -> Option<&V> {
        self.current.as_ref().map(|lww| &lww.value)
    }

    /// The current winner's timestamp, if ever written. Feeds the
    /// Lamport floor for stamp generation (see the module docs).
    pub fn timestamp(&self) -> Option<SharedTimestamp> {
        self.current.as_ref().map(|lww| lww.timestamp)
    }
}

impl<V: Ord> CmRDT for LwwCell<V> {
    type Op = Lww<V>;
    type Validation = Infallible;

    fn validate_op(&self, _op: &Self::Op) -> Result<(), Self::Validation> {
        Ok(())
    }

    fn apply(&mut self, op: Self::Op) {
        if self.current.as_ref().is_none_or(|current| *current < op) {
            self.current = Some(op);
        }
    }
}

impl<V: Ord> CvRDT for LwwCell<V> {
    type Validation = Infallible;

    fn validate_merge(&self, _other: &Self) -> Result<(), Self::Validation> {
        Ok(())
    }

    fn merge(&mut self, other: Self) {
        if let Some(theirs) = other.current {
            self.apply(theirs);
        }
    }
}

/// LWW data is never causally retracted: DessPlay uses no `Map::rm`
/// (removal is tombstone values purged at compaction), so reset-remove
/// has nothing to do. This is what makes the register immune to the
/// clock-trimming bugs described in the module docs.
impl<V, A: Ord> ResetRemove<A> for LwwCell<V> {
    fn reset_remove(&mut self, _clock: &VClock<A>) {}
}

/// Resolve a register to its winner. `None` if never written.
pub fn resolve<V: Ord + Clone>(cell: &LwwCell<V>) -> Option<Lww<V>> {
    cell.read().cloned()
}

/// Resolve to the winning value, dropping the timestamp.
pub fn resolve_value<V: Ord + Clone>(cell: &LwwCell<V>) -> Option<V> {
    cell.value().cloned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    #[test]
    fn apply_keeps_the_maximum_in_any_order() {
        let ops = [
            Lww::new(ts(3), "c"),
            Lww::new(ts(1), "a"),
            Lww::new(ts(2), "b"),
        ];
        let mut forward = LwwCell::new();
        let mut backward = LwwCell::new();
        for op in &ops {
            forward.apply(op.clone());
        }
        for op in ops.iter().rev() {
            backward.apply(op.clone());
        }
        assert_eq!(forward, backward);
        assert_eq!(forward.value(), Some(&"c"));
    }

    #[test]
    fn equal_timestamps_tiebreak_on_value() {
        let mut cell = LwwCell::new();
        cell.apply(Lww::new(ts(5), "alpha"));
        cell.apply(Lww::new(ts(5), "zeta"));
        cell.apply(Lww::new(ts(5), "beta"));
        assert_eq!(cell.value(), Some(&"zeta"));
    }

    #[test]
    fn merge_equals_apply_and_is_idempotent() {
        let mut a = LwwCell::new();
        a.apply(Lww::new(ts(1), 10));
        let mut b = LwwCell::new();
        b.apply(Lww::new(ts(2), 5));

        let mut merged = a.clone();
        merged.merge(b.clone());
        assert_eq!(merged.value(), Some(&5));
        merged.merge(b);
        merged.merge(a);
        assert_eq!(merged.value(), Some(&5));
    }
}
