//! The one field-editing form. Settings and the List-entry editor are
//! declarations over this widget: a model provides typed rows and one
//! semantic edit boundary; the form owns cursor movement, control
//! activation, the pop-up text editor, validation display, the save paths,
//! and Esc-to-cancel. Display order is never a field's identity.

use tuirealm::event::{Event, Key, NoUserEvent};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use unicode_width::UnicodeWidthStr;

use super::keys::{ctrl, plain, typed};
use super::line::TextField;
use super::list::ListCursor;
use super::table::{Align, Cell, table_row, truncate_display_start};
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

/// A standard form control. The form derives Enter's behavior from this
/// value, so a model cannot render a toggle while accidentally opening a text
/// editor for the same row.
#[derive(Clone, PartialEq, Eq)]
pub enum FormControl {
    /// Plain one-line text.
    Text {
        /// Current text.
        value: String,
    },
    /// Masked one-line text, both in the row and the pop-up editor.
    Secret {
        /// Current unmasked value (never rendered directly).
        value: String,
    },
    /// A yes/no value. Enter sends [`FormEdit::SetBool`] with the inverse.
    Toggle {
        /// Current boolean value.
        value: bool,
    },
    /// A finite choice. Enter sends [`FormEdit::Cycle`].
    Choice {
        /// Current choice label.
        value: String,
    },
    /// Selectable display data which only model-specific commands mutate.
    ReadOnly {
        /// Current display value.
        value: String,
    },
    /// A named action. Enter sends [`FormEdit::Activate`].
    Action {
        /// Bracketed action label.
        label: String,
    },
}

impl FormControl {
    fn display(&self) -> String {
        match self {
            FormControl::Text { value }
            | FormControl::Choice { value }
            | FormControl::ReadOnly { value } => value.clone(),
            FormControl::Secret { value } => "*".repeat(value.chars().count()),
            FormControl::Toggle { value } => if *value { "yes" } else { "no" }.into(),
            FormControl::Action { label } => format!("[{label}]"),
        }
    }
}

/// A typed row projected from a form model.
pub struct FormRow<Id> {
    /// Stable semantic identity, independent of display order.
    pub id: Id,
    /// Field label. Action controls render their own bracketed label instead.
    pub label: &'static str,
    /// The control and its current display value.
    pub control: FormControl,
    /// Style for the label and value (e.g. dormant IRC controls are dim).
    pub style: Style,
    /// Optional right-aligned lifecycle or scope annotation.
    pub annotation: Option<(String, Style)>,
    preserve_value_end: bool,
    gap_after: bool,
}

impl<Id> FormRow<Id> {
    /// Plain text control.
    pub fn text(id: Id, label: &'static str, value: impl Into<String>) -> Self {
        Self::new(
            id,
            label,
            FormControl::Text {
                value: value.into(),
            },
        )
    }

    /// Masked text control.
    pub fn secret(id: Id, label: &'static str, value: impl Into<String>) -> Self {
        Self::new(
            id,
            label,
            FormControl::Secret {
                value: value.into(),
            },
        )
    }

    /// Boolean toggle.
    pub fn toggle(id: Id, label: &'static str, value: bool) -> Self {
        Self::new(id, label, FormControl::Toggle { value })
    }

    /// Cycled finite choice.
    pub fn choice(id: Id, label: &'static str, value: impl Into<String>) -> Self {
        Self::new(
            id,
            label,
            FormControl::Choice {
                value: value.into(),
            },
        )
    }

    /// Selectable read-only value (usually with model-specific commands).
    pub fn read_only(id: Id, label: &'static str, value: impl Into<String>) -> Self {
        Self::new(
            id,
            label,
            FormControl::ReadOnly {
                value: value.into(),
            },
        )
    }

    /// Enter-triggered action.
    pub fn action(id: Id, label: impl Into<String>) -> Self {
        Self::new(
            id,
            "",
            FormControl::Action {
                label: label.into(),
            },
        )
    }

    fn new(id: Id, label: &'static str, control: FormControl) -> Self {
        Self {
            id,
            label,
            control,
            style: Style::default(),
            annotation: None,
            preserve_value_end: false,
            gap_after: false,
        }
    }

    /// Style the field's label and value.
    pub fn styled(mut self, style: Style) -> Self {
        self.style = style;
        self
    }

