# D7 (the TV picture): character and look critique

Lens: character and look. Read-only; mock made in this directory from frames of two local episodes
(Niwatori Fighter 01, Overlord 04). Mock files are review material only: don't commit anime frames to
the docs (see finding 9).

## Verdict

**Adopt, with the timings replaced and one image treatment fixed.** At true size the glass is about
**27 × 22 px, roughly 3 columns by 1 row**. A film frame that small doesn't read as *a scene*, but it
does read as *a picture on a TV*. Close-ups (faces, eyes) and silhouettes against a sky come through
recognisably. Wide or busy shots come through as the scene's colour mood, which is what a real TV
across a room looks like. Dark scenes turn to **mud** unless the levels are lifted. With a centre
zoom and a hue-preserving levels stretch, none of the ten sample frames was mud.

The charm is that it's *your* film's palette on her TV. The user knows what's on screen in mpv,
so a 27 px thumbnail is enough for them to recognise it. A stranger couldn't. Don't design for the
stranger.

The 60 s refresh, 2 s wait and 10 min staleness should give way to **one picture per switch-on**
(finding 3). Posterizing and scanlines should be dropped, both by what the mock shows.

## The numbers (verified)

- TV footprint `(6, 4)` cells: room.rs:181. `CELL_UNITS = (20.0, 42.0)` (art.rs:20) makes the frame
  120 × 168 units (`prop_frame`, art.rs:1562-1568). At 9 × 19 px cells the box is 54 × 76 px (the
  review sheet renders exactly that, art.rs:2920).
- `rasterize` scales uniformly by min(54/120, 76/168) = **0.45** and bottom-aligns (dy = 0.4)
  (art.rs:1572-1584).
- `GLASS = (26, 72, 58, 46)` (art.rs:1424) puts the glass at x 11.7 → 37.8 and y 32.8 → 53.5 px,
  which is **26.1 × 20.7 px** with rx 8 units ≈ 3.6 px corners and a 1.6-unit (0.7 px) outline
  (art.rs:1540). Snapped outward to whole pixels that's **(11, 32) 27 × 22 px**. In cells it covers
  columns 1.3 → 4.2 and rows 1.73 → 2.82: about one text line tall and three letters wide.
- Glass aspect 58:46 = 1.26, close to 4:3. A 16:9 frame centre-cropped to it loses about 29% of its
  width. Letterboxing would leave a 27 × 15 px picture between black bars (see the mock: clearly worse).
- The edges sit on half pixels, the same trap the poster hit (decor/snippets.md:146). The picture
  should be resampled **once**, straight to the snapped pixel rect, and composited in pixel space
  under the glass mask. If it goes into the SVG as an `<image>`, resvg resamples it a second time
  (bilinear), which blurs the ~600 pixels there are.

## Findings and amendments

### 1. Treatment: zoom, lift, saturate. No posterizing, no scanlines.

What the mock showed, column by column (`d7-mock-1x.png`, `d7-mock-1x-nn3x.png`):

| Column | At 1× |
|---|---|
| plain (centre crop, box downscale) | Readable on bright close-ups; dark frames are purple-brown mud (Overlord night scene, throne room). |
| letterbox | Worst. Half the glass is black bars and the picture is 15 px tall. |
| poster8 / poster5 (median cut, no dither) | Barely differs from plain at 8 colours. At 5 it loses the face detail that made close-ups read (old man, crying boy). At ~600 px the image is already flat, so quantizing adds no sticker feel. |
| zoom 1.5 + poster8 | The zoom helps a little; the posterizing still costs. |
| poster8 + CRT (alternate rows ×0.82, cool cast) | Eleven stripes across 22 px read as texture/noise over the picture, and the cool cast greys warm scenes. Not worth it. |
| **zoom 1.3 + lift** | **Best in every row.** The crying face reads as a face, the old man as glasses and white hair, the red eye as an eye. The dark night scene becomes figures in blue light. |
| zoom 1.3 + lift + sheen | The off-TV's highlight arc (props.svg:55) in white at 55% over the picture. A faint sticker cue that ties it to the drawn hand; marginal at 1×. The user's call. |

