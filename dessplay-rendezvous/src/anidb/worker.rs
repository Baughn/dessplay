//! The AniDB worker: one background task that turns replicated lookup
//! requests into replicated metadata.
//!
//! Each pass it (1) refreshes the anime-titles dump when due, (2)
//! drains the `lookup_requests` GSet and any newly-seen series ids
//! into the SQLite queues, and (3) performs one due lookup — files
//! before series, since files unblock watching. Pacing comes from the
//! client's rate limiter; the worker only sleeps when idle or told to
//! back off.
//!
//! The worker talks to the server through [`AniDbHost`] (implemented
//! by the real server over its shared state, and by an in-memory mock
//! in tests) and to AniDB through [`AniDbApi`] (real UDP client or
//! canned test data).

use std::collections::BTreeSet;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use dessplay_core::state::StateView;
use dessplay_core::types::{
    AniDbMetadata, AniDbSeriesId, Ed2kHash, MetadataSource, SeriesRelation, SeriesRelations,
};

use super::client::{AniDbApi, LookupError};
use super::protocol;
use super::schedule::{self, Outcome};
use super::titles::{self, TitlesSource};
use crate::storage::{NEVER, QueueEntry, ServerStorage};

/// Shortest idle wait between passes.
const POLL_MIN: Duration = Duration::from_secs(5);
/// Longest idle wait — new replicated lookup requests are only noticed
/// on wakeup, so this bounds the latency of a fresh request.
const POLL_MAX: Duration = Duration::from_secs(60);
/// Retry delay for a timed-out ANIME lookup.
const ANIME_TIMEOUT_RETRY_MILLIS: i64 = schedule::TIMEOUT_RETRY_MILLIS;

/// What the worker needs from the server: a clock, the resolved state
/// view, server-authored LWW writes, and the queue storage.
pub trait AniDbHost: Send + Sync + 'static {
    /// Shared-clock unix milliseconds.
    fn now(&self) -> u64;
    /// The current resolved view of the replicated state.
    fn view(&self) -> StateView;
    /// Server-authored metadata write (stamped, applied, broadcast).
    fn write_metadata(
        &self,
        hash: Ed2kHash,
        metadata: AniDbMetadata,
    ) -> impl Future<Output = ()> + Send;
    /// Server-authored relations write.
    fn write_relations(
        &self,
        series: AniDbSeriesId,
        relations: SeriesRelations,
    ) -> impl Future<Output = ()> + Send;
    /// Run a closure over the server storage; `None` if the server is
    /// running storageless (the worker then has no queue and idles).
    fn with_storage<R>(&self, f: impl FnOnce(&mut ServerStorage) -> R) -> Option<R>;
}

/// Run the worker until a fatal API error. Pacing is cooperative: the
/// API client enforces the rate limit internally, so a busy worker
/// simply awaits it.
pub async fn run<H: AniDbHost>(host: H, api: Arc<dyn AniDbApi>, titles: Arc<dyn TitlesSource>) {
    tracing::info!("anidb worker started");
    // Next time to consider a titles refresh; learned from storage on
    // the first pass.
    let mut titles_due: u64 = 0;
    loop {
        let now = host.now();
        refresh_titles_if_due(&host, &titles, now, &mut titles_due).await;
        seed_queues(&host, now);
        match step(&host, &*api, now).await {
            Ok(true) => {} // did work; the client paces the next send
            Ok(false) => {
                // Idle: sleep until the next scheduled attempt, within
                // [POLL_MIN, POLL_MAX].
                let due = store(&host, "next due", |s| s.next_attempt_at()).flatten();
                let wait = due
                    .map(|at| Duration::from_millis((at.max(0) as u64).saturating_sub(now)))
                    .unwrap_or(POLL_MAX)
                    .clamp(POLL_MIN, POLL_MAX);
                tokio::time::sleep(wait).await;
            }
            Err(LookupError::Timeout) => {
                // Rescheduled (+5s) by the step itself; the client has
                // also pushed its own send window out.
            }
            Err(LookupError::Backoff { millis }) => {
                tracing::warn!(millis, "anidb backoff");
                tokio::time::sleep(Duration::from_millis(millis)).await;
            }
            Err(LookupError::Fatal(reason)) => {
                tracing::error!("anidb worker stopping: {reason}");
                return;
            }
        }
    }
}

