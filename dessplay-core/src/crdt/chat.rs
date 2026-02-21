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

    /// Append a message. Returns true if it was new (not a duplicate seq).
    pub fn append(
        &mut self,
        user_id: UserId,
        seq: u64,
        timestamp: SharedTimestamp,
        text: String,
    ) -> bool {
        let log = self.logs.entry(user_id).or_default();

        // Check for duplicate sequence number
        if log.iter().any(|entry| entry.seq == seq) {
            return false;
        }

        // Insert maintaining seq order within this user's log
        let pos = log.partition_point(|entry| entry.seq < seq);
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

    /// Highest contiguous seq starting from 0. Returns `None` if the user has
    /// no entries or their first entry isn't seq 0 (need everything resent).
    pub fn version(&self, user_id: &UserId) -> Option<u64> {
        let log = self.logs.get(user_id)?;
        // Entries are sorted by seq (maintained by partition_point in append).
        // Walk from the start to find the contiguous prefix.
        for (i, entry) in log.iter().enumerate() {
            if entry.seq != i as u64 {
                return if i == 0 { None } else { Some(i as u64 - 1) };
            }
        }
        // All entries are contiguous 0..len
        if log.is_empty() {
            None
        } else {
            Some(log.len() as u64 - 1)
        }
    }

    /// All entries for a user after the given seq number.
    /// Pass `None` to get all entries (remote knows nothing about this user).
    pub fn entries_since(&self, user_id: &UserId, since_seq: Option<u64>) -> Vec<ChatEntry> {
        self.logs
            .get(user_id)
            .map(|log| {
                log.iter()
                    .filter(|e| match since_seq {
                        Some(seq) => e.seq > seq,
                        None => true,
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
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
    fn duplicate_seq_ignored() {
        let mut chat = Chat::new();
        assert!(chat.append(uid("alice"), 0, 100, "first".into()));
        assert!(!chat.append(uid("alice"), 0, 200, "duplicate".into()));
        let view = chat.merged_view();
        assert_eq!(view.len(), 1);
        assert_eq!(view[0].1.text, "first");
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
        assert_eq!(chat.version(&uid("alice")), Some(0));
        chat.append(uid("alice"), 1, 200, "b".into());
        assert_eq!(chat.version(&uid("alice")), Some(1));
    }

    #[test]
    fn entries_since() {
        let mut chat = Chat::new();
        chat.append(uid("alice"), 0, 100, "a".into());
        chat.append(uid("alice"), 1, 200, "b".into());
        chat.append(uid("alice"), 2, 300, "c".into());

        let since = chat.entries_since(&uid("alice"), Some(0));
        assert_eq!(since.len(), 2);
        assert_eq!(since[0].seq, 1);
        assert_eq!(since[1].seq, 2);

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
        assert_eq!(chat.version(&uid("alice")), Some(2));
        let entries = chat.entries_since(&uid("alice"), Some(u64::MAX));
        assert!(entries.is_empty());
    }

    // --- Regression test for fuzz-discovered bug ---

    #[test]
    fn test_version_contiguous_prefix() {
        let mut chat = Chat::new();

        // [0, 1, 5] → contiguous prefix is 0,1 → version = Some(1)
        chat.append(uid("alice"), 0, 100, "a".into());
        chat.append(uid("alice"), 1, 200, "b".into());
        chat.append(uid("alice"), 5, 600, "f".into());
        assert_eq!(chat.version(&uid("alice")), Some(1));

        // [3, 5] → no contiguous prefix from 0 → version = None
        let mut chat2 = Chat::new();
        chat2.append(uid("bob"), 3, 400, "d".into());
        chat2.append(uid("bob"), 5, 600, "f".into());
        assert_eq!(chat2.version(&uid("bob")), None);

        // [0, 1, 2] → all contiguous → version = Some(2)
        let mut chat3 = Chat::new();
        chat3.append(uid("carol"), 0, 100, "a".into());
        chat3.append(uid("carol"), 1, 200, "b".into());
        chat3.append(uid("carol"), 2, 300, "c".into());
        assert_eq!(chat3.version(&uid("carol")), Some(2));

        // Empty → None
        let chat4 = Chat::new();
        assert_eq!(chat4.version(&uid("dave")), None);
    }
}