**Amendment (replaces "cropped to the glass's aspect and scaled to the glass"):**
- Centre-crop to the glass aspect, **then zoom about 1.3×** (crop a further ~23% each way). Subjects
  get bigger, and most of a hardsub band or a letterbox matte is cut away (hardsubbed releases bake
  their subtitles in; mpv's `"video"` flag at mpv.rs:398-404 only drops soft subs).
- Area-average (box) downscale straight to the snapped 27 × 22 px rect at the live cell size: compute
  it from `picker.font_size()` (graphics.rs:438), not from a 9 × 19 constant.
- **Hue-preserving levels stretch:** one linear map for all three channels, taken from luma's 2nd
  and 98th percentiles, gain capped at about 3. The first mock used per-channel autocontrast and it
  shifted hues (the red eye's surround went green), so the spec should say "one stretch from luma".
  Then saturation ×1.2-1.3.
- No posterizing, no scanlines, no colour cast. Sheen optional (show the user both).
- The glass outline and the TV's bold frame already give it the sticker border; the picture inside
  it shouldn't try to be a sticker.

### 2. Black and flat frames fall back.

Frames at random moments are often black (fades, a file loaded but not started, the first frame
before the opening), credits (text on black) or flat title cards. A black glass on a switched-on
TV reads as "off" or broken.
**Amendment:** treat a frame whose mean luma is under ~20/255, or whose luma p98 − p2 is under ~24,
as a failed screenshot. Keep the last good picture of the same file if there is one, otherwise use a
drawn card.

### 3. Timings: one picture per switch-on, never a mid-watch flip.

- Change is this phase's theme. The D7 test only bounds flips faster than `USE_FRAME_MS` (1.4 s,
  osaka.rs:1515), so a 60 s flip passes it. But a 60 s flip is still a bright 3-cell patch changing
  for no visible reason. "She switched it on" explains a change; "a minute passed" doesn't.
- Watch runs 20-45 s today (osaka.rs:1407), so a 60 s refresh rarely fires inside one act. It
  starts to matter the moment Watch joins lingering (design, steps 8b/12, line 471), which is where
  "up to 3 minutes" comes from. Pin the rule now.

**Amendment (replaces "at most once every 60 s", "> 2 s" and "stale after 10 minutes"):**
- **Request when the Watch act is chosen**, while she's still walking to the TV. The walk is seconds
  long, so a slow mpv write has time to land.
  The shell can't see a job being chosen, and D7 says she stays pure. So the guest exposes the
  request as data: a deterministic "TV wanted" cue when it picks a Watch job, read by the shell the
  same way it reads her other cues (or the walk-to-TV job, through what the shell already reads).
  The shell does the request and the file I/O and hands back `Option<Arc<RgbaImage>>` plus a
  generation, as in D7. Without the early cue, the ~1 s window can't fit an mpv PNG write plus a
  decode. (For feasibility: decode and downscale on a blocking task, not the UI thread, since a
  1080p PNG takes tens of ms. A `.jpg` path makes mpv's write faster.)
- **Accept the picture until the switch-on static ends** (~1 s after she arrives). Arriving later, it
  waits for the next switch-on. The glass never swaps a card for a screenshot mid-watch.
- **A picture lasts its act.** For the next switch-on: same now-playing file and a failed or black
  request means reuse the last good picture; a different file, or none, means a card. That keys
  staleness on the film rather than a clock. A 10-minute-old frame of the same episode is still
  "the film". A frame of last night's episode isn't.
- **Surfing** (script.rs:1399-1436): flick through cards and colour bars, with at most one film
  channel per surf, never one screenshot per flick. Today SURF rests on `Snow(0)` (script.rs:1428-1434)
  and WATCH holds `Snow(0)` (script.rs:1386-1392). Under D7 both should rest on the held picture (the
  film if there is one, else a card), keeping the sunrise "ooh" as one flick. Landing on your film
  as the "ooh" target would be a nice beat.

### 4. The picture shows only while she watches (which is the same as "whenever it's on").

The TV is on only during a script act whose key carries a `Prop::Tv` (`Osaka::prop`,
osaka.rs:6262-6276). Otherwise it's drawn off, with the dark `tv-screen` glass (art.rs:1134,
props.svg:53-56). So "while she watches" and "whenever on" are the same thing today. **Keep it that
way:** a lit picture in an empty room (school, away, asleep) would be an unexplained bright patch, and
she wouldn't leave the TV on anyway.

### 5. Paused versus playing: show both. The rule above already bounds the change.

- Nothing playing (a paused or loaded film, the visiting case): every screenshot is the same paused
  frame, so it never changes at all. This is the best moment: she's watching what you just stopped.
- Playing (resident Osaka, design.md:2142-2146): the user's eyes are on mpv, and one picture per
  switch-on is a change her action explains. No need to suppress it.
- No spoiler risk: her home is local, never synced (design.md "Her record"), and the frame is
  wherever the local player already is.

### 6. Her reaction: no line on a picture change. At most a face, and one rare recognition line.

- A line per change spends the beat budget (`LINE_BUDGET` 8 a day, `LINE_COOLDOWN_MS` 10 min,
  mind.rs:981-984) on something the user can barely see. It also turns a quiet change into a loud one,
  against this phase's theme.
- **Free:** at a switch-on that lands on the film, `Face::Curious` → `Happy`, with no bubble.
- **Optional (ask the user):** once per now-playing file per visit, a line from a small pool outside
  the budget ("Ooh, I know this one!", "Hey, it's the thing you're watching!"). This is the one line
  that pays for itself, because it tells the user *why* their anime is on her TV. Drop "ooh, this
  part!": she can't know which part, and it would fire on a frame the user can't make out.

### 7. Mirroring flips the film.

`tv_scene` draws the picture inside the `facing` mirror group (art.rs:1516-1519 build `mirror`; the
picture sits in `<g{mirror}>` at art.rs:1540). A TV facing left would show the screenshot flipped. At
27 px you'd hardly notice, but hardsub text or a known shot would give it away.
**Amendment:** composite the film picture un-mirrored. Mirror the glass rect's position and paste the
image as-is. The drawn cards can stay mirrored.

### 8. A picture isn't a `Copy` channel; give it an identity in the image key.

`Channel` is `Copy + Hash` (art.rs:1382-1383) and `Look::Tv(Channel)` keys the image cache
(graphics.rs:77, 115). An `Arc<RgbaImage>` can't live in it.
**Suggest:** `Channel::Film(u32)`, where the u32 is a generation bumped per accepted picture. The
picture itself is held on the shell/`IdleView` side and looked up at render. Every frame then reuses
the cached image, and a new picture is exactly one new key. This keeps `Prop::framed` (script.rs:113)
and the scripts pure. (The feasibility lens may have more to say here; it's flagged because it
decides whether "one picture per act" holds by construction.)

### 9. Test and review fixtures: don't ship anime frames.

The "tests use a fixed picture" fixture, and any review sheet committed to docs, should be generated
(a synthetic gradient with a face-like blob) or a CC-BY frame (Sintel or Big Buck Bunny), not a
frame of a commercial episode.

### 10. Shopping versus D7's own test (minor).

D7 keeps the shopping channel's picture, which bobs at `CHANNEL_FRAME_MS` = 400 ms (script.rs:91,
116; osaka.rs:6274). D7's test, "no act longer than 30 s flips cells faster than `USE_FRAME_MS` after
its first 10 s", fails on a shopping act longer than 30 s (Watch draws 20-45 s). Either Chiyo-chichi
holds still after his hook (Span `Upto(2,5)`, script.rs:1439-1445) or the test exempts the hook
only. Say which in D7.

