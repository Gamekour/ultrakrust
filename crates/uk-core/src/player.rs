//! V1's movement: a clean-room port of the behaviour of ULTRAKILL's `NewMovement`,
//! `GroundCheck` and `WallCheck`, on top of a minimal rigidbody.
//!
//! Split like the original:
//! - [`Player::update`] runs once per rendered frame (Unity `Update`): inputs,
//!   slide state, wall cling, floor snap, slam.
//! - [`Player::fixed_update`] runs at 125 Hz (Unity `FixedUpdate` followed by
//!   the physics step): walking, air control, dash and slide velocity, then
//!   integration, collisions and trigger checks.

use crate::collide::{Capsule, ColliderId, World};
use crate::consts::*;
use crate::umath::*;
use bevy_math::{Quat, Vec2, Vec3};

/// One frame of player input, already in game terms.
#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    /// x = strafe right, y = forward; magnitude <= 1.
    pub move_axis: Vec2,
    pub jump_pressed: bool,
    pub jump_held: bool,
    pub slide_pressed: bool,
    pub slide_released: bool,
    pub dash_pressed: bool,
}

/// `GroundCheck` trigger state.
#[derive(Clone, Debug, Default)]
pub struct GroundCheck {
    pub touching: bool,
    pub on_ground: bool,
    pub cols: Vec<ColliderId>,
    pub since_last_grounded: f32,
    pub forced_off: i32,
    pub heavy_fall: bool,
    pub super_jump_chance: f32,
    pub extra_jump_chance: f32,
    pub bounce_chance: f32,
    pub has_impacted: bool,
    pub can_jump: bool,
}

#[derive(Clone, Debug, Default)]
pub struct WallCheck {
    pub on_wall: bool,
    pub poc: Vec3,
    pub cols: Vec<ColliderId>,
}

/// Events for the frontend (sounds, particles, camera shake).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Jump,
    DashJump,
    SlamJump,
    WallJump(u32),
    Dash,
    StaminaFail,
    SlideStart,
    SlideStop,
    SlamStart,
    Land { impact: bool },
    Ssj { gained: f32 },
}

#[derive(Clone, Debug)]
pub struct Player {
    // --- rigidbody ---
    pub pos: Vec3,
    pub prev_pos: Vec3,
    pub vel: Vec3,
    pub use_gravity: bool,
    pending_force: Vec3,
    pending_accel: Vec3,
    pending_dv: Vec3,
    pub collider_height: f32,

    // --- orientation (Unity: player yaw from CameraController.rotationY) ---
    pub yaw_deg: f32,

    // --- checks ---
    pub gc: GroundCheck,
    pub gc_local: Vec3,
    pub slope: GroundCheck,
    pub wall: WallCheck,

    // --- NewMovement fields ---
    pub activated: bool,
    pub input_dir: Vec3,
    target_vel: Vec3,
    pub walking: bool,
    pub falling: bool,
    pub(crate) fall_time: f32,
    pub fall_speed: f32,
    pub jumping: bool,
    jumping_timer: f32,
    pub jump_cooldown: bool,
    jump_cooldown_timer: f32,
    pub current_wall_jumps: u32,
    cling_fade: f32,
    pub boost: bool,
    pub boost_charge: f32,
    pub boost_left: f32,
    dashed_from_ground: bool,
    dash_storage: f32,
    pub dodge_direction: Vec3,
    pub still_holding: bool,
    pub slam_force: f32,
    slam_storage: bool,
    pub slam_cooldown: f32,
    pub sliding: bool,
    slide_safety: f32,
    slide_ending: bool,
    pub slide_length: f32,
    pub longest_slide: f32,
    pub crouching: bool,
    pub standing: bool,
    pub slow_mode: bool,
    pub pre_slide_speed: f32,
    pre_slide_delay: f32,
    friction: f32,
    pub wind_state: f32,
    pub pre_dash_speed: Vec3,
    frames_since_slide: u32,
    velocity_after_slide: Vec3,
    enemy_stepping: bool,
    pub slide_timestamp: f64,
    pub jump_timestamp: f64,
    last_jump: f32,
    /// 0 = forward, 1 = backward, 2 = other (drives dash FOV in the camera).
    pub cam_dodge_direction: u8,
    /// Camera eye target offset (CameraController.defaultTarget).
    pub cam_default_target: Vec3,
    pub cam_reset_requested: bool,

    /// GameObject layer 15 ("Invincible"): set while dashing or just hurt; blocks hits
    /// that are flagged invincible (most enemy attacks).
    pub invincible_layer: bool,
    pub hurt_invincibility: f32,

