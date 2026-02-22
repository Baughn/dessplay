#![no_main]

use arbitrary::Arbitrary;
use dessplay_core::crdt::CrdtState;
use dessplay_core::protocol::CrdtOp;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct ConvergenceInput {
    ops: Vec<CrdtOp>,
    /// Seed for the permutation.
    perm_seed: u64,
}

fn shuffle_with_seed<T>(items: &mut [T], seed: u64) {
    let len = items.len();
    let mut state = seed;
    for i in (1..len).rev() {
        // xorshift64
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let j = (state as usize) % (i + 1);
        items.swap(i, j);
    }
}

fuzz_target!(|input: ConvergenceInput| {
    if input.ops.is_empty() {
        return;
    }

    // Apply in original order
    let mut state_a = CrdtState::new();
    for op in &input.ops {
        state_a.apply_op(op);
    }

    // Apply in permuted order
    let mut shuffled = input.ops;
    shuffle_with_seed(&mut shuffled, input.perm_seed);

    let mut state_b = CrdtState::new();
    for op in &shuffled {
        state_b.apply_op(op);
    }

    // Snapshots must be identical regardless of application order
    assert_eq!(state_a.snapshot(), state_b.snapshot());

    // Version vectors must also agree (catches bugs where internal
    // representation diverges even though logical content matches)
    assert_eq!(state_a.version_vectors(), state_b.version_vectors());
});
