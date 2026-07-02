//! The sync actor: owns the replicated CRDT state.
//!
//! Responsibilities (docs/sync-state.md):
//! - Apply local mutations, stamping them with **monotonic** shared-clock
//!   timestamps (`max(shared_now, last_issued + 1)` — pure LWW means an
//!   older-stamped write never wins, so a client's own sequential writes
//!   must have increasing stamps even if NTP slews the clock backward).
//! - Route ops outward: reliable + eager datagram for ordinary ops;
//!   datagram-only for playback positions, with a reliable tick at most
//!   once per second.
//! - Apply remote ops (FIFO-guarded when they arrived by datagram),
//!   merges (same epoch), and snapshots (epoch adoption).
//! - Buffer ops generated while disconnected (positions coalesced to
//!   the latest), replaying them on reconnect.
//! - Watch the server's periodic `StateHash`; two consecutive
//!   mismatches trigger a loud log and a `RequestMerge` self-heal.
//! - Flush snapshots to SQLite periodically and at shutdown.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dessplay_core::net::ServerControl;
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{
    ActorId, AniDbMetadata, AniDbSeriesId, Ed2kHash, Epoch, FileAvailability, FileCatalogEntry,
    FileHashInfo, ListEntryId, ManualState, NextEpState, PlaybackIntent, PlaybackPosition,
    SeekAuthority, SeriesListEntry, SeriesRelations, SeriesWatchState, SharedTimestamp, UserId,
};
use dessplay_core::{ChatMessage, CrdtOp, CrdtState, StateSnapshot, StateView};
use tokio::sync::{mpsc, oneshot};

use super::network::{Clock, NetworkCommand};
use crate::storage::Storage;

/// A state mutation requested by the UI or player layers. The sync
/// actor supplies identity (actor, user) and timestamps.
#[derive(Debug, Clone, PartialEq)]
pub enum Mutation {
    /// Add a file after `anchor` (`None` = front).
    AddPlaylistAfter {
        /// Anchor entry, if any.
        anchor: Option<Ed2kHash>,
        /// The new entry.
        new: NewPlaylistEntry,
    },
    /// Append a file at the end of the playlist.
    PushPlaylist {
        /// The new entry.
        new: NewPlaylistEntry,
    },
    /// Move an entry after `anchor` (`None` = front).
    MovePlaylistAfter {
        /// The entry to move.
        hash: Ed2kHash,
        /// Anchor entry, if any.
        anchor: Option<Ed2kHash>,
    },
    /// Tombstone an entry.
    RemovePlaylist {
        /// The entry to remove.
        hash: Ed2kHash,
    },
    /// Set a watched flag (server-side use).
    SetWatched {
        /// The file.
        hash: Ed2kHash,
        /// The flag.
        watched: bool,
    },
    /// Set now-playing.
    SetNowPlaying {
        /// The file, or `None` to clear.
        file: Option<Ed2kHash>,
    },
    /// Take or assign seek authority.
    SetSeekAuthority {
        /// The new authority.
        authority: SeekAuthority,
    },
    /// Write the play/pause latch. The player layer pairs this with a
    /// manual-override write (pause sets both; play clears the override
    /// and writes `Playing`).
    SetPlaybackIntent {
        /// The new intent.
        intent: PlaybackIntent,
    },
    /// Set a per-user series preference.
    SetSeriesPreference {
        /// Whose preference (usually our own user).
        user: UserId,
        /// The series.
        series: AniDbSeriesId,
        /// Watching or not.
        pref: SeriesWatchState,
    },
    /// Set a manual override (own user; `Away` may target others).
    SetManualOverride {
        /// Whose override.
        user: UserId,
        /// The override; `None` clears.
        state: Option<ManualState>,
    },
    /// Set our availability for a file.
    SetFileAvailability {
        /// The file.
        file: Ed2kHash,
        /// Our availability.
        availability: FileAvailability,
    },
    /// Write file metadata (server-side use).
    SetAniDbMetadata {
        /// The file.
        hash: Ed2kHash,
        /// The metadata.
        metadata: Option<AniDbMetadata>,
    },
    /// Write series relations (server-side use).
    SetSeriesRelations {
        /// The series.
        series: AniDbSeriesId,
        /// Its relations.
        relations: SeriesRelations,
    },
    /// Write a file's catalog identity (server-side use).
    SetFileCatalog {
        /// The file.
        hash: Ed2kHash,
        /// Its identity.
        entry: FileCatalogEntry,
    },
    /// Create or edit a List entry.
    PutListEntry {
        /// Entry id.
        id: ListEntryId,
        /// The entry.
        entry: SeriesListEntry,
    },
    /// Update a List entry's progress.
    SetNextEp {
        /// Entry id.
        id: ListEntryId,
        /// The progress fields.
        next_ep: NextEpState,
    },
    /// Ask the server for an AniDB lookup.
    RequestLookup {
        /// What to look up.
        info: FileHashInfo,
    },
    /// Send a chat message as our user.
    Chat {
        /// Message text.
        text: String,
    },
    /// Report our playback position.
    SetPlaybackPosition {
        /// Position within the file, milliseconds.
        position_millis: u64,
        /// The file the position was sampled against (tags the synced
        /// position so a stale sample from a previous file is ignored after
        /// a now-playing transition).
        file: Ed2kHash,
    },
    /// Backfill a playlist entry's duration (probed by the player when
    /// the adder didn't provide one).
    SetPlaylistDuration {
        /// The entry.
        hash: Ed2kHash,
        /// Probed duration, milliseconds.
        duration_millis: u64,
    },
    /// Acknowledge a committed-but-absent user for a file: a per-file
    /// one-shot that lets the group play past them (see
    /// docs/design.md, Playback Rules).
    AcknowledgeAbsent {
        /// The now-playing file the acknowledgement is scoped to.
        file: Ed2kHash,
        /// The committed-absent user being acknowledged.
        user: UserId,
    },
}

