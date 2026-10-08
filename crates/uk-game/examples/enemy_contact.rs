//! Player vs enemy contact, for the first enemy of each type in the level. The enemy is activated
//! and held in place (a grounded enemy's Rigidbody is kinematic); the player starts 6 units away
//! with a clear line to it (on the floor, or in the air with gravity off for a floating enemy):
//! - walk: holds forward into it for 2 s. `gap` is the distance from the player's capsule surface
//!   to the enemy's solid colliders it collides with (negative = overlap); blocked if it never goes
//!   below -0.01 (PhysX contact offset).
//! - dash: dashes at it (layer 15 skips the enemy's layer 12) and reports whether it got past.
//! - top: dropped onto it; reports the jump taken on the first tick with GroundCheck canJump and
//!   not onGround (an enemy step).
//! cargo run --release -p uk-game --example enemy_contact -- [level0-1]
use bevy_math::{Vec2, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::collide::Capsule;
use uk_core::consts::{COLLIDER_CENTER_Y, FIXED_DT, RADIUS, STAND_HEIGHT};
use uk_core::player::{Event, Input, PLAYER_LAYER_COLLIDES};
use uk_game::enemy::Kind;
use uk_game::Game;

fn activate(g: &mut Game, n: u32) {
    let mut chain = vec![n];
    while let Some(p) = g.def.nodes[*chain.last().unwrap() as usize].parent {
        chain.push(p);
    }
    for &m in chain.iter().rev() {
        g.set_active(m, true);
    }
}

struct Run {
    g: Game,
    e: usize,
    hold: Vec3,
    t: f64,
    gravity: bool,
}

impl Run {
    fn new(def: &Arc<scenedef::SceneDef>, e: usize) -> Self {
        let mut g = Game::new(def.clone());
        let node = g.s.enemies[e].node;
        activate(&mut g, node);
        // only this enemy (its container activates its siblings too)
        for k in 0..g.s.enemies.len() {
            let n = g.s.enemies[k].node;
            if k != e && g.s.active[n as usize] {
                g.set_active(n, false);
            }
        }
        g.s.player.activated = true;
        let hold = g.s.enemies[e].pos;
        let mut r = Run { g, e, hold, t: 0.0, gravity: true };
        // let the enemy settle onto its floor with the player out of the way
        r.g.s.player.pos = hold + Vec3::Y * 500.0;
        r.gravity = false;
        for _ in 0..60 {
            r.tick(&Input::default());
        }
        r.gravity = true;
        r.hold = r.g.s.enemies[e].pos;
        r
    }

    fn tick(&mut self, input: &Input) {
        let g = &mut self.g;
        g.s.hp = 100;
        g.s.player.use_gravity = self.gravity;
        if !self.gravity {
            g.s.player.vel.y = 0.0;
        }
        g.fixed_update(input);
        self.t += FIXED_DT as f64;
        g.update(input, FIXED_DT, self.t);
        let en = &mut g.s.enemies[self.e];
        en.pos.x = self.hold.x;
        en.pos.z = self.hold.z;
        en.vel = Vec3::ZERO;
        en.cooldown = 99.0;
        // the player walking into arena triggers would wake other enemies: keep this one alone
        for k in 0..g.s.enemies.len() {
            let n = g.s.enemies[k].node;
            if k != self.e && g.s.active[n as usize] {
                g.set_active(n, false);
            }
        }
    }

    /// Player capsule surface to the enemy's solid colliders on layers the player's layer 2 hits.
    fn gap(&self) -> f32 {
        let b = &self.g.s.player.bodies;
        let grp = &b.world.groups[self.e + 1];
        let cap = self.g.s.player.capsule();
        // the enemy's pose now (the group transform is synced at the start of the next tick)
        let inv = self.g.s.enemies[self.e].delta().inverse();
        let (a, c) = (inv.transform_point3(cap.a), inv.transform_point3(cap.b));
        let mut best = f32::MAX;
        for (i, s) in grp.shapes.iter().enumerate() {
            let o = grp.owners[i] as usize;
            if b.trigger[o] || PLAYER_LAYER_COLLIDES & (1 << b.layer[o]) == 0 || !b.enabled(o as u32) {
                continue;
            }
            let (p, q) = s.closest_to_segment(a, c);
            best = best.min(p.distance(q) - RADIUS);
        }
        best
    }

    fn center(&self) -> Vec3 {
        self.g.s.enemies[self.e].center()
    }

    fn free(&self, p: Vec3) -> bool {
        self.g.world.overlap_capsule(Capsule::unity(p + Vec3::Y * COLLIDER_CENTER_Y, STAND_HEIGHT, RADIUS)).is_empty()
    }

    /// 6 units out with a clear line to the enemy: on its floor, else level with its centre.
    fn start(&self) -> Option<(Vec3, bool)> {
        let en = &self.g.s.enemies[self.e];
        let feet = en.pos.y + en.center_y - en.half_height.max(en.radius);
        let dirs: Vec<Vec3> = (0..16)
            .map(|k| {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                Vec3::new(a.cos(), 0.0, a.sin())
            })
            .collect();
        let clear = |p: Vec3, to: Vec3| self.g.world.raycast(p, to - p, p.distance(to)).is_none();
        dirs.iter()
            .find_map(|&d| {
                let p = Vec3::new(en.pos.x, feet + 1.65, en.pos.z) + d * 6.0;
                let chest = p + Vec3::Y * 0.5;
                let floor = self.g.world.raycast(p, Vec3::NEG_Y, 2.5).is_some_and(|h| (1.5..2.1).contains(&h.distance));
                (floor && clear(chest, Vec3::new(en.pos.x, chest.y, en.pos.z)) && self.free(p)).then_some((p, true))
            })
            .or_else(|| {
                let c = self.center();
                dirs.iter().find_map(|&d| {
                    let p = c - Vec3::Y * COLLIDER_CENTER_Y + d * (6.0 + en.radius);
                    (clear(p, c) && self.free(p)).then_some((p, false))
                })
            })
    }

    fn place(&mut self, p: Vec3, gravity: bool) {
        self.gravity = gravity;
        let c = self.center();
        let pl = &mut self.g.s.player;
        pl.pos = p;
        pl.prev_pos = p;
        pl.vel = Vec3::ZERO;
        let d = c - p;
        pl.yaw_deg = d.x.atan2(-d.z).to_degrees();
    }
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let probe = Game::new(def.clone());
    let mut kinds: Vec<(Kind, usize)> = Vec::new();
    for (i, en) in probe.s.enemies.iter().enumerate() {
        if !kinds.iter().any(|(k, _)| *k == en.kind) {
            kinds.push((en.kind, i));
        }
    }
    drop(probe);
    let fwd = Input { move_axis: Vec2::new(0.0, 1.0), ..Default::default() };
    for (kind, e) in kinds {
        let mut r = Run::new(&def, e);
        let en = r.g.s.enemies[e].clone();
        println!("{kind:?} #{e} {} at {:?}: root collider r {:.2}", def.path(en.node), r.hold, en.radius);
        let Some((start, grounded)) = r.start() else {
            println!("  no clear start");
            continue;
        };
        // walk
        r.place(start, grounded);
        let mut min = f32::MAX;
        let mut closest = f32::MAX;
        for _ in 0..250 {
            r.tick(&fwd);
            min = min.min(r.gap());
            closest = closest.min((r.g.s.player.pos - r.center()).with_y(0.0).length());
        }
        println!(
            "  walk ({}): min gap {min:.4}, closest axis distance {closest:.3} -> {}",
            if grounded { "floor" } else { "air" },
            if min >= -0.011 { "blocked" } else { "PENETRATES" }
        );
        // dash
        let mut r = Run::new(&def, e);
        r.place(start, grounded);
        let dir = (r.center() - start).with_y(0.0).normalize();
        let mut past = f32::MIN;
        let mut min = f32::MAX;
        for i in 0..120 {
            let input = Input { dash_pressed: i == 2, ..fwd };
            r.tick(&input);
            past = past.max((r.g.s.player.pos - r.center()).with_y(0.0).dot(dir));
            min = min.min(r.gap());
        }
        let after = r.gap();
        println!(
            "  dash: min gap {min:.3}, furthest {past:.2} past the centre, gap after {after:.3} -> {}",
            if min < -0.011 && past > en.radius + RADIUS {
                "passes through"
            } else if past > en.radius + RADIUS {
                "deflected around"
            } else {
                "stopped"
            }
        );
        // top: drop onto it, jump on the first tick with canJump && !onGround
        let mut r = Run::new(&def, e);
        let top = r.center().y + en.half_height.max(en.radius);
        r.place(Vec3::new(r.center().x + 0.05, top + 1.5 + 2.0, r.center().z), true);
        let mut seen = None;
        let mut jumped = None;
        let mut rest = None;
        for i in 0..250 {
            if i == 120 {
                let p = &r.g.s.player;
                rest = Some((p.gc.on_ground, p.gc.can_jump, p.pos.y - 1.5 - top));
            }
            let p = &r.g.s.player;
            let want = seen.is_none() && p.gc.can_jump && !p.gc.on_ground;
            if want {
                seen = Some((i, p.pos.y - 1.5 - top));
            }
            let input = Input { jump_pressed: want, jump_held: want, ..Default::default() };
            r.g.s.player.events.clear();
            let vy = r.g.s.player.vel.y;
            r.tick(&input);
            if want {
                let j = r.g.s.player.events.contains(&Event::Jump);
                r.tick(&Input::default());
                jumped = Some((j, vy, r.g.s.player.vel.y));
            }
        }
        match (seen, jumped) {
            (Some((i, h)), Some((j, v0, v1))) => {
                println!("  top: canJump && !onGround at tick {i}, feet {h:+.2} from the collider top; jump {j}, vel.y {v0:.2} -> {v1:.2} a tick later")
            }
            _ => println!("  top: never canJump && !onGround"),
        }
        if let (None, Some((g, c, h))) = (seen, rest) {
            println!("  top: after 120 ticks onGround {g} canJump {c}, feet {h:+.2} from the collider top");
        }
    }
}
