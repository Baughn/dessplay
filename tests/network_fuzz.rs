mod common;

use std::sync::Arc;

use rand::SeedableRng;
use rand::rngs::StdRng;
use tokio::time::Duration;

use dessplay::network::clock::ClockSyncService;
use dessplay::network::sync::SyncEngine;
use dessplay::network::PeerId;
use dessplay::state::SharedState;

use common::convergence::assert_converged;
use common::simulated_network::{LinkConfig, SimulatedNetwork};
use common::workload::run_random_workload;

fn peer(name: &str) -> PeerId {
    PeerId(name.to_string())
}

/// Run a fuzz test with random workload and random network conditions.
async fn fuzz_run(seed: u64) {
    let mut rng = StdRng::seed_from_u64(seed);
    let network = SimulatedNetwork::new(seed);
    let names = ["alice", "bob", "charlie"];
    let mut engines: Vec<Arc<SyncEngine>> = Vec::new();

    for name in &names {
        let peer_id = peer(name);
        let conn = Arc::new(network.add_peer(peer_id.clone()));
        let clock_svc = ClockSyncService::new(conn, Duration::from_millis(500));
        clock_svc.start();
        let (shared_state, _rx) = SharedState::new();
        let shared_state = Arc::new(shared_state);
        let engine = SyncEngine::new(clock_svc, peer_id, shared_state);
        engine.start();
        engines.push(engine);
    }

    // Apply random link configs that change over time
    let workload_duration = Duration::from_secs(5);
    let config_change_interval = Duration::from_millis(500);

    // Spawn workload tasks for each peer
    let mut workload_handles = Vec::new();
    for engine in &engines {
        let engine = Arc::clone(engine);
        let mut peer_rng = StdRng::seed_from_u64(seed.wrapping_add(
            engine.shared_state().view().peers[0].0.len() as u64,
        ));
        workload_handles.push(tokio::spawn(async move {
            run_random_workload(&engine, &mut peer_rng, workload_duration).await;
        }));
    }

    // Periodically change network conditions
    let start = tokio::time::Instant::now();
    while tokio::time::Instant::now() - start < workload_duration {
        // Random link config
        use rand::Rng;
        let config = LinkConfig {
            latency_ms: rng.random_range(0..100),
            jitter_ms: rng.random_range(0..50),
            loss_rate: rng.random_range(0.0..0.3),
            reorder_rate: rng.random_range(0.0..0.2),
        };

        for a in &names {
            for b in &names {
                if a != b {
                    network.set_link_symmetric(&peer(a), &peer(b), config.clone());
                }
            }
        }

        // Random partitions (10% chance per interval)
        let partition_roll: f64 = rng.random();
        if partition_roll < 0.1 {
            let a = names[rng.random_range(0..names.len())];
            let b = names[rng.random_range(0..names.len())];
            if a != b {
                network.partition(&peer(a), &peer(b));
                network.partition(&peer(b), &peer(a));
            }
        }

        // Heal all partitions periodically
        let heal_roll: f64 = rng.random();
        if heal_roll < 0.2 {
            for a in &names {
                for b in &names {
                    if a != b {
                        network.heal(&peer(a), &peer(b));
                    }
                }
            }
        }

        tokio::time::sleep(config_change_interval).await;
    }

    // Wait for all workloads to finish
    for handle in workload_handles {
        let _ = handle.await;
    }

    // Heal all partitions and set clean links for convergence
    for a in &names {
        for b in &names {
            if a != b {
                network.heal(&peer(a), &peer(b));
                network.set_link_symmetric(
                    &peer(a),
                    &peer(b),
                    LinkConfig {
                        latency_ms: 5,
                        ..Default::default()
                    },
                );
            }
        }
    }

    // Wait for convergence
    tokio::time::sleep(Duration::from_secs(10)).await;

    assert_converged(&engines, 1.0);
}

#[tokio::test(start_paused = true)]
#[ignore]
async fn fuzz_seed_1() {
    fuzz_run(1).await;
}

#[tokio::test(start_paused = true)]
#[ignore]
async fn fuzz_seed_2() {
    fuzz_run(2).await;
}

#[tokio::test(start_paused = true)]
#[ignore]
async fn fuzz_seed_3() {
    fuzz_run(3).await;
}

#[tokio::test(start_paused = true)]
#[ignore]
async fn fuzz_seed_42() {
    fuzz_run(42).await;
}

#[tokio::test(start_paused = true)]
#[ignore]
async fn fuzz_seed_1337() {
    fuzz_run(1337).await;
}
