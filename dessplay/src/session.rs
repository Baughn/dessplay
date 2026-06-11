//! Session policy: the synchronous decision core between the synced
//! state and the player actor.
//!
//! `run_interactive` (and the multi-client harness) feed it three kinds
//! of input — fresh state views, player actor outputs, and finished
//! file resolutions — and it answers with [`Directive`]s: player
//! commands, state mutations, EOF reports. Like [`crate::ui::app::Ui`],
//! it is deliberately synchronous and channel-free so whole-session
//! tests can drive it without threads or timing.
//!
//! The rules implemented here are design.md's Playback Rules:
//!
//! - The player runs iff the *derived* playback state says so; the
//!   wiring re-asserts [`PlayerCommand::SetPlaying`] on every state
//!   change and the actor dedups (observe-and-correct — a blocked
//!   user's unpause attempt is reverted by this round trip).
//! - A user pause writes both the manual override (so others see who)
//!   and the intent latch; an unpause attempt clears the override and
//!   sets intent Playing ("you tried!").
//! - A user seek takes seek authority and publishes the position;
//!   remote authority samples become [`PlayerCommand::SyncTo`].
//! - Files are verified (ed2k) before they can play: now-playing is
//!   only loaded once the matcher returns [`Resolution::Verified`].

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use dessplay_core::derive;
use dessplay_core::net::PeerInfo;
use dessplay_core::state::StateView;
use dessplay_core::types::{
    Ed2kHash, FileAvailability, ManualState, PlaybackIntent, SeekAuthority, UserId,
};

use crate::actors::player::{PlayerCommand, PlayerOutput};
use crate::actors::sync::Mutation;
use crate::matcher::Resolution;

/// One instruction to the async shell around the wiring.
#[derive(Debug)]
pub enum Directive {
    /// Send to the player actor. The shell spawns the player lazily on
    /// the first `Load`; other player directives before that are
    /// dropped (there is nothing to control yet).
    Player(PlayerCommand),
    /// Apply a state mutation.
    Mutate(Mutation),
    /// Report end-of-file to the server (it owns the transition).
    ReportEof(Ed2kHash),
    /// Resolve a playlist entry against the media roots (blocking IO —
    /// the shell runs the matcher and calls
    /// [`PlayerWiring::on_resolved`] with the outcome).
    Resolve {
        /// Playlist key to verify against.
        file: Ed2kHash,
        /// Filename to search for.
        filename: String,
    },
    /// A subtitle line for the UI's subtitle pane.
    Subtitle(String),
}

/// The session's player-side policy state.
pub struct PlayerWiring {
    me: UserId,
    resolved: HashMap<Ed2kHash, Resolution>,
    pending_resolve: HashSet<Ed2kHash>,
    /// What we've told the player to load.
    loaded: Option<Ed2kHash>,
    /// Last authority sample forwarded as SyncTo (dedup).
    last_synced: Option<(UserId, dessplay_core::types::PlaybackPosition)>,
    /// Chat messages already shown as OSD.
    chat_seen: Option<usize>,
}

impl PlayerWiring {
    /// A fresh wiring for `me`.
    pub fn new(me: UserId) -> Self {
        PlayerWiring {
            me,
            resolved: HashMap::new(),
            pending_resolve: HashSet::new(),
            loaded: None,
            last_synced: None,
            chat_seen: None,
        }
    }

    /// We just hashed and added this local file ourselves: skip the
    /// matcher, it is verified by construction.
    pub fn note_local_file(&mut self, file: Ed2kHash, path: PathBuf) -> Vec<Directive> {
        self.pending_resolve.remove(&file);
        self.resolved.insert(file, Resolution::Verified(path));
        vec![Directive::Mutate(Mutation::SetFileAvailability {
            file,
            availability: FileAvailability::Ready,
        })]
    }

