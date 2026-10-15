# D7 "The TV picture": mechanics critique

Lens: the real path from mpv to her glass. Read-only; refs are to the tree at efea6f57.

## Verdict

**Feasible and cheap, but not as written.** The feature will be seen often: a resident (on by
default) during playback, and anyone during a pause with a file loaded. The pieces are all in the tree
(a fire-and-forget screenshot command, a blocking-pool decode pattern, a UI mailbox with latest-wins
delivery). D7 as written has two blockers: the cache key can't carry a picture, and the screenshot slot
must not be shared with commentary. It also has one wrong layer: the picture belongs in a setter, not
in `IdleView`. Its timings don't fit the act it decorates. A Watch lasts 20–45 s, so "every 60 s while
a watch runs" almost never fires, and "stale after 10 min" is the wrong axis: a picture goes stale
because the file changed, not because time passed. Split step 12 into **12a** (drawn held pictures,
pure guest, a golden re-record) and **12b** (the film feed, presentation-only, no golden changes).

## Does she ever show while a file is loaded? Yes, in two ways

- `IdleView::open()` (dessplay/src/ui/houseguest/idle.rs:101–110): a resident is kept out only by
  `Busy::Overlay`, and gets through `Busy::Playing`. Resident is on by default (docs/design.md:2142).
  So a **resident sees the live film during playback**, every switch-on. (Her focused-pane protection
  while in use only moves her elsewhere: design.md:2145–2150.)
- `busy = Playing` only when `derive::playback_active` (dessplay/src/ui/app.rs:594, 604–607), which
  is false while the group is paused, blocked on someone, or a ready mark is pending
  (design.md:1348–1352). So **a visitor arrives during a pause, after the idle delay, with mpv holding
  the paused frame.** "Paused and walked off" is the case where visits happen. In that case every
  screenshot is the same frame (see dedupe below).
- Where it doesn't show: ASCII mode (no `Graphics`: graphics.rs:414–418 needs the kitty protocol), an
  overlay up, a terminal under 60×18, or mpv fullscreen over the terminal (cost then ≈ nil).

## The real path today (what D7 would reuse)

1. Requester → `Session::request_screenshot(path)` (dessplay/src/session.rs:2797–2804). It returns
   `false` only when no player was ever spawned.
2. Player actor `PlayerCommand::Screenshot` (dessplay/src/actors/player.rs:176, 634–639). With
   `self.player` `None` (mpv dead or relaunching) it is **dropped silently**, and the requester only
   learns of that by timing out.
3. `Mpv::screenshot_to_file` (dessplay/src/player/mpv.rs:397–406) sends
   `["screenshot-to-file", path, "video"]` through `command` → `send_command` (mpv.rs:257–284),
   **fire-and-forget**: the reply is only logged. `video` is the raw frame at the source's own
   resolution, with no OSD or subtitles, which is right for her TV as well. Per `man mpv`
   (Screenshot Commands): "For normal standalone commands, this is always asynchronous" (since
   0.29), so the encode and write run off mpv's core. Neither playback nor, probably, the IPC
   connection waits on them. Unverified: how long the IPC reply takes on a 4K frame (see Minor 3).
4. Commentary's consumer (dessplay/src/run.rs:2141–2158; dessplay/src/commentary.rs:608–640,
   1099–1140):
   - The slot is a private 0700 tempdir holding `frame.jpg`.
   - Before each request it deletes the path and stamps `requested_at`.
   - The blocking job polls 20 × 100 ms for a stable, non-empty size.
   - It rejects files whose mtime predates the request, reads the file, deletes it, and caps the bytes.
5. The blocking-decode-to-UI template is `FetchChatImage` (run.rs:1657–1682). It runs
   `spawn_blocking` with `chat_images::fetch` → `decode_capped` → prescale (chat_images.rs:17–60),
   then `ui.send(UiInput::ChatImage{..})`. The UI applies the answer in the shell's match
   (dessplay/src/ui/shell.rs:573) and draws after every input (shell.rs:667).
6. The guest lives on the UI thread, owned by `run_ui_loop` (shell.rs:472–498). Her images are
   composed, rasterized (resvg) and kitty-encoded **on the UI thread, on cache misses**
   (graphics.rs:447–489, 657–712).

## Findings

### Blocker 1: the image cache key cannot carry a picture

