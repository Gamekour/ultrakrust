//! Headless checks. Each expected value is derived by hand from the original
//! formulas and the constants in `consts.rs` (worked out in the comments).

use crate::collide::{BoxCollider, World};
use crate::consts::*;
use crate::player::{Event, Input, Player};
use bevy_math::{Vec2, Vec3};

struct Sim {
    world: World,
    p: Player,
    t: f64,
}

impl Sim {
    fn new(world: World, pos: Vec3) -> Self {
        let mut s = Self { world, p: Player::new(pos), t: 0.0 };
        s.run(60, Input::default()); // settle
        s
    }

    /// One 125 fps frame: FixedUpdate (+physics) then Update, as Unity orders them.
    fn frame(&mut self, input: Input) {
        self.p.events.clear();
        self.p.fixed_update(&self.world, &input);
        self.t += FIXED_DT as f64;
        self.p.update(&self.world, &input, FIXED_DT, self.t);
    }

    fn run(&mut self, frames: usize, input: Input) {
        for _ in 0..frames {
            self.frame(input);
        }
    }
}

fn flat() -> World {
    let mut w = World::default();
    w.add(BoxCollider::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(400.0, 1.0, 400.0)));
    w
}

fn fwd() -> Input {
    Input { move_axis: Vec2::Y, ..default() }
}

fn default() -> Input {
    Input::default()
}

#[test]
fn stands_on_floor() {
    let s = Sim::new(flat(), Vec3::new(0.0, 3.0, 0.0));
    assert!(s.p.gc.on_ground);
    // capsule bottom = pos.y + 0.25 - 1.75 = pos.y - 1.5 → feet on y = 0
    assert!((s.p.pos.y - 1.5).abs() < 0.01, "y = {}", s.p.pos.y);
}

#[test]
fn walk_speed_is_16_5() {
    // targetVel = 750 * 0.008 * 2.75 = 16.5; lerp 0.25/tick converges in ~40 ticks
    let mut s = Sim::new(flat(), Vec3::new(0.0, 1.5, 0.0));
    s.run(80, fwd());
    assert!((s.p.speed_h() - 16.5).abs() < 0.05, "{}", s.p.speed_h());
}

#[test]
fn jump_velocity_and_apex() {
    // dv = 90 * 1500 * 2.6 / 100 * 0.008 = 28.08, then gravity -0.32 in the same step.
    // The apex is about 28.08^2 / (2*40) = 9.86 above the start.
    let mut s = Sim::new(flat(), Vec3::new(0.0, 1.5, 0.0));
    let y0 = s.p.pos.y;
    s.frame(Input { jump_pressed: true, jump_held: true, ..default() });
    assert!(s.p.events.contains(&Event::Jump));
    s.frame(default());
    assert!((s.p.vel.y - (28.08 - 0.32)).abs() < 0.01, "vy = {}", s.p.vel.y);
    let mut apex = y0;
    for _ in 0..200 {
        s.frame(default());
        apex = apex.max(s.p.pos.y);
    }
    assert!((apex - y0 - 9.86).abs() < 0.25, "apex {}", apex - y0);
    assert!(s.p.gc.on_ground);
}

#[test]
fn dash_speed_and_duration() {
    // boostLeft 100, -4 per tick → 25 ticks at 3 * 16.5 = 49.5 u/s, then 16.5
    let mut s = Sim::new(flat(), Vec3::new(0.0, 1.5, 0.0));
    s.frame(Input { dash_pressed: true, ..default() });
    // -100, then Update regenerates 70/s for one frame
    assert!((s.p.boost_charge - (200.0 + 70.0 * FIXED_DT)).abs() < 1e-3);
    let x0 = s.p.pos;
    let mut fast = 0;
    for _ in 0..40 {
        s.frame(default());
        if s.p.speed_h() > 49.0 {
            fast += 1;
        }
    }
    assert_eq!(fast, 25);
    let d = (s.p.pos - x0).with_y(0.0).length();
    assert!(d > 9.9, "dash distance {d}");
}

#[test]
fn slide_from_standstill_is_24() {
    // 750 * 0.008 * 4 * preSlideSpeed(<=1 → 1) = 24
    let mut s = Sim::new(flat(), Vec3::new(0.0, 1.5, 0.0));
    s.frame(Input { slide_pressed: true, ..default() });
    assert!(s.p.sliding);
    assert_eq!(s.p.collider_height, SLIDE_HEIGHT);
    s.run(20, default());
    assert!((s.p.speed_h() - 24.0).abs() < 0.05, "{}", s.p.speed_h());
    s.frame(Input { slide_released: true, ..default() });
    s.run(5, default());
    assert!(!s.p.sliding);
    assert_eq!(s.p.collider_height, STAND_HEIGHT);
    assert!((s.p.pos.y - 1.5).abs() < 0.05, "stood up at {}", s.p.pos.y);
}

#[test]
fn air_strafe_caps_at_16_5() {
    let mut s = Sim::new(flat(), Vec3::new(0.0, 200.0, 0.0));
    s.p.fall_time = 2.0;
    s.run(150, fwd());
    assert!(!s.p.gc.on_ground);
    assert!(s.p.speed_h() <= 16.6 && s.p.speed_h() > 15.0, "{}", s.p.speed_h());
}

