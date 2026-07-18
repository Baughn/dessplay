//! Invariants of `merge_known_offline` (design.md #15 + Presence): real
//! peer entries are never altered or shadowed, synthesis only happens
//! within the gating horizon, and every synthesized entry is a Departed
//! interactive peer — so a committed known-offline user gates exactly
//! like a Departed one, and nobody gates forever.

use dessplay_core::derive::{KNOWN_OFFLINE_GATING_HORIZON_MILLIS, merge_known_offline};
use dessplay_core::net::{KnownUser, PeerInfo, Presence, Role};
use dessplay_core::types::UserId;
use proptest::collection::vec;
use proptest::prelude::*;

/// A small name pool so peers and known_offline collide often.
fn arb_name() -> impl Strategy<Value = String> {
    prop::sample::select(vec!["kim", "baughn", "nero", "dagger", "quickshot", "saya"])
        .prop_map(str::to_owned)
}

fn arb_peer() -> impl Strategy<Value = PeerInfo> {
    (
        arb_name(),
        prop::bool::ANY,
        prop::sample::select(vec![Presence::Present, Presence::Lost, Presence::Departed]),
        0u64..u64::MAX / 2,
    )
        .prop_map(|(name, seeder, presence, connected_since)| PeerInfo {
            username: UserId::new(&name),
            role: if seeder {
                Role::Seeder
            } else {
                Role::Interactive
            },
            presence,
            addresses: vec![],
            connected_since,
        })
}

fn arb_known() -> impl Strategy<Value = KnownUser> {
    (arb_name(), 0u64..u64::MAX / 2).prop_map(|(name, last_seen)| KnownUser {
        username: UserId::new(&name),
        last_seen,
    })
}

proptest! {
    #[test]
    fn merge_invariants(
        peers in vec(arb_peer(), 0..8),
        known in vec(arb_known(), 0..8),
        now in 0u64..u64::MAX / 2,
    ) {
        let merged = merge_known_offline(&peers, &known, now);

        // Real entries are preserved verbatim, in order, never shadowed.
        prop_assert_eq!(&merged[..peers.len()], &peers[..]);

        // No username is ever synthesized on top of an existing entry
        // (real or already-synthesized).
        for (i, entry) in merged.iter().enumerate().skip(peers.len()) {
            prop_assert!(
                merged[..i].iter().all(|p| p.username != entry.username),
                "duplicate synthesized entry for {}",
                entry.username
            );
        }

        // Every synthesized entry is a Departed interactive peer for a
        // known user seen within the horizon.
        for entry in &merged[peers.len()..] {
            prop_assert_eq!(entry.presence, Presence::Departed);
            prop_assert_eq!(entry.role, Role::Interactive);
            prop_assert!(entry.addresses.is_empty());
            // `known` may list the same name twice (within and outside
            // the horizon) — synthesis needs at least one within it.
            let within_horizon = known.iter().any(|k| {
                k.username == entry.username
                    && now.saturating_sub(k.last_seen) <= KNOWN_OFFLINE_GATING_HORIZON_MILLIS
            });
            prop_assert!(within_horizon, "synthesized {} outside horizon", entry.username);
        }

        // Completeness: every known user within the horizon has some
        // entry in the merged list (their real one, or a synthesized one).
        for user in &known {
            if now.saturating_sub(user.last_seen) <= KNOWN_OFFLINE_GATING_HORIZON_MILLIS {
                prop_assert!(merged.iter().any(|p| p.username == user.username));
            }
        }
    }
}
