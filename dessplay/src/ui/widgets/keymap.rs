//! Declarative key bindings: one table per component (or per mode) from
//! which *both* the event dispatch and the keybinding bar derive. A key
//! shown in the bar therefore always dispatches, and a dispatched key is
//! either advertised or deliberately hidden (`bar: None`) — the bar
//! cannot drift from behavior, which hand-maintained `keybindings()`
//! lists next to hand-written `match` arms could.
//!
//! What stays *outside* the tables is structural, not per-key: list
//! navigation ([`super::ListCursor`]) and text editing
//! ([`super::LineBuffer::edit`]) are shared vocabularies advertised by
//! the component that embeds them.
//!
//! An action returns `Option<Out>`: `None` means "declined — not
//! applicable right now", and the event falls through to the next layer
//! (e.g. a guarded Esc declines so the shared editor can see the key).

use tuirealm::event::{Event, Key, NoUserEvent};

use super::keys::{plain, typed};

/// A keybinding-bar entry: (key text, action label).
pub type BarEntry = (&'static str, &'static str);

/// How a binding matches an event.
pub enum KeyPattern {
    /// A plain (modifier-less) key.
    Plain(Key),
    /// One typed character (with or without shift) — the bare-letter
    /// binding style (see [`super::keys`] for why not Ctrl-letters).
    Char(char),
    /// Any of several typed characters sharing one action (`j` | `J`).
    Chars(&'static [char]),
}

impl KeyPattern {
    fn matches(&self, ev: &Event<NoUserEvent>) -> bool {
        match self {
            KeyPattern::Plain(key) => plain(ev) == Some(*key),
            KeyPattern::Char(c) => typed(ev) == Some(*c),
            KeyPattern::Chars(cs) => typed(ev).is_some_and(|t| cs.contains(&t)),
        }
    }
}

/// One binding: a pattern, its (optional) bar entry, and the action —
/// a method on the component, returning `None` to decline.
pub struct Binding<C: 'static, Out: 'static> {
    /// What triggers it.
    pub pattern: KeyPattern,
    /// The keybinding-bar entry; `None` = bound but not advertised
    /// (e.g. the second key of a shown "J/K" pair).
    pub bar: Option<BarEntry>,
    /// The action to run.
    pub action: fn(&mut C) -> Option<Out>,
}

/// A component's binding table (usually a `static`, one per mode).
pub struct Keymap<C: 'static, Out: 'static>(pub &'static [Binding<C, Out>]);

impl<C, Out> Keymap<C, Out> {
    /// Run the first matching, non-declining binding.
    pub fn dispatch(&self, component: &mut C, ev: &Event<NoUserEvent>) -> Option<Out> {
        for binding in self.0 {
            if binding.pattern.matches(ev)
                && let Some(out) = (binding.action)(component)
            {
                return Some(out);
            }
        }
        None
    }

    /// The advertised bar entries, in declaration order.
    pub fn bar(&self) -> Vec<BarEntry> {
        self.0.iter().filter_map(|binding| binding.bar).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuirealm::event::{KeyEvent, KeyModifiers};

    struct Counter {
        count: usize,
        armed: bool,
    }

    impl Counter {
        fn bump(&mut self) -> Option<usize> {
            self.count += 1;
            Some(self.count)
        }

        fn guarded(&mut self) -> Option<usize> {
            self.armed.then_some(99)
        }
    }

    static MAP: Keymap<Counter, usize> = Keymap(&[
        Binding {
            pattern: KeyPattern::Plain(Key::Enter),
            bar: Some(("Enter", "Bump")),
            action: Counter::bump,
        },
        Binding {
            pattern: KeyPattern::Chars(&['g', 'G']),
            bar: None,
            action: Counter::guarded,
        },
    ]);

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    #[test]
    fn dispatch_runs_matching_action_and_bar_lists_advertised() {
        let mut c = Counter {
            count: 0,
            armed: false,
        };
        assert_eq!(MAP.dispatch(&mut c, &key(Key::Enter)), Some(1));
        assert_eq!(MAP.dispatch(&mut c, &key(Key::Esc)), None);
        // The bar shows exactly the advertised entries, in order.
        assert_eq!(MAP.bar(), vec![("Enter", "Bump")]);
    }

    #[test]
    fn declined_actions_fall_through() {
        let mut c = Counter {
            count: 0,
            armed: false,
        };
        // Guard not armed: the binding matches but declines.
        assert_eq!(MAP.dispatch(&mut c, &key(Key::Char('g'))), None);
        c.armed = true;
        assert_eq!(MAP.dispatch(&mut c, &key(Key::Char('G'))), Some(99));
    }
}
