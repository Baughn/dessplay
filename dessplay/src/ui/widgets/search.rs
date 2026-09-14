//! Local fuzzy matching and the shared search editor/selection state machine.
use super::{ListCursor, TextField, plain};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers, NoUserEvent};

/// Ctrl-F is the distinct legacy control byte 0x06, also understood without
/// an enhanced keyboard protocol. Slash is an alias only outside text fields.
pub fn opens(ev: &Event<NoUserEvent>, slash: bool) -> bool {
    matches!(ev, Event::Keyboard(KeyEvent { code: Key::Char('f'), modifiers })
        if modifiers.contains(KeyModifiers::CONTROL) && !modifiers.contains(KeyModifiers::ALT))
        || (slash && plain(ev) == Some(Key::Char('/')))
}

/// Lower scores sort first. Every ordered whole-word match outranks every
/// ordered substring-word match, which outranks scattered letters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Score(u8, usize, usize, usize);

pub struct Query {
    words: Vec<String>,
    letters: Vec<char>,
}

impl Query {
    pub fn new(text: &str) -> Self {
        let lower = text.to_lowercase();
        Self {
            words: lower.split_whitespace().map(str::to_owned).collect(),
            letters: lower.chars().filter(|c| !c.is_whitespace()).collect(),
        }
    }

    pub fn score(&self, text: &str) -> Option<Score> {
        if self.letters.is_empty() {
            return Some(Score(0, 0, 0, 0));
        }
        let text = text.to_lowercase();
        let mut wanted = self.letters.iter().peekable();
        let mut first = None;
        let mut last = 0;
        for (i, c) in text.chars().enumerate() {
            if wanted.peek().is_some_and(|next| **next == c) {
                first.get_or_insert(i);
                last = i;
                wanted.next();
                if wanted.peek().is_none() {
                    break;
                }
            }
        }
        if wanted.peek().is_some() {
            return None;
        }
        // Try whole words separately: an earlier embedded occurrence must not
        // hide a later whole-word match ("foobar foo" queried with "foo").
        let ordered_words = |whole: bool| {
            let mut rest = text.as_str();
            for word in &self.words {
                let (at, _) = rest.match_indices(word.as_str()).find(|(at, _)| {
                    !whole
                        || (rest[..*at]
                            .chars()
                            .next_back()
                            .is_none_or(|c| !c.is_alphanumeric())
                            && rest[*at + word.len()..]
                                .chars()
                                .next()
                                .is_none_or(|c| !c.is_alphanumeric()))
                })?;
                rest = &rest[at + word.len()..];
            }
            Some(())
        };
        let category = if ordered_words(true).is_some() {
            0
        } else if ordered_words(false).is_some() {
            1
        } else {
            2
        };
        let first = first.unwrap_or(0);
        Some(Score(
            category,
            last + 1 - first - self.letters.len(),
            first,
            text.chars().count(),
        ))
    }
}

pub struct Entry<K> {
    pub key: K,
    pub text: String,
}

pub enum Effect<K> {
    Changed,
    Moved,
    Accept(K),
    Close,
    Consumed,
}

/// Both inline document find and ranked collection pickers use this editor.
/// Keys belong to the source, never to positions in a mutable source list.
pub struct Search<K> {
    pub editor: TextField,
    pub cursor: ListCursor,
    pub entries: Vec<Entry<K>>,
    pub matches: Vec<usize>,
    ranked: bool,
}

impl<K: Clone + PartialEq> Search<K> {
    pub fn new(entries: Vec<Entry<K>>, ranked: bool) -> Self {
        let mut search = Self {
            editor: TextField::new("Search…"),
            cursor: ListCursor::default(),
            entries,
            matches: Vec::new(),
            ranked,
        };
        search.rebuild();
        search
    }

    pub fn selected(&self) -> Option<&Entry<K>> {
        self.matches
            .get(self.cursor.visible_index()?)
            .and_then(|i| self.entries.get(*i))
    }

    pub fn replace(&mut self, entries: Vec<Entry<K>>) {
        let key = self.selected().map(|entry| entry.key.clone());
        self.entries = entries;
        self.rebuild();
        if let Some(index) = key.and_then(|key| {
            self.matches
                .iter()
                .position(|i| self.entries[*i].key == key)
        }) {
            self.cursor.set(index);
        }
    }

