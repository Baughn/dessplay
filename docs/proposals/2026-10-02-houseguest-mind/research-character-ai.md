# Survey: architectures for the Houseguest's mind

**Evidence key.**
- **[P]**: I read the primary text myself (PDF converted to text). All quotations come from these sources.
- **[S]**: a fetch tool summarised the page, so treat it as paraphrase, not quotation.
- **[Q]**: search-result snippet only, not verified.

---

## 0. Headline findings

1. **The sofa bug is structural, and the literature names the fix.**
   - In `brain.rs`, `Kind::serves()` returns `None` for both `Use::Crumple` and `Use::Lounge`.
   - Needs are debited when an act is *chosen*, not when it finishes.
   - So building serves no goal, and the finished sofa carries no intent forward. After the build, "Lounge" scores at neutral fit (0.5 × base 8), against Walk (14) and Pull (16).
   - Three independent sources say to put the *reason* for the next step into world state:
     - Humphreys' HTN `WsIsTired` fix: "ask yourself if you are properly representing the world" [P].
     - Zubek's needs-based AI: half-done work lives in the world as objects (his example is a prepped-food object). With "lazy action chaining", the last step of one action queues the next [P].
     - Conway's Tomb Raider GOAP: a `DinnerTable` has a "situational requirement" of `Food`. The plan is built backwards from the goal, so the prerequisite *is* the intent [P].

2. **The proposal rejected behaviour trees, GOAP and HTN because they "hold mid-plan state that must be unwound".**
   - That is right when the plan lives in the running program (a program counter).
   - It is wrong when the plan lives in the world. A half-crumpled heap is already a safe interrupt point if the heap is a room object that advertises "finish me / sit on me".
   - Two production patterns keep continuity while still replanning at every decision:
     - Killzone 3 and Transformers re-run an HTN planner at a fixed rate, and use "continue" branches that keep the current plan's priority [P].
     - Zubek grants rewards only when an action completes [P].

3. **Versu's decision rule suits a tiny CPU budget.**
   - "they actually execute the results of the action… evaluate this future world state with respect to their desires. Then, we undo" [P].
   - This is one step of lookahead over the *real* simulation.
   - Full MCTS is unnecessary. Rome II only made MCTS work by splitting the problem up heavily [S].

4. **"Dumbness" works best as a deliberate perturbation of a competent core.** Don't make the planner weak.
   - Mick West: compute the best move, then make "intelligent mistakes" [S].
   - Evans (Sims 3): traits are *adverbs* on ordinary actions ("Clumsy Sims will trip themselves up").
   - Evans also uses a Boltzmann temperature that rises "when the Sim is doing badly" [P].

5. **"Never lose intent silently" is a legibility problem, and there are known tools for it.**
   - Orkin used dialogue to explain inaction ("I've got nowhere to go!") [P].
   - Carlisle: gaze and the order of events decide whether a viewer reads an act as intended or as something that happened to the character [P].
   - Zubek recommends thought bubbles that name the need being served [P].

6. **Organising the house is a separate optimisation problem.**
   - Furniture-layout research uses simulated annealing or MCMC over a cost function built from design rules (Yu et al. 2011; Merrell et al. 2011) [Q].
   - The optimiser's output should become *desires* for the behaviour engine ("sofa wants to face the TV"), which she then carries out as ordinary tasks.
   - It should not replace the behaviour engine.

---

## 1. Architecture-by-architecture

### 1.1 Utility AI: Infinite Axis Utility System (Dave Mark; Lewis & Mark)

- **Core idea.**
  - Each candidate decision (a "DSE", decision score evaluator) is scored by multiplying many independent *considerations*.
  - Each consideration normalises one input to [0,1] using "bookends", then passes it through a designer-chosen response curve.
  - The best score wins the think cycle. Lewis: "These scores are then multiplied together to obtain the overall score of the DSE itself."
  - Any consideration scoring zero disqualifies the decision, and evaluation stops early [P, Lewis, *Game AI Pro 3* ch.13].
- **Goals, plans, state.**
  - There are no plans and no explicit goals; goals are implicit in the considerations.
  - State is whatever the inputs read.
  - Graham's introduction covers linear, quadratic, logistic and piecewise-linear curves, noting the piecewise-linear one "is exactly what The Sims uses" [P, Graham, *Game AI Pro* ch.9].
- **Interruption and replanning.**
  - It re-decides every think cycle, so it is trivially interruptible.
  - The known pathology is oscillation. Graham's fix is *inertia*: "add a weight to any action that you are already currently engaged in… remain committed until something truly better comes along", or a cooldown [P].
  - Lewis adds *runtime* and *cooldown* considerations "designed to break cycles of repetitive behavior" [P].
  - Graham also describes *bucketing* (dual utility): "a starving Sim will never even consider watching TV" [P].
- **Authoring model.**
  - Fully data-driven. Lewis: "the setup of the response curve… is entirely data driven, there is no engineering burden for modifying the preferences" [P].
  - To add a behaviour, a designer adds a DSE with a list of (input, curve) pairs.
