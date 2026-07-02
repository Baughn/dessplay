# DessPlay Codebase Review — Remediation Report

_Generated 2026-07-01 by a multi-agent audit (22 Opus finder agents, one per
area; every bug/security finding then independently adversarially verified with
a lenient disprove pass). 24 raw findings → **22 kept** (16 confirmed bugs, 6
non-bug/uncertain), 0 refuted._

_Revision: jj change `vstzkyvkxkvl` · commit `19a19da4`. Scope: whole codebase._

<!-- audit-revision
mode: whole
commit: 19a19da42373
jj-change: vstzkyvkxkvl
generated: 2026-07-01
-->

> Working a finding? Per `CLAUDE.md`, add a regression test (property/fuzz
> preferred) that fails *before* the fix, and treat each bug as a chance to
> tighten the architecture or a design-doc detail. Several findings below are
> themselves an invariant-erosion or stale-doc issue as much as a code bug.

## Executive summary

The codebase remains broadly healthy. All but two findings are new since the
2026-06-26 review, which means the prior review's cluster of file-actor and
narrator bugs was genuinely closed — the survivors here are mostly fresh,
lower-severity edges rather than regressions. The two standouts:

- **One real availability defect on the public server (🔴 HIGH).** The
  rendezvous accept loop runs the QUIC handshake and then blocks *inline*
  waiting for the client's control stream, so a single unauthenticated peer
  that completes the handshake and stalls wedges every subsequent join. The
  server's own 10s keep-alive defeats the idle timeout the code comment relies
  on to bound this, so it never self-heals. This is the only finding that a
  stranger with no password can trigger against `dessplay.brage.info`.

- **A file-transfer dead end that can block the whole group (🟠 MEDIUM, two
  findings that chain).** A manually-mapped file is advertised `Ready` but its
  block hashes are never cached, so the serve path silently bails; and the
  download side never re-solicits or snubs a `Ready` source stuck at the
  block-hash stage. Together, a present peer that advertises a file it cannot
  actually serve wedges a downloader permanently — and if that file is
  now-playing and the downloader is a present Watching/Maybe user, the group is
  blocked with no timeout and no recovery short of a new now-playing.

Recurring themes worth noting:

- **"`Ready` implies servable" is not actually enforced.** Findings 4, 5, and 9
  all stem from the servable set (`local_files`, the `Ready` advertisement, and
  the block-hash cache) drifting out of agreement with what this client can
  really hand a peer. The transfer scheduler trusts `Ready` as ground truth;
  three separate paths can advertise `Ready` for something unservable.

- **Backoff schedules that don't back off.** The IRC reconnect loop (1) resets
  its backoff on a channel-join rejection, and the attach-mode re-attach probe
  (8) blocks the actor for the full 10s socket wait per attempt — in both cases
  a capped-backoff design collapses into a tight retry.

- **Narrator/drift edges around the new position file-tag.** Commit `bc4607c`
  added a file-tag filter to the "current" side of the seek diff but not the
  captured "previous" side (10), the first seek of each episode is never
  narrated (11), and a slewing client that loses its position reference never
  releases the slew (21). None corrupts state, but 21 can drag the group ~2%
  off-rate.

### Fix-first order

1. 🔴 **QUIC accept-loop DoS** — `dessplay-core/src/net/quic.rs:319`
   (`QuicListener::accept`). Spawn the handshake+first-stream wait per
   connection so the loop returns to `endpoint.accept()` immediately, and bound
   the control-stream wait with an explicit timeout. Contained but security-
   critical: it is the one unauthenticated defect on the public server.
2. 🟠 **Manual map advertises unservable `Ready`** —
   `dessplay/src/actors/file.rs:855` (`serve_block_hashes` / `set_manual_mapping`).
   Hash a manual mapping when it is set so the block-hash cache is populated.
   This is the concrete trigger for #3.
3. 🟠 **Download wedges at the block-hash stage** —
   `dessplay/src/download.rs:507` (`progress_and_refill` / `snub`). Give the
   Pending block-hash stage a re-solicit/snub timeout so a silent source cannot
   permanently stall a transfer. Can block the whole group.
4. 🟠 **IRC reconnect storm on channel rejection** —
   `dessplay/src/actors/irc.rs:381`. A `Rejected` session-end that surfaces
   Disconnected without resetting backoff. Stops a ~2s Connected/Disconnected
   chat-spam loop.
5. 🟠 **No SQLite `busy_timeout`** — `dessplay/src/storage.rs:350`
   (`Storage::init`). One-line `busy_timeout` so concurrent same-process
   writers wait instead of dropping the write with `SQLITE_BUSY`.
6. 🟠 **Import duplicates a series named on two sheets** —
   `dessplay/src/import.rs:431` (`submit`). Track names created within the run
   so a re-import collapses rather than orphans duplicates.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| Core — Networking & transport (QUIC) | 1 | | | 1 |
| Core — CRDT state & sync broadcast | | | 1 | 1 |
| Core — Playback gating & derived state | | | 1 | 1 |
| Client — File actor: cache, serve, eviction | | 1 | 1 | 2 |
| Client — File transfer & download | | 1 | | 1 |
| Client — IRC bridge | | 1 | 1 | 2 |
| Client — Lifecycle, config & storage | | 1 | 1 | 2 |
| Client — List import | | 1 | 1 | 2 |
| Client — Player actor & mpv | | | 2 | 2 |
| Client — Session: narrator & drift | | | 3 | 3 |
| Client — Sync & network actors | | | 1 | 1 |
| Client — TUI | | | 2 | 2 |
| Server — Rendezvous: presence | | | 1 | 1 |
| Server — AniDB integration | | | 1 | 1 |
| **Total** | **1** | **5** | **16** | **22** |

---

## Core — Networking & transport (QUIC)

**Files:** `dessplay-core/src/net/quic.rs` (QUIC endpoint + accept loop);
`dessplay-rendezvous/src/server.rs` (connection serving).
**Read first:** network-design.md → *Connection Flow*, *TLS and Identity*;
design.md → *Presence*, *Security / Threat Model*.
**Key entry points:** `QuicListener::accept`, `serve_connection`. The invariant
at stake: the accept path must not let one pre-Auth connection deny service to
others, and password auth is application-level (the TLS handshake itself is
unauthenticated by design).
**Theme:** a single availability defect, but the only one a stranger can reach.

### 🔴 HIGH · Idle unauthenticated connection wedges the whole accept loop

**`dessplay-core/src/net/quic.rs:319`** · _security_

