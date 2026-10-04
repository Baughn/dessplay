# Phase 5b (the clock) — implementation design

**Working design, 2026-10-04.** Brief: docs/plan.md, Phase 38, "Phase 5 — vignettes and the clock (brief)"
and "For 5b's brief". Proposal: docs/proposals/2026-10-02-houseguest-mind.md (*Character*: the wall clock,
rarity and pity; migration row 5). Code map with file:line refs at HEAD `0661710`:
[phase5b/map.md](phase5b/map.md). Its last section ("Gaps and corrections") overrides the sections above it.
Paths are relative to `dessplay/src/ui/houseguest/`. HG is docs/proposals/2026-09-28-houseguest.md.

## User decisions

From the phase-5 brief (2026-10-03):
- A **game clock** that runs only while dessplay is open, at about **6×** real time, persisted, starting
  at **16:00 on a weekday** for a new ledger and never resyncing.
- It drives her routine, sleep and absence included. While she's away **the home shows empty, with
  rare dash-ins**. A resident leaves and comes back.
- Calendar **dates** are the **real** date: a starter set of cheap dates now; art-heavy ones wait.
- Tiers, rarity and pity. Rarity and pity gate *offering*: a factor below 1 deletes a want, because
  `brain::choose` truncates to the top four after factors.
- A **window** piece and a **wall-clock** piece make game time legible. Each gets a model sheet first.
- Later, maybe a cat who ignores the clock.

This session (2026-10-04):
- **Q1, a shorter school day.** School runs 08:30–12:30; she's home from 12:45. HG's 08:30–15:30 would
  leave every other two-hour session mostly empty or dark. What the time comes to is in D2.
- **Q2, errands while she's away or asleep.** At school, she dashes in through her door, pokes the
  accordion and goes straight back out. Asleep, she gets up groggy, pokes it and goes back to bed. This
  amends design.md's "whatever the idle gate says" only in *how* she comes; she still always comes.
- **Q3, the clock now, the window used.** The wall clock comes as a **one-time parcel** once the clock
  runs on a ledger with a home, so it reaches existing homes soon. The window joins the catalogue after
  the cat bed. She **looks out of it** (stars at night, "Pretty..." at dusk). She **glances at the clock**
  at routine changes ("Oh! It's late!").
- **Q4, real-date vacations.** Summer (Jul 20–Aug 31), year-end (Dec 25–Jan 7) and spring (Mar 25–Apr 5)
  cancel school on game weekdays. Only the real **date** reaches the routine, never the real time of
  day.

**Decided unless the user objects** (each is recorded in decisions.md when it lands):
- Chat at night doesn't wake her. She stirs ("mm..."), turns over and sleeps on (D4).
- The calendar day starts at 09:00 (`timeutil::biblical_date`), like the chat's day separators. So
  Halloween lasts past midnight, and Christmas morning before 09:00 is still Christmas Eve.
- There is no "Night stays" setting (HG:776) yet. A sleeping houseguest covers nothing a waking one
  doesn't.
- "Osaka moved out" also resets her clock to Monday 16:00.

## Design decisions (mine; the non-obvious ones go in decisions.md)

### D1. The clock

**Storage.**
- `Ledger.clock: u64` holds **game minutes elapsed since `START`**, where `START` is Monday 16:00 of game
  day 0.
- `Ledger::new` keeps it at 0, so every pre-5b record, and every record an older build saved since, reads
  as Monday 16:00. That is right for a new ledger and acceptable for an upgrade (CHANGELOG says so).
- `START` is part of the format and never changes. Storing game time, not real time, means the 6× ratio
  can be retuned without moving her past.
- It is written after `unsettled`, `skip_serializing_if` zero, and read leniently: `Option<Value>`, then
  `.as_u64()`, else 0. `the_record_as_written`, `missing_fields_default` and the `ends_with` pin don't
  move.
- Rejected:
  - Absolute game ms since a Monday-00:00 epoch: equally good, but `Ledger::new` would no longer be all
    zero.
  - A separate storage key. It would survive a downgrade, but needs a second write, a second `Raw`, and
    `move_out` clearing both keys. A downgrade to `stable` resetting her clock is acceptable and
    documented.

