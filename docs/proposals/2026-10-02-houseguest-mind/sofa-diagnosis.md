# Why Osaka makes a makeshift sofa and then doesn't sit on it: a diagnosis from running the code

## 1. Headline

**The handoff the task describes never failed at HEAD (`654dd11`).** That chain is Crumpled → `goal = Use(build.then)` → Admire → `decide` → `goal.take().filter(offers)` → `go_to` → Use. It worked in 190 of 190 completed crumples. The runs covered:

- ASCII and kitty line art
- 100x30 and 80x24
- a static chat, and a chat that really scrolls (a new line every 45–60 s)
- visitor, resident, and resident + `Busy::Playing` + `focus = chat`
- an empty home, and a home owning a sofa and a TV

Mechanisms (a)–(d) were never seen:

- **(a)/(c):** no `GOAL_CHECK offered=false` ever had a `Use` goal. All `Use` goals that were checked were offered: 940 + 380 + 1023 + 355.
- **(b):** zero `RECHECK_STARTLED`, zero `LOST_SEAT`.
- **(d):** `DECIDE_EARLY` happened with a goal pending (4527 `watch`, 658 `find_rest`, 24 `no_platform`), but those paths return without consuming `goal`. The goal was used on the next `decide`. Four times this showed as "used > 5 s later": a chat line arrived during the 1.2 s Admire, and she sat once the 15 s watch ended.

| Run | Conditions | Completed crumples | Sat straight after |
|---|---|---|---|
| run2 | static room, 24 seeds × 4 combos × 30 min | 70 | 70 |
| run3 | chat scrolls (~45 s), resident | 37 | 37 |
| run4 | chat ~60 s, resident, owns sofa + TV, 40 seeds × 40 min | 46 | 46 |
| run5 | resident + Playing + focus = chat, chat ~45 s | 37 | 37 |

Run4 gave no builds at 80x24. I didn't find out why; it is probably `fits`/`roomy` with the owned pieces present.

**What does lose the purpose: an interrupted crumple plus the single `making` slot.** This is proven with a deterministic test (section 3).

- At `osaka.rs:1254`, `self.making = Some(build)` overwrites any earlier plan.
- At `osaka.rs:1195`, `self.making.take().filter(...)` empties the slot on *any* crumple completion. On a mismatch the plan is dropped.
- `look()` (`osaka.rs:2264-2297`) clears `task` but not `making`, so an interrupted crumple leaves an unfinished heap plus a stale plan.

## 2. Evidence

### 2a. The `making` slot loss (deterministic)

The test: cue MakeSofa, let a chat line arrive mid-crumple, cue MakeBed, then live on for 10 minutes.

- **Failure rate:** 16 of 16 fail (8 seeds × ASCII and line art, 100x30). Every bed is finished first and used after 1533 ms. Every sofa is finished later, through ordinary `Use(Crumple)` scoring, with `making_matched=false` and no `GOAL_SET`. Excerpt:
  ```
  graphics=false seed=0: finished the Bed at 19641: planned use carried=true, next used it after Some(1533) ms
  graphics=false seed=0: finished the Sofa at 310928: planned use carried=false, next used it after Some(11333) ms; ... making_matched=false
  graphics=false seed=7: finished the Sofa at 281293: planned use carried=false, next used it after None ms
  graphics=true  seed=4: finished the Sofa at 220795: planned use carried=false, next used it after Some(359386) ms
  ```
- **Timings:** the sofa heap stayed unfinished for 137–458 s. After it was finished, she next sat on it 4.6–359 s later, or never (seed 7, ASCII).
- **Control:** the same test without the second build (`SKIP_BED=1`) passes for all 8 seed/mode runs. With `making` intact, finishing the heap later still hands off.
- **Caveat:** both cues go through `stage::direct` → `osaka.place()` (`stage.rs:300`, `osaka.rs:2252-2262`). `place()` clears `goal` but not `making`. The natural version is possible: after an interrupted crumple, `builds()` still offers the other kind, and `decide` adds `Kind::Use(b.then.what)` (`osaka.rs:1881-1887`). But in about 160 natural builds I saw **0** cases of a second tear landing while a heap was unfinished. So the mechanism is proven, but its natural frequency is low. It needs an interrupted crumple and a second build inside the 15–106 s window before she resumes.

