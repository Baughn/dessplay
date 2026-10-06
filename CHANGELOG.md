# DessPlay changelog

New features and fixes, grouped by calendar day, newest first. The client
embeds this file and shows unseen entries at startup; `/changelog` shows
the full history. Format: `## YYYY-MM-DD` headers in descending order,
`- ` bullets with an optional one-word `Category: ` prefix, continuation
lines indented two spaces. `cargo test` validates it. The list is
append-only; you cannot reword or remove existing entries.

Add new days at the top. Add new entries at the bottom of existing days.

## 2026-10-06

- Changed: after glancing at a chat line, Osaka gets back to what she was
  doing sooner, and lets a line go by if she can't look at it within a few
  seconds (say, while climbing or out of the room).
- Changed: when a chat line comes while Osaka is sitting, lying or
  lounging, she looks up where she is instead of getting up (and a doze
  just stirs).
- Added: Osaka blinks now and then while she sits, lounges, reads or gazes
  up, so a long rest looks restful rather than frozen. Gazing up, she says
  "ooh" only at first.
- Added: with no desk, Osaka does her homework lying on the floor, or
  tears off some text and builds a paper desk to kneel at (all the more
  once the floor has given her a sore back). With no bookshelf she reads
  lying on her back, and dozes under the book.
- Changed: she watches TV cross-legged when she isn't watching from a
  sofa.

## 2026-10-05

- Added: You can drag playlist entries with the mouse to reorder them.
  The entry follows the pointer and moves when you let go. J/K still
  work, and still reach rows you would have to scroll to.
- Added: In settings, you can drag media roots with the mouse to reorder
  them (the top one is where downloads go), and clicking a row selects it.

## 2026-10-04

- Fixed: The houseguest no longer forgets what happened just before you
  quit. A purchase she made or a parcel that arrived in your last moments
  is still there next time, and so are pane sizes you had just adjusted.
- Added: Osaka's days now pass while dessplay is open, about six times
  faster than real time. Her clock starts on a Monday at 4 in the
  afternoon, for a home she already has too.
- Added: Osaka keeps a daily routine on her own clock. She snacks and
  lounges in the afternoon, does her homework in the evening before a
  school day, and goes to bed at night: in her bed (or on her sofa, or
  the floor), turning the lamp off. Chat at night only makes her stir and
  murmur. In the morning she gets up with a stretch and a "Mornin'" that
  shows her mood for the new day. Visit her at night and she's already
  tucked in.
- Added: Osaka talks in her sleep now and then ("Mm... melon bread..."),
  and in the last hour before she wakes, it's "...five more minutes".
  With a fridge, some nights she pads over for a midnight snack in the
  dark, then goes back to bed. If a message arrives at night while
  you're scrolled back in the chat, she shuffles over half asleep, pokes
  it, and goes back to bed. A visitor who ends a night visit gets a
  sleepy blink goodbye instead of a startled one.
- Changed: Osaka's part-time job is on weekends (and school holidays)
  now, from 10 in the morning until 5 in the afternoon.
- Fixed: After her part-time job, Osaka always comes home with her
  shopping ("I'm home!"), even when she stumbles on her way back in.
  She no longer vanishes for minutes on a later trip off the edge of the
  screen, or says "I'm home!" without having left when something stopped
  her on her way to work.
- Fixed: When you came back while Osaka was out at work, or had just
  stepped through her door, she no longer appears for a moment to wave
  goodbye from where she left, and her door no longer blinks out
  before it fades away with the rest.
- Added: On school days Osaka goes to school in the morning. At a
  quarter past eight she says "I'm off!" and steps out through her pink
  door, which stays standing closed in her empty room, the lamp off,
  until she comes home through it after lunch ("I'm home!"). Start
  dessplay in school hours and her room is empty, door and all. A
  parcel that comes while she's out waits for her to get home.
