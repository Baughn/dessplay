# Phase 5b design critique: mechanics and correctness against the code

I checked the design against HEAD `5fc9310`. Paths are under `dessplay/src/ui/houseguest/` unless prefixed. Every cite below was read in the code.

## BLOCKERS

**B1. Input or chat cancels the return (D3, row "Away → Arriving").**
- **Problem.**
  - In the input path, the shell calls `guest.advance(now)` and then `guest.activity(now)` (shell.rs:505, 587).
  - `activity`'s resident arm sends `Arriving` to `Absent` (mod.rs:664).
  - `paint` runs `observe` first (mod.rs:766). Its chat arm sends `Arriving` to `Absent` and sets `quiet_since` (mod.rs:1337-1339).
  - So if Away→Arriving fires in the same frame as a resident keystroke or a chat line, the return is dropped.
  - The design's "own predicate" exists only while the state is `Away`. From `Absent`, she waits for the idle gate, which a live chat keeps resetting. That breaks D3's "the return isn't reset by input or chat".
- **Amendment.** Make it `State::Arriving(How)`, with `How = Idle | Return | Dash`. `activity`'s resident arm and `observe`'s chat arm leave `Arriving(Return | Dash)` alone. Only `!open` cancels it.

**B2. The routine boundary in `due()` and `tick` (D4, "Who owns a boundary").**
- **Problem 1: `due()` can't see the clock.** `Osaka::due(&self)` takes no clock (osaka.rs:1547), and `next_tick` calls it bare (mod.rs:755). The boundary would have to be stored on `Osaka`, and the design doesn't say so.
- **Problem 2: the wakeup fires the wrong thing.** Inside `tick`'s loop (osaka.rs:1671-1703), a due that isn't speech, pending or blink falls through to `fire(due, …)`. `fire` acts no matter what `act_due` says:
  - `Clamber` steps (1730-1742);
  - `Out` steps and draws rng (1750-1757);
  - `Away` brings her back in (1771-1785).
  So a boundary wakeup moves her a cell early, or ends a shift early.
- **Problem 3: a busy loop.** When the interrupt is skipped ("hidden or departing") and the stored boundary isn't advanced, `due()` stays ≤ `now`. The loop then spins 64 times per tick, `next_tick` returns 0, and the shell (shell.rs:477-485) pins a core.
- **Amendment.**
  - Store `Osaka.cut_at: Option<u64>` (monotonic), refreshed from the `GameClock` at every `tick` entry.
  - In the loop, check `due == cut_at` first, before speech, pending, blink and fire.
  - Always advance `cut_at` to the next cutting boundary strictly after `due`, whether or not it interrupts.
  - Recompute it after `Guest::skip_clock`.

**B3. Chat watching beats the routine reflexes, so D4's "unreachable through any path" is false (D4.4, reflex position).**
- **Problem.**
  - D4 puts the routine reflexes after the `Ctx` literal (osaka.rs:3008).
  - `watching chat` comes earlier (2961-2969): it is a `Stand` of up to 5 s, re-armed by every chat line through `watch_until = now + WATCH_MS` (15 s; osaka.rs:272, 3873).
  - So any chat with lines under 15 s apart keeps her standing at 08:15 or 22:30 for as long as it lasts. "Home at 10:00 on a school day" and "awake at 02:00" are both reachable.
  - Owed beats (2987) also come earlier, but they are one-shot and harmless.
- **Amendment.**
  - Run the routine reflexes right after `off text` (2955), before `watching chat`.
  - Build the `Ctx` there (it reads only `here`, `terrain`, `chances` and `self`), or build a local one for `plan`.
  - On the routine path she still turns toward `watch_x`.

**B4. Tucked in on a sofa, and the night's fallbacks (D4, "Tucked in" and `routine/bed`).**
- **Problem 1: the sofa can't host a sleep.** Sofa uses are `[Lounge, Nap]` (room.rs:122); `Sleep` is bed-only (148). So `Act::Use { Sleep, … }` on a sofa seat is an act with `seat.what` ≠ its play. Sofas are the commonest early home (CATALOGUE starts with them, mod.rs:101-111).
- **Problem 2: the fallbacks don't last.** "Nothing else lengthens", so `Use(Nap)` (30–60 s, osaka.rs:918), the makeshift sleep and `Idle(LieBack)` all end in seconds. Each end goes through `decide`, then the bed reflex again: she gets up and lies down every minute all night, and each `start_job` re-rolls splices (osaka.rs:2377).
- **Amendment.** Tucked in, and the bed reflex, use whatever bind succeeds (Sleep, else Nap, else makeshift, else LieBack). All of them get the wake-time `until` inside the Asleep slot.

