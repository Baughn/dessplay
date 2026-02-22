#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;

use dessplay_core::protocol::{CrdtOp, PeerControl};
use dessplay_core::sync_engine::{SyncAction, SyncEngine};
use dessplay_core::types::PeerId;

/// Events that the fuzzer can generate.
#[derive(Arbitrary, Debug)]
enum FuzzEvent {
    /// Apply a local CRDT operation on a peer.
    LocalOp { peer: u8, op: CrdtOp },
    /// Trigger a peer connecting to another.
    PeerConnect { from: u8, to: u8 },
    /// Trigger a peer disconnecting from another.
    PeerDisconnect { from: u8, to: u8 },
    /// Periodic tick on a peer (broadcasts StateSummary).
    PeriodicTick { peer: u8 },
    /// Send a state summary from one peer to another (anti-entropy).
    ExchangeSummary { from: u8, to: u8 },
    /// Send a full snapshot from one peer to another (simulates epoch upgrade).
    SendSnapshot { from: u8, to: u8 },
    /// Toggle a network partition between two peers.
    TogglePartition { a: u8, b: u8 },
    /// Set packet loss rate for a link direction.
    SetLoss { from: u8, to: u8, loss_percent: u8 },
    /// Drain all pending actions for a peer (simulates processing backlog).
    DrainActions { peer: u8 },
    /// Pause: stop mutating state and assert convergence within connected components.
    AssertConvergence,
}

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    seed: u64,
    peer_count: u8,
    events: Vec<FuzzEvent>,
}

/// Simple seeded RNG for deterministic fuzzing.
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.wrapping_add(1),
        }
    }

    fn next_u64(&mut self) -> u64 {
        // xorshift64
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    /// Returns true with the given probability (0.0 = never, 1.0 = always).
    fn should_drop(&mut self, loss: f64) -> bool {
        (self.next_u64() as f64 / u64::MAX as f64) < loss
    }
}

fn is_link_partitioned(
    from: usize,
    to: usize,
    partitioned: &HashMap<(usize, usize), bool>,
) -> bool {
    let key = (from.min(to), from.max(to));
    partitioned.get(&key).copied().unwrap_or(false)
}

/// Find connected components via union-find given current partition state.
fn connected_components(
    peer_count: usize,
    partitioned: &HashMap<(usize, usize), bool>,
) -> Vec<Vec<usize>> {
    let mut parent: Vec<usize> = (0..peer_count).collect();

    fn find(parent: &mut Vec<usize>, x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }

    fn union(parent: &mut Vec<usize>, a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }

    for a in 0..peer_count {
        for b in (a + 1)..peer_count {
            if !is_link_partitioned(a, b, partitioned) {
                union(&mut parent, a, b);
            }
        }
    }

    let mut components: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..peer_count {
        components.entry(find(&mut parent, i)).or_default().push(i);
    }
    components.into_values().collect()
}

/// Run reliable (no-loss) summary exchanges within a group until converged,
/// then assert all members have identical snapshots.
fn sync_and_assert_component(
    component: &[usize],
    engines: &mut Vec<SyncEngine>,
    pending: &mut Vec<Vec<SyncAction>>,
) {
    if component.len() < 2 {
        return;
    }

    // No loss for convergence sync — these links are up, we just need enough rounds.
    let no_loss: HashMap<(usize, usize), f64> = HashMap::new();
    let no_partitions: HashMap<(usize, usize), bool> = HashMap::new();
    // Dummy RNG that never drops (loss is 0).
    let mut rng = SimpleRng::new(0);

    for _ in 0..10 {
        // Exchange summaries between all pairs in the component
        for &from_idx in component {
            for &to_idx in component {
                if from_idx == to_idx {
                    continue;
                }
                let vv = engines[from_idx].version_vectors();
                let epoch = engines[from_idx].epoch();
                let actions =
                    engines[to_idx].on_state_summary(PeerId(from_idx as u64), epoch, vv);
                dispatch_actions_inner(
                    actions,
                    to_idx,
                    engines,
                    pending,
                    &no_loss,
                    &no_partitions,
                    &mut rng,
                );
            }
        }

        // Drain all pending within the component
        for &idx in component {
            for _ in 0..20 {
                if pending[idx].is_empty() {
                    break;
                }
                let actions = std::mem::take(&mut pending[idx]);
                dispatch_actions_inner(
                    actions,
                    idx,
                    engines,
                    pending,
                    &no_loss,
                    &no_partitions,
                    &mut rng,
                );
            }
        }
    }

    // Assert convergence within the component
    let snap_first = engines[component[0]].state().snapshot();
    for &idx in &component[1..] {
        let snap_i = engines[idx].state().snapshot();
        assert_eq!(
            snap_first, snap_i,
            "Engine {} and engine {} diverged within connected component {:?}",
            component[0], idx, component
        );
    }
}

