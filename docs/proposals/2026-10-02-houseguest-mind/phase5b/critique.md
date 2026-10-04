# Phase 5b design: synthesis of the four critiques

# Phase 5b design: synthesis of four critiques (mechanics, tests, persistence, character)

I checked every BLOCKER and MAJOR below against the code at HEAD `5fc9310`. Paths are under `dessplay/src/ui/houseguest/` unless prefixed. Critique tags: **Mech** (mechanics), **Test** (tests and determinism), **Pers** (persistence and shell), **Char** (character).

None of the blockers or majors failed verification. Three items are reworded rather than refuted:
- The design's "`furnish`'s body-rng draw (mod.rs:906)" points at the rng *argument*. The only draw is `place_gift` (mod.rs:1696).
- The real reasons `furnish` can't run in Away are the TV order (mod.rs:1680), the parcel delivery (mod.rs:1684-1694), and the ledger mutation in `Home::project`.
- D7's "2 a real hour" for the dial is an arithmetic error. A quarter-hour of game time is 2.5 real minutes, so the dial changes 24 times a real hour.

## A. Amendments the designer can settle (ranked)

### BLOCKERS

**A1. `GameClock::at` underflows in u64** (D1 Reading). *Pers B1.*
- Evidence: `tick` fires each decision at `due <= now` (osaka.rs:1671-1675). `clock_at` is set to `now` at the top of `advance`. So every catch-up decision has `t < at`.
- Overflow checks are on in dev and tests (Cargo.toml:142-148, `opt-level = 2`, checks kept). The first catch-up decision panics the UI thread.
- Amend: "`at(t) = max(0, game as i64 + 6·(t as i64 − at as i64))`. A decision earlier than the anchor reads earlier game time. Accrual saturates when `now` goes backward."

**A2. The routine boundary in `due()`/`tick` fires the wrong act and can busy-loop** (D4 "Who owns a boundary"). *Mech B2, Test m3.*
- Evidence:
  - `due(&self)` takes no clock (osaka.rs:1547).
  - A due that isn't speech, pending or blink falls to `fire`, which acts no matter what (osaka.rs:1702). `Clamber` steps (1730-1742), `Out` steps and draws rng (1750-1757), and `Away` returns early (1771-1785).
  - A skipped interrupt that leaves the boundary unadvanced spins 64 times a tick, and `next_tick` returns 0.
- Amend:
  - "`Osaka.cut_at: Option<u64>` (monotonic) is refreshed from the clock at `tick` entry. It is the first cutting boundary after the current act's start (not after `now`), so a catch-up past 64 steps keeps it."
  - "In the loop, `due == cut_at` is handled first, before speech, pending, blink and `fire`. It always advances `cut_at` to the next cutting boundary, whether or not it interrupts."
  - "`skip_clock` and a vacation latch change (A12) recompute it."

**A3. The night is not safe from chat, so "awake at 02:00 is unreachable" is false** (D4.4 reflex position). *Mech B3.*
- Evidence: the `watching chat` Stand comes before the `Ctx` literal (osaka.rs:2961-2969 vs 3008). Every chat line re-arms it, because `watch_until = now + WATCH_MS` (15 s; osaka.rs:272, 3873). A live chat with lines under 15 s apart keeps her standing at 22:30 or 08:15 indefinitely.
- Amend: "Routine reflexes run right after `off text` (osaka.rs:2955), before `watching chat`. On the routine path she still faces `watch_x`. Build `Ctx` before this point; it reads only `here`, `terrain`, `chances` and `self`."

**A4. The night sleep on a sofa or floor is unbuildable, and each fallback ends in seconds** (D4 "Tucked in", `routine/bed`). *Mech B4, Char F1.*
- Evidence:
  - The sofa offers `[Lounge, Nap]` (room.rs:122), and `Sleep` is bed-only (room.rs:148). So `Act::Use{Sleep}` on a sofa seat is an act the piece doesn't offer.
  - Nap lasts 30–60 s and Sleep 60–180 s (osaka.rs:918-920). LieBack lasts 15–40 s (osaka.rs:989).
  - So a bedless night is hundreds of up-and-down decisions. Each re-rolls splices, and none carries the stir, the sleep-talk or the Dream.
  - Every new record's first night is bedless (bedtime is 65 real minutes after 16:00), and so is the stage room.
