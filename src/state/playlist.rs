use crate::network::clock::SharedTimestamp;

use super::types::{PlaylistAction, PlaylistItem};

/// Extract a sort key from an action for deterministic tiebreaking.
fn action_sort_key(action: &PlaylistAction) -> (&str, u64) {
    match action {
        PlaylistAction::Add { id, .. } => (&id.user.0, id.seq),
        PlaylistAction::Remove { id } => (&id.user.0, id.seq),
        PlaylistAction::Move { id, .. } => (&id.user.0, id.seq),
    }
}

/// A playlist action with its shared clock timestamp, used for deterministic replay.
#[derive(Debug, Clone)]
pub struct TimestampedAction {
    pub action: PlaylistAction,
    pub timestamp: SharedTimestamp,
}

/// Deterministically replay playlist actions to produce the current playlist.
///
/// Actions are sorted by timestamp (stable sort preserves per-user ordering
/// for equal timestamps). Operations on nonexistent IDs are no-ops.
pub fn replay_playlist(actions: &[TimestampedAction]) -> Vec<PlaylistItem> {
    let mut sorted: Vec<&TimestampedAction> = actions.iter().collect();
    // Sort by timestamp, breaking ties by action's item ID for determinism.
    // This ensures all peers produce the same playlist regardless of insertion order.
    sorted.sort_by(|a, b| {
        a.timestamp.cmp(&b.timestamp).then_with(|| {
            let key_a = action_sort_key(&a.action);
            let key_b = action_sort_key(&b.action);
            key_a.cmp(&key_b)
        })
    });

    let mut items: Vec<PlaylistItem> = Vec::new();

    for entry in sorted {
        match &entry.action {
            PlaylistAction::Add {
                id,
                filename,
                after,
            } => {
                let item = PlaylistItem {
                    id: id.clone(),
                    filename: filename.clone(),
                };
                let pos = match after {
                    None => 0,
                    Some(after_id) => {
                        match items.iter().position(|i| i.id == *after_id) {
                            Some(idx) => idx + 1,
                            None => items.len(), // target not found, append
                        }
                    }
                };
                items.insert(pos, item);
            }
            PlaylistAction::Remove { id } => {
                items.retain(|i| i.id != *id);
            }
            PlaylistAction::Move { id, after } => {
                // Remove the item, then reinsert at the target position
                let removed = items.iter().position(|i| i.id == *id);
                if let Some(idx) = removed {
                    let item = items.remove(idx);
                    let pos = match after {
                        None => 0,
                        Some(after_id) => {
                            match items.iter().position(|i| i.id == *after_id) {
                                Some(idx) => idx + 1,
                                None => items.len(), // target not found, append
                            }
                        }
                    };
                    items.insert(pos, item);
                }
                // Move on nonexistent ID is a no-op
            }
        }
    }

    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::PeerId;
    use crate::state::types::ItemId;

    fn id(user: &str, seq: u64) -> ItemId {
        ItemId {
            user: PeerId(user.to_string()),
            seq,
        }
    }

    fn ts(us: i64) -> SharedTimestamp {
        SharedTimestamp(us)
    }

    #[test]
    fn empty_playlist() {
        assert!(replay_playlist(&[]).is_empty());
    }

    #[test]
    fn add_items() {
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "ep01.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 2),
                    filename: "ep02.mkv".to_string(),
                    after: Some(id("alice", 1)),
                },
                timestamp: ts(200),
            },
        ];
        let result = replay_playlist(&actions);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].filename, "ep01.mkv");
        assert_eq!(result[1].filename, "ep02.mkv");
    }

    #[test]
    fn add_at_beginning() {
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "ep01.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("bob", 1),
                    filename: "ep00.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(200),
            },
        ];
        let result = replay_playlist(&actions);
        assert_eq!(result[0].filename, "ep00.mkv");
        assert_eq!(result[1].filename, "ep01.mkv");
    }

    #[test]
    fn remove_item() {
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "ep01.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
            TimestampedAction {
                action: PlaylistAction::Remove {
                    id: id("alice", 1),
                },
                timestamp: ts(200),
            },
        ];
        assert!(replay_playlist(&actions).is_empty());
    }

    #[test]
    fn remove_nonexistent_is_noop() {
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "ep01.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
            TimestampedAction {
                action: PlaylistAction::Remove {
                    id: id("bob", 99),
                },
                timestamp: ts(200),
            },
        ];
        let result = replay_playlist(&actions);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn move_item() {
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "ep01.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 2),
                    filename: "ep02.mkv".to_string(),
                    after: Some(id("alice", 1)),
                },
                timestamp: ts(200),
            },
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 3),
                    filename: "ep03.mkv".to_string(),
                    after: Some(id("alice", 2)),
                },
                timestamp: ts(300),
            },
            // Move ep03 to beginning
            TimestampedAction {
                action: PlaylistAction::Move {
                    id: id("alice", 3),
                    after: None,
                },
                timestamp: ts(400),
            },
        ];
        let result = replay_playlist(&actions);
        assert_eq!(result[0].filename, "ep03.mkv");
        assert_eq!(result[1].filename, "ep01.mkv");
        assert_eq!(result[2].filename, "ep02.mkv");
    }

    #[test]
    fn move_nonexistent_is_noop() {
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "ep01.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
            TimestampedAction {
                action: PlaylistAction::Move {
                    id: id("bob", 99),
                    after: None,
                },
                timestamp: ts(200),
            },
        ];
        let result = replay_playlist(&actions);
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn concurrent_adds_from_different_users() {
        // Both adds survive — they have different IDs
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "alice_ep.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("bob", 1),
                    filename: "bob_ep.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100), // same timestamp, different user
            },
        ];
        let result = replay_playlist(&actions);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn timestamp_ordering() {
        // Actions replayed in timestamp order regardless of input order
        let actions = vec![
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 2),
                    filename: "ep02.mkv".to_string(),
                    after: Some(id("alice", 1)),
                },
                timestamp: ts(200),
            },
            TimestampedAction {
                action: PlaylistAction::Add {
                    id: id("alice", 1),
                    filename: "ep01.mkv".to_string(),
                    after: None,
                },
                timestamp: ts(100),
            },
        ];
        let result = replay_playlist(&actions);
        assert_eq!(result[0].filename, "ep01.mkv");
        assert_eq!(result[1].filename, "ep02.mkv");
    }
}
