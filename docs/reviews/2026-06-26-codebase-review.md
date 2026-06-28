# DessPlay Codebase Review — Remediation Report

_Generated 2026-06-26 by a multi-agent whole-codebase review (23 subsystem reviewers, each given the source plus the authoritative `docs/`; every finding then independently adversarially verified). 53 raw findings → **46 confirmed**, 7 refuted, 0 uncertain._

This report is organized **by code region** so it can be worked through across several sessions — each region opens with orientation pointers (which files, which design-doc sections, the key functions and invariants this reviewer learned) so whoever picks it up can get up to speed before touching code. Severity legend: 🔴 **HIGH** (fix first), 🟠 **MEDIUM**, ⚪ **LOW** (mostly spec-drift, dead code, doc-precision, and test-gaps). Each finding lists the exact location, the problem, the spec rule it touches, a suggested fix, and a collapsible **verification trail** with the precise code pointers the verifier checked.

> Working a finding? Per `CLAUDE.md`, add a regression test (property/fuzz preferred) that fails *before* the fix, and treat each bug as a chance to tighten the architecture or a design-doc detail — several findings below are themselves stale-doc or invariant-erosion issues.

## Executive summary

The codebase is broadly healthy: most findings are spec-drift or test gaps rather than active corruption, and convergence/idempotency consistently rescue the system from the few real bugs. The standouts:

- **File transfer is substantially under-built vs. its design** — effectively single-source; multi-source/rarest-first/partial-serve never engage, and a departed source can permanently stall a transfer. This is the largest gap between spec and reality.
- **A real position-estimation bug in the player actor** can make a client spuriously become the drift-correction leader and yank the whole group forward by an entire pause interval (plus a false 85% watched mark).

Three recurring themes: a cluster of **cache/archive bookkeeping omissions in `file.rs`** (the servable map diverges from the documented "drop + prune + re-resolve" contract); **chat-narrator spec-drift in `session.rs`** (narration derived from per-client local state instead of purely synced state, so clients narrate different lines); and **network-transport drift** where high-frequency position ops are relayed reliably, reintroducing forbidden head-of-line blocking.

### Fix-first order

1. 🔴 **Player pause re-anchor** — `dessplay/src/actors/player.rs` (`handle_pause_observation`). Group-wide false seek; small, contained fix.
2. 🔴 **Single-source download stall** — `dessplay/src/download.rs` (`progress_and_refill`). Unblocks the whole transfer design; fix the stale-`assigned` duplicate-fetch at the same time.
3. 🟠 **Server state-wipe on load error** — `dessplay-rendezvous/src/server.rs` (`load_state().ok().flatten()`). One-line correctness guard protecting authoritative state.
4. 🟠 **Reliable relay of position ops** — `dessplay-rendezvous/src/server.rs` (`broadcast_op`). Restores the datagram-only fast path.
5. 🟠 **EOF not gated on `holds_now_playing`** — `dessplay/src/session.rs`. Stops a placeholder from advancing the group.
6. 🟠 **Archive forgets `local_files`** — `dessplay/src/actors/file.rs`. One-line insert; prevents a spurious Missing-blocker.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| Core — Playback gating & derived state |  |  | 3 | 3 |
| Core — CRDT state, playlist & sync |  | 1 | 2 | 3 |
| Core — Franchise grouping |  |  | 1 | 1 |
| Core — ed2k hashing |  |  | 1 | 1 |
| Core — Networking & transport |  |  | 2 | 2 |
| Client — Player actor & mpv | 1 | 1 |  | 2 |
| Client — File actor: cache, retention, matching, archive |  | 1 | 3 | 4 |
| Client — File transfer & download | 1 | 1 | 3 | 5 |
| Client — Session layer: narrator & gating glue |  | 1 | 2 | 3 |
| Client — Sync & network actors |  |  | 1 | 1 |
| Client — IRC bridge |  | 1 | 2 | 3 |
| Client — TUI (ui/) |  | 1 | 6 | 7 |
| Client — Lifecycle, config & storage |  | 1 | 5 | 6 |
| Client — List import |  |  | 1 | 1 |
| Server — Rendezvous server |  | 1 | 1 | 2 |
| Server — Storage |  | 1 |  | 1 |
| Server — AniDB integration |  |  | 1 | 1 |
| **Total** | **2** | **10** | **34** | **46** |

---

## Core — Playback gating & derived state

**Files:** `dessplay-core/src/derive.rs` (the gating engine); its UI mirror in `dessplay/src/ui/props.rs`.
**Read first:** design.md → *User States*, *Ready States (UI Display)*, *Playback Rules* — especially the Watching/Maybe/NotWatching × present/absent blocking matrix and the manual-override precedence.
**Key entry points:** `derive::user_state`, `derive::playback_blockers`, `derive::playback_active`, `file_block_reason`. The Users-pane helper `committed_absent_blocker` (props.rs) is *supposed* to mirror `playback_blockers` exactly — divergence there is a display bug, not a gating bug.
**Theme:** the gating logic itself is correct and well-tested; the findings here are (a) one display/gating-parity mismatch, (b) an un-enforced half of the Downloading unpause rule, and (c) a missing test for a correct-but-subtle precedence case.

### ⚪ LOW · Downloading unpause rule enforces only the 20% half, not the download-speed > bitrate half

**`dessplay-core/src/derive.rs:126-135`** · _spec-mismatch_

design.md's File State / Downloading rule (lines 236-238) says unpausing while downloading is conditional on BOTH 'download speed must be higher than the file's computed bitrate' AND 'at least 20% downloaded'. file_block_reason only checks the 20%-equivalent half: `(*progress_bps < 2_000).then_some(Downloading)` permits at >= 20% regardless of throughput. A user can therefore unpause/the group can start at exactly 20% with a download speed below the file's bitrate and then stall mid-episode. The code comment (lines 122-125) frames the speed half as deferred Phase-9 work, but the transfer machinery (dessplay/src/download.rs, chunkstore.rs, actors/file.rs) now exists, while the synced FileAvailability::Downloading carries no speed/bitrate field, so the rule cannot be fully evaluated from synced state. Either FileAvailability needs a throughput/eligibility flag or the spec's speed clause should be marked permanently out-of-scope.

- **Spec:** design.md File State: 'Unpausing is conditional: their download speed must be higher than the file's computed bitrate, *and* at least 20% of the file must be downloaded.'
- **Suggested fix:** Carry an 'eligible-to-play' signal (download speed >= bitrate) in FileAvailability::Downloading computed by the downloading client, and AND it with the 20% threshold in file_block_reason; or update design.md to record that only the 20% half is enforced.

<details><summary>Verification trail — code pointers</summary>

Confirmed against code and spec. derive.rs:131-133 implements the Downloading unpause gate as `Some(FileAvailability::Downloading { progress_bps }) => (*progress_bps < 2_000).then_some(BlockReason::Downloading)` — it blocks below 20% and permits at >=20%, with no throughput check. The synced variant `FileAvailability::Downloading { progress_bps: u16 }` (types.rs:268-271) carries only progress in basis points (chunkstore.rs:235-247 computes it as verified-bytes/size*10000, a progress fraction, NOT bytes/sec), so there is no speed/throughput field to compare against bitrate; the speed half is structurally unevaluable from synced state. design.md:236-238 explicitly requires BOTH clauses: "their download speed must be higher than the file's computed bitrate, *and* at least 20% of the file must be downloaded." So only the 20% half is enforced. Gating is centralized in derive::playback_active (session.rs:523/1241/1359) -> file_block_reason; a grep across player.rs/session.rs/file.rs found no compensating client-side download-speed-vs-bitrate gate (only unrelated sim.rs link-throughput and a hash benchmark). The code comment derive.rs:120-125 frames the speed half as Phase-9-deferred, but plan.md:503/532 marks Phase 9A and 9B complete (2026-06-13) — the transfer machinery now exists, so the deferral rationale is stale while the rule is still unimplemented. The reviewer's two proposed resolutions (add a throughput/eligibility field to FileAvailability, or mark the spec's speed clause out-of-scope) are both valid. Low severity is correct: impact is a single user starting at exactly 20% with insufficient throughput stalling for themselves; it does not block the group and is an acknowledged-incomplete implementation.

</details>

### ⚪ LOW · No gating test that a present NotWatching user who is also manually Paused does not block

**`dessplay-core/src/derive.rs:189-204, 378-405`** · _test-gap_

playback_blockers short-circuits NotWatching to None (line 190) before the manual-pause check, so a present NotWatching user with a Paused manual override does NOT gate playback — the intended behavior ('NotWatching ... never gates playback on it, present or absent'). This is a subtle precedence inversion versus user_state, where the manual Paused override wins for display. The only test for this combination, `manual_pause_overrides_not_watching` (lines 378-405), asserts user_state == Paused but never calls playback_blockers / playback_active, leaving the gating side of this high-risk override-vs-commitment interaction uncovered.

- **Spec:** design.md User States: 'NotWatching (definite no): the user ... never gates playback on it, present or absent'.
- **Suggested fix:** Extend manual_pause_overrides_not_watching (or add a sibling) to assert playback_active(view, &[present(user)]) is true and playback_blockers is empty for a present NotWatching+Paused user.

<details><summary>Verification trail — code pointers</summary>

Code verified at /home/svein/dev/dessplay/dessplay-core/src/derive.rs. In playback_blockers (lines 189-204), the match arm `SeriesWatchState::NotWatching => None` (line 190) short-circuits before any manual-pause check — that check lives in present_block_reason (line 150), reached only for Maybe/Watching present users. The only pre-match override handling is the Away `continue` (line 185); a Paused override falls through and is ignored for a NotWatching user. So a present NotWatching user who is also manually Paused does NOT block. This matches the spec (docs/design.md User States: "NotWatching (definite no): the user ... never gates playback on it, present or absent"), so it is intended behavior, not a bug. The precedence inversion vs. user_state is real: user_state (line 100-115) returns DerivedUserState::Paused from the override (line 102) before consulting the watch state, so the override wins for display. Test gap confirmed: manual_pause_overrides_not_watching (lines 378-405) sets NotWatching + ManualState::Paused but only asserts user_state == Paused; it never calls playback_blockers/playback_active. not_watching_series_does_not_block (341-376) covers the gating side but without a manual Paused override. Grepped all other call sites (dessplay-rendezvous/tests/presence.rs, player.rs, ui/props.rs, ui/app.rs, session.rs) — the NotWatching tests there (player.rs:387-604) cover only the auto-NotWatching missing-file flow, none combine a manual Paused override with NotWatching on the gating side. The (present, NotWatching, manual-Paused) gating case is genuinely uncovered. Severity low is appropriate for a test-gap of a correctly-implemented, spec-confirmed behavior.

</details>

### ⚪ LOW · Users pane shows a committed-absent user as a red blocker even after Away excuses them, disagreeing with gating

**`dessplay/src/ui/props.rs:90-98, 164-170`** · _bug_

committed_absent_blocker (props.rs:164-170) checks only `series_watch_for_file == Watching && !acknowledged_absent.contains(...)`. It omits the manual-override Away check that derive::playback_blockers performs (derive.rs:185-187 early `continue`). The spec's per-user escape hatch (marking a committed-absent user Away clears the block; verified by derive test `away_excuses_a_committed_absent_user`) makes playback_active return true, but the Users pane still renders that user on the 'committed, away' Tone::Blocked (red) line. Result: playback proceeds while the Users pane shows a red blocker that is not actually blocking — directly contradicting the helper's own doc comment ('Mirrors the Watching + Lost|Departed arm of derive::playback_blockers so the Users pane and status bar agree'). The miss is exactly the case the derive.rs test covers but the props.rs path does not.

- **Spec:** design.md User States: '/away ... Away behaves like Not Watching for playback gating'; Playback Rules: 'Away (any presence) never blocks — also the manual escape hatch [for committed-absent]'.
- **Suggested fix:** In committed_absent_blocker, return false when view.manual_override.get(user) resolves to Some(ManualState::Away{..}) (and arguably Paused-handling parity), so the Users pane mirrors playback_blockers exactly; add a props test for an Away-excused committed-absent user.

<details><summary>Verification trail — code pointers</summary>

Confirmed against the code. props.rs:164-170 `committed_absent_blocker` returns true on `series_watch_for_file(view,user,file)==Watching && !acknowledged_absent.contains((file,user))` and never consults `view.manual_override`. By contrast, derive.rs:185-187 (`playback_blockers`) early-`continue`s on `ManualState::Away { .. }` before the commitment/presence match, so an Away override excuses a committed-absent user — proven by the test `away_excuses_a_committed_absent_user` (derive.rs:577-595), where `playback_active` returns true. The Users pane (props.rs:90-98) renders any Departed/Lost peer with `committed_absent_blocker==true` as a red `Tone::Blocked` "committed, away" row, while the status bar (props.rs:443-454) and the /ack target (app.rs:1119-1121) both go through the real `derive::playback_blockers` and therefore drop the away-excused user. Result: after the documented per-user Away escape hatch is applied to a committed-departed user, playback proceeds yet the Users pane keeps showing a red blocker — contradicting both the helper's own doc comment (props.rs:160-163, "so the Users pane and status bar agree") and design.md (Playback Rules: "Away (any presence) never blocks — this is also what an acknowledge writes"). The fix is to add the same `ManualState::Away` guard to `committed_absent_blocker`; an away-excused departed user would then fall through to the dim departed line. Down-rated to low because the defect is purely cosmetic: gating/playback are correct, only the Users-pane label and color are wrong in this edge case (no data or playback-correctness impact).

</details>

---

## Core — CRDT state, playlist & sync

**Files:** `dessplay-core/src/state.rs` (CrdtState + the datagram FIFO gap-detector), `lww.rs`, `compact.rs`, `playlist.rs`. Some fixes land in `dessplay-rendezvous/src/server.rs` (the broadcast path).
**Read first:** sync-state.md → *Delivery requirements / Phase-4 constraint* (datagram ordering), *Operation Broadcast*, *Compaction*, and the `Lww<V>` rationale (why `crdts::MVReg` was rejected).
**Key entry points:** `CrdtState::apply` (reliable path) vs `apply_if_orderly` / `next_in_sequence` (datagram fast path — the sole guard against silent op loss), `compact::rebuild` (the real compaction path; `rebalance_playlist` is *not* wired in).
**Theme:** convergence is sound; the gaps are a missing property test for the datagram-ordering guard (high-risk per CLAUDE.md) and minor broadcast/doc drift.

### 🟠 MEDIUM · Datagram FIFO gap-detection (apply_if_orderly / next_in_sequence) has no test coverage

**`dessplay-core/src/state.rs:685-735`** · _test-gap_

apply_if_orderly is the Phase-4 safety mechanism that prevents a datagram-delivered map op from being applied ahead of an undelivered earlier op from the same origin (which crdts would silently mask: 'later dots mask earlier ones and ops are lost silently', sync-state.md). I verified the logic is correct (per-map per-actor dot sequencing via add_clock.get(actor)+1==counter, Rm dropped, registers/GSet/GList treated as order-free). But no test exercises it: the convergence/property harness (dessplay-core/src/test_support.rs run_cluster, deliver_one and server_poll) applies every op exclusively through CrdtState::apply, i.e. the reliable FIFO path. A grep across dessplay-core/tests shows apply_if_orderly/next_in_sequence are never referenced. A regression here (off-by-one in the dot check, or wrongly classifying a Map-backed type into the order-free arm so a gap gets masked) would pass CI. This is exactly a high-risk area the project mandates extra coverage for (CRDT convergence, datagram delivery).

- **Spec:** sync-state.md 'Delivery requirements' / 'Phase 4 constraint: the datagram fast path must not apply an op ahead of undelivered earlier ops from the same origin — hold (or drop) such datagrams until the reliable stream catches up.'
- **Suggested fix:** Add a property test that generates a per-origin sequence of map ops, delivers them out of order via apply_if_orderly (interleaving gaps), and asserts: (a) only in-sequence ops apply, (b) the resolved view never diverges from the reliable-order replica, (c) order-free ops always apply and stay idempotent. Extend run_cluster with a datagram lane that routes through apply_if_orderly.

**Status (2026-06-28): fixed (tests added).** New integration suite `dessplay-core/tests/datagram_ordering.rs` (7 tests) now drives the datagram fast path end to end. Coverage: **(a)** `datagram_holds_an_out_of_sequence_map_op_until_the_gap_is_filled` (deterministic) and the proptest `datagram_holds_every_op_while_an_earlier_dot_is_missing` prove a map op whose dot skips an undelivered earlier same-origin op is held (not applied, view untouched) and applies only once the gap is filled; `datagram_redelivery_of_an_applied_map_op_is_a_no_op` covers idempotent re-delivery, and `map_backed_series_preference_is_gap_checked` guards against a Map-backed variant being misfiled into the order-free arm. **(b)** The proptest `datagram_lane_converges_to_the_reliable_view` builds a realistic server log via `run_cluster`, then delivers it through a new **datagram lane** in a seeded-shuffle order and asserts the converged view equals both the reliable in-order replica and the server. **(c)** `order_free_ops_apply_in_any_order_and_are_idempotent` (deterministic) and the proptest `order_free_log_is_order_insensitive_and_idempotent` confirm registers/GSet/GList ops always apply (`held == 0`) and stay idempotent under any ordering. Harness extension: `test_support::deliver_via_datagram_lane` + `DatagramLaneOutcome` — a reusable lane that offers every op through `apply_if_orderly`, retrying gap-dropped datagrams (modelling the reliable second copy) until quiescent, and reporting `held` / `undelivered` so tests can assert a gap was actually exercised. **Mutation check** (test-gap analogue of "confirm the regression fails first"): temporarily forcing `next_in_sequence`'s `Op::Up` arm to `true` (gaps no longer held) made all 5 gap-check tests FAIL — including (a) and (b) — while the 2 order-free tests correctly stayed green (they never hit the guard); reverting restored all 7 to passing. Production logic unchanged (the finding was a coverage gap, not a bug).

<details><summary>Verification trail — code pointers</summary>

CODE: state.rs:685-735 implements apply_if_orderly exactly as the claim describes. next_in_sequence (687-699) checks `map.read_ctx().add_clock.get(&dot.actor) + 1 == dot.counter` for Op::Up and returns false for Op::Rm; the guarded! macro (701-723) routes the 11 Map-backed variants through the gap check, while NowPlaying/SeekAuthority/PlaybackIntent/LookupRequest/Chat/AcknowledgeAbsent are applied unconditionally in the order-free arm (725-733). Logic matches the spec.

TEST GAP: repo-wide grep for `apply_if_orderly`/`next_in_sequence` returns only 3 hits — the definition (state.rs) and two PRODUCTION call sites (dessplay-rendezvous/src/server.rs:854, dessplay/src/actors/sync.rs:674). No test file references either. The convergence harness in dessplay-core/src/test_support.rs (run_cluster/deliver_one/server_poll) applies all ops via state.apply()/server.apply() (lines 585, 598) — the reliable FIFO path, never the datagram fast path. Every `via_datagram:` literal in the sync.rs test module (lines 811, 875, 916, 963, 1024, 1038, 1077, 1101, 1118) is `false`; grep for `via_datagram: true` across the whole repo returns zero matches, so no unit or actor test drives the datagram-ordered branch. The sim_transport.rs datagram tests exercise the transport layer (raw-byte loss/jitter/reorder of `b"gone"`/`&[i]`), not CRDT op application.

SPEC: sync-state.md lines 160-169 ('Delivery requirements'/'Phase 4 constraint') confirm this is a correctness requirement — without per-origin ordering 'later dots mask earlier ones and ops are lost silently.' Project CLAUDE.md mandates extra coverage for CRDT convergence and datagram delivery, exactly this code's domain.

SEVERITY: medium is right. It is a coverage gap, not an active bug (the logic is currently correct, as the reviewer and I both verified), so not high/critical; but it is more than low because the untested code is the sole guard against silent op loss in a project-designated high-risk subsystem, and a plausible regression (off-by-one in the dot check, or misfiling a Map-backed variant into the order-free arm) would pass CI undetected.

</details>

### ⚪ LOW · Module doc names rebalance_playlist as the compaction path, but compaction never calls it

**`dessplay-core/src/playlist.rs:4-8`** · _spec-mismatch_

The module-level doc states the server stops the rationals from growing by reassigning small fresh identifiers "at compaction ([`CrdtState::rebalance_playlist`])". The actual production compaction path is `compact::rebuild` (dessplay-core/src/compact.rs), invoked from `compact_state` in dessplay-rendezvous/src/server.rs:1078, which rebuilds the playlist from scratch with `push_playlist_entry` (producing fresh 0,1,2,… identifiers) and never calls `rebalance_playlist`. A full-repo search confirms `rebalance_playlist` has no production caller — only the fuzz target (fuzz_targets/playlist_identifier.rs:23), the property test (tests/playlist_props.rs:92), and the unit test (playlist.rs:290). A maintainer tracing the compaction/rebalance mechanism via this comment will look for a call that does not exist. (The design doc sync-state.md:242-245 is accurate: it describes the behavior, which compact::rebuild does satisfy; only this in-code comment names the wrong, unused function.)

- **Spec:** playlist.rs module doc lines 4-8: "the server reassigns small fresh identifiers at compaction ([`CrdtState::rebalance_playlist`])" — contradicted by compact::rebuild / server.rs compact_state, which use push_playlist_entry.
- **Suggested fix:** Either correct the comment to reference compact::rebuild's push-based rebuild, or wire rebalance_playlist into the compaction path if it is meant to be the mechanism. Consider deleting the now-unused public method if compact::rebuild is the intended approach.

<details><summary>Verification trail — code pointers</summary>

Verified independently. playlist.rs:4-8 module doc claims the server "reassigns small fresh identifiers at compaction ([`CrdtState::rebalance_playlist`])". A full-repo grep for `rebalance_playlist` (--include="*.rs") returns only 5 hits: the doc link (playlist.rs:7), the definition (playlist.rs:119), and three test-only callers — fuzz_targets/playlist_identifier.rs:23, tests/playlist_props.rs:92, and the unit test at playlist.rs:290. There is no production caller. The actual compaction path is server.rs:1055 -> compact_state -> server.rs:1078 `dessplay_core::compact::rebuild(...)`. compact::rebuild (compact.rs:35) rebuilds the playlist by iterating view.playlist in order and calling push_playlist_entry (compact.rs:44-55); push_playlist_entry (playlist.rs:85-93) appends after the last entry, yielding fresh sequential identifiers — the same flat rebalanced layout. compact.rs:26-27 even describes this as "positions are rebalanced to small flat identifiers" without naming rebalance_playlist. The design doc sync-state.md:241-245 is accurate (describes behavior, names no function), so compact::rebuild satisfies the spec; only the in-code doc-link names the wrong, unused (test-only) function. Behavior is correct; this is purely a stale documentation reference that would mislead a maintainer tracing the compaction/rebalance mechanism. Severity low is appropriate — no runtime effect.

