# ULTRAKRUST

A Rust (Bevy) reimplementation of ULTRAKILL, starting with V1's movement.

**Milestone 3: 0-1 is playable.** Start to finish: the drop-in intro, punching through the planks, the revolver
pickup and title card, every arena and wave, doors, glass floors, fans, checkpoints and respawns, Filth, Strays,
the Malicious Face, and the final pit. Gameplay scripts are ports of ULTRAKILL's own (see MODLOG.md), running on
the level data read from your install.

**Milestone 2: real levels.** `ultrakrust` loads level 0-1 straight from your Steam install at
runtime (geometry, textures, collision, spawn point) with a Rust reader for Unity's bundle format.
`--level 1-1` picks another level, `--sandbox` opens the movement test map, `N` toggles noclip.

**Milestone 1: movement sandbox.** Walk, jump, dash, slide, slam, slam-jump, wall-jump (×3),
wall cling, super slide jumps and the piercer revolver, on a test map built to exercise each of them.

## How it was built
- The movement logic is a clean-room port. It's based on reading how ULTRAKILL's `NewMovement`, `GroundCheck`,
  `WallCheck`, `CameraController` and `Revolver` behave in a locally decompiled copy of the
  author's own install. No decompiled code is included in this repository.
- Every tuning value (walk speed 750, jump power 90, air acceleration 6000, wall-jump power 150,
  the 125 Hz physics step, gravity -40, collider sizes, the frictionless player material) was read
  from the game's own data files. See [MODLOG.md](MODLOG.md) and
  [`crates/uk-core/src/consts.rs`](crates/uk-core/src/consts.rs).
- No game files, textures, models or sounds ship with this project. Levels and textures are read
  from *your* ULTRAKILL install at runtime (set `ULTRAKILL_DIR` if it isn't in the default Steam folder).

## Parity
`cargo run --release -p ultrakrust` draws levels with ULTRAKILL's own compiled shaders (`--legacy-render` for the old stand-in) (translated at load
from your install; nothing is stored). `cargo run --release -p uk-harness -- --render` checks every level that way.

See [PARITY.md](PARITY.md) for the measured gap analysis and work order. Run the headless harness with
`cargo run --release -p uk-harness -- --full`. It prints only failures, regressions and gaps.

## Layout
| Crate | What |
|---|---|
| `crates/uk-core` | Engine-agnostic simulation: collision world (oriented boxes + triangle meshes with a BVH), player movement, camera, revolver. Headless tests check it against values derived from the original formulas. |
| `crates/uk-assets` | Runtime reader for your install: UnityFS bundles (LZ4/LZMA), SerializedFiles via typetrees, meshes, textures (RGB24/RGBA32/DXT1/DXT5), materials, and level scene extraction. |
| `crates/uk-game` | Level runtime: Unity-like object model (activation, triggers, timers, UnityEvents) running ports of the progression scripts, enemies, combat, checkpoints. Headless probes + an autopilot in `examples/`. |
| `crates/uk-harness` | Headless parity, regression and performance harness: probes as checks, coverage of every scene, determinism, baseline in `parity/baseline.tsv`. |
| `crates/ultrakrust` | Bevy 0.19 frontend: level loading, input, test map, view model, HUD, `--tour` screenshots. |

## Run
```bash
cargo run --release -p ultrakrust
```
```bash
cargo test --release --workspace
```
```bash
cargo run --release -p uk-assets --example dump_level -- level0-1
```

Controls (ULTRAKILL defaults): **WASD** move, **Space** jump, **Left Shift** dash,
**Left Ctrl** slide (in the air: ground slam), **LMB** fire, **hold RMB** charge a piercing shot,
**F** punch (also parries projectiles), **R** restart from checkpoint, **N** noclip, **T** camera tilt,
**[ ]** mouse sensitivity, **Esc** release the mouse. `--demo <dir>` plays a scripted run and saves screenshots.

## Faithfulness notes
- Physics runs at the game's 125 Hz fixed step. Per-frame logic (inputs, slide state, cling, slam)
  runs every rendered frame, as it does in Unity.
- Quirks are kept on purpose. Wall cling calls `Clamp(-1, 1, x)` with the arguments in the wrong order, so the result
  is always 1. The stamina-fail dash jump scales by the frame delta.
- Not yet ported: navmesh pathing (enemies chase directly), animations (enemies are posed statically),
  sound/music, the level's lights and lightmaps, other weapons, style meter and ranks.

Built with AI assistance (Claude), using ILSpy and UnityPy for inspection. ULTRAKILL is © New Blood
Interactive / Arsi "Hakita" Patala. This is an unofficial fan project, and you need to own the game.
