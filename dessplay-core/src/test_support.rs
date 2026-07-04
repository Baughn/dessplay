//! Structured-input generators shared by the proptest suite and the fuzz
//! targets (feature `test-support`).
//!
//! A [`ScriptStep`] is an *intention* — "actor 2 marks file 3 watched at
//! t=17" — over deliberately tiny domains (few files, actors, users,
//! timestamps) so that generated histories collide on the same keys and
//! exercise conflict resolution. [`apply_step`] turns an intention into a
//! real [`CrdtOp`] carrying causal context from the state it was generated
//! against.
//!
//! ## Replay-order rules
//!
//! Register values (`LwwCell`) converge under *any* delivery order —
//! max-merge needs nothing. `crdts::Map` still requires **per-origin
//! FIFO**: its `Up` ops carry per-actor sequence dots, and applying a
//! later dot first masks earlier ones (ops are silently dropped). The
//! hub-and-spoke architecture provides per-origin FIFO everywhere (QUIC
//! control streams are ordered; the server broadcasts one total order;
//! a client seeing its own ops early preserves its own order), so
//! convergence is tested through [`Cluster`], a faithful model of that
//! topology: per-client states with local echo, a server hub consuming
//! client queues in arbitrary order, in-order delivery of the server
//! log, duplicate delivery of own ops, and CvRDT-merge reconnects.

use crate::playlist::NewPlaylistEntry;
use crate::state::{CrdtOp, CrdtState};
use crate::types::{
    ActorId, AniDbMetadata, AniDbSeriesId, ChatMessage, Ed2kHash, FileAvailability, FileHashInfo,
    ListEntryId, ListStatus, ManualState, MetadataSource, NextEpState, PlaybackPosition,
    SeriesListEntry, SeriesRelation, SeriesRelations, SeriesWatchState, SharedTimestamp, UserId,
};

/// Number of distinct actors scripts draw from.
pub const ACTORS: u8 = 4;
/// Number of distinct files scripts draw from.
pub const FILES: u8 = 5;
/// Number of distinct users scripts draw from.
pub const USERS: u8 = 4;
/// Number of distinct series scripts draw from.
pub const SERIES: u8 = 4;
/// Number of distinct List entries scripts draw from.
pub const LIST_ENTRIES: u8 = 4;

/// Deterministic file hash for a small index.
pub fn file(i: u8) -> Ed2kHash {
    Ed2kHash([i % FILES; 16])
}

/// Deterministic actor for a small index. Index 0 is the server.
pub fn actor(i: u8) -> ActorId {
    ActorId((i % ACTORS) as u128)
}

/// Deterministic user for a small index.
pub fn user(i: u8) -> UserId {
    UserId::new(format!("user{}", i % USERS))
}

/// Deterministic series id for a small index.
pub fn series(i: u8) -> AniDbSeriesId {
    AniDbSeriesId((i % SERIES) as u32 + 1)
}

/// Deterministic List entry id for a small index.
pub fn list_entry(i: u8) -> ListEntryId {
    ListEntryId((i % LIST_ENTRIES) as u128 + 1)
}

