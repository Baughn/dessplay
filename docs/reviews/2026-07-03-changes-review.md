# DessPlay Codebase Review — Remediation Report

_Generated 2026-07-03 by a multi-agent audit (8 Opus finder agents, one per
area; every bug/security finding then independently adversarially verified).
8 raw findings → 7 kept (3 confirmed bugs, 4 non-bug findings not subject to
adversarial verification), 0 refuted._

_Revision: jj change `wuukozlqqsss` · commit `fd835ffbe768`. Scope: changes
since `19a19da42373` (37 commits, Phases 11–18 feature-request batch)._

<!-- audit-revision
mode: scoped
commit: fd835ffbe768
jj-change: wuukozlqqsss
base: 19a19da42373
generated: 2026-07-03
-->

Per `CLAUDE.md`: any fix here should get a regression test *before* the fix
(property/fuzz test preferred over a narrow unit test where feasible), and
`cargo fmt` should run before committing.

## Executive summary

This batch (37 commits) landed the UI componentization refactor (shared
Form/LineBuffer/ListCursor widgets, keymap-driven dispatch), Phases 13–17
features (osd-overlay blocker summary, mismatch re-check, episode browser
copies/holders, attributed series preferences + known-offline users,
`/summon`), and a long tail of one-off bug fixes. Overall the batch is solid —
most of the targeted fixes (protocol version gate, timeout-ladder Departed
sweep, datagram-first dedup, IRC join backoff, IPv6 bracket handling) check
out. The audit surfaced one clear regression from the widget refactor and two
related "advertises Ready but can never actually serve" wedge bugs in the file
transfer path that share a root cause worth calling out as a theme.

**Recurring theme — partial fixes that close only one of two symmetric
cases.** Three of the seven findings follow the same shape: a fix correctly
closes one branch of a two-branch problem and leaves the mirror branch open.
The `UsersPane` cursor-clamp regression (widget migration correctly handled
the Playlist pane's `+1` extra row but missed the same pattern for Users'
`+known_offline.len()`); the download re-solicit fix (closes the Pending-stage
stall, not the symmetric post-Have stall); and the IRC backoff fix (closes the
reconnect-storm, not the still-inaccurate `Connected` narration before JOIN is
confirmed). Worth a quick sweep for "did I fix both branches" whenever a
patch targets one arm of a two-arm invariant.

### Fix-first order

1. 🟠 **MEDIUM** — UsersPane cursor snaps off a selected known-offline row on
   every snapshot — `dessplay/src/ui/components.rs` (`UsersPane::set_props`).
   One-line fix: clamp to `selectable_len()` instead of `rows.len()`.
2. 🟠 **MEDIUM** — Post-`Have` stalled source is never re-solicited or
   snubbed, wedging a download — `dessplay/src/download.rs` (`snub` /
   `progress_and_refill`). Broaden the re-solicit guard beyond `need_hashes`.
3. 🟠 **MEDIUM** — Seeders leak into `known_offline` and are shown as
   selectable regular users — `dessplay-rendezvous/src/server.rs`
   (`record_seen`). Guard both call sites on `Role::Interactive`.
4. ⚪ **LOW** — A content-mismatched manual mapping still advertises Ready and
   can never serve, causing perpetual re-solicitation —
   `dessplay/src/actors/file.rs` (`set_manual_mapping`).
5. ⚪ **LOW** — Re-import silently resolves a cross-sheet status conflict for
   pre-existing List entries without reporting it — `dessplay/src/import.rs`.
6. ⚪ **LOW** — IRC `Connected` narrated before JOIN is confirmed, so a
   permanently-rejected channel still logs a false "connected" each backoff
   cycle — `dessplay/src/actors/irc.rs`.
7. ⚪ **LOW** (deferred-by-decision, unchanged) — Downloading-unpause rule only
   enforces the 20% threshold, not the speed-vs-bitrate half —
   `dessplay-core/src/derive.rs`. Already tracked in design.md Future Plans;
   listed for accounting only.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| UI panes (Users pane cursor) | | 1 | | 1 |
| File transfer / download | | 1 | 1 | 2 |
| Rendezvous server (presence) | | 1 | | 1 |
| Import (The List) | | | 1 | 1 |
| IRC bridge | | | 1 | 1 |
| Playback gating (derive) | | | 1 | 1 |
| **Total** | **0** | **3** | **4** | **7** |

## Per-region sections

## UI panes (Users pane cursor)

