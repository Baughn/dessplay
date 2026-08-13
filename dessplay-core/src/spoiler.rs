//! Discord-style `||spoiler||` tags: display-time parsing and the
//! deterministic scramble that hides them.
//!
//! Spoilers are a **display concern**: the raw `||...||` text is what
//! syncs, archives, and persists (the same rule as CTCP actions — "only
//! the display sites decode them", see [`crate::types::decode_action`]).
//! Each display surface decides how to hide a run:
//!
//! - the TUI chat pane scrambles + [`zalgo`]s hidden runs and drives the
//!   click/reveal state machine;
//! - the mpv OSD and the outbound IRC bridge use [`mask_message`] — the
//!   static generation-0 scramble, no zalgo — because neither surface
//!   has a reveal affordance (and the IRC channel is public and logged,
//!   with one group member reading chat *only* there).
//!
//! Everything here is deterministic (FNV-1a over message identity — no
//! RNG), so repaints are stable and tests reproduce exactly. The
//! `generation` parameter is the re-randomization frame counter: bumping
//! it re-rolls every substituted letter, which is what animates the
//! click "tease".

/// One parsed piece of a chat body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Segment<'a> {
    /// Ordinary text, shown as-is.
    Text(&'a str),
    /// The inside of a `||...||` run (bars not included).
    Spoiler(&'a str),
}

/// Parse Discord `||spoiler||` runs out of a message body.
///
/// Rules (deterministic left-to-right scan, no nesting):
/// - an opener `||` pairs with the nearest later `||` that has at least
///   one character between them;
/// - `||||` (nothing between) is literal — the scan moves past the first
///   pair and continues;
/// - an opener with no closer is literal, as is everything after it.
///
/// Empty `Text` segments are never emitted; the concatenation of all
/// segments plus four bar characters per spoiler reproduces the input.
pub fn parse(text: &str) -> Vec<Segment<'_>> {
    let mut out = Vec::new();
    let mut pos = 0; // start of pending literal text
    let mut scan = 0; // where the next opener search begins
    while let Some(open_rel) = text[scan..].find("||") {
        let open = scan + open_rel;
        let after = open + 2;
        let Some(close_rel) = text[after..].find("||") else {
            break; // unmatched opener: the rest is literal
        };
        if close_rel == 0 {
            // `||||`: empty spoilers are literal; step past this pair.
            scan = after;
            continue;
        }
        let close = after + close_rel;
        if open > pos {
            out.push(Segment::Text(&text[pos..open]));
        }
        out.push(Segment::Spoiler(&text[after..close]));
        pos = close + 2;
        scan = pos;
    }
    if pos < text.len() {
        out.push(Segment::Text(&text[pos..]));
    }
    out
}

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

/// FNV-1a folded over `bytes`, continuing from `state`.
fn fnv1a(state: u64, bytes: &[u8]) -> u64 {
    let mut h = state;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(FNV_PRIME);
    }
    h
}

/// Stable identity hash for spoiler number `index` of the message from
/// `sender` at `millis` — the seed every scramble of that run derives
/// from, so a run shows the same letters on every repaint (and on every
/// client that uses the same inputs).
pub fn seed(millis: u64, sender: &str, index: usize) -> u64 {
    let h = fnv1a(FNV_OFFSET, &millis.to_le_bytes());
    let h = fnv1a(h, sender.as_bytes());
    fnv1a(h, &(index as u64).to_le_bytes())
}

/// Per-character decision hash. `tag` domain-separates the substitute
/// and zalgo streams so they don't correlate.
fn decide(seed: u64, tag: u8, generation: u32, index: usize) -> u64 {
    let h = fnv1a(seed, &[tag]);
    let h = fnv1a(h, &u64::from(generation).to_le_bytes());
    fnv1a(h, &(index as u64).to_le_bytes())
}

/// Deterministic 1:1 scramble of a spoiler run: every alphanumeric
/// becomes a hash-picked ASCII letter/digit of the same class
/// (uppercase → `A-Z`, digit → `0-9`, anything else alphanumeric —
/// including CJK/Cyrillic, which must not leak — → `a-z`); all other
/// characters pass through. The char count is preserved exactly, which
/// the chat pane's char-count wrap and click hit-testing rely on.
pub fn scramble(text: &str, seed: u64, generation: u32) -> String {
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            if !c.is_alphanumeric() {
                return c;
            }
            let h = decide(seed, 0, generation, i);
            if c.is_numeric() {
                char::from(b'0' + (h % 10) as u8)
            } else if c.is_uppercase() {
                char::from(b'A' + (h % 26) as u8)
            } else {
                char::from(b'a' + (h % 26) as u8)
            }
        })
        .collect()
}

