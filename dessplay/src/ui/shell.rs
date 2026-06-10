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
}

/// Run the UI on the current (dedicated) thread until the input
/// channel closes or the action receiver goes away. Returns the
/// terminal to its normal state on exit.
pub fn run_ui_thread(
    mut ui: Ui,
    inputs: std::sync::mpsc::Receiver<UiInput>,
    actions: mpsc::Sender<UserAction>,
) {
    let mut adapter = match CrosstermTerminalAdapter::new() {
        Ok(adapter) => adapter,
        Err(e) => {
            tracing::error!("cannot initialize the terminal: {e}");
            return;
        }
    };
    let _ = adapter.raw_mut().draw(|frame| ui.draw(frame));
    while let Ok(input) = inputs.recv() {
        match input {
            UiInput::Snapshot(snapshot) => ui.apply_snapshot(*snapshot),
            UiInput::Event(event) => {
                for action in ui.handle(event) {
                    let quit = action == UserAction::Quit;
                    if actions.blocking_send(action).is_err() || quit {
                        let _ = adapter.restore();
                        return;
                    }
                }
            }
        }
        if adapter.raw_mut().draw(|frame| ui.draw(frame)).is_err() {
            break;
        }
    }
    let _ = adapter.restore();
}

/// Read crossterm events on the current (dedicated) thread, forwarding
/// them as [`UiInput::Event`]s until the channel closes.
pub fn run_input_thread(inputs: std::sync::mpsc::SyncSender<UiInput>) {
    loop {
        match crossterm::event::read() {
            Ok(event) => {
                let event: Event<NoUserEvent> = event.into();
                if !matches!(event, Event::None) && inputs.send(UiInput::Event(event)).is_err() {
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
