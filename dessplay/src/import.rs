//! The List importer: a one-shot conversion of the group's tracking
//! spreadsheet (exported as CSV, one file per sheet) into
//! [`SeriesListEntry`] rows. See design.md, The List > Import.
//!
//! The data is messy by nature — section headers inline with rows,
//! columns re-purposed per section, drop reasons living in the Genre
//! column — so the importer is deliberately chatty: everything it was
//! unsure about lands in [`ImportReport::warnings`] for manual review.
//! Parsing is pure; submission (ids, CRDT ops) happens in the CLI
//! layer against a live client.
//!
//! Sheet shapes (detected from the header row):
//! - **Active** (has a "Next Ep" column): rows are Active until a
//!   "Current Season" or "Waiting" section header. Carries `NextEpState`
//!   (the ✓/✖ column is `available`).
//! - **Planning** (has a "Recommender" column): "Short List", "General"
//!   (-> Planned), "Refresh / Haitus" (-> Hiatus; a `Progress?` cell in
//!   the header row re-labels that column as the status note).
//! - **Finished** (everything else): Finished until an "Abandoned"
//!   header (-> Dropped). A field matching /abandon|drop/i also marks a
//!   row Dropped (with a warning when that happens outside the
//!   Abandoned section).

use std::collections::BTreeMap;

use dessplay_core::types::{ListStatus, NextEpState, SeriesListEntry, UserId};

/// Maps watcher initials (and, as a fallback, full names) to usernames.
#[derive(Clone, Debug)]
pub struct WatcherMap {
    by_initial: BTreeMap<String, UserId>,
}

impl WatcherMap {
    /// Parse a `B=Baughn,N=Nero,...` spec.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut by_initial = BTreeMap::new();
        for pair in spec.split(',') {
            let (initial, name) = pair
                .split_once('=')
                .ok_or_else(|| format!("bad watcher mapping {pair:?} (want INITIAL=Name)"))?;
            by_initial.insert(initial.trim().to_uppercase(), UserId::new(name.trim()));
        }
        if by_initial.is_empty() {
            return Err("empty watcher mapping".into());
        }
        Ok(Self { by_initial })
    }

    /// Resolve one token from a watchers cell ("B", "kim?", ...).
    fn resolve(&self, token: &str) -> Option<UserId> {
        if let Some(user) = self.by_initial.get(&token.to_uppercase()) {
            return Some(user.clone());
        }
        self.by_initial
            .values()
            .find(|user| user.0.eq_ignore_ascii_case(token))
            .cloned()
    }
}

/// One parsed row, ready for submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedEntry {
    /// The entry, sans id (ids are assigned at submission).
    pub entry: SeriesListEntry,
    /// Progress fields, when the sheet carries them.
    pub next_ep: Option<NextEpState>,
}

/// Everything one or more sheets parsed into.
#[derive(Clone, Debug, Default)]
pub struct ImportReport {
    /// Parsed entries, in sheet order.
    pub entries: Vec<ImportedEntry>,
    /// Everything the importer was unsure about — print this.
    pub warnings: Vec<String>,
}

impl ImportReport {
    /// Counts per status, for the summary printout.
    pub fn status_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for imported in &self.entries {
            *counts
                .entry(status_name(imported.entry.status))
                .or_insert(0) += 1;
        }
        counts
    }
}

fn status_name(status: ListStatus) -> &'static str {
    match status {
        ListStatus::ShortList => "short list",
        ListStatus::Planned => "planned",
        ListStatus::Active => "active",
        ListStatus::CurrentSeason => "current season",
        ListStatus::Waiting => "waiting",
        ListStatus::Hiatus => "hiatus",
        ListStatus::Finished => "finished",
        ListStatus::Dropped => "dropped",
    }
}

/// What kind of sheet a header row announces.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SheetKind {
    Active,
    Planning,
    Finished,
}

