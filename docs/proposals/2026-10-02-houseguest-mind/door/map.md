# Door batch: code map
**2026-10-08, at HEAD `057311b1`.** Paths relative to `dessplay/src/ui/houseguest/`. Seven readers (read-only); the last section ("Gaps and corrections") overrides the ones above it.

<!-- section: door-lifecycle -->
## Code map: her external door's lifecycle (door batch)

Paths are relative to `dessplay/src/ui/houseguest/`. Line numbers are from the working copy at 057311b1 (2026-10-08). Nothing was edited.

### 1. Lifecycle, in order

**1a. The 08:15 cut**
- `osaka.rs:3491` `fn cut(&mut self, at: u64) -> bool`. Called at each routine boundary.
  - `school` is `slot == Away` (3496-3498). It sets `self.late` when a snack is cut (3520).
  - If `self.shift.is_some()` it calls `cut_shift` and then `school_from_work` (3524-3527).
  - Cuttable (3529): `props().stays ∈ {Rest, Job}`, or a `Walk { then: Then::Job(_) }` toward a job (3507-3517).
  - Then `interrupt(Cause::Routine)` (3532). Logs info "her routine cuts in" with `act`; otherwise trace "passes".
  - `Act::Door/Out/Away` are `Stays::Pass, OnChat::Back` (`osaka.rs:1218`), so a door is never cut.
  - **Trap:** a future `Walk{then: Job::Leave}` is "to_job", so the next boundary would cut it.
- `osaka.rs:8910` `fn interrupt(cause, now)`. `Cause::Routine` gives `(0, 0)` startle/look (8913-8917). It sets `Act::Look` at her current `(x, y)`, clears `rest` and `hopping`, and does no `beside` step. She stays inside the bed, sofa or desk (her seat is in the piece: `room.rs:980` `Shown::seat`).
- `osaka.rs:5323` `fn wake(at, terrain)`. Only the night's wake steps her out: `beside(&seat, terrain)` (5326-5334, `beside` at `osaka.rs:258`). The caller is `end_night` (5303). There is no equivalent on the school path.

**1b. The decision that sends her out** (`choose_next`, from `osaka.rs:7256`)
- Order matters for `Job::Leave`:
  1. errand (7274)
  2. no floor (7278)
  3. `returning` → `home_from` (7284)
  4. shift (7289-7301)
  5. **off-text check** (7305): `!terrain.restful(x, y) && find_rest(..)`. `restful` passes in-piece cells, so she is not moved out of the bed.
  6. `self.rest` (7309)
  7. **dash** (7312-7317): `dash_on(..)`
  8. `to_bed`/`to_school` (7322-7327) → `settle_for_night` (`osaka.rs:5130`)
  9. Ctx build
  10. **clock glance** (7349-7362): `glance_first(ClockGlance::School)` (`osaka.rs:5144`). With a clock shown, the first decision after the cut is the glance; the decision after that is `go_out`.
  11. `to_bed` → `send_to_bed` (7368)
  12. `to_school` → **`go_out(Routine::School, whims, at)` (7371)**
- `osaka.rs:5266` `fn go_out(&mut self, why: Routine, whims: Whims, at: u64) -> Decision`
  - Faces the chat if watching. Says `LATE` or a `mind::OFF` line. Sets `leaving = Some(why)` and `credit = None`.
  - Then `Act::Door { since: at, to: (self.x, self.y), gap: u64::MAX }` (5283-5290). This is the bug: the door's spot is her feet.
  - Returns `Decision::reflex("routine/away")`. Tests assert this method name (`tests/dash.rs:199`).
  - Log: info "out through her door" with `?why` only (5277). No spot.
- `osaka.rs:5296` `pub fn gone_out(&self, now) -> Option<Routine>` is `leaving` once `hidden(now) && door(now).is_none()`, i.e. the gap beat (DOOR[6]).
- `osaka.rs:3671` `pub(super) fn leaving()`.

**1c. Act::Door and the DOOR beats** (`osaka.rs`)
- `Act::Door { since: u64, to: (i32, i32), gap: u64 }` (`osaka.rs:676-680`, enum `Act` at 565). One act serves both the door in space and her external door; nothing distinguishes them except `gap == u64::MAX` or `leaving`/`returning`/`dash`/`shift`.
- `DoorBeat { door: Option<DoorFrame>, her, there, ms }` (1230). `const DOOR: [DoorBeat; 13]` (1239-1319):
  - 0 Closed+her 600
  - 1 Ajar+her 300
  - 2 Open+her 700
  - 3 Open 400
  - 4 Ajar 250
  - 5 Closed 350
  - 6 None (gap) 600, or `gap` if larger
  - 7 Closed there 400
  - 8 Ajar there 250
  - 9 Open there 350
  - 10 Open+her there 600
  - 11 Ajar+her 300
  - 12 Closed+her 400
- `door_beat(elapsed, gap)` (1322, saturating). `DOOR_THROUGH_MS` (1339, the sum of beats 0-5). `DOOR_THERE_MS` (1353, the start of beat 7).
- The Door step in `tick` (`osaka.rs:4062-4092`):
  - When `beat.there`, it teleports `(x, y) = to`.
  - At the end it calls `poke` (errand), or `back_home` (4071: `returning` → `home_from`, else `shift` → `come_home`), or says a `mind::DOOR` line (not on a dash) and decides.
- Queries: `hidden(now)` (9056), `door(now) -> Option<DoorFrame>` (9067).
- Census: `census_moves` gives `Act::Door → Moves::Off(Body::Door)` (887). The act summary lists it as "moving" (7001); `act_summary` prints "Door to (x, y)" (2750).

**1d. Work by door and school_from_work**
- `mind.rs:464` `fn work(c, _, _) -> Option<Bind>`. Gated by `c.may_work`. It picks a `Route::Around` link, preferring one whose landing is not in the chat, and returns `Bind::Work(Option<Link>)` (`mind.rs:209`).
- `osaka.rs:6255` `may_work`: `furnished && !worked && episode.is_none() && day.work_open()`.
  - `routine.rs:463` `work_open` is `!school_day && WORK window`.
  - Unfed (no clock), it is `at >= arrived + WORK_AFTER_MS`.
- `osaka.rs:7702-7704`: `Bind::Work(out)` → `go_to_work`.
- `osaka.rs:9463` `pub fn go_to_work(&mut self, out: Option<Link>, at, rng)`.
  - Sets `shift = Some(Shift::Going)`, `worked = true`.
  - `Some(link)` → `travel(link)`, walking out at a screen edge.
  - `None` → `Act::Door { since: at, to: (x, y), gap: rng.range(SHIFT_MS) }` (9473-9480). Feet again; `SHIFT_MS` is 60-180 s (`osaka.rs:1407`).
  - Log: info "off to work", no spot.
  - Stage caller: `stage.rs:636` (`Scene::Work`, places her first at 600-634).
- `osaka.rs:3552` `fn school_from_work(at) -> bool`.
  - Sets `leaving = Some(School)`.
  - For an `Act::Door` not yet `there`, `gap = u64::MAX`; for `Act::Away`, `until = MAX`.
  - Plants no door of its own: it freezes `go_to_work`'s door wherever it stood. Only reachable when a shift spans 08:15 (stage, or unfed then fed).
  - Log: info "from work, on to school".
- `cut_shift` (`osaka.rs:9530`) reads `Act::Door{gap>0}` as the shift's share.

**1e. Out: the visit ends** (`mod.rs`)
- `mod.rs:1855` `fn gone_out(&self, now) -> bool`. Checked in `tick` (`mod.rs:1731`) and in `paint_state` (`mod.rs:2079`); both call `out_by_door(now)`.
- `mod.rs:1869` `fn out_by_door(&mut self, now)`
  - Records `DoorAt { x: visit.osaka.x, y: visit.osaka.y, facing: visit.osaka.facing }` (1874-1878) and sets `self.out = Some(Out { door: Some(door) })` (1881).
  - `stands = furnished && (Normal || (Dash && comes_in(now)))`.
    - If not, it rains the visit out via `leave` (1893-1897). `out` is kept with that spot, which is the dash's return spot next time.
    - Otherwise it rains her moved and made text out from origin `door.x`, then `State::Away(Empty::new)`.
  - Log: info "out, her door shut behind her" with `kind` only (1873). No spot. A second info line for the not-shown case (1894).
- Types (`mod.rs`):
  - `struct DoorAt { x, y, facing }` (691)
  - `struct Out { door: Option<DoorAt> }` (700, Default)
  - `Guest.out: Option<Out>` (844)
  - `enum State { Absent, Arriving(How, Option<Box<Empty>>), Visiting, Leaving, Away(Box<Empty>) }` (645)
  - `enum How { Idle, Return(Option<DoorAt>), Dash }` (663-675)
  - `Kind { Normal, Dash }` (679)

**1f. Away: paint_empty**
- `mod.rs:2092-2124` is the `State::Away` paint arm. `unkept` is the frame's protected list before the gate adds the focus (2107). It passes `&mut self.out.get_or_insert_default().door` (2108) into `paint_empty`. A cold start gets `Out{door: None}` here and from `absent` (1748).
- `mod.rs:4657` `fn paint_empty(empty, fades, door: &mut Option<DoorAt>, ledger, graphics, buf, view, unkept, nudge, time, now, truecolor) -> bool`
  - Rain origin is `door.x`, else the screen middle (4672).
  - `shown = ledger.home.project(buf, &view.nooks, &blocked)` (4704). `covers = Shown::cover`.
  - `fit = Terrain::read(buf, unkept, line_art); fit.furnish(covers)` (4710-4711). `furnish` does nothing in ASCII (`terrain.rs:318-322`).
  - Re-place (4712-4720): only if `!door.is_some_and(|d| door_fits(&fit, d))`. Then it uses `door_spot(&fit, near)`, where `near` is the old spot or `middle(size)` for a cold start and facing defaults to Right. Log: debug "her closed door stands" with `x, y`. The new spot is written back into `Out`.
  - Shown filter (4722-4727): `fits(door) && !protected.any(box_meets(rect, (x, y)))`, a 5×4 box test.
  - Line art: `terrain::image(door.x, door.y, &covers).with` decides which pieces go into the door's image (4750-4755). Then `draw_door` (4766-4770).
- `mod.rs:4607` `fn door_fits(terrain, (x, y)) -> bool` is `platform_at(x, y).is_some() && restful(x, y)`. `terrain.rs:328` `restful` accepts piece cells inside her image (`calm || floor || piece`, 356-358).
- `mod.rs:4613` `fn door_spot(terrain, near) -> Option<(i32, i32)>` returns `near` if it fits; otherwise the nearest platform cell by L1 distance, then y, then x. Callers:
  - `paint_empty` (4716)
  - the arrival `through` closure (2036)
  - `stage.rs:485` (`super::door_spot`)
  - tests (`tests/away.rs:388, 487`)
- `mod.rs:4600` `fn middle(size) -> (w/2, h)`.

**1g. Leaving Away**
- `mod.rs:1792` `fn school_out(now)`: `door = out.take().and_then(|o| o.door)`, then `State::Arriving(How::Return(door), home)`. Log: info "home from school", or debug "client is busy" → `call_off`. Called from the `Away` arm when `!school` (1671) and from `absent` (1751).
- `mod.rs:1294` `fn leave(now)`: the Away arm is `rain_home(*empty, self.closed_door(), now)` (1308). `out` is kept.
- `mod.rs:1365` `fn rain_home(empty, door: Option<DoorAt>, now)`: origin `door.x`. Log: info "her empty home goes".
- `mod.rs:1391` `fn call_off(now)`: uses `door_of(how)` (1399).
- `mod.rs:1422` `fn door_of(how) -> Option<DoorAt>`: `Return(d) → d`, `Dash | Idle → closed_door()`.
- `mod.rs:1849` `fn closed_door() -> Option<DoorAt>`. Also reached by tests.
- `mod.rs:2805` `fn door_rain(now) -> Option<Dissolve>`: rains the frozen cells under `Door::closed(door.x, door.y, door.facing).cells()` (the ASCII cells, also used for the line-art rain). Used only by `send` (errand, 2780).
- `mod.rs:1663-1674` (Away tick): a `dash && comes_in` gives `State::Arriving(How::Dash, home)`. Log: info "dashing home from school".
- `mod.rs:1740` `fn absent(now, school, dash)`: in school hours it sets `out.get_or_insert_default()` (1748), which is how a cold start gets `Out{door: None}`. A dash from Absent gives `Arriving(Dash, None)`. A furnished home on an open gate gives `State::Away` (log info "her home, empty", 1767).
- `mod.rs:2565` `fn begin_visit(osaka, terrain, size, kind, now)`: `Kind::Normal` sets `self.out = None` (2580). `Kind::Dash` keeps `out`.
- `mod.rs:1712-1720`: a dash still in at 12:45 and not leaving becomes Normal, with `out = None`.

**1h. Arrivals: Return and Dash** (`mod.rs:2019-2072`)
- `let terrain = Terrain::read(buf, &view.protected, graphics)` (2026). **Not furnished.** No `home.project` runs before the arrival.
- The `through` closure (2033-2037): `near = door.map_or(middle(size), xy)`, facing defaults to Right, then `door_spot(&terrain, near)`.
- `How::Return(door)` → `Osaka::back_through_door(spot, facing, Routine::School, now, rng)` (2041).
- `How::Dash` → `through(self.closed_door())` → `Osaka::dash_in(spot, facing, met, ..)` (2049-2052).
- If no spot: `!present()` → `quiet_since = now`, and the home vanishes (size changed) or `call_off` (2059-2077).
- `osaka.rs:9111` `pub fn back_through_door(spot, facing, why, now, rng) -> Self`: `Act::Door { since: now - DOOR_THERE_MS, to: spot, gap: 0 }`, `returning = Some(why)`, `greeted = true`. No log.
- `osaka.rs:9138` `pub fn dash_in(spot, facing, met, now, rng) -> Self`: the same act, `dash = Some(Dash::In)`. No log.
- `osaka.rs:9090` `arrive_for_errand(spot, ..)`: a door in space at the accordion (beats from `DOOR_THROUGH_MS`).
- `mod.rs:2755` `fn send(..)` (errand), from Away, Absent or Arriving:
  - `door_rain`, then `arrive_for_errand`, then `begin_visit(kind_of(at_school))`.
  - She does **not** come through her external door. After the poke she goes out again via `go_out` from wherever she stands.

**1i. Dash flows** (`osaka.rs`)
- `osaka.rs:5684` `fn dash_on(dash, here, terrain, chances, at) -> Option<Decision>`
  - Takes a stage cue, `pass_glance`, then `fridge_seat` → `go_to(Want::Use(Snack), Job::Use(seat))` ("dash/lunch"), or `SpaceOut` ("dash/forgot").
  - Returns `None` (5708, 5729-5730 path) once her lunch is done or the fridge is gone; `choose_next` then falls through to `go_out` at 7371.
  - **It never calls `go_out` itself.** Out again, the door is wherever her dash left her, usually the fridge's seat.
- `dashing()` is at 9156. `start_job` intercepts `Dash::Lunch` (4651-4654).
- `osaka.rs:9163` `pub fn dash_through(&mut self, spot, now)` (stage only): `place(spot)`, then `dash = In`, then `Act::Door{since: now - DOOR_THERE_MS, to: spot, gap: 0}`. Caller `stage.rs:465-496` (`Scene::DashIn | DashForgot`): `near = approach(terrain, fridge.x, ..)` beside the fridge, or her `(x, y)`, then `super::door_spot(terrain, near)`. That terrain is `visit.terrain`, which is furnished.

