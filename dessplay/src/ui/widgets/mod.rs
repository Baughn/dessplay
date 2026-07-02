//! Shared interaction primitives for the TUI.
//!
//! Every pane and modal builds on the same small vocabulary — a line
//! editor, a selection cursor, key-event matchers — implemented here
//! exactly once. This is deliberate architecture, not just tidiness:
//! "not all the text fields work the same way" was a real, recurring
//! bug class (word navigation and the horizontal-scroll reset each
//! landed in the chat input and silently missed the modal fields). A
//! behavior that exists in one place cannot drift.
//!
//! The widgets are pure state machines: events in, state out, with
//! rendering as a separate function over the state. Nothing in the
//! interaction logic touches ratatui except the `render` methods, which
//! keeps the door open for the future non-terminal UI
//! (ui-architecture.md, Web Renderer).

pub mod keys;
pub mod line;

pub(crate) use keys::{ctrl, plain, typed};
pub use line::{LineBuffer, TextField};
