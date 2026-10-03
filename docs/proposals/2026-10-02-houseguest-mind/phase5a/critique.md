**Phase 5a design: merged amendments from the three critics (mechanics, tests, character)**

Paths are relative to `dessplay/src/ui/houseguest/`. I re-checked every disputed claim in the code; the line refs below are the current ones.

## 1. Blockers and majors, each with the amendment to write in

### Blockers

**B1. Commit 1 can't keep the goldens unchanged while D1 changes `first_due`.** All three critics found this.
- Today a Use wakes only on the 1400 ms grid counted from `since`: osaka.rs:1485 and the re-arm at osaka.rs:1782-1783. `tick` returns changed, `advance` passes it on, and `drive` hashes a frame on every true (tests/golden.rs:134-140).
- Several key ends fall off that grid: Homework at L/2 and 3L/4, Snack at 1500, Pet at 7L/10, Shopping at 2L/5 and 3L/5. Each would add a hashed frame.
- `arrived_within` (golden.rs:146, 172, 230) makes chat arrival depend on the steps taken, so extra steps change her behaviour, not only the trace.
- Dropping grid wakeups for still keys freezes the TV. Its frame is computed at paint time (mod.rs:1027-1033). It also delays when a grievance counts as felt.
- **Amendment:**
  - Commit 1 keeps `first_due` and the re-arm exactly as today. The player only answers "which key".
  - Fold `fire`'s duplicated re-arm (1782-1783) into `self.act_due = self.first_due(at)`, as Lift already does (1813).
  - A separate commit, "keys change on time", makes the due `min(next grid frame from body_start, next key end, until)` and re-records with that reason. The grid wakeup never goes away for any key.
  - **Conflict:** the tests critic wanted this as its own commit; the mechanics critic wanted it folded into commit 3. I side with a separate commit, because CLAUDE.md wants each re-record to have its own reason.
  - Add a test that the TV screen alternates through a plain watch.

**B2. "Splices never change what they wrap" can't pass as stated.**
- The needs rise with `needs.pass(at - decided)` (osaka.rs:2702), so a coda changes the needs.
- Chat arrives at absolute times, so whole visits diverge after the first wrapped use.
- The grievance is timed from the act start (osaka.rs:2268).
- The mechanics critic called this a major and the tests critic a blocker. I side with blocker: commit 3 depends on this test.
- **Amendment:** rewrite it as a unit (scripted single-use) property over `start_job`. Same seat, chances, rng state and `at`; the splice forced on versus off through a cue slot, not rolled. Assert:
  1. The rng state afterwards is identical.
  2. `whole`, the body length, the purchase and the `HomeEvent`s (compared as a set) are equal.
  3. `acting(body_start+t)` (add a `cfg(test)` accessor; `acting` is private at osaka.rs:4096) and `prop(body_start+t)` are equal for every t in the body.
  4. The `act_due` schedule offset by `body_start` is equal.
  5. The credit share at sampled body times is equal.
  - Drop the needs clause. Any Guest-level run compares only up to the end of the first wrapped use.

### Majors

**M1. Spans are cumulative ends, not summed durations (all three).**
- D5's `Share(2,5)` then `Share(1,5)` rounds differently from today's code:
  - Watch at L=20004: summed gives 12001, the code gives 12002 (osaka.rs:983-989).
  - Homework at L=30003: summed gives 22501, the code gives 22502 (osaka.rs:977).
- **Amendment:** `Span::{Ms(abs_end), Upto(n, d) /* end = body*n/d */, Rest}`, all cumulative from `body_start` and clamped to the body. Lint that every `Ms` end is at most the shortest body, including `TRIAL_USE_MS` (osaka.rs:304).
- **Conflict over `Until(fn)`:** the mechanics critic kept it for Crumple and Unpack's strict `>` (osaka.rs:1000, 1009); the character critic would drop it. I side with dropping it:
  - Pet is exactly `Upto(7,10)` (`bite_at` = `length*7/10`, osaka.rs:911).
  - The strict `>` is a 1 ms difference. Add a tiny commit 0 changing both to `>=`, so the player is uniformly half-open with no fn-pointer variant.
  - A goldens check settles commit 0: if a paint lands exactly on L·4/5 or L·3/5 (for example Crumple at L=5250 gives 4200 = 3×1400), re-record with that reason.