/// One scripted intention against the shared state.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "test-support", derive(arbitrary::Arbitrary))]
pub enum ScriptOp {
    /// Add a file after another (`None` = front).
    AddPlaylist {
        /// File index.
        file: u8,
        /// Anchor file index, if any.
        after: Option<u8>,
    },
    /// Move a file after another (`None` = front).
    MovePlaylist {
        /// File index.
        file: u8,
        /// Anchor file index, if any.
        after: Option<u8>,
    },
    /// Remove a file.
    RemovePlaylist {
        /// File index.
        file: u8,
    },
    /// Set the watched flag.
    SetWatched {
        /// File index.
        file: u8,
        /// New flag value.
        watched: bool,
    },
    /// Set now-playing.
    SetNowPlaying {
        /// File index; `None` clears.
        file: Option<u8>,
    },
    /// Take seek authority for some actor.
    SetSeekAuthority {
        /// Actor index to install as authority.
        authority: u8,
    },
    /// Write the playback-intent latch.
    SetIntent {
        /// Playing vs Paused.
        playing: bool,
    },
    /// Set a series watch preference.
    SetSeriesPreference {
        /// User index.
        user: u8,
        /// List entry index.
        entry: u8,
        /// Preference selector: 0 = Watching, 1 = NotWatching, else Maybe.
        pref: u8,
        /// Attribution selector: 0 = self (`None`), else `Some(user(setter))`.
        setter: u8,
    },
    /// Set a manual override: 0 = None, 1 = Paused, otherwise Away.
    SetManualOverride {
        /// User index.
        user: u8,
        /// Override selector.
        kind: u8,
        /// Who set Away (user index).
        set_by: u8,
    },
    /// Set file availability: 0 = Ready, 1 = Missing, else Downloading.
    SetFileAvailability {
        /// User index.
        user: u8,
        /// File index.
        file: u8,
        /// Availability selector.
        kind: u8,
        /// Download progress for the Downloading case.
        progress_bps: u16,
    },
    /// Write metadata for a file (server-style).
    SetMetadata {
        /// File index.
        file: u8,
        /// Whether AniDB "knew" the file (drives source + series id).
        known: bool,
        /// Series index for known files.
        series: u8,
    },
    /// Write relations for a series (server-style).
    SetRelations {
        /// Series index.
        series: u8,
        /// Related series index.
        target: u8,
    },
    /// Write a file catalog entry (server-style).
    SetFileCatalog {
        /// File index.
        file: u8,
    },
    /// Create or rewrite a List entry.
    PutListEntry {
        /// Entry index.
        entry: u8,
        /// Status selector.
        status: u8,
        /// Note seed, to vary the payload.
        note: u8,
    },
    /// Update a List entry's progress.
    SetNextEp {
        /// Entry index.
        entry: u8,
        /// Episode number.
        ep: u8,
        /// Out-this-week flag.
        available: bool,
    },
    /// Insert a lookup request.
    RequestLookup {
        /// File index.
        file: u8,
    },
    /// Acknowledge a committed-absent user for a file.
    AcknowledgeAbsent {
        /// File index.
        file: u8,
        /// User index.
        user: u8,
    },
    /// Send a chat message.
    Chat {
        /// Message seed.
        text: u8,
    },
    /// Report a playback position.
    SetPosition {
        /// User index.
        user: u8,
        /// Position in millis.
        millis: u32,
    },
}

/// A scripted step: who does what, when.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "test-support", derive(arbitrary::Arbitrary))]
pub struct ScriptStep {
    /// Acting actor index (also used as the writing user where relevant).
    pub actor: u8,
    /// Shared-clock timestamp. Small domain forces LWW ties.
    pub ts: u16,
    /// The intention.
    pub op: ScriptOp,
}

fn list_status(i: u8) -> ListStatus {
    match i % 8 {
        0 => ListStatus::ShortList,
        1 => ListStatus::Planned,
        2 => ListStatus::Active,
        3 => ListStatus::CurrentSeason,
        4 => ListStatus::Waiting,
        5 => ListStatus::Hiatus,
        6 => ListStatus::Finished,
        _ => ListStatus::Dropped,
    }
}

