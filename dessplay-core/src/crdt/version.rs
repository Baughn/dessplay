use crate::protocol::{GapFillRequest, RegisterId, VersionVectors};
use crate::types::{SharedTimestamp, UserId};

impl VersionVectors {
    /// Create empty version vectors for a new epoch.
    pub fn new(epoch: u64) -> Self {
        Self {
            epoch,
            lww_versions: Default::default(),
            chat_versions: Default::default(),
            playlist_version: 0,
        }
    }
}

/// Detect what the remote has that we are missing.
///
/// Returns a `GapFillRequest` describing the ops we need. Returns `None`
/// if we are fully up to date (or on a different epoch — epoch mismatches
/// are handled at a higher level).
pub fn detect_gaps(local: &VersionVectors, remote: &VersionVectors) -> Option<GapFillRequest> {
    if local.epoch != remote.epoch {
        // Epoch mismatch — caller must handle via full snapshot
        return None;
    }

    let mut lww_needed: Vec<(RegisterId, SharedTimestamp)> = Vec::new();
    let mut chat_needed: Vec<(UserId, u64)> = Vec::new();
    let mut playlist_after: Option<SharedTimestamp> = None;

    // Check LWW registers: anything the remote has newer or at the same
    // timestamp (equal timestamps may hide value differences from Ord tiebreaking)
    for (reg, remote_ts) in &remote.lww_versions {
        let local_ts = local.lww_versions.get(reg).copied().unwrap_or(0);
        if *remote_ts >= local_ts {
            lww_needed.push((reg.clone(), local_ts));
        }
    }

    // Check chat versions (hash-based: any difference means we need data)
    for (uid, remote_hash) in &remote.chat_versions {
        let local_hash = local.chat_versions.get(uid).copied().unwrap_or(0);
        if *remote_hash != local_hash {
            chat_needed.push((uid.clone(), local_hash));
        }
    }

    // Check playlist (hash-based: any difference means we need data)
    if remote.playlist_version != local.playlist_version {
        playlist_after = Some(local.playlist_version);
    }

    if lww_needed.is_empty() && chat_needed.is_empty() && playlist_after.is_none() {
        return None;
    }

    Some(GapFillRequest {
        lww_needed,
        chat_needed,
        playlist_after,
    })
}

