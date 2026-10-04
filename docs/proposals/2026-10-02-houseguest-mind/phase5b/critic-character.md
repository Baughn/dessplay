# Phase 5b design critique: character and fun

The question for this review: does watching her day look like she has a life ("she's at school", "she's asleep"), or like the feature broke? And is it fun to watch? I checked every claim against the code at HEAD. Paths are under `dessplay/src/ui/houseguest/` unless they say otherwise. `[ART]` marks an idea that needs new art; nothing else here does.

**Line lengths:** every line the design quotes is 24 characters or fewer. I counted them all. The longest are "Rudolph's nose... why?" and "...wish I were a bird.", at 22. Every line I suggest below also fits.

## Blockers

None for this lens. The items below will show up as visible bugs or as long stretches with nothing happening.

## Majors

**F1 (MAJOR, D4 "The night's sleep", "Tucked in"). A night without a bed gets none of the night's content, and the sofa case can't be built as written.**
- Only the bed offers `Use::Sleep` (room.rs:148). The sofa offers `Use::Lounge` and `Use::Nap` (room.rs:122).
- `tucked_in(seat)` says "with a bed (else a sofa)" and builds `Act::Use { Sleep, Play { own: Sleep, .. } }`.
  - On a sofa, that is a use the piece doesn't offer.
  - It would also show the "asleep in bed under the quilt" pose (sprite.rs:61).
- D4 says "Nothing else lengthens". But a nap lasts 30–60 s (osaka.rs:918) and `Idle(LieBack)` lasts 15–40 s (osaka.rs:989). So a night on the sofa or the floor is roughly 100–300 short acts in a row, each a fresh decision.
- The stir, the sleep-talk and the Dream all live on the night sleep's script. A night without a bed gets none of them.
- This is the night viewers are most likely to see:
  - every new record spends its first night without a bed (bedtime comes 65 real minutes after the 16:00 start);
  - the stage room never gets a bed.
- LieBack swings a foot (sprite.rs:41-43), so she looks awake.
- **Amend:** "The night's sleep is one act, whatever she sleeps on. Each version lasts until the wake time and carries the same night script: stir, sleep-talk, the Dream splice, waking. `tucked_in` takes the surface and picks the pose."
  - bed: `Use::Sleep`;
  - sofa: `Use::Nap`, posed `Nap`;
  - makeshift heap: its own sleep method;
  - floor: `LieBack(0)`, held still.

**F2 (MAJOR, D3 transitions, "Away → Arriving … says 'I'm home!'"). The homecoming line collides with the door line.**
- When a door finishes, "I'm home!" comes only from `home_from_work`, which needs `at_work` (osaka.rs:1795-1807).
- Every other door rolls the DOOR pool one time in three (mind.rs:647-659), and that line replaces whatever she was saying.
- If `begin_visit` says "I'm home!", she says it from behind the opening door. She may then step out saying "Huh? How'd I get here?" over it.
- The dash-in's door can roll "Where was I?" just before "Forgot my lunch!".
- A parcel delivered on the return's first `furnish` says `PARCEL` (mod.rs:1692) over both. D3 counts on "deliveries already land on her return".
- **Amend:**
  - "She says the homecoming line as she steps out: at the door's end, or at the first decision after an edge walk-in. It goes through `home_from(why)`, which generalises `home_from_work`; leeks stay for Work only."
  - "Routine doors never draw the DOOR pool: the departure, the return, the dash-in in and out, and the errand at school."
  - "A parcel's line waits `speech_ms(HOME)` after 'I'm home!'."

**F3 (MAJOR, D6 "Starter rares"). Three of the four rares repeat lines she already says often.**
- MUSINGS (mind.rs:662-676) already holds:
  - "I wish I were a bird."
  - "Melon bread..."
  - "Escalator? Elevator?"
  - "Oh my gah."
  - "Chiyo-chan's dad..."
