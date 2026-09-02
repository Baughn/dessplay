//! The compiled-in changelog (design.md, Changelog).
//!
//! `CHANGELOG.md` at the repo root is embedded at compile time and
//! parsed once, lazily. Entries are grouped by **calendar day** —
//! Factorio-inspired, without the rigidity: `## YYYY-MM-DD` headers in
//! strictly descending order, `- ` bullets (optionally prefixed with a
//! one-word `Category: `), continuation lines indented two spaces.
//!
//! Each client persists a local [`SeenMarker`] (the `changelog_seen`
//! settings key — deliberately *not* a `Settings` field, so a whole-
//! struct settings save can never clobber it); [`unseen`] filters what
//! the startup "What's new" modal shows. The marker carries the entry
//! count of its newest day, so entries appended to a day the user
//! already saw are still surfaced later.
//!
//! The parser never panics and a malformed embedded file degrades to an
//! empty changelog at runtime; the test `embedded_changelog_parses` is
//! the real gate — a bad entry fails the test suite, not the user.

use std::fmt;
use std::str::FromStr;
use std::sync::LazyLock;

use chrono::NaiveDate;

/// The embedded changelog source (repo root, so it also reads on GitHub).
const RAW: &str = include_str!("../../CHANGELOG.md");

static PARSED: LazyLock<Vec<ChangelogDay>> = LazyLock::new(|| match parse(RAW) {
    Ok(days) => days,
    Err(e) => {
        // Unreachable in a binary that passed the test suite
        // (`embedded_changelog_parses`); degrade rather than panic.
        tracing::error!("embedded CHANGELOG.md is malformed: {e}");
        Vec::new()
    }
});

/// The embedded changelog, newest day first.
pub fn entries() -> &'static [ChangelogDay] {
    &PARSED
}

/// One calendar day's entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangelogDay {
    /// The `## YYYY-MM-DD` header date.
    pub date: NaiveDate,
    /// The day's bullets, in file order. Never empty.
    pub entries: Vec<ChangelogEntry>,
}

/// One `- ` bullet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangelogEntry {
    /// The optional one-word `Category: ` prefix (e.g. "Added", "Fixed").
    pub category: Option<String>,
    /// The entry text, continuation lines joined with spaces.
    pub text: String,
}

/// How far a client has read: everything up to `date`, and the first
/// `count` entries *of* that date, is seen. Serialized `YYYY-MM-DD:count`
/// under the `changelog_seen` settings key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SeenMarker {
    /// The newest day the user has seen (any of).
    pub date: NaiveDate,
    /// How many of that day's entries were shown.
    pub count: usize,
}

impl fmt::Display for SeenMarker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.date.format("%Y-%m-%d"), self.count)
    }
}

impl FromStr for SeenMarker {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (date, count) = s
            .split_once(':')
            .ok_or_else(|| format!("seen marker {s:?}: expected YYYY-MM-DD:count"))?;
        Ok(SeenMarker {
            date: NaiveDate::parse_from_str(date, "%Y-%m-%d")
                .map_err(|e| format!("seen marker date {date:?}: {e}"))?,
            count: count
                .parse()
                .map_err(|e| format!("seen marker count {count:?}: {e}"))?,
        })
    }
}

/// The marker that records `days` (newest-first) as fully seen; `None`
/// for an empty changelog.
pub fn latest_marker(days: &[ChangelogDay]) -> Option<SeenMarker> {
    days.first().map(|day| SeenMarker {
        date: day.date,
        count: day.entries.len(),
    })
}

/// The days (newest-first, like the input) holding entries the marker
/// has not seen. The marker's own day is included *partially* when it
/// grew since (entries beyond `count`); a count past the day's real
/// length clamps to fully-seen (entries can be reworded away).
pub fn unseen(days: &[ChangelogDay], marker: Option<SeenMarker>) -> Vec<ChangelogDay> {
    let Some(marker) = marker else {
        return days.to_vec();
    };
    let mut out = Vec::new();
    for day in days {
        if day.date > marker.date {
            out.push(day.clone());
        } else if day.date == marker.date && day.entries.len() > marker.count {
            out.push(ChangelogDay {
                date: day.date,
                entries: day.entries[marker.count..].to_vec(),
            });
        } else {
            // Days are strictly descending: everything from here on is
            // older than (or exactly) the marker.
            break;
        }
    }
    out
}

