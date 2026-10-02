# The Home: a persistent, projected model of Osaka's living space

Scope: the home (layer 3) and the world model under it. Paths are relative to `dessplay/src/ui/houseguest/`. Measurements come from the understand reports; anything else is marked as an estimate.

## 1. Thesis

Her home should be **one persisted model of things with stable ids and wall-relative anchors**.

- Pieces, the floors and poles she adds, and room boundaries are all a **projection** of that model onto the current frame, through one `ground` definition and one `project()` function.
- Rooms and their roles are **derived from contents** by a rule table.
- "Where things should be" is a separate, deterministic optimisation. Branch and bound searches a cheap floor-plan abstraction, and the top answers are verified through the real projection. It runs only when the layout changes.
- The difference between home and target becomes **projects**: persisted world facts that any mind can read, step through, simulate and score. A project closes only with evidence that it was done, or with a visible beat that abandons it.

"Built a sofa, never sat on it", "TV and sofa on different floors", "the room never moves back" and "a ladder would need special code" all come from one gap: nothing in the world records why a thing exists or where it should be.

## 2. Architecture

### 2.1 Layers

```
 L0  Frame + IdleView          every paint   truth (unchanged)
 L1  HomeModel                 persisted     things{id, kind, anchor, condition}, projects, habits, adopted target
 L2  Projection                every paint   Placed things + Ground overlay -> Terrain::read(.., &ground)
                                             + CellClass snapshot (blank/stroke bitgrids) kept with the terrain
 L3  Derived facts             home change   rooms (cut by boundaries), roles (rule table), relations, seats
 L4  Layout solver             signature     FloorPlan -> B&B -> top-3 -> verify via L2 -> adopt
                               change, in advance()
 L5  Projects                  world facts   diff(target, home) -> ordered goals; purposes of made things
 --  HomeOracle                              the mind interface (2.8)
```

**Data flows down only.** `paint` hashes a layout signature and sets `dirty`; `advance()` solves. The mind never mutates L1: it emits `HomeEvent`s, validated at the next paint like `LayerOp`s (propose against snapshot N, validate at N+1).

**`advance()` has no `Buffer`,** and in ASCII mode `calm` can't tell blank cells from text. So `paint` also stores a `CellClass` snapshot: two bitgrids, `blank` and `stroke`, about 24k bits at 200×60. `fits` and `roomy` take `&dyn CellClass` instead of `&Buffer`: the live buffer at paint, the snapshot in `advance`. One definition, used at two moments.

### 2.2 L1: the persisted model

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ThingId(u32);            // stable for the ledger's life (or the visit's, for makeshift)
pub struct ProjectId(u32);

pub struct FloorRef { pub nook: Nook, pub level: Level }   // pane-relative floor
pub enum Level { Bottom, Shelf(ThingId) }                  // bottom border, or a shelf she built

/// Order-preserving: `offset` cells in from one wall's inner edge.
pub struct Anchor { pub floor: FloorRef, pub wall: Side, pub offset: u16 }

pub enum Where {
    Anchored(Anchor),
    Legacy { nook: Nook, at: u16 },   // v1 thousandths; converted on first projection (width unknown at load)
    Stowed,                           // in the closet by her choice (parked mid-project, off-season)
}

pub struct Thing {
    pub id: ThingId,
    pub kind: Kind,            // Piece(Furniture) | Structure(Structure) | Made(Furniture, Scrap)
    pub at: Where,
    pub facing: Facing,
    pub condition: Condition,  // Boxed | Partial { stage: u8 } | Ready
    pub shape: Shape,          // structures: width / rise
    pub life: Life,            // Persistent | Visit
}

