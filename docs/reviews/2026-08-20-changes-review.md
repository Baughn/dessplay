# DessPlay Codebase Review — Remediation Report

_Generated 2026-08-20 by a multi-agent audit (7 Opus finder agents, one per
area; every bug/security finding then independently adversarially verified).
30 raw findings → 29 kept (15 bug/security confirmed, 2 uncertain, 12 non-bug
findings carried on the finder's own reasoning), 0 refuted._

_Revision: jj change `roolsspwyqxmmnnuyvvxxrmwvywtslwr` · commit
`22d85552028de11bd93e7b1e4aa497d71e985f4a`. Scope: changes since
`0c48f0b36682` (the 2026-08-12 review's anchor)._

<!-- audit-revision
mode: scoped
commit: 22d85552028de11bd93e7b1e4aa497d71e985f4a
jj-change: roolsspwyqxmmnnuyvvxxrmwvywtslwr
base: 0c48f0b36682076e5e06c0a4567b00b36d8107b0
generated: 2026-08-20
-->

> Project norms (CLAUDE.md): write the regression test *first* and confirm it
> fails — property/fuzz tests preferred over one-shot unit tests; a fix that
> makes the bug class unrepresentable may skip the test. Run `cargo fmt`
> before committing, and verify with `cargo clippy --workspace --all-targets
> -- -D warnings` plus the full workspace test suite.

## Executive summary

This window (~14k inserted lines: the anchored download-order / shared
chunk-budget overhaul, Phase 31 ad-hoc files and hash-first resolution, the
Phase 32/33 List rework with the AniDB short-title curator, chat
selection/clipboard, and the snapshot-fixture pinning) cleared most of the
last review's fix-first list — the marquee wedge, spoiler keying, the
snapshot compat guards, and the urgent-set scheduling all landed. The
verifiers refuted nothing they checked. The standouts:

- **The snapshot migration corrupts the authoritative store on the next
  deploy.** `upgrade_relations_map` re-dots every `series_relations` entry
  under one constant actor with position-derived counters, so two
  independently-migrated replicas assign *the same dots to different keys* —
  the exact double-spent-dot hazard `crdts` documents. The verifier
  hand-traced the merge: server {100,200,300} merged with client {200,300}
  yields {300}, both directions converge on the corrupted value, and the view
  hashes agree so no divergence alarm fires. The first client connect after a
  v11+ deploy silently shreds the relations graph on tsugumi (fix-first 1).
  The companion test-gap explains why it ships green: every fixture holds
  exactly one relations entry, the only size at which the bug is invisible.
- **One regression.** The 2026-08-12 fix "make `loaded` carry the path"
  killed the adopt-at-a-new-path case but not the browse-import case, which
  deliberately lands the payload at the *same* hash-addressed cache path on a
  new inode. Path equality is read as byte identity: the partial flag flips
  false with no reload, mpv keeps demuxing the deleted partial, and its
  truncated EOF once again marks the episode watched and advances the whole
  group (fix-first 2).
- **One still-open.** Last review's fix-first 3 — a dropped `OpenTransfer`
  wedges the transfer — was fixed inside `run_connection` but not in the
  outer reconnect loop, which silently discards commands during
  connect/backoff. The 2 s reconnect window against a 250 ms download tick
  makes the wedge all but deterministic after any wifi blip whose reconnect
  succeeds first try (fix-first 3).
- **The file actor learned to say "never" when it means "not yet".** Two
  independent HIGHs share this shape: the Phase 31 stale-resolve guard
  swallows an honest NotFound whenever `local_files` holds a dead manual
  mapping (which is routine, since mappings are insert-only and never
  existence-checked at load), permanently wedging the entry's resolution; and
  `CannotServe` for a not-yet-registered library file lands in the
  downloader's `denied` set, which has no expiry — a peer solicited during
  its post-restart resolve window is barred for the download's lifetime.
- **The curator trusts its model too much.** The reply cache writes every
  aid the model names — including aids never in the batch — so one
  hallucinated or prompt-injected row permanently poisons an unrelated
  series' display name across every client; and a batch that never resolves
  head-of-line-blocks all curation while re-sending the same Opus request
  every 5 seconds (no backoff on the omission path) or every 10 minutes
  (forever, on the refusal path).

Recurring themes, each worth a structural pass rather than point fixes:

1. **Ask-once latches with lossy answers.** `pending_streams` (transfer
   opens), `pending_resolve` (session resolution), `eof_reported` (player),
   `denied` (downloader), `blocker_overlay_sent` (OSD dedup): each is armed
   on send and cleared only by an answer that can be lost or never come.
   Either make the un-answered state unrepresentable (reply channels the
   type system won't let you drop) or give every latch a re-arm path.
2. **Caches treated as truth.** `local_files` seeded from never-verified
   manual mappings; the hash index resolving files under removed media
   roots; the curator cache accepting out-of-batch rows; path equality
   standing in for inode identity. Evidence needs a liveness check at the
   point of use.
3. **Positional indices into lists rebuilt every snapshot.** The chat
   selection (panic + silent retarget) and the List cursor both aim by row
   index at ~10 Hz rebuilt derivations. `SpoilerKey` and the Playlist's
   `selected_hash` are the in-tree identity-keyed precedents; adopt them.

### Fix-first order

1. 🔴 Snapshot migration destroys `series_relations` on merge —
   `dessplay-core/src/state.rs` (`upgrade_relations_map`): preserve original
   dots (or derive the dot from the key), plus the migration-merge
   convergence property test. **Deploy-blocking for v11+.**
2. 🔴 **Regression**: browse import at the same cache path plays the deleted
   partial and its false EOF advances the group —
   `dessplay/src/session.rs` (`note_local_file`): a partial→verified
   transition always reloads; carry inode identity in `LoadedFile`.
3. 🔴 **Still-open**: `OpenTransferStream` discarded during
   reconnect/backoff wedges the (source, file) transfer —
   `dessplay/src/actors/network.rs` (outer `run` loop): answer every open
   with `TransferStreamFailed`, or make the reply channel undroppable.
4. 🔴 Stale-resolve guard latches `pending_resolve` forever —
   `dessplay/src/actors/file.rs` (`Done::Resolved` guard): gate on a live
   file, existence-check manual mappings at load, prune them on loss.
5. 🔴 `CannotServe` for "not registered yet" is a permanent denial —
   `dessplay/src/actors/file.rs` (`serve_block_hashes`) +
   `download.rs` (`denied`): distinguish "never" from "not now"; give
   `denied` an escape hatch.
6. 🔴 Shift-Up on a held chat selection can panic the TUI —
   `dessplay/src/ui/components.rs` (`step_line`): clamp now; re-key the
   selection by message identity (also fixes the silent retarget).
7. 🟠 Curator caches out-of-batch aids — poisoned names, permanently —
   `dessplay-rendezvous/src/anidb/worker.rs` (`curate_short_titles`): filter
   answers against the batch; make extras unrepresentable in the reply type.
8. 🟠 Stuck curation batch retries forever (5 s cadence on omission) —
   same file: arm the backoff on zero progress, settle after N attempts,
   rotate the batch window.
9. 🟠 List cursor retargets across snapshot rebuilds —
   `dessplay/src/ui/components.rs` (`set_groups`): re-anchor by
   `ListEntryId`.
10. 🟠 Partial-EOF handling parks the client with no recovery —
    `dessplay/src/session.rs` + `actors/player.rs`: re-issue Load on a
    rejected EOF (clears `eof_reported`); stop advertising
    `DownloadingPlayable` after a partial `LoadFailed`.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| Transfer & network plane | 1 | 2 | 2 | 5 |
| Partial-file playback & session wiring | 1 | 2 | 1 | 4 |
| File actor: ad-hoc files & resolution | 2 | 1 | 2 | 5 |
| CRDT state & snapshot migration | 1 | 1 |  | 2 |
| Rendezvous: AniDB curator & storage |  | 2 | 4 | 6 |
| UI: chat selection, The List, clocks | 1 | 2 | 3 | 6 |
| AI commentary |  | 1 |  | 1 |
| **Total** | **6** | **11** | **12** | **29** |

---

## Transfer & network plane

**Files:** `dessplay/src/actors/network.rs` (QUIC actor, reconnect loop),
`dessplay/src/actors/file.rs` (stream queueing), `dessplay/src/download.rs`
(scheduler: shared budget, urgent set), `dessplay-core/src/net/sim.rs`
(simulated transport), `dessplay/tests/{transfer,download_props}.rs`,
`dessplay/fuzz/fuzz_targets/download_scheduler.rs`
**Read first:** network-design.md → *Scheduling: request window, snub, urgent
set* and *Transfer Resumption*; the transfer flow-control proposal
**Key entry points:** `network::run` (outer connect/backoff loop),
`queue_for_stream` (the "already asked" latch), `plan_all`/`plan_requests`
(budget + urgency)
**Theme:** the overhaul's request/answer contracts hold inside a connection
but not across the reconnect boundary, and the urgent set trusts a stale
playback anchor.

### 🔴 HIGH · `OpenTransferStream` is silently discarded during reconnect/backoff — the transfer wedges until the next disconnect

**`dessplay/src/actors/network.rs:450`** · _bug_ · confirmed

`NetworkCommand::OpenTransferStream`'s contract (network.rs:57–77) promises
"every open is answered — with the stream, or with `TransferStreamFailed` —
never silently dropped", and `queue_for_stream`'s emptiness latch
(file.rs:2151–2170) is sound *only because* of that promise. The contract is
honoured inside `run_connection` (buffered pre-AuthOk, full-buffer answered
with `TransferStreamFailed`) but not in the outer `network::run` loop: the
connect-in-flight companion arm (network.rs:444–459) and the backoff arm
(:498–505) discard every non-Shutdown command with only a debug log.

The recovery path cannot cover it: `on_transfer_link_reset` is driven only by
`NetworkEvent::Disconnected`, which is emitted *before* the discard window
opens (network.rs:480) and re-emitted only when a reconnect attempt *fails*
(:491–495). So: disconnect → reset clears `pending_streams` → 250 ms later
the download tick re-plans, `queue_for_stream` latches and emits
`OpenTransfer` → the command lands in the discard window (2 s backoff + dial
vs. a 250 ms tick — all but guaranteed) → reconnect succeeds first try → no
further reset, and nothing ever re-arms the latch. The peer advertises
`Downloading` and gates the group indefinitely.

- **Spec:** network-design.md, Transfer Resumption — "the next chunk request
  toward a source simply opens a fresh one"; design.md, Presence — "Brief
  glitches (< 30s) are invisible."
- **Prior:** **still-open** — this is the surviving half of the 2026-08-12
  fix-first 3; the in-connection half was fixed.
- **Suggested fix:** Regression test first over the `pump_transfer` harness
  (dessplay/tests/transfer.rs): drive a disconnect, swallow the open issued
  during the reconnect window, assert the transfer still completes. Then
  honour the contract in the outer loop — match `OpenTransferStream` in both
  discard arms and answer `TransferStreamFailed` (or buffer into
  `pending_opens` for `run_connection` to drain on AuthOk). Structurally
  better, per the unrepresentable-class norm: give the open a reply channel
  (`oneshot<Result<BiStream, ()>>`) so an unanswered open cannot be
  expressed.

<details><summary>Verification trail — code pointers</summary>

Verifier traced every link: contract at network.rs:62–67; honoured only at
:1147–1166 (inside `run_connection`); discarded at :444–459 and :498–505
(the `name()` arm at :87 exists purely for the discard log). One-shot latch:
file.rs:2157–2169; `pending_streams` cleared only at file.rs:1191, :1205,
:2178/:2198, :2325 — `on_tick` (:1220–1227) never touches it, and snub
traffic funnels back through the same dead queue (file.rs:1620–1624,
download.rs:346–355 shows `Abandon` fires only on store-open failure).
Timing: 2 s `reconnect_backoff` (network.rs:368) vs 250 ms `DOWNLOAD_TICK`
(file.rs:3217). `Disconnected` ordering: network.rs:480 (before window),
:491–495 (again only on a failed attempt) — so the wedge needs exactly one
successful reconnect, the common case. No compensating guard found.

</details>

### 🟠 MEDIUM · The shared-budget invariant has no liveness/non-starvation property test

**`dessplay/tests/download_props.rs:68`** · _test-gap_

The cycle's headline scheduler property — one per-source in-flight budget
walked in strict priority order (`plan_all`, download.rs:874–911) — is
covered only by a single-file liveness property, crash-only fuzzing
(the fuzz target's own header defers liveness to download_props), and four
hand-written single-step unit scenarios (download.rs:1443–1531). Starvation
and budget mis-accounting are this design's classic failure modes, and the
escape hatches (`has_budget` short-circuit, tick-only `urgent_sweep`,
urgent chunks bypassing budget/`assigned`/`max_sources`, never-cleared
`first_requested` stamps) are exactly where they'd hide: break the budget to
per-rank, or forget to charge urgent takes, and every existing test still
passes.

- **Spec:** network-design.md, Scheduling — "The window is a shared
  per-source budget across files … the file the group needs next takes the
  whole window; the rest of the want-set advances on leftovers."
- **Suggested fix:** Generalize download_props.rs to 2–3 concurrent files
  over a shared source set, add `SetPriority` to the `Chaos` enum, and assert
  (a) every file completes with exact bytes after the honest epilogue, and
  (b) at every step, per peer, `Σ in_flight ≤ pipeline_depth + (files in
  endgame) × pipeline_depth`.

### 🟠 MEDIUM · A demoted file's frozen playback anchor makes its chunks "urgent", bypassing the budget the now-playing file needs

**`dessplay/src/download.rs:982`** · _architecture_

Urgency is `in_window(c) && age(first_requested) ≥ urgent_age`
(download.rs:982–992), and urgent chunks bypass the shared budget
(:1074–1085), `assigned`, and the `max_sources` rank cap. But `in_window`
uses `d.play_chunk`, refreshed only via `set_sources` from
`play_anchor_chunk` — which is 0 (or frozen at its old value) for any file
the player isn't positioned in — and `first_requested` stamps are
deliberately never cleared (:256–263). A file the user skipped away from
keeps its old anchor; the moment its window chunks requeue (stream reset,
snub, source flap) they are all older than `urgent_age` and go urgent — up
to a full extra pipeline per source issued for an episode nobody is
watching, halving the throughput of the file actually gating the group.
The scheduler has no notion of "this file gates playback" even though
`set_priority` supplies exactly that; and nothing cancels a download that
left the want-set (`Downloads::cancel` has one caller).

- **Spec:** network-design.md, Scheduling — "a needed chunk turns urgent
  when its absence gates a deadline"; a file behind the cursor has no
  deadline.
- **Suggested fix:** Apply window-age urgency only to the top-ranked file
  (or plumb `now_playing: Option<Ed2kHash>` into `Downloads` beside
  `set_priority`); endgame urgency stays unconditional. Optionally drop
  `first_requested` stamps behind the current `gate_start`. Unit test: two
  files, priority [b, a], `a` demoted with requeued aged window chunks —
  assert the urgent sweep issues nothing for `a`.

### ⚪ LOW · Pending-stream queue overflow drops the *oldest* message — a lost `ChunkRequest` costs a flat ~30 s snub

**`dessplay/src/actors/file.rs:2165`** · _bug_ · confirmed

`queue_for_stream` caps the per-(peer,file) queue at 64 by `queue.remove(0)`
— drop-head. The scheduler has already recorded the evicted request's chunks
in `src.in_flight` and `assigned` (download.rs:1089–1101), so they are never
re-issued to that source and bulk mode won't hand them to another; the only
recovery is the 30 s snub. The justifying comment ("the scheduler re-plans
everything the moment the stream lands") is wrong for this path — the
contrasting failure path (`on_transfer_stream_failed`, file.rs:1184–1198)
requeues correctly. Reachability is low today (needs a long open stall plus
endgame churn) but rises with any lost answer (see the HIGH above).

- **Suggested fix:** Coalesce the queue to the latest ChunkRequest/Cancel per
  (peer,file) so overflow is unrepresentable; failing that, drop-tail or feed
  evicted chunks back through `requeue_source` (note: the finder's suggested
  `requeue_chunks` does not exist — use or extend the existing API).

<details><summary>Verification trail — code pointers</summary>

file.rs:2157–2170 (eviction), :3222 (cap = 64), :1620–1624 (chunk control
rides this path), download.rs:1037–1049 (in-flight excluded from
candidates), :681–707 (snub is the only drain). Verifier confirmed bulk
chunks are also blocked by `assigned` for other sources; urgent chunks can
escape. Severity low agreed on reachability grounds.

</details>

### ⚪ LOW · Sim reliable-delivery pumps leak one task + unbounded channel per connection direction

**`dessplay-core/src/net/sim.rs:318`** · _quality_

`deliver` creates per-`(ConnId, EndpointId)` pump tasks lazily
(sim.rs:311–329) but `kill_connections` (:205–213) and `SimTransport::close`
(:461–466) prune only `state.senders`, never `state.pumps`; `ConnId` is
monotonic so entries never get reused. Long integration/perf runs grow tasks
and memory linearly. Secondary: `kill_connections` bypasses the pump when
sending `Closed`, so a kill's Closed can overtake queued reliable frames —
defensible abrupt-kill semantics, but now an undocumented asymmetry with
`close`.

- **Suggested fix:** `state.pumps.remove(&(conn, endpoint))` alongside the
  senders prune in both paths (dropping the sender ends the task after its
  backlog drains); document or unify the kill-ordering asymmetry.

---

## Partial-file playback & session wiring

**Files:** `dessplay/src/session.rs` (`PlayerWiring` — player↔state glue),
`dessplay/src/actors/player.rs` (mpv actor, EOF/seek latches)
**Read first:** design.md → *File State* and *Playback Rules* (the partial
gate is a deferral, never a terminal state); the 2026-08-12 review's
"Partial-file playback" section — this cycle's findings are its direct
descendants
**Key entry points:** `note_local_file`, `on_download_playable`, the
`PlayerOutput::Eof`/`LoadFailed` arms
**Theme:** the partial-file machinery still conflates identity with path,
and its failure arms drop state without arranging recovery.

### 🔴 HIGH · REGRESSION: a browse import at the same cache path clears `partial` without reloading — mpv plays the deleted partial and its truncated EOF advances the group

**`dessplay/src/session.rs:1262`** · _bug_ · confirmed

The 2026-08-12 fix keyed the reload guard on (file, path). That kills the
adopt-at-a-new-path case but not the browse-import case, because the import
deliberately lands its payload at the same hash-addressed `<cache>/<hash>`
path the peer download was assembling into: `on_nyaa_import_hashed` →
`adopt_local_copy` → `cancel_redundant_peer_download` unlinks the partial
(file.rs:1801–1809 — mpv keeps its fd on the orphaned sparse inode, so no
`LoadFailed`), `place_in_cache` hard-links the complete payload at the same
path on a *new* inode (:852–866), and `note_local_file` sees `l.path == path`,
takes the in-place-completion branch, flips `l.partial = false`, and issues
no `PlayerCommand::Load` (session.rs:1257–1273; the `on_state` and
`on_resolved` branches repeat the test). With `partial` false, the
`PARTIAL_EOF_EPSILON_MILLIS` gate no longer applies (:2069–2080), so mpv's
EOF on the truncated orphan is forwarded as `ReportEof`: the episode is
marked watched, now-playing advances, and the whole group pauses mid-episode
— the exact failure the last review's fix-first 1 fixed, resurrected through
a different door. The test `in_place_download_completion_does_not_reload`
(session.rs:4166–4189) locks in the conflation.

- **Spec:** design.md, Download Cache and Retention — "A verified copy
  cancels the peer download … and the entry resolves Ready at the local
  path"; the 2026-08-12 remediation's own rule that a Verified resolution
  re-issues Load.
- **Prior:** **regressed** (partially — the new-path half of the fix holds).
- **Suggested fix:** Regression test first at the `spawn_torrent_rig` /
  PlayerWiring seam (and split `in_place_download_completion_does_not_reload`
  into the two cases it currently conflates). Then: a partial→verified
  transition always re-issues `PlayerCommand::Load` (`if l.partial { Load }`
  in all three same-path branches — mpv restores position, as crash-relaunch
  already relies on). Structurally: carry `(dev, ino)` or the ChunkStore's
  "assembled in place, same fd" signal in `LoadedFile`, so path equality can
  never again stand in for content identity.

<details><summary>Verification trail — code pointers</summary>

Full chain confirmed: partial load at session.rs:1879–1892 (path =
`download_path` = `<cache>/<hash>`, file.rs:1495–1497); adopt guard fires
because the import payload path ≠ download_path (file.rs:1760–1764); unlink
at :1801–1809; new inode at :852–866; `DownloadComplete` → `note_local_file`
same-path branch at session.rs:1257–1273 (no Load), snapshot branch
:1608–1637, resolve branch :1808–1830; EOF gate keyed on `l.partial` at
:2069–2080. No inode/identity check anywhere in `LoadedFile`; no
unload/reload signal from the cancel path.

</details>

### 🟠 MEDIUM · A rejected partial EOF leaves `eof_reported` latched — the client parks at the false EOF with no recovery, even after the download completes

**`dessplay/src/session.rs:2070`** · _bug_ · confirmed

The partial-EOF gate is one-sided: the player actor latches
`self.eof_reported = true` the moment it emits `Eof` (player.rs:862–874),
and the session's drop (session.rs:2070–2081) tells it nothing. The latch
clears only on `Load` (:504) or an attribution-passing `Seeked` (:768) —
neither of which a dropped report triggers. mpv (with `--keep-open=yes`)
parks at the phantom end; when the download completes, `note_local_file`
takes the same-path branch and issues no Load (the finding above), so
`eof_reported` stays latched and even the *genuine* end-of-file EOF is
swallowed — this client never advances the group off the episode while still
advertising the file. Incidental self-healing exists only in the multi-peer
case (a same-file peer running ahead can drag it forward via
`drift_correct`'s programmatic seek).

- **Spec:** design.md, Playback Rules #7 — the gap gate "flips back … and
  the group pauses until the window refills": a deferral, not a terminal
  state.
- **Suggested fix:** Regression test at the PlayerWiring+actor seam
  (partial loaded, PositionTick at 40%, Eof → assert no ReportEof and a
  recovery command; complete the download, assert a real EOF is still
  reportable). Fix: on rejecting a partial's EOF, re-issue
  `PlayerCommand::Load` (or a seek to `last_position`) — clears the latch
  and pulls mpv off the zeros; alternatively move the latch so it only sets
  for accepted reports.

<details><summary>Verification trail — code pointers</summary>

`eof_reported` has exactly six non-test sites (decl :271, inits :334/:1322,
clears :504/:768, read+set :867/:871) — no rejection channel exists. Drop
returns `vec![]` with only a warn (session.rs:2070–2081); completion path
issues no Load (:1257–1274, :1613–1637); `on_download_playable` early-returns
while `loaded` names the file (:1858). mpv park: mpv.rs:98, :378–400.
Mitigation (multi-peer drift rescue) noted at session.rs:1662–1676 +
player.rs:695–731 — absent when this client leads or is the only holder.

</details>

### 🟠 MEDIUM · After a partial `LoadFailed` the client keeps advertising `DownloadingPlayable` — the group plays on while this user sees nothing

**`dessplay/src/session.rs:2091`** · _bug_ · confirmed

The `LoadFailed`-on-a-partial arm clears `loaded`, records the failure
progress, and returns *no* directives (session.rs:2082–2111) — no
availability mutation, unlike the non-partial branch right below it (which
emits `ForgetLocalFile` + `Missing` + a re-resolve). The file actor keeps
writing `DownloadingPlayable` (file.rs:1635–1665), which
`derive::file_block_reason` treats as permitting playback
(derive.rs:155–163). So the client holds no video and simultaneously tells
the group it is fine: the group unpauses and watches without them. For the
canonical trigger — an .mp4 with the moov atom in the unfetched tail — every
retry (gated at +10% progress) fails identically until completion, with only
a `tracing::info!` line. The previous availability-flapping behaviour at
least gated; the backoff fix removed the gating side effect without
replacing it.

- **Spec:** design.md, File State — `DownloadingPlayable` means "can
  actually play"; UI Principles — "No silent long-running work."
- **Suggested fix:** Test first at the PlayerWiring level (load a partial,
  deliver LoadFailed, assert the emitted availability blocks). Fix: emit
  `SetFileAvailability { Downloading { progress_bps } }` alongside recording
  the failure, and suppress the playable advert while the file is in
  `partial_load_failed`; surface the state once in the health line.

<details><summary>Verification trail — code pointers</summary>

Empty directive list confirmed at session.rs:2082–2111; `partial_load_failed`
exists only in session.rs, so the file actor can't know (file.rs:1635–1665
keeps advertising); `DownloadingPlayable` permits at derive.rs:155–163 (and
:669–678 asserts it "never blocks — regardless"); nothing renders locally
(`SetPlaying` gated on `loaded.is_some()`, session.rs:1646–1652). Retry gate:
:1864–1877, `PARTIAL_RETRY_PROGRESS_BPS` = 1000 (:328).

</details>

### ⚪ LOW · `forget_blocker_overlay` re-arms with `(None, None)` — a legal key — so a dropped overlay *clear* is never resent

**`dessplay/src/session.rs:686`** · _bug_ · confirmed

`blocker_overlay_sent` is a bare tuple whose "nothing sent yet" sentinel
`(None, None)` is also a real key (the no-blocker, nothing-loaded state).
When the desired key *is* `(None, None)` and the `SetBlockerOverlay(None)`
`try_send` fails on a full player channel (:2489–2493),
`forget_blocker_overlay` writes exactly the key that failed to deliver, so
the dedup (`!=`, :1700–1708) never retries and a stale "Waiting for …" line
stays on the OSD — precisely what the helper's doc says it exists to
prevent. The neighbouring `last_synced` already uses the correct
`Option<…>`-wrapped shape.

- **Suggested fix:** Make the sentinel unrepresentable:
  `blocker_overlay_sent: Option<(Option<Ed2kHash>, Option<String>)>`,
  matching `forget_last_synced`. Unit test: drive `on_state` to a
  `(None, None)` key, call the forget, assert the next `on_state` re-emits.

---

## File actor: ad-hoc files & resolution

**Files:** `dessplay/src/actors/file.rs` (Phase 31: drag-in adds, servable
ad-hoc files, hash-first resolution), `dessplay-core/src/net/message.rs`
**Read first:** design.md → *File Matching* (hash-first, three-step) and
*Media Library Scanning* (removed-root grace); architecture.md → file actor
**Key entry points:** `resolve_with_cache` (by-hash loop), `Done::Resolved`
(the stale guard), `serve_block_hashes`, `adopt_local_copy` (the seam),
`adopt_hash_added`
**Theme:** Phase 31 added guards and answers that assume `local_files` and
the hash index are truthful; both can be stale, and the answers latch
permanently downstream.

### 🔴 HIGH · The stale-resolve guard swallows the resolution reply — `pending_resolve` latches forever and the entry never resolves, never downloads

**`dessplay/src/actors/file.rs:2342`** · _bug_ · confirmed

Phase 31's guard in `Done::Resolved` returns without emitting
`FileOutput::Resolved` when the resolution is NotFound/HashMismatch and
`local_files` contains the file. But `local_files` is seeded at startup from
the persisted manual-mapping table *unconditionally* (`manual.clone()`,
file.rs:913 — in explicit contrast to the cache-entry loop below, which
stats and prunes), out-of-root drag-ins persist manual mappings routinely
(:1778–1788), and nothing ever removes one (`self.manual` is insert-only;
`lost_local_file` never touches it; there is no `remove_manual_mapping` in
the tree; the scan-side pruner only walks media roots). Meanwhile the
session inserts into `pending_resolve` before emitting `Resolve`
(session.rs:1518–1531) and clears it only on the answer — so a swallowed
answer wedges the entry for the process lifetime: never re-requested, never
in `wanted` (scan adoption can't fire), no download, and the CRDT keeps the
stale `Ready` the previous session left, so peers keep soliciting a copy
that isn't there. `resolve()` itself handles the dead mapping correctly
(:2850–2864) — the actor computes the right NotFound and then throws it
away. Restarting reproduces it identically.

- **Spec:** design.md, File Matching step 3 — "Otherwise: Missing (red in
  UI), and the entry's hash joins the wanted set." A resolve that finds
  nothing must report Missing, not vanish.
- **Suggested fix:** Regression test first: persist a manual mapping to a
  nonexistent path, spawn a fresh actor, send Resolve, assert
  `Resolved{NotFound}` comes back. Fix in layers: (1) the guard requires a
  live copy (`is_some_and(|p| p.is_file())`) — or drops the stale entry via
  `lost_local_file` and lets the honest NotFound through; (2)
  existence-check manual mappings in `Actor::new` the way cache entries are
  reconciled, and make `lost_local_file` prune `self.manual` + storage;
  (3) belt-and-braces, give `pending_resolve` a timeout so no lost answer
  can ever wedge an entry again.

<details><summary>Verification trail — code pointers</summary>

Guard at file.rs:2340–2349 (returns before `wanted.insert` too); seeding
contrast :913 vs :923–945; insert-only manual :1784/:2988;
`lost_local_file` :2095–2121 (leaves `manual`); root-scoped pruner :2658;
dead-mapping fall-through :2850–2864; session latch :1518–1531, clears only
at :1258/:1769. Partial mitigations (peer solicitation hits the
`!path.exists()` serve check; a play attempt emits ForgetLocalFile + fresh
Resolve) require external triggers and the bad row returns next start.

</details>

### 🔴 HIGH · `CannotServe` for a not-yet-registered file is a permanent denial — a peer solicited during its post-restart resolve window is barred for the download's lifetime

**`dessplay/src/actors/file.rs:2019`** · _bug_ · confirmed

Phase 31 turned the "asked for block hashes we don't hold" bail into a
`CannotServe` reply, on the invariant "every Ready is backed by a servable
registration". That invariant is session-scoped: `Ready` is durable CRDT
state, while `local_files` is rebuilt at startup from manual mappings and
live cache entries only (file.rs:910–945) — media-root library files are
absent until resolved or scan-adopted, both lazy, and a resolve may
ed2k-hash a multi-GB file first; worse, watched entries are never resolved
at all (session.rs:1518–1532). On the requesting side, `CannotServe` is
permanent: source removed and inserted into `d.denied`, consulted on every
`set_sources` refresh, "never re-added … for this download's lifetime"
(download.rs:472–484, :244–251) — and new sources are solicited immediately
on their first snapshot (:827–835), exactly the restart race. A download
that loses all sources is never abandoned or rebuilt, so a single-holder
group gates on the entry until the *requester* restarts. Secondarily, the
unheld branch doesn't retract our own `Ready` (unlike the neighbouring
`!path.exists()` branch), so we keep advertising what we just refused to
serve.

- **Spec:** design.md, Download Cache and Retention — the runtime guards for
  "a peer asks for a file we no longer hold" are drop/prune/flip-to-Missing;
  a transient "not registered yet" is not one of them.
- **Suggested fix:** Regression tests both sides first. Distinguish
  "definitively cannot serve under this identity" (the hash-mismatch arm)
  from "not right now": cheapest is to answer nothing for the unheld case
  and emit `Availability::Missing` for the hash (stops the CRDT naming us,
  triggers re-resolve; the requester's snub path re-solicits). If a wire
  answer is wanted, add a non-latching `NotReady` that only re-arms the snub
  clock. Either way give `denied` an escape hatch — clear on a
  Missing→Ready availability transition for that peer.

<details><summary>Verification trail — code pointers</summary>

Serve side: file.rs:2010–2028 (sole condition: no `local_files` entry; no
availability retraction — contrast :2031–2034); startup :910–945; scan
adopts only wanted/active (:2575–2582); resolve may hash (:2876–2894).
Receive side: denied insert download.rs:472–484, skipped forever :399–418,
immediate solicitation :827–835, only Abandon site :350, only cancel site
file.rs:1795–1799. Verifier: severity high defensible (gates group when the
denied peer is the last holder); would also accept medium.

</details>

### 🟠 MEDIUM · Hash-first resolution ignores media-root scope — removed-root files keep resolving Verified and being served for the 7-day grace

**`dessplay/src/actors/file.rs:3530`** · _spec-drift_

The by-hash loop at the top of `resolve_with_cache` accepts any index row
whose (mtime, size) still match disk — it never consults `roots`, unlike the
basename search right below it. Removing a media root prunes `local_files`
and emits Missing but deliberately keeps `hash_cache` rows for the
`REMOVED_ROOT_GRACE` (7 days, for cheap re-adds). With hash-first matching
those retained rows are live evidence: the next restart re-resolves the
entry `Verified(<removed root>/…)`, re-registers it, re-advertises Ready,
and serves chunks from a directory the user explicitly de-listed —
deterministically, for a week.

- **Spec:** design.md, Media Library Scanning — "Removing a root … hides it
  immediately"; "Vanished rows … are not advertised as locally available."
- **Suggested fix:** Give the by-hash step the same scope as the rest of
  resolution: pass the visible-prefix set (current roots + cache dir +
  manual mappings) into `resolve_with_cache` and skip rows outside it (rows
  carry `media_root`, so a `hidden_roots` set is cheap). Regression test:
  index, remove root, re-resolve → NotFound and no serve registration; plus
  the mirror (re-add within grace resolves Verified without re-hashing).

### ⚪ LOW · `set_manual_mapping` bypasses the `adopt_local_copy` seam — the seam built so "a fourth path can't miss it" already has one

**`dessplay/src/actors/file.rs:2989`** · _architecture_

The manual-mapping channel has identical observable effects to the three
channels the seam's doc enumerates, yet writes `local_files` directly and
never cancels an in-flight peer download of the same hash — re-opening the
documented Ready↔Downloading flapping (`Progress` overwrites the fresh
Ready) and letting a later Abandon write Missing over a held file. There is
real design tension (a manual mapping is filename-trusted until
`Done::ManualHashed` confirms content), so this wants an explicit decision,
not a reflexive seam call.

- **Suggested fix:** Decide and encode: route through `adopt_local_copy`, or
  defer adoption to `ManualHashed`'s matching arm so the cancel happens on
  confirmation. Then make the seam enforceable (make `local_files` private
  behind adopt/lose) and fix the seam doc's channel list.

### ⚪ LOW · Drag-in paths persist un-canonicalized — a relative path becomes a permanent cwd-dependent registration

**`dessplay/src/actors/file.rs:1779`** · _quality_

`pasted_file_path` accepts anything `is_file()` (ui/app.rs:2410–2414),
including relative paths; `adopt_hash_added`'s in-root test
(`starts_with(root)`) then always classifies them out-of-root and persists
the literal string as a durable manual mapping. After any cwd change the row
names nothing — which is exactly the input that feeds the
`pending_resolve` wedge above, and it is never pruned.

- **Suggested fix:** `std::fs::canonicalize` once at the boundary
  (`pasted_file_path` and the file browser's FileChosen), falling back to
  the original on failure; unit test that a relative path comes back
  absolute. (The manual-mapping pruning from the HIGH's fix covers the
  legacy rows.)

---

## CRDT state & snapshot migration

**Files:** `dessplay-core/src/state.rs` (snapshot decode/upgrade),
`dessplay-core/tests/migration.rs` + `tests/fixtures/` (frozen bytes)
**Read first:** sync-state.md → *Snapshot Storage / Decoding and Migration* —
note its safety argument ("same rebuild compaction performs") assumes an
epoch bump migration does not have
**Key entry points:** `upgrade_relations_map`, `decode_snapshot_flagged`,
the v7–v10 and untagged-v6 arms
**Theme:** migration output is merged, not adopted — so it must be a merge
homomorphism, and nothing tests that.

### 🔴 HIGH · The snapshot upgrade re-dots `series_relations` under one constant actor — independently-migrated replicas destroy each other's entries on the ordinary reconnect merge

**`dessplay-core/src/state.rs:273`** · _bug_ · confirmed

`upgrade_relations_map` (state.rs:273–290) rebuilds the whole LwwMap with
`map_put(.., SNAPSHOT_MIGRATION_ACTOR, ..)`: every entry gets dot
`(ActorId(u128::MAX), n)` with n from BTreeMap iteration order, and both
migration arms (frozen v7–v10 at :371, untagged-v6 at :455) use it — so on
the first start of a v11+ build, the server and every client independently
re-dot their maps. `crdts::Map::merge` is dot-based: replicas holding
different key sets assign the same dots to *different keys*, the
`DoubleSpentDot` case crdts' own validator names. The verifier hand-traced
the finder's scenario through the real decode path: server {100,200,300}
merged with client {200,300} → keys 100 *and* 200 dropped, both directions
converge on {300}, view hashes agree, no divergence alarm. Both merge
directions sit on the hot reconnect path (sync.rs:749; server.rs:1512
persists the result), so the first client connect after deploying v11+
silently shreds the group's relations graph on the authoritative store. The
sync-state.md justification ("the same view-level rebuild the daily
compaction pass performs") doesn't transfer: compaction is broadcast with an
epoch bump and *replaces* state; migration happens per replica and is
*merged*.

- **Spec:** sync-state.md, Decoding and Migration — the quoted safety
  argument is the drifted spec; this is the double-spent-dot hazard
  `ActorId::session` exists to prevent.
- **Suggested fix:** Property test first: migration must be a merge
  homomorphism — `migrate(a).merge(migrate(b)).view() == a.merge(b).view()`
  over overlapping-but-unequal relation sets, both directions, driven
  through `encode_untagged_v6_for_tests` + `decode_snapshot_flagged`
  (reusing the tests/common generators). Fix: preserve the original dots —
  decode the frozen body into a deserialize-only mirror of
  `crdts::Map`/`Entry` parameterised over the old leaf type and map only the
  leaf values, carrying clocks and `deferred` byte-for-byte. Cheaper
  fallback: make the dot a pure function of the key (e.g.
  `ActorId(md4(series_id))`, counter 1) so every replica assigns identical
  dots regardless of which subset it holds. Only the AniDB worker's slow
  rate-limited refetch would repair the loss today — don't rely on it.

<details><summary>Verification trail — code pointers</summary>

`SNAPSHOT_MIGRATION_ACTOR` state.rs:265; `map_put` :627–642 →
`derive_add_ctx` = clock.inc(actor) (crdts ctx.rs:43–48). Merge semantics:
map.rs:237–305 (entry dropped when other's clock dominates or `common`
empty), vclock.rs:224–236 (intersection = exact actor+counter equality),
:81–91. Hand-trace: key 100 dropped by clock domination, key 200 by empty
intersection, key 300 survives. Epoch guard doesn't fire: storage keeps the
epoch on migration (rendezvous storage.rs:283–311), matching-epoch reconnect
sends StateMerge (server.rs:971–978) → sync.rs:749 → full-state push
(:707–713) → server merge+persist (server.rs:1496–1520). Mitigation noted:
worker re-arms lookups for missing series (worker.rs:376–405) — slow,
rate-limited, and a refetch storm on the authoritative store.

</details>

### 🟠 MEDIUM · Every snapshot fixture holds exactly one `series_relations` entry — the map rebuild is only ever tested at the size where its bug is invisible

**`dessplay-core/tests/migration.rs:395`** · _test-gap_

`rich_sample_state()` writes a single relations entry (migration.rs:187–205)
and every fixture blob encodes that state, so the rebuilt clock is trivially
{MAX:1} everywhere and no ordering question arises. No test anywhere merges
two independently-migrated states — the only way migrated snapshots are used
in production. The convergence machinery (tests/convergence.rs,
merge_props.rs) was never pointed at migration output, which is why the HIGH
above ships green. Secondary: `short_titles` is empty in every fixture, so
the v11 fixture pins the new field only at its zero encoding.

- **Spec:** testing-strategy.md — "High-risk areas get extra coverage: …
  CRDT convergence … reconnection/epoch handling."
- **Suggested fix:** Do **not** regenerate existing fixtures (their bytes
  are the contract). Add a separately-captured multi-entry fixture family
  (≥3 entries, distinct LWW timestamps, one non-empty `short_titles`), or
  drive the multi-entry case through `encode_untagged_v6_for_tests`; then
  add the migration-merge homomorphism property from the HIGH's fix.

---

## Rendezvous: AniDB curator & storage

**Files:** `dessplay-rendezvous/src/anidb/worker.rs` (drain loop, curation
pass), `anidb/curator.rs` (Anthropic call), `storage.rs` (SQLite),
`server.rs` (`SetAnthropicToken`)
**Read first:** design.md → The List / short titles ("the API is consulted
once per series, ever"); sync-state.md → Series Relations
**Key entry points:** `curate_short_titles` (batch → curate → cache →
reconcile), `Curator::curate`, `kv_set(ANTHROPIC_TOKEN_KEY)`
**Theme:** the curator pipeline has no notion of a model that answers
wrongly, partially, or not at all — every such case either poisons durable
state or retries identically forever.

### 🟠 MEDIUM · The curator caches every aid the model names — one hallucinated or injected row permanently poisons an unrelated series' name on every client

**`dessplay-rendezvous/src/anidb/worker.rs:288`** · _security_ · confirmed

`curate_short_titles` writes all `answers` to `ai_short_titles`
(worker.rs:288–293) with no filter against the batch (`asked` is built at
:281 and used only for the omission warning) — directly contradicting the
trait contract ("extras are ignored", curator.rs:62–63). `parse_reply`
accepts any integer aid (the json_schema constrains shape, not identity),
and the prompt interpolates raw community-submitted AniDB title rows with no
fencing or data-not-instructions framing (curator.rs:196–201), so both plain
hallucination (no attacker needed) and a one-line injected instruction can
name an out-of-batch aid. The poison is durable and self-sealing: the
poisoned series now counts as "already asked" (the batch loop skips it,
:265–269), the reconcile pass replicates it into `series_relations` and
every client's List, and there is no delete path short of hand-editing
SQLite. No test feeds a reply containing an out-of-batch aid.

- **Spec:** curator.rs:62–63 — "extras are ignored"; sync-state.md, Series
  Relations — "the API is consulted once per series, ever."
- **Suggested fix:** Regression test first: a MockCurator answering for an
  aid not in the batch → nothing written for it, asked series still
  retried. Fix per the unrepresentable norm: have `curate` return answers
  keyed to its input (positionally aligned `Vec<Option<String>>`, or
  `parse_reply` takes the asked set) so an out-of-batch aid cannot be
  expressed; warn on each dropped extra. Harden the prompt: fence each
  series' rows, state that row text is untrusted data. Note the existing
  mitigation is manual-only: the display substitutes the curated title only
  while `relations.title == entry.name`, so a human rename hides (not
  clears) a poisoned row.

<details><summary>Verification trail — code pointers</summary>

Unfiltered write worker.rs:288–293 (`asked` only feeds the warn loop
:294–298); `parse_reply` validates the short string, never the aid
(curator.rs:228–249); schema :135–168; upsert with no validation
storage.rs:841–850; skip-if-cached :265–269; reconcile :313–322;
`lookup_anime` reads the same cache :544–549; no clear path exists
repo-wide. Injection surface real but the weaker vector; hallucination
suffices. Display mitigation: props.rs:1944–1950.

</details>

### 🟠 MEDIUM · A batch that never resolves head-of-line-blocks all curation and re-sends forever — omissions at 5 s cadence with no backoff

**`dessplay-rendezvous/src/anidb/worker.rs:280`** · _bug_ · confirmed

The batch is the first `CURATE_BATCH` (20) uncached series in
`series_relations` BTreeMap key order — deterministic — and a series leaves
it only by acquiring a cache row, which only a model answer produces.
"Asked but unanswered" is unrepresentable (the `ai_short_titles` schema has
no attempts column) and the success arm never arms `curate_backoff`. So a
well-formed reply that omits aids (or returns `{"results": []}` — the
schema sets no `minItems`) caches nothing and the identical batch is
re-sent at pass cadence — `POLL_MIN` = 5 s when idle, up to ~17k Opus
requests/day for zero progress. A refusal/timeout/4xx arms the 10-minute
backoff and re-sends the same batch forever (deterministic failure ⇒
infinite retry). Either way every series after the stuck batch is starved,
and the only symptom is a warn line.

- **Spec:** sync-state.md, Series Relations — "Failures back off ten
  minutes and cache nothing" (the omission path does neither).
- **Suggested fix:** Regression test first with a MockCurator that answers
  nothing. Then: arm `curate_backoff` whenever `answered ∩ asked` is empty;
  track attempts (an `asked_at`/`attempts` column) and after N unanswered
  attempts settle a durable None with a warn — the same "asked and settled"
  shape `anime_queue.next_attempt = i64::MAX` already uses; rotate the
  batch window so one poisoned series can't starve the catalogue.

<details><summary>Verification trail — code pointers</summary>

Batch selection worker.rs:262–279 over BTreeMap keys (state.rs:1268); only
cache-row writer is the success arm :288–293; schema has no attempts column
(storage.rs:148–153); omission → warn only (:294–298), no backoff; refusal
parse error at curator.rs:209–211, no-minItems schema :146–158; loop cadence
:103–115, `POLL_MIN` :35; backoff arm :300–307. MockCurator exists but no
test covers omit-everything/repeat-forever.

</details>

### ⚪ LOW · The Anthropic call is awaited inline in the AniDB drain loop — a slow curator delays user-visible metadata by up to 120 s per pass

**`dessplay-rendezvous/src/anidb/worker.rs:283`** · _architecture_

`run`'s loop is strictly sequential; `curate_short_titles` awaits the
curator (120 s timeout) before `step` — the part that turns playlist
lookups into replicated metadata — gets to run. During a backfill every
AniDB lookup is spaced by curator latency instead of the AniDB rate limit.
Curation is cosmetic and eventually consistent; metadata is what users wait
on.

- **Suggested fix:** Run the curator on its own spawned task (or hold a
  JoinHandle polled non-blockingly); alternatively run curation only on the
  idle arm after `step`.

### ⚪ LOW · 120 s non-streaming timeout on an Opus call with max_tokens 16k and default thinking — every timeout is a permanent no-progress retry

**`dessplay-rendezvous/src/anidb/curator.rs:37`** · _quality_

The SDKs default to 600 s and recommend streaming at this max_tokens
precisely to avoid request timeouts; on the Opus 5 tier omitting `thinking`
means adaptive thinking runs by default, eating into the window. A
systematically slightly-too-slow batch times out → 10-minute backoff → the
identical batch, indefinitely, billing every aborted generation.
`stop_reason == "max_tokens"` is also unchecked, so truncation surfaces as
a generic parse error.

- **Suggested fix:** Raise the timeout toward 600 s or shrink
  `CURATE_BATCH` so generation is reliably short; branch on
  `stop_reason == "max_tokens"` with a distinct error. (The batch-rotation
  fix above removes the "identical batch" half.)

### ⚪ LOW · The server DB now holds a live Anthropic key in plaintext with default file modes, and the pre-migration backup copies it

**`dessplay-rendezvous/src/storage.rs:216`** · _security_ · **uncertain**

`SetAnthropicToken` upserts the client's API key into the `kv` table;
`ServerStorage::open` creates dir and DB with no mode restriction (0755/0644
modulo umask), and `backup_pre_migration`'s `VACUUM INTO` copy persists a
since-rotated token indefinitely. The verifier confirmed every code-level
claim but found the headline exposure largely blocked in production: the
tsugumi unit runs with `DynamicUser` + `StateDirectory`, so the DB lives
under `/var/lib/private` (0700, root-owned) and other local accounts cannot
traverse to it. The gap is real for manual runs / `default_path()` and as
defense-in-depth — and the codebase already has the 0600 idiom in
`tofu.rs:134–135`.

- **Prior:** kept despite the deployment mitigation — the code defect and
  the manual-run exposure are real; flagged uncertain per the lenient rule.
- **Suggested fix:** chmod 0700 on the data dir and 0600 on the DB and
  `.bak` in `open`/`backup_pre_migration` (two lines, matching the tofu
  precedent) — or record the DynamicUser reliance in design.md's rendezvous
  section so the config can't be changed out from under it.

### ⚪ LOW · The quiesced curation pass is N+1 over series_relations — ~2 unprepared point queries per series every ≤5 s, under the storage lock

**`dessplay-rendezvous/src/anidb/worker.rs:314`** · _quality_

Both the reconcile loop and the batch-selection loop do a per-series
`with_storage` lock + unprepared `query_row`; steady state with a token is
~2 × N locked queries every 2–5 s forever, on the same mutex `save_state`
uses — while the pass's own doc claims "nothing runs but cheap SQLite
reads", and `apply_series_hints` next door already demonstrates the bulk
pattern.

- **Suggested fix:** One `SELECT aid, title FROM ai_short_titles` per pass
  into a `BTreeMap`, driving both halves.

---

## UI: chat selection, The List, clocks

**Files:** `dessplay/src/ui/components.rs` (`ChatPane` selection,
`SeriesPane`), `dessplay/src/ui/app.rs` (merged log, clock, snapshot apply),
`dessplay/src/ui/props.rs` (`list_groups`), `dessplay/src/run.rs`
(clipboard)
**Read first:** ui-architecture.md → Mouse support (selection is
text-coordinate-based); design.md → The List → UI Integration; memory:
*TUI lag → franchises recompute*
**Key entry points:** `step_line`/`extend_selection`, `set_lines`,
`SeriesPane::set_groups`, `Ui::clock`
**Theme:** positional indices into per-snapshot-rebuilt lists — one panic,
one silent retarget, one mis-aimed cursor — plus a clock latch that freezes
animators.

### 🔴 HIGH · Shift-Up on a held chat selection panics if the log shrank — takes down the whole TUI

**`dessplay/src/ui/components.rs:597`** · _bug_ · confirmed

`extend_selection` passes the held `SelRange`'s stored line index straight
to `step_line`, whose up-branch is
`(0..from).rev().find(|&i| !self.lines[i].separator)` — a raw index bounded
by the *stale* `from`, not by `len()`. Every other consumer is
`.get()`-guarded; the "harmless by construction" doc at :168–176 covers the
highlight and the copy, not this. `set_lines` replaces the vec on every
snapshot without touching the selection, and the synced chat genuinely
shrinks (server compaction keeps only the trailing `chat_keep` messages and
broadcasts; a post-compaction reconnect delivers the same shrunk state).
Shift-Up is the one key not routed through `clear_selection`
(app.rs:1134–1149), the hold window is 5 s, and there is no `catch_unwind`
around the UI loop — the finder reproduced the panic through the public
`Ui::handle` interface. Rare in practice, but an unguarded index that exits
DessPlay mid-session.

- **Spec:** ui-architecture.md, Mouse support — nothing permits a snapshot
  to crash the client.
- **Suggested fix:** Clamp now (`.get()` in `step_line`) as the stopgap.
  The structural defect is the positional selection — shared with the two
  findings below: key the held range by message identity
  (millis + sender + text hash — `SpoilerKey`'s established triple) and
  resolve to indices at use time, dropping the selection when the line is
  gone. Regression: the reproducer plus a proptest applying arbitrary
  `set_lines` between release and Shift-Up/Down, asserting no panic.

<details><summary>Verification trail — code pointers</summary>

components.rs:595–601 (up-branch indexes by stale `from`; down-branch safe
by accident), :575–591, :381–383 (bare `self.lines = lines`), :649–691
(contrast: guarded copy_text); app.rs:866–867 (rebuild per snapshot),
:1134–1149 (Shift-Up bypasses clear); compact.rs:105 (chat truncated);
shell.rs:195–300 (no catch_unwind). Grep confirmed no snapshot-side
selection reconciliation exists.

</details>

### 🟠 MEDIUM · A held selection silently retargets when the merged log shifts — Shift-Up/Down copies messages the user never selected

**`dessplay/src/ui/components.rs:172`** · _bug_ · confirmed

Same root cause as the panic, different consequence. The merged list is not
append-only: `merged_chat` concatenates synced chat + three 100-entry local
rings (system, IRC, Intermixed subtitles) sorted by millis, and subtitles
carry *arrival* millis (props.rs:502) so they interleave with recent chat.
Once a ring saturates, every new entry evicts an early-sorting old one and
every later index shifts down by one — in Intermixed mode during dialogue,
roughly once every few seconds, well inside the 5 s hold window. The
highlight jumps to a neighbouring message, and `extend_selection` re-copies
from the shifted indices to the system clipboard with no indication — in a
pane whose point includes spoiler-safety, silently copying an adjacent line
is a correctness problem.

- **Spec:** design.md, Mouse support — "Shift-Up/Down extend *the
  selection*" — the extension must extend what the user selected.
- **Suggested fix:** The identity-keyed selection from the panic finding
  covers this; one fix, two findings. Regression test: hold a partial
  selection, push one subtitle into a saturated ring, re-render, assert the
  highlight still covers the same message text.

<details><summary>Verification trail — code pointers</summary>

SelRange purely positional (components.rs:161–190, filled from render rows
:246–271); rings: app.rs:556–565 (subtitle pop_front), :585–587/:596–599
(remove(0)); merged sort app.rs:625–645; both consumers read the shifted
index (`copy_text` `.get()` prevents the panic, not the wrong copy).
Preconditions confirmed ordinary; day-separator insertion and saturated
system/IRC logs are additional shift sources.

</details>

### 🟠 MEDIUM · The List's cursor is a bare row index over groups rebuilt every snapshot — `n`/`e`/`l`/Enter can act on the wrong entry

**`dessplay/src/ui/components.rs:2115`** · _bug_ · confirmed

`set_groups` only clamps; `nav_rows()` flattens headings and entries into
one positional space and every action resolves through it. The group vector
is rebuilt on every snapshot (~10 Hz during playback; the TheList arm has no
franchise-style cache) and the order is volatile by design: one "Watching —
⟨user⟩" group per committed user (empty groups dropped — a peer connecting,
aging out of `known_offline`, or newly committing inserts/removes a whole
group above the cursor), and Recency's comparator leads with `dimmed`, which
flips on availability/watched changes. Unlike the Playlist (identity-anchored
via `selected_hash`), The List combines a volatile order with destructive,
synced per-row writes (`n` → nero_name, `e` → whole entry via
PutListEntry). Mitigation the verifier noted: `n`/`e` open a modal seeded
with the (wrong) entry's name, so silent corruption requires not reading the
modal — but Enter's `BrowseListEntry` acts immediately, and the aim-drift
itself is real at snapshot rate.

- **Spec:** design.md, The List → UI Integration — the pane must stay
  aimable while the derived order moves.
- **Suggested fix:** Re-anchor by identity across `set_groups`: remember the
  `ListEntryId` (or heading) under the cursor and restore onto its new nav
  position, clamping only when it disappeared — `ListRow` already carries
  `id`. Regression test: cursor on entry B in "Watching — kim";
  `set_groups` with "Watching — amu" freshly inserted above; assert the
  cursor still resolves to B.

<details><summary>Verification trail — code pointers</summary>

set_groups/clamp components.rs:2114–2118/:2148–2150; nav_rows :2128–2141;
actions :2245–2294; mouse also bare-index :2296–2306; unconditional rebuild
app.rs:901–935; group churn props.rs:1849–1935 + server.rs:447
(known_offline is time-limited); Recency comparator props.rs:2007–2018;
identity precedent :1796 (Playlist `selected_hash`); modal mitigation
app.rs:1509–1529.

</details>

### ⚪ LOW · `list_groups` is recomputed from scratch on every snapshot — the exact per-frame rebuild the franchise cache exists to prevent

**`dessplay/src/ui/props.rs:1794`** · _architecture_

The All/Recent arm goes through `franchise_cache` (whose field doc records
the uncached rebuild "was ~⅓ of normal-play CPU"); the TheList arm — the
pane's *default* mode — calls `props::list_groups` unconditionally at
~10 Hz. The work is O(held × entries): full watch-history walk,
`file_availability` walk, and up to three linear scans of `list_entries`
per held hash. Finder measured 0.6 ms/call at 300 entries / 400 files,
9.4 ms/call (~9% of a core, continuously) at 800 / 3000. Latent rather than
present — and the perf rig seeds no List entries, so it can't catch the
regression.

- **Spec:** memory — *TUI lag → franchises recompute*: the fix is
  memoization, not throttling.
- **Suggested fix:** Memoize like `FranchiseCache` on the inputs it reads
  (position ticks change none of them); interim, hoist
  `resolve_series_entry_for_file`'s scans into per-call index maps. Extend
  the perf rig to seed a few hundred List entries.

### ⚪ LOW · A failed PRIMARY write aborts before the CLIPBOARD write — currently latent, live the moment the Wayland feature is enabled

**`dessplay/src/run.rs:1122`** · _bug_ · **uncertain**

`set_clipboard_text` writes PRIMARY first and propagates its error with
`?`, so the CLIPBOARD write — the one Ctrl-V reads, and the only one
off-Linux — never runs when PRIMARY fails, and the caller then discards the
session clipboard handle. The verifier confirmed the ordering is backwards
but found the concrete trigger unreachable in this build: the workspace
pins `arboard` with `default-features = false`, so only the X11 backend is
compiled (no `wl-clipboard-rs`), and under X11 a PRIMARY-only failure has
no distinct path. Kept as latent fragility: enabling `wayland-data-control`
makes it a live "every copy fails" bug on compositors without
zwp_primary_selection_v1.

- **Suggested fix:** Make the PRIMARY write best-effort
  (`let _ = … .inspect_err(debug!)`) and let `set_text` be the only
  fallible step, with a comment naming the ordering rule.

### ⚪ LOW · `Ui::clock` is a monotonic max over two rewindable domains — a backward step freezes every animator until wall time catches up

**`dessplay/src/ui/app.rs:440`** · _bug_ · confirmed

The 2026-08-12 clock-domain fix merges with `max` (app.rs:440, :816) — the
merge is right, but the accumulator never decreases while both inputs can:
`snapshot.now` is bare `SystemTime::now()` and `shared_now` adds a
`clock_offset` that a later ClockSync can shrink. Once latched high, every
tick computes an unchanged `now`: spoiler teases freeze mid-animation while
`spoiler_animating()` pins `next_tick_hint()` at 100 ms (10 wakes/second
doing nothing), marquee passes freeze with `done == false`, and a held
selection's TTL never expires. Bounded — the freeze lasts exactly the size
of the backward step — and a purely local NTP step reproduces it with no
server involved.

- **Suggested fix:** Drive animators from a monotonic source (`Instant`),
  keeping wall/shared time for display and message identity; or clamp the
  merge (refuse a `shared_now` further than a sanity bound from `now`, and
  follow `max(now, shared_now)` rather than latching the historic maximum).
  Unit test: apply_snapshot with a large shared_now, then one back at wall,
  then a tick — assert a running tease still advances.

---

## AI commentary

**Files:** `dessplay/src/commentary.rs` (thread lifecycle, request bodies)
**Read first:** design.md → AI Commentary (the cost model: "the growing
thread re-bills at cache-read rates")
**Key entry points:** `finish` (the frame trim), `build_comment_body`
**Theme:** one finding — the retention cap and the prompt cache fight, and
the cache lost.

### 🟠 MEDIUM · The RETAINED_FRAMES trim mutates already-sent history — the prompt cache the cost model depends on never hits

**`dessplay/src/commentary.rs:1011`** · _architecture_

`finish` strips `screenshot` from turns older than the last RETAINED_FRAMES,
and `build_comment_body` renders a turn as [image, text] vs [text]
accordingly — so each tick changes the bytes of a prefix the previous
request cached. Prompt caching is strict-prefix with the sole breakpoint on
the final user block; at tick T+1 the prefix diverges at turn T−3, and
every older cache entry diverges earlier. `cache_read_input_tokens`
collapses to ~0 for any thread past its third turn whenever mpv delivers
frames (the normal case) — full-price input *plus* the cache-write
surcharge, every tick, contradicting the module doc and the trim's own
comment. Tests check single rendered bodies, never two consecutive
requests' prefixes.

- **Spec:** commentary.rs:369 — "the rendered prefix is byte-stable across
  ticks — the caching invariant"; design.md, AI Commentary.
- **Suggested fix:** Pick one and document it: (a) stop mutating sent
  history — cap thread length (force a re-roll at N turns) so history is
  append-only, keeping both the upload cap and the cache; (b) keep the trim
  but put the breakpoint where it never moves (system + first K turns);
  (c) drop the caching claim from docs and `cache_worthwhile`. Regression
  test: two consecutive ticks on a >3-turn thread — assert the second body
  is a byte-equal prefix-extension of the first.

---

## Closing notes

- The **fastest risk reduction** is fix-first 1 (migration merge) *before*
  the next server/client deploy at v11+ — it is the only finding here that
  destroys durable authoritative state, and its companion test-gap explains
  exactly why the suite is green.
- Three findings (chat panic, chat retarget, List cursor) share one
  structural fix each on their own pane: **identity-keyed positions**
  resolved to indices at use time. `SpoilerKey` and the Playlist's
  `selected_hash` are the in-tree patterns; after these, "positional index
  into a per-snapshot rebuild" should be treated as a lint-on-sight.
- The ask-once latch theme (reconnect discard, pending_resolve,
  eof_reported, denied, blocker sentinel) suggests a small architectural
  pass: every request/answer pair in the actor mesh either carries an
  undroppable reply channel or has a tick-driven re-arm. The transfer
  contract already documents the right rule; it needs to be enforced by
  types, not comments.
- The curator findings (poisoned cache, infinite retry, inline await,
  timeout) are one design review of `curate_short_titles` end-to-end rather
  than four patches — reply keyed to input, progress-or-backoff, own task,
  settle-after-N.
