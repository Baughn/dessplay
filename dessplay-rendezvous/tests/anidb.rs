//! AniDB integration scenarios: replicated lookup requests flowing
//! through the real server (sim transport) and its worker into
//! replicated metadata/relations, name search over the wire, and the
//! EOF List advance. The API itself is canned — no test touches the
//! real AniDB.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use common::{Harness, eventually_views, hash, mutate, report_eof, view_of};
use dessplay::actors::sync::Mutation;
use dessplay_core::types::{
    AniDbSeriesId, FileHashInfo, ListEntryId, ListStatus, MetadataSource, NextEpState,
    SeriesListEntry,
};
use dessplay_rendezvous::anidb::client::{AniDbApi, BoxFuture, LookupError};
use dessplay_rendezvous::anidb::protocol::{AnimeResult, FileResult};
use dessplay_rendezvous::anidb::titles::TitlesSource;
use dessplay_rendezvous::server::{AniDbConfig, ServerConfig};
use dessplay_rendezvous::storage::ServerStorage;

/// Canned API tables.
#[derive(Default)]
struct CannedApi {
    files: Mutex<HashMap<dessplay_core::types::Ed2kHash, FileResult>>,
    anime: Mutex<HashMap<AniDbSeriesId, AnimeResult>>,
}

impl AniDbApi for CannedApi {
    fn file_by_hash(
        &self,
        _size: u64,
        hash: dessplay_core::types::Ed2kHash,
    ) -> BoxFuture<'_, Result<Option<FileResult>, LookupError>> {
        let result = self.files.lock().unwrap().get(&hash).cloned();
        Box::pin(async move { Ok(result) })
    }

    fn anime_by_id(
        &self,
        aid: AniDbSeriesId,
    ) -> BoxFuture<'_, Result<Option<AnimeResult>, LookupError>> {
        let result = self.anime.lock().unwrap().get(&aid).cloned();
        Box::pin(async move { Ok(result) })
    }
}

