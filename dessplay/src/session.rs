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

use crate::actors::file::{
    FileCommand, FileConfig, FileOutput, HashEvent, HashedAdd, IndexedFile, Resolution,
};
use crate::actors::player::{PlayerCommand, PlayerOutput};
use crate::actors::sync::Mutation;

/// A subtitle line bound for the UI. `video_millis` is the in-video
/// position (the displayed MM:SS timestamp); `arrival_millis` is the
/// shell's wall-clock stamp, used to interleave with chat. Strictly
/// local — never synced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubtitleLine {
    /// Subtitle text.
    pub text: String,
    /// In-video position when the cue appeared (milliseconds).
    pub video_millis: u64,
    /// Wall-clock arrival on the shared clock (milliseconds).
    pub arrival_millis: u64,
}

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
    /// A subtitle line for the UI. Carries the in-video position (the
    /// displayed timestamp); the shell stamps wall-clock arrival.
    Subtitle {
        /// Subtitle text.
        text: String,
        /// In-video position when the cue appeared (milliseconds).
        video_millis: u64,
    },
    /// Record a personally-watched file (the 85% rule crossed). The
    /// shell stamps `watched_at` with its clock and forwards it to the
    /// file actor; this feeds known-series detection and Recent Series
    /// sorting (design.md, Watch Tracking).
    RecordWatched {
        /// The watched file.
        file: Ed2kHash,
        /// Series id, if metadata has one.
        series_id: Option<dessplay_core::types::AniDbSeriesId>,
        /// Series name (always present once metadata arrives).
        series_name: Option<String>,
        /// Filename, for display in history.
        filename: String,
    },
    /// Ask the file actor whether the now-playing missing file's series
    /// is personally known (watch history). The answer drives the
    /// missing-file branch (design.md, File State).
    CheckSeriesKnown {
        /// The missing now-playing file.
        file: Ed2kHash,
        /// Series id, if metadata has one.
        series_id: Option<dessplay_core::types::AniDbSeriesId>,
        /// Series name, for the history-by-name lookup.
        series_name: String,
    },
    /// Render the not-watching placeholder PNG for `file`.
    RenderPlaceholder {
        /// The file the placeholder stands in for.
        file: Ed2kHash,
        /// Lines to draw (filename, explanation, session status).
        lines: Vec<String>,
    },
    /// Begin/refresh downloading a missing file from peers that have it.
    StartDownload {
        /// The file.
        file: Ed2kHash,
        /// File size, for chunk geometry.
        size_bytes: u64,
        /// Present peers advertising the file (Ready).
        sources: Vec<dessplay_core::net::PeerId>,
        /// Playback chunk anchor for the sequential window.
        play_chunk: u32,
    },
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
    /// AniDB lookups already requested this session (the request is a
    /// GSet insert; this just avoids re-sending every snapshot).
    lookups_requested: HashSet<Ed2kHash>,
    /// Series preferences already written from List watchers sets.
    watcher_prefs_written: HashSet<dessplay_core::types::AniDbSeriesId>,
    /// Files already recorded as personally watched this session (the
    /// 85% rule fires once per file).
    watched_recorded: HashSet<Ed2kHash>,
    /// Missing now-playing files we've already asked known-series about
    /// (the round trip fires once per file).
    series_known_checked: HashSet<Ed2kHash>,
}

/// Fraction of a file's duration that counts as "watched" (design.md,
/// Watch Tracking).
const WATCHED_FRACTION: f64 = 0.85;

/// How many queued playlist entries past now-playing an interactive
/// client prefetches (design.md, Pre-fetching). A small fixed lookahead;
/// disk/retention-aware depth is future work. Seeders fetch everything.
const PREFETCH_AHEAD: usize = 2;

