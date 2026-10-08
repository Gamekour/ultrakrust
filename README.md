# ULTRAKRUST

An unofficial reimplementation of ULTRAKILL in Rust, on the Bevy engine. ULTRAKRUST loads levels
straight from your own ULTRAKILL install while it runs, and draws them with the game's own compiled
shaders. The gameplay scripts are ports of ULTRAKILL's own, so movement, combat and level logic
behave the way the original does.

**You need to own ULTRAKILL on Steam.** This repository contains no game files: no models,
textures, sounds, shaders or decompiled code. Everything is read from your install at runtime and
never written back.

## What works
- **Level 0-1 from start to finish.** That covers the drop-in intro, the revolver pickup and title
  card, every arena and wave, doors, glass, fans, checkpoints and respawns, Filth, Strays, the
  Malicious Face, and the final pit with the results screen.
- **Every campaign scene loads,** with its geometry, collision, lights, triggers and level-start
  logic. The exit elevator loads the next level. Most enemies and weapons beyond those above are
  not ported yet.
- **V1's movement:** walk, jump, dash, slide, slam, slam-jump, wall-jump, wall cling and super
  slide jumps, at the game's 125 Hz physics step and with its tuning values.
- **Effects:** blood and gore, environment hit particles, slide and wall scrapes, the HUD and
  ULTRAKILL's post-processing.
- **Custom maps** from glTF files (see below).

[PARITY.md](PARITY.md) has the measured gap analysis against the original. [MODLOG.md](MODLOG.md)
records each change, with the probe numbers behind it.

## Getting started
1. Install ULTRAKILL through Steam.
2. Install Rust from [rustup.rs](https://rustup.rs) (stable toolchain).
3. Clone this repository and run:

```bash
cargo run --release -p ultrakrust
```

The first build takes several minutes. The game is found automatically in the default Steam folder
or any Steam library. If it isn't, point `ULTRAKILL_DIR` at the folder that contains
`ULTRAKILL_Data`:

```bash
ULTRAKILL_DIR="D:/Games/ULTRAKILL" cargo run --release -p ultrakrust
```

### Options
Pass these after `--`, e.g. `cargo run --release -p ultrakrust -- --level 1-1`.

| Flag | What it does |
|---|---|
| `--level 1-1` | Start in another level (default 0-1). |
| `--map path/to/map.glb` | Load a custom glTF map (see below). |
| `--all-weapons` | Start with every ported weapon. |
| `--sandbox` | The movement test map instead of a level. |
| `--legacy-render` | Bevy's own renderer instead of ULTRAKILL's shaders. |
| `--exit-after 10` | Quit after this many seconds. |

### Controls
ULTRAKILL's default bindings:

| Key | Action |
|---|---|
| **WASD** | Move |
| **Space** | Jump |
| **Left Shift** | Dash |
| **Left Ctrl** | Slide (in the air: ground slam) |
| **LMB** | Fire |
| **Hold RMB** | Charge a piercing shot |
| **F** | Punch (also parries projectiles) |
| **R** | Restart from the checkpoint |
| **Tab** | Level stats |

Extras:

| Key | Action |
|---|---|
| **N** | Noclip |
| **F3** | Developer overlay |
| **T** | Camera tilt |
| **[ ]** | Mouse sensitivity |
| **Esc** | Release the mouse |

Your saved settings and progress are read from your ULTRAKILL install (read-only).

## Custom maps
Build a map in Blender (or anything that exports glTF), export it as `.glb` (or `.gltf` with a
separate `.bin`), and run:

```bash
cargo run --release -p ultrakrust -- --map path/to/map.glb
```

You get V1, the HUD and the level timer, with your feet at the origin (0, 0, 0), facing −Z
(Blender's +Y).

**Collision** uses [Godot's import name suffixes](https://docs.godotengine.org/en/stable/tutorials/assets_pipeline/importing_3d_scenes/node_type_customization.html).
Add them to object names. They're case-insensitive, and `-x`, `_x` or `$x` all work.

| Suffix | Result |
|---|---|
| `-col` | Visible mesh + exact (triangle mesh) collision |
| `-convcol` | Visible mesh + convex hull collision |
| `-colonly` | Invisible collision only. On an empty: a sphere (radius 1) by default |
| `-convcolonly` | Invisible convex hull collision only |
| `-noimp` | Not imported (with its children) |

- **Shapes on empties.** glTF doesn't store an empty's display type. To give a `-colonly` empty a
  shape, add a custom property `empty_display_type` (exported as glTF extras):
  - `CUBE`: a box of size 2.
  - `IMAGE`: an infinite ground plane.
  - `SINGLE_ARROW`: a ray. Skipped, since Unity has no ray collider.
- **Meshes without a suffix** are visible but have no collision.
- **Normals matter.** ULTRAKILL culls back faces, and its surface queries are single-sided. Run
  *Mesh → Normals → Recalculate Outside* in Blender if a floor is invisible from above.

**Materials** draw with ULTRAKILL's master shader:
- **Base colour:** the texture times the factor.
- **Emission:** the texture times the colour, times *Emission Strength*.
- **Texture filtering:** set the image to *Closest* in Blender for the crunchy point-filtered look.
- **Formats:** PNG and JPEG textures; embedded or separate files both work. Embedded `data:` URIs
  in a `.gltf` don't.
- **Ignored for now:** smoothness, metallic and vertex colours.
- **No material:** objects fall back to the 0-2 elevator floor.
- **Surfaces:** everything you collide with counts as metal for footsteps and hit effects.

**Lights:**
- Point, spot and sun lights import by type and colour. Blender leaves them out of the export
  by default: tick *Include → Data → Punctual Lights* in the glTF export dialog.
- Brightness is converted from glTF's physical units and scaled down 1000x, to match Blender's
  export (with the export's lighting mode left on Standard). Expect to tune it by eye.
- Cameras are ignored.

Map-specific ULTRAKILL features (enemies, triggers, checkpoints, ...) aren't supported yet. The
planned format is drafted in [docs/MAP_FORMAT.md](docs/MAP_FORMAT.md).

## For developers
| Crate | What it does |
|---|---|
| `crates/uk-core` | Engine-agnostic simulation: collision world, player movement, camera, revolver |
| `crates/uk-assets` | Runtime reader for the install: Unity bundles, serialized files, meshes, textures, materials, shaders, scenes, particles, glTF maps |
| `crates/uk-game` | Level runtime: a Unity-like object model running the ported scripts, enemies, combat, effects. Headless probes in `examples/` |
| `crates/uk-harness` | Headless parity, regression and performance checks over every scene |
| `crates/ultrakrust` | The Bevy frontend: rendering with ULTRAKILL's shaders, input, HUD |

```bash
cargo test --release --workspace
```
```bash
cargo run --release -p uk-harness
```
```bash
cargo run --release -p uk-game --example map_probe -- path/to/map.glb
```

The harness prints only failures, regressions and gaps against `parity/baseline.tsv`.

## Credits
ULTRAKILL is © New Blood Interactive and Arsi "Hakita" Patala. ULTRAKRUST is an unofficial,
non-commercial fan project, not affiliated with or endorsed by them. Please buy the game. Built
with AI assistance (Claude), using ILSpy and UnityPy to inspect the author's own copy of the game.
