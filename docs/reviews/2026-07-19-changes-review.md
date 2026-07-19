# DessPlay Codebase Review — Remediation Report

_Generated 2026-07-19 by a multi-agent audit (7 Opus finder agents, one per
area; every bug/security finding then independently adversarially verified).
8 raw findings → 7 kept (3 confirmed bugs, 4 non-bug findings passed through
unverified), 0 refuted._

_Revision: jj change `mvmmvuturlsp` · commit `350cec3b8c64`. Scope: changes
since `141e41831c4c` (the 2026-07-05 scoped review)._

<!-- audit-revision
mode: scoped
commit: 350cec3b8c64
jj-change: mvmmvuturlsp
base: 141e41831c4c
generated: 2026-07-19
-->

> Project norm (CLAUDE.md): write a failing regression test **before** each
> fix — prefer a property test that happens to catch the bug over a unit test
> that only catches this instance — and treat every bug as a prompt to improve
> the architecture, not just patch the symptom.

## Executive summary

This audit covered two weeks of heavy work (~15k insertions): the entire
torrent-first download stack, the rebuilt 4-tab settings screen, the truecolor
theme and speaker-color system, known-offline playback gating, and Phase 19
List-identity completion. Overall health is good: the settings-UI and
theme/widgets areas produced **zero findings** despite being the largest UI
diffs, and **all seven findings from the 2026-07-05 review were verified fixed
with no regressions** (manual_files resolution, re-import preservation,
next_ep linked-entry bump, stale StartDownload, CannotServe, display-width
tables, edit-modal fields).

The findings that matter cluster in one place with one root cause: **librqbit's
own session persistence (`persistence: Json` + `fastresume`) is a second
source of truth that is never reconciled against the app's tracking** (the
`torrents` table, the `RqbitEngine.torrents` in-memory map, and the in-memory
`nyaa_imports`). The design says in-flight torrents don't survive restarts;
librqbit quietly makes them survive anyway, and every app-level cleanup path
assumes app-level bookkeeping is complete. Both medium-severity bugs — and the
right fix for both — are the same missing startup step: enumerate the restored
librqbit session and drop everything the app's records don't claim.

The remaining findings are low-severity: a guard missing on the
torrent-hash-completion path (a race that momentarily flips a valid file to
Missing), an unbounded HTTP read in the Nyaa client, a test gap on the
production-regression-born known-offline gating invariant, a robustness note
on snapshot-layout disambiguation, and one-shot DNS resolution.

### Fix-first order

1. 🟠 **Reconcile the librqbit session at startup** — `dessplay/src/torrent/rqbit.rs`
   (`RqbitEngine::new` / `remove_key`) + `dessplay/src/actors/file.rs`
   (`reconcile_torrents`): enumerate the restored session, delete torrents with
   no matching `torrents`-table row (this also sweeps abandoned `import-*`
   torrents), and record handles for rows that should live. One fix closes
   findings 1 and 2.
2. ⚪ **Guard `on_torrent_hashed` against an already-adopted local copy** —
   `dessplay/src/actors/file.rs` (`on_torrent_hashed`): mirror the
   StartDownload staleness guard so a late `TorrentHashed` never clobbers
   `local_files`.
3. ⚪ **Put a timeout on the Nyaa HTTP agent** — `dessplay/src/torrent/nyaa.rs`
   (`HttpNyaaSource`): one-line ureq configuration.
4. ⚪ **End-to-end test for known-offline gating across a server restart** —
   `dessplay-rendezvous/tests/known_users.rs`: the invariant exists because of
   a real 2026-07-18 incident; the load-bearing client-router merge is
   currently untested.
5. ⚪ **Assert the V5 snapshot decode path is actually taken** —
   `dessplay-core/src/state.rs` tests: have
   `legacy_blob_mid_phase19_layout_upgrades` assert `migrated == true`.
6. ⚪ **Re-resolve DNS on reconnect cycles** — `dessplay/src/run.rs` /
   `dessplay-core/src/net/quic.rs`: refresh the address set after repeated
   connect failures.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| Torrent downloads (engine + file actor) | | 2 | 2 | 4 |