### 2b. An interrupted crumple leaves a sofa-shaped heap she walks away from

This is the most plausible match for what you saw.

- **How it happens:** `look()` sets `task = None` and switches to `Act::Look` mid-`Use{Crumple}`. The crumple lasts 4–6 s (`osaka.rs:548`). `Crumpled` is only sent when the Use runs to its end (`osaka.rs:1193`).
- **What it looks like:** `tend_made` caps a heap that is being crumpled at stage `STAGES-1 = 3` (`mod.rs:1484`). At stage 3 the piece draws (3+1)/(4+2) = **67% of its shreds, bottom-up** (`scrap.rs:251`). That reads as a mostly-built sofa.
- **Measured:** 8 of 133 crumples were interrupted by `LOOK` when chat arrived every ~45–60 s (run3 2/39, run4 2/48, run5 4/41). She came back to finish it after 15.3, 22.1, 39.0, 57.4, 88.6, 104.5 and 105.7 s, and once never before the run ended (run4).
- **Why she comes back:** the heap itself carries the state. `Shown::uses` returns `[Crumple]` for an unfinished scrap (`room.rs:289-296`), and `Kind::Use(Crumple)` has base 12 (`brain.rs:167`).
- **Not counted:** in the real app, key presses (resident shake), focus eviction and `recheck` interrupt `Use{Crumple}` the same way. My harness only modelled chat.

### 2c. Cross-floor goals dropped by exact-equality validation

This is the only purpose loss I actually observed in natural runs. `Chances::offers` (`osaka.rs:140-147`) uses `contains`, i.e. full structural equality, against the fresh snapshot. When the chat scrolled, the `gap`, `cells` or `x` of a planned Pull/Swap/Build changed and the goal was dropped silently. There is no step that re-plans to an equivalent job.

| Run | Pull | Swap | Build | Use |
|---|---|---|---|---|
| run3 | 83 | 17 | 5 | 0 |
| run4 | 125 | 32 | 0 | 0 |
| run2 (static) | 0 | 0 | 0 | 0 |
| run5 | 0 | 0 | 0 | 0 |

### 2d. Pre-fix commit: not reproduced

At `0918622`, before `7fe56d0`, the plan and its execution judged the seat differently:

- **Plan:** `builds()`' `then` used `seats_of(piece, &visit.shown, &visit.terrain)`. The planned piece was *absent* from `shown`, so `stays_calm` judged her image without it.
- **Execution:** `seats()` ran after the piece existed, with it in `shown`, so her image grew by the piece's rectangle.

If text sat next to the piece, the Lounge seat would not be offered after crumpling, and `goal.take().filter(offers)` would drop it silently. HEAD fixed this with the planned piece's own cover (`mod.rs:1538-1550`).

But the stage room did not reproduce it before I stopped the run: 5 of 5 line-art handoffs succeeded. The playlist floor has no text next to the piece. I can't tell whether your binary predates `7fe56d0`. The test that would decide it is a room with text right beside the sofa's footprint, above its floor.

### 2e. Not measured

Whether a chat line cuts a makeshift Lounge short ("she sat, then stood straight up"). My `LOOK` note only fires when `goal` or `making` is set, and both are `None` after a successful handoff.

## 3. Would she sit on it later by ordinary scoring?

Yes, unless a real piece of the same kind is shown.

