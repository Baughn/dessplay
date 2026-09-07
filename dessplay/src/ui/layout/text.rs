use std::ops::Range;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// One measured display row and its source-character mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    /// Text painted without a second wrapping pass.
    pub text: String,
    /// Character range in the original binding (not UTF-8 bytes).
    pub source: Range<usize>,
    /// Leading cells reserved for a continuation prefix.
    pub indent: usize,
    /// Display width, excluding indentation.
    pub width: usize,
}

/// Measure text once at a fixed integer content width. Newlines are explicit;
/// continuation indentation is independent of the first-line prefix.
pub fn measure_text(
    text: &str,
    width: usize,
    first_indent: usize,
    continuation_indent: usize,
    wrap: bool,
    ellipsis: bool,
) -> Vec<Fragment> {
    if width == 0 {
        return Vec::new();
    }
    let mut rows = Vec::new();
    let mut offset = 0;
    for line in text.split('\n') {
        let first = if rows.is_empty() {
            first_indent
        } else {
            continuation_indent
        }
        .min(width);
        let measured = if wrap {
            crate::ui::components::wrap_body(
                line,
                width.saturating_sub(first),
                width.saturating_sub(continuation_indent),
            )
        } else {
            vec![(line.into(), 0)]
        };
        for (part, (text, source)) in measured.into_iter().enumerate() {
            let indent = if part == 0 {
                first
            } else {
                continuation_indent.min(width)
            };
            let available = width - indent;
            let overflow = text.width() > available;
            let limit = available.saturating_sub(usize::from(ellipsis && overflow));
            let mut shown = String::new();
            let mut used = 0;
            let mut count = 0;
            for c in text.chars() {
                let cells = c.width().unwrap_or(0);
                if used + cells > limit {
                    break;
                }
                shown.push(c);
                used += cells;
                count += 1;
            }
            if ellipsis && overflow && available > 0 {
                shown.push('…');
                used += 1;
            }
            rows.push(Fragment {
                text: shown,
                source: offset + source..offset + source + count,
                indent,
                width: used,
            });
        }
        offset += line.chars().count() + 1;
    }
    rows
}
