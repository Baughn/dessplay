//! The one field-editing form. Settings and the List-entry editor are
//! declarations over this widget: a model provides typed rows and one
//! semantic edit boundary; the form owns cursor movement, control
//! activation, the pop-up text editor, validation display, the save paths,
//! and Esc-to-cancel. Display order is never a field's identity.

use tuirealm::event::{Event, Key, NoUserEvent};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::Line;
use tuirealm::ratatui::widgets::{Clear, Paragraph};

use super::keys::{ctrl, plain, typed};
use super::line::TextField;
use super::list::ListCursor;
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

/// A semantic category tab; the template supplies its brackets and spacing.
pub struct FormTab {
    /// Stable category identity.
    pub key: String,
    /// Unpadded category caption.
    pub label: String,
    /// The active category.
    pub selected: bool,
    /// The category contains a missing required value.
    pub missing: bool,
}

/// A semantic form note with a stable identity.
pub struct FormNote {
    /// Stable note identity.
    pub key: String,
    /// Literal note contents.
    pub text: String,
    /// Semantic appearance, before authored declarations.
    pub style: Style,
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
    type RowId: Clone + Eq + std::fmt::Debug;
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

    /// Category tabs above the scrollable rows.
    fn tabs(&self) -> Vec<FormTab> {
        Vec::new()
    }

    /// Fixed notes below the rows and above Save (the public-IRC warning).
    fn notes(&self) -> Vec<FormNote> {
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
        if let Ok(bundle) = crate::ui::layout::LayoutBundle::builtin() {
            self.render_layout(frame, area, &mut crate::ui::layout::Renderer::new(bundle));
        }
    }