- Amend: "The night's sleep is one act, whatever the surface. It lasts until the wake time and carries one `ScriptId::Night` script (stir, talk, Dream, wake). The surface picks the act and pose: bed `Use(Sleep)`; sofa `Use(Nap)` posed Nap; a makeshift heap uses its sleep method; floor `LieBack` held still. Tucked-in and the bed reflex use the same binder."

**A5. "`None` ≡ today" can't be built as written** (Determinism, items 2–4; Steps 3–4). *Test B1.*
- Evidence:
  - `Trace::finish` hashes `ledger.to_json()` (tests/golden.rs:82-84). After step 2, any run with `visits > 0` accrues `clock`, so it can never match the pre-5b tables.
  - After step 4, no production path yields `None`.
- Amend:
  - "A cfg(test) `Guest::unfed()` still accrues `ledger.clock`, but returns `None` to every consumer: `tick`, Away transitions, the `next_tick` boundaries, dash wakeups, `Rares` (gives `none()`), `may_work`, the clock parcel, and the dial and sky."
  - "Step 4 first copies the end-of-step-3 tables into golden.rs as `UNFED_*` (one `#[test]` per scene, each under 30 s), then re-records the fed tables."
  - "Land `idle_min`, `rare_at` and `legend_at` accrual in step 2, so `finish` moves once."
  - "No later step may move `UNFED_*` without a trace diff in its commit."

**A6. Input or chat in the same frame cancels the return** (D3 row Away→Arriving). *Mech B1.*
- Evidence: `activity` sends `Arriving→Absent` (mod.rs:664), and so does `observe`'s chat arm (mod.rs:1337-1339). `paint` runs `observe` before arriving (mod.rs:766, 771). From Absent she waits for the idle gate, which a live chat keeps resetting.
- Amend: "`State::Arriving(How)`, with `How = Idle | Return | Dash`. Input and chat cancel only `Idle`. Only `!open` cancels `Return` and `Dash`."

### MAJORS

**A7. The clock and the date don't reach the sites that need them** (D1 Reading, D4 `Ctx`, D5). *Mech M3, Pers M5.*
- Evidence:
  - These run before or outside `Ctx`: `needs.pass` (osaka.rs:2981, with `Ctx` at 3008), the greeting (2971), `start_job`'s length draw, `SpliceCtx {what, trying, quiet}` (script.rs:615-624, which Scary's 22:00 `when` needs), `look`, and `due`.
  - The shell has only monotonic `now_millis()` (shell.rs:707-713), and `biblical_date` takes epoch ms (timeutil.rs:10-14). "`now_utc_ms`" names nothing, and passing `now_millis()` gives 1969-12-31.
  - The date reaches the guest only through `paint`'s `observe`, but D2's slot and D3's boundaries need it in `advance`.
- Amend:
  - "The shell computes `Option<NaiveDate>` from `SystemTime::now()` via `biblical_date`, with a pre-epoch value mapped to `None`. It calls `guest.set_date(d)` before every `advance`, and the same value goes into `view.local`. Drop the per-minute cache."
  - "At `tick` entry the guest sets `Osaka.clock: Option<GameClock>` and `Osaka.vacation`. One `fn day(&self, at) -> Option<DayTime>` serves every site. `SpliceCtx` gains `day`."

**A8. `clock_at` must re-latch while the clock is gated** (D1 When it runs, Accrual). *Pers M3, Test M2, Pers m1/m8, Mech N6.*
- Evidence: `visits` goes 0→1 inside `paint` (mod.rs:1148). A latch held since process start would add 6× the whole uptime on the first accrual; after `move_out` it does so again. Goldens and tests also paint before any `advance` (tests.rs:2216-2220), so `clock_at` is `None` at `begin_visit`.
- Amend: "`accrue(now)` always sets `clock_at = Some(now)`. Adding to `clock`, `clock_rem`, `idle_min` and `idle_rem` is gated on `visits > 0`. `game_clock()` uses `clock_at.unwrap_or(now)`. The first meeting is at exactly 16:00, not 16:00 plus the idle delay." The partition property gains a `visits` 0→1 case. Mutant: latch once and gate only the carry.