/// Column roles, resolved from the header row.
#[derive(Default, Debug)]
struct Columns {
    name: usize,
    nero_name: Option<usize>,
    genre: Option<usize>,
    notes: Vec<usize>,
    recommender: Option<usize>,
    watchers: Option<usize>,
    next_ep: Option<usize>,
    available: Option<usize>,
    source: Option<usize>,
    /// Re-labeled per section ("Progress?" in the Hiatus header).
    status_note: Option<usize>,
}

fn detect(headers: &[String]) -> (SheetKind, Columns) {
    let mut columns = Columns {
        name: 0,
        ..Columns::default()
    };
    for (index, header) in headers.iter().enumerate() {
        let header = header.trim();
        let lower = header.to_lowercase();
        if index == 0 {
            continue; // always the name
        }
        if lower.contains("nero") {
            columns.nero_name = Some(index);
        } else if lower == "genre" {
            columns.genre = Some(index);
        } else if lower.contains("notes") {
            columns.notes.push(index);
        } else if lower == "recommender" {
            columns.recommender = Some(index);
        } else if lower.contains("watchers") {
            columns.watchers = Some(index);
        } else if lower == "next ep" {
            columns.next_ep = Some(index);
            // The unnamed ✓/✖ column sits immediately after.
            columns.available = Some(index + 1);
        } else if lower.starts_with("source") {
            columns.source = Some(index);
        }
    }
    let kind = if columns.next_ep.is_some() {
        SheetKind::Active
    } else if columns.recommender.is_some() {
        SheetKind::Planning
    } else {
        SheetKind::Finished
    };
    (kind, columns)
}

/// Recognize a section header row; returns the status it starts.
fn section_status(name: &str, kind: SheetKind) -> Option<ListStatus> {
    let normalized = name.trim().to_lowercase();
    match normalized.as_str() {
        "short list" => Some(ListStatus::ShortList),
        "general" => Some(ListStatus::Planned),
        "current season" => Some(ListStatus::CurrentSeason),
        "waiting" => Some(ListStatus::Waiting),
        "abandoned" => Some(ListStatus::Dropped),
        _ if normalized.contains("hiatus") || normalized.contains("haitus") => {
            Some(ListStatus::Hiatus)
        }
        // "Refresh" alone also reads as the hiatus section.
        "refresh" => Some(ListStatus::Hiatus),
        _ => {
            let _ = kind;
            None
        }
    }
}