**Accrual.**
- It happens at the top of `Guest::advance` only, which the shell calls at least once a second in every
  state.
- Non-persisted `Guest` fields:
  - `clock_at: Option<u64>`: the monotonic ms of the last accrual. The first call only latches.
  - `clock_rem: u64`: game ms below a whole minute.
- Each call adds `6 × (now − clock_at)` game ms, carries whole minutes into `ledger.clock`, and keeps the
  rest in `clock_rem`.
- **No clamp.** `Instant` is `CLOCK_MONOTONIC` on Linux and doesn't count suspend; the builder confirms
  this against the std docs and cites it. The test helpers legitimately jump hours, and a clamp would
  silently lose game time in them.
- `paint` never accrues: a ledger change inside the furnish diff (mod.rs:895-911) would dirty every
  frame.

**Reading.** One affine snapshot, `GameClock { at: u64 /*monotonic ms*/, game: u64 /*game ms*/ }` with
`fn at(&self, t) -> u64 { game + 6 × (t − at) }`.
- `Guest::game_clock()` builds it from `(clock_at, ledger.clock, clock_rem)`, and nothing mutates it.
- It is passed **as a parameter to `Osaka::tick`** (`Option<GameClock>`).
- Every decision reads `clock.at(its own at)`, because `tick` catches up through up to 64 wakeups.
- It does **not** ride on `Chances`. `Chances` is rebuilt only in `paint` and is `Chances::default()` on a
  visit's first tick (map G2), so the mind would see a stale or missing clock exactly when it matters.

**When it runs.**
- Only once she has met you (`ledger.visits > 0`), and then whenever dessplay is open, whatever the
  Visits setting.
- A client that never met her writes no record. Her first meeting is at 16:00 plus the idle delay.
- With `persist == false` it runs in memory only.

**Saving.**
- Time dirties the ledger in batches. `unsaved` is set when 30 game minutes (5 real) have accrued since
  the last handout, and at every routine slot change (D2), so the record is consistent at departures and
  bedtimes.
- Events still dirty at once (`record`), and any handout carries the clock for free.
- **Exit save (structural).**
  - `run_ui_loop` / `run_ui_thread` return the final `Option<Ledger>`: `persist && touched`, with the clock
    accrued to the exit's `now`.
  - run.rs saves it after both joins (setup and session), before `Rejected` returns and before Resync's
    `exec_self`.
  - The loop body moves into an inner fn, so every `return` and `break` falls through to the one final
    save.
  - This also fixes today's bug: a `Bought` recorded in the Quit iteration is lost (map, ledger P5).
  - It does **not** go through `ledger_to_save()`'s `unsaved` flag: a handout parked in `ledger_unsent`
    has already cleared it.

**Stage.**
- The example scales `now` by its speed, so its clock runs at speed × 6.
- `t` skips forward to the next slot boundary (`Guest::skip_clock`, forward only).
- `d` cycles the stage's date through the starter calendar, then the real date, then `None`.
- The bar shows "Mon 16:05".

### D2. The routine (new `routine.rs`, pure)

`fn slot(day: GameDay, minute: u16, vacation: bool) -> Slot`. Every value below is a const table.

| Game time | School day | Weekend or vacation |
|---|---|---|
| 22:30 / 23:30 – 07:00 / 09:00 | **Asleep** (from 22:30 before a school day, else 23:30; until 07:00, else 09:00) | Asleep |
| 07:00 – 08:15 | **Morning**: up, stretch, breakfast (Hungry rises) | — |
| 08:15 – 12:45 | **Away**: school (leaves 08:15, "I'm off!"; back 12:45, "I'm home!") | — |
| 09:00 – 12:45 | — | **Morning**, slow (lounging, reading) |
| 12:45 – 18:00 | **Afternoon**: snack, sofa, TV | Afternoon (the part-time job may happen) |
| 18:00 – 20:00 | **Evening**: dinner (Hungry), TV | Evening |
| 20:00 – bedtime | **Homework** on school nights (Sun–Thu) | **Evening**, reading |

