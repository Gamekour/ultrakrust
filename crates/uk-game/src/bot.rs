//! A simple autopilot used to test level traversal headlessly and to drive the
//! showcase recording: walks toward waypoints, jumps/dashes when stuck, punches
//! what blocks it, and shoots active enemies with the revolver.

use crate::game::Game;
use bevy_math::{Vec2, Vec3};
use uk_core::player::Input;

#[derive(Clone, Debug)]
pub struct Waypoint {
    pub pos: Vec3,
    pub label: &'static str,
    /// Reached when within this horizontal distance (and roughly at that height).
    pub radius: f32,
}

#[derive(Clone, Debug)]
pub struct Bot {
    pub route: Vec<Waypoint>,
    pub idx: usize,
    pub stuck_t: f32,
    pub best_dist: f32,
    pub since_progress: f32,
    pub shot_cd: f32,
    pub punch_cd: f32,
    pub jump_toggle: bool,
    pub slide_t: f32,
    pub strafe: f32,
    pub strafe_t: f32,
    pub log: Vec<String>,
    pub t: f32,
}

/// Bot output for one frame: player input, camera yaw/pitch, and actions.
#[derive(Clone, Copy, Debug, Default)]
pub struct BotFrame {
    pub input: Input,
    pub yaw_deg: f32,
    pub pitch_deg: f32,
    pub fire: bool,
    pub punch: bool,
}

impl Bot {
    pub fn new(route: Vec<Waypoint>) -> Self {
        Self {
            route,
            idx: 0,
            stuck_t: 0.0,
            best_dist: f32::MAX,
            since_progress: 0.0,
            shot_cd: 0.0,
            punch_cd: 0.0,
            jump_toggle: false,
            slide_t: 0.0,
            strafe: 1.0,
            strafe_t: 0.0,
            log: Vec::new(),
            t: 0.0,
        }
    }

    pub fn done(&self) -> bool {
        self.idx >= self.route.len()
    }

    pub fn think(&mut self, g: &Game, dt: f32) -> BotFrame {
        self.t += dt;
        self.shot_cd -= dt;
        self.punch_cd -= dt;
        let p = &g.s.player;
        let mut out = BotFrame { yaw_deg: p.yaw_deg, ..Default::default() };
        if !p.activated || g.s.dead {
            return out;
        }
        // enemies first: aim at the nearest visible active enemy
        let eye = p.pos + Vec3::Y * 1.4;
        let mut target: Option<(f32, Vec3)> = None;
        for e in &g.s.enemies {
            if !e.alive || !g.active(e.node) || e.spawn_t > 0.0 {
                continue;
            }
            let c = e.center() + Vec3::Y * 0.3;
            let d = c.distance(eye);
            if d > 80.0 {
                continue;
            }
            let dir = (c - eye) / d;
            let blocked = g.world.raycast(eye, dir, d).is_some_and(|h| h.distance < d - 1.5);
            if !blocked && target.is_none_or(|t| d < t.0) {
                target = Some((d, c));
            }
        }
        // dodge: dash sideways from incoming projectiles or an imminent beam
        let threat = g.s.projectiles.iter().any(|pr| {
            !pr.friendly && pr.pos.distance(eye) < 22.0 && (eye - pr.pos).dot(pr.vel) > 0.0
        }) || g.s.enemies.iter().any(|e| e.alive && e.beam_fire_t > 0.0 && e.beam_fire_t < 0.35);
        // strafe, switching sides every few seconds or when a wall is close on that side
        self.strafe_t += dt;
        let right = p.right();
        let side_blocked = g.world.raycast(p.pos, right * self.strafe, 3.0).is_some();
        if side_blocked || self.strafe_t > 3.0 {
            self.strafe = -self.strafe;
            self.strafe_t = 0.0;
        }
        let strafe = self.strafe;
        if threat && p.boost_charge >= 100.0 && self.punch_cd <= 0.2 {
            out.input.dash_pressed = true;
        }
        if let (Some((_, c)), true) = (target, g.s.has_revolver) {
            let dir = (c - eye).normalize_or_zero();
            out.yaw_deg = dir.x.atan2(-dir.z).to_degrees();
            out.pitch_deg = dir.y.clamp(-1.0, 1.0).asin().to_degrees();
            if self.shot_cd <= 0.0 {
                out.fire = true;
                self.shot_cd = 0.5;
            }
            // strafe while fighting
            out.input.move_axis = Vec2::new(strafe, 0.0);
            return out;
        }
        if let (Some((d, c)), false) = (target, g.s.has_revolver) {
            // fists only: walk up and punch
            let dir = (c - eye).normalize_or_zero();
            out.yaw_deg = dir.x.atan2(-dir.z).to_degrees();
            out.input.move_axis = Vec2::Y;
            if d < 3.5 && self.punch_cd <= 0.0 {
                out.punch = true;
                self.punch_cd = 0.5;
            }
            return out;
        }
        let Some(wp) = self.route.get(self.idx).cloned() else { return out };
        let to = wp.pos - p.pos;
        let flat = Vec2::new(to.x, to.z);
        if flat.length() < wp.radius && to.y.abs() < 8.0 {
            self.log.push(format!("{:7.1}s reached {:2} {} at {:?}", self.t, self.idx, wp.label, p.pos));
            self.idx += 1;
            self.best_dist = f32::MAX;
            self.since_progress = 0.0;
            return out;
        }
        out.yaw_deg = to.x.atan2(-to.z).to_degrees();
        out.pitch_deg = -8.0;
        out.input.move_axis = Vec2::Y;
        // keep a slide going under low obstacles, then release
        if self.slide_t > 0.0 {
            self.slide_t -= dt;
            if self.slide_t <= 0.0 {
                out.input.slide_released = true;
            }
            return out;
        }
        let fwd = Vec3::new(to.x, 0.0, to.z).normalize_or_zero();
        let feet = p.pos - Vec3::Y * 1.2;
        let chest = p.pos + Vec3::Y * 0.8;
        let low_clear = g.world.raycast(feet, fwd, 2.5).is_none();
        let high_blocked = g.world.raycast(chest, fwd, 2.5).is_some();
        if low_clear && high_blocked && p.gc.on_ground {
            out.input.slide_pressed = true;
            self.slide_t = 0.9;
            return out;
        }
        let d = to.length();
        if d < self.best_dist - 0.5 {
            self.best_dist = d;
            self.since_progress = 0.0;
        } else {
            self.since_progress += dt;
        }
        // stuck: punch what's ahead (planks, glass), then jump / dash
        if self.since_progress > 0.4 && self.punch_cd <= 0.0 {
            out.punch = true;
            out.pitch_deg = -25.0;
            self.punch_cd = 0.5;
        }
        if self.since_progress > 0.8 {
            self.jump_toggle = !self.jump_toggle;
            out.input.jump_pressed = self.jump_toggle;
            out.input.jump_held = true;
        }
        if self.since_progress > 2.5 && (self.since_progress * 2.0).fract() < 0.05 {
            out.input.dash_pressed = true;
        }
        if self.since_progress > 12.0 {
            self.log.push(format!("{:7.1}s STUCK before {:2} {} at {:?} (dist {:.1})", self.t, self.idx, wp.label, p.pos, d));
            self.since_progress = -1000.0;
        }
        out
    }
}

