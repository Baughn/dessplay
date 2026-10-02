# Research report: how the Houseguest could organise her home (furniture layout, room recognition and visible rearranging)

## 1. Interior layout optimisation in the literature

**Yu et al., "Make it Home" (SIGGRAPH 2011).** I confirmed this from the paper's own text ([PDF](https://web.cs.ucla.edu/~dt/papers/siggraph11/siggraph11.pdf), [project page](https://www.saikit.org/static/projects/furniture/index.html), [SIGGRAPH history](https://history.siggraph.org/learning/make-it-home-automatic-optimization-of-furniture-arrangement-by-yu-yeung-tang-chan-osher-et-al/)).
- **How it works:** relationships between furniture are learned from example rooms and turned into "priors". These go into a cost function, which is minimised by simulated annealing (SA) using a Metropolis-Hastings step.
- **Cost (Eq. 8):** C = wa·Ca + wv·Cv + wpath·Cpath + wprd·Cprd + wprθ·Cprθ + wpair_d·Cpair_d + wpair_θ·Cpair_θ
  - Ca, accessibility: penalises an object intruding into the space in front of another object that a person needs to use it.
  - Cv, visibility: penalises intrusion into an object's viewing frustum (the cone in front of it).
  - Cpath: keeps the paths between doors clear.
  - Cprd and Cprθ, the "prior": distance and angle to the nearest wall, compared with the examples.
  - Cpair_d and Cpair_θ, pairwise: desired distance and angle between related pairs, e.g. TV and sofa, bedside table and bed.
  - Weights used: wa=0.1, wv=0.01, wpath=0.1, distance weights 1.0–5.0, angle weights 10.0.
- **Moves:**
  - translate or rotate one object, with step size proportional to the temperature;
  - swap two objects of the same tier;
  - move the control points of the paths.
- **Cost of running it:** 18k–42k iterations took 22–376 s on a 3.33 GHz Xeon (Table 1). Their figure shows the layout at 1,000, 5,000, 15,000 and 25,000 iterations. That sequence is the "watch the optimiser think" picture.