`Look::Tv(art::Channel)` (graphics.rs:77) is `Copy + Hash + Eq` and is part of `Key.layers`
(graphics.rs:145–157). `Look::render(self, facing, w, h)` (graphics.rs:104–121) has no context to
fetch pixels from. An `Arc<RgbaImage>` can't go in `Channel`.

**Amendment:**
- Add `Look::Film(FilmId)` (or `Channel::Film(FilmId)`), where `FilmId` is a `u64` **hash of the
  prescaled pixels**.
- `Graphics` holds the side table: one current `(FilmId, Arc<RgbaImage>)` is enough.
- `compose` passes it to the TV render.
- A key whose id isn't the current film renders the drawn fallback card. Old ids age out of the LRU
  (graphics.rs:465–473).
- Content-hashing the id makes identical paused frames hit the cache for free, with no new kitty
  transmission.

### Blocker 2: a separate slot, never commentary's

Commentary deletes its path before each request (run.rs:2150) and ships whatever its poll reads to
Anthropic (commentary.rs:1023, 1081–1092).

- If D7 wrote to `frame.jpg`, commentary could delete a D7 frame mid-poll.
- A commentary poll could also attach a frame D7 asked for. That frame is the same film, but it's a
  coupling bug, and it breaks the promise that her TV never feeds the API.
- In the other direction, D7 could read commentary's frame.

**Amendment:**
- Lift `ScreenshotSlot` and the poll into a small `screenshot` module: create a private dir, a
  `request` that deletes the path, sends and stamps the time, and a `poll(path, requested_at)` that
  checks for a stable size and mtime ≥ request.
- Commentary keeps its API byte cap on top. D7 instead decodes with `chat_images::decode_capped`
  (dimension and allocation caps).
- D7 owns its **own** slot (`dessplay-tv-*/frame.jpg`), created in `run.rs` next to the commentary
  engine.
- State it in design.md: her picture never leaves the process. It isn't in the ledger (`Channel` isn't
  serialized: ledger.rs has no TV state), it isn't logged (`UiInput` has no `Debug`: shell.rs:68–69),
  and it isn't in commentary or advisor context.

### Major 1: the picture enters through a guest setter, not `IdleView`

`IdleView` derives `Clone, Debug, Default, PartialEq` (idle.rs:69). `Ui::idle_view` rebuilds it every
draw (app.rs:588–656), and the guest clones it each frame in `gate` (mod.rs:2617). A pixel buffer
would be compared and cloned per frame, and printed whole by any `{:?}`. The request also has to start
from the guest: only she knows the TV is on, in sight, and drawn as `Look::Tv` rather than a scrap TV
(mod.rs:3664–3680).

**Amendment:** the shell owns the feed (shell.rs is already where she's polled:
`ledger_to_save`, shell.rs:504):
- `Guest::tv_wants_picture() -> bool`, set during paint when a held TV picture is about to show (see
  Major 3) and graphics is on.
- `Guest::set_tv_picture(Option<TvPicture>)`, where `TvPicture { id, file: Ed2kHash, image:
  Arc<RgbaImage> }`. It is forwarded into `Graphics`, never into the mind, brain or ledger.
- New `UserAction::TvPicture`, sent with `try_send`: on `Full`, keep the want for the next turn, the
  `dispatch_image_fetches` shape (shell.rs:776–789).
- New `UiInput::TvPicture(Option<TvPicture>)` with `Delivery::Latest(UpdateKey::TvPicture)`
  (delivery.rs:30–55).

### Major 2: request gating and the failure modes

A run-loop arm, modelled on `FetchChatImage`, answers every `UserAction::TvPicture`.

**Gate first:**
- Ask only if `last_view.now_playing` is `Ready` for me (`holds_file`, as commentary does:
  run.rs:2130–2134). Otherwise she paints **the not-watching placeholder PNG** (placeholder.rs:1–7:
  filename + "You don't have this file") on her TV.
- If `request_screenshot` returns `false` (session.rs:2801), answer `None` at once and skip the 2 s
  poll.

