# Phase 5b (the clock): code map

Written 2026-10-04 by six readers and a completeness critic at HEAD 0661710. Paths are under `dessplay/src/ui/houseguest/` unless prefixed. Anything marked (proposed) does not exist yet. The critic's corrections are at the end and override the sections.

---

<!-- section: visit-lifecycle -->

# Code map for 5b: the visit lifecycle and "away"

All refs were checked against the code on 2026-10-04 (HEAD 0661710). Files are `dessplay/src/ui/houseguest/` unless prefixed. **D:** is docs/design.md; **HG:** is docs/proposals/2026-09-28-houseguest.md. Anything marked **(proposed)** does not exist yet.

## 0. The organising fact

Today she can be away while her furnished home shows empty, but **only inside `State::Visiting`**, while a hidden act is running:
- `draw` skips her body when `osaka.hidden(now)` is true (mod.rs:1433).
- `draw_art` skips her layer the same way (mod.rs:2563).
- `furnish` and then `draw_props` still run on every Visiting paint (mod.rs:897, ~1100).
- The work test checks exactly this ("Her room is still there", in the `Scene::Work` test at tests.rs ~2950-3008).

Outside Visiting she leaves nothing of her own on screen:
- `Absent` and `Arriving` paint only the accordion nudge (mod.rs:791).
- `Leaving` paints her props as line art only before `RAIN_FROM_MS` (mod.rs:807-813). In ASCII the props are frozen cells inside the dissolve.

So "the home shows empty while she's at school" must be one of three things (§5): a long hidden act inside Visiting (A), a new state that paints the home without her (B), or nothing at all (C).

## 1. The state machine today (mod.rs)

`enum State` (353-360) has four variants: `Absent`, `Arriving` ("enters on the next paint"), `Visiting(Box<Visit>)` and `Leaving(Box<Leaving>)`.

`Guest` fields that matter here (362-402):
- `open`, `resident` and `delay` are copied from the view in `observe` (1303-1306).
- `quiet_since` is in monotonic ms.
- `errand: Option<Errand>`.
- `ledger` and `unsaved`.
- `rng` is reseeded from `ledger.visit_seed(visits)` at every arrival: on the Arriving paint (776) and in `send` from Absent (1267).

### Transitions

| From → To | Where | Trigger |
|---|---|---|
| Absent → Arriving | `advance` 704-713 | `open && now >= quiet_since + delay` |
| Arriving → Visiting | `paint` 771-787 → `begin_visit` 1147 | `Osaka::arrive` (osaka.rs:1281): random walk-in from an edge, or a drop from row 0 |
| Arriving → Absent | `paint` 772/784 | Nowhere to stand. `quiet_since = now` (retries after another delay) |
| Arriving → Absent | `activity` 664 | Resident and local input |
| Arriving → Absent | `observe` 1311, 1337-1339 | `!open`, or a chat line while not here (`quiet_since = now`) |
| Absent → Visiting | `send` 1265-1270 | Errand: `arrive_for_errand` (osaka.rs:4116), out of a door. **Also `begin_visit`** |
| Visiting → Leaving | `leave` 670-697 | Visitor input (`activity` 661), `!open` (`observe` 1318), errand done (`errand_progress` 1216) |
| Visiting → Absent | `observe` 1312-1315 | Visits switched off (no goodbye) |
| Visiting → Absent | `paint` 980-989 | No room: evicted, too small, or `settle` failed. `quiet_since = now` |
| Visiting → Absent | `move_out` 458-460 | Also applies to Arriving. Leaving is left to finish |
| Leaving → Absent | `advance` 730-736, `paint` 793-796 | Dissolve done, or the screen was resized |
| any → Arriving | `cue` 532-534 | Stage: `Scene::Arrive`, or any scene while not Visiting. **Ignores every gate** |

### Exhaustive `State` sites a new variant must join

These are the "class" sites:
- `restore` 418
- `move_out` 458
- `cue` 532
- `present` 645
- `activity` 663-667
- `leave` 671-695
- `advance` 703-737
- `next_tick` 743-757
- `paint` 771, 790-796 and the Visiting arm at 850
- `nudge_due` 1231
- `send` 1265-1272
- `observe` 1310-1319 and 1325-1341

The stage accessors (`playing`/`mood`/`explain`/`broken`/`repair`, 556-630) use `_ =>`, so a new variant falls through them silently.

### `begin_visit` (1147-1179)

- It does `ledger.visits += 1` and sets `unsaved`.
- Mood comes from `visit_seed(visits-1)` (1152).
- It builds a fresh `Visit`: empty `layer`, `made`, `shown` and `flap`.

**`visits` feeds:**
- `SHOP_EVERY` (`advert` 126)
- the TV/parcel delivery (1680-1685)
- `cat_home` (2239-2246, a seed bit)
- the mood
- every visit's seed

In `Osaka::new` (osaka.rs:1210), the visit-scoped flags start fresh: `greeted:false` (1228), `at_work`/`worked:false` (1230-1231) and `arrived=now`.

## 2. The idle gate, the delay and `quiet_since`

- `IdleView::open()` (idle.rs:90-98) is `delay.is_some()` and: `busy` is None, or `Playing`/`Selection` while resident. An overlay always closes it. `resident` is a view field (idle.rs:75-77).
- `observe` (1302-1344):
  - If `!open`: `quiet_since = now`, then Arriving→Absent; Visiting→Absent if `delay` is None; otherwise `leave`, unless she is on an errand.
  - A chat or IRC mark change while Absent/Arriving does `quiet_since = now` (1337-1339). A chat line counts as non-idle.
- `activity` (651-668): `quiet_since = now` first. For a visitor on an errand, `leave_after` is set. A visitor otherwise leaves. A resident goes Arriving→Absent, or `shake` if Visiting.
- `gate` (1286-1299): while in use (`!quiet || busy`), a resident's focused pane is protected, except during an errand.
- `next_tick`'s Absent arm (744-747) wakes only at `quiet_since + delay`. The shell's timeout arm (shell.rs:479-494) calls `advance` at least once per `ui.next_tick_hint()`, but it **redraws only when `advance` returns true**. Any clock-driven change while Absent must return true from `advance`, and should add its boundary to `next_tick`.
- D:1347-1356 ("Idle means… continuously for the delay"; the terminal must be ≥60×18) is the only arrival rule. **D has no time-of-day rule**, and 5b adds one: see D-conflicts in §8.

## 3. The part-time job: the existing absence (osaka.rs unless noted)

**Rule (D:1562-1568):**
- Once she has a home, at most once a visit and ≥3 min in.
- She goes out at an edge, else through her door.
- The room stands furnished for 1–3 min.
- A chat or IRC line doesn't fetch her, but mischief is undone at once.
- She comes back the same way with leeks: "I'm home!".

**Eligibility:**
- The want is `Want::Work` (brain.rs:373-374), with row `own_sake`, base 9.0 and Restless 0.5 (brain.rs:480-483). It binds through `WORK` / `fn work` (mind.rs:203, 293-308), gated by `Ctx.may_work` (mind.rs:89).
- `may_work = furnished && !worked && episode.is_none() && at >= arrived + WORK_AFTER_MS`. It is computed twice (3020-3023 and 3074-3077).
- The constants: `WORK_AFTER_MS` 3 min (882), `SHIFT_MS` (60 s, 180 s) (884), `HOME_MS` 3 s (886), `HOME` "I'm home!" (888).

**Going:**
- `Bind::Work(out)` (3225-3228) calls `go_to_work` (4358-4378). It sets `at_work = worked = true`.
- With an edge link (`Route::Around`), she does `travel` then `Act::Out`. At the edge, `Out` sets `Act::Away { until: at + SHIFT }` (1744-1770).
- Without one, `Act::Door { to: here, gap: SHIFT }`: the same spot, with a gap between the doors.

**While away:**
- `hidden()` (4082-4090) is true for `Away`, and for `Door` beats with `!beat.her`.
- `ActProps.on_chat = Back` for Out/Away/Door (716). `look` sets `watch_until` and pulls `pending` mischief to now, then returns early (3889-3891).
- `due()` for `Away` is `until` (1532), so a far wakeup costs nothing. `tick`'s 64-step loop then resumes from now (1704-1706).

**Back:**
- `Away` fires: she is placed at `enter` and walks to `to_x`. At the walk's end with no link, `home_from_work` (2173) runs.
- The door's end runs `home_from_work` (1797).
- `home_from_work` (4381-4394) does `take(at_work)`, `say(HOME)` and `Act::Home{until: +3 s}`. That act is drawn as `Pose::Carry` (4485). `Act::Home` ends in `decide` (1811).

**What cancels or keeps work:**
- `errand()` sets `at_work = false` ("Work can wait", 4145, 4150, D:1885).
- `place` sets `at_work = false` (3857).
- `evict` keeps the shift when she is leaving for work (4308-4322).

**`SHIFT_MS` is drawn from the body `rng` at three sites:** 1754, 4321 and 4365.

**Can it be reused?**
- Yes for drawing hidden inside Visiting, for chat handling while out, for the `Out`/`Away`/`Door{gap}` exit and return paths, and for `home_from_work`'s hook point.
- No for where the gap comes from: the three body-rng sites above. A clock-driven gap replaces all three or none of them, and changing them moves goldens.
- No for `worked` (once a visit) and `may_work`'s 3-min gate. School is clock-driven, not a want.
- No for the `at_work` cancellation by errands (§4).
- No for the leeks: `Pose::Carry` is "back from work with her shopping", not school.
- A school absence needs its own flag, or an enum replacing `at_work: bool`: `Out(Why)` **(proposed)**, with Why = Work | School | Errand…

## 4. The errand (scrollback accordion)

**Rule (D:1880-1901):** "she goes to poke it, whatever the idle gate says". Only an overlay or a held selection keeps her away. When Absent, she arrives for it out of a door. "Out at work, she comes straight back through a door." A visitor who shouldn't be here then leaves with the usual goodbye.

**Code:**
- `nudge_due` (1224-1241): `visits = view.delay.is_some()`. If she's not visiting or it's too small, the accordion shakes by itself (1239-1240).
- `send` (1245-1283):
  - Visiting: `osaka.errand(spot)` (osaka.rs:4132-4161). From `Away`/`Out` she goes straight to a door to the spot, and `at_work = false`.
  - Absent/Arriving: reseed, `arrive_for_errand`, then **`begin_visit` (visits += 1)**.
- `errand_progress` (1182-1220): when done, a visitor who isn't quiet, or who had input, or is `busy`, leaves via **`leave()`, the dissolve with the startled-and-wave beat** (1216). A resident stays.
- The errand poke ends in `Act::Poke` (osaka.rs:1812-1826). Off text, she leaves by `through_door`.

**At school, by option:**
- (i) The cheapest is to treat school like visits-off in `nudge_due`: `visits && home(game_now)` **(proposed)**. The accordion shakes by itself. This contradicts D:1880's "whatever the idle gate says", so the user must decide.
- (ii) A dash-in poke from Absent: the door-in exists, but the way out is the dissolve and it bumps `visits`. It needs a door-out end-of-visit that doesn't exist (§6).
- (iii) Under option A while she's away inside Visiting, `errand()` already brings her straight back and clears `at_work`. School would need re-departure after `Poke` ends: re-issue the absence instead of `decide` **(proposed)**.

## 5. How the empty home can be shown (choose one)

**A. A long hidden act inside `Visiting`: extend the shift.**
- What it reuses:
  - `furnish`, `draw_props`, parcels (1680-1693) and gifts.
  - The chat `Back` handling, `evict`'s Away arm (4274) and the resident door paths.
  - A huge `dt` into `needs.pass` is safe because it clamps (brain.rs:293-309).
- What it costs:
  - Visit-scoped state lives across ~70 real minutes (school 08:30-15:30 is 7 game hours at 6×): `worked`, `arrived` (`may_work`), `greeted`, mood, `made` pieces, the moved-text `layer`, `recent`, line cooldowns.
  - She comes home as the same visit: no greeting, no new mood, no `visits++`.
  - Every paint still does `Terrain::read` + `furnish` + `mend` (rules repair search) + `seats` for an empty room.
- Only a resident (or a visitor who stays idle) is Visiting long enough for this. A visitor who arrives idle in school hours still needs the arrival gate, plus B or C.

**B. (proposed) `State::Away(Box<Home>)`: the home painted without her.**
- The pure parts already exist: `home.project(buf, &view.nooks, &blocked)` (1625) and `draw_props(buf, graphics, &shown, &looks, truecolor)` (2385). `blocked` is `view.protected`, since there's no moved text.
- What it needs:
  - Its own `Leaving`-like goodbye for a visitor's input. `Dissolve::new` takes the props' frozen cells. `Leaving.image = None` gives no wave, which is correct.
  - A rule for parcels, which are delivered only in `furnish` with `visit.flap` and `visit.osaka.say` (1686-1693): deferred to her return, or a flap with no one home **(proposed)**.
  - Membership in every §1 site.
- The return goes through `begin_visit`, which is a new visit (`visits++`, a new mood and seed, the greeting at 2971). It must enter **by door or edge, never `Osaka::arrive`'s drop from the sky**.

**C. Gate Absent→Arriving on the routine; nothing shown.**
- The cheapest: one predicate in `advance`'s Absent arm, `next_tick`'s Absent arm and `send`.
- It contradicts the brief's "the home shows empty". It is only correct for the unfurnished room.

A mix fits the room kinds (§7): C for unfurnished; A for a resident who's already here; B or C for a visitor who arrives idle during an absence.

## 6. A dash-in as acts (proposed)

The dash-in is "forgot lunch", HG:284 and #47 (HG:485).

**Entry:**
- Under A she is Visiting and hidden: interrupt the `Away` like `errand()` does (4144-4158): `Act::Door { since: at - DOOR_THROUGH_MS, to: spot, gap: 0 }`.
- From Absent: a door-in constructor modelled on `arrive_for_errand` (4116-4126), `arrive_through_door(spot)` **(proposed)**, and **not** `begin_visit`'s `visits++`. That needs a `begin_visit` variant that doesn't count, or the dash-in counts as a visit, which moves `SHOP_EVERY` and deliveries.

**Grab:** walk to the fridge seat and play a script: `Act::Use` on the fridge with a dash key, or the free-standing `Act::Script` 5a deferred (plan.md "No free-standing `Act::Script` in 5a"). That is cross-area: script.rs/mind.rs. With no fridge, grab at the desk, or skip the dash.

