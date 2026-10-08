//! Parity registry: which ULTRAKILL script classes have a port in this crate, and how far.
//! The `parity_scripts` probe checks every scene in the install against this list.

/// How complete a port is. `Partial` = runs, but known behaviour is missing (see note).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Port {
    Full,
    Partial(&'static str),
    /// Read for data only (e.g. tuning values); the behaviour itself is not ported.
    DataOnly,
}

pub const PORTED: &[(&str, Port)] = &[
    // progression / level scripts (scripts.rs)
    ("ObjectActivator", Port::Full),
    ("ObjectActivationCheck", Port::Full),
    ("Door", Port::Partial("Normal and BigDoorController types; SubDoorController not ported; no sounds")),
    ("BigDoor", Port::Partial("no sounds, openLight or screen shake")),
    ("DoorController", Port::Full),
    ("DoorOpener", Port::Full),
    ("ActivateArena", Port::Partial("forEnemy (Enemy-tagged trigger) arenas never activate")),
    ("ArenaStatus", Port::Full),
    ("ActivateNextWave", Port::Full),
    ("Breakable", Port::Partial("breakParticle (bounds centre, lossy scale, customPositionRotation); no debris/sound")),
    ("Glass", Port::Partial("shatterParticle; no shards/sound")),
    ("CheckPoint", Port::Partial("no rooms reset beyond snapshot; no sound/anim")),
    ("DeathZone", Port::Full),
    ("TeleportPlayer", Port::Full),
    ("TeleportFinalPit", Port::Full),
    ("PlayerActivator", Port::Full),
    ("OnLevelStart", Port::Partial("hideFogUntilStart / levelNameOnStart / music not applied")),
    ("GetPlayerPref", Port::Full),
    ("PowerUpMeter", Port::Partial("juice meter drives VIGNETTE, endEffect; no meter UI")),
    ("DualWieldPickup", Port::Partial("grants DualWield juice; no duplicated weapon, pickup effect, camera shake")),
    ("DisablePowerUp", Port::Full),
    ("DeathSequence", Port::Partial("deathness + log + EndSequence BlackScreen; no LaughingSkull/Flash/ISeeYou sprites, audio, pitch")),
    ("TextAppearByLines", Port::Full),
    ("FinalDoor", Port::Partial("no animation/sound")),
    ("FinalDoorOpener", Port::Full),
    ("FinalPit", Port::Full),
    ("HudMessage", Port::Partial("no message sound; legacy InputManager key names print the input name")),
    ("HudMessageReceiver", Port::Partial("no message sound")),
    ("WeaponPickUp", Port::Partial("revolver only")),
    ("OutOfBoundsTargetSetter", Port::Full),
    ("UltrakillEvent", Port::Full),
    ("ClimbStep", Port::Full),
    // player (uk-core)
    ("NewMovement", Port::Partial("no hurt/death anim, sounds, slope/jump pad interplay (water forces ported)")),
    ("GroundCheck", Port::Full),
    ("WallCheck", Port::Full),
    ("CameraController", Port::Partial("no screenshake, no options")),
    ("Revolver", Port::Partial("piercer variant only; no coin/alt variants, no sound; viewmodel anim fed (Shoot/ChargeShoot)")),
    ("Punch", Port::Partial("Feedbacker basics; no Knuckleblaster; viewmodel Jab/Jab2 fed")),
    // enemies (enemy.rs)
    ("EnemyIdentifier", Port::Partial("damage/limb multipliers; no buffs, blessing, radiance, sand")),
    ("Zombie", Port::Partial("navmesh pursuit (TrackTick); no anim, no agent avoidance")),
    ("ZombieMelee", Port::Partial("no anim timing")),
    ("ZombieProjectiles", Port::Partial("no anim timing; Flee is a straight back-off (no destination sampling)")),
    ("SpiderBody", Port::Partial("Malicious Face: no anim, no enraged phase visuals, no head pitch (SetFollowHeadRotation), no BreakCorpse")),
    ("MaliciousFace", Port::Partial("see SpiderBody")),
    ("Projectile", Port::Partial("basic straight projectile")),
    ("SwingCheck2", Port::DataOnly),
    // hit effects (particles.rs)
    ("BloodsplatterManager", Port::Partial("pools + GetGore; no underwater/sand/blessed gore (eid flags not tracked), no bloodstain decals")),
    ("Bloodsplatter", Port::Partial("play on enable, heal sphere, repool when stopped; particle collisions only counted (no stains, no sound)")),
    ("BloodUnderwaterChecker", Port::Full),
    ("RemoveOnTime", Port::Partial("effect prefabs and scene objects; useAudioLength falls back to time (no audio)")),
    ("SpawnEffect", Port::Partial("bubble shrink, light enable + range fade, particles; no spawn sound")),
    ("DestroyOnCheckpointRestart", Port::Partial("effect prefabs only; scene objects come back with the checkpoint snapshot")),
    ("EnviroGibModifier", Port::Full),
    ("PortalAwareParticleSystem", Port::Partial("environment raycast kill; no portals")),
    // uGUI (ugui.rs, tmp.rs)
    ("Image", Port::Partial("Simple/Sliced/Tiled/Filled; useSpriteMesh drawn as a quad; no layout element sizes")),
    ("RawImage", Port::Full),
    ("Mask", Port::Full),
    ("RectMask2D", Port::Full),
    ("Shadow", Port::Full),
    ("Outline", Port::Full),
    ("CanvasScaler", Port::Full),
    ("TextMeshProUGUI", Port::Partial("no Ellipsis/Page/Linked overflow, sprites, underline/strikethrough meshes, rare tags; no preferred sizes")),
    ("Slider", Port::Partial("value -> fill/handle anchors and Filled amount; no dragging/navigation")),
    ("Button", Port::Partial("ColorTint normal/disabled only; no EventSystem")),
    ("Text", Port::Partial("not meshed (Unity's native TextGenerator)")),
    // HUD (hud.rs)
    ("PlayerActivatorRelay", Port::Full),
    ("HudOpenEffect", Port::Full),
    ("HealthBar", Port::Partial("NewMovement.antiHp (hard damage) not ported: always 0")),
    ("StaminaMeter", Port::Partial("no flash sound")),
    ("ColorBlindGet", Port::Full),
    ("ColorBlindSettings", Port::Partial("HUD and variation colours; enemy colours unused")),
    ("ColorBlindSetter", Port::Partial("prefs applied at load (ColorBlindActivator.Start); no options menu")),
    ("ColorBlindActivator", Port::Full),
    ("HudController", Port::Partial("no fist fill (WeaponCharges.punchStamina), no SetAlwaysOnTop material swap, no HideUI cheat")),
    ("HUDPos", Port::Full),
    ("LevelStatsEnabler", Port::Full),
    ("StyleHUD", Port::Partial("meter visibility only (comboActive || forceMeterOn); no style points, ranks, freshness")),
    ("RailcannonMeter", Port::Partial("raicharge is 0 until the railcannon is ported")),
    ("Crosshair", Port::Partial("crossHairColor 0's invertMaterial is not swapped in; no options menu (prefs at load)")),
    ("FadeOutBars", Port::Full),
    ("SliderToFillAmount", Port::Full),
    ("LevelStats", Port::Partial("challenge always NO (ChallengeManager not ported), style 0 (StyleHUD), no cyber grind wave text")),
];

