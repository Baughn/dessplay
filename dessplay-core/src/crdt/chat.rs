use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::protocol::ChatEntry;
use crate::types::{SharedTimestamp, UserId};

/// Per-user append-only chat log.
///
/// Each user's messages are sequenced independently with a monotonic
/// sequence number. There are no conflicts — each user exclusively
/// appends to their own log.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chat {
    logs: BTreeMap<UserId, Vec<ChatEntry>>,
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

impl Chat {
    pub fn new() -> Self {
        Self {
            logs: BTreeMap::new(),
        }
    }

    /// Append a message. Returns true if the operation changed state.
    ///
    /// On duplicate `(user_id, seq)`: applies LWW conflict resolution —
    /// higher timestamp wins; on equal timestamp, lexicographically greater
    /// text wins. This ensures convergence regardless of application order.
    pub fn append(
        &mut self,
        user_id: UserId,
        seq: u64,
        timestamp: SharedTimestamp,
        text: String,
    ) -> bool {
        let log = self.logs.entry(user_id).or_default();

        // Check for existing entry with same seq
        let pos = log.partition_point(|entry| entry.seq < seq);
        if pos < log.len() && log[pos].seq == seq {
            let existing = &log[pos];
            if timestamp > existing.timestamp
                || (timestamp == existing.timestamp && text > existing.text)
            {
                log[pos] = ChatEntry {
                    seq,
                    timestamp,
                    text,
                };
                return true;
            }
            return false;
        }

        // New seq — insert maintaining order
        log.insert(
            pos,
            ChatEntry {
                seq,
                timestamp,
                text,
            },
        );
        true
    }

    /// All messages from all users, sorted by timestamp (then by user for ties).
    pub fn merged_view(&self) -> Vec<(&UserId, &ChatEntry)> {
        let mut all: Vec<(&UserId, &ChatEntry)> = self
            .logs
            .iter()
            .flat_map(|(uid, entries)| entries.iter().map(move |e| (uid, e)))
            .collect();
        all.sort_by(|a, b| {
            a.1.timestamp
                .cmp(&b.1.timestamp)
                .then_with(|| a.0.cmp(b.0))
        });
        all
    }

    /// Deterministic hash of a user's chat log. Used for version vectors.
    ///
    /// Returns `None` if the user has no entries. When hashes differ between
    /// peers, all entries are exchanged and the receiver applies LWW per-seq.
    pub fn version(&self, user_id: &UserId) -> Option<u64> {
        let log = self.logs.get(user_id)?;
        if log.is_empty() {
            return None;
        }
        // FNV-1a style hash over entry count + each (seq, timestamp, text bytes)
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(log.len() as u64);
        for entry in log {
            h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(entry.seq);
            h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(entry.timestamp);
            for &b in entry.text.as_bytes() {
                h = h.wrapping_mul(0x0100_0000_01b3).wrapping_add(b as u64);
            }
        }
        Some(h)
    }

    /// All entries for a user that the remote may be missing.
    ///
    /// `remote_version` is the remote's `version()` hash for this user.
    /// Pass `None` to get all entries (remote knows nothing about this user).
    /// If the hash matches, returns empty (already in sync).
    pub fn entries_since(&self, user_id: &UserId, remote_version: Option<u64>) -> Vec<ChatEntry> {
        let Some(log) = self.logs.get(user_id) else {
            return Vec::new();
        };
        match remote_version {
            Some(rv) if Some(rv) == self.version(user_id) => Vec::new(),
            _ => log.clone(),
        }
    }

    /// Get the raw logs (for snapshot serialization).
    pub fn into_inner(self) -> BTreeMap<UserId, Vec<ChatEntry>> {
        self.logs
    }

    /// Construct from snapshot data.
    pub fn from_inner(logs: BTreeMap<UserId, Vec<ChatEntry>>) -> Self {
        Self { logs }
    }