## MAJORS

**M1. `interrupt` has no guard on the act (D4, "Who owns a boundary").**
- **Problem.** `interrupt` swaps in `Act::Look` whatever she's doing (osaka.rs:3940-3957). `look` and `errand` both guard against this: `Back` and `aloft` at 3889-3895, and 4135-4137.
- **What a routine cut does today, per act:**
  - Climb, Fall or Clamber (`OnChat::Landed`, 713-715): a Look in mid-air.
  - `Door` mid-beat: she pops into view at the near door.
  - `Poke`: a second "Somebody said something." when the errand reflex re-pokes (2945, 4245-4255).
  - `Use(Sleep)` at 22:30: she's startled awake, then goes back to bed.
- **Amendment.**
  - Cut only acts whose props are `Stays::Rest | Stays::Job` (690-702), and never `Poke`.
  - Skip `Landed` and `Back`. They end soon in `decide`, which hits the reflex anyway.
  - At bedtime, if she's already in a bed `Use(Sleep)`, extend `until` to the wake time in place.
  - Give the new `Cause::Routine` arm `(0, 0)`: no startle (the match at 3941 is exhaustive, so it has to say).

**M2. The night-sleep needs fix still saturates them (D4, "Needs").**
- **Problem 1: ×0.25 isn't enough.** Over 85 real minutes, ×0.25 is about 21 effective minutes. The rise times are Restless 90 s, Tidy 60 s, Mischief 4 min, Fun 6, Comfort 8, Daydreams 10 and Hungry 20 (brain.rs:65-78). So `pass` (293-309) still wakes her with every need except Sleepy, Beauty and Nesting at 1.
- **Problem 2: `asleep_ms` can't be computed where the design needs it.** It can't be derived at `decide` from `self.act` when the sleep was cut. An errand or chat goes through `set`, which runs `credit_done` (1715), so by the next `choose_next` (2976-2982) the act is `Door`, `Walk` or `Poke`.
- **Amendment.**
  - Accumulate `Osaka.slept_ms` in `credit_done` whenever she leaves a night `Use`, and consume it in the next `pass`.
  - On waking from the night sleep, set needs to the Morning arrival levels through `set_clock`, not `pass`.

**M3. The clock doesn't reach the sites that read it (D1 "Reading", D4 "as `Option<DayTime>` on `Ctx`").**
- **Sites that run before or outside `Ctx`:**
  - `needs.pass` (2981; `Ctx` is at 3008);
  - the greeting, which D4 wants to become "Mornin'..." (2971);
  - `start_job`'s length draw (2431; its signature at 2331 carries no clock);
  - `SpliceCtx { what, trying, quiet }` (script.rs:615-624), which Scary's 22:00 `when` needs;
  - `look`, for Stir;
  - `due`, for B2.
- **The view never reaches the mind.** `advance(now)` has no view, so `local` and `vacation` (which `slot()` and `school_day` need) never reach `tick`.
- **Amendment.**
  - At `tick` entry, set `Osaka.clock: Option<GameClock>` and `Osaka.vacation: bool`, latched from `observe`'s `view.local`.
  - Add one `fn day(&self, at) -> Option<DayTime>`.
  - Threading the clock as a parameter would mean adding it to `fire`, `decide`, `choose_next`, `plan`, `start_job` and the 15 `decide` sites.
  - Add `SpliceCtx.day`.

**M4. A vacation flip changes the slot with no wakeup (D2, Q4).**
- **Problem.** `local` changes at real 09:00 (`biblical_date`), mid-way through a game day. On Jul 20 a game 07:30 Morning becomes Asleep-until-09:00. On Sep 1 a vacation Morning becomes Away. No boundary fires for either, and `cut_at` and `next_tick` were computed with the old flag.
- **Amendment.** Latch `vacation` once per game day, at game 00:00 or the day's first read. Slots are then a pure function of game time within a day.

