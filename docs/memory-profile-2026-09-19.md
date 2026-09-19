# Client memory profile, 2026-09-19

The largest live objects are the collective file catalogue and AniDB metadata,
especially their CRDT clocks. Allocator retention is also substantial. No
production behavior or allocator defaults were changed by this investigation.

## Scope and measurement

There was no running client on this machine. This profiles the local saved state
at revision `df038aa0`, not the particular process reported at approximately
250 MiB. The snapshot contains 52,742 catalogue entries, 52,262 metadata entries,
4,464 series-relation records, 1,280 availability entries, 14 playlist entries,
43 List entries, and 150 chat messages. Its serialized payload is 8.15 MiB.

The offline census wraps the normal allocator and counts outstanding Rust
allocation sizes and allocations. Per-field measurements clone and drop fields;
these include tree nodes, owned strings, and nested collections, but exclude
stack space, allocator size rounding, free pages, and native SQLite allocations.
The field clone measurements sum to the measured decoded state's retained heap.
The UI measurements use the real controller and renderer with a 160×48 test
terminal. They do not fetch images or start a network connection.

A temporary instrumented client also ran the production interactive entrypoint
on copied databases, with a 160×48 pseudoterminal, empty media/cache directories,
IRC/torrents/automatic downloads disabled, and the server set to loopback port 9.
It could not connect; this is an offline idle baseline. No player ran. The harness
sampled allocation counters, mimalloc usable allocation sizes, SQLite's native
memory counter, and `/proc` RSS for 25 seconds, then captured `smaps`. There were
39 threads after startup workers settled. Private databases and terminal captures
were kept outside the repository.

## Live object census

All sizes below are MiB (1,048,576 bytes). Columns are additive: the CRDT remains
owned by the sync actor while a separate resolved view is shared by the session
and UI through `Arc`.

| Objects | CRDT heap | One resolved view |
|---|---:|---:|
| AniDB file metadata | 26.06 | 5.33 |
| File catalogue | 25.94 | 6.01 |
| Series relations | 2.79 | 0.98 |
| Everything else in shared state | 0.93 | 0.13 |
| **Total** | **55.72** | **12.46** |

The CRDT has 296,250 heap allocations; constructing a view leaves another
175,769 allocations. Chat itself uses only 34,448 bytes in the CRDT and 15,072
bytes in the view. The playlist and List are also small.

Other measurements:

- The local hash cache has 122 files: 277,328 bytes of block hashes and 16,660
  bytes of paths on disk, plus small in-memory collection overhead. It is not
  a major contributor for this particular client.
- Building the complete franchise collection retains 1.82 MiB. This is a
  separately measured derived object, not an additional permanent copy assumed
  in the totals above.
- An empty UI, applying its first snapshot, and its first draw retain about
  0.63 MiB combined, excluding the shared view. The test terminal's three cell
  buffers add 1.05 MiB. Three subsequent snapshot replacements and draws have
  zero net heap growth. This short check does not establish absence of leaks.

## Whole-process accounting

The default-allocator run ended at **152.65 MiB RSS**. Late samples varied from
about 151 to 160 MiB. Rounded values below account for the final `smaps` total;
allocator counters were stable near that capture.

| Resident memory category | MiB |
|---|---:|
| Requested live Rust allocations | 73.05 |
| Additional space in mimalloc allocation size classes | 7.77 |
| Remaining resident mimalloc pages | 52.64 |
| Resident executable pages | 14.89 |
| Libraries, stacks, native heaps, shared mappings, and other pages | 4.31 |
| **Total** | **152.65** |

The 73.05 MiB of live Rust objects includes the 68.18 MiB CRDT plus view above;
the remaining approximately 4.87 MiB covers UI, runtime, file-actor and other
objects. SQLite reported 1.28 MiB of native allocations, already included in the
last row rather than added again. The database files' sizes are not their RSS.

Mimalloc's mappings account for 133.45 MiB resident, versus 80.81 MiB of usable
space in live allocations. The difference includes freed/unused resident pages,
page occupancy and allocator metadata; this census cannot subdivide it further.
It should not be described as another 52.64 MiB of application objects.

