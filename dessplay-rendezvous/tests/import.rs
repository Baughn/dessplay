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
/// instead of duplicating.
#[tokio::test(start_paused = true)]
async fn import_submits_and_reimport_updates() {
    let harness = Harness::new(0x5EED);
    let kim = harness.client("kim", 1);
    let baughn = harness.client("baughn", 2);

    let report = parse_fixtures();
    let total = report.entries.len();
    assert!(total > 200, "fixtures shrank? {total}");

    let outcome = submit(&kim, &report).await.expect("submit");
    assert_eq!(outcome.created, total);
    assert_eq!(outcome.updated, 0);

    // Everyone sees The List, including next-ep progress.
    let snaps = eventually(&[&kim, &baughn], Duration::from_secs(60), |snaps| {
        snaps.iter().all(|s| s.view.list_entries.len() == total) && snaps[0].view == snaps[1].view
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

    // Re-import: everything matches by name, nothing duplicates.
    let outcome = submit(&kim, &parse_fixtures()).await.expect("re-submit");
    assert_eq!(outcome.created, 0);
    assert_eq!(outcome.updated, total);
    eventually(&[&kim, &baughn], Duration::from_secs(60), |snaps| {
        snaps.iter().all(|s| s.view.list_entries.len() == total)
    })
    .await;
}
