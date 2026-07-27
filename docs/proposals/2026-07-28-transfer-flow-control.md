# Proposal: Transfer Flow Control Overhaul (BBR, per-transfer streams, DSCP)

Status: **agreed, not yet implemented** (2026-07-28)

When implemented, fold the design into network-design.md (Connection
Types, Channel Usage, File Transfer, Flow Control, Relay Mechanics) and
mark this proposal implemented.

## Problem

2026-07-28: Quickshot served files over the peer relay and his health
line showed ~25s RTT; chat was delayed to match. Diagnosis, confirmed in
code:

- The QUIC congestion controller is quinn's default **Cubic**
  (`shared_transport_config()` never sets a factory) — pure loss-based.
  On a Starlink-class uplink with deep, AQM-less buffers, Cubic fills
  the bottleneck queue until something drops, which is approximately
  never. The standing queue *is* the RTT.
- The app-level transfer "flow control" is a **fixed window**: 16
  outstanding chunk requests per source (`pipeline_depth`) × 250 KiB,
  per downloader. Two downloaders (a user plus the seeder auto-fetching)
  keep ~8 MiB permanently in flight — ~25s at a few Mbit/s. Nothing
  measures RTT or adapts.
- The control stream's priority (100 vs 0) reorders only quinn's local
  send queue. Once bytes sit in the network's bloated buffer, every
  packet — chat, time-sync probes, position datagrams — waits behind
  them. The 25s reading was *accurate*: that was the path RTT.
- The `upload_limit` token bucket is opt-in, defaults to unlimited, and
  applies only at restart. It is a cap, not a controller.