    pub events: Vec<Event>,
    contact_normals: Vec<Vec3>,
}

impl Player {
    pub fn new(pos: Vec3) -> Self {
        Self {
            pos,
            prev_pos: pos,
            vel: Vec3::ZERO,
            use_gravity: true,
            pending_force: Vec3::ZERO,
            pending_accel: Vec3::ZERO,
            pending_dv: Vec3::ZERO,
            collider_height: STAND_HEIGHT,
            yaw_deg: 0.0,
            gc: GroundCheck::default(),
            gc_local: GROUND_CHECK_POS,
            slope: GroundCheck::default(),
            wall: WallCheck::default(),
            activated: true,
            input_dir: Vec3::ZERO,
            target_vel: Vec3::ZERO,
            walking: false,
            falling: false,
            fall_time: 0.0,
            fall_speed: 0.0,
            jumping: false,
            jumping_timer: 0.0,
            jump_cooldown: false,
            jump_cooldown_timer: 0.0,
            current_wall_jumps: 0,
            cling_fade: 0.0,
            boost: false,
            boost_charge: 300.0,
            boost_left: 0.0,
            dashed_from_ground: false,
            dash_storage: 0.0,
            dodge_direction: Vec3::ZERO,
            still_holding: false,
            slam_force: 0.0,
            slam_storage: false,
            slam_cooldown: 0.0,
            sliding: false,
            slide_safety: 0.0,
            slide_ending: false,
            slide_length: 0.0,
            longest_slide: 0.0,
            crouching: false,
            standing: true,
            slow_mode: false,
            pre_slide_speed: 0.0,
            pre_slide_delay: 0.0,
            friction: 1.0,
            wind_state: 0.0,
            pre_dash_speed: Vec3::ZERO,
            frames_since_slide: 100,
            velocity_after_slide: Vec3::ZERO,
            enemy_stepping: false,
            slide_timestamp: -1.0,
            jump_timestamp: -1.0,
            last_jump: 100.0,
            cam_dodge_direction: 0,
            cam_default_target: CAMERA_POS,
            cam_reset_requested: false,
            invincible_layer: false,
            hurt_invincibility: 0.0,
            events: Vec::new(),
            contact_normals: Vec::new(),
        }
    }

    // ------------------------------------------------------------------ helpers

    fn rot(&self) -> Quat {
        Quat::from_rotation_y(-self.yaw_deg.to_radians())
    }

    /// Unity forward (+Z) mapped to Bevy's -Z, with clockwise yaw.
    pub fn forward(&self) -> Vec3 {
        self.rot() * Vec3::NEG_Z
    }

    pub fn right(&self) -> Vec3 {
        self.rot() * Vec3::X
    }

    fn up(&self) -> Vec3 {
        Vec3::Y
    }

    fn gravity_dir(&self) -> Vec3 {
        Vec3::NEG_Y
    }

    pub fn capsule(&self) -> Capsule {
        Capsule::unity(self.pos + Vec3::Y * COLLIDER_CENTER_Y, self.collider_height, RADIUS)
    }

    fn gc_pos(&self) -> Vec3 {
        self.pos + self.gc_local
    }

    /// Moves the transform outside of the physics step (keeps interpolation continuous).
    fn shift(&mut self, d: Vec3) {
        self.pos += d;
        self.prev_pos += d;
    }

    fn add_force(&mut self, f: Vec3) {
        self.pending_force += f;
    }

    pub fn speed_h(&self) -> f32 {
        Vec2::new(self.vel.x, self.vel.z).length()
    }

    fn set_jumping_for(&mut self, secs: f32, cancel: bool) {
        // Invoke("NotJumping", t) — without CancelInvoke the earliest pending call wins.
        if cancel || !self.jumping || self.jumping_timer <= 0.0 {
            self.jumping_timer = secs;
        } else {
            self.jumping_timer = self.jumping_timer.min(secs);
        }
        self.jumping = true;
    }

    fn set_jump_cooldown_for(&mut self, secs: f32, cancel: bool) {
        if cancel || self.jump_cooldown_timer <= 0.0 {
            self.jump_cooldown_timer = secs;
        } else {
            self.jump_cooldown_timer = self.jump_cooldown_timer.min(secs);
        }
        self.jump_cooldown = true;
    }

    // ------------------------------------------------------------------ Update