Repeating with `MIMALLOC_PURGE_DELAY=0` gave **125.59 MiB RSS**, with essentially
identical requested and rounded live allocations. Mimalloc resident pages fell
to 106.23 MiB. That is a **27.05 MiB RSS reduction** in this short offline run.
It is evidence that reclamation policy matters, not a playback performance test
or a reason to change the default without measuring CPU/page-fault costs.

## Optimisation candidates

1. **Compact the common single-actor vector clock.** The metadata and catalogue
   maps contain 105,004 entry clocks, each with exactly one actor/counter pair.
   Each allocates a 288-byte `BTreeMap` node: **28.84 MiB** of heap nodes across
   these two maps alone, already included in their CRDT totals. A small inline
   representation with a general fallback could remove most of this allocation
   cost, less any increase in the containing entries' size. The clocks already
   have one actor, so more frequent compaction will not fix this overhead.
   Preserve serialization and all clock/merge semantics; changing the vendored
   clock representation deserves convergence and compatibility testing.

2. **Reuse unchanged parts of resolved views.** `SyncCommand::GetView` calls
   `CrdtState::view()` afresh. Sharing the resulting whole view through `Arc`
   prevents fan-out copies, but does not reuse its large maps across refreshes.
   Each full refresh creates at least 12.46 MiB and 175,769 retained allocations.
   At the existing 100 ms UI refresh cadence, when continuously dirty, that is
   about **125 MiB/s and 1.76 million allocations/s**, before temporary work.
   This is an extrapolation, not a measured playback rate. Cached immutable
   metadata/catalogue sections would reduce churn and overlapping generations.
   Include snapshot adoption, merges, compaction, local and remote mutations,
   and hash computation in any cache-invalidation design.

3. **Benchmark earlier mimalloc purging.** The measured 27 MiB reduction makes
   this the cheapest experiment to repeat during actual playback and transfers.
   It does not reduce the number or size of live objects, and may trade memory
   for allocator work and page faults. A global allocator replacement is not
   indicated by these results.

Lower-priority possibilities include sharing repeated series-name strings and
using more compact read-only map storage. Chat limits, playlist data, local
block hashes, and basic terminal buffers are poor first targets in this dataset.

## What remains unaccounted for in the reported 250 MiB

This workload did not reproduce 250 MiB, so the additional roughly 97 MiB cannot
be assigned to object types from these measurements. The saved settings enable
BitTorrent and IRC; the isolated run disabled both. A connected client also
receives/merges snapshots, refreshes state during playback, and may transfer
files or display images. These are specific follow-up measurements, not assumed
explanations for the missing memory.

Code inspection identifies possible workload-dependent additions:

- Reconnection clones the CRDT for its outgoing state merge (another roughly
  55.7 MiB for this state), in addition to serialization and received state.
- Adjacent UI view generations each cost roughly 12.5 MiB until released.
- QUIC's 16 MiB stream / 64 MiB connection receive windows are flow-control
  limits, not evidence that those bytes are allocated while idle.
- Chat retains at most eight image slots, with source images scaled to a
  1280-pixel longest edge. Eight square RGBA sources could occupy 50 MiB,
  before protocol/rendered copies; decoder peaks are separate. No images were
  loaded in the census. The 64 MiB compressed-image disk cache is not a RAM cap.

## Repeating the census

Use a consistent SQLite backup of the sync database, particularly when a live
client has a WAL. For a stopped client with no WAL, a plain database copy suffices.
Using a copy also permits SQLite's shared-memory housekeeping in restricted
environments without touching the original directory.

```sh
cargo run --release -p dessplay --example memory_census -- /tmp/dessplay.sync.db
```

The example opens the supplied database read-only, prints counts/sizes rather
than contents, and starts no actors. Do not use the census process's own peak RSS
as a client estimate: its intentional clones inflate that peak. The whole-client
comparison above used a separate temporary instrumentation harness; only the
offline census is maintained in-tree.