/// The "low-grade zalgo" combining marks: a conservative set (acute,
/// grave, tilde, breve, diaeresis, caron, dot below, tilde below) that
/// monospace fonts render reliably, mixing above- and below-marks.
const ZALGO_MARKS: &[char] = &[
    '\u{0300}', '\u{0301}', '\u{0303}', '\u{0306}', '\u{0308}', '\u{030C}', '\u{0323}', '\u{0330}',
];

/// Sprinkle combining marks over (already scrambled) spoiler text —
/// roughly one non-whitespace char in three gains one mark.
///
/// Combining marks are zero-width: ratatui writes the whole grapheme
/// cluster into one cell, so this cannot shift columns — but it *does*
/// add chars, so it must run **after** any char-count-based wrapping.
/// `char_offset` is the index of `text`'s first char within the whole
/// spoiler run, so a run split across wrap chunks keeps identical marks.
pub fn zalgo(text: &str, seed: u64, generation: u32, char_offset: usize) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for (i, c) in text.chars().enumerate() {
        out.push(c);
        if c.is_whitespace() {
            continue;
        }
        let h = decide(seed, 1, generation, char_offset + i);
        if h.is_multiple_of(3) {
            out.push(ZALGO_MARKS[((h / 3) as usize) % ZALGO_MARKS.len()]);
        }
    }
    out
}

