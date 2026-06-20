//! Property tests for compaction's rebuild: for any history, rebuilding
//! from the resolved view preserves the view exactly — modulo the
//! documented reductions (chat trim, lookup-set clear, playlist
//! position rebalance, dropped watched-flags for off-playlist files).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use common::arb_step;
use dessplay_core::compact::rebuild;
use dessplay_core::test_support::run_script;
use dessplay_core::types::{ActorId, SharedTimestamp};
use dessplay_core::{CrdtState, StateView};
use proptest::prelude::*;

const CHAT_KEEP: usize = 4;

/// Monotonic stamps starting above everything a script can issue
/// (script timestamps are 0..32) — the Lamport floor rule.
fn stamper() -> impl FnMut() -> SharedTimestamp {
    let mut next = 1_000u64;
    move || {
        next += 1;
        SharedTimestamp(next)
    }
}

/// The view a rebuild is *supposed* to produce.
fn expected_view(mut view: StateView) -> StateView {
    let tail = view.chat.len().saturating_sub(CHAT_KEEP);
    view.chat.drain(..tail);
    view.lookup_requests.clear();
    view.acknowledged_absent.clear();
    view.watched
        .retain(|hash, _| view.playlist.iter().any(|entry| entry.hash == *hash));
    view
}

/// Compare views ignoring playlist positions (rebalanced by design):
/// entry order, hashes, and metadata must survive.
fn assert_views_match(rebuilt: &StateView, expected: &StateView) {
    let strip = |view: &StateView| {
        let mut view = view.clone();
        for entry in &mut view.playlist {
            entry.state.position = crdts::Identifier::between(None, None, ActorId::SERVER);
        }
        view
    };
    assert_eq!(strip(rebuilt), strip(expected));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Rebuild preserves the resolved view (modulo reductions).
    #[test]
    fn rebuild_preserves_the_view(steps in proptest::collection::vec(arb_step(), 0..60)) {
        let (state, _) = run_script(&steps);
        let view = state.view();
        let rebuilt = rebuild(&view, ActorId::SERVER, CHAT_KEEP, stamper());
        assert_views_match(&rebuilt.view(), &expected_view(view));
    }

    /// Rebuild is idempotent: compacting a compacted state is a no-op
    /// at the view level.
    #[test]
    fn rebuild_is_idempotent(steps in proptest::collection::vec(arb_step(), 0..40)) {
        let (state, _) = run_script(&steps);
        let mut stamp = stamper();
        let once = rebuild(&state.view(), ActorId::SERVER, CHAT_KEEP, &mut stamp);
        let twice = rebuild(&once.view(), ActorId::SERVER, CHAT_KEEP, &mut stamp);
        assert_views_match(&twice.view(), &once.view());
    }

    /// The rebuilt state's map clocks know only the rebuilding actor —
    /// the session-actor collapse that keeps clocks bounded.
    #[test]
    fn rebuild_collapses_actors(steps in proptest::collection::vec(arb_step(), 0..40)) {
        let (state, _) = run_script(&steps);
        let rebuilt = rebuild(&state.view(), ActorId::SERVER, CHAT_KEEP, stamper());
        // Serialize and deserialize must round-trip (sanity), and a
        // fresh client merging the rebuilt state must agree with it.
        let bytes = dessplay_core::wire::encode(&rebuilt).unwrap();
        let mut fresh: CrdtState = dessplay_core::wire::decode(&bytes).unwrap();
        fresh.merge(rebuilt.clone());
        assert_eq!(fresh.view(), rebuilt.view());
        // Every dot in the playlist map (the busiest map) is the
        // server's.
        let ctx = rebuilt.playlist.read_ctx();
        for dot in ctx.add_clock.iter() {
            assert_eq!(*dot.actor, ActorId::SERVER);
        }
    }
}
