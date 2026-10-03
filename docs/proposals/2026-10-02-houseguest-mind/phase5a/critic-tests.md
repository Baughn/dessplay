# Phase 5a design critique: tests, goldens and the commit plan

The plan has two blockers. Commit 1, as written, cannot leave the goldens unchanged. The "splices never change what they wrap" property is false as stated, so as a test it can never pass. Every claim below was checked in the code; paths are relative to `dessplay/src/ui/houseguest/`.

## Blockers

### B1. Commit 1 can't keep the goldens while D1 changes `first_due`
- **How goldens are recorded:**
  - `drive` steps by `next_tick` → `due()` → `act_due` (tests/golden.rs:124-142).
  - `tick` sets `changed = true` on every fire (osaka.rs:1594-1597).
  - `drive` paints whenever `advance` returns true, so every tick adds a hashed line that includes `now` (golden.rs:73).
- **Today a Use ticks only on the 1400 ms grid** (osaka.rs:1485 and the re-arm at 1783).
  - Ticks at key ends would add frames: the fridge closing at 1500, the shopping pitch at 2L/5, the cat bite at 7L/10.
  - Dropping grid ticks for keys that don't bob (Watch, Lounge) would remove frames.
- **The TV would freeze.** Its snow frame is computed at paint time from `(now−since)/CHANNEL_FRAME_MS` (mod.rs:1027-1032). Nothing ticks at 400 ms; it animates only because of grid paints.
- **Fix:**
  - Commit 1 keeps the existing grid exactly: `first_due` and the re-arm are unchanged, and the player only answers "which key".
  - A new commit 1b, "keys change on time", schedules ticks at key ends and keeps a frame cadence for animated props. It re-records the goldens with that reason.
  - Add a test that the TV screen alternates during a plain watch. Mutant it catches: grid ticks dropped for non-bobbing keys, freezing the TV.

### B2. "Splices never change what they wrap … the needs after each use are the same" is false
- **Needs:** they rise with `needs.pass(at − decided)` (osaka.rs:2702). Prelude and coda time count, so the needs after a wrapped use always differ.
- **Grievance timing:** `grievance_from(at, quiet)` is timed from the act's start and the end of any speech (osaka.rs:942-944, 2267-2268). A prelude therefore moves the grievance to a different frame relative to the body.
- **Appearance:** `appearance` is `speech.or(bubble)` (osaka.rs:4093). Speech already playing, such as a greeting, ends at the same absolute time in both runs, so it lands at a different point in the body.
- **Whole visits diverge:** chat arrives at fixed absolute times, so the two runs interrupt differently.
- **Fix: a scripted single-use harness.**
  - Use the same body rng seed (the length draw at osaka.rs:2282 is then identical). Force the splice off or on through the cue seam, so the cue sets it directly rather than rolling it. Start once she is quiet.
  - Compare at body-relative times `t ∈ [0, body)`:
    1. `acting(now)`, not `appearance`. `acting` is private (osaka.rs:4096), so add a `cfg(test)` accessor.
    2. `prop(now)`.
    3. The schedule of `act_due` values, offset by the body start.
    4. The credit share at a set of times.
    5. The set of `HomeEvent`s, compared as a set, not by time.
  - Drop the needs clause.
- **Add to D2 explicitly:** the grievance is timed from the body start, and bob frames are anchored at the body start.
- Mutants this catches: `grievance_from(since, …)` timed from the act's start; bob anchored at the act's `since`; `whole` stretched by the splice lengths; a `Bought` event moved to the end of the coda.

## Major

### M1. Even with the grid kept, the "refactor proof" depends on luck
- **(a) Segment shares round differently from cumulative fractions.** `floor(2L/5)+floor(L/5) ≠ floor(3L/5)`; for example L=20004 gives 12001 against 12002. Today's code uses cumulative cut-offs:
  - Watch: osaka.rs:983-989
  - Homework: 975-981
  - Pet: 995
  - **Fix:** `Span` should be `Until(n, d)`, a cumulative end within the body (the `door_beat` style, osaka.rs:823-832), plus `Rest`. Not `Share`.