    /// Unity `Update`. `dt` is the frame delta, `now` is the input clock in seconds.
    pub fn update(&mut self, world: &World, input: &Input, dt: f32, now: f64) {
        self.tick_invokes(dt);
        self.update_ground_state(dt);

        if input.jump_pressed {
            self.jump_timestamp = now;
        }
        if input.slide_released && self.sliding {
            self.slide_timestamp = now;
        }

        if self.activated {
            let v = input.move_axis;
            self.input_dir = clamp_magnitude(self.right() * v.x + self.forward() * v.y, 1.0);
        } else {
            self.input_dir = Vec3::ZERO;
        }

        if self.gc.on_ground {
            self.fall_time = 0.0;
            self.cling_fade = 0.0;
        } else {
            if self.fall_time < 1.0 {
                self.fall_time += dt * 5.0;
            } else {
                self.falling = true;
            }
            let up_speed = (-self.gravity_dir()).dot(self.vel);
            if up_speed < -2.0 {
                self.fall_speed = up_speed;
            }
            if up_speed < -100.0 {
                self.vel = project_on_plane(self.vel, self.gravity_dir()) + self.gravity_dir() * 100.0;
            }
        }
        if self.falling {
            self.check_landing();
        }
        if self.activated {
            self.handle_inputs(world, input, dt, now);
        }
        if !self.gc.on_ground {
            self.cling(world, dt);
        }
        self.try_floor_snap(world, dt);
        if self.gc.heavy_fall {
            if !self.slam_storage {
                self.vel = self.gravity_dir() * 100.0;
            }
            self.slam_force += dt * 5.0;
        }
        if self.still_holding && input.slide_released {
            self.still_holding = false;
        }
        self.handle_slide_state(world, input, dt);
        self.walking = input.move_axis.length_squared() > f32::EPSILON && !self.sliding && self.gc.on_ground;

        if self.boost_charge != 300.0 && !self.sliding && !self.slow_mode {
            // difficulty >= 2 (Standard and up): multiplier 1
            self.boost_charge = move_towards(self.boost_charge, 300.0, 70.0 * dt);
        }
        if self.slam_cooldown > 0.0 {
            self.slam_cooldown = move_towards(self.slam_cooldown, 0.0, dt);
        }
        if self.hurt_invincibility >= 0.0 {
            self.hurt_invincibility = move_towards(self.hurt_invincibility, 0.0, dt);
        }
    }

    fn tick_invokes(&mut self, dt: f32) {
        self.last_jump += dt;
        if self.jumping_timer > 0.0 {
            self.jumping_timer -= dt;
            if self.jumping_timer <= 0.0 {
                self.jumping = false;
            }
        }
        if self.jump_cooldown_timer > 0.0 {
            self.jump_cooldown_timer -= dt;
            if self.jump_cooldown_timer <= 0.0 {
                self.jump_cooldown = false;
            }
        }
    }

    /// `GroundCheckGroup.Update` → `GroundCheck.UpdateState` for both checks, plus `WallCheck.Update`.
    fn update_ground_state(&mut self, dt: f32) {
        for slope in [false, true] {
            let g = if slope { &mut self.slope } else { &mut self.gc };
            if g.forced_off > 0 {
                g.on_ground = false;
            } else if g.on_ground != g.touching {
                g.on_ground = g.touching;
            }
            if g.on_ground {
                g.since_last_grounded = 0.0;
            } else {
                g.since_last_grounded += dt;
            }
        }
        let g = &mut self.gc;
        if g.super_jump_chance > 0.0 {
            g.super_jump_chance = move_towards(g.super_jump_chance, 0.0, dt);
            if g.super_jump_chance == 0.0 {
                // (shockwave spawns here when still holding slide)
                g.extra_jump_chance = 0.306;
                self.still_holding = false;
            }
        }
        if g.extra_jump_chance > 0.0 {
            g.extra_jump_chance = move_towards(g.extra_jump_chance, 0.0, dt);
            if g.extra_jump_chance <= 0.0 && g.super_jump_chance <= 0.0 && g.bounce_chance <= 0.0 {
                self.slam_force = 0.0;
            }
        }
        if g.bounce_chance > 0.0 {
            g.bounce_chance = move_towards(g.bounce_chance, 0.0, dt);
        } else {
            g.has_impacted = false;
        }
        if self.wall.on_wall {
            self.wall.on_wall = !self.wall.cols.is_empty();
        }
    }

    fn check_landing(&mut self) {
        if self.gc.on_ground {
            let impact = self.fall_speed <= -50.0;
            if impact {
                self.gc.has_impacted = true;
            }
            self.events.push(Event::Land { impact });
            if !self.jump_cooldown {
                self.falling = false;
            }
            self.fall_speed = 0.0;
            self.slam_storage = false;
            self.gc.heavy_fall = false;
        }
    }