/// Dispatch actions from a source peer to the network (free function so it
/// can be called from both the main loop and sync_and_assert_component).
fn dispatch_actions_inner(
    actions: Vec<SyncAction>,
    from_idx: usize,
    engines: &mut Vec<SyncEngine>,
    pending: &mut Vec<Vec<SyncAction>>,
    loss_rates: &HashMap<(usize, usize), f64>,
    partitioned: &HashMap<(usize, usize), bool>,
    rng: &mut SimpleRng,
) {
    for action in actions {
        match action {
            SyncAction::SendControl { peer, msg } => {
                let to_idx = peer.0 as usize;
                if to_idx >= engines.len() || to_idx == from_idx {
                    continue;
                }
                if is_link_partitioned(from_idx, to_idx, partitioned) {
                    continue;
                }
                let loss = loss_rates.get(&(from_idx, to_idx)).copied().unwrap_or(0.0);
                if rng.should_drop(loss) {
                    continue;
                }
                let new_actions = match msg {
                    PeerControl::StateOp { op } => {
                        engines[to_idx].on_remote_op(PeerId(from_idx as u64), op)
                    }
                    PeerControl::StateSummary { epoch, versions } => engines[to_idx]
                        .on_state_summary(PeerId(from_idx as u64), epoch, versions),
                    PeerControl::StateSnapshot { epoch, crdts } => {
                        engines[to_idx].on_state_snapshot(epoch, crdts)
                    }
                    _ => vec![],
                };
                pending[to_idx].extend(new_actions);
            }
            SyncAction::BroadcastControl { msg } => {
                for to_idx in 0..engines.len() {
                    if to_idx == from_idx {
                        continue;
                    }
                    if is_link_partitioned(from_idx, to_idx, partitioned) {
                        continue;
                    }
                    let loss =
                        loss_rates.get(&(from_idx, to_idx)).copied().unwrap_or(0.0);
                    if rng.should_drop(loss) {
                        continue;
                    }
                    let new_actions = match &msg {
                        PeerControl::StateOp { op } => engines[to_idx]
                            .on_remote_op(PeerId(from_idx as u64), op.clone()),
                        PeerControl::StateSummary { epoch, versions } => engines[to_idx]
                            .on_state_summary(
                                PeerId(from_idx as u64),
                                *epoch,
                                versions.clone(),
                            ),
                        PeerControl::StateSnapshot { epoch, crdts } => {
                            engines[to_idx].on_state_snapshot(*epoch, crdts.clone())
                        }
                        _ => vec![],
                    };
                    pending[to_idx].extend(new_actions);
                }
            }
            SyncAction::RequestGapFill { peer, request } => {
                let to_idx = peer.0 as usize;
                if to_idx >= engines.len() || to_idx == from_idx {
                    continue;
                }
                if is_link_partitioned(from_idx, to_idx, partitioned) {
                    continue;
                }
                let loss =
                    loss_rates.get(&(from_idx, to_idx)).copied().unwrap_or(0.0);
                if rng.should_drop(loss) {
                    continue;
                }
                let response = engines[to_idx].on_gap_fill_request(&request);
                let new_actions =
                    engines[from_idx].on_gap_fill_response(PeerId(to_idx as u64), response);
                pending[from_idx].extend(new_actions);
            }
            SyncAction::PersistOp { .. }
            | SyncAction::PersistSnapshot { .. }
            | SyncAction::SendDatagram { .. }
            | SyncAction::BroadcastDatagram { .. } => {}
        }
    }
}

