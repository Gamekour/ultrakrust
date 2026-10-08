//! Enemies and combat. Values come from the scene (health, bite damage) and from
//! the decompiled AI on Standard difficulty; movement is a direct-pursuit
//! approximation of Unity's NavMeshAgent (no navmesh yet).

use crate::game::{Game, GameEvent};
use crate::particles::{look_rotation, to_unity, unity_euler, GoreType, FLIP};
use crate::scripts::Script;
use bevy_math::{Affine3A, Mat4, Quat, Vec3};
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
    /// the collider (index into `SceneDef::colliders`) and its GameObject
    pub collider: u32,
    pub node: u32,
}

/// The GameObject a hit landed on (DamageData.hitTarget).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitTarget {
    pub zone: HitZone,
    pub node: u32,
    pub collider: Option<u32>,
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
    // Malicious Face
    pub burst_charge: f32,
    pub current_burst: u32,
    pub beam_prob: f32,
    pub beam_charge: f32,
    pub beam_fire_t: f32,
    pub beam_target: Vec3,
    pub rng: u32,
    /// NavMeshAgent (None without one)
    pub agent: Option<Agent>,
    /// the agent's velocity
    pub agent_vel: Vec3,
    /// MaliciousFace.ProcessDeath: the corpse falls (spiderFalling) until it lands on a Floor
    pub corpse_falling: bool,
    pub corpse_landed: bool,
    // NavMeshAgent (Zombie TrackTick / Enemy.SetDestination)
    pub nav_dest: Option<Vec3>,
    pub path: Vec<crate::nav::Corner>,
    pub path_i: usize,
    pub track_t: f32,
    /// Mecanim rig of the enemy's Animator (attack timing then comes from clip events)
    pub rig: Option<usize>,
    /// turning toward the target (cleared by the StopTracking clip event)
    pub tracking: bool,
    /// between the DamageStart and DamageEnd clip events
    pub damaging: bool,
    pub was_grounded: bool,
    /// seconds off the ground (step-down flicker is not a fall)
    pub air_t: f32,
    /// Enemy.thickLimbs: blood spawns at the hit collider's closest point to the player
    pub thick_limbs: bool,
    /// the starting health (MaliciousFace.maxHealth)
    pub max_health: f32,
    /// level time of death (GoLimp invokes StopHealing after 1 s, except on Mindflayers)
    pub died_at: Option<f64>,
}

/// NavMeshAgent settings in world units.
#[derive(Clone, Copy, Debug)]
pub struct Agent {
    /// baseOffset x the transform's scale
    pub offset: f32,
    pub speed: f32,
    pub accel: f32,
    /// navmesh query half extents: radius, height (x scale)
    pub half_ext: Vec3,
}

#[derive(Clone, Debug)]
pub struct Projectile {
    pub pos: Vec3,
    pub vel: Vec3,
    pub damage: f32,
    pub friendly: bool,
    pub life: f32,
    /// Projectile.explosionEffect and Projectile.explosive of the prefab it was instantiated from
    pub explosion: Option<(u32, bool)>,
}

/// The explosionEffect (and explosive flag) of the projectile prefab `field` of the `class`
/// script on an enemy's root.
fn projectile_explosion(g: &Game, node: u32, class: &str, field: &str) -> Option<(u32, bool)> {
    let (sc, _) = g.def.scripts_on(node).find(|(_, s)| s.class == class)?;
    let p = g.def.script_prefabs.get(&(sc, format!("{field}>Projectile.explosionEffect"))).copied()?;
    let explosive = g.def.script_nested.get(&(sc, format!("{field}>Projectile"))).is_some_and(|v| v.get("explosive").bool());
    Some((p, explosive))
}