`QuicListener::accept` is a serial loop: it drives `incoming.await` (line 319),
then blocks *inline* on `conn.accept_bi().await` (line 331) waiting for the
client's control stream, and only returns to `serve_connection` — which spawns
the per-connection task (`server.rs:562-577`) — after that stream arrives. A
peer that completes the QUIC/TLS handshake but never opens a control stream
parks `accept()` forever, and because the loop spawns nothing until `accept()`
returns, every subsequent connection sits unaccepted behind it. The code
comment assumes the idle timeout bounds this, but the server sets
`keep_alive_interval(10s)` (`quic.rs:43`), so a standard client QUIC stack
auto-ACKs the keep-alive PINGs and `max_idle_timeout` never fires. Since the
server uses `with_no_client_auth()` (`quic.rs:288`) and checks the password
only in the application-level `Auth` message, completing the handshake needs no
secret — this is an **unauthenticated** availability defect against a publicly
reachable server.

- **Spec:** network-design.md *Connection Flow* / design.md *Presence* — the
  accept path must not let one pre-Auth connection deny service to others.
- **Prior:** new.
- **Suggested fix:** in the accept loop, spawn a per-`Incoming` task that awaits
  the handshake *and* the first control stream, so the loop immediately returns
  to `endpoint.accept()`; additionally bound the control-stream wait with an
  explicit `tokio::time::timeout` rather than relying on the
  keep-alive-refreshed idle timeout. Regression test: a fake client that
  completes the handshake and never opens a stream must not prevent a second
  client from connecting (drive it under paused tokio time so the timeout is
  deterministic). Architecturally, this reframes "the idle timeout protects the
  accept path" as false whenever keep-alive is on — worth a one-line note in
  network-design.md.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `quic.rs:311-341` is a serial accept loop: after `incoming.await`
(319) it blocks on `conn.accept_bi().await` (331) inline. `server.rs:562-577`
spawns `serve_connection` only after `accept()` returns, so a blocked `accept()`
stalls all new connections. `with_no_client_auth()` (288) means the handshake
needs no password. `shared_transport_config` sets
`keep_alive_interval(Some(KEEP_ALIVE))` (43); a standard client auto-ACKs those
PINGs, resetting the server's idle timer so `max_idle_timeout` (44-45) never
fires — the comment at 329-330 is wrong. High severity: unauthenticated,
public, no self-heal.

</details>

---

## Core — CRDT state & sync broadcast

**Files:** `dessplay-core/src/state.rs` (op apply + broadcast decision);
`dessplay-rendezvous/src/server.rs` (StateOp handler); `dessplay/src/actors/sync.rs`
(eager reliable+datagram send).
**Read first:** sync-state.md → *Operation Broadcast*, *Dual-Mode Sync*.
**Key entry points:** `apply_for_broadcast`, `apply_if_orderly`. The invariant:
the server deduplicates, applies, and broadcasts — a duplicate delivery of the
same op must not re-broadcast.
**Theme:** one still-open bandwidth bug in the reliable/datagram symmetry.

### ⚪ LOW · `apply_for_broadcast` double-broadcasts a map op delivered datagram-first

**`dessplay-core/src/state.rs:841`** · _bug_

Every ordinary op is sent eager — a reliable control copy *and* a datagram
copy — and the server re-broadcasts whenever `apply_for_broadcast` returns
true. The two map-transport arms are asymmetric: the datagram arm (line 840)
calls `apply_if_orderly` (returns true only when the op is in-sequence/new),
but the reliable arm (841-844) does `{ self.apply(map_op); true }` — returns
true **unconditionally**. When the datagram copy arrives first (common on a
healthy link, and guaranteed for the first op after connect since datagrams
skip stream ordering), the datagram arm applies and broadcasts, then the
reliable copy re-applies as an idempotent no-op but still returns true and
broadcasts the identical op a second time. Convergence is unaffected (LWW map
is idempotent); the cost is doubled relay egress on the bandwidth-constrained
NAS uplink for the whole map-op class (playlist, watched, file availability
including per-progress Downloading, series preference, manual override, List
edits).

- **Spec:** sync-state.md *Operation Broadcast* step 4 — "the server
  deduplicates, applies, and broadcasts to other clients."
- **Prior:** still-open. The 2026-06-26 double-broadcast fix closed the
  reliable-first ordering but left the datagram-first case for map ops.
- **Suggested fix:** make the reliable map arm report whether the apply actually
  advanced the map's per-origin dot clock (reuse `next_in_sequence` as a
  precondition) and return true only when the op was new, mirroring the datagram
  arm. The existing `op_rebroadcast.rs` test only sends control-first on an
  order-free chat op — add a case that sends a *map* op datagram-first and
  asserts a single relay.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `state.rs:840` datagram arm returns `apply_if_orderly`'s result;
`:841-844` reliable arm returns true unconditionally. `send_eager`
(`network.rs:289-301`) sends both copies; `server.rs:952-973` rebroadcasts on
`applied==true` with no other dedup. Datagram-first → two broadcasts. The
order-free arms are change-detected; the reliable map arm is the lone
asymmetric path, contradicting the server's own comment (946-951). Idempotent,
so bandwidth-only — low.

</details>

---

## Core — Playback gating & derived state

**Files:** `dessplay-core/src/derive.rs`.
**Read first:** design.md → *File State*, *Playback Rules*.
**Theme:** the one carried-over deferral, re-reported only to keep the audit
chain honest.

### ⚪ LOW · Downloading unpause enforces only the 20% half of the rule (deferred by decision)

**`dessplay-core/src/derive.rs:131`** · _spec-drift_

`file_block_reason` permits a Downloading user at `progress_bps >= 2_000` (20%)
with no throughput check, while design.md requires *both* "download speed higher
than the file's computed bitrate" *and* ">= 20% downloaded."
`FileAvailability::Downloading { progress_bps }` carries only progress, so the
speed clause is structurally unevaluable from synced state.

- **Spec:** design.md *File State* — "their download speed must be higher than
  the file's computed bitrate, and at least 20% of the file must be downloaded."
- **Prior:** deferred-by-decision. Recorded in the 2026-06-26 review and
  explicitly deferred on 2026-06-28; design.md's *Future Plans* documents this.
  No action expected — listed only for accounting.
- **Suggested fix:** none unless the deferral is revisited. If ever implemented:
  add a downloader-computed "eligible-to-play" (speed ≥ bitrate) signal to
  `FileAvailability::Downloading` and AND it with the 20% threshold. Impact is a
  self-only edge (a user unpausing at exactly 20% below bitrate may stall their
  own playback); it never gates the group.

---

## Client — File actor: cache, serve, eviction

**Files:** `dessplay/src/actors/file.rs`.
**Read first:** design.md → *Download Cache and Retention*, *File Matching* 4a,
*Manual File Mapping*; network-design.md → *File Transfer*.
**Key entry points:** `serve_block_hashes`, `set_manual_mapping`, `resolve`,
`run_eviction`, `lost_local_file`. Invariant at stake: **advertising `Ready`
must mean this client can serve the file** — every `Ready` holder must have its
block hashes cached and its `local_files` entry consistent with disk.
**Theme:** the servable set drifts from what can actually be served.

