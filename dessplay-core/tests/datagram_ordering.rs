//! Coverage for the datagram fast-path gap detector
//! (`CrdtState::apply_if_orderly` / its inner `next_in_sequence`).
//!
//! Ordinary ops travel both the reliable control stream *and* a datagram;
//! the datagram copy can arrive early or out of order. For a `crdts::Map`
//! that is dangerous: its `Up` ops carry per-actor sequence dots, and
//! applying a later dot ahead of an earlier one silently masks the earlier
//! op (it is lost, not buffered). `apply_if_orderly` is the sole guard —
//! it applies a map op only when its dot is exactly the next in sequence
//! for that origin, and drops (to be retried via the reliable stream)
//! anything that would skip a gap. Register / GSet / GList ops are
//! order-free and always applied.
//!
//! The convergence harness applies every op through the reliable
//! `CrdtState::apply` path, so before these tests nothing exercised the
//! datagram-ordered branch. The three properties guarded here, per
//! docs/sync-state.md "Delivery requirements" / "Phase 4 constraint":
//!
//! - (a) **only in-sequence ops apply** — a map op whose dot skips an
//!   undelivered earlier same-origin op is held, then applies once the gap
//!   is filled;
//! - (b) **no divergence** — a replica fed a server log out of order
//!   through the datagram lane converges to the same resolved view as a
//!   replica fed the same log in reliable order;
//! - (c) **order-free ops** always apply and stay idempotent regardless of
//!   datagram ordering.

mod common;

use common::arb_cluster_event;
use dessplay_core::test_support::{ScriptOp, ScriptStep, deliver_via_datagram_lane, run_cluster};
use dessplay_core::types::{ActorId, Ed2kHash, SeriesWatchState, SharedTimestamp, UserId};
use dessplay_core::{CrdtOp, CrdtState};
use proptest::collection::vec;
use proptest::prelude::*;
use rand::SeedableRng;
use rand::rngs::StdRng;
use rand::seq::SliceRandom;

const ORIGIN: ActorId = ActorId(7);

fn ts(t: u64) -> SharedTimestamp {
    SharedTimestamp(t)
}

fn hash(i: u8) -> Ed2kHash {
    Ed2kHash([i; 16])
}

/// Build a clean single-origin sequence of `n` map ops on one map (the
/// watched flags). The i-th op carries per-origin dot `i + 1`, so the
/// gap check accepts them only in order.
fn watched_sequence(n: u8) -> (CrdtState, Vec<CrdtOp>) {
    let mut origin = CrdtState::new();
    let ops = (0..n)
        .map(|i| origin.set_watched(ORIGIN, ts(i as u64 + 1), hash(i), true))
        .collect();
    (origin, ops)
}

// ---------------------------------------------------------------------------
// (a) Only in-sequence ops apply.
// ---------------------------------------------------------------------------

#[test]
fn datagram_holds_an_out_of_sequence_map_op_until_the_gap_is_filled() {
    let (_origin, ops) = watched_sequence(3); // dots 1, 2, 3 on the watched map.
    let mut replica = CrdtState::new();

    // Deliver dot 2 first: it skips the undelivered dot 1, so it is held.
    assert!(
        !replica.apply_if_orderly(ops[1].clone()),
        "an op skipping an earlier same-origin dot must be held"
    );
    assert!(
        replica.view().watched.is_empty(),
        "a held op must not touch the resolved view"
    );

    // Deliver dot 3: still ahead of the gap, still held.
    assert!(!replica.apply_if_orderly(ops[2].clone()));
    assert!(replica.view().watched.is_empty());

    // Fill the gap with dot 1: it is next in sequence, so it applies.
    assert!(replica.apply_if_orderly(ops[0].clone()));
    assert_eq!(replica.view().watched.get(&hash(0)), Some(&true));
    assert!(!replica.view().watched.contains_key(&hash(1)));

    // Now the previously-held dot 2 is next in sequence and applies.
    assert!(replica.apply_if_orderly(ops[1].clone()));
    assert_eq!(replica.view().watched.get(&hash(1)), Some(&true));

    // And dot 3.
    assert!(replica.apply_if_orderly(ops[2].clone()));
    assert_eq!(replica.view().watched.len(), 3);
}