| Known-offline gating | | | 1 | 1 |
| CRDT snapshot versioning | | | 1 | 1 |
| Session / network dial | | | 1 | 1 |
| Settings UI | | | | 0 |
| Theme / widgets | | | | 0 |
| **Total** | **0** | **2** | **5** | **7** |

## Torrent downloads (engine + file actor)

**Files:** `dessplay/src/torrent/rqbit.rs` (librqbit wrapper),
`dessplay/src/torrent/nyaa.rs` (RSS search + .torrent fetch),
`dessplay/src/torrent/mod.rs` (TorrentFetches policy core),
`dessplay/src/actors/file.rs` (orchestration, cache, reconciliation)
**Read first:** design.md → *BitTorrent Downloads* and *Download Cache and
Retention* — the invariants: in-flight downloads don't survive restarts;
pending imports are not restored; a local copy trumps the download; the cache
is hash-addressed with the filesystem as source of truth.
**Key entry points:** `RqbitEngine::new` (session with Json persistence +
fastresume), `FileActor::reconcile_torrents` (startup, table-driven),
`start_nyaa_import` / `finish_nyaa_import` (browse imports),
`on_torrent_hashed` (completion → ed2k verify → cache placement).
**Theme:** librqbit's persisted session is an unreconciled second source of
truth; every cleanup path trusts app-level bookkeeping that the session
restore bypasses.

### 🟠 MEDIUM · A mid-download torrent silently survives restart: the app "drops" it but never tells librqbit

**`dessplay/src/torrent/rqbit.rs:251`** · _bug (confirmed)_

The session is created with `persistence: Some(Json)` and `fastresume: true`
(rqbit.rs:85–104), so librqbit auto-restores every persisted torrent at
startup. But `RqbitEngine.torrents` — the in-memory map `remove_key` uses to
find the handle to delete — is constructed empty (rqbit.rs:108–109) and never
reconciled against the restored session. At startup, `reconcile_torrents`
(file.rs:1676–1705) correctly identifies the incomplete torrent's
`torrents`-table row and calls `drop_torrent` → `engine.remove` → `remove_key`
— which finds nothing in its empty map and returns at line 251 **without ever
calling `session.delete`**. `drop_torrent` then deletes the DB row and
`remove_dir_all`s the payload dir, but librqbit still holds the torrent: it
re-creates the directory, resumes downloading untracked, and is restored again
on every subsequent startup.

- **Spec:** "A torrent mid-download at shutdown is dropped — in-flight
  downloads don't survive restarts, matching the peer path — and re-searched
  on the next `StartDownload`."
- **Suggested fix:** Regression test first: add a torrent, drop and re-create
  the engine to simulate a restart, assert the torrent is genuinely gone from
  the session. Then reconcile at startup: enumerate the restored session's
  torrents and either delete those the `torrents` table doesn't claim or
  record their handles into the wrapper map (so `remove_key` works). The
  existing test at file.rs:6277 uses `FakeTorrentEngine`, which doesn't model
  session persistence — consider teaching the fake to restore state across
  "restarts" so this class of bug is testable at the actor level.

<details><summary>Verification trail — code pointers</summary>

