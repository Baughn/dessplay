use std::collections::BTreeMap;

use super::{Diagnostic, Presentation, PresentedRow, RenderedScene, Renderer};
use tuirealm::ratatui::{Frame, layout::Rect, style::Style};

/// Controller-owned position in a document of keyed semantic items.
#[derive(Clone, Debug, Default)]
pub struct DocumentScroll {
    key: Option<String>,
    index: usize,
    offset: usize,
    source: Option<(String, usize, usize)>,
    pending: i64,
}

impl DocumentScroll {
    /// Open at the tail without measuring all preceding entries.
    pub fn to_end(&mut self) {
        *self = Self {
            index: usize::MAX,
            offset: usize::MAX,
            ..Self::default()
        };
    }

    /// Request movement by measured terminal rows; negative values move upward.
    pub fn advance(&mut self, rows: i64) {
        self.pending = self.pending.saturating_add(rows);
    }
}

#[cfg(test)]
#[test]
#[allow(clippy::unwrap_used)]
fn opening_a_long_document_at_the_tail_measures_only_visible_entries() {
    use tuirealm::ratatui::{Terminal, backend::TestBackend};
    let rows: Vec<_> = (0..5000)
        .map(|index| PresentedRow {
            key: index.to_string(),
            data: Presentation::default().text("body", format!("event {index}")),
            gap_after: false,
        })
        .collect();
    let mut renderer = Renderer::new(super::LayoutBundle::builtin().unwrap());
    let mut scroll = DocumentScroll::default();
    scroll.to_end();
    let mut terminal = Terminal::new(TestBackend::new(30, 5)).unwrap();
    terminal
        .draw(|frame| {
            renderer
                .paint_document(
                    frame,
                    frame.area(),
                    "rogue-recent-event",
                    &rows,
                    &mut scroll,
                    Style::default(),
                )
                .unwrap()
        })
        .unwrap();
    assert!(renderer.arrangement_count() <= 6);
    let last: String = (0..30)
        .map(|x| terminal.backend().buffer()[(x, 4)].symbol())
        .collect();
    assert!(last.contains("event 4999"), "{last}");
}

