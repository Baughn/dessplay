//! The production shell around [`super::app::Ui`]: a dedicated UI
//! thread owning the real terminal, an input thread reading crossterm
//! events, and the async bridge that feeds snapshots in and carries
//! [`UserAction`]s out. Tests bypass all of this and drive `Ui`
//! directly — that's the point of the synchronous dispatcher.

use tokio::sync::mpsc;
use tuirealm::event::{Event, MouseButton, MouseEventKind, NoUserEvent};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use super::app::{Ui, UiSnapshot};
use super::msg::UserAction;

/// Frame boundaries for physical output; headless buffers need no escape codes.
pub trait FrameBackend: tuirealm::ratatui::backend::Backend {
    /// Hold the displayed frame while the next frame is written.
    fn begin_frame(&mut self) -> Result<(), Self::Error>;
    /// Publish the completed frame and flush the output stream.
    fn end_frame(&mut self) -> Result<(), Self::Error>;
}

impl<W: std::io::Write> FrameBackend for tuirealm::ratatui::backend::CrosstermBackend<W> {
    fn begin_frame(&mut self) -> std::io::Result<()> {
        crossterm::queue!(self, crossterm::terminal::BeginSynchronizedUpdate)
    }
    fn end_frame(&mut self) -> std::io::Result<()> {
        crossterm::execute!(self, crossterm::terminal::EndSynchronizedUpdate)
    }
}