**Files:** `dessplay/src/ui/components.rs` (UsersPane), `dessplay/src/ui/widgets/list.rs` (ListCursor), `dessplay/src/ui/app.rs` (apply_snapshot)
**Read first:** design.md → *Presence* #15 ("Known but offline") — known-offline rows are individually selectable `a`/`n` targets, same as a present row
**Key entry points:** `UsersPane::set_props`, `UsersPane::selectable_len`, `UsersPane::nav`, `UsersPane::selected_username`
**Theme:** the widget-componentization refactor correctly generalized `PlaylistPane`'s "extra synthetic row" clamp (`rows.len()+1`) but the equivalent generalization for `UsersPane`'s wider selectable range (`rows.len()+known_offline.len()`) didn't make the jump.

### 🟠 MEDIUM · UsersPane cursor is clamped to only the live-rows count, dropping a selected known-offline user on every snapshot

**`dessplay/src/ui/components.rs:681`** · _bug_

`UsersPane::set_props` clamps the cursor with `self.cursor.clamp(self.props.rows.len())`, but the pane's actual selectable range is `rows.len() + known_offline.len()` (see `selectable_len()` at :724-725, which `nav()` at :765 and `selected_username()`/`act_away`/`act_not_watching` at :695-704 all use). Per design.md #15, known-offline users are meant to be individually selectable `a`/`n`/`/skip <name>` targets — "equally valid ... targets" alongside present users.

`apply_snapshot` (`dessplay/src/ui/app.rs:522`) calls `users.set_props(...)` unconditionally on *every* incoming snapshot (presence churn, chat, position updates, state writes — i.e. constantly during a session). The moment a user has navigated the cursor down into the known-offline range (`index >= rows.len()`), the very next snapshot clamps it back to `rows.len() - 1` — the last *present* user. The selection silently jumps off the intended offline target onto an unrelated present user; pressing `n` or `a` then acts on the wrong person. No panic (render uses the now-in-bounds `cursor.index()`), so this is purely a silent mis-targeting defect, easy to miss in manual testing and currently uncovered by tests.

