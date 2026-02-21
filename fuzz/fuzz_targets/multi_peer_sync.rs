#![no_main]

use arbitrary::Arbitrary;
use dessplay_core::crdt::CrdtState;
use dessplay_core::protocol::CrdtOp;
use libfuzzer_sys::fuzz_target;

/// Simulates 3 peers receiving random subsets of operations, then performing
/// multiple rounds of version-vector-based sync via ops_since. Exercises the
/// complete sync protocol: partial delivery, gap detection, catch-up, and
/// convergence.
#[derive(Arbitrary, Debug)]
struct Input {
    ops: Vec<CrdtOp>,
    /// Bitmask per op: which of 3 peers receives it (bits 0-2)
    delivery: Vec<u8>,
}

fuzz_target!(|input: Input| {
    if input.ops.is_empty() {
        return;
    }

    let mut peers = [CrdtState::new(), CrdtState::new(), CrdtState::new()];

    // Phase 1: partial delivery — each peer gets a random subset
    for (i, op) in input.ops.iter().enumerate() {
        let mask = input.delivery.get(i).copied().unwrap_or(0xFF);
        for (j, peer) in peers.iter_mut().enumerate() {
            if mask & (1 << j) != 0 {
                peer.apply_op(op);
            }
        }
    }

    // Phase 2: full sync — multiple rounds to handle transitive gaps
    for _ in 0..3 {
        let vvs: Vec<_> = peers.iter().map(|p| p.version_vectors()).collect();
        // Collect all catch-up ops before applying (avoid borrow issues)
        let mut catch_ups: Vec<(usize, Vec<CrdtOp>)> = Vec::new();

        for i in 0..3 {
            for j in 0..3 {
                if i == j {
                    continue;
                }
                let ops = peers[j].ops_since(&vvs[i]);
                if !ops.is_empty() {
                    catch_ups.push((i, ops));
                }
            }
        }

        for (target, ops) in &catch_ups {
            for op in ops {
                peers[*target].apply_op(op);
            }
        }
    }

    // After full sync, all peers must have identical snapshots
    let snap0 = peers[0].snapshot();
    let snap1 = peers[1].snapshot();
    let snap2 = peers[2].snapshot();
    assert_eq!(snap0, snap1);
    assert_eq!(snap1, snap2);
});
