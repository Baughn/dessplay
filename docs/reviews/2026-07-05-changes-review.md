# DessPlay Codebase Review — Remediation Report

_Generated 2026-07-05 by a multi-agent audit (5 Opus finder agents, one per
area; every bug/security finding then independently adversarially verified).
8 raw findings → 8 kept (4 confirmed bugs, 4 non-bug findings not subject to
adversarial verification), 0 refuted._

_Revision: jj change `wospuopswxoz` · commit `141e41831c4c`. Scope: changes
since `fd835ffbe768` (20 commits: Phase 19 series identity / ListEntryId
re-keying, Phase 18 UI polish, download-cancellation, perf tweaks, fuzz
targets, and the prior review's fixes)._

<!-- audit-revision
mode: scoped
commit: 141e41831c4c
jj-change: wospuopswxoz
base: fd835ffbe768
generated: 2026-07-05
-->

Per `CLAUDE.md`: any fix here should get a regression test *before* the fix
(property/fuzz test preferred over a narrow unit test where feasible), and
`cargo fmt` should run before committing.

## Executive summary

This batch landed Phase 19 (re-keying `series_preference` to `ListEntryId`,
the four-step Series Identity resolution order, deterministic auto-create,
filename episode parsing, unlinked `next_ep` advance), the List-pane table +
disambiguation UI, mid-transfer download cancellation when a local copy
appears, and fixes for all six actionable findings from the 2026-07-03
review. The structural core of Phase 19 checks out: the deterministic,
domain-separated `derive_entry_id` closes the concurrent-auto-create fork
race correctly, the CRDT migration follows the documented key-type recipe,
and no prior-review fix regressed.

**Recurring theme — the List entry's identity data has a leaky lifecycle.**
`local_aliases` / `manual_files` are the load-bearing identity data for
unlinked series (design.md: a mis-resolved entry "silently un-commits
someone from a show they're actively watching"), and four of the eight
findings are about their care and feeding: a re-import silently **wipes**
them (the one real data-loss bug in the batch); the edit modal that
design.md promises as the place to **enter** them doesn't expose them; the
resolution function **ignores** `manual_files` whenever metadata hasn't
landed yet; and the resolution steps that consume them have **zero direct
tests**. Individually low-to-medium; together they mean the mechanism
Phase 19 was built around is hard to populate, easy to lose, and unguarded
by tests. Fixing the import preservation + adding the resolution tests
covers most of the risk cheaply.

**Regression check: clean.** The five fixes from commit `4cf8ff32` (Users
pane cursor clamp, post-`Have` stall re-solicit, seeder `known_offline`
leak, import collapse reporting, IRC Connected-after-JOIN) all held up
under adversarial re-examination. The prior review's manual-mapping finding
is the one item still open: the new solicit-attempt cooldown bounds the
formerly-perpetual re-solicitation, but a content-mismatched manual map
still advertises Ready for content it can never serve. The
speed-vs-bitrate half of the Downloading unpause rule remains
deferred-by-decision (design.md Future Plans); not re-litigated here.

### Fix-first order

1. 🟠 **MEDIUM** — Re-import silently wipes an entry's app-accumulated
   `local_aliases`/`manual_files`/`anidb_unavailable` —
   `dessplay/src/import.rs` (`submit`, update branch). Carry the existing
   entry's app-owned fields onto the parsed row, like `anidb_series_id`
   already is.
2. 🟠 **MEDIUM** — Series Identity resolution steps 2 and 3 have zero
   direct tests — `dessplay-core/src/series_identity.rs`
   (`resolve_series_entry_for_file`). Add branch + precedence tests; one of
   them doubles as the regression test for the next item.
3. ⚪ **LOW** — The metadata `?` guard blocks the `manual_files` step for
   metadata-less files — `dessplay-core/src/series_identity.rs`
   (`resolve_series_entry_for_file`). Hoist the hash-membership scan above
   the metadata guard.
4. ⚪ **LOW** — `next_ep` filename-parse fallback fires for **linked**
   entries playing a special — `dessplay-rendezvous/src/server.rs`
   (`list_advances`). Gate the fallback on the entry being unlinked.
5. ⚪ **LOW** — Snapshot-driven `StartDownload` resurrects a
   just-cancelled redundant download — `dessplay/src/actors/file.rs`
   (StartDownload handler). Guard on `local_files` before
   `downloads.start`.
6. ⚪ **LOW** (still-open) — Content-mismatched manual mapping advertises
   Ready it can never serve — `dessplay/src/actors/file.rs`
   (`set_manual_mapping`). Defer/retract the Ready advertisement around the
   `Done::ManualHashed` content check.
7. ⚪ **LOW** — List table pads by `chars().count()`, so CJK titles
   misalign every column — `dessplay/src/ui/components.rs` (List entry
   row). Use a display-width-aware measure.
8. ⚪ **LOW** — Edit modal lacks the promised `local_aliases` /
   `manual_files` fields — `dessplay/src/ui/modals.rs` (`LIST_FIELDS`).
   Add the rows, or document the deferral in design.md.

## Region index

| Region | 🔴 | 🟠 | ⚪ | Total |
|---|---:|---:|---:|---:|
| Series identity core (dessplay-core) | | 1 | 1 | 2 |
| Import (The List) | | 1 | | 1 |
| Rendezvous server (next_ep advance) | | | 1 | 1 |
| File transfer / file actor | | | 2 | 2 |
| UI (List pane, edit modal) | | | 2 | 2 |
| **Total** | **0** | **2** | **6** | **8** |

## Per-region sections

## Series identity core (dessplay-core)

**Files:** `dessplay-core/src/series_identity.rs` (the 4-step resolution
order + deterministic auto-create), `dessplay-core/src/derive.rs`
(`series_watch_for_file` gating consumer)
**Read first:** design.md → *The List* → *Series Identity* — the resolution
order 1–4 and why it is deliberately stricter than the browsing heuristics;
sync-state.md → *Series Preference* (re-keyed to `ListEntryId`)
**Key entry points:** `resolve_series_entry_for_file` (pure query backing
gating), `resolve_or_build_entry` (auto-create), `derive_entry_id`
(deterministic id synthesis)
**Theme:** the new module's hard part (deterministic convergence of
concurrent auto-creates) is right and tested; the easy part (the resolution
order itself) has an ordering slip and no direct coverage.

### 🟠 MEDIUM · Series Identity resolution steps 2 (manual_files) and 3 (name/local_aliases) have zero direct tests

**`dessplay-core/src/series_identity.rs:43`** · _test-gap_

`resolve_series_entry_for_file` implements the resolution order that keys
all group commitment, and design.md flags mis-resolution as the failure
mode that "silently un-commits someone from a show they're actively
watching." Its only tests (`series_identity.rs:151-208`) cover
`derive_entry_id` determinism and `resolve_or_build_entry`'s
miss-then-build convergence. Step 2 (hash in `entry.manual_files`, line 36)
and step 3 (`entry.name == series_name || local_aliases.contains`, lines
43–46) are exercised by no test in the workspace — confirmed by grepping
for `local_aliases`/`manual_files` assertions (only struct-init sites). A
future edit to either predicate could silently regress commitment
resolution with the whole suite green — exactly the class of change
CLAUDE.md's testing philosophy marks as high-risk.

- **Spec:** design.md, Series Identity: resolution order 1–4; "a
  mis-resolved List entry silently un-commits someone from a show they're
  actively watching."
- **Suggested fix:** Unit tests for each branch: (a) a file whose hash sits
  in one entry's `manual_files` resolves to that entry, not to a
  name-colliding other entry; (b) name match resolves; (c) a
  `local_aliases`-only match resolves; (d) precedence — manual_files beats
  name-match. Test (a) with *no* metadata written for the hash is also the
  failing regression test for the finding below — write it first and fix
  both together.

<details><summary>Verification trail — code pointers</summary>

Non-bug/test-gap finding, not subject to the disprove pass. Finder pointers:
`series_identity.rs:36` (step 2), `:43-46` (step 3), existing tests
`:151-208`; workspace grep found no resolution assertions touching either
field. Finder confidence: high.

</details>

### ⚪ LOW · The metadata `?` guard makes step 2 (manual_files) unreachable for a file with no synced metadata

**`dessplay-core/src/series_identity.rs:22`** · _bug (confirmed)_

The function early-returns via `view.anidb_metadata.get(&file)?.as_ref()?`
before any resolution step. But step 2 — `manual_files.contains(&file)` —
is a pure ed2k-hash membership test that needs no metadata; design.md
places it unconditionally in the order. Gating it behind the metadata `?`
means a file a user explicitly attached to an entry via `manual_files` (the
documented "outliers whose name doesn't parse into any alias at all" case)
resolves to `None` until the server writes metadata, so
`series_watch_for_file` (`derive.rs:78-80`) treats everyone as **Maybe**
for it, ignoring the manual commitment. Impact is transient (fallback
filename-derived metadata usually lands within seconds, and until it does
no other step could resolve either) — but it deviates from the documented
order and is a latent trap if metadata-less files ever become a durable
state.

- **Spec:** design.md, Series Identity resolution order, step 2: "The
  file's hash is in some entry's `manual_files`: that entry."
- **Suggested fix:** Regression test first (the metadata-less
  `manual_files` test from the finding above). Then run the `manual_files`
  scan before, or independent of, the metadata guard; take metadata only
  for steps 1 and 3.

<details><summary>Verification trail — code pointers</summary>

**Confirmed** by adversarial verifier: `series_identity.rs:22` early-returns
`None` before the `manual_files` scan at `:33-39`; `derive.rs:78-80`
confirms `None` → `SeriesWatchState::Maybe`. Severity kept low because the
window is transient and self-healing once fallback metadata lands.

</details>

## Import (The List)

**Files:** `dessplay/src/import.rs`
**Read first:** design.md → *The List* → *Import* (re-import is the
supported refresh workflow; `local_aliases`/`manual_files` are "empty on
import" and grown in-app afterward), *Series Identity*
**Key entry points:** `submit` (match-by-name + update branch), the parser
(`:271-273`)
**Theme:** the update branch preserves the one field the previous phase
taught it to preserve (`anidb_series_id`) and overwrites the three fields
this phase added.

### 🟠 MEDIUM · Re-importing the spreadsheet silently wipes an entry's app-accumulated local_aliases, manual_files, and anidb_unavailable

**`dessplay/src/import.rs:461`** · _bug (confirmed)_

`submit` matches an existing entry by name and writes a full
`Mutation::PutListEntry` built from `imported.entry.clone()` (`:461`) — and
the parser always constructs rows with `local_aliases` / `manual_files` /
`anidb_unavailable` empty (`:271-273`), since none of them exist in the
CSV. The update branch (`:464-476`) explicitly preserves only
`anidb_series_id`; every other field of the pre-existing entry is
overwritten. So the documented lifecycle — import once, enrich in-app (add
an alias for a differently-hinted file, attach a `manual_files` hash, let
an empty AniDB search set `anidb_unavailable`), re-import later to refresh
statuses — silently destroys the enrichment. Losing the aliases un-does
file→entry resolution: files that previously resolved to the entry (and
drove `/watch` commitment, gating, and `next_ep` advance) stop resolving,
and the next commitment auto-creates a duplicate entry. The `collapsed`
conflict report doesn't cover it (it's data loss, not a cross-sheet
conflict), and the integration test re-imports fixtures that carry no
enrichment, so the loss is invisible to the suite.

- **Spec:** design.md, Series Identity: `local_aliases`/`manual_files` are
  app-owned, grown by hand after import; the `anidb_series_id` preservation
  in this same branch is the established precedent for exactly this rule.
- **Suggested fix:** Regression test first: submit an entry, mutate its
  `local_aliases`/`manual_files`/`anidb_unavailable` in the view, re-submit
  the same report, assert the fields survive. Then fetch the existing entry
  in the update branch and carry the three app-owned fields onto the parsed
  row, mirroring the `anidb_series_id` handling (and consider merging
  rather than replacing `watchers`).

<details><summary>Verification trail — code pointers</summary>

**Confirmed** by adversarial verifier: parser empties the fields
(`:271-273`); the `seen` map retains only `(id, anidb_series_id, bool)`
(`:451-455`), so the prior entry is never available to the update branch;
only `anidb_series_id` is preserved via `.or(old_anidb)` (`:468`); the
whole parsed entry is written via `PutListEntry` (`:485-492`). Fields
confirmed app-owned in `dessplay-core/src/types.rs:463-475`. Reproducible;
severity medium (silent data loss in the primary supported workflow).

</details>

## Rendezvous server (next_ep advance)

**Files:** `dessplay-rendezvous/src/server.rs` (`list_advances`)
**Read first:** design.md → *The List* → *Advancing next_ep* — bumping the
counter is split deliberately: linked entries bump from the authoritative
AniDB episode, only unlinked entries from a filename parse
**Key entry points:** `list_advances` (EOF / MarkWatched → next_ep bump)
**Theme:** the unlinked generalization is correct for unlinked entries; the
fallback just isn't fenced off from linked ones.

### ⚪ LOW · The filename-parse fallback fires for linked entries too, so a linked special can bump next_ep past an unwatched episode

**`dessplay-rendezvous/src/server.rs:1342`** · _spec-drift_

The refactor computes `episode` as the numeric AniDB episode
`.or_else(filename parse)` (`:1336-1352`) with no linked/unlinked guard.
For a **linked** file whose AniDB `episode_number` is non-numeric — a
special/OVA, which AniDB numbers "S1", "C1", … — the fallback now guesses
from the filename, where the old code deliberately returned nothing (its
own comment: "unlinked file or special episode ('S1')"). The
`next != episode` guard (`:1366`) limits harm to the coincidence where the
filename parse equals the current `next_ep` — e.g. `next_ep="13"`, the
group watches a special named `"[Group] Show - 13.5 [1080p].mkv"` whose
dash-parse yields "13" — but then the linked series advances to "14" past
an episode 13 nobody watched. No test covers the linked-special path.

- **Spec:** design.md, Advancing next_ep: linked series increment "from
  that file's own `AniDbMetadata.episode` (authoritative)"; only "for an
  **unlinked** entry the same bump happens from the just-finished file's
  own filename-parsed episode number."
- **Suggested fix:** Regression test first: linked entry, non-numeric AniDB
  episode, filename parsing to the current `next_ep`; assert no advance.
  Then gate the `.or_else` fallback on the resolved entry being unlinked.

<details><summary>Verification trail — code pointers</summary>

Non-bug/spec-drift finding, not subject to the disprove pass. Finder
pointers: `server.rs:1336-1352` (unconditional fallback), `:1329`
(resolve), `:1366` (`next != episode` guard). Finder confidence: low on
frequency (needs the parse-collision coincidence), high on the mechanism.

</details>

## File transfer / file actor

**Files:** `dessplay/src/actors/file.rs` (StartDownload handler,
`cancel_redundant_download`, `set_manual_mapping`),
`dessplay/src/download.rs` (`Downloads::start`, snub cooldown),
`dessplay/src/session.rs` (`plan_download`)
**Read first:** design.md → *Download Cache and Retention* → "A local copy
trumps the download"; the "Ready implies servable" invariant
network-design.md's solicitation rests on
**Key entry points:** the `FileCommand::StartDownload` handler
(`file.rs:699-725`), `cancel_redundant_download` (`file.rs:1027`),
`plan_download` (`session.rs:911-952`)
**Theme:** both findings are "the file actor's advertised state and its
actual holdings can disagree across an async gap" — one across the
session↔file-actor channel, one across the manual-map hash check.

### ⚪ LOW · A snapshot-driven StartDownload arriving after cancel_redundant_download resurrects the cancelled download

**`dessplay/src/actors/file.rs:712`** · _bug (confirmed)_

`cancel_redundant_download` removes the download, deletes the partial cache
file, and emits `FileOutput::Resolved{Verified}` — but the session only
updates its `self.resolved` map when it *processes* that output
(`session.rs:1531`), and the two run as separate tasks. In the gap,
`plan_download` (`session.rs:911-952`) — invoked on every snapshot, which
flow constantly (positions, presence) — still sees the stale
`NotFound`/`HashMismatch` resolution and re-emits
`Directive::StartDownload`. The file-actor handler (`file.rs:699-725`)
checks `is_active` only for a log line and unconditionally calls
`downloads.start(...)`, which re-creates the just-deleted `ChunkStore`
(`download.rs:262-285`) and re-solicits the sources. The whole file is then
re-downloaded over the relay, and `download_complete` (`file.rs:986`)
overwrites `local_files[file]` with the cache path — flipping the served
path from the media-root copy to a retention-evictable cache copy. The
feature's guarantee ("a verified copy cancels the peer download") is
intermittently defeated in exactly the race it was built for. Self-only
(the file is Ready either way), hence low.

- **Spec:** design.md, "A local copy trumps the download": "A verified copy
  cancels the peer download ... the partial cache file is deleted ... the
  entry resolves Ready at the local path."
- **Suggested fix:** Regression test first: cancel a download, feed a
  `StartDownload` for the same file, assert no ChunkStore/partial is
  re-created and no `BlockHashRequest` goes out. Then guard the handler:
  `if self.local_files.contains_key(&file) { return; }` before
  `downloads.start` — the file actor is the authority on what it holds, so
  this closes the gap regardless of session-side staleness and keeps
  StartDownload idempotent.

<details><summary>Verification trail — code pointers</summary>

**Confirmed** by adversarial verifier: `file.rs:1219-1221` (local_files set,
then cancel), `:1027-1038` (cancel removes download + deletes partial),
`:699-725` (no local_files guard), `download.rs:262-285` (start re-creates
ChunkStore when `!contains_key`), `session.rs:911-952` (plan_download keyed
on stale `self.resolved`), `:1512-1531` (resolved updated only on
`FileOutput::Resolved`). The async window is real and snapshot traffic makes
hitting it likely.

</details>

### ⚪ LOW · Content-mismatched manual mapping still advertises Ready it can never serve (prior finding; now bounded, not eliminated)

**`dessplay/src/actors/file.rs:1660`** · _bug (confirmed)_

`set_manual_mapping` (`:1657-1666`) unconditionally inserts into
`local_files` and emits `Resolution::Verified` — advertised to peers as
`FileAvailability::Ready` — before the async content check.
`Done::ManualHashed` (`:1275-1291`) then correctly refuses to cache a
mismatched encode's block hashes, so `serve_block_hashes` has nothing to
send under that identity and a soliciting peer never gets a reply. What
changed since the prior review: the new `MAX_SOLICIT_ATTEMPTS` +
`GIVE_UP_COOLDOWN_MULTIPLIER` cooldown (`download.rs:588-607`) bounds the
formerly-perpetual re-solicitation, so the *wedge* is mitigated — but
attempts resume after each cooldown, so a peer whose sole source is the
mismatched mapper still retries futilely forever, just paced. The
underlying "Ready implies servable" violation is unchanged.

- **Spec:** design.md, File Matching 4a (a manual map is a servable local
  copy); the Ready-implies-servable invariant peer solicitation rests on.
- **Prior:** still-open (2026-07-03 review, finding 4; the `4cf8ff32` fixes
  bounded the symptom, not the cause).
- **Suggested fix:** Don't advertise Ready for a manual mapping until
  `Done::ManualHashed` confirms the content match (keep it local-only /
  playable-but-not-advertised until then), or retract the advertisement the
  moment a mismatch is detected. Regression test: map a different encode,
  assert peers never see `Ready` for that hash (or that a peer download of
  it terminates rather than cycling).

<details><summary>Verification trail — code pointers</summary>

**Confirmed** by adversarial verifier: `:1657-1666` (unconditional insert +
Verified), `:1275-1291` (mismatch deliberately not cached, "won't serve to
peers"), `download.rs:588-607` (cooldown bounds but `solicit_attempts`
resets, so retries resume). The code's own comment at `:1672-1677`
acknowledges the invariant tension. Scenario is narrow (different-encode
map + sole-source peer); low severity appropriate.

</details>

## UI (List pane, edit modal)

**Files:** `dessplay/src/ui/components.rs` (List-mode table rows),
`dessplay/src/ui/modals.rs` (`LIST_FIELDS`, `ListEditForm`),
`dessplay/src/ui/props.rs` (`candidate_rows`)
**Read first:** design.md → *The List* → *UI Integration* — the edit modal
is "also where `local_aliases`/`manual_files` are edited for an unlinked
entry"
**Key entry points:** the List entry-row builder (`components.rs:~1270`),
`LIST_FIELDS`/`ListEditForm::commit` (`modals.rs`)
**Theme:** the new table is the right surface; one rendering measure and
one missing editing surface keep it from fully delivering.

### ⚪ LOW · List table pads columns by char count, not display width — CJK titles misalign every column to their right

**`dessplay/src/ui/components.rs:1292`** · _quality_

The List-mode row computes `visible_len` from `chars().count()` over the
name (`:1292`), nero_name (`:1296`) and marker (`:1289`), then right-pads
to `name_width` (`:1299-1301`). A full-width CJK glyph occupies two
terminal cells but counts as one scalar, so any Japanese title — routine
for this data, especially `nero_name` — under-pads and shifts the
episode/available/watchers columns, defeating the alignment that is the
feature's stated point ("fixed-width, aligned cells instead of drifting
with the name's length", comment at `:1270-1272`). The same char-count
convention exists elsewhere (e.g. `modals.rs:1356-1358` holder padding),
but the List table is where alignment is load-bearing and newly introduced.
Purely cosmetic.

- **Spec:** design.md, The List UI Integration: aligned fixed-width cells.
- **Suggested fix:** Measure with a display-width-aware count (the
  `unicode-width` crate's `UnicodeWidthStr::width`) in `visible_len` and
  the mirror pad sites. (New dependency — small, ubiquitous, and the
  standard answer here; flag per the global CLAUDE.md package rule.)

<details><summary>Verification trail — code pointers</summary>

Non-bug/quality finding, not subject to the disprove pass. Finder pointers:
`components.rs:1289,1292,1296,1299-1301`; mirror at `modals.rs:1356-1358`.
Finder confidence: high.

</details>

### ⚪ LOW · The edit modal exposes no local_aliases / manual_files fields, so the promised hand-editing surface doesn't exist

**`dessplay/src/ui/modals.rs:1405`** · _spec-drift_

design.md says editing happens in the edit modal, "(also where
`local_aliases`/`manual_files` are edited for an unlinked entry)", and
Series Identity's whole alias-growth story ("grown by hand ... whenever a
differently-hinted file for the same show shows up") routes through it. But
`LIST_FIELDS` (`:1405-1416`) contains only
Name/Nero/Genre/Notes/Recommender/Status/Status-note/Source/Next-ep/Available,
and `ListEditForm::commit` (`:1538`) never writes either set — while
`candidate_rows` (`props.rs:1155-1166`) actively reads both to rank
candidates. No data is lost (the form clones the whole entry), and the form
comment defers `watchers` explicitly but not these — so this is an
unimplemented promised capability, not a documented deferral. Today the
only route into `manual_files` is nothing at all, which also makes the
resolution-order step 2 finding above unreachable in practice.

- **Spec:** design.md, The List UI Integration: "Editing fields and adding
  entries happens in a small edit modal (also where
  `local_aliases`/`manual_files` are edited for an unlinked entry)."
- **Suggested fix:** Add editable rows (aliases as a joined list, like
  Notes; manual_files likely append-only by current-file hash) to
  `LIST_FIELDS`/`field_value`/`commit` — or update design.md to record the
  deferral alongside `watchers`. Either resolves the drift; per
  docs/CLAUDE.md, a deliberate deferral belongs in the doc.

<details><summary>Verification trail — code pointers</summary>

Non-bug/spec-drift finding, not subject to the disprove pass. Finder
pointers: `modals.rs:1405-1419`, `:1476-1558`; read sites
`props.rs:1155-1166`. Finder confidence: medium.

</details>

## Addendum: plan.md Phase 19 vs. reality (generated with a different model; user-requested extra look)

The Phase 19 plan text (plan.md:1166-1231) was checked line-by-line against
design.md and the landed code. The plan itself is largely faithful to the
design discussion; the gaps are mostly in what got *executed* against it.

**One substantive error in the plan text:** the resolution-order bullet
(plan.md:1191-1192) states the order as "AniDB link -> `manual_files` ->
`local_aliases` -> auto-create", **omitting the `name` match** — design.md's
step 3 is "the file's derived name matches some entry's **`name` or**
`local_aliases`". The implementation follows design.md (name is checked,
`series_identity.rs:44`), so the code is right and the plan is wrong. Worth
a one-word fix so a future session doesn't "correct" the code to match the
plan.

