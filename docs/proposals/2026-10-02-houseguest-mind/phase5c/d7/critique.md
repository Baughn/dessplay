# D7 (the TV picture): critique synthesis

Sources: [critic-mechanics.md](critic-mechanics.md) (M), [critic-character.md](critic-character.md) (C),
the mock (`d7-mock-1x.png`, `d7-mock-1x-nn3x.png`, `mock.py`), and my own re-check of the code.
Refs are to the tree at efea6f57 unless marked "working copy". The working copy has uncommitted
step-8b edits by other agents.

Where the critics disagreed, this is how it was settled:
- Film identity: a pixel hash (M), not a generation counter (C). Identical paused frames should
  dedupe.
- The id lives in `Look`, not in `Channel`. Scripts are `const` tables of `Channel`, so they can't
  carry a runtime id. A new `Channel::Held(card)` is resolved at paint time instead.
- ASCII needs no film glyph (C §11 is moot). `Graphics::new` is kitty-only (graphics.rs:414–418), so
  no picture is ever requested or set in ASCII mode.
- Resampling: both views merged. Prescale off the UI thread to a fixed source, then box-filter once
  more to the whole-pixel glass rect at compose time. Never use an SVG `<image>`.
- No placeholder PNG on her TV (M). The rule becomes: no file held means no request, and she shows a
  card.
- The static lasts 1.2 s, not 1.4 s (M was wrong). Snow animates on `CHANNEL_FRAME_MS` = 400 ms
  (script.rs:91, osaka.rs:6273), not on the 1.4 s use-frame grid. "About 1 s" already shows 2–3 snow
  frames. 1.2 s is three of them, ending on a frame boundary.
- Both critics give Watch's length as 20–45 s (osaka.rs:1407 at efea6f57). The working copy's 8b
  retune has it at **32–82 s**, and Round 3 holds Watch out of lingering only until D7 (design line
  471). The rule below doesn't depend on the act's length, which is the point.

---

## The amended D7 (replaces "D7. The TV picture (new)" in full)

### D7. The TV picture (amended after its two-lens critique, 2026-10-06)

**On and off.** The TV is on only during a TV act (Watch, Surf, Shopping). `Osaka::prop` gives a
channel only for a script key that carries `Prop::Tv` (osaka.rs:6262–6276). An empty room never has a
lit TV.

**The switch-on.**
- Static shows as she switches it on, for `STATIC_MS` = 3 × `CHANNEL_FRAME_MS` (1.2 s, three snow
  frames).
- After that comes a **held picture**, which lasts the rest of the act. Nothing on the glass animates
  after the static.

**Scripts (step 12a, pure, one golden re-record).**
- `WATCH` becomes two keys: static `Span::Ms(STATIC_MS)`, then `Channel::Held` for `Span::Rest`.
  Today it is one key holding `Snow(0)` for `Span::Rest` (script.rs:1386–1392).
- `SURF` keeps its flicks as they are: snow, colour bars, snow, the sunrise "ooh". Its resting key
  becomes `Channel::Held` instead of `Snow(0)` (script.rs:1428–1434). A surf therefore shows at most one
  film picture, by construction.
- Shopping: Chiyo-chichi bobs only through the hook (`Upto(2,5)`). The pitch and the rest show a held
  frame that `Prop::framed` leaves alone. Today every `Shopping` key bobs at 400 ms for the whole act
  (script.rs:1438–1459, osaka.rs:6273). On an act over 30 s that fails this section's test.
- The card: at the start of every Watch or Surf act she draws one of the four approved programme
  cards (`Programme::ALL`, art.rs:1410). The act carries it, and `Osaka::prop` resolves `Held` to
  `Channel::Held(card)`.
  - The draw happens **every time, even when the film will show**, so her RNG stream doesn't depend
    on whether a picture arrived.
  - ASCII shows the card's glyph pair (`screen_glyphs`, mod.rs:3940). There is no film arm.