    /// Add a right-aligned annotation.
    pub fn annotated(mut self, text: impl Into<String>, style: Style) -> Self {
        self.annotation = Some((text.into(), style));
        self
    }

    /// Keep the end of an overlong value visible (used for media-root paths).
    pub fn preserving_value_end(mut self) -> Self {
        self.preserve_value_end = true;
        self
    }

    /// Add one non-selectable blank display line after this row.
    pub fn with_gap_after(mut self) -> Self {
        self.gap_after = true;
        self
    }

    fn line(&self, width: usize) -> Line<'static> {
        let annotation_width = self
            .annotation
            .as_ref()
            .map(|(text, _)| text.width().min((width / 2).max(1)))
            .unwrap_or(0);
        let reserved = if annotation_width == 0 {
            0
        } else {
            annotation_width + 1
        };
        let flex_width = width.saturating_sub(reserved).max(8);
        let display = self.control.display();
        let content = match &self.control {
            FormControl::Action { .. } => self.control.display(),
            _ if self.preserve_value_end => {
                let value_width = flex_width.saturating_sub(24);
                let (value, _) = truncate_display_start(&display, value_width);
                format!("{:<24}{value}", self.label)
            }
            _ => format!("{:<24}{display}", self.label),
        };
        let cells = self
            .annotation
            .as_ref()
            .map_or_else(Vec::new, |(text, style)| {
                vec![Cell::new(
                    text.clone(),
                    *style,
                    annotation_width,
                    Align::Right,
                )]
            });
        table_row(width, vec![Span::styled(content, self.style)], cells)
    }
}

/// A semantic edit emitted by the shared form interaction layer.
pub enum FormEdit {
    /// Commit a text editor.
    SetText(String),
    /// Set a toggle to this value.
    SetBool(bool),
    /// Advance a choice to its next value.
    Cycle,
    /// Activate an action row.
    Activate,
    /// A form-specific command typed on the selected row.
    Command(char),
}

/// Successful result of applying a semantic edit.
pub enum FormEffect<Out> {
    /// The model did not use this edit; let structural routing continue.
    Ignored,
    /// The model changed locally.
    Handled,
    /// The model produced an output for the application.
    Out(Out),
}

/// An edit which could not be applied.
pub enum FormError {
    /// The control and edit kinds disagree. This indicates a declaration bug.
    InvalidEdit,
    /// User-entered text failed domain validation.
    Validation(String),
}

impl FormError {
    fn message(self) -> String {
        match self {
            FormError::InvalidEdit => "this field cannot accept that edit".into(),
            FormError::Validation(message) => message,
        }
    }
}

/// A form's content and semantics. Everything behavioral lives in [`Form`];
/// the model projects owned rows and applies edits by semantic row identity.
pub trait FormModel {
    /// Stable row identity.
    type RowId: Clone + Eq;
    /// What a completed form emits (the app's message type).
    type Out;

    /// The modal title.
    fn title(&self) -> String;

    /// Rows in display order, excluding the fixed `[Save]` footer.
    fn rows(&self) -> Vec<FormRow<Self::RowId>>;

    /// Apply one semantic edit.
    fn apply(
        &mut self,
        id: &Self::RowId,
        edit: FormEdit,
    ) -> Result<FormEffect<Self::Out>, FormError>;

    /// Fixed lines above the scrollable rows (settings category tabs).
    fn header(&self) -> Vec<Line<'static>> {
        Vec::new()
    }

    /// Fixed notes below the rows and above Save (the public-IRC warning).
    fn notes(&self) -> Vec<Line<'static>> {
        Vec::new()
    }

    /// Keybinding-bar label for Enter.
    fn enter_label(&self) -> &'static str {
        "Edit"
    }

    /// Extra advertised keys matching [`FormEdit::Command`] handling.
    fn extra_bar(&self) -> Vec<super::keymap::BarEntry> {
        Vec::new()
    }

    /// Why the form cannot save right now (`None` = saveable).
    fn save_hint(&self) -> Option<String> {
        None
    }

    /// Output of a successful save.
    fn save(&self) -> Self::Out;

    /// Overlay size as (percent_x, percent_y).
    fn overlay_percent(&self) -> (u16, u16) {
        (70, 70)
    }
}