    fn handle_inputs(&mut self, world: &World, input: &Input, dt: f32, now: f64) {
        let can_ground_jump = !self.falling;
        let enemy_step = !self.gc.on_ground && self.gc.can_jump;
        if input.jump_pressed && !self.jump_cooldown {
            if enemy_step {
                self.enemy_stepping = true;
                if self.sliding || now - self.slide_timestamp < 0.1 {
                    self.wind_state = 0.5;
                }
                self.jump(dt);
            } else if can_ground_jump {
                self.jump(dt);
            }
        }
        if input.slide_pressed {
            let near_ground = world.raycast(self.gc_pos() + self.up(), -self.up(), 2.0).is_some();
            if (self.gc.on_ground || self.gc.since_last_grounded < 0.06 || self.last_jump < 0.06 || near_ground)
                && (!self.slow_mode || self.crouching)
                && !self.sliding
            {
                self.start_slide();
            }
        }
        if !self.slow_mode && input.dash_pressed {
            self.try_dash();
        }
        if !self.gc.on_ground {
            if input.jump_pressed && self.current_wall_jumps < 3 && !self.jump_cooldown && self.wall.on_wall {
                self.wall_jump(now);
            }
            if input.slide_pressed {
                self.try_start_slam(world);
            }
        }
    }

    fn try_dash(&mut self) {
        self.dashed_from_ground = self.gc.on_ground;
        if self.boost_charge < 100.0 {
            self.events.push(Event::StaminaFail);
            return;
        }
        if self.sliding {
            self.stop_slide();
        }
        self.boost_left = 100.0;
        self.dash_storage = 1.0;
        self.boost = true;
        self.dodge_direction = if self.input_dir == Vec3::ZERO { self.forward() } else { self.input_dir };
        self.boost_charge -= 100.0;
        self.cam_dodge_direction = self.dodge_dir_code();
        self.events.push(Event::Dash);
        if self.gc.heavy_fall {
            self.fall_speed = 0.0;
            self.gc.heavy_fall = false;
        }
    }

    fn dodge_dir_code(&self) -> u8 {
        if self.dodge_direction == self.forward() {
            0
        } else if self.dodge_direction == -self.forward() {
            1
        } else {
            2
        }
    }

    fn try_start_slam(&mut self, world: &World) {
        if self.dashed_from_ground && self.boost && !self.sliding {
            self.dashed_from_ground = false;
            self.wind_state = 0.5;
            self.boost = false;
        }
        let ground_below = world.raycast(self.gc_pos() + self.up(), -self.up(), 3.0).is_some();
        if !ground_below && self.fall_time > 0.5 && self.slam_cooldown == 0.0 && !self.gc.heavy_fall {
            if self.boost {
                self.boost_left = 0.0;
                self.boost = false;
            }
            if self.sliding {
                self.stop_slide();
            }
            self.falling = true;
            self.fall_speed = -100.0;
            self.vel = Vec3::new(0.0, -100.0, 0.0);
            self.still_holding = true;
            self.gc.heavy_fall = true;
            self.slam_force = 1.0;
            self.events.push(Event::SlamStart);
        }
    }

    fn cling(&mut self, world: &World, dt: f32) {
        if !self.wall.on_wall {
            return;
        }
        let toward_wall = world.raycast(self.pos, self.input_dir, 1.0).is_some();
        let up_speed = (-self.gravity_dir()).dot(self.vel);
        if !self.sliding && toward_wall && !self.gc.heavy_fall && up_speed < -1.0 {
            // The original calls Mathf.Clamp(-1, 1, x): value -1 clamped to [1, x], which is
            // always 1. Kept as-is, since this is what the game does.
            let r = 1.0;
            let f = 1.0;
            self.vel = r * self.right() + f * self.forward() + self.gravity_dir() * 2.0 * self.cling_fade;
            self.cling_fade = move_towards(self.cling_fade, 50.0, dt * 4.0);
        }
    }