    /// React to a fresh state view + peer list.
    pub fn on_state(&mut self, view: &StateView, peers: &[PeerInfo]) -> Vec<Directive> {
        let mut out = Vec::new();

        // Kick the matcher for entries we haven't looked for yet.
        // Watched history is skipped (no point hashing gigabytes of
        // already-seen files) unless it becomes now-playing again.
        for entry in &view.playlist {
            let watched = view.watched.get(&entry.hash) == Some(&true)
                && view.now_playing != Some(entry.hash);
            if watched
                || self.resolved.contains_key(&entry.hash)
                || self.pending_resolve.contains(&entry.hash)
            {
                continue;
            }
            self.pending_resolve.insert(entry.hash);
            out.push(Directive::Resolve {
                file: entry.hash,
                filename: entry.state.filename.clone(),
            });
        }

        // Load now-playing once it has a verified local copy.
        if let Some(file) = view.now_playing
            && self.loaded != Some(file)
            && let Some(Resolution::Verified(path)) = self.resolved.get(&file)
        {
            self.loaded = Some(file);
            out.push(Directive::Player(PlayerCommand::Load {
                file,
                path: path.clone(),
            }));
        }

        // Re-assert the derived playback state; the actor dedups.
        let active = derive::playback_active(view, peers);
        if self.loaded.is_some() {
            out.push(Directive::Player(PlayerCommand::SetPlaying(active)));
        }

        // Follow the seek authority's position (never our own).
        if self.loaded.is_some()
            && let Some(SeekAuthority::User(authority)) = &view.seek_authority
            && *authority != self.me
            && let Some(position) = view.playback_position.get(authority)
        {
            let sample = (authority.clone(), *position);
            if self.last_synced.as_ref() != Some(&sample) {
                self.last_synced = Some(sample);
                out.push(Directive::Player(PlayerCommand::SyncTo {
                    position_millis: position.position_millis,
                    timestamp: position.timestamp,
                    playing: active,
                }));
            }
        }

        // New chat messages go to the OSD. The first view's backlog is
        // history, not news.
        match self.chat_seen {
            None => self.chat_seen = Some(view.chat.len()),
            Some(seen) => {
                for msg in view.chat.iter().skip(seen) {
                    out.push(Directive::Player(PlayerCommand::ShowOsd(format!(
                        "{}: {}",
                        msg.sender, msg.text
                    ))));
                }
                self.chat_seen = Some(view.chat.len());
            }
        }

        out
    }

    /// React to a finished file resolution.
    pub fn on_resolved(
        &mut self,
        file: Ed2kHash,
        resolution: Resolution,
        view: &StateView,
        peers: &[PeerInfo],
    ) -> Vec<Directive> {
        self.pending_resolve.remove(&file);
        let availability = match &resolution {
            Resolution::Verified(_) => FileAvailability::Ready,
            Resolution::HashMismatch(path) => {
                tracing::info!(path = %path.display(), "local copy has different contents");
                FileAvailability::Missing
            }
            Resolution::NotFound => FileAvailability::Missing,
        };
        self.resolved.insert(file, resolution);
        let mut out = vec![Directive::Mutate(Mutation::SetFileAvailability {
            file,
            availability,
        })];
        // If this was what the session is waiting on, load it now.
        if view.now_playing == Some(file)
            && self.loaded != Some(file)
            && let Some(Resolution::Verified(path)) = self.resolved.get(&file)
        {
            self.loaded = Some(file);
            out.push(Directive::Player(PlayerCommand::Load {
                file,
                path: path.clone(),
            }));
            out.push(Directive::Player(PlayerCommand::SetPlaying(
                derive::playback_active(view, peers),
            )));
        }
        out
    }

