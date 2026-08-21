# DessPlay Codebase Review — Remediation Report

_Generated 2026-08-21 by a multi-agent audit (8 Opus finder agents, one per
area; every bug/security finding then independently adversarially verified).
31 raw findings → 30 kept (14 bug/security confirmed, 16 non-bug pass-through),
1 refuted._

_Revision: jj change `yuunyzxuqstksupqnumruwsnwpyyokzt` · commit
`27c90355e1aa712bce44deebee40655cdd21c8e4`. Scope: changes since
`22d85552028de11bd93e7b1e4aa497d71e985f4a` (the 2026-08-20 review's anchor —
i.e. this audit reviews that review's remediation commits plus the v13
handshake and the storage split)._

<!-- audit-revision
mode: scoped
commit: 27c90355e1aa712bce44deebee40655cdd21c8e4
jj-change: yuunyzxuqstksupqnumruwsnwpyyokzt
base: 22d85552028de11bd93e7b1e4aa497d71e985f4a
generated: 2026-08-21
-->

> Project norms (CLAUDE.md): write the failing regression test **before** the
> fix — prefer property tests over one-shot units — unless the fix makes the
> bug class unrepresentable, which is better than either. Run `cargo fmt`
> before committing; verify with `cargo test --workspace --all-targets`.

## Executive summary

The batch under review is almost entirely remediation of the 2026-08-20
report, plus two new features (the protocol-v13 `SyncStatus` handshake and the
`dessplay.sync.db` split). The remediation landed well: **no prior finding
regressed**, and one is still-open only in a corner (the sim pump leak's
partition path). The bad news is a recurring shape: **several fixes cured the
reported reproduction but not the bug class**, and the two new features
shipped genuinely new HIGHs.

Standouts:

- **`/resync` can delete the cache it is supposed to leave alone.** The reset
  blanks the replica in place and the immediately re-derived (empty) view
  drives the eviction pass, which — seeing no playlist and no now-playing —
  deletes cached media, including the file mpv currently has open under
  `cache_retention = 0`. This is the manual remedy the divergence advisor
  tells users to run (finding 1).
- **The false-EOF recovery loops or no-ops.** The seek-back target is the very
  position that produced the phantom EOF, so recovery either spins
  EOF→seek→EOF at IPC rate or leaves the client parked in exactly the
  2026-08-20 wedge it was built to fix; the no-position fallback seeks to 0
  and can publish position 0 to the group (findings 12–14).
- **A network outage permanently un-names series.** Every `ureq` timeout —
  connect, DNS, TLS included, where the model never saw the batch — is
  classified as a model-side failure, so a few hours of blackholed egress
  walks the catalogue into durable `settled, no-short-name` rows with no
  repair path short of hand-editing the server's SQLite (finding 22). The
  curator's `<titles>` fence is also forgeable by a community-submitted title
  containing `</titles>` (finding 23).

Recurring themes:

1. **Fixed the repro, not the class.** The chunk-control rewrite still
   strands in-flight chunks on a write failure (8); the `CannotServe` fix
   rebuilt the unheld arm but left the stale-mismatch arm permanently denying
   (9); the sim teardown fix missed the partition path (18); the buffered-open
   fix reintroduced its own failure mode via a cancellable `await` (20). When
   closing these, prefer the structural version — one funnel per "stream is
   gone" / "copy appeared" event — over patching the remaining arm.
2. **Reset publishes an empty truth.** Both `/resync` HIGH/MEDIUMs (1, 2)
   stem from `reset_state` blanking the replica in place: derived layers
   can't tell "awaiting re-adoption" from "genuinely empty", and the kept
   `ActorId` makes post-reset dots collide with the server's memory.
   **Decided direction (2026-08-21):** `/resync` becomes clear-and-re-exec —
   one reset path shared with `--reset-sync`, fresh process, fresh `ActorId`
   — paired with an adoption gate on eviction to protect the startup path
   both then share.
3. **One-shot edges on lossy channels.** The divergence healed/persisted
   events (3) and the mid-dial `TransferStreamFailed` (20) are the same
   ask-once-latch-with-a-lossy-answer shape the last review flagged.
   Sampled/watch-style state beats edge events wherever the consumer holds a
   sticky flag.
4. **Load-bearing prose.** The migration epoch bump is documented as
   redundant when it is the only thing preventing silent op-swallowing (15);
   the sim kill doc asserts semantics the code doesn't have (19); the
   commentary cap claims "same order" as the trim it replaced while being ~5×
   (28). Each invites a correct-looking future edit that breaks the system.

### Fix-first order

1. 🔴 `/resync` eviction wipe — `dessplay/src/session.rs` (`plan_eviction` /
   `PlayerWiring::on_state`): gate eviction on a state having been loaded or
   adopted this session (this protects the startup path too), and rebuild
   `/resync` as clear-and-re-exec (see the finding for the decided design).
2. 🔴 False-EOF recovery loop — `dessplay/src/session.rs:2186` +
   `actors/player.rs` (`RecoverFalseEof`): seek only into verified data, gate
   re-issue on real progress, make the no-position case a re-arm without seek
   (covers findings 12–14 together).
3. 🔴 Curator timeout classification — `dessplay-rendezvous/src/anidb/curator.rs:159`
   (`curate`): classify by timeout phase, set explicit connect/resolve
   timeouts, add a re-arm path for give-up settles.
4. 🟠 Prompt-fence injection — `curator.rs:267` (`prompt`): nonce-tag or
   sanitise the `<titles>` fence.
5. 🟠 Actor identity on reset — `dessplay/src/actors/sync.rs:777`
   (`reset_state`): made unrepresentable by the clear-and-re-exec design (a
   fresh process mints a fresh session `ActorId`; the offline buffer dies
   with it). Falls out of fix 1 — only needs its own work if any in-place
   reset path survives.
6. 🟠 Stale `CannotServe` arm — `dessplay/src/actors/file.rs:2194`
   (`serve_block_hashes`): require a manual mapping for CannotServe; treat a
   stale registration as evidence to re-resolve, not identity.
7. 🟠 Write-failure requeue — `file.rs:2311` (`send_on_stream`): requeue the
   source when discarding a dead download stream; funnel every
   "stream is gone" site through one helper.
8. 🟠 Stale-stream install guard — `file.rs:2374` (`on_download_stream`):
   carry a link generation on the open→answer path, or cancel orphaned opens
   with the link.
9. 🟠 Divergence status as sampled state — `sync.rs:937` + `advisor.rs`:
   replace the lossy healed/persisted edges with a `watch`. Re-exec removes
   the worst consequence (the pinned post-`/resync` advisory), leaving the
   auto-heal path — still worth doing, at lower urgency.
10. 🟠 Migration epoch-bump rationale — `dessplay-rendezvous/src/storage.rs:353`:
    rewrite the "redundant" comment (it is load-bearing); pin with a
    merge-vs-adopt test.
11. 🟠 Commentary request growth — `dessplay/src/commentary.rs:122`: size the
    frame budget to the uplink, make governors cache-aware.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| Sync actor & `/resync` | 1 | 2 |  | 3 |
| Client storage split |  |  | 4 | 4 |
| CRDT migration & core state |  | 1 | 3 | 4 |
| File actor: transfer & resolution |  | 3 | 2 | 5 |
| Session & playback: false-EOF recovery | 1 | 1 | 1 | 3 |
| Network actor & sim transport |  |  | 4 | 4 |
| Rendezvous: AniDB curator | 1 | 1 | 2 | 4 |
| UI & commentary |  | 1 | 2 | 3 |
| **Total** | **3** | **9** | **18** | **30** |

---

## Sync actor & `/resync`