    fn handle_slide_state(&mut self, world: &World, input: &Input, dt: f32) {
        if self.sliding {
            if input.slide_released || (self.slow_mode && !self.crouching) {
                self.stop_slide();
            }
            self.standing = false;
            self.slide_length += dt;
            self.cam_default_target = CAMERA_POS - Vec3::Y * 0.625;
            if self.slide_safety > 0.0 {
                self.slide_safety -= dt * 5.0;
            }
            return;
        }
        if self.collider_height != STAND_HEIGHT {
            let cap = self.capsule();
            let bottom = cap.a - Vec3::Y * cap.radius;
            let blocked = world.raycast(bottom, self.up(), STAND_HEIGHT).is_some()
                || world.sphere_cast(bottom + self.up() * 0.25, 0.5, self.up(), 2.0).is_some();
            if blocked {
                self.crouching = true;
                self.slow_mode = true;
                return;
            }
            self.crouching = false;
            self.slow_mode = false;
            self.collider_height = STAND_HEIGHT;
            self.gc_local = GROUND_CHECK_POS;
            if world.raycast(self.pos, -self.up(), 2.25).is_some() {
                self.shift(self.up() * 1.125);
            } else {
                self.shift(self.up() * -0.625);
                self.cam_default_target = CAMERA_POS;
                self.standing = true;
            }
            self.cam_default_target = CAMERA_POS;
        } else if self.cam_default_target != CAMERA_POS {
            self.cam_default_target = CAMERA_POS;
        } else {
            self.standing = true;
        }
    }

    fn try_floor_snap(&mut self, world: &World, dt: f32) {
        if self.slope.on_ground || self.slope.forced_off > 0 || self.jumping || self.boost {
            return;
        }
        let num = self.collider_height / 2.0 - COLLIDER_CENTER_Y;
        if self.vel != Vec3::ZERO {
            if let Some(hit) = world.raycast(self.pos, -self.up(), num + 1.0) {
                let target = self.pos - self.up() * hit.distance + self.up() * num;
                let new = vmove_towards(self.pos, target, hit.distance * dt * 10.0);
                self.shift(new - self.pos);
                if normalized(self.vel).dot(self.up()) > 0.0 {
                    self.vel = project_on_plane(self.vel, self.up());
                }
            }
        }
    }

    fn start_slide(&mut self) {
        if !self.crouching {
            let old = self.collider_height;
            self.collider_height = SLIDE_HEIGHT;
            let diff = old - SLIDE_HEIGHT;
            self.shift(self.up() * -0.5 * diff);
            self.gc_local = GROUND_CHECK_POS + Vec3::Y * 1.125;
        }
        self.slide_safety = 1.0;
        self.sliding = true;
        self.boost = true;
        self.dodge_direction = self.input_dir;
        if self.dodge_direction == Vec3::ZERO {
            self.dodge_direction = self.forward();
        }
        self.cam_dodge_direction = self.dodge_dir_code();
        self.events.push(Event::SlideStart);
    }

    pub fn stop_slide(&mut self) {
        self.cam_reset_requested = true;
        self.sliding = false;
        self.slide_ending = true;
        self.longest_slide = self.longest_slide.max(self.slide_length);
        self.slide_length = 0.0;
        self.frames_since_slide = 0;
        self.velocity_after_slide = normalized(self.dodge_direction) * 24f32.max(self.pre_dash_speed.length());
        self.events.push(Event::SlideStop);
    }

    pub fn jump(&mut self, frame_dt: f32) {
        self.fall_time = 0.0;
        self.last_jump = 0.0;
        let num = 1500.0;
        self.set_jumping_for(0.25, true);
        self.falling = true;
        self.vel = project_on_plane(self.vel, self.up());
        let up = self.up() * JUMP_POWER * num;
        let gc = &self.gc;
        let slam_window = gc.super_jump_chance > 0.0 || gc.bounce_chance > 0.0 || gc.extra_jump_chance > 0.0;
        if self.sliding {
            self.add_force(up * if self.slow_mode { 1.0 } else { 2.0 });
            self.stop_slide();
            self.events.push(Event::Jump);
        } else if self.boost && self.jump_timestamp - self.slide_timestamp > (FIXED_DT * SSJ_MAX_FRAMES) as f64 {
            if self.enemy_stepping {
                self.wind_state = 0.5;
                self.events.push(Event::DashJump);
            } else if self.boost_charge >= 100.0 {
                self.boost_charge -= 100.0;
                self.events.push(Event::DashJump);
            } else {
                // Uses the *frame* delta in the original (Jump runs from Update).
                let s = WALK_SPEED * frame_dt * 2.75;
                self.vel = Vec3::new(self.input_dir.x * s, 0.0, self.input_dir.z * s);
                self.events.push(Event::StaminaFail);
            }
            self.add_force(up * if self.slow_mode { 0.75 } else { 1.5 });
        } else if self.slow_mode {
            self.add_force(up * 1.25);
            self.events.push(Event::Jump);
        } else if slam_window {
            let k = if self.slam_force < 5.5 { 3.0 + (self.slam_force - 1.0) } else { 12.5 };
            self.add_force(up * k);
            self.events.push(Event::SlamJump);
        } else {
            self.add_force(up * 2.6);
            self.events.push(Event::Jump);
        }
        let dir = normalized(self.dodge_direction);
        self.try_ssj(dir, 0.5, |f| 1.0 / 2f32.powi(f - 1));
        self.set_jump_cooldown_for(0.2, true);
        self.boost = false;
        self.gc.bounce_chance = 0.0;
        self.gc.heavy_fall = false;
        self.enemy_stepping = false;
    }

