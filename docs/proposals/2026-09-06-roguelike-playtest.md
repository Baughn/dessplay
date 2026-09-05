# The Waiting Below: playtest and fun proposals

Date: 2026-09-06. Status: draft ideas, not adopted design rules.

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

No gameplay rules are changed by this document. Adopted proposals should
update `design.md` and `decisions.md` when implemented.