    /// Render using the UI thread's active template renderer, preserving editors.
    pub fn render_layout(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        renderer: &mut crate::ui::layout::Renderer,
    ) {
        self.reconcile_selection();
        let (px, py) = self.model.overlay_percent();
        let modal = overlay(area, px, py);
        frame.render_widget(Clear, modal);

        let tabs = self.model.tabs();
        let notes = self.model.notes();
        // The fixed-footer priority remains a measured-content policy.
        let available = modal.height.saturating_sub(2);
        let header_height = u16::from(!tabs.is_empty()).min(available.saturating_sub(1));
        let notes_height =
            (notes.len() as u16).min(available.saturating_sub(header_height).saturating_sub(1));
        let tab_items: Vec<_> = tabs
            .into_iter()
            .map(|tab| {
                let mut style = if tab.selected {
                    theme::highlight_style()
                } else {
                    Style::default()
                };
                if tab.missing {
                    style = style.patch(theme::tone_style(crate::ui::props::Tone::Blocked));
                }
                crate::ui::layout::PresentedItem {
                    key: tab.key,
                    data: crate::ui::layout::Presentation::default()
                        .text("open", "[")
                        .text("label", tab.label)
                        .text("missing", "!")
                        .text("close", "]")
                        .boolean("invalid", tab.missing)
                        .selected(tab.selected)
                        .component_style("form-tab", style),
                }
            })
            .collect();
        let note_items: Vec<_> = notes
            .into_iter()
            .map(|note| crate::ui::layout::PresentedItem {
                key: note.key,
                data: crate::ui::layout::Presentation::default()
                    .text("body", note.text)
                    .style("body", note.style),
            })
            .collect();
        let data = crate::ui::layout::Presentation::default()
            .text("title", self.model.title())
            .list("tabs", tab_items.clone())
            .list("note-items", note_items.clone())
            .slot("header", 0, header_height)
            .slot("body", 0, 0)
            .slot("notes", 0, notes_height)
            .slot("save", 0, available.min(1))
            .slot("editor", 0, 1)
            .slot("error", 0, 1)
            .boolean("editing", self.editor.is_some())
            .boolean(
                "invalid",
                self.editor.as_ref().is_some_and(|e| e.error.is_some()),
            );
        let scene = match renderer.arrange("form", modal, &data) {
            Ok(scene) => scene,
            Err(error) => {
                tracing::error!(%error, "form layout failed");
                return;
            }
        };
        scene.paint(frame);
        let header_area = scene.slot("header");
        let body_area = scene.slot("body");
        let notes_area = scene.slot("notes");
        let save_area = scene.slot("save");

        if header_height > 0 {
            let data = crate::ui::layout::Presentation::default()
                .list("tabs", tab_items)
                .inherit(scene.style("header"), 0);
            if let Ok(header) = renderer.arrange("form-tabs", header_area, &data) {
                header.paint_with_slots(frame, |_, _, _, _| {});
            }
        }

        let rows = self.model.rows();
        if !body_area.is_empty() {
            let items = rows
                .iter()
                .map(|row| {
                    let mut data = crate::ui::layout::Presentation::default()
                        .text("label", row.label)
                        .text("value", row.control.display())
                        .text(
                            "annotation",
                            row.annotation
                                .as_ref()
                                .map(|(text, _)| text.clone())
                                .unwrap_or_default(),
                        )
                        .style("label", row.style)
                        .style("value", row.style)
                        .style(
                            "annotation",
                            row.annotation
                                .as_ref()
                                .map(|(_, style)| *style)
                                .unwrap_or_default(),
                        )
                        .boolean(
                            "labelled",
                            !matches!(row.control, FormControl::Action { .. }),
                        )
                        .boolean("annotated", row.annotation.is_some());
                    if row.preserve_value_end {
                        data = data.preserve_end("value");
                    }
                    crate::ui::layout::PresentedRow {
                        key: format!("{:?}", row.id),
                        data,
                        gap_after: row.gap_after,
                    }
                })
                .collect::<Vec<_>>();
            if let Err(error) = renderer.paint_rows(
                frame,
                body_area,
                "form-row",
                &items,
                Some(self.cursor.index()),
                scene.style("body"),
            ) {
                tracing::error!(%error, "form row layout failed");
            }
        }

        if notes_height > 0 {
            let data = crate::ui::layout::Presentation::default()
                .list("notes", note_items)
                .inherit(scene.style("notes"), 0);
            if let Ok(notes) = renderer.arrange("form-notes", notes_area, &data) {
                notes.paint_with_slots(frame, |_, _, _, _| {});
            }
        }

        let save_line = match self.model.save_hint() {
            None => Line::raw("[Save]"),
            Some(hint) => Line::styled(format!("[Save] — needs {hint}"), theme::dim()),
        };
        let save = Paragraph::new(save_line).style(scene.style("save").patch(
            if self.cursor.index() == rows.len() {
                theme::highlight_style()
            } else {
                Style::default()
            },
        ));
        frame.render_widget(save, save_area);

        scene.paint_overlays(frame);
        if let Some(editor) = &mut self.editor {
            editor
                .input
                .render_content(frame, scene.slot("editor"), true, editor.masked);
            frame
                .buffer_mut()
                .set_style(scene.slot("editor"), scene.style("editor"));
            if let Some(error) = &editor.error {
                frame.render_widget(
                    Paragraph::new(Line::styled(
                        error.clone(),
                        theme::tone_style(crate::ui::props::Tone::Blocked)
                            .patch(scene.style("error")),
                    )),
                    scene.slot("error"),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tuirealm::event::{KeyEvent, KeyModifiers};
    use tuirealm::ratatui::Terminal;
    use tuirealm::ratatui::backend::TestBackend;
    use tuirealm::ratatui::layout::Rect;

    #[derive(Clone, Debug, PartialEq, Eq)]
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
    fn bracketed_paste_lands_in_the_active_editor() {
        // Regression (2026-07-26): pasting into a settings text field
        // (cmd-v = bracketed paste) was silently dropped; only typed
        // keystrokes reached the editor.
        let mut form = form();
        assert!(matches!(form.on(&key(Key::Enter)), FormEvent::Handled));
        assert!(matches!(
            form.on(&Event::Paste("hunter2\n".into())),
            FormEvent::Handled
        ));
        assert_eq!(
            form.editor.as_ref().map(|e| e.input.text()),
            Some("hunter2".into())
        );
        assert!(matches!(form.on(&key(Key::Enter)), FormEvent::Handled));
        assert_eq!(form.model.text, "hunter2");
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
        let rendered = row.control.display();
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

    struct LongModel;

    impl FormModel for LongModel {
        type RowId = usize;
        type Out = ();

        fn title(&self) -> String {
            "Long form".into()
        }

        fn rows(&self) -> Vec<FormRow<Self::RowId>> {
            (0..20)
                .map(|i| {
                    let row = FormRow::read_only(i, "Row", format!("value-{i:02}"));
                    if i == 4 { row.with_gap_after() } else { row }
                })
                .collect()
        }

        fn apply(
            &mut self,
            _id: &Self::RowId,
            _edit: FormEdit,
        ) -> Result<FormEffect<Self::Out>, FormError> {
            Ok(FormEffect::Handled)
        }

        fn save(&self) -> Self::Out {}

        fn overlay_percent(&self) -> (u16, u16) {
            (100, 100)
        }
    }

    #[test]
    fn centers_long_list_inside_form_body() {
        let mut form = Form::new(LongModel);
        assert!(form.select_row(&10));

        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let buffer = terminal
            .draw(|frame| form.render(frame, frame.area()))
            .unwrap()
            .buffer
            .clone();
        let y = (0..buffer.area.height).find(|&y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                .contains("value-10")
        });

        // Border leaves eight rows and fixed Save leaves seven for the body.
        // The blank display row after field 4 must count when centering.
        assert_eq!(y, Some(4));
    }
}
