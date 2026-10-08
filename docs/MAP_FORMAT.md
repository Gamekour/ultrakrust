# Custom map format: ULTRAKILL features (draft)

> **Status: draft, not implemented.** Today's loader handles geometry, collision suffixes,
> materials and lights (see the README). This document specifies the next layer: enemies,
> arenas and waves, doors, checkpoints and the other level scripts, authored in Blender with name
> suffixes and collections. Every feature below maps onto a script the port already runs, so a
> custom level plays by the same rules as the campaign. Features that aren't ported yet are left
> out and get added here when they are.

## Design rules
1. **Names carry the type, custom properties carry the tuning.** Every feature works with
   just its suffix and the defaults. Custom properties (glTF *extras*) are optional overrides.
2. **Collections carry structure.** A room, an arena, a wave: the collection's children are its
   contents. You never wire objects together with lists of names, except for a few explicit
   cross-references (`activates`, `target`).
3. **The game's own scripts do the work.** The parser writes ULTRAKILL components
   (`ActivateArena`, `ActivateNextWave`, `Door`, `CheckPoint`, ...) with the same fields the
   campaign uses, and the existing ports run them unchanged. Enemies are cloned from 0-1's own
   enemies, which the custom-map scene already loads.
4. **Godot compatibility stays.** The current Godot collision suffixes keep working and
   combine with the new ones.

## Blender export settings
| Setting | Why |
|---|---|
| *Include → Data → Custom Properties* | Exports the tuning values as glTF extras. |
| *Include → Data → Punctual Lights* | Exports lights (already required today). |
| *Data → Scene Graph → Full Collection Hierarchy* | Exports collections as empty nodes, which is how rooms, arenas and waves reach the parser. An object in several collections is exported once, under the first one. |
| *Data → Lighting → Lighting Mode: Standard* | Matches the loader's light conversion. |

## Name syntax
`<name>-<suffix>[-<suffix>...]`
- **Suffixes peel off from the end** while they're recognised. Everything before them is the
  object's name, and that name is what cross-references use. `Gate-door-convcol` is a door named
  `Gate` with convex collision.
- **Separators and case:** `-`, `_` and `$` all work, case doesn't matter, and Blender's
  duplicate counters (`.001`) and trailing digits are ignored. Suffix words therefore never
  contain `_`: write `maliciousface`, not `malicious_face`.
- **One ULTRAKILL suffix per object, at most.** It can combine with one Godot collision suffix.
  Some types imply their own collision (see each type).
- **Unknown suffixes** stay part of the name, with a warning.

## Objects
Placement conventions:
- An empty's origin is the position.
- An empty's local **+Y in Blender** is "forward", the same convention as the player start
  (glTF −Z, Unity +Z).
- Trigger volumes use the existing `-colonly` shape rules:
  - a mesh gives its convex hull;
  - an empty gives a sphere of radius 1 by default, or a box of size 2 with
    `empty_display_type: CUBE`;
  - the object's scale applies.

### Player and level
| Suffix | Object | Becomes | Properties (default) |
|---|---|---|---|
| `-playerstart` | Empty | Where V1 starts, facing its forward. Replaces today's fixed origin start. One per map. | — |
| `-finalpit` | Trigger | `FinalPit`: entering it ends the level and turns the view to the pit's forward; the results screen follows. | `rankless` (false) |
| `-deathzone` | Trigger | `DeathZone`: kills the player (or hurts them, with `damage`) and respawns them at the last checkpoint. | `damage` (0 = instakill), `affects`: `all` / `player` / `enemies` (`all`) |
| `-checkpoint` | Empty or trigger | `CheckPoint`: touching it saves the respawn point, facing its forward. An empty gets ULTRAKILL's default checkpoint volume. | `invisible` (false) |
| `-teleport` | Trigger | `TeleportPlayer`: moves the player to the empty named in `target`. | `target` (required), `reset_speed` (false) |

### Enemies
| Suffix | Becomes | Template (from 0-1) |
|---|---|---|
| `-filth` | Filth (melee husk) | `Zombie` |
| `-stray` | Stray (projectile husk) | `Projectile Zombie` |
| `-maliciousface` | Malicious Face (boss) | `13 Content/Boss/Spider` |

- **Placement:** an empty, standing on the floor, facing its forward.
- **Starting state:**
  - Inside a `-wave` collection: the enemy starts inactive and arrives with the wave, with the
    spawn bubble.
  - Anywhere else: the enemy is in the level from the start, with no bubble.