### 🟠 MEDIUM · A manually-mapped file advertises `Ready` but its block hashes are never cached

**`dessplay/src/actors/file.rs:855`** · _bug_

`set_manual_mapping` (`file.rs:1260-1268`) and the manual-mapping short-circuit
in `resolve` (`file.rs:1139-1151`) both emit `Resolution::Verified` →
`FileAvailability::Ready` for the file, but the mapped path is never hashed:
`commit_fresh_hashes` is reached only from download-complete, ordinary
`resolve`, `hash_add`, and the library scan. A manual path is by design often
*outside* the media roots, so it is never scanned and has no `hash_cache` row.
When a downloader selects this `Ready` holder and sends a `BlockHashRequest`,
`serve_block_hashes` finds the file in `local_files` and on disk but
`hash_cache.get(&path)` misses (`file.rs:855`), logs "asked for block hashes we
haven't cached," and returns nothing. The "`Ready` implies servable" contract
that source selection depends on is violated — if this holder is the only one,
the file is undownloadable group-wide.

- **Spec:** network-design.md *File Transfer* (a `Ready` peer serves its block
  hashes and chunks); design.md *File Matching* 4a (a manual map is a servable
  local copy).
- **Prior:** new.
- **Suggested fix:** when a manual mapping is set or resolved, hash the file
  once to populate `hash_cache` (guarding that the served root matches the
  playlist key) — mirroring the download-complete path. Alternatively mark
  manual mappings as a local-only `Ready` that `download_sources` excludes, so
  peers never solicit an unservable holder. Regression test: a peer manually
  maps a file outside its roots, advertises `Ready`, and a second peer's
  `BlockHashRequest` must receive hashes (not a silent bail).

<details><summary>Verification trail — code pointers</summary>

Confirmed. `set_manual_mapping` (`file.rs:1260-1268`) and resolve's short-circuit
(`file.rs:1139-1148`) emit `Verified` with no hashing; `session.rs:1404` maps
`Verified` → `Ready`. `serve_block_hashes` (`file.rs:855-860`) misses the
`hash_cache` for an unscanned manual path and returns nothing. Ready-implies-
servable violated. Medium.

</details>

### ⚪ LOW · `run_eviction` leaves the deleted file in the in-memory servable map

**`dessplay/src/actors/file.rs:1377`** · _bug_

`run_eviction` (`file.rs:1337-1384`) deletes the cached file and calls
`remove_cache_entry` / `remove_hash_cache` (rebuilding the in-memory hash cache
without the entry), but never calls `self.local_files.remove(&entry.hash)` —
`local_files` is only pruned in `lost_local_file` (line 896). After an eviction
pass, `local_files[h]` still points at the just-deleted path, so the servable
map claims a file that is gone. The synced `FileAvailability` is handled
correctly (`session::note_evicted` retracts `Ready` → Missing), so group-visible
state is right; the gap is purely the in-memory servable map, and it self-heals
because the serve paths guard reads with `path.exists()` and route a vanished
path through `lost_local_file`. Observable effect is a redundant serve attempt +
a Missing re-emit.

- **Spec:** design.md *Download Cache and Retention* — cache bookkeeping is an
  index; a copy that no longer exists must be dropped from the servable set.
- **Prior:** new. (Same *class* as the prior review's file.rs servable-map
  omissions, but a distinct call site.)
- **Suggested fix:** in `run_eviction`, after deleting and pruning, also
  `self.local_files.remove(&entry.hash)`. Regression test: run an eviction pass,
  then drive a `BlockHashRequest` for the evicted hash and assert the actor
  reports it not-held (the existing eviction test never drives a serve request).

<details><summary>Verification trail — code pointers</summary>

Confirmed. `run_eviction` (`file.rs:1364-1379`) prunes disk + `cache_entries` +
`hash_cache` but not `local_files`; only `lost_local_file` (896) removes it.
Serve guards (`enqueue_serve` `contains_key` at 919; `drain_serve_queue`
`path.exists()` at 943-951) self-heal via `lost_local_file`. Group state
corrected by `note_evicted`. Self-only transient — low.

</details>

---

## Client — File transfer & download

**Files:** `dessplay/src/download.rs`.
**Read first:** network-design.md → *File Transfer* (scheduling, snub);
design.md → *Download Cache and Retention*.
**Key entry points:** `progress_and_refill`, `snub`, `set_sources`. Invariant:
a source that fails to supply usable data must eventually be snubbed and
re-solicited/abandoned — no source may permanently wedge a transfer.
**Theme:** the block-hash stage has no timeout, so a silent source wedges forever.

### 🟠 MEDIUM · A source stuck at the Pending block-hash stage is never re-solicited or snubbed

**`dessplay/src/download.rs:507`** · _bug_

Block-hash solicitation is one-shot: `progress_and_refill` emits a
`BlockHashRequest` only when `!src.solicited` (`download.rs:506-514`), and once
`solicited` latches true a source is never re-asked — it is cleared only by
removing the source. While `block_hashes` is `Pending`, `progress_and_refill`
returns before `plan_requests` (`download.rs:517`), so a Pending-stage source
never accrues `in_flight` chunks. `snub` (`download.rs:449`) filters on
`!s.in_flight.is_empty()`, so it never drops a Pending-stage source.
`set_sources`, re-emitted each snapshot, re-adds present sources via `or_insert`
(a no-op for an existing peer), so `solicited` is never reset. The prior
per-request `requested_at` staleness path was removed by the single-source-stall
fix, so the Pending stage now has **no timeout at all**. With a single
Present-but-unresponsive `Ready` source the download stalls forever — and this is
concretely reachable via finding #4 (the unservable manual mapping) or any
serve-side `hash_cache` miss. If the file is now-playing and the downloader is a
present Watching/Maybe user, the whole group is blocked.

- **Spec:** network-design.md scheduling/snub — a source failing to supply
  usable block hashes must not permanently wedge the transfer (the module doc
  itself: "a source that sends nothing for `snub_timeout` is dropped").
- **Prior:** new. (Adjacent to the prior review's fix-first #2 single-source
  stall, but that fix removed the very timeout that would have covered this.)
- **Suggested fix:** track a solicitation timestamp (or reintroduce a
  Pending-stage deadline) and, on tick, snub/re-solicit a source that produced
  neither hashes nor chunks within `snub_timeout` — mirroring the chunk-stage
  snub. Regression test in `transfer.rs`: the sole source silently declines to
  serve block hashes; a later good source must complete the download rather than
  the transfer wedging. Architecturally, this pairs with #4 — fixing the
  manual-map hashing removes the most likely trigger, but the missing timeout is
  the underlying robustness gap and should be closed regardless.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `download.rs:507` solicit is one-shot (`!src.solicited`, reset only by
removing+re-adding); `progress_and_refill` returns at 517-518 while Pending so
`in_flight` stays empty; `snub` (449) requires non-empty `in_flight`;
`set_sources` `or_insert` (261-271) is a no-op for existing peers; `Pending`
(143) has no timestamp. Serve-side `serve_block_hashes` has two silent no-response
bails (`file.rs:841-848`, `855-860`) whose own comment flags the stuck-download
suspicion — proving the trigger is reachable. Medium.

