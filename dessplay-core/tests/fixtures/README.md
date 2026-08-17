# Snapshot fixture blobs

Binary `crdt_state` snapshot blobs pinning the storage decode paths in
`tests/migration.rs` (`layout_compatible_fixture_blobs_decode_to_the_expected_view`,
`frozen_layout_fixture_blobs_decode_to_the_expected_view`,
`untagged_v6_fixture_blob_decodes_via_the_legacy_fallback`). Every blob
encodes `rich_sample_state()`.

| File | Layout |
|------|--------|
| `snapshot-v7.bin` … `snapshot-v10.bin` | Tagged envelope, one per entry of `FROZEN_LAYOUT_SNAPSHOT_VERSIONS` (the shared v7–v10 layout, decoded via `CrdtStateV10`) |
| `snapshot-untagged-v6.bin` | The pre-envelope untagged v6 layout (`CrdtStateUntaggedV6`) |

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
