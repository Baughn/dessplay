//! Measured line navigation for inspection entries taller than their viewport.
use super::{Diagnostic, PresentedRow, RenderedScene, Renderer};
use tuirealm::ratatui::{Frame, layout::Rect, style::Style};

/// One navigable physical line, retaining its semantic item and source anchor.
pub(crate) struct MeasuredLine {
    pub key: String,
    pub source: Option<(String, usize)>,
    #[cfg(test)]
    pub text: String,
}

/// Measured entries and their line-to-controller mapping, published together.
#[derive(Default)]
pub(crate) struct MeasuredLines {
    pub rows: Vec<MeasuredLine>,
    entries: Vec<RenderedScene>,
}

impl Renderer {
    /// Measure a bounded inspection list using the same fragments it will paint.
    pub(crate) fn measure_lines(
        &mut self,
        template: &str,
        rows: &[PresentedRow],
        width: u16,
        style: Style,
    ) -> Result<MeasuredLines, Diagnostic> {
        let mut document = MeasuredLines::default();
        for row in rows {
            let scene = self.measure_content(
                template,
                &format!("lines:{template}/{}", row.key),
                width,
                &row.data.clone().inherit(style, 0),
            )?;
            for line in 0..scene.height() {
                let source = scene
                    .text_regions
                    .iter()
                    .find(|region| region.bounds.y == line)
                    .map(|region| (region.binding.clone(), region.source.start));
                document.rows.push(MeasuredLine {
                    key: row.key.clone(),
                    source,
                    #[cfg(test)]
                    text: scene.line_text(line),
                });
            }
            document.entries.push(scene);
        }
        Ok(document)
    }
}

impl MeasuredLines {
    /// Center a visual cursor, clipping complete measured entries without rewrap.
    pub fn paint(&self, frame: &mut Frame, area: Rect, selected: usize) {
        if area.is_empty() {
            return;
        }
        let selected = selected.min(self.rows.len().saturating_sub(1));
        let top = selected
            .saturating_sub(usize::from(area.height) / 2)
            .min(self.rows.len().saturating_sub(usize::from(area.height)));
        let mut offset = -(top as i32);
        for scene in &self.entries {
            scene.paint_scrolled(frame, area, offset);
            let selected_row = selected as i32 - top as i32;
            if selected_row >= offset && selected_row < offset + i32::from(scene.height()) {
                let selected = Rect::new(
                    area.x,
                    area.y.saturating_add(selected_row as u16),
                    area.width,
                    1,
                )
                .intersection(area);
                frame.buffer_mut().set_style(
                    selected,
                    crate::ui::theme::with_authored_style(
                        crate::ui::theme::highlight_style(),
                        scene.root_style(),
                    ),
                );
            }
            offset += i32::from(scene.height());
            if offset >= i32::from(area.height) {
                break;
            }
        }
    }
}