/// Log-and-discard wrapper around storage results: a storage failure
/// here loses scheduling bookkeeping, never replicated data.
fn store<H: AniDbHost, R>(
    host: &H,
    what: &str,
    f: impl FnOnce(&mut ServerStorage) -> crate::storage::Result<R>,
) -> Option<R> {
    match host.with_storage(f) {
        Some(Ok(value)) => Some(value),
        Some(Err(e)) => {
            tracing::error!("anidb storage failure ({what}): {e}");
            None
        }
        None => None,
    }
}

/// Drain replicated lookup requests into the file queue, and any
/// not-yet-fetched series ids (from metadata and List links) into the
/// anime queue. All inserts are idempotent against existing schedules
/// and tombstones.
fn seed_queues<H: AniDbHost>(host: &H, now: u64) {
    let view = host.view();
    let wanted = wanted_series(&view);
    store(host, "seeding queues", |storage| {
        for info in &view.lookup_requests {
            storage.enqueue_lookup(info, now as i64)?;
        }
        for series in wanted {
            storage.enqueue_anime(series, now as i64)?;
        }
        Ok(())
    });
}

/// Series ids referenced anywhere that don't have relations yet.
fn wanted_series(view: &StateView) -> BTreeSet<AniDbSeriesId> {
    let from_metadata = view
        .anidb_metadata
        .values()
        .filter_map(|meta| meta.as_ref()?.series_id);
    let from_list = view
        .list_entries
        .values()
        .filter_map(|entry| entry.anidb_series_id);
    from_metadata
        .chain(from_list)
        .filter(|series| !view.series_relations.contains_key(series))
        .collect()
}

/// Perform one due lookup, files first. Returns whether work was done.
async fn step<H: AniDbHost>(
    host: &H,
    api: &dyn AniDbApi,
    now: u64,
) -> Result<bool, LookupError> {
    if let Some(entry) = store(host, "due file", |s| s.due_lookups(now as i64, 1))
        .and_then(|mut due| due.pop())
    {
        lookup_file(host, api, now, entry).await?;
        return Ok(true);
    }
    if let Some(entry) =
        store(host, "due anime", |s| s.due_anime(now as i64, 1)).and_then(|mut due| due.pop())
    {
        lookup_anime(host, api, now, entry.series).await?;
        return Ok(true);
    }
    Ok(false)
}

/// Fallback metadata when AniDB doesn't know the file: the series name
/// is the filename minus its extension; smarter parsing happens at the
/// display level (docs/sync-state.md).
fn filename_derived(filename: &str) -> AniDbMetadata {
    let stem = std::path::Path::new(filename)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| filename.to_string());
    AniDbMetadata {
        source: MetadataSource::FilenameDerived,
        series_name: stem,
        series_id: None,
        episode_number: None,
    }
}

