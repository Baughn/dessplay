//! Input resolution: maps crossterm `KeyEvent` to `Action` via the `ViewSpec`.
//!
//! Pure function: no side effects, no state mutation.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use dessplay_core::view_spec::*;

/// Resolve a crossterm key event against the current `ViewSpec`.
///
/// Resolution order:
/// 1. Topmost modal's bindings
/// 2. Focused pane's bindings
/// 3. Global bindings (Ctrl-C always quits)
///
/// For character input in text contexts (chat, settings text fields, metadata
/// episode input), we synthesize `InsertChar`/`SettingsInsertChar`/`MetadataInsertChar`
/// actions when no explicit binding matches.
pub fn resolve_input(key: KeyEvent, spec: &ViewSpec) -> Option<Action> {
    // Ctrl-C always quits, regardless of context
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(Action::Quit);
    }

    // Check topmost modal first
    if let Some(modal) = spec.modals.last() {
        if let Some(action) = match_bindings(key, &modal.bindings) {
            return Some(action);
        }
        // Synthesize char input for modal text contexts
        if let Some(action) = synthesize_modal_char_input(key, &modal.content) {
            return Some(action);
        }
        // Modal captures input — don't fall through to panes
        return None;
    }

    // Check focused pane
    if let Some(pane) = find_focused_pane(&spec.base) {
        if let Some(action) = match_bindings(key, &pane.bindings) {
            return Some(action);
        }
        // Synthesize char input for text-input panes (chat)
        if let Some(action) = synthesize_pane_char_input(key, pane) {
            return Some(action);
        }
    }

    None
}

/// Try to match a key event against a list of bindings.
fn match_bindings(key: KeyEvent, bindings: &[Keybinding]) -> Option<Action> {
    for binding in bindings {
        if key_matches(key, &binding.key) {
            return Some(binding.action.clone());
        }
    }
    None
}

/// Check if a crossterm key event matches a ViewSpec key combo.
fn key_matches(key: KeyEvent, combo: &KeyCombo) -> bool {
    match combo {
        KeyCombo::Plain(k) => {
            !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
                && key_code_matches(key.code, k)
        }
        KeyCombo::Ctrl(k) => {
            key.modifiers.contains(KeyModifiers::CONTROL) && key_code_matches(key.code, k)
        }
        KeyCombo::Shift(k) => {
            key.modifiers.contains(KeyModifiers::SHIFT) && key_code_matches(key.code, k)
        }
    }
}

fn key_code_matches(code: KeyCode, key: &Key) -> bool {
    match (code, key) {
        (KeyCode::Char(a), Key::Char(b)) => a == *b,
        (KeyCode::Enter, Key::Enter) => true,
        (KeyCode::Esc, Key::Esc) => true,
        (KeyCode::Tab, Key::Tab) => true,
        (KeyCode::Backspace, Key::Backspace) => true,
        (KeyCode::Delete, Key::Delete) => true,
        (KeyCode::Up, Key::Up) => true,
        (KeyCode::Down, Key::Down) => true,
        (KeyCode::Left, Key::Left) => true,
        (KeyCode::Right, Key::Right) => true,
        (KeyCode::Home, Key::Home) => true,
        (KeyCode::End, Key::End) => true,
        _ => false,
    }
}

/// Find the focused pane in the layout tree.
fn find_focused_pane(node: &LayoutNode) -> Option<&PaneSpec> {
    match node {
        LayoutNode::Pane(pane) if pane.focused => Some(pane),
        LayoutNode::HSplit { left, right, .. } => {
            find_focused_pane(left).or_else(|| find_focused_pane(right))
        }
        LayoutNode::VSplit { top, bottom, .. } => {
            find_focused_pane(top).or_else(|| find_focused_pane(bottom))
        }
        _ => None,
    }
}

/// Synthesize character input actions for the focused pane.
///
/// The Chat pane accepts arbitrary text input when focused.
fn synthesize_pane_char_input(key: KeyEvent, pane: &PaneSpec) -> Option<Action> {
    if pane.id != PaneId::Chat {
        return None;
    }
    // Only plain characters (no Ctrl/Alt modifiers)
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    if let KeyCode::Char(c) = key.code {
        return Some(Action::InsertChar(c));
    }
    None
}

