# ULTRAKRUST — MODLOG

Goal: a Rust (Bevy) reimplementation of ULTRAKILL, built from reading the user's own Steam install.
Milestone 1 (agreed 2026-10-04): **movement sandbox**, meaning faithful V1 movement plus the revolver on a test map, with no asset loading yet.
Done = runs in Bevy, the movement numbers match the decompiled formulas (core unit tests), a short clip.

## Install facts
- Path: `C:\Program Files (x86)\Steam\steamapps\common\ULTRAKILL`, Steam app 1229490, buildid 22957324
- Unity **2022.3.29f1**, **Mono** (`ULTRAKILL_Data/Managed/Assembly-CSharp.dll`, 2.8 MB), no loader installed, single-player, no anti-cheat
- Content is in Addressables: `ULTRAKILL_Data/StreamingAssets/aa/StandaloneWindows64/*.bundle` (81 bundles, 3.5 GB total).
  Levels: `campaign_scenes_levelX-Y.bundle`; player prefab: `gameprefabs_assets_all.bundle`; scripts: `monoscript_monoscripts.bundle`
- Saves: `<install>/Saves/Slot1` (untouched; nothing writes to the game folder)

## Route
"Reimplement / clean rewrite" route. Decompiled code is read locally in `~/ultrakill-decomp` (outside the repo, never committed).
The repo contains only original Rust code. Constants are facts read from the install. A future asset loader will read the user's own bundles at runtime ("bring your own game files").

## Tools
- ilspycmd 8.2 (dotnet tool, .NET 6 SDK): `ilspycmd -p -o ~/ultrakill-decomp/ac Assembly-CSharp.dll -r Managed` → 1538 .cs files
- UnityPy 1.25.4 in `~/ultrakill-decomp/.venv` (MonoBehaviour typetrees ARE present in the bundles, so no TypeTreeGenerator is needed)
  - `~/ultrakill-decomp/gm.py`: globalgamemanagers (timestep, gravity, layers)
  - `~/ultrakill-decomp/dumpplayer.py`: player prefab hierarchy + NewMovement fields