pub struct HomeModel {
    pub next_id: u32,
    pub things: Vec<Thing>,
    pub projects: Projects,
    pub adopted: Option<Target>,   // hysteresis survives visits
    pub habits: Habits,            // decayed use counts and use->use transitions
    pub quiet: [u8; 3],            // per nook: EMA of "a piece here was closeted"
    pub wishlist: Vec<Kind>,       // structures the solver wants; the shop sells from it
}
```

**One `Spec` row per kind** replaces seven parallel matches: `Furniture::{footprint, room, pitch, name, art_id, ascii}`, `Use::of`, and the seat arms of `Shown::seat`. Those matches are history.md's class B ("8–10 files, 25–35 arms per new piece").

```rust
pub struct Spec {
    pub size: (u16, u16),
    pub offers: &'static [Offer],   // Seat, Screen, Bed, Desk, Light, Cold, Books, CatSpot, Step, Boundary
    pub uses: &'static [UseSpec],   // UseSpec { what: Use, at: Inside { col } | Beside }
    pub ground: GroundEffect,
    pub build: Option<BuildSpec>,   // stages + per-stage time; progress lives in Condition
    pub art: &'static str, pub ascii: &'static [&'static str], pub name: &'static str, pub pitch: &'static str,
}
pub enum GroundEffect {
    None,
    Ledge,     // shelf: its top row becomes floor cells
    Poles,     // ladder/stairs: pole cells from its floor up to the first ground row above
    Boundary,  // folding screen: splits rooms; solid to furniture and text, NOT to her
}
pub fn spec(kind: Kind) -> &'static Spec   // the only exhaustive match over kinds
```

### 2.3 L2: projection and the single `ground` definition

`project(&HomeModel, cells: &dyn CellClass, nooks, solid) -> Projection` replaces `Home::resolve`, `layout` and `place`. It works in three steps:

1. **Strips:** each floor's span between its pane's walls, further cut by `Boundary` things.
2. **1D packing:** left-anchored things stand at `wall + offset`, right-anchored ones mirror that. On collision, order is kept and both groups are pushed inward. Whatever still overlaps is closeted this frame, lowest priority first. Packing can't reorder or overlap, so world.md's fraction collision (sofa `at=0` and TV `at=500` in a 24-wide pane) is gone.
3. **Blank cells:** each thing must pass `fits` or is closeted. Text never moves a thing.

```rust
pub struct Projection {
    pub placed: Vec<Placed>,             // today's Shown + `id`
    pub ground: Ground,                  // overlay from structures
    pub closeted: Vec<(ThingId, Why)>,   // PaneGone | TooSmall | Text | Protected | Packed
    pub violations: Vec<ThingId>,        // hard: pane gone / strip too small for its contents
}
impl Ground {
    /// THE definition of "she or a piece may stand here": a real line glyph or a shelf she built.
    pub fn floor(&self, cells: &dyn CellClass, x: i32, y: i32) -> bool;
}
```

Today four sites each demand a real stroke glyph. All four must call `Ground::floor` instead:

1. **`Terrain::read`** gains `&Ground` and ORs the overlay into its `ledges` and `poles` grids **before** `add_platforms` and `find_links`. Platforms, Climb and Clamber links, and `landing` then fall out of existing code. No new `Route` variant is needed.
2. **`room::fits`**: the floor row under a footprint.
3. **`room::roomy`**: the floor under a beside-spot.
4. **`graphics::paint_layers`**: the floor-row test (graphics.rs:400-421) currently returns `None` on a blank floor cell. It must accept one when a structure layer in the same image redraws it.

Structure covers join `Terrain::furnish` and `terrain::image` exactly as furniture covers do, so the one-image rule from 7fe56d0 holds for them too.

### 2.4 L3: rooms, roles and relations

**Rooms.** A room is a strip region bounded by pane walls and boundaries, including the shelves above it. `RoomId = (Nook, left boundary: Option<ThingId>)`, which stays stable as long as the boundaries do.

**Roles.** The rules follow RimWorld (highest score wins; earlier rule wins a tie) and ONI (required and forbidden contents, minimum size). Requirements are categories, as in Terraria.

```rust
pub struct RoleRule { role: Role, requires: &'static [Req], forbids: &'static [Offer],
                      min_width: u16, base: f32, per: &'static [(Offer, f32)] }
pub enum Req { Offer(Offer), Rel(RelId) }
const ROLES: &[RoleRule] = &[
  RoleRule { role: Living,  requires: &[Offer(Screen), Offer(Seat)], forbids: &[Bed], base: 100.0, .. },
  RoleRule { role: Bedroom, requires: &[Offer(Bed)], forbids: &[Screen, Cold], base: 90.0, .. },
  RoleRule { role: Study,   requires: &[Offer(Desk)], per: &[(Light, 5.0), (Books, 5.0)], base: 60.0, .. },
  RoleRule { role: Kitchen, requires: &[Offer(Cold)], forbids: &[Bed], base: 50.0, .. },
  RoleRule { role: Den,     requires: &[], base: 0.99, .. },   // mixed contents land here
];
```

**Relations** are the layout terms (2.5) read as predicates on `FloorRef` and anchors. `Faces { Seat→Screen, same floor, gap 2..=14 }` replaces the Watch-from-sofa special case (mod.rs:1414-1437) and its `platform_at(tv.left, …)` failure for a TV at `at=0` (world.md cause b).

**Seats** are `seat(thing, use)`, derived per frame from `Spec::uses` + `Placed`, then filtered by `restful` and `platform_at`. A seat is always named by `(ThingId, Use)` and never stored as coordinates.

### 2.5 L4: objective and solver

```rust
struct FloorPlan {
    strips: Vec<Strip>,         // { floor, room, x0, x1, free: [Intervals; 3] (2/3/4-row blank), quiet }
    route_ms: Vec<Vec<u32>>,    // strip x strip, Dijkstra over the last paint's platform graph
    candidates: Vec<Candidate>, // possible ladders / boundaries / shelves
}
```

**Travel costs.** `route_ms` uses osaka.rs's own timings: `WALK_MS` 333/cell, `CLIMB_MS` 500/row, and the door (~5,500 ms) as the fallback edge between any two strips. Each strip maps to a platform through `platform_at` at its middle free column. This is the cost model the hop-count `route()` lacks.

**Cost terms** (`TERMS: &[Term]`; the weights are starting points):

| term | side-view meaning | weight |
|---|---|---|
| Fit | footprint in a free interval; use spots fit (approximate `roomy`) | hard |
| Faces(Seat→Screen) | same `FloorRef`, gap 2..=14, facing it, nothing taller than 2 rows between | 5 |
| Near(Light→Bed\|Desk), Near(CatSpot→Seat) | same strip, gap ≤ 4 | 1–2 |
| AgainstWall(Cold\|Books) | offset ≤ 1 | 0.5 |
| Clearance | free cells beside each Beside-use | 1 |
| Coherence | Σ over rooms of (role score − Den) | 2 |
| Travel | Σ over pairs of habits(a,b) × route_ms(a,b) / 1000 (QAP term) | 1 |
| Slack | spare columns × the strip's `quiet` | 0.5 |
| Attachment | cells moved from the current anchor, +50 per floor change | 0.2 |
| Effort | carry ms + build stages × stage ms | 0.1 |

**Solver choice.**

| | exact B&B + per-strip enumeration | SA over placements | "SA as visible behaviour" |
|---|---|---|---|
| answer | global optimum + ranked top-k, every time | seed-dependent local optima | a presentation of choice, not a solver |
| cost (layout.md) | 6 µs–1.8 ms at 8–10 items × 4–8 rooms with a *weak* bound | 0.6–0.8 ms / 20k steps; worse at 2k | minutes of her time |
| stability | Attachment/Effort make the optimum the stable answer | noise → thrash | bounded tries |
| test oracle | equals brute force on small cases | statistical only | trace properties |

**Recommendation: exact B&B for the target, and Metropolis only as visible behaviour (§6).**

How the B&B runs:
- **Ordering:** items by affinity degree, each trying its current strip first, for an early incumbent.
- **Domains:** pre-filtered by `Fit`, typically 1–4 strips.
- **Bound:** the sum of unary minima, admissible since pairwise terms are ≥ 0.
- **Within a strip:** exact orderings (k!·2^k ≤ 384 for k ≤ 4; insertion beyond that), memoised per (strip, item set).
- **Budget:** `HomeBudget { nodes }`, 20,000 by default and 2,000 in tests. Anytime; ties break by `ThingId`.

**Structures** are added greedily after the item solve. Each candidate is added and the solve rerun, and it is kept only if the cost drops by more than `δ_build`. A kept structure she doesn't own goes on `wishlist`. Structures appear only when they pay.

**Verification.** The top 3 are run through the real `project()` against the `CellClass` snapshot, then `terrain.clone().furnish(covers)`. Every `UseSpec` must yield a restful seat on a platform, or the answer isn't adopted. This kills 7fe56d0's class for whole layouts (a plan judged on one terrain picture, executed on another; sofa.md 2d). It also kills world.md's "`roomy` checks neither `restful` nor `platform_at`".

**Adoption.** There are two cases:
- **Soft change.** The new target must beat `adopted.cost` by `δ`, scaled by her restlessness. The signature must also have been stable for 10 s.
- **Hard violation.** The new target is adopted at once, and the affected things are **re-anchored instantly**. This keeps the user's "a room that loses its pane moves whole" (decisions.md 2026-09-28): she can't carry a sofa out of a pane that no longer exists.

Unlike today, a moved room **moves back**. When its pane returns, Attachment no longer holds the emergency spot, and the old layout wins by more than `δ`. That becomes ordinary projects.

### 2.6 L5: projects, the persisted form of intent

```rust
pub struct Project { id: ProjectId, goal: Goal, why: Why, life: Life, opened_visit: u64, tries: u8, status: Status }
pub enum Goal {
    Use   { thing: ThingId, what: Use },      // "I made / unboxed it for this"
    Place { thing: ThingId, at: Anchor, facing: Facing },
    Build { thing: ThingId },                 // Partial -> Ready
    Clear { floor: FloorRef, cols: (u16, u16), for_thing: ThingId },   // per visit (§4)
    Unbox { thing: ThingId },
}
pub enum Why { Target(u64), Made, Bought, Cameo(&'static str), Script(&'static str) }
pub enum Status { Open, Waiting(Wait), Done(Evidence), Abandoned(Beat) }

impl Projects {
    /// The ONLY way out of Open/Waiting. `Evidence` names the committed HomeEvent or
    /// completed Use; `Beat` is constructible only by the body once it has scheduled
    /// the visible beat (glance, "...eh.", shrug).
    pub fn close(&mut self, id: ProjectId, end: End) -> Option<Closed>;
}

pub enum HomeEvent {   // validated at the next paint
    Bought(Kind), Unpacked(ThingId), Crumpled(ThingId), Made { piece: Thing, purpose: Use },
    Lift(ThingId), SetDown { id: ThingId, at: Anchor, facing: Facing },
    Turn(ThingId), Stow(ThingId), Fetch(ThingId), Assemble(ThingId),
}
```

**From diff to projects.** `diff(target, home)` produces ordered `Place` and `Build` goals. Blockers go first. A cycle is broken by `Stow`: one thing goes through her door to the closet and is fetched later. A `Build` precedes the `Place` that needs it. The rest are ordered by gain ÷ Effort.

**Commit points.** `Lift` is visit-local: the ledger keeps the old anchor until `SetDown`, so a goodbye mid-carry leaves the piece in place with its project open. `SetDown`, `Turn` and `Stow` commit at once. `Assemble` commits one stage at its end, like `Unpacked`.

### 2.7 Keep, change, delete

- **Keep:** `Terrain` (`read` takes `&Ground`), `restful`, `image`, `find_links`; `tend_made`'s rules, as the made-thing projection test.
- **Change:** `fits` and `roomy` take `CellClass` and use `Ground::floor`; `Shown` becomes `Placed{id}`; `Seat` gains `thing`; `paint_layers` gets the floor exception.
- **Delete:**
  - `Home::{resolve, spot}`, `layout` and `place`, replaced by `project()` and the solver;
  - `Furniture::room`, `RoomKind`, `Home.rooms` and `Shown.nook`, after a v1 shim;
  - `Osaka::making`, which becomes `Goal::Use`;
  - the Watch-from-sofa special case.
- **The home gets its own stream,** `Rng(visit_seed ^ HOME_SALT)`.

### 2.8 The mind interface (vii)

```rust
pub struct StepView { pub step: Step, pub pre: &'static [Fact], pub post: &'static [Fact], pub ms: u32, pub status: StepStatus }
pub enum Step { Travel { to: FloorRef }, Lift { id: ThingId }, SetDown { id: ThingId, at: Anchor, facing: Facing },
                Turn { id: ThingId }, Assemble { id: ThingId }, Shove { project: ProjectId, row: u16 },
                Use { id: ThingId, what: Use }, Stow { id: ThingId }, Fetch { id: ThingId } }
pub enum Fact { Holding(Option<ThingId>), On(FloorRef), At(ThingId, Anchor), Ready(ThingId), Free(FloorRef, u16, u16), Used(ThingId, Use) }
pub enum StepStatus { Ready, Blocked(Blocker), Gone }   // Blocker: Text | Protected | Thing(ThingId) | Unreachable

pub trait HomeOracle {
    fn rooms(&self) -> &[RoomView];                 // id, role, members
    fn things(&self) -> &[ThingView];               // id, kind, floor, usable seats by (id, Use)
    fn projects(&self) -> &[ProjectView];           // goal, gain, effort, next StepViews
    fn check(&self, step: &Step) -> StepStatus;     // monitoring: predicate on ids, every frame
    fn simulate(&self, s: &HomeState, step: &Step) -> HomeState;   // symbolic, O(things)
    fn value(&self, s: &HomeState) -> f32;          // −objective + open-project credits
    fn seat(&self, id: ThingId, what: Use) -> Option<Seat>;        // this frame's geometry
}
```

It serves three kinds of mind. Utility+HTN treats projects as desires scored by `gain`, and a method takes the first `Ready` step. GOAP reads `pre`/`post` as STRIPS facts. Search (Versu's one step, or a capped MCTS) uses `simulate` and `value`.

Each re-reads `projects()` at every act boundary; the mind holds nothing needed for continuity.

## 3. Authoring: worked examples

**(a) Lounge, including making a sofa and sitting on it.** When tearing ends, the mind emits `Made { piece, purpose: Lounge }`. That one commit creates the Visit-life `Thing` T7 (`Partial{0}`) **and** `Project { goal: Use{T7, Lounge}, why: Made }`.

The build site is scored by the same `TERMS`, so a site on the TV's floor wins through `Faces`. That fixes world.md cause (a), building wherever she tore. The open project gives T7 a claim bonus until it closes `Done(Used)`. If `tend_made` destroys the heap, the project closes `Abandoned(Beat::Glance)` ("...aw"). Two concurrent makes can't overwrite each other, because each purpose is keyed by its own thing (sofa.md 2a). Makeshift things are never persisted, since their glyphs are holes in the text layer for this visit.

**(b) A vignette: the shopping channel.** It stays a keyframe script. The home supplies its preconditions as derived facts (`Role(Living)`, `Usable(Screen, Watch)`) and a seat preference (`Rel(SEAT_FACES_SCREEN)`). `Commit::Buy(Pick::Wishlist.or(Catalogue))` is committed at the start, so the channel sells what her layout wants: "Today: stepladder!"

**(c1) TV and sofa.** A 100×30 frame:
- **Users pane:** 30 wide, blank bottom rows, TV at `Left+3`.
- **Playlist below it:** 30 wide, sofa at `Right+2`.
- **Travel between them:** `route_ms(Users, Playlist)` is 5,500, the door.

The solver puts the sofa in Users at `Right+1`, facing left:
- **Gap:** 30−2−6−9−3−1 = 9, inside `2..=14`, so `Faces` holds and saves 5.
- **Travel:** saves 5.5 × habit(Lounge, Watch).
- **Attachment:** costs 0.2 × (≈2 + 50) ≈ 10.4. The gains exceed that once the habit weight is ≥ 1.

The project is `Place{sofa, Users/Bottom, Right, 1, Left}`, with steps `Travel(Playlist)`, `Lift`, `Travel(Users)` and `SetDown`. The cross-floor carry goes through the door, for the image budget (§5). Living now holds `SEAT_FACES_SCREEN`, so the sofa offers a Watch seat.

**(c2) A ladder.** Same frame, but with the fridge in Playlist and the sofa in Users.

The candidate is a stepladder at Playlist column c. It needs three things, and they are exactly the existing Climb conditions:
- c's column is blank up to the top border row;
- both platforms cover c, so `find_links`' overlap `upper.x0..x1 ∩ lower.x0..x1` is non-empty and the pole column lies within `lo−HALF−1..=hi+HALF+1`;
- `clear` holds along the climb.

The overlay's pole cells are the "unbroken pole" it asks for, so **the link appears without new terrain code**. Border rows are redrawable, hence `open`, so she climbs through them as she does today.

It saves about 5,500 − 8×500 ms per crossing, which clears `δ_build` once Snack↔Lounge habits are strong. The ladder is wishlisted, bought and delivered boxed. Then come `Unbox` and `Build`: 3 `Assemble` stages of ~6 s each. An interrupted stage leaves `Partial{n}`, drawn as half a ladder. `Done` requires `check` to see the link in the live terrain.

Stairs are the same `Spec` with `ground: Poles` and different art.

**(c3) A partition.** Users holds the bed together with the TV and sofa. `Bedroom` forbids `Screen`, so that room falls to `Den`.

The candidate is a `FoldingScreen` boundary in the ≥3-blank-column gap between the two groups. That gives Living + Bedroom, a Coherence gain of about 190, so it is built. It is **solid to furniture and text but not to her**, and she walks in front of it as she does furniture.

A solid wall would cut the floor, and `find_links` has no same-row link, so she'd need her door in space to cross her own wall. decisions.md rejected "pieces solid to her" for this reason.

**(d) A resize that invalidates the target.** The user narrows Users, the home from (c3), from 30 to 20 columns, 18 inside.

- **At paint.**
  - TV and sofa alone need 3+6+9+1 = 19 cells, so the sofa is closeted (`Packed`).
  - The bed's room behind the screen no longer fits: a hard violation.
  - `dirty` is set.
- **At the next `advance`.**
  - Verification fails, and a new target is adopted at once.
  - The bed and its screen re-anchor to Playlist immediately, so the room moves whole.
  - The sofa's open `Place` project is **re-targeted**: same id, new `at`. If the sofa is already there, the project closes `Done`.
- **Widening back.** After 10 s of stability, the old layout wins by more than `δ`, and projects carry the room back.
- **A 1-cell drag jitter** leaves the 2-cell-quantised intervals unchanged, so nothing is re-solved.

**(e) Places a new behaviour touches.**

| addition | touches |
|---|---|
| furniture (kotatsu) | `Kind` variant, 1 `Spec` row, art, calendar row; **1** exhaustive match |
| structure (shelf) | `Kind` variant, 1 `Spec` row (`ground: Ledge`), art; no change to terrain, rooms or solver |
| relation / role | 1 `TERMS` / `ROLES` row |
| vignette | 1 `Vignette` row + beats (+ a new pose if needed) |

For comparison, a new furniture use touches 8–10 files and 25–35 match arms today.

## 4. Interruption and intent

| interrupt | home model | project | visible beat |
|---|---|---|---|
| chat arrives mid-crumple | heap stays `Partial{s}` | `Use{T7}` stays Open; `Use(Crumple)` is `Ready` with a claim bonus | glance at chat; the claim bonus makes the heap her likeliest next choice |
| local input (`shaken`) | Lifted thing: `SetDown` in place if `fits`, else `Stow` via the door | Open | "Oops!", she drops it |
| focused-pane eviction | Lifted thing goes with her as `Stow`; things in the pane are closeted (protected) | `Waiting(Evicted)` | rain + door (as today) |
| resize | hard violations re-anchor; the target is recomputed | **re-targeted, never dropped** | only if abandoned |
| goodbye | the ledger anchor of a Lifted thing never changed | Persistent projects persist | the goodbye |
| `recheck` / `lost_*` | none | the step reports `Blocked`; the mind chooses again | today's Look |

**Why intent can't be lost silently:**
- Purposes live in `Projects`, keyed by `ThingId`.
- `check(step)` re-validates with predicates on ids, never with `PartialEq` on regenerated structs (`Chances::offers`, osaka.rs:140-147).
- `close()` demands either `Evidence` or a `Beat`, and only the body can construct a `Beat`.

Deliberate forgetting: `Abandoned(Beat::Shrug)` after 3 tries, or `Waiting(Forgot)`, reopened later with "Ah! The ladder!".

**Killed:**
- **Class C (stranded intent),** the family in sofa.md §4: `making` overwritten (2a), `goal` dropped on an equality mismatch (2c), `look()` clearing `task` after `goal` was spent.
- **Class A, plan-vs-execution version** (7fe56d0, sofa.md 2d). Verification goes through the paint path.
- **Class B** for ground (four checks become `Ground::floor`), same floor (`FloorRef` instead of `platform_at(tv.left)`) and room membership (`Shown.nook` and `room()` are deleted).
- **World gaps**: fraction collision, set down but never usable, a room that never moves back, and one-of-each.

**Not killed:**
- **Class A, read/paint-order version** (36a99d28, b76589ce a/b). Ground and CellClass must also be built before the terrain read.
- **Classes D, F and G.**
- **Class E (tuning).** This design *adds* tuning surface: TERMS, `δ` and `ε`.

### Making space

Three rules collide: furniture goes only on blank cells, holes are solid (mod.rs:1325-1331), and the text layer resets at goodbye. So **space she clears lasts for one visit only**: next visit the anchor is closeted until she clears it again. That is Sisyphean but in character. It is contained like this:

1. **`Goal::Clear` is a reservation.** It is created when the target's slot is blocked only by `Text` and the text fits a budget. Its steps are `Shove`: today's `LayerOp::Pull` with a purpose, sliding a row segment sideways out of the footprint's columns. Every Pull rule still applies, so protected rows stay untouched.
2. **Reserved holes are blank for the reserved thing.** Holes owned by an open `Clear` project are furnishable, but only by the thing it names. They must also be left out of `Visit::solid` (mod.rs:190-194) while that thing stands on them. If they weren't, they would sit in the solid set, `calm` would be false there, and the sofa's *inside* Lounge seat could never be restful. Every other hole stays solid.
3. **When the text comes back.** If `validate` drops a glyph because its source changed, the hole closes. The thing fails `fits` and is closeted, which is today's rule when text appears over a piece. Real text now lies under her image, so the existing per-frame `recheck` (654dd11) startles her. The project re-opens.
4. **One cap count.** The 60-glyph cap is counted two ways today (`count()` entries vs `cells()` source+target). Use entries everywhere, with a 24-glyph sub-budget for clearing. The FloorPlan marks slots that need more as `Fit = false`.
5. **Steer away from it.** `Slack × quiet` keeps targets out of strips that keep filling with text. Clearing is the fallback, not the norm.

## 5. Determinism, CPU, testing

**Determinism.** The home has its own rng stream. B&B depends only on the FloorPlan, ties break by id, budgets count nodes, and solves happen in `advance`. `osaka_at_home_seed_7` is re-blessed once, when `home.spot` stops drawing from the shared stream.

**Cost.** Measured figures: `Terrain::read` 313 µs at 200×60, `decide` at most 22 µs, `restful` 25–80 lookups. Everything else below is an estimate.

| work | when | estimate |
|---|---|---|
| `project()` + Ground + CellClass snapshot | every paint | ~1k lookups + 2 bitgrid fills: **~10 µs** (replaces `resolve`) |
| rooms, roles, relations | home change | **< 5 µs** |
| FloorPlan | signature change | ≤ 8 strips × 4 rows × 100 cols + Dijkstra over ≤ 40 platforms: **~20 µs** |
| B&B | signature change | ≤ 20k nodes × ~50 ns: **≤ 1 ms worst**, typically 50–300 µs |
| verify top-3 | after a solve | 3 × (terrain clone + furnish + ~12 restful checks): **~100 µs** |
| `projects()` / `check` | per decision | **< 5 µs** |

With solves debounced to one per 10 s, the amortised cost per decision is about 0, and the worst synchronous call about 1.5 ms. Tests use `HomeBudget { nodes: 2_000 }`.

**Image budget.** A carry within a floor costs ≤ ~25 images, and carries across floors use the door. With one carry per visit, the 200-image test holds.

**New properties and oracles:**
1. **Purposes are honoured.** Every `Made` or `Bought` project ends `Done(Used)` or `Abandoned(Beat)`, or is still Open at goodbye. Test it in `long_visits…`, and again with chat every 37 s.
2. **Projection is safe.** For random homes, rects and text: blank unprotected cells only, pairwise disjoint, on `Ground::floor`.
3. **Packing preserves order** and survives a resize and restore unchanged.
4. **B&B equals brute force** for ≤ 5 items × 3 strips.
5. **Adopted targets are livable.** Every `UseSpec` yields a restful seat on a platform.
6. **No thrash.** Over 50 random ±2-cell resizes, the target changes at most once, and a `Place` project re-opens at most twice per visit.
7. **Structures only add or cut links locally.** Adding `Poles` never removes a link, and a `Boundary` never changes the terrain.
8. **Conservation.** Every thing is in exactly one of Anchored, Lifted, Stowed, or Closeted-this-frame.
9. **Ledger.** v2 round-trips. A golden v1 record gives the same placed set on the same frame. Unknown kinds are skipped.

## 6. Character

- **Metropolis as visible behaviour.** When the verified top 3 are within `ε` of each other, she *tries* them. She carries the sofa to candidate 1, sits ("hmm…"), then candidate 2. She keeps the new spot with probability `exp(−Δ/T)`, where Δ is the true difference plus seeded mood noise. `T` is her restlessness: hot after deliveries, cooling as the home settles. She tries at most 3 spots, then says "Here."
- **Intelligent mistakes.** Occasionally she sets a piece down facing the wrong way ("the TV was looking at me"). The objective notices, and a later visit opens a `Turn` project with an "Oh!". It reads as her mistake because it gets corrected.
- **Pacing.** Unprompted, at most one carry or build stage per visit. A half-built ladder can wait (Johnny Castaway's raft).
- **Calendar.** The item set is dated: December to February the kotatsu replaces the sofa, so the rotation is just projects. Holiday props are Visit-life things.
- **Cameos, rarity and pity.** These gate vignettes; the home only reacts to them. Tomo kicks the sofa across the room, which opens a `Place` project. Chiyo ("everything snaps into perfect alignment") applies the adopted target **instantly**: she is the exact solver, Osaka the approximate one.
- **LLM remarks.** The oracle gets a read-only summary of roles, open projects and the last abandonment. It phrases remarks and never decides anything.

## 7. Migration (each phase shippable)

| phase | contents | size | tests |
|---|---|---|---|
| **H0** | `ThingId`, `Anchor`/`Where`, packing `project()`, `Placed{id}`, `CellClass`; ledger v2 reads v1 (`Legacy` converted on first projection; older builds reject v2 → one fresh session, record kept); rooms kept via a `room()` shim | ~600 | all body properties and terrain snapshots survive; room tests adapted; new 2, 3, 9 |
| **H1** | `spec()` table, offers, derived rooms and roles; delete `RoomKind`/`Home.rooms`/`Shown.nook`; the `Faces` relation replaces Watch-from-sofa (fixes TV at `at=0`) | ~400 | `a_furnished_home_gets_used_and_stays_cheap` and `a_real_piece_wins_nineteen_times_in_twenty` survive |
| **H2** | FloorPlan, TERMS, B&B, verification, hysteresis; deliveries go to the target; hard violations re-anchor; home rng split | ~900 | `osaka_at_home_seed_7` re-blessed; new 4, 5, 6 |
| **H3** | `Projects`, new `HomeEvent`s, Lift/SetDown/Stow (`Pose::Carry`, the door), `HomeOracle` behind today's `decide` (`Kind::Organise` when a step is `Ready`); made pieces become Things, `making` deleted | ~800 | sofa.md's `an_interrupted_crumple_keeps_its_purpose` written **first** (fails today); new 1, 8 |
| **H4** | Ground overlay in `read`/`fits`/`roomy`/`paint_layers`; stepladder, shelf, folding screen; build stages; wishlist | ~900 + art | new terrain snapshots; `long_visits_never_touch_what_is_protected` gains random structures; new 7 |
| **H5** | `Goal::Clear` + `solid` exemption, single glyph count, `quiet`, try-three-places, wrong-facing gag, Chiyo hook | ~400 | properties 1–2 extended to clearances |

## 8. Weaknesses and risks

- **Possibly over-built.** With ≤ 4 visible pieces in 1–2 feasible strips, a greedy placer plus verification gets most of the benefit. H0, H1 and `Faces` are worth doing regardless. Without H4, I'd choose the greedy placer over B&B.
- **Tuning.** TERMS weights, `δ` and `ε` are new class-E risk. Expect "she moved it back" reports. Property 6 defends against this but doesn't prove it away.
- **A volatile frame.** Targets keep being invalidated, so she may chase them. `quiet` needs measuring.
- **Line-art cost.** Structures enlarge her image, and climbs add images. Measure this.
- **Per-visit clearing** may read as forgetfulness. That is inherent in leaving no persistent marks.
- **A screen she walks through** may look odd. The art has to carry it.
- **Unknown project kinds** are dropped at load. That is the one silent drop, and it is fine because no visit is running then.
- **When I'd choose otherwise.**
  - For phase 5's free-form 2D building, strips break down; Infinigen-style SA over declarative constraints is better there.
  - If the home should be a fixed tableau, drop L4 and keep L1–L3. Ids, anchors, roles and projects alone still fix every intent-loss bug.
