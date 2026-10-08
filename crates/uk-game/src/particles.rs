//! Shuriken particle systems (UnityEngine.ParticleSystem) for the effect prefabs the game spawns at
//! runtime, and the scripts on them: BloodsplatterManager (pools, GetGore), Bloodsplatter (play on
//! enable, heal trigger, repool when stopped), BloodUnderwaterChecker, RemoveOnTime and
//! PortalAwareParticleSystem (particles die where they cross the environment).
//!
//! Everything here is in Unity space (left-handed, as `anim::world_of`): effect roots, prefab
//! nodes and particles. Systems simulate in their node's space (every effect prefab uses local
//! simulation space) with the scaling mode applied; world space is `sim_to_world * p`.

use crate::game::Game;
use bevy_math::{Mat3, Mat4, Quat, Vec3};
use std::collections::VecDeque;
use std::sync::Arc;
use uk_assets::particles::{ParticlePrefab, ParticleSystemDef};
use uk_core::collide::Capsule;
use uk_core::consts::GRAVITY;

/// GoreType (the enum the game passes to BloodsplatterManager.GetGore).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoreType {
    Head,
    Limb,
    Body,
    Small,
    Splatter,
    Smallest,
}

/// BSType of the pooled prefabs, as indices into `POOLS` (BloodsplatterManager field names).
pub const POOLS: [&str; 8] = ["head", "limb", "body", "small", "smallest", "splatter", "underwater", "sand"];
const BS_UNDERWATER: usize = 6;
/// BSType.dontpool / unknown: Repool destroys the object
const BS_DONTPOOL: i64 = 15;

/// PortalAwareParticleSystem's environment mask: Environment, Outdoors, OutdoorsBaked, EnvironmentBaked.
const STAIN_LAYERS: [u8; 4] = [8, 24, 6, 7];

pub const FLIP: Mat4 = Mat4::from_cols_array(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);

/// Bevy <-> Unity (the z flip is its own inverse).
pub fn to_unity(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.y, -p.z)
}

pub fn to_bevy(p: Vec3) -> Vec3 {
    to_unity(p)
}

#[derive(Clone, Debug)]
pub struct TrailState {
    /// (position in simulation space, particle age when recorded), oldest first
    pub points: Vec<(Vec3, f32)>,
    /// seconds each trail vertex lives
    pub vertex_life: f32,
}

#[derive(Clone, Debug)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    pub lifetime: f32,
    pub start_size: f32,
    pub start_color: [f32; 4],
    pub rot: f32,
    pub gravity: f32,
    /// per-particle random blends for the over-lifetime curves (color, size, velocity, rotation)
    pub rand: [f32; 4],
    pub trail: Option<TrailState>,
    /// world position after the last update (PortalAwareParticleSystem's previous position)
    pub prev_world: Vec3,
    /// hit the environment: removed on the next update
    pub kill: bool,
}

impl Particle {
    pub fn color(&self, def: &ParticleSystemDef) -> [f32; 4] {
        let t = self.age / self.lifetime;
        match &def.color_over_lifetime {
            Some(g) => mul4(self.start_color, g.eval(t, self.rand[0])),
            None => self.start_color,
        }
    }

    pub fn size(&self, def: &ParticleSystemDef) -> f32 {
        match &def.size_over_lifetime {
            Some(c) => self.start_size * c.eval(self.age / self.lifetime, self.rand[1]),
            None => self.start_size,
        }
    }
}

pub fn mul4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

/// One ParticleSystem instance (on prefab node `node`).
#[derive(Clone, Debug)]
pub struct System {
    pub node: u32,
    pub def: Arc<ParticleSystemDef>,
    /// system time since Play (after the start delay)
    pub time: f32,
    pub delay: f32,
    pub playing: bool,
    pub particles: Vec<Particle>,
    rate_acc: f32,
    burst_next: Vec<u32>,
    /// simulation space -> world (Unity), refreshed every update
    pub sim_to_world: Mat4,
}

impl System {
    fn new(node: u32, def: Arc<ParticleSystemDef>) -> Self {
        let n = def.bursts.len();
        Self { node, def, time: 0.0, delay: 0.0, playing: false, particles: Vec::new(), rate_acc: 0.0, burst_next: vec![0; n], sim_to_world: Mat4::IDENTITY }
    }