    fn rebuild(&mut self) {
        let query = Query::new(&self.editor.text());
        let mut hits: Vec<_> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| query.score(&entry.text).map(|score| (score, i)))
            .collect();
        if self.ranked {
            hits.sort_by_key(|(score, _)| *score);
        }
        self.matches = hits.into_iter().map(|(_, i)| i).collect();
        self.cursor.reset();
        self.cursor.set_hidden(&[]);
        self.cursor.clamp(self.matches.len());
    }

    pub fn on(&mut self, ev: &Event<NoUserEvent>) -> Effect<K> {
        match plain(ev) {
            Some(Key::Esc) => return Effect::Close,
            Some(Key::Enter) => {
                return self
                    .selected()
                    .map_or(Effect::Consumed, |e| Effect::Accept(e.key.clone()));
            }
            Some(key @ (Key::Up | Key::Down | Key::PageUp | Key::PageDown)) => {
                self.cursor.nav(key, self.matches.len());
                return Effect::Moved;
            }
            _ => {}
        }
        let before = self.editor.text();
        self.editor.edit(ev);
        if before != self.editor.text() {
            self.rebuild();
            Effect::Changed
        } else {
            Effect::Consumed
        }
    }

    pub fn keybindings(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("type", "Search"),
            ("↑↓", "Match"),
            ("Enter", "Go to"),
            ("Esc", "Close search"),
        ]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn words_outrank_substrings_which_outrank_scattered_letters() {
        let q = Query::new("foo bar");
        assert!(q.score("foo and bar").unwrap() < q.score("foobar").unwrap());
        assert!(q.score("foobar").unwrap() < q.score("f o o b a r").unwrap());
        assert!(q.score("bar foo").is_none());
        assert_eq!(Query::new("foo").score("foobar foo").unwrap().0, 0);
        assert_eq!(Query::new("猫 犬").score("猫と犬").unwrap().0, 1);
        assert!(Query::new("É猫").score("é—猫").is_some());
    }

    #[test]
    fn editor_navigation_and_empty_results_share_one_controller() {
        let mut search = Search::new(
            vec![
                Entry {
                    key: 1,
                    text: "f o o b a r".into(),
                },
                Entry {
                    key: 2,
                    text: "foo and bar".into(),
                },
            ],
            true,
        );
        assert!(matches!(
            search.on(&Event::Paste("foo bar".into())),
            Effect::Changed
        ));
        assert_eq!(search.selected().unwrap().key, 2);
        let ctrl_a = Event::Keyboard(KeyEvent {
            code: Key::Char('a'),
            modifiers: KeyModifiers::CONTROL,
        });
        assert!(matches!(search.on(&ctrl_a), Effect::Consumed));
        search.on(&Event::Paste("missing".into()));
        assert!(search.selected().is_none());
        assert!(matches!(
            search.on(&Event::Keyboard(KeyEvent {
                code: Key::Enter,
                modifiers: KeyModifiers::NONE
            })),
            Effect::Consumed
        ));
    }

    proptest! {
        #[test]
        fn eligibility_is_exactly_case_insensitive_ordered_letters(text in ".{0,120}", query in ".{0,25}") {
            let lower = text.to_lowercase();
            let mut source = lower.chars();
            let expected = query.to_lowercase().chars().filter(|c| !c.is_whitespace())
                .all(|wanted| source.any(|c| c == wanted));
            prop_assert_eq!(Query::new(&query).score(&text).is_some(), expected);
        }
        #[test]
        fn replacing_source_rows_keeps_the_selected_identity(len in 1usize..100, seed in any::<usize>()) {
            let selected = seed % len;
            let entries = || (0..len).map(|key| Entry { key, text: format!("entry {key}") });
            let mut search = Search::new(entries().collect(), true);
            search.cursor.set(selected);
            search.replace(entries().rev().collect());
            prop_assert_eq!(search.selected().map(|e| e.key), Some(selected));
        }
        #[test]
        fn deleting_characters_always_produces_a_match(text in ".{0,100}") {
            let query: String = text.chars().step_by(2).collect();
            prop_assert!(Query::new(&query).score(&text).is_some());
        }
    }
}