fn cell(record: &[String], index: Option<usize>) -> Option<String> {
    let value = record.get(index?)?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Parse one exported sheet. `label` names the file in warnings.
pub fn import_sheet(
    content: &str,
    label: &str,
    watchers: &WatcherMap,
    report: &mut ImportReport,
) -> Result<(), String> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .flexible(true)
        .from_reader(content.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| format!("{label}: bad header row: {e}"))?
        .iter()
        .map(str::to_string)
        .collect();
    let (kind, mut columns) = detect(&headers);

    let mut status = match kind {
        SheetKind::Active => ListStatus::Active,
        SheetKind::Planning => ListStatus::Planned,
        SheetKind::Finished => ListStatus::Finished,
    };

    for (line, record) in reader.records().enumerate() {
        let line = line + 2; // 1-based, after the header
        let record: Vec<String> = record
            .map_err(|e| format!("{label}:{line}: {e}"))?
            .iter()
            .map(str::to_string)
            .collect();
        if record.iter().all(|cell| cell.trim().is_empty()) {
            continue;
        }
        let name = record.first().map(|s| s.trim()).unwrap_or("");

        // Section header?
        if let Some(new_status) = section_status(name, kind) {
            status = new_status;
            // Cells beyond the name can re-label columns for the
            // section ("Progress?"); anything else is decoration.
            columns.status_note = record
                .iter()
                .position(|cell| cell.trim().eq_ignore_ascii_case("progress?"));
            continue;
        }
        if name.is_empty() {
            report.warnings.push(format!(
                "{label}:{line}: no name; skipped: {:?}",
                record.join(",")
            ));
            continue;
        }

        let mut entry = SeriesListEntry {
            name: name.to_string(),
            nero_name: cell(&record, columns.nero_name),
            genre: cell(&record, columns.genre),
            // A "Progress?" section relabel points status_note at a column
            // that may also be a notes column (e.g. the Ivory sheet's "Extra
            // Notes"). Read it only as the status note, never duplicated into
            // notes. Excluding here (rather than mutating columns.notes) auto-
            // restores the notes role once a later section clears status_note.
            notes: columns
                .notes
                .iter()
                .filter(|&&index| Some(index) != columns.status_note)
                .filter_map(|&index| cell(&record, Some(index)))
                .collect(),
            recommender: cell(&record, columns.recommender),
            status,
            status_note: cell(&record, columns.status_note),
            source: cell(&record, columns.source),
            watchers: Default::default(),
            anidb_series_id: None,
            local_aliases: Default::default(),
            manual_files: Default::default(),
            anidb_unavailable: false,
        };

        // Watchers.
        if let Some(raw) = cell(&record, columns.watchers) {
            for token in raw.split('/') {
                let token = token.trim();
                if token.is_empty() {
                    continue;
                }
                let uncertain = token.ends_with('?');
                let bare = token.trim_end_matches('?').trim();
                match watchers.resolve(bare) {
                    Some(user) => {
                        if uncertain {
                            report.warnings.push(format!(
                                "{label}:{line}: {name}: uncertain watcher {token:?} (included)"
                            ));
                        }
                        entry.watchers.insert(user);
                    }
                    None => report.warnings.push(format!(
                        "{label}:{line}: {name}: unknown watcher token {token:?}"
                    )),
                }
            }
        }

        // Drop detection: the Genre column doubles as the drop reason
        // on the finished sheet ("Abandoned after 4 eps", "3 (drop)").
        if kind == SheetKind::Finished {
            let marked = entry.genre.as_deref().is_some_and(is_drop_marker)
                || entry.notes.iter().any(|note| is_drop_marker(note));
            if marked {
                if entry.status != ListStatus::Dropped {
                    report.warnings.push(format!(
                        "{label}:{line}: {name}: drop marker outside the Abandoned section; \
                         marked Dropped"
                    ));
                    entry.status = ListStatus::Dropped;
                }
                if let Some(genre) = entry.genre.take_if(|g| is_drop_marker(g)) {
                    entry.status_note = Some(genre);
                }
            } else if entry.status == ListStatus::Dropped && entry.status_note.is_none() {
                // Abandoned-section row without an explicit reason: the
                // genre column usually holds the progress ("3", "S3 Ep2").
                entry.status_note = entry.genre.take();
            }
        }

        // Progress fields (active sheet only).
        let next_ep = if kind == SheetKind::Active {
            let next = cell(&record, columns.next_ep);
            let available = cell(&record, columns.available)
                .map(|mark| match mark.as_str() {
                    "✓" => Ok(true),
                    "✖" => Ok(false),
                    other => Err(other.to_string()),
                })
                .transpose()
                .unwrap_or_else(|other| {
                    report.warnings.push(format!(
                        "{label}:{line}: {name}: unrecognized availability mark {other:?}"
                    ));
                    None
                })
                .unwrap_or(false);
            next.map(|next_ep| NextEpState {
                next_ep: Some(next_ep),
                available,
            })
        } else {
            None
        };

        // Anything in columns we never mapped is data we'd silently
        // lose — surface it.
        let known: Vec<usize> = [
            Some(columns.name),
            columns.nero_name,
            columns.genre,
            columns.recommender,
            columns.watchers,
            columns.next_ep,
            columns.available,
            columns.source,
            columns.status_note,
        ]
        .into_iter()
        .flatten()
        .chain(columns.notes.iter().copied())
        .collect();
        for (index, value) in record.iter().enumerate() {
            if !value.trim().is_empty() && !known.contains(&index) {
                report.warnings.push(format!(
                    "{label}:{line}: {name}: unmapped column {index} contains {value:?}"
                ));
            }
        }

        report.entries.push(ImportedEntry { entry, next_ep });
    }
    Ok(())
}