Verifier traced the full path and confirmed: rqbit.rs:85–104
(persistence+fastresume), 108–109 (empty map), 237–253 (`remove_key`
early-return at 251 — `let Some(handle) = handle else { return }` before
`session.delete`); file.rs:1626 (table row written at Add time, so a
mid-download torrent has one), 1687–1703 (`reconcile_torrents`), 1710–1723
(`drop_torrent` deletes only DB row + dir), 6277–6342 (fake-engine test
doesn't model restoration, so it neither catches nor refutes this).

</details>

### 🟠 MEDIUM · An interrupted Nyaa browse-import is fully orphaned: librqbit restores and seeds it forever, invisible to every cleanup path

**`dessplay/src/actors/file.rs:1109`** · _bug (confirmed, high finder confidence)_

`start_nyaa_import` adds the import torrent to the engine into
`<cache>/torrents/import-{id}/` (file.rs:1109) but writes **no**
`torrents`-table row — the row appears only on successful completion
(file.rs:1823). The only tracking is the in-memory `nyaa_imports` map, and
`next_nyaa_import_id` resets to 1 each launch (ui/app.rs:317). Quit
mid-import and all three startup reclamation paths miss it: the orphan sweep
(file.rs:783–817) only inspects top-level hash-named files and never descends
into `torrents/`; `reconcile_torrents` iterates only the table; and
`finish_nyaa_import`'s cleanup branch (file.rs:1210–1238) is unreachable
because `nyaa_imports` is empty. Meanwhile librqbit's persistence restores
the import torrent, which downloads and then seeds indefinitely — untracked,
unevictable, consuming disk and upload — and the reset import-id counter can
collide a fresh import onto the stale `import-1` directory.

- **Spec:** "Pending imports are not restored after restart, matching other
  in-flight downloads."
- **Suggested fix:** Regression test first: create a stale
  `<cache>/torrents/import-N/` dir plus a pending session import, start the
  FileActor, assert both the directory and the session torrent are removed.
  Fix inside the same startup reconciliation as the finding above: delete any
  `import-*` directory (completed imports are always promoted to the
  hash-keyed path, so `import-*` at startup is by definition abandoned) and
  drop session torrents with no table row. That single sweep covers both
  medium findings.

<details><summary>Verification trail — code pointers</summary>

Verifier confirmed every link: file.rs:1109 (`add_import`, no table write),
1204–1208 (`nyaa_import_dir`), 1210–1238 (cleanup only on error/cancel),
1823 (row on completion only, keyed by ed2k), 783–817 (sweep is top-level
only), 1676–1705 (reconcile is table-only); rqbit.rs:7–9, 85–104 (session
auto-restore documented in the module doc); app.rs:317, 1269–1270 (import id
resets to 1 → `import-1` collision possible).

</details>

### ⚪ LOW · A late `TorrentHashed` can clobber an adopted media-root copy and flap the file to Missing

**`dessplay/src/actors/file.rs:1737`** · _bug (confirmed)_

`on_torrent_hashed` runs unconditionally — it never checks whether the fetch
was cancelled or whether `local_files` already holds this file at a
non-download path. Between the completion-time ed2k hash being spawned and its
`Done::TorrentHashed` delivery, the library scan can adopt a media-root copy
of the same file ("a local copy trumps the download"), which cancels the
torrent and `remove_dir_all`s its payload. When the stale `TorrentHashed`
then arrives, `place_in_cache` fails on the deleted payload and the `Err` arm
calls `on_download_complete` with the deleted path — overwriting the valid
media-root entry in `local_files` and advertising Ready for a nonexistent
file. The next peer solicitation hits the serve-time `!path.exists()` guard
and flips the file to Missing before it re-resolves. In the variant where
`place_in_cache` succeeds, a permanent media-root copy is silently downgraded
to a retention-evictable cache path. Same class as the prior review's
stale-StartDownload finding — fixed there, not mirrored here.

- **Spec:** "A verified copy cancels the peer download … and the entry
  resolves Ready at the local path" — a torrent completion must not clobber an
  already-adopted local copy.
- **Suggested fix:** Regression test first: complete a torrent, adopt a
  media-root copy for the same hash before delivering `TorrentHashed`, assert
  `local_files` still points at the media-root path and no Missing is
  emitted. Fix: at the top of `on_torrent_hashed`, if the fetch is no longer
  active or `local_files[file]` is already a non-`download_path` location,
  delete the payload and return. Architecturally this is the third
  late-completion-message guard in the actor — consider a single
  "is this completion still wanted?" predicate all of them share.

<details><summary>Verification trail — code pointers</summary>

Verifier traced the full race: file.rs:1729–1779 (no guard), 1487
(unconditional `local_files.insert`), 2027–2033 (scan adoption cancels the
fetch — `is_active` is true in the Verifying phase per torrent/mod.rs:184–189,
and cancel-in-Verifying returns `TorrentFetchAction::Remove` per
mod.rs:391–394), 1636 → 1717–1722 (`drop_torrent` deletes the payload the
pending hash refers to), 1749–1760 (`Err` arm uses the stale path),
1850–1852 (serve-time `!path.exists()` → `lost_local_file` → Missing).
Severity kept low: the state self-heals on the next re-resolve; the damage is
a momentary Missing flap or a cache-path downgrade.

</details>

### ⚪ LOW · Nyaa HTTP fetches have no read timeout — a black-holing host parks blocking threads indefinitely

**`dessplay/src/torrent/nyaa.rs:124`** · _quality_

`HttpNyaaSource::search` / `fetch_torrent` (nyaa.rs:118–149) issue
`ureq::get(...).call()` with no configured timeout, on the tokio blocking
pool. The policy-level `search_timeout_millis` watchdog (torrent/mod.rs:410–413)
recovers the *policy* — but the comment at mod.rs:50 calls it "belt-and-braces
over the HTTP timeout", and there is no HTTP timeout. A connected-but-silent
nyaa.si leaves each search's blocking thread parked until TCP gives up;
per-file retries every 15 minutes plus browse searches can accumulate leaked
threads over a long session.

- **Suggested fix:** Configure an explicit timeout (~30s, matching the
  watchdog) on the ureq agent used for both search and `.torrent` fetch.

<details><summary>Verification trail</summary>

Non-bug finding — passed through without a disprove pass. Finder confidence
low: whether ureq 3's defaults bound a stalled read was not confirmed either
way; the fix is cheap enough to apply regardless.

</details>

## Known-offline gating

**Files:** `dessplay-core/src/derive.rs` (`merge_known_offline`, gating),
`dessplay/src/client.rs` (router merge), `dessplay-rendezvous/src/server.rs` +
`storage.rs` (`known_users` persistence), `dessplay-rendezvous/tests/known_users.rs`
**Read first:** design.md → *Presence*, "Known-offline users gate too (for a
week)" — born from the 2026-07-18 incident where a server restart silently
waived a committed, offline Nero's block.
**Key entry points:** `derive::merge_known_offline` (synthesizes Departed
entries), `client.rs:146–149` (merges `known_offline` into the gating peer
list before any derivation).
**Theme:** the implementation checks out; the one finding is a coverage hole
on exactly the wiring whose failure caused the incident.