#[test]
fn datagram_redelivery_of_an_applied_map_op_is_a_no_op() {
    let (_origin, ops) = watched_sequence(2);
    let mut replica = CrdtState::new();

    assert!(replica.apply_if_orderly(ops[0].clone()));
    // Re-delivering an already-applied dot is not "next in sequence"
    // (clock is already at 1), so it is dropped rather than double-applied.
    assert!(!replica.apply_if_orderly(ops[0].clone()));
    assert_eq!(replica.view().watched.len(), 1);

    assert!(replica.apply_if_orderly(ops[1].clone()));
    assert!(!replica.apply_if_orderly(ops[1].clone()));
    assert_eq!(replica.view().watched.len(), 2);
}

proptest! {
    /// For any single-origin map-op sequence, delivering a strict suffix
    /// (the prefix withheld) applies nothing — every op is held because an
    /// earlier dot is missing. Once the prefix arrives in order, the whole
    /// sequence applies.
    #[test]
    fn datagram_holds_every_op_while_an_earlier_dot_is_missing(
        n in 2u8..9,
        gap in 1u8..8,
        seed in any::<u64>(),
    ) {
        let gap = (gap % (n - 1)) + 1; // 1..n: at least dot 1 is withheld.
        let (_origin, ops) = watched_sequence(n);
        let mut replica = CrdtState::new();

        // Offer the suffix [gap..] in a shuffled order: all are ahead of
        // the still-missing prefix, so none may apply.
        let mut suffix: Vec<usize> = (gap as usize..n as usize).collect();
        suffix.shuffle(&mut StdRng::seed_from_u64(seed));
        for &i in &suffix {
            prop_assert!(
                !replica.apply_if_orderly(ops[i].clone()),
                "op at dot {} applied while dot 1..{} were missing",
                i + 1,
                gap
            );
        }
        prop_assert!(replica.view().watched.is_empty());

        // Deliver the prefix in order, then re-offer the suffix in
        // sequence: now that the gap is filled the whole sequence drains.
        for op in ops.iter().take(gap as usize) {
            prop_assert!(replica.apply_if_orderly(op.clone()));
        }
        let mut sorted_suffix = suffix.clone();
        sorted_suffix.sort_unstable();
        for i in sorted_suffix {
            prop_assert!(replica.apply_if_orderly(ops[i].clone()));
        }
        prop_assert_eq!(replica.view().watched.len(), n as usize);
    }
}

// ---------------------------------------------------------------------------
// (b) No divergence: datagram-ordered delivery converges to the reliable view.
// ---------------------------------------------------------------------------

proptest! {
    /// A realistic server log (mixed map + order-free ops from several
    /// origins) delivered through the datagram lane in a shuffled order —
    /// with dropped early datagrams retried — converges to exactly the
    /// reliable in-order view.
    #[test]
    fn datagram_lane_converges_to_the_reliable_view(
        events in vec(arb_cluster_event(), 1..80),
        seed in any::<u64>(),
    ) {
        let cluster = run_cluster(&events);
        let log = &cluster.log;

        // Reliable replica: the server's total order, applied in order.
        let mut reliable = CrdtState::new();
        for op in log {
            reliable.apply(op.clone());
        }
        prop_assert_eq!(reliable.view(), cluster.server.view());

        // Datagram lane: the same log offered in a shuffled order.
        let mut order: Vec<usize> = (0..log.len()).collect();
        order.shuffle(&mut StdRng::seed_from_u64(seed));
        let outcome = deliver_via_datagram_lane(log, &order);

        prop_assert_eq!(
            outcome.undelivered, 0,
            "datagram lane stalled: an op never reached its in-sequence slot"
        );
        prop_assert_eq!(
            outcome.replica.view(),
            cluster.server.view(),
            "datagram-ordered replica diverged from the reliable view"
        );
    }
}

// ---------------------------------------------------------------------------
// (c) Order-free ops always apply and stay idempotent under any ordering.
// ---------------------------------------------------------------------------

/// A scripted op whose CRDT op is order-free (register / GSet / GList):
/// these bypass the gap check entirely.
fn arb_order_free_op() -> impl Strategy<Value = ScriptOp> {
    prop_oneof![
        proptest::option::of(any::<u8>()).prop_map(|file| ScriptOp::SetNowPlaying { file }),
        any::<u8>().prop_map(|authority| ScriptOp::SetSeekAuthority { authority }),
        any::<bool>().prop_map(|playing| ScriptOp::SetIntent { playing }),
        any::<u8>().prop_map(|file| ScriptOp::RequestLookup { file }),
        (any::<u8>(), any::<u8>())
            .prop_map(|(file, user)| ScriptOp::AcknowledgeAbsent { file, user }),
        any::<u8>().prop_map(|text| ScriptOp::Chat { text }),
    ]
}