</details>

### ⚪ LOW · Server re-broadcasts every eager op twice instead of deduplicating the reliable+datagram copies

**`dessplay-rendezvous/src/server.rs:840-867`** · _spec-mismatch_

Each ordinary (non-position) op is sent eager = both the reliable control stream AND a datagram (dessplay/src/actors/network.rs send_eager, lines 289-301). The server's recv() loop surfaces Control and Datagram as two separate StateOp events (server.rs:779-781), so it processes both copies. On the second copy the op is already applied, yet the server still broadcasts it: the reliable path hardcodes `applied = true` (server.rs:856-857) regardless of whether apply changed anything, and for order-free ops the datagram path's CrdtState::apply_if_orderly returns `true` even when the element was already present (state.rs:725-733). Net effect: essentially every eager op is broadcast to all other clients twice (~2x op fan-out for a 5-peer group). Convergence is unaffected because client application is idempotent, but it contradicts the documented 'the server deduplicates, applies, and broadcasts' and doubles steady-state op traffic. Map-op duplicates arriving via datagram-after-reliable are correctly suppressed (apply_if_orderly returns false), which makes the order-free/reliable asymmetry the leak.

- **Spec:** sync-state.md, Operation Broadcast step 4: 'The server deduplicates, applies, and broadcasts to other clients.'
- **Suggested fix:** Have the apply paths report whether the op actually changed state (CrdtState::apply / apply_if_orderly returning a 'changed' bool, e.g. compare view_hash or check the register/GSet pre/post), and gate broadcast_op on that, so the second (no-op) copy of an eager op is not rebroadcast.

<details><summary>Verification trail — code pointers</summary>

The code mechanism is real and reproduces exactly as described. Verified chain: (1) sync.rs:599 dispatches every non-position op as NetworkCommand::SendEager; (2) network.rs:289-301 send_eager sends the op on the reliable control stream AND as a datagram (when it fits — small order-free ops fit); (3) server.rs:779-781 surfaces Control and Datagram as two separate StateOp events feeding the same ServerControl::StateOp arm; (4) on the reliable path server.rs:855-857 does `state.apply(op.clone()); true` (applied hardcoded true); (5) on the datagram path for order-free types, state.rs:730-733 (`CrdtOp::NowPlaying | SeekAuthority | PlaybackIntent | LookupRequest | Chat | AcknowledgeAbsent`) does `self.apply(op); true` — unconditionally true even when the value was already present; (6) server.rs:860-867 only gates on `applied` then calls broadcast_op, which (server.rs:214-235) has no op-identity dedup and re-sends reliable+datagram to every other conn. So when both copies of an order-free eager op arrive, the server broadcasts it twice. Map ops are guarded by next_in_sequence (state.rs:687-699), so their datagram-after-reliable duplicate returns false and is suppressed — exactly the asymmetry the claim identifies. This contradicts sync-state.md:608 ('The server deduplicates, applies, and broadcasts to other clients'). Severity low is correct: client application is idempotent so convergence is unaffected (sync-state.md:712-714 notes datagrams are a pure optimization). I down-weight the claim's 'doubles steady-state op traffic' framing — steady-state high-frequency traffic is playback positions, which are CrdtOp::PlaybackPosition, a guarded MAP op (state.rs:723) whose duplicates ARE suppressed; only the low-frequency order-free control ops (chat, now-playing, seek authority, intent, lookup, ack) are double-broadcast. Real and actionable but minor; low stands.

</details>

---

## Core — Franchise grouping

**Files:** `dessplay-core/src/franchise.rs`.
**Read first:** design.md → *Franchise relations* (only `RelationKind::groups_franchise` edges merge; the Isekai-Quartet crossover case is deliberately excluded).
**Key entry points:** `franchises()` (union-find over relation edges), `FranchiseCache` + `inputs_fingerprint()` (memoization — see the existing perf memo). 
**Theme:** one memoization test-gap; the fingerprint code itself is correct. Related: see [[tui-lag-franchises-recompute]] in the project memory for why this is perf-sensitive.

### ⚪ LOW · Memoization regression test never exercises a series_relations change

**`dessplay-core/src/franchise.rs:384-437`** · _test-gap_

inputs_fingerprint() hashes both anidb_metadata AND series_relations (lines 270-277), and franchises() output depends on series_relations for the entire connected-components grouping. The only memoization regression test, cache_recomputes_only_when_inputs_change, seeds relations once *before* the cache is created (line 390) and afterward mutates only playback_position and anidb_metadata. It never changes series_relations after the cache exists, so it asserts nothing about the relations half of the fingerprint. A regression that dropped series_relations from inputs_fingerprint would leave the grouping stale after a relations-only update -- a realistic scenario, since the server fills in the relations graph over hours (design.md 1271-1278) while a series' file metadata stays unchanged (e.g. a newly-arrived sequel/prequel edge that should merge two existing franchises) -- and this test would still pass.

- **Spec:** docs/design.md 1289-1295 (franchise grouping must reflect the relations graph as it fills in); franchise.rs 242-258 cache contract that get() recomputes when either input changes
- **Suggested fix:** Extend cache_recomputes_only_when_inputs_change (or add a sibling test) to call state.set_series_relations(...) after the cache is built -- e.g. add a Sequel edge that merges two previously-separate file-bearing series -- and assert cache.recomputes increments and the returned grouping reflects the new merge.

<details><summary>Verification trail — code pointers</summary>

Independently confirmed against /home/svein/dev/dessplay/dessplay-core/src/franchise.rs. (1) inputs_fingerprint (lines 264-279) hashes anidb_metadata (270-273) AND series_relations (274-277). (2) franchises() depends on series_relations for grouping: union-find over relation edges at lines 72-84 (only groups_franchise edges union), so a relations-only edge merges two file-bearing franchises while the metadata map is unchanged — a realistic case given the module docstring (lines 5-9: "The graph fills in slowly... groupings must degrade gracefully") and design.md. (3) The only memoization regression test is cache_recomputes_only_when_inputs_change (lines 384-437); grep confirms no other test references FranchiseCache/recomputes. It seeds relations once at lines 390-395 BEFORE the cache is created at line 403, then mutates only playback_position (loop 408-419) and anidb_metadata (lines 426-431). It never changes series_relations after the cache exists. (4) Tracing a regression that drops lines 274-277: first get() recomputes unconditionally because key goes None->Some (line 248), the position ticks leave the metadata-only fingerprint unchanged so recomputes stays 1 (assertion line 421 passes), and the KonoSuba metadata change still bumps recomputes to 2 (assertion line 434 passes). Thus the test passes even with series_relations removed from the fingerprint — the relations half of the cache contract (documented lines 242-245) is genuinely unasserted. This is a real, actionable test-coverage gap; the production fingerprint code itself is correct, so low severity is appropriate.

</details>

---

## Core — ed2k hashing

**Files:** `dessplay-core/src/hash.rs`.
**Read first:** design.md → *Key Definitions / FileId* (eMule/AniDB "red" variant; trailing empty-block hash for exact-multiple sizes).
**Theme:** code is correct (the module doc is precise); the single finding is a doc-precision fix in design.md (empty-file edge of "exact multiple").

### ⚪ LOW · "exact multiple" in FileId spec literally includes zero, but a 0-byte file must NOT get the trailing empty-block hash

**`docs/design.md:1650-1652`** · _spec-mismatch_

The Key Definitions / FileId paragraph states the red variant adds a trailing empty-block hash for "files whose size is an exact multiple of the 9,728,000-byte block size." Zero is a multiple of the block size, so a literal reading prescribes appending the empty block for an empty file. That would be wrong: the canonical ed2k hash of a 0-byte file is MD4 of the empty string (31d6cfe0d16ae931b73c59d7e0c089c0), with no trailing block. The code is correct (hash.rs:106 routes the single empty block via the `size_bytes < ED2K_BLOCK_SIZE` guard and returns MD4(empty) directly; hash.rs:80-82 ensures the empty file yields exactly one block), and the module doc at hash.rs:5 correctly says "exact non-zero multiple." The design doc wording is the stale/imprecise side and could mislead a reimplementer (e.g. a server or alternate client) into producing a non-AniDB-compatible hash for empty files.

- **Spec:** design.md FileId (Key Definitions): "Computed with the eMule/AniDB (\"red\") ed2k variant — files whose size is an exact multiple of the 9,728,000-byte block size include a trailing empty-block hash"
- **Suggested fix:** Change "exact multiple" to "exact non-zero multiple" in design.md to match hash.rs (which special-cases the zero-byte file to MD4(empty)).

<details><summary>Verification trail — code pointers</summary>

Verified against code and both doc sites. CODE IS CORRECT: hash.rs:80-82 makes a 0-byte file produce exactly one block (MD4 of empty), and root_from_blocks (hash.rs:104-118) matches the arm `[single] if size_bytes < ED2K_BLOCK_SIZE` at line 106 for a 0-byte file (0 < 9_728_000), returning the single block hash directly and BYPASSING the `is_multiple_of(ED2K_BLOCK_SIZE)` empty-block append at lines 112-114. Test `empty_file_has_known_hash` (hash.rs:140-146) asserts root == 31d6cfe0d16ae931b73c59d7e0c089c0 (canonical MD4 of empty) with blocks.len()==1. The internal module doc (hash.rs:4-5) correctly says "exact NON-ZERO multiple." DESIGN DOC IS THE IMPRECISE SIDE: design.md:1650-1652 says "files whose size is an exact multiple of the 9,728,000-byte block size include a trailing empty-block hash" — omitting "non-zero." Zero is a multiple of the block size, so a literal reading prescribes appending the empty block for an empty file, yielding MD4(MD4("")||MD4("")) instead of the canonical MD4("") — a non-AniDB-compatible hash. The mismatch between the precise module doc and the imprecise design doc, plus the mathematically-wrong literal reading for empty files, confirms the spec-precision issue. Practical impact is narrow (empty files are not real videos, and the in-repo server reuses dessplay-core hashing), so low severity is correct; the only exposure is an external reimplementer reading the design doc literally.

</details>

---

## Core — Networking & transport

**Files:** `dessplay-core/src/net/{quic,transfer,transport,framing,message,timesync,tofu}.rs`.
**Read first:** network-design.md → *QUIC Transport / Channel Usage* (stream priority, per-stream flow control), *Transfer Stream / Relay Envelope*.
**Theme:** both findings are spec-drift, not data-loss: the server never elevates its control-stream priority (so a bulk relay download can add latency to state sync in the server→client direction), and a stale doc-comment names a `WireMessage::Relay` variant that the design deliberately does not have.

### ⚪ LOW · Doc comment claims a WireMessage::Relay variant that does not (and will not) exist

**`dessplay-core/src/net/message.rs:16-22`** · _spec-mismatch_

The module/type doc comment states 'Phase 9 adds a `Relay` variant for file transfer envelopes', but WireMessage only ever has the Control variant. Relay traffic is intentionally carried as RelayEnvelope frames on a dedicated relay stream (net/transfer.rs, exercised in actors/network.rs run_relay_reader / SendPeer), never wrapped in WireMessage. The comment is factually stale and would send a maintainer looking for a WireMessage::Relay path that is architecturally absent.

- **Spec:** network-design.md, Transfer Stream / Relay Envelope: relay envelopes ride a separate QUIC stream, not the WireMessage control channel.
- **Suggested fix:** Drop or correct the comment to note that relay envelopes are framed on a dedicated relay stream (RelayEnvelope), not as a WireMessage variant.

<details><summary>Verification trail — code pointers</summary>

message.rs:16-17 doc comment says "Phase 9 adds a `Relay` variant for file transfer envelopes," but the enum at lines 19-22 has only `Control(ServerControl)`. A grep of `WireMessage::` over dessplay-core/src yields only `Control` — no `Relay` variant exists. Relay traffic is instead carried by a distinct `RelayEnvelope` type (net/transfer.rs:250, variants Forward/Forwarded/Hello) over a dedicated relay BiStream: dessplay/src/actors/network.rs run_relay_reader (line 306) decodes RelayEnvelope frames, and SendPeer (lines 503-511) wraps peer messages in RelayEnvelope::Forward — never in WireMessage. The spec (docs/network-design.md:501 "one dedicated relay BiStream", :540 "distinct QUIC stream from control", :585-598 RelayEnvelope definition) confirms relay deliberately rides a separate stream off the WireMessage control channel, so a WireMessage::Relay variant is architecturally contrary to the design, not merely unimplemented. The comment is factually stale and would send a maintainer looking for a nonexistent code path. Low severity is correct: it is a misleading comment with no functional impact.

</details>

### ⚪ LOW · Server never prioritizes its control stream above transfer/relay streams

**`dessplay-core/src/net/quic.rs:304-326`** · _spec-mismatch_

set_priority(CONTROL_PRIORITY) is only ever called on the client side, in QuicConnector::connect (quic.rs:255). The server path (QuicListener::accept -> QuicTransport::new at lines 304-326 / 72-93) constructs the transport from the accepted control stream and never calls set_priority on its control SendStream, and the relay/transfer streams the server writes ChunkData on (accepted via recv()->accept_bi) are also at quinn's default priority 0. Because the rendezvous server is the relay hub, server->client is exactly the direction that carries BOTH bulk relayed ChunkData and state-sync control traffic (StateOp/StateHash/PeerList/Chat) to a downloading peer. With both at priority 0, quinn's send scheduler interleaves them fairly instead of giving control precedence, so during a sustained download from the seeder (the documented common case) state-sync updates to that peer are delayed. The spec's explicit goal -- 'a bulk download never starves state sync' -- is therefore not met on the direction that matters most. (Per-stream flow control still prevents true head-of-line blocking, so this is latency-under-load, not data loss.)

- **Spec:** network-design.md, QUIC Transport > Channel Usage: 'Stream priority: the control stream is prioritized above transfer streams (quinn set_priority), so a bulk download never starves state sync.' (and Flow Control: 'The relay stream is a distinct QUIC stream from control, so QUIC handles backpressure per-stream without starving control traffic.')
- **Suggested fix:** Move the set_priority(CONTROL_PRIORITY) call into QuicTransport::new so both client and server elevate their control SendStream, or call send.set_priority(CONTROL_PRIORITY) in QuicListener::accept after accept_bi succeeds.

<details><summary>Verification trail — code pointers</summary>

Confirmed against code and spec. (1) `set_priority(CONTROL_PRIORITY)` appears exactly once in the whole crate (grep over dessplay-core/src): quic.rs:255 in QuicConnector::connect, on the CLIENT's control send stream. (2) The server's QuicListener::accept (quic.rs:304-334) accepts the control stream via conn.accept_bi() and passes `send` into QuicTransport::new(conn, send, recv) at line 326 with NO set_priority — so the server control send stream is quinn default priority 0. (3) Relay/transfer streams are BiStreams opened via open_stream()/open_bi (line 134-144) and accepted server-side via accept_bi() in recv() (line 160-166); neither path sets priority, so the server's relay send stream is also priority 0, equal to its control stream. (4) shared_transport_config() (flow-control windows) IS applied to both endpoints (lines 207, 288), confirming the asymmetry is specifically the per-stream set_priority being wired only into the client connect path. The server is the relay hub, so server->client carries BOTH bulk relayed ChunkData and control traffic (PeerList/StateHash/StateMerge/StateOp/Chat) to a downloading peer — both at priority 0. This contradicts network-design.md:95-96 ('the control stream is prioritized above transfer streams (quinn set_priority), so a bulk download never starves state sync'), whose own rationale (bulk download = server->client) names exactly the unprioritized direction. The finding is real and the fix is a one-line set_priority on the server's accepted control send stream.

</details>

---

## Client — Player actor & mpv

**Files:** `dessplay/src/actors/player.rs`, `dessplay/src/player/mpv.rs`, `player/mock.rs`.
**Read first:** design.md → *Player Integration* (echo suppression — commanded vs user-originated events; crash escalation 1st/2nd/3rd; attach mode), *Subtitle Display*, architecture.md → *PlayerActor*.
**Key entry points:** `estimate_now` / `note_position` / `believed_pause` (the position estimator — anchor the estimate *before* mutating pause state, the way `apply_desired_pause`/`set_speed` do), `handle_pause_observation`, `handle_player_death` (crash counter), `wait_for_socket` (attach re-attach).
**Theme:** one **High** correctness bug (the pause re-anchor that can make this client the spurious drift leader) and an attach-mode spec-mismatch where a re-attach is mis-counted as a crash and gives up after 10s.

### 🔴 HIGH · User pause/unpause re-anchors the position estimate with the NEW pause state, jumping position forward by the whole pause duration

**`dessplay/src/actors/player.rs:561-566`** · _bug_

handle_pause_observation sets `self.believed_pause = Some(paused)` (line 562) BEFORE calling `estimate_now()` (line 564) to re-anchor. estimate_now() extrapolates iff believed_pause == Some(false). So on a user UNPAUSE, it computes `est.millis + (now - est.at)*speed` using the new (playing) state, but `est.at` was anchored at the moment of the previous pause — so it adds the ENTIRE paused interval as phantom playback. After a 5-minute bathroom break, a user-initiated unpause snaps the estimate ~5 minutes ahead. This is the opposite ordering from apply_desired_pause (estimate at 396, then set state at 399) and set_speed (413 then 416), which correctly anchor with the old state. The bogus estimate feeds PositionTick (broadcast to peers for leader election / drift), subtitle MM:SS timestamps, and crash-restore position; a single bad datagram can make this client look like the furthest-ahead leader and trigger a group-wide forward hard-seek before mpv's next time-pos event overwrites it. (Our own pause flips go through apply_desired_pause first, so echoes are pre-anchored correctly; only genuine user flips after a real paused interval hit this.)

- **Spec:** design.md Playback Rules / drift correction: the position reference and leader election follow the furthest-ahead present peer's broadcast position; architecture.md PlayerActor: PositionTick is 'extrapolated between player reports'
- **Suggested fix:** Capture the settle position before mutating state: `let settled = self.estimate_now();` then `self.believed_pause = Some(paused);` then `if let Some(p) = settled { self.note_position(p); }` — mirroring apply_desired_pause/set_speed.

**Status (2026-06-27): fixed.** `handle_pause_observation` now snapshots `estimate_now()` *before* flipping `believed_pause`. The shared "settle the estimate, then mutate the dependent state" invariant — previously open-coded at three sites (`apply_desired_pause`, `set_speed`, and this one) — was extracted into a single documented `reanchor_estimate()` helper, so a fourth site can't silently reintroduce the jump. Regression tests added (`dessplay/src/actors/player.rs`): a proptest `user_unpause_never_counts_the_paused_interval_as_playback` and an end-to-end run-loop test `unpausing_in_mpv_after_a_long_pause_does_not_jump_the_broadcast_position` (broadcast was `310000` for a 5-min pause at 10s pre-fix). Commit `86f884`.

<details><summary>Verification trail — code pointers</summary>

CONFIRMED. player.rs:561-566 `handle_pause_observation` sets `self.believed_pause = Some(paused)` (562) BEFORE `estimate_now()` (564). `estimate_now()` (299-307) extrapolates `est.millis + est.at.elapsed()*speed` only when `believed_pause == Some(false)`. On a user unpause (`paused=false`), believed_pause is already flipped to Some(false) when estimate_now runs, so it extrapolates from `est.at`. `est.at` is the pause moment because the only thing refreshing it is `PlayerEvent::Position` (488-490), which comes solely from mpv `time-pos` property-changes (mpv.rs:454-460) — and mpv freezes time-pos while paused, emitting no property-changes. So after a 5-min pause the unpause re-anchors to `M0 + 5min`. The ordering is the inverse of the two correct sites: apply_desired_pause estimates at 396 then sets state at 399; set_speed at 413 then 416. The 'our own flips are safe' qualifier holds: a dessplay-driven unpause runs apply_desired_pause first (re-anchoring est.at to now before the echo), while a direct mpv unpause early-returns at 388 without re-anchoring — exactly the attach-mode/normal 'press space in mpv' flow. Downstream confirmed: next cadence tick (player.rs:250/269, ≤100ms, due-threshold drops to 100ms on unpause) emits PositionTick(M0+5min) if it beats the first post-resume Position event (a real ~100ms race); session.rs:1459-1464 turns it into a broadcast SetPlaybackPosition since the client holds now-playing; position_reference elects leader via max_by_key(position_millis) (1048-1050) so this client becomes furthest-ahead leader; peers' drift_correct sees magnitude > DRIFT_HARD_SEEK_MILLIS and hard-seeks forward (player.rs:457-460). Secondary harm: maybe_record_watched(M0+5min) (1465) can falsely mark the file watched past 85%. Every particular of the claim checks out; the fix is reordering to match apply_desired_pause/set_speed (estimate with old state, then set believed_pause), which also fixes a symmetric minor under-count on the pause direction.

</details>

### 🟠 MEDIUM · Attach mode: a re-attach that doesn't reconnect within 10s exits the actor, and attached-mpv restarts are counted as crashes

**`dessplay/src/actors/player.rs:623-690`** · _spec-mismatch_

In attach mode (MpvFactory::Attach), supervise_attached emits Exited{clean:true} whenever the user's mpv closes its socket, driving handle_player_death. Two consequences contradict the spec's 'the relaunch path re-attaches, waiting for the user's mpv to come back': (1) handle_player_death increments consecutive_crashes for every attached-mpv close (lines 639-664), so a developer restarting their mpv twice within 30s triggers PlayerOutput::FatalCrash — a SYNCED 'my player crashed -- pausing' chat message that pauses the whole group — and three times triggers GaveUp; these are normal attach-mode events, not crashes. (2) The relaunch re-attaches via factory.spawn() -> MpvPlayer::attach -> wait_for_socket, which has a hard 10s SOCKET_WAIT deadline (mpv.rs:42,194-210); if the user's mpv is down longer than 10s the attach errors, the Err arm (lines 685-690) emits FatalCrash and returns false, so run() breaks and the player actor terminates entirely — it does NOT keep waiting for mpv to return.

- **Spec:** design.md Player Integration, Attach mode: 'dessplay never quits an attached mpv on shutdown ...; if that mpv dies, the relaunch path re-attaches, waiting for it to come back.'
- **Suggested fix:** In attach mode, treat socket-close as a transient detach: don't count it toward consecutive_crashes/FatalCrash, and retry the re-attach indefinitely (loop wait_for_socket) instead of exiting after one 10s timeout.