- `um` is not on PATH under PowerShell (it's a bash script); run it via Git Bash

## Engine facts (read from the install)
- TimeManager fixed timestep **0.008** (125 Hz); max allowed timestep 0.1
- Physics gravity **(0,-40,0)**; bounce threshold 2; default solver iterations 6 (player ×5)
- Layers: 8 Environment, 24 Outdoors, 2 Ignore Raycast (player), 15 Invincible (dashing), 20 GroundCheck, 12 EnemyTrigger
- NewMovement (all 7 player prefab variants agree): walkSpeed **750**, jumpPower **90**, airAcceleration **6000**,
  wallJumpPower **150**, pushForce 0, hp 100, boostCharge 300
- Player Rigidbody: mass **100**, drag 0, interpolate, continuous dynamic, rotation frozen
- Player CapsuleCollider: r **0.5**, h **3.5**, center y **+0.25** (spans -1.5..+2.0 around the transform)
- GroundCheck: local (0,-1.256,0), scale (0.85,0.8,0.85), trigger capsule r0.25 h1.3 → world r 0.2125, h 1.04 (y -1.776..-0.736)
- SlopeCheck: local (0,-0.1,0), trigger capsule r0.45 h3.5 center +0.25 (y -1.6..1.9)
- WallCheck: local (0,-0.1,0), scale 1.8, trigger sphere r0.5 → world r **0.9**
- Main Camera: local (0,1.4,0); default FOV 105 (Unity's vertical FOV)

## Movement code (NewMovement.cs) — what it actually does
- `Time.deltaTime` inside FixedUpdate returns fixedDeltaTime (0.008), so all "per second" factors in Move/Dodge are per tick
- Ground move: targetVel = inputDir·750·0.008·2.75 = **16.5 u/s** (slowMode 1.25 → 7.5); vel = lerp(vel, target, 0.25·friction) per tick.
  When standing still on a slope (slopeCheck), gravity is turned off and the vertical part is zeroed
- Air: accel 6000·0.008/100 = 0.48 u/s per tick, split into right/forward components capped at 16.5 each; then a steering term (see Move())
- Jump: AddForce(up·90·1500·k) → dv = 1350·k·0.008 = **10.8·k** (k: 2.6 normal = 28.08 u/s, 2 slide, 1.5 dash, 1.25 slowMode, slam: 3+(slamForce-1) or 12.5)
- Dash: needs boostCharge ≥ 100 (max 300, regen 70/s); 25 ticks at 49.5 u/s (boostLeft 100, -4/tick), then 16.5; layer 15 = invincible
- Slide: collider h 1.25, transform shifts down 1.125, speed = 24·preSlideSpeed (1..3); preSlideSpeed comes from fall speed/24 or slamForce
- Wall jump: max 3 until grounded; dv = (awayFromWall_horizontal + up)·2000·150/100·0.008 = 24 per component
- Slam: in air, more than 3 above ground, fallTime > 0.5 → vel (0,-100,0); slamForce += 5/s; on landing superJumpChance = 0.1 s → slam jump
- SSJ/wall-SSJ: jump within 4 physics frames (0.032 s) of releasing slide adds speed (TrySSJ)
- Wall cling: falling against a wall caps the slide-down speed (2·clingFade, where clingFade ramps up 4/s to 50)
- Terminal fall speed 100 (clamped in Update)
- Camera: tilt toward -strafe (×5 while dashing); FOV -5% when dashing forward, +10% when dashing back; slide eye drop of 0.625 smoothed

## Revolver (Revolver.cs, variant 0 "piercer")
- shootCharge refills at 200/s → 0.5 s between shots
- Alt fire: hold to charge pierceShotCharge at 175/s, fire at 100

## Log
- 2026-10-04: recon, decompiled, extracted the constants above. Next: workspace = `uk-core` (pure-Rust sim, glam, unit tests) + `ultrakrust` (Bevy 0.19.1 frontend)
- 2026-10-04: uk-core done, 12/12 headless tests pass (walk 16.5, jump 28.08 / apex 9.86, dash 25 ticks @49.5, slide 24,
  air cap 16.5, slam + slam jump, wall jump 24/24 ×3, coyote 0.2 s). Bevy 0.19.1 frontend builds and runs (~200 fps release).
  Bevy 0.19 gotchas: `TextFont.font_size` is `FontSize::Px(..)`; `GlobalAmbientLight` resource; `shadow_maps_enabled`;
  `CursorOptions` is its own component (`Single<&mut CursorOptions>`).
  Screenshot via `um win shot --exe ultrakrust.exe` (run `um win setup` once). Next: scripted input take + clip, then
  milestone 2 = Rust UnityFS/SerializedFile reader to load level geometry from the user's bundles.

## Milestone 2 — level 0-1 geometry from the install (2026-10-04)
- `crates/uk-assets`: UnityFS (LZ4/LZMA) → SerializedFile v22 → typetree-driven `Value` reader → Mesh/Texture2D/Material decoding → scene extraction.
- 0-1 = `campaign_scenes_level0-1.bundle` (9.5 MB): scene file `CAB-55b465a6…` (60,209 objects, 339 types) + `.sharedAssets` (39) + `.resS`/`.resource`.
  Materials/textures live in other bundles (`assets_assets_assets/materials.bundle`, `textures.bundle`), reached via `CAB-*` externals.
  531 MeshFilters point at `Library/unity default resources` (built-in Cube/Plane...) = `ULTRAKILL_Data/Resources/unity default resources`.
- Gotchas:
  1. `unity default resources` has **no typetrees** → borrow class typetrees from any loaded file of the same Unity version.
  2. 268 renderers are **static batched**: MeshFilter points at `Combined Mesh (root: StaticSceneOptimizer)` (world-space verts); draw submeshes `m_StaticBatchInfo.firstSubMesh..+subMeshCount` with identity (or StaticBatchRoot) transform.
  3. Most rooms are **inactive roots** (enabled by triggers as you play). Force root rooms on; skip roots containing `Alt`, `OLD`, `OutOfBounds` (unused alternates / kill volumes).
  4. Handedness: Unity→Bevy = negate z (positions, normals; quats → (-x,-y,z,w)). **Keep** triangle winding (mirroring already turns Unity's CW fronts into Bevy's CCW); reverse only for mirrored transforms (det < 0).
  5. Texture rows are bottom-up in Unity and UV v=0 is bottom; uploading rows as-is to wgpu with untouched UVs is already correct.
  6. Mesh formats seen: pos float32, normal float16×4, uv float16/float32; index u16; uncompressed; no stream data for level meshes. Textures: RGB24 (1488), RGBA32 (531), DXT1 (285), DXT5 (256), Alpha8, 2× BC7 (unsupported); mostly Point filtered, many streamed from .resS.
  7. Environment collision = non-trigger MeshCollider/BoxCollider on layers 0, 6, 8, 24 (LayerMaskDefaults LMD.Environment = 6|8|24).
- Results: extraction 1.6–2.3 s; 2440 renderers → 134 material batches (133 textured), 133k tris; collision 81,315 tris + 376 boxes; spawn = `FirstRoom/Player` (0,105,-253).
- Verified: Rust mesh decode == UnityPy (positions/indices/uv checksums on the 64k-vert combined mesh); headless test `uk-core/tests/level_0_1.rs`: V1 falls 105 u into FirstRoom, lands, walks into the closed door (box collider), turns, walks back, never tunnels.
  Visual: `ultrakrust --tour <dir>` saves spawn + per-room screenshots (27 for 0-1) — no input injection needed.
- Tooling note: `um win drive` failed (WinDrive.ps1 missing after `um win setup`); in-app tour used instead.
- Next: doors (first-room door blocks the exit — needs Door/ObjectActivator logic or a "doors open" option), lights/lightmaps, enemies.