**Files:** `dessplay/src/actors/sync.rs` (sync actor: replica, offline buffer,
divergence ladder), `dessplay/src/run.rs` (bridge loop, `/resync` dispatch),
`dessplay/src/advisor.rs` (health-row suggestion seam)
**Read first:** docs/sync-state.md → *Reconnection Sync*, *Divergence Alarm*,
*Manual Reset* — the v13 invariant is "connects always converge; a bare epoch
match never buys a merge"
**Key entry points:** `reset_state` (sync.rs:771), `synced()` (sync.rs:734),
the divergence ladder around sync.rs:922–966
**Theme:** the reset publishes a transiently-empty replica that every derived
layer trusts, and the ladder's terminal events ride a lossy channel.
**Design decision (2026-08-21):** `/resync` will be rebuilt as
clear-and-re-exec — run the `--reset-sync` clearing routine, tear down
cleanly, exec self — unifying the two reset paths. That makes the in-place
reset hazards (kept `ActorId`, preserved latches) unrepresentable; the
adoption gate on eviction (first finding) remains the load-bearing fix, since
the re-exec lands on the startup half of the same hole.

### 🔴 HIGH · `/resync` blanks the view in place — the eviction pass fired on the empty view deletes cached media the playlist still references, including the now-playing file

**`dessplay/src/session.rs:1583`** · _bug (confirmed)_

`SyncCommand::ResetState` replaces the replica with `CrdtState::new()`
(sync.rs:777) and run.rs deliberately re-derives immediately (run.rs:1398-1400).
The `GetView` is queued behind `ResetState` on the same mpsc, so the returned
view is deterministically empty, and it flows into `PlayerWiring::on_state`.
The eviction gate fires (`last_now_playing != view.now_playing`, Some→None),
`plan_eviction` builds `protected = {}`, `playlist = {}`, and `run_eviction`
(file.rs:3366) short-circuits every protection: under
`CacheRetention::AfterWatch` **every cache entry is deleted, including the
episode mpv currently has open**; under the default `Keep(7d)` every
still-queued unwatched episode older than the window dies. The same hole
exists at startup after `dessplay --reset-sync`, before the connect handshake
re-adopts server state. `/resync` is exactly what the divergence advisor tells
the user to run, so the path is realistic, not exotic.

- **Spec:** design.md, Download Cache and Retention — "The now-playing file
  and queued unwatched playlist entries are never evicted, regardless of
  retention."
- **Suggested fix (decided design, 2026-08-21):** Two halves.
  (a) Rebuild `/resync` as clear-and-re-exec: run the `--reset-sync`
  clearing routine, tear down cleanly (player shutdown through the normal
  quit path, terminal restored, one-shot flags stripped from argv), then
  exec self. One reset path shared with `--reset-sync`, and the in-place
  hazards (kept `ActorId`, preserved latches, the transiently-empty
  published view) become unrepresentable. The instance lock is safe across
  exec — Rust opens files `CLOEXEC`, so the fd is released atomically and
  the new process re-acquires.
  (b) The load-bearing fix, since the re-exec lands on the startup half of
  this same hole: gate eviction on adoption — no `RunEviction` until a
  snapshot has been loaded from disk or adopted from the server this
  session. Regression test first at the PlayerWiring seam: start with an
  empty (post-reset) view and a populated cache, assert no `RunEviction` is
  emitted until a non-empty state lands and that the first one after
  adoption protects the playlist.

<details><summary>Verification trail — code pointers</summary>

Confirmed; every link in the chain was traced and no missed guard found:
`reset_state` blanks then publishes (sync.rs:771-793);
`UserAction::ResetSyncedState` → `refresh_ui` → `GetView` ordered behind the
reset on one mpsc (run.rs:1390-1401, :2043-2052); the eviction gate and empty
`plan_eviction` (session.rs:1575-1587, :1143-1165); the directive forwarded
verbatim (session.rs:2732-2744 → file.rs:1166-1174); `evictable`'s only
protections are the two view-derived sets, then the retention arm decides
alone (file.rs:3437-3455; config.rs:214-231 — `AfterWatch` is unconditional
`true`). `eviction_started` (session.rs:270-271) does not distinguish "empty
because reset" from "empty playlist"; `--reset-sync` startup follows the same
shape (sync_storage.rs:254). The protection set is derived from the very view
the reset empties — contradicting the invariant the code's own comments
assert (session.rs:1137-1142, file.rs:3437-3444). Severity note: data is a
re-downloadable cache; the full wipe needs `cache_retention = 0`, but the
advisor actively steers users onto this path (advisor.rs:414-418).

</details>

### 🟠 MEDIUM · `ResetState` keeps the session `ActorId` — post-reset offline edits restart at dot 1 and are silently swallowed on replay over the adopted snapshot

**`dessplay/src/actors/sync.rs:777`** · _bug (confirmed)_

`reset_state` wipes the replica but never rotates `self.actor`. Post-reset map
writes derive dots from the fresh (empty) map clock, so the first write to any
map is `(A, 1)`. Offline, those ops land in `offline_buffer`; on reconnect the
client adopts the server's snapshot — which remembers this actor's pre-reset
counters — and `synced()` replays the buffer, where `crdts::Map::apply` drops
every op whose dot the adopted clock already dominates. The edit vanishes with
no error or log. Verified at the CRDT level against the real `crdts` types.
The existing test (`reset_state_while_down_is_healed_by_the_reconnect_handshake`)
only makes a pre-reset edit, so the loss ships green.

- **Spec:** sync-state.md, Manual Reset / Reconnection Sync #4 — "the client
  re-applies any ops buffered while offline … onto the adopted state".
- **Suggested fix (decided design, 2026-08-21):** Made unrepresentable by
  the clear-and-re-exec `/resync`: the fresh process mints a fresh session
  nonce (client.rs:95), so the actor rotates by construction, and the
  offline buffer dies with the process. Per CLAUDE.md the regression test is
  then optional; prefer the structural assertion — delete
  `SyncCommand::ResetState` (or shrink it to a test-only surface) so no
  in-place reset path remains to get this wrong. If any in-place reset
  survives, it must rotate the actor, and the rig test applies as written
  (offline reset, offline edit, reconnect-adopt a snapshot carrying
  pre-reset dots; assert the edit survives into the view and the upward
  StateMerge). Either way, document the dot-rotation rule next to the
  `last_issued` comment, which covers this hazard for LWW stamps but misses
  dots.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `self.actor` is assigned once per process (sync.rs:399 ←
client.rs:95) and never touched by `reset_state` (sync.rs:771-812);
`map_put` derives dots from the now-empty map clock (state.rs:645-661);
offline buffering at sync.rs:707-711; wholesale adoption then `synced()`
replay at sync.rs:914-920, :749-753; the crdts already-seen check
(crdts-7.3.2 map.rs:186-193) silently drops the replayed `(A,1)`. The
codebase documents this exact hazard class for migration dots
(state.rs:273-280), corroborating the mechanism. Medium: silent loss of user
edits, but gated on `/resync` while offline followed by further offline edits.

</details>

### 🟠 MEDIUM · `DivergenceHealed`/`DivergencePersisted` are one-shot edges on a lossy shared channel — a dropped healed event pins the sticky "run /resync" advisory forever

**`dessplay/src/actors/sync.rs:937`** · _bug (confirmed)_

Both events are emitted with `let _ = try_send(...)` on the channel that also
carries 10 Hz `StateChanged` traffic, and the latch is flipped **before** the
send (`heal_attempts = 0; escalated = false;` precede the healed send), so a
failed send can never be re-attempted for the same episode. The consumer makes
that terminal: `Advisor::on_divergence_healed` is the only clearer of the
deliberately-sticky `diverged_persistent` flag. The 256+256-slot pipeline
backs up during main-loop snapshot stalls, which is precisely when heavy
StateChanged traffic is in flight. Result: a Warning-severity health row
demanding a `/resync` that already succeeded, for the rest of the session.

- **Spec:** sync-state.md, Escalation Ladder — "Any matching hash after a
  failed heal … emits SyncEvent::DivergenceHealed, which clears the sticky
  advisor flag."
