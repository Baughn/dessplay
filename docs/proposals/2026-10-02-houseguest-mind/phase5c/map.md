# Phase 5c (stillness): code map

Written 2026-10-05 by six readers and a completeness critic at HEAD cabf5d1. Paths are under `dessplay/src/ui/houseguest/` unless prefixed. Anything marked (proposed) does not exist yet. The critic's corrections are at the end and override the sections.

---

<!-- section: brain -->
# Code map for 5c: brain.rs and mind.rs (needs, wants, choosing, methods, spots)

Paths are relative to `dessplay/src/ui/houseguest/`, at HEAD cabf5d1. Rules in
prose: docs/design.md:1445-1500 ("Choosing what to do"). Anything that does not
exist yet is marked **(proposed)**.

## 1. Needs (brain.rs)

`enum Need` has ten variants (brain.rs:22-48). `Need::ALL` (brain.rs:51-62) is
also the index order of `Needs.levels`.

| Need | `rise_ms` 0→1 (brain.rs:67-81) | `arriving` (brain.rs:87-95) | rises only when (`Needs::pass`, brain.rs:450-455) |
|---|---|---|---|
| Sleepy | 15 min | 0.0 | always (not while asleep) |
| **Restless** | **90 s** | **0.7** | always |
| **Tidy** | **60 s** | 0.5 | `Rising.mess` (`!chances.pulls.is_empty()`, osaka.rs:5517) |
| Mischief | 4 min | 0.2 | always |
| Hungry | 20 min | 0.2 | always |
| Comfort | 8 min | 0.5 | always |
| Fun | 6 min | 0.5 | always |
| Daydreams | 10 min | 0.5 | always |
| Nesting | 3 min ("a starting value", brain.rs:77) | 0.0 | `Rising.grieved` |
| Beauty | 20 min | 0.3 | `Rising.plain` |

- **Arrival by slot** (`arriving_in`, brain.rs:100-108): Sleepy 0.3 Evening, 0.5
  Homework, 0.8 Asleep; Hungry 0.6 Morning/Evening; otherwise `arriving`.
- **Clock rate** (`rate_in`, brain.rs:114-121): Sleepy ×`SLEEPY_BY_DAY` 0.3
  (Morning/Away/Afternoon), ×`SLEEPY_AT_NIGHT` 3.0 (Homework/Asleep); Hungry
  ×`HUNGRY_AT_MEALS` 2.0 (Morning/Evening). Restless and Tidy have no clock
  rate. `clock_rate(None, _) == 1.0` (brain.rs:152-154).
- **The cap:** mood rate × clock rate is capped at `RATE_MAX` 4.0
  (brain.rs:148, 462).
- **While asleep**, a need other than Sleepy rises at `ASLEEP_PACE` 0.25 of its
  pace, and Sleepy doesn't rise at all (brain.rs:394, 447-461).
- **When needs rise:** `Needs::pass` is called in one place,
  `choose_next`, with `at - self.decided` (osaka.rs:5522-5526). Needs rise
  **between decisions**, so during a 60 s still act Restless climbs 60/90 ≈
  0.67 before the next roll. This is the feedback loop: a longer still act on
  its own makes the next walk likelier unless Restless also slows.
- **Serving a need:** `Osaka::serve` (osaka.rs:5032-5044) applies
  `amount × share × quality(need, spot) × fresh` for each `serves` entry, then
  calls `enjoyed`. `credit_done` (osaka.rs:4945-5020) credits an Idle or Use
  act by the share of its span done, and a Pull by its offset/goal.
  **Walk and Travel are credited a whole 1.0 as she sets off, at `Spot::Any`**
  (osaka.rs:5705-5708): a 1-cell walk and a 60-cell walk ease Restless by the
  same 0.4.
- **Quality by spot** (`quality`, brain.rs:337-346): Comfort and Sleepy on
  `Spot::Real(item)` get `item.spec().comfort`, which is 1.0 for every piece
  (room.rs:173-334). Comfort Made 0.6 and Floor 0.1; Sleepy Made 0.7 and
  Floor 0.3; every other pair 1.0.
- **Fun tolerance:** there are seven `FUN_SOURCES` (brain.rs:351-359), with
  `TOLERANCE_PER_USE` 0.6 and `TOLERANCE_MS` 10 min (brain.rs:361-363).
  `fresh()` scales the Fun term (brain.rs:480-485, 805-809).
- **"Pressing"** is `Needs::pressing` (brain.rs:474-477): the need is felt
  (> 0) and no other need is higher (ties count). **It plays no part in
  scoring.** Its only readers are `advert` (decor first on the shopping
  channel, mod.rs:138) and the census printout (tests/census.rs:503). The
  "pressing" in the module doc (brain.rs:9-10, 795) only describes the squared
  term in the fit.

## 2. Moods (brain.rs)

`enum Mood { Ordinary, Lazy, Industrious, Dreamy }` (brain.rs:159-168).

- **Assignment:** `Mood::of(seed)` takes a splitmix hash mod 10 (brain.rs:176-186):
  0-4 Ordinary (50%), 5-6 Lazy (20%), 7-8 Industrious (20%), 9 Dreamy (10%).
  Fed, the seed is `day_seed(master, day)` (brain.rs:259-264), set at visit
  start (mod.rs:2349-2356) and in `begin_day` (osaka.rs:4002). Unfed, it is
  the visit's seed (mod.rs:2354).
- **`Mood::rate`** (brain.rs:189-205):
  - Lazy: Comfort 2.0, Sleepy 1.5, **Restless 0.4**, Tidy 0.7.
  - Industrious: Tidy 1.6, **Restless 1.5**, Comfort 0.5, Sleepy 0.8,
    Daydreams 0.6, Nesting 1.6.
  - Dreamy: Daydreams 2.5, **Restless 0.7**.
  - Everything else 1.0.
- **What a mood touches:** besides the rates, only `home_acts` (Lazy 0,
  Ordinary/Dreamy 1, Industrious 3; brain.rs:209-215), `greeting` and
  `wake_line`. **A mood changes no base, no duration and no factor.** There is
  no `Factor::Mood` variant (brain.rs:543-553). `Mood::rate` is the whole
  surface by which mood reaches behaviour today.
- **Restless refill after one Walk** (Walk serves 0.4, so 0.4 must come back),
  at clock rate 1: Ordinary 36 s, Lazy 90 s, Industrious 24 s, Dreamy ≈ 51 s.
  - At the brief's ~5 min (300 s) Restless: Ordinary 120 s, Lazy 300 s,
    Industrious 80 s, Dreamy ≈ 171 s.

## 3. Wants and the DesireDef table (brain.rs)

`enum Want` (brain.rs:519-539): `Stand, SpaceOut, Sneeze, Idle(Activity), Walk,
Travel, Pull, Swap, Use(Use), Work, Arrange`. `Want::ALL` lists 27 of them
(brain.rs:636-664). The fields of `DesireDef` are `base`, `serves`, `own_sake`
and `factors` (brain.rs:558-566). The table is `Want::def` (brain.rs:667-746).
The chat factor is `CHAT_FACTOR` 0.1 (osaka.rs:169), applied through
`IN_CHAT`/`CHAT` (brain.rs:569-571).

| Want | base | serves | own_sake | factors | class |
|---|---|---|---|---|---|
| Stand | 4 | — (NEUTRAL fit 0.5) | | | still (filler) |
| SpaceOut | 6 | Daydreams 0.6 | | | still |
| Sneeze | 2 | — | | Season(HAY_FEVER) ×3 | still (accident) |
| Idle(Stretch) | 4 | Restless 0.5 | | Clock(Morning) ×2 | exercise, in place |
| Idle(LieBack) | 4 | Sleepy 0.15, Comfort 0.3 | | | still |
| Idle(Sit) | 4 | Sleepy 0.1, Comfort 0.3 | | | still |
| Idle(Jacks/ToeTouch) | 6 | Restless 0.5 | | | exercise, in place |
| Idle(LieFront) | 6 | Daydreams 0.3, Comfort 0.2 | | | still |
| Idle(Gaze) | 6 | Daydreams 0.5 | | Clock(DUSK_TO_DAWN) ×2 | still |
| **Walk** | **14** | Restless 0.4 | | | **movement** |
| **Travel** | **10** | Restless 0.4 | | in chat | **movement** (climb/drop/clamber/door) |
| **Pull** | **16** | Tidy 0.6 | | in chat | walk to text, then the pull (mischief) |
| Swap | 8 | Mischief 0.8, Fun 0.3 | | in chat | walk to text |
| Work | 9 | Restless 0.5 | yes | | out (moving) |
| Use(Watch) | 10 | Fun 0.6 | | chat, Clock(Evening) ×2 | walk to seat, then still |
| Use(Sleep) | 10 | Sleepy 0.7, Comfort 0.3 | | in chat | walk, then still |
| Use(Unpack) | 40 | Fun 0.8 | yes | in chat | walk, then a chore |
| Use(Crumple) | 12 | — (no method: mind.rs:215) | | in chat | step of a build |
| Use(Nap) | 8 | Sleepy 0.1, Comfort 0.4 | | in chat | walk, then still |
| Use(Lounge) | 8 | Comfort 0.5 | | chat, Clock(Afternoon) ×2 | walk, then still |
| Use(Homework) | 8 | Daydreams 0.3, Comfort 0.2 | | chat, Clock(Homework) ×3, EXAMS ×2, PANIC_WEEK ×2 | walk, then still |
| Use(Read) | 8 | Fun 0.5, Daydreams 0.2 | | chat, Clock(EveningOff) ×2 | walk, then still |
| Use(Snack) | 8 | Hungry 0.8, Fun 0.1 | | chat, Clock(Afternoon) ×2 | walk, then brief |
| Use(Pet) | 8 | Fun 0.6 | | in chat | walk, then brief |
| Use(LookOut) | 8 | Daydreams 0.5, Fun 0.3 | | chat, Clock(DUSK_TO_DAWN) ×2 | walk, then still ("spacing out" in the census, osaka.rs:5252) |
| Arrange | 6 | Nesting 1.0 | | | walk, lift, carry, set down |

`BOOST` is 2.0 and `BOOST_STRONG` 3.0 (brain.rs:574-577). The factor slices are
at brain.rs:580-610. `factor()` multiplies a want's factors together
(brain.rs:753-775).

### Durations of the acts a want becomes (osaka.rs)

| Act | Range | Ref |
|---|---|---|
| Stand (chosen) | 2-5 s | osaka.rs:5739-5741 |
| Stand ("nothing bound") | 2 s | osaka.rs:5720 |
| SpaceOut / Muse | **`SPACE_OUT_MS` = (6000, 14_000)** | osaka.rs:419 |
| Sit | 10-25 s | osaka.rs:1216-1226 |
| LieBack | 15-40 s | same |
| LieFront | 10-25 s | same |
| Jacks | 4-8 s | same |
| ToeTouch | 5-9 s | same |
| Stretch | 2-4 s | same |
| Gaze | **4-10 s** | same |
| Lounge | 15-30 s | `use_duration`, osaka.rs:1131-1145 |
| Nap | 30-60 s | same |
| Sleep | 60-180 s | same |
| Homework | 30-60 s (120-240 s in its slot, `HOMEWORK_IN_SLOT_MS`, osaka.rs:1148, 1154-1159) | same |
| Watch | 20-45 s | same |
| Read | 20-40 s | same |
| LookOut | **15-30 s** | same |
| Snack | 6-9 s | same |
| Pet | 6-9 s | same |
| Unpack | 4-6 s | same |
| Crumple | 4-6 s | same |
| Walk | `WALK_MS` 333 per cell (3 cells/s) | osaka.rs:358 |
| Climb | `CLIMB_MS` 500 per row | osaka.rs:360 |

Every range is drawn from the body stream (`rng.range`). Changing a range keeps
the number of draws but changes every value drawn.

## 4. Scoring and choosing (brain.rs:777-857)

- Constants: `WEIGHT` 2.0, `INERTIA` 3.0 (pub), `FLOOR` 0.1, `NEUTRAL` 0.5,
  **`COOLDOWN` 0.4**, **`TOP` 4** (brain.rs:779-789). `RECENT` 3 (osaka.rs:410).
- `score(want, spot, needs, recent) -> f64` (brain.rs:793-817):
  - The fit is `NEUTRAL` if the want serves nothing.
  - Otherwise the fit is `FLOOR + Σ level² × quality(need, spot) × fresh × amount × WEIGHT`.
    If `own_sake`, it is `max(NEUTRAL, fit)`.
  - The score is `base × fit × COOLDOWN^(times want is in recent)`.
- `choose(offers: &[(Want, Spot)], needs, recent, factor: &dyn Fn(Want)->f64,
  whims: Whims, attempt: u64) -> Option<(usize, Vec<(Want,f64)>)>`
  (brain.rs:823-857):
  1. Score × `factor(want)` for each offer.
  2. Sort descending and **truncate to `TOP` = 4**.
  3. Roll `whims.below_at("roll", attempt, 1000)/1000 × total`.
  4. Pick by cumulative weight.
- **Because of the truncation, any multiplier below 1 can delete a want.** This
  is why `sane_factor` requires Clock/Season factors > 1 and InChat factors < 1
  (brain.rs:1110-1115, linted for every row at 1147-1149).
- **Caller:** only `Osaka::choose_next` (osaka.rs:5385-5724). The steps:
  1. **Reflexes.** Errand, no floor, routine (home, bed, school), shift, off
     text (`find_rest`), dash, watching chat, calendar beat, day-off line,
     owed beats.
  2. **Continuations.** `heading/hop`, `heading/mine`, `arrange_next`,
     `leftover`.
  3. **Offers:** every want whose `mind::bind` returns a bind (osaka.rs:5638-5644).
  4. **Heading.** A heading found again replaces its want's offer and gets
     `INERTIA` ×3 in the `factor` closure (osaka.rs:5648-5677).
  5. **Loop over `attempt`.** `choose`, `offers.remove(i)`, `plan`. On success
     the want is pushed to `recent` (capped at `RECENT`), and Walk/Travel are
     credited at once (osaka.rs:5684-5717).
- **Cooldown counts the same want only** (`w == want`, brain.rs:815). Walk and
  Travel cool apart, and so do Idle(Sit) and Idle(LieBack).

### Illustrative scores

Computed with the table above; quality and fresh are 1 unless noted; floor
Sit/LieBack/LieFront use Floor quality.

- **Arrival** (R 0.7, T/C/F/D 0.5, M 0.2), home with text:
  - Top 4: Walk 6.89 (31%), Pull 6.40 (29%), Travel 4.92 (22%), Watch 4.00 (18%).
  - Just outside: LookOut 4.00, Read 3.60, Jacks/ToeTouch 3.54.
- **Arrival, bare room without text:**
  - Top 4: Walk 6.89 (36%), Travel 4.92 (26%), ToeTouch and Jacks 3.54 each.
  - The whole top four is movement or exercise.
- **After a walk** (R 0.3, T 0.3, M 0.4, C 0.5, F/D 0.4), bare room:
  - Top 4: Walk 2.41, Stand 2.00, SpaceOut 1.75, Travel 1.72.
  - She still moves 53% of rolls.
- **Floor Sit and LieBack are effectively dead while she's awake.** They score
  about 0.46 against a top-4 cutoff of 1.7-3.5 in every state above: Comfort's
  Floor quality 0.1 makes even Comfort 1.0 worth only +0.06 to the fit. Only
  Sleepy (Floor quality 0.3) brings them back. Bare-room stillness today is
  SpaceOut, Gaze, LieFront and Stand.

## 5. How a chosen want becomes an act (mind.rs)

- **The model** (mind.rs:1-7): each want has named methods with pure guards
  `fn(&Ctx, Whims, Want) -> Option<Bind>`, tried in order. The first that binds
  wins (`bind`, mind.rs:240-249). A `Bind::Job` must stand on a platform.
  **Invariant:** a want is offered only if a method binds, so offering and
  planning can't disagree.
- **`Whims`** (mind.rs:33-84): one draw from the mind's stream per decision,
  hashed per label. Golden traces depend on the labels.
- **`Bind` variants** (mind.rs:124-136): `Here(Here)`, `WalkTo(i32)`,
  `Take(Link)`, `Door((i32,i32))`, `Work(Option<Link>)`, `Job(Job)`.
  - `Here` (mind.rs:112-120) is one of `Stand, SpaceOut, Muse, Sneeze,
    Idle(Activity)`.
  - `Bind::on()` maps a bind to a `Spot` for scoring (mind.rs:150-158): a Use
    job gives Real or Made, a Build gives Made, `Here(Idle)` gives Floor, and
    everything else gives Any. **A `Spot` carries no position.**
  - `Bind::spot()` gives the destination for Take/Door/Job (mind.rs:140-147),
    but **it is private** (`fn spot`). Only `in_chat` uses it (mind.rs:161-164).

The method table (`methods`, mind.rs:203-219; constants at mind.rs:178-200):

| Want | Methods (in order) | What binds | Act (`Osaka::plan`, osaka.rs:5729-5800) |
|---|---|---|---|
| Stand | `stand` | `Here::Stand` | Stand 2-5 s |
| SpaceOut | `space-out/muse` (1 in 3, mind.rs:223-225), `space-out` | `Here::Muse` / `Here::SpaceOut` | `Osaka::muse` (osaka.rs:2154-2264: escalator rare, hour glance, riddle or musing) / SpaceOut |
| Sneeze | `sneeze` | `Here::Sneeze` | Sneeze |
| Idle(a) | `idle` | `Here::Idle(a)` | `idle_act` (osaka.rs:6382-6398; Sit/Lie face a random way) |
| Walk | `walk/along` (mind.rs:253-264) | `WalkTo(x)` | `Act::Walk { then: Then::Nothing }` |
| Travel | `travel/link` (mind.rs:267-275), `travel/door` (mind.rs:278-298, only with no links) | `Take` / `Door` | `travel` (osaka.rs:6363-6374) walks to the link: `Act::Walk { then: Then::Link }`; `through_door` |
| Work | `work` (mind.rs:302-316) | `Work(out)` | `go_to_work` |
| Pull | `pull` (mind.rs:318-326) | `Job(Pull)` | `go_to` → `pursue` (osaka.rs:6348-6360): `Act::Walk { then: Then::Job }` |
| Swap | `swap` (mind.rs:329-340; none while one is owed) | `Job(Swap)` | same |
| Use(_) | `use/finish-my-heap`, `use/mine`, `use/real`, `use/made`, `use/make` (mind.rs:194-200, 344-464) | `Job(Use(seat))` / `Job(Build)` | same; `start_job` at the spot (osaka.rs:3386 on) |
| Use(Crumple) | none (mind.rs:215) | — | — |
| Arrange | `arrange/use-it`, `arrange/carry`, `arrange/lift` (mind.rs:189-193, 468-541) | Use / SetDown / Lift job | same |

**`go_to`** (osaka.rs:5885-5923):

- **Same floor:** `pursue`, a walk to the job.
- **Another floor:** a `Heading` is set and she takes the first link
  (`route`), or a door when there is no route.
- **Every job is therefore reached by walking**, and those walks count as
  "moving".

### Movement vs still, by what the body does

`census_group` (osaka.rs:5249-5283) defines the census buckets:

- **"moving":** `Walk, Climb, Clamber, Out, Away, Door, Fall, Peer, Dazed`.
- **Exercise, in place:** Jacks, ToeTouch, Stretch.
- **Mischief:** Pull, Swap, Giggle, Innocent, Tear, Sneeze, PutBack, Admire.
  The pull itself moves her, but it is bucketed as mischief.
- **Home:** Lift and SetDown.
- **Still:** Stand, Look, Glance, Home and Poke ("standing"); SpaceOut, Gaze
  and LookOut ("spacing out"); floor Sit/LieBack/LieFront ("floor rest");
  every other Use ("furniture").

**By want:**

| Kind | Wants |
|---|---|
| Pure movement | **Walk, Travel**, Work's walk out |
| Movement then still/chore | every **Use**, **Pull, Swap, Arrange** |
| Still from the start | **Stand, SpaceOut, Sneeze, Idle(Sit/LieBack/LieFront/Gaze)** |
| Exercise in place, no travel | **Idle(Jacks/ToeTouch/Stretch)** |

**Step 1's split needs no new state.** `Act::Walk { to, then: Then }`, with
`Then::{Nothing, Link, Job}` (osaka.rs:633-641), already tells wandering
(Nothing) from travel (Link) and walking to a job (Job). A Link walk can also
be a hop of a heading toward a job (`self.heading.is_some()`).

## 6. How a spot is picked for a want: no distance term anywhere

| Picker | Rule | Ref |
|---|---|---|
| `walk` | **Uniform column** on her platform (the chat 0.1), any `to != x` | mind.rs:253-264 via `pick` (osaka.rs:275-281) |
| `take_link` | Uniform over the links off her floor (the chat 0.1) | mind.rs:267-275 |
| `door_away` | Uniform over the other platforms, then a uniform column | mind.rs:278-298 |
| `pull`, `swap` | Uniform over every offer in `chances` (the chat 0.1) | mind.rs:318-340 |
| `places` + `place` | All seats for the use. Makeshift seats only if no real one of the kind, else 1 in `MAKESHIFT_ODDS` 20 (mind.rs:24, 382-431). Then **one uniform `w.below("place", n)`** (mind.rs:434-441) | |
| `use_real`, `use_made`, `use_make` | **Each calls `place` with the same whims and label**, so all three see the same pick and partition on its kind | mind.rs:443-464 |
| `pick_build` | Weighted: the chat ×0.1, facing a TV ×`FACING_TV` 5 | mind.rs:28, 546-576 |
| `lift` | A whim among `Trials::ties` (repair cost is cells the *piece* moves, rules.rs:351, 848-877, not her distance) | mind.rs:507-541 |
| `use_mine` / `finish_my_heap` | First match in `chances.mine` | mind.rs:344-367 |

**What already prefers nearness:**

- `leftover` sorts her own pieces same floor first, then by Manhattan distance
  (osaka.rs:5818-5832).
- `find_rest` goes to the nearest calm spot (design.md:2170).
- `beside()` takes the nearer side first (osaka.rs:248-268).

`night_seats` (osaka.rs:234-245) and `fridge_seat` (osaka.rs:224-231) take the
first seat in `chances.seats` order, which is shown-piece order (mod.rs:3400-3405).

**Walk length:** a uniform target column averages about a third of the floor's
width. For a 60-column floor that is about 20 cells, about 6.7 s at 333 ms per
cell, for the same 0.4 of Restless served.

## 7. Watch without a sofa

- **Watch binds with only a TV.** The TV's spec has `uses: &[Use::Watch]`
  (room.rs:184), and `spots_for` seats Watch `beside()` the TV (mod.rs:3446-3451),
  facing it (room.rs:961-968).
- **The sofa seat is an extra.** A sofa that faces the TV adds a second Watch
  seat (mod.rs:3467-3491).
- **Her pose is already sitting.** Away from a sofa the host pose is
  `Pose::Sit` (osaka.rs:7189-7194), and the script `WATCH` uses
  `Posed::Host` (script.rs:1360-1366).
- So a TV-only home already has "TV from the floor": she sits **beside** the
  TV. Sitting **before** it, cross-legged and facing out, is **(proposed)**
  new art and a new seat spot.
- `WATCH_FACTORS` boosts only the Evening (brain.rs:585-588).

## 8. Tests that pin today's numbers

**brain.rs unit tests** (brain.rs:859-1563):

- `a_sleepy_osaka_goes_to_bed` (968), `a_hungry_osaka_has_a_snack` (1025).
- `a_restless_osaka_mostly_moves` (1035): Walk + Travel > 50% at Restless 1.0,
  floor offers only. At base 8 it would still pass:
  - Walk 8×0.9 = 7.2, Travel 9.0, Jacks/ToeTouch 5.4.
  - Top 4: Travel, Walk, Jacks, ToeTouch, so Walk + Travel ≈ 60%.
- `sleepiness_draws_her_to_lie_down` (986): sleepy 0.9 lies back more than 2×
  as often as sleepy 0.3, with a cooldown.
  - At 0.3, LieBack (≈0.51) is outside the top 4 (Stand 2, Pull 1.6, Walk
    1.51, Swap 1.31). Its count comes only through cooldown.
  - Lowering Walk's base, or lengthening/strengthening floor rest, shifts this
    ratio. It is sensitive.
- `choices_vary_whatever_the_needs` (1047): at least 3 kinds, none above 1500/2000.
- `repeating_herself_is_discouraged` (1065): two repeats score below 0.2×,
  which pins `COOLDOWN` ≤ about 0.447.
- `every_want_has_a_sane_row` (1140), `the_lint_rejects_a_boost_that_is_not_one`
  (1120), `the_chat_keeps_its_factor` (1323, `CHAT_WANTS` lists 14):
  structural lints that hold any new row or factor.
- `a_boost_raises_its_want_only_in_its_time` (1360): `seen.len() == 8`
  boosted wants. A new Clock factor changes it.
- `looking_out_is_a_daydream_and_some_fun` (1197): pins LookOut's base 8,
  its serves, and `LOOK_OUT_FACTORS`. The window step (5) must update it.
- `moods_come_in_their_shares` (1207), `home_acts_by_mood` (1551).
- `nesting_rises_only_while_grieved` (1233): `minute(Ordinary) > 0.2` pins
  Nesting's 3 min.
- `beauty_rises_only_in_a_plain_room` (1259): 0.3 + 10 min / 20 min = 0.8
  pins Beauty.
- `needs_rise_by_her_day` (1434), `she_arrives_as_her_day_has_left_her` (1520).
- `a_nights_sleep_slows_her_needs` (1484): its 6 s span is chosen so that
  nothing reaches 1 ("tidiness rises in 60 s", 1494). That is safe if Tidy
  slows.
- No test pins `rise_ms` for Restless or Tidy directly.

