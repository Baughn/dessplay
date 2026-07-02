//! Key-event matchers shared by every pane and modal, and the one place
//! the terminal-compatibility policy lives:
//!
//! - **Bare letters over Ctrl-letters** for bindings: Ctrl-modified
//!   letters collide with control codes (Ctrl-J == LF, Ctrl-M == Enter,
//!   Ctrl-S == XOFF) in terminals lacking the enhanced keyboard
//!   protocol, so panes bind `J`/`K`/`S` etc. as typed characters.
//! - **Ctrl and Alt both accepted** for word motion/deletion: desktop
//!   terminals send Ctrl for Ctrl-arrow; macOS terminals (ghostty) send
//!   Alt for Option-arrow and are unreliable about Ctrl-arrow.
//! - **`.contains`, not `==`,** when matching Ctrl/Alt: the kitty
//!   keyboard protocol can set extra modifier bits alongside them.

use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};

/// Match helper: a plain (modifier-less) key press.
pub(crate) fn plain(ev: &Event<NoUserEvent>) -> Option<Key> {
    match ev {
        Event::Keyboard(KeyEvent { code, modifiers }) if *modifiers == KeyModifiers::NONE => {
            Some(*code)
        }
        _ => None,
    }
}

/// Match helper: a Ctrl-modified key press.
pub(crate) fn ctrl(ev: &Event<NoUserEvent>) -> Option<Key> {
    match ev {
        Event::Keyboard(KeyEvent { code, modifiers }) if *modifiers == KeyModifiers::CONTROL => {
            Some(*code)
        }
        _ => None,
    }
}

/// Match helper for word navigation/deletion: a key carrying *either* the
/// Ctrl or the Alt modifier (see the module docs for why both, and why
/// `.contains`). The modifiers are returned so callers can keep a binding
/// Ctrl-only where Alt would collide (e.g. `w`).
pub(crate) fn word_mod(ev: &Event<NoUserEvent>) -> Option<(Key, KeyModifiers)> {
    match ev {
        Event::Keyboard(KeyEvent { code, modifiers })
            if modifiers.contains(KeyModifiers::CONTROL)
                || modifiers.contains(KeyModifiers::ALT) =>
        {
            Some((*code, *modifiers))
        }
        _ => None,
    }
}

/// Is this a typed character (with or without shift)?
pub(crate) fn typed(ev: &Event<NoUserEvent>) -> Option<char> {
    match ev {
        Event::Keyboard(KeyEvent {
            code: Key::Char(c),
            modifiers,
        }) if *modifiers == KeyModifiers::NONE || *modifiers == KeyModifiers::SHIFT => Some(*c),
        _ => None,
    }
}