**Merrell et al., "Interactive Furniture Layout Using Interior Design Guidelines" (SIGGRAPH 2011).** Sources: [PDF](https://paulmerrell.org/furnitureLayout2.pdf), [Berkeley page](http://graphics.berkeley.edu/papers/Merrell-IFL-2011-08/).
- **Cost:** c = Σ wi·mi over eleven terms:
  - clearance violation and circulation (cv, ci);
  - pairwise distance and angle (pd, pa);
  - conversation distance and angle (cd, ca);
  - visual balance (vb);
  - furniture alignment and wall alignment (fa, wa);
  - emphasis, meaning symmetry and facing a focal point (sy, ef).
  - Circulation counts connected components of the free space a person (an 18" disc) can move through. Seats in a conversation group should be 4–8 ft apart.
- **Sampler:** samples from a density exp(−βc) with Metropolis-Hastings. The three proposal moves are equally likely: move one item (Gaussian), rotate one item (Gaussian), or swap two items. All three are symmetric.
- **Parallel tempering:** several chains run at different temperatures and periodically swap states. It ran on CUDA on a GTX 480, 10k iterations per warp, and produced a set of suggestions in 0.73–1.03 s.

**Later work, briefly:**
- [SceneGraphNet](https://openaccess.thecvf.com/content_ICCV_2019/papers/Zhou_SceneGraphNet_Neural_Message_Passing_for_3D_Indoor_Scene_Augmentation_ICCV_2019_paper.pdf) (ICCV 2019) runs learned message passing over a graph of object relationships and predicts which object type fits at a given spot.
- [ATISS](https://arxiv.org/pdf/2110.03675) (NeurIPS 2021) is a transformer that treats a room as an unordered set of objects and can complete or partly rearrange a room.
- [Holodeck](https://openaccess.thecvf.com/content/CVPR2024/papers/Yang_Holodeck_Language_Guided_Generation_of_3D_Embodied_AI_Environments_CVPR_2024_paper.pdf) (CVPR 2024) has an LLM emit ten kinds of spatial constraint in five categories:
  - global: edge, middle;
  - distance: near, far;
  - position: in front of, side of, above, on top of;
  - alignment: centre-aligned;
  - rotation: face to.
  
  A depth-first-search solver then places objects one at a time. When placement fails, the unsatisfied constraints are fed back.
- [Infinigen Indoors](https://arxiv.org/abs/2406.11824) (CVPR 2024) is the closest to what we want. It has a declarative constraint language covering symmetry, relations, quantity, physics and accessibility. Its SA solver works in stages (floor plan, then large furniture, then small objects) and has moves that change object-to-object relations.
- The learned methods need datasets and offer us nothing. Holodeck's vocabulary and Infinigen's "declarative constraints plus an SA solver" are useful as design patterns.

**What carries over to a 2D side view (my mapping):**

| Source term | Side-view, discrete version | Keep? |
|---|---|---|
| Overlap and fit (hard) | Fits on blank floor cells with no overlap; reachable via poles and ladders | Keep, as a hard constraint |
| Accessibility (Yu Ca) / clearance (Merrell cv) | Free cells to the left or right of the use point or seat, on the same floor | Keep |
| Visibility (Yu Cv) | TV and sofa on the same floor, horizontal gap within [dmin, dmax], facing each other (left/right only), nothing tall between them | Keep, as the main pairwise term |
| Pairwise distance and angle | Preferred gap and left/right order, e.g. lamp beside bed or desk, cat bed near sofa, bookshelf near desk | Keep. Angle reduces to facing. |
| Prior / wall alignment | "Against the wall" means near a vertical pane border (fridge in a corner, bookshelf against the wall) | Keep, cheap |
| Path / circulation | Walking distance over the floor-and-ladder graph, weighted by how often she goes between two items. This is the Koopmans–Beckmann quadratic assignment problem (QAP), which is NP-hard in general ([Wikipedia](https://en.wikipedia.org/wiki/Quadratic_assignment_problem), [Burkard & Çela](https://www.opt.math.tugraz.at/~cela/papers/qap_bericht.pdf)). | Keep. This term puts the sofa on the TV's floor and the fridge near the living room. |
| Conversation | Only applies if there are guests or the cat | Drop for now |
| Visual balance | Spread across floors and panes; don't cram one pane | Optional, low weight |
| Furniture alignment | Everything already stands on a border | Drop |
| Emphasis / focal point | TV as the focal point of the living room; symmetry around it | Optional |

## 2. SA versus exhaustive search versus constraint solving at our size

**Arithmetic:**
- Placing items directly into slots naively is 100^10 = 10^20 combinations. That is hopeless.
- Assigning items to rooms naively is R^N: 10^10 for 10×10.
- Within one room, everything sits in a row along one floor, so placement is one-dimensional. With k ≤ 4 items in a room there are k! ≤ 24 orderings, and each needs one pass over the W cells of the floor to place them. That is roughly 24 × 4 × W, a few thousand operations per room.
- So the problem splits in two: choose which items go in which room with branch-and-bound (B&B), then order each room exactly, caching each (room, item set) result. That should be well under a millisecond. This is an estimate, not a measurement.

**Microbenchmark (I wrote and ran it).** It solves the item→room level only. The cost is a random preference for each item in each room, plus "how much the two items go together" × "how far apart their rooms are" for each pair, with room capacity limits. It used `rustc -O` on this dev machine (the benchmark source was not kept).

| items × rooms | exhaustive | B&B (optimum) | SA, 20k steps | SA, 2k steps |
|---|---|---|---|---|
| 8×4 | 1.0 ms | 6 µs | 0.6 ms, optimal | optimal |
| 8×6 | 33 ms | 14 µs | 0.7 ms, optimal | optimal |
| 10×6 | 1.34 s | 1.1 ms | 0.76 ms, optimal | worse (48.1 vs 42.9) |
| 10×8 | 28 s | 1.8 ms | 0.84 ms, optimal | optimal |
| 10×10 | skipped (10^10) | 20 ms | 0.83 ms, worse (47.6 vs 42.9) | worse |

What the benchmark does not show:
- The instances are synthetic and random.
- It does not cover the per-room slot level.
- The SA recomputes the whole cost every step, so its times are pessimistic.
- The B&B's lower bound only counts each item's own room preference, so its times are also pessimistic.
- With a better bound, or by moving whole groups of items (the living-room set, the sleep set) instead of single items, both drop a lot.

**Conclusion:**
- At this size SA buys nothing on speed.
- The real trade-off is determinism and stability against flexibility.
  - Exact search (B&B, or exhaustive on a split-up problem) always returns the same global optimum. It can also cheaply return a ranked top-k list.
  - SA accepts any cost term, but its answers are noisy, and noise means thrashing.
- Answer-set programming and constraint solving are the declarative alternative ([Smith & Mateas 2011, IEEE TCIAIG 3(3)](https://researchr.org/publication/SmithM11-3)). They are a better fit for hard rules than for weighted soft preferences, and they would add a dependency. I did not evaluate specific solvers.
- MCTS has been used for generating game content that must be playable (e.g. [Sokoban generation](https://motion.cs.umn.edu/pub/SokobanMCTS/DataDrivenSokobanMCTS.pdf)), and [Bhaumik et al.](https://arxiv.org/pdf/1903.11678) compare tree search with optimisation for map generation. MCTS earns its place when you must search over sequences of actions with uncertain outcomes. Choosing the target layout is not that.

## 3. How games recognise rooms and their roles

- **RimWorld** ([wiki](https://rimworldwiki.com/wiki/Rooms)) is the cleanest declarative model.
  - A room is any area fully enclosed by walls, doors and similar.
  - Every role scores the room from its contents, and the highest score wins. Ties go to the role listed earlier.
  - Example scores: Bedroom 100,000 if it has an ordinary bed (a medical or prisoner bed disqualifies it); Laboratory 60 per research bench; Workshop 27 per workbench; Dining Room 12 per table; the generic "Room" role always scores 0.99, so every room gets a role.
  - Impressiveness combines four stats, each normalised and soft-capped with a log: wealth/1500, beauty/3, space/125, cleanliness. The formula is I = 65·mean + 35·min, so the weakest stat supplies about half the result. A space-based cap stops tiny rooms from being luxurious.
  - The role tables are XML defs that mods can extend. This is exactly a declarative rule table.
- **Oxygen Not Included** ([wiki.gg Room Overlay](https://oxygennotincluded.wiki.gg/wiki/Room_Overlay)): each room type lists required buildings, forbidden buildings (for example "no industrial machinery") and a size range (12–64 tiles, 32–120 for halls).
  - One tension in the source. The wiki says a room matching several types becomes "Miscellaneous" with no bonus. It also lists upgrade chains (Barracks → Luxury Barracks → Private Bedroom; Mess Hall → Great Hall → Banquet Hall) where the stricter type evidently wins.
  - Either way, the pattern is required set + forbidden set + size range, with conflicts resolved either by an upgrade order or by falling back to a generic room.
- **Terraria** ([wiki](https://terraria.wiki.gg/wiki/House)): a house is found by flood-filling from a point to an enclosed frame.
  - It must have 60 to 750 tiles.
  - It needs one item from each of four categories: light, "table" (any flat surface), "chair" (any comfort item, beds and sofas included) and an entrance.
  - It needs a valid standing tile.
  - Requirements are categories, not specific items. This is a "requires one thing that does X" model.
- **Dwarf Fortress** ([wiki](https://dwarffortresswiki.org/index.php/DF2014:Room)): room quality is the summed value of floor, walls and furniture, mapped to tiers (Meager 1 … Royal 10,000).
- **The Sims:**
  - In The Sims 4, rooms get an environment score from seven décor categories ([Sims wiki](https://www.thesimswiki.com/wiki/Environment)).
  - Autonomy works through objects advertising actions. Each ad is scored by need, distance and relationship ("your own bed"). The game picks at random among the top several ([HN thread on Motive.c](https://news.ycombinator.com/item?id=14997725), [GMTK](https://gmtk.substack.com/p/the-genius-ai-behind-the-sims)).
- **Spelunky** ([summary](https://takenapeveryday.wordpress.com/category/spelunky/)): levels are a 4×4 grid of rooms, each 10×8 tiles. A random guaranteed route from top to bottom is laid first, then each room gets a hand-made template. This matters for us because it is the precedent for building a structure with floors, but ours would be done by her actions instead of a generator.

**For us (my synthesis):**
- A room is an enclosed region: a pane, or part of a pane cut off by a partition she builds, found by flood fill.
- Its role is the best-scoring role from its contents (RimWorld), with required, forbidden and size rules (ONI) and category-based requirements (Terraria, e.g. "a seat that faces a screen").
- Partitions, ladders and stairwells are just more placeable items that change the room-detection flood fill and the walking graph. That avoids special-casing each one.
- A skim of `/home/svein/dev/dessplay/dessplay/src/ui/houseguest/room.rs` shows `Furniture::room()` hardcodes each item's room kind (Sofa/Tv/CatBed → Living, etc.). That should be inverted: roles are derived from contents, and items only declare what they offer ("seat", "screen", "sleep", "storage", "light").

## 4. Turning a target layout into visible actions, and coping with a changing world

- **Moving furniture is a known planning task.** "Rearrangement" means bringing an environment to a specified goal state, and furniture is one of the examples given ([Batra et al. 2020](https://arxiv.org/pdf/2011.01975)).
- **Planner shape:** GOAP from F.E.A.R. is STRIPS-style actions with preconditions and effects, chained with A* ([Orkin, GDC 2006](https://www.gamedevs.org/uploads/three-states-plan-ai-of-fear.pdf)). Our actions would be things like `carry(item, from, to)`, `build_ladder(at)` and `build_partition(at)`. Preconditions would be "reachable" (a ladder exists) and "target spot is free".
- **Ordering the moves (my method, not sourced):**
  1. Compare the current layout with the target to find the items that need to move.
  2. Build a "blocked-by" graph: the item currently in another item's target spot has to move first.
  3. Sort it so blockers move first.
  4. Break any cycles by parking one item temporarily in a staging spot such as the closet or hallway.
  5. Put ladder or partition building before any move that needs it.
  6. Order the rest by gain divided by carrying distance, so the sofa-to-the-TV move comes first.
  - With N ≤ 10 this is trivial.
- **Stability:** repairing the current plan versus replanning from scratch, measured by how little the plan changes ([Fox, Gerevini, Long & Serina, ICAPS 2006](https://strathprints.strath.ac.uk/2776/1/strathprints002776.pdf)). For us that means choosing targets that stay close to the current layout.
- **Hysteresis (my mechanism, following Fox's principle):**
  - Add an "attachment" term: Yu's prior term, but with her *current* layout as the prior.
  - Add a carry-effort term: the sum over moved items of carrying distance times item size.
  - Only adopt a new target if it beats the current target by a margin δ, and still does so after a debounce period (e.g. the world stays stable for N seconds or visits). That stops pane resizes from causing thrashing.
  - If a resize makes an item invalid ("in the closet"), that is a hard violation and skips the margin.
  - Add a "slack" term: prefer spots with spare blank cells, and spots whose panes have historically stayed quiet. Placements that survive small resizes come out of this naturally.
- **Being responsive:** re-solving costs about a millisecond, so re-solve on every layout change event. Only the *adopted target* changes slowly.

## 5. Recommendation

**Architecture:** keep "what is a good home" separate from "what she does next".
1. **Declarative world model:**
   - Items list what they offer: seat, screen, sleep, light, cold storage, surface, and so on.
   - A room-role table scores rooms by contents, RimWorld-style, with required, forbidden and size rules from ONI.
   - Structural items (ladder, partition) are ordinary items that change the flood fill and the walking graph.
2. **Evaluating a layout:** cost = hard validity (fit, no overlap, reachable), plus soft terms:
   - **Role coherence:** each room has one clear role; mixed contents are penalised (the ONI "Miscellaneous" idea). Gives a bonus for completing a role's set (Terraria-style categories).
   - **Pairwise:** TV and sofa on the same floor, facing, within a gap range, nothing tall between; lamp beside bed or desk; cat bed near sofa.
   - **Use-point clearance:** free cells beside each seat or use spot.
   - **Against the wall:** fridge and bookshelf near a vertical border.
   - **Usage-weighted travel (the QAP term):** how often she goes between two items × walking distance over floors and ladders. This puts the sofa on the TV's floor.
   - **Robustness slack:** spare room and historically quiet panes.
   - **Attachment and carry effort:** the hysteresis terms above.
3. **Solver:** exact search with a split. Items go to rooms by B&B (or by moving functional groups as units); each room is ordered exactly along its floor and cached. It should return the **top-k** layouts in about a millisecond, deterministically. Use SA only if the term set later grows enough to break the split. Infinigen shows SA scales to that richer case.
4. **Planner:** small GOAP or HTN-style tasks driven by the difference between current and target layout, with the blocker ordering and staging from section 4. A move or build action should only exist as a precondition of a *use* action. For example, "sit and watch TV" needs "sofa on the TV's floor, facing it", so after carrying the sofa she immediately sits. That fixes "she made a sofa and never sat on it": building has no value of its own; its value is the use it enables. A novelty bonus on newly built items would help too, scored the way The Sims scores advertised actions.

**Making her organising feel characterful:**
- **Trying the sofa in three places.** Do not show raw SA jitter; random back-and-forth would read as dumb. Instead take the solver's top-k and do what The Sims does: choose among the top few. She physically tries candidate 1, sits, "evaluates", moves on to candidate 2, and keeps the best. Add a little noise from her mood each visit so she sometimes changes her mind. This is a Metropolis acceptance applied to the actual moves she makes, not to the solver's internal steps.
- **Temperature as restlessness.** High after moving in, or after a big layout change (re-heat), so she reorganises often. It cools as the home settles, so a mature home is stable. Temperature also scales the hysteresis margin δ.
- **One meaningful act per visit.** Show a single carry, build or try per idle visit. The process stays visible and slow, and an in-progress layout is always a valid, usable state.
- **Ambitious items** (walls, stairwells, moving the sofa to another floor) fall out of the same machinery. They are actions whose effects improve the travel and role terms. Nothing needs special-casing.

**Sources:**
- https://web.cs.ucla.edu/~dt/papers/siggraph11/siggraph11.pdf
- https://www.saikit.org/static/projects/furniture/index.html
- https://paulmerrell.org/furnitureLayout2.pdf
- http://graphics.berkeley.edu/papers/Merrell-IFL-2011-08/
- https://openaccess.thecvf.com/content_ICCV_2019/papers/Zhou_SceneGraphNet_Neural_Message_Passing_for_3D_Indoor_Scene_Augmentation_ICCV_2019_paper.pdf
- https://arxiv.org/pdf/2110.03675
- https://openaccess.thecvf.com/content/CVPR2024/papers/Yang_Holodeck_Language_Guided_Generation_of_3D_Embodied_AI_Environments_CVPR_2024_paper.pdf
- https://arxiv.org/abs/2406.11824
- https://en.wikipedia.org/wiki/Quadratic_assignment_problem
- https://www.opt.math.tugraz.at/~cela/papers/qap_bericht.pdf
- https://researchr.org/publication/SmithM11-3
- https://motion.cs.umn.edu/pub/SokobanMCTS/DataDrivenSokobanMCTS.pdf
- https://arxiv.org/pdf/1903.11678
- https://rimworldwiki.com/wiki/Rooms
- https://oxygennotincluded.wiki.gg/wiki/Room_Overlay
- https://terraria.wiki.gg/wiki/House
- https://dwarffortresswiki.org/index.php/DF2014:Room
- https://www.thesimswiki.com/wiki/Environment
- https://news.ycombinator.com/item?id=14997725
- https://gmtk.substack.com/p/the-genius-ai-behind-the-sims
- https://takenapeveryday.wordpress.com/category/spelunky/
- https://arxiv.org/pdf/2011.01975
- https://www.gamedevs.org/uploads/three-states-plan-ai-of-fear.pdf
- https://strathprints.strath.ac.uk/2776/1/strathprints002776.pdf