/// Check if local state is at least as up-to-date as remote.
///
/// For LWW registers, "up to date" means local timestamp > remote timestamp
/// (equal timestamps may hide value differences from Ord tiebreaking).
/// For chat and playlist (hash-based versions), "up to date" means hashes match.
pub fn is_up_to_date(local: &VersionVectors, remote: &VersionVectors) -> bool {
    if local.epoch != remote.epoch {
        return false;
    }

    for (reg, remote_ts) in &remote.lww_versions {
        let local_ts = local.lww_versions.get(reg).copied().unwrap_or(0);
        if *remote_ts >= local_ts {
            return false;
        }
    }

    // Chat: hash-based — must match exactly
    for (uid, remote_hash) in &remote.chat_versions {
        let local_hash = local.chat_versions.get(uid).copied().unwrap_or(0);
        if *remote_hash != local_hash {
            return false;
        }
    }

    // Playlist: hash-based — must match exactly
    remote.playlist_version == local.playlist_version
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::types::FileId;

    fn uid(name: &str) -> UserId {
        UserId(name.to_string())
    }

    fn fid(n: u8) -> FileId {
        let mut id = [0u8; 16];
        id[0] = n;
        FileId(id)
    }

    #[test]
    fn no_gaps_when_identical() {
        let v = VersionVectors::new(1);
        assert!(detect_gaps(&v, &v).is_none());
        assert!(is_up_to_date(&v, &v));
    }

    #[test]
    fn epoch_mismatch_returns_none() {
        let local = VersionVectors::new(1);
        let remote = VersionVectors::new(2);
        assert!(detect_gaps(&local, &remote).is_none());
        assert!(!is_up_to_date(&local, &remote));
    }

    #[test]
    fn detects_lww_gap() {
        let local = VersionVectors::new(1);
        let mut remote = VersionVectors::new(1);
        let reg = RegisterId::UserState(uid("alice"));
        remote.lww_versions.insert(reg.clone(), 100);

        let gap = detect_gaps(&local, &remote).unwrap();
        assert_eq!(gap.lww_needed, vec![(reg, 0)]);
        assert!(gap.chat_needed.is_empty());
        assert_eq!(gap.playlist_after, None);
    }

    #[test]
    fn detects_chat_gap() {
        let local = VersionVectors::new(1);
        let mut remote = VersionVectors::new(1);
        remote.chat_versions.insert(uid("bob"), 5);

        let gap = detect_gaps(&local, &remote).unwrap();
        assert!(gap.lww_needed.is_empty());
        assert_eq!(gap.chat_needed, vec![(uid("bob"), 0)]);
    }

    #[test]
    fn detects_playlist_gap() {
        let local = VersionVectors::new(1);
        let mut remote = VersionVectors::new(1);
        remote.playlist_version = 42;

        let gap = detect_gaps(&local, &remote).unwrap();
        assert_eq!(gap.playlist_after, Some(0));
    }

    #[test]
    fn partial_gap() {
        let mut local = VersionVectors::new(1);
        local.lww_versions.insert(RegisterId::AniDb(fid(1)), 50);
        local.chat_versions.insert(uid("alice"), 3);

        let mut remote = VersionVectors::new(1);
        remote.lww_versions.insert(RegisterId::AniDb(fid(1)), 100);
        remote.chat_versions.insert(uid("alice"), 3); // Same — no gap

        let gap = detect_gaps(&local, &remote).unwrap();
        assert_eq!(gap.lww_needed.len(), 1);
        assert_eq!(gap.lww_needed[0].1, 50); // Our known timestamp
        assert!(gap.chat_needed.is_empty());
    }

    #[test]
    fn up_to_date_when_local_has_more() {
        let mut local = VersionVectors::new(1);
        local.lww_versions.insert(RegisterId::UserState(uid("alice")), 200);

        let mut remote = VersionVectors::new(1);
        remote.lww_versions.insert(RegisterId::UserState(uid("alice")), 100);

        assert!(is_up_to_date(&local, &remote));
    }

    #[test]
    fn not_up_to_date_when_remote_has_more() {
        let local = VersionVectors::new(1);
        let mut remote = VersionVectors::new(1);
        remote.playlist_version = 1;

        assert!(!is_up_to_date(&local, &remote));
    }

    // --- Regression tests for LWW tiebreak divergence (fuzz round 3) ---

    #[test]
    fn detect_gaps_lww_same_timestamp() {
        // Regression: detect_gaps used `>` instead of `>=` for LWW timestamps.
        // When local and remote have the same timestamp for a register, we cannot
        // be sure the values match (tiebreak may differ), so a gap must be reported.
        let mut local = VersionVectors::new(1);
        local
            .lww_versions
            .insert(RegisterId::UserState(uid("alice")), 100);

        let mut remote = VersionVectors::new(1);
        remote
            .lww_versions
            .insert(RegisterId::UserState(uid("alice")), 100);

        let gap = detect_gaps(&local, &remote);
        assert!(
            gap.is_some(),
            "detect_gaps must report a gap when LWW timestamps are equal \
             (values may differ due to tiebreak)",
        );
        let gap = gap.unwrap();
        assert_eq!(gap.lww_needed.len(), 1);
        assert_eq!(gap.lww_needed[0].0, RegisterId::UserState(uid("alice")));
    }

    #[test]
    fn is_up_to_date_lww_same_timestamp() {
        // Regression: is_up_to_date used `>` instead of `>=` for LWW timestamps.
        // Equal timestamps do not guarantee equal values (tiebreak may differ).
        let mut local = VersionVectors::new(1);
        local
            .lww_versions
            .insert(RegisterId::UserState(uid("alice")), 100);

        let mut remote = VersionVectors::new(1);
        remote
            .lww_versions
            .insert(RegisterId::UserState(uid("alice")), 100);

        assert!(
            !is_up_to_date(&local, &remote),
            "is_up_to_date must return false when LWW timestamps are equal \
             (cannot guarantee values match due to tiebreak)",
        );
    }
}