- Added: Now and then on a school day, Osaka dashes home through her
  door for something she forgot: her lunch from the fridge ("Forgot my
  lunch!"), or, with no fridge, she stands there wondering what it was.
  Then she's off to school again.
- Added: If new messages are waiting while you're scrolled back in the
  chat and Osaka is at school, she pops in through her door to poke the
  chat for you, then heads straight back out.
- Fixed: Opening a menu or dialog while Osaka has dashed home from
  school, or while she's on her way to poke the chat, now sends her off
  at once, as on any visit, instead of her carrying on over it. If she
  was on her way to the chat, it still gets its poke.
- Fixed: When Osaka heads out by her door and her room isn't left
  standing (she has no furniture yet, or you're at the keys), her door
  and anything she moved now fade away instead of vanishing at once.
- Added: Osaka notices the real date. On New Year's Day she wishes you
  a happy New Year and watches the first sunrise on her TV; on Setsubun
  she throws beans ("Oni wa soto!"); and she has something to say on her
  debut day (April 8), Tanabata, the day they graduated (September 30),
  Halloween, Christmas and New Year's Eve. She says it once a day, on
  your first visit (or as she wakes, or when she gets home from school).
- Added: Osaka has her seasons. In exam season she does more homework
  and splits chopsticks before every bit of it, in spring she sneezes
  with hay fever, in December she muses about Rudolph and kotatsu, and
  in the last week of the summer holidays it's "Homework! Homework!".
- Added: Osaka greets the time of day: "Breakfast!" and "Dinner time~"
  with her first snack of a morning or evening, "No school today!" when
  she gets up on a weekend or holiday, and "Night-night..." as she turns
  the lamp off.
- Added: Every so often she does something rare, like dreaming about a
  certain orange cat, and if you haven't seen anything new from her in a
  while, something is sure to turn up. She remembers which ones you've
  seen.
- Added: A wall clock arrives for Osaka's home as a parcel, once she has
  a TV, and shows her time of day, its hands moving on each quarter of
  her hour. She can also buy a window after the cat bed: dawn, day, dusk,
  evening and night show outside it, with a little town below.
- Fixed: Osaka unpacks a cat bed that arrives in its box (it used to sit
  boxed for good, and she stopped shopping), and parcels arrive only
  where she can get to them to unpack them.
- Added: Osaka looks out of her window and says what she sees: the stars
  at night, "Pretty..." at sunset, the lights coming on of an evening.
  She does it most at dusk and after dark. With her wall clock where she
  can see it, she glances up at it at bedtime ("Oh! It's late!") and as
  she leaves for school ("Time for school!"), and now and then of an
  afternoon tells you the hour, roughly ("Three-ish.").
- Fixed: A chat message arriving just as Osaka gets to her fridge no
  longer sends her back out to school without the lunch she dashed home
  for, or back to bed without her midnight snack. She looks at the
  chat, then has it anyway.

## 2026-10-03

- Changed: The houseguest watches TV from her sofa only when the sofa is
  turned toward the TV. A sofa turned away is still for sitting and
  napping on.
- Added: The houseguest notices when something in her home isn't right,
  and grumbles about it while she's using it: "Can't see the telly..."
  from a sofa turned away from the TV, "Too noisy to sleep..." in a bed
  in the same room as the TV. She says it once a visit, and it stays on
  her mind while it's still wrong.
- Added: The houseguest puts right what she grumbled about. "Hup!", and
  the piece is in her pocket; she carries it where it belongs (upstairs
  or down, if need be), sets it down ("There!"), and sits back down to
  enjoy it, watching the TV from a sofa she has turned toward it. She
  does it once a visit, a few times when she's feeling busy, and never
  when she's feeling lazy.
- Added: Now and then the houseguest tries a piece in a couple of spots
  before she settles on one: she sets it down, sits on it with a
  thoughtful "hmm...", and keeps it there ("There!") or lifts it again
  ("Hup!") for the next spot. The calmer she is, the pickier.
- Added: A potted plant and a poster (a sunset with a whale's tail) now
  come on the houseguest's shopping channel once her room feels bare to
  her, or once she has all the furniture. The poster hangs on the wall,
  above her other things.
- Fixed: When the houseguest tries a lamp, a fridge or a TV in a few
  spots, they're different spots, not the same spot turned round; and
  when the other spot she meant to try turns out to be taken, she keeps
  the piece where it is with a "There!" instead of falling silent.
- Fixed: When something in the houseguest's home can't be put right by
  moving one piece, she gets on with the next thing she noticed instead
  of leaving everything as it is.
- Changed: The houseguest is less fussy about what makes a room. She'll
  move her sofa over to the TV even when her desk stands there too
  (the study becomes a living room), and a desk in her kitchen no
  longer bothers her. A bed beside the TV, or a fridge in her bedroom,
  still does.
- Fixed: The houseguest no longer stops in front of text to look at
  each new chat line. In a lively chat that could keep the text behind
  her hidden for as long as people kept talking. Now she steps
  somewhere clear first and watches the chat from there.
- Fixed: The houseguest's little moments land on cue. She nods off over
  her homework, shuts the fridge to eat her snack, yelps when the cat
  bites and says "There!" over a finished piece right when she gets to
  it, rather than up to a second or so late.
- Changed: Coming out of a door, the houseguest no longer always asks
  "Where was I?". Most times she says nothing; now and then she
  wonders how she got there, or that she's forgotten what she forgot.
- Added: Spacing out, the houseguest sometimes tells a riddle and
  answers it herself straight away, pleased with it: "Bread ya can't
  eat?" "A fryin' pan!" Within a visit, she doesn't tell the same one
  twice in ten minutes.
- Added: Now and then the houseguest flicks through the channels while
  she watches TV: snow, colour bars, snow, then a sunrise ("Ooh!"),
  and back to snow, humming, pleased with herself.
- Changed: Getting into bed, the houseguest lies a moment, thoughtful,
  before she drops off. With line-art graphics, her lamp stays on for
  that moment and goes off as she falls asleep.
- Added: Before her homework, the houseguest sometimes splits a pair of
  disposable chopsticks. A clean split earns a twinkle and a giggle; a
  bad one leaves her glum: "Hold 'em by the ends!"
- Added: After a snack, the houseguest sometimes finds a sata andagi in
  the fridge. She holds it up and says "Sata andagi." a few times,
  happier each time, then eats it. Ask her something in chat
  meanwhile (end it with "?") and she turns to the chat and answers
  "Sata andagi." without putting it down.
- Fixed: The houseguest no longer stops noticing IRC lines after the
  first hundred in a session.

## 2026-10-02

- Fixed: Something the houseguest bought off the shopping channel, or
  unpacked, is no longer forgotten when you press a key at that very
  moment and she leaves.
- Fixed: The houseguest naps on a sofa she made of text, rather than
  lying down on the floor beside it.
- Fixed: The houseguest uses what she makes. If a chat line interrupts
  her while she's crumpling text into a sofa or a bed, or on her way to
  it, she comes back to it afterwards, instead of leaving a finished
  sofa she never sits on.
- Fixed: A chat line no longer knocks the houseguest off a divider
  she's clambering over.
- Changed: The houseguest prefers to make a sofa where she can watch her
  TV from it, watches from her sofa even when the TV stands in a
  corner, and her furniture no longer vanishes into the closet when two
  pieces would overlap: they stand side by side.
- Fixed: The houseguest no longer sometimes stands over text that came
  up around her, hiding it, when there was room to move on.
- Fixed: Selecting a pane next to where the houseguest stands no longer
  flickers noise over its border.
- Added: The houseguest notices what she loses. A sofa she made that
  goes before she sat on it gets a glance and "...my sofa."; so does
  text she was tearing off when she was interrupted ("...never mind.",
  now and then), somewhere she was heading that's gone, and a piece she
  gives up on ("Nah."). Going back to something after an interruption,
  she sometimes says "Ah, right!".
- Changed: The houseguest finds a line of chat she was heading for even
  after it scrolls up a little, and may change her mind on the way
  when something else matters more.
- Changed: The houseguest lives in her home more. Sitting on her sofa,
  watching TV, reading and napping now answer how she feels (she gets
  comfortable, she wants some fun, she drifts into daydreams), so she
  uses her furniture rather than the floor, and varies what she enjoys
  instead of watching TV every time.
- Added: The houseguest arrives in a mood: some visits she's lazy
  ("Mm... lazy day.") and lounges about, some she's busy ("Okay! Let's
  tidy up!"), and now and then she's dreamy and spaces out a lot.
- Changed: The houseguest's rooms are whatever her furniture makes
  them, rather than one room per pane: a sofa and a TV make a living
  room and a bed a bedroom, wherever they stand, and pieces can share a
  pane. When a pane closes or gets too small, its furniture moves out
  together, into any pane with room for all of it.
- Added: The houseguest's parcels come in through a flap in the wall at
  the edge of the screen, pushing in beside it whatever already stands
  there.
- Changed: The houseguest watches TV from her sofa when the two stand
  a few cells apart in the same pane, even when something splits the
  floor between them; not from a sofa far across the room or pushed
  right up against the TV.

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
- Changed: If a chat message or other text appears where the houseguest
  is resting, she's startled and moves somewhere clear instead of
  covering it until she's done.

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
