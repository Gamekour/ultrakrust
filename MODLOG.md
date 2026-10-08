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

## Milestone 3 — 0-1 playable (2026-10-04)
New crate `uk-game` (engine-agnostic runtime) + `uk-assets::scenedef` (full scene graph: 13,884 nodes, 5,708 renderers incl.
CPU-skinned enemies, 3,393 colliders/569 triggers, 11,758 MonoBehaviours with typetree fields; loads in ~3 s).
- Unity object model: activeSelf/activeInHierarchy, Awake/OnEnable/Start/OnDisable, Invoke timers, trigger enter/exit
  (incl. messages to the attached Rigidbody's GameObject — compound colliders like CheckPoint + child "Hitbox"),
  OnCollisionEnter-style contacts (DeathZone), UnityEvent persistent calls (SetActive, set_enabled, Activate, Break, Door.*, ...).
- Ported scripts: ObjectActivator(+ObjectActivationCheck), UltrakillEvent, Door (Normal type: all 33 in 0-1),
  DoorController, DoorOpener, ActivateArena, ActivateNextWave (enemy deaths -> nearest parent wave, linked waves),
  Breakable, Glass (shot / punch / windState sweep), CheckPoint (state snapshot; respawn at transform + up*1.25),
  DeathZone (instakill / damage + respawn target), OutOfBoundsTargetSetter, TeleportPlayer, PlayerActivator (intro: V1
  frozen while falling in), FinalDoor, FinalDoorOpener, FinalPit (level end), HudMessage.
- ClimbStep (player auto-step up to 2.1) — the rig had it, stairs need it.
- Enemies: Filth (speed 20/accel 30, swing range 3, bite = SwingCheck2.damage 30, cooldown 0.5 @0.4/s), Stray (speed 10,
  flee < 15, shoot 1-2.5 s, Projectile prefab: 65 u/s, 25 dmg, r 0.5), Malicious Face (25 HP; Standard: 6-shot bursts aimed
  at the head, 1 s cooldown; beam: 2 s charge, predicted aim, 50 dmg ignoring dash i-frames). Damage = m + m*limb*crit
  (head x2, limb x1.5), airborne zombies x1.5. Hitboxes = collider tags Head/Limb/EndLimb. Death zones kill enemies.
  Movement is direct pursuit (no navmesh yet). Health on blood: approximation (+3 hit, +10 head/kill within 9 u).
- Weapons: revolver from RevolverPickUp (the real unlock is save-progress driven), punch (Feedbacker, range 4, parries
  projectiles: reflected, full heal).
- Camera culling mask (0x8fd2dfd7) hides layers 3,5,13,16,18,19,21,28,29,30 — trigger volumes are layer 16 cubes.
- Gotchas: 1) trigger events go to the attached Rigidbody's object too; 2) DeathZone also fires OnCollisionEnter;
  3) DeathZone notInstakill => damage + teleport to respawnTarget (set by OutOfBoundsTargetSetter triggers);
  4) windState sweep must ignore the floor already touched (Unity SweepTest skips contacts) or glass floors break underfoot;
  5) MF burst spread only on difficulty >= 4; 6) GunRoom arena trigger appears 8.867 s after the pickup (title card).
- Verified: probes (`crates/uk-game/examples/probe_*`): all 14 arenas spawn waves and unlock their doors; 11/11 live
  DoorControllers open/close (5 decorative ones have no door); boss -> FinalDoor -> FinalPit = level complete; checkpoint
  respawn restores state. Autopilot (`bot_0_1`) walks from the start through ~70% of 0-1 on foot (planks, slide corridor,
  Gun Room, 4 Hallway, glass drop, 2 checkpoints, Glass Hallway, Fan Room incl. gap jump, Projectile Arena, Combo Hallway,
  door 7). `ultrakrust --demo <dir>`: scripted run to LEVEL COMPLETE with screenshots; recorded with `um win record`.
- Not done: navmesh pathing, animations (enemies are posed statically), sound/music, lighting/lightmaps, BC7 textures,
  style meter/ranks, other weapons, parry nuances.

## Parity pass — harness, NavMesh, winding, ULTRAKILL's own shaders (2026-10-04)
- Harness (`uk-harness`): checks + coverage + perf + determinism vs `parity/baseline.tsv`; `--full` all scenes, `--render`
  runs the game per level (GPU errors, renderer coverage, read-back frame stats, frame time). See PARITY.md.
- Scene load iterated a HashMap -> nondeterministic script/collider order. Fixed (node order).
- NavMeshData = Detour "DNAV" v16 tiles: header 72 B (counts: polys, verts, detail meshes, detail verts, detail tris,
  bv nodes), verts 12 B, polys 32 B (u16 verts[6], neis[6], u32 flags, u8 count, u8 area), detail mesh 12 B, detail verts
  12 B, detail tris 8 B (4 × u16), bv 16 B. All 6,811 tiles in the install match the size formula. 6-2's bake is stale.
- Winding: mirroring z (Unity -> Bevy) reverses the geometric face normal, so baked triangles must be reversed (kept for
  det<0 transforms). Skinned: inverse-transpose normals, per-triangle flip where the dominant bone matrix mirrors.
- Project: gamma color space, pixelLightCount 0, no AA, no shadows. ULTRAKILL/Master = one pass, LightMode=Vertex
  (legacy vertex lights: unity_LightPosition/Color/Atten/SpotDirection[8] in view space; ambient enters doubled).
- Shader blob (per platform, LZ4 chunks): chunk 0 = index (count, then offset/length/chunk per entry). Program entry:
  version 202012090, gpu type (25 = SPIR-V), 4 stats, keywords, code (u32 flags + 6 stage slots of SMOL-V), then one
  u32 + bind channels (ShaderChannel -> attribute location = target - 13). Parameter entry: cbuffers (first is an
  unnamed empty block) with members (name, type, rows, cols, is_matrix, array, offset), then bindings (name, kind
  0 tex/1 cb/4 sampler, packed = stage mask (0x04 VS, 0x08 PS) << 24 | set << 16 | binding).
- The player subprogram list interleaves platforms (m_GpuProgramType 6 Metal?, 15 D3D11 VS, 25 Vulkan; fragment list is
  D3D11 PS only = 17). On Vulkan one vertex-variant entry holds both linked stages.
- Unity strips DXBC RDEF; parameter lists name only referenced members. StandardProperties: @0 _MainTex_ST (deduced),
  @16 _Color (D3D11 lists), @32 _TextureWarping (deduced), @36 _VertexWarping, @40 _VertexWarpScale, @44 _HeightFog,
  @48/52 unity_FogStart/End, @64 _ScreenRatio = (w,h)/max(w,h).
- Master's fragment writes 3 targets: color, vec2, packed normal (n*0.5+0.5, for outlines). UVs are uv*w / w (PSX affine
  warp controlled by _TextureWarping). Final color = lerp(fogColor, texture * vertexLight, fogFactor).
- naga: rejects combined image-samplers (split pass in uk_assets::spirv), Unity stage pairs can leave fragment inputs
  unwritten (vertex outputs added as zero), `_CausticVolumeData` is a read-only storage buffer (bound zeroed).
- Bevy 0.19: custom Core3d system + own passes; pipeline layout = Vec<BindGroupLayoutDescriptor>; ViewTarget
  get_color_attachment() clears on first use; wgpu Color/types from wgpu-types 29.0.4.

