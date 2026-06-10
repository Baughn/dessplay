//! Convergence through the hub-and-spoke topology: arbitrary client op
//! generation, server scheduling, partial deliveries, and reconnects —
//! after a flush, every replica resolves to the same view.

mod common;

use common::arb_cluster_event;
use dessplay_core::CrdtState;
use dessplay_core::test_support::run_cluster;
use proptest::collection::vec;
use proptest::prelude::*;

proptest! {
    /// All clients and the server agree after any event schedule.
    #[test]
    fn cluster_converges(events in vec(arb_cluster_event(), 1..80)) {
        let cluster = run_cluster(&events);
        let server_view = cluster.server.view();
        for (i, client) in cluster.clients.iter().enumerate() {
            prop_assert_eq!(
                client.view(),
                server_view.clone(),
                "client {} diverged from server",
                i
            );
        }
    }

    /// A fresh replica applying the server's total order reproduces the
    /// server's state exactly, and duplicate delivery (even out of order,
    /// once everything has been seen) changes nothing.
    #[test]
    fn log_replay_matches_server(events in vec(arb_cluster_event(), 1..60)) {
        let cluster = run_cluster(&events);

        let mut replica = CrdtState::new();
        for op in &cluster.log {
            replica.apply(op.clone());
        }
        prop_assert_eq!(&replica, &cluster.server);
        prop_assert_eq!(replica.view(), cluster.server.view());

        let before = replica.clone();
        for op in cluster.log.iter().rev() {
            replica.apply(op.clone());
        }
        prop_assert_eq!(&replica, &before);
    }
}
