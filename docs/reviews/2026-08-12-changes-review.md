# DessPlay Codebase Review — Remediation Report

_Generated 2026-08-12 by a multi-agent audit (8 Opus finder agents, one per
area; every bug/security finding then independently adversarially verified).
41 raw findings → 36 kept (23 confirmed, 13 uncertain), 0 refuted._

_Revision: jj change `tzmtzlplmosq` · commit `0c48f0b36682`. Scope: changes
since `350cec3b8c64` (the 2026-07-19 scoped review)._

<!-- audit-revision
mode: scoped
commit: 0c48f0b36682076e5e06c0a4567b00b36d8107b0
jj-change: tzmtzlplmosq
base: 350cec3b8c64
generated: 2026-08-12
-->

> Project norms (CLAUDE.md): write the failing regression test **before** the
> fix — property/fuzz tests preferred — unless the fix makes the bug class
> unrepresentable; every bug is an invitation to improve the architecture.
> `cargo fmt` before committing; verify with `cargo test --workspace
> --all-targets`.

## Executive summary

This window (~18k inserted lines: the transfer flow-control overhaul,
partial-file playback, the AI commentary engine, spoiler tags, the tagged
snapshot envelope, the drift controller, mouse support, the torrent
browse-only refactor) is in decent shape at the feature level — the verifiers
refuted nothing, but also confirmed every bug they checked. The standouts:

- **Partial-file playback trusts `loaded` too much.** Two HIGH findings share
  one root cause: `PlayerWiring::loaded` is a bare `Ed2kHash`, so the session
  cannot tell "the sparse partial is loaded" from "the verified copy is
  loaded". A partial can emit a spurious mpv EOF that **marks the episode
  watched and advances now-playing for the whole group** (fix-first 1), and an
  adopted local copy never reloads the player, which keeps playing a deleted
  partial while advertising Ready (fix-first 2). Making `loaded` carry its
  path (and partiality) kills the class.
- **Fire-and-forget stream opens.** The per-transfer-stream redesign left
  `OpenTransferStream` droppable with no retry and no failure event: one
  30-second wifi blip can permanently wedge a download that is gating the
  group (fix-first 3). Several neighbours in the transfer region (the
  stream-limit stall, dead-stream-not-a-snub, the generation-less stream
  keys) are the same theme: the file actor assumes the network actor always
  answers, and the network actor is allowed to stay silent.
- **One regression.** The prior review's "torrent completion clobbers the peer
  download" fix did not survive the browse-only rewrite: a completed Nyaa
  import still never cancels the in-flight peer download of the same hash
  (fix-first 4), re-introducing Ready→Downloading flapping that gates the
  group. The companion guard (the media-root-copy check) is also still
  missing — see the torrent section's still-open finding.
- **The snapshot-envelope coupling already bricked the server once.** v7→v9
  bumped `PROTOCOL_VERSION` for wire-only reasons and tsugumi refused its own
  authoritative blob (fixed same-day in `93ac552`). The remediation patched
  the compat list, not the coupling: nothing today forces a bump to leave the
  previous version decodable, and the test that guards the list cannot detect
  a wrong entry (both in the state-storage section).

Recurring themes: state that conflates "not yet measured" with "measured as
zero/absent" (the marquee redraw wedge, the `loaded` hash), clock-domain
mixing between wall and shared clocks (the marquee staleness guard, the
spoiler tease), and long awaits parked inline in actor select loops (the
player relaunch, the transfer-link stream open) — the actor model's one
rule, already documented at the player actor's own attach-mode counterpart.

### Fix-first order

1. 🔴 Spurious partial-file EOF advances now-playing group-wide —
   `dessplay/src/session.rs` (`on_download_playable` / Eof handling): gate
   `ReportEof` for partials + re-anchor verdict on seek.
2. 🔴 Adopted local copy never reloads the player —
   `dessplay/src/session.rs` (`on_resolved`): make `loaded` carry the path;
   one type change fixes #1 and enables the #2 gate.
3. 🔴 Dropped `OpenTransfer` wedges the download forever —
   `dessplay/src/actors/file.rs` (`queue_for_stream`) +
   `actors/network.rs`: answer every open request or retry on tick.
4. 🟠 **Regression**: Nyaa import completion doesn't cancel the peer download —
   `dessplay/src/actors/file.rs` (`on_nyaa_import_hashed`): hoist a shared
   `adopt_local_copy` helper so the third path can't miss it again.
5. 🟠 Marquee wedges the UI at 10 Hz on <~83-column terminals —
   `dessplay/src/ui/app.rs` (`advance_marquee`): distinguish "unmeasured"
   from "zero-width" slot.
6. 🟠 Player relaunch awaits `spawn()` inline for up to 30s, freezing the
   session loop — `dessplay/src/actors/player.rs` (`handle_crash`).
7. 🟠 Snapshot-version coupling can refuse the server's own blob on any
   wire-only bump — `dessplay-core/src/state.rs`: split the layout version
   from `PROTOCOL_VERSION` (or add the exhaustiveness test), and make the
   compat-list test decode real old bytes (both state-storage findings
   together).
8. 🟠 Spoiler reveal state collides across same-millisecond messages —
   `dessplay/src/ui/components.rs` (`SpoilerKey`): fold message text into
   the key.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| Partial-file playback & session wiring | 2 | 2 |  | 4 |
| Transfer & network plane | 1 | 3 | 2 | 6 |
| Player actor & drift | | 2 | 1 | 3 |
| Spoiler tags & chat | | 1 | 4 | 5 |
| AI commentary & marquee | | 2 | 6 | 8 |
| Snapshot envelope & state storage | | 2 | 1 | 3 |
| Torrent browse imports | | | 4 | 4 |
| UI shell & health line | | 1 | 2 | 3 |
| **Total** | **3** | **13** | **20** | **36** |

---

## Partial-file playback & session wiring

**Files:** `dessplay/src/session.rs` (PlayerWiring — player↔state glue),
`dessplay/src/actors/file.rs` (downloads, adoption, imports),
`dessplay/src/chunkstore.rs` (sparse assembly)
**Read first:** design.md → *File State*, *Playback Rules* #7, *Download
Cache and Retention* ("a local copy trumps the download")
**Key entry points:** `on_download_playable`, `on_resolved`,
`PlayerOutput::Eof`/`LoadFailed` arms, `cancel_redundant_peer_download`
**Theme:** `loaded: Option<Ed2kHash>` predates partial loads; every finding
here follows from it (or from an adoption path skipping the cancel step).

### 🔴 HIGH · A partial file can emit a spurious EOF that advances now-playing and marks the episode watched for the whole group

**`dessplay/src/session.rs:1843`** · _bug, confirmed_

`on_download_playable` loads the sparse partial (`ChunkStore::create` does
`set_len(size)`, so unfetched regions read as zeros) and sets
`loaded = Some(file)`. From then `speaks_for_now_playing` is true — the
**only** gate on `PlayerOutput::Eof` → `Directive::ReportEof`. mpv's
`eof-reached` rising edge is forwarded with no position-vs-duration check
anywhere in the chain (mpv.rs:573-577, player.rs:804-816, session.rs:1843,
server.rs:1512-1551). A seek into an unfetched region (or a stalled download
that lets playback walk into the zeros) can make mpv report EOF mid-episode;
the server then marks the file **watched**, advances now-playing, forces
intent Paused, takes seek authority, and auto-advances The List — for
everyone, off one bogus report. The playable verdict also lags seeks: it is
recomputed from `last_position`, which only updates on `PositionTick`, never
on `UserSeeked` — so for 1–2s after a seek past the downloaded region the
client still advertises `DownloadingPlayable` (non-blocking, and a valid
leader/position source) while demuxing zeros.

- **Spec:** design.md, Playback Rules #7 / File State — "if playback catches
  up to a gap — or a seek lands past the downloaded region — the verdict
  flips back, the user gates, and the group pauses until the window refills."
