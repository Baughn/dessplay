# Phase 5b design critique: persistence, the shell and operations

I checked everything against HEAD (`5fc9310`, the same code as the map's `0661710`). Paths are relative to `dessplay/src/ui/houseguest/` unless they have a prefix. I edited no files.

## BLOCKER

**B1 — The `GameClock::at` formula underflows (D1 "Reading").**
- `fn at(&self, t) { game + 6 × (t − at) }` uses u64, with `at = clock_at` set at the top of `advance`.
- `Osaka::tick` (osaka.rs:1668-1700) fires each decision at `due <= now` (`if due > now { return }`), so every catch-up decision has `t ≤ clock_at` and `t − at ≤ 0`.
- Debug and dev builds panic on overflow on the first catch-up decision. Release builds wrap.
- The same happens in tests that `paint` at an earlier `now` than their last `advance`.
- **Amendment:** "`at(t)` is computed in i64, `game as i64 + 6·(t as i64 − at as i64)`, clamped at 0. Equivalently, `game − 6·(at − t)` when `t < at`. Decisions earlier than the anchor read earlier game time."

## MAJORS

**M1 — The std docs say the opposite of what D1 asks the builder to cite (D1 "No clamp").**
- The implementation does what D1 says. Toolchain `rust-default-1.98.0-nightly-2026-06-13`, `rust-src .../library/std/src/sys/time/unix.rs:66-70`: `CLOCK_ID = libc::CLOCK_MONOTONIC` on non-Apple targets and `CLOCK_UPTIME_RAW` on Apple. The Apple clock's own comment says it "does not increment while the system is asleep".
- The documented contract doesn't promise it. `library/std/src/time.rs:56-57`: "it is also not specified whether system suspends count as elapsed time or not. The behavior varies across platforms and Rust versions."
- So "the builder confirms this against the std docs and cites it" can't be done. "No clamp" rests on an implementation detail that a toolchain bump could change. If it did, a laptop closed overnight would add about two game days.
- The code already expects such jumps: osaka.rs:1702 says "Far behind (a suspended laptop): resume from now."
- I could not check Windows, where `Instant` is QueryPerformanceCounter. The crate does build there (`main.rs:15`, `Cargo.toml:58`).
- **Amendment:**
  - decisions.md cites the implementation (path and toolchain) and says plainly that the docs disclaim it.
  - Add `Guest::cap_steps(max_ms)`. The shell sets it (say 10 min, which a loop running at least once a second never reaches honestly) and logs at debug when the cap triggers.
  - The cap defaults to `None`, so `a_busy_client_gets_no_visit` (tests.rs:137, a 1 h jump), `tests.rs::run` and the censuses stay unclamped.

**M2 — The `idle_min` predicate is always true (D6 "Pity").**
- `open && now ≥ quiet_since`: `quiet_since` is only ever set to `now` (`activity` mod.rs:652, `observe` mod.rs:1309 and 1338, and also 784, 987), so `now ≥ quiet_since` always holds.
- That leaves `open` alone (idle.rs:104-111): the setting is on, and the client isn't busy or she is resident.
- So a visitor-mode user typing in chat accrues "idle" minutes, which is not what HG's "cumulative idle hours" means.
- **Amendment:**
  - "Idle means the arrival gate is open: `open && now ≥ quiet_since + delay`, one `fn gate_open(now)` used by both `advance`'s Absent arm (mod.rs:703-711) and the pity accrual."
  - State the consequences. A resident's hands-off playback counts as idle. Pity pauses while Visits is Off (`open` needs `delay.is_some()`) even though the clock runs.
  - `idle_min` needs its own real-ms remainder (`idle_rem`).

**M3 — What `clock_at` does while the clock is gated isn't specified (D1 "When it runs" and "Accrual").**
- D1 says "the first call only latches" and that the clock runs "only once visits > 0", but not whether `clock_at` keeps latching while `visits == 0`.
- If it doesn't, the first `advance` after `begin_visit` adds the whole process uptime × 6 in one step. That `begin_visit` runs in `paint`, which bumps `visits` at mod.rs:1147-1148.
- The same happens after `move_out` (mod.rs:451-461, back to `visits == 0`) at the next first meeting. With an hour of uptime, the first meeting would land around 22:00.
- `idle_min`'s gating isn't stated either. If it accrues at `visits == 0`, then "a client that never met her writes no record" breaks.
- **Amendment:** "`accrue(now)` always sets `clock_at = Some(now)`. Only adding to `clock`, `clock_rem`, `idle_min` and `idle_rem` is gated on `visits > 0`."

**M4 — Holding the CHANGELOG until step 9 breaks CLAUDE.md (Steps; Docs that change).**
- CLAUDE.md says each user-visible change adds its entry "in the same commit".
- Steps 4 (night sleep), 5 (Away), 6 (calendar), 7 (rares) and 8 (window and clock) each ship visible behaviour on `master`. Startup's "What's new" is how users learn why she's "at school".
- **Amendment:** each step's commit carries its own entry. Step 2's entry covers "existing homes start at Monday 16:00". Step 9 only reviews the wording.

**M5 — The date doesn't reach `advance`-time decisions, and the shell's date source is undefined (D5 "The date"; D2; D4).**
- (a) `biblical_date` (timeutil.rs:10-14) takes epoch ms and reads `Local` itself. The shell has only `now_millis()`, which counts monotonic ms from process start (shell.rs:707-713). "`now_utc_ms`" names nothing.
  - Passing `now_millis()` would give 1969-12-31, and "Dec 1–31 seasonal musings" would be on forever.
  - `system_clock()` (run.rs:92-99) is private and `unwrap_or(0)`s, which gives the same 1969 date.
- (b) The guest can learn `local` only through `observe` in `paint`. Several things need it during `advance`:
  - `DayTime.school_day` (vacation);
  - the slot-change save batching;
  - the boundaries `next_tick` wakes at in Absent and Away;
  - the calendar "owed at decision time".

  While she's asleep or Away on an idle client, `paint` can be minutes apart, so the date is stale across 09:00. Also, the only clock carrier into `tick` is `Option<GameClock>`, which has no date.
- **Amendment:**
  - "The shell computes `Option<NaiveDate>` from `SystemTime::now()` through `biblical_date`, mapping a pre-epoch error to `None`. It hands the date to the guest with `guest.set_date(d)` before every `advance`, and the same value goes into `view.local` for the example."
  - "`tick` takes `Option<Clock { game: GameClock, date: Option<NaiveDate> }>`."
  - Drop the per-minute cache. `draw()` is a stateless free fn called from three sites (shell.rs:447, 489, 617), and chrono 0.4.45 already re-checks the zone at most once a second (`offset/local/unix.rs:107-116`).

## MINORS

**m1 (D1 "When it runs").** "Her first meeting is at 16:00 plus the idle delay" is wrong under the gate. The clock is frozen until `begin_visit`, so the first meeting is at exactly 16:00.

**m2 (D1 "Exit save").** `touched` is used but never defined.
- Proposal: `final_ledger(now)` accrues, then returns `(persist && ledger != loaded).then(clone)`. `loaded` is a clone kept at `restore`, which works because `Ledger: PartialEq`.
- This covers events, accrual, a parked `ledger_unsent` and `move_out`. A client that never met her still writes nothing.

**m3 (D1 "Exit save").** "Every exit" holds for the UI side only.
- run.rs's `?` returns at 985, 988, 1013, 1016, 1020-1021, 1051 and 1062 never join the UI thread.
- The interactive client handles no signals; the only `ctrl_c` handler is the seeder's (run.rs:616). Closing the terminal window sends SIGHUP and loses the current batch, which makes it the common unclean exit.
- A handout parked in `ledger_unsent` at crash time can double the "≤ 30 game minutes" loss.
- Say all this, or add a SIGHUP/SIGTERM handler that routes into the session's Quit.

**m4 (D1 "Exit save").** Moving the loop body into an inner fn needs about 9 `&mut` parameters (ui, renderer, custom_enabled, watcher, guest, ledger_unsent, inputs, actions, adapter).
- That trips `clippy::too_many_arguments`; there's no `clippy.toml`, so the default threshold applies, and the stop hook runs `-D warnings`.
- Use a labelled `'ui: loop` instead. `run_ui_loop` returning `Option<Ledger>` makes a stray bare `return;` a compile error.
- The perf.rs:426 and 608 call sites are statements and stay fine. `Ledger` is public (ui/mod.rs:8, mod.rs:92).

**m5 (Steps 1).** "A regression test first: the Quit iteration loses a `Bought`" can't be staged through the honest interface: it needs an rng-driven purchase in the last `advance`.
- The fix is structural, which per CLAUDE.md makes the test optional.
- Write an exit-path table test instead (Quit key, `UiInput::Shutdown`, dropped input sender, closed channel on `SearchResults`) that asserts the returned ledger, including one parked in `ledger_unsent`.
- The draw-error path probably can't be tested with `TestTerminalAdapter`; say so.

**m6 (Steps 2 vs 3).** Step 2's slot-change batching and the stage's `t` key (skip to the next slot boundary) both need `routine.rs`, which arrives in step 3. Move them, or batch on the 30 minutes alone in step 2.

**m7 (D1 "Saving").** Batching is implementable without dirtying every tick, but it needs a stated field.
- Add `clock_saved: u64`. Reset it whenever `ledger_to_save` *takes* `unsaved`, whether or not it hands anything out. Otherwise, with `persist == false`, or in the stage and goldens that never drain, the threshold trips on every tick.
- `move_out` resets it (or use `saturating_sub`).

**m8 (D1 "Reading").** The GameClock's anchor before the first `advance` is unspecified.
- Tests do `cue(Arrive); paint(..,0)` with no `advance` (tests.rs:2216-2220), so `clock_at` is `None` at `begin_visit`, and `set_clock` and `tucked_in` can't fire there.
- Make it `GameClock { at: clock_at.unwrap_or(now), .. }`, and say `Some` whenever `visits > 0`.

**m9 (D3 `next_tick`).** Waking at the next boundary in Absent must be gated on the clock running.
- `a_busy_client_gets_no_visit` asserts `next_tick(0) == None` for an Absent `Guest::new(7)` (tests.rs:135-136).

**m10 (D1 pins).** `a_goodbye_mid_carry_leaves_the_piece_where_it_stood` (tests.rs:7350-7395) compares `record_of` = `to_json` across advances, so it fails once the clock serializes.
- Name it, and compare `home` and `ordered` there instead.

**m11 (D1 storage; D6; D5).** The lenient read needs:
- `seen` read as `Vec<Value>` filtered per entry, and `calendar_on` as `Option<Value>`. `Option<String>` or `Vec<String>` in `Raw` would fail the whole record on one bad element.
- Clamp `clock` on read (say below 2^40), or use saturating math. A garbage `u64::MAX` overflows `× 60_000` and panics the UI thread.
- `rare_at` and `legend_at` use `saturating_sub` against `idle_min`.

**m12 (decisions.md "the downgrade loss").** A `stable` build loses more than the clock.
- `Raw` drops `Furniture` variants it doesn't know (ledger.rs:91-121) and an `ordered` it doesn't know (122-125), so the window and clock pieces and a pending order vanish on its first save.
- `clock_sent` resets too, so the clock parcel is sent again on return, which heals itself. Say so. decisions.md:2083-2084 already states the "loses a piece, not the home" principle.

**m13 (D7 `clock_sent`).** The check's site and its guard aren't specified.
- Make the site `begin_visit`, before shopping.
- The condition needs `!home.owns(Clock)` if the clock is also in `CATALOGUE` (D7 says both pieces are appended), and should say whether it sets `bought_on` (mod.rs:477, 1356; `SHOP_EVERY` at mod.rs:126).

**m14 (D5 "marked delivered when shown").** If the mark is written in `paint` outside the furnish diff (mod.rs:895-911), it must dirty the ledger explicitly.
- Cleaner: when the first key plays, `tick` emits `HomeEvent::Calendar` and `HomeEvent::Seen`, and `record` consumes them in `advance` (mod.rs:727).

**m15 (Steps 9, dump).** `--dump` runs without the instance lock, beside a live client (run.rs:2571).
- The `houseguest` section therefore shows the last batched save, up to 30 game minutes stale. Label it "as of the last save".
- It needs:
  - a `pub` summary accessor, since `Ledger`'s fields are `pub(super)`;
  - a date to compute the slot (use `biblical_date` of the system time);
  - an entry in `dump::SECTIONS` (dump.rs:32);
  - an update to the dump-state skill.
- An unreadable record reports its error without failing the dump.

**m16 (Docs that change).** design.md's Settings bullet (1902-1905) and the move-out prompt ("her room and everything she owns", modals.rs:1521) don't mention that her clock, pity and seen rares reset.
- The "Her record" bullet (1906-1917) should list the new fields, and say the clock doesn't run before a first meeting.

**m17 (D1 "Stage").** `Guest::new(seed)` starts with visits at 0, so the stage bar reads 16:00 until she's cued in (examples/houseguest.rs:94-102).

## Left for the builder to guess

- The order of the new `Saved` fields after `unsettled`, and a `the_record_as_written` twin with every field non-default.
- Whether accrual in `final_ledger` reuses a private `accrue(now)`. D1 says accrual happens "in `advance` only".
- Whether the slot-change save fires when only the real date toggles vacation (at 09:00), not the clock.
- The perf test (perf.rs:591-631) is the only test that sees the real date. Say it must pass on every starter date, or let the shell take a date override.

## The lens questions, answered

- **Is "runs once visits > 0, whatever the setting" right?** Yes, for restore, new and `keep_unsaved` alike, with two fixes: re-latch while gated (M3), and gate `idle_min` the same way. The first meeting is then at exactly 16:00 (m1).
- **Can batching avoid dirtying every tick?** Yes, given the `clock_saved` base reset on every take (m7).
- **Can the final-ledger return be structural?** Yes, with a labelled loop rather than an inner fn (m4). It covers every UI-side exit and both joins, before `Rejected` and before `exec_self`; run.rs's `?` exits and SIGHUP stay unsaved (m3).
- **Is `Instant` CLOCK_MONOTONIC, excluding suspend?** In this toolchain's source, yes (CLOCK_UPTIME_RAW on Apple), but the docs say it's unspecified (M1). I could not check Windows.
- **LocalTime via `biblical_date`:** the source of the epoch ms is undefined, and the date reaches only `paint` (M5).
- **Is `idle_min` "idle"?** Not as written (M2).