**A9. `routine/away` can't reuse the work exit** (D3 row 1, D4). *Mech M7, Test M8.*
- Evidence:
  - `Out` draws `rng.range` (osaka.rs:1753-1757), and `Away` walks her back in (1771-1785).
  - `evict`'s `leaving_for_work` draws `SHIFT_MS` (osaka.rs:4308-4322).
  - "Off screen" is undefined.
- Amend:
  - "`Osaka.leaving: Option<Routine>`. With it set, `Out` sets `Away{until: u64::MAX}` with no draw, and a door's gap is `u64::MAX`. `evict` keeps `leaving`, never sets `at_work`, and draws nothing."
  - "The guest ends the visit when `leaving.is_some() && hidden(now) && door(now).is_none()`. If she's already out at work when Away begins, the visit ends at once, so she doesn't come home with leeks."

**A10. `interrupt` has no guard on the act** (D4 boundary). *Mech M1.*
- Evidence: `interrupt` swaps in `Look` unconditionally (osaka.rs:3940-3957), while `look` guards `Back` and `aloft` (3889-3895). A cut means a Look in mid-air, a pop out from behind a door, or a second poke.
- Amend: "Cut only acts whose props are `Stays::Rest | Stays::Job`, and never `Poke`. `Landed` and `Back` are left alone (they end soon in `decide`, which hits the reflex). At bedtime, a bed `Use(Sleep)` already in progress is converted in place to the night act. `Cause::Routine` maps to `(0, 0)`."

**A11. Night needs saturate, and `asleep_ms` can't be derived** (D4 Needs). *Mech M2.*
- Evidence: over 85 min at ×0.25 the effective time is about 21 min. Rise times are Restless 90 s, Tidy 60 s, Fun 6 min and Hungry 20 min (brain.rs:65-78), so most needs reach 1. After a groggy errand, `set` runs `credit_done` (osaka.rs:1715), so the act is gone by the next `pass`.
- Amend: "`Osaka.slept_ms` accumulates in `credit_done` when she leaves the night act and is consumed by the next `pass`. Waking sets needs to the Morning arrival levels via `set_clock`. `asleep_ms` is zero for every other act, so unfed goldens hold."

**A12. A vacation flip mid-game-day changes the slot with no wakeup** (D2, Q4). *Mech M4.*
- Evidence: `biblical_date` flips at real 09:00 (timeutil.rs:13). On Jul 20, a game-07:30 Morning becomes Asleep; on Sep 1, a Morning becomes Away.
- Amend: "`vacation` is latched once per game day, at game 00:00 or the day's first read. Slots are a pure function of game time within a day."

**A13. Chat::Stir is indistinguishable, and `look` won't route to it** (D4 Chat at night). *Mech M5.*
- Evidence: `on_chat` is keyed on `ScriptId` (script.rs:276-294). Night and day sleeps share `Sleep`, and branch 1 is the trial branch (script.rs:299-301). `answer()` reads only splices (osaka.rs:3917-3933), and `look` consults it only `if asks` (3897).
- Amend: "Add `ScriptId::Night` with `on_chat → Stir`. In `look`, after the `aloft` check, the own script's `Stir` is checked regardless of `asks`, and then `look` returns. `answer()`'s match is extended."

**A14. Sleep-talk and the Dream have no mechanism and no draw source** (D4, D6). *Mech M6, Test M8, Char F12.*
- Evidence: splices are only a prelude or a coda (script.rs:580-600). There is one `mind.next()` per decision (osaka.rs:2940), and an 85-minute act makes none.
- Amend: "`Osaka.next_talk: Option<u64>` joins `due()`. Lines come from a ~8-line pool, one every 6–10 real minutes, drawn from `self.whims` labelled `"sleep-talk"` with salt = line index. The schedule is a pure function of the night act's start, and no line counts against the line budget. The Dream is a `Night` body branch that fires once after 30 game minutes, counted from her first sleep of the night (a groggy errand doesn't reset it). '...five more minutes' is said only in the last game hour."

