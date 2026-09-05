# The Waiting Below

A local, turn-based roguelike inside DessPlay. Explore five dungeon floors,
pick up the ember (`*`) on floor five, then return to the upward stair (`<`)
on floor one and use it to escape. Picking up the ember does not end the run.
You can play in short sittings; the expedition need not take five minutes.

## Start or resume in DessPlay

Launch `dessplay`, then press **F4** or enter **`/rogue`** in chat. Your first
visit starts an expedition; later visits resume it. Press **?** for the
scrollable in-game guide.

Every action saves locally under your username. **F4** returns to the party;
**Esc** closes the guide first, then the game. Closing the game or quitting
DessPlay preserves your expedition. Nothing in the dungeon happens while you
are reading, away, or watching an episode. A living expedition cannot be
replaced; after death or escape, **n** starts another.

Party chat remains visible below the game. An arrival banner stays until you
press **Enter**. Playing does not change your Ready/Away status or stop video
playback. Death and escape publish your expedition summary to party chat.

## Controls

| Action | Keys |
|---|---|
| Move or attack an adjacent creature | Arrow keys, vi keys, or numpad digits |
| Wait one turn; catch your breath | `.` or `5` |
| Bandage the most urgent wound | `a` |
| Eat one ration | `e` |
| Rest one turn, when safe and not bleeding | `r` |
| Use the stair you are standing on | `<` or `>`; either key works on either stair |
| Open/close the guide | `?` |
| Return to the party | F4; Esc also closes the game when the guide is closed |
| Acknowledge an arrival | Enter |
| Start again after death or escape | `n` |

Diagonals use the outer keys in these grids:

```text
y k u       7 8 9
h @ l       4 @ 6
b j n       1 2 3
```

Move **into** a creature to attack it. An attack uses a turn but does not move
you, even when it kills the creature. Move again to enter the vacated square.
Diagonal movement and attacks cannot cut across a wall corner: take a straight
step through the doorway first. Walking into stone does not spend a turn.

Walk over loot to collect it. Better weapons and armor equip automatically;
there is no inventory equipment screen. Stand on a stair and explicitly use
it to change floors. Your `@` hides the symbol underneath you.

## Read the map and stay alive

| Symbol | Meaning |
|---|---|
| `@`, `#`, `.` | You, wall, floor |
| `<`, `>` | Stairs up, stairs down |
| `r`, `h`, `W` | Ash rat, hollow pilgrim, iron warden |
| `!`, `%` | Two linen bandages, one food ration |
| `)`, `[` | Weapon, armor |
| `$`, `*` | Gold, the ember |

Blank space is unexplored. Explored terrain remains visible, dimmed in the
DessPlay UI, but creatures and loot appear only while in sight. A creature
can cover a loot symbol. Gold contributes to your final score; there is no
shop or spending action.

- **Blood** runs from 0 to 1000. Zero is fatal. **Bleed** is continuing blood
  loss each turn: press `a` to dress a wound. If several wounds bleed, you may
  need several bandages. Bandages can also treat non-bleeding injuries.
- **Wounds** affect six body parts. Severe head or torso injuries can kill
  even with blood remaining. Arm injuries weaken attacks; leg injuries make
  movement more tiring; pain can drain breath during combat. Full blood does
  not mean every wound has healed.
- **Breath** (called Stamina in the standalone harness) is your exertion
  reserve. Attacking spends it. Waiting restores it, but creatures still act
  during that turn. Letting an approaching enemy come to you can preserve
  breath and give you the first attack.
- **Nutrition** is nourishment: a higher number is better. A ration restores
  up to 50 nutrition, capped at 100. Eating above 75 is refused; eating around
  50 avoids wasting much of the ration. At zero, starvation drains blood and
  rest cannot heal you.
- **Rest** requires no visible creature and no bleeding. Each press spends
  one turn, restores breath and blood, and slowly reduces wounds while you
  have nutrition. Check your condition again afterward. Unseen creatures can
  still move; losing sight of an enemy does not freeze it.

A useful opening routine is to look for equipment, approach unfamiliar rooms
carefully, fight enemies one at a time, stop bleeding, and recover before
the next fight. A narrow corridor can prevent a group from surrounding you.
Creatures on other floors freeze, so enemies you bypass may be waiting when
you return with the ember.

## Play without launching the watch party

From the repository root, with the development toolchain available:

```sh
cargo run -p dessplay --example roguelike -- 20260906
```

If using the repository's Nix development environment, enter `nix develop`
first. The argument is a numeric seed; omit it for seed `42`. The same seed
and the same sequence of actions reproduce the same expedition.

This is a plain stdin harness using the real game engine. **Type a command
and press Enter** to act and redraw the map. It accepts the vi/numpad movement
keys and `a`, `e`, `r`, `.`, `<`, `>` listed above. **q**, then Enter, prints
the current summary and quits. Arrow keys, F4, `?`, and the post-game restart
control belong to the DessPlay UI, not this harness; run the command again
to start over.

A line can contain several commands, such as `lll`, but the harness executes
the whole line without pausing when an enemy appears. Use single commands
near danger. It shows only the last four log messages after each line and
does not distinguish remembered terrain with dim coloring or show individual
wounds. Use the in-app game for the full display.

The harness has no save/resume, database writes, server connection, or party
chat reports. Quitting discards its run. It is useful for isolated playtests;
it does not load or replace your DessPlay expedition.

For observations from a completed expedition and proposed improvements, see
the [2026-09-06 playtest](proposals/2026-09-06-roguelike-playtest.md).