**M5. Chat::Stir can't be told apart, and `look` won't route to it (D4, "Chat at night").**
- **Problem.**
  - `on_chat(self)` is keyed on `ScriptId` alone (script.rs:276-294).
  - The night and day sleeps share `ScriptId::Sleep`: `Play::of(Use::Sleep)` builds it, and so does tucked-in's `own: Sleep, branch: 1`. Branch 1 is the trial branch (script.rs:299-301, 984-990).
  - `answer()` reads only `spliced_at` (3917-3933), and `look` consults it only `if asks` (3897).
- **Amendment.**
  - Add a new `ScriptId::Night` for the night sleep, with `on_chat` → `Stir`.
  - In `look`, after the `aloft` check, check the own script's `Stir` whatever `asks` is, then return.
  - Extend `answer()`'s match.

**M6. Dream and sleep-talk have no mechanism (D4 "Sleep-talk", D6 `Dream`).**
- **Problem.** Splices are only a prelude or a coda (`Part`, script.rs:580-600; `splices` 663). A coda on the night sleep plays at the wake time, not "after 30 min". Nothing schedules a line every few minutes inside a `Use`; keys carry pose, face and bubble only.
- **Amendment.**
  - Either `Night`'s body keys use `Span::Ms` for timed talk keys, with Dream as a rolled key branch.
  - Or add a `next_talk` due on `Osaka`, fed from the pool, and added to `due()`.
  - Say which.

**M7. The departure exit as written draws rng and comes back (D3 row 1, D4 `routine/away`).**
- **Problem.**
  - "`go_to_work`'s exit with no rng" still hits `Out`'s draw (1753-1757). `Away { until }` then walks her back in (1771-1785).
  - The door arm's `gap` is drawn from rng (4365).
  - "Off screen" isn't defined. Ending at the first `!beat.her` cuts the door's closing beats, because the gap beat is the one with `door: None` (816-839, 4093-4098).
  - Hidden at work at 08:15, she isn't interrupted. She returns with leeks saying "I'm home!" (4381-4393), then "I'm off!".
- **Amendment.**
  - Add `Osaka.leaving: Option<Routine>`.
  - `Out` with `leaving` set: `Away { until: u64::MAX }`, no draw.
  - The door arm: `gap = u64::MAX`.
  - The guest ends the visit when `leaving.is_some() && hidden(now) && door(now).is_none()`.
  - At work when Away starts, end the visit too.

**M8. Parcels in dash-in and departing visits (D3, "Parcels").**
- **Problem.** A dash-in is `State::Visiting`, so `furnish` runs. The first-TV order sets `bought_on = visits - 1` (mod.rs:1680-1682), so it is delivered at once (1684-1693), mid-dash or behind a closed door. That says `PARCEL` while she's hidden: map trap 3, still unfixed. The D7 clock order does the same unless it sets `bought_on = visits`.
- **Amendment.**
  - Deliver only when `!osaka.hidden(now)` and `visit.kind != Dash`.
  - The clock order sets `bought_on = visits`.

**M9. Dash and errand visits need a kind, and their end depends on the gate (D3a).**
- **Problem.** Nothing marks a `Visit` as uncounted. The "ends by door → Away (furnished)" rule would show the empty home to a visitor who is typing (the errand row "from Absent").
- **Amendment.**
  - `Visit.kind: Normal | Dash`.
  - At the end, go to Away only if `open && quiet` and she has a home; otherwise go to Absent.

**M10. The visit's end clears her made pieces and moved text (D3 row 1).**
- **Problem.** Dropping the `Visit` (mod.rs:193-204) drops `made`, `layer` and `fades`. So her makeshift furniture blinks out and moved text snaps back the moment her door closes. Visible on every school morning.
- **Amendment.** Say so, or rain them out with a `Dissolve` over the `made` cells kept in `Empty.fades`.

## MINORS

