# Proposal: The Houseguest's mind and home

Status: **DRAFT 2026-10-02 — for discussion; nothing implemented**

Supporting notes: [the sofa diagnosis](2026-10-02-houseguest-mind/sofa-diagnosis.md) (with the phase-0 regression test), [character-AI survey](2026-10-02-houseguest-mind/research-character-ai.md), [layout survey](2026-10-02-houseguest-mind/research-layout.md), [the full home model](2026-10-02-houseguest-mind/alternative-full-home-model.md) (alternative C).

Paths are relative to `dessplay/src/ui/houseguest/`, and line numbers are at `654dd11`. "Est." marks an estimate. Every other number was either measured in this review (scratch benches and harnesses kept outside the repo) or read from the code.

## Summary

- **Why she made a sofa and didn't sit on it.** There are four mechanisms. In order of how often they happen naturally:
  1. **One sit, then nothing.** Your own log (2026-10-01, 14:15) shows the handoff working. She made the sofa, sat on it for about 17 s, then chose to walk. Nothing ever brings her back: lounging answers no need, so it scores a flat 4.0, against bases of 14 for Walk and 16 for Pull.
  2. **Chat mid-crumple.** A chat line strands a two-thirds-drawn heap. With chat every 45–60 s this hit 8 of 133 crumples, and she came back 15–106 s later, or never.
  3. **Chat on the way to the finished piece.** With chat every 37 s, 3 of 42 finished pieces were never used.
  4. **A second build overwriting the first's purpose.** This happened 16 times in 16 when cued, but 0 times in about 160 natural builds.

  The common cause: her purposes live in loose fields in her head, interrupts clear different subsets of them, and plans are re-checked by struct equality.
- **The house complaint is a missing feature, not a bug.** Her real sofa and TV always share a pane. A sofa "on another floor" is one of two things:
  - a makeshift sofa, built wherever she tore text;
  - a shared pane whose floor reads as two, split by text or by a geometry bug.

  Nothing models where things *should* be. She can't move anything, and nothing of hers can be terrain.
- **Recommendation: keep the body, and move her memory into the world.**
  - **Things hold purposes.** A piece she made carries what she made it for. One slot holds the job she is heading for, found again by meaning, not by equality. Every way of losing a purpose shows a beat.
  - **Choosing.** The brain still chooses *what* she does. A table of named methods with pure guards chooses *how*, one step at a time, and holds no plan.
  - **The home.** It gets anchors and a few relational rules ("the sofa faces the TV"). She makes at most one visible repair per visit, prompted by a grievance she shows.
  - **Structures** are drawn with the UI's own line glyphs.
- **No search engine.** There is no MCTS, no annealing solver, no GOAP core, no rule engine and no new crate. The only search is one step of lookahead.
- **Phases.**
  - Phase 0 (≈ 650 lines with tests) fixes the sofa class.
  - Phase 1 is a pure refactor, proven by golden trajectory hashes.
  - Then come the mind as data, the home, organising, vignettes, structures, cameos and the factory.