/// Text for the not-watching placeholder image (design.md, Placeholder
/// Image): the filename, the explanation, and who *is* watching.
fn placeholder_lines(view: &StateView, peers: &[PeerInfo], file: Ed2kHash) -> Vec<String> {
    let filename = view
        .playlist
        .iter()
        .find(|e| e.hash == file)
        .map(|e| e.state.filename.clone())
        .unwrap_or_else(|| file.to_string());
    let watching: Vec<String> = peers
        .iter()
        .filter(|p| {
            p.role == dessplay_core::net::Role::Interactive
                && !matches!(
                    derive::user_state(view, &p.username),
                    derive::DerivedUserState::NotWatching
                )
        })
        .map(|p| p.username.to_string())
        .collect();
    let status = if watching.is_empty() {
        "Nobody is watching this".to_string()
    } else {
        format!("Watching: {}", watching.join(", "))
    };
    vec![
        filename,
        "You don't have this file".to_string(),
        String::new(),
        status,
    ]
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
            lookups_requested: HashSet::new(),
            watcher_prefs_written: HashSet::new(),
            watched_recorded: HashSet::new(),
            series_known_checked: HashSet::new(),
        }
    }

    /// If the now-playing file is missing and has metadata, ask the file
    /// actor whether its series is personally known (once per file).
    /// The answer drives the missing-file branch in [`Self::on_series_known`].
    fn maybe_check_series_known(&mut self, view: &StateView) -> Vec<Directive> {
        let Some(file) = view.now_playing else {
            return vec![];
        };
        let missing = matches!(
            self.resolved.get(&file),
            Some(Resolution::NotFound) | Some(Resolution::HashMismatch(_))
        );
        if !missing || self.series_known_checked.contains(&file) {
            return vec![];
        }
        // Need metadata to identify the series; before it arrives we
        // simply block (a Missing file gates), and re-check when it does.
        let Some(Some(metadata)) = view.anidb_metadata.get(&file) else {
            return vec![];
        };
        self.series_known_checked.insert(file);
        vec![Directive::CheckSeriesKnown {
            file,
            series_id: metadata.series_id,
            series_name: metadata.series_name.clone(),
        }]
    }

    /// Start/refresh downloads for missing files we want: the now-playing
    /// file plus a small **prefetch** window of queued entries ahead of
    /// it (design.md, Pre-fetching), so next episodes arrive before they
    /// are needed. Each download is idempotent in the file actor; sources
    /// are present peers (interactive or seeder) advertising the file
    /// Ready. Watched/already-local entries are skipped.
    /// Present peers (not us) advertising `file` Ready — the sources a
    /// download can pull from. Used both to start downloads and to
    /// decide a missing file is obtainable (so it downloads rather than
    /// flipping us to NotWatching).
    fn download_sources(
        &self,
        view: &StateView,
        peers: &[PeerInfo],
        file: Ed2kHash,
    ) -> Vec<dessplay_core::net::PeerId> {
        peers
            .iter()
            .filter(|p| {
                p.username != self.me
                    && p.presence == dessplay_core::net::Presence::Present
                    && view.file_availability.get(&(p.username.clone(), file))
                        == Some(&FileAvailability::Ready)
            })
            .map(|p| p.username.clone())
            .collect()
    }

    fn plan_download(&self, view: &StateView, peers: &[PeerInfo]) -> Vec<Directive> {
        let Some(now) = view.now_playing else {
            return vec![];
        };
        let Some(start) = view.playlist.iter().position(|e| e.hash == now) else {
            return vec![];
        };
        let mut out = Vec::new();
        // Now-playing, then the next PREFETCH_AHEAD queued entries.
        for entry in view.playlist.iter().skip(start).take(1 + PREFETCH_AHEAD) {
            let file = entry.hash;
            // Have it (or not resolved yet), or already watched: skip.
            if matches!(self.resolved.get(&file), Some(Resolution::Verified(_)) | None)
                || view.watched.get(&file) == Some(&true)
            {
                continue;
            }
            let sources = self.download_sources(view, peers, file);
            if sources.is_empty() {
                continue;
            }
            out.push(Directive::StartDownload {
                file,
                size_bytes: entry.state.size_bytes,
                sources,
                // Sequential from the start; seek-aware windowing is
                // future work (downloads still prioritise early chunks).
                play_chunk: 0,
            });
        }
        out
    }

    /// React to the file actor's known-series answer (design.md, File
    /// State, missing-file branch). Known series stay Missing (you
    /// should have the file). For an *unknown* series we auto-mark
    /// NotWatching — but only when there is an AniDB series id to key
    /// the preference on; a no-id file keeps blocking, with the manual
    /// not-watching action as the escape hatch. A pre-existing
    /// preference (a manual choice) is never overridden.
    pub fn on_series_known(
        &mut self,
        file: Ed2kHash,
        series: Option<dessplay_core::types::AniDbSeriesId>,
        known: bool,
        view: &StateView,
        peers: &[PeerInfo],
    ) -> Vec<Directive> {
        if known {
            return vec![];
        }
        let Some(series) = series else {
            // No series id: stays Missing/blocking (option B). The
            // placeholder would contradict the blocking state, so none.
            return vec![];
        };
        // A pre-existing preference is a manual choice and wins: if the
        // user chose to watch this series, they block legitimately on
        // the missing file (no auto-NotWatching, no placeholder).
        if view
            .series_preference
            .contains_key(&(self.me.clone(), series))
        {
            return vec![];
        }
        // Obtainable from a peer (e.g. the seeder)? Then it will
        // download — don't write a sticky NotWatching; just show the
        // placeholder while it arrives. (Residual race: if the source's
        // Ready hasn't synced when this fires we may still write
        // NotWatching once; the Users-pane downloading display masks it
        // and Ctrl-r clears it.)
        if !self.download_sources(view, peers, file).is_empty() {
            tracing::debug!(aid = series.0, "missing file is downloadable; not marking NotWatching");
            let mut out = vec![];
            if view.now_playing == Some(file) {
                out.push(Directive::RenderPlaceholder {
                    file,
                    lines: placeholder_lines(view, peers, file),
                });
            }
            return out;
        }
        tracing::info!(aid = series.0, "missing file from an unknown series; marking NotWatching");
        let mut out = vec![Directive::Mutate(Mutation::SetSeriesPreference {
            user: self.me.clone(),
            series,
            pref: dessplay_core::types::SeriesWatchState::NotWatching,
        })];
        // Show the placeholder instead of a stale frame / blank window.
        if view.now_playing == Some(file) {
            out.push(Directive::RenderPlaceholder {
                file,
                lines: placeholder_lines(view, peers, file),
            });
        }
        out
    }

    /// If the now-playing file has crossed the 85% watched threshold and
    /// hasn't been recorded yet, emit a [`Directive::RecordWatched`].
    /// Driven by position ticks; idempotent per file per session.
    fn maybe_record_watched(&mut self, view: &StateView, position_millis: u64) -> Vec<Directive> {
        let Some(file) = view.now_playing else {
            return vec![];
        };
        if self.watched_recorded.contains(&file) {
            return vec![];
        }
        let Some(entry) = view.playlist.iter().find(|e| e.hash == file) else {
            return vec![];
        };
        let Some(duration) = entry.state.duration_millis else {
            return vec![]; // can't judge the threshold without a duration
        };
        if duration == 0 || (position_millis as f64) < WATCHED_FRACTION * duration as f64 {
            return vec![];
        }
        self.watched_recorded.insert(file);
        let metadata = view.anidb_metadata.get(&file).and_then(|m| m.as_ref());
        vec![Directive::RecordWatched {
            file,
            series_id: metadata.and_then(|m| m.series_id),
            series_name: metadata.map(|m| m.series_name.clone()),
            filename: entry.state.filename.clone(),
        }]
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

    /// Request an AniDB lookup for `info.hash` if it still lacks metadata
    /// and we haven't requested it this session. The server records the
    /// file's identity (filename + size) in the broadcast file catalog
    /// when it drains the request, so the file becomes addable group-wide.
    fn maybe_request_lookup(
        &mut self,
        info: dessplay_core::types::FileHashInfo,
        view: &StateView,
        out: &mut Vec<Directive>,
    ) {
        let missing = view
            .anidb_metadata
            .get(&info.hash)
            .is_none_or(|meta| meta.is_none());
        if missing && self.lookups_requested.insert(info.hash) {
            out.push(Directive::Mutate(Mutation::RequestLookup { info }));
        }
    }

    /// The media-library scan reported indexed files: request AniDB
    /// lookups for any still lacking metadata. The scan re-reports
    /// cache-hit files every pass, so this naturally re-arms; the
    /// `lookups_requested` set keeps it from re-sending within a session.
    pub fn on_library_indexed(
        &mut self,
        files: Vec<IndexedFile>,
        view: &StateView,
    ) -> Vec<Directive> {
        let mut out = Vec::new();
        for f in files {
            self.maybe_request_lookup(
                dessplay_core::types::FileHashInfo {
                    hash: f.hash,
                    size: f.size,
                    filename: f.filename,
                    mtime: Some(f.mtime),
                    series_hint: f.series_hint,
                },
                view,
                &mut out,
            );
        }
        out
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

        // Ask the server to look up entries with no AniDB metadata yet.
        // Hash, size, and filename all live in the entry, so any client
        // can request; the server dedups, and the GSet dedups the
        // replicated insert. Covers the adder going offline and the
        // request set being cleared at compaction.
        for entry in &view.playlist {
            self.maybe_request_lookup(
                dessplay_core::types::FileHashInfo {
                    hash: entry.hash,
                    size: entry.state.size_bytes,
                    filename: entry.state.filename.clone(),
                    // Playlist entries carry no mtime (a client may not even
                    // hold the file); the server then anchors on first-seen.
                    // No local path here either, so no directory to derive a
                    // series hint from — the library scan supplies that.
                    mtime: None,
                    series_hint: None,
                },
                view,
                &mut out,
            );
        }

        // The List's watchers sets: a linked series we're *not* watching
        // gets a NotWatching preference, so we never gate playback on a
        // show we skip (docs/design.md, The List). Written once per
        // series per session, and only when no preference exists — a
        // manual choice always wins.
        for entry in view.list_entries.values() {
            let Some(series) = entry.anidb_series_id else {
                continue;
            };
            // An empty watchers set means "unrecorded", not "nobody".
            if entry.watchers.is_empty() {
                continue;
            }
            let has_pref = view
                .series_preference
                .contains_key(&(self.me.clone(), series));
            if entry.watchers.contains(&self.me)
                || has_pref
                || !self.watcher_prefs_written.insert(series)
            {
                continue;
            }
            tracing::info!(aid = series.0, name = %entry.name, "not in watchers; marking series NotWatching");
            out.push(Directive::Mutate(Mutation::SetSeriesPreference {
                user: self.me.clone(),
                series,
                pref: dessplay_core::types::SeriesWatchState::NotWatching,
            }));
        }

        // A missing now-playing file with metadata: ask whether its
        // series is personally known (drives the missing-file branch).
        out.extend(self.maybe_check_series_known(view));

        // Retrieve a missing now-playing file from peers (design.md:
        // downloading is the default). The file actor's download start
        // is idempotent, so re-emitting on each snapshot just refreshes
        // the source set as peers/availability change.
        out.extend(self.plan_download(view, peers));

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
        // `self.loaded` only ever names the real verified now-playing
        // video — a placeholder is never "now-playing" by this measure,
        // which is exactly why it must not be told to play. When the
        // loaded file is *not* the now-playing one (now-playing switched
        // to something we don't hold, so a stale frame or placeholder is
        // on screen), force pause: never resume the wrong file.
        let active = derive::playback_active(view, peers);
        let showing_now_playing = self.loaded.is_some() && self.loaded == view.now_playing;
        if self.loaded.is_some() {
            out.push(Directive::Player(PlayerCommand::SetPlaying(
                showing_now_playing && active,
            )));
        }

        // Follow the seek authority's position (never our own). Only when
        // the real now-playing video is what's loaded.
        if showing_now_playing
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
                let mut out = vec![Directive::Mutate(Mutation::SetPlaybackPosition {
                    position_millis,
                })];
                out.extend(self.maybe_record_watched(view, position_millis));
                out
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
            PlayerOutput::SubtitleLine {
                text,
                position_millis,
            } => vec![Directive::Subtitle {
                text,
                video_millis: position_millis,
            }],
            PlayerOutput::Eof { file } => vec![Directive::ReportEof(file)],
            PlayerOutput::LoadFailed { file } => {
                // The path we loaded is gone/unreadable. Forget it, flip
                // to Missing, and re-resolve so a re-download (or a
                // re-appeared file) can recover. Next derive re-loads.
                if self.loaded == Some(file) {
                    self.loaded = None;
                }
                self.resolved.remove(&file);
                let mut out = vec![Directive::Mutate(Mutation::SetFileAvailability {
                    file,
                    availability: FileAvailability::Missing,
                })];
                if let Some(entry) = view.playlist.iter().find(|entry| entry.hash == file) {
                    self.pending_resolve.insert(file);
                    out.push(Directive::Resolve {
                        file,
                        filename: entry.state.filename.clone(),
                    });
                }
                out
            }
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
    me: UserId,
    wiring: PlayerWiring,
    /// Taken on the first `Load`, when the player actor spawns.
    factory: Option<F>,
    clock: crate::actors::network::Clock,
    clock_offset: i64,
    player: Option<tokio::sync::mpsc::Sender<PlayerCommand>>,
    player_out_tx: tokio::sync::mpsc::Sender<PlayerOutput>,
    /// Player actor outputs; feed each into [`Self::on_player_output`].
    pub player_outputs: tokio::sync::mpsc::Receiver<PlayerOutput>,
    /// Commands to the file actor (resolve, hash, mapping, eviction…).
    file: tokio::sync::mpsc::Sender<FileCommand>,
    /// File-actor outputs; feed each into [`Self::on_file_output`].
    pub file_outputs: tokio::sync::mpsc::Receiver<FileOutput>,
    /// Paths whose playlist-add hash is still running (the quit-time
    /// warning; the file actor does its own re-add dedup).
    hashing: HashSet<PathBuf>,
    sync: tokio::sync::mpsc::Sender<crate::actors::sync::SyncCommand>,
    network: tokio::sync::mpsc::Sender<crate::actors::network::NetworkCommand>,
}

impl<F: crate::player::PlayerFactory> SessionShell<F> {
    /// Build a shell, spawning the file actor with `file_config`.
    /// Nothing else runs until directives start flowing.
    pub fn new(
        me: UserId,
        factory: F,
        clock: crate::actors::network::Clock,
        file_config: FileConfig,
        sync: tokio::sync::mpsc::Sender<crate::actors::sync::SyncCommand>,
        network: tokio::sync::mpsc::Sender<crate::actors::network::NetworkCommand>,
    ) -> Self {
        let (player_out_tx, player_outputs) = tokio::sync::mpsc::channel(256);
        let (file_tx, file_rx) = tokio::sync::mpsc::channel(64);
        let (file_out_tx, file_outputs) = tokio::sync::mpsc::channel(128);
        tokio::spawn(crate::actors::file::run(file_config, file_rx, file_out_tx));
        SessionShell {
            me: me.clone(),
            wiring: PlayerWiring::new(me),
            factory: Some(factory),
            clock,
            clock_offset: 0,
            player: None,
            player_out_tx,
            player_outputs,
            file: file_tx,
            file_outputs,
            hashing: HashSet::new(),
            sync,
            network,
        }
    }

    /// Hash a local file (in the file actor) and add it to the playlist
    /// when done. Progress and completion arrive on
    /// [`Self::file_outputs`] as [`FileOutput::Hash`] events. Hashing is
    /// around a second per gigabyte, so it must never run inline in the
    /// bridge loop — a stuck hash must not stop the UI or a quit. The
    /// file actor dedups re-adds; this just tracks the in-flight count.
    pub async fn hash_and_add(&mut self, path: PathBuf, after: Option<Ed2kHash>) {
        if !self.hashing.insert(path.clone()) {
            tracing::debug!(path = %path.display(), "already hashing; ignoring re-add");
            return;
        }
        let _ = self.file.send(FileCommand::HashAdd { path, after }).await;
    }

    /// A background hash finished: add the file to the playlist.
    pub async fn on_hashed(&mut self, done: HashedAdd) -> Vec<SubtitleLine> {
        self.hashing.remove(&done.path);
        let hashed = match done.result {
            Ok(hashed) => hashed,
            Err(e) => {
                tracing::error!(path = %done.path.display(), "hashing failed: {e}");
                return Vec::new();
            }
        };
        let filename = done
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| done.path.display().to_string());
        let _ = self
            .sync
            .send(crate::actors::sync::SyncCommand::Mutate(Box::new(
                Mutation::AddPlaylistAfter {
                    anchor: done.after,
                    new: dessplay_core::playlist::NewPlaylistEntry {
                        hash: hashed.root,
                        added_by: self.me.clone(),
                        filename,
                        size_bytes: hashed.size_bytes,
                        // Backfilled by the player's duration probe on
                        // first load.
                        duration_millis: None,
                    },
                },
            )))
            .await;
        // We picked this file: it is its own verified local copy.
        self.note_local_file(hashed.root, done.path).await
    }

    /// Add a file to the playlist by hash, taking its identity from the
    /// synced file catalog. Unlike [`Self::hash_and_add`] the user need
    /// not hold the file: the normal snapshot-driven resolve then marks
    /// it Ready (we have it) or Missing → download (we don't). Returns a
    /// local chat notice on failure (the catalog entry hasn't arrived —
    /// no client has requested a lookup for this hash yet).
    pub async fn add_by_hash(
        &self,
        hash: Ed2kHash,
        after: Option<Ed2kHash>,
        view: &StateView,
    ) -> Option<String> {
        match add_by_hash_mutation(&self.me, hash, after, view) {
            Ok(mutation) => {
                let _ = self
                    .sync
                    .send(crate::actors::sync::SyncCommand::Mutate(Box::new(mutation)))
                    .await;
                None
            }
            Err(notice) => {
                tracing::info!(%hash, "add-by-hash: not in the file catalog yet");
                Some(notice)
            }
        }
    }

    /// Tell the file actor the media roots changed (settings save).
    pub async fn set_media_roots(&self, roots: Vec<PathBuf>) {
        let _ = self.file.send(FileCommand::SetMediaRoots(roots)).await;
    }

    /// Tell the file actor the retention policy changed (settings save).
    pub async fn set_retention(&self, retention: crate::config::CacheRetention) {
        let _ = self.file.send(FileCommand::SetRetention(retention)).await;
    }

    /// Persist a manual mapping (and resolve it Verified at once).
    pub async fn set_manual_mapping(
        &self,
        file: Ed2kHash,
        path: PathBuf,
        series: Option<crate::storage::SeriesKey>,
    ) {
        let _ = self
            .file
            .send(FileCommand::SetManualMapping { file, path, series })
            .await;
    }

    /// Archive a cached file into the library under the download root.
    pub async fn archive(&self, file: Ed2kHash, series_name: Option<String>, filename: String) {
        let _ = self
            .file
            .send(FileCommand::Archive {
                file,
                series_name,
                filename,
            })
            .await;
    }

    /// How many playlist-add hashes are still running (logged at quit —
    /// their adds are dropped).
    pub fn hashes_in_flight(&self) -> usize {
        self.hashing.len()
    }

    /// A fresh state view arrived. Returns subtitle lines for the UI.
    pub async fn on_state(&mut self, view: &StateView, peers: &[PeerInfo]) -> Vec<SubtitleLine> {
        let directives = self.wiring.on_state(view, peers);
        self.execute(directives).await
    }

    /// The player actor reported something.
    pub async fn on_player_output(
        &mut self,
        output: PlayerOutput,
        view: &StateView,
    ) -> Vec<SubtitleLine> {
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
    ) -> Vec<SubtitleLine> {
        let directives = self.wiring.on_resolved(file, resolution, view, peers);
        self.execute(directives).await
    }

    /// We hashed and added this file ourselves.
    pub async fn note_local_file(&mut self, file: Ed2kHash, path: PathBuf) -> Vec<SubtitleLine> {
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

    async fn execute(&mut self, directives: Vec<Directive>) -> Vec<SubtitleLine> {
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
                    let _ = self.file.send(FileCommand::Resolve { file, filename }).await;
                }
                Directive::RecordWatched {
                    file,
                    series_id,
                    series_name,
                    filename,
                } => {
                    let _ = self
                        .file
                        .send(FileCommand::RecordWatched(crate::storage::WatchRecord {
                            hash: file,
                            series_id,
                            series_name,
                            filename,
                            watched_at: (self.clock)() as i64,
                        }))
                        .await;
                }
                Directive::CheckSeriesKnown {
                    file,
                    series_id,
                    series_name,
                } => {
                    let key = match series_id {
                        Some(id) => crate::storage::SeriesKey::AniDb(id),
                        None => crate::storage::SeriesKey::Name(series_name),
                    };
                    let _ = self
                        .file
                        .send(FileCommand::CheckSeriesKnown {
                            file,
                            series: series_id,
                            key,
                        })
                        .await;
                }
                Directive::RenderPlaceholder { file, lines } => {
                    let _ = self
                        .file
                        .send(FileCommand::RenderPlaceholder { file, lines })
                        .await;
                }
                Directive::StartDownload {
                    file,
                    size_bytes,
                    sources,
                    play_chunk,
                } => {
                    let _ = self
                        .file
                        .send(FileCommand::StartDownload {
                            file,
                            size_bytes,
                            sources,
                            play_chunk,
                        })
                        .await;
                }
                Directive::Subtitle {
                    text,
                    video_millis,
                } => subtitles.push(SubtitleLine {
                    text,
                    video_millis,
                    // Stamp arrival with the same clock chat/system lines
                    // use, so all three share one interleave domain.
                    arrival_millis: (self.clock)(),
                }),
            }
        }
        subtitles
    }

    /// A file-transfer message relayed from a peer: hand it to the file
    /// actor (download scheduling or serving).
    pub async fn on_network_peer(
        &self,
        from: dessplay_core::net::PeerId,
        message: Box<dessplay_core::net::PeerMessage>,
    ) {
        let _ = self
            .file
            .send(FileCommand::PeerMessage { from, message })
            .await;
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

    /// Dispatch one file-actor output. Returns subtitle lines (always
    /// empty here, for a uniform shape with the other `on_*` methods)
    /// plus, for a hash-progress event, the [`HashEvent`] the caller
    /// forwards to the UI overlay. Resolution completions are applied
    /// through [`PlayerWiring::on_resolved`], so the caller passes the
    /// current view and peer list.
    pub async fn on_file_output(
        &mut self,
        output: FileOutput,
        view: &StateView,
        peers: &[PeerInfo],
    ) -> FileEffect {
        match output {
            FileOutput::Resolved { file, resolution } => {
                let directives = self.wiring.on_resolved(file, resolution, view, peers);
                self.execute(directives).await;
                FileEffect::None
            }
            FileOutput::Hash(HashEvent::Progress {
                path,
                done_bytes,
                total_bytes,
            }) => FileEffect::HashProgress {
                path,
                done_bytes,
                total_bytes,
            },
            FileOutput::Hash(HashEvent::Done(done)) => {
                let path = done.path.clone();
                self.on_hashed(done).await;
                FileEffect::HashDone { path }
            }
            FileOutput::SeriesKnown {
                file,
                series,
                known,
            } => {
                let directives = self.wiring.on_series_known(file, series, known, view, peers);
                self.execute(directives).await;
                FileEffect::None
            }
            FileOutput::PlaceholderReady { file, path } => {
                // Show the placeholder only if it's still the
                // now-playing file. This is a direct Load that does not
                // touch the wiring's `loaded` state, so the real video
                // still loads if the file later becomes available.
                if view.now_playing == Some(file) {
                    self.execute(vec![Directive::Player(PlayerCommand::Load { file, path })])
                        .await;
                }
                FileEffect::None
            }
            FileOutput::SendPeer { to, message } => {
                let _ = self
                    .network
                    .send(crate::actors::network::NetworkCommand::SendPeer { to, message })
                    .await;
                FileEffect::None
            }
            FileOutput::Availability { file, availability } => {
                let _ = self
                    .sync
                    .send(crate::actors::sync::SyncCommand::Mutate(Box::new(
                        Mutation::SetFileAvailability { file, availability },
                    )))
                    .await;
                FileEffect::None
            }
            FileOutput::DownloadComplete { file, path } => {
                // A finished download is now a verified local copy:
                // resolve it (loads now-playing if we were waiting).
                let directives = self.wiring.note_local_file(file, path);
                self.execute(directives).await;
                FileEffect::None
            }
            FileOutput::Archived { file, result } => {
                // The archive moved the file out of the cache (or failed).
                // Either way, tell the user via a local chat notice; on
                // success the bridge also refreshes the snapshot so the
                // "temporary" marker clears.
                FileEffect::Archived {
                    timestamp: (self.clock)(),
                    text: archive_notice(view, file, &result),
                }
            }
            FileOutput::LibraryIndexed { files } => {
                let directives = self.wiring.on_library_indexed(files, view);
                self.execute(directives).await;
                FileEffect::None
            }
            FileOutput::ScanProgress { done, total } => {
                FileEffect::ScanProgress { done, total }
            }
            FileOutput::WatchRecorded => FileEffect::WatchRecorded,
            // The remaining outputs are consumed by the bridge loop as
            // those features land (eviction notices).
            other => FileEffect::Other(other),
        }
    }
}