/// Apply one scripted step to a state, returning the real op (tagged with
/// the lane it must keep its order within — the generating actor).
pub fn apply_step(state: &mut CrdtState, step: &ScriptStep) -> (u8, CrdtOp) {
    let actor_index = step.actor % ACTORS;
    let a = actor(actor_index);
    let ts = SharedTimestamp(step.ts as u64);
    let op = match &step.op {
        ScriptOp::AddPlaylist { file: f, after } => {
            let anchor = after.map(file);
            state.add_playlist_entry_after(
                a,
                ts,
                anchor.as_ref(),
                NewPlaylistEntry {
                    hash: file(*f),
                    added_by: user(actor_index),
                    filename: format!("file{}.mkv", f % FILES),
                    size_bytes: 1_000_000 + (*f as u64),
                    duration_millis: Some(1_440_000),
                },
            )
        }
        ScriptOp::MovePlaylist { file: f, after } => {
            let anchor = after.map(file);
            match state.move_playlist_entry_after(a, ts, file(*f), anchor.as_ref()) {
                Some(op) => op,
                // Entry absent (or anchor == file): fall back to a chat
                // message so every step yields exactly one op.
                None => state.append_chat(ChatMessage {
                    timestamp: ts,
                    sender: user(actor_index),
                    text: format!("noop move {f}"),
                }),
            }
        }
        ScriptOp::RemovePlaylist { file: f } => state.remove_playlist_entry(a, ts, file(*f)),
        ScriptOp::SetWatched { file: f, watched } => state.set_watched(a, ts, file(*f), *watched),
        ScriptOp::SetNowPlaying { file: f } => state.set_now_playing(a, ts, f.map(file)),
        ScriptOp::SetSeekAuthority { authority } => state.set_seek_authority(
            a,
            ts,
            if *authority % ACTORS == 0 {
                crate::types::SeekAuthority::Server
            } else {
                crate::types::SeekAuthority::User(user(*authority))
            },
        ),
        ScriptOp::SetIntent { playing } => state.set_playback_intent(
            a,
            ts,
            if *playing {
                crate::types::PlaybackIntent::Playing
            } else {
                crate::types::PlaybackIntent::Paused
            },
        ),
        ScriptOp::SetSeriesPreference {
            user: u,
            entry: e,
            pref,
            setter,
        } => state.set_series_preference(
            a,
            ts,
            user(*u),
            list_entry(*e),
            match pref % 3 {
                0 => SeriesWatchState::Watching,
                1 => SeriesWatchState::NotWatching,
                _ => SeriesWatchState::Maybe,
            },
            (*setter != 0).then(|| user(*setter)),
        ),
        ScriptOp::SetManualOverride {
            user: u,
            kind,
            set_by,
        } => {
            let value = match kind % 3 {
                0 => None,
                1 => Some(ManualState::Paused),
                _ => Some(ManualState::Away {
                    set_by: user(*set_by),
                }),
            };
            state.set_manual_override(a, ts, user(*u), value)
        }
        ScriptOp::SetFileAvailability {
            user: u,
            file: f,
            kind,
            progress_bps,
        } => {
            let value = match kind % 3 {
                0 => FileAvailability::Ready,
                1 => FileAvailability::Missing,
                _ => FileAvailability::Downloading {
                    progress_bps: progress_bps % 10_001,
                },
            };
            state.set_file_availability(a, ts, user(*u), file(*f), value)
        }
        ScriptOp::SetMetadata {
            file: f,
            known,
            series: s,
        } => {
            let metadata = AniDbMetadata {
                source: if *known {
                    MetadataSource::AniDb
                } else {
                    MetadataSource::FilenameDerived
                },
                series_name: format!("series{}", s % SERIES),
                series_id: known.then(|| series(*s)),
                episode_number: known.then(|| format!("{}", f % FILES)),
            };
            state.set_anidb_metadata(a, ts, file(*f), Some(metadata))
        }
        ScriptOp::SetRelations { series: s, target } => {
            let relations = SeriesRelations {
                title: format!("series{}", s % SERIES),
                year: Some(2020),
                episode_count: Some(12),
                relations: [SeriesRelation {
                    kind: crate::types::RelationKind::Sequel,
                    target: series(*target),
                }]
                .into_iter()
                .collect(),
            };
            state.set_series_relations(a, ts, series(*s), relations)
        }
        ScriptOp::SetFileCatalog { file: f } => state.set_file_catalog(
            a,
            ts,
            file(*f),
            crate::types::FileCatalogEntry {
                filename: format!("file{}.mkv", f % FILES),
                size_bytes: 1_000_000 + (*f as u64),
                duration_millis: None,
            },
        ),
        ScriptOp::PutListEntry {
            entry,
            status,
            note,
        } => {
            let value = SeriesListEntry {
                name: format!("entry{}", entry % LIST_ENTRIES),
                nero_name: None,
                genre: None,
                notes: vec![format!("note{note}")],
                recommender: None,
                status: list_status(*status),
                status_note: None,
                source: None,
                watchers: [user(*entry)].into_iter().collect(),
                anidb_series_id: None,
                local_aliases: Default::default(),
                manual_files: Default::default(),
            };
            state.put_list_entry(a, ts, list_entry(*entry), value)
        }
        ScriptOp::SetNextEp {
            entry,
            ep,
            available,
        } => state.set_next_ep(
            a,
            ts,
            list_entry(*entry),
            NextEpState {
                next_ep: Some(format!("{ep}")),
                available: *available,
            },
        ),
        ScriptOp::RequestLookup { file: f } => state.request_lookup(FileHashInfo {
            hash: file(*f),
            size: 1_000_000 + (*f as u64),
            filename: format!("file{}.mkv", f % FILES),
            // Vary mtime by file index so distinct files stay distinct GSet
            // elements; deterministic, no clock read.
            mtime: Some(*f as i64 * 1000),
            series_hint: None,
        }),
        ScriptOp::AcknowledgeAbsent { file: f, user: u } => {
            state.acknowledge_absent(file(*f), user(*u))
        }
        ScriptOp::Chat { text } => state.append_chat(ChatMessage {
            timestamp: ts,
            sender: user(actor_index),
            text: format!("msg{text}"),
        }),
        ScriptOp::SetPosition { user: u, millis } => state.set_playback_position(
            a,
            ts,
            user(*u),
            PlaybackPosition {
                position_millis: *millis as u64,
                timestamp: ts,
                // Convergence fuzzing exercises the position register, not
                // drift; a fixed file keeps it deterministic.
                file: file(0),
            },
        ),
    };
    (actor_index, op)
}