impl Mutation {
    /// The variant's name, for logging.
    pub fn name(&self) -> &'static str {
        match self {
            Mutation::AddPlaylistAfter { .. } => "AddPlaylistAfter",
            Mutation::PushPlaylist { .. } => "PushPlaylist",
            Mutation::MovePlaylistAfter { .. } => "MovePlaylistAfter",
            Mutation::RemovePlaylist { .. } => "RemovePlaylist",
            Mutation::SetWatched { .. } => "SetWatched",
            Mutation::SetNowPlaying { .. } => "SetNowPlaying",
            Mutation::SetSeekAuthority { .. } => "SetSeekAuthority",
            Mutation::SetPlaybackIntent { .. } => "SetPlaybackIntent",
            Mutation::SetSeriesPreference { .. } => "SetSeriesPreference",
            Mutation::SetManualOverride { .. } => "SetManualOverride",
            Mutation::SetFileAvailability { .. } => "SetFileAvailability",
            Mutation::SetAniDbMetadata { .. } => "SetAniDbMetadata",
            Mutation::SetSeriesRelations { .. } => "SetSeriesRelations",
            Mutation::SetFileCatalog { .. } => "SetFileCatalog",
            Mutation::PutListEntry { .. } => "PutListEntry",
            Mutation::SetNextEp { .. } => "SetNextEp",
            Mutation::RequestLookup { .. } => "RequestLookup",
            Mutation::Chat { .. } => "Chat",
            Mutation::SetPlaybackPosition { .. } => "SetPlaybackPosition",
            Mutation::SetPlaylistDuration { .. } => "SetPlaylistDuration",
            Mutation::AcknowledgeAbsent { .. } => "AcknowledgeAbsent",
        }
    }
}

/// Commands into the sync actor.
#[derive(Debug)]
pub enum SyncCommand {
    /// A local mutation.
    Mutate(Box<Mutation>),
    /// A state-sync message from the server.
    Server {
        /// The message.
        msg: Box<ServerControl>,
        /// Arrived as a datagram (unordered path)?
        via_datagram: bool,
    },
    /// The connection authenticated (replay any offline buffer).
    Connected,
    /// The connection dropped (start buffering).
    Disconnected,
    /// New clock offset from time sync.
    ClockSync {
        /// Server-minus-local offset, milliseconds.
        offset_millis: i64,
    },
    /// Fetch the current resolved view.
    GetView(oneshot::Sender<StateView>),
    /// Fetch the current epoch.
    GetEpoch(oneshot::Sender<Epoch>),
    /// Flush to storage and exit.
    Shutdown,
}

/// Events out of the sync actor (toward the UI layer).
#[derive(Debug)]
pub enum SyncEvent {
    /// The state changed; pull a fresh view if you care.
    StateChanged,
    /// The divergence alarm fired (a `RequestMerge` was sent).
    Diverged,
}

/// Static sync actor configuration.
pub struct SyncConfig {
    /// Our user.
    pub user: UserId,
    /// Our session-scoped actor.
    pub actor: ActorId,
    /// Local clock (unix millis).
    pub clock: Clock,
    /// Stored snapshot to start from, if any.
    pub initial: Option<StateSnapshot>,
    /// Persistence; `None` runs stateless (tests).
    pub storage: Option<Storage>,
    /// Snapshot flush cadence.
    pub flush_interval: Duration,
    /// Epoch cell shared with the network actor (drives reconnect auth).
    pub epoch: Arc<AtomicU64>,
}

impl SyncConfig {
    /// Defaults: 30s flush.
    pub fn new(user: UserId, actor: ActorId, clock: Clock, epoch: Arc<AtomicU64>) -> Self {
        Self {
            user,
            actor,
            clock,
            initial: None,
            storage: None,
            flush_interval: Duration::from_secs(30),
            epoch,
        }
    }
}

/// At most one reliable playback-position send per second; the rest go
/// datagram-only.
const POSITION_RELIABLE_INTERVAL: Duration = Duration::from_secs(1);

/// Connection lifecycle as the sync actor sees it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Link {
    /// No connection: buffer everything.
    Down,
    /// Authenticated, but the initial StateMerge/StateSnapshot hasn't
    /// landed yet. Keep buffering: ops sent now could be wiped locally
    /// by a bootstrap snapshot that predates them server-side.
    AwaitingSync,
    /// Fully synced: ops flow immediately.
    Synced,
}

