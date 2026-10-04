//! Enemies and combat. Values come from the scene (health, bite damage) and from
//! the decompiled AI on Standard difficulty; movement is a direct-pursuit
//! approximation of Unity's NavMeshAgent (no navmesh yet).

use crate::game::{Game, GameEvent};
use crate::scripts::Script;
use bevy_math::{Affine3A, Quat, Vec3};
use uk_assets::scenedef::{tags, SceneDef, ShapeDef};
use uk_core::collide::{BoxCollider, Capsule, Shape, Triangle};
use uk_core::consts::{FIXED_DT, GRAVITY};
use uk_core::umath::move_towards;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    /// Zombie + ZombieMelee ("Filth")
    Filth,
    /// Zombie + ZombieProjectiles ("Stray")
    Stray,
    /// SpiderBody ("Malicious Face")
    MaliciousFace,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HitZone {
    Head,
    Limb,
    Body,
}

#[derive(Clone, Debug)]
pub struct Hitbox {
    pub shape: ShapeDef,
    pub zone: HitZone,
}

#[derive(Clone, Debug)]
pub struct Enemy {
    pub node: u32,
    pub kind: Kind,
    pub eid_script: u32,
    pub pos0: Vec3,
    pub yaw0: f32,
    pub pos: Vec3,
    pub vel: Vec3,
    pub yaw: f32,
    pub health: f32,
    pub alive: bool,
    pub dead_time: f32,
    pub spawn_t: f32,
    pub spawn_in: bool,
    pub radius: f32,
    pub half_height: f32,
    pub center_y: f32,
    pub grounded: bool,
    pub cooldown: f32,
    pub attack_t: f32,
    pub attacking: bool,
    pub hit_done: bool,
    pub bite_damage: i32,
    pub hitboxes: Vec<Hitbox>,
    pub activate_on_death: Vec<u32>,
    pub counted: bool,
}

#[derive(Clone, Debug)]
pub struct Projectile {
    pub pos: Vec3,
    pub vel: Vec3,
    pub damage: f32,
    pub friendly: bool,
    pub life: f32,
}

fn subtree(def: &SceneDef, root: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(m) = stack.pop() {
        out.push(m);
        stack.extend(def.nodes[m as usize].children.iter().copied());
    }
    out
}

impl Enemy {
    pub fn from_def(def: &SceneDef, node: u32, eid_script: u32) -> Option<Self> {
        let classes: Vec<&str> = def.scripts_on(node).map(|(_, s)| s.class.as_str()).collect();
        let kind = if classes.contains(&"ZombieMelee") {
            Kind::Filth
        } else if classes.contains(&"ZombieProjectiles") {
            Kind::Stray
        } else if classes.iter().any(|c| *c == "SpiderBody" || *c == "MaliciousFace") {
            Kind::MaliciousFace
        } else {
            Kind::Other
        };
        let eid = &def.scripts[eid_script as usize].data;
        let health = def
            .scripts_on(node)
            .find(|(_, s)| s.class == "Enemy" || s.class == "SpiderBody" || s.class == "MaliciousFace")
            .map(|(_, s)| s.data.get("health").f32())
            .filter(|h| *h > 0.0)
            .unwrap_or_else(|| eid.get("health").f32().max(1.0));
        let (_, rot, pos0) = def.nodes[node as usize].world0.to_scale_rotation_translation();
        let fwd = rot * Vec3::NEG_Z;
        let yaw0 = fwd.x.atan2(-fwd.z);
        let nodes = subtree(def, node);
        let mut radius = 0.5;
        let mut half_height = 1.0;
        let mut center_y = 1.0;
        let mut hitboxes = Vec::new();
        let mut bite_damage = 30;
        for (_, c) in def.colliders.iter().enumerate() {
            if !nodes.contains(&c.node) {
                continue;
            }
            if c.node == node && !c.trigger {
                if let ShapeDef::Capsule { a, b, radius: r } = &c.shape {
                    radius = *r;
                    half_height = a.distance(*b) * 0.5 + r;
                    center_y = ((*a + *b) * 0.5 - pos0).y;
                }
            }
            if c.trigger {
                continue;
            }
            let tag = def.nodes[c.node as usize].tag;
            let zone = match tag {
                tags::HEAD => HitZone::Head,
                tags::LIMB | tags::END_LIMB => HitZone::Limb,
                _ => HitZone::Body,
            };
            hitboxes.push(Hitbox { shape: c.shape.clone(), zone });
        }
        for (_, s) in def.scripts.iter().enumerate().filter(|(_, s)| s.class == "SwingCheck2" && nodes.contains(&s.node)) {
            bite_damage = s.data.get("damage").i64() as i32;
            break;
        }
        let activate_on_death = crate::scripts::nodes(def, eid.get("activateOnDeath"));
        Some(Self {
            node,
            kind,
            eid_script,
            pos0,
            yaw0,
            pos: pos0,
            vel: Vec3::ZERO,
            yaw: yaw0,
            health,
            alive: true,
            dead_time: 0.0,
            spawn_t: 0.0,
            spawn_in: eid.get("spawnIn").bool(),
            radius,
            half_height,
            center_y,
            grounded: false,
            cooldown: 0.0,
            attack_t: 0.0,
            attacking: false,
            hit_done: false,
            bite_damage,
            hitboxes,
            activate_on_death,
            counted: false,
        })
    }

