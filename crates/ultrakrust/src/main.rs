//! ULTRAKRUST: Bevy frontend for `uk-core` / `uk-game`.
//!
//! `ultrakrust` plays level 0-1 from your ULTRAKILL install (if found);
//! `ultrakrust --level 1-1` picks another level; `ultrakrust --sandbox` opens the test map;
//! `ultrakrust --tour <dir>` saves screenshots of every room and exits;
//! `--all-weapons` starts with every ported weapon unlocked (kept across level changes).
//! The exit elevator's results continue to the next level on Fire1, as in ULTRAKILL.
//!
//! Controls (ULTRAKILL defaults): WASD move, Space jump, Left Shift dash,
//! Left Ctrl slide / slam, LMB fire, hold RMB to charge a piercing shot, F punch (parries).
//! R restart from checkpoint, N noclip, [ ] sensitivity, T camera tilt, Esc release mouse.

mod unity_render;
mod present_probe;
mod demo;
mod level;
mod map;
mod tour;

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::light::NotShadowCaster;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, WindowResolution};
use std::sync::Arc;
use uk_core::camera::FpCamera;
use uk_core::collide::World;
use uk_core::consts::FIXED_DT;
use uk_core::player::{Event, Input as PInput, Player};
use uk_core::revolver::{Revolver, Shot};
use uk_game::{Game, GameEvent};

const SANDBOX_SPAWN: Vec3 = Vec3::new(0.0, 1.5, 100.0);
const VIEW_LAYER: usize = 1;

#[derive(Resource)]
struct Sim {
    game: Game,
    cam: FpCamera,
    revolver: Revolver,
    /// Input latched for the fixed step (held state only).
    fixed_input: PInput,
    clock: f64,
    log: Vec<(String, f32)>,
    banner: Option<(String, f32)>,
    recoil: f32,
    punch_cd: f32,
    punch_anim: f32,
    noclip: bool,
    title: String,
    sandbox_targets: Vec<map::Target>,
    level_time: f32,
    /// FinalRank: seconds since the results came up, and whether Fire1 skipped the tally.
    results_t: f32,
    results_skipped: bool,
    /// A scene to load (bundle name, e.g. "0-2") and how many frames the LOADING card has shown.
    load_request: Option<(String, u32)>,
    /// Bumped per level load so the Unity renderer rebuilds its GPU state.
    generation: u64,
    unity_shaders: bool,
}

/// FinalRank's tally: one line per step, then complete.
const TALLY_STEP: f32 = 0.5;
const TALLY_LINES: f32 = 4.0;

/// SceneHelper.LoadScene names ("Level 0-2", "Level 4-S") -> campaign bundle names ("0-2", "4-s").
fn scene_bundle(scene: &str) -> String {
    scene.strip_prefix("Level ").unwrap_or(scene).to_lowercase()
}

#[derive(Component)]
struct MainCam;
#[derive(Component)]
struct ViewModel;
#[derive(Component)]
struct FistModel;
#[derive(Component)]
struct HudText;
#[derive(Component)]
struct HintText;
#[derive(Component)]
struct BannerText;
#[derive(Component)]
struct StaminaBar(usize);
#[derive(Component)]
struct HpBar;
#[derive(Component)]
struct HpText;
#[derive(Component)]
struct PierceBar;
#[derive(Component)]
struct Beam {
    life: f32,
    max: f32,
}