    fn try_ssj(&mut self, dir: Vec3, speed_mult: f32, loss: impl Fn(i32) -> f32) {
        let num = self.jump_timestamp - self.slide_timestamp;
        if num <= 0.0 {
            return;
        }
        let frames = (num / FIXED_DT as f64) as i32;
        if frames == 0 || frames as f32 >= SSJ_MAX_FRAMES {
            return;
        }
        let gained = loss(frames) * speed_mult * WALK_SPEED * 2.75 * 3.0 * FIXED_DT;
        let y = self.vel.y;
        let mut v = self.velocity_after_slide + dir * gained;
        v.y = y;
        self.vel = v.length().min(100.0) * normalized(v);
        self.events.push(Event::Ssj { gained });
    }

    fn wall_jump(&mut self, now: f64) {
        self.set_jumping_for(0.25, false);
        self.current_wall_jumps += 1;
        if self.gc.heavy_fall {
            self.slam_storage = true;
        }
        self.events.push(Event::WallJump(self.current_wall_jumps));
        let wall_jump_pos = self.pos - self.wall.poc;
        let g = self.gravity_dir();
        if self.sliding || now - self.slide_timestamp < (SSJ_MAX_FRAMES * FIXED_DT) as f64 {
            let mut v = reflect(normalized(self.dodge_direction), normalized(wall_jump_pos));
            v = normalized(project_on_plane(v, -g));
            v = normalized(v + normalized(wall_jump_pos) * 0.35);
            self.dodge_direction = v;
            self.vel = normalized(v) * self.vel.length();
            self.try_ssj(v, 0.75, |f| (SSJ_MAX_FRAMES - f as f32 + 1.0) / SSJ_MAX_FRAMES);
            let a = self.vel.dot(-g);
            self.vel = project_on_plane(self.vel, g) + -g * a.max(15.0);
            self.wind_state = 0.5;
        } else {
            self.boost = false;
            self.vel = Vec3::ZERO;
            let mut v2 = project_on_plane(normalized(wall_jump_pos), -g);
            v2 += g * -1.0;
            self.add_force(v2 * 2000.0 * WALL_JUMP_POWER);
        }
        self.set_jump_cooldown_for(0.1, false);
    }

    // ------------------------------------------------------------------ FixedUpdate

    /// Unity `FixedUpdate` + one physics step (dt = [`FIXED_DT`]).
    pub fn fixed_update(&mut self, world: &World, input: &Input) {
        let dt = FIXED_DT;
        self.prev_pos = self.pos;
        self.friction = 1.0;
        if self.sliding {
            if self.slide_safety <= 0.0 {
                let h = Vec3::new(self.vel.x, 0.0, self.vel.z);
                if h.length() < 10.0 {
                    self.slide_safety = move_towards(self.slide_safety, -0.1, dt);
                    if self.slide_safety <= -0.1 {
                        self.stop_slide();
                    }
                } else {
                    self.slide_safety = 0.0;
                }
            }
            let up_speed = (-self.gravity_dir()).dot(self.vel);
            if self.wall.on_wall && up_speed < 0.0 {
                self.pending_accel += -Vec3::Y * GRAVITY * 0.4;
            }
        }
        if !self.sliding && self.activated {
            self.frames_since_slide = self.frames_since_slide.saturating_add(1);
            if self.gc.heavy_fall {
                self.pre_slide_delay = 0.2;
                self.pre_slide_speed = self.slam_force;
                let origin = self.pos + self.up() * 1.5;
                let dist = 3.0 + dt * self.vel.y.abs();
                if let Some(hit) = world.sphere_cast(origin, 0.35, -self.up(), dist) {
                    let target = hit.point + self.up() * 1.5;
                    self.pos = target;
                    self.vel = Vec3::ZERO;
                }
            } else if !self.boost && self.falling && self.vel.length() / 24.0 > self.pre_slide_speed {
                self.pre_slide_speed = self.vel.length() / 24.0;
                self.pre_slide_delay = 0.2;
            } else {
                self.pre_slide_delay = move_towards(self.pre_slide_delay, 0.0, dt);
                if self.pre_slide_delay <= 0.0 {
                    self.pre_slide_delay = 0.2;
                    self.pre_slide_speed = self.vel.length() / 24.0;
                }
            }
        }
        self.wind_state -= dt;
        if self.boost {
            self.use_gravity = true;
            self.dodge(input);
        } else {
            self.do_move();
        }
        if !self.boost || self.boost_left <= 0.0 {
            self.pre_dash_speed = self.vel;
        }
        self.physics_step(world);
    }

