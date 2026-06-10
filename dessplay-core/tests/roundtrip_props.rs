//! Wire round-trips: every generated op and every reachable state
//! snapshot survives postcard encode/decode unchanged.

mod common;

use common::arb_step;
use dessplay_core::test_support::run_script;
use dessplay_core::types::Epoch;
use dessplay_core::{CrdtOp, StateSnapshot, wire};
use proptest::collection::vec;
use proptest::prelude::*;

proptest! {
    #[test]
    fn ops_round_trip(steps in vec(arb_step(), 1..40)) {
        let (_, ops) = run_script(&steps);
        for (_, op) in ops {
            let bytes = wire::encode(&op)
                .map_err(|e| TestCaseError::fail(format!("encode failed: {e}")))?;
            let decoded: CrdtOp = wire::decode(&bytes)
                .map_err(|e| TestCaseError::fail(format!("decode failed: {e}")))?;
            prop_assert_eq!(decoded, op);
        }
    }

    #[test]
    fn snapshots_round_trip(steps in vec(arb_step(), 0..40), epoch in any::<u64>()) {
        let (state, _) = run_script(&steps);
        let snapshot = StateSnapshot {
            epoch: Epoch(epoch),
            state,
        };
        let bytes = wire::encode(&snapshot)
            .map_err(|e| TestCaseError::fail(format!("encode failed: {e}")))?;
        let decoded: StateSnapshot = wire::decode(&bytes)
            .map_err(|e| TestCaseError::fail(format!("decode failed: {e}")))?;
        prop_assert_eq!(&decoded, &snapshot);
        prop_assert_eq!(decoded.state.view(), snapshot.state.view());

        // Restoration: a fresh replica adopting the snapshot behaves
        // identically (stale-epoch reconnect path).
        let mut adopted = decoded.state;
        adopted.merge(snapshot.state.clone());
        prop_assert_eq!(adopted, snapshot.state);
    }
}
