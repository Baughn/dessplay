//! The one table row. Panes that lay out columns — the playlist's tag
//! columns, The List's episode/watchers spreadsheet — build each line
//! through [`table_row`], so the column discipline (display-cell widths,
//! truncation with `…`, fixed cells that never drift with the name's
//! length) exists exactly once. "A long filename shoved the tags off
//! the pane" was the observable drift this replaces.
//!
//! A row is one flexible cell (styled spans; truncated and padded to
//! whatever width the fixed cells leave over) followed by fixed-width
//! cells, each preceded by a single space. All widths are terminal
//! *display* cells (CJK glyphs occupy two), never char counts —
//! char-count padding drifts the columns on every Japanese title.

use tuirealm::ratatui::style::Style;
use tuirealm::ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// Horizontal alignment of a fixed cell's text within its column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Text at the left edge of the column, padding to the right.
    Left,
    /// Text centered in the column (odd padding leans left).
    Center,
    /// Text at the right edge of the column, padding to the left.
    Right,
}

/// One fixed-width table cell.
pub struct Cell {
    /// The cell's text; truncated with `…` if wider than the column.
    pub text: String,
    /// Style applied to the text (padding is unstyled).
    pub style: Style,
    /// Column width in display cells.
    pub width: usize,
    /// Where the text sits within the column.
    pub align: Align,
}

impl Cell {
    /// A cell with the given text, style, column width, and alignment.
    pub fn new(text: impl Into<String>, style: Style, width: usize, align: Align) -> Self {
        Self {
            text: text.into(),
            style,
            width,
            align,
        }
    }
}

/// The flexible cell never shrinks below this many display cells, even
/// in a pane too narrow for all its columns (the fixed cells then clip
/// at the border, which beats an unreadable empty name).
const MIN_FLEX: usize = 8;