impl FrameBackend for tuirealm::ratatui::backend::TestBackend {
    fn begin_frame(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
    fn end_frame(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// The only draw boundary in the shell, including startup, input, and idle ticks.
/// The guard also releases synchronization after an I/O error or render panic.
fn draw_frame<B: FrameBackend>(
    terminal: &mut tuirealm::ratatui::Terminal<B>,
    render: impl FnOnce(&mut tuirealm::ratatui::Frame<'_>),
) -> Result<(), B::Error> {
    struct Guard<'a, B: FrameBackend> {
        terminal: &'a mut tuirealm::ratatui::Terminal<B>,
        finished: bool,
    }
    impl<B: FrameBackend> Drop for Guard<'_, B> {
        fn drop(&mut self) {
            if !self.finished {
                let _ = self.terminal.backend_mut().end_frame();
            }
        }
    }
    let mut guard = Guard {
        terminal,
        finished: false,
    };
    guard.terminal.backend_mut().begin_frame()?;
    let drawn = guard.terminal.draw(render).map(|_| ());
    let ended = guard.terminal.backend_mut().end_frame();
    guard.finished = ended.is_ok();
    drawn.and(ended)
}

/// Everything the UI thread consumes.
pub enum UiInput {
    /// A dungeon snapshot after its turn has been saved, or a storage error.
    Roguelike(Result<Box<crate::roguelike::RunView>, String>),
    /// Fresh state to render.
    Snapshot(Box<UiSnapshot>),
    /// A terminal input event.
    Event(Event<NoUserEvent>),
    /// A subtitle line from the local player. `video_millis` is the
    /// in-video position (displayed timestamp); `arrival_millis` is the
    /// wall-clock arrival used to interleave with chat. Local only.
    Subtitle {
        /// Subtitle text.
        text: String,
        /// The ASS speaker/actor, if any (used for optional name display and
        /// to color the line in separate-pane mode).
        speaker: Option<crate::player::SpeakerName>,
        /// In-video position when the cue appeared (milliseconds).
        video_millis: u64,
        /// Wall-clock arrival on the shared clock (milliseconds).
        arrival_millis: u64,
    },
    /// Playlist-add hashing progress (the no-silent-work rule: shown as
    /// a progress overlay).
    Hashing {
        /// File being hashed.
        filename: String,
        /// Bytes hashed so far.
        done_bytes: u64,
        /// File size (0 = unknown).
        total_bytes: u64,
        /// True when this file is done (row removed).
        finished: bool,
    },
    /// A local-only system message for the chat log (e.g. an archive
    /// result). Not synced — it appears only in this client's chat.
    System {
        /// Shared-clock millis (orders the line within the chat log).
        timestamp: u64,
        /// The message body.
        text: String,
    },
    /// The answer to a [`UserAction::FetchChatImage`]: the decoded,
    /// pre-scaled image (or a fetch/decode failure, which leaves the
    /// URL as plain text).
    ChatImage {
        /// The URL this answers, the chat pane's image-store key.
        url: String,
        /// Decoded pixels, or a debug-level error string.
        result: Result<Box<image::DynamicImage>, String>,
    },
    /// A local-only chat line from an external IRC user (the IRC bridge).
    /// Not synced — each client runs its own bridge.
    Irc {
        /// Shared-clock millis (orders the line within the chat log).
        timestamp: u64,
        /// The IRC nick of the sender.
        sender: String,
        /// The message body (CTCP already decoded).
        text: String,
        /// True if the message was a CTCP ACTION (an emote).
        action: bool,
    },
    /// The answer to a [`UserAction::Browse`]: the library index and
    /// watched set the file browser needs, echoing the request. Opens
    /// the browser modal.
    Browse {
        /// The request being answered (which browser, and its anchor).
        request: crate::ui::msg::BrowseRequest,
        /// Every indexed file: (path, ed2k root, mtime millis) from the
        /// hash cache.
        files: Vec<(std::path::PathBuf, dessplay_core::types::Ed2kHash, i64)>,
        /// Personally-watched hashes (the group's flags are unioned in
        /// UI-side from the synced view).
        watched: std::collections::BTreeSet<dessplay_core::types::Ed2kHash>,
        /// Mapping browser: the series' last-used directory.
        start: Option<std::path::PathBuf>,
    },
    /// Open the local-copy offer modal: the missing now-playing file and
    /// the ranked plausible local copies (proposal
    /// 2026-08-31-local-copy-offer). Sent by the main loop when the
    /// session requests an offer and candidates exist.
    LocalCopyOffer {
        /// The missing now-playing file.
        file: dessplay_core::types::Ed2kHash,
        /// Its playlist filename (the modal title).
        filename: String,
        /// Ranked candidates, strong evidence first.
        candidates: Vec<dessplay_core::local_copy::CopyCandidate>,
    },
    /// AniDB name-search results (delivered to the search modal).
    SearchResults {
        /// The query these results answer.
        query: String,
        /// The hits.
        results: Vec<dessplay_core::net::AniDbSearchHit>,
    },
    /// Live work for a locally requested Nyaa search.
    NyaaSearchProgress {
        /// Unique local search request.
        request_id: u64,
        /// Current search work.
        progress: crate::torrent::nyaa::NyaaSearchProgress,
    },
    /// Nyaa single-file browse results for the open modal.
    NyaaResults {
        /// Unique local search request.
        request_id: u64,
        /// Echoed query.
        query: String,
        /// Safe results or request-level failure.
        result: Result<Vec<crate::torrent::nyaa::NyaaBrowseResult>, String>,
    },
    /// Pending Nyaa import progress.
    NyaaImportProgress {
        /// Local pending-import identity.
        id: crate::torrent::engine::TorrentImportId,
        /// Payload filename.
        filename: String,
        /// Current work stage.
        stage: crate::actors::file::NyaaImportStage,
        /// Completed bytes.
        done_bytes: u64,
        /// Total bytes.
        total_bytes: u64,
    },
    /// Remove a pending Nyaa import from local UI state.
    NyaaImportFinished {
        /// Local pending-import identity to remove.
        id: crate::torrent::engine::TorrentImportId,
    },
    /// Restore the terminal and exit the UI thread. The explicit
    /// message exists because channel-closure can't signal it: the
    /// input thread holds a sender clone forever (it's blocked in
    /// `crossterm::event::read`), so the channel never closes.
    Shutdown,
    /// Test-only latency probe: the UI loop stamps `Instant::now()` into
    /// the cell the moment it dequeues this input, then draws like any
    /// other input. Lets a test measure how long the UI thread takes to
    /// service new input while a snapshot flood is in flight. Always
    /// present (no cargo feature) to keep it reachable from the
    /// integration-test crate; the production loop never sends it.
    #[doc(hidden)]
    Probe(std::sync::Arc<std::sync::Mutex<Option<std::time::Instant>>>),
}

/// Run the UI on the current (dedicated) thread until the input
/// channel closes or the action receiver goes away. Returns the
/// terminal to its normal state on exit, and returns what the caller
/// saves ([`UiExit`], see [`run_ui_loop`]); nothing when the terminal
/// couldn't be set up.
///
/// `on_terminal_ready` is called exactly once, after every stdin
/// round-trip this thread performs (the image-protocol query below)
/// and before the first event could be consumed. The caller uses it
/// to start the input thread: spawning that thread earlier would
/// race its `event::read` against the query's reply on stdin (the
/// same stdin-ownership hazard as the `Terminal::clear` ban in
/// [`run_ui_loop`]). It fires on the error paths too — a failed
/// terminal setup must not leave the caller waiting forever.
pub fn run_ui_thread(
    mut ui: Ui,
    inputs: super::delivery::UiReceiver,
    actions: mpsc::Sender<UserAction>,
    on_terminal_ready: impl FnOnce(),
) -> UiExit {
    // Drop guard: `on_terminal_ready` fires on every exit path, early
    // returns included.
    struct Ready<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for Ready<F> {
        fn drop(&mut self) {
            if let Some(f) = self.0.take() {
                f();
            }
        }
    }
    let ready = Ready(Some(on_terminal_ready));

    let started = std::time::Instant::now();
    tracing::debug!("UI thread started");
    let mut adapter = match CrosstermTerminalAdapter::new() {
        Ok(adapter) => adapter,
        Err(e) => {
            tracing::error!("cannot initialize the terminal: {e}");
            return UiExit::default();
        }
    };
    // The adapter enables nothing by itself: raw mode so keys arrive as
    // events (arrows are escape sequences — line buffering eats them),
    // the alternate screen so we don't paint over scrollback. restore()
    // undoes exactly what was enabled.
    if let Err(e) = adapter
        .enable_raw_mode()
        .and_then(|()| adapter.enter_alternate_screen())
    {
        tracing::error!("cannot set up the terminal: {e}");
        let _ = adapter.restore();
        return UiExit::default();
    }
    // Establish the cursor-addressing state we depend on instead of
    // inheriting whatever the previous occupant of this terminal left
    // behind (design.md, Inline chat images; decisions.md, "The terminal
    // state prologue"). Non-fatal: a terminal that ignores these is one
    // that never had the modes to begin with.
    if let Err(e) = crossterm::execute!(
        std::io::stdout(),
        crossterm::style::Print(TERMINAL_STATE_PROLOGUE)
    ) {
        tracing::warn!("cannot reset terminal margins: {e}");
    }
    // Bracketed paste (design.md #33): without it, a terminal delivers
    // pasted text as a stream of individual key-press events instead of
    // one `Event::Paste`, so a dropped-in file path can't be told apart
    // from typing. `enable_bracketed_paste` is an inherent method on the
    // concrete crossterm adapter (not part of `TerminalAdapter`), and
    // isn't tracked by `restore()` — it never emits `DisableBracketedPaste`
    // — so we explicitly undo it ourselves below.
    if let Err(e) = adapter.enable_bracketed_paste() {
        tracing::warn!("cannot enable bracketed paste: {e}");
    }
    // Mouse capture (design.md, Mouse support): clicks focus panes and
    // select list rows, the wheel scrolls, and dragging selects chat
    // text for copying. Non-fatal — a terminal
    // without mouse reporting keeps the full keyboard UI. Unlike
    // bracketed paste this *is* tracked by the adapter, so restore()
    // (and the panic hook) disable it on exit.
    if let Err(e) = adapter.enable_mouse_capture() {
        tracing::warn!("cannot enable mouse capture: {e}");
    }
    let color_depth = super::theme::ColorDepth::detect();
    ui.set_color_depth(color_depth);
    // Image-protocol detection round-trips stdio (it writes a query and
    // reads the terminal's reply from stdin), so it MUST complete before
    // the input thread starts and its `event::read` owns stdin — that's
    // the `on_terminal_ready` contract above. Ordered after the alt
    // screen is entered, per ratatui-image's docs. Besides choosing the
    // protocol, the query reports the real font cell size, which gives
    // inline images the correct aspect ratio under every renderer.
    let picker = select_image_picker();
    tracing::debug!(
        protocol = ?picker.protocol_type(),
        font_size = ?picker.font_size(),
        "image protocol selected"
    );
    ui.set_image_picker(picker);
    drop(ready);
    tracing::debug!(
        ?color_depth,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "terminal setup complete"
    );
    let exit = run_ui_loop(ui, inputs, actions, &mut adapter);
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
    let _ = adapter.restore();
    tracing::debug!("UI thread exiting");
    exit
}

/// What the UI loop hands back when it ends, for the caller to save:
/// the local state whose last change its periodic handouts may not
/// have got out (see [`run_ui_loop`]). `None` fields are unchanged
/// since startup, and need no save.
#[derive(Debug, Default)]
pub struct UiExit {
    /// The houseguest's ledger
    /// ([`Guest::final_ledger`](super::houseguest::Guest::final_ledger)).
    pub houseguest: Option<super::houseguest::Ledger>,
    /// The pane sizes, when they differ from the ones loaded at startup.
    pub layout: Option<super::layout::LayoutSettings>,
}

/// Escapes that put the terminal's cursor addressing into the state the
/// renderer assumes, written once after entering the alternate screen.
///
/// In order: DECLRMM off (`CSI ?69l`), scroll region reset to the full
/// screen (`CSI r`), origin mode off (`CSI ?6l`). None of these are
/// changed by the alternate-screen switch (ghostty keeps margins per
/// terminal, not per screen), so a previous program that left them set
/// — a suspended TUI, say — would otherwise leak into our frames. The
/// concrete casualty was left/right margin mode: with it on, `CSI s`
/// means "set margins" (and homes the cursor) rather than "save
/// cursor", which is the sequence ratatui-image embeds in every Kitty
/// image row, so each row of a picture printed at the screen's
/// top-left instead of under its message.
pub const TERMINAL_STATE_PROLOGUE: &str = "\x1b[?69l\x1b[r\x1b[?6l";

/// Pick the image-rendering protocol for inline chat images.
///
/// Defaults to whatever the terminal's query reports (Kitty, sixel, or
/// iTerm2 graphics where supported, half-blocks otherwise). The graphics
/// protocols embed cursor-moving escapes inside cell symbols; those are
/// correct as emitted and depend only on the terminal state established
/// by [`TERMINAL_STATE_PROLOGUE`]. A terminal that never answers the
/// query gets half-blocks at an assumed font size — the one path that
/// works everywhere. Even for half-blocks the query matters: it reports
/// the real font cell size, so `▀` cells render at the true aspect ratio.
///
/// `DESSPLAY_IMAGE_PROTOCOL` overrides the choice: `halfblocks` opts
/// out of graphics (the robust path, keeping the queried font size);
/// `kitty`, `sixel`, or `iterm2` force a protocol the query did not
/// pick, for experiments; `auto` (or unset) is the default.
pub fn select_image_picker() -> ratatui_image::picker::Picker {
    use ratatui_image::picker::{Picker, ProtocolType};
    let mut picker = match Picker::from_query_stdio() {
        Ok(picker) => picker,
        Err(e) => {
            tracing::debug!("image protocol query failed ({e}); using half-blocks");
            return Picker::halfblocks();
        }
    };
    let forced = std::env::var("DESSPLAY_IMAGE_PROTOCOL").ok();
    match forced
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        // The opt-out: ordinary cells, queried font size kept for the
        // aspect ratio.
        Some("halfblocks") => picker.set_protocol_type(ProtocolType::Halfblocks),
        Some("kitty") => picker.set_protocol_type(ProtocolType::Kitty),
        Some("sixel") => picker.set_protocol_type(ProtocolType::Sixel),
        Some("iterm2") => picker.set_protocol_type(ProtocolType::Iterm2),
        // Default (unset, "auto", or anything unrecognized): the
        // detected protocol, untouched.
        Some(other) if other != "auto" => {
            tracing::warn!(
                value = other,
                "unknown DESSPLAY_IMAGE_PROTOCOL; using detected"
            );
        }
        _ => {}
    }
    picker
}

/// The terminal-agnostic UI loop body: apply inputs and redraw until the
/// input channel closes (or a quit action arrives). Generic over the
/// [`TerminalAdapter`] so the production path drives a real crossterm
/// terminal while tests drive a headless `TestTerminalAdapter` and run
/// the *real* draw/refresh work. The caller owns terminal
/// setup/teardown (raw mode, alternate screen, `restore`).
///
/// Returns what the caller saves once the loop has ended ([`UiExit`]:
/// the houseguest's ledger and the pane sizes, when changed). Every
/// exit — quit, shutdown, a closed channel, a failed draw — leaves the
/// loop through the one exit at its end, so none can skip that last
/// save: the drains at the loop's top miss a change made in the last
/// iteration, a handout parked on a full action queue, and one queued
/// but dropped when the session ends. The loop runs inside [`turns`],
/// so a `return` in it can only end the turns, never skip the exit.
pub fn run_ui_loop<A: TerminalAdapter>(
    mut ui: Ui,
    inputs: super::delivery::UiReceiver,
    actions: mpsc::Sender<UserAction>,
    adapter: &mut A,
) -> UiExit
where
    A::Backend: FrameBackend,
{
    let builtin = match super::layout::LayoutBundle::builtin() {
        Ok(bundle) => bundle,
        Err(error) => {
            // Before anything could change: nothing to save.
            tracing::error!(%error, "embedded layout is invalid");
            return UiExit::default();
        }
    };
    let mut renderer = super::layout::Renderer::new(builtin.clone());
    let mut custom_enabled = ui.layout_options.as_ref().is_some_and(|o| !o.builtin);
    let watcher = ui.layout_options.as_ref().map(|options| {
        let directory = options
            .directory
            .clone()
            .unwrap_or_else(super::layout::default_directory);
        let directory = if directory.is_absolute() {
            directory
        } else {
            std::env::current_dir().unwrap_or_default().join(directory)
        };
        let watcher = super::layout::Watcher::start(directory);
        if custom_enabled {
            watcher.reload();
        }
        watcher
    });
    // Deliberately NO Terminal::clear() here (or anywhere while the
    // input thread lives): ratatui's clear() queries the cursor
    // position, and crossterm answers that by reading the terminal's
    // reply from stdin — which our input thread's event::read() starves,
    // so the query burns its full 2-second timeout. The alternate
    // screen is already blank and the first fullscreen draw paints
    // every cell.
    // The idle houseguest: a post-render overlay that reads each frame
    // and paints over it, never feeding back into `ui`.
    let mut guest = match ui.houseguest_ledger.take() {
        Some(Ok(ledger)) => super::houseguest::Guest::restore(ledger),
        None => super::houseguest::Guest::new(rand::random()),
        Some(Err(())) => {
            let mut guest = super::houseguest::Guest::new(rand::random());
            guest.keep_unsaved();
            guest
        }
    };
    let mut ledger_unsent: Option<super::houseguest::Ledger> = None;
    let layout_loaded = ui.layout_settings.clone();
    if let Some(picker) = ui.image_picker() {
        guest.set_picker(picker);
    }
    let _ = draw(adapter, &mut ui, &mut renderer, &mut guest);
    // Every exit below is a `break 'ui`; inside `turns` even a `return`
    // only ends the turns, and the exit after them runs regardless.
    turns(|| {
        'ui: loop {
            take_move_out(&mut ui, &mut guest);
            if let Some(ledger) = guest.ledger_to_save() {
                ledger_unsent = Some(ledger);
            }
            if let Some(ledger) = ledger_unsent.take() {
                match actions.try_send(UserAction::SaveHouseguest(ledger)) {
                    Ok(()) => {}
                    Err(mpsc::error::TrySendError::Full(UserAction::SaveHouseguest(ledger))) => {
                        ledger_unsent = Some(ledger);
                    }
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => break 'ui,
                }
            }
            if ui.layout_settings_dirty {
                match actions.try_send(UserAction::SaveLayoutSettings(ui.layout_settings.clone())) {
                    Ok(()) => ui.layout_settings_dirty = false,
                    Err(mpsc::error::TrySendError::Full(_)) => {}
                    Err(mpsc::error::TrySendError::Closed(_)) => break 'ui,
                }
            }
            // Adaptive cadence: ~100ms while a marquee pass animates, the
            // lazy 1s otherwise. Idle cost is unchanged — the timeout arm
            // only repaints when advance_clock reports a visible change.
            let timeout = guest
                .next_tick(now_millis())
                .map_or(ui.next_tick_hint(), |due| due.min(ui.next_tick_hint()));
            let input = match inputs.recv_timeout(timeout) {
                Ok(input) => input,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    let now = now_millis();
                    let mut redraw = ui.advance_clock(now);
                    redraw |= guest.advance(now);
                    redraw |= poll_layout(&mut ui, &mut renderer, watcher.as_ref(), custom_enabled);
                    redraw |= dispatch_due_recovery(&mut ui, &actions);
                    dispatch_image_fetches(&mut ui, &actions);
                    if redraw && draw(adapter, &mut ui, &mut renderer, &mut guest).is_err() {
                        break 'ui;
                    }
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break 'ui,
            };
            // Freshen the animator clock to the dequeue moment before any
            // handler runs: animation starts (marquee passes, spoiler
            // teases) and TTL stamps read `Ui::clock`, which would
            // otherwise be up to one lazy tick stale — and snapshots
            // advance running animations this way too (~10Hz during
            // playback). The redraw hint is moot; every input is followed
            // by a draw below anyway.
            let now = now_millis();
            let _ = ui.advance_clock(now);
            let _ = guest.advance(now);
            match input {
                UiInput::Roguelike(result) => ui.set_roguelike(result),
                UiInput::Shutdown => break 'ui,
                UiInput::Snapshot(snapshot) => ui.apply_snapshot(*snapshot),
                UiInput::Subtitle {
                    text,
                    speaker,
                    video_millis,
                    arrival_millis,
                } => ui.push_subtitle(video_millis, arrival_millis, text, speaker),
                UiInput::Hashing {
                    filename,
                    done_bytes,
                    total_bytes,
                    finished,
                } => ui.set_hash_progress(filename, done_bytes, total_bytes, finished),
                UiInput::System { timestamp, text } => ui.push_system(timestamp, text),
                UiInput::ChatImage { url, result } => {
                    ui.set_chat_image(&url, result.map(|img| *img))
                }
                UiInput::Irc {
                    timestamp,
                    sender,
                    text,
                    action,
                } => ui.push_irc(timestamp, sender, text, action),
                UiInput::Browse {
                    request,
                    files,
                    watched,
                    start,
                } => ui.open_file_browser(request, files, watched, start),
                UiInput::LocalCopyOffer {
                    file,
                    filename,
                    candidates,
                } => ui.offer_local_copies(file, filename, candidates),
                UiInput::SearchResults { query, results } => {
                    for action in ui.set_search_results(&query, results) {
                        if actions.blocking_send(action).is_err() {
                            tracing::debug!("UI thread exiting (actions channel closed)");
                            break 'ui;
                        }
                    }
                }
                UiInput::NyaaSearchProgress {
                    request_id,
                    progress,
                } => ui.set_nyaa_search_progress(request_id, progress),
                UiInput::NyaaResults {
                    request_id,
                    query,
                    result,
                } => ui.set_nyaa_results(request_id, &query, result),
                UiInput::NyaaImportProgress {
                    id,
                    filename,
                    stage,
                    done_bytes,
                    total_bytes,
                } => ui.set_nyaa_import_progress(id, filename, stage, done_bytes, total_bytes),
                UiInput::NyaaImportFinished { id } => ui.finish_nyaa_import(id),
                UiInput::Probe(cell) => {
                    // Stamp the moment we dequeued this input — measured by a
                    // test against the send time. Fall through to a draw so
                    // the probe pays the same per-input cost real input does.
                    // Recover a poisoned lock (the stamp is the only state):
                    // the crate denies `unwrap`, and a panicking probe would
                    // be a poor reason to take down the UI loop.
                    *cell
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(std::time::Instant::now());
                }
                UiInput::Event(event) => {
                    // Local input sends the houseguest away (a resident stays,
                    // putting back what she moved in the chat); a resize only
                    // re-anchors her, and focus changes (alt-tab) are not
                    // activity.
                    if matches!(
                        event,
                        Event::Keyboard(_) | Event::Mouse(_) | Event::Paste(_)
                    ) {
                        guest.activity(now);
                    }
                    for action in ui.handle(event) {
                        let quit = action == UserAction::Quit;
                        if actions.blocking_send(action).is_err() || quit {
                            tracing::debug!("UI thread exiting (quit or actions channel closed)");
                            break 'ui;
                        }
                    }
                }
            }
            dispatch_due_recovery(&mut ui, &actions);
            dispatch_image_fetches(&mut ui, &actions);
            match ui.layout_request.take() {
                Some('r') => {
                    custom_enabled = true;
                    if let Some(watcher) = &watcher {
                        watcher.reload();
                    }
                }
                Some('b') => {
                    custom_enabled = false;
                    renderer.install(builtin.clone());
                    ui.cancel_layout_grabs();
                }
                _ => {}
            }
            poll_layout(&mut ui, &mut renderer, watcher.as_ref(), custom_enabled);
            if draw(adapter, &mut ui, &mut renderer, &mut guest).is_err() {
                break 'ui;
            }
        }
    });
    // The one exit: whatever ended the loop, what changed goes back to
    // the caller to save (a parked or dropped handout included, and a
    // move-out confirmed in the last turn).
    take_move_out(&mut ui, &mut guest);
    let exit = UiExit {
        houseguest: guest.final_ledger(),
        layout: (ui.layout_settings != layout_loaded).then(|| ui.layout_settings.clone()),
    };
    tracing::trace!(
        houseguest = exit.houseguest.is_some(),
        parked = ledger_unsent.is_some(),
        layout = exit.layout.is_some(),
        "UI state handed back at exit"
    );
    exit
}