/// Build the playlist-add mutation for an add-by-hash, taking the file's
/// identity from the synced catalog. `Err` (with a user notice) when the
/// catalog has no entry yet — no client has requested a lookup for this
/// hash, so we don't know its filename/size.
fn add_by_hash_mutation(
    me: &UserId,
    hash: Ed2kHash,
    after: Option<Ed2kHash>,
    view: &StateView,
) -> Result<Mutation, String> {
    let entry = view
        .file_catalog
        .get(&hash)
        .ok_or_else(|| "Can't add that yet — its file info hasn't synced.".to_string())?;
    Ok(Mutation::AddPlaylistAfter {
        anchor: after,
        new: dessplay_core::playlist::NewPlaylistEntry {
            hash,
            added_by: me.clone(),
            filename: entry.filename.clone(),
            size_bytes: entry.size_bytes,
            // Backfilled by the player's duration probe on first load (or
            // carried by the catalog later).
            duration_millis: entry.duration_millis,
        },
    })
}

/// The user-facing chat line for an archive result. Uses the playlist
/// entry's filename (falling back to the hash if the entry is gone).
fn archive_notice(view: &StateView, file: Ed2kHash, result: &Result<PathBuf, String>) -> String {
    let name = view
        .playlist
        .iter()
        .find(|entry| entry.hash == file)
        .map(|entry| entry.state.filename.clone())
        .unwrap_or_else(|| file.to_string());
    match result {
        Ok(_) => format!("Archived {name}"),
        Err(error) => format!("Archive failed ({name}): {error}"),
    }
}

