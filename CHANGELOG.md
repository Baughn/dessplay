# DessPlay changelog

New features and fixes, grouped by calendar day, newest first. The client
embeds this file and shows unseen entries at startup; `/changelog` shows
the full history. Format: `## YYYY-MM-DD` headers in descending order,
`- ` bullets with an optional one-word `Category: ` prefix, continuation
lines indented two spaces. `cargo test` validates it.

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