- Same family: `DOOR`'s `beat.ms.max(gap)` (osaka.rs:825-831) fits no Span. Share only a generic cumulative lookup `at<T>(&[T], elapsed, end_of)` rather than forcing `DoorBeat` into `Key`.

**M2. The bob phase is anchored at `body_start`, not at the key start (all three).**
- Today the frame is `elapsed/USE_FRAME_MS % 2` from `since` (osaka.rs:959).
- Snack's Eat key starts at 1500, and `ToeTouch(frame)` runs across both of Crumple's and Unpack's keys.
- **Amendment:** `Bob` is relative to the body start.

**M3. One `body_start = since + before.len`, used at every site (all three).**
- `grievance_from(at, quiet)` at osaka.rs:2268. Today it would put the grievance over the chopsticks prelude. Homework feels the lamp rule.
- The credit span `span(*since, since + whole)` at osaka.rs:2406.
- The overlay offset `from.saturating_sub(since)` at osaka.rs:4127.
- The bob and the grid (M2, B1).
- **Amendment:** write it in D2 as "the grievance, bob, grid and credit are all timed from `body_start`". Without a prelude, `body_start == since`, so commit 1 is unchanged.
- A prelude interrupt still matches the Use arm (osaka.rs:2396-2409). "Credits nothing" means a share of 0.0, which still calls `enjoyed` and clears `credit`.

**M4. Pending speech hides a script's first key (character M2).**
- Appearance is `speech.or(bubble)` (osaka.rs:4093).
- Speech pending at the moment a host starts:
  - the door says `THROUGH` and then `decide`s (osaka.rs:1717-1718);
  - `AH_RIGHT` is said just before `Job::Use` (osaka.rs:3020-3022);
  - `muse` speaks (osaka.rs:1530).
- So a riddle could show its answer with the question hidden, and a bad chopsticks split could lose its line.
- **Amendment (my choice of the two fixes):** a Before splice or a riddle is not rolled while speech is pending. Add `quiet <= at` to `SpliceCtx`, and give the riddle choice in `muse` the same gate, falling back to a plain musing.
- I chose this over the alternative, storing a start offset in `Play`, because it keeps `Play` as drawn and keeps `body_start` to a single expression. It adds no body draw.

**M5. `Play` cannot hold what `appearance(now)` needs (mechanics M4, tests M8).**
- **Amendment:** `Play { own: ScriptId, branch: u8, before, after, drawn: [u8;2], bought: Option<Furniture> }` and `Spliced { splice: SpliceId, len: u64, branch: u8 /* or count */ }`.
- The andagi count must be stored, not inferred from `len`.
- Hold ids only. A `&'static Script` with fn pointers would make the derived `PartialEq` on `Act` compare addresses.

