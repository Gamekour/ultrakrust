//! ULTRAKRUST: Bevy frontend for `uk-core`.
//!
//! `ultrakrust` loads level 0-1 from your ULTRAKILL install (if found);
//! `ultrakrust --level 1-1` picks another level; `ultrakrust --sandbox` opens the test map.
//!
//! Controls (ULTRAKILL defaults): WASD move, Space jump, Left Shift dash,
//! Left Ctrl slide / slam, LMB fire, hold RMB to charge a piercing shot.
//! R respawn, N noclip, [ ] sensitivity, T toggle camera tilt, Esc release mouse, click to grab.

mod level;
mod map;
mod tour;

use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::light::NotShadowCaster;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, WindowResolution};
use uk_core::camera::FpCamera;
use uk_core::collide::World;
use uk_core::consts::FIXED_DT;
use uk_core::player::{Event, Input as PInput, Player};
use uk_core::revolver::{Revolver, Shot};

const SANDBOX_SPAWN: Vec3 = Vec3::new(0.0, 1.5, 100.0);
const VIEW_LAYER: usize = 1;

#[derive(Resource)]
struct Sim {
    world: World,
    player: Player,
    cam: FpCamera,
    revolver: Revolver,
    targets: Vec<map::Target>,
    /// Input latched for the fixed step (held state only).
    fixed_input: PInput,
    clock: f64,
    log: Vec<(String, f32)>,
    recoil: f32,
    spawn: Vec3,
    spawn_yaw: f32,
    noclip: bool,
    title: String,
}

