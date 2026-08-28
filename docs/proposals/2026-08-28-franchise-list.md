# Proposal: The List at franchise granularity

Status: **ACCEPTED and implemented, 2026-08-28.** Decisions: `/watch` commits to the
franchise; legacy per-season entries are folded at read time (no data
migration); the episode browser's second level is an indented tree, not a
drawn DAG; OVAs that AniDB chains as Sequel/Prequel stay inline in the main
line.

## Problem

The List showed four rows named "YuruYuri", every one opening the same
season list, and none of them near the top of the Recency sort despite the
group having watched San Hai two days earlier.

Root cause: a List entry's identity is an **AniDB series id**, and AniDB's
unit is the *season*. Series Identity resolution rule 4 ("no entry claims the
file → auto-create one, linked to the file's series id") therefore mints a
fresh entry the first time any season is played, and the short-title curator
then gives every one of them the same display name. Girls und Panzer will do
the same the first time a short is played. The group's own unit of
commitment is the *franchise* ("we're watching Yuru Yuri"), so the entry
model and the group's mental model disagree.

The purpose of The List, restated: pick tonight's episode of a series the
group follows in a couple of keystrokes, see at a glance whether such an
episode exists, in a predictable order.

## Design

### 1. Resolution is franchise-aware (core)

`series_identity::resolve_series_entry_for_file` rule 1 becomes: *an entry
linked to any series in the file's franchise component* (the same
`groups_franchise` connected components `franchise::franchises` builds),
walked locally from the file's season (`franchise::reachable_component`)
plus a one-hop check from each linked season's own relations row — the
backstop for a brand-new season whose relations row hasn't landed yet,
which is exactly the window in which a duplicate would otherwise be minted.
When several entries are linked into one component (legacy duplicates), the
**canonical** entry is a human-created one over an auto-created one (an
entry auto-created in that window must never hide the one carrying the
notes), then the one linked deepest along the prequel chain (highest
season ordinal — it holds the live `next_ep`), lowest entry id on ties
(`series_identity::canonical_first`). Every
caller — gating derivation, the server's EOF auto-advance, `/watch`,
`entries_with_unwatched_files` — gets the same answer, so a new season never
creates a second entry: the file resolves to the existing franchise entry
before rule 4 runs. This fixes the class rather than the display.

`resolve_series_entry_for_file` is now a thin wrapper over
`SeriesEntryIndex` — one implementation, so the two cannot drift; the old
scan/index equivalence proptest became a franchise-invariance proptest
(every season of a chain resolves to one canonical entry). The full
union-find lives in `franchise::series_components(view) -> BTreeMap<series,
root>`, shared by `franchises()` and The List's grouping; per-file
resolution deliberately avoids it (it runs per playlist row per snapshot).

### 2. Preference folding (core)

`derive::series_watch_for_file` reads the preference of *every* entry linked
into the resolved entry's component and folds them: **Watching > NotWatching
> Maybe** (Maybe is the neutral state; an explicit choice on any season
counts; Watching wins the unlikely conflict). Writes go to the canonical
entry. This makes legacy per-season commitments keep working with no
migration and no server-side merge — the duplicate entries become inert
members whose only remaining effect is their stored preference.

### 3. One List row per franchise (client)

`props::list_groups` groups entries by component; a `ListRow`'s `id` is the
canonical entry (edit/nero/link/Enter target — unchanged for callers).
Aggregation over members and the component's series:

- `watchers`: union of live commitments.
- `available`: any member.
- `dimmed`: no member has an unwatched held file and none is `available`.
- recency: max over every member entry's keys **and every series id in the
  component** — the "most recent episode is in season three" case.
- `status`/`next_ep`/name: the canonical entry's.

The **Recency sort no longer partitions on `dimmed`**: dimming is purely
visual; order is most-recently-watched first, never-watched last, name as
tiebreak. Unlinked entries are singleton "components" and behave as before.

### 4. Episode browser: season tree, season dimming, opening cursor

Seasons are ordered along the prequel chain instead of `(year, id)`: main
line = walk Sequel edges from the chain root(s); a member reached only via a
non-chain structural edge (SideStory, Summary, AlternativeVersion, …) renders
**indented one level under the season it attaches to**, with a `└` gutter.
Members AniDB chains as Sequel/Prequel — including OVAs like Nachuyachumi —
stay inline; that is how the group watched them. A true drawn DAG was
rejected as more work than a TUI can pay back.

A season whose every known file is watched renders dim (watched-ness only —
*not* The List's held-copy rule: after the first cut, seasons nobody
happened to advertise looked "done"; user report 2026-08-29). Branches
sit chronologically after the last main-line season that aired no later
than them (Oomuro-ke, 2024, at the bottom rather than under season one).
Opening a franchise places the cursor on the **first season with an
unwatched file**, mirroring the episode level's first-unwatched rule.

### 5. `w` on a season row

`w` in the season list marks **every known file of the season** watched
(group flag), behind a yes/no confirmation modal (a new small `Confirm`
modal; its answer carries the pending `Msg`). If every row is already
watched, the same key unmarks them all (so the toggle stays a toggle),
likewise confirmed. This is the fast path for restoring order to a database
whose history predates dessplay.

## Non-goals / follow-ups

- No automatic merge of legacy duplicate entries. Their notes/next_ep are
  simply not shown (the canonical entry's are). A hand-merge action can
  come later if the folded view proves insufficient.
- `available` automation is unchanged.
