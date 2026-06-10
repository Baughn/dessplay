//! Wire robustness: arbitrary bytes fed to the postcard decoders must
//! error gracefully, never panic.

#![no_main]

use dessplay_core::{CrdtOp, StateSnapshot, wire};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = wire::decode::<CrdtOp>(data);
    let _ = wire::decode::<StateSnapshot>(data);
});