async fn lookup_file<H: AniDbHost>(
    host: &H,
    api: &dyn AniDbApi,
    now: u64,
    entry: QueueEntry,
) -> Result<(), LookupError> {
    let hash = entry.info.hash;
    let result = api.file_by_hash(entry.info.size, hash).await;
    let now_i = now as i64;
    match result {
        Ok(Some(file)) => {
            tracing::info!(
                file = %entry.info.filename,
                series = %file.series_name(),
                epno = %file.epno,
                "anidb file lookup hit"
            );
            let metadata = AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: file.series_name().to_string(),
                series_id: Some(file.aid),
                episode_number: Some(file.epno.clone()),
            };
            host.write_metadata(hash, metadata).await;
            let next = schedule::next_attempt(now_i, entry.first_seen, true, Outcome::Data)
                .unwrap_or(NEVER);
            store(host, "file hit", |s| {
                s.record_lookup_attempt(hash, now_i, next, true)
            });
            Ok(())
        }
        Ok(None) => {
            tracing::debug!(file = %entry.info.filename, "anidb does not know this file");
            // Write the filename fallback once — never clobber real
            // metadata if a later re-validation misses.
            let missing = host
                .view()
                .anidb_metadata
                .get(&hash)
                .is_none_or(|existing| existing.is_none());
            if missing {
                host.write_metadata(hash, filename_derived(&entry.info.filename))
                    .await;
            }
            let next =
                schedule::next_attempt(now_i, entry.first_seen, entry.has_data, Outcome::NoData)
                    .unwrap_or(NEVER);
            store(host, "file miss", |s| {
                s.record_lookup_attempt(hash, now_i, next, false)
            });
            Ok(())
        }
        Err(LookupError::Timeout) => {
            let next = schedule::next_attempt(now_i, entry.first_seen, entry.has_data, Outcome::Timeout)
                .unwrap_or(NEVER);
            store(host, "file timeout", |s| {
                s.record_lookup_attempt(hash, now_i, next, false)
            });
            Err(LookupError::Timeout)
        }
        Err(other) => Err(other),
    }
}

async fn lookup_anime<H: AniDbHost>(
    host: &H,
    api: &dyn AniDbApi,
    now: u64,
    series: AniDbSeriesId,
) -> Result<(), LookupError> {
    let result = api.anime_by_id(series).await;
    let now_i = now as i64;
    match result {
        Ok(Some(anime)) => {
            tracing::info!(aid = series.0, title = %anime.title(), "anidb anime lookup hit");
            let relations = SeriesRelations {
                title: anime.title().to_string(),
                year: anime.year,
                episode_count: anime.episode_count,
                relations: anime
                    .relations
                    .iter()
                    .map(|&(code, target)| SeriesRelation {
                        kind: protocol::relation_kind(code),
                        target,
                    })
                    .collect(),
            };
            host.write_relations(series, relations).await;
            // Walk: queue every related series we haven't fetched yet.
            let known = host.view().series_relations;
            store(host, "anime hit", |storage| {
                storage.record_anime_attempt(series, now_i, NEVER)?;
                for &(_, target) in &anime.relations {
                    if !known.contains_key(&target) {
                        storage.enqueue_anime(target, now_i)?;
                    }
                }
                Ok(())
            });
            Ok(())
        }
        Ok(None) => {
            // Settle as a tombstone: asking again won't help.
            tracing::warn!(aid = series.0, "anidb reports no such anime");
            store(host, "anime miss", |s| {
                s.record_anime_attempt(series, now_i, NEVER)
            });
            Ok(())
        }
        Err(LookupError::Timeout) => {
            store(host, "anime timeout", |s| {
                s.record_anime_attempt(series, now_i, now_i + ANIME_TIMEOUT_RETRY_MILLIS)
            });
            Err(LookupError::Timeout)
        }
        Err(other) => Err(other),
    }
}