**1j. Other ways an exit door is planted** (missing from the brief's table)
- `osaka.rs:9334` `evict(focus, terrain, chat, now, rng)`, `leaving.is_some()` arm (9345-9370): when her box meets the focused pane, she is pushed through.
  - In an `Act::Door` it backdates `since` and sets `gap = MAX`.
  - Otherwise it sets `Act::Away{until: MAX}` at her `(x, y)`.
  - `out_by_door` then records her feet, which may be inside the focused pane. Log: debug "out of the focused pane, out for good".
- `evict` non-leaving arm: a door not yet `there` is retargeted with `calm_elsewhere` (log debug "her door opens elsewhere"). A new door with a `SHIFT_MS` gap is used if she is walking out to work.
- `osaka.rs:9196` `errand(spot, ..)`: sets `leaving = None`.
  - In an `Act::Door` not yet `there` (this includes the routine door, which has `gap = MAX`), it sets `*to = spot; *gap = 0` (9205-9208).
  - In `Away`/`Out`, it opens a new door to `spot`.
  - After the poke, the routine `go_out` runs again from the accordion.
- `osaka.rs:8611` `place(x, y, at)` clears `leaving`, `returning` and `dash`, and calls `cut_shift`.

### 2. Job and go_to: how a `Job::Leave` would travel

- `scenes.rs:235` is `enum Job`, with 7 variants: `Pull`, `Swap`, `Build`, `Use(Seat)`, `Lift`, `SetDown`, `Borrow`.
- A new variant must be handled at every exhaustive site:
  - `Job::by_ref` (`scenes.rs:277`)
  - `JobRef` (`scenes.rs:315`): `spot` (327), `text_cells` (342), `side` (353), `box_row` (367)
  - `Heading::find` (`mind.rs:1473`): it needs an arm finding the door spot in `Chances`
  - `Heading::row` (`mind.rs:~1540`)
  - `Bind::on` (`mind.rs:221-232`)
  - `start_job` (`osaka.rs:4636`, match at 4637ff)
  - the job-kind label in `osaka.rs:7066`
  - `act_summary` (2735-2740)
  - stage `Scene` jobs (`stage.rs:533`)
  - `JobRef` matches at `osaka.rs:1138` and 7213
- `osaka.rs:8026` `fn go_to(&mut self, want: Want, job: Job, here: usize, terrain, at) -> bool`
  - Same floor: `pursue(job)` (8500), i.e. `Walk { to: x, then: Then::Job(job) }`. Log: debug "walking to a job".
  - Routed: sets `heading = Heading{want, job}`, `hopping = true`, then `travel(link)` (8515). Log: debug "heading for a job on another floor".
  - No route: **a door in space**, `through_door((x, y))` (8051-8061).
  - No floor: `false`.
- On arrival, the walk's end (4403-4415):
  - `Then::Job(job)` with `job.spot() == (x, y)` → `start_job`.
  - Elsewhere → `came_to_nothing`.
  - `settle` (8931) turns `then` into `Then::Nothing` if the floor shrinks (8971-8977).
- A heading is resumed only through the mind's offers (`osaka.rs:7567-7576`). Routine reflexes run before that point, so the school reflex re-enters on each landing, as `send_to_bed` (5200) does. `send_to_bed` is the model: it loops `go_to(want, job, ..)` and returns `Decision::reflex(method)`.
- `osaka.rs:9076` `pub fn through_door(to, at)`: `Act::Door{since: at, to, gap: 0}`. Log: debug "a door in space" with `from` and `to`. Callers:
  - `go_to` (8059)
  - `Bind::Door` (7699)
  - `find_rest` (9274)
  - `head_for_errand` (9304)
  - the Poke end (4109)
  - `stage.rs:658`
- `osaka.rs:9251` `find_rest(here, terrain, chances, at, rng)`: `nearest_rest` walk, else `elsewhere(.., restful, chat)`, then `through_door`.
- `osaka.rs:9281` `head_for_errand(terrain, at)`: walks within `ERRAND_WALK` on the same floor, else `through_door`.
- `mind.rs:440` `door_away(c, w, _)`: only with no links. It picks another platform (chat ones less likely) and x uniformly on it, then `Bind::Door((x, p.y))`. It runs no `restful` or piece check (cleanup item). Registered in `TRAVEL` (`mind.rs:267`).

### 3. Chances (what the guest passes the mind each tick)

- `osaka.rs:26` `pub(super) struct Chances` (derives `Default`). Fields:
  - `pulls`, `swaps`, `loose`, `seats`, `real`, `builds`, `borrows`, `mine`, `advert`
  - `furnished` (49)
  - `chat: Option<Rect>` (52)
  - `broken`, `repairs`, `lift_at`, `judged`, `beauty_here`
  - `clock: Option<ClockOn>` (66; `ClockOn` at 72)
  - `in_chat` helper (`impl` at 351)
- Built exhaustively, with no `..Default`, at two production sites. Both need a new `door` field:
  - `mod.rs:2353` (`offered`, for `stage::direct`)
  - `mod.rs:2403` (the tick's `visit.chances`)
  - Inputs come from `visit.shown`/`view.nooks`; `clock = clock_on(&visit.shown, &view.nooks)` (`mod.rs:211`, call at 2345).
- `visit.terrain` is read and furnished at `mod.rs:2299-2300`, before chances are built.
- `osaka.rs:2770` `take_in(&chances)` keeps `beauty_here` and `clock_on` between frames. The stage calls it first (`stage.rs:454`).
- There are about 60 test constructors; most use `..Default::default()`.

### 4. How the door is drawn

- `mod.rs:347` `struct Door { frame: DoorFrame, x, y, facing, standing }`
  - `Door::of(osaka, now)` (357) puts the door at her `x, y, facing`, always her feet.
  - `Door::closed(x, y, facing)` (370).
  - `layer()` (382) gives `Look::Door(frame)` at `(x, y)`.
  - `cells()` (392) comes from `sprite::door_cells`.
- `Figure { her, door }` (409), `Figure::of` (416), `door_alone` (423), `BoxArt { figure, with }` (453).
- During a visit, `draw` (`mod.rs:3022`) puts the ASCII door from `osaka.door(now)` → `sprite::door_cells(frame, osaka.facing)` at `osaka.x/y` (3033-3049). Line art goes through `Figure::of` (2437).
- `mod.rs:4786` `fn draw_door(buf, graphics, door, with: Vec<Shown>, looks, terrain, truecolor) -> (Vec<Frozen>, Option<BoxArt>)`
  - ASCII: `put` each cell with `DOOR_INK`, only where `terrain.open` (4813-4830).
  - Line art: the `with` pieces back to front, then **the door layer last** (4880), then `paint_layers`.
- `graphics.rs:71` `Look::Door(art::DoorFrame)`, size `(WIDTH, HEIGHT)` (104), render `art::render_door` (114).
- `art.rs:1073` `enum DoorFrame { Closed, Ajar, Open }`. `door_scene` (1081), `render_door` (1102), `art/door.svg`.
- `sprite.rs:466` `DOOR: [[&str; 4]; 3]` (5 wide). `door_cells(frame, facing)` (489) is anchor-relative, `dx = col - WIDTH/2`, `dy = row - HEIGHT`.
- **The side-on door is already in the tree, as unwired dead code** (commit 8eaa25c1, `#[cfg_attr(not(test), allow(dead_code))]`):
  - `art/wall-door.svg`; `const WALL_DOOR` include (`art.rs:16`)
  - `art::WallDoor { Shut{flap: u8, away: bool}, Ajar, Open, Post }` (1121)
  - `WALL_DOOR_FRAME` = 6×4 cells (1141), `WALL_DOOR_SHIFT` (1142)
  - `flap_plate(degrees)` (1146), `wall_door_body` (1175), `wall_door_scene` (1206)
  - `render_wall_door(door, facing, line, w, h)` (1221)
  - `sprite::WALL_DOOR: [[&str; 4]; 4]` (479), covering cols `to-3..=to`
  - Lints: `art::tests::the_wall_door_keeps_to_its_columns` (`art.rs:3869`), `sprite::tests::the_wall_door_is_ascii_and_keeps_to_its_columns` (`sprite.rs:~777`)
  - Ignored `wall_door_sheet` (`art.rs:3430`); `door_sheet` (3344)
- Still missing for the side-on door:
  - a `Look` variant for it
  - a `Door::closed` sibling
  - a `door_cells` sibling that places at the wall
  - the wall-line clip of her image
  - `[`/`]` in `sprite::mirror` (`sprite.rs:453`), which the left wall needs
- `mod.rs:4143` `draw_flap` is the precedent for painting over the wall column (skips protected cells, records `Frozen`).

### 5. Logging at these sites today

| Site | Level | What it logs | Spot logged? |
|---|---|---|---|
| `go_out` | info | `?why` (`osaka.rs:5277`) | no |
| `out_by_door` | info | `kind` (`mod.rs:1873`), plus "rains out, not shown" (1894) | no |
| `paint_empty` | debug | `x, y` (4717), only on re-place | yes, only then |
| `cut` | info / trace | `act` (3531 / 3534) | no |
| `school_from_work` | info | (3553) | no |
| `go_to_work` | info | (9464) | no |
| `school_out` | info / debug | (1797 / 1799) | no |
| `absent` | info | "her home, empty" (1767), "dashing home" (1667, 1751) | no |
| `begin_visit` | info | arrived / dashed home (2582 / 2586) | no |
| `rain_home` | info | (1366) | no |
| `through_door` | debug | `from, to` (9077) | yes |
| `evict` | debug | "out for good" / "opens elsewhere" / "out of the focused pane" | no |
| `go_to` | debug | `?job` | no |

`back_through_door`, `dash_in`, `dash_through`, `door_rain`, `door_spot` and `door_fits` log nothing.

### 6. Tests covering these paths

`tests/away.rs`:

| Line | Test or helper | Note for builders |
|---|---|---|
| 55 | `closed(DoorAt)` | |
| 63 | `door_shows` | assumes `Door::closed` cells and box |
| 96 | `until_away` | |
| 110 | `out_to_school` | returns a `DoorAt` |
| 125 | `a_school_morning_out_through_her_door_and_home_again` | asserts the door is where she set off (175-177) and she comes home there (216); breaks with `Job::Leave` |
| 246 | `what_she_moved_rains_out_as_she_goes` | |
| 317 | `a_visitor_at_school_time_sees_her_home_and_a_key_rains_it_out` | |
| 376 | `her_door_stays_where_she_left_whatever_takes_her_home_away` | uses `door_spot(middle)` (388) |
| 466 | `a_resize_moves_her_door_to_the_nearest_spot_it_fits` | calls private `door_fits`/`door_spot` |
| 504 | `a_residents_focused_pane_keeps_her_home_out_while_shes_away` | |
| 578 | `a_focused_pane_across_her_door_keeps_all_of_it_away` | |
| 626 | `a_cold_start_in_school_hours` | |
| 697 | `a_chat_line_while_shes_out_doesnt_stop_her_coming_home` | |
| 736 | `her_coming_home_isnt_called_off_by_a_key_or_a_chat_line` | |
| 801 | `busy_as_school_ends_she_comes_in_later_on_the_idle_gate` | |
| 844 | `a_parcel_waits_for_her_to_come_home` | |
| 892 | `a_parcel_waits_while_shes_between_her_doors` | |
| 985 | `out_at_work_as_school_begins_she_goes_on_to_school` | |
| 1014 | `rained_out_on_her_way_to_school_she_is_simply_gone` | |
| 1054 | `placed_as_she_sets_off_she_isnt_leaving` | |
| 1078 | `late_for_school_only_when_it_cuts_her_breakfast` | |
| 1129 | `cued_in_at_school_time_her_routine_sends_her_out_again` | |
| 1160 | `an_errand_from_her_empty_home_rains_her_door_out` | |
| 1186 | `somewhen` | |
| 1232 | proptest `her_days_never_touch_what_is_protected` | `owned` draws `Furniture::ALL[0..4]` = Sofa/Tv/Bed/Desk; `graphics` is `any::<bool>()` |
| 1257 | `her_days_case_pulling_text_as_school_begins` | |
| 1277 | `door_raining` | |
| 1292 | `a_cued_arrival_called_off_leaves_her_empty_home_as_the_cause_would` | |
| 1341 | `an_errand_as_she_comes_in_from_her_empty_home_rains_her_door_out` | |
| 1379 | `school_ending_on_a_busy_client_rains_her_empty_home_out` | |
| 1410 | `home_from_school_with_nowhere_to_come_in_her_empty_home_rains_out` | |

`tests/dash.rs`:
- 153 `a_dash_home_for_her_lunch_and_out_again`: asserts the last method is `"routine/away"` (199) and `closed_door().is_some()`.
- 267 `lunch_run`
- 426 `a_fridge_gone_from_her_lunch_doesnt_keep_her_home`
- 454 `dashed_home_with_no_fridge_she_cant_think_what_for`
- 544 `a_restart_after_her_dash_never_dashes`
- 614 `the_errand_at_school_is_a_dash`
- 686 `still_in_from_a_dash_as_school_ends_she_is_home`
- 741 `on_her_way_out_from_a_dash_as_school_ends_she_comes_home_once`
- 900 `with_no_home_she_dashes_home_from_absent`
- 980 `the_errand_at_school_for_a_visitor_at_the_keys`
- 1029 `an_overlay_ends_a_dash_home_at_once`
- 1144 `a_dash_cued_on_a_visit_keeps_the_visit`
- 1222 proptest `a_dash_home_has_her_lunch_whatever_the_chat`
- 1272 proptest `a_dash_home_never_touches_what_is_protected`
- 1299 `an_overlay_as_she_dashes_home_rains_her_empty_home_out`

Elsewhere:
- `tests/window.rs:207` `her_door_at_her_sofa_keeps_her_window_behind_it` writes `out.door = Some(DoorAt{..})` directly (223).
- `tests.rs:759` `long_visit_of`: in Away, `feet = closed_door().(x, y)` (818) and `assert_untouched_but_feet`. A side-on door painting the wall column breaks this oracle.
- `tests.rs:3763` `a_door_on_a_protected_floor_redraws_only_its_floor`; `tests.rs:3821` `out_at_work`; `tests.rs:2586` `she_gets_out_of_a_pit_through_a_door`.
- `tests/census.rs:1913` `Where::of` reads `guest.out.is_some()`.
- `tests/rain.rs:306` `covered_as_school_ends_her_home_rains_out`.
- `tests/calendar.rs:409` `home_from_school_its_the_first_beat_she_owes`.
- Goldens: `tests/golden.rs:577` `school_morning`, `:589` `home_from_school`, `:600` `dash_home`; tests `golden_school_morning` (983), `golden_home_from_school` (997).
- `osaka.rs` unit tests:
  - `her_routine_leaves_what_runs_its_course` (12038)
  - `a_shift_by_the_edge_ends_at_home` (13497)
  - `walking_out_to_work_as_school_begins_she_walks_on_to_school` (13561)
  - census tests at 15643, 15764, 15815 and 16147 (these build `Act::Door` directly)

### 7. What the `Out`/`DoorAt` shape change touches

- `How::Return(Option<DoorAt>)` (`mod.rs:669`)
- `door_of` (1422)
- `closed_door` (1849)
- `rain_home(.., door: Option<DoorAt>)` (1365, origin `door.x`)
- `call_off` (1399)
- `door_rain` (2805)
- the `through` closure (2033)
- `out_by_door` (1874-1881) and its rain origin (1909)
- `paint_empty`'s `door: &mut Option<DoorAt>` and origin (4672)
- the Away arm (2108)
- tests: `tests/away.rs` 55, 63, 110, 388, 466; `tests/window.rs:223`; `tests.rs:818`; `tests/census.rs:1920`

### 8. Brief claims that are now wrong or stale

| Brief says | Correct now |
|---|---|
| `go_out` osaka.rs:5151-5179, called at :7181, logs at :5164 | 5266-5291, called at 7371, log at 5277 |
| `cut` :3395 treats **any `Act::Use`** as cuttable (:3436) | `cut` 3491; cuttable is `props().stays ∈ {Rest, Job}` or a Walk toward a job (3507-3529); `Act::Door` is `Stays::Pass` (1218) |
| `interrupt` 8710-8727 | 8910-8927 (`Cause::Routine` gives a 0 ms look) |
| `wake` 5208, beside 5212-5222 | 5323, beside 5326-5334; `fn beside` at 258 |
| `Shown::seat` room.rs:1000-1004 | room.rs:980 |
| Off-text check before go_out, osaka.rs:7113-7116 | 7305; also the clock glance (7349-7362) and `settle_for_night` run between the cut and `go_out`, which the brief doesn't mention |
| "Out again after a dash: `go_out` through `dash_on`, osaka.rs:5568" | `dash_on` is at 5684 and never calls `go_out`; it returns `None` and `choose_next` reaches `go_out` at 7371 |
| `go_to_work` None arm 9263-9282, called at :7511 | 9463-9482, called at 7703 |
| `school_from_work` 3456-3474 | 3552-3570 |
| "Work is open only on days off (osaka.rs:3427-3428)" | the rule is `routine::work_open` (`routine.rs:463`) via `may_work` (`osaka.rs:6255`); unfed, work opens after `WORK_AFTER_MS` with no clock (and no school) |
| `out_by_door` mod.rs:1691-1703, log :1695 | 1869-1882, log 1873 |
| `leave` 1200-1222 | 1294-1336 (Away arm 1308) |
| `door_rain` 2603-2625 | 2805-2835 |
| `school_out` 1615 | 1792 |
| `begin_visit` 2378 | 2565 (`out = None` at 2580) |
| Arrival mod.rs:1840-1851, idle wait 1872-1875 | `Terrain::read` at 2026 (still unfurnished), `through` 2033-2037, back/dash 2041/2049, no-room wait 2059-2077 |
| `paint_empty` 4466-4486, image 4508-4512, ASCII put 4564ff, door layer last 4619 | 4657; fits 4712-4720; standing 4722-4727; image 4750-4755; ASCII 4813-4830; layer 4880 |
| `door_fits` 4365-4367, `door_spot` 4371-4381 | 4607-4609, 4613-4623; `door_spot` also has a stage caller (`stage.rs:485`) |
| `Door::closed` mod.rs:308 | 370 |
| `draw_flap` 3901-3935 | 4143 |
| Stage `dash_through` osaka.rs:8963 | 9163; caller `stage.rs:465-496` |
| `go_to` door-in-space ~7866, `go_to` call 7834 | `go_to` 8026; door-in-space arm 8051-8061 |
| Chances `door` "next to `clock` (osaka.rs:~70)" | struct at 26, `clock` field at 66; production constructors at `mod.rs:2353` and 2403 |
| `room::free` 1792-1807 "her box may overlap none" over `cover()` | `free` is at room.rs:1820, private, tests `s.rect()` per cell (not `cover()`); `her_box` is at 1801-1813 |
| Other room/rules lines | `Extent` 736, `strips` 795, `Home` 1120, `top()` 954, `hung_clear` 1281, `project` 1348, `move_off` 1378, `doorstep` 1517, test `hung_pieces_clear_every_standing_piece` 2426; rules `Before::new` 462 |
| `door_sheet` art.rs:3025-3082; `render_door` art.rs:1101; `door_cells` sprite.rs:475; `Look::Door` graphics.rs:69-110 | 3344; 1102; 489; 71 (size 104, render 114) |
| §5.3 / §7.2: side-on door is the *next* batch, 1-column or 2×4 / 40×160 canvas, art not drawn | **In this batch** (plan.md). Art exists in the tree (§4 above): B, **6×4-cell frame (120×168, cols `to-5..=to`)**, ASCII 4 columns `to-3..=to`; shut B takes `to-1..=to`; `mirror` lacks `[`/`]` |
| §5.7 / Q1: parcels through another wall (A) | Superseded: parcels come through the flap in her door (B), resting at the narrowed extent's offset 0 (trailing edge `to-7`); `draw_flap` stays only as the fallback when the space yields |
| §5.6: work at a screen edge unchanged unless Q3 says otherwise | Superseded: work leaves by the same door; plan.md says "screen edge first when her floor reaches one", which is ambiguous about whether the `Route::Around` walk-out survives — ask before building |
| §7.3: felt rule `DoorClear` optional | In the batch, and it also covers pieces in the chat pane (plan.md) |
| §3 table lists the sites | Missing: `evict`'s leaving arm (`osaka.rs:9345-9370`) pins the exit door at her feet inside the focused pane; `errand` (`osaka.rs:9196-9230`) retargets the routine door to the accordion; errand arrivals from Away (`mod.rs:2771-2786`) come in by a door in space at the accordion, not her external door |
| `tests/away.rs:1232` "for `graphics in [false, true]`" | The proptest draws `graphics in any::<bool>()`; `owned` already includes Bed/Sofa/Desk (`Furniture::ALL[0..4]`) |

<!-- section: layout -->
## Code map: home layout (room.rs, ledger.rs) for the door batch

Paths are relative to `dessplay/src/ui/houseguest/`. Line numbers are for the working copy at `057311b1`, read on 2026-10-08. Sprite sizes: `sprite::WIDTH = 5` and `sprite::HEIGHT = 4` (sprite.rs:6, 8).

### Answers to the three questions

**What a strip is on screen.**
- `Strip::Bottom(Nook)` (room.rs:629-631) is the bottom border row of one quiet pane, between that pane's two side borders.
- `Nook` (room.rs:399-407) has three values: `List` (the series pane), `Users` and `Playlist`. Its doc says "the chat is too busy". The chat pane is never a nook and never a strip.
- The nooks come from app.rs:664-672: `[(List, panes.series), (Users, panes.users), (Playlist, panes.playlist)]`, with empty rects filtered out. The panes come from `renderer.controller_bounds(..)` (app.rs:3160-3164), borders included (`IdleView.nooks` doc, idle.rs:96).
- In the default layout (style.css:11-16) `#chat-column` and `#right-column` each take 50%. Series, users and playlist are stacked in the right column, so all three nooks touch the screen's **right** edge. None touches the left edge: their left wall is the middle border.
- Pane rects can overlap in tests, so nothing structural forbids it. In real layouts the panes are distinct controllers.

**How screen-edge walls are detected.**
- Only in `Home::doorstep`, room.rs:1526-1533:
  - `let screen = buf.area;`
  - `walls.push((rect.right() == screen.right(), strip, e, Side::Right))`
  - `walls.push((rect.x == screen.x, strip, e, Side::Left))`
  - then `walls.sort_by_key(|&(edge, ..)| !edge)`, a stable sort: edge walls first, otherwise nook order, Right before Left within a nook.
- Nothing else in room.rs knows about the screen. It zips `nooks` with `strips(nooks)`, which works because `strips` maps 1:1 in order.

**Where `Extent` from/to come from.** From `strips()`, room.rs:795-813, per nook `rect`:
- `from = rect.x + 1`: the first **interior** column. The left wall's column is `from - 1`.
- `to = rect.right() - 1` (room.rs:803): ratatui's `right()` is exclusive, so this is the right wall's **own** column.
- `floor = rect.bottom() - 1`: the bottom border row.
- `rows = rect.height - 2`: the interior rows.

**The two ends are not mirror images.** Pieces occupy `[from, to)`. The right wall is `to`; the left wall is `from - 1`. `doorstep`'s flap x confirms this (room.rs:1596-1599: Left → `e.from - 1`, Right → `e.to`).
- Right wall: space `to-6..=to-1`, her spot `to-3`, door columns `to-1..=to`.
- Left wall: space `from..=from+5`, her spot `from+2`, door columns `from-1..=from` (the art's `render_wall_door` puts its left column at `to` for a left wall facing Left: "to" there means the left wall column).
- A builder writing `to -= 6` / `from += 6` gets this right. One writing `wall - 6` for both sides does not.

### Types (room.rs)

| Item | Line | Notes |
|---|---|---|
| `enum Furniture` | 21 | `ALL` has 12 (53). `decor()` 69, `legacy()` 76, `may_overlap(other)` 98 (only Window↔Sofa), `spec()` 122 |
| `struct Spec` | 142 | `footprint`, `hang: Option<u16>`, `sit`, `uses`, `offers`… POSTER 309 (`hang: Some(4)` at 320, 4×2), CLOCK 324 (`Some(4)` at 336, 3×2), WINDOW 340 (`Some(1)` at 355, 4×2, `uses: [LookOut]`) |
| `enum Nook` | 399 | List/Users/Playlist; serde |
| `enum Use` | 410 | `inside()` 455, `asks_room()` 470 |
| `struct Seat` | 505 | `what, item, piece, x, y, facing`; `makeshift()` 516 |
| `enum Strip { Bottom(Nook) }` | 629 | derives Serialize/Deserialize/Hash |
| `enum Side { Left, Right }` | 635 | serde |
| `struct Anchor { side, offset: u16 }` | 643 | the near edge `offset` cells from `side`'s wall; serde |
| `struct Flap { x, rows: (i32,i32), side }` | 652 | wall column `x`, rows `rows.0..rows.1` |
| `struct Prop` | 660 | `item, strip, anchor: Option<Anchor>, at: u16` (thousandths, for older builds, 668), `facing, boxed, settled`. Not serde (ledger mirrors it) |
| `enum Lane { Floor, Wall }` | 684 | |
| `fn lift(item, boxed)` | 692 | 0 if boxed, else `hang.unwrap_or(0)`. **A boxed poster, clock or window is a Floor piece.** |
| `Prop::lane` / `needs` / `new` | 703 / 713 / 720 | `needs = (cols, rows + lift)`. `new` sets `anchor: None, settled: true` |
| `struct Extent { from, to, floor, rows }` | 736 | `holds((cols, rows))` 747: `to - from >= cols && self.rows >= rows`. `left(anchor, cols)` 753: Right → `to - offset - cols`, clamped to `[from, to-cols]`. `share` 764 (private). `pin(left, cols) -> (Anchor, at)` 771: the anchor counts from the nearer wall |
| `fn strips(nooks)` | 795 | see above; `pub(super)` |
| `fn order_key(anchor, index)` | 817 | Left by offset asc, newer nearer; then Right by offset desc |
| `fn pack(pieces, extent) -> Option<Vec<(idx, left)>>` | 831 | `None` if any piece fails `holds`, or if the two-pass squeeze overruns `from` (`edge >= extent.from`, 864). Order never depends on width |
| `fn pack_each` | 867 | wall lane: greedy, a piece that doesn't pack is left out alone |
| `struct Shown` | 890 | `item, facing, boxed, strip: Option<Strip>` (None = makeshift), `left, floor, scrap` |
| `struct Home { props: Vec<Prop> }` | 1120 | `#[derive(Clone, Debug, Default, PartialEq, Hash)]` |

### `Shown` geometry (room.rs:906-1117)

- `size()` 916: scrap footprint for makeshift pieces, otherwise the spec footprint.
- `lift()` 937: 0 for makeshift pieces. `lane()` 945.
- `top()` 954, private: `floor - lift - rows`.
  - Poster/clock: rows `f-6..=f-5`.
  - Window: `f-1-2 = f-3`, so `f-3..=f-2`.
  - Standing pieces: `f-rows..=f-1`.
- `rect()` 959: footprint only (not the floor). `cover()` 966: Floor lane is rect plus the floor row beneath; Wall lane is the rect.
- `seat(what, beside)` 980.
  - The `Lounge | Nap | Sleep | Homework` arm is at room.rs:1000-1004; it puts her at `mirrored(sit col)`, **inside** the piece.
  - Unpack/Crumple at 1006: `left + cols/2`, inside.
  - Watch/Read/Snack/Pet: `beside`. LookOut: `beside`.
  - Always `y: self.floor`.
- `beside()` 1035: `[rect.x - 3, rect.right() + 2]`, so her box just clears the footprint.
- `may_overlap(&Shown)` 1049: a real window and a real sofa, with one look-out spot whose `her_box` misses the sofa's cover.
- `look_out_spots()` 1081. `cells()` 1092 (ASCII/parcel glyphs). `screen()` 1110 (TV).

### `her_box`, `free`, `fits` and friends (room.rs:1696-1861)

- `pub(super) fn her_box(x, y) -> Option<Rect>` (1801): `Rect(x-2, y-4, 5, 5)`, i.e. columns `x-2..=x+2`, rows `y-4..=y` (her sprite **plus the floor cell row**). Returns `None` when it would leave the screen's top or left (it uses `u16::try_from`).
- `fn free(shown, blocked, at: Option<&Shown>, x, y) -> bool` (1820), **private**.
  - True when `!blocked(x, y)` and no shown `s.rect()` (not `cover()`) contains the cell, unless `at.may_overlap(s)`.
  - `at = None` means "her": she overlaps nothing.
  - For a box over Floor pieces, the rect test equals the cover test, because her box rows `f-4..=f-1` meet any standing piece whose columns overlap. So `her_box ∩ cover = ∅` is equivalent to `free(.., None, ..)` over her box's cells, apart from her floor row against a Wall piece (impossible: hung pieces sit above `f-1`).
  - The brief's "strict predicate" can be `her_box(x, y).is_some_and(|b| shown.iter().all(|s| !s.cover().intersects(b)))`. That is exactly `clear_of` in mod.rs:3702-3709, without its `of` exception.
- `pub(super) fn fits(buf, at, clear)` (1837):
  - every footprint cell is `clear`, blank, and not the right half of a wide glyph;
  - unless Wall lane, every floor cell `left..left+cols` on `at.floor` is `clear` and a line glyph (`strokes(c).is_some()`);
  - `untouchable` cells read as `None`, so they fail.
- `roomy` 1722 and `room_to_look` 1730 go through `room_for` (1736). Her box must be blank and `clear`, except for cells inside `at.rect()`. Beside-uses also need an unbroken floor under her box.
- `enum Room { None, ToUse, ToLook }` 1789 (private).
- `FACING_GAP` 1696, `faces` 1703.

### `Home` methods (room.rs:1124-1680)

- `rooms()` 1148, `owns` 1126, `unbox` 1131, `boxed` 1142.
- `furnished()` 1164 (private): the strips her props are on, in **prop insertion order**, not nook order. A strip with no props is never laid out. A door chooser that covers unfurnished strips (or no home) must iterate `strips(nooks)`.
- `on(strip, lane)` 1176 (private): `(index, anchor, needs)` for **anchored** props only.
- `pin_anchors(&mut, strips)` 1187: anchors any `anchor: None` prop by `e.pin(e.share(at))` on that strip's extent. If extents get narrowed, this must pin on the narrowed one, or a migrated `at` lands up to 6 columns off.
- `pub fn layout(&mut self, nooks)` 1209: `strips(nooks)`, then `pin_anchors`, then `laid_on`. **Takes `&mut`**: it writes anchors the first time a strip is seen ("moves nothing" means positions only).
- `laid_on(&self, strips)` 1217 and `hung_shifts(&self, strips)` 1225 both wrap `laid_and_shifted`.
- `laid_and_shifted(&self, strips) -> (Vec<Shown>, Vec<(Furniture, Option<u32>)>)` 1230. Per `furnished()` strip:
  1. finds `e`;
  2. `pack(on(Floor), e)`; on `None` the whole strip (wall included) is **skipped**;
  3. `stand`s the floor pieces;
  4. `pack_each(on(Wall), e)`;
  5. `hung_clear(.., &standing)`;
  6. records shifts;
  7. merges and sorts by index.

  **One `e` per strip serves both lanes** (1235-1248). The brief's 5.5 ("wall lane on the raw extent, floor on the narrowed") needs two extents per strip here, or the space refusal goes only into `hung_clear.meets`.
- `hung_clear(&self, hung, strip, e, standing)` 1281 (private):
  - `meets(left)` at 1299: `stand(prop, …).rect().intersects(s.cover()) && !may_overlap(s)` against `standing` only.
  - On a hit it searches between the neighbours (`from..=to`, 1320-1321), preferring `leans` and then the nearest; with none, it is left out (trace log at 1328).
  - This is the hook for "the window must not meet the space" (and the chat pane: n/a, see below). The space is not a `Shown`, so `meets` needs an extra `|| at.rect().intersects(space)`. That applies to the window only; poster and clock at `f-6..f-5` never meet a 4-row space.
- `pub fn project(&mut self, buf, nooks, blocked) -> Vec<Shown>` 1348:
  1. `strips`, `pin_anchors`;
  2. for each furnished strip that doesn't `pack` its Floor lane (or is absent), `move_off`;
  3. then `layout(nooks)`, keeping each `at` where `fits(buf, &at, &|x,y| free(&shown, blocked, Some(&at), x, y))` (1367).

  So `blocked` closets a piece for the frame and never moves it. That is the brief's reason not to put the space in `blocked` (it would closet a fridge at `Right,0`), and it is correct.
- `move_off(&mut, strip, buf, strips, blocked)` 1378 (private):
  - finds the first other strip where the moved Floor pieces (with that strip's own) `pack`, all on free cells;
  - Wall pieces re-anchor by share; with only Wall pieces it uses the Wall lane;
  - `tracing::info!` at 1431.
  - **With no target it returns silently**: the pieces stay on a gone or too-small strip, all closeted. The door chooser will meet "the door strip is absent and nothing moved".
- `pub fn spot(&self, buf, nooks, shown, blocked, item, rng) -> Option<Prop>` 1445:
  - stage gift only (caller `place_gift`, mod.rs:3644);
  - tries `at = 0,100,…,1000` on every `strips(nooks)` entry, with a fresh `strips` call at 1462;
  - requires `fits` + `roomy` and prefers `room_to_look`.
- `pub fn doorstep(&self, buf, nooks, shown, blocked, seats, item) -> Option<(Prop, Flap)>` 1517:
  - walls as above;
  - LookOut items try look-first;
  - the Prop has `Anchor { side, offset: 0 }`, `at` 0 or 1000, facing into the room, `settled: false`;
  - first `admits(parcel boxed, Room::None)`, then `admits(prop, ToUse|ToLook)`, then the `seats` closure on `with.project(..)` for the box and then the piece;
  - returns a boxed Prop and `Flap { x: from-1 | to, rows: (at.floor-2, at.floor), side }` (1596-1608), i.e. rows `f-2..=f-1`. These are exactly the approved art's flap rows.
  - Callers: mod.rs:3322 (order) and 3352 (the clock gift), inside `furnish`.
- `admits(&self, buf, shown, blocked, e, prop, room) -> Option<Shown>` 1617 (private):
  - `pack(with.on(strip, lane), e)`; every showing piece in that lane must still `fits`;
  - the new piece must `fits`, plus `roomy` / `room_to_look` per `room`;
  - other strips and the other lane stay as `shown`.
- `add(&mut, prop) -> bool` 1672: one of each item.
- `fn stand(prop, strip, extent, left) -> Shown` 1682 (private): `floor = extent.floor`.

### The parcel and its flap, against plan.md

- Today a parcel stands at `Anchor { side, offset: 0 }`, so its box is flush with the wall: right wall `to-cols..to-1`, left wall `from..`.
- With the narrowed extent the parcel lands at `to-6-cols..=to-7`, just past the space with a cell of air. That is what plan.md now **wants** ("Parcels come through it and rest just past the door's space"; snippets band 5: "at rest at the space's inner edge").
- So brief 5.7's "doorstep skips the door's wall" is **superseded**: deliveries come **through her door** (sheet B, the flap in its lower half).
  - On the door's wall, `Flap.x` stays the wall column and its rows stay `f-2..=f-1`.
  - Drawing changes from `draw_flap` (mod.rs:4143, glyph `╲`/`╱` over the wall line, `FLAP_MS`) to the wall-door art's flap frames.
  - Open design point: when her door's wall is not the first edge wall in `doorstep`'s order, does a parcel still come through her door, or through another wall's plain flap? plan.md implies her door; the sort at 1533 would need her door's wall first.
- The flap swung up reaches `to-5` (snippets geometry), inside the space, and is clear of the parcel at `≤ to-7`.

### The chat pane: where pieces can actually reach it today

- **Strip pieces cannot.** Floor pieces stand on `[from, to)` above a nook's floor (`holds` needs `rows <= extent.rows`). Hung pieces stay inside the nook too: `top ≥ floor - extent.rows = rect.y + 1`, checked by the wall-lane proptest at 3193. Parcels from `doorstep` are strip pieces too.
- **Makeshift pieces can.**
  - `scenes::pulls(buf, terrain, protected)` (scenes.rs:584) scans every `terrain.platforms`, including the chat pane's borders and lines.
  - `scenes::builds` (scenes.rs:837) builds at `pull.x/pull.y` with `strip: None`.
  - The `clear` in mod.rs `builds` (3922; closure at 3940-3946) checks only `protected` and `clear_of_real`.
  - `tend_made` / `made_stands` (mod.rs:3837-3853) keep them under the same rule.
  - This is the live route for "furniture in the chat pane".
  - `rules::Frame::free` (rules.rs:428-442) refuses `made` rects, but that is the reverse direction.
- **Her door can.** `door_spot` (mod.rs:4613) searches every terrain platform and is not bounded by nooks (the case the user saw). `middle(size)` (mod.rs:4600) is `(w/2, h)`.
- A real-layout guard for the strip pieces would be an assertion that the nook rects don't meet `view.chat`. `IdleView.chat` (idle.rs:86) is the only chat rect. `protected` contains it only while a resident's selection is held (app.rs:645-647).

### ledger.rs: record format and adding a field

- `Ledger` (54-87) has `pub(super) home: Home` and the other fields. `clock_sent: bool` is at 86. Its doc at 36-37 says it is written last.
- `Home` is **not serialized directly**. `to_json` (237-282) builds `Saved` (568-600):
  - `rooms` comes from `rooms(&home)` (520);
  - `props: Vec<SavedProp>` (451: `item, at` with `#[serde(default)]`, `facing` defaulting to Right, `boxed`);
  - `anchors: Vec<SavedAnchor>` (605: `item, strip, anchor: Option<Anchor>` with `#[serde(default)]`);
  - `unsettled` is skipped when empty; the clock and counters are skipped at zero (`is_zero`, 513);
  - `calendar_on` is skipped when `None`, `seen` when empty;
  - `clock_sent` has `#[serde(skip_serializing_if = "std::ops::Not::not")]` (598-599).
- `from_json` (154-235):
  - reads `Raw` (623-655) through `object()` (615), which needs a JSON object;
  - every newer field is `#[serde(default)] Option<serde_json::Value>` and is read leniently: `clock_sent: raw.clock_sent.as_ref().and_then(Value::as_bool).unwrap_or(false)` (228-232);
  - props are rebuilt with `Prop::new(item, nook, at, facing)` and then the strip and anchor are overlaid from `anchors`.
- `Raw` has no `deny_unknown_fields`. Older builds ignore a new `door` key and **drop it on re-save**, so the wall is chosen again on the next read. That is harmless if the chooser is deterministic.
- `VERSION = 1` (46) must not bump; a version mismatch is an `Err` (157-159).
- **Recipe for `Home.door: Option<DoorWall>`:**
  - `Strip` and `Side` already derive serde, so `DoorWall` can derive serde directly.
  - In `Saved`: `#[serde(skip_serializing_if = "Option::is_none")] door: Option<DoorWall>`.
  - In `Raw`: `#[serde(default)] door: Option<serde_json::Value>`, read as `raw.door.and_then(|v| object::<DoorWall>(v)?.ok())`. Garbage reads as `None`.
  - Add a paragraph to the module doc (1-37) as well.
- **Test trap:** `the_clock_sent_round_trips` (ledger.rs:1226) asserts `text.ends_with(r#","clock_sent":true}"#)`. serde writes fields in declaration order, so declare `door` **before** `clock_sent` in `Saved`, or update that test and the module doc's "comes last".
- Other ledger tests to mirror:
  - `an_older_build_reads_past_the_clock` (1214) and the `as_an_older_build_reads` helper (708);
  - `an_older_record_reads` (764), `a_ledger_round_trips` (900), `furnished()` (664), `timed()` (906).
- `Summary` (369; `summary()` 306-365, `clock_sent` at 349 and 398) feeds `dessplay --dump` (dump.rs:232). Add the door wall there for the dump-state skill.
- **Migration** needs no rewrite: no `door` field reads as `None`, and the chooser fills it on first need. Nothing in `from_json` closets or moves a piece.

### Adding a field to `Home` breaks

- Struct literals `Home { props }` at room.rs:2335, room.rs:2837-2843 and room.rs:3141 (tests) need `..Default::default()`.
- `Home` is hashed into the mend basis at mod.rs:3581, so the door's wall changing re-runs the mend search. That is probably desired.
- `PartialEq` is used by `project` change detection (mod.rs:4704-4706: `changed = *ledger != before`), so choosing the wall saves the record.

### Callers that call `strips(nooks)` themselves (all must switch to `extents` or knowingly keep raw)

- In room.rs, `strips(nooks)` is called inside `layout` (1210), `project` (1353), `spot` (1462) and `doorstep` (1528). Only `laid_on`, `laid_and_shifted`, `hung_shifts` and `admits` take an extent from outside.
- **rules.rs** (another mapper's area; refs only):
  - `Before::new` 462: `strips` at 465, `layout` at 464, `hung_shifts(&strips)` at 473.
  - `evaluate` 517 uses `before.strips` at 534 and 548 (scratch homes: recompute per scratch if narrowing depends on packing).
  - `search` 734 builds candidates from `before.strips` at 760-835 (`hi: e.to - cols` at 776, `p.e.from..=p.hi` at 785, anchors at 826-827).
  - `against_wall` 324 (`WALL_GAP`).
  - `broken(layout, strips, home)` 183.
  - `Frame` 404; `Frame::free` 428-442.
- **mod.rs:**
  - `clock_on` 211 (strips at 216) and `beauty_at` 228 keep raw.
  - `furnish` 3211:
    - `project` at 3227, 3231, 3265, 3329, 3359;
    - the stage `cued` re-project at 3231;
    - set-down pinning with `room::strips` + `e.pin(e.left(..))` at 3253-3262;
    - `layout` for refused set-downs at 3269;
    - `rules::broken(&laid, &room::strips(..), home)` at 3366-3367.
  - `stage_arrange` 3485 (strips at 3495, `e.pin(e.left(..))` at 3511).
  - `place_gift` 3631 → `home.spot` at 3644.
  - Mend hash at 3581.
  - `paint_empty` 4657 → `ledger.home.project` at 4704; Away's `empty.shown` holds real pieces only, never makeshift ones.
  - `door_fits` 4607 and `door_spot` 4613.

### Proptests and generators (room.rs tests, 1863-3259)

- Helpers:
  - `anywhere` 1868 / `nowhere` 1877: seat stubs for `doorstep`.
  - `pane(rows)` 1893; `USERS` 1897 (18×6).
  - `prop(item, at)` 1906: Users, facing Right.
  - `home(nook, props)` 1910.
  - `empty(title, w, h)` 1922: bordered pane rows.
- Generators:
  - `standing()` 2235: items with `hang.is_none()`.
  - `pieces()` 2245: 1-5 standing items.
  - `decorated()` 2252: 0-4 standing, plus a poster, plus maybe a window, inserted at a random index.
  - `placed(items)` 2269: each prop either `Anchor { side: Left|Right, offset: 0..40 }` or only `at: 0..=1000` (older record). So `Right, 0` is already sampled.
- Layout builders:
  - `users(width, text)` 2306: one Users pane `Rect(0,0,width,6)` filling the buffer. **Both walls are screen edges** under `doorstep`'s test.
  - `two_panes(width, height, text)` 2388: Users `Rect(0,0,w,h)` at x=0 (left edge) and Playlist `Rect(w,0,40,12)` flush to the right edge. The buffer is `w+40` wide, `max(h,12)` tall.
  - Neither has a chat pane. For the chat rule, add a layout with a chat rect (e.g. chat at x=0, nooks to its right, as `tests.rs:resident_view` 6501 does).
- `packing_keeps_order_and_a_resize_and_back_restores` (proptest, 2328; 256 cases through `proptest_cases`):
  - order and walls (`s.left >= 1 && right < wide`);
  - a resize and back is the identity on `room` and on `project`;
  - text and blocked cells only closet.
  - Natural home for "packs both raw and narrowed keeps anchors and order".
- `hung_pieces_clear_every_standing_piece` (2426) is a plain `#[test]`, **not** a proptest. It asserts:
  - `may_overlap` is symmetric and holds only for Window/Sofa;
  - `Window.hang == Some(1)`;
  - every other hung item has `hang >= tallest`;
  - nothing hung is used `inside`.
  - Extend it statically: poster and clock `hang >= sprite::HEIGHT` so they clear a 4-row space; the window does not, so it must be refused from the space.
- `the_wall_lane_keeps_order_and_never_moves_the_room` (proptest, 3133; 128 cases):
  - per lane and strip: order, inside walls (3191-3193), wall vs floor overlap only with the window over a sofa's corner;
  - resize, low wall, text.
  - Natural home for "no floor piece or window meets the space".
- Unit tests to mirror for doorstep, migration and the window:
  - `a_delivery_never_meets_her_window_but_for_a_sofa` 2863 (40×9 Users; the right wall is tried first);
  - `a_poster_is_delivered_where_it_fits_boxed_and_hung` 2915 (asserts `flap.rows == (6, 8)` and a right-side anchor);
  - `a_window_comes_in_where_she_can_look_out` 3052;
  - `a_crowded_wall_leaves_out_only_what_it_cannot_hold` 2828;
  - `what_hangs_goes_where_the_room_goes` 2773;
  - `a_strip_too_small_moves_its_pieces_together_or_not_at_all` 1991.
  - Several hard-code the right wall as first and lefts like `[1, 5]`. A narrowed extent on the door's wall changes them; give these fixtures `door: None` or a chosen wall explicitly.
- **Day-long fixture trap (tests.rs):**
  - `view()` (tests.rs:20) sets `chat: Rect::new(0, 0, 30, 20)`.
  - `long_visit_of` (759) uses `nooks: nooks(w, h)` (tests.rs:62), which **includes List at `Rect(0,0,w/2,h-3)`**, overlapping that chat rect. A "nothing meets the chat pane" property added to `her_days_never_touch_what_is_protected` would fail on the fixture itself.
  - Set `chat` to a rect that is not a nook: drop List from nooks, or put chat at `nooks[0]` as `resident_view` (6501-6511) does.
  - Also, `rooms(w, h)` boxes: the left box touches the screen's left edge; the right boxes touch the right edge.

### Brief claims in this area that are stale or wrong

| Brief | Current |
|---|---|
| `Extent` room.rs:738-743 | 736-741 |
| `to = rect.right() - 1` at 803-806 | 803. Correct that `to` is the wall column; but the **left** wall is `from - 1`, not symmetric |
| `strips` room.rs:797 | 795 |
| `top()` room.rs:956 | 954-957 |
| `hang: Some(4)` ~321/337, window ~355 | 320, 336, 355 |
| `Shown::seat` arm 1000-1004, Unpack 1006 | still 1000-1004 / 1006 (`seat` starts at 980) |
| `Home` room.rs:1119-1121 | 1120-1122 |
| `hung_clear`'s `meets` 1298-1303 | 1299-1304 |
| `project`'s `fits` 1366-1370 | 1367-1371 |
| `move_off` 1377 | 1378 (info log 1431) |
| doorstep edge test 1519-1523, sort 1525 | doorstep starts at 1517; edge test 1529-1530; sort 1533 |
| doorstep flap 1536-1569 | the Prop is built at 1542-1558; the Flap at 1596-1608 |
| `her_box` 1773-1787 | 1801-1815 |
| `free` 1792-1807 | 1820-1835. Note it tests `s.rect()`, not `cover()` |
| `Prop.at` 668-670 | 667-668 |
| `hung_pieces_clear_every_standing_piece` room.rs:2383, "proptest next to it" | 2426, a plain `#[test]`. The random-home proptests are at 2328 and 3133 |
| ledger `Saved` 567ff, `Raw` 615ff | `Saved` 568-600, `Raw` 623-655, `object()` 615 |
| rules.rs:465 `Before::new` | `fn new` 462, `strips` at 465 (534, 548 unchanged) |
| rules.rs:428-440 `Frame::free` | 428-442 |
| rules.rs:324-327 `against_wall` | 324-327 (unchanged) |
| mod.rs `clock_on` 154, `beauty_at` 167 | 211, 228 |
| mod.rs `broken` caller 3141 | 3366-3367 |
| set-down pinning 3047 | 3253-3262 |
| `stage_arrange` 3269 | 3485 (called at 3229) |
| `builds`' `clear` ~3696 | `builds` at 3922, `clear` at 3940-3946; `clear_of_real` 3776 |
| `draw_flap` 3901-3935 | 4143ff |
| `door_fits` 4365-4367, `door_spot` 4371-4381 | 4607-4609, 4613-4626 |
| `paint_empty` 4466-4486 | starts at 4657; project at 4704; door re-place at ~4710-4720 |
| `out_by_door` 1691-1703 | 1869ff |
| `begin_visit` 2378 | 2565 |
| 5.4 "strips she has pieces on (in Nook order)" | `furnished()` is **prop insertion order**; nook order exists only as the `nooks` vec order (List, Users, Playlist, app.rs:664) |
| 5.4 "the same test `doorstep` already uses" | correct, but it is inline in `doorstep` (1529-1530), not a reusable fn; extract it |
| 5.5 "the wall lane packs on the raw extent" | `laid_and_shifted` uses one `e` for both lanes (1235-1248); needs two extents or a `meets` change |
| 5.5 "`spot`, `doorstep`/`admits` switch to extents" | `admits` already takes `e` from `doorstep`; only `doorstep` and `spot` call `strips` |
| 5.7 "doorstep skips the door's wall" | superseded by plan.md: parcels come through the door's flap; the narrowed extent already rests them just past the space |
| 5.3 canvas 40×160, 2×4 cells | superseded by snippets.md: 120×168, 6×4 cells at a piece's scale; art placed from `to-5` (right wall) or `to` (left wall), top `f-4` |
| §6.3 "doorstep … place no floor piece in the space" | with deliveries through the door, the **parcel** must also stay out of the space, at the inner edge |
| §5.8 implies strip pieces could be in the chat pane | they cannot (nooks never include chat); only makeshift pieces (`pulls` over every platform) and the door (`door_spot` over every platform) can. Hung decor is confined to nook rects |

<!-- section: rules -->
# Code map: felt rules and phase-4 mending (rules.rs, mind.rs, osaka.rs, mod.rs glue)

Paths are relative to `dessplay/src/ui/houseguest/`. Line numbers are from the working copy at 057311b1 (2026-10-08).

## 0. Findings the coordinator must see first

1. **Under the default layout, a strip-anchored piece can't be in the chat pane.** Every `Prop` stands on `Strip::Bottom(nook)`, and the only nooks are List, Users and Playlist (`room.rs:397-406`, doc comment "the chat is too busy"). `room::strips` (`room.rs:795-813`) builds each Extent inside its nook rect: `from = rect.x+1`, `to = rect.right()-1`, `floor = rect.bottom()-1`, `rows = height-2`. `Extent::holds` (`room.rs:747`) caps a piece's height, hang included (`Prop::needs`, `room.rs:711`), at `rows`. The nooks come from `panes.series/users/playlist`, the chat is `panes.chat` (`ui/app.rs:653-669`), and both come from `layout/renderer.rs:1098 controller_bounds`. The default layout tiles them without overlap. Runtime layouts do allow overlays (`layout/compiler.rs:933-943`). I did not prove that a nook can never meet the chat under a user layout.
   - **So "an older home's piece in the chat pane" probably can't happen for bought, hung or delivered pieces unless the layout overlaps.** The ones that do reach the chat pane today:
     - **Makeshift pieces.** `scenes::builds` (`scenes.rs:837`) runs over text pulls anywhere, chat included. `builds`' `clear` (`mod.rs:3938`) refuses only protected cells and real pieces. `pick_build` (`mind.rs:~735-770`) only weights the chat by `CHAT_FACTOR = 0.1` (`osaka.rs:178`), and only for a resident, because `Chances.chat` is `None` otherwise (`osaka.rs:50-52`, `in_chat` at `osaka.rs:353`).
     - **Her door,** which stands wherever her feet are.
   - Makeshift pieces have `strip: None` (`room.rs:~900`, `Shown.strip`). The rules never judge them: `judge`'s `here()` requires `s.strip.is_some()` (`rules.rs:203-207`), and `Belongs` skips strip-less pieces (`rules.rs:264`). They also live only for the visit (`Visit.made`, `mod.rs:493`), so nothing old needs migrating. For them the chat rule is **refusing a placement**, not a felt rule.
   - Recommendation: ask the user or coordinator whether the felt chat-pane rule has any case for anchored pieces. If not, judge it anyway (`s.cover().intersects(chat)`) as cheap insurance against an overlapping layout, and make the real fix the refusal in `builds`, `pick_build`, `doorstep`, `door_place` and `Frame::free`.
2. **Feeling a rule only happens during `Act::Use` on a real piece.** Lamp, Plant, Poster and Clock have `uses: &[]` (`room.rs:245,301,315,331`), so a use-triggered DoorClear would never fire for a lamp or plant standing in the door's space. It needs another trigger (§4).
3. **"To the closet" is not an act or a state.** A piece is in the closet when it is laid out but missing from `project`'s `shown` for that frame. No persisted flag, `HomeEvent`, or `Osaka` method sends a piece there (§6).
4. **She never mends anything in a Lazy mood.** `Mood::home_acts` is Lazy 0, Ordinary/Dreamy 1, Industrious 3 (`brain.rs:209-215`). `Osaka::to_mend` returns nothing once `home_acts >= mood.home_acts()` (`osaka.rs:8430-8432`). Felt rules last one visit only (phase4 user decision). So "until she clears it the space yields" can go on indefinitely. That's by design, but tests must force the mood (as `stage`, `arranging` and `force` do).

## 1. The rule table (rules.rs)

| Item | Line | Notes |
|---|---|---|
| `enum Rule { Faces{seat,screen}, Near{a,b,gap}, AgainstWall(Furniture), Apart{a,b}, Belongs }` | 18-37 | `Copy`, `PartialEq`. Exhaustive matches: `kind` (168-177), `judge` (223-277), `partners` (701-708, has `_` arm), and the facings match inside `search` (754-757, has `_`). Only one use outside rules.rs: `tests.rs:10127` (`Some(rules::Rule::AgainstWall(Furniture::Fridge))`). |
| `struct RuleRow { rule, felt_on: &'static [Use], grievance: &'static str }` | 41-46 | |
| `ANY_USE: &[Use]` | 50-60 | Used by `Belongs`'s `felt_on`. |
| `RULES: [RuleRow; 6]` | 63-104 | A **fixed-size array**. Add a row by bumping the count. Rows: 0 Faces(Sofa,Tv) "Can't see the telly...", 1 Near(Lamp,[Bed,Desk],3) "Too dark in here...", 2 AgainstWall(Fridge) "This wants a wall...", 3 AgainstWall(Bookshelf) "Wobbly... needs a wall.", 4 Apart(Bed,Tv) "Too noisy to sleep...", 5 Belongs "Hm... not in here." |
| `FACES_ROW = 0` | 107 | Used by the stage force at `mod.rs:3377`. |
| `WALL_GAP = 1` | 110 | |
| `struct Grievance { row, piece }` (Hash) | 116-120 | The key of a felt rule. The same while the rule stays broken. `label()` returns `"kind(piece)"`. |
| `struct Broken { row, pieces, involved, key }` | 136-147 | `pieces` are the pieces a move may mend it by. `involved[0]` is `key.piece`. `felt_using(piece, what)` (162-165) returns `involved.contains(piece) && felt_on.contains(what)`. |
| `fn kind(row) -> &str` | 168-177 | Log words. Needs an arm per new variant. |

**New rows go at the END.**
- Tests index rows by position: `tests.rs:9891`, `:10206` and `:10253` use `RULES[0]`; `tests.rs:10355` and `:10838` use `RULES[5]` (Belongs).
- `osaka.rs:10824-10833 broken_for` (test helper) takes the *first* row whose `felt_on` contains a use. A new `ANY_USE` row placed before Belongs would change which row that is.

**Where the speech line lives.** It is `RuleRow.grievance`, a `&'static str` written with the houseguest's `line!` macro (`mod.rs:102-119`). The macro checks at compile time that the line fits a bubble and is ASCII or narrow.
- Grievances are **not** `mind::Pool`s (pools are `mind.rs:877-1130`, type at `mind.rs:1321`).
- They are not script.rs scripts, and `mind::all_lines` (`mind.rs:~1343`) doesn't list them.
- "Can't get to the door!" (22 chars) goes in its `RuleRow`. A chat-pane line, e.g. `line!("Not in the chat...")`, also goes in its row.
- If DoorClear is felt at the door rather than while using a piece (§4), the line is said with `Osaka::say(text, now)`, as `THERE` (`osaka.rs:486`) and `PARCEL` (`mod.rs:185`) are. It is not the `Act::Use` grievance bubble (`osaka.rs:9789-9793`, which shows `g.rule()?.grievance` only during `Act::Use`).

## 2. Judging what's broken

- `pub(super) fn broken(layout: &[Shown], strips: &[(Strip, Extent)], home: &Home) -> Vec<Broken>` (`rules.rs:183-189`) calls `judge(row, ..)` for each row.
- `fn judge(row, layout, strips, home, out: &mut Vec<Broken>)` (`rules.rs:193-277`):
  - `here(item)`: a laid piece with `scrap.is_none() && !boxed && strip.is_some()`.
  - `settled(item)`: from `home.props`; a piece that's missing counts as settled.
  - `push(pieces, involved)`: the key is `Grievance{row, piece: involved[0]}`.
  - `AgainstWall` (244-251) finds the piece's strip in `strips`, then `!against_wall(at, e)`.
  - `Belongs` (262-276) pushes **one `Broken` per offending piece**. **This is the template for DoorClear and the chat rule:** one Broken per piece in the space or the pane, with `pieces = involved = [piece]`.
- `fn against_wall(at: &Shown, e: Extent) -> bool` (`rules.rs:324-327`) returns `at.left - e.from <= 1 || e.to - (at.left + cols) <= 1`. The brief wants `e` to be the **narrowed** extent, so a fridge beside the door space still counts as against the wall. That follows automatically if every `strips` passed in becomes `Home::extents`.
- Helpers: `spoils` (279), `spoilt` (287), `newly_spoilt` (302), `between` (318).

**No input for the space or the chat pane.**
- `judge` sees only `layout`, `strips` and `home`. An `Extent` can't hold the door's space or the chat rect.
- DoorClear and the chat rule need one more input, e.g. `Keep { door: Option<Rect>, chat: Option<Rect> }`. It could be passed to `broken`/`judge`, or carried as `Extent` gaining the space.
- **Every site that must change with it:**
  - `rules.rs:466` (`Before::new`)
  - `rules.rs:573` (evaluate: "it mends", `judge(key.row, &laid, &before.strips, ..)`)
  - `rules.rs:595` (evaluate: "no rule that held is broken")
  - `rules.rs:1035`, `rules.rs:1112`, `rules.rs:2336` (tests)
  - `mod.rs:3367` (the paint)

## 3. Repair search (rules.rs)

**`struct Frame<'a> { buf, nooks, blocked: &dyn Fn(i32,i32)->bool, shown, made: &[Rect] }`** (404-410)
- `clear(laid, at, x, y)` (417) calls `free(.., Some(at))`. Used for the piece itself: `room::fits` in `fits_now`.
- `room(laid, who, x, y)` (424) calls `free(.., None)`. Used for her box: `room::roomy`.
- `free(laid, who, over, x, y)` (428-441) returns `!blocked(x,y) && !made.contains(cell) && !laid.any(other piece's rect contains cell && !over.may_overlap(s))`.
- **Trap:** `Frame.blocked` is the **same closure** `project` closets by: `mod.rs:3220` builds it, then `project` at 3227 and the Frames at 3238, 3386 and 3591 all use it (`mend` receives it at 3385).
  - Putting the door space or the chat pane into that closure would also make `project`'s `fits` closet any older home's piece standing there on the first frame. The brief (§5.4) rejected that, and it would leave her nothing to notice.
  - Give `Frame` its own field (e.g. `keep_out: &[Rect]`, the space and the chat) checked in `free`, or a second predicate. Don't widen `blocked`.

**`struct Before`, `fn new(home, frame)`** (444-496)
- `home.clone().layout(frame.nooks)` (464). `layout` pins anchors and calls `room::strips` internally.
- `strips = room::strips(frame.nooks)` (465). The brief says this becomes `extents`. It must be **the same** extents `layout` used, or `evaluate` pins against different walls than layout packs on.
- `broken` (466), `roles`, `spoilt`, `hung_shifts(&strips)` (473), and `showing` (each shown piece plus whether `roomy`).
- Built once per `search` (735) and once per `check`/`check_again` (`judge_move`, 884).

**`fn evaluate(before, scratch: &mut Home, key, piece, to: Placement, again) -> Option<(Repair, Vec<Shown>)>`** (517-651), in order:
1. `key` is in `before.broken`, or `again`.
2. Finds the prop. Rejects a boxed piece or no change.
3. `e` comes from **`before.strips`** (534). `moved.at = e.pin(e.left(anchor, cols), cols).1`.
4. `scratch.laid_and_shifted(&before.strips)` (548). The prop is then restored.
5. The same pieces are still laid (551-558).
6. `pushes_hung` (567).
7. **The key is mended**: re-`judge` on `before.strips` (573).
8. No new spoiling (585-589), `becomes_den` (590).
9. **No rule that held breaks** (595-600).
10. Tier: settled pieces get 0, or `SETTLED_BEHIND = 3` when an unsettled rival is in `m.pieces` and this isn't a turn in place. Unsettled pieces get 0 / 1 / 2 by room role (607-628).
11. Cost: cells shifted, plus 1 for a turn (629-638).

- **Brief trap, still true and wider than the brief says:** narrowing depends on packing, so a scratch home may narrow when the original yielded (moving a piece off the door's strip is exactly what lets the strip pack narrowed). `before.strips` is read at 534, 548, 573, 595 **and** 760 (search's places). The brief lists only :534 and :548. For DoorClear, compute `extents` from `scratch` after the move (548/573/595) or the mend will never register.

**Other helpers**
- `pushes_hung` (653), `fits_now(frame, before, laid, piece)` (673-697: the moved piece `fits` and is `roomy`, and every piece that showed still fits), `partners(rule, piece)` (701).

**`pub(super) fn search(home, frame, target: &Broken) -> Search`** (734-866)
- For each piece in `target.pieces`, for each strip in `before.strips` (760) where `e.holds(...)`, the partner check and the `Apart` check pass, it builds a `Places`.
- Lefts run `e.from..=e.to-cols`, strided down to `CANDIDATES = 2000`.
- Anchor: kept where the piece stands now, else taken from the nearest partner's wall, else `e.pin`.
- Sorts by `(tier, cost, order, generated)`, filters with `fits_now`, and keeps `REPAIRS = 3`.
- A DoorClear or chat row falls into `partners`' `_ => vec![]`, so `keeps_company` is true. It needs no new match arm in `search`, only the narrowed or keep-out geometry.
- **It only moves pieces along nook strips.** It can't take a piece to the closet. An empty `repairs` is the only "fits nowhere" signal.

**Re-judging and trials**
- `check` (871), `check_again` (879) → `judge_move` (883-897). Both rebuild `Before`.
- `Trials` (905-983): `ties`, `of`, `remain`, `next`. `TIE_CELLS = 4`.

## 4. How a broken rule becomes her act

1. **Paint (`mod.rs:3206 furnish`)**
   - `blocked` (3220) → `home.project` (3227) → stage cue (3229) → set-down pinning (3236-3282) → deliveries, clock, gift (3284-3363).
   - Then `laid = home.layout(&view.nooks)` (3366) and **`rules::broken(&laid, &room::strips(&view.nooks), home)` (3367)** into `visit.broken` (3368-3374).
   - Stage force (3377-3384) → `mend(..)` (3385) → `visit.judging` (3386-3401) → ghost / carrying (3402-3407).
2. **Chances.** `Chances.broken` and `repairs` are cloned from `visit` (`mod.rs:2369-2371`, `2419-2421`). `lift_at` and `judged` come from `fn arranging(visit)` (`mod.rs:3449-3477`), with `reach` at 3430. `may_arrange = !to_mend(&broken).is_empty()` (`osaka.rs:7345`).
3. **Feeling it.**
   - On starting `Act::Use` of a real piece (`osaka.rs:4670-4677`): the first `b.felt_using(piece, seat.what) && !has_felt(b.key)` becomes `Act::Use{grievance: Some((key, from))}` (`osaka.rs:717-728`).
   - When the bubble window `from+GRIEVANCE_MS` (`osaka.rs:1650`, which is 2×`USE_FRAME_MS`) has passed, the Use tick pushes `Felt{key, on:(seat.item, seat.what), let_go:false}` (`osaka.rs:4122-4138`).
   - `felt: Vec<Felt>` is per visit (`osaka.rs:2136-2140`).
   - The only non-Use path is `pub fn feel(&mut self, key, on: (Furniture, Use))` (`osaka.rs:8121-8130`), used by the stage today.
4. **`to_mend(&broken) -> Vec<Grievance>`** (`osaka.rs:8430-8445`): felt, not `let_go`, still broken, under the mood cap. While she's carrying a piece, only that episode's key.
5. **`mod.rs:3553 fn mend(home, buf, view, visit, shown, blocked, force, now)`**
   - Targets are the felt keys found in `visit.broken`.
   - Throttle: a hash of `(home, &view.nooks, &wanted, &moved)` (3579-3581), `MEND_MS = 1000` (3544).
   - **Any new input (door wall, chat rect) must join the hash, or repairs go stale.** `Home` derives Hash, so a new `Home.door` field joins automatically. The chat rect does not.
   - The first target with repairs wins (3599-3608). A `debug_assert` re-checks each (3609-3616). Result goes into `visit.repairs`.
6. **Choosing.** Methods `"arrange/use-it"`, `"arrange/carry"` and `"arrange/lift"` (`mind.rs:274-276`).
   - `lift` (`mind.rs:~703-728`): when there's no episode and `may_arrange` holds, `Trials::ties` with whim-shuffled order, giving `Job::Lift(Lift{repair, trials, x, y, side})` (`scenes.rs:252-258`).
   - `carry` (`mind.rs:~687`): while the piece is in her pocket and the frame judges the move still holds, giving `Job::SetDown` (`scenes.rs:262-269`).
   - `use_it` (`mind.rs:660-676`): sits back down on `just_set`'s piece and use.
7. **Acting.**
   - `pursue` → `Job::Lift` (`osaka.rs:4856`) → `begin_episode(repair, trials)` (`osaka.rs:8150`) → `Act::Lift`. When it ends, `ep.pocket = true` (`osaka.rs:4180`); the piece then shows nowhere and its place is kept as `visit.ghost` (`mod.rs:3404`).
   - `Job::SetDown` (`osaka.rs:4875`) → `Act::SetDown` ends → **`HomeEvent::SetDown{piece, to}`** (`osaka.rs:4200-4206`).
8. **Recording** (`mod.rs:2926 fn record`): `SetDown` becomes `visit.set_down = Some((piece, to))` (`mod.rs:2956`).
9. **Committing** (next paint, `mod.rs:3236-3282`).
   - If the episode matches, `judge_move` runs (`mod.rs:3414`).
   - On success, the prop is written from **`room::strips(&view.nooks)` (3253)**: `strip`, `anchor`, `at = e.pin(..).1`, `facing`, `settled = true` (3254-3263). Then `osaka.set_down_done` (`osaka.rs:8181`): `home_acts += 1`, "There!", or trials. `shown` is re-projected.
   - On refusal, `set_down_refused(back)` (`osaka.rs:8231`) → `drop_episode` (8243), which sets `let_go`, so it's one try per rule per visit.
   - **The pinning at 3253 must use the same extents as `Before.strips`**, or `at` (the older-build share) and the anchor pin disagree.
10. `arrange_next` (`osaka.rs:~8289-8420`) runs trials, waits for the frame's judgement, takes another way, or drops it after `TRIES`.

**`HomeEvent`** (`osaka.rs:329-349`): `Bought`, `Unpacked`, `Crumpled`, `Used`, `SetDown{piece,to}`, `Calendar`, `Seen`. There is no event for "put away" or the closet.

## 5. Other mod.rs callers in scope

| Site | Line | Uses | Change for the batch |
|---|---|---|---|
| `broken`'s caller | `mod.rs:3366-3367` | `home.layout(&view.nooks)` + `room::strips(&view.nooks)` | → `extents`, plus the keep-out input |
| set-down pinning | `mod.rs:3253-3261` | `room::strips` | → the same `extents` as `Before` |
| `stage_arrange(home, buf, view, blocked) -> bool` | `mod.rs:3485-3540` | `room::strips` loop (3495). Plants TV at `Anchor{side,0}` and sofa at `tv_cols+4`, checks with `project` | → `extents`. On the door's wall the TV at offset 0 would sit beside the space (fine once layout narrows) |
| `mend` | `mod.rs:3553-3626` | Frame (3591) with the shared `blocked` | add keep-out to Frame and to the hash |
| `builds(buf, visit, pulls, protected)` | `mod.rs:3922-3960`, called at `mod.rs:2341` | `clear` (3938-3943): `!protected && clear_of_real(&visit.shown)` | refuse the chat rect (`view.chat`, not in the signature today) and the door's space |
| `made_stands` | `mod.rs:3837-3854`, from `tend_made` (`mod.rs:3791`, called at 2257) and delivery `seats` (3317) | — | a makeshift piece meeting the space or chat should fall apart (or never be built: refusing in `builds` is enough while the rect is stable; a resize already drops all made pieces, see `resized` at 3792) |
| `clock_on` / `beauty_at` | `mod.rs:211` / `mod.rs:227` | raw `room::strips` | keep raw (brief agrees) |

## 6. "A piece that fits nowhere goes to the closet": what exists

- The closet happens inside `project` (`room.rs:1348-1375`): `move_off` (`room.rs:1377`) runs when a strip's floor doesn't `pack`. A piece whose cells aren't `fits`-free with `blocked` (`room.rs:1366-1371`) is left out of `shown`; it stays in `layout` and in the record. `hung_clear` (`room.rs:~1253`) drops a hung piece that has no clear place, with a trace log.
- Nothing chooses it. When `search` returns nothing, `mend` just offers no repair and her felt rule stays offered until the visit ends (no `let_go`, since she never started an episode).
- Ways to build "fits nowhere → closet" (for the coordinator to choose):
  - **(a) Per-frame, no persistence.** After `mend` finds no repair for a ChatPane or DoorClear key, closet that piece for the frame by excluding it from `shown`. A `project` variant could take the keep-out rects for **pieces whose rule has no repair**. Cheapest, but the piece comes back each frame for her to "notice" again, and there's no act.
  - **(b) Persisted.** A new `Prop` flag, e.g. `stored: bool`, serialised `skip_serializing_if` false, as `clock_sent` is (ledger.rs `Saved`/`Raw`). `layout` and `project` skip it, and a new `HomeEvent::Stored(piece)` comes from an act (lift into her pocket, then "set down" into the closet: an `Episode` whose `repair.to` is a closet target, but `Placement` has no such variant). Real work: `Repair`, `Placement` and `evaluate` all assume a strip.
  - **(c) Narrowed-extent only (the door case).** `Home::extents` narrows only when the strip still packs. When it doesn't, the space yields: there is no closeting by construction, which is what brief §5.5 intends. DoorClear then moves one piece off the strip. If no other strip takes it, the space keeps yielding forever (no closet unless (a) or (b)).
- For anchored pieces in the chat pane, see §0.1. With disjoint panes it never arises, so "closet when nothing fits" only matters for DoorClear and any layout that overlaps.

## 7. What adding `Rule::DoorClear` and `Rule::ChatPane` takes

1. **`Rule` variants.** `DoorClear` and `InChat` (no payload: judged per piece, like `Belongs`). Add `kind` arms "door" and "chat".
2. **`RULES: [RuleRow; 8]`, rows appended at 6 and 7.**
   - `RuleRow{ rule: DoorClear, felt_on: ANY_USE, grievance: line!("Can't get to the door!") }`
   - `RuleRow{ rule: InChat, felt_on: ANY_USE, grievance: line!(...) }`
   - Feeling it through a Use covers only pieces with uses. Lamp and Plant are floor pieces with no uses, so DoorClear also needs a non-Use trigger:
     - **Option A:** `osaka.feel(key, on)` at the yielded door, i.e. when `go_out`/`Job::Leave` or the arrival uses the fallback spot. It's `pub` but documented as stage-only. `Felt.on` is mandatory `(Furniture, Use)` and feeds `just_set` → `arrange/use-it`. With a piece that has no use, `use_it` finds no seat and `arrange_next` returns None (`osaka.rs:8308-8320`), which is harmless.
     - **Feeling on the way out is useless.** `felt` dies with the visit (school runs 08:15 to 12:45), so feel it on the *return* or dash arrival out of the fallback door, and say the line there with `say`.
   - The `felt_on` set also matters for `Broken::felt_using` and `osaka.rs:10826`'s helper.
3. **`judge` arms.**
   - DoorClear: for each `here`-style laid floor or low-hung piece on the door's strip whose `cover()` meets the space rect (the window included; hung pieces at or above `f-5` exempt), `push(vec![p], vec![p])`.
   - InChat: for each laid piece whose `cover()` meets `chat`.
   - Both need the keep-out input (§2).
   - **Yield subtlety:** the space only exists while narrowed. When the strip yields, `layout` packs raw, so "meets the space" must be judged against the **would-be space rect** (the reserved cells), not the narrowed extent. Otherwise it's never broken while yielding, which is the only time it matters.
4. **evaluate/search with per-scratch extents and keep-out** (§3). `Frame::free` refuses keep-out cells for every piece (`over` or not).
5. **mod.rs:** the inputs at 3367, 3253 and 3495; the `mend` hash (3579-3581); Frame keep-out at 3238, 3386 and 3591; a felt trigger at the door's fallback site (door glue, other mappers' area).
6. **Tests to keep valid:**
   - rules.rs `mends` property (`rules.rs:2157`, judged at 2336), `a_move_leaves_no_settled_piece_spoiling_its_room` (1586), `the_fridge_and_the_bookshelf_want_a_wall` (1118). The narrowed extent must not make AgainstWall break for a fridge beside the space.
   - `repair_search_is_cheap` (2038): extents per scratch home add a pack per candidate.
   - New: an older record with a piece in the space is felt, moved, the space un-yields, and nothing is closeted. Do the same with a strip that has no other place, which stays yielding (or goes to the closet, depending on §6's choice).

## 8. Brief, plan and phase-4 claims that are now wrong or stale

| Claim | Where | Correct fact (now) |
|---|---|---|
| "`broken`'s caller (mod.rs:3141)" | brief §5.5 | `mod.rs:3367` (layout at 3366) |
| "set-down pinning (mod.rs:3047)" | brief §5.5 | `mod.rs:3253-3263` inside `furnish` (3206) |
| "`stage_arrange` (mod.rs:3269)" | brief §5.5 | `mod.rs:3485`; strips loop at 3495 |
| "`builds`' `clear` (mod.rs:~3696)" | brief §5.5 | `mod.rs:3938-3943` (`fn builds` 3922) |
| "`clock_on` (mod.rs:154)", "`beauty_at` (mod.rs:167)" | brief §5.5 | `mod.rs:211`, `mod.rs:227` |
| "evaluate judges against `before.strips` … used at :534 and :548" | brief §5.5 traps | Also `rules.rs:573` (it mends), `:595` (none broken) and `:760` (search places). Five sites, not two |
| `rules::Before::new` (rules.rs:465), `against_wall` (324-327), `Frame::free` (428-440) | brief | Still right: `new` at 462 with `strips` at 465; `against_wall` 324-327; `free` 428-441 |
| "the repairs' search" refuses keep-out (one placement check) | brief §5.8 | It has two halves: geometry (`Frame::free`, `fits_now`) and rule-holds (`judge` in `evaluate`). Both need the keep-out. The brief's "`blocked`" route is wrong for the search too, because `Frame.blocked` is `project`'s closure (§3) |
| "Older homes with a piece in the chat pane: she moves it out" | plan.md, brief §5.8 | Anchored pieces sit strictly inside nook rects. With disjoint panes the case can't happen; it mainly applies to makeshift pieces (strip-less, never judged by rules, per-visit) and the door. See §0.1 |
| "goes to the closet" | plan.md, brief §5.8 | There is no "send to closet". It's per frame and emergent (§6) |
| Phase4 D2 `RULES: &[RuleRow]` | phase4-design.md | `[RuleRow; 6]`, a fixed array |
| Phase4 D2 `Broken { row, pieces }` | phase4 | `{ row, pieces, involved, key: Grievance }` |
| Phase4 D4 `Osaka.felt: Vec<usize>`, felt marked by "`fire`'s frame tick" | phase4 | `felt: Vec<Felt{key: Grievance, on: (Furniture, Use), let_go}>` (`osaka.rs:2136-2140`), pushed in the `Act::Use` tick at `osaka.rs:4122-4138` |
| Phase4 D4: Faces felt on Watch too; Apart on Sleep/Watch; AgainstWall on Snack/Read | phase4 | Faces is felt on `[Lounge, Nap]` only; Apart on `[Sleep]`; AgainstWall(Fridge) on `[Snack]`, AgainstWall(Bookshelf) on `[Read]` |
| Phase4 D5: repair search lives in room.rs | phase4 | `rules.rs:734 search`, run from `mod.rs:3553 mend` |
| Phase4 D6 `Osaka.pocket: Option<Carry>` | phase4 | `episode: Option<Episode>` (`osaka.rs:104-125`) with `pocket: bool`, `set_down`, `tries`, `trials`, `tried`, `trying` |
| Brief §5.6 `Job::Leave` at scenes.rs:235 | brief | `enum Job` is now at `scenes.rs:234-249` (Pull, Swap, Build, Use, Lift, SetDown, Borrow) |

<!-- section: chat-pane -->
## Code map: the chat pane, the frame and the terrain (door batch)

Paths are relative to `dessplay/src/ui/houseguest/` unless they start with `ui/`. Line numbers are from the working copy on 2026-10-08, at master `057311b1`.

---

### 1. How she learns the screen layout

**`IdleView`**, at idle.rs:69-99, is the only input. `Guest::paint(buf, &IdleView, now)` (mod.rs:1992) takes it.
- `chat: Rect` (idle.rs:86) is a plain `Rect`, never an `Option`. When the chat is hidden (`display:none`) it is `Rect::default()`, which is empty and contains nothing.
- `nooks: Vec<(Nook, Rect)>` (idle.rs:96) holds "the quiet panes she may furnish, as drawn (borders included)".
- `protected: Vec<Rect>` (idle.rs:94) holds what her body and bubbles never cover.
- `focus: Option<Rect>` (idle.rs:81) is the focused pane, and is set only for a resident.
- `resident`, `scrollback`, `chat_mark`, `busy`, `delay` and `truecolor` are also on it.
- `IdleView::default()` has an empty `chat`. Any test built with `..Default::default()` therefore passes a "nothing in the chat pane" check without testing anything.

**Who builds it.** `Ui::idle_view(&self, images: &[Rect])` at ui/app.rs:597-673. It has one production caller, `ui/shell.rs:762` (`draw`), followed by `guest.paint(frame.buffer_mut(), &view, now)`. So `buf.area` is the whole terminal. The stage example (examples/houseguest.rs:165) and tests.rs:997 build it the same way.
- `chat: self.panes.chat` (app.rs:660).
- `protected` (app.rs:621-647) holds:
  - `grow(chat.input_area(), 1)`, which is the input frame and its border;
  - `panes.status` and `panes.keybar`;
  - each image grown by 1;
  - `chat.newest_message_rows()`;
  - the scrollback accordion;
  - **the whole `panes.chat`, but only when `resident && selecting`** (app.rs:644-646).
  Empty rectangles are dropped.
- `nooks` (app.rs:664-671) is `[(Nook::List, panes.series), (Nook::Users, panes.users), (Nook::Playlist, panes.playlist)]`, with empty rectangles filtered out. **The chat is never a nook.** Subtitles and health are not nooks either.
- `focus` (app.rs:639-642) is `panes.bounds(focus.name())`, resident only.

**`PaneRects`** (ui/app.rs:196-219) is private to app.rs. It has `chat, status, keybar, subs, series, users, playlist`.
- It is set twice per draw: from `scene.slot(..)` (app.rs:3105-3117), then overwritten from `renderer.controller_bounds(name)` (app.rs:3160-3164).
- `controller_bounds` is at ui/layout/renderer.rs:1098, and `record_controller` at renderer.rs:1078-1097. Each records the controller scene's first node, `bounds ∩ clip`.
- For the chat, that call is `renderer.record_controller("chat", ..)` at ui/components.rs:1421. Its root is `#chat-pane` (templates/chat.xml), so **`view.chat` is the whole chat column**: the log frame, the suggestions and the input frame.
- The input frame is already protected. The chat cells she can reach are:
  - the log frame's borders, in particular its bottom border, which is the input frame's top floor area;
  - day separators (`chat-separator` rows of box glyphs);
  - the accordion `╱╲`, which is a floor (terrain.rs:120-123).

**`gate()`** (mod.rs:2838-2851) produces the per-frame view she obeys. While the client is in use (input within the delay, or busy) and no errand is running, it pushes `view.focus` onto `protected`. Otherwise it sets `focus = None`.
- So a resident's focused chat is solid only while in use. **During an idle visit, the chat's floors are ordinary platforms** for both a visitor and a resident.
- `whole_glyphs` (mod.rs:541-562) widens each protected rectangle over a wide glyph that straddles its edge. It does not touch `chat`.

**Inside the mind, the chat is resident-only.** `Chances.chat: Option<Rect>` (osaka.rs:50-52) is `view.resident.then_some(view.chat)`, set at mod.rs:2302 and passed on at mod.rs:2368 and 2418. `Chances::in_chat` is at osaka.rs:353 and `CHAT_FACTOR = 0.1` at osaka.rs:178.
- **A visitor's brain never sees the chat.**
- The new rule (no piece and no door in the chat pane) holds whether or not she is resident. Builders must pass `view.chat` into `project`, `doorstep`, `spot`, `builds`, the repairs and `door_place`, **not** `Chances.chat`.
- Other chat consumers today, none of them about placement:
  - `drop_in(view.chat)` and `shaken(.., view.chat)` (mod.rs:2197, 2201) undo her mischief there;
  - `look(..)` toward the chat's middle column (mod.rs:2893-2899);
  - `evict(focus, terrain, chat, ..)` (osaka.rs:9334), where `calm_elsewhere` (osaka.rs:1364-1400) weighs chat floors at 0.1;
  - `mind::work` (mind.rs:460-476) prefers a `Route::Around` exit whose landing is not in the chat, resident only;
  - the brain's `Factor::InChat` (brain.rs:545-547, 641, 809-829).

---

### 2. Strips: the quiet panes she furnishes

- **`Nook`** (room.rs:397-406): `List | Users | Playlist`. Its doc says "the chat is too busy". The `List` variant's comment says "(series), when short", which is stale: app.rs passes `panes.series` at any height.
- **`Strip::Bottom(Nook)`** (room.rs:629-631) is the only variant. A `Strip` cannot name the chat.
- **`strips(nooks) -> Vec<(Strip, Extent)>`** (room.rs:795-813) maps each nook rect to an `Extent`:
  - `from = rect.x + 1`;
  - `to = rect.right() - 1`, which is the right wall's own column; `from - 1` is the left wall's column;
  - `floor = rect.bottom() - 1`;
  - `rows = height - 2`.
- **`Extent`** (room.rs:736-741) has `from, to, floor, rows`. Its methods are `holds` (747), `left(anchor, cols)` (753), `share` (764) and `pin` (771).
- **Callers of `strips` and of the layout built on it:**
  - room.rs: `layout` 1209, `project` 1348 (calls `strips` at 1353), `move_off` 1378, `spot` 1445, `doorstep` 1517;
  - rules.rs: `Before::new` 463-466 (`home.layout(frame.nooks)` at 464, `room::strips(frame.nooks)` at 465);
  - mod.rs: `clock_on` 211, `beauty_at` 228;
  - mod.rs `furnish` (3206-3420): `project` at 3227, 3231 and 3265; `strips` at 3253; `layout` at 3269 and 3366; `broken(&laid, &strips(..), home)` at 3367; `Frame { nooks }` at 3240 and 3388;
  - mod.rs `stage_arrange` 3485 (strips loop at 3495); mod.rs 3581, 3593, 3644, 3658;
  - `paint_empty` at mod.rs:4704.
- **Floor pieces stay inside their nook rect by construction.** `pack` (room.rs:831) places them between `from..to`, and `holds` requires `rows ≥ needs`. Hung pieces sit at `top() = floor - lift - rows` (room.rs:954-956), so they are inside the rect too.
  - Poster and clock: `hang: Some(4)` (room.rs:320, 336), on rows `f-6..=f-5`.
  - Window: `hang: Some(1)` (room.rs:355), 2 rows tall, on rows `f-3..=f-2`.
- **Parcels and flaps also come only from nooks.** `doorstep` builds its walls from `nooks.zip(strips(nooks))`:
  - edge test at room.rs:1529-1530: `rect.right() == screen.right()` for the right wall, `rect.x == screen.x` for the left;
  - edge first: `walls.sort_by_key(|&(edge, ..)| !edge)` at room.rs:1532;
  - the flap's `x` is `e.from - 1` or `e.to`, and its rows are `(at.floor - 2, at.floor)`.
- **Placement predicates:**
  - `her_box(x, y)` (room.rs:1801): 5 columns × 5 rows, her sprite plus her floor row.
  - `free(shown, blocked, at, x, y)` (room.rs:1820-1832) tests `s.rect()`, the footprint, **not `s.cover()`**. So `free(None)` over `her_box` does not catch her floor row meeting a floor piece's floor row. The brief's "strict check = `free(None)` = her_box ∩ cover = ∅" is therefore not quite that predicate: it needs a cover intersection, or `free` extended.
  - `fits(buf, at, clear)` (room.rs:1837): the piece stands on unbroken line glyphs, and its footprint is blank and `clear`.
  - `blocked` today is `view.protected` only, in `furnish` (mod.rs:~3218-3225) and in `paint_empty` (mod.rs:4697-4702). The closures `rules::Frame::free` (rules.rs:428-440) and `builds`' `clear` (mod.rs:3937-3943) also go through it.
  - **Trap:** adding the chat pane to `blocked` makes `project` closet a piece in the chat on that very frame (`fits` fails at room.rs:~1370), and `move_off` never fires for it, because a strip that packs never moves. That gives the "held out of the pane, she has nothing to see" branch of the open question, by default. Choosing "it stands where it is until she moves it" needs a chat check outside `blocked`, applied in `layout` callers but not in `project`'s showing pass.

**Is the chat pane ever furnished today?**

| What | In the chat today? | Why |
|---|---|---|
| Bought floor pieces, hung decor (poster, clock, window), parcels and flaps | **No** in the bundled layout | `Strip` cannot name the chat, and `doorstep` walls come from nooks |
| Same, in a custom overlapping layout | **Yes** | See trap 3 in section 5 |
| **Makeshift pieces** | **Yes**, and routinely | `scenes::pulls(buf, terrain, protected)` (scenes.rs:584) scans **every terrain platform**, chat floors included; the call site is mod.rs:2331 |

On the makeshift row: `builds` (mod.rs:3922-3961), then `scenes::builds` (scenes.rs:837-890), puts the piece at `floor: pull.y` under the pull, with `strip: None`. `builds`' `clear` is only `!protected && clear_of_real`. The chat enters only as the resident's 0.1 weight on the pull (`Use(Crumple) => in_chat(..)` at brain.rs:770). Text lives mostly in the chat, so this is the main way a piece ends up there today, and the brief only touches it under "makeshift lines". The rule exempts "props she carries in an act"; a makeshift *piece* is not one of those.

**The door is in the chat today.** Its spot comes from her feet, and her feet are on chat floors during idle visits. In `paint_empty` (section 3), `fit` reads `unkept`, the protection as drawn without the focus pane. So a door in the (usually focused) chat "fits" and stays recorded, and is then hidden by the `standing` filter's `box_meets(protected)` while the chat is in use. That is the mechanism behind the user's *"I noticed this because the door is there"*.

---

### 3. terrain.rs: the frame as floors

- **`Terrain`** (terrain.rs:181-196) holds `width, height, open, calm, graphics, covers, platforms, links`. **It knows nothing about panes, nooks or the chat.** It is read from glyphs:
  - a ledge is any box glyph with a horizontal stroke, or `╱` or `╲` (terrain.rs:120-127);
  - a pole is a box glyph with a vertical stroke.
  So chat borders, day separators and the accordion are floors exactly like nook borders.
- **`Terrain::read(buf, protected, graphics)`** (terrain.rs:208-268) marks every cell inside `protected` as not open and not calm. A ledge inside a protected rect still counts as a floor, but `add_platforms` (270-296) needs her whole body clear above it. It builds `links` (`find_links` 482-560) plus `around_links` (447-480).
- **`around_links`**: from every platform whose end reaches a screen edge (`edge_left && x0 <= HALF`, or `edge_right && x1 >= width-1-HALF`) to every other such platform, as `Route::Around { out, enter }`.
  - In the default layout the left-edge floors are the chat's.
  - The right-edge floors are the nooks' bottom borders; a corner glyph has a horizontal stroke, so it is a ledge.
  - `Osaka::arrive` (osaka.rs:2607-2640) uses the same edge test to walk in.
- **`clear(x, y)`** (terrain.rs:311): her 5×4 body is all open.
- **`furnish(covers)`** (terrain.rs:318-322) stores covers only when `graphics` is set. It does nothing in ASCII.
- **`restful(x, y)`** (terrain.rs:328-370) asks whether she may stay there: her box is calm, and the shared `image()` (terrain.rs:33-63) covers only calm cells, her floor, pieces' floors (`floor`) or pieces' cells (`piece`). **It deliberately passes the cells of pieces in her image.** That is the root-cause predicate. Its only door callers are `door_fits` (mod.rs:4607-4609) and, through it, `door_spot` (4613-4624).
- **`platform_at(x, y)`** (terrain.rs:391): `platforms.position(p.y == y && p.contains(x))`.
- **`image(x, y, covers) -> Image { left, top, right, bottom, with }`** (terrain.rs:33) grows over every cover it meets, transitively. Callers:
  - `paint_empty` (mod.rs:~4757), which decides which pieces go in the door's image;
  - `restful`;
  - the line-art draw paths.
- **Where platforms and panes relate:** only through `Extent.floor == platform.y`.
  - `beauty_at` (mod.rs:228) and `ClockOn::seen_from` (osaka.rs:88-91) map a standing spot to a strip by `e.floor == y && (e.from..e.to).contains(&x)`.
  - Live platform indices are renumbered whenever text splits a floor, so `Strip` is the stable identity (room.rs:625-628).
- **Terrain reads that matter to the door** (all use the gated `view.protected` unless noted):
  - **Arrival** (Return, Dash, Idle) at mod.rs:2026: `Terrain::read(buf, &view.protected, graphics)`, **never furnished**. The `through` closure at mod.rs:2032-2037 calls `door_spot(&terrain, near)`, with `near = middle(size)` when there is no door.
  - **Visit** at mod.rs:2298-2300: `visit.solid(&view.protected)` (mod.rs:581, which adds her moved text), then furnished with `visit.shown` covers.
  - **`paint_empty`** (mod.rs:4657-4784):
    - `fit = Terrain::read(buf, unkept, ..)`, furnished (mod.rs:4710-4711);
    - `door_fits` check and move at mod.rs:4712-4722;
    - `standing` filter, `fits && !box_meets(protected)`, at mod.rs:4724-4730;
    - a second terrain on `view.protected` when that differs from `unkept` (mod.rs:4731-4737);
    - `unkept` is computed at mod.rs:2111 as `whole_glyphs(buf, as_drawn.clone()).protected`.
  - **`nudge` send** at mod.rs:2769. **Leaving** at mod.rs:2137 always reads with `true`.
- **`middle(size)`** (mod.rs:4600) is `(w/2, h)`, the screen's foot at the middle. In the default layout `w/2` is the boundary between the chat and the right column, so the nearest floor can be in either.
- **`box_meets(rect, (x, y))`** (osaka.rs:191-196) is her box meeting a rect. It is the natural test for "the door's box meets the chat".

---

### 4. Runtime layouts (docs/ui-layouts.md; ui/layout/assets/)

- **There is one bundled app layout**, `templates/app.xml`:
  ```
  column#app[ column#main[ row#panes(resizable)[ column#chat-column[chat, subtitles?] | column#right-column[series, users, playlist] ], health ], status, keybar ]
  ```
  From style.css: `#chat-column, #right-column { flex-basis: 50% }`, `#series 34%`, `#users, #playlist 33%`, health 1 row, status 3 rows, keybar 1 row, no padding on `#app`.
  - The **chat spans the left screen edge** (x = 0). Its right edge is the split.
  - Series, Users and Playlist are stacked, **each with its right wall at `screen.right()`**. Each one's left wall is `x == chat.right()`, inside no pane but at no screen edge.
  - Health, status and keybar span the full width below. Status and keybar are protected; health is a 1-row slot.
  - `subtitles` (when `separate-subtitles` is set) sits under the chat in the chat column. It is not a nook, not the chat and not protected.
  - Drag resizing keeps a 10% minimum share "where it fits", so the right column never collapses.
  - **In the bundled layout the door wall is always a nook's right wall at the screen edge. It is never in the chat and never needs the open question.** A nook and the chat are disjoint in flex layouts, so any door space inside a nook's strip is outside the chat by construction. In disjoint layouts, the chat matters only for doors that do not stand on a strip: the no-home chooser, the strict fallback, and today's from-her-feet spots.
- **Custom layouts are user-editable at runtime.** Slots: `chat, subtitles, series, users, playlist, health, status, keybar`. They can be reordered with row, column or grid, hidden with `display:none` (which empties the rect, and empty nooks are filtered), padded or given margins, and **overlapped** with grid placement. The overlap is tested at ui/app.rs:4003, which puts `users` and `chat` in the same grid cell and asserts that the last painted pane gets the click. `idle_view` does not check nooks against `panes.chat`.
- **The open question, in concrete terms.** Her strips' only screen-edge wall is unusable or in the chat in these real cases:
  1. **No nook touches a screen edge.** Examples: `row[chat | column(series, users, playlist) | subtitles]`; `row[subtitles | nooks | chat]`; `#app` or `#panes` with horizontal padding or margin; nooks inside a centred column. Then no wall is at the edge, `doorstep`'s edge sort is moot, and `door_place` must take a non-edge nook wall, which is outside the chat when the layout is disjoint. The only edge floors there are the chat's or subtitles' (`around_links`), which matters for work's screen-edge exit.
  2. **A nook overlapping the chat** (the grid overlap). Every wall of that nook, its door space and all its pieces are in the chat. This is the only layout where "her strip's only screen edge is in the chat pane" is literally true.
  3. **No nooks at all** (all three hidden). There are no strips, so it is the no-home path. Floors exist only in the chat, subtitles and health, so the no-home door chooser has nowhere outside the chat except subtitles or health floors.
  4. **Chat on the right** (columns swapped). The nooks' left walls are at `screen.x`. This works, mirrored.
- **Headroom on short terminals.** `#panes = H - 5`, split about 34/33/33. A strip needs `rows = height-2 ≥ 4` for her and for the door space (4 rows plus the floor), so each nook must be at least 6 rows: that needs `H - 5 ≥ ~18`, i.e. **H ≥ about 23** in the default layout.
  - At `MIN_HEIGHT = 18` (mod.rs:241-242) the nooks are about 5/4/4 rows, so `rows` is about 3/2/2. No strip holds the door space, and she cannot stand inside a nook.
  - **The yield path is the normal path on short terminals.**
  - Width is never the limit: nooks are 50% of `MIN_WIDTH 60` = 30 columns, so an `Extent` is 28 columns, minus 6 for the space.

---

### 5. Test fixtures that collide with the new rule

1. **`tests.rs:19-32` `view()`** hard-codes `chat: Rect::new(0, 0, 30, 20)`. **`nooks(w, h)`** (tests.rs:62-76) makes `Nook::List` the left box `Rect(0, 0, w/2, h-3)`, which **overlaps that chat rect**. `long_visit_of` (tests.rs:759, view built at about 807-811) and `her_days_never_touch_what_is_protected` (tests/away.rs:1232) own pieces on `nook[at]`, List included.
   - Extending the property with "no piece meets `view.chat`" fails trivially until the fixture changes: make the chat the left box and drop it from the nooks, or derive the chat from `rooms()`.
   - `nooks(` is used 34 times across tests.rs, tests/away.rs, rain.rs, dash.rs and golden.rs.
2. Tests that use `IdleView { .., ..Default::default() }` get an empty chat.
3. Tests using `chat: panes[0].1` (tests.rs:6507, 10611, 10829, 11727, 12100) take the chat from a pane list. Check each one's nooks for overlap.
4. Real-UI tests (`real_ui`, `stage_ui`, `chatty_ui` in stage.rs, `ui.idle_view` at tests.rs:997) use the bundled layout: chat on the left, nooks on the right, disjoint. These are the honest fixtures for the chat property.

---

### 6. Brief claims in this area that are wrong or stale

**Line numbers:**

| Item | Brief | Now |
|---|---|---|
| `door_fits` | mod.rs:4365-4367 | mod.rs:4607-4609 |
| `door_spot` | 4371-4381 | 4613-4624 |
| `paint_empty` door move | 4466-4476 | fn at 4657, move at 4712-4722 |
| `standing` filter | 4478-4484 | 4724-4730 |
| `terrain::image(..).with` in `paint_empty` | 4508-4512 | about 4757-4760 |
| `draw_door` | 4619 | 4786 |
| `draw_flap` | 3901-3935 | 4143 |
| `out_by_door` | 1691-1703 | 1869 |
| `DoorAt` / `Out` / `Empty` | — | 691 / 700 / 713 |
| Arrival terrain | mod.rs:1840 | 2026 |
| Arrival `door_spot` | 1847-1851 | the `through` closure at 2032-2037 |
| Arrival waits (no room) | 1872-1875 | about 2063-2080 |
| `clock_on` / `beauty_at` | 154 / 167 | 211 / 228 |
| `broken`'s caller | 3141 | 3366-3367 |
| Set-down pinning | 3047 | inside `furnish` 3206-3420 (pins at about 3253-3269) |
| `stage_arrange` | 3269 | 3485 |
| `builds`' `clear` | about 3696 | 3937-3943 (fn at 3922) |
| `Extent` | 738-743 | 736-741 |
| `strips` | 797 (`to` at 803-806) | 795-813 (`to` at 803) |
| `Home` | 1119-1121 | 1120-1122 |
| `move_off` | 1377 | 1378 |
| `project`'s `fits` | 1366-1370 | about 1367-1374 |
| `hung_clear`'s `meets` | 1298-1303 | about 1314-1319 |
| `top()` | 956 | 954-956 |
| `hang: Some(4)` | ~321/337 | 320/336 |
| Window hang | ~355 | 355 |
| `doorstep` edge test | room.rs:1519-1523 | 1529-1530 (fn at 1517) |
| `doorstep` sort | 1525 | 1532 |
| `doorstep` flap | 1536-1569 | about 1595-1608 |
| `her_box` | 1773-1787 | 1801 |
| `free` | 1792-1807 | 1820-1832 |
| `Terrain::restful` | 328-361 | 328-370 |
| `restful`'s `calm \|\| floor \|\| piece` | 356-358 | 367-369 |
| `Chances.clock` | osaka.rs ~70 | ~68 |
| `hung_pieces_clear_every_standing_piece` | room.rs:2383 | 2426 |

Unchanged: `furnish` terrain.rs:318-322, `against_wall` rules.rs:324-327, `Frame::free` rules.rs:428-440, `Before::new` rules.rs:~465.

**Substance:**
- **§4, "strict check = `room::free(shown, blocked, None, ..)` over `her_box` = her_box ∩ cover = ∅".** `free` tests `s.rect()` (the footprint), not `cover()`, so her floor row meeting a floor piece's floor row passes. Use a `her_box(..).intersects(s.cover())` test, as `hung_clear`'s `leans` does at room.rs:~1328.
- **§5.8 scope.** It lists makeshift pieces among the sites but does not say they are **the one kind in the chat today**: pulls, and therefore builds, come from every platform, chat floors included, with only a 0.1 weight for a resident and none for a visitor. Bought pieces, hung decor and parcels **cannot** be in the chat today, except in an overlapping custom layout.
- **§5.8, "her door is never in the chat pane", is met structurally for any door on a nook strip in a disjoint layout.** The chat check is only needed in the no-home chooser, in `door_place`'s fallback (which scans terrain platforms, chat floors included), in overlapping layouts, and, today, in every from-her-feet spot.
- **The brief treats "the chat pane" as one known rectangle without saying where it comes from.** It is `IdleView.chat`, the whole `#chat-pane` column, which is never an `Option` but can be empty. The resident-only `Chances.chat` is the wrong source for this rule.
- **§5.4 and the open question assume the edge-first chooser normally finds an edge wall.** In the bundled layout it always does (the right wall of every nook). It finds none only in custom layouts (section 4, cases 1-3). Its practical failure in the bundled layout is height: below H ≈ 23 the door space never fits, so it yields (section 4, headroom).
- **§2 item 5 and §5.6, "each arrival builds its terrain furnished with `ledger.home.project(..)` covers".** The arrival at mod.rs:2026 runs before `take_home`'s pieces are re-projected, and `begin_visit` is called with that unfurnished terrain. Builders must furnish it there, or do `door_place` before `Terrain::read`.
- **`Nook::List`'s doc "(series), when short"** is stale (room.rs:400): the List nook is passed at any height.

<!-- section: art -->
# Code map: art wiring for the side-on door (door batch)

Paths are relative to `dessplay/src/ui/houseguest/` unless noted. Line numbers are from HEAD `057311b1` (2026-10-08). I edited nothing in the repo.

## 0. Status of `door/art/worktree.diff`: already applied, nothing to apply

- `git apply --check docs/proposals/2026-10-02-houseguest-mind/door/art/worktree.diff` **fails**: `art.rs` hunk 1 doesn't match, `wall-door.svg` "already exists in working directory", and `sprite.rs` hunk 1 doesn't match.
- `git apply --check -R` (the reverse) **succeeds**, with offsets: sprite.rs −8 lines; art.rs hunk 3 −97 and hunk 4 −189.
- **So the diff is already in the tree.** Commit `8eaa25c1` ("approved art for her side-on front door … (not wired yet)") put all of it in: art.rs +632, art/wall-door.svg +113, sprite.rs +34.
- **Builders must not re-apply it.** What's there is dead outside tests (`#[cfg_attr(not(test), allow(dead_code))]` on `WallDoor` and `render_wall_door`, art.rs:1120/1220, and on `sprite::WALL_DOOR`, sprite.rs:478).
- Stale in snippets.md: "worktree.diff … against `1e44b050`" / "for review, not wired in". It's committed now, and only the wiring remains.

## 1. What already exists (art.rs, sprite.rs, wall-door.svg)

### art.rs
- `const WALL_DOOR: &str = include_str!("art/wall-door.svg")` (:16).
- `pub(super) enum WallDoor` (:1121), derives `Clone, Copy, Debug, PartialEq, Eq, Hash`:
  - `Shut { flap: u8, away: bool }`: `flap` is degrees (0 = shut); `away` adds the slippers and the card.
  - `Ajar`
  - `Open`
  - `Post`: the front post and threshold alone, drawn over her in the doorway.
- `const WALL_DOOR_FRAME: (f32, f32) = (6.0 * CELL_UNITS.0, 4.0 * CELL_UNITS.1)` is 120×168 units (:1140). `const WALL_DOOR_SHIFT: f32 = 40.0` (:1141).
- `fn flap_plate(degrees: u8) -> String` (:1146), private. It builds the swung plate's SVG: hinge `[(49,83),(66,81)]`, LONG 78, THICK 5.
- `fn wall_door_body(door: WallDoor) -> String` (:1175), private. Parts per state:
  - `Shut`: `wd-shut` + (`wd-flap-shut` | `wd-flap-hole`) + `wd-frame`, then `flap_plate(flap)` if flap > 0, then `wd-slippers` + `wd-tag` if away.
  - `Ajar`: `wd-beyond`, `wd-ajar`, `wd-frame`.
  - `Open`: `wd-beyond`, `wd-open`, `wd-frame`.
  - `Post`: `wd-post`.
- **The plate is drawn inside the Shut body**, so it ends up under any parcel drawn later. That's why the sheet draws it a second time over the parcel (§6).
- `fn wall_door_scene(body, facing, line) -> String` (:1206), private:
  - Facing::Left mirrors with `translate(w 0) scale(-1 1)`.
  - It wraps the body in `translate(40 0)`.
  - The wall's line is x = 70 in part units, which is 110 in the frame: the middle of frame column 5.
- `pub(super) fn render_wall_door(door, facing, line, width, height) -> Option<RgbaImage>` (:1221). It calls `rasterize(scene, WALL_DOOR_FRAME, w, h)`.
  - Facing::Right means a right wall. The frame's columns are `to-5..=to` at a right wall, and `to..=to+5` mirrored at a left wall.
- `pub(super) fn rasterize(svg, (fw,fh), w, h)` (:1787): uniform scale, centred horizontally, bottom-aligned.
  - With 9×19 cells, `6·9 / 120 = 0.45` is the binding scale, so the door is about 75.6 px tall with its bottom at the feet line. That matches "from the lower half of f-4 down to the floor line".
- Lint `the_wall_door_keeps_to_its_columns` (:3869) pins which frame columns get ink. **Use these to crop the image** (§4.3):

  | State | Columns, right wall | Frame cols |
  |---|---|---|
  | `Shut{0,false}` | `to-1..=to` | 4..=5 |
  | `Shut{0,true}` | `to-3..=to` | 2..=5 |
  | `Shut{90,_}` | `to-5..=to` | 0..=5 |
  | `Ajar` / `Open` | `to-3..=to` | 2..=5 |
  | `Post` | `to-1..=to` | 4..=5 |

  At a left wall the same columns mirror.
- Ignored `wall_door_sheet` (:3430). It holds the **test-only** helpers that snippets says the builder will want in production:
  - `lean(item, lead)` (:3615): the flap's angle as it rides on the parcel. It reads `PARCEL` (:1465, `(100.0, 80.0)`, private) and the parcel's lid at `168 − 64·scale`.
  - The per-pixel wall clip, in the sheet's `put` (:3703-3720). Right wall: drop pixels with `sx >= lx`. Left wall: drop pixels with `sx < lx + s`. Here `lx = line_x(at) = w*at + w/2 - s/2` (:3702).
  - The parcel at a half-column offset `xh` (:3784ff): pixel x = `xh * w / 2`, clipped while i < 3.
  - The plate drawn a second time over the parcel (`Door::Plate`, :3671). It's built from `wall_door_scene(&flap_plate(flap), ..)` (:3753-3758), which uses private functions.
- The door in space is unchanged: `DoorFrame` (:1073), `door_scene` (:1081), `render_door` (:1102). Its canvas is her 100×160 (`CANVAS_W/H`, :23-24).

### wall-door.svg
- Part ids: `wd-day` (a gradient, :20), `wd-a-shut` (rejected), `wd-frame`, `wd-post`, `wd-beyond` (:52), `wd-shut` (:61), `wd-flap-shut`, `wd-flap-hole` (fill `url(#wd-day)`, :76), `wd-ajar`, `wd-open`, `wd-slippers`, `wd-tag`.
- **The daylight is hard-coded:** `wd-beyond` and `wd-flap-hole` both fill with the single gradient `#wd-day`. Following the window's sky needs new art (§7).

### sprite.rs
- `pub(super) const WALL_DOOR: [[&str; 4]; 4]` (:479). Its rows are `f-4..=f-1` of columns `to-3..=to` at a right wall. Index: 0 shut, 1 flap open, 2 ajar, 3 open.
- `fn mirror(c)` (:453) maps `/ \ < > ( )` only. **`[` and `]` are not mirrored yet**, and snippets says to add them for the left wall. `mirror` is private, so a `wall_door_cells` sibling in sprite.rs can use it.
- `pub(super) fn door_cells(frame: usize, facing) -> Vec<SpriteCell>` (:489) is the door-in-space sibling to copy. It assumes her 5×4 box centred on her anchor: `dx = col - WIDTH/2`, `dy = row - HEIGHT`.
- Lint `the_wall_door_is_ascii_and_keeps_to_its_columns` (:780).
- There's no ASCII Away cue.

## 2. Rendering looks: graphics.rs

- `pub(super) enum Look` (:65), derives `Copy, Hash, Eq` and is used inside the cache `Key`. Variants: `Pose(Pose,Face,InSight)`, `Wave(bool,InSight)`, **`Door(art::DoorFrame)`** (:71), `Prop(Furniture, art::Layer)`, `Parcel(Furniture, bool)`, `Tv`, `Film`, `Piece(Furniture, PieceState)`, `Scrap(..)`.
- `Look::size(self) -> (i32,i32)` (:90): `Pose | Wave | Door => (WIDTH, HEIGHT)` = (5,4); `Parcel(item,_)` is the item's footprint.
  - **New:** `Look::WallDoor(..)` needs size (6,4), or a cropped width (§4.3).
- `Look::render(self, facing, w, h)` (:108) dispatches; `Door(frame) => art::render_door` is at :110.
  - **New arm:** `art::render_wall_door(..)`, plus a plate-only render.
- `pub(super) struct Layer { look, facing, at: (i32,i32), standing: bool }` (:136).
  - `bounds()` (:146): `left = at.0 - width/2`, `top = at.1 - height`, plus one floor row when `standing`.
  - **No sub-cell offset and no per-layer clip today.**
- `struct Key` (:154): `layers: Vec<(Look, Facing, (u16,u16), bool)>`, `size`, `lines`, `clip` (the on-screen visible rect, shared by the whole image), `cell`.
- `pub fn paint_layers(&mut self, buf, layers: &[Layer], open: &dyn Fn(i32,i32)->bool) -> Option<Rect>` (:503) paints **one image** over the union of the layers' boxes.
- `fn key(..)` (:568):
  - It crops the union to `buf.area` (that's how off-screen cells are dropped).
  - Every cell in the union must pass `open(cx,cy)` unless it's a standing layer's floor cell. Floor cells must be non-blank stroke glyphs.
  - **If any cell fails, the whole image is refused (`None`)**, and the caller paints nothing: her art vanishes that frame.
  - Lines under the image are recorded (redrawn); other non-blank text is recorded and **derezzed** (`draw_alien`).
- `fn compose(&self, key)` (:735):
  1. Draws the lines and aliens first.
  2. For each layer, renders at width `cw*width` and height `feet` (standing: `ch*height + line.offset + line.thickness`).
  3. `image::imageops::overlay`s it at `(ox*cw, oy*ch)`.
  4. Crops to `clip`.
- `LineGeometry::for_cell` (:179): thickness `max(h/16, 1)`. `draw_glyph` (:276): the vertical stroke's x is `vx = (cw - t)/2`.
  - The **wall line's pixel column in compose is `to*cw + (cw - t)/2`.** It equals the sheet's `line_x` at 9×19 (4 px), but use LineGeometry, not the sheet's formula.
- Test hooks `take_looks()` and `canvas()` record the looks of placed layers. Tests match `Look::Door(_)`: tests.rs:3740, tests/window.rs:240.

## 3. The door in space and her in mod.rs (`mod placement`, :273-470)

- `Placement { x, y, facing, standing }` (:300); `Placement::of(osaka, now)` (:309) uses `osaka.x/y` and is `None` when `osaka.hidden(now)`.
  - **Her doorway offset `d` (columns toward the wall) has nowhere to live.** It needs a field, or a constructor taking `d`, plus a clip flag carried into the `Layer`.
- `Door { frame: DoorFrame, x, y, facing, standing }` (:347):
  - `Door::of(osaka, now)` (:357) takes **her** x/y/facing.
  - `Door::closed(x, y, facing)` (:370) is the Away door.
  - `layer()` (:381) gives `Look::Door(frame)` at `(x, y)`.
  - `cells()` (:391) gives `sprite::door_cells(frame as usize, facing)` offset by `(x, y)`.
  - **The wall door needs its own spot** (the space's door columns, not her feet) and its own `cells()` from `WALL_DOOR` that includes the wall column `to`.
- `Figure { her, door }` (:409): `Figure::of` (:417), `door_alone` (:424). `BoxArt { figure, with }` (:455).
- The goldens trace `visit.image…figure.her()` (tests/golden.rs:56), so changing what `Placement` holds changes the golden traces.

### Beats (osaka.rs)
- `struct DoorBeat { door: Option<DoorFrame>, her: bool, there: bool, ms }` (:1230). `const DOOR: [DoorBeat; 13]` (:1239). `door_beat(elapsed, gap)` (:1322).
- `Osaka::hidden` (:9056) is `!beat.her`. `Osaka::door(now) -> Option<DoorFrame>` (:9067).
- **The pose during a door is always `Pose::Stand`** (osaka.rs:9857-9861; the face is Curious, or Pleased once `there`).
  - The wall door's table (snippets) needs per-sub-beat poses (`Side`, `Walk(2)`, `Walk(0)`, `Stand`) and offsets `d ∈ {0,2,4}` within beats 2 and 10.
  - It also needs `away` true from beat 6 to beat 10. **Beat 6 has `door: None` today**, while the wall door must show `Shut{away:true}` during the gap.
  - So `DoorBeat` needs a wall-door variant, or a mapping `DoorFrame → WallDoor` plus a gap override.
- Wake schedule: `first_due` uses beat ends (osaka.rs:2956). Sub-steps in thirds inside beats 2 and 10 (700 ms and 600 ms) need their own wake points, or nothing repaints mid-beat.

## 4. Composition paths that draw her door

### 4.1 Visit, line art: `draw_art` (mod.rs:4482-4598)
- Caller (mod.rs:2439-2444): `figure = Figure::of(..)`, and `drawn = terrain::image(osaka.x, osaka.y, &covers).with` splits `with` from `apart`.
  - `terrain::image` (terrain.rs:33) assumes **her** box: `x±HALF`, `y-HEIGHT..y+1`. The wall door's box (6 cols, offset one column wallward of her box) and her doorway boxes (`to-3..=to+1`, `to-1..=to+3`) need their own union.
- Layer order today (:4581-4586): pieces back → `door.map(Door::layer)` → her pose → pieces front. Then one `paint_layers(.., &|x,y| terrain.open(x,y))`.
  - Wall door order (snippets): door (leaf behind her) → her (clipped) → `Post` layer after her, while `d > 0`.
- Frozen body (:4509-4542): her ASCII glyphs as burst cells, else `door.cells()` with `DOOR_INK`.

### 4.2 Visit, ASCII: `draw` (mod.rs:3022-3115)
- Door cells come from `sprite::door_cells(frame, osaka.facing)` at her x/y. Her sprite wins over the door cells (:3051-3057). Everything is then filtered by `terrain.open` (:3080) and written with `put`.
- New pieces:
  - Doorway clip: drop her cells at `x >= to` (right wall) or `x <= to` (left wall).
  - Wall-column glyphs must go through the `draw_flap` rule (§5): only over `│`/`┃`, skipping protected cells.
- `DOOR_INK` = LightMagenta (:3016).

### 4.3 Away: `paint_empty` (mod.rs:4657-4779) → `draw_door` (mod.rs:4786-4871)
- `paint_empty`:
  - Door-spot logic: :4705-4720.
  - `standing` filter (:4722-4728) uses `osaka::box_meets(rect, (x,y))` (osaka.rs:191), which is **her 5×4 box**.
  - `drawn = terrain::image(door.x, door.y, &covers).with` (:4752-4755), also her box.
  - It then calls `draw_door(.., Door::closed(..), with, ..)` (:4766-4771).
- `draw_door`:
  - ASCII branch (:4802-4821): `put(buf, x, y, glyph, DOOR_INK)` for each `door.cells()` that passes `terrain.open`. **`put` overwrites any non-image cell, so a wall glyph would be overwritten blindly.**
  - Line art (:4823-4870): pieces first, then `layers.push(door.layer())` (:4865), then `paint_layers(.., terrain.open)`.
- **Trap: text under the door for hours.**
  - In line art, `Terrain::open` accepts narrow text (terrain.rs:270, `free && (!graphics || quiet || narrow)`). A full 6×4 (+floor) image over the space would therefore derez pane text for 4.5 h.
  - The `Hidden` test checker allows at most `HIDDEN_MS = 10_000` (tests.rs:555, asserted ~:620).
  - Today's door avoids this only because `door_fits` uses `restful` (calm = blank or lines).
  - Fix options:
    - (a) Make the image span only the inked columns (§1 table). For example, `Look::WallDoor { door, cols }` renders the 6-col frame and crops to the last `cols` columns, or the first `cols` for a left wall.
    - (b) Have `door_place` require the drawn cells to be `calm` (blank or lines), or else yield.
  - The same applies in ASCII: `put` writes over text.

## 5. The flap (deliveries), rains, Frozen cells

- `struct room::Flap { x, rows: (i32,i32), side }` (room.rs:652).
- `Home::doorstep` (room.rs:1517-1611) returns it at (:1604-1608): `x = e.from-1` (left) or `e.to` (right), with `rows = (floor-2, floor)`, i.e. rows `f-2..=f-1` (half-open).
- `Extent { from, to, floor, rows }` (room.rs:736). `strips()` (room.rs:797): `from = rect.x+1`, `to = rect.right()-1` (the right wall's own column), `floor = rect.bottom()-1`. **The left wall's column is `from-1`, not `from`.**
- `Visit.flap: Option<(room::Flap, u64)>` (mod.rs:500). It's set on delivery at mod.rs:3326 (bought parcel) and :3356 (wall clock).
- Expiry: `flapped = visit.flap.take_if(now >= since + FLAP_MS)` (mod.rs:1684). Wake: `since + FLAP_MS` only (mod.rs:1972-1975). `const FLAP_MS: u64 = 800` (mod.rs:4138).
  - **The parcel slide's beats (0/150/350/500/650 ms) need a wake and a repaint at each boundary.** Today there's exactly one wake, at 800.
- `fn draw_flap(buf, flap, age, protected) -> Vec<Frozen>` (mod.rs:4143-4180):
  - It returns at once if `age >= FLAP_MS`.
  - Each wall cell: skip it if protected or not `│`/`┃`; otherwise `set_char('╲'|'╱')` with ink = that cell's fg, recording `Frozen { under: original, burst: false }`.
  - Called at mod.rs:2470-2477, before `draw_props`.
  - This is the precedent for every wall-column cell the side-on door paints, in both modes. Snippets keeps it as the fallback when the space yields.
- **Today the delivered parcel is drawn at rest at once.** It's in `visit.shown` (boxed), drawn by `draw_props` or `draw_art` as `Look::Parcel(item, unpacking)` (`piece_look`, mod.rs:3998) through `prop_layer` (mod.rs:4094-4105, `at = (left + cols/2, floor - lift)`).
  - During the slide, the at-rest draw must be suppressed and replaced by one image: `[door Shut{flap: lean}] → parcel (offset, clipped) → plate(flap)`.
  - These must be **one** `paint_layers` call. Two images over the same cells cut each other out, and her box image may also meet the space if she's standing there.
- `door_rain` (mod.rs:2805-2832) filters `empty.painted` to `Door::closed(..).cells()`, the 5×4 face-on glyphs. It must take the wall door's cells, including the wall column and any slipper cells.
- `Frozen` (dissolve.rs:40): `{ x, y, glyph, ink, under: Cell, face, burst }`. Burst cells are hidden until the rain, then show as noise; non-burst cells hold their glyph.
  - For wall-column cells, `under` must be the border glyph so the rain restores the line. `draw_flap` already does this.

## 6. What each wiring item needs

1. **A new look for the wall door.** Add `Look::WallDoor(art::WallDoor[, cols])` beside `Look::Door` in graphics.rs:65, with size `(6,4)` or `(cols,4)`, and a render arm calling `art::render_wall_door`.
   - **`Layer.at` math:** for a 6-wide look, `left = at.0 - 3`. A right wall wants left `to-5`, so `at = (to-2, f)`. A left wall wants left `to` (the wall column), so `at = (to+3, f)`. Set `standing: true`.
   - Floor row: the cells `to-5..=to` on row `f` must be strokes. The corner `┘` or `└` in column `to` qualifies.
   - Remove the `cfg_attr(..dead_code)` attributes on `WallDoor`, `render_wall_door` and `WALL_DOOR`.
2. **Clipping her image at the wall's line.** `Layer` and `Key.layers` gain a clip, e.g. `Option<(Side, i32 /*to*/)>`. In `compose`, zero the alpha of body pixels beyond `to*cw + (cw-t)/2` (right wall: `>=`; left wall: `< that + t`) before `overlay`.
   - **Also clip `bounds()` to the wall.** Her doorway box reaches `to+1..=to+3`.
     - At the screen's edge, `key` crops it.
     - At a non-edge wall (a fallback), those cells belong to the neighbouring pane. `open()` is false there if that pane is protected, so **the whole image is refused** and she and her door vanish.
   - In ASCII, drop her cells at and beyond `to`.
   - She gets `Post` as a second layer after her, while `d > 0`.
3. **The parcel at an offset, clipped.** `Layer` needs a sub-cell x offset: the slide's leads are half columns (`xh * cw / 2`; `half = -3-cols`, `out = -4-2·cols`, `rest = -13-2·cols`).
   - Use `Look::Parcel(item, false)` with that offset and the same wall clip. Its bounds must include the partial cells and stop at the wall.
   - Move `lean` (art.rs:3615, test-only) and `PARCEL` access into production art.rs.
4. **The flap plate drawn over the parcel.** Add `WallDoor::Plate(u8)`, or a `render_flap_plate(degrees, facing, ..)` built from `wall_door_scene(&flap_plate(d), ..)`, as the third layer.
   - `Shut{flap>0}` already draws the hole (`wd-flap-hole`) and the plate under the parcel; keep it.
   - ASCII: `WALL_DOOR[1]` for the 650 ms. The parcel's ASCII stands at its spot from the start.
   - A large parcel's plate reaches `to-5` (frame col 0), so it covers the whole space. If she's standing at her spot during a delivery, her box and the slide must share one image.
5. **The slippers and card cue.** These are `WallDoor::Shut { away: true }` (`wd-slippers`, `wd-tag`); the ink spans `to-3..=to`.
   - It must show from beat 6 (the gap, `door: None` today) through beat 9, and in `paint_empty` and the cold start.
   - There's no ASCII cue (snippets).
6. **The doorway's sky.** Mirror how the window gets its sky:
   - `piece_state` (mod.rs:4059-4089) maps `World.time: Option<u16>` to `PieceState::Sky(art::Sky::at(minute))` (mod.rs:4072).
   - `time = self.time_of_day(now)` (mod.rs:1931-1934, computed at :2083) is `None` when unfed, which means the window shows a plain day sky. `Osaka::sky_at` (osaka.rs:5996) also defaults to `Day`.
   - `Sky` (art.rs:1385, `Copy + Hash`) and `Sky::at` (art.rs:1408) exist.
   - Thread `Sky` into `WallDoor::{Ajar, Open, Shut{flap>0}}` (or as a render argument carried in the `Look`), and make `wd-beyond` and `wd-flap-hole` pick per-sky gradients.
   - **New art is needed:** the SVG has only `wd-day`. The window's skies are `window-sky-*` groups in props.svg:444-505, clipped to `#window-glass`, so they can't be reused directly.
   - Wakes: `next_quarter` fires only when `tells_time(shown)` (mod.rs:4048). The door shows sky only during short beats, so this is probably fine; say so in the commit.

## 7. Brief and snippets claims in this area that are wrong or stale

| Claim (source) | Correct now |
|---|---|
| brief §5.3: canvas 40×160, 2×4 cells, her units | **120×168, 6×4 cells at a piece's scale** (`WALL_DOOR_FRAME`, art.rs:1140); the wall line at part x = 70, shifted +40 |
| brief §5.3: open question whether a 1-col door reads; 2-col fallback | Settled: **B** (2 columns: face in `to-1`, front post over `to`). A survives only as `wd-a-shut` for the sheet |
| brief §5.3 ASCII: "≤2 cols × 4 rows shut, pink `|`, `o` on f-2" | `WALL_DOOR` is 4 cols × 4 rows, last col `|` (wall); shut uses cols 2..=3 (`/`, `o`, `#`, `#`) |
| brief §5.2: side-on open leaf "at most 3 columns `to-3..=to-1`"; shut door "`to` maybe `to-1`" | Shut `to-1..=to`; ajar/open/away `to-3..=to`; the flap plate reaches `to-5` |
| brief §5.7 and Q1: `doorstep` skips the door's wall (A) | Superseded: parcels come **through the door's flap** (B); the parcel rests at the narrowed extent's `Anchor{side,0}`, trailing edge `to-7`; `draw_flap` remains only as the fallback when the space yields |
| brief §7.2: side-on door is "next batch" | plan.md: **in this batch** |
| snippets: "the sheet's helpers the builder will want in art.rs" | `flap_plate` is already production code (private, art.rs:1146); `lean` (:3615) and the clip (`put`, :3703) are still test-only inside `wall_door_sheet` |
| snippets: worktree.diff is "not wired in", "against 1e44b050" | Already committed in `8eaa25c1`; `git apply --check` fails because it's applied (reverse check passes) |
| brief: `draw_flap` mod.rs:3901-3935 | **mod.rs:4143-4180** (`FLAP_MS` :4138) |
| brief: `standing` filter mod.rs:4478-4484 | **mod.rs:4722-4728** |
| brief: `paint_empty` re-place mod.rs:4466-4476 | **mod.rs:4710-4720** (fn at :4657) |
| brief: `door_fits` mod.rs:4365-4367; `door_spot` 4371-4381 | **:4607-4609; :4613-4624** |
| brief: `terrain::image(..).with` mod.rs:4508-4512 | **mod.rs:4752-4755** (visit path :2442) |
| brief: `draw_door` pushes the door layer at mod.rs:4619; ASCII at :4564ff | **:4865; ASCII :4802-4821** |
| brief: `put` cells.rs:53 | **cells.rs:57** |
| brief: `door_rain` mod.rs:2603-2625 | **mod.rs:2805-2832** |
| brief: `Look::Door` graphics.rs:69-110 | enum :65, variant :71, render arm :110 |
| brief: `render_door` art.rs:1101 | :1102 |
| brief: `door_cells` sprite.rs:475 | :489 |
| brief: `Door::closed` mod.rs:308 | :370 |
| brief: `door_sheet` art.rs:3025-3082 | :3344 |
| brief: `doorstep` room.rs:1536-1569 (edge sort :1525) | fn :1517, Flap returned :1604-1608, edge sort :1531 |
| brief §2: `restful` "terrain.rs:328-361, piece clause 356-358" | :328-358 (still accurate) |
| brief §5.2: flap "rows f-2..f" | Half-open; the cells are `f-2..=f-1` (unchanged) |

## 8. Tests and goldens that would move

- **Goldens** (tests/golden.rs, hashes per seed × mode):
  - `golden_school_morning` (:983), `golden_home_from_school` (:997), `golden_dash_home` (:1011): the door's spot, the beats, her sub-steps and the Away cue all change.
  - `golden_stage_room` (:885) and the unfed stage table (:1189): `Scene::Parcel` at 255 s goes through the door flap, with new beat wakes.
  - Any stage `dash_through` (osaka.rs:~8963) and errand runs, if the stage uses the space.
  - `golden_errand` (:927) if errands use her door.
  - The trace line includes `figure.her()` (:56), so a `Placement` shape change moves every golden that records it.
- **tests.rs:**
  - `open_flap` (:642) and `Hidden::check` (:557-630) exempt only `│/┃ → ╲/╱` at the flap's cells. They need the wall door's wall-column cells (ASCII `|`, and in line art the image cells over a line, already allowed) plus the slide's cells. Callers: :830, :2544, :2619, :5507, :5586, :5834, :6057, :6177, tests/clock.rs:561.
  - `a_parcel_comes_in_through_a_flap_at_the_screens_edge` (:4971) asserts `["╲","╲"]` in the users pane's left wall and the TV at `users.x + 1`. Both change with the door's wall and the narrowed extent.
  - `a_goodbye_while_she_is_out_shows_nothing_of_her` (:3634) uses `sprite::door_cells` (:3669) and `Look::Door(_)` (:3740).
- **tests/away.rs:** `closed()` / `door_shows` (:56ff) build `Door::closed(..).cells()`; `her_days_never_touch_what_is_protected` (:1232).
- **tests/window.rs:** `her_door_at_her_sofa_keeps_her_window_behind_it` (:207, matches `Look::Door(_)` at :240) is to be rewritten (brief test 6). The `Look` match at :33 needs the new look.
- **Lints in art.rs and sprite.rs:** `the_wall_door_keeps_to_its_columns` (:3869) and `the_wall_door_is_ascii_and_keeps_to_its_columns` (sprite.rs:780) must still pass. If `[`/`]` mirroring is added, extend the sprite lint to the left wall.
- **Image budget:** tests/census.rs and `CACHE_LIMIT` (graphics.rs:59, sized by `image_census`) get new door looks: sky × state × crop, her clipped poses, parcel offsets. Re-run the census.
- **Stillness:** tests/stillness.rs counts `Body::Door` changes (:844-859). Sub-steps inside beats 2 and 10 add seen changes, so check the per-mood budget.
- **The terrain snapshots** (`snapshots/*.snap`) don't involve the door and shouldn't move.

<!-- section: tests -->
## Code map: the houseguest test harness for the door batch

Line refs are from the working copy on 2026-10-08 (`@` = `5cdb69ca`, clean). Paths are relative to `dessplay/src/ui/houseguest/` unless noted. Nothing in the repo was edited.

### 0. Biggest trap: the harness's chat rect overlaps the nooks

- **The fake chat rect.** `tests.rs:20` `fn view(protected: Vec<Rect>) -> IdleView` sets `chat: Rect::new(0, 0, 30, 20)` at tests.rs:27, as a placeholder. Nearly every test builds its view from it.
- **It overlaps the nooks on both standard screens.**
  - `rooms(w,h)` (tests.rs:37) with `nooks(w,h)` (tests.rs:62): List is `(0,0,w/2,h-3)`, which contains the chat rect.
  - `home_screen()` (tests.rs:2264), 100×20: Users is `(0,8,50,9)` and Playlist is `(50,8,50,9)`. The chat rect covers Users cols 0..30, rows 8..17.
  - Production keeps chat and nooks apart: `app.rs:664` builds `nooks` from series/users/playlist only, and `chat: self.panes.chat`.
- **Effect if the chat-pane rule reads `view.chat`.** These all count as "in the chat pane" because of harness geometry alone:
  - away.rs's `HOME` sofa (Users 300).
  - window.rs's sofa at Users 0 (cols 1..10).
  - the parcel against Users' left wall (`a_parcel_comes_in_through_a_flap_at_the_screens_edge`).
  - census `furnished_room()`.
  - golden `home_at` (sofa at Users 50).
  - the `her_days` generator's List and Users placements.
- **The door chooser is affected too.** On home_screen, Users' left wall (x=0) is a screen edge and its space falls inside the fake chat rect, so it would be skipped. Which wall gets chosen therefore depends on this placeholder.
- **Existing disjoint pattern to copy.** Use `chat = nooks(w,h)[0].1` with `nooks: nooks(w,h)[1..]`, as in:
  - `resident_view` (tests.rs:6501)
  - dash.rs:620 (`the_errand_at_school_is_a_dash`) and dash.rs:987
  - golden `errand` (golden.rs:369)
  - tests.rs:10611, :10829, :11727, :12100
- **Who reads `view.chat` for behaviour:**
  - `visit.layer.drop_in(view.chat)` and `osaka.shaken(now, view.chat)`, both on a key press (mod.rs:2197, mod.rs:2201)
  - `Chances.chat`, residents only (mod.rs:2302, via `elsewhere` at osaka.rs:4107/9272)
  - mod.rs:2893
  - Changing the rect is mostly inert for a visitor, but every golden runs `drive` with these views. Re-check after any change.
- **Decision needed before tests are written.** Either fix `view()`'s chat rect (to empty, or a pane disjoint from the nooks) or give the chat-pane property its own disjoint view. Otherwise the "no piece and no door in the chat pane" property fails on the harness itself.

### 1. tests.rs helpers (the 12,737-line parent module; submodules declared at tests.rs:12725-12737)

| Helper | Line | Signature and behaviour |
|---|---|---|
| `view` | 20 | `fn view(protected) -> IdleView`: delay=DELAY, not resident, chat (0,0,30,20), nooks empty |
| `rooms` / `nooks` / `bottom_strip` | 37 / 62 / 81 | 3-pane screen: List left `(0,0,w/2,h-3)`, Users and Playlist stacked right; protected bottom 3 rows. Right panes' right wall = screen right; List's left wall = screen left |
| `paint` / `run` | 86 / 93 | one frame over a clone of `real`; shell loop `next_tick`→`advance`→paint |
| `kitty()` / `kitty_cells` | 537 / 544 | **graphics mode = `guest.set_picker(kitty())`** (Kitty, 9×19 px). ASCII = no picker |
| `Hidden::check` / `check_raining` | 563 / 576 | Her image may cover only blank, line or passing text (≤`HIDDEN_MS`=10 s). Any other change must be her layer or the **flap exemption**: cells in `flap` whose real glyph is `│/┃` and drawn glyph is `╲/╱` (tests.rs:595-601) |
| `open_flap` | 642 | `visit.flap` cells while `now < since + FLAP_MS`. **The parcel flap moving into door B changes its cells and glyphs; extend this helper** |
| `scatter` | 654 | writes text, marks skip cells, `cells::sanitize` |
| `long_visits_never_touch_what_is_protected` | 678 | proptest, `proptest_cases(24)`; `owned` = `(0..4 → Furniture::ALL[i], pane 0..3, along 0..=1000, left)` |
| `long_visits_with_decor_…` | 699 | same, plus Poster/Plant |
| `long_visit` → `long_visit_of` | 732 → **759** | `long_visit_of(guest, graphics, sizes, text, skips, chats, protect, owned, span) -> Result<(), TestCaseError>`; details below |
| `real_frame` | 990 | the real UI's buffer and `idle_view` (production chat and nooks) |
| `terrain_map(_in)` | 1005/1009 | for the insta snapshots |
| `home_screen` | 2264 | 100×20, Users `(0,8,50,9)` + Playlist `(50,8,50,9)`; Users' left wall and Playlist's right wall are screen edges |
| `furnished_home(_with/_in)` | 2466/2471/2485 | `Guest::new(seed)`, kitty always, `cue(Scene::Arrive)`, then `guest.give(item)` + paint for Sofa/Tv/Bed/Desk (+`more`). Placement as delivered |
| `on_saturday` | 2478 | |
| `live_in(_watching)` | 2517/2522 | runs a visit with `Hidden::check` |
| `mon/tue/sat(h,m)` | 3556/**3561**/3567 | `GameTime{day:0/1/5,h,m}`; day 1 = Tuesday, a school day |
| `out_at_work` | 3821 | seed 3, `sat(11,0)`, sofa; `osaka.place(x,y)` then `osaka.go_to_work(None, now, &mut Rng(1))` called directly |
| `feet` | 3855 | `visit.image.feet()` |
| **`home_at`** | **3865** | `home_at(seed, at: GameTime, pieces: &[(Furniture, Nook, u16 thousandths)], graphics) -> Guest`: `Ledger::new_at(seed, at)`, `clock_sent=true`, `ledger.home.add(Prop::new(item, nook, x, Facing::Right))` asserted, `Guest::restore`, `set_date(date(2026,6,17))`, picker if graphics. Absent; arrives when the idle gate opens. **This is how to build a guest with owned pieces.** |
| `shell_step` / `until_visiting` / `real_of` | 3891/3903/3914 | `real_of(&guest, now, tue(8,15))` = monotonic ms for a game time |
| `given` | 4647 | busy screen; `Guest::new`, fed/unfed, `cue(Arrive)`, `give` each piece |
| `shown_piece(guest, item)` | 5335 | the visit's `Shown` for `item`; use it to find the bed/sofa/desk spot |
| `resident_view` | 6501 | disjoint chat/nooks (see §0) |
| `rule_home` | 9790 | Playlist-only screen; props with explicit `Anchor{side,offset}` and `settled` |
| `wordy_rooms` / `wordy_home_screen` / `home_screens` | 12053/5954/5975 | text-dense variants |
| `visit_of` | 10402 | |

`Prop::new(item, nook, at, facing)` is at room.rs:720: anchor None, `at` thousandths, settled. Anchors are pinned on the first `project`. `Home::add` (room.rs:1672) refuses a second of the same kind.

**`long_visit_of` (tests.rs:759-979), what it asserts each frame:**
- `assert_untouched_but_feet`, where feet are the visit image's feet, or **`closed_door().(x,y)` while Away** (tests.rs:814-820).
- Every non-scrap shown piece's strip floor equals `room::strips(&view.nooks)`'s, using **raw strips** (tests.rs:841-848).
- Every piece cell is on blank or line, clear of protected cells and moved text.
- `Hidden::check_raining` in graphics.
- Changed cells ≤ her box + 24 + furniture area + `spanned`. **While Away, `spanned` is the door's box unioned with every piece `terrain::image(door.x, door.y, covers).with` takes in** (tests.rs:921-944). That allowance encodes the bug (a door image swallowing a bed) as accepted. Remove it. A side-on door in wall column `to` is outside her box at `(to-3,f)`, so the budget and feet logic need door cells, as `open_flap` does.
- At the end, a key press and `dissolve::DURATION_MS` must restore the real frame.

### 2. tests/away.rs (1454 lines)

- **Helpers:**
  - `HOME` (:13): Sofa Users 300, Tv Users 800, Bed Playlist 300, Lamp Playlist 800.
  - `empty_of` :26; `box_changed` :34 (her 5×4+1 box).
  - `roomy_screens` :45.
  - `closed(door: DoorAt) -> Door` :55, i.e. `Door::closed(x,y,facing)`.
  - `door_shows(empty, frame, real, door: DoorAt)` :63. Line art compares `image.figure.door()` cells to `closed(door)`. ASCII checks door glyphs on blank cells **not under any piece cover** (it tolerates a door overlapping pieces).
  - `until_away` :96.
  - `out_to_school(real, view, graphics) -> (Guest, u64, DoorAt)` :110: seed 4, `home_at(4, tue(8,10), &HOME, g)`.
  - `somewhen()` :1186; `door_raining(guest, door: DoorAt)` :1277.
- **:125 `a_school_morning_out_through_her_door_and_home_again`.** Asserts `spot == from` ("through her door where she stood") and `door == from`, and at 12:45 asserts `(osaka.x,y) == from`. **Under `Job::Leave` she walks to the space first**, so `from` must become the space spot.
- **:376 `her_door_stays_where_she_left_whatever_takes_her_home_away`.**
  - Asserts the door is off `door_spot(&Terrain::read(..), middle(size))`.
  - Then for key, overlay, too small and visits off: the door's spot is kept, and it shows again at the same spot.
  - Uses `door_spot` and `middle` (mod.rs:4613/4600).
  - Brief §6.7: rename it to "stays in its space"; the `middle` comparison becomes "equals `door_place`'s space spot".
- **:466 `a_resize_moves_her_door_to_the_nearest_spot_it_fits`.**
  - Switches to `wordy_rooms(100,30)`, furnishes `fit` with `empty.shown` covers, and asserts `!door_fits(old)`, `moved == door_spot(&fit, old)`, same facing, and that it stays.
  - Brief §6.7 keeps it as the fallback's test with the strict predicate. `door_fits`/`door_spot` must be replaced by the new strict predicate and `door_place`.
  - Under the new rule the door is recomputed per frame. "Nearest to the old spot" becomes "the space on the new frame, or the fallback nearest the space or `middle`".
- **:504 / :578 (focused pane).** `resident_at` (:554) is a resident at `tue(9,0)` with Sofa Users 300, Bed Playlist 300, Lamp Playlist 800.
  - :578 builds `focus` from the face-on door box `(door.x, door.y-HEIGHT, WIDTH, HEIGHT+1)`. That assumption breaks for side-on door B.
  - :504 asserts `doors_pane`, i.e. her door stood in some pane, via `osaka::box_meets`.
- **:626 `a_cold_start_in_school_hours`.** Furnished and unfurnished at `tue(12,30)`. Asserts `door_shows` after the gate, and that she comes home with no `Fall`. Site for the cold-start `door_place`.
- **:985 `out_at_work_as_school_begins_she_goes_on_to_school`.** Calls `visit.osaka.go_to_work(None, now, &mut Rng(1))` directly (:996). The signature is `pub fn go_to_work(&mut self, out: Option<Link>, at: u64, rng: &mut Rng)` at osaka.rs:9463; it must change if the door arm needs a `DoorSpot`.
- **:1014 `rained_out_on_her_way_to_school_she_is_simply_gone`.** Asserts `closed_door() == (x,y)` where she stood in `Act::Door`.
- **:1054** `placed_as_she_sets_off_she_isnt_leaving`.
- **:1078** `late_for_school_only_when_it_cuts_her_breakfast`.
- **:1232 `her_days_never_touch_what_is_protected`.**
  - Config: `proptest_cases(16)`. Params: `seed`, `start in somewhen()`, `graphics: any::<bool>()` (drawn, not looped), sizes `(48..130, 14..45)` 1..3, text, skips, chats `<120 000`, `protect`, and `owned` `vec((0..4, 0..3, 0..=1000, bool), 0..4)`.
  - Builds the guest with `Guest::restore(Ledger::new_at(seed, start))` and `set_date(date(2026,6,17))` (no `clock_sent`), then calls `long_visit_of(.., 120_000)`.
  - `somewhen()` mixes `near(8,15)` (days 1..4, ±3 min), `near(12,45)`, bedtime ±3 and anywhere.
- **:1257** `her_days_case_pulling_text_as_school_begins`: the pinned-case pattern, `long_visit_of` with fixed args. Copy it for any proptest failure you want to keep.

### 3. tests/window.rs (432 lines)

- `sofa_and_window_at(seed, start, graphics)` :11: Sofa Users anchor 0 (cols 1..10) and Window Users 160 (cols 8..12). Its comment pins exact columns; **a push from a door space on Users' left wall moves both**.
- `window_first(looks, at)` :41.
- **:207 `her_door_at_her_sofa_keeps_her_window_behind_it`.**
  - Seed 4 at `tue(8,10)`, line art; runs to Away.
  - Then **mutates `guest.out.as_mut().door = Some(DoorAt{x:10, y:sofa.floor, facing:Right})`** (:223-228), paints, and asserts `closed_door == (10, floor)`, `Look::Door(_)` drawn, and window layers before sofa.
  - It can't survive `Out { door: bool }`. Rewrite it per brief §6.6: a door forced onto her sofa stands in its space, and its image takes in no piece.
- **:307 `an_old_window_loads_hung_low_drawn_and_in_reach`.** Builds an old record with `Ledger::from_json(&format!(..))` (:309-325). **This is the template for the migration test (brief §6.4)**: an old record with no `door` field and a fridge at `Anchor{Right,0}`.
- :389 `nothing_is_made_under_her_window` calls `builds(&real, visit, &[pull], &[])` directly. Makeshift placement sits here (`builds`' `clear`).

### 4. tests/dash.rs (1343 lines)

- **Helpers:**
  - `FRIDGE_HOME` :15 (Sofa U300, Tv U800, Bed P200, Fridge P800); `BARE_HOME` :23.
  - `dash_seed(from)` :34 uses `brain::dash(seed, TUESDAY)`; `no_dash_seed` :719.
  - `tue_at(min)` :41; `long_step` :68.
  - `Seen { came: Option<(u64, Option<DoorAt>)>, first, said, fridge_open, methods, back }` :93-108.
  - `watch_dash` :113 records `closed_door()` before each step.
- **Assertions tied to the door's spot:**
  - :153 `a_dash_home_for_her_lunch_and_out_again`: `seen.first == (door.x, door.y)`, out of her door.
  - :900 `with_no_home_she_dashes_home_from_absent`: `closed_door() == seen.first`, "out where she came in", with no home. This is the unsaved no-home chooser's test.
  - :614 `the_errand_at_school_is_a_dash` and :980: `guest.out.is_some_and(|o| o.door.is_some())` at **:670 and :1018**. These break when `Out.door` becomes `bool`.
  - :1144 `a_dash_cued_on_a_visit_keeps_the_visit`: asserts **`door_fits(&visit.terrain, spot)`** (:1171) for the stage `dash_through` (osaka.rs:9163). This one is a door in space. If `door_fits` is removed it needs the in-space predicate (`platform_at && restful`), not the strict one, unless stage `dash_through` moves to the space, as brief §5.6 suggests.
- **Proptests**, `proptest_cases(16)` at :1210:
  - `a_dash_home_has_her_lunch_whatever_the_chat` :1222.
  - `a_dash_home_never_touches_what_is_protected` :1272: `long_visit_of` from `before_a_dash(seed, from)` (:1193); owned `0..4` plus a Fridge.

### 5. room.rs and rules.rs property tests

- **Fixtures (room.rs `mod tests` :1863):**
  - `standing()` :2235; `pieces()` :2245 (1..=5 standing); `decorated()` :2252 (adds Poster and maybe Window); `placed()` :2269 (random `Anchor{side, offset 0..40}` or share).
  - `users(width, text)` :2306: a single Users pane that **is** the whole buffer, so both walls are screen edges.
  - `two_panes(w,h,text)` :2388: Users plus a 40×12 Playlist.
- **:2328 `packing_keeps_order_and_a_resize_and_back_restores`**, `proptest_cases(256)`.
  - Uses `Home { props }`, a struct literal (:2335).
  - Asserts `room == pinned` after `layout`, order, apart, `s.left >= 1 && right < wide`.
  - Asserts a resize and back restores, and that text closets only what it covers.
- **:3133 `the_wall_lane_keeps_order_and_never_moves_the_room`**, `proptest_cases(128)`.
  - Uses `Home { props }` (:3141).
  - Checks bounds against **raw `strips(&nooks)`** (:3167), plus window/sofa overlap and resize behaviour.
  - Its `holds` uses `pack(&pinned.on(users_strip, Floor), small_users)` on the **raw** extent (:3201), which matches "narrow only if it packs".
- **:2426 `hung_pieces_clear_every_standing_piece`** is a plain `#[test]` over `Furniture::spec()`, not a proptest. Brief test 3 belongs beside the two proptests above, or as a new one.
- **`Home { .. }` literals** at room.rs:2335, :2837, :3141. `Home` (room.rs:1119-1122) is `#[derive(Clone, Debug, Default, PartialEq, Hash)] struct Home { props }`. A new `door` field breaks these three literals; `Home::default()` sites are fine. The `PartialEq` equalities in these proptests (`room == pinned`) will compare `door` too. That's intended: a resize must not re-choose it.
- **rules.rs:**
  - `rules_hold_across_a_resize_and_text` :1319, `proptest_cases(128)`.
  - `every_repair_mends_and_breaks_nothing` :2281, `proptest_cases(64)`. It uses `three(width, 8, n)` (:2020, full-width stacked panes, every wall a screen edge) and computes `broken(&laid, &strips(&nooks), &home)` on **raw strips**. This is where "no repair places a piece in the space or the chat pane" goes.
  - `repair_search_is_cheap` :2038.
  - **`rules::Frame { buf, nooks, blocked, shown, made }`** (rules.rs:404) is built literally at rules.rs:1431, 1733, 1842, 1919, 2060, 2312, 2328 and mod.rs:3238, 3386, 3591. If it grows a door-space or chat field, all ten change.
  - `against_wall` is still at rules.rs:324.

### 6. Census, band and stillness (stillness budget)

- **The band gate never sees a school morning.**
  - tests/band.rs runs `afternoon_rooms()` from `AFTERNOON = Tue 13:00` (census.rs:1588) for `BAND_MINUTES = 17` real minutes, about 13:00–14:42 game time at 6×, with `GATE_SEEDS = 2`.
  - The stillness drawn tests (tests/stillness.rs:989, :1027) also run afternoons, or 16:30 to dusk.
  - So **a walk to her door at 08:15 moves no band test.**
- **Indirect drift.** The band and stillness rooms (`furnished_room()` = home_screen + 7 pieces, `resident_room()`) will shift if the door's space pushes pieces 6 columns, because her trips change. Re-run the band at the gate after the space lands:
  - `cargo nextest run -p dessplay --run-ignored all -E 'test(/houseguest::tests::band::/)'`
  - full strength per band.rs:31-35 with `CENSUS_BAND_SEEDS=<n> … --release --profile band`.
- **Where the morning walk is counted.** Only in the ignored `day_census` (census.rs:2310): `MONDAY` = day 7 00:00, a week, `DAY_HOME` (:1515) in `day_rooms()` (:1530). Run it with:
  - `cargo test --release -p dessplay --lib day_census -- --ignored --nocapture`
  - Moving counts through `Osaka::census_motion` (osaka.rs:7032) and `census_purpose` (osaka.rs:7064). Set-offs go through `set_offs += 1` and `set_off_log` (osaka.rs:3950).
- **`census_purpose` checks `heading` first**, through an exhaustive `match job { Pull|Swap|Build|Borrow => "to text", Use => "to seat", Lift|SetDown => "to home" }` (osaka.rs:7065-7069). Only after that does it fall back to `leaving.is_some() → "routine"` (:7082).
  - `Job::Leave` will not compile until it gets an arm. Give it `"routine"`, or her walk to the door reads as something else.
  - `JobRef` (scenes.rs:315) is an exhaustive mirror too.
- **Census code that reads the `Out` shape:** census.rs:1920 (`Where::of`: `guest.out.is_some()` → Away) and census.rs:2184 (`moved(was, &is, guest.out.is_some())`). Both survive `Out { door: bool }` because they only use `is_some`.
- **stillness.rs (the module, 277 lines)** is the lever table (`Stillness::{NEUTRAL, STARTING, TUNED}`, `SHIPPED` :277). Nothing in it is door-related.

### 7. Goldens (tests/golden.rs) and snapshots

- **What gets hashed.** `Trace::frame` (:43) hashes act, x, y, facing, appearance, image `her()` and every changed cell. `finish` (:88) also hashes **`guest.ledger.to_json()`**.
- **A saved `Home.door` moves every golden whose record has a home.** Line art's `0x…` changes alongside.
- **Fed scenes** (golden.rs:884-1060). golden.rs has its own `home_at` (:426): Sofa U50, Tv U400, Desk U850, Bed P500, no `set_date`.

| Scene | Fn | Start | Expected to move? |
|---|---|---|---|
| stage | `stage_room` :202 | cues MakeSofa, Swap, … MakeBed, Parcel | yes if makeshift or parcel placement or door changes (parcel flap moves into the door) |
| resident | :246 | | yes if the layout pushes or a door is chosen |
| furnished | :324 | `give` Sofa/Tv/Bed/Desk | yes (push and saved door) |
| errand | :369 | accordion room, disjoint chat | maybe |
| evening | :449 | Mon 20:30 | yes (push and record) |
| tucked | :482 | Mon 23:00 | yes |
| weekend | :515 | Sat 10:00 (work open) | yes (work through her door) |
| school | :577 | Tue 08:10, 3 min | **yes (the bug's scene)** |
| home | :589 | Tue 12:40 | **yes** |
| dash | :603 | first dash −1 min, Fridge P900 | **yes** |

- **`UNFED_*` tables** (:1066-1206: stage, resident, furnished, errand) run `Guest::unfed()`: no routine and no Away. Unfed she still goes to work. **No step may move them without a trace diff in the commit.** Expect them to move only through layout, record or work-door changes.
- **Re-recording** (docs/testing-strategy.md:776-819):
  - A failing check prints the replacement table (`check_fed`, golden.rs:651).
  - Before re-recording, run old and new revisions with `HOUSEGUEST_GOLDEN_TRACE=<existing dir>` and diff `<scene>-<seed>-<graphics>[-unfed].txt`.
  - The commit says where the traces first differ and why, and that the rest are byte-identical.
- **Insta snapshots** (`snapshots/`, 4 files): `default_layout_terrain_100x30` / `_graphics_100x30` / `_80x24` (tests.rs:1051-1065) read only the terrain. `osaka_at_home_seed_7` (tests.rs:1069) is `Guest::new(7)` with no home, run 95 s on the real layout. None should move unless the no-home door shows in it. Re-accept with `cargo insta review`, with a reason in the commit.

### 8. Proptest regression files (`dessplay/proptest-regressions/ui/houseguest/`)

- Counts: `tests.txt` (26), `tests/away.txt` (3), `tests/dash.txt` (2), `room.txt` (4), `rules.txt` (7), `osaka.txt` (3), `dissolve.txt` (5), `layer.txt` (1), `scrap.txt` (1), `tests/rain.txt` (2).
- **What `away.txt` holds:** three `her_days` cases, `start` Tue 08:12 / 08:14 / 08:12, owned `[]`, `[(0,2,510,false)]` (Sofa in Playlist), `[]`. None is a bed or desk at the cut.
- **The `cc` lines are RNG seeds, not values.** The "shrinks to" comment is only a note. Changing a strategy's shape (a new parameter, a reweighted `owned`) makes these seeds generate different cases without any warning. Every seed in a file is replayed for every proptest in that source file.
- To keep a case across a strategy change, pin it as a fixed-argument unit test, as away.rs:1257 does.

### 9. What the planned shape change breaks in tests (full list)

- **`DoorAt` in test signatures:** away.rs:55 `closed`, :63 `door_shows`, :110 `out_to_school`, :1277 `door_raining`; dash.rs:95 `Seen.came`.
- **`guest.out…door` mutated or matched:** window.rs:223-228 (mutation); dash.rs:670, :1018 (`o.door.is_some()`).
- **`door_fits` / `door_spot` / `middle` (mod.rs:4607/4613/4600):** away.rs:388, :484, :487; dash.rs:1171.
- **Face-on box assumptions around `closed_door().(x,y)`:**
  - tests.rs:818 (feet), tests.rs:921-944 (Away `spanned`), tests.rs:953 (debug).
  - away.rs:34 `box_changed`, :63 `door_shows` (via `Door::closed(..).cells()`), :578-600 focus box.
  - tests.rs:3740 and window.rs:240 match `Look::Door(_)`. Side-on door B is a new look; brief §5.3 keeps `Look::Door` for the door in space.
- **Direct `go_to_work(None, now, &mut Rng(1))` calls:** tests.rs:3838, away.rs:996. These change if the door arm takes a spot.
- **`Home { props }` literals:** room.rs:2335, :2837, :3141.
- **`rules::Frame` literals:** ×10 (§5).
- **Exhaustive `Job` matches:** osaka.rs:7065 (`census_purpose`) and scenes.rs:315 `JobRef`.
- **Parcel flap position and glyphs:**
  - tests.rs:4971 `a_parcel_comes_in_through_a_flap_at_the_screens_edge` asserts the parcel at `users.x+1` and `╲` on `users.x` rows `floor-2..floor`.
  - `open_flap` and `Hidden`'s flap rule (tests.rs:595-601, 642).
  - Under B the flap is in the door and parcels rest just past the space.

### 10. Brief claims in this area that are now wrong or stale

1. **§6.1, "make sure the generator includes bed/sofa/desk and school mornings".** It already does: `owned` indexes `Furniture::ALL[0..4]` = Sofa, Tv, Bed, Desk (room.rs:53-57), and `somewhen()` includes `near(8,15)`. The real gap is that at 16 pinned cases (32 under the hook) being *in* a piece at the cut is luck. (a) is not reliably red today. The §6.2 unit test is the reliable red test.
   - Recipe: `home_at(seed, tue(8,14), &[(Bed|Sofa|Desk, …)], g)` → `until_visiting` → `guest.cue(Scene::Sleep | Nap | Lounge | Homework)` (stage.rs:88-96), or `osaka.place` at `shown_piece(..)`'s seat → run to Away.
   - Commit any proptest failure into away.txt and pin it as a unit test.
2. **§6.1, "for `graphics in [false, true]`".** The proptest draws `graphics: any::<bool>()`. Only the unit tests loop.
3. **§6.3, "room.rs proptest next to `hung_pieces_clear_every_standing_piece` (room.rs:2383)".** That test is at :2426 and is a plain `#[test]`. The proptests are at :2328 and :3133.
4. **§7 step 2, "Next batch: the side-on door".** plan.md puts door B with its flap **in this batch**. Every face-on 5×4 door-box assumption in §9 is therefore not a stable intermediate.
5. **§5.6, "Census cost: one short walk per school morning, against the stillness budget".** No gated test sees it. The band and stillness tests run afternoons (§6). Only the ignored `day_census` counts it.
6. **§6.6 (window.rs:207).** Confirmed: it forces the door by mutating `guest.out`, so it has to be rewritten, not adjusted.
7. **§6.7.** away.rs:376 and :466 are still at those lines. :466 depends on `door_fits` / `door_spot`, which are slated for replacement, so its "nearest to the old spot" assertion has to change too (the spot is per-frame now).
8. **Drifted line refs:**

| Item | Brief | Now |
|---|---|---|
| `out_by_door` | mod.rs:1691 | mod.rs:1869 |
| `door_fits` | mod.rs:4365 | mod.rs:4607 |
| `door_spot` | mod.rs:4371 | mod.rs:4613 |
| `paint_empty` | mod.rs:4466 | mod.rs:4657 |
| `draw_door` | mod.rs:4619 | mod.rs:4786 |
| `DoorAt` / `Out` | — | mod.rs:691 / 700 |
| `closed_door` | — | mod.rs:1849 |
| `go_out` | osaka.rs:5151 | osaka.rs:5266 |
| `wake` | osaka.rs:5208 | osaka.rs:5323 |
| `dash_on` | osaka.rs:5568 | osaka.rs:5684 |
| `dash_through` | osaka.rs:8963 | osaka.rs:9163 |
| `go_to_work` | osaka.rs:9263 | osaka.rs:9463 |
| `school_from_work` | osaka.rs:3456 | osaka.rs:3552 |
| `Saved` / `Raw` | ledger.rs:567 / 615 | ledger.rs:568 / 623 |
| `Home` | room.rs:1119-1121 | room.rs:1119-1122 |

   Unchanged: away.rs 376/466/1232, tests.rs:759, window.rs:207, mind.rs:440 `door_away`, rules.rs:324, scenes.rs:235 `Job`.

### 11. How to run, and timings (measured 2026-10-08, warm cache, 32 cores, dev opt-level 2)

- **All houseguest tests at gate strength (one run):**
  `PROPTEST_CASES=32 cargo nextest run -p dessplay -E 'test(/houseguest::/)'`
  - 739 tests, **32.8 s wall**, after a 53 s incremental build. All passed at `5cdb69ca`.
- **Long poles:**
  - `a_trial_keeps_every_promise` 29.4 s, just under the default profile's 30 s SLOW flag (60 s kill).
  - `credit::what_she_chooses_eases_what_it_serves` 25.5 s; `film::her_film_never_moves_her` 24.9 s; `every_made_piece_is_used_or_let_go` 24.1 s.
  - `stillness::no_long_act_flips_drawn_cells…_by_her_clock` 19.1 s and the plain version 18.5 s; `text_arriving_where_she_rests…` 18.6 s.
- **Door-relevant tests:**
  - `long_visits_never_touch…` 10.2 s; `her_needs_shape_long_visits` 14.5 s; `every_parcel_on_her_doorstep_gets_unpacked` 7.7 s; `long_visits_with_decor…` 2.6 s.
  - `away::her_days_never_touch…` **2.0 s**, so its case count could be raised cheaply.
  - `dash::a_dash_home_never_touches…` 1.6 s; `dash::a_dash_home_has_her_lunch…` 2.9 s.
  - goldens 0.5–6.5 s each (`golden_furnished_home` 6.5 s, `golden_school_morning` 1.2 s).
  - away/window unit tests ≤0.8 s; room and rules proptests ≤0.07 s at 32 cases.
- **Targeted runs:**
  - `-E 'test(/houseguest::tests::away::/)'`, likewise `::dash::`, `::window::`, `::golden::`, `houseguest::room::tests`, `houseguest::rules::tests`.
  - Use `--run-ignored all` for the ignored band, spread and census tests.
- **Case counts:**
  - `PROPTEST_CASES=N` overrides only the default config, plus every site written as `ProptestConfig::with_cases(dessplay_core::test_support::proptest_cases(K))` (dessplay-core/src/test_support.rs:43).
  - Every houseguest proptest uses `proptest_cases`: tests.rs 24/24/12, away and dash 16, room 256/128, rules 128/64.
  - **Never write a bare `with_cases(N)`.**
- **Profiles** (`.config/nextest.toml`):
  - **default** (the gate): excludes `binary(perf)`, SLOW at 30 s, kill at 60 s.
  - **deep:** `PROPTEST_CASES=256 cargo nextest run --release --profile deep -p dessplay houseguest`, flag at 90 s, kill at 180 s. Per its comment, `her_film_never_moves_her` takes about 72–81 s there and `a_trial_keeps_every_promise` about 52 s.
  - **full:** `--profile full --release`, adds perf tests.
  - **band:** `CENSUS_BAND_SEEDS=<n> … --release --profile band --run-ignored all`, 4.5–10.5 min per band test at N=525.
- **The full gate** (the stop hook): `PROPTEST_CASES=32 cargo nextest run`, plus fmt and clippy. Verify across crates with `--workspace --all-targets`.
- **Builders in a worktree commit with git:** jj there acts on the main working copy.

<!-- section: critic -->
## Gaps and corrections (completeness critic)

Paths are relative to `dessplay/src/ui/houseguest/` and line numbers are from HEAD `057311b1` (2026-10-08). The tests section's `5cdb69ca` is the jj working-copy id for the same clean tree. I edited nothing.

### A. Spot-checks (38 claims checked against the code)

Confirmed, with correct lines:
- osaka.rs
  - `cut` 3491. Cuttable is `Stays::{Rest,Job} || to_job` (3529). `Act::Door/Out/Away` are `(Stays::Pass, OnChat::Back)` (1218).
  - `go_out` 5266-5291: `to: (self.x, self.y)` at 5285, info log `?why` at 5277, returns `Decision::reflex("routine/away")`.
  - `choose_next` order: off-text 7305, dash 7312, settle 7322, clock glance 7349-7362, `go_out` 7371.
  - `go_to_work` 9463 (None arm `to: here`). `evict` 9334, with its leaving arm at 9345-9370. Door tick step 4062-4092. `census_purpose`'s exhaustive `Job` match 7065-7069. `CHAT_FACTOR` 178, `in_chat` 353, `box_meets` 191, `beside` 258.
- mod.rs
  - `out_by_door` 1869 (DoorAt from her feet 1874-1878, log 1873).
  - Arrival: `Terrain::read` unfurnished at 2026, `through` closure 2033-2037, `back_through_door` hard-codes `Routine::School`.
  - `paint_empty` 4657. It projects at 4704, which mutates the ledger: `changed` feeds `self.unsaved`. Re-place 4712-4720, standing filter 4722-4727. `door_fits` 4607, `door_spot` 4613, `middle` 4600.
  - `door_rain` 2805, `FLAP_MS` 4138, `draw_flap` 4143, `builds` 3922 with `clear` at 3938-3943. `clock_on` 211, `beauty_at` 228.
  - Chances literals 2353 and 2403. Away arm 2092-2124, with `unkept` at 2107.
- room.rs
  - `Extent` 736, `strips` 795, `Home` 1120 (derive on 1119), `furnished()` 1164 in prop insertion order, `layout` 1209, `hung_clear` 1281 with `meets` 1299-1304.
  - `doorstep` 1517: edge tests 1529-1530, sort **1533**, flap x 1596-1599.
  - `her_box` 1801, `free` 1820-1832 (tests `s.rect()`), `Home { props }` literals 2335 and 3141. Tests 2328, 2426 (plain `#[test]`) and 3133.
- rules.rs: `RULES: [RuleRow; 6]` at 63, `broken` 183, `against_wall` 324, `Frame` 404, `Before::new` 462. `before.strips` is read at 534, 548, 573, 595 and 760. There are 10 `Frame {` literals: 7 in rules tests and mod.rs 3238, 3386 and 3591.
- art.rs: `WallDoor` 1121, `WALL_DOOR_FRAME` **1140**, `WALL_DOOR_SHIFT` **1141**, `render_wall_door` 1221, `dead_code` attributes at 1119 and 1220. `lean` is a **closure** inside `wall_door_sheet` (3615), not a fn. The sheet's `put` clip is at 3703-3720.
- sprite.rs: `mirror` 453, `WALL_DOOR` 479 (attribute at 478), `door_cells` 489, lint 780.
- graphics.rs: `Look` 65 with `Door` at 71, `paint_layers` 503, `key` 568, `compose` 735.
- Tests
  - tests.rs: `view()` chat `Rect::new(0,0,30,20)`. `nooks()` puts List at `(0,0,w/2,h-3)`, which overlaps that chat rect. `HIDDEN_MS` 556, `open_flap` 642, `home_at` 3865, parcel-flap test 4971, `resident_view` 6501.
  - tests/away.rs:1232: `graphics in any::<bool>()`, `owned` from `Furniture::ALL[0..4]`.
  - tests/dash.rs: `o.door.is_some()` at 670 and 1018, `door_fits` at 1171. tests/window.rs:224 mutates `out.door`.
  - stage.rs: `door_spot` 485, `dash_through` 490, `go_to_work` 636, `through_door` 658.
- brain.rs:209 `home_acts` is Lazy 0, Ordinary/Dreamy 1, Industrious 3. Lamp and Plant have `uses: &[]` (room.rs:245 and 301).
- idle.rs: `chat: Rect` 86, `protected` 94, `nooks` 96. app.rs: `chat: self.panes.chat` at **661**, resident selecting pushes `panes.chat` at 645-646.

Corrections to the sections' line numbers (all small, but builders grep by them):

| Section | Says | Correct |
|---|---|---|
| door-lifecycle | `errand(spot, ..)` at 9196 | fn at **9181**; the Door retarget is at 9198ff |
| door-lifecycle | `absent` 1740 | **1744** |
| door-lifecycle | `Guest.out` 844 | **842** |
| door-lifecycle | `Chances.clock` 66 | **69** (chat-pane's "~68" is also off) |
| layout | `furnish` 3211 | **3206** (rules and chat-pane are right) |
| chat-pane | `hung_clear`'s `meets` ~1314-1319 | **1299-1304**; 1311-1317 is `leans` |
| chat-pane / art | doorstep sort 1532 / 1531 | **1533** |
| chat-pane | `MIN_HEIGHT` 241-242 | **243** |
| art | `CACHE_LIMIT` graphics.rs:59; `Sky::at` 1408 | **61**; **1410** (`Sky` 1385) |
| art / layout | flap Prop "1604-1608" / "1542-1558" | `Flap` built **1603-1607**, x match **1595-1598** |

### B. Contradictions between sections, resolved

1. **Is the strict check the same as `free(None)`?** The layout section says yes for floor pieces. The chat-pane section says no.
   - Both are partly right. `free` tests `s.rect()`. Her box is rows `y-4..=y`.
   - The only difference is a piece whose *floor row* falls in her box but whose rect does not. For example, a piece standing on a ledge at row `y-4` (her head row): its cover meets her box, but its rect does not. A makeshift piece on a pull line can do this.
   - So build the strict predicate as `her_box(x,y).is_some_and(|b| shown.iter().all(|s| !s.cover().intersects(b)))`. That is `clear_of` (mod.rs:3702) without its `of` exception. Don't use `free`.
2. **Chat pane in `blocked` or a separate keep-out?** The chat-pane section names `blocked` as the cheap "held out of the pane" option. The rules section says not to widen `blocked`.
   - These don't conflict. Widening `blocked` *picks* one branch of the user's open question (the piece is held out, so she has nothing to notice). It also does nothing to `move_off` (a strip that packs never moves).
   - The keep-out field (`Frame.keep_out` plus a `judge` input) keeps both branches open. Don't build either until the user answers the open question.
3. **"Work: screen edge first"** (plan.md) conflicts with design.md:2233-2235: "Everything of hers that comes and goes by her routine uses her door, never a screen edge". Work isn't routine, but design.md:1763-1766 has work "out at a screen edge if her floor reaches one, else through her door".
   - The plan.md parenthetical reads as keeping the `Route::Around` walk-out. door-lifecycle flags the ambiguity correctly. Ask before building.
4. **Left-wall naming.** art says a left-wall Layer `at = (to+3, f)`, where "to" is the wall column. In layout's terms the left wall column is `from-1`. So `at = (from+2, f)`, which is also her spot.
   - `render_wall_door`'s doc and the snippets call the wall column "to" on both sides. Name it `wall` in the new `DoorSpot` so builders don't write `e.to` for a left wall.
5. **Headroom.** chat-pane computes a nook needs H ≳ 23 for a 4-row space. layout's `Extent::holds` reasoning agrees, since the space needs `rows ≥ 4`.
   - So at `MIN_HEIGHT` 18 the space always yields. The yield path is the normal path on short terminals, and needs gate-level test coverage, not just one unit test.

### C. What the door batch needs that no section covers

**C1. Sites that lose their door x when `Out` stops storing a spot.**
- `Out{door: bool}` (brief §4) leaves these with no spot: `closed_door()` (mod.rs:1849), `door_of` (1422), `leave`'s Away arm (1308), `call_off` (1391-1402) and `door_rain` (2805).
- They run from tick and key paths with **no `Buffer`**, so they can't call `door_place`. They need the door x for the rain origin. `door_rain` also needs the exact cells (it filters `empty.painted` by `Door::closed(..).cells()`).
- Fix: cache the last frame's drawn `Option<DoorSpot>` in `Empty`. `Empty` already caches last-frame `shown`, `painted` and `image` (mod.rs:713-731). Treat the cache as derived, never a source of truth.
- Tag door cells in `painted`, or keep their set, so `door_rain` rains the wall-column cells and the slippers, not a 5×4 face-on box.
- `How::Return(Option<DoorAt>)` then carries nothing useful and can become a unit variant.

**C2. `door_place` must place by `unkept`, not the gated view.**
- The Away arm deliberately places the door against `unkept`, the frame without the focus pane (mod.rs:2103-2107). The design says "her closed door in the focused pane rains out with her pieces there, and comes back with them" (design.md:2397-2399).
- If `door_place` checks the space against gated `view.protected`, the door hops to the fallback every time the resident's focused pane is in use.
- So: place against `unkept` (and the chat rect), then run the standing/hidden filter against `view.protected`, using the **wall door's** box, not `box_meets`' 5×4.

**C3. When `leaving` is set, under `Job::Leave`.**
- Today `go_out` sets `leaving` at the door. Under `Job::Leave` she walks first. Several things read `leaving.is_some()`:
  - `evict`'s leaving arm (osaka.rs:9345) sends her `Act::Away` at her feet: "out for good" mid-walk.
  - `census_purpose` returns "routine" only after its heading and Walk-job checks.
  - `errand` and `place` clear it.
  - `gone_out` needs `leaving && hidden && door none`.
- Decide whether `leaving` is set at walk start or at the door.
  - If at walk start: evict mid-walk sends her out without a door. Fine once the Away door is per-frame, but say so.
  - If at the door: a cut or a chat interruption mid-walk must re-enter through the reflex, which it does today because `go_out` is reached from `choose_next` each decision.
- Either way, **`go_out` must not re-say the `OFF`/`LATE` line on every re-entry**: after a hop landing, an interruption, or a `came_to_nothing`. `send_to_bed` (5200) is the model and says nothing per landing. Add a "set off" latch.

**C4. `evict`'s non-leaving arm and work by door.**
- `leaving_for_work` (osaka.rs:9406-9417) matches only `Act::Out` or `Walk{then: Link(Route::Around)}`.
- A `Walk{then: Job(Leave)}` with `shift == Going` gets `gap = 0`: an instant door in space, so she "comes home from work" at once with `Shift::Going` still set.
- Add the arm. Also `cut`'s `to_job` (3507-3517) would cut a Leave walk at the *next* boundary (door-lifecycle flags this). Exclude `Job::Leave`.

**C5. A door's `to` goes stale during a visit.**
- `Act::Door{to}` is fixed when planted. For work's gap (60-180 s), and for the Away-cue gap beats 6-9 drawn mid-visit, a resize or a move of the space leaves `to` and the drawn door at the old spot.
- At the `there` teleport (osaka.rs:4064) and in the visit's draw, for an external door, re-read `Chances.door` (the frame's `door_place`) rather than `to`.
- `school_from_work` (3552) freezes a work door's gap to MAX. With work through the space, that door is at the space, so it is fine. Note it in the commit as immune.

**C6. Facing.**
- Going out, `DoorSpot.facing` faces the wall. Coming in (Return, Dash, the work return), she faces the room (snippets beats 10-12).
- `back_through_door` and `dash_in` take `facing` from the stored door today (mod.rs:2035). They need the inverse of the space's facing, not the same `DoorSpot.facing`.

**C7. Where `Home.door` is re-chosen.**
- Brief §5.4 says the wall changes only when its strip leaves `nooks`, following the room as `move_off` moves it.
- No section names the write site. It should be inside `project`'s `move_off` (room.rs:1378). `move_off` returns silently with no target (layout §), so define what `Home.door` becomes then: kept, which means the space yields.
- `pin_anchors` (1187) must pin on the narrowed extent (layout §), and so must mod.rs:3253 (rules §). That makes `Home::extents` the **single** function every strips caller uses, `Before::new` included.

**C8. Doors in space and the chat pane (open; ask the user).**
- The plan says "her door is never in the chat pane". It doesn't say whether that covers **doors in space** (the far end of `through_door`, `calm_elsewhere`, `door_away`) or the errand's arrival.
- `send`'s errand arrives by a door in space **at the scrollback accordion, which is inside the chat pane** (mod.rs:2755; `arrive_for_errand` osaka.rs:9090). If doors in space are in scope, the errand design itself changes.
- Recommendation: the external door only. Doors in space last about 1.5 s and sit in her box. Confirm with the user before building.

**C9. Stage support for checking the art by eye.**
- `stage.rs` has `Parcel`, `Work`, `DashIn` and `DashForgot`, but **no school-out or Away scene** (stage.rs:60-140).
- The approved art (going-out thirds, clip, `Post`, slippers) can't be checked in `cargo run -p dessplay --example houseguest` without a cue such as `Scene::School`.
- `Scene::DashIn` places through `door_spot` (stage.rs:485). Brief §5.6 says it moves to the space.

**C10. CHANGELOG.**
- This is user-visible: her door at the screen's edge, side-on, slippers, parcels through the door, no furniture in the chat pane. It needs a `CHANGELOG.md` entry under the commit's date, in the same commit. No section mentions it.
- The format is validated by `changelog::tests::embedded_changelog_parses`. It isn't a protocol change, so `stable` isn't advanced.

**C11. Docs to change.** No section lists the current lines.
- design.md
  - 1380-1382: "The door stays within her box, so it never covers anything". True only for doors in space; reword.
  - 1763-1766: work out at a screen edge. Depends on the C3/B3 answer.
  - 1784-1790: deliveries "pushed in through a flap in a wall … `╱`/`╲` for 800 ms … stands against that wall". Now through her door's flap, resting past the space. `draw_flap` stays only as the yield fallback.
  - 1813-1830 (*Strips and anchors*): add the door space (rule text in brief §5.1) and "no piece in the chat pane". Note that it already says "never the chat" for strips at 1815-1816.
  - 2209-2220: "stays standing, closed, where she left … If a resize leaves her door's spot unfit, it moves to the nearest floor spot where she'd fit". Replace with the space plus the strict fallback, and add the slippers and card cue.
  - 2233-2235: reconcile with work (B3).
  - 2392-2399: the focused pane and her closed door. Keep it, per C2.
- decisions.md: a new entry, plus a pointer from "Away at school ends the visit at her door" (3334; "closed where she went out" at 3350) saying it is superseded in *where*.
- CLAUDE.md line 25: "planned next … not yet wired" becomes implemented, with plan.md's record.
- snippets.md header: "for review, not wired in, worktree.diff against 1e44b050" is stale. The diff was committed in `8eaa25c1` (art §0).
- room.rs:400: `Nook::List`'s doc "(series), when short" is stale (chat-pane §6).

**C12. Ledger and dump.** layout gives the field recipe. Also add the wall to `Summary` (ledger.rs:369) for `dessplay --dump`, which the dump-state skill reads.
- `the_clock_sent_round_trips` (ledger.rs:1226) asserts the JSON ends with `clock_sent`. Declare `door` before it in `Saved`.

**C13. Tests no section lists.**
- **Yield at minimum height.** A `her_days`-style case at H 18-22, where the space always yields (B5). It asserts the strict fallback meets no cover in both modes, and that the arrival waits when there is no spot.
- **Chat property on honest geometry.** Both tests and chat-pane flag the `view()` overlap. Also add one run of `long_visit_of` on `real_frame` (tests.rs:990, the production `idle_view`) so the chat rule is tested on the bundled layout, not only on a fixed fixture.
- **Re-entry line latch** (C3). A Leave walk across a floor (hop, or a door in space) says `OFF` exactly once. Pin it as a unit test with a two-floor `home_at`.
- **Evict mid-Leave-walk for work** (C4). The shift isn't lost and the gap isn't 0.
- **Focused pane while Away** (C2). The door doesn't move while the pane is in use and comes back at the space. Extend away.rs:504/578, whose focus box assumes the face-on 5×4 box.
- **Older piece in the chat pane.** Anchored pieces can reach it only in an overlapping custom layout (ui/app.rs:4003 tests a grid overlap). The "moved out" test needs a nook rect that meets `view.chat`. The "closeted when nothing fits" test needs a closet mechanism, which doesn't exist yet (rules §6 options a/b/c). Choose one before writing the test.
- **Felt-rule trigger without a Use.** Lamp and Plant have no uses (rules §0.2). A test that a lamp in the space is felt on the return or dash out of the fallback door, and moved, with the mood forced to Industrious (Lazy never mends).

**C14. Open questions to put to the user before builders start.** These come from plan.md's own "Open" list plus what this map found:
1. A layout whose only reachable edge is in the chat pane, or that has no edge nook (chat-pane §4 cases 1-3). Where does the door go then?
2. What the frame shows of an older piece in the chat pane until she moves it (B2).
3. Work: does it keep the `Route::Around` edge walk-out (B3)?
4. Does "never in the chat" cover doors in space and the errand's accordion arrival (C8)?
5. How a piece goes to the closet when nothing fits (rules §6: per frame, persisted, or yield only).