/// What a [`FileOutput`] means to the bridge loop after the shell has
/// applied its own side effects: a hashing-overlay update, or an output
/// the loop still needs to route (placeholder, eviction, …).
#[derive(Debug)]
pub enum FileEffect {
    /// Nothing further for the caller to do.
    None,
    /// Update the hashing progress overlay.
    HashProgress {
        /// File being hashed.
        path: PathBuf,
        /// Bytes read.
        done_bytes: u64,
        /// Total size (0 when unknowable).
        total_bytes: u64,
    },
    /// A hash finished; clear its overlay row.
    HashDone {
        /// The file that finished.
        path: PathBuf,
    },
    /// An archive attempt finished (success or failure): show a local
    /// system chat line, and refresh the snapshot so a now-archived
    /// file loses its "temporary" marker.
    Archived {
        /// Shared-clock millis for the chat line.
        timestamp: u64,
        /// Human-readable result ("Archived …" / "Archive failed …").
        text: String,
    },
    /// Media-library scan hashing progress (the no-silent-work rule):
    /// the UI shows a transient one-line status while files are hashing.
    ScanProgress {
        /// Files hashed so far this scan.
        done: usize,
        /// Files needing a hash this scan.
        total: usize,
    },
    /// A watch was just recorded: refresh the snapshot so Recent Series
    /// re-reads watch history and reflects the new recency. Carries no
    /// data — watch recording produces no sync event of its own.
    WatchRecorded,
    /// An output not yet consumed by the shell.
    Other(FileOutput),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use dessplay_core::net::{Presence, Role};
    use dessplay_core::playlist::NewPlaylistEntry;
    use dessplay_core::state::CrdtState;
    use dessplay_core::types::{ActorId, PlaybackPosition, SeriesWatchState, SharedTimestamp};

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
    fn archive_notice_uses_filename_and_reports_outcome() {
        let view = playing_state().view();
        assert_eq!(
            archive_notice(&view, hash(1), &Ok("/media/Frieren/ep1.mkv".into())),
            "Archived ep1.mkv"
        );
        assert_eq!(
            archive_notice(&view, hash(1), &Err("disk full".into())),
            "Archive failed (ep1.mkv): disk full"
        );
        // Unknown file (not in the playlist) falls back to the hash.
        let notice = archive_notice(&view, hash(9), &Ok("/x".into()));
        assert!(notice.starts_with("Archived "), "{notice}");
    }