- **What a makeshift piece offers:** a sofa offers only `[Lounge]` (`room.rs:294`), a bed `[Sleep]` (`room.rs:293`). `Nap` is never offered for a makeshift sofa, so choosing `Use(Nap)` never picks it. `Watch` is added from a makeshift sofa when an unboxed TV is on the same floor (`mod.rs:1414-1437`).
- **What `build.then` can be:** Lounge, Sleep or Watch — the same uses she would pick again for the piece.
- **Score:** `Use(Lounge)` scores 8 × 0.5 = 4 (`brain.rs:168`), usually in the top 4.
- **Observed:** run2 has 1223 makeshift-Lounge starts across 70 pieces. Run1 seed 0 sat at 54 s, then again at 229, 312, 416 and 546 s. Run4 has 574 makeshift Lounge and 397 makeshift Watch starts.
- **With a real one shown:** `places_for` (`osaka.rs:2115-2162`) offers the makeshift piece only 1 time in `MAKESHIFT_ODDS = 20` (`osaka.rs:218`; covered by `tests.rs:3262`). In your situation (you own a real sofa), the made one will mostly be ignored once the handoff is gone.
- Run4's real sofa was in the closet (0 real Lounge starts), so it didn't exercise that 1-in-20 gate.

## 4. The general pattern

**What she intends is spread across separate `Option`/flag slots on `Osaka` (`osaka.rs:711-772`):**

| Slot | Holds |
|---|---|
| `task` | the job on this floor |
| `goal` | the job on another floor, or the "then use it" after a build |
| `making` | the build's purpose |
| `errand` | the accordion poke |
| `rest` | where she settled |
| `watch_until` / `watch_x` | watching the chat |
| `at_work` / `worked` | the part-time job |
| `pending` | scheduled layer ops |

**Each interrupt clears a different subset:**

| Interrupt | Site | Clears | Leaves |
|---|---|---|---|
| `decide` | 1825-1826 | `task`, `rest` | — |
| `look()` | 2264-2297 | `task` (unless falling or climbing) | `goal`, `making` |
| `recheck` | 1741-1775 | `task`, `rest` | — |
| `lost_grip` | 1809-1820 | `task` | — |
| `lost_seat` | 1720-1730 | `task` | — |
| `refused(Swap)` | 1660 | `task` | — |
| `shaken` | 2684 | `task` | — |
| `place()` | 2252-2262 | `task`, `goal`, `watch_until`, `at_work` | `making` |
| `head_for_errand` | 2549-2550 | `task`, `goal` (it drops a Make→Use handoff) | — |
| `errand()` while out | 2481-2483 | `at_work`, `task`, `goal` | — |
| crumple completion | 1195 | `making` (unconditionally) | — |

**How a plan is checked again:** only at `decide`, by exact equality against a fresh `Chances` (`osaka.rs:1859`). On failure it is dropped silently: no repair, no re-plan, no log.

- The `then` seat is computed at plan time against a counterfactual terrain (`mod.rs:1538-1550`). Its correctness depends on that hypothetical terrain matching the real one later, exactly the class `decisions.md` names: "a spot chosen against one picture of the screen and used against another".
- Nothing ties the slots together: `making` is not linked to the heap it belongs to (`visit.made`, owned by the guest in `mod.rs`).
- Matching across the actor boundary is done by recomputing `piece.seat(Crumple, 0)` on both sides (`osaka.rs:1196`, `mod.rs:691`, `mod.rs:1480`).

**By contrast, purposes kept in world state survive interruption:**

- a boxed parcel (`boxed`, `Unpack` base 40)
- the scrap's `stage`, which is why an interrupted heap is eventually finished
- the scheduled `Restore` ops for mischief (`pending`, `decisions.md` "mischief undoes itself on a schedule")

The failing purposes are the ones kept only in her head.

**Other multi-step behaviours open to the same loss:**

1. **Make → Use** (`making`): overwritten by a second build; stale after the piece falls apart (`tend_made`, `mod.rs:1446-1468` removes the piece but `making` stays); dropped by `head_for_errand`, though only via `goal`.
2. **Cross-floor Pull/Swap/Build/Use** (`goal`): dropped by exact-match validation (counted in 2c), and by `place()` and errands.
3. **Part-time job** (`at_work`/`worked`): an errand while out ends the shift (2476, 2481); `evict` special-cases it (`leaving_for_work`).
4. **Shopping → parcel → unpack:** robust, because each step is world state.
5. **Tear (reel-in):** `lost_grip` and `look` drop it; the guest unreels it (`mod.rs:740-752`). That loses the purpose but is visually consistent.
6. **Pull/Swap at a spot:** `look`/`shaken`/`refused` clear `task`. Swap's undo is safe in `pending`.
7. **Rest/recheck** (`rest`): fine today, but another separate slot with its own clearing rules.