**The film (step 12b, presentation only, no golden moves).**
- *When.* With kitty graphics on, the TV drawn as `Look::Tv` (not a scrap TV, mod.rs:3664–3680), and
  a Watch or Surf act **chosen** (usually while she walks to the TV), the guest raises a want.
  - `Guest::tv_wants_picture()`, take-style, read by the shell as it reads `ledger_to_save`
    (shell.rs:504). The want is output only: nothing in the mind, brain, scripts or ledger reads it
    back.
- *Gate (run loop).* Make no request unless:
  - this client holds the now-playing file Ready (`holds_file`, as commentary does: run.rs:2130–2134);
  - no TV request has gone out in the last 60 s, **counting failures**;
  - `Session::request_screenshot` returns true (session.rs:2797–2804).

  If the last check returns false, answer `None` at once without polling.
- *Its own slot.* Lift commentary's slot and poll into a small `screenshot` module: a private 0700
  dir, a `request` that deletes the path, sends the command and stamps the time, and a
  `poll(path, requested_at)` that waits for a stable size and an mtime ≥ the request.
  - Commentary keeps its slot and its API byte cap.
  - The TV gets a separate `dessplay-tv-*/frame.jpg`, created in run.rs next to the commentary
    engine. Sharing one path would race: commentary deletes its path before each request
    (run.rs:2150) and sends what it reads to Anthropic (commentary.rs:1023).
- *Treatment (blocking pool, the `FetchChatImage` shape, run.rs:1657–1682):*
  1. Poll for at most 2 s.
  2. `chat_images::decode_capped`.
  3. Centre-crop to the glass's 58:46 (`GLASS`, art.rs:1424), then **zoom 1.3×**. Faces get bigger,
     and most hardsub bands and letterbox mattes are cut off.
  4. **One levels stretch from luma**: the same linear map on all three channels, from luma's 2nd
     and 98th percentiles, with gain ≤ 3. Then saturation ×1.25.
  5. Box-downscale to a fixed source of ≤ 128×102.
  6. Hash the pixels to get the `FilmId`.
  7. Reject a black or flat frame (mean luma < 20/255, or luma p98 − p2 < 24) as a failure.

  Don't posterize, don't add scanlines, don't tint, don't letterbox (the mock shows each is worse).
- *Delivery.* `UiInput::TvPicture(Option<TvPicture>)` with `Delivery::Latest`. `UpdateKey` gains a
  `TvPicture` key (ui/delivery.rs:15).
  - `TvPicture { id: FilmId, file: Ed2kHash, image: Arc<RgbaImage> }`.
  - The shell keeps the request state and drops `in_flight` after 3 s with no answer.
  - `UserAction::TvPicture` is sent with `try_send`. On `Full` the want is kept for the next turn
    (the `dispatch_image_fetches` shape, shell.rs:776–789).
- *Into the guest.* `Guest::set_tv_picture(..)` forwards into `Graphics` only, never into `IdleView`.
  `IdleView` derives `Clone, Debug, PartialEq` and is rebuilt every draw (idle.rs:69, app.rs:588).
  `Graphics` holds two slots:
  - `next`: the latest good picture delivered.
  - `showing`: the picture latched for the current act, keyed by the act's start time.
- *Latch.* When a held key first paints in an act, `next` is copied into `showing` if `next` exists
  and its file is the current now-playing. Otherwise the act shows its card.
  - A picture that arrives later goes into `next` and waits for the next switch-on. **Nothing swaps
    mid-act**, and the act's own picture can't fall out of the table: that is why there are two slots
    and not one.
  - `piece_look` resolves `Held(card)` to `Look::Film(id)` while `showing` is set, and to
    `Look::Tv(Programme(card))` otherwise.
- *Staleness is by file, not by clock.*
  - When `now_playing` changes or stops being Ready, the shell clears `next` (the snapshot it already
    holds, app.rs:590).
  - `showing` lasts its act.
  - A failed or black refresh on the same file keeps the last good picture.
  - Within 60 s of the last request, a switch-on reuses `next` without asking.