    /// Transform from the load-time pose to the current pose.
    pub fn delta(&self) -> Affine3A {
        Affine3A::from_translation(self.pos)
            * Affine3A::from_quat(Quat::from_rotation_y(-(self.yaw - self.yaw0)))
            * Affine3A::from_translation(-self.pos0)
    }

    fn capsule(&self) -> Capsule {
        Capsule::unity(self.pos + Vec3::Y * self.center_y, (self.half_height * 2.0).max(self.radius * 2.0), self.radius)
    }

    pub fn center(&self) -> Vec3 {
        self.pos + Vec3::Y * self.center_y
    }

    /// Ray against this enemy's hitboxes (world space). Returns (distance, zone).
    pub fn raycast(&self, o: Vec3, d: Vec3, max: f32) -> Option<(f32, HitZone)> {
        let inv = self.delta().inverse();
        let lo = inv.transform_point3(o);
        let ld = inv.transform_vector3(d);
        let mut best: Option<(f32, HitZone)> = None;
        for h in &self.hitboxes {
            let t = match &h.shape {
                ShapeDef::Box { center, half, rot } => {
                    // a ray starting inside still counts (point blank)
                    let b = BoxCollider { center: *center, half: *half, rot: *rot, slippery: false };
                    if b.closest_point(lo).distance_squared(lo) < 1e-6 { Some(0.0) } else { b.raycast(lo, ld, max).map(|r| r.0) }
                }
                ShapeDef::Sphere { center, radius } => ray_sphere(lo, ld, *center, *radius, max),
                ShapeDef::Capsule { a, b, radius } => ray_capsule(lo, ld, *a, *b, *radius, max),
                ShapeDef::Mesh(tris) => tris.iter().filter_map(|t| Triangle::new(t[0], t[1], t[2]).raycast(lo, ld, max).map(|r| r.0)).reduce(f32::min),
            };
            if let Some(t) = t {
                if best.is_none_or(|b| t < b.0 || (t - b.0 < 0.05 && h.zone == HitZone::Head)) {
                    best = Some((t, h.zone));
                }
            }
        }
        best
    }
}

fn ray_sphere(o: Vec3, d: Vec3, c: Vec3, r: f32, max: f32) -> Option<f32> {
    let m = o - c;
    let b = m.dot(d);
    let cc = m.length_squared() - r * r;
    if cc > 0.0 && b > 0.0 {
        return None;
    }
    let disc = b * b - cc;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()).max(0.0);
    (t <= max).then_some(t)
}

