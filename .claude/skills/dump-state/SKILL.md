---
name: dump-state
description: Inspect a dessplay client's persisted CRDT state and settings to debug a sync/state question — "check the database state", "what's my watch state for the now-playing file", "why is X blocking playback", "is so-and-so committed to this series", "trace a hash through the state", "dump the crdt state". Runs `dessplay --dump` (JSON) and slices it with jq.
---

# Skill usage

The dump-state skill should contain recipes and useful tips for understanding
the dump output. If it's missing information for your current problem, then you
can explore the code or design documentation; but you should also add to the
skill as a follow-up.

# Inspecting dessplay client state

The client persists its CRDT state as a postcard blob in SQLite — not
directly queryable with SQL. `dessplay --dump` decodes it and prints a
JSON document on **stdout** (logs go to **stderr**), so you slice it with
`jq`. Use this to answer "what does the database actually say" questions.

## Quick start

```bash
# Whole state (large — the metadata/catalog maps are multi-MB):
cargo run -q --bin dessplay -- --dump 2>/dev/null | jq 'keys'

# Trim to the sections you need (fast, small):
cargo run -q --bin dessplay -- --dump \
  --section now_playing --section series_preference --section playlist \
  2>/dev/null | jq .
```

- No `--db` → the default db for the current user
  (`~/Library/Application Support/dessplay/dessplay.db` on macOS,
  `~/.local/share/dessplay/dessplay.db` on Linux). Pass `--db <path>` for
  a non-default instance (e.g. a second client / a seeder run with its own
  `--db`).
- Safe to run **while the app is running**: `--dump` opens read-only and
  takes no instance lock; SQLite WAL allows the concurrent read.
- Always `2>/dev/null` (or `2>/tmp/dump.err`) — stderr carries tracing
  logs, stdout is pure JSON.
- The password is **redacted** in the `settings` section (shows
  `"<set>"`/`"<unset>"`, never the cleartext).

## Document shape

Top level: `database`, `epoch`, `settings`, `media_roots`, `state`.
Everything else is a field of `state`. Selectable `--section` names:

| Section | Shape | Notes |
|---|---|---|
| `playlist` | array, display order | `{hash, filename, added_by, size_bytes, duration_millis}` |
| `watched` | `{hex_hash: bool}` | group watched flags (server-written at EOF) |
| `now_playing` | hex hash or null | the current file |
| `seek_authority` | `"Server"` or `{User: "<name>"}` | |
| `playback_intent` | `"Playing"` / `"Paused"` | the play/pause latch |
| `series_preference` | array | `{user, series, state}`, state ∈ Watching/Maybe/NotWatching |
| `manual_override` | `{user: null \| "Paused" \| {Away:{set_by}}}` | |
| `file_availability` | array | `{user, hash, availability}` (Ready/Missing/Downloading) |
| `anidb_metadata` | `{hex_hash: meta\|null}` | `meta = {source, series_name, series_id, episode_number}` |
| `series_relations` | `{series_id: {...}}` | franchise graph |
| `file_catalog` | `{hex_hash: {filename, size_bytes, duration_millis}}` | identity for files you don't hold |
| `list_entries` / `list_next_ep` | `{id: {...}}` | The List |
| `lookup_requests` | array of `FileHashInfo` | pending AniDB lookups |
| `chat` | array of `{timestamp, sender, text}` | |
| `playback_position` | `{user: {position_millis, timestamp}}` | |
| `acknowledged_absent` | array of `{hash, user}` | per-file committed-absent acks |

Hashes are hex strings everywhere (keys and values), so you can grep/match
them directly.

## Recipes

**Why is the now-playing file showing a given watch state for a user?**
Watch state is keyed per **AniDB series**, not per file. A file with no
`anidb_metadata` entry (or no `series_id`) has no series to look up, so it
falls back to **Maybe** — see `series_watch_for_file` in
`dessplay-core/src/derive.rs`. This is the usual cause of "committed to one
episode but not the next": the next episode's AniDB lookup hasn't landed
yet.

```bash
cargo run -q --bin dessplay -- --dump \
  --section now_playing --section anidb_metadata --section series_preference \
  2>/dev/null | jq '
    .state.now_playing as $np
    | (.state.anidb_metadata[$np] // null) as $meta
    | { now_playing: $np,
        series_id: ($meta.series_id // null),
        series_name: ($meta.series_name // null),
        my_pref: ( [ .state.series_preference[]
                     | select(.user=="<NAME>" and .series==($meta.series_id))
                     | .state ][0] // null ) }'
# my_pref null ⇒ no stored preference for that series ⇒ derives to Maybe.
# A null series_id (no metadata yet) also derives to Maybe — wrap the
# select in [ ... ][0] // null so the row still prints in that case.
```

**A user's full series preferences:**
```bash
... --dump --section series_preference 2>/dev/null \
  | jq '[.state.series_preference[] | select(.user=="<NAME>")]'
```

**Trace one hash everywhere it appears** (playlist, metadata, catalog,
who-has-it):
```bash
H=<hex_hash>
cargo run -q --bin dessplay -- --dump \
  --section playlist --section anidb_metadata --section file_catalog \
  --section file_availability --section watched 2>/dev/null | jq --arg h "$H" '
    { playlist: [.state.playlist[] | select(.hash==$h)],
      metadata: .state.anidb_metadata[$h],
      catalog:  .state.file_catalog[$h],
      watched:  .state.watched[$h],
      holders:  [.state.file_availability[] | select(.hash==$h)] }'
```

**Who is blocking playback right now** (file availability for the
now-playing file):
```bash
cargo run -q --bin dessplay -- --dump \
  --section now_playing --section file_availability --section manual_override \
  2>/dev/null | jq '
    .state.now_playing as $np
    | { intent_file: $np,
        availability: [.state.file_availability[] | select(.hash==$np)],
        overrides: .state.manual_override }'
```

## Interpreting watch/gating state

The displayed states are **derived**, not stored. When a dump doesn't
obviously explain the UI, read `dessplay-core/src/derive.rs` — it is the
source of truth for how `series_preference` + `anidb_metadata` +
presence + `manual_override` + `playback_intent` combine. Key rule:
absent metadata/series_id ⇒ **Maybe**, not committed.

## When the fix is "wait"

A missing `anidb_metadata` entry usually means the server's AniDB FILE
lookup is still pending (rate-limited to 1 packet / 2s). The file's
`file_catalog` entry existing but `anidb_metadata` being null/absent
confirms "requested, not yet resolved". It resolves on its own; re-adding
the file or a library rescan re-arms `lookup_requests`.