**mind.rs tests** (mind.rs:1233-1629) cover lines, pools, scripts and
headings. **None pins a want's base, a method or a spot pick.**

**Elsewhere:**

- tests.rs:7737 `every_want_can_be_cued`: every new want needs a `Scene`
  (Stand and Walk are exempt).
- tests/census.rs:1435 `at_home_her_furniture_beats_the_floor`: furniture
  > 10× floor rest, in a furnished room, Ordinary. More floor stillness could
  break it.
- tests/census.rs:1451 `her_mood_shows`: lazy furniture > industrious, lazy
  exercise < industrious, dreamy spacing out > ordinary. Its doc says moving
  is "no measure", within a few percent.
- tests/census.rs:1607 reads `SPACE_OUT_MS.0`.
- Several tests serve Restless 1.0 to keep her still (tests.rs:9243, 10135,
  10891, 11140). Slower Restless only makes them more robust.
- **Goldens** (tests/golden.rs:563-765): 10 scenes × 4 seeds × ASCII and line
  art, plus 4 `UNFED_*` tables. **Any change to a base, rise time, duration,
  `TOP`, `COOLDOWN` or a whims label moves essentially every hash.** Re-record
  them with a reason.
- **Docs:** design.md:1445-1500 states the fit formula, "top four", the
  arrival levels, the mood multipliers, and "Repeating one of her last three
  choices multiplies by 0.4". A numbers change is a docs change too
  (CLAUDE.md: rule in design.md, reason in decisions.md).

## Levers and risks for 5c

**Step 1, measure.**
- **Plug-in:** split `census_group`'s "moving" by `Act::Walk.then`:
  - `Nothing`: wandering.
  - `Link` with no heading: travel.
  - `Job`, or `Link` with a heading: walking to a job.
  - Climb, Clamber, Fall, Peer, Dazed and Door: climbs and falls.
  - Out and Away: doors and routine.
- **New API:** a new accessor (proposed). `census_group` is `&'static str`
  and is also used by the day census (tests/census.rs:1177).
- **Risk:** low. Census tests only; no goldens move. The "mischief" bucket
  hides the pull's own motion: decide whether a pull counts as movement.

**Step 2, band test.**
- **What exists:** nothing per mood beyond `Mood::rate`, and `her_mood_shows`
  is the only mood-split census test.
- **Plug-in:** a band per room × mood needs fixed-mood runs, as `simulate(...,
  Some(mood))` already allows (tests/census.rs:286, 325).

**Step 3, cheap levers.**

- **Slower Restless and Tidy:** `Need::rise_ms` (brain.rs:70-71).
  - Restless 90 s → 300 s cuts the Ordinary refill from 36 s to 120 s.
  - Tidy 60 s → 240 s weakens Pull (base 16, a walk to the text).
  - Lazy (×0.4) then refills in 300 s and Industrious (×1.5) in 80 s, so the
    mood spread in movement comes through `Mood::rate` for free.
  - **Risk:** goldens. `Need::arriving` Restless 0.7 still makes the arrival
    restless; consider lowering it (design.md:1475 states 0.7).
- **Lower Walk base:** `Want::Walk => row(14.0, …)` (brain.rs:691).
  - Travel 10 and Pull 16 are the other movement-heavy bases.
  - Note the 3-recent cooldown alternation: Walk and Travel cool apart, so
    one can replace the other.
  - A mood-based dampener **cannot go through `Factor`**. A factor below 1 is
    forbidden by the lint and deletes wants under the top-4 truncation.
    Options (proposed): a `Factor::Mood(Mood, f64)` with the lint relaxed for
    moods, or put the per-mood difference into `Mood::rate` only.
- **Credit Walk by length, not 1.0** (osaka.rs:5705-5708), so a short wander
  doesn't serve less. **(proposed)**
  - Crediting by distance (say cells/20) would make short walks serve less
    and so be repeated more. Probably better is serve-on-arrival by distance,
    or a minimum walk length. Ask the user.