/// What one event did to the form, for the modal wrapper to map onto the
/// application's message type.
pub enum FormEvent<Out> {
    /// Consumed; re-render.
    Handled,
    /// The form produced an output (a save or row action).
    Out(Out),
    /// Esc outside the editor: close the modal.
    Cancelled,
    /// Not a form key.
    Ignored,
}

struct Editor<Id> {
    id: Id,
    input: TextField,
    masked: bool,
    error: Option<String>,
}

enum Selection<Id> {
    Row(Id),
    Save,
}

/// The form widget: a [`FormModel`] plus all editing behavior.
pub struct Form<M: FormModel> {
    /// The model (public: modal wrappers expose domain-specific accessors).
    pub model: M,
    cursor: ListCursor,
    selection: Selection<M::RowId>,
    editor: Option<Editor<M::RowId>>,
}

impl<M: FormModel> Form<M> {
    /// A form over `model`, cursor on the first row.
    pub fn new(model: M) -> Self {
        let selection = model
            .rows()
            .first()
            .map(|row| Selection::Row(row.id.clone()))
            .unwrap_or(Selection::Save);
        Self {
            model,
            cursor: ListCursor::default(),
            selection,
            editor: None,
        }
    }

    fn row_count(&self) -> usize {
        self.model.rows().len() + 1
    }

    /// Is a text/secret editor active?
    pub fn is_editing(&self) -> bool {
        self.editor.is_some()
    }

    /// Select a row by semantic identity. Returns false when it is absent.
    pub fn select_row(&mut self, id: &M::RowId) -> bool {
        let Some(index) = self.model.rows().iter().position(|row| &row.id == id) else {
            return false;
        };
        self.cursor.set(index);
        self.selection = Selection::Row(id.clone());
        true
    }

    /// Select the fixed Save footer.
    pub fn select_save(&mut self) {
        self.cursor.set(self.model.rows().len());
        self.selection = Selection::Save;
    }

    /// Currently selected semantic row (`None` means Save).
    pub fn selected_row(&self) -> Option<M::RowId> {
        match &self.selection {
            Selection::Row(id) if self.model.rows().iter().any(|row| &row.id == id) => {
                Some(id.clone())
            }
            Selection::Row(_) => self
                .model
                .rows()
                .get(self.cursor.index())
                .map(|row| row.id.clone()),
            Selection::Save => None,
        }
    }

    /// Is the fixed Save footer selected?
    pub fn save_selected(&self) -> bool {
        matches!(self.selection, Selection::Save)
    }

    /// The keybinding bar: Enter, model commands, then save/cancel.
    pub fn bar(&self) -> Vec<super::keymap::BarEntry> {
        let mut items = vec![("Enter", self.model.enter_label())];
        items.extend(self.model.extra_bar());
        items.push(("S", "Save"));
        items.push(("Esc", "Cancel"));
        items
    }

    fn try_save(&self) -> FormEvent<M::Out> {
        match self.model.save_hint() {
            None => FormEvent::Out(self.model.save()),
            Some(_) => FormEvent::Handled,
        }
    }

    fn restore_selection(&mut self, id: &M::RowId) {
        if !self.select_row(id) {
            self.cursor.clamp(self.row_count());
            self.selection_from_cursor();
        }
    }

    fn reconcile_selection(&mut self) {
        let rows = self.model.rows();
        match &self.selection {
            Selection::Row(id) => {
                if let Some(index) = rows.iter().position(|row| &row.id == id) {
                    self.cursor.set(index);
                } else {
                    self.cursor.clamp(rows.len() + 1);
                    self.selection = rows
                        .get(self.cursor.index())
                        .map(|row| Selection::Row(row.id.clone()))
                        .unwrap_or(Selection::Save);
                }
            }
            Selection::Save => self.cursor.set(rows.len()),
        }
    }

    fn selection_from_cursor(&mut self) {
        let rows = self.model.rows();
        self.selection = rows
            .get(self.cursor.index())
            .map(|row| Selection::Row(row.id.clone()))
            .unwrap_or(Selection::Save);
    }