fuzz_target!(|input: FuzzInput| {
    let peer_count = 2 + (input.peer_count % 3) as usize; // 2, 3, or 4
    let mut rng = SimpleRng::new(input.seed);

    let mut engines: Vec<SyncEngine> = (0..peer_count).map(|_| SyncEngine::new()).collect();

    let mut loss_rates: HashMap<(usize, usize), f64> = HashMap::new();
    let mut partitioned: HashMap<(usize, usize), bool> = HashMap::new();
    let mut pending: Vec<Vec<SyncAction>> = vec![Vec::new(); peer_count];

    for event in &input.events {
        match event {
            FuzzEvent::LocalOp { peer, op } => {
                let idx = *peer as usize % peer_count;
                let actions = engines[idx].apply_local_op(op.clone());
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut engines,
                    &mut pending,
                    &loss_rates,
                    &partitioned,
                    &mut rng,
                );
            }
            FuzzEvent::PeerConnect { from, to } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx == to_idx {
                    continue;
                }
                let actions =
                    engines[from_idx].on_peer_connected(PeerId(to_idx as u64));
                dispatch_actions_inner(
                    actions,
                    from_idx,
                    &mut engines,
                    &mut pending,
                    &loss_rates,
                    &partitioned,
                    &mut rng,
                );
            }
            FuzzEvent::PeerDisconnect { from, to } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx != to_idx {
                    engines[from_idx].on_peer_disconnected(PeerId(to_idx as u64));
                }
            }
            FuzzEvent::PeriodicTick { peer } => {
                let idx = *peer as usize % peer_count;
                let actions = engines[idx].on_periodic_tick();
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut engines,
                    &mut pending,
                    &loss_rates,
                    &partitioned,
                    &mut rng,
                );
            }
            FuzzEvent::ExchangeSummary { from, to } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx == to_idx {
                    continue;
                }
                if is_link_partitioned(from_idx, to_idx, &partitioned) {
                    continue;
                }
                let vv = engines[from_idx].version_vectors();
                let epoch = engines[from_idx].epoch();
                let actions =
                    engines[to_idx].on_state_summary(PeerId(from_idx as u64), epoch, vv);
                dispatch_actions_inner(
                    actions,
                    to_idx,
                    &mut engines,
                    &mut pending,
                    &loss_rates,
                    &partitioned,
                    &mut rng,
                );
            }
            FuzzEvent::SendSnapshot { from, to } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx == to_idx {
                    continue;
                }
                if is_link_partitioned(from_idx, to_idx, &partitioned) {
                    continue;
                }
                let epoch = engines[from_idx].epoch();
                let snapshot = engines[from_idx].state().snapshot();
                let actions = engines[to_idx].on_state_snapshot(epoch, snapshot);
                dispatch_actions_inner(
                    actions,
                    to_idx,
                    &mut engines,
                    &mut pending,
                    &loss_rates,
                    &partitioned,
                    &mut rng,
                );
            }
            FuzzEvent::TogglePartition { a, b } => {
                let a_idx = *a as usize % peer_count;
                let b_idx = *b as usize % peer_count;
                if a_idx == b_idx {
                    continue;
                }
                let key = (a_idx.min(b_idx), a_idx.max(b_idx));
                let current = partitioned.get(&key).copied().unwrap_or(false);
                partitioned.insert(key, !current);
            }
            FuzzEvent::SetLoss { from, to, loss_percent } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx != to_idx {
                    let loss = (*loss_percent as f64 / 255.0).min(0.9);
                    loss_rates.insert((from_idx, to_idx), loss);
                }
            }
            FuzzEvent::DrainActions { peer } => {
                let idx = *peer as usize % peer_count;
                for _ in 0..100 {
                    if pending[idx].is_empty() {
                        break;
                    }
                    let actions = std::mem::take(&mut pending[idx]);
                    dispatch_actions_inner(
                        actions,
                        idx,
                        &mut engines,
                        &mut pending,
                        &loss_rates,
                        &partitioned,
                        &mut rng,
                    );
                }
            }
            FuzzEvent::AssertConvergence => {
                // Find connected components under current partition state
                // and assert convergence within each one (using reliable delivery).
                let components = connected_components(peer_count, &partitioned);
                for component in &components {
                    sync_and_assert_component(component, &mut engines, &mut pending);
                }
            }
        }
    }

    // Final: heal everything and assert global convergence
    partitioned.clear();
    loss_rates.clear();

    // Drain remaining pending
    for idx in 0..peer_count {
        for _ in 0..10 {
            if pending[idx].is_empty() {
                break;
            }
            let actions = std::mem::take(&mut pending[idx]);
            dispatch_actions_inner(
                actions,
                idx,
                &mut engines,
                &mut pending,
                &HashMap::new(),
                &HashMap::new(),
                &mut rng,
            );
        }
    }

    let all_peers: Vec<usize> = (0..peer_count).collect();
    sync_and_assert_component(&all_peers, &mut engines, &mut pending);
});
