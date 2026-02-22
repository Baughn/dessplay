#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use std::collections::HashMap;

use dessplay::app_state::{AppEffect, AppEvent, AppState};
use dessplay_core::protocol::PeerControl;
use dessplay_core::sync_engine::SyncAction;
use dessplay_core::types::{FileId, FileState, PeerId, UserId, UserState};

// ---------------------------------------------------------------------------
// Fuzz input types
// ---------------------------------------------------------------------------

#[derive(Arbitrary, Debug)]
struct FuzzInput {
    seed: u64,
    peer_count: u8,
    events: Vec<FuzzEvent>,
}

#[derive(Arbitrary, Debug)]
enum FuzzEvent {
    // User actions (go through AppState op construction)
    SendChat { peer: u8, text_byte: u8 },
    SetUserState { peer: u8, state: u8 },
    SetFileState { peer: u8, file: u8, state: u8 },
    AddToPlaylist { peer: u8, file: u8, after: Option<u8> },
    RemoveFromPlaylist { peer: u8, file: u8 },
    MoveInPlaylist { peer: u8, file: u8, after: Option<u8> },

    // Network events
    PeerConnect { from: u8, to: u8 },
    PeerDisconnect { from: u8, to: u8 },
    Tick { peer: u8 },
    DrainActions { peer: u8 },

    // Network conditions
    TogglePartition { a: u8, b: u8 },
    SetLoss { from: u8, to: u8, loss_percent: u8 },

    // Assertion
    AssertConvergence,
}

// ---------------------------------------------------------------------------
// Helpers (copied from sync_engine.rs — no shared helper crate)
// ---------------------------------------------------------------------------

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
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

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

// ---------------------------------------------------------------------------
// Fuzz state
// ---------------------------------------------------------------------------

struct FuzzState {
    apps: Vec<AppState>,
    pending: Vec<Vec<SyncAction>>,
    loss_rates: HashMap<(usize, usize), f64>,
    partitioned: HashMap<(usize, usize), bool>,
    rng: SimpleRng,
    timestamp: u64,
    sent_chats: Vec<(usize, String)>,
}

fn make_file_id(n: u8) -> FileId {
    let mut id = [0u8; 16];
    id[0] = n;
    FileId(id)
}

fn make_user_id(idx: usize) -> UserId {
    UserId(format!("peer{idx}"))
}

fn make_user_state(n: u8) -> UserState {
    match n % 3 {
        0 => UserState::Ready,
        1 => UserState::Paused,
        _ => UserState::NotWatching,
    }
}

fn make_file_state(n: u8) -> FileState {
    match n % 3 {
        0 => FileState::Ready,
        1 => FileState::Missing,
        _ => FileState::Downloading {
            progress: (n as f32 / 255.0).clamp(0.0, 1.0),
        },
    }
}

// ---------------------------------------------------------------------------
// Action dispatch (adapted from sync_engine.rs for AppState layer)
// ---------------------------------------------------------------------------

fn extract_sync_actions(effects: Vec<AppEffect>) -> Vec<SyncAction> {
    let mut actions = Vec::new();
    for effect in effects {
        if let AppEffect::Sync(a) = effect {
            actions.extend(a);
        }
    }
    actions
}