- **Suggested fix:** Regression test first (PlayerWiring level): load a
  partial via `on_download_playable`, feed a `PositionTick` at 40% of
  duration, then `PlayerOutput::Eof`, assert no `ReportEof`. Fix both halves:
  (a) when the loaded copy is a partial, gate `ReportEof` on the last known
  position being within an epsilon of `duration_millis`; (b) re-anchor
  `play_chunk` synchronously on `UserSeeked` instead of waiting for the next
  `PositionTick` — (b) is the design-faithful half. Both want `loaded` to
  know it holds a partial (see the next finding).

<details><summary>Verification trail — code pointers</summary>

Verifier confirmed every link: `on_download_playable` sets `loaded` for the
partial with a comment saying EOF reports "exactly as for a verified copy"
(session.rs:1665-1687); sparse zeros from `set_len` (chunkstore.rs:88-95);
`holds_now_playing`/`speaks_for_now_playing` are hash-equality only
(session.rs:1241-1243, 1252-1254); the Eof arm's sole gate is
`speaks_for_now_playing` (session.rs:1833-1847); mpv edge → `PlayerEvent::Eof`
unchecked (player/mpv.rs:571-577); actor gate is only
`!eof_reported && player_on_current_file` (player.rs:804-816); server
`handle_eof` checks role/now-playing/derived-state only
(server.rs:1512-1551). `UserSeeked` never writes `last_position`
(session.rs:1750-1775; written only at :1797); `play_anchor_chunk` reads it
(:1007-1025). Unverified (empirical, affects likelihood not existence): mpv's
exact behaviour demuxing a zero-filled hole (`eof-reached` vs `end-file
error`); the trigger also exists without a seek via a stalled download.

</details>

### 🔴 HIGH · An adopted local copy never reloads the player — it keeps playing the deleted partial while advertising Ready

**`dessplay/src/session.rs:1639`** · _bug, confirmed_

