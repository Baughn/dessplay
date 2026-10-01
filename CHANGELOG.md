# DessPlay changelog

New features and fixes, grouped by calendar day, newest first. The client
embeds this file and shows unseen entries at startup; `/changelog` shows
the full history. Format: `## YYYY-MM-DD` headers in descending order,
`- ` bullets with an optional one-word `Category: ` prefix, continuation
lines indented two spaces. `cargo test` validates it. The list is
append-only; you cannot reword or remove existing entries.

Add new days at the top. Add new entries at the bottom of existing days.

## 2026-10-01

- Added: With no sofa or bed of her own, the houseguest makes do: she
  tears the end off a line of text ("Rrrip!"), crumples it into a heap
  of shredded letters in the text's own colours, and sits on it,
  watches TV from it, or sleeps in it. The line keeps its gap until she
  leaves, or until you select that pane, when the text comes back at
  once. Once she owns the real thing she mostly uses that instead.
- Fixed: The houseguest no longer gets stuck going back and forth when
  she comes to poke the chat's scrollback accordion, and no longer naps
  on her sofa with text hidden behind her.
- Changed: If the houseguest's sofa is on the same floor as her TV, she
  sometimes watches the TV from the sofa.
- Fixed: The houseguest no longer idles or lies down right beside her
  furniture with nearby text hidden behind her picture.

## 2026-09-30

- Added: The houseguest can now stay while you watch. With F3 →
  Houseguest → Resident on (the default), she no longer leaves when the
  video starts or when you type: while you're using the client she
  keeps out of whichever pane you have selected (she rains out of it and
  steps through her door to somewhere else), she rarely wanders into the
  chat, and pressing any key undoes whatever she's been up to in the
  chat.
- Changed: The houseguest's settings have their own tab, F3 →
  Houseguest.
- Added: When you've scrolled up in the chat, the bottom of the chat log
  turns into a ╱╲╱╲ accordion showing how many new messages are below;
  click it to jump back down. If messages sit unread for a minute,
  Osaka comes over and pokes it until it shakes (it shakes on its own
  if her visits are off).
- Changed: In kitty-graphics terminals the houseguest now walks past
  text instead of treating it as a wall. What she passes in front of
  turns briefly into alien glyphs, and she never stops on top of it. She
  uses her magic door far less, and can now reach the chat's scrollback
  accordion to poke it.
- Changed: A chat message makes the houseguest stop and watch for 15
  seconds instead of a minute, so a lively chat no longer leaves her
  standing frozen.

## 2026-09-29

- Added: The houseguest is moving in. From her second visit she has a
  TV, and now and then Chiyo-chichi's shopping channel sells her
  something for her room — a sofa, a bed, a desk — which turns up on a
  later visit as a parcel she unpacks and then uses (naps on the sofa,
  sleeps in her bed, homework at the desk). Each pane she furnishes is a
  room, and her home is kept between sessions; F3 → Playback → Osaka
  moved out clears it.
- Fixed: The houseguest no longer leaves as soon as someone marks
  themselves ready; she stays until the video actually starts.
- Added: Once she has a home, the houseguest now and then goes off to
  her part-time job, leaving her room standing for a few minutes, and
  comes back with the shopping.
- Added: More for the houseguest's home: a floor lamp, a bookshelf, a
  fridge and a cat bed. She reads, raids the fridge when she's peckish,
  and on some visits there's a grey cat asleep in the cat bed. He does
  not like being petted.
- Fixed: The client no longer refuses to start with "bad houseguest"
  after her first visit. Her room starts over once, and the "houseguest
  arrives after" setting goes back to one minute.

## 2026-09-28

- Added: A houseguest. When nothing has happened for five minutes, Osaka
  wanders into the terminal and makes herself at home on the pane
  borders, drawn as line art in terminals with kitty graphics (Ghostty),
  and tidies up short chat lines by pulling them along. Press any key and
  she waves goodbye while everything rains back into place; a chat
  message only makes her stop and look. F3 → Playback → Houseguest changes the delay or turns her
  off.
- Added: The houseguest gets up to mischief: she sometimes swaps two
  letters of a word, giggles, and quietly swaps them back, and her
  sneezes knock a few letters loose, which she then puts back one by
  one.
- Changed: The houseguest can reach text a few cells away, reeling a line
  in before she pulls it, so she tidies and plays with chat more often.
- Added: The houseguest talks a little: she says hello when she arrives,
  "...I'm OK." after a tumble, and muses aloud while spacing out.
- Changed: The houseguest now follows her moods: she gets sleepier the
  longer she stays, restless after a while, keen to tidy when
  there's mess, and mischievous now and then.
- Changed: The houseguest now visits after one idle minute by default
  (was five).