pub fn status(class: &str) -> Option<Port> {
    PORTED.iter().find(|(c, _)| *c == class).map(|(_, p)| *p)
}

/// Unity native component/asset types that matter for parity, with how much of each is implemented (0..1).
/// Weighted by instance count in the harness `--full` run.
pub const NATIVE: &[(i32, &str, f64)] = &[
    (4, "Transform", 1.0),
    (23, "MeshRenderer", 0.6),         // drawn; no Unity shaders, lightmaps or light probes
    (33, "MeshFilter", 1.0),
    (137, "SkinnedMeshRenderer", 0.7), // skinned from the animated bones each frame; no blend shapes
    (65, "BoxCollider", 1.0),
    (64, "MeshCollider", 1.0),
    (135, "SphereCollider", 0.8),
    (136, "CapsuleCollider", 0.8),
    (54, "Rigidbody", 0.1), // only the player; no rigidbody dynamics
    (153, "ConfigurableJoint", 0.0),
    (82, "AudioSource", 0.0),
    (83, "AudioClip", 0.0),
    (169, "AudioLowPassFilter", 0.0),
    (170, "AudioDistortionFilter", 0.0),
    (165, "AudioHighPassFilter", 0.0),
    (167, "AudioReverbZone", 0.0),
    (108, "Light", 0.05),
    (157, "LightmapSettings", 0.0),
    (104, "RenderSettings", 0.1),
    (198, "ParticleSystem", 0.75), // effect prefabs + scene systems: every module the campaign enables except sub-emitters, lights, external forces, custom data; Play/Stop/Clear calls, stopAction; RNG not Unity's
    (199, "ParticleSystemRenderer", 0.6), // billboard, stretched, horizontal/vertical, mesh; texture sheet; sortingFudge and culling ignored
    (96, "TrailRenderer", 0.0),
    (120, "LineRenderer", 0.05),
    (212, "SpriteRenderer", 0.0),
    (95, "Animator", 0.5), // state machines, transitions, blend trees, layers, clip events; Filth/Stray driven; no root motion, IK, additive layers, humanoid muscles
    (74, "AnimationClip", 0.8), // streamed/dense/constant transform curves + events; non-transform bindings unapplied
    (91, "AnimatorController", 0.85), // fully decoded; override controllers + state machine behaviours not run
    (195, "NavMeshAgent", 0.3), // path following via TrackTick/SetDestination; walking agents stay on the mesh; no avoidance, off-mesh links walked, no autoBraking
    (208, "NavMeshObstacle", 0.0),
    (238, "NavMeshData", 0.8),  // tiles + off-mesh links decoded and queried; area costs/masks not applied
    (224, "RectTransform", 0.8), // anchors, pivots, scaler; no layout groups/fitters
    (222, "CanvasRenderer", 0.75), // meshes drawn with the game's UI shaders, stencil, clip rect; TMP text, no legacy Text
    (223, "Canvas", 0.7), // overlay + world modes, sorting, override sorting; camera mode drawn as overlay
    (225, "CanvasGroup", 0.8), // alpha; interactable/blocksRaycasts unused (no EventSystem)
    (20, "Camera", 0.3),
    (48, "Shader", 0.0),
    (21, "Material", 0.4), // main texture + colour only
    (28, "Texture2D", 0.95),
    (89, "Cubemap", 0.0),
    (328, "VideoPlayer", 0.0),
];

pub fn native_status(class_id: i32) -> Option<f64> {
    NATIVE.iter().find(|(id, _, _)| *id == class_id).map(|(_, _, w)| *w)
}