fn ray_capsule(o: Vec3, d: Vec3, a: Vec3, b: Vec3, r: f32, max: f32) -> Option<f32> {
    // march: capsules are small, a fine sample is accurate enough for hit tests
    let step = (r * 0.25).max(0.02);
    let mut t = 0.0;
    while t <= max {
        let p = o + d * t;
        let ab = b - a;
        let s = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        if (a + ab * s).distance(p) <= r {
            return Some(t);
        }
        t += step;
    }
    None
}

pub fn on_enable(g: &mut Game, e: usize) {
    let en = &mut g.s.enemies[e];
    if en.alive && en.spawn_in && en.spawn_t == 0.0 {
        en.spawn_t = 0.6;
    }
}

fn player_target(g: &Game) -> Vec3 {
    g.s.player.pos + Vec3::Y * 0.5
}

/// Movement + collisions at the fixed rate.
pub fn fixed_update(g: &mut Game) {
    let dt = FIXED_DT;
    let target = player_target(g);
    for i in 0..g.s.enemies.len() {
        let node = g.s.enemies[i].node;
        if !g.s.active[node as usize] {
            continue;
        }
        let en = &mut g.s.enemies[i];
        if !en.alive || en.kind == Kind::MaliciousFace || en.kind == Kind::Other {
            continue;
        }
        if en.spawn_t > 0.0 {
            continue;
        }
        let to = target - en.pos;
        let flat = Vec3::new(to.x, 0.0, to.z);
        let dist = flat.length();
        let dir = flat.normalize_or_zero();
        let (speed, accel) = match en.kind {
            Kind::Filth => (20.0, 30.0),
            Kind::Stray => (10.0, 30.0),
            _ => (0.0, 0.0),
        };
        let desired = match en.kind {
            _ if en.attacking => Vec3::ZERO,
            Kind::Filth if dist > 2.5 => dir * speed,
            // Strays keep their distance: back off inside flee range, close in beyond shoot range
            Kind::Stray if dist < 15.0 => -dir * speed,
            Kind::Stray if dist > 30.0 => dir * speed,
            _ => Vec3::ZERO,
        };
        let h = Vec3::new(en.vel.x, 0.0, en.vel.z);
        let nh = uk_core::umath::vmove_towards(h, desired, accel * dt * 2.0);
        en.vel.x = nh.x;
        en.vel.z = nh.z;
        en.vel.y += GRAVITY * dt;
        en.vel.y = en.vel.y.max(-100.0);
        if dist > 0.1 {
            let want = dir.x.atan2(-dir.z);
            let diff = (want - en.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            en.yaw += diff.clamp(-8.0 * dt, 8.0 * dt);
        }
        // integrate with collisions
        en.pos += en.vel * dt;
        let mut cap = en.capsule();
        let before = cap.a;
        let mut normals = Vec::new();
        g.world.depenetrate_capsule(&mut cap, &mut normals);
        let en = &mut g.s.enemies[i];
        en.pos += cap.a - before;
        en.grounded = false;
        for n in normals {
            if n.y > 0.5 {
                en.grounded = true;
            }
            let into = en.vel.dot(n);
            if into < 0.0 {
                en.vel -= n * into;
            }
        }
        if en.pos.y < -1000.0 {
            en.alive = false;
        }
    }
    // projectiles
    let cap = g.s.player.capsule();
    let mut hits = Vec::new();
    let world = &g.world;
    g.s.projectiles.retain_mut(|p| {
        p.life -= dt;
        let step = p.vel * dt;
        if let Some(h) = world.raycast(p.pos, step, step.length()) {
            let _ = h;
            return false;
        }
        p.pos += step;
        if !p.friendly {
            let ab = cap.b - cap.a;
            let s = ((p.pos - cap.a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            if (cap.a + ab * s).distance(p.pos) < cap.radius + 0.5 {
                hits.push(p.damage as i32);
                return false;
            }
        }
        p.life > 0.0
    });
    for d in hits {
        g.hurt_player(d, true);
    }
    // friendly (parried) projectiles hit enemies
    let parried: Vec<(usize, Vec3)> = g.s.projectiles.iter().enumerate().filter(|(_, p)| p.friendly).map(|(i, p)| (i, p.pos)).collect();
    let mut remove = Vec::new();
    for (pi, pos) in parried {
        for e in 0..g.s.enemies.len() {
            let en = &g.s.enemies[e];
            if en.alive && g.s.active[en.node as usize] && en.center().distance(pos) < en.radius + 1.0 {
                damage_enemy(g, e, 5.0, HitZone::Body, pos);
                remove.push(pi);
                break;
            }
        }
    }
    remove.sort();
    for i in remove.into_iter().rev() {
        g.s.projectiles.remove(i);
    }
}

/// Attacks, timers, deaths (per frame).
pub fn update(g: &mut Game, dt: f32) {
    let target = player_target(g);
    let player_pos = g.s.player.pos;
    for i in 0..g.s.enemies.len() {
        let node = g.s.enemies[i].node;
        if !g.s.active[node as usize] {
            continue;
        }
        let en = &mut g.s.enemies[i];
        if !en.alive {
            en.dead_time += dt;
            if en.dead_time > 0.25 && !en.counted {
                en.counted = true;
            }
            continue;
        }
        if en.spawn_t > 0.0 {
            en.spawn_t = (en.spawn_t - dt).max(0.0);
            if en.spawn_t == 0.0 {
                en.spawn_t = -1.0;
            }
            continue;
        }
        let dist = (target - en.center()).length();
        let flat = Vec3::new(target.x - en.pos.x, 0.0, target.z - en.pos.z);
        match en.kind {
            Kind::Filth => {
                if en.cooldown > 0.0 {
                    en.cooldown = move_towards(en.cooldown, 0.0, 0.4 * dt);
                }
                if !en.attacking && en.cooldown <= 0.0 && en.grounded && flat.length() < 3.0 {
                    en.attacking = true;
                    en.attack_t = 0.0;
                    en.hit_done = false;
                }
                if en.attacking {
                    en.attack_t += dt;
                    // bite: damage window during the lunge
                    if !en.hit_done && en.attack_t > 0.45 && en.attack_t < 0.65 {
                        let fwd = Vec3::new(en.yaw.sin(), 0.0, -en.yaw.cos());
                        let to = player_pos - en.pos;
                        if to.length() < 3.5 && fwd.dot(Vec3::new(to.x, 0.0, to.z).normalize_or_zero()) > 0.3 {
                            en.hit_done = true;
                            let d = en.bite_damage;
                            g.hurt_player(d, true);
                        }
                    }
                    let en = &mut g.s.enemies[i];
                    if en.attack_t > 1.0 {
                        en.attacking = false;
                        en.cooldown = 0.5;
                    }
                }
            }
            Kind::Stray => {
                if en.cooldown > 0.0 {
                    en.cooldown = move_towards(en.cooldown, 0.0, dt);
                }
                let in_range = dist < if en.cooldown <= 0.0 { 60.0 } else { 30.0 };
                if !en.attacking && en.cooldown <= 0.0 && in_range {
                    en.attacking = true;
                    en.attack_t = 0.0;
                    en.hit_done = false;
                }
                if en.attacking {
                    en.attack_t += dt;
                    if !en.hit_done && en.attack_t > 0.55 {
                        en.hit_done = true;
                        let from = en.center() + Vec3::Y * 1.0;
                        let dir = (target + Vec3::Y * 1.0 - from).normalize_or_zero();
                        g.s.projectiles.push(Projectile { pos: from + dir, vel: dir * 65.0, damage: 25.0, friendly: false, life: 10.0 });
                    }
                    let en = &mut g.s.enemies[i];
                    if en.attack_t > 0.9 {
                        en.attacking = false;
                        en.cooldown = 1.0 + pseudo_rand(i as f32 + g_time_seed(dist)) * 1.5;
                    }
                }
            }
            Kind::MaliciousFace => {
                if en.cooldown > 0.0 {
                    en.cooldown = move_towards(en.cooldown, 0.0, dt);
                }
                let dir = (target - en.center()).normalize_or_zero();
                en.yaw = dir.x.atan2(-dir.z);
                if en.cooldown <= 0.0 && dist < 150.0 {
                    en.cooldown = 2.0;
                    let from = en.center();
                    for k in -1..=1 {
                        let spread = Quat::from_rotation_y(k as f32 * 0.08) * dir;
                        g.s.projectiles.push(Projectile { pos: from + spread * 4.0, vel: spread * 65.0, damage: 25.0, friendly: false, life: 10.0 });
                    }
                }
            }
            Kind::Other => {}
        }
    }
}

fn pseudo_rand(x: f32) -> f32 {
    ((x * 12.9898).sin() * 43758.547).fract().abs()
}

fn g_time_seed(d: f32) -> f32 {
    d * 7.13
}

pub fn damage_enemy(g: &mut Game, e: usize, base: f32, zone: HitZone, hit_pos: Vec3) {
    let en = &mut g.s.enemies[e];
    if !en.alive {
        return;
    }
    let mut m = base;
    // Enemy.GetHurt: zombies take x1.5 in the air
    if matches!(en.kind, Kind::Filth | Kind::Stray) && !en.grounded {
        m *= 1.5;
    }
    let limb = match zone {
        HitZone::Head => 1.0,
        HitZone::Limb => 0.5,
        HitZone::Body => 0.0,
    };
    let crit = 1.0;
    let dmg = m + m * limb * crit;
    en.health -= dmg;
    // knockback
    let push = (en.center() - g.s.player.pos).normalize_or_zero();
    en.vel += Vec3::new(push.x, 0.3, push.z) * 4.0 * base;
    let pos = en.center();
    let head = zone == HitZone::Head;
    let dead = en.health <= 0.0;
    g.events.push(GameEvent::EnemyHit { pos: hit_pos, damage: dmg, head });
    // Blood within reach heals (Bloodsplatter: 3 per hit, 10 for big splashes)
    if pos.distance(g.s.player.pos) < 9.0 {
        g.heal_player(if dead || head { 10 } else { 3 });
    }
    if dead {
        kill_enemy(g, e);
    }
}

pub fn kill_enemy(g: &mut Game, e: usize) {
    let en = &mut g.s.enemies[e];
    if !en.alive {
        return;
    }
    en.alive = false;
    en.attacking = false;
    let (node, aod, pos) = (en.node, en.activate_on_death.clone(), en.center());
    g.s.kills += 1;
    g.events.push(GameEvent::EnemyKilled { pos });
    for n in aod {
        g.set_active(n, true);
    }
    g.add_dead_enemy(node);
    // corpse disappears (gibs/ragdoll not ported)
    g.set_active(node, false);
}

// ---------------------------------------------------------------- player weapons

impl Game {
    /// Revolver hitscan. Normal beams stop at the first body; piercing beams pass through enemies.
    pub fn fire_revolver(&mut self, eye: Vec3, dir: Vec3, pierce: bool) {
        let dir = dir.normalize_or_zero();
        let max = 1000.0;
        let env = self.world.raycast(eye, dir, max);
        let env_t = env.map(|h| h.distance).unwrap_or(max);
        let mut hits: Vec<(f32, usize, HitZone)> = Vec::new();
        for (i, en) in self.s.enemies.iter().enumerate() {
            if !en.alive || !self.s.active[en.node as usize] || en.spawn_t > 0.0 {
                continue;
            }
            if let Some((t, z)) = en.raycast(eye, dir, env_t) {
                hits.push((t, i, z));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let damage = if pierce { 2.0 } else { 1.0 };
        let mut end = eye + dir * env_t;
        if pierce {
            for (t, i, z) in hits {
                damage_enemy(self, i, damage, z, eye + dir * t);
            }
            self.hit_environment(env, damage);
        } else if let Some(&(t, i, z)) = hits.first() {
            end = eye + dir * t;
            damage_enemy(self, i, damage, z, end);
        } else {
            self.hit_environment(env, damage);
        }
        self.events.push(GameEvent::Shot { from: eye, to: end, pierce });
    }

    fn hit_environment(&mut self, hit: Option<uk_core::collide::RayHit>, damage: f32) {
        let Some(h) = hit else { return };
        let owner = self.world.owner(h.collider);
        if owner == uk_core::collide::ALWAYS {
            return;
        }
        let node = self.def.colliders[owner as usize].node;
        let scs: Vec<u32> = self.def.scripts_on(node).map(|(i, _)| i).collect();
        for sc in scs {
            let (is_breakable, is_glass) =
                (matches!(self.s.scripts[sc as usize], Script::Breakable(_)), matches!(self.s.scripts[sc as usize], Script::Glass(_)));
            if is_breakable {
                self.breakable_break(sc, damage);
            } else if is_glass {
                self.glass_shatter(sc);
            }
        }
    }

    /// Feedbacker punch: enemies within 4 (ray, then r=1 sphere cast), else breakables; parries projectiles.
    pub fn punch(&mut self, eye: Vec3, dir: Vec3) {
        let dir = dir.normalize_or_zero();
        // parry: projectiles in the zone in front of the camera
        let mut parried = false;
        for p in &mut self.s.projectiles {
            if !p.friendly && (p.pos - (eye + dir * 3.0)).length() < 3.5 {
                p.friendly = true;
                p.vel = dir * p.vel.length().max(65.0) * 1.5;
                p.damage *= 2.0;
                parried = true;
            }
        }
        if parried {
            self.heal_player(100);
            self.events.push(GameEvent::PunchHit);
            return;
        }
        let mut best: Option<(f32, usize, HitZone)> = None;
        for (i, en) in self.s.enemies.iter().enumerate() {
            if !en.alive || !self.s.active[en.node as usize] || en.spawn_t > 0.0 {
                continue;
            }
            let hit = en.raycast(eye, dir, 4.0).or_else(|| {
                let c = en.center();
                let t = (c - eye).dot(dir).clamp(0.0, 4.0);
                ((eye + dir * t).distance(c) < 1.0 + en.radius).then_some((t, HitZone::Body))
            });
            if let Some((t, z)) = hit {
                if best.is_none_or(|b| t < b.0) {
                    best = Some((t, i, z));
                }
            }
        }
        if let Some((t, i, z)) = best {
            damage_enemy(self, i, 1.0, z, eye + dir * t);
            self.events.push(GameEvent::PunchHit);
            return;
        }
        if let Some(h) = self.world.raycast(eye, dir, 4.0) {
            self.events.push(GameEvent::PunchHit);
            let owner = self.world.owner(h.collider);
            if owner != uk_core::collide::ALWAYS {
                let node = self.def.colliders[owner as usize].node;
                let scs: Vec<u32> = self.def.scripts_on(node).map(|(i, _)| i).collect();
                for sc in scs {
                    if matches!(self.s.scripts[sc as usize], Script::Breakable(_)) {
                        self.breakable_break(sc, 1.0);
                    }
                }
            }
        }
    }
}

#[allow(dead_code)]
fn shape_of(s: &ShapeDef) -> Option<Shape> {
    match s {
        ShapeDef::Box { center, half, rot } => Some(Shape::Box(BoxCollider { center: *center, half: *half, rot: *rot, slippery: false })),
        _ => None,
    }
}
