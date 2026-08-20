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
//!   mismatches trigger a loud log and a `RequestMerge` self-heal;
//!   three consecutive failed heals escalate to the user
//!   ([`SyncEvent::DivergencePersisted`] → the `/resync` advisory).
//! - Flush snapshots to SQLite periodically and at shutdown.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use dessplay_core::net::ServerControl;
use dessplay_core::playlist::NewPlaylistEntry;
use dessplay_core::types::{
    ActorId, AniDbMetadata, AniDbSeriesId, Ed2kHash, Epoch, FileAvailability, FileCatalogEntry,
    FileHashInfo, ListEntryId, ManualState, MarqueeMessage, NextEpState, PlaybackIntent,
    PlaybackPosition, SeekAuthority, SeriesListEntry, SeriesRelations, SeriesWatchState,
    SharedTimestamp, UserId,
};
use dessplay_core::{ChatMessage, CrdtOp, CrdtState, StateSnapshot, StateView};
use tokio::sync::{mpsc, oneshot};

use super::network::{Clock, NetworkCommand};
use crate::sync_storage::SyncStorage;

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
    /// Record an explicit user seek and grant that user authority.
    SetUserSeek {
        /// The now-playing file on which the seek occurred.
        file: Ed2kHash,
        /// Position when scrubbing began.
        from_millis: u64,
        /// Final position after debouncing.
        to_millis: u64,
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
        /// Whose preference (usually our own user, but `n` / `/skip <name>`
        /// can target another).
        user: UserId,
        /// The List entry (design.md, Series Identity -- not the AniDB
        /// series id; AniDB linking is enrichment only).
        entry: ListEntryId,
        /// Watching or not.
        pref: SeriesWatchState,
        /// Who wrote this, if not `user` themself (design.md #7/#13).
        /// `None` for every self-directed write and system auto-write.
        set_by: Option<UserId>,
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
    /// Write (or clear) the synced marquee line (design.md, AI
    /// Commentary — every client scrolls it on update).
    SetMarquee {
        /// The line to show; `None` clears the register.
        message: Option<MarqueeMessage>,
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
            Mutation::SetUserSeek { .. } => "SetUserSeek",
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
            Mutation::SetMarquee { .. } => "SetMarquee",
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
    /// Discard the replicated state wholesale and re-adopt the server's
    /// copy (`/resync`, Settings → "Reset synced state"). Safe in any
    /// link state: connected, a `RequestMerge` fetches the curative
    /// snapshot; down, the reconnect handshake covers it.
    ResetState,
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
    /// [`HEAL_ATTEMPTS_ESCALATE`] consecutive heals failed to produce a
    /// matching hash: auto-healing isn't working, tell the user to
    /// `/resync`. Emitted once per escalation (the advisor's flag is
    /// sticky; the chat line must not repeat every hash period).
    DivergencePersisted,
    /// A matching `StateHash` arrived after at least one failed heal or
    /// an escalation: the divergence is over. Clears the advisor's
    /// sticky flag — `HealthLevel::Ok` deliberately does not, because a
    /// healthy link says nothing about replica equality.
    DivergenceHealed,
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
    /// Persistence (the sync database); `None` runs stateless (tests).
    pub storage: Option<SyncStorage>,
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

/// Consecutive failed heals tolerated before escalating to the user.
/// Three, not two: an op sent between our `RequestMerge` and the
/// server's curative snapshot re-diverges the replica once through no
/// fault of the mechanism (~one hash period to re-converge), so the
/// second alarm can be that race rather than a real failure.
const HEAL_ATTEMPTS_ESCALATE: u32 = 3;

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
    storage: std::sync::Mutex<Option<SyncStorage>>,
    offset_millis: i64,
    last_issued: u64,
    link: Link,
    /// Ops generated while disconnected, replayed on reconnect.
    offline_buffer: Vec<CrdtOp>,
    /// Latest position op while disconnected (coalesced).
    offline_position: Option<CrdtOp>,
    last_reliable_position: Option<tokio::time::Instant>,
    hash_mismatches: u32,
    /// Consecutive divergence alarms without a matching hash between
    /// them — the escalation ladder (docs/sync-state.md).
    heal_attempts: u32,
    /// `DivergencePersisted` has been emitted and no matching hash has
    /// arrived since. Survives `ResetState` (which zeroes the counters)
    /// so the eventual heal still announces itself and clears the
    /// advisor's sticky flag.
    escalated: bool,
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
        heal_attempts: 0,
        escalated: false,
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
                // The connect handshake (protocol v13): report what we
                // hold — epoch AND view hash — so the server can answer
                // with a merge only when our replica is identical, and
                // a snapshot otherwise. The hash is what protects a
                // restored server from our stale union when the epoch
                // counter collides.
                self.send_out(NetworkCommand::SendReliable(Box::new(
                    ServerControl::SyncStatus {
                        epoch: self.epoch(),
                        state_hash: self.state.view_hash(),
                    },
                )))
                .await;
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
            SyncCommand::ResetState => self.reset_state().await,
            SyncCommand::Shutdown => unreachable!("handled by the run loop"),
        }
    }

    async fn mutate(&mut self, mutation: Mutation) {
        let actor = self.actor;
        let user = self.user.clone();
        let ts = self.stamp();
        let is_position = matches!(mutation, Mutation::SetPlaybackPosition { .. });
        // Position ticks fire at 10Hz — trace. Everything else is rare
        // and diagnosis-worthy, so it stays visible at debug.
        if is_position {
            tracing::trace!(mutation = mutation.name(), ts = ts.0, "local mutation");
        } else {
            tracing::debug!(mutation = mutation.name(), ts = ts.0, "local mutation");
        }

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
            Mutation::SetUserSeek {
                file,
                from_millis,
                to_millis,
            } => self.state.set_seek_authority(
                actor,
                ts,
                SeekAuthority::User(dessplay_core::types::UserSeek {
                    user,
                    file,
                    event_at: ts,
                    from_millis,
                    to_millis,
                }),
            ),
            Mutation::SetPlaybackIntent { intent } => {
                self.state.set_playback_intent(actor, ts, intent)
            }
            Mutation::SetSeriesPreference {
                user,
                entry,
                pref,
                set_by,
            } => self
                .state
                .set_series_preference(actor, ts, user, entry, pref, set_by),
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
            Mutation::SetMarquee { message } => self.state.set_marquee(actor, ts, message),
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
            // With an ordered transport this should be unreachable —
            // Connected always precedes the first snapshot/merge. It
            // once fired anyway (the sim reordered frames, 2026-08-17)
            // and the resulting wedge was silent for 30s+, so shout.
            tracing::warn!("initial sync arrived while the link is down; not replaying");
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

    /// `/resync`: discard the replicated state wholesale and re-adopt
    /// the server's. The server's copy is authoritative and losslessly
    /// recoverable, so nothing local is worth preserving — including
    /// the offline buffer, whose ops were stamped against the discarded
    /// state. Local-only derivations (file availability, manual
    /// mappings) re-announce through their own paths once the fresh
    /// state lands.
    async fn reset_state(&mut self) {
        tracing::info!(
            epoch = self.epoch().0,
            discarded_ops = self.offline_buffer.len(),
            "resetting synced state on user request"
        );
        self.state = CrdtState::new();
        self.offline_buffer.clear();
        self.offline_position = None;
        self.set_epoch(Epoch(0));
        self.hash_mismatches = 0;
        // The reset IS the remedy the escalation asked for: the
        // failed-heal ladder starts over. `escalated` deliberately
        // survives, so the eventual matching hash still emits
        // DivergenceHealed and clears the advisor's sticky flag.
        self.heal_attempts = 0;
        // `last_issued` (the Lamport floor) deliberately survives:
        // pre-reset stamps live on in the server's state, and a
        // post-reset write re-issuing one of them would tie — and could
        // lose — the LWW comparison against a value it causally
        // supersedes.
        self.changed();
        // Make the reset durable now, not at the next flush tick: a
        // crash must not resurrect the discarded state from storage.
        self.flush_to_storage();
        match self.link {
            Link::Synced => {
                // The reply is a curative StateSnapshot at the server's
                // epoch (protocol v13), adopted wholesale by the
                // mid-session snapshot path below.
                self.send_out(NetworkCommand::SendReliable(Box::new(
                    ServerControl::RequestMerge,
                )))
                .await;
            }
            // Down (or still awaiting the initial sync): nothing to
            // send. The reconnect handshake covers adoption — our next
            // SyncStatus reports epoch 0 plus the empty-state hash, and
            // the server answers any mismatch with a snapshot.
            Link::Down | Link::AwaitingSync => {}
        }
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
                } else if self.link == Link::AwaitingSync {
                    // Belt-and-braces: a v13 server answers the connect
                    // handshake's epoch mismatch with a snapshot, never
                    // a merge, so this arm should be unreachable — but
                    // if a backward-epoch merge does arrive during the
                    // connect window, adopting wholesale beats the
                    // pre-fix alternative (an early return that left
                    // the link wedged in AwaitingSync forever).
                    tracing::warn!(
                        from = ours.0,
                        to = snapshot.epoch.0,
                        "backward-epoch merge during the connect window; adopting wholesale"
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
                    // Mid-session, a backward snapshot is a stale frame
                    // (e.g. a reordered datagram-era leftover): refuse.
                    // During the connect window it is the server's
                    // *authoritative* answer to our SyncStatus — after
                    // a DB restore the server's epoch legitimately
                    // rolls backwards, and refusing here is exactly the
                    // wedge that stranded every client in the 2026-08
                    // tsugumi incident. The server is authoritative:
                    // adopt, loudly.
                    if self.link != Link::AwaitingSync {
                        tracing::warn!("ignoring stale snapshot ({:?})", snapshot.epoch);
                        return;
                    }
                    tracing::warn!(
                        from = self.epoch().0,
                        to = snapshot.epoch.0,
                        "server epoch went backwards (restored from backup?); \
                         adopting its snapshot"
                    );
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
                    if self.heal_attempts > 0 || self.escalated {
                        tracing::info!(
                            failed_heals = self.heal_attempts,
                            "divergence healed: view hash matches the server again"
                        );
                        self.heal_attempts = 0;
                        self.escalated = false;
                        // Lossy for the same reason as Diverged below.
                        let _ = self.events.try_send(SyncEvent::DivergenceHealed);
                    }
                    return;
                }
                self.hash_mismatches += 1;
                if self.hash_mismatches >= 2 {
                    tracing::error!(
                        "DIVERGENCE: view hash mismatched the server twice; requesting merge"
                    );
                    self.hash_mismatches = 0;
                    self.heal_attempts += 1;
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
                    if self.heal_attempts == HEAL_ATTEMPTS_ESCALATE {
                        tracing::error!(
                            attempts = self.heal_attempts,
                            "divergence is not healing; advising a manual /resync"
                        );
                        self.escalated = true;
                        let _ = self.events.try_send(SyncEvent::DivergencePersisted);
                    }
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
    /// handshake. Consumes the client's outgoing SyncStatus and its
    /// upward merge push.
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
        let status = rig.net.recv().await.unwrap();
        assert!(
            matches!(
                &status,
                NetworkCommand::SendReliable(msg)
                    if matches!(**msg, ServerControl::SyncStatus { .. })
            ),
            "expected the connect handshake's SyncStatus, got {status:?}"
        );
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
        let status = rig.net.recv().await.unwrap();
        assert!(
            matches!(
                &status,
                NetworkCommand::SendReliable(msg)
                    if matches!(**msg, ServerControl::SyncStatus { .. })
            ),
            "expected the connect handshake's SyncStatus, got {status:?}"
        );
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
                other => {
                    assert!(
                        matches!(other, SyncEvent::StateChanged),
                        "unexpected event before Diverged: {other:?}"
                    );
                }
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

        // The connect handshake's SyncStatus goes out first...
        let status = rig.net.recv().await.unwrap();
        assert!(
            matches!(
                &status,
                NetworkCommand::SendReliable(msg)
                    if matches!(**msg, ServerControl::SyncStatus { .. })
            ),
            "expected the connect handshake's SyncStatus, got {status:?}"
        );
        // ...then the upward merge push carries the adopted server state
        // PLUS the replayed offline edit — proof unsent ops survive the
        // snapshot path.
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

    /// One full connect window, from a stored state at `client_epoch`
    /// with `offline_chats` edits buffered, against a server snapshot at
    /// `server_epoch`. Asserts the window ALWAYS ends Synced: the upward
    /// `StateMerge` push arrives at the *adopted* (server) epoch, and
    /// the client's view is the server's view plus the replayed offline
    /// edits — regardless of how the epochs compare. The backward case
    /// (`server_epoch < client_epoch`) is the DB-restore incident: the
    /// pre-fix actor refused the snapshot and wedged in AwaitingSync.
    async fn connect_window_converges(
        client_epoch: u64,
        server_epoch: u64,
        client_chats: usize,
        server_chats: usize,
        offline_chats: usize,
    ) {
        // Stored state from a previous session, at an arbitrary epoch.
        let mut initial = CrdtState::new();
        for i in 0..client_chats {
            initial.append_chat(ChatMessage {
                timestamp: SharedTimestamp(100 + i as u64),
                sender: UserId::new("old-self"),
                text: format!("stale{i}"),
            });
        }
        let epoch_cell = Arc::new(AtomicU64::new(0));
        let mut config = SyncConfig::new(
            UserId::new("baughn"),
            ActorId::session("baughn", 1),
            Arc::new(|| 1_000_000),
            Arc::clone(&epoch_cell),
        );
        config.initial = Some(StateSnapshot {
            epoch: Epoch(client_epoch),
            state: initial,
        });
        let (command_tx, command_rx) = mpsc::channel(64);
        let (net_tx, mut net_rx) = mpsc::channel(64);
        let (event_tx, _event_rx) = mpsc::channel(64);
        tokio::spawn(run(config, command_rx, net_tx, event_tx));

        // Edits made while offline: buffered, replayed after the window.
        for i in 0..offline_chats {
            command_tx
                .send(SyncCommand::Mutate(Box::new(Mutation::Chat {
                    text: format!("offline{i}"),
                })))
                .await
                .unwrap();
        }

        // Connect; the server answers the handshake with a snapshot at
        // ITS epoch — which after a DB restore can be BELOW ours.
        command_tx.send(SyncCommand::Connected).await.unwrap();
        let mut server_state = CrdtState::new();
        for i in 0..server_chats {
            server_state.append_chat(ChatMessage {
                timestamp: SharedTimestamp(200 + i as u64),
                sender: UserId::new("server"),
                text: format!("srv{i}"),
            });
        }
        let server_view = server_state.view();
        command_tx
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateSnapshot(StateSnapshot {
                    epoch: Epoch(server_epoch),
                    state: server_state,
                })),
                via_datagram: false,
            })
            .await
            .unwrap();

        // The window must end Synced; the upward StateMerge push is the
        // observable proof (other sends — the handshake's own status
        // message — may precede it).
        let push = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                match net_rx.recv().await {
                    Some(NetworkCommand::SendReliable(msg))
                        if matches!(*msg, ServerControl::StateMerge(_)) =>
                    {
                        let ServerControl::StateMerge(snapshot) = *msg else {
                            unreachable!()
                        };
                        break snapshot;
                    }
                    Some(_) => continue,
                    None => panic!("sync actor gone"),
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "connect window never converged: no upward merge after the \
                 snapshot (client epoch {client_epoch}, server epoch \
                 {server_epoch}) — the link is wedged in AwaitingSync"
            )
        });

        // The push rides the adopted epoch — even a backward one.
        assert_eq!(
            push.epoch,
            Epoch(server_epoch),
            "upward merge must carry the adopted (server) epoch"
        );
        assert_eq!(epoch_cell.load(Ordering::SeqCst), server_epoch);

        // Client view == server view + replayed offline edits; the
        // stale pre-connect chats are gone (snapshot adoption replaces
        // wholesale). Compared as multisets: GList interleaving may
        // order independently-authored messages either way.
        let mut expected: Vec<String> = server_view
            .chat
            .iter()
            .map(|m| m.text.clone())
            .chain((0..offline_chats).map(|i| format!("offline{i}")))
            .collect();
        expected.sort();
        let (tx, rx) = oneshot::channel();
        command_tx.send(SyncCommand::GetView(tx)).await.unwrap();
        let view = rx.await.unwrap();
        let mut got: Vec<String> = view.chat.iter().map(|m| m.text.clone()).collect();
        got.sort();
        assert_eq!(
            got, expected,
            "client view must be the server view plus the replayed offline edits"
        );
        let mut pushed: Vec<String> = push
            .state
            .view()
            .chat
            .iter()
            .map(|m| m.text.clone())
            .collect();
        pushed.sort();
        assert_eq!(
            pushed, expected,
            "the upward merge must carry the replayed offline edits"
        );
    }

    /// Drain everything currently queued on the events channel.
    fn drained_events(rig: &mut Rig) -> Vec<SyncEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rig.events.try_recv() {
            events.push(event);
        }
        events
    }

    fn count_persisted(events: &[SyncEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, SyncEvent::DivergencePersisted))
            .count()
    }

    fn count_healed(events: &[SyncEvent]) -> usize {
        events
            .iter()
            .filter(|e| matches!(e, SyncEvent::DivergenceHealed))
            .count()
    }

    /// One failed heal cycle: two consecutive mismatching `StateHash`
    /// frames (the alarm threshold), consuming the `RequestMerge` the
    /// alarm sends. Waiting on the net channel also serializes with the
    /// actor, so the events it emitted are drainable on return.
    async fn fail_one_heal(rig: &mut Rig) {
        let bogus = ServerControl::StateHash {
            epoch: Epoch(0),
            hash: [0xAB; 32],
        };
        for _ in 0..2 {
            rig.commands
                .send(SyncCommand::Server {
                    msg: Box::new(bogus.clone()),
                    via_datagram: false,
                })
                .await
                .unwrap();
        }
        let cmd = rig.net.recv().await.unwrap();
        assert!(
            matches!(
                &cmd,
                NetworkCommand::SendReliable(msg)
                    if matches!(**msg, ServerControl::RequestMerge)
            ),
            "expected the alarm's RequestMerge, got {cmd:?}"
        );
    }

    /// A matching `StateHash` for an empty replica at epoch 0.
    async fn send_honest_hash(rig: &Rig) {
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateHash {
                    epoch: Epoch(0),
                    hash: CrdtState::new().view_hash(),
                }),
                via_datagram: false,
            })
            .await
            .unwrap();
        // GetView round-trips through the actor, so the hash frame has
        // been fully handled (and its events emitted) on return.
        let _ = view_of(rig).await;
    }

    /// The escalation ladder (docs/sync-state.md, Divergence Alarm):
    /// the first two failed heals stay silent toward the user — the
    /// second can be the one-cycle heal race (an op sent between
    /// RequestMerge and the curative snapshot), not a real failure —
    /// the third emits `DivergencePersisted`, exactly once; the honest
    /// hash afterwards emits `DivergenceHealed`.
    #[tokio::test(start_paused = true)]
    async fn divergence_escalates_only_after_three_failed_heals() {
        let mut rig = rig();
        go_online(&mut rig).await;

        for _ in 0..2 {
            fail_one_heal(&mut rig).await;
        }
        let early = drained_events(&mut rig);
        assert_eq!(
            count_persisted(&early),
            0,
            "escalated before the third failed heal: {early:?}"
        );

        fail_one_heal(&mut rig).await;
        let third = drained_events(&mut rig);
        assert_eq!(
            count_persisted(&third),
            1,
            "the third failed heal must escalate exactly once: {third:?}"
        );

        // Divergence keeps failing to heal: no re-escalation (the
        // advisor flag is sticky; the chat line must not repeat every
        // hash period).
        fail_one_heal(&mut rig).await;
        let fourth = drained_events(&mut rig);
        assert_eq!(
            count_persisted(&fourth),
            0,
            "a later failed heal must not re-escalate: {fourth:?}"
        );

        // The honest hash ends it.
        send_honest_hash(&rig).await;
        let after = drained_events(&mut rig);
        assert_eq!(
            count_healed(&after),
            1,
            "a matching hash after failed heals must emit DivergenceHealed: {after:?}"
        );
    }

    /// A matching hash between failed heals resets the ladder: the
    /// count is of *consecutive* failures, so two failures, a heal, and
    /// two more failures never escalate — only a third consecutive one
    /// does.
    #[tokio::test(start_paused = true)]
    async fn a_matching_hash_resets_the_heal_ladder() {
        let mut rig = rig();
        go_online(&mut rig).await;

        for _ in 0..2 {
            fail_one_heal(&mut rig).await;
        }
        send_honest_hash(&rig).await;
        let healed = drained_events(&mut rig);
        assert_eq!(
            count_healed(&healed),
            1,
            "healing after failed attempts must announce itself: {healed:?}"
        );

        for _ in 0..2 {
            fail_one_heal(&mut rig).await;
        }
        let events = drained_events(&mut rig);
        assert_eq!(
            count_persisted(&events),
            0,
            "the ladder must restart after a heal: {events:?}"
        );
        fail_one_heal(&mut rig).await;
        let events = drained_events(&mut rig);
        assert_eq!(count_persisted(&events), 1);
    }

    /// `ResetState` discards the replica wholesale — fresh state, epoch
    /// 0, `RequestMerge` out for the curative snapshot — but keeps the
    /// Lamport floor: a post-reset write must out-stamp every pre-reset
    /// stamp we ever observed, or the server's copy of superseded state
    /// wins LWW ties when it merges back.
    #[tokio::test(start_paused = true)]
    async fn reset_state_discards_state_and_keeps_the_lamport_floor() {
        let mut rig = rig();
        go_online(&mut rig).await;

        // A remote write stamped far in our future.
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
        assert_eq!(view_of(&rig).await.now_playing, Some(hash(1)));

        rig.commands.send(SyncCommand::ResetState).await.unwrap();

        // Wholesale discard: fresh state, epoch 0, and a RequestMerge
        // out (the reply is the server's curative snapshot).
        assert_eq!(view_of(&rig).await.now_playing, None);
        let (tx, rx) = oneshot::channel();
        rig.commands.send(SyncCommand::GetEpoch(tx)).await.unwrap();
        assert_eq!(rx.await.unwrap(), Epoch(0));
        let cmd = rig.net.recv().await.unwrap();
        assert!(
            matches!(
                &cmd,
                NetworkCommand::SendReliable(msg)
                    if matches!(**msg, ServerControl::RequestMerge)
            ),
            "expected ResetState's RequestMerge, got {cmd:?}"
        );

        // The Lamport floor survives: our post-reset write dominates the
        // pre-reset 5_000_000 stamp when the server's state merges back.
        rig.commands
            .send(SyncCommand::Mutate(Box::new(Mutation::SetNowPlaying {
                file: Some(hash(2)),
            })))
            .await
            .unwrap();
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateMerge(StateSnapshot {
                    epoch: Epoch(0),
                    state: origin,
                })),
                via_datagram: false,
            })
            .await
            .unwrap();
        assert_eq!(
            view_of(&rig).await.now_playing,
            Some(hash(2)),
            "post-reset write lost an LWW tie: the Lamport floor did not survive ResetState"
        );
    }

    /// `ResetState` while the link is down sends nothing; the ordinary
    /// reconnect handshake heals it — SyncStatus reports epoch 0 plus
    /// the empty-state hash, the server answers with a snapshot, and
    /// the discarded offline buffer stays discarded (the upward push
    /// carries the adopted state only).
    #[tokio::test(start_paused = true)]
    async fn reset_state_while_down_is_healed_by_the_reconnect_handshake() {
        let mut rig = rig();
        go_online(&mut rig).await;
        rig.commands.send(SyncCommand::Disconnected).await.unwrap();
        // An offline edit sits in the buffer when the user resets.
        rig.commands
            .send(SyncCommand::Mutate(Box::new(Mutation::Chat {
                text: "pre-reset".into(),
            })))
            .await
            .unwrap();

        rig.commands.send(SyncCommand::ResetState).await.unwrap();
        assert_eq!(view_of(&rig).await.chat.len(), 0);
        tokio::task::yield_now().await;
        assert!(rig.net.try_recv().is_err(), "nothing sent while down");

        // Reconnect: the handshake must advertise the reset (epoch 0,
        // empty-state hash), so the server answers with a snapshot.
        rig.commands.send(SyncCommand::Connected).await.unwrap();
        let status = rig.net.recv().await.unwrap();
        let NetworkCommand::SendReliable(msg) = status else {
            panic!("expected reliable SyncStatus, got {status:?}");
        };
        let ServerControl::SyncStatus { epoch, state_hash } = *msg else {
            panic!("expected SyncStatus, got {msg:?}");
        };
        assert_eq!(epoch, Epoch(0));
        assert_eq!(state_hash, CrdtState::new().view_hash());

        // The snapshot is adopted; the upward push carries it at the
        // adopted epoch and does NOT resurrect the pre-reset edit.
        let mut server = CrdtState::new();
        server.set_now_playing(ActorId::SERVER, SharedTimestamp(10), Some(hash(2)));
        rig.commands
            .send(SyncCommand::Server {
                msg: Box::new(ServerControl::StateSnapshot(StateSnapshot {
                    epoch: Epoch(7),
                    state: server,
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
        assert_eq!(snapshot.epoch, Epoch(7));
        let pushed = snapshot.state.view();
        assert_eq!(pushed.now_playing, Some(hash(2)));
        assert_eq!(
            pushed.chat.len(),
            0,
            "the pre-reset offline edit must stay discarded"
        );
        let view = view_of(&rig).await;
        assert_eq!(view.now_playing, Some(hash(2)));
        assert_eq!(view.chat.len(), 0);
    }

    /// `ResetState` also restarts the heal ladder: the reset IS the
    /// remedy the escalation asked for, so the count of failed heals
    /// starts over.
    #[tokio::test(start_paused = true)]
    async fn reset_state_resets_the_heal_ladder() {
        let mut rig = rig();
        go_online(&mut rig).await;

        for _ in 0..2 {
            fail_one_heal(&mut rig).await;
        }
        rig.commands.send(SyncCommand::ResetState).await.unwrap();
        let cmd = rig.net.recv().await.unwrap();
        assert!(
            matches!(
                &cmd,
                NetworkCommand::SendReliable(msg)
                    if matches!(**msg, ServerControl::RequestMerge)
            ),
            "expected ResetState's RequestMerge, got {cmd:?}"
        );
        drained_events(&mut rig);

        for _ in 0..2 {
            fail_one_heal(&mut rig).await;
        }
        let events = drained_events(&mut rig);
        assert_eq!(
            count_persisted(&events),
            0,
            "the reset must restart the ladder: {events:?}"
        );
        fail_one_heal(&mut rig).await;
        let events = drained_events(&mut rig);
        assert_eq!(count_persisted(&events), 1);
    }

    /// After an escalation, a reset followed by a matching hash still
    /// emits `DivergenceHealed` — without this the advisor's sticky
    /// "run /resync" flag would never clear after the /resync worked.
    #[tokio::test(start_paused = true)]
    async fn healing_after_a_reset_still_emits_divergence_healed() {
        let mut rig = rig();
        go_online(&mut rig).await;

        for _ in 0..3 {
            fail_one_heal(&mut rig).await;
        }
        let events = drained_events(&mut rig);
        assert_eq!(count_persisted(&events), 1);

        rig.commands.send(SyncCommand::ResetState).await.unwrap();
        let _ = rig.net.recv().await.unwrap(); // the reset's RequestMerge
        drained_events(&mut rig);

        // The reset replica is empty, so the honest hash now matches:
        // the heal must be announced even though the reset zeroed the
        // failed-heal counter.
        send_honest_hash(&rig).await;
        let events = drained_events(&mut rig);
        assert_eq!(
            count_healed(&events),
            1,
            "the post-reset heal must clear the escalation: {events:?}"
        );
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            cases: 32,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// The connect window always converges — whatever the epoch
        /// relation. The name calls out the case that broke in
        /// production (the DB-restore incident): a snapshot at an epoch
        /// BELOW the client's, arriving while AwaitingSync, must be
        /// adopted, the offline buffer replayed, and the upward merge
        /// pushed at the adopted epoch.
        #[test]
        fn awaiting_sync_adopts_a_backward_epoch_snapshot(
            client_epoch in 0u64..6,
            server_epoch in 0u64..6,
            client_chats in 0usize..4,
            server_chats in 0usize..4,
            offline_chats in 0usize..4,
        ) {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            runtime.block_on(async move {
                tokio::time::pause();
                connect_window_converges(
                    client_epoch,
                    server_epoch,
                    client_chats,
                    server_chats,
                    offline_chats,
                )
                .await;
            });
        }
    }
}
