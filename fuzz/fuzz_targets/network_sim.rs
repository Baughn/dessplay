#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use std::time::Duration;

use dessplay_core::network::simulated::{LinkConfig, SimulatedNetwork};
use dessplay_core::network::Network;
use dessplay_core::protocol::{CrdtOp, LwwValue, PeerControl, PeerDatagram};
use dessplay_core::types::{UserState, UserId};

/// Actions that can be performed on a SimulatedNetwork.
#[derive(Arbitrary, Debug)]
enum NetworkAction {
    SendControl { from: u8, to: u8, msg_seed: u8 },
    SendDatagram { from: u8, to: u8, msg_seed: u8 },
    SetLoss { from: u8, to: u8, loss_percent: u8 },
    Partition { a: u8, b: u8 },
    Heal { a: u8, b: u8 },
    DrainEvents { peer: u8 },
}

#[derive(Arbitrary, Debug)]
struct NetworkFuzzInput {
    seed: u64,
    peer_count: u8,
    actions: Vec<NetworkAction>,
}

fuzz_target!(|input: NetworkFuzzInput| {
    // Limit peer count to 2-4 for meaningful tests
    let peer_count = 2 + (input.peer_count % 3) as usize; // 2, 3, or 4

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build();
    let Ok(rt) = rt else { return };

    rt.block_on(async {
        let net = SimulatedNetwork::new(input.seed);

        // Add peers
        let mut handles = Vec::new();
        for i in 0..peer_count {
            handles.push(net.add_peer(&format!("peer{i}")));
        }

        // Drain initial connect events
        for handle in &handles {
            for _ in 0..peer_count {
                let _ = tokio::time::timeout(
                    Duration::from_millis(10),
                    handle.recv(),
                ).await;
            }
        }

        // Execute actions
        for action in &input.actions {
            match action {
                NetworkAction::SendControl { from, to, msg_seed } => {
                    let from_idx = *from as usize % peer_count;
                    let to_idx = *to as usize % peer_count;
                    if from_idx == to_idx {
                        continue;
                    }
                    let to_peer_id = handles[to_idx].peer_id();
                    let msg = PeerControl::StateOp {
                        op: CrdtOp::LwwWrite {
                            timestamp: *msg_seed as u64 + 1,
                            value: LwwValue::UserState(
                                UserId(format!("peer{from_idx}")),
                                UserState::Ready,
                            ),
                        },
                    };
                    let _ = handles[from_idx].send_control(to_peer_id, &msg).await;
                }
                NetworkAction::SendDatagram { from, to, msg_seed } => {
                    let from_idx = *from as usize % peer_count;
                    let to_idx = *to as usize % peer_count;
                    if from_idx == to_idx {
                        continue;
                    }
                    let to_peer_id = handles[to_idx].peer_id();
                    let msg = PeerDatagram::Position {
                        timestamp: *msg_seed as u64 + 1,
                        position_secs: *msg_seed as f64,
                    };
                    let _ = handles[from_idx].send_datagram(to_peer_id, &msg).await;
                }
                NetworkAction::SetLoss { from, to, loss_percent } => {
                    let from_idx = *from as usize % peer_count;
                    let to_idx = *to as usize % peer_count;
                    if from_idx == to_idx {
                        continue;
                    }
                    let loss = (*loss_percent as f64) / 100.0;
                    net.set_link(
                        handles[from_idx].peer_id(),
                        handles[to_idx].peer_id(),
                        LinkConfig {
                            packet_loss: loss.min(1.0),
                            ..Default::default()
                        },
                    );
                }
                NetworkAction::Partition { a, b } => {
                    let a_idx = *a as usize % peer_count;
                    let b_idx = *b as usize % peer_count;
                    if a_idx == b_idx {
                        continue;
                    }
                    net.partition(handles[a_idx].peer_id(), handles[b_idx].peer_id());
                }
                NetworkAction::Heal { a, b } => {
                    let a_idx = *a as usize % peer_count;
                    let b_idx = *b as usize % peer_count;
                    if a_idx == b_idx {
                        continue;
                    }
                    net.heal(handles[a_idx].peer_id(), handles[b_idx].peer_id());
                }
                NetworkAction::DrainEvents { peer } => {
                    let idx = *peer as usize % peer_count;
                    // Drain up to 100 events with a short timeout
                    for _ in 0..100 {
                        if tokio::time::timeout(
                            Duration::from_millis(1),
                            handles[idx].recv(),
                        ).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }
    });
});