#[derive(Component)]
struct MainCam;
#[derive(Component)]
struct ViewModel;
#[derive(Component)]
struct HudText;
#[derive(Component)]
struct StaminaBar(usize);
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
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, fixed_sim)
        .add_systems(Update, (cursor_grab, frame_sim, apply_view, update_beams, update_hud, tour::run_tour).chain())
        .run();
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let tex = |images: &mut Assets<Image>, d: [u8; 3], l: [u8; 3], cells| images.add(map::checker(d, l, cells));
    let mats = map::Materials {
        floor: materials.add(StandardMaterial {
            base_color_texture: Some(tex(&mut images, [38, 36, 34], [52, 49, 46], 4)),
            perceptual_roughness: 0.95,
            ..default()
        }),
        wall: materials.add(StandardMaterial {
            base_color_texture: Some(tex(&mut images, [60, 26, 24], [78, 34, 30], 2)),
            perceptual_roughness: 0.9,
            ..default()
        }),
        accent: materials.add(StandardMaterial {
            base_color_texture: Some(tex(&mut images, [70, 70, 76], [96, 96, 104], 2)),
            perceptual_roughness: 0.8,
            ..default()
        }),
        target: materials.add(StandardMaterial {
            base_color: Color::srgb(0.9, 0.75, 0.2),
            emissive: LinearRgba::rgb(0.6, 0.4, 0.0),
            ..default()
        }),
    };

    // Level from the user's install, or the movement test map.
    let args: Vec<String> = std::env::args().collect();
    let level_arg = args.iter().position(|a| a == "--level").and_then(|i| args.get(i + 1)).cloned();
    let want_level = !args.iter().any(|a| a == "--sandbox");
    let mut world = World::default();
    let mut targets = Vec::new();
    let (mut spawn, mut spawn_yaw, mut title) = (SANDBOX_SPAWN, 0.0, "movement sandbox".to_string());
    let mut loaded_level = false;
    let tour_dir = args.iter().position(|a| a == "--tour").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    let mut rooms = Vec::new();
    if want_level {
        let name = level_arg.unwrap_or_else(|| "0-1".into());
        match level::load(&name, &mut commands, &mut meshes, &mut materials, &mut images) {
            Ok(l) => {
                info!("{}", l.summary);
                world = l.world;
                spawn = l.spawn;
                spawn_yaw = l.yaw;
                title = l.summary;
                rooms = l.rooms;
                loaded_level = true;
            }
            Err(e) => warn!("could not load level {name}: {e}; falling back to the sandbox"),
        }
    }
    if !loaded_level {
        targets = map::spawn(&mut commands, &mut meshes, &mats, &mut world);
        world.build();
    }

    commands.spawn((
        DirectionalLight { illuminance: 9000.0, shadow_maps_enabled: true, ..default() },
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
                falloff: if loaded_level {
                    FogFalloff::Linear { start: 250.0, end: 900.0 }
                } else {
                    FogFalloff::Linear { start: 120.0, end: 320.0 }
                },
                ..default()
            },
            Transform::from_translation(spawn),
        ))
        .with_children(|c| {
            c.spawn((
                Camera3d::default(),
                Camera { order: 1, ..default() },
                Projection::from(PerspectiveProjection { fov: 90f32.to_radians(), near: 0.01, ..default() }),
                RenderLayers::layer(VIEW_LAYER),
            ));
            // Placeholder revolver: body + barrel + cylinder.
            let gun = materials.add(StandardMaterial { base_color: Color::srgb(0.55, 0.56, 0.6), metallic: 0.7, ..default() });
            let grip = materials.add(Color::srgb(0.25, 0.12, 0.06));
            c.spawn((ViewModel, Transform::from_xyz(0.32, -0.3, -0.55), Visibility::default(), RenderLayers::layer(VIEW_LAYER)))
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
        TextFont { font_size: bevy::text::FontSize::Px(18.0), ..default() },
        TextColor(Color::srgb(0.95, 0.95, 0.95)),
        Node { position_type: PositionType::Absolute, top: px(12), left: px(12), ..default() },
    ));
    commands.spawn((
        Text::new("+"),
        TextFont { font_size: bevy::text::FontSize::Px(28.0), ..default() },
        Node { position_type: PositionType::Absolute, left: percent(50), top: percent(50), margin: UiRect { left: px(-8), top: px(-18), ..default() }, ..default() },
    ));
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: px(24),
            left: px(24),
            flex_direction: FlexDirection::Column,
            row_gap: px(6),
            ..default()
        })
        .with_children(|ui| {
            ui.spawn(Node { flex_direction: FlexDirection::Row, column_gap: px(6), ..default() }).with_children(|row| {
                for i in 0..3 {
                    row.spawn((Node { width: px(90), height: px(14), ..default() }, BackgroundColor(Color::srgb(0.15, 0.15, 0.15))))
                        .with_children(|b| {
                            b.spawn((StaminaBar(i), Node { width: percent(100), height: percent(100), ..default() }, BackgroundColor(Color::srgb(0.2, 0.75, 1.0))));
                        });
                }
            });
            ui.spawn((Node { width: px(282), height: px(6), ..default() }, BackgroundColor(Color::srgb(0.15, 0.15, 0.15))))
                .with_children(|b| {
                    b.spawn((PierceBar, Node { width: percent(0), height: percent(100), ..default() }, BackgroundColor(Color::srgb(1.0, 0.85, 0.2))));
                });
        });

    if let Some(dir) = tour_dir {
        let _ = std::fs::create_dir_all(&dir);
        let t = tour::Tour::new(dir, spawn, spawn_yaw, &rooms);
        info!("tour: {} shots", t.shots.len());
        commands.insert_resource(t);
    }
    let mut player = Player::new(spawn);
    let mut cam = FpCamera::default();
    cam.rotation_y = spawn_yaw;
    player.yaw_deg = cam.rotation_y;
    commands.insert_resource(Sim {
        world,
        player,
        cam,
        revolver: Revolver::default(),
        targets,
        fixed_input: PInput::default(),
        clock: 0.0,
        log: Vec::new(),
        recoil: 0.0,
        spawn,
        spawn_yaw,
        noclip: false,
        title,
    });
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
    if keys.pressed(KeyCode::KeyW) { v.y += 1.0; }
    if keys.pressed(KeyCode::KeyS) { v.y -= 1.0; }
    if keys.pressed(KeyCode::KeyD) { v.x += 1.0; }
    if keys.pressed(KeyCode::KeyA) { v.x -= 1.0; }
    // Unity's composite Move action is normalized
    v.normalize_or_zero()
}