**M6. `ScriptId` is an enum with `ALL`, and each scene is checked against its script (tests M4).**
- D1's `name: &'static str` can't be matched exhaustively the way `every_want_can_be_cued` does (tests.rs:4684).
- **Amendment:**
  - `enum ScriptId` and `enum SpliceId`, each with `ALL`, plus an exhaustive `fn scene(id) -> Scene`.
  - The stage lint's `happened` checks the specific script or splice through a `cfg(test) playing()` accessor, not a pose.
  - One scene per chopsticks branch, for example `ChopsticksClean` and `ChopsticksBad`.

**M7. `Act::SpaceOut` needs `since` and `play: Option<Play>` (mechanics M5, character m2).**
- It is built at three sites: `muse` (osaka.rs:1534), swap-back (osaka.rs:1900, 1.5-3 s), and `plan` (osaka.rs:2916).
- **Amendment:**
  - Only the `Here::Muse` binding may carry a riddle (6-14 s fits about 3 + 2.5 s). The swap-back and plain sites get `None`.
  - `first_due` stays `until` when there is no play.
  - A riddle on 1 musing in 3.
  - Riddles mean SpaceOut now ticks at key ends; today it ticks only at `until`. That re-record goes with the riddles commit.

**M8. `Lines`: the budget and the salt (mechanics M7, character m1).**
- `pick` gates on `said.len() >= LINE_BUDGET` and rolls `w.chance("line", 0, n, d)` (mind.rs:626). `w.below("which-line", …)` takes no salt (mind.rs:639).
- **Amendment:**
  - Store `(pool, line, at)` and count the budget over beat pools only.
  - Salt both the "line" and "which-line" rolls by pool id. The beat pool is id 0, so existing beat picks keep their rolls.
  - A line drawn at an act's start but shown later (a riddle's answer) is recorded at its key's start.

**M9. `ChatMark.asks` as one level breaks arrival detection (mechanics M8, tests minor).**
- Arrival is detected as `mark != view.chat_mark` (mod.rs:1273). Compaction changes the mark too (idle.rs:20-21), so a compaction while the newest line is a question would be answered.
- "Newest synced or IRC" has no defined order. app.rs:642 builds the mark statelessly.
- **Amendment:**
  - Per-source levels `synced_asks` and `irc_asks`, each from that source's last line after trimming.
  - The guest treats an arrival as asking only when that same source's counter increased and its level is set.
  - Test the trimmed-`?` detection in app.rs, and fix the full literal at tests.rs:192.

**M10. Where the Answer goes in `look()` (mechanics M9).**
- `look()` sets `watch_until`, clears `hopping`, drops a mine heading and resets pending (osaka.rs:3587-3603). It sets `watch_x` only after the `props()` check (osaka.rs:3606).
- The `props()` early return only fires for `OnChat::Back`, so placing Answer "before props()" buys nothing and leaves `watch_x` and `facing` stale.
- **Amendment:**
  - Put the Answer branch after `watch_x` and the `aloft` check, before the restful/interrupt split.
  - It sets `facing = toward(x, chat_x)`, says the answer, and returns without interrupting.
  - Keep the pending reset above it, so mischief still goes back.
  - State that she then stands watching the chat for `WATCH_MS` after the coda (intended), and that she stays turned to the chat for the rest of the coda.

**M11. Props in both modes, and the ASCII gap (mechanics M10 vs. the tests critic).**
- Both critics are half right:
  - `art::Channel` matches are exhaustive in both modes (mod.rs:2331-2335, art.rs:1023).
  - ASCII's state match ends in `_ => None` (mod.rs:2326-2329). `LampOff` and `FridgeOpen` exist only in line art (art.rs:886-887), so #70 is invisible in ASCII today.
- **Amendment:**
  - Make the ASCII state match exhaustive, so a new prop forces a decision.
  - Either add ASCII glyphs for lamp-off and fridge-open in a commit that re-records, or record "line art only".
  - Replace the lint with "each prop's look is distinct from the others in each mode" (mutant: ColourBars copying Snow's glyphs).
  - `Prop` is keyed by Furniture kind and applies to every shown piece of that kind. `CatBiting` stays under the `if cat` guard. `Prop::Tv` carries the channel; `prop(now)` resolves the 400 ms frame from the use's start.

**M12. The image-budget test is vacuous (tests M2).**
- `reencoded` counts only after an eviction, and eviction happens only at `CACHE_LIMIT` = 256 (graphics.rs:33, 420-436). A short vignette can't trigger it.
- **Amendment:**
  - (a) The growth in `encoded` across a cued vignette is at most the number of distinct looks derived from the script table.
  - (b) Force the vignettes on (chance 1) in the busy harness (`furnished_home`/`live_in`, tests.rs:1875/1904, shaped like tests.rs:1846).
  - Commit 5 expects tests.rs:1846 and 1796 to move.

**M13. Drop `Key.hold` (mechanics and tests).**
- `Act::props(&self)` has no `now` (osaka.rs:697). In 5a every host is a Use (`Stays::Job`) or a SpaceOut (`Stays::Rest`), so `hold` is dead data and its lint can't catch a bug.
- **Amendment:** defer it to 5b's free-standing act.

**M14. The census can't see Snack or the lamp (character M4).**
- `furnished_room` owns Sofa, TV, Bed, Desk and Bookshelf only (tests/census.rs:48-56). The andagi and lamp rows would read 0.
- **Amendment:** add Fridge and Lamp to the home room (or add a kitchen room), and re-pin the baseline with the reason.

**M15. Sata andagi won't read as "a few times, happier each time" (character M3).**
- Identical back-to-back bubbles read as one bubble, and the census counts text changes (tests/census.rs:170-179).
- **Amendment:**
  - Each "Sata andagi." key lasts about `speech_ms` (1.9 s), then a 0.5-0.8 s quiet beat (a bob or `Hum`).
  - Face ramp Vacant → Pleased → Happy. Curious reads as a question; Pleased and Happy exist at sprite.rs:83/85.
  - Open the coda with a ~1 s `FridgeOpen` key, hold the andagi up (`Still`) through the saying keys, and bite on the last two.
  - The Answer uses Happy and is said toward the chat.
  - Count 4-6, not 5-7: the user said "a few", and 6 keys already take about 15 s after a 6-9 s snack. That count is the design's call.

**M16. Channel surfing (character M5).**
- **Amendment:**
  - Use the advert's own filter, `!trying && grievance.is_none()` (osaka.rs:2272-2274). A trial watch lasts 3.5-5 s, and a grievance overlays the surf.
  - Add a per-script cooldown (no script twice in 10 minutes) in the `Lines` store. It bounds chopsticks and andagi too.
  - Chance 1 in 5.
  - The joke: Ooh on the sunrise, then `Rest` on snow with Happy + Hum.

**M17. Door pool chance gate (conflict: mechanics "accept silence or 1/1" vs. character "1/3").**
- I side with 1 in 3. The simulator heard "Where was I?" 467 times in 16 half-hour stage visits (plan.md:2316-2319).
- Without a gate, 5 lines on a 10-minute cooldown come in bursts: five doors talk, then the rest are silent until the cooldowns expire.
- Gated at 1 in 3 (with the 10-minute cooldowns), the census critic projected about 8-10 lines a stage visit.
- State that the door may now be silent; `THROUGH` at osaka.rs:1717 was unconditional.

**M18. Test-level fixes (tests M1, M5-M7).**
- **Refactor oracle:** keep `use_look` as a `cfg(test)` oracle through commit 1. Assert that player equals oracle for every Use × {all `TRIAL_USE_MS` lengths, a stride through `use_duration`, lengths where key ends hit the grid}, at every grid time, at every key end, and at every key end −1. Delete the oracle in commit 2.
- **Andagi scene:** it can't show within the stage lint's 10 s cap (tests.rs:1401). Check "After splice chosen" at `start_job`, and see the pose in a `watch_scene`-style test (tests.rs:2862, 20 s cap).
- **Lines test:** replace "no pooled line twice in 10 minutes" (a tautology given mind.rs:627-634) with two tests:
  - (a) pure tests across pools and salts (mutants: salt ignored; budget charged to every pool);
  - (b) a routing check over 2 seeds × 3 rooms × 12 minutes: every pooled line shown has a `Lines` record within bounds (mutant: the door still calling `say(THROUGH)`).
- **Credit tests:** use the unit pattern (osaka.rs:4297, 4422-4428) with `#[cfg(test)] served: Vec<(Want, f64)>` filled in `serve`. Assert exactly 0.0 for an interrupt in the prelude and 1.0 in the coda.
- **Interrupt property:** force the splice and land the weather at offsets inside the prelude or coda. With natural chances most cases test nothing.