    fn do_move(&mut self) {
        let dt = FIXED_DT;
        self.slide_ending = false;
        if self.hurt_invincibility <= 0.0 {
            self.invincible_layer = false;
        }
        if self.gc.on_ground && !self.jumping {
            self.current_wall_jumps = 0;
        }
        if self.gc.on_ground && self.friction > 0.0 && !self.jumping {
            let mut num = self.gravity_dir().dot(self.vel);
            if self.slope.on_ground && self.input_dir.x == 0.0 && self.input_dir.z == 0.0 {
                num = 0.0;
                self.use_gravity = false;
            } else {
                self.use_gravity = true;
            }
            let num2 = if self.slow_mode { 1.25 } else { 2.75 };
            self.target_vel = self.input_dir * (WALK_SPEED * dt * num2) + self.gravity_dir() * num;
            self.vel = vlerp(self.vel, self.target_vel, 0.25 * self.friction);
            return;
        }
        self.use_gravity = true;
        let g = self.gravity_dir();
        let vector3 = (if self.slow_mode { 1.25 } else { 2.75 }) * dt * WALK_SPEED * self.input_dir;
        let num3 = g.dot(self.vel);
        self.target_vel = vector3 + g * num3;
        let vector4 = project_on_plane(self.vel, g);
        let mut air_direction = Vec3::ZERO;
        let num8 = if self.wind_state > 0.0 {
            1.0
        } else if !(self.wind_state > -0.25) {
            0.0
        } else {
            inverse_lerp(0.0, -0.25, self.wind_state)
        };
        let num9 = lerp(1.0, 2.0, num8);
        if self.input_dir.length() > 0.0 {
            let num10 = AIR_ACCELERATION * lerp(1.0, 4.0, num8) * dt * (1.0 / MASS);
            let vector5 = project(self.input_dir, self.right());
            let num11 = vector4.dot(normalized(vector5));
            let mut num12 = num10;
            if num11 + num12 > 16.5 {
                num12 = (16.5 - num11).max(0.0);
            }
            let vector6 = project(self.input_dir, self.forward());
            let num13 = vector4.dot(normalized(vector6));
            let mut num14 = num10;
            if num13 + num14 > 16.5 {
                num14 = (16.5 - num13).max(0.0);
            }
            let vector7 = vector5 * num12 + vector6 * num14;
            let vector8 = if vector4.length() > 0.001 { normalized(vector4) } else { self.input_dir };
            let vector9 = project_on_plane(vector7, vector8);
            let vector10 = project(vector7, vector8);
            let num15 = lerp(0.1, 0.4, 1.0 - num8);
            let flag = vector10.dot(vector8) > 0.0;
            let flag2 = vector4.length() > 16.5;
            air_direction += vector9 + vector10 * if flag && flag2 { num15 } else { 1.0 };
            let f = (1.0 - air_direction.length() / (num10 + 0.0001)).powi(3);
            let max_len = AIR_ACCELERATION * (1.0 / MASS) * dt * num9 * f;
            let mut vector11 = self.input_dir * vector4.length() - vector4;
            let vector12 = project(vector11, vector8);
            vector11 = project_on_plane(vector11, vector8);
            if vector12.dot(vector4) < 0.0 {
                vector11 += vector12 * lerp(0.4, 1.0, 1.0 - num8);
            } else {
                vector11 += vector12;
            }
            air_direction += clamp_magnitude(vector11, max_len);
        }
        self.pending_dv += air_direction;
    }

