//! The core convergence property, through the hub-and-spoke cluster
//! model: after a flush, every client's resolved view equals the
//! server's.

#![no_main]

use dessplay_core::test_support::{ClusterEvent, run_cluster};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|events: Vec<ClusterEvent>| {
    let cluster = run_cluster(&events);
    let server_view = cluster.server.view();
    for client in &cluster.clients {
        assert_eq!(client.view(), server_view, "client diverged from server");
    }
});