**Phase status:** Phase 19 carries no `Status:` line and is genuinely
incomplete — but the Phase 10 status line added this batch ("Completed as
side-effects of phase 1-19") reads as though 19 were done. Built: protocol
v4, the re-key + two-map migration (reuse + synthesis, covered by the three
`legacy_blob_*` tests in `state.rs`), the new `SeriesListEntry` fields, the
resolution function, the unlinked `next_ep` bump, the disambiguation view,
the List default mode. **Not built:** the edit-modal
`local_aliases`/`manual_files` fields (finding above) — promised at
plan.md:1209-1210.

**Testing promises vs. reality — 3 of 5 missing.** This is the addendum's
main yield; it independently corroborates the audit's two medium findings:

| Promised (plan.md:1212-1226) | Reality |
|---|---|
| Property test: re-keying migration preserves every (subject, value) pair | **Partial** — landed as unit tests over crafted legacy blobs (`state.rs::legacy_blob_reuses_an_existing_linked_list_entry`, `legacy_blob_synthesizes_one_shared_entry_with_watchers_seeded`), not a property test; substance mostly covered |
| Property test: resolution order deterministic + idempotent | **Missing** — only `derive_entry_id` determinism is tested; steps 2/3 have zero coverage (= finding 2) |
| Gating property tests parameterized over linked/unlinked | **Missing** — every `derive.rs` gating test routes through `link_series(...)` with an `AniDbSeriesId`; no test gates on an unlinked entry, so the phase's own milestone claim ("gates playback across absence exactly like a linked one") is unverified |
| Unit test: two differently-hinted files resolve via `local_aliases` to one `ListEntryId` | **Missing** — no `local_aliases` resolution assertion anywhere in the workspace |
| insta snapshot: candidate-ranked disambiguation tree | **Missing as a snapshot** — behavioral coverage exists (`app.rs:2696` opens-the-browser assertion, `props.rs:2431` manual-files-first ranking) but no rendered-tree snapshot |

Suggested handling: fold the missing linked/unlinked gating parameterization
and the `local_aliases` resolution test into the fix-first item 2 work unit
(they are the same test file + neighborhood), correct the plan's
resolution-order wording, and either mark Phase 19's remaining items
explicitly or complete them before adding a status line.