**A15. The visit's per-visit rationale fails for a resident weekend** (D3 "Why B"). *Char F5.*
- Evidence: Friday 12:45 to Monday 08:15 is one visit, about 11 real hours. `LINE_BUDGET = 8` (mind.rs:735), `worked` allows one shift (osaka.rs:3021), and one mood holds all weekend. Visitors get more rare rolls than residents.
- Amend: rewrite the rationale. B stays because Away is a real state. "Waking from the night act starts a new day: without `visits++`, it refreshes the line budget, `worked`, the mood and the `Rares` draw, salted with the game day." Whether the mood greeting merges into the wake line is in B.

**A16. Dash and errand visits need a kind, and their trigger must be an edge** (D3a). *Mech M9/N5, Test M5.*
- Evidence: `send` calls `begin_visit`, which increments `visits` (mod.rs:1270, 1148), so the errand at school is counted. `next_tick` waking only at boundaries skips the dash time.
- Amend:
  - "`Visit.kind: Normal | Dash`. A dash doesn't bump `visits`."
  - "Absent and Away `next_tick` = min(boundary, dash time, gate). The dash fires only when one accrual step crosses it (`from < t ≤ to`), so a cold start or restart after it never dashes."
  - "It is kept out of the first and last 10 game minutes of the away period."
  - "Pure `fn dash(seed, day) -> Option<u16>`, plus a stage `Scene::DashIn`."
  - "At the end it goes to Away only if `open && quiet` and she has a home, else to Absent."

**A17. The clock parcel must not use `ordered`/`bought_on`** (D7 Getting them, Q3). *Test M4, Mech M8, Char F6, Pers m13.*
- Evidence:
  - `ordered.is_some()` stops `advert` (mod.rs:127-139), and `bought_on` feeds `SHOP_EVERY`.
  - The TV order's `bought_on = visits − 1` delivers in the same visit (mod.rs:1680-1693), saying `PARCEL` with no hidden check.
  - Being in `CATALOGUE` breaks the "she has it all" test (tests.rs:2317).
- Amend:
  - "The clock is in `Furniture::ALL`, not `CATALOGUE`. It is delivered by its own one-shot doorstep path in `furnish` when the clock is fed, `owns(Tv)`, `!clock_sent`, `!osaka.hidden(now)` and `kind != Dash`. `clock_sent` is set on delivery. Shopping is untouched."
  - "All parcel delivery, the TV included, gains `!hidden && kind != Dash`, and a parcel line waits `speech_ms(HOME)` after 'I'm home!'."
  - "The window is *inserted* before `Plant`, not appended."
  - "Step 8 is listed as a golden re-record, naming tests.rs:2317 and 2470-2525."