The RTT probe design anticipated this ("bufferbloat shows up as
seconds") — the measurement worked; nothing acts on it.

## Design

Four changes, agreed 2026-07-28. Guiding principle: **congestion control
is QUIC's job** — the app stops throttling (fixed pipelines) and stops
compensating (oversized static windows as "the limiter"), and instead
lets a delay-aware transport pace everything end to end. Blocks/chunks
remain for load-spreading, verification, and resume — never throttling.

### 1. BBR

Set `congestion_controller_factory` to `quinn::congestion::BbrConfig` in
`shared_transport_config()` (both sides — the server serves bulk toward
downloaders too). BBR is delay-aware: it paces to the estimated
bottleneck bandwidth and bounds in-flight to ~a couple of BDPs instead
of filling the buffer until loss. This alone kills the 25-second queue.

quinn's BBR is marked experimental (BBRv1-ish). Acceptable: the
alternative (Cubic) is the proven cause of the incident, and the
blast radius is our own five-person overlay.

The existing flow-control windows (16 MiB/stream, 64 MiB/connection)
**stay**. They are receive-side memory bounds, not queue-builders; the
queue was Cubic's doing. "No statically configured windows" lands as
"the app pipeline stops being the throttle", not "shrink QUIC windows".

### 2. Two connections, DSCP-tagged

DSCP is a per-packet IP-header field, but quinn exposes no per-transmit
DSCP (quinn-udp's `Transmit` carries only ECN) — so tagging is
per-socket via `setsockopt(IP_TOS / IPV6_TCLASS)`, one value per
endpoint. Streams within a connection cannot be tagged apart. Therefore
the client↔server link splits into **two QUIC connections**:

| Connection | Carries | DSCP |
|------------|---------|------|
| **Control** | control stream, state-op datagrams, position datagrams, time-sync probes | **AF41** (34) |
| **Transfer** | peer-message stream, per-transfer data streams (§3) | **AF21** (18) |

Torrents (librqbit's own sockets) stay untagged at CS0. librqbit 8.1.1
exposes no socket-TOS hook, so "torrents lowest" is achieved by raising
dessplay instead — on any DSCP-respecting router the relative order
comes out control > transfer > torrents. An upstream librqbit PR
exposing socket TOS (so torrents could get LE, RFC 8622) is a nice-to-
have, explicitly not blocking.

Mechanics and caveats:

- Sockets are built by us (`socket2`), tagged, then handed to
  `quinn::Endpoint::new` — the connector already creates one endpoint
  per address family, so both `IP_TOS` (v4) and `IPV6_TCLASS` (v6) get
  set. The server tags its socket(s) symmetrically (AF41 on the control
  endpoint's socket, AF21 on transfer) so the *downlink* direction is
  classifiable too. Windows ignores `IP_TOS` without system QoS policy;
  tagging is best-effort everywhere (a failed setsockopt logs at debug
  and continues).
- **ISP bleaching doesn't matter.** The queue that hurt is the sender's
  own router/uplink egress; DSCP survives from the host to that first
  queue, which is exactly where a configured router (Dagger's) acts.
  What the wider internet does with the bits is irrelevant.
- **Binding.** `Auth`/`AuthOk` stay on the control connection.
  `AuthOk` gains an appended `transfer_token` field (bump policy:
  append-only; `Auth` itself is untouched). The client then dials the
  transfer connection and opens a single setup stream carrying
  `TransferAuth { username, token }`; the server binds that connection
  to the session. No password re-send, no `Auth` reshaping.
- **Presence stays keyed to the control connection only.** The transfer
  connection carries no keep-alive obligation toward presence; if it
  dies, in-flight transfers fail over/restart after redial (the
  existing "transfers have no connection of their own" resumption
  logic), and the user is never marked Lost by it. Redial is lazy —
  on demand, when a transfer or serve next needs it.
- **Probe caveat, recorded deliberately:** time-sync probes ride the
  control connection. On a DSCP-aware router they measure the
  *prioritized* queue and under-report bulk-induced bloat. With BBR
  bounding the bloat this is acceptable; the health line's sync-age
  detector is unaffected.

### 3. TCP-like transfers: one QUIC stream per transfer

Today every relayed envelope for a peer shares **one** relay stream, and
the fixed request pipeline is the de-facto flow control. Both go away:

- **Peer-message stream** (per peer, on the transfer connection): the
  existing envelope protocol keeps carrying the *small* messages —
  `FileAvailability`, `BlockHashRequest`/`BlockHashes`, `CannotServe`,
  and scheduling control. Bulk data leaves it entirely.
- **Data streams** (per active (source, file) transfer): the downloader
  opens a fresh bidirectional stream through the relay with a header
  frame `TransferOpen { to, file }`; the server opens a matching stream
  on the uploader's transfer connection and becomes a **per-stream byte
  pump** with a small bounded buffer (order 256 KiB) in each direction,
  so QUIC stream backpressure propagates end to end: slow downloader →
  server pump stalls → uploader's stream write awaits. Down the stream
  flow `RangeRequest { chunks }` frames; up come `ChunkData` frames,
  written back-to-back as fast as the stream accepts — **pacing comes
  from BBR + stream flow control, not from counting requests.**
- The downloader keeps only enough ranges outstanding to hide relay
  round-trip latency (request the next range when the current one is
  half-consumed) — requests are tiny; there is no in-flight *data*
  budget at the app layer at all. `--pipeline-depth` is deleted.
- This also fixes a latent head-of-line defect: today one slow
  downloader stalls the uploader's single relay stream for *all*
  recipients. Per-transfer streams give each recipient independent
  flow control, and QUIC interleaves fairly.

**What stays, unchanged in role:**

- 250 KiB chunks and ed2k block verification — the units of
  load-spreading, verification, and resume (their designed jobs).
- The scheduler's source assignment: sequential window ahead of
  playback, rarest-first outside it, ≤4 concurrent sources — now
  expressed as which *ranges* go to which source's stream.
- **Snub**: a data stream silent for 30s is closed and its unfinished
  ranges are reassigned; a closed/reset stream is the same signal.
- **Endgame**: the tail may be requested on several sources' streams;
  first arrival wins, losers' streams get a `Cancel` (or are closed).
- The `UploadLimiter` token bucket, as the user's explicit cap on
  serve-side reads.

### 4. What this deletes

- `--pipeline-depth` and the fixed per-source request window.
- The oversized-windows-as-limiter doctrine ("sized ≥ BDP so the
  app-level pipeline depth is the limiter, not QUIC backpressure") —
  inverted: QUIC *is* the limiter, by design.
- The single shared relay stream as the bulk path (it survives as the
  small peer-message stream).

## Protocol impact

`PROTOCOL_VERSION` bump (wire messages change: `AuthOk` gains a field,
new `TransferAuth`/`TransferOpen`/`RangeRequest` shapes, `ChunkRequest`
retired from the peer-message stream). Everyone updates in lockstep, as
usual; `Auth` is untouched so downlevel clients still get a readable
`ProtocolMismatch`.

## Testing sketch

- Simulated transport grows per-stream backpressure semantics (bounded
  stream buffers), so the core property is testable deterministically:
  **a slow reader bounds the uploader's in-flight bytes** to
  buffers + windows, regardless of file size or source count.
- Snub/endgame/resume tests port to the range protocol (same scheduler
  core, `dessplay/src/download.rs`, still synchronous and seedable).
- Real-QUIC localhost regression tests: the two-connection handshake
  (token binding, transfer-connection death not affecting presence),
  in the same family as `quic_localhost::*`.
- DSCP: `getsockopt` readback smoke test (tagging applied), platform-
  gated; behavioral verification on a configured router is manual
  (Dagger).
- BBR itself is not unit-tested (quinn's job); the incident's symptom
  is covered by the backpressure property above plus a manual
  saturated-uplink check against the health line's probe RTT.