**Then, on the blocking pool:**
- `poll` (≤ 2 s, commentary's 20 × 100 ms).
- `decode_capped` (a 4K JPEG ≈ 50–100 ms here, off the UI thread).
- Centre-crop to the glass's 58:46 aspect (art.rs:1430). Prescale to a **fixed small source**
  (≤ 128×102). Hash it.
- Send `Some(TvPicture{..})` or `None`.

| Case | What happens |
|---|---|
| mpv dead or relaunching | Command dropped (player.rs:634–639), poll times out, `None` |
| mpv idle or audio-only | Command fails in mpv (logged at debug only), poll times out, `None` |
| User scrubs | The next request just gets the new frame; nothing special |
| Paused | Identical frames hash the same and reuse the cached image |
| File changes | See Major 4 |
| UI backstop | The shell drops `in_flight` after 3 s even with no answer |

**Throttle requests, not successes.** One request per 60 s **whether or not it succeeded**. D7's "at
most once every 60 s" read as success-only would let a futile mpv-idle request run every switch-on in
a surf.

### Major 3: the timings don't fit the act; latch the picture

Watch lasts 20–45 s (osaka.rs:1407), and surfing holds each channel 2.8 s (`SURF_MS = 2 ×
USE_FRAME_MS`, script.rs:1395; `USE_FRAME_MS = 1400`, osaka.rs:1515). So "every 60 s while a watch
runs" means one picture per watch.

The "about 1 s" static is shorter than one snow frame (snow animates on the 1.4 s frame grid,
script.rs:113–118), so it would show one still frame of snow.

**Amendments:**
- Static is **one `USE_FRAME_MS` (1.4 s)**, fixed in the script.
- **Never let the guest's timeline wait on the picture.** Holding the static "until the picture or
  2 s" would make her trace depend on mpv's latency. Instead, the held span **latches** whatever fresh
  picture the guest holds when it begins: the film if there is one, otherwise the card her whims
  picked (pure, chosen as today).
- A picture that arrives later is **not cut in**. A card turning into the film half a second later
  reads as a glitch. It's kept for the next switch-on.
- **Ask early: when a Watch act is chosen (she's walking to the TV), not when the TV switches on.**
  Her walk gives seconds of budget, so the film is usually ready when the static ends. A typical
  request finishes in about 0.2–0.5 s: mpv's write, two stable 100 ms polls, the decode, the mailbox.
- 60 s becomes a **reuse window**: within it a held span reuses the last picture without asking.
  Mid-watch refresh: none. That suits the user's "changing infrequently" and keeps churn minimal.
  Open question for the user: should a 3-minute evening watch change picture once?
- Surfing: the first held channel may latch the film, the rest are cards. Shopping and sunrise are
  unchanged.

### Major 4: replace "stale after 10 minutes" with invalidation by file

A 9-minute-old frame of the film still playing is fine. A 30-second-old frame of the previous episode
is wrong.

**Amendment:**
- Stamp each `TvPicture` with the now-playing hash it was taken under.
- The shell drops it (`set_tv_picture(None)`) when `ui.snapshot.view.now_playing` differs or isn't
  `Ready`. The UI already holds that snapshot (app.rs:590), so no new messages are needed.
- If a refresh fails while the file is unchanged, keep the last good frame: it's still the film.
- No wall-clock staleness rule.

### Major 5: the decode, scale and compose split, and resize

Where the cost goes:
- Full decode and crop on the blocking pool, once per picture.
- A per-compose resize from the ≤128 px source to the glass's pixel size on the UI thread
  (microseconds), inside the existing `compose`, which runs only on a cache miss.

How small the glass is: TV footprint 6×4 cells (room.rs:178–181) → frame 120×168 units
(art.rs:20, 1561–1567). At the census's 10×20-px cells the scale is min(0.5, 0.476), so **the glass
is ≈ 28×22 px**, ≈ 55×44 on a 2× display. It's a moving colour patch, not a legible picture (the
character critic should weigh that).

Resize: `Key.cell` (graphics.rs:156) already re-keys everything on a font or cell change, and the
fixed-size source means a resize never needs a new screenshot.

### Minor 1: compositing in the mirrored, clipped TV

`resvg` is built with `default-features = false` (Cargo.toml:86), so SVG `<image>` won't render; the
film has to be pixel-composited.

`tv_scene` mirrors the whole TV for `Facing::Left` (art.rs:1515–1519). A film must not mirror (any
on-screen text would read backwards), so paste it un-mirrored at the mirrored glass x
(120 − 26 − 58 = 36 units).

The glass is clipped with `rx=8` and stroked over (art.rs:1541). So:
1. Rasterize the TV with an empty glass.
2. Paste the film through a mask rasterized from the same glass rect at the same transform.
3. Overlay a stroke-only SVG.

That's three tiny resvg passes per cache miss, through `render_tv(.., picture: Option<&RgbaImage>)`.

### Minor 2: kitty churn

When she overlaps the TV she's composed into the same image (mod.rs:2217–2235). Each film id
therefore multiplies with her poses, faces, the slow blink and facing: about 2–8 new encodes per
watch, each ~40–60 KB of transmission. At ≤ 1 film per watch that is a few dozen extra images in a
two-hour visit, against `CACHE_LIMIT` 1024 (graphics.rs:58).

Paused frames dedupe by hash. Dead ids are the LRU's first victims.

**Amendment:** `image_census` (tests/census.rs:1304) gains a room with a TV and a film that changes
on every switch-on (the worst case), re-measured in step 13.

### Minor 3 (verify in step 12, not a finding): IPC latency of a 4K grab

The man page says standalone screenshot commands encode and write on a worker by default, so the
JSON-IPC request (no `"async": true`, mpv.rs:262–284) probably doesn't hold the connection ahead of
pause, seek and speed commands.

To confirm: time the reply. It is already logged with its request id (`log_reply`, mpv.rs:296ff).
Only if a 4K `video` grab measurably delays the next command, add an `async` variant of
`send_command` (helps commentary too).

### Minor 4: tests that keep the guest pure (no golden changes in 12b)

- **Property:** for random seeds and random `set_tv_picture` calls at random times, her state trace
  (acts, positions, faces, bubbles, RNG draws) and her ASCII frames are **byte-identical** with and
  without pictures. That makes "the film is presentation only" a checked invariant.
- **Canvas test** with a fixed synthetic picture (a gradient with a marked corner), through
  `Graphics::canvas`:
  - left- and right-facing TV (the corner is not mirrored);
  - the rounded clip;
  - aspect crop;
  - an unknown id falls back to the card.
- **Feed state machine** in the shell, pure with an injected `now`:
  - one request per Watch;
  - reuse within 60 s;
  - failures throttled;
  - file change clears;
  - `in_flight` backstop.
- **Run arm:** a `MockPlayer` writes a fixture JPEG; `holds_file == false` sends nothing.
- The four insta snapshots (ui/houseguest/snapshots/) and every census are unchanged, because the
  default is `None`.

### Minor 5: logging

Per CLAUDE.md:
- The request and the answer are cross-actor, so trace.
- A failed or timed-out grab is debug, as in commentary.
- Never log pixels or the path's contents.

## Recommended minimal design (12b)

1. **Guest** (pure, `ui/houseguest/`):
   - `tv_wants_picture()` turns true when a Watch act is chosen or the TV is on, graphics is on, and
     the TV is drawn as `Look::Tv`.
   - `set_tv_picture(Option<TvPicture>)` goes into `Graphics` only.
   - A held span latches the current picture at its start, else her card.
   - `Look::Film(FilmId)` is a pixel-hash id with a one-entry side table.
   - The mind, brain, scripts and ledger never see the picture.
2. **Shell feed** (`ui/shell.rs`, small pure struct):
   - Ask on the want when there's no picture under 60 s old and no request in the last 60 s; send
     `UserAction::TvPicture`.
   - Apply `UiInput::TvPicture`.
   - Clear on a now-playing change.
   - 3 s `in_flight` backstop.
3. **Run loop** (`run.rs`):
   - Its own private `screenshot::Slot`. Gate on `holds_file` and on `request_screenshot` returning
     true.
   - `spawn_blocking`: poll ≤ 2 s, `decode_capped`, centre-crop 58:46, prescale ≤ 128×102, hash.
   - `ui.send(UiInput::TvPicture(..))`, latest-wins.
4. **Art** (`art.rs`): `render_tv` composites the picture un-mirrored through the glass mask, under
   the stroke.
5. **Docs:**
   - design.md rule: the TV shows a frame of the loaded film when there is one; frames are local and
     never sent anywhere.
   - decisions.md: why latch rather than wait, why invalidation is by file, why a separate slot.
6. **Order:** 12a (drawn held pictures and the 1.4 s static; re-record) lands first and alone. 12b is
   golden-neutral behind the purity property. Optionally an `"async": true` IPC commit.