**A18. CHANGELOG entries per step** (Steps 9). *Pers M4.* CLAUDE.md requires each entry "in the same commit". Steps 2 and 4–8 each ship visible behaviour. Amend: each step carries its own entry (step 2's says "existing homes start at Monday 16:00"), and step 9 reviews the wording.

**A19. `idle_min` is always true as written** (D6 Pity). *Pers M2.*
- Evidence: `quiet_since` is only ever set to `now` (mod.rs:652, 784, 987, 1309, 1338).
- Amend: "One `fn gate_open(now)`: `open && now ≥ quiet_since + delay`, used by `advance`'s Absent arm and pity. `idle_rem` holds the remainder. A resident's hands-off playback counts; Visits Off pauses pity."

**A20. `Instant`'s suspend behaviour is unspecified in the docs** (D1 No clamp). *Pers M1.*
- Evidence: std time.rs:56 says "it is also not specified whether system suspends count". `CLOCK_MONOTONIC` is an implementation detail, and Windows is unchecked.
- Amend: "decisions.md cites the implementation (toolchain path) and the disclaimer. `Guest::cap_steps(Option<u64>)` is set by the shell to 10 min and logs at debug when it triggers. It defaults to `None`, so tests and censuses stay unclamped."

**A21. The gate breakage the design omits** (Determinism, landing). *Test M1, M3, M6, M10, Pers m9/m10.* State each step's fallout:
- **Step 2:** `advance` returns `false` on accrual alone (tests.rs:137). Absent `next_tick` stays `None` at `visits == 0` (tests.rs:136). `a_goodbye_mid_carry_…` compares `record_of` across the rain (tests.rs:7362, 7386), so it should compare `home` and `ordered` instead.
- **Step 4:** `a_furnished_home_gets_used…` asserts `Work > 0` (tests.rs:1888), and the furnished golden is "long enough to go to work" (golden.rs:251). Both move to a game Saturday at 10:00 or later (or run unfed), and a fed weekend golden is added. The seed-7 snapshot is re-recorded.
- **Fed scenes and censuses:** a test-only `Ledger::new_at(seed, GameTime)` with `visits = 1`, furniture pre-placed via `ledger.home.add`, starting Absent with the gate open. `Room.start: None` means unfed.
- **Protected-cells proptest:** `start` from `prop_oneof`[08:15±3, 12:45±3, bedtime±3, forced dash, uniform], `proptest_cases(16)`.
- **One-day sim:** split into `skip_clock` windows. The full day runs only in `day_census`.
- **Perf** (`Guest::new(rand::random())`, shell.rs:438): the date is injectable so perf runs on a date with no calendar entry.

**A22. The visit's end and Away paint** (D3). *Mech M10/N1/N2/N10, Test M9, Char F17.*
- Dropping `Visit` drops `made` and `layer` (mod.rs:213). Away keeps them in `Empty.fades` and rains them out.
- `Empty` also holds `painted` and `size`. Away's diff marks the ledger unsaved when `project` mutates it.
- The hidden-goodbye fix clears `placement` too, and lands as its own commit with a trace diff.
- Pick one: "`Leaving` carries its frozen `Looks`."
- Away paints the cat of the *next* visit (`cat_home` reads `visits − 1`, mod.rs:2245).

**A23. The calendar's "one entry a day" contradicts its own table** (D5, Tests). *Test M11, Char F7.* Feb 3 falls in exams, Apr 8 in hay fever, Dec 24 in December. Amend: "Two kinds. *Owed*: at most one a day, the most specific window wins. *Tints* (boosts, pools) stack. Exam chopsticks are a tint." Replace the vacuous "Rares at most one unseen" test with `new ∉ seen`, `open ⊆ seen`.

### MINORS (text fixes)

- **Overlay rule** (Mech N3/N4). "As Visiting does today" is wrong: an overlay makes Visiting `leave` (mod.rs:1318). Away+chat is "no change".
- **Sleep length** (Mech N7). The longest night is 105 min (Sun→Mon), not 85. Round `next_tick` boundaries up (Mech N9).
- **Tucked-in placement** (Mech N8). It goes after `visit.chances` (mod.rs:1042-1061), sets `credit = Some(Want::Use(..))`, and handles `Osaka::arrive == None` (it only needs the bed). Her `!`/`?` look at chat and the greeting wait until she wakes (Char gap 7).
- **Saving** (Pers m7, m2). A `clock_saved` base, reset on every take of `unsaved`. `touched` = `ledger != loaded` (a clone kept at restore).
- **Step 1** (Pers m4/m5). A labelled `'ui: loop`, not an inner fn (`too_many_arguments`). An exit-path table test (Quit, Shutdown, dropped sender, a handout parked in `ledger_unsent`) replaces the unstageable "lose a `Bought`" test. SIGHUP and run.rs's `?` exits stay unsaved; say so.
- **Step order** (Pers m6, Test m1). Slot-change batching and the stage `t` key need `routine.rs`, so they move to step 3. Renumber the landing order to match the Steps list.
- **Lenient read** (Pers m11/m12). `seen` is read as `Vec<Value>` filtered per entry, `clock` is clamped below 2^40, `rare_at`/`legend_at` use `saturating_sub`. The downgrade also loses the window, the clock piece and a pending order.
- **Dump** (Pers m15). Label it "as of the last save", add a `pub` summary accessor, an entry in `dump::SECTIONS`, and the dump-state skill update.
- **Docs** (Test m7/m8, Pers m16). Add testing-strategy.md "Golden Trajectories" to "Docs that change". Add an `"away"` trace label and `DayTime` in `Decision`. design.md's move-out text must mention the clock, pity and seen reset.
- **Clock-change rate** (Char F14). The dial changes 24 times a real hour. Fix D7 and redo the image-budget test on a fed 17:30–19:30 visit with LookOut and glances (Test M7).
- **Lines** (Char F3, F4, F8, F9, F11, F13).
  - Rares duplicate MUSINGS (mind.rs:665-675).
  - NoMelon follows an `Eat` melon-bread pose.
  - Apr 8's line equals the Ordinary mood greeting.
  - The sunrise watch is Jan 1 only.
  - Single-line seasonal and LookOut pools repeat. Make them pools of 2–3 per phase or season.
- **Waking pose** (Char F19). There is no sit-up pose: wake as `Stretch` beside the bed.
- **Routine doors** (Char F2). The departure, return, dash and errand-at-school doors never draw the DOOR pool. "I'm home!" goes through `home_from(why)` at the door's end; leeks stay Work-only.
- **Rare when-windows** (Test m4, Char gap 2). Gated splices are skipped before `may` and the roll. `Rares.new` is chosen only from rares whose `when` can occur this visit.
- **Line garble** (Char F20). Replace "at most `LINE_BUDGET`-free".

## B. The user's decisions (product and feel)

1. **When the wall clock arrives:** in the visit where the clock first runs (Char F6: it explains the first bedtime and departure), or on the next visit. A17 makes either free of test fallout.
2. **The Away cue:** a closed pink door (`DoorFrame::Closed`, art.rs:811-834) stands where she left, or only the lamp off (Char). Lamp-off looks like asleep and is easy to miss at noon.
3. **Each morning a new day (A15):** a fresh mood, line budget, shift and rare roll each game morning. Should the mood greeting merge into the wake line ("Mornin'... lazy day.")?
4. **Retire the musings the rares grow from** (Char F3): drop "Escalator? Elevator?", "Oh my gah." and "Chiyo-chan's dad..." from MUSINGS. The Dream becomes "Hello everynyan…/Fine sankyu…/Oh my gah!". Tanabata becomes "Wrote my wish. Secret!". This changes her common voice and goldens.
5. **Interim for Toast #46** (Char F16): until the art exists, does a departure that cuts breakfast say "Late, late, late!" with a faster walk (needs a second `WALK_MS`)? Door rather than edge for every routine exit?
6. **Dash-in without a fridge** (Char F18): "Forgot somethin'..." / "...what was it?" rather than "Forgot my lunch!" with nothing grabbed.
7. **Cheap extras** (Char): meal lines by time of day, "Night-night..." on the lamp key, "No school today!" on weekend mornings, a midnight-snack errand, pooled "I'm off!"/"I'm home!" variants, a sleepy "Mm... someone said..." poke, an afternoon clock glance.
8. **A visitor ending a night visit** (Char F21): a `Face::Blink` goodbye instead of the startled one?
9. **Line replacements:** Apr 8 becomes "It's my debut day!". NoMelon becomes "That was the last one.". Add a Dec 31 "Year's almost over...". December musings become a pool of 3.
10. **Art notes for the model sheet** (Char F15): a light window frame, a cream dial, visible stars, and a 1× crop on #1e2127.

## C. Left unspecified (the designer should state)

- The return's constructor: door spot, `since = now − DOOR_THROUGH_MS`, keeping the reseed at mod.rs:776, and a flag for "I'm home!" without `at_work`.
- From Absent with no home: is the 12:45 return the normal `arrive` (which may drop from the sky)? Does the no-drop rule apply?
- Who clears `leaving`: `place`, `errand`, `evict`.
- `DashLunch`'s `ScriptId`, its `played_on` host for the script lints, and whether it sets `credit`.
- Groggy errand: the `Face::Blink` flag in `appearance`; whether `poke` says `POKE` or "...mm?"; whether the night act resumes or restarts.
- `whole` for a lengthened use: the draw or the override (it changes `credit_done`'s share).
- A resident Absent at 12:45 (after an overlay or restart): the routine return or the idle gate?
- The stage: which `How` does `cue(Arrive)` from Away use? Does `cue(Arrive)` at night tuck her in? `skip_clock` refreshes `cut_at` and any stretched `until`.
- The calendar's owed check needs an in-Osaka "played today" flag, because `record` runs after `tick` (mod.rs:724-727). Calendar greeting order versus the mood greeting, "I'm home!" and tucked-in. A resident on screen at the day change gets it as an owed spaceout.
- The five sky phases' minute boundaries.
- Whether `idle_min` accrues while `persist == false`. Whether accrual alone sets `touched`.
- Which modes the plan.md numbers were measured in. The `Saved` field order and a `the_record_as_written` twin with every field set.