**Out:**
- `through_door(here)` with `gap` = ms until the routine says home (`Act::Door{gap}`, as `go_to_work`'s None arm does). Or `Out` at an edge, then a long `Away`.
- Under B, the visit ends by door, then `State::Away`. **No such end exists today.** A visit ends only by dissolve (`leave`), abruptly (Absent at 986, 1314, 459), or by Leaving finishing.

**Randomness:** whether and when a dash-in happens must hash the visit or master seed plus game-day and salt, as `Mood::of`/`cat_home` do. It must not be a body-`rng` draw from Absent: those are harmless because of the reseed, but they are irreproducible across sessions. The 5a map's A6 rule applies.

## 7. Each room kind against "she is at school"

The census rooms (tests/census.rs) are `stage_room` (38-49: a visitor, owns nothing), `furnished_room` (58-76: a visitor, `home_screen` view, resident false) and `resident_room` (79-99: `resident_view`, `busy: Playing`).

| Room kind | Today, idle elapsed | At school (5b) | Errand at school | Dash-in |
|---|---|---|---|---|
| Unfurnished visitor ("stage") | Arrives | No arrival (C). Nothing to paint. `furnished:false` already blocks `may_work` (3020) | Shakes itself (i), or a door-in poke and door-out (ii) | Nothing to grab. Skip, or a "forgot something" door beat |
| Furnished visitor ("home") | Arrives | Home shown empty (B), or nothing (C). Visitor input ends it. Under B that's a dissolve of props with no wave | Same choice. A poke from Absent is `begin_visit` | Door in, fridge or desk, door out |
| Resident, here | Stays through input and playback (D:1854-1879) | Leaves by edge or door at the boundary (`Bind::Work`-shaped, but clock-forced, not a want), room stays (A). Returns by door or edge, "I'm home!"-like (`home_from_work` hook) | `errand()` brings her straight back (D:1885). Must re-leave after the poke (iii) | Interrupt the long `Away` as `errand()` does. Grab. Door out with the remaining gap |
| Resident, Absent at a boundary (client started at school time, or after an overlay or no-room) | Arrives after the delay | Arrival gated until home time. Then a door or edge entry, not a drop. B shows the home meanwhile | As the visitor row | As the visitor row |

**"A resident leaves and comes back":** today a resident leaves only on an overlay (`observe` 1318), visits off (1314), no room (986) or `move_out` (459). The return is idle-driven (`advance` 704-713) and reset by input and chat (`quiet_since`). A clock-driven homecoming must not be blocked by `quiet_since` resets from `activity` (652), chat-in-Absent (1338) or failed arrivals (784, 987). It needs its own predicate (`home(game_now) && open`) **(proposed)** beside, or instead of, the delay check, plus a `next_tick` wakeup at the boundary's monotonic time.

**Sleep** (in the brief, not this area's core): sleeping in her bed is "present", not away. Under the routine a resident who is asleep is Visiting with a long Use(Sleep). Nothing here changes.

## 8. Invariants a builder must respect

1. **She is an overlay only** (D:1357-1359). An empty home is still overlay cells, and every one of her cells must rain or vanish on visits-off (D:1851-1852: "switching the setting off removes her at once") and on overlays (D:1897-1898). B's state must be in `observe`'s `!open` block (1308-1320).
2. **Determinism:** every time and choice comes from monotonic `now` plus seeded draws. Clock-driven gaps and boundaries must be a pure function of (ledger game-minutes, session start, now), with no body `rng`. Replacing `rng.range(SHIFT_MS…)` at 1754/4321/4365 re-records goldens and the seed-7 snapshot (the 5a map's traps d11/d12).
3. **`ledger.visits` increments only in `begin_visit`.** Decide explicitly whether a dash-in or a homecoming is a visit, because it changes shop, delivery, cat and mood.
4. **The rng reseeds at each arrival** (776, 1267). Keep that true for any new arrival constructor.
5. **No drop-from-sky homecoming:** `Osaka::arrive` (osaka.rs:1281-1330) is random walk-in or drop. A return from school is by her door or an edge.
6. **`local: None` disables every calendar path** (5a map, A7). The routine runs on the *game* clock, not `LocalTime`, so it is independent of `local`. Say which one the stage and tests disable.
7. **`present()` (645) means on screen.** It is only read in paint (782). If B paints a home without her, decide whether that counts.

## 9. Traps and risks

1. **The goodbye can show her while she's hidden (PLAUSIBLE, not run).**
   - `with` is computed from her box without a `hidden` check: mod.rs:1066 calls `terrain::image(osaka.x, osaka.y, covers)` (terrain.rs:33-63).
   - With `go_to_work`'s door-in-place (osaka.rs:4362-4376), she is "away" at her own spot. If a piece overlaps her box, `layers` is non-empty, so `paint_layers` gives `Some` and `visit.image = Some` (draw_art 2565-2568).
   - Visitor input then does `leave` and `Leaving` paints `Pose::Stand/Surprised`, then the wave, at that spot (mod.rs:814-840), and `body` cells rain in her box although she was out.
   - If she is off screen (`Out`/`Away`), `key()` returns None (graphics.rs:511-521), so there's no image.
   - Long school absences make this far likelier. Fix the class: drop `with`/`placement` when `hidden`, or make the goodbye `image: None` while hidden.
2. **Errands clear `at_work`** (osaka.rs:4145, 4150) and `place` clears it (3857). A school absence that rides `at_work` is silently cancelled by any poke or stage placement.
3. **The `PARCEL` line is said while she's hidden** (mod.rs:1692). Deliveries run in `furnish` on every Visiting paint, so under A a parcel can arrive while she's at school: she says an invisible line, and `flap` is drawn. Defer the line to her return, or hold the delivery.
4. **`errand_progress` ends with the dissolve** (1216), which is wrong for a dash-in's exit. Branch it on "away" so she goes back out through her door.
5. **`cue` forces Arriving whatever the clock says** (532-534). The stage needs either a clock cue (set or jump game time) or a bypass. The example scales `now` by speed (examples/houseguest.rs:96-100), so a `now`-derived game clock runs faster there too. That's useful for watching a day, but it changes the 6× rate.
6. **Two places compute `may_work`** (osaka.rs:3020-3023, 3074-3077). Any "not during school" or "after school only" condition must change both, or be lifted into one fn.
7. **The visit-scoped state goes stale under A**:
   - `watch_until` from a chat line during school is stale on return. That's harmless: it is in the past.
   - `pending` mischief is undone at its time while she's hidden. That's fine.
   - `Visit.layer` text she moved stays displaced for the whole absence, unless it was pending-undone. Check it before choosing A.
8. **Work's `Out` at a screen edge, then `Away.enter`:** if the terminal is resized during a 70-minute absence, `enter`/`to_y`/`to_x` can be stale. Today's 1–3 min shift hides this. `evict`'s Away arm defers ("when she's back in, she'll be moved on", osaka.rs:4274) and `settle` re-anchors, but test it with a resize during a long `Away`.
9. **A routine boundary while she's in a long Use** (sleep, homework) at a school start: the departure must use `interrupt` or a forced bind. The 5a map's trap 5 lists exits that bypass `interrupt` (`place`, `settle`'s Fall/Dazed, `muse`).
10. **Census and goldens:** the census rooms simulate minutes from `now = 0`. A game clock starting at 16:00 (brief) means 20–120-minute runs never reach school (game 18:00–04:00 at 6×). They do reach dinner and sleep. School-time tests need a seeded clock offset **(proposed)** on `Guest::restore` or on the ledger.

## 10. Hook points 5b needs (proposed, by site)

- `advance`'s Absent arm (704-713) and `next_tick`'s Absent arm (744-747): `&& self.home_at(now)`, plus a boundary wakeup.
- `paint`'s Arriving branch (771-787): a door or edge entry when coming home, not `Osaka::arrive`.
- `advance`'s Visiting arm (715-729): check the routine boundary and call `osaka.go_away(why, until_ms)`, modelled on `go_to_work` but with no rng, no `worked` and a clock gap.
- `send` (1265-1270) and `nudge_due` (1228): the school policy for errands, (i), (ii) or (iii).
- `errand_progress` (1209-1219): the exit when she's away is a door, not `leave`.
- `home_from_work` (osaka.rs:4381): generalise it to `home_from(why)`, with leeks only for Work.
- `observe` and `move_out`: include B's state if B is chosen.

## 11. Rule sources to cite

**docs/design.md**
- The idle gate: D:1347-1356.
- Overlay-only: D:1357-1359.
- The input dissolve: D:1383-1390.
- The part-time job: D:1562-1568.
- Playback, overlay and visits-off endings: D:1850-1853.
- The resident: D:1854-1879, including "Headed out to work, she still goes; on her way home, the door is how she gets in" at 1869-1870.
- The errand: D:1880-1901, including "out at work, she comes straight back through a door" at 1885.
- Settings (a delay or Off only; no time setting): D:1902-1905.

**docs/proposals/2026-09-28-houseguest.md**
- The school-day table: HG:276-288.
- #46 toast morning: HG:483-484.
- #47 forgot lunch: HG:485.
- "Night stays" (off means she leaves at bedtime): HG:776.

**docs/decisions.md**
- The resident decision: decisions.md:2091.

**D-conflicts 5b must resolve in design.md and decisions.md** (the 5a map's d20, re-verified):
- The delay alone brings her (D:1347).
- A resident stays (D:1854).
- An errand fetches her whatever the gate says (D:1880).
- No time-of-day rule exists anywhere in D.

---

<!-- section: ledger-persistence -->

# Phase 5b code map: persistence of the ledger (clock, pity, seen rares, calendar date)

Line references were checked against the code on 2026-10-04 (after 5a). Anything marked **(proposed)** does not exist yet.

## P1. What exists today

### `ledger.rs`
- **`Ledger`** (ledger.rs:31-43) has five fields, all `pub(super)`: `master_seed`, `visits`, `home`, `ordered` and `bought_on`. It derives `Clone, Debug, PartialEq`.
  - `Ledger::new(seed)` (47-55) zeroes everything.
  - `visit_seed(visit)` (59-61) returns `master_seed ^ visit * φ64`.
- **`VERSION = 1`** (25). `from_json` rejects any other version (67-69). That makes it unreadable, and the shell then calls `keep_unsaved`. The design keeps VERSION at 1 for older builds (design.md:1911), so new fields must be additive and must not bump it.
- **`KEY = "houseguest_ledger"`** (28). decisions.md:2075 still says key `houseguest`, which is stale; ledger.rs:604 tests the old key.
- **`Raw`** (284-303) is the lenient reader:
  - It has no `deny_unknown_fields`, so a field this build doesn't know is ignored. That is how an older build reads a 5b record.
  - Scalars use `#[serde(default)] u64` (`master_seed`, `visits`, `bought_on`). They are lenient only when **missing**. A type mismatch fails the whole record.
  - Collections are `Vec<serde_json::Value>`, filtered per entry with `serde_json::from_value(..).ok()`: `rooms` 71-79, `anchors` 80-84, `unsettled` 85-89, `props` 91-121. `ordered` is `Option<Value>` (122-125).
- **`Saved`** (257-271) is the writer. serde writes fields in declaration order. `unsettled` is last and is the only skipped field: `#[serde(skip_serializing_if = "Vec::is_empty")]` (269).
- **`to_json`** (136-174) is a `serde_json::to_string`, with `unwrap_or_default` (173).
- **`load` / `save`** (177-191) go through `Storage::setting` / `set_setting` (storage.rs:476-500). A save is a single `INSERT … ON CONFLICT DO UPDATE` of the whole JSON.
- **Pinning tests:**
  - `the_record_as_written` (372-387): exact JSON of `furnished()`.
  - `unsettled_pieces_are_written_and_read_back` (471-493): asserts `text.ends_with(r#""ordered":"Desk","bought_on":6,"unsettled":["Tv"]}"#)`.
  - `an_older_record_is_all_settled` (498-503).
  - `missing_fields_default` (564-567): `{"version":1}` must equal `Ledger::new(0)`.
  - `an_older_build_keeps_every_piece` (405-422).
  - `it_persists_in_local_storage` (577-583).

### Dirtiness and the handout, `Guest` (mod.rs)
- Fields (mod.rs:361-401):
  - `ledger` (383)
  - `unsaved: bool` (385-386): "changed since last handed out"
  - `persist: bool` (387-389): false when the stored record couldn't be read
  - `quiet_since` (373-374): monotonic ms the client has been idle since
- `restore(ledger)` (412-435) seeds `rng` from `visit_seed(visits)`. Clock state must never draw from it.
- `keep_unsaved()` (440-442) sets `persist = false`.
- `ledger_to_save()` (445-447) is `(take(&mut unsaved) && persist).then(|| ledger.clone())`. It clones the whole ledger, so any handout carries every field.
- `move_out(seed)` (451-461) does the following: `ledger = Ledger::new(seed)`, `unsaved = true`, `persist = true`, clears `gift` and `shop_now`, and sets state to Absent if she was Visiting or Arriving.
- Where `unsaved` is set:
  - `send_parcel` (478)
  - `advance`'s Visiting arm: `self.unsaved |= record(..)` (727)
  - `paint` (896): `record`, then a ledger **`PartialEq` diff** around `furnish` (`let before = self.ledger.clone()` 895, `if self.ledger != before { unsaved = true }` 909-911)
  - `begin_visit`: `visits += 1; unsaved = true` (1147-1149)
- `record()` (1350-1384) records events: Bought and Unpacked dirty the ledger at once. The design says the record is "saved whenever it changes" (design.md:1910). decisions.md:2072-2090 rejected "saving on exit only", because a crash would lose a delivery.

### The shell (shell.rs)
- Guest construction (436-445):
  - `Some(Ok)` → `restore`
  - `None` → `Guest::new(rand::random())`, a fresh master seed every run until the first save
  - `Some(Err)` → `new` plus `keep_unsaved`
- `ledger_unsent: Option<Ledger>` (446) holds a handout parked when the action channel is full.
- Top of the loop (450-466): a move-out request, then the **only drain**: `ledger_to_save()` → `ledger_unsent` → `try_send(SaveHouseguest)`. On Full it re-parks. On Closed it breaks.
- `UserAction::SaveHouseguest(Ledger)` is at msg.rs:344. The trace log is at app.rs:136.
- **Monotonic now:** `now_millis()` (700-712) is `OnceLock<Instant>` elapsed ms since the first call, and is the UI thread's only time source. On Linux, `Instant` uses CLOCK_MONOTONIC, which doesn't count system suspend; neither does macOS's clock (my understanding, not checked here). A SIGSTOP or debugger pause does advance it.

### `run.rs` (the sqlite write)
- Load: run.rs:932-940, into `ui.houseguest_ledger` (app.rs:387).
- Saves are synchronous `ledger.save(&storage)` calls on the async loop:
  - the setup loop: run.rs:966-970 (`setup_storage`)
  - the session loop: run.rs:1786-1790 (`self.storage`)
  - Each dirtying costs one sqlite UPSERT, and it blocks that tokio task for its duration.
- Action channel: capacity 64 (run.rs:943).

## P2. How often `advance` runs (the per-tick entry point)
- `Guest::advance(now)` (mod.rs:701-740) is called from two places:
  - shell.rs:485, the recv-timeout arm. The timeout is `guest.next_tick(now)` min `ui.next_tick_hint()`. `next_tick_hint` is at most **1 s** (app.rs:711-725: 50 ms dungeon, 100 ms marquee or spoiler, otherwise 1 s).
  - shell.rs:505, before **every** input (snapshots at about 10 Hz during playback, keys, chat, …).
- So `advance` runs **at least once a second, whatever the state**: visit or not, client idle or busy, setting on or off, resident or not. When she is Absent and the client isn't idle, `next_tick` returns None (mod.rs:744-748) and the 1 s hint governs.
- Its arms:
  - Absent: 703-712. Arrival gate: `open && now >= quiet_since + delay`.
  - Arriving: 713.
  - Visiting: 714-728.
  - Leaving: 729-735.
- `nudge.advance` runs first (702).
- `paint` (mod.rs:765) runs only when `draw` runs (shell.rs:617 after inputs; 489 when `advance` reports a change). `draw` takes its own `now_millis()` (shell.rs:633).
- `observe` (mod.rs:1302-1344), called from `paint`, latches `open`, `resident`, `delay` and `truecolor`. Before the first paint, `delay` is None.
- Gaps between calls can grow when the UI thread blocks in `actions.blocking_send` (shell.rs:543, 591) or on a slow draw. That is still time dessplay was open.

**Conclusion.** Accrual belongs at the top of `Guest::advance`, before the `match`. `paint` is wrong for accrual for two reasons:
- It runs only on draws.
- A ledger change between mod.rs:895 and 909 would make the `PartialEq` diff dirty the ledger on **every frame**, which is one sqlite write per paint.

## P3. Storing the game clock (proposed)

### Representation
- **(proposed)** `Ledger.clock: u64`: **game minutes elapsed since her first meeting**. It is not an absolute time of day, and never a monotonic `now`.
- Game time of day and weekday are computed in code as `START + clock`, where `START` (proposed) = 16:00 on a chosen weekday (day 0).
- Why "elapsed" rather than "minutes since midnight of day 0":
  - `Ledger::new` stays all-zero, so `moving_out_wipes_her_record…` (tests.rs:2259, `assert_eq!(ledger, Ledger::new(77))`) holds.
  - `skip_serializing_if = zero` keeps `the_record_as_written` and `missing_fields_default` unchanged.
  - A pre-5b record, or one an older build saved since, reads as clock 0, which is "16:00 on a weekday". That is the right meaning for "a new ledger", and also for an upgrade.
- **Invariant:** `START` must never change after it ships, because a change shifts every existing ledger's time of day. The same goes for the 6× ratio only if real ms were stored instead. Storing *game* minutes means retuning the ratio later never moves her past.
- Which weekday is day 0 is a design choice:
  - Monday 16:00 puts the first weekend about 4.7 game days in, about 18.7 real hours with dessplay open at 6×.
  - A later weekday reaches a weekend sooner.

### Accrual (proposed)
- New non-persisted fields on `Guest`, all reset by `move_out`:
  - `clock_at: Option<u64>`: monotonic ms of the last accrual. None until the first `advance`.
  - `clock_rem: u64`: real ms not yet worth a whole game minute.
  - `clock_unflushed: u64`: game minutes since the last handout.
- At the top of `advance(now)`:
  1. `let dt = now.saturating_sub(clock_at.replace(now)?)`. The first call only latches. Tests start at arbitrary `now`, and the shell's first call is at `now_millis()` of about 0.
  2. Clamp `dt` to `MAX_STEP` (proposed, e.g. 60 s real) against SIGSTOP or debugger jumps.
  3. `clock_rem += dt * 6`. Then `ledger.clock += clock_rem / 60_000` and `clock_rem %= 60_000`. At 6×, one game minute is 10 real seconds.
  4. Add the whole minutes to `clock_unflushed`.
- The sub-minute remainder is deliberately not persisted. At most 10 real seconds are lost per process, which is negligible, and it keeps the JSON free of changes below a minute.
- **Readers** (mind, routine, window, wall clock) read game time through one accessor, **(proposed)** `fn game_time(&self) -> GameTime`, which uses `ledger.clock` plus `clock_rem`. They never accrue. Readers in `paint` see time as of the last `advance`, at most about 1 s (6 game seconds) stale.
- **Trap: the clamp against test helpers.**
  - `tests.rs::run` (93-107) steps `next_tick` or `until - now` with no 1 s cap, so with nothing due she jumps straight to `until`.
  - Golden `drive` (golden.rs:~118-140) clamps steps to 1..1000 ms.
  - A routine test must step at most `MAX_STEP`, or set `guest.ledger.clock` directly (the fields are `pub(super)`).
- **Trap: deadlines in `next_tick`.**
  - Adding a per-game-minute or routine-boundary deadline to `next_tick` (mod.rs:742-763) changes `drive`'s step sizes, and so every per-frame golden hash.
  - The shell's at-least-1 Hz `advance` already crosses any boundary within 1 s, so a deadline is only needed for a repaint at second precision. A wall-clock piece needs at most one repaint per game minute (10 s real). That can be the `bool` `advance` returns when the minute rolls over, with no `next_tick` change.

### Gating: should the clock run before she's ever met, or with the setting Off? (open question)
1. **Always.** This matches the brief literally. But a time-based flush then writes a record for someone who never met her. That breaks the `Ok(None)` = "never visited" meaning (ledger.rs:176) and pins the per-run random master seed (shell.rs:438) before any visit. Her first meeting also lands at a random time of day.
2. **Only once `visits > 0`.** Her first meeting is then about 16:00 plus the idle delay (16:06 at the 1-minute default). No record is written for users who never met her.
3. **Only while the setting is on** (`self.delay.is_some()`, latched in `observe`, so time before the first paint is skipped). The house is "closed" while she's switched off.

I lean to 2. Option 3 can be combined with it; that is the user's call.

### Flush batching (proposed)
- There are two kinds of change:
  - **Time moved** (clock, pity accumulation): set `unsaved = true` only when `clock_unflushed >= FLUSH` (proposed: 30 game minutes = 5 real minutes, at most 12 writes an hour), and at routine-phase changes (leaves for school, goes to bed, …), so the record is consistent at each phase change.
  - **Events** (seen-set insert, pity reset on a rare, calendar date): dirty the ledger at once, as `record()` does today.
- `ledger_to_save()` clears `clock_unflushed` whenever it hands out. Any event save carries the clock for free, because the handout clones the whole ledger.
- The calendar date can be written inside `begin_visit`, which already dirties (mod.rs:1148-1149), so it costs no extra write.
- Worst-case loss on a crash or SIGHUP: `FLUSH` of game time, plus pity minutes. Both are harmless.
- Don't dirty the ledger per game minute: about 6 synchronous sqlite writes a real minute on the session's tokio loop (run.rs:1786).

## P4. Pity counters, seen rares, calendar date (proposed storage)
Append all new `Saved` fields **after `unsettled`**, each skipped when at its default:
- `clock: u64`: `skip_serializing_if` zero.
- `pity: Vec<(TierOrRare, u64)>` (or one `u64` if there is a single tier): game or present minutes since that tier last offered. Skipped when empty.
- `seen: Vec<RareId>`: skipped when empty.
- `calendar_on: Option<String>`: the real date `"YYYY-MM-DD"` of the last calendar content owed. `skip_serializing_if = "Option::is_none"`.
  - chrono has **no `serde` feature** (Cargo.toml:103: `features = ["clock"]`). Write a string, read it with `NaiveDate::parse_from_str`, and treat an unparsable value as None. This mirrors changelog.rs.

Why the order and the skipping matter:
- serde writes in declaration order. Appending after `unsettled` keeps `unsettled_pieces_are_written_and_read_back`'s `ends_with` (ledger.rs:475-477) and `the_record_as_written` (372) unchanged for `furnished()`, whose new fields are all zero or empty.

Reading:
- Copy the `unsettled` pattern (85-89): read `Vec<Value>` and filter each entry with `from_value(..).ok()`. A later build's rare id or tier is then skipped, not fatal.
- For new **scalars**, choose:
  - (a) `#[serde(default)] u64`, like `visits`: simple, but a later build changing the type makes the record unreadable, and she starts afresh and saves nothing.
  - (b) `Option<Value>` plus `.as_u64()`: fully lenient, consistent with "skip what this build doesn't know".
  - I'd take (b) for the new fields.
- Ids are serde enum variant names. Renaming a variant silently drops saved data. Don't use `#[serde(other)]`; the Value filter already skips unknown entries.

Accrual:
- Present minutes: the Visiting arm (mod.rs:714-728).
- Client-idle minutes: `self.open` (latched by `observe`) together with `now >= quiet_since`.
- Both use the same `dt` from P3. Keep a separate remainder per counter if its unit is not game minutes.

Reset rules for `move_out` (mod.rs:451-461): everything resets.
- `Ledger::new` zeroes the persisted fields, so the clock goes back to 16:00 on the weekday, pity to 0, `seen` to empty, and `calendar_on` to None. Calendar content is owed again the same real day, which is consistent with a first meeting.
- **(proposed)** also reset `clock_rem`, `clock_unflushed` and any in-memory routine or away state. Keep the `clock_at` latch.

## P5. Exit paths: the drain is skipped on every exit (map d14, confirmed, wider than d14)

The only drain is at the top of the loop (shell.rs:454-466). Every exit skips it:

| Exit | Site | Effect |
|---|---|---|
| Quit | shell.rs:589-594 | `blocking_send(Quit)` then `return`. A Bought or Unpacked event recorded by the `advance` at 505 in the same iteration is **lost now, before 5b**. |
| `SearchResults` send failure | 541-546 | `return` |
| `UiInput::Shutdown` | 508 | `break`. Sent by run.rs:994 / 1151 after Quit, Resync or Rejected; the channel is already closed by then. |
| Disconnected | 491 | `break` |
| Draw error | 489, 617 | `break` |

- `ledger_unsent` (446) is dropped on every one of these.
- On the run.rs side, the session's Quit arm (run.rs:1537) returns. Then `session.actions.close()` (1149) and `ui_thread.join()` (1152). Anything queued after Quit is never processed.

Options:
- **Q1 (minimal).** In the Quit arm, before `blocking_send(Quit)`, call **(proposed)** `guest.flush(now)`: accrue up to now, then force `unsaved`. Then `blocking_send(SaveHouseguest(ledger_unsent or the handout))`.
  - FIFO means run.rs saves it before handling Quit, in both loops (966 and 1786).
  - It covers Quit only. Resync, Rejected, Shutdown and draw errors still lose up to `FLUSH`.
- **Q2 (structural, recommended; proposed).** The final ledger becomes the UI thread's return value.
  - `run_ui_loop` returns `Option<Ledger>`; `run_ui_thread` (shell.rs:226) returns it through the `JoinHandle`. Its early returns at 249-250 and 261-263 happen before a guest exists and return None.
  - run.rs saves right after **both** joins:
    - run.rs:995 with `setup_storage`
    - run.rs:1152 with `session.storage`, **before** the `Rejected` return (1153-1155) and before Resync's `exec_self`. The settings DB is separate from the sync DB (`clear_sync_db`, run.rs:1232), so a resync keeps her record.
  - **Shape:** move the loop body into an inner fn (or a labeled block), so its `return`s and `break`s all fall through to one final `guest.final_ledger(now_millis())`. An exit can then never skip the last save.
  - **Trap:** the final value must **not** go through `ledger_to_save()`'s `unsaved` flag. A handout parked in `ledger_unsent`, or queued in the channel and then dropped by `close()`, has already cleared `unsaved`. Return `persist.then(|| ledger.clone())`, gated on "anything was dirtied or accrued this session" (proposed `touched: bool`). This keeps clients that never met her from writing a record on every exit, if gating option 2 or 3 applies.
  - A panic in the UI thread means `join` returns Err and the last changes are lost. That's acceptable.
  - Signature ripple: `dessplay-rendezvous/tests/perf.rs:426` (closure returns `()`) and `:608` (closure returns `adapter`). Both are statement calls, so a new return value is simply dropped. Keep `run_ui_loop` free of `#[must_use]`, or update both sites.

## P6. Tests and goldens that a persisted clock touches
- **Goldens.**
  - `drive` (golden.rs:~118-140) calls `advance` every step, and `Trace::finish` (golden.rs:81-97) hashes `guest.ledger.to_json()` after the run (stage 300 s, resident 180 s, … that is 18-30 game minutes).
  - Any accruing serialized field moves **every** `finish` hash, even with skip-if-zero.
  - Recommendation: the commit that adds the clock re-records the tables (golden.rs:341-386, printed by `check`). Prove behaviour is unchanged with `HOUSEGUEST_GOLDEN_TRACE=<dir>` before and after: frame lines identical, only the trailing JSON line differs.
  - Don't hash a clock-zeroed JSON in `finish`; that would hide a regression in what's saved.
  - Routine wiring changes frames once a boundary falls inside a golden. From 16:00, 30 game minutes probably don't reach one, but check.
- **`a_goodbye_mid_carry_leaves_the_piece_where_it_stood`** (tests.rs:7358-7395) compares `record_of(&guest)` (7350-7352, `ledger.to_json()`) before and after `advance` plus `run(.., now + DURATION_MS)`. It **fails** once the clock is serialized and accrues. Its intent is "her home is as it was": compare `ledger.home` and `ordered` instead, or zero the clock in `record_of`.
- Other tests that restore through JSON (`her_home_outlives_a_restart` 2213-2242, `her_home_fills_up_over_visits` 2472-2488, tests.rs:2668) compare fields, not whole ledgers, and survive. census.rs only reads `ledger.ordered` (census.rs:463).
- `moving_out_wipes…` (2246-2268) has no `advance` between `move_out` and `ledger_to_save`, so it holds if `move_out` resets the clock fields (P4).
- **(proposed) new tests:**
  - A property: accrual is independent of tick partition. The same total `dt` split any way, each piece at most `MAX_STEP`, gives the same `clock` and `clock_rem`.
  - The first `advance` doesn't accrue.
  - `FLUSH` batching: no `ledger_to_save` before the threshold, one at it, and an event handout resets the batch.
  - Q2: every exit path returns the final ledger, including one parked in `ledger_unsent`.
  - The record as written with new fields at their defaults is unchanged.
  - An older build reads a 5b record (`as_an_older_build_reads`, ledger.rs:343) and keeps every piece.
  - Unknown `seen` and `pity` entries are skipped.
  - A garbage `calendar_on` reads as None.

## P7. Downgrades (map d13)
- `Raw` ignores unknown fields, so a `stable`-track build reads a 5b record fine, and on its next save **drops** `clock`, `pity`, `seen` and `calendar_on`.
- When the newer build reads that back:
  - the clock is back at 0, so START (16:00 on the weekday)
  - pity is 0, so she waits longer for a rare
  - `seen` is empty, so a rare can count as "unseen" again
  - the calendar is owed again that real day
- Nothing breaks, and no build can prevent it short of a version bump, which would make 5b records unreadable to older builds.
- **Docs:** design.md:1906-1917 says "saved whenever it changes". Amend it to "events at once, the clock in batches and on exit". decisions.md:2072-2090 needs the batching and downgrade rationale, and the stale `houseguest` key fixed to `houseguest_ledger`.

## P8. Stage and examples
- examples/houseguest.rs:94-102 feeds `advance` a speed-scaled `now` (`SPEEDS[speed]`), so the clock scales with stage speed. It uses `Guest::new` per seed, so the clock starts at 16:00.
- Reviewing the window and wall-clock art needs **(proposed)** a stage cue to jump game time (e.g. +1 h, or "set 07:00"), in the stage cue set (`stage::Scene`, mod.rs:497-537). It writes `ledger.clock` directly and marks the ledger dirty.

## P9. Invariants for the builder
1. Accrual happens only in `advance`, from monotonic `now` deltas. Never in `paint`, and never across the furnish diff (mod.rs:895-911).
2. Clock and calendar code never draws from `self.rng` or the mind's streams. Rolls are visit seed plus a salt (A6).
3. New fields: additive, appended after `unsettled`, skipped at default, read leniently. `VERSION` stays 1.
4. The `LocalTime` real date never reaches the persisted clock. The clock never resyncs to the wall clock.
5. `persist == false` (`keep_unsaved`): the clock runs in memory and nothing is ever handed out, including the Q2 final ledger.
6. `move_out` resets every persisted and per-ledger in-memory 5b field.

---

<!-- section: mind-offering -->

# Code map: the mind (brain.rs, mind.rs, osaka.rs `choose_next`) for 5b

Every ref below was re-read against the current tree (after 5a and the image cache). Paths are under `dessplay/src/ui/houseguest/`. Anything marked **(proposed)** does not exist yet.

## 1. What exists today

### brain.rs
- **`Need`** (20-45), with ten needs. `Need::ALL` is at 48-60.
- **`rise_ms`** (65-79) gives the real ms each need takes to rise from 0 to 1: Sleepy 15 min, Restless 90 s, Tidy 60 s, Mischief 4 min, Hungry 20 min, Comfort 8 min, Fun 6 min, Daydreams 10 min, Nesting 3 min, Beauty 20 min.
- **`arriving`** (85-93) sets the levels at arrival: Sleepy 0, Restless 0.7, Hungry 0.2, and so on.
- **`Mood`** (114-123). `Mood::of(seed)` (131-141) is a splitmix of the visit seed with salt `0x6d6f_6f64`. `Mood::rate(need)` (144-160) is a per-visit multiplier and is 1.0 by default. `home_acts` is at 164-170.
- **`quality(need, spot)`** (208-217). `FUN_SOURCES` and its tolerance are at 221-232.
- **`Rising { mess, grieved, plain }`** (237-246) says which conditional needs rise.
- **`Needs::pass(ms, rising, mood)`** (293-309) adds `ms * mood.rate(need) / rise_ms` to each need, then decays tolerance, then clamps.
  - Its **only caller** is osaka.rs:2981-2982.
  - `Needs::default()` (265-272) uses `arriving`. Its only non-test caller is osaka.rs:1232.
- **`Want`** (358-378). `Want::ALL: [Want; 26]` (422-449) is the order she considers wants in, which is also the order offers are built.
- **`Factor`** (382-386) has one variant, `InChat(f64)`.
- **`DesireDef { base, serves, own_sake, factors }`** (391-399).
  - The rows are in `Want::def` (452-516).
  - `row()` (404-411) sets `factors: &[]`.
  - **`in_chat(def)` (413-418) overwrites `factors` with `IN_CHAT`.** It does not append to them.
- **`score`** (535-559) is `base × fit × COOLDOWN^repeats`, with `COOLDOWN` = 0.4 and `RECENT` = 3 (osaka.rs:311).
- **`choose`** (565-599):
  - each offer scores `score × factor(want)` (576);
  - offers are sorted, then `truncate(TOP=4)` (579);
  - one roll is made with `whims.below_at("roll", attempt, 1000)` (585).
  - **A factor below 1 is effectively removal at home** (map A6 still holds).
- **Tests:**
  - `every_want_has_a_sane_row` (804-820) uses **`for &Factor::InChat(times) in def.factors`** (811). That pattern becomes refutable, and stops compiling, once `Factor` has a second variant. The test also asserts `times < 1`.
  - `all_lists_every_want` (770-799) assigns each want a kind from 0 to 10, and checks every kind is present.

### mind.rs
- **`Whims(u64)`** (34-76) hashes FNV-1a of its label, then a splitmix with `salt * golden` (37-49). It offers `below`, `below_at`, `chance` and `odds`.
  - A label that repeats within one decision gives the same number.
- **`Ctx<'a>`** (79-100) holds `x, y, here, links, terrain, chances, may_work, owes, episode, just_set, may_arrange`. It has no time, tier or rarity input.
  - Its **one literal** is osaka.rs:3008-3028.
- **`methods(want)`** (195-211). `USE` (186-192) is shared by every `Want::Use(_)` except Crumple, which has no methods.
  - A per-use time gate therefore can't live in the `USE` guards without matching on `want` inside them.
- **`bind(ctx, whims, want)`** (232-241) returns the first guard that binds; a job must also stand on a floor. It has three callers:
  - the offer list, osaka.rs:3096;
  - `arrange_next`, osaka.rs:3623 and 3716 (Arrange only).

  The stage does **not** call `bind`: cues bypass the mind.
- **`work`** (294-308) is gated only by `ctx.may_work`.
- **`Lines`** (862-926) is per visit, created by `Default` at osaka.rs:1252. `try_play(script, at)` (916-925) applies a 10-minute per-visit script cooldown (`SCRIPT_COOLDOWN_MS`, 737). Nothing persists across visits.

### osaka.rs
- **`Chances`** (19-53) derives `Default` (18).
  - The two production literals list every field: mod.rs:1007 (the stage's `offered`) and mod.rs:1042.
  - The other literals all end in `..Chances::default()`: osaka.rs:5423, 5706, 5897, 5903 and tests.rs:4764, 4771.
- **`HomeEvent`** (217-230) carries `Bought`, `Unpacked`, `Crumpled`, `Used` and `SetDown`.
  - `record` (mod.rs:1350-1383) consumes them on the same tick and returns `changed`, which sets `unsaved` (mod.rs:727, 896).
- **`Bucket`** (589-595) has `Reflex`, `Owed`, `Continuation` and `Normal`. `Decision { at, bucket, method, want, top, act, heading }` is at 604-616.
- **Constants:** `WORK_AFTER_MS` = 3 min (882), `SHIFT_MS` = 1–3 min (884). `use_duration` (915-927) gives Sleep 60–180 s and Homework or Nap 30–60 s.
- **Fields on `Osaka`:** `mood` (1116), `recent` (1118), `decided` (1120), `mind: Rng` (1135), `whims` (1140), `worked` (1113), `home_acts` (1180).
- **`Osaka::new`** (1210-1276):
  - `needs: Needs::default()` (1232);
  - `mind: Rng(rng.next() ^ MIND_SALT)` (1240), which is one body draw;
  - `whims = Whims(mind.0 ^ WHIMS_SALT)` (1273).
- **`choose_next`** (2933-3179), in order:
  1. 2940: `Whims(self.mind.next())`, **the only mind-stream draw**.
  2. 2943: `credit_done(at)`.
  3. 2945-2948: errand **reflex**.
  4. 2949-2970: the "no floor", "off text" and "watching chat" reflexes.
  5. 2971-2974: greeting.
  6. 2976-2983: `Rising` and `needs.pass(at - decided, …)`.
  7. 2987-3007: owed beat.
  8. 3008-3028: the `Ctx` literal.
  9. 3033-3061: heading continuation (hop or mine).
  10. 3064-3078: `arrange_next`.
  11. 3081-3090: `leftover`.
  12. **3093-3098: the offer filter**, `Want::ALL.filter_map(bind)`.
  13. **3103-3112: heading re-offer.** The heading's want is pushed with `Bind::Job` *without* `bind`, and gets `INERTIA` ×3.
  14. **3119-3138: the factor closure.** It is an exhaustive `match` on `Factor` (3132-3135), so a new variant fails to compile here.
  15. 3139-3140: `brain::choose`.
  16. 3149-3164: `plan`, `recent.push` and `credit`.
- **Needs handling:**
  - `credit_done` (2539-2591) credits the share of the body that was done.
  - `serve` (2603-2615).
- **Other sites:**
  - `start_job` (2331). It calls `script::splices` (2377-2385) and makes the length draw `rng.range(lo, hi)` at 2431.
  - `to_mend` (3743), `set_mood` (3782-3785), `interrupt(cause, now)` (3940), `head_for_errand` (4218), `go_to_work` (4358-4377).

### Elsewhere
- **The visit seed.** `Ledger::visit_seed(v) = master_seed ^ v*golden` (ledger.rs:59-61).
  - The **body** `Rng` is seeded with the *raw* visit seed (mod.rs:414, 776, 1267).
  - `cat_home` reads a raw bit, `>>17 & 1` (mod.rs:2245).
  - `Mood::of` hashes the seed first.
- **The mood is set** in `begin_visit` (mod.rs:1147), at 1152-1154, via `osaka.set_mood(Mood::of(visit_seed(visits-1)))`.
- **`advert(ledger, shop_now, osaka.needs())`** (mod.rs:125-137; called at 1018 and 1053) sells decor iff Beauty is `pressing`. Time-shaped needs, such as sleepy at night, therefore change what the channel sells.
- **Splices.**
  - `SpliceCtx { what, trying, quiet }` (script.rs:604-613) is `Copy`.
  - In `splices()` (script.rs:663), the candidate loop (681-698) checks `row.when(ctx)` and then `whims.chance(SPLICE, 2*row.salt, n, d)` (695).

## 2. Time arithmetic at 6×

- One real minute is 6 game minutes, a game hour is 10 real minutes, and **a game day is 4 real hours**.
- A new ledger starts at 16:00. A 20-minute real visit covers about 2 game hours.
- **The goldens run at most 600 s real** (golden.rs: stage 300 s, resident 180 s, a 600 s scene), which is **16:00–17:00 game** on a fresh ledger.
- Window lengths in real time:

| Window (HG table) | Game hours | Real minutes |
|---|---|---|
| sleep, 22:30–07:00 | 8.5 | 85 |
| homework, 20:00–22:30 | 2.5 | 25 |
| school, 08:30–15:30 | 7 | 70 |
| home-and-snack, 15:30–18:00 | 2.5 | 25 |

- Needs rise in **real** ms. Sleepy fills in 15 real minutes, which is 1.5 game hours, so without time shaping she gets sleepy about 16 times a game day. Time has to shape the *rates*, not only the offers.

## 3. How game time enters the mind

**The seam: `Chances.clock: Option<DayTime>` (proposed)**, read as `ctx.chances.clock`. `Ctx` keeps its single literal and needs no new field.
- `DayTime { minute: u16 /* 0..1440 */, school_day: bool }` (proposed). The game weekday comes from the clock, not the real date.
- `None` means the clock is disabled, and every path must then be byte-identical to today.
- **Wiring:**
  - Add the field to the two full literals (mod.rs:1007, 1042).
  - Every other literal picks up `None` from `Default`.
  - `Needs::pass` and arrival levels are outside `Ctx`. Osaka reads `chances.clock` at 2976-2983, which already has `chances`.
- **The real date stays separate** (`LocalTime` on `IdleView`, another agent's area). The mind must never read real time-of-day, only the real *date*, for season factors.

There are four levers. Use each for one job only.

**(a) Boosts: `Factor::Clock(Window, f64)` (proposed), times > 1 only.**
- Evaluate it in the closure at 3132-3135: `Factor::Clock(w, t) if clock.is_some_and(|c| w.holds(c)) => t`, and `_ => 1.0` otherwise.
- `Factor::Season(DateWindow, f64)` (proposed) is the same idea on the real date.
- Make `Window { from, to, days }` (proposed) a const struct whose `holds` handles wrapping past midnight.
- Fix brain.rs:811 to a `match`, and assert `> 1` for `Clock` and `Season`.
- **Trap:** `in_chat()` replaces `factors`, so a row with both a chat factor and a clock factor needs its own `const` slice, e.g. `&[InChat(CHAT_FACTOR), Clock(EVENING, 3.0)]`. `DesireDef { factors: X, ..in_chat(row(..)) }` silently drops `InChat`.

**(b) Gates: `DesireDef.open: Option<Window>` (proposed), for "not at this time".**
- Check it at the top of `mind::bind` (232) as `if !want.def().open_at(ctx.chances.clock) { return None }`. This keeps the rule that a want is offered iff it binds.
- `None` for either the row's window or the clock means open.
- These paths **bypass** the gate, deliberately; say so in the doc:
  - the heading re-offer (3103-3112): a homework heading set at 22:29 survives 22:31, which counts as commitment;
  - `leftover` (3081): a made piece's purpose;
  - `arrange_next` (3623, 3716): Arrange is never gated;
  - stage cues and the errand.
- If a closed window should drop a heading, apply the same predicate at 3104 and `drop_heading(Letting::Gone)`.

**(c) Need rates: `pass(ms, rising, mood, clock)` (proposed).**
- Multiply by a `clock_rate(need, DayTime)` (proposed), which is exactly `1.0` for `None`.
  - Sleepy: about ×0.3 from 07:00 to 20:00, ×1 from 20:00 to 22:30, about ×3 after that.
  - Hungry: higher before 07:00, 12:00 and 18:00, which is how "meals" emerge without a meal want.
- `pass` covers `at - decided`, which can be up to about 3 real minutes (18 game minutes). The rate at `at` is good enough; integrating across a band edge is optional.

**(d) Arrival levels: `osaka.set_clock(DayTime)` (proposed), called in `begin_visit` beside `set_mood` (mod.rs:1152).**
- It re-seats the `arriving` levels by time: arriving at 23:00 she comes in sleepy.
- **16:00 must give today's exact `arriving` values.**

**What a window cannot do.** Nothing wakes a decision at a band edge: acts run to their end. The longest is a sleep of 3 min real, which is 18 game minutes of lag. If an edge must cut an act, `interrupt(Cause::…, now)` (3940) is the hook, with a new `Cause` (proposed).

## 4. Obligations are reflexes, not wants

Anything that **must** happen at a time (bedtime, leaving for school, coming in on a dash-in) cannot go through `choose`:
- a top-4 weighted roll never guarantees a pick;
- `COOLDOWN` 0.4^repeats penalises choosing Sleep again and again through a night.

**Precedent:** the errand reflex at 2945-2948, which calls `head_for_errand` (4218) and returns `Decision::reflex("errand")`.

**Proposed:** a `routine` step in the `Reflex` bucket, after the "watching chat" reflex (2970) and before the owed beat.
- It returns `Decision::reflex("routine/bed")` or `"routine/school"`.
- At bed it calls `mind::bind(&ctx, whims, Want::Use(Use::Sleep))`, falling back to `Use(Nap)`, then `Idle(LieBack)`, then `plan`.
- `plan` can't run before the `Ctx` exists, so in practice the reflex sits right after the `Ctx` literal (3028), before the heading continuation.

**Rejected alternative: at night, gate every want closed except sleep-serving ones.** `Stand` always binds and is the filler (4.0 × neutral fit), so she would sometimes stand at 02:00. The `recent` cooldown also thins repeated Sleep choices.

**A night's sleep as one act.** In `start_job` the length comes from `use_duration` and the draw at 2431.
- At night, set `until` from the clock: game minutes to 07:00 × 10 000 ms. This is at most 85 min real.
- **Keep exactly one `rng.range` draw**: draw, then override or use it as jitter (map c3).

**Work.**
- `Want::Work` is today's in-visit 1–3 min shift, gated by `may_work` (3020-3023, recomputed at 3074-3077).
- Options:
  - (i) keep it as the part-time job and give it an `open` window, e.g. after school or at weekends;
  - (ii) fold it into the routine reflex.
- School absence is a guest-gate matter (another agent's area). On the mind's side, "off to school" is a reflex reusing `go_to_work`'s exit (4358-4377).

## 5. Rarity, pity and "seen"

**Where rares live.** No want is rare today. Rare content is scripts, splices and calendar extras. So:
- put the tier on **`SpliceRow`/`ScriptId` first**;
- add it to `DesireDef` only if a whole want becomes rare (`tier: Tier`, defaulting to `Common` in `row()`, proposed).
- Calendar content **bypasses** rarity (HG: guaranteed once on the first visit of the day). It is an owed or reflex item, not part of `Rares`.

**The roll source.** Roll **once per visit** in `begin_visit`:
- use the visit seed through a splitmix with a new salt (like `Mood::of`, brain.rs:131-141);
- **never** read raw seed bits: the body `Rng` *is* the raw visit seed (mod.rs:414 and the others), and `cat_home` already reads a raw bit;
- never draw from `mind.next()` or the body `rng`;
- snapshot the pity counters at `begin_visit`, so the roll can't move mid-visit.

Per decision would instead be a `Whims` label `"rare"` with the id as salt. Its rate would then scale with the number of decisions in a visit (busy visits would get more rares), which is harder to tune and to bound with pity. Rejected.

**At most one unseen rare a visit, structurally: `Rares` (proposed).**
```rust
pub(super) struct Rares { open: Vec<RareId>, new: Option<RareId> }   // private fields
impl Rares {
    pub fn draw(seed: u64, seen: &[RareId], pity: Pity) -> Self;      // the only constructor
    pub fn none() -> Self;                                            // clock off / tests: nothing rare
    pub fn open(&self, id: RareId) -> bool;
}
```
- `open` holds the *seen* rares whose tier and pity roll succeeded this visit.
- `new` holds at most one *unseen* rare, picked among the unseen with pity weighting. A single `Option` makes "two unseen rares in a visit" unrepresentable.
- Seeing `new` mid-visit only moves it into the ledger's seen set; nothing else unlocks.
- Store it on `Osaka` beside `mood` through `set_rares` (proposed, in `begin_visit`).
- **Gates:**
  - for splices, skip a closed row in the candidate loop (script.rs:681-698) when `forced.is_none()`, so cues still force it. Pass `open: bool` or a `&Rares` in `SpliceCtx`; a reference field needs a lifetime but stays `Copy`;
  - for wants, use the same `mind::bind` pre-check as the windows in 3(b).
- **Pity**, e.g. `p = base(tier) + k × idle_minutes_since_any_rare`, capped. The counters are the ledger agent's area; the mind sees only the drawn `Rares`.

**What "seen" means (recommended): first shown.** That is when its script starts (a splice's first key, or a script's start), not when it was planned or offered:
- a walk to it can be interrupted, and an interrupted rare stays unseen and keeps its slot;
- record it with `HomeEvent::Seen(ScriptId)` (proposed), pushed where the script starts (the splice path near 2377-2385);
- `record` (mod.rs:1350) inserts it into the ledger's seen set (a stable string id, map c10) and returns `changed`. That is the same tick and the same deterministic path as `Bought` and `Used`.

## 6. Every want, and whether time should touch it

| Want | Gate / boost | Window (game) | Why |
|---|---|---|---|
| Stand | never | — | The filler. "Nothing bound" must stay possible. |
| SpaceOut | boost | late evening | Fine at any time. Musing is her signature. |
| Sneeze | season boost (real date) | Mar–Apr hay fever | Not the clock. |
| Idle Sit / LieBack | none (needs do it) | — | Sleepy already pulls LieBack. |
| Idle LieFront | boost | afternoon | Kicking her feet after school. |
| Idle Jacks / ToeTouch | gate closed | 22:30–07:00 | Exercise at 02:00 reads wrong. |
| Idle Stretch | boost | 07:00–09:00 | Waking up. |
| Idle Gaze | boost (×more with a window piece) | dusk / night | The window piece makes time legible. |
| Walk, Travel | never | — | Movement is how every job is reached. |
| Pull, Swap | none | — | Tidiness and mischief follow text, not time. |
| Work | gate | its shift window, or a reflex | §4, options (i) and (ii). |
| Use Lounge / Nap | boost | 15:30–18:00 | HG: "flops on sofa". |
| Use Sleep | reflex at night; open by day | §4 | Bedtime must happen. A daytime bed nap follows sleepy. |
| Use Homework | gate on school days 20:00–22:30, or boost ×3 there | §7 option | HG. A gate also removes chopsticks outside it (5a splice). |
| Use Watch | boost | 18:00–20:00 | HG: dinner and TV. |
| Use Read | boost on weekends | evening | HG: weekend evenings. |
| Use Snack | boost (and Hungry rate) | 15:30–18:00 | HG: after-school snack. Meals come from Hungry's rate. |
| Use Pet | none | — | The cat ignores the clock (the plan says so too). |
| Use Unpack | never | — | A parcel beats everything (own_sake, base 40). |
| Use Crumple | never | — | It has no methods; it is a step of a purpose. |
| Arrange | never | — | Episodes re-bind it directly (3623, 3716). Gating would strand a piece in her pocket. |

## 7. Determinism and the goldens

- **Exact neutrality.** `x * 1.0 == x` in f64, so a factor or rate of exactly `1.0`, a `None` clock, `Rares::none()` and closed-nothing windows leave scores, rolls and frames bit-identical.
  - Appending a want to `Want::ALL` that never binds while the clock is off also changes nothing: `sort_by` is stable and a non-binding want never enters the offers. It does change the array length and needs a kind arm in `all_lists_every_want`.
- **No new draws.**
  - Rolls happen per visit (seed + salt) or as `Whims` labels with **new** label names.
  - There is never a second `mind.next()`, and never an added or removed body `rng` draw (map c3).
  - The night-sleep length keeps its single 2431 draw.
- **Goldens.** `finish` (golden.rs:83-84) hashes `ledger.to_json()`, so once the clock accrues in the ledger every golden moves, whatever the mind does. Hence:
  1. Land the mind plumbing (`Chances.clock`, `Factor::Clock`, windows, `Rares`, `pass` and arrival hooks) **unfed**. Goldens stay unchanged; that is the proof the plumbing is inert.
  2. The ledger/clock commit re-records once. Diff the per-frame traces (`HOUSEGUEST_GOLDEN_TRACE=<dir>`) to show the frames didn't move, only the ledger.
  3. If the 16:00–17:00 band is all exactly 1.0, frames stay identical even when fed, as a second line of defence. But HG wants snack and sofa boosts from 15:30. **The choice:** start the boosts at 17:00 and keep frames identical, or re-record the goldens deliberately.
  4. Add new golden scenes with the clock fed at 20:30 (homework window) and 23:00 (bedtime reflex, long sleep).
- **Censuses.** `visit_census` and `sofa_census` (the plan.md tables) start at 16:00, and a persistent census ledger would roll the clock on across visits. They need a clock-off mode to stay comparable, plus a per-band census for 5b's own numbers.
- **The stage.** It needs a clock control (proposed) to cue a time. Cues bypass `bind`, so `every_want_can_be_cued` (tests.rs:5417) is unaffected by gates.

## 8. Traps

1. **A long sleep wakes her maximally sleepy.**
   - `credit_done` (2943) runs before `needs.pass` (2981-2982) over the whole gap since `decided`.
   - After an 85-minute sleep she is credited −0.7 sleepy, then 85 minutes of rise adds +1.0 (clamped), and Hungry, Restless and the rest all reach 1.
   - It doesn't show today only because sleeps last 1–3 minutes.
   - **Fix:** `Rising.asleep_ms: u64` (proposed). Sleepy doesn't rise for the time asleep, and other needs maybe rise at a lower rate. When it is 0, behaviour is identical.
   - **Do not reorder** credit and pass: clamping makes the order observable and would move the goldens.
2. **brain.rs:811's refutable `for` pattern** fails to compile when `Factor` gets a second variant. Its `< 1` assertion would also reject boosts.
3. **`in_chat()` overwrites `factors`** (brain.rs:413-418). See 3(a).
4. **A factor below 1 is not rarity.** `choose` truncates to the top 4 after factors (579). Use gates for "not now" and `Rares` for "rare"; never a dampening factor.
5. **The heading re-offer** (3103-3112) skips `bind`, so it skips any gate there. Decide deliberately (§3(b)).
6. **The body `Rng` is the raw visit seed.** Any new per-visit roll must hash with its own salt, as `Mood::of` does, not read raw bits as `cat_home` does.
7. **The mood and clock rates multiply.** A lazy night has Sleepy at ×1.5 × ×3. Bound the product, or the needs saturate within a minute.
8. **Window lag.** No decision happens at a band edge (§3), so a reflex fires at the first decision after it, up to about 3 min real late. That is acceptable, or use `interrupt`.
9. **Shopping drift.** Time-shaped needs change which need is `pressing`, so `advert` (mod.rs:133) sells furniture rather than decor at night.
10. **Logs.** Factor effects show only inside `Decision.top` scores. Adding the `DayTime` to `Decision` (proposed) keeps the explain log answering "why now".

## 9. Recommended shape, in commit order

1. **Plumbing, inert.** `Chances.clock: Option<DayTime>`, `Factor::Clock` and `Factor::Season` (boosts only, with the brain.rs:811 fix), `DesireDef.open` checked in `bind`, and the `pass` and arrival hooks, all neutral at `None`. Goldens unchanged.
2. **`Rares`.** Drawn per visit from seed + salt, with at most one unseen, gating splices (script.rs:681-698) and gated wants. Add `HomeEvent::Seen`.
3. **The routine reflex** (bed, school) and the night-long sleep with `Rising.asleep_ms`.
4. **Feed the clock** (ledger agent). Re-record the goldens once, after trace-diffing, and add the clock-fed golden scenes.

**Open choices for the user:**
- whether homework is gated to its window or only boosted there;
- whether Work stays a want with a window or becomes a reflex;
- whether the boosts start at 17:00 so the frames stay identical in the 16:00 band.

---

<!-- section: scripts-calendar -->

# 5b code map: scripts, splices, line pools, and how calendar content can use them

All refs were re-read on 2026-10-04 against `dessplay/src/ui/houseguest/`. Unprefixed `NNNN` refs are `osaka.rs`. "(proposed)" marks something that doesn't exist yet.

## 1. What exists

### 1.1 Script data (script.rs)
- **`Span`** (18-39): `Ms(u64)` cumulative from the part's start, `Upto(n,d)` a share of the body, or `Rest`. `Span::end(body: Option<u64>)`: with `None` (no set length), only `Ms` ends.
- **`Posed`** (43-56): `Host`, `Still(Pose)`, or `Bob(fn(u8)->Pose, period)`. A bob counts from the part's start.
- **`Say`** (60-70): `Bubble(Bubble)`, `Pitch` (`play.bought`'s pitch), `Riddle`/`Answer` (`RIDDLES[play.drawn[0]]`). There is no generic "drawn line" slot. Fixed lines go in `Bubble::Say(&'static str)`, written with `line!`, which fails to compile above 24 chars (mod.rs:18 `BUBBLE_CHARS`, 35-61).
- **`Prop`** (75-85): `Tv(Channel)`, `LampOff`, `FridgeOpen`, `CatBiting`.
  - `Prop::item` (92-99) applies a prop to **every shown piece of that kind**.
  - `framed` (110-119): only Snow and Shopping animate.
- **`Key`** (124-130): `{span, pose, face: Face, say, prop}`. `Key::look` (136-157) turns `Say` into a `Bubble`.
- **Player:**
  - `at` (166-175) is the generic cumulative-end finder; `door_beat` uses it too (osaka 816-827).
  - `key_at` (180-186) holds the last key past the body's end.
- **`ScriptId`** (196-227) has 15 scripts, each with branches.
  - Wildcard-free tables, which make up the **per-script ceremony**: `ALL` (231), `branches` (250), `on_chat` (276), `trial_branch` (299), `played_on` (323), `host` (341, test), `shortest_body` (367, test), `scene` (389, test; needs a `stage::Scene`), and `every_script_is_listed`'s `player` (1398-1470).
  - Branch choice: `keys(branch)` (412) falls back to branch 0.
- **`SLEEP`** (968-991):
  - Branch 0: lamp on for `LAMP_ON_MS` = 2000 ms (Dots), then `LampOff` + Zzz. This is the #70 "bedtime" the census counts.
  - Branch 1: `LampOff` from the first frame (the trial sit, via `trial_branch`).
- **Other scripts:**
  - `SURF` (1028-1064) uses `Channel::{Snow, ColourBars, Sunrise}`. The art for all three exists.
  - `SNACK` key 0 = `Still(Pose::Side)` + `FridgeOpen` for 1500 ms (1099-1113).
  - `RIDDLE` (1164-1183): three `Still(Stand)` keys.
  - `CHOPSTICKS` (1202-1255) and `ANDAGI` (1280-1350) are const-built.

### 1.2 Play and splices (script.rs)
- **`Play`** (740-747): `{own, branch, before, after, drawn:[u8;2], bought}`.
  - Constructors: `plain` (751), `riddle` (763), `of(what, bought)` (772-781; the shopping channel if something was bought).
  - Parts and timing: `body_start` and `body_end` (784-794). `part` (833-847) times each part from its own start. `next_end` (800-817), `spliced_at` (821-827), `note` (862-892) and `next_frame` (898-902) build on them.
- **Splice rows:** `SpliceId` (449-468, plus cfg(test) rows) → `Splice` (580-601): `{name, salt, around, at: Part, chance, when: fn(&SpliceCtx)->bool, lens}`.
  - `SpliceCtx` (605-613) is `{what, trying, quiet}`. **It carries no time or date.**
  - `splices()` (663-716): at most one before and one after. A prelude only when quiet. The chance is rolled before `try_play` (a non-roll doesn't cool). The branch comes from `whims.below_at("splice", 2*salt+1, …)`. A forced (cued) splice skips the chance and the `when` check, but still calls `try_play`.
- **`Cue`** (621-631): `Script(ScriptId)` or `Splice(SpliceId, Option<u8>)`.
  - `plays_on` (640-649) is true for any `Script(id)` whose `played_on() == Some(what)`. Surf and Shopping are excluded when she's grieved.
  - There is one slot: `Osaka.cued` (1144), set by `Osaka::cue` (1434-1436). It is taken in `start_job` (2360-2362) or in `muse` (a riddle, 1578-1582).

### 1.3 Hosts
- **`Act::Use`** (471-482), built only in `start_job`'s `Job::Use` arm (2331-2455):
  1. trial `HMM`, then grievance (2350-2357)
  2. cue taken (2360-2362)
  3. `script::splices` (2377-2385)
  4. hush for a cued prelude (2389-2392)
  5. advert → `bought` (2400), then surf (2406-2418)
  6. `HomeEvent::{Bought, Used}` (2422-2430)
  7. **own-script choice** (2438-2439: `own = if surf { Surf } else { plain.own }`)
  8. `until = body_start + length + after.len` (2450)
- **`Act::Use` at runtime:**
  - Wakeups: on the frame grid, at key ends and at grievance edges (`first_due` 1502-1516).
  - End (1851-1866): Unpack/Crumple events; Crumple chains to `Admire` (1864); otherwise `decide`.
  - Props: `Osaka::prop` (2761-2771) reads **only `Act::Use`**.
- **`Act::SpaceOut { since, until, play: Option<Play> }`** (338-342):
  - Started by `plan`: `Here::SpaceOut` (3196-3200) has `play: None`; `Here::Muse` calls `muse` (3201-3204).
  - `muse` (1576-1620): a riddle (`RIDDLE` pool 1-in-3, only when quiet) or a musing via `self.say`. The length is drawn from the **body rng**, `SPACE_OUT_MS` = 6-14 s (318).
  - Look (4474-4482): `play.key(...)` with host pose `Stand`.
  - Wakeups (1492-1495): **key ends only, no frame grid**. So the lint `spacing_out_neither_bobs_nor_shows_a_prop` (script.rs:1518-1528) forbids `Bob` and props on SpaceOut-hosted scripts.
  - **Legal today:** alternating `Still(Pose::X(0))`/`Still(Pose::X(1))` keys at `Ms` spans do animate, because every key end is a wakeup.

### 1.4 Line pools (mind.rs)
- **`Pool`** (829-834): `{id: PoolId, lines, n, d}`.
- **`PoolId`** (743-758): `Beat` (0), `Door` (1), `Musing` (2), `Riddle` (3), `Test` (MAX). Its wildcard-free tables:
  - `ALL` (763)
  - `id` (772-782; the stable salt)
  - `budgeted` (785-792; **only `Beat`**)
  - `listed` (798-824; feeds `all_lines`)
- **`Lines`** (862-865): `said: Vec<(PoolId, &str, u64)>` and `played: Vec<(ScriptId, u64)>`.
  - `pick` (872-892): the budget check applies to Beat only, then `chance("line", id, n, d)`, then cooldown filtering. The cooldown matches **by text across pools** (885), so a calendar line identical to a musing shares its cooldown. Then `below_at("which-line", id, fresh)`.
  - Map trap d7 is **fixed**: both rolls are salted by `pool.id.id()` (874, 888).
  - `note` (897) and `unsay` (903) handle a line spoken over in the same instant (called from `hush` 1564-1570).
  - `try_play` (916-925) implements `SCRIPT_COOLDOWN_MS` = 10 min (737).
- **Constants:** `LINE_COOLDOWN_MS` = 10 min (733), `LINE_BUDGET` = 8 Beat lines a visit (735).
- **Pools:**
  - `DOOR` (648-659), 1 in 3, five lines.
  - `MUSINGS` (662-679), 1 in 1, eleven lines.
  - `RIDDLES` (683-690); `RIDDLE` pool (713-718), 1 in 3.
- **Callers:**
  - The door line is said in `fire`'s `Act::Door` end (1804-1806), only when the door wasn't an errand or a return from work.
  - The musing is in `muse` (1606).
  - Beats are in `choose_next` (2992-2996).

### 1.5 The once-a-visit line: the greeting
- `choose_next` (2971-2974): `if !self.greeted { greeted = true; say(mood.greeting()) }`. The lines are in brain.rs:183-190.
- Exception: `fire`'s `Dazed` arm (1964-1971) sets `greeted` with "...I'm OK." instead, after a drop-in arrival.
- `say` (1554-1558) **hushes whatever she was saying**, so a later `say` in the same instant overwrites the greeting.

### 1.6 Absence today (the precedent for school, dash-ins and coming home)
- **Work:** `go_to_work` (4358-4377) sets `at_work`, then takes a link out or an `Act::Door{to: here, gap: SHIFT_MS}` (884: 60-180 s).
  - `Out` → `Away` (1744-1770) → back as `Walk` (1771-1785).
  - At the door's end, `home_from_work` (4381-4394) says `HOME` "I'm home!" (888) → `Act::Home` (`Pose::Carry`, the leeks: 4483-4486).
- **Hidden acts:** `hidden()` (4082-4090) is true for `Away` and for door beats without her. `props()` (716) gives `Away`/`Out`/`Door` `OnChat::Back`, so chat is ignored until she's back.
- **Errand:** an errand during `Away`/`Out` doors her straight to the accordion ("Work can wait", 4148-4158). `choose_next` has the **errand reflex first** (2945-2948), then "off text", then "watching chat", then the greeting, then owed beats (2987-3007), then offers (3093).
- **Arrival:**
  - `Osaka::arrive` (1281-1357) uses the body rng only. It produces `Walk` / `Fall` / `Stand` ("already home").
  - `arrive_for_errand` (4116-4125) builds her mid-door (`since = now - DOOR_THROUGH_MS`).
  - `whims` come from the mind seed (1240, 1273).
  - Visit-level draws hash the visit seed: `Mood::of` (brain.rs:131-141), applied at mod.rs:1152-1154.

## 2. Seams for 5b

| Seam | Where | What hooks in |
|---|---|---|
| Calendar greeting | choose_next 2971-2974; Dazed 1966-1968 | A day-keyed greeting in place of `mood.greeting()`, owed separately from `greeted` (proposed `owed_cal`) |
| Own-script override | start_job 2438-2439 | Generalise to (proposed) `fn own_script(seat, bought, surf, day, dash) -> (ScriptId, u8)`; e.g. a New Year watch = `Tv(Sunrise)` |
| Calendar splices | `SpliceCtx` 605-613; `Splice.when` 596; `chance` 594 | (proposed) `SpliceCtx.day: Option<Day>` and `clock: GameTime`; `when` reads them. "Every visit in exam season" needs the chance to depend on the day (see 4.4) |
| On-the-spot calendar script | `Act::SpaceOut{play: Some}`; `plan` 3196-3204 | (proposed) an owed pre-empt in `choose_next` after the beat block (2987-3007), with a fixed `until` (not `rng.range`) |
| Seasonal pools | `muse` 1606; Door arm 1804 | (proposed) `fn musings(day) -> Pool` / `fn door_lines(day)`, each season its own `PoolId` (new stable salt, `budgeted: false`) |
| Calendar decor | mod.rs `piece_state` 2254-2269; `art::PieceState` art.rs:930-941; `state_parts` 944 | (proposed) a `day` input → e.g. `PieceState::Dressed` on Plant/Poster (art) |
| Lamp while away | `Osaka::prop` 2761 is Use-only; `piece_state` | Lights off in an empty home: either `prop()` returns `LampOff` while she's away (one source), or `piece_state` takes the clock |
| Routine reflex | choose_next, right after the errand reflex (2945-2948) | (proposed) `Decision::reflex("away")`: if `away_until > now` and no dash-in is owed → `Act::Door{to: here, gap: away_until - now}` |
| Home again | Door end 1797; `home_from_work` 4381 | (proposed) `home_from_school` beside it: "I'm home!" without the leeks (`Pose::Carry` is the leek art) |
| Asleep at arrival | mod.rs paint: Arriving (771-783), then the Visiting branch's `furnish` (894-908) and chances | (proposed) `Osaka::tucked_in(seat, now, len)`, building `Act::Use{Sleep, Play{own: Sleep, branch: 1}}` directly |
| Script answering chat | `look` 3872+, `answer` 3917 | (proposed) `answer` consults `play.own.on_chat()` too, and a new `Chat::Stir` (sleeps through: "mm...") |

## 3. Invariants a builder must respect
1. **No body-rng or mind-stream draws for 5b choices.** Arrival-time choices (calendar item, dash-in time, asleep-or-not) hash `visit_seed` plus a new salt (the `Mood::of` and `cat_home` precedent). Decision-time choices use `self.whims` labels with new, unique labels or salts. `local: None` must leave every golden hash and the seed-7 snapshot unchanged.
2. **Every wildcard-free table must be filled in**, not wildcarded: the ScriptId ceremony (1.1); for a new pool, `PoolId::{ALL, id, budgeted, listed}`; for a new act, `Act::props` (690-719) and `census_group` (2804).
3. **A splice salt and a pool id never change** once shipped, because other rows' rolls depend on them (script.rs:583-587, mind.rs:744-746).
4. **SpaceOut-hosted scripts use `Still` poses only and no prop**, unless `first_due` (1492-1495) gains the frame grid. Alternating `Still` keys are the legal way to animate.
5. **A cued or calendar-forced script still cools** (`try_play`), like a rolled one. A calendar slot must not reuse the stage's `cued` slot: the stage would overwrite it, or it would overwrite the stage.
6. **A pooled line counts as said whether it shows or not.** Only a line spoken over in the same instant is `unsay`d (1564-1570).
7. **Keys end inside their body.** Lints: `every_set_time_fits_its_shortest_host` (script.rs:1498), `every_script_fills_its_body_in_order` (1474). A fixed-length SpaceOut script must declare its `shortest_body`.

## 4. Traps
1. **An alternate own-script is silently dropped.** `Cue::plays_on` takes any `Script(id)` with a matching `played_on`, but `start_job` only acts on Surf (2398, 2409) and the advert. A new `DashLunch`/`NewYearWatch` declared `played_on: Some(Use::X)` and cued is taken (`take_if`) and then never played. Generalise 2438-2439 in the same step.
2. **The greeting is lost on a drop-in.** Dazed sets `greeted` with "...I'm OK." (1966-1968). A calendar greeting hooked only at 2971 never plays on a fall arrival. Keep an owed flag apart from `greeted`.
3. **A same-instant `say` erases the greeting.** The greeting (2973), then an owed beat line (2995) in the same decide: the greeting never shows. It isn't pooled, so no `unsay` bookkeeping happens. Mark a calendar line delivered **when it has shown** (as a grievance is felt at `from + GRIEVANCE_MS`, 1836-1850), not when it's set.
4. **Exam-season chopsticks "every visit".** `Splice.when` is a bool gate after the chance roll, so it can't raise 1/3 to 1. The options:
   - (a) an owed `Cue::Splice(Chopsticks, None)` placed by the calendar slot;
   - (b) (proposed) `chance: fn(&SpliceCtx) -> (u64, u64)`.

   (a) needs no row change. The 10-min `try_play` cooldown still caps it at one per 10 min.
5. **Chat wakes a sleeper.** `Act::Use` is `OnChat::Look` (692-699), and `look` calls `interrupt(Cause::Chat)`. `answer` (3917-3930) reads only `spliced_at`, so an own script's `on_chat` is never consulted.
   - A night visit (22:30→07:00 game time ≈ 85 min real at 6×) under a live chat is broken by every line.
   - Fix it at the script level (proposed `Chat::Stir` on `Sleep`), and route `look` through the own script.
6. **`start_job` side effects.** A dash-in or tucked-in `Act::Use` must not go through `start_job`. It would roll splices, feel a grievance (the census fridge is off its wall in 14 of 16 visits), say `HMM`, or push `HomeEvent::Used`. Build the act directly; `credit` stays `None`, so no needs are credited.
7. **Any exit from a dash-in must lead back out.** `place` (3846), `settle`'s Fall/Dazed, `interrupt` and `lost_seat` (2700) all end in `decide`. A routine reflex in `choose_next` (proposed, after 2945) makes "home during school hours" impossible for every path at once. A dash-in's own continuation would not.
8. **Is the visit "empty" while she's away?** Hidden acts exist (`Away`, door gaps), and `furnish` runs per frame in the Visiting branch. **Open check:** whether `furnish` / `seats` treat a hidden Osaka's `(x, y)` as occupied (the comment at mod.rs:891-894 says furniture keeps clear of her). If they do, a hidden placeholder position could push a piece into the closet.
9. **Asleep at arrival needs furniture first.** `seats` comes from `visit.shown`, which only exists after the first `furnish` (mod.rs:908) and after `begin_visit`. `Osaka::arrive` can't pick a bed seat. See 5.3.
10. **The clock's 16:00 default lands in the "home, snack, sofa" window.** Any routine factor active at 16:00 moves every golden, unless the clock can be `None` (or neutral) in tests. This crosses into the clock/brain area.
11. **Summer vacation** keys "no school" on the **real** date, while school days come from the **game** weekday. Mixing the two clocks is a decision for the user (5.4).
12. **`Pose::Carry` is the leek art.** Coming home from school can't reuse `Act::Home` without the leeks.
13. **Shared cooldown by text.** A seasonal pool reusing a musing ("I wish I were a bird." for Tanabata) cools both together. That's intended dedupe, but the census then counts it under one pool or the other.

## 5. The settled questions

### 5.1 Does calendar content need a free-standing `Act::Script`? **No.**
`Act::SpaceOut{play: Some}` already hosts an on-the-spot script with any keys: `Still` poses, faces and bubbles, animated by alternating `Still` keys. Its gaps are a `then` continuation, props and bobs, and no starter date needs those. The routes, cheapest first:

1. Greeting variant: guaranteed; lines only.
2. Own-script override of a use, at 2438: e.g. the New Year watch on `Tv(Sunrise)` with "ooh". The art exists.
3. Calendar splice, or an owed `Cue::Splice`: exam-season chopsticks.
4. Owed SpaceOut script at a `choose_next` pre-empt (`Bucket::Owed`, method "calendar"): Setsubun.
5. Seasonal musing or door pools: flavour, not guaranteed.
6. `PieceState` decor: art.

"Owed once on the first visit of the day" means a delivery record:
- On `Osaka`: (proposed) `owed_cal: Option<CalItem>`, set in `begin_visit` beside `set_mood`, from `view.local` and the ledger's last-delivered date.
- Cleared when the item has shown.
- Written to the ledger then (one sqlite write a day).

Whether "day" is the calendar date or `timeutil::biblical_date` (day starts 09:00) is a choice to make. The biblical date keeps Halloween alive at 01:00 on Nov 1.

### 5.2 A dash-in ("Forgot my lunch!", 16 chars)
No new act is needed. It reuses `Act::Door`, `Walk` and `Act::Use`/`SpaceOut`, plus the (proposed) away reflex:

1. While away, she's in `Act::Door{gap}`. A dash-in is due at a time hashed from `visit_seed`.
2. The door ends **early** at a spot by the fridge: `to` and `gap` rewritten, exactly as `errand` does at 4139-4147.
3. The arrival `decide` hits the reflex. It sees the dash-in owed and does one of:
   - **(a)** fridge reachable: `Walk{then: Job(Use(snack seat))}` with a direct `Act::Use{play: plain(DashLunch)}`. `DashLunch` keys: `Still(Side)` + `FridgeOpen`, then "Forgot my lunch!".
   - **(b)** no fridge: a ~2.5 s SpaceOut with `Still(Side)` and the same line.
4. At its end, `decide`, then the reflex: `Door{to: here, gap: away_until - now}`. Back out.

Tradeoffs:
- **(a)** shows the fridge open but needs the trap 6 bypass and a fridge.
- **(b)** costs nothing.
- An errand during school is already a dash-in; only the exit through the reflex is new.
- `DashLunch` is one ScriptId ceremony. A bento in her hands would be new art; skip it.

### 5.3 Asleep when the visit begins
**Option A** (what the brief has in mind):
- The visit begins with Osaka hidden: a placeholder `Act::Away{until: now + ε}`, position subject to the open check in trap 8.
- After the first `furnish` and seats in the same paint call (no tick runs between; ticks are in `advance`), the Visiting branch takes (proposed) `Visit.tuck_in` and calls `tucked_in(bed seat)`.
- That builds `Act::Use{Sleep}` with **branch 1** (`LampOff` from frame 0: the existing trial branch) and `until` = game 07:00 converted to real ms.
- Fallbacks: a sofa, as `Use::Nap`; with neither, a normal arrival.
- Needs trap 5's `Chat::Stir` too.

**Option B** (zero new mechanics):
- She arrives normally, and the routine reflex sends her to bed. Branch 0 then plays #70, "lamp off before bed".
- Reads as "home late". It doesn't satisfy "asleep at the start".

Either way:
- The greeting on waking needs a clock variant: "...mm? Mornin'." in place of "Nice to meet you."
- After an interrupt, a factor above 1 on `Want::Use(Sleep)` at night brings her back to bed. Factors above 1 are safe; only factors below 1 delete a want (the map's A6 point).

### 5.4 For the user
- **Vacation:** does the real date override game-clock school days?
- **Day boundary:** midnight, or `biblical_date`'s 09:00?
- **Night chat:** does a chat line at night wake her (today's behaviour), or does she stir and sleep on?

## 6. Starter calendar dates, by cost

Cheap = lines ≤24 chars, existing poses, Channel and Prop, and factor boosts. Medium = one `PieceState` variant or one simple pose (a model sheet first). Expensive = new furniture, effects, text-layer content, or a uniform variant.

| Date | Cheap core (route) | Cost | Waits for art |
|---|---|---|---|
| Jan 1-3 New Year | Greeting "Happy New Year!"; owed watch on `Tv(Sunrise)` (first sunrise) "ooh"; musing "Omikuji... upside down?" | **Cheap** | Flying-pigtails dream, shrine, coin (L) |
| Late Jan-Mar exams | Owed `Cue::Splice(Chopsticks)` once a visit; `Use(Homework)` factor above 1 | **Cheap** (code only) | none |
| Feb 3 Setsubun | Owed SpaceOut script: alternating `Still(Jack(0/1))` keys, "Oni wa soto!" then "Fuku wa uchi!" | **Cheap** | Beans off the screen bottom (effect) |
| Mar-Apr hay fever | `Want::Sneeze` factor above 1 (sneezes exist) | **Cheap** | Tissues, eyedrops, petals |
| Apr 8 debut day | Greeting "I'm Ayumu Kasuga." / "Nice to meet you!" | **Cheap** | A bow pose (medium) |
| Jul 7 Tanabata | Greeting or musing "My wish: be a bird." (pairs with the MUSINGS line) | Cheap (lines) | Bamboo/wish slip as a Plant `PieceState` (medium) |
| Jul 20-Aug 31 summer | Routine: no school (user question, 5.4); last week `Homework` factor above 1 plus "Homework! Homework!" | Cheap code, open design | Beach float, swim ring (expensive) |
| Sep-Oct sports festival | Musing "Team Sea Slug!" | Cheap (thin out of context) | Flag, shoe kick (expensive) |
| Sep 30 finale | Greeting "We graduated, huh..." | Cheap partial | Graduation bow pose (medium) |
| Oct 31 Halloween | Greeting "Trick or treat!" | Cheap partial | **Sheet ghost** = one new pose (medium: a simple shape; ASCII trivial) |
| Oct-Nov culture festival | none | **Expensive** | Café in the playlist (text layer), penguins |
| December | Seasonal musing pool "Rudolph's nose... why?" | Cheap | Kotatsu (furniture plus head-only pose), winter uniform (all poses) |
| Dec 24-25 | Greeting "Merry Christmas!" | Cheap partial | Tree: a "Dressed" Plant `PieceState` (medium) or a new piece (expensive); present next morning |

**Recommended starter set (lines and code only):**
- New Year: greeting plus the Sunrise watch
- Exam-season chopsticks
- Setsubun on `Jack` keys
- Hay-fever sneezes
- Apr 8 greeting
- Halloween greeting, December musings, Christmas greeting

**First art candidates, by value per cost:**
- The Halloween sheet ghost (one pose)
- A dressed plant for Tanabata and Christmas (one `PieceState`, two dresses)

---

<!-- section: room-art-pieces -->

## Furniture pieces and art: a WINDOW and a WALL CLOCK (5b code map)

All paths are under `dessplay/src/ui/houseguest/` unless a path is given. Every line ref below was re-read on 2026-10-04 (`0661710`). **(proposed)** marks names that do not exist yet. The phase5a map's refs into these files are stale. Use these instead.

### 1. What exists

**The catalogue row** is `room.rs`.
- `Furniture` enum: 21-42. `ALL: [Self; 10]`: 46-57. `decor()` = `offers.contains(&Offer::Decor)`: 60-62. `spec()`: 65-78.
- `Spec` (83-114) holds name, pitch, footprint, ascii, ink, uses, offers, sit, comfort, `hang: Option<u16>` (111) and `beauty: f64` (113).
- The decor precedent is `PLANT` (222-234) and `POSTER` (235-248). Both have `uses: &[]`, `offers: &[Offer::Decor]` and `beauty: 1.0`. The poster has `hang: Some(4)`.
- Pitches go through `line!()` (mod.rs:46-60), a const assert that the line fits a bubble.
- ASCII: `glyph()` (252-262) mirrors each row for `Facing::Left` through `mirror()` (274-284), which swaps `/\ () []`.

**Wall lane**, room.rs:
- `Lane {Floor, Wall}`: 535-538.
- `lift()` (543-549): the hang once out of its box, and 0 as a parcel.
- `Prop::lane`/`needs`: 554-568. `needs` = rows + lift.
- `pack` (682) and `pack_each` (718). The wall lane is packed alone, and a piece that doesn't fit is left out by itself.
- `Shown::lift`/`lane`/`top`/`rect`/`cover`: 788-826. A hung piece's `cover()` is just its rect, with no floor row.
- `seat()` (828-866): the beside-uses take `beside` (869-874).
- `cells()` (877-893) is lift-aware. `screen()` (895-901) gives the TV's two ASCII cells.
- `layout`/`laid_on` (994-1021): `pack` on the floor, then `pack_each` on the wall.
- `project` (1037), `spot` (1134), `doorstep` (1185) and `admits` (1252). A hung item must fit both as a parcel on the floor and hung (1218-1229).
- `roomy` (1346) checks the beside-spots, standing on a line, for non-inside uses. `fits` (1400) doesn't need a floor when the piece is hung.
- Design rule: design.md:1613-1626 ("Wall pieces").

**Shopping**, mod.rs:
- `CATALOGUE: [Furniture; 9]` (101-111): the TV comes first, on its own (`FIRST_TV_VISIT`).
- `advert()` (125-139): `next(decor)` takes the first item in CATALOGUE order whose `decor()` matches and that she doesn't own. Decor comes first only while `Need::Beauty` is pressing.
- `beauty_at` (143-155) sums `spec().beauty` over the strip.
- `send_parcel` (471-482) chains TV + CATALOGUE. `wishlist` (488) and `give` (465) are stage paths. `Home::spot` sizes by `needs()`, so `give` should work for hung pieces (untested; see O2(d)).
- `cue()` (497-536) gifts the piece whose `uses` contain a scene's `furniture()` Use (stage.rs:187-200).
- design.md:1572-1576 gives the order: "…cat bed, then decor (a potted plant, a poster)".

**Ledger**:
- An older build drops unknown kinds silently: unknown `SavedProp` at ledger.rs:94, `ordered` at 124 and `unsettled` at 88.
- `rooms()` (212-232) skips `decor()` because "older builds don't know it" (215).
- `RoomKind::of` (242-254) is exhaustive, so a new piece has to be added to it.

**Art (line art)**, art.rs:
- SVG `art/props.svg`, `PROPS` include at art.rs:13. 20×42 units per cell (`CELL_UNITS`, art.rs:20). The frame list is in the header comment, props.svg:1-11.
- Groups `plant` (296) and `poster` (321). The poster's edges are snapped to whole pixels at 9×19 (comment at 317-320).
- `parts()` (870-886) is exhaustive.
- `PieceState {Plain, LampOff, FridgeOpen, Cat, CatBiting}`: 930-941.
- `state_parts` (944-954) returns **static** SVG ids. Its fallthrough is `parts(item, Whole)`.
- `render_piece` (958-980) wraps every part in the facing mirror group.
- The procedural precedent is the TV:
  - `Channel` (1021-1027): `Snow(u8)`, `Shopping(u8)`, `ColourBars`, `Sunrise`.
  - `sunrise()` (1080) already draws a dawn sky over a sea.
  - `tv_scene` (1119-1146): a procedural picture clipped to `GLASS`, **inside** the mirror group. `render_tv` is at 1150.
  - `prop_frame` (1166).
- Tests:
  - `CHANNELS`/`STATES` const lists (1522-1542).
  - `every_new_piece_renders_inside_its_footprint_in_every_state` (1545).
  - `catalogue_sheet` (1575, `HOUSEGUEST_CATALOGUE`).
  - `every_parcel_and_channel_renders_inside_its_box` (1668).
  - `every_prop_renders_inside_its_footprint` (1908).
  - `props_sheet` (1956, `HOUSEGUEST_PROPS`).
  - `vignette_sheet` (1294, `HOUSEGUEST_VIGNETTES`): 1×, 1×-nn3× and 3× over `#1e2127`.

**Dynamic looks**, mod.rs:
- In the Visiting paint, `cat = cat_now || cat_home(ledger)` (1079). `prop = osaka.prop(now)`. `Looks{tv, states}` (1081-1088) uses `piece_state(p, prop, cat)`.
- `piece_look` (2211-2234): a state that isn't Plain gives `Look::Piece(item, state)` only for `Layer::Whole`.
- `piece_state` (2252-2268) maps the script's `Prop` (script.rs:75-120: `Tv`, `LampOff`, `FridgeOpen`, `CatBiting`; only her script sets these) and `cat`.
- `prop_layer` (2273-2282): `at = (left+cols/2, floor-lift)` and `standing = lift==0`.
- `Looks` (2287-2299). `paint_prop_art` (2302).
- `cat_glyphs` (2363-2369) is wildcard-free by design. `screen_glyphs` (2373-2380).
- `draw_props` (2385-2458): ASCII overrides come after `glyph()`, so they are not mirrored. The cat's row test `top = y == prop.floor - size.1` (2413) **ignores lift**.

**Image cache**, graphics.rs:
- `CACHE_LIMIT = 1024` (58).
- `Look` (62-80) has `Piece(Furniture, PieceState)` and `Tv(Channel)`. Both must be `Copy + Hash`.
- `Key` (143-156) is `(Look, Facing, origin, standing)` per layer, plus size, the covered line glyphs, clip and cell.
- The busy-visit bound is `VISIT_IMAGES = 512` (tests.rs:1835). The ignored `image_census` is at tests/census.rs:757.

**Is a piece part of her image or the room's?** The room's. Each piece in `apart` is its own one-layer image (`draw_props` → `paint_prop_art`).
- Exception: a piece whose `cover()` her box overlaps, transitively (`terrain::image`, terrain.rs:33-65), goes into **her** image as a layer. The split is at mod.rs:1062-1075. Her key then carries that piece's `Look`, so every clock face multiplies her poses.
- With `hang` 4, a hung piece's bottom row is floor−5 and her top is floor−4 (`HEIGHT` 4, sprite.rs:8). From the strip floor she never takes in a hung piece, even standing right under it.
- **Builder to verify:** whether `Act::Climb` on a pane-wall pole (osaka.rs:2157-2164) pulls in a piece hung at offset 0 from that wall. Deliveries anchor at offset 0 (room.rs `doorstep`). A test should settle it. If it does, the cost is a few climb images per dial state.

**Poses already on hand:** `Pose::Gaze` "gazing up at something" (sprite.rs), `Face::Curious` "looking up", and Activity::Gaze → `(Pose::Gaze, Face::Curious, Some(Bubble::Ooh))` (osaka.rs:1017). This is how she would look up at a hung piece without new art.

### 2. Seams where 5b hooks in

1. **Two rows (proposed) `WINDOW` and `CLOCK`** in room.rs, plus the `Furniture` variants. Appending them at the end of `ALL` keeps existing serde names and `ALL[i]` indices (tests.rs:402/425 index `0..4`).
   - The rest of the compiler-forced list: `spec()`, `RoomKind::of`, `parts()`, `CATALOGUE` length, props.svg groups and the header frame list.
2. **World state into looks.** The window and clock show *game* time. That is world state like `cat`, not her script.
   - Widen `piece_state`'s `cat: bool` into a (proposed) `World { cat, game: GameMinute }`, built at mod.rs:1079 from the 5b clock.
   - Do **not** add these states to `script::Prop`, which is only set from a key and only while she's in `Act::Use`.
3. **The state's shape.** There are two options:
   - (a) Data-carrying `PieceState::Sky(Sky)` / `PieceState::Dial(Dial)` (proposed). These go through `Look::Piece` unchanged, and the wildcard-free `cat_glyphs` makes the compiler point at every site.
   - (b) New `Look::Window(Sky)` / `Look::Clock(Dial)` like `Look::Tv`, plus `Look::size`/`render` arms.

   (a) touches fewer sites. Either way the dial needs a **procedural** render branch, because `state_parts` returns `&'static` ids. A `dial_scene(dial)` (proposed), modelled on `tv_scene`, would rotate two `<use href="#clock-hand-*">` around the face centre. The sky can be 4-5 static groups (`window-sky-night`, …) behind a `window` frame group.
4. **ASCII overrides.** Generalise `cat_glyphs` plus `screen` into one (proposed) `overlay(prop, looks) -> Vec<((i32,i32), char, Option<Ink>)>`, applied after `glyph()` as today. Compute positions from `prop.rect()`, never from `floor - size.1`. The per-cell ink would let a night sky be dark in 16 colours.
5. **The repaint wakeup.** `Guest::next_tick` (mod.rs:742-761) chains only fades, flap and `osaka.due()`. A dial or sky change between her wakeups shows only when something else repaints, which could be 60-180 s while she sleeps.
   - Chain the next quantum boundary (proposed `World::next_change(now)`) **only while a window or clock is shown** (projected, not only laid out: a piece in the closet paints nothing).
6. **The home while she's away.** `paint` draws furniture only in `State::Visiting` (mod.rs:791, `Absent | Arriving => nudge.paint`). 5b's empty home is where a clock reading 10:00 explains her absence. That paint path (outside this area) must build the same `Looks`/`World` and call `draw_props`.
7. **A use, if one is chosen** (options C2/C3 below): `spots_for` (mod.rs:2015-2030) turns each `piece.uses()` into a seat via `seat()` and `beside()`. A hung piece flows through it unchanged.

### 3. Invariants a builder must respect

- **Hang ≥ the tallest piece that stands (4).** `hung_pieces_clear_every_standing_piece` (room.rs:1925-1938) asserts this, and also asserts `uses.is_empty()` ("out of reach").
  - A window or clock with a use **breaks this lint by design**. It has to be relaxed on purpose to "no `inside()` use".
  - That is sound: a beside or under seat stands on the strip floor, and `roomy` already checks beside-spots for any piece.
- **The pane must be tall enough.** The strip's clear rows must be ≥ `hang + rows`, or the piece is in the closet (`Extent::holds`).
  - `home_screen` (tests.rs:1807) has 9-high panes, so 7 clear rows. A 3-row window fits exactly, and a 2-row one leaves one row spare.
- **The wall lane is one-dimensional.** `pack_each` packs along x only. Each hung piece has its own `hang`, but nothing stacks vertically, so a clock *above* a window on the same strip can't be represented. Hung pieces stand side by side, with the poster.
- **One of each kind** (`Home::add`). New kinds go at the end of `Furniture` (serde names) and of `ALL`.
- **ASCII is dev/test-only** (the user's client is kitty). Keep it minimal, but it must fill its footprint exactly (room.rs:1430), and every glyph must be mirror-safe or drawn as an override.
- **Line art:** 20×42 units per cell, outline `currentColor` = `#1d1714`, edges snapped to whole pixels at 9×19 (the poster's comment), following the style in docs/houseguest-style/README.md. Every state must ink more than w·h/6 (art.rs:1545). New entries go in `STATES`/`CHANNELS`.
- **Goldens.** Homes without a window or clock must hash exactly as before. That means no unconditional wakeups, no CATALOGUE reordering ahead of what golden runs buy, and no body `rng` draws in the look code.
- **Keep `VISIT_IMAGES` ≤ 512** in the busy tests, with no re-encodes.

### 4. Traps

1. **A mirrored clock tells the wrong time.** `render_piece` (art.rs:958-980) and `tv_scene` put *every* part, the procedural picture included, inside `scale(-1 1)` for `Facing::Left`.
   - Deliveries through the right wall arrive `Facing::Left` (room.rs `doorstep`), so 3:00 would read 9:00.
   - ASCII has the same problem: `glyph()` mirrors `(` and `)`, and reverses the columns of each row (`cols - 1 - dx`).
   - **Fix (proposed):** a `Spec.symmetric: bool` (or `fn turns(self)`). `prop_layer`/`piece_look` then normalise `facing` to Right for symmetric pieces. This also halves their cache keys. Dial and sky glyphs go in overrides, not the static ASCII.
2. **One image per game minute exhausts the cache.** At 6×, a game minute is 10 s real.

   | Quantum | Distinct dials per 12 game h (2 h real) | New images per 20-min visit |
   |---|---|---|
   | Per minute | 720 (about 360 per real hour) | about 120: on top of the busiest census (350) that's ~470 of 512, and it churns her working set |
   | 10 minutes | 72 | 12 |
   | Quarter hour | 48 | 8 |
   | Hour hand only | 12 | 2 |

   Sky phases add 4-6 images in total.
   - Recommendation: quarter-hour; 10 minutes is the sane upper bound.
   - Normalised facing keeps each count as given; without it, double it.
3. **`rooms()` uses `decor()` to stand for "older builds don't know it"** (ledger.rs:215). A window or clock that has a use isn't decor, yet a 5a build doesn't know it either.
   - Replace it with an explicit (proposed) `Furniture::legacy()`, true for the first eight kinds.
   - On the `stable` track, a 5a build drops the piece and an `ordered` one (ledger.rs:94/124), and its next save erases them. `bought_on` has already advanced, so the purchase is lost. Record this in decisions.md, as was done for decor.
4. **Sales order.** `next(false)` takes the first *non-decor* item wherever it sits in CATALOGUE.
   - A non-decor window or clock is sold right after the cat bed, ahead of the plant and poster, which changes census purchases.
   - Inserting it near the head of CATALOGUE may also change what `golden_stage_room`'s Shopping cue buys (at 170 s per the 5a map's d11, not re-verified here; re-check golden.rs).
5. **Copied cat code ignores the hang.** The cat's `top` formula (mod.rs:2413) has no lift. Copying it for a hung clock draws the override five rows too low.
6. **Repairs never consider a hung piece.** `rules::search` sizes candidates with `footprint`, not `needs()` (rules.rs:708/720).
   - That costs time only, since `evaluate` rejects through `laid.len()`.
   - Carrying a hung piece in her pocket and setting it down (mod.rs:1736-1740, `middle_of` on the floor) is untested for `Lane::Wall`.
   - Judging a rule already works on hung pieces: `Near` uses `between` on rect x, and `AgainstWall` uses `left` and `cols` (rules.rs:229-246, 317-327). The gaps are only in mending: `search`'s candidate sizing, and the carry and set-down path. A rule on them is cheap to judge and expensive to mend.
   - Recommendation: no 5b rule involves the window or clock.
7. **Property strategies hardcode the poster.**
   - `decorated()` (room.rs:1762) inserts exactly one `Poster`.
   - `long_visits_with_decor_never_touch_what_is_protected` (tests.rs:408-433) picks Poster or Plant from a bool.
   - Generalise both to a sample of the hung or decor set, with several hung pieces at once on a crowded wall (`pack_each`).
8. **Rare pieces stay in the closet.** A crowded wall (poster 4 + window + clock) or a short pane can closet a legibility piece, and nothing gives it priority. Hung pieces pack in anchor order.

### 5. Options with tradeoffs

**O1. Kind: decor or furniture-with-a-use.**
- *Decor* (`Offer::Decor`, `beauty` 1.0, `uses: &[]`): no lint changes, no `Use` cascade, and it is sold in the decor tail. Beauty sums are capped at 1 (osaka.rs:2589), so a third or fourth decor piece adds nothing to her want. The clock and window would only be read, never used.
- *Furniture with a use*: see O3. This adds a Want, a script and a scene, and it changes the sales order (trap 4).

**O2. When she gets them.** Legibility is the goal, but the catalogue tail comes roughly 7 purchases × `SHOP_EVERY` 3 = 20+ visits in.
- (a) Catalogue decor tail: it's simple, and comes earlier while beauty is pressing.
- (b) Put the clock first among decor, i.e. ahead of the plant in CATALOGUE. Goldens don't move unless a golden home reaches decor.
- (c) A one-time delivery when the 5b clock is first set up on a ledger (proposed: `ordered = Some(Clock)` if nothing is on order). This touches the ledger area, and it is the only option that reaches the user's existing home soon.
- (d) Stage `give()` should work by reading: `spot` sizes by `needs()`, `fits` skips the floor when hung, and `roomy` checks nothing when there are no uses. It is untested for a hung piece; the only gift test gives a Lamp.

**O3. What "using" them is.**
- (A) **No use.** They show time, and that's all. She can still glance at the clock from the routine (C3).
- (B) **Gaze through a method on `Want::Idle(Activity::Gaze)`.** This doesn't fit: `Bind::WalkTo` doesn't chain into a pose, so walking to the window needs `Job::Use(seat)`, which means a Use.
- (C2) **`Use::LookOut` (proposed) on the window.** She stands *under* it (a new `seat()` arm: `left + cols/2`, facing the piece's way) or beside it, in `Pose::Gaze`/`Face::Curious`. Lines are keyed on the sky ("Stars!" at night, "Pretty..." at dusk).
  - Touch list: `Use::ALL` (room.rs:325), `inside()` (340), the `seat()` arm, `use_duration` (osaka.rs:915), `Use::script` (script.rs:907), `ScriptId` with `played_on`/`trial_branch`/`host` and the lints, `Want::ALL` plus `def` (brain.rs:430-515; Daydreams about 0.5 like Gaze), `ANY_USE` and its test (rules.rs:50, 1161-1185), `Scene` (ALL count, name, furniture, cue; stage.rs), census `group`, tests.rs:5450, and the hung lint (section 3).
- (C3) **The clock as a glance in routine transitions:** a prelude on Sleep, or before leaving for school, "Oh! It's late!", played only when a clock shows on her strip.
  - A splice plays at the wrapped use's seat (she can't walk to the clock). So `SpliceCtx` (script.rs:605) needs (proposed) `clock: Option<Facing>`, the way to a clock shown on her strip, and the game time.
  - The prelude turns her toward it with `Pose::Gaze`. Or it could be a free-standing `Act::Script` if 5b adds one.

**O4. Footprints** (diameter = min(cols·20, rows·42) units ≈ 0.45 px per unit at 1×). The pane must have `hang + rows` clear rows.

| Piece | Option | Line art | ASCII and notes |
|---|---|---|---|
| Clock | 2×1 | ~18 px face | `()` |
| Clock | **3×2** | ~25 px, legible | e.g. `.-.` / `(o)`, hand glyph as an override |
| Clock | 4×2 | ~36 px, as big as the poster | — |
| Clock | 2×2 pendulum | same face as 2×1 | the pendulum stays static: animation means more images |
| Window | **4×2** | 80×84 units like the poster; 2 panes and a sill, sky clipped inside like `GLASS` | `.--.` / `\|  \|`, inner two cells sky overrides (` o` day, `~~` dusk, `*.` night) |
| Window | 5×3 | larger sky | needs 7 clear rows, exactly `home_screen`'s |

**O5. Sky phases** on the game minute-of-day: night, dawn, day, dusk and evening, with lamp-lit interplay optional. A new ledger starts at 16:00, so dusk arrives within its first session (1-3 game hours is 10-30 min real), which makes it visibly legible. A season tint from the real date can wait for calendar content.

### 6. Model sheets and art review

- **Decor precedent:** `docs/proposals/2026-10-02-houseguest-mind/decor/` holds `new-pieces-sheet.png`, `snippets.md` (SVG groups, Spec consts, edit list, tests) and `worktree.diff`. The art was drawn in a throwaway worktree and approved before wiring.
- **Vignette precedent:** `…/vignettes/` holds a README-style `snippets.md`, `vignettes-1x.png`, `vignettes-1x-nn3x.png` (judge what reads) and `vignettes-3x.png` (judge the drawing). All are over `#1e2127`, generated by an `#[ignore]`d sheet test.
- **Style brief:** `docs/houseguest-style/README.md`, with 9×19 cells and a bold outline.
- **Proposed:**
  - A sibling dir `…/houseguest-mind/clock/` (or `phase5b/art/`).
  - A `clock_sheet` test (`HOUSEGUEST_CLOCK=dir`) rendering every sky × the window, and the 12 hours × 4 quarters of the dial, at 1×, nn3× and 3×. It should include both facings, to prove trap 1 is fixed. It should also show her in `Pose::Gaze` under the window, and the two pieces hung over a sofa beside the poster.
  - Add both pieces to `catalogue_sheet`/`props_sheet` rows once they're wired in.

---

<!-- section: shell-tests -->

# 5b map: time injection and tests

Every line ref below was re-read at `0661710`. The phase-5a map's refs are stale. Corrected ones: design.md rule at **:1919** (was 1856); seed-7 at **tests.rs:703**; `real_frame` **623-634**; `observe` **mod.rs:1302**; `gate` **1286**; `Chances` literals **mod.rs:1007, 1042**; `Scene::ALL` has **37** entries.

## T1. Time sources today

- **Shell clock.** `now_millis()` is at shell.rs:707-713. Its doc comment (700-706) says it is "the UI thread's only time source (the `Ui` itself never reads a clock)". It counts monotonic ms from the first call, so each process restarts near 0. design.md:1919 says the same: "Timing uses the UI thread's monotonic clock".
  - **5b breaks this rule on purpose** by stamping `LocalTime`. Rewrite the comment and design.md:1919 to carve the date out: the `Ui` and `Guest` still never read a clock; the shell stamps the date into the view. Give the reason in decisions.md.
- **`draw()`** (shell.rs:623-638) reads `now` once (633), then `ui.idle_view(renderer.image_regions())` (635), then `guest.paint(buf, &view, now)` (636).
- **Run loop** (shell.rs:450-619):
  - The timeout arm calls `guest.advance(now)` (485).
  - Every input calls `guest.advance(now)` first (505).
  - Key, mouse and paste input calls `guest.activity(now)` (587).
  - The quit path is 588-593: `blocking_send(action)` and then `return`.
  - The ledger drain runs at the **top** of each iteration (453-466): `ledger_to_save()`, then `try_send(SaveHouseguest)`, re-queued while the channel is full.
- **The quit drop (map d14), confirmed.**
  - run.rs:1537 handles `Quit` in FIFO order with `SaveHouseguest` (1786). Teardown at run.rs:1150-1152 closes `actions` and then sends `Shutdown`. Anything sent after `Quit` is lost.
  - Both `return` (592) and every `break` (`Shutdown` 508, `Closed` 464/472, a draw error, `Disconnected` 494) skip a final drain.
- **Guest time.** `Guest` (mod.rs:362-402) keeps only `quiet_since: u64` (375) in shell millis. Nothing in the guest counts elapsed time across a restart.
  - `advance(&mut self, now)` is at 701-739.
  - `next_tick(&self, now)` is at 742-761. It takes `&self`, so it can't accrue time.
  - `paint` is at 765. It calls `observe` (1302) and then `gate` (1286).
- **Suspend.** Rust's `Instant` is `CLOCK_MONOTONIC` on Linux, which doesn't advance during suspend. That is why "runs only while dessplay is open" holds through a laptop sleep. **Check this against the std `Instant` docs before relying on it in decisions.md.**

## T2. `IdleView` and its literals

- `IdleView` (idle.rs:69-99) derives `Clone, Debug, Default, PartialEq`. Its fields are `delay, busy, resident, focus, chat_mark, chat, scrollback, protected, nooks, truecolor`.
- **Correction to map A7:** a field `local: Option<LocalTime>` keeps `derive(Default)`, because `Option<T>: Default` for any `T`. `LocalTime` needs only `Clone, Copy, Debug, PartialEq, Eq` (`NaiveDate` has them).
- **Only two literals list every field** (checked by a script over all 42 `IdleView {` sites):
  - app.rs:640 (`Ui::idle_view`) gets `local: None`, with a comment that the shell stamps it.
  - tests.rs:21 (`fn view`) gets `local: None`.
  - Every other site spreads (`..view(..)`, `..view.clone()`, `..resident_view(..)`, `..on`, `..quiet.clone()`). This covers tests.rs, golden.rs:175/242/277/302 and `Weather::view` (tests.rs:8346-8361).
  - mod.rs `whole_glyphs` (255) and `gate` (1286) clone and mutate; they build no literal.
- **`idle_view` callers:** shell.rs:635; examples/houseguest.rs:105; `real_frame` (tests.rs:630); app.rs tests 5366-5414. They all get `local: None` automatically, so every existing test stays calendar-free with no edits.

## T3. Seams (proposed)

### Game clock: lives in the ledger, accrued from `now` deltas
- **Ledger fields (proposed):**
  - `clock: GameTime`, game milliseconds since a game epoch of Monday 00:00. Using ms avoids rounding loss. The type is `u64`; never store an absolute `now`.
  - `Ledger::new` sets it to `START` = Monday 16:00 (960 game minutes).
  - `Raw` reads it as `Option<Value>` and falls back to `START`, matching the lenient pattern at ledger.rs:284-303.
  - `Saved` writes it with `skip_serializing_if = "is_start"`.
  - This keeps `missing_fields_default` (ledger.rs:564: `from_json("{version:1}") == Ledger::new(0)`) and `the_record_as_written` (372, exact JSON) passing unchanged.
- **Guest fields (proposed):**
  - `clock_seen: Option<u64>`, the last shell `now`. It is `None` after `new` or `restore`.
  - `clock_pending: u64`, game ms not yet flushed.
- **Accrual (proposed):** `fn accrue(&mut self, now)` is called first in `advance` (701) and `paint` (765), before `observe`.
  - It adds `6 × now.saturating_sub(seen)` and sets `seen = max(seen, now)`.
  - It is idempotent at an equal `now`, so the double call per input (505 and then 633) is harmless.
  - A first observation sets the baseline and accrues nothing.
  - Existing tests force all of this:
    - `her_home_fills_up_over_visits` (tests.rs:2476-2490) restores mid-run while `now` keeps rising.
    - `a_busy_client_gets_no_visit` jumps `advance(3_600_000)` (137), which is 6 game hours in one step. That's allowed; nothing caps it.
    - The `furnished` golden paints at 0 five times before `drive` (golden.rs:252-261).
- **Flush (proposed):** pending time moves into `ledger.clock` and sets `unsaved` when either:
  - `clock_pending ≥ FLUSH` (proposed: 60 s real, 6 game minutes, at most one sqlite write a minute); or
  - something else already set `unsaved`, so the clock rides along for free.
- **Invariant (`her_home_outlives_a_restart`, tests.rs:2226):** `ledger_to_save()` must stay `None` when only sub-threshold clock time is pending.
- **Readers** read `ledger.clock + clock_pending` through one accessor (proposed `Guest::game_time()`), never `ledger.clock` alone.
- **`move_out`** (mod.rs:451) already replaces the ledger. It must also zero `clock_pending`; `clock_seen` stays.
- **Reaching the brain (proposed):** add `clock: Option<GameTime>` to `Chances` (osaka.rs:19, which derives `Default`).
  - Fill it at both literals (mod.rs:1007, 1042), taking the minute from `paint`.
  - `None` means a neutral routine. Every osaka.rs/mind.rs unit literal uses `..Chances::default()` (e.g. osaka.rs:5423-5426, 5706-5708; mind.rs:1044-1048), so they all stay neutral for free.

### LocalTime: on the view, stamped by the shell
- **Type (proposed):** `pub struct LocalTime { pub date: NaiveDate, pub minute: u16 }` in idle.rs, with a test constructor `LocalTime::on(y, m, d)`.
- **Day boundary, a design choice:**
  - Civil date: Christmas content shows at 00:30 on the 25th.
  - `timeutil::biblical_date` (timeutil.rs:10-14), where the day starts at 09:00: consistent with the chat day separators. Taking it needs `minute` or the stamp.
- **Shell (proposed):** in `draw()` after line 635, `view.local = Some(local_now(now))`.
  - Cache per `now / 60_000`: the stamp is per frame, and `Local::now()` resolves the zone on each call.
  - A DST or zone change shows up at most one minute late.
- **Guest:** `observe` (1302) latches `self.local = view.local`, beside `self.delay`, and passes it on through `Chances`. `None` disables every calendar path.
- **Example:** examples/houseguest.rs:105 builds `view` from `ui.idle_view` in its draw closure. That is the **same seam** as the shell: set `view.local` there.

## T4. Stage (examples/houseguest.rs)

- **Today:**
  - Keys are documented at 8-14 and matched at 182-212.
  - `now_ms` advances by `SPEEDS[speed]` (0.125× to 4×, lines 35, 97-100), so speed already scales game time.
  - Free keys: `t`, `d`, `w`, `c` (lowercase only; Ctrl-C quits).
- **Proposed stage API:**
  - `Guest::game_time() -> GameTime` for the bar. Show "Mon 16:05" in `menu` (line 132).
  - `Guest::skip_clock(to: GameTime)`, **forward only**. It jumps to the next routine boundary on `t`, so the "never backward" property holds even for the stage. `n` (new seed, 199-203) already resets through `Guest::new`.
  - `d` cycles a stage date offset through the starter calendar dates, then the real date, then `None`. Apply it in the closure as `view.local = ...`.
- **Cues:** `Guest::cue` (mod.rs:497-536) sets `State::Arriving` unconditionally (531-533), and `send` (1244-1283) brings her for an errand whatever the gate says. **A routine "away" gate must leave both alone.** Otherwise `every_scene_has_a_spot_in_the_stage_room` (tests.rs:1374) breaks, as does every `cue(Scene::Arrive)` fixture run at a non-home hour (`one_visit` 2452, census `arrive_drawn` 270).
- **New `Scene`s** (dash-in, calendar) need rows in `Scene::ALL` (stage.rs:110), `name()` and that test's `happened` match.

## T5. What the goldens hash, and what must hold for them not to move

`golden.rs` has 4 scenes × 4 seeds × {ASCII, line art} = 32 hashes. Tables are at golden.rs:337-389. `check` (318) prints a replacement table. `HOUSEGUEST_GOLDEN_TRACE=<dir>` writes a per-frame trace.

**Frame lines** (`Trace::frame`, golden.rs:40-78) hash `now`, `act_name()` (osaka.rs:1387), x, y, facing, `appearance(now)` Debug (osaka.rs:4397, a tuple `(Pose, Face, Option<Bubble>)`), `visit.image` Debug (`Placement`, mod.rs:184-190) and every changed cell. `drive` (118-144) picks each step from `next_tick` (clamped 1..1000) and hashes a frame only when `advance` returns true.

With a clock that starts at weekday 16:00, frame lines stay unchanged if and only if all of these hold:
1. **The routine is neutral on weekdays 16:00–18:00:** no factor ≠ 1, no gate, no departure.
   - The longest default-start golden is `furnished`, 600 s, ending at exactly 17:00 game.
   - The 20-minute `live_in` tests (1850, 1908, 1958) and `her_needs_shape_long_visits` (1696) reach 18:00.
   - `live_in_watching` panics "still visiting" if she isn't visiting on a painted step (2058-2060).
   - `her_home_fills_up_over_visits` runs about 5 game hours, to about 21:00. It cues every arrival, so it's safe if cues bypass the gate (T4).
2. **`advance` returns false for clock-only accrual, and `next_tick` adds no wakeups** while no routine boundary is due. Either would shift `drive`'s step `now`s, which are hashed.
3. **Rolls use `Whims` labels** (mind.rs:34-58, keyed by label so new labels shift nothing) **or visit-seed salts**, like `Mood::of` at begin_visit (mod.rs:1152). Never the body `rng` or `self.mind`.
4. **New pieces (window, wall clock) go at the end** of `CATALOGUE` (mod.rs:101) and `Furniture::ALL` (room.rs:46). `send_parcel` (471) and `advert` (124) then pick what they pick today. `RoomKind::of` (ledger.rs:242) is exhaustive, so the compiler forces a mapping.
5. **No new field in the hashed Debug output**, i.e. the `appearance` tuple or `Placement`. A new `None` field moves all 32 hashes. Calendar looks ride as new variants of an existing `Pose`, as 5a did with `Pose::EatAndagi`.
6. **`local: None` turns every calendar path off.** Goldens use `real_frame` or `view()`, so this holds automatically.

**The `finish` hash** (golden.rs:83-96) hashes `ledger.to_json()`. Once the clock, or any pity counter, is persisted and accrues, **all 32 finish hashes move, however carefully the rest is done**. Options, both (proposed):
- **(a) Re-record once per persisted field**, proved by a trace diff: run `HOUSEGUEST_GOLDEN_TRACE` before and after, and show that only the trailing JSON line differs in each of the 32 files. `finish` should then hash the *drained* clock (`game_time()`), not a value that depends on when the last flush happened. This keeps the clock under golden coverage.
- **(b) A separate storage key** (`houseguest_clock`) for the clock and the counters.
  - Goldens stay untouched, and **trap d13 goes away**: an older build that re-saves the ledger no longer wipes the clock.
  - Cost: a second write per flush, `move_out` must clear both keys (shell.rs:451 sets `unsaved` for the ledger only), and a second `Raw`.
  - plan.md says "in the ledger", so this is the user's call.

**`State` variants:** `Trace::frame`'s match on `State` (golden.rs:42-58) is exhaustive. A new state such as `Away` won't compile until it's named there; choose its label on purpose.

**Seed-7 snapshot** (tests.rs:703-719, `snapshots/..._osaka_at_home_seed_7.snap`): it covers 95 s real (9.5 game minutes from 16:00) with `Guest::new(7)`, an empty home and `local: None`. It moves only if 1–6 break, or if a greeting or arrival script changes.

## T6. Fixtures to extend (proposed)

- **`view()`** (tests.rs:20-33): add `local: None`. Calendar tests use `IdleView { local: Some(LocalTime::on(..)), ..view(..) }`.
- **Clock start:** set `guest.ledger.clock = GameTime::at(Weekday::Sat, 7, 30)` directly. Ledger fields are `pub(super)`, and tests already write `guest.ledger.home`/`visits` (golden.rs:197-207, tests.rs:461). A test-only `Ledger::new_at(seed, t)` would be nicer.
- **Census `Room`** (census.rs:23-34): add `start: GameTime`, defaulting to `START`.
  - `visit_census` (629, 30 minutes, to 19:00) and `image_census` (757, 120 minutes, to 04:00) now cross the evening and night, so their numbers stop being comparable with 5a's tables (plan.md "Measured").
  - Option: a cfg(test) `Guest::freeze_clock()` for the 5a-comparable rooms, with a separate day census for the routine. Say which one any recorded number used.
- **`rooms()`** (37), `home_screen()` (1807), `furnished_home_with` (2004): unchanged. A window or clock piece enters only through `give`.

## T7. Cheap headless censuses of a day or week

- **Layer 1, a pure tape (gate-safe):** `routine(GameTime) -> Slot` evaluated at each of 7×1440 minutes. Microseconds. It prints or asserts the week table.
- **Layer 2, simulated (#[ignore], release):** a `visit_from`-like loop (census.rs:312-446) without the "already visiting" assumption, starting from `ledger.clock = Mon 00:00`.
  - **Cost, derived from plan.md's numbers, not measured:** `image_census` does 192 visits × 2 h in about 40 s release with line art, roughly 0.2 s per sim-hour. A game day is 4 sim-hours, under a second. A week is 28 h, a few seconds per seed. ASCII is cheaper.
  - **To make away-time nearly free:** `next_tick` must report the next routine boundary while `Absent` (as `(boundary − game) / 6` real ms), and the census loop steps unclamped while `Absent`. This matters for correctness too: `run()` (tests.rs:93-106) jumps straight to `until` on `None` and would skip a boundary.
  - **Per game hour, report:** % present, % away, dash-ins, act groups (`group`, census.rs:477), sleeps and meals by slot, departures and returns (resident and visitor), calendar plays, rares offered and first seen.

## T8. Properties to add (proposed), ordered by cost

**Case-count trap:** `proptest_cases(N)` is overridden *upward* by `PROPTEST_CASES` (dessplay-core/src/test_support.rs:43-48). A pinned 8 runs 32 cases at the gate and 256 on the deep pass, under nextest's 30 s SLOW / 60 s kill (`.config/nextest.toml`; only `every_made_piece_is_used_or_let_go` has an override, 120 s).
- Keep anything that simulates a full visit out of proptest, or bound it to under ~0.1 s a case.
- New suites use `proptest_cases(N)` or the default config, never a bare `with_cases`.

**Headless proptests (no paint):**
- **Clock monotone across save-load.** Generate a sequence of steps: real deltas (some zero or backward), plus restarts that go through either `Ledger::from_json(ledger_to_save().to_json())` with `now` reset to 0, or a crash (restore from the last handed-out ledger).
  - Asserts: `game_time()` never decreases within a process.
  - After a clean drain and restore it is equal.
  - After a crash it loses at most `FLUSH × 6`.
  - With no crash, the total equals `6 × Σ positive deltas`.
  - It's cheap with `State::Absent` and `delay: None` (no arrival path).
- **Ledger round trip:** an arbitrary clock and counters round-trip. A missing clock reads as `START`. A malformed clock (string, negative, float) reads as `START` without failing the record. A record at `START` writes no `clock` key.
- **Routine tape:** every slot occurs each week; weekends have no school; slots are contiguous and above a minimum length; the hour after boundary `b` is the slot that `b` starts. **The neutral window is pinned:** the factors at weekday 16:00–18:00 are all 1. This makes T5 rule 1 an explicit test.
- **Calendar over years:** every date of 2024 (leap year) through 2040 (for moveable feasts) gives at most one entry, the same entry every call, and each starter date fires on exactly its day. Line-length lints apply. The test is time-zone independent: it constructs `LocalTime` directly and never calls `Local::now`.
- **Pity gate as a pure function** of `(seed, salt, counter)`: offering is monotone non-decreasing in the counter, certain at the cap, and the base rate falls within tolerance over fixed seeds.

**Fixed-seed sims (one or two seeds, ASCII, gate):**
- **None ≡ neutral:** a visit traced with `Chances.clock = Some(weekday 16:xx)` is identical to one with `None` (reuse `Trace` from golden.rs). This guards the goldens without re-recording them.
- **One game day** from Monday 06:00 at one seed. Each slot is reached; she sleeps in the night slot; she is away in the school slot except for dash-ins; a resident leaves and comes back.
- **Calendar owed once a day:** two visits with the same `local.date` play it once; a third visit on the next date plays it again; `local: None` never plays it.
- **At most one unseen rare a visit:** structural (a slot drawn at `begin_visit`, mod.rs:1147, from the visit seed plus a salt), so check it over 1000 seeds of `begin_visit` alone, plus one short sim with every cfg(test) rare row eligible. "Seen rares" are written only when non-empty.
- **Protected cells while away or dashing in:** a *new* proptest with `start in 0..WEEK`, kept out of `long_visits_never_touch_what_is_protected` (tests.rs:383-430) so its strategies stay as they are. There are no houseguest `.proptest-regressions` files today.

**Shell quit drain (proposed test):**
- Extract `fn quit(guest, unsent, actions, now)`. On a `Quit` action at shell.rs:590-592 it folds the pending clock, `blocking_send`s `SaveHouseguest` (and any `ledger_unsent`), and only then sends `Quit`.
- Test it like `full_and_closed_action_queues_stop_recovery_without_waiting` (shell.rs:862): assert the receiver sees `SaveHouseguest` before `Quit`, and that a closed channel neither blocks nor panics.
- `break` paths can't save; the receiver may already be closed. Their loss is bounded by `FLUSH`. Document it.

## T9. Traps and risks

1. **The perf test is date-dependent.** `houseguest_visit_cpu_is_negligible` (dessplay-rendezvous/tests/perf.rs:591-631) runs the real `run_ui_loop`, so the shell stamps the **real date**, and it asserts `|V|` is on screen (626). Calendar content must not stop her arriving, cover `|V|`, or add steady redraws (the bound is 3% of a core). It is also the only test that sees `Local::now()`.
2. **`ledger_to_save` must not be dirtied by every tick.** Per-tick accrual that sets `unsaved` means a sqlite write per tick (map d15) and breaks `her_home_outlives_a_restart`'s "handed out once" check (tests.rs:2226).
3. **The drain on quit must come before `Quit`**, not after (run.rs FIFO, then close at 1150).
4. **Crash rollback:** after a crash, the clock resumes from the last flush. It is "monotone" over the persisted sequence; the in-memory value can roll back by up to `FLUSH × 6` game ms. State this in design.md.
5. **The wall-clock piece's minute hand** changes every 10 s real. It may wake `next_tick` only while that piece is shown, so the empty homes in goldens and perf get no new wakeups.
6. **Long-running tests at the default start:** the `live_in` 20-minute tests and `her_needs_shape_long_visits` reach 18:00; `furnished` reaches 17:00; `her_home_fills_up_over_visits` reaches about 21:00. Any routine change before about 21:00 needs these re-checked, beyond the 16:00–18:00 neutrality pin.
7. **Downgrade (d13):** an older build re-saves the ledger without the new fields, so the clock resets to 16:00 Monday and pity is lost. Option T5(b) avoids it for the clock. Otherwise document it in decisions.md.
8. **Off-screen `Act::Away`/`Out` already exist** for the part-time job (osaka.rs:424-436; `SHIFT_MS` at 1754). A visit-level routine absence (a resident leaves and comes back) should reuse them rather than add a `State`. If it does add a `State`, golden.rs, `next_tick` and `present()` (mod.rs:644) all have to handle it.

---

<!-- section: gaps -->

## Gaps and corrections

All checks were made against HEAD `0661710` on 2026-10-04. Paths are under `dessplay/src/ui/houseguest/` unless prefixed. **(proposed)** marks something that does not exist.

### G1. Spot-checks of the most load-bearing claims

| Claim (section) | Verdict |
|---|---|
| `enum State` has 4 variants at mod.rs:353-360 (visit-lifecycle) | Correct |
| `advance` 701-739 and `next_tick` 742-761; the Absent arm wakes only at `quiet_since + delay` (all) | Correct |
| The shell's timeout arm calls `guest.advance` at shell.rs:485 and redraws only when it returns true; the per-input `advance` is at 505; `next_tick_hint` is at most 1 s (app.rs:711-725) (ledger, shell-tests) | Correct. `advance` runs at least once a second in every state |
| `VERSION`/`KEY` (ledger.rs:24/27), `Saved` 257-271 with `unsettled` last and the only skipped field, `Raw` 284-303 with no `deny_unknown_fields`, the `ends_with` pin at ledger.rs:476 (ledger) | Correct. VERSION is on line 24, not 25 |
| decisions.md:2075 still names the key `houseguest` (ledger) | Correct, and stale. The comment at ledger.rs:26 gives the reason: that key is the arrival setting |
| brain.rs:811 `for &Factor::InChat(times) in def.factors` with `times < 1.0`; `in_chat` overwrites `factors` (413-418); `choose` truncates after factors (577-579); the closure at osaka.rs:3132-3135 is an exhaustive match (mind) | Correct |
| golden `finish` hashes `guest.ledger.to_json()`; `drive` clamps steps to 1..1000 (shell-tests, ledger) | Correct. The file is `tests/golden.rs`, not `golden.rs`; several sections cite it bare. `census.rs` is `tests/census.rs` |
| `SHIFT_MS` is drawn from the body rng at osaka.rs:1754, 4321 and 4365; `may_work` is computed at 3020 and 3074-3077 (visit-lifecycle) | Correct |
| `Pose::Carry` is the leek pose (sprite.rs:66-68; art.rs:530-539) (visit-lifecycle, scripts) | Correct |
| Trap 1 in visit-lifecycle: a goodbye shows her while she is hidden | The code path is confirmed. `with` comes from `terrain::image(osaka.x, osaka.y, …)` with no `hidden` check (mod.rs:1066). `Leaving` paints `Stand/Surprised` and then `Wave` whenever `leaving.image` is `Some` (mod.rs:814-840). It has not been run, so it stays PLAUSIBLE |
| `IdleView` derives `Default` (idle.rs:69) (shell-tests) | Correct |
| Mind map: "`Chances` … its two production literals list every field" | **Incomplete.** `begin_visit` sets `chances: Chances::default()` (mod.rs:1161), so the first tick of every visit sees `clock: None`. See G2 |

### G2. Correction: `Chances` is the wrong carrier for game time

Mind-offering and shell-tests both route the clock through `Chances.clock`.
- `Chances` is rebuilt **only in `paint`** (mod.rs:1007, 1042), and `advance` hands the *previous paint's* copy to `osaka.tick` (mod.rs:724-726).
- `paint` runs only when `advance` returns true or input arrives. During a long sleep, or a hidden `Away`/door gap, the clock the mind sees can be minutes stale. On the first tick of a visit it is `None` (from `begin_visit`).
- `Osaka::tick` (osaka.rs:1668-1700) catches up through up to 64 wakeups, each at its own `due`. A single minute snapshot is wrong for every decision in the batch except the last.
- **Fix (proposed):** pass an affine `GameClock { at: u64 /*monotonic ms*/, game: u64 }` into `tick`, with `fn at(t) = game + 6·(t − at)`. Decisions then read `clock.at(at)`. `Chances` keeps `advert` and the other paint-derived fields, and the clock leaves it. The `None` neutrality argument still holds: the parameter becomes `Option<GameClock>`.

### G3. Contradictions between sections the builder must resolve

1. **How the clock is stored.**
   - Ledger: `clock: u64`, game **minutes elapsed** since the first meeting. The default is 0 and it is skipped at 0.
   - Shell-tests: game **ms since Monday 00:00**. The default is `START` and it is skipped when `is_start`.
   - Both keep `missing_fields_default` and `the_record_as_written`. Pick one. "Elapsed from START" makes the START constant part of the format forever; the absolute form makes the epoch part of it. Either way that is a doc-level invariant.
2. **Where accrual happens.**
   - Ledger: only in `advance`, never in `paint` (it would break the furnish diff at mod.rs:895-911).
   - Shell-tests: in `advance` **and** `paint`, into a pending field.
   - Both work if readers never mutate. Under G2, `game_time(now)` is a pure function of `(ledger.clock, pending, anchor, now)`, so `paint` needs no accrual at all.
3. **Clamping a step.**
   - Ledger clamps each `dt` to `MAX_STEP` (about 60 s).
   - Shell-tests says nothing caps it. `a_busy_client_gets_no_visit` jumps 1 h (tests.rs:127-137), and the T7 day census needs unclamped steps while Absent. tests.rs `run` (93-106) jumps straight to `until` when `next_tick` is `None`.
   - A clamp silently loses game time in every such helper. `Instant` already excludes suspend (`CLOCK_MONOTONIC`), so the only thing the clamp protects against is SIGSTOP or a debugger pause. Recommend no clamp, or a generous one (hours), stated in design.md.
4. **The flush threshold.** Ledger: 30 game minutes (5 min real). Shell-tests: 60 s real (6 game minutes). This trades sqlite writes against crash loss. Pick one constant.
5. **Gating the clock** (always / once `visits > 0` / only while visits are on).
   - Ledger leans to "once visits > 0".
   - Shell-tests' headless proptest ("cheap with `State::Absent` and `delay: None`") and the perf test (`Guest::new(rand::random())`, perf.rs:608) assume it runs anyway.
   - Under the gated options that proptest accrues nothing. Decide first, then write the tests.
6. **Where the routine departure hooks in**, three proposals:
   - visit-lifecycle: `advance`'s Visiting arm calls `osaka.go_away`;
   - scripts: a reflex right after the errand reflex (osaka.rs:2945);
   - mind: a reflex after the `Ctx` literal (3028), because `plan` needs the `Ctx`.

   A reflex fires only at a decision. It cannot cut a running `Use` (up to 3 min real today, and the full night under the night-long sleep). Recommend both:
   - the guest or `tick` detects the boundary and calls `interrupt(Cause::Routine)` (proposed) when the act is not already a departure;
   - the single reflex then does the leaving.

   Write down which one owns the decision.
7. **Errand policy versus the stage invariant.**
   - visit-lifecycle option (i) gates `nudge_due` on the routine.
   - Shell-tests T4 says the away gate must leave `cue` and `send` alone.
   - (i) is compatible with T4 only if the gate sits in `nudge_due` and not in `send`. Then (i) also contradicts D:1880 ("whatever the idle gate says"). This is the user's call either way.
8. **Neutrality at 16:00.**
   - Shell-tests T5 rule 1 requires every factor to be exactly 1 on weekdays 16:00–18:00, or the frames move.
   - Mind §6 and HG want snack and sofa boosts from 15:30.
   - Mind lists this as open. Shell-tests states it as a requirement. One decision is needed: delay the boosts to 18:00, or re-record the goldens deliberately.
9. **Gating homework against the exam-season calendar.**
   - Mind proposes gating `Use(Homework)` to 20:00–22:30 on school days.
   - Scripts proposes exam season as an owed `Cue::Splice(Chopsticks)` once a visit. Chopsticks wraps a homework use.
   - Under the gate, an owed chopsticks splice outside the window never plays, and the cue sits in its slot. Either the calendar slot waits for the window, or homework is boosted rather than gated.
10. **School days.** Mind: `DayTime.school_day` comes from the game weekday. Scripts 4.11/5.4: summer vacation on the real date. These are compatible only if `school_day` is computed with the real date in hand, which means `LocalTime` reaches routine code. That contradicts visit-lifecycle §8.6 ("the routine … independent of `local`"). Decide.
11. **`LocalTime.minute`** (shell-tests T3). Mind §3 says the mind must never read real time of day. Carrying `minute` on the view invites exactly that. If `biblical_date` is chosen, compute the date in the shell and stamp only the `NaiveDate` (`timeutil::biblical_date` takes epoch ms and reads `Local` itself, timeutil.rs:10-14).

### G4. Seams nobody covered

1. **The "visit" stops being a meaningful unit under option A.** Several per-visit mechanisms assume visits are short:
   - `LINE_BUDGET` = 8 Beat lines **a visit** (mind.rs:735, 873). A resident who lives through school inside one `Visiting` runs out of beats for the rest of a multi-hour visit.
   - Rares are drawn once per visit (mind §5). A resident (on by default) whose visit spans a game day or more gets one roll. "Unseen rares prefer the first 10 minutes" (HG:322) is meaningless for her.
   - "Calendar owed on the first visit of the day" (scripts §5.1) is set in `begin_visit`. A resident still visiting across real midnight never gets the new day's item. The owed check must also run at decision time against `local.date`.
   - `worked` is once a visit, the mood is per visit, and greetings fire only on arrival.

   This couples the away design (A/B/C) to the rarity and calendar design. A routine absence that *ends* the visit (B: door out, then a fresh `begin_visit` on return) keeps every per-visit mechanism meaningful. Under A each one needs re-keying to the game day or real date. No section names this coupling.
2. **How much of the time she's actually home.** At 6× a game day is 240 real minutes. On a school day she is asleep 85 min, in the morning 15, at school 70, and home awake 70 (about 30%). Over a week she is home and awake about 44% of the time dessplay is open.
   - HG's table assumed the **real** wall clock, where evening sessions coincide with her home time ("evening visits feel like her real home time", HG:278).
   - A free-running game clock loses that alignment. Users will mostly find her away or asleep.
   - This is a pacing decision for the user: school length, whether sleep counts as "present" (it does in visit-lifecycle §7), and the ratio. No section raises it.
3. **Dash-ins and the tier system.** HG #47 "Forgot lunch" is **[U]** (HG:485–486), and #46 toast morning is [C, school days 07:00–08:30] (HG:483).
   - Mind §5 puts tiers on splices and scripts only. Visit-lifecycle §6 hashes the dash-in schedule from the seed.
   - Neither says whether a dash-in counts against tiers, pity or "at most one unseen". #46 (the departure vignette, a toast pose that is new art) is unmapped by every section.
4. **There is no rare content yet.** Of the 17 [R]/[L] entries in HG, none is built. The 5a vignettes are [U]/[C]: surfing #37 U, chopsticks #40 C, andagi #41 U, riddle #56 U, lamp #70 C.
   - U and C have no pity bound (HG:314-319), and these rows already carry splice `chance`s.
   - 5b must either promote some content to R or ship the rarity machinery with only cfg(test) rows. Say how tier rarity combines with the existing `chance` rolls, or two rarity systems will stack.
   - The HG table speaks of a "share of selections". Gating *offering* per visit is a reinterpretation the user should sign off in design.md.
   - Name clash: `rules.rs:355` already has `pub tier: u8` (repair cost). Call the new type `Rarity` (proposed) or similar.
5. **Clock-keyed content HG already lists.** Chopsticks is "[C; before homework, **and on the hour**]" (HG:472), which is a game-hour trigger. #71 Window is "day sun / night moon from the wall clock, **a cloud drifting across**" (HG:551-552), which is an animation room-art didn't cost (repaint cadence and images). Room-art O5 assumes static sky phases.
6. **The dissolve paints props in their default looks.** `Leaving` calls `paint_prop_art(…, &Looks::default())` (mod.rs:810-812), and `with` pieces use `PieceState::Plain` (mod.rs:834-840).
   - A clock or window with a data-carrying `PieceState` (room-art option 3a) snaps to `Plain` during the goodbye: hands vanish or jump, and the sky resets.
   - `Plain` must be a sane look for these pieces, or `Leaving` must carry the `World`/`Looks` it froze.
7. **The cat.** It is computed only in the Visiting paint: `cat_now || cat_home(ledger)` (mod.rs:1014, 1049, 1079). `cat_home` keys on `visit_seed(visits−1) >> 17` (mod.rs:2239-2246).
   - Under B, the empty-home paint must compute it too.
   - If a dash-in or homecoming counts as a visit, the cat appears or vanishes mid-day. That may read fine ("came home with her"), but decide it.
   - The brief's "a cat who ignores the clock" is "later". 5b must not tie the cat bed's occupancy to the routine by accident, for example through a `piece_state` that takes the clock.
8. **The resident's focused-pane rain for the empty home.** D:1860-1866: "everything of hers in it … rains out at once". This is done through `fades` and `gate` inside Visiting. B's home-without-her state needs the same treatment when the pane gains focus, or her furniture stays in a newly protected pane. Visit-lifecycle §5B mentions only `blocked = view.protected`.
9. **Settings.** D:1902-1905 has Visits, Resident and Moved-out only. HG:776 planned "**Night stays** on/off (off = she leaves at bedtime instead of sleeping over, for shared screens left on overnight)". It is directly relevant now that sleep is clock-driven. Neither the settings screen (docs/proposals/2026-07-12-settings-screen.md), a possible "keeps hours" toggle, nor the move-out confirmation text ("next visit is a first meeting": does the clock reset too? the ledger map says yes) is mapped.
10. **CHANGELOG.** No section mentions it. The routine, the calendar dates, the window and clock pieces, and the night sleep are all user-visible and need `CHANGELOG.md` entries under the commit date (CLAUDE.md "Changelog"). A clock that starts at 16:00 for *existing* users on upgrade (ledger P3) is itself visible. An entry is also how users learn why she is "at school".
11. **Other docs.**
    - D:1919-1921 says "she redraws only when her pose changes, never at a fixed frame rate". The clock and window repaints (room-art seam 5) conflict with it.
    - D:1910 "saved whenever it changes" conflicts with batched clock flushes (ledger P7 covers this).
    - D:1562-1568, the part-time job "at most once a visit, ≥3 min in", must be restated against the routine.
    - HG:276-278 "Local wall clock, injected" is now wrong; the routine is on the game clock.
    - plan.md Phase 38 status.
    - CLAUDE.md must list any new `phase5b-design.md` (project rule).
    - docs/memory-profile-2026-09-19.md and plan.md's image numbers change with the dial images.
12. **`--dump` doesn't show her record.** `dump.rs` has a `settings` section built from `Settings` (dump.rs:162). `houseguest_ledger` isn't included, so "what time is it for her?" has no inspection path. A `houseguest` section (proposed: game time, weekday, pity, seen) would serve the dump-state skill.
13. **Concurrency is not a risk.** `instance_lock.rs` holds an exclusive flock per DB, so two processes never accrue into one ledger. Say so in decisions.md so nobody designs a merge.
14. **Restart while she's away.** Every process restart (the launcher, `exec_self` on resync, Q2's final save) starts Absent. The ledger map's Q2 saves the clock, but no section says what a restart *during* school shows. The visit-lifecycle §7 row "Resident, Absent at a boundary" covers the arrival gate; B's empty home also has to appear from cold start, without a departure having happened in this process.
15. **Act lengths are real time; routine windows are game time.** `use_duration` (osaka.rs:915-927) gives a homework use 30-60 s real (3-6 game minutes), and `COOLDOWN 0.4^repeats` penalises repeating it. A 2.5-game-hour homework window (25 real minutes) cannot be "homework at the desk" with today's lengths and cooldown. Mind §6's boost ×3 makes homework likelier but doesn't fill the window. Decide whether in-window acts lengthen (as the night-long sleep does) or whether the window just tints her choices.
16. **The pity unit.** HG gives the pity bounds in "cumulative idle **hours**" (HG:314-319: R within about 6 h, L within about 40 h). Ledger P4 offers present minutes or client-idle minutes, in game or real time. At 6× those differ sixfold. Pin the unit (real idle minutes matches HG) and say whether time while she's away or asleep counts.

### G5. Smaller corrections

- **Ledger P5, Q2:** "keep `run_ui_loop` free of `#[must_use]`". It is `pub fn run_ui_loop<A: TerminalAdapter>` at shell.rs:394, used at perf.rs:426 and 608. Correct as stated. Note that `run_ui_thread` (shell.rs:226) is the production entry and also needs the new return type.
- **Visit-lifecycle §1:** "Visiting → Absent: visits switched off (no goodbye)" at `observe` 1312-1315 is correct. Note also that `observe` sets `quiet_since = now` on every `!open` paint (1309). A clock-driven homecoming predicate (§7) must not be defeated by that either. Only `activity`, chat and failed arrivals are listed.
- **Mind §1:** "the stage does not call `bind`". True, but the stage builds its own full `Chances` literal (mod.rs:1007-1026). If the clock stays on `Chances` despite G2, cues get it there too.
- **Scripts trap 8**, which asks whether `furnish` keeps pieces clear of a hidden Osaka. The comment at mod.rs:891-894 says pieces stand "clear of … her". `furnish` reads `visit.osaka` (mod.rs:1644) and takes the **body rng** (`&mut self.rng`, mod.rs:906). So under A a door-in-place absence can still push pieces aside around an empty spot. Under B, painting the home outside Visiting either draws from the body rng in Absent (harmless, given the reseed at arrival, but outside invariant 2's spirit) or needs a furnish without the rng. Still open; verify before choosing A.
- **Room-art trap 2:** the image-count table should also count the `Leaving` frames (G4.6) and B's empty-home paint, which builds images with no Osaka in them. Both are small.