/// Run a whole script against a fresh state, returning the state and the
/// lane-tagged ops it generated.
pub fn run_script(steps: &[ScriptStep]) -> (CrdtState, Vec<(u8, CrdtOp)>) {
    let mut state = CrdtState::new();
    let ops = steps
        .iter()
        .map(|step| apply_step(&mut state, step))
        .collect();
    (state, ops)
}

/// Number of clients in a [`Cluster`].
pub const CLUSTER_CLIENTS: usize = 3;

/// One scheduling/mutation event in a cluster run.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "test-support", derive(arbitrary::Arbitrary))]
pub enum ClusterEvent {
    /// A client generates an op (applied locally, queued for the server).
    ClientOp {
        /// Client index.
        client: u8,
        /// Shared-clock timestamp.
        ts: u16,
        /// The intention.
        op: ScriptOp,
    },
    /// The server generates an op itself (EOF transitions, metadata).
    ServerOp {
        /// Shared-clock timestamp.
        ts: u16,
        /// The intention.
        op: ScriptOp,
    },
    /// The server consumes one pending op from some client's queue.
    ServerPoll {
        /// Selects which non-empty queue (modulo their count).
        lane: u8,
    },
    /// A client receives the next `count` entries of the server log.
    Deliver {
        /// Client index.
        client: u8,
        /// How many log entries to deliver.
        count: u8,
    },
    /// A client reconnects: CvRDT-merges the server's full state.
    Reconnect {
        /// Client index.
        client: u8,
    },
}

/// A faithful in-process model of the hub-and-spoke sync topology.
pub struct Cluster {
    /// The server's state (also the merge source for reconnects).
    pub server: CrdtState,
    /// Per-client states.
    pub clients: Vec<CrdtState>,
    /// Ops sent by each client, not yet processed by the server.
    pending: Vec<std::collections::VecDeque<CrdtOp>>,
    /// The server's total broadcast order.
    pub log: Vec<CrdtOp>,
    /// Per-client index of the next undelivered log entry.
    delivered: Vec<usize>,
}

impl Default for Cluster {
    fn default() -> Self {
        Self::new()
    }
}

impl Cluster {
    /// A fresh cluster with [`CLUSTER_CLIENTS`] empty clients.
    pub fn new() -> Self {
        Self {
            server: CrdtState::new(),
            clients: vec![CrdtState::new(); CLUSTER_CLIENTS],
            pending: vec![std::collections::VecDeque::new(); CLUSTER_CLIENTS],
            log: Vec::new(),
            delivered: vec![0; CLUSTER_CLIENTS],
        }
    }

    /// Client `i` acts as actor `i + 1` (0 is the server).
    fn client_actor_index(client: u8) -> u8 {
        (client as usize % CLUSTER_CLIENTS) as u8 + 1
    }

    /// Run one event.
    pub fn apply_event(&mut self, event: &ClusterEvent) {
        match event {
            ClusterEvent::ClientOp { client, ts, op } => {
                let index = *client as usize % CLUSTER_CLIENTS;
                let step = ScriptStep {
                    actor: Self::client_actor_index(*client),
                    ts: *ts,
                    op: op.clone(),
                };
                if let (Some(state), Some(queue)) =
                    (self.clients.get_mut(index), self.pending.get_mut(index))
                {
                    let (_, op) = apply_step(state, &step);
                    queue.push_back(op);
                }
            }
            ClusterEvent::ServerOp { ts, op } => {
                let step = ScriptStep {
                    actor: 0,
                    ts: *ts,
                    op: op.clone(),
                };
                let (_, op) = apply_step(&mut self.server, &step);
                self.log.push(op);
            }
            ClusterEvent::ServerPoll { lane } => {
                self.server_poll(*lane);
            }
            ClusterEvent::Deliver { client, count } => {
                let index = *client as usize % CLUSTER_CLIENTS;
                for _ in 0..*count {
                    self.deliver_one(index);
                }
            }
            ClusterEvent::Reconnect { client } => {
                let index = *client as usize % CLUSTER_CLIENTS;
                if let Some(state) = self.clients.get_mut(index) {
                    state.merge(self.server.clone());
                }
            }
        }
    }