- Fixed: The houseguest's hums and snores now float by her head when
  she lies down, not high above her.
- Fixed: The houseguest can still swap letters in a line after pulling it
  along, instead of running out of things to play with.
- Added: The houseguest gets around more. She clambers over the divider
  between panes, steps out at one edge of the screen and wanders back in
  at another, and when she's truly stuck she opens a pink door in space
  and walks through it.
- Fixed: The houseguest no longer stumbles and lands dazed as she walks
  in from the edge of the screen.

## 2026-09-26

- Changed: The oracle now knows when you switched episodes during the
  recent chat, so questions about the previous episode are answered
  about that one.

## 2026-09-24

- Fixed: Drift correction no longer makes an audible click when it starts
  or stops adjusting playback speed.

## 2026-09-23

- Changed: AI commentary now uses Claude Opus 5.5.
- Added: Ask the oracle in chat. Start a line with `oracle:` and a question,
  and it answers briefly, searching the web when needed and avoiding
  spoilers past the current episode. `Tab` completes `oracle: `.
- Changed: Seeders now tab-complete in chat like other users.

## 2026-09-16

- Fixed: A busy terminal no longer loses browser or search replies, leaves
  finished jobs on screen, or drops local chat and subtitle lines. Quitting
  bypasses pending display updates.

## 2026-09-15

- Changed: Chat search waits for a one-second typing pause before jumping to
  matches. Enter and result navigation apply pending searches immediately.
- Fixed: Terminal redraws use synchronized output to prevent partially painted
  frames from flashing during search, scrolling, and other screen updates.
- Fixed: Chat search no longer briefly blanks most of the pane when visiting
  recent matches. Resizing near the end of chat also fills the pane immediately.

- Changed: Chat now retains the latest 10,000 messages after daily cleanup,
  giving search and scrollback a longer history as new messages arrive.
- Improved: Searching and scrolling old chat stays responsive with long histories.
  Chat images load near your reading position with bounded memory use.

- Changed: Chat search shows results by relevance. Use arrow keys to select a
  match, then Enter to return to the conversation with that message centered.

- Improved: Installed clients clean up obsolete build files after updates while
  keeping the current binary, dependencies, and incremental compilation cache.

- Fixed: Live layout edits keep working on Linux when unrelated directories
  are unreadable, and recover after replacement of a parent directory.

## 2026-09-14

- Added: Nyaa searches show progress while inspecting torrent results.
- Added: Recent Nyaa searches are saved across restarts and can be recalled
  with Up/Down for editing.
- Added: Nyaa results have checkboxes: Space selects releases and Enter
  downloads the selected files concurrently. Tab returns to query editing.

- Fixed: Nyaa results now require Tab before editing the query, so typing or
  pasting cannot accidentally clear your checked downloads.

- Added: Ctrl-F searches every content pane, including chat, The List,
  subtitles, and logs. Slash also opens search in panes without a text field.
- Changed: Local searches accept fuzzy abbreviations and rank matching words
  above scattered letters. Chat search jumps between messages without losing
  your draft; list searches select a result back in its pane.

## 2026-09-13

- Added: Chat now shows when IRC users join or leave, including quits and
  kicks, with departure reasons when available.

## 2026-09-12

- Improved: Reduced CPU work and temporary memory use when updating the media
  library, checking for group activity, and building the episode browser.

- Improved: Reduced temporary memory use during playback, downloads, chat and
  series navigation, and periodic state saves.

- Improved: Reduced temporary memory use when laying out terminal panes and
  wrapping chat messages.

- Improved: Reduced playback CPU and temporary memory use with large media
  libraries.

## 2026-09-11

- Added: Click a chat image to view it fullscreen. Press any key or click again
  to return to chat.

## 2026-09-09

- Added: Make DessPlay's interface your own with local XML templates and CSS.
  Move or hide panes, rearrange table columns and chat fields, restyle forms
  and dialogs, or relocate the dungeon sidebar. Customize colors, borders,
  spacing, and text; edits apply live without rebuilding or losing your place.
  Run `dessplay layout init` to export the defaults and `dessplay layout check`
  to validate your edits. F12 opens layout tools and lets you restore the
  bundled layout if needed. Chat images now have a border aligned after the
  timestamp that scrolls with the image.
- Fixed: Installed launchers preserve your current directory, so commands like
  `dessplay layout init dirname` use paths relative to where you ran DessPlay.

- Added: Installed clients can choose master or stable in F3 → Account → Update
  track. Stable gets protocol changes and critical fixes with fewer rebuilds;
  your saved choice takes effect the next time you run the launcher.
- Fixed: When the launcher updates itself, it restarts before building so the
  new launch instructions apply immediately.

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