    fn play(&mut self, rng: &mut Rng) {
        if self.playing {
            return;
        }
        self.playing = true;
        self.time = 0.0;
        self.rate_acc = 0.0;
        self.burst_next.iter_mut().for_each(|b| *b = 0);
        self.delay = self.def.start_delay.eval(0.0, rng.f());
    }

    fn clear(&mut self) {
        self.particles.clear();
    }

    /// Still emitting (ParticleSystem.isEmitting).
    pub fn emitting(&self) -> bool {
        self.playing && (self.def.looping || self.time < self.def.duration)
    }

    /// ParticleSystem.isStopped (for this system alone).
    pub fn stopped(&self) -> bool {
        !self.emitting() && self.particles.is_empty()
    }
}

/// Bloodsplatter state.
#[derive(Clone, Debug)]
pub struct Splatter {
    /// BSType (index into POOLS, or BS_DONTPOOL and up)
    pub bs_type: i64,
    pub hp: i32,
    pub ready: bool,
    pub can_collide: bool,
    pub been_played: bool,
    pub from_explosion: bool,
    pub underwater: bool,
    /// SphereCollider.enabled (on the root)
    pub col_enabled: bool,
    /// Invoke("DisableCollider") due time
    pub disable_at: Option<f64>,
    /// the player is inside the trigger (OnTriggerEnter fires on the transition)
    pub inside: bool,
}

/// An instantiated effect prefab.
#[derive(Clone, Debug)]
pub struct Effect {
    pub prefab: u32,
    /// root transform (Unity world) when not attached
    pub pos: Vec3,
    pub rot: Quat,
    pub scale: Vec3,
    /// parented to a scene node: (node, root transform local to it)
    pub attach: Option<(u32, Mat4)>,
    pub active: bool,
    /// per prefab node: activeSelf
    pub node_active: Vec<bool>,
    /// per prefab node: current world matrix (Unity)
    pub world: Vec<Mat4>,
    pub systems: Vec<System>,
    pub splatter: Option<Splatter>,
    /// BloodUnderwaterChecker.cancelled / done
    pub checker_done: bool,
    /// RemoveOnTime due time
    pub remove_at: Option<f64>,
    pub destroyed: bool,
}

/// Global effect state: live instances and the gore pools.
#[derive(Default, Clone, Debug)]
pub struct Fx {
    pub effects: Vec<Effect>,
    /// BloodsplatterManager script index
    pub bsm: Option<u32>,
    /// prefab index per pooled BSType
    pub pool_prefab: [Option<u32>; 8],
    /// pooled (inactive) objects per BSType: their world position (they stay where they were
    /// repooled; SetParent(goreStore) keeps world position)
    pub pools: Vec<VecDeque<Vec3>>,
    /// goreStore world matrix (Unity)
    pub gore_store: Mat4,
    pub rng: Rng,
    /// PortalAwareParticleSystem bloodstain requests (stains are not drawn yet)
    pub stains: u64,
    /// heals granted by Bloodsplatter triggers (probe counter)
    pub heals: Vec<i32>,
    /// PrefsManager bloodEnabled (BloodsplatterManager.goreOn)
    pub gore_on: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Rng(pub u32);

impl Default for Rng {
    fn default() -> Self {
        Rng(0x9E37_79B9)
    }
}

impl Rng {
    /// uniform in [0, 1)
    pub fn f(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    fn on_unit_sphere(&mut self) -> Vec3 {
        let z = self.f() * 2.0 - 1.0;
        let a = self.f() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), r * a.sin(), z)
    }
}

/// Quaternion.Euler (degrees): Z, then X, then Y.
pub fn unity_euler(e: Vec3) -> Quat {
    let r = e * (std::f32::consts::PI / 180.0);
    Quat::from_rotation_y(r.y) * Quat::from_rotation_x(r.x) * Quat::from_rotation_z(r.z)
}

/// Quaternion.LookRotation(forward, Vector3.up).
pub fn look_rotation(f: Vec3) -> Quat {
    let f = f.normalize_or_zero();
    if f == Vec3::ZERO {
        return Quat::IDENTITY;
    }
    let mut x = Vec3::Y.cross(f);
    if x.length_squared() < 1e-12 {
        x = Vec3::X;
    }
    let x = x.normalize();
    let y = f.cross(x);
    Quat::from_mat3(&Mat3::from_cols(x, y, f))
}

