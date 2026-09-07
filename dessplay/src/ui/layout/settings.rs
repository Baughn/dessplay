use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::LayoutBundle;

/// Local drag state. Each source retains only its latest successful revision.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutSettings {
    sources: BTreeMap<String, SavedLayout>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SavedLayout {
    revision: String,
    splits: BTreeMap<String, BTreeMap<String, u16>>,
}

impl LayoutSettings {
    pub(crate) fn source(bundle: &LayoutBundle) -> String {
        bundle
            .source
            .as_ref()
            .map_or_else(|| "builtin".into(), |p| p.to_string_lossy().into_owned())
    }

    /// Invalidate old drag sizes before the first frame of a new revision.
    pub(crate) fn activate(
        &mut self,
        bundle: &LayoutBundle,
        legacy: crate::config::PaneLayout,
    ) -> bool {
        let source = Self::source(bundle);
        if self
            .sources
            .get(&source)
            .is_some_and(|s| s.revision == bundle.revision)
        {
            return false;
        }
        let mut splits = BTreeMap::new();
        if self.sources.is_empty() && bundle.source.is_none() {
            let legacy = legacy.clamped();
            for (id, children) in [
                (
                    "panes",
                    vec![
                        ("chat-column", legacy.chat_width),
                        ("right-column", 100 - legacy.chat_width),
                    ],
                ),
                (
                    "chat-column",
                    vec![
                        ("chat", 100 - legacy.subtitle_height),
                        ("subtitles", legacy.subtitle_height),
                    ],
                ),
                (
                    "right-column",
                    vec![
                        ("series", legacy.series_height),
                        ("users", legacy.users_height),
                        ("playlist", legacy.playlist_height()),
                    ],
                ),
            ] {
                splits.insert(
                    id.into(),
                    children
                        .into_iter()
                        .map(|(id, n)| (id.into(), u16::from(n) * 100))
                        .collect(),
                );
            }
        }
        self.sources.insert(
            source,
            SavedLayout {
                revision: bundle.revision.clone(),
                splits,
            },
        );
        true
    }

    pub(crate) fn shares(&self, source: &str) -> BTreeMap<String, u16> {
        self.sources
            .get(source)
            .into_iter()
            .flat_map(|s| s.splits.values())
            .flat_map(|children| children.iter().map(|(id, n)| (id.clone(), *n)))
            .collect()
    }

    pub(crate) fn set(&mut self, source: &str, split: &str, shares: BTreeMap<String, u16>) {
        if let Some(saved) = self.sources.get_mut(source) {
            saved.splits.insert(split.into(), shares);
        }
    }

    pub(crate) fn reset(&mut self, source: &str) {
        if let Some(saved) = self.sources.get_mut(source) {
            saved.splits.clear();
        }
    }

    pub(crate) fn load(storage: &crate::storage::Storage) -> crate::storage::Result<Self> {
        storage.setting("layout_sizes")?.map_or_else(
            || Ok(Self::default()),
            |text| {
                let value: Self = serde_json::from_str(&text).map_err(|e| {
                    crate::storage::StorageError::Corrupt(format!("layout sizes: {e}"))
                })?;
                if value.sources.values().any(|source| {
                    source.splits.values().any(|children| {
                        children.values().map(|n| u32::from(*n)).sum::<u32>() != 10_000
                    })
                }) {
                    return Err(crate::storage::StorageError::Corrupt(
                        "layout shares must sum to 10000".into(),
                    ));
                }
                Ok(value)
            },
        )
    }

    pub(crate) fn save(&self, storage: &crate::storage::Storage) -> crate::storage::Result<()> {
        let text = serde_json::to_string(self)
            .map_err(|e| crate::storage::StorageError::Corrupt(format!("layout sizes: {e}")))?;
        storage.set_setting("layout_sizes", Some(&text))
    }
}