#[derive(Resource)]
struct BeamAssets {
    mesh: Handle<Mesh>,
    normal: Handle<StandardMaterial>,
    pierce: Handle<StandardMaterial>,
}

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "ULTRAKRUST".into(),
                resolution: WindowResolution::new(1600, 900),
                present_mode: PresentMode::AutoNoVsync,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(Time::<Fixed>::from_seconds(FIXED_DT as f64))
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.03)))
        .insert_resource(GlobalAmbientLight { brightness: 350.0, ..default() })
        .init_resource::<level::LevelView>()
        .add_plugins(unity_render::UnityRenderPlugin)
        .add_systems(Startup, setup)
        .add_systems(Last, exit_after)
        .add_systems(Update, present_probe::run.after(update_hud).run_if(present_probe::enabled))
        .add_systems(FixedUpdate, fixed_sim)
        .add_systems(Update, (cursor_grab, exit_probe, frame_sim, change_level, sync_level, unity_frame, apply_view, update_beams, update_hud, tour::run_tour).chain())
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut shaders: ResMut<Assets<bevy::shader::Shader>>,
) {
    let args: Vec<String> = std::env::args().collect();
    // ULTRAKILL's own shaders by default; `--legacy-render` keeps the old Bevy-PBR stand-in
    let unity_shaders = !args.iter().any(|a| a == "--legacy-render");
    let level_arg = args.iter().position(|a| a == "--level").and_then(|i| args.get(i + 1)).cloned();
    let want_level = !args.iter().any(|a| a == "--sandbox");
    let all_weapons = args.iter().any(|a| a == "--all-weapons");
    let tour_dir = args.iter().position(|a| a == "--tour").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);

    let mut loaded: Option<level::Loaded> = None;
    if want_level {
        let name = level_arg.unwrap_or_else(|| "0-1".into());
        match level::load(&name, &mut commands, &mut meshes, &mut materials, &mut images, unity_shaders.then_some(&mut *shaders), 1) {
            Ok(l) => {
                info!("{}", l.summary);
                loaded = Some(l);
            }
            Err(e) => warn!("could not load level {name}: {e}; falling back to the sandbox"),
        }
    }
    let is_level = loaded.is_some();
    let (mut game, title, rooms, sandbox_targets) = match loaded {
        Some(l) => {
            commands.insert_resource(l.view);
            if let Some(scene) = l.unity {
                commands.insert_resource(unity_render::UnityScene(Some(std::sync::Arc::new(scene))));
            }
            (l.game, l.summary, l.rooms, Vec::new())
        }
        None => {
            let tex = |images: &mut Assets<Image>, d: [u8; 3], l: [u8; 3], cells| images.add(map::checker(d, l, cells));
            let mats = map::Materials {
                floor: materials.add(StandardMaterial { base_color_texture: Some(tex(&mut images, [38, 36, 34], [52, 49, 46], 4)), perceptual_roughness: 0.95, ..default() }),
                wall: materials.add(StandardMaterial { base_color_texture: Some(tex(&mut images, [60, 26, 24], [78, 34, 30], 2)), perceptual_roughness: 0.9, ..default() }),
                accent: materials.add(StandardMaterial { base_color_texture: Some(tex(&mut images, [70, 70, 76], [96, 96, 104], 2)), perceptual_roughness: 0.8, ..default() }),
                target: materials.add(StandardMaterial { base_color: Color::srgb(0.9, 0.75, 0.2), emissive: LinearRgba::rgb(0.6, 0.4, 0.0), ..default() }),
            };
            let mut world = World::default();
            let targets = map::spawn(&mut commands, &mut meshes, &mats, &mut world);
            world.build();
            let mut game = Game::new(Arc::new(uk_assets::scenedef::SceneDef::default()));
            game.world = world;
            game.s.player = Player::new(SANDBOX_SPAWN);
            game.s.has_revolver = true;
            game.spawn_yaw = 0.0;
            game.rebase_start();
            (game, "movement sandbox".to_string(), Vec::new(), targets)
        }
    };
    if all_weapons {
        give_all_weapons(&mut game);
        info!("--all-weapons: has_revolver={}", game.s.has_revolver);
    }
    let spawn = game.s.player.pos;
    let spawn_yaw = game.spawn_yaw;

    commands.spawn((
        DirectionalLight { illuminance: 9000.0, shadow_maps_enabled: !is_level, ..default() },
        Transform::from_xyz(40.0, 120.0, 60.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // World camera + view-model camera (drawn on top, own FOV like ULTRAKILL's HUD camera).
    commands
        .spawn((
            MainCam,
            Camera3d::default(),
            Projection::from(PerspectiveProjection { fov: 105f32.to_radians(), near: 0.05, ..default() }),
            DistanceFog {
                color: Color::srgb(0.02, 0.02, 0.03),
                falloff: if is_level { FogFalloff::Linear { start: 250.0, end: 900.0 } } else { FogFalloff::Linear { start: 120.0, end: 320.0 } },
                ..default()
            },
            Transform::from_translation(spawn),
        ))
        .insert_if(
            // ULTRAKILL's own shaders: no tonemapping or MSAA (the game has neither), and the view is
            // composited from the gamma-space scene target.
            // The HUD is drawn straight onto this view after the composite: no second camera on
            // the window, whose own target would cover the composite and keep stale UI.
            (unity_render::UnityCamera, bevy::core_pipeline::tonemapping::Tonemapping::None, Msaa::Off, IsDefaultUiCamera),
            || unity_shaders && is_level,
        )
        .with_children(|c| {
            // ULTRAKILL's own viewmodel (layer 13, HUD Camera) replaces the placeholder view-model
            // camera on the Unity path
            if unity_shaders && is_level {
                return;
            }
            c.spawn((
                Camera3d::default(),
                Camera { order: 1, ..default() },
                Projection::from(PerspectiveProjection { fov: 90f32.to_radians(), near: 0.01, ..default() }),
                RenderLayers::layer(VIEW_LAYER),
            ));
            // Placeholder revolver: body + barrel + cylinder.
            let gun = materials.add(StandardMaterial { base_color: Color::srgb(0.55, 0.56, 0.6), metallic: 0.7, ..default() });
            let grip = materials.add(Color::srgb(0.25, 0.12, 0.06));
            c.spawn((ViewModel, Transform::from_xyz(0.32, -0.3, -0.55), Visibility::Hidden, RenderLayers::layer(VIEW_LAYER)))
                .with_children(|g| {
                    for (mesh, mat, t) in [
                        (Cuboid::new(0.07, 0.09, 0.22), gun.clone(), Transform::from_xyz(0.0, 0.0, 0.0)),
                        (Cuboid::new(0.04, 0.04, 0.32), gun.clone(), Transform::from_xyz(0.0, 0.025, -0.22)),
                        (Cuboid::new(0.1, 0.1, 0.1), gun.clone(), Transform::from_xyz(0.0, 0.0, -0.06)),
                        (Cuboid::new(0.06, 0.16, 0.08), grip.clone(), Transform::from_xyz(0.0, -0.1, 0.08).with_rotation(Quat::from_rotation_x(-0.35))),
                    ] {
                        g.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat), t, RenderLayers::layer(VIEW_LAYER), NotShadowCaster));
                    }
                });
            // Placeholder blue fist (Feedbacker)
            let fist = materials.add(StandardMaterial { base_color: Color::srgb(0.25, 0.45, 0.9), metallic: 0.5, ..default() });
            c.spawn((
                FistModel,
                Mesh3d(meshes.add(Cuboid::new(0.07, 0.07, 0.18))),
                MeshMaterial3d(fist),
                Transform::from_xyz(-0.38, -0.42, -0.75),
                RenderLayers::layer(VIEW_LAYER),
                NotShadowCaster,
            ));
        });
    commands.spawn((PointLight { intensity: 3000.0, range: 3.0, ..default() }, Transform::from_xyz(0.5, 0.5, 0.0), RenderLayers::layer(VIEW_LAYER)));

    commands.insert_resource(BeamAssets {
        mesh: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        normal: materials.add(StandardMaterial {
            base_color: Color::srgba(1.0, 0.95, 0.6, 0.9),
            emissive: LinearRgba::rgb(8.0, 6.0, 1.5),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            ..default()
        }),
        pierce: materials.add(StandardMaterial {
            base_color: Color::srgba(0.6, 0.85, 1.0, 0.9),
            emissive: LinearRgba::rgb(2.0, 6.0, 12.0),
            alpha_mode: AlphaMode::Add,
            unlit: true,
            ..default()
        }),
    });

    // HUD
    commands.spawn((
        HudText,
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgba(0.95, 0.95, 0.95, 0.85)),
        Node { position_type: PositionType::Absolute, top: px(10), left: px(12), ..default() },
    ));
    commands.spawn((
        Text::new("+"),
        TextFont { font_size: FontSize::Px(28.0), ..default() },
        Node { position_type: PositionType::Absolute, left: percent(50), top: percent(50), margin: UiRect { left: px(-8), top: px(-18), ..default() }, ..default() },
    ));
    commands.spawn((
        HintText,
        Text::new(""),
        TextFont { font_size: FontSize::Px(22.0), ..default() },
        TextColor(Color::srgb(1.0, 1.0, 1.0)),
        TextLayout::justify(Justify::Center),
        Node { position_type: PositionType::Absolute, bottom: px(140), width: percent(100), justify_content: JustifyContent::Center, ..default() },
    ));
    commands.spawn((
        BannerText,
        Text::new(""),
        TextFont { font_size: FontSize::Px(54.0), ..default() },
        TextColor(Color::srgb(1.0, 0.25, 0.2)),
        TextLayout::justify(Justify::Center),
        Node { position_type: PositionType::Absolute, top: percent(30), width: percent(100), justify_content: JustifyContent::Center, ..default() },
    ));
    commands
        .spawn(Node { position_type: PositionType::Absolute, bottom: px(24), left: px(24), flex_direction: FlexDirection::Column, row_gap: px(6), ..default() })
        .with_children(|ui| {
            ui.spawn((HpText, Text::new("100"), TextFont { font_size: FontSize::Px(26.0), ..default() }, TextColor(Color::srgb(1.0, 0.3, 0.2))));
            ui.spawn((Node { width: px(282), height: px(16), ..default() }, BackgroundColor(Color::srgb(0.15, 0.15, 0.15)))).with_children(|b| {
                b.spawn((HpBar, Node { width: percent(100), height: percent(100), ..default() }, BackgroundColor(Color::srgb(0.85, 0.15, 0.1))));
            });
            ui.spawn(Node { flex_direction: FlexDirection::Row, column_gap: px(6), ..default() }).with_children(|row| {
                for i in 0..3 {
                    row.spawn((Node { width: px(90), height: px(12), ..default() }, BackgroundColor(Color::srgb(0.15, 0.15, 0.15)))).with_children(|b| {
                        b.spawn((StaminaBar(i), Node { width: percent(100), height: percent(100), ..default() }, BackgroundColor(Color::srgb(0.2, 0.75, 1.0))));
                    });
                }
            });
            ui.spawn((Node { width: px(282), height: px(6), ..default() }, BackgroundColor(Color::srgb(0.15, 0.15, 0.15)))).with_children(|b| {
                b.spawn((PierceBar, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(Color::srgb(1.0, 0.85, 0.2))));
            });
        });

    if let Some(dir) = args.iter().position(|a| a == "--demo").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        commands.insert_resource(demo::Demo::new(dir));
    }
    if let Some(dir) = tour_dir {
        let _ = std::fs::create_dir_all(&dir);
        let t = tour::Tour::new(dir, spawn_yaw, &rooms);
        info!("tour: {} shots", t.shots.len());
        commands.insert_resource(t);
    }
    let mut cam = FpCamera::default();
    cam.rotation_y = spawn_yaw;
    commands.insert_resource(Sim {
        game,
        cam,
        revolver: Revolver::default(),
        fixed_input: PInput::default(),
        clock: 0.0,
        log: Vec::new(),
        banner: None,
        recoil: 0.0,
        punch_cd: 0.0,
        punch_anim: 0.0,
        noclip: false,
        title,
        sandbox_targets,
        level_time: 0.0,
        results_t: 0.0,
        results_skipped: false,
        load_request: None,
        generation: 1,
        unity_shaders: unity_shaders && is_level,
    });
}

