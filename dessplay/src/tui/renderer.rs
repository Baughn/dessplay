//! Ratatui renderer: walks a `ViewSpec` and produces terminal output.
//!
//! Stateless between frames — all information comes from the `ViewSpec`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};

use dessplay_core::view_spec::*;

// =========================================================================
// Public entry point
// =========================================================================

/// Render a `ViewSpec` into a ratatui `Frame`.
pub fn render(spec: &ViewSpec, frame: &mut ratatui::Frame) {
    let area = frame.area();
    if area.width == 0 || area.height == 0 {
        return;
    }

    // Reserve 1 line at bottom for status bar
    let (main_area, bar_area) = if area.height > 1 {
        (
            Rect {
                height: area.height - 1,
                ..area
            },
            Rect {
                x: area.x,
                y: area.y + area.height - 1,
                width: area.width,
                height: 1,
            },
        )
    } else {
        (area, Rect::default())
    };

    // Render base layout
    render_layout(&spec.base, main_area, frame.buffer_mut());

    // Render modals (in stack order, last = topmost)
    for modal in &spec.modals {
        render_modal(modal, main_area, frame.buffer_mut());
    }

    // Render status bar
    if let Some(ref bar) = spec.status_bar {
        render_status_bar(bar, bar_area, frame.buffer_mut());
    }
}

// =========================================================================
// Layout rendering
// =========================================================================

fn render_layout(node: &LayoutNode, area: Rect, buf: &mut Buffer) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    match node {
        LayoutNode::HSplit { left, right, ratio } => {
            let left_width = ((area.width as f32) * ratio.clamp(0.0, 1.0)) as u16;
            let right_width = area.width.saturating_sub(left_width);
            let left_area = Rect {
                width: left_width,
                ..area
            };
            let right_area = Rect {
                x: area.x + left_width,
                width: right_width,
                ..area
            };
            render_layout(left, left_area, buf);
            render_layout(right, right_area, buf);
        }
        LayoutNode::VSplit { top, bottom, ratio } => {
            // Check if bottom is a Spacer with fixed height
            let bottom_height = match bottom.as_ref() {
                LayoutNode::Spacer { height, .. } => *height,
                // Check if top's bottom child is a Pane with id ChatInput (1 line)
                _ => 0,
            };

            // Check if top is a Pane with id ChatInput (fixed 1-line input)
            let top_is_input = matches!(
                top.as_ref(),
                LayoutNode::Pane(PaneSpec { id: PaneId::ChatInput, .. })
            );
            let bottom_is_input = matches!(
                bottom.as_ref(),
                LayoutNode::Pane(PaneSpec { id: PaneId::ChatInput, .. })
            );

            if bottom_height > 0 {
                // Fixed-height bottom (spacer)
                let bh = bottom_height.min(area.height);
                let top_height = area.height.saturating_sub(bh);
                let top_area = Rect {
                    height: top_height,
                    ..area
                };
                let bottom_area = Rect {
                    y: area.y + top_height,
                    height: bh,
                    ..area
                };
                render_layout(top, top_area, buf);
                render_layout(bottom, bottom_area, buf);
            } else if bottom_is_input {
                // Fixed 1-line bottom (chat input)
                let top_height = area.height.saturating_sub(1);
                let top_area = Rect {
                    height: top_height,
                    ..area
                };
                let bottom_area = Rect {
                    y: area.y + top_height,
                    height: 1,
                    ..area
                };
                render_layout(top, top_area, buf);
                render_layout(bottom, bottom_area, buf);
            } else if top_is_input {
                // Fixed 1-line top (shouldn't happen normally but handle it)
                let bottom_height = area.height.saturating_sub(1);
                let top_area = Rect { height: 1, ..area };
                let bottom_area = Rect {
                    y: area.y + 1,
                    height: bottom_height,
                    ..area
                };
                render_layout(top, top_area, buf);
                render_layout(bottom, bottom_area, buf);
            } else {
                // Proportional split
                let top_height = ((area.height as f32) * ratio.clamp(0.0, 1.0)) as u16;
                let bottom_height = area.height.saturating_sub(top_height);
                let top_area = Rect {
                    height: top_height,
                    ..area
                };
                let bottom_area = Rect {
                    y: area.y + top_height,
                    height: bottom_height,
                    ..area
                };
                render_layout(top, top_area, buf);
                render_layout(bottom, bottom_area, buf);
            }
        }
        LayoutNode::Pane(pane) => {
            render_pane(pane, area, buf);
        }
        LayoutNode::Spacer { content, .. } => {
            render_spacer_content(content, area, buf);
        }
    }
}

