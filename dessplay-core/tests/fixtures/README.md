# Snapshot fixture blobs

Binary `crdt_state` snapshot blobs pinning the storage decode paths in
`tests/migration.rs` (`layout_compatible_fixture_blobs_decode_to_the_expected_view`,
`frozen_layout_fixture_blobs_decode_to_the_expected_view`,
`untagged_v6_fixture_blob_decodes_via_the_legacy_fallback`). The
`snapshot-v*.bin` and `snapshot-untagged-v6.bin` blobs encode
`rich_sample_state()`; the `snapshot-multi-relations-*.bin` family
encodes `multi_relations_sample_state()` (both in tests/migration.rs).

| File | Layout |
|------|--------|
| `snapshot-v7.bin` … `snapshot-v10.bin` | Tagged envelope, one per entry of `FROZEN_LAYOUT_SNAPSHOT_VERSIONS` (the shared v7–v10 layout, decoded via `CrdtStateV10`) |
| `snapshot-v11.bin`, `snapshot-v12.bin` | Tagged envelope, one per entry of `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` (current layout under an older tag) |
| `snapshot-untagged-v6.bin` | The pre-envelope untagged v6 layout (`CrdtStateUntaggedV6`) |
| `snapshot-multi-relations-untagged-v6.bin` | Untagged v6 layout again, but with a **three-entry** `series_relations` map (distinct keys, stamps, and actors) — the map-rebuild pin (`multi_relations_untagged_v6_fixture_decodes_to_the_expected_view`, `multi_relations_fixture_merges_with_an_independently_migrated_subset`) |
| `snapshot-multi-relations-v12.bin` | Tagged envelope at v12 (current at capture), the only blob whose `series_relations` carries a **non-empty `short_titles`** (`multi_relations_current_fixture_pins_nonempty_short_titles`) |

## Policy: written once, never regenerated

A fixture is captured **once**, at the moment its version enters a
handled-versions list, and is **never regenerated** — its whole value is
that its bytes stay frozen from capture time, so any later drift of the
decoding types against them fails the decode test instead of silently
orphaning deployed databases. Regenerating a fixture from the current
encoder would re-anchor the pin at "today" and erase exactly the drift
it exists to catch. If a layout-compatible fixture stops decoding, the
fix is never to re-bake it: its version must leave
`LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` for a frozen-layout decode arm —
taking its already-captured fixture along as that arm's pin, exactly as
v7–v9 did at the v11 bump.

When a `PROTOCOL_VERSION` bump adds a new version to
`LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` (the red-build guard
`every_tagged_snapshot_version_is_deliberately_handled` forces the
decision), capture the new version's fixture and commit it:

```
cargo test -p dessplay-core --test migration -- --ignored capture_missing_snapshot_fixtures
```

The capture test only writes files that don't exist yet; existing
fixtures are structurally protected from rewrites.

Capturing with the *current* encoder is honest because a version is
only listed as layout-compatible when its persisted layout is
byte-identical to the current one — so at the moment of the bump, the
current encoder reproduces the old version's real bytes.

When a bump instead **reshapes** persisted state (a frozen-layout arm),
the outgoing version's fixture must be captured **at the bump, before
the shape change lands** — that is the last moment any encoder produces
its real bytes; afterwards they are unrecoverable. `snapshot-v10.bin`
was captured this way. The capture test cannot do this for you (it runs
the already-changed encoder), so it is a manual step of the bump.

## Provenance

`snapshot-v7.bin`–`snapshot-v9.bin` and `snapshot-untagged-v6.bin` were
captured 2026-08-13 (current version v10). v7 → v9 were wire-only bumps
and v9 → v10 only appended an enum variant, so these bytes are faithful
to what those builds wrote — except that the v6 blob's nested value
shapes had already drifted append-only (`anidb_unavailable`,
`DownloadingPlayable`) since the envelope landed; it pins the layout
from 2026-08-13 forward. The v6 fixture (and the fallback it tests) can
be deleted under the condition in `CrdtStateUntaggedV6`'s docs.

`snapshot-v10.bin` was captured 2026-08-17, immediately **before** the
v11 bump appended `SeriesRelations::short_titles` — the change that
moved v7–v10 from `LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` to the frozen
`CrdtStateV10` decode arm. Its body is byte-identical to
`snapshot-v9.bin`'s (only the envelope tag differs), which is the
layout-compatibility claim made literal.

`snapshot-v11.bin` was captured 2026-08-18 at the v12 bump (wire-only:
the `SetAnthropicToken` message), when v11 entered
`LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` — the ordinary capture-test flow,
since the current encoder still reproduces v11 bodies.

`snapshot-v12.bin` was captured 2026-08-21 at the v13 bump (wire-only:
the `SyncStatus` connect-handshake message), when v12 entered
`LAYOUT_COMPATIBLE_SNAPSHOT_VERSIONS` — the same capture-test flow.

The `snapshot-multi-relations-*.bin` pair was captured 2026-08-20
(current version v12, `capture_missing_multi_relations_fixtures`),
closing the 2026-08-20 audit's fixture gap: every `rich_sample_state`
blob holds exactly one `series_relations` entry — the size at which the
constant-actor migration re-dot bug was invisible — and `short_titles`
empty, so the field was pinned only at its zero encoding. The untagged
blob pins the frozen v6 layout with a multi-entry map (the v6 encoding
drops `short_titles`, so its expected view clears them); the v12 blob
pins the non-empty `short_titles` encoding. The v12 blob decodes as the
current version today; at the next `PROTOCOL_VERSION` bump it becomes
the drift pin for however v12 bodies are then handled — the same role
`snapshot-v10.bin` took on at the v11 bump. Neither blob is ever
regenerated.