struct SyncActor {
    state: CrdtState,
    user: UserId,
    actor: ActorId,
    clock: Clock,
    epoch_cell: Arc<AtomicU64>,
    /// Mutex only to make the actor `Sync` (rusqlite connections are
    /// not); accessed solely from this task, never across an await.
    storage: std::sync::Mutex<Option<Storage>>,
    offset_millis: i64,
    last_issued: u64,
    link: Link,
    /// Ops generated while disconnected, replayed on reconnect.
    offline_buffer: Vec<CrdtOp>,
    /// Latest position op while disconnected (coalesced).
    offline_position: Option<CrdtOp>,
    last_reliable_position: Option<tokio::time::Instant>,
    hash_mismatches: u32,
    dirty: bool,
    net: mpsc::Sender<NetworkCommand>,
    events: mpsc::Sender<SyncEvent>,
}

/// Run the sync actor until shutdown.
pub async fn run(
    mut config: SyncConfig,
    mut commands: mpsc::Receiver<SyncCommand>,
    net: mpsc::Sender<NetworkCommand>,
    events: mpsc::Sender<SyncEvent>,
) {
    let (state, epoch) = match config.initial.take() {
        Some(snapshot) => (snapshot.state, snapshot.epoch),
        None => (CrdtState::new(), Epoch(0)),
    };
    config.epoch.store(epoch.0, Ordering::SeqCst);

    let storage = std::sync::Mutex::new(config.storage.take());
    // Lamport floor from stored state: a restart must not re-issue
    // stamps the previous incarnation already spent.
    let last_issued = state.max_lww_timestamp().0;
    let mut actor = SyncActor {
        state,
        user: config.user.clone(),
        actor: config.actor,
        clock: Arc::clone(&config.clock),
        epoch_cell: Arc::clone(&config.epoch),
        storage,
        offset_millis: 0,
        last_issued,
        link: Link::Down,
        offline_buffer: Vec::new(),
        offline_position: None,
        last_reliable_position: None,
        hash_mismatches: 0,
        dirty: false,
        net,
        events,
    };

    let mut flush = tokio::time::interval(config.flush_interval);
    flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    flush.reset(); // don't flush immediately

    loop {
        tokio::select! {
            cmd = commands.recv() => {
                match cmd {
                    Some(SyncCommand::Shutdown) | None => {
                        actor.flush_to_storage();
                        return;
                    }
                    Some(cmd) => actor.handle(cmd).await,
                }
            }
            _ = flush.tick() => actor.flush_to_storage(),
        }
    }
}

impl SyncActor {
    fn epoch(&self) -> Epoch {
        Epoch(self.epoch_cell.load(Ordering::SeqCst))
    }

    fn set_epoch(&self, epoch: Epoch) {
        self.epoch_cell.store(epoch.0, Ordering::SeqCst);
    }

    /// Lamport-monotonic shared-clock stamp: above our own previous
    /// stamps *and* above everything we've observed (see
    /// [`Self::observe`]).
    fn stamp(&mut self) -> SharedTimestamp {
        let shared = (self.clock)().saturating_add_signed(self.offset_millis);
        let ts = shared.max(self.last_issued + 1);
        self.last_issued = ts;
        SharedTimestamp(ts)
    }

    /// Raise the Lamport floor to an observed remote timestamp, so our
    /// next write dominates it. Without this, a write issued in the
    /// same shared-clock millisecond as a just-received remote write
    /// would tie and fall to the value tiebreak — and causally-later
    /// writes must never lose.
    fn observe(&mut self, ts: Option<SharedTimestamp>) {
        if let Some(ts) = ts {
            self.last_issued = self.last_issued.max(ts.0);
        }
    }

    fn flush_to_storage(&mut self) {
        if !self.dirty {
            return;
        }
        let guard = match self.storage.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(storage) = &*guard {
            let started = std::time::Instant::now();
            let snapshot = StateSnapshot {
                epoch: self.epoch(),
                state: self.state.clone(),
            };
            let now = (self.clock)() as i64;
            match storage.save_state(&snapshot, now) {
                Ok(()) => {
                    self.dirty = false;
                    tracing::debug!(
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "state flushed to storage"
                    );
                }
                Err(e) => tracing::error!("state flush failed: {e}"),
            }
        } else {
            self.dirty = false;
        }
    }

    fn changed(&mut self) {
        self.dirty = true;
        // StateChanged is an edge-triggered "pull a view if you care"
        // signal, so dropping it when the channel is full is free
        // coalescing — and crucially, a stalled (or absent) consumer
        // must never block the sync actor. A blocking send here
        // deadlocked the whole client once >256 ops arrived unpolled.
        let _ = self.events.try_send(SyncEvent::StateChanged);
    }

    async fn send_out(&self, cmd: NetworkCommand) {
        if self.net.send(cmd).await.is_err() {
            tracing::warn!("network actor gone; op not sent");
        }
    }

    async fn handle(&mut self, cmd: SyncCommand) {
        match cmd {
            SyncCommand::Mutate(mutation) => self.mutate(*mutation).await,
            SyncCommand::Server { msg, via_datagram } => {
                self.remote(*msg, via_datagram).await;
            }
            SyncCommand::Connected => {
                // Don't replay yet: wait for the initial merge/snapshot
                // (a bootstrap snapshot would wipe ops we sent early,
                // leaving the server with edits we ourselves lack).
                self.link = Link::AwaitingSync;
                tracing::debug!(
                    buffered_ops = self.offline_buffer.len(),
                    buffered_position = self.offline_position.is_some(),
                    "link: awaiting initial sync"
                );
            }
            SyncCommand::Disconnected => {
                self.link = Link::Down;
                self.hash_mismatches = 0;
                tracing::debug!(
                    buffered_ops = self.offline_buffer.len(),
                    "link: down; buffering ops locally"
                );
            }
            SyncCommand::ClockSync { offset_millis } => {
                self.offset_millis = offset_millis;
            }
            SyncCommand::GetView(reply) => {
                let _ = reply.send(self.state.view());
            }
            SyncCommand::GetEpoch(reply) => {
                let _ = reply.send(self.epoch());
            }
            SyncCommand::Shutdown => unreachable!("handled by the run loop"),
        }
    }