### ⚪ LOW · No end-to-end test that a committed known-offline user blocks playback across a server restart

**`dessplay-rendezvous/tests/known_users.rs:37`** · _test-gap_

Coverage is split into two halves that never meet:
`derive.rs::known_offline_committed_user_still_blocks` unit-tests
`merge_known_offline` + `playback_blockers` in isolation, and
`known_users.rs::known_user_survives_a_server_restart` tests only the display
side. The load-bearing link — the client router merging `known_offline` into
the peer list that feeds gating (client.rs:146–149, where a separate raw copy
feeds the UI) — is exactly the one-line wiring that could regress with all
tests green, and exactly what failed in production on 2026-07-18.

- **Spec:** "Clients therefore merge `known_offline` into the peer list before
  any derivation … so a committed user blocks."
- **Suggested fix:** Integration test: session one commits a user (Watching)
  to the now-playing series and disconnects them; restart the server on the
  same storage with a fresh registry; a second, present client connects and
  attempts to play; assert playback is blocked purely via the synthesized
  known-offline entry, and that `/skip <name>` or `/ack` clears it.

<details><summary>Verification trail</summary>

Non-bug finding — finder's own reasoning. Pointers checked by the finder:
client.rs:146–149 (router merge), derive.rs:238–263 (`merge_known_offline`),
server.rs:661–685 (server-side `known_offline` build), known_users.rs:37–90
(restart scenario asserts display only).

</details>

## CRDT snapshot versioning

**Files:** `dessplay-core/src/state.rs` (decode chain, layout structs),
`dessplay-rendezvous/src/storage.rs` (pre-migration backup)
**Read first:** sync-state.md → snapshot forward-compat; the "mid-development
layouts can become load-bearing" lesson (the V5 incident).
**Key entry points:** `CrdtState::decode_snapshot` — tries current, then the
versioned fallbacks.
**Theme:** the versioning works today, but its stated safety mechanism has
quietly stopped applying to the newest layouts.

### ⚪ LOW · The trailing `protocol_version` length guard no longer disambiguates the three newest layouts

**`dessplay-core/src/state.rs:885`** · _architecture_

