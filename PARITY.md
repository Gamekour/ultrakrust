# ULTRAKRUST — Parity gap analysis

**Goal:** a 1:1 reconstruction of ULTRAKILL in Rust/Bevy: every feature, menu, sound, texture, light, animation,
reaction and control behaving as the original does. Content (meshes, textures, audio, levels, UI layouts) is read from the
user's own install at runtime; behaviour is ported from reading the decompiled scripts locally.

**Rule for deviations:** something may differ from the original **only** when an architectural limit forces it. Each
one is listed in [§4](#4-architectural-limits-the-only-allowed-deviations) with the reason and the closest achievable substitute. Artistic or "good enough" deviations are bugs.

All numbers below are measured by tooling (`uk-harness --full`, `parity_scripts`, `census`) against install build 22957324,
2026-10-04, not estimated.

---

## 1. Where we are (measured)

| Measure | Original | Ported | |
|---|---|---|---|
| Decompiled C# | 1,538 files, 205,681 lines | ~11,100 lines Rust | ~5% by volume |
| Script classes used by scenes | 776 distinct, 54 scenes | 32 present in scenes (registry: 15 full, 20 partial or data-only) | **4.1%** |
| Script instances in scenes | 617k | 3.6% have a port, 1.4% a full one | |
| Scenes that load | 54 | 54 | geometry, colliders, scene graph |
| Enemy types (`EnemyType`) | 43 | 3, all partial (Filth, Stray, Malicious Face) | 7% |
| Weapons | 5 guns × 3 variants + 3 arms + coins | Piercer revolver, Feedbacker (partial); ULTRAKILL's own viewmodel prefabs, animated, through the HUD Camera | ~12% |
| Style bonuses (`AddPoints`) | 61 distinct, 156 call sites | 0 | 0% |
| Engine systems (unweighted mean, 37 tracked) | — | **25.7%** (was 22.7% at the gap analysis) | see §3 |
| Level 0-1 autopilot (deterministic) | 56 waypoints | **43/56** (was 25 before NavMesh) | stalls at the Combo Hallway east stairs: the hand-written route asks for a 2-high step under a 3-high lintel; a bot route problem, not a game one |
| Scenes that build a level runtime | 54 | 54 | every scene builds a `Game` without panicking |
| Level start through the level's own trigger (`FinalDoorOpener.GoTime` / `PlayerActivator.startTimer` → `OnLevelStart.StartLevel`, `onStart`, StatsManager timer) | 41 campaign levels | 41/41 | harness `first_rooms` |
| Exit elevator -> next level (`FinalPit`, `TeleportFinalPit`, FinalRank, `targetLevelName`) | every campaign level | Full in the sim; results tally, Fire1 continue, R ignored once over | harness `0-1.exit_to_next_level`, `0-1.exit_ignores_restart`; frontend `UK_PROBE_EXIT` 0-1 -> 0-2 |
| Custom glTF maps (`--map`, not in ULTRAKILL) | — | Godot suffix colliders, lights by type, 0-1's player and managers, materials (base colour / emissive texture and factor on ULTRAKILL/Master, smoothness and metallic 0); `-filth` / `-stray` / `-maliciousface` empties copy 0-1's enemies (active from the start, no spawn effect). an `-env` empty sets the skybox (any campaign RenderSettings skybox, by name), fog and ambient light for the level; `-room` collections and `-door` meshes load and unload rooms with 0-1's Door and DoorController (room lists derived from where the doors are); `-navmesh` meshes become the level's navmesh (one tile, one polygon per triangle, not eroded by the agent radius). Not yet: waves and arenas, the other suffixes in docs/MAP_FORMAT.md | `map_probe` |

**0-1 checks passing:** load, all 11 live door controllers, all 14 arenas, boss → final door → final pit, checkpoint respawn,
tick-for-tick determinism, sim under budget (p99 1.3 ms/tick), navmesh, Fan Room path.

**NavMesh (all levels):** all 6,811 tiles in the install decode (size formula exact). Smoothed paths pass through every
portal of their corridor on all 45 navmeshes (300 random pairs each). 0-1: 411/412 polygons sit on collision geometry.
6-2 ships a stale bake (2.8% on geometry; no colliders or renderers within 535 units of it, in the original too).

## 2. Misunderstandings in the previous plan, corrected

1. **"0-1 playable, the bot gets through ~70%" was luck.** Scene loading iterated a `HashMap`, so script and collider
   order (and with it Awake/Start/Update order and outcomes) changed on every launch. Fixed (`scenedef.rs`, `scene.rs`).
   Deterministically the bot reaches 25/56, and the boss check only passed when the random order happened to put the
   live `FinalPit` first. Every result before this commit should be treated as unverified.
2. **Wrong axis of work.** The plan went level by level, hand-porting whatever scripts each level needed. The census
   shows content is ~95% *data* consumed by a few dozen engine systems (UI 329k RectTransforms, 30k AudioSources,
   15k Lights, 11k ParticleSystems, 9k Animators). The right axis is **data-driven interpreters for Unity's native systems**
   first, then scripts. One correct `ParticleSystem` interpreter fixes 10,675 effects at once; 10,675 hand-made effects would never match.
3. **Menus were treated as something to rebuild.** Every level carries the full pause, options, shop, cheats, sandbox and
   console UI as scene data (uGUI + TextMeshPro). A uGUI/TMP interpreter plus the menu scripts gives every menu exactly as
   authored. Hand-built menus cannot reach parity.
4. **Lighting was approximated** ("additive light pillars"). Every shader ships with a **Vulkan SPIR-V** build
   (platform set D3D11/Metal/Vulkan on all 301 parsed shaders), and wgpu's naga reads SPIR-V. The target is to run the
   game's own compiled shaders, not lookalikes.
5. **Enemy AI was planned as "direct pursuit for now".** The baked NavMesh is in every level as Detour-format tiles
   (`DNAV`, Unity version 16; 0-1: 43 tiles, 71 KB, off-mesh links, agent r 0.5 h 2 climb 1.5 drop 15). We read and query
   *that*, so we get the original paths without re-baking.
6. **Scene hierarchy child order uses path-id order**, not Unity's sibling order (`m_Children`). Anything that iterates
   children (`GetChild(i)`, `GetComponentsInChildren`, wave lists) can come out in the wrong order. Open bug.
7. **No oracle.** Parity was judged by reading code and looking at screenshots. Behaviour needs to be diffed against the
   real game numerically (§5).

## 3. Gap map by subsystem

Status 0–1 is what `crates/uk-game/src/parity.rs` declares; the harness turns it into the coverage metrics.

| Pillar | Original (instances in install) | Now | Approach | Verified by |
|---|---|---|---|---|
| **Asset reading** | 81 bundles | Meshes, textures (not BC7), materials (main tex), scene graph | + BC7, AudioClip (FSB5/Vorbis), AnimationClip, AnimatorController, Avatar, ParticleSystem, Sprite, Font/TMP, NavMeshData, Shader blobs, VideoClip, LightmapSettings | round-trip vs UnityPy per type |
| **Rendering** | 416 shaders, 4,474 materials, 14,703 lights, 54 lightmap sets, 97 cubemaps | `--unity-shaders`: ULTRAKILL's own Vulkan programs (SMOL-V → SPIR-V → naga → WGSL) draw 99.9% of renderers on the levels tested; Unity built-ins filled by name (matrices, `_Time`, fog, the 8 `Vertex`-mode lights); gamma-space offscreen target composited to linear. UNDERWATER from Water + UnderwaterController (the CameraCollisionChecker sphere picks up Water colliders; overlay color from the Water's `clr`, or the overlay Image's color at alpha 0.3; the player's capsule gets ApplyWaterForces per touched Water). RenderSettings skybox (cubemap, 6-sided, procedural) on a camera sphere, or the Main Camera's solid clear color. PostProcessV2: the main camera also writes SV_Target1 into an RG8 outline buffer (per-target blend state, EnemySimplifier's property block), OutlinePx pass 3 multiplies the scene by it before the HUD Camera, then the PostProcessV2 shader composites with the material's textures (`_Dither`, `_NoiseTex`, `_VignetteTex`) and a variant per runtime keyword combination (DEAD, UNDERWATER, VIGNETTE, WICKED: all 16 built): the hurt flash `_HurtScreenColor` from NewMovement.hurtScreen, DEAD with DeathSequence's `_Deathness`/`_Sharpness` (then EndSequence's BlackScreen with YouDiedText, the TextAppearByLines log over it, and StatsManager's R/Fire1 restart), WICKED with ScreenDistortionField/Controller's `_RandomNoiseStrength` (0-S Wicked, 1-2 rodents), VIGNETTE with PowerUpMeter's `_VignetteColor` (DualWield's (1, 0.6, 0) at alpha juice/latestMaxJuice; DualWieldPickup grants juiceAmount, death/respawn/FinalPit zero it, DisablePowerUp ends it). The player's graphics prefs (`Preferences/Prefs.json`, read-only) drive GraphicsSettings' globals: pixelization renders the scene, depth and outline targets at the virtual size (point-upscaled by PostProcessV2 at screen size, `_VirtualRes`/`_ResY`), `_ColorPrecision`, `_DitherStrength`, `_Gamma`, `_VertexWarping` (+ VERTEX_WARPING keyword), `_TextureWarping`, `_StainWarping`; fieldOfView and cameraTilt reach the camera. Not yet: colorPalette (PALETTIZE + `_PaletteTex`; off in the user's prefs), the mouseSensitivity scale (Input System delta units unknown), outlineThickness, DualWield's duplicated weapon (the copy firing with `delay`), the power-up meter UI, Water's DryZoneController/wetness/audio and its forces on non-player rigidbodies, the death screen's sprites (LaughingSkull, red Flash, ISeeYou) and its audio, (heat-wave is never called in ULTRAKILL), the view-normal target (stains), lightmaps (0-1 has none), light probes, stencil/portals Playtest (2026-10-08): no muzzle flashes on gunfire. | keep the remaining renderers translating; per-material uniform audit; PostProcessV2 next | `uk-harness` `shaders` (variants validate, buffers at exact binding+size, vertex channels = SPIR-V inputs) and `--render` (GPU errors, coverage, read-back frame stats per level) |
| **Animation** | 9,202 Animators, 116 controllers, 775 clips, 116 avatars | none (static poses) Playtest: no enemy death animations. | Mecanim interpreter: state machines, transitions, blend trees, layers/masks, root motion, animation events (scripts rely on these for hit timing), humanoid retargeting | state/time traces vs oracle; event timing tests |
| **Audio** | 29,878 sources, 1,606 clips, mixers + 5 filter kinds, 80 reverb zones, 12 music bundles | none | FSB5 Vorbis decode (rebuild Vorbis headers), AudioSource 3D rolloff and priority, mixer groups/snapshots, filters, MusicManager layering (clean/battle/boss crossfade) | decode checksums; per-event "sound played" traces vs oracle |
| **Particles / FX** | 10,675 ParticleSystems, 3,115 trails, 1,429 lines, 6,289 sprites | Shuriken interpreter (`uk-assets/src/particles.rs`, `uk-game/src/particles.rs`) for the effect prefabs the game spawns and for scene-placed systems: initial, shape (incl. mesh / MeshRenderer shapes, arc and burst-spread modes), emission (rate over time and distance, bursts), colour / size / rotation over lifetime, velocity and limit velocity over lifetime, force, inherit velocity, colour / size / rotation by speed, noise, collision (world colliders on collidesWith, or planes), texture sheet animation, trails; renderers: billboard, stretched, horizontal / vertical and mesh. MainModule.stopAction (Disable / Destroy) when a system finishes. Scene systems follow their GameObject (activation plays playOnAwake, deactivation stops and clears), and UnityEvent calls Play / Stop / Clear them with children (ObjectActivator, TriggerEnterMessage, Door, SubDoor, EventOnRandomTimer, MovingPlatform, Morse, Lerp, EnemyIdentifier). Spawns: BloodsplatterManager pools + GetGore, Bloodsplatter, BloodUnderwaterChecker, RemoveOnTime (prefab root and child nodes, scene objects), SpawnEffect (enemy spawn bubble shrink, light fade, burst), DestroyOnCheckpointRestart, PortalAwareParticleSystem; Enemy.GetHurt blood and MaliciousFace.HandleSpiderDamage; Projectile explosions and environment gibs; RevolverBeam hit particles; Punch dust; ZombieMelee pullOut; Malicious Face impact; Breakable / Glass / CheckPoint / TeleportFinalPit / DualWieldPickup / PowerUpMeter effects. SceneHelper surfaces (`surface.rs`): the footstep physics scene (renderable environment colliders, single-sided), material `_SurfaceType` / `_EnviroParticleColor` with VERTEX_BLENDING, CreateEnviroGibs with EnviroGibModifier and SetParticlesColors. NewMovement: dodge, slide (trail gradient while invincible), fall (slam), impact dust and LandingImpact gibs, slide scrapes and wall scrapes (CreateSlideScrape / CreateWallScrape / DetachScrape), the frictionless slide particle, the wind particle. Deviations: Unity's particle RNG is not matched; noise is Perlin (not Unity's); collision dampen is per 30 Hz step; inherit velocity applies in world space only; cullingMode and sortingFudge ignored; more than 64 live renderers of one material merge into one draw; the burst count rounds half away from zero; billboard rotation sign, stretched-billboard conventions, the mesh-particle rotation basis and the maxParticleSize clamp are assumptions pending the oracle. The revolver gib point is a ray, not a SphereCast. The footstep scene uses load-time transforms; HasProperty(_SurfaceType) is approximated. Not yet: rigidbody gib meshes, sprite renderers, PhysicalShockwave, CustomGroundProperties (the grounded frictionless particle), audio, scrnBlood, woundedMaterial, CameraShake, BreakChunks, ragdolls, underwater / sand / blessed gore, stains/decals, dive / parry / BreakCorpse / enrage effects; the airborne frictionless path is unprobed (no slippery / layer-0 slope found) Playtest: blood is not fully 1:1 (possibly the missing gore settings) and spawns at the enemy root, not at the hit point. | Shuriken module interpreter (emission, shape, velocity/limit, colour/size over life, noise, collision, sub-emitters, texture sheet) | particle-count/lifetime traces |
| **Physics** | 44,058 rigidbodies, 233 joints, PhysX 4.1 | own collision (boxes, tri-mesh BVH, spheres/capsules exact; a convex MeshCollider collides as its mesh's hull, like PhysX); the player against the level and against enemy colliders: PhysicsManager's layer matrix for layer 2 (15 while dashing or hurt-invincible, which skips EnemyTrigger 12, so dashes pass through enemies but not through layer-11 BigCorpse meshes like the Malicious Face head), solid colliders only, enemies kinematic (the player can't push them). GroundCheck against enemy colliders: layer 11/26 non-trigger = ground, layer 12 (triggers included, not Slippery) sets currentEnemyCol + canJump, exit clears it; UpdateState drops canJump when that collider is inactive or > 40 away. WallCheck.CheckForEnemyCols (r 2.5 overlap, mask 12, < 40) and EnemyStepResets (wall jumps, cling fade). WallCheck takes layer-11 bodies. ClimbStep only climbs Environment contacts. Deviations: enemy colliders follow the root pose, not bones; non-kinematic (airborne) enemies aren't pushed by the player. Malicious Face corpse: falls with gravity on layer 11 until its sphere touches a Floor-tagged collider, then drops 1.5 and loses its sphere and trigger, leaving the head mesh solid (no BreakCorpse) | Option A: link **PhysX 4.1** (BSD-3, `physx-sys`), Unity's engine, with Unity's settings. Option B: own solver. A is closer to 1:1 (§4.1) | trajectory traces vs oracle |
| **Navigation** | 43 NavMeshData, 2,346 agents, 3,463 obstacles | none | Parse Unity Detour v16 tiles, A* + string pull, NavMeshAgent steering/avoidance, off-mesh links | polygon verts lie on level geometry; path validity; oracle path traces |
| **UI** | 329k RectTransforms, 998 canvases, TMP | debug HUD text; `ugui.rs` lays out and meshes every canvas (scaler, rects, Image Simple/Sliced/Tiled/Filled, RawImage, Shadow/Outline, Mask stencil, RectMask2D clip/cull, CanvasGroup); the renderer draws them with the game's UI shaders (UI/Default and friends, clip-rect/alpha-clip variants, Mask stencil state per draw): Screen Space - Overlay canvases after PostProcessV2 at screen resolution, World Space canvases after their camera's geometry (layer 13 in the HUD Camera pass). Deviation: Screen Space - Camera canvases are drawn as overlay (none seen in the gameplay HUD). TextMeshProUGUI: `tmp.rs` ports TMP's text processing (escapes, `<br>`/`<nbsp>`/`<zwsp>`), SetArraySizes (glyph lookup through weights and fallbacks, missing-glyph substitution, fallback / atlas materials), ValidateHtmlTag for b i u s size color alpha noparse lowercase uppercase smallcaps /line-height /width, and GenerateTextMesh (kerning, bold/italic, word wrap with soft hyphens and CJK breaks, auto sizing incl. character-width adjustment and line-spacing reduction, Truncate, every horizontal/vertical alignment incl. Justified/Flush, SDF uv2 packing), padding and scale ratios from ShaderUtilities, TMP's own stencil/cull rules and sub-mesh draw order. TMP_Settings is read from resources.assets (missing glyph, default font, global fallbacks; line-breaking rules are not read). Dynamic font assets get their glyphs rendered at runtime into the atlas (`uk-assets/src/sdf.rs`: hinted metrics at the face's point size, MaxRects BestShortSideFit packing). Deviation: FontEngine's native distance transform is approximated by exact distances to the flattened outline, calibrated against the game's baked atlases (`sdf_calibrate`). Assumptions: `HasProperty` = the property is serialized, mesh padding computed once from the shared material, serialized text parsed like TextInputBox text. Not ported (unused by the scenes or warned): Ellipsis/Page/Linked overflow (drawn as Overflow), sprites, RTL, underline/strikethrough/highlight meshes, other rich-text tags, vertex gradients, non-Character uv mapping. Slider (Set/UpdateVisuals: fill/handle anchors or Filled amount) and Selectable ColorTint instant normal/disabled colour (CanvasGroup interactable); no pointer/selection states. Auto layout: LayoutRebuilder's passes (CalculateLayoutInput post-order, SetLayout pre-order, self controllers first, layout roots) with Horizontal/Vertical/Grid LayoutGroup, ContentSizeFitter, AspectRatioFitter, LayoutElement (ignoreLayout), LayoutUtility priorities and Image preferred sizes. Deviation: layout is recomputed every frame from the current values, so a disabled controller's driven values revert (Unity keeps them). TMP/Text/InputField preferred sizes and ScrollRect.SetLayout are not ported; they read as 0 and `ui_placement` lists any that are hit (none in the gameplay HUD). A player build serializes every RectTransform's m_LocalPosition as 0; one under a non-RectTransform parent takes localPosition.xy from its anchoredPosition. That places the HUD Camera's StyleCanvas top-right and GunCanvas bottom-left; `ui_placement` checks them at five resolutions (at 5:4 both clip at the screen edge, as Unity's fixed vertical fov 90 gives). HUD scripts (`hud.rs`): PlayerActivatorRelay (0.2 s staggered activation from PlayerActivator, gun panel only with weapons + weaponIcons, crosshair always), HudOpenEffect (scale/sizeDelta open and reverse/close), HealthBar (hp count-up, after-image, anti-hp slider, F0 text, colour rules), StaminaMeter (intro ramp, three sliders, full flash, empty/charging colours), ColorBlindSettings + ColorBlindSetter prefs (`hudColor.*`) + ColorBlindGet, HudController (hudType classic/alt, weaponIcons/armIcons/styleMeter/styleInfo, hudBackgroundOpacity), HUDPos (weaponHoldPosition), GunControl.UpdateWeaponIcon, NewMovement's screenHud death/respawn toggle and HUD/hudCam velocity sway (the HUD Camera moves per frame). LevelStatsEnabler (rank / secret-mission gating from the save slot, Tab hold and double-tap keep-open, LevelStatsTutorial message); HudMessageReceiver + HudMessage (pref gates, ShowText typewriter, timed / automatic timer, action key names from InputActions + Binds.json); StyleHUD meter visibility (hidden until a combo); RailcannonMeter (CheckStatus from the save's rai gear, weapon.rai prefs and weapons; meter colours / fill); LevelStats (level name from StockMapInfo, StatsManager time `m:ss.fff`, kills, style, rich-text ranks, secrets filled from the save's secretsFound, assists; challenge always NO until ChallengeManager is ported); save data is read from `Saves/Slot<n>/*.bepis` (MS-NRBF, `uk-assets/src/nrbf.rs`) and PlayerPrefs from the registry, read-only (PlayerPrefs writes stay in memory). Not yet: antiHp (hard damage) is 0, fist icon fill, SetAlwaysOnTop TMP material swap, Crosshair, WeaponHUD, StyleHUD scoring, Speedometer, PowerUpMeter UI, UI audio. Legacy Text not meshed yet Playtest: no style meter; the death screen lacks the skull icon and text effects. | RectTransform layout, Image (sliced/filled), masks, layout groups, CanvasGroup, TMP SDF text from the game's font atlases, EventSystem navigation (mouse + gamepad) | layout rects vs oracle dump; every button reachable |
| **Player** | NewMovement, weapons, arms | movement (tested), piercer, partial punch Playtest: only the revolver and Feedbacker exist; no freeze frames (hit-stop); item pickup (picking up and carrying items) not ported; the DualWield power-up does nothing. | All 15 gun variants + alt fire, coins/ricoshots, Feedbacker/Knuckleblaster/Whiplash, hard damage, healing, death, weapon switching/order, variation colours | per-weapon unit tests from decompiled formulas + oracle traces |
| **Enemies** | 43 types incl. bosses | 3 partial Playtest: enemy projectiles are placeholders; no ragdolls or death animations; bosses spawn but do little else; many enemies only partly ported. | Port each `EnemyType`; EnemyIdentifier buffs, radiance, sand, puppets, blessings, idols | per-enemy behaviour probes (attack cadence, damage, health) |
| **Style / ranks** | StyleHUD, 61 bonuses, StatsManager | none | Style meter, multipliers, freshness, level ranks (time/kills/style/P) | scripted kills → expected points |
| **Game flow** | intro, main menu, level select, results, saves, options, cheats, sandbox, Cyber Grind, terminals/shop, secrets, encore, prime sanctums, challenges | level load + level end Playtest: no menus, difficulty selection or settings; no secret orbs, overheal orbs, skull keys / pedestals, touchscreens or other interactables; no level intro text; no cutscenes; Cyber Grind and the special levels not ported. | Port GameStateManager, OptionsManager, GameProgressSaver (MS-NRBF `.bepis` read), PrefsManager, cheats, sandbox, EndlessGrid | menu graph walk; save round-trip on a *copy* |
| **Input** | Input System action maps (`InputActions.cs`), rebinding, gamepad, rumble | hard-coded keys | Read action maps from the game, rebinding UI, gilrs gamepad + rumble | every action bound; rebind round-trip |
| **Level scripts** | 744 unported classes | 21 progression scripts Playtest: no bounce pads. | Burn down the `parity/scripts.tsv` list, ordered by scene reach | per-level probes: every arena/door/checkpoint, bot completion |
| **Video** | 5 VideoPlayers, 2 clips | none | Decode clip format in-process | frame count / duration |
| **Portals** | `ULTRAKILL.Portal` (47 files), portal shaders | none Playtest: 8-x (Fraud) non-Euclidean spaces and gravity changes not ported. | Render-to-texture portal cameras + teleport/physics through portals (7-x, 8-x) | traversal probes |

## 4. Architectural limits (the only allowed deviations)

1. **Physics solver.** Unity's PhysX wrapper (contact offsets, solver ordering, sleep thresholds, CCD) is closed source.
   Linking PhysX 4.1 itself gets the same core algorithms; the wrapper layer is matched from settings read from
   `globalgamemanagers` and checked against oracle traces. Bit-exact equality is not achievable; agreement within a
   stated tolerance is. The player controller is our own deterministic code already, so movement tech (dash, slide, slam,
   wall jumps, SSJ) stays exact to the decompiled formulas regardless.
2. **Unity engine internals are closed source:** Mecanim blending, Shuriken simulation, NavMeshAgent steering, the
   audio mixer DSP (FMOD inside Unity), uGUI layout rounding. These are reimplemented from Unity's documented behaviour
   and verified against the oracle. Expect small numeric differences, for example in filter curves, particle noise
   sequences, and crowd avoidance.
3. **Shader translation.** Found while doing it:
   - Unity strips DXBC RDEF reflection from builds and its parameter lists name only members a variant references, so a
     few fixed-layout members are never named anywhere (`StandardProperties` @0 and @32). They are named by how the
     shader uses them (`uk_assets::shader::KNOWN_MEMBERS`, each entry with its evidence). `_Color` @16 is named by the
     D3D11 lists.
   - Unity's Vulkan stage pairs may read fragment inputs the vertex stage never writes (undefined in Vulkan); WebGPU
     forbids that, so the missing outputs are added and written as zero.
   - Global buffers whose producers aren't ported yet (`_CausticVolumeData`) are bound zeroed.
   - Combined image-samplers (sprites, legacy particles) are split into image + sampler before naga.
   Original text: The original SPIR-V runs through naga, which can reject some constructs. If it does, that
   shader gets a hand-written WGSL port, verified by numeric frame diff. Exact rasterisation differences between D3D11
   and wgpu backends (MSAA resolve, derivative precision) are out of our control.
4. **Steam / platform services** (achievements, leaderboards, Cyber Grind scores, Workshop maps, Discord presence) need
   Steamworks running under ULTRAKILL's app id and would write to the user's real account. They are implemented behind
   a switch that stays **off** unless the user turns it on.
5. **Saves.** We can read ULTRAKILL's `.bepis` saves (MS-NRBF). We never write to the game's save folder; ULTRAKRUST keeps its own.
6. **Nothing from the game ships in this repo.** Every asset is read from the user's install at runtime. This limits
   distribution, not parity.

## 5. Verification: the harness

`cargo run --release -p uk-harness [-- --full] [-- --bless]`. Headless only, no screenshots. It prints only failures,
regressions and gaps, writes `parity/report.tsv`, and exits non-zero when any metric regresses against
`parity/baseline.tsv`.

- **Checks** (`pass.*`): behavioural probes that must stay green. Today: the 0-1 suite above, all scenes load.
- **Coverage** (`cov.*`, higher is better): script classes/instances ported, engine-system mean. These may never drop.
- **Play** (`play.*`): bot progress and deaths per level.
- **Perf** (`perf.*`, lower is better, with noise tolerance): scene load ms, game build ms, sim tick p50/p99.
  Known bottlenecks: 0-1 scene load **2.2–3.6 s** (target < 1 s, by caching decoded bundles and loading lazily), tick spikes
  up to **23–36 ms** (p99 1.3 ms; cause not yet profiled). Playtest (2026-10-08): 8-3 is laggy throughout, and so
  is 0-2's first arena; neither is profiled yet.
- **Determinism:** two independent loads (fresh HashMap seeds) run 90 s of bot play and must match tick for tick.
  Probes and the harness share `uk_game::bot::drive`, so every tool runs the same run.

**The oracle (proposed, needs your go-ahead).** A small BepInEx recorder plugin inside the real game would replay a
scripted input file and write per-tick traces: player state, enemy state, events, sounds played, animator states, and
UI rects. The harness would then diff ULTRAKRUST against those traces, which is the only way to prove "how the game
feeds back to the player" numerically instead of by eye. It means installing BepInEx into the ULTRAKILL folder. That
folder is untouched today, and BepInEx is easy to remove.

## 6. Work order

Each phase ends when its harness metrics are green and added to the baseline. Coverage numbers only go up.

1. **Foundation (done):** harness, determinism fix, coverage registry, baseline.
2. **Unblock play:** ~~NavMesh reader + agents~~ (done: Detour v16 reader, polygon graph, A*, funnel, off-mesh links;
   Filth and Strays re-path with `TrackTick`/`SetDestination`). Still open here: agent avoidance (enemies can stack),
   NavMeshObstacle carving, area costs, sibling order fix, ~~Animator + AnimationClip~~ (done: Mecanim runtime; Filth
   bites and Stray throws are timed by clip events; still open: root motion, IK, additive layers, humanoid muscles, V2 /
   script-driven animators; the viewmodel Revolver and Arm Blue animators are fed from Shoot / Punch), Rigidbody dynamics (gibs, physics props, knockback).
3. **Look (next):** ~~outline buffer + OutlinePx composite~~ (done). ~~Hurt flash + DEAD~~ (done). ~~Keyword variants + WICKED~~ (done); ~~Prefs: pixelization, color compression, dithering, gamma, warping, FOV~~ (done; palette open); ~~Water/UnderwaterController~~ (done); ~~PowerUpMeter/DualWield (VIGNETTE)~~ (done; duplicated weapon open); ~~the death screen~~ (done; sprites open); next: a uGUI sprite path (death skull, power-up meter, HUD). PostProcessV2 as ULTRAKILL wires it (`PostProcessV2_Handler`): the main camera renders into
   color (ARGB32) + RG16 + view normal (the Master shader's 3 outputs) + depth; command buffers run the heat-wave blit and
   the 4-pass outline shader; the PostProcessV2 shader composites with `_Dither`, `_PaletteTex`, `_ColorPrecision` 2048,
   `_VignetteTex`, `_VirtualRes` (pixelization). Our gamma target + composite already has this shape.
3. **Senses:** audio pipeline + MusicManager; SPIR-V shaders + lightmaps + lights + fog; ParticleSystem.
4. **Player complete:** every weapon/variant/arm, coins, style meter, ranks, HUD as uGUI.
5. **UI + flow:** uGUI/TMP interpreter, main menu, options, level select, results, saves (read), cheats, sandbox, Cyber Grind.
6. **Content burn-down:** enemies by first appearance (0-x → P-2), then per-level scripts down `parity/scripts.tsv`
   until every scene has 0 unported classes; one bot route and completion check per level.
7. **Human testing.** Only once the harness is green on every level.

## 7. Known bugs (from playtesting)

Reported from the user's own playtesting on 2026-10-08 and not yet reproduced by a probe. Each one gets a probe before
it is fixed, and moves to MODLOG.md when it is. Missing features from the same notes are in the §3 rows.

| Area | Bug | Where |
|---|---|---|
| Enemies | Some enemies spawn frozen | 0-1: a Stray in a hallway near the end of the level |
| Enemies | Every enemy spawner loads with a Cerberus pedestal | all levels with spawners |
| Enemies | Crash when fighting Swordsmachine | Swordsmachine fights |
| Level scripts | Breakable glass breaks from every damage type; the original filters by type | all glass |
| Level scripts | 4-2 sand does not damage the player | 4-2 |
| Level scripts | The Swordsmachine cutscene does not trigger | 0-2 |
| Animation | Presses do not animate | 0-2 |
| Physics | Gaps in level collision or geometry | 0-3: the Swordsmachine room at the end |
| Rendering | Draw-order errors in some shaders | the checkpoint, the weapon terminal screen, water |
| Rendering | The 3D skybox shader draws white | 1-3: the first red-path arena |
| Rendering | Outdoor light bleeds into the start area | start areas (level not noted) |
| Weapons | The Piercer charge-up effect is always on and doesn't follow the charge | revolver |
| UI | A debug overlay shows at level end | level end |
| UI | The level indicator at the level start shows the wrong level | level start |
| Particles / FX | Some surfaces destroy hit particles almost at once | surfaces not noted |