    async fn mutate(&mut self, mutation: Mutation) {
        let actor = self.actor;
        let user = self.user.clone();
        let ts = self.stamp();
        let is_position = matches!(mutation, Mutation::SetPlaybackPosition { .. });
        tracing::trace!(mutation = mutation.name(), ts = ts.0, "local mutation");

        let op = match mutation {
            Mutation::AddPlaylistAfter { anchor, new } => {
                self.state
                    .add_playlist_entry_after(actor, ts, anchor.as_ref(), new)
            }
            Mutation::PushPlaylist { new } => self.state.push_playlist_entry(actor, ts, new),
            Mutation::MovePlaylistAfter { hash, anchor } => {
                match self
                    .state
                    .move_playlist_entry_after(actor, ts, hash, anchor.as_ref())
                {
                    Some(op) => op,
                    None => return, // entry vanished; nothing to do
                }
            }
            Mutation::RemovePlaylist { hash } => self.state.remove_playlist_entry(actor, ts, hash),
            Mutation::SetWatched { hash, watched } => {
                self.state.set_watched(actor, ts, hash, watched)
            }
            Mutation::SetNowPlaying { file } => self.state.set_now_playing(actor, ts, file),
            Mutation::SetSeekAuthority { authority } => {
                self.state.set_seek_authority(actor, ts, authority)
            }
            Mutation::SetPlaybackIntent { intent } => {
                self.state.set_playback_intent(actor, ts, intent)
            }
            Mutation::SetSeriesPreference { user, series, pref } => self
                .state
                .set_series_preference(actor, ts, user, series, pref),
            Mutation::SetManualOverride { user, state } => {
                self.state.set_manual_override(actor, ts, user, state)
            }
            Mutation::SetFileAvailability { file, availability } => self
                .state
                .set_file_availability(actor, ts, user, file, availability),
            Mutation::SetAniDbMetadata { hash, metadata } => {
                self.state.set_anidb_metadata(actor, ts, hash, metadata)
            }
            Mutation::SetSeriesRelations { series, relations } => self
                .state
                .set_series_relations(actor, ts, series, relations),
            Mutation::SetFileCatalog { hash, entry } => {
                self.state.set_file_catalog(actor, ts, hash, entry)
            }
            Mutation::PutListEntry { id, entry } => self.state.put_list_entry(actor, ts, id, entry),
            Mutation::SetNextEp { id, next_ep } => self.state.set_next_ep(actor, ts, id, next_ep),
            Mutation::RequestLookup { info } => self.state.request_lookup(info),
            Mutation::Chat { text } => self.state.append_chat(ChatMessage {
                timestamp: ts,
                sender: user,
                text,
            }),
            Mutation::SetPlaybackPosition {
                position_millis,
                file,
            } => self.state.set_playback_position(
                actor,
                ts,
                user,
                PlaybackPosition {
                    position_millis,
                    timestamp: ts,
                    file,
                },
            ),
            Mutation::SetPlaylistDuration {
                hash,
                duration_millis,
            } => {
                // Whole-entry LWW rewrite: read the winning state, fill
                // in the duration. (A concurrent move loses its position
                // to this write — rare and self-healing, the next move
                // rewrites again.)
                let Some(entry) = self
                    .state
                    .view()
                    .playlist
                    .into_iter()
                    .find(|entry| entry.hash == hash)
                else {
                    return; // entry vanished; nothing to backfill
                };
                let mut updated = entry.state;
                updated.duration_millis = Some(duration_millis);
                self.state.set_playlist_entry(actor, ts, hash, updated)
            }
            Mutation::AcknowledgeAbsent { file, user } => self.state.acknowledge_absent(file, user),
        };

        if self.link == Link::Synced {
            let msg = Box::new(ServerControl::StateOp {
                epoch: self.epoch(),
                op,
            });
            if is_position {
                let now = tokio::time::Instant::now();
                let due_reliable = self
                    .last_reliable_position
                    .is_none_or(|last| now.duration_since(last) >= POSITION_RELIABLE_INTERVAL);
                if due_reliable {
                    self.last_reliable_position = Some(now);
                    self.send_out(NetworkCommand::SendEager(msg)).await;
                } else {
                    self.send_out(NetworkCommand::SendDatagramOnly(msg)).await;
                }
            } else {
                self.send_out(NetworkCommand::SendEager(msg)).await;
            }
        } else if is_position {
            self.offline_position = Some(op);
        } else {
            self.offline_buffer.push(op);
        }

        self.changed();
    }