## Mecanim runtime + animated enemies (2026-10-06)
- `uk_game::anim`: per-Animator rigs (bindings resolved by CRC32 of the transform path, as Unity's `m_TOS`), layers
  (override/weight; additive still skipped), state machines with exit-time / condition transitions, ANY-state,
  trigger consumption, cross-fades (normalized durations), 1D/2D/direct blend trees, write-defaults to the rest pose,
  clip events -> `GameEvent::AnimEvent`. Controllers inactive with `keepAnimatorStateOnDisable` off reset to defaults.
- Skinned meshes re-skin from the animated bones every frame (Unity renderer).
- Zombie feed (Zombie.Update): `Running` = agent speed > 0.1, `RunSpeed` = speed / max (Filth 20, Stray 10),
  `Falling` / `StartFalling` only after 0.15 s airborne (the NavMeshAgent keeps a walking zombie on the mesh; raw
  ground flicker on steps would otherwise cut the Attack state).
- Attack timing from the clips, as ZombieMelee / ZombieProjectiles: Filth Bite (1.15 s, speed 0.85) StopTracking 0.22,
  DamageStart 0.34, DamageEnd 0.51, SwingEnd 0.97; Stray ThrowProjectile (2.08 s, speed 1.25) SpawnProjectile 0.31,
  ThrowProjectile 0.64, SwingEnd 0.95. Harness: `animrt.0-1.filth_attack` / `stray_attack`.
- Cost: ~2.3 us per animated rig (all 110 rigs of 0-1 on: 0.25 ms).
- Bot route 0-1: the projectile-arena climb was a lucky wall-jump that animated (slower-dying) Strays
  broke; now explicit waypoints under the balcony -> balcony -> arena stair, plus a hallway corner
  before door 7. Holding at a waypoint gives up after hold+6 s with no targetable enemy; standing on a
  mover (lift/door group) doesn't count as stuck. probe_floor maps floor heights / surface columns.

## Viewmodel: ULTRAKILL's prefabs through the HUD Camera (2026-10-06)
- `uk_assets::addressables`: catalog.json reader (keys -> bundle + internal id); `scenedef::spawn_viewmodel` instantiates
  GunSetter.revolverPierce and FistControl.blueArm into the scene (prefab objects keep `ScriptDef.file`).
- HUD Camera (child of Main Camera, fov 90, culling mask layer 13, depth-only clear): layer-13 draws render in a second
  pass (color Load, depth Clear) with the HUD Camera's view, frustum culling skipped.
- Visibility as the scripts set it: HookArm.Start hides `model` (shows only while the hook is out); GunControl shows the
  revolver once picked up; Arm Blue/Arm2 is off in the prefab.
- Animator feed: Revolver.Shoot -> `RandomChance` roll + `Shoot` (Shoot / Shoot2), charged beam -> `ChargeShoot` (Shoot3);
  Punch -> `PunchRandomizer` roll + `Punch` (Jab / Jab2). Deterministic xorshift (`s.vm_rng`) stands in for UnityEngine.Random.
- The Feedbacker's Idle pose is below the HUD frustum (0 vertices on screen) as in ULTRAKILL; punching brings
  52-77% of its vertices on screen. Probe: `UNITY_FRAME_STATS` logs `unity hud series` every 10 frames 200..400;
  `UK_PROBE_PUNCH=1` punches on cooldown. Harness: `render.<level>.viewmodel_on_screen_max_pct`.
- Arm Blue's controller carries curves for two rigs: Feedbacker (`Armature/UpperArm/...`) and the disabled Arm2
  (`Armature/Upper Arm/...`). One Animator binds against one root, so the 51 Arm2 slots stay inert, as in Unity
  (`animrt.slots_bound_pct` 90.96 -> 90.27). `examples/unbound.rs` resolves unbound hashes against every node-path suffix.
- Prefab lookups cache the catalog and each bundle's m_Container (was ~265 ms per lookup; scene load back to baseline).
- Re-blessed: 10 authored viewmodel triangles face against their normals (det > 0, winding as stored).

## Playability fixes: first room, HUD clearing, black window (2026-10-06)
- **First room void:** ULTRAKILL activates each level's first room through `OnLevelStart.onStart` (toActivateObjects /
  onActivate...), fired when the level timer starts (= PlayerActivator sets `player.activated`). Ported as
  `Script::OnLevelStart`, run once by `Game::level_start_update`. `hideFogUntilStart` / `levelNameOnStart` not applied (Partial).
  Probe: `cargo run -p uk-game --example spawn_room -- <level>`. Harness `--only first_rooms`: every campaign level
  activates every onStart target and the player walks into it (`first_rooms.levels_ok` 41/41).
- **Text piling up / black window until resize (Unity renderer):** the order-1 VIEW_LAYER child camera (MSAA, own
  main texture) was drawn over the Unity composite and its UI target was never cleared, so old HUD text accumulated
  and the scene stayed black until a resize recreated the textures. Unity mode no longer spawns it; MainCam is
  `IsDefaultUiCamera`.
- Developer overlay (top-left debug text) is hidden by default, toggled with **F3**.
- Probe: `UK_PROBE_PRESENT=1` reduces swapchain captures to numbers (lit fraction, near-white ink in the hint band and
  debug corner) while forcing the hint on for frames 160-199. 0-1: hint ink 0 -> 2445 -> 0, corner ink stable, lit
  fraction 0.72 by frame 120 with no resize (frames 3-90 dark/flickering while pipelines compile).
  Harness: `pass.render.<level>.hint_clears`, `render.<level>.present_lit_frac`.

## Skybox, --all-weapons, exit elevator -> next level, R in the elevator (2026-10-06)
- **Skybox:** `RenderSettings.m_SkyboxMaterial` is drawn on a sphere around the camera, first, depth test off, when the
  player's Main Camera (culling mask 0x8fd2dfd7) clears to skybox (flags 1); otherwise the clear color is that camera's
  `m_BackGroundColor`. Cubemaps decode all 6 faces (`m_ImageCount`); `_*_HDR` defaults to (1,1,0,0); Procedural skies take
  `_WorldSpaceLightPos0`/`_LightColor0` from the brightest active directional light. 4-2 was black: its sky is a cubemap.
  Probe: `UNITY_SKY_ONLY=1` draws the sky alone; `examples/skybox.rs` dumps every level's sky material.
- **`--all-weapons`:** unlocks every ported weapon (Revolver only so far), rebased into the level-start snapshot so restarts keep it,
  carried through level changes.
- **Exit:** `FinalPit` (levelOver, centering, view turn, SendInfo -> results) and `TeleportFinalPit` (+20 forward +20 up into the
  second pit's shaft, which closed 0-1's endless fall) are Full. The FinalRank tally runs a line per 0.5 s (Fire1 skips);
  Fire1 once complete in the second pit loads `targetLevelName` (SceneHelper.LoadScene), with a LOADING card and the Unity
  renderer rebuilt per generation. Style rank is 0 (no style meter yet).
- **R in the elevator:** `respawn` is a no-op once `level_complete` (NewMovement.levelOver), in the sim and the frontend.
- Harness: `0-1.exit_ignores_restart`, `0-1.exit_to_next_level` (results 0.248 s, second pit 1.448 s, axis dist 0, view err 0).
  Frontend probe: `UK_PROBE_EXIT=1` drops into the live first pit, `UK_PROBE_CONTINUE=1` holds Fire1; `UK_PROBE_PRESENT`
  samples `generationN +30/+120/+240/+600` after a level change. 0-1 -> 0-2 lands at mean_lum 0.214 / lit 0.98 by +600
  (direct 0-2 load: 0.227 / 1.0).