- That pool fires every time she muses (`n: 1, d: 1`), and one spaceout in three is a musing (mind.rs:216).
- So the Dream ("Oh my gah!", "...wish I were a bird."), NoMelon and Escalator are lines a viewer has heard many times a day. A changed punctuation mark is just enough to slip past the "no line in two pools" lint (mind.rs:1100-1105). A rare that sounds like a common musing doesn't land.
- Tanabata's "My wish: be a bird." would be the fourth bird line.
- **Amend:**
  - "A rare retires the musing it grows from. Drop 'Escalator? Elevator?', 'Oh my gah.' and 'Chiyo-chan's dad...' from MUSINGS. Keep 'Melon bread...' only if NoMelon changes (F4)."
  - "The Dream's lines are 'Hello everynyan...' / 'Fine sankyu...' / 'Oh my gah!' (HG #74)."
  - "Tanabata says 'Wrote my wish. Secret!'."
  - Make the lint compare lines with punctuation stripped.

**F4 (MAJOR, D6 NoMelon). The rare contradicts the pose she just played.**
- `Pose::Eat` is "Eating a melon bread" (sprite.rs:70-71), and the Snack script plays it.
- As an after-splice on a snack, she eats a melon bread and then says "No melon bread...".
- **Amend:** keep it an after-splice and make it the last one: fridge open, `Face::Droop`, "That was the last one." (22). HG #45's yakisoba bread waits for art `[ART]`.

**F5 (MAJOR, D3 "Why B" and D6 "The visit gate"). With option B, a resident's whole weekend is still one visit.**
- Resident is on by default (design.md:1854).
- A resident's visit runs from her return to the next departure:
  - on a school day, 19.5 game hours (195 real minutes);
  - over the weekend, Friday 12:45 to Monday 08:15: 67.5 game hours, about 11 real hours.
- So the per-visit mechanisms D3 says stay meaningful still stretch across a weekend:
  - `LINE_BUDGET` allows 8 short reaction lines a visit (mind.rs:735). After the first hour she stops saying them for the rest of the weekend.
  - `worked` (osaka.rs:3021, 3075) allows one shift per visit, so one a weekend. D2 opens the job every weekend day.
  - `Mood::of(seed)` (brain.rs:131) gives her one mood all weekend.
  - Rares get one roll per visit. A game week is 28 real hours with 5 resident visits, so 5 × 0.15 ≈ 0.75 rare offers per 28 real hours.
    - Once a rare has been seen, it practically never comes back.
    - A visitor rolls at every idle arrival, so the default mode, which people watch most, gets the fewest rares.
- **Amend:** "Waking (the end of the night act) starts a new day. Without `visits++`, it refreshes the line budget, `worked`, the mood and the rare draw, salted with the game day. The mood's greeting merges into the wake line, e.g. 'Mornin'... lazy day.'" A mood each morning is good variety in its own right.

**F6 (MAJOR, D7 "Getting them"). The clock arrives a day too late to explain anything.**
- The first TV arrives in the same visit it is ordered, because it sets `bought_on = visits − 1` (mod.rs:1680-1682). D7 has the clock arrive "on her next visit".
- For a resident, the next visit is the return from school. So after an upgrade, her first bedtime, first morning and first departure all happen with no clock on the wall.
- **Amend:** "Order the clock with `bought_on = visits − 1`, so it arrives in the visit where the clock first starts running."
- Give it an unpacking line that explains the 6× speed in character: "This clock runs fast..." (23).

**F7 (MAJOR, D5 starter set; Tests "at most one entry a day"). The test contradicts the table.**
- Several dates overlap:
  - Feb 3 falls inside exam season;
  - Apr 8 falls inside hay-fever season;
  - Dec 24–25 falls inside the December musings.
- Exam chopsticks are owed "once a visit", but D5 says items are "owed once a day".
- On Feb 3, which owed item plays first? Does Setsubun wait behind the chopsticks?
- **Amend:**
  - "Calendar entries come in two kinds. *Owed items*: at most one a day, and the most specific date window wins. *Tints*: boosts and line pools, which stack."
  - "Exam chopsticks are a tint: a boosted splice row, not owed."

## Minors

**F8 (D5, Apr 8).**
- "Nice to meet you!" can't be told apart from the Ordinary mood greeting "Nice to meet you." (brain.rs:185).
- Worse, a long-time user may think her record was reset.
- **Amend:** "It's my debut day!" (18) or "They call me Osaka." (19).