## 2. Minors

- **Stale whims.**
  - `self.whims` needs a defined initial value with no mind draw, for example `Whims(mind.0 ^ SALT)`, because `arrive_for_errand` (osaka.rs:3804) can reach a door line before any decision.
  - Stage cues (stage.rs:296/316/341) and Tear → `pursue(Crumple)` (osaka.rs:1878) see the previous decision's whims.
  - Add `Osaka.cued: Option<ScriptId|SpliceId>`, consumed in `start_job` and `muse`, so cues force rather than roll.
  - The character critic's "all doors in one trip share a roll" is refuted: every door end calls `decide` → `choose_next`, which sets fresh `Whims` (osaka.rs:1718, 2638, 2661).
- **Splicing again after an interrupt.** A use resumed after an interrupt goes through a new decision, new whims and a new roll, so it can splice again. Say so in D4; "can't splice twice" holds only within one act.
- **Trial uses get no splice.** Add a unit test: `start_job` with `trying` and a forced splice gives none.
- **Splice scripts use `Ms` spans only** (a share would scale with the wrapped body). Lint it. Lint that `Posed::Host` appears only in scripts hosted on Watch, since the base pose comes from osaka.rs:961.
- **Lamp key on trial sleeps.** Skip #70's 2 s lamp-on key on trials (3.5-5 s), or the lamp flickers.
- **`HomeEvent::Used`** is pushed at `start_job` (osaka.rs:2276-2280). Moot for 5a, since no splice wraps a made piece. Note it moves to `body_start` if that changes.
- **`use_span` sees the whole act including splices:** the Crumple scrap stage (mod.rs:2056-2070), census `doing`, and tests.rs:5288/5295. All are safe under the no-wrap lint. Note it in D2.
- **Andagi delays need relief.** Snack is credited at `until`, after the coda. Accept it explicitly.
- **Drop vacuous interrupt clauses.** "Nothing bought twice" and "lamp and fridge come back" can't fail for the 5a rows (`prop(now)` is pure in the act). Drop them, or test Sleep and Snack interrupts directly.
- **24-character rule.** A `line!` macro with an inline `const` assert makes an over-long line a compile error, inline literals like `"Ow!"` (osaka.rs:997) included. `all_lines()` then only has to cover pools, and must include both halves of every riddle pair.
- **Chopsticks.**
  - Spans: joined ~1 s, split ~0.8 s, result ~2.5 s (the `speech_ms` of "Hold 'em by the ends!", 21 characters).
  - Branches even.
  - There is no sparkle `Bubble` (osaka.rs:1087-1101): use `*` then `Hehe` with Happy. No wide emoji.