/// Parse a changelog. Freeform preamble before the first `## ` header is
/// ignored; from there the format is strict (see the module docs) so a
/// malformed entry is caught by the test suite, with a line number.
pub fn parse(input: &str) -> Result<Vec<ChangelogDay>, String> {
    let mut days: Vec<ChangelogDay> = Vec::new();
    for (idx, line) in input.lines().enumerate() {
        let lineno = idx + 1;
        let Some(current) = days.last_mut() else {
            // Preamble: skip until the first day header.
            if let Some(header) = line.strip_prefix("## ") {
                days.push(parse_header(header, lineno)?);
            }
            continue;
        };
        if let Some(header) = line.strip_prefix("## ") {
            if current.entries.is_empty() {
                return Err(format!("line {lineno}: previous day has no entries"));
            }
            let day = parse_header(header, lineno)?;
            if day.date >= current.date {
                return Err(format!(
                    "line {lineno}: day {} is not older than the {} above it \
                     (newest first, no duplicates)",
                    day.date, current.date
                ));
            }
            days.push(day);
        } else if let Some(text) = line.strip_prefix("- ") {
            let text = text.trim();
            if text.is_empty() {
                return Err(format!("line {lineno}: empty entry"));
            }
            current.entries.push(parse_entry(text));
        } else if let Some(cont) = line.strip_prefix("  ") {
            let cont = cont.trim();
            let Some(entry) = current.entries.last_mut() else {
                return Err(format!("line {lineno}: continuation before any entry"));
            };
            if !cont.is_empty() {
                entry.text.push(' ');
                entry.text.push_str(cont);
            }
        } else if !line.trim().is_empty() {
            return Err(format!(
                "line {lineno}: expected a `## YYYY-MM-DD` header, a `- ` entry, \
                 or a two-space-indented continuation"
            ));
        }
    }
    if let Some(last) = days.last()
        && last.entries.is_empty()
    {
        return Err(format!("day {} has no entries", last.date));
    }
    Ok(days)
}

fn parse_header(header: &str, lineno: usize) -> Result<ChangelogDay, String> {
    let date = NaiveDate::parse_from_str(header.trim(), "%Y-%m-%d")
        .map_err(|e| format!("line {lineno}: day header {:?}: {e}", header.trim()))?;
    Ok(ChangelogDay {
        date,
        entries: Vec::new(),
    })
}