- **The days.** Game day 0 is Monday. A school day is Mon–Fri and not vacation. A school night is the
  evening before a school day.
- **Vacation** (Q4) is a pure `fn vacation(date: NaiveDate) -> bool` on the real date, `false` when
  `local` is `None`. Its last week of August, Aug 25–31, is homework-panic week: homework is boosted and
  she says "Homework! Homework!".
- **What the time comes to at 6×.** A game day is 240 real minutes.

  | Day | Asleep | Away | Home and awake |
  |---|---|---|---|
  | School day | 85 min (35%) | 45 min (19%) | 110 min (46%) |
  | Weekend day | 95 min | 0 | 145 min (60%) |
  | Week | 37% | 14% | 49% |

  Asleep is visible, so she's on screen about 86% of the time.
- **Work.** The part-time job stays today's short in-visit shift, still a want. It is now open only on
  weekend or vacation days from 10:00 to 17:00, not "≥3 min into any visit". `may_work` is computed in
  one fn, not two (map, visit-lifecycle trap 6).
- **One seam.** Every routine question (the slot now, the next boundary, "is this a school day") goes
  through `routine.rs`. The mind, the guest and the art read slots, never raw minutes. The window and the
  dial are the only readers of the minute.

### D3. Away: the departure ends the visit (option B)

**State.** `State::Away(Box<Empty>)` is the furnished home painted without her. `Empty` holds what
painting needs: the projected home, the cat, and the fades for a focused-pane rain.

**Why B, not a long hidden act.**
- Every per-visit mechanism stays meaningful: `LINE_BUDGET`, the mood and greeting, `worked`, the
  once-a-visit rare draw (D6), and "calendar owed on the first visit of the day".
- Under a 45-minute hidden act inside `Visiting`, each of them needs re-keying (map G4.1).
- **The visible cost:** a resident's school day adds a visit (the return). That brings `SHOP_EVERY` and
  deliveries a little sooner, and deliveries already land on her return.

**Transitions.**

| From | Trigger | To |
|---|---|---|
| Visiting | Slot becomes Away (the reflex in D4 sends her out by door or edge). Once she's off screen, the visit ends **by door**, a new end with no dissolve | Away (furnished) or Absent (no home) |
| Absent | Idle gate opens during Away and she has a home | Away |
| Absent | Idle gate opens during Away with no home | stays Absent (wakes at the return) |
| Away | Slot leaves Away (her return), whether the idle gate is open or she's resident. The return isn't reset by input or chat (`quiet_since`): it has its own predicate | Arriving by **door or edge**, never the drop from the sky. `begin_visit` counts it, says "I'm home!" with no leeks, and the greeting is skipped |
| Away | Dash-in due (D3a), or an errand | a **dash-in visit**: not counted, ends by door back into Away |
| Away | Visitor input | Leaving: the props rain, no wave (`Leaving.image = None`) |
| Away | Visits off, an overlay, or too small | Absent at once, as Visiting does today |
| Away | Resident's focused pane gains focus | that pane's pieces rain out, as in Visiting (D:1860-1866) |
| any (cold start) | The client starts during Away | the Absent → Away row, when the gate opens |

**Touch list** (a builder must join every one; the compiler forces most):
- `State` sites: `restore`, `move_out`, `cue`, `present`, `activity`, `leave`, `advance`, `next_tick`,
  `paint`, `nudge_due`, `send`, `observe`, and `Trace::frame` in tests/golden.rs.
- The stage accessors use `_ =>`. Name `Away` in each of them on purpose.
- Away's paint builds the same `Looks` (cat via `cat_home`, the dial and sky in D7, the lamp **off**
  while she's out) and calls `draw_props`.
- `furnish`'s body-rng draw (mod.rs:906) must not run in Away. Project the home without it, or settle in
  `begin_visit` only.
- **Parcels:** none are delivered in Away. The parcel waits for her return, where `furnish` delivers it as
  today.
- **The goodbye shows her while hidden** (map, visit-lifecycle trap 1): drop `with`/`placement` when
  `osaka.hidden(now)`. Long absences make it likely, and the departure exposes it. Fix the class: a
  hidden Osaka never yields an image.