- **Riddle delivery.** The question with a Curious face, the answer immediately with Happy, then `Hehe` rather than Dots.
- **Adverb.** Dropping it from 5a is sound, but record in plan.md that it is deferred and why (moods already set rates). Optional: give `SpliceCtx` the mood so rows can scale their chance by it.
- **Keep 5b open.** Make the player host-agnostic (`at(keys, elapsed, body: Option<u64>)`), and make census `group` exhaustive now (tests/census.rs:267-281).
- **design.md text to amend:**
  - 1389: the Answer exception to the chat reaction.
  - 1377, 1803: the door pool.
  - 1501: the budget and cooldown by pool.
  - 1512: musings, some now riddles.
  - 1620: the snack may end in an andagi.
  - 1622: the lamp goes dark from 2 s in.
  - Add a line for channel surfing.
- **Gate budget is fine.** The long pole stays near 18 s.
  - Prefer the unit form of B2 (milliseconds; at Guest level it would be about 15 s).
  - The routing test is about 6 s, the interrupt property about 12 s at 32 cases.
  - Every new property must be budgeted at 32 cases, since `PROPTEST_CASES` overrides the pinned counts.

## 3. Proposed content

All counts were checked with `len()`. The lines are ASCII (straight apostrophes, three-dot ellipses), so `len()` equals `chars().count()`.

**Riddles (question → answer):**

| # | Question | Chars | Answer | Chars |
|---|---|---|---|---|
| 1 | "Bread ya can't eat?" | 19 | "A fryin' pan!" | 13 |
| 2 | "Which animal's bread?" | 21 | "The pan-da!" | 11 |
| 3 | "What has keys, no locks?" | 24 | "A keyboard!" | 11 |
| 4 | "Has a neck but no head?" | 23 | "A bottle!" | 9 |
| 5 | "All holes, holds water?" | 23 | "A sponge!" | 9 |
| 6 | "Goes up, never down?" | 20 | "Yer age!" | 8 |
| 7 | "What never comes today?" | 23 | "Tomorrow!" | 9 |