The trailing `protocol_version: u32` was appended so `decode_snapshot` could
tell an old blob from a new one **by byte length, not content** (field doc at
state.rs:82–100). That still works at the V1–V4 boundary, but three layouts
now all carry the u32 — current `CrdtState`, `CrdtStateProtocol5`, and
`CrdtStateV5` — so telling them apart falls back on a wrong-layout decode
*erroring on misaligned bytes*: precisely the content-dependent fragility the
guard was added to eliminate. If `decode::<CrdtState>` ever spuriously
succeeds on a V5 blob (reading a following byte as the absent per-entry
`anidb_unavailable` bool while staying postcard-aligned), the corrupt state is
adopted with `migrated == false`, which also skips the pre-migration backup
(storage.rs:284–287) — and the next save overwrites the only copy. Defended
in practice by newest-first ordering and postcard's misalignment behavior; a
latent robustness gap, not a live bug. Notably,
`legacy_blob_mid_phase19_layout_upgrades` never asserts `migrated`, so a
mis-decode via the wrong path would pass it.

- **Spec:** state.rs field doc: the guard exists for "a byte-length guarantee
  no content-dependent check can give".
- **Suggested fix:** Minimum: make `legacy_blob_mid_phase19_layout_upgrades`
  assert `migrated == true` via `decode_snapshot_flagged`. Better: store a
  distinct `protocol_version` value per layout and read it early to *select*
  the decoder, instead of relying on decode-failure fall-through among
  same-length layouts.

<details><summary>Verification trail</summary>

Non-bug finding — finder's own reasoning; confidence low on whether an
adversarially-aligned V5 blob can actually mis-decode (not demonstrated).
Kept because the invariant's silent erosion is real regardless: state.rs:885–895
(chain order), 668–716 (V5/Protocol5 both carry the u32), storage.rs:284–287
(backup gated on `migrated`).

</details>

## Session / network dial

**Files:** `dessplay/src/run.rs` (startup wiring), `dessplay-core/src/net/quic.rs`
(multi-address connector), `dessplay/src/actors/network.rs` (reconnect loop)
**Read first:** network-design.md → *Dialing* — family-interleaved multi-address
dial with a 10s per-address budget, born from the stale-NDP IPv6 incident.
**Key entry points:** `prepare` (one-shot `lookup_host`), `QuicConnector::connect`.
**Theme:** the new dial ladder is sound; its input is frozen at startup.

### ⚪ LOW · Server addresses are resolved once at startup — a mid-session IP change is unrecoverable without a restart

**`dessplay/src/run.rs:374`** · _architecture_

`prepare` calls `lookup_host` exactly once (run.rs:374) and the connector
stores the frozen `Vec<SocketAddr>` (quic.rs:213); every reconnect attempt in
the network actor's loop (network.rs:193–267) re-dials only that set. If the
rendezvous host's records change mid-session (dynamic IPv6 prefix rotation is
the realistic case — the same class of failure the dial ladder was built for),
the client cycles the dead addresses forever, showing "connecting to server
(attempt N)…" until restarted, even though a fresh lookup would succeed.
Pre-existing limitation surfaced by the dial rework, which is also the natural
place to fix it.

- **Suggested fix:** After M consecutive failed connect cycles (or on every
  fresh reconnect cycle), re-run `lookup_host` against the stored hostname and
  refresh the connector's address set. Keep startup resolution as the fast
  path.

<details><summary>Verification trail</summary>

Non-bug finding — finder's own reasoning. Pointers: run.rs:374 (one-shot
resolve), run.rs:411 (fixed addrs into connector), quic.rs:213 / 338 (frozen
set iterated on every connect), network.rs:193–267 (reconnect loop, no
re-resolution).

</details>

## Prior-review status (2026-07-05)

All seven findings from the previous report were checked and are **fixed with
no regressions**:

| Prior finding | Fix commit | Status |
|---|---|---|
| Series-identity steps 2/3 untested | `7c0f0447` (test-suite audit) | closed |
| `manual_files` unreachable without metadata | `3334a557` | fixed |
| Re-import wipes app-owned fields | `99245977` | fixed |
| Linked entries bump next_ep from filename parse | `e5fc3283` | fixed |
| Stale StartDownload resurrects cancelled download | `d71f87f6` | fixed (but see the unmirrored `on_torrent_hashed` analogue above) |
| Mismatched mapping advertises unservable Ready | `250bcb08` (CannotServe) | fixed |
| List table char-count padding / missing edit-modal fields | `a8959f32`, `781a79a5` | fixed |