- **(b) Crumple and Unpack use a strict `>`** (osaka.rs:1001, 1009). A half-open player flips one millisecond earlier, and that instant can land on the grid: L=5250 gives `4L/5 = 4200 = 3×1400`.
- **(c) Crumple's `ToeTouch(frame)` bob runs across the 4/5 boundary.** If the bob were anchored at the key's start, its parity would change on grid ticks after the boundary. The golden stage room cues MakeSofa (golden.rs:159, 222).
- **Fix:**
  - Keep `use_look` as a `cfg(test)` oracle in commit 1.
  - Assert that the player equals it for every Use × lengths: every length in `TRIAL_USE_MS` (osaka.rs:304), plus a stride through `use_duration` (916-929).
  - Include, on purpose, the lengths where a key end hits the grid.
  - Check at every grid time, and at every key end and key end −1.
  - Do the same for `door_beat` if `DOOR` moves onto the player.
  - Delete the oracle in commit 2.
  - The goldens sample only a handful of lengths, so this test is the real proof.

### M2. "None encoded twice" can't fail for a short vignette
- `reencoded` counts only after an eviction (graphics.rs:430-436), and eviction happens only at `CACHE_LIMIT = 256` (graphics.rs:33, 420-428). A cued vignette never fills the cache.
- **Fix, two parts:**
  - (a) Assert the growth in `encoded` across the vignette is at most the number of distinct looks derived from the script table (distinct (Pose, Face, Prop look) × bob frames). Mutants: a `Sunrise` channel with an unquantised frame counter; andagi keys each given a distinct face.
  - (b) Force the vignettes on (chance 1) in the busy harness: `furnished_home` / `live_in` (tests.rs:1875, 1904) shaped like `a_busy_furnished_home_stays_cheap` (1846, ≤4 re-encodes). That is where the working set is real; `furnished_home` has the desk, so chopsticks will bite.
  - Commit 5 must also expect that test and `a_furnished_home_gets_used_and_stays_cheap` (1796, ==0) to move.

### M3. The ">2 s keys declare `hold`" lint checks data nothing reads
- `Act::props()` has no `now` (osaka.rs:697), and `recheck` reads `self.act.props().stays` (2563).
- In 5a every host is a Use (`Stays::Job`) or SpaceOut (`Stays::Rest`), so a per-key `hold` changes no behaviour and the lint can't catch a bug.
- **Fix:**
  - Drop `Key.hold` in 5a. The rule holds by construction, because `Play` exists only on `Act::Use` and `Act::SpaceOut`. Defer `hold` to 5b's free-standing act.
  - If it is kept, wire it into `props(now)`. The lint must then compute each key's longest possible duration per (script, host), at `use_duration(host).1` and for every splice row's host.

### M4. The cue lint needs `ScriptId` to be an enum, and the scene checks must see the script
- `every_want_can_be_cued` (tests.rs:4684) works because it matches exhaustively over `Want::ALL`. D1's `name: &'static str` can't be matched that way.
- **Fix:** make `enum ScriptId` with `ALL` and an exhaustive `fn scene(ScriptId) -> Scene`. A new script without a scene then fails to compile.
- Each new arm in `every_scene_has_a_spot_in_the_stage_room` (`happened`, tests.rs:1424-1465) must check that the specific script or splice played, through a `cfg(test)` `playing()` accessor. A check like `posed(Homework(0))` passes even when the Chopsticks cue forgets to force the splice.
- Chopsticks needs one scene per branch, such as `ChopsticksClean` and `ChopsticksBad`, so the image tests can cue both branches.

### M5. The andagi scene can't be seen within the stage test's 10 s cap
- The cap is at tests.rs:1401. A snack body is 6–9 s (osaka.rs:924), plus the walk, and the coda comes after that.
- **Fix:** in the stage lint, check "the After splice was chosen" at `start_job`. Add a `watch_scene`-style test (tests.rs:2862, 20 s cap) that sees the andagi pose. Chopsticks before homework fits within 10 s.

### M6. The 10-minute line test on shown bubbles reports false repeats
- The census `said` counts changes in `appearance().2` (census.rs:170-179).
  - A key bubble hidden by speech and shown again counts twice.
  - "Sata andagi." five to seven times in a row is intended.
- `pick` already filters out lines said within the cooldown (mind.rs:627-634) and is unit-tested (`lines_cool_down_and_run_out`, mind.rs:791). A test of the cooldown alone is therefore tautological.
- **Honest form:**
  - (a) Extend the pure test across pools and salts. Mutants: the salt ignored, so two picks in one decision get the same roll; the 8-line budget charged to every pool rather than beat lines only.
  - (b) A routing check over simulated visits (2 seeds × 3 rooms × 12 min): every pooled line shown at time t has a `Lines` record (line, w) with `w ≤ t ≤ w + bound`, read through a test-only accessor, plus the 10-minute gap on the records. Mutants: the door arm still calls `self.say(THROUGH)` (osaka.rs:1717); riddles indexed from whims without going through `Lines`.
- **Related fix:** a line drawn at an act's start but shown later (a riddle's answer, about 3 s in) should be recorded at `since + key start`.

### M7. Credit assertions need the unit level
- A Guest-level needs comparison needs a tolerance; tests.rs:4640-4647 uses 0.15. It can't tell "a prelude credits nothing" from "credits 5%".
- **Fix:**
  - Use the osaka.rs test pattern: `pressed` (4297), then `start_job` and `credit_done` (4422-4428).
  - Add `#[cfg(test)] served: Vec<(Want, f64)>`, filled in `serve` (2438).
  - Assert a share of exactly 0.0 for an interrupt in the prelude, and 1.0 in the coda.