- Sky sweep (`UNITY_SKY_ONLY=1 UK_PROBE_PRESENT=1`, all 43 campaign scenes): every level with a skybox material draws it;
  levels whose Main Camera clears to solid color (0-1, 3-2, 4-4, 5-1, 5-4, p-1) or have no sky material stay their clear color.
  Dark results trace to the data: 0-5 "BlackNoFog", 7-3 night texture (mean 1.6/255), 7-2 tint 0.113, and 1-3/1-4 where
  no level sets `m_Sun` and the brightest active directional light (Unity's fallback) is below the horizon
  (`examples/suns.rs`). 4-2 GreedSky2: mean_lum 0.60, lit 1.0 (was black).

## PostProcessV2 outlines (2026-10-07)
- The main camera renders color + an RG8 outline buffer (Unity RG16, cleared black). Master's `rtBlend1` state is read
  per pass (`rtSeparateBlend`); Min/Max ops get One factors (WebGPU requires them, D3D/Vulkan ignore them).
- `EnemySimplifier` (enabled, on the renderer's own GameObject) gives the renderer a property block: `_Outline` = prefs
  simplifyEnemies (default 0), `_ForceOutline` 0.5 (both 0 with `neverOutlineAndRemoveSimplifier`), `_BlendOp1` 0,
  `_SrcBlend1` 1, `_DstBlend1` 0, `_ForceOutlineBehind` 0.
- OutlinePx pass 3 "Composite1Px" (blend DstColor Zero) runs after the main camera, before the HUD Camera: a pixel
  that is unmarked but has a marked neighbour (R > 0.999 or R + G > 1) turns black.
- Probes: `UNITY_PROBE_ENEMY=1` activates the enemy nearest the player (and its ancestors) and frames it from 4 m;
  `UNITY_SIMPLIFY_ENEMIES=1` sets `_Outline` 1; `UNITY_NO_OUTLINE=1` skips the composite; `examples/outline_px.rs`
  dumps the shader's passes and blend states. 0-1 Filth: default prefs write 0.63% of the buffer, all R = 0.5, 0 marked
  (no visible outline, as in ULTRAKILL). With simplifyEnemies: 0.64% marked, 533 outline pixels predicted, 533 black.
- Harness: `pass.render.<level>.outline_composite`, `render.<level>.outline_buffer_pct`.

## PostProcessV2 hurt flash + DEAD (2026-10-07)
- PostProcessV2's default pass lerps the scene toward `_HurtScreenColor`.rgb by its alpha. NewMovement.GetHurt sets
  `currentColor.a` to 0.8 (damage >= 50) or 0.5 after its early returns, and Update fades it by dt, dead or alive; the
  RGB is NewMovement.hurtScreen's Image `m_Color` (1, 0.186, 0), read from the scene. Sim: `State::hurt_alpha`.
- DeathSequence enables the DEAD keyword and drives `_Sharpness` = `_Deathness` = t * 0.5 for 2 s; `_ChromaticAberration`
  is never set (0). The DEAD variant gets its own pipeline and constant buffers (`_Time`, `_ScreenParams` filled) and
  replaces the default pass while `State::dead`. (The 1.5 s auto-respawn gap here was closed by the
  death screen entry below; deathness now reaches 1.)
- Heat-wave: `HeatWaves()` has no caller. ColorSchemeSetter is in no campaign scene.
- Probe: `UNITY_PROBE_HURT=frame:damage`; logs `unity post globals` and `unity post shift` (mean post - scene).
  `examples/post_vsrm.rs` dumps the shader's keyword variants and constant buffers, `examples/hurt_screen.rs` the color.
- 0-1: baseline shift (-3.9, -3.8, -4.3). 30 damage at frame 232: alpha 0.433 at frame 240, shift (93.8, 11.1, -6.4) vs
  predicted (94.0, 11.2, -7.1). 200 damage at frame 200: alpha 0.470, deathness 0.165 (0.33 s), DEAD pass on.

## PostProcessV2 keyword variants: UNDERWATER, VIGNETTE, WICKED (2026-10-07)
- All 16 combinations of PostProcessV2's runtime keywords (DEAD, UNDERWATER, VIGNETTE, WICKED) are built as separate
  pipelines. The frame picks the variant by mask and falls back to mask 0 if one is missing. This replaces the DEAD-only
  pipeline.
- Textures are bound by slot name instead of putting the dither texture in every non-scene slot:
  - from the material's TexEnvs: `_NoiseTex` 256x256 (DualNoise)
  - from the handler's ditherTexture: `_Dither` 16x16
  - from the handler's vignetteTexture: `_VignetteTex` 512x512
  - names with no definition get white
- With a real `_NoiseTex`, the 0-1 baseline shift is now (-2.3, -2.6, -4.0); it was (-3.9, -3.8, -4.3) with the
  dither texture in that slot.
- New constant-buffer inputs: `_UnderwaterOverlay`, `_VignetteColor` and `_RandomNoiseStrength`. Sim state:
  `underwater_overlay`, `vignette` and `screen_noise`.
- WICKED is a port of ScreenDistortionField and ScreenDistortionController:
  - For each enabled field, d = distance from the player to the closest point on its first collider (shape in load pose,
    following the owning mover); with no collider, its position.
  - strength = ((distance - d) / distance)^2 * strength when d < distance.
  - The keyword is on while any field is enabled; the noise value is the maximum strength.
  - Fields: 0-S Wicked's RadiationField (70 m, 0.33), and the two 1-2 Cancerous Rodent Radiation fields (30 m, 0.33).
- Gaps: nothing in the sim sets UNDERWATER (Water + UnderwaterController, 36 scenes) or VIGNETTE
  (PowerUpMeter/DualWield juice) yet.
- Probes:
  - `UNITY_PROBE_POST=underwater,vignette,wicked` forces the inputs.
  - `UNITY_PROBE_DISTORT=m` holds the player m metres along +X from the first field and activates its ancestors.
- 0-1 results at frame 240 (exact variant every time), shift = mean rgb post - scene:
  - UNDERWATER (0, .5, 1, .3): shift (-11.6, 35.9, 74.5), predicted about (-11, 32, 71).
  - VIGNETTE (1, .6, 0, 1): shift (2.2, 0.2, -4.1), edge-only.
  - WICKED 1.0: mean abs diff 16.9, 1544 distinct colors (baseline 55).
  - All three: 3836 distinct colors.
- Distortion results (frame 240, WICKED on, exact variant), as predicted ((distance - d) / distance)^2 * 0.33:
  - 1-2 rodent, player 10 m from the field (sphere radius 7.5, so d = 2.5): noise 0.27729, predicted 0.27729.
  - 1-2 rodent, 40 m: noise 0.0. WICKED stays on because the field is enabled.
  - 0-S Wicked, 20 m (radius 3.75, so d = 16.25): noise 0.19457, predicted 0.19457.

## Graphics prefs: pixelization, color compression, dithering, gamma, warping, FOV (2026-10-07)

- New `uk_assets::prefs`: reads `<install>/Preferences/Prefs.json` + `LocalPrefs.json` (never written).
  - Ports GraphicsSettings' GetPixelizationValue, GetColorCompressionValue and GetVertexWarpingValue.
  - `UNITY_PREFS="k=v,..."` overrides values in memory for probes.
- `GraphicsPrefs` feeds the globals:
  - `_ResY`, `_ColorPrecision`, `_DitherStrength`, `_Gamma`, `_VertexWarping`, `_TextureWarping` (clamp01 × 0.5)
    and `_StainWarping`.
  - The VERTEX_WARPING keyword is on when vertex warping is nonzero.
- Pixelization follows PostProcessV2_Handler.SetupRTs:
  - The scene, depth and outline targets are (w/min, h/min) × resY, point filtered.
  - PostProcessV2 writes a screen-size target with `_VirtualRes`, `_ScreenRatio`, outline `_Resolution` and
    `_ResolutionDiff` from that.
- CameraController.Start: `fieldOfView` and `cameraTilt` prefs reach the sim camera, kept across reloads.
- Gaps:
  - colorPalette / PALETTIZE (off in the user's prefs).
  - mouseSensitivity's scale (Input System delta units).
  - outlineThickness.
- Probes, 1-2 at frame 240 (1600×900 window):
  - User prefs: precision 2048, post equals scene (diff 0.00), 240 red levels.
  - colorCompression=2, dithering=0.2: 32 red levels, diff 2.27.
  - colorCompression=5: precision 3, exactly 4 red levels (0, 1/3, 2/3, 1).
  - pixelization=4: scene 426×240 (1600/900 × 240), post 1600×900. 93% of post pixels equal their left neighbour;
    post vs the covering scene pixel diff 0.33.
  - pixelization=6, colorCompression=2: scene 64×36, 32 red levels.
  - pixelization=1: scene 1280×720 with mean rgb (177.3, 80.0, 51.7) vs (177.2, 79.9, 51.7) native.
  - The darker pixelization=4 frame was timing: at 143 fps, frame 240 is 2.16 s in, with the camera still dropping
    (y 13.4 vs 1.9). The camera log now prints shader time.
  - `camera prefs: fov 105 tilt true` from the user's Prefs.json.

## Water + UnderwaterController (2026-10-07)

- `Game` gathers every `Water` script: its child colliders, `clr` (default (0, 0.5, 1, 1)), and `visualsOnly`.
- The UnderwaterController's CameraCollisionChecker sphere (r 0.35) is stored relative to the player node.
  - `water_tracking` runs after `update_triggers`.
  - Waters whose live colliders contain the sphere are kept in entry order, as UnderwaterController.touchingWaters.
  - Entering one sets `_UnderwaterOverlay` to its `clr` with a = 0.3.
  - If `clr` is all zero, it uses the overlay Image's `m_Color` with a = 0.3 instead (read from the scene:
    (0, 0.497, 1)).
  - With no waters left, UNDERWATER is off.
- Waters (not visualsOnly) whose colliders contain the player capsule set `Player::touching_waters`.
  - Each of them applies ApplyWaterForces before the physics step.
  - Below −8 m/s, vy eases toward −8 by fixedDt·10·|vy+7.5|. Otherwise −0.75·g·mass.
- Probes (`UNITY_PROBE_WATER`, `UNITY_PROBE_WATER_DROP=<box path>`; the drop log also names the solid surface
  under the player):
  - 5-1 "2B - Arena B/B Nonstuff/Water/Cube" (top −98):
    - Free fall vy −18.4 entering. Drag settles at about −12.5; the extra difference over plain gravity is
      NewMovement's +0.4 g fall force.
    - The overlay is on at y −99.55 = (0, 0.5, 1, 0.3), and stays on at the floor (−106).
  - 2-1 "0-1 Connector/Water": the player lands on Secret/Cube (61) at −20.75, which is solid. The capsule touches
    2 Waters while the eye stays above them, so the overlay is None.
  - 1-2 lava: Decorations/Cube (12) at −19.76 sits above the lava trigger (top −20), so the player never touches it.
- Gaps:
  - DryZoneController.
  - Wetness and notWet.
  - Splash and audio.
  - Forces on non-player rigidbodies (layers 9/10 get −0.45).

## PowerUpMeter + DualWieldPickup + DisablePowerUp (2026-10-07)

- `State` carries the PowerUpMeter singleton: `power_juice`, `power_max` (latestMaxJuice), `has_power_up`,
  and the live DualWield count.
- DualWieldPickup (trigger enter) runs PickedUp and DualWield.Start at once:
  - It deactivates itself unless `infiniteUses`.
  - juiceAmount 0 becomes 30. If juice < amount, juice = latestMaxJuice = amount.
- PowerUpMeter.UpdateMeter runs every frame, dead or alive:
  - While juice > 0, juice falls by dt and `_VignetteColor` = (1, 0.6, 0, juice/max), which turns VIGNETTE on.
  - Otherwise EndPowerUp: the vignette goes off, and every DualWield sees juice ≤ 0 and is destroyed.
- Juice is zeroed by NewMovement death, by respawn (the meter itself is carried over the checkpoint
  snapshot) and by entering a FinalPit. DisablePowerUp.Start ends a running power-up (5-3).
- `examples/find_class.rs` lists every instance of the given classes across all scene bundles (path, enabled,
  fields). It was used to find the 11 pickups in 8 scenes.
- Probe `UNITY_PROBE_POWERUP=<path>` (4-2 "6A Stuff/DualWieldPowerup", juiceAmount 30):
  - Juice + t stays constant at about 31.3 (picked up at t ≈ 1.3); alpha 0.984 → 0.779 over t 1.78 → 7.93.
  - With `UNITY_PROBE_HURT=600:200`: juice is 0 on the death frame, and the next update turns VIGNETTE off
    (0 DualWields). It stays off after the respawn.
  - Full run with HP held at 100: alpha 0.902 (t 4.19) → 0.0089 (t 30.99). At t 31.50 it reads juice 0, max 0,
    VIGNETTE off, 0 DualWields, and it stays off.
- Gaps:
  - The duplicated weapon (DualWield's copy firing with `delay` = 0.05 + n/20, offset ±1.5).
  - The meter UI and endEffect.
  - The pickup effect and camera shake.

## Death screen: DeathSequence + BlackScreen + StatsManager restart (2026-10-07)

- There is no auto-respawn anymore. Death leaves the player dead until StatsManager.Update's restart input:
  R, or Fire1 (LMB while the cursor is captured), while `hp <= 0`, at any point of the sequence.
  - `Game::death_restart` reloads the level without a checkpoint (`restart_level`), else runs CheckPoint.OnRespawn.
  - The bot presses restart 1.5 s after dying (`BotFrame::restart`).
  - R while alive stays as a developer shortcut for the pause menu's Restart Checkpoint.
- `DeathUi::from_def` reads the scene's player canvas:
  - DeathSequence's TextAppearByLines `delay` (0.05) and its TMP text (39 lines; `<color=orange>` lines flagged).
  - `deathScreen`: the BlackScreen Image color (0.05, 0.05, 0.05, 1).
  - YouDiedText: m_Text, overridden by TextOverride's m_KeyboardText "[YOU ARE DEAD] … Press [R] TO RESTART", white.
  - Sibling order: DeathSequence is after BlackScreen, so the log draws over it.
- DeathSequence.timeSinceDeath is `dead_timer`. The log shows min(floor(t/delay)+1, 39) lines.
  At t ≥ 2, EndSequence sets `death_screen`: the BlackScreen and YouDiedText.
  Respawn (OnDisable) clears both.
- Bevy UI layout: CanvasScaler ScaleWithScreenSize 1280×720, match 0.5, so scale = sqrt(w/1280 · h/720).
  - The log is TMP fontSize 16 (top-left); YouDiedText is size 14 (centered); TMP "orange" is (255, 128, 0).
  - NewMovement's screenHud is hidden while dead. The "YOU DIED" banner is gone.
- Probe `UNITY_PROBE_HURT=60:200 UNITY_PROBE_DEATH=600` on 0-1 (1600×900, scale 1.25):
  - t 0.616: 12 spans (6 orange). t 1.282: 26. t 1.949: 39, last "I DON'T WANT TO DIE.", 20 px.
  - The BlackScreen shows from t 2.016, with the log at z 11 over it and YouDiedText at 17.5 px. The HUD is hidden throughout.
  - Restart at frame 600: alive, hp 100, at the level start (no checkpoint yet). The canvas is cleared and the HUD is back.
- Harness: 0-1.checkpoint_respawn now idles 300 ticks dead. It asserts the BlackScreen is waiting (no auto-respawn),
  then restarts from the checkpoint. It passes.
- `examples/find_class.rs`: FIND_SUBTREE, class `*` with FIND_TEXT, FIND_FULL.
- Gaps:
  - The LaughingSkull and ISeeYou sprites and the red Flash (HudOpenEffect): there is no uGUI sprite path yet.
  - Death audio and the pitch drop.
  - The rb torque roll.
  - The scene reload is our `restart_level` (it respawns at the level start; it does not reload the scene).

## 2026-10-07 — uGUI interpreter, part 1 (layout + meshing)
- `uk-assets/src/ui.rs`: the UI assets a scene's scripts reference (sprites with outer/inner UV, border, padding,
  textures with wrap mode, TMP fonts and atlases, legacy fonts, materials, and the built-in UI/Default material).
- `uk-game/src/ugui.rs`: a pure (bevy_math only) uGUI pass over the scene's real Canvas hierarchies. These are literal ports
  of the decompiled UnityEngine.UI:
  - CanvasScaler.Handle (all 3 modes and the world-space dynamic PPU) and RectTransform layout in root-canvas space.
  - GetPixelAdjustedRect on pixel-perfect screen canvases.
  - Graphic, Image and RawImage OnPopulateMesh: Simple with PreserveAspect, Sliced, Tiled (incl. the 65000 vertex cap), and
    Filled (Horizontal, Vertical, Radial90/180/360 with RadialCut).
  - Shadow and Outline (ApplyShadowZeroAlloc).
  - Mask and MaskableGraphic stencil materials (StencilMaterial.Add, push and pop).
  - RectMask2D clip and cull, CanvasGroup alpha, the CanvasRenderer colour, and override-sorting sub-batches.
  - Output: per-canvas batches of draws (vertices, material, texture, stencil, clip rect).
- `examples/ugui_frame.rs` (level0-1, 1920×1080, static scene activation):
  - 11 canvases, 3140 graphics, 38 Masks, 1 RectMask2D. Frame built in 1.7 ms.
  - Player/Canvas: overlay, order 40, scale 1.5 (1280×720 match 0.5), 28 draws.
  - Level Stats panel at px [15, 592.5]-[442.5, 1065], sliced 36-vertex quads.
- Not yet:
  - Text and TMP meshes.
  - Slider/layout groups (Slider fills are zero-width until Slider sets their anchors).
  - HUD scripts (StatsManager hiding FinalRank, StyleHUD, HudOpenEffect...); the probe shows the scene's initial state.
- uGUI renderer path (`unity_render.rs` `prepare_ui`/`draw_ui`):
  - Every draw is rebuilt per frame: vertex streams packed in the UI shader variant's input order, `_MainTex` = the Graphic's texture (else white), the material's other textures, uniform buffers with `_ClipRect`/softness, the canvas matrix and the camera.
  - Overlay canvases use a pixel-space projection and are drawn onto the post output with their own depth/stencil. World canvases are drawn in their camera's pass (layer 13 → HUD Camera).
  - Mask stencil state per draw (`set_stencil_reference`), with pipelines keyed by (variant, state, mrt).
  - Probe (0-1, 1600×900):
    - Player/Canvas: 28 draws, 256/256 vertices in view.
    - StyleCanvas (HUD Camera): 13 draws, 292/292 vertices in view, ndc x −0.09..0.13.
    - The overlay changes 9.74% of the post output's pixels (bounds [12,12]..[368,405]).
    - The 24 skipped draws are all empty meshes (text not meshed yet). Every material and variant resolves.
  - A zero-sized Texture2D now fails to decode instead of panicking.

- TextMeshProUGUI meshing (`crates/uk-game/src/tmp.rs`, wired through `ugui.rs` and `prepare_ui`):
  - Literal port of PopulateTextProcessingArray, SetArraySizes, ValidateHtmlTag (the tags the scenes use), GenerateTextMesh (wrap, auto size, Truncate, alignment, uv2) with InsertNewLine / Save / RestoreWordWrappingState, TMP_MaterialManager fallback materials and ShaderUtilities ratios / padding.
  - Each text's mesh is cached on (text, rect, uv2 scale, color); runtime materials (ratios applied, fallback copies) live in `UiDef::tmp_mats` and reach the shader through `UiDraw::props`. TMP draws use tangent (−1,0,0,1).
  - TMP's stencil skips MaskableGraphic's own-Mask check; sub meshes add the text's Mask to their depth and draw after the main mesh in reverse order; Cull tests the compound mesh bounds.
  - `ugui_frame` (0-1, 1600×900): 65 texts in 4-2 build in 5.8 ms (first frame, cached after). VCR OSD Mono advances 14.01 at 24 pt in every label (TIME:/KILLS:/SECRETS: widths 63.34/77.36/105.38); centered titles symmetric (±222.52); "GET 5 KILLS WITH A SINGLE GLASS PANEL" wraps into 3 lines in 316 units. Hash checks: orange 26556144, color 281955, /color 1071884.
  - Probe (0-1): overlay gpu draws 8 → 24, HUD 9 → 12; skipped empty meshes 24 → 5; no missing material or variant.

- Slider + Selectable (ugui.rs):
  - SliderDef from any script with m_FillRect/m_HandleRect/m_Direction/m_WholeNumbers (UpdateCachedReferences rules); `UiState::slider` values, `UiState::set_slider` = Set (clamp, whole numbers) + UpdateVisuals writing the driven anchors / Filled fillAmount, also while inactive.
  - Enabled sliders drive their fill/handle anchors every layout (OnEnable -> UpdateVisuals).
  - SelectableDef from any script with m_Transition/m_Colors/m_TargetGraphic: enabled ColorTint selectables set their target's CanvasRenderer colour to normal or disabled (m_Interactable and ParentGroupAllowsInteraction, CanvasGroup.interactable) times the multiplier. No pointer/selection states yet (no EventSystem).
  - `ugui_frame` (0-1): style meter fill at value 0 is the 10x25 sizeDelta nub (was zero-height); freshness fills Dull 442 wide (value 1), others 0.

- HUD scripts (`crates/uk-game/src/hud.rs`, `Script::Hud`):
  - PlayerActivatorRelay: PlayerActivator (not onlyActivatePlayer) runs ResetIndex + Activate, one `toActivate` entry per `delay` (Act::HudRelay). The gun panel needs weapons and weaponIcons; the crosshair always shows. GunControl.Start hides `gunPanel` without weapons; the revolver pickup runs UpdateWeaponIcon.
  - HudOpenEffect: Awake Initialize, OnEnable ResetValues (scale 0.05 or sizeDelta), Update's MoveTowards (|t−v|+0.1)·speed, x then y (or YFirst), closes into SetActive(false) at 0,0; Reverse.
  - HealthBar, StaminaMeter: literal Updates over `UiState` sliders, Graphic colours and TMP text. `hud::net_fixed` formats "F0" and "0.00" like Mono (7 significant digits, then half away from zero).
  - ColorBlindSettings colours with ColorBlindActivator.Start's ColorBlindSetter prefs (`hudColor.<name>.r/g/b`); ColorBlindGet on Start/OnEnable.
  - HudController.Start: CheckSituation (hudType 1 = GunCanvas enabled at z 1, else z −100 and disabled; alt HUDs by hudType 2/3), weaponIcons/armIcons/styleMeter/styleInfo, hudBackgroundOpacity. HUDPos.CheckPos with weaponHoldPosition.
  - NewMovement: screenHud off at death, on at respawn; HUD/hudCam velocity sway (`UiState::local`, `Game::node_world`); the renderer takes the HUD Camera from `UnityFrame::hud_cam` each frame.
  - Game gets `prefs` (the frontend sets them before `set_ui`). set_ui replays the HUD scripts' Awake/OnEnable/Start, then retakes the start snapshot.
  - `hud_probe` (0-1):
    - The player-only activator fires at landing (t 2.28). The ObjectActivator's PlayerActivator follows 0.208 s later, and the relay then takes 0.2 s per entry: StatsPanel, Panel (2), Panel (3), GunPanel (skipped, no weapon), RailcannonChargePanel, Image, Crosshair Filler, SpeedometerPanel.
    - StatsPanel opens 0.05 → 4 in under 0.2 s.
    - HP counts up 0 → 63 → 87 → 96 → 99 → 100 over 1.2 s, text "63"…"100".
    - Stamina ramps to 300 and each slider flashes when it fills.
    - Hurt 75: hp 25 at once, after-image 51.4 → 33.9 → 27.6 → 25.3. The classic text stays white (changeTextColor 0, normalTextColor clear).
    - Boost −100: slider 3 at 215.6, alpha 0.6 charging colour, then a full flash.
    - Revolver: GunPanel on.
    - Death/respawn toggle screenHud.
  - Save data, read-only: `nrbf.rs` decodes BinaryFormatter streams (classes with/without member types, binary/primitive/object arrays, references, null runs); `save.rs` finds `Saves/Slot<selectedSaveSlot+1>`, GetRank(returnNull) and GameProgressMoneyAndGear, and reads Unity PlayerPrefs (`HKCU\Software\Hakita\ULTRAKILL`, `key_h<hash>`, DWORD/QWORD/BINARY) via `reg query`. `Game::save`; PlayerPrefs.Set* only changes the in-memory copy.
  - LevelStatsEnabler: Start hides the controller without a rank for StatsManager.levelNumber (or secret mission < 2) and sets LevStaOpe 0; LevelStatsTutorial after 1.5 s when LevStaTut is 0 (SendHudMessage); the panel child follows LevStaOpe. Update: Tab hold shows, release hides, a second press within 0.5 s keeps it open (LevStaOpe 1), the next press closes. `Game::hud_input` carries the Stats action (frontend: Tab just_pressed / just_released).
  - hud_probe on 0-1 with the user's save: levelNumber 1, rank 1, LevStaOpe 0, so the panel starts hidden and no Level Stats draws are produced. Tab down shows it and up hides it; a double tap sets LevStaOpe 1 and keeps it open; the next press closes it (LevStaOpe 0).
  - StatsManager run state (`Game::s.stats`): seconds while the timer runs (StatsManager.Update, TimerModifier 1), StartTimer / StopTimer (FinalPit), checkpoint restarts keep it and restart the timer, a scene restart resets it; prevSecrets from the save's RankData.secretsFound (Awake).
  - Level start is the game's own chain now, replacing the stand-in that fired `onStart` when the player was activated: FinalDoorOpener.GoTime (from Awake without a FinalDoor, from its 1 s Invoke, or called by a UnityEvent: the FirstRoom's `Cube (1)`) and PlayerActivator.startTimer → StatsManager.StartTimer → PlayerTracker.LevelStart → OnLevelStart.StartLevel (onStart, timer, DisableOnLevelStart).
    - 0-1 starts at its title: TitleActivator's 8.867 s delay after the revolver activates the Gun Room trigger's FinalDoorOpener, 8.64 s after the pickup in `hud_probe`.
    - Every other level starts when the player walks into the FirstRoom's `Cube (1)` (tick 623 in `level_start`).
  - GetPlayerPref.Awake (DisCha PlayerPrefs, ShoUseTut = hideShotgunPopup, MainMenuEncorePopUp, weapon.* with fallback 1). The prefs now exist before Awake: `Game::with_prefs(def, prefs, save)`. 0-2's ShopTutorialDoor goes away once the shop tutorial is done.
  - Harness `first_rooms` is rewritten for the real chain. The bot lands, steers into the FirstRoom trigger (0-1: the revolver pickup) and requires the level started, the timer counting and onStart's objects active. Prefs model a returning player (hideShotgunPopup). Result: 41/41.
  - LevelStats: Start (rank gating like the enabler, StockMapInfo LargeText, extra secret icons hidden) and CheckStats every frame. `hud::stats_time_text` reproduces `minutes + ":" + seconds.ToString("00.000")`, including "10:60.000" when the rounding carries. `rank_text` is GetRanks' rich-text letters.
  - `hud_probe` with the double-tapped panel open: "0-1: INTO THE FIRE", time "0:00.000" (8 quads), time rank S, kills 0 D, style 0 D, five secrets on the filledSecret sprite (the save has all five), challenge NO, assists NO.

## 2026-10-08 — HudMessage, TMP_Settings + dynamic fonts, uGUI auto layout, HUD placement
- HudMessageReceiver / HudMessage (`hudmsg.rs`): SendHudMessage / SendHudMessage2 (string.Format), ShowHudMessage, the ShowText typewriter (skipLineBreaks, writing cursor), automatic 5 s timer, Done / ClearMessage / ForceEnable, the PlayerPref gates (SecMisTut / ShoUseTut aliases). Action key names come from the InputActions asset plus the player's `Preferences/Binds.json` (`uk-assets/src/input.rs`, read-only). `msg_probe` shows the 0-1 revolver hint typed out and the box hiding 5.0 s after a timed send.
- TMP_Settings is read from resources.assets without a typetree (fields in serialized order): missing glyph, default font, global fallbacks. Dynamic TMP font assets render missing glyphs at runtime (`sdf.rs`: skrifa hinted outlines, exact-distance SDF over the padding, MaxRects packing); the renderer uploads the atlas when its version changes. The SDF spread is calibrated against the baked static atlases (`sdf_calibrate`).
- uGUI auto layout (`ugui.rs`): LayoutRebuilder passes, Horizontal/Vertical/Grid groups, ContentSizeFitter, AspectRatioFitter, LayoutElement, LayoutUtility priorities, Image preferred sizes; the driven RectTransform values override the serialized/scripted ones for the frame. TMP preferred sizes and ScrollRect are not ported; `UiFrame::layout_unported` reports any use.
- HUD placement bug: player builds serialize RectTransform m_LocalPosition as 0. For a RectTransform under a non-rect parent, Unity's localPosition.xy is its anchoredPosition. Before the fix the HUD Camera's StyleCanvas (anchoredPosition 1.3, 0.3) and GunCanvas (-1.06, -0.53) sat at the HUD root's centre; `scenedef` now takes xy from anchoredPosition (also in world0).
- StyleHUD meter visibility (styleHud = child 0, shown only while comboActive || forceMeterOn) and RailcannonMeter (CheckStatus, Update; raicharge 0).
- `ui_placement` (normalized screen bounds of every visible gameplay UI draw a few seconds after the level starts, at 1920x1080, 1280x720, 2560x1080, 1280x1024 and 3840x2160): crosshair centred; GunCanvas x 0.04–0.31, y 0.05–0.46 (16:9); RailcannonChargePanel only with the saved railcannon (0-1 with the revolver), hidden in 1-1 without weapons; StyleCanvas hidden with no combo. At 5:4 the HUD Camera canvases clip at the screen edge as Unity's (implicit vertical fov 90, no gate fit) do. No unported layout inputs are hit.
- Present probe: the hint-ink band is now the centre third, clear of the bottom-left HUD. Note: `cargo run -p uk-harness -- --render` does not rebuild `ultrakrust.exe`, so `cargo build --release -p ultrakrust` first.

## 2026-10-08 — Player vs enemy collision
- `collide.rs`: `Shape::Capsule` (spheres are capsules with a == b): closest point, raycast (a ray starting inside misses, like Unity), segment distance (Ericson), depenetration. Level Sphere/Capsule colliders use it instead of box approximations. `depenetrate_capsule_filtered` takes an owner filter.
- `player.rs` `Bodies`: a second collision world with one moving group per enemy (owner = scene collider index; layer, isTrigger, Slippery tag, transform anchor). The game syncs each group to the enemy's pose and enables a collider while the enemy is alive, its node active and the collider enabled (HandleStandardDeath destroys the root collider).
  - Physics: after the level depenetration, the capsule is pushed out of the solid enemy colliders on layers in the player's matrix row (layer 2: 0,1,2,4–8,11–14,16,18,19,22–24,26,28,30,31; layer 15 while invincible: no 2,5,12–14,19,23). Only level contacts feed ClimbStep.
  - GroundCheck (layer 20 row) OnTriggerEnter/Exit against bodies: checkable = ground; layer 12 = enemy step (currentEnemyCol, canJump); exits from disabled colliders are not sent. UpdateState's 40-unit / active check.
  - HandleInputs: enemy step = !onGround && (canJump || CheckForEnemyCols()), then EnemyStepResets (currentWallJumps, clingFade; rocket/hammer counters not ported).
  - WallCheck: layer-11 non-trigger bodies count as walls, with the point of contact when no level collider is touched.
- `enemy_contact` probe (0-1, first enemy of each type, held in place): Filth / Stray walk min gap -0.0001 / blocked, dash passes through, dropped on top: canJump && !onGround then a jump (vel.y -11.5 → 27.8). Malicious Face: blocked in the air, dash deflected around the layer-11 head mesh, dropped on top: onGround on the mesh. `enemy_colliders` lists every enemy's colliders by type.
- Bot (`bot.rs`): the gap-jump check samples 1.3/1.6/1.9 ahead (a single ray fell through the 0.14 seam between two Fan Room glass panels and jumped the bot into the fan pit), and a waypoint whose hold ends with the bot strafed away is walked back to before advancing. 0-1 autopilot back at waypoint 47; quick harness 0 failing.

## 2026-10-08 — F3 debug overlay, crosshair rings, Malicious Face movement and corpse, arena status gating
- F3 toggles a debug overlay (off by default): the placeholder crosshair, the bottom-left debug text and the checkpoint banner only draw with it on.
- Crosshair HUD (`hud.rs`): Crosshair.CheckCrossHair (crossHair / crossHairColor / crossHairHud prefs: main and alt crosshairs, colour, the chuds' circle sprite), FadeOutBars (CheckState, ResetTimer, the fade) and SliderToFillAmount (fillAmount from the slider × maxFill, copyColor, alpha from the parent FadeOutBars, Image disabled while invisible). `hud_probe`: a 40 hit takes HealthSlider to 0.291 at once, HealthSliderAfterImage follows it down to 0.291, and the HealthBars show at alpha 1 for 2 s, then fade out within 1 s. Boost -100 draws the stamina rings down (0.120 → 0.051) and back to 0.150 as it recharges, and StaminaBars fades the same way.
- NavMeshAgent settings come out of the scene (`SceneDef.nav_agents`, class 195: baseOffset, speed, acceleration, radius, height).
- Malicious Face:
  - MovementUpdate: while not charging a beam, the agent heads for the player along a complete navmesh path (speed 3.5, acceleration 8, braking into the destination), riding baseOffset × scale (10) above the navmesh. It stops when there is no complete path, and the beam charge stops it.
  - ProcessDeath / HandleCollision: the corpse stays. It falls with gravity on layer 11 until it touches the Floor-tagged arena floor (0-1: centre 53.5), drops 1.5, loses its SphereCollider and SpiderBodyTrigger, and keeps the head mesh solid.
  - `mf_probe`: the live Face blocks walking, deflects dashes, can be stood on and stops jumps. The corpse blocks walking and dashing (min gap 0), and a player dropped onto it slides off the round top.
- ActivateArena waitForStatus with ArenaStatus (GetComponentInParent; SetStatus / AddToStatus calls):
  - OnEnable activates only once the status is reached.
  - OnTriggerEnter below the status remembers the player (playerIn); exit forgets them.
  - Update activates once the status is reached while the player is in.
  - Before this, every wave trigger in a room fired on the first visit.
- `arena_status` probe in 2-3's Main Hall (ArenaStatus 0):
  - Entering Triggers 1A/1B spawns wave 1. Trigger 2 (wait 1) and Trigger 3 (wait 2: the Mindflayer and the four door locks) stay off.
  - AddToStatus to 2 with the player standing in Trigger 3 activates it. Re-entering Trigger 2 at status ≥ 1 activates it.
  - In the game, the status moves when skulls are taken off the four altars (ItemPlaceZone reverseArenaStatuses). ItemPlaceZone / ItemIdentifier (skulls) are not ported, so those two waves stay dormant until skulls exist.
- forEnemy arenas (activated by an Enemy-tagged collider) are still not ported; parity.rs marks ActivateArena Partial.

## 2026-10-08 — Hit particles (blood)
- `uk-assets/src/particles.rs`: ParticleSystem / ParticleSystemRenderer decoding (initial, shape, emission, colour/size/rotation over lifetime, velocity, trail modules; MinMaxCurve / MinMaxGradient evaluation) and particle prefabs loaded outside the scene graph; unsupported enabled modules are listed per system (`particle_defs`, `gore_dump`, `tag_list` examples).
- `uk-game/src/particles.rs`: the simulation (local space, scaling modes, trails), Instantiate / clone / Play, BloodsplatterManager pools and GetGore, Bloodsplatter (play on enable, heal sphere radius from the prefab, repool via DisableCollider when stopped), BloodUnderwaterChecker, RemoveOnTime, PortalAwareParticleSystem. Play only starts systems whose GameObject is active in hierarchy.
- `enemy.rs` damage follows Enemy.GetHurt: air x1.5 for zombies, knockback, OnDamage (Malicious Face can cancel), limb multiplier, HandleBloodSelection (Head / Body / Limb / Small by tag, damage and hitter), kill, ProcessBloodEffects (thickLimbs closest point, drill x2, shotgun/explosion hp 3, nail hp 1, GetReady unless noheal). Hits carry the hit GameObject (HitTarget) and the hitter string. The old placeholder heal is gone: healing now comes only from the gore's heal sphere.
- Malicious Face HandleSpiderDamage: the pooled Small original plays where it was left, its clone goes to the hit target (hp 30, ready while alive, played when goreOn), dripBlood at the target parented to the face (LookAt + Rotate 180), woundedParticle at the half-health crossing; corpse hits spawn an unplayed clone and cancel. GoLimp sets noheal 1 s after death for every ported enemy (the exemption is for Mindflayers).
- Renderer: particle materials from the prefabs get synthesized quad draws in 64 slots per material; each frame live billboards (view-aligned, size clamped by min/maxParticleSize of the viewport height) and trail strips are expanded on the CPU into the variant's vertex layout. `UNITY_FRAME_STATS` logs slot use and red-blood pixel coverage; `UNITY_PROBE_GORE` spawns BS Head / Body splashes in front of the camera.
- Probes: `gore_probe` (0-1): Filth hits spawn BS Body (heal 10 at 0.09 s); Malicious Face hits spawn the Small original + clone, DripStainless and the selected gore, heals [30, 10]; the woundedParticle at 13 -> 12 of 25; pools refill. Render: 5/5 particle materials resolved, no validation errors, blood coverage 5.8% -> 39-74% with the probe splashes.

## 2026-10-08 — Particles: full Shuriken modules, scene systems, surfaces, enviro gibs, scrapes, events
- Interpreter (`uk-assets/src/particles.rs`, `uk-game/src/particles.rs`): noise (Perlin), collision (world colliders on collidesWith, or planes from scene nodes; bounce / dampen / lifetime loss / min kill speed / radius scale), limit velocity, force, inherit velocity, colour / size / rotation by speed, lifetime by emitter speed, texture sheet animation, mesh and MeshRenderer shapes, arc / radius spread modes, rate over distance; stretched, horizontal / vertical and mesh render modes. `particle_modules`, `particle_inventory`, `scene_particles` list what each level's systems enable.
- Scene-placed systems (`SceneDef::particle_systems`): they follow their GameObject (activation plays the playOnAwake ones, deactivation stops and clears), movers included; NewMovement.windStateParticle via the camera's default position.
- MainModule.stopAction: when a system goes from running to stopped, Disable deactivates its GameObject and Destroy destroys it (the effect when it's the root). `stop_actions` lists the non-zero ones (none in 0-1, 1-1, 2-3, 4-2, 7-1).
- UnityEvent ParticleSystem calls (`Target::Particle`): Play / Stop (StopEmitting) / Clear with children, skipping inactive GameObjects. `particle_calls` counts them across the campaign (ObjectActivator Play 37 / Stop 23 / Clear 1, TriggerEnterMessage Play 100, Door Stop 9, SubDoor Play 11, EventOnRandomTimer Play 10, MovingPlatform Play 7, Morse Play 2, Lerp Play 1, EnemyIdentifier Stop 1). `ps_call_probe` (0-E): the Furnace Morse Play starts emission (10 particles in 0.5 s), the HeatActivation ObjectActivator stops the Snowfall (120 particles left to die), the Cooling Chamber vents play and stop.
- Lifecycle: RemoveOnTime on child nodes (the node and its systems go), DestroyOnCheckpointRestart on effects at a checkpoint restart only (CheckPoint.OnRespawn). `lifecycle_probe`: RubbleFilth chunks go at 1 ± 1 s, the Explosion survives a level-start respawn and is destroyed by a checkpoint restart.
- SceneHelper surfaces (`surface.rs`): the footstep physics scene from the environment's renderable colliders (IsValidForPhysicsScene, PreservedOriginalMesh / MeshCollider / MeshFilter mesh, load-time transforms), raycast single-sided; ResolveHitSurfaceData picks the submesh material's `_SurfaceType` / `_EnviroParticleColor`, the secondary pair under VERTEX_BLENDING by vertex red. `surface_probe`, `surface_survey`.
- CreateEnviroGibs: the surface's enviroGibParticle facing the normal, EnviroGibModifier (increaseBurstEmission: startSpeed and first burst x size, localScale / size), root scale x size, SetParticlesColors (non-white colour keeps the modifier's alpha). Callers: the revolver (ray, not SphereCast), Projectile hits on static environment (size clamp(dmg / d, 0.5, 1), d 4 friendly else 10), LandingImpact. `enviro_gib_probe` (0-1): sizes 1 / 0.5, wood / glass / grass / tall with a tint, revolver dust + LaserHitParticle, a 60 m landing (DustBig + EnviroGibDust).
- NewMovement: CreateSlideScrape (surface under the player, or at the wall check's contact airborne; new scrape on type change at pos + dodgeDir x 2 facing back, old one detached), HandleSlideState placement (pos + n facing -n, parked at 5000 off the ground), CreateWallScrape on Cling (hit point + up, facing the wall normal), DetachScrape (StopEmitting + RemoveOnTime 10), the frictionless slide particle (airborne branch; the grounded branch needs CustomGroundProperties, not ported). Unity's Vector3.normalized (zero under 1e-5) where it matters: a tiny velocity had put the slide particle 10 m off. `scrape_probe` (0-1): the slide scrape spawns with the slide, follows at pos + 1, detaches with 10 s left; the wall scrape flickers at the start of a cling (clingFade starts at 0, as in the game).
- Other spawns: Breakable breakParticle (bounds centre, applyScaleToParticle, customPositionRotation), Glass shatterParticle, CheckPoint activateEffect (not when invisible), TeleportFinalPit, DualWieldPickup, PowerUpMeter endEffect, ZombieMelee pullOut, Malicious Face impactParticle, Punch dust.
- Assumptions pending the oracle: Unity's particle RNG and noise function, the 30 Hz collision dampen reference, inherit velocity in world space only, stretched-billboard and mesh rotation conventions, flip by particle hash, burst counts rounding half away from zero, culling ignored. Not ported: rigidbody gib meshes, sprite renderers, PhysicalShockwave, CustomGroundProperties, dive / parry / BreakCorpse / enrage effects; the airborne frictionless path is unprobed (no roomy slippery or layer-0 slope in 0-1, 1-1, 1-2, 2-1, 3-1, 4-1).

## 2026-10-08 — Custom glTF maps (`--map <file.glb>`)
- `uk-assets/src/gltf_map.rs`: loads a .glb / .gltf (default scene) at runtime. Godot's import suffixes (`-x` / `_x` at the end, trailing digits and dots ignored, or `$x` anywhere; stripped from the name): `-noimp` drops the subtree, `-col` / `-convcol` keep the mesh and add a trimesh / convex-hull collider, `-colonly` / `-convcolonly` keep only the collider. A `-colonly` empty takes its shape from the extras `empty_display_type` / `empty_draw_type` (glTF does not store Blender's draw type): CUBE gives a box of size 2, IMAGE a world-boundary slab, SINGLE_ARROW a ray (skipped, Unity has no ray collider), anything else a sphere of radius 1. `-occ` / `-navmesh` / `-rigid` / `-vehicle` / `-wheel` are warned and ignored. Cameras are not imported.
- Every collider is a static, enabled Environment (layer 8) collider; its triangles go into the footstep surface scene with the placeholder material's surface type (MetalGrill, 9). Meshes draw with 0-2's exit elevator floor material ("Metal Pattern 2 15"). Vertex colours are ignored (white).
- Lights (KHR_lights_punctual): linear colour -> sRGB (gamma-space rendering). Directional intensity = lux. Point / spot match the inverse-square tail of Unity's falloff (intensity x range^2 / 25 d^2 = I / d^2): with no glTF range, intensity 1 and range 5 sqrt(cd); with one, that range and intensity 25 cd / range^2. Spot angle = 2 x outer cone. These are assumptions to tune by eye.
- The rest of the scene is 0-1 pruned (`SceneDef::retain_nodes`, with every index remapped) to FirstRoom without Room and the PlayerActivator Cube, plus StatsManager and EventSystem: the player rig and HUD, GameController managers, OnLevelStart. `SceneDef::move_root` moves FirstRoom so the player's feet are on the origin, facing -Z (glTF forward). `Game::start_custom_map` activates the player and starts the level and timer, as the missing PlayerActivator would. 0-1's RenderSettings (fog, ambient) are kept.
- `map_probe` (custom_maps/example.glb): 4 nodes, 2 renderers (996 tris), 1 trimesh collider (`Plane -col`, 36 tris); the player starts with feet at y 0, falls and lands on the Plane at y -3.83, and the surface under them is 9 (MetalGrill). `Sphere -conv` is not a Godot suffix, so it imports as a plain mesh. In the game, all of the map's renderers draw and nothing errors.

## 2026-10-08 — Custom map materials, Steam library lookup, repo for sharing
- glTF materials (`gltf_map::material_override` -> `SceneDef::material_overrides` / `renderer_override`): each one is a MaterialOverride on the placeholder material (ULTRAKILL/Master), since the master shader has no smoothness / metallic inputs (taken as 0, per the user). `_Color` = sRGB(baseColorFactor), `_MainTex` = the base colour texture or 1x1 white. With a non-zero emissiveFactor: keyword EMISSIVE, `_EmissiveTex` (or white), `_EmissiveColor` = sRGB(factor), `_EmissiveIntensity` = KHR_materials_emissive_strength (default 1), `_UseAlbedoAsEmissive` / `_EmissiveReplaces` / `_EmissiveMask` / `_EmissiveToVertexColors` 0. Images: PNG / JPEG (the `image` crate), buffer views or files beside the map, rows flipped to Unity's bottom-first order; filter = point when magFilter is NEAREST, wrap from wrapS (repeat / clamp / mirror). A texture on a UV set other than 0 is warned. The legacy renderer ignores overrides.
- unity_render: an override clones the base props and adds its keywords / floats / colours, caches the variant per override, and its textures upload before the bundle lookup. The summary lists "custom materials". Test map (scratchpad make_gltf.py): Checker `_MainTex` 2x2, filter 0, wrap 1, `ULTRAKILL/Master#62331@0`; Glow `_EmissiveTex 1x1` + EMISSIVE, `#62343@1`, `_EmissiveColor` [1, 0.485, 0], intensity 4.
- Winding fix: the surface scene stores Unity's (mirrored) winding, so glTF's counter-clockwise triangles are stored reversed (and mirrored nodes flipped); box / sphere surface triangles face out, degenerate pole triangles dropped. Before this, surface raycasts missed glTF floors (the earlier "9 under the player" on example.glb came from the reversed winding). Test map: the player lands at y 0.5 on Floor -col (-1), surface 9; the -colonly CUBE empty is a box at (6, 1, 0), half (1, 2, 4). example.glb's Plane: 13 of its 36 triangles face out, and the floor triangle at y -3.83 faces down (culled from above, no surface hit), as Unity would do; the fix is in the file (recalculate normals).
- `find_install`: `ULTRAKILL_DIR`, else `steamapps/common/ULTRAKILL` under the default Steam roots (Windows Program Files (x86) / Program Files, ~/.steam/steam, ~/.local/share/Steam) or any library in their `libraryfolders.vdf`.
- Repo: README rewritten for players (build, flags, controls, custom-map guide, crate layout, credits), .gitignore extended to every game-derived extension (.cs, .dll, .ttf / .otf, .wgsl, .spv, bundles) plus editor / OS clutter. `material_props` example dumps a material's props and shader variant inputs.

## 2026-10-08 — Enemy spawn effects clear (SpawnEffect, scene RemoveOnTime)
- Bug: an enemy's spawn bubble stayed at full size forever. Root cause: EnemyIdentifier.Start activates the enemy's `spawnEffect` child (SpawnEffect Melee / Projectile / 2), whose own scripts do the rest, and neither was ported for scene objects. RemoveOnTime (time 2, 5 on the Malicious Face's) ran only on instantiated effect prefabs, and SpawnEffect was `Script::Other`, so the bubble never shrank, the light never lit and the burst never played.
- `Script::RemoveOnTime`: Start invokes Remove after time + Random.Range(-randomizer, randomizer) -> Destroy(gameObject). `Script::SpawnEffect`: Start takes the first child as the bubble; unless the `simpleSpawns` pref is set, it enables the first active Light in children and plays the ParticleSystem (with children). Update: bubble localScale -= 2 dt (clamped to zero), light range -= 50 dt while > 0. The audio pitch / play is not ported.
- Runtime transform / light state: `State::local_scale` (script-changed localScales; `Game::scale_delta` maps a node's animated world to the rescaled one, and unity_render applies it per draw) and `State::lights` (Light.enabled / range per scene light; the frame's light tuple carries the range, a point / spot light with range <= 0 is off). The legacy renderer does not apply them.
- `spawn_fx` (0-1, Gun Room Wave 2 / Zombie (6)): 108 enemies have a spawnEffect, 103 with spawnIn. Bubble world x-scale 2.25 -> 1.35 (0.2 s) -> 0 (0.5 s). The light goes on at the first frame, range 25 -> 15 -> 5 -> -0.2 (off). 60 particles, all dead by 0.6 s. The object is destroyed between 2.0 and 2.2 s, and no scale overrides are left.

## 2026-10-08 — Custom maps: "no lights" was the export
- Report: custom maps import no lights. `glb_json.py` on custom_maps/corinth_packed.glb and example.glb (Blender glTF I/O 5.0.21): no KHR_lights_punctual in extensionsUsed and no light entries. Blender's exporter only writes lights with Include > Data > Punctual Lights, which is off by default. The loader is fine: the scratchpad test.gltf's point light imports (kind 2, colour [1, 0.735, 0.537], range 50) and is enabled and active in the game (`map_probe` now prints the map's lights with their runtime state).
- The loader now warns when a map has no lights, naming the export option. README: the same note under Lights.
- Lights were 1000x too bright from Blender (normalize on, per the user). `GLTF_LIGHT_SCALE` = 1/1000 multiplies every glTF intensity before the Unity conversion (for point / spot, brightness at a distance is proportional to it). Test map: 100 cd -> 0.1 -> intensity 1, range 1.58 (was 15.8).

## 2026-10-08 — Convex MeshColliders, BigDoor (2-3 collision, 2-1 doors)
- 2-3, first arena ("1 - Main Hall"): walkways and floor pieces had no collision. Their MeshColliders are convex, and PhysX collides a convex MeshCollider as the hull of its mesh, but the port collided the raw triangles; for these ProBuilder pieces those left gaps. scenedef now replaces a convex MeshCollider's triangles with `gltf_map::convex_hull` (flat meshes keep theirs). `solid_gaps` (rays along each renderer's triangle normals): RAY at (-13.3, 14.5, -373.3) now hits `Walkways/Cube(Clone)` at y 12.5 and (8.3, -8.0, -381.7) hits `Floor/Cube` at y -10 (both missed before); Main Hall renderers with > 30% of their area missed went 62 -> 52, and the rest are layer-25 visuals (not solid to the player per the PhysicsManager matrix, row 2 = 0xd5cd79ff), water and decor. The 24 MeshColliders the scene drops (`dropped_colliders`) are faithful: null meshes, or ProBuilder MeshFilters with 0 vertices.
- 2-1: the DoorLust doors are `Door` with doorType 1 (BigDoorController) whose leaves are `BigDoor`s, both unported, so Open returned at once (open offset 0 = "already open"). Door now follows doorType: BigDoorController collects its BigDoors (inactive too) in Awake (startOpen opens them), finds DoorControllers below itself, skips the Normal position checks and movement, opens the leaves with its reverseDirection (set by DoorController) and closes them, and Lock closes it when a leaf is open; the close-other-doors loop finds a non-Normal door's controller in its children. `Script::BigDoor`: Awake (origRotation, target = euler + openRotation, snap if open), Open (target recomputed with the reverse sign within 20° of closed), Close, Update (RotateTowards at speed deg/s with gradualSpeedMultiplier and playerSpeedMultiplier; onFullyOpened / BigDoorClosed's onFullyClosed). Unity's quaternion ==, Angle, RotateTowards and eulerAngles are reproduced. Runtime rotations live in `State::local_rot`, and `mover_delta` now carries rotation (parent · T(p) · R · R0⁻¹ · T(-p0) · parent⁻¹), so leaf colliders and renderers turn with them; each BigDoor is its own mover. Not ported: sounds, openLight, screen shake, SubDoorController.
- `probe_doors level2-1`: DoorLust, (2), (3), (4): 8 leaves each reach the open target, and return to the closed rotation after the player leaves. The 2 remaining NO DOOR FOUND (`Decorations/DoorLust`) are decorative copies with no Door script. 0-1 unchanged: 11 OK, the 5 decorative frames without a door.
- Cerberus spawn trigger in 4-2: skipped (Cerberus is not fully ported).

## 2026-10-08 — 1-1 hung on load (convex hull)
- Report: 1-1 does not load. `load_time level1-1` never finished. The last hull started was `4 - Altar Field/Decorations/StatueLionOutside/SunStatue` (320 triangles). The determinism fix in 7c39d09 had turned the hull's horizon-edge set into a Vec, which kept duplicate edges, so faces multiplied with every added point.
- The hull was also wrong before that: with the old set it gave 3,771 faces for SunStatue. Worse, `LimboSkybox/Ruins/Layer1Pillar (2)` had faces cutting through the inside, with a point 0.58 m outside one of them. These colliders sit about 150 m from the origin, where f32 spacing (about 1.5e-5) exceeded the tolerance (size x 1e-5), and the faces folded. Brute force gives 12 true planes for that pillar's 19 points.
- `convex_hull` now:
  - works in coordinates centred on the mesh, with tolerance size x 1e-4;
  - adds the farthest outside point first (quickhull order) and drops points once they're inside;
  - keeps the horizon edges in a BTreeSet (no duplicates, and the same order every load).
- `hull_check` checks every hull-shaped collider: closed (each edge has its twin), all points on or behind every face, faces = 2V - 4.
  - 1-1: 911 hulls, 0 bad, largest 96 faces. 1-1 loads in 3.2 s (Game::new 80 ms).
  - 2-3 and 0-1: the 3 and 2 meshes it flags are non-convex MeshColliders that aren't flagged convex (left as triangles). Their hull happens to have the same face count.

## 2026-10-08 — Custom maps: Filth, Stray and Malicious Face placements
- Suffixes on empties (docs/MAP_FORMAT.md): `-filth`, `-stray`, `-maliciousface`. A suffix on a mesh warns, and the mesh is not imported.
- Templates are 0-1's own enemies:
  - Filth: `11 - Projectile Zombies Room/.../Wave 1/Filth/Zombie`
  - Stray: `10 - Combo Hallway/.../Wave 1/Projectile Zombie`
  - Malicious Face: `13 - Malicious Face Arena/13 Content/Boss/Spider`
  - `prune_base` keeps them, with their ancestors, until they are copied. Then they go.
- `SceneDef::clone_subtree` does what Object.Instantiate does for a scene subtree. It copies:
  - nodes, renderers (skin bones remapped) and colliders;
  - scripts (with prefab and nested refs), rigidbodies and nav agents;
  - lights, animators, UI natives, surface meshes and particle systems.
  - The copy gets fresh path ids, and scene-file PPtrs inside it point at the copies.
- Placing a copy:
  - The root keeps the template's scale and tilt, turned by the yaw between the template and the empty, at the empty's origin.
  - It is set active, and EnemyIdentifier spawnIn is false (no SpawnEffect).
- `map_probe` on a scratchpad map (Filth at (-4, -1, -5); a Stray at (4, -1, -5) turned 90 degrees; a Malicious Face at (0, -1, 8)), all three alive with spawn_in false and 0 spawn effects:
  - At start: Filth yaw 0 (hp 0.5), Stray yaw -90 (hp 1.5), Malicious Face hp 25, with its body at (0, 9, 8), the template's offset.
  - After 3 s: the Filth walked to the player and hit them (player hp 100 to 70), and the Stray turned to face them.
- Custom maps have no navmesh (retain_nodes clears 0-1's), so Filth and Strays use the port's straight-line fallback. The parser warns. MAP_FORMAT said they would stand still, which is now corrected.

## 2026-10-08 — Custom maps: `-navmesh`
- A `-navmesh` mesh (invisible, no collision; several merge) becomes the level's navmesh: `navmesh::from_triangles` lays the triangles out as one baked tile.
  - One polygon per triangle, with vertices welded within 1 mm.
  - Each polygon edge links to the polygon sharing it, and each triangle is its own detail mesh.
  - Vertical or degenerate triangles are dropped (warned), with Detour's 65,535-per-tile cap.
  - Polygons are wound clockwise from above in Bevy space, as every baked tile is (`nav_winding level0-1`: 412 cw, 0 ccw).
  - Agent settings come from 0-1's humanoid navmesh (radius 0.5, height 2, climb 1.5), read before pruning clears it.
  - The mesh is assigned after the template `retain_nodes`, which clears navmeshes.
- No erosion: the mesh is used as-is (Godot's convention). Authors leave about 0.5 m clear of walls.
  - With a hole exactly the barrier's footprint, the Filth's path corner sat on the wall corner, and it stuck at (2.47, -3.49).
- Walking Filth and Strays now stay on the navmesh, as a NavMeshAgent does: grounded feet snap back to `nav.nearest`, and outward velocity is removed. Airborne (knockback, ledges) is free. Before, the Stray's back-off (the port's stand-in for ZombieProjectiles.Flee, which samples its destination onto the navmesh) walked it off the 20 m floor, and it fell to y -57.
- `map_probe` on a scratchpad map: a 1 m grid navmesh with 1 m margins around a barrier (x -8..2, z -3..-2) between the Filth and the player, and around the wall. Result: 648 polygons, 1,806 links, 1 component.
  - The Filth: (-4, -5), then around the barrier's east end ((3.06, -3.97) at 0.72 s, (3.76, -1.94) at 0.96 s), then to the player. It hit them at 1.9 s (hp 100 to 70).
  - The Stray backs off to the navmesh corner (10, -10) and stays there.
  - The Malicious Face now walks its navmesh toward the player.
- 0-1 harness: only sim_budget fails (accepted); the east-stairs autopilot gap was already there.