/// The static mask for surfaces without a reveal affordance (mpv OSD,
/// outbound IRC): spoiler runs replaced by their generation-0 scramble,
/// bars dropped, no zalgo. `seed_base` is the message's shared-clock
/// millis where known (OSD — matches the TUI's letters) or any fixed
/// value where not (IRC tap, which runs before the timestamp exists).
pub fn mask_message(text: &str, seed_base: u64, sender: &str) -> String {
    if !text.contains("||") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    for segment in parse(text) {
        match segment {
            Segment::Text(t) => out.push_str(t),
            Segment::Spoiler(s) => {
                out.push_str(&scramble(s, seed(seed_base, sender, index), 0));
                index += 1;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn spoiler_count(segments: &[Segment<'_>]) -> usize {
        segments
            .iter()
            .filter(|s| matches!(s, Segment::Spoiler(_)))
            .count()
    }

    #[test]
    fn parse_plain_text() {
        assert_eq!(parse("hello world"), vec![Segment::Text("hello world")]);
        assert_eq!(parse(""), vec![]);
    }

    #[test]
    fn parse_single_spoiler() {
        assert_eq!(
            parse("a ||b|| c"),
            vec![
                Segment::Text("a "),
                Segment::Spoiler("b"),
                Segment::Text(" c"),
            ]
        );
    }

    #[test]
    fn parse_spoiler_at_edges() {
        assert_eq!(parse("||b||"), vec![Segment::Spoiler("b")]);
        assert_eq!(
            parse("||start|| end"),
            vec![Segment::Spoiler("start"), Segment::Text(" end")]
        );
        assert_eq!(
            parse("start ||end||"),
            vec![Segment::Text("start "), Segment::Spoiler("end")]
        );
    }

    #[test]
    fn parse_two_spoilers() {
        assert_eq!(
            parse("||a|| and ||b||"),
            vec![
                Segment::Spoiler("a"),
                Segment::Text(" and "),
                Segment::Spoiler("b"),
            ]
        );
    }

    #[test]
    fn parse_adjacent_spoilers() {
        assert_eq!(
            parse("||a||||b||"),
            vec![Segment::Spoiler("a"), Segment::Spoiler("b")]
        );
    }

    #[test]
    fn parse_unmatched_bars_are_literal() {
        assert_eq!(parse("a ||b"), vec![Segment::Text("a ||b")]);
        assert_eq!(parse("||"), vec![Segment::Text("||")]);
        assert_eq!(parse("a | b || c"), vec![Segment::Text("a | b || c")]);
    }

    #[test]
    fn parse_empty_spoiler_is_literal() {
        assert_eq!(parse("||||"), vec![Segment::Text("||||")]);
        // The scan steps past the empty pair and still finds a real run.
        assert_eq!(
            parse("||||x||"),
            vec![Segment::Text("||"), Segment::Spoiler("x")]
        );
    }

    #[test]
    fn parse_extra_bars_inside() {
        // `|||a|||`: opener, spoiler "|a", closer, trailing literal "|".
        assert_eq!(
            parse("|||a|||"),
            vec![Segment::Spoiler("|a"), Segment::Text("|")]
        );
    }

    proptest! {
        /// Segments reassemble to the input: every char is accounted
        /// for, with exactly four bar chars consumed per spoiler.
        #[test]
        fn parse_accounts_for_every_char(text in "\\PC*") {
            let segments = parse(&text);
            let segment_chars: usize = segments
                .iter()
                .map(|s| match s {
                    Segment::Text(t) | Segment::Spoiler(t) => t.chars().count(),
                })
                .sum();
            prop_assert_eq!(
                segment_chars + 4 * spoiler_count(&segments),
                text.chars().count()
            );
        }

        /// Scramble is deterministic, char-count-preserving, leaves
        /// non-alphanumerics alone, and keeps the character class.
        #[test]
        fn scramble_invariants(text in "\\PC*", seed in any::<u64>(), generation in any::<u32>()) {
            let out = scramble(&text, seed, generation);
            prop_assert_eq!(&out, &scramble(&text, seed, generation));
            prop_assert_eq!(out.chars().count(), text.chars().count());
            for (a, b) in text.chars().zip(out.chars()) {
                if a.is_alphanumeric() {
                    prop_assert!(b.is_ascii_alphanumeric(), "{a:?} -> {b:?} not ASCII");
                    prop_assert_eq!(a.is_numeric(), b.is_ascii_digit());
                    if !a.is_numeric() {
                        prop_assert_eq!(a.is_uppercase(), b.is_ascii_uppercase());
                    }
                } else {
                    prop_assert_eq!(a, b);
                }
            }
        }

        /// Stripping the combining marks recovers the input exactly, and
        /// marks never follow whitespace.
        ///
        /// The input is constrained to text that carries none of our
        /// mark codepoints: with one already present (NFD text can),
        /// strip-based recovery is ill-defined — an original mark is
        /// indistinguishable from an inserted one. Nothing in
        /// production strips marks; this pins "zalgo only inserts,
        /// never alters" on the domain where stripping is well-defined.
        #[test]
        fn zalgo_marks_strip_cleanly(raw in "\\PC*", seed in any::<u64>(), generation in any::<u32>(), offset in 0usize..64) {
            let text: String = raw.chars().filter(|c| !ZALGO_MARKS.contains(c)).collect();
            let out = zalgo(&text, seed, generation, offset);
            let stripped: String = out.chars().filter(|c| !ZALGO_MARKS.contains(c)).collect();
            prop_assert_eq!(stripped, text.clone());
            let mut prev: Option<char> = None;
            for c in out.chars() {
                if ZALGO_MARKS.contains(&c) {
                    prop_assert!(prev.is_some_and(|p| !p.is_whitespace()));
                }
                prev = Some(c);
            }
        }

        /// Splitting a run at any char boundary and zalgo-ing the halves
        /// with the right offsets equals zalgo-ing the whole run: wrap
        /// chunk boundaries can't change which marks appear.
        #[test]
        fn zalgo_is_split_stable(text in "\\PC{0,40}", seed in any::<u64>(), split in 0usize..41) {
            let chars: Vec<char> = text.chars().collect();
            let split = split.min(chars.len());
            let head: String = chars[..split].iter().collect();
            let tail: String = chars[split..].iter().collect();
            let joined = format!("{}{}", zalgo(&head, seed, 0, 0), zalgo(&tail, seed, 0, split));
            prop_assert_eq!(joined, zalgo(&text, seed, 0, 0));
        }
    }

    #[test]
    fn scramble_generation_changes_letters() {
        let a = scramble("secret words", 42, 0);
        let b = scramble("secret words", 42, 1);
        assert_ne!(a, b);
    }

    #[test]
    fn scramble_hides_non_ascii_alphanumerics() {
        // CJK and Cyrillic must not leak through the mask.
        let out = scramble("秘密のことば тайна", 7, 0);
        assert!(out.is_ascii());
        assert!(!out.contains('秘') && !out.contains('т'));
    }

    #[test]
    fn mask_message_drops_bars_and_hides_content() {
        let masked = mask_message("a ||b|| c", 1234, "Baughn");
        assert!(!masked.contains('|'));
        assert!(masked.starts_with("a ") && masked.ends_with(" c"));
        // Matches the TUI's generation-0 scramble letter for letter.
        assert_eq!(
            masked,
            format!("a {} c", scramble("b", seed(1234, "Baughn", 0), 0))
        );
    }

    #[test]
    fn mask_message_without_spoilers_is_identity() {
        assert_eq!(mask_message("plain text", 0, "x"), "plain text");
        assert_eq!(mask_message("half ||open", 0, "x"), "half ||open");
    }
}