/// Run the UI loop's turns. Its `()` result makes a `return` with a
/// value inside them a type error: they can only end, and whatever
/// follows (the exit save) runs however they did.
fn turns(turns: impl FnOnce()) {
    turns();
}

/// Apply a confirmed "Osaka moved out" (the settings screen sets the
/// flag). The loop's top and its exit both look, so a move-out
/// confirmed in the last turn isn't lost.
fn take_move_out(ui: &mut Ui, guest: &mut super::houseguest::Guest) {
    if std::mem::take(&mut ui.houseguest_moved_out) {
        guest.move_out(rand::random());
    }
}

/// Draw one frame: the UI, then the houseguest over it. The idle view is
/// built after the UI's draw, which measures the pane rectangles.
fn draw<A: TerminalAdapter>(
    adapter: &mut A,
    ui: &mut Ui,
    renderer: &mut super::layout::Renderer,
    guest: &mut super::houseguest::Guest,
) -> Result<(), <A::Backend as tuirealm::ratatui::backend::Backend>::Error>
where
    A::Backend: FrameBackend,
{
    let now = now_millis();
    draw_frame(adapter.raw_mut(), |frame| {
        ui.draw_with_renderer(frame, renderer);
        let view = ui.idle_view(renderer.image_regions());
        guest.paint(frame.buffer_mut(), &view, now);
    })
}