Riddles 1 and 2 both rest on the pan = bread pun; keeping both is the author's call.

**Door pool:**

| Line | Chars |
|---|---|
| "Where was I?" | 12 |
| "Huh? How'd I get here?" | 22 |
| "...What was I doin'?" | 20 |
| "I forgot what I forgot." | 23 |
| "Handy, these doors." | 19 |

The design's examples "Back again!" (11) and "Hm? Oh, right." (14) fit the length limit. The character critic replaced them on meaning:
- "Back again!" doesn't fit: she has just arrived somewhere new.
- "Hm? Oh, right." collides with `AH_RIGHT` ("Ah, right!", mind.rs:597), which the same `decide` can say a moment later.

**Other lines:** "Hold 'em by the ends!" (21) and "Sata andagi." (12).

## 4. Revised commit plan

Each commit: fmt, clippy -D warnings, nextest. Every golden re-record states its reason.

| # | Commit | Goldens / seed 7 |
|---|---|---|
| 0 | Crumple and Unpack switch at ⅘ and ⅗ inclusively (`>` to `>=`, osaka.rs:1000, 1009). | Expected unchanged; re-record with that reason if a paint lands on the boundary. |
| 1 | Scripts as data: `script.rs`, `ScriptId`/`SpliceId` enums, cumulative `Span`, a player anchored at `body_start`, `Play` (with `bought`), `Prop` (ASCII state match exhaustive), `advert`/`watching()` removed, `fire` re-arm folded into `first_due`. The grid wakeup is unchanged. Oracle test against `use_look`. | Unchanged. |
| 1b | Keys change on time: due = min(grid, key end, until); TV-alternates test. | Re-record (key ends). |
| 2a | Lines by pool: pool ids, salts, beat-only budget, `(pool, line, at)` store, per-script cooldown, `line!`/`all_lines()` lint. | Unchanged. |
| 2b | Musings drawn from whims, the door pool (1 in 3), `self.whims` with a defined initial value, `SpaceOut { since, play }`, riddles on Muse only and only when quiet. | Re-record (one `muse` draw fewer, door lines, SpaceOut key-end ticks). |
| 3 | Splice machinery with a `cfg(test)` forced-`Play` row or cue slot, `body_start` at all sites, the B2 unit property, the interrupt property, channel surfing (advert filter, 1 in 5), lamp off with the lamp key skipped on trials, stage cues (`cued` slot, `playing()` accessor), lints. | Re-record (surfing, lamp). |
| 4 | Art (D7), drawn in parallel and wired in after approval. Keep `Pose::Eat(u8)` and add a sibling `EatAndagi(u8)` so the hashed Debug output of melon bread doesn't change. Otherwise list a re-record here. | Unchanged if the sibling is used. |
| 5 | Chopsticks and andagi rows live, the speech gate on Before splices, per-source `ChatMark` asks, the `Chat::Answer` branch placed after `watch_x`, the image-budget tests in the busy harness, the andagi `watch_scene` test. | Re-record goldens and seed 7 (later timing shifts once rows are live); tests.rs:1796 and 1846 move. |
| 6 | Census: Fridge and Lamp added to the home room (re-pin), script, splice and line rows, an exhaustive census `group`. 256-case release pass, perf, plan.md (including the Adverb deferral), design.md / decisions.md amendments, CHANGELOG. | — |

**Sound as designed (leave alone):**
- D1's choice of hosts and no free-standing `Act::Script` in 5a.
- The grievance overlay as a post-pass, once it is timed from `body_start`.
- D2's body/credit model.
- `prop(now)` being pure, so "the lamp comes back" holds by construction.
- D5's purchase committing in `start_job` before `rng.range` (osaka.rs:2272-2282).
- Unpack and Crumple events at `until`, plus the no-wrap lint.
- `recheck`, `lost_seat`, `seat` and `at_job` through a prelude or coda.
- Nothing compares `Act` with `==` (only `matches!`), so adding `Play` is safe.
- D7 model sheet first.
- D8 stage cues.
- The explain log unchanged.
- The silent `Dots` key for #70.