- *Composite (art.rs).* `render_tv(.., film: Option<&RgbaImage>)`:
  1. Rasterize the TV with an empty glass.
  2. Box-downscale the source once to the glass's **whole-pixel** rect at the live cell size
     (`picker.font_size()`, graphics.rs:438): about 27×22 px at 9×19 cells, 28×22 at 10×20.
  3. Paste it **un-mirrored** through a mask rasterized from the same `rx=8` glass rect. For
     `Facing::Left` the glass sits at x = 120 − 26 − 58 = 36 units.
  4. Overlay the stroke.

  `tv_scene` mirrors everything inside `<g{mirror}>` (art.rs:1515–1540), and resvg is built without
  default features (Cargo.toml:86), so an SVG `<image>` won't do. The drawn cards stay mirrored. No
  sheen over the film unless the user asks (Q1).
- *Her reaction.* None to the picture. Her faces and bubbles at the TV are what the script already
  gives (see Q3).
- *Privacy.* The frame never leaves the process:
  - it isn't in the ledger (no TV state there) or in commentary or advisor context;
  - `UiInput` has no `Debug` (shell.rs:69);
  - logging is trace for the request and answer, debug for a failure, and never pixels.

  design.md states it.
- *Who sees it.* A resident (on by default) during playback (`IdleView::open`, idle.rs:101–110).
  Anyone during a pause or a block, since a paused group doesn't count as playing (app.rs:594,
  derive.rs:316). Paused frames hash the same, so they cost no new kitty images.

**Test.**
- *Stillness.* After its first 10 s, no act longer than 30 s changes cells (line art: image keys;
  ASCII: glyphs) faster than `USE_FRAME_MS`, in both modes. Exempt are the slow blink's two flips
  (step 8) and the shopping hook's bob, which ends at `Upto(2,5)`. Pose, prop and bubble changes all
  count.