## 5. Architectural observations

- **Keep purposes on the thing, not in a head slot.** Put the purpose on the world object it concerns (e.g. `Made { purpose: Use }`) or in an explicit plan/stack. The bug class then can't be represented: an interrupt can delay the plan but never silently erase or mis-assign it. Parcels and scrap stages already work this way.
- **Re-plan instead of exact-match validation.** Check that a plan can still be carried out by its post-condition ("a seat for Lounge on piece P exists") and let the planner pick a fresh concrete job, rather than `contains()` on a stale struct.
- **One owner for interruption.** Each interrupt should push or suspend the current intention on a single structure, rather than every `pub fn` clearing a hand-picked subset of fields.
- **Choosing vs. finishing.** The brain's needs × base scoring is fine for *choosing* what to do. The missing piece is a layer that *commits to and resumes* multi-step intentions. A planner (HTN/GOAP) with utility-scored top-level goals fits this split.

## 6. Scratch code

**Regression test.** Doesn't need the instrumentation; append to `tests.rs`. It fails at HEAD with `graphics=false seed=0: finished the sofa at 310028 and didn't sit on it`, and passes with `SKIP_BED=1` (the control):
```rust
/// What she means to make a piece *for* survives her being interrupted
/// while crumpling it: a chat line arrives mid-crumple (she looks up;
/// the heap stays unfinished), then she makes a bed. Whenever she does
/// finish the sofa, she sits on it within a few seconds. (Fails today:
/// `Osaka::making` is a single slot that the bed's tear overwrites.)
#[test]
fn an_interrupted_crumple_keeps_its_purpose() {
    use super::room::Use;
    for graphics in [false, true] {
        for seed in 0..4u64 {
            let at = format!("graphics={graphics} seed={seed}");
            let mut ui = stage_ui();
            let (real, mut view) = real_frame(&mut ui, 100, 30);
            let mut guest = Guest::new(seed);
            if graphics {
                guest.set_picker(kitty());
            }
            let mut now = 0;
            let step = |guest: &mut Guest, view: &IdleView, now: &mut u64| {
                *now += guest
                    .next_tick(*now)
                    .map_or(100, |d| d.as_millis() as u64)
                    .clamp(1, 100);
                guest.advance(*now);
                paint(guest, &real, view, *now);
            };
            fn visit(guest: &Guest) -> &Visit {
                match &guest.state {
                    State::Visiting(visit) => visit,
                    _ => panic!("visiting"),
                }
            }
            guest.cue(Scene::MakeSofa);
            paint(&mut guest, &real, &view, now);
            while !visit(&guest)
                .osaka
                .seat()
                .is_some_and(|s| s.what == Use::Crumple && s.item == Furniture::Sofa)
            {
                assert!(now < 60_000, "{at}: never crumpled the sofa");
                step(&mut guest, &view, &mut now);
            }
            let until = now + 2_000;
            while now < until {
                step(&mut guest, &view, &mut now);
            }
            view.chat_mark.synced += 1;
            step(&mut guest, &view, &mut now);
            if std::env::var("SKIP_BED").is_err() {
                guest.cue(Scene::MakeBed);
                paint(&mut guest, &real, &view, now);
            }
            // Live on until the sofa is finished, then give her 5 s.
            let sofa_done = |guest: &Guest| {
                visit(guest).made.iter().any(|m| {
                    m.piece.item == Furniture::Sofa && m.piece.scrap.is_some_and(|s| s.done())
                })
            };
            while !sofa_done(&guest) {
                assert!(now < 900_000, "{at}: never finished the sofa");
                step(&mut guest, &view, &mut now);
            }
            let finished = now;
            let mut sat = false;
            while now < finished + 5_000 && !sat {
                step(&mut guest, &view, &mut now);
                sat = visit(&guest).osaka.seat().is_some_and(|s| {
                    s.item == Furniture::Sofa && s.makeshift && s.what != Use::Crumple
                });
            }
            assert!(sat, "{at}: finished the sofa at {finished} and didn't sit on it");
        }
    }
}
```

**Natural-run harness.** Driven by environment variables. It needs `stage::stage_ui_n(n)`: `stage_ui()` becomes `stage_ui_n(40)`, with `(0..n)` in place of `(0..40)`. It also needs the instrumentation below. Run it with `cargo test -p dessplay --lib scratch_makeshift_handoff -- --nocapture`; nextest's 60 s timeout kills it.
```rust
/// SCRATCH: let her live naturally in the stage room (optionally with
/// chat arriving, a resident guest, and a real sofa + TV) and log what
/// happens to her intent around every makeshift piece she makes.
#[test]
fn scratch_makeshift_handoff() {
    let env = |k: &str, d: u64| {
        std::env::var(k)
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(d)
    };
    let (seed0, seeds, minutes) = (
        env("SCRATCH_SEED0", 0),
        env("SCRATCH_SEEDS", 4),
        env("SCRATCH_MIN", 20),
    );
    // 0 = no chat; 1 = chat mark only (she looks, frame unchanged);
    // 2 = chat mark and the chat really scrolls (a new line each time).
    let chat_mode = env("SCRATCH_CHAT", 0);
    // Mean milliseconds between chat messages.
    let chat_every = env("SCRATCH_CHAT_MS", 60_000);
    let resident = env("SCRATCH_RESIDENT", 0) == 1;
    // Own a real sofa (Playlist pane) and TV (Users pane)?
    let furnished = env("SCRATCH_HOME", 0) == 1;
    let modes: Vec<bool> = match std::env::var("SCRATCH_GFX").as_deref() {
        Ok("0") => vec![false],
        Ok("1") => vec![true],
        _ => vec![false, true],
    };
    let sizes: Vec<(u16, u16)> = match std::env::var("SCRATCH_SIZE").as_deref() {
        Ok("100") => vec![(100, 30)],
        Ok("80") => vec![(80, 24)],
        _ => vec![(100, 30), (80, 24)],
    };
    for &graphics in &modes {
        for &(w, h) in &sizes {
            for seed in seed0..seed0 + seeds {
                let mut lines = 40;
                let mut ui = super::stage::stage_ui_n(lines);
                let (mut real, base) = real_frame(&mut ui, w, h);
                let playing = env("SCRATCH_PLAYING", 0) == 1;
                let mut view = IdleView {
                    resident,
                    busy: playing.then_some(Busy::Playing),
                    focus: playing.then_some(base.chat),
                    ..base.clone()
                };
                let mut guest = Guest::new(seed);
                if graphics {
                    guest.set_picker(kitty());
                }
                if furnished {
                    let _ = guest.ledger.home.add(
                        Nook::Playlist,
                        room::Prop {
                            item: Furniture::Sofa,
                            at: 300,
                            facing: sprite::Facing::Right,
                            boxed: false,
                        },
                    );
                    let _ = guest.ledger.home.add(
                        Nook::Playlist,
                        room::Prop {
                            item: Furniture::Tv,
                            at: 800,
                            facing: sprite::Facing::Left,
                            boxed: false,
                        },
                    );
                }
                let mut chat_rng = Rng(seed.wrapping_mul(7919).wrapping_add(13));
                let mut next_chat = chat_rng.range(chat_every / 4, chat_every * 7 / 4);
                let mut now = 0;
                paint(&mut guest, &real, &view, now);
                let (mut seen, mut visit_no, mut was_visiting) = (0usize, 0u32, false);
                let end = minutes * 60_000;
                while now < end {
                    now += guest
                        .next_tick(now)
                        .map_or(1000, |d| d.as_millis() as u64)
                        .clamp(1, 1000)
                        .min(next_chat.saturating_sub(now).max(1));
                    if chat_mode > 0 && now >= next_chat {
                        next_chat = now + chat_rng.range(chat_every / 4, chat_every * 7 / 4);
                        view.chat_mark.synced += 1;
                        if chat_mode == 2 {
                            lines += 1;
                            let mut ui = super::stage::stage_ui_n(lines);
                            let (r, v) = real_frame(&mut ui, w, h);
                            real = r;
                            view = IdleView {
                                resident,
                                chat_mark: view.chat_mark,
                                busy: view.busy,
                                focus: view.focus.map(|_| v.chat),
                                ..v
                            };
                        }
                        if let State::Visiting(_) = &guest.state {
                            eprintln!(
                                "LOG gfx={graphics} size={w}x{h} seed={seed} visit={visit_no} t={now} pos=(0,0) made=0 CHAT_MSG"
                            );
                        }
                    }
                    guest.advance(now);
                    paint(&mut guest, &real, &view, now);
                    match &guest.state {
                        State::Visiting(visit) => {
                            if !was_visiting {
                                visit_no += 1;
                                seen = 0;
                            }
                            was_visiting = true;
                            let log = &visit.osaka.log;
                            for (t, line) in log.iter().skip(seen) {
                                eprintln!(
                                    "LOG gfx={graphics} size={w}x{h} seed={seed} visit={visit_no} t={t} pos=({},{}) made={} {line}",
                                    visit.osaka.x,
                                    visit.osaka.y,
                                    visit.made.len()
                                );
                            }
                            seen = log.len();
                        }
                        _ => {
                            if was_visiting {
                                eprintln!(
                                    "LOG gfx={graphics} size={w}x{h} seed={seed} visit={visit_no} t={now} pos=(0,0) made=0 VISIT_ENDED"
                                );
                            }
                            was_visiting = false;
                        }
                    }
                }
            }
        }
    }
}
```

**Instrumentation (test-only, in `osaka.rs`).** The full diff is in `instrument.patch`.

- A field `#[cfg(test)] pub log: Vec<(u64, String)>`, initialised in `new`.
- A helper:
  ```rust
  #[allow(unused_variables)]
  fn note(&mut self, at: u64, what: impl FnOnce() -> String) {
      #[cfg(test)]
      self.log.push((at, what()));
  }
  ```