/// Route through 0-1 (Bevy coordinates), from the level's doors/checkpoints/arenas.
pub fn route_0_1() -> Vec<Waypoint> {
    let w = |x: f32, y: f32, z: f32, label: &'static str, radius: f32| Waypoint { pos: Vec3::new(x, y, z), label, radius };
    vec![
        w(0.0, -0.5, -262.0, "past planks", 2.0),
        w(0.0, 0.0, -279.0, "corridor end", 1.5),
        w(21.0, 0.0, -279.0, "corridor east", 1.5),
        w(21.0, 0.0, -339.0, "corridor south", 1.5),
        w(40.25, 0.0, -338.3, "corridor east 2", 0.6),
        w(40.25, 0.0, -376.0, "corridor south 2", 1.0),
        w(40.0, 0.0, -382.0, "gun room entrance", 2.0),
        w(40.0, 1.5, -393.0, "revolver pickup", 1.5),
        w(40.0, 1.0, -405.0, "gun room door", 2.0),
        w(41.0, 1.0, -419.0, "4 hallway", 2.0),
        w(60.0, 1.0, -422.0, "4 hallway east", 2.5),
        w(62.0, 1.0, -452.0, "4 hallway south", 2.5),
        w(41.0, 1.0, -452.0, "4 hallway west", 2.5),
        w(41.0, 1.0, -469.5, "door 1", 2.0),
        w(40.0, -10.0, -485.0, "checkpoint", 3.0),
        w(40.0, -8.0, -490.5, "door 2", 3.0),
        w(40.0, -6.0, -511.0, "glass hallway", 4.0),
        w(40.0, -8.0, -551.5, "door 3", 3.0),
        w(39.0, 11.0, -588.0, "fan room", 5.0),
        w(40.0, 11.0, -624.5, "door 4", 3.0),
        w(65.5, 19.0, -640.0, "door 5", 3.0),
        w(76.0, 18.0, -640.0, "checkpoint 4", 4.0),
        w(88.0, 36.0, -640.0, "projectile arena", 6.0),
        w(146.5, 30.0, -640.0, "door 6", 3.0),
        w(157.0, 28.0, -640.0, "checkpoint 1", 4.0),
        w(180.0, 30.0, -640.0, "combo hallway", 6.0),
        w(192.0, 30.0, -594.5, "door 7", 3.0),
        w(182.0, 56.0, -579.0, "projectile zombies room", 6.0),
        w(202.0, 53.0, -542.0, "checkpoint 5", 4.0),
        w(202.0, 54.0, -533.5, "door 8", 3.0),
        w(202.0, 53.0, -493.0, "boss hallway", 5.0),
        w(202.0, 54.0, -452.5, "door 10", 3.0),
        w(202.0, 53.0, -442.0, "checkpoint 2", 4.0),
        w(202.0, 54.0, -431.5, "door 9", 3.0),
        w(202.0, 60.0, -421.0, "boss arena", 6.0),
        w(202.0, 53.0, -401.0, "final door", 3.0),
        w(202.0, -15.0, -354.0, "final pit", 6.0),
    ]
}