    fn dodge(&mut self, input: &Input) {
        let dt = FIXED_DT;
        if self.sliding {
            let mut num = 1.0;
            if self.pre_slide_speed > 1.0 {
                if self.pre_slide_speed > 3.0 {
                    self.pre_slide_speed = 3.0;
                }
                num = self.pre_slide_speed;
                if self.gc.on_ground && self.friction != 0.0 {
                    self.pre_slide_speed -= dt * self.pre_slide_speed * self.friction;
                }
                self.pre_slide_delay = 0.0;
            }
            let g = self.gravity_dir();
            let num2 = (-g).dot(self.vel);
            let mut v = project_on_plane(self.dodge_direction, g) * WALK_SPEED * dt * 4.0 * num;
            v += -g * num2;
            if self.boost_left > 0.0 {
                self.dash_storage = move_towards(self.dash_storage, 0.0, dt);
                if self.dash_storage <= 0.0 {
                    self.boost_left = 0.0;
                }
            }
            self.input_dir = clamp_magnitude(input.move_axis.x * normalized(project_on_plane(self.right(), g)), 1.0) * 5.0;
            self.vel = v + self.input_dir;
            return;
        }
        if self.frames_since_slide <= 3 {
            return;
        }
        let mut num3 = 0.0;
        if self.slide_ending {
            num3 = (-self.gravity_dir()).dot(self.vel);
        }
        let ang = angle_deg(self.up(), self.dodge_direction);
        if !(85.0..=95.0).contains(&ang) {
            self.boost = false;
            return;
        }
        if self.gc.on_ground {
            self.dashed_from_ground = true;
        }
        self.target_vel = self.dodge_direction * WALK_SPEED * dt * 2.75 + -self.gravity_dir() * num3;
        self.invincible_layer = true;
        if self.slide_ending {
            self.slide_ending = false;
            if !self.gc.on_ground || self.friction == 0.0 {
                self.boost = false;
                return;
            }
        }
        if self.boost_left > 0.0 {
            self.vel = self.target_vel * 3.0;
            self.boost_left -= 4.0;
            return;
        }
        if !self.gc.on_ground || self.friction != 0.0 {
            self.vel = self.target_vel;
        }
        self.boost = false;
    }

    /// Rigidbody step: forces → gravity → integrate with collisions → triggers.
    fn physics_step(&mut self, world: &World) {
        let dt = FIXED_DT;
        self.vel += self.pending_force / MASS * dt + self.pending_accel * dt + self.pending_dv;
        self.pending_force = Vec3::ZERO;
        self.pending_accel = Vec3::ZERO;
        self.pending_dv = Vec3::ZERO;
        if self.use_gravity {
            self.vel.y += GRAVITY * dt;
        }

        // Continuous collision: sub-step so no step moves more than 0.2 units.
        let travel = self.vel.length() * dt;
        let steps = ((travel / 0.2).ceil() as usize).clamp(1, 64);
        let sub = dt / steps as f32;
        for _ in 0..steps {
            self.pos += self.vel * sub;
            let mut cap = self.capsule();
            let before = cap.a;
            self.contact_normals.clear();
            world.depenetrate_capsule(&mut cap, &mut self.contact_normals);
            self.pos += cap.a - before;
            for n in self.contact_normals.iter() {
                let into = self.vel.dot(*n);
                if into < 0.0 {
                    self.vel -= *n * into;
                }
            }
        }
        self.update_triggers(world);
    }

    fn update_triggers(&mut self, world: &World) {
        // GroundCheck: scaled trigger capsule below the feet.
        let gc_cap = Capsule::unity(self.gc_pos(), GC_HEIGHT, GC_RADIUS);
        let now = world.overlap_capsule(gc_cap);
        let entered: Vec<ColliderId> = now.iter().copied().filter(|c| !self.gc.cols.contains(c)).collect();
        for _ in &entered {
            if self.gc.heavy_fall {
                // Environment hit while slamming (no breakables in the sandbox).
                self.gc.heavy_fall = false;
                self.gc.super_jump_chance = 0.1;
            }
        }
        self.gc.touching = !now.is_empty();
        self.gc.cols = now;

        let slope_cap = Capsule::unity(self.pos + SLOPE_CHECK_POS + Vec3::Y * COLLIDER_CENTER_Y, STAND_HEIGHT, SLOPE_RADIUS);
        let now = world.overlap_capsule(slope_cap);
        self.slope.touching = !now.is_empty();
        self.slope.cols = now;

        let wc = self.pos + WALL_CHECK_POS;
        let cols: Vec<ColliderId> = world.overlap_sphere(wc, WALL_RADIUS).into_iter().filter(|c| !world.get(*c).slippery()).collect();
        if cols.iter().any(|c| !self.wall.cols.contains(c)) {
            self.wall.on_wall = true;
        }
        let mut best = f32::MAX;
        for c in &cols {
            let p = world.closest_point(*c, wc);
            let d = p.distance(wc);
            if d < best && d < 5.0 {
                best = d;
                self.wall.poc = p;
            }
        }
        self.wall.cols = cols;
    }

    /// Render position (Rigidbody interpolation).
    pub fn interpolated_pos(&self, alpha: f32) -> Vec3 {
        self.prev_pos.lerp(self.pos, alpha.clamp(0.0, 1.0))
    }
}