struct CannedTitles(&'static str);

impl TitlesSource for CannedTitles {
    fn fetch(&self) -> std::io::Result<String> {
        Ok(self.0.to_string())
    }
}

const FRIEREN: AniDbSeriesId = AniDbSeriesId(8692);

fn frieren_harness(seed: u64) -> Harness {
    let api = CannedApi::default();
    api.files.lock().unwrap().insert(
        hash(1),
        FileResult {
            fid: 1,
            aid: FRIEREN,
            romaji: "Sousou no Frieren".into(),
            english: "Frieren: Beyond Journey's End".into(),
            epno: "01".into(),
        },
    );
    api.anime.lock().unwrap().insert(
        FRIEREN,
        AnimeResult {
            aid: FRIEREN,
            year: Some(2023),
            relations: vec![],
            romaji: "Sousou no Frieren".into(),
            english: String::new(),
            episode_count: Some(28),
        },
    );
    let mut config = ServerConfig::new(common::PASSWORD);
    config.anidb = Some(AniDbConfig {
        api: Arc::new(api),
        titles: Arc::new(CannedTitles(
            "8692|1|x-jat|Sousou no Frieren\n8692|3|en|Frieren\n5391|1|x-jat|Gochuumon wa Usagi Desu ka?\n5391|3|en|GochiUsa\n",
        )),
    });
    Harness::with_config_and_storage(seed, config, Some(ServerStorage::open_in_memory().unwrap()))
}

fn lookup_request(i: u8, filename: &str) -> Mutation {
    Mutation::RequestLookup {
        info: FileHashInfo {
            hash: hash(i),
            size: 1_000_000,
            filename: filename.into(),
            mtime: None,
            series_hint: None,
        },
    }
}

#[tokio::test(start_paused = true)]
async fn lookup_requests_become_replicated_metadata_and_relations() {
    let harness = frieren_harness(801);
    let kim = harness.client("kim", 1);
    let nero = harness.client("nero", 2);

    mutate(
        &kim,
        lookup_request(1, "[SubsPlease] Sousou no Frieren - 01.mkv"),
    )
    .await;

    // Both clients converge on server-written metadata and relations.
    eventually_views(
        &[&kim, &nero],
        std::time::Duration::from_secs(120),
        |views| {
            views.iter().all(|view| {
                let meta = view.anidb_metadata.get(&hash(1));
                meta.is_some_and(|m| {
                    m.as_ref().is_some_and(|m| {
                        m.source == MetadataSource::AniDb
                            && m.series_id == Some(FRIEREN)
                            && m.episode_number.as_deref() == Some("01")
                    })
                }) && view
                    .series_relations
                    .get(&FRIEREN)
                    .is_some_and(|r| r.title == "Sousou no Frieren" && r.year == Some(2023))
            })
        },
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn unknown_files_get_filename_derived_metadata() {
    let harness = frieren_harness(802);
    let kim = harness.client("kim", 1);
    mutate(&kim, lookup_request(9, "Totally Obscure Show - 03.mkv")).await;
    eventually_views(&[&kim], std::time::Duration::from_secs(120), |views| {
        views[0].anidb_metadata.get(&hash(9)).is_some_and(|m| {
            m.as_ref().is_some_and(|m| {
                m.source == MetadataSource::FilenameDerived
                    && m.series_name == "Totally Obscure Show - 03"
                    && m.series_id.is_none()
            })
        })
    })
    .await;
}

#[tokio::test(start_paused = true)]
async fn eof_advances_a_linked_list_entry() {
    let harness = frieren_harness(803);
    let kim = harness.client("kim", 1);
    // Nero only exists to observe convergence: a mutation he can see
    // has definitely reached the server.
    let nero = harness.client("nero", 2);

    // Playlist entry + lookup, so the file has linked metadata.
    mutate(
        &kim,
        Mutation::PushPlaylist {
            new: common::entry(1),
        },
    )
    .await;
    mutate(
        &kim,
        lookup_request(1, "[SubsPlease] Sousou no Frieren - 01.mkv"),
    )
    .await;
    eventually_views(&[&kim], std::time::Duration::from_secs(120), |views| {
        views[0]
            .anidb_metadata
            .get(&hash(1))
            .is_some_and(|m| m.is_some())
    })
    .await;

    // A linked List entry sitting on episode 1, plus an unlinked decoy.
    let id = ListEntryId(42);
    let entry = SeriesListEntry {
        name: "Frieren".into(),
        nero_name: None,
        genre: None,
        notes: vec![],
        recommender: None,
        status: ListStatus::Active,
        status_note: None,
        source: None,
        watchers: Default::default(),
        anidb_series_id: Some(FRIEREN),
    };
    mutate(
        &kim,
        Mutation::PutListEntry {
            id,
            entry: entry.clone(),
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNextEp {
            id,
            next_ep: NextEpState {
                next_ep: Some("1".into()),
                available: true,
            },
        },
    )
    .await;
    let decoy = ListEntryId(43);
    mutate(
        &kim,
        Mutation::PutListEntry {
            id: decoy,
            entry: SeriesListEntry {
                anidb_series_id: None,
                ..entry
            },
        },
    )
    .await;
    mutate(
        &kim,
        Mutation::SetNextEp {
            id: decoy,
            next_ep: NextEpState {
                next_ep: Some("1".into()),
                available: true,
            },
        },
    )
    .await;

    // Play the file to EOF. EofReached bypasses the sync actor, so
    // wait for the now-playing write to round-trip before reporting —
    // otherwise the report can overtake it and read as stale.
    mutate(
        &kim,
        Mutation::SetNowPlaying {
            file: Some(hash(1)),
        },
    )
    .await;
    eventually_views(&[&nero], std::time::Duration::from_secs(60), |views| {
        views[0].now_playing == Some(hash(1)) && views[0].list_next_ep.len() == 2
    })
    .await;
    report_eof(&kim, hash(1)).await;

    eventually_views(&[&kim], std::time::Duration::from_secs(120), |views| {
        views[0]
            .list_next_ep
            .get(&id)
            .is_some_and(|n| n.next_ep.as_deref() == Some("2") && !n.available)
    })
    .await;
    // The unlinked decoy is untouched, and the file is marked watched.
    let view = view_of(&kim).await;
    assert_eq!(
        view.list_next_ep[&decoy].next_ep.as_deref(),
        Some("1"),
        "unlinked entries must not advance"
    );
    assert_eq!(view.watched.get(&hash(1)), Some(&true));
}

#[tokio::test(start_paused = true)]
async fn name_search_answers_over_the_wire() {
    use dessplay::actors::network::{NetworkCommand, NetworkEvent};
    use dessplay::client::ClientEvent;
    use dessplay_core::net::ServerControl;

    let harness = frieren_harness(804);
    let mut kim = harness.client("kim", 1);

    // The titles dump is ingested by the worker shortly after start;
    // re-send the search until it answers with hits.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(600);
    let results = 'search: loop {
        assert!(tokio::time::Instant::now() < deadline, "no search results");
        kim.network
            .send(NetworkCommand::SendReliable(Box::new(
                ServerControl::AniDbSearch {
                    query: "gochiusa".into(),
                },
            )))
            .await
            .unwrap();
        let wait = tokio::time::sleep(std::time::Duration::from_secs(2));
        tokio::pin!(wait);
        loop {
            tokio::select! {
                event = kim.events.recv() => {
                    if let Some(ClientEvent::Network(NetworkEvent::SearchResults { query, results })) = event {
                        assert_eq!(query, "gochiusa");
                        if !results.is_empty() {
                            break 'search results;
                        }
                    }
                }
                _ = &mut wait => break,
            }
        }
    };
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].series, AniDbSeriesId(5391));
    assert_eq!(results[0].title, "Gochuumon wa Usagi Desu ka?");
    assert_eq!(results[0].matched, "GochiUsa");
}