/// `--all-weapons`: every weapon the port has (the Revolver; the rest of the arsenal is not ported).
fn give_all_weapons(game: &mut Game) {
    game.s.has_revolver = true;
    // the level-start snapshot keeps them through restarts
    game.rebase_start();
}

fn cursor_grab(mut cursor: Single<&mut CursorOptions>, mouse: Res<ButtonInput<MouseButton>>, key: Res<ButtonInput<KeyCode>>) {
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None {
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
    }
    if key.just_pressed(KeyCode::Escape) {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

fn move_axis(keys: &ButtonInput<KeyCode>) -> Vec2 {
    let mut v = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        v.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) {
        v.y -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) {
        v.x += 1.0;
    }
    if keys.pressed(KeyCode::KeyA) {
        v.x -= 1.0;
    }
    // Unity's composite Move action is normalized
    v.normalize_or_zero()
}

fn fixed_sim(mut sim: ResMut<Sim>) {
    let sim = &mut *sim;
    if sim.noclip {
        return;
    }
    let input = sim.fixed_input;
    sim.game.fixed_update(&input);
}

#[allow(clippy::too_many_arguments)]
fn frame_sim(
    mut sim: ResMut<Sim>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions>,
    time: Res<Time>,
    mut commands: Commands,
    beams: Res<BeamAssets>,
    mut target_tf: Query<&mut Transform, (Without<MainCam>, Without<Beam>)>,
    demo: Option<ResMut<demo::Demo>>,
    mut exit: MessageWriter<AppExit>,
) {
    let dt = time.delta_secs();
    let sim = &mut *sim;
    let bot = match demo {
        Some(mut d) => d.tick(&mut sim.game, dt, &mut commands, &mut exit),
        None => None,
    };
    sim.clock += dt as f64;
    let captured = cursor.grab_mode != CursorGrabMode::None;

    // NewMovement.levelOver: no restarting from the exit elevator
    if keys.just_pressed(KeyCode::KeyR) && !sim.game.s.level_complete {
        sim.game.respawn();
        sim.cam.rotation_y = sim.game.s.player.yaw_deg;
    }
    if keys.just_pressed(KeyCode::BracketLeft) {
        sim.cam.sensitivity *= 0.8;
    }
    if keys.just_pressed(KeyCode::BracketRight) {
        sim.cam.sensitivity *= 1.25;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        sim.cam.tilt_enabled = !sim.cam.tilt_enabled;
    }
    if keys.just_pressed(KeyCode::KeyN) {
        sim.noclip = !sim.noclip;
        sim.game.s.player.vel = Vec3::ZERO;
    }

    let axis = bot.map(|b| b.input.move_axis).unwrap_or_else(|| move_axis(&keys));
    let input = match bot {
        Some(b) => b.input,
        None => PInput {
            move_axis: axis,
            jump_pressed: keys.just_pressed(KeyCode::Space),
            jump_held: keys.pressed(KeyCode::Space),
            slide_pressed: keys.just_pressed(KeyCode::ControlLeft),
            slide_released: keys.just_released(KeyCode::ControlLeft),
            dash_pressed: keys.just_pressed(KeyCode::ShiftLeft),
        },
    };
    sim.fixed_input = PInput { move_axis: axis, jump_held: input.jump_held, ..default() };

    // Camera input is locked while V1 falls into the level (GameState "pit-falling").
    let can_look = captured && (sim.game.s.player.activated || sim.noclip);
    if let Some(b) = bot {
        sim.cam.rotation_y = b.yaw_deg;
        sim.cam.rotation_x = b.pitch_deg;
    } else if can_look {
        sim.cam.look(motion.delta);
    }
    // FinalPit turns the view towards the elevator
    if let Some((yaw, pitch)) = sim.game.s.forced_view {
        sim.cam.rotation_y = yaw;
        sim.cam.rotation_x = pitch;
    }
    sim.game.s.player.yaw_deg = sim.cam.rotation_y;
    sim.game.s.view_pitch = sim.cam.rotation_x;
    let clock = sim.clock;
    if sim.noclip {
        let rot = sim.cam.rotation();
        let mut d = rot * Vec3::new(axis.x, 0.0, -axis.y);
        if keys.pressed(KeyCode::Space) {
            d.y += 1.0;
        }
        if keys.pressed(KeyCode::ControlLeft) {
            d.y -= 1.0;
        }
        let speed = if keys.pressed(KeyCode::ShiftLeft) { 120.0 } else { 35.0 };
        let p = &mut sim.game.s.player;
        p.pos += d.normalize_or_zero() * speed * dt;
        p.prev_pos = p.pos;
    } else {
        sim.game.update(&input, dt, clock);
    }
    if !sim.game.s.level_complete && sim.game.s.player.activated {
        sim.level_time += dt;
    }
    let strafe = axis.x;
    sim.cam.late_update(&mut sim.game.s.player, strafe, dt);

    for e in sim.game.s.player.events.drain(..) {
        let msg = match e {
            Event::Ssj { gained } => format!("SUPER SLIDE JUMP +{gained:.0} u/s"),
            Event::SlamJump => "SLAM JUMP".into(),
            Event::DashJump => "DASH JUMP".into(),
            Event::WallJump(n) => format!("WALL JUMP {n}/3"),
            Event::StaminaFail => "no stamina".into(),
            _ => continue,
        };
        sim.log.push((msg, 2.0));
    }

    // Weapons
    let eye = sim.game.s.player.pos + sim.cam.local_pos;
    let aim = sim.cam.rotation() * Vec3::NEG_Z;
    let alive = !sim.game.s.dead && sim.game.s.player.activated && !sim.game.s.level_complete;
    sim.recoil = (sim.recoil - dt * 6.0).max(0.0);
    sim.punch_cd = (sim.punch_cd - dt).max(0.0);
    sim.punch_anim = (sim.punch_anim - dt * 5.0).max(0.0);
    if let Some(b) = bot {
        if b.fire && sim.game.s.has_revolver && alive {
            sim.recoil = 1.0;
            sim.game.fire_revolver(eye, aim, false);
        }
        if b.punch && alive {
            sim.punch_anim = 1.0;
            sim.game.punch(eye, aim);
        }
    } else if sim.game.s.has_revolver && alive {
        let fire1 = captured && mouse.pressed(MouseButton::Left);
        let shot = sim.revolver.update(fire1, captured && mouse.pressed(MouseButton::Right), mouse.just_released(MouseButton::Right), dt);
        if let Some(shot) = shot {
            sim.recoil = 1.0;
            sim.game.fire_revolver(eye, aim, shot == Shot::Pierce);
        }
    }
    let probe_punch = std::env::var_os("UK_PROBE_PUNCH").is_some();
    if alive && (probe_punch || captured && keys.just_pressed(KeyCode::KeyF)) && sim.punch_cd <= 0.0 {
        // FistControl: fistCooldown = cooldownCost (2) * 0.25
        sim.punch_cd = 0.5;
        sim.punch_anim = 1.0;
        sim.game.punch(eye, aim);
    }

    // Game events -> visuals / HUD
    let events: Vec<GameEvent> = sim.game.events.drain(..).collect();
    for ev in &events {
        if matches!(ev, GameEvent::Hurt(_) | GameEvent::Died | GameEvent::Respawned | GameEvent::Checkpoint | GameEvent::LevelComplete) {
            info!("game event {:?} hp={} pos={:?}", ev, sim.game.s.hp, sim.game.s.player.pos);
        }
    }
    for ev in events {
        match ev {
            GameEvent::Shot { from: _, to, pierce } => {
                let muzzle = eye + sim.cam.rotation() * Vec3::new(0.32, -0.25, -0.9);
                let len = muzzle.distance(to).max(0.01);
                let width = if pierce { 0.35 } else { 0.12 };
                commands.spawn((
                    Beam { life: 0.12, max: 0.12 },
                    Mesh3d(beams.mesh.clone()),
                    MeshMaterial3d(if pierce { beams.pierce.clone() } else { beams.normal.clone() }),
                    Transform::from_translation((muzzle + to) * 0.5).looking_at(to, Vec3::Y).with_scale(Vec3::new(width, width, len)),
                    NotShadowCaster,
                ));
                // sandbox targets
                for t in sim.sandbox_targets.iter_mut() {
                    if let Some(h) = sim.game.world.raycast(eye, aim, 1000.0) {
                        if h.collider.index == t.collider.index && h.collider.group == t.collider.group {
                            t.hits += if pierce { 2 } else { 1 };
                            t.flash = 0.15;
                        }
                    }
                }
            }
            GameEvent::EnemyHit { head: true, .. } => sim.log.push(("HEADSHOT".into(), 1.0)),
            GameEvent::EnemyKilled { .. } => sim.log.push(("KILL".into(), 1.0)),
            GameEvent::Checkpoint => sim.banner = Some(("CHECKPOINT".into(), 1.5)),
            GameEvent::WeaponGot(w) => sim.banner = Some((format!("{w} ACQUIRED"), 2.5)),
            GameEvent::Died => sim.banner = Some(("YOU DIED".into(), 1.6)),
            GameEvent::Respawned => {
                sim.cam.rotation_y = sim.game.s.player.yaw_deg;
                sim.revolver = Revolver::default();
            }
            _ => {}
        }
    }

    // FinalRank: the tally runs line by line (Fire1 skips it); once complete, Fire1 in the second
    // pit loads the next level. A rankless exit (AbruptLevelChanger) loads straight away.
    if sim.game.s.results_shown && sim.load_request.is_none() {
        let fire1 = bot.is_some_and(|b| b.fire) || (captured && mouse.just_pressed(MouseButton::Left)) || std::env::var_os("UK_PROBE_CONTINUE").is_some();
        let complete = sim.results_skipped || sim.results_t >= TALLY_STEP * TALLY_LINES;
        if sim.results_t == 0.0 {
            info!("results up at {:.2}s: ranks {:?}", sim.level_time, sim.game.final_ranks(sim.level_time));
        }
        if fire1 && !complete {
            sim.results_skipped = true;
        } else if fire1 && complete && sim.game.s.reached_second_pit {
            if let Some(next) = sim.game.s.next_level.clone() {
                info!("results: continue -> {next:?} (bundle {})", scene_bundle(&next));
                sim.load_request = Some((scene_bundle(&next), 0));
            }
        }
        sim.results_t += dt;
    }
    if sim.game.s.rankless_continue && sim.load_request.is_none() {
        if let Some(next) = sim.game.s.next_level.clone() {
            sim.load_request = Some((scene_bundle(&next), 0));
        }
    }
    sim.log.retain_mut(|(_, t)| {
        *t -= dt;
        *t > 0.0
    });
    if let Some((_, t)) = &mut sim.banner {
        *t -= dt;
        if *t <= 0.0 {
            sim.banner = None;
        }
    }
    for t in sim.sandbox_targets.iter_mut() {
        t.flash = (t.flash - dt).max(0.0);
        if let Ok(mut tf) = target_tf.get_mut(t.entity) {
            tf.scale = Vec3::splat(1.0 + t.flash * 2.0);
        }
    }
}

/// SceneHelper.LoadScene: after a couple of frames of the LOADING card, the current level's render
/// entities go and the next level replaces the game, its view and the Unity scene (weapons carry over).
#[allow(clippy::too_many_arguments)]
fn change_level(
    mut sim: ResMut<Sim>,
    view: Option<ResMut<level::LevelView>>,
    scene: Option<ResMut<unity_render::UnityScene>>,
    frame: Option<ResMut<unity_render::UnityFrame>>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut shaders: ResMut<Assets<bevy::shader::Shader>>,
    beams: Query<Entity, With<Beam>>,
) {
    let sim = &mut *sim;
    let Some((name, frames)) = &mut sim.load_request else { return };
    *frames += 1;
    if *frames < 3 {
        // let the LOADING card reach the screen before the load blocks
        return;
    }
    let name = name.clone();
    sim.load_request = None;
    let generation = sim.generation + 1;
    let t = std::time::Instant::now();
    let l = match level::load(&name, &mut commands, &mut meshes, &mut materials, &mut images, sim.unity_shaders.then_some(&mut *shaders), generation) {
        Ok(l) => l,
        Err(e) => {
            warn!("could not load level {name}: {e}");
            sim.banner = Some((format!("COULD NOT LOAD {}", name.to_uppercase()), 3.0));
            return;
        }
    };
    info!("level change -> {name} (generation {generation}, {:.1}s): {}; has_revolver carried {}", t.elapsed().as_secs_f32(), l.summary, sim.game.s.has_revolver);
    for e in beams.iter() {
        commands.entity(e).despawn();
    }
    match view {
        Some(mut view) => {
            for e in view.node_entities.values().flatten().map(|(e, _)| *e).chain(view.projectiles.iter().copied()) {
                commands.entity(e).despawn();
            }
            *view = l.view;
        }
        None => commands.insert_resource(l.view),
    }
    if let Some(s) = l.unity {
        let s = Some(Arc::new(s));
        match scene {
            Some(mut scene) => scene.0 = s,
            None => commands.insert_resource(unity_render::UnityScene(s)),
        }
    }
    if let Some(mut frame) = frame {
        *frame = default();
    }
    let mut game = l.game;
    if sim.game.s.has_revolver {
        // weapons are unlocks (GameProgressSaver), not per-level state
        give_all_weapons(&mut game);
    }
    let (sensitivity, tilt) = (sim.cam.sensitivity, sim.cam.tilt_enabled);
    sim.cam = FpCamera::default();
    sim.cam.sensitivity = sensitivity;
    sim.cam.tilt_enabled = tilt;
    sim.cam.rotation_y = game.spawn_yaw;
    sim.game = game;
    sim.generation = generation;
    sim.title = l.summary;
    sim.revolver = Revolver::default();
    sim.fixed_input = PInput::default();
    sim.log.clear();
    sim.banner = None;
    sim.level_time = 0.0;
    sim.results_t = 0.0;
    sim.results_skipped = false;
    sim.recoil = 0.0;
    sim.punch_cd = 0.0;
    sim.punch_anim = 0.0;
}

fn sync_level(
    mut sim: ResMut<Sim>,
    mut view: ResMut<level::LevelView>,
    mut commands: Commands,
    mut vis: Query<&mut Visibility>,
    mut tfs: Query<&mut Transform, (Without<MainCam>, Without<ViewModel>, Without<FistModel>)>,
) {
    let sim = &mut *sim;
    level::sync(&mut sim.game, &mut view, &mut commands, &mut vis, &mut tfs);
}

#[allow(clippy::type_complexity)]
fn apply_view(
    mut sim: ResMut<Sim>,
    fixed: Res<Time<Fixed>>,
    tour: Option<Res<tour::Tour>>,
    mut cam: Single<(&mut Transform, &mut Projection), With<MainCam>>,
    vm: Option<Single<(&mut Transform, &mut Visibility), (With<ViewModel>, Without<MainCam>)>>,
    fist: Option<Single<&mut Transform, (With<FistModel>, Without<MainCam>, Without<ViewModel>)>>,
) {
    let alpha = fixed.overstep_fraction();
    let pos = sim.game.s.player.interpolated_pos(alpha) + sim.cam.local_pos;
    cam.0.translation = pos;
    cam.0.rotation = sim.cam.rotation();
    if let Projection::Perspective(p) = cam.1.as_mut() {
        p.fov = sim.cam.fov.to_radians();
    }
    if let Some((eye, yaw, pitch)) = tour.as_ref().and_then(|t| t.camera()) {
        cam.0.translation = eye;
        cam.0.rotation = Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(pitch.to_radians());
        if let Projection::Perspective(p) = cam.1.as_mut() {
            p.fov = 75f32.to_radians();
        }
    }
    // UNITY_PROBE_ENEMY: frame the enemy nearest the player from 4 m, for the outline buffer stats
    if std::env::var_os("UNITY_PROBE_ENEMY").is_some() {
        // once: stand on the nearest enemy so its room's triggers activate it
        static MOVED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        if MOVED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) == 30 {
            let p = sim.game.s.player.pos;
            if let Some((node, pos)) = sim.game.s.enemies.iter().min_by(|a, b| a.pos.distance(p).total_cmp(&b.pos.distance(p))).map(|e| (e.node, e.pos)) {
                let to = pos + Vec3::Y;
                info!("enemy probe: player moved to node {node} at {to:.1?}");
                sim.game.s.player.pos = to;
                // the room's trigger may not fire: activate the enemy and its ancestors directly
                let mut n = Some(node);
                let mut chain = vec![];
                while let Some(i) = n {
                    chain.push(i);
                    n = sim.game.def.nodes[i as usize].parent;
                }
                for i in chain.into_iter().rev() {
                    sim.game.set_active(i, true);
                }
            }
        }
        let p = sim.game.s.player.pos;
        if let Some(e) = sim.game.s.enemies.iter().filter(|e| e.alive && sim.game.active(e.node)).min_by(|a, b| a.pos.distance(p).total_cmp(&b.pos.distance(p))) {
            static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 120 == 0 {
                info!("enemy probe: framing node {} {:?} at {:.1?} ({} active of {})", e.node, e.kind, e.pos, sim.game.s.enemies.iter().filter(|e| e.alive && sim.game.active(e.node)).count(), sim.game.s.enemies.len());
            }
            let target = e.pos + Vec3::Y;
            let dir = (p - e.pos).with_y(0.0).normalize_or(Vec3::Z);
            cam.0.translation = target + dir * 4.0 + Vec3::Y;
            cam.0.look_at(target, Vec3::Y);
        } else {
            static M: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if M.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 120 == 0 {
                info!("enemy probe: none active ({} alive of {})", sim.game.s.enemies.iter().filter(|e| e.alive).count(), sim.game.s.enemies.len());
            }
        }
    }
    let (Some(mut vm), Some(mut fist)) = (vm, fist) else { return };
    // Revolver kick
    vm.0.translation = Vec3::new(0.32, -0.3, -0.55 + sim.recoil * 0.08);
    vm.0.rotation = Quat::from_rotation_x(sim.recoil * 0.35);
    *vm.1 = if sim.game.s.has_revolver && !sim.game.s.dead { Visibility::Inherited } else { Visibility::Hidden };
    // Punch jab
    let j = (sim.punch_anim * std::f32::consts::PI).sin();
    fist.translation = Vec3::new(-0.38 + j * 0.2, -0.42 + j * 0.15, -0.75 - j * 0.5);
}

