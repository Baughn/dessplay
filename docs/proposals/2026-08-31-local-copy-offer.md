# Proposal: Offer local copies when now-playing is missing (auto-download off)

Status: **ACCEPTED 2026-08-31, not yet implemented.** Decisions: "same
episode" is keyed on `(series_id, parsed episode number)` — the Episode
Browser's existing copy-grouping equivalence — not a new AniDB eid field;
the filename branch uses an episode-number guard plus normalized
Levenshtein ≤ 8.

## Problem

Two valid releases of the same episode can carry different ed2k roots —
the motivating case is `[RESubs] Higurashi no Naku Koro Ni Kai - 01v2 (BD
1920x1080 AC3) [40B49C7B].mkv`, which exists in two content-distinct
versions under the *same* filename (the uploader never updated the name,
CRC tag included). The playlist is keyed by ed2k hash, so a user holding
the "other" version is simply Missing for the entry.

For a user with **auto-download off** (the "bring your own files"
participant), the current behavior when now-playing lands on a file they
don't hold is the worst corner in the design:

- **Known series**: they stay Missing with no placeholder, no prompt, and
  no affordance — a silent hard blocker on the whole group until they
  press `M` and hand-browse to their copy.
- **Unknown series (with a series id)**: they are auto-set NotWatching and
  shown the placeholder — even when a perfectly good same-episode copy of
  their own sits in a media root (the group starts a show the user already
  owns in a different encode).

The manual-mapping machinery that solves this already exists (`M`), but it
is reactive and unadvertised at exactly the moment it matters.

## Design

When now-playing resolves locally Missing for a client with auto-download
disabled, and viable local candidates exist, the client shows a **modal
list of candidate copies**. Selecting one writes a **manual mapping** —
the existing local mechanism, with all its existing semantics. Dismissing
(`Esc`) leaves today's behavior untouched.

### Trigger

The prompt is **derived from state, not hooked on events**: it fires when
the conjunction becomes true —

- this client's `auto_download` is off (interactive clients only; seeders
  have no UI and always fetch),
- the now-playing entry resolves locally to `NotFound` or `HashMismatch`
  (i.e. own `FileAvailability` is Missing for now-playing; a mismatch *is*
  the Higurashi case — the exact-name file with the other hash),
- the user's preference for the entry's series is not NotWatching (someone
  who opted out is not blocking and doesn't want a dialog),
- at least one candidate exists,
- the file has not already been offered this session.

Deriving from the condition rather than the advance event covers every
channel the situation arrives through — EOF advance, manual selection,
joining/startup with now-playing already missing, a manual mapping
pruned mid-session, a local copy deleted behind our back — with one seam
instead of one hook per site.

**Once per (file, session).** Dismissal is remembered in session-local
state (not persisted); `M` remains available afterwards, and the exact
target hash appearing later (scan, wanted-set adoption) resolves the
entry as today. A *new* near-candidate appearing later does not re-raise
the modal — that would be spam.

**Interplay with auto-NotWatching**: the unknown-series
auto-NotWatching write (the auto-download-off branch of the
series-known check) is deferred while the offer is pending for that
file and proceeds on dismissal. Otherwise the user would be marked
NotWatching under the modal asking whether they want to watch their own
copy.

### Candidates

Computed over the live library index (vanished-root rows excluded, as
everywhere) joined with the synced metadata view by hash. Two branches:

1. **Same episode** (strong evidence): the candidate's hash has
   `AniDbMetadata` whose `(series_id, parsed episode number)` equals the
   target entry's. This is the Episode Browser's copy-grouping
   equivalence (`(category, number)` parse of the server-supplied epno);
   the parse helper moves to core so the session layer and the UI share
   one implementation. Requires the target itself to have metadata with a
   series id and parseable epno.
2. **Name-similar unknowns** (weak evidence): the candidate's hash has
   **no episode identity** (no metadata, or metadata lacking series id or
   parseable epno), and the filename is close to the target entry's
   filename: normalize both (lowercase, spaces → underscores), then
   - **episode-number guard**: when *both* filenames parse an episode
     number (core `episode_parse::parse_episode_number`), the numbers
     must agree — pure Levenshtein rates `… - 01` vs `… - 02` at
     distance 1, so without the guard the wrong episode ranks above a
     `v2` rename of the right one;
   - **Levenshtein ≤ 8** on the normalized names — generous enough to
     admit a `v2` marker or a changed 8-char CRC tag, safe because the
     guard screens the adjacent-episode failure mode.

A file whose metadata carries a **different** known episode identity is
never offered, however close its name — it is positively known to be the
wrong episode. Both branches cover the motivating case: if AniDB knows
both Higurashi files they share `(series_id, epno)` (branch 1); if it
knows only one, the local unknown matches at distance 0 (branch 2).

Ranking: branch-1 candidates first; within a branch, ascending name
distance (the same "nearest release" instinct as the Episode Browser's
cursor placement). Each row shows the filename and an evidence tag —
`same episode` or `name match`.

### Selection

Writes a **manual mapping** for the entry, exactly as the `M` browser
does: canonicalized, durable, filename-trusted (exempt from the
hash-mismatch verdict), never served to peers (`CannotServe` on
solicitation), joining the adoption seam on content confirmation. No
change to any of that machinery, and **no synced-state changes anywhere
in this feature** — it is entirely client-local.

### UI

A new list modal on the existing modal stack, pattern-matched on the
async-results search modals (AniDbSearch/NyaaSearch): the session layer
detects the trigger, computes the candidate list (it needs storage and
the state view, which the UI thread doesn't hold), and pushes it to the
UI as a `UiInput`; `Enter` maps, `Esc` dismisses, keybinding bar derives
from the modal as usual. The blocking "Waiting for …" OSD continues to
tell the rest of the group what's happening meanwhile.

## Non-goals

- No change for clients with auto-download on (the download resolves it).
- No automatic mapping even with a single strong candidate — a mapping
  knowingly desyncs content identity from the group (subtitle timing,
  hash compare), so the user stays in the loop.
- No offer for non-now-playing entries (prefetch has no urgency; `M`
  covers the queue).
- No new AniDB fields (eid/fid/CRC) and no server changes.

## Testing

- Candidate selection is a pure function → property tests: never offers
  a candidate whose parsed filename episode disagrees with the target's;
  never offers a hash with a different known `(series_id, epno)`; always
  offers the distance-0 same-name unknown; always offers a same-episode
  candidate regardless of name.
- Session-level: the trigger fires exactly once per file per session
  across the arrival channels (EOF advance, manual select, startup with
  missing now-playing, mapping pruned mid-session), and never when
  auto-download is on, the user is NotWatching, or no candidate exists.
- The deferred auto-NotWatching write proceeds on dismissal and is
  suppressed on selection.