- Note that a prelude interrupt still matches the Use arm (2396-2409). "Credits nothing" therefore means a share of 0, which still calls `enjoyed(want, 0)` and clears `credit`; it is not an early return.
- Mutants: the span taken from the act's `since`; the coda share's denominator taken as `until`.

### M8. What the commit plan leaves out
- **Commit 1:**
  - `Play` (D2) has no field for the bought item that `Say::Pitch` needs (D5). Add `bought: Option<Furniture>`.
  - `DOOR`'s gap beat, `ms.max(gap)` (osaka.rs:825-831), is none of `Ms`, `Share` or `Rest`. Either add a variant or have the door share only the lookup.
- **Commit 2:** split it so each re-record can be attributed.
  - 2a: pool ids, with the beat pool on salt 0, and the `all_lines()` lint. Golden-neutral.
  - 2b: musings drawn from whims, the door pool, and riddles. Riddles mean SpaceOut ticks at key ends; today it ticks only at `until` (osaka.rs:1468). Re-records the goldens and seed 7.
- **Commit 3:** splice machinery with no row live can't exercise the neutrality or interrupt properties. Land a `cfg(test)` forced `Play` or a test row with the machinery; CLAUDE.md asks for tests first.
- **Commit 4:** `Pose::Eat { frame, food }` changes the `appearance` Debug output, which the goldens hash (golden.rs:41-56). The plan doesn't list this re-record. Either keep `Eat(u8)` and add a sibling such as `EatAndagi(u8)`, or list it.
- **Commit 5:** once the rows are live, every later timing shifts. The goldens and probably seed 7 re-record, which the plan doesn't list.

## Minor

- **Answer test.**
  - How to set `asks`:
    - `ChatMark` derives `Default` and `PartialEq` (idle.rs:23), and `observe` fires on `mark != view.chat_mark` (mod.rs:1273). A test sets it by injecting `ChatMark { synced: m.synced + 1, asks: true, ..m }`, then calling `advance(now+1)` and `paint`, as tests.rs:4640 does.
    - Reset `asks` afterwards, or the next increment asks again.
    - `look()` (osaka.rs:3586) must take `asks`.
  - What to assert:
    - She still uses the piece, with the same `until`.
    - Her speech is the answer, and she faces the chat.
    - A non-question interrupts her.
    - A question during the body interrupts her.
    - Mutants: `asks` ignored; `Chat::Answer` read from her own script rather than the playing part.
  - The trimmed-`?` detection needs its own test in app.rs, where app.rs:642 builds the mark. It should cover IRC versus synced ordering, since `ChatMark` has no order between the two.
  - The full literal at tests.rs:192 needs the new field.
  - Per idle.rs:21-22 the mark also changes on compaction, so an old question could be answered.
  - Since `look()` sets `watch_until` before the answer check, she will stand watching the chat after the coda. State whether that's intended.