- Hook sites:

| Note | Where |
|---|---|
| `CRUMPLED seat=.. making_matched=..` and `GOAL_SET ..` | crumple completion (~1195) |
| `TORN_ALL` | before `LayerOp::Make` (~1245) |
| `DECIDE_EARLY errand / no_platform / find_rest / watch` | each early return in `decide`, only when `goal` is set |
| `GOAL_CHECK offered=.. goal=.. same_item_seats=[..]` | before `goal.take()` |
| `GOAL_TAKEN` | after a successful `go_to` |
| `RECHECK_STARTLED act=..` | `recheck` |
| `LOST_SEAT` | `lost_seat` |
| `LOST_GRIP` | `lost_grip` |
| `PLACE_CLEARS_GOAL` | `place` |
| `LOOK act=.. goal=.. making=..` | `look`, when `goal` or `making` is set |
| `USE_START seat` | arrival at a Use seat |
| `BUILD_CHOSEN ok kind piece left floor then` | the `Place::Make` arm of `start` |

`tests.rs` also has `scratch_an_interrupted_crumple_keeps_its_purpose`, a log-reading version of the regression test (the 16/16 result). It has 8 seeds and a 10-minute tail, and prints whether each finished piece carried its plan and how long until she next sat on it.

## Files

The instrumentation, harnesses and run logs lived in a throwaway worktree and scratchpad during the 2026-10-02 review and were not kept. The regression test above is self-contained; the natural-run harness needs the instrumentation re-added (the table above lists its hook sites).