/// Truncate `s` to at most `max` display cells, appending `…` (one cell)
/// when anything was cut. Returns the text and its display width — cells,
/// not chars, so CJK (two cells per glyph) truncates and pads correctly.
pub fn truncate_display(s: &str, max: usize) -> (String, usize) {
    use unicode_width::UnicodeWidthChar;
    if max == 0 {
        return (String::new(), 0);
    }
    let full = s.width();
    if full <= max {
        return (s.to_string(), full);
    }
    let budget = max - 1; // reserve a cell for the ellipsis
    let mut out = String::new();
    let mut used = 0;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > budget {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    (out, used + 1)
}

/// Truncate the start of `s` to at most `max` display cells, retaining its
/// suffix behind an ellipsis. Useful for paths whose final component is the
/// part that distinguishes otherwise-identical roots.
pub fn truncate_display_start(s: &str, max: usize) -> (String, usize) {
    use unicode_width::UnicodeWidthChar;
    if max == 0 {
        return (String::new(), 0);
    }
    let full = s.width();
    if full <= max {
        return (s.to_string(), full);
    }
    let budget = max - 1;
    let mut suffix = Vec::new();
    let mut used = 0;
    for ch in s.chars().rev() {
        let width = ch.width().unwrap_or(0);
        if used + width > budget {
            break;
        }
        suffix.push(ch);
        used += width;
    }
    suffix.reverse();
    let mut out = String::from("…");
    out.extend(suffix);
    (out, used + 1)
}

/// Build one table row exactly `width` display cells wide: the `flex`
/// spans truncated and padded to the width the fixed `cells` leave over,
/// then each cell padded to its declared width behind a single-space
/// separator. Whatever the flex content, every fixed cell starts at the
/// same column.
pub fn table_row(width: usize, flex: Vec<Span<'static>>, cells: Vec<Cell>) -> Line<'static> {
    let reserved: usize = cells.iter().map(|c| c.width + 1).sum();
    let flex_width = width.saturating_sub(reserved).max(MIN_FLEX);

    let mut spans: Vec<Span<'static>> = Vec::with_capacity(flex.len() + cells.len() * 2 + 1);
    let mut used = 0;
    for span in flex {
        if used >= flex_width {
            break;
        }
        let (text, w) = truncate_display(&span.content, flex_width - used);
        used += w;
        spans.push(Span::styled(text, span.style));
    }
    if used < flex_width {
        spans.push(Span::raw(" ".repeat(flex_width - used)));
    }

    for cell in cells {
        let (text, w) = truncate_display(&cell.text, cell.width);
        let pad = cell.width - w;
        let (left, right) = match cell.align {
            Align::Left => (0, pad),
            Align::Center => (pad / 2, pad - pad / 2),
            Align::Right => (pad, 0),
        };
        spans.push(Span::raw(" ".repeat(1 + left)));
        spans.push(Span::styled(text, cell.style));
        if right > 0 {
            spans.push(Span::raw(" ".repeat(right)));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn cells(specs: &[(&str, usize, Align)]) -> Vec<Cell> {
        specs
            .iter()
            .map(|(t, w, a)| Cell::new(*t, Style::default(), *w, *a))
            .collect()
    }

    #[test]
    fn short_name_pads_and_aligns_cells() {
        let line = table_row(
            20,
            vec![Span::raw("name")],
            cells(&[("aa", 4, Align::Left), ("b", 3, Align::Center)]),
        );
        //             flex (11)      + " " + left(4) + " " + center(3)
        assert_eq!(line_text(&line), "name        aa    b ");
    }

    #[test]
    fn long_name_truncates_with_ellipsis() {
        let line = table_row(
            16,
            vec![Span::raw("a very long name indeed")],
            cells(&[("tag", 4, Align::Left)]),
        );
        let text = line_text(&line);
        assert_eq!(text, "a very lon… tag ");
        assert_eq!(text.width(), 16);
    }

    #[test]
    fn cjk_name_truncates_and_pads_by_display_cells() {
        // Each glyph is two cells; truncation must not split one, and the
        // padding must account for the doubled width.
        let line = table_row(
            16,
            vec![Span::raw("葬送のフリーレン")],
            cells(&[("tag", 4, Align::Left)]),
        );
        let text = line_text(&line);
        assert_eq!(text.width(), 16);
        assert!(text.ends_with(" tag "), "{text:?}");
    }

    #[test]
    fn multi_span_flex_truncates_across_spans() {
        let line = table_row(
            14,
            vec![Span::raw("first"), Span::raw(" second")],
            cells(&[("t", 1, Align::Left)]),
        );
        // flex_width = 14 - 2 = 12: "first" fits, " second" clips.
        assert_eq!(line_text(&line), "first second t");
    }

    #[test]
    fn overwide_cell_text_truncates_within_its_column() {
        let line = table_row(
            16,
            vec![Span::raw("n")],
            cells(&[("overlong", 4, Align::Left)]),
        );
        let text = line_text(&line);
        assert_eq!(text.width(), 16);
        assert!(text.ends_with(" ove…"), "{text:?}");
    }

    #[test]
    fn narrow_width_keeps_a_minimum_flex_cell() {
        // Cells reserve more than the row width: the flex cell floors at
        // MIN_FLEX and the row overflows (clipped by the border) instead
        // of vanishing the name.
        let line = table_row(
            6,
            vec![Span::raw("somename")],
            cells(&[("tag", 10, Align::Left)]),
        );
        let text = line_text(&line);
        assert!(text.starts_with("somename"), "{text:?}");
    }

    #[test]
    fn start_truncation_keeps_the_distinguishing_suffix() {
        let (text, width) = truncate_display_start("/a/very/long/path/19", 10);
        assert_eq!(text, "…g/path/19");
        assert_eq!(width, 10);
    }

    mod properties {
        use super::*;
        use proptest::prelude::*;

        /// Arbitrary text mixing ASCII and double-width CJK.
        fn text() -> impl Strategy<Value = String> {
            proptest::collection::vec(
                prop_oneof![
                    proptest::char::range(' ', '~'),
                    proptest::char::range('ぁ', 'ゖ'),
                    proptest::char::range('一', '鿋'),
                ],
                0..30,
            )
            .prop_map(|chars| chars.into_iter().collect())
        }

        proptest! {
            /// The columns never drift: whatever the flex content (any
            /// width, any script), the row's total display width and the
            /// entire fixed-cell suffix are identical — this is the
            /// "long filename shoves the tags off-screen" bug class made
            /// unrepresentable.
            #[test]
            fn fixed_cells_are_independent_of_flex_content(
                a in text(),
                b in text(),
                width in 12usize..80,
                cell_text in "[ -~]{0,12}",
                cell_width in 1usize..12,
            ) {
                let mk = |name: String| {
                    table_row(
                        width,
                        vec![Span::raw(name)],
                        vec![
                            Cell::new(cell_text.clone(), Style::default(), cell_width, Align::Left),
                            Cell::new("x", Style::default(), 3, Align::Center),
                        ],
                    )
                };
                let ta = line_text(&mk(a));
                let tb = line_text(&mk(b));
                let reserved = (cell_width + 1) + (3 + 1);
                let expected = width.saturating_sub(reserved).max(MIN_FLEX) + reserved;
                prop_assert_eq!(ta.width(), expected);
                prop_assert_eq!(tb.width(), expected);
                // Both rows end with the identical reserved-width
                // rendering of the fixed cells: same columns, same text.
                let tail_a: String = ta.chars().skip(ta.chars().count() - tail_len(&ta, reserved)).collect();
                let tail_b: String = tb.chars().skip(tb.chars().count() - tail_len(&tb, reserved)).collect();
                prop_assert_eq!(&tail_a, &tail_b);
                prop_assert_eq!(tail_a.width(), reserved);
            }
        }

        /// Number of trailing chars spanning exactly `cells` display
        /// cells (the fixed-cell suffix is ASCII, one cell per char, but
        /// count defensively).
        fn tail_len(s: &str, cells: usize) -> usize {
            use unicode_width::UnicodeWidthChar;
            let mut acc = 0;
            let mut n = 0;
            for c in s.chars().rev() {
                if acc >= cells {
                    break;
                }
                acc += c.width().unwrap_or(0);
                n += 1;
            }
            n
        }
    }
}
