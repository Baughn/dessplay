# Critique: phase5b-design.md, through the tests, determinism and goldens lens

Every reference below was checked against the code at HEAD. Paths without a prefix are under `dessplay/src/ui/houseguest/`.

## BLOCKER

**B1. "None ≡ today" doesn't work as written.** (Determinism and goldens, landing items 2–4; Steps 3–8)

What's wrong:
- **It compares against the wrong tables.** Item 4 traces a clock-`None` visit "against the pre-5b tables". But `Trace::finish` hashes `guest.ledger.to_json()` (tests/golden.rs:81-97), and item 1 moves every `finish` hash on purpose. No `None` run can match the pre-5b tables after step 2.
- **Nothing in the code can produce `None`.** D1 runs the clock whenever `visits > 0`, so after step 4 no shipping code path ever gives the mind `None`.
- **Later steps move both tables.** The design says only steps 2 and 4 move goldens. But:
  - step 7's `idle_min` accrues whenever the gate is open (`open && now ≥ quiet_since`), probably in all four scenes. Stage runs 300 s idle, which writes `"idle_min":5`, and that moves all 32 `finish` hashes again;
  - step 8's clock parcel moves frames and JSON (see M4).
- **Step 4 in the Steps list never creates the table.** It says only "the re-record, plus the clock-fed golden scenes".