</details>

---

## Client — IRC bridge

**Files:** `dessplay/src/actors/irc.rs`; `dessplay/src/run.rs` (event → chat
system line).
**Read first:** design.md → *IRC Bridge* (Lifecycle, Outbound); architecture.md
→ *IrcActor*.
**Key entry points:** `run_session`, `run_with_connector`, `format_privmsg`.
Invariant: reconnects use *capped* backoff; long lines split to fit 512 bytes.
**Theme:** a rejection that resets backoff, and a CTCP length exemption.

### 🟠 MEDIUM · Channel-join rejection drives a permanent ~2s reconnect loop

**`dessplay/src/actors/irc.rs:381`** · _bug_

`run_session`'s `001` arm sets `registered=true`, sends `JOIN`, and emits
`IrcEvent::Connected` (`irc.rs:351-357`) **before** the JOIN is accepted. A
channel-join-failure numeric (473/474/475/471/477) then returns
`SessionEnd::Lost { registered: true }` (`irc.rs:381-385`); the `registered`
branch (`irc.rs:183-192`) treats this as a transient drop, emits `Disconnected`,
and **resets** backoff to `INITIAL_BACKOFF` (2s). The next cycle reconnects,
re-registers, re-JOINs, is re-rejected, and resets backoff again — a
self-sustaining ~2s loop that never engages the capped exponential backoff the
spec requires. `run.rs:1209-1220` turns each event into a local chat system
line, so the pane accumulates a misleading Connected+Disconnected pair every
~2s for the whole session. The post-registration `ERROR` arm (`irc.rs:376-378`)
shares the defect. The +R (477) case is realistic: the bridge connects as
unauthenticated `[Username]Dess` and never touches NickServ.

- **Spec:** design.md *IRC Bridge* Lifecycle / architecture.md *IrcActor* —
  "reconnects with capped backoff."
- **Prior:** new.
- **Suggested fix:** add a `SessionEnd::Rejected { reason }` variant for the
  473/474/475/471/477 and post-`001` `ERROR` cases that surfaces `Disconnected`
  but does **not** reset backoff (or gives up until a `Reconfigure`); consider
  not emitting `Connected` until the JOIN is confirmed (RPL_ENDOFNAMES/366 or a
  JOIN echo). Regression test: a connector that always answers JOIN with 477
  must produce exponentially-growing reconnect delays, not a fixed 2s cadence.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `irc.rs:351-357` emits `Connected` before JOIN confirmation;
`:381-385` returns `Lost { registered: true }` (registered already true from
001); `:183-192` unconditionally resets `backoff = INITIAL_BACKOFF`, wiping the
`grow_backoff` (212) each cycle. `run.rs:1209-1220` → a system line per event.
Post-registration `ERROR` (376-378) shares the path. +R realistic. Medium
(chat spam + tight loop, no data loss).

</details>

### ⚪ LOW · An over-long `/me` CTCP action is never split and gets truncated

**`dessplay/src/actors/irc.rs:568`** · _spec-drift_

`format_privmsg` splits ordinary text into ≤`MAX_PRIVMSG_TEXT` (400-byte)
chunks, but the condition `segment.starts_with('\u{1}') || segment.len() <=
MAX_PRIVMSG_TEXT` (`irc.rs:568`) exempts any CTCP-wrapped segment from
length-based splitting. A `\x01ACTION …\x01` payload is emitted as a single
PRIVMSG regardless of length; a very long `/me` exceeds 512 bytes, the server
truncates it, and the trailing `\x01` is cut — so receiving bridges see a broken
unterminated CTCP ACTION.

- **Spec:** design.md *IRC Bridge* Outbound — "Long lines are split to fit IRC's
  512-byte limit; newlines become separate messages" (no CTCP exception stated).
- **Prior:** new.
- **Suggested fix:** for an over-long CTCP ACTION, split the inner phrase
  between the `\x01` markers and re-wrap each chunk as its own
  `\x01ACTION <chunk>\x01`; or explicitly document that CTCP actions are capped
  at one message. Marked uncertain only in the sense that the disprove pass does
  not run on non-bug findings — the code path itself is unambiguous.

---

## Client — Lifecycle, config & storage

**Files:** `dessplay/src/storage.rs` (SQLite init); `dessplay/src/run.rs`
(address parsing, connection wiring).
**Read first:** design.md → *Data Storage*, *Schema*; architecture.md →
*Composition Root*.
**Key entry points:** `Storage::init`, `with_default_port`. Invariant: several
same-process tokio tasks each hold their own write connection to one DB.
**Theme:** a write-contention gap and an IPv6 address-formatting edge.

### 🟠 MEDIUM · No `busy_timeout` — concurrent same-process writers drop writes on `SQLITE_BUSY`

**`dessplay/src/storage.rs:350`** · _bug_