- **N1, the furnish claim (D3).** `furnish` uses rng only for `place_gift` (mod.rs:1696), and it can't run outside `Visiting` anyway (it takes `&mut Visit`). The real issue: `Home::project` mutates the ledger (`pin_anchors`, `move_off`; room.rs:972-981, 1067). Away's paint must diff and mark the ledger unsaved, as at mod.rs:895-911.
- **N2, `Empty`'s fields (D3).** `Empty` also needs `painted: Vec<Frozen>` and `size`, for `Dissolve::new` (mod.rs:676-682) and the focus rain (864-882).
- **N3, Away and overlays (D3).** "Overlay → Absent at once, as Visiting does today" is wrong: an overlay makes Visiting `leave` (mod.rs:1318). Only visits-off is instant (1312-1315). Pick one on purpose.
- **N4, Away and chat (D3).** Away's reaction to a chat line isn't stated. Copying the Absent arm (1337-1339) would send her to Absent on every line. Say "no change".
- **N5, dash-in timing (D3a).** "Due" must be a crossing (`prev < T ≤ now`) inside this away period, or a restart after T fires a second dash-in.
- **N6, first meeting (D1).** If the clock runs only at `visits > 0`, the first meeting is at 16:00 flat, not "16:00 plus the idle delay".
- **N7, sleep length (D4).** "At most about 85 min" is wrong: a weekend night (23:30–09:00) is 95, and Sunday into Monday is 105.
- **N8, where tucked-in goes (D4).** "After the first furnish" means inside the Visiting arm: after `visit.chances` (mod.rs:1042-1061), so `seats` exist, and before `covers`/`with` (1064). It must set `credit = Some(Want::Use(..))`, or the sleep serves nothing (`credit_done` returns at 2540; Sleep serves Sleepy 0.7, brain.rs:488-490). Also say `whole`, `grievance: None` and the `recent` push.
- **N9, rounding (D3).** The `next_tick` boundary must round up: ⌈(boundary − game)/6⌉. Rounding down wakes 1 ms early, then gets a zero duration.
- **N10, the hidden-goodbye fix (D3).** It must clear `placement`, not just `with`. `draw_art` pushes the door layer while she's hidden (mod.rs:2557-2561), so `paint_layers` still returns `Some`.
- **N11, where the reflex goes (D4).** The reflex fires before heading continuation, so multi-hop trips to bed re-plan at every hop. That's fine, but say so, and say the routine reflex drops `episode` (`Letting::Carry`).
- **N12, the return without a home (D3).** From Absent with no home, the 12:45 return is the normal `Osaka::arrive`, which can drop from the sky. Say whether "I'm home!" and the no-drop rule apply there too.

## Unspecified (a builder would guess)

- **The return constructor.** None exists today:
  - `arrive` is a walk-in or a drop (osaka.rs:1281-1330).
  - The door spot is unspecified: `calm_elsewhere`?
  - The door start should be `since = now − DOOR_THROUGH_MS`, as `arrive_for_errand` does (4116-4125).
  - "I'm home!" without leeks needs a flag: `home_from_work` keys on `at_work` (4382).
  - It must keep the reseed at mod.rs:776.
- **The departing flag (`leaving`).** Who clears it: `place` (3857)? `errand` (4145, 4150)? `evict` (4308-4322)?
- **`DashLunch`.** Its `ScriptId`, and its `played_on` host for the script lints. Its seat when there's no fridge. Whether its act sets `credit`.
- **The groggy errand.** The `Face::Blink`-while-walking flag in `appearance`, and whether `poke` says `POKE` or "...mm?".
- **`whole` for a lengthened sleep or homework.** The draw, or the overridden length? This changes `credit_done`'s share when it's cut (2567-2569).
- **A tucked-in arrival when `Osaka::arrive` returns `None`.** Today that's Absent at mod.rs:777-785, though she only needs the bed.
- **A resident who is Absent at the 12:45 boundary** (after an overlay or a restart). Does the routine return apply, or the idle gate?
- **The stage.** `cue` from Away goes to `Arriving` (mod.rs:532); say which `How`. `skip_clock` must refresh `cut_at` (B2) and any stretched `until`.
- **The calendar's owed check in `tick`.** It needs the latched `local` and an in-Osaka "played today" flag. `record` runs only after `tick` (mod.rs:724-727), so a 64-step batch would re-offer the item.