/// Split an optional one-word `Category: ` prefix off an entry.
fn parse_entry(text: &str) -> ChangelogEntry {
    if let Some((word, rest)) = text.split_once(": ")
        && !word.is_empty()
        && word.chars().all(|c| c.is_ascii_alphabetic() || c == '-')
    {
        return ChangelogEntry {
            category: Some(word.to_string()),
            text: rest.trim_start().to_string(),
        };
    }
    ChangelogEntry {
        category: None,
        text: text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use proptest::prelude::*;

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// The gate that keeps a malformed CHANGELOG.md out of a release:
    /// the runtime path degrades to an empty changelog, this test fails
    /// loudly instead.
    #[test]
    fn embedded_changelog_parses() {
        let days = parse(RAW).unwrap();
        assert!(!days.is_empty(), "CHANGELOG.md has no entries");
    }

    #[test]
    fn parses_preamble_categories_and_continuations() {
        let days = parse(
            "# Title\nfreeform preamble - not an entry\n\n\
             ## 2026-09-02\n- Added: a thing\n- plain entry\n  with a continuation\n\n\
             ## 2026-09-01\n- Fixed: another; colons: later stay: put\n",
        )
        .unwrap();
        assert_eq!(days.len(), 2);
        assert_eq!(days[0].date, date("2026-09-02"));
        assert_eq!(
            days[0].entries[0],
            ChangelogEntry {
                category: Some("Added".into()),
                text: "a thing".into()
            }
        );
        assert_eq!(
            days[0].entries[1],
            ChangelogEntry {
                category: None,
                text: "plain entry with a continuation".into()
            }
        );
        assert_eq!(
            days[1].entries[0],
            ChangelogEntry {
                category: Some("Fixed".into()),
                text: "another; colons: later stay: put".into()
            }
        );
    }

    #[test]
    fn multiword_prefix_is_not_a_category() {
        let days = parse("## 2026-09-02\n- not a category: text\n").unwrap();
        assert_eq!(days[0].entries[0].category, None);
        assert_eq!(days[0].entries[0].text, "not a category: text");
    }

    #[test]
    fn rejects_bad_date() {
        assert!(parse("## someday\n- entry\n").is_err());
        assert!(parse("## 2026-13-01\n- entry\n").is_err());
    }

    #[test]
    fn rejects_wrong_order_and_duplicates() {
        assert!(parse("## 2026-09-01\n- a\n## 2026-09-02\n- b\n").is_err());
        assert!(parse("## 2026-09-01\n- a\n## 2026-09-01\n- b\n").is_err());
    }

    #[test]
    fn rejects_empty_day() {
        assert!(parse("## 2026-09-02\n## 2026-09-01\n- a\n").is_err());
        assert!(parse("## 2026-09-02\n- a\n## 2026-09-01\n").is_err());
        assert!(parse("## 2026-09-02\n").is_err());
    }

    #[test]
    fn rejects_stray_lines_after_first_header() {
        assert!(parse("## 2026-09-02\n- a\nstray prose\n").is_err());
    }

    #[test]
    fn seen_marker_round_trips() {
        let marker = SeenMarker {
            date: date("2026-09-02"),
            count: 3,
        };
        assert_eq!(marker.to_string().parse::<SeenMarker>().unwrap(), marker);
        assert!("2026-09-02".parse::<SeenMarker>().is_err());
        assert!("garbage:3".parse::<SeenMarker>().is_err());
    }

    #[test]
    fn unseen_includes_grown_day_partially() {
        let days = parse("## 2026-09-02\n- a\n- b\n- c\n## 2026-09-01\n- d\n").unwrap();
        let marker = Some(SeenMarker {
            date: date("2026-09-02"),
            count: 1,
        });
        let unseen = unseen(&days, marker);
        assert_eq!(unseen.len(), 1);
        assert_eq!(unseen[0].entries.len(), 2);
        assert_eq!(unseen[0].entries[0].text, "b");
    }

    #[test]
    fn unseen_clamps_shrunk_day() {
        let days = parse("## 2026-09-02\n- a\n").unwrap();
        let marker = Some(SeenMarker {
            date: date("2026-09-02"),
            count: 5,
        });
        assert!(unseen(&days, marker).is_empty());
    }

    #[test]
    fn no_marker_means_everything_unseen() {
        let days = parse("## 2026-09-02\n- a\n## 2026-09-01\n- b\n").unwrap();
        assert_eq!(unseen(&days, None), days);
    }

    prop_compose! {
        /// A structurally valid changelog: strictly descending days, each
        /// with 1..4 single-line entries.
        fn valid_days()(
            starts in 1u32..200_000,
            gaps in prop::collection::vec(1u32..40, 0..6),
            counts in prop::collection::vec(1usize..4, 6),
        ) -> Vec<ChangelogDay> {
            let mut days = Vec::new();
            let mut ordinal = starts;
            for (i, gap) in std::iter::once(&0u32).chain(gaps.iter()).enumerate() {
                ordinal += gap;
                let date = NaiveDate::from_num_days_from_ce_opt(ordinal as i32).unwrap();
                days.push(ChangelogDay {
                    date,
                    entries: (0..counts[i % counts.len()])
                        .map(|n| ChangelogEntry { category: None, text: format!("entry {n}") })
                        .collect(),
                });
            }
            days.reverse(); // newest first
            days
        }
    }

    proptest! {
        /// Arbitrary input never panics the parser.
        #[test]
        fn parse_never_panics(input in ".{0,400}") {
            let _ = parse(&input);
        }

        /// A rendered valid changelog parses back to itself — the format
        /// and the parser agree.
        #[test]
        fn valid_changelog_round_trips(days in valid_days()) {
            let mut text = String::from("preamble\n\n");
            for day in &days {
                text.push_str(&format!("## {}\n", day.date.format("%Y-%m-%d")));
                for entry in &day.entries {
                    text.push_str(&format!("- {}\n", entry.text));
                }
            }
            prop_assert_eq!(parse(&text).unwrap(), days);
        }

        /// The marker from `latest_marker` marks everything seen, and
        /// anything older than it stays seen.
        #[test]
        fn latest_marker_sees_all(days in valid_days()) {
            let marker = latest_marker(&days);
            prop_assert!(unseen(&days, marker).is_empty());
        }

        /// `unseen` is monotone: a marker one entry earlier surfaces
        /// exactly that one extra entry first.
        #[test]
        fn unseen_returns_suffix_of_entries(days in valid_days(), day_idx in 0usize..6, count in 0usize..4) {
            let day_idx = day_idx % days.len();
            let m = SeenMarker {
                date: days[day_idx].date,
                count: count.min(days[day_idx].entries.len()),
            };
            let unseen = unseen(&days, Some(m));
            // Everything returned is newer than the marker, or the
            // marker day's tail beyond `count`.
            for day in &unseen {
                prop_assert!(day.date >= m.date);
                if day.date == m.date {
                    let full = &days[day_idx].entries;
                    prop_assert_eq!(&day.entries[..], &full[m.count..]);
                }
            }
            // Days strictly newer than the marker are all present, whole.
            let newer: Vec<_> = days.iter().filter(|d| d.date > m.date).cloned().collect();
            prop_assert_eq!(&unseen[..newer.len()], &newer[..]);
        }
    }
}