fn poll_layout(
    ui: &mut Ui,
    renderer: &mut super::layout::Renderer,
    watcher: Option<&super::layout::Watcher>,
    enabled: bool,
) -> bool {
    let mut changed = false;
    if let Some(error) = watcher.and_then(|w| w.error()) {
        ui.layout_watch_error = error.to_string();
        changed = true;
    }
    let Some(result) = watcher.and_then(|w| w.poll()) else {
        return changed;
    };
    if !enabled {
        return changed;
    }
    match result {
        Ok(bundle) => {
            tracing::info!(revision = %bundle.revision, "layout installed");
            renderer.install(bundle);
            ui.cancel_layout_grabs();
            ui.layout_message = "Layout valid".into();
        }
        Err(error) => {
            tracing::warn!(%error, "keeping the last working layout");
            ui.layout_message = format!("{error}\nKeeping the last working layout");
        }
    }
    true
}

/// Forward queued chat-image fetch requests to the main loop. Lossy on
/// a full channel by re-queueing: the request stays pending in the `Ui`
/// and is retried on the next input or tick, so a burst of actions
/// can't silently strand an image in its loading state.
fn dispatch_image_fetches(ui: &mut Ui, actions: &mpsc::Sender<UserAction>) {
    for url in ui.take_image_fetches() {
        if let Err(error) = actions.try_send(UserAction::FetchChatImage { url }) {
            match error {
                mpsc::error::TrySendError::Full(UserAction::FetchChatImage { url }) => {
                    ui.requeue_image_fetch(url);
                }
                _ => return,
            }
        }
    }
}