/// Refresh the titles dump when older than [`titles::REFRESH_MILLIS`].
/// `titles_due` caches the next time a refresh is worth considering so
/// the kv read doesn't run every pass.
async fn refresh_titles_if_due<H: AniDbHost>(
    host: &H,
    source: &Arc<dyn TitlesSource>,
    now: u64,
    titles_due: &mut u64,
) {
    if now < *titles_due {
        return;
    }
    let fetched_at: u64 = store(host, "titles bookkeeping", |s| s.kv_get(titles::FETCHED_AT_KEY))
        .flatten()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    if now.saturating_sub(fetched_at) < titles::REFRESH_MILLIS {
        *titles_due = fetched_at + titles::REFRESH_MILLIS;
        return;
    }
    let source = Arc::clone(source);
    let fetched = tokio::task::spawn_blocking(move || source.fetch()).await;
    match fetched {
        Ok(Ok(text)) => {
            let rows = titles::parse_dump(&text);
            if rows.is_empty() {
                tracing::warn!("titles dump parsed to zero rows; keeping the old table");
                *titles_due = now + titles::RETRY_MILLIS;
                return;
            }
            let count = rows.len();
            store(host, "titles replace", |storage| {
                storage.replace_titles(&rows)?;
                storage.kv_set(titles::FETCHED_AT_KEY, &now.to_string())
            });
            tracing::info!(titles = count, "anime-titles dump refreshed");
            *titles_due = now + titles::REFRESH_MILLIS;
        }
        Ok(Err(e)) => {
            tracing::warn!("titles dump fetch failed (will retry): {e}");
            *titles_due = now + titles::RETRY_MILLIS;
        }
        Err(e) => {
            tracing::error!("titles fetch task died: {e}");
            *titles_due = now + titles::RETRY_MILLIS;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use dessplay_core::CrdtState;
    use dessplay_core::types::{ActorId, FileHashInfo, ListEntryId, SharedTimestamp};
    use tokio::time::Instant;

    use super::super::client::BoxFuture;
    use super::super::protocol::{AnimeResult, FileResult};
    use super::*;

    /// In-memory host over a real CrdtState and an in-memory SQLite.
    struct MockHost {
        state: Mutex<CrdtState>,
        storage: Mutex<Option<ServerStorage>>,
        start: Instant,
    }

    impl MockHost {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(CrdtState::new()),
                storage: Mutex::new(Some(ServerStorage::open_in_memory().unwrap())),
                start: Instant::now(),
            })
        }

        /// Anchored to tokio time so paused-time tests control it.
        fn clock(&self) -> u64 {
            1_700_000_000_000 + self.start.elapsed().as_millis() as u64
        }

        fn mutate(&self, f: impl FnOnce(&mut CrdtState, SharedTimestamp)) {
            let ts = SharedTimestamp(self.clock());
            f(&mut self.state.lock().unwrap(), ts);
        }
    }

    impl AniDbHost for Arc<MockHost> {
        fn now(&self) -> u64 {
            self.clock()
        }

        fn view(&self) -> StateView {
            self.state.lock().unwrap().view()
        }

        async fn write_metadata(&self, hash: Ed2kHash, metadata: AniDbMetadata) {
            self.mutate(|state, ts| {
                state.set_anidb_metadata(ActorId::SERVER, ts, hash, Some(metadata));
            });
        }

        async fn write_relations(&self, series: AniDbSeriesId, relations: SeriesRelations) {
            self.mutate(|state, ts| {
                state.set_series_relations(ActorId::SERVER, ts, series, relations);
            });
        }

        fn with_storage<R>(&self, f: impl FnOnce(&mut ServerStorage) -> R) -> Option<R> {
            self.storage.lock().unwrap().as_mut().map(f)
        }
    }

    /// Canned API: lookup tables plus call counts.
    #[derive(Default)]
    struct MockApi {
        files: Mutex<HashMap<Ed2kHash, Result<Option<FileResult>, LookupError>>>,
        anime: Mutex<HashMap<AniDbSeriesId, Result<Option<AnimeResult>, LookupError>>>,
        file_calls: AtomicUsize,
        anime_calls: AtomicUsize,
    }

    impl AniDbApi for Arc<MockApi> {
        fn file_by_hash(
            &self,
            _size: u64,
            hash: Ed2kHash,
        ) -> BoxFuture<'_, Result<Option<FileResult>, LookupError>> {
            self.file_calls.fetch_add(1, Ordering::SeqCst);
            let result = self
                .files
                .lock()
                .unwrap()
                .get(&hash)
                .cloned()
                .unwrap_or(Ok(None));
            Box::pin(async move { result })
        }

        fn anime_by_id(
            &self,
            aid: AniDbSeriesId,
        ) -> BoxFuture<'_, Result<Option<AnimeResult>, LookupError>> {
            self.anime_calls.fetch_add(1, Ordering::SeqCst);
            let result = self
                .anime
                .lock()
                .unwrap()
                .get(&aid)
                .cloned()
                .unwrap_or(Ok(None));
            Box::pin(async move { result })
        }
    }

    struct MockTitles {
        dump: &'static str,
        fail: bool,
        calls: AtomicUsize,
    }

    impl TitlesSource for Arc<MockTitles> {
        fn fetch(&self) -> std::io::Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(std::io::Error::other("offline"))
            } else {
                Ok(self.dump.to_string())
            }
        }
    }

    fn titles_source(dump: &'static str, fail: bool) -> (Arc<dyn TitlesSource>, Arc<MockTitles>) {
        let mock = Arc::new(MockTitles {
            dump,
            fail,
            calls: AtomicUsize::new(0),
        });
        (Arc::new(Arc::clone(&mock)) as Arc<dyn TitlesSource>, mock)
    }

    fn hash(i: u8) -> Ed2kHash {
        Ed2kHash([i; 16])
    }

    fn request(host: &Arc<MockHost>, i: u8, filename: &str) {
        let info = FileHashInfo {
            hash: hash(i),
            size: 1000 + i as u64,
            filename: filename.to_string(),
        };
        host.mutate(|state, _| {
            state.request_lookup(info);
        });
    }

    fn file_hit(aid: u32, name: &str, epno: &str) -> Result<Option<FileResult>, LookupError> {
        Ok(Some(FileResult {
            fid: 1,
            aid: AniDbSeriesId(aid),
            romaji: name.to_string(),
            english: String::new(),
            epno: epno.to_string(),
        }))
    }

    fn anime_hit(
        aid: u32,
        title: &str,
        relations: &[(u16, u32)],
    ) -> Result<Option<AnimeResult>, LookupError> {
        Ok(Some(AnimeResult {
            aid: AniDbSeriesId(aid),
            year: Some(2023),
            relations: relations
                .iter()
                .map(|&(kind, target)| (kind, AniDbSeriesId(target)))
                .collect(),
            romaji: title.to_string(),
            english: String::new(),
            episode_count: Some(12),
        }))
    }

    /// Spawn the worker and wait (in paused time) for `pred` over the
    /// host to hold. The virtual deadline is generous (over an hour)
    /// because re-validation schedules reach ~30 minutes out.
    async fn eventually(host: &Arc<MockHost>, what: &str, pred: impl Fn(&StateView) -> bool) {
        for _ in 0..1500 {
            if pred(&host.view()) {
                return;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn spawn_worker(
        host: &Arc<MockHost>,
        api: &Arc<MockApi>,
        titles: Arc<dyn TitlesSource>,
    ) -> tokio::task::JoinHandle<()> {
        let api: Arc<dyn AniDbApi> = Arc::new(Arc::clone(api));
        tokio::spawn(run(Arc::clone(host), api, titles))
    }

    #[tokio::test(start_paused = true)]
    async fn file_hit_writes_metadata_and_walks_relations() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        api.files
            .lock()
            .unwrap()
            .insert(hash(1), file_hit(8692, "Sousou no Frieren", "01"));
        api.anime.lock().unwrap().insert(
            AniDbSeriesId(8692),
            anime_hit(8692, "Sousou no Frieren", &[(1, 17617)]),
        );
        api.anime
            .lock()
            .unwrap()
            .insert(AniDbSeriesId(17617), anime_hit(17617, "Frieren S2", &[(2, 8692)]));
        request(&host, 1, "[SubsPlease] Sousou no Frieren - 01.mkv");

        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);

        eventually(&host, "metadata", |view| {
            view.anidb_metadata.get(&hash(1)).is_some_and(|m| {
                m.as_ref().is_some_and(|m| {
                    m.source == MetadataSource::AniDb
                        && m.series_name == "Sousou no Frieren"
                        && m.series_id == Some(AniDbSeriesId(8692))
                        && m.episode_number.as_deref() == Some("01")
                })
            })
        })
        .await;
        // The relations walk reaches the sequel, and the sequel's own
        // back-edge doesn't loop (8692 is already known).
        eventually(&host, "relations walk", |view| {
            view.series_relations.contains_key(&AniDbSeriesId(8692))
                && view.series_relations.contains_key(&AniDbSeriesId(17617))
        })
        .await;
        let relations = &host.view().series_relations[&AniDbSeriesId(8692)];
        assert_eq!(relations.title, "Sousou no Frieren");
        assert_eq!(relations.year, Some(2023));
        assert_eq!(relations.episode_count, Some(12));
        assert_eq!(relations.relations.len(), 1);

        // Settled: more virtual time passes without new ANIME calls.
        let calls = api.anime_calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(120)).await;
        assert_eq!(api.anime_calls.load(Ordering::SeqCst), calls);
        worker.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn miss_writes_filename_fallback_and_revalidates() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        // Not in the canned table -> Ok(None), a definitive miss.
        request(&host, 2, "Some Show - 05.mkv");
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);

        eventually(&host, "fallback metadata", |view| {
            view.anidb_metadata.get(&hash(2)).is_some_and(|m| {
                m.as_ref().is_some_and(|m| {
                    m.source == MetadataSource::FilenameDerived
                        && m.series_name == "Some Show - 05"
                        && m.series_id.is_none()
                })
            })
        })
        .await;

        // Re-validation: the queue holds a future attempt ~30min out.
        let now = host.now() as i64;
        let due_soon = host
            .with_storage(|s| s.due_lookups(now + 31 * 60 * 1000, 10))
            .unwrap()
            .unwrap();
        assert_eq!(due_soon.len(), 1, "entry should be scheduled for ~30min");
        assert!(host
            .with_storage(|s| s.due_lookups(now + 5 * 60 * 1000, 10))
            .unwrap()
            .unwrap()
            .is_empty());

        // When AniDB learns the file, re-validation upgrades the
        // metadata in place.
        api.files
            .lock()
            .unwrap()
            .insert(hash(2), file_hit(123, "Some Show", "05"));
        eventually(&host, "upgraded metadata", |view| {
            view.anidb_metadata.get(&hash(2)).is_some_and(|m| {
                m.as_ref().is_some_and(|m| m.source == MetadataSource::AniDb)
            })
        })
        .await;
        worker.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn fallback_never_clobbers_real_metadata() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        api.files
            .lock()
            .unwrap()
            .insert(hash(3), file_hit(50, "Real Name", "01"));
        request(&host, 3, "file.mkv");
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);
        eventually(&host, "metadata", |view| {
            view.anidb_metadata.contains_key(&hash(3))
        })
        .await;

        // AniDB "forgets" the file; the weekly re-validation misses.
        api.files.lock().unwrap().insert(hash(3), Ok(None));
        tokio::time::sleep(Duration::from_secs(8 * 24 * 3600)).await;
        let meta = host.view().anidb_metadata[&hash(3)].clone().unwrap();
        assert_eq!(meta.source, MetadataSource::AniDb, "fallback clobbered real data");
        assert_eq!(meta.series_name, "Real Name");
        worker.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn fatal_errors_stop_the_worker() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        api.files
            .lock()
            .unwrap()
            .insert(hash(4), Err(LookupError::Fatal("bad credentials".into())));
        request(&host, 4, "x.mkv");
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);
        tokio::time::timeout(Duration::from_secs(600), worker)
            .await
            .expect("worker should stop on fatal errors")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn backoff_pauses_then_recovers() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        api.files
            .lock()
            .unwrap()
            .insert(hash(5), Err(LookupError::Backoff { millis: 60_000 }));
        request(&host, 5, "y.mkv");
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);

        // Give it time to hit the backoff, then heal the API.
        tokio::time::sleep(Duration::from_secs(20)).await;
        api.files
            .lock()
            .unwrap()
            .insert(hash(5), file_hit(9, "Healed", "01"));
        eventually(&host, "post-backoff metadata", |view| {
            view.anidb_metadata.contains_key(&hash(5))
        })
        .await;
        worker.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn list_links_seed_the_relations_walk() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        api.anime
            .lock()
            .unwrap()
            .insert(AniDbSeriesId(777), anime_hit(777, "Linked Show", &[]));
        host.mutate(|state, ts| {
            let entry = dessplay_core::types::SeriesListEntry {
                name: "Linked Show".into(),
                nero_name: None,
                genre: None,
                notes: vec![],
                recommender: None,
                status: dessplay_core::types::ListStatus::Active,
                status_note: None,
                source: None,
                watchers: Default::default(),
                anidb_series_id: Some(AniDbSeriesId(777)),
            };
            state.put_list_entry(ActorId::SERVER, ts, ListEntryId(1), entry);
        });
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);
        eventually(&host, "list-seeded relations", |view| {
            view.series_relations
                .get(&AniDbSeriesId(777))
                .is_some_and(|r| r.title == "Linked Show")
        })
        .await;
        worker.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn titles_refresh_daily_and_retry_on_failure() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        let (titles, mock) = titles_source("1|1|x-jat|Seikai no Monshou\n", false);
        let worker = spawn_worker(&host, &api, titles);

        // First pass fetches.
        eventually(&host, "first fetch", |_| {
            mock.calls.load(Ordering::SeqCst) >= 1
        })
        .await;
        let hits = host
            .with_storage(|s| s.search_titles("seikai", 5))
            .unwrap()
            .unwrap();
        assert_eq!(hits.len(), 1);

        // No refetch inside 24h.
        tokio::time::sleep(Duration::from_secs(12 * 3600)).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        // Past 24h, it refreshes.
        tokio::time::sleep(Duration::from_secs(13 * 3600)).await;
        assert!(mock.calls.load(Ordering::SeqCst) >= 2);
        worker.abort();

        // A failing source retries within the hour but not sooner.
        let host2 = MockHost::new();
        let (failing, fail_mock) = titles_source("", true);
        let worker2 = spawn_worker(&host2, &api, failing);
        eventually(&host2, "first failed fetch", |_| {
            fail_mock.calls.load(Ordering::SeqCst) >= 1
        })
        .await;
        let calls = fail_mock.calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        assert_eq!(fail_mock.calls.load(Ordering::SeqCst), calls);
        tokio::time::sleep(Duration::from_secs(40 * 60)).await;
        assert!(fail_mock.calls.load(Ordering::SeqCst) > calls);
        worker2.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn no_such_anime_settles() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        api.files
            .lock()
            .unwrap()
            .insert(hash(6), file_hit(404, "Ghost", "01"));
        // aid 404 is not in the anime table -> Ok(None).
        request(&host, 6, "ghost.mkv");
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);
        eventually(&host, "metadata", |view| {
            view.anidb_metadata.contains_key(&hash(6))
        })
        .await;
        // Wait for the (single) ANIME attempt, then confirm it settles.
        eventually(&host, "anime attempt", |_| {
            api.anime_calls.load(Ordering::SeqCst) >= 1
        })
        .await;
        let calls = api.anime_calls.load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(3600)).await;
        assert_eq!(api.anime_calls.load(Ordering::SeqCst), calls);
        worker.abort();
    }
}