`Storage::init` (`storage.rs:350-355`) opens each connection with WAL +
`synchronous=NORMAL` but never sets `busy_timeout` (rusqlite default 0).
`run_interactive` opens several independent write connections to the same
`dessplay.db` — sync actor (`run.rs:416`), file actor (`run.rs:747`/`788`),
session (`run.rs:830`), settings reopen (`run.rs:1066`) — all distinct tokio
tasks on the multi-threaded runtime. In WAL mode only one writer holds the write
lock; with `busy_timeout=0` a second concurrent write transaction returns
`SQLITE_BUSY` immediately instead of waiting. The file actor's
`hash_cache`/`cache_entries`/`record_watched` writes (`file.rs:563`/`806`/`1128`)
each merely log and move on, so a collision with the sync actor's 30s
`save_state` flush silently drops the write. The `run.rs` comment ("WAL is fine
with two connections") is misleading — WAL removes reader/writer, not
writer/writer, contention.

- **Spec:** design.md *Data Storage* — the client persists settings, CRDT
  snapshot, watch history, and cache bookkeeping in one SQLite DB.
- **Prior:** new.
- **Suggested fix:** set `conn.busy_timeout(Duration::from_secs(5))` (PRAGMA
  `busy_timeout=5000`) in `Storage::init` so a contended writer waits and
  retries. Regression test: two connections issue overlapping write transactions
  and both must succeed. Note the verifier found one mitigating detail —
  `save_state` does *not* clear `self.dirty` on `Err`, so a busy snapshot
  retries on the next 30s flush; but the file-actor writes are genuinely dropped
  with no retry (e.g. a dropped `record_watched` row means a series is never
  marked "known," so a later missing episode blocks instead of resolving to
  Not-Watching).

<details><summary>Verification trail — code pointers</summary>

Confirmed. `storage.rs:350-355` sets WAL + `synchronous=NORMAL`, no
`busy_timeout`. Concurrent write connections at `run.rs:416`/`747`/`788`/`1066`,
distinct tasks. `file.rs:563`/`806`/`1128` log-and-drop. `sync.rs:425-433`
`save_state` self-retries via the dirty flag; the file-actor writes do not.
Medium: real robustness bug, low per-collision probability, highest-value write
self-retries.

</details>

### ⚪ LOW · `with_default_port` double-brackets a bracketed IPv6 literal given without a port

**`dessplay/src/run.rs:192`** · _bug_

For `server = "[::1]"` (bracketed IPv6, no port) the colon count is 2, so the
match falls to the `_` arm, which requires `starts_with('[') && contains("]:")`.
`"[::1]"` has no `"]:"` substring, so `has_port` is false; the function then
hits `else if server.contains(':')` and wraps the already-bracketed string again
as `[{server}]:PORT`, yielding `"[[::1]]:9876"`. `lookup_host` then fails to
resolve it. Bare host and host:port forms are handled correctly; only the
bracketed-without-port form is mishandled.

- **Prior:** new.
- **Suggested fix:** detect an already-bracketed literal — if
  `server.starts_with('[') && server.ends_with(']')`, append `:PORT` directly,
  reserving `[{server}]:PORT` for un-bracketed multi-colon literals. The
  existing tests cover `::1` and `[::1]:7000` but not `[::1]` without a port —
  add that case.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `with_default_port("[::1]")`: colon count 2 → `_` arm; `starts_with('[')`
true but `contains("]:")` false → `has_port=false`; then `else if
server.contains(':')` → `"[[::1]]:9876"`. Low: uncommon config.

</details>

---

## Client — List import

**Files:** `dessplay/src/import.rs`.
**Read first:** design.md → *Import*, *The List (Series Tracker)*.
**Key entry points:** `submit`, `regex_lite`. Invariant: re-imports don't
duplicate The List; drop detection matches `/abandon|drop/i`.
**Theme:** a dedup snapshot that goes stale mid-run, and a dead regex constant.

### 🟠 MEDIUM · A series named on two sheets becomes two List entries, and re-import never collapses them

**`dessplay/src/import.rs:431`** · _bug_

`find_existing` closes over `existing`, a single `StateView` snapshot captured
once (line 430) before the submit loop (441-474). Each report entry is matched
only against that frozen snapshot — never against entries created earlier in the
same run. Two `ImportedEntry`s sharing a name (case-insensitive) each get a
fresh random `ListEntryId`, producing two `list_entries` rows for one series
(with conflicting statuses from different sheets). On re-import both same-named
entries resolve via `.find()` to whichever duplicate id sorts first, so both
`PutListEntry` mutations target that id and the other duplicate is orphaned
forever — contradicting the doc comment "re-imports don't duplicate The List."
No collision warning is emitted. This hits the group's real data (Dr. Stone:
Passing + Ebony; Steins;Gate: Ivory + Passing; etc.).

- **Spec:** design.md *The List*; `import.rs submit` doc — "re-imports don't
  duplicate The List."
- **Prior:** new.
- **Suggested fix:** track names created during this submit in a local
  lowercased-name map (seeded from `existing`); the create branch checks that
  map so a second same-named entry updates the just-created id, and emit a
  warning when two rows would collapse so the status conflict is surfaced (the
  importer already prints an "unsure" summary — this belongs there). Regression
  test: import two sheets naming the same series; assert one entry results and a
  re-import updates it in place.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `import.rs:430` captures `existing` once; `find_existing` (431-437)
consults only that snapshot; the loop (441-454) never refreshes it, so the
create branch assigns a fresh random id (452). `report.entries` is built
one-per-row across sheets (374) with no dedup. Re-import writes both same-named
rows to whichever id `.find()` hits first, orphaning the other. Contradicts the
doc claim (384-385). Medium.

</details>

### ⚪ LOW · `regex_lite` ignores its pattern argument, making `DROP_MARKER` a dead decoy

**`dessplay/src/import.rs:497`** · _quality_

`DROP_MARKER` is declared `r"(?i)abandon|drop"` (line 195) and passed to
`regex_lite(DROP_MARKER)` (215), but `regex_lite` takes `_pattern: &str` and
never reads it — it hardcodes `lower.contains("abandon") || lower.contains("drop")`
(497-502). The constant is a decoy: a maintainer updating `DROP_MARKER` changes
nothing, and the `(?i)` syntax implies a real regex engine when none exists — a
maintenance trap on the drop-detection path the spec (`/abandon|drop/i`) depends
on.

- **Spec:** design.md *Import* — "a field matches `/abandon|drop/i`."
- **Prior:** new.
- **Suggested fix:** either drop `DROP_MARKER` and rename the helper to make the
  hardcoded terms explicit (`is_drop_marker`), or derive the match terms from the
  constant so the two cannot drift.

---

## Client — Player actor & mpv

**Files:** `dessplay/src/actors/player.rs`; `dessplay/src/player/mpv.rs`.
**Read first:** design.md → *Player Integration* (Attach mode, Crash handling),
*Events from Player*; architecture.md → *PlayerActor*.
**Key entry points:** `try_reattach`, `PlayerCommand::Load`, `handle_player_death`,
`seek_programmatic`. Invariant: echo bookkeeping tracks what *we* commanded and
must be invalidated when the player's file/position changes; re-attach waits with
capped backoff without blocking the actor.
**Theme:** two state-hygiene gaps around load/re-attach.

### ⚪ LOW · Attach-mode re-attach probe blocks the whole actor for up to 10s per attempt

**`dessplay/src/actors/player.rs:731`** · _bug_

`try_reattach()` calls `self.factory.spawn().await` inline inside the reattach
`select!` arm. For an attach-mode factory this is `spawn` →
`MpvPlayer::attach(socket)` → `wait_for_socket(socket)`, which loops connect
attempts against a hard `SOCKET_WAIT=10s` deadline (`mpv.rs:45`, `197-213`).
While that future is awaited the `run()` `select!` loop is parked in that arm
and cannot service any other: `Shutdown`, `Load`, `SyncTo`, `ClockOffset`,
`ShowOsd`, and the cadence tick are all blocked. So while the user's mpv is
down, every re-attach attempt hangs the actor ~10s, and the intended
`REATTACH_BACKOFF_INITIAL`(500ms) → `MAX`(10s) schedule is meaningless since
each probe already costs ~10s. A `/quit` or a `Load` issued during the wait sits
unread until the probe returns.