- **"Every Prop drawn in both modes"** is already enforced by the compiler: both matches are exhaustive (mod.rs:2332-2336, art.rs:1023-1037). The honest lint is that each prop's look is distinct from the others in each mode. Mutant: ColourBars glyphs copy-pasted from Snow.
- **Trial uses:** add a unit test that `start_job` with `episode.trying` and a forced splice gives no splice.
- **Vacuous interrupt clauses:** "nothing bought twice" and "the lamp and fridge come back" can't fail for 5a splices. No splice wraps Watch or Sleep, the fridge key is first in the body, and `prop(now)` is pure in the act. Drop them, or test Sleep and Snack interrupts directly.
- **24-character rule:** this can be made impossible to break. A `line!` macro that asserts the char count in an inline `const` turns an over-long line into a compile error, including inline literals like `"Ow!"` (osaka.rs:997). `all_lines()` then only has to cover pools. The `all_lines()` walk must include both halves of each riddle pair.
- **Splicing again after an interrupt:** an interrupt in a prelude leads to a new decision, new whims and a new roll, so she can do the chopsticks again for the same homework. State whether that's acceptable.
- **Sound, one line each:**
  - D3 `prop(now)` purity: sound, and it makes "comes back" hold by construction.
  - Leaving the explain log unchanged: fine.
  - Making census `group` exhaustive (census.rs:267-281): right.

## Gate budget

| | |
|---|---|
| Measured (`PROPTEST_CASES=32`, houseguest lib, full parallel load) | Longest: `every_made_piece_is_used_or_let_go` 17.7 s, `text_arriving_…` 14.0 s, `a_trial_keeps_every_promise` 12.1 s. Goldens 1.3–5.7 s. `every_scene_has_a_spot` 3.9 s. Census ≤ 7.9 s. |
| Case counts | `PROPTEST_CASES=32` overrides every pinned count (test_support.rs:43-48, stop-checks.py:96), so budget every new property at 32 cases. |
| Simulation cost | About 0.08 s per simulated minute (`her_needs_shape_long_visits`: 80 min in 6.4 s). |
| Line routing test | 72 simulated minutes ≈ 6 s. |
| Splice neutrality | At Guest level (32 cases × 2 modes × about 1 painted minute), roughly 15 s, which would be the risk. The unit or scripted form costs milliseconds; prefer it. |
| Interrupt property | Shaped like `a_trial_keeps_every_promise`, about 12 s. It must force the splice and land chat or resizes at offsets inside the prelude or coda (cue, `run_until` to the prelude, offset drawn by proptest). With natural chances and random weather, most cases test nothing. |
| `every_scene` | About 5 new scenes × 4 runs ≈ +0.6 s. |
| Forced-vignette busy home | 2 seeds × 10 min ≈ 1.5 s. |

Nothing approaches the 60 s kill; the long pole stays near 18 s.

## Helpers to reuse

- **Promise checks:** `keep_promises` / `Weather` / `promised_frame` (tests.rs:7430 / 7384 / 7487), with `wordy_rooms` (7352) for text-dense panes.
- **Stepping and looking:** `run_until` (5227), `look_now` (5203).
- **Cued scenes:** `watch_scene` (2862) and `state_of` (2912).
- **Lamp:** extend `her_things_answer_what_she_does` (2921) to check the lamp is on during the first key, then off.
- **Chat injection:** the pattern in `an_interrupted_sleep_eases_only_what_she_slept` (4593, 4640).
- **Image budget:** `furnished_home` / `live_in` (1875 / 1904), `a_busy_furnished_home_stays_cheap` (1846), `a_carry_stays_within_the_image_budget` (6548).
- **Frame comparison:** extract `Trace::frame`'s per-frame line from golden.rs:41 (without `now`) for frame comparisons relative to the body start.
- **Simulation:** `simulate` (tests/census.rs:144).
- **Unit level:** `lines_cool_down_and_run_out` (mind.rs:791), and the `pressed` / `start_job` / `credit_done` pattern (osaka.rs:4297, 4422-4428).