fn fixed_sim(mut sim: ResMut<Sim>) {
    let sim = &mut *sim;
    if sim.noclip {
        return;
    }
    let input = sim.fixed_input;
    sim.player.fixed_update(&sim.world, &input);
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
    mut target_tf: Query<&mut Transform, Without<MainCam>>,
) {
    let dt = time.delta_secs();
    let sim = &mut *sim;
    sim.clock += dt as f64;
    let captured = cursor.grab_mode != CursorGrabMode::None;

    if keys.just_pressed(KeyCode::KeyR) {
        sim.player = Player::new(sim.spawn);
        sim.cam.rotation_y = sim.spawn_yaw;
        sim.player.yaw_deg = sim.spawn_yaw;
    }
    if keys.just_pressed(KeyCode::BracketLeft) { sim.cam.sensitivity *= 0.8; }
    if keys.just_pressed(KeyCode::BracketRight) { sim.cam.sensitivity *= 1.25; }
    if keys.just_pressed(KeyCode::KeyT) { sim.cam.tilt_enabled = !sim.cam.tilt_enabled; }
    if keys.just_pressed(KeyCode::KeyN) {
        sim.noclip = !sim.noclip;
        sim.player.vel = Vec3::ZERO;
    }

    let axis = move_axis(&keys);
    let input = PInput {
        move_axis: axis,
        jump_pressed: keys.just_pressed(KeyCode::Space),
        jump_held: keys.pressed(KeyCode::Space),
        slide_pressed: keys.just_pressed(KeyCode::ControlLeft),
        slide_released: keys.just_released(KeyCode::ControlLeft),
        dash_pressed: keys.just_pressed(KeyCode::ShiftLeft),
    };
    sim.fixed_input = PInput { move_axis: axis, jump_held: input.jump_held, ..default() };

    if captured {
        sim.cam.look(motion.delta);
    }
    sim.player.yaw_deg = sim.cam.rotation_y;
    let clock = sim.clock;
    if sim.noclip {
        // Free fly for looking around a level: camera-relative, Space up, Ctrl down, Shift fast.
        let rot = sim.cam.rotation();
        let mut d = rot * Vec3::new(axis.x, 0.0, -axis.y);
        if keys.pressed(KeyCode::Space) { d.y += 1.0; }
        if keys.pressed(KeyCode::ControlLeft) { d.y -= 1.0; }
        let speed = if keys.pressed(KeyCode::ShiftLeft) { 120.0 } else { 35.0 };
        let step = d.normalize_or_zero() * speed * dt;
        sim.player.pos += step;
        sim.player.prev_pos = sim.player.pos;
    } else {
        sim.player.update(&sim.world, &input, dt, clock);
    }
    sim.cam.late_update(&mut sim.player, axis.x, dt);

    for e in sim.player.events.drain(..) {
        let msg = match e {
            Event::Ssj { gained } => format!("SUPER SLIDE JUMP +{gained:.0} u/s"),
            Event::SlamJump => "SLAM JUMP".into(),
            Event::DashJump => "DASH JUMP".into(),
            Event::WallJump(n) => format!("WALL JUMP {n}/3"),
            Event::StaminaFail => "no stamina".into(),
            Event::Land { impact: true } => "impact".into(),
            _ => continue,
        };
        sim.log.push((msg, 2.0));
    }
    sim.log.retain_mut(|(_, t)| {
        *t -= dt;
        *t > 0.0
    });

    // Revolver
    let fire1 = captured && mouse.pressed(MouseButton::Left);
    let shot = sim.revolver.update(fire1, captured && mouse.pressed(MouseButton::Right), mouse.just_released(MouseButton::Right), dt);
    sim.recoil = (sim.recoil - dt * 6.0).max(0.0);
    if let Some(shot) = shot {
        sim.recoil = 1.0;
        let eye = sim.player.pos + sim.cam.local_pos;
        let dir = sim.cam.rotation() * Vec3::NEG_Z;
        let max = 1000.0;
        // Normal beams stop at the first thing; piercing beams pass through targets until a wall.
        let mut end = eye + dir * max;
        let mut from = eye;
        let mut remaining = max;
        loop {
            let Some(hit) = sim.world.raycast(from, dir, remaining) else { break };
            if let Some(t) = sim.targets.iter_mut().find(|t| t.collider == hit.collider) {
                t.hits += if shot == Shot::Pierce { 2 } else { 1 };
                t.flash = 0.15;
                if shot == Shot::Pierce {
                    let step = hit.distance + 2.5;
                    from += dir * step;
                    remaining -= step;
                    if remaining > 0.0 { continue; }
                }
            }
            end = from + dir * hit.distance;
            break;
        }
        let muzzle = eye + sim.cam.rotation() * Vec3::new(0.32, -0.25, -0.9);
        let len = muzzle.distance(end);
        let width = if shot == Shot::Pierce { 0.35 } else { 0.12 };
        commands.spawn((
            Beam { life: 0.12, max: 0.12 },
            Mesh3d(beams.mesh.clone()),
            MeshMaterial3d(if shot == Shot::Pierce { beams.pierce.clone() } else { beams.normal.clone() }),
            Transform::from_translation((muzzle + end) * 0.5).looking_at(end, Vec3::Y).with_scale(Vec3::new(width, width, len)),
            NotShadowCaster,
        ));
    }
    for t in sim.targets.iter_mut() {
        t.flash = (t.flash - dt).max(0.0);
        if let Ok(mut tf) = target_tf.get_mut(t.entity) {
            tf.scale = Vec3::splat(1.0 + t.flash * 2.0);
        }
    }
}