    /// All user IDs that have messages.
    pub fn users(&self) -> impl Iterator<Item = &UserId> {
        self.logs.keys()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn uid(name: &str) -> UserId {
        UserId(name.to_string())
    }

    #[test]
    fn append_and_read() {
        let mut chat = Chat::new();
        assert!(chat.append(uid("alice"), 0, 100, "hello".into()));
        let view = chat.merged_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].0, &uid("alice"));
        assert_eq!(view[0].1.text, "hello");
    }

    #[test]
    fn duplicate_seq_lww_higher_timestamp_wins() {
        let mut chat = Chat::new();
        assert!(chat.append(uid("alice"), 0, 100, "first".into()));
        // Higher timestamp wins via LWW
        assert!(chat.append(uid("alice"), 0, 200, "updated".into()));
        let view = chat.merged_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].1.text, "updated");
    }

    #[test]
    fn duplicate_seq_lww_lower_timestamp_rejected() {
        let mut chat = Chat::new();
        assert!(chat.append(uid("alice"), 0, 200, "winner".into()));
        // Lower timestamp loses
        assert!(!chat.append(uid("alice"), 0, 100, "loser".into()));
        let view = chat.merged_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].1.text, "winner");
    }

    #[test]
    fn merged_view_sorted_by_timestamp() {
        let mut chat = Chat::new();
        chat.append(uid("bob"), 0, 200, "second".into());
        chat.append(uid("alice"), 0, 100, "first".into());
        chat.append(uid("alice"), 1, 300, "third".into());
        let view = chat.merged_view();
        assert_eq!(view.len(), 3);
        assert_eq!(view[0].1.text, "first");
        assert_eq!(view[1].1.text, "second");
        assert_eq!(view[2].1.text, "third");
    }

    #[test]
    fn same_timestamp_sorted_by_user() {
        let mut chat = Chat::new();
        chat.append(uid("bob"), 0, 100, "bob".into());
        chat.append(uid("alice"), 0, 100, "alice".into());
        let view = chat.merged_view();
        assert_eq!(view[0].0, &uid("alice"));
        assert_eq!(view[1].0, &uid("bob"));
    }

    #[test]
    fn version_tracking() {
        let mut chat = Chat::new();
        assert_eq!(chat.version(&uid("alice")), None);
        chat.append(uid("alice"), 0, 100, "a".into());
        let v1 = chat.version(&uid("alice"));
        assert!(v1.is_some());
        chat.append(uid("alice"), 1, 200, "b".into());
        let v2 = chat.version(&uid("alice"));
        assert!(v2.is_some());
        assert_ne!(v1, v2, "version must change when entries are added");
    }

    #[test]
    fn entries_since() {
        let mut chat = Chat::new();
        chat.append(uid("alice"), 0, 100, "a".into());
        chat.append(uid("alice"), 1, 200, "b".into());
        chat.append(uid("alice"), 2, 300, "c".into());

        // Matching hash returns empty (already in sync)
        let my_version = chat.version(&uid("alice"));
        let synced = chat.entries_since(&uid("alice"), my_version);
        assert!(synced.is_empty(), "matching version means in sync");

        // Mismatched hash returns all entries
        let since = chat.entries_since(&uid("alice"), Some(999));
        assert_eq!(since.len(), 3);

        // None means "need everything"
        let all = chat.entries_since(&uid("alice"), None);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn out_of_order_seq_inserted_correctly() {
        let mut chat = Chat::new();
        chat.append(uid("alice"), 2, 300, "c".into());
        chat.append(uid("alice"), 0, 100, "a".into());
        chat.append(uid("alice"), 1, 200, "b".into());
        assert!(chat.version(&uid("alice")).is_some());

        // entries_since with matching version returns empty
        let v = chat.version(&uid("alice"));
        let entries = chat.entries_since(&uid("alice"), v);
        assert!(entries.is_empty());
    }

    // --- Regression tests for fuzz-discovered bugs ---

    #[test]
    fn duplicate_seq_different_content_converges() {
        // Two peers receive the same (user, seq) with different content.
        // Regardless of application order, result must be identical.
        let mut chat_a = Chat::new();
        chat_a.append(uid("alice"), 0, 100, "first".into());
        chat_a.append(uid("alice"), 0, 200, "second".into());

        let mut chat_b = Chat::new();
        chat_b.append(uid("alice"), 0, 200, "second".into());
        chat_b.append(uid("alice"), 0, 100, "first".into());

        assert_eq!(chat_a, chat_b);

        // Same timestamp, different text — should also converge
        let mut chat_c = Chat::new();
        chat_c.append(uid("alice"), 0, 100, "aaa".into());
        chat_c.append(uid("alice"), 0, 100, "zzz".into());

        let mut chat_d = Chat::new();
        chat_d.append(uid("alice"), 0, 100, "zzz".into());
        chat_d.append(uid("alice"), 0, 100, "aaa".into());

        assert_eq!(chat_c, chat_d);
    }

    #[test]
    fn version_is_hash_of_content() {
        // Same entries → same version hash
        let mut chat_a = Chat::new();
        chat_a.append(uid("alice"), 0, 100, "a".into());
        chat_a.append(uid("alice"), 1, 200, "b".into());

        let mut chat_b = Chat::new();
        chat_b.append(uid("alice"), 1, 200, "b".into());
        chat_b.append(uid("alice"), 0, 100, "a".into());

        assert_eq!(chat_a.version(&uid("alice")), chat_b.version(&uid("alice")));

        // Different entries → different version hash
        let mut chat_c = Chat::new();
        chat_c.append(uid("alice"), 0, 100, "different".into());
        assert_ne!(chat_a.version(&uid("alice")), chat_c.version(&uid("alice")));

        // Empty → None
        let chat_d = Chat::new();
        assert_eq!(chat_d.version(&uid("dave")), None);

        // Non-contiguous seqs still produce a version
        let mut chat_e = Chat::new();
        chat_e.append(uid("bob"), 3, 400, "d".into());
        chat_e.append(uid("bob"), 5, 600, "f".into());
        assert!(chat_e.version(&uid("bob")).is_some());
    }
}
