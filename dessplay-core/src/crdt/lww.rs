use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::SharedTimestamp;

/// A generic Last-Writer-Wins Register map.
///
/// Each key maps to a timestamped value. A write only succeeds if the
/// timestamp is strictly greater than the current timestamp for that key.
/// Equal timestamps are broken by comparing the serialized value bytes
/// (higher wins), ensuring deterministic convergence.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LwwRegister<K: Ord, V> {
    entries: BTreeMap<K, (SharedTimestamp, V)>,
}

impl<K: Ord, V> Default for LwwRegister<K, V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord, V> LwwRegister<K, V> {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Write a value if the timestamp is newer than the existing one.
    /// On equal timestamp, the higher value (by `PartialOrd`) wins for
    /// deterministic convergence regardless of application order.
    /// Returns true if the write was applied.
    pub fn write(&mut self, key: K, timestamp: SharedTimestamp, value: V) -> bool
    where
        K: Clone,
        V: PartialOrd,
    {
        let dominated = match self.entries.get(&key) {
            Some((existing_ts, existing_val)) => {
                timestamp > *existing_ts
                    || (timestamp == *existing_ts
                        && value.partial_cmp(existing_val) == Some(std::cmp::Ordering::Greater))
            }
            None => true,
        };
        if dominated {
            self.entries.insert(key, (timestamp, value));
        }
        dominated
    }

    /// Read the current value for a key.
    pub fn read(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|(_, v)| v)
    }

    /// Get the timestamp for a key's current value.
    pub fn version(&self, key: &K) -> Option<SharedTimestamp> {
        self.entries.get(key).map(|(ts, _)| *ts)
    }

    /// Iterate over all entries.
    pub fn iter(&self) -> impl Iterator<Item = (&K, &(SharedTimestamp, V))> {
        self.entries.iter()
    }

    /// Get the underlying BTreeMap (for snapshot serialization).
    pub fn into_inner(self) -> BTreeMap<K, (SharedTimestamp, V)> {
        self.entries
    }

    /// Construct from a BTreeMap (for snapshot deserialization).
    pub fn from_inner(entries: BTreeMap<K, (SharedTimestamp, V)>) -> Self {
        Self { entries }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn newer_wins() {
        let mut reg = LwwRegister::new();
        assert!(reg.write("key", 10, "old"));
        assert!(reg.write("key", 20, "new"));
        assert_eq!(reg.read(&"key"), Some(&"new"));
    }

    #[test]
    fn older_ignored() {
        let mut reg = LwwRegister::new();
        assert!(reg.write("key", 20, "new"));
        assert!(!reg.write("key", 10, "old"));
        assert_eq!(reg.read(&"key"), Some(&"new"));
    }

    #[test]
    fn equal_timestamp_higher_value_wins() {
        let mut reg = LwwRegister::new();
        assert!(reg.write("key", 10, "first"));
        // "second" > "first" lexicographically, so it wins the tiebreak
        assert!(reg.write("key", 10, "second"));
        assert_eq!(reg.read(&"key"), Some(&"second"));
    }

    #[test]
    fn equal_timestamp_lower_value_ignored() {
        let mut reg = LwwRegister::new();
        assert!(reg.write("key", 10, "second"));
        // "first" < "second" lexicographically, so it loses the tiebreak
        assert!(!reg.write("key", 10, "first"));
        assert_eq!(reg.read(&"key"), Some(&"second"));
    }

    #[test]
    fn read_missing() {
        let reg: LwwRegister<&str, &str> = LwwRegister::new();
        assert_eq!(reg.read(&"nope"), None);
    }

    #[test]
    fn version_tracking() {
        let mut reg = LwwRegister::new();
        assert_eq!(reg.version(&"key"), None);
        reg.write("key", 42, "val");
        assert_eq!(reg.version(&"key"), Some(42));
    }

    #[test]
    fn multiple_keys() {
        let mut reg = LwwRegister::new();
        reg.write("a", 1, 10);
        reg.write("b", 2, 20);
        assert_eq!(reg.read(&"a"), Some(&10));
        assert_eq!(reg.read(&"b"), Some(&20));
    }
}
