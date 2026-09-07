# DessPlay changelog

New features and fixes, grouped by calendar day, newest first. The client
embeds this file and shows unseen entries at startup; `/changelog` shows
the full history. Format: `## YYYY-MM-DD` headers in descending order,
`- ` bullets with an optional one-word `Category: ` prefix, continuation
lines indented two spaces. `cargo test` validates it. The list is
append-only; you cannot reword or remove existing entries.

Add new days at the top. Add new entries at the bottom of existing days.

## 2026-09-08

- Added: Customize file and episode browsers, AniDB and Nyaa search results,
  active imports, local-copy offers, confirmations, and name dialogs in layout
  files. Their frames, row fields, and editor placement now follow templates.

## 2026-09-07

- Added: Image links posted in chat (DessPlay or IRC) now download and
  display inline in the chat log, under the message and at most a third of
  the pane tall, rendered as colored half-block art that works in every
  terminal. Downloads are https-only and size-capped; turn the feature off
  under Settings → Playback & display ("Inline chat images").
- Fixed: Inline images drawn with the Kitty graphics protocol
  (`DESSPLAY_IMAGE_PROTOCOL=kitty`/`auto`) no longer land at the top-left
  of the screen when the terminal was left in margin mode by a previous
  program; DessPlay now resets that state at startup.
- Changed: Inline chat images now use your terminal's own graphics
  protocol (Kitty, sixel, or iTerm2) when it has one, showing real pixels
  instead of half-block art; terminals without one still get half-blocks.
  Set `DESSPLAY_IMAGE_PROTOCOL=halfblocks` to go back to the old look.
- Added: Settings and List-entry forms, their field layouts, and the log
  viewer's sections can now be customized with local XML templates and CSS.
  Use `dessplay layout init` to export defaults, `layout check` to validate,
  and F12 for live-reload diagnostics and recovery tools.
- Added: Move or hide panes by editing the application layout template. Tab
  order and mouse targets follow your layout; named splitters support dragging
  and remember sizes until the layout files change. F12 resets drag sizes.

- Improved: Chat images now have a border aligned after the timestamp. The
  frame fits within the existing height limit and scrolls with the image.
  Edit the chat template or stylesheet to change its border and alignment.
- Added: Customize chat composition, command suggestions, recent chat, and
  separate subtitle rows in layout files. Scrolled-back chat keeps its context
  when the layout changes or new messages arrive.

- Added: Rearrange Playlist columns and customize Users rows in layout files.
  Wrapped rows and row padding keep mouse clicks attached to the right item.

- Added: Customize the dungeon frame, statistics, map, sidebar, and recent
  journal in layout files. Moving the sidebar keeps your expedition intact.


- Added: Restyle dungeon recovery, equipment rows, guide and journal text,
  and expedition endings through layout templates. Recovery labels and values
  wrap together and remain independently editable.

- Added: Customize log-viewer controls, dropdown options, and footer in layout
  files. Dropdowns follow their controls when you move them.

- Fixed: Layout auto-reload now works with relative directory paths and keeps
  watching when the override directory is created or replaced.

- Added: Customize chat timestamp, sender, action markers, message body, and
  day separators in layout files. Selection and spoiler clicks follow the layout.
- Fixed: Wide characters after a long chat prefix continue onto the next line
  instead of disappearing when the remaining space is too narrow.

- Added: Customize Recent Series, All Series, and The List in layout files,
  including episode and watcher columns, group headings, and the filter caption.

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

- Improved: Dungeon movement now uses spear reach automatically: move toward
  an enemy two tiles away to thrust without advancing. Equipped spears explain
  this on screen, and diagonals can pass a single wall beside you.
- Improved: Dungeon inventory separates ground items with a blank line, and
  resting no longer floods the journal with identical recovery messages.
- Fixed: Ash rats now have forelegs and paws in combat messages, and enemy
  attacks describe what the creature does and where it hits you.
- Improved: Dungeon wounds use descriptions such as "left foot scratched,
  bone damaged". Health details wrap, with a blank line before threats and
  a hint when more wounds are available under v. Scroll through long injury
  entries and treat the region on the highlighted line.
- Added: the log now says (at debug level) whether your `[dessplay]` mpv
  profile was applied, and reports any command mpv rejects — previously
  mpv's answers were discarded, so a typo'd profile failed silently.

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