- **Spec:** design.md *Player Integration* / Attach mode — "if that mpv dies, the
  relaunch path re-attaches, waiting for it to come back" (implying non-blocking
  capped-backoff waiting; the comment claims commands "still flow" while waiting,
  which holds only *between* probes).
- **Prior:** new.
- **Suggested fix:** in attach mode the re-attach probe should be a single quick
  connect attempt (short-timeout `UnixStream::connect`) governed by the actor's
  own `reattach_backoff`, or wrap `try_reattach`'s spawn in a short
  `tokio::time::timeout` so a down socket cannot park the select loop for the
  full `SOCKET_WAIT`. Regression test: with a factory whose socket stays down,
  a `Shutdown` delivered during a re-attach wait must be serviced promptly.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `try_reattach` (`player.rs:729-731`) awaits `factory.spawn()` inline
in the reattach arm (310-314); attach `spawn` → `wait_for_socket` (`mpv.rs:110`,
197-213) loops against `SOCKET_WAIT=10s`. `tokio::select!` runs a resolved
branch's body to completion before re-polling, so the ~10s await blocks all
other commands. Comment at 307-309 holds only between probes. Low.

</details>

### ⚪ LOW · `PlayerCommand::Load` leaves echo counters stale, so a leftover seek echo swallows the user's next real seek

**`dessplay/src/actors/player.rs:377`** · _bug_

The `Load` handler (`player.rs:371-411`) resets `eof_reported`,
`restore_millis`, `pending_user_seek`, `estimate`, `believed_pause`, and
`speed`, but does **not** clear `pending_pause_echoes` or `pending_seek_echoes`.
`handle_player_death` (692-698) clears both, because a gone player invalidates
outstanding echo bookkeeping — and a `Load` likewise replaces the player's
file/position, so any echo awaited from the previous file's commands is
meaningless. The damaging leak is `pending_seek_echoes`: if it survives a `Load`
> 0, the next genuine user `Seeked` event is decremented and swallowed as an
echo (532-537), so the user's seek is silently dropped instead of surfacing
`UserSeeked`. Since `session.rs:1497` writes `SetSeekAuthority` +
`SetPlaybackPosition` on `UserSeeked`, the group never seeks and drift correction
hard-seeks this client back — the scrub appears to do nothing.

- **Spec:** design.md *Events from Player* — the player actor tracks what it
  commanded and swallows matching observations as echoes.
- **Prior:** new.
- **Suggested fix:** in the `Load` arm, clear echo bookkeeping like
  `handle_player_death` — `pending_pause_echoes.clear()` and
  `pending_seek_echoes = 0` — since a `loadfile` invalidates outstanding
  commanded-pause/seek echoes. Regression test: issue a programmatic seek
  (increment the counter), then a `Load`, then a user `Seeked`, and assert
  `UserSeeked` is emitted (not swallowed).

<details><summary>Verification trail — code pointers</summary>

Confirmed. `Load` (371-411) resets six fields but never touches the echo
counters; `handle_player_death` (694-698) clears both, confirming maintainers
treat them as invalid when the file/position changes. `seek_programmatic`
(471-477) increments; `Seeked` handler (532-537) decrements and swallows one
event per count. Timing-dependent (needs an outstanding programmatic-seek echo
at Load time) — low.

</details>

---

## Client — Session: narrator & drift

**Files:** `dessplay/src/session.rs`.
**Read first:** design.md → *System Messages* (Seek row, the same-lines
invariant), *Playback Rules* (Drift correction).
**Key entry points:** `NarratorState::capture`, `current_seek_sample`, the
seek-narration block, `position_reference`, the `on_state` drift path. Invariant:
every client diffs the same synced inputs and narrates the same lines; slew must
release back to 1.0 once converged or unreferenced.
**Theme:** three edges introduced or exposed by the position file-tag work.

### ⚪ LOW · A slewing client that loses its position reference never releases the slew

**`dessplay/src/session.rs:1333`** · _bug_

Drift correction is purely reactive: speed resets to 1.0 only inside
`drift_correct` (`player.rs:498` converged band, 510 hard-seek) or on a fresh
`Load` (381). `drift_correct` runs only when the session emits
`PlayerCommand::SyncTo`, which happens only when
`following = position_reference(...)` is `Some` (`session.rs:1328-1343`). When
`following` becomes `None`, `on_state` emits no `SyncTo` and nothing releases the
slew — there is no periodic self-drift check in the player loop. So a client
mid-slew (speed 1.02/0.98) that loses its reference stays slewed. Two triggers:
(a) the peer it chased departs/pauses, leaving this client the furthest-ahead
same-file peer, so `position_reference` returns `None` (`leader.0 == self.me`,
1127); (b) this client seeks and becomes seek authority, so `position_reference`
returns `None` (1096-1098). In case (a) the stuck-fast client is now the leader
everyone follows, so laggards slew up to its 1.02×-advancing reported position
and the **whole group runs ~2% fast** until the next `Load` or seek.

- **Spec:** design.md *Playback Rules* / Drift correction — "100ms–3s: slew …
  until converged."
- **Prior:** new.
- **Suggested fix:** in `on_state`, when `showing_now_playing` is true but
  `following` is `None`, command the player back to normal rate (a "release
  slew" directive that calls `set_speed(1.0)`); or have the player actor reset
  speed to 1.0 after N cadence ticks with no `SyncTo`. Regression test: a
  three-client sim where the leader departs mid-slew and the new furthest-ahead
  client must return to 1.0× rather than dragging the group. This is the one
  narrator/drift finding that affects the *group*, so rank it above the two
  cosmetic ones below.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `set_speed(1.0)` appears only in `drift_correct` (498/510), `Load`
(381), and crash/reattach (719/796/802) — none periodic. `drift_correct` is
reached only via `SyncTo` (417-422), emitted only when `following` is `Some`
(1333-1343). `position_reference` returns `None` when self is leader (1127) or
authority (1096-1098). `cadence.tick()` only emits position (299-300), never
touches speed. Slew persists until next Load/hard-seek. Low (silent,
pitch-corrected, needs the reference to drop mid-slew).

</details>

### ⚪ LOW · Narrator captures the previous seek sample unfiltered by file tag

**`dessplay/src/session.rs:363`** · _bug_

Commit `bc4607c` added a `p.file == now_playing` filter to `current_seek_sample`
(`session.rs:386-392`, the "current" side of the seek diff) but left
`NarratorState::capture`'s `seek_sample` (363-369) **unfiltered** — it stores
the authority's `playback_position` regardless of file tag. The seek-narration
block (556-571) compares the filtered current against the unfiltered
`prev.seek_sample`. When the authority's position register still holds a
previous file's sample at prev-capture (the exact lag the file tag defends
against), `prev.seek_sample` carries a stale wrong-file position; once the tag
catches up, `expected = stale_prev + elapsed` and a >5s diff narrates a false
"{authority} skipped to …". Because the lag is timing-dependent, one client can
narrate the phantom seek while another does not, eroding the same-lines
invariant.

