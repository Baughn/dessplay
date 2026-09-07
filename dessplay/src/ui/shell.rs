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
    /// Nyaa single-file browse results for the open modal.
    NyaaResults {
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
/// terminal to its normal state on exit.
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
    inputs: std::sync::mpsc::Receiver<UiInput>,
    actions: mpsc::Sender<UserAction>,
    on_terminal_ready: impl FnOnce(),
) {
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
            return;
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
        return;
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
    // screen is entered, per ratatui-image's docs. The query is worth
    // running even though we default to half-blocks: it reports the real
    // font cell size, which gives inline images the correct aspect ratio.
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
    run_ui_loop(ui, inputs, actions, &mut adapter);
    let _ = crossterm::execute!(std::io::stdout(), crossterm::event::DisableBracketedPaste);
    let _ = adapter.restore();
    tracing::debug!("UI thread exiting");
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
const TERMINAL_STATE_PROLOGUE: &str = "\x1b[?69l\x1b[r\x1b[?6l";

/// Pick the image-rendering protocol for inline chat images.
///
/// Defaults to **half-blocks** even on graphics-capable terminals: they
/// render as ordinary `▀` cells with foreground/background colors, so
/// they compose with the cell grid, the diff, and the color pass in
/// every terminal. The graphics protocols (Kitty, sixel, iTerm2) embed
/// cursor-moving escapes inside cell symbols; those are correct as
/// emitted, but they depend on terminal state we now establish in
/// [`TERMINAL_STATE_PROLOGUE`], and only Kitty is actually implemented
/// by Ghostty (the others show a blank block there).
///
/// The stdio query still runs, because it reports the terminal's real
/// font cell size — half-blocks rendered at the true aspect ratio look
/// far better than the assumed 10×20 of a bare `halfblocks()` picker.
///
/// `DESSPLAY_IMAGE_PROTOCOL` overrides the choice: `auto` uses the
/// detected protocol as-is (real Kitty/sixel/iTerm2 graphics);
/// `halfblocks`, `kitty`, `sixel`, or `iterm2` force one.
fn select_image_picker() -> ratatui_image::picker::Picker {
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
        // Detected protocol, untouched — the escape hatch for testing
        // real graphics on a terminal that supports them.
        Some("auto") => {}
        Some("kitty") => picker.set_protocol_type(ProtocolType::Kitty),
        Some("sixel") => picker.set_protocol_type(ProtocolType::Sixel),
        Some("iterm2") => picker.set_protocol_type(ProtocolType::Iterm2),
        // Default (unset, "halfblocks", or anything unrecognized): the
        // robust path, keeping the queried font size for aspect ratio.
        _ => picker.set_protocol_type(ProtocolType::Halfblocks),
    }
    picker
}

/// The terminal-agnostic UI loop body: apply inputs and redraw until the
/// input channel closes (or a quit action arrives). Generic over the
/// [`TerminalAdapter`] so the production path drives a real crossterm
/// terminal while tests drive a headless `TestTerminalAdapter` and run
/// the *real* draw/refresh work. The caller owns terminal
/// setup/teardown (raw mode, alternate screen, `restore`).
pub fn run_ui_loop<A: TerminalAdapter>(
    mut ui: Ui,
    inputs: std::sync::mpsc::Receiver<UiInput>,
    actions: mpsc::Sender<UserAction>,
    adapter: &mut A,
) {
    // Deliberately NO Terminal::clear() here (or anywhere while the
    // input thread lives): ratatui's clear() queries the cursor
    // position, and crossterm answers that by reading the terminal's
    // reply from stdin — which our input thread's event::read() starves,
    // so the query burns its full 2-second timeout. The alternate
    // screen is already blank and the first fullscreen draw paints
    // every cell.
    let _ = adapter.raw_mut().draw(|frame| ui.draw(frame));
    loop {
        // Adaptive cadence: ~100ms while a marquee pass animates, the
        // lazy 1s otherwise. Idle cost is unchanged — the timeout arm
        // only repaints when advance_clock reports a visible change.
        let input = match inputs.recv_timeout(ui.next_tick_hint()) {
            Ok(input) => input,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                let mut redraw = ui.advance_clock(now_millis());
                redraw |= dispatch_due_recovery(&mut ui, &actions);
                dispatch_image_fetches(&mut ui, &actions);
                if redraw && adapter.raw_mut().draw(|frame| ui.draw(frame)).is_err() {
                    break;
                }
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        };
        // Freshen the animator clock to the dequeue moment before any
        // handler runs: animation starts (marquee passes, spoiler
        // teases) and TTL stamps read `Ui::clock`, which would
        // otherwise be up to one lazy tick stale — and snapshots
        // advance running animations this way too (~10Hz during
        // playback). The redraw hint is moot; every input is followed
        // by a draw below anyway.
        let _ = ui.advance_clock(now_millis());
        match input {
            UiInput::Roguelike(result) => ui.set_roguelike(result),
            UiInput::Shutdown => break,
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
            UiInput::ChatImage { url, result } => ui.set_chat_image(&url, result.map(|img| *img)),
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
                        return;
                    }
                }
            }
            UiInput::NyaaResults { query, result } => ui.set_nyaa_results(&query, result),
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
                for action in ui.handle(event) {
                    let quit = action == UserAction::Quit;
                    if actions.blocking_send(action).is_err() || quit {
                        tracing::debug!("UI thread exiting (quit or actions channel closed)");
                        return;
                    }
                }
            }
        }
        dispatch_due_recovery(&mut ui, &actions);
        dispatch_image_fetches(&mut ui, &actions);
        if adapter.raw_mut().draw(|frame| ui.draw(frame)).is_err() {
            break;
        }
    }
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
pub fn run_input_thread(inputs: std::sync::mpsc::SyncSender<UiInput>) {
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
        // Mouse capture reports every motion, but only left-button
        // events (click / selection drag / release) and wheel ticks do
        // anything — and each forwarded event costs a full redraw on
        // the UI thread, so drop the rest here.
        if let Event::Mouse(mouse) = &event
            && !matches!(
                mouse.kind,
                MouseEventKind::Down(MouseButton::Left)
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