**Status (2026-06-28): fixed.** The player actor now knows whether its player is owned (spawn) or attached, via a new `PlayerFactory::is_attach()`. `handle_player_death` splits into two explicit paths: spawn mode keeps the unchanged crash escalation (silent relaunch → FatalCrash on the 2nd death in 30s → GaveUp on the 3rd); attach mode treats a socket close / `Exited` as a **transient detach** — it never touches `consecutive_crashes`, never emits FatalCrash/GaveUp (so it never writes the synced "my player crashed — pausing" chat or pauses the group), and instead enters a re-attach state (`reattach_at`/`reattach_backoff`) driven by a new run-loop select arm that re-probes the socket with capped backoff (500 ms → 10 s) **indefinitely**, re-attaching and reloading (position/pause restored on `Loaded`) once mpv returns — no FatalCrash, no actor exit after 10 s. The same waiting state now also absorbs an attach-mode *startup* race (mpv not up yet), which previously FatalCrash-paused the group and exited. Regression tests added in `dessplay/src/actors/player.rs`: `attach_mode_socket_close_is_a_transient_detach_not_a_crash` (two quick detaches emit no FatalCrash/GaveUp and re-attach each time) and `attach_mode_waits_indefinitely_for_mpv_to_return` (mpv down well past 10 s keeps retrying, then re-attaches) — both `[FatalCrash]` vs `[]` before the fix. The existing spawn-mode trio (`crash_relaunches_…`, `second_crash_within_window_is_fatal`, `third_crash_within_window_gives_up_then_recovers_on_new_file`) is the no-regression net and still passes unchanged; `MockFactory` gained `attach()`/`then_down()`/`then_up()` to script attach + down-then-up deterministically.

<details><summary>Verification trail — code pointers</summary>

Verified both consequences against the code; the spec promise (docs/design.md:1441-1443, "dessplay never quits an attached mpv on shutdown ...; if that mpv dies, the relaunch path re-attaches, waiting for it to come back") is genuinely violated.

CONSEQUENCE 1 (attached-mpv restarts counted as crashes) — CONFIRMED:
- supervise_attached (mpv.rs:299-309) emits PlayerEvent::Exited{clean:true} whenever the read loop ends, i.e. whenever the user's mpv closes the socket — the *normal* event for an attach-mode mpv restart.
- That reaches handle_player_event (player.rs:554-555) -> handle_player_death(clean=true). In handle_player_death the `clean` flag only changes the log line (player.rs:628-632); consecutive_crashes is incremented unconditionally (player.rs:639-648). The actor is generic over PlayerFactory and has no attach-awareness, so there is no guard distinguishing a deliberate re-launch from a crash.
- consecutive_crashes==2 -> PlayerOutput::FatalCrash (player.rs:650-655); ==3 (CRASH_GIVE_UP_COUNT, player.rs:65,656) -> PlayerOutput::GaveUp. FatalCrash/GaveUp are turned into SYNCED Mutation::Chat ("my player crashed — pausing" / "...keeps crashing — giving up...") plus SetPlaybackIntent::Paused in session.rs:1521-1537, which pauses the whole group. So a dev restarting their mpv twice within 30s (CRASH_FATAL_WINDOW, player.rs:61) fires a group-pausing synced chat message; three times stops relaunching. These are normal attach events, not crashes.

CONSEQUENCE 2 (mpv down >10s exits the actor) — CONFIRMED:
- Relaunch calls factory.spawn() (player.rs:673) -> Mode::Attach -> MpvPlayer::attach (mpv.rs:696) -> wait_for_socket (mpv.rs:107). wait_for_socket has a hard SOCKET_WAIT=10s deadline (mpv.rs:42,194-210) and returns Err on timeout — there is no retry loop.
- The Err arm of handle_player_death (player.rs:685-690) emits FatalCrash and returns false. Returning false propagates out of handle_player_event (player.rs:555) and breaks the run loop (player.rs:265-266), after which run() falls through to exit (player.rs:279-283). The factory is taken once on first Load (session.rs:1549-1550), so the actor is not respawned. Hence if the user's mpv stays down >10s the player actor terminates instead of "waiting for it to come back."

The code comment at mpv.rs:296-298 even states the intended behavior ("re-attaches, waiting for the user's mpv to come back"), confirming the implementation falls short of both spec and its own stated intent.

Severity: keeping medium. It is confined to attach mode (an explicit "dev/headless aid", interactive-only), which argues toward low, but the FatalCrash is a *synced* message that pauses every real participant in the group, so the blast radius is not limited to the developer. Medium is appropriate.

</details>

---

## Client — File actor: cache, retention, matching, archive

**Files:** `dessplay/src/actors/file.rs` (large — read in chunks).
**Read first:** design.md → *File Matching*, *Download Cache and Retention*, *Archive*, *Content Hash*, *Manual File Mapping*.
**Key invariant (the cluster theme):** the servable-set map `self.local_files`, the `hash_cache` table, and the `FileAvailability` register must stay in lockstep through *every* mutation. Five sites mutate the servable set (reconcile, download-complete, resolve-verified, manual-map, **archive**); **archive is the lone site that forgets `local_files`**, which can spuriously flip a still-held file to Missing and gate the group. The serve paths (`serve_block_hashes`, `drain_serve_queue`) and the runtime guards (`lost_local_file`, load-failure) are where the inconsistency bites.
**Note:** the archive bug surfaced in two reviewers (cache + matching) — same root cause, fix once (`self.local_files.insert(file, dest)`).

### 🟠 MEDIUM · Archive does not update local_files; a held file can spuriously flip to Missing

**`dessplay/src/actors/file.rs:1287-1310`** · _bug_

archive_inner() moves the file from the hash-named cache path (entry.path) to dest = <download_root>/<series>/<filename> and re-keys hash_cache to dest, but it never updates self.local_files. local_files[file] keeps pointing at entry.path, which move_file just deleted. The four other servable-set mutations (reconcile 493, download-complete 791, resolve-verified 984, manual-map 1243) all keep local_files in sync; archive is the one path that leaves it dangling. The session's Archived handler (session.rs:2022) only posts a chat notice and does NOT re-resolve, so the stale entry persists until an unrelated Resolve for that file happens. Consequence: if any peer sends a BlockHashRequest/ChunkRequest for that file in the meantime, serve_block_hashes (831/841) / drain_serve_queue (933/937) read the dead path, see path.exists()==false, and call lost_local_file(), which flips our own FileAvailability to Missing even though we hold the archived copy. We stop serving the file, and if it is the now-playing file we become a Missing-file blocker that pauses the whole group until the next on_state round-trip re-resolves dest and flips back to Ready.

- **Spec:** design.md Download Cache and Retention / Archive: "Archiving moves the file into the library, so the marker clears" — the file stays a Ready, servable local copy at its new path.
- **Suggested fix:** After a successful move in archive_inner, insert the new location: self.local_files.insert(file, dest.clone()); (mirroring the hash_cache re-key already done at 1297-1309).

**Status (2026-06-28): fixed.** `archive_inner` now does `self.local_files.insert(file, dest.to_path_buf())` right after the move, in lockstep with the existing `hash_cache` re-key, so the archived copy stays a held, Ready, servable file at its new path. Same one-line root cause as the LOW finding below — fixed together. Regression test `archived_file_stays_servable_to_peers` (`dessplay/src/actors/file.rs`): after archiving a cached download, a peer `BlockHashRequest` is served (block hashes + complete-bitfield advertisement) instead of flipping our own `FileAvailability` to Missing; pre-fix it panicked with "archived file spuriously flipped to Missing on a peer serve request".

<details><summary>Verification trail — code pointers</summary>

Verified against /home/svein/dev/dessplay/dessplay/src/actors/file.rs and session.rs. archive_inner (file.rs:1261-1311) moves the file from entry.path to dest via move_file (1464-1473, which renames or copy+remove_file, deleting the source), removes the cache entry (1289), and re-keys hash_cache to dest (1294-1309) — but never updates self.local_files. The four other servable-set writers DO keep it in sync: reconcile local_files.insert(entry.hash, entry.path) at 493, download-complete at 791, resolve-verified at 984, manual-map at 1243. Archive is the lone gap; local_files[file] keeps pointing at the now-deleted entry.path. The serve paths confirm the consequence: serve_block_hashes reads self.local_files.get(&file) (831) and bails via lost_local_file when !path.exists() (841-843); drain_serve_queue does the same (933-940). lost_local_file (885-905) flips our own FileAvailability to Missing and drops the file. The session Archived handler (session.rs:2022-2031) only posts a chat notice and issues no Resolve. Recovery is in fact weaker than the claim states: the on_state resolve loop is gated on !self.resolved.contains_key (1143), and the archived file is still Resolution::Verified(old_path) in self.resolved; FileOutput::Availability{Missing} (2006-2014) only writes the CRDT register and does not clear self.resolved. Only LoadFailed (1504) or note_evicted (1312) clear it, and LoadFailed won't fire for the now-playing file since mpv's open fd survives the rename — so the spurious Missing can persist. Spec (design.md, Download Cache and Retention / Archive: 'Archiving moves the file into the library, so the marker clears') intends the archived copy to remain a held, Ready, servable file. The fix is the same one-liner the other four sites use: self.local_files.insert(file, dest). The existing test archive_moves_the_file_and_rekeys_bookkeeping (2284) only checks the move/rekey, not local_files, which is why this slipped through. Medium severity is fair: the race requires a concurrent peer serve request for the just-archived file; worst case (now-playing) it makes us a Missing blocker and pauses the group.

</details>

### ⚪ LOW · archive_inner re-keys hash_cache to the new path but leaves local_files pointing at the moved-away cache file

**`dessplay/src/actors/file.rs:1287-1311`** · _bug_

archive_inner moves the cached file from entry.path to dest, removes the cache_entries row, and re-keys hash_cache from entry.path -> dest, but it never updates self.local_files[file] (which still maps file -> entry.path, the now-vanished cache path). The actor still holds the file (at dest) and the session still advertises Ready (archiving issues no resolve/availability change, and the resolve guard `self.resolved.contains_key` keeps the old Verified(old_path) in place). If a peer that is downloading from us then sends a ChunkRequest/BlockHashRequest, drain_serve_queue/serve_block_hashes sees local_files[file]=old_path, finds path.exists()==false, and calls lost_local_file -> emits FileAvailability::Missing for a file we actually still hold. When the archived file is now-playing this transiently flips us to Missing and gates the group until a later re-resolve (triggered by a player reload / LoadFailed) rediscovers it at dest. The other archive bookkeeping (DB cache_entry, hash_cache) is updated; only the in-memory servable map is left stale.

- **Spec:** Download Cache and Retention: 'Archive ... moves a cached file into [Series name]/[Original filename]'; the file remains held/servable after archiving, so its availability must stay Ready.
- **Suggested fix:** In archive_inner, after a successful move, update the servable map: self.local_files.insert(file, dest.to_path_buf()) (replacing the old entry.path).

**Status (2026-06-28): fixed.** Same root cause as the MEDIUM finding above — one `local_files.insert(file, dest)` in `archive_inner`, fixed together. See that finding's Status line for the regression test (`archived_file_stays_servable_to_peers`).

<details><summary>Verification trail — code pointers</summary>

Verified in /home/svein/dev/dessplay/dessplay/src/actors/file.rs. archive_inner (lines 1261-1311) moves the cached file via move_file(&entry.path,&dest) (1287), removes the cache_entries row (1288-1290), removes the old hash_cache row and re-keys it to dest (1294-1309), but never touches self.local_files. The only local_files writes are at 791, 984, 1243 — none on the archive path. local_files for a cached download maps file->cache path (init 479-493; on_download_complete 791). move_file (1464-1473) renames or copies+removes the source, so entry.path is gone after archive. Both serve paths then break: serve_block_hashes (831-844) does local_files.get -> old path, !path.exists() true -> lost_local_file(file); drain_serve_queue (932-942) same (enqueue_serve at 909 only checks contains_key, which still passes). lost_local_file (885-905) drops the mapping and emits FileOutput::Availability{ Missing } for a file actually still held at dest. archive() (1253-1259) emits only FileOutput::Archived — no Resolved/Availability — and nothing re-issues FileCommand::Resolve (only dispatch is line 547, session-driven), so local_files stays stale until an unrelated re-resolve. One claim detail is wrong: there is no self.resolved field in the file actor (the `resolved` tokens at 1703-1713 are local symlink-resolution vars); that guard, if it exists, is in the session layer and is not load-bearing for the file.rs defect. The 'gates the group' effect is conditional on the archived file being now-playing, but the spurious Missing advertisement plus dropped peer serve for a still-held file is confirmed. Low severity is correct: narrow trigger, transient, self-heals on next resolve.

</details>

### ⚪ LOW · Retention window measured from download time, not last access (touch_cache_entry is dead code)

**`dessplay/src/actors/file.rs:1379-1381`** · _spec-mismatch_

evictable()'s CacheRetention::Keep(window) branch compares now - entry.last_access >= window. cache_entries.last_access is written in exactly one production place — on_download_complete (file.rs:796-801) — and never afterward. Storage exposes touch_cache_entry (storage.rs:509) for exactly this purpose, but it is referenced only by a storage unit test (storage.rs:961) and has no production caller anywhere in the repo. So for a finite Keep(window) retention, the eviction clock starts at download time and a file that is repeatedly re-watched is still evicted `window` after it was downloaded, contradicting the "after its last access" wording. The unused touch_cache_entry indicates intended-but-unwired behavior. (Common settings 0/infinite are unaffected; only Keep(window) re-watch is.)

- **Spec:** design.md Download Cache and Retention: "An evictable file is deleted `cache_retention` after its last access."
- **Suggested fix:** Bump last_access when a cached file is loaded/played (e.g. call storage.touch_cache_entry(hash, now) on the load/resolve-of-a-cache-entry path), or amend the doc to say the window runs from download time if that is the intended behavior.

<details><summary>Verification trail — code pointers</summary>

All factual claims verified. (1) file.rs:1377-1383 — evictable()'s Keep(window) branch computes `now.saturating_sub(entry.last_access) >= window.as_millis() as i64`, anchoring the eviction clock on last_access. (2) last_access is written in exactly one production place: on_download_complete at file.rs:796-800 (`last_access: now` at download time). The startup cache reconciliation (file.rs:488-509) only inserts survivors into the in-memory local_files map and does NOT re-write last_access; grep over all last_access/upsert_cache_entry/touch_cache_entry sites shows no serve-time or playback-time bump anywhere. (3) storage.rs:509 touch_cache_entry ("Bump an entry's last-access time") has no production caller — its only reference is storage.rs:961, inside the `#[test]` block starting at storage.rs:944 — confirming dead, intended-but-unwired code. Consequently, for a finite Keep(window) retention the window effectively runs from download time, never extended by re-watching or serving, which contradicts design.md (Download Cache and Retention): "An evictable file is deleted `cache_retention` after its last access." Scope is correctly limited: AfterWatch ("0", config.rs:115/123) returns true unconditionally and Infinite returns false, both independent of last_access. Severity low is right: impact is narrow (eviction-timing only; now-playing/queued-unwatched files are protected via the `protected` guard at file.rs:1374, so no in-use file is lost), though I note Keep(7 days) is the *default* (config.rs:107-108), so the affected branch is the common interactive case rather than an exotic one.

</details>

### ⚪ LOW · Player-load-failure runtime guard does not drop the local copy / prune file-actor bookkeeping (only the serve-time guard does)

**`dessplay/src/actors/file.rs:545-631, 885-905`** · _spec-mismatch_

design.md (Download Cache) specifies TWO runtime guards for files deleted mid-session, and says BOTH 'drop the local copy, prune its bookkeeping, and flip the file to Missing so it re-resolves'. The serve-time guard (lost_local_file, 885-905) does all three. The player-load-failure guard is implemented entirely in session.rs (PlayerOutput::LoadFailed, session.rs ~1497-1517): it flips availability to Missing and re-resolves, but the file actor exposes no FileCommand to forget the copy, so on a load failure self.local_files / cache_entries / hash_cache retain the stale entry. The mismatch is benign in practice because it self-heals (the next serve attempt hits lost_local_file, and startup reconciliation prunes the rows), and the user-visible outcome (Missing + re-resolve) is correct -- but the file actor's bookkeeping is not pruned on the load-failure path as the doc states.

- **Spec:** Download Cache and Retention: 'Two runtime guards cover deletions that happen mid-session ... a player load failure (file gone under us) and a serve-time absence ... both drop the local copy, prune its bookkeeping, and flip the file to Missing so it re-resolves.'
- **Suggested fix:** Add a FileCommand (e.g. ForgetLocalFile { file }) that calls lost_local_file, and have the session's LoadFailed handler send it; or relax the doc to state that the load-failure path relies on the serve-time guard + startup reconciliation for the prune.

<details><summary>Verification trail — code pointers</summary>

Verified the asymmetry the claim describes. The serve-time guard `lost_local_file` (file.rs 885-905) does all three spec actions: drops the in-memory copy (`local_files.remove`, 886), prunes DB + in-memory bookkeeping (`remove_cache_entry` 888, `remove_hash_cache` 891, in-memory `hash_cache` rebuild 894-896), and flips availability to Missing (898-904). It is only reachable from serve paths (`serve_block_hashes` 842, `drain_serve_queue` 940). The player-load-failure guard lives in session.rs PlayerOutput::LoadFailed (1497-1517): it clears session/player-wiring state (`self.loaded = None` 1502, `self.resolved.remove` 1504), emits `Mutation::SetFileAvailability{Missing}` (1505-1508), and emits `Directive::Resolve` (1511-1514). The only thing it hands the file actor is `FileCommand::Resolve`. The `FileCommand` enum (file.rs 79-174) has no command to forget a held copy, so the session cannot instruct the actor to prune `local_files`/`cache_entries`/`hash_cache`. And `resolve()`/`Done::Resolved` (976-989, calling `resolve_with_cache` 1480-1513) only *inserts* into `local_files` on `Resolution::Verified` (983-985) and never removes on a NotFound, so the stale rows survive the re-resolve. design.md 1103-1107 states BOTH runtime guards "drop the local copy, prune its bookkeeping, and flip the file to Missing so it re-resolves" — but the load-failure path only flips-to-Missing + re-resolves; it does not drop the file-actor copy or prune the DB/in-memory bookkeeping. This is a genuine spec-vs-code mismatch. It is benign and self-heals exactly as the claim states (a later serve attempt hits `lost_local_file`; startup reconciliation at file.rs 488-505 prunes dead `cache_entries`/`hash_cache` rows), and the user-visible outcome (Missing + re-resolve) is correct, so low severity is accurate.

</details>

---

## Client — File transfer & download

**Files:** `dessplay/src/download.rs`, `dessplay/src/chunkstore.rs`, and the serve side of `dessplay/src/actors/file.rs`.
**Read first:** network-design.md → *File Transfer* (multi-source, rarest-first, partial serving), design.md → *Download Cache* and the Downloading *File State* unpause rule.
**Key entry points:** `progress_and_refill` (source solicitation), `plan_requests` (chunk assignment), `block_hashes` state machine (`Pending`/`Have`).
**Theme — the biggest spec-vs-reality gap in the codebase:** the transfer is effectively **single-source**. `progress_and_refill` only ever solicits `sources.keys().next()`, secondary sources keep empty bitfields and are never asked, and a `Have` block-hash state never re-solicits when the lone source departs → **permanent stall**. The multi-source/rarest-first/partial-serve machinery exists but is dead code until solicitation is fixed. The stale-`assigned`-set duplicate-fetch bug is currently *masked* by the single-source defect and will surface once it's fixed — fix them together.

### 🔴 HIGH · Only one source is ever solicited for availability; multi-source download never happens and stalls if that source departs

**`dessplay/src/download.rs:497-510`** · _bug_

A download only learns a peer's chunk bitfield from a `PeerMessage::FileAvailability`, and the only production site that emits one is `serve_block_hashes` (dessplay/src/actors/file.rs:864-877), sent strictly in reply to a `BlockHashRequest`. But `progress_and_refill` sends `BlockHashRequest` to exactly ONE peer — `d.sources.keys().next()` (line 499) — and never to the others. Consequently every secondary source stays with the empty bitfield it was inserted with in `set_sources` (lines 261-267), is never a candidate in `plan_requests` (the `src.bitfield.get(c)` filter), and is never asked for a single chunk. Effects: (a) the documented 'up to 4 concurrent sources' / rarest-first multi-source transfer never occurs — all bytes come from one peer regardless of how many Ready sources `download_sources` supplied; (b) worse, once block hashes are obtained `block_hashes` becomes `Have`, and if the lone advertising source is then dropped (departure → `set_sources` retains the others but does not reset `block_hashes` to Pending, lines 244-269), no new `BlockHashRequest` is ever issued, the remaining sources keep empty bitfields, and the download stalls permanently even though other present peers advertise the file Ready.

- **Spec:** network-design.md 'Scheduling: pipeline, snub, endgame' / 'Flow Control' ("up to 4 concurrent sources"), 'Chunk Selection: Rarest First' ("count how many sources advertise each missing chunk"), 'Availability Tracking' ("Sent when a peer begins serving a file (complete bitfield)")
- **Suggested fix:** Solicit availability from every source (broadcast BlockHashRequest, or add a dedicated bitfield-want message, or have a source push FileAvailability on first contact from a downloader), and when block_hashes is already Have but a source has an empty bitfield (or the advertising source leaves), re-solicit so remaining sources can take over.

**Status (2026-06-27): fixed (minimal/incomplete).** `progress_and_refill` now solicits a `BlockHashRequest` from **every** source it hasn't yet asked, not just `sources.keys().next()` — and `serve_block_hashes` answers each with both the per-block hashes *and* a full `FileAvailability` bitfield, so one existing message learns every source's availability (no new wire-protocol type). A per-source `Source::solicited` flag drives this; the lone-peer `BlockHashes::Pending { peer, requested_at }` tracking (and its snub-reset) is removed. Re-solicitation now covers a source that joins *after* block hashes validate (empty bitfield, not yet solicited) and a source re-added after departure (a fresh `Source`, re-solicited), so the permanent stall is gone. The `assigned` duplicate-fetch (next finding) was fixed in the same change. Regression tests added in `dessplay/src/download.rs` (`every_source_is_solicited_for_block_hashes`, `a_source_added_after_block_hashes_is_solicited`, `download_completes_after_the_driving_source_departs`) plus the `dessplay/tests/transfer.rs` two-seed test now genuinely exercises both sources. **This is deliberately a minimal correctness fix, not the full transfer design:** true multi-source *concurrency*, rarest-first load-balancing across sources, and partial-serve from still-downloading peers remain **unimplemented** (see the two Low findings below — partial-serve dead code, and the invalid-block-hash prompt re-ask, both **untouched**). The throughput half of the Downloading unpause rule (the Low finding in *Core — Playback gating*) is likewise still unenforced.