// =========================================================================
// Pane rendering
// =========================================================================

fn render_pane(pane: &PaneSpec, area: Rect, buf: &mut Buffer) {
    // Chat input is a special case — no border, just a prompt line
    if pane.id == PaneId::ChatInput {
        render_text_input_bare(
            &pane.content,
            area,
            buf,
            pane.focused,
        );
        return;
    }

    let border_style = if pane.focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if pane.title.is_empty() {
        String::new()
    } else {
        format!(" {} ", pane.title)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    render_content(&pane.content, inner, buf, pane.focused);
}

// =========================================================================
// Content rendering
// =========================================================================

fn render_content(content: &ContentKind, area: Rect, buf: &mut Buffer, focused: bool) {
    match content {
        ContentKind::TextLog { lines, scroll_back } => {
            render_text_log(lines, *scroll_back, area, buf);
        }
        ContentKind::SelectableList {
            items,
            selected,
            scroll_offset,
        } => {
            render_selectable_list(items, *selected, *scroll_offset, area, buf, focused);
        }
        ContentKind::TextInput {
            text,
            cursor_pos,
            placeholder,
        } => {
            render_text_input(text, *cursor_pos, placeholder, area, buf, true);
        }
        ContentKind::ProgressBar { fraction, label } => {
            render_progress_bar(*fraction, label, area, buf);
        }
        ContentKind::Composite { children } => {
            render_composite(children, area, buf, focused);
        }
        ContentKind::Form {
            fields,
            focused_field,
            alert,
            hint,
        } => {
            render_form(fields, *focused_field, alert.as_deref(), hint.as_deref(), area, buf);
        }
        ContentKind::Empty => {}
    }
}

fn render_text_log(
    lines: &[Vec<StyledSpan>],
    scroll_back: usize,
    area: Rect,
    buf: &mut Buffer,
) {
    let ratatui_lines: Vec<Line<'_>> = lines
        .iter()
        .map(|spans| {
            Line::from(
                spans
                    .iter()
                    .map(|s| styled_span_to_ratatui(s))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();

    // Scroll from bottom
    let total = ratatui_lines.len();
    let visible = area.height as usize;
    let max_scroll = total.saturating_sub(visible);
    let effective_scroll = scroll_back.min(max_scroll);
    let skip = total.saturating_sub(visible + effective_scroll);

    let paragraph = Paragraph::new(ratatui_lines)
        .wrap(Wrap { trim: false })
        .scroll((skip as u16, 0));
    paragraph.render(area, buf);
}

fn render_selectable_list(
    items: &[Vec<StyledSpan>],
    selected: usize,
    _scroll_offset: usize,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
) {
    let visible = area.height as usize;
    // Auto-scroll to keep selected visible
    let scroll_start = if selected >= visible {
        selected - visible + 1
    } else {
        0
    };

    for (i, item) in items.iter().enumerate().skip(scroll_start).take(visible) {
        let y = area.y + (i - scroll_start) as u16;
        let is_selected = i == selected && focused;
        let row_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: 1,
        };

        let mut spans: Vec<Span<'_>> = Vec::new();
        let prefix = if is_selected { "> " } else { "  " };
        spans.push(Span::raw(prefix));

        for s in item {
            let mut style = styled_span_to_style(s);
            if is_selected {
                style = style.add_modifier(Modifier::REVERSED);
            }
            spans.push(Span::styled(s.text.clone(), style));
        }

        let line = Line::from(spans);
        let paragraph = Paragraph::new(line);
        paragraph.render(row_area, buf);
    }
}

fn render_text_input(
    text: &str,
    cursor_pos: usize,
    _placeholder: &str,
    area: Rect,
    buf: &mut Buffer,
    show_cursor: bool,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let line = Line::from(Span::raw(text));
    let paragraph = Paragraph::new(line);
    paragraph.render(area, buf);

    if show_cursor {
        let cx = area.x.saturating_add(cursor_pos as u16);
        if cx < area.x.saturating_add(area.width) && buf.area.contains((cx, area.y).into()) {
            buf[(cx, area.y)]
                .set_style(Style::default().fg(Color::Black).bg(Color::White));
        }
    }
}

/// Render chat input as a bare prompt line (no border).
fn render_text_input_bare(
    content: &ContentKind,
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let (text, cursor_pos) = match content {
        ContentKind::TextInput {
            text, cursor_pos, ..
        } => (text.as_str(), *cursor_pos),
        _ => return,
    };

    let style = if focused {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let prompt = "> ";
    let display = format!("{prompt}{text}");
    let line = Line::from(Span::styled(display, style));
    let paragraph = Paragraph::new(line);
    paragraph.render(area, buf);

    if focused {
        let cx = area
            .x
            .saturating_add(prompt.len() as u16)
            .saturating_add(cursor_pos as u16);
        if cx < area.x.saturating_add(area.width)
            && buf.area.contains((cx, area.y).into())
        {
            buf[(cx, area.y)]
                .set_style(Style::default().fg(Color::Black).bg(Color::White));
        }
    }
}

fn render_progress_bar(fraction: f64, label: &str, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let label_len = label.len();
    let bar_width = (area.width as usize).saturating_sub(label_len + 3); // 2 for [ ] + 1 space
    let clamped = fraction.clamp(0.0, 1.0);
    let filled = (bar_width as f64 * clamped) as usize;
    let empty = bar_width.saturating_sub(filled);

    let bar = format!(
        "[{}{}] {label}",
        "=".repeat(filled),
        " ".repeat(empty),
    );

    let line = Line::from(Span::styled(bar, Style::default().fg(Color::Cyan)));
    let paragraph = Paragraph::new(line);
    paragraph.render(area, buf);
}

fn render_composite(
    children: &[ContentKind],
    area: Rect,
    buf: &mut Buffer,
    focused: bool,
) {
    let mut y = area.y;
    for child in children {
        if y >= area.y + area.height {
            break;
        }
        let child_height = content_height(child, area.width);
        let child_area = Rect {
            x: area.x,
            y,
            width: area.width,
            height: child_height.min(area.y + area.height - y),
        };
        render_content(child, child_area, buf, focused);
        y += child_height;
    }
}

fn render_form(
    fields: &[FormField],
    focused_field: usize,
    alert: Option<&str>,
    hint: Option<&[StyledSpan]>,
    area: Rect,
    buf: &mut Buffer,
) {
    let mut y = area.y;
    let w = area.width;

    // Alert banner
    if let Some(alert_text) = alert
        && y < area.y + area.height
    {
        let alert_area = Rect {
            x: area.x,
            y,
            width: w,
            height: 1,
        };
        let line = Line::from(Span::styled(
            alert_text,
            Style::default()
                .fg(Color::White)
                .bg(Color::Red)
                .add_modifier(Modifier::BOLD),
        ));
        Paragraph::new(line).render(alert_area, buf);
        y += 2; // alert + blank line
    }

    for (i, field) in fields.iter().enumerate() {
        if y >= area.y + area.height {
            break;
        }
        let is_focused = i == focused_field;

        // Label line
        let label_style = if is_focused {
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };

        let mut label_spans = vec![Span::styled(&field.label, label_style)];
        if let Some(ref err) = field.error {
            label_spans.push(Span::styled(
                format!("  ({err})"),
                Style::default().fg(Color::Red),
            ));
        }

        let label_area = Rect {
            x: area.x,
            y,
            width: w,
            height: 1,
        };
        Paragraph::new(Line::from(label_spans)).render(label_area, buf);
        y += 1;

        // Value line(s)
        match &field.kind {
            FormFieldKind::Text { value } => {
                if y < area.y + area.height {
                    let val_area = Rect {
                        x: area.x + 2,
                        y,
                        width: w.saturating_sub(2),
                        height: 1,
                    };
                    let style = if is_focused {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    Paragraph::new(Line::from(Span::styled(value, style))).render(val_area, buf);
                    if is_focused {
                        let cx = val_area.x + value.len() as u16;
                        if cx < val_area.x + val_area.width
                            && buf.area.contains((cx, val_area.y).into())
                        {
                            buf[(cx, val_area.y)].set_style(
                                Style::default().fg(Color::Black).bg(Color::White),
                            );
                        }
                    }
                    y += 1;
                }
            }
            FormFieldKind::Masked { value } => {
                if y < area.y + area.height {
                    let val_area = Rect {
                        x: area.x + 2,
                        y,
                        width: w.saturating_sub(2),
                        height: 1,
                    };
                    let masked = "*".repeat(value.len());
                    let style = if is_focused {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    };
                    Paragraph::new(Line::from(Span::styled(masked, style))).render(val_area, buf);
                    if is_focused {
                        let cx = val_area.x + value.len() as u16;
                        if cx < val_area.x + val_area.width
                            && buf.area.contains((cx, val_area.y).into())
                        {
                            buf[(cx, val_area.y)].set_style(
                                Style::default().fg(Color::Black).bg(Color::White),
                            );
                        }
                    }
                    y += 1;
                }
            }
            FormFieldKind::Toggle { value, .. } => {
                if y < area.y + area.height {
                    let val_area = Rect {
                        x: area.x + 2,
                        y,
                        width: w.saturating_sub(2),
                        height: 1,
                    };
                    let style = Style::default().fg(Color::Cyan);
                    Paragraph::new(Line::from(Span::styled(
                        format!("[{value}]"),
                        style,
                    )))
                    .render(val_area, buf);
                    y += 1;
                }
            }
            FormFieldKind::PathList { paths, selected } => {
                for (j, path_entry) in paths.iter().enumerate() {
                    if y >= area.y + area.height {
                        break;
                    }
                    let val_area = Rect {
                        x: area.x + 2,
                        y,
                        width: w.saturating_sub(2),
                        height: 1,
                    };
                    let is_sel = is_focused && j == *selected;
                    let color = if path_entry.is_download_target {
                        Color::Blue
                    } else {
                        Color::Gray
                    };
                    let mut style = Style::default().fg(color);
                    if is_sel {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    let prefix = if path_entry.is_download_target {
                        "[download] "
                    } else {
                        "           "
                    };
                    Paragraph::new(Line::from(Span::styled(
                        format!("{prefix}{}", path_entry.path),
                        style,
                    )))
                    .render(val_area, buf);
                    y += 1;
                }
            }
        }

        // Blank line between fields
        y += 1;
    }

    // Validation hint at the bottom
    if let Some(spans) = hint
        && y < area.y + area.height
    {
        let hint_area = Rect {
            x: area.x,
            y,
            width: w,
            height: 1,
        };
        let ratatui_spans: Vec<Span<'_>> =
            spans.iter().map(|s| styled_span_to_ratatui(s)).collect();
        Paragraph::new(Line::from(ratatui_spans)).render(hint_area, buf);
    }
}

// =========================================================================
// Spacer (player status)
// =========================================================================

fn render_spacer_content(content: &ContentKind, area: Rect, buf: &mut Buffer) {
    // Render with a border like player status
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    block.render(area, buf);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Add 2-space indent to composite children
    let indented = Rect {
        x: inner.x + 2,
        width: inner.width.saturating_sub(2),
        ..inner
    };
    render_content(content, indented, buf, false);
}

// =========================================================================
// Modal rendering
// =========================================================================

fn render_modal(modal: &ModalSpec, area: Rect, buf: &mut Buffer) {
    let modal_width = ((area.width as f32) * modal.width_pct) as u16;
    let modal_height = ((area.height as f32) * modal.height_pct) as u16;
    let modal_width = modal_width.max(10).min(area.width);
    let modal_height = modal_height.max(5).min(area.height);
    let x = area.x + (area.width.saturating_sub(modal_width)) / 2;
    let y = area.y + (area.height.saturating_sub(modal_height)) / 2;
    let modal_area = Rect::new(x, y, modal_width, modal_height);

    // Clear the area underneath
    Clear.render(modal_area, buf);

    let block = Block::default()
        .title(modal.title.as_str())
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(modal_area);
    block.render(modal_area, buf);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    render_content(&modal.content, inner, buf, true);
}

// =========================================================================
// Status bar rendering
// =========================================================================

fn render_status_bar(bar: &StatusBarSpec, area: Rect, buf: &mut Buffer) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let key_style = Style::default()
        .fg(Color::Black)
        .bg(Color::White)
        .add_modifier(Modifier::BOLD);
    let desc_style = Style::default().fg(Color::DarkGray);
    let sep_style = Style::default().fg(Color::DarkGray);

    let mut spans: Vec<Span<'_>> = Vec::new();
    for (i, (key, desc)) in bar.bindings.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" | ", sep_style));
        }
        spans.push(Span::styled(format!(" {key} "), key_style));
        spans.push(Span::styled(format!(" {desc}"), desc_style));
    }

    let line = Line::from(spans);
    Paragraph::new(line).render(area, buf);
}

// =========================================================================
// Style conversion
// =========================================================================

fn styled_span_to_ratatui(span: &StyledSpan) -> Span<'static> {
    Span::styled(span.text.clone(), styled_span_to_style(span))
}