### 11. ASCII: a held pair, optionally tinted. Don't invest.

ASCII is dev/test-only (memory note; no art review). Give the film a held glyph pair in
`screen_glyphs` (mod.rs:3940), e.g. `['#', '#']`. In truecolor, optionally tint each of the two
screen cells (room.rs:1046-1051) with the mean colour of its half of the picture. It's cheap and
consistent, but optional.

## What to show the user

1. **`d7-mock-1x.png`** (true size) and **`d7-mock-1x-nn3x.png`** (3× nearest, to judge) in this
   directory. Top row: the four approved drawn cards. Then ten real frames (the source thumbnail at
   right) in 8 treatments: plain, letterbox, poster8, poster5, zoom1.5+poster8, poster8+CRT,
   **zoom1.3+lift**, zoom1.3+lift+sheen. The questions for the user:
   (a) Is a picture this small worth it over the drawn cards?
   (b) Lift with or without the sheen?
   (c) One picture per switch-on: OK?
   (d) The once-per-film recognition line: yes or no?
2. How it was made, cheaply (`mock.py` here, about 100 lines). Pillow + numpy + resvg via
   `nix-shell -p "python3.withPackages(ps:[ps.pillow ps.numpy])" resvg`.
   - Frames come from `nix run nixpkgs#ffmpeg -- -ss T -i FILE -frames:v 1`.
   - The TV is props.svg's `#tv` rendered at 54 × 76 on the room background (30, 33, 39), with the
     glass filled magenta and clipped by the rx 8 rect plus its outline.
   - The treated 27 × 22 picture is blended in wherever the render is magenta. This is the same pixel-space
     composite recommended for the build.
   - Swapping in frames from the episode the group is actually watching makes the most convincing
     review.
3. **Before step 12 lands:** an in-tree sheet in the style of `stillness_sheet` (art.rs:2371),
   rendering `Channel::Film` from a CC-BY fixture through the real path. It also proves the mirror
   fix and the whole-pixel composite.