#[test]
fn slam_then_slam_jump() {
    // Slam sets v = (0,-100,0). On landing superJumpChance = 0.1, and a jump in the window
    // uses k = 3 + (slamForce - 1).
    let mut s = Sim::new(flat(), Vec3::new(0.0, 1.5, 0.0));
    s.p.pos.y = 40.0;
    s.p.prev_pos = s.p.pos;
    s.run(70, default()); // fall a bit: fallTime > 0.5
    s.frame(Input { slide_pressed: true, ..default() });
    assert!(s.p.gc.heavy_fall, "slam should start");
    assert_eq!(s.p.vel.y, -100.0);
    let mut landed = false;
    for _ in 0..100 {
        s.frame(default());
        if s.p.gc.on_ground {
            landed = true;
            break;
        }
    }
    assert!(landed);
    assert!(s.p.gc.super_jump_chance > 0.0 || s.p.gc.extra_jump_chance > 0.0);
    let slam_force = s.p.slam_force;
    assert!(slam_force > 1.0);
    s.frame(Input { jump_pressed: true, jump_held: true, ..default() });
    assert!(s.p.events.contains(&Event::SlamJump));
    s.frame(default());
    let k = if slam_force < 5.5 { 3.0 + (slam_force - 1.0) } else { 12.5 };
    let expected = 10.8 * k - 0.32;
    assert!((s.p.vel.y - expected).abs() < 0.5, "vy {} vs {}", s.p.vel.y, expected);
}

#[test]
fn wall_jump_kicks_away_and_up() {
    // Wall at x = 1..3. Away (-X) + up, each 2000*150/100*0.008 = 24 u/s (gravity -0.32).
    let mut w = flat();
    w.add(BoxCollider::new(Vec3::new(2.0, 20.0, 0.0), Vec3::new(2.0, 40.0, 40.0)));
    let mut s = Sim::new(w, Vec3::new(0.0, 1.5, 0.0));
    s.p.pos = Vec3::new(0.45, 20.0, 0.0);
    s.p.prev_pos = s.p.pos;
    s.run(30, default());
    assert!(s.p.wall.on_wall, "wall check should see the wall");
    for n in 1..=4 {
        s.p.jump_cooldown = false;
        s.frame(Input { jump_pressed: true, jump_held: true, ..default() });
        if n <= 3 {
            assert!(s.p.events.contains(&Event::WallJump(n)), "walljump {n}");
        } else {
            assert!(!s.p.events.iter().any(|e| matches!(e, Event::WallJump(4))), "max 3 wall jumps");
        }
        s.frame(default()); // queued wall-jump force is applied here
        if n == 1 {
            assert!((s.p.vel.y - (24.0 - 0.32)).abs() < 0.3, "vy {}", s.p.vel.y);
            assert!((s.p.vel.x + 24.0).abs() < 0.5, "vx {}", s.p.vel.x);
        }
        // drift back onto the wall for the next one
        s.p.pos = Vec3::new(0.45, 20.0, 0.0);
        s.p.prev_pos = s.p.pos;
        s.p.vel = Vec3::ZERO;
        s.run(3, default());
        s.p.events.clear();
    }
}

#[test]
fn coyote_time_is_0_2s() {
    // fallTime += dt*5 → `falling` after 0.2 s off the ground; ground jumps need !falling.
    let mut w = World::default();
    w.add(BoxCollider::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(4.0, 1.0, 4.0)));
    let mut s = Sim::new(w, Vec3::new(0.0, 1.5, 0.0));
    s.p.pos.x = 1.7;
    s.p.vel = Vec3::new(30.0, 0.0, 0.0);
    s.run(10, default()); // off the edge for ~0.08 s
    assert!(!s.p.gc.on_ground);
    s.frame(Input { jump_pressed: true, jump_held: true, ..default() });
    assert!(s.p.events.contains(&Event::Jump), "coyote jump should work");
}

#[test]
fn climb_step_onto_low_ledge() {
    // ClimbStep: walking into a 1.0-high step lifts V1 onto it.
    let mut w = flat();
    w.add(BoxCollider::new(Vec3::new(0.0, 0.5, -10.0), Vec3::new(10.0, 1.0, 10.0)));
    let mut s = Sim::new(w, Vec3::new(0.0, 1.5, 0.0));
    s.run(100, fwd());
    assert!(s.p.pos.z < -6.0, "should have walked onto the step: {:?}", s.p.pos);
    assert!((s.p.pos.y - 2.5).abs() < 0.2, "should stand on the step top (y=1 -> 2.5): {:?}", s.p.pos);
}

#[test]
fn climb_step_not_onto_tall_wall() {
    let mut w = flat();
    w.add(BoxCollider::new(Vec3::new(0.0, 2.5, -10.0), Vec3::new(10.0, 5.0, 10.0)));
    let mut s = Sim::new(w, Vec3::new(0.0, 1.5, 0.0));
    s.run(100, fwd());
    assert!(s.p.pos.z > -5.6, "a 5-high wall must block: {:?}", s.p.pos);
}