    /// React to a player actor output.
    pub fn on_player(&mut self, output: PlayerOutput, view: &StateView) -> Vec<Directive> {
        match output {
            PlayerOutput::UserPaused => vec![
                // Both writes: the override shows *who* is blocking,
                // the latch keeps everyone paused even if they leave.
                Directive::Mutate(Mutation::SetManualOverride {
                    user: self.me.clone(),
                    state: Some(ManualState::Paused),
                }),
                Directive::Mutate(Mutation::SetPlaybackIntent {
                    intent: PlaybackIntent::Paused,
                }),
            ],
            PlayerOutput::UserUnpaused => vec![
                // "You tried!" — clear our own block and latch Playing;
                // playback starts when the last blocker clears.
                Directive::Mutate(Mutation::SetManualOverride {
                    user: self.me.clone(),
                    state: None,
                }),
                Directive::Mutate(Mutation::SetPlaybackIntent {
                    intent: PlaybackIntent::Playing,
                }),
            ],
            PlayerOutput::UserSeeked { position_millis } => vec![
                Directive::Mutate(Mutation::SetSeekAuthority {
                    authority: SeekAuthority::User(self.me.clone()),
                }),
                Directive::Mutate(Mutation::SetPlaybackPosition { position_millis }),
            ],
            PlayerOutput::PositionTick { position_millis } => {
                vec![Directive::Mutate(Mutation::SetPlaybackPosition {
                    position_millis,
                })]
            }
            PlayerOutput::DurationKnown {
                file,
                duration_millis,
            } => {
                // Backfill only: the adder's probe is authoritative.
                let needs_it = view
                    .playlist
                    .iter()
                    .any(|entry| entry.hash == file && entry.state.duration_millis.is_none());
                if needs_it {
                    vec![Directive::Mutate(Mutation::SetPlaylistDuration {
                        hash: file,
                        duration_millis,
                    })]
                } else {
                    vec![]
                }
            }
            PlayerOutput::SubtitleLine(line) => vec![Directive::Subtitle(line)],
            PlayerOutput::Eof { file } => vec![Directive::ReportEof(file)],
            PlayerOutput::FatalCrash => vec![
                Directive::Mutate(Mutation::SetPlaybackIntent {
                    intent: PlaybackIntent::Paused,
                }),
                Directive::Mutate(Mutation::Chat {
                    text: "[my player crashed twice in a row; pausing]".into(),
                }),
            ],
        }
    }
}

/// The async half of the session: owns the channels around
/// [`PlayerWiring`] and executes its [`Directive`]s. Shared between
/// `run_interactive` and the multi-client harness — the caller runs the
/// select loop (it knows about its UI), the shell does everything else.
pub struct SessionShell<F: crate::player::PlayerFactory> {
    wiring: PlayerWiring,
    /// Taken on the first `Load`, when the player actor spawns.
    factory: Option<F>,
    clock: crate::actors::network::Clock,
    clock_offset: i64,
    player: Option<tokio::sync::mpsc::Sender<PlayerCommand>>,
    player_out_tx: tokio::sync::mpsc::Sender<PlayerOutput>,
    /// Player actor outputs; feed each into [`Self::on_player_output`].
    pub player_outputs: tokio::sync::mpsc::Receiver<PlayerOutput>,
    res_tx: tokio::sync::mpsc::Sender<(Ed2kHash, Resolution)>,
    /// Finished matcher runs; feed each into [`Self::on_resolution`].
    pub resolutions: tokio::sync::mpsc::Receiver<(Ed2kHash, Resolution)>,
    /// Media roots for the matcher (update when settings change).
    pub media_roots: Vec<PathBuf>,
    /// Manual mappings (hash → user-picked path); checked before the
    /// matcher and exempt from hash verification by design.
    manual: HashMap<Ed2kHash, PathBuf>,
    sync: tokio::sync::mpsc::Sender<crate::actors::sync::SyncCommand>,
    network: tokio::sync::mpsc::Sender<crate::actors::network::NetworkCommand>,
}

