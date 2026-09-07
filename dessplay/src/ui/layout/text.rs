use std::ops::Range;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// One measured display row and its source-character mapping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    /// Visual row within the measured content; inline prefix/body pieces can share a row.
    pub row: usize,
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
            wrap_body(
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
                row: rows.len(),
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

/// A first-line prefix is measured independently of wrapped body text, retaining
/// separate source intervals when wrapping drops a boundary space.
pub(super) fn measure_flow(
    text: &str,
    width: usize,
    prefix_chars: usize,
    indent: usize,
    wrap: bool,
    ellipsis: bool,
) -> Vec<Fragment> {
    if prefix_chars == 0 || width == 0 {
        return measure_text(text, width, 0, indent, wrap, ellipsis);
    }
    let prefix: String = text
        .chars()
        .take(prefix_chars)
        .map(|c| if c == '\n' { ' ' } else { c })
        .collect();
    let body: String = text.chars().skip(prefix_chars).collect();
    let prefix_width = prefix.width();
    let mut rows = measure_text(&prefix, width, 0, 0, false, ellipsis);
    if body.is_empty() {
        return rows;
    }
    if !wrap && prefix_width >= width {
        return rows;
    }
    let indent = indent.min(width.saturating_sub(1));
    let first_glyph = body
        .split('\n')
        .next()
        .unwrap_or("")
        .chars()
        .find(|c| !c.is_whitespace())
        .and_then(UnicodeWidthChar::width)
        .unwrap_or(0);
    let next_row = usize::from(
        prefix_width >= width
            || (wrap
                && first_glyph > width.saturating_sub(prefix_width)
                && first_glyph <= width - indent),
    );
    let first_indent = if next_row == 0 { prefix_width } else { indent };
    for mut fragment in measure_text(&body, width, first_indent, indent, wrap, ellipsis) {
        fragment.row += next_row;
        fragment.source = fragment.source.start + prefix_chars..fragment.source.end + prefix_chars;
        rows.push(fragment);
    }
    rows
}

/// Greedy word-wrap over **display width** (terminal cells, via
/// `unicode-width`) — ratatui lays out by cell width, so double-width
/// CJK must consume two cells of budget, not one. The first visual line
/// gets `first_width` cells (the chat prefix eats into it); later lines
/// get `rest_width`. Breaks at spaces where possible, hard-breaks any
/// word wider than the available cells.
///
/// Each chunk is a contiguous char-slice of `text` (only boundary join
/// spaces are dropped); the second tuple element is the chunk's starting
/// **char offset** in `text` (identity, not geometry), which lets
/// callers map char ranges of the input (spoiler runs) onto the wrapped
/// lines.
pub(crate) fn wrap_body(text: &str, first_width: usize, rest_width: usize) -> Vec<(String, usize)> {
    use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
    let width_for = |idx: usize| if idx == 0 { first_width } else { rest_width }.max(1);
    let mut lines: Vec<(String, usize)> = Vec::new();
    let mut cur = String::new();
    let mut cur_start = 0;
    let mut width = width_for(0);
    let mut next_word_start = 0;
    for mut word in text.split(' ') {
        let mut word_start = next_word_start;
        next_word_start += word.chars().count() + 1;
        loop {
            let cur_cells = cur.width();
            let space = usize::from(!cur.is_empty());
            let word_cells = word.width();
            if cur_cells + space + word_cells <= width {
                if space == 1 {
                    cur.push(' ');
                } else {
                    // Chunk begins with this word.
                    cur_start = word_start;
                }
                cur.push_str(word);
                break;
            }
            if cur.is_empty() {
                // Word alone exceeds the line: hard-break it after the
                // last char that still fits `width` cells — but always
                // after at least one, so a single over-wide char cannot
                // stall the loop.
                let mut used = 0;
                let mut split_at = word.len();
                for (taken, (i, c)) in word.char_indices().enumerate() {
                    let cells = c.width().unwrap_or(0);
                    if taken > 0 && used + cells > width {
                        split_at = i;
                        break;
                    }
                    used += cells;
                }
                let (head, tail) = word.split_at(split_at);
                cur_start = word_start;
                cur.push_str(head);
                lines.push((std::mem::take(&mut cur), cur_start));
                width = width_for(lines.len());
                word_start += head.chars().count();
                word = tail;
            } else {
                // Flush and retry the word on a fresh line.
                lines.push((std::mem::take(&mut cur), cur_start));
                width = width_for(lines.len());
            }
        }
    }
    lines.push((cur, cur_start));
    lines
}
