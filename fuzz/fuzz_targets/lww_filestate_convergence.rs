#![no_main]

use arbitrary::Arbitrary;
use dessplay_core::crdt::lww::LwwRegister;
use dessplay_core::types::FileState;
use libfuzzer_sys::fuzz_target;

/// Constrained input: small key/timestamp space forces same-key same-timestamp
/// tiebreaks. Arbitrary FileState generates NaN/Inf/subnormal progress values,
/// which exposes PartialOrd convergence failures.
#[derive(Arbitrary, Debug)]
struct Input {
    /// Each op: (key mod 4, timestamp mod 4, FileState value)
    ops: Vec<(u8, u8, FileState)>,
}

fuzz_target!(|input: Input| {
    if input.ops.is_empty() {
        return;
    }

    // Apply in original order
    let mut reg_a: LwwRegister<u8, FileState> = LwwRegister::new();
    for (key, ts, val) in &input.ops {
        reg_a.write(key % 4, u64::from(*ts % 4), val.clone());
    }

    // Apply in reverse order
    let mut reg_b: LwwRegister<u8, FileState> = LwwRegister::new();
    for (key, ts, val) in input.ops.iter().rev() {
        reg_b.write(key % 4, u64::from(*ts % 4), val.clone());
    }

    // CRDT convergence: same ops in any order must produce identical state
    assert_eq!(reg_a, reg_b);
});
