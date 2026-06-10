//! CvRDT merge laws: commutative, associative, idempotent — and merge
//! agrees with op replay.
//!
//! Each replica gets its own actor (forced per script) because a CRDT
//! actor must not fork: two replicas generating ops as the same actor is
//! a protocol violation, not something merge has to survive.

mod common;

use common::arb_step;
use dessplay_core::CrdtState;
use dessplay_core::test_support::{ScriptStep, run_script};
use proptest::collection::vec;
use proptest::prelude::*;

/// Force every step in a script onto one actor (one replica = one actor).
fn pin_actor(steps: Vec<ScriptStep>, actor: u8) -> Vec<ScriptStep> {
    steps
        .into_iter()
        .map(|mut step| {
            step.actor = actor;
            step
        })
        .collect()
}

fn merged(a: &CrdtState, b: &CrdtState) -> CrdtState {
    let mut out = a.clone();
    out.merge(b.clone());
    out
}

proptest! {
    #[test]
    fn merge_laws(
        steps_a in vec(arb_step(), 0..25),
        steps_b in vec(arb_step(), 0..25),
        steps_c in vec(arb_step(), 0..25),
    ) {
        let (a, ops_a) = run_script(&pin_actor(steps_a, 1));
        let (b, ops_b) = run_script(&pin_actor(steps_b, 2));
        let (c, _) = run_script(&pin_actor(steps_c, 3));

        // Commutativity.
        prop_assert_eq!(merged(&a, &b), merged(&b, &a));

        // Associativity.
        let ab_then_c = merged(&merged(&a, &b), &c);
        let a_then_bc = merged(&a, &merged(&b, &c));
        prop_assert_eq!(&ab_then_c, &a_then_bc);
        prop_assert_eq!(ab_then_c.view(), a_then_bc.view());

        // Idempotence.
        prop_assert_eq!(merged(&a, &a), a.clone());

        // Merge agrees with op replay: a replica that saw both replicas'
        // ops (in per-actor order) equals the merge of their states.
        let mut replayed = CrdtState::new();
        for (_, op) in ops_a.iter().chain(ops_b.iter()) {
            replayed.apply(op.clone());
        }
        let state_merged = merged(&a, &b);
        prop_assert_eq!(&replayed, &state_merged);
        prop_assert_eq!(replayed.view(), state_merged.view());
    }

    /// Merging a snapshot into a replica that already has overlapping ops
    /// (the reconnection path) is safe and converges.
    #[test]
    fn partial_overlap_merge_converges(
        steps_a in vec(arb_step(), 1..25),
        steps_b in vec(arb_step(), 1..25),
        split in any::<prop::sample::Index>(),
    ) {
        let (a, ops_a) = run_script(&pin_actor(steps_a, 1));
        let (b, _) = run_script(&pin_actor(steps_b, 2));

        // A reconnecting replica holds a prefix of a's history plus all
        // of b, then receives a's full state.
        let mut reconnecting = b.clone();
        let prefix_len = split.index(ops_a.len() + 1);
        for (_, op) in ops_a.iter().take(prefix_len) {
            reconnecting.apply(op.clone());
        }
        reconnecting.merge(a.clone());

        prop_assert_eq!(&reconnecting, &merged(&b, &a));
        prop_assert_eq!(reconnecting.view(), merged(&a, &b).view());
    }
}