    /// The initial sync landed: re-apply anything generated while
    /// offline onto the adopted state (idempotent on the merge path,
    /// restorative on the snapshot path), then push our **full state**
    /// up as a `StateMerge` and open the gates.
    ///
    /// The upward merge — not per-op replay — is what makes reconnects
    /// self-healing: ops that were sent but undelivered when the old
    /// connection died exist only in our state, and no replay queue
    /// knows about them. CvRDT merge carries everything, including the
    /// offline buffer just re-applied. The server broadcasts a merge to
    /// all clients if ours changed anything.
    async fn synced(&mut self) {
        if self.link == Link::Down {
            // A merge can also arrive from divergence healing while we
            // believe ourselves down; don't replay into a dead link.
            return;
        }
        let first_sync = self.link == Link::AwaitingSync;
        self.link = Link::Synced;
        if !first_sync {
            tracing::debug!("link: synced (mid-session heal)");
            return; // mid-session heal: nothing to push
        }
        tracing::debug!(
            replayed_ops = self.offline_buffer.len(),
            replayed_position = self.offline_position.is_some(),
            "link: synced; replaying offline buffer and pushing upward merge"
        );
        for op in std::mem::take(&mut self.offline_buffer) {
            self.state.apply(op);
        }
        if let Some(op) = self.offline_position.take() {
            self.state.apply(op);
        }
        self.send_out(NetworkCommand::SendReliable(Box::new(
            ServerControl::StateMerge(StateSnapshot {
                epoch: self.epoch(),
                state: self.state.clone(),
            }),
        )))
        .await;
    }

    async fn remote(&mut self, msg: ServerControl, via_datagram: bool) {
        match msg {
            ServerControl::StateOp { epoch, op } => {
                if epoch != self.epoch() {
                    // Op from across a compaction boundary. The reliable
                    // copy of a post-compaction op always follows the
                    // snapshot on the ordered control stream; anything
                    // mismatching here is a stale datagram or a stale op,
                    // and applying it would corrupt dot sequences.
                    tracing::debug!(
                        op = op.variant_name(),
                        op_epoch = epoch.0,
                        our_epoch = self.epoch().0,
                        "dropping cross-epoch op"
                    );
                    return;
                }
                tracing::trace!(op = op.variant_name(), via_datagram, "applying remote op");
                self.observe(op.lww_timestamp());
                if via_datagram {
                    // Unordered path: only apply if it cannot mask a gap.
                    self.state.apply_if_orderly(op);
                } else {
                    self.state.apply(op);
                }
                self.changed();
            }
            ServerControl::StateMerge(snapshot) => {
                let ours = self.epoch();
                // The view hash is only worth computing when someone is
                // listening at debug — merges are rare, but not free.
                let before =
                    tracing::enabled!(tracing::Level::DEBUG).then(|| self.state.view_hash());
                if snapshot.epoch == ours {
                    self.state.merge(snapshot.state);
                } else if snapshot.epoch > ours {
                    // Compaction raced our merge: adopt wholesale.
                    tracing::debug!(
                        from = ours.0,
                        to = snapshot.epoch.0,
                        "adopting newer epoch via merge"
                    );
                    self.set_epoch(snapshot.epoch);
                    self.state = snapshot.state;
                } else {
                    tracing::warn!("ignoring stale-epoch merge ({:?})", snapshot.epoch);
                    return;
                }
                if let Some(before) = before {
                    tracing::debug!(
                        epoch = self.epoch().0,
                        changed_view = before != self.state.view_hash(),
                        "merge applied"
                    );
                }
                self.observe(Some(self.state.max_lww_timestamp()));
                self.hash_mismatches = 0;
                self.synced().await;
                self.changed();
            }
            ServerControl::StateSnapshot(snapshot) => {
                if snapshot.epoch < self.epoch() {
                    tracing::warn!("ignoring stale snapshot ({:?})", snapshot.epoch);
                    return;
                }
                tracing::debug!(
                    from = self.epoch().0,
                    to = snapshot.epoch.0,
                    "adopting snapshot"
                );
                self.set_epoch(snapshot.epoch);
                self.state = snapshot.state;
                self.observe(Some(self.state.max_lww_timestamp()));
                self.hash_mismatches = 0;
                self.synced().await;
                self.changed();
            }
            ServerControl::StateHash { epoch, hash } => {
                if epoch != self.epoch() {
                    return; // snapshot in flight; not comparable
                }
                if self.state.view_hash() == hash {
                    self.hash_mismatches = 0;
                    return;
                }
                self.hash_mismatches += 1;
                if self.hash_mismatches >= 2 {
                    tracing::error!(
                        "DIVERGENCE: view hash mismatched the server twice; requesting merge"
                    );
                    self.hash_mismatches = 0;
                    self.send_out(NetworkCommand::SendReliable(Box::new(
                        ServerControl::RequestMerge,
                    )))
                    .await;
                    // Diverged is an edge-triggered "you may want to react"
                    // signal, exactly like StateChanged in `changed()`: a
                    // stalled (or absent) consumer must never block the sync
                    // actor. The RequestMerge above is already queued to the
                    // network actor regardless of whether the UI observes
                    // this, so dropping it when the channel is full is safe.
                    let _ = self.events.try_send(SyncEvent::Diverged);
                }
            }
            other => {
                tracing::debug!("sync actor ignoring: {other:?}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::types::Epoch;

    use super::*;

    struct Rig {
        commands: mpsc::Sender<SyncCommand>,
        net: mpsc::Receiver<NetworkCommand>,
        events: mpsc::Receiver<SyncEvent>,
    }

    fn rig() -> Rig {
        let epoch = Arc::new(AtomicU64::new(0));
        let config = SyncConfig::new(
            UserId::new("baughn"),
            ActorId::session("baughn", 1),
            Arc::new(|| 1_000_000),
            epoch,
        );
        let (command_tx, command_rx) = mpsc::channel(64);
        let (net_tx, net_rx) = mpsc::channel(64);
        let (event_tx, event_rx) = mpsc::channel(64);
        tokio::spawn(run(config, command_rx, net_tx, event_tx));
        Rig {
            commands: command_tx,
            net: net_rx,
            events: event_rx,
        }
    }

    async fn view_of(rig: &Rig) -> StateView {
        let (tx, rx) = oneshot::channel();
        rig.commands.send(SyncCommand::GetView(tx)).await.unwrap();
        rx.await.unwrap()
    }

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    /// Connected + empty same-epoch merge: the full open-gates
    /// handshake. Consumes the client's upward merge push.
    async fn go_online(rig: &mut Rig) {
        rig.commands.send(SyncCommand::Connected).await.unwrap();
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateMerge(StateSnapshot {
                    epoch: Epoch(0),
                    state: CrdtState::new(),
                })),
                via_datagram: false,
            })
            .await
            .unwrap();
        let push = rig.net.recv().await.unwrap();
        assert!(
            matches!(
                &push,
                NetworkCommand::SendReliable(msg)
                    if matches!(**msg, ServerControl::StateMerge(_))
            ),
            "expected upward merge push, got {push:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn monotonic_stamps_survive_backward_clock() {
        // Clock pinned at 1_000_000; offset drops by 5000 between writes.
        let rig = rig();
        rig.commands
            .send(SyncCommand::ClockSync { offset_millis: 0 })
            .await
            .unwrap();
        rig.commands
            .send(SyncCommand::Mutate(Box::new(Mutation::SetNowPlaying {
                file: Some(hash(1)),
            })))
            .await
            .unwrap();
        rig.commands
            .send(SyncCommand::ClockSync {
                offset_millis: -5_000,
            })
            .await
            .unwrap();
        rig.commands
            .send(SyncCommand::Mutate(Box::new(Mutation::SetNowPlaying {
                file: Some(hash(2)),
            })))
            .await
            .unwrap();
        // The second write must win despite the backward clock step.
        assert_eq!(view_of(&rig).await.now_playing, Some(hash(2)));
    }

    #[tokio::test(start_paused = true)]
    async fn stamps_dominate_observed_remote_timestamps() {
        // The Lamport condition: a local write issued causally after a
        // remote op must out-stamp it, even when the remote stamp is
        // far ahead of our clock. Found by the Phase 5 EOF tests: the
        // server's forced Paused tied with (and lost to) a client's
        // Playing written in the same simulated millisecond.
        let mut rig = rig();
        go_online(&mut rig).await;

        // A remote write stamped way in our future.
        let mut origin = CrdtState::new();
        let op = origin.set_now_playing(ActorId::SERVER, SharedTimestamp(5_000_000), Some(hash(1)));
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateOp {
                    epoch: Epoch(0),
                    op,
                }),
                via_datagram: false,
            })
            .await
            .unwrap();

