//! Best-effort episode-number extraction from a filename, for List
//! entries with no AniDB link (design.md, Advancing next_ep). AniDB-less
//! files still often carry a parseable episode number in their name --
//! this recognizes the common, high-confidence fansub conventions and
//! declines (returns `None`) on anything else, since a wrong guess would
//! silently bump `next_ep` to the wrong value.

/// Extract a plausible episode number from a filename, as a plain decimal
/// string (no leading zeros, e.g. `"5"` not `"05"`) -- matching how
/// `next_ep` is written by hand elsewhere ("12", "S3-05", ...).
///
/// Recognizes two conventions, in priority order: an explicit `E12` /
/// `EP12` / `Episode 12` token, and the classic dash separator
/// (`Title - 12`, optionally followed by a `v2`-style revision suffix).
/// Declines whenever a season marker (`S2`, `S02E05`, `Season 2`) appears
/// anywhere in the name -- a season-qualified filename needs season *and*
/// episode tracked together, which a flat numeric `next_ep` can't
/// express, so guessing here would silently drop the season.
pub fn parse_episode_number(filename: &str) -> Option<String> {
    if has_season_marker(filename) {
        return None;
    }
    parse_explicit_episode_token(filename).or_else(|| parse_dash_number(filename))
}

/// A run of ASCII digits starting at `bytes[start]`, and the index just
/// past it. `None` if `start` isn't a digit.
fn digit_run(bytes: &[u8], start: usize) -> Option<(usize, usize)> {
    if start >= bytes.len() || !bytes[start].is_ascii_digit() {
        return None;
    }
    let mut end = start;
    while end < bytes.len() && bytes[end].is_ascii_digit() {
        end += 1;
    }
    Some((start, end))
}

/// Is `bytes[i]` a word-boundary character (not part of an identifier) —
/// used to make sure a matched token isn't a substring of a larger word.
fn is_boundary(bytes: &[u8], i: usize) -> bool {
    match bytes.get(i) {
        None => true,
        Some(b) => !b.is_ascii_alphanumeric(),
    }
}

/// True if `filename` contains a season marker: the word "season", or an
/// `S<digits>` token (optionally followed by `E<digits>`) at a word
/// boundary on both sides.
fn has_season_marker(filename: &str) -> bool {
    let lower = filename.to_ascii_lowercase();
    if lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| word == "season")
    {
        return true;
    }
    let bytes = lower.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] != b's' {
            continue;
        }
        if i > 0 && !is_boundary(bytes, i - 1) {
            continue;
        }
        let Some((_, digits_end)) = digit_run(bytes, i + 1) else {
            continue;
        };
        if is_boundary(bytes, digits_end) {
            return true;
        }
        // "S02E05": digits immediately followed by an E<digits> token is
        // still a season marker, just spelled without a boundary between
        // the season and episode numbers.
        if bytes.get(digits_end) == Some(&b'e')
            && digit_run(bytes, digits_end + 1).is_some_and(|(_, e)| is_boundary(bytes, e))
        {
            return true;
        }
    }
    false
}

/// A trailing decimal token with no leading zero, or `None` if the digits
/// don't parse (shouldn't happen for a matched digit run) or are empty.
fn normalize(digits: &str) -> Option<String> {
    digits.parse::<u32>().ok().map(|n| n.to_string())
}

/// An explicit `E12` / `EP12` / `Episode 12` token, at a word boundary.
fn parse_explicit_episode_token(filename: &str) -> Option<String> {
    let lower = filename.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for i in 0..bytes.len() {
        if i > 0 && !is_boundary(bytes, i - 1) {
            continue;
        }
        let rest = &bytes[i..];
        let digit_start = if rest.starts_with(b"episode") {
            i + "episode".len()
        } else if rest.starts_with(b"ep") {
            i + "ep".len()
        } else if rest.starts_with(b"e") {
            i + 1
        } else {
            continue;
        };
        // Skip a single separating space ("Episode 12"), not required
        // for the compact forms ("E12", "EP12").
        let digit_start = if bytes.get(digit_start) == Some(&b' ') {
            digit_start + 1
        } else {
            digit_start
        };
        let Some((start, end)) = digit_run(bytes, digit_start) else {
            continue;
        };
        // 1-3 digits only -- a longer run is more likely a CRC/date than
        // an episode number, and isn't a pattern real releases use here.
        if end - start > 3 || !is_boundary(bytes, end) {
            continue;
        }
        if let Some(number) = normalize(&lower[start..end]) {
            return Some(number);
        }
    }
    None
}

/// The classic dash-separated episode number: `Title - 12`, `Title - 12v2`.
fn parse_dash_number(filename: &str) -> Option<String> {
    let mut search_from = 0;
    while let Some(rel) = filename[search_from..].find(" - ") {
        let dash_end = search_from + rel + " - ".len();
        let bytes = filename.as_bytes();
        if let Some((start, end)) = digit_run(bytes, dash_end)
            && end - start <= 3
        {
            // Allow a `v2`-style revision suffix right after the number;
            // either way, what follows must be a boundary.
            let after = if bytes.get(end) == Some(&b'v') {
                digit_run(bytes, end + 1).map_or(end, |(_, e)| e)
            } else {
                end
            };
            if is_boundary(bytes, after)
                && let Some(number) = normalize(&filename[start..end])
            {
                return Some(number);
            }
        }
        search_from = dash_end;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_episode_number;

    #[test]
    fn dash_convention() {
        assert_eq!(
            parse_episode_number("[SubGroup] Some Obscure Show - 12.mkv"),
            Some("12".into())
        );
        assert_eq!(
            parse_episode_number("[SubGroup] Some Obscure Show - 05 [1080p][deadbeef].mkv"),
            Some("5".into())
        );
        assert_eq!(parse_episode_number("Title - 12v2.mkv"), Some("12".into()));
    }

    #[test]
    fn explicit_episode_token() {
        assert_eq!(parse_episode_number("Title E12.mkv"), Some("12".into()));
        assert_eq!(parse_episode_number("Title EP12.mkv"), Some("12".into()));
        assert_eq!(
            parse_episode_number("Title Episode 12.mkv"),
            Some("12".into())
        );
        assert_eq!(parse_episode_number("Title E05.mkv"), Some("5".into()));
    }

    #[test]
    fn declines_season_qualified_names() {
        assert_eq!(parse_episode_number("Title S02E05.mkv"), None);
        assert_eq!(parse_episode_number("Title - S2 - 05.mkv"), None);
        assert_eq!(parse_episode_number("Title Season 2 - 05.mkv"), None);
    }

    #[test]
    fn declines_ambiguous_or_unmatched_names() {
        // No recognized token at all.
        assert_eq!(parse_episode_number("Title [1080p][deadbeef].mkv"), None);
        // A bare number with no dash/E marker is too risky to guess.
        assert_eq!(parse_episode_number("Title 12.mkv"), None);
        // A resolution tag alone (not after " - ") must not be mistaken
        // for an episode number.
        assert_eq!(parse_episode_number("Title [720p].mkv"), None);
    }

    #[test]
    fn does_not_confuse_a_word_containing_e_for_the_marker() {
        // "Frieren" contains no "e<digit>" token, so this must not
        // spuriously match; only the dash convention should fire.
        assert_eq!(
            parse_episode_number("[Judas] Frieren - 03.mkv"),
            Some("3".into())
        );
    }
}
