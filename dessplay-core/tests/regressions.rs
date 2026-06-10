//! Pinned regressions: minimal counterexamples found by property
//! testing, kept verbatim so the bugs stay dead.

use dessplay_core::test_support::*;

/// Found by `cluster_converges` during Phase 3. With `crdts`'
/// `Map<_, MVReg<Lww<_>>>`, a CvRDT merge trimmed map-global put clocks
/// down to entry scope ("information the other side deleted"), which
/// destroyed the dominance between two sequential writes to the same
/// key; a later merge then resurrected the older write as "concurrent",
/// and LWW picked it on one replica but not the other. No removal was
/// involved anywhere. Fixed by replacing MVReg<Lww<V>> with the pure
/// max-merge `LwwCell<V>`, which carries no causal metadata to corrupt.
#[test]
fn merge_must_not_resurrect_dominated_writes() {
    use ClusterEvent::*;
    use ScriptOp::*;

    let events = vec![
        ServerOp {
            ts: 0,
            op: SetPosition { user: 4, millis: 0 },
        },
        Deliver {
            client: 4,
            count: 1,
        },
        ClientOp {
            client: 32,
            ts: 0,
            op: AddPlaylist {
                file: 0,
                after: None,
            },
        },
        ClientOp {
            client: 10,
            ts: 12,
            op: SetPosition {
                user: 87,
                millis: 0,
            },
        },
        ClientOp {
            client: 190,
            ts: 0,
            op: SetPosition {
                user: 59,
                millis: 0,
            },
        },
        ClientOp {
            client: 43,
            ts: 0,
            op: AddPlaylist {
                file: 0,
                after: None,
            },
        },
        Reconnect { client: 28 },
        ServerPoll { lane: 66 },
        Reconnect { client: 190 },
    ];

    let cluster = run_cluster(&events);
    let server_view = cluster.server.view();
    for (i, client) in cluster.clients.iter().enumerate() {
        assert_eq!(
            client.view(),
            server_view,
            "client {i} diverged from server"
        );
    }
}