/// A prefab node's local matrix in Unity space.
fn local_unity(pf: &ParticlePrefab, n: usize) -> Mat4 {
    let nd = &pf.nodes[n];
    FLIP * Mat4::from_scale_rotation_translation(nd.local_scale, nd.local_rot, nd.local_pos) * FLIP
}

/// Prefab root's local transform in Unity space: (position, rotation, scale).
fn root_unity(pf: &ParticlePrefab) -> (Vec3, Quat, Vec3) {
    let (s, r, t) = local_unity(pf, 0).to_scale_rotation_translation();
    (t, r, s)
}

impl Effect {
    fn new(def_prefab: &ParticlePrefab, prefab: u32) -> Self {
        let systems = def_prefab.nodes.iter().enumerate().filter_map(|(i, n)| n.system.clone().map(|s| System::new(i as u32, s))).collect();
        let (pos, rot, scale) = root_unity(def_prefab);
        Self {
            prefab,
            pos,
            rot,
            scale,
            attach: None,
            active: false,
            node_active: def_prefab.nodes.iter().map(|n| n.active_self).collect(),
            world: vec![Mat4::IDENTITY; def_prefab.nodes.len()],
            systems,
            splatter: None,
            checker_done: false,
            remove_at: None,
            destroyed: false,
        }
    }

    pub fn node_active_in_hierarchy(&self, pf: &ParticlePrefab, mut n: u32) -> bool {
        loop {
            if !self.node_active[n as usize] {
                return false;
            }
            match pf.nodes[n as usize].parent {
                Some(p) => n = p,
                None => return true,
            }
        }
    }

    /// ParticleSystem.isStopped of the root system with its children.
    pub fn stopped(&self) -> bool {
        self.systems.iter().all(|s| s.stopped())
    }

    /// ParticleSystem.Play() on the root (with children): systems on inactive objects stay off.
    fn play(&mut self, pf: &ParticlePrefab, rng: &mut Rng) {
        for s in &mut self.systems {
            if node_active_up(pf, &self.node_active, s.node) {
                s.play(rng);
            }
        }
    }