impl Renderer {
    /// Paint a virtualized document, retaining an item/source anchor through resize.
    #[allow(clippy::too_many_arguments)]
    pub fn paint_document(
        &mut self,
        frame: &mut Frame<'_>,
        area: Rect,
        template: &str,
        rows: &[PresentedRow],
        scroll: &mut DocumentScroll,
        style: Style,
    ) -> Result<(), Diagnostic> {
        if area.is_empty() {
            return Ok(());
        }
        if rows.is_empty() {
            *scroll = DocumentScroll::default();
            return Ok(());
        }
        let mut keys = std::collections::BTreeSet::new();
        if rows
            .iter()
            .any(|row| row.key.is_empty() || !keys.insert(&row.key))
        {
            return Err(super::renderer::internal(
                "document items require nonempty unique keys",
            ));
        }
        let mut measured = BTreeMap::<usize, RenderedScene>::new();
        let measure = |renderer: &mut Renderer,
                       index: usize,
                       measured: &mut BTreeMap<usize, RenderedScene>|
         -> Result<usize, Diagnostic> {
            if let std::collections::btree_map::Entry::Vacant(entry) = measured.entry(index) {
                let data: Presentation = rows[index].data.clone().inherit(style, 0);
                entry.insert(renderer.measure_content(
                    template,
                    &format!("document:{template}/{}", rows[index].key),
                    area.width,
                    &data,
                )?);
            }
            Ok(measured[&index].height() as usize + usize::from(rows[index].gap_after))
        };
        let mut index = scroll
            .key
            .as_ref()
            .and_then(|key| rows.iter().position(|row| &row.key == key))
            .unwrap_or(scroll.index.min(rows.len() - 1));
        let height = measure(self, index, &mut measured)?;
        let mut offset = scroll.offset.min(height) as i64;
        if scroll.key.as_ref() == Some(&rows[index].key)
            && let Some((binding, source, leading)) = &scroll.source
            && let Some(region) = measured[&index]
                .text_regions
                .iter()
                .rev()
                .find(|region| &region.binding == binding && region.source.start <= *source)
        {
            offset = usize::from(region.bounds.y).saturating_sub(*leading) as i64;
        }
        offset = offset.saturating_add(scroll.pending);
        scroll.pending = 0;
        while offset < 0 && index > 0 {
            index -= 1;
            offset = offset.saturating_add(measure(self, index, &mut measured)? as i64);
        }
        offset = offset.max(0);
        loop {
            let height = measure(self, index, &mut measured)? as i64;
            if offset < height {
                break;
            }
            if index + 1 == rows.len() {
                offset = height;
                break;
            }
            offset -= height;
            index += 1;
        }
        let mut available = 0;
        for next in index..rows.len() {
            available += measure(self, next, &mut measured)?;
            if available >= area.height as usize + offset as usize {
                break;
            }
        }
        // At the tail, backfill the viewport rather than leaving an empty end.
        let missing = (area.height as usize + offset as usize).saturating_sub(available);
        offset = offset.saturating_sub(missing as i64);
        while offset < 0 && index > 0 {
            index -= 1;
            offset = offset.saturating_add(measure(self, index, &mut measured)? as i64);
        }
        offset = offset.max(0);
        scroll.index = index;
        scroll.key = Some(rows[index].key.clone());
        scroll.offset = offset as usize;
        scroll.source = measured[&index]
            .text_regions
            .iter()
            .find(|region| usize::from(region.bounds.y) >= scroll.offset)
            .map(|region| {
                (
                    region.binding.clone(),
                    region.source.start,
                    usize::from(region.bounds.y) - scroll.offset,
                )
            });
        let mut y = -(offset as i32);
        for next in index..rows.len() {
            if y >= i32::from(area.height) {
                break;
            }
            let height = measure(self, next, &mut measured)?;
            measured[&next].paint_scrolled(frame, area, y);
            y += height as i32;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use crate::ui::layout::LayoutBundle;
    use tuirealm::ratatui::{Terminal, backend::TestBackend};

    fn row(index: usize, text: String, gap_after: bool) -> PresentedRow {
        PresentedRow {
            key: index.to_string(),
            data: Presentation::default().text("body", text),
            gap_after,
        }
    }
    #[test]
    fn visible_document_items_are_cached_and_resize_keeps_the_anchor() {
        let rows: Vec<_> = (0..5000)
            .map(|index| {
                row(
                    index,
                    format!("row-{index} alpha beta gamma delta epsilon 界"),
                    false,
                )
            })
            .collect();
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let mut terminal = Terminal::new(TestBackend::new(50, 20)).unwrap();
        let mut scroll = DocumentScroll::default();
        let area = Rect::new(2, 3, 24, 6);
        terminal
            .draw(|frame| {
                renderer
                    .paint_document(
                        frame,
                        area,
                        "rogue-document",
                        &rows,
                        &mut scroll,
                        Style::default(),
                    )
                    .unwrap()
            })
            .unwrap();
        let first = renderer.arrangement_count();
        assert!(
            first < 10,
            "only the visible document window should be arranged"
        );
        terminal
            .draw(|frame| {
                renderer
                    .paint_document(
                        frame,
                        area,
                        "rogue-document",
                        &rows,
                        &mut scroll,
                        Style::default(),
                    )
                    .unwrap()
            })
            .unwrap();
        assert_eq!(renderer.arrangement_count(), first);
        scroll.advance(9);
        terminal
            .draw(|frame| {
                renderer
                    .paint_document(
                        frame,
                        area,
                        "rogue-document",
                        &rows,
                        &mut scroll,
                        Style::default(),
                    )
                    .unwrap()
            })
            .unwrap();
        let key = scroll.key.clone();
        let old_source = scroll.source.clone().unwrap().1;
        let narrow = Rect::new(5, 2, 12, 6);
        terminal
            .draw(|frame| {
                renderer
                    .paint_document(
                        frame,
                        narrow,
                        "rogue-document",
                        &rows,
                        &mut scroll,
                        Style::default(),
                    )
                    .unwrap()
            })
            .unwrap();
        assert_eq!(scroll.key, key);
        assert!(scroll.source.as_ref().unwrap().1 <= old_source);
        // Movement is a delta, so a wrapping change cannot exhaust an obsolete
        // absolute scroll counter before we reach the actual start.
        scroll.advance(-10_000);
        terminal
            .draw(|frame| {
                renderer
                    .paint_document(
                        frame,
                        narrow,
                        "rogue-document",
                        &rows,
                        &mut scroll,
                        Style::default(),
                    )
                    .unwrap()
            })
            .unwrap();
        assert_eq!(scroll.index, 0);
        assert_eq!(scroll.offset, 0);
    }

    #[test]
    fn document_anchor_keeps_a_noninteractive_gap_at_the_top() {
        let rows = vec![
            row(0, "alpha".into(), true),
            row(1, "beta".into(), false),
            row(2, "gamma".into(), false),
        ];
        let mut renderer = Renderer::new(LayoutBundle::builtin().unwrap());
        let mut terminal = Terminal::new(TestBackend::new(20, 6)).unwrap();
        let mut scroll = DocumentScroll::default();
        scroll.advance(1);
        let area = Rect::new(2, 2, 10, 2);
        for _ in 0..2 {
            terminal
                .draw(|frame| {
                    renderer
                        .paint_document(
                            frame,
                            area,
                            "rogue-document",
                            &rows,
                            &mut scroll,
                            Style::default(),
                        )
                        .unwrap()
                })
                .unwrap();
            assert_eq!(scroll.index, 0);
            assert_eq!(scroll.offset, 1);
            assert_eq!(terminal.backend().buffer()[(2, 2)].symbol(), " ");
            assert_eq!(terminal.backend().buffer()[(2, 3)].symbol(), "b");
        }
    }
}