/// Avoid blocking the terminal on automated work. A full or closed queue
/// interrupts recovery and leaves the last committed observation intact.
fn dispatch_due_recovery(ui: &mut Ui, actions: &mpsc::Sender<UserAction>) -> bool {
    let Some(action) = ui.due_roguelike_action() else {
        return false;
    };
    if let Err(error) = actions.try_send(action) {
        ui.set_roguelike(Err(format!("recovery interrupted: {error}")));
    }
    true
}

/// Monotonic millis since the first call — the UI thread's only time
/// source (the `Ui` itself never reads a clock; tests drive it with
/// synthetic millis). Monotonic by construction (`Instant`): wall-clock
/// steps (NTP corrections) must never reach `Ui::clock`, whose
/// consumers all measure elapsed local time — a backward step would
/// freeze every animator for the size of the step (2026-08-20 review).
fn now_millis() -> u64 {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

/// Coarse event description for trace logs. Deliberately omits the
/// event's contents: keystrokes can include a password being typed
/// into the settings modal, and this thread cannot see modal state.
/// [`Ui::handle`] logs full contents with modal-aware redaction.
fn event_kind(event: &Event<NoUserEvent>) -> &'static str {
    match event {
        Event::Keyboard(_) => "keyboard",
        Event::Mouse(_) => "mouse",
        Event::WindowResize(..) => "resize",
        Event::FocusGained => "focus-gained",
        Event::FocusLost => "focus-lost",
        Event::Paste(_) => "paste",
        Event::Tick => "tick",
        Event::User(_) | Event::None => "other",
    }
}

/// Whether a raw crossterm event is a left-button drag (the kind a
/// selection produces in bursts, and the only kind worth coalescing).
fn is_left_drag(event: &crossterm::event::Event) -> bool {
    matches!(
        event,
        crossterm::event::Event::Mouse(m)
            if m.kind == crossterm::event::MouseEventKind::Drag(crossterm::event::MouseButton::Left)
    )
}

