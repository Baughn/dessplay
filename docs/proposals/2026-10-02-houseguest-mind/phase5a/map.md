# Phase 5 (plan.md Phase 38, "phase 5 — Vignettes and the clock"): implementation map

**Paths.** All refs are under `dessplay/src/ui/houseguest/` unless a path is given. Where the reports say `tests/census.rs` and `tests/golden.rs`, they mean `dessplay/src/ui/houseguest/tests/census.rs` and `.../tests/golden.rs`. `dessplay/tests/` has neither file. Docs refs are `docs/…`. "HG" is `docs/proposals/2026-09-28-houseguest.md`. "Proposal" is `docs/proposals/2026-10-02-houseguest-mind.md`.

**What was checked.** I re-read these claims in the code:
- `choose` multiplies by `factor` before `truncate(TOP)`.
- The refutable `for &Factor::InChat` loop.
- The draw order in `start_job`.
- `credit_done`'s early return.
- `watching()`, `Looks.tv` and `piece_state`.
- The golden frame hash.
- The census `group` fallthrough.
- `Lines::pick`'s fixed labels.
- `use_look`'s Watch arm.
- design.md's commit rule.

Anything marked **(proposed)** is a name or site I am suggesting. It does not exist in the code yet.

**Plan state.** docs/plan.md:2108 says "phases 0–4 done… phase 5 next". There is no phase-5 section yet; Phase 38 ends at plan.md:2695. plan.md:2317-2319 asks for "Where was I?" to join a pool.

---

## (a) Seams and exact sites to change

### A1. The keyframe player
- **Nothing exists yet.** No `Script`, `Key`, `Splice`, `Adverb`, `Tier`, `LocalTime`, `PropOverride` or `Line` type exists. `scenes.rs:131-318/323-400` holds text jobs and `LayerOp`s, not keyframes.
- **The one table-driven player is the door.** `DoorBeat`/`DOOR`/`door_beat`/`DOOR_THROUGH_MS` are at osaka.rs:731-846. `door_beat(elapsed, gap)` turns cumulative beat `ms` into the current beat plus its end. Its users:
  - `fire`'s Door arm (osaka.rs:1705-1720)
  - `hidden()`/`door()` (3769-3787)
  - `acting` (4148)

  Generalise it into the Script player. Don't write a second one.
- **Timing path.**
  - `due()` (1516) takes the minimum of `pose_due` (1541), `pending_due` (1555) and speech.
  - `tick` (1587-1627) loops up to 64 times, then runs `debug_assert!(undoes_its_mischief())` (1569/1621-1623).
  - `set(act, at)` (1629-1638) calls `credit_done` and then `first_due` (1466-1514).
  - `first_due` must return **cumulative key starts** for a script, not `USE_FRAME_MS` multiples. Otherwise a commit or op at a key boundary lands late.
  - The Use re-arm is at osaka.rs:1786: `since + (elapsed/USE_FRAME_MS + 1)*USE_FRAME_MS`.
- **`Key` needs an animation period** (a field, or a pose that carries its own frame rule). Eat, Pet, Homework and Lift all bob on `elapsed/PERIOD % 2`, and the proposal's `Key` (proposal:296-307) has no period.

### A2. Shopping channel → a Script (removes `Act::Use.advert`)

Every read and write of the advert:

| Site | Role |
|---|---|
| osaka.rs:33 | `Chances.advert` |
| osaka.rs:487 | `Act::Use.advert` field (variant 480-488) |
| osaka.rs:2272-2278, 2296 | chosen, `HomeEvent::Bought` pushed, stored; **before** `rng.range` at 2282 |
| osaka.rs:950-1022 | `use_look`; the Watch arm is at 983-989; the sofa pose `watching` is at 959-961 (**corrected** from the proposal's "osaka.rs:2765-2775") |
| osaka.rs:2591-2601 | `watching() -> Option<(since, advert)>` |
| osaka.rs:4116-4129 | `acting` passes advert to `use_look` |
| osaka.rs:4357 | test fixture `advert: None` |
| mod.rs:81-95 | `advert()` (`SHOP_EVERY`=3 at 67; `CATALOGUE`/`FIRST_TV_VISIT` at 55-69) |
| mod.rs:964, 999 | fill `Chances.advert`: stage-cue `offered` (953-970) and live `visit.chances` (988-1004) |
| mod.rs:1026-1033 | `Looks.tv` from `watching()` with `CHANNEL_FRAME_MS`=400 (71) (**corrected** from the proposal's "mod.rs:851") |
| mod.rs:2241-2254 | `struct Looks { tv, states }`, the natural PropOverride sink |
| mod.rs:2160-2183 | `piece_look` TV → `Look::Tv` |
| mod.rs:2257-2262, 2471-2475 | line-art paths pass `looks.tv` |
| mod.rs:2331-2337 | text-mode screen glyphs per channel |
| mod.rs:1303-1308 | `record`: `Bought` sets `ordered` and `bought_on`, clears `shop_now` |
| mod.rs:353-355, 438-442, 466-473 | stage `shop_now`/`shop()`/`cue(Scene::Shopping)` |
| stage.rs:177 | `Scene::Shopping` → `Use::Watch` |
| art.rs:959-964, 968, 1014-1041 | `enum Channel { Snow, Shopping }`, `snow()`, `tv_scene` |
| tests.rs:2130, 2144-2172, 2190-2237, 4219-4250, 5655-5700 | `advert()` units; `a_bare_room_has_her_buy_decor` (2193); `commits_survive_the_visit_ending`; `a_grievance_wins_over_the_shopping_channel` (5661) |

### A3. PropOverride (TV channel, fridge, lamp, cat bite)
- **(proposed)** Add `Osaka::prop_override(now)`, pure in `now`. mod.rs then only consumes it:
  - `Looks.tv` (mod.rs:1026-1033)
  - `piece_state` (mod.rs:2196-2222)
- This removes the exported `osaka::FRIDGE_OPEN_MS` and `bite_at`. Their users are the Pet look (995-996, 911) and the snack's fridge (992-993).
- `PieceState::Cat` comes from `cat || cat_home(&ledger)` (mod.rs:2188-2196). That is home state, not Osaka's, so it stays where it is. PropOverride replaces only `LampOff`, `FridgeOpen`, `CatBiting` and the TV channel.
- A new channel needs three things:
  - an `art::Channel` arm (art.rs:959-964)
  - a `tv_scene` picture (art.rs:1014-1041)
  - the ASCII two-glyph screen (mod.rs:2331-2337)

### A4. The Act shape, and every `Act::Use`-keyed match
**Exhaustive matches.** A new variant won't compile until it has an arm here:
- `props` (697-723)
- `first_due` (1466-1514)
- `fire` (1640-2209); its only `_` (2121) is nested
- `acting` (4096-4225)

**Non-exhaustive matches keyed on `Act::Use`.** These miss a new variant silently:
- `at_job` (673-685) and `job` (688-695). Used by `recheck`'s `Stays::Job` spot (2563/2577), `hands_row` (3752) and `act_summary` (1421-1440).
- `credit_done` (2390-2425): `_ => return`, plus the `restful` beauty credit.
- `use_span` (2581): feeds mod.rs `piece_state` and census `doing` (census.rs:126-129).
- `seat` (2527/2528): feeds `draw_art`'s inside-piece layering (mod.rs:2458, 2463-2481).
- `lost_seat` (2536-2543): without an arm, `Cause::SeatGone` never fires.
- `watching` (2591).
- `grumbling` (3480-3483).
- Test-only `using`/`grievance` (2612, 2621) and `matches!(…Act::Use…)` (4422).
- Harmless: `hidden`/`door`/`reeling`/`lifting`/`settle` use `_`.

**Recommended hybrid** (the user must confirm; see f1):
- **During a use**, for example the shopping channel and later channel-surfing or rabbit ears:
  - **(proposed)** a script rides on `Act::Use` as `playing: Option<Playing { script, branch, durations… }>`, replacing `advert`.
  - All of the queries above stay correct, `act_name` stays `"Use"`, and the proposal's `Key.pose: None` = "the seat's own look" maps directly onto `use_look`'s `watching` pose.
- **Before or after a use** (chopsticks before homework, andagi after a snack) or free-standing:
  - **(proposed)** a separate `Act::Script { seat: Option<Seat>, script, since, branch, durations, then: Then }`.
  - This is the 11+-site job above, plus mod.rs:1026, 2203 and 2458, and census `group`.

### A5. Splice hooks
- **Before-splice (chopsticks #40 before homework or a snack).**
  - Hook in `start_job`'s `Job::Use` arm (osaka.rs:2244), before the `Act::Use` is built.
  - The splice decision must be made in `choose_next`, after `offers.remove(i)` and before `self.plan(...)` (osaka.rs:2863-2868). It then rides the payload:
    - `Then::Job` (osaka.rs:507-515; built at 690, 3520; consumed at 2010-2016)
    - `Heading` (mind.rs:652-655) for another floor. `Heading::find` (mind.rs:668-723) rebuilds the job, so the splice must live on the heading, not on the job.
    - The leftover/arrange continuations (osaka.rs:2797, 3341).
- **After-splice (sata andagi #41 after a snack).**
  - Hook in `fire`'s Use arm at `at >= until` (osaka.rs:1765-1785), before `decide`.
  - The Crumple → `Admire` chain at 1773-1780 is the precedent: `return self.set(Act::Admire{..}, at)`.
- **Grievance.** Already "during a use": `Act::Use.grievance: Option<(Grievance, u64)>`, felt at 1747-1764, shown in `use_look` at 962-967, timed by `grievance_from` (941-944). Leave it as it is.
- **Adverb.** Hook in `plan` (osaka.rs:2889-2950), where a `Bind` becomes an `Act`. The adverb is picked by a whim label in `choose_next` and passed in.
- **Explain log.** `Decision` (osaka.rs:610-623, `Display` 643-668) gains **(proposed)** `splice: Option<&'static str>`.

### A6. Mind: tiers, factors, rarity, pity
- **Today.**
  - `DesireDef { base, serves, own_sake, factors }` (brain.rs:391-399), with rows from `Want::def` (452-516). There is no `tier`.
  - `Factor` has only `InChat` (brain.rs:381-386).
  - The factor closure in `choose_next` is at osaka.rs:2839-2858; its `match` is at 2852-2856.
  - The offer filter is at osaka.rs:2812-2817.
  - `Ctx` is at mind.rs:76-97. It has no clock, tier or pity input.
- **Tier is not a factor.**
  - `choose` (brain.rs:565-599) scores `score × factor` **then** does `truncate(TOP=4)` (576-579) and then rolls `whims.below_at("roll", attempt, 1000)` (585).
  - At home about 15-20 wants bind. A dampener below 1 doesn't make a want rare; it **removes** the want whenever four others outscore it.
  - So proposal:536 ("Tier is a factor") does not hold against this code. Rarity and pity must gate **offering**, in one of two places:
    - **Option A:** the offer filter (2812-2817).
    - **Option B:** `mind::bind` (mind.rs:227). This keeps "offered iff a method binds" true for offering and planning alike.
  - Clock and season **boosts** (>1) work as factors in the closure.
- **The roll source.** Use the visit seed plus a new salt, like `Mood::of` (brain.rs:131-141, mod.rs:1102-1104) and `cat_home` (mod.rs:2188-2194). Alternatively, hash it as a `Whims` label on the existing per-decision draw. **Never** use a new `self.mind.next()` or a body `rng` draw.
- **At most one unseen rare a visit.** Make it structural: **(proposed)** one slot drawn at `begin_visit` (mod.rs:1098), or a flag on `Osaka` beside `worked`/`home_acts`, set in `plan` on success (near osaka.rs:2868-2884).

### A7. LocalTime
- **The clock today.**
  - Monotonic `now_millis()` (shell.rs:707-713; the no-wall-clock doc comment is at 700-706; design.md:1856).
  - `draw()` (shell.rs:623-638) calls `ui.idle_view(...)` and then `guest.paint`.
- **Attach the date in the shell after `idle_view`**, at shell.rs:635:
  - The `Ui` must not read a clock, because `real_frame` (tests.rs:616-627) goes through `idle_view` (app.rs:585-655).
  - Cache it per minute, because `Local::now()` resolves the timezone on every call.
- **The field.** `IdleView` (idle.rs:45-75) gains **(proposed)** `pub local: Option<LocalTime>`, where `None` means no calendar.
  - It can't be a plain derived `Default`: chrono's `Weekday` has no `Default`, and `NaiveDate::default()` is 1970-01-01 (New Year's Day).
  - Only two literals list every field: app.rs:637 and tests.rs:20-33 (`fn view`). Every other literal uses `..view(..)` or `..Default::default()`, including tests.rs, golden.rs:175/242/277/302 and mod.rs `whole_glyphs`/`gate`.
- **Flow.**
  - Latch it in `observe` (mod.rs:1253-1259), as `delay` is.
  - Pass it on through both `Chances` literals (mod.rs:953-970, 988-1004) into `Ctx` (mind.rs:76).
- **Callers that pass a view:** shell.rs:635, examples/houseguest.rs:102, tests.rs:616-627 and the app.rs:5358-5363 test.
- **Calendar dates.** Existing helpers: `timeutil::biblical_date` (timeutil.rs:10-14; the day starts at 09:00) and `NaiveDate` parsing of `"YYYY-MM-DD"` (changelog.rs:24).

### A8. Ledger (pity counters, seen rares, calendar date)
- **Today.**
  - `Ledger` (ledger.rs:31-43) has `master_seed`, `visits`, `home`, `ordered` and `bought_on`.
  - `VERSION = 1` (25), `KEY` (28).
  - `Raw` (284-303) reads leniently: `#[serde(default)]`, and entries are read as `Vec<Value>` and filtered.
  - `Saved` (257-271) writes it. Copy the `unsettled` pattern: `skip_serializing_if = "Vec::is_empty"` (269).
- **Where to accrue counters.**
  - Present minutes: the `Visiting` arm of `advance` (mod.rs:659-674).
  - Client-idle minutes: the `Absent` arm (mod.rs:651-657), from `quiet_since` (mod.rs:331).
  - Keep the sub-minute remainder in a non-persisted `Guest` field. Move whole minutes into the ledger at coarse points.
- **Saving.** Every dirtying costs one sqlite write: `unsaved`, then `ledger_to_save()` (shell.rs:453-465), then `SaveHouseguest` (run.rs:966-970, 1786).
- **`move_out`** resets everything (mod.rs:407-418).

### A9. Line pools
- **Today.**
  - `Lines { said }` (mind.rs:612-643), with `LINE_COOLDOWN_MS` = 10 min per string (607) and `LINE_BUDGET` = 8 per visit shared across all pools (609).
  - Labels are fixed: `chance("line", 0, n, d)` and `below("which-line", …)`.
  - Callers: beats (osaka.rs:2707-2727, via `Loss::says` at mind.rs:583-592) and `AH_RIGHT` (osaka.rs:3020).
- **Direct `self.say` calls with no cooldown:** osaka.rs:1531 (musings, a body `rng.below` at 1527-1535), 1717 (`THROUGH`, const at 897), 1849, 1884, 2255, 2308, 2693, 3258/3293/3382, 3936 and 4074.
- **Bubbles.** `Bubble` (1087-1121) is `Copy` and holds `Say(&'static str)`. `speech` is `&'static str` too (1522; `speech_ms` at 344).

### A10. Art (chopsticks #40, sata andagi #41)
- **ASCII sprites.** `Pose` (sprite.rs:18-75), `Face` (79-100), rows (102-202), `cells` (301-347). The hand-kept `const ALL: [Pose; 41]` is at sprite.rs:355.
- **Line art.**
  - `Rig::for_pose` (art.rs:136-313) is exhaustive.
  - `Expression` (art.rs:38-62) needs `face-*` and `p-face-*` SVG parts.
  - `Rig.hold` (101-105).
  - Eat hard-codes melon bread (art.rs:516, 528-534).
- **Image cache key.** `graphics::Look::Pose(Pose, Face)` (graphics.rs:37-55), so food must live in the `Pose` value. The cache cap is 256 (graphics.rs:33).
- **Review sheets.** `model_sheet` (art.rs:1531), `poses()` (1097), `uses()` (1693), coverage tests (1337).
- **#40 needs:**
  - chopsticks parts (joined, clean split, bad split)
  - a new pose or a `hold` variant
  - a droop face: `Face` + ASCII glyphs + `Expression` + two SVG parts
  - the sparkle can only be a bubble glyph; effect sprites are phase 7
- **#41 needs:** Eat to take a food (`Eat{frame, food}` or a new pose), an andagi part plus a bitten frame, and 5-7 "Sata andagi." keys going Vacant → Pleased → Happy.

### A11. Stage, census and lints
- **Stage.** `Scene::ALL: [Scene; 32]` (stage.rs:97), `name()` (133), `furniture()` (171), `direct()` (236/239), `Guest::cue` (mod.rs:458).
- **Every-scene test.** `every_scene_has_a_spot_in_the_stage_room` (tests.rs:1360). Its `happened` match is at 1424-1465. Its "116" comment is stale; it is 128 runs.
- **Census.** `group()` (census.rs:267-281) ends in `_ => "standing"`. `Visit` (census.rs:86-123). `simulate` (144-237) calls `shop()` on every visit (157). `said` is at 162-179.

---

## (b) What must be reused, not duplicated

| New thing | Reuse |
|---|---|
| Script player | `door_beat` and `DOOR` (osaka.rs:731-846): cumulative-ms beats, current beat plus end |
| Key ops | `pending` with a `Restore` scheduled in the same step; Sneeze (1953-1979, 2491) and Swap (1902-1952) are the templates |
| During-use keys | `use_look` (950-1022) and `Act::Use { since, until, whole }`; `pose: None` is the existing `watching` pose |
| Draw-at-start | `start_job` (2244-2300) already fixes grievance, advert, length and whole up front; branch and durations go beside them |
| PropOverride sink | `Looks` (mod.rs:2241-2254) and `piece_state` (2196-2222) |
| Line pools | `mind::Lines::pick` (mind.rs:613-643), generalised with a pool id or salt; don't write a second cooldown store |
| Deterministic choice | `Whims` (mind.rs:31-71); the `chance` signature is `chance(label, salt, n, d)` (corrected from the proposal's `chance(label, n, d)`) |
| Per-visit rolls | `visit_seed(v)` (ledger.rs:59-61) plus a salt, as `Mood::of` and `cat_home` do |
| Per-visit caps | `Mood::home_acts` (brain.rs:164-170) |
| Owed beats | `Beat { loss, toward }` (mind.rs:601-604), `owe()` with `OWED`=3 (osaka.rs:123, 3119-3128). Calendar "owed once" needs a variant with no spot, because `Beat` requires `toward` |
| Interrupts | `interrupt(Cause)` (3628-3644) and `ActProps { stays, on_chat }` (521-536). "Passing" is `Stays::Pass`, not a third field (corrected from the proposal at proposal:281 and the scenes report). Key `hold` maps onto `Stays`/`OnChat` |
| Ledger fields | the `unsettled` lenient-read and skip-empty pattern (ledger.rs:85-89, 269) |
| Dates | `"YYYY-MM-DD"` via `NaiveDate` (changelog.rs:24), or days since the epoch as `i32`. Not serde `NaiveDate`: chrono's `serde` feature only comes through librqbit-dht |
| Multi-visit tests | `one_visit` (tests.rs:2295-2312) and `her_home_fills_up_over_visits` (2318-, a ledger round-trip) |
| Image-budget test shape | tests.rs:6547/6609 (`a_carry_stays_within_the_image_budget`) |

---

## (c) Invariants and lints a builder must respect

1. **The crate lints.** lib.rs:6-11 denies `missing_docs`, `unwrap_used`, `expect_used`, `panic`, `todo` and `dbg_macro`. `stage` is a `pub mod`, so `LocalTime` and its fields need doc comments. Tests opt out (osaka.rs:4292). The stop hook runs `clippy -D warnings`.
2. **`appearance(now)` stays pure.** `appearance` (4085-4094) and `acting` are called several times a frame (mod.rs:1381, 1427, 2420-2421). Branch and key durations are drawn at start and stored in the act.
3. **The body-stream draw count.**
   - Any added or removed `rng.range`/`rng.below` shifts every later duration and every golden hash.
   - Splice, adverb, rarity and pool picks use `Whims` labels or a visit-seed salt.
   - `Whims(self.mind.next())` at osaka.rs:2661 is the only use of the mind stream; keep it that way.
   - A label repeated within one decision gives the same answer. That is deliberate: `place` is shared by mind.rs:421-451. Choose new labels on purpose.
4. **Hard rule for the refactor-first shopping step: no extra draws.**
   - The proposal's `Key.ms: (u32, u32)` needs a draw per key, so it can't be used here.
   - Watch keys must take their spans from the Use's `length`: the hook below 2/5, the pitch below 3/5, then plain watching (osaka.rs:983-989). That needs **(proposed)** a share-of-length form such as `Span::Share(n, d)`, or durations computed from `length` and stored.
   - The `Bought` push must stay in the same tick, before `rng.range` (2275-2282).
   - The TV must show `Shopping` for the **whole** watch, as it does today, because `visit.image` Debug is hashed.
5. **The mischief lint.** `undoes_its_mischief` (osaka.rs:1559-1570), checked by `debug_assert!` after every `fire`. A key `op` goes through `pending` as Swap/Knock with its `Restore` scheduled in the same step.
6. **Speech overrides bubbles.** `speech.or(bubble)` (4093); only `grumbling` (3480) is exempt, and only for Use.
   - A key's line must be the key's own bubble, not `say()`, or it outlives the key and hides later keys.
   - Pending speech ("Where was I?" at 1717) hides key bubbles. Wait for quiet, as `grievance_from` (942) does.
7. **Bubble length is 24 characters at most.** Today that is checked by four hand-kept lists:
   - `osaka::LINES` (osaka.rs:316, `cfg(test)`)
   - `every_fixed_line_fits_a_bubble` (tests.rs:4664-4677)
   - greetings in `moods_come_in_their_shares` (brain.rs:853)
   - `beat_lines_fit_a_bubble` (mind.rs:769-786)

   Inline literals such as `"Ow!"` (osaka.rs:997) are unchecked. "Somebody said something." (osaka.rs:905) is exactly 24. Script keys must be linted by iterating the script table (see e2).
8. **Key hold.** A key longer than 2 s declares `hold`. That needs a new lint walking `Script.keys`, because `Act::props` classifies a whole variant, not keys.
9. **Cueing.** Every want and script can be cued: `every_want_can_be_cued` (tests.rs:4684) is exhaustive over `Want`, and scripts need a parallel lint. Bump `Scene::ALL` (stage.rs:97).
10. **The ledger.**
    - `VERSION` stays 1.
    - New fields default to what `new()` sets (`missing_fields_default`, ledger.rs:563-567).
    - Skip them when zero or empty, or deliberately re-pin `the_record_as_written` (ledger.rs:372-387).
    - `a_ledger_round_trips` and `another_version_is_not_read` must keep passing.
    - Seen sets are keyed by a stable string `ScriptId`, never an index or discriminant.
    - Use no `HashMap` iteration; use `BTreeMap` or `Vec`.
11. **The wall clock is calendar-only.** It is never used for durations or animation. Persist accumulated deltas, never an absolute `now`, because `now` restarts at 0 per process and the stage starts at 0 (examples/houseguest.rs:92-98). A clock tick must not force a paint (design.md:1856-1858). The rule goes in design.md:1856 and the reason in decisions.md.
12. **Proptest counts.** Pinned counts go through `proptest_cases(N)` (test_support.rs:43). nextest flags tests at 30 s and kills them at 60 s (.config/nextest.toml:14).
13. **Image budget.**
    - `a_busy_furnished_home_stays_cheap` (tests.rs:1846/1857) allows at most 4 re-encodes.
    - These require 0: tests.rs:1815, 6021 and 6609.
    - Each new (Pose, Face) pair and each PropOverride state is a new cached image (graphics.rs:118, 361).

---

## (d) Traps and risks

1. **Compile trap.** `every_want_has_a_sane_row` (brain.rs:811) loops with `for &Factor::InChat(times) in def.factors`. It becomes a refutable-pattern error as soon as `Factor` has a second variant. Rewrite it as a `match`. Do the same for the closure at osaka.rs:2852-2856.
2. **A rare-dampening factor deletes the want.** See A6.
3. **A before-splice can loop.** If the before-splice decision rides on a `Job`/`Then`/`Heading` and the script's end re-enters `start_job` with the same payload, she splices again. The splice must be consumed (cleared) when it starts. No report raised this.
4. **Credit under splices.**
   - `set()` calls `credit_done` first. An after-splice `set` while the act is still `Use` credits the Use in full.
   - A before-splice set while she is walking leaves `self.credit` pending through the script. `credit_done` returns early on a non-matching act (osaka.rs:2390-2410, `_ => return`), and the Use credits at its own end.
   - An interrupt **during** a before-splice loses the wrapped want's credit, as an interrupted walk does today. Say whether that is acceptable.
   - Anchors that must keep recording the wrapped want: `recent.push` (2871), `credit = Some(want)` (2881), `credit_done` (2663).
5. **Exits that bypass `interrupt`.** `place` (3565-3579), `settle`'s Fall and Dazed arms (3700-3740), `muse` (1527) and any `decide`. A `Script.on_interrupt: Resume` must cover them, or resume state leaks.
6. **Arrival has no whims.** `start_job` runs in the tick with only the body `rng`. A multi-branch script's branch must be either:
   - carried from the decision on `Then::Job`/`Heading`, surviving `Heading::find` (mind.rs:668); or
   - hashed from the visit seed plus a salt.

   Recommendation: carry the splice id and branch from the decision, so the explain log shows it. Use the visit-seed hash only for scripts started outside a decision (calendar, door). The shopping script has one branch, so the refactor step needs neither.
7. **`Lines::pick` shares its roll.** Its labels are fixed (`"line"`, 0 / `"which-line"`), so two picks in one decision get the same roll. That is safe today only because Owed returns early (2707) and `leftover` returns at the first seat (3017-3024). A single `LINE_BUDGET` covers all pools, so vignette pools would use up the beat budget.
8. **"Where was I?" is said from the tick.** `fire`'s Door arm (osaka.rs:1717) has no whims. Pooling it means either:
   - queueing it as an owed beat; or
   - drawing from the body stream, which re-records the goldens and `osaka_at_home_seed_7`.
9. **Musings duplicate catalogue lines.** `MUSINGS` (osaka.rs:321-334) already holds the one-liners for #41, #45, #52, #54, #55, #57 and #74. A script for #41 plus the musing means she says "Sata andagi!" from two places. Remove those musings or share one cooldown. Moving musings to whims removes a body draw (1529) and re-records the goldens and seed 7.
10. **The census misclassifies silently.**
    - A free-standing `Act::Script` name falls into `"standing"`.
    - `doing()` keys furniture time on `use_span()`, so a Script that isn't a Use drops the furniture %.
    - Make `group` exhaustive, or classify by seat, in the same step that adds the act.
11. **What the goldens hash.** Each frame hashes `act_name()` (osaka.rs:1410-1417: the Debug prefix of the act), x, y, facing, `appearance(now)` Debug, `visit.image` Debug and changed cells. `finish` (golden.rs:83) hashes `ledger.to_json()`. The tables are pinned at golden.rs:341-386 and re-recorded via the table that `check` (323) prints; `HOUSEGUEST_GOLDEN_TRACE=<dir>` writes per-frame traces.
    - Only `golden_stage_room` cues Shopping (golden.rs:167, at 170 s), because `SHOP_EVERY`=3.
    - Ledger counters that go nonzero move `finish` hashes in every scene where they accrue, including the errand golden's even seeds.
    - `local: None` must disable every calendar path.
12. **The seed-7 snapshot** (tests.rs:696, the snapshots/ dir) moves if her first 95 s change: a greeting script, arrival calendar content, or musings.
13. **Downgrades.** An older build, for example one on the `stable` track, re-saves the ledger without the new fields. Pity and seen sets are then lost. Acceptable, but document it in decisions.md.
14. **Quit drops the last change.** The Quit path returns (shell.rs:588-593) before the next `ledger_to_save()` drain, so a counter flushed in that last iteration is lost. Drain once on exit.
15. **Per-minute accrual means per-minute sqlite writes.** Batch the flushes.
16. **Lines in the proposal that are too long:** #40 "Ya gotta hold 'em by the ends!" (30) and #54 "The box one's the escalator. ...No?" (35). #53's scary-story line (37) is not phase 5.
17. **HG numbering.** HG uses #30 twice (HG:422 Remarks, HG:437 Waving).
18. **Proposal and code drift.**
    - `Method.guard` is `fn(&Ctx, Whims, Want)` (mind.rs:157-164).
    - `Bind` is an enum (mind.rs:112).
    - There is no `brain::Kind`; the table is `DesireDef`.
    - `RuleRow { grievance: ScriptId }` (proposal:426) doesn't match the code, where a grievance is `(Grievance, u64)` on the Use.
19. **Bubbles that don't fit aren't drawn** (`bubble_spot`, mod.rs:1470-1544, design.md:1502-1510). Pool cooldowns should count a line as said whether or not it showed, as grievances already do (design.md:1699-1702).
20. **The school-day absence contradicts design.md.** HG:276-288 has her absent 08:30-15:30 on school days. design.md has no time-of-day rule. Against it:
    - the idle delay alone brings her (D:1339-1356)
    - a resident stays (D:1791-1800)
    - the errand fetches her whatever the idle gate says (D:1812-1838)
    - settings offer only a delay or Off (D:1839-1842)
    - proposal:532-534 limits `LocalTime` to the calendar

    This blocks the routine row and #46/#47/#42 only.

---

## (e) Proposed step order

Each step is one or more commits. Run clippy and `cargo nextest run` before each.

**1. Script types and player; shopping channel as a Script.** Refactor only. About 250 lines.
- **(proposed)** Add `Script`/`Key` with a period and a share-of-length span. Generalise `door_beat` as the player and move `DOOR` onto it, or at least make them share the player.
- Replace `Act::Use.advert` with `playing: Option<Playing>`.
- `use_look`'s Watch arm reads the keys. The `Bought` push stays at 2275-2278, before `rng.range`.
- **(proposed)** Add `prop_override(now)`, which replaces `watching()` for `Looks.tv` (mod.rs:1026-1033) and the osaka-derived states in `piece_state`. Drop the exported `FRIDGE_OPEN_MS`/`bite_at`.
- Adjust the tests that read `watching()`: tests.rs:2193, 5661 and 4219-4250. The `advert()` units stay.
- **Goldens unchanged.** The seed-7 snapshot is unchanged.

**2. Lines.** About 150 lines.
- **(proposed)** Add a `Line { Fixed, Pool }` enum and one `all_lines()` lint that replaces the four hand-kept lists, plus a script-table walk and the "Ow!" literal.
- Give `Lines::pick` a pool id or salt.
- Decide the budget per pool (f5).
- Add the ">2 s keys declare hold" lint.
- **Goldens unchanged** as long as the existing beat picks keep their labels: use salt 0 for the existing pool.

**3. Mind plumbing.** About 100 lines.
- Rewrite brain.rs:811 and osaka.rs:2852-2856 as `match`.
- Add `DesireDef.tier`, and `Factor::{Clock, Season}` with no row using them yet.
- Add a `tier` sanity check in `every_want_has_a_sane_row`.
- **Goldens unchanged.**

**4. LocalTime.** About 120 lines.
- Add `Option<LocalTime>` to `IdleView` (update the literals at app.rs:637 and tests.rs:20). Set it in shell.rs `draw()` after `idle_view`, cached per minute.
- Latch it in `observe`, then pass it into both `Chances` literals and `Ctx`.
- Add a stage key to set and step the date (examples/houseguest.rs).
- Docs: the rule in design.md:1856, the reason in decisions.md.
- **Goldens unchanged** (`None` everywhere).

**5. Ledger fields.** Its own commit; about 120 lines.
- Idle-minute counters (definition per f4), a seen-scripts set keyed by `ScriptId` string, and the last calendar date.
- Read leniently, skip when empty, deliberately re-pin `the_record_as_written`.
- Drain the ledger on Quit.
- **Goldens:** `finish` hashes move wherever a counter accrues, likely every scene that runs a minute or more. Re-record with the reason in the commit message.

**6. Rarity and pity gate.** About 150 lines.
- A gate at the offer filter (2812-2817) or in `mind::bind` (227), rolled from the visit seed plus a salt. Pity comes from ledger counters through `Ctx`.
- One unseen-rare slot drawn at `begin_visit` (mod.rs:1098).
- Tests:
  - a pity-bound property over synthetic ledgers and seeds;
  - an every-rare-owed adversarial property at `proptest_cases(8)`;
  - counters advance with monotonic minutes, never with `LocalTime`.
- **Goldens:** unchanged only if no existing want gets a tier below Common. Otherwise re-record.

**7. Free-standing script act and splices.** About 250 lines.
- **(proposed)** `Act::Script { seat, script, since, branch, durations, then }`. Give it arms in every exhaustive match and in each non-exhaustive Use-keyed site listed in A4.
- A before-splice decision in `choose_next` (2863-2868) riding `Then::Job`/`Heading`, consumed at start.
- An after-splice in `fire` at 1765-1785.
- `Decision.splice`.
- Census `group` made exhaustive.
- Stage `Scene` entries, a `Scene::ALL` bump, `name`/`furniture`/`direct`/`cue`, and the `every_scene_has_a_spot` predicate set to "the script started".
- A "Where was I?" pool (per f7) and a musings cooldown or merge.
- **Goldens:** unchanged until a splice row has a nonzero chance; then re-record.

**8. Art and content: #40 and #41.** About 150 lines plus art.
- Chopsticks parts and a droop face: `Face`, ASCII glyphs, `Expression`, `face-*`/`p-face-*`, `sprite::ALL`, `Rig::for_pose`, the gallery `poses()`/`uses()`.
- Eat takes a food (andagi plus a bitten frame).
- **Show the user the model sheet before wiring the art in** (plan.md:2100-2103 rule).
- Add an image-budget test for each vignette.
- **Goldens:** re-record.

**9. Calendar content.** About 150 lines plus art.
- A pure `fn(NaiveDate, Weekday) -> Option<Day>` plus `owed(ledger, local)`.
- Tests: a year test over one leap and one non-leap year (fixed entries fire; floating entries resolve once; no clashes; lines fit), and a Guest-level test that two visits on one date play it once, in both drawing modes.
- Census rows: scripts, splices, tiers, unseen rares, and a `said_at` timeline. Add a gated test of at least 10 minutes that no pooled line repeats within 10 minutes (about 2 seeds × 3 rooms × 12-15 min, kept under 30 s).
- Update plan.md (a Phase 38 phase-5 section), design.md, decisions.md and CHANGELOG.md (user-visible).

---

## (f) Open questions for the user

1. **Act shape:** a script on `Act::Use` for during-use content plus a free-standing `Act::Script` for splices (the hybrid), or one `Act::Script` for everything? The hybrid keeps about 11 Use-keyed sites correct. Blocks steps 1 and 7.
2. **When the purchase commits:** "at key 1, when the channel comes on" (proposal:314) or the current "the moment it comes on, whatever happens next" (design.md:1542-1547; test tests.rs:2193)? If there is a key before the channel, an interrupt there means no purchase. Blocks a behaviour change after step 1.
3. **Her "day":** the civil date, or the client's biblical day (09:00 start; timeutil.rs:10-14)? Blocks steps 4, 5 and 9.
4. **"Idle minutes" for pity:** minutes she is present (the `Visiting` arm) or minutes the client is idle (the `Absent` arm)? Blocks steps 5 and 6.
5. **Line budget:** one shared `LINE_BUDGET` of 8, or one per pool? Blocks step 2.
6. **Cooldown span:** does the 10-minute cooldown span visits? If so, `Lines` moves to `Guest`. Blocks step 2.
7. **"Where was I?" pool:** an owed beat, or a body-stream draw (which re-records the goldens)? Blocks step 7.
8. **Musings:** remove the musings that duplicate catalogue scripts (#41, #45, #52, #54, #55, #57, #74), or share one cooldown with them? Blocks step 7.
9. **School-day absence (HG:276-288) vs design.md (D:1339-1356, 1791-1800, 1812-1842):** drop it so the clock only weights what she does, or define school days, breaks and weekends? Blocks #42, #46, #47 and the routine row.
10. **Over-long lines:** shorten #40 (for example "Hold 'em by the ends!"), and shorten or split #54 over keys.
11. **#41 "every question":** where do the questions come from, a line pool or chat lines ending in "?" (screen reading)?
12. **The 2 s hold rule:** confirm "a key longer than 2 s must declare Stay or Pass" applies per key, not per script.
13. **Scope:** is the rest of the calendar content (omikuji, Feb 3, Apr 8, Jul 7, Oct 31, Dec 24-25) part of phase 5, or only the mechanism plus a few dates? Oct 31 is 28 days away. Sep 30 has passed this year.