- `next_tick` in Absent and Away wakes at the slot's next boundary, `(boundary − game) / 6` real ms. So
  a census or `tests.rs::run` never skips one.

**Dash-ins (D3a).**
- "Forgot my lunch!" (HG #47).
- **When.** At most one per away period, with chance 1 in 3, at a time inside it. Both are hashed from
  `visit_seed(visits) ^ DASH_SALT` and the game day: no body rng.
- **How.** She comes in by door to a spot by the fridge if she owns one, and plays `DashLunch`: fridge open,
  `Still(Side)`, then the line. Without a fridge, a ~2.5 s SpaceOut with the line. Then she goes back
  out by door.
- **What it builds on.** It is an `Errand`-like dash visit: the rng is reseeded from that hashed seed,
  and `visits` isn't bumped. The act is built **directly**, not through `start_job`, so it rolls no
  splice, feels no grievance and pushes no `Used`.
- **The errand at school** (Q2) is the same dash: door in, poke, and after the poke the routine reflex
  sends her out by door, not the dissolve (`errand_progress` branches on the slot). From Absent with no
  home, the errand still brings her by door, and she leaves by door.

### D4. Sleep, and the routine in the mind

**The clock reaches the mind as `Option<DayTime>` on `Ctx`.** `DayTime { minute, slot, school_day, night_before_school }`
comes from `routine.rs` at the decision's own `at`. `None` makes every path below exactly today's: each
factor and rate is exactly `1.0`, no reflex fires, no gate closes.

**Four levers, each with one job.**
1. **Boosts.** `Factor::Clock(SlotSet, f64)` and `Factor::Season(DateWindow, f64)`, always **> 1**.
   - The lint in brain.rs:811 becomes a `match` asserting `InChat < 1` and Clock/Season `> 1`.
   - `in_chat()` overwrites `factors`, so a row with both gets its own `const` slice.
   - The table: Snack and Lounge in the Afternoon; Watch in the Evening; Homework on school nights
     (and in exam season, and panic week); Read on weekend evenings; Stretch in the Morning; Gaze at
     dusk and night (×more with a window); Sneeze in Mar–Apr.
2. **Need rates by slot.** `Needs::pass(ms, rising, mood, rate)`, where `rate` is `fn(Need) -> f64` from the
   slot (exactly 1.0 at `None`).
   - Sleepy: ×0.3 by day, ×1 in the Evening, ×3 in Homework.
   - Hungry: ×2 in the Morning and Evening, so meals emerge without a meal want.
   - The product with mood is bounded at ×4 (map, mind trap 7).
3. **Arrival levels.** `osaka.set_clock(day)` in `begin_visit` beside `set_mood`. She arrives sleepy at
   23:00 and hungry at 18:30. The Afternoon slot gives exactly today's `arriving` values.
4. **Reflexes,** for what *must* happen. A new step in the Reflex bucket, after the `Ctx` literal and
   before the heading continuation:
   - `routine/away`: leave by edge or door (`go_to_work`'s exit, with no rng and no `worked`), saying
     "I'm off!".
   - `routine/bed`: bind `Use(Sleep)`, else `Use(Nap)`, else a makeshift sleep (its own methods), else
     `Idle(LieBack)`.
   - With a wall clock on her strip, either one starts with the clock glance (D7).

**Who owns a boundary.**
- `Osaka::due()` includes the next **cutting** boundary (Away's start, bedtime) while the clock is
  `Some`. At it, `tick` calls `interrupt(Cause::Routine)` unless she's already hidden or departing.
- **The reflex owns the decision; the boundary only interrupts.**
- Non-cutting boundaries (Afternoon → Evening) add no wakeup. So a golden in 16:00–17:00 gains no step.
- The exits that bypass `interrupt` (`place`, `settle`'s Fall and Dazed arms, `muse`) all end in `decide`,
  which hits the reflex. That makes "awake at 02:00" and "home at 10:00 on a school day" unreachable
  through any path.

**Homework is boosted, not gated.** A gate would strand exam season's owed chopsticks (map G3.9).
**In-window uses lengthen.**
- Inside its own slot, a homework use lasts 2–4 min real instead of 30–60 s: the same single `rng.range`
  draw, over a slot-dependent range.
- Sleep in the Asleep slot lasts until the wake time: the draw is made, then overridden.
- Nothing else lengthens, so the Homework slot reads as homework with breaks.

**The night's sleep.**
- **One act.** `until` is the wake time in real ms, at most about 85 min.
- **Needs.** `Rising.asleep_ms`: Sleepy doesn't rise while she sleeps, and the other needs rise at ×0.25.
  Otherwise an 85-minute sleep wakes her with every need at 1 (map, mind trap 1). The credit/pass order
  is unchanged.
- **Tucked in.** A visit that begins in the Asleep slot, with a bed (else a sofa), begins with her in it:
  `Osaka::tucked_in(seat)`.
  - It builds `Act::Use { Sleep, Play { own: Sleep, branch: 1 } }` (lamp off from the first frame)
    directly, after the first `furnish`.
  - With neither piece, it's a normal arrival, and the bed reflex follows.
- **Chat at night.** A new `script::Chat::Stir` on the night sleep's script.
  - `look` consults the own script's `on_chat`: she stirs ("mm...", `Face::Blink`, a turn of the
    `Sleep` pose) and sleeps on.
  - The day's 1–3 min sleeps and naps keep today's Look.
- **Waking.** At the wake time she sits up, stretches, and says "Mornin'..." (a greeting variant).
- **Errand while asleep** (Q2). She gets up groggy: she walks with `Face::Blink` and says "...mm?". She
  pokes the accordion, then the bed reflex takes her back to bed.
- **Sleep-talk.** A pool on the night sleep: "...five more minutes", "Mm... melon bread...". It is said
  while asleep, every few minutes, at most `LINE_BUDGET`-free, with each line's 10-min cooldown.

### D5. The calendar (the real date)

**The date.**
- `IdleView.local: Option<NaiveDate>`. `None` disables every calendar path, and vacations with it.
- The shell stamps it in `draw()` after `idle_view`, from `timeutil::biblical_date(now_utc_ms)`, cached
  per real minute.
- The `Ui` and `Guest` still read no clock. shell.rs:700-706 and design.md:1919 get the carve-out.
- Only a `NaiveDate` crosses over, never a time of day (Q4).

**Owed once a day.**
- `Ledger.calendar_on: Option<String>` ("YYYY-MM-DD", parsed leniently, skipped when `None`).
- An item is owed when `local`'s entry exists and `calendar_on != local`.
- It is checked **at decision time**, so a resident across midnight gets the new day.
- It is marked delivered **when shown**: the bubble drawn, or the script's first key. Not when set.
- `HomeEvent::Calendar(date)` writes it, one sqlite write a day.

**How an item plays** (no free-standing `Act::Script`, map 5.1):
- **Greeting.** `greeted` and the calendar's owed flag are separate, so a drop-in arrival ("...I'm OK.")
  still gets it.
- **An owed SpaceOut script.** In the Owed bucket with method `calendar`, at a fixed length.
- **An owed splice cue** in its own slot, never the stage's `cued`.
- **Seasonal pools,** each its own stable `PoolId`, not budgeted.
- **Boosts** (D4.1).

**The starter set** (lines and code only; every line ≤ 24 chars, enforced by `line!`):

| Real date | Content |
|---|---|
| Jan 1–3 | Greeting "Happy New Year!"; an owed watch on `Tv(Sunrise)`, "Ooh... first sunrise."; sleep-talk "Pigtails... flying..." |
| Jan 20 – Mar 10, exams | Homework ×2; an owed chopsticks splice once a visit, played at her next homework |
| Feb 3, Setsubun | Owed SpaceOut on alternating `Jack` keys: "Oni wa soto!", then "Fuku wa uchi!" |
| Mar 1 – Apr 30 | Sneeze ×3 (hay fever) |
| Apr 8, debut day | Greeting "Nice to meet you!" |
| Jul 7, Tanabata | Greeting "My wish: be a bird." |
| Aug 25–31 | Homework-panic week (D2) |
| Sep 30, the finale | Greeting "We graduated, huh..." |
| Oct 31 | Greeting "Trick or treat!" |
| Dec 1–31 | Seasonal musings ("Rudolph's nose... why?") |
| Dec 24–25 | Greeting "Merry Christmas!" |

The art-heavy dates wait: the sheet ghost, the tree, the kotatsu, the flying pigtails.

### D6. Rarity and pity

**Rows.**
- `Rarity { Common, Uncommon, Rare, Legendary }` goes on `ScriptId`, not `Tier`: rules.rs already has a
  `tier`.
- Common and Uncommon are **ungated**. Their existing `chance` rolls *are* their rarity, so no second
  system stacks on them.
- Only Rare and Legendary rows pass the visit gate.

**The visit gate.** `Rares`, drawn once per visit in `begin_visit` from `visit_seed ^ RARE_SALT`
(splitmix, like `Mood::of`), and stored on `Osaka`:
- `open: Vec<ScriptId>` holds the **seen** rares whose roll passed this visit.
- `new: Option<ScriptId>` holds **at most one unseen** rare. The `Option` makes two unseen rares in a
  visit unrepresentable.
- The roll for a tier is `p(tier) + ramp(pity)`. Rare is 0.15 a visit, certain once 6 real idle hours
  have passed since a new rare was last seen. Legendary is 0.02 a visit, certain at 40 hours (HG's
  bounds).
- Gated wants check `Rares` in `mind::bind`. Gated splices are skipped in `splices()`' candidate loop
  unless forced. Inside an open visit the row's own `chance` and `when` still apply.
- `Rares::none()` (clock off, tests) opens nothing.

**Pity.**
- `Ledger.idle_min: u64` counts **real** minutes the client was idle (`open && now ≥ quiet_since`),
  present or not, asleep included.
- `Ledger.rare_at: u64` and `legend_at: u64` hold `idle_min` when a new one of each was last seen.
- They are batched with the clock and skipped at zero.

**Seen** means first shown: the script's first key plays. `HomeEvent::Seen(id)` pushes it into
`Ledger.seen: Vec<String>` (stable ids, unknown ones skipped on read). An interrupted walk to a rare
leaves it unseen.

**Starter rares** (lines on existing poses):

| Id | Rarity | Where | What |
|---|---|---|---|
| `Dream` | Rare | a splice on the night sleep, after 30 min asleep | Sleep-talk in three keys: "Everynyan..." / "Oh my gah!" / "...wish I were a bird." (HG #74, Chiyo-chichi, as words) |
| `NoMelon` | Rare | an after-splice on a snack | Fridge open, `Face::Droop`: "No melon bread..." (HG #45) |
| `Escalator` | Rare | a SpaceOut musing | "The box one's the..." then "...escalator? No?" (HG #54) |
| `Scary` | Rare | an after-splice on Lounge or Read, 22:00 – bedtime | "Scary story time..." then "A fart. Not mine." (HG #53, no dimming) |

There is no Legendary row in 5b. The machinery is tested with cfg(test) rows.

### D7. The window and the wall clock

**Rows.** Both are appended to `Furniture::ALL` and `CATALOGUE`, and both are hung.
- The **clock** is 3×2, hung at 4. It's decor-like, with beauty 0.5 and no use.
- The **window** is 4×2, hung at 4, with `Use::LookOut`.
- `Spec.symmetric: bool` normalises facing to Right for both. A mirrored dial would read 3:00 as 9:00
  (map, room-art trap 1), and this also halves their cache keys.
- The hung-piece lint is relaxed on purpose to "no `inside()` use".
- `rooms()`' `decor()` stand-in becomes an explicit `Furniture::legacy()`.

**Looks.**
- `piece_state` gains `World { cat, clock: Option<GameClock> }`.
- The **dial** is quantised to the quarter-hour: 48 images at most, and 2 a real hour. It is drawn
  procedurally, rotating two hand `<use>`s, never mirrored.
- The **sky** has five phases (night, dawn, day, dusk, evening): static groups behind the frame, clipped
  like the TV's `GLASS`.
- `Plain` stays a sane look, because `Leaving` paints `Looks::default()` (map G4.6). Better: `Leaving`
  carries the `Looks` it froze.
- `next_tick` chains the next quarter-hour **only while a clock or window is projected**. Homes without
  one gain no wakeup, and their goldens don't move.

**Getting them.**
- **The clock** (Q3): a one-time order, once the clock runs on a ledger that owns a TV, nothing is on
  order, and `Ledger.clock_sent` is false. It is then set (skipped while false). It arrives on her next
  visit as a parcel.
- **The window** is sold after the cat bed, before the decor: `CATALOGUE` order. That changes census
  purchases only for homes that have a cat bed.

**Uses.**
- **LookOut** seats her under or beside the window in `Pose::Gaze` / `Face::Curious`, with a script
  whose line comes from the sky ("Stars!", "Pretty...", "Sunny!").
- It serves Daydreams 0.5 and Fun, and is boosted at dusk and night.
- Its touch list is the map's room-art O3 (C2) list.
- **The clock glance** is a prelude on the `routine/away` and `routine/bed` reflexes when a clock is
  projected on her strip. She turns to face it in `Pose::Gaze`: "Oh! It's late!" at bed, "Time for
  school!" at 08:15. It is the reflex's first key, not a splice, so it can't be rolled away.

**Art.** One model sheet with:
- the window in all five skies;
- the dial at 12 hours × 4 quarters;
- both facings, to prove the normalisation;
- her in `Gaze` under the window;
- both pieces hung beside the poster over a sofa.

It is drawn in a throwaway worktree (commit with git there) and saved under `phase5b/art/`. It needs the
user's approval before wiring.

## Determinism and goldens

**No new draws.**
- Per-visit rolls hash `visit_seed` with new salts.
- Decision rolls are `Whims` labels with new names.
- The lengthened uses keep their one `rng.range`.
- `furnish` doesn't run in Away.
- No body rng is drawn in Absent or Away.

**Order of landing.**
1. The **exit save and the ledger clock.** The frames don't change. Every `finish` hash moves, because
   `to_json` now carries `clock`. Re-record once, and prove it with `HOUSEGUEST_GOLDEN_TRACE` before and
   after: in all 32 traces only the trailing JSON line differs.
2. The **mind plumbing, unfed** (`Ctx.clock` is `None` in production). Goldens are unchanged, which proves
   the plumbing is inert.
3. **Feeding the clock** moves frames, since 16:00 is in the Afternoon slot with its boosts. Re-record
   deliberately, with the diff explained in the commit. The design isn't bent to keep 16:00–18:00 neutral.
4. **The test "`None` ≡ today"** traces a visit with the clock `None` against the pre-5b tables in
   golden.rs, kept as a second table. It guards every later step.
5. **New golden scenes:** 20:30 (homework), 23:00 (a tucked-in sleep with a chat line: the stir), 08:10
   (the departure, then Away), 12:40 (the return), and a dash-in.

**Censuses.**
- The census `Room` gains `start: Option<GameTime>`. `None` keeps 5a's tables comparable.
- A new `day_census` (`#[ignore]`, release) runs a game week from Monday 00:00 for each room and seed.
  Per game hour it reports % present, % away, % asleep, the act groups, dash-ins, departures and returns,
  calendar plays, rares offered and first seen.

## Tests (written first where they apply)

**Pure and cheap (the gate):**
- **The routine tape** over 7×1440 minutes:
  - every slot occurs;
  - weekends and vacations have no school;
  - slots are contiguous;
  - school nights are Sun–Thu.
- **Accrual** is independent of how ticks are partitioned. The first `advance` doesn't accrue.
- **The clock across save and load:**
  - it never decreases within a process;
  - it is equal after a clean exit and restore;
  - after a crash it loses at most 30 game minutes.
- **Exit paths:** every one returns the final ledger, including one parked in `ledger_unsent`.
- **Ledger round trips:** `clock`, `idle_min`, `seen`, `calendar_on` and `clock_sent` round-trip.
  - Defaults write nothing.
  - Garbage reads as the default without failing the record.
  - An older build keeps every piece.
- **Pity** is a pure function of `(seed, counter)`:
  - monotone in the counter;
  - certain at the bound;
  - within tolerance of its base rate over fixed seeds.
- **Rares:** at most one unseen per visit, over 1000 `begin_visit` seeds.
- **The calendar over 2024–2040:** at most one entry a day, stable across calls, each starter date on
  exactly its days.

**Sims (fixed seeds, ASCII and line art, text-dense panes):**
- One game day from Monday 06:00:
  - she wakes;
  - she leaves at 08:15, and the home stands empty;
  - she returns at 12:45 ("I'm home!", by door or edge);
  - homework in the evening;
  - bed at 22:30;
  - asleep through a chat line.
- Visitor at school time: Away shows the home, and input rains the props with no wave.
- Resident: she leaves and comes back, and her focused pane is protected in Away.
- Cold start during school: Away. With no home: nothing, then an arrival at 12:45.
- Errand at school: a dash in and out by door. Errand asleep: groggy, then back to bed.
- A dash-in never counts as a visit.
- No parcel is delivered in Away; it's delivered on her return.
- A hidden Osaka never yields a goodbye image.
- The calendar is owed once a day:
  - two visits on one date play it once;
  - the next date plays it again;
  - `local: None` never plays it;
  - a resident across the day boundary gets it.
- The window and clock:
  - never mirrored;
  - the dial changes only on quarter-hours;
  - no wakeups without them;
  - `VISIT_IMAGES` ≤ 512 with both shown.
- **Protected cells:** a new proptest over `start in 0..WEEK`, with Away and dash-ins included.

## Steps (each implemented, reviewed twice, then fixed)

0. **Docs.** This design, the map and its critiques, CLAUDE.md, and the plan.md status, committed before
   any builder runs.
1. **The exit save.** A regression test first: the Quit iteration loses a `Bought`.
2. **The ledger clock:** accrual, `GameClock`, batching, the stage clock and keys, and the golden `finish`
   re-record with the trace diff.
3. **`routine.rs` and the mind plumbing, unfed:** `DayTime`, the factors and lint, rates, arrival levels,
   reflexes, `Cause::Routine`, `due` boundaries, `Rising.asleep_ms`, and the lengthened uses. Goldens are
   unchanged.
4. **Feed the clock:**
   - the night sleep, tucked in, `Chat::Stir`, waking, and sleep-talk;
   - the bed reflex;
   - the re-record, plus the clock-fed golden scenes.
5. **Away:**
   - `State::Away`, the departure and return, and cold start;
   - visitor input, focus, the cat, parcels;
   - the hidden-goodbye class fix;
   - dash-ins and errands.
6. **The date:** `local`, vacations and the starter calendar.
7. **Rarity:** `Rares`, pity, seen, and the four starter rares.
8. **Art,** in parallel from step 1, then the wiring once approved:
   - the window and clock rows, the dial and sky;
   - the clock parcel;
   - LookOut and the clock glance.
9. **Finishing:**
   - the day census, the 256-case pass and perf;
   - the dump section (`houseguest`: game time, slot, pity, seen);
   - design.md and decisions.md;
   - CHANGELOG, with the clock starting at Monday 16:00 for existing homes;
   - the plan.md record.

## Docs that change

**design.md:**
- the idle gate gains the routine (Away and asleep);
- the resident leaves for school;
- the errand comes by door while she's away;
- the part-time job is on weekends and vacations;
- "saved whenever it changes" becomes "events at once, time in batches and on exit";
- the monotonic-clock carve-out for the date;
- "redraws only when her pose changes" gains the dial and sky;
- the furniture list gains the window and clock;
- new bullets for the routine, the calendar, rarity, and the window and clock.

**decisions.md:** B over A, the stored clock form, no clamp, the downgrade loss, rarity per visit (with
Common and Uncommon left on their chances), night chat, the 09:00 day, and the fixed stale `houseguest`
key name. A note that `instance_lock` means two processes never accrue into one ledger.

**HG:** "Local wall clock, injected" is superseded: the routine runs on the game clock.