/// Read crossterm events on the current (dedicated) thread, forwarding
/// them as [`UiInput::Event`]s until the channel closes.
pub fn run_input_thread(inputs: super::delivery::UiSender) {
    tracing::debug!("input thread started");
    // An event read ahead while coalescing a drag run, still to forward.
    let mut queued: Option<crossterm::event::Event> = None;
    loop {
        let mut raw = match queued.take().map(Ok).unwrap_or_else(crossterm::event::read) {
            Ok(event) => event,
            Err(e) => {
                tracing::error!("terminal input died: {e}");
                return;
            }
        };
        // A selection drag emits motion events faster than the UI
        // thread's full-redraw-per-event can drain; coalesce each run
        // of drags to its newest point (the intermediate points draw
        // nothing the final one doesn't).
        while is_left_drag(&raw)
            && matches!(crossterm::event::poll(std::time::Duration::ZERO), Ok(true))
        {
            match crossterm::event::read() {
                Ok(next) if is_left_drag(&next) => raw = next,
                Ok(next) => {
                    queued = Some(next);
                    break;
                }
                Err(e) => {
                    tracing::error!("terminal input died: {e}");
                    return;
                }
            }
        }
        let event: Event<NoUserEvent> = raw.into();
        if matches!(event, Event::None) {
            continue;
        }
        // Mouse capture reports every motion, but only button presses,
        // left-button selection drag / release, and wheel ticks do
        // anything — and each forwarded event costs a full redraw on
        // the UI thread, so drop the rest here.
        if let Event::Mouse(mouse) = &event
            && !matches!(
                mouse.kind,
                MouseEventKind::Down(_)
                    | MouseEventKind::Drag(MouseButton::Left)
                    | MouseEventKind::Up(MouseButton::Left)
                    | MouseEventKind::ScrollUp
                    | MouseEventKind::ScrollDown
            )
        {
            continue;
        }
        tracing::trace!(kind = event_kind(&event), "input event forwarded");
        if inputs.send(UiInput::Event(event)).is_err() {
            tracing::debug!("input thread exiting (channel closed)");
            return;
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod roguelike_tests {
    use super::*;
    use crate::config::Settings;
    use crate::roguelike::{Action, Run, RunView};
    use crate::roguelike_store::Command;
    use dessplay_core::types::UserId;
    use tuirealm::event::{Key, KeyEvent, KeyModifiers};
    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }
    fn resting() -> (Ui, RunView) {
        let mut ui = Ui::with_setup(
            UserId::new("kim"),
            Settings {
                username: Some("kim".into()),
                password: Some("test".into()),
                ..Settings::default()
            },
            vec![],
            false,
        );
        assert_eq!(
            ui.handle(key(Key::Function(4))),
            vec![UserAction::Roguelike(Command::Open)]
        );
        let mut view = Run::new(19).view();
        view.can_rest = true;
        view.danger = false;
        view.last_step.changed = true;
        view.last_step.interrupted = false;
        ui.set_roguelike(Ok(Box::new(view.clone())));
        assert_eq!(
            ui.handle(key(Key::Char('r'))),
            vec![UserAction::Roguelike(Command::Act(Action::Rest))]
        );
        (ui, view)
    }
    #[test]
    fn busy_shell_dispatches_once_after_committed_ack() {
        let (mut ui, view) = resting();
        let (sender, mut receiver) = mpsc::channel(1);
        ui.advance_clock(1000);
        assert!(!dispatch_due_recovery(&mut ui, &sender));
        ui.set_roguelike(Ok(Box::new(view)));
        ui.advance_clock(1250);
        assert!(dispatch_due_recovery(&mut ui, &sender));
        assert_eq!(
            receiver.try_recv().unwrap(),
            UserAction::Roguelike(Command::Act(Action::Rest))
        );
        ui.advance_clock(10000);
        assert!(!dispatch_due_recovery(&mut ui, &sender));
    }
    #[test]
    fn full_and_closed_action_queues_stop_recovery_without_waiting() {
        for closed in [false, true] {
            let (mut ui, view) = resting();
            let (sender, receiver) = mpsc::channel(1);
            if closed {
                drop(receiver);
            } else {
                sender.try_send(UserAction::Quit).unwrap();
            }
            ui.set_roguelike(Ok(Box::new(view.clone())));
            ui.advance_clock(250);
            assert!(dispatch_due_recovery(&mut ui, &sender));
            ui.set_roguelike(Ok(Box::new(view)));
            ui.advance_clock(1000);
            assert!(!dispatch_due_recovery(&mut ui, &sender));
        }
    }
    #[test]
    fn covering_closing_and_input_cancel_before_late_replies() {
        for key_code in [
            Key::Char('h'),
            Key::Function(11),
            Key::Function(4),
            Key::Esc,
        ] {
            let (mut ui, view) = resting();
            ui.handle(key(key_code));
            ui.set_roguelike(Ok(Box::new(view)));
            ui.advance_clock(1000);
            if key_code == Key::Function(11) {
                ui.handle(key(Key::Esc));
            }
            assert!(ui.due_roguelike_action().is_none());
        }
    }
}

#[cfg(test)]
mod prologue_tests {
    use super::TERMINAL_STATE_PROLOGUE;

    /// The prologue resets exactly the three modes that remap absolute
    /// cursor moves or repurpose `CSI s`: left/right margin mode, the
    /// scroll region, and origin mode. Anything else here would be a
    /// silent change to what every frame assumes.
    #[test]
    fn prologue_resets_margin_region_and_origin() {
        assert_eq!(TERMINAL_STATE_PROLOGUE, "\x1b[?69l\x1b[r\x1b[?6l");
    }
}

#[cfg(test)]
mod frame_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::draw_frame;
    use std::{cell::RefCell, io, rc::Rc};
    use tuirealm::ratatui::{
        Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect,
    };

    #[derive(Clone, Default)]
    struct Output(Rc<RefCell<Vec<u8>>>);
    impl io::Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.borrow_mut().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn synchronization_is_released_after_output_failure_and_render_panic() {
        struct FailOnce {
            output: Output,
            failed: bool,
        }
        impl io::Write for FailOnce {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if !self.failed && bytes.contains(&b'X') {
                    self.failed = true;
                    return Err(io::Error::other("injected output failure"));
                }
                self.output.write(bytes)
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let output = Output::default();
        let mut terminal = Terminal::with_options(
            CrosstermBackend::new(FailOnce {
                output: output.clone(),
                failed: false,
            }),
            TerminalOptions {
                viewport: Viewport::Fixed(Rect::new(0, 0, 30, 6)),
            },
        )
        .unwrap();
        let error = draw_frame(&mut terminal, |frame| {
            frame.render_widget("X", frame.area())
        });
        assert!(error.is_err());
        assert!(output.0.borrow().starts_with(b"\x1b[?2026h"));
        assert!(output.0.borrow().ends_with(b"\x1b[?2026l"));

        output.0.borrow_mut().clear();
        let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = draw_frame(&mut terminal, |_| panic!("injected render panic"));
        }));
        assert!(panic.is_err());
        assert!(output.0.borrow().starts_with(b"\x1b[?2026h"));
        assert!(output.0.borrow().ends_with(b"\x1b[?2026l"));
    }

    proptest::proptest! {
        #[test]
        fn terminal_repaints_are_complete_synchronized_frames(
            texts in proptest::collection::vec("[a-z ]{1,120}", 2..8)
        ) {
            let output = Output::default();
            let mut terminal = Terminal::with_options(
                CrosstermBackend::new(output.clone()),
                TerminalOptions { viewport: Viewport::Fixed(Rect::new(0, 0, 30, 6)) },
            ).unwrap();
            for text in texts {
                output.0.borrow_mut().clear();
                draw_frame(&mut terminal, |frame| {
                    frame.render_widget(text.as_str(), frame.area());
                    frame.set_cursor_position((1, 1));
                }).unwrap();
                let bytes = output.0.borrow();
                proptest::prop_assert!(bytes.starts_with(b"\x1b[?2026h"), "repaint starts without synchronization");
                proptest::prop_assert!(bytes.ends_with(b"\x1b[?2026l"), "repaint does not release synchronization");
            }
        }
    }
}

#[cfg(test)]
mod exit_save_tests {
    //! The exit save: every way out of [`run_ui_loop`] hands back what
    //! changed since startup — the houseguest's ledger, the pane sizes —
    //! for the caller to save (design.md, Houseguest; phase 5b D1).
    //!
    //! Not staged: a draw failing on an idle tick (the timeout arm's
    //! redraw needs a visible change on a tick, which isn't
    //! deterministic here), and a ledger change recorded by
    //! `guest.advance` in the last turn (needs her clock; the design
    //! calls it unstageable). Both exit through [`turns`], whose `()`
    //! result leaves them no way past the exit save; the draw-failure
    //! row below covers a change made in the exiting turn.
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use std::cell::Cell;
    use std::rc::Rc;

