# The Waiting Below: playtest and fun proposals

Date: 2026-09-06. Status: original playtest preserved below; the adopted
overhaul and its validation are recorded after the original report.
Current rules are in [design.md](../design.md#the-waiting-below).

## What I played

I manually played seed **20260906** from a fresh start to escape using
`cargo run -p dessplay --example roguelike -- 20260906`, at parent revision
`f2fef732`. I chose actions from the revealed map and the player instructions.
I used short command batches during exploration, individual actions or small
batches in fights, and long batches across the remembered return route.
I did not use an explorer bot, change the engine, or inspect hidden floor
contents to choose moves. I checked the implementation after finishing.

This evaluates one expedition through the real engine, not the in-app modal,
persistence, or social experience. The harness omits wound details and map
dimming, and displays only four recent messages per submitted line. Some
awkward positioning and lost combat context were consequences of that
interface and my command batching. They are not evidence of a modal defect.
One successful seed does not establish the difficulty distribution.

| Milestone | Turn | Blood | Nutrition | Bandages | Food | Weapon / armor |
|---|---:|---:|---:|---:|---:|---:|
| Start | 0 | 1000 | 100 | 4 | 3 | 0 / 0 |
| Enter floor 2 | 127 | 997 | 87 | 8 | 5 | 1 / 1 |
| Enter floor 3 | 220 | 1000 | 75 | 9 | 7 | 2 / 2 |
| Enter floor 4 | 336 | 1000 | 62 | 9 | 9 | 2 / 3 |
| Enter floor 5 | 441 | 1000 | 97 | 13 | 8 | 4 / 4 |
| Collect ember | 542 | 1000 | 84 | 13 | 9 | 5 / 5 |
| Escape | 935 | 1000 | 45 | 13 | 9 | 5 / 5 |

Final summary: **escaped with the ember; floor 5/5, 30 kills, 249 gold,
935 turns**. I ate once, at turn 419, and used three bandages. The return
took 393 turns (42% of the run), with one fight against a rat bypassed on
the descent. This was a completed manual expedition, not a claim derived
from the existing automated completion tests.

## How it felt

The opening worked. Uncovering rooms, finding a blade, and later buckling on
armor gave clear rewards. The restrained language is good: dried apples,
clean linen, the iron choir, and an ember that fits in a palm make a coherent
little place. The recover-and-return objective is immediately understandable.

The first interesting decision was bandaging a bleeding torso while a hollow
pilgrim approached on floor two (turns 156–161). I could treat the wound,
collect food, wait for the enemy, and then fight. Position and timing mattered.

By floor three I was mostly repeating a routine: walk along the corridor,
bump the enemy until it dies, bandage if needed, rest a few times, continue.
Blood and breath recovered readily, although wounds could remain. Supplies
accumulated, so I began ignoring pickups. Floors had different names but
largely the same experience of crossing the top, descending on the right,
and returning across the bottom.

The strongest encounter came on floor five, around turns 507–515: two
wardens, a pilgrim, and a rat converged near a doorway. One step backward
into the corridor let me fight them one at a time. That was a satisfying
choice with a visible result. It suggests the existing movement and combat
can support more interesting encounters without a large ability system.

The ember pickup was a good line of text followed by an anticlimax. I retraced
five floors, fought the leftover rat, and escaped. The final daylight message
was pleasant, but it arrived hundreds of uneventful actions after the climax.

## Proposed experiments, in priority order

### 1. Give the return a short, distinct shape

Keep the return objective, but prototype an ember-powered shortcut revealed
on pickup: a compact ascent with two or three encounters, ending at the
original surface stair. Foreshadow its sealed doors on the way down so the
payoff feels like using knowledge of the dungeon. The player should see the
new route and its purpose immediately.

For example, carrying the ember could open a short passage with a resting
warden and a clearly marked treasure detour. The direct route gets you out;
the detour offers score at a visible risk. Avoid simply repopulating all five
old corridors, which could turn a long walk into a long cleanup.

First measure whether the ascent contains memorable decisions and ends while
the pickup still feels exciting. A provisional target is 15–25% of total
turns on the return, subject to how it actually plays. If a new ascent is too
large a change, an immediate exit at the ember is a useful comparison
prototype for testing how much the return objective earns its length.

### 2. Make enemies ask for different responses

The current warden already attacks only every other turn, confirmed after
the playthrough, but I perceived it mainly as a tougher bump fight. Show a
visible **raising weapon / strike next turn / recovering** state, tied to
the actual next action. Let stepping away from a telegraphed strike or
attacking during recovery pay off. The timing must survive saving and closing.

Give the other two creatures one readable difference each. A rat could
retreat after biting, creating a choice between chasing and holding position;
a pilgrim could visibly prepare a call that attracts nearby creatures,
creating a reason to prioritize it. These are alternatives to prototype,
not a requirement to add all three mechanics at once.

Start with the warden and one deliberately composed room. The test is
whether a player can explain why they waited, moved, or changed targets.
More damage or more enemy health alone would not address what felt repetitive.

### 3. Put a route decision on each floor

Code inspection confirmed that generation connects six room slots in the
same snake order. Keep guaranteed connectivity, but try an optional branch
and a loop: a guarded equipment room, a safer supply route, and a shortcut
back to the main path. Show enough of the reward and threat at the junction
to make the choice informed.

Let floor names affect play with one small terrain rule, introduced plainly:
shallow water in the drowned pantry could slow movement; cell doors could
separate creatures in the pilgrim cells. Prototype one floor before making
five terrain systems. Test whether players choose different routes based on
their condition, rather than always clearing everything in the same order.

### 4. Make supplies support decisions before tightening scarcity

This run ended with more bandages and food than it began with. Generation
currently supplies four bandages and two rations per floor, on top of the
starting inventory. A ration restores 50 nutrition; ordinary time consumes
one nutrition per ten turns. The abundance I experienced has a plausible
mechanical explanation, but several seeds and less cautious players should
be checked before changing quantities.

Once encounters and route choices exist, try fewer guaranteed supplies with
clearly signposted optional caches. Keep safe recovery convenient. Making
players press `r` twenty more times would add maintenance, not a decision.
If recovery needs a meaningful cost, a visible rest site with a stated
benefit and ration price is a more promising experiment than hidden attrition.

Keep all pressure tied to game actions. Five-minute sessions must still be
safe to interrupt for the watch party. Compare scarcity by whether players
choose a supply detour, retreat, or save an item, not merely by death rate.

### 5. Offer one build choice and make score tempting

Automatic numeric upgrades feel good to find, but there is no equipment
decision afterward. Try one early choice between a spear with reach but
costly close fighting and a knife with a useful recovery-window attack.
State the tradeoffs in the pickup prompt and replace the current weapon;
an inventory grid is unnecessary for this experiment.

Gold currently affects only the summary. Make an optional, visibly guarded
treasure cache near the escape route so it poses a concrete question:
leave safely, or risk the ember for a better story? Test that before adding
shops or persistent progression. Neither a currency economy nor permanent
stat unlocks is needed to find out whether score chasing is fun here.

## Small improvements and what to preserve

The player guide now documents the standalone launch command, Enter-to-act,
the map legend, corner movement, using stairs, nutrition, and the differences
between the harness and the saved in-app game.

For a later UI pass, show actionable consequences beside wound values,
such as an attack penalty or movement cost, and expose enemy intent if it
is introduced. The in-app game already shows individual wounds, pain, an
objective, and an ember-return reminder; this is a suggestion to make those
values easier to use, not to add missing status panels.

Preserve the sparse atmosphere, instant pickups, small control set, local
save/resume, and explicit-turn pacing. The most promising first slice is a
shorter, purposeful return and one readable warden encounter. Play that
manually across several seeds, recording route choices, recoveries, supply
use, and the point repetition begins. Then decide whether wider generation
or resource changes are still needed. Existing seeded invariant and
completion tests can protect the engine; they cannot establish that a run
is fun.

The original report ended here. Its experiments were suggestions, not rules;
the following implementation supersedes the proposed short ascent.


## Adopted overhaul (2026-09-06)

The author's follow-up established a different ambition: most committed
expeditions should fail, and escaping alive without the ember is still a
victory. Taking the ember permanently awakens the existing dungeon, with
breaches, swarms, telegraphed collapses, and lulls. There is no required
shortcut or 15–25% return-length target. The design keeps the original
observation that an empty return was the central problem.

The implementation adds generated loops and branches, optional set-pieces,
a shared layered anatomy system with lasting wounds and limb/organ loss,
regional armor, knife/spear/mace choices and a spare weapon, sprinting that
spends breath, and walking without breath recovery. Rest chooses useful
ordinary treatment and recovery automatically, with visible finite supplies
and cancellable pacing. A rare single-use fountain fully restores even lost
anatomy. Visible enemy commitments and an expanded journal explain tactical
consequences. Endings distinguish survival from ember recovery, award points,
and describe the character's fate. Cosmetic injury feedback is optional and
the observation-only agent harness always disables it.

The existing local save/report transaction remains the persistence boundary.
The author explicitly approved a one-time reset of the roguelike's local
tables because nobody else had played the pre-release game. Schema v8 clears
old runs and local history/pending reports; version-2 saves thereafter retain
the normal error-and-preserve policy. No other local or synced data is reset.
See [decisions](../decisions.md#lasting-injuries-and-an-awakened-dungeon-2026-09-06)
for rationale and [the player guide](../roguelike.md) for controls.

### Initial manual cohort

Two agents chose actions from the player observation and guide, without
reading hidden maps or the engine to decide moves. Each played two cautious
retreats and two committed attempts on the first integrated build. These
runs exposed defects and informed density changes; they do not describe the
final balance.

| Seed | Goal | Result | Actions | Notes |
|---|---|---|---:|---|
| 20260906 | Cautious | Escaped floor 1 | 50 | 16 gold, one rat killed, treated leg wounds |
| 42 | Cautious | Died floor 1 | 51 | Trapped by two pilgrims and two rats during retreat |
| 101 | Cautious | Escaped floor 1 | 39 | 52 gold, bleeding and bone injuries; baited a call |
| 1001 | Cautious | Escaped floor 1 | 62 | 19 gold, uninjured; closing a door enabled retreat |
| 7 | Ember | Engine error on floor 1 | 62 | Pursuers could enter another creature's occupied tile |
| 2026 | Ember | Died floor 2 | 95 | 24 gold, one kill; exhausted linen after a costly passage |
| 314159 | Ember | Died floor 2 | 82 | Pilgrim pincer, exhaustion, and continuing bleeding |
| 271828 | Ember | Engine error on floor 1 | 43 | Same occupied pursuit-goal defect |

Replaying the two interrupted prefixes after the repair changed earlier
creature movement and subsequent randomness. Their continued deaths are not
fresh manual balance evidence and are excluded from the table. Exact action
prefixes remain regression fixtures alongside a generation-independent test
of occupied pursuit goals. A fuzz campaign independently found the same
validation error. Other checks caught unseen named windups leaking into the
journal, a future cavern losing connectivity after a collapse, and closing
a door over dropped equipment. Those classes now have regression coverage.

Final integration review additionally caught responders stalling beside an
occupied call destination, diagonal door closing through walls, blind spear
hits disclosing unseen anatomy, and dodged heavy swings costing enemies no
breath. Each received a failing regression before correction. Responders now
approach a free adjacent tile, door access shares a corner rule, unseen
contact gives generic feedback, and committed heavy swings spend breath
even when dodged.

### Density diagnostics

The observation-only survey ran 100 seeds per policy. The initial cautious
policy escaped 80 times; its committed counterpart died 99 times and hit its
action cap once, with no ember pickups. Injury severity was left intact while
two passes reduced and separated ordinary floor encounters and moved optional
guards away from stair routes. The resulting frozen benchmark had 100
cautious escapes, 91 committed deaths, nine incomplete action caps, and six
ember pickups; all six ember carriers died during the awakened phase.
Seeds 61 and 85 were selected for follow-up manual play because the diagnostic
policy demonstrated that they reached that phase. They are selected coverage
cases, not an unbiased win-rate sample. No difficulty guarantee is inferred
from either scripted policies or this small manual cohort.


The final engine survey, after the tactical edge-case fixes, again ran seeds
1–100 for each policy: 100 cautious escapes; 92 committed deaths and eight
incomplete caps; six ember pickups and no ember escapes. This supports the
intended direction but leaves successful full returns sparsely exercised.
The selected manual benchmark below used the frozen second density build,
before those final edge-case fixes; it is kept separate from this survey.

### Shared-seed manual follow-up

| Seed | Goal | Result | Actions | Ember / first ascent | Score |
|---|---|---|---:|---|---:|
| 61 | Cautious | Escaped floor 1, uninjured, 18 gold | 36 | — | 391 |
| 85 | Cautious | Escaped floor 1, minor arm bone damage, no gold | 146 | — | 564 |
| 61 | Committed | Died from blood loss on floor 4; 18 kills, 132 gold | 959 | 881 / 934 | 2535 |
| 85 | Committed | Died from blood loss on floor 4; 12 kills, 142 gold | 853 | 786 / 814 | 2368 |

Both committed players deliberately prepared before pickup. Seed 85 used a
fountain on floor three to restore an eye and hand injury, then reached the
ember with one linen remaining after descent. Seed 61 brought six linen and
more regional armor into the awakened phase, but took the ember at 883 blood
and 67 breath with unrecovered flesh. Adding iron footwear had also pushed
walking from 100 to 150 time. This was imperfect preparation, not an optimal
return-policy trial. Current walking and sprinting costs are now conspicuous
in equipment inspection and the plain harness. Spears gave useful reach, but
the 200-time adjacent penalty needed a clearer equipment description; that
number is now displayed explicitly.

The return produced the intended new decisions. In seed 85 a collapse erased
a planned bypass around an existing pilgrim; the player lured it aside,
passed it, dodged a warden's marked strike, and finally diverted to an unknown
corridor looking for linen. A brute ahead and rat behind made the last dodge
costly, and continuing bleeding killed the character. Seed 61 used a door and
the rats' bite/retreat rhythm to escape the fifth-floor entrance room, where
seven or eight rats had gathered. All six linen were gone before ascending.
A newly opened shortcut helped on floor four, but accumulated injuries and
more pursuers ended the attempt just short of that floor's upward stair.

Warnings, sprinting, alternate paths, doors, and committed heavy strikes
supported real choices. Neither death was an immediate unavoidable loss on
pickup. Nevertheless, rat accumulation and depleted medical supplies made
both returns sustained attrition. **No sampled generated ember attempt won.**
This demonstrates an active, dangerous return, not settled balance or proof
that its difficulty is appropriate for a human player.

Other pacing questions remain. Both agents spent hundreds of first-floor
actions exploring, including substantial backtracking after mistaking an
unseen room corner for a dead end. Seed 85 took 122 safe recovery steps before
pickup. The harness performs one care action per `r`; the actual TUI automates
those steps at about four per second, so harness input burden is not a TUI
regression. Food was rarely needed in these expeditions. Dense footstep
messages often stopped harness movement batches. These are follow-up tuning
observations, not reasons to make ordinary care manual again.

The helpers used JSON observations and readable map glyphs; one omitted
colored/underlined terrain markers. Its report therefore evaluates audible
collapse warnings and changed routes, not the quality of the TUI's marked
impact tiles. Actual terminal presentation was checked separately below.

### Presentation and automated validation

The production terminal UI and SQLite action handler were exercised in an
isolated tmux session with no networking or party messages. A marked,
validated injury fixture demonstrated the Full red flash and decorative
brain-injury title, Reduced static emphasis, and Off. Normal recovery actions
committed about 261–262 ms apart, visibly consumed splints and food, and
stopped immediately on input. Live rendering exposed a clipped cancellation
instruction; a failing regression led to moving it into the recovery panel's
bottom border. A subsequent equipment-rendering test caught a shared height
helper ignoring explicit line breaks; paragraph sizing now counts those in
all affected panels. This is presentation validation, not a combat playtest.

The full default gate passes **1,404 tests** (five default exclusions), and
`cargo clippy --workspace --all-targets -- -D warnings` passes. Properties
cover world connectivity through crisis transitions, anatomical invariants,
ordinary-care limits, save/resume equality, and finished-run immutability.
A short connected scenario also verifies a successful normal-action descent,
ember pickup, saved/resumed ascent, banked bonus, and immutable ending. This
is contract coverage, not a generated-dungeon win. Regressions cover both
recorded failures and the broader classes found in review. The final AddressSanitizer fuzz campaign completed
**30,694 executions in 601 seconds** with no failure, using long structured action sequences as well
as the mutating corpus. LeakSanitizer was disabled because the environment
uses tracing; address checking and simulation assertions remained enabled.