    /// Server consumes one pending client op (queue picked by `lane`
    /// modulo the number of non-empty queues). Returns false if idle.
    fn server_poll(&mut self, lane: u8) -> bool {
        let nonempty: Vec<usize> = self
            .pending
            .iter()
            .enumerate()
            .filter(|(_, queue)| !queue.is_empty())
            .map(|(i, _)| i)
            .collect();
        if nonempty.is_empty() {
            return false;
        }
        let pick = nonempty[lane as usize % nonempty.len()];
        if let Some(op) = self.pending.get_mut(pick).and_then(|q| q.pop_front()) {
            self.server.apply(op.clone());
            self.log.push(op);
        }
        true
    }

    /// Deliver the next server log entry to a client (own ops included —
    /// duplicate delivery is part of the protocol).
    fn deliver_one(&mut self, index: usize) {
        if let (Some(state), Some(position)) =
            (self.clients.get_mut(index), self.delivered.get_mut(index))
            && let Some(op) = self.log.get(*position)
        {
            state.apply(op.clone());
            *position += 1;
        }
    }

    /// Drain every queue and bring every client fully up to date. After
    /// this, all states must agree on the resolved view.
    pub fn flush(&mut self) {
        let mut round = 0u8;
        while self.server_poll(round) {
            round = round.wrapping_add(1);
        }
        for index in 0..CLUSTER_CLIENTS {
            while self.delivered.get(index).copied().unwrap_or(usize::MAX) < self.log.len() {
                self.deliver_one(index);
            }
        }
    }
}

/// Run a sequence of cluster events and flush.
pub fn run_cluster(events: &[ClusterEvent]) -> Cluster {
    let mut cluster = Cluster::new();
    for event in events {
        cluster.apply_event(event);
    }
    cluster.flush();
    cluster
}

/// Outcome of delivering an op log through the simulated **datagram lane**
/// ([`deliver_via_datagram_lane`]).
#[derive(Clone, Debug)]
pub struct DatagramLaneOutcome {
    /// The replica after the lane went quiescent.
    pub replica: CrdtState,
    /// How many times an offered op was held back by the per-origin gap
    /// check (`apply_if_orderly` returned `false`). A positive value means
    /// the gap-detection branch was actually exercised; it is `0` for a
    /// log of purely order-free ops, which never hit the gap check.
    pub held: usize,
    /// Ops still undelivered when the lane went quiescent. For a complete
    /// log (every map op an `Up`, plus order-free ops) this is always `0`:
    /// every op eventually reaches its in-sequence slot. A non-zero value
    /// means an op could never apply — a real stall, which a test should
    /// flag.
    pub undelivered: usize,
}

/// Deliver a server-ordered `log` to a fresh replica through a simulated
/// **datagram lane**, the unreliable, possibly-reordered counterpart to
/// [`Cluster`]'s reliable in-order delivery.
///
/// Every op is offered via [`CrdtState::apply_if_orderly`] in `order`
/// (indices into `log` — an arbitrary reorder, typically a shuffled
/// permutation). Any op the per-origin gap check drops is retried on a
/// later pass, modelling the reliable control stream that always carries
/// a second copy of a lost/early datagram. Unlike the reliable path,
/// *every* delivery here — including the eventually-successful ones — goes
/// through the datagram fast path, so the gap-detection logic is exercised
/// end to end. The lane stops once it is quiescent (no op applied in a
/// full pass).
pub fn deliver_via_datagram_lane(log: &[CrdtOp], order: &[usize]) -> DatagramLaneOutcome {
    let mut replica = CrdtState::new();
    let mut pending: std::collections::VecDeque<usize> =
        order.iter().copied().filter(|&i| i < log.len()).collect();
    let mut held = 0usize;
    loop {
        let mut progressed = false;
        // One pass over the currently-pending ops, in offer order.
        for _ in 0..pending.len() {
            let Some(i) = pending.pop_front() else { break };
            if replica.apply_if_orderly(log[i].clone()) {
                progressed = true;
            } else {
                held += 1;
                pending.push_back(i);
            }
        }
        if pending.is_empty() || !progressed {
            break;
        }
    }
    DatagramLaneOutcome {
        replica,
        held,
        undelivered: pending.len(),
    }
}
