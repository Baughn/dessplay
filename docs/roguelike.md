# The Waiting Below

A local, turn-based roguelike inside DessPlay. Explore five generated floors,
then return to the surface stair (`<`) on floor one. **Escaping alive is a
victory, with or without the ember.** Taking the ember (`*`) on floor five
awakens the dungeon permanently and makes returning much harder. Bringing it
home is worth an exceptional score. Committed expeditions are meant to be
risky; an injury can turn the next encounter into a fatal mistake.

## Start or resume in DessPlay

Press **F4** or enter **`/rogue`** in chat. Your first visit starts an
expedition; later visits resume it. **?** opens the scrollable guide.
Every action saves locally under your username before its result appears.
**F4** returns to the party; **Esc** closes an inspection page first, then
the game. Closing the game or quitting DessPlay preserves your expedition.
No simulation time passes while idle, reading, or watching an episode.
Automatic recovery advances only while you have deliberately enabled it.
A living expedition cannot be replaced; after death or escape, **n** starts
another.

Party chat stays visible below the game. Arrival notices remain until
**Enter** acknowledges them and interrupt automatic recovery. Playing does
not change Ready/Away or stop playback. Death and escape publish a summary
with your score and the character's fate to party chat, including after an
offline expedition reconnects.

## Controls

| Action | Keys |
|---|---|
| Walk; bump an adjacent creature to attack | Arrows, vi keys, numpad digits |
| Sprint one tile using breath | Uppercase vi keys: `H J K L Y U B N` |
| Attack along a direction, including spear reach | `f`, then a direction |
| Wait and catch breath | `.` or `5` |
| Bandage the most urgent bleeding wound | `a` |
| Eat one ration | `e` |
| Start automatic care and recovery | `r`; any input stops it |
| Use the stair underfoot | `<` or `>`; either works on either stair |
| Take the ember or use a fountain underfoot | `g` |
| Close an adjacent door | `c`, then a direction |
| Swap active and spare weapons | `x` |
| Inspect equipment and ground items | `i`; select an item and Enter to equip |
| Inspect anatomy | `v`; select a region and `a` to treat it |
| Browse the journal | `p` |
| Guide / close current inspection | `?` / Esc |
| Return to the party | F4; Esc from the main game |
| Acknowledge arrival / start again after an ending | Enter / `n` |

```text
y k u       7 8 9
h @ l       4 @ 6
b j n       1 2 3
```

Attacks spend time without moving you. Sprinting never attacks or opens a
door: walk into a closed door to open it, then enter. Diagonals cannot cut
wall corners. Walking into stone spends no time. Supplies and gold are
collected by walking over them; equipment requires an explicit choice.
Your `@` hides the terrain symbol underfoot.

## Time, wounds, and equipment

Walking **does not restore breath**. An ordinary step costs 100 simulation
time; an unburdened healthy sprint costs 50 and consumes breath. Injured legs,
heavy armor, water, and rubble slow movement. Enemies and bleeding act over
that elapsed time, so a slow attack can give a creature multiple opportunities.
Waiting catches breath but still gives the dungeon time to act.

Your kit holds one active weapon, one spare, and armor for the head, torso,
arms, hands, legs, and feet. A knife attacks quickly; a spear reaches two
tiles in a straight direction but needs two working hands and costs 200 time
adjacent to an enemy (150 at reach); a mace is slow and better at breaking protected bones.
Regional armor trades protection against weight. At total carried weight
28 or higher, movement takes 50 additional time units. Equipment inspection
shows your current walking and sprinting time and sprint breath cost. Read
the item details before equipping it. An injury can make your equipped weapon unusable, leaving a
weak unarmed attack until you change weapons or restore your hands.

Blood, bleeding, breath, nutrition, and pain are separate from tissue damage.
Both you and the creatures have flesh, bones, nerves, eyes, and internal
organs. Damaged arms and hands weaken attacks and grip; broken legs and feet
slow escape. Lung damage reduces breath capacity, eye damage shortens sight,
and destroying both eyes leaves only adjacent perception. Blood loss,
starvation, destroyed vital organs, and catastrophic head or torso injuries
can kill. Full blood does not mean your body has healed.