        // Our causally-later write must win despite our clock reading
        // 1_000_000.
        rig.commands
            .send(SyncCommand::Mutate(Box::new(Mutation::SetNowPlaying {
                file: Some(hash(2)),
            })))
            .await
            .unwrap();
        assert_eq!(view_of(&rig).await.now_playing, Some(hash(2)));
    }

    #[tokio::test(start_paused = true)]
    async fn slow_ui_consumer_never_wedges_the_sync_actor() {
        // The events receiver is held but never drained (a stalled or
        // absent UI). A flood of remote ops generates a StateChanged
        // per op; if those sends block, the actor deadlocks and stops
        // answering queries — found by the Phase 6 import test, which
        // was the first to push >256 ops through a handle nobody
        // polled.
        let mut rig = rig();
        go_online(&mut rig).await;

        let mut origin = CrdtState::new();
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            for i in 0..600u64 {
                let op = origin.append_chat(dessplay_core::ChatMessage {
                    timestamp: SharedTimestamp(i + 1),
                    sender: UserId::new("kim"),
                    text: format!("m{i}"),
                });
                rig.commands
                    .send(SyncCommand::Server {
                        msg: Box::new(ServerControl::StateOp {
                            epoch: Epoch(0),
                            op,
                        }),
                        via_datagram: false,
                    })
                    .await
                    .unwrap();
            }
            view_of(&rig).await
        })
        .await;
        let view = result.expect("sync actor wedged by an undrained event channel");
        assert_eq!(view.chat.len(), 600);
    }

    #[tokio::test(start_paused = true)]
    async fn offline_buffer_replays_with_positions_coalesced() {
        let mut rig = rig();
        // Disconnected from the start: mutate twice + many positions.
        for text in ["one", "two"] {
            rig.commands
                .send(SyncCommand::Mutate(Box::new(Mutation::Chat {
                    text: text.into(),
                })))
                .await
                .unwrap();
        }
        for millis in [100, 200, 300] {
            rig.commands
                .send(SyncCommand::Mutate(Box::new(
                    Mutation::SetPlaybackPosition {
                        position_millis: millis,
                        file: Ed2kHash([1; 16]),
                    },
                )))
                .await
                .unwrap();
        }
        // Nothing sent while offline.
        tokio::task::yield_now().await;
        assert!(rig.net.try_recv().is_err());

        // The handshake's upward merge must carry the buffered edits,
        // with positions coalesced to the latest.
        rig.commands.send(SyncCommand::Connected).await.unwrap();
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateMerge(StateSnapshot {
                    epoch: Epoch(0),
                    state: CrdtState::new(),
                })),
                via_datagram: false,
            })
            .await
            .unwrap();
        let push = rig.net.recv().await.unwrap();
        let NetworkCommand::SendReliable(msg) = push else {
            panic!("expected reliable push, got {push:?}");
        };
        let ServerControl::StateMerge(snapshot) = *msg else {
            panic!("expected upward merge, got {msg:?}");
        };
        let pushed = snapshot.state.view();
        assert_eq!(pushed.chat.len(), 2);
        let pos = pushed.playback_position.values().next().unwrap();
        assert_eq!(pos.position_millis, 300, "position not coalesced to latest");
        assert!(rig.net.try_recv().is_err());
    }

    #[tokio::test(start_paused = true)]
    async fn position_cadence_one_reliable_per_second() {
        let mut rig = rig();
        go_online(&mut rig).await;

        let mut eager = 0;
        let mut datagram_only = 0;
        for i in 0..25u64 {
            rig.commands
                .send(SyncCommand::Mutate(Box::new(
                    Mutation::SetPlaybackPosition {
                        position_millis: i * 100,
                        file: Ed2kHash([1; 16]),
                    },
                )))
                .await
                .unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        while let Ok(cmd) = rig.net.try_recv() {
            match cmd {
                NetworkCommand::SendEager(_) => eager += 1,
                NetworkCommand::SendDatagramOnly(_) => datagram_only += 1,
                other => panic!("unexpected: {other:?}"),
            }
        }
        // 2.5 simulated seconds of 100ms updates: ~3 reliable ticks,
        // the rest datagram-only.
        assert!((2..=4).contains(&eager), "eager: {eager}");
        assert_eq!(eager + datagram_only, 25);
    }

    #[tokio::test(start_paused = true)]
    async fn divergence_alarm_after_two_mismatches() {
        let mut rig = rig();
        go_online(&mut rig).await;

        let bogus = ServerControl::StateHash {
            epoch: Epoch(0),
            hash: [0xAB; 32],
        };
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(bogus.clone()),
                via_datagram: false,
            })
            .await
            .unwrap();
        tokio::task::yield_now().await;
        // One mismatch: no alarm yet (StateChanged events are fine).
        while let Ok(event) = rig.events.try_recv() {
            assert!(!matches!(event, SyncEvent::Diverged));
        }
        assert!(rig.net.try_recv().is_err());

        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(bogus),
                via_datagram: false,
            })
            .await
            .unwrap();
        // Second mismatch: RequestMerge + Diverged.
        let cmd = rig.net.recv().await.unwrap();
        assert!(matches!(
            cmd,
            NetworkCommand::SendReliable(msg) if matches!(*msg, ServerControl::RequestMerge)
        ));
        loop {
            match rig.events.recv().await.unwrap() {
                SyncEvent::Diverged => break,
                SyncEvent::StateChanged => continue,
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn divergence_never_wedges_a_stalled_consumer() {
        // A stalled (or absent) UI: rig.events is held but never drained.
        // The same hazard slow_ui_consumer_never_wedges_the_sync_actor
        // guards for StateChanged — but the Diverged send must obey the
        // same non-blocking contract, or two consecutive hash mismatches
        // arriving while the events channel is full deadlock the actor.
        let mut rig = rig();
        go_online(&mut rig).await;

        // Flood remote ops to fill (and keep full) the undrained events
        // channel: each op fires a StateChanged.
        let mut origin = CrdtState::new();
        for i in 0..200u64 {
            let op = origin.append_chat(dessplay_core::ChatMessage {
                timestamp: SharedTimestamp(i + 1),
                sender: UserId::new("kim"),
                text: format!("m{i}"),
            });
            rig.commands
                .send(SyncCommand::Server {
                    msg: Box::new(ServerControl::StateOp {
                        epoch: Epoch(0),
                        op,
                    }),
                    via_datagram: false,
                })
                .await
                .unwrap();
        }

        // Two consecutive hash mismatches → divergence, which sends
        // SyncEvent::Diverged. With a blocking send and a full, undrained
        // channel this wedges the actor; with try_send it does not.
        let bogus = ServerControl::StateHash {
            epoch: Epoch(0),
            hash: [0xAB; 32],
        };
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(bogus.clone()),
                via_datagram: false,
            })
            .await
            .unwrap();
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(bogus),
                via_datagram: false,
            })
            .await
            .unwrap();

        // The actor must still answer queries (RequestMerge already queued
        // to the net actor regardless of whether the UI observes Diverged).
        let view = tokio::time::timeout(Duration::from_secs(30), view_of(&rig))
            .await
            .expect("sync actor wedged by a blocking Diverged send on a full event channel");
        assert_eq!(view.chat.len(), 200);
    }

    #[tokio::test(start_paused = true)]
    async fn matching_hash_resets_the_mismatch_counter() {
        let mut rig = rig();
        go_online(&mut rig).await;

        let bogus = ServerControl::StateHash {
            epoch: Epoch(0),
            hash: [0xAB; 32],
        };
        let honest_hash = {
            // An empty state's real hash.
            CrdtState::new().view_hash()
        };
        let honest = ServerControl::StateHash {
            epoch: Epoch(0),
            hash: honest_hash,
        };
        for msg in [bogus.clone(), honest, bogus] {
            rig.commands
                .send(SyncCommand::Server {
                    msg: Box::new(msg),
                    via_datagram: false,
                })
                .await
                .unwrap();
        }
        tokio::task::yield_now().await;
        // mismatch, match (reset), mismatch: never two in a row.
        assert!(rig.net.try_recv().is_err());
        while let Ok(event) = rig.events.try_recv() {
            assert!(!matches!(event, SyncEvent::Diverged));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn snapshot_adoption_updates_epoch() {
        let rig = rig();
        let mut fresh = CrdtState::new();
        fresh.set_now_playing(ActorId::SERVER, SharedTimestamp(5), Some(hash(7)));
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateSnapshot(StateSnapshot {
                    epoch: Epoch(9),
                    state: fresh,
                })),
                via_datagram: false,
            })
            .await
            .unwrap();
        assert_eq!(view_of(&rig).await.now_playing, Some(hash(7)));

        let (tx, rx) = oneshot::channel();
        rig.commands.send(SyncCommand::GetEpoch(tx)).await.unwrap();
        assert_eq!(rx.await.unwrap(), Epoch(9));

        // A stale snapshot is refused.
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateSnapshot(StateSnapshot {
                    epoch: Epoch(3),
                    state: CrdtState::new(),
                })),
                via_datagram: false,
            })
            .await
            .unwrap();
        assert_eq!(view_of(&rig).await.now_playing, Some(hash(7)));
    }

    /// Regression coverage for the epoch-adoption-via-*snapshot* reconnect
    /// path. On a post-compaction reconnect the server sends a higher-epoch
    /// `StateSnapshot`; the actor must (1) adopt it wholesale, discarding
    /// stale local state absent from it, (2) replay the offline buffer onto
    /// the adopted state so unsent edits survive, (3) push those edits up as
    /// a `StateMerge`, and (4) advance the epoch and open the gates. The only
    /// prior snapshot test drove a fresh (never-`Connected`) actor, so link
    /// stayed `Down` and `synced()`'s replay/push never ran on this path.
    #[tokio::test(start_paused = true)]
    async fn snapshot_reconnect_discards_stale_replays_offline_and_advances_epoch() {
        let mut rig = rig();

        // Stale local state from a previous session: adopt an epoch-0
        // snapshot (now-playing hash(1)) while still Down. synced()
        // early-returns on Down, so this only seeds self.state — the gates
        // stay closed and nothing is pushed.
        let mut prev = CrdtState::new();
        prev.set_now_playing(ActorId::SERVER, SharedTimestamp(1), Some(hash(1)));
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateSnapshot(StateSnapshot {
                    epoch: Epoch(0),
                    state: prev,
                })),
                via_datagram: false,
            })
            .await
            .unwrap();
        assert_eq!(view_of(&rig).await.now_playing, Some(hash(1)));

        // An edit made while offline: it applies locally AND buffers, but
        // nothing goes out (we're Down).
        rig.commands
            .send(SyncCommand::Mutate(Box::new(Mutation::Chat {
                text: "offline edit".into(),
            })))
            .await
            .unwrap();
        tokio::task::yield_now().await;
        assert!(rig.net.try_recv().is_err(), "nothing sent while offline");

        // Reconnect: Connected -> AwaitingSync, then a higher-epoch snapshot
        // that knows nothing of hash(1) or our offline chat (post-compaction).
        rig.commands.send(SyncCommand::Connected).await.unwrap();
        let mut server = CrdtState::new();
        server.set_now_playing(ActorId::SERVER, SharedTimestamp(10), Some(hash(2)));
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateSnapshot(StateSnapshot {
                    epoch: Epoch(4),
                    state: server,
                })),
                via_datagram: false,
            })
            .await
            .unwrap();

        // The upward merge push carries the adopted server state PLUS the
        // replayed offline edit — proof unsent ops survive the snapshot path.
        let push = rig.net.recv().await.unwrap();
        let NetworkCommand::SendReliable(msg) = push else {
            panic!("expected reliable push, got {push:?}");
        };
        let ServerControl::StateMerge(snapshot) = *msg else {
            panic!("expected upward merge, got {msg:?}");
        };
        let pushed = snapshot.state.view();
        assert_eq!(
            pushed.now_playing,
            Some(hash(2)),
            "adopted the server's now-playing"
        );
        assert_eq!(pushed.chat.len(), 1, "offline edit replayed into the push");
        assert_eq!(snapshot.epoch, Epoch(4));

        // Local view: stale hash(1) discarded, server hash(2) adopted, the
        // offline chat restored, epoch advanced.
        let view = view_of(&rig).await;
        assert_eq!(view.now_playing, Some(hash(2)));
        assert_eq!(view.chat.len(), 1);
        let (tx, rx) = oneshot::channel();
        rig.commands.send(SyncCommand::GetEpoch(tx)).await.unwrap();
        assert_eq!(rx.await.unwrap(), Epoch(4));
    }
}