- **Longer still acts:**
  - `SPACE_OUT_MS` (osaka.rs:419) is read at osaka.rs:2247, 2270, 4498 (the
    calendar greeting's minimum), 5744 and 7446 (a test), and at
    tests/census.rs:1607.
  - `Activity::duration` (osaka.rs:1216-1226) is also used by
    `night_on_floor` (osaka.rs:3685) and the wake stretch (osaka.rs:3979).
  - `use_duration` (osaka.rs:1131-1145) feeds `use_range`, which script.rs:528
    uses for the shopping, surf and sunrise lengths.
  - `use_duration_in` + `HOMEWORK_IN_SLOT_MS` is the template for a
    slot- or mood-lengthened use.
  - **Feedback risk:** needs rise between decisions (osaka.rs:5523). Longer
    still acts without slower Restless just bank restlessness for the next
    roll.
- **Settling in.** No mechanism exists. The brain *fights* it today:
  - The cooldown ×0.4 applies to a repeat of the same want (brain.rs:815).
  - Finishing Sit serves Comfort and lowers its own fit.
  - Options (proposed):
    - a continuation in `choose_next` beside `leftover` and `heading` that
      chains Sit → LieBack → doze without calling `choose`;
    - or exempt still wants from `recent`;
    - or `INERTIA`-like bonuses for "a still want after a still act".
  - Floor Sit and LieBack also need a better fit to compete at all: Comfort's
    Floor quality is 0.1 (brain.rs:341). Raising it moves
    `a_sleepy_osaka_goes_to_bed` (floor < nap) and
    `at_home_her_furniture_beats_the_floor`.
- **Prefer the nearest spot.** Three places it could plug in:
  - (a) `place` (mind.rs:434-441): weight `places` by distance through
    `pick_weighted`. **Main risk:** `use_real`, `use_made` and `use_make` each
    recompute `place` with the same whims. Any weighting must stay a pure
    function of `(Ctx, Whims)`, or the three methods diverge, breaking the
    "offering and planning can't disagree" invariant (mind.rs:3). `Ctx` already
    carries `x`, `y` and `here` (mind.rs:87-108).
  - (b) `walk` (mind.rs:253-264): bias toward shorter walks or a nearby
    column. This directly shortens wandering.
  - (c) a distance multiplier in `choose_next`'s `factor` closure
    (osaka.rs:5669-5682). This needs `Bind::spot` made `pub(super)`, and a
    multiplier below 1 has the same top-4 deletion hazard as a Factor below 1.
  - `pull` and `swap` are uniform over every offer too (mind.rs:318-340).
    Whether "nearest" covers them is an open question.
  - Every option changes whims consumption or the picks, so goldens move.
- **Daydream sessions:** `muse` (osaka.rs:2154-2264) sets one SpaceOut with
  one line. A session would be a new `Play` or script with several musings,
  bounded by `LINE_COOLDOWN_MS` 10 min on each line (mind.rs:878) and by
  `MUSINGS`' 8 lines (mind.rs:673-687). **(proposed)** Cloud-watching on her
  back would be a LieBack play. **(proposed)**
- **TV from the floor:** Watch already binds TV-only, sitting beside it
  (section 7). Facing it cross-legged is new art and a seat spot. **(proposed)**

**Step 4, bare-room stillness.**
- **Floor homework:** a want or Use without furniture **(proposed)**.
  `Use(Homework)` binds only at a desk seat today (room.rs:213).
- **Makeshift desk:** a new `Build` kind. `places` would pick it up via
  `chances.builds` (mind.rs:387-393).
- **"×3 weight on make one while no real piece":** today `places` gives
  `Place::Make` equal odds with the seats (mind.rs:420-429, 439). There is no
  weight on the *want* for owning nothing. A Factor > 1 keyed on "no real
  piece" would fit the lint **(proposed)**.
- **Reading a pulled line (HG #72)** has no code **(proposed)**.
- **Cost:** each new want needs a `def` row, a `methods` entry, a Scene
  (`every_want_can_be_cued`), a `Want::ALL` entry and its count, and
  `FUN_SOURCES` if it serves Fun.

**Step 5, the window.**
- **LookOut today:** base 8, Daydreams 0.5 and Fun 0.3,
  `LOOK_OUT_FACTORS` (chat, dusk ×2), 15-30 s.
- **Rarer:** lower the base, or raise the Fun tolerance per use.
- **Longer:** the `use_duration` LookOut entry.
- **Risk:** `looking_out_is_a_daydream_and_some_fun` pins all of it. The
  afternoon clock glance competes inside `muse` (`HOUR_GLANCE` 1 in 3, once a
  visit, osaka.rs:425). Look-outs crowd out SpaceOut, which is how a glance
  happens at all.

**Open questions:**
1. Does "walking to a job counts" call only for lower Walk/Travel bases, or for
   a distance cost on Pull, Swap and Use approaches too?
2. Should "nearest" apply to `pull` and `swap` as well as `place` and `walk`?
3. Should the per-mood band come only from `Mood::rate`, or from a new
   mood-aware factor or duration?
4. Should settling bypass `choose` (a reflex/continuation) or stay inside it
   (cooldown exemptions)?
5. Should Walk's Restless credit stay whole at set-off?
6. Should arrival Restless 0.7 drop with the slower rise?


---

<!-- section: acts -->
# Code map for 5c: acts, still acts, walking, musings, Watch, the window and the clock glance

All paths are relative to `dessplay/src/ui/houseguest/` at HEAD cabf5d1. Every
ref was checked against the code. Anything that doesn't exist yet is marked
(proposed).

## 1. The act machine in brief

- `enum Act` (osaka.rs:440-606) is her whole body state: one act at a time,
  set by `Osaka::set` (osaka.rs:2742-2757). `set` credits what she leaves
  (`credit_done`), restamps `act_since` and recomputes `act_due = first_due(at)`.
- `tick` (osaka.rs:2664-2740) loops up to 64 times while `due() <= now`:
  routine cut first, then Dream, sleep talk, midnight snack, then speech end,
  then pending layer ops, then Stand blinks, then `fire` (osaka.rs:2759).
- `fire` (osaka.rs:2759-3354) is one match on the act. A finished act either
  chains to a fixed follow-up act (a few listed below) or calls
  `decide` (osaka.rs:5367) → `choose_next` (osaka.rs:5385).
- `Act::props` (osaka.rs:833-865) is the one exhaustive classification:
  `Stays::{Job, Rest, Pass}` and `OnChat::{Look, Landed, Back}`. Any new act
  fails to compile until it's classified there.
- The census reads `Osaka::census_group` (osaka.rs:5249-5284), also
  exhaustive (see §6).

## 2. Every act, with its duration

Constants: osaka.rs:358-433. Walking is `WALK_MS = 333` per cell ("3 cells/s:
dreamy, not brisk", osaka.rs:357-358), climbing is `CLIMB_MS = 500` per row
(osaka.rs:360), and a fall uses `GRAVITY = 28.0` rows/s² (osaka.rs:362).

| Act (osaka.rs line) | How long | What ends it (fire arm) | census group |
|---|---|---|---|
| `Stand {until}` (441) | 2000–5000 ms when chosen (`plan`, osaka.rs:5739-5741); 800 ms after a climb/clamber (3324, 2779), 600 ms after a short fall (3345), 1000 ms with no floor (5404), 2000 ms "nothing bound" (5721), 300/500 ms carry waits, `≤5000` ms chunks watching chat (5495-5503; since step 5 one Stand to `watch_until`, never more than `WATCH_MS` ahead) | `decide` (3030) | standing |
| `SpaceOut {since,until,play}` (445) | `SPACE_OUT_MS = (6000, 14_000)` (419) for a plain or musing space-out; 1500–3000 ms after undoing a swap (3040-3048); `CLOCK_GLANCE_MS = 2500` for a clock glance (421, 3826) | waits for each key of its play (3029), then `decide` (3030) | spacing out |
| `Idle {what,since,until,play}` (520) | `Activity::duration` (osaka.rs:1216-1226): Sit 10–25 s, LieBack 15–40 s, LieFront 10–25 s, Jacks 4–8 s, ToeTouch 5–9 s, Stretch 2–4 s, Gaze 4–10 s | `decide` (2762-2769), or `end_night` if it's her night on the floor | floor rest (Sit/LieBack/LieFront), spacing out (Gaze), exercise (5254-5258) |
| `Use {seat,since,until,whole,play,grievance}` (581) | `use_duration` (osaka.rs:1130-1145): Lounge 15–30 s, Nap 30–60, Sleep 60–180, Homework 30–60 (`HOMEWORK_IN_SLOT_MS = 120–240 s` in her Homework slot, 1148, 1153-1158), Watch 20–45, Unpack 4–6, Read 20–40, Snack 6–9, Pet 6–9, Crumple 4–6, **LookOut 15–30**; a trial is `TRIAL_USE_MS = 3500–5000` (404); plus any splice prelude/coda (3492, 3579) | `decide` (2905-2922); Crumple chains to `Admire` 1200 ms (2913-2920); night sleep → `end_night` | furniture, except LookOut = spacing out (5252) |
| `Walk {to, then}` (450) | path length × 333 ms | at `to`: `Then::Job` → `start_job`; `Then::Link` → Clamber / Out / Climb / Peer; `Then::Nothing` → 50% `Peer` at a screen-edge end (3236), else `decide` (3136-3246) | moving |
| `Peer {until, then}` (454) | `PEER_MS = 1200` (366) | a Drop link → `Fall`; else turns round and `decide` (3250-3271) | moving |
| `Climb {to_y}` (458) | rows × 500 ms | `Stand` 800 ms (3321-3328) | moving |
| `Fall {from_y,since,to_y}` (461) | gravity | `Dazed` if ≥3 rows, else `Stand` 600 ms (3329-3352) | moving |
| `Dazed {until}` (466) | `DAZED_MS = 1500` (364) | "...I'm OK." half the time, `decide` (3020-3028) | moving |
| `Look {surprised_until,until}` (469) | `SURPRISED_MS = 1200` + `LOOK_MS = 4000` (368-369); half for Restless/SeatGone/Shaken (6509-6514) | `decide` (3124-3135) | standing |
| `Pull` (474) | `heave_ms(glyphs) = 600·(40+g)/40` per cycle (353-355), goal gap + 2–7 cells (3637) | `finish_pull` | mischief |
| `Admire` (481) / `Glance` (486) | 1200 ms / `GLANCE_MS = 900` (158) | `decide` (3030, 3033-3039) | mischief / standing |
| `Swap`/`Giggle`/`Innocent` (490-507) | `FIDDLE_MS = 700` (374), giggle 1500 ms, kept `SWAP_KEPT_MS = 7–14 s` (376) | chain to each other, then SpaceOut 1.5–3 s | mischief |
| `Sneeze`/`PutBack` (508-519) | `WINDUP_MS 1400` + `RECOIL_MS 600` (378-379) | | mischief |
| `Clamber` (528) | cells × 333 + rows × 500 | `Stand` 800 ms (2770-2783) | moving |
| `Out` (534) → `Away` (541) | walk off screen; Away 4–12 s, or a work shift `SHIFT_MS = 60–180 s` (1043, 2794-2805), or `u64::MAX` out by her routine | `Away` → `Walk` back in (2818-2832) | **moving** (both) |
| `Door {since,to,gap}` (549) | 13 `DOOR` beats (876-957) = 5.5 s with no gap; her visible 1.6 s + 1.3 s; `gap` is a work shift's 60–180 s (7046-7055) | at the far side: a DOOR line, `decide` (2833-2874) | **moving** |
| `Home` (555) / `Poke` (560) | `HOME_MS = 3000` (1045) / `POKE_MS = 2000` (1053) | `decide` | standing |
| `Tear` (567) | `BRACE_MS 700`, `REEL_MS 220`/step (428-430) | `pursue(Job::Use(crumple seat))` | mischief |
| `Lift`/`SetDown` (595, 601) | `LIFT_MS`/`SET_DOWN_MS = 900` (387-388) | `Stand` `AFTER_CARRY_MS = 500` (2927-2971) | home |

**There's no homework anywhere but the desk, and no reading anywhere but the
bookshelf.** `Use::Homework` is "Homework at the desk" (room.rs:401-402),
posed `Pose::Homework(u8)` on a stool (sprite.rs, `Homework` variant).
`Use::Read` is "Sit beside the bookshelf reading" (room.rs:405-406), posed
`Pose::Read`. Floor homework, a makeshift desk and reading a pulled line are
all (proposed).

## 3. How a want becomes an act

- `choose_next` (osaka.rs:5385-5726) runs, in order: credit what she finished
  (5393), errand, no floor, returning home, a shift, off-text rest, a dash,
  the routine's clock glance first (5463-5474), bed or school, **watching chat
  (5492-5504)**, the greeting, needs pass (5517-5526), a calendar beat, the
  day-off line, owed beats (`Bucket::Owed`), **continuations**: the next hop of
  a heading (`heading/hop` / `heading/mine`, 5577-5605) and arranging
  (5613-5627) and `leftover` (5630-5636). Then the normal roll: every
  `Want::ALL` (brain.rs:636, 27 wants) bound by `mind::bind` (5639-5644), the
  heading re-offered ×`INERTIA = 3.0` (brain.rs:781; 5648-5660), scored by
  `brain::choose` (top `TOP = 4`, brain.rs:788), each attempt `plan`ned
  (5729-5796).
- `plan` (osaka.rs:5729-5796) maps a `Bind` to an act:
  `Here(Stand)` → Stand 2–5 s; `Here(SpaceOut)` → SpaceOut 6–14 s;
  `Here(Muse)` → `muse()`; `Here(Idle(a))` → `idle_act` (6382-6400; Sit/Lie
  facings random); `WalkTo(x)` → `Walk{then: Nothing}`; `Take(link)` →
  `travel`; `Door` → `through_door`; `Work` → `go_to_work`; `Job` → `go_to`.
- Methods per want: mind.rs:178-217. SpaceOut is `[space-out/muse,
  space-out]` (mind.rs:179): `muse` binds one time in three
  (`w.chance("muse", 0, 1, 3)`, mind.rs:222-225), else plain space-out.
  `Use(_)` is `[finish-my-heap, mine, real, made, make]` (mind.rs:191-197).

## 4. Walking: Walk vs Travel vs walking to a job

- **Walk (wandering).** `Want::Walk` base **14**, Restless 0.4 (brain.rs:691).
  Method `walk/along` (mind.rs:252-265): a column of her floor drawn
  **uniformly** (`pick` with the chat ×0.1, `w.below("walk", n)`), and never
  her own column. The mean walk is about a third of the floor's width; on a
  100-column floor that's ~33 cells ≈ 11 s at 333 ms/cell, as long as an
  average space-out.
- **Travel.** Base **10**, Restless 0.4, chat ×0.1 (brain.rs:692). Methods
  `travel/link` (a uniform link off her floor, mind.rs:268-276) then
  `travel/door` (with no links, a door to a random other floor, mind.rs:279-299).
  `travel()` (osaka.rs:6363-6374) is `Walk{to: link.x, then: Link(link)}`,
  then a Climb / Peer→Fall (a drop) / Clamber / Out→Away→Walk (osaka.rs:3160-3230).
- **Walk and Travel are credited as she sets off** ("Moving is the point of
  moving", osaka.rs:5703-5710): `serve(want, 1.0)` at once, so Restless drops
  by 0.4 when she sets off, not when she arrives.
- **Walking to a job.** `go_to` (osaka.rs:5885-5924): on her floor,
  `pursue` (6348-6360) = `Walk{to: spot.x, then: Job(job)}`; on another floor,
  `heading = Some(Heading{want, job})` and `travel(first link of the BFS
  route)` (`route`, osaka.rs:7344-7370), or a door straight there when there's
  no route. She lands and carries on as `heading/hop` without choosing anew
  (5577-5605). The census can't tell this apart from wandering: both are
  `Act::Walk` (`doing()` uses `act_name`, census.rs:205-219, osaka.rs:1882-1889).
  `act_summary` (osaka.rs:1893-1915) already tells them apart for the explain
  log (`Walk to (x,y) for Watch (Tv)` vs `Walk to x`): a ready-made source for
  step 1's split.
- **Which seat.** `mind::place` (mind.rs:434-441) picks one of `places()`
  **uniformly** (`w.below("place", len)`), never the nearest. The same goes
  for `walk`, `take_link`, `pull` and `swap`.
- **Moving needs** (brain.rs:67-80): Restless rises in **90 s**, Tidy in 60 s,
  Daydreams in 10 min, Comfort in 8 min. Mood rates (brain.rs:188-204): Lazy
  Restless ×0.4, Industrious ×1.5, Dreamy ×0.7 and Daydreams ×2.5.

## 5. Still acts end straight into a new decision (no settling)

- Idle end → `decide` (osaka.rs:2762-2769). Use end → `decide`
  (osaka.rs:2905-2922). SpaceOut/Stand end → `decide` (osaka.rs:3029-3031).
  Nothing chains a still act into another still act. The only fixed chains
  are mischief (Swap→Giggle→Innocent→Swap back→SpaceOut 1.5–3 s,
  3040-3101), Crumple→Admire, carry→Stand, and climb/fall→Stand.
- When `decide` runs, `self.act` **is still the finished act** (`choose_next`
  calls `set` only through `plan`/reflexes), so a settling-in step can read
  what she just did. `Bucket::Continuation` (osaka.rs:725-732) already exists
  for that kind of decision (heading/hop, leftover).
- **Repeats are damped**: `recent` keeps the last `RECENT = 3` choices
  (osaka.rs:410) and each repeat multiplies by `COOLDOWN = 0.4`
  (brain.rs:786, 815-816). Sit after Sit is 0.4×, so a settling chain that
  goes through the roll fights the cooldown. One that skips the roll (a
  continuation) doesn't.
- **Chat cuts every still act.** Each of them is `OnChat::Look` (osaka.rs:841-847).
  `look()` (osaka.rs:6432-6482) sets `watch_until = now + WATCH_MS` (15 s,
  371) and `interrupt(Cause::Chat)` → `Look` 1.2 s + 4 s (6508-6527, timings 6510), then
  Stand chunks while `at < watch_until` (5492-5504), then a fresh roll. There's
  no resuming. The only exception is answering a spliced use (6463-6468). The
  census rooms chat every 45 s (stage), 90 s (home) and 60 s (resident)
  (census.rs:54, 82, 107).
- **Text coming up under her** cuts Rest/Job acts too (`recheck`,
  osaka.rs:5155-5175, `Cause::Restless`).

## 6. What counts as "moving" (census)

`census_group` (osaka.rs:5249-5284): **moving** = `Walk | Climb | Clamber |
Out | Away | Door | Fall | Peer | Dazed`. Walking to a job, wandering and a
link's walk are all `Walk`. Notes for step 1:
- `Away` covers a work shift's 60–180 s off the screen edge, and `Door`
  covers a shift's gap (osaka.rs:7046-7055). The unfed `visit_from` counts every
  step of a visit with no hidden filter (census.rs:371-374), so **a work
  shift is counted as "moving" in `visit_census`/`sofa_census`-style unfed
  tables**. `day_census` counts groups only while `Where::Present | Dash`
  (census.rs:1176-1178), and `Where::Hidden` comes from `Osaka::hidden`
  (osaka.rs:6650-6659), so there only the door's visible beats (~2.9 s plus the
  shown-door beats) and Out's walk count.
- `Peer` (1.2 s at an edge, and before every drop), `Dazed` (1.5 s) and
  `Fall` are "moving" too.
- `doing()` (census.rs:205-219) names acts by `act_name` (`"Walk"`), so the
  purpose split needs `Then` exposed (proposed: e.g. an `Osaka::moving_for()
  -> Option<&'static str>` returning wander / link / job:<Use> / home / door,
  built from `Act::job()`, osaka.rs:822-831).
- census.rs:1445-1449 says outright: "Time spent moving about is no measure:
  in this room it's within a few percent either way" by mood.

## 7. Musings and thought bubbles while spacing out

- `muse()` (osaka.rs:2154-2251), in order: a cued Setsubun / ClockGlance /
  Escalator; her rare musing (the escalator) one in three when quiet on a day
  it's open (2185-2192); **the afternoon hour glance** (2196-2198); then a
  riddle or a musing: `RIDDLE` pool `n/d = 1/3` (mind.rs:858-863) when quiet,
  else seasonal `PANIC` 1/2 (mind.rs:819-824) or `DECEMBER` 1/3 (806-815), else
  `MUSINGS` `n/d = 1/1`, 8 lines (mind.rs:673-686). Then **one** SpaceOut of
  6–14 s (2243-2250).
- Pacing: **one line per space-out**, no run of musings. Each line cools for
  `LINE_COOLDOWN_MS = 10 min` (mind.rs:878, checked in `Lines::pick`
  1063-1083), so 8 musings at most every 10 minutes. A riddle's keys:
  `RIDDLE_ASKED_MS = 3000`, `RIDDLE_ANSWERED_MS = 5500` (script.rs:193-195).
  Spoken lines show for `speech_ms = 1200 + 60·chars` (osaka.rs:435-437).
  Musing lines aren't budgeted (only beat lines count toward `LINE_BUDGET = 8`,
  mind.rs:881).
- Bubbles: a plain SpaceOut shows `Bubble::Dots` "..." (osaka.rs:7236). Idle
  bubbles come from `Activity::look` (osaka.rs:1239-1249): LieBack `Zzz`,
  LieFront `Hum`, Gaze `Ooh`, Sit none.
- The muse share of SpaceOut: `SpaceOut` binds `muse` 1 in 3 (mind.rs:224), so
  2/3 of space-outs are silent "...".

## 8. Watch: TV from the floor already binds

- **Yes, Watch binds without a sofa.** The TV's own spec has
  `uses: &[Use::Watch]` (room.rs:184). `spots_for` (mod.rs:3437-3459) gives
  each non-inside use one seat at the first of `piece.beside()` (left, then
  right; room.rs:990-994) that passes `seat_spot`. `Seat::seat` puts her
  beside the TV, facing it (room.rs:941-984, the Watch arm at 961). `places(Watch)` offers it as a
  real seat, and `use_real` binds (mind.rs:443-448).
- **She sits on the floor for it**: `appearance` for `Act::Use` uses host
  `Pose::Lounge` only when `seat.item == Furniture::Sofa`, else `Pose::Sit`
  (osaka.rs:7189-7193). WATCH/SURF/SHOPPING keys are `Posed::Host`
  (script.rs:1360-1430). `Pose::Sit` is "Sitting on the floor hugging her
  knees" (sprite.rs:39-40; ASCII `SIT`, sprite.rs:126; line art "in profile,
  hugging her knees", art.rs:188-197). The test
  `surfing_flicks_through_the_channels_in_turn` pins Sit vs Lounge
  (osaka.rs:8712-8714).
- **With a sofa facing the TV, she may still watch from the floor.** The sofa
  adds a second Watch seat (mod.rs:3461-3496), and whenever both seats exist,
  `place` is uniform over them (mind.rs:439). Whether the TV's own seat
  survives beside a sofa depends on `seat_spot` alone (restful, on a floor).
  Watch has no `clear_of` check; only LookOut does (mod.rs:3453-3455). `a_sofa_facing_the_tv_is_watched_from`
  (tests.rs:8176) only filters for the sofa's seat (tests.rs:8282-8287) and
  doesn't exclude the TV's. `use_it` after a sofa turn prefers the sofa
  (mind.rs:466-482).
- So the brief's "TV from the floor" item is **art, not binding**: a
  cross-legged pose (proposed `Pose::CrossLegged` or a Watch-only host pose)
  for the TV-beside seat. The brief's "the cheapest win for a TV-only home"
  is already true of binding. Making it more frequent is a weight question
  (Watch base 10, Fun 0.6, ×2 Evening; brain.rs:703, 585-588).

## 9. The window: look-out (5b)

- Spec: `WINDOW` (room.rs:320-335), `uses: &[Use::LookOut]`, hung
  (`hang: Some(4)`). One seat, at the first of `look_out_spots()` clear of
  every shown piece (mod.rs:3446-3458, `clear_of` 3424-3434). She faces the
  window's middle (room.rs:969-976; spots `look_out_spots`, room.rs:1003).
- Want: `Use(LookOut)` base **8**, Daydreams **0.5** and Fun **0.3**,
  `LOOK_OUT_FACTORS` = chat ×0.1 and ×`BOOST = 2.0` over `DUSK_TO_DAWN`
  17:00–05:00 (brain.rs:737-740, 606-608; routine.rs:179). It's offered whenever
  the window seat exists (`use_real`). There's no rarity gate, no cooldown
  beyond `recent`, and Fun wears off with `fresh`.
- Duration: `use_duration(LookOut) = (15_000, 30_000)` (osaka.rs:1144), pinned
  by `looking_out_lasts_and_counts_as_spacing_out` (osaka.rs:12415-12428).
- Script: `ScriptId::LookOut` (host `Use`, script.rs:502). Branch from
  `look_out_branch` (osaka.rs:4606-4613), the sky by game minute: a line
  (`LOOK_OUT_LINES`, 11, script.rs:1704-1716) shows for `LOOK_OUT_LINE_MS =
  3000` (script.rs:1719), then `Pose::Gaze`, Face::Curious, silent to the end
  (`looking`, script.rs:1722-1733).
- Census: "spacing out" (osaka.rs:5252), but the walk there is "moving".
  The `day_census` row "look out" comes from `named()` (census.rs:243).
- **~19 a game day** (plan.md, 5b Measured): 19 × ~22.5 s ≈ 7 min of 4 real
  hours a game day, plus the walks. Daydreams refills in 10 min (×2.5
  dreamy), and a whole look-out serves 0.5 of it.
- D4's "Gaze ×more with a window" isn't built (plan.md 5b Open). `Gaze`
  `Idle` is base 6, Daydreams 0.5, ×2 dusk to dawn (brain.rs:690, 605), with
  no window term.

## 10. The clock glances (5b)

- `ClockOn` (osaka.rs:68-87): `seen_from` holds only on the **clock's own
  strip** (same floor, `from..to` columns).
- **Routine glance** (`ClockGlance::Bed`/`School`): `glance_first`
  (osaka.rs:3787-3798) as the first key of the Asleep or Away slot
  (5463-5474), once a slot (`pass_glance` keys on the slot's end,
  3803-3812), and only if she's on the clock's strip and not hidden.
- **Afternoon hour glance** (`ClockGlance::Hour(h)`): `hour_glance`
  (osaka.rs:2282-2296), reached **only from `muse()`** (2196-2198), so the chain
  is: SpaceOut wins the roll in the Afternoon slot (12:45–18:00, routine.rs:38-40)
  × `muse` binds (1/3, mind.rs:224) × not cued as a riddle × the escalator
  didn't take it × `HOUR_GLANCE = (1, 3)` (osaka.rs:425) × `!hour_glanced` ×
  quiet × on the clock's strip. `hour_glanced` is set at construction (false,
  osaka.rs:1762) and at `glance_up` (2259) and nowhere else, so it's
  **once per `Osaka` (visit), not per game day**.
- Both glances are `glance_at_clock` (osaka.rs:3817-3833): a SpaceOut of
  `CLOCK_GLANCE_MS = 2500`, `ScriptId::ClockGlance` branch
  (`ClockGlance::branch`, script.rs:1808-1815), credit cleared ("a glance
  eases nothing").
- **Why look-outs crowd it out:** the glance can only happen inside a
  SpaceOut. SpaceOut's fit is Daydreams-only (base 6, 0.6, brain.rs:671).
  LookOut has the higher base (8), answers Daydreams **and** Fun, and drains
  Daydreams by 0.5 a use. So in a home with a window, SpaceOut is rarely in
  the top 4. The next bullet makes it worse.
- **Bug (code vs design.md):** **spacing out never eases daydreams.**
  `credit_done` (osaka.rs:4945-5021) only has arms for `Idle`, `Use` and
  `Pull`; `(Act::SpaceOut, Want::SpaceOut)` falls to `_ => return`
  (osaka.rs:5003) before `credit = None`, and the next `choose_next`
  overwrites `credit`. The only other callers of `serve` are Walk/Travel at
  set-off (5703-5710) and `credit_whole` for Swap/Pull (3057, 4934).
  design.md:1459-1460 says "*daydreams* rise slowly and spacing out, gazing,
  lying on her front and drifting off at her desk answer them". `Want::Work`
  (Restless 0.5, brain.rs:697-700) has the same gap: `go_to_work` serves
  nothing. No test asserts SpaceOut is credited (`credited` is read only in
  osaka.rs:8069, 8263, 10279, 11904; tests.rs:11282).
  - Effect: in a bare room Daydreams stays high, so SpaceOut stays
    attractive but only `recent` throttles it. In a window home, LookOut
    alone drains it.

## 11. Stage (stage.rs) and scenes (scenes.rs)

- `Scene` (stage.rs:28-136, 48 scenes) with wildcard-free `name` (192-245),
  `furniture` (247-262), `cue` (270-331) and `activity` (333-345). A furniture
  scene picks a random seat of its use (stage.rs:489-507), so `Scene::Watch`
  in a sofa room lands on the TV's floor seat or the sofa's at random.
  `Scene::Muse`/`Gaze`/`Sit` etc. go to `osaka.idle`/`osaka.muse` on the spot
  (stage.rs:627-645). Placement: `APPROACH = 6` cells off (stage.rs:379,
  654-668), "so the walk there shows".
- `scenes.rs` holds the jobs: `Job::{Pull, Swap, Build, Use(Seat), Lift,
  SetDown}` (scenes.rs:122-133), with `spot`, `side` and `carry`
  (scenes.rs:158-186). `Build` is the torn-text makeshift path (scenes.rs:84-118,
  `builds` 648). Makeshift items today are a sofa and a bed (scrap.rs:1-2), with
  `MAKESHIFT_ODDS = 20` and `FACING_TV = 5.0` (mind.rs:24, 28). `places()`
  (mind.rs:381-431) offers `Place::Make` as one uniform place while she owns no
  real piece of that kind.
- `ScriptId::scene()` (script.rs, "The stage scene that cues it.
  Wildcard-free", after 541) maps every script to a scene.

## Levers and risks for 5c

**Step 1 (measure).**
- Plug: a purpose for `Act::Walk` from `then` (`Then::Nothing` = wander,
  `Then::Link` = travel, `Then::Job(Use(seat))` = to a seat by `seat.what`,
  Pull/Swap/Build = to text, Lift/SetDown = home), plus Climb / Clamber / Fall
  / Peer / Dazed / Door / Out, all from one exhaustive match beside
  `census_group` (osaka.rs:5249) (proposed). One catch: a heading's hop is
  `Then::Link` while `self.heading` is `Some`, so walking to a job on another
  floor must read `heading` too (osaka.rs:5904-5907).
- Risk: exclude `Away` and hidden `Door` from "moving" in the unfed census, or
  the work shift inflates it (§6). Decide whether a door's visible beats are
  movement.

**Step 2 (band test).** The census comment says moving share is
mood-insensitive (census.rs:1445-1449), while the band asks for 15 lazy vs 30
industrious. The mood rates only scale Restless (×0.4 to ×1.5, brain.rs:192,
196, 202). A band per mood probably needs mood-dependent bases or still-act
lengths. Open question: does Lazy's band include walking to her sofa?

**Step 3 levers, by plug point:**
- *Slower needs / lower Walk base*: brain.rs:69 (`Restless 90_000`), 71
  (`Tidy 60_000`), 691 (Walk 14). This moves every golden hash
  (tests/golden.rs:562-700, `(seed, graphics, fed)` tuples) and the unfed
  census tables (`sofa_census` tests.rs:8012, `visit_census` census.rs:662).
  `her_mood_shows` (census.rs:1450) may flip.
- *Longer still acts*: `SPACE_OUT_MS` (osaka.rs:419), `Activity::duration`
  (1216-1226), `use_duration` (1130-1145). Lengthening is safe for the script
  lints: `shortest_body` uses `SPACE_OUT_MS.0` and `shortest_use_ms`
  (script.rs:507-541), `BODIES[0] == shortest_use_ms` (script.rs:2098). Surf
  needs 4×`SURF_MS` = 11.2 s inside Watch's minimum (script.rs:1369), and
  LOOK_OUT_LINE_MS needs 3 s inside the shortest use. **Shortening** a minimum
  breaks those. The 6000 in `SPACE_OUT_MS.0` is also read by census.rs:1607,
  osaka.rs:4498 and the riddle test (7446). LieBack's range also sets her night
  on the floor by day (osaka.rs:3685) and Stretch's minimum her wake
  (3979).
- *Settling in*: plug in `choose_next` after `leftover` (osaka.rs:5630-5636)
  as a `Bucket::Continuation` "settle" that reads the still-finished
  `self.act` (Idle Sit → LieBack → doze; SpaceOut → Sit; Use Watch/Lounge →
  Nap) (proposed). It bypasses the roll and `recent`'s 0.4 cooldown. It needs
  its own credit (`self.credit = Some(want)`), or the §10 crediting gap
  repeats.
- *Chat resumption*: longer acts barely move the stage (45 s chat) or the
  resident room (60 s) unless a cut still act is resumed after the watch.
  Plug: remember the cut act in `interrupt` (6508) and offer it as a
  continuation after the "watching chat" reflex (5492-5504) (proposed). Risk:
  `Stays::Rest` resumption has to re-check `terrain.restful`.
- *Nearest spot*: `mind::place` (mind.rs:434-441) and `walk` (252-265) are
  uniform. A distance weight there changes `pick_weighted`-style rolls, which
  moves goldens. The tests that pin random seat choice need a look:
  `a_made_sofa_mostly_faces_the_tv` (tests.rs:8385).
- *Daydream sessions*: today a SpaceOut is one line and 6–14 s
  (osaka.rs:2243-2250). A session needs a play with several musing keys
  (`Play` and keys, like `Play::riddle`) or a re-muse at each key end in
  `fire`'s SpaceOut arm (3029) (proposed). Mind the 10-min line cooldown and
  the 8 MUSINGS lines: a session of 3 eats them in ~3 sessions.
  `every_pooled_line_shown_was_drawn_from_its_pool` (census.rs:1490) holds
  every shown line to its pool.
- *TV from the floor*: binding exists (§8). New art means a pose for the
  TV-beside seat: change the host-pose rule at osaka.rs:7189-7193 (e.g.
  `seat.item == Tv` → cross-legged) (proposed), plus sprite/art and the surf
  test (osaka.rs:8714). Fix first: SpaceOut not easing Daydreams, and Work
  not easing Restless (§10). Fixing them reshapes every table, so it should
  land before step 1's measurement, or the baseline is wrong.

**Step 4 (new acts):** a new Use or Activity touches `Want::ALL: [Want; 27]`
(brain.rs:636), `Use::ALL: [Self; 11]` (room.rs:418), `Activity::ALL: [_; 7]`
(osaka.rs:1195), `Act::props` (833), `census_group` (5249), `credit_done`
(4945), `use_duration` (1130), `Shown::seat` (room.rs:941-984), `ANY_USE` and
its exhaustive test (rules.rs:50-60, 1152-1178), `ScriptId::{host,
shortest_body, scene}` (script.rs:478-541), `Scene` and its four matches
(stage.rs), and census `named()` (census.rs:231). A makeshift desk extends
scrap/`builds` (scenes.rs:648) and `places()`'s `Place::Make` weight
(mind.rs:439, uniform today, where the "×3 make one" lever goes).

**Step 5 (window as a daydream):**
- Rarer: lower LookOut's base (brain.rs:737-740) and/or add a per-visit or
  per-game-day cap (proposed). Longer: `use_duration(LookOut)` (osaka.rs:1144)
  and the test pinning `(15_000, 30_000)` (osaka.rs:12416). Possible keys:
  several sky lines over its span (`looking`, script.rs:1722) or a muse inside
  it (proposed).
- Afternoon glance: it can't come back while it's reachable only through
  SpaceOut→muse (osaka.rs:2196). Options: let a long look-out or Gaze carry
  the hour glance too, key `hour_glanced` per game day like `clock_glanced`
  (proposed), and fix SpaceOut's credit so SpaceOut and LookOut share
  Daydreams fairly. Tests: `of_an_afternoon_she_glances_at_the_hour`
  (osaka.rs:12174), `a_cued_riddle_wins_over_a_rolled_glance` (12357).

**Open questions.**
1. Is the SpaceOut credit gap a bug to fix in 5c step 0 (design.md says it
   answers daydreams), or intended? Fixing it moves every table.
2. Should a work shift off-screen count as moving (unfed census) at all?
3. Should a resumed still act after a chat line count as "settling"?
4. Cross-legged: a new `Pose` or a variant of `Sit` for the TV only?
5. Gaze ×more with a window (D4): still wanted once look-outs are rare?


---

<!-- section: census -->
# Code map for 5c: tests/census.rs (the censuses, and what "moving" means)

HEAD cabf5d1. Paths relative to `dessplay/src/ui/houseguest/`. All refs checked
against the code. Numbers marked **(run)** come from this mapping session's own
runs of the already-built test binaries, built 2026-10-05 at 18:14 (debug) and
18:20 (release), after HEAD's 18:13 commit:
`CENSUS_MOODS=1 … visit_census --ignored` (release, 85 s) and `day_census --ignored`
(release, 61 s). The logs are in the scratchpad: `visit_census_moods.log`, `day_census.log`.

## 0. Three corrections to the brief (read first)

1. **The visit census's "moving" counts time she is out of sight.** `census_group`
   (osaka.rs:5249) puts `Act::Away`, `Act::Out` and `Act::Door` in `"moving"`
   (osaka.rs:5260-5268). `visit_from` records every step spent in `State::Visiting`
   (census.rs:371-374), with no `hidden()` or `sleeping()` filter. `Act::Away` is
   hidden (osaka.rs:6650-6652), and so is the middle of a door (osaka.rs:6653-6655).
   **(run)** `Away` alone is 2.4–3.0% of visit time on the stage, 6.8–8.8% in the home
   and 6.1–8.2% in the resident's room. In the home and the resident's room it is
   her work shift (`SHIFT_MS = (60_000, 180_000)`, osaka.rs:1043, chosen at
   osaka.rs:2801). On the stage it is a `Route::Around` hop under Travel, off screen
   for `rng.range(4000, 12_000)` (osaka.rs:2803). The stage's `Door` takes 8.4–12.3%
   of visit time, and at least 2600 of each door's 5500 ms is hidden (DOOR table,
   osaka.rs:876; see §5). The day census leaves this out: `Where::of`
   (census.rs:1011-1021) sends hidden time to `Where::Hidden`, and `groups` are
   recorded only for `Present | Dash` (census.rs:1176-1178). **So the 5a/5b visit-census
   "moving" figures (plan.md:2808-2812, and 38/42/38 for ordinary today) are not the
   band's quantity.** The band has to be in-sight moving ÷ in-sight awake time, and
   nobody has measured that unfed yet. My own rough estimate, not a measurement: about
   33% for the stage ordinary ((38.1 − 2.4 − ~4.7) / (100 − 2.4 − ~4.7)) and about 38%
   for the home ordinary ((42.4 − 6.8) / 93.2). Step 1 has to produce the real numbers
   before step 2's thresholds are set.
2. **"35–45% of awake time in every slot" is not what the day census shows.**
   **(run)** Week totals for moving as a share of time here and awake: stage 35,
   home 40, resident 37. The home's Homework slot is **28–29** (furniture 49–50).
   The Asleep and Away slots show **59–81** moving, but on only 0.2–0.5% of their
   time awake and here (walking to bed, to the door, or a dash). A band test per slot
   would fail by construction. The band belongs to a visit (unfed) or to the awake
   time of a week (fed).
3. **The day census has no mood dimension.** `Stretch` is keyed by hour and slot only
   (census.rs:1026-1035, `Week` at 1088-1099). The brief's "every slot, room and mood"
   holds for rooms and slots in the day census. Moods were only ever measured in the
   unfed visit census with `CENSUS_MOODS` (census.rs:664-669).

## 1. The file at a glance

`tests/census.rs` (1700 lines) is `mod census;` of `tests.rs` (tests.rs:11306). It runs
under `use super::*` (census.rs:16), so it shares tests.rs's helpers: `paint`
(tests.rs:86), `real_frame` (744), `home_screen` (1969), `resident_view` (6024),
`kitty` (291), `stage_ui`/`DELAY` from `stage` (tests.rs:18). No other test module
uses its items (grep for `census::` finds nothing outside the file).

| Item | Line | What it is |
|---|---|---|
| `SEEDS: u64 = 16` | 24 | seeds per room in the visit and image censuses |
| `struct Room` | 28-43 | name, `real: Buffer`, `view: IdleView`, `owns: &'static [Furniture]`, `chat_every: Option<u64>`, `live: Option<fn(u64)->(Buffer, IdleView)>`, `start: Option<GameTime>` (`None` = unfed) |
| `stage_room()` | 46-58 | stage UI at 100×30, owns nothing, chat every 45 s |
| `furnished_room()` | 67-86 | `home_screen()`, owns Sofa, Tv, Bed, Desk, Bookshelf, Fridge, Lamp; chat every 90 s; the pieces land breaking rules |
| `resident_room()` | 89-111 | `rooms(100,30)` with 10 text lines scattered, owns Sofa, Tv; chat every 60 s |
| `live_room()`/`live_frame` | 118-145 | 200×50 chat that scrolls every 45 s (image census only) |
| `struct Visit` | 148-202 | per-visit tallies (§2) |
| `doing()` | 205-219 | the census key for an act (§3) |
| `simulate()` | 223-225 | `simulate_with(room, seed, minutes, mood, false, no-op)` |
| `named()` | 231-281 | vignette names from a `Play` (§3) |
| `arrive_in`/`arrive_drawn` | 286-321 | unfed arrival, forced mood, pieces given (§6) |
| `simulate_with` | 325-340 | arrive, then `visit_from` |
| `visit_from` | 345-479 | the step loop (§2) |
| `home_census` | 483-506 | felt and mended rules, purchase |
| `group()` | 510-512 | looks up `groups[doing]`, else `"unrecorded"` |
| `pct()` | 515-517 | `100*part/whole.max(1)` |
| `vignette_summary`/`home_summary` | 522-551 / 556-658 | printers |
| `visit_census` | 660-773 | `#[ignore]` |
| `image_census` | 788-888 | `#[ignore]` |
| `MONDAY`, `WEEK_MS`, `DAY_HOME`, `day_rooms`, `day_guest` | 894-969 | day census setup |
| `enum Where` | 973-1022 | Present, Dash, Asleep, Hidden, Away, Gone |
| `Stretch`, `Week`, `phase`, `moved`, `when` | 1026-1141 | day census tallies |
| `live_week` | 1147-1278 | a fed game week |
| `day_census` | 1296-1372 | `#[ignore]` |
| gate tests | 1377-1700 | §7 |

`sofa_census` is **not in census.rs**. It is in tests.rs:8012 (§4.1).

## 2. The visit loop (`visit_from`, census.rs:345-479)

- Setup: `guest.shop()` (356), so the shopping channel is on at her first watch.
  `broken_at_start` is taken from `visit.broken` (358-360).
- Step: `next_tick(now)` clamped to `1..=1000` ms (366-369). Each step is credited
  **before** advancing, to `doing(&visit.osaka, now)`, and
  `out.groups[doing] = osaka.census_group()` is recorded (372-374). Speech changes
  are counted in `said` (375-384). New plays are named through `named()` (385-402),
  with `lamp` meaning a Lamp is shown and not boxed (389-392). Answered chat goes
  in `answers` (403-407).
- Every 5 min, `needs().summary()` is pushed (409-413).
- Chat: when `now / every` changes, `chat_mark.synced += 1`. Every other line asks
  her something (`synced_asks`, 429-430). A `live` room swaps the frame (420-427).
- `advance(now)`, then `paint` if anything changed (432-434), then `home_census`
  (435-437).
- End (439-477): mood, home_acts, set_downs, retried, dropped (`Loss::Moved` beats),
  broken (with `*` for felt), beauty, nesting, repairs, `choices` (from
  `osaka.decisions[..].want`, 462-467), headings, beats, pooled lines.
- **Not filtered**: hidden steps (Away, the door gap) and the unfed "Sleep" use
  (furniture) are all in `time`. The only cut is `State::Visiting`, so
  `Leaving`/`Absent` are skipped.

## 3. The classification helpers

**`doing(osaka, now) -> String`** (205-219):
- in a use (`osaka.use_span()`, osaka.rs:5177): `"use:{What:?}:{made|real}"`, made
  being `seat.makeshift()`;
- `act_name() == "Idle"`: `"idle:{Pose}"`, the appearance pose's `Debug` cut at `(`
  (giving "idle:Jack", "idle:LieFront", "idle:Gaze" and so on);
- otherwise the bare `act_name()` (osaka.rs:1882): `Debug` of `Act` cut at the first
  space, `{` or `(`. That gives "Walk", "Climb", "Clamber", "Fall", "Peer", "Dazed",
  "Door", "Out", "Away", "Stand", "Look", "SpaceOut", "Pull" and so on.
  **`act_name` is hashed by the goldens** (its doc comment, osaka.rs:1881): leave it alone.
- Consequence: **`"Walk"` lumps together every purpose of walking**: wandering
  (`Bind::WalkTo`, osaka.rs:5756), a Travel hop to its link (`travel`, 6363), a hop
  toward a job on another floor (`go_to`, 5885), a walk to a job on her own floor
  (`pursue`, 6348), the walk-in at arrival (1805, 1832), the walk back in after
  `Away` (2828), off the text (`find_rest`, 6832, `Then::Nothing` at 6846), the
  errand walk (6880), and leaving for work by a `Route::Around` link (`go_to_work`,
  7039).

**`Osaka::census_group()`** (osaka.rs:5249-5290, `#[cfg(test)]`, wildcard-free, so a
new act won't compile until it has a group):
- `"moving"`: Walk, Climb, Clamber, Out, Away, Door, Fall, Peer, Dazed (5260-5268);
- `"furniture"`: any Use except LookOut, which is `"spacing out"` (5252-5253);
- `"floor rest"`: Idle Sit, LieBack or LieFront; `"spacing out"`: Idle Gaze and
  SpaceOut; `"exercise"`: Jacks, ToeTouch, Stretch;
- `"mischief"`: Pull, Swap, Giggle, Innocent, Tear, Sneeze, PutBack, Admire;
  `"home"`: Lift, SetDown;
- `"standing"`: Stand, Look, Glance, Home, Poke.
- Note that a Pull (the heave) is mischief, not moving, while walking to the pull
  is moving.

**`group(groups, doing)`** (510-512): the group recorded for that key, else
`"unrecorded"`. This only works because each `doing` string maps to one group.
A purpose split must keep that true, or record groups separately.

**`named(play, lamp) -> Vec<(bool, String)>`** (231-281): a script's own name for
Shopping, Surf, Riddle, Night, DashLunch, DashForgot, Setsubun, FirstSunrise, Dream,
Escalator, LookOut, ClockGlance, and "bedtime" (`Sleep` with branch 0 and a lamp);
`None` for the rest (wildcard-free list at 246-259). Splices come out as
`(true, name)`: chopsticks clean/bad, `sata andagi ×{n}` from `ANDAGI_COUNTS`,
no melon, scary story, and the Test* splices.

**`phase(guest)`** (1103-1111) and **`moved(was, now, out) -> Option<&'static str>`**
(1114-1131) cover the day census's comings and goings: "arrive" (`arriving Idle…`),
"return" (`arriving Return…`), "dash in" (`visiting Dash`), "goodbye" (`leaving`),
and "out to school" (from `visiting Normal` to a non-visiting state with
`guest.out.is_some()`). `live_week` checks both after the tick and after the paint
(1195-1206), because an arrival lasts only until the paint.

## 4. The four censuses

### 4.1 `sofa_census` (tests.rs:8012-8099, `#[ignore]`)
- 100 seeds (`SEEDS = 100`, 8015) × 5-minute visits (`now < 300_000`, 8034) in the
  stage room (`stage_ui`, 100×30), unfed (8024), `guest.cue(Scene::MakeSofa)` (8028).
- Run with chat `None` and `Some(37_000)`, the chat phase shifted by `seed * 373`
  (8042), and in ASCII and line art (8017-8018).
- Measures what became of each piece she made: used (with median/p90/max seconds
  from made to used), let be (`gave_up`), lost, or still waiting (8072-8095).
  Prints only, no assertions.
- **Nothing in it is about movement.** "5a-comparable" in the brief means the visit
  census.

### 4.2 `visit_census` (census.rs:660-773, `#[ignore]`)
- `MINUTES = 30`, `SEEDS = 16`, rooms stage, home and resident (670), unfed.
  Mood drawn per visit, or forced to each of `Mood::ALL` when `CENSUS_MOODS` is set
  (665-669).
- Prints per room×mood: choices (count and %), **groups as % of all visit time**
  (742-749), time by `doing` (≥0.5%, 750-759), dominant-want share and longest run,
  headings, beats, said, vignettes, the home summary, and seed 0's needs every 5 min.
- **(run)** Forced moods; groups "moving" as % of all visit time (hidden time included):

| Room | Ordinary | Lazy | Industrious | Dreamy | of which `Away` (hidden) |
|---|---|---|---|---|---|
| stage | 38.1 | 34.1 | 39.5 | 36.4 | 2.4 / 2.9 / 2.4 / 3.0 |
| home | 42.4 | 40.4 | 47.3 | 40.1 | 6.8 / 7.7 / 8.8 / 7.0 |
| resident | 37.9 | 37.3 | 40.4 | 33.2 | 8.2 / 8.0 / 7.6 / 6.1 |

  Breakdown of moving by `doing` (ordinary): stage Walk 17.7, Door 10.1, Clamber 4.1,
  Climb 2.5, Away 2.4, Out 0.9; home Walk 33.3, Away 6.8, Climb 1.5, Out 0.6;
  resident Walk 23.7, Away 8.2, Climb 4.8, Out 0.8. Choices (ordinary home):
  Walk 15.9%, Travel 11.2%, Work 1.9%; **the Walk act takes 33% of time while
  Walk+Travel are 27% of choices**, so a large share of walking is to jobs (Uses).
  These match plan.md:2808-2812 to within about 1 point; 5b said within 0.2–3 points.

### 4.3 `image_census` (census.rs:788-888, `#[ignore]`)
- Rooms stage, home, resident, live; each of `Mood::ALL` forced; 16 seeds in threads
  (`std::thread::scope`, 837-844); line art at 10×20 px cells, cache limit
  `usize::MAX`; samples at `MARKS = [20, 40, 60, 120]` minutes.
- Measures distinct images, working set, bytes, and re-encodes per `LIMITS`
  (256/512/1024/2048). The one assertion is `counts.evicted == 0` (808).
  **Movement-relevant only indirectly**: fewer walk frames mean fewer images, so 5c
  may shrink its numbers.

### 4.4 `day_census` (census.rs:1296-1372, `#[ignore]`, 61 s in release **(run)**:
stage 29 s, home 18 s, resident 13 s)
- `day_rooms()` (917-933): stage; home with `DAY_HOME` (9 pieces, adding Clock and
  Window, 902-912); resident. Each with `start: Some(MONDAY)`, where
  `MONDAY = GameTime { day: 7, h: 0, m: 0 }` (894).
- `DAY_SEEDS = [0, 1, 2]` × dates `[2026-10-31, None]` = 6 runs per room, one thread
  each (1304-1318). Each run lasts `WEEK_MS = 7*24*60*60_000 / CLOCK_SPEED`
  (897; `CLOCK_SPEED = 6`, mod.rs:795), i.e. 28 real hours.
- `day_guest` (940-969) first runs an unfed `arrive_in` to place the pieces, asserting
  each is shown (955-962). It then builds `Ledger::new_at(seed, start)` with that
  home, `clock_sent` if she owns a Clock, and `Guest::restore`; she starts absent.
- `live_week` (1147-1278): `view.delay = Some(DELAY)`; each step is credited to
  `Where::of` and to `osaka.census_group()` **only for Present | Dash in
  `State::Visiting`** (1176-1178), bucketed by `[school day|day off][hour]` and by
  slot name. Events: phases (`moved`), plays (clock glances by branch 0/1/2:
  bed/school/hour, 1217-1222), rares drawn, first seen, calendar, wall clock delivered.
- Prints: the week line, then per-hour and per-slot lines (`Stretch::line`,
  1048-1072): **the top 4 groups as integer % of awake-here time**, events summed over
  runs, and each run's rares and one-off events.
- How "35–45%" was read: off the slot lines' `moving NN` (top-4 groups, % of
  `self.groups.values().sum()`, 1053-1059). See correction 2 for what they actually
  say.
- No mood is recorded. Moods in a fed week come from `Mood::of(day_seed(master, day))`
  at `begin_day` (osaka.rs:4002) and at each arrival (mod.rs:2347-2356). With 3 seeds
  × 7 days per room, expect roughly 10 ordinary, 4 lazy, 4 industrious and 2 dreamy
  days per date.

## 5. Moving acts, their purposes, and how long they last

Where each moving act is set, and what it can be for:

| Act | Set at | Possible purposes |
|---|---|---|
| `Walk{then: Nothing}` | `Bind::WalkTo` (osaka.rs:5756, from `walk`, mind.rs:253: **a uniformly random column of her floor**); arrival walk-in (1805/1832); back from Away (2828); off text (6846); errand (6880) | wander (Walk), arrival, return, off-text reflex, errand |
| `Walk{then: Link}` | `travel()` (6363) from `Bind::Take` (5763), `go_to` hops (5906-5911), `go_to_work` (7044) | wander (Travel), hop to a job, work |
| `Walk{then: Job}` | `pursue()` (6348) from `go_to` (5885, same floor) | to a job: Pull/Swap/Build (text), Use (seat), Lift/SetDown (home) |
| `Climb`/`Clamber`/`Peer`→`Fall` | the end of a `Then::Link` walk (3168-3245) | Travel or a hop to a job |
| `Fall` (floor gone) / `Dazed` | 6586 / 6605 (no decision); `Dazed` after a fall of ≥3 rows (3337-3343) | accident |
| `Out`→`Away`→`Walk` | `Route::Around` (3192-3212; Away 4–12 s, or the shift, 2793-2804) | Travel, work, routine out |
| `Door` | `through_door` (6670): Travel's `door_away` (mind.rs:278), `go_to` with no route (5912-5921), `find_rest`, errand, work with no edge (7045-7055), school | all of the above |

Details:
- `Heading { want, job }` (mind.rs:1149-1152) is set only for jobs on another floor
  (osaka.rs:5906, 5917), so a same-floor job's walk has `Then::Job` instead.
- Speeds: `WALK_MS = 333` per cell (osaka.rs:358), `CLIMB_MS = 500` per row (360),
  `PEER_MS = 1200` (366), `DAZED_MS = 1500` (364).
- A door is 13 beats (osaka.rs:876-970). She is visible for 600+300+700+600+300+400
  = 2900 ms and hidden for 400+250+350+max(600, gap)+400+250+350 ≥ 2600 ms.
- After a climb or a short fall she stands 800/600 ms (3324, 3345). That counts as
  "standing", not moving.
- Why she moves (brain.rs): `Walk` row(14.0, Restless 0.4) (691);
  `Travel` in_chat(row(10.0, Restless 0.4)) (692); `Pull` 16.0 (693); `Stand` 4.0
  (670); `SpaceOut` 6.0 (671). `rise_ms`: Restless 90 000, Tidy 60 000 (67-71).
  Mood rates: Lazy Restless 0.4, Industrious 1.5, Dreamy 0.7 (brain.rs:189-205).
  Walk and Travel are credited as she sets off (osaka.rs:5705-5707).
- Still acts are short. `Activity::duration` (osaka.rs:1216-1226): Sit 10–25 s,
  LieBack 15–40 s, Gaze 4–10 s. `SPACE_OUT_MS = (6000, 14_000)` (419).
  `use_duration` (1130-1144): Lounge 15–30 s, Watch 20–45 s, LookOut 15–30 s,
  Nap 30–60 s.

## 6. Can a mood be forced?

- **Unfed: yes.** `arrive_drawn` (census.rs:295-321) runs `Guest::new(seed).unfed()`,
  `cue(Scene::Arrive)`, one `paint`, then `visit.osaka.set_mood(mood)`
  (313-315; `set_mood`, osaka.rs:6328). **(run)** It holds: in the home, "home acts
  a visit by mood" is Lazy 0.00 and Industrious 2.62, matching `Mood::home_acts`
  (brain.rs:209-214). Caveats: it is an `if let`, so if she isn't `Visiting` after
  one paint it **silently does nothing** (worth an `assert!`); her greeting is the
  drawn mood's; and the mood's need rates apply only from the forcing on (her
  arrival needs are not mood-dependent, so there is no other effect).
- **Fed: no.** `begin_day` sets `self.mood = Mood::of(day_seed(master, day))`
  (osaka.rs:4002-4004), as does each arrival (mod.rs:2347-2356). A `set_mood` after
  arrival lasts only until the next wake or catch-up. There are two ways to get mood
  into the day census:
  (a) bucket each step's `Stretch` by `osaka.mood()` (osaka.rs:6322). This needs no
  product change. `Mood` derives only `Clone, Copy, Debug, PartialEq, Eq`
  (brain.rs:158), so key by `format!("{m:?}")` as `home_summary` does (census.rs:560).
  It is thin on dreamy days.
  (b) a (proposed) `#[cfg(test)]` mood override on `Ledger`/`Osaka`, read in
  `begin_day` and at arrival.
- The golden helper `home_at` (golden.rs:353) gives a fed home at any `GameTime`. A
  fed band test could arrive there in an afternoon and force the mood after the
  first paint; within a visit no `begin_day` runs unless it crosses a wake.

## 7. What is pinned and what is only printed

The censuses' tables are **printed only**. The `#[test]`s in census.rs that the gate
runs, with timings **(run)** for the debug binary run alone:

| Test | Line | Pins | Time |
|---|---|---|---|
| `an_uninterrupted_trip_runs_its_course` | 1377 | home without chat, seeds 0..6 × 10 min: no "let go: Other", and **`set_off >= 8`** trips | 0.94 s |
| `the_census_home_shows_every_piece` | 1401 | each owned piece shown, 16 seeds × 2 modes | — |
| `at_home_her_furniture_beats_the_floor` | 1434 | home Ordinary, 4 seeds × 10 min: furniture > 10 × floor rest | 0.67 s |
| `her_mood_shows` | 1450 | resident, 4 seeds × 15 min per mood: lazy furniture > industrious; lazy exercise < industrious; dreamy spacing out > ordinary. Its doc says moving "is no measure … within a few percent either way, seed set to seed set" (1447-1449) | 3.57 s |
| `every_pooled_line_shown_was_drawn_from_its_pool` | 1486 | 3 rooms × 2 seeds × 12 min × 2 modes, plus 12 cued muses | — |
| `the_census_counts_her_vignettes` | 1636 | home without chat, 3 seeds × 15 min × 2 modes: shopping 1, andagi and chopsticks counts agree with speech, riddle pooled 2× | 2.40 s |

Elsewhere: `a_restless_osaka_mostly_moves` (brain.rs:1034-1042: Walk+Travel more than
half of 2000 rolls at Restless 1.0), `repeating_herself_is_discouraged`
(brain.rs:1063-1069, a Walk repeated scores <0.2× a fresh one), `census_group` of
LookOut (osaka.rs:12427).

Cost model: `her_mood_shows` is 16 visits × 15 min = 240 resident sim-minutes in
3.57 s, **about 15 ms per sim-minute** (debug opt-2). The home is about 17 ms
(40 min in 0.67 s). The stage is roughly twice the resident's: the release day census
took 29 s for the stage against 13 s for the resident. The gate's long pole is
`every_made_piece_is_used_or_let_go`, about 13 s at 32 cases (.config/nextest.toml
comment, and the memory). Plain `#[test]`s don't scale with `PROPTEST_CASES`.

## 8. Design: splitting "moving" by purpose (step 1)

**Where the code goes.** `Act`, `Then` (osaka.rs:440, 633) and the fields `heading`,
`shift`, `leaving`, `returning`, `errand`, `dash` (1321, 1342-1366, 1544) are private
to `osaka.rs`. The classifier must therefore be a `#[cfg(test)] pub fn` on `Osaka`
beside `census_group` (proposed name `census_motion(&self, now) -> Option<(Purpose,
Motion)>`). census.rs can't see those types.

**Two axes** (proposed), so that "climbs, falls and doors" is a column, not a purpose:
- *Motion*: `walk` (Walk, Out), `climb` (Climb, Clamber), `fall` (Peer, Fall, Dazed),
  `door` (visible beats), plus `hidden` (Away, and the door's hidden beats via
  `hidden(now)`). `hidden` is reported but left out of the band.
- *Purpose*, in priority order:
  1. `heading.is_some()`: to a job, by `heading.job` kind;
  2. `Walk{then: Then::Job(job)}`: to a job by kind. Job kinds (scenes.rs:122):
     **text** = Pull, Swap, Build; **seat** = Use; **home** = Lift, SetDown;
  3. `shift.is_some()`: work; `leaving` or `returning`: routine (school);
     `dash`: dash; `errand`: errand;
  4. otherwise by the chain tag (below): `walk/along` is wander:walk;
     `travel/link` or `travel/door` is wander:travel; `off text` is off-text;
     arrival or back from Away is arrival/return; no decision is an accident.
- Ambiguity to decide: `use/make` binds `Job::Build` (tearing text for a made sofa).
  By job kind that is "to text"; by want it is a Use. I suggest job kind (it's the
  walk's goal), with the want shown in a second table.

**The chain-tag gap.** Acts chained without a decision (a Climb after a Travel hop,
the floor-gone Fall at 6586, Dazed at 6605, the arrival walk-in at 1805/1832, the
walk back from Away at 2828) leave `decisions.last()` (osaka.rs:5377 pushes it) stale.
A Fall under a Use would then read as "use/real". Proposed: a `#[cfg(test)]
chain: &'static str` set in `decide()` (osaka.rs:5367) from `Decision.method`, and
overwritten at the non-decision `set` sites (arrival, floor gone, back from Away).
This changes no behaviour and moves no goldens.

**census.rs changes** (proposed):
- `Visit` gains `motion: BTreeMap<(String, String), u64>` (purpose, motion) and
  `in_sight_ms`/`hidden_ms`. In `visit_from`, after line 374, add the step to
  `motion` when the group is moving, and count hidden or sleeping steps apart.
- `visit_census` prints one more row per room×mood: moving as % of **in-sight**
  time, split by purpose and by motion. It also prints the **per-seed min/max (or
  sd)** of the in-sight moving share. Today only 16-seed means are printed, so the
  band's tolerance can't be set.
- `day_census`: `Stretch` gains `moving: BTreeMap<String, u64>` (by purpose) and
  `by_mood: BTreeMap<String, …>` (per §6a), filled at 1176-1178. Print a per-room
  week table of purpose × mood. Per slot, print only the purpose split, because a
  slot isn't the band's unit.
- Keep `doing()` and `group()` as they are. A purpose suffix on "Walk" would break the
  one-key-one-group assumption and change every census's `time:` row. Better to record
  purpose apart.

## 9. Design: the band test (step 2)

- **Shape**: 12 plain `#[test]` fns, one per room × mood (or a `macro_rules!`
  generating them), so nextest runs them in parallel and a failure names its cell.
  Each runs `simulate(room, seed, minutes, Some(mood))` for N seeds and asserts
  in-sight moving / in-sight time is inside the band. Bands from the brief: lazy
  ≈ ≤15, ordinary and dreamy 20–25, industrious ≤30. Each needs a lower bound too
  (she should still move: perhaps ≥10) and a tolerance to be set from step 1's
  per-seed spread.
- **Unfed**, so the mood can be forced (§6) and the result compares with 5a. A fed
  per-mood band would need §6(b), or `home_at` at a fixed afternoon.
- **Budget**: 6 seeds × 15 min = 90 sim-minutes is about 1.4 s for the resident or
  home and about 2.7 s for the stage (estimate from §7). 12 tests is about 25 CPU-s,
  spread across the gate's cores; none comes near the ~13 s long pole. If the spread
  needs more seeds, shorten the minutes rather than pin counts (memory: "shorten the
  simulated span"). A proposed `CENSUS_BAND_SEEDS` env knob would scale the band for
  deep passes, as `CENSUS_MOODS` does for the census.
- **Failing today**: the estimated 33–38% in sight would fail every cell's upper
  bound except perhaps the dreamy resident's. Confirm after step 1 before writing the
  thresholds.
- Its room list should be stage, home and resident (`visit_census`'s). Consider a
  TV-only home as well, since the brief points at it for "TV from the floor".
- Mark it, like `her_mood_shows`, as a statistics test: re-pinned, never `#[ignore]`d.

## Levers and risks for 5c

**Where each brief item plugs in, and what it moves:**

| Brief item | Plugs in at | Breaks or moves |
|---|---|---|
| 1. split "moving" | `#[cfg(test)]` fn beside `census_group` (osaka.rs:5249), a chain tag in `decide` (5367), the `Visit`/`Stretch` fields in census.rs | nothing pinned; printed tables only |
| 2. band test | new `#[test]`s in census.rs (or `tests/stillness.rs`, proposed) | gate time: about 1.5–3 s each, in parallel |
| 3a. slower Restless/Tidy, lower Walk base | `rise_ms` (brain.rs:70-71), `Want::def` (691-692), maybe `Mood::rate` (189) | **all 14 golden scenes × 4 seeds × 2 modes** (golden.rs:532-560; `UNFED_*` 710-745) and the `osaka_at_home_seed_7` insta snapshot; `a_restless_osaka_mostly_moves` (brain.rs:1034-1042) if Walk+Travel lose the majority at Restless 1.0; `an_uninterrupted_trip_runs_its_course` `set_off >= 8` (census.rs:1394) if fewer jobs are off-floor; `her_mood_shows`'s orderings (should survive) |
| 3b. longer still acts, settling | `Activity::duration` (osaka.rs:1216), `SPACE_OUT_MS` (419), `use_duration` (1130) | goldens; `every_pooled_line_shown…` (its muse loop runs to `SPACE_OUT_MS.0`, census.rs:1607); `the_census_counts_her_vignettes` (fewer uses in 15 min may starve andagi/chopsticks over 3 seeds); `at_home_her_furniture_beats_the_floor` if floor rest grows (furniture > 10× floor, census.rs:1442) |
| 3c. nearest spot | `walk` picks uniformly (mind.rs:253-264); `use/*` methods choose seats | goldens; trip counts |
| 3d. daydream sessions | SpaceOut/Muse plan arms (osaka.rs:5742-5750) | goldens; the pooled-line test |
| 4. floor homework, makeshift desk | new Use/Build paths; `census_group` must classify any new act (wildcard-free, so it won't compile until it does); `named()` must name any new `ScriptId` (wildcard-free at 246-259) | `sofa_census` (made-piece fates) gains a second made kind; `image_census` (new frames) |
| 5. the window as a daydream | `use_duration(LookOut)` (osaka.rs:1142), `LOOK_OUT_FACTORS` (brain.rs) | the day census's "look out" counts (about 19 a home day); the afternoon clock glance should return (print-only today) |
| rain-out fix | Visiting's `fading` | 24 traces (plan.md:2997-3000, 3023-3025) |

**Risks:**
- Measuring the wrong quantity: the band has to exclude Away and the door's hidden
  beats (correction 1). Otherwise a shorter work shift would "improve" stillness
  without changing anything on screen.
- Per-seed noise is unknown. `her_mood_shows` already says moving swings "a few
  percent" between seed sets at 4 seeds × 15 min. A ±5-point band at that size may
  flake.
- `arrive_drawn`'s silent `if let` on the mood (census.rs:313): a band test that
  thinks it forced Lazy might not have.
- The day census's 35/40/37 week totals mix moods; a fed band can't be read per mood
  without §6.
- Walking to a job counts (user decision), so in a furnished home the lever is
  **fewer, longer jobs and nearer seats**, not just fewer wanders: there, Walk-act
  time (33%) exceeds the wander choices.

**Open questions:**
1. Does a door's visible part (2.9 s) count as moving for the band? (Suggest yes.
   The door appearing is the attention-grabber.)
2. Is `Build` (walking to text to tear for a made sofa) "to text" or "to a seat"?
3. Unfed only, or also a fed afternoon/evening band through `home_at`, given that 5b
   boosts change the mix by slot (Watch ×2 in the evening, homework ×3)?
4. A lower bound per mood: what is "too still"?
5. Should the day census gain a mood split (§6a), or is the per-mood claim left to
   the unfed census?


---

<!-- section: makeshift -->
# Code map for 5c: makeshift furniture, homework and the art pipeline

HEAD `cabf5d1`. Paths are relative to `dessplay/src/ui/houseguest/` unless they
start with `docs/`. Anything marked **(proposed)** does not exist in the code.

Brief items this section covers (docs/plan.md, Phase 5c, step 4, lines
3052–3058): floor homework lying on her front (3052), the **makeshift desk**
driven by its discomfort (3053), reading a pulled line like a book, HG #72
(3055), and **more makeshift furniture while she owns no real piece**, "three
times the weight on make one" (3056–3057). Also the art pipeline for the new
poses ("New art is fine this phase (model sheets first, as always)", 3017).

---

## 1. What a makeshift piece is (scrap.rs)

- `Scrap { id: MadeId, glyphs: [(char, Color); GLYPHS], len, stage, seed }`
  (scrap.rs:25–36). Constants: `GLYPHS: usize = 10` (:18), `MIN_GLYPHS: usize = 5`
  (:20), `STAGES: u8 = 4` (:22). `done()` means `stage >= STAGES` (:53).
- **The kinds she can make:** `pub(super) const MAKES: [Furniture; 2] =
  [Furniture::Sofa, Furniture::Bed];` (:94). A makeshift piece reuses the real
  `Furniture` kind. No variant of its own.
- **Footprint:** `footprint(item)` gives `Bed => (7, 2)` and **`_ => (7, 3)`**
  (:99–103). Her box is centred on `SEAT: i32 = 3` (:107).
- **ASCII drawing:** `Scrap::cell(item, facing, dx, dy)` (:67). The bed is a
  pillow plus mattress. **`_ =>` gets the sofa's backrest, arms and seat**
  (:77). While it's still a heap, it draws only the middle of the bottom row.
- **Line art:** ellipses of shreds. `static BED: [Heap; 3]` (:155) and
  `static SOFA: [Heap; 4]` (:161). `heaps()` falls back with **`_ => &SOFA`**
  (:168–172). Each shred is the `alien_bits` pattern of a torn glyph, in the
  text's own colour. `render(item, scrap, part, facing, outline, w, h)` (:334)
  goes through `art::rasterize`. `Part::Front` exists only for the bed (:343).
- Tests: `nothing_makeshift_tells_the_time` iterates `MAKES` (:367).
  `a_piece_renders_inside_its_footprint_at_every_stage` and
  `every_ascii_cell_is_one_of_its_letters` iterate `MAKES` too.
  `scraps_sheet` (:436, `#[ignore]`, `HOUSEGUEST_SCRAPS=…`) is the review PNG.
  It has rows for each stage and her `Lounge`/`Nap`/`Sleep` poses in use.

**Risk:** if `Furniture::Desk` is added to `MAKES`, the three catch-alls
(`footprint` :102, `cell` :77, `heaps` :171) quietly give it the **sofa's**
shape. Nothing fails to compile.

## 2. Where she can make one: `scenes::builds` and `mod.rs::builds`

- `Build { x, y, row, side, cells, glyphs, piece: Shown, then: Use }`
  (scenes.rs:84–95). She tears `cells` off line `row` beside her box. The piece
  stands **centred under her** (`left: pull.x - cols/2`, scenes.rs:674).
- `scenes::builds(buf, pulls, items, id, clear, then)` (scenes.rs:648). It needs
  a pull of at least `MIN_GLYPHS + 2` glyphs (:662). The piece must pass
  `room::fits` and `room::roomy`. She tears up to `GLYPHS` glyphs and always
  leaves at least 2. It pushes one `Build` per use in `then(&piece)`.
- `mod.rs::builds` (mod.rs:3585):
  - `wanted` is `MAKES` minus kinds with a piece **standing** in
    `visit.made` (mod.rs:3593).
  - It returns nothing if `layer.cells().count() + GLYPHS > layer::CAP` (:3595).
    `CAP` is 60 (layer.rs:20).
  - `then` = `seats_of(piece…)` minus `Crumple`. It's judged with the piece
    already furnished into the terrain, and the crumple spot must be `restful`.
- **Discrepancy (flagged, not resolved):** design.md:1869–1870 says "She makes
  at most one of each a visit and doesn't make a second while the first
  stands". The code at mod.rs:3593 filters only on pieces still in
  `visit.made`. `tend_made` drops pieces that fell apart (mod.rs:3520), so
  after one falls apart she may make another of that kind in the same visit.

## 3. The act: tear, reel, make, crumple (osaka.rs)

- `Job::Build(build)` becomes `Act::Tear` (osaka.rs:3622). She braces for
  `BRACE_MS = 700` (:428), says `RIP = "Rrrip!"` (:431, said at :2989), then
  reels at `REEL_MS = 220` per step (:430). Each step pushes `LayerOp::Reel`.
  The pose while reeling is `Pose::Pull`.
- Once every glyph is in her hands, `LayerOp::Make { row, cells, piece,
  purpose }` (osaka.rs:3012; scenes.rs:281). Then
  `pursue(Job::Use(seat(Use::Crumple)))`. The guest applies the op and pushes
  `made_of(...)` into `visit.made` (mod.rs:2055, made_of at :3558). The seed
  comes from `left`/`floor`/`row` (:3567).
- Crumple: `Use::Crumple => (4_000, 6_000)` ms (osaka.rs:1141). Its script says
  `SCRUNCH`/`THERE` (:432–433) over `Pose::ToeTouch`. `tend_made` grows
  `stage` as she goes (mod.rs:3543). `HomeEvent::Crumpled` sets it to `STAGES`
  (mod.rs:2697).
- If she's interrupted while reeling, the text goes back: `layer.unreel`
  (mod.rs:2071), and she owes a `Loss::Tear` ("...never mind.", 1 in 4).

## 4. Choosing to make one: the mind's method table (mind.rs)

- `const USE: &[Method]` (mind.rs:194) tries, in order: `use/finish-my-heap`
  (:344), `use/mine` (:357), `use/real` (:443), `use/made` (:450), and
  **`use/make`** (:457). `Want::Use(Use::Crumple) => &[]` (:215): crumpling is
  never chosen for itself.
- `places(what, chances, w)` (mind.rs:382):
  - Each makeshift kind is allowed when no **real seat for that kind is in this
    frame's `chances.seats`**.
  - Otherwise it's allowed on a whim, `w.chance("makeshift", item as u64, 1,
    MAKESHIFT_ODDS)` with `MAKESHIFT_ODDS = 20` (:24, :405). When that whim
    fires, the real one is dropped.
  - `Place::Make(item)` (:374) is added once per allowed kind that has a build
    and no made seat yet.
- `place()` (mind.rs:434) picks **uniformly**: `w.below("place", places.len())`
  (:439). The three methods share the same draw, so exactly one of them binds.
  **This is where "3× on make one" would plug in (proposed):** a weighted pick
  that gives `Place::Make` a weight of 3 while she has no real piece for that
  use.
- `pick_build` (mind.rs:546) weights the spots where she'd make it: the chat by
  `CHAT_FACTOR = 0.1`, and a sofa that would face the TV by `FACING_TV = 5.0`
  (:28).
- **"Owns" vs "on offer":** `places()` checks real seats offered this frame.
  A boxed, closeted (doesn't fit) or unreachable real desk counts as absent.
  `Home::owns` (room.rs:1049) is the ownership test the brief's wording ("owns
  no real piece") implies. Open question.

## 5. How a made piece is used, and let go

- **Purpose lives on the piece:** `struct Made { piece, torn, purpose, used }`
  (mod.rs:530), stored in `Visit.made: Vec<Made>` (mod.rs:432), with
  `next_made` at :434. What she sees is `osaka::Mine { id, purpose, done, used,
  at }` (osaka.rs:144). `Chances.mine` is at osaka.rs:38 and `next_for` at :339.
- Once in shape, it's for whatever the real piece is: `Shown::uses()`
  (room.rs:887) returns `[Crumple]` while it's a heap (:890), then
  `item.spec().uses`.
- Seat: the `_ if self.scrap.is_some() => (mirrored(scrap::SEAT),
  self.facing)` arm in `Shown::seat` (room.rs:953). Every makeshift piece seats
  her **in** it at column 3. (A real desk seats her at `sit: Some((7, true))`,
  one cell past its end and facing back, room.rs:216.)
- `Osaka::leftover` (osaka.rs:5799) runs before any fresh choice. It takes the
  nearest unfinished or unused piece first. Each setting-off is a try. After
  `TRIES = 3` (:165) she owes `Loss::LetBe` ("Nah.", 1 in 2) and leaves it be.
  `tries_at` is at :5860 and `gave_up` at :5878. Going back says `AH_RIGHT`.
- The needs see it as `Bind::on` → `Spot::Made` for a makeshift `Job::Use` or a
  `Job::Build` (mind.rs:152, :154). `quality(Comfort, Made) = 0.6` and
  `(Comfort, Floor) = 0.1` (brain.rs:340–341). Sleepy gets 0.7 made and 0.3
  floor.
- **Gone:** `tend_made` (mod.rs:3506) keeps a piece while its torn glyphs stay
  torn, it still `fits` clear of real furniture (`real.iter().any(cover)`), and
  there's no resize. Anything else ends it, and the text is mended. If it goes
  before she used it, she owes `Loss::Piece(item)` (mod.rs:3535).
  `Loss::says`: `Piece(Bed) => MY_BED`, **`Piece(_) => MY_SOFA`**
  (mind.rs:598–599). **Risk:** a lost makeshift desk would say "...my sofa."
- At goodbye, what she made rains out with what she moved (mod.rs:668–669).
  Made seats never feel a home rule: `PieceRef::Made(_) => None` (osaka.rs:3428).
  `rules.rs` filters `scrap.is_none()` everywhere (:204, :265, :290, :311).
  So the Lamp-near-Desk rule (rules.rs:72–79, `felt_on: [Sleep, Homework]`)
  can't fire at a made desk.
- **The test** `every_made_piece_is_used_or_let_go` (tests.rs:7452, proptest,
  `proptest_cases(8)`, 180 s of visit) cues `Scene::MakeSofa`, with chat every
  20–40 s and optionally `Scene::MakeBed` (:7515). It asserts three things:
  - while a piece waits that she can get to and hasn't let be, she chooses
    nothing new (:7498–7511);
  - every piece lost unused is mourned (:7541–7550);
  - nothing waits longer than `BOUND = 90_000` ms without progress
    (:7460, :7551–7562).

  It's the slowest houseguest test (45–47 s at 256 cases, 120 s override,
  plan.md 5b "Measured"). It names only sofa and bed scenes, so a desk would
  need its own cue.

**Real furniture replacing it:** nothing removes a standing makeshift piece
when a real one arrives. The real one wins the next choice (19 in 20). The
made one lasts out the visit unless a real piece is set down over it (then
`fits` fails and it falls apart). Next visit she won't make one, because the
real seat is on offer. `rules::Frame.made` (rules.rs:409) keeps her moves off
made footprints.

## 6. Is there homework today?

- **Yes, at a real desk only.** `Furniture::Desk` (room.rs:29) is `DESK`
  (room.rs:207) with `uses: &[Use::Homework]` (:213). `Use::Homework` is in no
  other piece's uses, and `MAKES` has no desk.
- Want row: `Want::Use(Use::Homework)` gets `row(8.0, [(Daydreams, 0.3),
  (Comfort, 0.2)])` with `HOMEWORK_FACTORS` (brain.rs:722, :591):
  - `CHAT`;
  - `Clock(Slot::Homework) × BOOST_STRONG (3.0)`;
  - `Season(EXAMS) ×2` and `Season(PANIC_WEEK) ×2`.
- Duration: `Use::Homework => (30_000, 60_000)` (osaka.rs:1135), or
  `HOMEWORK_IN_SLOT_MS = (120_000, 240_000)` in the slot (:1148).
- Script `ScriptId::Homework` (script.rs:204, keys at :1339–1356):
  - `Bob(Pose::Homework)` up to 1/2;
  - `Still(Homework(2))` with `Dots` up to 3/4;
  - `Still(Homework(3))` with `Zzz`.
- Splice `Chopsticks`, `around: &[Use::Homework]`, `when: |_| true`
  (script.rs:727–735). **Risk:** nothing checks the seat, so it would also wrap
  homework at a made desk, with the stool rigs (art.rs:492).
- **In a bare room nothing is offered in `Slot::Homework`.** No method binds
  `Want::Use(Homework)`, so the ×3 never applies. The floor homework and the
  made desk fill this gap.
- Golden `homework_evening` (tests/golden.rs:376) runs in `home_at` (:353),
  which **has a real desk** (:358). Wanting a desk-less homework golden is new.
- Scene and Activity: `Scene::Homework` exists (stage.rs:77; `ScriptId::Homework`
  maps to it at script.rs:553). It is cued on a real desk. `Activity`
  (osaka.rs:1183) has no homework variant: Sit, LieBack, LieFront, Jacks,
  ToeTouch, Stretch, Gaze.
- There's no "homework" need. `Need` has 10 needs (brain.rs:21–47), and needs
  rise only with time (`Needs::pass`, `impl Needs` at brain.rs:412–520,
  `rise_ms` :67). The only other mutators are `serve` (brain.rs:496, subtracts)
  and `enjoyed` (fun tolerance). **No in-game act raises a need.** The one
  caller with a negative amount is the stage's `press`:
  `self.needs.serve(need, -1.0)` (osaka.rs:6344). `serve` already accepts a
  negative amount, so a "discomfort" bump could hook there (proposed).

## 7. Reading a pulled line (HG #72)

docs/proposals/2026-09-28-houseguest.md:560–561:

> 72. **Reading** — pulls a playlist row out like a book spine, reads,
>     slides it back. [C]

It isn't built. Today `Use::Read` is only beside a bookshelf (BOOKSHELF,
room.rs:236; beside-spot `Use::Watch | Use::Read | Use::Snack | Use::Pet`,
room.rs:961). The machinery a pulled-line read could reuse:
- the tear and reel ops (`LayerOp::Reel`; `TextLayer::tear` layer.rs:143,
  `unreel` :187, `torn_intact` :194, `mend` :201);
- `Loss::Tear` on interruption.

The new part is the end. Instead of `LayerOp::Make` she would `unreel` (slide
it back). Holes count toward `CAP`.

## 8. Watch without a sofa (the brief's check)

**Watch already binds without a sofa.**
- The TV's own `uses: [Use::Watch]`. `Use::inside()` is false for Watch
  (room.rs:438), so `spots_for` uses `piece.beside()` (mod.rs:3450).
- The look's host is `Pose::Lounge` only when `seat.item == Sofa`, otherwise
  **`Pose::Sit`** (hugging her knees) (osaka.rs:7189–7194).
- She sits *beside* the TV and faces it. She doesn't sit in front of it.

"Cross-legged before it" is a new pose (and maybe a new spot), not a new
binding.

## 9. A makeshift desk and floor homework: what each would need (proposed)

**A. Floor homework, lying on her front with a paper.** The cheapest fit is a
new `Activity` (osaka.rs:1183), say `Activity::Homework`. It would go through
`Here::Idle` and get `Spot::Floor` (mind.rs:155). It touches:
- `Activity::ALL` (7), `restful`, `duration`, `period`, `look` (osaka.rs:1195–1248);
- `Want::ALL` (27, brain.rs:636);
- a `def` row, and `HOMEWORK_FACTORS` if it should get the slot's ×3;
- `Scene` and the stage;
- a new pose.

The other option is a `Place::Floor` for `Use::Homework` in `places()`, so the
want, its factors and the chopsticks all carry over. Open question.

**B. The makeshift desk.**
- Add `Furniture::Desk` to `MAKES`, with explicit arms in `footprint`, `cell`
  and `heaps`. A low "cube" is suggested, e.g. 3–4 × 2 cells.
- Give it a per-item seat. A const `SEAT` seats her *in* it.
- `Shown::seat`'s makeshift arm (room.rs:953).
- `roomy` must allow for her spot beside it (room.rs:1518).
- `scenes::builds` centres the piece under her (scenes.rs:674). For a desk she
  sits beside, the build spot and seat differ.
- `Loss::says` needs a "...my desk." pool.
- A `Scene::MakeDesk` cue. stage.rs:509–512 is an `if MakeSofa {Sofa} else
  {Bed}`.
- The homework script needs a floor-desk pose branch, not `Pose::Homework`
  (stool, desk height).

"Its discomfort drives the desk like the sofa": the sofa isn't driven by
discomfort today. `Lounge` serves Comfort 0.5, and a made spot scores 0.6
quality against no option at all. For the desk:
- if floor homework is `Spot::Floor` (0.1) and desk homework is `Spot::Made`
  (0.6), high Comfort tilts toward the desk;
- but Homework serves Comfort at only 0.2, so the tilt is small;
- a stronger lever (proposed): floor homework *raises* Comfort, or sets an
  "ache" that the made desk answers.

**C. Persistence: no format change** if the desk stays a per-visit makeshift
reusing `Furniture::Desk`. Made pieces live only in `Visit.made` (mod.rs:432).
`ledger.rs` has no makeshift or scrap field:
- `Saved` (ledger.rs:568), `SavedProp` (:451), `SavedAnchor` (:605);
- `VERSION = 1` (:47).

A **new `Furniture` variant** would instead touch:
- `Furniture::ALL` (12, room.rs:53) and `legacy()` (room.rs:76);
- the serde names, the `Spec`, `parts()` (art.rs:870), and the
  older-build-skips tests (ledger.rs:799, :1250, :1328).

Avoid it.

**D. Match sites a Desk-in-MAKES hits.**
- **Silent wildcards:**
  - scrap.rs:77, :102, :171;
  - mind.rs:599 (`MY_SOFA`);
  - room.rs:953 (one `SEAT`);
  - stage.rs:510 (binary `if`);
  - `osaka.rs:7189` host (Sit for a desk, fine).
- **Wildcard-free (compile errors guide you):** `Surface::of` (script.rs:640,
  lists `Use::Homework`), `ScriptId::played_on`/`scene`, `Use::script`
  (script.rs:1202), `Loss::kind` (mind.rs:613).

## 10. The art pipeline

- **Her sprites, two renderers:**
  - ASCII: `sprite.rs`, 5×4-cell boxes. `rows(pose)` is at sprite.rs:196, e.g.
    `SIT` :126, `LIE_BACK` :127, `LIE_FRONT` :132, `GAZE` :145,
    `HOMEWORK` (4 frames) :157, `READ` :168. `sprite::ALL: [Pose; 47]` (:380)
    feeds the lints.
  - Line art: `art.rs`, a `Rig` (art.rs:76). It has `profile`, `turn` (±90
    lies her down), `scale`, `stool` (:97), `seated`, `legs_front`, and
    `hold: Option<(&str, x, y, angle)>` (:110), a held SVG part. Parts are in
    `art/osaka.svg` (book, book-turn, stool, chopsticks…) and furniture is in
    `art/props.svg` (desk, sofa… listed by `parts()` art.rs:870).
  - `graphics.rs` composites `Look::Pose`, `Look::Prop`, and
    `Look::Scrap(item, scrap, part)` (graphics.rs:81, :118).
- **Lints a new pose meets:**
  - `every_pose_is_drawn_its_own_way` (art.rs:1573: no two rigs equal, except
    Nap frames and `Homework(0|1)`);
  - `every_pose_renders_something_inside_the_box`;
  - sprite.rs `every_pose_has_a_head`, `every_head_has_three_face_cells`,
    `every_pose_fits_the_box_and_is_ascii`;
  - `every_use_look_span_is_half_open` (osaka.rs:7636, a case per `Use::ALL`
    :7810).
- **Review-sheet tests** (`#[ignore]`, env var → PNG):
  - `model_sheet` (art.rs:2302), `use_sheet` (:2509);
  - `vignette_sheet` (:1397), `clock_sheet` (:1791), `catalogue_sheet` (:2015);
  - `scraps_sheet` (scrap.rs:436).
- **How the 5a and 5b art was made:**
  - It's drawn in a **throwaway worktree, committed there with git**. Memory:
    jj inside a worktree acts on the main working copy. Examples: 5b `bba7efc`;
    decor "nothing was committed".
  - It's saved under the design dir:
    - `phase5b/art/`: `gen.py` generates SVG groups from 1×-pixel geometry and
      splices them into `props.svg`. Its header says
      `px x -> x * 20/9 units`, hung pieces at scale 0.45. With `snippets.md`
      and `worktree.diff`.
    - `vignettes/`: `snippets.md`, `worktree.diff`, sheets.
    - `decor/`: `snippets.md`, `worktree.diff`, `new-pieces-sheet.png`.
  - Sheets come at three sizes, over `#1e2127` with the outline `#1d1714`:
    `*-1x.png` (true size, 9×19 px cells), `*-1x-nn3x.png` (judge
    readability) and `*-3x.png` (judge the drawing).
  - **Approval:** the design doc says the sheet "needs the user's approval
    before wiring" (phase5b-design.md:403). The approval is recorded with its
    choices (:568, "Art approved (2026-10-04)…"), then wired as its own step.
    5a noted that ASCII frames not on the sheet went unreviewed (plan.md:2769).
- **docs/houseguest-style/README.md** is a style study (sticker chibi, easel
  CLI, 2026-10-01): "Nothing here is wired into the client yet." It holds the
  target look and an open pose-box-size issue, not production parts.

### Poses and props 5c would need

| Want | Reuse | New (proposed) |
|---|---|---|
| Floor homework, lying on her front with a paper | `LieFront` rig (art.rs:218: turn 90, chin on hands) as the base | Writing arms and a `paper` (+ pencil) `hold` part. Frames: writing ×2, dozing face-down. ASCII: `LIE_FRONT` already fills all 5 columns (`"\\_V_o"`), so a paper glyph has no column. Redraw needed. |
| Crumpled-cube desk | Shred renderer (heaps, alien glyphs) | A `Desk` heap set and ASCII cell shape. A low cube, footprint TBD. |
| Homework at the cube | Expressions; the `homework()` arm aiming (art.rs:461) | A floor-seated (kneeling or cross-legged) writing pose, `stool: false`, with a lower surface. Nod-off frames. |
| Reading on the floor | `Read` / `reading()` (art.rs:548: floor, knees up, `hold` book) | Only a new held part, a torn text strip (or a book-spine row), plus an ASCII variant of `READ` `"[]V| "`. |
| Cross-legged TV from the floor | `Sit` is the current host (hug knees) | Cross-legged seated rig, maybe with a spot in front of the TV. |
| Cloud-watching lying back | `LieBack` rig (art.rs:201: hands behind head) | Probably only `Activity::look` changes: Curious/Vacant face, musing bubble instead of `Blink`+`Zzz` (osaka.rs:1242). |
| Long window daydream | `Gaze` (standing, used by LookOut) | A lean, or chin on hands at the sill, for a long hold. Standing `Gaze` for 60 s+ reads stiff. |

## Levers and risks for 5c

**Where each brief item plugs in**
- **3× on "make one" while she owns none:** a weighted pick in `place()`
  (mind.rs:439) instead of `w.below("place", n)`. Gate it on
  `!chances.seats.any(real of kind)` or on `Home::owns` (needs plumbing into
  `Chances`).
  - It changes every decision with ≥2 places, so all goldens move (fed and
    unfed).
  - `sofa_census` (tests.rs:8012) and `visit_census` "made" columns
    (tests/census.rs:207) shift.
- **Makeshift desk:** `MAKES` (scrap.rs:94) plus the explicit shape arms, a
  per-item seat (room.rs:953), `Loss::says` (mind.rs:599), `Scene::MakeDesk`
  (stage.rs:509), and a homework branch for the made desk (script.rs:1339).
  - `Want::Use(Homework)` becomes offerable in **every desk-less room**,
    boosted ×3 at `Slot::Homework`.
  - It moves `golden_stage_room` (tests/golden.rs:157, a bare stage that cues
    MakeSofa and MakeBed) and the day census's school-night share.
  - It eats a third ≤10-glyph slice of `CAP = 60`.
- **Floor homework:** a new `Activity`, or `Place::Floor` for `Use::Homework`
  (§9A). A new `Activity` grows `Activity::ALL`, `Want::ALL` (27) and the
  stage scenes. A new `Use` grows `Use::ALL` (11, room.rs:418), `ANY_USE`
  (rules.rs:50), `CHAT_WANTS` (14, brain.rs:1293) and the half-open look
  lint.
- **Reading a pulled line:** a `Job` variant that reuses `Act::Tear`'s reel
  and ends in `unreel`. `Loss::Tear` already mourns an interruption. Choose
  the `Use::Read` want's place (`Place::Line`, proposed) so READ_FACTORS
  carry.
- **Watch from the floor:** already bound (`Pose::Sit` beside the TV). The
  stillness lever here is only longer `Use::Watch` (20–45 s, osaka.rs:1136)
  and settling in.

**What breaks**
- **Goldens:** any new offer, weight or duration re-records the 10 scenes × 4
  seeds × 2 renderers plus the `UNFED_*` tables (tests/golden.rs header).
  Batch the re-recordings with the rain-out fix's 24.
- **`every_made_piece_is_used_or_let_go`** (tests.rs:7452) covers sofa and
  bed only. A desk needs a cue in it. At 45–47 s it's already the long pole.
- **Pose lints:** `sprite::ALL` 47 (sprite.rs:380), the distinct-rig lint
  (art.rs:1573), the box-fit and head lints.
- **The chopsticks splice** wraps any `Use::Homework` (script.rs:730). Add a
  real-desk guard, or give the made desk a floor-seated chopsticks frame.

**Open questions**
1. Is "while she owns no real piece" `Home::owns` (boxed and closeted count
   as owned) or "no real seat on offer this frame" (today's `places()`
   test)?
2. Should the "one of each a visit" doc rule be made true (track kinds ever
   made), or should design.md be corrected to "while one stands"
   (mod.rs:3593)?
3. Is floor homework an `Activity` (cheap, `Spot::Floor`, no chopsticks) or a
   place for `Use::Homework` (inherits the factors, script and splice)?
4. How should discomfort drive the desk? Today no in-game act raises a need (only
   the stage's `press`, via `serve(need, -1.0)`, osaka.rs:6344). Either
   add one (floor homework raises Comfort) or rely on `quality` 0.1 vs 0.6 at
   a 0.2 serve, which is weak.
5. The made desk's shape and seat: does she sit beside it (like the real
   desk's `(7, true)`) or behind it? This decides the build geometry
   (scenes.rs:674) and the roomy check.
6. Should a real desk delivery end a standing makeshift desk (rain it out with
   a beat), or let it last the visit as the sofa and bed do?


---

<!-- section: window -->
# Code map for 5c: the window, the clock glance, daydreaming, settling in, nearest spot

Paths are relative to `dessplay/src/ui/houseguest/` unless they start with `docs/`. HEAD cabf5d1.
Anything that does not exist yet is marked **(proposed)**.

## 0. The brief items this section covers

- 5c step 5: "The window as a daydream: rarer, much longer look-outs; then check the afternoon
  clock glance has room again" (docs/plan.md:3061-3062).
- Step 3 levers: "settling in", "prefer the nearest spot that answers a want", "daydream
  sessions: a long space-out with musings in a row, or lying back cloud-watching"
  (docs/plan.md:3040-3047).
- 5b "Open": look-outs (about 19 a game day) crowd out spacing out, so the afternoon glance came
  once in six home-weeks; the glance is once per visit, not per game day; D4's "Gaze ×more with a
  window" isn't built (docs/plan.md:2988-2991). The source of D4's item:
  docs/proposals/2026-10-02-houseguest-mind/phase5b-design.md:219-220.
- The design rule today: design.md:2029-2048 (look-out 15–30 s, "twice as likely from 17:00 to
  05:00", afternoon glance "a musing in three, once a visit").

## 1. The window piece (room.rs, mod.rs)

- `Furniture::Window` is listed at room.rs:45. Its spec `WINDOW` is at room.rs:323-337: footprint
  `(4, 2)`, `uses: &[Use::LookOut]`, `offers: &[]`, `hang: Some(4)`, `beauty: 0.0`,
  `symmetric: true`. So the window is **not decor**. It doesn't ease Beauty, which the clock does
  (beauty 0.5, room.rs:306-322).
- `tells_time()` is true for Clock and Window (room.rs:92-94). The sky is
  `PieceState::Sky(art::Sky::at(minute))` (mod.rs:3732). It changes on her clock's quarter-hours
  (mod.rs:1475).
- `Use::LookOut` is defined at room.rs:411-412. `inside()` is false for it, so she uses it from
  beside the window (room.rs:436-440). `asks_room()` is false for it (room.rs:450-452): a window
  is hung wherever the wall has room. A new window comes in first where she could look out of it
  (room.rs:1311, 1353; `room_to_look` at room.rs:1522-1527).
- Where she stands: `Shown::look_out_spots()` gives `[i32; 4]`, two spots under the window (a
  cell either side of its middle, on the side she faces from), then the two `beside()` spots
  (room.rs:1003-1011). `Shown::seat(Use::LookOut, beside)` faces her toward the window's middle
  (room.rs:969-976).
- Which spot is offered: in `spots_for` (mod.rs:3437-3462), the first of the four spots where
  `seat_spot` holds and `clear_of(shown, x, y)` holds (her box overlaps no shown piece,
  mod.rs:3426-3435). `seats_of` also keeps only restful spots (mod.rs:3410-3414). **So a window
  offers at most one LookOut seat.**
- Rules: `Use::LookOut` is in `rules::ANY_USE` (rules.rs:50-60), so she can feel the
  `Rule::Belongs` grievance ("Hm... not in here.", felt on `ANY_USE`) while looking out
  (rules.rs:99-103). I did not check whether a hung window can be lifted and tried in an arrange
  episode (see open question Q6).

## 2. How a look-out is chosen today

1. **The table row.** `Want::Use(Use::LookOut)` is the 26th of 27 in `Want::ALL`
   (brain.rs:641-669). Its row (brain.rs:737-740):
   `row(8.0, &[(Need::Daydreams, 0.5), (Need::Fun, 0.3)])` with `LOOK_OUT_FACTORS`.
   - `LOOK_OUT_FACTORS = &[CHAT, Factor::Clock(routine::DUSK_TO_DAWN, BOOST)]` (brain.rs:608).
     `BOOST = 2.0` (brain.rs:574). `DUSK_TO_DAWN = When::Hours(17:00, 05:00)` (routine.rs:179).
     `CHAT` is ×0.1 when the seat is in the chat pane (brain.rs:569-571; `CHAT_FACTOR`,
     osaka.rs:169).
   - It competes for Daydreams with SpaceOut (`row(6.0, Daydreams 0.6)`, brain.rs:671), Gaze
     (`row(6.0, Daydreams 0.5)` + `GAZE_FACTORS`, brain.rs:690, 605), LieFront (brain.rs:686-688),
     Homework and Read. **LookOut has the highest base of them, and the only Fun term.**
   - LookOut is one of the 7 `FUN_SOURCES` (brain.rs:351-359). Each whole use adds
     `TOLERANCE_PER_USE = 0.6` to its tolerance, which wears off over `TOLERANCE_MS = 10 min`
     (brain.rs:361-363). That staleness only scales the Fun term, so the Daydreams term keeps it
     offered.
   - Scoring is `base × fit × COOLDOWN^repeats`, with `COOLDOWN = 0.4` over the last
     `RECENT = 3` choices (brain.rs:795-817, 787; osaka.rs:409). `choose` keeps the
     `TOP = 4` and rolls weighted (brain.rs:823-856).
2. **Binding.** `Want::Use(_)` uses the `USE` methods (mind.rs:194-200; `methods`, mind.rs:203).
   For a window, `use_real` → `place()` (mind.rs:434-441). That is
   `w.below("place", places.len())`, **uniform over every LookOut seat in the room, with no
   distance term**. `places()` (mind.rs:382-431) only filters makeshift against real pieces.
3. **Getting there.** `plan` → `Bind::Job(job)` → `go_to` (osaka.rs:5885-5923). On her floor that
   is `pursue` → `Act::Walk { to, then: Then::Job(job) }` (osaka.rs:6348-6360; `Then`,
   osaka.rs:633-641). On another floor it is a `Heading` plus travel, or a door.
   `census_group` counts every `Act::Walk` as `"moving"`, whatever its `then`
   (osaka.rs:5260-5268). **So the walk to the window is already part of the 35–45%.** She walks
   at `WALK_MS = 333` ms a column (osaka.rs:358), so a 30-column walk is 10 s against a
   15–30 s look-out.
4. **Starting.** `start_job` → `Job::Use(seat)` (osaka.rs:3386 ff.):
   - The length is `use_duration_in(seat.what, slot)` (osaka.rs:3414). For LookOut that is
     `use_duration`'s `(15_000, 30_000)` (osaka.rs:1142). It is not lengthened by slot
     (osaka.rs:1153-1158).
   - Splices are rolled (osaka.rs:3468-3483), but no splice row lists LookOut in `around`
     (script.rs:716-765). LookOut is `Host::Use` (script.rs:502).
   - The branch is `self.look_out_branch(at)` (osaka.rs:3563-3566). It is defined at
     osaka.rs:4606-4612: the sky from `Sky::at(day.minute)` (`Sky::Day` when unfed), then
     `whims.below_at("look-out", 0, 6)`.
   - **There is no `try_play` or script cooldown for LookOut**, unlike Surf or the Escalator
     (`Lines::try_play`, mind.rs:1130-1139). The same line can come on consecutive look-outs.
5. **Credit.** `credit_done` (osaka.rs:4945-5019) credits a `Use` by the share of its body done
   (osaka.rs:4976-4999). `serve` subtracts `amount × share × quality × fresh` (osaka.rs:5032-5044).
   **The amount is per whole act, not per second.** A 2-minute look-out eases exactly what a
   15-second one does (Daydreams 0.5, Fun 0.3·fresh), while Restless (fills in 90 s,
   brain.rs:70) keeps rising throughout.
6. **Census.** A look-out counts as `"spacing out"` (osaka.rs:5252). tests/census.rs:243 names
   it "look out". 5b measured about 19 look-outs a game day in the furnished home
   (docs/plan.md:2973-2975).

## 3. How long it lasts and what she says (script.rs)

- `LOOK_OUT_LINES: [(Sky, &str); 11]` (script.rs:1703-1715) has three Day lines ("Sunny!", "A
  bird!", "That cloud's a bun."), two Dawn, two Dusk ("Pretty...", "The sky's on fire."), two
  Evening ("Lights are on...", "First star!") and two Night ("Stars!", "So many stars...").
- `LOOK_OUT_LINE_MS = 3000` (script.rs:1719). `looking(line)` is two keys:
  `Span::Ms(3000)` with `Pose::Gaze`, `Face::Curious` and `Say(line)`, then `Span::Rest`, Gaze,
  Curious, no bubble (script.rs:1721-1732). `LOOK_OUT` has one branch per line
  (script.rs:1735-1747). `look_out_branch(sky, pick)` takes the pick-th line of that sky,
  wrapping (script.rs:1751-1762).
- **Length lint.** `ScriptId::LookOut.shortest_body()` is `shortest_use_ms()` (script.rs:540).
  That is the minimum of every use's low end and `TRIAL_USE_MS.0 = 3500` (osaka.rs:1128-1134,
  403). `every_set_time_fits_its_shortest_host` (script.rs:2254 ff.) holds every `Span::Ms` key to
  that. **A longer multi-key look-out script must use `Span::Upto(n, d)`** (script.rs:25), or
  change LookOut's `shortest_body` to `use_range(LookOut).0`. That change is safe only if a
  window can never be a trial sit (Q6).
- `on_chat()` is `Chat::Look` for LookOut and ClockGlance (script.rs:406-407): a chat line
  startles her off it (see §8).

## 4. The clock glance (osaka.rs, script.rs)

- **Lines.** `ITS_LATE` "Oh! It's late!" and `SCHOOL_TIME` "Time for school!" (script.rs:1765,
  1767). `HOURS[13]`, "Noon-ish." to "Midnight-ish." (script.rs:1770-1784). `hour_line`
  (script.rs:1787-1792). `ClockGlance { Bed, School, Hour(u16) }`, whose `branch()` is
  0, 1, then `2 + hour_line` (script.rs:1796-1817). `CLOCK_GLANCE` is 15 one-key branches
  (`Span::Rest`, Gaze; script.rs:1819-1846).
- **Length.** `CLOCK_GLANCE_MS = 2500` (osaka.rs:421). It always plays as
  `Act::SpaceOut { play: Some(ClockGlance) }` (`glance_at_clock`, osaka.rs:3817-3835), which
  also sets `credit = None` ("a glance eases nothing").
- **Where.** `ClockOn { x, floor, from, to }` and `seen_from((x, y))`: the clock's column, only
  if she stands on its strip (osaka.rs:66-85). It reaches her through `Chances::clock` and
  `self.clock_on = chances.clock` (osaka.rs:62, 1933).
- **The routine glance.** In `choose_next`, when `to_bed || to_school`, `glance_first` runs
  before `send_to_bed` / `go_out` (osaka.rs:5470-5484). It is defined at osaka.rs:3787-3797 and
  needs `pass_glance` (once per slot, keyed on the slot's end in `clock_glanced`,
  osaka.rs:3802-3811), the clock on her strip, and her not hidden.
- **The afternoon glance.** It is reached only through `muse()` (osaka.rs:2154-2208), which is
  reached only through `Want::SpaceOut`'s first method `"space-out/muse"`, 1 in 3
  (`w.chance("muse", 0, 1, 3)`, mind.rs:223-225). Inside `muse` the order is:
  1. A cued Setsubun, ClockGlance or Escalator.
  2. The rare Escalator, if `rares.allows` and quiet: 1 in 3, then `try_play`.
  3. `hour_glance(now, quiet, false)` (osaka.rs:2282-2296). This needs
     `slot == Afternoon` (12:45–18:00, routine.rs:37-39), `!self.hour_glanced`, `quiet`, the
     clock seen from her spot, and `whims.chance("hour-glance", 0, 1, 3)`
     (`HOUR_GLANCE = (1, 3)`, osaka.rs:425).
  4. Then a riddle or a musing.
- So **each SpaceOut decision has at most about 1/9 chance** of the glance, on the clock's strip,
  in the afternoon, once a visit. `hour_glanced` is an `Osaka` field (osaka.rs:1583-1585), reset
  in `Osaka::new` (osaka.rs:1762), and `Osaka::arrive` / `back_through_door` / `dash_in` build a
  new `Osaka` each visit (osaka.rs:1776-1860, mod.rs:1830-1842). That makes it **once a visit,
  not a game day**.
- Neither Gaze (Idle), LookOut (Use) nor LieBack ever reach `muse`, so no daydream other than
  SpaceOut can produce the glance.

## 5. Finding: spacing out never eases Daydreams (confirmed by reading)

- `credit_done`'s match (osaka.rs:4964-5003) has arms for `Act::Idle`, `Act::Use` and
  `Act::Pull`, then `_ => return`. **It has no `Act::SpaceOut` arm.**
- `serve` is called from five places: osaka.rs:5008 (credit_done), 5018 (Beauty), 5026
  (`credit_whole`: Swap and Pull only, 3057 and 4934), 5707 (Walk and Travel as she sets off) and
  6064 (Arrange). None of them is for SpaceOut.
- So `Want::SpaceOut`'s `Daydreams 0.6` (brain.rs:671) is **never served**. design.md:1459-1460
  says "daydreams rise slowly and spacing out, gazing, lying on her front and drifting off at her
  desk answer them". The answers that actually work are Gaze, LieFront, Homework, Read and
  LookOut.
- This plausibly explains the 5b "Open" item. In a home with a window, LookOut (base 8, plus Fun)
  is what drains Daydreams. That lowers SpaceOut's fit (it shares the need), so the
  `muse` → `hour_glance` path runs less. In the stage and resident rooms nothing drains
  Daydreams as hard, so SpaceOut stays high (9 glances in six weeks there against 1 in the home).
- The osaka.rs:12223-12233 test sets `credit = Some(Want::SpaceOut)` "credited already", which
  suggests the author assumed an arm exists.
- **Blast radius of a fix:** her needs change after every spacing-out, which moves every golden
  in which she spaces out (tests/golden.rs, including the `UNFED_*` tables) and the unfed censuses.
  A daydream session built on SpaceOut (§7a) would need this arm anyway. This is a finding for
  the orchestrator; it isn't fixed here.

## 6. D4's "Gaze ×more with a window" (not built)

- `GAZE_FACTORS = &[Factor::Clock(routine::DUSK_TO_DAWN, BOOST)]` (brain.rs:605). There is no
  window term.
- `brain::factor(want, into_chat, day, date)` (brain.rs:753-776) sees no room state. The
  `Factor` variants are `InChat`, `Clock` and `Season` (brain.rs:543-553). `sane_factor`
  (brain.rs:1110 ff.) requires InChat < 1 and Clock/Season > 1. The Factor doc warns that
  "below 1 would delete the want, since choose keeps only the top few" (brain.rs:547-549).
- **(proposed)** A `Factor::Owns(Furniture, f64)` (> 1) or a `with_window: bool` argument
  (filled from `chances.seats` having a LookOut seat). Either touches `factor`'s signature and its
  two callers (osaka.rs:5671; the brain tests).
- The 5c brief makes the window "a daydreaming place, not a quick glance", so D4's item may be
  better dropped, or redirected so that with a window Gaze relocates to it (a long look-out).
  That is Q1.

## 7. Design levers (all proposed) and where they plug in

### 7a. Daydream sessions: a long space-out with musings in a row

- What exists:
  - `Act::SpaceOut { since, until, play }` (osaka.rs:445). `SPACE_OUT_MS = (6000, 14_000)`
    (osaka.rs:418).
  - The end arm: `Act::SpaceOut { until, .. } if at < until` steps keys, then `decide`
    (osaka.rs:3029-3032).
  - A precedent for chaining without a decision: `Act::Swap { back: true }` → a 1.5–3 s
    SpaceOut (osaka.rs:3039-3048), and Crumple → `Act::Admire` (osaka.rs:2916).
- Shape A **(proposed)**: one act. Musings are pool lines said with `self.say` (osaka.rs:2232-2238),
  not script keys, so a session would be an `Act::SpaceOut` with a `left: u8` counter, or a new
  act. The fire arm says the next musing at intervals instead of deciding.
  - The `MUSINGS` pool has 8 lines (mind.rs:673-687) and `LINE_COOLDOWN_MS = 10 min`
    (mind.rs:878). A 3-musing session spends 3 of the 8 and leaves the rest cooling. The pool is
    unbudgeted (`budgeted()` is only `Beat`, mind.rs:954 ff.).
  - The seasonal pools (PANIC, DECEMBER) are picked first (osaka.rs:2232-2236).
- Shape B **(proposed)**: a new `ScriptId::Daydream` (`Host::SpaceOut`). It is wildcard-free in
  `rarity`, `branches`, `on_chat`, `trial_branch`, `played_on`, `host`, `shortest_body` and
  `scene` (script.rs:298-578), plus a stage `Scene`. Its keys use `Span::Upto`.
- The Escalator (Rare, `Host::SpaceOut`, script.rs:258-259, rarity.rs:187-197) rolls once per
  `muse`. A session that calls `muse` per musing multiplies its rolls but not its plays (gated by
  `rares.allows` and `try_play`'s 10 min, osaka.rs:2187-2192). Pity is unaffected.
- The afternoon glance becomes natural as one beat of a session (it is already "a musing's
  place"). **(proposed)** Let `hour_glance` roll once per session rather than once per `muse`
  call. Also consider once a game day instead of once a visit (Q3).
- It needs the §5 credit arm. With the amount fixed per act, a long session should credit by
  elapsed time, or its amount should scale (Q4).

### 7b. Lying back, cloud-watching

- `Activity::LieBack` lasts `(15_000, 40_000)` (osaka.rs:1219). Its look is
  `(Pose::LieBack(frame), Face::Blink, Some(Bubble::Zzz))` with a 1400 ms period
  (osaka.rs:1228-1243). **Today, lying back already reads as a doze.**
- In ASCII the LieBack sprite has no face cell (`LIE_BACK`, sprite.rs:126-129: "o=V=^"). In line
  art the rig takes an expression (art.rs:199-215, `Rig::for_pose(pose, face)`). So a
  cloud-watching LieBack with `Face::Curious` and a sky line differs from the doze only by its
  bubble in ASCII, and needs an art check in line art.
- `Act::Idle.play` exists (osaka.rs:517-523), but is set only for the floor night
  (`night_on_floor`, osaka.rs:3685-3700). Its appearance arm uses the play's keys
  (osaka.rs:7160-7172), and its fire arm's `sleeping()` path ends the night (osaka.rs:2761-2768).
- `Host` has `SpaceOut`, `Night`, `Splice` and `Use` (enum at script.rs:594; `ScriptId::host`, script.rs:479-504), and no Idle host. A
  cloud-watch script on Idle needs a `Host::Idle` **(proposed)** and the lint updates that come
  with it.
- **(proposed)** Cloud-watching under the window: a LookOut variant on the same seat
  (`look_out_spots`; `clear_of` already guarantees floor room) posed `LieBack` with sky lines.
  It needs a model sheet: lying under a 4×2 window hung at 4 puts her head well below it.

### 7c. Settling in: when a still act ends she settles further

- Where still acts end:
  - `Act::Idle` → `decide` (osaka.rs:2761-2768).
  - `Act::Use` → `decide` (osaka.rs:2906-2922).
  - `Act::SpaceOut` → `decide` (osaka.rs:3030-3032).
- In `choose_next`, `self.act` is still the act that ended: `credit_done` reads it at
  osaka.rs:5395, and `set` replaces it later.
- **(proposed) plug point:** a `Bucket::Continuation` method `"settle"` after `leftover`
  (osaka.rs:5623-5636) and before the offers (osaka.rs:5638). That keeps routine reflexes,
  watching the chat, calendar beats, owed glances, arranging and leftovers ahead of it. It rolls
  `whims.chance("settle", …)` keyed on the ended act and binds in place, with no walk:
  - SpaceOut or Gaze → `Idle(Sit)` (osaka.rs:1218) → `Idle(LieBack)` (the doze).
  - `Use::Lounge` → `Use::Nap` on the same sofa. Both are sofa uses (`Spec.sit`; seat at
    room.rs:956-960), so the seat doesn't move.
  - `Use::Read`, `Use::Watch` (sitting, `Pose::Sit` host, osaka.rs:7189-7193) → nodding off
    **(new art: Q5)**.
  - `Use::LookOut` → sit under the window **(proposed; there is no sitting-gaze pose:
    `Pose::Gaze` is standing, sprite.rs:52-53)**.
- Doze poses usable by day today:
  - `Idle(LieBack)`: Zzz.
  - `Use::Nap`: `(30_000, 60_000)` on a sofa (osaka.rs:1137), `Pose::Nap(u8)`.
  - `Use::Sleep`: `(60_000, 180_000)` in bed (osaka.rs:1138), `Pose::Sleep(u8)`, and its day
    script turns the lamp off (`Prop::LampOff`, script.rs:1280-1286).
  - The night code (`night_in`, osaka.rs:3660-3681; `NIGHT` branches, script.rs:1294-1328: bed,
    sofa, and floor as `Pose::LieBack(0)` still) is gated to the Asleep slot or a stage cue
    (`at_night`, osaka.rs:3436-3452). **It should not be reused by day**: it arms the night
    (`arm_night`) and stirs at chat.
- Needs interplay: Idle(LieBack) serves Sleepy 0.15 and Comfort 0.3 (brain.rs:681-683). Sit serves
  Sleepy 0.1 and Comfort 0.3 (brain.rs:684). Sleepy rises at `SLEEPY_BY_DAY` by day
  (`rate_in`, brain.rs:111-118). Settling into dozes by day eases Sleepy and could push bedtime
  sleepiness down; check the 5b night census.
- Settling must not fire when the act was cut by `interrupt` (a chat line or the routine). Those
  go through `Act::Look`, or decide with `Cause::Routine`, so the ended act is `Look`, not the
  still act. That gives the right behaviour for free, but see §8.

### 7d. Prefer the nearest spot that answers a want

- Today **distance appears nowhere in `score` or `choose`** (brain.rs:795-856). Within one use,
  `place()` is uniform (mind.rs:434-441). `walk` picks any column of her floor uniformly
  (mind.rs:253-265), so walks average about a third of the floor. Travel and door picks are
  uniform too (mind.rs:268-307).
- **(proposed, non-gating)** In `place()`, replace `w.below("place", n)` with
  `pick_weighted(n, |i| weight(distance_i), …)` (osaka.rs:285-305). `Ctx` already has `x`, `y`,
  `here` and `terrain` (mind.rs:10-15). Take the same-floor distance as `|dx|`; for another
  floor, use the hops from `route` (osaka.rs:7344) or a fixed penalty. This keeps every want
  offered, because only the seat changes. A home has one window, so this does nothing for
  LookOut.
- **(proposed, gating risk)** A distance factor on the want itself. Any multiplier < 1 can drop a
  want out of `TOP = 4`, which is the brain.rs:547-549 warning. Expressed as a boost (> 1) for
  near binds, it has the same relative effect. Distance-weighting `walk`'s column is the
  movement lever most directly tied to the band.

## 8. Interactions

- **Chat lines cap every still act.** `Osaka::look` (osaka.rs:6433-6480) runs
  `interrupt(Cause::Chat)` (osaka.rs:6507-6526), unless she's out (`OnChat::Back`), aloft, asleep
  (`Chat::Stir`), or answering a splice. That gives `Act::Look` for `SURPRISED_MS 1200` and then
  `LOOK_MS 4000` (osaka.rs:368-369). After that she runs the `"watching chat"` reflex: `Stand`
  in chunks of up to 5 s until `WATCH_MS = 15_000` after the last line (osaka.rs:371, 5495-5504).
  Then a fresh decision, with **no inertia back to the interrupted spot or act**.
  - The censuses send strictly periodic lines (`now / every` ticks, tests/census.rs:416-418,
    1184-1186): home every 90 s, resident every 60 s, stage every 45 s (tests/census.rs:82, 107,
    54).
  - **So a look-out or daydream longer than the cadence is always cut** in the census rooms, and
    each cut costs a fresh decision, often a walk. This is the hard ceiling on "much longer
    look-outs" and on the band test, unless one of these is built **(proposed)**:
    - resume after the look (the interrupted still act, or its spot, offered with `INERTIA`
      like a heading, brain.rs:781);
    - a Stir-like response for a deep daydream ("Hm?", then on).
  - The user's "a dash keeps looking at chat lines" decision (docs/plan.md:3020) is about dashes
    only; for daydreams this is open (Q2).
- **Text under her:** `recheck` → `Cause::Restless` interrupts any staying act when text comes up
  under her (osaka.rs:5148-5171). Longer acts are more exposed in a live chat.
- **The routine:** `Osaka::due` includes the next cutting boundary (Away or Asleep), and `tick`
  interrupts with `Cause::Routine`. Long acts end cleanly at bedtime and school (design 5b,
  phase5b-design.md:235-243; `Slot::cuts`, routine.rs:111-113). 18:00 doesn't cut.
  - The sky changes on quarter-hours. At 6× (`CLOCK_SPEED`, mod.rs:795), a 2-minute real
    look-out is 12 game minutes, and its line is drawn from the sky at the start
    (osaka.rs:4606-4612). Across 17:00, 19:00 or 21:00 it can go stale; a multi-line script
    should redraw each line's sky.
  - The look-out boost is only 17:00–05:00, so of the Afternoon slot (12:45–18:00) only the last
    hour is boosted. The afternoon look-outs that crowd out the glance are unboosted.
- **Rares and vignettes:**
  - None of the rares is LookOut-hosted (rarity.rs:174-204).
  - Scary is a coda on Lounge and Read from 22:00 (script.rs:757-760). Settling Lounge → Nap puts
    a knee-hugging coda before the nap; check the pose seam.
  - NoMelon and Andagi wrap Snack; Chopsticks wraps Homework. Settling doesn't touch those.
  - The Dream is night-only.
- **The line budget:** look-out lines are script branches, not pool lines, so they cost no budget
  and have no cooldown. Musings are unbudgeted but cool for 10 min each (mind.rs:878-881,
  1062-1082).

## 9. Tests, goldens and tables a change here moves

- **Goldens:** no golden room owns a window (tests/golden.rs rooms at 157 ff., 196-203, 274-277,
  356-359, 519). The unfed census rooms `furnished_room` and `resident_room` own none either
  (tests/census.rs:73-81, 106). Only the day census's `DAY_HOME` has one (tests/census.rs:900-911,
  917-935), and **`day_census` prints only and asserts nothing** (tests/census.rs:1298-1371).
- **So LookOut-only changes** (row, duration, script, rarity gate) move only:
  - `use_duration(Use::LookOut) == (15_000, 30_000)` (osaka.rs:12416);
  - `Want::Use(Use::LookOut).def()` factors (brain.rs:1194-1202);
  - the boost-hours lint pairing Gaze with LookOut (brain.rs:1370-1372);
  - `she_looks_out_at_the_sky_she_sees` (tests/clock.rs:787-834; it asserts the bubble at first
    show, so it is fine with more keys);
  - `her_clock_and_window_stay_cheap` (tests/clock.rs:497-589; cue-driven, so it needs
    `looked >= 1`; longer look-outs could add sky and dial images inside one act, under
    `VISIT_IMAGES`);
  - the census naming at tests/census.rs:243-244;
  - the day census numbers (19 a day, the glance counts).
  - design.md:2041-2047 ("15–30 s", "twice as likely") needs its rule rewritten, with the reason
    in decisions.md.
- **Anything touching** SpaceOut, Idle durations, the §5 credit arm, settling, nearest-seat or
  walk weighting changes her decisions in every room. That moves all the golden tables, including
  the `UNFED_*` ones (tests/golden.rs:748-765), `sofa_census` (tests.rs:8012) and `visit_census`
  (tests/census.rs:662), which are 5a-comparable. Re-record with reasons, as 5b did.
- **Wildcard-free lints** a new ScriptId, Use, Activity or Host must satisfy: script.rs:298-578,
  2150-2175 (`use_at`), the census group match (osaka.rs:5249-5283), `Surface::of`
  (script.rs:640-655), `rules::ANY_USE` exhaustiveness (rules.rs:1155-1186), and
  `Activity::ALL` and `restful` (osaka.rs:1196-1213).

## Levers and risks for 5c

| Brief item | Plug-in point | What it would break or move | Notes |
|---|---|---|---|
| Rarer look-outs | LookOut base 8 → lower (brain.rs:737); or a per-use gate (proposed: a `look_out_until` latch on `Osaka`, or `try_play(ScriptId::LookOut)` with `SCRIPT_COOLDOWN_MS` at start, but `try_play` mutates, so it can't go in a pure guard); a heavier `TOLERANCE_PER_USE` only touches Fun | The LookOut pins above; day-census numbers | A lower base hands its Daydreams draws to SpaceOut and Gaze, which helps the glance |
| Much longer look-outs | `use_duration` (osaka.rs:1142) or `use_duration_in` (osaka.rs:1153); a multi-key `looking()` with `Span::Upto` (script.rs:1721) | osaka.rs:12416; `shortest_body` lint (script.rs:540, 2254); image budget test | Chat every 90 s in the home cuts them (§8). Credit per act is fixed (§2.5) |
| Look-out as a daydream (musings, sitting) | A LookOut script with a mid-act musing key, or a session; a sitting-under-window pose | New art, model sheet first | Sky lines per key, redrawn at each key's time |
| Afternoon glance has room | Fix the §5 credit arm; let the glance roll in a daydream or look-out; once a game day | §5 blast radius; osaka.rs:12230-12253 asserts once a visit | Today its odds are about 1/9 per SpaceOut, on the clock's strip only |
| Gaze ×more with a window (D4) | A new Factor variant or `factor` argument (brain.rs:753) | brain.rs:1110 lint, 1370 test | May conflict with "window = daydream place" (Q1) |
| Daydream sessions | SpaceOut fire arm (osaka.rs:3029-3032), or a new `ScriptId` (`Host::SpaceOut`) | Every golden (decisions change); the census "spacing out" share | Needs the §5 arm; MUSINGS has 8 lines with a 10-min cooldown |
| Cloud-watching | LieBack with a non-Zzz look, or a LookOut-on-the-floor variant | `Host` lints; line-art check of LieBack plus Curious | LieBack is today's doze (Zzz, Blink) |
| Settling in | A `"settle"` continuation in `choose_next` after `leftover` (osaka.rs:5623-5636) | Every golden; Sleepy by day (night census) | Chains: SpaceOut → Sit → LieBack; Lounge → Nap. No day use of the night code |
| Nearest spot | `place()` with `pick_weighted` by distance (mind.rs:434-441); `walk` column weighting (mind.rs:253-265) | tests.rs:7078-7114 (`places`); goldens | Never as a sub-1 want factor (TOP-4 gating) |

**Risks**

- R1: Longer still acts without slower Restless just front-load the walk. Restless fills in 90 s
  (brain.rs:70), and a long act eases it by nothing, so she gets up at Restless 1.0 and Walk
  (base 14) wins. These levers are coupled with step 3's need rates.
- R2: Chat cadence (90/60/45 s) bounds stillness in the census, and every cut is a fresh decision
  (§8). The band test may be unreachable in the resident and stage without resume-after-look.
- R3: The §5 credit fix moves everything, so sequence it first (or with step 1's measurement), so
  that 5c's re-measure has a stable baseline.
- R4: A multi-key look-out script trips the `shortest_body` lint unless it uses `Span::Upto`.

**Open questions (for the user / orchestrator)**

- Q1: With the window a daydream place, should D4's "Gaze ×more with a window" be built,
  redirected (Gaze → a long look-out), or dropped?
- Q2: Should a deep daydream (a long look-out, a session, a doze) look at chat lines as today, or
  stir and carry on (like the night's `Chat::Stir`, or like the andagi's answer-and-play-on)? Or
  resume after looking?
- Q3: Afternoon glance once a visit, or once a game day (per-process carry like `budget_day`,
  osaka.rs:4019)? Roll it once per daydream?
- Q4: Should a long still act ease its need by its length (per second) rather than per act?
  Today a 2-minute look-out eases as much as a 15-second one.
- Q5: New art wanted for settling (sitting under the window, nodding off over a book or TV,
  cloud-watching on her back)? Model sheets first.
- Q6: Can a hung window be lifted and tried (`trying`, TRIAL_USE_MS)? If not, LookOut's
  `shortest_body` can become its own use length. Not verified here.
- Q7: Is the §5 gap (SpaceOut never credited) a bug to fix in 5c, given that it moves every
  golden?


---

<!-- section: rainout -->
# Code map for 5c: the rain-out's last frame, determinism, test infra

HEAD cabf5d1. Paths are relative to `dessplay/src/ui/houseguest/` unless they
start with `docs/`, `.config/`, `.claude/` or `dessplay/src/ui/shell.rs`.
"(proposed)" marks anything that doesn't exist yet. "(inferred)" marks a claim
drawn from reading the code and not checked by running it.

Brief: docs/plan.md:2997-3002 (5b "Open": "A rain-out's last frame may
linger ... Visiting's `fading` is computed after `retain`. The fix moves 24
golden traces, unfed tables included ... Away's arm has the fix.") and
docs/plan.md:3023-3025 (5c: "apply the fix ... re-recording the 24 traces it
moves with the reason").

## 1. The bug

### The mechanism: fades and the redraw decision

- Each live state holds `fades: Vec<Dissolve>`: `Visit.fades` (mod.rs:430),
  `Leaving.fades` (mod.rs:573), `Empty.fades` for Away (mod.rs:670).
- `Guest::advance(&mut self, now: u64) -> bool` (mod.rs:1464) returns
  "whether the screen could change". The shell redraws only on `true` when its
  timer fires (dessplay/src/ui/shell.rs:531-540: `redraw |= advance_guest(..)`;
  `if redraw && draw(..)`). The input arm also calls `advance_guest` and always
  draws afterwards (shell.rs:553-555, "every input is followed by a draw").
- `Guest::next_tick` (mod.rs:1765) wakes the shell at every live fade's
  `next_frame(now)`: Away at mod.rs:1780-1787, Visiting at mod.rs:1788-1796,
  Leaving at mod.rs:1798 (its own dissolve only: "Every fade began before the
  goodbye's rain, and ends first").
- dissolve.rs timings: `DURATION_MS = 3750` (:17), `SETTLE_BY_MS = 3600`
  (:19), `RAIN_FROM_MS = 675` (:21), `RAIN_LATEST_MS = 1950` (:23),
  `FRAME_MS = 60` (:25).
  - `done(now)`: `now - t0 >= DURATION_MS` (:194-196).
  - `next_frame(now)`: the next 60 ms boundary from `t0`, capped at
    `t0 + DURATION_MS` (:199-202). So the frames fall at t0+...3600, 3660,
    3720, then 3750.
  - `phase()` returns `Phase::Settled` for `t >= SETTLE_BY_MS` (:226-229), and
    `paint` puts nothing for a settled cell (:296-298, :325-329).
- Pane-focus rains are made with `t0 = now - RAIN_FROM_MS` (Visiting
  mod.rs:1987-1993, Away mod.rs:4184-4190, the door rain mod.rs:2584-2590,
  what she moved as she goes out mod.rs:1722-1728), so they skip the startled
  beat.

### The two arms

Away, already fixed (mod.rs:1480-1496; landed in commit `dadd8076`, "feat(houseguest): away at school, her closed door in the empty room", an ancestor of HEAD, whose message records the Away design and its tests):

```rust
State::Away(empty) => {
    // A rain ending this tick needs its last frame too (the
    // frame without it).
    let fading = !empty.fades.is_empty();
    empty.fades.retain(|fade| !fade.done(now));
    ...
    } else {
        fading || ticked
    }
```

Visiting, the bug (mod.rs:1498-1500, returned at :1538):

```rust
State::Visiting(visit) => {
    visit.fades.retain(|fade| !fade.done(now));
    let fading = !visit.fades.is_empty();
    ...
    changed || fading || flapped || quarter && tells_time(&visit.shown)
```

On the tick where the last fade reaches `done`, `retain` empties the vector
first, so `fading` is false. If nothing else changed on that tick
(`Osaka::tick` false, no flap, no quarter-hour with a clock or window shown,
no nudge), `advance` returns false and the frame without the rain is never
drawn.

### When can anyone actually see it?

The visible symptom is narrower than "the last frame lingers":

- A shell that honours `next_tick` paints at t0+3600, 3660 and 3720. Every
  cell is `Settled` from 3600 on (dissolve.rs:227), so those frames are already
  the real UI. The redraw skipped at 3750 is a visual no-op.
- A rain frame stays on screen only if no paint lands in
  `[t0+3600, t0+3750)`: a timer tick 150 ms or more late (a loaded machine, a
  stall) and no input in that window (input always redraws,
  shell.rs:550-555). It's rare and cosmetic, but it breaks a real invariant:
  `advance` is meant to say whether the screen could change, and here it can.
- This is why no test caught it. `what_she_moved_rains_out_as_she_goes`
  (tests/away.rs:245-292), the Away arm's test, ticks on `next_tick`
  (:271-274). It sees a clean last frame with or without the Away fix (the
  3600/3660/3720 frames are clean), so that fix very likely survives the
  mutant (inferred).

### The fix

Swap two lines in the Visiting arm, computing `fading` before `retain`, as in
Away. A better fix makes the bug unrepresentable (proposed): one helper that
all three arms share.

```rust
/// Drop the fades done at `now`. Whether the screen could change: any was
/// running (its frame, or the frame without it).
fn tend_fades(fades: &mut Vec<Dissolve>, now: u64) -> bool {
    let any = !fades.is_empty();
    fades.retain(|fade| !fade.done(now));
    any
}
```

The Away arm (mod.rs:1484-1485), the Visiting arm (:1499-1500) and Leaving
(:1541, whose result can be ignored since Leaving always returns `true`)
would all call it. Then no arm can get the order wrong.

### Why 24 traces move although nothing visible changes

- `drive` (tests/golden.rs:119-149) steps exactly like the shell: `step =
  next_tick.clamp(1, 1000)`, and paints only
  `if guest.advance(now) || input || cue.is_some()` (:144-147). With the fix,
  every Visiting fade that ends on a quiet tick adds one painted frame at
  t0+3750.
- `Trace::frame` (golden.rs:43-84) hashes one line per painted frame
  (`"{now} {her} {cells:016x}"`), changed cells or not. One extra line moves
  the run's FNV hash.
- Paint also has side effects. The Visiting draw calls `furnish(..., &mut
  self.rng)` (mod.rs:2009-2020), `record`, `tend_made` and `osaka.shown`, so a
  trajectory can drift after the extra paint, not just gain a line.
- **Which 24 (inferred):** the only Visiting fades the golden scenes make are
  pane-focus rains (mod.rs:1975-1994) and an errand's door rain
  (mod.rs:2544-2552, only from Away/Absent). `gate()` (mod.rs:2598-2608) sets
  `view.focus = None` unless the client is in use and she isn't on an errand.
  - `resident` (golden.rs:194): focus cycles (:244-249) with key presses.
    4 seeds × 2 modes × fed and unfed = **16**.
  - `errand` (golden.rs:303): odd seeds are residents with
    `focus: resident.then_some(chat)` (:320) and `busy: Playing`. Focus counts
    once the errand is done. Seeds 1 and 3 × 2 modes × fed and unfed = **8**.
  - That makes 24. It also fits `UNFED_ERRAND` (golden.rs:740-745): seeds 0-2
    are identical to the fed table (:609-612) and seed 3 differs, so the
    errand seeds already depend on behaviour.
  - To confirm: run the golden tests with `HOUSEGUEST_GOLDEN_TRACE=<dir>` on
    HEAD and on the fix, then diff. Every moved trace should first differ at a
    frame `t0 + 3750` after a focus rain began.

### Sibling sites (the "fix the class" list)

| Site | Same class? | Why |
|---|---|---|
| Away arm, mod.rs:1484-1485 | fixed | `fading` read before `retain` |
| Visiting arm, mod.rs:1499-1500 | **the bug** | read after `retain` |
| Leaving arm, mod.rs:1540-1547 | immune | returns `true` every tick; its dissolve `done` → `Absent`, also `true` |
| `flap`, mod.rs:1501-1504 | immune | `take_if(.. now >= since + FLAP_MS).is_some()` reports on the expiring tick; pinned by tests.rs:4686 ("the flap shuts") |
| `nudge.advance`, nudge.rs:121-127 | immune | `shaking` captured before it's cleared |
| `Osaka::tick`, osaka.rs:2664-2702 | immune | `changed = true` for every due event except the night's own (`cut`, `dream`, `sleep_talk`, `midnight_snack`, each returning its own bool) |
| Draw-side retains, mod.rs:1996-1998, :4193-4195 | immune | inside `paint`, so they make no redraw decision; a resize (`fade.size() != size`) redraws anyway |
| `quarter` tick, mod.rs:1477, :1487, :1538 | immune | edge-triggered by `routine::quarter_crossed` |
| `gone_out` → `out_by_door`, mod.rs:1550-1553 | immune | sets `changed = true`; `out_by_door` carries `visit.fades` into `Empty.fades` (mod.rs:1720-1731) |
| Visiting/Away → Leaving in `leave`, mod.rs:1182-1245 | immune | `fades` carried into `Leaving.fades` |

A different class, not in the brief, worth one line: **a rain cut short.**
Away → `Arriving(How::Dash)` (mod.rs:1490-1493) and `school_out` →
`Arriving(How::Return)` (mod.rs:1605-1612) replace the state and drop
`empty.fades`. `begin_visit` starts with `fades: Vec::new()` (mod.rs:2428),
and the errand path pushes only the door rain (mod.rs:2544-2552). The rain of
what she moved snaps back if she comes in within 3.75 s of going out (22.5
game seconds at 6×). It's cosmetic and rare (inferred, not seen).

### Tests that pin `advance`'s boolean (the no-busy-loop invariants)

tests.rs:116-117 (`!advance(4_999)`, `advance(5_000)`), tests.rs:137, :2486
("time alone changes nothing"), :2488, :2490, :2512, tests.rs:4686,
tests/away.rs:430 ("redrawn for nothing"). None involve fades, so the fix
shouldn't trip them (inferred).

### Regression test (proposed)

A test driven by `next_tick` can't fail before the fix, because the frames
from 3600 on are clean. The honest-interface test is a **late shell**:

1. A resident visiting, a pane focused while in use, so a Visiting fade
   starts at `t0`.
2. Step and paint on `next_tick` until some `t < t0+3600` with rain cells
   still painting (`Dissolve::painting`, test-only, dissolve.rs:212).
3. Jump to `t0+3750` or later in one `advance`, and paint only if it returns
   `true`.
4. Assert that the last painted frame equals `real` in the focused pane, or
   simply that `advance` returned `true`.

It needs Osaka quiet at that instant (a long act, no due event), or her own
`changed` masks the bug. Choose the seed by search, or assert only on the
fade-only part.

The class-level property (proposed): "after any `advance` that returns false,
a fresh paint equals the last painted frame". It needs a shadow paint, but
`Guest` isn't `Clone` (mod.rs:692) and `paint` mutates, so it's costly.
`tend_fades` makes the class structural instead.

## 2. Determinism: what moves when brain constants or act durations change

### Golden trajectories (tests/golden.rs)

- Module doc (golden.rs:1-7): "A refactor that means to change nothing she does
  keeps every hash (re-record them when behaviour is meant to change)".
- The hash is FNV-1a 64 (golden.rs:11-25). Per frame: `act_name`, x, y, facing,
  `appearance(now)`, `image.figure.her()` and the hash of every cell that
  differs from the real frame (a kitty image's cell reads as `"image"`)
  (:43-84). Then `ledger.to_json()` at the end (:88-90).
- **Fed tables:** 10 scenes × seeds 0-3 × (ASCII, line art) = **80 hashes**.
  `golden_stage_room` :563, `golden_resident` :577, `golden_furnished_home`
  :591, `golden_errand` :605, `golden_homework_evening` :619,
  `golden_tucked_in` :633, `golden_weekend` :647, `golden_school_morning`
  :661, `golden_home_from_school` :675, `golden_dash_home` :689.
- **Unfed tables:** `UNFED_STAGE` :710, `UNFED_RESIDENT` :720,
  `UNFED_FURNISHED` :730, `UNFED_ERRAND` :740 = **32 hashes**, checked at
  :748-765 through `Guest::unfed()` (`guest_of`, :339-342).
  - Each table's doc comment lists every deliberate move since 5b step 3
    (:702-738).
  - `check_unfed` (:543-548): "No step may move them without a trace diff in
    its commit."
- Scene lengths: stage 300 s (:176), resident 180 s (:237), furnished 600 s
  (:285), errand 150 s (:314), evening (:387) and weekend (:439) 600 s, tucked 300 s (:413), the
  school, home and dash scenes 180 s (`three_minutes_from`, :460;
  `dash_home` :525). Any change to brain weights, need fill rates, act
  durations or rng draw order moves effectively every fed hash and every
  unfed one (they all run the brain).

### Re-recording (there's no script)

1. A failing `check_fed` (golden.rs:550-561) prints the replacement table:
   `"{name}: her trajectories changed; if that's meant, re-record:"`, then
   `(seed, 0x..., 0x...),` rows to paste.
2. Before pasting: set `HOUSEGUEST_GOLDEN_TRACE=<existing dir>` (it fails if
   the dir is missing: `std::fs::write(..).unwrap()`, golden.rs:97) on the old
   revision and on the new one. Each run writes `<scene>-<seed>-<graphics>[-unfed].txt`
   (:91-98; names at :188, :258, :297, ...): one line per frame, then her
   ledger JSON.
3. Diff them. The commit says **where the traces first differ and why, and
   that the rest are byte-identical** (docs/testing-strategy.md:796-806).
4. For the UNFED tables, append the move to that table's doc comment
   (golden.rs:702-738), as for "step 5a's hidden-goodbye fix ... its trace diff
   is in that commit".
5. Precedent: decisions.md:3081 ("That changed the trajectories, and was
   re-recorded on its own, for that reason alone"), and plan.md:2213-2219
   (behaviour-preserving commits keep the hashes; each behaviour-changing
   commit re-records "from the table the failure prints, with the reason in its
   message").

### Other pinned outputs

- **insta snapshot** `osaka_at_home_seed_7` (tests.rs:823-840): `Guest::new(7)`,
  fed by default, 95 s into a visit, over the real default layout. File
  `snapshots/dessplay__ui__houseguest__tests__osaka_at_home_seed_7.snap`. Brain
  changes will move it. Re-accept with `cargo insta review` only with a reason
  in the commit (testing-strategy.md:804-806). The three `default_layout_terrain_*`
  snapshots are terrain only and unaffected.
- **Gated census tests that pin seeds** (tests/census.rs; they run in the
  gate, `SEEDS = 16` at :24):
  - `an_uninterrupted_trip_runs_its_course` (:1377-1395): furnished room, no
    chat, seeds 0-5, 10 min. `assert!(set_off >= 8)`, and no
    `"let go: Other"`.
  - `at_home_her_furniture_beats_the_floor` (:1434-1443): seeds 0-3, Ordinary.
    `furniture > 10 * floor rest`.
  - `her_mood_shows` (:1450-1474): resident room, seeds 0-3, 15 min. Lazy
    furniture > Industrious; Lazy exercise < Industrious; Dreamy spacing out >
    Ordinary. Its doc says "(Time spent moving about is no measure: in this
    room it's within a few percent either way, seed set to seed set.)"
  - `the_census_counts_her_vignettes` (:1636-1700): seeds 0-2, 15 min,
    `assert_eq!(shopping, 1)` per seed, plus splice counts.
  - `every_pooled_line_shown_was_drawn_from_its_pool` (:1486).
- **Printed only (ignored), compared by hand against plan.md's records:**
  - `sofa_census` (tests.rs:8010-8013, 100 seeds, unfed; 5b "matches 5a
    exactly")
  - `visit_census` (census.rs:660-662, 30 min, unfed)
  - `image_census` (census.rs:788-790; asserts `counts.evicted == 0` at :808)
  - `day_census` (census.rs:1296-1298, fed week, seeds 0-2: `cargo test
    --release -p dessplay --lib day_census -- --ignored --nocapture`, about
    65 s)
  - `reach_census` (tests.rs:1674-1676)
- **Fixed-seed scenario tests:** many away, dash and clock tests use fixed
  seeds and time bounds, e.g. `home_at(4, tue(8, 14), ..)` with
  `assert!(now < 60_000, "never out")` (tests/away.rs:252, :263). Expect some
  to need a new seed or bound when movement is retuned. Treat them as a class
  and fix each by search, not by hand-picking a seed.
- **Census grouping:** `Osaka::census_group` (osaka.rs:5246-5287, test-only,
  wildcard-free, so "a new act doesn't compile until it's put in one").
  "moving" = `Walk | Climb | Clamber | Out | Away | Door | Fall | Peer | Dazed`
  (:5260-5268). `LookOut` counts as "spacing out" (:5252).
  - The census sums `step` per `doing()` string per group (census.rs:367-373).
    Each step is `next_tick.clamp(1, 1000)`, so time is attributed to the act
    at the start of the step.

### The brain constants the brief names (for scale)

- Need fill: `Self::Restless => 90_000.0` (brain.rs:70).
- Mood multipliers on Restless: Lazy 0.4, Industrious 1.5, Dreamy 0.7
  (brain.rs:193-202).
- Rows: `row(4.0, &[(Need::Restless, 0.5)])` (:676), Jacks and ToeTouch
  `row(6.0, ..)` (:685).

Each of these moves all 112 golden hashes, the seed-7 snapshot, and possibly
the gated census pins.

## 3. Test infrastructure

- **`proptest_cases(N)`** (dessplay-core/src/test_support.rs:43-48):
  `PROPTEST_CASES` env wins, else N. Only for `with_cases`.
  `ProptestConfig::default()` reads the env itself (testing-strategy.md:316-324).
  - The houseguest has 21 sites, all wrapped. Pinned counts: 256 ×1
    (room.rs:2032), 128 ×3 (layer.rs:774, rules.rs:1264, room.rs:2457), 64 ×6
    (rules.rs:2121, graphics.rs:773, dissolve.rs:410, osaka.rs:8159, :9951,
    :10796), 24 ×2 (tests.rs:425, :7211), 16 ×3 (tests.rs:6299,
    tests/away.rs:1224, tests/dash.rs:1209), 12 ×1 (tests.rs:4703), 8 ×5
    (tests.rs:6694, :7443, :10915, :10986, :11226).
  - Never a bare `with_cases(N)`.
- **The gate:** the stop hook runs `cargo nextest run --no-fail-fast` with
  `PROPTEST_CASES` defaulting to 32 (.claude/hooks/stop-checks.py:97-105). It
  stands down while subagents are live (memory note).
- **The 256-case pass:** `PROPTEST_CASES=256`, release, houseguest tests (5b:
  557 tests, passed three times; docs/plan.md:2958-2960). No exact command is
  checked in. The memory note says: run in release, and read nextest's own
  exit code (`cmd > log; echo $?`) rather than piping through `tail`.
- **Slow tests:** `.config/nextest.toml`:
  - default `slow-timeout = { period = "30s", terminate-after = 2 }` (SLOW at
    30 s, killed at 60 s); `default-filter = "not binary(perf)"`.
  - One override: `filter = 'test(every_made_piece_is_used_or_let_go)'`,
    `period = "60s"` (killed at 120 s), "~50 s at the deep pass's 256".
    That test is at tests.rs:7452 (8 cases pinned, :7443).
  - 5b recorded it at 45-47 s and "four tests at 29-39 s" (plan.md:2960),
    unnamed in the record. They're close to the 60 s kill at 256 cases on a
    loaded machine, and none has an override.
  - `profile.full`: `default-filter = "all()"`, 60 s period.
  - The convention (testing-strategy.md:303-308) is that a test legitimately
    slower at the deep pass gets a named override with its reason.
- **Sims:** censuses use `simulate`/`simulate_with`/`visit_from`
  (census.rs:325-420), a shell-like loop with `next_tick.clamp(1, 1000)`. The
  memory note asks to keep a new sim off the gate's long pole by shortening
  its simulated span, not by pinning the case count.
- **Shell-like helpers in tests:** `shell_step(guest, real, view, &mut now,
  always)` (tests.rs:3576-3584) paints if `advance` or `always`;
  `until_visiting` (:3588). A late-shell regression test would add a sibling
  (proposed) that takes an explicit jump.

## Levers and risks for 5c

**Order.** Make the rain-out fix the **first commit of 5c, on its own**,
before step 1's measuring. Then every census baseline and the band test
include it, and its 24-trace diff isn't mixed up with brain changes. The
commit:
- swaps the lines, or adds `tend_fades` (proposed);
- re-records `golden_resident`, `golden_errand`, `UNFED_RESIDENT` and
  `UNFED_ERRAND`;
- appends to the two UNFED doc comments;
- says in its message that traces first differ at a `t0+3750` frame after a
  focus rain, the rest byte-identical;
- updates plan.md 5b "Open". No CHANGELOG entry needed (nothing visible in the
  common case; arguably "a rare leftover rain frame" is a fix a user might
  notice, so the main thread decides).

**Brief items and where they land:**

| Brief item | Plugs in at | Breaks |
|---|---|---|
| Step 1: split "moving" by purpose | `census_group` (osaka.rs:5246) is a single `&'static str`. A purpose split needs `Act::Walk`'s purpose (wander, to text, to seat), maybe a new `census_purpose()` (proposed), or `doing()` (census.rs:205) | nothing pinned; printed tables change shape |
| Step 2: band test per room × mood | new gated test in tests/census.rs (proposed) | `her_mood_shows`'s premise (census.rs:1446-1449) says moving share differs by only a few percent by mood in the resident room. Step 1 must show the signal exists before step 2 pins it |
| Step 3: slower needs (brain.rs:70), lower Walk base, longer still acts | brain.rs rows and durations | all 112 golden hashes, seed-7 snapshot; may break `an_uninterrupted_trip_runs_its_course` (`set_off >= 8`) with fewer trips; `the_census_counts_her_vignettes` (shopping == 1 per seed) |
| Step 3: settling in, nearest spot | brain and osaka choice | as above; also fixed-seed scenario bounds (`now < 60_000`) |
| Step 4: floor homework, makeshift desk | new acts → `census_group` must place them | `at_home_her_furniture_beats_the_floor` (floor rest is its denominator, census.rs:1442) if floor homework counts as "floor rest"; new made-piece kinds also run through `every_made_piece_is_used_or_let_go` (already 45-47 s at 256 cases, 120 s kill) |
| Step 5: rarer, longer look-outs | `Use::LookOut` | already "spacing out" in the census (osaka.rs:5252); day_census look-out counts (about 19 a game day) and the afternoon glance |

**Re-record budget.** One re-record per behaviour-changing commit, each with
its trace diff and reason (plan.md:2213-2219), not one at the end of the
phase. The unfed tables are 5a-comparable baselines. 5c moves them on
purpose, so each UNFED doc comment gets a 5c line, and the printed censuses
(`sofa_census`, `visit_census`) stop being comparable with 5a once the brain
changes. Record that in plan.md.

**Risks:**
- Longer still acts mean fewer painted frames, so a sim covers fewer
  decisions per real minute. Censuses may need longer spans for the same
  statistical power, which works against the gate's long pole.
- The deep pass's 29-39 s tests have no overrides. Longer acts in sims that
  run until something happens (`while ... { assert!(now < N) }`) could push
  them past 60 s at 256 cases, or past their `now < N` bounds.
- `tend_fades` touches the Leaving arm too. Leaving returns `true`
  regardless, so it's behaviour-neutral there (inferred).

**Open questions:**
- Fix the rain cut short too (Away → Arriving dropping `empty.fades`,
  mod.rs:1492, :1611)? It's the same "fade lifetime vs state transition"
  family. Carrying the fades into `begin_visit` would move the dash and home
  traces as well.
- Is the late-shell regression test worth writing, given the visible window
  is about 150 ms of timer lateness? Or is `tend_fades` (structural) enough
  on its own, per CLAUDE.md's "unrepresentable" rule?
- Should the band test run on the unfed census (5a-comparable rooms) or the
  fed day census (where the brief measured 35-45%), or both? The brief says
  both for measuring (step 1). The pinned test needs one fast enough for the
  gate.
- Does the fix's commit get a CHANGELOG line? The user-visible effect is near
  zero.


---

<!-- section: critic -->
# Gaps and corrections

The completeness critic for the six 5c code maps (brain, acts, census, makeshift, rainout,
window). **Where this section disagrees with another, this one wins.** HEAD cabf5d1. Paths are
relative to `dessplay/src/ui/houseguest/` unless they start with `docs/`. Tags:
- **(proposed)**: not in the code.
- **(inferred)**: read from the code, not run.
- **(run)**: taken from the census logs a reader made at HEAD, in the scratchpad
  (`visit_census_moods.log`, `day_census.log`). I read those logs; I didn't re-run them.

I checked 34 claims against the code. The ones a 5c design rests on are in §4; the rest held.

---

## 1. Wrong claims (with the right answer)

**C1. brain.md, "after a walk … she still moves 53% of rolls" is wrong.**
The table leaves out the cooldown on the Walk she just chose. `score` multiplies by
`COOLDOWN.powi(repeats)`, with `COOLDOWN = 0.4` (brain.rs:786, 815-816), and `recent` holds that
Walk (osaka.rs:5696-5702).
- Recomputed with brain.md's own levels (R 0.3, D 0.4, C 0.5, bare room), Walk drops from
  2.41 to 0.96 and falls out of the top four.
- The top four become Stand 2.00, SpaceOut 1.75, Travel 1.72, Gaze 1.56. Movement is Travel
  alone, about **25%**.
- The arrival tables in brain.md are right (I checked Walk 6.89, Pull 6.40, Travel 4.92,
  Watch 4.00, LookOut 4.00, Read 3.60, Jacks 3.54). The steady-state picture is not.
- So walking comes in waves: a walk, then something still while Walk cools down, then
  another walk.

**C2. brain.md's tables assume Tidy 0.5. On the stage, Tidy sits at 1.0 and Pull is what moves
her.**
- (run) In stage seed 0, tidy reads 1.0 at 5, 10, 15, 20 and 25 min (ordinary, and lazy too).
- Stage choices (ordinary): Pull 28.7%, Walk 13.8%, Swap 13.4%, Travel 11.0%. Walks to text
  outnumber wanders.
- At Tidy 1.0, Pull scores 16 × (0.1 + 1.0 × 0.6 × 2) = **20.8**, three times Walk's 6.89 at
  arrival.
- Why Tidy stays at 1.0:
  - It rises in 60 s while there's text to pull (brain.rs:71, `Rising.mess` osaka.rs:5517).
  - A pull is credited only by `offset/goal` (osaka.rs:5000-5002).
  - Many pulls are dropped before she gets there: on the stage, 129 "let go: Other" against
    731 set off.
- **The home and resident logs show no Pull or Swap choices at all.** The resident's text is
  in the chat (Pull and Swap carry `in_chat` ×0.1, brain.rs:692-694), and the census home
  offers none.
- So the brief's "Tidy toward about 4 min" matters on stage-like rooms only. Restless and the
  use walks are what matter for the resident view the user actually runs.

**C3. brain.md, "the mood spread in movement comes through `Mood::rate` for free", is
contradicted by census.md's own measured table.**
- Lazy Restless ×0.4 against industrious ×1.5 (brain.rs:192, 196) is a 3.75× ratio. Today it
  buys only 3–7 points of "moving" (run):

  | Room | Lazy | Industrious |
  |---|---|---|
  | stage | 34.1 | 39.5 |
  | home | 40.4 | 47.3 |
  | resident | 37.3 | 40.4 |

- The brief's band (about 15 lazy, up to 30 industrious) needs about a 2× spread.
- The mood mostly moves time between standing or exercise and furniture, and every furniture
  use begins with a walk. `her_mood_shows`'s doc says the same (census.rs:1445-1449).
- Expect step 3 to need a mood lever beyond need rates: mood-scaled still-act lengths, a
  settle chance by mood, or a mood term on Walk/Travel (all proposed). Raise this with the
  user before step 2 pins thresholds.

**C4. makeshift.md, "3× on make one plugs into `place()`": it does nothing in the room it's
meant for.**
- With no real piece of the kind on offer, `places()` (mind.rs:382-431) returns only
  `Place::Make(item)` for that use (Lounge, Nap, Sleep), or the made seat once it stands.
- Re-weighting a list of one changes nothing. It only matters where real seats of another
  kind share the use: Nap is on the sofa only, Sleep on the bed only (room.rs:170, 198).
- The live lever is brain.md's: a multiplier above 1 on the **want** when its bind is
  `use/make`.
- The `factor` closure (osaka.rs:5669-5682) already looks up each offer's bind through
  `offers`. A test on `Bind::Job(Job::Build(_))` there needs no new `Factor` variant, no
  `sane_factor` change, and no access to the private `Bind::spot`.
- It is above 1, so it can't push a want out of the top four (brain.rs:547-549). It can still
  push other wants out.

**C5. A chat line's Look lasts 4.0 s in all, not 5.2 s** (acts.md §5, window.md §8 and
summary 5).
- `interrupt` sets `surprised_until: now + SURPRISED_MS` and `until: now + LOOK_MS`
  (osaka.rs:6507-6522), with `LOOK_MS = 4000` (osaka.rs:369). The first 1.2 s are the start.
- Then she stands in chunks of up to 5 s until `watch_until`, which is 15 s after the last line
  (`WATCH_MS`, osaka.rs:371, 5495-5503).

**C6. Golden counts.** census.md's "all 14 golden scenes × 4 seeds × 2 modes" counts tables as
scenes.
- There are 10 fed scenes (golden.rs:562-700) and 4 `UNFED_*` tables (golden.rs:710-765), each
  4 seeds × 2 modes.
- That is **112 hashes** (rainout.md's 80 + 32 is right), plus the `osaka_at_home_seed_7`
  snapshot.

**C7. The name `settle` is taken.** `Osaka::settle(&mut self, now, terrain) -> bool` already
re-anchors her on a fresh terrain (osaka.rs:6528). acts.md and window.md both propose a
"settle" continuation.
- The `Decision` method label is only a string, but a `fn settle` would clash.
- Pick another name, e.g. `settle_in` / "settle in" (proposed).
- `self.rest` (osaka.rs:1375) is the spot she rests on, used by `recheck`. It has nothing to
  do with settling in.

**C8. window.md: "each SpaceOut decision has at most about 1/9 chance" of the hour glance.**
That's an overstatement in the other direction.
- The 1/3 `muse` whim is drawn when SpaceOut is *bound* (mind.rs:223-225), not after it wins.
- Then Setsubun, a cued glance or the escalator can take the muse first (osaka.rs:2154-2198).
- So 1/9 per *chosen* SpaceOut is an upper bound, and only on the clock's strip.

**C9. acts.md §4: "on a 100-column floor that's ~33 cells ≈ 11 s".** The arithmetic is fine.
census.md has the measured form: in the home, the Walk act is 33.3% of visit time while
Walk + Travel are 27% of choices (run). Most of the walking there is **to uses**, not
wandering. Cite that, not the estimate.

---

## 2. Contradictions between sections

| Topic | Sections | Resolution |
|---|---|---|
| Where "3× make one" plugs in | makeshift: `place()`; brain: a want factor | The want factor in the `factor` closure (C4) |
| Mood spread from need rates | brain: "for free"; census, rainout, acts: barely a few points | The measurement wins (C3) |
| Look length | acts and window: 1.2 + 4 s; code: 4 s | 4 s (C5) |
| Golden count | census: 14 scenes; brain: 10 + 4 tables; rainout: 112 | 10 fed + 4 unfed tables = 112 (C6) |
| SpaceOut credit gap: fix before step 1? | acts and window: yes; brain: not mentioned | Fix it as its own commit before step 1's baseline (see §3, G1) |
| The band's quantity | census: in-sight moving ÷ in-sight time, unfed; brain: `census_group` "moving" as is | census is right: hidden Away and Door time must come out (census.rs:371-374 has no hidden filter; osaka.rs:6650-6659) |
| "35–45% in every slot" (brief) | census: week 35/40/37; home Homework slot 28–29 | The day-census log agrees with census (run). A per-slot band would fail by design |

---

## 3. Gaps: what a 5c designer needs that no section covers

**G1. Order of the pre-commits.** Three fixes each move all 112 hashes or reshape the tables.
Each should land alone, before step 1 measures anything:
1. The rain-out fix (24 traces, inferred split).
2. The **SpaceOut credit arm** (confirmed: `credit_done` falls to `_ => return` at
   osaka.rs:5003; design.md:1459-1460 says spacing out answers daydreams).
3. The **Work credit**, which has the same gap: `Want::Work` serves Restless 0.5
   (brain.rs:697-700) and nothing ever serves it.

Fixing the class here means asking which chosen wants with non-empty `serves` can reach
`set` without a `credit_done` arm. Those are SpaceOut (including Muse) and Work.
- Stand and Sneeze serve nothing.
- Walk and Travel are paid as she sets off (osaka.rs:5705-5710).
- Swap and Pull use `credit_whole` (osaka.rs:3057, 4934).

A test over `Want::ALL` that every want with non-empty `serves` is credited (proposed)
would make this hard to reintroduce.

**G2. The chat cadence is half the denominator.**
- (run) Standing takes 36% of stage time, 24% of the resident's and 16% of the home's. Nearly
  all of it is the chat-watch reflex: Look, then Stand until 15 s after the line
  (osaka.rs:5495-5503), at the census's strict 45 s, 60 s and 90 s cadences
  (census.rs:54, 82, 107).
- The band therefore depends on the census's chat as much as on her. A quiet room reads much
  higher. `an_uninterrupted_trip_runs_its_course` runs the home with no chat.
- The same cadence cuts every long still act (no resuming, acts §5). On the stage, any still
  act longer than about 45 s is always cut.
- The user's real chat cadence is unknown (the IRC bridge joins #dess: memory note).
- Decide whether the band is pinned at a fixed cadence, with no chat, or both. Consider
  whether a cut still act resumes after the watch (proposed). Without that, step 3's longer
  acts can't show up in the stage or resident numbers.

**G3. Every movement census runs in ASCII.**
- `simulate` passes `graphics = false` (census.rs:223-225, 330), and `day_guest` calls
  `arrive_in(.., false, ..)` (census.rs:948).
- In line art, "restful" means the image she'd be drawn in is clear of text (`seats_of`
  doc, mod.rs:3405-3413; `seat_spot` mod.rs:3420). So seats, rest spots and offers differ.
- The user's client is kitty, i.e. line art (memory note).
- Measure both modes once in step 1 before pinning a band to one of them.

**G4. A fed week's rooms don't stay fixed.** Shopping furnishes them as the week goes on (run):
- The day-census **resident** (owning Sofa and Tv) shows "chopsticks" 61 times. That splice
  wraps only `Use::Homework` (script.rs:727-735), which only a desk offers (room.rs:213). So
  she bought a desk.
- The "bare" stage shows 202 surfs and 12 shoppings.

So "bare-room stillness" (step 4) can't be read off a fed week past its first day. Use the
unfed stage, or a fed run with shopping off (proposed).

**G5. Unfed is not fed, in three ways that move the band.**
1. Unfed has no clock rate, so Sleepy fills in 15 min instead of about 50 min by day
   (`SLEEPY_BY_DAY` 0.3, brain.rs:141, 114-121). (run) In the unfed home, `Use(Sleep)` is 13% of
   choices and 11.9% of time.
2. Unfed visits last 30 min from arrival levels (Restless 0.7, brain.rs:87-95). Fed visits
   run for hours at steady state.
3. Unfed moods are drawn per visit. Fed moods are per game day (C10 below).

A band pinned unfed (census.md's recommendation, for forcing the mood) needs a fed check
alongside it. census.md §6 lists the options: bucket by `osaka.mood()`, or a test-only
override (proposed).

**G6. The time-share metric can be gamed.** Two things would meet the band without making her
calmer:
- `WALK_MS = 333` (osaka.rs:358). Walking faster lowers the time share; that's the wrong
  direction for attention.
- Shorter walks (nearest spot) lower the share without lowering the number of times she sets
  off.

Attention is caught by **onsets**: setting off, a door opening, a climb. Step 1 should also
print **walk, travel and door starts per awake minute in sight** (proposed) beside the share,
so a lever that only shortens walks shows as such.

Likewise, animation counts as "still" in the census but reads as motion: `Activity::period`
is 500 ms for LieFront (kicking her feet) and 450 ms for Jacks (osaka.rs:1229-1237), with
bubbles. The band doesn't see this; flag it to the user.

**G7. On the stage, Travel is doors, and job hops are most of the motion.**
- `door_away` binds only on a floor with no links (mind.rs:279-283). (run) Door is 10.1% of
  stage time, and "Handy, these doors." is said 21 times in 16 visits.
- There were 731 heading set-offs in 16 visits: jobs on other floors (`go_to`,
  osaka.rs:5885-5923).
- `pull` and `swap` choose uniformly over every offer on every floor (mind.rs:318-340), only
  ×0.1 for the chat.

"Nearest, same floor first" for pull and swap is likely the biggest stage lever. brain.md
leaves it as open question 2, but the data answers it. A same-floor preference also cuts doors,
climbs and falls, and the door's hidden beats.

**G8. Changing weights moves the draw itself.** `pick_weighted` switches from `below(n)` to
`below(1_000_000)` as soon as any weight isn't 1 (osaka.rs:296-301). A distance weight
therefore moves goldens even where it picks the same thing. Budget a re-record per lever, not
per visible change.

**G9. The made-piece property's 90 s bound is tuned to today's act lengths.**
- `every_made_piece_is_used_or_let_go` asserts that nothing waits more than `BOUND = 90_000`
  without progress. Its comment reads "a chat gap, the watch after it, and a walk"
  (tests.rs:7460).
- `bed_at` cues a made bed at any time up to 90 s (tests.rs:7456, 7515), which can fall while
  she's lounging on the made sofa.
- Longer Lounge or Nap (step 3), or a settle-in chain that isn't strictly after `leftover`,
  can break the bound.
- It is already the slowest test (120 s override, .config/nextest.toml). Raising the bound
  needs a reason that scales with the new act lengths.

**G10. How moods are spread, as the user sees them.**
- Fed, the mood comes from `Mood::of(day_seed(master, day))` at `begin_day` (osaka.rs:4002) and
  at each arrival (mod.rs:2349-2356).
- At `CLOCK_SPEED = 6` (mod.rs:795) a game day is **4 real hours**, about 2 of them awake and
  here (day census: here 47–49%). So a session is usually **all one mood**.
- Shares are 50/20/20/10 (brain.rs:176-186). At the brief's targets the expected share is
  0.5×22.5 + 0.2×15 + 0.2×30 + 0.1×22.5 = **22.5%**.
- A lazy session (15%) is the one the user will notice. An industrious one (30%) is still
  below today's 35–40.

**G11. Mapping the day census onto the band.**
- `live_week` credits groups only for `Where::Present | Dash` in Visiting (census.rs:1176-1178),
  so its "moving NN" is in-sight, awake, in-visit time. That is the band's quantity.
- But it has four problems:
  1. It has no mood dimension (census.md correction 3).
  2. It prints the top four groups as integers only (census.rs:1048-1072).
  3. It mixes in dash-ins (49–87% moving, but under 1% of time).
  4. Its rooms gain furniture (G4).
- The Asleep and Away slot lines (59–87% moving) come from under 0.5% of their time. Exclude
  them or weight them by time.

**G12. What re-running the censuses costs** (partly unmeasured).
- (run) Release: `visit_census` with `CENSUS_MOODS` takes 85 s, and `day_census` 61 s (stage
  29, home 18, resident 13).
- Neither section gives the **release build** time for the test binary, which comes on top
  for every behaviour-changing commit.
- A golden re-record needs traces from **both** revisions (`HOUSEGUEST_GOLDEN_TRACE`,
  golden.rs:91-98). That means two builds, or a worktree for the old one; worktree agents
  commit with git (memory note).
- With G1's three pre-commits, plus one re-record per step-3 lever, expect about 6–10
  re-record cycles in the phase.
- Debug band tests cost about 15 ms per sim-minute (census.md §7, run).

**G13. What poses exist, stated plainly.** `sprite::ALL: [Pose; 47]` (sprite.rs:380-433):
- Floor-seated: `Sit` (hugging her knees), `Read(0|1)` (floor, knees up, a book: art.rs:548).
- Lying: `LieBack(0|1)` (Zzz, blink), `LieFront(0|1)`.
- Standing: `Gaze`.
- Furniture: `Lounge`, `Nap`, `Sleep`, `Homework(0..3)` (on a stool).

There is **no cross-legged pose, no seated gaze, no sitting doze, and no floor-writing pose**.
Four brief items need new art: TV cross-legged, floor homework, homework at the cube, and a
long window daydream (sitting under the sill or leaning).

The pulled-line read can reuse `Read` with a new held part (makeshift.md §10 is right there).
Cloud-watching can reuse `LieBack` with a new face and bubble. In ASCII, `LIE_BACK` has no
face cell (sprite.rs:126-129), so it differs from the doze only by its bubble there.

**G14. Settling in by day collides with bedtime.**
- Dozing serves Sleepy: LieBack 0.15, Sit 0.1, Nap 0.1 (brain.rs:681-684, 719).
- More day dozes mean less Sleepy by evening. With `SLEEPY_AT_NIGHT` 3.0 (brain.rs:143) that
  may be harmless, but the 5b night numbers (bed 22:00, "bedtime" counts) should be
  re-checked after settle-in lands.
- window.md §7c says not to reuse the night code by day. That holds, since `night_in` arms
  the night.

**G15. A design-rule hole: "nothing is ever ruled out".** design.md:1497-1499 says needs only
weight choices.
- A settle-in continuation, like `leftover` and `heading`, skips the roll.
- That needs a sentence in design.md (rule) and in decisions.md (why): settling is a
  continuation of what she chose, not a new choice.
- The same applies to resuming after a chat line (G2).

**G16. LookOut has no rarity gate, and a lower base hands its draws elsewhere.**
- window.md is right that `try_play` mutates, so it can't sit in a pure guard
  (mind.rs:1130-1139).
- No section notes this: lowering LookOut's base 8 with Daydreams now actually served by
  SpaceOut (G1) changes who wins Daydreams. Daydreams fills in 10 min (×2.5 dreamy), and
  SpaceOut, Gaze, LieFront, Read and Homework all draw on it.
- Re-measure the afternoon glance only after G1. The once-a-visit latch (`hour_glanced`,
  set only at osaka.rs:1762 and 2259) is a separate decision (window Q3).

---

## 4. Spot-checks that held (what the design can rest on)

| Claim | Where checked |
|---|---|
| Restless 90 s, Tidy 60 s, Daydreams 10 min, Comfort 8 min, Sleepy 15 min | brain.rs:67-81 |
| Arriving Restless 0.7, Tidy/Comfort/Fun/Daydreams 0.5 | brain.rs:87-95 |
| Mood rates (lazy Restless 0.4, industrious 1.5, dreamy 0.7, …) and shares 50/20/20/10 | brain.rs:176-205 |
| Bases: Walk 14, Travel 10, Pull 16, Stand/Sit/LieBack 4, SpaceOut/Gaze 6, LookOut 8, Watch 10 | brain.rs:670-740 |
| `COOLDOWN` 0.4, `TOP` 4, `INERTIA` 3, `FLOOR` 0.1, `NEUTRAL` 0.5, `WEIGHT` 2 | brain.rs:779-789 |
| Needs rise only in `choose_next`, by `at - decided` | osaka.rs:5515-5526 |
| Walk and Travel credited whole as she sets off | osaka.rs:5705-5710 |
| `SPACE_OUT_MS` (6000, 14000); `Activity::duration`; `use_duration` (LookOut 15–30 s, Watch 20–45 s, Homework 30–60 s / 120–240 s in its slot) | osaka.rs:419, 1130-1158, 1216-1226 |
| SpaceOut (and Work) never credited | osaka.rs:4964-5003 |
| Watch binds with only a TV, seated beside it, `Pose::Sit` unless the seat is a sofa | room.rs:184; mod.rs:3437-3459; osaka.rs:7189-7194 |
| `census_group` puts Away, Out and Door in "moving"; `visit_from` has no hidden filter; `Osaka::hidden` covers Away and the door's hidden beats | osaka.rs:5249-5283, 6650-6659; census.rs:366-374 |
| Away off screen 4–12 s, or a shift `SHIFT_MS` | osaka.rs:2793-2804 |
| Census chat every 45 s, 90 s and 60 s | census.rs:54, 82, 107 |
| `place`, `walk`, `pull`, `swap` and `take_link` uniform; `Bind::spot` private | mind.rs:140-147, 253-276, 318-340, 434-441 |
| `MAKESHIFT_ODDS` 20; `MAKES` [Sofa, Bed]; `footprint` falls back to the sofa's shape | mind.rs:24; scrap.rs:94-103 |
| `builds` filters only kinds standing in `visit.made`, and `tend_made` drops fallen ones (so "one of each a visit" in design.md is not enforced) | mod.rs:3591-3594, 3520 |
| `hour_glanced` set only in `Osaka::new` and `glance_up` | osaka.rs:1762, 2259, 2294 |
| Visiting computes `fading` after `retain`; Away before | mod.rs:1482-1500, 1538 |
| The shell redraws on a timer only if `advance` says so, but also on `ui.advance_clock` and `next_tick_hint`, which narrows the visible window further | dessplay/src/ui/shell.rs:527-541 |
| No golden room owns a window or clock; `day_census` asserts nothing | tests/golden.rs (grep); census.rs:1296-1372 |
| `her_mood_shows`: moving "within a few percent" by mood | census.rs:1445-1449 |
| `set_off >= 8`; furniture > 10× floor rest | census.rs:1394, 1442 |
| Made-piece property: `BOUND` 90 s, `proptest_cases(8)`, 120 s override | tests.rs:7443-7460; .config/nextest.toml |

Left (inferred) as the sections had it:
- Which 24 traces the rain-out fix moves (rainout: resident 16 + errand seeds 1 and 3, 8).
  plan.md says 24; the split isn't verified.
- That the Away fix survives a mutant.

---

## Levers and risks for 5c

| Brief item | Plugs in at | What it moves | Critic's note |
|---|---|---|---|
| Pre-commits (G1) | Visiting arm (mod.rs:1499-1500) or a `tend_fades` helper (proposed); `credit_done` arms for SpaceOut and Work (osaka.rs:4964-5003) | 24 traces; then all 112 hashes plus the snapshot | Three commits, each with its own trace diff, before any measuring |
| 1. Measure | test-only purpose × motion classifier beside `census_group` (census.md §8, proposed); in-sight filter; starts per minute (G6) | printed tables only | Run ASCII **and** line art (G3), at a fixed chat cadence and with none (G2), unfed and fed |
| 2. Band test | 12 plain tests, room × mood, unfed (census.md §9) | gate time about 1.5–3 s each | Thresholds come from step 1's per-seed spread; the mood spread probably needs a lever beyond need rates (C3) |
| 3a. Slower needs, lower Walk | brain.rs:70-71, 691 | goldens; `a_restless_osaka_mostly_moves` | Tidy matters on the stage only (C2); the waves of C1 mean Walk's base is less dominant in steady state than the arrival tables suggest |
| 3b. Longer still acts, settling in | durations (osaka.rs:419, 1130, 1216); a continuation after `leftover` (osaka.rs:5630), **not named `settle`** (C7) | goldens; the made-piece `BOUND` (G9); Sleepy by day (G14) | Without resume-after-chat (G2), longer acts barely show in the stage or resident rooms; design.md rule (G15) |
| 3c. Nearest spot | `place` / `walk` / **`pull` / `swap`** with `pick_weighted` | goldens even where the pick is the same (G8) | Same floor first for pull and swap is the stage's big lever (G7) |
| 3d. Daydream sessions | SpaceOut fire arm or a new `ScriptId` (window §7a) | goldens; pooled-line test | Needs G1's SpaceOut credit first |
| 3e. TV from the floor | host pose rule (osaka.rs:7189-7194) | the surf test (osaka.rs:8712-8714); art | Binding already works; new art only (G13) |
| 4. Floor homework, desk, read, 3× make | new `Activity` or a floor place for `Use::Homework`; `MAKES` with explicit shape arms; **want factor for `use/make` in the `factor` closure** (C4) | `golden_stage_room`; `sofa_census`; the made-piece property; `at_home_her_furniture_beats_the_floor` | Measure bare rooms unfed, not from a fed week (G4); "...my sofa." fallback (mind.rs:599) |
| 5. Window as a daydream | LookOut row, duration, script (`Span::Upto`) | LookOut pins only (no golden owns a window) | Re-check the glance after G1 (G16) |

**Open questions for the user** (beyond those the sections raise):
1. At what chat cadence is the band judged: none, the census's 45/60/90 s, or one fixed
   "typical" rate? And should a still act cut by chat resume? (G2)
2. Is the band judged in line art (what the user sees), ASCII, or both? (G3)
3. Is a time share enough, or should a cap on starts per minute sit beside it, so faster or
   shorter walks can't meet the band on their own? (G6) And do animated still acts
   (LieFront's kicks, Jacks) count?
4. Which mood lever is acceptable, given that need rates alone give about 4 points between
   lazy and industrious (C3)?
5. Fix SpaceOut's and Work's missing credit as pre-commits? Both differ from design.md:1459
   and the Work row. (G1)


---