    #[test]
    fn add_by_hash_builds_from_catalog_and_fails_without_it() {
        let mut state = CrdtState::new();
        state.set_file_catalog(
            A,
            ts(1),
            hash(5),
            dessplay_core::types::FileCatalogEntry {
                filename: "Frieren - 05.mkv".into(),
                size_bytes: 700_000_000,
                duration_millis: None,
            },
        );
        let view = state.view();

        // In the catalog: builds an AddPlaylistAfter with its identity.
        match add_by_hash_mutation(&me(), hash(5), None, &view) {
            Ok(Mutation::AddPlaylistAfter { anchor, new }) => {
                assert_eq!(anchor, None);
                assert_eq!(new.hash, hash(5));
                assert_eq!(new.filename, "Frieren - 05.mkv");
                assert_eq!(new.size_bytes, 700_000_000);
            }
            other => panic!("unexpected: {other:?}"),
        }

        // Not in the catalog yet: a user-facing notice, no mutation.
        assert!(add_by_hash_mutation(&me(), hash(9), None, &view).is_err());
    }

    /// The hashes a directive list asks the server to look up.
    fn lookup_hashes(directives: &[Directive]) -> Vec<Ed2kHash> {
        directives
            .iter()
            .filter_map(|d| match d {
                Directive::Mutate(Mutation::RequestLookup { info }) => Some(info.hash),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn library_index_requests_lookups_for_unknown_files_only() {
        let mut state = CrdtState::new();
        // hash(2) already has metadata; hash(1) and hash(3) don't.
        state.set_anidb_metadata(
            A,
            ts(1),
            hash(2),
            Some(dessplay_core::types::AniDbMetadata {
                source: dessplay_core::types::MetadataSource::AniDb,
                series_name: "Known".into(),
                series_id: Some(dessplay_core::types::AniDbSeriesId(7)),
                episode_number: Some("1".into()),
            }),
        );
        let view = state.view();
        let mut wiring = PlayerWiring::new(me());

        let indexed = |h, size, name: &str, mtime| IndexedFile {
            hash: h,
            size,
            filename: name.to_string(),
            mtime,
            series_hint: None,
        };
        let files = vec![
            indexed(hash(1), 100, "a.mkv", 1_000),
            indexed(hash(2), 200, "b.mkv", 2_000),
            indexed(hash(3), 300, "c.mkv", 3_000),
        ];
        let out = wiring.on_library_indexed(files.clone(), &view);
        // hash(2) is skipped (already has metadata); the rest are requested.
        assert_eq!(lookup_hashes(&out), vec![hash(1), hash(3)]);

        // Re-reporting the same batch (every scan does) requests nothing
        // new — the per-session dedup holds.
        let again = wiring.on_library_indexed(files, &view);
        assert!(lookup_hashes(&again).is_empty());
    }

    #[test]
    fn load_failure_flips_to_missing_and_re_resolves() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        // Pretend the now-playing file resolved and loaded.
        wiring.on_resolved(
            hash(1),
            Resolution::Verified("/media/ep1.mkv".into()),
            &view,
            &[peer("kim")],
        );

        let out = wiring.on_player(PlayerOutput::LoadFailed { file: hash(1) }, &view);
        assert!(
            out.iter().any(|d| matches!(
                d,
                Directive::Mutate(Mutation::SetFileAvailability {
                    file,
                    availability: FileAvailability::Missing,
                }) if *file == hash(1)
            )),
            "load failure must flip the file to Missing: {out:?}"
        );
        assert!(
            out.iter().any(|d| matches!(
                d,
                Directive::Resolve { file, filename } if *file == hash(1) && filename == "ep1.mkv"
            )),
            "load failure must re-resolve the file: {out:?}"
        );
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

    fn watched_records(directives: &[Directive]) -> Vec<Ed2kHash> {
        directives
            .iter()
            .filter_map(|d| match d {
                Directive::RecordWatched { file, .. } => Some(*file),
                _ => None,
            })
            .collect()
    }

    /// A playing_state whose entry has a known duration, so the 85%
    /// threshold is computable.
    fn timed_state(duration_millis: u64) -> CrdtState {
        let mut state = CrdtState::new();
        let mut e = entry(1, "ep1.mkv");
        e.duration_millis = Some(duration_millis);
        state.push_playlist_entry(A, ts(1), e);
        state.set_now_playing(A, ts(2), Some(hash(1)));
        state.set_playback_intent(A, ts(3), dessplay_core::types::PlaybackIntent::Playing);
        state
    }

    #[test]
    fn crossing_85_percent_records_watched_once() {
        let mut wiring = PlayerWiring::new(me());
        let view = timed_state(100_000).view();
        // Below threshold: nothing.
        let before = wiring.on_player(
            PlayerOutput::PositionTick {
                position_millis: 80_000,
            },
            &view,
        );
        assert!(watched_records(&before).is_empty());
        // At 85%: recorded.
        let at = wiring.on_player(
            PlayerOutput::PositionTick {
                position_millis: 85_000,
            },
            &view,
        );
        assert_eq!(watched_records(&at), vec![hash(1)]);
        // Later ticks: not re-recorded.
        let after = wiring.on_player(
            PlayerOutput::PositionTick {
                position_millis: 95_000,
            },
            &view,
        );
        assert!(watched_records(&after).is_empty());
    }

    #[test]
    fn watched_record_carries_series_metadata() {
        let mut state = timed_state(100_000);
        state.set_anidb_metadata(
            A,
            ts(4),
            hash(1),
            Some(dessplay_core::types::AniDbMetadata {
                source: dessplay_core::types::MetadataSource::AniDb,
                series_name: "Frieren".into(),
                series_id: Some(dessplay_core::types::AniDbSeriesId(42)),
                episode_number: Some("1".into()),
            }),
        );
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_player(
            PlayerOutput::PositionTick {
                position_millis: 90_000,
            },
            &state.view(),
        );
        match directives.iter().find_map(|d| match d {
            Directive::RecordWatched {
                series_id,
                series_name,
                ..
            } => Some((series_id, series_name)),
            _ => None,
        }) {
            Some((series_id, series_name)) => {
                assert_eq!(*series_id, Some(dessplay_core::types::AniDbSeriesId(42)));
                assert_eq!(series_name.as_deref(), Some("Frieren"));
            }
            None => panic!("expected a RecordWatched directive"),
        }
    }

    #[test]
    fn no_duration_means_no_watched_record() {
        // Entry without a duration: the threshold is uncomputable, so
        // the 85% rule never fires (the EOF report still marks group
        // progress separately).
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view(); // entry(1) has duration None
        let directives = wiring.on_player(
            PlayerOutput::PositionTick {
                position_millis: 9_999_999,
            },
            &view,
        );
        assert!(watched_records(&directives).is_empty());
    }

    fn series_pref_writes(
        directives: &[Directive],
    ) -> Vec<(dessplay_core::types::AniDbSeriesId, SeriesWatchState)> {
        directives
            .iter()
            .filter_map(|d| match d {
                Directive::Mutate(Mutation::SetSeriesPreference { series, pref, .. }) => {
                    Some((*series, *pref))
                }
                _ => None,
            })
            .collect()
    }

    fn has_placeholder(directives: &[Directive]) -> bool {
        directives
            .iter()
            .any(|d| matches!(d, Directive::RenderPlaceholder { .. }))
    }

    fn with_metadata(state: &mut CrdtState, file: Ed2kHash, series_id: Option<u32>) {
        state.set_anidb_metadata(
            A,
            ts(50),
            file,
            Some(dessplay_core::types::AniDbMetadata {
                source: dessplay_core::types::MetadataSource::AniDb,
                series_name: "Some Show".into(),
                series_id: series_id.map(dessplay_core::types::AniDbSeriesId),
                episode_number: Some("1".into()),
            }),
        );
    }

    #[test]
    fn missing_now_playing_with_metadata_triggers_a_known_series_check() {
        let mut state = playing_state();
        with_metadata(&mut state, hash(1), Some(7));
        let view = state.view();
        let mut wiring = PlayerWiring::new(me());
        wiring.on_state(&view, &[peer("kim")]);
        // The file resolves Missing (not found).
        let on_missing = wiring.on_resolved(hash(1), Resolution::NotFound, &view, &[peer("kim")]);
        assert!(on_missing.iter().any(|d| matches!(
            d,
            Directive::Mutate(Mutation::SetFileAvailability {
                availability: FileAvailability::Missing,
                ..
            })
        )));
        // The next snapshot asks whether the series is known (once).
        let next = wiring.on_state(&view, &[peer("kim")]);
        let checks: Vec<_> = next
            .iter()
            .filter(|d| matches!(d, Directive::CheckSeriesKnown { .. }))
            .collect();
        assert_eq!(checks.len(), 1);
        // Not re-asked on the following snapshot.
        let again = wiring.on_state(&view, &[peer("kim")]);
        assert!(
            !again
                .iter()
                .any(|d| matches!(d, Directive::CheckSeriesKnown { .. }))
        );
    }

    #[test]
    fn unknown_series_with_id_marks_not_watching_and_shows_placeholder() {
        let mut state = playing_state();
        with_metadata(&mut state, hash(1), Some(7));
        let view = state.view();
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_series_known(
            hash(1),
            Some(dessplay_core::types::AniDbSeriesId(7)),
            false, // unknown
            &view,
            &[peer("kim")],
        );
        assert_eq!(
            series_pref_writes(&directives),
            vec![(
                dessplay_core::types::AniDbSeriesId(7),
                SeriesWatchState::NotWatching
            )]
        );
        assert!(has_placeholder(&directives));
    }

    #[test]
    fn known_series_keeps_blocking_no_placeholder() {
        let view = playing_state().view();
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_series_known(
            hash(1),
            Some(dessplay_core::types::AniDbSeriesId(7)),
            true, // known: you should have it
            &view,
            &[peer("kim")],
        );
        assert!(directives.is_empty());
    }

    #[test]
    fn unknown_series_without_id_blocks_with_no_auto_not_watching() {
        // Option B: a no-series-id missing file stays blocking; the
        // manual not-watching action is the escape hatch.
        let view = playing_state().view();
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_series_known(hash(1), None, false, &view, &[peer("kim")]);
        assert!(directives.is_empty());
    }

    #[test]
    fn a_manual_watch_choice_is_never_overridden_by_the_missing_branch() {
        let mut state = playing_state();
        with_metadata(&mut state, hash(1), Some(7));
        state.set_series_preference(
            A,
            ts(60),
            me(),
            dessplay_core::types::AniDbSeriesId(7),
            SeriesWatchState::Watching,
        );
        let view = state.view();
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_series_known(
            hash(1),
            Some(dessplay_core::types::AniDbSeriesId(7)),
            false,
            &view,
            &[peer("kim")],
        );
        // They chose to watch: no auto-NotWatching, and no placeholder
        // (they block legitimately on the missing file).
        assert!(directives.is_empty());
    }

    #[test]
    fn downloadable_missing_file_is_not_auto_not_watching() {
        // Bug 1b: a missing/unknown-series file that a present peer (the
        // seeder) advertises Ready is obtainable — it should download,
        // not flip us to a sticky NotWatching. We still show a
        // placeholder while it arrives.
        let mut state = playing_state();
        with_metadata(&mut state, hash(1), Some(7));
        state.set_file_availability(
            A,
            ts(40),
            UserId::new("nas"),
            hash(1),
            FileAvailability::Ready,
        );
        let view = state.view();
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_series_known(
            hash(1),
            Some(dessplay_core::types::AniDbSeriesId(7)),
            false, // unknown
            &view,
            &[peer("kim"), peer("nas")],
        );
        assert!(
            series_pref_writes(&directives).is_empty(),
            "a downloadable file must not be auto-NotWatching: {directives:?}"
        );
        assert!(has_placeholder(&directives));
    }

    #[test]
    fn unpause_does_not_resume_a_stale_file_after_now_playing_changes() {
        // Bug 2: once now-playing switches to a file we don't hold, the
        // previously-loaded real video must be held paused — not resumed
        // when the group unpauses.
        let mut state = playing_state();
        state.push_playlist_entry(A, ts(4), entry(2, "ep2.mkv"));
        let view = state.view();
        let mut wiring = PlayerWiring::new(me());
        // ep1 resolves and loads as the real video.
        wiring.on_resolved(
            hash(1),
            Resolution::Verified("/media/ep1.mkv".into()),
            &view,
            &[peer("kim")],
        );
        // Now-playing switches to ep2, which we don't have.
        state.set_now_playing(A, ts(5), Some(hash(2)));
        let view = state.view();
        let directives = wiring.on_state(&view, &[peer("kim")]);
        let cmds = player_cmds(&directives);
        assert!(
            cmds.iter()
                .any(|c| matches!(c, PlayerCommand::SetPlaying(false))),
            "the stale ep1 must be held paused: {cmds:?}"
        );
        assert!(
            !cmds
                .iter()
                .any(|c| matches!(c, PlayerCommand::SetPlaying(true))),
            "must not resume stale ep1 while ep2 is now-playing: {cmds:?}"
        );
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

    fn lookup_requests(directives: &[Directive]) -> Vec<Ed2kHash> {
        directives
            .iter()
            .filter_map(|d| match d {
                Directive::Mutate(Mutation::RequestLookup { info }) => Some(info.hash),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn entries_without_metadata_get_one_lookup_request() {
        let mut wiring = PlayerWiring::new(me());
        let view = playing_state().view();
        let first = wiring.on_state(&view, &[peer("kim")]);
        assert_eq!(lookup_requests(&first), vec![hash(1)]);
        // Not re-requested on the next snapshot.
        let second = wiring.on_state(&view, &[peer("kim")]);
        assert!(lookup_requests(&second).is_empty());
    }

    #[test]
    fn entries_with_metadata_are_not_looked_up() {
        let mut state = playing_state();
        state.set_anidb_metadata(
            A,
            ts(4),
            hash(1),
            Some(dessplay_core::types::AniDbMetadata {
                source: dessplay_core::types::MetadataSource::AniDb,
                series_name: "Frieren".into(),
                series_id: Some(dessplay_core::types::AniDbSeriesId(1)),
                episode_number: Some("1".into()),
            }),
        );
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_state(&state.view(), &[peer("kim")]);
        assert!(lookup_requests(&directives).is_empty());
    }

    fn list_entry(series: Option<u32>, watchers: &[&str]) -> dessplay_core::types::SeriesListEntry {
        dessplay_core::types::SeriesListEntry {
            name: "Some Show".into(),
            nero_name: None,
            genre: None,
            notes: vec![],
            recommender: None,
            status: dessplay_core::types::ListStatus::Active,
            status_note: None,
            source: None,
            watchers: watchers.iter().map(|w| UserId::new(*w)).collect(),
            anidb_series_id: series.map(dessplay_core::types::AniDbSeriesId),
        }
    }

    fn preference_writes(
        directives: &[Directive],
    ) -> Vec<(dessplay_core::types::AniDbSeriesId, SeriesWatchState)> {
        directives
            .iter()
            .filter_map(|d| match d {
                Directive::Mutate(Mutation::SetSeriesPreference { series, pref, .. }) => {
                    Some((*series, *pref))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn non_watchers_of_a_linked_entry_become_not_watching_once() {
        use dessplay_core::types::{AniDbSeriesId, ListEntryId};
        let mut state = CrdtState::new();
        // kim is not in the watchers set.
        state.put_list_entry(A, ts(1), ListEntryId(1), list_entry(Some(7), &["baughn", "nero"]));
        let mut wiring = PlayerWiring::new(me());
        let view = state.view();
        let first = wiring.on_state(&view, &[peer("kim")]);
        assert_eq!(
            preference_writes(&first),
            vec![(AniDbSeriesId(7), SeriesWatchState::NotWatching)]
        );
        // Once per session, not per snapshot.
        let second = wiring.on_state(&view, &[peer("kim")]);
        assert!(preference_writes(&second).is_empty());
    }

    #[test]
    fn watcher_membership_and_manual_choices_are_respected() {
        use dessplay_core::types::ListEntryId;
        let mut state = CrdtState::new();
        // kim watches this one: no write.
        state.put_list_entry(A, ts(1), ListEntryId(1), list_entry(Some(7), &["kim", "nero"]));
        // Unlinked: no write.
        state.put_list_entry(A, ts(2), ListEntryId(2), list_entry(None, &["nero"]));
        // Empty watchers means "unrecorded", not "nobody": no write.
        state.put_list_entry(A, ts(3), ListEntryId(3), list_entry(Some(8), &[]));
        // kim already chose to watch series 9 despite not being listed:
        // the manual preference wins.
        state.put_list_entry(A, ts(4), ListEntryId(4), list_entry(Some(9), &["nero"]));
        state.set_series_preference(
            A,
            ts(5),
            me(),
            dessplay_core::types::AniDbSeriesId(9),
            SeriesWatchState::Watching,
        );
        let mut wiring = PlayerWiring::new(me());
        let directives = wiring.on_state(&state.view(), &[peer("kim")]);
        assert!(
            preference_writes(&directives).is_empty(),
            "got: {:?}",
            preference_writes(&directives)
        );
    }
}