impl<F: crate::player::PlayerFactory> SessionShell<F> {
    /// Build a shell. Nothing runs until directives start flowing.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        me: UserId,
        factory: F,
        clock: crate::actors::network::Clock,
        media_roots: Vec<PathBuf>,
        manual: HashMap<Ed2kHash, PathBuf>,
        sync: tokio::sync::mpsc::Sender<crate::actors::sync::SyncCommand>,
        network: tokio::sync::mpsc::Sender<crate::actors::network::NetworkCommand>,
    ) -> Self {
        let (player_out_tx, player_outputs) = tokio::sync::mpsc::channel(256);
        let (res_tx, resolutions) = tokio::sync::mpsc::channel(64);
        SessionShell {
            wiring: PlayerWiring::new(me),
            factory: Some(factory),
            clock,
            clock_offset: 0,
            player: None,
            player_out_tx,
            player_outputs,
            res_tx,
            resolutions,
            media_roots,
            manual,
            sync,
            network,
        }
    }

    /// A fresh state view arrived. Returns subtitle lines for the UI.
    pub async fn on_state(&mut self, view: &StateView, peers: &[PeerInfo]) -> Vec<String> {
        let directives = self.wiring.on_state(view, peers);
        self.execute(directives).await
    }

    /// The player actor reported something.
    pub async fn on_player_output(
        &mut self,
        output: PlayerOutput,
        view: &StateView,
    ) -> Vec<String> {
        let directives = self.wiring.on_player(output, view);
        self.execute(directives).await
    }

    /// A matcher run finished.
    pub async fn on_resolution(
        &mut self,
        file: Ed2kHash,
        resolution: Resolution,
        view: &StateView,
        peers: &[PeerInfo],
    ) -> Vec<String> {
        let directives = self.wiring.on_resolved(file, resolution, view, peers);
        self.execute(directives).await
    }

    /// We hashed and added this file ourselves.
    pub async fn note_local_file(&mut self, file: Ed2kHash, path: PathBuf) -> Vec<String> {
        let directives = self.wiring.note_local_file(file, path);
        self.execute(directives).await
    }

    /// Forward a clock-sync offset to the player layer.
    pub async fn set_clock_offset(&mut self, offset_millis: i64) {
        self.clock_offset = offset_millis;
        if let Some(player) = &self.player {
            let _ = player.send(PlayerCommand::ClockOffset(offset_millis)).await;
        }
    }

    /// Quit the player (if one ever spawned).
    pub async fn shutdown(&mut self) {
        if let Some(player) = &self.player {
            let _ = player.send(PlayerCommand::Shutdown).await;
        }
    }

    async fn execute(&mut self, directives: Vec<Directive>) -> Vec<String> {
        let mut subtitles = Vec::new();
        for directive in directives {
            match directive {
                Directive::Player(cmd) => {
                    if self.player.is_none() {
                        // The player spawns lazily on the first load;
                        // there is nothing to control before that.
                        if matches!(cmd, PlayerCommand::Load { .. }) {
                            self.spawn_player().await;
                        } else {
                            continue;
                        }
                    }
                    if let Some(player) = &self.player {
                        let _ = player.send(cmd).await;
                    }
                }
                Directive::Mutate(mutation) => {
                    let _ = self
                        .sync
                        .send(crate::actors::sync::SyncCommand::Mutate(Box::new(mutation)))
                        .await;
                }
                Directive::ReportEof(file) => {
                    let _ = self
                        .network
                        .send(crate::actors::network::NetworkCommand::SendReliable(
                            Box::new(dessplay_core::net::ServerControl::EofReached { file }),
                        ))
                        .await;
                }
                Directive::Resolve { file, filename } => {
                    self.spawn_resolve(file, filename);
                }
                Directive::Subtitle(line) => subtitles.push(line),
            }
        }
        subtitles
    }

    async fn spawn_player(&mut self) {
        let Some(factory) = self.factory.take() else {
            return; // already spawned and gone (fatal launch failure)
        };
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(crate::actors::player::run(
            factory,
            std::sync::Arc::clone(&self.clock),
            rx,
            self.player_out_tx.clone(),
        ));
        let _ = tx.send(PlayerCommand::ClockOffset(self.clock_offset)).await;
        self.player = Some(tx);
    }

    fn spawn_resolve(&self, file: Ed2kHash, filename: String) {
        // Manual mappings skip the matcher *and* hash verification —
        // the user explicitly chose that file (design.md).
        if let Some(path) = self.manual.get(&file) {
            if path.is_file() {
                let _ = self
                    .res_tx
                    .try_send((file, Resolution::Verified(path.clone())));
                return;
            }
            tracing::info!(path = %path.display(), "manual mapping points at nothing; re-matching");
        }
        let roots = self.media_roots.clone();
        let res_tx = self.res_tx.clone();
        tokio::task::spawn_blocking(move || {
            let started = std::time::Instant::now();
            let resolution = crate::matcher::resolve(&filename, &roots, file);
            tracing::debug!(
                filename,
                elapsed_ms = started.elapsed().as_millis() as u64,
                ?resolution,
                "file resolution finished"
            );
            let _ = res_tx.blocking_send((file, resolution));
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::net::{Presence, Role};
    use dessplay_core::playlist::NewPlaylistEntry;
    use dessplay_core::state::CrdtState;
    use dessplay_core::types::{ActorId, PlaybackPosition, SharedTimestamp};

    use super::*;

    const A: ActorId = ActorId(1);

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn ts(t: u64) -> SharedTimestamp {
        SharedTimestamp(t)
    }

    fn me() -> UserId {
        UserId::new("kim")
    }

    fn peer(name: &str) -> PeerInfo {
        PeerInfo {
            username: UserId::new(name),
            role: Role::Interactive,
            presence: Presence::Present,
            addresses: vec![],
            connected_since: 0,
        }
    }

    fn entry(i: u8, filename: &str) -> NewPlaylistEntry {
        NewPlaylistEntry {
            hash: hash(i),
            added_by: UserId::new("baughn"),
            filename: filename.into(),
            size_bytes: 1000,
            duration_millis: None,
        }
    }

    /// State with one playlist entry, now-playing, intent Playing.
    fn playing_state() -> CrdtState {
        let mut state = CrdtState::new();
        state.push_playlist_entry(A, ts(1), entry(1, "ep1.mkv"));
        state.set_now_playing(A, ts(2), Some(hash(1)));
        state.set_playback_intent(A, ts(3), dessplay_core::types::PlaybackIntent::Playing);
        state
    }

    fn player_cmds(directives: &[Directive]) -> Vec<&PlayerCommand> {
        directives
            .iter()
            .filter_map(|d| match d {
                Directive::Player(cmd) => Some(cmd),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn unresolved_entries_trigger_one_resolve_each() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        let first = wiring.on_state(&view, &[peer("kim")]);
        let resolves: Vec<_> = first
            .iter()
            .filter(|d| matches!(d, Directive::Resolve { .. }))
            .collect();
        assert_eq!(resolves.len(), 1);
        // Same view again: the resolve is pending, not re-issued.
        let second = wiring.on_state(&view, &[peer("kim")]);
        assert!(
            !second
                .iter()
                .any(|d| matches!(d, Directive::Resolve { .. })),
            "resolve must not be re-issued while pending"
        );
    }

    #[test]
    fn watched_history_is_not_resolved_unless_now_playing() {
        let mut state = playing_state();
        state.push_playlist_entry(A, ts(4), entry(2, "old.mkv"));
        state.set_watched(A, ts(5), hash(2), true);
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_state(&state.view(), &[peer("kim")]);
        let resolves: Vec<_> = directives
            .iter()
            .filter_map(|d| match d {
                Directive::Resolve { file, .. } => Some(*file),
                _ => None,
            })
            .collect();
        assert_eq!(resolves, vec![hash(1)], "watched history must be skipped");
    }

    #[test]
    fn verified_resolution_loads_now_playing_and_reports_ready() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        wiring.on_state(&view, &[peer("kim")]);
        let directives = wiring.on_resolved(
            hash(1),
            Resolution::Verified("/media/ep1.mkv".into()),
            &view,
            &[peer("kim")],
        );
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetFileAvailability {
                availability: FileAvailability::Ready,
                ..
            })
        )));
        assert!(
            player_cmds(&directives)
                .iter()
                .any(|cmd| matches!(cmd, PlayerCommand::Load { .. })),
            "verified now-playing must load"
        );
    }

    #[test]
    fn mismatch_reports_missing_and_never_loads() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        wiring.on_state(&view, &[peer("kim")]);
        let directives = wiring.on_resolved(
            hash(1),
            Resolution::HashMismatch("/media/ep1.mkv".into()),
            &view,
            &[peer("kim")],
        );
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetFileAvailability {
                availability: FileAvailability::Missing,
                ..
            })
        )));
        assert!(
            player_cmds(&directives).is_empty(),
            "mismatch must not load"
        );
    }

    #[test]
    fn derived_playing_state_is_reasserted_once_loaded() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        // Not loaded yet: no SetPlaying.
        let before = wiring.on_state(&view, &[peer("kim")]);
        assert!(
            !player_cmds(&before)
                .iter()
                .any(|cmd| matches!(cmd, PlayerCommand::SetPlaying(_)))
        );
        wiring.on_resolved(
            hash(1),
            Resolution::Verified("/media/ep1.mkv".into()),
            &view,
            &[peer("kim")],
        );
        let after = wiring.on_state(&view, &[peer("kim")]);
        assert!(
            player_cmds(&after)
                .iter()
                .any(|cmd| matches!(cmd, PlayerCommand::SetPlaying(true))),
            "intent Playing + no blockers must reach the player"
        );
    }

    #[test]
    fn authority_samples_become_sync_to_but_never_our_own() {
        let mut state = playing_state();
        state.set_seek_authority(
            A,
            ts(10),
            dessplay_core::types::SeekAuthority::User(UserId::new("baughn")),
        );
        state.set_playback_position(
            A,
            ts(11),
            UserId::new("baughn"),
            PlaybackPosition {
                position_millis: 60_000,
                timestamp: ts(11),
            },
        );
        let mut wiring = PlayerWiring::new(me());
        let view = state.view();
        wiring.on_state(&view, &[peer("kim"), peer("baughn")]);
        wiring.on_resolved(
            hash(1),
            Resolution::Verified("/media/ep1.mkv".into()),
            &view,
            &[peer("kim"), peer("baughn")],
        );
        let directives = wiring.on_state(&view, &[peer("kim"), peer("baughn")]);
        assert!(
            player_cmds(&directives).iter().any(|cmd| matches!(
                cmd,
                PlayerCommand::SyncTo {
                    position_millis: 60_000,
                    ..
                }
            )),
            "remote authority position must be followed"
        );
        // The same sample again: deduped.
        let again = wiring.on_state(&view, &[peer("kim"), peer("baughn")]);
        assert!(
            !player_cmds(&again)
                .iter()
                .any(|cmd| matches!(cmd, PlayerCommand::SyncTo { .. }))
        );

        // Authority moves to us: our own samples must not echo back.
        state.set_seek_authority(A, ts(20), dessplay_core::types::SeekAuthority::User(me()));
        state.set_playback_position(
            A,
            ts(21),
            me(),
            PlaybackPosition {
                position_millis: 70_000,
                timestamp: ts(21),
            },
        );
        let directives = wiring.on_state(&state.view(), &[peer("kim"), peer("baughn")]);
        assert!(
            !player_cmds(&directives)
                .iter()
                .any(|cmd| matches!(cmd, PlayerCommand::SyncTo { .. })),
            "we never sync to ourselves"
        );
    }

    #[test]
    fn chat_backlog_is_history_but_new_messages_are_osd() {
        let mut state = playing_state();
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: ts(5),
            sender: UserId::new("baughn"),
            text: "old".into(),
        });
        let mut wiring = PlayerWiring::new(me());
        let first = wiring.on_state(&state.view(), &[peer("kim")]);
        assert!(
            !player_cmds(&first)
                .iter()
                .any(|cmd| matches!(cmd, PlayerCommand::ShowOsd(_))),
            "the backlog is not news"
        );
        state.append_chat(dessplay_core::types::ChatMessage {
            timestamp: ts(6),
            sender: UserId::new("baughn"),
            text: "hello!".into(),
        });
        let second = wiring.on_state(&state.view(), &[peer("kim")]);
        let osd: Vec<_> = player_cmds(&second)
            .into_iter()
            .filter_map(|cmd| match cmd {
                PlayerCommand::ShowOsd(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(osd, vec!["baughn: hello!"]);
    }

    #[test]
    fn user_pause_writes_override_and_latch() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        let directives = wiring.on_player(PlayerOutput::UserPaused, &view);
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetManualOverride {
                state: Some(ManualState::Paused),
                ..
            })
        )));
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused
            })
        )));
    }

    #[test]
    fn user_unpause_clears_override_and_latches_playing() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        let directives = wiring.on_player(PlayerOutput::UserUnpaused, &view);
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetManualOverride { state: None, .. })
        )));
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Playing
            })
        )));
    }

    #[test]
    fn user_seek_takes_authority_and_publishes_position() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        let directives = wiring.on_player(
            PlayerOutput::UserSeeked {
                position_millis: 90_000,
            },
            &view,
        );
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetSeekAuthority {
                authority: SeekAuthority::User(_)
            })
        )));
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetPlaybackPosition {
                position_millis: 90_000
            })
        )));
    }

    #[test]
    fn duration_backfills_only_when_absent() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        let directives = wiring.on_player(
            PlayerOutput::DurationKnown {
                file: hash(1),
                duration_millis: 1_440_000,
            },
            &view,
        );
        assert!(
            directives
                .iter()
                .any(|d| matches!(d, Directive::Mutate(Mutation::SetPlaylistDuration { .. })))
        );

        // Entry already has a duration: nothing to do.
        let mut state = CrdtState::new();
        let mut with_duration = entry(1, "ep1.mkv");
        with_duration.duration_millis = Some(1_440_000);
        state.push_playlist_entry(A, ts(1), with_duration);
        let directives = wiring.on_player(
            PlayerOutput::DurationKnown {
                file: hash(1),
                duration_millis: 999,
            },
            &state.view(),
        );
        assert!(directives.is_empty());
    }

    #[test]
    fn eof_and_fatal_crash_map_to_their_directives() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        let directives = wiring.on_player(PlayerOutput::Eof { file: hash(1) }, &view);
        assert!(
            directives
                .iter()
                .any(|d| matches!(d, Directive::ReportEof(h) if *h == hash(1)))
        );
        let directives = wiring.on_player(PlayerOutput::FatalCrash, &view);
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetPlaybackIntent {
                intent: PlaybackIntent::Paused
            })
        )));
        assert!(
            directives
                .iter()
                .any(|d| matches!(d, Directive::Mutate(Mutation::Chat { .. })))
        );
    }

    #[test]
    fn note_local_file_skips_the_matcher_and_reports_ready() {
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.note_local_file(hash(1), "/media/ep1.mkv".into());
        assert!(directives.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetFileAvailability {
                availability: FileAvailability::Ready,
                ..
            })
        )));
        // The entry arriving later in the view must not re-resolve.
        let view = playing_state().view();
        let on_state = wiring.on_state(&view, &[peer("kim")]);
        assert!(
            !on_state
                .iter()
                .any(|d| matches!(d, Directive::Resolve { .. })),
            "locally-added files are already verified"
        );
        // And now-playing loads straight away.
        assert!(
            player_cmds(&on_state)
                .iter()
                .any(|cmd| matches!(cmd, PlayerCommand::Load { .. }))
        );
    }
}
