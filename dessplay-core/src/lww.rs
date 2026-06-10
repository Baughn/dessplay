//! Last-writer-wins conflict resolution on top of `MVReg`.
//!
//! `MVReg` preserves all causally-concurrent values; we wrap stored values
//! in [`Lww`] and resolve reads by taking the maximum, giving deterministic
//! convergence: highest shared-clock timestamp wins, with value-based
//! tiebreaking when timestamps are equal.

use crdts::MVReg;
use serde::{Deserialize, Serialize};

use crate::types::{ActorId, SharedTimestamp};

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

/// A multi-value register holding LWW-wrapped values, keyed by our actor
/// type. This is the standard register shape throughout DessPlay.
pub type LwwReg<V> = MVReg<Lww<V>, ActorId>;

/// Resolve the concurrent values of a register to the LWW winner.
/// `None` if the register has never been written (or was reset-removed).
pub fn resolve<V: Ord + Clone>(reg: &LwwReg<V>) -> Option<Lww<V>> {
    reg.read().val.into_iter().max()
}

/// Resolve to the winning value, dropping the timestamp.
pub fn resolve_value<V: Ord + Clone>(reg: &LwwReg<V>) -> Option<V> {
    resolve(reg).map(|lww| lww.value)
}