- **Spec:** design.md *System Messages* — "every client diffs the same synced
  inputs, every client narrates the same lines."
- **Prior:** new.
- **Suggested fix:** mirror `current_seek_sample` in `capture` — filter the
  authority's `PlaybackPosition` by `p.file == view.now_playing` before storing
  `prev.seek_sample`. Regression test: a captured prev sample tagged for the old
  file must not produce a "skipped to" line after a now-playing transition.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `capture` (363-369) stores `seek_sample` with no file-tag filter;
`current_seek_sample` (386-392) filters. The diff block (556-571) compares
filtered-current vs unfiltered-prev; `prev.seek_sample` is used nowhere else.
`bc4607c` guarded only the current side. Timing-dependent transient, stray local
line — low.

</details>

### ⚪ LOW · The first seek of each episode is never narrated

**`dessplay/src/session.rs:556`** · _bug_

The seek-narration block fires only when *both* `current_seek_sample` and
`prev.seek_sample` are `Some` (558-559). `capture` records a non-`None`
`seek_sample` only when authority is already a `User` (363-369) — under `Server`
authority it is `None`. The server resets seek authority to `Server` on every
EOF-advance and manual now-playing change, so the first user seek transitions
`Server → User`: `prev.seek_sample` is `None`, the `(Some, Some)` pattern fails,
and no line is emitted. The block's own comment (552-555) says "a fresh sample on
an unchanged file is itself a jump candidate," implying the first seek *should*
narrate; only the second and later seeks in an episode do.

- **Spec:** design.md *System Messages*, Seek row — "(> 5s jump) → 'Baughn
  skipped to 12:34'."
- **Prior:** new.
- **Suggested fix:** when authority is a `User` and `prev.seek_sample` is `None`
  (just flipped from `Server` via this seek), treat the fresh sample as a jump
  candidate — narrate if the new position is > `SEEK_NARRATE_MILLIS` from the
  expected pre-seek baseline. Cosmetic/local — lowest priority of the three.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `capture` (363-369) yields `None` under Server authority; the block
(556-560) requires `(Some, Some)`. Server resets authority on every
EOF/manual-select, so the first user seek is a Server→User transition with
`prev` `None`. Cosmetic, local — low.

</details>

---

## Client — Sync & network actors

**Files:** `dessplay/src/actors/sync.rs`.
**Read first:** sync-state.md → *Epochs*, *Dual-Mode Sync*; architecture.md →
*SyncActor*.
**Key entry points:** `synced`, the `StateSnapshot` handler. Invariant: an
epoch mismatch on reconnect adopts the server snapshot, discards stale local
state, and replays the offline buffer.
**Theme:** the high-risk epoch-adoption path is untested.

### ⚪ LOW · The epoch-adoption-via-snapshot reconnect path has no test coverage

**`dessplay/src/actors/sync.rs:646`** · _test-gap_

`sync.rs:618-620` documents that reconnect replay is restorative on the snapshot
path: a `StateSnapshot` with a newer epoch is adopted wholesale
(`self.state = snapshot.state`, line 732), which must discard stale local state,
then `synced()` (646-651) replays the offline buffer and pushes an upward merge.
In production this runs at `Link::AwaitingSync`. But the only `StateSnapshot`
test, `snapshot_adoption_updates_epoch` (1167), sends the snapshot to a fresh
actor that was never `Connected`, so link is `Link::Down` and `synced()` returns
at its Down guard (630-634) — the offline-buffer replay, upward-merge push, and
"discard stale local state" behaviors are never exercised on the snapshot path.
`offline_buffer_replays_with_positions_coalesced` (943) covers only the
same-epoch `StateMerge` path. This is the reconnection/epoch invariant
`CLAUDE.md` flags as high-risk.

- **Spec:** sync-state.md *Epochs* — epoch mismatch on reconnect must adopt the
  server snapshot and discard stale local state; unsent ops are memory-only and
  replayed.
- **Prior:** new.
- **Suggested fix:** add a test that seeds local state + offline buffer, sends
  `Connected` (→ `AwaitingSync`), delivers a higher-epoch `StateSnapshot`
  lacking the pre-existing entries, and asserts the stale entries are gone, the
  offline ops appear in the pushed upward `StateMerge`, the epoch advanced, and
  gates opened. A future edit reordering the snapshot handler or the Down guard
  would otherwise silently drop unsent offline edits on a post-compaction
  reconnect with no test failing.

---

## Client — TUI

**Files:** `dessplay/src/ui/app.rs`.
**Read first:** ui-architecture.md → *State to Props Mapping*, *Modals*;
design.md → *Acknowledge a committed-absent blocker*; `CLAUDE.md` Testing.
**Key entry points:** `draw_hash_overlay`, `acknowledge_blockers`.
**Theme:** a u16 overflow of the same class the prior review fixed elsewhere, and
a missing test on a high-risk command.

### ⚪ LOW · `draw_hash_overlay` overflows u16 on extremely wide terminals

**`dessplay/src/ui/app.rs:1366`** · _bug_

The overlay width is `(area.width * 3 / 5).clamp(...)` (line 1366). ratatui
`Rect` fields are `u16`, so `area.width * 3` is a u16 multiply that overflows
once `area.width >= 21846`. The `.clamp` runs *after* the multiply, so it does
not prevent overflow. In debug builds (overflow-checks on) this panics on the
overlay render path when a playlist-add hash is in flight; release silently
wraps to a garbage rect. This is the identical pattern the prior review fixed for
`overlay()` in `modals.rs` (widened to u32) — this `app.rs` call site was not
covered.

- **Prior:** new. (Same class as a prior-review fix; the fix did not sweep this
  call site.)
- **Suggested fix:** widen intermediates to u32 before the divide/clamp,
  mirroring the `modals.rs` fix; add a wide-terminal regression test (a
  `TestBackend` of width ≥ 21846 with `self.hashing` non-empty). Reachability
  requires a ~21846-column terminal, hence low.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `app.rs:1366` `area.width * 3` is a u16 multiply overflowing at
≥ 21846; `.clamp` runs after. `modals.rs::overlay` (48-65) was fixed via
`u32::from`; this site was missed. Low (needs an enormous terminal).

</details>

### ⚪ LOW · The `/ack` command (`acknowledge_blockers`) has no test

**`dessplay/src/ui/app.rs:1193`** · _test-gap_

