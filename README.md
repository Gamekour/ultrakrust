# ULTRAKRUST

A Rust (Bevy) reimplementation of ULTRAKILL, starting with V1's movement.

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
- No game files, textures, models or sounds ship with this project. A later milestone will load
  levels and assets from *your* ULTRAKILL install at runtime.

## Layout
| Crate | What |
|---|---|
| `crates/uk-core` | Engine-agnostic simulation: collision world (oriented boxes), player movement, camera, revolver. Headless tests check it against values derived from the original formulas. |
| `crates/ultrakrust` | Bevy 0.19 frontend: window, input, test map, view model, HUD. |

## Run
```bash
cargo run --release -p ultrakrust
```
```bash
cargo test -p uk-core
```

Controls (ULTRAKILL defaults): **WASD** move, **Space** jump, **Left Shift** dash,
**Left Ctrl** slide (in the air: ground slam), **LMB** fire, **hold RMB** charge a piercing shot,
**R** respawn, **T** toggle camera tilt, **[ ]** mouse sensitivity, **Esc** release the mouse.

## Faithfulness notes
- Physics runs at the game's 125 Hz fixed step. Per-frame logic (inputs, slide state, cling, slam)
  runs every rendered frame, as it does in Unity.
- Quirks are kept on purpose. Wall cling calls `Clamp(-1, 1, x)` with the arguments in the wrong order, so the result
  is always 1. The stamina-fail dash jump scales by the frame delta.
- Not yet ported: enemies, parry/punch, other weapons, water, moving platforms, gravity volumes,
  portals, and sound.

Built with AI assistance (Claude), using ILSpy and UnityPy for inspection. ULTRAKILL is © New Blood
Interactive / Arsi "Hakita" Patala. This is an unofficial fan project, and you need to own the game.