    fn clear(&mut self) {
        for s in &mut self.systems {
            s.clear();
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Game side

impl Game {
    pub(crate) fn fx_init(&mut self) {
        let def = self.def.clone();
        let mut fx = Fx { gore_on: self.prefs.bool("bloodEnabled", true), ..Default::default() };
        fx.pools = vec![VecDeque::new(); POOLS.len()];
        fx.bsm = def.scripts.iter().position(|s| s.class == "BloodsplatterManager").map(|i| i as u32);
        if let Some(b) = fx.bsm {
            for (i, f) in POOLS.iter().enumerate() {
                fx.pool_prefab[i] = def.script_prefabs.get(&(b, f.to_string())).copied();
            }
            let store = def.node_ref(def.scripts[b as usize].data.get("goreStore"));
            fx.gore_store = store.map_or(Mat4::IDENTITY, |n| self.anim.world_of(n));
            // BloodsplatterManager.InitPool: 100 of each (200 bodies) instantiated under goreStore
            for i in 0..POOLS.len() {
                let Some(p) = fx.pool_prefab[i] else { continue };
                let (pos, _, _) = root_unity(&def.particle_prefabs[p as usize]);
                let at = fx.gore_store.transform_point3(pos);
                let n = if i == 2 { 200 } else { 100 };
                fx.pools[i] = std::iter::repeat_n(at, n).collect();
            }
        }
        self.fx = fx;
    }

    /// Current world matrix (Unity) of a scene node, movers included.
    pub fn node_world_now(&self, n: u32) -> Mat4 {
        let mover = match self.node_mover[n as usize] {
            Some(mv) => FLIP * Mat4::from(self.mover_delta(mv)) * FLIP,
            None => Mat4::IDENTITY,
        };
        mover * self.anim.world_of(n)
    }

    /// Object.Instantiate of an effect prefab (active, so its systems with playOnAwake start).
    pub fn fx_instantiate(&mut self, prefab: u32, pos: Vec3, rot: Quat) -> usize {
        let pf = self.def.particle_prefabs[prefab as usize].clone();
        let mut e = Effect::new(&pf, prefab);
        e.pos = pos;
        e.rot = rot;
        e.splatter = splatter_of(&pf);
        if let Some(r) = pf.script(0, "RemoveOnTime") {
            let t = r.data.get("time").f32() + (self.fx.rng.f() * 2.0 - 1.0) * r.data.get("randomizer").f32();
            e.remove_at = Some(self.s.time + t as f64);
        }
        self.fx.effects.push(e);
        let i = self.fx.effects.len() - 1;
        self.fx_enable(i);
        i
    }

    /// GameObject.SetActive(true) on an effect root: playOnAwake systems start and the
    /// Bloodsplatter's OnEnable runs.
    fn fx_enable(&mut self, i: usize) {
        let pf = self.def.particle_prefabs[self.fx.effects[i].prefab as usize].clone();
        let now = self.s.time;
        let gore_on = self.fx.gore_on;
        let Fx { effects, rng, .. } = &mut self.fx;
        let e = &mut effects[i];
        e.active = true;
        e.node_active[0] = true;
        for s in &mut e.systems {
            if s.def.play_on_awake && node_active_up(&pf, &e.node_active, s.node) {
                s.play(rng);
            }
        }
        let Some(sp) = e.splatter.as_mut() else { return };
        let delay = if sp.underwater { 2.5 } else { 0.25 };
        if sp.been_played {
            if sp.col_enabled {
                sp.disable_at = Some(now + delay);
            }
            return;
        }
        sp.been_played = true;
        e.clear();
        if gore_on {
            e.play(&pf, rng);
        }
        let sp = e.splatter.as_mut().unwrap();
        sp.can_collide = true;
        sp.col_enabled = true;
        sp.disable_at = Some(now + delay);
    }

    /// BloodsplatterManager.GetFromQueue: a pooled object of `bs` activated where it was left.
    fn fx_from_queue(&mut self, bs: usize) -> Option<usize> {
        let p = self.fx.pool_prefab[bs]?;
        let pf = self.def.particle_prefabs[p as usize].clone();
        let pos = match self.fx.pools[bs].pop_front() {
            Some(pos) => pos,
            // Instantiate(prefab, goreStore)
            None => self.fx.gore_store.transform_point3(root_unity(&pf).0),
        };
        let mut e = Effect::new(&pf, p);
        e.pos = pos;
        e.splatter = splatter_of(&pf);
        self.fx.effects.push(e);
        let i = self.fx.effects.len() - 1;
        self.fx_enable(i);
        Some(i)
    }

    /// BloodsplatterManager.GetGore (not blessed, not sandified; `eid` set is irrelevant here).
    pub fn get_gore(&mut self, got: GoreType, underwater: bool, from_explosion: bool) -> Option<usize> {
        let (bs, scale, hp) = match (got, underwater) {
            (GoreType::Head, false) => (0, 1.0, -1),
            (GoreType::Limb, false) => (1, 1.0, -1),
            (GoreType::Body, false) => (2, 1.0, -1),
            (GoreType::Small, false) => (3, 1.0, -1),
            (GoreType::Smallest, false) => (4, 1.0, -1),
            (GoreType::Splatter, false) => (5, 1.0, -1),
            (GoreType::Head | GoreType::Splatter, true) => (BS_UNDERWATER, 1.0, -1),
            (GoreType::Limb, true) => (BS_UNDERWATER, 0.75, 20),
            (GoreType::Body, true) => (BS_UNDERWATER, 0.5, 10),
            (GoreType::Small, true) => (BS_UNDERWATER, 0.25, 10),
            (GoreType::Smallest, true) => (BS_UNDERWATER, 0.15, 5),
        };
        let i = self.fx_from_queue(bs)?;
        let e = &mut self.fx.effects[i];
        e.scale *= scale;
        if let Some(sp) = e.splatter.as_mut() {
            // PrepareGore
            if hp >= 0 {
                sp.hp = hp;
            }
            if from_explosion {
                sp.from_explosion = true;
            }
        }
        Some(i)
    }

    /// Object.Instantiate of a live effect (MaliciousFace instantiates the pooled object GetGore
    /// returned): serialized state is copied, the particles are not, and the copy is not pooled
    /// in place of the original.
    pub fn fx_clone(&mut self, i: usize) -> usize {
        let src = &self.fx.effects[i];
        let mut e = Effect::new(&self.def.particle_prefabs[src.prefab as usize], src.prefab);
        e.pos = src.pos;
        e.rot = src.rot;
        e.scale = src.scale;
        e.attach = src.attach;
        e.node_active = src.node_active.clone();
        // public fields (hpAmount, ready, beenPlayed, fromExplosion, underwater) serialize;
        // canCollide is private and starts true
        e.splatter = src.splatter.clone().map(|s| Splatter { can_collide: true, disable_at: None, inside: false, ..s });
        self.fx.effects.push(e);
        let j = self.fx.effects.len() - 1;
        self.fx_enable(j);
        j
    }

    pub fn fx_play(&mut self, i: usize) {
        let pf = self.def.particle_prefabs[self.fx.effects[i].prefab as usize].clone();
        let Fx { effects, rng, .. } = &mut self.fx;
        effects[i].play(&pf, rng);
    }

    /// Bloodsplatter.Repool / ReturnToQueue.
    fn fx_repool(&mut self, i: usize) {
        let e = &mut self.fx.effects[i];
        let Some(sp) = &e.splatter else {
            e.destroyed = true;
            return;
        };
        e.destroyed = true;
        let bs = sp.bs_type;
        if (0..POOLS.len() as i64).contains(&bs) && bs < BS_DONTPOOL {
            let pos = self.fx.effects[i].pos;
            self.fx.pools[bs as usize].push_back(pos);
        }
    }

    /// Particle and script updates, after the frame's scripts (Unity simulates particles late in
    /// the frame).
    pub(crate) fn fx_update(&mut self, dt: f32) {
        let now = self.s.time;
        // invokes: Bloodsplatter.DisableCollider, RemoveOnTime.Remove
        let mut repool = Vec::new();
        for (i, e) in self.fx.effects.iter_mut().enumerate() {
            if e.destroyed || !e.active {
                continue;
            }
            if e.remove_at.is_some_and(|t| t <= now) {
                e.destroyed = true;
                continue;
            }
            let Some(sp) = e.splatter.as_mut() else { continue };
            if sp.disable_at.is_some_and(|t| t <= now) {
                sp.disable_at = None;
                sp.can_collide = false;
                if e.systems.first().is_none_or(|s| s.stopped()) {
                    repool.push(i);
                }
            }
        }
        for i in repool {
            self.fx_repool(i);
        }
        // transforms
        for i in 0..self.fx.effects.len() {
            if self.fx.effects[i].destroyed {
                continue;
            }
            let root = match self.fx.effects[i].attach {
                Some((n, local)) => self.node_world_now(n) * local,
                None => {
                    let e = &self.fx.effects[i];
                    Mat4::from_scale_rotation_translation(e.scale, e.rot, e.pos)
                }
            };
            let pf = self.def.particle_prefabs[self.fx.effects[i].prefab as usize].clone();
            let e = &mut self.fx.effects[i];
            e.world[0] = root;
            for n in 1..pf.nodes.len() {
                let p = pf.nodes[n].parent.unwrap_or(0) as usize;
                e.world[n] = e.world[p] * local_unity(&pf, n);
            }
            if let Some((_, _)) = e.attach {
                let (_, r, t) = root.to_scale_rotation_translation();
                e.pos = t;
                e.rot = r;
            }
        }
        // simulation
        let world = std::mem::take(&mut self.world);
        let def = self.def.clone();
        let solid = |id| {
            let o = world.owner(id);
            o != uk_core::collide::ALWAYS && def.colliders.get(o as usize).is_some_and(|c| !c.trigger && STAIN_LAYERS.contains(&c.layer))
        };
        let mut stopped = Vec::new();
        let Fx { effects, rng, stains, .. } = &mut self.fx;
        for (i, e) in effects.iter_mut().enumerate() {
            if e.destroyed || !e.active {
                continue;
            }
            let pf = &def.particle_prefabs[e.prefab as usize];
            let was_stopped = e.stopped();
            // the Bloodsplatter is found from the root (GetComponentInChildren): only the root
            // system's hits queue stains
            let stain_root = e.splatter.is_some();
            for s in &mut e.systems {
                if !e.node_active[s.node as usize] || !node_active_up(pf, &e.node_active, s.node) {
                    continue;
                }
                let w = e.world[s.node as usize];
                s.sim_to_world = sim_matrix(w, &pf.nodes[s.node as usize], &s.def);
                step_system(s, dt, rng);
                // PortalAwareParticleSystem: a particle whose last move crossed the environment dies
                let m = s.sim_to_world;
                for p in &mut s.particles {
                    let now_w = m.transform_point3(p.pos);
                    if !p.kill {
                        let d = now_w - p.prev_world;
                        let len = d.length();
                        if len > 1e-6 {
                            if world.raycast_filtered(to_bevy(p.prev_world), to_bevy(d / len), len, &solid).is_some() {
                                p.kill = true;
                                if stain_root && s.node == 0 {
                                    *stains += 1;
                                }
                            }
                        }
                    }
                    p.prev_world = now_w;
                }
            }
            // stopAction Callback -> Bloodsplatter.Repool
            if !was_stopped && e.stopped() && e.splatter.is_some() {
                stopped.push(i);
            }
        }
        self.world = world;
        for i in stopped {
            self.fx_repool(i);
        }
        self.fx.effects.retain(|e| !e.destroyed);
    }

    /// Physics-step parts: Bloodsplatter heal triggers and BloodUnderwaterChecker.
    pub(crate) fn fx_fixed_update(&mut self) {
        let cap = self.s.player.capsule();
        let mut heal = Vec::new();
        let mut checks = Vec::new();
        for (i, e) in self.fx.effects.iter_mut().enumerate() {
            if e.destroyed || !e.active {
                continue;
            }
            let pf = &self.def.particle_prefabs[e.prefab as usize];
            if let Some(sp) = e.splatter.as_mut() {
                let inside = sp.col_enabled
                    && pf.nodes[0].sphere.is_some_and(|(c, r, _, on)| {
                        let w = e.world[0];
                        let center = to_bevy(w.transform_point3(to_unity(c)));
                        let s = w.to_scale_rotation_translation().0.abs().max_element();
                        on && seg_dist(cap.a, cap.b, center) < r * s + cap.radius
                    });
                if inside && !sp.inside && sp.ready && sp.can_collide {
                    heal.push((sp.hp, sp.from_explosion));
                    // DisableCollider
                    sp.can_collide = false;
                    if e.systems.first().is_none_or(|s| s.stopped()) {
                        checks.push((i, None));
                    }
                }
                sp.inside = inside;
            }
            if !e.checker_done {
                if let Some(n) = pf.nodes.iter().position(|n| n.scripts.iter().any(|s| s.class == "BloodUnderwaterChecker")) {
                    if e.node_active_in_hierarchy(pf, n as u32) {
                        let (c, r, _, _) = pf.nodes[n].sphere.unwrap_or((Vec3::ZERO, 0.5, true, true));
                        let w = e.world[n];
                        let center = w.transform_point3(to_unity(c));
                        let rad = r * w.to_scale_rotation_translation().0.abs().max_element();
                        checks.push((i, Some((center, rad))));
                    }
                }
            }
        }
        for (hp, _fx) in heal {
            // NewMovement.GetHealth(hpAmount, silent: false, fromExplosion)
            self.heal_player(hp.max(1));
            self.fx.heals.push(hp);
        }
        for (i, chk) in checks {
            match chk {
                None => self.fx_repool(i),
                Some((center, rad)) => self.underwater_check(i, center, rad),
            }
        }
        self.fx.effects.retain(|e| !e.destroyed);
    }

    /// BloodUnderwaterChecker.OnTriggerEnter against water (layer 4) colliders.
    fn underwater_check(&mut self, i: usize, center: Vec3, rad: f32) {
        self.fx.effects[i].checker_done = true;
        let cb = to_bevy(center);
        let probe = Capsule { a: cb, b: cb, radius: rad };
        let up = cb + Vec3::Y * 1.5;
        let hit = self.waters_colliders().into_iter().any(|ci| {
            let c = &self.def.colliders[ci as usize];
            c.layer == 4 && self.trigger_contains(ci, &probe) && self.collider_bounds_closest(ci, up).distance(up) < 0.5
        });
        if !hit {
            return;
        }
        let Some(j) = self.get_gore(GoreType::Body, true, false) else { return };
        let src = self.fx.effects[i].splatter.clone();
        let e = &mut self.fx.effects[j];
        if let (Some(s), Some(d)) = (src, e.splatter.as_mut()) {
            d.hp = s.hp;
            d.from_explosion = s.from_explosion;
            if s.ready {
                d.ready = true;
            }
        }
        e.pos = center;
        // the original goes inactive (never repooled)
        self.fx.effects[i].active = false;
        self.fx.effects[i].destroyed = true;
    }
}

fn splatter_of(pf: &ParticlePrefab) -> Option<Splatter> {
    let s = pf.script(0, "Bloodsplatter")?;
    Some(Splatter {
        bs_type: s.data.get("bloodSplatterType").i64(),
        hp: s.data.get("hpAmount").i64() as i32,
        ready: s.data.get("ready").bool(),
        can_collide: true,
        been_played: s.data.get("beenPlayed").bool(),
        from_explosion: s.data.get("fromExplosion").bool(),
        underwater: s.data.get("underwater").bool(),
        col_enabled: pf.nodes[0].sphere.is_some_and(|s| s.3),
        disable_at: None,
        inside: false,
    })
}

fn node_active_up(pf: &ParticlePrefab, active: &[bool], mut n: u32) -> bool {
    loop {
        if !active[n as usize] {
            return false;
        }
        match pf.nodes[n as usize].parent {
            Some(p) => n = p,
            None => return true,
        }
    }
}

fn seg_dist(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

/// Simulation space -> world for a system on a node with world matrix `w`: position and rotation
/// of the node, scaled per scalingMode (0 hierarchy: lossy scale, 1 local: the node's own
/// localScale, 2 shape: none).
fn sim_matrix(w: Mat4, node: &uk_assets::particles::PrefabNode, def: &ParticleSystemDef) -> Mat4 {
    let (lossy, r, t) = w.to_scale_rotation_translation();
    let s = match def.scaling_mode {
        0 => lossy,
        1 => node.local_scale,
        _ => Vec3::ONE,
    };
    Mat4::from_scale_rotation_translation(s, r, t)
}

/// Shape module: (position, direction) in simulation space.
fn emit_shape(def: &ParticleSystemDef, rng: &mut Rng) -> (Vec3, Vec3) {
    let Some(sh) = &def.shape else { return (Vec3::ZERO, Vec3::Z) };
    let arc = sh.arc.to_radians();
    let (p, d) = match sh.kind {
        // Sphere: radiusThickness 1 fills the volume, 0 is the surface
        0 | 1 => {
            let u = rng.on_unit_sphere();
            let inner = (1.0 - sh.radius_thickness).max(0.0).powi(3);
            let r = sh.radius * (inner + (1.0 - inner) * rng.f()).cbrt();
            (u * r, u)
        }
        // Hemisphere
        2 | 3 => {
            let mut u = rng.on_unit_sphere();
            u.z = u.z.abs();
            let inner = (1.0 - sh.radius_thickness).max(0.0).powi(3);
            let r = sh.radius * (inner + (1.0 - inner) * rng.f()).cbrt();
            (u * r, u)
        }
        // Cone (base) / ConeVolume: a point on the base disk, heading along the cone's side
        4 | 7 | 8 | 9 => {
            let th = rng.f() * arc;
            let inner = (1.0 - sh.radius_thickness).max(0.0).powi(2);
            let rn = (inner + (1.0 - inner) * rng.f()).sqrt();
            let dir2 = Vec3::new(th.cos(), th.sin(), 0.0);
            let tan = sh.angle.to_radians().tan();
            let d = (dir2 * rn * tan + Vec3::Z).normalize();
            let base = dir2 * rn * sh.radius;
            let p = if sh.kind == 7 || sh.kind == 9 { base + d * (sh.length * rng.f() / d.z.max(1e-6)) } else { base };
            (p, d)
        }
        // Circle: the XY disk, heading outward
        10 | 11 => {
            let th = rng.f() * arc;
            let inner = (1.0 - sh.radius_thickness).max(0.0).powi(2);
            let rn = (inner + (1.0 - inner) * rng.f()).sqrt();
            let dir2 = Vec3::new(th.cos(), th.sin(), 0.0);
            (dir2 * rn * sh.radius, dir2)
        }
        // Box (5 volume, 15 shell, 16 edge): volume, heading +z
        _ => {
            let p = Vec3::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5);
            (p, Vec3::Z)
        }
    };
    let q = unity_euler(sh.rotation);
    let p = sh.position + q * (p * sh.scale);
    let d = (q * (d * sh.scale)).normalize_or(Vec3::Z);
    (p, d)
}

fn spawn(s: &System, rng: &mut Rng) -> Particle {
    let def = &s.def;
    let tn = if def.duration > 0.0 { (s.time / def.duration).clamp(0.0, 1.0) } else { 0.0 };
    let lifetime = def.start_lifetime.eval(tn, rng.f()).max(1e-4);
    let speed = def.start_speed.eval(tn, rng.f());
    let size = def.start_size.eval(tn, rng.f());
    let rot = def.start_rotation.eval(tn, rng.f());
    let color = def.start_color.eval(tn, rng.f());
    let gravity = def.gravity_modifier.eval(tn, rng.f());
    let (pos, dir) = emit_shape(def, rng);
    let rand = [rng.f(), rng.f(), rng.f(), rng.f()];
    let trail = def.trail.as_ref().and_then(|t| {
        (rng.f() < t.ratio).then(|| TrailState { points: vec![(pos, 0.0)], vertex_life: t.lifetime.eval(tn, rng.f()) * lifetime })
    });
    Particle {
        pos,
        vel: dir * speed,
        age: 0.0,
        lifetime,
        start_size: size,
        start_color: color,
        rot,
        gravity,
        rand,
        trail,
        prev_world: s.sim_to_world.transform_point3(pos),
        kill: false,
    }
}

fn step_particle(p: &mut Particle, def: &ParticleSystemDef, g_local: Vec3, dt: f32) -> bool {
    p.age += dt;
    if p.age >= p.lifetime {
        return false;
    }
    p.vel += g_local * p.gravity * dt;
    let t = p.age / p.lifetime;
    let mut v = p.vel;
    if let Some(vol) = &def.velocity_over_lifetime {
        v += Vec3::new(vol.x.eval(t, p.rand[2]), vol.y.eval(t, p.rand[2]), vol.z.eval(t, p.rand[2]));
        v *= vol.speed_modifier.eval(t, p.rand[2]);
    }
    p.pos += v * dt;
    if let Some(r) = &def.rotation_over_lifetime {
        p.rot += r.eval(t, p.rand[3]) * dt;
    }
    if let (Some(tr), Some(td)) = (p.trail.as_mut(), def.trail.as_ref()) {
        let age = p.age;
        tr.points.retain(|(_, born)| age - born < tr.vertex_life);
        let far = tr.points.last().is_none_or(|(q, _)| q.distance(p.pos) >= td.min_vertex_distance);
        if far {
            tr.points.push((p.pos, age));
        }
    }
    true
}

fn step_system(s: &mut System, dt: f32, rng: &mut Rng) {
    let dt = dt * s.def.simulation_speed;
    // particles that hit the environment last update die now
    s.particles.retain(|p| !p.kill);
    let (_, rot, _) = s.sim_to_world.to_scale_rotation_translation();
    let g_local = rot.inverse() * Vec3::new(0.0, GRAVITY, 0.0);
    let def = s.def.clone();
    s.particles.retain_mut(|p| step_particle(p, &def, g_local, dt));
    if !s.playing {
        return;
    }
    let mut dt_sys = dt;
    if s.delay > 0.0 {
        let used = s.delay.min(dt_sys);
        s.delay -= used;
        dt_sys -= used;
        if dt_sys <= 0.0 {
            return;
        }
    }
    if !s.emitting() {
        return;
    }
    let (t0, t1) = (s.time, s.time + dt_sys);
    let end = if def.looping { t1 } else { t1.min(def.duration) };
    let mut born: Vec<(f32, Particle)> = Vec::new();
    if def.emission_enabled {
        for (bi, b) in def.bursts.iter().enumerate() {
            while s.burst_next[bi] < b.cycles.max(1) {
                let bt = b.time + s.burst_next[bi] as f32 * b.interval;
                if bt >= end {
                    break;
                }
                s.burst_next[bi] += 1;
                if rng.f() > b.probability {
                    continue;
                }
                let tn = if def.duration > 0.0 { t0 / def.duration } else { 0.0 };
                let n = b.count.eval(tn, rng.f()).round().max(0.0) as u32;
                for _ in 0..n {
                    born.push((t1 - bt, spawn(s, rng)));
                }
            }
        }
        let tn = if def.duration > 0.0 { t0 / def.duration } else { 0.0 };
        let rate = def.rate_over_time.eval(tn, rng.f());
        if rate > 0.0 && end > t0 {
            s.rate_acc += rate * (end - t0);
            let n = s.rate_acc.floor();
            s.rate_acc -= n;
            for k in 0..n as u32 {
                let te = t0 + (end - t0) * (k as f32 + 0.5) / n;
                born.push((t1 - te, spawn(s, rng)));
            }
        }
    }
    s.time = t1;
    for (age, mut p) in born {
        if s.particles.len() as u32 >= def.max_particles {
            break;
        }
        if step_particle(&mut p, &def, g_local, age) {
            s.particles.push(p);
        }
    }
}