**F9 (D5, Jan 1–3).** "Ooh... first sunrise." (hatsuhinode) is only true on Jan 1. Limit the owed sunrise watch to Jan 1; Jan 2–3 keep the greeting.

**F10 (D5, New Year's Eve and the 09:00 day).**
- With `biblical_date`, midnight at a New Year's Eve watch party is still "Dec 31". At the stroke of the year she muses about Rudolph.
- **Amend:** add a Dec 31 owed item, "Year's almost over..." (20). It reads right both at midnight and in the hours after.

**F11 (D5, December).**
- The "seasonal musings" are a single line. With the 10-minute cooldown she says it every 10 minutes all month.
- Make it a pool of at least three: "Rudolph's nose... why?", "Santa's sleigh... flies?" (24), "Kotatsu weather..." (17).

**F12 (D4, sleep-talk).** Two lines said "every few minutes", each with a 10-minute cooldown, alternate about 8 times each over 85 minutes.
- **Amend:** use a pool of about 8 lines, one every 6–10 real minutes:
  - "...forty-two..." (HG #66's test score);
  - "...sata andagi...";
  - "...eye bubbles..." (#48);
  - "...Team Sea Slug..." (HG:303);
  - "Mm... melon bread...";
  - and a few others.
- Say "...five more minutes" only in the last game hour before she wakes.
- Count the Dream's "after 30 min asleep" from her first sleep of the night, so a groggy errand doesn't reset it.

**F13 (D7, LookOut).**
- Three lines on a 10-minute script cooldown (mind.rs:737), boosted at dusk and night, will repeat.
- **Amend:** 2–3 lines per sky phase:
  - night: "Stars!", "The moon's out." (16);
  - dawn: "Mornin', sun.";
  - day: "Sunny!", "Good laundry day!" (17);
  - dusk: "Pretty...", "Sky's all orange..." (19);
  - evening: "Gettin' dark...".

**F14 (D7, the clock face).**
- A quarter-hour is 15 game minutes, or 2.5 real minutes, so the face changes 24 times a real hour, not "2".
- The hands visibly jumping helps people read the clock. Fix the number, and the performance and image-count estimates built on it.

**F15 (D7, art).**
- The style README measures the outline at lightness 0.31 (OKLCH) against the terminal background #1e2127 at about 0.25, and says it disappears at 1×.
- A night sky in the window, or a dark clock face, will read as "nothing there".
- Ask the model sheet for a light window frame, a cream clock face, visible stars and moon, and a 1× crop on the terminal background.
- This comes from the README's numbers; it's an art note, not a confirmed bug.

**F16 (D3/D4, departure).** The design never mentions Toast #46 ([C, school days 07:00–08:30], HG:483-484).
- **Amend:**
  - "Toast waits for art `[ART]`. Until then, if the departure cuts into breakfast, she hurries out: `Face::Surprised`, 'Late, late, late!' (17), walking faster."
  - Walking faster needs a second `WALK_MS`; today it is a single constant (osaka.rs:259).
- D3 allows "door or edge". Pick **the door** for the departure, the dash-in and the return, so viewers learn that the door means school.

**F17 (D3, Away and her return). The cat can change as she walks in.**
- `cat_home` reads `visit_seed(visits − 1)` (mod.rs:2245), and the return's `begin_visit` increments `visits`.
- So a cat asleep in the empty home has an even chance of vanishing just as she says "I'm home!".
- **Amend:** "Away paints the cat of the *next* visit. It changes as the door shuts behind her and stays the same through her return."

**F18 (D3a, a dash-in with no fridge).**
- She comes in, spaces out for 2.5 s saying "Forgot my lunch!", and leaves with nothing. That looks like a glitch.
- **Amend:** she says "Forgot somethin'..." then "...what was it?". The DOOR pool's "I forgot what I forgot." is the precedent.
- With a fridge, have her hold `Eat(0)` (bread held up) for a beat as she turns to go: lunch, grabbed.

**F19 (D4, waking).**
- "She sits up" has no pose. `Sit` is hugging her knees on the floor (sprite.rs:39-40).
- **Amend:** "Waking is `Stretch`, standing beside the bed." Otherwise a sitting-up pose `[ART]`.
- I couldn't confirm that the `Sleep` pose can turn over inside the bed for the stir. Check the bed's `sit` spec before promising it.

**F20 (D4, sleep-talk wording).** "at most `LINE_BUDGET`-free" is garbled. Say "not counted against the line budget; each line has its own cooldown."

**F21 (Leaving at night).** When a visitor's input ends a night visit, she gives the usual "startled face, a goodbye smile" (design.md:1384). Woken from sleep, a `Face::Blink` goodbye would read better.

## Making the empty home readable (D3 "Away")

In the first seconds of a visitor's Away, the furniture just appears with nobody there. Pieces appear with the state and nothing fades in (mod.rs:771-787).
- The design's only cue is the lamp switched off.
  - The lamp glows by default (art.rs:880, against 946).
  - The TV is already off, because `tv` is `None` when no script sets it (mod.rs:1082).
  - Lamp-off looks the same as when she's asleep, and it's easy to miss at noon.
- The clock and the window only help homes that own them. A home that is just a TV, sitting alone on screen for 45 minutes, looks like a glitch.
- **Proposed, with existing art:** "In Away, a closed pink door stands where she left (`DoorFrame::Closed`, art.rs:811-834). The dash-in and the return open it."
  - It follows the rule that her door stays inside her box (design.md:1375-1382). It belongs to her and rains out with the furniture.
  - One frame then says "she went out".
- A note on screen ("At school! ~O") would be a new kind of overlay and need its own design call. The door is the cheaper cue.

## Cheap touches missing (no new art)

- **Meal lines by time of day**, drawn on the Snack script's first key (a `Say::Drawn` slot): "Breakfast!", "Snack time!", "Dinner! ...melon bread." (22). She eats melon bread at every meal, which is in character.
- **"Night-night..."** in place of the `Dots` on the night sleep's `LAMP_ON_MS` key (script.rs:966-975). Lamp off follows.
- **Make weekends obvious when she wakes:** "No school today!" (16) on weekend and vacation mornings, and "Summer vacation!" on Jul 20.
- **Midnight snack [U]**, at most once a night: reuse the groggy errand walk (`Face::Blink`), go to the fridge, play `Eat`, back to bed.
- **Pools for the routine lines.**
  - "I'm off!" with "Gonna be late!" and "Ittekimasu!".
  - "I'm home!" with "Tadaima~!" and "School was long...".
  - These play every school day, and "I'm home!" also plays on work returns (osaka.rs:888).
- **Errands while away or asleep:**
  - "Back to school!" after the poke during school.
  - A sleepy "Mm... someone said..." (20) instead of `POKE` when she's asleep.
- **Checking the clock:** a low-weight clock glance in the afternoon ("Is it dinner yet?") gives the clock a use beyond the two routine moments. Q3 asked for glances "at routine changes".
- **CHANGELOG** wording that explains why she's gone: "Added: Osaka keeps her own day: school mornings, homework, bedtime. Her day runs about six times faster than yours."

## Left for the builder to guess

1. The game-minute boundaries of the five sky phases (night, dawn, day, dusk, evening), and whether dawn comes before a 09:00 weekend wake.
2. Which unseen rare becomes `Rares.new`.
   - Is the choice limited to rares that can happen in this visit? In practice Scary only fits Friday or Saturday 22:00–23:30, about 15 real minutes, so a pity-forced draw could keep landing on it.
   - Is `open` rolled per rare or per tier?
3. Whether the Dream follows HG #74, "guaranteed once she has slept long enough", the first time.
4. How a calendar greeting fits with the other greetings:
   - the mood greeting: does it replace it or follow it?
   - "I'm home!": it should follow after `speech_ms`.
   - a tucked-in visit: it should wait for the wake line.
   - a resident already on screen when the real day changes: there's no arrival, so play it as an owed spaceout line.
5. When a dash-in falls within the away period. Keep it out of the first and last ~10 game minutes, so it doesn't look like a botched departure or return.
6. How a groggy errand puts her back to bed: does the night act resume or restart (its timers, the lamp beat)?
7. Whether her `!`/`?` look at chat is suppressed while she's tucked in, before her first decision. The greeting must not fire before she wakes: `decide` greets whenever `!greeted` (osaka.rs:2971-2974).