    fn apply(&mut self, id: M::RowId, edit: FormEdit) -> Result<FormEvent<M::Out>, String> {
        let effect = self.model.apply(&id, edit).map_err(FormError::message)?;
        self.restore_selection(&id);
        Ok(match effect {
            FormEffect::Ignored => FormEvent::Ignored,
            FormEffect::Handled => FormEvent::Handled,
            FormEffect::Out(out) => FormEvent::Out(out),
        })
    }

    fn on_editor(&mut self, ev: &Event<NoUserEvent>) -> FormEvent<M::Out> {
        match plain(ev) {
            Some(Key::Enter) => {
                let Some(mut editor) = self.editor.take() else {
                    return FormEvent::Handled;
                };
                let id = editor.id.clone();
                let value = editor.input.text();
                match self.apply(id, FormEdit::SetText(value)) {
                    Ok(FormEvent::Ignored | FormEvent::Cancelled) => {
                        editor.error = Some("this field cannot accept text".into());
                        self.editor = Some(editor);
                        FormEvent::Handled
                    }
                    Ok(event) => event,
                    Err(message) => {
                        editor.error = Some(message);
                        self.editor = Some(editor);
                        FormEvent::Handled
                    }
                }
            }
            Some(Key::Esc) => {
                self.editor = None;
                FormEvent::Handled
            }
            _ => {
                if let Some(editor) = &mut self.editor {
                    editor.error = None;
                    editor.input.edit(ev);
                }
                FormEvent::Handled
            }
        }
    }

    /// Route one event.
    pub fn on(&mut self, ev: &Event<NoUserEvent>) -> FormEvent<M::Out> {
        self.reconcile_selection();
        if self.editor.is_some() {
            return self.on_editor(ev);
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
            if let Some(id) = self.selected_row() {
                return match self.apply(id, FormEdit::Command(c)) {
                    Ok(FormEvent::Ignored) => FormEvent::Ignored,
                    Ok(event) => event,
                    Err(_) => FormEvent::Handled,
                };
            }
        }

        let Some(key) = plain(ev) else {
            return FormEvent::Ignored;
        };
        if self.cursor.nav(key, self.row_count()) {
            self.selection_from_cursor();
            return FormEvent::Handled;
        }
        match key {
            Key::Enter => {
                let rows = self.model.rows();
                let Some(row) = rows.get(self.cursor.index()) else {
                    return self.try_save();
                };
                let id = row.id.clone();
                match &row.control {
                    FormControl::Text { value } => {
                        self.editor = Some(Editor {
                            id,
                            input: TextField::with_text(value),
                            masked: false,
                            error: None,
                        });
                        FormEvent::Handled
                    }
                    FormControl::Secret { value } => {
                        self.editor = Some(Editor {
                            id,
                            input: TextField::with_text(value),
                            masked: true,
                            error: None,
                        });
                        FormEvent::Handled
                    }
                    FormControl::Toggle { value } => self
                        .apply(id, FormEdit::SetBool(!value))
                        .unwrap_or(FormEvent::Handled),
                    FormControl::Choice { .. } => self
                        .apply(id, FormEdit::Cycle)
                        .unwrap_or(FormEvent::Handled),
                    FormControl::Action { .. } => self
                        .apply(id, FormEdit::Activate)
                        .unwrap_or(FormEvent::Handled),
                    FormControl::ReadOnly { .. } => FormEvent::Handled,
                }
            }
            Key::Esc => FormEvent::Cancelled,
            _ => FormEvent::Ignored,
        }
    }

    /// Render a centered modal with fixed header, notes, and Save footer
    /// around a scrollable list of controls.
    pub fn render(&mut self, frame: &mut Frame, area: Rect) {
        self.reconcile_selection();
        let (px, py) = self.model.overlay_percent();
        let modal = overlay(area, px, py);
        frame.render_widget(Clear, modal);

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme::border_style(true))
            .title(self.model.title());
        let inner = block.inner(modal);
        frame.render_widget(block, modal);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let header = self.model.header();
        let notes = self.model.notes();
        let header_height = (header.len() as u16).min(inner.height.saturating_sub(1));
        let notes_height =
            (notes.len() as u16).min(inner.height.saturating_sub(header_height).saturating_sub(1));
        let body_height = inner
            .height
            .saturating_sub(header_height)
            .saturating_sub(notes_height)
            .saturating_sub(1);

