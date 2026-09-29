# Proposal: The Houseguest (idle Osaka)

Status: **ACCEPTED; phases 1–2 and line art implemented 2026-09-28**
(design.md, Houseguest). Brainstormed by five parallel research passes
(Osaka canon, prior art, the TUI as terrain, progression/AI, terminal
animation craft) and merged here; revised after phase 1 and the switch to
line art. Sections describe what is built and mark what is planned
(*planned*). See [Status](#status) for the phase overview and
[Open questions](#open-questions) for what still needs deciding.

## Summary

When the client is fully idle, Ayumu "Osaka" Kasuga (*Azumanga Daioh*)
wanders into the terminal and makes herself at home among the panes:
she walks the pane borders, climbs between panes, peers over edges and
spaces out. *Planned:* she tidies the chat log by dragging lines out, and
gets a sofa, a desk for homework she never finishes, a bed, and a kotatsu
in winter. In Ghostty (kitty graphics) she is **anime-styled line art**; elsewhere an
ASCII sprite. She is a **pure visual overlay**: she reads the rendered
frame and paints over it, and never touches app state. Local input ends
the visit with a ~3.75 s goodbye: startled, a wave, then she bursts into
letters that rain away through the spoiler scramble, back to the real UI.

The direct ancestor is the 2004 fan-made screensaver *Ayumu Kasuga's Mail
Order Life* (hirahira.net), where Osaka potters around her room, goes out
to a part-time job, and buys furniture from Chiyo-chichi's shopping
channel on her TV. We borrow that backbone (see
[Progression](#progression-the-mail-order-life)).

## Status

| Phase | Content | State |
|---|---|---|
| 1 | Seam, idle gate, terrain, walking/climbing/falling, goodbye dissolve, setting, perf test | **done** |
| 1b | Line art: SVG rig, kitty placement, redrawn lines, letter-burst goodbye | **done** |
| 2 | Tidying and mischief: text layer and newest-line protection, pulling lines, letter swap, sneeze scatter, scheduled self-reverting, the stage (**done**); bubbles and speech, the brain with needs (**done**) | done |
| 3 | The room: eight pieces of furniture in pane-rooms, composited with her; the ledger; deliveries and Chiyo-chichi's shopping channel; her part-time job; ways out of pits (clambering, stepping out, the door); hunger and the fridge | **done** (routine and calendar moved to 4) |
| 3b | Resident Osaka: stays through playback and local input, rains out of the focused pane and leaves it by her door, a tenth as likely into the chat, input undoes her chat mischief; the Houseguest settings tab | **done** 2026-09-30 |
| 4 | Colour: calendar, screen reading, dreams, cameos, rarity and pity, credits | planned |
| 5 | The text factory: text hauled into an industrial space and compacted into materials | planned, last |

## The seam (architecture contract)

The guest (`ui::houseguest::Guest`) is owned by the shell loop next to
`Renderer`. It is not a tui-realm component and has no `Msg` variants.
Every frame goes through the shell's `draw` helper:

```rust
draw_frame(adapter.raw_mut(), |frame| {
    ui.draw_with_renderer(frame, renderer);
    let view = ui.idle_view(renderer.image_regions());
    guest.paint(frame.buffer_mut(), &view, now);
});
```

- **Reads**: the finished frame buffer (symbols and styles) and an
  `IdleView`, built after the draw because the draw measures the pane
  rectangles. **Writes**: only that frame's buffer. Nothing flows back
  into `Ui`.
- **Ticks**: the shell's timeout is `min(ui.next_tick_hint(),
  guest.next_tick(now))`; timeout ticks call `guest.advance(now)`, and a
  redraw happens only when something reports a visible change. No fixed
  frame rate.
- **Input**: every `UiInput` still goes to `Ui` unchanged. Key, mouse,
  and paste events additionally call `guest.activity(now)`.
- **Graphics**: at startup the shell hands the guest the image picker;
  with the kitty protocol she is line art, otherwise ASCII.
- **Removal**: deleting the module and the shell's calls removes the
  feature entirely.

### `IdleView`

| Field | Source |
|---|---|
| `delay` | the Houseguest setting (`None` when off) |
| `busy` | Playing (intent Playing with a now-playing file), Overlay (modal, layout tools, hashing or Nyaa-import overlay), or Selection (held chat selection) |
| `chat_mark` | synced chat count and newest stamp, IRC line count — compared for change only |
| `chat` | the chat pane rectangle (she turns toward it) |
| `protected` | the chat input line plus its frame, the Player Status block, the keybinding bar, inline images plus one cell of margin |
| `truecolor` | the detected colour depth |

## Idle and activity

**A visit may begin** when all hold for the configured delay (default
1 minute): not busy, no local input, no new chat or IRC line, terminal
at least 60×18.

**Activity** (ends a visit with the goodbye): any local key, mouse, or
paste event; becoming busy. Switching the setting off removes her
without a goodbye.

**Remote chat does not end a visit.** A new chat / IRC / `/me` line from
someone else cuts her current act (any instant is a safe cut) and she
**stops and looks**: turns toward the chat pane with a `!`, then a `?`,
and keeps watching until chat has been quiet for a minute. *Planned
(phase 2, when she starts touching chat):* she walks over and pokes the
line (its letters jiggle for a frame). *Built:* the newest message's
painted text is protected, so she never touches it.

**Not activity**: snapshot churn (health metrics, sync age, marquee,
download progress), subtitle and system lines, layout reload, focus
changes.

**Resize** is one rule with two halves: during a visit the next frame
re-reads the terrain and re-anchors her (she falls if her floor went
away, or reappears dazed on the nearest floor); during the goodbye it
ends the goodbye at once, since the frozen geometry is gone.

## Terrain and what she may cover

Terrain is **read from the rendered cells** every frame she is on screen
(~150 µs for 200×60 in release), not from pane rectangles:

- A run of horizontal box-drawing glyphs is a **floor** wherever her
  5×4 box fits above it; a floor's true end is a **drop-off** to the
  floor below. Pane titles interrupt floors, leaving gaps she can drop
  through.
- Vertical borders are **poles** linking floors she can climb between.
- Protected rectangles and image cells are **solid**: her body never
  enters them, though she may stand on a line inside one (the status
  separator).
- Any layout works, including borderless panes; chat day separators are
  floors that scroll away and make her fall.

**What she may cover depends on how she's drawn:**

- **Line art** (kitty unicode placeholders *replace* the cells they
  cover): her box covers only **blank cells and solid box-drawing lines,
  which her image redraws** in the cell's colour at the terminal's line
  geometry. Text is never hidden behind her, and neither is half of a
  wide glyph (a wide glyph's trailing cell is never "blank" to her).
  Standing, the image grows one row to include the floor, so her feet
  rest *on* the line.
- **ASCII**: her glyphs overdraw anything outside the protected set;
  sprite spaces are transparent, her head is solid.

**Generalising the rule for later phases** (*planned*): her image may
cover only what it can redraw. Blank cells, lines, and **her own
furnishings** (we render those, so they can be composited into her
image) qualify; text does not. Text she carries or pushes stays in text
cells beside her box, her hands drawn at the box edge.

**Depth order**: where she and a prop overlap, one image composites
both, in whichever order the scene needs — usually she stands in front,
but some scenes put her *behind* or *inside* a prop: asleep in bed under
the covers, in the shower, legs under the kotatsu.

## The room model (*planned*, phase 3)

Everything she owns is stored **pane-relative**, never as absolute cells:

```
Anchor { surface: Pane(id) | Screen, edge: Top|Bottom|Left|Right|Interior,
         frac: 0..1 along the edge, offset: (dx, dy) }
Prop   { kind, anchor, footprint }
```

Screen positions are recomputed each frame, so resizes and layout
changes carry her room along. A prop whose pane vanished or became too
small goes "into the closet" (hidden, still owned). Props occupy blank
cells only, like her.

**Displaced glyphs** (phase 2: holes she dug, letters she carries, lines
she dragged) record what they expect underneath:

```
Displaced { source: (pane, row, col), glyph, style, expected: (symbol, style),
            pos, state: Held | Placed | Falling | Returning }
```

**Validation rule**: each frame, a displaced glyph whose source cell no
longer shows `expected` is dropped — the hole closes, the real content
shows through, and she turns to look at it with a `?`. She can never
paint a stale hole over changed text. Cap: ≤ 60 displaced glyphs per
visit.

**Wide glyphs are atomic 2-cell bricks** everywhere: harvest, carry,
drop, and dissolve move or settle both halves together (already true of
the dissolve).

## The brain

**As built (phase 1):** a small state machine of wall-clock-timed acts —
stand (blinking), space out (`...`), walk to a random spot, travel along
a link (climb a pole, or peer over a drop-off and hop), peer over an
edge, fall, land dazed, look at chat. Choices are weighted random from a
per-visit seeded generator; every act is finite, so any instant is a
safe cut.

**As built (phase 2): needs and offers** (`brain.rs`). Each decision
gathers what's on offer here — stand, space out (a third of the time
musing aloud), sneeze, walk on this floor, travel along a link, each
activity, a pull if a line is in reach, a swap if a word is and no
mischief is owed — scores each as `base × fit × cooldown`, and picks at
random, weighted by score, among the **top four**. `fit` is 0.5 for an
offer that answers no need, else `0.1 + need²`: a need weighs little
until it's pressing, and the floor keeps every offer possible.
`cooldown` is 0.4 per repeat among her last three choices. Four needs,
each with a driver and something that answers it today:

| Need | Starts | Rises (0→1) | Answered by |
|---|---|---|---|
| sleepy | 0 | over 15 min | a doze on her back (−0.15), sitting (−0.1) |
| restless | 0.7 | over 90 s | walking, travelling (−0.4); jacks, toe touches, stretching (−0.5) |
| tidy | 0.5 | over 60 s while a line is on offer | a pull (−0.6) |
| mischief | 0.2 | over 4 min | a swap (−0.8) |

Needs move on at each decision by the time since the last one, and are
answered when an act is chosen (acts are finite, so this is where they
commit). A doze only takes the edge off, so over a visit she gets
sleepier and dozes more (measured over eight twenty-minute visits in the stage
room, after phase 3 gave her more ways about: ~4% of the first ten
minutes, ~8% of the next ten). Restless starting high replaces phase 1's "busier in the
first five minutes". A chat conversation and a job on another floor
still come first. `hungry` and `social` wait for something to answer
them (food words, cameos). The stage shows the needs, and keys 1–4 make
one pressing.

*Planned (phase 3 on):* **utility-scored scenes over interruptible
keyframe lists** — The Sims' "smart objects" plus Johnny Castaway's
scene pool.

- **Needs** (0..1, slow, never punitive): add `hungry` and `social`
  (drives cameos) as their affordances arrive. They weight choices; she
  never sickens, starves, or guilt-trips (the Tamagotchi lesson).
- **Advertising**: panes and props advertise affordances ("chat: messy",
  "users: people to wave at", "sofa: rest", "desk: homework"). Adding a
  prop adds behaviour without touching the brain.
- **Selection**: `score = base × need_fit × time_fit × calendar_fit ×
  room_fit × cooldown × novelty`; weighted-random among the **top few**,
  never argmax (robotic) and never flat random (slot machine).
- **Scenes** are finite `Vec<Keyframe>`; each keyframe is a complete
  pose + position + glyph overrides + optional bubble, with a duration.
  Rendering stays a pure function of state and time.
- **Room mutations commit at scene start.** "Sofa delivery" writes the
  sofa into the room before the 40 s carrying animation begins; an
  interrupt loses nothing.
- **Locomotion** keeps the eSheep-style successor graph phase 1 already
  has: finished animation → weighted successor; hitting a border → a
  border transition (climb, turn, peer over); floor vanished → gravity.
- **Determinism**: visit RNG seeded from `(master_seed, visit_index)`
  once the ledger exists (phase 1 seeds per process); clock and calendar
  derived from the injected `now`. All pacing is wall-clock (the Mail
  Order Life famously runs absurdly fast on modern PCs because it was
  frame-timed).

## Progression: the Mail Order Life (*planned*, phase 3)

Borrowed from the original screensaver:

1. Sometimes she **leaves for her part-time job** (walks off an edge);
   the room stands empty-but-furnished, and she returns later with a
   shopping bag.
2. Occasionally she sits before her TV (a piece of furniture) and
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

## Routine and calendar (*planned*, phases 3–4)

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
| Feb 3 | Setsubun: beans thrown off the bottom of the screen ("oni wa soto") | — |
| Mar–Apr | hay-fever sneezes common, tissue pile, eyedrops lying down; cherry petals, yawning contest | Ep 26, Ep 19 |
| **Apr 8** | her "debut day" (anime premiere, 2002): transfer-student intro variant | — |
| Jul 7 | Tanabata: bamboo on a pane border, wish slip | — |
| ~Jul 20–Aug 31 | summer vacation: no school; beach float, swim ring drifting away; last week: homework panic | Ep 4, 14 |
| Sep–Oct | sports festival: shoe-kick forecast, losing to a child, "Team Sea Slug!" flag; typhoon joy | Ep 3, 6, manga |
| **Sep 30** | final episode anniversary (2002): graduation bow | Ep 26 |
| Oct 31 | Halloween: sheet ghost with two dots | — |
| Oct–Nov | culture festival (configurable weekend): café stand in the playlist, penguin feeding | Ep 16 |
| Dec | kotatsu season; Rudolph critique; a winter uniform variant | Ep 17, 24–25 |
| Dec 24–25 | a tree in a corner; a present under it next morning | — |

Osaka has **no canon birthday**; we do not invent one.

## Rarity and pacing (*planned*, phase 4)

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
**Cal** calendar. ★ = signature. **Built** marks what phase 1 has.
Canon references from the Miraheze episode pages and Wikipedia's
episode list.

Glyph sketches below (`(._.)`, `@_@`, `╲_╱`) are the ASCII-era
shorthand. In line art they become poses, props and effects in her
image; anything that changes **text** (swapping letters, eating words,
pulling lines) happens in text cells outside her box. Nothing she draws
covers the keybar or the Player Status block; her lowest floor is the
**status separator** line above Player Status.

### Arrival

1. ★ **Edge peek** — one eye at a pane edge, blinks, retreats, then she
   walks in and bows stiffly: "Nice to meet you." [C] *Built: she walks
   in from a screen edge a floor reaches and says hello (no bow yet).*
2. **Falls from the sky** — drops in from the top, lands dazed on a
   floor, "...I'm OK." [U] *Built.*
3. **Trapdoor** — climbs up through a gap in a floor line. [R]
4. **Nameplate** — `Kasuga` appears on a border, a scribble overwrites it
   with `OSAKA`; she thinks the nickname is too simple (Ep 1). [U]
5. **Already home** — when there's nowhere to walk in from or drop onto,
   she's simply there, blinking. *Built.*

### Terrain

6. ★ **Ledge walking** — along pane borders; at a real edge she stops,
   peers over, wobbles. [C] *Built.*
7. **Climbing the divider** — shins up a vertical border beside the
   pole. [C] *Built.*
8. **Falling off** — hops off a ledge end, falls under gravity, lands
   dazed. [C] *Built.*
9. **Border sag** — standing mid-border, the span she stands on bows
   under her weight (redrawn in her image). [U]
10. **Status separator as skirting board** — her default floor; sits with
    legs dangling over the separator line, kicking. [C]
11. **"Escape the Earth" / "Laundry"** — back hip circle on a horizontal
    border, then hangs folded over it like a futon (adult Osaka as a PE
    teacher, *Yotsuba&!* vol. 16). [R]
12. **Jump rope** — two borders as rope-turners; trips half the time
    (Ep 12). [U]
13. **Hanging laundry** — strings a line between two corners in an empty
    pane, hangs socks. [U]

### Tidying and mischief

14. ★ **Tidying the chat** — drags playlist titles / old lines out of the
    chat pane with a tug rhythm (brace, heave), stacks them in an empty
    corner, dusts her hands. Heavier lines are slower. [C]
15. **Sweeping** — sweeps the last lines leftward into a heap; a few
    glyphs always slip back out of the dustpan (Ep 12). [U]
16. **Oops** — notices the gap she left and drags the line back, "sorry!".
    [U]
17. **Harvesting punctuation** — pockets every `.,!` in view; hands them
    back one by one, some in the wrong spots, fixes them on a second
    glance. [U]
18. **Letter swap** — swaps two adjacent letters (`teh`), giggles, waits,
    nobody noticed, swaps back. Always self-reverting. [U] *Built:* the
    last two pairs of a word ending beside her; the swap-back is
    scheduled when she swaps, and a chat arrival brings it forward.
19. **Magnet** — every `o` in chat slides toward her; she gets scared and
    shoos them home. [R]
20. **Shelf rearranging** — swaps two playlist rows, notices the current
    entry's highlight moved, panics, swaps back. [U]
21. **Getting distracted halfway** — carrying a letter home, a moth flits
    past; she follows it and leaves the letter in the Users pane until
    the next tidy. [U]

### Reading the screen

22. ★ **A message arrives** — she stops mid-whatever, turns, stares at
    the chat (`!`, then `?`); a conversation keeps her watching. [always,
    on remote chat] *Built.* Planned: walks over and pokes the new line.

Semantic reading is best-effort: concatenate a row's cells per pane,
skip wide-glyph continuation cells, match case-insensitively, accept
false negatives.

23. **Sounding out** — stands under a chat line and reads it word by word
    (reverse-video highlight walks along), mouth opening and closing. [C]
24. **Food words** (`ramen`, `onigiri`, `bread`, `curry`, `watermelon`…) —
    walks over and eats the word letter by letter, `nom`, restores it
    later with a burp. [U]
25. **America** in chat, a user name or a title — lands like a plane, tiny
    figures pop up beside the Users rows: "Hallo! Hallo!", "America-ya!"
    (Ep 20). [R, boosted by trigger]
26. **Cats** (`cat`, `neko`, `=^.^=`) — crouches and pets the word, calls
    it "Yamapikarya!" (Ep 21). [U]
27. **Azumanga** in the playlist or series list — gasps, "that's me?" [R]
28. **Blue Three** — a title containing `3`: "Blue Three... so where's
    Blue One?" (Ep 13, Bruce Lee). [R]
29. **Long words** — tries to pronounce any ≥ 10-letter word, garbles it.
    [U]

30. **Remarks on her surroundings** (*later*; the user's idea,
    2026-09-28) — now and then she comments on what's around her: the
    playlist, the chat log, the List, in Osaka's voice, written by Opus
    from what's on screen. Needs network protocol work (the model key
    lives server-side, so the client asks for a remark through the
    oracle), which is why it waits. Shape: a slow cadence (minutes, not
    seconds), only mid-visit, the request built from text already on
    screen, the reply shown as her speech bubble (so the ≤ 24-char bubble
    limit becomes wrapping), and a canned Osaka-ism when the answer
    doesn't arrive in time. The AI commentary engine (design.md, AI
    Commentary) is the nearest existing pattern. [U]

### The neighbours (Users pane)

30. **Waving** — waves at each name in turn; the brightest-styled row
    waves back (first letter lifts for a frame). Dim rows: she tiptoes
    past, `shh`. Styles compared relatively within the pane. [C]
31. **Counting** — counts the names on her fingers, loses count at 4,
    starts over. [C]
32. ★ **Contagious yawn** — a huge yawn; an `o` in a user name yawns,
    then the next… (Ep 19). [U]
33. **Explain this part to me** — at the desk, turns to each user name in
    turn with homework questions (Ep 22). [C at homework time]
34. **Hiccups that jump** — `hic!` bubbles, remedies fail, the `hic!`
    hops onto a user name and stays (Ep 2). [U]
35. **Kanji trivia to a silent friend** — "Seals are 'sea leopards'…" at
    the Users pane, which says nothing back (Ep 5). [U]

### The TV

The TV is **one of her furnishings** (the Player Status block is far too
wide and short to read as a screen, and it's protected anyway). It is
among the first things she owns, since the shopping channel plays on it.

36. **Watching TV** — sits cross-legged before it; static, or harvested
    title letters scrolling. [C]
37. **Channel surfing** — static, a colour-bar card, a tiny sunrise. [U]
38. **Whacking it** — taps it; the picture shakes. [U]
39. ★ **Chiyo-chichi's shopping channel** — the progression engine (see
    above). [U, paced by progression]
39a. **Rabbit ears** — static; she adjusts the antenna, it gets worse,
    she whacks the set, and the picture comes back upside down. [U]

### Food

40. ★ **Chopstick ritual** — the chopsticks split; a clean split
    sparkles, a bad one makes her droop: "Ya gotta hold 'em by the
    ends!" (Ep 25). [C; before homework, and on the hour]
41. ★ **Sata andagi** — every question gets "Sata andagi.", 5–7 times,
    happier each time; then she eats it (Ep 21). [U]
42. **Curry or hashed beef** — tastes two bowls: "...same." (Ep 19). [R,
    12:00–13:00]
43. **Chili croquette** — turns red, steam, lies flat: "Thought I'd
    die." (Ep 2). [R]
44. **Five breads** — dithers between five breads on a string while the
    rest of the "race" finishes (Ep 23). [R]
45. **Melon bread** — gets yakisoba bread instead: "Melon bread..."
    (Ep 12). [R]
46. **Toast morning** — toast in mouth, sprints off for school. [C, school
    days 07:00–08:30]
47. **Forgot lunch** — dashes in during school hours, grabs a bento, out.
    [U]

### Daydreams and overthinking

48. ★ **Eye floaters** — a mote drifts across a pane; her head follows
    it, she lunges and misses; it slides away whenever she looks at it
    ("trackin' my eye bubbles", Ep 2). [C]
49. ★ **Chasing the cursor** — stalks the blinking terminal cursor, which
    is always one cell ahead; falls asleep next to it (the Neko lesson).
    [U; only when a cursor is visible outside the protected cells]
50. **Spacing out** — stops, `...` for 8–20 s, carries on. [C] *Built.*
51. ★ **Flying pigtails** — a thought bubble with a pigtailed head; the
    pigtails spin off like rotors and fly around the panes; she panics
    and sticks them back (Ep 2, Ep 8). [U; always on Jan 1]
52. ★ **Panda debate** — draws a panda in an empty pane, three worsening
    attempts: "Black spots on white? Or white on black?" (Ep 17). [U]
53. **Scary story** — screen dims except her: "...I smelled a fart that
    wasn't mine." (Ep 17). [R, after 23:00]
54. **Escalator or elevator** — "The box one's the escalator. ...No?"
    (Ep 14). [R]
55. **Rooftop** — stands on the very top border, arms out: "Feels like ya
    could fly away..." Never falls from this one. (Ep 12) [R]
56. **Riddle queen** — a pun riddle in a speech bubble; she answers
    instantly (her one academic talent). [U]
57. **Left or right** — an arrow key hint in the keybar catches her eye;
    she mimes chopsticks to work out which hand, walks the wrong way
    (Ep 15). [U]
58. **Peering over the edge** — at a real ledge end she leans over and
    looks down. [C] *Built.*

### Physical comedy

59. ★ **Shoe-kick weather forecast** — kicks her shoe up; instead of
    landing it sticks to something moving (the marquee, a progress bar);
    she stares after it (Ep 6). [U, boosted when something animates]
60. ★ **Tiny sneeze** — "...chu"; the recoil knocks 2–4 nearby glyphs off
    their cells, they fall, she quietly puts them back (Ep 26). [U; C in
    Mar–Apr] *Built (always uncommon; the season comes with the
    calendar).*
61. **Loses a race to a child** — a pigtailed figure overtakes her along
    the status separator; enormous determination, no progress. [U]
62. **Float like a corpse** — an empty playlist becomes a pool; she drifts
    face-up (Ep 4). [R]
63. **Swim ring drifts away** — while she isn't looking; she returns
    later and looks around (Ep 14). [R, summer]
64. **Free tissues** — tiny vendors keep handing her packets; the pile
    grows until she gives them to the Users list (Ep 14). [U, spring]
65. **Rain dance** — one raindrop falls, a flash, her hair briefly
    frizzed (Castaway). [R]

### Home life (prop-gated)

66. ★ **Homework** — at the desk, writes, gets heavier-lidded, asleep on
    the paper, the desk lamp drooping lower as she does; the test paper
    shows **42** (Ep 13, 22). [C, 20:00–22:30]
67. ★ **Kotatsu** — only her head visible, for hours; mikan on top
    (Ep 24–25). [C, Dec–Feb evenings]
68. **Sofa nap** — curls up hugging the throw cushion, `z` drifts;
    sometimes the cushion rolls off onto the floor line. Falls asleep
    standing if she has no sofa. [C]
69. **Blank-line nap** — lies in the widest blank run in chat; a real
    repaint of that run is her alarm clock. [U]
70. **Lamp** — switches it off before bed. [C, bedtime]
71. **Window** — day sun / night moon from the wall clock, a cloud
    drifting across. [ambient]
72. **Reading** — pulls a playlist row out like a book spine, reads,
    slides it back. [C]
73. **Closet reveal** — cycles through past seasonal props. [R, owns ≥ 10
    items]

### Sleep and dreams

74. ★ **Chiyo-chichi dream** — asleep ≥ N minutes, a floating orange cat
    head drifts over the playlist: "Hello everynyan. How are you? Fine
    sankyu." Her bubble: "OH MY GAH". "I wish I were a bird." (Ep 25). [R,
    guaranteed once she has slept long enough]
75. **Office dream inversion** — asleep on the sofa, a bubble shows a tiny
    classroom where dream-Osaka is asleep at her desk dreaming of the sofa
    (Castaway). [R]
76. **Wavy borders** — pane borders go wavy while she sleeps; one pane
    briefly shows sky. [R, asleep > 2 h]

### Cameos

At most one per visit, long cooldowns, keyed off `(date, master_seed)`
so each friend's Osaka gets different visitors (comparing notes in chat
is the social payoff). Cameo characters are line art like her and follow
the same covering rules.

77. **Chiyo** — they do homework together; Chiyo tidies properly
    (everything snaps into perfect alignment). [U, homework hours]
78. **Tomo** — barges in, kicks the sofa across the room, yells, leaves;
    Osaka blinks for 30 s. [R]
79. **Yomi** — refuses snacks (diet) while Osaka eats. [R]
80. **Kagura** — jogs along the status separator, push-ups, jogs out. [U,
    mornings]
81. **Yukari-sensei** — a car screeches along the status separator;
    Osaka's hair stands up. [R, school days 08:00–08:30]
82. **Kimura** — appears at a pane edge and stares; she waves, he doesn't
    move. [R, once per season max]
83. **Sakaki** — reaches to pet a cat; it bites; she is quietly sad. [R]
84. **Kamineko stays** — the biting cat follows Sakaki in, stays behind,
    and sleeps on the sofa for the rest of the visit. [L, after ≥ 2 Sakaki
    cameos]
85. **The long stay** — whole cast around the kotatsu (winter) or at the
    beach (summer). [L, ≥ 8 cameos and visit > 4 h]

### Milestones and endings

86. **Leaves on her own** — after > 3 h (not night), waves and walks out;
    the furniture stays. [U]
87. **Anniversary photo** — a framed photo on a wall at 30 days, 100
    days, one year since her first visit. [L]
88. **Credits** — the full catalogue owned: credits scroll in chat, TV
    cracks, life goes on. [once]

### The text factory (phase 5)

89. **Haulage** — instead of only pushing text out of sight, she drags
    some into an industrial space at a pane's edge; the text's font
    changes on the way in (a deliberately low-res, DOS-styled face, drawn
    by us in her image layer), is compacted, and comes out as raw
    materials for furniture. [U, once a factory exists]

Cut on review: the frying-pan/knife wake-up (Ep 22 is canon, but a
figure walking at a sleeper with a knife reads badly out of context);
mosquito musings (no source found); gags that drew on the keybar or
Player Status (both protected).

## Art

Four visual layers, from the app outward:

1. **Text** — the app. Never hidden behind her image.
2. **Panes** — lines; they fit both worlds, and her image redraws any it
   covers.
3. **Furnishings** (*planned*) — **coloured, unfilled line art**, halfway
   between the text UI and her anime style; the bridge between the two.
4. **Osaka** (and cameos) — anime-styled line art.

### Osaka as line art (built)

- **Parts** in `ui/houseguest/art/osaka.svg`, drawn after the character
  sheet in `docs/ayumu.webp`: long straight dark-brown hair with ragged
  bangs (split so it hangs behind her body), very large round eyes with
  brown irises, the winter uniform (salmon top, white sailor collar with
  red trim, thin red cord with a yellow toggle, maroon pleated skirt,
  white knee socks, brown loafers), two-segment arms and legs. **Posed by
  a rig** in `art.rs`: lean, head tilt, bob, shift, a whole-figure turn
  and scale (for lying down), and shoulder/elbow and hip/knee angles.
  resvg renders the posed scene at the terminal's cell size into her
  5×4-cell box (one row taller when standing). Dark outlines, muted
  fills; at 1× she reads right, at full resolution she looks like a
  poseable doll.
- **Two views.** *Frontal* (her vacant stare at the viewer) for
  standing, looking, falling, dazed and waving; *profile* for walking,
  pulling, peering and climbing, so she walks with a real stride instead
  of crab-walking. The profile has its own head (the bob covering the
  back of her head, the fringe, one sleepy eye), torso (the sailor
  collar's square flap on her back, the neckerchief in front) and skirt;
  far limbs are drawn behind her body, near limbs in front.
- **Poses**: stand, blink, four walk frames, two climb frames (in
  profile, facing the pole: one hand up beside her head, the other on the
  pole at her waist, the high-hand knee lifted; the pole's position is
  carried on the climb link), fall, dazed, peer, pull (brace and heave,
  hands at the box edge on the line's row), look (surprised), side-on
  standing, sit (hugging her knees), lie on her back and on her stomach,
  jumping jacks, toe touches, stretch, gaze, and a two-frame wave.
  Expressions: vacant (her wide-eyed blank look), blink, surprised,
  smile, happy (open smile), curious (looking up), dizzy, in both views.
  Facing left mirrors the scene. Climbing hangs straight from her
  handholds.
- **Placement**: kitty unicode placeholders via ratatui-image, which
  transmits each image once and afterwards only places it. Frames are
  cached per (look, facing, covered lines, clip, cell size), up to 256.
- **Redrawn lines** use the terminal's box-drawing geometry: thickness
  ≈ cell height / 16, centred with integer division;
  `DESSPLAY_HOUSEGUEST_LINE=thickness[,offset]` overrides it. In Ghostty
  the position matches; the redrawn stretch looks slightly brighter than
  its neighbours (image pixels and text glyphs blend differently), which
  reads as a highlight and is accepted. Rounded corners are redrawn
  square.
- An ignored test renders a **character sheet** for review:
  `HOUSEGUEST_SHEET=/tmp/sheet.png cargo test model_sheet -- --ignored`.

### ASCII fallback (built)

Without kitty graphics she is a 5×4 ASCII sprite: `(._.)` head (always
solid), `/|V|\` body with the `V` ribbon in LightRed, skirt and legs with
transparent gaps. Faces swap in one row (`._.` vacant, `-_-` blink, `o_o`
surprised, `^_^` pleased, `@_@` dizzy). Her colours: foreground and
modifiers only, never background.

### Furnishings (*planned*, phase 3)

Coloured, unfilled line art, rendered through the same pipeline as her,
occupying blank cells only. Where she and a prop overlap, one image
composites both in the scene's depth order (in front of the sofa; under
the bed covers; behind the shower curtain; legs under the kotatsu), so
neither is cut out by the other's placeholders. The TV is furniture.

### Building from harvested letters (*planned*)

1. **Blueprint**: a dim outline of the prop.
2. **Harvest**: she lifts letters (keeping their original style — you can
   see she's carrying bob's message in bob's colour) from old chat lines,
   series rows; or borrows a run of `───` from a pane border, leaving a
   visible gap. Never protected cells. Carried text stays in text cells
   beside her box.
3. **Deliver**: one letter per 250 ms, bottom row first, falling into
   place.
4. **Cure**: the finished prop swaps from its letters to its line-art
   image.
5. **Conservation**: prop size tracks letters harvested; the screen
   visibly runs out of text as the room fills.

## Motion and scheduling

No fixed frame rate; every act carries its next change time, and these
are the resulting rates:

| Phase | Redraws |
|---|---|
| held poses (most of a visit) | 0.1–0.3/s (blinks every 4–9 s) |
| walking | 3 cells/s, one redraw per cell |
| climbing | 2 rows/s |
| falling | one redraw per row crossed, g ≈ 28 rows/s² |
| goodbye (3.75 s) | ~16/s |

Measured: a visit's CPU is below `/proc`'s 10 ms resolution over 3 s
(perf.rs, release).

*Planned:* dragging a line uses a 600 ms brace/heave cycle scaled by
length, off-row moves snake-follow her hand; dropped glyphs fall and
sandpile, no bounces; speech bubbles pick a placement around her head by
what they'd cover (blank only in line-art mode), ≤ 24 chars, shown
1.2 s + 60 ms/char. *Built:* bubbles pick the first all-blank spot of
seven around her head; speech up to 24 characters is timed as above
(greeting, "...I'm OK.", musings), over the act's own bubble.

## The goodbye (~3.75 s)

Reuses the spoiler scramble (`spoiler::rain_glyph`, a helper beside
`scramble` that never passes anything through, keeps the class of the
text a cell is becoming, and draws borders from a box class so they
re-knit) and the spoiler tease's discipline: frames derived from wall
time, never per-tick counters.

- At activity time `T0` the composite `C` freezes: every cell she
  painted, with the real cell beneath. Each draw renders the live real
  frame `R`; only frozen cells animate.
- **Read before paint**: everything that compares against `R` does so
  before any of her pixels or glyphs go on, so her own image never looks
  like "the UI changed here".
- **Fast lane**: a cell whose real content changed since `T0` settles
  immediately and stays settled; whatever the user is doing (typing, a
  modal the key opened, new chat) is never scrambled. In line art, if
  anything lands in her box the image is dropped at once.
- **Wide pairs settle together**, so a settled half is never knocked out
  again by its partner.

| t − T0 | Line art | ASCII |
|---|---|---|
| 0 | startled face (image) | startled face `(o_o)` |
| 375 ms | waving (two image frames, 150 ms each) | goodbye smile `(^_^)` |
| 675 ms | bursts into letters: every box cell becomes noise | rain begins where it reaches |
| → 3600 ms | rain ripples out from her column, 90 ms per column plus jitter, trails 3–6 rows | same |
| 3750 ms | overlay gone; tick hint back to lazy | same |

Colours: TrueColor head `Rgb(210,255,215)` bold, trail green then muted;
limited depth LightGreen then DIM. Background untouched. Further input
never restarts or extends it; a resize ends it at once.

## Settings

Built, under F3 → Houseguest, local only:

- **Visits**: After 1 (default), 2, 5, 10 or 30 idle minutes, or Off.
- **Resident** (default on): she stays through playback and input,
  keeping out of the focused pane (design.md, Houseguest).
- **Osaka moved out**: wipes her record, after asking.

*Planned with the ledger (phase 3):* Full / Visits only (no persisted
room); **Night stays** on/off (off = she leaves at bedtime instead of
sleeping over, for shared screens left on overnight); a reduced-motion
goodbye (a plain short scramble, or an instant cut). First visit ever: a
shy intro scene, plus one local system line in chat, "Someone seems to
have moved into your terminal. (F3 → Houseguest)"; until then
the CHANGELOG entry announces her.

## Persistence (*planned*, phase 3)

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

Built, in `dessplay/src/ui/houseguest/`:

```
mod.rs        Guest: activity, advance, next_tick, paint; visit lifecycle
idle.rs       IdleView (what she may know) and the gate
terrain.rs    floors, poles, drop-offs and open cells, read from the frame
osaka.rs      her acts, timing, re-anchoring
sprite.rs     ASCII sprite, poses, faces, facing
art.rs        the rig and resvg rendering; art/osaka.svg holds the parts
graphics.rs   kitty placement, redrawn lines, frame cache
cells.rs      panic-free writes that keep wide glyphs whole
layer.rs      the text layer: moved glyphs, holes, validation
scenes.rs     what text scenes are possible (pulls, swaps, loose glyphs)
              and the layer operations they queue
brain.rs      needs, offers, and the top-few choice
stage.rs      cue any scene on demand; the rooms the tests share
dissolve.rs   the goodbye, a pure function of (C, R, T0, t)
tests.rs      gate, lifecycle, property and snapshot tests
```

`Ui` gains `idle_view()` and `image_picker()`; `ChatPane` records its
input area; the renderer exposes `image_regions()`; the shell gains the
`draw` helper, the tick term and `activity()`; `dessplay-core::spoiler`
gains `rain_glyph`; `theme` gains `truecolor_rgb`. Planned modules:
`room.rs`, `calendar.rs`, `ledger.rs` (phase 3 brief: plan.md, Phase 37).

## Testing

Per docs/testing-strategy.md: seeded RNG, injected clock, no sleeps.

Built:

- **Idle gate and lifecycle**: arrival only after the delay; busy
  clients (playing, overlay, selection) get no visit; switching off
  removes her without a goodbye; a chat message makes her look, not
  leave; a tiny terminal gets no visit; a resize during the goodbye ends
  it.
- **Goodbye properties** (proptest over random frozen cells, real frames
  with CJK, sizes, origins, `T0`, line-art burst on/off): the frame at
  the end equals the real frame exactly; settled cells never un-settle;
  output is deterministic; no frame holds half a wide glyph; cells the
  real UI changed are never scrambled. Plus: line art bursts into
  letters that rain away; the line-art beats never paint over new
  content.
- **Long-visit property** over random layouts, resizes, chat arrivals,
  image (skip) cells and CJK text, in both ASCII and line-art modes:
  protected rectangles and image cells are never touched, no half wide
  glyphs, the overlay stays small, and in line-art mode every cell she
  changes was blank or a line her image redraws.
- **Terrain snapshots** of the default layout (80×24 and 100×30, ASCII
  and line-art modes) and a **scene snapshot** (seed 7 at 95 s).
- **Line art**: every pose renders inside the box with a transparent
  background; each frame is transmitted once, then only placed;
  standing adds the floor row; redrawn line geometry and colour.
- **Perf** (`tests/perf.rs`): visit CPU is below measurement resolution;
  terrain read time is bounded in release.
- **Stage** (`houseguest::stage`): every scene can be cued on demand and
  is placed where it works. `cargo run -p dessplay --example houseguest
  [seed]` shows the real default layout with an evening's chat; ←/→ pick
  a scene, Enter plays it, `m` a chat message arrives, `g` goodbye, `n`
  new seed, `[`/`]` slow motion down to ⅛× or fast forward to 4×. The same
  rooms back the tests, and a test cues every scene at 100×30 and 80×24
  in both modes and checks it visibly happens.
- **Mischief**: a swap undoes itself on schedule and at once on chat; a
  refused swap owes nothing; a sneeze's glyphs all go back.

Planned: room anchors surviving resize sequences; displaced-glyph
validation; pity bounds across simulated weeks; ledger round-trip,
version mismatch and unknown items.

## Open questions

None open. Noted for later: **image lifetime** — ratatui-image never
deletes images from the terminal; the frame cache bounds how many
distinct images a session makes and Ghostty evicts old ones past its
storage limit. If a cached pose ever renders blank after a long session,
delete images as they leave the cache.

**Going underneath ratatui-image** (later, one piece of work): it writes
each image row as a single run of kitty placeholders with inferred
columns, so every cell of her 5×4 box hides the text beneath it, and she
may only stand where the whole box is clear. That limits tidying most
(census, 2026-09-28: line-art screens with a chat pull 33/60 at 100×30,
17/60 at 80×24; ASCII, which has no box-clear rule, 42/60 and 31/60).
Writing the placeholder runs ourselves — only her opaque cells, with
explicit column diacritics — would let lines run under the empty parts
of her box (terrain then needs each pose's footprint). Once we own that
layer, true translucency (her drawing over the text itself) becomes
possible too, which would lift the rule altogether. Reaching head- and
foot-height rows is small beside that (+5/60, +2/60) and needs tiptoe and
crouch poses.

## Decisions

Settled with the user, 2026-09-28:

1. **On by default**, with the setting to turn it off.
2. **Idle threshold**: 5 minutes default, configurable.
3. **Remote chat**: she stops and looks, maybe pokes the message, but
   never changes it; only local input ends a visit.
4. **Persistence**: the room persists across visits.
5. **Name**: "Houseguest" in the UI.
6. **Cameos**: included (phase 4); variety is the point.
7. **Art direction** (after the ASCII sprite read as "a very
   large-headed alien"): Osaka is anime-styled line art through the
   kitty protocol (everyone runs Ghostty), at the terminal's real cell
   size, in a 5×4 box; ASCII remains the fallback. Furnishings are
   coloured, unfilled line art. The text factory comes last.
8. **Covering rule**: she only covers what her image can redraw; text
   stays visible. Pushing and pulling text keeps her hands at her box
   edge.
9. **Feet on the line**: the floor row joins her image; a small
   geometry mismatch was accepted and tuned by eye. The slightly brighter
   redrawn stretch is accepted as a highlight.
10. **The goodbye** takes ~3.75 s (2.5 s felt slightly too fast) and ends
    in a letter burst rather than a stream of per-frame images.
11. **The TV is furniture**: the Player Status block is far too wide and
    short to work as a screen.
12. **Depth order both ways**: props can be in front of her too (bed
    covers, shower, kotatsu), composited into one image.
13. **tmux is out of scope**: dessplay's core use is playing video, so it
    runs on a local terminal.

## Rejected alternatives

- **Integrating with the layout renderer / tui-realm components**: would
  need `Msg` variants, focus handling and template slots for a feature
  that exists only while nothing else is happening. The post-render
  overlay keeps the blast radius to one module.
- **Moving real UI state** (actually reordering playlist rows for the
  "tidy" gag): violates the read-only contract and would sync.
- **Reversing her changes on exit** ("drag everything back"): too slow;
  the dissolve replaces it.
- **ASCII as the main look**: at 5×4 cells she reads as an alien, not
  Osaka; a larger ASCII sprite would still be limited to characters.
- **Floating over text**: hand-rolled kitty placements (or vendoring
  ratatui-image) to draw over text with real transparency; possible
  later, but "only cover what she can redraw" costs nothing and fits her
  tidying character.
- **Rendering the text ourselves** under her (a DOS-styled font) to fake
  transparency; kept for the text factory instead, where the font change
  is the point.
- **A stream of per-frame images for the goodbye**: ratatui-image never
  deletes images, so each goodbye would leave dozens behind in the
  terminal.
- **A larger box**: more detail, but fewer floors on 80×24.
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
