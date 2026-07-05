//! Phase 6: the List importer submitting through a real client against
//! the real server (sim transport, real fixture data).

mod common;

use std::time::Duration;

use common::*;
use dessplay::import::{ImportReport, WatcherMap, import_sheet, submit};

fn parse_fixtures() -> ImportReport {
    let map = WatcherMap::parse("B=Baughn,N=Nero,Q=Quickshot,D=Dagger,K=Kim").expect("watcher map");
    let mut report = ImportReport::default();
    for file in [
        "Things-to-watch, The List - Passing.csv",
        "Things-to-watch, The List - Ivory.csv",
        "Things-to-watch, The List - Ebony.csv",
    ] {
        let path = format!("{}/../spreadsheet/{file}", env!("CARGO_MANIFEST_DIR"));
        let content = std::fs::read_to_string(&path).expect("fixture");
        import_sheet(&content, file, &map, &mut report).expect("parse");
    }
    report
}

/// The full spreadsheet lands, replicates, and a re-import updates
/// instead of duplicating -- including a series that appears on more than
/// one sheet, which must collapse onto a single entry rather than mint a
/// duplicate the pre-loop snapshot could never match.
#[tokio::test(start_paused = true)]
async fn import_submits_and_reimport_updates() {
    let harness = Harness::new(0x5EED);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);

    let report = parse_fixtures();
    let total = report.entries.len();
    assert!(total > 200, "fixtures shrank? {total}");
    // The real fixtures name some series on two sheets (e.g. Steins;Gate,
    // Dr. Stone). Those rows must collapse to one entry per distinct name.
    let unique = report
        .entries
        .iter()
        .map(|e| e.entry.name.to_ascii_lowercase())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let dups = total - unique;
    assert!(
        dups > 0,
        "fixtures should contain cross-sheet duplicate names ({total} rows, {unique} unique)"
    );

    let outcome = submit(&kim, &report).await.expect("submit");
    assert_eq!(outcome.created, unique, "one entry per distinct name");
    assert_eq!(outcome.updated, dups, "duplicate-named rows collapse");
    assert_eq!(
        outcome.collapsed.len(),
        dups,
        "each same-run collapse is surfaced"
    );

    // Everyone sees The List (one entry per distinct name), incl. next-ep.
    let snaps = eventually(&[&kim, &baughn], Duration::from_secs(60), |snaps| {
        snaps.iter().all(|s| s.view.list_entries.len() == unique) && snaps[0].view == snaps[1].view
    })
    .await;
    let view = &snaps[0].view;
    assert!(
        view.list_entries
            .values()
            .any(|entry| entry.name == "Doulou Dalu 2")
    );
    let gochiusa_id = view
        .list_entries
        .iter()
        .find(|(_, entry)| entry.name == "GochiUsa")
        .map(|(id, _)| *id)
        .expect("GochiUsa imported");
    assert_eq!(
        view.list_next_ep
            .get(&gochiusa_id)
            .and_then(|next| next.next_ep.as_deref()),
        Some("Sisters")
    );

    // Re-import: every row matches an existing entry by name, nothing
    // duplicates -- but the cross-sheet duplicate names still collapse onto
    // one entry each *within this run*, and that conflict is still
    // surfaced even though every entry pre-existed (collapsing is a status
    // conflict between two imported sheets, not a "was it just created"
    // question -- a re-import, the primary supported workflow, must not
    // silently swallow it).
    let outcome = submit(&kim, &parse_fixtures()).await.expect("re-submit");
    assert_eq!(outcome.created, 0);
    assert_eq!(outcome.updated, total);
    assert_eq!(
        outcome.collapsed.len(),
        dups,
        "a re-import must still surface cross-sheet duplicates, not just first-time imports"
    );
    eventually(&[&kim, &baughn], Duration::from_secs(60), |snaps| {
        snaps.iter().all(|s| s.view.list_entries.len() == unique)
    })
    .await;
}

/// Regression (2026-07-05 review): the app-owned identity/enrichment
/// fields — `local_aliases`, `manual_files`, `anidb_unavailable` — have
/// no CSV representation (design.md, Series Identity: grown in-app after
/// import), so a re-import matching the entry by name must carry them
/// over, exactly as it already carries a manually-set AniDB link. The
/// update branch used to write the freshly-parsed row wholesale, silently
/// wiping in-app enrichment on the primary supported workflow.
#[tokio::test(start_paused = true)]
async fn reimport_preserves_app_owned_identity_fields() {
    use dessplay::actors::sync::Mutation;
    use dessplay_core::types::Ed2kHash;

    let harness = Harness::new(0x5EED);
    let kim = harness.client("kim", 1);

    let report = parse_fixtures();
    submit(&kim, &report).await.expect("submit");

    // Enrich GochiUsa in-app: an alias, a manual file, and a failed
    // AniDB search.
    let snaps = eventually(&[&kim], Duration::from_secs(60), |snaps| {
        snaps[0]
            .view
            .list_entries
            .values()
            .any(|entry| entry.name == "GochiUsa")
    })
    .await;
    let (id, mut enriched) = snaps[0]
        .view
        .list_entries
        .iter()
        .find(|(_, entry)| entry.name == "GochiUsa")
        .map(|(id, entry)| (*id, entry.clone()))
        .expect("GochiUsa imported");
    enriched
        .local_aliases
        .insert("Gochuumon wa Usagi Desu ka".into());
    enriched.manual_files.insert(Ed2kHash([7; 16]));
    enriched.anidb_unavailable = true;
    mutate(
        &kim,
        Mutation::PutListEntry {
            id,
            entry: enriched,
        },
    )
    .await;
    eventually(&[&kim], Duration::from_secs(60), |snaps| {
        snaps[0]
            .view
            .list_entries
            .get(&id)
            .is_some_and(|entry| entry.anidb_unavailable)
    })
    .await;

    // Re-import the unchanged sheet; the enrichment must survive.
    let outcome = submit(&kim, &parse_fixtures()).await.expect("re-submit");
    assert_eq!(outcome.created, 0, "re-import must match, not duplicate");
    // `submit` sends every PutListEntry and its final GetView poll down
    // the same sync-actor channel, so by the time it returns the local
    // view reflects the re-import — this snapshot observes post-import
    // values, not the pre-import ones.
    let snaps = eventually(&[&kim], Duration::from_secs(60), |snaps| {
        snaps[0].view.list_entries.contains_key(&id)
    })
    .await;
    let entry = &snaps[0].view.list_entries[&id];
    assert!(
        entry.local_aliases.contains("Gochuumon wa Usagi Desu ka"),
        "re-import wiped local_aliases: {:?}",
        entry.local_aliases
    );
    assert!(
        entry.manual_files.contains(&Ed2kHash([7; 16])),
        "re-import wiped manual_files"
    );
    assert!(entry.anidb_unavailable, "re-import reset anidb_unavailable");
}