        let header_area = Rect::new(inner.x, inner.y, inner.width, header_height);
        let body_area = Rect::new(inner.x, inner.y + header_height, inner.width, body_height);
        let notes_area = Rect::new(
            inner.x,
            body_area.y + body_area.height,
            inner.width,
            notes_height,
        );
        let save_area = Rect::new(inner.x, notes_area.y + notes_area.height, inner.width, 1);

        if header_height > 0 {
            frame.render_widget(Paragraph::new(header), header_area);
        }

        let rows = self.model.rows();
        if body_height > 0 {
            let mut items = Vec::with_capacity(rows.len());
            let mut selected_item = None;
            for (index, row) in rows.iter().enumerate() {
                if self.cursor.index() == index {
                    selected_item = Some(items.len());
                }
                items.push(ListItem::new(row.line(inner.width as usize)));
                if row.gap_after {
                    items.push(ListItem::new(Line::raw("")));
                }
            }
            let mut state = ListState::default();
            state.select(selected_item);
            frame.render_stateful_widget(
                List::new(items).highlight_style(theme::highlight_style()),
                body_area,
                &mut state,
            );
        }

        if notes_height > 0 {
            frame.render_widget(Paragraph::new(notes), notes_area);
        }

        let save_line = match self.model.save_hint() {
            None => Line::raw("[Save]"),
            Some(hint) => Line::styled(format!("[Save] — needs {hint}"), theme::dim()),
        };
        let save = Paragraph::new(save_line).style(if self.cursor.index() == rows.len() {
            theme::highlight_style()
        } else {
            Style::default()
        });
        frame.render_widget(save, save_area);