fn update_beams(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Beam, &mut Transform)>) {
    for (e, mut b, mut tf) in &mut q {
        b.life -= time.delta_secs();
        if b.life <= 0.0 {
            commands.entity(e).despawn();
        } else {
            let k = b.life / b.max;
            tf.scale.x *= 0.5 + 0.5 * k.max(0.5);
            tf.scale.y = tf.scale.x;
        }
    }
}

#[allow(clippy::type_complexity)]
fn update_hud(
    sim: Res<Sim>,
    diag: Res<Time<Real>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut debug_shown: Local<bool>,
    mut text: Single<&mut Text, (With<HudText>, Without<HintText>, Without<BannerText>, Without<HpText>)>,
    mut hint: Single<&mut Text, (With<HintText>, Without<HudText>, Without<BannerText>, Without<HpText>)>,
    mut banner: Single<&mut Text, (With<BannerText>, Without<HudText>, Without<HintText>, Without<HpText>)>,
    mut hp_text: Single<&mut Text, (With<HpText>, Without<HudText>, Without<HintText>, Without<BannerText>)>,
    mut bars: Query<(&StaminaBar, &mut Node, &mut BackgroundColor), (Without<PierceBar>, Without<HpBar>)>,
    mut pierce: Single<&mut Node, (With<PierceBar>, Without<HpBar>)>,
    mut hp_bar: Single<&mut Node, (With<HpBar>, Without<PierceBar>)>,
) {
    let g = &sim.game;
    let p = &g.s.player;
    // the developer overlay (not part of ULTRAKILL's HUD) is off until F3
    if keys.just_pressed(KeyCode::F3) {
        *debug_shown = !*debug_shown;
    }
    let mut state = Vec::new();
    if p.gc.on_ground {
        state.push("GROUND")
    } else {
        state.push("AIR")
    }
    if p.sliding {
        state.push("SLIDE")
    }
    if p.boost && !p.sliding {
        state.push("DASH")
    }
    if p.gc.heavy_fall {
        state.push("SLAM")
    }
    if p.wall.on_wall {
        state.push("WALL")
    }
    if p.slow_mode {
        state.push("CROUCH")
    }
    let alive_enemies = g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)).count();
    let mut s = format!(
        "ULTRAKRUST  {:.0} fps{}\n{}\nspeed {:5.1} u/s  {}  |  kills {}  enemies here {}  |  time {:.1}s\n",
        1.0 / diag.delta_secs().max(1e-4),
        if sim.noclip { "  [NOCLIP]" } else { "" },
        sim.title,
        p.speed_h(),
        state.join(" "),
        g.s.kills,
        alive_enemies,
        sim.level_time,
    );
    s.push_str("WASD SPACE SHIFT CTRL move | LMB fire, hold RMB pierce | F punch/parry | R checkpoint | N noclip\n");
    for (msg, _) in sim.log.iter().rev().take(4) {
        s.push_str(&format!("\n{msg}"));
    }
    text.0 = if *debug_shown { s } else { String::new() };
    hint.0 = g.s.messages.last().map(|m| m.text.clone()).unwrap_or_default();
    banner.0 = if let Some((name, _)) = &sim.load_request {
        format!("LOADING {}", name.to_uppercase())
    } else if g.s.results_shown {
        results_text(&sim)
    } else if g.s.level_complete && !g.s.rankless_continue {
        "LEVEL COMPLETE".into()
    } else {
        sim.banner.as_ref().map(|b| b.0.clone()).unwrap_or_default()
    };
    hp_text.0 = format!("{}", g.s.hp);
    hp_bar.width = percent(g.s.hp.clamp(0, 100) as f32);
    for (bar, mut node, mut bg) in &mut bars {
        let fill = ((p.boost_charge - bar.0 as f32 * 100.0) / 100.0).clamp(0.0, 1.0);
        node.width = percent(fill * 100.0);
        bg.0 = if fill >= 1.0 { Color::srgb(0.2, 0.75, 1.0) } else { Color::srgb(0.35, 0.4, 0.45) };
    }
    let r = &sim.revolver;
    pierce.width = percent(if !g.s.has_revolver { 0.0 } else if r.pierce_ready { r.pierce_shot_charge } else { r.pierce_charge });
}

