//! Phase 32, end to end: a `/watch` commitment made on one client shows
//! up in another client's List under a "Watching — ⟨user⟩" group. The
//! rigs exchange the exact mutations the `/watch` path emits (the entry
//! upsert and the series-preference write); the grouping itself is the
//! client-side `list_groups` derivation over the converged view.

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::*;
use dessplay::actors::sync::Mutation;
use dessplay::ui::msg::UserAction;
use dessplay::ui::props::{self, ListSort};
use dessplay_core::types::{ListEntryId, ListStatus, SeriesListEntry, SeriesWatchState, UserId};

#[tokio::test(flavor = "multi_thread")]
async fn a_watch_commitment_appears_in_the_other_clients_list() {
    init_test_logging();
    let harness = Harness::new(3201);
    let dir_a = tempfile::tempdir().expect("tempdir");
    let dir_b = tempfile::tempdir().expect("tempdir");

    let rig_a = loop_rig(&harness, "kim", 1, dir_a.path());
    let rig_b = loop_rig(&harness, "nero", 1, dir_b.path());

    // kim `/watch`es a series: the List entry upsert plus her own
    // Watching preference, exactly what the command emits.
    let entry_id = ListEntryId(1);
    rig_a
        .actions
        .send(UserAction::Mutate(Mutation::PutListEntry {
            id: entry_id,
            entry: SeriesListEntry {
                name: "Frieren".into(),
                nero_name: None,
                genre: None,
                notes: vec![],
                recommender: None,
                status: ListStatus::Active,
                status_note: None,
                source: None,
                watchers: Default::default(),
                anidb_series_id: None,
                local_aliases: ["Frieren".to_string()].into_iter().collect(),
                manual_files: Default::default(),
                anidb_unavailable: false,
            },
        }))
        .await
        .expect("loop gone");
    rig_a
        .actions
        .send(UserAction::Mutate(Mutation::SetSeriesPreference {
            user: UserId::new("kim"),
            entry: entry_id,
            pref: SeriesWatchState::Watching,
            set_by: None,
        }))
        .await
        .expect("loop gone");

    // nero's replica converges on both writes.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let view = loop {
        let view = rig_b.view().await;
        if view
            .series_preference
            .get(&(UserId::new("kim"), entry_id))
            .is_some_and(|pref| pref.state == SeriesWatchState::Watching)
            && view.list_entries.contains_key(&entry_id)
        {
            break view;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "kim's commitment never reached nero; entries: {:?}; preferences: {:?}",
            view.list_entries,
            view.series_preference
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    // nero's List, derived exactly as the UI derives it: the entry sits
    // under "Watching — kim", with kim's initial in the users column.
    let groups = props::list_groups(
        &view,
        &UserId::new("nero"),
        &[UserId::new("nero"), UserId::new("kim")],
        ListSort::Recency,
        &BTreeMap::new(),
        &Default::default(),
    );
    let kim = groups
        .iter()
        .find(|group| group.heading == "Watching — kim")
        .unwrap_or_else(|| panic!("no 'Watching — kim' group; got {:?}", groups));
    assert_eq!(kim.rows.len(), 1);
    assert_eq!(kim.rows[0].name, "Frieren");
    assert_eq!(kim.rows[0].watchers, "K");

    rig_a.quit().await;
    rig_b.quit().await;
}