**Rest takes care of routine treatment for you.** It binds bleeding, applies
available splints, eats when needed, catches breath, and recovers blood and
superficial injuries. Linen, splints, food, and nutrition are finite. Ordinary
care stabilizes fractures and cannot regrow limbs or repair lasting nerve
and organ damage. A rare fountain (`&`) restores the whole body, including
lost anatomy, once; press `g` to drink. It then becomes dry (`;`).

In DessPlay, recovery takes at most four saved steps per second, shows your
reserves and supply use, and stops for danger, injuries, arrivals, input,
covering/closing the game, storage errors, or when nothing useful remains.
Manual bandaging remains available under pressure. Losing sight of a creature
does not freeze it; pay attention to audible warnings as well as the map.

## Read the dungeon

| Symbol | Meaning |
|---|---|
| `@`, `#`, `.` | You, wall, floor |
| `<`, `>` | Stairs up, stairs down |
| `+`, `/` | Closed and open doors |
| `~`, `:` | Water and rubble; slower footing |
| `&`, `;` | Restoring fountain and spent fountain |
| `r`, `h`, `W`, `B` | Ash rat, hollow pilgrim, iron warden, cavern brute |
| `!`, `=`, `%` | One linen bandage, splint, food ration |
| `)`, `[` | Weapon, regional armor |
| `$`, `*` | Gold, the ember |

Floors have branches and loops, with optional guarded treasure, fountains,
water, doors, and sealed caverns. Set-pieces do not occur in every run.
Unexplored tiles are blank; remembered terrain is dim. A room corner can
hide an exit until you approach it. Hidden creatures and items are not shown. A remembered route may have changed since you saw it.

Rats bite and retreat. Pilgrims can prepare a call that attracts other
creatures; injuring a caller interrupts it. Wardens and brutes commit to a marked strike location, then
recover: stepping away can leave them hitting empty stone. Visible intent
and threatened squares are shown beside the map. Enemies suffer the same
functional injuries as you. Sprinting and fighting make more noise than
walking; doors and alternative routes can buy room to recover.

Taking the ember explicitly starts waves of warning, breaches, rat swarms,
and falling stone, with lulls between them. The return uses the floors you
explored; there is no fixed shortcut or required return length. Collapses
preserve a traversable route, but do not promise a safe one. Other floors
freeze until you enter them, including their scheduled threats.

Your score rewards exploration and depth. Escaping adds a survival bonus,
banked gold, and a **10,000-point ember bonus**. Kills appear in the report
but do not directly earn points. The ending journal describes death or the
rest of your life, including lasting injuries. There are no shops or
permanent stat unlocks between runs.

The journal retains the latest 512 events. Serious injury feedback and
perception remain available as text. **Settings → Playback & display →
Dungeon injury effects** offers Full, Reduced, and Off. Full briefly flashes
the border red and adds light title distortion; Reduced uses a steady injury
color; Off removes cosmetic effects. These settings never change simulation,
sight, enemy intent, controls, or journal text.

## Standalone and agent play

From the repository root:

```sh
cargo run -p dessplay --example roguelike -- 20260906
cargo run -p dessplay --example roguelike -- 20260906 --json --transcript /tmp/expedition.jsonl
```

Use `nix develop` first if needed. Omit the numeric seed for 42. A seed and
identical action sequence reproduce a run within the same game version.
The harness uses the real engine and the same player observation as the TUI,
with cosmetic effects always off. **Type a command and press Enter.** It
accepts vi/numpad controls, including sprint, attack and door prefixes.
`equip N` selects a ground item; `treat N` selects an anatomical region (both
zero-based). `i`, `v`, or an empty line redisplay the current view; `p` prints
the retained journal; `q` quits. The plain view includes wound details,
intent, supplies, and all events produced by the submitted line.

A line can contain several commands (`lll`), but stops when an action is
blocked, new danger or a new creature appears, the floor changes, or the
run ends. Use individual commands near danger. Here **`r` performs one care
step**: automatic timed recovery, arrows, F4, and inspection pages belong
to the TUI. JSON emits one complete `RunView` per line. Transcripts record
executed actions and observations, without hidden floor state.

The harness writes no game database, connects to no server, and publishes no
party report. Quitting discards its run. For diagnostic policy surveys, see
[testing strategy](testing-strategy.md#roguelike-tests); these are not a
substitute for manual play. Historical observations and implementation
results are in the [playtest proposal](proposals/2026-09-06-roguelike-playtest.md).