- **Properties:** `spawn_in` (true in waves, else false) overrides the bubble.
- **Filth and Strays need a navmesh to path to the player** (see `-navmesh`). Without one they
  walk straight at the player (the port's fallback), and the parser warns.
- **Navmesh, implemented:** one tile, with one polygon per triangle. Vertices are welded within
  1 mm. Polygons link across shared edges, and vertical or degenerate triangles are dropped. The
  mesh is used as-is: authors leave the agent radius (0.5 m) clear of walls themselves, since
  there's no erosion. Agent settings come from 0-1's humanoid navmesh.
- **Implemented:** the three suffixes, outside waves only (no `-wave` or `spawn_in` yet).

### Geometry with behaviour
| Suffix | Object | Becomes | Collision | Properties (default) |
|---|---|---|---|---|
| `-door` | Mesh | `Door` (Normal type) plus a `DoorController` area: it opens when the player comes near and closes behind them. Arenas lock it. | Convex | `open`: local offset when open, metres (up by its own height), `speed` m/s (to match 0-1's doors), `start_open` (false), `locked` (false), `trigger_size`: depth of the approach area each side, metres (to match 0-1) |
| `-breakable` | Mesh | `Breakable`: shatters when hit. | Convex | `weak` (false: any hit breaks it), `precision_only` (false) |
| `-glass` | Mesh | `Glass`: breaks when hit or touched. | Tri | — |
| `-water` | Mesh or empty volume | A `Water` volume: swimming physics and the underwater overlay. | Trigger | `color` RGBA (0, 0.5, 1, 1) |
| `-navmesh` | Mesh | The enemies' walkable surface: invisible, no collision. Several are merged. Already a Godot suffix. Implemented. | — | — |
| `-hint` | Trigger | `HudMessage`: shows `message` while the player is inside it (or for `seconds`). | Trigger | `message` (required), `seconds` (0 = while inside), `once` (true) |
| `-trigger` | Trigger | `ObjectActivator`: activates everything named in `activates` when the player enters it. | Trigger | `activates` (required), `delay` s (0), `once` (true) |
| `-start` | Trigger | (inside an `-arena`) starts the fight; see below. | Trigger | — |
| `-dualwield` | Trigger | `DualWieldPickup`: gives the dual-wield power-up. | Trigger | `infinite` (false), `seconds` (30, to check against the game) |

## Collections
| Suffix | Becomes | Contains |
|---|---|---|
| `-room` | A room: an organising group, and the scope of the arenas and doors inside it. | Anything |
| `-arena` | `ActivateArena` on each of its `-start` triggers. | One or more `-start` triggers, plus one or more `-wave` collections |
| `-wave` | `ActivateNextWave` on the collection's node. | Enemies, plus an optional `-onclear` collection |
| `-hidden` | Starts inactive. | Anything |
| `-onclear` | (inside a wave) `ActivateNextWave.toActivate`: shown when the wave is cleared. | Anything |

### How an arena runs
This is exactly what the campaign's components do:
1. **The fight starts** when the player enters one of the arena's `-start` triggers (`ActivateArena`):
   - the first wave's enemies spawn;
   - the arena's doors lock. By default those are all the `-door`s in the arena's `-room`.
2. **Waves follow in order.** Waves are sorted by name, so number them: `1-wave`, `2-wave`, ...
   - Each wave counts its own enemies (`enemyCount`); every kill reports to it.
   - When all of a wave's enemies are dead, the next wave spawns (`nextEnemies`).
   - Its `-onclear` contents appear.
3. **After the last wave** (`lastWave`) the doors unlock.

Arena properties are read from the arena collection's extras if the exporter writes them;
otherwise from the first `-start` trigger's:

| Property | Meaning | Default |
|---|---|---|
| `doors` | Comma-separated door names, replacing the room's doors. | The room's doors |
| `spawn_delay` | Off: `noActivationDelay`. | On |

### Cross-references
`activates` and `target` take comma-separated **names** (the part before the suffixes). They
can name objects or collections, and activating a collection activates everything in it. A
name that matches nothing is a load warning, not an error.

## Example (Blender outliner)
```
Scene
├─ Start-playerstart                    (empty)
├─ Floor-navmesh                        (mesh)
├─ Hallway-room
│  ├─ Hall-col                          (mesh)
│  ├─ Tip-hint                          (box empty; message "Watch out!")
│  └─ Ambush-trigger                    (box empty; activates "Ambush")
├─ Ambush-hidden
│  └─ Lurker-stray
├─ Arena-room
│  ├─ Room-col                          (mesh)
│  ├─ Entrance-door                     (mesh)
│  ├─ Exit-door                         (mesh)
│  └─ Fight-arena
│     ├─ Tripwire-start                 (box empty)
│     ├─ 1-wave
│     │  ├─ A-filth
│     │  └─ B-filth
│     └─ 2-wave
│        ├─ C-stray
│        ├─ D-filth
│        └─ Reward-onclear
│           └─ Bridge-col               (mesh, appears when wave 2 is cleared)
├─ Save-checkpoint                      (empty)
├─ Lava-deathzone                       (mesh volume)
└─ End-finalpit                         (mesh volume)
```

## Load-time checks
The parser warns on the following. A warning never stops the map from loading.
- No `-playerstart`, or more than one.
- An arena with no `-start` or no waves, a `-start` outside an arena, or a wave with no enemies.
- Filth or Strays with no `-navmesh` in the map.
- A cross-reference name that matches nothing, or matches more than one object.
- An ULTRAKILL suffix on the wrong kind of object, e.g. `-filth` on a mesh.
- An object in several collections. The exporter keeps only the first one.

## Implementation notes
- **Enemy templates:** clone 0-1's enemy subtrees (renderers, colliders, animators, scripts)
  under the placement node. 0-1 is already the base scene for custom maps, so no extra bundle
  is read. `SceneDef` needs a subtree-copy helper, the counterpart of `retain_nodes`.
- **Scripts:** components are written as `ScriptDef`s whose `data` holds the serialized field
  names of the original. Then `scripts::parse` and the ports run them as if they came from a
  bundle, and parity fixes to those scripts reach custom maps for free.
- **Door defaults and the `DoorController` area size:** to be read from 0-1's doors, not
  invented.
- **Navmesh:** build a `NavMeshData` from the `-navmesh` triangles (polygons, adjacency), the
  same structure that's loaded from the bundles.
- **Name parsing:** `classify` moves from "first matching suffix" to peeling suffixes, ULTRAKILL
  and Godot alike.
- **Probe:** extend `map_probe` to print the generated components per object, and add a sample
  map to the scratchpad generator covering every suffix in this document.

## Not in this draft (not ported yet)
Other enemies, other weapons and weapon pickups (only the revolver exists, and it is given at
the start), the final door animation, music and sound, `HurtZone`, jump pads, moving platforms,
the style meter and challenges, secrets, and level-name cards. Each one gets a suffix here once
its script is ported.