fn arb_order_free_step() -> impl Strategy<Value = ScriptStep> {
    (any::<u8>(), 0u16..32, arb_order_free_op()).prop_map(|(actor, ts, op)| ScriptStep {
        actor,
        ts,
        op,
    })
}

#[test]
fn order_free_ops_apply_in_any_order_and_are_idempotent() {
    // Two competing now-playing writes and a chat line, hand-built so the
    // LWW winner (highest timestamp) is unambiguous.
    let mut origin = CrdtState::new();
    let np_early = origin.set_now_playing(ORIGIN, ts(5), Some(hash(1)));
    let np_late = origin.set_now_playing(ORIGIN, ts(9), Some(hash(2)));
    let chat = origin.append_chat(dessplay_core::types::ChatMessage {
        timestamp: ts(7),
        sender: UserId::new("a"),
        text: "hi".into(),
    });

    let mut replica = CrdtState::new();
    // Deliver out of order; every order-free op applies unconditionally.
    assert!(replica.apply_if_orderly(np_late.clone()));
    assert!(replica.apply_if_orderly(chat.clone()));
    assert!(replica.apply_if_orderly(np_early.clone()));

    // The later timestamp still wins regardless of arrival order, and
    // re-delivery (idempotent) changes nothing while still returning true.
    let view = replica.view();
    assert_eq!(view.now_playing, Some(hash(2)));
    assert_eq!(view.chat.len(), 1);
    for op in [np_late, chat, np_early] {
        assert!(replica.apply_if_orderly(op));
    }
    assert_eq!(replica.view(), view);
}

proptest! {
    /// A log of purely order-free ops: the datagram lane never holds one
    /// (held == 0), and the resulting view matches reliable in-order
    /// delivery. Re-delivering the whole log is idempotent.
    #[test]
    fn order_free_log_is_order_insensitive_and_idempotent(
        steps in vec(arb_order_free_step(), 1..40),
        seed in any::<u64>(),
    ) {
        let ops: Vec<CrdtOp> = {
            let mut state = CrdtState::new();
            steps
                .iter()
                .map(|step| dessplay_core::test_support::apply_step(&mut state, step).1)
                .collect()
        };

        let mut reliable = CrdtState::new();
        for op in &ops {
            reliable.apply(op.clone());
        }

        let mut order: Vec<usize> = (0..ops.len()).collect();
        order.shuffle(&mut StdRng::seed_from_u64(seed));
        let outcome = deliver_via_datagram_lane(&ops, &order);

        prop_assert_eq!(outcome.held, 0, "order-free op was held by the gap check");
        prop_assert_eq!(outcome.undelivered, 0);
        prop_assert_eq!(outcome.replica.view(), reliable.view());

        // Idempotent re-delivery: every op still applies, view unchanged.
        let mut replica = outcome.replica;
        let view = replica.view();
        for &i in &order {
            prop_assert!(replica.apply_if_orderly(ops[i].clone()));
        }
        prop_assert_eq!(replica.view(), view);
    }
}

// ---------------------------------------------------------------------------
// Cross-check: a Map-backed series-preference op really does go through the
// gap check (guards against a variant being misfiled into the order-free arm).
// ---------------------------------------------------------------------------

#[test]
fn map_backed_series_preference_is_gap_checked() {
    let mut origin = CrdtState::new();
    let user = UserId::new("kim");
    let s = dessplay_core::types::AniDbSeriesId(1);
    let s2 = dessplay_core::types::AniDbSeriesId(2);
    let op1 = origin.set_series_preference(
        ORIGIN,
        ts(1),
        user.clone(),
        s,
        SeriesWatchState::Watching,
        None,
    );
    let op2 = origin.set_series_preference(
        ORIGIN,
        ts(2),
        user.clone(),
        s2,
        SeriesWatchState::NotWatching,
        None,
    );

    let mut replica = CrdtState::new();
    // dot 2 ahead of dot 1 is held — proving SeriesPreference is gap-checked,
    // not treated as order-free.
    assert!(!replica.apply_if_orderly(op2.clone()));
    assert!(replica.view().series_preference.is_empty());
    assert!(replica.apply_if_orderly(op1));
    assert!(replica.apply_if_orderly(op2));
    assert_eq!(replica.view().series_preference.len(), 2);
}
