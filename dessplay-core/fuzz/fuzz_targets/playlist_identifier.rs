//! Playlist `Identifier` ordering: after any script the playlist is
//! strictly sorted, and rebalancing preserves order while staying
//! convergent.

#![no_main]

use dessplay_core::test_support::{ScriptStep, run_script};
use dessplay_core::types::{ActorId, SharedTimestamp};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|steps: Vec<ScriptStep>| {
    let (mut state, _) = run_script(&steps);

    let entries = state.playlist_entries();
    for pair in entries.windows(2) {
        let key_a = (&pair[0].state.position, pair[0].hash);
        let key_b = (&pair[1].state.position, pair[1].hash);
        assert!(key_a < key_b, "playlist not strictly sorted");
    }

    let before: Vec<_> = entries.iter().map(|e| e.hash).collect();
    let mut replica = state.clone();
    let ops = state.rebalance_playlist(ActorId::SERVER, SharedTimestamp(u64::MAX));
    let after: Vec<_> = state.playlist_entries().iter().map(|e| e.hash).collect();
    assert_eq!(before, after, "rebalance reordered the playlist");

    for op in ops {
        replica.apply(op);
    }
    assert_eq!(replica, state, "rebalance ops did not converge");
});