/// FinalRank: time, kills, style (with ranks), then the total; the continue prompt once complete
/// in the second pit.
fn results_text(sim: &Sim) -> String {
    let g = &sim.game;
    let (time, kills, style, total) = g.final_ranks(sim.level_time);
    let shown = if sim.results_skipped { TALLY_LINES } else { (sim.results_t / TALLY_STEP).floor().min(TALLY_LINES) };
    let secs = sim.level_time;
    let lines = [
        format!("TIME  {}:{:06.3}  {time}", (secs / 60.0) as u32, secs % 60.0),
        format!("KILLS  {}  {kills}", g.s.kills),
        format!("STYLE  0  {style}"),
        format!("RANK  {total}{}", if g.s.restarts > 0 { format!("   ({} restarts)", g.s.restarts) } else { String::new() }),
    ];
    let mut s = String::from("LEVEL COMPLETE");
    for l in lines.iter().take(shown as usize) {
        s += "\n";
        s += l;
    }
    if shown >= TALLY_LINES && g.s.reached_second_pit {
        s += "\n\n[FIRE] CONTINUE";
    }
    s
}

/// Per-frame game state for the Unity-shader renderer: visibility, movers, lights.
fn unity_frame(sim: Res<Sim>, scene: Res<unity_render::UnityScene>, time: Res<Time>, mut out: ResMut<unity_render::UnityFrame>, mut n: Local<u32>) {
    let Some(scene) = scene.0.as_ref() else { return };
    let t = std::time::Instant::now();
    *out = unity_render::frame(&sim.game, scene, time.elapsed_secs());
    *n += 1;
    if *n == 120 && std::env::var_os("UNITY_FRAME_STATS").is_some() {
        info!("unity frame state (main world): {:.2} ms", t.elapsed().as_secs_f64() * 1e3);
        let (checked, err) = unity_render::skin_rest_error(&sim.game, scene);
        let rigid = out.object_to_world.iter().zip(&scene.draws).filter(|(o, d)| d.skin.is_none() && sim.game.anim.animated(d.node) && !o.abs_diff_eq(Mat4::IDENTITY, 1e-4)).count();
        let a = &sim.game.anim;
        let bound = scene.draws.iter().filter(|d| match &d.skin { Some(sk) => sk.def.bones.iter().flatten().any(|&b| a.animated(b)), None => a.animated(d.node) }).count();
        let bound_vis = scene.draws.iter().zip(out.visible.iter()).filter(|(d, v)| **v && match &d.skin { Some(sk) => sk.def.bones.iter().flatten().any(|&b| a.animated(b)), None => a.animated(d.node) }).count();
        info!(
            "unity anim stats: anim_bound {bound} anim_bound_visible {bound_vis} updated {} skinned {} reskinned {} rigid_moved {} rest_checked {checked} rest_err {err:.6}",
            a.stats.updated_last_frame,
            scene.draws.iter().filter(|d| d.skin.is_some()).count(),
            out.skinned.len(),
            rigid
        );
    }
}

