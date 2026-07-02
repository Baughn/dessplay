//! The one field-editing form. Settings and the List-entry editor are
//! *declarations* over this widget: a model says what the rows are and
//! what Enter means on each; the form owns everything behavioral —
//! cursor movement, the pop-up text editor, the save paths (capital `S`,
//! the `[Save]` row, and the unadvertised Ctrl-S alias — capital-S
//! exists because Ctrl-S is eaten as XOFF in terminals lacking the
//! enhanced keyboard protocol), and Esc-to-cancel. A new field, or a
//! whole new form, cannot behave differently from its neighbors.

use tuirealm::event::{Event, Key, NoUserEvent};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::widgets::{Clear, ListItem};

use super::keys::{ctrl, plain, typed};
use super::line::TextField;
use super::list::{ListCursor, render_list};
use crate::ui::theme;

/// The centered overlay area: `percent` of the frame, clamped.
pub fn overlay(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    // Widen to u32 for the multiply: `area.width * percent` overflows u16 on
    // a very wide terminal (panic in debug, garbage rect in release). The
    // result is clamped back below `area.width`, so the final `as u16` is
    // always in range.
    let width = (u32::from(area.width) * u32::from(percent_x) / 100)
        .max(20)
        .min(u32::from(area.width)) as u16;
    let height = (u32::from(area.height) * u32::from(percent_y) / 100)
        .max(8)
        .min(u32::from(area.height)) as u16;
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// What Enter does on a form row, decided by the model.
pub enum RowAction<Out> {
    /// Open the pop-up text editor prefilled with the row's value.
    Edit {
        /// The current value, loaded into the editor.
        current: String,
    },
    /// The model changed its own state (toggle, cycle); just re-render.
    Handled,
    /// The row produces an output for the app (e.g. open a picker).
    Out(Out),
}

/// What a typed letter did in the model (reorder, delete, ...).
pub enum CharOutcome {
    /// Not a model key; the form ignores it.
    Ignored,
    /// The model changed; keep the cursor where it is (re-clamped).
    Handled,
    /// The model changed and the cursor should follow to this row.
    MoveTo(usize),
}

/// A form's content and semantics. Everything behavioral lives in
/// [`Form`]; the model only answers "what are the rows" and "what does
/// this row do".
pub trait FormModel {
    /// What a completed form emits (the app's message type).
    type Out;

    /// The modal title.
    fn title(&self) -> String;

    /// The rows, in display order, excluding the trailing `[Save]` row
    /// (the form appends and handles that one).
    fn rows(&self) -> Vec<Line<'static>>;

    /// Enter on row `index`.
    fn activate(&mut self, index: usize) -> RowAction<Self::Out>;

    /// Commit the pop-up editor's text back to row `index`.
    fn commit(&mut self, index: usize, value: String);

    /// A typed letter on row `index` (model-specific extras like
    /// reorder/delete). Default: none.
    fn on_char(&mut self, _index: usize, _c: char) -> CharOutcome {
        CharOutcome::Ignored
    }

    /// Why the form cannot save right now (`None` = saveable). Drives
    /// both the save gate and the `[Save]` row's "needs …" hint, so a
    /// refused save always explains itself.
    fn save_hint(&self) -> Option<String> {
        None
    }

    /// The output of a successful save.
    fn save(&self) -> Self::Out;

    /// Overlay size as (percent_x, percent_y).
    fn overlay_percent(&self) -> (u16, u16) {
        (70, 70)
    }
}

/// What one event did to the form, for the modal wrapper to map onto
/// the app's message type.
pub enum FormEvent<Out> {
    /// Consumed; re-render.
    Handled,
    /// The form produced an output (a save, or a row's action).
    Out(Out),
    /// Esc outside the editor: close the modal.
    Cancelled,
    /// Not a form key.
    Ignored,
}

/// The form widget: a [`FormModel`] plus all editing behavior.
pub struct Form<M: FormModel> {
    /// The model (public: wrappers reach through for domain accessors).
    pub model: M,
    cursor: ListCursor,
    editor: Option<(usize, TextField)>,
}

impl<M: FormModel> Form<M> {
    /// A form over `model`, cursor on the first row.
    pub fn new(model: M) -> Self {
        Self {
            model,
            cursor: ListCursor::default(),
            editor: None,
        }
    }

    /// Rows plus the `[Save]` row.
    fn row_count(&self) -> usize {
        self.model.rows().len() + 1
    }

    /// Index of the `[Save]` row (last).
    pub fn save_index(&self) -> usize {
        self.model.rows().len()
    }

    /// Put the cursor on a row (tests).
    pub fn select(&mut self, index: usize) {
        self.cursor.set(index);
    }

    /// The row the cursor is on.
    pub fn selected(&self) -> usize {
        self.cursor.index()
    }

    fn try_save(&self) -> FormEvent<M::Out> {
        match self.model.save_hint() {
            None => FormEvent::Out(self.model.save()),
            Some(_) => FormEvent::Handled,
        }
    }

    /// Route one event.
    pub fn on(&mut self, ev: &Event<NoUserEvent>) -> FormEvent<M::Out> {
        // An active text editor swallows everything.
        if let Some((index, editor)) = &mut self.editor {
            match plain(ev) {
                Some(Key::Enter) => {
                    let index = *index;
                    let value = editor.text();
                    self.editor = None;
                    self.model.commit(index, value);
                }
                Some(Key::Esc) => {
                    self.editor = None;
                }
                _ => {
                    editor.edit(ev);
                }
            }
            return FormEvent::Handled;
        }
        // Ctrl-S is kept as an alias for terminals where it isn't eaten as
        // XOFF; capital `S` and the `[Save]` row are the reliable paths.
        if ctrl(ev) == Some(Key::Char('s')) {
            return self.try_save();
        }
        if let Some(c) = typed(ev) {
            if c == 'S' {
                return self.try_save();
            }
            match self.model.on_char(self.cursor.index(), c) {
                CharOutcome::Ignored => {}
                CharOutcome::Handled => {
                    self.cursor.clamp(self.row_count());
                    return FormEvent::Handled;
                }
                CharOutcome::MoveTo(index) => {
                    self.cursor.set(index);
                    self.cursor.clamp(self.row_count());
                    return FormEvent::Handled;
                }
            }
        }
        let Some(key) = plain(ev) else {
            return FormEvent::Ignored;
        };
        if self.cursor.nav(key, self.row_count()) {
            return FormEvent::Handled;
        }
        match key {
            Key::Enter => {
                let index = self.cursor.index();
                if index == self.save_index() {
                    return self.try_save();
                }
                match self.model.activate(index) {
                    RowAction::Edit { current } => {
                        self.editor = Some((index, TextField::with_text(&current)));
                        FormEvent::Handled
                    }
                    RowAction::Handled => FormEvent::Handled,
                    RowAction::Out(out) => FormEvent::Out(out),
                }
            }
            Key::Esc => FormEvent::Cancelled,
            _ => FormEvent::Ignored,
        }
    }

    /// Render as a centered modal: the rows, the `[Save]` row (with its
    /// "needs …" hint when blocked), and the pop-up editor when active.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        let (px, py) = self.model.overlay_percent();
        let modal = overlay(area, px, py);
        frame.render_widget(Clear, modal);
        let mut items: Vec<ListItem> =
            self.model.rows().into_iter().map(ListItem::new).collect();
        let save_line = match self.model.save_hint() {
            None => Line::raw("[Save]"),
            Some(hint) => Line::styled(format!("[Save] — needs {hint}"), theme::dim()),
        };
        items.push(ListItem::new(save_line));
        render_list(
            frame,
            modal,
            self.model.title(),
            items,
            Some(self.cursor.index()),
            true,
        );
        if let Some((_, editor)) = &mut self.editor {
            let edit_area = Rect {
                x: modal.x + 2,
                y: modal.y + modal.height.saturating_sub(4),
                width: modal.width.saturating_sub(4),
                height: 3,
            };
            frame.render_widget(Clear, edit_area);
            editor.render(frame, edit_area, true, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuirealm::event::{KeyEvent, KeyModifiers};
    use tuirealm::ratatui::layout::Rect;

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// A two-field model: one text field, one toggle; saveable only when
    /// the text is non-empty.
    struct TestModel {
        text: String,
        flag: bool,
    }

    impl FormModel for TestModel {
        type Out = (String, bool);

        fn title(&self) -> String {
            "Test".into()
        }

        fn rows(&self) -> Vec<Line<'static>> {
            vec![
                Line::raw(format!("Text: {}", self.text)),
                Line::raw(format!("Flag: {}", self.flag)),
            ]
        }

        fn activate(&mut self, index: usize) -> RowAction<Self::Out> {
            match index {
                0 => RowAction::Edit {
                    current: self.text.clone(),
                },
                _ => {
                    self.flag = !self.flag;
                    RowAction::Handled
                }
            }
        }

        fn commit(&mut self, _index: usize, value: String) {
            self.text = value.trim().to_string();
        }

        fn save_hint(&self) -> Option<String> {
            self.text.is_empty().then(|| "some text".to_string())
        }

        fn save(&self) -> Self::Out {
            (self.text.clone(), self.flag)
        }
    }

    fn form() -> Form<TestModel> {
        Form::new(TestModel {
            text: String::new(),
            flag: false,
        })
    }

    #[test]
    fn edit_commit_and_save_roundtrip() {
        let mut f = form();
        // Enter on the text row opens the editor; type; Enter commits.
        assert!(matches!(f.on(&key(Key::Enter)), FormEvent::Handled));
        for c in "hi".chars() {
            f.on(&key(Key::Char(c)));
        }
        assert!(matches!(f.on(&key(Key::Enter)), FormEvent::Handled));
        assert_eq!(f.model.text, "hi");
        // Capital S saves now that the gate is satisfied.
        let ev = Event::Keyboard(KeyEvent {
            code: Key::Char('S'),
            modifiers: KeyModifiers::SHIFT,
        });
        assert!(matches!(f.on(&ev), FormEvent::Out((text, false)) if text == "hi"));
    }

    #[test]
    fn blocked_save_is_swallowed_not_emitted() {
        let mut f = form(); // empty text: save gated
        let ctrl_s = Event::Keyboard(KeyEvent {
            code: Key::Char('s'),
            modifiers: KeyModifiers::CONTROL,
        });
        assert!(matches!(f.on(&ctrl_s), FormEvent::Handled));
        f.select(f.save_index());
        assert!(matches!(f.on(&key(Key::Enter)), FormEvent::Handled));
    }

    #[test]
    fn editor_esc_discards_and_form_esc_cancels() {
        let mut f = form();
        f.on(&key(Key::Enter)); // open editor
        f.on(&key(Key::Char('x')));
        assert!(matches!(f.on(&key(Key::Esc)), FormEvent::Handled));
        assert_eq!(f.model.text, ""); // discarded
        // Esc with no editor open cancels the whole form.
        assert!(matches!(f.on(&key(Key::Esc)), FormEvent::Cancelled));
    }

    #[test]
    fn enter_on_toggle_row_flips_it() {
        let mut f = form();
        f.on(&key(Key::Down));
        assert!(matches!(f.on(&key(Key::Enter)), FormEvent::Handled));
        assert!(f.model.flag);
    }

    #[test]
    fn overlay_does_not_overflow_on_a_very_wide_terminal() {
        // Regression: the percent multiply must be widened past u16 before
        // dividing — e.g. 2000 cols * 70 = 140000 > u16::MAX. The result
        // must stay clamped to the frame.
        let area = Rect::new(0, 0, 2000, 2000);
        let rect = overlay(area, 70, 70);
        assert_eq!(rect.width, 1400);
        assert_eq!(rect.height, 1400);
        assert!(rect.width <= area.width && rect.height <= area.height);
        assert_eq!(rect.x, (area.width - rect.width) / 2);
        assert_eq!(rect.y, (area.height - rect.height) / 2);
    }
}