- **Production uses.**
  - Guild Wars 2: Heart of Thorns, "modeled on the Infinite Axis Utility System (Mark 2013)" [P, Lewis].
  - GDC 2015, "Building a Better Centaur: AI at Massive Scale" (Lewis & Mark) [Q, [GDC Vault](https://www.gdcvault.com/play/1021848/Building-a-Better-Centaur-AI)].
  - GDC 2010, "Improving AI Decision Modeling Through Utility Theory" (Mark & Dill) [Q].
- **Failure modes.**
  - No lookahead: it cannot express "do X *so that* Y". That is exactly the sofa problem.
  - Tuning becomes global and fragile as content grows; Zubek makes the same point about needs systems [P].
  - It oscillates without inertia.
  - Multiplying many considerations drags scores towards zero, so some compensation is needed.
- **Fit.**
  - Excellent for **goal selection** (which desire to pursue now). It is close to what `brain.rs` already does (`base × fit × cooldown`, top-4 weighted).
  - Not enough on its own for **sequencing**.
  - Concrete upgrades: an inertia bonus for the active project, and Lewis-style runtime/cooldown curves in place of the flat `0.4^repeats`.

### 1.2 The Sims: smart objects and advertisements (Wright, Forbus, Hopkins; Evans for Sims 3; Zubek's generalisation)

- **Core idea.**
  - Behaviour lives in objects, not in the Sim.
  - Forbus & Wright: each object has "a procedure that implements it…, a procedure that checks to see whether or not it is possible, and a set of advertisements that describe its properties in terms of what need(s) of a Sim it will satisfy". The chosen procedure "is then run in the thread of the Sim itself" [P, [Forbus & Wright 2001](https://qrg.northwestern.edu/papers/Files/Programming_Objects_in_The_Sims.pdf)].
  - Advertisements fade with distance ("attenuation"), and their motive ranges scale with personality [P].
- **Goals, plans, state.**
  - Motives are numeric needs. Advertisements are (action, promised change) pairs.
  - Zubek's loop: "Examine objects… Score each advertisement… Pick the best… Push the action sequence on your queue" [P, [Zubek, *Needs-Based AI*, GPG8 draft](https://robert.zubek.net/publications/Needs-based-AI-draft.pdf)].
  - Multi-step activities are *chained*. With **lazy chaining**, "only one action step is created and run at a time, and when it ends, it knows how to create the next action and front-loads it on the queue" [P].
  - **Progress through a chain is saved in the world**: "the action of prepping food creates a 'prepped food' object… if the agent is interrupted while prepping, the cut up food will just sit there, until the agent picks it up later" [P].
- **Interruption and replanning.**
  - Choices are re-made between actions.
  - Zubek grants rewards inside the action, not at selection, so "Interrupted actions will not be rewarded" [P].
  - **This is the opposite of the current `brain.rs`, which debits needs at choice time.**
- **Authoring model.**
  - Data and scripts live on objects. Adding an object adds behaviour without touching the agent: "the AI can be 'reconfigured' literally by adding or removing objects" [P, Zubek].
  - Evans (Sims 3) states the design rule: "minimize the arrows between code systems… maximize the arrows between design systems".
  - He gives `if (sim.HasTrait(Bookworm) && object is Book) score *= 1.5` as what *not* to put in `FindBestAction` [P, [Evans GDC 2010](https://media.gdcvault.com/gdc10/slides/Evans_Richard_ModelingIndividualPersonalitiesInTheSims3.pdf)].
- **Production uses.**
  - The Sims 1–3 [P].
  - Sims 3 additions [P, Evans]:
    - choosing in layers (lot, then agent, then interaction);
    - maps from need to interactions, so a full Sim skips food entirely;
    - an extra motive per trait ("A couch potato has an extra motive, encouraging him to watch TV and nap");
    - situation motives added and removed over time (a visiting guest gets a "behave appropriately" motive);
    - Boltzmann selection.
  - Zubek reused the approach in Roller Coaster Kingdom [Q].
  - Tomb Raider combines it with GOAP: "The object can 'advertise' an effect that is different from the change that will happen after actual usage" [P, [Conway GDC 2015](https://media.gdcvault.com/gdc2015/presentations/Conway_Chris_Goal-Oriented_Action_Planning.pdf)].
- **Failure modes.**
  - **Addiction and loops.** Forbus: "Early versions of the Joy Booth were so addictive that Sims would continue using it until they collapsed" [P]. Zubek warns that false advertising "create[s] action loops that are very difficult to tune" [P].
  - **Short-sightedness.** Evans (in the Versu paper) notes the Sim expects only that "he will relieve his bladder motive". If someone is already in the bathroom it is blocked and retries, and "This behavior can repeat indefinitely" [P].
  - **Weak at scripted moments.** "Needs-based AI works better for simulated worlds, rather than scripted ones" [P, Zubek].
  - A cheap fix for over-promising objects: Forbus proposes "Skeptical Sims" that compare what an object advertised with what it actually delivered, and rescale its future ads [P].
- **Fit.** Very high, and already the plan of record. Three pieces are missing:
  - objects she creates should advertise *to their maker*, with an ownership/purpose bonus;
  - lazy chaining, with progress stored in the world;
  - reward on completion, not on choice.

### 1.3 HTN planning in games (Killzone 2/3, Horizon Zero Dawn, Transformers: Fall of Cybertron; SHOP lineage)

- **Core idea.**
  - HTN = hierarchical task network. Planning runs forwards and in a fixed order.
  - A top-level compound task has *ordered methods*, each with conditions and subtasks. The planner breaks tasks down depth-first until only primitive actions remain [P, [Humphreys, *Game AI Pro* ch.12](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter12_Exploring_HTN_Planners_through_Example.pdf)].
  - There is no heuristic or cost: "Because we aren't using a heuristic or cost… we can skip any kind of sorting. These features allowed the HTN planner in Transformers: Fall of Cybertron to be considerably faster than our GOAP system used in Transformers: War for Cybertron" [P].
- **Goals, plans, state.**
  - World state is "a vector of properties", and "the world state only needs to represent what is needed" [P].
  - A primitive task has an operator, conditions and effects. Different tasks can wrap the same operator.
  - During execution, the plan runner applies each task's effects to a "working world state". The plan fails if any remaining precondition stops holding [P].
  - Killzone 3 writes its domain in a syntax based on SHOP and compiles it to C++ [P, [Straatman et al., *Game AI Pro* ch.29](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter29_Hierarchical_AI_for_Multiplayer_Bots_in_Killzone_3.pdf)].
- **Interruption and replanning.**
  - Killzone 3: "the agent reruns the planner at a fixed rate and replaces the current plan if it finds one which is better (that is, one that traverses branches that are farther up the list…)".
  - To stay on a plan, Killzone 3 adds branches containing "a single continue task, which lets the planner know that further planning is unnecessary" [P].
  - Humphreys records which method was chosen at each step (a "Method Traversal Record") to compare plan priorities. He also uses partial plans to bound planning cost [P].
  - **His key cautionary example maps directly onto the sofa.**
    - A troll went straight from a slam into a whirlwind attack, because a replan cancelled the recovery step (`DoRecovery`) in between.
    - The fix was to store *why* recovery is needed as world state (`WsIsTired`).
    - Humphreys: "It's important to ask yourself if you are properly representing the world when you run into these types of behavior issues" [P].
- **Authoring model.**
  - The domain is data. Ordered methods read like prioritised recipes, which designers find intuitive.
  - Killzone 3 chose HTN because "it provides good control over what plans can be generated and a clear definition of priorities between plans" [P].
  - To add a behaviour, add a method branch under the relevant compound task.
- **Production uses.**
  - Transformers: Fall of Cybertron [P].
  - Killzone 2/3 multiplayer bots [P for KZ3; KZ2 [Q](https://www.guerrilla-games.com/read/killzone-2-multiplayer-bots)].
  - Horizon Zero Dawn: "a combination of HTN planning and utility based decision making" [S, [Guerrilla, 2017](https://www.guerrilla-games.com/read/the-ai-of-horizon-zero-dawn)].
- **Failure modes.**
  - Priority by list order can be brittle; it is a hidden total order.
  - Replanning on every world change cancels plan endings that were still valid (the whirlwind bug).
  - Doing two things at once needs separate domains [P].
  - How expressive it is depends on how well the world state is designed.
- **Fit: strong for action sequencing.**
  - Planning forwards from the current state is cheap, deterministic, and bounded by the size of the domain.
  - Ordered methods are easy to author, e.g. "if a heap exists, finish it; else if scraps are in reach, gather them; else wander off to look". Comic failure branches are just more methods.
  - Replan at every act boundary (her acts are already finite).
  - Store **no** plan beyond the current act plus an *intention fact in the world*. That satisfies the interrupt rule by construction.

### 1.4 GOAP (Orkin / F.E.A.R.; STRIPS + A*)

- **Core idea.**
  - GOAP = goal-oriented action planning. Actions have preconditions and effects, in the style of the STRIPS planner.
  - A* searches backwards from a goal state to the current state, over a fixed-size array of world-state values.
  - Orkin changed STRIPS in four ways: "We added a cost per action, eliminated Add and Delete Lists for effects, and added procedural preconditions and effects" [P, [Orkin GDC 2006](https://www.gamedevs.org/uploads/three-states-plan-ai-of-fear.pdf)].
  - The character's state machine has only three states: Goto, Animate and UseSmartObject [P].
- **Goals, plans, state.**
  - World state is "an array of four-byte values" (`TargetDead`, `WeaponLoaded`, `AtNode`…), with one slot per concept: "he can only reason about one of each during planning". Systems outside the planner choose which weapon or target is in focus [P].
  - Goals compete for activation. Each character type has its own set of goals and set of actions [P].
- **Interruption and replanning.** It replans when the plan is invalidated, and records what it learned in working memory. Orkin's example: the door is blocked, so the soldier kicks it, then tries the window [P].
- **Authoring model.**
  - Goals and actions are decoupled. Adding `TurnOnLights` (effect `LightsOn`) and making `LightsOn` a precondition of Goto "would affect every goal that was satisfied by using the Goto action" [P].
  - Behaviours are layered like a "seven layer dip" [P].
- **Production uses.**
  - F.E.A.R., F.E.A.R. 2, Condemned 1/2 and Shadow of Mordor [P, [Higley GDC 2015](https://media.gdcvault.com/gdc2015/presentations/Higley_Peter_Goal-Oriented_Action_Planning.pdf)].
  - Tomb Raider 2013 and its sequel [P, Conway]. Tomb Raider added [P]:
    - action costs that depend on the situation;
    - motives that control which goals are available;
    - several candidate plans per object;
    - "situational requirements" that objects impose (`DinnerTable` requires `Food` and `Drink`);
    - open-ended actions that track a "remaining cost";
    - learning from success rates;
    - a designer-authored *behaviour graph* for choosing goals.
- **Failure modes.**
  - **Hard to debug.** Conway: "Designers tend to force behaviors (via scripting) when they don't understand why an NPC is doing something". A debugger must answer three questions: what is it doing now, why not something else, and what did it do before [P].
  - The cheapest plan can look out of character.
  - One slot per concept limits reasoning about several objects at once.
  - A* gets more expensive as the number of actions grows.
  - Transformers moved from GOAP to HTN for speed and control [P, Humphreys].
- **Fit: moderate.**
  - Backward search is the natural way to discover "to lounge I need a seat; to get a seat I can crumple scraps", which is attractive for emergent furniture.
  - HTN gives the same result with explicit, authored methods and better control over charm.
  - Only worth adding as a small "how could I satisfy X?" search if the domain grows too large to author by hand.
  - Conway's idea of objects introducing their own requirements is worth borrowing either way.

### 1.5 Behaviour trees vs "replan every decision"

- **Core idea.**
  - A tree of composite nodes (sequence, selector, parallel) and leaves is "ticked" repeatedly.
  - Nodes that are still running keep their state between ticks. Parallel nodes watch conditions and abort their children when those conditions break [P, [Champandard & Dunstan, *Game AI Pro* ch.6](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter06_The_Behavior_Tree_Starter_Kit.pdf)].
- **Goals, plans, state.**
  - Plans are implicit in sequences, which "purposefully follow 'plans' that are hand-specified by the designers" [P].
  - The state lives in running nodes, i.e. in the program, not in the world.
- **Interruption and replanning.**
  - Done by watching conditions and aborting, with cleanup in `onTerminate` [P].
  - Correct interruption needs cleanup code in each node, which is exactly what the proposal rejected.
- **Authoring model.** Visual trees, which are great for designer-scripted sequences.
- **Production uses.** Ubiquitous. Tomb Raider used a behaviour graph *above* GOAP to choose goals [P, Conway].
- **Failure modes.**
  - Hidden state partway through a sequence.
  - Re-ticking from the root loses context unless memory nodes are added.
  - Large trees are hard to maintain.
- **Fit.**
  - The rejection was correct, but for a narrower reason than the proposal states.
  - The real alternative is not "no plans". It is **replanning at every decision over world state, with all continuity in world facts (heaps, intentions, ownership) rather than in an execution stack.**
  - Killzone 3 (replan at a fixed rate, "continue" branches) and Zubek (lazy chaining, progress stored in the world) are the production proofs.

### 1.6 Colony-sim job systems: RimWorld, Dwarf Fortress, Prison Architect

**RimWorld**
- Each pawn has a **ThinkTree**, defined in XML (`ThinkTreeDef`), with `insertTag`s that let modders add branches. Its flow nodes are:
  - `ThinkNode_Priority`: takes the first child that yields a job;
  - `ThinkNode_PrioritySorter`: sorts children by `GetPriority`;
  - `ThinkNode_Random`: picks a child at random.
- The leaves are **JobGivers/WorkGivers**, which return a **Job**.
- A **JobDriver** turns a job into an ordered list of steps called **Toils**:
  - it reserves its targets first (`TryMakePreToilReservations`);
  - it attaches `FailOn…` conditions that cancel the job when they break.
- A separate "constant" think tree is checked about every 30 ticks and can interrupt the current job.
- Sources [S]: [roxxploxx guide](https://github.com/roxxploxx/RimWorldModGuide/wiki/SHORTTUTORIAL:-How-Pawns-Think), [CBornholdt tutorial](https://github-wiki-see.page/m/CBornholdt/RimWorld-AI-Tutorial/wiki/Part-1---Introduction), [RimWorld wiki](https://rimworldwiki.com/wiki/Modding_Tutorials/Code_MendingJob).
- `JobGiver_GetJoy` picks a kind of leisure by weighted random. Weights drop steeply with *tolerance* (boredom with that kind).
- A per-pawn, seeded `pctPawnsEverDo` filter means some pawns simply never do some things [S, [decompile](https://github.com/josh-m/RW-Decompile/blob/master/RimWorld/JobGiver_GetJoy.cs)].

**Dwarf Fortress**
- Jobs come from a global list.
- Per Tarn Adams, as summarised: every hundred ticks, dwarves apply for jobs through an invisible auction that balances competing priorities.
- The player is the "official will of the fortress", while dwarves keep some autonomy.
- A quartermaster-style bureaucracy was dropped as too slow and buggy [S, [Game Developer 2019](https://www.gamedeveloper.com/design/q-a-dissecting-the-development-of-i-dwarf-fortress-i-with-creator-tarn-adams)].
- His published principles: "Don't Overcomplicate… Operate at the level of what the player sees or one layer below" [P, [Adams, *Game AI Pro 2* ch.41](https://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter41_Simulation_Principles_from_Dwarf_Fortress.pdf)].

**Prison Architect**
- When a need crosses a threshold, the prisoner seeks the nearest free object that satisfies it.
- Jobs come from a queue, picked by type, priority and distance.
- Source: a *fan* forum post by user "Daiky", not the developer [S, [Introversion forum](https://forums.introversion.co.uk/viewtopic.php?t=45058)].

**Common analysis**
- **Goals, plans, state.**
  - A job is a short, linear plan stored as data (a list of Toils), with explicit reservations and fail conditions.
  - Progress on things such as construction lives in the world.
- **Interruption and replanning.** Via `FailOn` conditions, the constant think tree, and ending the job with a status. The job is discarded and the next think picks again.
- **Authoring model.** XML trees plus C# job drivers. To add a behaviour, add a JobDef, a driver and a giver, then insert it into the tree.
- **Failure modes.**
  - Reservation deadlocks, and jobs being dropped and re-picked repeatedly. "Dwarves idling with plenty of jobs" is a perennial forum topic [Q].
  - Priority lists are opaque.
- **Fit.**
  - The **Toil list with `FailOn` conditions is a good execution format** for one act or one short plan segment.
  - **Reservations** map onto her claims on things (a heap she is building is hers).
  - Tolerance/boredom and `pctPawnsEverDo` are cheap, deterministic levers for personality.
  - A full think tree is more machinery than one character needs.

### 1.7 MCTS for character behaviour

- **Core idea.**
  - MCTS = Monte Carlo tree search. It grows a tree by sampling: select a branch, expand it, play out to the end (randomly or by heuristic), and propagate the result back up.
  - It is an "anytime" search: it can stop whenever the budget runs out and still give an answer.
- **Goals, plans, state.**
  - Needs a forward model of the world plus a reward.
  - The plan is the most-visited first move, and a fresh search runs at every decision.
- **Interruption and replanning.** Built in: it re-decides every time and can stop at any moment.
- **Authoring model.**
  - Reward/utility weights plus a playout policy.
  - Holmgård et al. built *procedural personas* that "vary solely in the weights of decision-making utilities", passed directly to MCTS [Q, [AIIDE 2015](https://ojs.aaai.org/index.php/AIIDE/article/view/12849)].
- **Production uses.**
  - Total War: Rome II's campaign AI used MCTS to distribute resources and execute tasks in its task-allocation system.
  - In Tommy Thompson's reading, it effectively looked only one turn ahead before switching to random playouts, because the game was too complex [S, [Game Developer 2018](https://www.gamedeveloper.com/design/revolutionary-warfare-the-ai-of-total-war-part-3-)].
  - Roelofs: playouts guided by "a portfolio of heuristics" work better than purely random ones, but expensive heuristics reduce how many iterations fit in the budget [P, [*Game AI Pro 3* ch.28](http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter28_Pitfalls_and_Solutions_When_Using_Monte_Carlo_Tree_Search_for_Strategy_and_Tactical_Games.pdf)].
- **Failure modes.**
  - Needs a forward model that is both accurate and cheap.
  - Random playouts are noisy in a world where the payoffs are comedic or long-term.
  - Results vary between runs unless seeded.
  - Hard to author charm into; it optimises.
  - Quality depends on how many iterations fit in the budget.
- **Fit: low as the core.**
  - The useful part is *shallow lookahead over the real model*, and Versu shows one step is enough (§1.8).
  - Optionally, a capped two- or three-step search could choose *between projects*, e.g. build a sofa now or later.

### 1.8 Interactive drama and declarative character AI

#### Versu (Evans & Short)
- **Core idea.**
  - *Social practices* are reactive joint plans that "never control the agents directly; they merely provide suggestions. It is always the individual agent who decides what to do, using utility-based reactive action selection" [P, [Evans & Short 2014](https://www.cs.uky.edu/~sgware/reading/papers/evans2014versu.pdf)].
  - Practices offer possible actions. The agent's options are "the union of the affordances from each of the practices he is participating in" [P].
- **Goals, plans, state.**
  - The world is a set of sentences in *exclusion logic*: "the simulation state is entirely determined by a set of sentences" [P]. Its tree structure and `!` operator clean up old facts automatically when state changes.
  - Desires are logic sentences with utility modifiers. Example: Brown wants an upper-class man to be displeased with him, worth +20 for each one [P].
  - **Decision rule:** "When considering an action, they actually execute the results of the action, rather than some crude approximation. Then, they evaluate this future world state with respect to their desires. Then, we undo the consequences" [P].
  - Evans calls this "broad rather than deep", and notes it is enough to "play a strong game of whist" [P].
- **Interruption.** None is needed. Every decision is fresh, and practice state lives in the world.
- **Authoring model.**
  - A domain-specific language called Praxis. Practices don't depend on which characters take part, and each personality is a set of desires.
  - A drama manager is just another practice. It "will occasionally lower these desires for certain agents… when it wants them to behave outlandishly for dramatic purposes" [P].
- **Production use.** Versu (iOS, 2013) and Blood & Laurels [Q].
- **Failure modes.**
  - Short-sighted by design.
  - A custom logic language is a big investment.
  - Each decision costs a clone/apply/undo for every option.
- **Fit: very high as a decision rule.**
  - Her world (room, props, scraps) is small, so apply/undo per candidate is cheap and deterministic.
  - It also fixes the short-sightedness Evans describes in The Sims.
  - Practices map well onto routines (morning, work, TV evening) that offer things to do without controlling her.

#### ABL / Façade (Mateas & Stern)
- **Core idea.**
  - A reactive-planning language descended from the Oz Project's Hap [P, [Mateas & Stern 2002](https://users.soe.ucsc.edu/~michaelm/publications/mateas-is-2002.pdf)].
  - Behaviours have preconditions and a *specificity*. The most specific behaviour whose preconditions hold wins, with ties broken at random.
  - *Success tests* and *context conditions* are monitored continuously.
  - It supports parallel behaviours and joint behaviours between characters, and keeps an *active behaviour tree*.
- **Interruption.**
  - "If the context condition fails during execution, then the behavior immediately fails." The failure propagates upwards, and a sibling method is tried [P].
  - *Reflection* lets meta-behaviours "succeed, fail or suspend" other running goals [P]. That is a natural home for distraction: a meta-behaviour that suspends her current goal when something shiny appears.
- **Authoring model.** A code-like DSL compiled to Java.
- **Production use.** Façade (2005).
- **Failure modes.**
  - Enormous authoring effort per dramatic beat.
  - The active behaviour tree is exactly the mid-plan state the proposal wants to avoid.
- **Fit.** Borrow the *ideas* (success tests, context conditions, specificity, reflective meta-behaviours for distraction), not the runtime.

#### Ceptre (Martens)
- **Core idea.**
  - Rules in linear logic consume and produce resources, so state changes need no extra bookkeeping about what stays the same.
  - Rules are grouped into *stages* that run "to quiescence" (until no rule applies).
  - When several rules apply, it uses "uniformly random selection by the engine among all rules that apply", which can be replaced by interactive choice [P, [Martens, AIIDE 2015](https://www.cs.cmu.edu/~cmartens/ceptre.pdf)].
- **Fit.**
  - A good mental model for her material economy: crumpling consumes torn letters, and a heap is consumed into a sofa.
  - It is a prototyping language, not a runtime. Worth borrowing the notation for design docs and for property tests (resources are conserved).

#### Comme il Faut / Prom Week (McCoy, Treanor, Samuel, Reed, Mateas, Wardrip-Fruin)
- **Core idea.**
  - *Social exchanges* with preconditions, and influence rules of the form `<condition> → ±volition`.
  - A *prospective memory*, a "vector of volitions".
  - Explanations "generated from an analysis of the most important rules that fired" [P, [*Game AI Pro* ch.43](https://www.gameaipro.com/GameAIPro/GameAIPro_Chapter43_An_Architecture_for_Character-Rich_Social_Simulation.pdf)].
- **Fit.** Rule-based volition with *explanations* is a good pattern for her debug overlay, and for thought bubbles explaining why she did something.

#### Related, outside the requested list: TED (Horswill & Hill, AIIDE 2024)
- Bottom-up, Datalog-style simulation that "can compactly express the need maximization algorithm in two lines".
- Reported as 2–3 orders of magnitude faster than Prolog, and 25% slower than C# [P, [paper](https://ojs.aaai.org/index.php/AIIDE/article/download/31866/34033)].
- Evidence that declarative character rules need not be slow, and that "derived tables recomputed each tick" is a workable model.

### 1.9 Desktop pets and screensavers

- **eSheep.**
  - Animations are XML with `next` successors carrying probabilities, plus separate `border` and `gravity` transitions.
  - Tooling renders the successor graph to Graphviz [S, [Adrianotiger/desktopPet](https://github.com/Adrianotiger/desktopPet), [issue #6](https://github.com/Adrianotiger/desktopPet/issues/6)].
  - This is the source of the proposal's locomotion graph.
- **Shimeji-ee.**
  - `actions.xml` holds animation sequences. `behaviors.xml` holds weighted behaviours with `Frequency`, `Condition` and `NextBehavior`.
  - It also supports affordances: one mascot `Broadcast`s an affordance, another `ScanMove`s towards it, then both `Interact`.
  - The tutorial notes a delay is needed "to prevent an infinite loop" [S, [Kilkakon tutorial](https://kilkakon.com/shimeji/affordances.php)].
  - This is a smart-object mechanism inside a desktop pet.
- **Neko / oneko** (Watanabe, 1980s; Mac 1989).
  - A hard-coded state machine: chase the cursor, then an idle chain (sit, groom, scratch, yawn, sleep), and a startle when the cursor moves [Q, [Wikipedia](https://en.wikipedia.org/wiki/Neko_(software))].
- **Johnny Castaway** (Dynamix/Sierra 1992).
  - Reimplementations describe a scene director ("ADS") with "conditionals, weighted randomness, sequence chaining", running animation bytecode ("TTM"), with time-of-day vignettes and island state [S, [reimplementation README](https://github.com/1kevgriff/screensaver-johnny-castaway-modern)].
  - There is a story arc of about 120 days: he eventually builds a raft and leaves, then parachutes back [S, [Wikipedia](https://en.wikipedia.org/wiki/Johnny_Castaway)].
- **Little Computer People** (Crane & Gold, 1985).
  - A unique person per disk, with moods and a varying willingness to listen, and no goal [Q, [Wikipedia](https://en.wikipedia.org/wiki/Little_Computer_People)].
- **Goals, plans, state.**
  - None beyond the current animation plus a little world state (tide, raft).
  - Johnny's raft is *progress stored in the world across days*, which is exactly the pattern a persistent heap or sofa needs.
- **Fit.**
  - These are the right layer for **motion and presentation** (successor graphs, scene pools, calendar gating), which the proposal already uses.
  - They have no answer for intent.
  - Johnny's slow raft arc is a precedent for long-running projects that advance in world state and end in comic failure.

---

## 2. Deliberately imperfect, believable agents

- **Oz Project / Mateas.**
  - Believability "is not the same thing as realism… Bugs Bunny is a believable character, but not a realistic character".
  - "For believable agents, personality is king. A character may be smart or dumb… But regardless… everything they do, they do in their own personal style".
  - The requirements include self-motivation and "pursuing multiple, simultaneous goals" [P, [Mateas 1999](https://users.soe.ucsc.edu/~michaelm/publications/mateas-LNAI1600-1999.pdf); Loyall's thesis CMU-CS-97-123 [Q](https://www.cs.cmu.edu/Groups/oz/papers/CMU-CS-97-123.pdf)].
- **Mick West, "Intelligent Mistakes" (2009).**
  - Run the full AI first, then apply plausible mistakes with some probability.
  - That way "the player could never tell in any individual situation if the AI was actually making a mistake".
  - Handicapping takes *more* intelligence, not less [S, [Game Developer](https://www.gamedeveloper.com/programming/intelligent-mistakes-how-to-incorporate-stupidity-into-your-ai-code)].
- **Lidén, "Artificial Stupidity: The Art of Intentional Mistakes"** (*AI Game Programming Wisdom 2*): search snippet only, not read [Q].
- **Carlisle (attribution theory)** [P, [*Game AI Pro 2* ch.38](https://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter38_Psychologically_Plausible_Methods_for_Character_Behavior_Design.pdf)].
  - Viewers classify an action as something the character intended, or something that happened to it, and designers can steer that.
  - "we might hint at the incompetence of a character, which leads the player to likely ascribe positive actions as being external".
  - Gaze is a key cue for intent.
  - "in order for a viewer to correctly attribute a behavior, that behavior must be seen within a context that is temporally plausible".
- **Orkin** [P].
  - "We also use dialogue to explain a lack of action… The A.I. says 'I've got nowhere to go!'"
  - "Vocalizing intentions can sometimes even be enough, without any actual implementation".
- **Evans (Sims 3)** [P].
  - Choice modes: "Always choose the highest-scoring action / Choose randomly from one of the n highest / Choose randomly using the score distribution".
  - A simplified Boltzmann choice, where "Temperature should be cool when he is happy, and should go up when the Sim is doing badly".
  - "Traits provide adverbial modifiers on common actions… Clumsy Sims will trip themselves up… Insane Sims will talk to imaginary people".
  - Insane Sims "care not a jot" for the visitor-etiquette motive, so they "walk straight into your house and eat your food".
- **Zubek** [P].
  - A failed chain can create new objects ("a failed 'cook food' action sequence could create a new 'burned food' object"), or trigger failure actions ("a kitchen fire").
  - This is comic failure expressed as world content.
- **RimWorld** [S]. Tolerance/boredom and per-pawn `pctPawnsEverDo` quirks give deterministic, seeded personality.

---

## 3. The current brain, read against the literature

How `brain.rs` works today:
- `score = base × fit × 0.4^repeats`;
- `fit = 0.1 + need²` if the offer serves a need, else 0.5;
- a weighted pick among the top 4;
- needs are debited **at choice**;
- `Crumple` and `Lounge` serve no need.

| Symptom | Mechanism | Literature fix |
|---|---|---|
| Builds a sofa, doesn't sit | Building has no goal, and the sofa doesn't advertise to its maker any more than to anyone else | Store the reason in the world (Humphreys `WsIsTired` [P]); lazy chaining where the last step queues "sit" (Zubek [P]); objects introduce requirements, so the goal "lounge" plans "make seat" (Conway [P]) |
| Credit at choice | Interrupted acts still satisfy needs | "Interrupted actions will not be rewarded" (Zubek [P]) |
| No commitment | Each decision is independent; only the cooldown links decisions | Inertia bonus for the current project (Graham [P]); Killzone "continue" branches [P] |
| Crude repetition control | Flat 0.4 per repeat among the last three choices | Runtime/cooldown curves (Lewis [P]); tolerance curve (RimWorld [S]) |
| Short-sighted scoring | Scores the advertised change, not the actual outcome | Apply the real effect to a cloned room, score it, discard it (Versu [P]) |
| Personality is only base weights | No traits, no adverbs | Trait motives and adverbs (Evans [P]); persona as utility weights (Holmgård [Q]) |

---

## 4. Synthesis: the recommended combination

**Principle: separate the competent core from the character layer, and keep all plan continuity in world state.**

### 4.0 World state as the single source of truth
- World state is the room, props, scraps and heaps, plus a small set of **intention facts**. Examples: `project: Lounge via Seat#3, started t, owner osaka`, or exclusion-logic style `osaka.project!lounge.seat!3`.
- Half-built things are objects with progress (Zubek's prepped food, Johnny's raft).
- Every instant stays a safe cut, because nothing lives in a program counter. This keeps the proposal's interrupt invariant while making planning possible.
- Her claim on what she is using works like RimWorld's reservations.

### 4.1 Goal selection: utility over desires
- *Desires* are her needs, standing projects, trait motives, and suggestions from routines/practices (as in Versu practices and Sims 3 situation motives).
- Score them with IAUS-style considerations: response curves, multiplied together, with zero disqualifying.
- Add an **inertia bonus for the active project**, which decays with time and distraction.
- Choose with a temperature-controlled softmax or top-k (Evans). Raise the temperature when she is bored or frustrated, so she gets *more* erratic as things go wrong.
- Objects advertise, including objects she made. A made object gets an **owner/purpose advertisement** to its maker ("my sofa: Lounge ×2") until she has used it once.

### 4.2 Action sequencing: a tiny forward HTN, replanned at every act boundary
- The domain is data: ordered methods per desire. Example for `Lounge`:
  1. a seat exists → go and sit;
  2. a heap of mine exists → finish crumpling, then sit;
  3. scraps are in reach → gather, crumple, sit;
  4. otherwise → lie on the floor and grumble.
- Comic failure methods sit in the same table. For example, a sofa that collapses becomes a "squashed heap" object (Zubek).
- **Only the next act is executed. The rest is recomputed at the next decision from world facts.**
  - A Killzone-style continue rule plus the inertia bonus provide commitment.
  - The whirlwind bug can't happen, because the reason (the project fact) is in the world.
- Optional Versu-style evaluation:
  - For the top few candidate first steps, apply their real effects to a cloned room and score the result against her desires.
  - The room is small, so this should take well under a millisecond. That figure is not measured; add a perf test in `tests/perf.rs`.
- MCTS is not needed. If longer horizons are ever wanted ("build a shelf so the TV fits"), use a two- or three-step capped search over projects, with heuristic playouts (Roelofs) and a seeded RNG.
- GOAP-style backward search is only worth adding if there are too many methods to author by hand. Conway's object-introduced requirements give much of its benefit inside HTN.

### 4.3 Personality and dumbness as first-class perturbations
Apply these *after* the competent choice (West). All are seeded and deterministic.
- **Distraction.**
  - An ABL-style meta-behaviour, or a spike in how salient something is, suspends the project fact without deleting it.
  - The project's inertia decays, so she may come back to it later. That is forgetting with a trace.
- **Forgetting.**
  - Project facts expire on a decay curve.
  - **Invariant: no project fact is removed without an `Abandon` event that produces a visible beat.** Examples:
    - a glance back at the sofa;
    - a "…" or "what was I doing?" bubble;
    - carrying on with an Orkin-style excuse line.
  - Carlisle: the glance makes it read as *her* forgetting, not a bug.
  - Make this a property test. For every visit trace: `projects_started == completed + abandoned_with_beat + still_open`.
- **Adverbs.** Traits change *how* acts play out (clumsy trips, spacey pauses, overthinking stalls) rather than adding new acts (Evans).
- **Quirks.** Seeded per visit or per ledger, e.g. "never uses the bed" or "always sits backwards" (RimWorld `pctPawnsEverDo`).
- **Boredom.** Tolerance curves per kind of activity (RimWorld; Lewis's runtime consideration).
- **Wrong-model gags.**
  - Objects may falsely advertise on purpose, but only for a scripted gag, never as a general mechanism. Zubek and Forbus both warn that it causes loops.
  - Forbus's "Skeptical Sims" memory could let her learn that a prop disappoints.

### 4.4 House organisation (the "high-end wish" items)
- Layout preferences are *desires* over the room, for example:
  - the sofa faces the TV;
  - the sofa is on the same floor as the TV;
  - the bed is somewhere quiet;
  - paths stay clear.
- They are scored by a cost function, as in the furniture-layout papers (Yu et al. 2011: simulated annealing with Metropolis–Hastings; Merrell et al. 2011: MCMC over design guidelines) [Q].
- A background **anytime proposer** runs a few Metropolis steps per decision, deterministic from the visit seed.
  - It proposes projects: "move X to Y", "build stairs here", "wall off this nook".
  - She pursues them through the same HTN, as tasks with progress stored in the world (carrying the sofa, a half-built wall).
- Walls, stairwells and moved furniture are just new object types with affordances and terrain effects. Nothing is special-cased. This is Zubek's and Evans' "add objects, not code".
- She doesn't need the optimal layout. Proposals go through the same temperature and dumbness layer, so she might turn the sofa to face the window "because the TV was looking at her".

### 4.5 Authoring format
- Rust data tables (consts or RON) for:
  - desires: curves and inputs;
  - advertisements per kind of object;
  - HTN methods: ordered, with conditions as small predicates over the room;
  - trait adverbs.
- Follow Evans' rule: no `if trait && object` in the scorer.
- CiF-style explanations of which rules fired feed both the stage debug view and her thought bubbles.
- The log should answer Conway's three debug questions: what is she doing now, why not something else, and what did she do before.
- A full logic DSL (Praxis, TED, Ceptre) is not warranted. Exclusion-logic-style tree paths for intention facts are worth borrowing if useful.

### 4.6 How this answers the user's framing
- **"Split state search from preference/utility":**
  - utility chooses *what* she wants (4.1);
  - the HTN, plus optional one-step evaluation of real consequences, chooses *how* (4.2);
  - the layout annealer chooses *where things should be* (4.4);
  - the character layer chooses *how badly it goes* (4.3).
- **"Simulated annealing / MCTS":**
  - Simulated annealing / MCMC belongs to room layout, where the literature supports it.
  - MCTS is unnecessary at this scale. Versu shows one step over the real model is enough for strong play, and Rome II needed heavy decomposition before MCTS paid off.
- **"Declarative":**
  - Methods, advertisements and desires become data.
  - All plan state becomes world facts.
  - The interrupt invariant then holds by construction, instead of by forbidding planning.

---

## 5. Gaps and unverified claims

- **Not read in primary form:**
  - Dave Mark's IAUS talks (I used Lewis's chapter on Heart of Thorns instead);
  - Sims 2/4 internals and Hopkins' writings;
  - SHOP2 papers (only reached via Killzone 3's statement that its domain syntax is based on SHOP);
  - Killzone 2 slides, and Horizon Zero Dawn slides (only Guerrilla's abstract);
  - Lidén, and Loyall's thesis;
  - Little Computer People internals;
  - Prom Week's own papers (CiF chapter only);
  - the furniture-layout papers (abstracts and search results only).
- RimWorld details come from community tutorials and a decompile, summarised by a fetch tool. They are not Ludeon documentation.
- The Prison Architect description is a fan forum post.
- The Rome II "one turn ahead" detail is Tommy Thompson's secondary reading.
- CPU claims in §4 are estimates and need a perf test.

## Sources

**Read in full [P]**
- [Orkin, Three States and a Plan (GDC 2006)](https://www.gamedevs.org/uploads/three-states-plan-ai-of-fear.pdf)
- [Humphreys, Exploring HTN Planners through Example](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter12_Exploring_HTN_Planners_through_Example.pdf)
- [Straatman et al., Killzone 3 bots](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter29_Hierarchical_AI_for_Multiplayer_Bots_in_Killzone_3.pdf)
- [Graham, Utility Theory](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter09_An_Introduction_to_Utility_Theory.pdf)
- [Lewis, Utility Considerations](http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter13_Choosing_Effective_Utility-Based_Considerations.pdf)
- [Champandard & Dunstan, BT Starter Kit](http://www.gameaipro.com/GameAIPro/GameAIPro_Chapter06_The_Behavior_Tree_Starter_Kit.pdf)
- [Roelofs, MCTS Pitfalls](http://www.gameaipro.com/GameAIPro3/GameAIPro3_Chapter28_Pitfalls_and_Solutions_When_Using_Monte_Carlo_Tree_Search_for_Strategy_and_Tactical_Games.pdf)
- [Adams, DF Principles](https://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter41_Simulation_Principles_from_Dwarf_Fortress.pdf)
- [Carlisle, Psychologically Plausible Methods](https://www.gameaipro.com/GameAIPro2/GameAIPro2_Chapter38_Psychologically_Plausible_Methods_for_Character_Behavior_Design.pdf)
- [McCoy et al., CiF](https://www.gameaipro.com/GameAIPro/GameAIPro_Chapter43_An_Architecture_for_Character-Rich_Social_Simulation.pdf)
- [Forbus & Wright, Programming Objects in The Sims](https://qrg.northwestern.edu/papers/Files/Programming_Objects_in_The_Sims.pdf)
- [Evans, Sims 3 Personalities (GDC 2010)](https://media.gdcvault.com/gdc10/slides/Evans_Richard_ModelingIndividualPersonalitiesInTheSims3.pdf)
- [Zubek, Needs-Based AI](https://robert.zubek.net/publications/Needs-based-AI-draft.pdf)
- [Conway, GOAP in Tomb Raider](https://media.gdcvault.com/gdc2015/presentations/Conway_Chris_Goal-Oriented_Action_Planning.pdf)
- [Higley, GOAP at Monolith](https://media.gdcvault.com/gdc2015/presentations/Higley_Peter_Goal-Oriented_Action_Planning.pdf)
- [Evans & Short, Versu](https://www.cs.uky.edu/~sgware/reading/papers/evans2014versu.pdf)
- [Mateas & Stern, ABL](https://users.soe.ucsc.edu/~michaelm/publications/mateas-is-2002.pdf)
- [Mateas, Oz-Centric Review](https://users.soe.ucsc.edu/~michaelm/publications/mateas-LNAI1600-1999.pdf)
- [Martens, Ceptre](https://www.cs.cmu.edu/~cmartens/ceptre.pdf)
- [Horswill & Hill, TED](https://ojs.aaai.org/index.php/AIIDE/article/download/31866/34033)
- [Bourse, AI in The Sims (student report, secondary)](https://yo252yo.com/old/ens/sims-rapport.pdf)

**Summarised by fetch tool [S]**
- [Guerrilla, HZD AI](https://www.guerrilla-games.com/read/the-ai-of-horizon-zero-dawn)
- [Thompson, Total War AI pt 3](https://www.gamedeveloper.com/design/revolutionary-warfare-the-ai-of-total-war-part-3-)
- [Adams Q&A 2019](https://www.gamedeveloper.com/design/q-a-dissecting-the-development-of-i-dwarf-fortress-i-with-creator-tarn-adams)
- [RimWorld: How Pawns Think](https://github.com/roxxploxx/RimWorldModGuide/wiki/SHORTTUTORIAL:-How-Pawns-Think)
- [RimWorld AI Tutorial](https://github-wiki-see.page/m/CBornholdt/RimWorld-AI-Tutorial/wiki/Part-1---Introduction)
- [RimWorld wiki Mending Job](https://rimworldwiki.com/wiki/Modding_Tutorials/Code_MendingJob)
- [JobGiver_GetJoy decompile](https://github.com/josh-m/RW-Decompile/blob/master/RimWorld/JobGiver_GetJoy.cs)
- [Prison Architect fan post](https://forums.introversion.co.uk/viewtopic.php?t=45058)
- [West, Intelligent Mistakes](https://www.gamedeveloper.com/programming/intelligent-mistakes-how-to-incorporate-stupidity-into-your-ai-code)
- [Shimeji affordances](https://kilkakon.com/shimeji/affordances.php)
- [Johnny Castaway reimplementation](https://github.com/1kevgriff/screensaver-johnny-castaway-modern)
- [Johnny Castaway (Wikipedia)](https://en.wikipedia.org/wiki/Johnny_Castaway)
- [Versu (IFWiki)](https://www.ifwiki.org/Versu)

**Search snippets only [Q]**
- [Building a Better Centaur](https://www.gdcvault.com/play/1021848/Building-a-Better-Centaur-AI)
- [Killzone 2 Bots](https://www.guerrilla-games.com/read/killzone-2-multiplayer-bots)
- [MCTS Personas](https://ojs.aaai.org/index.php/AIIDE/article/view/12849)
- [Yu et al., Make it Home](https://web.cs.ucla.edu/~dt/papers/siggraph11/siggraph11.pdf)
- [Merrell et al., Furniture Layout](http://graphics.berkeley.edu/papers/Merrell-IFL-2011-08/)
- [Loyall thesis](https://www.cs.cmu.edu/Groups/oz/papers/CMU-CS-97-123.pdf)
- [Neko](https://en.wikipedia.org/wiki/Neko_(software))
- [Little Computer People](https://en.wikipedia.org/wiki/Little_Computer_People)
- [eSheep](https://github.com/Adrianotiger/desktopPet)
- [Artificial stupidity](https://en.wikipedia.org/wiki/Artificial_stupidity)

**Relevant project files**
- `/home/svein/dev/dessplay/dessplay/src/ui/houseguest/brain.rs`: `Kind::base`, `Kind::serves` (where `Crumple`/`Lounge` → `None`), `score`, `choose`.
- `/home/svein/dev/dessplay/docs/proposals/2026-09-28-houseguest.md`: sections "The brain" and "Rejected alternatives".