fn apply_view(
    sim: Res<Sim>,
    tour: Option<Res<tour::Tour>>,
    fixed: Res<Time<Fixed>>,
    mut cam: Single<(&mut Transform, &mut Projection), With<MainCam>>,
    mut vm: Single<&mut Transform, (With<ViewModel>, Without<MainCam>)>,
) {
    let alpha = fixed.overstep_fraction();
    let pos = sim.player.interpolated_pos(alpha) + sim.cam.local_pos;
    cam.0.translation = pos;
    cam.0.rotation = sim.cam.rotation();
    if let Projection::Perspective(p) = cam.1.as_mut() {
        p.fov = sim.cam.fov.to_radians();
    }
    // Revolver kick
    if let Some((eye, yaw, pitch)) = tour.as_ref().and_then(|t| t.camera()) {
        cam.0.translation = eye;
        cam.0.rotation = Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(pitch.to_radians());
        if let Projection::Perspective(p) = cam.1.as_mut() {
            p.fov = 75f32.to_radians();
        }
    }
    vm.translation = Vec3::new(0.32, -0.3, -0.55 + sim.recoil * 0.08);
    vm.rotation = Quat::from_rotation_x(sim.recoil * 0.35);
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

fn update_hud(
    sim: Res<Sim>,
    diag: Res<Time<Real>>,
    mut text: Single<&mut Text, With<HudText>>,
    mut bars: Query<(&StaminaBar, &mut Node, &mut BackgroundColor), Without<PierceBar>>,
    mut pierce: Single<&mut Node, With<PierceBar>>,
) {
    let p = &sim.player;
    let mut state = Vec::new();
    if p.gc.on_ground { state.push("GROUND") } else { state.push("AIR") }
    if p.sliding { state.push("SLIDE") }
    if p.boost && !p.sliding { state.push("DASH") }
    if p.gc.heavy_fall { state.push("SLAM") }
    if p.wall.on_wall { state.push("WALL") }
    if p.slow_mode { state.push("CROUCH") }
    let hits: u32 = sim.targets.iter().map(|t| t.hits).sum();
    let mut s = format!(
        "ULTRAKRUST   {:.0} fps{}\n{}\n\nspeed  {:6.2} u/s  (vertical {:+.1})\nstate  {}\nwall jumps left  {}\nslam force  {:.2}   pre-slide x{:.2}\ntarget hits  {}\n\nWASD move  SPACE jump  SHIFT dash  CTRL slide/slam\nLMB fire  hold RMB pierce  R respawn  N noclip  T tilt  [ ] sens {:.3}\n",
        1.0 / diag.delta_secs().max(1e-4),
        if sim.noclip { "   [NOCLIP]" } else { "" },
        sim.title,
        p.speed_h(),
        p.vel.y,
        state.join(" "),
        3 - p.current_wall_jumps.min(3),
        p.slam_force,
        p.pre_slide_speed.max(1.0),
        hits,
        sim.cam.sensitivity,
    );
    for (msg, _) in sim.log.iter().rev().take(4) {
        s.push_str(&format!("\n{msg}"));
    }
    text.0 = s;
    for (bar, mut node, mut bg) in &mut bars {
        let fill = ((p.boost_charge - bar.0 as f32 * 100.0) / 100.0).clamp(0.0, 1.0);
        node.width = percent(fill * 100.0);
        bg.0 = if fill >= 1.0 { Color::srgb(0.2, 0.75, 1.0) } else { Color::srgb(0.35, 0.4, 0.45) };
    }
    let r = &sim.revolver;
    pierce.width = percent(if r.pierce_ready { r.pierce_shot_charge } else { r.pierce_charge });
}