fn styled_span_to_style(span: &StyledSpan) -> Style {
    let mut style = Style::default().fg(semantic_to_color(span.color));
    if span.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    style
}

/// Map semantic colors to ratatui terminal colors.
pub fn semantic_to_color(color: SemanticColor) -> Color {
    match color {
        SemanticColor::Default => Color::White,
        SemanticColor::Ready => Color::Green,
        SemanticColor::Paused => Color::Red,
        SemanticColor::NotWatching => Color::DarkGray,
        SemanticColor::Downloading => Color::Blue,
        SemanticColor::Missing => Color::Red,
        SemanticColor::Muted => Color::DarkGray,
        SemanticColor::Accent => Color::Blue,
        SemanticColor::Focused => Color::Cyan,
        SemanticColor::Error => Color::Red,
        SemanticColor::System => Color::Cyan,
        SemanticColor::Username => Color::Yellow,
    }
}

// =========================================================================
// Helpers
// =========================================================================

/// Estimate the height in lines for a content kind.
fn content_height(content: &ContentKind, _width: u16) -> u16 {
    match content {
        ContentKind::TextLog { lines, .. } => lines.len().max(1) as u16,
        ContentKind::SelectableList { items, .. } => items.len().max(1) as u16,
        ContentKind::TextInput { .. } => 1,
        ContentKind::ProgressBar { .. } => 1,
        ContentKind::Composite { children } => {
            children.iter().map(|c| content_height(c, _width)).sum()
        }
        ContentKind::Form { fields, .. } => (fields.len() * 3) as u16,
        ContentKind::Empty => 0,
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        Terminal::new(backend).unwrap()
    }

    fn minimal_spec() -> ViewSpec {
        ViewSpec {
            base: LayoutNode::Pane(PaneSpec {
                id: PaneId::Chat,
                title: "Test".to_string(),
                focused: true,
                content: ContentKind::TextLog {
                    lines: vec![vec![StyledSpan::plain("hello")]],
                    scroll_back: 0,
                },
                bindings: Vec::new(),
            }),
            modals: Vec::new(),
            status_bar: Some(StatusBarSpec {
                bindings: vec![("Ctrl-C".to_string(), "Quit")],
            }),
        }
    }

    #[test]
    fn render_does_not_panic_on_minimal_spec() {
        let mut term = test_terminal(80, 24);
        let spec = minimal_spec();
        term.draw(|frame| render(&spec, frame)).unwrap();
    }

    #[test]
    fn render_does_not_panic_on_tiny_terminal() {
        let mut term = test_terminal(5, 3);
        let spec = minimal_spec();
        term.draw(|frame| render(&spec, frame)).unwrap();
    }

    #[test]
    fn render_does_not_panic_with_modal() {
        let mut term = test_terminal(80, 24);
        let spec = ViewSpec {
            base: LayoutNode::Pane(PaneSpec {
                id: PaneId::Chat,
                title: "Chat".to_string(),
                focused: false,
                content: ContentKind::Empty,
                bindings: Vec::new(),
            }),
            modals: vec![ModalSpec {
                title: " Test Modal ".to_string(),
                width_pct: 0.5,
                height_pct: 0.5,
                content: ContentKind::TextLog {
                    lines: vec![vec![StyledSpan::plain("modal content")]],
                    scroll_back: 0,
                },
                bindings: Vec::new(),
            }],
            status_bar: None,
        };
        term.draw(|frame| render(&spec, frame)).unwrap();
    }

    #[test]
    fn render_hsplit_distributes_width() {
        let mut term = test_terminal(80, 24);
        let spec = ViewSpec {
            base: LayoutNode::HSplit {
                left: Box::new(LayoutNode::Pane(PaneSpec {
                    id: PaneId::Chat,
                    title: "Left".to_string(),
                    focused: true,
                    content: ContentKind::Empty,
                    bindings: Vec::new(),
                })),
                right: Box::new(LayoutNode::Pane(PaneSpec {
                    id: PaneId::Playlist,
                    title: "Right".to_string(),
                    focused: false,
                    content: ContentKind::Empty,
                    bindings: Vec::new(),
                })),
                ratio: 0.5,
            },
            modals: Vec::new(),
            status_bar: None,
        };
        term.draw(|frame| render(&spec, frame)).unwrap();
        // No panic = success; visual correctness is verified manually
    }

    #[test]
    fn semantic_color_mapping() {
        assert_eq!(semantic_to_color(SemanticColor::Ready), Color::Green);
        assert_eq!(semantic_to_color(SemanticColor::Paused), Color::Red);
        assert_eq!(semantic_to_color(SemanticColor::Muted), Color::DarkGray);
    }

    #[test]
    fn render_progress_bar_no_panic() {
        let mut term = test_terminal(40, 3);
        let spec = ViewSpec {
            base: LayoutNode::Pane(PaneSpec {
                id: PaneId::PlayerStatus,
                title: "Status".to_string(),
                focused: false,
                content: ContentKind::ProgressBar {
                    fraction: 0.5,
                    label: "50%".to_string(),
                },
                bindings: Vec::new(),
            }),
            modals: Vec::new(),
            status_bar: None,
        };
        term.draw(|frame| render(&spec, frame)).unwrap();
    }

    /// Regression test for fuzz crash: render_text_input accesses buffer
    /// out of bounds when a VSplit with ratio > 1.0 gives a pane a height
    /// far exceeding the terminal buffer, and a Composite child pushes y
    /// past the actual buffer boundary.
    /// Artifact: fuzz/artifacts/render_viewspec/crash-6c37c3c1142f8cc0c6d583e020ec440b609d5e12
    #[test]
    fn render_text_input_oob_from_bad_vsplit_ratio() {
        let mut term = test_terminal(20, 5);
        let spec = ViewSpec {
            base: LayoutNode::VSplit {
                top: Box::new(LayoutNode::Pane(PaneSpec {
                    id: PaneId::Chat,
                    title: "T".to_string(),
                    focused: false,
                    content: ContentKind::Composite {
                        children: vec![
                            // 6 lines pushes the TextInput to y >= 7 (after border),
                            // well past the 5-row buffer
                            ContentKind::TextLog {
                                lines: vec![vec![StyledSpan::plain("a")]; 6],
                                scroll_back: 0,
                            },
                            ContentKind::TextInput {
                                text: "x".to_string(),
                                cursor_pos: 0,
                                placeholder: String::new(),
                            },
                        ],
                    },
                    bindings: Vec::new(),
                })),
                bottom: Box::new(LayoutNode::Pane(PaneSpec {
                    id: PaneId::Playlist,
                    title: "B".to_string(),
                    focused: false,
                    content: ContentKind::Empty,
                    bindings: Vec::new(),
                })),
                ratio: 999.0,
            },
            modals: Vec::new(),
            status_bar: None,
        };
        // Should not panic — the cursor write must be bounds-checked
        term.draw(|frame| render(&spec, frame)).unwrap();
    }

    #[test]
    fn render_form_no_panic() {
        let mut term = test_terminal(60, 20);
        let spec = ViewSpec {
            base: LayoutNode::Pane(PaneSpec {
                id: PaneId::Chat,
                title: "Form".to_string(),
                focused: true,
                content: ContentKind::Form {
                    fields: vec![
                        FormField {
                            label: "Username".to_string(),
                            kind: FormFieldKind::Text {
                                value: "alice".to_string(),
                            },
                            error: None,
                        },
                        FormField {
                            label: "Password".to_string(),
                            kind: FormFieldKind::Masked {
                                value: "secret".to_string(),
                            },
                            error: None,
                        },
                    ],
                    focused_field: 0,
                    alert: None,
                    hint: Some(vec![StyledSpan::plain("Press Ctrl-S to save")]),
                },
                bindings: Vec::new(),
            }),
            modals: Vec::new(),
            status_bar: None,
        };
        term.draw(|frame| render(&spec, frame)).unwrap();
    }
}