`loaded` is a bare `Ed2kHash`, so once the partial is loaded, both reload
sites (`on_resolved` session.rs:1638-1651, the now-playing load in `on_state`
:1479-1489) are guarded by `self.loaded != Some(file)` and skip the reload
when the *verified* copy arrives at a different path. The
"local copy trumps the download" path (`cancel_redundant_peer_download`,
file.rs:1644-1659) **deletes the partial** and emits
`Resolved{Verified(media_root_path)}`; the session flips to Ready but issues
no `Load`. On Linux the unlinked inode stays readable through mpv's fd, so no
`LoadFailed` fires: the user watches a truncated orphan to its end while the
group sees them green. The in-tree invariant comment ("`loaded` only ever
names the real verified now-playing video", session.rs:1235-1241) is exactly
what the partial-load feature broke.

- **Spec:** design.md, Download Cache and Retention — "A verified copy
  cancels the peer download … and the entry resolves Ready at the local
  path."
- **Suggested fix:** Make the bug class unrepresentable: change `loaded` to
  carry the loaded path (e.g. `Option<(Ed2kHash, PathBuf)>` or a small enum
  with a `partial` flag). The reload guard becomes "same file **and** same
  path", so a `Verified` resolution at a new path re-issues
  `PlayerCommand::Load` (mpv restores position, as the crash-relaunch path
  already relies on). Regression test at the PlayerWiring level: partial via
  `on_download_playable`, then `on_resolved(Verified(other path))`, assert a
  `Load` for the new path. This same type change gives the spurious-EOF fix
  its partial-ness bit — do them together.

<details><summary>Verification trail — code pointers</summary>

Confirmed: `loaded: Option<Ed2kHash>` (session.rs:211); both reload guards
hash-only (session.rs:1478-1489, 1636-1651); `note_local_file` (the
DownloadComplete path) emits no Load at all, relying on the assembles-in-place
invariant that adoption violates (session.rs:1180-1187); partial deleted at
file.rs:1644-1659; both adoption paths guarantee a different path
(file.rs:2088-2095 enters only when `path != download_path`, :2280-2296 hands
a media-root path); `LoadFailed` self-heal can't fire through an open fd
(session.rs:1850-1854). Stale invariant comments at session.rs:1235-1241,
:1492.

</details>

### 🟠 MEDIUM · An unopenable partial loops load → fail → full re-hash → re-load with no backoff

**`dessplay/src/session.rs:1849`** · _bug, confirmed_

If mpv cannot open the partial (an `.mp4` with its `moov` atom in the unfetched
tail is the canonical case), `LoadFailed` clears `loaded`, drops `resolved`,
writes `Missing`, and re-issues `Resolve`; the resolve hands `<cache>/<hash>`
in as a by-hash candidate, and since the live partial's mtime keeps moving,
`candidate_root` does a **full ed2k/MD4 pass over the full-size sparse file**
every iteration (seconds of CPU on a multi-GB file). `NotFound` → the next
snapshot re-emits `StartDownload` → the file actor sees the download active
with `last_playable_written == Some(true)` and unconditionally re-offers
`DownloadPlayable` (file.rs:1067-1085) → the session re-loads the same partial
→ mpv fails again. No counter, no backoff, no give-up; each lap emits two
availability mutations, so the group watches this user flap between blocking
and not-blocking. The crash ladder (design.md: third death → give up) covers
the player *process* dying, not this `end-file error` path.

- **Spec:** design.md, Player Integration / Crash handling — repeated
  failures must escalate and stop ("A file that reliably kills the player
  would otherwise loop forever").
- **Suggested fix:** Track partial-load failures in PlayerWiring; after the
  first `LoadFailed` on a partial, stop honouring `DownloadPlayable` for that
  file until the download completes or meaningful new data lands (e.g. +10%),
  and suppress the pointless re-resolve (a mid-download partial can never
  resolve Verified). Separately, skip the `<cache>/<hash>` by-hash candidate
  in `resolve` while `downloads.is_active(&file)` — that also kills the
  re-hash cost for every other caller.

<details><summary>Verification trail — code pointers</summary>

Every link confirmed: LoadFailed arm has no counter/backoff and re-resolves
unconditionally (session.rs:1849-1874); the crash ladder is a different event
(player/mpv.rs:621 → player.rs:818-837, never touches it); `ForgetLocalFile`
doesn't cancel the download (file.rs:1907-1933); re-arm via
session.rs:957-964, :989; unconditional re-offer at file.rs:1075-1085;
re-load at session.rs:1666-1688; by-hash candidate always passed
(file.rs:2577-2579) into `resolve_with_cache` → `candidate_root`
(:3173-3178, :3199-3235), mtime churn defeats the hash-cache row; flapping
via file.rs:1536-1552. Caveat: paced by hash latency + 1s snapshots, not a
tight spin; the moov-at-end trigger is plausible-but-unverified — the
unbounded-retry defect stands regardless.

</details>

### 🟠 MEDIUM · REGRESSION: a completed Nyaa import never cancels the in-flight peer download of the same hash

**`dessplay/src/actors/file.rs:1824`** · _bug, confirmed, **regressed**_

The 2026-07-19 review found this on `on_torrent_hashed`; the browse-only
rewrite re-introduced it on the replacement path. `on_nyaa_import_hashed`
calls `on_download_complete` directly and never runs
`cancel_redundant_peer_download` (whose only callers are the resolve and
scan-adoption paths). Consequences: (1) `on_download_complete` clears
`last_playable_written`, so the still-active download's next `Progress`
computes `flipped == true` and **overwrites Ready with
`Downloading {..}`** — the group gates on a user who holds a complete
verified copy, until the redundant relay transfer finishes; if the orphaned
download later stalls, `Abandon` writes `Missing` over a held file. (2)
`place_in_cache` unlinks `<cache>/<hash>` under the running `ChunkStore`'s
open fd, so subsequent chunk writes land in an orphaned inode. (3) Wasted
relay bandwidth. This is the scenario design.md names verbatim ("a
bittorrent download racing the prefetch").

- **Spec:** design.md, Download Cache and Retention: "A local copy trumps the
  download … A verified copy cancels the peer download … the partial cache
  file is deleted."
- **Prior:** **regressed** — same class as 2026-07-19's "late `TorrentHashed`
  clobbers an adopted copy" finding, on the rewritten path.
- **Suggested fix:** Cancel first (`cancel_redundant_peer_download`), then
  `place_in_cache`, then `on_download_complete` — the two paths share the
  cache path, so ordering matters. Better, per the project norm of making the
  class unrepresentable: hoist an `adopt_local_copy(file, path)` helper that
  all three "a local copy arrived" sites must go through, so a fourth path
  can't miss it. Regression test first in the `spawn_torrent_rig` suite:
  peer download active for H, browse import completes hashing to H, drive
  `on_tick`, assert no `Downloading` follows the `Ready` and
  `downloads.is_active(&H)` is false.

<details><summary>Verification trail — code pointers</summary>

Confirmed: import path shares `download_path(file)` with the live download
(file.rs:1809-1825 vs :1667); `cancel_redundant_peer_download` called only at
:2093/:2287; `Downloads` drops state only on Complete/Abandon/cancel
(download.rs:355, 661-676) so `is_active` stays true; `drop_download_streams`
is not a cancel — streams re-open while active (file.rs:1945-1982, gate at
:1977); flap mechanics at file.rs:1600-1604 (clear), :1537 (`flipped`
bypasses the 1s throttle), :1543-1551 (overwrite), :1567-1580
(Abandon→Missing); unlink-under-fd at file.rs:794-803 + chunkstore.rs:70-96.

</details>

---

## Transfer & network plane

**Files:** `dessplay/src/actors/network.rs` (QUIC links, relay ops),
`dessplay/src/actors/file.rs` (stream bookkeeping), `dessplay/src/download.rs`
(scheduler), `dessplay-core/src/net/quic.rs` (endpoints, transport config)
**Read first:** docs/proposals/2026-07-28-transfer-flow-control.md;
network-design.md → *Connection Types*, *Transfer Resumption*
**Key entry points:** `queue_for_stream`/`on_download_stream` (file.rs),
`run_transfer_link` (network.rs), `DownloadScheduler::tick` (download.rs)
**Theme:** the per-transfer-stream redesign made stream opens
fire-and-forget: the file actor assumes an answer, the network actor is
allowed to drop the request silently, and nothing retries.

### 🔴 HIGH · A dropped `OpenTransfer` permanently wedges that (source, file) transfer

**`dessplay/src/actors/file.rs:1965`** · _bug, confirmed_

`queue_for_stream` emits `FileOutput::OpenTransfer` **only** when the pending
queue for `(peer, file)` was empty; every later message is appended with no
further request, and `pending_streams` is cleared only when the stream
arrives or the download ends. The request is droppable at four unreported
points downstream: the transfer link handle being `None` (the whole
reconnect-until-`AuthOk` window), a full 64-slot `try_send`, an
`open_stream()` error breaking the link loop, and a silent header-write
failure in `announce_data_stream`. After a loss, the wedge is
self-sustaining: solicitation still succeeds over the relay, the scheduler
keeps generating `ChunkRequest`s every tick, each is appended to the dead
queue (unbounded growth), no stream is ever opened, and — with the usual
single NAS-seeder source — the download never progresses again. The peer
keeps advertising `Downloading` and gates the group until restart. A
30-second wifi blip is a sufficient trigger, directly contradicting
"brief glitches are invisible". The `OpenTransferStream` drop site carries
the same "transfer logic retries" comment as `SendPeer` — but only
`SendPeer`'s retry exists.

- **Spec:** network-design.md, Transfer Resumption — "the next chunk request
  toward a source simply opens a fresh one"; design.md, Presence — "Brief
  glitches (< 30s) are invisible."
- **Suggested fix:** Stop using queue-emptiness as the "already asked" latch.
  Minimum: per-key `asked_at` + re-emit from `on_tick` past a short timeout;
  cap the queue. Better (unrepresentable): make the network actor answer
  every `OpenTransferStream` — the stream or an explicit
  `TransferStreamFailed { peer, file }` that clears `pending_streams` — and
  buffer instead of `try_send`-dropping while the link is momentarily down.
  Regression test first over the `dessplay/tests/transfer.rs` pump harness:
  swallow the first `OpenTransfer`, assert the actor re-requests and the
  file completes. Fix the misleading comment at network.rs:668-669.

<details><summary>Verification trail — code pointers</summary>

Verifier traced all five claims: one-shot latch (file.rs:1965-1972); grep
shows `pending_streams` touched nowhere else (decl :629, init :951, insert
:1966, drains :1980/:1997/:2066 only; `on_tick` :1110-1117 has no retry);
all four drop sites (network.rs:1011-1019, 636-639, 684-687; `transfer` set
only at AuthOk :841-862); self-sustaining wedge — only `ChunkRequest`/
`Cancel` route via `send_on_stream` (file.rs:1510-1514), snub re-adds the
source but its Cancel lands on the same dead queue (download.rs:426-434,
557-651); comment asymmetry (network.rs:998-1009 vs :667-669).

</details>

### 🟠 MEDIUM · Inline `open_stream().await` + quinn's default 100-stream cap can stall the whole transfer link

**`dessplay/src/actors/network.rs:627`** · _bug, confirmed_

`shared_transport_config()` never raises `max_concurrent_bidi_streams` from
quinn's default 100, and `open_bi()` at the limit does not fail — it *waits*
for a credit. `run_transfer_link` awaits `conn.open_stream()` inline in its
select arm, so while parked the loop polls neither `conn.recv()` (accepting
server-pumped serve streams) nor `outbound.recv()` — the 64-slot op channel
fills and `SendPeer`/`OpenTransferStream` get dropped. Unbounded concurrency
is reachable on the seeder (a redeploy with an empty cache against a long
playlist starts one download per entry, ≤4 sources each): past 100 streams it
silently stops serving everyone while grinding its own backlog. The header
write was already moved off-loop for exactly this hazard; the open await —
the one that can block indefinitely — was left inline.

- **Spec:** proposal §3 — "Per-transfer streams give each recipient
  independent flow control"; network-design.md, Upload Prioritization.
- **Suggested fix:** Raise `max_concurrent_bidi_streams` (e.g. 1024) in
  `shared_transport_config()` now that one-stream-per-transfer is the design;
  move open + header write + event send into a spawned task, as
  `announce_data_stream` already does for the write half. Optionally cap
  concurrent seeder downloads. Test: drive the sim transport with more
  concurrent opens than the limit; assert relay traffic and stream acceptance
  keep flowing.

<details><summary>Verification trail — code pointers</summary>

Confirmed: inline await (network.rs:623-641), `open_bi` waits (quic.rs:
246-256), no `max_concurrent` anywhere (grep, vendor excluded; config at
quic.rs:89-113), starved arms (:646-655), 64-slot channel (:849) with
`try_send` drops (:1007, :1016), no active-download cap (file.rs:1963-1971,
download.rs:767, seeder.rs:93-115). Medium: needs seeder-scale concurrency.

</details>

### 🟠 MEDIUM · A closed/reset data stream is not treated as a snub — 30s of dead air per reset

**`dessplay/src/actors/file.rs:2048`** · _spec-drift (finder-verified reasoning; no disprove pass)_

The proposal says a closed/reset stream *is* the snub signal, with immediate
range reassignment. The implementation only removes the `download_streams`
entry ("a dead source is the snub logic's business") — but `Downloads` is
never told and has no API to be told. The dead source's chunks stay in
`in_flight`, so `plan_requests` computes zero free slots and issues nothing to
anyone until the 30s snub fires. Every transfer-link redial and server-side
pump teardown costs a download — usually one gating the group — a flat ~30s.

- **Spec:** proposal §3 — "**Snub**: a data stream silent for 30s is closed
  and its unfinished ranges are reassigned; a closed/reset stream is the same
  signal."
- **Suggested fix:** Add `Downloads::on_source_stream_lost(file, peer)`
  (requeue that source's in-flight chunks, clear its solicited flag — the
  snub body already does this for one peer) and call it from the
  `DownloadClosed` arm. Regression test in download.rs: full 64-chunk window
  outstanding, deliver stream-lost, assert immediate re-planning without
  advancing past the snub timeout.

<details><summary>Verification trail — finder reasoning</summary>

file.rs:2048-2052 (removal only); download.rs public surface has no
source-lost entry point (:83-426); slots math at :777; snub at :557-580 is
the only requeue path.

</details>

### 🟠 MEDIUM · The overhaul's core property — slow reader bounds uploader in-flight bytes — has no test

**`dessplay/tests/transfer.rs:474`** · _test-gap_

The proposal names exactly one deterministic property as proof the redesign
works; the sim transport got bounded buffers, but the one test with a stalled
reader asserts only *isolation* (the other leecher completes), never a bound
on bytes pushed into the stalled transfer. If a refactor reintroduces
serve-side buffering (spawning the chunk read, draining into a channel), the
2026-07-28 bufferbloat returns with a green suite.

- **Spec:** proposal, Testing sketch — "a slow reader bounds the uploader's
  in-flight bytes to buffers + windows, regardless of file size or source
  count."
- **Suggested fix:** Property test over the pump harness: serve stream whose
  far end never reads, request N × 250 KiB ≫ buffers, advance time, assert
  total bytes written ≤ (stream buffer + one chunk); parameterize over file
  size and source count.

### ⚪ LOW · A stale `ServeEnded`/`DownloadClosed` can remove the freshly-installed replacement stream

**`dessplay/src/actors/file.rs:2054`** · _bug, confirmed_

`serve_tasks`/`download_streams` are keyed by bare `(peer, file)`; new
streams deliberately replace stale predecessors, but the removal events carry
no generation, and streams arrive on the command channel while
`ServeEnded`/`DownloadClosed` arrive on `stream_rx` — the unbiased `select!`
can drain the stale event after the replacement is installed. For serves the
removal aborts the *new* task via `TaskGuard`, silencing the downloader's
fresh stream until its 30s snub.

- **Suggested fix:** Stamp streams/tasks with a generation id, carry it in
  the events, remove only on match — the same `transfer_conn_id` guard the
  server already uses (`serve_transfer_connection` server.rs:1160-1170,
  `classify_stream` :1200-1206).

<details><summary>Verification trail — code pointers</summary>

Keys at file.rs:625/:634; events carry only `{peer, file}` (:710-738);
replacing inserts :1990-1996/:2026-2028; unguarded removals :2048-2056;
two channels + unbiased select (:564-588, :1092, :3058); `TaskGuard::drop`
aborts (:694-700). Downloads self-heal via re-queue; serves eat the snub.

</details>

### ⚪ LOW · A permanently-unreachable transfer link is invisible above `RUST_LOG=debug`

**`dessplay/src/actors/network.rs:547`** · _quality_

The transfer connection lives on port+1, which the operator must open
separately (main.rs even carries a firewall reminder). If it's blocked, the
client authenticates, presence is green, chat works — and every download sits
at 0% forever, with dial failures logged at debug and no health-line
representation. The health row exists precisely for "QUIC is alive but a
sub-plane is dead".

- **Spec:** design.md, UI Principles ("No silent long-running work") and
  Connection Health Line.
- **Suggested fix:** Count consecutive transfer-dial failures; past a small
  threshold surface a `NetworkEvent` the advisor turns into "file transfer
  link down — is port <n+1> open?". Keep the first few failures silent.

---

## Player actor & drift

**Files:** `dessplay/src/actors/player.rs` (actor, attribution gate, crash
ladder), `dessplay/src/actors/drift.rs` (controller), `dessplay/src/player/
{mpv,mock}.rs`
**Read first:** design.md → *Events from Player* (evidence-based
attribution), *Player Lifecycle* (crash ladder); the `REATTACH_PROBE_TIMEOUT`
doc comment (player.rs:91-98) — the rule the region's own MEDIUM violates
**Key entry points:** `handle_player_event`, `player_on_current_file`,
`drift_correct`, `handle_crash`
**Theme:** the attribution gate landed almost everywhere; the gaps are the
paths added around it (relaunch awaits, drift, the mock).

### 🟠 MEDIUM · Player relaunch awaits `factory.spawn()` inline for up to 30s, backing up into the session main loop

**`dessplay/src/actors/player.rs:1047`** · _bug, confirmed_

`handle_crash` (and the startup/Load-recovery paths at :324/:489) awaits
`spawn()` — for mpv, `wait_for_socket` against the 30s `SOCKET_WAIT` that
7e7ffc9 raised — inline in the actor's select arm. While parked, the actor
services nothing, including `Shutdown`. The session's player channel is
bounded (64) and `execute` **awaits** `send` from the main loop; `plan`
pushes a `SetPlaying` on every snapshot while a file is loaded (`loaded` is
not cleared by a crash), and with peers present, position datagrams keep
snapshots firing ~10/s — so the channel fills in ~6.4s and the session main
loop then blocks for the rest of the wait: sync ops stop applying, chat
stops, Ctrl-C is not serviced. The identical hazard is recognized and bounded
one function away (`try_reattach` + `REATTACH_PROBE_TIMEOUT`, whose doc
states the rule).

- **Spec:** the actor's own `REATTACH_PROBE_TIMEOUT` doc; design.md UI
  Principles.
- **Suggested fix:** Bound the spawn like the attach path and reschedule with
  backoff (a `relaunch_at` arm mirroring `reattach_at`), or spawn the launch
  into a task delivering through the select loop. Independently, make the
  session's re-derived idempotent commands (`SetPlaying`, `SyncTo`,
  `SetBlockerOverlay`) `try_send` + drop-on-full. Regression test: mock
  factory whose post-crash spawn hangs; assert `Shutdown` is still serviced
  (the existing `a_hung_reattach_probe_does_not_block_shutdown` shape, which
  covers attach mode only).

<details><summary>Verification trail — code pointers</summary>

Confirmed: inline awaits (player.rs:1047, :489; select arm :360-364);
30s deadline (mpv.rs:55, :111; child poll only short-circuits on death);
bounded channel + awaited send (session.rs:2363, :2196; run.rs:1655-1665);
unconditional per-snapshot `SetPlaying` while loaded (session.rs:1500-1504;
crash doesn't clear `loaded`, only LoadFailed does :1854); 100ms dirty tick
(run.rs:1186, :1523). Verifier trimmed two overstatements: UI goes stale
rather than frozen, and keep-alives prevent a Lost. Severity medium — bounded
at ~30s, self-heals.

</details>

### 🟠 MEDIUM · `MockPlayer` never emits `PathChanged`, so the attribution gate silently voids what the harness thinks it tests

**`dessplay/src/player/mock.rs:127`** · _test-gap_

The real sequence is `path` → `file-loaded` → `duration`, and `path` is the
load-bearing step for `player_on_current_file`. The mock still acks
`Loaded` + `DurationKnown` only; the rendezvous harness injects `PathChanged`
*after* observing the Load, so the deterministic order is `Loaded,
DurationKnown, PathChanged` — the gate drops the `DurationKnown` in every
harness test. Nothing notices because `file_entry` helpers pre-populate
`duration_millis`, so the duration-backfill path has no end-to-end coverage;
`tests/mpv_real.rs` never asserts the ordering either, so the ordering the
attribution design rests on is assumed, not verified.

- **Spec:** design.md, Events from Player — the mock is the double meant to
  model the attribution contract.
- **Suggested fix:** Make `MockPlayer::load` ack `PathChanged` → `Loaded` →
  `DurationKnown`; drop the harness injection; assert in
  `mpv_real.rs::full_journey_against_real_mpv` that `PathChanged` for the
  loaded file precedes `Loaded` and `DurationKnown`. Then add the missing
  case: a playlist entry with `duration_millis: None` whose duration arrives
  via the backfill.

### ⚪ LOW · `drift_correct` is the one file-attributed operation without the evidence gate — and each mis-aimed hard seek leaks a `pending_seek_echoes` count

**`dessplay/src/actors/player.rs:662`** · _bug, confirmed_

Every other file-attributed path checks `player_on_current_file`
(:711/:734/:739/:810/:891); `drift_correct` runs off `estimate_now()`, which
keeps extrapolating while the gate is dropping `Position` anchors. The
session's `SyncTo` gate is `holds_now_playing`, which a user drag-in never
changes — so in attach mode (where drag-in is "the normal workflow") the
actor can slew and hard-seek the user's own unrelated file. Second-order:
`seek_programmatic` increments `pending_seek_echoes` unconditionally, but the
decrement lives *inside* the gated `Seeked` branch (:721) — a gated-out echo
is never consumed, and the user's next genuine seeks get swallowed as stale
echoes until a `Load` clears the counter.

- **Spec:** design.md, Events from Player — observations accepted "only
  while the last observed path equals the commanded one".
- **Suggested fix:** Gate `drift_correct` on `player_on_current_file()` and
  reset the `DriftController` when the gate goes false. Structurally better
  for the leak: consume a matching programmatic echo regardless of
  attribution (it is *our* seek); only the user-seek/debounce half belongs
  behind the gate. Regression test: `loaded_rig`, inject a foreign
  `PathChanged`, feed `SyncTo` 10s away, assert no `Seek`/`SetSpeed`.

<details><summary>Verification trail — code pointers</summary>

Confirmed: no gate in `drift_correct` (:662-688; entry via SyncTo :516-522);
`holds_now_playing` from commanded state (session.rs:1240-1242, emission
:1499-1527); free-running estimate (:408-416; ungated pause observation
:621-637 supplies the divergence); unconditional increment (:652-659) vs
gated decrement (:721-723), cleared only by Load (:474) / death (:929).
Verifier note: with no pause/seek event the delta doesn't grow on its own —
divergence needs one; mechanism intact.

</details>

---

## Spoiler tags & chat

**Files:** `dessplay-core/src/spoiler.rs` (parse/scramble/mask),
`dessplay/src/ui/components.rs` (ChatPane render + click), `dessplay/src/run.rs`
(IRC tap), `dessplay/src/ui/app.rs` (clock plumbing)
**Read first:** design.md → *Chat*, Spoiler tags; *IRC Bridge*, Outbound
**Key entry points:** `spoiler::seed`/`scramble`/`mask_message`,
`SpoilerKey`, `ChatPane::click`/`advance_spoilers`, `mask_irc_chat`
**Theme:** the scramble core is solid; identity/seeding choices around it
(what keys a reveal, what seeds the IRC mask, which clock drives frames)
carry the defects.

### 🟠 MEDIUM · Spoiler reveal state is keyed by (millis, sender, index) — same-millisecond messages share it

**`dessplay/src/ui/components.rs:93`** · _bug, confirmed_

`SpoilerKey` and `spoiler::seed` deliberately exclude the message text, so
two lines colliding on the triple share reveal state and scramble letters.
The realistic collision source is the IRC bridge: inbound lines are stamped
locally one-per-select-iteration, so two PRIVMSGs from one nick arriving in a
single TCP read almost always share a millisecond. Revealing line A's spoiler
silently reveals line B's — the exact disclosure the feature exists to
prevent — and before that, both runs render identical letters (a visible
tell).

- **Spec:** design.md, Chat → Spoiler tags — "seeded by message identity".
- **Suggested fix:** Fold a hash of the message text into `SpoilerKey`
  (keying only — leave `spoiler::seed` text-free so the OSD reproduces the
  letters), or give locally-appended IRC/system lines a monotonic per-line
  sequence. Regression test: two `irc_line`s, same stamp and sender, one
  spoiler each; reveal the first; assert the second stays scrambled.

<details><summary>Verification trail — code pointers</summary>

Confirmed: key fields and single map (components.rs:93-98, :172, built at
:743-747; revealed → plaintext :756-761; click mutates shared entry
:236-273); letters shared (spoiler.rs:88-92, :108-125 — source char only
selects class); IRC stamping (run.rs:1681-1686 wall-clock ms, one event per
iteration; props.rs:423-435 copies it; shell.rs:240-245 adds nothing). No
mitigation anywhere in the chain.

</details>

### ⚪ LOW · Spoiler click hit-testing counts characters, ratatui lays out by display width

**`dessplay/src/ui/components.rs:816`** · _bug, confirmed_

Hit ranges are chunk-relative *char* indices and the prefix offsets are
`chars().count()`, but ratatui advances by `cell_width()` — every
double-width (CJK/emoji) character before the run shifts the real columns
right. Load-bearing for spoilers specifically: the scramble converts wide
alphanumerics to single-width ASCII, so the run shrinks while its prefix
doesn't. The scrambled text becomes unclickable and an innocent character
takes the click; `/reveal` still works. The char-count wrap shares the
confusion (pre-existing); the hit-test turns it into a broken interaction.

- **Suggested fix:** Compute hit columns and `prefix_width` with
  `unicode_width` over the composed display text — ideally move the chat wrap
  onto display widths too (also fixes over-long CJK wrapping). UI test:
  "彼は||死ぬ||よ" — click on the rendered run reveals; one cell left does not.

<details><summary>Verification trail — code pointers</summary>

components.rs:790-819 (char-index spans), :917-920/:969 (char-count
prefixes), :989-996/:937-944/:544-557 (applied to hit ranges), :140-145/
:236-237 (raw mouse column); spoiler.rs:102-125 with the :106-107 doc naming
the char-count invariant; ratatui-core buffer.rs:352/:598 (width layout).

</details>

### ⚪ LOW · Non-alphanumeric characters pass through the scramble — an emoji can carry the spoiler in the clear

**`dessplay-core/src/spoiler.rs:112`** · _spec-drift_

`scramble` passes every `!is_alphanumeric()` char verbatim. Right for spaces
and punctuation; wrong for emoji/pictographs/arrows, which can carry the
whole spoiler — uniformly across chat, OSD, and the public IRC channel
("||💀 for the elf||" ships the skull everywhere). Also breaks the
char-count-as-column assumption (emoji are double-width).

- **Spec:** design.md — "alphanumerics replaced class-for-class, CJK
  included, **so nothing leaks**".
- **Suggested fix:** Flip the test to "is not whitespace/basic structural
  punctuation", mapping symbols/emoji to ASCII letters like the CJK case;
  extend `scramble_hides_non_ascii_alphanumerics` with an emoji case plus a
  property that output contains no non-ASCII outside whitespace/punctuation.

### ⚪ LOW · The outbound IRC mask is seeded from a hash of the plaintext — a guess-confirmation oracle on the public channel

**`dessplay/src/run.rs:1074`** · _security, confirmed_

`mask_irc_chat` seeds with `DefaultHasher` (fixed-key) over the whole message
text; `scramble` is a pure function of (seed, position, class). The published
masked line is therefore a deterministic fingerprint of the plaintext: a
channel lurker who guesses the hidden run (the surrounding text is published
verbatim) recomputes the mask and confirms the guess — exactly the audience
the mask exists to defeat. It also diverges from the chat/OSD letters, which
the design calls "the same static scramble".

- **Spec:** design.md, IRC Bridge → Outbound.
- **Suggested fix:** Seed from something that is not the plaintext — a
  per-process monotonic counter + sender, or the tap-time millis. (A constant
  seed is also wrong: every message from one sender would mask identically.)
  Test that the seed input contains no message text.

<details><summary>Verification trail — code pointers</summary>

run.rs:1074-1082 (DefaultHasher over text → mask_message); spoiler.rs:88-92,
:108-125 (deterministic given seed); OSD seeds from the shared-clock stamp
instead (session.rs:1576); spoiler.rs:157-161's own doc says the IRC case
wants "any fixed value" — not what the caller supplies. Low: yields
confirmation only, requires guessing the exact text.

</details>

### ⚪ LOW · The tease animation mixes clock domains and can freeze at generation 0

**`dessplay/src/ui/app.rs:416`** · _bug, confirmed_

Clicks are stamped from `Ui::clock` (max of wall and shared clocks) but
`advance_clock` forwards the **raw** wall millis to `advance_spoilers`. If
the local clock trails the shared clock, `now - anim_started` saturates to 0
on every tick-driven frame: the state never leaves `Animating`, generation-0
letters look unclicked, and `spoiler_animating()` pins `next_tick_hint` at
100ms for the whole skew window (snapshots do advance it — so the symptom
shows exactly when connected-but-idle, i.e. while someone reads chat).

- **Suggested fix:** Merge first: compute `self.clock = max(...)`, then feed
  the merged value to `speaker_colors.advance`, `advance_marquee`, and
  `advance_spoilers`; route `apply_snapshot` through `advance_clock`. Test:
  click with clock advanced by a snapshot to T+30s, tick with wall T+100ms,
  assert the generation advances.

<details><summary>Verification trail — code pointers</summary>

app.rs:413/:780 (the two clock writers), :982-983 (click uses merged),
:412-417/:820 (animators get unmerged values); components.rs:288-295
(saturating elapsed → early continue), :316-321 + app.rs:424-430 (10Hz pin).
Kept low: needs a genuinely mis-set local clock (normal offsets ≪ the 150ms
frame), double-click reveal unaffected, any snapshot unfreezes it.

</details>

---

## AI commentary & marquee

**Files:** `dessplay/src/commentary.rs` (engine, threads, API),
`dessplay/src/run.rs` (tick wiring), `dessplay/src/ui/app.rs` +
`components.rs` (marquee animation/slot)
**Read first:** design.md → *AI Commentary (the marquee)* — the failure
policy ("a log line and a skipped tick") and the thread/cursor rules
**Key entry points:** `plan_tick`/`spawn_job`/`finish`, `poll_screenshot`,
`advance_marquee`, the `startup_shared_millis` staleness guard
**Theme:** the engine's happy path matches the spec closely; the findings
are lifecycle edges (panic, stale file, zero-width slot, clock domains) —
acceptable-ish for a "single-user gimmick", except where they leak into the
shared UI thread (#5).

### 🟠 MEDIUM · A marquee pass never terminates when the middle slot is zero-width — permanent 10 Hz full-screen redraw

**`dessplay/src/ui/app.rs:450`** · _bug, confirmed_

`advance_marquee` latches `done` only when `anim.slot_width > 0` — but on any
terminal narrower than ~83 columns *while a file is playing* (progress text
47 cells + health ~32), the progress bar truncates to `width - health - 2`,
`free == 2`, and the measured slot width is genuinely **0**. The guard
conflates "not yet measured" with "measured as zero", so `done` never
latches: `next_tick_hint()` returns 100ms forever and the shell repaints all
four panes at ~10 Hz until compaction clears the register or the process
restarts. Existing marquee tests use a 100×30 buffer with an empty progress
text, so the collapse regime is untested.

- **Spec:** design.md, AI Commentary — "idle cost unchanged — a tick only
  repaints when something moved."
- **Suggested fix:** Make termination independent of a possibly-zero
  measurement: `slot_width: Option<usize>` (or a `measured` flag) set on
  first draw; latch `done` when `measured && (slot_width == 0 || offset >=
  slot_width + text_width)`. Regression test: playing snapshot at 80
  columns, deliver a stamp, advance a minute, assert `next_tick_hint() == 1s`.

<details><summary>Verification trail — code pointers</summary>

app.rs:437-454 (the only in-pass latch; grep confirms), :801-809 (anim starts
`slot_width: 0`), :424-431 (100ms hint); components.rs:2128/:2135/:2171
(free==2 ⇒ slot 0; `truncate_display` returns exactly max), :2051-2066
(47-cell progress), props.rs:899-955 (~32-cell health); shell.rs:206-217
(full repaint on timeout arm). Verifier re-derived the arithmetic: collapse
at ≲83 columns.

</details>

### 🟠 MEDIUM · Commentator threads retain every screenshot forever; the 5% re-roll's geometric tail means multi-MB bodies and per-tick clones

**`dessplay/src/commentary.rs:499`** · _architecture_

`Thread.turns` keeps `screenshot: Option<Vec<u8>>` per turn, untrimmed.
Costs scale with turn count: `spawn_job` clones the whole history on the
session-loop thread every tick; `build_comment_body` base64-encodes every
historical frame into every request (caching reduces *billing*, not upload);
`finish` appends unconditionally. The re-roll is geometric — P(>60 turns) ≈
4.6% — and at ~300KB/frame, 60 turns ≈ 24MB of base64 per request every 2
minutes, on the uplink whose saturation this project's health line exists to
catch, approaching the API's request cap.

- **Spec:** design.md — "The 5% re-roll doubles as the size limit: threads
  die young, no compaction needed" (true in expectation, not in the tail).
- **Suggested fix:** Cap retained *frames*, not the thread: on `finish`,
  clear `screenshot` on all but the last 1–2 turns (text stays, conversation
  unbroken, cached prefix preserved); optionally hard-cap `turns.len()`.

### ⚪ LOW · Episode key is `(series_name, Option<episode>)` — never changes across an unlinked series' episodes

**`dessplay/src/commentary.rs:781`** · _bug, confirmed_

AniDB-unknown files all get `episode_number: None` and share the
directory-hint series name, so `same_episode` stays true across every episode
change: the "Now playing" header is never re-emitted (undermining the spoiler
bound), and `episode_comments` grows all night — every 5% re-roll seeds the
fresh commentator with every comment across all episodes. A stable per-file
key (`ctx.filename`, or the now-playing hash) is already available.

- **Spec:** design.md, AI Commentary — "An episode — or series — change stays
  in-thread: the next turn opens with a 'Now playing' header."
- **Suggested fix:** Include the file identity in `episode_key`. Regression
  test: two ticks with `episode: None` and different filenames → second turn
  carries "Now playing"; a re-roll's seed carries only the second file's
  comments.

<details><summary>Verification trail</summary>

commentary.rs:587 (key type), :781 (`same_episode`), :800-810 (header/seed),
:876-880 (clear only on key change); advisor.rs:278-280; the server's
fallback writes `episode_number: None` + hint-derived name
(anidb/worker.rs:309-326). Verifier caveat narrowing scope: hint-less files
key by filename stem and do change — the bug is specific to the ordinary
directory-hint layout.

</details>

### ⚪ LOW · A late mpv screenshot survives on disk and the next tick uploads the stale frame

**`dessplay/src/commentary.rs:974`** · _bug, confirmed_

`poll_screenshot` polls 2s, then `remove_file`s on a miss — but if mpv writes
*after* the window, the file survives and the next tick (2–10 minutes later)
attaches it: the model comments on a scene one full interval old, repeating
while mpv stays slow. Nothing ties the file to the request; the mtime is
already in hand.

- **Suggested fix:** Delete the path before issuing the screenshot command,
  and/or reject any file whose `modified()` predates the request instant.

### ⚪ LOW · Screenshot path is predictable in world-writable $TMPDIR, read symlink-following, uploaded to a third party

**`dessplay/src/commentary.rs:659`** · _security, confirmed_

`$TMPDIR/dessplay-commentary-<pid>.jpg`; `metadata`/`read` follow symlinks;
contents ship to api.anthropic.com. A local attacker pre-creating the path as
a symlink gets up to 7.5MB of any dessplay-readable file exfiltrated
(`remove_file` unlinks the symlink, so it re-arms). `fs.protected_symlinks`
+ sticky /tmp mitigate on stock Linux, but that's an OS sysctl, not a guard.

- **Suggested fix:** Private per-process directory (mode 0700, or promote
  `tempfile` from dev-deps) — or `create_new(true)` the path before handing
  it to mpv so a pre-existing symlink is refused.

### ⚪ LOW · A panic in the commentary job latches `in_flight` and silently kills the feature for the session

**`dessplay/src/commentary.rs:848`** · _bug, confirmed_

`in_flight` is reset only in `finish`, reachable only via the channel send at
the end of the `spawn_blocking` body. A panic (fs read, base64 of a multi-MB
frame, `ureq`, a future model impl) drops `tx` unsent, the `JoinHandle` is
never awaited, and every later tick logs "still in flight" at debug —
permanent silence; `reconfigure` doesn't reset it either.

- **Spec:** design.md, Failure policy — "Every failure … is a log line and a
  skipped tick."
- **Suggested fix:** `catch_unwind(AssertUnwindSafe(...))` mapping a panic to
  `CommentaryError`, or hold `tx` in a Drop guard that sends a failure if
  nothing was sent; log at warn.

### ⚪ LOW · The pinned model + deprecated fixed-budget `thinking` shape makes a model bump silently kill the feature

**`dessplay/src/commentary.rs:366`** · _quality_

`build_comment_body` sends `thinking: {type: "enabled", budget_tokens}`
alongside `output_config.effort` — accepted-but-deprecated on the pinned
`claude-opus-4-6`, rejected (400) on newer models. Since `MODEL` is
"hardcoded on purpose", the natural maintenance bump turns every call into a
warn-level 400 and a dead marquee. (`build_character_body` also omits
`thinking` entirely — two calls, two unexplained depths.)

- **Suggested fix:** `thinking: {"type": "adaptive"}` with depth from
  `output_config.effort` (valid on 4.6 and newer); drop the
  `MAX_TOKENS - 768` arithmetic.

### ⚪ LOW · Marquee staleness guard compares a shared-clock stamp against raw wall-clock `snapshot.now`

**`dessplay/src/ui/app.rs:794`** · _bug, confirmed_

`startup_shared_millis` is documented as shared-clock but seeded from
`snapshot.now` = bare `SystemTime::now()` (run.rs:1887, :80-87); the stamp it
gates is genuinely shared-clock (sync.rs:413-420). A clock *leading* the
group by N suppresses every marquee written in the first N ms (adopted
pre-`done`, no log — the author sees their own fresh comment as "stale"); a
clock *lagging* far enough replays last night's final comment, the exact case
the guard exists for.

- **Spec:** design.md — "A stamp from before this session's first snapshot
  never plays"; Time Synchronization — "All state timestamps use this shared
  clock."
- **Suggested fix:** Thread the shared clock into the snapshot (apply
  `clock_offset` in `SessionLoop::snapshot`, or a separate `shared_now`
  field), or seed from the sync actor's Lamport floor; fix the field doc.
  Unit test: snapshot `now` deliberately ahead of a fresh stamp.

---

## Snapshot envelope & state storage

**Files:** `dessplay-core/src/state.rs` (envelope, compat list, untagged-v6
fallback), `dessplay-rendezvous/src/{server,storage}.rs`
**Read first:** sync-state.md → *Snapshot Storage* — the per-change-assertion
contract on `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS`
**Key entry points:** `encode_snapshot`/`decode_snapshot_flagged`,
`CrdtStateUntaggedV6`, `initial_snapshot` (server)
**Theme:** the envelope's *refuse-loudly* posture is right, but its two
safety nets — the compat-list test and the "frozen" v6 layout — both test
the current code against itself, and the version coupling already caused one
production outage. No confirmed bugs here; three structural risks on the one
data store that cannot be re-synced.

### 🟠 MEDIUM · The compat-list test re-tags a current-layout body — it cannot catch a wrongly-listed version

**`dessplay-core/src/state.rs:1151`** · _test-gap_

`layout_compatible_tagged_versions_migrate_and_others_refuse` encodes a
**current** `CrdtState`, rewrites the four version bytes, and decodes — by
construction decodable whatever version is stamped. The loop iterates the
array itself, so *any* added entry passes. The array is the single point
where an authoritative-server corruption decision is made, and postcard
ignores trailing bytes, so a misaligned decode of genuinely-old bytes can
succeed with silently wrong values.

- **Spec:** sync-state.md — each entry is "a per-change assertion … not a
  default".
- **Suggested fix:** Per listed version, keep a frozen fixture encoder (like
  `encode_untagged_v6_for_tests`) or a checked-in binary blob in that
  version's real layout, and assert the current type decodes it to the same
  resolved view.

### 🟠 MEDIUM · Layout compatibility is keyed on `PROTOCOL_VERSION`, which wire-only changes also bump — the forgotten-entry failure already bricked tsugumi once

**`dessplay-core/src/state.rs:360`** · _architecture_

v8 (DSCP split) and v9 (per-transfer streams) never touched `CrdtState`, yet
each silently invalidated the previously-written storage tag unless the
developer remembered the compat list. On the server a miss is fatal
(`initial_snapshot` refuses to start) — which happened in production on
2026-07-28; `93ac552` patched the list, not the coupling. One constant is
carrying two contracts: "which peers may connect" and "which stored blobs I
can read". Nothing turns the build red when `PROTOCOL_VERSION` moves past
the newest handled snapshot version.

- **Prior:** the 2026-07-28 outage is acknowledged in sync-state.md as
  working-as-designed ("exactly as designed"); this finding argues the
  *coupling* remains the root cause. Not previously reported by a review.
- **Suggested fix:** (a) a separate `SNAPSHOT_LAYOUT_VERSION` bumped only when
  the persisted shape changes — wire-only bumps then need no storage action;
  or (b) keep the coupling but add a test asserting every version in
  `FIRST_TAGGED_VERSION..PROTOCOL_VERSION` is handled by the compat list or
  an explicit decode arm, so the bump fails the build instead of the deploy.
  Pair with the fixture fix above (#12) so the list's entries mean something.

### ⚪ LOW · `CrdtStateUntaggedV6` is "frozen" only at the top level; its nested types drift with `crate::types`, and the fixture drifts along

**`dessplay-core/src/state.rs:242`** · _quality_

The eighteen top-level fields are frozen; every payload type
(`SeriesListEntry`, `FileAvailability`, …) is imported live — and both
changed within this very window (`anidb_unavailable`,
`DownloadingPlayable`). `encode_untagged_v6_for_tests` builds its fixture
from a *current* `CrdtState`, so fixture and decoder drift together and the
test stays green while real pre-envelope blobs quietly become undecodable —
on a not-yet-restarted server, a fatal refusal *before* `backup_pre_migration`
runs.

- **Suggested fix:** Check in a binary v6 blob captured from a real
  pre-envelope database (cannot drift), or freeze the nested types
  (`V6SeriesListEntry`, … with `From` impls). Given the fallback is
  transitional, the checked-in blob is the cheap honest option; add a dated
  note for when the fallback and blob can be deleted.

---

## Torrent browse imports

**Files:** `dessplay/src/torrent/{mod,engine,nyaa,rqbit}.rs`,
`dessplay/src/actors/file.rs` (import lifecycle)
**Read first:** design.md → *BitTorrent Downloads* — the immediate-disable
contract and the session-only-seeding rationale
**Key entry points:** `on_nyaa_import_hashed`, `set_torrent_enabled(false)`,
`RqbitEngine::promote_import`/`remove_key`
**Theme:** the browse-only refactor is clean overall (its biggest defect,
the missing peer-download cancel, is filed under the playback region as the
regression). What's left are lifecycle corners where the immediate-disable
guarantee leaks.

### ⚪ LOW · The `place_in_cache` fallback registers a cached copy the live-disable then deletes

**`dessplay/src/actors/file.rs:1814`** · _bug, confirmed_

When `place_in_cache` fails, the import registers the payload path *inside*
`<cache>/torrents/import-N/` as the cached copy. `drop_torrent` always passes
`delete_files = true` (its doc — "the cached copy is a separate hardlink and
survives" — is only true on the success path), so a live disable deletes the
registered copy; `local_files`/`cache_entries` keep naming it and the client
advertises Ready for a gone file until a serve-miss or load failure
self-heals it. `sweep_torrents_dir` explicitly spares this fallback case —
the invariant is honoured in one place and violated in the other.

- **Spec:** design.md, BitTorrent Downloads — "Cached copies of *completed*
  imports are untouched."
- **Suggested fix:** Structurally better per the norms: make the fallback
  unrepresentable — if `place_in_cache` fails, fail the import instead of
  registering a cache entry inside a directory the engine owns (then
  `sweep_torrents_dir` drops its sparing branch too). Minimum: `delete_files
  = false` when `local_files[file]` lives under `<cache>/torrents/`.

<details><summary>Verification trail</summary>

file.rs:1793-1802 (fallback), :1593-1636 (registers path + Ready),
:1755-1778 (disable → `engine.remove(file, true)`; nothing prunes
`local_files`/`cache_entries` — unlike eviction :2830-2857 and
`lost_local_file` :1908-1932); rqbit.rs:245-255 (delete forwarded);
sweep-sparing at file.rs:1691-1717 with test :6338-6400.

</details>

### ⚪ LOW · STILL OPEN: an import of an already-held library file demotes it to a retention-evictable cache path

**`dessplay/src/actors/file.rs:1786`** · _bug, confirmed, still-open_

The prior review recommended two guards on torrent completion; only the
"import still active" half survived the rewrite. With the file already under
a media root, a byte-identical browse import overwrites `local_files[file]`
with the cache path and adds a `cache_entries` row — retention later evicts
it and the client flips Missing for a file that never left its library
(self-healing at the next scan, but a needless download plus an availability
flap).

- **Prior:** still-open (2026-07-19, "A late `TorrentHashed` can clobber an
  adopted media-root copy" — the `local_files` half).
- **Suggested fix:** At the top of `on_nyaa_import_hashed`: if
  `local_files[file]` names an existing non-download path, finish the import
  against it (skip `place_in_cache`/`on_download_complete`). Fold into the
  same "is this completion still wanted?" predicate / `adopt_local_copy`
  helper as the peer-download-cancel regression — one seam for all of these.

### ⚪ LOW · `promote_import` drops a colliding entry — the second torrent seeds untracked past the disable sweep

**`dessplay/src/torrent/rqbit.rs:302`** · _bug, confirmed_

`torrents.entry(File(file)).or_insert(entry)` silently discards the newly
promoted entry — the wrapper's only `Arc<ManagedTorrent>` handle — when two
imports hash to one ed2k (two mirrors of one release; the pending-duplicate
guard checks info-hash only). librqbit keeps seeding it, but `active()`
reports the hash once and `remove_key` reaches only the first torrent: the
live-disable sweep misses it and it uploads until process exit — the exact
failure the immediate-disable escape hatch exists to prevent.

- **Spec:** design.md — "disabling applies immediately — removes every
  seeding torrent".
- **Suggested fix:** Handle the collision explicitly: delete the redundant
  newly-promoted torrent from the session (or replace-and-delete the old) —
  either way the discarded handle must reach `session.delete(.., true)`.
  Unit test: two `add_import` + two `promote_import` to one hash, then
  `remove`; assert both torrents are gone.

### ⚪ LOW · A browse search has no aggregate deadline — up to ~10 minutes of "Searching…", stackable by Enter

**`dessplay/src/torrent/nyaa.rs:128`** · _quality_

One RSS GET + up to 20 sequential `.torrent` GETs at a 30s per-call cap =
~10.5 minutes worst case; the aggregate watchdog died with the auto
torrent-first path, and `act_enter` re-issues the search on every Enter while
`searching` is true, stacking blocking-pool threads. Stale answers are
correctly discarded — waste and a stuck-looking modal, not wrong results.

- **Spec:** design.md, UI Principles — "No silent long-running work … a user
  who sees nothing happen assumes nothing is happening, and retries."
- **Suggested fix:** Aggregate budget in `browse_single_file_results` (~30s,
  return what's collected) or a few-second per-call cap on the `.torrent`
  fetches; and `act_enter` → `Msg::None` while `searching`.

---

## UI shell & health line

**Files:** `dessplay/src/ui/components.rs` (bottom line), `dessplay/src/run.rs`
(advisor wiring), `dessplay/src/advisor.rs`
**Read first:** design.md → *Connection Health Line* (the truncation-order
contract), *TUI Layout*
**Key entry points:** `HealthLine::render`, the health arm in
`SessionLoop`, `ChatPane::insert_text`
**Theme:** the health row itself classifies correctly; the failures are at
the edges — layout budget and the disconnected state. (The marquee redraw
wedge lives with the commentary region.)

### 🟠 MEDIUM · The progress bar reserves everything but two cells — the suggestion/marquee slot is unconditionally dropped below ~80 columns

**`dessplay/src/ui/components.rs:2128`** · _spec-drift_

The spec's truncation order is health > progress > suggestion; the
implementation computes `progress_max = width - health_width - 2` and lets
the bar consume it all, so in the tight regime `free == 2 < 8` and the middle
slot never renders: below `health_width + 49` columns, advisor warnings and
the entire marquee are invisible whenever a file is playing — the progress
bar never actually yields. The existing 200/100-column test runs with "no
progress bar in this snapshot" by its own comment. (This is also the terrain
of the marquee redraw wedge — fixing the budget here shrinks that bug's
trigger window, but its unmeasured/zero conflation needs its own fix.)

- **Spec:** design.md, Connection Health Line — "the health metrics keep
  their full width …, the progress bar truncates next, and the suggestion
  takes whatever middle space remains."
- **Suggested fix:** Reserve the middle-slot budget first when a suggestion
  or live marquee exists (e.g. `min(text_width + 4, remaining_after_health)`),
  give the bar the rest. Rendering test at 80 columns with
  position+duration set and a warning suggestion; assert the suggestion
  renders.

### ⚪ LOW · The advisor never re-runs on disconnect — a stale suggestion renders beside "link: down"

**`dessplay/src/run.rs:1646`** · _bug, confirmed_

The health arm's `None` branch clears `self.health` but neither clears
`self.suggestion` nor lets the advisor see `link == Down`; `advise` is
reachable only from `on_health`. The frozen suggestion ("sync stalled —
server silent 80s") renders next to `link: down — retrying` for the whole
outage, plus ~35s (ADVISE_INTERVAL + CLEAR_HOLD) after reconnect.

- **Suggested fix:** Clear `self.suggestion` in the `None` branch (or call
  `on_health` with a synthetic Down sample, bypassing the throttle, so rules
  retire their suggestions). Test: stalled sample sets the suggestion; push
  `None`; assert cleared.

<details><summary>Verification trail</summary>

client.rs:157-161 (None on Disconnected); run.rs:1601-1653 (None branch
touches only health/level; suggestion written only at :1525-1530);
advisor.rs:258-260 (sole `advise` call site), :247/:299/:361-371 (timings);
components.rs:2131-2164 + props.rs:895-898 (both rendered).

</details>

### ⚪ LOW · Chat-input paste bypasses `LineBuffer::insert_paste` — control characters reach the synced chat

**`dessplay/src/ui/components.rs:399`** · _quality_

`Ui::handle` routes a non-path paste to `ChatPane::insert_text`, which
inserts every char verbatim (`\n`, `\r`, `\x1b`) — the one text field exempt
from the "typing can never produce a control character" invariant every
other editor now enforces. Sent, the bytes sync to every peer's chat log,
OSD, IRC, and the archive; ratatui writes cell symbols through, so an escape
byte lands raw in other clients' terminals.

- **Suggested fix:** Delegate `insert_text` to `LineBuffer::insert_paste`
  (plus the `history_pos` reset). Consider also stripping control characters
  on the inbound display side (synced chat + IRC lines) so a hostile remote
  message can't write raw escapes to a terminal.

---

## Closing notes

- The two structural moves that pay off most: **`loaded` carrying its path +
  partiality** (kills both HIGHs in the playback region and gives the
  spurious-EOF fix its gate), and **a single `adopt_local_copy` seam** for
  every "a local copy turned up" path (fixes the peer-download-cancel
  regression and the still-open library-demotion finding, and makes a third
  recurrence unrepresentable — this class has now appeared in two
  consecutive reviews).
- The transfer plane wants one design decision, not four patches: **every
  `OpenTransferStream` gets an answer**. The explicit-failure-event shape
  fixes the wedged-transfer HIGH outright and gives the stream-limit,
  snub-signal, and stream-generation findings their natural signals.
- Per CLAUDE.md: regression tests first (the pump harness in
  `dessplay/tests/transfer.rs` and the PlayerWiring level in `session.rs`
  cover nearly every confirmed bug here), property tests where the space is
  wide (the slow-reader in-flight bound), and skip the test only where the
  fix makes the class unrepresentable.