Amendment:
- Add a cfg(test) `Guest::unfed()`. It still accrues `ledger.clock`, but `game_clock()` returns `None` to every consumer:
  - the `tick` parameter and `DayTime`;
  - the Away transitions;
  - the Absent/Away `next_tick` boundary and dash wakeups;
  - `Rares` (gives `none()`) and `may_work` (gives today's rule);
  - the clock parcel, and the dial and sky.
- Land `idle_min`, `rare_at` and `legend_at` accrual with the clock at step 2, so there is one `finish` re-record. Otherwise step 7 needs its own trace-diff re-record of both tables.
- Step 4 copies the end-of-step-3 tables into `golden.rs` as `UNFED_*`, as four separate `#[test]`s so each stays under nextest's 30 s SLOW mark, before re-recording the fed tables.
- State the invariant: no later step may move `UNFED_*` without a trace diff in its commit.

## MAJOR

**M1. What moves at each step, stated exactly.** (Landing items 1–3)
- **Step 2: frames hold only under two conditions.**
  - `advance` must return `false` on accrual alone. tests.rs:137 already asserts `!guest.advance(3_600_000)`.
  - Absent `next_tick` must stay `None` while `visits == 0`, or tests.rs:136 (`next_tick(0) == None` on `Guest::new`) fails. Say both.
- **Step 2 breaks a gate test the design doesn't name.** `a_goodbye_mid_carry_leaves_the_piece_where_it_stood` (tests.rs:7358-7395) compares the whole `to_json()` across 3.75 s or more (`DURATION_MS`, dissolve.rs:17, plus the set-down loop). That can cross a whole-minute carry, depending on `clock_rem`. Compare `ledger.home` and `ordered` instead. Don't zero the clock in `record_of`.
- **Step 3 (unfed): goldens hold only if `Rising.asleep_ms` is set for the night sleep alone.** That sleep needs a fed clock. If the builder applies it to every `Use(Sleep)` or `Nap`, today's 1–3 min day sleeps change `pass` (osaka.rs:2976-2982) and every golden with a nap moves. Say "zero unless the act is the night sleep". The `may_work` merge is neutral: osaka.rs:3020-3024 and 3074-3078 are the same expression.
- **Step 4: what moves.** The Afternoon boosts on Snack and Lounge, Sleepy at ×0.3, and `may_work` closing on a Monday. No wakeups are added in 16:00–17:00, because the next cutting boundary is 22:30. It also breaks:
  - `a_furnished_home_gets_used_and_stays_cheap`. It asserts `count(Want::Work) > 0` (tests.rs:1888) in Monday-16:00 20-minute visits, which D2 now forbids. Its "she never chose [Sleep]" branch (tests.rs:1874-1882) is at risk under ×0.3.
  - The furnished golden. Its doc says "long enough to go to work" (golden.rs:217), and it loses all emergent-work coverage.
  - The seed-7 snapshot (tests.rs:703). Sleepy ×0.3 changes need levels even in an empty home.

  Amendment: start those tests on a game Saturday at 10:00 or later (or run them unfed), add a fed weekend furnished golden, and list the seed-7 re-record in step 4.

**M2. `game_clock()` before the first `advance`, and the latch while `visits == 0`.** (D1, Accrual and Reading)
- **Unlatched clock.** `begin_visit` runs inside `paint` (mod.rs:772-779; via `send`, mod.rs:1264-1270). Golden `drive` (golden.rs:124-126) and `arrive_drawn` (census.rs:276-279) paint at 0 before any `advance`. With `clock_at == None`, `set_clock`, `tucked_in`, `Rares` and the calendar all see no clock. So the planned "23:00 tucked-in" scene would silently test an untucked arrival.
- **Latching while she hasn't met you.** If `clock_at` latches while `visits == 0`, the first accrual after the first meeting adds 6× the whole wait. Two hours of playback before the first idle stretch puts her first meeting at 04:00.
- Amendment: while `visits == 0`, each `advance` re-latches `clock_at = now` and accrues nothing. An unlatched clock reads as frozen at `ledger.clock + clock_rem`. Extend the partition property to cover a `visits` 0→1 transition. Mutant: latch once and gate only the carry.

**M3. Building fed scenes, sims and censuses.** (Determinism, item 5; Censuses; Sims)
- A `Guest::new` never runs its clock until a visit, and `cue` bypasses the routine. Map T4 requires this, or `every_scene_has_a_spot_in_the_stage_room` breaks. `give` places a piece only once she's visiting.
- So "Monday 06:00", "08:10", "23:00 tucked-in" and `day_census` "from Monday 00:00 for each room" can't use `arrive_drawn` (cue plus give). That path gives an awake arrival at midnight with no bed at `begin_visit`.
- Amendment:
  - add a test-only `Ledger::new_at(seed, GameTime)` with `visits = 1`;
  - pre-place furniture with `ledger.home.add`, as the resident golden does (golden.rs:197-207);
  - start Absent with the gate open, so tucked-in, Away and the return happen naturally;
  - define `Room.start: None` as unfed (B1). Otherwise `visit_census` (30 min, reaching 19:00) and `image_census` (120 min, reaching 04:00 and asleep from 22:30) aren't comparable with 5a;
  - name which gate census tests stay unfed: `her_mood_shows`, `the_census_counts_her_vignettes` and `every_pooled_line_shown_was_drawn_from_its_pool`.

**M4. The clock parcel moves goldens and breaks tests.** (D7, Getting them; landing order)
- **Golden homes qualify at once.** Resident owns a TV from the start. Furnished is given one, and the stage room gets one through `Scene::Shopping`.
- **Same-visit delivery.** Copying the TV order (mod.rs:1680-1683, `bought_on = visits−1`) would deliver it in the same visit.
- **It blocks the shopping channel.** Even with next-visit delivery, `ordered.is_some()` makes `advert` return `None` (mod.rs:127,138), so:
  - the stage golden's `Scene::Shopping` at 170 s and census purchases change;
  - `her_home_fills_up_over_visits` misses its SHOP_EVERY windows (tests.rs:2512-2514). The clock counts as "bought", and it's ambiguous whether it moves `bought_on`.
- **CATALOGUE membership is unclear.** The clock "appended to CATALOGUE" makes it sellable and breaks tests.rs:2317 ("she has it all"). D7 also contradicts itself: "appended" versus the window "after the cat bed, before the decor".
- Amendment:
  - the clock is in `Furniture::ALL` but not in `CATALOGUE`;
  - order it at `begin_visit` with `bought_on = visits`, so it is delivered on the next visit;
  - leave `bought_on` and SHOP_EVERY alone and exempt it from the shopping assertions;
  - gate it on the fed clock;
  - insert the window before `Plant`, and say it's inserted, not appended;
  - list step 8 as a golden re-record, and name tests.rs:2317 and 2470-2525.

**M5. Dash-ins: the design's "never skips one" claim is false for them.** (D3a; D3 Touch list, last bullet)
- `next_tick` in Absent and Away wakes only at slot boundaries. `tests.rs::run` (93-106) steps straight to `next_tick`, so it jumps from 08:15 to 12:45 and the dash time inside is skipped. At the 12:45 step the slot is no longer Away.
- The trigger isn't defined as level or edge. A level trigger ("game time ≥ dash time and still Away") fires again after a cold start, a restart via `exec_self`, or Away → Leaving → Absent → Away.
- Amendment:
  - Absent and Away `next_tick` = the minimum of the boundary, the dash time and the gate;
  - the dash fires only when one in-process accrual step crosses it (`from < t ≤ to`);
  - expose a pure `fn dash(seed, day) -> Option<u16>`, so tests can choose seeds with a dash, and add a stage `Scene::DashIn`;
  - tests: a cold start after the dash time doesn't dash; `run()` from 08:00 to 13:00 on a dash seed shows exactly one dash.

**M6. Proptest and sim costs.** (Tests, Protected cells and Sims) These are estimates; measure them.
- **The new protected-cells proptest rarely tests what it claims.**
  - A uniform `start in 0..WEEK` with a ~200 s case window reaches a departure in about 3% of cases and a dash in about 1%. At 32 cases, Away is usually never exercised.
  - Spanning a whole Away period takes 45 real minutes per case. The precedent is `every_made_piece_is_used_or_let_go`: about 13 s for 32 cases of a 90 s visit, and about 50 s at 256 (.config/nextest.toml).
  - Amendment: draw `start` from `prop_oneof` over (08:15 ± 3 min, 12:45 ± 3 min, bedtime ± 3 min, a forced dash, uniform). Keep the visible window at 200 s or less, and step Away segments without the 1000 ms clamp, since they paint nothing. Use `proptest_cases(16)` and keep cases under about 0.1 s in release.
- **The one-game-day sim is too slow for the gate.** It has about 110 real minutes of visible time, over 2 graphics modes, with text-dense hidden checks every frame. That is roughly 30 s per mode in the debug gate: past SLOW and near the 60 s kill.
  - Amendment: split it into windowed scenes using `skip_clock`: 07:00 to 08:30, 12:40 to 12:50, 20:00 to 20:10, and 22:25 to 22:40 with a chat line. Run the full day only in the ignored `day_census`.

**M7. The image budget misses where the dial and sky actually cost images.** (D7, Looks; Sims, `VISIT_IMAGES`)
- Pieces she overlaps go into her image (mod.rs:1062-1067). LookOut seats her under the window and the glance turns her toward the clock. So each dial quarter combines with each of her poses.
- A Monday 16:00–18:00 test hardly sees the dial, and sees one sky phase change at most.
- `CACHE_LIMIT` (1024) was sized on 2-hour visits (plan.md:2836-2849: 664 images by 120 min). A weekend resident's visit now runs about 145 awake real minutes plus the night.
- Amendment: the budget test runs a fed 17:30–19:30 visit, forces LookOut and two glances, and asserts the cache evicts nothing and encodes no image twice. Rerun `image_census` with a weekend resident day. If `Leaving` carries frozen `Looks`, count its frames too.

**M8. Body-rng and mind-stream draws the design doesn't close off.** (Determinism, "No new draws")
- **The departure.** `evict`'s `leaving_for_work` branch draws `SHIFT_MS` from the body rng and brings her back through the door after the gap (osaka.rs:4308-4322). `go_to_work` sets `at_work` and draws on the door path (4352-4362). A `routine/away` that reuses that exit must not set `at_work`. Test: a resident focuses a pane during the departure walk, and she still ends up Away.
- **Sleep-talk, Stir and Dream have no draw source.** There is one `mind.next()` per decision (osaka.rs:2940), and an 85-minute `Use` makes none. Specify `self.whims` with a label (`"sleep-talk"`) and salt = line index. The schedule should be a pure function of the act's start.
- **`furnish`.** Its only body-rng draw is `place_gift` (mod.rs:1696→1978). The real reasons it can't run in Away are the TV order (1680), parcel delivery (1684-1694) and arranging. Reword D3.
- **Blinks.** These draw only in `Stand` (osaka.rs:1692-1698). They are safe while Away paints no Osaka.

**M9. The hidden-goodbye fix and frozen `Looks` are unfed code paths.** (D3 Touch list; D7 Looks)
- Both change today's behaviour. In the errand golden, even seeds are visitors who leave on key presses from 70 s (golden.rs:279, 292), possibly mid-door.
- Amendment: land the fix as its own commit with a `HOUSEGUEST_GOLDEN_TRACE` diff, and re-record both tables if it moves them. Choose one of "`Plain` is sane" or "`Leaving` carries `Looks`" rather than writing "Better:".
- `Dissolve::new` needs `osaka.x` (mod.rs:679). Say what Away's rain uses instead.

**M10. The perf test sees the real date.** (D5; Steps 9)
- `houseguest_visit_cpu_is_negligible` (dessplay-rendezvous/tests/perf.rs:591-631) runs the real `run_ui_loop` with `Guest::new(rand::random())` (shell.rs:438). It is the only test that reads the real clock.
- So Feb 3, Jan 1–3 and Apr 8 change what it measures, and only on those days, and only under `--profile full --release`.
- Amendment: make the shell's stamp a pure `fn local_date(utc_ms)` with its own test. Give `run_ui_loop` an injectable date source, so perf passes a fixed date with no calendar entry.
- The return-type change compiles as is, since perf.rs:426 and :608 are statement calls. Say "no `#[must_use]`".

**M11. The tests as specified are vacuous, wrong or not buildable.** (Tests, Pure and cheap)
- **"Calendar: at most one entry a day" contradicts the starter table.** Dec 24–25 falls inside Dec 1–31, Feb 3 inside exams, Apr 8 inside Mar 1–Apr 30, and panic week inside summer vacation. Make it per kind: at most one greeting and at most one owed day-item per date. Boosts and pools may overlap. Exam chopsticks (once a visit) is outside `calendar_on`.
- **"Rares: at most one unseen" can't fail,** because the `Option` makes two unseen rares unrepresentable. Replace it with `new ∉ ledger.seen` and `open ⊆ ledger.seen`. Mutant: dropping the seen filter.
- **"Never decreases within a process" is false at `move_out`,** which resets her clock to Monday 16:00. Exclude it, and allow for backward `now`: `GameClock::at` and accrual must saturate.
- **"Exit paths" needs a recipe.** `run_ui_loop` uses the real `now_millis()` (shell.rs:707), and a whole-minute carry takes 10 real seconds. Recipe: set `ui.houseguest_moved_out`, fill the actions channel so the handout is parked in `ledger_unsent`, send `Shutdown`, and assert the result is `Some`. Repeat for `Quit` and `Disconnected`. Mutant: returning `ledger_to_save()`. Also state that `clock_rem` (under 1 game minute) is lost at each exit.

## MINOR

- **m1.** The landing order's numbers don't match the Steps list (landing item 1 = steps 1+2). Step 2 batches saves at "every routine slot change", but `routine.rs` arrives at step 3.
- **m2.** The `Needs` rate is evaluated at the decision's `at` for the whole gap since `decided`. Say so.
- **m3.** The `due()` boundary must be "the first cutting boundary after the act's start". If it's "after now", a catch-up past 64 wakeups (osaka.rs:1704-1706) loses it.
- **m4.** Gated rare splices must be skipped before `may`, the roll and `try_play` (script.rs:690-697). Append them with fresh salts.
- **m5.** The routine tape's "contiguous" needs a definition, since Asleep wraps midnight. Add "the slot at boundary `b` is the one `b` starts" and a minimum run length.
- **m6.** Missing tests:
  - `advance` returns `false` on accrual alone;
  - a client that never met her returns `None` on exit;
  - `skip_clock` only goes forward;
  - an Absent guest with `visits == 0` gets no boundary wakeup.
- **m7.** Add `docs/testing-strategy.md`'s "Golden Trajectories" section (lines 764-779) to "Docs that change": the unfed table, the fed scenes, and the re-record rule per step.
- **m8.** `Trace::frame` needs an `"away"` label (golden.rs:42-58), and the trace needs a `DayTime` in `Decision` so the explain log answers "why now" (map, mind trap 10).

## Left unspecified (a builder would guess)

- Exactly which consumers the unfed switch covers (B1).
- Whether `cue(Arrive)` at night tucks her in.
- Whether the errand at school counts as a visit. Today `send` calls `begin_visit` and counts it (mod.rs:1270), while D3a says a dash isn't counted.
- How `Dream` ("a splice … after 30 min asleep") can work, since splices play only before or after a use.
- Whether `touched` is set by accrual alone.
- Where Away's rain originates.
- Whether `idle_min` accrues while `persist == false`.
- Which mode each number recorded in plan.md was measured in.