/// Synthesize character input for modal text contexts.
///
/// Settings text fields and metadata episode input accept typed characters.
fn synthesize_modal_char_input(key: KeyEvent, content: &ContentKind) -> Option<Action> {
    if key.modifiers.contains(KeyModifiers::CONTROL) || key.modifiers.contains(KeyModifiers::ALT) {
        return None;
    }
    let KeyCode::Char(c) = key.code else {
        return None;
    };

    // Determine which modal type we're in based on content structure
    // Settings modal has a Form content
    if is_settings_form_text_field(content) {
        return Some(Action::SettingsInsertChar(c));
    }

    // Metadata episode input has a Composite with TextInput
    if is_metadata_episode_input(content) {
        return Some(Action::MetadataInsertChar(c));
    }

    None
}

fn is_settings_form_text_field(content: &ContentKind) -> bool {
    match content {
        ContentKind::Form {
            fields,
            focused_field,
            ..
        } => fields.get(*focused_field).is_some_and(|f| {
            matches!(
                f.kind,
                FormFieldKind::Text { .. } | FormFieldKind::Masked { .. }
            )
        }),
        _ => false,
    }
}

fn is_metadata_episode_input(content: &ContentKind) -> bool {
    match content {
        ContentKind::Composite { children } => {
            children
                .iter()
                .any(|c| matches!(c, ContentKind::TextInput { .. }))
        }
        _ => false,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn make_chat_spec() -> ViewSpec {
        ViewSpec {
            base: LayoutNode::Pane(PaneSpec {
                id: PaneId::Chat,
                title: "Chat".to_string(),
                focused: true,
                content: ContentKind::TextLog {
                    lines: Vec::new(),
                    scroll_back: 0,
                },
                bindings: vec![
                    Keybinding {
                        key: KeyCombo::Plain(Key::Enter),
                        label: "Send",
                        action: Action::SendChat,
                        show_in_bar: true,
                    },
                    Keybinding {
                        key: KeyCombo::Plain(Key::Esc),
                        label: "Clear",
                        action: Action::ClearInput,
                        show_in_bar: true,
                    },
                ],
            }),
            modals: Vec::new(),
            status_bar: None,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn resolve_ctrl_c_always_quits() {
        let spec = make_chat_spec();
        assert_eq!(resolve_input(ctrl_key('c'), &spec), Some(Action::Quit));
    }

    #[test]
    fn resolve_enter_sends_chat() {
        let spec = make_chat_spec();
        assert_eq!(
            resolve_input(key(KeyCode::Enter), &spec),
            Some(Action::SendChat)
        );
    }

    #[test]
    fn resolve_char_input_synthesized() {
        let spec = make_chat_spec();
        assert_eq!(
            resolve_input(key(KeyCode::Char('h')), &spec),
            Some(Action::InsertChar('h'))
        );
    }

    #[test]
    fn resolve_modal_captures_input() {
        let mut spec = make_chat_spec();
        spec.modals.push(ModalSpec {
            title: " Test ".to_string(),
            width_pct: 0.5,
            height_pct: 0.5,
            content: ContentKind::TextLog {
                lines: Vec::new(),
                scroll_back: 0,
            },
            bindings: vec![Keybinding {
                key: KeyCombo::Plain(Key::Esc),
                label: "Close",
                action: Action::MetadataCancel,
                show_in_bar: true,
            }],
        });
        // Enter should NOT reach the chat pane — modal captures
        assert_eq!(resolve_input(key(KeyCode::Enter), &spec), None);
        // Esc should match the modal's binding
        assert_eq!(
            resolve_input(key(KeyCode::Esc), &spec),
            Some(Action::MetadataCancel)
        );
    }

    #[test]
    fn resolve_unmatched_key_returns_none() {
        let spec = make_chat_spec();
        assert_eq!(resolve_input(key(KeyCode::F(12)), &spec), None);
    }

    #[test]
    fn resolve_settings_char_input() {
        let spec = ViewSpec {
            base: LayoutNode::Pane(PaneSpec {
                id: PaneId::Chat,
                title: "Chat".to_string(),
                focused: false,
                content: ContentKind::Empty,
                bindings: Vec::new(),
            }),
            modals: vec![ModalSpec {
                title: " Settings ".to_string(),
                width_pct: 0.5,
                height_pct: 0.5,
                content: ContentKind::Form {
                    fields: vec![FormField {
                        label: "Username".to_string(),
                        kind: FormFieldKind::Text {
                            value: String::new(),
                        },
                        error: None,
                    }],
                    focused_field: 0,
                    alert: None,
                    hint: None,
                },
                bindings: vec![Keybinding {
                    key: KeyCombo::Plain(Key::Esc),
                    label: "Cancel",
                    action: Action::SettingsCancel,
                    show_in_bar: true,
                }],
            }],
            status_bar: None,
        };
        assert_eq!(
            resolve_input(key(KeyCode::Char('a')), &spec),
            Some(Action::SettingsInsertChar('a'))
        );
    }
}