/// Projectile.CreateExplosionEffect (at the transform) or, for an explosive one, Explode (at
/// position - velocity * 0.02), both with the projectile's rotation (it faces its velocity).
fn projectile_explode(g: &mut Game, pos: Vec3, vel: Vec3, explosion: Option<(u32, bool)>) {
    let Some((p, explosive)) = explosion else { return };
    let at = if explosive { pos - vel * 0.02 } else { pos };
    let rot = crate::particles::look_rotation(crate::particles::to_unity(vel));
    g.fx_instantiate(p, crate::particles::to_unity(at), rot);
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
        let thick_limbs = def
            .scripts_on(node)
            .find(|(_, s)| s.class == "Enemy" || s.class == "SpiderBody")
            .is_some_and(|(_, s)| s.data.get("thickLimbs").bool());
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
        for (ci, c) in def.colliders.iter().enumerate() {
            if !nodes.contains(&c.node) {
                continue;
            }
            if c.node == node && !c.trigger {
                match &c.shape {
                    ShapeDef::Capsule { a, b, radius: r } => {
                        radius = *r;
                        half_height = a.distance(*b) * 0.5 + r;
                        center_y = ((*a + *b) * 0.5 - pos0).y;
                    }
                    ShapeDef::Sphere { center, radius: r } => {
                        radius = *r;
                        half_height = *r;
                        center_y = (*center - pos0).y;
                    }
                    _ => {}
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
            hitboxes.push(Hitbox { shape: c.shape.clone(), zone, collider: ci as u32, node: c.node });
        }
        for (_, s) in def.scripts.iter().enumerate().filter(|(_, s)| s.class == "SwingCheck2" && nodes.contains(&s.node)) {
            bite_damage = s.data.get("damage").i64() as i32;
            break;
        }
        let activate_on_death = crate::scripts::nodes(def, eid.get("activateOnDeath"));
        let scale = def.nodes[node as usize].world0.to_scale_rotation_translation().0;
        let agent = def.nav_agents.iter().find(|a| a.node == node).map(|a| Agent {
            offset: a.base_offset * scale.y,
            speed: a.speed,
            accel: a.acceleration,
            half_ext: Vec3::new(a.radius * scale.x, a.height * scale.y, a.radius * scale.x),
        });
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
            burst_charge: 5.0,
            current_burst: 0,
            beam_prob: 0.0,
            beam_charge: -1.0,
            beam_fire_t: -1.0,
            beam_target: Vec3::ZERO,
            rng: node.wrapping_mul(2654435761).max(1),
            agent,
            agent_vel: Vec3::ZERO,
            corpse_falling: false,
            corpse_landed: false,
            nav_dest: None,
            path: Vec::new(),
            path_i: 0,
            track_t: 0.0,
            rig: None,
            tracking: true,
            damaging: false,
            was_grounded: true,
            air_t: 0.0,
            thick_limbs,
            max_health: health,
            died_at: None,
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

    /// Can weapons hit it: alive, or a Malicious Face corpse (its colliders stay).
    pub fn hittable(&self) -> bool {
        self.alive || (self.kind == Kind::MaliciousFace && (self.corpse_falling || self.corpse_landed))
    }

    /// The root GameObject as the hit target (projectiles, the punch's sphere fallback).
    pub fn root_target(&self, def: &SceneDef) -> HitTarget {
        let zone = match def.nodes[self.node as usize].tag {
            tags::HEAD => HitZone::Head,
            tags::LIMB | tags::END_LIMB => HitZone::Limb,
            _ => HitZone::Body,
        };
        HitTarget { zone, node: self.node, collider: self.hitboxes.iter().find(|h| h.node == self.node).map(|h| h.collider) }
    }

    /// Ray against this enemy's hitboxes (world space). Returns (distance, target).
    pub fn raycast(&self, o: Vec3, d: Vec3, max: f32) -> Option<(f32, HitTarget)> {
        self.raycast_n(o, d, max).map(|(t, z, _)| (t, z))
    }

    /// `raycast` with the surface normal at the hit (world space; -d for a point-blank start).
    pub fn raycast_n(&self, o: Vec3, d: Vec3, max: f32) -> Option<(f32, HitTarget, Vec3)> {
        let fwd = self.delta();
        let inv = fwd.inverse();
        let lo = inv.transform_point3(o);
        let ld = inv.transform_vector3(d);
        let mut best: Option<(f32, HitTarget, Vec3)> = None;
        for h in &self.hitboxes {
            // a landed Malicious Face corpse has lost its root SphereCollider
            if !self.alive && self.corpse_landed && h.node == self.node {
                continue;
            }
            let hit: Option<(f32, Vec3)> = match &h.shape {
                ShapeDef::Box { center, half, rot } => {
                    // a ray starting inside still counts (point blank)
                    let b = BoxCollider { center: *center, half: *half, rot: *rot, slippery: false };
                    if b.closest_point(lo).distance_squared(lo) < 1e-6 { Some((0.0, -ld)) } else { b.raycast(lo, ld, max) }
                }
                ShapeDef::Sphere { center, radius } => ray_sphere(lo, ld, *center, *radius, max).map(|t| (t, lo + ld * t - *center)),
                ShapeDef::Capsule { a, b, radius } => ray_capsule(lo, ld, *a, *b, *radius, max).map(|t| {
                    let p = lo + ld * t;
                    let ab = *b - *a;
                    let s = if ab.length_squared() > 0.0 { ((p - *a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
                    (t, p - (*a + ab * s))
                }),
                ShapeDef::Mesh(tris) => tris.iter().filter_map(|t| Triangle::new(t[0], t[1], t[2]).raycast(lo, ld, max)).reduce(|a, b| if b.0 < a.0 { b } else { a }),
            };
            if let Some((t, n)) = hit {
                if best.is_none_or(|b| t < b.0 || (t - b.0 < 0.05 && h.zone == HitZone::Head)) {
                    let n = fwd.transform_vector3(if t == 0.0 { -ld } else { n }).normalize_or_zero();
                    best = Some((t, HitTarget { zone: h.zone, node: h.node, collider: Some(h.collider) }, n));
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
        // EnemyIdentifier.Start: spawnEffect.SetActive(true); spawnIn = false
        en.spawn_in = false;
        let fx = g.def.node_ref(g.def.scripts[en.eid_script as usize].data.get("spawnEffect"));
        if let Some(n) = fx {
            g.set_active(n, true);
        }
    }
}

fn player_target(g: &Game) -> Vec3 {
    g.s.player.pos + Vec3::Y * 0.5
}

/// ZombieMelee.TrackTick + Enemy.SetDestination: re-path to the floor point under the player every
/// 0.2 s (0.5 s while 10+ units of path remain), sampling the goal onto the navmesh within 1 unit and
/// keeping the old destination if the new one is within 0.5 units of it.
fn track_tick(g: &mut Game, dt: f32) {
    let Some(nav) = g.nav.as_ref() else { return };
    // EnemyTarget.GetNavPoint: raycast down from the target onto the environment
    let p = g.s.player.pos;
    let nav_point = g.world.raycast(p + Vec3::Y * 0.1, Vec3::NEG_Y, f32::INFINITY).map_or(p, |h| h.point);
    for en in g.s.enemies.iter_mut() {
        if !en.alive || !g.s.active[en.node as usize] || en.spawn_t > 0.0 || !matches!(en.kind, Kind::Filth | Kind::Stray) {
            continue;
        }
        en.track_t -= dt;
        if en.track_t > 0.0 {
            continue;
        }
        let remaining: f32 = en.path.get(en.path_i..).map_or(0.0, |rest| {
            std::iter::once(en.pos).chain(rest.iter().map(|c| c.pos)).collect::<Vec<_>>().windows(2).map(|w| w[0].distance(w[1])).sum()
        });
        en.track_t = if remaining >= 10.0 { 0.5 } else { 0.2 };
        if !en.grounded || en.attacking {
            continue;
        }
        let dest = nav.sample_position(nav_point, 1.0).unwrap_or(nav_point);
        if en.nav_dest.is_some_and(|d| (dest - d).length_squared() <= 0.25) {
            continue;
        }
        // the agent stands on the navmesh at its feet
        let feet = en.pos + Vec3::Y * (en.center_y - en.half_height.max(en.radius));
        if let Some((path, _)) = nav.find_path(feet, dest) {
            en.path = path;
            en.path_i = 1;
            en.nav_dest = Some(dest);
        }
    }
}

/// Next corner the agent walks toward (NavMeshAgent.steeringTarget), advancing past reached corners.
fn steering_target(en: &mut Enemy) -> Option<Vec3> {
    while let Some(c) = en.path.get(en.path_i) {
        let d = Vec3::new(c.pos.x - en.pos.x, 0.0, c.pos.z - en.pos.z).length();
        if d < 0.5 && en.path_i + 1 < en.path.len() {
            en.path_i += 1;
        } else {
            return Some(c.pos);
        }
    }
    None
}

/// Movement + collisions at the fixed rate.
pub fn fixed_update(g: &mut Game) {
    let dt = FIXED_DT;
    track_tick(g, dt);
    let target = player_target(g);
    let mut out_of_world = Vec::new();
    for i in 0..g.s.enemies.len() {
        let node = g.s.enemies[i].node;
        if !g.s.active[node as usize] {
            continue;
        }
        let en = &mut g.s.enemies[i];
        if en.corpse_falling {
            malicious_face_corpse(g, i, dt);
            continue;
        }
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
        // walk the navmesh path when there is one; straight at the player otherwise
        let has_nav = g.nav.is_some();
        let en = &mut g.s.enemies[i];
        let path_dir = steering_target(en).map(|c| Vec3::new(c.x - en.pos.x, 0.0, c.z - en.pos.z).normalize_or_zero());
        let chase = if has_nav { path_dir.unwrap_or(Vec3::ZERO) } else { dir };
        let desired = match en.kind {
            _ if en.attacking => Vec3::ZERO,
            Kind::Filth if dist > 2.5 => chase * speed,
            // Strays keep their distance: back off inside flee range, close in beyond shoot range
            Kind::Stray if dist < 15.0 => -dir * speed,
            Kind::Stray if dist > 30.0 => chase * speed,
            _ => Vec3::ZERO,
        };
        let h = Vec3::new(en.vel.x, 0.0, en.vel.z);
        let nh = uk_core::umath::vmove_towards(h, desired, accel * dt * 2.0);
        en.vel.x = nh.x;
        en.vel.z = nh.z;
        en.vel.y += GRAVITY * dt;
        en.vel.y = en.vel.y.max(-100.0);
        if dist > 0.1 && en.tracking {
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
        // a walking NavMeshAgent can't leave the navmesh: the feet snap back onto it (knockback
        // and ledges, airborne, are free)
        if let (true, Some(nav), Some(agent)) = (en.grounded, g.nav.as_ref(), en.agent) {
            let feet = en.pos + Vec3::Y * (en.center_y - en.half_height.max(en.radius));
            if let Some((_, on)) = nav.nearest(feet, agent.half_ext) {
                let off = Vec3::new(on.x - feet.x, 0.0, on.z - feet.z);
                if off.length_squared() > 1e-6 {
                    en.pos += off;
                    let out = -off.normalize();
                    let v = en.vel.dot(out);
                    if v > 0.0 {
                        en.vel -= out * v;
                    }
                }
            }
        }
        if en.pos.y < -1000.0 {
            out_of_world.push(i);
        }
    }
    // enemies that fell out of the level still count as dead for their wave
    for i in out_of_world {
        kill_enemy(g, i);
    }
    // DeathZones affect enemies too (AffectedSubjects All / EnemiesOnly)
    let zones: Vec<(u32, u32)> = g
        .triggers
        .iter()
        .copied()
        .filter_map(|ci| {
            let node = g.def.colliders[ci as usize].node;
            if !g.s.active[node as usize] || !g.s.collider_enabled[ci as usize] {
                return None;
            }
            g.def.scripts_on(node).find(|(_, s)| s.class == "DeathZone").map(|(sc, _)| (ci, sc))
        })
        .collect();
    if !zones.is_empty() {
        let mut doomed = Vec::new();
        for (i, en) in g.s.enemies.iter().enumerate() {
            if !en.alive || !g.s.active[en.node as usize] || en.kind == Kind::MaliciousFace {
                continue;
            }
            let c = en.center();
            for &(ci, sc) in &zones {
                let affects_enemies = matches!(&g.s.scripts[sc as usize], Script::DeathZone(dz) if dz.enemy_affected);
                if affects_enemies && g.point_in_trigger(ci, c) {
                    doomed.push(i);
                    break;
                }
            }
        }
        for i in doomed {
            kill_enemy(g, i);
        }
    }
    // projectiles
    let cap = g.s.player.capsule();
    let mut hits = Vec::new();
    let mut booms = Vec::new();
    let mut gibs = Vec::new();
    let world = &g.world;
    g.s.projectiles.retain_mut(|p| {
        p.life -= dt;
        let step = p.vel * dt;
        if let Some(h) = world.raycast(p.pos, step, step.length()) {
            booms.push((h.point, p.vel, p.explosion));
            gibs.push((h.point, p.vel.normalize_or_zero(), h.collider, p.damage, p.friendly));
            return false;
        }
        p.pos += step;
        if !p.friendly {
            let ab = cap.b - cap.a;
            let s = ((p.pos - cap.a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
            if (cap.a + ab * s).distance(p.pos) < cap.radius + 0.5 {
                hits.push(p.damage as i32);
                booms.push((p.pos, p.vel, p.explosion));
                return false;
            }
        }
        p.life > 0.0
    });
    for d in hits {
        g.hurt_player(d, true);
    }
    // Projectile.Collided (Environment layer): if IsStaticEnvironment, CreateEnviroGibs(position -
    // forward, forward, 5, max(2, round(damage / d)), clamp(damage / d, 0.5, 1)), d = 4 for player
    // / friendly bullets, else 10
    for (pos, fwd, c, damage, friendly) in gibs {
        if g.is_static_environment(c) {
            let r = damage / if friendly { 4.0 } else { 10.0 };
            g.create_enviro_gibs(pos - fwd, fwd, 5.0, crate::particles::round_half_even(r).max(2), r.clamp(0.5, 1.0));
        }
    }
    for (pos, vel, ex) in booms {
        projectile_explode(g, pos, vel, ex);
    }
    // friendly (parried) projectiles hit enemies
    let parried: Vec<(usize, Vec3)> = g.s.projectiles.iter().enumerate().filter(|(_, p)| p.friendly).map(|(i, p)| (i, p.pos)).collect();
    let mut remove = Vec::new();
    for (pi, pos) in parried {
        for e in 0..g.s.enemies.len() {
            let en = &g.s.enemies[e];
            if en.alive && g.s.active[en.node as usize] && en.center().distance(pos) < en.radius + 1.0 {
                let target = g.s.enemies[e].root_target(&g.def);
                damage_enemy(g, e, 5.0, target, pos, "projectile");
                remove.push(pi);
                break;
            }
        }
    }
    remove.sort();
    for i in remove.into_iter().rev() {
        let p = g.s.projectiles.remove(i);
        projectile_explode(g, p.pos, p.vel, p.explosion);
    }
}

/// Attacks, timers, deaths (per frame).
fn start_attack(g: &mut Game, i: usize) {
    let en = &mut g.s.enemies[i];
    en.attacking = true;
    en.attack_t = 0.0;
    en.hit_done = false;
    en.damaging = false;
    if let Some(r) = en.rig {
        crate::anim::set_param(g, r, "Swing", 1.0);
    }
}

fn end_attack(en: &mut Enemy) {
    en.attacking = false;
    en.damaging = false;
    en.tracking = true;
    en.cooldown = match en.kind {
        Kind::Filth => 0.5,
        _ => 1.0 + pseudo_rand(en.node as f32 + en.attack_t) * 1.5,
    };
}

fn stray_throw(g: &mut Game, i: usize, target: Vec3) {
    let explosion = projectile_explosion(g, g.s.enemies[i].node, "ZombieProjectiles", "projectile");
    let en = &mut g.s.enemies[i];
    en.hit_done = true;
    let from = en.center() + Vec3::Y * 1.0;
    let dir = (target + Vec3::Y * 1.0 - from).normalize_or_zero();
    g.s.projectiles.push(Projectile { pos: from + dir, vel: dir * 65.0, damage: 25.0, friendly: false, life: 10.0, explosion });
}

/// Each enemy's Animator rig: the first Animator on the enemy root or below it.
pub fn bind_rigs(g: &mut Game) {
    for i in 0..g.s.enemies.len() {
        let node = g.s.enemies[i].node;
        g.s.enemies[i].rig = (0..g.def.animators.len() as u32).filter(|&a| g.def.is_descendant(g.def.animators[a as usize].node, node)).find_map(|a| g.anim.rig_of_animator(a));
    }
}

/// The Animator feed of Zombie.Update: Running / RunSpeed from the agent's velocity, Falling
/// while airborne (StartFalling on leaving the ground).
fn drive_animator(g: &mut Game, i: usize, dt: f32) {
    let en = &mut g.s.enemies[i];
    let Some(r) = en.rig else { return };
    let max = match en.kind {
        Kind::Filth => 20.0,
        Kind::Stray => 10.0,
        _ => return,
    };
    let h = Vec3::new(en.vel.x, 0.0, en.vel.z).length();
    // the NavMeshAgent pins a walking zombie to the mesh: only a real airborne spell (knockback,
    // ledge) reads as falling
    en.air_t = if en.grounded { 0.0 } else { en.air_t + dt };
    let (grounded, was) = (en.air_t < 0.15, en.was_grounded);
    en.was_grounded = grounded;
    crate::anim::set_param(g, r, "Running", (h > 0.1) as i32 as f32);
    crate::anim::set_param(g, r, "RunSpeed", (h / max).min(1.0));
    crate::anim::set_param(g, r, "Falling", (!grounded) as i32 as f32);
    if was && !grounded {
        crate::anim::set_param(g, r, "StartFalling", 1.0);
    }
}

/// Clip events fired this frame (`g.events[from..]`) routed to the enemy owning the Animator:
/// the attack's tracking / damage window / projectile / end come from the animation, as in
/// ZombieMelee and ZombieProjectiles.
pub fn anim_events(g: &mut Game, from: usize) {
    let mut hits = Vec::new();
    for e in &g.events[from.min(g.events.len())..] {
        let GameEvent::AnimEvent { node, function, .. } = e else { continue };
        let Some(i) = g.s.enemies.iter().position(|en| en.alive && en.rig.is_some_and(|r| g.def.animators[g.anim.rig_info(r).0 as usize].node == *node)) else { continue };
        hits.push((i, function.clone()));
    }
    if hits.is_empty() {
        return;
    }
    let target = player_target(g);
    for (i, f) in hits {
        if f == "PullOut" {
            // ZombieMelee.PullOut: Instantiate(pullOutParticle, position, identity)
            let node = g.s.enemies[i].node;
            let zm = g.def.scripts_on(node).find(|(_, s)| s.class == "ZombieMelee").map(|(k, _)| k);
            if let Some(p) = zm.and_then(|k| g.script_prefab(k, "pullOutParticle")) {
                let at = g.node_world_now(node).w_axis.truncate();
                g.fx_instantiate(p, at, Quat::IDENTITY);
            }
        }
        let en = &mut g.s.enemies[i];
        if !en.attacking {
            continue;
        }
        match f.as_str() {
            "StopTracking" => en.tracking = false,
            "DamageStart" => en.damaging = true,
            "DamageEnd" => en.damaging = false,
            "ThrowProjectile" if !en.hit_done => stray_throw(g, i, target),
            "SwingEnd" => end_attack(en),
            _ => {}
        }
    }
}

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
                    start_attack(g, i);
                }
                let en = &mut g.s.enemies[i];
                if en.attacking {
                    en.attack_t += dt;
                    // bite: damage window during the lunge (DamageStart..DamageEnd when animated)
                    let window = if en.rig.is_some() { en.damaging } else { en.attack_t > 0.45 && en.attack_t < 0.65 };
                    if !en.hit_done && window {
                        let fwd = Vec3::new(en.yaw.sin(), 0.0, -en.yaw.cos());
                        let to = player_pos - en.pos;
                        if to.length() < 3.5 && fwd.dot(Vec3::new(to.x, 0.0, to.z).normalize_or_zero()) > 0.3 {
                            en.hit_done = true;
                            let d = en.bite_damage;
                            g.hurt_player(d, true);
                        }
                    }
                    let en = &mut g.s.enemies[i];
                    // animated: SwingEnd ends it (timeout only guards a rig stuck elsewhere)
                    if en.attack_t > if en.rig.is_some() { 4.0 } else { 1.0 } {
                        end_attack(en);
                    }
                }
            }
            Kind::Stray => {
                if en.cooldown > 0.0 {
                    en.cooldown = move_towards(en.cooldown, 0.0, dt);
                }
                let in_range = dist < if en.cooldown <= 0.0 { 60.0 } else { 30.0 };
                if !en.attacking && en.cooldown <= 0.0 && in_range {
                    start_attack(g, i);
                }
                let en = &mut g.s.enemies[i];
                if en.attacking {
                    en.attack_t += dt;
                    // animated: the ThrowProjectile clip event throws
                    if en.rig.is_none() && !en.hit_done && en.attack_t > 0.55 {
                        stray_throw(g, i, target);
                    }
                    let en = &mut g.s.enemies[i];
                    if en.attack_t > if en.rig.is_some() { 4.0 } else { 0.9 } {
                        end_attack(en);
                    }
                }
            }
            Kind::MaliciousFace => {
                // Update: MovementUpdate runs while neither charging nor holding a beam charge
                if g.s.enemies[i].beam_charge < 0.0 {
                    malicious_face_movement(g, i, dt);
                }
                malicious_face(g, i, dt, target);
            }
            Kind::Other => {}
        }
        drive_animator(g, i, dt);
    }
}

fn next_rand(state: &mut u32) -> f32 {
    // xorshift32
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 17;
    x ^= x << 5;
    *state = x;
    (x as f32) / (u32::MAX as f32)
}

/// MaliciousFace (Standard difficulty): projectile bursts of 6 in a cross pattern,
/// and a 2 s charged beam aimed at where the player is heading.
fn malicious_face(g: &mut Game, i: usize, dt: f32, target: Vec3) {
    let player_vel = g.s.player.vel;
    let player_pos = g.s.player.pos;
    let en = &mut g.s.enemies[i];
    let mouth = en.center();
    let to = target - mouth;
    let dist = to.length();
    let dir = to.normalize_or_zero();
    en.yaw = dir.x.atan2(-dir.z);
    // beam: charging -> locked target -> fire
    if en.beam_charge >= 0.0 {
        en.beam_charge = (en.beam_charge + 0.5 * dt).min(1.0);
        if en.beam_charge >= 1.0 && en.beam_fire_t < 0.0 {
            en.beam_target = player_pos + Vec3::Y * 1.67 + Vec3::new(player_vel.x, player_vel.y / 2.0, player_vel.z) / 2.0;
            en.beam_fire_t = 0.5;
        }
        if en.beam_fire_t >= 0.0 {
            en.beam_fire_t -= dt;
            if en.beam_fire_t < 0.0 {
                let bdir = (en.beam_target - mouth).normalize_or_zero();
                en.beam_charge = -1.0;
                en.burst_charge = 1.0;
                let world_hit = g.world.raycast(mouth + bdir * 2.5, bdir, 400.0).map(|h| h.distance).unwrap_or(400.0);
                let cap = g.s.player.capsule();
                let rel = |p: Vec3| p - mouth;
                let along = rel((cap.a + cap.b) * 0.5).dot(bdir);
                let closest = mouth + bdir * along.clamp(0.0, world_hit);
                let ab = cap.b - cap.a;
                let t = ((closest - cap.a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0);
                let d = (cap.a + ab * t).distance(closest);
                g.events.push(GameEvent::Shot { from: mouth, to: mouth + bdir * world_hit, pierce: true });
                if d < cap.radius + 1.5 && along > 0.0 && along < world_hit + 1.0 {
                    // ContinuousBeam: GetHurt(..., ignoreInvincibility: true)
                    g.hurt_player_ignoring_invincibility(50);
                }
            }
        }
        return;
    }
    if en.burst_charge > 0.0 {
        en.burst_charge = move_towards(en.burst_charge, 0.0, dt);
    }
    if en.current_burst > 5 && en.burst_charge == 0.0 {
        en.current_burst = 0;
        en.burst_charge = 1.0;
    }
    if en.burst_charge > 0.0 || dist > 150.0 {
        return;
    }
    // AttackCheck
    let shoot = if en.current_burst != 0 {
        true
    } else {
        let r = next_rand(&mut en.rng) * en.health * 0.4;
        let beam = (en.beam_prob > 5.0 || r < en.beam_prob) && dist <= 50.0;
        if beam {
            en.beam_charge = 0.0;
            // BeamChargeUpdate: speed 0 and isStopped; BeamFire disables the agent
            en.agent_vel = Vec3::ZERO;
            en.beam_fire_t = -1.0;
            en.beam_prob = if en.health > 10.0 { 0.0 } else { 1.0 };
            false
        } else {
            en.beam_prob += 1.0;
            true
        }
    };
    if shoot {
        // Standard difficulty: every projectile of the burst aims at the head (the
        // cross-shaped spread is difficulty >= 4 only).
        let aim = target + Vec3::Y * 1.0;
        let pdir = (aim - mouth).normalize_or_zero();
        en.current_burst += 1;
        en.burst_charge = 0.1;
        let node = en.node;
        let explosion = projectile_explosion(g, node, "MaliciousFace", "proj");
        g.s.projectiles.push(Projectile { pos: mouth + pdir * 3.0, vel: pdir * 65.0, damage: 25.0, friendly: false, life: 10.0, explosion });
    }
}

/// MaliciousFace.MovementUpdate: the NavMeshAgent heads for the player when a complete path to
/// them exists and stops otherwise. The transform rides `baseOffset` above the agent's navmesh
/// position (spiderTargetHeight stays at its default without a buff targeter, so the offset does
/// not change). Steering: velocity moves toward the next corner at `speed` by `acceleration`,
/// braking into the destination (autoBraking).
fn malicious_face_movement(g: &mut Game, i: usize, dt: f32) {
    let Some(nav) = g.nav.as_ref() else { return };
    let Some(agent) = g.s.enemies[i].agent else { return };
    let player = g.s.player.pos;
    let en = &mut g.s.enemies[i];
    // the agent's position on the navmesh (isOnNavMesh)
    let Some((_, on)) = nav.nearest(en.pos - Vec3::Y * agent.offset, agent.half_ext) else { return };
    // CalculatePath(target) must be PathComplete; otherwise SetDestination(transform.position)
    let path = nav.nearest(player, agent.half_ext).and_then(|(_, goal)| nav.find_path(on, goal)).filter(|(_, complete)| *complete);
    let (steer, remaining) = match &path {
        Some((corners, _)) if corners.len() > 1 => {
            let pts: Vec<Vec3> = std::iter::once(on).chain(corners[1..].iter().map(|c| c.pos)).collect();
            (corners[1].pos - on, pts.windows(2).map(|w| w[0].distance(w[1])).sum::<f32>())
        }
        _ => (Vec3::ZERO, 0.0),
    };
    let desired = steer.with_y(0.0).normalize_or_zero() * agent.speed.min((2.0 * agent.accel * remaining).sqrt());
    en.agent_vel = uk_core::umath::vmove_towards(en.agent_vel, desired, agent.accel * dt);
    // stay on the navmesh: the moved point snaps back onto it
    let to = on + en.agent_vel * dt;
    let new_on = nav.nearest(to, Vec3::new(1.0, agent.half_ext.y, 1.0)).map_or(on, |(_, p)| p);
    if (new_on - to).with_y(0.0).length() > 0.01 {
        en.agent_vel = Vec3::ZERO;
    }
    en.pos = new_on + Vec3::Y * agent.offset;
}

/// MaliciousFace corpse (ProcessDeath / HandleCollision): the Rigidbody turns dynamic with gravity
/// and falls (excludeLayers: Default) until its sphere touches a "Floor"-tagged collider; there it
/// turns kinematic, drops 1.5 along its up axis and loses its SphereCollider and SpiderBodyTrigger.
/// The head's mesh collider (layer 11) stays as a solid corpse.
fn malicious_face_corpse(g: &mut Game, i: usize, dt: f32) {
    let def = g.def.clone();
    let solid = |o: u32| o == uk_core::collide::ALWAYS || def.colliders.get(o as usize).is_some_and(|c| c.layer != 0 && !c.trigger);
    let en = &mut g.s.enemies[i];
    en.vel.y += GRAVITY * dt;
    en.pos += en.vel * dt;
    let mut cap = Capsule { a: en.center(), b: en.center(), radius: en.radius };
    let before = cap.a;
    let mut normals = Vec::new();
    g.world.depenetrate_capsule_filtered(&mut cap, &mut normals, solid);
    let en = &mut g.s.enemies[i];
    en.pos += cap.a - before;
    for n in &normals {
        let into = en.vel.dot(*n);
        if into < 0.0 {
            en.vel -= *n * into;
        }
    }
    if normals.is_empty() {
        return;
    }
    let (c, r) = (en.center(), en.radius);
    let floor = g.world.overlap_sphere(c, r + 0.05).into_iter().map(|id| g.world.owner(id)).any(|o| {
        solid(o) && o != uk_core::collide::ALWAYS && def.nodes[def.colliders[o as usize].node as usize].tag == tags::FLOOR
    });
    if floor {
        // OnCollisionEnter with a Floor: Instantiate(impactParticle, transform.position, transform.rotation)
        let node = g.s.enemies[i].node;
        let mf = g.def.scripts_on(node).find(|(_, s)| s.class == "MaliciousFace").map(|(sc, _)| sc);
        if let Some(p) = mf.and_then(|sc| g.script_prefab(sc, "impactParticle")) {
            let (_, rot, pos) = g.node_world_now(node).to_scale_rotation_translation();
            g.fx_instantiate(p, pos, rot);
        }
        let en = &mut g.s.enemies[i];
        en.corpse_falling = false;
        en.corpse_landed = true;
        en.vel = Vec3::ZERO;
        en.pos -= Vec3::Y * 1.5;
    }
}

fn pseudo_rand(x: f32) -> f32 {
    ((x * 12.9898).sin() * 43758.547).fract().abs()
}


/// Enemy.GetHurt (Standard difficulty, critMultiplier 1) with the blood it spawns
/// (HandleBloodSelection, ProcessBloodEffects) and the Malicious Face's OnDamage.
/// `hit_pos` is the world-space (Bevy) hit point, `hitter` DamageData.hitter.
pub fn damage_enemy(g: &mut Game, e: usize, base: f32, target: HitTarget, hit_pos: Vec3, hitter: &str) {
    let en = &g.s.enemies[e];
    if !en.hittable() {
        return;
    }
    let mut m = base;
    // zombies take x1.5 in the air
    if matches!(en.kind, Kind::Filth | Kind::Stray) && !en.grounded && hitter != "fire" {
        m *= 1.5;
    }
    // knockback
    let push = (en.center() - g.s.player.pos).normalize_or_zero();
    if en.alive {
        g.s.enemies[e].vel += Vec3::new(push.x, 0.3, push.z) * 4.0 * base;
    }
    // MaliciousFace.OnDamage -> HandleSpiderDamage (cancels the hit on a corpse)
    if g.s.enemies[e].kind == Kind::MaliciousFace && malicious_face_on_damage(g, e, target, m, hitter) {
        return;
    }
    let limb = match target.zone {
        HitZone::Head => 1.0,
        HitZone::Limb => 0.5,
        HitZone::Body => 0.0,
    };
    let crit = 1.0;
    let dmg = m + m * limb * crit;
    let en = &mut g.s.enemies[e];
    en.health -= dmg;
    let dead = en.health <= 0.0;
    let gore = blood_selection(g, e, target, dmg, hitter);
    g.events.push(GameEvent::EnemyHit { pos: hit_pos, damage: dmg, head: target.zone == HitZone::Head });
    if dead {
        kill_enemy(g, e);
    }
    if let Some(gi) = gore {
        process_blood_effects(g, e, gi, target, hitter);
    }
}

/// Enemy.HandleBloodSelection (`health` is after the subtraction). eid.underwater, sandified and
/// blessed are not tracked for enemies, so the gore is always the plain kind.
fn blood_selection(g: &mut Game, e: usize, target: HitTarget, damage: f32, hitter: &str) -> Option<usize> {
    if hitter == "fire" || damage <= 0.0 {
        return None;
    }
    let tag = g.def.nodes[target.node as usize].tag;
    let flag = damage >= 1.0 || g.s.enemies[e].health <= 0.0;
    let explosion = hitter == "explosion";
    let got = if (tag == tags::HEAD && flag) || hitter == "hammer" || hitter == "heavypunch" {
        GoreType::Head
    } else if (explosion && tag == tags::END_LIMB) || (flag && !explosion) {
        if tag == tags::BODY { GoreType::Body } else { GoreType::Limb }
    } else if !explosion {
        GoreType::Small
    } else {
        return None;
    };
    g.get_gore(got, false, false)
}

/// `target.transform.position` (Unity space).
fn target_position(g: &Game, target: HitTarget) -> Vec3 {
    g.node_world_now(target.node).w_axis.truncate()
}

/// Enemy.noheal: StopHealing runs 1 s after GoLimp (every enemy here; Mindflayers are exempt).
fn noheal(g: &Game, e: usize) -> bool {
    g.s.enemies[e].died_at.is_some_and(|t| g.s.time >= t + 1.0)
}

/// Enemy.ProcessBloodEffects.
fn process_blood_effects(g: &mut Game, e: usize, gi: usize, target: HitTarget, hitter: &str) {
    let en = &g.s.enemies[e];
    // IsMachine() || IsSpider() with thickLimbs: the closest point of the hit collider to the
    // player (no machines are ported; the Malicious Face is a spider)
    let mut at = None;
    if en.kind == Kind::MaliciousFace && en.thick_limbs {
        if let Some(ci) = target.collider {
            let c = &g.def.colliders[ci as usize];
            // the collider's current pose relative to its load pose (Bevy space)
            let xf = FLIP * g.node_world_now(c.node) * FLIP * g.def.nodes[c.node as usize].world0.inverse();
            let p = xf.inverse().transform_point3(g.s.player.pos);
            at = Some(to_unity(xf.transform_point3(crate::game::shape_closest_point(&c.shape, p))));
        }
    }
    let at = at.unwrap_or_else(|| target_position(g, target));
    let noheal = noheal(g, e);
    let fx = &mut g.fx.effects[gi];
    fx.pos = at;
    if hitter == "drill" {
        fx.scale *= 2.0;
    }
    let Some(sp) = fx.splatter.as_mut() else { return };
    if hitter == "shotgun" || hitter == "shotgunzone" || hitter == "explosion" {
        // (its 50% particle-collision toggle changes nothing: the collision module is not simulated)
        sp.hp = 3;
    } else if hitter == "nail" {
        sp.hp = 1;
    }
    if !noheal {
        sp.ready = true;
    }
}

/// MaliciousFace.HandleSpiderDamage. Returns true when the hit is cancelled (a corpse).
fn malicious_face_on_damage(g: &mut Game, e: usize, target: HitTarget, damage: f32, hitter: &str) -> bool {
    let (node, health, max_health, alive) = {
        let en = &g.s.enemies[e];
        (en.node, en.health, en.max_health, en.alive)
    };
    let at = target_position(g, target);
    // Instantiate(bsm.GetGore(Small)): the pooled original activates (and plays) where it was
    // left; the copy goes to the hit target
    let small = |g: &mut Game| -> Option<usize> {
        let orig = g.get_gore(GoreType::Small, false, false)?;
        let c = g.fx_clone(orig);
        let fx = &mut g.fx.effects[c];
        fx.pos = at;
        if hitter == "drill" {
            fx.scale *= 2.0;
        }
        Some(c)
    };
    if !alive {
        if hitter != "fire" {
            if let Some(c) = small(g) {
                if let Some(sp) = g.fx.effects[c].splatter.as_mut() {
                    if damage >= 1.0 {
                        sp.hp = 30;
                    }
                    if health > 0.0 {
                        sp.ready = true;
                    }
                }
            }
        }
        return true;
    }
    let script = g.def.scripts_on(node).find(|(_, s)| s.class == "MaliciousFace").map(|(i, _)| i);
    let prefab = |g: &Game, field: &str| script.and_then(|s| g.def.script_prefabs.get(&(s, field.to_string())).copied());
    if hitter != "fire" {
        if let Some(c) = small(g) {
            if let Some(sp) = g.fx.effects[c].splatter.as_mut() {
                if health > 0.0 {
                    sp.ready = true;
                }
                if hitter == "nail" {
                    sp.hp = 3;
                } else if damage >= 1.0 {
                    sp.hp = 30;
                }
            }
            if g.fx.gore_on {
                g.fx_play(c);
            }
        }
        if hitter != "shotgun" && hitter != "drill" && g.active(node) {
            if let Some(p) = prefab(g, "dripBlood") {
                // Instantiate at the hit target, parent to the face, LookAt(face), Rotate(180, 180, 180)
                let face = g.node_world_now(node);
                let rot = look_rotation(face.w_axis.truncate() - at) * unity_euler(Vec3::splat(180.0));
                let d = g.fx_instantiate(p, at, rot);
                let scale = g.fx.effects[d].scale;
                g.fx.effects[d].attach = Some((node, face.inverse() * Mat4::from_scale_rotation_translation(scale, rot, at)));
                if g.fx.gore_on {
                    g.fx_play(d);
                }
            }
        }
    }
    if health >= max_health / 2.0 && health - damage < max_health / 2.0 {
        if let Some(p) = prefab(g, "woundedParticle") {
            let pos = g.node_world_now(node).w_axis.truncate();
            g.fx_instantiate(p, pos, Quat::IDENTITY);
        }
    }
    false
}

pub fn kill_enemy(g: &mut Game, e: usize) {
    let en = &mut g.s.enemies[e];
    if !en.alive {
        return;
    }
    en.alive = false;
    en.attacking = false;
    en.died_at = Some(g.s.time);
    let (node, aod, pos) = (en.node, en.activate_on_death.clone(), en.center());
    g.s.kills += 1;
    g.events.push(GameEvent::EnemyKilled { pos });
    for n in aod {
        g.set_active(n, true);
    }
    g.add_dead_enemy(node);
    let en = &mut g.s.enemies[e];
    if en.kind == Kind::MaliciousFace {
        // ProcessDeath: the corpse falls and stays (BreakCorpse needs a ground slam, breaker or
        // cannonball hitter, none of which is ported)
        en.corpse_falling = true;
        en.vel = Vec3::ZERO;
        en.agent_vel = Vec3::ZERO;
        return;
    }
    // corpse disappears (gibs/ragdoll not ported)
    g.set_active(node, false);
}

// ---------------------------------------------------------------- player weapons

impl Game {
    /// Revolver hitscan. Normal beams stop at the first body; piercing beams pass through enemies.
    pub fn fire_revolver(&mut self, eye: Vec3, dir: Vec3, pierce: bool) {
        // Revolver.Shoot: RandomChance picks Shoot / Shoot2; the charged beam plays Shoot3
        if let Some(r) = self.vm_rigs.0 {
            let roll = next_rand(&mut self.s.vm_rng);
            crate::anim::set_param(self, r, "RandomChance", roll);
            crate::anim::set_param(self, r, if pierce { "ChargeShoot" } else { "Shoot" }, 1.0);
        }
        let dir = dir.normalize_or_zero();
        let max = 1000.0;
        let env = self.world.raycast(eye, dir, max);
        let env_t = env.map(|h| h.distance).unwrap_or(max);
        let mut hits: Vec<(f32, usize, HitTarget, Vec3)> = Vec::new();
        for (i, en) in self.s.enemies.iter().enumerate() {
            if !en.hittable() || !self.s.active[en.node as usize] || en.spawn_t > 0.0 {
                continue;
            }
            if let Some((t, z, n)) = en.raycast_n(eye, dir, env_t) {
                hits.push((t, i, z, n));
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let damage = if pierce { 2.0 } else { 1.0 };
        let mut end = eye + dir * env_t;
        // RevolverBeam.hitParticle of the beam prefab this shot instantiates
        let hit_fx = self.class_prefab("Revolver", if pierce { "revolverBeamSuper>RevolverBeam.hitParticle" } else { "revolverBeam>RevolverBeam.hitParticle" });
        let beam_rot = crate::particles::look_rotation(crate::particles::to_unity(dir));
        if pierce {
            for (t, i, z, _) in hits {
                damage_enemy(self, i, damage, z, eye + dir * t, "revolver");
                // PiercingShotCheck, an enemy body: at the hit point with the beam's rotation
                if let Some(p) = hit_fx {
                    self.fx_instantiate(p, crate::particles::to_unity(eye + dir * t), beam_rot);
                }
            }
            let glass = env.is_some_and(|h| self.collider_has_glass(h.collider));
            self.hit_environment(env, damage);
            if !glass {
                self.beam_enviro_gibs(env, damage);
            }
            // PiercingShotCheck, anything else but glass: LookRotation(hit normal)
            if let (Some(p), Some(h)) = (hit_fx, env.filter(|_| !glass)) {
                self.fx_instantiate(p, crate::particles::to_unity(h.point), crate::particles::look_rotation(crate::particles::to_unity(h.normal)));
            }
        } else if let Some(&(t, i, z, n)) = hits.first() {
            end = eye + dir * t;
            damage_enemy(self, i, damage, z, end, "revolver");
            // HitSomething: at the hit point with the beam's rotation, then forward = hit normal
            if let Some(p) = hit_fx {
                self.fx_instantiate(p, crate::particles::to_unity(end), crate::particles::set_forward(beam_rot, crate::particles::to_unity(n)));
            }
        } else {
            self.hit_environment(env, damage);
            self.beam_enviro_gibs(env, damage);
            if let (Some(p), Some(h)) = (hit_fx, env) {
                self.fx_instantiate(p, crate::particles::to_unity(h.point), crate::particles::set_forward(beam_rot, crate::particles::to_unity(h.normal)));
            }
        }
        self.events.push(GameEvent::Shot { from: eye, to: end, pierce });
    }

    /// RevolverBeam.ExecuteHits on the environment: CreateEnviroGibs(hit, round(3 * damage), damage)
    /// when it is static environment.
    fn beam_enviro_gibs(&mut self, env: Option<uk_core::collide::RayHit>, damage: f32) {
        if let Some(h) = env.filter(|h| self.is_static_environment(h.collider)) {
            self.create_enviro_gibs(h.point + h.normal, -h.normal, 5.0, crate::particles::round_half_even(3.0 * damage), damage);
        }
    }

    /// Is a Glass script on this world collider's GameObject?
    fn collider_has_glass(&self, c: uk_core::collide::ColliderId) -> bool {
        let owner = self.world.owner(c);
        if owner == uk_core::collide::ALWAYS {
            return false;
        }
        let node = self.def.colliders[owner as usize].node;
        self.def.scripts_on(node).any(|(sc, _)| matches!(self.s.scripts[sc as usize], Script::Glass(_)))
    }

    /// Does this ray hit a Glass object first?
    pub fn ray_hits_glass(&self, eye: Vec3, dir: Vec3, max: f32) -> bool {
        let Some(h) = self.world.raycast(eye, dir, max) else { return false };
        let owner = self.world.owner(h.collider);
        if owner == uk_core::collide::ALWAYS {
            return false;
        }
        let node = self.def.colliders[owner as usize].node;
        self.def.scripts_on(node).any(|(sc, _)| matches!(self.s.scripts[sc as usize], Script::Glass(_)))
    }

    /// Would a revolver shot along this ray hit an enemy before the environment?
    pub fn aim_hits_enemy(&self, eye: Vec3, dir: Vec3) -> bool {
        let dir = dir.normalize_or_zero();
        let env_t = self.world.raycast(eye, dir, 1000.0).map(|h| h.distance).unwrap_or(1000.0);
        self.s.enemies.iter().any(|en| {
            en.alive && self.s.active[en.node as usize] && en.spawn_t <= 0.0 && en.raycast(eye, dir, env_t).is_some()
        })
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
        // Punch.PunchStart: PunchRandomizer picks Jab / Jab2
        if let Some(r) = self.vm_rigs.1 {
            let roll = next_rand(&mut self.s.vm_rng);
            crate::anim::set_param(self, r, "PunchRandomizer", roll);
            crate::anim::set_param(self, r, "Punch", 1.0);
        }
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
        let mut best: Option<(f32, usize, HitTarget)> = None;
        for (i, en) in self.s.enemies.iter().enumerate() {
            if !en.hittable() || !self.s.active[en.node as usize] || en.spawn_t > 0.0 {
                continue;
            }
            let hit = en.raycast(eye, dir, 4.0).or_else(|| {
                let c = en.center();
                let t = (c - eye).dot(dir).clamp(0.0, 4.0);
                ((eye + dir * t).distance(c) < 1.0 + en.radius).then_some((t, en.root_target(&self.def)))
            });
            if let Some((t, z)) = hit {
                if best.is_none_or(|b| t < b.0) {
                    best = Some((t, i, z));
                }
            }
        }
        if let Some((t, i, z)) = best {
            damage_enemy(self, i, 1.0, z, eye + dir * t, "punch");
            self.events.push(GameEvent::PunchHit);
            return;
        }
        if let Some(h) = self.world.raycast(eye, dir, 4.0) {
            self.events.push(GameEvent::PunchHit);
            // Instantiate(dustParticle, hit.point, transform.rotation).transform.forward = hit.normal
            if let Some(p) = self.class_prefab("Punch", "dustParticle") {
                let rot = crate::particles::look_rotation(crate::particles::to_unity(dir));
                self.fx_instantiate(p, crate::particles::to_unity(h.point), crate::particles::set_forward(rot, crate::particles::to_unity(h.normal)));
            }
            let owner = self.world.owner(h.collider);
            if owner != uk_core::collide::ALWAYS {
                let node = self.def.colliders[owner as usize].node;
                let scs: Vec<u32> = self.def.scripts_on(node).map(|(i, _)| i).collect();
                for sc in scs {
                    if matches!(self.s.scripts[sc as usize], Script::Breakable(_)) {
                        self.breakable_break(sc, 1.0);
                    } else if matches!(self.s.scripts[sc as usize], Script::Glass(_)) {
                        self.glass_shatter(sc);
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