/// `UK_PROBE_EXIT=1`: two seconds in, drops the player into the top of the level's first exit pit
/// (activating it if the level hasn't opened it yet), so the FinalPit -> results -> next level
/// chain can be driven without playing the level; pair with `UK_PROBE_CONTINUE`.
fn exit_probe(mut sim: ResMut<Sim>, mut done: Local<bool>) {
    if *done || sim.generation != 1 || sim.level_time < 2.0 || std::env::var_os("UK_PROBE_EXIT").is_none() {
        return;
    }
    *done = true;
    let g = &mut sim.game;
    let def = g.def.clone();
    let pit = (0..def.scripts.len()).find(|&i| matches!(&g.s.scripts[i], uk_game::scripts::Script::FinalPit(p) if !p.second_pit));
    let Some(pit) = pit else {
        info!("exit probe: no FinalPit in this level");
        return;
    };
    let node = def.scripts[pit].node;
    let mut n = Some(node);
    while let Some(i) = n {
        g.set_active(i, true);
        n = def.nodes[i as usize].parent;
    }
    let Some(top) = def.colliders.iter().filter(|c| c.node == node && c.trigger).find_map(|c| match &c.shape {
        uk_assets::scenedef::ShapeDef::Box { center, half, .. } => Some(*center + Vec3::Y * (half.y - 5.0)),
        _ => None,
    }) else {
        return;
    };
    g.s.player.pos = top;
    g.s.player.prev_pos = top;
    info!("exit probe: dropped into {} at {:?}", def.path(node), top);
}

/// `--exit-after <seconds>`: quit after a while (headless-ish smoke runs that only read the log).
fn exit_after(time: Res<Time>, mut exit: MessageWriter<AppExit>, mut limit: Local<Option<f32>>) {
    let l = limit.get_or_insert_with(|| {
        let args: Vec<String> = std::env::args().collect();
        args.iter().position(|a| a == "--exit-after").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok()).unwrap_or(f32::INFINITY)
    });
    if time.elapsed_secs() > *l {
        exit.write(AppExit::Success);
    }
}
