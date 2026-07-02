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
    // duplicates, and no fresh same-run collapse is reported (the duplicate
    // rows now match the already-present entry, not one created this run).
    let outcome = submit(&kim, &parse_fixtures()).await.expect("re-submit");
    assert_eq!(outcome.created, 0);
    assert_eq!(outcome.updated, total);
    assert!(outcome.collapsed.is_empty());
    eventually(&[&kim, &baughn], Duration::from_secs(60), |snaps| {
        snaps.iter().all(|s| s.view.list_entries.len() == unique)
    })
    .await;
}