- **Suggested fix:** The re-exec design removes this finding's worst path —
  after a clear-and-re-exec `/resync` the advisor restarts clean, so the
  pinned post-resync advisory cannot happen. The lossy edge remains for
  auto-heals (a heal after failed attempts, no resync involved), so the fix
  below still applies at lower urgency. Make divergence status a sampled
  value, not an edge: publish a `watch::Sender<DivergenceStatus>`
  (mirroring the `health` watch,
  the documented pattern for state that must not compete for channel
  capacity, client.rs:47-53) and let the advisor read it on its 1 Hz pass. At
  minimum, split the latch from the announcement and re-attempt on each hash
  frame. Regression test: fill the event channel, emit a heal, drain, assert
  the advisor flag clears.

<details><summary>Verification trail — code pointers</summary>

Confirmed. Latch-before-send at sync.rs:922-939 and :955-966 (equality guard
`heal_attempts == HEAL_ATTEMPTS_ESCALATE` means a dropped escalation is also
never retried); sole clearer at run.rs:1694-1696 → advisor.rs:281, stickiness
deliberate per advisor.rs:265-272; `/resync` provides no second path —
`reset_state` deliberately preserves `escalated` so the single lossy send is
by design the only clearing mechanism (sync.rs:770-786); shared 256-slot
channel + blocking forwarder at client.rs:100, :218-226. The in-code
justification for lossiness ("the RequestMerge above is already queued
regardless") genuinely covers only `Diverged`. Residual doubt is likelihood
only (needs ~512 queued events during a stall), not mechanism.

</details>

---

## Client storage split

**Files:** `dessplay/src/sync_storage.rs` (new: the derived
`dessplay.sync.db`), `dessplay/src/storage.rs`, `dessplay/src/run.rs`
(`--reset-sync`, `--dump`), `dessplay/src/main.rs` (CLI)
**Read first:** docs/sync-state.md → *Snapshot Storage* (the split, the
one-time legacy move, the remediation story); docs/design.md → the two
databases and the shared `<db>.lock`
**Key entry points:** `SyncStorage::open` vs `open_at`, `adopt_legacy_state`,
`run_reset_sync` (run.rs:2196)
**Theme:** the split itself is sound; the edges — the read-only `--dump`
promise, the legacy-move composition, and the CLI surface — are
comment-enforced.

### ⚪ LOW · `SyncStorage::open_at` — the `--dump` constructor — opens read-write with CREATE and runs `migrate`, so the documented read-only guarantee is comment-enforced only

**`dessplay/src/sync_storage.rs:104`** · _architecture_

sync-state.md says `--dump` "runs unlocked beside a live client and is
therefore read-only here"; the code opens READ_WRITE|CREATE, issues
pragma_updates, and runs `migrate` (a `PRAGMA user_version` write). The moment
MIGRATIONS grows a v2 entry, a freshly-built `dessplay --dump` against the
installed running client migrates the schema under the live sync actor —
whose next `save_state` then fails every flush, silently losing the session's
state on a crash. If the sync file is unlinked between the `exists()` check
(run.rs:2164) and the open, `--dump` recreates and migrates an empty database.

- **Spec:** docs/sync-state.md, Snapshot Storage → Locking.
- **Suggested fix:** A dump-path constructor that cannot write:
  `SQLITE_OPEN_READ_ONLY`, no pragmas, no migrate — read `user_version` and
  return a "sync database is from a newer build" error instead. Unit test:
  read-only open against a higher user_version errors; mtime unchanged after
  a dump.

<details><summary>Trail</summary>Non-bug (architecture); finder's reasoning:
sync_storage.rs:104-126, run.rs:2159-2171, docs/sync-state.md:970-974.</details>

### ⚪ LOW · The documented `rm dessplay.sync.db*` remedy is a no-op on a pre-split install — the legacy `crdt_state` row is re-adopted on the next start

**`docs/sync-state.md:954`** · _spec-drift_

Remedy #3 is called "equivalent, just messier" to `--reset-sync`, but
`--reset-sync` deliberately uses `SyncStorage::open` so the legacy row is
moved out of `dessplay.db` first and therefore cleared. On a machine whose
wedged `dessplay.db` has never started a post-split binary, `dessplay.sync.db`
doesn't exist, the `rm` matches nothing, and the next start runs
`adopt_legacy_state` — the wedged state is back. Exactly the incident shape
the split exists to serve, and silent apart from one info line.

- **Suggested fix:** Amend the doc (remedy #3 is equivalent only after the
  one-time move has run); add a warn when the move adopts a row, naming
  `--reset-sync` as the way to discard it.

<details><summary>Trail</summary>docs/sync-state.md:950-956, run.rs:2196-2201,
sync_storage.rs:142-208.</details>

### ⚪ LOW · `--reset-sync` only conflicts with `--dump` — combined with a subcommand it silently resets and discards the requested work

**`dessplay/src/main.rs:86`** · _quality_

`dessplay import-list sheet.csv --reset-sync` parses cleanly, clears the sync
database, prints success, and exits 0 — the import never runs and nothing says
so. Likewise `--seeder`/`--headless`. The `interactive` computation was
updated for `reset_sync`; the clap declaration was not (`--section` next door
models the right idiom with `requires = "dump"`).

- **Suggested fix:** `conflicts_with_all = ["dump", "seeder", "headless",
  "command"]` (or an explicit dispatch guard) plus a parse-rejection unit
  test beside `pipeline_depth_flag_is_gone`.

<details><summary>Trail</summary>main.rs:86-87, :126-127, :205-214.</details>

### ⚪ LOW · Nothing tests the legacy-move + `--reset-sync` composition — the documented operator remedy is asserted only in a code comment

**`dessplay/src/run.rs:2340`** · _test-gap_

The move and the reset are each tested in isolation; the composition
(`run_reset_sync` uses `open`, not `open_at`, precisely so a pre-split legacy
row is moved in and cleared too) is the incident-response path the split was
built for, and the one place where a refactor to `open_at` would silently
reintroduce state resurrection with every test green.

- **Suggested fix:** Test beside `reset_sync_clears_only_the_sync_db`: legacy
  main DB with a distinguishable snapshot, no sync DB; `run_reset_sync`;
  assert `load_state()` is None and the legacy table is gone. Confirm it
  fails under `open_at`.

<details><summary>Trail</summary>run.rs:2196-2201, :2340-2402;
sync_storage.rs:298-320, :413-500.</details>

---

## CRDT migration & core state

**Files:** `dessplay-core/src/state.rs` (snapshot decode/migration,
`upgrade_relations_map`), `dessplay-core/src/series_identity.rs` (new),
`dessplay-rendezvous/src/storage.rs` (server load path),
`dessplay-core/tests/migration.rs` + frozen fixtures
**Read first:** docs/sync-state.md → *Decoding and Migration* — key-derived
migration dots, the epoch bump, and the fixture-pinning policy
**Key entry points:** `upgrade_relations_map` (state.rs:292), server
`load_state` (rendezvous storage.rs:340), `tagged_fixture_bytes`
(migration.rs:302)
**Theme:** the key-derived-dot fix is correct; what's wrong is the recorded
*reasoning* around it — a comment calling the epoch bump redundant, a doc one
bump stale, and a fixture helper that quietly breaks its own provenance claim.

### 🟠 MEDIUM · The migration epoch bump is documented as "redundant" but is load-bearing — without it a same-epoch merge makes the server's next N relations writes look like replays

**`dessplay-rendezvous/src/storage.rs:353`** · _architecture_

`upgrade_relations_map` builds a fresh LwwMap, discarding the pre-migration
`{ActorId::SERVER: N}` clock entry, so the server's next relations write
carries dot `(SERVER, 1)`. Safe today only because `load_state` bumps the
epoch, forcing every client onto the snapshot branch. The comment asserts the
bump is redundant under the v13 hash gate — wrong: migration preserves the
resolved view, so a client matching the pre-migration state still matches
post-migration, the hash gate answers `StateMerge`, the union keeps
`{SERVER:N}` on the client, and `Map::apply` discards the server's next N
relations ops. Not permanent (the divergence ladder eventually forces a
snapshot), but every client silently drops relations updates after a server
migration — and the recorded rationale invites a maintainer to delete the
only guard.

- **Spec:** docs/sync-state.md, Decoding and Migration — "Belt-and-braces,
  the server also bumps the epoch on any migrated load".
- **Suggested fix:** Rewrite the comment (and the matching sync-state.md
  paragraph) to record the real invariant: a rebuild that drops map-level
  dots is only safe if the result is **adopted, never merged**, because the
  rebuild also resets every live writer's counter. Pin with a test: migrate a
  state whose relations clock holds `{SERVER:N}`, merge same-epoch into an
  unmigrated replica, apply a `(SERVER,1)` op, assert it lands.

<details><summary>Trail</summary>Non-bug (architecture — the code is correct,
the rationale is wrong): storage.rs:340-362, state.rs:292-309,
server.rs:1510-1514, worker.rs:794/1492, crdts map.rs:190-193.</details>

### ⚪ LOW · sync-state.md still records `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` as `[11]` — the code says `[11, 12]`

**`docs/sync-state.md:1032`** · _spec-drift_

The v13 bump added 12 (state.rs:576, with rationale: v12→v13 changed only the
SyncStatus wire message); the doc — where the next maintainer looks when the
version-handling test reddens at v14 — stops at v11→v12. Also "the v7-v9
fixture blobs pin this arm" should read v7-v10, and the new
`snapshot-multi-relations-*.bin` pair is unmentioned.

- **Suggested fix:** Update the list, append the v12→v13 rationale, fix the
  fixture range, and point at the multi-relations pins.

<details><summary>Trail</summary>docs/sync-state.md:1032-1036, :1072;
state.rs:573-576.</details>

### ⚪ LOW · `SeriesEntryIndex::resolve` accepts an arbitrary `&StateView` — the doc's "a stale index is unrepresentable" claim is not enforced by the API

**`dessplay-core/src/series_identity.rs:101`** · _quality_

The index borrows the view only through its `&'a str` keys, and `resolve`
takes any view; two StateViews routinely coexist, so a caller can index view A
and resolve against view B, producing resolutions matching neither. The single
caller is correct today; nothing would catch misuse.

- **Suggested fix:** Store `view: &'a StateView` in the struct and drop the
  parameter — the sole caller compiles unchanged and the doc claim becomes
  true.

<details><summary>Trail</summary>series_identity.rs:59-71, :101-112;
ui/props.rs:1835-1841. CLAUDE.md: unrepresentable beats avoided.</details>

### ⚪ LOW · `tagged_fixture_bytes` re-tags a current-encoder blob — snapshot-v12.bin's envelope says 12 while its body's `protocol_version` says 13

**`dessplay-core/tests/migration.rs:311`** · _test-gap_

The helper overwrites only the 4-byte envelope tag; the body's trailing
`protocol_version` varint keeps the capture-time value (verified on the
checked-in blobs: v12 tag/13 body, v11/12, v7/10 — while the natively-captured
`snapshot-multi-relations-v12.bin` is 12/12, so the two v12 fixtures disagree
about what a v12 body looks like). Inert today; contradicts the README's
"these bytes are faithful to what those builds wrote" and goes actively wrong
when PROTOCOL_VERSION crosses the varint boundary at 128.

- **Suggested fix:** Do **not** regenerate existing blobs. Make the helper
  also rewrite the trailing varint for future captures, and note the
  discrepancy for the pre-change fixtures in the README's Provenance section.

<details><summary>Trail</summary>migration.rs:302-312, :340-344;
fixtures/README.md:57-63; state.rs:618-620.</details>

---

## File actor: transfer & resolution

**Files:** `dessplay/src/actors/file.rs` (serving, download streams,
resolution, manual mappings, eviction), `dessplay/src/download.rs`
(scheduler), `dessplay-core/src/net/transfer.rs` (wire types)
**Read first:** docs/network-design.md → *File transfer*, solicitation and
`CannotServe` semantics; docs/design.md → the adoption seam and manual
mappings
**Key entry points:** `serve_block_hashes` (file.rs:2118), `send_on_stream`
(the write-failure arm, file.rs:2303), `on_download_stream` (file.rs:2358),
`lost_local_file` (file.rs:2266)
**Theme:** every finding here is a remainder of a 2026-08-20 fix — an arm the
rewrite didn't reach, or a guard present on the close path but absent on the
install path.

### 🟠 MEDIUM · A data-stream write failure discards the live stream without requeueing its in-flight chunks — a guaranteed 30 s stall per occurrence

**`dessplay/src/actors/file.rs:2311`** · _bug (confirmed)_

The write-failure arm removes the `DownloadStream` (whose `TaskGuard` aborts
the reader, cancelling the trailing `DownloadClosed` — the **only** producer
of the requeue path) and re-queues only the current control message. Every
chunk already handed to the dead stream stays in `src.in_flight`, consuming
the source's window until the 30 s snub. This is the defect class commit
5a3b28a set out to eliminate ("nothing can overflow or be dropped here"), and
if a replacement stream installs first, even a lucky late `DownloadClosed` is
rejected as stale by the generation guard. With a single source, playback
gating on the file stalls the whole group for the snub window.

- **Spec:** docs/network-design.md, Transfer Resumption; 2026-07-28 proposal §3.
- **Suggested fix:** Requeue the source in the write-failure arm (matching
  `on_transfer_stream_failed`, file.rs:1256-1270). Regression test over the
  actor rig: dead far end, drive a ChunkRequest, assert the scheduler
  releases the in-flight chunks without waiting out the snub. Better: funnel
  every "this download stream is gone" site through one remove-and-requeue
  helper.

<details><summary>Verification trail — code pointers</summary>

Confirmed. Write-failure arm removes + requeues only the control message
(file.rs:2309-2315); `TaskGuard::Drop` aborts the reader before its trailing
send (file.rs:759-765, :3519-3524 — sole `DownloadClosed` producer);
`DownloadClosed` is the only caller of `on_source_stream_lost` →
`requeue_source` (file.rs:2455-2485, download.rs:566-600); replacement
install flushes only newly queued chunks (file.rs:2357-2388); fallback is the
30 s snub (download.rs:741-760). Partial mitigation noted: if the connection
is also dead, the re-open fails and `TransferStreamFailed` does requeue — the
unrecovered case is a single stream reset on a healthy connection.

</details>

### 🟠 MEDIUM · `serve_block_hashes`'s mismatch arm answers `CannotServe` from a stale registration — a permanent denial plus a Ready never retracted

**`dessplay/src/actors/file.rs:2194`** · _bug (confirmed)_

Commit 8c2e195 reserved `CannotServe` for definitive identity mismatch and
rebuilt the unheld arm; the mismatch arm below was left untouched, and its
condition is not identity-definitive: it fires whenever `local_files[file]`
names a path whose cached hash differs — reachable with no manual mapping,
because nothing invalidates a registration when content at its path changes
in place (`commit_fresh_hashes` updates only the hash cache). The arm never
consults the index for a real copy elsewhere, never retracts Ready (so the
requester's advert-scoped denial escape hatch never fires), and the stale
registration actively suppresses re-resolution. Transient staleness latches a
permanent denial — the exact shape the commit was eliminating, one arm over.

- **Spec:** docs/design.md — "CannotServe is reserved for a definitive
  identity mismatch … a transient 'not right now' must never be answered
  with it".
- **Suggested fix:** Make the arm's condition match its claim: CannotServe
  only when the registration is the user's manual mapping
  (`self.manual.get(&file) == Some(&path)`); otherwise retry
  `live_index_match` and on a miss `lost_local_file` (drop + prune + flip
  Missing), as the `!path.exists()` branch does. Regression test first:
  resolve to Ready, overwrite the path with different content, rescan, send
  BlockHashRequest — assert no CannotServe and a Missing retraction.

<details><summary>Verification trail — code pointers</summary>

Confirmed. Arm at file.rs:2187-2211 vs. the rebuilt unheld arm :2150-2178;
no invalidation on same-path-new-content — `commit_fresh_hashes`
(:3062-3081) never touches `local_files`, `prune_stale_index` (:2890-2911)
handles vanished paths only, all nine `local_files` write/remove sites
checked; requester-side permanence via `denied.retain` keyed on the offered
set (download.rs:449-478); self-heal suppressed by the held-file early return
(:1196) and the stale-resolve guard (:2542-2551). Reachability skews toward
the stale case: a purely-manual mismatch usually lands in the uncached branch
instead (the `ManualHashed` mismatch arm deliberately doesn't cache).

</details>

### 🟠 MEDIUM · A late `TransferStream` from the previous dead link silently replaces the live download stream — the install path has no generation guard

**`dessplay/src/actors/file.rs:2374`** · _bug (confirmed)_

`open_data_stream` is spawned detached holding an Arc of the transfer
connection; link death aborts only `run_transfer_link` (never its children,
never `conn.close`), so an orphaned open can still succeed after a reconnect
and nothing downstream carries a link identity. `on_download_stream` does a
bare insert, replacing the fresh stream: its reader is aborted (no
`DownloadClosed`), its chunks strand until the snub. The asymmetry is
telling — the close path and `on_serve_stream` are generation-guarded; the
install path mints the generation but never checks one. Newly reachable
because the reconnect-window buffering means a fresh stream on link B now
routinely exists while a slow open from link A is outstanding.

- **Spec:** docs/network-design.md, Transfer Resumption — a reset stream is
  never resumed; a stale stream must not displace the fresh one.
- **Suggested fix:** Carry a monotonic link generation on
  `OpenTransferStream` → `NetworkEvent::TransferStream` →
  `FileCommand::TransferStream` and drop older-generation installs
  (mirroring file.rs:2466). Structurally better: own the open tasks in a
  JoinSet so link death cancels them (answering `TransferStreamFailed`) and
  `conn.close` on teardown. Seam test: stall an open, kill the link, let the
  reconnect open install, release the stalled open, assert the live stream
  survives.

<details><summary>Verification trail — code pointers</summary>

Confirmed, with a noted narrowing: the server closes the session's transfer
connection when the control session ends (rendezvous server.rs:1026), so the
stale success must beat the CONNECTION_CLOSE — a genuine but narrow race
nothing client-side forecloses. Bare insert at file.rs:2374 (no link identity
on FileCommand::TransferStream, file.rs:236; forwarded unfiltered,
session.rs:2784-2799, seeder.rs:221); detached spawn + skipped close
(network.rs:776-782, :804, :643-649); concurrent-opens reachability via
`on_transfer_link_reset` clearing pending entries while the detached task
lives (file.rs:1276-1288). Close-path guards for contrast: file.rs:2466-2473,
:2407, :2495-2504.

</details>

### ⚪ LOW · The one case `CannotServe` is documented for — an out-of-root manual mapping to a different encode — can never reach the CannotServe arm

**`dessplay/src/actors/file.rs:2671`** · _spec-drift_

`Done::ManualHashed`'s mismatch arm deliberately never caches the observed
hash, while `set_manual_mapping` registers the path — so a later solicitation
finds no hash-cache row and falls into the *silent* arm. CannotServe is
unreachable for exactly its documented case unless the mapped file happens to
sit under a media root; the requester re-solicits on every snub forever
(bounded by MAX_SOLICIT_ATTEMPTS backoff) while the holder's stale Ready
stands. transfer.rs:200-220 and download.rs:212-231 currently document
opposite contracts for the same case.

- **Suggested fix:** Pick one contract and encode it — record the definitive
  mismatch at `ManualHashed` time (e.g. a `mismatched_mappings` map the serve
  path consults) and reconcile download.rs's comment, or delete the
  manual-mapping example from both docs.

<details><summary>Trail</summary>file.rs:2671-2682, :2187-2193, :3235-3245;
transfer.rs:200-220; download.rs:212-231.</details>

### ⚪ LOW · `lost_local_file` deletes the durable manual-mapping row on any observed absence — the new stale-resolve guard makes an unplugged drive destroy the mapping

**`dessplay/src/actors/file.rs:2271`** · _spec-drift_

Commit d9655a4's split keeps a mapping absent at startup ("an offline mount …
revives if the path returns") but the new `Done::Resolved` call site prunes
the durable row on any NotFound answer while the path is dead — and a resolve
is routine background work, not the "failed serve or load" the doc names. An
unmounted removable drive is enough to permanently delete the mapping; on
remount the file is out-of-root and out-of-scope, so nothing re-adopts it.
This also inverts the library-scan policy for the same situation
(`reconcile_scan_roots` retains the index when all files vanish).

- **Spec:** docs/design.md, File Matching 4a.
- **Suggested fix:** Narrow the prune to the doc's triggers, or discriminate
  on transience (parent directory gone ⇒ keep the row), as the scan does per
  root. In both cases still drop the registration and flip Missing — only
  the durable row should survive. Unit test: delete the containing dir, drive
  a re-resolve, assert the row survives while `local_files` loses the entry.

<details><summary>Trail</summary>file.rs:964-977 (startup keep),
:2266-2282 (prune), :2538-2555 (new trigger), :2928-2943 (scan contrast);
design.md:314-319.</details>

---

## Session & playback: false-EOF recovery

**Files:** `dessplay/src/session.rs` (PlayerWiring: EOF verdict gating,
partial-file identity), `dessplay/src/actors/player.rs` (mpv IPC,
`RecoverFalseEof`)
**Read first:** docs/design.md → *Playback Rules* #7 (the gap gate: a
deferral, never terminal — and not a spin) and the partial-file section
**Key entry points:** the rejection arm (session.rs:2168-2192),
`RecoverFalseEof` (player.rs:570), `position_near_end` (session.rs:1442)
**Theme:** the recovery added for the 2026-08-20 wedge re-arms and seeks —
but to the wrong place, with nothing bounding repetition.

### 🔴 HIGH · The false-EOF recovery seeks back to the very position that produced the EOF — it spins EOF↔seek or no-ops into the original wedge

**`dessplay/src/session.rs:2186`** · _bug (confirmed)_

The seek target is `last_position` filtered to the file — which is by
construction the position that walked into the zeros (after a user seek it is
the seek target itself; otherwise at most 100 ms behind the phantom end). If
mpv accepts the seek, `eof-reached` re-fires at the same data-less offset →
reject → another `RecoverFalseEof` at the identical position, with no bound,
backoff, or progress requirement — spinning at IPC rate, warn+info per
iteration, incrementing `pending_seek_echoes` (drift there later swallows a
genuine user seek as stale). If mpv no-ops the seek, only the latch clears:
with `--keep-open` and the in-place completion path deliberately not
reloading, no further EOF is ever produced — the exact 2026-08-20 wedge
survives. Neither existing test closes the loop through the seek.

- **Spec:** design.md, Playback Rules #7 — the gap gate "flips back … and
  the group pauses until the window refills": a deferral, not a spin.
- **Suggested fix:** (a) seek only into verified data (the playable-window
  boundary, or last tick observed while advertised playable); (b) make
  repetition unrepresentable — refuse to re-issue for the same (file, target)
  until a PositionTick strictly past the target or the playable verdict
  flips back; (c) when an in-place completion lands while a rejected EOF is
  outstanding, re-issue `Load` instead of only flipping `partial`. Loop test
  first through the PlayerWiring + actor seam asserting the recovery-command
  count is bounded (see the test-gap below).

<details><summary>Verification trail — code pointers</summary>

Confirmed on both branches. Target = `last_position`, no clamp, no
fetched-window consult (session.rs:2186-2192); `UserSeeked` writes
`last_position` synchronously to the seek target (session.rs:2076, with a
comment saying so); re-arm + unconditional seek (player.rs:570-577,
:707-714), echo handler also clears the latch (:775-800), Eof gated only on
the latch (:884-897); `position_near_end` unchanged ⇒ closed cycle. No-op
branch: `assembled_in_place` flips `partial` without reload
(session.rs:1305-1313). Tests: session.rs:4454 hand-feeds the progress;
player.rs:2268's MockPlayer never re-raises EOF. Softening noted: loop rate
is bounded by mpv's seek/demux round-trip, and in the stalled-download case
the target may sit ~100 ms inside fetched data; the user-seek-past-window
case is provably data-less.

</details>

### 🟠 MEDIUM · With no position attributed to the file, the recovery seeks to 0 — rewinding the episode and publishing position 0 for the group

**`dessplay/src/session.rs:2189`** · _bug (confirmed)_

`map_or(0, …)` turns "no honest position for this file" into "seek to the
beginning". The branch only runs when we speak for now-playing, so the
resulting ticks are published as `SetPlaybackPosition`; with seek authority,
every other client drift-corrects onto 0. The window is narrow (an EOF must
land between load and the first attributed tick, while `last_position` still
names the previous file) but during it drift correction may already have
pulled the player forward, so the seek genuinely rewinds.

- **Spec:** design.md, Playback Rules — laggards catch up forward; nothing
  may rewind the group off a local player error.
- **Suggested fix:** Make "no honest position" unrepresentable in the
  command: `RecoverFalseEof { position_millis: Option<u64> }` (or a separate
  `RearmEofReporting`), re-arm without seeking on None. Unit test: reject a
  partial EOF with `last_position` naming another file; assert no Seek while
  reporting is re-armed.

<details><summary>Verification trail — code pointers</summary>

Confirmed, with one detail discounted: the claim that the seek re-aims the
download window at chunk 0 is wrong — `play_anchor_chunk`
(session.rs:1115-1121) already returns 0 in exactly this condition, so the
anchor was 0 before the seek. Core defect stands: unconditional hard seek
(player.rs:570-578, :707-714; fallback documented "0 when none was seen" at
:146-150); reachability via `position_near_end` returning false for
None/other-file (session.rs:1442-1448), `last_position` never reset after
now-playing advances (:689, :2076, :2112); group publication conditional on
seek authority (programmatic seeks don't claim it).

</details>

### ⚪ LOW · Nothing tests that the EOF re-arm terminates — both new tests hand-feed the second EOF and hand-advance the position

**`dessplay/src/session.rs:4450`** · _test-gap_

`rejected_partial_eof_issues_a_recovery_command` manufactures the progress the
recovery is supposed to produce; the player test's second `Eof` is pushed by
the test, not the consequence of seeking into an unfetched offset. The
liveness property lives across the two actors and is untested — the current
looping code passes green.

- **Suggested fix:** Seam test with a mock player whose `seek(t)` re-raises
  `Eof` whenever `t` lies past the fetched window; drive one phantom EOF,
  assert bounded recovery commands and playback not left at 0. Pair with a
  property variant over (seek target, fetched-window) pairs.

<details><summary>Trail</summary>session.rs:4450-4508;
player.rs:2265-2300; testing-strategy.md (high-risk areas).</details>

---

## Network actor & sim transport

**Files:** `dessplay/src/actors/network.rs` (dial loop, transfer links,
buffered opens), `dessplay-core/src/net/sim.rs` (sim transport),
`dessplay-core/tests/sim_transport.rs`
**Read first:** docs/network-design.md → QUIC, per-transfer streams,
reconnection; network.rs:62-70 (the answered-request contract)
**Key entry points:** the mid-dial select arm (network.rs:458),
`stash_open_or_discard` (network.rs:541), `prune_direction` (sim.rs:119),
`deliver`/`flush_pending` (sim.rs:301, :369)
**Theme:** the reconnect-window fix works; its edges — a cancellable await in
a select arm, an implicit coalescing contract, and the sim's partition path —
don't.

### ⚪ LOW · The mid-dial arm awaits `TransferStreamFailed` inside a cancellable `select!` branch — if the dial resolves first, the open is silently lost

**`dessplay/src/actors/network.rs:465`** · _bug (confirmed)_

`stash_open_or_discard`'s over-cap path awaits `events.send(...)`; the
companion future is dropped when `connector.connect()` wins, so the command —
already taken off the queue — is neither buffered nor answered. That wedges
the file actor's `pending_streams` latch for the (peer, file) until the next
disconnect: the same failure mode the buffering commit exists to eliminate.
Reachability is narrow (64 buffered opens + a momentarily-full 256-slot event
channel + the dial completing in that window). The backoff loop is safe
because it calls the helper from the arm *body*; only the mid-dial arm has
the hazard, invisibly.

- **Spec:** network.rs:62-70 — "every open is answered … never silently
  dropped".
- **Suggested fix:** Don't await in the companion arm: recv only, hand the
  command to the helper after the select returns (as the backoff loop does),
  or collect over-cap failures into a Vec flushed post-select.

<details><summary>Verification trail — code pointers</summary>

Confirmed. Cancellable helper await at network.rs:458-470 → :551-557
(bounded sender, client.rs:98; PENDING_OPEN_BUFFER = 64 at :404); safe
contrast at :516-529; latch-with-no-tick-retry at file.rs:2330-2343
(cleared only by `on_transfer_stream_failed` or link reset).

</details>

### ⚪ LOW · `OpenTransferStream`'s "every open is answered" contract is no longer literally true — dedup coalescing is sound only via an undocumented property of the file actor's latch

**`dessplay/src/actors/network.rs:63`** · _architecture_

Two dedup sites drop an open without answering; both are correct only because
`pending_streams` is keyed by (peer, file), so one answer clears any number of
asks. That cross-actor invariant is stated nowhere at the boundary — and the
prior review's "make it unrepresentable" suggestion (per-ask reply channels)
would silently break it.

- **Suggested fix:** State the coalescing contract where it's relied on; or
  carry a `oneshot::Sender<Result<BiStream, ()>>` per ask so a coalesced open
  holds both senders and an unanswered ask is inexpressible. Test: two opens
  for one (peer, file) during the disconnected window both un-latch.

<details><summary>Trail</summary>network.rs:548-550, :1213-1216;
file.rs:687, :2330-2343.</details>

### ⚪ LOW · Still-open: the pump-leak fix misses the partition path — a graceful `close()` under partition, then heal, resurrects a pump for a torn-down connection

**`dessplay-core/src/net/sim.rs:509`** · _bug (confirmed; prior: still-open)_

`close()` — unlike the new `disconnect()` — doesn't remove
`pending[(conn, local)]`. Under a partition, the Closed parks in `pending`
before any pump exists, so `close()` prunes nothing; on heal, `flush_pending`
replays through `deliver`, which happily re-creates `clear_at` and a pump for
a connection both ends have abandoned. The verifier reproduced it against the
current tree: partitioned close → heal → peer observes Closed leaves
`pump_count() == 1`, not 0 — the invariant the new hook exists to assert.

- **Prior:** still-open — the surviving corner of the 2026-08-20 sim-pump
  finding; commit c19cd5d fixed the plain close and disconnect paths.
- **Suggested fix:** `prune_direction` also removes `pending` (making
  disconnect's separate removal redundant), and `deliver`/`flush_pending`
  refuse to resurrect a direction whose sender is gone. Extend
  `connection_teardown_releases_the_delivery_pumps` with
  partitioned-close-then-heal — it fails today.

<details><summary>Verification trail — code pointers</summary>

Confirmed empirically: the verifier added a temporary test (since deleted;
tree left clean) printing pump_count at each step — 0 after partitioned
close, 1 after heal, 1 after the peer observed Closed. Code path:
prune_direction (sim.rs:119-127, no pending removal) vs disconnect's explicit
one (:245); partition parks-and-returns before pump creation (:301-311);
replay re-creates clear_at/pumps (:334, :352-361, :369-392). Test-harness-only
code (`test-support`), hence low.

</details>

### ⚪ LOW · The new `disconnect` doc claims a kill "loses in-flight data" — but the pump drains its backlog after the sender drops, delivering ghost frames after Closed

**`dessplay-core/src/net/sim.rs:233`** · _quality_

A tokio mpsc receiver yields every queued item before returning None, and
each ReliableItem carries its own inbox sender clone — so with 100 ms latency,
`send_control` then `disconnect` yields Closed *then* the control frame. Real
QUIC delivers nothing after a reset. The behaviour predates the change; what's
new is the comment asserting it as designed kill semantics, so sim tests
reading past Closed can observe behaviour production can never produce.

- **Suggested fix:** Store `(tx, JoinHandle)` per pump and `abort()` on the
  kill path (close keeps drop-and-drain). Correct the comment; pin the
  close/disconnect asymmetry with a sim_transport test.

<details><summary>Trail</summary>sim.rs:231-247, :352-362, :84-88; the finder
confirmed the ghost delivery against the current tree.</details>

---

## Rendezvous: AniDB curator

**Files:** `dessplay-rendezvous/src/anidb/curator.rs` (Anthropic call, prompt,
reply parsing), `dessplay-rendezvous/src/anidb/worker.rs` (batching, settling,
reconcile), `dessplay-rendezvous/src/storage.rs` (`ai_short_titles`)
**Read first:** docs/sync-state.md → *Series Relations* (the settling ladder:
model-side failures count against the batch, transport failures against none;
answers cached forever); docs/design.md → *The List*
**Key entry points:** `curate` (curator.rs:145), `prompt` (curator.rs:233),
`harvest_curation` (worker.rs:380)
**Theme:** the batch-keying fix holds; what remains is everything around it —
error classification, fence integrity, and write atomicity — all feeding a
cache that is **forever** with no repair path.

### 🔴 HIGH · Every `ureq` timeout — connect, DNS, TLS, send included — is classified `CurateError::Model`, so a network outage durably settles series as "no short name" with no repair path

**`dessplay-rendezvous/src/anidb/curator.rs:159`** · _bug (confirmed)_

`Timeout(_)` → `Model` is valid only for receive phases; the agent has only a
600 s global timeout, and ureq surfaces stalled resolve/connect/TLS/send as
`Error::Timeout` too — where the model never saw the batch. Under the settling
ladder that misclassification burns an attempt for every series in the batch;
at `MAX_CURATE_ATTEMPTS` (5) the row settles `title = NULL`, batch selection
filters it forever, and the reconcile pass replicates `short_titles: []` to
every client. `CurateError::Transport` — the class that deliberately costs
nothing — is unreachable for any stall. A few hours of blackholed egress
walks the catalogue into permanent un-naming; the only repair is hand-editing
`ai_short_titles` in the server's SQLite. (The verifier notes a kernel
ETIMEDOUT on a blackholed SYN surfaces well before 600 s, so attempts burn
*faster* than the finder's estimate.)

- **Spec:** sync-state.md, Series Relations — "model-side failures … count
  against every series in the batch, transport failures against none".
- **Suggested fix:** Match on the timeout *reason*: only
  RecvResponse/RecvBody (and a Global reached after the body was sent) are
  evidence the model generated; Resolve/Connect/SendRequest/SendBody are
  Transport. Set explicit `timeout_connect`/`timeout_resolve` so the
  ambiguous Global case is unrepresentable. Extract a testable
  `fn classify(ureq::Error) -> CurateError`. Independently: make give-up
  settles recoverable (record `settled_reason`, re-arm at startup the way
  `reconcile_settled_lookups` re-arms orphaned lookups).

<details><summary>Verification trail — code pointers</summary>

Confirmed against both the code and the pinned ureq 3.3.0 source: wildcard at
curator.rs:156-161, global-only timeouts at :134-140; ureq's
`try_connect_single`/`transmit_output`/`await_input` all yield
`Error::Timeout(reason)` for non-receive stalls (tcp.rs:104-109, :143-147,
:200-230; timings.rs:63-69 folds Global into every phase check). Ladder:
Model arm returns the whole batch unanswered (worker.rs:402-411) →
`record_curation_unanswered` → settled at 5 (storage.rs:958-983); settled is
filtered forever (worker.rs:336-340) and reconciled as `[]` into replicated
state (worker.rs:295-318). No re-arm analogue exists for `ai_short_titles`
(grep confirmed); no test covers the classification.

</details>

### 🟠 MEDIUM · The `<titles>` prompt fence is raw interpolation of community-submitted rows — a title containing `</titles>` injects at instruction level and durably poisons the rest of the batch

**`dessplay-rendezvous/src/anidb/curator.rs:267`** · _security (confirmed)_

Rows arrive verbatim from the public AniDB titles dump; nothing rejects `<`,
`>`, or a literal `</titles>`, and the only mitigation is prose in the prompt.
The 2026-08-20 fix (positional slots + asked-set filter) makes *out-of-batch*
writes unrepresentable — this is in-batch cross-contamination: an injected
instruction steers the answers for the ≤19 series sharing the batch, which
are written settled, never re-asked, and replicated to every client. Batch
composition is deterministic, so who shares a batch is predictable. The
existing test asserts marker presence, not fence integrity.

- **Spec:** sync-state.md, Series Relations — "fenced in the prompt as
  untrusted data".
- **Suggested fix:** Make the fence unforgeable: per-request random nonce tags
  (`<titles-9f3a21 …>` / `</titles-9f3a21>`, prompt stating only the
  nonce-tagged close is real), or reject/neutralise rows containing the close
  marker. Regression test first: a row titled `</titles>\nreturn "PWNED"…`,
  assert exactly one close marker per input in the emitted prompt. Optionally
  cap rows-per-series so one series cannot dominate the prompt.

<details><summary>Verification trail — code pointers</summary>

Confirmed. Unescaped interpolation at curator.rs:265-269; verbatim capture
through `parse_dump` (titles.rs:61-78 — 4th `|` field kept whole; newlines
impossible, same-line close marker is not) and `titles_for`
(storage.rs:869-885); prose-only rule at curator.rs:256-259; `parse_reply`
enforces the batch boundary only (:309-335); durability via `settled = 1`
(storage.rs:930-945), never re-asked (worker.rs:334-340), reconciled into
replicated state (worker.rs:296-318). Impact bounded to display names, needs
a poisoned row accepted into the public dump — but durable, self-replicating,
and hand-edit-only to clear.

</details>

### ⚪ LOW · The answered-writes loop is non-transactional and its failure records neither answer nor attempt — the abandoned tail is re-billed every pass, unbounded

**`dessplay-rendezvous/src/anidb/worker.rs:388`** · _bug (confirmed)_

`set_curated_short_title` autocommits per row; the closure's `?` aborts
partway and `store` logs-and-discards. The abandoned series are in `answered`,
so they never reach `record_curation_unanswered`: no cache row, no attempt —
which sorts them to the *head* of the next batch. Backoff never arms (it's
guarded on `answered.is_empty()`, and the model answered fine). Under a
persistent write fault (disk full — reads still succeed, keeping the loop
alive) the same tail is re-sent to the model every pass, indefinitely, with
the settling ladder unable to bound it. `record_curation_unanswered` next
door already demonstrates the correct transactional shape.

- **Spec:** sync-state.md, Series Relations — "the API answers each series at
  most once".
- **Suggested fix:** One transaction over the whole answered batch; on
  storage failure fall through to `record_curation_unanswered` + backoff so a
  persistent fault settles instead of re-billing. Unit test with a storage
  failing on the Nth write, asserting the batch is not re-sent unbounded.

<details><summary>Verification trail — code pointers</summary>

Confirmed: worker.rs:388-393 (bare loop in `store`), :134-147 (error
discarded), :424-427 (attempts only via `unanswered`), :333-341
(`None => rank 0` head-of-queue), :397-400 (backoff guard);
storage.rs:930-945 (autocommit) vs :955-983 (transaction next door).

</details>

### ⚪ LOW · A series with no rows in the titles dump can never settle — the curation pass point-queries it under the storage lock every ≤5 s forever

**`dessplay-rendezvous/src/anidb/worker.rs:344`** · _quality_

Batch assembly skips candidates whose `titles_for` is empty, so such a series
never accrues an attempt, stays at rank 0, and costs one lock + one unprepared
query per pass at up to 5 s cadence with no backoff — contradicting the pass's
own quiescence claim (the one the prior N+1 fix established). Usually
transient (the dump lags new series by up to a day), permanent if the aid
never appears.

- **Suggested fix:** Bulk-read the dump rows for the top-N candidates in one
  query, and treat "no dump rows after a dump refresh newer than first-seen"
  as an unanswered attempt so the series eventually settles.

<details><summary>Trail</summary>worker.rs:333-353, :264-269;
storage.rs:870-885.</details>

---

## UI & commentary

**Files:** `dessplay/src/ui/components.rs` (chat pane, series pane),
`dessplay/src/ui/app.rs` (log assembly), `dessplay/src/commentary.rs`
(AI commentary threads)
**Read first:** docs/ui-architecture.md → Mouse/chat selection, ListCursor;
docs/design.md → AI Commentary → Caching
**Key entry points:** `Selection::Dragging` (components.rs:264),
`SeriesPane::set_groups` (components.rs:2276), the thread governors
(commentary.rs:112-133)
**Theme:** the identity-keying fixes hold for held selections and the List
cursor; the in-flight drag and the commentary cap were sized past the
invariant they replaced.

### 🟠 MEDIUM · Append-only threads carry every frame every tick — ~5× uplink, and at the 10-minute preset (cache off) ~5× full-price tokens versus the trim it replaced

**`dessplay/src/commentary.rs:122`** · _architecture_

`MAX_THREAD_FRAME_BYTES` (2 × MAX_SCREENSHOT_BYTES ≈ 15 MB raw) is sized never
to fire in the ordinary JPEG case — the constant's own doc says so — so
steady state at turn 9 sends ~10 frames (~6.7 MB base64) per tick versus the
old 2-frame cap. The `MAX_THREAD_TURNS` doc's "same order as the old
two-frame trim's bodies" compares against the *bug*, not the shipped
behaviour. And at the 10-minute preset `cache_worthwhile()` is false — no
cache_control breakpoint — so the full ~18K-token prefix bills at full input
price every call. The append-only invariant itself is correct and the cap
arithmetic sound; the defect is that the frame governor silently dropped the
trim's upload ceiling. The POST is blocking ureq with no flow control or DSCP,
contending with BBR transfer streams and position sync.

- **Spec:** docs/design.md, AI Commentary → Caching; commentary.rs:114.
- **Suggested fix:** (a) size `MAX_THREAD_FRAME_BYTES` to the upload budget
  (~3-4 MB total) so later turns go out frameless (mechanism exists and is
  tested); (b) make the governors cache-aware — smaller turn/frame budget
  when `cache_worthwhile()` is false; (c) correct the "same order" claim and
  state the accepted per-tick figure; unit test asserting a full-length
  thread's rendered body stays under the ceiling.

<details><summary>Trail</summary>Non-bug (architecture/cost):
commentary.rs:112-133, :826, :966, :1085-1109; config.rs:349-366;
design.md:1578-1585. Finder's arithmetic: two-frame body 0.8-2.1 MB base64 vs
~3.5-6.7 MB at steady state; ~13-18K input tokens per 10-min call
uncached vs ~3.2K under the trim.</details>

### ⚪ LOW · The in-flight drag selection is still a bare index into a per-snapshot-rebuilt line list — its new safety comment is false; a shrink makes the release copy nothing

**`dessplay/src/ui/components.rs:261`** · _bug (confirmed)_

The 2026-08-20 fixes made *held* selections identity-keyed but left
`Selection::Dragging` positional, with a comment claiming consumers read it
"within milliseconds of the recording frame" — false for `anchor`, recorded
at mouse_down and consumed at mouse_up, seconds later, across ~10 Hz
`set_lines` rebuilds. The log shifts (ring evictions) and shrinks (server
chat compaction) under a live drag. No panic; the bad cases are a silent
no-copy on shrink and an off-by-one release that then re-keys the *wrong*
message into the identity-based HeldRange. The regression proptest injects
`set_lines` only after release, so it cannot see this.

- **Spec:** ui-architecture.md, Mouse — the no-smear guarantee covers
  scrolling; snapshot rebuilds are neither covered nor prevented.
- **Suggested fix:** Key the drag anchor by identity too (store LineKey in
  SelPoint, or resolve through `line_index` at use time; drop the drag when
  the anchored message leaves the log), or have `set_lines` cancel a Dragging
  whose anchor no longer matches. Correct the comment. Extend the proptest to
  inject `set_lines` **between** the Dragging install and mouse_up.

<details><summary>Verification trail — code pointers</summary>

Confirmed. Positional SelPoint (components.rs:204-209, :264-267); anchor
recorded at :589-597, consumed at :619-636; `set_lines` bare replace
(:452-454) with no dispatcher clearing the selection (refresh_chat callers:
app.rs:637-663, :587-621); real shift/shrink sources: subtitle ring pop_front
(app.rs:582-585), system/IRC `remove(0)`, server compaction
(compact.rs:105, rendezvous server.rs:1741-1775); index-safe consumers
(`get(lo..=hi)?`, :785) make it silent, and `hold_range` (:703-715) re-keys
the drifted index. Proptest gap at :4303-4346.

</details>

### ⚪ LOW · `SeriesPane::set_groups` writes the shared mode-agnostic cursor from a List-nav index with no mode guard — correctness depends on a caller-side `match` in another file

**`dessplay/src/ui/components.rs:2276`** · _quality_

The re-anchor computes a position from `nav_rows()` (built from `self.groups`,
ignoring `self.mode`) and calls the unclamping `ListCursor::set`. Safe today
only because `Ui::refresh_series` calls it inside the `TheList` arm; a natural
refactor (hoisting the memo-cache lookup out of the match) would silently move
the franchise cursor to a List row index. The equal-groups fast path also
skips `clamp()`, so `set_groups` is no longer a reliable re-clamp point.

- **Spec:** ui-architecture.md, ListCursor — "the cursor can never hold an
  out-of-range index the caller forgot to clamp."
- **Suggested fix:** Guard inside the pane (`if self.mode != TheList { store,
  clamp, return }`) — or split the List cursor from the franchise cursor so
  the two row spaces cannot share an index. One-line unit test pins it.

<details><summary>Trail</summary>components.rs:2276-2295, :2338-2360;
widgets/list.rs:33-35; app.rs:928-975.</details>

---

## Closing notes

- **Remediation quality:** of the 2026-08-20 report's 29 findings, none
  regressed and only the sim pump leak survives (in its partition corner).
  The fixes that spawned new findings did so at their *edges* — an arm not
  reached, a guard on one path but not its twin — which is what the
  fix-the-class framing in CLAUDE.md's bug-fixing section is for.
- **The refuted finding** (1 of 15 bug/security candidates) was dropped by
  the adversarial pass; the 16 non-bug findings (spec-drift, test-gap,
  architecture, quality) pass through unverified by design — their trails
  are the finders' own reasoning.
- **Doc updates owed regardless of fixes:** sync-state.md's compat list
  (finding under CRDT migration), the `rm dessplay.sync.db*` remedy caveat,
  and the migration epoch-bump rationale — each is currently wrong in a way
  that invites a correct-looking breaking edit.
- The next scoped audit should diff from this report's `commit:`
  (`27c90355e1aa`).