/// What [`submit`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportOutcome {
    /// Entries created fresh.
    pub created: usize,
    /// Existing entries (matched by name, case-insensitive) updated in
    /// place — re-imports don't duplicate The List.
    pub updated: usize,
    /// Series that appeared on more than one imported sheet and were
    /// collapsed onto a single entry (the later row overwrote the earlier).
    /// Surfaced so the user can reconcile a status conflict between sheets.
    pub collapsed: Vec<String>,
}

/// Submit a parsed report through a connected client. Existing entries
/// are matched by name (case-insensitive) and updated in place,
/// preserving a manually-set AniDB link when the import has none;
/// `next_ep` is only written when the sheet carried one.
pub async fn submit(
    handle: &crate::client::ClientHandle,
    report: &ImportReport,
) -> Result<ImportOutcome, String> {
    use crate::actors::sync::{Mutation, SyncCommand};
    use dessplay_core::types::{AniDbSeriesId, ListEntryId};
    use std::time::Duration;

    let view = async |handle: &crate::client::ClientHandle| {
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle
            .sync
            .send(SyncCommand::GetView(tx))
            .await
            .map_err(|_| "sync actor gone".to_string())?;
        rx.await.map_err(|_| "sync actor gone".to_string())
    };

    // Wait for the initial sync (epoch leaves 0 when the server's
    // snapshot or merge lands).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let (tx, rx) = tokio::sync::oneshot::channel();
        handle
            .sync
            .send(SyncCommand::GetEpoch(tx))
            .await
            .map_err(|_| "sync actor gone".to_string())?;
        if rx.await.map_err(|_| "sync actor gone".to_string())?.0 > 0 {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("timed out waiting for the initial state sync".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let existing = view(handle).await?;
    // Name (case-insensitive) -> (entry id, its AniDB link, has-an-earlier-
    // row-in-this-run-already-matched-it). Seeded from the current List so
    // a re-import matches existing entries, and *grown as we go* so a
    // series named on more than one sheet collapses onto one entry instead
    // of creating a duplicate the frozen snapshot could never see (the
    // bug: matching only the pre-loop snapshot left each same-named row to
    // mint its own random id).
    //
    // The third field tracks "matched by an earlier row in *this* run", not
    // "created in this run" -- collapsing is a status conflict between two
    // *imported* sheets, and that applies whether the entry already existed
    // in The List or was just created a moment ago by an earlier row of
    // this same import. Gating on "created this run" instead (the prior
    // bug) silently resolved a conflict between two sheets whenever the
    // entry pre-existed -- which re-imports, the primary supported
    // workflow, hit on every single row.
    let mut seen: std::collections::HashMap<String, (ListEntryId, Option<AniDbSeriesId>, bool)> =
        std::collections::HashMap::new();
    for (id, entry) in &existing.list_entries {
        seen.entry(entry.name.to_ascii_lowercase())
            .or_insert((*id, entry.anidb_series_id, false));
    }

    let mut outcome = ImportOutcome::default();
    let mut expected: Vec<(ListEntryId, String)> = Vec::new();
    for imported in &report.entries {
        let mut entry = imported.entry.clone();
        let key = entry.name.to_ascii_lowercase();
        let id = match seen.get(&key).copied() {
            Some((id, old_anidb, matched_this_run)) => {
                // A manually-set AniDB link outlives re-imports (imports
                // arrive unlinked, so `old_anidb` only ever comes from an
                // existing entry).
                entry.anidb_series_id = entry.anidb_series_id.or(old_anidb);
                outcome.updated += 1;
                if matched_this_run {
                    // Two sheets named the same series: the later row wins.
                    outcome.collapsed.push(entry.name.clone());
                }
                seen.insert(key, (id, old_anidb, true));
                id
            }
            None => {
                let id = ListEntryId::from_bytes(rand::random());
                seen.insert(key, (id, entry.anidb_series_id, true));
                outcome.created += 1;
                id
            }
        };
        expected.push((id, entry.name.clone()));
        handle
            .sync
            .send(SyncCommand::Mutate(Box::new(Mutation::PutListEntry {
                id,
                entry,
            })))
            .await
            .map_err(|_| "sync actor gone".to_string())?;
        if let Some(next_ep) = &imported.next_ep {
            handle
                .sync
                .send(SyncCommand::Mutate(Box::new(Mutation::SetNextEp {
                    id,
                    next_ep: next_ep.clone(),
                })))
                .await
                .map_err(|_| "sync actor gone".to_string())?;
        }
    }

    // Wait until the local view holds everything (the upward path to
    // the server is the reliable control stream; local application is
    // what we can observe here).
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let now = view(handle).await?;
        if expected
            .iter()
            .all(|(id, _)| now.list_entries.contains_key(id))
        {
            return Ok(outcome);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("timed out waiting for entries to apply".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Case-insensitive "abandon"/"drop" match on the finished/dropped sheet's
/// genre/notes column (design.md: `/abandon|drop/i`). A full regex isn't
/// worth the dependency, so the terms are matched directly — no decoy
/// pattern string that could silently drift from what is matched.
fn is_drop_marker(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("abandon") || lower.contains("drop")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use dessplay_core::types::ListStatus;

    fn real_map() -> WatcherMap {
        WatcherMap::parse("B=Baughn,N=Nero,Q=Quickshot,D=Dagger,K=Kim").unwrap()
    }

    fn import_fixture(file: &str) -> ImportReport {
        let path = format!("{}/../spreadsheet/{file}", env!("CARGO_MANIFEST_DIR"));
        let content = std::fs::read_to_string(&path).unwrap();
        let mut report = ImportReport::default();
        import_sheet(&content, file, &real_map(), &mut report).unwrap();
        report
    }

    fn find<'a>(report: &'a ImportReport, name: &str) -> &'a ImportedEntry {
        report
            .entries
            .iter()
            .find(|imported| imported.entry.name == name)
            .unwrap_or_else(|| panic!("no entry named {name:?}"))
    }

    fn users(names: &[&str]) -> std::collections::BTreeSet<UserId> {
        names.iter().map(|name| UserId::new(*name)).collect()
    }

    #[test]
    fn active_sheet_sections_and_progress() {
        let report = import_fixture("Things-to-watch, The List - Passing.csv");

        // Pre-header rows are Active.
        let lain = find(&report, "Serial Experiments Lain");
        assert_eq!(lain.entry.status, ListStatus::Active);
        assert_eq!(
            lain.entry.watchers,
            users(&["Baughn", "Quickshot", "Dagger", "Nero"])
        );
        assert_eq!(
            lain.next_ep,
            Some(NextEpState {
                next_ep: Some("1".into()),
                available: false,
            })
        );

        // Current Season rows, with the ✓ column and source.
        let doulou = find(&report, "Doulou Dalu 2");
        assert_eq!(doulou.entry.status, ListStatus::CurrentSeason);
        assert_eq!(doulou.entry.source.as_deref(), Some("Nyaa"));
        assert_eq!(
            doulou.next_ep,
            Some(NextEpState {
                next_ep: Some("140".into()),
                available: true,
            })
        );
        let gochiusa = find(&report, "GochiUsa");
        assert_eq!(
            gochiusa.next_ep,
            Some(NextEpState {
                next_ep: Some("Sisters".into()),
                available: false,
            })
        );

        // Waiting rows: free-text next_ep survives.
        let bunny = find(&report, "Seishun Buta Yarou wa Bunny Girl");
        assert_eq!(bunny.entry.status, ListStatus::Waiting);
        assert_eq!(
            bunny.next_ep.as_ref().unwrap().next_ep.as_deref(),
            Some("2026-Movie")
        );
    }

    #[test]
    fn planning_sheet_sections() {
        let report = import_fixture("Things-to-watch, The List - Ivory.csv");

        let steins = find(&report, "Steins;Gate");
        assert_eq!(steins.entry.status, ListStatus::ShortList);
        assert_eq!(steins.entry.recommender.as_deref(), Some("Quickshot"));

        let katanagatari = find(&report, "Katanagatari");
        assert_eq!(katanagatari.entry.status, ListStatus::Planned);
        assert_eq!(katanagatari.entry.recommender.as_deref(), Some("Nero"));

        // Hiatus rows pick up the re-labeled Progress? column.
        let shirobako = find(&report, "Shirobako");
        assert_eq!(shirobako.entry.status, ListStatus::Hiatus);
        assert_eq!(shirobako.entry.status_note.as_deref(), Some("13"));
        // The Progress? relabel *re-labels* the column as the status note;
        // the value must not also land in `notes` (no stray duplicate).
        assert!(
            shirobako.entry.notes.is_empty(),
            "Progress? value duplicated into notes: {:?}",
            shirobako.entry.notes
        );

        // The nameless short-list slot is reported, not silently lost.
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("no name") && w.contains("Dagger")),
            "{:#?}",
            report.warnings
        );
        // "kim?" resolves by full name, flagged uncertain.
        let starship = find(&report, "Starship Operators");
        assert_eq!(starship.entry.watchers, users(&["Kim"]));
        assert!(report.warnings.iter().any(|w| w.contains("kim?")));
    }

    #[test]
    fn finished_sheet_drops() {
        let report = import_fixture("Things-to-watch, The List - Ebony.csv");

        let symphogear = find(&report, "Symphogear");
        assert_eq!(symphogear.entry.status, ListStatus::Finished);
        assert_eq!(symphogear.entry.watchers, users(&["Baughn", "Nero"]));

        // Abandoned section: Dropped, reason rescued from Genre.
        let kuma = find(&report, "Kuma Kuma Kuma Bear");
        assert_eq!(kuma.entry.status, ListStatus::Dropped);
        assert_eq!(
            kuma.entry.status_note.as_deref(),
            Some("Abandoned after 4 eps")
        );
        assert_eq!(kuma.entry.notes, vec!["Ruined by adaption".to_string()]);

        // Abandoned-section row with no drop marker still drops.
        let vividred = find(&report, "Vividred");
        assert_eq!(vividred.entry.status, ListStatus::Dropped);

        // Mew Mew's "3 (drop)" genre becomes the status note.
        let mew = find(&report, "Tokyo Mew Mew New");
        assert_eq!(mew.entry.status, ListStatus::Dropped);
        assert_eq!(mew.entry.status_note.as_deref(), Some("3 (drop)"));
        assert!(mew.entry.genre.is_none());
    }

    #[test]
    fn full_import_summary() {
        let mut report = ImportReport::default();
        for file in [
            "Things-to-watch, The List - Passing.csv",
            "Things-to-watch, The List - Ivory.csv",
            "Things-to-watch, The List - Ebony.csv",
        ] {
            let path = format!("{}/../spreadsheet/{file}", env!("CARGO_MANIFEST_DIR"));
            let content = std::fs::read_to_string(&path).unwrap();
            import_sheet(&content, file, &real_map(), &mut report).unwrap();
        }
        let counts = report.status_counts();
        // Sanity bounds from today's export — the exact numbers will
        // drift with the sheets; what matters is every section parsed.
        assert!(counts["active"] >= 5, "{counts:?}");
        assert!(counts["current season"] >= 8, "{counts:?}");
        assert!(counts["waiting"] >= 3, "{counts:?}");
        assert!(counts["short list"] >= 3, "{counts:?}");
        assert!(counts["planned"] >= 10, "{counts:?}");
        assert!(counts["hiatus"] >= 5, "{counts:?}");
        assert!(counts["finished"] >= 100, "{counts:?}");
        assert!(counts["dropped"] >= 15, "{counts:?}");
        // No entry was invented or lost relative to the row count.
        assert!(report.entries.len() >= 180, "{}", report.entries.len());
    }

    #[test]
    fn watcher_map_rejects_garbage() {
        assert!(WatcherMap::parse("").is_err());
        assert!(WatcherMap::parse("BBaughn").is_err());
        let map = WatcherMap::parse("B=Baughn").unwrap();
        assert_eq!(map.resolve("b"), Some(UserId::new("Baughn")));
        assert_eq!(map.resolve("baughn"), Some(UserId::new("Baughn")));
        assert_eq!(map.resolve("X"), None);
    }
}