`acknowledge_blockers()` reads now-playing, runs `derive::playback_blockers`
over `self.snapshot.peers`, filters to `BlockReason::CommittedAbsent`, emits one
`Mutation::AcknowledgeAbsent { file, user }` per blocker plus
`SetPlaybackIntent::Playing`, and returns a `Notice` on the empty paths. Every
other chat command in `app.rs` has a dedicated test, but `/ack` has none —
neither the happy path (correct `AcknowledgeAbsent` set + `Playing` latch, scoped
to now-playing) nor the two `Notice` branches. A regression that dropped the
`CommittedAbsent` filter, mis-scoped the file, or forgot the `Playing` latch
would pass CI. `derive::playback_blockers` is tested in `dessplay-core`, but the
UI wiring (filter + file scoping + intent latch) is not.

- **Spec:** `CLAUDE.md` Testing — "High-risk areas get extra coverage" (per-file
  committed-absent acknowledgement is exactly such an area).
- **Prior:** new.
- **Suggested fix:** add a unit test with a now-playing file whose series a
  Departed/Lost peer is committed to, asserting `/ack` emits
  `AcknowledgeAbsent { file: <now-playing>, user: <peer> }` and
  `SetPlaybackIntent::Playing`; add sibling tests for the two `Notice` branches.

---

## Server — Rendezvous: presence

**Files:** `dessplay-rendezvous/src/server.rs`.
**Read first:** design.md → *Presence* (Departed), *Playback Rules* (forced
Paused on Lost/departure); network-design.md → *Presence*.
**Key entry points:** `sweep_departed`, `force_pause`, the `AuthedEnd::Lost` arm,
`stamp`. Invariant: the server forces intent Paused on Lost and on
graceful-quit departure; the Lamport stamp is monotonic.
**Theme:** a redundant force-pause at the 60s mark that overrides a legitimate
resume.

### ⚪ LOW · The Lost→Departed sweep re-forces Paused, overriding a resume from the present users

**`dessplay-rendezvous/src/server.rs:645`** · _bug_

On a dying connection the `AuthedEnd::Lost` arm already calls `force_pause()`
for an interactive peer (`server.rs:819-825`). 30s later `sweep_departed`
promotes that peer Lost→Departed and calls `force_pause()` a **second time,
unconditionally**, for every interactive departed peer (642-648). This is not
idempotent from the group's view: in the interval between Lost and Departed the
remaining present users can legally resume when the absent user is non-blocking
(absent Maybe/NotWatching/Away per Playback Rules), writing intent=Playing. The
Departed sweep then writes intent=Paused with a strictly-later Lamport stamp,
which wins the LWW and silently re-pauses the group. The graceful-quit path
(809-818) legitimately needs the departure force-pause (it never went through
Lost); the timeout ladder does not.

- **Spec:** design.md *Presence* (Departed) — "Playback stays paused … No
  auto-unpause" vs *Playback Rules* — forced Paused "on Lost … and on
  departure." The wording partially condones the departure force-pause, which is
  why this is low, but the *timeout-ladder* departure double-applies it.
- **Prior:** new.
- **Suggested fix:** skip `force_pause()` in `sweep_departed` for a peer that was
  already Lost (the Lost transition already paused; Departed only changes
  gating), while keeping `force_pause()` on the graceful-quit immediate-departure
  path. Reconcile the design wording so "force Paused on departure" is scoped to
  the immediate/graceful-quit departure. Regression test: a present user resumes
  during the 30s Lost window of a non-blocking absent peer; the 60s Departed
  sweep must not re-pause them.

<details><summary>Verification trail — code pointers</summary>

Confirmed. `sweep_departed` (619-648) only promotes peers already in
`Presence::Lost` (627), each already force-paused during the Lost arm (822-824),
then calls `force_pause()` again unconditionally (644-645). `stamp` (199-207) is
Lamport-monotonic, so the later write dominates a present user's intent=Playing.
Absent Maybe/NotWatching/Away are non-blocking, and Lost→Departed changes nothing
about the absent user's gating. Narrow window, self-corrects with another play
press — low.

</details>

---

## Server — AniDB integration

**Files:** `dessplay-rendezvous/src/anidb/worker.rs`;
`dessplay-rendezvous/src/server.rs` (`view()`).
**Read first:** design.md → *Parsing files to series/season/episode* (Lookup
flow), *Seeder Behavior* (terabyte scale).
**Key entry points:** the worker main loop, `seed_queues`, `populate_catalog`,
`apply_series_hints`, `host.view()`.
**Theme:** per-pass redundant full-state work that does not scale to a seeder
library.

### ⚪ LOW · Each worker pass re-clones the full CRDT state and re-upserts the entire grow-only lookup set

**`dessplay-rendezvous/src/anidb/worker.rs:136`** · _architecture_

The main loop (`worker.rs:81-112`) calls `seed_queues` (136),
`populate_catalog` (155), and `apply_series_hints` (183) every iteration, each
calling `host.view()` (137/156/187) — which for the real server locks the state
mutex and clones the *entire* CRDT state (`server.rs:369-371`). `seed_queues`
iterates all of `view.lookup_requests` and issues an `enqueue_lookup` upsert per
entry (141-142) — but the GSet is grow-only and only cleared at daily
compaction, so already-processed entries are re-upserted (a no-op) every pass.
During active draining the loop is paced by the AniDB rate limiter (~1 send/2s),
so this O(|lookup_requests|) upsert + three full-state clones runs about every
2s. A seeder is designed for terabyte libraries feeding every indexed hash into
`lookup_requests`, so after a large index this is tens of thousands of redundant
upserts every ~2s (up to 24h until compaction), each under the server's hot
state lock. Convergence is unaffected — pure wasted work and state-lock
contention.

- **Spec:** design.md *Seeder Behavior* — "a seeder may hold terabytes" (the
  scale this path does not accommodate).
- **Prior:** new.
- **Suggested fix:** only seed/populate/reconcile when the input changed — track
  the last-processed `lookup_requests` size/version and skip the re-seed when
  nothing new arrived, or drain requests once and remove processed hashes. At
  minimum, resolve the `StateView` **once** per pass and share it across the
  three calls rather than cloning the full state three times.

---

## Appendix — run notes

- The audit ran as 22 area finders (18 subsystem + 4 cross-cutting), covering
  all 106 tracked Rust source files across `dessplay-core`, `dessplay`, and
  `dessplay-rendezvous`, over commit `19a19da4`.
- The first two workflow attempts were largely lost to a transient Anthropic
  server-side rate limit (not a usage cap) that failed 16–18 of the concurrent
  Opus finders; the run was completed by batching the finder/verifier fan-out to
  4-wide, which eliminated the failures. All 22 areas ultimately produced
  results (a handful returned no findings — expected for the smaller/cleaner
  areas such as core-hash, core-franchise, and several UI/echo/transfer passes).
- Every bug/security finding above was independently adversarially verified with
  a lenient bias (kept unless concretely disproven); 0 were refuted. Non-bug
  findings (spec-drift, test-gap, architecture, quality) skip the disprove pass —
  their trail is the finder's own reasoning and is flagged as such.