- *Purity property (12b's guard).* For random seeds and `set_tv_picture` calls at random times,
  including during an act, her state trace and her ASCII frames are byte-identical with and without
  pictures. The trace covers acts, positions, faces, bubbles and RNG draws.
- *Latch.* In line art, a picture delivered after a held key's first paint never changes that act's
  image keys. The next act shows it.
- *Canvas* (`Graphics::canvas`, a synthetic gradient with a marked corner): left and right TVs (the
  corner isn't mirrored), the rounded clip, the aspect crop, the whole-pixel rect at two cell sizes,
  and an unknown id falling back to the card.
- *Feed state machine* (shell, pure, `now` injected): one request per chosen act; reuse within 60 s;
  failures throttled; a file change clears `next`; the 3 s backstop; no request while the file isn't
  held.
- *Run arm.* `MockPlayer` (player/mock.rs:171) writes a fixture JPEG: the answer is `Some` with the
  right file. Black and flat fixtures get `None`. With `holds_file` false nothing is sent.
- *Fixtures and review art* are synthetic or CC-BY (Sintel, Big Buck Bunny), never commercial
  episodes.
- `image_census` (tests/census.rs:1304) gains a TV room whose film changes on every switch-on (the
  worst case). Re-measured in step 13.
- *Before 12b lands:* an in-tree review sheet in the style of `stillness_sheet` (art.rs:2371),
  rendering a CC-BY film through the real path, facing both ways.
- *Measure in 12b, don't assume:* the IPC reply time of a 4K `video` grab (`log_reply`, mpv.rs:296).
  Add an `"async": true` variant of `send_command` only if it delays the next command.

**Steps.**
- 12a: static and held cards, the shopping hold, the stillness test; one re-record. With the held
  picture in, Watch may join lingering (Round 3, 8b).
- 12b: the screenshot module and both slots, the feed, the latch and composite, the purity property;
  no golden moves.
- Docs:
  - design.md: the TV shows the loaded film's frame when this client holds it, and the frame is
    local;
  - decisions.md: why a latch rather than a wait, why invalidation by file, why a separate slot, why
    no reaction.

---

## Blockers

**B1. The image key can't carry a picture.** `Look::Tv(art::Channel)` is part of the `Copy + Hash`
cache key (graphics.rs:77, 145–157), and `Channel` is `Copy` (art.rs:1382). `Look::render(self, ..)`
has no context (graphics.rs:104–121).
- Fix: `Look::Film(FilmId)` with a pixel-hash id, resolved from `Channel::Held(card)` at paint
  (`piece_look`, mod.rs:3681).
- Not `Channel::Film` (C §8), because scripts are `const` and can't hold a runtime id.
- Sources: M Blocker 1, C §8; the `Held` resolution is the synthesis's.

**B2. The screenshot slot is commentary's.** Commentary deletes `frame.jpg` before each request
(run.rs:2150) and sends what it reads to Anthropic (commentary.rs:1023). If D7 shares the slot,
commentary can delete a TV frame mid-poll, attach a frame the TV asked for, or the TV reads
commentary's frame.
- Fix: a shared `screenshot` module and a private TV slot. Source: M Blocker 2.

**B3. A one-entry film table breaks "never swap mid-act".** If `Graphics` holds only the latest
picture, a later delivery evicts the id the current act is drawn with. That key then misses and falls
back to the card mid-watch.
- Fix: two slots, `showing` (latched per act) and `next`. Source: the synthesis. This is a conflict
  between M's one-entry table and M's own latch rule.

## Majors

**M1. The timings don't fit the act.** "Every 60 s while a watch runs", "> 2 s late" and "stale after
10 min" don't fit:
- a Watch lasts 20–45 s (osaka.rs:1407 at efea6f57), or 32–82 s in the working copy's 8b retune;
- lingering is coming (design line 471).

So the 60 s refresh goes from "never fires" to "flips mid-watch" with no rule change. A change "a
minute passed" explains is churn; "she switched it on" explains it.
- Fix: one picture per switch-on, requested when the act is chosen, latched at the held key, never
  swapped. 60 s becomes a request throttle and a reuse window.
- Sources: M Major 3, C §3. Both agree.

**M2. Staleness by clock is the wrong axis.** A 9-minute-old frame of the episode still on is right;
a 30-second-old frame of last night's episode is wrong.
- Fix: tag each picture with the now-playing hash, clear `next` on a change, and keep the last good
  frame on a same-file failure. Sources: M Major 4, C §3.

**M3. The guest's timeline must not wait on mpv.** "Static until the picture or 2 s" would put mpv's
latency into her trace.
- Fix: a fixed `STATIC_MS`, a latch, and a card drawn every act whether or not the film shows, so the
  RNG stream is identical.
- Guarded by the purity property, which also drops C §6's free face change: the face would depend on
  whether the picture arrived.
- Sources: M Major 3 and Minor 4; the card-draw invariant is the synthesis's.

**M4. The picture enters through a setter, not `IdleView`.** `IdleView` is `Clone + Debug +
PartialEq` and rebuilt every draw (idle.rs:69, app.rs:588). The want has to come from the guest,
since only she knows a TV act was chosen and the TV is drawn as `Look::Tv`.
- Fix: `tv_wants_picture` / `set_tv_picture`, the shell owns the feed, latest-wins delivery.
- Sources: M Major 1, C §3 (a guest cue).

**M5. Request gating and failure modes.**
- Gate on `holds_file` (run.rs:2130–2134). Otherwise mpv is showing the "You don't have this file"
  placeholder (placeholder.rs:1–7), which is unreadable at 27 px: show the card, don't paint it.
- Answer `None` at once when `request_screenshot` is false. A dead mpv drops the command silently
  (player.rs:634–639), so it's covered by the 2 s poll and a 3 s shell backstop.
- Throttle requests, not successes.
- Sources: M Major 2; rejecting the placeholder is the synthesis's.

**M6. Treatment, or dark scenes are mud.** Plain crop and downscale turns night and interior scenes
into purple-brown mud (mock, column 1). Fix:
- crop, zoom 1.3×, then one luma-based levels stretch;
- per-channel autocontrast shifted hues in C's first pass;
- saturation ×1.25;
- black and flat frames rejected.

Posterizing, scanlines, CRT tint and letterboxing are all worse in the mock.
- Source: C §1–2. I checked the mock: the "zoom 1.3 + lift" column is the most legible in every row.

**M7. The film mirrors with a left-facing TV.** The picture sits inside `<g{mirror}>` (art.rs:1515–1540).
- Fix: composite in pixel space, un-mirrored, through the glass mask, at the whole-pixel rect (the
  half-pixel edges, as with the poster).
- Sources: M Minor 1, C §7 and "numbers".

**M8. Shopping fails D7's own stillness test.** Every `Shopping` key bobs at 400 ms for the whole
act (script.rs:1438–1459; `prop` frames it, osaka.rs:6273). A shopping act over 30 s, which is most of
them at 32–82 s, fails "no flips after 10 s".
- Fix: the bob ends with the hook, and the test names that exemption. Source: C §10, made concrete
  here.

## Minors

1. The static is 1.2 s, three 400 ms snow frames (script.rs:91, osaka.rs:6273). M's 1.4 s rests on a
   wrong frame grid. *Synthesis, correcting M Major 3.*
2. Surf: the flicks stay drawn and only the rest key latches, so a surf has at most one film by
   construction. Landing on the film as the "ooh" is left for later. *C §3, M Major 3.*
3. Resampling: prescale to ≤ 128×102 off-thread, then one box downscale to the snapped rect at the
   live `font_size()`. A resize then never needs a new screenshot (`Key.cell` already re-keys,
   graphics.rs:156). *M Major 5, C "numbers".*
4. ASCII: no film glyph needed. Graphics is kitty-only (graphics.rs:414–418), so `Held(card)` shows
   the card's pair. *Supersedes C §11.*
5. Kitty churn: film id × her overlapping looks gives about 2–8 encodes per film. Fine against
   `CACHE_LIMIT` 1024 (graphics.rs:58), and paused frames dedupe. Add the worst case to
   `image_census`. *M Minor 2.*
6. Fixtures and review sheets: synthetic or CC-BY only. *C §9.*
7. IPC latency of a 4K grab: measure via `log_reply` (mpv.rs:296) before adding `"async"`. *M Minor 3.*
8. Logging: trace for cross-actor request and answer, debug for failures, never pixels. *M Minor 5,
   CLAUDE.md.*
9. In-tree review sheet through the real path before 12b lands. *C "what to show".*
10. The mpv `"video"` flag drops only soft subs. Hardsubs are mostly cut by the 1.3× zoom.
    *C §1 (M said "no subtitles", which holds only for soft subs).*

## Questions for the user

**Q1. Is a ~27×22 px picture of your film worth it over the drawn cards, and with or without the
sheen?** Compare columns 7 and 8 of `d7-mock-1x-nn3x.png` (columns 1–6 are the rejected treatments).
- *Recommended:* yes, "zoom 1.3 + lift", no sheen. It reads as your film on her TV, and the TV's bold
  frame and glass outline already give it the sticker edge.

**Q2. One picture per switch-on, never refreshed mid-watch, even on a long watch (Watch is 32–82 s
in the 8b retune, longer once it lingers)?**
- *Recommended:* yes. A change she caused explains itself; a timed refresh is a bright patch
  changing for no reason. The next switch-on (or the next Watch) takes a fresh frame.

**Q3. Should she ever react to the film? (C proposes one "Ooh, I know this one!" per file per visit,
outside the line budget.)**
- *Recommended:* not in step 12. Whether the film or a card is on depends on mpv's timing, so a line
  keyed on it puts that timing into her trace. The purity property would have to exempt it, and it
  would sometimes miss (a card shows when the frame was late).
- If you want it, it can be a follow-up: a presentation-side bubble, named in the property's
  exemptions, firing only when `showing` latched a film.
- The critics split here: C called it the one line that pays for itself.