fn dispatch_actions_inner(
    actions: Vec<SyncAction>,
    from_idx: usize,
    apps: &mut Vec<AppState>,
    pending: &mut Vec<Vec<SyncAction>>,
    loss_rates: &HashMap<(usize, usize), f64>,
    partitioned: &HashMap<(usize, usize), bool>,
    rng: &mut SimpleRng,
    timestamp: &mut u64,
) {
    for action in actions {
        match action {
            SyncAction::SendControl { peer, msg } => {
                let to_idx = peer.0 as usize;
                if to_idx >= apps.len() || to_idx == from_idx {
                    continue;
                }
                if is_link_partitioned(from_idx, to_idx, partitioned) {
                    continue;
                }
                let loss = loss_rates.get(&(from_idx, to_idx)).copied().unwrap_or(0.0);
                if rng.should_drop(loss) {
                    continue;
                }
                let new_actions = dispatch_peer_control(
                    &msg,
                    from_idx,
                    to_idx,
                    apps,
                    timestamp,
                );
                pending[to_idx].extend(new_actions);
            }
            SyncAction::BroadcastControl { msg } => {
                for to_idx in 0..apps.len() {
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
                    let new_actions = dispatch_peer_control(
                        &msg,
                        from_idx,
                        to_idx,
                        apps,
                        timestamp,
                    );
                    pending[to_idx].extend(new_actions);
                }
            }
            SyncAction::RequestGapFill { peer, request } => {
                let to_idx = peer.0 as usize;
                if to_idx >= apps.len() || to_idx == from_idx {
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
                let response = apps[to_idx].on_gap_fill_request(&request);
                *timestamp += 1;
                let effects = apps[from_idx].process_event(
                    AppEvent::GapFillResponse {
                        from: PeerId(to_idx as u64),
                        response,
                    },
                    *timestamp,
                );
                let new_actions = extract_sync_actions(effects);
                pending[from_idx].extend(new_actions);
            }
            SyncAction::PersistOp { .. }
            | SyncAction::PersistSnapshot { .. }
            | SyncAction::SendDatagram { .. }
            | SyncAction::BroadcastDatagram { .. } => {}
        }
    }
}

/// Route a PeerControl message through AppState, returning SyncActions.
fn dispatch_peer_control(
    msg: &PeerControl,
    from_idx: usize,
    to_idx: usize,
    apps: &mut Vec<AppState>,
    timestamp: &mut u64,
) -> Vec<SyncAction> {
    *timestamp += 1;
    let event = match msg {
        PeerControl::StateOp { op } => AppEvent::RemoteOp {
            from: PeerId(from_idx as u64),
            op: op.clone(),
        },
        PeerControl::StateSummary { epoch, versions } => AppEvent::StateSummary {
            from: PeerId(from_idx as u64),
            epoch: *epoch,
            versions: versions.clone(),
        },
        PeerControl::StateSnapshot { epoch, crdts } => AppEvent::StateSnapshot {
            epoch: *epoch,
            snapshot: crdts.clone(),
        },
        _ => return vec![],
    };
    let effects = apps[to_idx].process_event(event, *timestamp);
    extract_sync_actions(effects)
}

// ---------------------------------------------------------------------------
// Convergence assertion (adapted from sync_engine.rs)
// ---------------------------------------------------------------------------

fn sync_and_assert_component(
    component: &[usize],
    apps: &mut Vec<AppState>,
    pending: &mut Vec<Vec<SyncAction>>,
    sent_chats: &[(usize, String)],
) {
    if component.len() < 2 {
        return;
    }

    let no_loss: HashMap<(usize, usize), f64> = HashMap::new();
    let no_partitions: HashMap<(usize, usize), bool> = HashMap::new();
    let mut rng = SimpleRng::new(0);
    let mut ts = u64::MAX / 2; // high timestamp to avoid collisions with fuzz events

    for _ in 0..10 {
        // Exchange summaries between all pairs
        for &from_idx in component {
            for &to_idx in component {
                if from_idx == to_idx {
                    continue;
                }
                let vv = apps[from_idx].sync_engine.version_vectors();
                let epoch = apps[from_idx].sync_engine.epoch();
                ts += 1;
                let effects = apps[to_idx].process_event(
                    AppEvent::StateSummary {
                        from: PeerId(from_idx as u64),
                        epoch,
                        versions: vv,
                    },
                    ts,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    to_idx,
                    apps,
                    pending,
                    &no_loss,
                    &no_partitions,
                    &mut rng,
                    &mut ts,
                );
            }
        }

        // Drain pending within the component
        for &idx in component {
            for _ in 0..20 {
                if pending[idx].is_empty() {
                    break;
                }
                let actions = std::mem::take(&mut pending[idx]);
                dispatch_actions_inner(
                    actions,
                    idx,
                    apps,
                    pending,
                    &no_loss,
                    &no_partitions,
                    &mut rng,
                    &mut ts,
                );
            }
        }
    }

    // --- Assertion 1: CRDT snapshot equality ---
    let snap_first = apps[component[0]].sync_engine.state().snapshot();
    for &idx in &component[1..] {
        let snap_i = apps[idx].sync_engine.state().snapshot();
        assert_eq!(
            snap_first, snap_i,
            "AppState {} and {} diverged within component {:?}",
            component[0], idx, component
        );
    }

    // --- Assertion 2: Chat completeness ---
    // Every sent chat should appear in the merged view of every peer in the component.
    let merged = apps[component[0]].sync_engine.state().chat.merged_view();
    for &(peer_idx, ref text) in sent_chats {
        // Only check if the sending peer is in this component
        if !component.contains(&peer_idx) {
            continue;
        }
        let user_id = make_user_id(peer_idx);
        let found = merged.iter().any(|(uid, entry)| **uid == user_id && entry.text == *text);
        assert!(
            found,
            "Chat message '{}' from peer {} not found in merged view",
            text, peer_idx
        );
    }

    // Verify per-user seq ordering is monotonic in merged view
    let mut last_seqs: HashMap<&UserId, u64> = HashMap::new();
    for (uid, entry) in &merged {
        if let Some(last) = last_seqs.get(uid) {
            assert!(
                entry.seq >= *last,
                "Non-monotonic seq for user {:?}: {} after {}",
                uid,
                entry.seq,
                last
            );
        }
        last_seqs.insert(uid, entry.seq);
    }

    // --- Assertion 3: Playlist agreement (implied by snapshot eq, but explicit) ---
    let playlist_first = apps[component[0]].sync_engine.state().playlist.snapshot();
    for &idx in &component[1..] {
        let playlist_i = apps[idx].sync_engine.state().playlist.snapshot();
        assert_eq!(
            playlist_first, playlist_i,
            "Playlist diverged: peer {} vs peer {}",
            component[0], idx
        );
    }

    // --- Assertion 4: Playback derivation consistency ---
    // Give all peers identical connected_peers so playback derivation is comparable.
    let peer_map: HashMap<PeerId, UserId> = component
        .iter()
        .map(|&idx| (PeerId(idx as u64), make_user_id(idx)))
        .collect();
    for &idx in component {
        apps[idx].connected_peers = peer_map.clone();
        // Remove self from connected_peers (AppState checks our_user_id separately)
        apps[idx].connected_peers.remove(&PeerId(idx as u64));
        ts += 1;
        // Trigger recompute via Tick (process_event doesn't mutate playback for Tick,
        // but SetUserState does — just re-set the current state to trigger recompute).
        // Actually, we need to call recompute_playback. Simplest: process a no-op event
        // that triggers recompute. Use a dummy RemoteOp that won't change state but
        // will call recompute_playback. Instead, let's just read the playback state
        // after setting connected_peers — it was computed on the last state change.
        // We need to force a recompute. The cleanest way: process a Tick, which doesn't
        // recompute, so we process a SetUserState with the current state.
        let current_user_state = apps[idx]
            .sync_engine
            .state()
            .user_states
            .read(&apps[idx].our_user_id)
            .copied()
            .unwrap_or(UserState::Ready);
        apps[idx].process_event(
            AppEvent::SetUserState {
                state: current_user_state,
            },
            ts,
        );
    }

    let playback_first = &apps[component[0]].playback;
    for &idx in &component[1..] {
        let playback_i = &apps[idx].playback;
        assert_eq!(
            playback_first.should_play, playback_i.should_play,
            "should_play diverged: peer {} ({}) vs peer {} ({})",
            component[0], playback_first.should_play, idx, playback_i.should_play
        );
        assert_eq!(
            playback_first.current_file, playback_i.current_file,
            "current_file diverged: peer {} vs peer {}",
            component[0], idx
        );
    }
}

// ---------------------------------------------------------------------------
// Fuzz target
// ---------------------------------------------------------------------------

fuzz_target!(|input: FuzzInput| {
    let peer_count = 2 + (input.peer_count % 3) as usize; // 2, 3, or 4
    let mut state = FuzzState {
        apps: (0..peer_count)
            .map(|i| AppState::new(make_user_id(i)))
            .collect(),
        pending: vec![Vec::new(); peer_count],
        loss_rates: HashMap::new(),
        partitioned: HashMap::new(),
        rng: SimpleRng::new(input.seed),
        timestamp: 1, // start at 1 (timestamp 0 is rejected)
        sent_chats: Vec::new(),
    };

    for event in &input.events {
        match event {
            FuzzEvent::SendChat { peer, text_byte } => {
                let idx = *peer as usize % peer_count;
                let text = String::from(char::from(*text_byte));
                state.sent_chats.push((idx, text.clone()));
                state.timestamp += 1;
                let effects = state.apps[idx].process_event(
                    AppEvent::SendChat { text },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::SetUserState { peer, state: s } => {
                let idx = *peer as usize % peer_count;
                let user_state = make_user_state(*s);
                state.timestamp += 1;
                let effects = state.apps[idx].process_event(
                    AppEvent::SetUserState { state: user_state },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::SetFileState {
                peer,
                file,
                state: s,
            } => {
                let idx = *peer as usize % peer_count;
                let file_id = make_file_id(*file % 4);
                let file_state = make_file_state(*s);
                state.timestamp += 1;
                let effects = state.apps[idx].process_event(
                    AppEvent::SetFileState { file_id, state: file_state },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::AddToPlaylist { peer, file, after } => {
                let idx = *peer as usize % peer_count;
                let file_id = make_file_id(*file % 4);
                let after = after.map(|a| make_file_id(a % 4));
                state.timestamp += 1;
                let effects = state.apps[idx].process_event(
                    AppEvent::AddToPlaylist { file_id, after },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::RemoveFromPlaylist { peer, file } => {
                let idx = *peer as usize % peer_count;
                let file_id = make_file_id(*file % 4);
                state.timestamp += 1;
                let effects = state.apps[idx].process_event(
                    AppEvent::RemoveFromPlaylist { file_id },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::MoveInPlaylist { peer, file, after } => {
                let idx = *peer as usize % peer_count;
                let file_id = make_file_id(*file % 4);
                let after = after.map(|a| make_file_id(a % 4));
                state.timestamp += 1;
                let effects = state.apps[idx].process_event(
                    AppEvent::MoveInPlaylist { file_id, after },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::PeerConnect { from, to } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx == to_idx {
                    continue;
                }
                state.timestamp += 1;
                let effects = state.apps[from_idx].process_event(
                    AppEvent::PeerConnected {
                        peer_id: PeerId(to_idx as u64),
                        username: make_user_id(to_idx).0,
                    },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    from_idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::PeerDisconnect { from, to } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx == to_idx {
                    continue;
                }
                state.timestamp += 1;
                let effects = state.apps[from_idx].process_event(
                    AppEvent::PeerDisconnected {
                        peer_id: PeerId(to_idx as u64),
                    },
                    state.timestamp,
                );
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    from_idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::Tick { peer } => {
                let idx = *peer as usize % peer_count;
                state.timestamp += 1;
                let effects = state.apps[idx].process_event(AppEvent::Tick, state.timestamp);
                let actions = extract_sync_actions(effects);
                dispatch_actions_inner(
                    actions,
                    idx,
                    &mut state.apps,
                    &mut state.pending,
                    &state.loss_rates,
                    &state.partitioned,
                    &mut state.rng,
                    &mut state.timestamp,
                );
            }
            FuzzEvent::DrainActions { peer } => {
                let idx = *peer as usize % peer_count;
                for _ in 0..100 {
                    if state.pending[idx].is_empty() {
                        break;
                    }
                    let actions = std::mem::take(&mut state.pending[idx]);
                    dispatch_actions_inner(
                        actions,
                        idx,
                        &mut state.apps,
                        &mut state.pending,
                        &state.loss_rates,
                        &state.partitioned,
                        &mut state.rng,
                        &mut state.timestamp,
                    );
                }
            }
            FuzzEvent::TogglePartition { a, b } => {
                let a_idx = *a as usize % peer_count;
                let b_idx = *b as usize % peer_count;
                if a_idx == b_idx {
                    continue;
                }
                let key = (a_idx.min(b_idx), a_idx.max(b_idx));
                let current = state.partitioned.get(&key).copied().unwrap_or(false);
                state.partitioned.insert(key, !current);
            }
            FuzzEvent::SetLoss { from, to, loss_percent } => {
                let from_idx = *from as usize % peer_count;
                let to_idx = *to as usize % peer_count;
                if from_idx != to_idx {
                    let loss = (*loss_percent as f64 / 255.0).min(0.9);
                    state.loss_rates.insert((from_idx, to_idx), loss);
                }
            }
            FuzzEvent::AssertConvergence => {
                let components = connected_components(peer_count, &state.partitioned);
                for component in &components {
                    sync_and_assert_component(
                        component,
                        &mut state.apps,
                        &mut state.pending,
                        &state.sent_chats,
                    );
                }
            }
        }
    }

    // Final: heal everything and assert global convergence
    state.partitioned.clear();
    state.loss_rates.clear();

    // Drain remaining pending
    for idx in 0..peer_count {
        for _ in 0..10 {
            if state.pending[idx].is_empty() {
                break;
            }
            let actions = std::mem::take(&mut state.pending[idx]);
            dispatch_actions_inner(
                actions,
                idx,
                &mut state.apps,
                &mut state.pending,
                &HashMap::new(),
                &HashMap::new(),
                &mut state.rng,
                &mut state.timestamp,
            );
        }
    }

    let all_peers: Vec<usize> = (0..peer_count).collect();
    sync_and_assert_component(
        &all_peers,
        &mut state.apps,
        &mut state.pending,
        &state.sent_chats,
    );
});
