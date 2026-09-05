# DessPlay Decision Log

Last updated: 2026-09-02

The reasoning behind the rules in [design.md](design.md): the failure that
motivated each one, the alternatives that were rejected, and the date it
was decided. design.md states *what* the system does; this file records
*why*, so the spec stays readable and the history stays available.

Sections mirror design.md's top-level sections. Each entry names the rule
and links back to the section that states it; design.md links here with
`(why: …)` pointers where a rule's reason is non-obvious.

**Workflow rule:** when a design change is made for a reason worth
remembering (a bug, a review finding, a user decision), the rule goes in
design.md and the reason goes here, in the same commit. Entries are never
deleted; a superseded decision gets a note saying what replaced it.

## User Experience

### Reset synced state has no confirmation

**Rule:** The Settings → Account "Reset synced state" row (and `/resync`) clears the local replica and restarts the client with no confirm step; see [design.md](design.md#settings-screen).

**Why:** The shared state is losslessly recoverable from the server (the restart re-adopts the server's copy), and local-only tables (watch history, hash cache, manual mappings) are untouched. A confirm step would guard against a loss that cannot happen; the deliberate act of typing the command or choosing the row is enough.

### Bare letter keys instead of Ctrl-modified letters

**Rule:** Root reordering in Settings uses bare `J`/`K` (not Ctrl-J/Ctrl-K), the playlist pane reorders the same way, and Series-pane filtering is gated behind `/` rather than a Ctrl chord; see [design.md](design.md#settings-screen) and [design.md](design.md#adding-files-to-the-playlist).

**Why:** Ctrl-modified letters collide with control codes in terminals without the enhanced keyboard protocol: Ctrl-J is LF, Ctrl-M is Enter. Bare letters are delivered reliably everywhere. Gating the filter behind `/` also keeps the bare `m` / `s` keys live in the Series pane while no filter is being typed.

### Series pane opens on The List

**Rule:** The Series pane's three modes cycle Recent Series → All Series → The List, and the pane opens on The List by default; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** The spreadsheet view is the day-to-day "what are we watching" surface, so it is what the user should see first.

### Episode browser season tree shape and dimming

**Rule:** The Episode Browser shows a franchise's seasons as a tree: the prequel chain in order with Sequel/Prequel-chained OVAs inline, side branches indented under the season they branch from and placed chronologically; a season whose every known file is watched renders dim; the cursor opens on the first season with an unwatched file; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** OVAs that AniDB chains as Sequel/Prequel stay inline because that is how the group watched them. Dimming uses watched-ness rather than The List's held-copy rule because a season nobody advertises right now is not *done*; the library index is the durable record of what exists, so "no one is seeding it tonight" must not read as "finished".

### Episode grouping and the any-copy watched rule

**Rule:** Episodes group by AniDB episode identity; a file with no parseable episode number never merges with an adjacent one; a multi-copy episode's header is muted when any copy is watched while copies keep their own marks; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Two files with no episode number sitting next to each other is no evidence that they are the same episode, so they are never merged on adjacency alone. A multi-copy episode counts as watched when any copy is because the group saw the episode, whichever encoding carried it; per-copy marks are kept so "which file did we actually play" is still answerable.

### Add browser opens on the selected entry's file

**Rule:** Pressing `a` in the Playlist pane opens the file browser on the selected entry's local file when it has one, else at the media roots; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Pressing `a` on the just-watched episode is the common way to queue the next one; opening beside it puts the next episode a keypress away.

### Browser sort by newest mtime

**Rule:** `Tab` toggles the add/map browser between alphabetical and newest-mtime-first; directories always stay alphabetical; the choice is persisted; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Files that just landed (a fresh download, a copy dropped in moments ago) float to the top under newest-mtime-first. Directories stay alphabetical because a directory has no single meaningful mtime. The choice is persisted for the same reason the All Series sort is: a sort preference is stable per user, not per session.

### Paste-to-add anchoring and in-place registration

**Rule:** A pasted single existing-file path adds it after the playlist's currently selected entry whichever pane is focused; the path is canonicalized; an out-of-root file is registered in place as a manual-mapping row; any other paste goes to the chat input; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** A drag lands wherever the cursor happens to be, and there is no use for posting a file *path* to chat, so the add is anchored to the playlist regardless of focus. Terminals produce several shapes on drag (bare, shell-escaped, quoted, percent-encoded `file://`), so each reading is tried in turn. Canonicalizing at the boundary keeps a relative or symlinked form from becoming a cwd-dependent registration. Registering an out-of-root add in place (rather than copying into the cache) makes it servable across restarts; the cost is that moving the file afterwards breaks it, exactly like a moved manual mapping.

### Control characters are stripped from pastes and inbound chat

**Rule:** Pastes drop control characters whether they land in a modal text field or the chat input; the chat display strips control characters from inbound synced and IRC-bridged lines too; see [design.md](design.md#adding-files-to-the-playlist).

**Why:** Typing can never produce a control character, so a copied value's trailing newline must not land invisibly in a settings field, nor in a chat message whose bytes would sync to every peer's terminal. Stripping inbound lines is the defensive half: a hostile or malformed remote message must not be able to write raw escape bytes into the terminal.

### A missing file from a known series blocks

**Rule:** A playlist file that is not found locally blocks playback (state Missing) when its series is one the user has watched before; an unknown series instead sets the user Not Watching; see [design.md](design.md#file-matching).

**Why:** If you have watched this series before, you probably should have this file, so the group waits rather than silently going on without you. For a series you have never touched, the opposite default is right: nobody expects you to have it, and the placeholder plus Not Watching lets the group proceed.

### Manual mapping path canonicalization and loss handling

**Rule:** A manually mapped (or dragged-in) path is canonicalized at the boundary; a mapping whose file has vanished falls back to normal matching and resolves Missing; a loss observed mid-session prunes the durable row, while a mapping absent at startup is kept unregistered and revives if the path returns; see [design.md](design.md#file-matching).

**Why:** Canonicalizing means the durable mapping row never depends on the working directory. Falling back to Missing is the honest state: the alternative is wedging on a phantom copy. The two loss cases differ in evidence: a failed serve or load mid-session proves the file is gone, so the row is pruned; a path merely absent at startup may be an offline mount, so the row is kept and the mapping revives when the path returns.


### Commitment keyed by List entry, not AniDB id

**Rule:** Per-series watch preference is keyed by `(UserId, ListEntryId)`; AniDB linking is enrichment only. See [design.md](design.md#user-states).

**Why:** A real number of series the group watches have no AniDB entry at all (obscure OVAs, doujin work, non-Japanese content, very new simulcasts). Hanging commitment on an AniDB id would make those series impossible to commit to. See also [Series Identity](design.md#series-identity) for why the per-file derived name was rejected as a key.

### Ctrl-R never commits to Watching

**Rule:** `Ctrl-R` / "mark ready" / `/ready` clears a pause or an auto-`NotWatching` back to Maybe, never to Watching. See [design.md](design.md#user-states).

**Why:** Watching is the one state that blocks the group across the user's absence. That commitment must always be opt-in through a deliberate act (`/watch`, the watch-cycle key, or a List entry's `watchers` set), never a side effect of the reflexive "I'm ready" key.

### Away is cleared by activity, not by typing

**Rule:** Any user may mark another Away; the mark is cleared only by the marked user's client attempting to unpause or sending a chat message, not by typing. See [design.md](design.md#user-states).

**Why:** Away exists for when someone walks off without quitting and would otherwise block playback forever. Clearing on *sending* rather than *typing* lets the marked user compose a message while still marked away. There is no permission system because the app is for five trusted friends.

### Marking others not-watching is plain LWW

**Rule:** `n` on a user in the Users pane or `/skip <name>` writes that user's series preference to NotWatching as an ordinary LWW write with `set_by: Some(actor)`. See [design.md](design.md#user-states).

**Why:** This is the "Kim tool": rule on someone's commitment to a show without waiting for them to show up, or acknowledging them file-by-file. A durable preference change keeps playback unblocked for the whole series. Plain LWW (the subject's own later write overrides it) was chosen over Away's special clear-by-activity rule because a series preference is a durable opinion, not a transient presence state. `set_by` carries the setter so the narrator can attribute the write ("by Baughn").

### Playable verdict is computed by the downloader

**Rule:** The downloading client computes whether the 20% window ahead of its playback position is complete and advertises `Downloading` or `DownloadingPlayable`; no other client recomputes the gate. See [design.md](design.md#file-state).

**Why:** The downloader is the only party holding both the chunk bitmap and its own player position. Publishing the verdict as an availability variant means every client derives the same gate with no arithmetic and no shared view of the bitmap. The window is approximate on purpose: time maps to bytes proportionally, and the 20% buffer exists precisely to absorb variable bitrate.

### In-place completion needs no reload; other channels always reload

**Rule:** A download that completes at its own cache path does not reload the player; a verified copy arriving via any other channel at the same path always re-issues the load. See [design.md](design.md#file-state).

**Why:** The download assembles in place, so completion is the same inode and the player's open fd sees the full content. A browse import placing a fresh inode over the same path is a different file as far as the open fd is concerned. Path equality never implies content identity.

### Phantom EOF on a sparse partial (2026-08-21)

**Rule:** A partial's EOF report is believed only when the last known position is within a few seconds of the entry's duration; a rejection re-arms EOF reporting and seeks back to the last position observed while the file was advertised playable, never seeking the same target twice without a position tick past it. See [design.md](design.md#file-state).

**Why:** Unfetched regions of a sparse partial read as zeros, so the player can report end-of-file mid-episode. The 2026-08-21 review found the rejection path could spin. The seek target is the last position observed *while advertised playable* because that is verified data by construction: the raw last position is the very offset that produced the phantom EOF, and after a user seek it is the data-less seek target itself. The in-place completion path re-issues the Load when a rejected EOF is outstanding because mpv parked at a phantom end under `--keep-open` never fires another EOF on its own.

### Unopenable partial retry gap

**Rule:** A partial the player cannot open is not offered again until ~10% more of the file has arrived; meanwhile the client advertises plain `Downloading`. See [design.md](design.md#file-state).

**Why:** The same bytes fail the same way, and repeated failures must not loop. This is the file-level analogue of the player-process crash ladder (see [Player Lifecycle](design.md#player-lifecycle)). Advertising `Downloading` rather than `DownloadingPlayable` during the deferral is deliberate: the client holds no playable video, so it should gate rather than let the group play on without it.

### Bitrate-vs-download-speed unpause rule dropped (2026-08-17)

**Rule:** There is no download-speed-vs-bitrate half to the Downloading gate. See [design.md](design.md#file-state).

**Why:** The original rule compared download speed to the file's bitrate before allowing unpause. Since the anchored download policy the situation comes up rarely, and the group decides by watching how fast the download percentage moves. Dropped in the 2026-08-17 usage triage.

### Paused is yellow, not red

**Rule:** A Paused user renders yellow in the Users pane although they block exactly like a red row. See [design.md](design.md#ready-states-ui-display).

**Why:** Yellow says "a friend paused", red says "something is broken". Both block; the colour distinguishes the deliberate from the faulty.

### Downloads are never shadowed in the Users pane

**Rule:** A peer downloading the now-playing file always reads as Downloading, whatever its derived state; the colour (green / blue / red) carries the rest. See [design.md](design.md#ready-states-ui-display).

**Why:** An in-progress download must never be hidden behind a Paused / Away / Not Watching label. The download is the thing to watch; red on a Downloading row says they still won't be watching right now. A present Maybe user displays exactly like Ready because both gate on their file state while present; the per-series distinction is shown in the playlist's watch tag instead.

### Waiting-for OSD shares the gating derivation

**Rule:** The "Waiting for …" mpv overlay is derived from the same gating derivation as the Users pane, sits in its own `osd-overlay` slot (top-right), is shown to everyone including the blockers, and is re-applied after a relaunch. See [design.md](design.md#ready-states-ui-display).

**Why:** Deriving both from one source means the overlay and the Users pane can never disagree. A separate overlay slot from the chat OSD means chat traffic never hides the blocker line.

### Auto-NotWatching suppressed when the file is obtainable

**Rule:** A missing file from an unknown series sets Not Watching, unless a present peer advertises the file Ready, in which case the client downloads instead. See [design.md](design.md#ready-states-ui-display).

**Why:** Writing a sticky Not Watching for a file the seeder is about to serve would opt the user out of a show they could have watched. The residual race (the source's Ready not yet synced when the decision fires) is accepted: it can set Not Watching once, the Downloading display masks it, and Ctrl-R clears it.

### Download progress visible without selection

**Rule:** Every playlist entry the client is downloading shows its percentage in the Playlist pane. See [design.md](design.md#ready-states-ui-display).

**Why:** Downloads mostly happen in the background (prefetch), so progress must be visible without selecting the file. This is an instance of the "no silent long-running work" UI principle.

### Absent Maybe users do not block

**Rule:** A Maybe user blocks only while present; Lost or Departed Maybe users never block. See [design.md](design.md#playback-rules).

**Why:** We don't hold up the night for someone who isn't here and only *maybe* wanted this. Watching (committed) is the deliberate opt-in for "wait for me even if I've been gone a week."

### Playback intent is a latch

**Rule:** Playback intent is a synced `LwwCell<PlaybackIntent>` that the server forces to Paused on any Lost, on graceful quit during playback, and on EOF-advance. See [design.md](design.md#playback-rules).

**Why:** The register is the latch that keeps playback paused after a blocker leaves instead of silently auto-resuming. The server forces Paused on *any* Lost, committed or not; gating then decides whether pressing play resumes (yes for an absent Maybe user, no for a committed one until acknowledged).

### Lost-to-Departed promotion does not re-pause

**Rule:** The timeout-ladder Lost->Departed promotion does not re-force Paused; only the graceful-quit immediate departure force-pauses. See [design.md](design.md#playback-rules).

**Why:** The peer was already paused at its Lost transition 30s earlier. Re-pausing at Departed would clobber a resume the present users legitimately made during the Lost window (an absent Maybe user is non-blocking, so such a resume is valid). Graceful quit skips Lost entirely, so it is the one departure that has not already paused.

### Acknowledge is a per-file set, not an Away

**Rule:** `/ack` records `(now-playing file, absent user)` in the grow-only `acknowledged_absent` set, cleared at compaction. See [design.md](design.md#playback-rules).

**Why:** The block should re-raise on the next episode and be re-acknowledged consciously each file. Reusing the per-user Away override would persist across episodes until the user returned. The per-file scoping is why this is a dedicated set.

### Drift correction hysteresis and rate limiting

**Rule:** Slew engages at 150ms and releases at 25ms; speed updates are quantized and rate-limited to ~1/s. See [design.md](design.md#playback-rules).

**Why:** A sustained pitch-corrected 2% slew is inaudible, but each speed *transition* is a broadband click. Corrections must therefore be few and long rather than frequent and brief, which is what the wide engage/release gap and the rate limit buy.

### Invalid user authority is never followed

**Rule:** A user seek authority is followed only when that user is present and advertises the now-playing file `Ready` or `DownloadingPlayable`; otherwise it is treated like Server authority. See [design.md](design.md#playback-rules).

**Why:** A user can hold seek authority without being on the real video, e.g. a not-watching client whose player shows a placeholder, which still reports a position. Following it would freeze the whole group on its bogus position. `DownloadingPlayable` counts because a downloader's partial is the real video, and when the whole group is downloading a fresh episode they are the only valid sources there are.

### Leader fallback under Server authority

**Rule:** Under Server authority (or an invalid user authority) each client follows the furthest-ahead present peer that has the now-playing file loaded. See [design.md](design.md#playback-rules).

**Why:** The Server has no position, and it holds authority for most of an episode. Without a fallback the player would run open-loop: any initial offset (a late-starting player, a brief decode stall) would sit uncorrected for the whole episode. Following the front means laggards catch up forward with no group rewind.

### Position file tag gates leader eligibility

**Rule:** Leader election and user-authority validation both require the peer's `PlaybackPosition` to carry a `file` tag equal to now-playing, in addition to `Ready`/`DownloadingPlayable`. See [design.md](design.md#playback-rules).

**Why:** `Ready` is set on prefetch, so a peer advertises Ready for next week's episode long before playing it. Right after a now-playing transition a peer can be Ready for the new file while its position register still holds the *previous* file's sample; forward-only leader election would latch the group onto that stale value instead of starting the new file at T=0. The tag is a clock-free identity check that excludes absent users, users on a different file, and users watching a placeholder, whose positions are stale or for another file. The tag is trustworthy at its source because the player actor attributes positions to a file only after mpv's path echo confirms the load (see [Events from Player](design.md#events-from-player)).

### Resume point on load

**Rule:** Every `Load` of the real now-playing video seeks to the furthest position any user, present or not, has persisted for this file. See [design.md](design.md#playback-rules).

**Why:** Drift correction follows only *present* peers, so it cannot restart a session that ended mid-episode: with everyone gone there is no leader, and a fresh client would sit at zero under Server authority. Positions are replicated CRDT state that outlives the users who wrote them, so the furthest one is available. Furthest-ahead matches the leader rule, so whoever loads later converges on the same point. The seek goes through the crash-restore path so it is programmatic, echo-suppressed, and never a `UserSeek`.

### Manual select does not mark watched

**Rule:** Manually selecting a different playlist entry loads it paused at the start and resets seek authority, but does not mark the abandoned file watched or advance The List. See [design.md](design.md#playback-rules).

**Why:** Selecting a different file abandons the current one rather than finishing it. The pause-at-start half mirrors the EOF transition so the group presses play when ready either way.


### OSD chat lines expire individually

**Rule:** each chat line on the player OSD stays a minimum of 8 seconds and expires on its own; see [design.md](design.md#chat).

**Why:** a burst of messages must never erase an unread line. A single shared timeout for the whole overlay would let the newest message push an older, still-unread one off the screen.

### Tab completion yields to pane cycling

**Rule:** `Tab` in the chat input completes a username prefix, and otherwise keeps its normal job of cycling panes; see [design.md](design.md#chat).

**Why:** completion should be invisible until it is useful. Reserving `Tab` for completion alone would cost the pane-cycling key for every keystroke that is not a name.

### Spoilers are a display concern

**Rule:** `||spoiler||` runs sync and archive as raw text; every display surface scrambles them; only the chat pane can reveal; the scramble is seeded by message identity; see [design.md](design.md#chat).

**Why:** Discord's `||...||` syntax was chosen for familiarity. Keeping the raw text on the wire follows the same rule as CTCP actions (only the display sites decode), so no message type or schema change is needed.

The scramble replaces characters class-for-class (letters and digits keep their class; CJK, emoji, arrows, and other symbols become letters) so nothing about the original leaks through the shape of the text. Seeding from message identity rather than an RNG keeps the scramble stable across repaints and identical between the chat pane and the OSD.

The OSD and IRC deliberately have no reveal: IRC is public, logged, and one group member's primary chat surface, so a reveal affordance there would hand the spoiler to exactly the people the sender hid it from.

### Summon decides client-side and matches nicks in the IRC actor

**Rule:** `/summon` decides "IRC bridge disabled" and "everyone's here" in the client, and the IRC actor performs the nick matching and sends the ping directly as a PRIVMSG, not via `Mutation::Chat`; see [design.md](design.md#chat).

**Why:** both early-exit conditions are already known client-side, so deciding them needs no round trip. Channel membership (from NAMES/JOIN/PART/QUIT/NICK) lives in the IRC actor, so that is where the edit-distance matching of absent usernames to live nicks belongs. The ping addresses specific nicks rather than broadcasting to the group, so it is not a chat message: it is not mirrored into the local chat log and not synced. Only the outcome (who was pinged) becomes a local system line.

### /me actions ride inline as CTCP ACTION

**Rule:** `/me` is carried inline in the message text as `"\x01ACTION …\x01"`; only display sites decode it; the action phrase renders grey; see [design.md](design.md#chat).

**Why:** using the CTCP `ACTION` convention inline means no separate message type and no schema change, and the wire form forwards verbatim to IRC. Terminals have no italics, so colour (grey) is what marks the emote in the chat log.

### /resync needs no confirmation

**Rule:** `/resync` (and the Settings → Account action row) clears the local synced state and restarts the client without a confirm modal; see [design.md](design.md#chat).

**Why:** typing the command is the deliberate act, and the shared state is losslessly recoverable from the server. Local-only tables (watch history, hash cache, manual mappings) are untouched, so there is nothing to lose that a confirmation would protect.

### IRC bridge motivation

**Rule:** each interactive client optionally mirrors its own chat into a shared IRC channel and surfaces plain-IRC messages back into the chat pane; see [design.md](design.md#irc-bridge).

**Why:** DessPlay logs are unavailable when the program is not running, so the chat is gone the moment the app is closed. An IRC channel is something others can keep open or log independently of the app.

### Dess suffix stays terminal

**Rule:** the IRC nick is `[Username]Dess`, and a collision retry keeps `Dess` as the suffix (`Baughn2Dess`); see [design.md](design.md#irc-bridge).

**Why:** the suffix is how *other* bridges recognize and de-duplicate bridged messages (see the inbound `*Dess` filter). A disambiguator appended after the suffix would defeat that recognition.

### IRC spoiler mask seeding

**Rule:** outbound `||spoiler||` runs are masked at the `Mutation::Chat` tap with a static scramble seeded from a per-process message counter, never from the message text; see [design.md](design.md#irc-bridge).

**Why:** the channel is public and logged, one group member reads chat *only* there, and IRC has no reveal affordance, so raw bars would hand the spoiler to exactly the people the sender hid it from. The mask is seeded from a counter because a plaintext-derived mask would let a channel lurker confirm a guessed spoiler by recomputing it. The consequence, that the IRC letters differ from the chat/OSD rendering of the same message, is harmless: nobody cross-checks them.

### CTCP actions are never split

**Rule:** long plain IRC lines are split at the 512-byte limit, but a `/me` CTCP action is never split; see [design.md](design.md#irc-bridge).

**Why:** chunking an action would break the `\x01` framing or emit several separate emotes for one action. Leaving an over-long emote to the server's 512-byte truncation is the conventional IRC client behaviour; this is intentional.

### Inbound IRC lines are local and Dess nicks are dropped

**Rule:** messages from IRC nicks not ending in `Dess` are shown locally and never synced; messages from `*Dess` nicks are dropped; see [design.md](design.md#irc-bridge).

**Why:** each client runs its own bridge, so syncing inbound lines would duplicate them once per client. `*Dess` nicks are other bridges echoing DessPlay users who are already present via CRDT sync. The heuristic cost, that a genuine IRC user whose nick ends in "dess" (e.g. `Goddess`) is also dropped, is accepted: the actor deliberately does not hold the roster.

### System messages are derived, not synced

**Rule:** the chat narrator derives system lines locally by diffing successive (state view, peer list) pairs; nothing is sent on the wire; see [design.md](design.md#system-messages).

**Why:** the underlying facts already live in the synced CRDT state or in the server's `PeerList`, so syncing the lines would carry no new information. Because every client diffs the same synced inputs, every client narrates the same lines, consistent without extra wire traffic.

The cost is that a late joiner does not see past events: a transition cannot be reconstructed from a snapshot that holds only the current value. That is acceptable. System lines are a real-time "what's happening now" cue, and the durable answers live elsewhere: the Users pane shows who is present now, and the playlist pane shows the full play history in muted colors. The two exceptions (the player crash, written as a real synced chat message, and the day separators, recomputed from persisted timestamps) exist precisely because those are the cases where a late joiner does need the information.

### Narrator attribution comes from the data

**Rule:** narrator lines attribute changes from map keys and `set_by` fields, never from who wrote the register; new-file lines are un-attributed; watch-preference lines are scoped to the now-playing series; see [design.md](design.md#system-messages).

**Why:** the resolved `StateView` does not record who wrote a register, so attribution has to come from the data. The now-playing writer is not recoverable at all (manual selection takes no seek authority), which is why new-file lines carry no name and EOF-advance is distinguished only by the prior file's watched flag flipping true.

Scoping watch-preference lines to the now-playing series keeps The List's bulk auto-writes for other series out of the chat. The series-preference value carries `set_by: Option<UserId>` (mirroring `ManualState::Away`) for exactly this reason: it is the only way to render "(by Baughn)" when one user rules on another's commitment.

### One narrator line per action

**Rule:** the narrator emits one line per user-meaningful action, not one per register written; server-forced pauses, drift corrections, and position samples never produce lines; see [design.md](design.md#system-messages).

**Why:** the server-forced intent -> Paused on Lost / departure / EOF is already explained by the corresponding lost / left / new-file line, so narrating it as a bare "paused" would be cascade spam. Drift-correction slews and automatic hard seeks never create a `UserSeek`, so they never produce a "skipped to" line; the 1500ms seek debounce coalesces a scrub into one authority write for the same reason.

### Day boundary at 09:00

**Rule:** chat day separators fall on a 09:00 local-time boundary, are computed at render time from persisted timestamps, and are never synced; see [design.md](design.md#system-messages).

**Why:** watch parties straddle midnight, and the small hours still belong to last night's session, so a "biblical" day boundary at 09:00 matches how the group experiences an evening. Making it a view concern rather than a stored event means a late joiner sees the separators too. The boundary is local-time and per-client by design: it is a reading aid, not shared state.

### Changelog is compiled in and day-grouped

**Rule:** `CHANGELOG.md` is compiled into the binary, grouped by calendar day, validated by a test, and shown as a "What's new" modal at startup; the first run skips the modal; see [design.md](design.md#changelog).

**Why:** users never read the commit log, so new features and fixes must be surfaced in-app. The format is Factorio-inspired, without the rigidity. Validating it in the test suite means a malformed entry fails the suite rather than the user; at runtime a bad file degrades to an empty changelog rather than a crash. The first run skips the modal because the user is in the settings screen and the whole history is trivially "unseen".

### What's-new modal swallows other keys

**Rule:** in the "What's new" modal, `Enter` or `Esc` dismisses and every other key is swallowed; see [design.md](design.md#changelog).

**Why:** the modal opens under the user's hands at startup. Both dismiss keys do the same harmless thing, so an accidental Enter costs nothing, and swallowing everything else keeps a startup keystroke from reaching a pane.

### Changelog seen marker lives outside the Settings struct

**Rule:** the `changelog_seen` marker is a bare settings key (`YYYY-MM-DD:count`), not a field of the typed `Settings` struct; see [design.md](design.md#changelog).

**Why:** settings saves round-trip the whole struct from the UI's copy, so a marker field would be clobbered by any unrelated save. The `:count` suffix exists so that entries appended to a day the user already saw are still surfaced later.

## Client Roles

### Primary seeder is colocated with the rendezvous server

**Rule:** The primary seeder runs next to the rendezvous server and connects over loopback; see [design.md](design.md#seeder-behavior).

**Why:** Serving a file then costs one trip over the NAS uplink per recipient. The relay-only transfer design (no client-to-client connections) depends on this arrangement being the common case; without colocation every relayed byte would cross the seeder's uplink and the server's uplink both.

### Seeders persist a hash cache but no settings

**Rule:** A seeder is configured purely by flags/env and persists no settings, but it does persist the hash cache and cache bookkeeping in a database, and it rescans its media roots once a day rather than once a minute; see [design.md](design.md#seeder-behavior).

**Why:** A seeder may hold terabytes, so re-hashing its store on every startup is a nonstarter; the hash-addressed cache lets it re-discover prior downloads by hash on restart without a media-root filename scan. Its store is big and stable, so a minute-cadence rescan would be wasted work. Settings are not persisted because seeders run as systemd services from a NixOS config and never show the settings screen.

## Presence

### Graceful quit skips Lost but keeps the commitment

**Rule:** `/quit` / Ctrl-C moves the user straight to Departed, keeps them listed like a timed-out peer, still gates if they are committed to the now-playing series, forces playback intent to Paused if playback was running, and hands seek authority back to the server at once; see [design.md](design.md#presence).

**Why:** Skipping the Lost stage is the *only* thing a clean quit buys over a silent disconnect. It does not waive a commitment: per User States, the group waits for a committed user even when absent, "Lost, Departed, or quit". Playback pauses because leaving mid-episode should not be silent. The server reclaims seek authority immediately because a clean quit is final, unlike a Lost that may recover.

### Known-offline users are one list and valid skip targets

**Rule:** The Users pane renders every known-offline user (within 30 days) as one dim italic list, with "last seen" ages, whether they left minutes ago or have not shown up today; `n` and `/skip <name>` work on any of them; see [design.md](design.md#presence).

**Why:** The server's in-memory registry only spans its own process lifetime, so a user who has not connected since the last server restart is otherwise invisible. Both kinds of absent user are equally valid `n` / `/skip <name>` targets, and the point of the list is to let the group rule on someone's commitment without waiting for them to reconnect. Unifying them avoids two lists that mean the same thing.

### Known-offline users gate for seven days

**Rule:** Clients synthesize a Departed interactive peer entry for each known-offline user seen within the last seven days before deriving playback gating, so a committed absent user keeps blocking across server restarts; past seven days the synthesis ages out; see [design.md](design.md#presence).

**Why:** Playback gating quantifies over peer entries, and the server's registry spans only its own process lifetime, so a server restart would silently waive every absent user's commitment. The seven-day horizon matches the commitment's own "wait for me even if I've been gone a week" phrasing. It also bounds how long a stale username can keep blocking — for example after a naming-convention change, since commitments are keyed by username. Identity aliasing was deliberately rejected as a fuzzy heuristic in the gating path; the bounded horizon plus the explicit dismissals (`/skip <name>`, Away, per-file `/ack`) cover the stale case instead.

## The List (Series Tracker)

### AniDB linking is enrichment only

**Rule:** A List entry's `anidb_series_id` provides episode metadata, franchise grouping, and the search modal, and is never a prerequisite for commitment or gating; see [design.md](design.md#the-list-series-tracker).

**Why:** A real number of series the group watches — obscure OVAs, doujin work, non-Japanese content, very new simulcasts — simply have no AniDB entry, and the design must not depend on them getting one. Commitment therefore keys on the entry's `ListEntryId`, never its AniDB link.

### next_ep is free text

**Rule:** `NextEpState.next_ep` is `Option<String>`, not a number; see [design.md](design.md#schema).

**Why:** Real spreadsheet entries include season prefixes, OVA names, and guesses ("12", "S3-05", "Sisters", "movie 5?"). A numeric field could not represent the data the group actually keeps.

### The List is never pruned

**Rule:** List entries are never compacted or removed; see [design.md](design.md#schema).

**Why:** It is a few hundred rows of text, and the history is the point.

### Unlinked entries carry their own identity data

**Rule:** Unlinked List entries resolve files through `local_aliases` and `manual_files`; a linked entry's AniDB series id is authoritative and skips both. The resolution order is deliberately stricter than the franchise-browsing and known-series heuristics; see [design.md](design.md#series-identity).

**Why:** Two mechanisms already turn a file into "a series," and neither is a safe foundation for group commitment (whether the group waits for someone across absence):

- AniDB's relations graph, for files with a series id — structural (sequel/prequel/etc.) and stable, but only exists for series AniDB knows.
- The per-file derived name (the AniDB-miss fallback's `series_hint`, or the bare filename otherwise), used for franchise-browsing's fallback grouping and personal known-series detection. This name is not stable enough to hang commitment on: it is computed per file, from that file's own directory context, and the group does not reliably keep every episode of an untracked show in one dedicated directory. Two files of the same show, one hinted from `Anime/ShowName/` and one sitting loose elsewhere, derive different names — silently splitting one show into two "series" for the one question that most needs a single stable answer.

So entries carry confirmed aliases (seeded from the first file's derived name, grown by hand) and explicit file hashes for outliers whose name parses into no alias at all. The browsing heuristics stay soft and unchanged because a mis-grouped franchise row is a browsing annoyance, but a mis-resolved List entry silently un-commits someone from a show they are actively watching.

### Commitment is per franchise, not per season (2026-08-28)

**Rule:** Resolution step 1 matches a file to any List entry linked anywhere in its structural-relations franchise; with several linked entries the canonical one answers and a user's commitment is the fold Watching > NotWatching > Maybe over all of them; see [design.md](design.md#series-identity).

**Why:** Proposal [2026-08-28-franchise-list](proposals/2026-08-28-franchise-list.md). `/watch` on season three should commit to the show, and a new season should never mint a second entry. Legacy per-season duplicates already existed, so the canonical-entry rule (human-created over auto-created, then deepest along the prequel chain, then lowest id) and the preference fold let them coexist without a migration. The one-hop check from each linked season's own relations row covers a brand-new season whose relations row has not landed yet.

### Bumping next_ep is certain; resolving it to a file is not

**Rule:** The server bumps `next_ep` from the just-finished file's episode number (AniDB for linked entries, filename-parsed for unlinked ones), but jumping to the next episode of an unlinked entry goes through the Episode Browser's candidate-ranked disambiguation view rather than queueing a guess; see [design.md](design.md#advancing-next_ep).

**Why:** Two distinct problems hide under "auto-advance," with very different certainty. Bumping the counter from the finished file has no real ambiguity: it is a fact about a file already confirmed watched, not a guess about one that has not aired yet. Finding *which* library file is episode `next_ep + 1` is the genuinely uncertain step for an unlinked series — there is no AniDB episode identity to match against, only heuristics. Rather than guess silently, the existing multi-file disambiguation UI ("several files claim the same episode number ... expand into a lightweight tree") is generalized from "several files, one confirmed identity" to "several candidate files, ranked by score, no confirmed identity." This is deliberately not a new kind of synced Playlist entry (no `Map<Ed2kHash, ...>` schema change); it lives entirely in the Series/List pane and the episode browser, exactly like choosing which copy of a linked episode to play today.

### One List row per franchise (2026-08-28)

**Rule:** Entries linked into the same relations component collapse into one List row showing the canonical entry, the union of commitment initials, and `available` if any member is; see [design.md](design.md#ui-integration).

**Why:** Proposal [2026-08-28-franchise-list](proposals/2026-08-28-franchise-list.md): commitment, recency, and progress are franchise-level facts, and per-season rows duplicated them. The recency sort likewise takes the newest watch of *any season in the franchise*, entry or not, so "the latest episode is in season three" still floats the row.

### Dim List rows are never reordered (2026-08-28)

**Rule:** Rows with nothing to watch render dim in either sort but keep their sort position; see [design.md](design.md#ui-integration).

**Why:** Proposal [2026-08-28-franchise-list](proposals/2026-08-28-franchise-list.md): a predictable order beats a partition that shuffles as files come and go.

### Watchable ignores who currently advertises a copy (2026-08-29)

**Rule:** A List row is watchable when `available` is set or any known library file for it is unwatched; whether some peer currently advertises a copy is not a condition; see [design.md](design.md#ui-integration).

**Why:** User decision 2026-08-29, matching the episode browser's season rows: the library index is the durable record of what exists, and a show nobody happens to be seeding tonight is not "nothing to watch".

### Curated short titles replace the official name; human edits win (2026-08-18)

**Rule:** A linked entry whose relations row carries a curated short title renders and alphabetizes under it instead of the official name, but only while the entry's name still equals the official title; a human-typed or edited name always wins; see [design.md](design.md#ui-integration).

**Why:** User decision 2026-08-17: save the space — "GochiUsa", not "Gochuumon wa Usagi Desu ka??"; the full name still lives in the edit modal and the episode browser. User decision 2026-08-18: a name a human typed or edited always wins, which is also the fix-it path for a bad curated pick.

### Short titles are AI-curated with a client-provisioned token

**Rule:** The server asks an Anthropic model for each series' short title, caches the answer forever, settles repeated declines as "no short name", and gets its API token pushed from one client's settings (`SetAnthropicToken`, protocol v12); see [design.md](design.md#ui-integration).

**Why:** The titles dump's kind-3 rows are lowercase search tags ("gochiusa s2", "s;g", "HnNKn") and only a quarter of series have one, so they cannot be read raw. The answer is trusted as returned because the human-name precedence above is the backstop; answers for series not in the batch are dropped as a sanity guard. Settling after a few declines means no series is billed indefinitely. The token is client-provisioned rather than server-configured so the settings screen is the whole lifecycle interface for rotating or removing the server-side credential, reusing the `anthropic_token` the commentary engine already stores.

## TUI Layout

### Explicit dark theme on true-color terminals

**Rule:** A true-color terminal gets an explicit app-wide dark theme with RGB semantic foregrounds; dim text is an explicit muted RGB, never SGR 2; limited-color terminals keep their own theme and the ten-color palette; see [design.md](design.md#ui-principles).

**Why:** Painting the whole alternate-screen buffer with a known background and mapped foregrounds makes text contrast deterministic instead of depending on the user's terminal theme. Dim text is materialized as RGB because the treatment of SGR 2 alongside explicit RGB colors varies between terminal emulators. The color capability is injected into the synchronous `Ui` rather than read from process-global terminal state so that tests do not depend on the terminal they happen to run in.

### Visible progress for every long-running operation

**Rule:** Anything that can take more than a moment shows visible progress while it runs, and the status bar shows the server link whenever the client is not connected; see [design.md](design.md#ui-principles).

**Why:** A user who sees nothing happen assumes nothing is happening, and retries. The progress overlay for playlist-add hashing is visually modal but captures no input so chat keeps working underneath. The server link is shown for the same reason: stale gating text ("⏸ paused") while the client silently fails to connect reads as a hang, and a dead handshake can take the full per-address timeout ladder before it gives up.

### Progress bar on its own terminal-wide row

**Rule:** The progress bar + time, the health metrics, and the suggestion slot share one terminal-wide row above the status bar, reserved before the column split; see [design.md](design.md#ui-principles).

**Why:** The bottom status bar carries the variable-width "waiting on ..." blocker text, which would shove the bar sideways as blockers come and go; a row of its own gives it the same placement in every subtitle mode. Reserving the row before the column split keeps the playlist's bottom border level with the chat input's.

### Health line exposes sync starvation on a live connection

**Rule:** The right end of the bottom row shows ▲/▼ throughput (QUIC plus torrent), rtt from datagram probes, and seconds since anything arrived from the server; see [design.md](design.md#connection-health-line).

**Why:** A saturated uplink can let BitTorrent drown CRDT sync while the QUIC connection stays nominally "connected"; this row is what makes that visible. The torrent engine's speeds are folded into ▲/▼ so the culprit of a saturated uplink shows even though that traffic never crosses the server connection. The rtt comes from the time-sync probes rather than QUIC's estimate because the probes are datagrams and reflect real path latency (bufferbloat shows up as seconds). The server broadcasts a `StateHash` every 30s unconditionally, which makes the sync-age field a zero-false-positive stalled-sync detector.

### Sync age reads sync ok until remarkable

**Rule:** The sync-age field renders as a static `sync ok` until 5s of silence during group playback, or the 40s warning threshold when alone or idle; display only; see [design.md](design.md#connection-health-line).

**Why:** A counting number draws the eye, and what counts as remarkable depends on how chatty the wire should be. During group playback peers' position datagrams arrive continuously, so 5s of silence is already notable. Alone or idle the only incoming traffic is two interleaved 30s heartbeats, so the age legitimately sawtooths toward ~30s and is only worth showing past the 40s threshold, where it colors anyway.

### Health level hysteresis

**Rule:** The displayed health level shows trouble immediately but must hold calm ~5s, stepping down through intermediate levels, before relaxing; see [design.md](design.md#connection-health-line).

**Why:** Without the filter a single quiet sample flickers the row from red back to dim.

### Suggestion slot hold, truncation, and precedence

**Rule:** A cleared advisor condition holds the slot ~30s but a disconnect clears it at once; under width pressure the metrics keep full width, the bar truncates next, and the suggestion is dropped rather than shown as an ellipsis; the slot's reservation is text-width-capped; precedence is warning/critical suggestion > live marquee > info suggestion > blank; see [design.md](design.md#connection-health-line).

**Why:** The 30s hold guards against threshold flicker. On a full disconnect the `link:` notice supersedes the suggestion, and a condition that persists across the reconnect simply re-emits. The health metrics keep their width because they are the row's reason to exist. A lone ellipsis conveys nothing, so a suggestion that does not fit is dropped. The text-width cap (occupant text plus the 2-space margins, the marquee included) means the bar never shrinks further than the middle actually needs. Commentary yields to health warnings because a health warning is the row's job.

### Commentary model and request shape

**Rule:** Commentary calls `claude-opus-4-6` with adaptive thinking at low effort, hardcoded, jittered ±15 s per tick; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** The feature is just for fun and explicitly a single-user gimmick. Adaptive thinking is the forward-compatible request shape; the deprecated fixed thinking-token budget is never used. The jitter keeps the comments from feeling metronomic.

### Persistent commentator with a 5% re-roll

**Rule:** The commentator persists across ticks and API failures, is re-rolled with 5% probability per tick, is not reset on a series change, and keeps a character card pinned to its home series; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** A quietly changing persona is funnier than a fresh voice every time. The voice deliberately follows the group to the next show — Hinamori Amu commenting on Grave of the Fireflies is an accepted (welcomed) outcome — until the dice or a client restart retire it. Pinning the card to the home series is what lets a carried-over voice know it is watching someone else's show.

### Commentary thread structure

**Rule:** Each commentator is a multi-turn thread: subtitle turns are cursor-delimited and speaker-attributed, episode identity is keyed by file, a re-roll cuts the thread and seeds the new one with prior comment text only, a ~10-turn cap force-re-rolls, sent history is append-only, and a per-thread screenshot-byte budget sends turns frameless once exhausted; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** Speaker attribution exists because a model that cannot watch the video needs the dialogue attributed; the ASS Name field is the same one the separate subtitle pane colors by. The cursor is the advisor ring's per-line sequence numbers so consecutive turns never overlap and a failed call does not advance it.

Episode identity includes the now-playing file because AniDB-unknown files all share one hint-derived series name and no episode number; without the file in the key an unlinked series' episode changes would never re-header or reset the comment seed.

Seeding a fresh commentator with the text of the current episode's earlier comments (never the images or subtitles behind them) lets the voice change without the conversation restarting from nothing.

The 5% re-roll keeps threads young in expectation, but its tail is geometric, so the hard cap at ~10 turns backs it up, going through the same fresh-thread path the dice take.

Sent history is append-only because the prompt cache matches on a byte-stable prefix. An earlier design instead stripped screenshots from turns older than the last two; that rewrote the cached prefix every tick and silently re-billed the whole thread at full price whenever frames flowed. With history frozen, the turn cap is what bounds the request body, and the screenshot-byte budget (two worst-case frames' worth) keeps accumulated frames from outgrowing the API's request-size cap.

### Commentary prompt caching by interval preset

**Rule:** The 2 min and 4 min presets set an ephemeral `cache_control` breakpoint on the final text block; the 10 min preset sets none; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** The Anthropic prompt cache's ephemeral TTL is 5 minutes. The 4 min preset exists precisely to duck under that TTL with jitter included (which is also why the settings ladder is 4:00 rather than 5:00). At 10 min the cache would be cold anyway, so the write surcharge is skipped.

### Marquee distribution and replay rules

**Rule:** Comments go through a generic synced marquee register; a pass is keyed by LWW stamp; a stamp from before the session's first snapshot is adopted as already-played; the text enters fully off-screen right; the UI ticks at ~100ms only while a pass animates; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** The register is deliberately generic so marquee sources beyond commentary can use it later. It persists in synced state until compaction, so without the pre-startup rule a freshly started client would replay last night's final comment on launch. The off-screen entry delay gives people time to notice motion and glance down before the sentence starts leaving. The faster tick costs nothing when idle because a tick only repaints when something moved.

### Commentary failures are silent and the log is loud

**Rule:** Every commentary failure is a log line and a skipped tick, never user-visible; the feature logs its enablement, requests, commentator, token usage, and replies at info; see [design.md](design.md#ai-commentary-the-marquee).

**Why:** A gimmick that speaks once every few minutes is otherwise indistinguishable from a broken token. Logging per-call cache read/write usage means "is the cache hitting?" is one grep away.

### Wheel scrolls only the focused pane

**Rule:** The mouse wheel scrolls a pane only when it is already focused, except the non-focusable subtitle pane, which the wheel scrolls whenever the pointer is over it; subtitle scroll-back is mouse-only; see [design.md](design.md#tui-layout).

**Why:** Touchpads emit wheel events by accident, so a graze must neither scroll invisibly nor steal focus. Subtitle scroll-back has no keyboard path because keyboard users scroll subtitles in Intermixed mode, where they share the chat log.

### Mouse actions have key equivalents except resize and selection

**Rule:** Every mouse action has a keyboard equivalent, except pane resizing and chat text selection; see [design.md](design.md#tui-layout).

**Why:** Resizing is rare and not something keyboard speed matters for. Chat text selection is mouse-native and uncommon enough to need no keyboard path.

### Chat selection copies on release with no copy key

**Rule:** Drag-selecting chat text copies to the clipboard on release (CLIPBOARD and PRIMARY on X11), a hidden spoiler copies as its scramble, and cross-message selections snap to whole lines in irccloud log format; see [design.md](design.md#tui-layout).

**Why:** Once mouse capture is on, the terminal's own selection needs Shift and knows nothing of panes, so the app provides its own. There is no copy key because the terminal owns Cmd-C and Ctrl-C stays Quit. PRIMARY is written for the terminal-user reflex of middle-click / Shift-Insert. Copying the scramble rather than the hidden text is WYSIWYG and cannot leak a spoiler. Day separators are render furniture, so they are skipped. Widening a partial selection to its whole line before extending follows the gdocs convention.


### Live diagnostics and indexing reasons (2026-09-05)

**Rule:** F11 shows a bounded live log over the upper two-thirds of the screen,
with independent, session-only DessPlay and dependency logging levels. Default
logs explain every cache-backed hashing decision with old/new metadata; see
[design.md](design.md#diagnostic-logs).

**Why:** Dagger reported a recurring indexing notice without debug logging
enabled. Counts and paths could show the repeated work but not why the cache
was rejected. The decision point must record its evidence at info level, and
hash failures must be visible before the next attempt. This adds diagnostics;
it does not infer or fix the cause of that report.

An in-app tail lets a player inspect current activity and enable detail without
restarting. Keeping the bottom chat lines visible preserves party context.
The same formatted stream feeds the daily file and memory, so changing the
level captures the evidence for later inspection too. Bounds on retained lines,
bytes, and individual displayed events keep trace logging from growing memory
without limit; disk files retain complete events under the existing rotation.
A stable line identity prevents scrollback moving under the reader during
appends or eviction.

Separate workspace and dependency scopes let a player enable application trace
without enabling every networking dependency. Overrides last only for the
session (the user's preference), and Startup restores each scope's original
filter independently, preserving target-specific RUST_LOG settings. Polling the
buffer revision on the existing UI idle tick avoids a log-event channel flooding
the UI queue or tracing its own delivery indefinitely.


## Network Protocol

### Compaction hour and server placement

**Rule:** compaction runs daily at 12:00 UTC by default, and the rendezvous server is colocated with the primary seeder on the NAS; see [design.md](design.md#rendezvous-server).

**Why:** 12:00 UTC was chosen to be far from watch-party hours, so a compaction (which bumps the epoch and makes every client re-adopt a snapshot) never lands mid-episode. The NAS placement matters because the transfer design is relay-only: with the seeder on loopback next to the server, serving a file costs one trip over the NAS uplink per recipient, and the relay-only design depends on that arrangement being the common case (see Client Roles, Seeder Behavior).

### UI animation on a local monotonic clock

**Rule:** all state timestamps use the shared clock, but the TUI's animators run on a local monotonic clock and use shared/wall time only for display and message identity; see [design.md](design.md#time-synchronization).

**Why:** both the wall clock and the shared clock can step backward — NTP corrections on the wall clock, and a later time-sync round shrinking the computed offset on the shared clock. An animator driven by either would stall or jump on a step. A monotonic clock cannot step, so animation timing is immune; the shared clock is still the right identity for messages and the right value to display (see ui-architecture.md).

### LwwCell instead of crdts::MVReg

**Rule:** every replicated register is `LwwCell<V>`, DessPlay's own max-merge last-writer-wins register; see [design.md](design.md#state-sync-protocol).

**Why:** `crdts::MVReg` proved non-convergent when nested inside `crdts::Map` — replicas could end up holding different values after the same set of merges. A max-merge LWW register over the shared clock converges by construction. Details of the `Lww<V>` design and the failure are in sync-state.md.

## File Management

### Scan hashing yields to transfers

**Rule:** while transfer traffic is active, library-scan hashing defers and resumes ~10s after the traffic goes quiet; the stat-only walk continues; a walked file whose name matches an unmet playlist entry is hashed immediately regardless; see [design.md](design.md#media-library-scanning).

**Why:** scan hashing is bulk disk work with no deadline, while transfers are latency-sensitive — a source that serves nothing for 30s is snubbed by the requester (network-design.md). Letting a rescan compete for disk bandwidth mid-transfer risked exactly that snub. The exemption exists because without it an active download would defer the discovery of the very local file that makes the download redundant ("a local copy trumps the download"): the file would sit on disk, unhashed, while the client kept fetching it from a peer.

### Vanished roots and the removed-root grace period

**Rule:** a root none of whose indexed files exist is marked vanished and its hashes are retained indefinitely; a root with at least one surviving file has its missing rows pruned at once; a removed root's index rows survive for a seven-day grace period; see [design.md](design.md#media-library-scanning).

**Why:** the all-files-missing case is taken as removable storage being disconnected (an unmounted NAS or external drive), not as a mass deletion. Pruning those rows would force a full re-hash of a possibly multi-terabyte store on remount, so the hashes are kept and the root simply hidden until any recorded file returns. When at least one file still exists the root is demonstrably online, so missing siblings are genuine moves or deletions. The seven-day grace on an explicitly removed root exists only so that re-adding the identical path is cheap (no re-hash); it is bounded so a root the user really did abandon does not keep its index rows forever. A configured-but-vanished root never expires because the user has not removed it — it is merely unplugged.

### Library-wide lookup requests and AniDB load

**Rule:** every indexed hash lacking metadata is inserted into the `lookup_requests` GSet with its mtime and a directory-derived `series_hint`; see [design.md](design.md#media-library-scanning).

**Why:** the scan has each file's path and mtime in hand anyway (the mtime keys `hash_cache`; the path yields the directory hint), so carrying both costs nothing and gives the server the inputs it needs for the AniDB-miss fallback name and for the age-anchored re-validation ladder. Feeding the whole library, not just the playlist, into the lookup set is what lets the franchise browser span the group's collective collection. The AniDB budget stays bounded even when several clients index overlapping collections because the server de-duplicates per hash and the `anidb_queue` table records what has already been checked across clients.

### Resolution reads the index instead of hashing candidates

**Rule:** file matching looks the entry's hash up in the library index, falls back to a single exact-basename search, and otherwise marks the entry Missing and waits for the hash to enter the index; it never walks the disk hashing candidates; see [design.md](design.md#file-matching).

**Why:** the scanner builds and maintains the index ahead of demand, so resolution can assume it already exists; a copy the index has not yet absorbed is picked up by the wanted-set check when the scan reaches it. The basename search exists only to close the gap between "the file just appeared" and "the next scan pass" (a copy dropped in moments ago). It is an optimization: if it misses, nothing is lost but a minute. Hashing every candidate on demand would turn each playlist add into a disk walk, and was rejected.

### Cache reconciliation and the orphan sweep

**Rule:** at startup the file actor reconciles `cache_entries` against disk — pruning rows whose file is gone or mis-sized, re-registering survivors as servable — and deletes hash-named cache files with no row that are older than a week by mtime; see [design.md](design.md#download-cache-and-retention).

**Why:** the `cache_entries` table is an index over the cache, not an authority: a user may delete, move, or truncate files behind the app's back, so the filesystem has to be the source of truth. Pruning a stale row makes the playlist entry honestly re-resolve to Missing and re-download instead of advertising a copy that does not exist. The orphan sweep exists because eviction only iterates `cache_entries`: a hash-named file with no row is invisible to it and would leak forever. An orphan is either a completed download whose bookkeeping was lost (a DB reset leaves the files but not the rows) or an abandoned peer-download partial (`download_path` is the final `<cache>/<hash>` path, so an interrupted download leaves one). The one-week threshold matches the "in-flight downloads don't survive restarts" contract while leaving anything recent alone, since it may still be in flight or wanted.

### Serve-time answers: nothing versus CannotServe

**Rule:** a solicitation for a file the session has not registered is recovered from the library index; if nothing on disk backs the advert the holder answers nothing and retracts its Ready; `CannotServe` is sent only for a definitive identity mismatch; see [design.md](design.md#download-cache-and-retention).

**Why:** after a restart, Ready is durable synced state while the servable set is rebuilt lazily, so an unregistered-but-held file is the normal post-restart condition, not a loss — a live, visible index row bearing the hash is a genuine copy and can be adopted on the spot. The requester treats `CannotServe` as a denial that lasts as long as the advert that earned it stands (it drops the source rather than re-asking forever, network-design.md). Answering a transient "not right now" with it would therefore permanently remove a source that was merely slow to register; answering nothing lets the requester's ordinary source refresh drop and later re-add the holder.

### Eviction rules: unreferenced files and the adoption gate (2026-08-21)

**Rule:** a cached file is evictable once watched or once no playlist entry references it; eviction passes run at startup and on EOF-advance, never touch now-playing or queued unwatched entries, and do not run until a synced state has been adopted this session; see [design.md](design.md#download-cache-and-retention).

**Why:** the "unreferenced" clause exists because an abandoned download must not pin cache space just because nobody happened to watch it. The adoption gate came out of the 2026-08-21 review: before a synced state is adopted, the replica is transiently empty — a first run, or the window after `--reset-sync`/`/resync` before the connect handshake — and an eviction pass planned from that view protects nothing. It deleted cached media the real playlist still referenced, including the now-playing file. Gating on the sync actor's `adopted` watch makes the pass wait for a view that actually describes the playlist.

### Archive layout has no Season level

**Rule:** archiving produces `[Series name]/[Original filename]` (or just `[Original filename]` with the subdirectory setting off), with no `Season #` directory; see [design.md](design.md#download-cache-and-retention).

**Why:** AniDB models each season as its own anime (a franchise member), so a single series name already denotes one season's folder. A separate season level would either duplicate that or invent a numbering the metadata does not carry.

### Auto-archive trigger and ordering

**Rule:** auto-archive fires on the personal 85% watch record, not the group watched flag; a file watched off a partial archives at download completion; it always precedes the EOF-advance eviction pass; the policy is owned by the file actor; see [design.md](design.md#download-cache-and-retention).

**Why:** the group's watched flag (the `w` key, EOF-advance) is the group's history, not this user's viewing — a user who skipped an episode should not have it archived into their library. The partial case is covered because a file watched off a still-downloading partial only becomes a cached download when the download completes, so that is the first moment it can be archived. Because the personal record fires at 85%, auto-archive necessarily precedes the EOF-advance eviction pass, which is what makes `cache_retention: 0` and auto-archive compose (watched files are moved, never deleted). The file actor owns both the subdirectory layout and the auto trigger, pushed on settings save, so the manual `A` path and the automatic path can never disagree about the destination.

### Archiving an open file without a reload

**Rule:** a same-filesystem archive renames inline; a cross-device archive copies in a background task with the cache copy servable until the copy lands; the session updates its resolution and loaded path without reloading the player; see [design.md](design.md#download-cache-and-retention).

**Why:** an archive can move a file the player currently has open. A multi-gigabyte cross-device copy must not stall serving mid-session, hence the background task and the still-servable cache copy. The player is not reloaded because a reload at the 85% mark would be a visible hiccup for the viewer; the bookkeeping is still updated because a stale resolution would send a later rewatch to the vanished cache path.

### One adoption seam for local copies

**Rule:** every channel through which a local copy turns up — resolve, scan adoption by hash, a completed browse import, and a manual mapping — funnels through one adoption seam that cancels the redundant peer download; a manual mapping joins the seam only once its background hash confirms the content; see [design.md](design.md#download-cache-and-retention).

**Why:** a file being fetched from peers can land locally through another channel mid-transfer (a bittorrent download racing the prefetch, a copy dropped into a media root). If each channel cancelled the download on its own, one would eventually forget to, and the client would keep fetching bytes it already had. A single seam makes the cancel unskippable. The manual mapping is the exception on purpose: it is filename-trusted for the user's own playback, so it resolves Ready immediately, but a mapping to a different encode must never cancel a good download — the download is still the only route to the real bytes — so adoption waits for the hash to prove the content matches. A browse import cancels before placing its payload because both share the same hash-addressed cache path, and an import of a file already held under a media root finishes against the library copy so as not to demote a permanent library file into a retention-evictable cache row.

### Prefetch anchored at now-playing

**Rule:** a downloading client wants every unwatched playlist entry plus now-playing; fetch order is anchored at now-playing (ahead nearest-first, then behind nearest-first) at the chunk level; watched entries ahead of now-playing do not prefetch; NotWatching series are not auto-downloaded; seeders fetch everything with watched back-catalog last; see [design.md](design.md#download-cache-and-retention).

**Why:** the goal is that next week's episode — and the whole queue behind it — is local before the session starts. Anchoring at now-playing puts the bytes that will be needed soonest first. Applying the order at the chunk level rather than per file means the per-source request window is one shared budget: a now-playing advance or a playlist edit re-targets running transfers within a tick, with no cancels and no restarts. There is no point fetching a show the user has opted out of, so NotWatching series are skipped (a local NotWatching file still loads; you can mute).

### BitTorrent is browse-only (2026-08)

**Rule:** BitTorrent serves only the Playlist pane's explicit browse search; missing playlist files are never fetched by torrent; nothing torrent-related survives a restart; see [design.md](design.md#bittorrent-downloads).

**Why:** an earlier torrent-first automatic fetch path was removed in 2026-08 once the relayed peer transfer matured. For rare files, a manual search plus a full BitTorrent client covers the gap. DessPlay is not primarily a torrent client, and keeping the footprint session-scoped keeps it from behaving like one.

### Torrent setting: default off, asymmetric lifecycle, no seeder path

**Rule:** `torrent_enabled` defaults off; enabling applies at startup, disabling applies immediately (seeding torrents removed, pending imports cancelled, completed cached copies untouched); seeders run no torrent path; see [design.md](design.md#bittorrent-downloads).

**Why:** the engine opens ports and joins the DHT, so it must never start unless the user opted in. Disabling has to apply immediately because it is the mid-session escape hatch for a saturated uplink — torrent traffic can drown CRDT sync, and the connection-health line's suggestion points the user here. Enabling only at startup is a simplification: the engine is constructed once or never. Completed imports are untouched because they were hardlinked into the hash-addressed cache at verification and are ordinary cache files by then. Seeders have no torrent path because the browse import is an interactive feature, and a file nyaa can supply makes the seeder redundant; the seeder's job is the rare, peer-only files.

### Session-only torrent seeding

**Rule:** a completed import seeds from its import directory until the app closes, the cached file is evicted, or the setting is disabled; seeding never resumes on the next launch; nothing about a torrent is recorded in SQLite; `<cache>/torrents/` is swept at startup; see [design.md](design.md#bittorrent-downloads).

**Why:** a session typically lasts long enough to clear a 1:1 ratio on a release worth importing, so seeding for the session is a fair contribution without persistence. "The video player is seeding last week's torrents" is unexpected behavior for something that is not primarily a torrent client, so seeding deliberately does not resume. Running the engine with no persistence and no SQLite rows is what makes the startup sweep safe: the only directory spared is one still hosting a registered cache file (the rare failed-hardlink fallback).

### Local-copy offer: evidence classes and trigger (2026-08-31)

**Rule:** with auto-download off, a now-playing file that resolves Missing offers same-episode and near-name local copies for manual mapping; the trigger is derived from state, fires once per file per session, and defers the unknown-series auto-NotWatching write while open; see [design.md](design.md#bittorrent-downloads) and [the proposal](proposals/2026-08-31-local-copy-offer.md).

**Why:** the motivating case is two valid encodes under one filename — a hash mismatch, not just NotFound, so both count as Missing here. The "same episode" class reuses the episode browser's copy-grouping equivalence, `(series id, parsed episode number)`, because there is no AniDB episode id in the schema. The "name match" class is guarded by the filename episode parse because raw Levenshtein rates `- 01` against `- 02` at distance 1 and would happily offer the wrong episode. Deriving the trigger from state rather than hooking the advance event is what makes every arrival channel land on it (EOF advance, manual select, startup with the file already missing, a mapping pruned mid-session) without each needing its own hook. The auto-NotWatching write is deferred while the offer is open so the user is never marked NotWatching underneath a dialog asking whether they want to watch their own copy.


### Directory hint as the AniDB-miss series name

**Rule:** When AniDB does not know a file, the fallback series name is the requester's title-like containing-directory `series_hint`, else the filename stem; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** Without the directory hint, per-episode filenames each parse to a distinct series name, so a series' AniDB-unknown episodes split into one franchise per episode instead of grouping under the folder they share. The hint is computed client-side (only the client has the path) and stored once per `anidb_queue` row as the first non-null value reported.

### Re-validation ladder anchored on file age

**Rule:** The never-seen re-validation ladder's age is the *older* of the row's `first_seen` and the file's mtime; clients supply the mtime and the server only ever lowers it; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** Anchoring on `first_seen` alone keeps files owned for years on the aggressive new-file cadence: a queue reset stamps every long-owned unknown file with a fresh `first_seen` and re-polls it every 30 minutes indefinitely. The file's own mtime is the honest age. A request without an mtime (a playlist add from a client that doesn't hold the file) must never *raise* the stored value, or a later hint-less request would undo the anchoring.

### Startup reconciliation of settled AniDB rows

**Rule:** At startup the AniDB worker re-arms any `anidb_queue` row marked `has_data` whose hash has no metadata in the loaded CRDT state; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** The queue attempt (settled, re-check in a week) is written to SQLite at once, but the metadata write lands only in the periodically-snapshotted CRDT state. A restart in that window loses the metadata yet keeps the settled queue row, orphaning the file (no metadata, no retry for a week). NoData rows are left alone because they self-heal on their short ladder anyway.

### Directory-hint reconciliation each worker pass

**Rule:** Each worker pass rewrites a filename-derived `series_name` to the row's learned `series_hint` when they differ, without an AniDB call; real hits are never touched; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** The fallback name is written once, at the first lookup, using whatever hint the row holds *then*. But the hint can arrive after that write: a playlist add carries no hint (the client may not hold the file) and races ahead of the hinted library scan, so the first-seen episode of a series could be frozen with its per-episode filename stem and split into its own franchise. Reconciling on every pass, independent of the settled lookup schedule, closes the race; skipping names that already match makes it quiesce.

### Only structural relations merge a franchise

**Rule:** Only sequel/prequel, alternative-version, and side/parent/summary/full-story edges (`RelationKind::groups_franchise`) merge two series into one franchise; crossover and shared-universe edges are ignored; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** Crossover and shared-universe edges — same setting, shared characters, music videos, AniDB's catch-all crossover code — link related but *separate* works. Without the filter a single crossover like *Isekai Quartet* (which relates to Overlord, KonoSuba, Re:Zero and Youjo Senki) would collapse every show it touches into one giant component.

### Name search through the titles dump

**Rule:** The AniDbSearch modal is answered from a locally stored copy of AniDB's daily anime-titles dump (case-insensitive substring over all titles and synonyms, ranked exact > prefix > substring, one hit per series), as plain wire messages rather than CRDT state; see [design.md](design.md#parsing-files-to-seriesseasonepisode).

**Why:** The UDP API has no multi-result search — `ANIME aname=` is an exact-title lookup, useless for informal names like "GochiUsa". The titles dump is AniDB's sanctioned approach for name search, at most one download per day. Search results are transient request/response data with no reason to be replicated.

### Drag-in adoption is filename-trusted

**Rule:** A file the user loads directly into mpv whose basename matches the now-playing entry is adopted as a manual mapping with no hash check; see [design.md](design.md#manual-file-mapping).

**Why:** It is the same "the user explicitly chose this file" exemption the browser map gets (see Content Hash). The trade-off — a same-named *different encode* dropped in silently desyncs that client from the group — is accepted for parity with the browser map and because the user deliberately loaded that exact file. A mismatched mapping is still never *served* (`CannotServe`), so the damage stays local. The route is especially handy in attach mode, where driving mpv directly, including dragging files in, is the normal workflow.

### Mismatch re-check watcher

**Rule:** A name-matched file that fails the hash is polled for `(mtime, size)` about once a second and re-resolved once it has changed since the failed hash and then held still; an unchanging mismatch is never re-hashed and its watch expires after 10 minutes; see [design.md](design.md#content-hash).

**Why:** A name-matched file that fails the hash is usually a copy or external download still being written into a media root — the hash ran mid-write. Watching the path flips the entry to Ready seconds after the write finishes rather than at the next library scan a minute later. A genuine different encode never changes on disk, so re-hashing it would be wasted work; its hash-cache row still matches the disk, and the periodic scan remains the long-tail safety net.

### Personal and group watch records are separate

**Rule:** Personal watch history (local SQLite, 85% rule) and the group's synced watched flag (server-written at EOF) are tracked separately and used for different things; see [design.md](design.md#watch-tracking).

**Why:** The personal record is keyed by hash/series so it survives cache eviction, and it answers per-user questions: recency sorting, unwatched filtering, which *copy* of the previous episode this client played, known-series detection, and the auto-archive trigger. The group flag is the shared answer to "where are we?": play-history muting, "behind the group" eviction, and The List's position — so a user who misses a session still sees the group's progress, not their own.

## Player Integration

### keep-open always and autoload disabled

**Rule:** mpv is launched with `--keep-open=always` (not `yes`) and `--script-opts-add=autoload-disabled=yes`; see [design.md](design.md#player-lifecycle).

**Why:** `always` parks the file at EOF regardless of playlist length. User scripts such as autoload.lua pad mpv's playlist with sibling files, and `yes` would auto-advance into one, hijacking end-of-file and skipping the group forward. The script-opt additionally switches autoload off so stray playlist-next keys typed into the mpv window find nothing. The user's mpv.conf is otherwise honoured (no `--no-config`).

### Crash ladder escalation

**Rule:** Player deaths within 30s of each other escalate: relaunch silently, then pause globally with a shared chat message, then give up until a different file is loaded; see [design.md](design.md#player-lifecycle).

**Why:** A file that reliably kills the player would otherwise loop forever, spamming the log and re-pausing on every death — hence the give-up step, with a different now-playing as the deliberate recovery action. The second-death notice is a real synced chat message rather than a derived system line because a crash is the one state change peers cannot derive from their own view (they have no signal for *another* user's player dying), so it must be communicated; being an ordinary chat message it also persists and reaches late joiners. The relaunch after the second death comes up paused because that is the safe state if the file itself is crashing the player.

### Observed pause re-anchors the position estimate

**Rule:** An observed pause is followed by a `get_property time-pos` query whose reply re-anchors the position estimate on the frame mpv actually stopped at; see [design.md](design.md#events-from-player).

**Why:** Otherwise the wall-clock-extrapolated estimate counts the observation's in-flight window as phantom playback, and a paused mpv emits no further `time-pos` changes to correct the overshoot.

### File attribution gated on the observed path

**Rule:** File-attributed observations (position, seek, EOF, duration) are accepted only while the last observed `path` equals the commanded file, and drift correction is suspended while the player is off that file; load-failure reports and programmatic-seek echo accounting are exempt; see [design.md](design.md#events-from-player).

**Why:** `loadfile` is asynchronous: after a load is commanded, mpv stays on — and keeps reporting positions, seeks, even the EOF of — the *previous* file until the new one actually opens, and on a slow machine (cold NAS, heavy mpv scripts) that window is long. mpv's events carry no file identity, but its IPC event stream is ordered, so the observed `path` closes the window. Suspending drift correction in the gap means a mid-load window, or a file the user dragged in themselves, is never slewed or hard-seeked.

The two carve-outs: a file that fails to open may never produce a path observation at all, so gating the load-*failure* report would suppress it entirely — and a stale one merely re-resolves the file (wrong but self-healing, the safe direction). The echo accounting of a programmatic seek is consumed even when the echo arrives gated-out because it is our own seek; leaving it outstanding would swallow the user's next genuine seek as a stale echo. Only the user-seek/debounce half of seek handling sits behind the gate.

### Collapsing incremental subtitle cues

**Rule:** A subtitle observation that is a prefix or suffix of the previous line replaces it in place (growth) or is dropped (shrink-back); see [design.md](design.md#subtitle-display).

**Why:** mpv re-emits the whole joined on-screen value on every change, so the on-screen cue-set evolving produces observations that are the *same* utterance, not a new line. Some subs reveal a line letter-by-letter over 2–3s as rapid-fire cues; when two ASS events display at once mpv joins them with a space, and as one ends the text shrinks back to just the other, from either end since mpv's join order is not fixed. Without collapsing the shrink-back, a brief interjection overlapping a stable line duplicates when it clears. The known cost — an unrelated later cue that happens to be a prefix or suffix of its predecessor is collapsed too — is rare and accepted; no time-window guard.

## Data Storage

### Local and synced state in separate databases (2026-08-21)

**Rule:** Local-only data lives in `dessplay.db`; the replicated CRDT snapshot lives in the derived sibling `dessplay.sync.db`, which is disposable; see [design.md](design.md#sqlite-database).

**Why:** The sync file's contents are a replica of server-authoritative state, so resetting wedged sync state (`--reset-sync`, `/resync`) should never cost local data such as the hash cache, watch history, or manual mappings. Before the 2026-08-21 split both lived in one database and a reset was destructive. The first-open migration moves the legacy `crdt_state` row over and drops the old table, idempotently and crash-safely.

### Unsent local ops are not persisted

**Rule:** Local ops the server has not yet seen are buffered in memory only; a crash loses the most recent local edits; see [design.md](design.md#sqlite-database).

**Why:** Crashes should be rare enough not to matter, and an edit that *caused* a crash should not be replayed into the next session. A persisted op log would buy durability for exactly the edits most likely to be poisonous.

### Tagged snapshot envelope

**Rule:** The stored `crdt_state` blob carries a 4-byte magic plus the protocol version ahead of the postcard body; one untagged legacy layout (v6) is decoded and migrated, byte-identical tagged versions decode via an explicit compatible list, and anything else is refused; the server backs up its database before first persisting a migrated blob; see [design.md](design.md#schema).

**Why:** A blob should name its own layout instead of being identified by trial decode — postcard will happily "succeed" on the wrong layout. The first byte 0xFF is chosen because no untagged postcard state can start with it, which is what makes the single legacy arm safe. Refusing unknown versions matters most for the *server*, which is authoritative and cannot re-sync its lost state from anyone; a deliberate migration adds an explicit decode arm instead of guessing. An interactive client has the cheaper fallback of dropping an unreadable blob and re-syncing from the server.


## A local expedition for the waiting room (2026-09-05)

The Waiting Below gives people something to play while friends arrive. Five
floors, a recover-and-return objective, fog, finite supplies, and body-part
injuries provide a small complete roguelike without introducing another
multiplayer protocol. Turns depend only on explicit commands: five-minute
sessions must not punish someone for leaving to watch an episode. The
existing log-modal layout keeps party chat visible, while a sticky presence
banner makes arrivals noticeable even during help or after a death.

The session bridge owns persistence and the UI only displays committed
results. Saving after every action, including the RNG state, makes closing,
crashing, and reopening ordinary lifecycle paths rather than special game
save operations. SQLite transactions also contain the finished-run history
and an outbox report. A disk failure cannot leave an advanced UI paired with
an older save. Invalid/future saves are preserved for recovery rather than
silently starting over. Saves are per username in the irreplaceable local
database, so a sync reset does not erase them.

Death summaries are real synced chat, because a late-arriving friend should
see how the expedition ended. Local narrator lines cannot provide that.
Their saved timestamp, sender, and expedition-numbered text form a stable
retry identity. The sync actor deduplicates and flushes before acknowledging
an outbox record; a crash between the two databases therefore safely retries.
This adds a local command, without changing network or CRDT schemas.
Automatic reports do not imply returning from Away and do not enter IRC.

The command popup derives its height from the filtered command table. Adding
`/rogue` exposed the old fixed fourteen-row cap, which hid `/quit`; removing
that independent cap makes future commands discoverable without a second edit.
The whole-app test requires every command to render on a sufficiently tall
terminal.