- **Spec:** design.md #15 ("Known but offline"): known-offline rows are "equally valid `n` / `/skip <name>` targets" and the Users pane renders them as selectable rows.
- **Prior:** new (introduced by this batch's UI widget migration).
- **Suggested fix:** Write a regression test first: select a known-offline row, call `set_props` again with an unchanged `known_offline` list (simulating any snapshot refresh), and assert `selected_username()` (and the `act_not_watching` target) is unchanged. Then fix `UsersPane::set_props` to clamp against `self.selectable_len()` (i.e. `rows.len() + known_offline.len()`), mirroring `PlaylistPane::set_props`'s existing `rows.len()+1` clamp at `components.rs:801`.

<details><summary>Verification trail — code pointers</summary>

Traced end-to-end: `UsersPane::set_props` (`components.rs:681`) clamps to `props.rows.len()` only; `selectable_len()` (`:724-725`) includes `known_offline.len()`; `nav()` (`:765`) and `selected_username()` (`:695-704`) both operate over the wider range. `ListCursor::clamp(len)` (`dessplay/src/ui/widgets/list.rs:43-45`) computes `sel = sel.min(len-1)`, so a cursor pointing at a known-offline row (`index >= rows.len()`) is forced back to `rows.len()-1`. `apply_snapshot` (`app.rs:522`) calls `set_props` unconditionally on every snapshot. The sibling `PlaylistPane::set_props` (`:801`) correctly clamps to `rows.len()+1`, confirming this is the intended pattern and the Users pane is the outlier. Verifier confidence: high.

</details>

## File transfer / download

**Files:** `dessplay/src/download.rs` (per-file download state machine, source scheduling, snub/re-solicit), `dessplay/src/actors/file.rs` (manual mapping, servable-set bookkeeping)
**Read first:** design.md → *File Matching* #4a/4c, *Download Cache and Retention*; network-design.md → File Transfer protocol
**Key entry points:** `download.rs::snub`, `download.rs::progress_and_refill`, `download.rs::set_sources`, `file.rs::set_manual_mapping`, `file.rs::serve_block_hashes`
**Theme:** two related "advertises Ready but can't actually serve, so the peer re-solicits forever" wedges — one from an incomplete stall-recovery fix, one from a manual-mapping edge case that was never fully closed.

### 🟠 MEDIUM · Post-`Have` stalled source is never re-asked or snubbed, and can permanently wedge a download

**`dessplay/src/download.rs:479`** · _bug_

This batch added a stall-recovery loop in `snub` gated by `need_hashes = matches!(d.block_hashes, BlockHashes::Pending)` (`:444`, `:479`), which correctly fixes the case where block hashes themselves were never solicited/received. But `progress_and_refill` (`:536`) also solicits a *newly-added* source once block hashes are already `Have`, sending a `BlockHashRequest` that doubles as a bitfield solicitation and latching `solicited = true`. If that request or its `FileAvailability` reply is lost (a dropped relay message, or the peer reconnecting mid-flight), the source's bitfield stays empty forever: it's never picked for chunks (empty bitfield → chunk-stage snub at `:450` requires non-empty `in_flight`, which this source never has), and `need_hashes` is now `false` so the Pending-stage re-solicit at `:479` skips it too. `set_sources` (`:265`) re-adds via `entry().or_insert_with`, which does not reset `solicited` for a source that stays present, so it is never re-tried.

- **Spec:** network-design.md File Transfer implies no source should be able to permanently wedge a transfer — a source that fails to supply usable data must eventually be re-solicited or abandoned.
- **Prior:** new (the Pending-stage fix landed this batch; the Have-stage mirror case was not covered).
- **Suggested fix:** Regression test first: bring a download to `Have` via one source, add a second source whose solicitation reply is dropped (empty bitfield, `solicited=true`, empty `in_flight`), advance past the snub timeout, and assert it gets re-solicited. Then broaden the re-solicit condition in `snub` to also cover `src.solicited && src.in_flight.is_empty() && src.bitfield.count_ones() == 0 && now - src.last_progress >= timeout`, independent of the `need_hashes` gate.

<details><summary>Verification trail — code pointers</summary>

Confirmed: the Pending-only re-solicit loop is `download.rs:479-495`, gated by `need_hashes` (`:444`). The post-`Have` one-shot solicit is `:536-543` (`!src.solicited` guard, latches `solicited=true`, never re-fires on loss). Chunk-stage snub (`:450`) requires non-empty `in_flight`, which an empty-bitfield source never accumulates. `set_sources` (`:261-271`) uses `retain` + `entry().or_insert_with`, so a continuously-present source keeps `solicited=true` and is never reset — the only escape is the source going Lost and returning with a fresh entry, which the failure scenario (a continuously-present Ready holder) excludes. The code's own comment near `:529-532` names this exact mechanism as the thing that should prevent a wedge, but implements retry-on-loss only for the Pending stage. Verifier confidence: medium (requires an uncommon message-loss event on an otherwise-reliable relay, but consequence — a wedged now-playing download blocking the group if the downloader is present/committed — is real).

</details>

### ⚪ LOW · Content-mismatched manual mapping still advertises Ready, causing perpetual re-solicitation

**`dessplay/src/actors/file.rs:1517`** · _bug_

`hash_manual_mapping`'s `Done::ManualHashed` handler (`:1217-1233`) correctly only caches block hashes when the mapped file's content matches the target (`hashed.root == file`) — a deliberate, correct guard against serving wrong content. However, `set_manual_mapping` (`:1517`, `:1519-1525`) unconditionally inserts the mapping into `local_files` and emits `Resolution::Verified`, which the session advertises to peers as `FileAvailability::Ready` — even when the content check above will fail. A peer that later solicits block hashes from this "Ready" holder gets bailed by `serve_block_hashes` (`:1015-1029`, "haven't cached" / "don't match"), and — compounded by the download.rs finding above — that peer's downloader re-sends `BlockHashRequest` on every snub timeout indefinitely, since the source is never marked unusable and never departs.

- **Spec:** design.md *File Matching* 4a (a manual map is meant to be a servable local copy) and the general "Ready implies servable" invariant network-design.md relies on for peer solicitation.
- **Prior:** still-open (noted in the prior review; not addressed by this batch's fixes, which targeted the Pending-stage re-solicit case instead).
- **Suggested fix:** Either don't advertise Ready for a manual mapping until `Done::ManualHashed` confirms a content match (so the entry stays Missing/local-only until confirmed), or retract the Ready advertisement the moment a mismatch is detected, so no peer solicits an unservable holder. Add a regression test: manually map to a different-encode file, confirm the peer's `FileAvailability` never claims Ready for that hash, or (if Ready is kept for local playback) confirm a peer download of it terminates rather than looping.

<details><summary>Verification trail — code pointers</summary>

Confirmed: `set_manual_mapping` (`:1516-1524`) unconditionally inserts into `local_files` and emits `Resolution::Verified`, regardless of the later hash outcome. `Done::ManualHashed` (`:1217-1233`) intentionally skips caching a mismatched encode. `serve_block_hashes` (`:1015-1029`) bails on the resulting cache miss/root mismatch. Combined with `download.rs:479-495`'s gap above, the requesting peer re-solicits roughly every snub timeout with no termination condition. Verifier confidence: low (requires a same-named-but-different-encode manual map plus another peer being the sole holder, but the mechanism traces cleanly and severity is correctly scoped low since it's self-limited to peers of that specific manual-mapper).

</details>

## Rendezvous server (presence)

**Files:** `dessplay-rendezvous/src/server.rs` (`record_seen`, `known_offline` construction), `dessplay-core/src/net/message.rs` (`KnownUser`), `dessplay/src/ui/props.rs` (`users_props`)
**Read first:** design.md → *Client Roles* ("Seeders are not listed as users"), *Presence* → "Known but offline (#15)"
**Key entry points:** `Server::record_seen` (connect/disconnect hooks), `known_offline` snapshot builder, `users_props`
**Theme:** the new #15 known-offline feature didn't inherit the seeder exclusion that every other presence-derived surface (Users pane, gating, chat narration) already has.

### 🟠 MEDIUM · Seeders leak into `known_offline` and render as selectable regular users

**`dessplay-rendezvous/src/server.rs:882`** · _spec-drift_

`record_seen` is called unconditionally on connect (`:882`) and disconnect (`:951`) regardless of `role`, writing a seeder's username into the persisted `known_users` table. `known_offline` (`:642-666`) filters only on current `Present` status — it carries no role information at all (`KnownUser` in `dessplay-core/src/net/message.rs:77-83` has just `username` + `last_seen`). So whenever a seeder isn't currently connected — a secondary NAS seeder, a seeder service restart while the rendezvous stays up, or the window before a seeder reconnects after a rendezvous restart — its username appears in `PeerList.known_offline`. The client can't filter it out: `users_props` (`dessplay/src/ui/props.rs:200-211`) dedups `known_offline` only against `props.rows`, not against the separate "seeders:" line, so the seeder shows up as an ordinary selectable known-offline user — a meaningless `n`/`/skip <name>` target on something that should never gate or appear as a user at all.

- **Spec:** design.md: "Seeders are not listed as users; they appear on a separate dim 'seeders:' line" and "Seeders are excluded from every presence-derived line."
- **Prior:** new.
- **Suggested fix:** Regression test first: a seeder connects then disconnects while an interactive peer stays connected; assert that peer's next `known_offline` list never contains the seeder's username. Then guard both `record_seen` call sites on `role == Role::Interactive`, mirroring the existing seeder-exclusion guards used elsewhere in `server.rs` (e.g. the force-pause/take-seek-authority paths).

<details><summary>Verification trail — code pointers</summary>

Non-bug/spec-drift finding — not subject to the adversarial disprove pass per the workflow's convention (bug/security findings only), but the finder's reasoning traces the full path: `record_seen` call sites (`:882`, `:951`), `known_offline` construction (`:642-666`) with no role filter, `KnownUser` struct (`dessplay-core/src/net/message.rs:77-83`) carrying no role, and `users_props` (`dessplay/src/ui/props.rs:200-211`) deduping only against live rows. Confidence: high (finder), unverified by adversarial pass.

</details>

## Import (The List)

**Files:** `dessplay/src/import.rs`
**Read first:** design.md → *The List (Series Tracker)* → Import section
**Key entry points:** `import.rs::run_import` (or equivalent), the `seen` map / `ImportOutcome.collapsed`
**Theme:** the fix that "collapses a series named on more than one sheet" (prior batch) only reports the conflict for brand-new List entries, not for re-imports touching an existing entry — which is the primary supported workflow (design.md explicitly expects repeat imports).

### ⚪ LOW · Cross-sheet status conflict on a pre-existing List entry is silently resolved, never reported

**`dessplay/src/import.rs:457`** · _quality_

`ImportOutcome.collapsed` is documented as surfacing series that appear on more than one imported sheet, so the user can manually reconcile a status conflict. Detection is gated on `created_this_run` (`:457`): the `seen` map is seeded from *existing* List entries with the flag `false` (`:442`), and only the create branch (`:465`) sets it `true`. So when a series already exists in The List and is named on two sheets in the same import run, both matching rows take the `Some(.., created_this_run=false)` branch, both increment `outcome.updated`, and neither is pushed into `collapsed` — even though the two rows still overwrite each other via LWW (later sheet wins), which is exactly the conflict `collapsed` exists to surface.

- **Spec:** `import.rs`'s own `ImportOutcome.collapsed` doc comment (`:384-387`): "Series that appeared on more than one imported sheet ... Surfaced so the user can reconcile a status conflict between sheets."
- **Prior:** new.
- **Suggested fix:** Track "already touched in this run" independent of "newly created" (e.g. a separate `HashSet` of consumed keys, or a `seen_this_run` flag distinct from `created_this_run`), and push to `collapsed` whenever a second imported row in the same run targets an entry already touched — whether that entry was pre-existing or created moments earlier.

<details><summary>Verification trail — code pointers</summary>

Non-bug/quality finding, not subject to adversarial verification. Finder traced: `seen` map seeded `created_this_run=false` for existing entries (`:438-443`), update branch only pushes to `collapsed` when `created_this_run` is true (`:450-461`), create branch sets the flag true (`:463-468`). Confidence: medium.

</details>

## IRC bridge

**Files:** `dessplay/src/actors/irc.rs`
**Read first:** design.md → *IRC Bridge* → Lifecycle
**Key entry points:** the `001`/`RPL_WELCOME` handler, `IrcEvent::Connected`/`Disconnected`, the join-rejection backoff added this batch
**Theme:** the reconnect-storm fix (backing off on persistent join rejection instead of tight-looping) is correct and tested; the follow-on refinement (don't claim "connected" until the join is actually confirmed) wasn't part of it.

### ⚪ LOW · `IrcEvent::Connected` fires before JOIN is confirmed, so a permanently-rejected channel still narrates a false "connected" every backoff cycle

**`dessplay/src/actors/irc.rs:430`** · _quality_

The backoff fix (Rejected → Disconnected without resetting the backoff clock, growing 2s→4s→...→60s) is correct and well covered by tests. But `IrcEvent::Connected` is still emitted immediately on `RPL_WELCOME` (`001`), right after sending JOIN and before any confirmation the join succeeded. For a channel the bridge can never join (e.g. `+R`, or an unregistered nick), each backoff cycle still: connects, registers, emits `Connected`, gets rejected, emits `Disconnected`. The chat pane accumulates a misleading `Connected`/`Disconnected` pair every cycle (at growing intervals, settling around one pair/minute), and the `Connected` line claims a join that never happened. The `IrcEvent::Connected` doc comment ("Registered and joined the channel", `:122-123`) is inaccurate for the same reason.

- **Spec:** design.md IRC Bridge Lifecycle ("reconnects with capped backoff"); the `IrcEvent::Connected` doc comment's own claim of "joined the channel."
- **Prior:** still-open (residual/partial fix from the prior review's reconnect-storm finding — the storm itself is resolved).
- **Suggested fix:** Defer emitting `Connected` until the join is actually confirmed (`RPL_ENDOFNAMES`/366, or the first `RPL_NAMREPLY`/353 for the channel), so a rejected channel never narrates a false "connected." If deferring is more invasive than warranted, at minimum correct the doc comment to say "registered" rather than "joined the channel."

<details><summary>Verification trail — code pointers</summary>

Non-bug/quality finding, not subject to adversarial verification. Finder traced the `001` handler (`:424-431`, emits `Connected` pre-join-confirmation), the doc comment (`:122-123`), and the (correct) `Rejected` handling (`:230-239`). Confidence: medium.

</details>

## Playback gating (derive)

**Files:** `dessplay-core/src/derive.rs`
**Read first:** design.md → *File State* → Downloading unpause rule
**Key entry points:** `file_block_reason`
**Theme:** carried forward from the prior review, unchanged by this batch — listed for accounting continuity, not as new work.

### ⚪ LOW (deferred-by-decision) · Downloading-unpause rule enforces only the 20% threshold, not the speed-vs-bitrate half

**`dessplay-core/src/derive.rs:131`** · _spec-drift_

`file_block_reason` permits a Downloading user to unpause once `progress_bps >= 2_000` (20%), with no throughput check, while design.md's File State rule requires *both* ≥20% downloaded *and* download speed higher than the file's computed bitrate. `FileAvailability::Downloading { progress_bps }` structurally carries only progress, so the speed clause can't be evaluated from synced state as it exists today — this is explicitly recorded in design.md's Future Plans section as deferred pending a synced eligibility signal. Impact is self-only: a user unpausing at exactly 20% with throughput below bitrate may stall their own playback; it never gates the group.

- **Spec:** design.md File State: "their download speed must be higher than the file's computed bitrate, and at least 20% of the file must be downloaded."
- **Prior:** deferred-by-decision — already ruled on in the prior review and documented as future work; not re-litigated here, just carried forward for accounting.
- **Suggested fix:** None unless the deferral is revisited. If it is: add a downloader-computed "eligible-to-play" (speed ≥ bitrate) signal to `FileAvailability::Downloading` and AND it with the existing 20% threshold.

<details><summary>Verification trail — code pointers</summary>

Non-bug/spec-drift finding, not subject to adversarial verification. Unchanged since the prior review; `derive.rs:131-133`. Confidence: high.

</details>
