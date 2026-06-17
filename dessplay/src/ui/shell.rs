//! The production shell around [`super::app::Ui`]: a dedicated UI
//! thread owning the real terminal, an input thread reading crossterm
//! events, and the async bridge that feeds snapshots in and carries
//! [`UserAction`]s out. Tests bypass all of this and drive `Ui`
//! directly — that's the point of the synchronous dispatcher.

use tokio::sync::mpsc;
use tuirealm::event::{Event, NoUserEvent};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use super::app::{Ui, UiSnapshot};
use super::msg::UserAction;

/// Everything the UI thread consumes.
pub enum UiInput {
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
    /// AniDB name-search results (delivered to the search modal).
    SearchResults {
        /// The query these results answer.
        query: String,
        /// The hits.
        results: Vec<dessplay_core::net::AniDbSearchHit>,
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
pub fn run_ui_thread(
    ui: Ui,
    inputs: std::sync::mpsc::Receiver<UiInput>,
    actions: mpsc::Sender<UserAction>,
) {
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
    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        "terminal setup complete"
    );
    run_ui_loop(ui, inputs, actions, &mut adapter);
    let _ = adapter.restore();
    tracing::debug!("UI thread exiting");
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
    while let Ok(input) = inputs.recv() {
        match input {
            UiInput::Shutdown => break,
            UiInput::Snapshot(snapshot) => ui.apply_snapshot(*snapshot),
            UiInput::Subtitle {
                text,
                video_millis,
                arrival_millis,
            } => ui.push_subtitle(video_millis, arrival_millis, text),
            UiInput::Hashing {
                filename,
                done_bytes,
                total_bytes,
                finished,
            } => ui.set_hash_progress(filename, done_bytes, total_bytes, finished),
            UiInput::System { timestamp, text } => ui.push_system(timestamp, text),
            UiInput::SearchResults { query, results } => ui.set_search_results(&query, results),
            UiInput::Probe(cell) => {
                // Stamp the moment we dequeued this input — measured by a
                // test against the send time. Fall through to a draw so
                // the probe pays the same per-input cost real input does.
                *cell.lock().unwrap() = Some(std::time::Instant::now());
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
        if adapter.raw_mut().draw(|frame| ui.draw(frame)).is_err() {
            break;
        }
    }
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

/// Read crossterm events on the current (dedicated) thread, forwarding
/// them as [`UiInput::Event`]s until the channel closes.
pub fn run_input_thread(inputs: std::sync::mpsc::SyncSender<UiInput>) {
    tracing::debug!("input thread started");
    loop {
        match crossterm::event::read() {
            Ok(event) => {
                let event: Event<NoUserEvent> = event.into();
                if matches!(event, Event::None) {
                    continue;
                }
                tracing::trace!(kind = event_kind(&event), "input event forwarded");
                if inputs.send(UiInput::Event(event)).is_err() {
                    tracing::debug!("input thread exiting (channel closed)");
                    return;
                }
            }
            Err(e) => {
                tracing::error!("terminal input died: {e}");
                return;
            }
        }
    }
}