    use super::super::houseguest::Ledger;
    use super::super::layout::{LayoutBundle, LayoutSettings};
    use super::super::msg::Msg;
    use super::*;
    use crate::config::Settings;
    use dessplay_core::state::CrdtState;
    use dessplay_core::types::{
        ActorId, ListEntryId, ListStatus, SeriesListEntry, SharedTimestamp, UserId,
    };
    use tuirealm::event::{Key, KeyEvent, KeyModifiers};
    use tuirealm::ratatui::layout::Rect;
    use tuirealm::ratatui::{Terminal, TerminalOptions, Viewport};
    use tuirealm::terminal::{TerminalAdapter, TerminalResult};

    /// Output that starts failing once `fail` is set.
    struct Sink(Rc<Cell<bool>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.0.get() {
                Err(std::io::Error::other("the terminal went away"))
            } else {
                Ok(buf.len())
            }
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.write(&[]).map(|_| ())
        }
    }

    /// A headless terminal whose `fail_at`th draw (counting the one
    /// before the loop, whose result is ignored) and every later one
    /// fail. Each draw takes the terminal once through `raw_mut`. A
    /// fixed viewport never asks the real terminal its size.
    struct FlakyAdapter {
        terminal: Terminal<tuirealm::ratatui::backend::CrosstermBackend<Sink>>,
        fail: Rc<Cell<bool>>,
        draws: usize,
        fail_at: usize,
    }

    impl FlakyAdapter {
        fn new(fail_at: usize) -> Self {
            let fail = Rc::new(Cell::new(false));
            let backend = tuirealm::ratatui::backend::CrosstermBackend::new(Sink(fail.clone()));
            let terminal = Terminal::with_options(
                backend,
                TerminalOptions {
                    viewport: Viewport::Fixed(Rect::new(0, 0, 120, 40)),
                },
            )
            .unwrap();
            Self {
                terminal,
                fail,
                draws: 0,
                fail_at,
            }
        }
    }

    impl TerminalAdapter for FlakyAdapter {
        type Backend = tuirealm::ratatui::backend::CrosstermBackend<Sink>;
        fn enable_raw_mode(&mut self) -> TerminalResult<()> {
            Ok(())
        }
        fn disable_raw_mode(&mut self) -> TerminalResult<()> {
            Ok(())
        }
        fn enter_alternate_screen(&mut self) -> TerminalResult<()> {
            Ok(())
        }
        fn leave_alternate_screen(&mut self) -> TerminalResult<()> {
            Ok(())
        }
        fn enable_mouse_capture(&mut self) -> TerminalResult<()> {
            Ok(())
        }
        fn disable_mouse_capture(&mut self) -> TerminalResult<()> {
            Ok(())
        }
        fn raw_mut(&mut self) -> &mut Terminal<Self::Backend> {
            self.draws += 1;
            if self.draws >= self.fail_at {
                self.fail.set(true);
            }
            &mut self.terminal
        }
        fn raw(&self) -> &Terminal<Self::Backend> {
            &self.terminal
        }
    }

    /// How the loop is made to end.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Exit {
        /// `UiInput::Shutdown`, as run.rs sends after the session ends.
        Shutdown,
        /// Ctrl-C: `UserAction::Quit`, sent and then the loop ends.
        Quit,
        /// Every input sender dropped: the mailbox disconnects.
        InputsClosed,
        /// The action receiver dropped: the first handout's send fails
        /// (her ledger's, else the pane sizes'). No input follows, so a
        /// case with nothing to hand out would hang, not pass.
        ActionsClosed,
        /// The action receiver dropped, then an AniDB search answers
        /// empty for an open link search: the List flag's send fails.
        SearchFails,
        /// "Osaka moved out" confirmed with `y`, and that turn's closing
        /// draw fails: the move-out is still pending when the loop ends.
        DrawFails,
    }

    /// Her stored record.
    #[derive(Clone, Copy, Debug)]
    enum Record {
        /// She never visited.
        Never,
        /// A record of three visits ([`stored`]).
        Stored,
        /// One that couldn't be read (kept as it is).
        Unreadable,
    }

    /// One way of running the loop.
    #[derive(Clone, Copy, Debug)]
    struct Case {
        record: Record,
        /// "Osaka moved out" confirmed before the first turn (changing
        /// her ledger at its top).
        moved_out: bool,
        exit: Exit,
        /// The action queue is full before the loop starts, so every
        /// try-send handout is parked.
        parked: bool,
        /// The pane sizes as stored: `false` for none, so the first
        /// draw fills them in and they're handed out like any change.
        layout_stored: bool,
    }

    impl Case {
        fn new(record: Record, exit: Exit) -> Self {
            Self {
                record,
                moved_out: false,
                exit,
                parked: false,
                layout_stored: true,
            }
        }
    }

    /// A stored record that isn't a first meeting, so a moved-out one
    /// (no visits) differs from it whatever seed the move drew.
    fn stored() -> Ledger {
        let mut raw: serde_json::Value = serde_json::from_str(&Ledger::new(7).to_json()).unwrap();
        raw["visits"] = 3.into();
        Ledger::from_json(&raw.to_string()).unwrap()
    }

    fn visits(ledger: &Ledger) -> u64 {
        let raw: serde_json::Value = serde_json::from_str(&ledger.to_json()).unwrap();
        raw["visits"].as_u64().unwrap()
    }

    fn key(c: char, modifiers: KeyModifiers) -> UiInput {
        UiInput::Event(Event::Keyboard(KeyEvent {
            code: Key::Char(c),
            modifiers,
        }))
    }

    /// An unlinked List entry, its AniDB search open (awaiting a reply).
    fn open_link_search(ui: &mut Ui) {
        let id = ListEntryId(1);
        let mut state = CrdtState::new();
        state.put_list_entry(
            ActorId::SERVER,
            SharedTimestamp(1),
            id,
            SeriesListEntry {
                name: "Some Obscure Show".into(),
                nero_name: None,
                genre: None,
                notes: Vec::new(),
                recommender: None,
                status: ListStatus::Active,
                status_note: None,
                source: None,
                watchers: Default::default(),
                anidb_series_id: None,
                local_aliases: Default::default(),
                manual_files: Default::default(),
                anidb_unavailable: false,
            },
        );
        ui.apply_snapshot(UiSnapshot {
            view: std::sync::Arc::new(state.view()),
            ..UiSnapshot::default()
        });
        // Its search request goes nowhere: the reply is staged.
        let _ = ui.test_update(Msg::LinkListEntry(id));
    }

    /// The loop's result and every action it sent.
    struct Run {
        exit: UiExit,
        sent: Vec<UserAction>,
    }

    /// Run the real loop headless until `case.exit`.
    fn run(case: Case) -> Run {
        let settings = Settings {
            username: Some("kim".into()),
            password: Some("test".into()),
            ..Settings::default()
        };
        let mut ui = Ui::with_setup(UserId::new("kim"), settings.clone(), vec![], false);
        ui.houseguest_ledger = match case.record {
            Record::Never => None,
            Record::Stored => Some(Ok(stored())),
            Record::Unreadable => Some(Err(())),
        };
        ui.houseguest_moved_out = case.moved_out;
        if case.layout_stored {
            let mut layout = LayoutSettings::default();
            layout.activate(&LayoutBundle::builtin().unwrap(), settings.pane_layout);
            ui.layout_settings = layout;
        }
        let (input_tx, input_rx) = super::super::delivery::channel();
        let (action_tx, mut action_rx) = mpsc::channel(if case.parked { 1 } else { 64 });
        if case.parked {
            action_tx
                .try_send(UserAction::FetchChatImage {
                    url: "filler".into(),
                })
                .unwrap();
        }
        let mut fail_at = usize::MAX;
        match case.exit {
            Exit::Shutdown => input_tx.send(UiInput::Shutdown).unwrap(),
            Exit::Quit => input_tx.send(key('c', KeyModifiers::CONTROL)).unwrap(),
            Exit::InputsClosed => {}
            Exit::ActionsClosed => action_rx.close(),
            Exit::SearchFails => {
                open_link_search(&mut ui);
                action_rx.close();
                input_tx
                    .send(UiInput::SearchResults {
                        query: "Some Obscure Show".into(),
                        results: vec![],
                    })
                    .unwrap();
            }
            Exit::DrawFails => {
                assert_eq!(
                    ui.test_update(Msg::Confirm {
                        prompt: "Osaka moves out?".into(),
                        then: Box::new(Msg::HouseguestMovedOut),
                    }),
                    None
                );
                input_tx.send(key('y', KeyModifiers::NONE)).unwrap();
                // The first draw is before the loop; the second closes
                // the `y` turn.
                fail_at = 2;
            }
        }
        // Kept alive otherwise, so a case that misses its exit hangs.
        let input_tx = (case.exit != Exit::InputsClosed).then_some(input_tx);
        let mut adapter = FlakyAdapter::new(fail_at);
        let exit = run_ui_loop(ui, input_rx, action_tx, &mut adapter);
        drop(input_tx);
        let mut sent = Vec::new();
        while let Ok(action) = action_rx.try_recv() {
            sent.push(action);
        }
        Run { exit, sent }
    }

    fn ledger_handouts(sent: &[UserAction]) -> Vec<&Ledger> {
        sent.iter()
            .filter_map(|action| match action {
                UserAction::SaveHouseguest(ledger) => Some(ledger),
                _ => None,
            })
            .collect()
    }

    fn layout_handouts(sent: &[UserAction]) -> Vec<&LayoutSettings> {
        sent.iter()
            .filter_map(|action| match action {
                UserAction::SaveLayoutSettings(layout) => Some(layout),
                _ => None,
            })
            .collect()
    }

    /// Run `case` and check what came back: her ledger exactly when she
    /// moved out (a wiped record: no visits), the pane sizes exactly
    /// when none were stored, and each equal to the last handout (if
    /// any got onto the queue).
    fn check(case: Case) -> Run {
        let label = format!("{case:?}");
        let run = run(case);
        let moved_out = case.moved_out || case.exit == Exit::DrawFails;
        match &run.exit.houseguest {
            Some(ledger) => {
                assert!(moved_out, "{label}: saved an unchanged ledger");
                assert_eq!(visits(ledger), 0, "{label}: a wiped record");
                if let Some(last) = ledger_handouts(&run.sent).last() {
                    assert_eq!(*last, ledger, "{label}: the last handout is the final one");
                }
            }
            None => assert!(!moved_out, "{label}: her move-out is lost"),
        }
        match &run.exit.layout {
            Some(layout) => {
                assert!(!case.layout_stored, "{label}: saved unchanged sizes");
                if let Some(last) = layout_handouts(&run.sent).last() {
                    assert_eq!(*last, layout, "{label}: the last handout is the final one");
                }
            }
            None => assert!(case.layout_stored, "{label}: the filled-in sizes are lost"),
        }
        if case.parked {
            // Nothing got past the filler: only the exit save carries
            // what changed.
            assert_eq!(run.sent.len(), 1, "{label}: only the filler was queued");
        }
        if case.exit == Exit::Quit {
            assert_eq!(run.sent.last(), Some(&UserAction::Quit), "{label}");
        }
        run
    }

    #[test]
    fn every_exit_returns_what_changed() {
        let mut cases = Vec::new();
        for record in [Record::Stored, Record::Unreadable] {
            for exit in [
                Exit::Shutdown,
                Exit::Quit,
                Exit::InputsClosed,
                Exit::ActionsClosed,
            ] {
                // Handed out and queued (or refused, when closed): a
                // queued handout the session never reads is lost with
                // its queue, so the exit save carries it all the same.
                for layout_stored in [true, false] {
                    cases.push(Case {
                        moved_out: true,
                        layout_stored,
                        ..Case::new(record, exit)
                    });
                }
            }
            for exit in [Exit::Shutdown, Exit::InputsClosed] {
                cases.push(Case {
                    moved_out: true,
                    parked: true,
                    layout_stored: false,
                    ..Case::new(record, exit)
                });
            }
            cases.push(Case::new(record, Exit::DrawFails));
        }
        for case in cases {
            check(case);
        }
    }

    /// A move-out confirmed in the turn whose draw ends the loop never
    /// reaches the loop's top again: only the exit save has it.
    #[test]
    fn a_move_out_in_the_last_turn_is_saved() {
        let run = check(Case::new(Record::Stored, Exit::DrawFails));
        assert!(run.exit.houseguest.is_some());
        assert!(ledger_handouts(&run.sent).is_empty(), "never handed out");
    }

    /// The pane sizes alone changed, and the closed queue refuses their
    /// handout: they come back at exit, and her unchanged ledger doesn't.
    #[test]
    fn unsent_pane_sizes_come_back_at_exit() {
        let run = check(Case {
            layout_stored: false,
            ..Case::new(Record::Stored, Exit::ActionsClosed)
        });
        assert!(run.exit.layout.is_some());
        assert!(run.exit.houseguest.is_none());
    }

    #[test]
    fn nothing_unchanged_is_saved_at_exit() {
        for record in [Record::Never, Record::Stored, Record::Unreadable] {
            for exit in [
                Exit::Shutdown,
                Exit::Quit,
                Exit::InputsClosed,
                Exit::SearchFails,
            ] {
                let run = check(Case::new(record, exit));
                assert!(ledger_handouts(&run.sent).is_empty(), "{record:?} {exit:?}");
            }
            // A closed queue needs a handout to fail: the sizes', which
            // the first draw filled in.
            check(Case {
                layout_stored: false,
                ..Case::new(record, Exit::ActionsClosed)
            });
        }
    }
}