        if let Some(editor) = &mut self.editor {
            let edit_height = if editor.error.is_some() { 4 } else { 3 };
            let edit_area = Rect {
                x: modal.x + 2,
                y: modal
                    .y
                    .saturating_add(modal.height.saturating_sub(edit_height + 1)),
                width: modal.width.saturating_sub(4),
                height: edit_height,
            };
            frame.render_widget(Clear, edit_area);
            let field_area = Rect::new(edit_area.x, edit_area.y, edit_area.width, 3);
            editor.input.render(frame, field_area, true, editor.masked);
            if let Some(error) = &editor.error {
                let error_area = Rect::new(edit_area.x, edit_area.y + 3, edit_area.width, 1);
                frame.render_widget(
                    Paragraph::new(Line::styled(
                        error.clone(),
                        theme::tone_style(crate::ui::props::Tone::Blocked),
                    )),
                    error_area,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuirealm::event::{KeyEvent, KeyModifiers};
    use tuirealm::ratatui::layout::Rect;

    #[derive(Clone, PartialEq, Eq)]
    enum Field {
        Text,
        Flag,
    }

    fn key(code: Key) -> Event<NoUserEvent> {
        Event::Keyboard(KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
        })
    }

    struct TestModel {
        text: String,
        flag: bool,
    }

    impl FormModel for TestModel {
        type RowId = Field;
        type Out = (String, bool);

        fn title(&self) -> String {
            "Test".into()
        }

        fn rows(&self) -> Vec<FormRow<Field>> {
            vec![
                FormRow::text(Field::Text, "Text", self.text.clone()),
                FormRow::toggle(Field::Flag, "Flag", self.flag),
            ]
        }

        fn apply(
            &mut self,
            id: &Field,
            edit: FormEdit,
        ) -> Result<FormEffect<Self::Out>, FormError> {
            match (id, edit) {
                (Field::Text, FormEdit::SetText(value)) if value.trim().is_empty() => {
                    Err(FormError::Validation("text is required".into()))
                }
                (Field::Text, FormEdit::SetText(value)) => {
                    self.text = value.trim().into();
                    Ok(FormEffect::Handled)
                }
                (Field::Flag, FormEdit::SetBool(value)) => {
                    self.flag = value;
                    Ok(FormEffect::Handled)
                }
                (_, FormEdit::Command(_)) => Ok(FormEffect::Ignored),
                _ => Err(FormError::InvalidEdit),
            }
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
    fn edit_commit_roundtrip() {
        let mut form = form();
        assert!(matches!(form.on(&key(Key::Enter)), FormEvent::Handled));
        for c in "hi".chars() {
            form.on(&key(Key::Char(c)));
        }
        assert!(matches!(form.on(&key(Key::Enter)), FormEvent::Handled));
        assert_eq!(form.model.text, "hi");
    }

    #[test]
    fn every_successful_save_path_emits() {
        let capital_s = Event::Keyboard(KeyEvent {
            code: Key::Char('S'),
            modifiers: KeyModifiers::SHIFT,
        });
        let ctrl_s = Event::Keyboard(KeyEvent {
            code: Key::Char('s'),
            modifiers: KeyModifiers::CONTROL,
        });

        let valid = || {
            Form::new(TestModel {
                text: "hi".into(),
                flag: false,
            })
        };
        assert!(matches!(valid().on(&capital_s), FormEvent::Out((text, false)) if text == "hi"));
        assert!(matches!(valid().on(&ctrl_s), FormEvent::Out((text, false)) if text == "hi"));
        let mut save_row = valid();
        save_row.select_save();
        assert!(
            matches!(save_row.on(&key(Key::Enter)), FormEvent::Out((text, false)) if text == "hi")
        );
    }

    #[test]
    fn invalid_commit_keeps_editor_and_value_until_corrected() {
        let mut form = Form::new(TestModel {
            text: "old".into(),
            flag: false,
        });
        form.on(&key(Key::Enter));
        for _ in 0..3 {
            form.on(&key(Key::Backspace));
        }
        assert!(matches!(form.on(&key(Key::Enter)), FormEvent::Handled));
        assert!(form.is_editing());
        assert_eq!(form.model.text, "old");
        assert_eq!(
            form.editor
                .as_ref()
                .and_then(|editor| editor.error.as_deref()),
            Some("text is required")
        );
        form.on(&key(Key::Char('n')));
        form.on(&key(Key::Enter));
        assert!(!form.is_editing());
        assert_eq!(form.model.text, "n");
    }

    #[test]
    fn blocked_save_is_swallowed_not_emitted() {
        let mut form = form();
        let capital_s = Event::Keyboard(KeyEvent {
            code: Key::Char('S'),
            modifiers: KeyModifiers::SHIFT,
        });
        let ctrl_s = Event::Keyboard(KeyEvent {
            code: Key::Char('s'),
            modifiers: KeyModifiers::CONTROL,
        });
        assert!(matches!(form.on(&capital_s), FormEvent::Handled));
        assert!(matches!(form.on(&ctrl_s), FormEvent::Handled));
        form.select_save();
        assert!(matches!(form.on(&key(Key::Enter)), FormEvent::Handled));
    }

    #[test]
    fn editor_esc_discards_and_form_esc_cancels() {
        let mut form = Form::new(TestModel {
            text: "ok".into(),
            flag: false,
        });
        form.on(&key(Key::Enter));
        form.on(&key(Key::Char('x')));
        assert!(matches!(form.on(&key(Key::Esc)), FormEvent::Handled));
        assert_eq!(form.model.text, "ok");
        assert!(matches!(form.on(&key(Key::Esc)), FormEvent::Cancelled));
    }

    #[test]
    fn enter_on_toggle_row_flips_it() {
        let mut form = Form::new(TestModel {
            text: "ok".into(),
            flag: false,
        });
        assert!(form.select_row(&Field::Flag));
        assert!(matches!(form.on(&key(Key::Enter)), FormEvent::Handled));
        assert!(form.model.flag);
    }

    #[test]
    fn secret_row_masks_its_value() {
        let row = FormRow::secret(Field::Text, "Password", "hunter2");
        let rendered: String = row
            .line(50)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(!rendered.contains("hunter2"), "{rendered:?}");
        assert!(rendered.contains("*******"), "{rendered:?}");
    }

    #[test]
    fn overlay_does_not_overflow_on_a_very_wide_terminal() {
        let area = Rect::new(0, 0, 2000, 2000);
        let rect = overlay(area, 70, 70);
        assert_eq!(rect.width, 1400);
        assert_eq!(rect.height, 1400);
        assert!(rect.width <= area.width && rect.height <= area.height);
        assert_eq!(rect.x, (area.width - rect.width) / 2);
        assert_eq!(rect.y, (area.height - rect.height) / 2);
    }
}
