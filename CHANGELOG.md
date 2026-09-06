# DessPlay changelog

New features and fixes, grouped by calendar day, newest first. The client
embeds this file and shows unseen entries at startup; `/changelog` shows
the full history. Format: `## YYYY-MM-DD` headers in descending order,
`- ` bullets with an optional one-word `Category: ` prefix, continuation
lines indented two spaces. `cargo test` validates it. The list is
append-only; you cannot reword or remove existing entries.

Add new days at the top. Add new entries at the bottom of existing days.

## 2026-09-06

- Improved: The Waiting Below now has branching dungeon routes, optional
  fountains and guarded treasure. Escape alive whenever you choose; taking
  the ember awakens swarms, opening caverns, and warned cave-ins on the return.
- Added: Lasting wounds, damaged organs, lost limbs, regional armor, and a
  two-weapon kit make every encounter a risk. Uppercase vi keys sprint using
  breath; walking no longer restores it. Rest automatically treats injuries
  and recovers while supplies last, stopping for danger or party arrivals.
- Added: Inspect equipment and movement costs with i, injuries with v, and
  the dungeon journal with p. Visible enemy intentions help you dodge heavy blows. Endings include
  points and the rest of your character's life; injury effects can be reduced
  or disabled in Settings.
- Changed: This dungeon overhaul resets existing local expeditions and their
  history once. Other DessPlay data and messages are preserved.

## 2026-09-05

- Added: F11 opens live logs above recent chat, with scrolling and separate
  DessPlay and Rust logging levels that apply for the current session.
- Improved: default logs explain why files need indexing or re-indexing,
  including changed file size or modification time and failed hashing attempts.

- Added: F4 or /rogue opens The Waiting Below, a five-floor roguelike with
  wounds, exploration, and supplies. Every turn saves locally; friends joining
  get a persistent notice, and your expedition ends with a summary in chat.

## 2026-09-02

- Added: this changelog. New features and fixes since your last session
  pop up at startup; browse the full history any time with /changelog.

## 2026-09-01

- Added: Shift-Tab cycles pane focus in reverse.
- Added: watched downloads can be archived into your library automatically
  the moment you finish them (Settings → Files → Auto-archive watched,
  default off).

## 2026-08-31

- Added: with auto-download off, a missing now-playing file now offers
  likely local copies (same episode, or a near-identical name) to map in
  with one keypress.
- Added: the playlist's w key cycles your series commitment starting with
  commit: Maybe → Watching → Not watching.

## 2026-08-30

- Added: loading the current episode resumes from the furthest position
  anyone reached — a session that ended mid-episode picks up where the
  group left off, even if you join alone later.
- Fixed: files whose duration mpv reports as zero no longer confuse
  end-of-file handling.

## 2026-08-29

- Added: mpv starts under a dessplay profile, so you can tune player
  options per-app in your own mpv.conf.
- Added: The List marks a series watchable when any known file is
  unwatched, not only files someone currently has loaded.
- Fixed: episode-browser seasons dim when actually watched, and side
  stories sit in chronological order under the season they branch from.

## 2026-08-28

- Added: the Series pane's List shows one row per franchise — commitment,
  recency, and progress at franchise granularity, with a season tree for
  multi-season franchises.
- Added: pane splitters are mouse-draggable, and the layout persists.
- Fixed: a user mpv.conf can no longer silently unpause the player on
  file load.

## 2026-08-25

- Added: the episode browser opens on the copy matching the file you
  actually played last, and w jumps straight to the next episode.
- Added: mouse-wheel scroll-back in the separate subtitle pane.
- Fixed: playlist-padding mpv scripts can no longer hijack end-of-file
  and skip the group forward.

## 2026-08-21

- Added: /resync (also Settings → Account) clears wedged sync state and
  restarts the client; persistent divergence now heals itself or tells
  you what to do.
- Fixed: recovering from a false end-of-file mid-download no longer loops
  or seeks into unverified data.