- **Ten [open questions](#open-questions-for-the-user)** follow, each with a recommendation.

## What we have

### What is good, and stays

The body is the hard-won part. Most of its rules came from a bug, and its property tests found all four of the 2026-10-01 fixes.

| Part | Why it stays |
|---|---|
| Terrain read from the rendered cells every frame | Any layout is walkable; 313 µs at 200×60 |
| One definition of her image (`terrain::image`) for `restful` and drawing | It ended "decided on one picture, used on another" for seats (7fe56d0) |
| The text layer; mischief that undoes itself on a schedule (`pending`) | A purpose kept in the world already survives any interrupt |
| Door, `settle`, eviction, line art, image cache, dissolve | Art and render bugs, about 40% of past fixes, are settled |
| `base × fit × 0.4^repeats`, a weighted pick among the top four | Never argmax, never flat random; tuned on real visits |

`decide` costs at most 22 µs. `paint` averages 275–730 µs, at 1.3–1.9 redraws a second.

### What is wrong, and why

**The sofa.** Uninterrupted, the build-then-sit chain worked in 190 of 190 completed crumples. It is fragile, not wrong: it survives only if nothing happens between tearing and sitting. And once she has sat, owning the piece is worth nothing to her.

| Mechanism | How often | Where | Fixed in |
|---|---|---|---|
| Sat once, then nothing brings her back | Your log, lines 756–778: a 17 s sit, then Walk | `Use(Lounge)` serves no need (brain.rs:190-199): 8 × 0.5 = 4.0 | Q1, Q2 |
| Chat mid-crumple strands the heap 67% drawn | 8 of 133 crumples; back after 15–106 s, or never | `look` clears `task` (osaka.rs:2264-2297); `Crumpled` only fires at the use's end (osaka.rs:1193); the heap is capped at stage 3 (mod.rs:1484, scrap.rs:251) | phase 0 |
| Chat on the walk to the finished piece | 3 of 42 unused with chat every 37 s; 54 of 54 used without | `pursue` moves the job into `task` (osaka.rs:2209), `look` clears it (osaka.rs:2281), and `goal` was already spent (osaka.rs:1859) | phase 0 |
| Cross-floor job dropped by struct equality | 83 and 125 dropped pulls in two long runs with scrolling chat | `Chances::offers` (osaka.rs:140-147) | phase 0 |
| A second build overwrites the first's purpose | 16 of 16 cued; 0 in about 160 natural builds | `making` is overwritten (osaka.rs:1254) and emptied by any crumple (osaka.rs:1195) | phase 0 |

**The pattern.**
- **Intent in seven fields.** It is spread over `task`, `goal`, `making`, `errand`, `rest`, `watch_*` and `at_work`. About eleven sites each clear a different subset, mostly silently.
- **Re-checking by equality.** Plans are re-checked by exact equality against a fresh `Chances`, so a line that scrolled one row is a different job.
- **Hand-kept act lists.** About 19 `match self.act` sites classify every act by hand. `recheck`'s first list was wrong (654dd11).
- **The guest cleans up after the mind.**
  - It puts back abandoned tears by diffing `reeling()` from frame to frame (mod.rs:739-752).
  - It finds a heap by recomputing its seat on both sides (mod.rs:689-697, osaka.rs:1196).

These are history.md's class C (stranded intent) and the mind's half of class B (one rule in N places). The other fixes were art, wide glyphs, integration and read order. They live in the body, which no mind design touches.

**The house.**
- **Same pane.** Both real pieces are `RoomKind::Living` (room.rs:69), and `spot` offers only that room's pane (room.rs:504-510).
- **Why a sofa ends up "on another floor".** It is one of three cases:
  - a makeshift sofa built under whatever line she tore (scenes.rs:576-584);
  - a pane floor split by text in the four rows above the border (terrain.rs:280-302);
  - a TV at the far left: `platform_at(tv.left, …)` tests a column of the TV, not a standing spot, so it fails (mod.rs:1417-1422).
- **Fixed fractions.** Two pieces that collide send their room to the closet, even when they would fit side by side (room.rs:608-620).

**A commit race, found in review.**
- `HomeEvent`s pushed during `tick` are taken only at the next paint (mod.rs:678).
- `leave` (mod.rs:460) drops the Osaka together with her queued events.
- Three paths reach `leave` before that paint:
  - a visitor's key press (shell.rs:505, then mod.rs:451);
  - the client becoming busy (mod.rs:1073);
  - an errand ending (mod.rs:971).

A `Bought` on that tick is lost. It is rare, but it is exactly what decisions.md promises survives a key press.

## The decomposition

```
 L5 CHARACTER  whims · beats (a glance first, line pools) · splices & adverbs · scripts · calendar
 L4 HOME       anchors on strips · projection · rules · one repair per visit · structures
 L3 CHOICE     brain: what (top-four roll) · methods: how (first guard that binds) · explain log
 L2 MEMORY     purposes on things (made pieces, the carried piece) · one heading · owed beats
 ══════════ Offers, naming pieces ▲   ▼ an act + binding · LayerOps · HomeEvents ══════════
 L1 BODY       acts + ActProps · door · settle · evict · route · appearance · pending undos
 L0 FRAME      the rendered buffer + IdleView; terrain, text layer and her image read from it
```

| Your framing | Where | Mechanism |
|---|---|---|
| Goal selection, utility | L3 brain | Today's scores and top-four roll, plus named factors |
| Action sequencing, intent | L2, L3 methods | Ordered methods with pure guards; one step per decision; continuation read from world facts |
| Spatial design of her home | L4 | Relational rules; a one-step repair search |
| Locomotion, routing | L1 | Unchanged: BFS over links, and the door |
| Character, presentation | L5 | Quirks after a competent choice; scripted vignettes |

**Rules of flow.**
- The mind proposes and the frame decides. Ops and home events are validated against the live frame at the next paint, as today.
- The mind writes only its own state: the heading, owed beats and needs.
- `paint` alone writes things (made pieces, anchors, the ledger), and does so from events. It runs no search.

## The design

### Purposes live on things

```rust
pub enum PieceRef { Real(Furniture), Made(MadeId) }   // real pieces are one of each (ledger.rs:76)

struct Made {            // stays in Visit (mod.rs:200)
    id: MadeId,
    piece: Shown,        // its scrap stage is the crumple's progress
    torn: Vec<(u16, u16)>,
    purpose: Use,        // what she made it for: Lounge, Sleep or Watch
    used: bool,          // a use of that purpose has started
}
```

- **Making it.** The tear's last step pushes `LayerOp::Make { id, purpose, .. }`. The guest creates the `Made` when the op applies (mod.rs:732-734).
- **Crumpling it.** `HomeEvent::Crumpled(MadeId)` replaces matching by recomputed seat (mod.rs:689-697, osaka.rs:1196, mod.rs:1480).
- **Using it.** `HomeEvent::Used(PieceRef)` commits when a use *starts*.
- **What goes.** `Osaka::making` is deleted. Her head keeps only how often she has tried to get back to each piece.
- **Carried pieces** are the same kind of fact (`Visit.carrying`).

### One heading

Only one purpose has no thing to hold it: the job she is walking to on another floor. It gets one slot.

```rust
struct Heading { want: Want, hint: Hint }     // private to the mind

/// Found again by meaning. Text has no identity, so it is matched by content: the same
/// glyphs in the same columns, on the same row or up to K rows above (chat scrolls up).
enum Hint { Use(PieceRef, Use), Text { glyphs: Box<str>, cells: Range<u16>, row: u16, job: TextJob } }

impl Osaka {
    fn head_for(&mut self, want: Want, hint: Hint);
    fn drop_heading(&mut self, why: Why);   // the only remover; owes a glance once she has set off
}
```

- **Found again by content.** `Pull`, `Swap` and `Build` carry their glyphs. A resolved hint yields the *fresh* job, and she carries on with that.
- **Heading first.** `plan` resolves the heading before any guard runs, so no guard can quietly re-bind a committed target.
- **Reflexes keep their own fields.** The chat watch, the work shift and the errand's poke spot are reflexes, not purposes.
- **The Guest-level `Errand` stays on `Guest`**, because it outlives the visit (`errand_progress`, `gate`, `send`, `nudge_due`).
- **`head_for_errand` stops clearing the heading.**

### Choosing what, and how

```rust
pub struct DesireDef { want: Want, base: f64, serves: Option<(Need, f64)>, tier: Tier, factors: &'static [Factor] }
pub enum Factor { InChat(f64), Inertia(f64), Clock(Window, f64), Season(Window, f64), Rarity }
pub struct Method { want: Want, name: &'static str, guard: fn(&Ctx, &Whims) -> Option<Bind> }  // pure
pub struct Bind { act: Prim, spot: (i32, i32), facing: Facing, piece: Option<PieceRef>, job: Option<Job> }
```

- **`DesireDef` is `brain::Kind`**, with its tables unchanged.
- **A want is offered when one of its methods binds.** That reproduces today's gated offers (osaka.rs:1875-1889).
- **There are no fallback methods.**
  - Offering and planning see the same context, so a guard binds for both or for neither.
  - A job that goes stale on the way is resolved again on arrival. If it is gone, she glances and decides again.
  - Failure lines such as "No sofa..." are splices on ordinary acts.
- **`Go` is implicit.** For another floor, the body takes one hop along `route()` (or uses the door) and sets the heading. On her own floor she walks with the job in the walk's payload.
- **Whims.** One `u64` is drawn per decision from the mind's own stream, and `whims.chance(label, n, d)` hashes it with a label. Guards stay pure, every decision makes the same number of draws, and no two choices share a number.

**The decision** tries four buckets in order:
1. **Reflex.** Today's pre-empts, unchanged: the errand; no platform; off text, which sends her to `find_rest`; watching chat.
2. **Owed.** A beat she owes, played as a short act. It stays owed if interrupted.
3. **Continuation.**
   - First, the next step of whatever just completed: a landed hop, a finished tear or a finished crumple.
   - Otherwise, this visit's purposed thing that needs her: her unfinished heap, her unused made piece, or the piece in her pocket.
   - A thing that has used up 3 tries no longer counts (rule 2 below).
4. **Normal.** Score the offered wants, with ×3 inertia on the heading's want, and make a weighted pick among the top four. If she picks anything else, she drops the heading, with a glance.

Then `plan(want)` resolves the heading, or takes the first method whose guard binds.

**The explain log** records each decision: bucket, top four with factors, method, binding and heading. It answers Conway's three debugging questions (what now, why not something else, what before), shows on the stage, and replaces `choices: Vec<Kind>` in the statistics tests.

**Authoring.** Lounging:

```rust
M("lounge/finish-my-heap", |c, _| c.mine(Lounge).filter(|m| !m.done).and_then(|m| c.seat(m.piece, Crumple))),
M("lounge/sit-on-mine",    |c, _| c.mine(Lounge).filter(|m| !m.used).and_then(|m| c.seat(m.piece, Lounge))),
M("lounge/real-sofa",      |c, w| c.real_seat(Lounge).filter(|_| !w.chance("makeshift", 1, 20))),
M("lounge/made-sofa",      |c, _| c.made_seat(Lounge)),        // the whim landed, or she owns no sofa
M("lounge/make-one",       |c, w| c.pick_build(Lounge, w, |b| if b.also_watches { 5 } else { 1 })),
```

The `Use(Crumple)` want disappears, because a heap is crumpled as a step of its purpose. `also_watches` is already computed: the `then` closure (mod.rs:1538-1549) finds a Watch seat whenever the planned sofa would share the TV's floor.

| Addition | Today | Here |
|---|---|---|
| A behaviour on existing poses | 4–6 sites: `Kind`, base, serves, an offer, a `start` arm, often an `Act` in every whitelist | 1 `DesireDef` row, 1–3 methods |
| A static vignette (shopping channel, chopsticks) | New `Act`s and whitelist entries | 1 `Script`, 1 row, 1 method (or 1 `Splice`), plus art |
| Furniture with a new use | 8–10 files, 25–35 match arms | 1 `Spec` row, 1 `DesireDef` row, methods; the art is irreducible |
| A structure: another of a kind / a new kind | Impossible | 1 `Spec` row / plus a placement generator and usually a rule |
| A layout preference | Impossible | 1 `Rule` row with its grievance |

### The body interface

The body changes in six places, listed with the phase that makes each change:

1. **`ActProps`** (phase 1).
   - One exhaustive `match` gives every act its `stays`, `on_chat` and `passing`.
   - Nine queries, from `recheck` to `using`, derive from it.
   - A new act won't compile until it is classified.
2. **The job moves into the act's payload** (phase 1).
   - `task` and its 35 references go.
   - "A Pull with no Pull job" can't be written.
3. **One `interrupt(Cause)`** (phase 1) carries the reactions of six methods, from `look` to `refused`. Detection stays where it is.
4. **Offers name pieces by `PieceRef`, and text jobs carry their glyphs** (phase 0).
5. **Lift and SetDown are beats in the existing `Pose::Carry`** (phase 4). **`Act::Script` plays keyframes** (phase 5).
6. **`settle`'s Climb arm checks the pole** (phase 6). Today it checks only the landing and her box (osaka.rs:2308-2312).

### Vignettes

```rust
pub struct Script { id: ScriptId, keys: &'static [Key], branches: u8, commit_at: Option<u8>, on_interrupt: Resume }
pub struct Key {
    ms: (u32, u32),
    pose: Option<Pose>,          // None: the seat's own look (use_look), e.g. watching from the sofa
    face: Face,
    say: Option<Line>,           // Fixed | Pool (with cooldowns) | Dynamic (a remark, later)
    prop: Option<PropOverride>,  // the TV's channel (feeds Looks.tv), the fridge open, the lamp off
    op: Option<OpTemplate>,      // a layer change, scheduled through `pending`
    motion: Option<Motion>,      // GoTo(spot) | Follow(EffectId): phase 7
    only: Option<u8>,            // shown on this branch only
    hold: Occupancy,             // Stay | Pass; a lint requires it beyond 2 s
}
pub struct Splice { kind: SpliceKind, around: &'static [Want], when: fn(&Ctx) -> bool, chance: (u16, u16), script: ScriptId }
```

- **A script's branch** is drawn when it starts and stored in the act. That keeps `appearance(now)` pure.
- **The shopping channel** becomes one `Script`, played from a Watch seat.
  - Its `PropOverride` feeds `Looks.tv`, which today comes from `osaka.watching()` (mod.rs:851).
  - Its keys leave `pose` empty, so on the sofa she keeps the sofa's watching pose (osaka.rs:2765-2775).
  - The purchase commits at key 1, when the channel comes on.
- **The chopstick ritual** (#40) is a `Splice` around homework and snacks, gated by the clock.

### Worked example: she makes a sofa, chat interrupts, she sits on it

This is phases 0–2, with no real sofa and a TV in the Users pane.

| t | What happens | Mind | On screen |
|---|---|---|---|
| 0 s | Normal decision | `Use(Lounge)` (4.0) wins the roll; `make-one` binds a site whose sofa faces the TV | walks off |
| 6–9 s | The walk's job starts; the last reel pushes `Make { id: 7, purpose: Lounge }` | — | "Rrrip!", reels the text in |
| +1 frame | `Made #7` exists, a heap at stage 0 | Continuation: finish the heap | starts crumpling |
| 12 s | A chat line arrives | `interrupt(Chat)`: Look, then watch. #7 is still a heap (1 try) | looks up; the heap is 67% drawn |
| ≈ 27 s | The watch ends | Continuation: her unfinished heap | glances at it (sometimes "Ah, right!") and goes back |
| ≈ 32 s | `Crumpled(#7)` | Continuation: her unused piece | "There!" |
| ≈ 33 s | She sits; `Used(Made(#7))` commits | — | lounges for 15–30 s |

The old failures are covered too:
- **A second build** gets its own `Made`.
- **A look on the way** leaves #7 unused, so she comes back to it.
- **A scroll** is matched by glyphs.
- **A resize** removes the piece, and she says "...my sofa." toward where it stood.

## Interruption and intent

**The rules.**
1. **Finishing what she started is not a choice.** The next step of whatever just completed runs without a roll.
2. **This visit's leftovers resume once the reflexes are done.** A leftover is what a purpose left in the world: her unfinished heap, her unused made piece, or the piece in her pocket.
   - At most two made pieces stand at once, one of each kind (mod.rs:1523-1526), and she carries one piece at most, so this never becomes a to-do list.
   - Each interruption without progress is a try, and so is each decision where its next step can't bind (its seat isn't calm, its TV is hidden). After three tries she gives up, with a beat. A blocked leftover can't wait forever.
3. **A purpose that left nothing behind competes.** The heading enters the normal roll with ×3 inertia. If she picks something else, she lets it go with a glance.
4. **Earlier visits never take the continuation bucket.** A half-built ladder or a broken rule reaches her only through the normal roll, unprompted at most once a visit.
5. **Nothing ends silently.** Each loss has one site, and each site owes a beat.

| Loss site | What is lost | Beat owed |
|---|---|---|
| `tend_made` removes an unused purposed piece (text changed, resize, protected) | the piece | "...my sofa." toward where it stood |
| The guest puts back an abandoned tear (mod.rs:739-752) | the torn text | a glance at the line; rarely "...never mind." |
| `drop_heading` | the job she set off for | a glance toward it |
| A leftover reaches 3 tries | the follow-through | a glance; sometimes "Nah." |
| A carry is dropped (the target no longer fits, the set-down is refused, or 3 tries) | the move; the piece reappears at its old anchor | a glance back at it |

**Beats.**
- A beat is a glance by default: a look toward the thing, at least 600 ms long. Gaze is what makes a lapse read as hers (Carlisle).
- Speech comes from line pools, with per-line cooldowns and a per-visit budget. This matters, because about 35% of tears are abandoned under chat.
- An interrupted beat stays owed.

| Interrupt | Body (today's reaction) | Her purpose | You see |
|---|---|---|---|
| Chat while walking or hopping to a job | Look, then 15 s watching chat | The heading is kept, then competes with ×3 inertia | The look, and a glance if she lets it go |
| Chat mid-tear | Look; the guest puts the text back | Nothing was made | The look, and a glance at the line |
| Chat mid-crumple, or on the way to her piece | Look | Continuation after the watch (1 try) | A glance, sometimes "Ah, right!" |
| Chat mid-use | Look; she gets up | `used` was set at the start; credited by fraction (Q4) | The look |
| Chat mid-carry, behind her door, falling or climbing | Look once she lands or returns; the pocketed piece isn't drawn | Continues after the watch | The look |
| Local input, visitor | The goodbye | Events are drained into the ledger first; a carried piece's anchor never changed | The goodbye |
| Local input, resident | Chat mischief undone; Look | A heading into the chat stops resolving and is dropped | The look, a glance |
| Resize, focused-pane eviction | `settle`, `tend_made`; rain and door, as today | Made pieces are lost with a beat; real pieces are re-projected or hidden, never dropped | "...my sofa." |
| `recheck`, lost grip, refused op | A startle or a short Look; a tear is put back | One try | The look |
| Errand | Door and poke, as today | The heading and continuation wait; nothing is cleared | The poke |

## Home and structures

### The model

- **Strips.** A strip is a pane's bottom border, between its walls: `(Nook, Bottom)`.
  - It is the stable floor identity a home needs.
  - Live platforms can't be that, because they are renumbered every frame and split wherever text comes near.
- **Anchors.** Each real piece has a wall-relative anchor on its room's strip (a side and an offset), plus a facing.
  - Anchors live in a new top-level ledger field, and `props[].at` is still written.
  - An older record without anchors derives them from `at`.
- **`project()`** replaces `Home::{resolve, spot}`, `layout` and `place`.
  - It packs each strip in anchor order: a collision pushes both groups inward, never reorders.
  - Pieces that fit side by side are therefore shown side by side, and a resize and back restores the same placement.
- **One `Spec` row per kind** replaces the scattered `Furniture` matches (footprint, room, name, art, ASCII, uses, seats).
- **Made pieces** stay out of the home. They are per-visit `Made`s, standing where she tore the text.

### Rules, not an objective

```rust
pub enum Rule {
    Faces { seat: Furniture, screen: Furniture },             // same strip, facing it, gap 2..=14
    Near { a: Furniture, b: &'static [Furniture], gap: u16 }, // the lamp by the bed or the desk
    AgainstWall(Furniture),                                   // the fridge, the bookshelf: offset ≤ 1
    Connected { a: RoomKind, b: RoomKind },                   // a way between them besides her door (phase 6)
}
pub struct RuleRow { rule: Rule, grievance: ScriptId }        // what she shows while it is broken
```

- **Rules are judged on strips, not live platforms.** A user name splitting the floor between sofa and TV blocks her for a frame, but breaks no rule.
  - That is a rule change: she will watch from the sofa across a floor split by text. `seats_of` forbids that today (mod.rs:1414-1436).
  - `Faces` replaces that special case, and the `tv.left` bug goes with it.
- **A satisfied rule never moves anything.** With no target layout, no adoption and no hysteresis, nothing can thrash.
- **Deliveries still land wherever the box fits.** Noticing that the sofa faces the wall is hers to do.

### One repair a visit, prompted by a grievance

1. **She feels it.** Using a piece that a broken rule involves plays the rule's grievance as an interlude. She cranes toward the TV from the sofa ("Can't see the telly..."), or reaches for a lamp across the room. A rule she has never felt moves nothing.
2. **She chooses it.** `Want::Arrange` (base 6) is offered while a rule she has felt is broken, unprompted at most once a visit.
3. **She works it out.** The search covers each piece the rule names, at every anchor on its strip, in both facings.
   - Each move is applied to a copy of the home, and the strip is projected.
   - A move qualifies if it satisfies the rule, breaks no satisfied rule, and fits the strip's blank intervals (snapshotted at paint).
   - The cheapest qualifying move wins: fewest cells moved, then a whim.
   - That is at most about 400 candidates, ≤ 0.5 ms (est.), run in `advance()`.
4. **She does it, then uses it**, through the same method table:

```rust
M("arrange/use-it",   |c, _| c.just_set_down().and_then(|p| c.seat(p, c.felt_use()?))),  // then she sits
M("arrange/set-down", |c, _| c.carrying().filter(|k| c.standing_at(k.to)).map(Bind::set_down)),
M("arrange/carry",    |c, _| c.carrying().map(|k| Bind::go(k.to))),
M("arrange/lift",     |c, w| c.repair(w).map(|r| Bind::lift(r.piece))),                   // the search
```

**The carry is a literal pocket.**
- **Lift.** "Hup!": the piece goes into her pocket, and the projection stops drawing it.
- **The trip.** She walks, climbs or uses her door as usual, and nothing extra is drawn. `Pose::Carry` is a standing pose with no walking or climbing frames (sprite.rs:66-68).
- **SetDown.** "There!": `HomeEvent::SetDown { piece, anchor, facing }` commits at paint if the spot fits. Until then, the ledger keeps the old anchor.
- **Interruptions.**
  - A dropped carry puts the piece back at its old anchor, and she glances back at it.
  - Eviction takes the pocket through her door with her.
  - A goodbye leaves the piece where it was.
- **Cost.** No new art, and about 4 images.

**Trials, as behaviour.**
- When moves tie, she may try up to three spots: she sets it down, sits, and says "hmm…".
- She keeps a spot with probability exp(−Δ/T), where T is her restlessness.
- The episode counts as the visit's one carry. The spot she keeps satisfies the rule, so nothing moves it back.
- This is the only place annealing appears: as something you watch, not as a solver.

**Your literal case: a made sofa on another floor from the TV.**
- Made pieces have no anchor, so no repair touches them.
- Phase 0 weights build sites ×5 when the sofa would also face the TV.
- If no such site has text to tear, she still builds elsewhere.
- Whether she may carry a made piece, or keep it, is Q6.

### Structures from the UI's own lines

**What a structure is.** A stepladder is rows of `│─│`, two rails and a rung, between two floors. A shelf is a run of `─` above one. They are line glyphs in text cells, like pane borders. She builds with the UI's own material, which the original proposal called "borrowed border segments".

Only glyphs her image already redraws are used: solid light and heavy lines, corners and tees. Double and dashed lines aren't redrawn (graphics.rs:181-183), so `╫` or `┆` would each need a new `strokes` arm first.

**What that buys.**
- The terrain reads them with no new code: non-horizontal glyphs are poles, non-vertical ones are ledges.
- Her line-art image redraws them where it covers them, as it does borders.
- No SVG art, no terrain overlay, no `paint_layers` exception, and her image never grows to a ladder's height.

**One function draws them.** `read_ground(buf, view)` runs three steps in order:
1. `layer.validate`, and every other read of the real frame.
2. It writes each structure's glyphs where its cells are blank, unprotected, and clear of layer cells, made pieces and furniture. The cells come back as `Frozen`, so the goodbye and the focused-pane rain take them.
3. `Terrain::read`.

**Where it runs.** The arrival read (mod.rs:556), the visit read (754) and the errand read (1014) all go through it. The goodbye's read (585) is the one exception: by then the structures are frozen cells the dissolve owns.

**Rules for structures.**
- Structure cells join the solid set for text, like furniture covers.
- "Her own overlays are painted after every read" (decisions.md 2026-10-01) gains this one exception, because structures are terrain by design (Q8).
- App text or a protected area over a structure closets it for that frame. If she is on it, the pole check drops her, and she lands dazed.

**Building a ladder.**
- `Connected` breaks once she has gone between two rooms by door three times in a visit.
- The channel sells a stepladder, which arrives boxed.
- She assembles it in three stages of about 6 s, and each stage commits `Assembled`. A half-built ladder (`Partial{n}`) waits for the next visit, like Johnny Castaway's raft.
- It stands in a column blank between the floors, where both platforms reach. These are the existing Climb conditions (terrain.rs:511-532).
- It is done when the live terrain shows the link.

**Not now:**
- partitions (a folding screen drawn as `│` would be a pole);
- pieces moving between panes (Q7);
- making space ("no space means the closet");
- duplicates.

## Character

- **A competent core with visible quirks.** West's intelligent mistakes, and Evans' traits as adverbs.
- **Whims** are explicit, seeded guards: the one-in-twenty makeshift whim, dawdling, going the wrong way at a fork (#57).
- **Beats start as glances.** "Where was I?" after her door joins a pool. The simulator checks that no line repeats within 10 minutes.
- **Failure lines are splices:** "No sofa..." as she sits on the floor, "Too heavy..." when a repair finds no move.
- **Splices never change what they wrap.** Each is one `Splice` row:
  - chopsticks before homework (#40);
  - sata andagi after a snack (#41);
  - dreams over sleep (#74–76);
  - grievances during a use.
- **A wall clock, for the calendar only.**
  - Today `now` is monotonic milliseconds since process start (shell.rs:707-713), and `IdleView` carries no date.
  - `IdleView` gains a separate `LocalTime { date, minute_of_day, weekday }` from the shell. Chrono is already a dependency.
  - It feeds the calendar and routine, never durations or animation, which stay monotonic as the shell's comment requires.
  - Tests inject it, so replay is from (seed, inputs, local time). The ledger keeps calendar facts as dates.
- **Rarity and pity.**
  - Tier is a factor.
  - Pity comes from ledger counters of idle minutes.
  - At most one unseen rare appears per visit.
  - Calendar content is owed once, on the first visit of the day.
- **Deferred until she has been watched: forgetting, rediscovery and temperature.** When they return, only her own failures and time feed them, never chat or input. That way she isn't most erratic when someone is watching. Rediscovering something then takes minutes, not seconds.
- **Moving vignettes and cameos** (phase 7) need an effect sprite or a second figure in her image, under the same covering rules. This is the biggest body cost on the roadmap.
- **LLM remarks** stay later, as planned (#30): `Line::Dynamic`, never awaited, with a canned line on timeout.

## Determinism, cost, testing

**Determinism.**
- The body keeps today's stream. From phase 2 the mind has its own (`visit_seed ^ MIND_SALT`) and draws one number per decision.
- Repairs break ties by item order, then by whim.
- Budgets count candidates, never time.
- Nothing iterates a `HashMap`.

| Work | When | Cost |
|---|---|---|
| `decide` today | about 4 a minute | ≤ 22 µs (measured) |
| Scoring, guards, `plan` | per decision | 10–50 µs (est.) |
| Strip blank intervals | per paint, in the loop `Terrain::read` runs | ≈ 10 µs (est.) |
| Writing structures | per paint | ≤ about 40 cells |
| The repair search | at most once a visit, in `advance()` | ≤ 0.5 ms (est.) |
| The pocket carry | per move | about 4 images |

All of this fits the roughly 1 ms per decision the test suite can afford (`her_needs_shape_long_visits` makes about 360 decisions).

**New tests.** Each is written first and confirmed failing wherever it applies today.

| Test | Phase |
|---|---|
| `an_interrupted_crumple_keeps_its_purpose` | 0 |
| `every_made_piece_is_used_or_visibly_lost`: a use starts within 30 s of her next free decision, or the piece is lost or given up, or the visit ends (beats checked from phase 2) | 0, 2 |
| `commits_survive_the_visit_ending`: a commit tick, `activity()` before paint, then check the ledger | 0 |
| A TV at the far left offers watching from the sofa | 0 |
| Golden trajectory hashes | 1 |
| `a_heading_ends_by_arriving_or_with_a_beat`; `continuation_is_bounded`; a headless `decide` always returns an act | 2 |
| Projection uses only blank, unprotected cells; packing keeps order; a resize and back restores; ledger golden files both ways | 3 |
| A satisfied rule never moves anything; a repair breaks no satisfied rule; no piece returns within 3 visits to a spot it left, unless a resize forced it; at most 1 unprompted repair a visit; the image budget holds with a carry | 4 |
| Simulator: pity bounds; at most 1 unseen rare a visit; the calendar over a year; no beat line twice in 10 minutes; no want dominates a visit | 5 |
| A structure never removes a link; one closeted under her drops her; long visits with random structures keep every covering property | 6 |

**Phase 0 under the gate.** nextest kills a test at 60 s, so:
- **The crumple test** caps its tail at 120 s simulated. At HEAD the stranded sofas were finished by ordinary scoring at 220–311 s, so it should fail fast; that is still to be confirmed.
- **The bench's three failing runs** become pinned, deterministic tests. Each is a (seed, mode, chat every 37 s) case, run to 30 s past its recorded failure. Pinning is needed because, at about 4% failing visits, a 32-case random run catches the bug only 73% of the time.
- **The property** runs as `proptest_cases(8)` with 5-minute visits and chat every 20–40 s. It is timed at opt-level 2 before it gates anything.

**Golden trajectory hashes** are recorded before phase 1 changes anything, and phase 1 must leave all of them unchanged.
- Each tick hashes t, the act's variant, x, y, facing, `appearance(now)`, and the ops and events.
- The cases cover 4 seeds × {ASCII, kitty} × {the stage room with scripted chat, a resident with shakes and focus changes, a furnished home, an errand}.

`osaka_at_home_seed_7` stays a smoke test. It is a static ASCII room with no chat, furniture or errand, so it proves little on its own.

**Lints** are unit tests over the tables:
- every act or key longer than 2 s declares Stay or Pass (this would have caught 654dd11's first `recheck`);
- every mischief op schedules its undo in the same commit;
- fixed lines are at most 24 characters;
- every want and script can be cued from the stage.

The four body properties, the terrain snapshots, the dissolve properties and `perf.rs` stay as they are.

## Migration plan

Every phase:
- passes nextest and clippy;
- runs the 256-case property suite once;
- updates design.md and decisions.md for each rule it changes;
- adds a CHANGELOG entry for anything a user would see.

None is a protocol change, so `stable` stays put.

| # | Content | Size | You'll see |
|---|---|---|---|
| **0** Fix the sofa class | Tests first. Then:<ul><li>`MadeId`, `PieceRef`, `Made { purpose, used }`, `Crumpled(MadeId)` and `Used`; `making` deleted</li><li>Continuation after the reflexes: finish her heap, or use her unused piece, until a use starts or 3 tries. This replaces osaka.rs:1204</li><li>The goal pre-empt (osaka.rs:1859) stays, but resolves by meaning and continues with the fresh job</li><li>Events drained in `leave()` and at every switch to Absent</li><li>The TV's beside-spot; `layout` re-lays in order; ×5 for build sites facing the TV</li></ul>`seed_7` re-pinned. | ≈ +300 test, ≈ +350/−150 code | She sits on what she made, even after chat interrupts her. Her sofa is built where she can watch TV. A TV at the far left works. Nothing bought is lost |
| **1** Pure refactor | `ActProps`; the job in the act's payload (`task` deleted); `interrupt(Cause)`. Golden hashes unchanged | ≈ 650 touched, net negative | Nothing |
| — | **Checkpoint:** re-run the 37 s-chat bench and a 256-case run, and record the result in plan.md | — | — |
| **2** The mind as data | `Want`/`DesireDef` (tables unchanged); the method table; the heading (replaces `goal`); beats at the loss sites; credit by fraction (Q4); the mind stream and whims; the explain log; lints | ≈ 1,100 touched, ≈ +200 net | Glances and "...my sofa."; no dropped cross-floor jobs; the stage shows why |
| **3** The home model | Strips; anchors (a new ledger field); the packing `project()`; the `Spec` table (a refactor first, against golden hashes re-recorded after phase 2); `Faces` on strips | ≈ 800 | Pieces stop vanishing on collisions. She watches from the sofa whenever it faces the TV |
| **4** Organising | Rules and grievances; `Arrange`; the repair; the pocket carry; trials | ≈ 600 | She notices the sofa faces the wall, moves it, and sits down to watch |
| **5** Vignettes and the clock | `Script`/`Key`/`PropOverride`; line pools; `Splice`/`Adverb`; the shopping channel converted; chopsticks; sata andagi; `LocalTime`; calendar, tiers, rarity and pity; the simulator | ≈ 900 + art | Vignettes, calendar days, rarities |
| **6** Structures | `read_ground`; stepladder and shelf; `Connected`; staged `Assembled`; the pole check; the wishlist feeds the channel | ≈ 500 | She buys a stepladder, builds it over two visits, and climbs it |
| **7** Effects and cameos | Effect sprites and `Key.motion`; a second figure in her image; cast scripts keyed by (date, master seed) | ≈ 800 + art | Moving vignettes; Chiyo, Tomo, Kagura… |
| **8** The text factory | Materials as persisted things; a bounded planner behind one method, if crafting needs one | Sized then | The factory |

**Phase 2 is justified by authoring, not by bugs.**
- It makes every later behaviour a row rather than a set of match arms, and phases 4–7 are written against it.
- "Tables unchanged" refers to the base and serves numbers. The statistics tests still shift once, because credit moves from the choice to the completed fraction.
- If the checkpoint shows the sofa class still leaking, phase 2 closes it first.

**New types by phase.** This is the honest budget; most of these are small enums.

| Phase | Adds | Removes |
|---|---|---|
| 0 | `MadeId`, `PieceRef`, fields on `Made` and text jobs, `HomeEvent::Used` | `making`, seat-recompute matching |
| 1 | `ActProps` (`Stays`, `OnChat`), `Then`, `Cause` | `task`, the hand-kept act lists |
| 2 | `Want`, `DesireDef`, `Factor`, `Method`, `Bind`, `Whims`, `Heading`, `Hint`, `Beat`, `Line`, the explain log | `goal`, `places_for`, `Place`, `start`'s arms, `MAKESHIFT_ODDS` (now a guard), `choices` |
| 3 | `Strip`, `Anchor`, `Spec`, `Projection` | `Home::{resolve, spot}`, `layout`, `place`, the watch-from-sofa case |
| 4 | `Rule`, `Carry`, `SetDown`, `Turned` | — |
| 5 | `Script`, `Key`, `PropOverride`, `Splice`, `Adverb`, `Tier`, `LocalTime` | `Act::Use.advert` |
| 6 | `Structure`, `Partial`, `Assembled` | — |
| 7 | `Effect`, `Motion`, `Cast` | — |

## Alternatives considered

### A. Least-change: stop after phase 1, hand-write the rest

**What.** Phases 0 and 1, then hand-written recipes (`next(purpose) -> Next { Do, Done, Gone, Blocked }`) instead of a method table. Vignettes go in a plain table, and an in-pane layout scorer (22 states per piece) comes with a carry.

**Cost.** About 1,300–2,000 touched lines, and every mind-side bug class is still removed.

**Choose it if** the catalogue stays mostly as it is and the house wishes stay "someday", or if having the fewest concepts matters most.

**You give up:**
- new behaviours still touch 4–6 sites;
- there is no explain log and no lints;
- around ten interacting purposes (the factory), the recipes turn into a hand-coded planner.

Nothing from phases 0–1 is lost if you switch later.

### B. A generative planner (GOAP) as the backbone

**What.**
- A bounded A* over symbolic facts, with STRIPS-style actions.
- Vignettes become macro-actions.
- Ladders emerge from a "linked" precondition once big furniture can't pass her door.

**Cost.**
- About +5,000 lines.
- 0.3–0.6 ms per decision, 2 ms at worst.
- Every action needs a designed fact interface, and a wrong effect silently makes plans that can't run.
- Costs turn whims into trade-offs.

**Choose it if** emergent problem-solving is the heart of the feature ("she worked out she needed a ladder"), or when the factory's crafting chains arrive. One method's guard can call a bounded planner later (phase 8) without touching the rest.

### C. The full home model under the same mind

**What.** [The fuller design](2026-10-02-houseguest-mind/alternative-full-home-model.md) this proposal pared down:
- every piece and structure is a `Thing` with a stable id and a wall-relative anchor;
- rooms and their roles are derived from contents (RimWorld/ONI-style rule tables), so one pane can hold two rooms behind a partition;
- a scalar layout objective (pairwise, clearance, usage-weighted travel, attachment) solved exactly by branch and bound, with the top answers verified through the real projection;
- persisted projects that close only with evidence or a beat;
- structures as SVG art projected into the terrain through one `Ground::floor` definition;
- `Goal::Clear`, so her tidying makes space for furniture.

**Cost.** About 4,000 lines over six phases, more tuning surface (weights, adoption margins), and an objective whose moves a viewer can't always read.

**Choose it if** the answer to Q7 is "derive rooms" and partitions, moves between panes and making space are wanted soon rather than someday. Phases 0–2 here are a prefix of it either way; phase 3's anchors and `Spec` table are shared.

### Why not …

- **MCTS.**
  - Nothing needs it: there is no adversary, the world is deterministic between interrupts, and the payoffs are comic, not numeric.
  - At an estimated 4–16 ms per decision, it would add tens of seconds to the property tests.
  - Random playouts add variance exactly where legibility matters.
  - Its forward model would be a second definition of every effect, a bug class this module has already paid for twice.
  - The repair already takes the one step of lookahead that Versu shows is enough.
- **Simulated annealing as the layout solver.**
  - A repair weighs about 400 candidates, and enumeration is exact and deterministic.
  - The layout benchmark found SA no faster at this size, and noisy. Noise reads as her moving things back and forth.
  - Its role is the visible trials, with her restlessness as T.
  - It becomes the right solver only if free-form 2-D building in the factory breaks the strips, as in Infinigen.
- **GOAP as the core.** About 70 of the roughly 90 catalogue entries gain nothing from search. Planning against a hypothetical terrain moves the read-order bug class rather than removing it.
- **Behaviour trees.** The 2026-09-28 rejection was right, for a narrower reason than it gave: intent living in a running node, which every interrupt must unwind. Here the only program counters are in motor acts and keyframes, and they hold no intent.
- **A rule DSL or engine** (Praxis, Ceptre, a Datalog crate, a 1,200-line engine of our own).
  - It multiplies concepts for a catalogue that is mostly vignettes.
  - Bugs move from "the wrong field was cleared" to "the wrong rule won".
  - A VM fights the seeded tests.
  - Its ideas are taken anyway: lints, splices and the explain log.

## Open questions for the user

1. **"My new sofa" versus nineteen in twenty.** Continuation makes her use what she made, once. Should an unused made piece keep beating a real one after that? That would amend decisions.md 2026-10-01. **Recommended: no.** The first use answers "I'd want to use it", and after that your rule stands.
2. **Should owning furniture answer a need, so she lives in her home rather than just owning it?**
   - Lounging, napping and watching answer none today, so each scores a flat `base × 0.5`.
   - A need-serving act scores `base × (0.1 + need²)`. That is *lower* until the need passes about 0.63.
   - So "lounging eases sleepiness" means less lounging early (0.8 instead of 4.0) and more late (8.8 when sleepy).
   - **Recommended:** no change yet. Measure each piece's use in phase 5's simulator first.
3. **How certain is her follow-through?** **Recommended:** [the rules above](#interruption-and-intent):
   - certain for the step just completed;
   - certain for this visit's made or carried piece, up to 3 tries;
   - ×3 inertia for the heading, with a glance when she lets it go.

   A ~50% resume roll is more scatterbrained, but harder to test, and easier to mistake for a bug.
4. **Credit needs by completed fraction.** An interrupted sleep credits only what she slept, so she goes back to bed sooner. This replaces "answered when chosen" (decisions.md 2026-09-28). Zero credit would bring back the doze oscillation. **Recommended: yes,** in phase 2.
5. **Organise by grievances she shows, not by a design objective.** That means relational rules and one repair a visit, made only after she has felt the problem. It includes watching from the sofa across a floor split by text. **Recommended: yes.**
6. **Made pieces: movable, or kept?**
   - (a) As now: per visit, where she tore the text, with phase 0 steering the build toward the TV.
   - (b) She may pocket one. The holes stay where they were, as evidence.
   - (c) A used piece is kept across visits. This needs the factory's materials, since torn text is mended at the goodbye.

   **Recommended: (a)** for now.
7. **Rooms.** Keep "panes are rooms" (your 2026-09-28 call), or derive rooms from their contents, RimWorld-style? Deriving them allows moves between panes and partitions. It also means older builds put moved pieces back in their default pane. **Recommended: keep panes** until a rule can't be met inside one.
8. **The terrain rule.** Should structures be line glyphs, drawn by the one terrain-reading function before the read? **Recommended: yes,** in phase 6.
9. **Furniture through her door.** **Recommended: the pocket.** It is cheap and absurd. The alternative is that big pieces don't fit through the door. Ladders then become a payoff you watch her earn, but some homes can't be organised ("Too much bother."), and climbing while carrying needs new art.
10. **The shape of a visit.** By default she does at most one unprompted home act a visit (a repair or a build stage), vignettes come by tier, and tidying stays as today. Should home-making take more of a visit, or less?

**Decided unless you object:**
- Record the narrower rejection of behaviour trees and GOAP in decisions.md, with phase 2.
- Keep ledger version 1 and add new top-level fields.
  - Version 2 would make an older build refuse the record and meet Osaka afresh every session (ledger.rs:56).
  - An older build keeps every piece, placed from `at`, and ignores structures and counters.
  - If it saves, those are lost, and the newer build rebuilds anchors from `at`.
- Add `LocalTime` to `IdleView`, for the calendar only.
- Add no new crate.
- Leave making space and LLM remarks deferred, as already decided.

## Sources

**Game AI:**
- Humphreys, [Exploring HTN Planners through Example](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter12_Exploring_HTN_Planners_through_Example.pdf): ordered methods, and `WsIsTired` ("ask yourself if you are properly representing the world").
- Straatman et al., [Killzone 3 bots](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter29_Hierarchical_AI_for_Multiplayer_Bots_in_Killzone_3.pdf): replanning with "continue" branches.
- Zubek, [Needs-Based AI](https://robert.zubek.net/publications/Needs-based-AI-draft.pdf): lazy chaining, progress kept in world objects, reward on completion.
- Forbus & Wright, [Programming Objects in The Sims](https://qrg.northwestern.edu/papers/Files/Programming_Objects_in_The_Sims.pdf), and Graham, [Utility Theory](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter09_An_Introduction_to_Utility_Theory.pdf): advertising objects, and inertia.
- Conway, [GOAP in Tomb Raider](https://media.gdcvault.com/gdc2015/presentations/Conway_Chris_Goal-Oriented_Action_Planning.pdf): the three debugging questions.
- Orkin, [Three States and a Plan](https://www.gamedevs.org/uploads/three-states-plan-ai-of-fear.pdf): GOAP, and dialogue that explains inaction.
- Evans, [Sims 3 personalities](https://media.gdcvault.com/gdc10/slides/Evans_Richard_ModelingIndividualPersonalitiesInTheSims3.pdf): traits as adverbs; choosing among the top few.
- Evans & Short, [Versu](https://www.cs.uky.edu/~sgware/reading/papers/evans2014versu.pdf): apply, score, undo.
- Carlisle, [Psychologically Plausible Methods](https://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter38_Psychologically_Plausible_Methods_for_Character_Behavior_Design.pdf): gaze decides whether an act reads as intended.
- West, [Intelligent Mistakes](https://www.gamedeveloper.com/programming/intelligent-mistakes-how-to-incorporate-stupidity-into-your-ai-code): a competent core with deliberate mistakes.
- Roelofs, [MCTS pitfalls](http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter28_Pitfalls_and_Solutions_When_Using_Monte_Carlo_Tree_Search_for_Strategy_and_Tactical_Games.pdf): why not MCTS.

**Layout:**
- Yang et al., [Holodeck](https://openaccess.thecvf.com/content/CVPR2024/papers/Yang_Holodeck_Language_Guided_Generation_of_3D_Embodied_AI_Environments_CVPR_2024_paper.pdf): relational constraints, objects placed one at a time, unsatisfied constraints fed back. This is the rule repair.
- Fox, Gerevini, Long & Serina, [Plan Stability](https://strathprints.strath.ac.uk/2776/1/strathprints002776.pdf): change as little as possible.
- Yu et al., [Make it Home](https://web.cs.ucla.edu/~dt/papers/siggraph11/siggraph11.pdf), and Merrell et al., [Interactive Furniture Layout](https://paulmerrell.org/furnitureLayout2.pdf): pairwise and wall terms, with annealing as *their* solver.
- [Infinigen Indoors](https://arxiv.org/abs/2406.11824): where annealing becomes the right tool.
- [RimWorld rooms](https://rimworldwiki.com/wiki/Rooms): roles derived from contents (Q7).

**Precedent:**
- [Johnny Castaway](https://en.wikipedia.org/wiki/Johnny_Castaway): progress kept in the world across days.
