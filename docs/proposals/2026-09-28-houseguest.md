# Proposal: The Houseguest (idle Osaka)

Status: **DRAFT, 2026-09-28.** Brainstormed by five parallel research
passes (Osaka canon, prior art, the TUI as terrain, progression/AI,
terminal animation craft) and merged here. Nothing is implemented; see
[Open questions](#open-questions).

## Summary

When the client is fully idle, a stick-figure Ayumu "Osaka" Kasuga
(*Azumanga Daioh*) wanders into the terminal and makes herself at home
among the panes: walks the pane borders, tidies the chat log by dragging
lines out, gets a sofa, a desk for homework she never finishes, a bed,
a kotatsu in winter. She is a **pure visual overlay**: she reads the
rendered frame and the pane rectangles and paints over them, and never
touches app state. Any activity ends the visit with a ~2.5 s matrix-style
rain built on the spoiler scramble, settling cell by cell back to the real
UI.

The direct ancestor is the 2004 fan-made screensaver *Ayumu Kasuga's Mail
Order Life* (hirahira.net), where Osaka potters around her room, goes out
to a part-time job, and buys furniture from Chiyo-chichi's shopping
channel on her TV. We borrow that backbone (see
[Progression](#progression-the-mail-order-life)).

## The seam (architecture contract)

The guest is a separate object owned by the shell loop, next to
`Renderer`. It is not a tui-realm component and has no `Msg` variants.

```rust
draw_frame(adapter.raw_mut(), |frame| {
    ui.draw_with_renderer(frame, &mut renderer);
    guest.paint(frame.buffer_mut(), &ui.idle_view(), now);
});
```

- **Reads**: the finished frame buffer (symbols + styles) and an
  `IdleView` built by `Ui`. **Writes**: only that frame's buffer, after
  the real UI has painted it. Nothing flows back into `Ui`.
- **Ticks**: every guest state carries `next_change_at`; the shell folds
  it into the timeout as `min(ui.next_tick_hint(), guest.next_tick())`.
  Timeout ticks call `guest.advance(now)`; a redraw happens only if it
  reports a visible change. No fixed frame rate.
- **Input**: every `UiInput` still goes to `Ui` unchanged. The shell
  additionally calls `guest.activity(now)` for inputs that count as
  activity (below). Mouse clicks over her hit the real UI underneath.
- **Removal**: deleting the module and the two shell lines removes the
  feature entirely.

### `IdleView`

A read-only value `Ui` computes on demand:

| Field | Source |
|---|---|
| Pane rects by identity: `chat`, `subtitles`, `series`, `users`, `playlist`, `health`, `status`, `keybar` | the arranged `app` scene slots (`self.panes` / `scene.slot`) |
| Image rects | the renderer's `record_image_regions` record (needs a getter) |
| Playback idle | snapshot: nothing playing, no pending ready-check |
| Last chat activity | newest chat / IRC / `/me` line timestamp (not system/narrator/subtitle lines) |
| Overlay state | modals empty, `layout_tools` off, no hashing / nyaa-import overlay |
| Held selection | chat selection or pane search active |
| Colour depth | `ui.color_depth` |

Layouts are user-authored (docs/ui-layouts.md), so every behaviour
resolves against the rects actually present and skips when a pane is
absent, zero-sized, or unbordered (then the rect edge is an invisible
ledge).

## Idle and activity

**A visit may begin** when all hold for the configured threshold
(default 5 min, see open questions): playback idle, no chat activity, no
local input, no overlay, no held selection, terminal ≥ 60×18.

**Activity** (ends a visit): any key, mouse, or paste event; a new chat /
IRC line from anyone; playback starting; any modal or overlay opening.

**Not activity**: snapshot churn (health metrics, sync age, marquee,
download progress), subtitle/system lines, layout reload.

**Resize** is one rule with two halves: during a visit it re-anchors
(pause, re-resolve anchors, drop stale glyphs, re-drop Osaka onto the
nearest floor below her); during a dissolve it aborts instantly with no
animation, since the frozen geometry is gone.

## The room model

Everything she owns is stored **pane-relative**, never as absolute cells:

```
Anchor { surface: Pane(id) | Screen, edge: Top|Bottom|Left|Right|Interior,
         frac: 0..1 along the edge, offset: (dx, dy) }
Prop   { kind, anchor, footprint }
```

Screen positions are recomputed from the current `IdleView` each frame,
so resizes and layout changes carry her room along. A prop whose pane
vanished or became too small goes "into the closet" (hidden, still owned).

**Displaced glyphs** (holes she dug, letters she carries, lines she
dragged) record what they expect underneath:

```
Displaced { source: (pane, row, col), glyph, style, expected: (symbol, style),
            pos, state: Held | Placed | Falling | Returning }
```

**Validation rule**: each frame, a displaced glyph whose source cell no
longer shows `expected` is dropped — the hole closes, the real content
shows through, and she turns to look at it with a `?`. This one rule
makes clock ticks, marquee steps, scrolling logs and new content safe;
she can never paint a stale hole over changed text. Cap: ≤ 60 displaced
glyphs per visit.

**Surface map** (rebuilt when rects change): walkable = pane top/bottom
borders, the `health` row, the `keybar`, prop tops; climbable = vertical
borders; forbidden = image rects + 1-cell margin, the chat input row, the
terminal cursor cell. The `status` slot (Player Status) is **the TV**.

**Wide glyphs are atomic 2-cell bricks** everywhere: harvest, carry,
drop, and dissolve all move or settle both halves together. No frame ever
contains half a wide glyph. (She carries CJK titles two-handed, as heavy
bricks.)

## The brain

**Utility-scored scenes over interruptible keyframe lists** — The Sims'
"smart objects" plus Johnny Castaway's scene pool.

- **Needs** (0..1, slow, never punitive): `sleepy`, `hungry`, `bored`,
  `tidy_urge` (rises with foreign glyphs in the chat rect), `social`
  (drives cameos). They weight choices; she never sickens, starves, or
  guilt-trips (the Tamagotchi lesson).
- **Advertising**: panes and props advertise affordances ("chat: messy",
  "users: people to wave at", "sofa: rest", "desk: homework", "TV:
  watch"). Adding a prop adds behaviour without touching the brain.
- **Selection**: `score = base × need_fit × time_fit × calendar_fit ×
  room_fit × cooldown × novelty`; weighted-random among the **top few**,
  never argmax (robotic) and never flat random (slot machine).
- **Scenes** are finite `Vec<Keyframe>`; each keyframe is a complete
  renderable pose + position + glyph overrides + optional bubble, with a
  duration. Rendering is a pure `frame(&Visit, now) -> Overlay`. No
  coroutines, behaviour trees, or exit handlers: **any instant is a safe
  cut**.
- **Room mutations commit at scene start.** "Sofa delivery" writes the
  sofa into the room before the 40 s carrying animation begins; an
  interrupt loses nothing.
- **Locomotion** uses an eSheep-style successor graph: when an animation
  ends, pick a weighted successor; on hitting a border, take a *border*
  transition (climb, turn, peer over); when the floor vanishes, take the
  *gravity* transition (fall, land dazed `@_@`).
- **Determinism**: visit RNG seeded from `(master_seed, visit_index)`;
  clock and calendar derived from the injected `now`, never `std::time`.
  All pacing is wall-clock (the Mail Order Life famously runs absurdly fast
  on modern PCs because it was frame-timed).

## Progression: the Mail Order Life

Borrowed from the original screensaver:

1. Sometimes she **leaves for her part-time job** (walks off an edge);
   the room stands empty-but-furnished, and she returns later with a
   shopping bag.
2. Occasionally she sits before the TV (`status` slot) and
   **Chiyo-chichi's shopping channel** comes on: a round orange cat head
   advertising one item ("Hello everynyan. Today: sofa."). She buys it.
3. A box slides in from the edge; she unpacks it; it becomes a prop, and
   **each prop unlocks scenes** (sofa → naps, TV-watching from the sofa;
   desk → homework; bed → proper sleep; bookshelf → reading; fridge →
   snacks; lamp; window; cat bed → Kamineko may stay).
4. Unowned items are also built the hard way: harvested letters and
   borrowed border segments (see [Building](#building-from-harvested-letters)).
5. Once she owns the full catalogue, a one-time **credits roll** scrolls
   through the chat pane listing her items, then the TV cracks. Life
   continues (Castaway's raft-and-parachute lesson: the ending is a beat,
   not a stop).

Rooms cap at ~8 placed props; the rest rotate through the closet
seasonally (kotatsu replaces the sofa December–February).

## Routine and calendar

Local wall clock, injected. School-day absence is a feature: evening
visits feel like her real home time.

| Local time | School day | Weekend / vacation |
|---|---|---|
| 07:00–08:30 | wakes, toast in mouth, sprints off to school | sleeps in |
| 08:30–15:30 | absent; rare "forgot lunch" dash-in | lounging, outings |
| 15:30–18:00 | home, snack, flops on sofa | same |
| 18:00–20:00 | dinner, TV | same |
| 20:00–22:30 | homework at desk, dozes onto it | reading, drawing |
| 22:30–07:00 | sleep; `z` drifts every ~20 s | sleep (later) |

Holidays are **details inside normal scenes**, not takeovers (Castaway's
banner on the palm tree). Calendar-locked content bypasses rarity and is
guaranteed once on the first visit that day.

| Date | Content | Canon |
|---|---|---|
| Jan 1–3 | New Year's dream is always the flying pigtails; hatsumōde, omikuji read upside-down, 10-yen coin shrine | Ep 8, Ep 25 |
| Late Jan–Mar | exam season: chopstick ritual every visit, study desk | Ep 25 |
| Feb 3 | Setsubun: beans at the keybar ("oni wa soto") | — |
| Mar–Apr | hay-fever sneezes common, tissue pile, eyedrops lying down; cherry petals, yawning contest | Ep 26, Ep 19 |
| **Apr 8** | her "debut day" (anime premiere, 2002): transfer-student intro variant | — |
| Jul 7 | Tanabata: bamboo on a pane border, wish slip | — |
| ~Jul 20–Aug 31 | summer vacation: no school; beach float, swim ring drifting away; last week: homework panic | Ep 4, 14 |
| Sep–Oct | sports festival: shoe-kick forecast, losing to a child, "Team Sea Slug!" flag; typhoon joy | Ep 3, 6, manga |
| **Sep 30** | final episode anniversary (2002): graduation bow | Ep 26 |
| Oct 31 | Halloween: sheet ghost with two dots | — |
| Oct–Nov | culture festival (configurable weekend): café stand in the playlist, penguin feeding | Ep 16 |
| Dec | kotatsu season; Rudolph critique | Ep 17, 24–25 |
| Dec 24–25 | `^` tree in a corner; a present under it next morning | — |

Osaka has **no canon birthday**; we do not invent one.

## Rarity and pacing

| Tier | Share of selections | Pity bound (cumulative idle) |
|---|---|---|
| Common | ~65% | — |
| Uncommon | ~25% | — |
| Rare | ~8.5% | first sighting within ~6 h |
| Legendary | ~1.5% | first sighting within ~40 h |

- At most **one unseen rare-or-better per visit**, so content reveals
  gradually.
- Unseen rares prefer the **first 10 minutes** of a visit (when someone
  may still be glancing), except overnight-premised ones.
- Line pool with cooldowns so she rarely repeats herself (the Ukagaka
  staleness lesson).

## Behaviour catalogue

Tiers: **C** common, **U** uncommon, **R** rare, **L** legendary,
**Cal** calendar. ★ = signature. Canon references from the Miraheze
episode pages and Wikipedia's episode list.

### Arrival

1. ★ **Edge peek** — one eye `(._` at a pane edge, blinks, retreats, then
   she walks in and bows stiffly: "Nice to meet you." [C]
2. **Falls from the sky** — drops from the top border, lands on the
   keybar, lies there, "...I'm OK." [U]
3. **Trapdoor** — climbs out of the keybar between two key hints. [R]
4. **Nameplate** — `Kasuga` appears on a border, a scribble overwrites it
   with `OSAKA`; she thinks the nickname is too simple (Ep 1). [U]

### Terrain

5. ★ **Ledge walking** — along pane top borders; at a corner she stops,
   peers over, wobbles. [C]
6. **Climbing the divider** — shins up a vertical border, `╫` rungs under
   her hands. [C]
7. **Falling off** — walks off a border run, falls under gravity, dust
   puff, `@_@`. [C]
8. **Border sag** — standing mid-border, the span bows to `╲_╱` under
   her weight (box-drawing set only). [U]
9. **Keybar as skirting board** — her default floor; sits with legs
   dangling, kicking; each kick nudges a key-hint letter down a row. [C]
10. **"Escape the Earth" / "Laundry"** — back hip circle on a horizontal
    border, then hangs folded over it like a futon (adult Osaka as a PE
    teacher, *Yotsuba&!* vol. 16). [R]
11. **Jump rope** — two borders as rope-turners; trips half the time
    (Ep 12). [U]
12. **Hanging laundry** — strings `╌` between two corners in an empty
    pane, hangs socks. [U]

### Tidying and mischief

13. ★ **Tidying the chat** — drags playlist titles / old lines out of the
    chat pane with a tug rhythm (brace, heave), stacks them on the keybar,
    dusts her hands. Heavier lines are slower. [C]
14. **Sweeping** — broom `/` sweeps the last lines leftward into a heap;
    a few glyphs always slip back out of the dustpan (Ep 12). [U]
15. **Oops** — notices the gap she left and drags the line back, "sorry!".
    [U]
16. **Harvesting punctuation** — pockets every `.,!` in view; hands them
    back one by one, some in the wrong spots, fixes them on a second
    glance. [U]
17. **Letter swap** — swaps two adjacent letters (`teh`), giggles, waits,
    nobody noticed, swaps back. Always self-reverting. [U]
18. **Magnet** — every `o` in chat slides toward her; she gets scared and
    shoos them home. [R]
19. **Shelf rearranging** — swaps two playlist rows, notices the current
    entry's highlight moved, panics, swaps back. [U]
20. **Getting distracted halfway** — carrying a letter home, a moth `ʚ`
    flits past (verify width); she follows it and leaves the letter in the
    Users pane until the next tidy. [U]

### Reading the screen

Semantic reading is best-effort: concatenate a row's cells per pane,
skip wide-glyph continuation cells, match case-insensitively, accept
false negatives.

21. **Sounding out** — stands under a chat line and reads it word by word
    (reverse-video highlight walks along), mouth opening and closing. [C]
22. **Food words** (`ramen`, `onigiri`, `bread`, `curry`, `watermelon`…) —
    walks over and eats the word letter by letter, `nom`, restores it
    later with a burp. [U]
23. **America** in chat, a user name or a title — lands like a plane
    `-o-`, tiny `o` figures pop up in the Users rows: "Hallo! Hallo!",
    "America-ya!" (Ep 20). [R, boosted by trigger]
24. **Cats** (`cat`, `neko`, `=^.^=`) — crouches and pets the word, calls
    it "Yamapikarya!" (Ep 21). [U]
25. **Azumanga** in the playlist or series list — gasps, "that's me?" [R]
26. **Blue Three** — a title containing `3`: "Blue Three... so where's
    Blue One?" (Ep 13, Bruce Lee). [R]
27. **Long words** — tries to pronounce any ≥ 10-letter word, garbles it.
    [U]

### The neighbours (Users pane)

28. **Waving** — waves at each name in turn; the brightest-styled row
    waves back (first letter lifts for a frame). Dim rows: she tiptoes
    past, `shh`. Styles compared relatively within the pane. [C]
29. **Counting** — counts the names on her fingers, loses count at 4,
    starts over. [C]
30. ★ **Contagious yawn** — her mouth grows `o`→`O`→`( )`; an `o` in a
    user name yawns, then the next… (Ep 19). [U]
31. **Explain this part to me** — at the desk, turns to each user name in
    turn with homework questions (Ep 22). [C at homework time]
32. **Hiccups that jump** — `hic!` bubbles, remedies fail, the `hic!`
    hops onto a user name and stays (Ep 2). [U]
33. **Kanji trivia to a silent friend** — "Seals are 'sea leopards'…" at
    the Users pane, which says nothing back (Ep 5). [U]

### The TV (`status` slot)

34. **Watching TV** — sits cross-legged before it; the interior shows a
    slow `.:'` static loop or harvested title letters scrolling. [C]
35. **Channel surfing** — static, a colour-bar card from the palette, a
    tiny sunrise. [U]
36. **Whacking it** — taps the border; the box shakes one column. [U]
37. ★ **Chiyo-chichi's shopping channel** — the progression engine (see
    above). [U, paced by progression]

### Food

38. ★ **Chopstick ritual** — `||` splits to `| |`; a clean split sparkles,
    a bad `|/` makes her droop: "Ya gotta hold 'em by the ends!" (Ep 25).
    [C; before homework, and on the hour]
39. ★ **Sata andagi** — holds a round `@`; every question gets "Sata andagi.", 5–7 times, happier each time; eats
    it `@`→`c`→`(`→gone (Ep 21). [U]
40. **Curry or hashed beef** — tastes two bowls: "...same." (Ep 19). [R,
    12:00–13:00]
41. **Chili croquette** — turns red, steam `~`, lies flat: "Thought I'd
    die." (Ep 2). [R]
42. **Five breads** — dithers between five breads on a string while the
    rest of the "race" finishes (Ep 23). [R]
43. **Melon bread** — gets yakisoba bread instead: "Melon bread..."
    (Ep 12). [R]
44. **Toast morning** — toast in mouth, sprints off for school. [C, school
    days 07:00–08:30]
45. **Forgot lunch** — dashes in during school hours, grabs a bento, out.
    [U]

### Daydreams and overthinking

46. ★ **Eye floaters** — a `o` drifts across a pane; her head follows it,
    she lunges and misses; it slides away whenever she looks at it
    ("trackin' my eye bubbles", Ep 2). [C]
47. ★ **Chasing the cursor** — stalks the blinking terminal cursor, which
    is always one cell ahead; falls asleep next to it (the Neko lesson).
    [U; only when a cursor is visible outside the input row's protected
    cells]
48. **Spacing out** — stops mid-stride, `...` for 8–20 s, carries on. [C]
49. ★ **Flying pigtails** — a thought bubble with a pigtailed head; the
    pigtails spin off like rotors and fly around the panes; she panics
    and sticks them back (Ep 2, Ep 8). [U; always on Jan 1]
50. ★ **Panda debate** — draws a panda in an empty pane, three worsening
    attempts: "Black spots on white? Or white on black?" (Ep 17). [U]
51. **Scary story** — screen dims except her: "...I smelled a fart that
    wasn't mine." (Ep 17). [R, after 23:00]
52. **Escalator or elevator** — "The box one's the escalator. ...No?"
    (Ep 14). [R]
53. **Rooftop** — stands on the very top border, arms out: "Feels like ya
    could fly away..." Never falls from this one. (Ep 12) [R]
54. **Riddle queen** — a pun riddle appears on the health row; she
    answers instantly (her one academic talent). [U]
55. **Left or right** — an arrow in the keybar; she mimes chopsticks to
    work out which hand, walks the wrong way (Ep 15). [U]

### Physical comedy

56. ★ **Shoe-kick weather forecast** — kicks her shoe up; instead of
    landing it sticks to something moving (the marquee, a progress bar);
    she stares after it (Ep 6). [U, boosted when something animates]
57. ★ **Tiny sneeze** — `(._.)`→`(o_o)`→`(-o-)`→`(>_<)` "...chu"; the
    recoil knocks 2–4 glyphs off their cells, they fall, she quietly puts
    them back (Ep 26). [U; C in Mar–Apr]
58. **Loses a race to a child** — a pigtailed `o` overtakes her along the
    keybar; enormous determination, no progress. [U]
59. **Float like a corpse** — an empty playlist becomes a pool; she drifts
    flat `—o` (Ep 4). [R]
60. **Swim ring drifts away** — while she isn't looking; she returns
    later and looks around (Ep 14). [R, summer]
61. **Free tissues** — tiny vendors keep handing her packets; the pile
    grows until she gives them to the Users list (Ep 14). [U, spring]
62. **Rain dance** — one `'` raindrop falls, a flash, her hair briefly
    `%` (Castaway). [R]

### Home life (prop-gated)

63. ★ **Homework** — at the desk, writes, the `…` gets heavier, asleep on
    the paper; the test paper shows **42** (Ep 13, 22). [C, 20:00–22:30]
64. ★ **Kotatsu** — only her head visible, for hours; mikan on top
    (Ep 24–25). [C, Dec–Feb evenings]
65. **Sofa nap** — curls up, `z` drifts; falls asleep standing if she has
    no sofa. [C]
66. **Blank-line nap** — lies in the widest blank run in chat; a real
    repaint of that run is her alarm clock. [U]
67. **Lamp** — switches it off (dim) before bed. [C, bedtime]
68. **Window** — day sun / night moon from the wall clock, a cloud
    drifting one cell per 5 s. [ambient]
69. **Reading** — pulls a playlist row out like a book spine, reads,
    slides it back. [C]
70. **Closet reveal** — cycles through past seasonal props. [R, owns ≥ 10
    items]

### Sleep and dreams

71. ★ **Chiyo-chichi dream** — asleep ≥ N minutes, a floating orange cat
    head drifts over the playlist: "Hello everynyan. How are you? Fine
    sankyu." Her bubble: "OH MY GAH". "I wish I were a bird." (Ep 25). [R,
    guaranteed once she has slept long enough]
72. **Office dream inversion** — asleep on the sofa, a bubble shows a tiny
    classroom where dream-Osaka is asleep at her desk dreaming of the sofa
    (Castaway). [R]
73. **Wavy borders** — pane borders go wavy while she sleeps; one pane
    briefly shows sky. [R, asleep > 2 h]

### Cameos

At most one per visit, long cooldowns, keyed off `(date, master_seed)`
so each friend's Osaka gets different visitors (comparing notes in chat
is the social payoff).

74. **Chiyo** — they do homework together; Chiyo tidies properly
    (everything snaps into perfect alignment). [U, homework hours]
75. **Tomo** — barges in, kicks the sofa across the room, yells, leaves;
    Osaka blinks for 30 s. [R]
76. **Yomi** — refuses snacks (diet) while Osaka eats. [R]
77. **Kagura** — jogs along the keybar, push-ups, jogs out. [U, mornings]
78. **Yukari-sensei** — a car glyph screeches along the health row;
    Osaka's hair stands up. [R, school days 08:00–08:30]
79. **Kimura** — appears at a pane edge and stares; she waves, he doesn't
    move. [R, once per season max]
80. **Sakaki** — reaches to pet a cat; it bites; she is quietly sad. [R]
81. **Kamineko stays** — the biting cat follows Sakaki in, stays behind,
    and sleeps on the sofa for the rest of the visit. [L, after ≥ 2 Sakaki
    cameos]
82. **The long stay** — whole cast around the kotatsu (winter) or at the
    beach (summer). [L, ≥ 8 cameos and visit > 4 h]

### Milestones and endings

83. **Leaves on her own** — after > 3 h (not night), waves and walks out;
    the furniture stays. [U]
84. **Anniversary photo** — `[:)]` framed on a wall at 30 days, 100
    days, one year since her first visit. [L]
85. **Credits** — the full catalogue owned: credits scroll in chat, TV
    cracks, life goes on. [once]

Cut on review: the frying-pan/knife wake-up (Ep 22 is canon, but a stick
figure walking at a sleeper with a knife reads badly out of context);
mosquito musings (no source found); anything requiring block elements.

## Sprites and props

Body is **pure ASCII**; props use the **box-drawing set the app already
renders** (`─│┌┐└┘├┤┬┴┼╭╮╰╯` and `╱╲╫╌` sparingly). No block elements, no
emoji, no ambiguous-width symbols (`·°‾♪♥☆`). Sprites are defined facing
right and mirrored through a glyph table; anchor is bottom-centre.

Osaka at 3 rows reads as "stick figure"; she reads as *Osaka* through
the vacant `._.` face, her pacing (stops mid-stride, stares, wanders off
the other way), and her lines.

```
M (default, 5x4)      walk right             S (3x3, tight panes)
(._.)                 ( ._)  ( ._)             o      o
/|V|\                 /|V|>  ||V||            /V\    /V>
 /_\                   /_\    /_\             / \    / \
 / \                   / \     |
```

Faces are the cheapest animation (one cell row): `._.` vacant, `-_-`
blink, `u_u` asleep, `o_o` surprised, `^_^` pleased, `>_<` sneeze,
`@_@` dizzy, `._.;` sweat. A peek variant `(._.)` shows just her head
over a border in very tight terminals.

```
sofa          desk + chair         kotatsu         TV (the status slot)
 ╭───╮            [] @              o             existing border;
╭┤   ├╮       │   ┬──────┬      ──────────        interior shows .:'
╰┴───┴╯       ├─┐ │      │      (~~~~~~~~)        static when on
```

### Building from harvested letters

1. **Blueprint** (600 ms): dim `.` outline of the prop.
2. **Harvest**: she lifts letters (keeping their original style — you can
   see she's carrying bob's message in bob's colour) from old chat lines,
   series rows, the keybar; or borrows a run of `───` from a pane border,
   leaving a visible gap. Never the status slot or the input row.
3. **Deliver**: one letter per 250 ms, bottom row first, falling into
   place.
4. **Cure**: each finished row sets left to right, 2 frames of scramble
   noise then the box glyph.
5. **Conservation**: non-space prop cells = letters harvested; the screen
   visibly runs out of text as the room fills.

## Motion and scheduling

No fixed fps; these are targets that `next_change_at` produces:

| Phase | Redraws |
|---|---|
| held poses (≥ 70% of a visit) | 0.1–0.3/s (blinks every 4–9 s, `z` cycle) |
| walking / carrying | 3 / 2 cells/s, one redraw per cell |
| falling | one redraw per row crossed, g ≈ 28 rows/s² |
| dissolve (2.5 s) | ~16/s |

Visit average ≲ 1 redraw/s. Asleep she needs almost none.

- **Dragging a line**: 600 ms brace/heave cycle, `600 ms × (1 + len/40)`;
  off-row moves snake-follow her hand's path, so a sentence goes vertical
  when she climbs.
- **Dropped glyphs**: gravity, rest on the first non-blank cell below, a
  sandpile slide off single-glyph peaks; no bounces.
- **Speech bubbles**: placement search around her head scored by what
  they'd cover (blank 0, border 1, text 3, status/keybar 6, input row /
  off-screen never); full / compact / inline forms; ≤ 24 chars; she
  freezes while speaking; shown whole for 1.2 s + 60 ms/char.
- **Colour**: fg and modifiers only, never bg. Body default fg, face
  BOLD, the `V` ribbon LightRed (a `USER_PALETTE` hue, never a
  state-meaning colour). Props DarkGray / muted like unfocused borders. At
  most one other accent on screen.

## The exit dissolve (~2.5 s)

Reuses the spoiler scramble (`dessplay_core::spoiler::scramble`,
`spoiler::seed`) and the spoiler tease's discipline: frames derived from
wall time, never per-tick counters.

- At activity time `T0`, freeze the composite `C` (her world). Each draw
  renders the live real frame `R`. Only the dirty set `C ≠ R` animates;
  clean cells never flicker.
- **Fast lane**: a cell whose real content changed since `T0` settles
  immediately. Whatever the user is doing (typing, a modal the key
  opened, new chat) is **never scrambled**.

| t − T0 | |
|---|---|
| 0 | freeze `C`; Osaka startled `o_o` with `!` |
| 240–720 ms | she waves, "mata ne~" |
| 400 ms → | rain starts, rippling outward from her column |
| 720–960 ms | she poofs first: noise, `.*.`, gone |
| ≤ 2400 ms | every column settled, by construction |
| 2400–2500 ms | safety band: anything still ≠ `R` snaps |
| 2500 ms | overlay dropped; tick hint back to lazy |

Per column `x`: drop start `s_x = 400 + 12·|x − osaka_x| + (h(x) mod
160)` (clamped), trail length 3–6 rows, a global speed chosen so the
last column settles by 2400 ms. A cell ahead of the drop shows `C`;
under the head, bold noise; in the trail, churning noise (dimming);
behind it, `R`. Settle time is a pure function of `(x, y)`, so settled
cells never un-settle.

**Noise source** (core change): `scramble` passes ASCII punctuation and
whitespace through, which would leave her `/|\` body standing. A small
helper beside it in `dessplay-core/src/spoiler.rs`: scramble `R`'s glyph
if it is alphanumeric/non-ASCII (so the trail already has the shape of
the text it becomes), else `C`'s, else a letter; box-drawing in `R`
draws from a box class so borders "re-knit" rather than turning into
letters.

Colours: TrueColor head `Rgb(210,255,215)` bold, trail ramping green to
muted; Limited depth LightGreen then DIM. Background untouched. Further
input during the dissolve never restarts or extends it.

## Settings

Under F3 → Playback & display (next to Roguelike effects), local only:

- **Houseguest**: Off / Visits only (no persisted room) / Full.
- **Arrival after**: idle threshold (default: open question).
- **Night stays**: on/off (off = she leaves at bedtime instead of
  sleeping over, for shared screens left on overnight).
- The dissolve follows the Full/Reduced/Off pattern of `RoguelikeEffects`:
  Reduced = a plain 300 ms scramble-to-real without rain; Off = instant
  cut.

First visit ever: a shy intro scene, plus one local system line in chat:
"Someone seems to have moved into your terminal. (F3 → Playback &
display → Houseguest)". A CHANGELOG entry announces the feature.

## Persistence

A small **local, never-synced** record in the client's local storage, the
same tier as `layout_sizes` and the roguelike save (not CRDT state). JSON
envelope with a version field and `#[serde(default)]` fields; unknown
items ignored.

`master_seed`, `visit_count`, cumulative idle hours, first-visit date,
`seen: set<SceneId>`, per-scene last-fired timestamps, pity counters,
owned items with anchors, closet contents, `credits_done`,
`intro_done`.

Each client has its own Osaka; nothing syncs. A settings action
"Osaka moved out" resets the record.

## Module layout

```
dessplay/src/houseguest/
  mod.rs        Guest: activity(), advance(now), next_tick(), paint()
  idle.rs       IdleView + visit gate
  terrain.rs    surface map from rects + buffer; wide-glyph bricks
  room.rs       anchors, props, displaced glyphs + validation
  brain.rs      needs, advertising, scoring (pure)
  scenes/       keyframe tables per category (data)
  calendar.rs   date table
  sprites.rs    sprite + prop glyph tables, mirroring
  bubble.rs     placement search
  dissolve.rs   the exit, pure fn of (C, R, T0, t)
  ledger.rs     persisted record
```

`Ui` gains `idle_view()`; the shell gains the paint call, the tick-hint
term, and `activity()`. `dessplay-core::spoiler` gains the noise helper.

## Testing

Per docs/testing-strategy.md: seeded RNG, injected clock, no sleeps.

- **Snapshots**: `TestBackend` frames of "seed N at t = T" over a fixed
  UI fixture; one per signature scene.
- **Dissolve properties** (proptest over random `C`, `R`, sizes, Osaka
  positions, `T0`): frame at `T0 + 2500` equals `R` exactly; settled
  cells never un-settle; no frame holds half a wide glyph; output is
  identical for the same inputs; fast-lane cells are never scrambled.
- **Long-visit property**: an 8-hour visit simulated in milliseconds over
  random layouts and resizes: forbidden rects (input row, image rects)
  never overdrawn; displaced count ≤ cap; every validation-failed glyph
  dropped within one frame; every legendary within its pity bound across
  simulated weeks; any instant yields a valid overlay.
- **Idle gate**: each activity source ends a visit; each non-activity
  source (health, marquee, system lines) does not.
- **Room anchors**: a prop survives arbitrary resize sequences without
  landing out of bounds or on a forbidden rect.
- **Ledger**: round-trip, version mismatch, unknown items.
- **Perf** (`tests/perf.rs`): idle CPU with the guest walking and asleep
  stays within budget; input-to-draw latency with a visit active is
  unchanged (the dissolve must not delay the first real frame).

## Phasing

1. **Seam + skeleton**: `IdleView`, visit gate, shell hook and tick hint,
   terrain map, Osaka walking/climbing/falling on borders, the dissolve
   with its property tests, settings, perf test. Shippable on its own.
2. **Tidying and mischief**: displaced glyphs with validation, dragging,
   sweeping, sneeze scatter, letter swap, bubbles, the brain with needs.
3. **The room**: props, anchors, building from letters, ledger,
   Chiyo-chichi's shop and progression, routine.
4. **Colour**: calendar table, screen reading, dreams, cameos, rarity
   tiers and pity, credits.

## Open questions

1. **Default on or off?** On gets changelog discovery and the joy of
   surprise; off is safer for people who leave the client on a shared or
   streamed screen.
2. **Idle threshold default**: 5 min? 10?
3. **Remote chat**: should a friend's message end the visit (current
   draft; she scurries off) or merely make her pause and look at it,
   ending only on local input?
4. **Persistence default**: Full (room grows over weeks) or Visits only?
5. **Name**: "Houseguest" for the setting; is a character-neutral name
   preferred in the UI, with Osaka as the (only) guest?
6. **Cameo scope**: other characters in phase 4, or keep it Osaka-only?

## Rejected alternatives

- **Integrating with the layout renderer / tui-realm components**: would
  need `Msg` variants, focus handling and template slots for a feature
  that exists only while nothing else is happening. The post-render
  overlay keeps the blast radius to one module.
- **Moving real UI state** (actually reordering playlist rows for the
  "tidy" gag): violates the read-only contract and would sync.
- **Reversing her changes on exit** ("drag everything back"): too slow;
  the dissolve replaces it.
- **Behaviour trees / GOAP / async scripts**: hold mid-plan state that
  must be unwound; "interruptible at any instant" forbids it.
- **Full needs simulation** (Sims/Tamagotchi): charmless and guilt-driven;
  Osaka's humour is scripted absurdity.
- **Fixed frame rate**: wastes CPU in held poses and ties pacing to
  redraw speed (the Mail Order Life speed-run bug).
- **Syncing her between friends**: turns a toy into a protocol concern;
  divergent Osakas are funnier.

## Sources

- Mail Order Life: [Internet Archive](https://archive.org/details/osakasimulator),
  [Azumanga wiki](https://azudaioh.miraheze.org/wiki/Ayumu_Kasuga's_Mail_Order_Life)
- Canon: [Wikipedia episode list](https://en.wikipedia.org/wiki/List_of_Azumanga_Daioh_episodes),
  [Miraheze: Ayumu Kasuga](https://azudaioh.miraheze.org/wiki/Ayumu_Kasuga),
  [Hello Everynyan](https://azudaioh.miraheze.org/wiki/Hello_Everynyan_(OH_MY_GAH)),
  [Sata Andagi](https://azudaioh.miraheze.org/wiki/Sata_Andagi),
  [Yotsuba&! vol. 16 cameo](https://www.siliconera.com/new-yotsuba-manga-features-azumanga-daioh-cameo/)
- Prior art: [Johnny Castaway](https://en.wikipedia.org/wiki/Johnny_Castaway),
  [Little Computer People](https://en.wikipedia.org/wiki/Little_Computer_People),
  [eSheep animation graph](https://github.com/Adrianotiger/desktopPet/wiki/XML%5CNext),
  [Shimeji](https://github.com/DalekCraft2/Shimeji-Desktop),
  [xpenguins](http://xpenguins.seul.org/),
  [The Sims AI](https://gmtk.substack.com/p/the-genius-ai-behind-the-sims)
