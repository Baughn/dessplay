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
    AniDbMetadata, AniDbSeriesId, Ed2kHash, FileCatalogEntry, MetadataSource, SeriesRelation,
    SeriesRelations,
};

use super::client::{AniDbApi, LookupError};
use super::curator::{CurateError, Curation, CurationInput, ShortTitleCurator};
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
    /// Server-authored file-catalog write (file identity for the
    /// collective library).
    fn write_catalog(
        &self,
        hash: Ed2kHash,
        entry: FileCatalogEntry,
    ) -> impl Future<Output = ()> + Send;
    /// Run a closure over the server storage; `None` if the server is
    /// running storageless (the worker then has no queue and idles).
    fn with_storage<R>(&self, f: impl FnOnce(&mut ServerStorage) -> R) -> Option<R>;
}

/// Run the worker until a fatal API error. Pacing is cooperative: the
/// API client enforces the rate limit internally, so a busy worker
/// simply awaits it.
pub async fn run<H: AniDbHost>(
    host: H,
    api: Arc<dyn AniDbApi>,
    titles: Arc<dyn TitlesSource>,
    curator: Option<Arc<dyn ShortTitleCurator>>,
) {
    tracing::info!("anidb worker started");
    reconcile_settled_lookups(&host);
    // Next time to consider a titles refresh; learned from storage on
    // the first pass.
    let mut titles_due: u64 = 0;
    // Curation backoff plus the in-flight (non-blocking) model call.
    let mut curation = CurationState::default();
    loop {
        let now = host.now();
        refresh_titles_if_due(&host, &titles, now, &mut titles_due).await;
        // Resolve the replicated state once per pass and share it. On the
        // real server host.view() locks the state mutex and clones the whole
        // CrdtState; the three seeders below used to each clone it, three
        // full-state clones under the hot lock every ~2s during draining --
        // costly on a terabyte-scale seeder. None of the three depend on
        // another's writes within a pass (catalog vs metadata vs storage
        // queues), so one snapshot is correct.
        let view = host.view();
        seed_queues(&host, &view, now);
        populate_catalog(&host, &view).await;
        apply_series_hints(&host, &view).await;
        curate_short_titles(&host, &view, curator.as_ref(), now, &mut curation).await;
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
fn seed_queues<H: AniDbHost>(host: &H, view: &StateView, now: u64) {
    let wanted = wanted_series(view);
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

/// Record file identity (filename + size) for every lookup request not
/// yet in the catalog, so a client that has never held the file can add
/// it to the playlist and download it. Write-if-absent: requests re-arm
/// after compaction, but the catalog persists, so we never rewrite an
/// existing entry (which would spam ops and clobber a filled duration).
async fn populate_catalog<H: AniDbHost>(host: &H, view: &StateView) {
    for info in &view.lookup_requests {
        if view.file_catalog.contains_key(&info.hash) {
            continue;
        }
        host.write_catalog(
            info.hash,
            FileCatalogEntry {
                filename: info.filename.clone(),
                size_bytes: info.size,
                duration_millis: None,
            },
        )
        .await;
    }
}

/// Reconcile filename-derived metadata with a learned directory hint. The
/// fallback series name is written once, at the first lookup — but a client
/// can report the title-like containing directory only later: a playlist add
/// carries no hint and races ahead of the library scan that does, so the
/// first-seen episode of a series can be frozen with its per-episode stem
/// name and split off into its own franchise. Here we rewrite — with no AniDB
/// call, independent of the (possibly settled) lookup schedule — any
/// filename-derived entry whose series name no longer matches the hint. A
/// real AniDB hit is never touched; once a name matches its hint there is
/// nothing to write, so this quiesces.
async fn apply_series_hints<H: AniDbHost>(host: &H, view: &StateView) {
    let Some(hints) = store(host, "reading series hints", |s| s.series_hints()) else {
        return;
    };
    for (hash, hint) in hints {
        let hint = hint.trim();
        if hint.is_empty() {
            continue;
        }
        if let Some(Some(meta)) = view.anidb_metadata.get(&hash)
            && meta.source == MetadataSource::FilenameDerived
            && meta.series_name != hint
        {
            tracing::info!(
                %hash, from = %meta.series_name, to = %hint,
                "applying learned directory hint to filename-derived metadata"
            );
            host.write_metadata(
                hash,
                AniDbMetadata {
                    series_name: hint.to_string(),
                    ..meta.clone()
                },
            )
            .await;
        }
    }
}

/// How many uncurated series to send the model per pass.
const CURATE_BATCH: usize = 20;
/// Back off this long after a curator failure (API down, refusal,
/// bad token) — or a reply that answered nothing — before trying again.
const CURATE_RETRY_MILLIS: u64 = 10 * 60 * 1000;
/// Give up on a series after this many batches whose reply didn't
/// answer it (model omissions, refusals, timeouts — not transport
/// failures, which say nothing about the batch): it settles as a
/// durable no-short-name answer, the curation analogue of the anime
/// queue's `next_attempt = NEVER` tombstone.
const MAX_CURATE_ATTEMPTS: u32 = 5;

/// Curation-pass state carried across worker passes: the failure
/// backoff and the in-flight model call. The call runs on its own
/// blocking task and is only ever *polled* here, so a slow curator
/// (the HTTP timeout is minutes) never delays [`step`]'s user-visible
/// metadata lookups (2026-08-20 audit — the call used to be awaited
/// inline in the drain loop).
#[derive(Default)]
struct CurationState {
    /// Earliest next curator attempt after a failure or empty reply.
    backoff: u64,
    /// The in-flight batch, if any.
    job: Option<CurationJob>,
}

struct CurationJob {
    /// The series sent, in batch order — the key for the positional
    /// answers.
    asked: Vec<AniDbSeriesId>,
    handle: tokio::task::JoinHandle<Result<Vec<Curation>, CurateError>>,
}

/// Reconcile replicated short titles with the AI curator's cache, and
/// grow that cache one batch at a time. Settled `series_relations`
/// rows are written once and never revisited by the lookup schedule,
/// so this pass is both the backfill for rows settled before curation
/// existed and the steady state for new series. Same quiescence shape
/// as [`apply_series_hints`]: once every series in view has a settled
/// cache row and the replicated state matches it, nothing runs but one
/// cheap bulk SQLite read — the API answers each series at most once,
/// and a series it won't answer is retried in rotated batches until it
/// settles as no-short-name after [`MAX_CURATE_ATTEMPTS`].
///
/// The curator only runs when a token is stored
/// ([`crate::storage::ANTHROPIC_TOKEN_KEY`], client-provisioned over
/// the wire) and the titles table has rows to send. Series without a
/// settled answer are left untouched — a tokenless server never
/// clears anything.
async fn curate_short_titles<H: AniDbHost>(
    host: &H,
    view: &StateView,
    curator: Option<&Arc<dyn ShortTitleCurator>>,
    now: u64,
    state: &mut CurationState,
) {
    // Harvest a finished model call (never block on a running one).
    if let Some(job) = state.job.take_if(|job| job.handle.is_finished()) {
        harvest_curation(host, job, now, state).await;
    }

    // One bulk read drives both the reconcile and the batch selection
    // (2026-08-20 audit: this used to be two point queries per known
    // series per pass, under the same lock save_state uses).
    let Some(cache) = store(host, "curated cache", |s| s.curated_titles()) else {
        return; // storageless: no cache, no curation
    };

    // Reconcile replicated state with the settled cache. Unsettled
    // series are skipped, never cleared.
    for (series, relations) in &view.series_relations {
        let Some(row) = cache.get(series).filter(|row| row.settled) else {
            continue;
        };
        let short_titles: Vec<String> = row.title.clone().into_iter().collect();
        if relations.short_titles != short_titles {
            tracing::info!(
                aid = series.0,
                title = %relations.title,
                short = ?short_titles,
                "updating curated short title"
            );
            host.write_relations(
                *series,
                SeriesRelations {
                    short_titles,
                    ..relations.clone()
                },
            )
            .await;
        }
    }

    // Grow the cache: launch the next batch of unsettled series.
    if state.job.is_none()
        && now >= state.backoff
        && let Some(curator) = curator
        && let Some(Some(token)) = store(host, "curator token", |s| {
            s.kv_get(crate::storage::ANTHROPIC_TOKEN_KEY)
        })
        && store(host, "titles presence", |s| s.titles_available()) == Some(true)
    {
        // Fewest-attempts first: series a reply already failed to
        // answer sink behind fresh ones, so one stuck batch can't
        // starve the rest of the catalogue (the ordering *is* the
        // batch rotation — it needs no cursor and survives restarts).
        let mut candidates: Vec<(u32, AniDbSeriesId)> = view
            .series_relations
            .keys()
            .filter_map(|series| match cache.get(series) {
                None => Some((0, *series)),
                Some(row) if !row.settled => Some((row.attempts, *series)),
                Some(_) => None, // settled — never re-asked
            })
            .collect();
        candidates.sort_unstable();
        let mut batch = Vec::new();
        for (_, series) in candidates {
            if batch.len() >= CURATE_BATCH {
                break;
            }
            // The dump can lag a brand-new series; retry after refresh.
            match store(host, "titles for series", |s| s.titles_for(series)) {
                Some(rows) if !rows.is_empty() => batch.push(CurationInput { series, rows }),
                _ => continue,
            }
        }
        if !batch.is_empty() {
            let asked: Vec<AniDbSeriesId> = batch.iter().map(|input| input.series).collect();
            let curator = Arc::clone(curator);
            let handle = tokio::task::spawn_blocking(move || curator.curate(&token, &batch));
            state.job = Some(CurationJob { asked, handle });
        }
    }
}

/// Fold a finished curation call into the durable cache. Answers are
/// positional against the batch we sent, so nothing outside it can be
/// written. Series the reply left unanswered — including the whole
/// batch on a model-side failure — accrue a durable attempt and settle
/// as no-short-name at [`MAX_CURATE_ATTEMPTS`]; a reply that answered
/// nothing arms the backoff, since re-sending immediately would repeat
/// it.
async fn harvest_curation<H: AniDbHost>(
    host: &H,
    job: CurationJob,
    now: u64,
    state: &mut CurationState,
) {
    let CurationJob { asked, handle } = job;
    let unanswered: Vec<AniDbSeriesId> = match handle.await {
        Ok(Ok(answers)) => {
            let mut answered = Vec::new();
            let mut unanswered = Vec::new();
            for (index, series) in asked.iter().enumerate() {
                match answers.get(index) {
                    Some(Curation::Short(name)) => answered.push((*series, Some(name.clone()))),
                    Some(Curation::NoShortName) => answered.push((*series, None)),
                    Some(Curation::Unanswered) | None => unanswered.push(*series),
                }
            }
            store(host, "caching curated titles", |s| {
                for (series, short) in &answered {
                    s.set_curated_short_title(*series, short.as_deref(), now as i64)?;
                }
                Ok(())
            });
            for series in &unanswered {
                tracing::warn!(aid = series.0, "curator reply omitted a series");
            }
            if answered.is_empty() {
                tracing::warn!("curator reply answered nothing; backing off");
                state.backoff = now + CURATE_RETRY_MILLIS;
            }
            unanswered
        }
        Ok(Err(e @ CurateError::Model(_))) => {
            // The model saw this batch and gave nothing usable:
            // evidence against the batch, so every series accrues an
            // attempt (a deterministic refusal must eventually settle,
            // not re-bill forever).
            tracing::warn!("short-title curation failed (will retry): {e}");
            state.backoff = now + CURATE_RETRY_MILLIS;
            asked
        }
        Ok(Err(e @ CurateError::Transport(_))) => {
            // Never reached the model; says nothing about the batch.
            tracing::warn!("short-title curation failed (will retry): {e}");
            state.backoff = now + CURATE_RETRY_MILLIS;
            Vec::new()
        }
        Err(e) => {
            tracing::error!("curator task died: {e}");
            state.backoff = now + CURATE_RETRY_MILLIS;
            Vec::new()
        }
    };
    if !unanswered.is_empty()
        && let Some(gave_up) = store(host, "recording curation attempts", |s| {
            s.record_curation_unanswered(&unanswered, now as i64, MAX_CURATE_ATTEMPTS)
        })
    {
        for series in gave_up {
            tracing::warn!(
                aid = series.0,
                attempts = MAX_CURATE_ATTEMPTS,
                "curator never answered for this series; settling as no-short-name"
            );
        }
    }
}

/// One-time startup reconciliation: re-arm "settled" lookups whose result
/// is missing from the replicated state. A successful lookup records its
/// queue attempt durably in SQLite (settled) but writes the result only
/// into the periodically-snapshotted CRDT state; a restart in that window
/// keeps the settled row yet loses the result, orphaning the entry (no
/// data, no near-term retry). Making both queues honest at startup heals
/// such entries — and any future occurrence — on the next pass.
///
/// - **FILE queue**: re-arm `has_data` rows whose metadata is gone. NoData
///   rows self-heal on their short ladder and are left alone. See
///   [`ServerStorage::rearm_settled_without_metadata`].
/// - **ANIME queue**: re-arm settled rows whose relations are gone — the
///   same restart window, but for the relations graph (a permanent
///   tombstone with no `has_data` marker; see
///   [`ServerStorage::rearm_settled_anime_without_relations`]).
fn reconcile_settled_lookups<H: AniDbHost>(host: &H) {
    let now = host.now();
    let view = host.view();

    let present: BTreeSet<Ed2kHash> = view
        .anidb_metadata
        .iter()
        .filter_map(|(hash, meta)| meta.as_ref().map(|_| *hash))
        .collect();
    if let Some(rearmed) = store(host, "reconcile settled lookups", |s| {
        s.rearm_settled_without_metadata(&present, now as i64)
    }) && !rearmed.is_empty()
    {
        tracing::warn!(
            count = rearmed.len(),
            "re-armed AniDB lookups settled but missing metadata (lost to a restart)"
        );
        for file in rearmed {
            tracing::info!(file = %file, "re-arming orphaned lookup");
        }
    }

    let present_relations: BTreeSet<AniDbSeriesId> =
        view.series_relations.keys().copied().collect();
    if let Some(rearmed) = store(host, "reconcile settled anime", |s| {
        s.rearm_settled_anime_without_relations(&present_relations, now as i64)
    }) && !rearmed.is_empty()
    {
        tracing::warn!(
            count = rearmed.len(),
            "re-armed AniDB relations settled but missing from state (lost to a restart)"
        );
        for series in rearmed {
            tracing::info!(aid = series.0, "re-arming orphaned relations lookup");
        }
    }

    // Curation rows the settling ladder gave up on are not real
    // answers: re-arm them so they re-enter batch rotation with a
    // fresh ladder (at most one ladder of attempts per server start;
    // see `ServerStorage::rearm_curation_give_ups`).
    if let Some(rearmed) = store(host, "reconcile curation give-ups", |s| {
        s.rearm_curation_give_ups()
    }) && !rearmed.is_empty()
    {
        tracing::warn!(
            count = rearmed.len(),
            "re-armed curation rows the settling ladder had given up on"
        );
        for series in rearmed {
            tracing::info!(aid = series.0, "re-arming given-up curation");
        }
    }
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
async fn step<H: AniDbHost>(host: &H, api: &dyn AniDbApi, now: u64) -> Result<bool, LookupError> {
    if let Some(entry) =
        store(host, "due file", |s| s.due_lookups(now as i64, 1)).and_then(|mut due| due.pop())
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

/// Fallback metadata when AniDB doesn't know the file. The series name is
/// the requester's title-like containing-directory hint when one was
/// supplied (so a series' episodes group into one franchise instead of one
/// per episode), else the filename minus its extension. Smarter filename
/// parsing happens at the display level (docs/sync-state.md).
fn filename_derived(filename: &str, series_hint: Option<&str>) -> AniDbMetadata {
    let series_name = series_hint
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            std::path::Path::new(filename)
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|stem| !stem.is_empty())
                .unwrap_or_else(|| filename.to_string())
        });
    AniDbMetadata {
        source: MetadataSource::FilenameDerived,
        series_name,
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
    // The unknown-file backoff is anchored on the older of when we first
    // saw the file and its own mtime, so long-owned files AniDB doesn't
    // know aren't re-polled on the new-file ladder after a queue reset.
    let anchor = schedule::effective_anchor(entry.first_seen, entry.info.mtime);
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
            let next = schedule::next_attempt(now_i, anchor, true, Outcome::Data).unwrap_or(NEVER);
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
                host.write_metadata(
                    hash,
                    filename_derived(&entry.info.filename, entry.info.series_hint.as_deref()),
                )
                .await;
            }
            let next = schedule::next_attempt(now_i, anchor, entry.has_data, Outcome::NoData)
                .unwrap_or(NEVER);
            store(host, "file miss", |s| {
                s.record_lookup_attempt(hash, now_i, next, false)
            });
            Ok(())
        }
        Err(LookupError::Timeout) => {
            let next = schedule::next_attempt(now_i, anchor, entry.has_data, Outcome::Timeout)
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
                // From the curator's cache, not the ANIME reply. A
                // never-curated series starts empty;
                // curate_short_titles fills it in on a later pass.
                short_titles: store(host, "curated cache", |s| s.curated_short_title(series))
                    .flatten()
                    .flatten()
                    .into_iter()
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
    let fetched_at: u64 = store(host, "titles bookkeeping", |s| {
        s.kv_get(titles::FETCHED_AT_KEY)
    })
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

        async fn write_catalog(&self, hash: Ed2kHash, entry: FileCatalogEntry) {
            self.mutate(|state, ts| {
                state.set_file_catalog(ActorId::SERVER, ts, hash, entry);
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
        request_with_mtime(host, i, filename, None);
    }

    fn request_with_mtime(host: &Arc<MockHost>, i: u8, filename: &str, mtime: Option<i64>) {
        request_full(host, i, filename, mtime, None);
    }

    fn request_full(
        host: &Arc<MockHost>,
        i: u8,
        filename: &str,
        mtime: Option<i64>,
        series_hint: Option<&str>,
    ) {
        let info = FileHashInfo {
            hash: hash(i),
            size: 1000 + i as u64,
            filename: filename.to_string(),
            mtime,
            series_hint: series_hint.map(str::to_string),
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
        tokio::spawn(run(Arc::clone(host), api, titles, None))
    }

    #[tokio::test]
    async fn lookup_request_populates_catalog() {
        // Draining a lookup request records the file's identity in the
        // catalog, so a client without the file can still add it.
        let host = MockHost::new();
        request(&host, 1, "Frieren - 01.mkv");
        populate_catalog(&host, &host.view()).await;
        let entry = host.view().file_catalog.get(&hash(1)).cloned();
        assert_eq!(
            entry,
            Some(FileCatalogEntry {
                filename: "Frieren - 01.mkv".into(),
                size_bytes: 1001,
                duration_millis: None,
            })
        );
    }

    #[tokio::test]
    async fn reconcile_rearms_settled_lookup_whose_metadata_was_lost() {
        // hash(1): lookup succeeded (queue settled, has_data, a week out)
        // but its metadata write was lost to a restart -> must be re-armed.
        // hash(2): settled AND present in the metadata view -> left alone.
        let host = MockHost::new();
        let week = 7 * 24 * 60 * 60 * 1000;
        let now = host.now() as i64;
        host.with_storage(|s| {
            for (i, name) in [(1u8, "orphan.mkv"), (2, "known.mkv")] {
                let info = FileHashInfo {
                    hash: hash(i),
                    size: 1,
                    filename: name.into(),
                    mtime: None,
                    series_hint: None,
                };
                s.enqueue_lookup(&info, now).unwrap();
                s.record_lookup_attempt(hash(i), now, now + week, true)
                    .unwrap();
            }
        });
        host.mutate(|state, ts| {
            state.set_anidb_metadata(
                ActorId::SERVER,
                ts,
                hash(2),
                Some(AniDbMetadata {
                    source: MetadataSource::AniDb,
                    series_name: "Known".into(),
                    series_id: Some(AniDbSeriesId(5)),
                    episode_number: Some("1".into()),
                }),
            );
        });
        // Both settled: nothing due now.
        assert!(
            host.with_storage(|s| s.due_lookups(now, 10).unwrap())
                .unwrap()
                .is_empty()
        );

        reconcile_settled_lookups(&host);

        let due = host
            .with_storage(|s| s.due_lookups(host.now() as i64, 10).unwrap())
            .unwrap();
        let due_hashes: Vec<_> = due.iter().map(|e| e.info.hash).collect();
        assert!(
            due_hashes.contains(&hash(1)),
            "metadata-less orphan must re-arm"
        );
        assert!(
            !due_hashes.contains(&hash(2)),
            "a settled lookup with metadata must stay settled"
        );
    }

    #[tokio::test]
    async fn reconcile_rearms_settled_anime_whose_relations_were_lost() {
        // aid 1: an ANIME lookup settled (next_attempt = NEVER) but its
        // relations write was lost to a restart before the CRDT snapshot ->
        // must be re-armed. aid 2: settled AND present in the relations
        // view -> left alone. Mirrors the FILE-queue reconcile.
        let host = MockHost::new();
        let now = host.now() as i64;
        host.with_storage(|s| {
            for aid in [1u32, 2] {
                s.enqueue_anime(AniDbSeriesId(aid), now).unwrap();
                // Settle as a hit (NEVER).
                s.record_anime_attempt(AniDbSeriesId(aid), now, crate::storage::NEVER)
                    .unwrap();
            }
        });
        // Only aid 2's relations survived the restart.
        host.write_relations(
            AniDbSeriesId(2),
            SeriesRelations {
                title: "Known".into(),
                year: Some(2023),
                episode_count: Some(12),
                relations: Default::default(),
                short_titles: vec![],
            },
        )
        .await;
        // Both settled: nothing due now.
        assert!(
            host.with_storage(|s| s.due_anime(now, 10).unwrap())
                .unwrap()
                .is_empty()
        );

        reconcile_settled_lookups(&host);

        let due = host
            .with_storage(|s| s.due_anime(host.now() as i64, 10).unwrap())
            .unwrap();
        let due_aids: Vec<_> = due.iter().map(|e| e.series).collect();
        assert!(
            due_aids.contains(&AniDbSeriesId(1)),
            "a relations-less orphan must re-arm"
        );
        assert!(
            !due_aids.contains(&AniDbSeriesId(2)),
            "a settled anime row with relations must stay settled"
        );
    }

    #[tokio::test]
    async fn populate_catalog_is_write_if_absent() {
        // Requests re-arm after compaction, but the catalog persists; a
        // re-arm must not rewrite an existing entry (which would clobber
        // a filled duration and spam ops).
        let host = MockHost::new();
        request(&host, 1, "Frieren - 01.mkv");
        populate_catalog(&host, &host.view()).await;
        // Simulate a later duration fill.
        host.mutate(|state, ts| {
            state.set_file_catalog(
                ActorId::SERVER,
                ts,
                hash(1),
                FileCatalogEntry {
                    filename: "Frieren - 01.mkv".into(),
                    size_bytes: 1001,
                    duration_millis: Some(1_440_000),
                },
            );
        });
        // Re-arm the request and populate again.
        request(&host, 1, "Frieren - 01.mkv");
        populate_catalog(&host, &host.view()).await;
        let entry = host.view().file_catalog.get(&hash(1)).cloned().unwrap();
        assert_eq!(entry.duration_millis, Some(1_440_000));
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
        api.anime.lock().unwrap().insert(
            AniDbSeriesId(17617),
            anime_hit(17617, "Frieren S2", &[(2, 8692)]),
        );
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
        assert!(
            host.with_storage(|s| s.due_lookups(now + 5 * 60 * 1000, 10))
                .unwrap()
                .unwrap()
                .is_empty()
        );

        // When AniDB learns the file, re-validation upgrades the
        // metadata in place.
        api.files
            .lock()
            .unwrap()
            .insert(hash(2), file_hit(123, "Some Show", "05"));
        eventually(&host, "upgraded metadata", |view| {
            view.anidb_metadata.get(&hash(2)).is_some_and(|m| {
                m.as_ref()
                    .is_some_and(|m| m.source == MetadataSource::AniDb)
            })
        })
        .await;
        worker.abort();
    }

    /// Regression: when AniDB doesn't know a file but the requester supplied
    /// a title-like directory hint, the fallback series name is the hint, not
    /// the per-episode filename stem — so a series' episodes group into one
    /// franchise instead of one entry per episode. Before the hint, the two
    /// episodes below would derive distinct names ("RahXephon - 01" vs "- 02")
    /// and split into two franchises.
    #[tokio::test(start_paused = true)]
    async fn miss_prefers_the_series_hint_over_the_filename_stem() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        // Two unknown episodes of the same show, each carrying the folder hint.
        request_full(&host, 1, "RahXephon - 01.mkv", None, Some("RahXephon"));
        request_full(&host, 2, "RahXephon - 02.mkv", None, Some("RahXephon"));
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);

        for i in [1u8, 2] {
            eventually(&host, "hinted fallback metadata", move |view| {
                view.anidb_metadata.get(&hash(i)).is_some_and(|m| {
                    m.as_ref().is_some_and(|m| {
                        m.source == MetadataSource::FilenameDerived
                            && m.series_name == "RahXephon"
                            && m.series_id.is_none()
                    })
                })
            })
            .await;
        }
        worker.abort();
    }

    /// Regression: an episode whose filename-derived metadata was written
    /// before a directory hint was known (a playlist add carries no hint and
    /// races ahead of the hinted library scan) keeps its per-episode stem
    /// name and splits into its own franchise. Once the hint is learned, the
    /// reconciliation rewrites the stale name to the hint -- with no AniDB
    /// call, even though the file is settled -- so the episode rejoins its
    /// siblings. A real AniDB hit is never rewritten.
    #[tokio::test(start_paused = true)]
    async fn learned_hint_reconciles_a_frozen_filename_derived_name() {
        let host = MockHost::new();
        let now = host.now() as i64;

        // hash(1): frozen with the per-episode stem (the early, hint-less
        // write), but the queue has since learned the folder hint.
        host.write_metadata(
            hash(1),
            AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: "Cardcaptor Sakura - 01 - The Magic Book".into(),
                series_id: None,
                episode_number: None,
            },
        )
        .await;
        // hash(2): a real AniDB hit; the queue hint must not disturb it.
        host.write_metadata(
            hash(2),
            AniDbMetadata {
                source: MetadataSource::AniDb,
                series_name: "Real Series Name".into(),
                series_id: Some(AniDbSeriesId(123)),
                episode_number: Some("02".into()),
            },
        )
        .await;
        // hash(3): filename-derived and already matching its hint -> no churn.
        host.write_metadata(
            hash(3),
            AniDbMetadata {
                source: MetadataSource::FilenameDerived,
                series_name: "Cardcaptor Sakura".into(),
                series_id: None,
                episode_number: None,
            },
        )
        .await;
        host.with_storage(|s| {
            for i in [1u8, 2, 3] {
                let info = FileHashInfo {
                    hash: hash(i),
                    size: 1,
                    filename: "x.mkv".into(),
                    mtime: None,
                    series_hint: Some("Cardcaptor Sakura".into()),
                };
                s.enqueue_lookup(&info, now).unwrap();
            }
        });

        // Advance paused time so the reconciling write carries a later
        // timestamp than the frozen one and wins the LWW merge (in
        // production minutes pass between the two).
        tokio::time::advance(Duration::from_secs(1)).await;
        apply_series_hints(&host, &host.view()).await;

        let view = host.view();
        let name = |i: u8| {
            view.anidb_metadata
                .get(&hash(i))
                .unwrap()
                .as_ref()
                .unwrap()
                .series_name
                .clone()
        };
        // hash(1) rejoins the franchise under the folder name.
        assert_eq!(name(1), "Cardcaptor Sakura");
        // The real AniDB hit is untouched.
        assert_eq!(name(2), "Real Series Name");
        assert_eq!(name(3), "Cardcaptor Sakura");
    }

    /// Regression: a long-owned file AniDB doesn't know (old mtime) but
    /// only just enqueued (first_seen ~ now, e.g. after a queue reset)
    /// must settle to "never re-validate", not the aggressive 30-min
    /// new-file ladder. The mtime anchors the backoff past the 90-day
    /// cutoff. Before the mtime anchor this entry was re-polled every
    /// half hour forever.
    #[tokio::test(start_paused = true)]
    async fn old_mtime_unknown_file_settles_despite_recent_first_seen() {
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        // Owned for 200 days; first_seen is "now" (just enqueued).
        let old_mtime = host.now() as i64 - 200 * 24 * 60 * 60 * 1000;
        request_with_mtime(&host, 2, "Ancient Show - 05.mkv", Some(old_mtime));
        let (titles, _) = titles_source("", false);
        let worker = spawn_worker(&host, &api, titles);

        // The miss still writes the filename fallback (so the file is
        // browsable), exactly as for any unknown file.
        eventually(&host, "fallback metadata", |view| {
            view.anidb_metadata
                .get(&hash(2))
                .is_some_and(|m| m.as_ref().is_some_and(|m| m.series_id.is_none()))
        })
        .await;

        // ...but the queue is settled: nothing is due, ever (NEVER).
        let due_ever = host
            .with_storage(|s| s.due_lookups(i64::MAX, 10))
            .unwrap()
            .unwrap();
        assert_eq!(due_ever.len(), 1, "row kept as a tombstone");
        assert_eq!(
            due_ever[0].next_attempt,
            crate::storage::NEVER,
            "old-mtime unknown file must not be re-polled on the new-file ladder"
        );
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
        assert_eq!(
            meta.source,
            MetadataSource::AniDb,
            "fallback clobbered real data"
        );
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
                local_aliases: Default::default(),
                manual_files: Default::default(),
                anidb_unavailable: false,
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

    /// Kind-3 (short) rows for GochiUsa in three languages, plus a
    /// second series with relations but no shorts at all.
    const GOCHIUSA_DUMP: &str = "\
        5391|1|x-jat|Gochuumon wa Usagi Desu ka?\n\
        5391|3|en|GochiUsa\n\
        5391|3|x-jat|Gochiusa\n\
        777|1|x-jat|Linked Show\n";

    fn seed_relations(host: &Arc<MockHost>, aid: u32, title: &str) {
        let relations = SeriesRelations {
            title: title.into(),
            year: Some(2014),
            episode_count: Some(12),
            relations: Default::default(),
            short_titles: vec![],
        };
        host.mutate(|state, ts| {
            state.set_series_relations(ActorId::SERVER, ts, AniDbSeriesId(aid), relations);
        });
    }

    /// Canned curator: positional answers from a fixed map (aids
    /// missing from the map come back [`Curation::Unanswered`]),
    /// recording every batch it was asked, optionally failing.
    struct MockCurator {
        answers: HashMap<u32, Option<&'static str>>,
        calls: AtomicUsize,
        asked: Mutex<Vec<Vec<u32>>>,
        fail: std::sync::atomic::AtomicBool,
    }

    impl MockCurator {
        fn new(answers: &[(u32, Option<&'static str>)]) -> Arc<Self> {
            Arc::new(Self {
                answers: answers.iter().copied().collect(),
                calls: AtomicUsize::new(0),
                asked: Mutex::new(Vec::new()),
                fail: std::sync::atomic::AtomicBool::new(false),
            })
        }
    }

    impl ShortTitleCurator for Arc<MockCurator> {
        fn curate(
            &self,
            _token: &str,
            batch: &[CurationInput],
        ) -> Result<Vec<Curation>, CurateError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.asked
                .lock()
                .unwrap()
                .push(batch.iter().map(|input| input.series.0).collect());
            if self.fail.load(Ordering::SeqCst) {
                return Err(CurateError::Transport("offline".into()));
            }
            Ok(batch
                .iter()
                .map(|input| match self.answers.get(&input.series.0) {
                    Some(Some(short)) => Curation::Short(short.to_string()),
                    Some(None) => Curation::NoShortName,
                    None => Curation::Unanswered,
                })
                .collect())
        }
    }

    fn dyn_curator(mock: &Arc<MockCurator>) -> Option<Arc<dyn ShortTitleCurator>> {
        Some(Arc::new(Arc::clone(mock)) as Arc<dyn ShortTitleCurator>)
    }

    fn provision_token(host: &Arc<MockHost>) {
        host.with_storage(|s| {
            s.kv_set(crate::storage::ANTHROPIC_TOKEN_KEY, "sk-ant-test")
                .unwrap();
        });
    }

    /// One logical curation pass: run the function, then drive any
    /// spawned model call to completion and harvest it, so tests see
    /// synchronous results (production instead polls across passes).
    async fn curate_pass(
        host: &Arc<MockHost>,
        curator: &Option<Arc<dyn ShortTitleCurator>>,
        state: &mut CurationState,
    ) {
        curate_short_titles(host, &host.view(), curator.as_ref(), host.now(), state).await;
        for _ in 0..1000 {
            let Some(job) = &state.job else { return };
            if !job.handle.is_finished() {
                // The mock runs on the blocking pool; give it a beat.
                std::thread::sleep(std::time::Duration::from_millis(1));
                tokio::task::yield_now().await;
                continue;
            }
            curate_short_titles(host, &host.view(), curator.as_ref(), host.now(), state).await;
        }
        panic!("curation job never completed");
    }

    #[tokio::test]
    async fn curator_answers_are_cached_and_written_once() {
        // Two settled series: one gets a community name, one is
        // asked-and-answered "no short name". Both answers cache, the
        // replicated state updates in the same pass, and later passes
        // neither re-ask nor rewrite.
        let host = MockHost::new();
        seed_relations(&host, 5391, "Gochuumon wa Usagi Desu ka?");
        seed_relations(&host, 777, "Linked Show");
        host.with_storage(|s| {
            s.replace_titles(&titles::parse_dump(GOCHIUSA_DUMP))
                .unwrap();
        });
        provision_token(&host);
        let mock = MockCurator::new(&[(5391, Some("GochiUsa")), (777, None)]);
        let curator = dyn_curator(&mock);
        let mut state = CurationState::default();

        curate_pass(&host, &curator, &mut state).await;

        let view = host.view();
        let gochiusa = &view.series_relations[&AniDbSeriesId(5391)];
        assert_eq!(gochiusa.short_titles, ["GochiUsa"]);
        // The rest of the settled record is untouched.
        assert_eq!(gochiusa.title, "Gochuumon wa Usagi Desu ka?");
        assert_eq!(gochiusa.year, Some(2014));
        assert!(
            view.series_relations[&AniDbSeriesId(777)]
                .short_titles
                .is_empty()
        );
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);

        // Quiesced: a second pass asks nothing and writes nothing.
        let before = host.state.lock().unwrap().clone();
        curate_pass(&host, &curator, &mut state).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        assert_eq!(*host.state.lock().unwrap(), before);
    }

    /// Regression (2026-08-20 audit): a reply naming an aid that was
    /// never in the batch — hallucinated or injected via title rows —
    /// must not be cached. The fix keys answers to the batch
    /// *positionally*, so a curator cannot name an out-of-batch series
    /// at all (the identity filtering itself is pinned at the
    /// `parse_reply` level in curator.rs); what remains expressible is
    /// a reply with surplus entries beyond the batch, which must be
    /// ignored.
    #[tokio::test]
    async fn curator_reply_beyond_the_batch_is_dropped() {
        struct SurplusCurator;
        impl ShortTitleCurator for SurplusCurator {
            fn curate(
                &self,
                _token: &str,
                batch: &[CurationInput],
            ) -> Result<Vec<Curation>, CurateError> {
                let mut out: Vec<Curation> = batch
                    .iter()
                    .map(|_| Curation::Short("GochiUsa".to_string()))
                    .collect();
                // Surplus entries answer nothing that was asked.
                out.push(Curation::Short("Evil".to_string()));
                out.push(Curation::NoShortName);
                Ok(out)
            }
        }
        let host = MockHost::new();
        seed_relations(&host, 5391, "Gochuumon wa Usagi Desu ka?");
        host.with_storage(|s| {
            s.replace_titles(&titles::parse_dump(GOCHIUSA_DUMP))
                .unwrap();
        });
        provision_token(&host);
        let curator: Option<Arc<dyn ShortTitleCurator>> = Some(Arc::new(SurplusCurator));
        let mut state = CurationState::default();

        curate_pass(&host, &curator, &mut state).await;

        // The asked series is answered and cached; nothing else is.
        let cache = host.with_storage(|s| s.curated_titles().unwrap()).unwrap();
        assert_eq!(
            cache.keys().copied().collect::<Vec<_>>(),
            vec![AniDbSeriesId(5391)],
            "only the asked series may gain a cache row"
        );
        assert_eq!(
            host.with_storage(|s| s.curated_short_title(AniDbSeriesId(5391)).unwrap())
                .unwrap(),
            Some(Some("GochiUsa".into()))
        );
    }

    /// Regression (2026-08-20 audit): a well-formed reply that answers
    /// nothing must arm the curator backoff — before the fix the
    /// identical batch was re-sent at pass cadence (POLL_MIN = 5s)
    /// forever, and every series after it was starved.
    #[tokio::test]
    async fn unanswered_batch_arms_the_backoff() {
        let host = MockHost::new();
        seed_relations(&host, 5391, "Gochuumon wa Usagi Desu ka?");
        host.with_storage(|s| {
            s.replace_titles(&titles::parse_dump(GOCHIUSA_DUMP))
                .unwrap();
        });
        provision_token(&host);
        // Answers for no aid at all: the reply is Ok but empty.
        let mock = MockCurator::new(&[]);
        let curator = dyn_curator(&mock);
        let mut state = CurationState::default();

        curate_pass(&host, &curator, &mut state).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        assert!(
            state.backoff > host.now(),
            "a batch that answered nothing must arm the backoff"
        );
        // The asked series is not settled — it will be retried…
        assert_eq!(
            host.with_storage(|s| s.curated_short_title(AniDbSeriesId(5391)).unwrap())
                .unwrap(),
            None,
            "an unanswered series must not look like a settled answer"
        );
        // …and its attempt was recorded durably.
        let cache = host.with_storage(|s| s.curated_titles().unwrap()).unwrap();
        let row = &cache[&AniDbSeriesId(5391)];
        assert_eq!((row.attempts, row.settled), (1, false));
    }

    /// After [`MAX_CURATE_ATTEMPTS`] unanswered batches a series
    /// settles as a durable no-short-name answer and is never sent
    /// again — the "asked and settled" shape the anime queue uses.
    #[tokio::test]
    async fn unanswered_series_settles_after_max_attempts() {
        let host = MockHost::new();
        seed_relations(&host, 5391, "Gochuumon wa Usagi Desu ka?");
        host.with_storage(|s| {
            s.replace_titles(&titles::parse_dump(GOCHIUSA_DUMP))
                .unwrap();
        });
        provision_token(&host);
        let mock = MockCurator::new(&[]);
        let curator = dyn_curator(&mock);
        let mut state = CurationState::default();

        for attempt in 1..=MAX_CURATE_ATTEMPTS {
            state.backoff = 0; // the test drives past each backoff
            curate_pass(&host, &curator, &mut state).await;
            assert_eq!(mock.calls.load(Ordering::SeqCst) as u32, attempt);
        }
        // Settled: reads as a durable "no short name"…
        assert_eq!(
            host.with_storage(|s| s.curated_short_title(AniDbSeriesId(5391)).unwrap())
                .unwrap(),
            Some(None),
            "a series the model never answers must settle as no-short-name"
        );
        // …and no further pass asks the model about it.
        state.backoff = 0;
        curate_pass(&host, &curator, &mut state).await;
        assert_eq!(
            mock.calls.load(Ordering::SeqCst) as u32,
            MAX_CURATE_ATTEMPTS
        );
    }

    /// Attempted series sink behind fresh ones in batch selection, so
    /// one batch the model won't answer can't starve the catalogue:
    /// with more series than one batch holds, the second batch leads
    /// with the never-attempted series.
    #[tokio::test]
    async fn batch_selection_rotates_past_unanswered_series() {
        let host = MockHost::new();
        let count = CURATE_BATCH as u32 + 5;
        let mut rows = Vec::new();
        for aid in 1..=count {
            seed_relations(&host, aid, &format!("Series {aid}"));
            rows.push(crate::storage::TitleRow {
                series: AniDbSeriesId(aid),
                kind: 1,
                lang: "x-jat".into(),
                title: format!("Series {aid}"),
            });
        }
        host.with_storage(|s| s.replace_titles(&rows).unwrap());
        provision_token(&host);
        let mock = MockCurator::new(&[]);
        let curator = dyn_curator(&mock);
        let mut state = CurationState::default();

        curate_pass(&host, &curator, &mut state).await;
        state.backoff = 0;
        curate_pass(&host, &curator, &mut state).await;

        let asked = mock.asked.lock().unwrap().clone();
        assert_eq!(asked.len(), 2);
        assert_eq!(
            asked[0],
            (1..=CURATE_BATCH as u32).collect::<Vec<_>>(),
            "first batch: lowest aids, none attempted yet"
        );
        assert_eq!(
            asked[1][..5],
            (CURATE_BATCH as u32 + 1..=count).collect::<Vec<_>>()[..],
            "second batch must lead with the never-attempted series"
        );
        assert_eq!(
            asked[1][5..],
            (1..=15).collect::<Vec<_>>()[..],
            "…then wrap back to the once-attempted ones"
        );
    }

    #[tokio::test]
    async fn curator_updates_replace_stale_replicated_titles() {
        // A cached answer overrides whatever is replicated — including
        // raw dump tags written before curation existed.
        let host = MockHost::new();
        // Seeded strictly in the past: the mock's clock stamps writes
        // without the real server's Lamport floor, and an equal-stamp
        // LWW tie would resolve by value, not recency.
        host.state.lock().unwrap().set_series_relations(
            ActorId::SERVER,
            SharedTimestamp(1),
            AniDbSeriesId(5391),
            SeriesRelations {
                title: "Gochuumon wa Usagi Desu ka?".into(),
                year: Some(2014),
                episode_count: Some(12),
                relations: Default::default(),
                short_titles: vec!["gochiusa s2".into()],
            },
        );
        host.with_storage(|s| {
            s.set_curated_short_title(AniDbSeriesId(5391), Some("GochiUsa"), 1000)
                .unwrap();
        });

        // No token, no curator needed: the cache alone drives the write.
        curate_short_titles(
            &host,
            &host.view(),
            None,
            host.now(),
            &mut CurationState::default(),
        )
        .await;
        assert_eq!(
            host.view().series_relations[&AniDbSeriesId(5391)].short_titles,
            ["GochiUsa"]
        );
    }

    #[tokio::test]
    async fn curator_needs_a_provisioned_token_and_never_clears_uncached() {
        // Without a stored token nothing is asked; and series without a
        // cached answer keep their replicated titles untouched.
        let host = MockHost::new();
        seed_relations(&host, 5391, "Gochuumon wa Usagi Desu ka?");
        host.with_storage(|s| {
            s.replace_titles(&titles::parse_dump(GOCHIUSA_DUMP))
                .unwrap();
        });
        let mock = MockCurator::new(&[(5391, Some("GochiUsa"))]);
        let curator = dyn_curator(&mock);
        let before = host.state.lock().unwrap().clone();

        curate_pass(&host, &curator, &mut CurationState::default()).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 0, "no token, no call");
        assert_eq!(*host.state.lock().unwrap(), before);

        // Token present but the titles table is empty (fresh server,
        // dump not fetched): still nothing to send.
        let fresh = MockHost::new();
        seed_relations(&fresh, 5391, "Gochuumon wa Usagi Desu ka?");
        provision_token(&fresh);
        curate_pass(&fresh, &curator, &mut CurationState::default()).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn curator_failures_back_off_and_cache_nothing() {
        let host = MockHost::new();
        seed_relations(&host, 5391, "Gochuumon wa Usagi Desu ka?");
        host.with_storage(|s| {
            s.replace_titles(&titles::parse_dump(GOCHIUSA_DUMP))
                .unwrap();
        });
        provision_token(&host);
        let mock = MockCurator::new(&[(5391, Some("GochiUsa"))]);
        mock.fail.store(true, Ordering::SeqCst);
        let curator = dyn_curator(&mock);
        let mut state = CurationState::default();

        curate_pass(&host, &curator, &mut state).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        assert!(state.backoff > host.now(), "failure arms the backoff");
        assert_eq!(
            host.with_storage(|s| s.curated_short_title(AniDbSeriesId(5391)).unwrap())
                .unwrap(),
            None,
            "a failed batch caches nothing"
        );
        // A transport failure says nothing about the batch: no durable
        // attempt accrues (contrast the unanswered-reply tests).
        assert!(
            host.with_storage(|s| s.curated_titles().unwrap())
                .unwrap()
                .is_empty(),
            "a transport failure must not count against the batch"
        );

        // Within the backoff window: no further call, even though the
        // series is still uncurated.
        curate_pass(&host, &curator, &mut state).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);

        // Past the backoff and healthy again: answered and cached.
        mock.fail.store(false, Ordering::SeqCst);
        state.backoff = 0;
        curate_pass(&host, &curator, &mut state).await;
        assert_eq!(mock.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            host.view().series_relations[&AniDbSeriesId(5391)].short_titles,
            ["GochiUsa"]
        );
    }

    /// A model-side failure (refusal, truncation, timeout) is evidence
    /// against the batch: every series in it accrues a durable attempt,
    /// so a deterministic refusal eventually settles instead of
    /// re-billing forever.
    #[tokio::test]
    async fn model_failures_count_against_the_whole_batch() {
        struct RefusingCurator;
        impl ShortTitleCurator for RefusingCurator {
            fn curate(
                &self,
                _token: &str,
                _batch: &[CurationInput],
            ) -> Result<Vec<Curation>, CurateError> {
                Err(CurateError::Model("model refused".into()))
            }
        }
        let host = MockHost::new();
        seed_relations(&host, 5391, "Gochuumon wa Usagi Desu ka?");
        host.with_storage(|s| {
            s.replace_titles(&titles::parse_dump(GOCHIUSA_DUMP))
                .unwrap();
        });
        provision_token(&host);
        let curator: Option<Arc<dyn ShortTitleCurator>> = Some(Arc::new(RefusingCurator));
        let mut state = CurationState::default();

        curate_pass(&host, &curator, &mut state).await;
        assert!(state.backoff > host.now(), "refusal arms the backoff");
        let cache = host.with_storage(|s| s.curated_titles().unwrap()).unwrap();
        let row = &cache[&AniDbSeriesId(5391)];
        assert_eq!(
            (row.attempts, row.settled),
            (1, false),
            "a refusal must count as an unanswered attempt"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn anime_hit_uses_the_curated_cache() {
        // A lookup for a series whose curated answer is already cached
        // writes it straight into the fresh relations record.
        let host = MockHost::new();
        let api = Arc::new(MockApi::default());
        api.anime.lock().unwrap().insert(
            AniDbSeriesId(5391),
            anime_hit(5391, "Gochuumon wa Usagi Desu ka?", &[]),
        );
        host.mutate(|state, ts| {
            state.set_anidb_metadata(
                ActorId::SERVER,
                ts,
                hash(1),
                Some(AniDbMetadata {
                    source: MetadataSource::AniDb,
                    series_name: "Gochuumon wa Usagi Desu ka?".into(),
                    series_id: Some(AniDbSeriesId(5391)),
                    episode_number: Some("1".into()),
                }),
            );
        });
        host.with_storage(|s| {
            s.set_curated_short_title(AniDbSeriesId(5391), Some("GochiUsa"), 1000)
                .unwrap();
        });
        let (titles, _) = titles_source(GOCHIUSA_DUMP, false);
        let worker = spawn_worker(&host, &api, titles);
        eventually(&host, "curated title on the anime hit", |view| {
            view.series_relations
                .get(&AniDbSeriesId(5391))
                .is_some_and(|r| r.short_titles == ["GochiUsa"])
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