<details><summary>Verification trail — code pointers</summary>

All load-bearing claims verified against the code at /home/svein/dev/dessplay/dessplay/ (note the cited paths omit the nested `dessplay/` crate dir).

(1) download.rs:497-509: the BlockHashRequest is emitted to exactly one peer, `d.sources.keys().next().cloned()`, and only while `block_hashes` is `Pending { peer: None }`. This is the ONLY production BlockHashRequest emit (the other at file.rs:2637 is inside a #[cfg(test)] test).

(2) file.rs:830-877 `serve_block_hashes`: the only production `PeerMessage::FileAvailability` emit (line 875), reached only from the BlockHashRequest handler (file.rs:693). The four FileAvailability emits in download.rs (799/849/889/960) are all inside the #[cfg(test)] module that starts at line 630. A grep over src confirms no other production FileAvailability PeerMessage emit and no proactive 'begins serving' advertisement.

(3) download.rs:259-267 `set_sources`: new sources are inserted with an empty `Bitfield::new(...)`; the only place a source bitfield is populated is the FileAvailability handler (download.rs:287-289).

(4) download.rs:585-594 `plan_requests`: candidacy requires `src.bitfield.get(c)`, so an empty-bitfield source is never requested any chunk and never contributes to rarity (542-548). Hence multi-source / rarest-first / 'up to 4 concurrent sources' never engages — effect (a) holds and is universal (every download is single-source).

(5) block_hashes assignments: 228 (init Pending), 336 (Have), 455-468 (reset is guarded by `if let BlockHashes::Pending` — only resets a still-Pending state to peer:None), 500 (Pending peer:Some). There is NO Have→Pending transition. `set_sources` (251-268) retains the surviving sources and never touches block_hashes. So once the solicited source delivers valid hashes (Have) and then departs, no new BlockHashRequest is ever issued, the remaining sources keep empty bitfields, and the download stalls permanently — effect (b) holds. The snub block-hash reset (455-468) cannot fire because it only runs while block_hashes is Pending, and snub's source-removal (430-451) requires non-empty in_flight, which a pre-chunk block-hash source lacks.

(6) Sources come from synced Ready peers via StartDownload (file.rs:601-626, re-emitted every snapshot per the comment at 608-613), confirming other present Ready peers exist yet remain unusable after the lone solicited source leaves. Since `keys().next()` iterates a HashMap (nondeterministic order), the solicited source can easily be a transient client even when the stable seeder is also Ready — so the stall fires even with the seeder present.

(7) Spec confirms intended-but-unimplemented behavior: network-design.md:449 ('Sent when a peer begins serving a file (complete bitfield)'), 450 ('Updated when a downloading peer completes new chunks' — also unimplemented), 461-467 (rarest-first counting across sources; 'those leechers can then serve each other'), 474 ('up to 4 concurrent sources'), 484-487 (endgame multi-source). None can occur with a single populated bitfield.

Both effects are real and actionable. Severity 'high' is appropriate: (a) is a universal deviation from the documented multi-source design and (b) is a hard, unrecoverable stall that can wedge a download while other Ready peers are present. The seeder-centric deployment mitigates the typical case but does not prevent the stall, since the nondeterministically-chosen single source may be a transient peer.

</details>

### 🟠 MEDIUM · plan_requests `assigned` set is not updated within the loop, so two sources can be assigned the same chunk in one bulk planning pass

**`dessplay/src/download.rs:551-625`** · _bug_

`assigned` (chunks already in flight) is computed once before the source loop (lines 551-555). Inside the loop a source's taken chunks are inserted into `src.in_flight` (lines 617-621) but `assigned` is never updated. In bulk (non-endgame) mode the candidate filter excludes `assigned.contains(&c)` using the stale snapshot, so when two sources with overlapping bitfields and empty in-flight are processed in the SAME `plan_requests` call, both pick the same window/rarest chunks and both are sent a ChunkRequest for them — duplicate fetches, exactly what the inline comment ('bulk mode avoids duplicating', line 550) says is prevented. The duplicates self-heal (is_written → bytes_duplicate + cancel_elsewhere) so it is wasted bandwidth, not corruption. This is currently masked by the single-source defect above and by the unit/integration tests delivering bitfields one at a time (so two full-bitfield empty-in-flight sources never coexist in one call); it triggers as soon as multiple sources advertise before block hashes arrive, or once the single-source defect is fixed.

- **Spec:** network-design.md 'Chunk Selection: Rarest First' / inline invariant at download.rs:550 ("bulk mode avoids duplicating")
- **Suggested fix:** Insert each taken chunk into `assigned` as it is committed to a source within the loop (or recompute the assigned set per source iteration), so later sources in the same pass don't re-pick it.

**Status (2026-06-27): fixed.** `plan_requests`' `assigned` set is now `mut` and each committed chunk is inserted into it (`assigned.extend(take...)`) as it is handed to a source, so a later source in the **same** bulk-mode pass can no longer be assigned a chunk already taken. Endgame still intentionally ignores `assigned` (multi-source tail). This was fixed together with the single-source stall above (one logical change). Regression test `two_sources_in_one_pass_are_not_assigned_the_same_chunk` (download.rs) reproduced the duplicate pre-fix; the `dessplay/tests/transfer.rs` two-seed test now asserts total waste is bounded by the endgame tail (≤ `pipeline_depth` chunks) — proving bulk mode does not duplicate — instead of the previous goodput-% threshold that had been calibrated against the (buggy) single-source path. As above, this is the **minimal** correctness fix only; full multi-source concurrency / rarest-first balancing / partial-serve remain out of scope.

<details><summary>Verification trail — code pointers</summary>

Confirmed against /home/svein/dev/dessplay/dessplay/src/download.rs. In plan_requests (lines 530-628): `assigned` is a non-mut local HashSet built ONCE at lines 551-555 from every source's in_flight, with the inline comment at 550 ("bulk mode avoids duplicating"). The source loop (568-626) is the only cross-source dedup, and it relies entirely on the stale `assigned`: line 592 filters candidates by `(endgame || !assigned.contains(&c))`; line 591's `!src_in_flight.contains(&c)` (cloned at 584) only dedups within one source. When a source takes chunks (613) it inserts them into `src.in_flight` (617-621) but never updates `assigned`. So in non-endgame (bulk) mode, two sources A and B both with empty in_flight and overlapping bitfields, processed in the same call: `assigned` stays empty for both; rarity/window ordering and `needed` are global, so both compute identical candidate lists (597-612) and take(slots) the same chunks → two ChunkRequests for the same chunks (622-625). This is the duplicate the comment claims is prevented. Reachability holds: set_sources (261-266) and FileAvailability (287-289) add sources with empty in_flight and later populate bitfields, while progress_and_refill returns early at line 509 ("no chunks until hashes are validated") whenever BlockHashes is Pending — so multiple bitfield'd, empty-in_flight sources accumulate, and the first plan_requests after hashes validate plans them together. Impact is wasted bandwidth, not corruption: a redundant chunk hits is_written → bytes_duplicate + cancel_elsewhere (362-364) and fresh writes also cancel_elsewhere (370), so duplicates self-heal once one copy arrives. Severity medium is defensible because the relay-through-NAS design (design.md: "serving a file costs one trip over the NAS uplink per recipient") makes duplicate fetches directly waste the scarce uplink; the only caveats are that it self-heals quickly and may be partially masked by a separate single-source defect, which would make this latent rather than always-active. Fix: make `assigned` mutable and insert each take into it within the loop.

</details>

### ⚪ LOW · Downloading unpause condition enforces only the 20% half, not the download-speed-vs-bitrate half

**`dessplay-core/src/derive.rs:122-134`** · _spec-mismatch_

The spec's File State / Downloading rule is: 'Unpausing is conditional: their download speed must be higher than the file's computed bitrate, *and* at least 20% of the file must be downloaded.' `file_block_reason` only checks `progress_bps < 2_000` (>= 20%); the bitrate condition is absent. The doc-comment acknowledges this was deferred 'with the transfer machinery in Phase 9' — but Phase 9 (download.rs/chunkstore.rs) has landed, so a client downloading at far below the file's bitrate can be treated as ready-to-play at 20% and stop blocking the group, causing playback to start and immediately stall on that client. No bitrate is computed from size_bytes/duration_millis anywhere in the gating path.

- **Spec:** design.md 'File State' → Downloading: "their download speed must be higher than the file's computed bitrate, and at least 20% of the file must be downloaded"
- **Suggested fix:** Have the downloading client publish (or the gate derive) a measured download rate vs the file's bitrate (size_bytes*8/duration), and require both rate>bitrate and progress>=20% before Downloading stops blocking — or update the spec to drop the bitrate half if intentionally abandoned.

<details><summary>Verification trail — code pointers</summary>

Code check (derive.rs:126-135): `file_block_reason` matches `FileAvailability::Downloading { progress_bps }` and returns a block only when `*progress_bps < 2_000`. `progress_bps` is documented in types.rs:269 as "Progress in basis points (0-10000)", so 2_000 == 20%. This is purely the ">= 20% downloaded" half of the rule; no download-speed term appears.

Spec check (design.md:236-238, "File State" -> Downloading): "Unpausing is conditional: their download speed must be higher than the file's computed bitrate, *and* at least 20% of the file must be downloaded." The spec requires BOTH conditions; the code enforces only the second. design.md presents the full rule as current behavior (it is not marked deferred there), so code and spec genuinely disagree.

Bitrate/speed absent from gating path: grepping both crates for bitrate/throughput/speed yielded only (a) the derive.rs:122-125 doc-comment that *acknowledges* the deferral, (b) types.rs:185 and types.rs:307-310 comments that call the bitrate rule "reserved for now," and (c) net/sim.rs link-throughput (unrelated). `FileAvailability::Downloading` carries only `progress_bps` (types.rs:268-271) — no speed field — and download.rs only tracks completion (`store.progress_bps()` at download.rs:513), never throughput. So no `size_bytes/duration_millis` bitrate is computed anywhere in gating, confirming the claim.

The claim's framing is correct: a present Maybe/Watching downloader stops blocking at 20% regardless of whether their speed exceeds the file's bitrate, defeating the protection the rule was written to provide. The doc-comment deferral is an acknowledgment of incompleteness, not a refutation; Phase 9 (download.rs/chunkstore.rs) has indeed landed, though note the speed-measurement infrastructure the rule needs was NOT part of what landed (download.rs measures progress, not throughput) — so closing this requires new local speed tracking, not just wiring.

Severity: I lower medium -> low. The impact is self-limited: a downloading peer advertises `Downloading`, not `Ready`, so it is excluded from leader election / drift-follow (derive.rs leader rules require `FileAvailability::Ready`). The group thus follows the Ready leader and is not frozen by the slow client; only the slow downloader itself plays its 20% and stalls. Combined with the explicit in-code deferral and the narrow trigger (a present watcher between 20% and 100% whose speed is below bitrate), the real-world blast radius is one degraded client rather than a group-wide failure. Medium is defensible as a fairness-guarantee regression, but low is the honest rating.

</details>

### ⚪ LOW · Downloading peers never serve their verified chunks; partial-serve path (ChunkStore::available/read_chunk) is dead code

**`dessplay/src/actors/file.rs:907-966`** · _spec-mismatch_

Serving (`enqueue_serve` line 909, `drain_serve_queue` line 933) only serves files present in `self.local_files`, which is populated solely with COMPLETE local copies (resolved media-root files or finished downloads). A peer mid-download never serves the blocks it has already verified, so the documented bittorrent-style propagation ('those leechers can then serve each other') and the 'Availability Tracking … Updated when a downloading peer completes new chunks' both never happen. `ChunkStore::available()` and `ChunkStore::read_chunk()` (chunkstore.rs:186-195, 249-259) — the machinery for partial serving — have no production callers (only tests). At the one-seeder scale this is acceptable per the doc's bottleneck argument, but it contradicts the stated availability/leecher-serving behavior.

- **Spec:** network-design.md 'Availability Tracking' ("Updated when a downloading peer completes new chunks") and 'Chunk Selection' ("those leechers can then serve each other")
- **Suggested fix:** Either wire serving to the in-progress ChunkStore (serve verified chunks, advertise its `available()` bitfield as it completes blocks), or amend the docs to state that only complete copies are served at v1 scale.

<details><summary>Verification trail — code pointers</summary>

Verified at /home/svein/dev/dessplay/dessplay/src/actors/file.rs and chunkstore.rs and download.rs. (1) Serve path serves only complete copies: enqueue_serve (file.rs:908-926) early-returns unless local_files.contains_key(file); drain_serve_queue (930-966) reads via local_files.get(file). local_files is populated solely with complete copies — reconciled cache_entries (479/493), Resolution::Verified (984), on_download_complete (791), manual mapping (1243). No partial ChunkStore file is ever inserted. (2) A downloading peer never serves: download.rs:298-301 returns vec![] for BlockHashRequest/ChunkRequest/Cancel ("Serve-side messages are handled by the file actor, not here"). The only production outgoing PeerMessage::FileAvailability is file.rs:867-875, which builds a COMPLETE bitfield (for i in 0..len { set(i) }) only when we hold the whole file; all partial-bitfield FileAvailability sends in download.rs are inside #[cfg(test)] mod tests (line 631+). During download only FileAvailability::Downloading { progress_bps } is emitted (file.rs:757) — a synced UI/gating value, not a peer chunk bitfield. (3) Dead code confirmed: grep across src/ shows ChunkStore::read_chunk has no callers (only the def, chunkstore.rs:186) and .available() only chunkstore.rs:335 inside the test module. Spec mismatch confirmed against docs/network-design.md:450 ("Updated when a downloading peer completes new chunks") and 465-467 ("those leechers can then serve each other") — neither is implemented. Severity low is correct: functionally acceptable at the one-seeder/relay-through-NAS scale the design assumes; it is doc-vs-code drift, actionable by either implementing partial serving or amending the spec.

</details>

### ⚪ LOW · A source that sends invalid block hashes is not re-asked promptly and the same peer may be re-picked

**`dessplay/src/download.rs:327-334`** · _bug_

When the asked source returns block hashes that fail `block_hashes_match`, `on_block_hashes` returns early leaving `block_hashes = Pending { peer: Some(bad), requested_at }` unchanged (lines 330-334). That peer has no chunks in flight, so the snub loop's `!s.in_flight.is_empty()` filter never drops it; only the `now - requested_at >= snub_timeout` staleness path (lines 455-468) clears the pending peer, a full ~30s later — and refill then re-picks `d.sources.keys().next()` (line 499), which can be the same bad peer again (HashMap order). So a single misbehaving/buggy source can delay acquiring block hashes by repeated 30s rounds. Relatedly, despite `DownloadAction::Abandon`'s doc ('no source could supply valid block hashes'), no code path ever emits Abandon for that case — such a download retries forever instead.

- **Spec:** network-design.md 'Scheduling' ("an invalid list is rejected and re-asked from another source")
- **Suggested fix:** On a block-hash mismatch, immediately reset to Pending{peer:None} and prefer a source other than the rejecting one (e.g. track tried peers), and consider emitting Abandon after exhausting all sources.

<details><summary>Verification trail — code pointers</summary>

All mechanical claims match the code in /home/svein/dev/dessplay/dessplay/src/download.rs (note: file is under dessplay/dessplay/src, not dessplay/src). (1) on_block_hashes (lines 330-334) on a block_hashes_match failure logs and `return vec![]`, leaving block_hashes == Pending{peer:Some(bad), requested_at} unchanged. (2) The snub source-drop loop (lines 430-451) filters on `!s.in_flight.is_empty() && ...`; a source that only returned block hashes has empty in_flight (the download early-returns at line 509 while Pending, so plan_requests/chunk assignment is never reached), so it is never dropped here. (3) The only thing that clears the pending peer is the staleness reset (lines 455-468): `!d.sources.contains_key(p) || now - requested_at >= timeout`. Since the bad peer remains a source, only the timeout (default snub_timeout 30s, network-design.md:478) fires, and while peer is Some no new request is issued (need_request at line 498 requires peer:None), so the whole download stalls ~30s. (4) sources is HashMap<PeerId,Source> (line 161); after reset, refill picks d.sources.keys().next() (line 499) with no exclusion of the peer that just gave bad hashes, so it can re-pick the same bad peer (deterministically, for stable map membership). (5) DownloadAction::Abandon's doc (lines 125-126) cites the no-valid-block-hashes case, but grep shows the only emission site is line 216 (ChunkStore::open failure); file.rs:767 only consumes it. No Abandon is ever emitted for unsatisfiable block hashes, so such a download never gives up. This deviates from network-design.md:488-490 ("an invalid list is rejected and re-asked from another source"). The existing test invalid_block_hashes_are_rejected_and_re_asked (line 733) does not actually exercise prompt re-ask: it injects the good source's hashes unsolicited and only asserts no chunk requests flow. Severity low is correct: the trigger (a peer serving block hashes that don't match the root) is rare under the 5-trusted-friends threat model, and the common multi-source case can self-heal after one ~30s round; only the worst case (HashMap order keeps the bad peer first) stalls indefinitely.

</details>

---

## Client — Session layer: narrator & gating glue

**Files:** `dessplay/src/session.rs` (very large — the session orchestrator: chat narrator, playback gating glue, EOF, drift correction, mappings).
**Read first:** design.md → *System Messages* (the narrator's "every client diffs the same synced inputs → every client narrates the same lines" and "No cascade spam" invariants), *Playback Rules*, the `holds_now_playing` gate in *Player Integration*.
**Key entry points:** `narrate()` (diffs state-view + peer-list), the `PlayerOutput` match arms (`Eof` / `UserSeeked` / `PositionTick` — note only `Eof` is missing the `holds_now_playing` guard), `watcher_prefs_written` (the half-implemented List-auto-write suppression).
**Theme:** one **Medium** bug (placeholder EOF can advance the whole group) plus narrator spec-drift where derivation isn't a pure function of *synced* state (per-client local suppression makes different clients narrate different lines).

### 🟠 MEDIUM · EOF report to the server is not gated on holds_now_playing (placeholder/stale-frame EOF can advance the whole group)

**`dessplay/src/session.rs:1496`** · _bug_

The `PlayerOutput::Eof { file } => vec![Directive::ReportEof(file)]` arm reports end-of-file unconditionally, unlike the two sibling arms that can also speak for the group: `UserSeeked` (line 1426) and `PositionTick` (line 1459) both first check `!self.holds_now_playing(view)` (and PositionTick also `view.now_playing != Some(file)`) and return `vec![]` for a placeholder / stale frame, with explicit comments that a placeholder's position must not be published or mark the file watched. The same reasoning applies to EOF, but the guard is missing. A not-watching/downloading client loads the placeholder PNG via `FileOutput::PlaceholderReady` reusing the now-playing hash as the file id (line 1990-1991), so its `PlayerEvent::Eof` carries `file == now_playing`. mpv's default `image-display-duration` (~1s) makes a *playing* image reach `eof-reached` (player/mpv.rs: eof-reached -> PlayerEvent::Eof), which the player echoes as `Eof{now_playing}`. The session then sends `EofReached{now_playing}`; the server (dessplay-rendezvous/src/server.rs:973-979) accepts EOF from any present Ready/Maybe/Paused reporter, so it advances now-playing, sets the watched flag, advances The List, and resets to the next entry -- for the entire group -- even though nobody watched the episode. Reachability: placeholders are normally held paused (so the image never reaches its display-duration end), but when `self.loaded` is still None (no real now-playing video has loaded this session) the SetPlaying re-assert at line 1243 is skipped (`if self.loaded.is_some()`), so a user who manually unpauses the placeholder in the mpv window is never re-paused, the image plays out, and the spurious advance fires. Less-common but concrete shared-state corruption.

- **Spec:** design.md Playback Rules / Player Integration: 'a client that does not hold the real now-playing video never takes seek authority or publishes a position from its placeholder in the first place -- see the holds_now_playing gate'. EOF reports should follow the same gate.
- **Suggested fix:** Gate the Eof arm with the same condition the PositionTick arm uses: `if view.now_playing != Some(file) || !self.holds_now_playing(view) { vec![] } else { vec![Directive::ReportEof(file)] }`. This also harmlessly suppresses duplicate/stale EOF reports for an already-advanced file (the server ignores them anyway).

**Status (2026-06-27): fixed.** The `Eof` arm now reports only when the client holds the real now-playing video, mirroring the `PositionTick` / `UserSeeked` gates — a placeholder/stale EOF (`loaded != now_playing`) yields `vec![]` and no longer advances the group. The shared "current now-playing file AND we hold the real video" precondition was extracted into a documented `speaks_for_now_playing(view, file)` helper, now used by both file-tagged group-speaking arms (`Eof` and `PositionTick`), so a future file-bearing arm can't forget the gate (`UserSeeked` carries no file and keeps the bare `holds_now_playing`). Regression test added (`dessplay/src/session.rs`): `eof_from_a_placeholder_is_not_reported_to_the_server` (`loaded == None`, EOF for now-playing → no `ReportEof`; emitted one pre-fix → failed), and the existing `eof_and_fatal_crash_map_to_their_directives` was tightened to hold the real now-playing video so it keeps covering the real-EOF path.

<details><summary>Verification trail — code pointers</summary>

Verified the full chain in code. session.rs:1496 `PlayerOutput::Eof { file } => vec![Directive::ReportEof(file)]` is unguarded, unlike its two sibling "speak for the group" arms: UserSeeked (1426) does `if !self.holds_now_playing(view) { vec![] }` and PositionTick (1459) does `if view.now_playing != Some(file) || !self.holds_now_playing(view) { vec![] }`, both with comments that a placeholder must not publish position or mark a file watched. holds_now_playing (1007-1009: `self.loaded.is_some() && self.loaded == view.now_playing`) is documented (1001-1006) as the single gate, and the placeholder Load (FileOutput::PlaceholderReady, 1983-1997) deliberately leaves `loaded` untouched while reusing the now-playing hash as the file id. The player actor (actors/player.rs:318-322) sets `self.current = (file, path, title)` and resets `eof_reported=false` on every Load, and the Eof arm (545-551) emits `Eof { file: self.current.0 }` — so a placeholder's EOF carries file == now_playing. The SetPlaying re-assert that would re-pause an unpaused player is skipped for a placeholder because line 1243 gates on `if self.loaded.is_some()` (None when the client has only ever held a placeholder), and following=None (1256, !showing_now_playing) means no SyncTo re-pauses it either; mpv opens the placeholder paused (mpv.rs:216), so only a manual unpause starts it, after which the keep-open image plays out to eof-reached (mpv.rs:473-478). Server side (dessplay-rendezvous/src/server.rs:970-993) handle_eof rejects mismatched now_playing and rejects NotWatching/Away reporters but accepts Ready/Maybe/Paused, then sets watched, advances now-playing, forces intent Paused, resets seek authority, and advances The List for the whole group. Correction to the claim's framing: a strictly NotWatching placeholder reporter is filtered out by server line 974, so the leading 'not-watching' example does not fire by itself — but derive::user_state (dessplay-core/src/derive.rs:100-115) ignores FileAvailability, so a *downloading* client with the default Maybe preference derives to Maybe and IS accepted, making the bug real for the common 'next episode still downloading, placeholder on screen' case. The spec (design.md Playback Rules item 5 / Player Integration: the placeholder 'never takes seek authority or publishes a position from its placeholder in the first place — see the holds_now_playing gate') clearly intends this gate to cover any group-speaking action; the EOF report is one and the guard is genuinely missing. Reachability requires a deliberate manual unpause of a placeholder plus a ~1s image play-out, not an automatic trigger, so medium severity is appropriate.

</details>

### ⚪ LOW · Ctrl-R / /ready double-narrates ('X unpaused' + 'X set S to maybe') for a single action

**`dessplay/src/session.rs:535-615`** · _spec-mismatch_

The narrator processes manual-override changes (lines 535-565) and now-playing series-preference changes (lines 567-615) in independent blocks and emits a separate line for each. The 'become ready' action (Ctrl-R and /ready) in src/ui/app.rs `become_ready` (lines 708-733) writes BOTH a manual-override clear (state: None) AND a NotWatching->Maybe series flip in one user action, when the user currently has a manual override set AND the now-playing series is NotWatching for them. In that combined state, one Ctrl-R produces two narrator lines: 'X unpaused' and 'X set S to maybe'. This contradicts the documented 'one line per action' rule. The combination is reachable (e.g. a manual pause plus an auto-NotWatching from a missing unknown-series file, then Ctrl-R clears both).

- **Spec:** System Messages, No cascade spam: 'A single user-meaningful action often writes several registers at once ... The narrator emits one line per action, not one per register.'
- **Suggested fix:** Coalesce the override-clear and the same-user NotWatching->Maybe flip when they appear together in one diff (e.g. prefer the readiness line and suppress the redundant 'set to maybe', or emit a single combined 'X is ready' line), so a single become_ready action narrates one line.

<details><summary>Verification trail — code pointers</summary>

Independently confirmed against code and spec. become_ready (dessplay/src/ui/app.rs:708-733) writes SetManualOverride{None} (712-715) AND, when the now-playing series is NotWatching for the user (guard 720-724), SetSeriesPreference{Maybe} (726-730) in one UserAction batch. The narrator processes these in two independent, non-coalescing blocks: manual-override diff (session.rs:535-565) pushes "{user} unpaused" (line 560) when clearing a Paused override (or "{user} is back" line 561 for Away); now-playing series-preference diff (session.rs:567-615) pushes "{user} set {name} to maybe" (609-611) on a NotWatching->Maybe change. The only skip guard at line 597 requires was.is_none(), but here was=Some(NotWatching), so it does not fire. Both mutations land in the same synced diff, so one narrate() pass (489-654, no coalescing — only NARRATOR_BURST_CAP at 647 suppresses wholesale-replacement bursts, irrelevant to a 2-line diff) returns TWO Directive::SystemLine lines for a single Ctrl-R/ /ready. The combined precondition is genuinely reachable: the auto-NotWatching write at session.rs:899-904 (missing unknown-series file with an AniDB series id) sets the series pref to NotWatching, while a manual Paused override is independently set (ready-on-startup off, or a manual pause). docs/design.md "System Messages -> No cascade spam" states the narrator "emits one line per action, not one per register"; its two illustrating examples (play, EOF) yield one line only because the extra registers (intent, watched) are never independently narrated, whereas become_ready touches two separately-narrated registers, so the documented invariant is violated. Real and actionable; cosmetic chat-log double-line, so low severity is correct.

</details>

### ⚪ LOW · List-derived watch-preference auto-writes for the now-playing series are narrated inconsistently across clients

**`dessplay/src/session.rs:567-615`** · _spec-mismatch_

The watch-preference block narrates any (user, now-playing-series) preference change, with the ONLY suppression being for the local client's own List auto-write: `if user == &self.me && was.is_none() && self.watcher_prefs_written.contains(&series)` (line 597). `watcher_prefs_written` is purely local per-client state (line 204), and each client writes only its OWN preference from a linked List entry (line 1204-1208, `user: self.me`). Consequence: when a List entry whose series is the now-playing series is linked, each watcher's/non-watcher's client independently writes its own None->Watching / None->NotWatching preference and syncs it. On every OTHER client this arrives as a None->value diff for a remote user and is narrated as a deliberate action — e.g. 'Nero is committed to S (by Nero)' / 'Kim set to not-watching S (by Kim)' — even though no one ran /watch or /skip; the List link did. Worse, because each client suppresses only ITS OWN auto-write, every client narrates a DIFFERENT set of lines from the SAME synced inputs, directly contradicting the spec's core invariant ('Because every client diffs the same synced inputs, every client narrates the same lines') and the 'No cascade spam' intent that the watcher_prefs_written suppression was meant to realize. The narrator cannot distinguish a remote List auto-write from a remote manual /watch (no setter is recorded), so the suppression is fundamentally only half-implemented.

- **Spec:** System Messages: 'Because every client diffs the same synced inputs, every client narrates the same lines -- consistent without any extra wire traffic.' and 'Watch-preference lines are scoped to the now-playing series ... which keeps the List's bulk auto-writes for other series out of the chat'
- **Suggested fix:** Either narrate List-derived watch-preference changes for the now-playing series on no client (treat any None->value transition matching a known-linked watchers set as a List auto-write and suppress it uniformly, derived from synced list_entries rather than local watcher_prefs_written), or accept that bulk List writes for the now-playing series are noise and skip narrating preference changes whose new value exactly matches the linked List entry's implied preference. The decision must be a function of synced state only, never per-client local state.

<details><summary>Verification trail — code pointers</summary>

Mechanism verified in code. session.rs:597 suppresses watch-preference narration only when `user == &self.me` (local writer); watcher_prefs_written (declared session.rs:204) is purely local per-client state; the List-watchers loop (session.rs:1198-1208) has every client independently write and sync ONLY its own preference (`user: self.me.clone()`, Watching if `entry.watchers.contains(&self.me)` else NotWatching). Per design.md:576-588 the StateView records no writer, so the narrator cannot tell a remote List auto-write from a remote manual /watch. Therefore, when a List entry linked to the now-playing file's series is populated with ≥2 watchers lacking prior prefs, each client's own None->value write syncs to all peers; on every OTHER client it appears as a None->value diff for a remote user (user != self.me at line 590) and is narrated ("Nero is committed to S (by Nero)"), while each client suppresses only its own write — so each client narrates a different set of lines. This directly violates design.md:540-541 ('every client narrates the same lines -- consistent without any extra wire traffic') and the 'No cascade spam' intent (design.md:590-593, one line per action). The code comment at session.rs:569-571 shows suppression of List auto-writes for the now-playing series WAS intended but is only half-implemented (local writer only). The existing regression test narrator_reports_manual_change_after_list_auto_write (line 2409) only exercises the local writer, so it does not cover the remote-write case. Severity corrected to low because the effect is purely cosmetic, local-only, ephemeral system chat lines with a narrow trigger (linking a List entry for the currently-playing series with multiple present watchers); no state divergence, convergence, or playback impact.

</details>

---

## Client — Sync & network actors

**Files:** `dessplay/src/actors/sync.rs`, `dessplay/src/actors/network.rs`.
**Read first:** architecture.md → *SyncActor* / *NetworkActor* / message flow, sync-state.md → reconnect/epoch handling.
**Theme:** one bug — the divergence-recovery path uses a blocking event send, reintroducing the consumer-stall wedge that the `changed()`-guarded design avoids.

### ⚪ LOW · Divergence path uses a blocking event send, reintroducing the consumer-stall wedge changed() guards against

**`dessplay/src/actors/sync.rs:748`** · _bug_

`self.events.send(SyncEvent::Diverged).await` is a blocking send on the same SyncEvent channel that `changed()` deliberately uses `try_send` for (line 443), with the explicit comment (438-442) that 'a stalled (or absent) consumer must never block the sync actor. A blocking send here deadlocked the whole client once >256 ops arrived unpolled.' Because StateChanged is try_send (dropped when full), the 256-deep channel can sit full of undrained StateChanged events; if the SyncEvent consumer (client.rs forwarder -> bridge loop) is stalled at the moment two consecutive hash mismatches fire, this await blocks the sync task indefinitely, so it stops answering GetView/GetEpoch and applying ops — the exact failure the try_send convention was added to prevent. The hazard requires a stalled consumer (itself a bug) coinciding with divergence, hence low severity, but it is a clear, actionable inconsistency with a stated invariant.

- **Suggested fix:** Use `let _ = self.events.try_send(SyncEvent::Diverged);` to match changed()'s non-blocking contract; the RequestMerge has already been queued to the network actor regardless of whether the UI observes the Diverged signal.

<details><summary>Verification trail — code pointers</summary>

Verified against /home/svein/dev/dessplay/dessplay/src/actors/sync.rs. Line 748 does `let _ = self.events.send(SyncEvent::Diverged).await;` — a blocking send on the exact same `self.events` channel that the `changed()` helper (line 443) deliberately uses `try_send` for, with the comment at 438-442: 'a stalled (or absent) consumer must never block the sync actor. A blocking send here deadlocked the whole client once >256 ops arrived unpolled.' Both senders target `sync_event_tx`, a bounded mpsc of capacity 256 (client.rs:78), handed to the sync actor at client.rs:112; StateChanged and Diverged share it. The actor's main loop (sync.rs:360-373) is a single `tokio::select!` that awaits `actor.handle(cmd).await` to completion, so a block inside line 748 stalls the whole actor — it stops servicing GetView/GetEpoch (lines 480-483), Mutate, and Server ops, exactly the wedge the try_send convention prevents. The stalled-consumer precondition is explicitly a supported design condition, not merely a coincident bug: test `slow_ui_consumer_never_wedges_the_sync_actor` (line 891) holds the events receiver undrained and floods 600 ops to prove the StateChanged path is non-blocking, but there is NO analogous protection or test for the Diverged path (divergence_alarm_after_two_mismatches at line 1012 actively drains rig.events, so it never tests a full channel). The forwarder chain (client.rs:157-163 → 256-cap event channel → owner) means a stalled owner backs up both 256-deep channels; a Diverged fired then blocks the sync task indefinitely. No comment or guard justifies the deviation. Low severity is appropriate — the trigger requires ~512 backed-up events plus two consecutive hash mismatches coinciding with a stalled consumer.

</details>

---

## Client — IRC bridge

**Files:** `dessplay/src/actors/irc.rs` (newest subsystem — added in commit `f618914`).
**Read first:** design.md → *IRC Bridge* (identity/`Dess` suffix, outbound = own Chat only, inbound non-`*Dess` local-only, reconnect/backoff, live reconfigure), architecture.md → *IrcActor*.
**Key entry points:** `run()` (the outer reconnect state machine), `run_session()` (one connection), `is_bridge_nick`.
**Theme:** a panic on non-ASCII inbound nicks (`is_bridge_nick` slices at a non-char-boundary), a capped-backoff bypass when a chat send arrives mid-wait, and no actor-loop tests for the reconnect machine (high-risk per CLAUDE.md).

### 🟠 MEDIUM · The reconnect loop run() (backoff pacing, live Reconfigure, disable->QUIT->idle) has no actor-loop tests

**`dessplay/src/actors/irc.rs:126-192`** · _test-gap_

Tests cover the pure helpers and run_session (single connection), but the outer run() reconnection state machine — capped backoff growth/reset across attempts, the disabled-idle loop that drops SendChat, Reconfigure causing QUIT+reconnect or QUIT+idle, and the registered-vs-unregistered Disconnected gating — is entirely untested. Reconnection is a CLAUDE.md-designated high-risk area, and the backoff-bypass defect above lives precisely in this untested code: an actor-loop test driving run() over a duplex pipe (or a deterministic connect injector with paused tokio time) would have caught it and would guard the disable->QUIT->idle transition the spec requires.

- **Spec:** CLAUDE.md Testing: 'High-risk areas get extra coverage ... reconnection/epoch handling'; architecture.md IrcActor: reconnect-with-backoff and 'reconfigured live ... disabling it makes it QUIT and idle'.
- **Suggested fix:** Add tests over the run() loop using a stream/connect injector and paused tokio time: assert backoff growth across failed connects, prompt reset after a registered drop, QUIT-then-idle on Reconfigure(disabled), QUIT-then-reconnect on Reconfigure, and that a SendChat during the wait does not abort the backoff.

**Status (2026-06-28): fixed (tests added).** Fixed together with the entangled backoff-bypass LOW below (the loop and its bug are one change). The reconnect loop now has a testable seam: `run()` is a thin wrapper over a generic `run_with_connector(config, commands, events, connect)` that injects how a connection is established (production = TCP/TLS `connect`; tests = an in-memory `tokio::io::duplex` pipe per attempt), so the outer state machine is driven end to end with paused tokio time. Eight new `#[tokio::test(start_paused = true)]` tests over `run_with_connector` (`dessplay/src/actors/irc.rs`): `backoff_grows_and_caps_across_failed_connects` (asserts the inter-attempt gaps are exactly `[2,4,8,16,32,60,60]`s — doubling then capped at MAX_BACKOFF), `registered_drop_emits_disconnected_and_resets_backoff` (a registered drop emits `Disconnected` and resets the grown backoff so the next reconnect is prompt), `unregistered_drop_does_not_emit_disconnected` (a pre-001 drop stays silent), `disabled_config_idles_until_reconfigured` (disabled = no socket; SendChat dropped; `Reconfigure` re-enables), `reconfigure_while_connected_quits_and_reconnects` (QUIT + reconnect joining the new channel) and `reconfigure_to_disabled_quits_and_idles` (QUIT + idle), `run_answers_ping_after_connecting` (PONG through the full connect path), plus the backoff-bypass regression `send_chat_during_backoff_does_not_abort_the_wait` (see the LOW below). A scripted connect injector (`injector`) hands out duplex pipes or failures and signals each attempt, so the timing tests read attempt times straight off paused-time auto-advance — no sleeps. **Mutation sanity-check** (these tests assert already-correct behavior, so confirmed they bite): temporarily neutering `grow_backoff` to a no-op made `backoff_grows_and_caps_across_failed_connects` FAIL with `left: [2,2,2,2,2,2,2]` vs `right: [2,4,8,16,32,60,60]`; reverted.

<details><summary>Verification trail — code pointers</summary>

Confirmed against code and spec. `run()` (irc.rs:126-192) is the outer reconnection state machine; a repo-wide grep shows it is called only from production code (dessplay/src/run.rs:698), never from any test. The sole actor-loop test helper `spawn_session()` (irc.rs:774-792) spawns `run_session` (the single-connection inner loop, irc.rs:257), and all seven actor-loop tests (registers_joins_and_emits_connected, forwards_external_messages_and_drops_bridges, answers_ping, sends_chat_as_privmsg, retries_nick_on_collision, shutdown_quits_and_exits) drive `run_session`, not `run`. `backoff_grows_and_caps` (irc.rs:746) tests only the pure `grow_backoff` helper, not its use inside `run()`'s select loop. Therefore the following are entirely uncovered: the disabled-idle loop dropping SendChat (lines 137-147); backoff pacing across attempts via the tokio::select! sleep-vs-command race and grow_backoff/reset interplay (lines 177-190); Reconfigure causing QUIT+reconnect or QUIT+idle (lines 140-143, 154-158, 181-185); and the registered-vs-unregistered Disconnected gating with prompt-retry reset (lines 160-170). The module doc (lines 28-29) and architecture.md:384-388 claim the loop is duplex-pipe driven in tests and reconnects with capped backoff / QUITs-and-idles on disable, but only the inner loop is tested. CLAUDE.md Testing explicitly designates reconnection as a high-risk area warranting extra coverage. The gap is real and readily testable (drive run() over a duplex pipe with a connect injector and paused tokio time). Medium severity stands.

</details>

### ⚪ LOW · A SendChat during the reconnect-backoff wait aborts the wait and triggers an immediate reconnect, defeating capped backoff

**`dessplay/src/actors/irc.rs:176-190`** · _bug_

The reconnect-wait select races sleep(backoff) against commands.recv(). The SendChat arm matches `{}` (the message is dropped because there is no connection), the select block ends, `backoff = grow_backoff(backoff)` runs, and the loop immediately retries connect() — the remaining backoff sleep is skipped. During a server outage the local user is typically still chatting (the common watch-party case), so every chat line wakes the wait and forces an immediate reconnect attempt. Against a host that refuses the connection quickly (server process down), this becomes a reconnect attempt per chat line regardless of the grown backoff value, undermining the intended capped exponential backoff. Shutdown/Reconfigure correctly need to interrupt the wait, but a dropped SendChat should not.

- **Spec:** architecture.md IrcActor: 'reconnects with capped exponential backoff'; design.md IRC Bridge Lifecycle: 'reconnects with capped backoff'.
- **Suggested fix:** Don't end the backoff wait for a dropped SendChat. Loop the select so only Shutdown/Reconfigure (and timer expiry) break out — e.g. wrap the select in a loop and `continue` the inner wait on SendChat without growing/resetting backoff, or drain SendChat via a separate non-waking path.

**Status (2026-06-28): fixed.** Fixed together with the MEDIUM test-gap above (same code region — the bug lived in the untested reconnect loop). The inline `tokio::select!` reconnect-wait was extracted into a `wait_backoff(backoff, commands)` helper returning a `WaitOutcome` (`Elapsed` / `Shutdown` / `Reconfigure`). The sleep future is now created once and polled across iterations via `tokio::pin!` + `&mut sleep`, with the select wrapped in a `loop`: only `Shutdown` and `Reconfigure` break the wait; a `SendChat` is dropped (the bridge is lossy while down, as designed — it was already dropped before, the bug was that it *also* ended the wait) and the loop continues on the *same* sleep, so the dropped chat neither shortens nor extends the remaining backoff. A deliberate `Reconfigure` still takes effect promptly (it returns `WaitOutcome::Reconfigure`, reconnecting immediately) — that distinction is intentional: capped backoff holds for transient disconnects, a settings change is the exception. Regression test `send_chat_during_backoff_does_not_abort_the_wait` (`dessplay/src/actors/irc.rs`, paused-time, over `run_with_connector` with an always-failing connector): after the first failed connect it injects a `SendChat` during the wait and asserts no reconnect fires before the backoff elapses, then that the reconnect does fire afterward. Confirmed FAILING before the fix — temporarily restoring the bug (SendChat → end the wait) panicked with `SendChat aborted the backoff wait and forced an immediate reconnect` — and passing after.

<details><summary>Verification trail — code pointers</summary>

Control flow is exactly as claimed. In irc.rs run() the reconnect-wait is `tokio::select!` over `sleep(backoff)` vs `commands.recv()` (lines 177-189). The `SendChat(_) => {}` arm (line 187) drops the message and lets the select block end; execution then falls to `backoff = grow_backoff(backoff)` (line 190) and the `loop` re-runs `connect(&config).await` (line 150) immediately — the remaining sleep is genuinely skipped, while Shutdown returns and Reconfigure `continue`s (legitimately interrupting). SendChat is produced by a lossy `try_send` tapped on the local user's own `Mutation::Chat` (run.rs:861-863), so an actively-chatting user does wake the wait per line. INITIAL_BACKOFF=2s, MAX_BACKOFF=60s, grow_backoff doubles+caps (irc.rs:42,44,370-371): the backoff *value* still grows and caps, but because the sleep future is dropped on each SendChat, the actual inter-attempt delay collapses to roughly the chat arrival rate against a quickly-refusing host — a real deviation from the spec's intent (architecture.md:384 "reconnects with capped exponential backoff"; design.md IRC Bridge Lifecycle "reconnects with capped backoff"). No design note licenses SendChat to interrupt the wait. However the severity is overstated: it only manifests during an IRC outage where connect() returns fast (port refused; an unreachable host instead blocks on the OS connect timeout, so no storm), the magnitude is bounded by human watch-party chat rate (a handful of messages/min), and the cost is just a few extra cheap TCP connect attempts — no correctness, crash, data, or security impact, and the dropped chat line is consistent with the documented lossy design. Real and actionable (e.g. re-enter the wait on SendChat without retrying connect), but low, not medium.

</details>

### ⚪ LOW · is_bridge_nick panics on a non-ASCII inbound nick at a non-char boundary

**`dessplay/src/actors/irc.rs:474-477`** · _bug_

is_bridge_nick slices `nick[n - BRIDGE_SUFFIX.len()..]` where n is the byte length. BRIDGE_SUFFIX is 4 bytes, so it slices at byte n-4. If the inbound nick contains a multi-byte UTF-8 char such that byte n-4 is not a char boundary (e.g. nick "\u{1F600}x", a 4-byte emoji followed by 'x', len 5, n-4=1 lands inside the emoji), the slice panics. This is reachable from untrusted input: privmsg_event (line 543) calls is_bridge_nick(&from) on the nick parsed from any inbound PRIVMSG prefix, and BufReader::lines() yields valid-UTF-8 String lines, so a multi-byte nick passes parsing and reaches the slice. The panic unwinds the spawned IRC task; tokio swallows it, the events sender drops, and run.rs sets irc_alive=false (line 1058), permanently killing the bridge for the rest of the session. A single crafted PRIVMSG from any IRC user in the channel takes the bridge down. Confirmed by reproduction: slicing "\u{1F600}x" at byte 1 panics with 'byte index 1 is not a char boundary'.

- **Spec:** design.md IRC Bridge: 'Messages from IRC nicks that do not end in Dess are shown locally' — the *Dess filter must tolerate arbitrary IRC nicks without crashing.
- **Suggested fix:** Compare on bytes instead of slicing the str: e.g. `nick.as_bytes()` ends_with check (`let s = BRIDGE_SUFFIX.as_bytes(); nb.len() >= s.len() && nb[nb.len()-s.len()..].eq_ignore_ascii_case(s)`), or use `nick.get(n-4..)` and handle None.

<details><summary>Verification trail — code pointers</summary>

Verified the code defect and the full reachability/consequence chain in /home/svein/dev/dessplay/dessplay/src/actors/irc.rs.

(1) is_bridge_nick (lines 474-477): `let n = nick.len(); n >= BRIDGE_SUFFIX.len() && nick[n - BRIDGE_SUFFIX.len()..].eq_ignore_ascii_case(BRIDGE_SUFFIX)`. BRIDGE_SUFFIX = "Dess" (line 50), 4 bytes. The slice nick[n-4..] is a str byte slice; if n-4 is not a char boundary it panics. I reproduced it: is_bridge_nick("\u{1F600}x") (len 5, slice index 1 inside the 4-byte emoji) panics with "start byte index 1 is not a char boundary".

(2) Reachability: privmsg_event (line 543) calls is_bridge_nick(&from) on from = nick_of_prefix(parsed.prefix...).to_string(). nick_of_prefix (434-440) only splits at ASCII '!'/'@' and never restricts to ASCII, so a multi-byte nick survives. Lines come from BufReader::new(read).lines() / next_line() (265, 278) yielding valid-UTF-8 Strings, and parse_line preserves the prefix, so a multi-byte nick reaches the slice.

(3) Consequence: run_session is .await-ed directly inside the spawned run task (irc.rs:152), so the panic unwinds the whole run task and drops the events Sender. In run.rs the self.irc_events.recv() arm (line 1050) then yields None -> `None => self.irc_alive = false` (line 1058), disabling the arm for the rest of the session; the reconnect loop is inside the dead task, so the bridge stays down until restart. The app survives (isolated tokio task panic).

Severity downgraded to low: the claim's "single crafted PRIVMSG from any IRC user" overstates reachability. On the default Rizon ircd, nicks are validated to ASCII at registration and the server stamps the PRIVMSG prefix, so an ordinary channel user cannot present a multi-byte nick; the realistic trigger is a malicious/non-compliant or UTF-8-permitting IRC server (server is user-configurable). Blast radius is bounded to the optional, non-critical IRC bridge dying for the session (recoverable on restart, no data loss, no security impact). Spec docs/design.md (IRC Bridge) say *Dess-suffixed nicks are dropped and the actor must tolerate arbitrary IRC nicks, supporting that the unchecked byte slice is a genuine defect; the trivial fix is to slice nick.as_bytes() with <[u8]>::eq_ignore_ascii_case or guard with is_char_boundary.

</details>

---

## Client — TUI (ui/)

**Files:** `dessplay/src/ui/{app,components,props,modals,commands,theme}.rs`.
**Read first:** ui-architecture.md (tui-realm Elm model, ViewSpec, keybinding declarations), design.md → *TUI Layout*, *Keyboard Shortcuts*, *The List UI*, settings screen.
**Theme:** mostly spec-drift between the documented keybindings/behaviours and the implementation: the List edit modal can't reach `next_ep`/`available` (so "maintained by hand" is impossible) and saves only on Ctrl-S; settings media-root reorder uses `J/K` not the documented `ctrl-j/ctrl-k`; All-Series sort isn't persisted; Users-pane `a` ignores Away attribution; and an `overlay()` arithmetic overflow on very wide terminals.

### 🟠 MEDIUM · List edit modal cannot edit next_ep or the 'available' (✓/✖) marker

**`dessplay/src/ui/modals.rs:937-1003`** · _spec-mismatch_

ListEditModal's LIST_FIELDS (lines 937-946) only exposes Name, Nero's name, Genre, Notes, Recommender, Status, Status note, Source. There is no field for `next_ep` or `available` (NextEpState), and `commit` (984-1003) never touches them. A repo-wide grep confirms no other UI path writes NextEpState (only `props.rs` reads it and a test). The ListRow props (props.rs:500-502, 810-816) and the SeriesPane render (components.rs:1218-1222) DISPLAY next_ep and the ✓ marker, but nothing can set them. Consequence: for any List entry that is unlinked, or linked but non-numeric, the documented 'maintained by hand' path is absent — a user can never toggle 'this week's episode is out' for a CurrentSeason show, defeating the weekly-airing tracking the design emphasizes.

- **Spec:** ui-architecture.md Modals: "ListEntryEdit: Edit a List entry's fields (status, notes, next_ep, ...)". design.md The List: "available is maintained by hand" / "next_ep is free text by necessity".
- **Suggested fix:** Add editable Next-ep and Available rows to the modal (a free-text field and a toggle), committing through a NextEpState mutation, since NextEpState is a separate CRDT register from SeriesListEntry.

**Status (2026-06-28): fixed.** `ListEditModal` gained two rows: **Next ep** (free-text, edited via the existing `FieldEditor`) and **Available** (a boolean toggled in place by Enter, mirroring the Status cycle row — no new field-kind framework needed). The modal now loads the entry's current `NextEpState` on open (`ListEditModal::new(id, entry, next_ep)`, pre-populated from `view.list_next_ep`) so both rows show current values. On save it reports the edited register only when `next_ep`/`available` actually changed (`Msg::ListEntrySaved` now carries `Option<Box<NextEpState>>`); `app.rs::save_list_entry` then emits a `Mutation::SetNextEp` alongside the existing `PutListEntry` (routed through `handle()` as a two-action save). The conditional write preserves the register's separation rationale — a note-only edit never clobbers a concurrent server EOF auto-advance, and vice versa. `NextEpState` gained a `Default` derive. Regression tests in `dessplay/tests/ui_app.rs`: `list_edit_modal_edits_next_ep_and_available` (open shows current values; edit + toggle + save emits `SetNextEp{next_ep:"12", available:true}` plus `PutListEntry` — failed before the change, the modal had no Next-ep row) and `list_edit_modal_save_without_next_ep_change_emits_no_set_next_ep` (an unrelated field edit emits no `SetNextEp`). The separate ⚪ LOW "saves only on Ctrl-S" finding is untouched.

<details><summary>Verification trail — code pointers</summary>

Code check (modals.rs:937-1003): LIST_FIELDS (937-946) enumerates exactly 8 labels — Name, Nero's name, Genre, Notes, Recommender, Status, Status note, Source. field_value() (970-982) and commit() (984-1003) match only indices 0-7; there is no case for next_ep or available, and no index for NextEpState anywhere in the modal. cycle_status() (1005-1017) only walks ListStatus. The List-mode keybindings (components.rs:1166-1173) are m/Enter/e/l — `e` opens this same ListEditModal, and there is no other key.

Writer audit: grep for set_next_ep/SetNextEp/NextEpState across the whole repo shows NextEpState is written in only three non-test places: (1) import.rs:461 emits Mutation::SetNextEp during the one-shot CSV `import-list` subcommand; (2) dessplay-rendezvous/src/server.rs:1000 auto-advances numeric next_ep on EOF for AniDB-linked entries and resets available→false (server.rs:1041-1043); (3) compact.rs:88 preserves existing values across compaction. No interactive UI path writes NextEpState. So a user genuinely cannot set next_ep (for unlinked or non-numeric-linked entries) nor toggle `available` to true at all in-app — once the server resets available to false there is no hand path to set it back.

Spec check: ui-architecture.md:305 — "ListEntryEdit: Edit a List entry's fields (status, notes, next_ep, ...)" explicitly lists next_ep as editable in this modal. design.md:761 — "Otherwise `available` is maintained by hand" (the ✓/✖ column, design.md:752), and design.md:778-779 — "Editing fields and adding entries happens in a small edit modal". No future-work/Phase caveat defers next_ep editing (the only future-work note nearby is about *automating* available via air dates, not about the manual path being absent). The implementation contradicts both docs: the documented in-app maintenance of next_ep and available is missing.

Severity: medium is defensible but arguably high — this is a missing UI affordance on a secondary tracking aid (The List), with a partial workaround via CSV re-import for next_ep, and zero playback/sync/correctness impact. The genuinely irrecoverable part is `available` (no hand-set path at all), which undermines the weekly-airing "is it out" tracking for CurrentSeason shows. I'd lean low-to-medium; leaving medium as a reasonable rating.

</details>

### ⚪ LOW · Re-selecting the already-playing entry still emits SetNowPlaying, yanking seek authority back to Server

**`dessplay/src/ui/app.rs:743-753`** · _bug_

`play_selected` always pushes `Mutation::SetNowPlaying { file: Some(hash) }`, and only conditionally adds the pause when the file actually changes. But the server resets seek authority to Server on ANY non-datagram NowPlaying op (dessplay-rendezvous/src/server.rs:864: `now_playing_changed = matches!(op, CrdtOp::NowPlaying(_)) && !via_datagram`, then server_write SeekAuthority::Server). A redundant LWW write is applied unconditionally (server.rs:856-857). So pressing Enter on the row that is already now-playing silently takes seek authority away from whoever currently holds it (e.g. a user who just manually seeked), forcing the group back to leader-following — a side effect the spec says should not occur for a non-transition.

- **Spec:** design.md, EOF/Playback Rules: "Re-selecting the entry that is already now-playing is not a transition and does not pause." The matching code comment guards only the pause with the same `now_playing != Some(hash)` condition.
- **Suggested fix:** Return no actions (or omit the SetNowPlaying) when `self.snapshot.view.now_playing == Some(hash)`, so re-selecting the current entry is a true no-op; alternatively gate the server's seek-authority reset on an actual value change (as the StateMerge path at server.rs:924-927 already does).

<details><summary>Verification trail — code pointers</summary>

Mechanism fully traced and confirmed. (1) components.rs:1033-1034: Enter unconditionally emits Msg::PlaySelected(hash), no same-entry guard. (2) app.rs:743-753: play_selected pushes Mutation::SetNowPlaying { file: Some(hash) } unconditionally; only the Paused intent is guarded by `if self.snapshot.view.now_playing != Some(hash)`. (3) sync.rs:516 -> state.rs:409 set_now_playing -> reg_put (state.rs:293-300) -> LwwCell::write (lww.rs:83-85), which always returns Lww::new(ts, value) with NO value-equality check, so a same-value write still yields a CrdtOp::NowPlaying. (4) sync.rs:582-586: NowPlaying is not a position mutation, so the op is sent on the reliable control stream (not via_datagram). (5) server.rs:864: now_playing_changed = matches!(op, CrdtOp::NowPlaying(_)) && !via_datagram == true regardless of value change. (6) server.rs:871-877: server then writes SeekAuthority::Server. Spec (design.md:415-423) couples the seek-authority reset with the transition for a *different* file and explicitly says re-selecting the already-now-playing entry "is not a transition and does not pause"; the code honors only the no-pause half. So the claim's chain is accurate. Downgraded to low because: seek authority is Server "for most of an episode" (design.md rule 5) and is only held by a user right after a manual seek, so the precondition is uncommon; and when it fires the effect self-heals — Server authority falls back to leader-following (furthest-ahead peer with the file), so the group re-converges forward without freeze or group-rewind in the common case. Real and actionable but narrow impact; correct fix is to guard the SetNowPlaying emission (or have the server skip the authority reset when the NowPlaying value is unchanged).

</details>

### ⚪ LOW · Users-pane `a` clears any user's Away regardless of who set it

**`dessplay/src/ui/app.rs:791-807`** · _spec-mismatch_

`Msg::ToggleAway` clears an existing Away whenever `manual_override` is `Some(Away{..})`, ignoring the `set_by` field. The keyboard table scopes the clear half of this toggle to an Away the acting user set themselves. As written, a third party can press `a` on a peer to clear an Away that someone else set (and that the marked user is supposed to clear via their own "I'm here" action).

- **Spec:** design.md, Keyboard Shortcuts: "`a` | Users | Mark selected user as Away (or clear an Away you set)". Also User States: Away "is cleared by a deliberate 'I'm here' action from the marked user's client."
- **Suggested fix:** Only clear when the existing `ManualState::Away { set_by }` equals `self.me`; otherwise treat `a` as a re-mark (or a no-op). If the looser any-clearer behavior is actually intended given the five-friends threat model, update the keyboard table to drop "you set".

<details><summary>Verification trail — code pointers</summary>

Confirmed against code and spec. In /home/svein/dev/dessplay/dessplay/src/ui/app.rs lines 791-807, the `Msg::ToggleAway` handler computes `currently_away` via `matches!(self.snapshot.view.manual_override.get(&user), Some(Some(ManualState::Away { .. })))`. The `{ .. }` pattern discards the `set_by` field, so ANY existing Away (whoever set it) is treated as clearable; when true the override is set to `None`. There is no `set_by == self.me` guard. The keyboard source at dessplay/src/ui/components.rs:886-891 emits `Msg::ToggleAway(selected_row.name)` for whatever Users-pane row is highlighted, with no actor-scoping, so a third party can press `a` on a peer to clear an Away another user set. The data needed to scope is present: `self.me: UserId` (app.rs:160) is the acting user, and `ManualState::Away { set_by: UserId }` (dessplay-core/src/types.rs:250-254) records the setter. The spec contradicts the code: design.md Keyboard Shortcuts table says "`a` | Users | Mark selected user as Away (or clear an Away you set)" — the clear is explicitly scoped to "an Away you set" — and the User States section frames the normal clear path as the marked user's own "I'm here" action (unpause / send chat). The code's unscoped clear thus deviates from the documented behavior. Severity low is appropriate: the threat model already accepts that any peer can affect availability ("any peer can mark any other peer as Away... by design"), so this is a minor semantic/UI deviation rather than a security or convergence issue. Fix is a one-line guard adding `if set_by == &self.me` to the currently_away match.

</details>

### ⚪ LOW · All-Series sort mode is never persisted across sessions

**`dessplay/src/ui/components.rs:1336-1342`** · _spec-mismatch_

Pressing `s` in All-Series mode toggles `SeriesPane.sort` in memory and emits `Msg::ToggleSeriesSort`, which only triggers `refresh_series`. Nothing writes the choice to the database: `Settings` (config.rs:186) has no `series_sort` field, no SaveSettings is emitted for the toggle, and `SeriesPane::default()` always starts at `SeriesSort::Title`. So the sort resets to title-order on every launch.

- **Spec:** design.md, Adding Files to the Playlist (#6): "Sort mode for All Series is persisted across sessions." Also the keyboard table: `s` Series (All mode) toggle sort.
- **Suggested fix:** Add a `series_sort` field to Settings (load/save like `subtitle_mode`), initialize `SeriesPane.sort` from it in `Ui::with_setup`, and emit a SaveSettings (or persist) when `ToggleSeriesSort` fires.

**Status (2026-06-28): fixed.** Done exactly as suggested — the code was the wrong side (design.md is correct that the sort persists). `Settings` gained a `series_sort: SeriesSort` field loaded/saved like `subtitle_mode` (config.rs), with `SeriesSort::as_str`/`parse` added in props.rs for the string round-trip. `SeriesPane` gained `set_sort`, called from `Ui::with_setup` to seed the pane from the persisted setting. The `Msg::ToggleSeriesSort` arm in `update()` is split out from `CycleSeriesMode`: it now mirrors the pane's flipped sort into `self.settings.series_sort` and returns a `SaveSettings`. Regression tests: `all_series_sort_toggle_persists` (ui_app.rs — pressing `s` in All mode emits a `SaveSettings` carrying `SeriesSort::Year`; confirmed FAILING with the old grouped no-op arm), `series_sort_initializes_from_settings` (app.rs — `with_setup` seeds the pane to `Year`), and the extended `settings_round_trip` (config.rs).

<details><summary>Verification trail — code pointers</summary>

Independently verified against code and spec. (1) components.rs:1336-1342: `s` in All mode toggles in-memory `self.sort` and emits `Msg::ToggleSeriesSort`. (2) app.rs:763: `Msg::ToggleSeriesSort` only calls `refresh_series()` and returns `None` — no `UserAction::SaveSettings` is emitted (the SaveSettings sites are app.rs:350/962 and run.rs:609/928, none reachable from the sort toggle). (3) A repo-wide grep for `series_sort`/`sort_mode` across all of `dessplay/` finds nothing; `Settings` (config.rs:186-220, Default at 222-240) has no sort field and there is no storage column/key for it. (4) `SeriesPane` is `#[derive(Default)]` (components.rs:1069) and `SeriesSort` defaults to `Title` (props.rs:470-477, `#[default] Title`); `SeriesPane::default()` is the sole constructor (app.rs:213). There is no `set_sort` method and no startup path loading a saved sort — `self.series.sort()` is only read (app.rs:469). Therefore the sort resets to Title every launch. Spec contradicts this: docs/design.md:114 "Sort mode for All Series is persisted across sessions." Severity low is correct: cosmetic UX, no correctness/sync/playback impact.

</details>

### ⚪ LOW · List edit modal only saves on Ctrl-S, the trap the settings modal deliberately avoids

**`dessplay/src/ui/modals.rs:1020-1026,1225`** · _bug_

ListEditModal's only save trigger is `ctrl(ev) == Some(Key::Char('s'))` (line 1225), and keybindings() advertises only `("Ctrl-s", "Save")` (1023). The SettingsModal explicitly added a capital-`S` key AND a `[Save]` row because "Ctrl-S == XOFF" is unreliable in terminals without the enhanced keyboard protocol (modals.rs:711, 654-657). On such a terminal a user can open a List edit modal, change fields, and have no reachable way to save — Esc discards the edits. This is a latent defect by the project's own stated standard for save keys.

- **Suggested fix:** Mirror SettingsModal: accept capital `S` via typed(), and/or add a selectable [Save] row; keep Ctrl-s as the alias.

<details><summary>Verification trail — code pointers</summary>

Verified in /home/svein/dev/dessplay/dessplay/src/ui/modals.rs. ListEditModal::on() (lines 1214-1249) has exactly one save trigger: `ctrl(ev) == Some(Key::Char('s'))` → Msg::ListEntrySaved (line 1225). There is no `typed()` call in the handler, so no capital-`S` path; Enter (1237-1244) only edits a field or cycles status; Esc (1245) closes/discards. ListEditModal::render() (1028-1062) draws no `[Save]` row. keybindings() (1020-1026) advertises only `("Ctrl-s", "Save")`. By contrast SettingsModal deliberately provides three save paths and documents the reason: line 712 Ctrl-S alias with comment 710-711 ("Ctrl-S is kept as an alias for terminals where it isn't eaten as XOFF; capital `S` and the `[Save]` row are the reliable paths"), capital-`S` via typed() at line 722, and a `[Save]` row (rendered 648-667, handled 776-778) with comment 652-655 ("Saving needs no Ctrl combo: a plain `[Save]` row, alongside the capital-`S` key (avoids the Ctrl-S == XOFF terminal trap)"). So the project itself establishes that Ctrl-S as the sole save key is unreliable on terminals lacking the enhanced keyboard protocol, and ListEditModal violates that standard: on such a terminal there is no reachable save, and Esc discards. The claim is accurate; this is a real, actionable inconsistency. Low severity is correct given the narrow trigger conditions and the workaround on capable terminals.

</details>

### ⚪ LOW · overlay() multiplies u16 Rect width/height by percent, overflowing on very wide terminals

**`dessplay/src/ui/modals.rs:48-57`** · _bug_

`overlay` computes `area.width * percent_x / 100` and `area.height * percent_y / 100` entirely in u16 (ratatui Rect fields are u16). With percent_x=70 the product overflows u16 once area.width >= 937 (>=1093 at 60%, used by ListEditModal/AniDbSearchModal). That panics in debug builds and silently wraps in release, producing a garbage overlay rect. Rare (needs a very wide terminal), but a genuine arithmetic-overflow panic on the modal-render path.

- **Suggested fix:** Widen the intermediate to u32 (e.g. `(area.width as u32 * percent_x as u32 / 100) as u16`) before clamping.

**Status (2026-06-28): fixed.** `overlay()` now widens both `area.width * percent_x` and `area.height * percent_y` to `u32` before the `/ 100` and clamp, casting back to `u16` only after the `.min(area.width)` clamp guarantees the value is in range. Regression test `overlay_does_not_overflow_on_a_very_wide_terminal` (modals.rs) builds a 2000×2000 `Rect` and asserts a centered, clamped `1400×1400` overlay; before the fix it panicked with `panic_const_mul_overflow` inside `overlay` (debug overflow-checks).

<details><summary>Verification trail — code pointers</summary>

modals.rs:49-50 compute `area.width * percent_x / 100` and `area.height * percent_y / 100`. Rust parses this as `(area.width * percent_x) / 100`, so the multiply is done in u16 (Rect.width/height are u16 — confirmed in ratatui-core-0.1.1 src/layout/rect.rs:136-140). For percent_x=70 the product exceeds u16::MAX (65535) once area.width>=937 (937*70=65590); at 60% (ListEditModal/AniDbSearchModal, overlay(area,60,60) at lines 1029/1114) once area.width>=1093 — matching the claim's thresholds precisely. The `.max(20).min(area.width)` clamp runs after the multiply, so it does not prevent the overflow. Cargo.toml defines no `[profile.dev]` overriding overflow-checks, so dev builds (default overflow-checks=on) panic and release builds (default off) silently wrap to a garbage rect. No guard or design note mitigates it. Reachability requires a ~937+ column/row terminal, which is rare; the claim states this and rates it low, which is correct.

</details>

### ⚪ LOW · Settings media-root reorder uses J/K (and j/k), not the documented ctrl-j/ctrl-k

**`dessplay/src/ui/modals.rs:721-736`** · _spec-mismatch_

The settings modal reorders media roots with bare `j`/`J`/`k`/`K` (typed match at 721-736), but design.md's Settings Screen section says "Media roots can be reordered with ctrl-j/ctrl-k." The code's rationale (Ctrl-J == LF collides in terminals lacking the enhanced keyboard protocol) is sound and matches how the playlist pane reorder is documented ("J / K (or j / k)"), so the CODE is right and design.md line 50 is stale.

- **Spec:** design.md Settings Screen: "Media roots can be reordered with ctrl-j/ctrl-k."
- **Suggested fix:** Update design.md line 50 to read J/K (or j/k), consistent with the playlist reorder keys and the code.

<details><summary>Verification trail — code pointers</summary>

Confirmed against code and spec. modals.rs:721-736: the SettingsModal reorders media roots via `typed(ev)` matching `Some(c @ ('j' | 'J' | 'k' | 'K')) if self.sel >= FIXED_FIELDS` (swaps roots[index]/roots[target], carries cursor). The accompanying comment (lines 715-719) explicitly says "`J`/`K` (and lowercase `j`/`k`) reorder ... Bare letters rather than Ctrl-J/Ctrl-K, which collide with control codes (Ctrl-J == LF) in terminals lacking the enhanced keyboard protocol." I grep-verified there is no ctrl-j/ctrl-k handler in the modal — the only Ctrl combo handled is Ctrl-S (line 712). docs/design.md line 50 (Settings Screen section) states "Media roots can be reordered with ctrl-j/ctrl-k." These contradict. The code's bare-letter approach is corroborated as intentional by the design.md keyboard table, which documents the playlist reorder as "J / K (or j / k)" with the same Ctrl-collision rationale ("Ctrl-M == Enter in terminals lacking the enhanced keyboard protocol"). So the code is correct and design.md line 50 is stale. Real, actionable doc mismatch; severity low (docs-only, no functional impact) is correct.

</details>

---

## Client — Lifecycle, config & storage

**Files:** `dessplay/src/{run,config,storage,main,seeder,client}.rs`.
**Read first:** design.md → *Data Storage* (client tables, the "storage never reads the clock" and "flags/env override but are never persisted" invariants), *Client Roles* / *Seeder Behavior*, architecture.md → actor wiring.
**Theme:** two invariant violations worth attention — flag/env overrides get **persisted** on the next settings save (a one-off `--username` becomes permanent), and storage **reads the monotonic clock** in `save_state`/`load_state` (breaks the deterministic-test invariant). Plus seeder fetch-ordering drift, non-UTF-8 path corruption via `to_string_lossy`, and a `--pipeline-depth` default that disagrees with its help text.

### 🟠 MEDIUM · Flag/env overrides (--username, env DESSPLAY_PASSWORD) get persisted on the next settings save, contradicting "never persisted"

**`dessplay/src/run.rs:560-586, 592`** · _spec-mismatch_

design.md (Data Storage, SQLite Database): "Command-line flags and environment variables override stored settings at runtime but are never persisted." The code's own comment at run.rs:117-120 reaffirms that --username/--server override "never persisted." But during interactive bootstrap the resolved overrides are folded directly into the in-memory Settings: settings.username = resolve_username(flag, stored, $USER) (run.rs:560) and settings.password is filled from --password / DESSPLAY_PASSWORD when no stored password exists (run.rs:581-586). That same Settings is then handed to the UI (run.rs:592, Ui::with_setup(..., settings.clone(), ...)). The UI keeps it as self.settings and SaveSettings emits SaveSettings(Box::new(self.settings.clone()), ...) (ui/app.rs:350), which run.rs persists verbatim (first-run run.rs:609-616; mid-session run.rs:928-931 -> Storage::save_settings). Concrete consequences: (1) a one-off `--username Foo` flag becomes the permanently stored username the first time the user saves anything in the settings screen, clobbering the previously stored identity; (2) an env-only operator who deliberately keeps the password out of the DB via the .env DESSPLAY_PASSWORD gets it written to the settings table (plaintext) on the first settings save -- reachable mid-session without first-run setup, since needs_setup is false once env supplies password and $USER supplies username. storage.rs/config.rs themselves are correct (they faithfully persist what they are given); the defect is that the override layer never isolates flag/env-sourced values from the persisted Settings.

- **Spec:** design.md, Data Storage > SQLite Database: "Command-line flags and environment variables override stored settings at runtime but are never persisted."
- **Suggested fix:** Keep flag/env overrides in a separate runtime overlay rather than mutating the Settings that seeds the UI/save path; or, before Storage::save_settings, restore the flag/env-sourced username/password fields to their stored (pre-override) values so a settings save never writes a value the user didn't type.

**Status (2026-06-28): fixed (approach a — runtime overlay).** The `--username`/`$USER` override is no longer folded into the persistable `Settings`. A new single chokepoint `resolve_runtime_identity` (run.rs) returns the runtime identity (flag > stored > `$USER`, used for the UI/session/auth) while leaving `settings.username` at its **stored** value — the flag is only folded in as a *first-run prefill* (no stored value to clobber, and the modal confirms it). So the `Settings` handed to the UI/save path structurally never carries an untouched override; the F2/F3 save persists the stored value. A **real user edit still persists** because the settings modal edits that stored base directly — any value in the save is either the stored value or one the user typed, never the override (so the "deliberately re-typed the same value" edge of approach b can't arise). A matching `identity_locked` guard in `Ui` (app.rs, `SettingsSaved`) keeps `self.me` on the override when it differs from the persisted name, so opening F3 and saving in an overridden session no longer silently re-keys our own writes under the stored name (the 2026-06-14 identity-agreement invariant). The first-run env-password pre-fill is untouched (it was already guarded by `password.is_none()`, i.e. first-run only, and the password is auth-resolved separately in `prepare()`). Regression tests (run.rs) confirmed failing→passing: `flag_username_override_is_not_folded_into_persistable_settings`, `flag_username_override_survives_a_settings_save` (stored "Real" + `--username Foo` → save leaves stored "Real"; was "Foo"), plus no-regression `user_edited_username_still_persists` and `first_run_prefills_username_from_flag_then_env`, and `locked_identity_is_not_moved_by_a_settings_save` (app.rs). Note: `--media-root` has the *same* leak class (the comment at `resolve_media_roots` claims "never persisted" but a settings save persists the flag roots via `set_media_roots`); left for a separate finding — the roots-editing flow needs its own touched-vs-override handling.

<details><summary>Verification trail — code pointers</summary>

The core defect is real and confirmed against the code, though one of the two cited consequences is overstated.

CONFIRMED — `--username` flag leaks into persisted settings:
- run.rs:560 `settings.username = resolve_username(args.username.clone(), settings.username.clone(), env_user)` — with `--username Foo` and a stored username "Bob", `resolve_username` (run.rs:165-171: `flag.or(stored).or(env_user)`) returns Some("Foo").
- That same `settings` is cloned into the UI at run.rs:592 `Ui::with_setup(..., settings.clone(), ...)`, stored as `self.settings` (app.rs:227).
- It is then persisted verbatim on any settings save. Two reachable mid-session paths, neither requiring first-run setup:
  • F2 → `cycle_subtitle_mode` (app.rs:345-351) returns `UserAction::SaveSettings(Box::new(self.settings.clone()), ...)` — no modal, the username field is never re-confirmed.
  • F3/`/settings` → `open_settings` (app.rs:542-546) builds the modal from `self.settings.clone()` (carrying "Foo"); saving it emits `SaveSettings` (app.rs:962).
- run.rs:928-931 handles `SaveSettings` by calling `self.storage.save_settings(&saved)` with no stripping; `Settings::save` (config.rs:320-323) writes `username`/`password` verbatim. So "Foo" clobbers the stored "Bob".
This directly violates the spec (design.md, Data Storage > SQLite Database: "Command-line flags and environment variables override stored settings at runtime but are never persisted") and the code's own comment (run.rs:117-120, "mirroring how `--username` / `--server` override their stored settings ... never persisted"). Note `--server` does NOT have this bug: it is resolved separately inside `prepare()` (run.rs:233) and never folded into the persisted UI settings.

PARTIALLY REFUTED — env `DESSPLAY_PASSWORD` "reachable mid-session without first-run": this premise is wrong. The env-password override at run.rs:581-586 is guarded by `if settings.password.is_none()`. But `needs_setup` (config.rs:252: `username.is_none() || password.is_none()`) is computed at run.rs:554 from the STORED settings, BEFORE the overrides at 560/581. So whenever the stored password is None (the only case the env override fires), `needs_setup` is necessarily true and the first-run flow (run.rs:606-627) runs. The env password therefore can only be persisted via the first-run modal, which run.rs:531-534 explicitly frames as intended ("Prefills ($USER, the .env password, flags) only become the modal's editable defaults"). The claim's "needs_setup is false once env supplies password and $USER supplies username" misreads the ordering.

Net: the defect (flag-sourced values are not isolated from persisted Settings) is genuine and actionable for `--username` — a transient flag silently becomes the permanently stored identity on the next save. The env-password half is real only through first-run pre-fill, not the mid-session mechanism described. Medium severity is appropriate: a real, documented-guarantee violation reachable via the ubiquitous F2 keypress, though it requires the user to actually pass `--username` interactively.

</details>

### 🟠 MEDIUM · `--media-root` flag/env override gets persisted on the next settings save (same class as the username leak)

**`dessplay/src/run.rs:121-127, 577, 635, 977-990`** · _bug_ · _discovered 2026-06-28 while fixing the `--username` leak above; not part of the original review pass_

`resolve_media_roots` (run.rs:121-127) returns the `--media-root` flag values when any are given, **replacing** the stored roots entirely, and that merged list is handed to the UI (run.rs:635). On any `SaveSettings` — including unrelated saves like an F2 subtitle-mode cycle — run.rs:984 calls `storage.set_media_roots(&roots)` with the UI's current roots, which still carry the flag override. So launching once with `--media-root /tmp/x` and later saving anything **permanently overwrites the stored media roots** with the transient flag value. This is the identical leak class to the `--username` finding above and violates the same invariant (design.md, Data Storage: "flags/env override at runtime but are never persisted"); the `resolve_media_roots` comment even claims the roots are never persisted. It was deliberately left out of the username fix because the roots-editing flow needs its own touched-vs-override handling: unlike the single username field, the settings modal genuinely edits the roots list, so a fix must distinguish "user edited the roots" (persist) from "untouched flag override present" (don't persist) — likely the same runtime-overlay shape as the username fix (hand the UI the stored roots as the persistable base; use the override-merged roots only for runtime scanning).

- **Spec:** design.md Data Storage: "Command-line flags and environment variables override stored settings at runtime but are never persisted."
- **Suggested fix:** apply the same approach-(a) separation used for `--username`: keep the override-merged roots as a runtime-only value for scanning, and persist only the stored base plus genuine in-modal edits. Add a regression test mirroring `flag_username_override_survives_a_settings_save` for media roots.

**Status (2026-06-28): fixed.** Applied the same approach-(a) separation as the `--username` fix. A new chokepoint `resolve_runtime_media_roots` (run.rs) mirrors `resolve_runtime_identity`: it returns a `RuntimeMediaRoots { runtime, persistable }` split — `runtime` honours the `--media-root` override (flag wins when non-empty, else stored; fed to the file actor for scan/serve/resolve), while `persistable` keeps the **stored** base and only takes the flag as a *first-run prefill* when nothing is stored. The UI is now seeded with `media_roots.persistable` (run.rs), so the settings modal shows it and **every** save (including an unrelated F2 subtitle cycle, which carries `self.media_roots`) writes the persistable base, never an untouched override. A **real edit still persists**: the modal edits the persistable base directly, so the carried roots are either the stored base or the user's actual edits. **Runtime effect preserved**: `FileConfig.media_roots` is `media_roots.runtime` (the override), and a new `roots_locked` flag (`persistable != runtime`, the analogue of `identity_locked`) keeps the override after first-run setup; in the session loop, a save only calls `shell.set_media_roots` when the saved roots *differ* from the tracked persistable base (`SessionLoop.media_roots`), so an unrelated save can't silently switch the running file actor off an active `--media-root`. **First-run intact**: `needs_setup` is computed from `media_roots.runtime` (so a flag-supplied run isn't forced into setup), and on a first run with no shadowing override the file actor scans the roots the user just confirmed (mirroring `me` following the confirmed username). The stale `resolve_media_roots` "never persisted" comment is corrected (it is now documented as the *runtime* resolver, distinct from the persistable base). Regression tests (run.rs) confirmed failing→passing by toggling the one-line persistable rule: `media_root_override_is_not_folded_into_persistable_base` and `media_root_override_survives_a_settings_save` (stored `[/real]` + `--media-root /flag` → save leaves stored `[/real]`; was `[/flag]`), plus no-regression `user_edited_media_roots_still_persist`, the runtime-effect assertion (`split.runtime == [/flag]`) inside the not-folded test, and `first_run_prefills_media_roots_from_flag`. The existing `media_roots_flag_overrides_stored` remains the runtime-resolver coverage.

### ⚪ LOW · --pipeline-depth help text and run.rs doc state default 16, but production default is 48

**`dessplay/src/main.rs:44-45`** · _bug_

The CLI help for --pipeline-depth says "(default 16)" (main.rs:44-45) and run.rs's download_config doc comment says "or the default of 16" (run.rs:129-130), but the production code path used by both interactive clients and seeders is run::download_config, which sets pipeline_depth = args.pipeline_depth.unwrap_or(48) (run.rs:135). The unit test pipeline_depth_flag_overrides_default asserts the unset default is 48 (run.rs:1416), and the HeadlessArgs field doc itself says "(default 48)" (run.rs:42-43). So DownloadConfig::default()'s 16 is always overridden in production; a user reading --help is told the wrong default for the one knob the flag exists to tune.

- **Suggested fix:** Change the --pipeline-depth help string (main.rs:44) and the download_config doc comment (run.rs:130) to state default 48, matching the code and the test.

<details><summary>Verification trail — code pointers</summary>

All cited facts are accurate. main.rs:43 (the clap doc-comment help for --pipeline-depth) reads "(default 16)"; run.rs:130 download_config's doc says "or the default of 16"; but run.rs:135 is `pipeline_depth: args.pipeline_depth.unwrap_or(48)`, the sole production construction used by both interactive clients and seeders. The HeadlessArgs field doc at run.rs:42-43 correctly says "(default 48)", and the test at run.rs:1416 asserts download_config(&HeadlessArgs::default()).pipeline_depth == 48. DownloadConfig::default() at download.rs:50 sets pipeline_depth: 16, but that 16 is always overridden by unwrap_or(48) in the production path, so the help/doc figure of 16 is never the effective default for this flag. Genuine help-text/doc inconsistency: a user reading --help is told the wrong default for the knob the flag exists to tune. No guard or design note rescues it. Low severity is correct (documentation only, no runtime behavior change).

</details>

### ⚪ LOW · Seeder auto-fetch does not prioritize unwatched entries first

**`dessplay/src/seeder.rs:70-101`** · _spec-mismatch_

SeederTransfer::on_state iterates view.playlist in plain playlist order and issues StartDownload for every missing entry that has a source, with no unwatched-first prioritization. design.md (Seeder Behavior) specifies the seeder 'downloads every playlist entry as it is added (unwatched entries first, in playlist order)'. Because all missing entries get StartDownload concurrently (each bounded only by pipeline_depth per source), when bandwidth is saturated the seeder may complete already-watched back-catalog files before the next unwatched episode the group actually needs. Functionally everything still downloads; only the completion ordering deviates.

- **Spec:** design.md, Client Roles > Seeder Behavior: 'A seeder downloads every playlist entry as it is added (unwatched entries first, in playlist order)'
- **Suggested fix:** Sort/iterate the playlist so entries whose group watched flag is unset are resolved/started before watched ones (still within playlist order), or stage StartDownload calls in that priority order.

<details><summary>Verification trail — code pointers</summary>

Code path verified. In /home/svein/dev/dessplay/dessplay/src/seeder.rs:70-101, `on_state` does `for entry in &view.playlist` (line 71) and issues `StartDownload` for every entry where `self.have.get(&file) == Some(&false)` (line 86) that has a source — with no reference to watched state (grep for "watched" in seeder.rs returns nothing) and no reordering. `view.playlist` is, per playlist.rs:1-54 and state.rs:742-743, "in display order... sorted by (position, hash)" (dense Identifier order). Since EOF does not remove entries (they persist as muted play history at their original earlier positions while new unwatched entries are appended last), plain playlist order is watched-history-first — the inverse of the spec's "unwatched entries first." design.md:649-651 (Client Roles > Seeder Behavior) explicitly states: "A seeder downloads every playlist entry as it is added (unwatched entries first, in playlist order)". The download layer (download.rs:168-240 `Downloads` is a flat HashMap with no global cap; file.rs:601-627 starts each independently, capped only by pipeline_depth per source) confirms all missing entries download concurrently, so the deviation manifests as completion ordering under bandwidth saturation, exactly as the claim states. No guard, default, or design note neutralizes this: the interactive client (session.rs:719-734 prefetch_window, anchored at now-playing and moving forward) does avoid watched back-catalog, but the seeder has no equivalent. The mismatch is real though minor (everything still downloads; in steady state the seeder already holds watched files, so the missing set is usually the upcoming unwatched episodes). Low severity is correctly rated.

</details>

### ⚪ LOW · Not-watching placeholder shows only who is watching, not who is not; label includes paused/away users

**`dessplay/src/session.rs:277-298`** · _spec-mismatch_

placeholder_lines (the content fed to placeholder::render_to) builds a single status line that lists peers whose derived state is not NotWatching as 'Watching: ...' (or 'Nobody is watching this'). design.md (Placeholder Image) says the image should display 'Current session status (who's watching, who's not)'. The 'who's not' half is omitted. Additionally the filter only excludes DerivedUserState::NotWatching, so users who are Paused or Away are listed under the 'Watching:' label, which can misstate the session status the placeholder is meant to convey.

- **Spec:** design.md, File Management > Placeholder Image: the image displays 'Current session status (who's watching, who's not)'
- **Suggested fix:** Include the not-watching/absent users in the status block (e.g. a second 'Not watching: ...' line) and tighten the 'Watching:' classification so paused/away users are not labeled as watching.

<details><summary>Verification trail — code pointers</summary>

Verified against code and spec. In /home/svein/dev/dessplay/dessplay/src/session.rs:270-299, placeholder_lines builds only a `watching` vector (peers that are Interactive and whose derive::user_state is anything except DerivedUserState::NotWatching), rendered as "Watching: {names}" or "Nobody is watching this". No "not watching" list is ever produced. DerivedUserState (dessplay-core/src/derive.rs:22-39) defines Ready, Maybe, Paused, Away, NotWatching; since the filter excludes only NotWatching, users who are Paused (line 30) or Away (line 32) are included under the "Watching:" label. The spec (docs/design.md:1379, "Placeholder Image") states the image displays "Current session status (who's watching, who's not)" — so the "who's not" half is omitted, and the "Watching:" label can misstate status by including paused/away peers. Both factual halves of the claim are confirmed. The code's own docstring (lines 268-269: "and who *is* watching") shows this is a documented partial implementation rather than a regression, but the spec text was not narrowed to match, so a real spec/implementation gap exists. Severity 'low' is correct: it affects only the placeholder PNG shown to the local not-watching user, is cosmetic, and is not seen by other clients.

</details>

### ⚪ LOW · Path columns stored via to_string_lossy() silently corrupt non-UTF-8 paths

**`dessplay/src/storage.rs:331, 500, 572, 631, 645, 699`** · _bug_

Every path written to a TEXT column goes through Path::to_string_lossy(): set_media_roots (storage.rs:331), upsert_cache_entry (500), upsert_hash_cache (572), remove_hash_cache (631), set_manual_mapping (645), set_series_map_dir (699). On Linux a path may contain arbitrary non-UTF-8 bytes; to_string_lossy replaces them with U+FFFD, so the round-tripped PathBuf no longer points at the real file. Effects: a media root or cached/mapped file with non-UTF-8 bytes can never be resolved or matched again; remove_hash_cache(path) won't match the (also-lossy) stored key; and two distinct non-UTF-8 paths that lossy-collapse to the same string would spuriously violate media_roots.UNIQUE(path), failing the whole set_media_roots transaction. Low probability and a pervasive existing choice, but a real latent correctness defect.

- **Suggested fix:** Store paths as BLOBs of their OS bytes (std::os::unix::ffi::OsStrExt) with a portable fallback, or at minimum reject/normalize non-UTF-8 paths at the settings/media-root boundary so the lossy conversion can never silently desync a stored path from disk.

<details><summary>Verification trail — code pointers</summary>

Verified all six cited write sites in /home/svein/dev/dessplay/dessplay/src/storage.rs: set_media_roots (line 331), upsert_cache_entry (line 500), upsert_hash_cache (line 572), remove_hash_cache (line 631), set_manual_mapping (line 645), set_series_map_dir (line 699) all store paths via Path::to_string_lossy() into TEXT columns. The matching read sites reconstruct via PathBuf::from(String): media_roots line 319, hash_cache line 608, manual_mapping line 660, series_map_dir line 714. So the round-trip is PathBuf -> to_string_lossy -> String -> PathBuf. On Linux a path is an arbitrary byte sequence; to_string_lossy substitutes U+FFFD for non-UTF-8 bytes, so the reconstructed PathBuf cannot point at the real file. Confirmed effects: a non-UTF-8 media-root/cache/mapping path is unresolvable after round-trip, and two distinct non-UTF-8 paths that collapse to the same lossy string would violate media_roots UNIQUE(path) and fail the set_media_roots transaction (lines 327-334). No spec section under docs/ governs path encoding and there is no upstream UTF-8 guard. One sub-claim is inaccurate: 'remove_hash_cache won't match the (also-lossy) stored key' is wrong, since upsert and remove both apply the same deterministic to_string_lossy, so a DELETE for the same path matches its own stored key; the real risk there is cross-path collision, not self-mismatch. That is a minor imprecision in one of three effects, not in the core defect. The defect is genuine but only triggers on non-UTF-8 paths (rare), so the reviewer's 'low' severity and 'low probability / pervasive existing choice' framing are accurate; actionable fix is to store paths as BLOB via OsStrExt::as_bytes on Unix.</reasoning>
<parameter name="corrected_severity">low

</details>

### ⚪ LOW · Storage reads the monotonic clock in save_state/load_state, contradicting the documented "storage never reads the clock" invariant

**`dessplay/src/storage.rs:342, 356, 364, 381`** · _spec-mismatch_

The module header (storage.rs:11-13) and design.md (Schema) both state timestamps are caller-supplied and "storage never reads the clock -- keeps tests deterministic." save_state (storage.rs:342) and load_state (storage.rs:364) call std::time::Instant::now() to compute elapsed_ms for a debug log (storage.rs:356, 381). This is a literal clock read inside the storage layer. It is harmless in practice -- the value is monotonic, only logged, never stored, and no test asserts on it, so determinism is unaffected -- so the documented invariant is the over-broad side and should be narrowed to "no wall-clock reads that affect stored data or test output," or the Instant::now() timing should be dropped.

- **Spec:** design.md, Schema: "Timestamps are unix milliseconds, caller-supplied (storage never reads the clock -- keeps tests deterministic)."
- **Suggested fix:** Either remove the Instant::now()/elapsed_ms timing logs, or amend the doc/module comment to scope the invariant to stored timestamps only.

<details><summary>Verification trail — code pointers</summary>

Verified statically against storage.rs and the spec. storage.rs:342 (`let started = std::time::Instant::now();` in save_state) and 364 (same in load_state) read the monotonic clock; lines 356 and 381 use `started.elapsed().as_millis() as u64` as the `elapsed_ms` field of a `tracing::debug!`. The module header (storage.rs:11-13) states "storage never reads the clock, which keeps tests deterministic," and design.md (Data Storage > Schema) repeats it verbatim: "Timestamps are unix milliseconds, caller-supplied (storage never reads the clock -- keeps tests deterministic)." `Instant::now()` is literally a clock read, so the code contradicts the documented invariant. The claim is accurate that this is harmless: the `started` value is monotonic, used only for the debug log's `elapsed_ms`, never persisted (save_state stores the caller-supplied `now: i64` at line 351, not `started`), and no test asserts on the log field, so determinism is preserved. This is therefore a genuine but minor doc/spec-comment over-statement, not a functional bug — actionable by narrowing the wording (e.g. "no wall-clock reads affecting stored data or test output") or dropping the timing. Severity low is correct.

</details>

---

## Client — List import

**Files:** `dessplay/src/import.rs`.
**Read first:** design.md → *The List → Import* (section-header status rows, finished/dropped mapping, `--watchers` initials).
**Theme:** one bug — the Hiatus "Progress?" relabel duplicates the value into both `status_note` and `notes`.

### ⚪ LOW · Hiatus "Progress?" relabel duplicates the value into both status_note and notes

**`dessplay/src/import.rs:240-264`** · _bug_

On the Planning sheet the "Refresh / Haitus" section header relabels the column under its `Progress?` cell as `status_note` (lines 240-242). But that column is also a notes column: detect() classifies any header containing "notes" into `columns.notes` (line 145-146), and the Ivory sheet's last column "Extra Notes" is index 6 — exactly the index `Progress?` points at. The relabel never removes index 6 from `columns.notes`, so when the entry is built the same cell is read into BOTH `notes` (lines 257-261, filter_map over columns.notes) and `status_note` (line 264). Verified against spreadsheet/Things-to-watch, The List - Ivory.csv: e.g. Shirobako (row 44, cell index 6 = "13") imports as status_note=Some("13") AND notes=["13"]; the same happens for every hiatus entry (Zettai Karen Children "28", Record of Lodoss War "6", Tower of God "1", Golden Kamuy "27", Mitsuboshi Colours "3", Koi wa Futago "4"). The module doc (lines 16-18) says the Progress? cell "re-labels that column as the status note" — i.e. it should replace the notes role, not add a second copy. Result is a stray duplicated note column on every imported hiatus entry. Not caught by tests: planning_sheet_sections asserts shirobako.entry.status_note but never inspects shirobako.entry.notes.

- **Spec:** import.rs module doc lines 16-18: "Refresh / Haitus (-> Hiatus; a `Progress?` cell in the header row re-labels that column as the status note)." "re-labels" implies replacing the column's prior (notes) role.
- **Suggested fix:** When the Progress? relabel sets columns.status_note = Some(i), also remove i from columns.notes (e.g. columns.notes.retain(|&n| n != i)) so the cell is captured only once as the status note; restore it to notes when a later section clears status_note. Add a notes assertion to planning_sheet_sections to lock this in.

<details><summary>Verification trail — code pointers</summary>

Confirmed at code, data, and runtime levels in /home/svein/dev/dessplay/dessplay/src/import.rs. detect() lines 145-146 push every "notes"-containing header into columns.notes (a Vec<usize>); the Ivory fixture header (spreadsheet/Things-to-watch, The List - Ivory.csv line 1) has "Notes" (idx 4) and "Extra Notes" (idx 6), giving columns.notes = [4,6]. The Hiatus section relabel at lines 240-242 sets columns.status_note = position of "progress?" = idx 6 (Progress? sits in the last cell of CSV line 41) but never removes idx 6 from columns.notes. Entry build at lines 257-264 then reads idx 6 into BOTH notes (filter_map over columns.notes) and status_note (cell(...columns.status_note)). No downstream dedup: the only block touching notes/status_note afterward (lines 295-316) is gated on kind == SheetKind::Finished, so Planning/Hiatus rows are unaffected. Runtime check: I temporarily added assert_eq!(shirobako.entry.notes, vec!["13".to_string()]) to planning_sheet_sections and the test passed, proving Shirobako (CSV line 44, idx-6 cell "13") imports with notes == ["13"] AND status_note == Some("13"); every hiatus row 42-48 has the same shape (idx-4 empty, idx-6 progress value), so all seven get the stray duplicate. Edit reverted, git diff --stat clean. Test gap real: planning_sheet_sections lines 590-592 assert only status and status_note, never notes. Spec: module doc lines 16-18 ("re-labels that column as the status note") implies a role change, and design.md distinguishes status_note (hiatus progress) from free-form notes, so the duplicate is unintended. Low severity is appropriate: cosmetic data-quality issue on a one-shot import, no crash or data loss.

</details>

---

## Server — Rendezvous server

**Files:** `dessplay-rendezvous/src/server.rs`.
**Read first:** design.md → *Rendezvous Server*, *Presence* (Present/Lost@30s/Departed@60s), *Playback Rules* (server-forced intent→Paused; the EOF transition), architecture.md.
**Key entry points:** `handle_eof` (who may advance the group), `broadcast_op` (relay transport selection), the presence state machine.
**Theme — transport drift (Medium):** `broadcast_op` relays 100ms `PlaybackPosition` ops over the **reliable** control stream, reintroducing exactly the O(N²) head-of-line blocking the datagram-only design avoids. Also: EOF accepts a Paused reporter (beyond the documented Ready-or-Maybe).

### 🟠 MEDIUM · Server fans out playback-position ops over the reliable control stream, reintroducing the head-of-line blocking the design forbids

**`dessplay-rendezvous/src/server.rs:214-235`** · _spec-mismatch_

Clients send PlaybackPosition ops datagram-only at 100ms (with a 1s reliable catch-up) specifically to avoid head-of-line blocking (network-design.md "Exception -- playback position"; client implements this with a dedicated DatagramOnly path in dessplay/src/actors/network.rs). When the server relays a received StateOp to the other peers (handler at lines 840-878, relay call at line 866) it uses broadcast_op, which UNCONDITIONALLY does send_control (reliable, line 227) plus an eager datagram copy for EVERY op, including PlaybackPosition. So every 100ms datagram-received position is re-delivered RELIABLY to all other peers, producing O(N^2) reliable position frames through the server's control streams and the exact HOL blocking of state sync the design calls out as 'exactly the head-of-line blocking we are avoiding'. The server never inspects via_datagram or the op type when choosing relay transport.

- **Spec:** network-design.md Sync Flow: 'position ops are datagram-only at the 100ms cadence ... Reliable delivery of every stale position is exactly the head-of-line blocking we are avoiding'; state.rs:72 'High-frequency, datagram transport.'
- **Suggested fix:** In the StateOp relay path (or in broadcast_op) special-case PlaybackPosition: relay it datagram-only (or mirror the inbound transport via the existing via_datagram flag — datagram-received positions relayed datagram-only, the 1s reliable tick relayed reliably), so 100ms positions never hit the reliable control stream.

**Status (2026-06-27): fixed.** The relay path now **mirrors the inbound transport**. A new `relay_transport(op, via_datagram)` helper (`dessplay-rendezvous/src/server.rs`) states the rule once: a `PlaybackPosition` that arrived on the 100ms datagram fast path relays `DatagramOnly` (best-effort, no reliable copy); everything else — ordinary ops, server-authored writes, and the 1s reliable position tick (sent eager by the client) — relays `Eager` (reliable + an eager datagram copy, unchanged). `broadcast_op` gained a `RelayTransport` parameter; the StateOp relay site threads the flag already in scope, and `server_write`/`handle_eof` pass `Eager`. Regression tests in `dessplay-rendezvous/tests/position_relay.rs` drive raw `SimTransport` peers and assert *transport selection* by the `TransportEvent` variant each relayed op leaves on: a datagram-received position reaches the peer on the datagram channel and **not** on the reliable control stream (pre-fix it appeared on reliable — `control == 1`, test failed); a reliably-received position relays reliably; a non-position op still relays reliably. The separate ⚪ Low finding (order-free eager ops broadcast twice, server.rs:840-867) is **untouched** — this change does not add the per-op change-detection it calls for, and the double-broadcast of low-frequency control ops remains.

<details><summary>Verification trail — code pointers</summary>

Verified against code and spec. broadcast_op (dessplay-rendezvous/src/server.rs:214-235) unconditionally relays every op over the reliable control stream (conn.send_control at line 227) plus an eager datagram copy (lines 228-233); its own doc comment says "reliable, plus an eager datagram copy when it fits." The received-StateOp relay handler (server.rs:840-878) calls broadcast_op at line 866 for every applied op with no op-type or transport guard. The only op inspection is line 864 (CrdtOp::NowPlaying for seek authority) — nothing for PlaybackPosition. The via_datagram flag is in scope at line 851 but is used solely to choose apply_if_orderly vs apply, never to pick relay transport, and there is no datagram-only broadcast variant in the server (broadcast_op is the only state-op broadcaster per grep). Meanwhile the client (dessplay/src/actors/sync.rs:587-600) sends position ops datagram-only at 100ms with a single reliable SendEager tick per second, exactly to honor the spec: network-design.md:357-361 "position ops are datagram-only at the 100ms cadence ... Reliable delivery of every stale position is exactly the head-of-line blocking we are avoiding," and architecture.md:168-169 "playback position ops are datagram-only." Thus every applied 100ms datagram position is re-fanned-out reliably to all other peers (O(N^2) reliable position frames through server control streams), reintroducing the HOL blocking the design explicitly forbids — the server never inspects via_datagram or op type when relaying. Medium is correct: functionally works, but degrades the control stream precisely under the flaky-link conditions the design prioritizes; with N=5 it is real but not catastrophic.

</details>

### ⚪ LOW · EOF transition accepts a reporter whose derived state is Paused, beyond the documented 'Ready or Maybe'

**`dessplay-rendezvous/src/server.rs:958-979`** · _spec-mismatch_

handle_eof advances now-playing for DerivedUserState::Ready | Maybe | Paused (line 978), but design.md says the advancing report must come from a present, watching user that is 'Ready (committed) or Maybe'. Accepting Paused is broader than the enumerated positive list. In practice this is near-unreachable (a manually-paused player cannot reach EOF, and only the user who paused carries the Paused override) and the doc's negative enumeration ('not one whose derived state is Not Watching or Away') does match the code, so the doc's positive list looks like the stale side; flagged so the code comment's rationale and the doc are reconciled rather than left contradictory.

- **Spec:** design.md Playback Rules (EOF): 'the first report matching the current now-playing file from a present, watching user -- Ready (committed) or Maybe, but not a seeder and not one whose derived state is Not Watching or Away'.
- **Suggested fix:** Either drop Paused from the accepting arm to match the doc's 'Ready or Maybe', or update the EOF spec sentence to state that any present watcher except NotWatching/Away advances (matching the code comment's reasoning).

<details><summary>Verification trail — code pointers</summary>

server.rs:973-979 — handle_eof returns early only for DerivedUserState::NotWatching | Away (line 974); line 978 accepts Ready | Maybe | Paused, with an explicit comment (975-977) that Paused is intentionally admitted. The DerivedUserState enum (dessplay-core/src/derive.rs:22-39) has exactly {Ready, Maybe, Paused, Away, NotWatching}, so the code's accepted set is {Ready, Maybe, Paused}. design.md lines 406-408 give two enumerations for the qualifying reporter: a positive one ("Ready (committed) or Maybe") that omits Paused, and a negative one ("not one whose derived state is Not Watching or Away") that matches the code exactly. The doc thus contradicts itself, and the code follows the negative reading while diverging from the positive list — a real, narrow code/doc mismatch. It is intentional in code (the comment justifies Paused), and near-unreachable at runtime as the claim notes, so the fix is reconciling the doc's stale positive enumeration. Low severity is accurate; not a behavioral bug.

</details>

---

## Server — Storage

**Files:** `dessplay-rendezvous/src/storage.rs`.
**Read first:** design.md → *Data Storage* (server tables, the postcard `crdt_state` forward-compat decode), sync-state.md → compaction / chat archive.
**Theme — Medium, fix early:** `load_state().ok().flatten()` collapses a decode/IO **error** into "first run", so on genuine corruption the authoritative server silently wipes all state and resets to epoch 1 (it cannot re-sync from anyone). Match explicitly and abort on `Err`.

### 🟠 MEDIUM · A load_state error silently wipes the authoritative server state and resets epoch to 1

**`dessplay-rendezvous/src/server.rs:374-380`** · _bug_

`storage.load_state()` returns `Result<Option<StateSnapshot>>`, where `Err` means the persisted blob could not be decoded (genuinely corrupt/truncated, or a layout neither current nor `CrdtStateV1` — e.g. a server downgraded onto a newer-version blob) or a SQLite read failed. The caller does `s.load_state().ok().flatten()`, which collapses `Err` into `None` exactly like 'no row exists', then `unwrap_or` substitutes a fresh `StateSnapshot { epoch: Epoch(1), state: CrdtState::new() }`. So on any decode/read error the authoritative node silently discards ALL of its state — playlist, the never-pruned List, watched flags, AniDB metadata, file catalog, series relations, recent chat — and reboots at epoch 1. Worse, because the epoch is reset below the epoch every connected client already holds, each client treats its own (higher) epoch as stale on reconnect and adopts the empty epoch-1 snapshot, propagating the wipe to the whole group. storage.rs correctly distinguishes Ok(None) from Err; the forward-compat `decode_snapshot` fallback (state.rs:262) exists precisely to avoid this for the known version-bump case, but `.ok()` defeats the durability guarantee for every other failure mode. The server should fail loud (refuse to start so an operator can restore/inspect) rather than silently reset, since per the spec it 'cannot re-sync its lost state from anyone'.

- **Spec:** design.md (Data Storage / Schema): 'The postcard crdt_state blob ... is decoded with a small forward-compat fallback ... This matters most for the *server*, which is authoritative and cannot re-sync its lost state from anyone; an interactive client can fall back to dropping an unreadable blob and re-syncing from the server.'
- **Suggested fix:** Do not swallow the error path. Match `load_state()`: `Ok(Some(s)) => s`, `Ok(None) => fresh epoch-1 state` (genuine first run), `Err(e) => abort startup` (or at minimum log at error and refuse to overwrite, never reset the epoch). Treating a decode/read failure identically to 'no state yet' is the bug.

**Status (2026-06-27): fixed.** The `load_state().ok().flatten()` swallow was replaced by an extracted, documented `initial_snapshot(storage)` helper that matches explicitly: `None`/`Ok(None)` → fresh epoch-1 (genuine first run), `Ok(Some)` → use the stored snapshot, `Err(e)` → **abort**. `server::run` now returns `Result<(), String>` and propagates that error to `main` (where it already wraps DB errors), so on a corrupt/undecodable blob, an unreadable layout, or a SQLite read failure the server **refuses to start** with an actionable message (names the cause and tells the operator to investigate/restore rather than lose state) instead of silently resetting the epoch to 1. The known version-bump path is unaffected — that fallback lives inside `decode_snapshot` and surfaces as `Ok`, so only genuine load failures abort. Regression tests added in `dessplay-rendezvous/src/server.rs` (`initial_snapshot_aborts_on_a_corrupt_blob`, confirmed failing before the fix — the buggy helper returned fresh epoch-1 — plus three good-path tests for no-storage, empty-storage, and a valid blob). The client's `load_state_tolerant` (the only other load site) already fails loud on non-`Codec` errors and is correct (it can re-sync); no sibling swallow elsewhere.

<details><summary>Verification trail — code pointers</summary>

CORE BUG CONFIRMED. server.rs:374-380: `storage.as_ref().and_then(|s| s.load_state().ok().flatten()).unwrap_or(StateSnapshot { epoch: Epoch(1), state: CrdtState::new() })`. `.ok().flatten()` maps both Err and Ok(None) to None, so a load error becomes "fresh server" -> empty epoch-1 substitution. load_state (storage.rs:249-274) returns Err on a SQLite read error (`.optional()?`, line 258) OR a decode failure (`CrdtState::decode_snapshot(&blob)?`, line 263); Ok(None) is the only genuine "no row" case (lines 259-262). decode_snapshot (state.rs:262-269) auto-migrates only the CrdtStateV1 prefix and otherwise "surfaces the *original* error" — so genuine corruption or a downgrade onto a newer-than-V2 blob yields Err, which is then swallowed. CrdtState (state.rs:60-83) holds the never-pruned List (list_entries/list_next_ep), watched flags, playlist, file_catalog, series_relations, chat — so the authoritative node silently loses real, server-unrecoverable state and resets epoch to 1. The asymmetry confirms intent: the client's load_state_tolerant (run.rs:292-301) deliberately drops only Codec errors (safe: it re-syncs) and fails loud on others, whereas the server (which design.md says "cannot re-sync its lost state from anyone") has no fail-loud path. Spec quote is accurate and supports the "fail loud" recommendation. PROPAGATION SUB-CLAIM REFUTED: the assertion that clients "adopt the empty epoch-1 snapshot, propagating the wipe to the whole group" is contradicted by sync.rs:713-717: `if snapshot.epoch < self.epoch() { warn "ignoring stale snapshot"; return; }` — a client at epoch N>1 rejects the empty epoch-1 snapshot and keeps its state. Result is a server/client split-brain (server drops client ops as stale-epoch at server.rs:844-850 and ignores stale-epoch merges at 917-923), not a silent group-wide wipe. Severity corrected high->medium: the real, actionable defect (authoritative silent state loss + epoch reset on a non-recoverable load error; should fail loud) stands, but the group-propagation amplification that would justify "high" is false, the trigger is rare (common version-bump handled by V1 fallback; only fires on genuine corruption / version downgrade / SQLite I/O error — and save_state is a single atomic INSERT at storage.rs:232), and existing clients retain their copies so recovery is possible.

</details>

---

## Server — AniDB integration

**Files:** `dessplay-rendezvous/src/anidb/{worker,client,protocol,record,schedule,titles}.rs`.
**Read first:** design.md → *Parsing files to series/season/episode* (rate limits: ≥2s/packet, 4s+burst-60, 5s on missing; re-validation ladder anchored on `min(first_seen, mtime)`; directory-hint reconciliation; relations walk; titles-dump search).
**Theme:** one bug — the ANIME/relations queue lacks the startup durability reconciliation the FILE queue has, so a restart in the wrong window can permanently orphan a series' relations graph.

### ⚪ LOW · ANIME/relations queue has no durability reconciliation, unlike the FILE queue — a restart can permanently orphan a series' relations

**`dessplay-rendezvous/src/anidb/worker.rs:223-245`** · _bug_

`reconcile_settled_lookups` (worker.rs 223-245, calling `rearm_settled_without_metadata`) repairs the documented durability hole for FILE metadata: a row settled durably in SQLite whose CRDT metadata write was lost to a restart before the periodic snapshot is re-armed. The ANIME/relations path has the exact same structure but no analogous repair. In `lookup_anime` (worker.rs 398-409) the server first calls `write_relations` (volatile — only persisted at the next CRDT snapshot) and then durably settles the queue row with `record_anime_attempt(series, now_i, NEVER)`. If the server restarts in that window, the relations are lost from CRDT state while the `anime_queue` row remains a NEVER tombstone. Because `enqueue_anime` is `INSERT OR IGNORE` (storage.rs 517-524) and `due_anime` never returns a NEVER row, that aid is never looked up again: `wanted_series` keeps re-deriving it (no relations in state) but every re-enqueue is a no-op against the tombstone, and no other path re-arms `anime_queue`. The series stays permanently ungrouped (falls back to grouping by parsed name), so e.g. a sequel shows as its own franchise instead of merging with the rest. It does not self-heal at compaction (compaction clears the lookup-requests GSet, not the SQLite anime queue).

- **Spec:** docs/design.md "Durability reconciliation" (lines 1239-1246): the design only specifies re-arming for the FILE queue (`anidb_queue` rows marked has_data with no metadata). The same restart-window argument applies to ANIME relations (also a durable SQLite settle plus a snapshot-only CRDT write), but neither the spec nor the code covers it. The code side looks wrong (missing reconciliation); the doc is silent on the anime queue.
- **Suggested fix:** Add an anime analogue of `rearm_settled_without_metadata`: at startup, re-arm (set next_attempt = now) any settled `anime_queue` row whose aid is absent from the loaded `series_relations` map, and call it from `reconcile_settled_lookups`. Alternatively, write relations durably to SQLite alongside the queue settle so the CRDT write is no longer the only copy.

<details><summary>Verification trail — code pointers</summary>

The asymmetry and race described are real and independently verified against the code. (1) FILE queue: reconcile_settled_lookups (worker.rs:223-245) is called once at startup (worker.rs:77) and re-arms anidb_queue rows via rearm_settled_without_metadata (storage.rs:481-511). A crate-wide grep shows NO analogous reconcile for anime_queue — its only ops are enqueue_anime, due_anime, record_anime_attempt. (2) The restart window is real: in lookup_anime on a hit (worker.rs:382-410), host.write_relations writes to in-memory CRDT state (server.rs:332-338 server_write, snapshotted only periodically per server.rs:351), while record_anime_attempt(series, now_i, NEVER) (storage.rs:548-561) durably UPDATEs SQLite immediately. A restart between the durable settle and the next snapshot loses relations but keeps the NEVER tombstone. (3) The tombstone is permanent: enqueue_anime is INSERT OR IGNORE (storage.rs:517-524), due_anime filters next_attempt <= now (storage.rs:527-532), so NEVER (=i64::MAX) never re-emits — confirmed by the anime_queue_lifecycle test (storage.rs:1024-1027) which asserts re-enqueue after a NEVER settle stays a no-op. wanted_series (worker.rs:248-261) keeps re-deriving the aid each pass but seed_queues (worker.rs:136-148) re-enqueue is a no-op against the tombstone. (4) Compaction (compact_state, server.rs:1073) operates on CRDT state and never touches anime_queue, so no self-heal. (5) Spec: docs/design.md 'Durability reconciliation' covers only the anidb_queue/has_data FILE case; the symmetric ANIME case is genuinely uncovered. Severity 'low' is correct: impact is degraded franchise grouping in the browser (fallback to parsed-name grouping), often self-mitigated by a neighboring series' bidirectional relation re-merging the component on the client; no effect on playback, sync correctness, or persisted state; narrow trigger window.

</details>

