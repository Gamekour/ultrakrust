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
use uk_assets::particles::{MinMaxGradient, ParticlePrefab, ParticleSystemDef};
use uk_assets::serialized::Value;
use uk_core::umath::normalized;
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
    /// the particle's own velocity (start speed, gravity, forces, collisions)
    pub vel: Vec3,
    /// the whole motion of the last update (own velocity plus the velocity / noise / orbital
    /// modules): stretched billboards, speed-driven curves
    pub total_vel: Vec3,
    pub age: f32,
    pub lifetime: f32,
    /// per axis (all equal unless size3D)
    pub start_size: Vec3,
    pub start_color: [f32; 4],
    /// radians per axis; z is the billboard roll
    pub rot: Vec3,
    /// -1 for the flipRotation share of particles
    pub rot_sign: f32,
    pub gravity: f32,
    /// per-particle random blends: 0 color, 1 size, 2 velocity, 3 rotation, 4 force / inherit,
    /// 5 limit / lifetime by speed, 6 noise, 7 collision
    pub rand: [f32; 8],
    /// texture sheet: start frame (normalized), row, flip u / v
    pub sheet_start: f32,
    pub sheet_row: u32,
    pub flip: [bool; 2],
    /// renderer mesh choice (render mode 4)
    pub mesh_pick: f32,
    /// NoiseModule size multiplier this update
    pub noise_size: f32,
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

    /// Size per axis: start size, SizeOverLifetime (separate axes or not), noise.
    pub fn size3(&self, def: &ParticleSystemDef) -> Vec3 {
        let t = self.age / self.lifetime;
        let r = self.rand[1];
        let m = match (&def.size_over_lifetime, &def.size_axes) {
            (Some(x), Some([y, z])) => Vec3::new(x.eval(t, r), y.eval(t, r), z.eval(t, r)),
            (Some(x), None) => Vec3::splat(x.eval(t, r)),
            _ => Vec3::ONE,
        };
        self.start_size * m * self.noise_size
    }

    pub fn size(&self, def: &ParticleSystemDef) -> f32 {
        self.size3(def).x
    }

    /// TextureSheetAnimation (grid mode): the frame's UV rect (u0, v0, du, dv), negative extents
    /// for flipped axes. Frames run left to right, rows from the top of the texture.
    pub fn sheet_uv(&self, def: &ParticleSystemDef) -> Option<[f32; 4]> {
        let ts = def.texture_sheet.as_ref()?;
        let (tx, ty) = (ts.tiles_x.max(1), ts.tiles_y.max(1));
        let single = ts.animation_type == 1;
        let frames = if single { tx } else { tx * ty };
        let t = self.age / self.lifetime;
        let r = self.rand[6];
        let n = match ts.time_mode {
            1 => {
                let range = ts.speed_range;
                let f = if range[1] > range[0] { ((self.total_vel.length() - range[0]) / (range[1] - range[0])).clamp(0.0, 1.0) } else { 0.0 };
                ts.frame_over_time.eval(f, r)
            }
            2 => self.age * ts.fps / frames as f32,
            _ => ts.frame_over_time.eval((t * ts.cycles).fract(), r),
        };
        let f = (((n + self.sheet_start) * frames as f32).floor() as i64).rem_euclid(frames as i64) as u32;
        let (col, row) = if single { (f, self.sheet_row.min(ty - 1)) } else { (f % tx, f / tx) };
        let (du, dv) = (1.0 / tx as f32, 1.0 / ty as f32);
        let (mut u0, mut v0, mut su, mut sv) = (col as f32 * du, 1.0 - (row + 1) as f32 * dv, du, dv);
        if self.flip[0] {
            u0 += du;
            su = -du;
        }
        if self.flip[1] {
            v0 += dv;
            sv = -dv;
        }
        Some([u0, v0, su, sv])
    }
}

pub fn mul4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2], a[3] * b[3]]
}

/// One ParticleSystem instance (on prefab node `node`, or scene node `node` for scene systems).
#[derive(Clone, Debug)]
pub struct System {
    pub node: u32,
    pub def: Arc<ParticleSystemDef>,
    /// time within the current cycle (after the start delay)
    pub time: f32,
    /// completed loop cycles
    pub cycle: u32,
    pub delay: f32,
    pub playing: bool,
    pub particles: Vec<Particle>,
    rate_acc: f32,
    dist_acc: f32,
    /// a script's emission.rateOverDistanceMultiplier: replaces the curve's scalar (the constant
    /// itself in Constant mode)
    pub rate_over_distance_mul: Option<f32>,
    burst_next: Vec<u32>,
    /// Play() with prewarm: one full cycle is simulated before the next update
    prewarm_pending: bool,
    /// the emitter's world position last update (rateOverDistance)
    prev_emitter: Option<Vec3>,
    /// emitter (node with the scaling mode applied) -> world (Unity), refreshed every update
    pub emitter: Mat4,
    /// simulation space -> world (Unity): the emitter for local simulation space, identity for world
    pub sim_to_world: Mat4,
    /// particle size multiplier from the emitter's scale
    pub size_scale: f32,
    /// the emitter's velocity (world, per emitterVelocityMode) over the last update
    pub emitter_vel: Vec3,
    noise_scroll: f32,
    /// MeshRenderer shape: that renderer's transform in emitter space
    pub shape_xf: Option<Mat4>,
    /// CollisionModule planes this update: (world point, unit normal)
    pub planes: Vec<(Vec3, Vec3)>,
    /// collisions so far (probe counter)
    pub collisions: u64,
}

impl System {
    pub fn new(node: u32, def: Arc<ParticleSystemDef>) -> Self {
        let n = def.bursts.len();
        Self {
            node,
            def,
            time: 0.0,
            cycle: 0,
            delay: 0.0,
            playing: false,
            particles: Vec::new(),
            rate_acc: 0.0,
            dist_acc: 0.0,
            rate_over_distance_mul: None,
            burst_next: vec![0; n],
            prewarm_pending: false,
            prev_emitter: None,
            emitter: Mat4::IDENTITY,
            sim_to_world: Mat4::IDENTITY,
            size_scale: 1.0,
            emitter_vel: Vec3::ZERO,
            noise_scroll: 0.0,
            shape_xf: None,
            planes: Vec::new(),
            collisions: 0,
        }
    }

    /// ParticleSystem.Play (this system alone): a no-op while playing.
    pub fn play(&mut self, rng: &mut Rng) {
        if self.playing {
            return;
        }
        self.playing = true;
        self.time = 0.0;
        self.cycle = 0;
        self.rate_acc = 0.0;
        self.dist_acc = 0.0;
        self.prev_emitter = None;
        self.burst_next.iter_mut().for_each(|b| *b = 0);
        // prewarm only applies to looping systems and replaces the start delay
        self.prewarm_pending = self.def.prewarm && self.def.looping;
        self.delay = if self.prewarm_pending { 0.0 } else { self.def.start_delay.eval(0.0, rng.f()) };
    }

    /// ParticleSystem.Stop(withChildren, StopEmitting): live particles play out.
    pub fn stop(&mut self) {
        self.playing = false;
    }

    pub fn clear(&mut self) {
        self.particles.clear();
    }

    /// The emitter's world matrix this update: `w` is the node's world matrix, `local_scale` its
    /// localScale.
    pub fn set_transform(&mut self, w: Mat4, local_scale: Vec3) {
        self.emitter = sim_matrix(w, local_scale, &self.def);
        self.sim_to_world = if self.def.simulation_space == 1 { Mat4::IDENTITY } else { self.emitter };
        self.size_scale = self.emitter.x_axis.truncate().length();
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
    /// per prefab node: a script's multiplier on its localScale (EnviroGibModifier)
    pub node_scale: Vec<f32>,
    pub systems: Vec<System>,
    pub splatter: Option<Splatter>,
    /// BloodUnderwaterChecker.cancelled / done
    pub checker_done: bool,
    /// RemoveOnTime due time
    pub remove_at: Option<f64>,
    /// RemoveOnTime on child nodes: (node, due time); the node and its subtree are destroyed
    pub child_remove: Vec<(u32, f64)>,
    pub destroyed: bool,
    /// the script reference holding this instance (TAG_*), 0 when none
    pub tag: u8,
}

/// NewMovement.currentFallParticle
pub const TAG_FALL: u8 = 1;
/// NewMovement.currentSlideParticle
pub const TAG_SLIDE: u8 = 2;
/// NewMovement.slideScrape
pub const TAG_SLIDE_SCRAPE: u8 = 3;
/// NewMovement.wallScrape
pub const TAG_WALL_SCRAPE: u8 = 4;
/// NewMovement.currentFrictionlessSlideParticle
pub const TAG_FRIC_SLIDE: u8 = 5;
/// `Effect::attach` node standing for the player's transform
pub const ATTACH_PLAYER: u32 = u32::MAX;

/// A ParticleSystem in the scene graph (`SceneDef::particle_systems[index]`). It follows its
/// GameObject: activation plays it when playOnAwake is set, deactivation stops and clears it.
#[derive(Clone, Debug)]
pub struct SceneSystem {
    pub index: u32,
    pub sys: System,
    /// activeInHierarchy at the last update
    pub on: bool,
    /// a PortalAwareParticleSystem on the same GameObject
    pub portal_aware: bool,
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
    /// the scene's own ParticleSystems
    pub scene: Vec<SceneSystem>,
    /// NewMovement script index
    pub nm: Option<u32>,
    /// `scene` index of NewMovement.windStateParticle
    pub wind: Option<usize>,
    /// NewMovement.normalSlideGradient (the slide particle's own trail colorOverLifetime) and
    /// invincibleSlideGradient
    pub slide_gradients: Option<(MinMaxGradient, MinMaxGradient)>,
    /// NewMovement.currentSlideSurfaceType / currentScrapeSurfaceType
    pub slide_surface: i32,
    pub scrape_surface: i32,
    /// NewMovement.currentFricSlideSurfaceType
    pub fric_surface: i32,
    /// per prefab: instances spawned so far and the last one's position and rotation (probes)
    pub spawns: std::collections::BTreeMap<u32, (u32, Vec3, Quat)>,
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

/// Transform.forward = f: rotation = Quaternion.FromToRotation(forward, f) * rotation.
pub fn set_forward(rot: Quat, f: Vec3) -> Quat {
    let f = f.normalize_or_zero();
    if f == Vec3::ZERO {
        return rot;
    }
    Quat::from_rotation_arc((rot * Vec3::Z).normalize(), f) * rot
}

/// Vector3.ProjectOnPlane(v, n).normalized
fn project_on_plane_n(v: Vec3, n: Vec3) -> Vec3 {
    normalized(v - n * v.dot(n))
}

/// SceneHelper.MultiplyCurve: the curve multiplier, both constants, or the constant.
fn multiply_curve(c: &mut uk_assets::particles::MinMaxCurve, m: f32) {
    c.scalar *= m;
    if c.mode == 3 {
        c.min_scalar *= m;
    }
}

/// Mathf.RoundToInt (banker's rounding).
pub fn round_half_even(x: f32) -> i32 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r as i32 % 2 != 0 {
        (r - x.signum()) as i32
    } else {
        r as i32
    }
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
            node_scale: vec![1.0; def_prefab.nodes.len()],
            systems,
            splatter: None,
            checker_done: false,
            remove_at: None,
            child_remove: Vec::new(),
            destroyed: false,
            tag: 0,
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
        fx.scene = def
            .particle_systems
            .iter()
            .enumerate()
            .map(|(i, p)| SceneSystem {
                index: i as u32,
                sys: System::new(p.node, p.system.clone()),
                on: false,
                portal_aware: def.scripts_on(p.node).any(|(_, s)| s.class == "PortalAwareParticleSystem"),
            })
            .collect();
        fx.nm = def.scripts.iter().position(|s| s.class == "NewMovement").map(|i| i as u32);
        if let Some(nm) = fx.nm {
            let nm_node = def.scripts[nm as usize].node;
            fx.wind = fx.scene.iter().position(|ss| {
                let n = &def.nodes[ss.sys.node as usize];
                n.name == "WindStateParticle" && n.parent == Some(nm_node)
            });
        }
        if let Some(nm) = fx.nm {
            let normal = def
                .script_prefabs
                .get(&(nm, "slideParticle".to_string()))
                .and_then(|&p| def.particle_prefabs[p as usize].nodes[0].system.as_ref())
                .and_then(|s| s.trail.as_ref())
                .map(|t| t.color_over_lifetime.clone());
            let inv = MinMaxGradient::read(def.scripts[nm as usize].data.get("invincibleSlideGradient"));
            fx.slide_gradients = normal.map(|n| (n, inv));
        }
        self.fx = fx;
    }

    /// The player's transform (Unity world): rigidbody position, rotation Euler(0, rotationY, 0).
    pub fn player_world(&self) -> Mat4 {
        let p = &self.s.player;
        Mat4::from_rotation_translation(unity_euler(Vec3::new(0.0, p.yaw_deg, 0.0)), to_unity(p.pos))
    }

    /// SceneHelper.CreateEnviroGibs (Bevy space): the surface's enviroGibParticle at the hit,
    /// facing along the normal; EnviroGibModifiers with increaseBurstEmission scale their
    /// system's startSpeed and first burst by `size` (and their localScale by 1 / size), the root
    /// scales by `size`, and a non-white surface color tints the modifiers' systems. (The
    /// rigidbody gib meshes are not particles and are not ported; disableHitParticles is off.)
    pub fn create_enviro_gibs(&mut self, pos: Vec3, dir: Vec3, dist: f32, gib_amount: i32, size: f32) {
        let _ = gib_amount;
        if size <= 0.0 {
            return;
        }
        if let Some(h) = self.surface_data(pos, dir, dist) {
            self.enviro_gibs_at(&h, size);
        }
    }

    /// CreateEnviroGibs past the surface lookup.
    pub fn enviro_gibs_at(&mut self, h: &crate::surface::SurfaceHit, size: f32) {
        let Some(p) = self.def.footstep_fx.enviro_gib_particle(h.surface) else { return };
        let i = self.fx_instantiate(p, to_unity(h.point), look_rotation(to_unity(h.normal)));
        let pf = self.def.particle_prefabs[p as usize].clone();
        // GetComponentsInChildren<EnviroGibModifier>(): active objects only
        let mods: Vec<(usize, &Value)> = (0..pf.nodes.len())
            .filter(|&n| pf.is_active_in_hierarchy(n as u32))
            .flat_map(|n| pf.nodes[n].scripts.iter().filter(|s| s.class == "EnviroGibModifier").map(move |s| (n, &s.data)))
            .collect();
        let e = &mut self.fx.effects[i];
        for &(n, m) in &mods {
            if !m.get("increaseBurstEmission").bool() {
                continue;
            }
            // TryGetComponent<ParticleSystem>: the modifier's own GameObject
            let Some(s) = e.systems.iter_mut().find(|s| s.node == n as u32) else { continue };
            if s.def.bursts.is_empty() {
                continue;
            }
            if n == 0 {
                e.scale /= size;
            } else {
                e.node_scale[n] /= size;
            }
            let d = Arc::make_mut(&mut s.def);
            multiply_curve(&mut d.start_speed, size);
            multiply_curve(&mut d.bursts[0].count, size);
        }
        e.scale *= size;
        self.set_particles_colors(i, h.color);
    }

    /// SceneHelper.IsStaticEnvironment: a non-trigger world collider without a non-kinematic
    /// attachedRigidbody, an enabled Renderer on its GameObject, on an Environment layer.
    pub fn is_static_environment(&self, c: uk_core::collide::ColliderId) -> bool {
        let o = self.world.owner(c);
        let Some(col) = (o != uk_core::collide::ALWAYS).then(|| &self.def.colliders[o as usize]) else { return false };
        if col.trigger || !uk_assets::scenedef::FOOTSTEP_LAYERS.contains(&col.layer) {
            return false;
        }
        // attachedRigidbody: the nearest Rigidbody up the hierarchy
        let mut n = Some(col.node);
        while let Some(k) = n {
            if self.def.rigidbodies.contains(&k) {
                if self.def.dynamic_rigidbodies.contains(&k) {
                    return false;
                }
                break;
            }
            n = self.def.nodes[k as usize].parent;
        }
        self.def.renderers.iter().any(|r| r.node == col.node && r.enabled)
    }

    /// SceneHelper.TryGetSurfaceData (Bevy space).
    pub fn surface_data(&self, pos: Vec3, dir: Vec3, dist: f32) -> Option<crate::surface::SurfaceHit> {
        self.surface.data(&self.def, &self.s.active, pos, dir, dist)
    }

    /// The effect prefab a field of the first script of `class` holding one instantiates.
    pub fn class_prefab(&self, class: &str, field: &str) -> Option<u32> {
        self.def.script_prefabs.iter().filter(|((sc, f), _)| f == field && self.def.scripts[*sc as usize].class == class).min_by_key(|((sc, _), _)| *sc).map(|(_, &p)| p)
    }

    /// The effect prefab a script field (see `scenedef::EFFECT_PREFABS`) instantiates.
    pub fn script_prefab(&self, sc: u32, field: &str) -> Option<u32> {
        self.def.script_prefabs.get(&(sc, field.to_string())).copied()
    }

    /// Object.Instantiate(prefab, parent): the prefab's local transform under a scene node, which
    /// it then follows.
    pub fn fx_instantiate_child(&mut self, prefab: u32, node: u32) -> usize {
        let (t, r, sc) = root_unity(&self.def.particle_prefabs[prefab as usize]);
        let local = Mat4::from_scale_rotation_translation(sc, r, t);
        let (ws, wr, wt) = (self.node_world_now(node) * local).to_scale_rotation_translation();
        let i = self.fx_instantiate(prefab, wt, wr);
        self.fx.effects[i].scale = ws;
        self.fx.effects[i].attach = Some((node, local));
        i
    }

    /// Object.Destroy of the instance a script holds (all its particles go at once).
    pub fn fx_destroy_tag(&mut self, tag: u8) {
        for e in self.fx.effects.iter_mut().filter(|e| e.tag == tag) {
            e.destroyed = true;
        }
    }

    /// NewMovement's particles, after its Update: dodgeParticle on a dodge, slideParticle
    /// following the slide (trail tinted while invincible), fallParticle parented to the player
    /// during a slam, impactDust on a heavy landing (LandingImpact).
    pub(crate) fn fx_player(&mut self, ev0: usize) {
        use uk_core::player::Event;
        let Some(nm) = self.fx.nm else { return };
        let evs: Vec<Event> = self.s.player.events.get(ev0..).unwrap_or_default().to_vec();
        for ev in evs {
            match ev {
                Event::Dash { pos, dir } => {
                    let d = to_unity(dir);
                    if let Some(p) = self.script_prefab(nm, "dodgeParticle") {
                        self.fx_instantiate(p, to_unity(pos) + d * 10.0, look_rotation(-d));
                    }
                }
                Event::SlideStart { pos, dir, boosted, scrape } => {
                    self.fx_destroy_tag(TAG_SLIDE);
                    self.detach_scrape(TAG_SLIDE_SCRAPE);
                    let d = to_unity(dir);
                    if let Some(p) = self.script_prefab(nm, "slideParticle") {
                        let i = self.fx_instantiate(p, to_unity(pos) + d * 10.0, look_rotation(-d));
                        self.fx.effects[i].tag = TAG_SLIDE;
                        self.slide_gradient(i, boosted);
                    }
                    self.create_slide_scrape(true, &scrape, dir, false);
                }
                Event::SlideStop => self.detach_scrape(TAG_SLIDE_SCRAPE),
                Event::Shockwave { gc, up } => {
                    // GroundCheck: Instantiate(shockwave) (the PhysicalShockwave is not ported),
                    // CreateEnviroGibs(position, -up, 5, 10)
                    self.create_enviro_gibs(gc, -up, 5.0, 10, 1.0);
                }
                Event::SlamStart => {
                    self.fx_destroy_tag(TAG_FALL);
                    if let Some(p) = self.script_prefab(nm, "fallParticle") {
                        // Instantiate(fallParticle, transform): the prefab's local transform under the player
                        let (t, r, sc) = root_unity(&self.def.particle_prefabs[p as usize]);
                        let local = Mat4::from_scale_rotation_translation(sc, r, t);
                        let (_, wr, wt) = (self.player_world() * local).to_scale_rotation_translation();
                        let i = self.fx_instantiate(p, wt, wr);
                        self.fx.effects[i].attach = Some((ATTACH_PLAYER, local));
                        self.fx.effects[i].tag = TAG_FALL;
                    }
                }
                Event::Land { impact, gc, pos, up, fall_speed } => {
                    if impact {
                        if let Some(p) = self.script_prefab(nm, "impactDust") {
                            // Instantiate(impactDust, gc.position, identity).transform.forward = transform.up
                            self.fx_instantiate(p, to_unity(gc), look_rotation(Vec3::Y));
                        }
                        // CreateEnviroGibs(position, -up, 5, round(lerp(3, 5, (|fallSpeed| - 50) / 50)))
                        let t = ((fall_speed.abs() - 50.0) / 50.0).clamp(0.0, 1.0);
                        self.create_enviro_gibs(pos, -up, 5.0, round_half_even(3.0 + 2.0 * t), 1.0);
                    }
                    self.fx_destroy_tag(TAG_FALL);
                }
                _ => {}
            }
        }
        let p = &self.s.player;
        let (live, sliding, heavy) = (p.activated && !self.s.dead, p.sliding, p.gc.heavy_fall);
        let (vel, at, dodge, inv) = (p.vel, p.pos, p.dodge_direction, p.boost_left > 0.0 && p.invincible_layer);
        if !live || !heavy {
            self.fx_destroy_tag(TAG_FALL);
        }
        self.frictionless_slide();
        // Cling: CreateWallScrape(point + up, wallScrape == null), forward = the wall normal;
        // otherwise DetachWallScrape (OnDisable destroys it)
        match self.s.player.wall_scrape.filter(|_| !self.s.dead) {
            Some((at, normal)) => {
                let fresh = !self.fx.effects.iter().any(|e| e.tag == TAG_WALL_SCRAPE && !e.destroyed);
                self.create_wall_scrape(at, fresh);
                if let Some(e) = self.fx.effects.iter_mut().find(|e| e.tag == TAG_WALL_SCRAPE && !e.destroyed) {
                    e.rot = look_rotation(to_unity(normal));
                }
            }
            None if self.s.dead => self.fx_destroy_tag(TAG_WALL_SCRAPE),
            None => self.detach_scrape(TAG_WALL_SCRAPE),
        }
        if !live || !sliding {
            self.fx_destroy_tag(TAG_SLIDE);
            self.detach_scrape(TAG_SLIDE_SCRAPE);
            return;
        }
        // HandleSlideState: the scrape at position + the velocity direction along the floor
        // (facing back) while on the ground or a wall, else parked at (5000, 5000, 5000)
        let p = &self.s.player;
        let n = project_on_plane_n(normalized(p.vel), Vec3::Y);
        let (sp, sr) = if p.gc.on_ground || p.wall.on_wall { (to_unity(p.pos + n), Some(look_rotation(-to_unity(n)))) } else { (Vec3::splat(5000.0), None) };
        if let Some(e) = self.fx.effects.iter_mut().find(|e| e.tag == TAG_SLIDE_SCRAPE && !e.destroyed) {
            e.pos = sp;
            if let Some(r) = sr {
                e.rot = r;
            }
        }
        // HandleSlideState: position + flat velocity direction * 10, forward = -dodgeDirection
        let flat = project_on_plane_n(normalized(vel), Vec3::Y);
        let pos = to_unity(at + flat * 10.0);
        let rot = look_rotation(-to_unity(dodge));
        if let Some(i) = self.fx.effects.iter().position(|e| e.tag == TAG_SLIDE && !e.destroyed) {
            self.fx.effects[i].pos = pos;
            self.fx.effects[i].rot = rot;
            self.slide_gradient(i, inv);
        }
    }

    /// NewMovement.Update's frictionless slide particle. Grounded it needs a CustomGroundProperties
    /// with friction 0 (not ported: always destroyed); airborne and off the slope check, a ray
    /// 0.5 down from 0.1 above the ground check hitting Environment or layer 0 shows it when the
    /// collider is on layer 0 or tagged Slippery, keeps it on other hits, destroys it on none.
    fn frictionless_slide(&mut self) {
        let p = &self.s.player;
        let show = if p.gc.on_ground {
            false
        } else {
            let o = p.gc_pos() + Vec3::Y * 0.1;
            let def = &self.def;
            let hit = (!p.slope.on_ground).then(|| self.world.raycast_filtered(o, -Vec3::Y, 0.5, |c| {
                let ow = self.world.owner(c);
                ow != uk_core::collide::ALWAYS && matches!(def.colliders[ow as usize].layer, 0 | 6 | 7 | 8 | 24)
            })).flatten();
            match hit {
                None => false,
                Some(h) => {
                    let ow = self.world.owner(h.collider);
                    let node = &def.nodes[def.colliders[ow as usize].node as usize];
                    if !(node.layer == 0 || node.tag == uk_assets::scenedef::tags::SLIPPERY) {
                        return;
                    }
                    true
                }
            }
        };
        if !show || self.s.dead {
            self.fx_destroy_tag(TAG_FRIC_SLIDE);
            return;
        }
        // FrictionlessSlideParticle: CreateSlideScrape(none yet, frictionless), then scale by the
        // flat speed (up to 35), at the player, facing the flat velocity
        let fresh = !self.fx.effects.iter().any(|e| e.tag == TAG_FRIC_SLIDE && !e.destroyed);
        let q = self.s.player.scrape_query();
        let dir = self.s.player.dodge_direction;
        self.create_slide_scrape(fresh, &q, dir, true);
        let p = &self.s.player;
        let flat = to_unity(Vec3::new(p.vel.x, 0.0, p.vel.z));
        let k = flat.length().min(35.0) / 35.0;
        let pos = to_unity(p.pos);
        if let Some(e) = self.fx.effects.iter_mut().find(|e| e.tag == TAG_FRIC_SLIDE && !e.destroyed) {
            e.scale = Vec3::splat(0.25 * k);
            e.pos = pos;
            e.rot = look_rotation(normalized(flat));
        }
    }

    /// NewMovement's FixedUpdate events: CreateSlideScrape while sliding on the ground or a wall.
    pub(crate) fn fx_player_fixed(&mut self, ev0: usize) {
        use uk_core::player::Event;
        if self.fx.nm.is_none() {
            return;
        }
        let evs: Vec<Event> = self.s.player.events.get(ev0..).unwrap_or_default().to_vec();
        for ev in evs {
            if let Event::SlideScrape(q) = ev {
                let dir = self.s.player.dodge_direction;
                self.create_slide_scrape(false, &q, dir, false);
            }
        }
    }

    /// NewMovement.CreateSlideScrape (not the frictionless version): the surface under the player
    /// (or, airborne, at the wall check's point of contact) picks the slide particle; a new
    /// one (the old one detached) at position + dodgeDirection * 2 facing back when the type
    /// changed or `ignore_previous`, tinted by the surface color.
    fn create_slide_scrape(&mut self, ignore_previous: bool, q: &uk_core::player::ScrapeQuery, dodge: Vec3, fric: bool) {
        let hit = match q.wall {
            Some((wc, poc)) if !q.on_ground => self.surface_data(wc, normalized(poc - wc), 3.0),
            _ => self.surface_data(q.pos, q.grav, 3.0),
        };
        let ff = &self.def.footstep_fx;
        let (surface, color) = match hit {
            Some(h) => (if ff.slide_particles.get(&h.surface).copied().flatten().is_some() { h.surface } else { 0 }, Some(h.color)),
            None => (0, None),
        };
        let cur = if fric { self.fx.fric_surface } else { self.fx.slide_surface };
        if hit.is_some() && !ignore_previous && surface == cur {
            return;
        }
        if hit.is_none() && !(cur != 0 || ignore_previous) {
            return;
        }
        if fric {
            // the frictionless version: the old one destroyed, the new one at the player
            // (FrictionlessSlideParticle places it), untinted
            self.fx_destroy_tag(TAG_FRIC_SLIDE);
            self.fx.fric_surface = surface;
            let Some(p) = self.def.footstep_fx.slide_particle(surface) else { return };
            let i = self.fx_instantiate(p, to_unity(q.pos), Quat::IDENTITY);
            self.fx.effects[i].tag = TAG_FRIC_SLIDE;
            return;
        }
        self.detach_scrape(TAG_SLIDE_SCRAPE);
        self.fx.slide_surface = surface;
        let Some(p) = self.def.footstep_fx.slide_particle(surface) else { return };
        let d = to_unity(dodge);
        let i = self.fx_instantiate(p, to_unity(q.pos) + d * 2.0, look_rotation(-d));
        self.fx.effects[i].tag = TAG_SLIDE_SCRAPE;
        if let Some(c) = color {
            self.set_particles_colors(i, c);
        }
    }

    /// NewMovement.CreateWallScrape: the surface toward `at` picks the wall scrape particle; a
    /// new one (the old one detached) when the type changed or `ignore_previous`, else the old
    /// one moves to `at`.
    fn create_wall_scrape(&mut self, at: Vec3, ignore_previous: bool) {
        let pos = self.s.player.pos;
        let hit = self.surface_data(pos, at - pos, 5.0);
        let ff = &self.def.footstep_fx;
        let surface = hit.map_or(0, |h| if ff.wall_scrape_particles.get(&h.surface).copied().flatten().is_some() { h.surface } else { 0 });
        let fresh = match hit {
            Some(_) => ignore_previous || self.fx.scrape_surface != surface,
            None => self.fx.scrape_surface != 0 || ignore_previous,
        };
        if !fresh {
            if let Some(e) = self.fx.effects.iter_mut().find(|e| e.tag == TAG_WALL_SCRAPE && !e.destroyed) {
                e.pos = to_unity(at);
            }
            return;
        }
        self.detach_scrape(TAG_WALL_SCRAPE);
        self.fx.scrape_surface = surface;
        let Some(p) = self.def.footstep_fx.wall_scrape_particle(surface) else { return };
        let i = self.fx_instantiate(p, to_unity(at), Quat::IDENTITY);
        self.fx.effects[i].tag = TAG_WALL_SCRAPE;
        if let Some(h) = hit {
            self.set_particles_colors(i, h.color);
        }
    }

    /// NewMovement.DetachScrape: every system stops emitting and RemoveOnTime(10) is added; the
    /// script lets go of it.
    fn detach_scrape(&mut self, tag: u8) {
        let now = self.s.time;
        for e in self.fx.effects.iter_mut().filter(|e| e.tag == tag && !e.destroyed) {
            for s in &mut e.systems {
                s.stop();
            }
            e.remove_at = Some(e.remove_at.map_or(now + 10.0, |t| t.min(now + 10.0)));
            e.tag = 0;
        }
    }

    /// SceneHelper.SetParticlesColors: a non-white color becomes the startColor (keeping its
    /// alpha) of each active EnviroGibModifier's particleSystem.
    fn set_particles_colors(&mut self, i: usize, c: [f32; 4]) {
        if c == [1.0; 4] {
            return;
        }
        let pf = self.def.particle_prefabs[self.fx.effects[i].prefab as usize].clone();
        let e = &mut self.fx.effects[i];
        for n in (0..pf.nodes.len()).filter(|&n| pf.is_active_in_hierarchy(n as u32)) {
            for m in pf.nodes[n].scripts.iter().filter(|s| s.class == "EnviroGibModifier") {
                let Some(&k) = pf.obj_node.get(&m.data.get("particleSystem").pptr().1) else { continue };
                for s in e.systems.iter_mut().filter(|s| s.node == k) {
                    let d = Arc::make_mut(&mut s.def);
                    let a = d.start_color.max_color[3];
                    d.start_color = MinMaxGradient { mode: 0, max_color: [c[0], c[1], c[2], a], ..d.start_color.clone() };
                }
            }
        }
    }

    /// slideTrail.colorOverLifetime = invincible ? invincibleSlideGradient : normalSlideGradient
    fn slide_gradient(&mut self, i: usize, invincible: bool) {
        let Some((normal, inv)) = &self.fx.slide_gradients else { return };
        let g = if invincible { inv } else { normal };
        for s in &mut self.fx.effects[i].systems {
            if s.def.trail.as_ref().is_some_and(|t| t.color_over_lifetime != *g) {
                std::sync::Arc::make_mut(&mut s.def).trail.as_mut().unwrap().color_over_lifetime = g.clone();
            }
        }
    }

    /// A scene system's activation change since the last look: disabling stops and clears it,
    /// enabling plays it when playOnAwake. Returns activeInHierarchy.
    fn scene_sync(&mut self, k: usize) -> bool {
        let node = self.fx.scene[k].sys.node;
        let act = self.s.active[node as usize];
        let Fx { scene, rng, .. } = &mut self.fx;
        let ss = &mut scene[k];
        if act != ss.on {
            ss.on = act;
            ss.sys.stop();
            ss.sys.clear();
            if act && ss.sys.def.play_on_awake {
                ss.sys.play(rng);
            }
        }
        act
    }

    /// UnityEvent calls on a scene ParticleSystem: Play / Stop (StopEmitting) / Clear, each
    /// with withChildren (the systems on the GameObject's descendants too). Systems on inactive
    /// GameObjects are left alone.
    pub(crate) fn scene_particle_call(&mut self, ps: u32, method: &str) {
        let root = self.def.particle_systems[ps as usize].node;
        for k in 0..self.fx.scene.len() {
            let n = self.fx.scene[k].sys.node;
            let mut up = Some(n);
            while up.is_some_and(|u| u != root) {
                up = self.def.nodes[up.unwrap() as usize].parent;
            }
            if up.is_none() || !self.scene_sync(k) {
                continue;
            }
            let Fx { scene, rng, .. } = &mut self.fx;
            match method {
                "Play" => scene[k].sys.play(rng),
                "Stop" => scene[k].sys.stop(),
                _ => scene[k].sys.clear(),
            }
        }
    }

    /// The scene systems: activation changes since the last update, then the simulation.
    fn fx_scene_update(&mut self, dt: f32) {
        let mut xf = Vec::with_capacity(self.fx.scene.len());
        for k in 0..self.fx.scene.len() {
            let act = self.scene_sync(k);
            xf.push(act.then(|| self.node_world_now(self.fx.scene[k].sys.node)));
        }
        // NewMovement.Update: windStateParticle.position = cc.GetDefaultPos() + velocity.normalized,
        // LookAt(GetDefaultPos()), rateOverDistanceMultiplier = min(windState, 0.5) * 10
        if let Some(k) = self.fx.wind.filter(|&k| xf[k].is_some()) {
            let p = &self.s.player;
            let eye = to_unity(p.pos + p.cam_default_pos);
            let v = normalized(to_unity(p.vel));
            let at = eye + v;
            let rot = if v == Vec3::ZERO { Quat::IDENTITY } else { look_rotation(-v) };
            let scale = self.def.nodes[self.fx.scene[k].sys.node as usize].local_scale;
            xf[k] = Some(Mat4::from_scale_rotation_translation(scale, rot, at));
            self.fx.scene[k].sys.rate_over_distance_mul = Some(p.wind_state.min(0.5) * 10.0);
        }
        let ray = |o: Vec3, d: Vec3, len: f32, mask: u32| -> Option<(f32, Vec3)> {
            let h = self.world.raycast_filtered(to_bevy(o), to_bevy(d), len, |id| {
                let o = self.world.owner(id);
                o == uk_core::collide::ALWAYS || self.def.colliders.get(o as usize).is_some_and(|c| !c.trigger && mask & (1 << c.layer) != 0)
            })?;
            Some((h.distance, to_unity(h.normal)))
        };
        let env = Env { ray: Some(&ray) };
        let mut extra: Vec<(Option<Mat4>, Vec<(Vec3, Vec3)>)> = Vec::with_capacity(xf.len());
        for (ss, w) in self.fx.scene.iter().zip(&xf) {
            let p = &self.def.particle_systems[ss.index as usize];
            let shape = match (w, p.shape_node) {
                (Some(_), Some(n)) => Some(self.node_world_now(n)),
                _ => None,
            };
            let planes = match w {
                Some(_) if ss.sys.def.collision.as_ref().is_some_and(|c| c.kind == 0) => p
                    .planes
                    .iter()
                    .map(|&n| {
                        let m = self.node_world_now(n);
                        (m.w_axis.truncate(), m.y_axis.truncate().normalize_or(Vec3::Y))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            extra.push((shape, planes));
        }
        let world = &self.world;
        let def = &self.def;
        let solid = |id| {
            let o = world.owner(id);
            o != uk_core::collide::ALWAYS && def.colliders.get(o as usize).is_some_and(|c| !c.trigger && STAIN_LAYERS.contains(&c.layer))
        };
        let Fx { scene, rng, .. } = &mut self.fx;
        for ((ss, w), (shape, planes)) in scene.iter_mut().zip(xf).zip(extra) {
            let Some(w) = w else { continue };
            let s = &mut ss.sys;
            if !s.playing && s.particles.is_empty() {
                continue;
            }
            s.set_transform(w, def.nodes[s.node as usize].local_scale);
            s.shape_xf = shape.map(|m| s.emitter.inverse() * m);
            s.planes = planes;
            step_system(s, dt, rng, &env);
            if ss.portal_aware {
                portal_kill(s, world, &solid);
            }
        }
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
        let c = self.fx.spawns.entry(prefab).or_insert((0, pos, rot));
        *c = (c.0 + 1, pos, rot);
        e.splatter = splatter_of(&pf);
        if let Some(r) = pf.script(0, "RemoveOnTime") {
            let t = r.data.get("time").f32() + (self.fx.rng.f() * 2.0 - 1.0) * r.data.get("randomizer").f32();
            e.remove_at = Some(self.s.time + t as f64);
        }
        // child RemoveOnTime: Start runs on the instantiated (active) objects
        for n in 1..pf.nodes.len() as u32 {
            if let Some(r) = pf.script(n, "RemoveOnTime").filter(|_| pf.is_active_in_hierarchy(n)) {
                let t = r.data.get("time").f32() + (self.fx.rng.f() * 2.0 - 1.0) * r.data.get("randomizer").f32();
                e.child_remove.push((n, self.s.time + t as f64));
            }
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

    /// CheckPoint.OnRespawn: every active DestroyOnCheckpointRestart (without dontDestroy) on
    /// an effect destroys its GameObject.
    pub(crate) fn fx_checkpoint_restart(&mut self) {
        for e in self.fx.effects.iter_mut().filter(|e| e.active && !e.destroyed) {
            let pf = &self.def.particle_prefabs[e.prefab as usize];
            for n in 0..pf.nodes.len() as u32 {
                let Some(d) = pf.script(n, "DestroyOnCheckpointRestart") else { continue };
                if d.data.get("dontDestroy").bool() || !node_active_up(pf, &e.node_active, n) {
                    continue;
                }
                if n == 0 {
                    e.destroyed = true;
                    break;
                }
                e.node_active[n as usize] = false;
                for s in e.systems.iter_mut().filter(|s| pf.is_descendant_of(s.node, n)) {
                    s.stop();
                    s.clear();
                }
            }
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
            if e.child_remove.iter().any(|c| c.1 <= now) {
                let pf = &self.def.particle_prefabs[e.prefab as usize];
                for (n, _) in e.child_remove.iter().filter(|c| c.1 <= now) {
                    e.node_active[*n as usize] = false;
                    for s in e.systems.iter_mut().filter(|s| pf.is_descendant_of(s.node, *n)) {
                        s.stop();
                        s.clear();
                    }
                }
                e.child_remove.retain(|c| c.1 > now);
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
                Some((n, local)) if n == ATTACH_PLAYER => self.player_world() * local,
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
                e.world[n] = e.world[p] * local_unity(&pf, n) * Mat4::from_scale(Vec3::splat(e.node_scale[n]));
            }
            if let Some((_, _)) = e.attach {
                let (_, r, t) = root.to_scale_rotation_translation();
                e.pos = t;
                e.rot = r;
            }
        }
        self.fx_scene_update(dt);
        // simulation
        let world = std::mem::take(&mut self.world);
        let def = self.def.clone();
        let solid = |id| {
            let o = world.owner(id);
            o != uk_core::collide::ALWAYS && def.colliders.get(o as usize).is_some_and(|c| !c.trigger && STAIN_LAYERS.contains(&c.layer))
        };
        let ray = |o: Vec3, d: Vec3, len: f32, mask: u32| -> Option<(f32, Vec3)> {
            let h = world.raycast_filtered(to_bevy(o), to_bevy(d), len, |id| {
                let o = world.owner(id);
                o == uk_core::collide::ALWAYS || def.colliders.get(o as usize).is_some_and(|c| !c.trigger && mask & (1 << c.layer) != 0)
            })?;
            Some((h.distance, to_unity(h.normal)))
        };
        let env = Env { ray: Some(&ray) };
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
            let mut stop_actions = Vec::new();
            for s in &mut e.systems {
                if !e.node_active[s.node as usize] || !node_active_up(pf, &e.node_active, s.node) {
                    continue;
                }
                let n = s.node as usize;
                let local_scale = if n == 0 && e.attach.is_none() { e.scale } else { pf.nodes[n].local_scale * e.node_scale[n] };
                s.set_transform(e.world[n], local_scale);
                s.shape_xf = pf.nodes[s.node as usize].shape_node.map(|n| s.emitter.inverse() * e.world[n as usize]);
                let was = s.stopped();
                step_system(s, dt, rng, &env);
                let hits = portal_kill(s, &world, &solid);
                if stain_root && s.node == 0 {
                    *stains += hits as u64;
                }
                if !was && s.stopped() {
                    stop_actions.push((s.node, s.def.stop_action));
                }
            }
            // MainModule.stopAction once the system has stopped: Disable deactivates its
            // GameObject, Destroy destroys it (the whole instance for the root)
            for (n, a) in stop_actions {
                match a {
                    1 => e.node_active[n as usize] = false,
                    2 if n == 0 => e.destroyed = true,
                    2 => {
                        e.node_active[n as usize] = false;
                        if let Some(s) = e.systems.iter_mut().find(|s| s.node == n) {
                            s.clear();
                        }
                    }
                    _ => {}
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

/// PortalAwareParticleSystem: a particle whose last move crossed the environment dies (on the next
/// update). Returns how many were hit.
fn portal_kill(s: &mut System, world: &uk_core::collide::World, solid: &dyn Fn(uk_core::collide::ColliderId) -> bool) -> usize {
    let m = s.sim_to_world;
    let mut hits = 0;
    for p in &mut s.particles {
        let now_w = m.transform_point3(p.pos);
        if !p.kill {
            let d = now_w - p.prev_world;
            let len = d.length();
            if len > 1e-6 && world.raycast_filtered(to_bevy(p.prev_world), to_bevy(d / len), len, solid).is_some() {
                p.kill = true;
                hits += 1;
            }
        }
        p.prev_world = now_w;
    }
    hits
}

fn seg_dist(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

/// Simulation space -> world for a system on a node with world matrix `w`: position and rotation
/// of the node, scaled per scalingMode (0 hierarchy: lossy scale, 1 local: the node's own
/// localScale, 2 shape: none).
fn sim_matrix(w: Mat4, local_scale: Vec3, def: &ParticleSystemDef) -> Mat4 {
    let axes = [w.x_axis.truncate(), w.y_axis.truncate(), w.z_axis.truncate()];
    let lossy = Vec3::new(axes[0].length(), axes[1].length(), axes[2].length());
    // the rotation from whichever axes still have length (a zero-scaled node keeps the
    // orientation it still defines)
    let [x, y, z] = axes.map(|a| a.normalize_or_zero());
    let (x, y) = match (x != Vec3::ZERO, y != Vec3::ZERO, z != Vec3::ZERO) {
        (true, true, _) => (x, y.reject_from_normalized(x).normalize_or(x.any_orthonormal_vector())),
        (true, false, true) => (x, z.cross(x).normalize_or(x.any_orthonormal_vector())),
        (false, true, true) => (y.cross(z).normalize_or(y.any_orthonormal_vector()), y),
        (true, false, false) => (x, x.any_orthonormal_vector()),
        (false, true, false) => (y.any_orthonormal_vector(), y),
        (false, false, true) => (z.any_orthonormal_vector(), z.cross(z.any_orthonormal_vector())),
        (false, false, false) => (Vec3::X, Vec3::Y),
    };
    let r = Quat::from_mat3(&Mat3::from_cols(x, y, x.cross(y)));
    let mut s = match def.scaling_mode {
        0 => lossy,
        1 => local_scale,
        _ => Vec3::ONE,
    };
    // a mirrored hierarchy
    if def.scaling_mode == 0 && w.determinant() < 0.0 {
        s.z = -s.z;
    }
    // just off zero so the matrix stays invertible (the particles shrink to nothing as in Unity)
    let s = Vec3::select(s.abs().cmplt(Vec3::splat(1e-6)), Vec3::splat(1e-6), s);
    Mat4::from_scale_rotation_translation(s, r, w.w_axis.truncate())
}

/// What the simulation needs from the world: CollisionModule (world) raycasts in Unity space,
/// `(origin, unit direction, length, layer mask) -> (distance, normal)`.
pub struct Env<'a> {
    pub ray: Option<&'a dyn Fn(Vec3, Vec3, f32, u32) -> Option<(f32, Vec3)>>,
}

impl Env<'_> {
    pub const NONE: Env<'static> = Env { ray: None };
}

/// When and how a particle is emitted: normalized system time, seconds since Play, and its index
/// within a burst (burst-spread shape modes).
#[derive(Clone, Copy)]
struct Emit {
    tn: f32,
    elapsed: f32,
    burst: Option<(u32, u32)>,
}

/// Arc / radius / mesh-spawn position in [0, 1): random, loop, ping-pong or burst spread, snapped
/// to `spread`.
fn spawn_param(mode: u8, spread: f32, speed: &uk_assets::particles::MinMaxCurve, e: &Emit, rng: &mut Rng) -> f32 {
    let x = |rng: &mut Rng| e.elapsed * speed.eval(e.tn, rng.f());
    let u = match mode {
        1 => x(rng).rem_euclid(1.0),
        2 => {
            let k = x(rng).rem_euclid(2.0);
            if k > 1.0 { 2.0 - k } else { k }
        }
        3 => match e.burst {
            Some((i, n)) => i as f32 / n.max(1) as f32,
            None => rng.f(),
        },
        _ => rng.f(),
    };
    if spread > 0.0 && mode != 0 {
        (u / spread).floor() * spread
    } else if spread > 0.0 {
        ((u / spread).floor() * spread).min(1.0)
    } else {
        u
    }
}

/// A radius fraction for `thickness` (1 fills the shape, 0 is its edge) in `dims` dimensions.
fn thick(rng: &mut Rng, thickness: f32, dims: i32) -> f32 {
    let inner = (1.0 - thickness.clamp(0.0, 1.0)).powi(dims);
    (inner + (1.0 - inner) * rng.f()).powf(1.0 / dims as f32)
}

/// Shape module: (position, direction) in emitter space.
fn emit_shape(s: &System, e: &Emit, rng: &mut Rng) -> (Vec3, Vec3) {
    let def = &s.def;
    let Some(sh) = &def.shape else { return (Vec3::ZERO, Vec3::Z) };
    let arc = |rng: &mut Rng| spawn_param(sh.arc_mode, sh.arc_spread, &sh.arc_speed, e, rng) * sh.arc.to_radians();
    let (p, d) = match sh.kind {
        // Sphere (shell: the surface only)
        0 | 1 => {
            let u = rng.on_unit_sphere();
            let r = sh.radius * if sh.kind == 1 { 1.0 } else { thick(rng, sh.radius_thickness, 3) };
            (u * r, u)
        }
        // Hemisphere (+z)
        2 | 3 => {
            let mut u = rng.on_unit_sphere();
            u.z = u.z.abs();
            let r = sh.radius * if sh.kind == 3 { 1.0 } else { thick(rng, sh.radius_thickness, 3) };
            (u * r, u)
        }
        // Cone: 4 the base disk, 7 the base's rim, 8 the volume, 9 the volume's surface; heading
        // along the cone's side
        4 | 7 | 8 | 9 => {
            let th = arc(rng);
            let rn = if sh.kind == 7 || sh.kind == 9 { 1.0 } else { thick(rng, sh.radius_thickness, 2) };
            let dir2 = Vec3::new(th.cos(), th.sin(), 0.0);
            let tan = sh.angle.to_radians().tan();
            let d = (dir2 * rn * tan + Vec3::Z).normalize();
            let base = dir2 * rn * sh.radius;
            let p = if sh.kind >= 8 { base + d * (sh.length * rng.f() / d.z.max(1e-6)) } else { base };
            (p, d)
        }
        // Box: the unit cube (sized by the shape scale) heading +z; 5 the volume (boxThickness
        // per axis: 0 fills it), 15 the faces, 16 the edges
        5 => {
            let mut p = Vec3::ZERO;
            for a in 0..3 {
                let t = sh.box_thickness[a].clamp(0.0, 1.0);
                let v = rng.f() - 0.5;
                p[a] = if t <= 0.0 || t >= 1.0 { v } else { v.signum() * (0.5 - t * 0.5 * rng.f()) };
            }
            (p, Vec3::Z)
        }
        15 => {
            let sc = sh.scale.abs();
            let areas = [sc.y * sc.z, sc.x * sc.z, sc.x * sc.y];
            let total: f32 = areas.iter().sum();
            let mut k = rng.f() * total.max(1e-9);
            let axis = (0..3).find(|&a| {
                k -= areas[a];
                k < 0.0
            });
            let axis = axis.unwrap_or(2);
            let mut p = Vec3::new(rng.f() - 0.5, rng.f() - 0.5, rng.f() - 0.5);
            p[axis] = if rng.f() < 0.5 { -0.5 } else { 0.5 };
            (p, Vec3::Z)
        }
        16 => {
            let sc = sh.scale.abs();
            let total = sc.x + sc.y + sc.z;
            let mut k = rng.f() * total.max(1e-9);
            let axis = (0..3).find(|&a| {
                k -= sc[a];
                k < 0.0
            });
            let axis = axis.unwrap_or(2);
            let mut p = Vec3::new(if rng.f() < 0.5 { -0.5 } else { 0.5 }, if rng.f() < 0.5 { -0.5 } else { 0.5 }, if rng.f() < 0.5 { -0.5 } else { 0.5 });
            p[axis] = rng.f() - 0.5;
            (p, Vec3::Z)
        }
        // Circle: the XY disk (11: its rim), heading outward
        10 | 11 => {
            let th = arc(rng);
            let rn = if sh.kind == 11 { 1.0 } else { thick(rng, sh.radius_thickness, 2) };
            let dir2 = Vec3::new(th.cos(), th.sin(), 0.0);
            (dir2 * rn * sh.radius, dir2)
        }
        // Single-sided edge: along x from -radius to radius, heading +y
        12 => {
            let u = spawn_param(sh.radius_mode, sh.radius_spread, &sh.radius_speed, e, rng);
            (Vec3::new((u * 2.0 - 1.0) * sh.radius, 0.0, 0.0), Vec3::Y)
        }
        // Donut: a torus around z (ring `radius`, tube `donutRadius`), heading out of the tube
        17 => {
            let th = arc(rng);
            let radial = Vec3::new(th.cos(), th.sin(), 0.0);
            let phi = rng.f() * std::f32::consts::TAU;
            let rr = sh.donut_radius * thick(rng, sh.radius_thickness, 2);
            let out = radial * phi.cos() + Vec3::Z * phi.sin();
            (radial * sh.radius + out * rr, out)
        }
        // Rectangle: the unit XY square, heading +z
        18 => (Vec3::new(rng.f() - 0.5, rng.f() - 0.5, 0.0), Vec3::Z),
        // Mesh / MeshRenderer: vertices, edges or triangles, heading along the normal
        6 | 13 => match &sh.mesh {
            Some(m) if !m.positions.is_empty() => {
                let (p, n) = emit_mesh(m, sh, e, rng);
                let n = n.normalize_or(Vec3::Z);
                let (p, n) = match (sh.kind, s.shape_xf) {
                    (13, Some(xf)) => (xf.transform_point3(p), xf.transform_vector3(n).normalize_or(Vec3::Z)),
                    _ => (p, n),
                };
                (p + n * sh.normal_offset, n)
            }
            _ => (Vec3::ZERO, Vec3::Z),
        },
        _ => (Vec3::ZERO, Vec3::Z),
    };
    let mut d = d;
    if sh.spherical_direction > 0.0 {
        d = d.lerp(p.normalize_or(d), sh.spherical_direction).normalize_or(d);
    }
    if sh.random_direction > 0.0 {
        d = d.lerp(rng.on_unit_sphere(), sh.random_direction).normalize_or(d);
    }
    let mut p = p;
    if sh.random_position > 0.0 {
        p += rng.on_unit_sphere() * sh.random_position * rng.f().cbrt();
    }
    let q = unity_euler(sh.rotation);
    let p = sh.position + q * (p * sh.scale);
    let d = (q * (d * sh.scale)).normalize_or(Vec3::Z);
    (p, d)
}

/// One shape-module sample (emitter space) at a random point of the cycle (probes).
pub fn shape_sample(s: &System, rng: &mut Rng) -> (Vec3, Vec3) {
    let e = Emit { tn: 0.0, elapsed: rng.f() * s.def.duration, burst: None };
    emit_shape(s, &e, rng)
}

/// A point on an emission mesh (mesh space) and its normal.
fn emit_mesh(m: &uk_assets::particles::EmitMesh, sh: &uk_assets::particles::Shape, e: &Emit, rng: &mut Rng) -> (Vec3, Vec3) {
    let pick = |rng: &mut Rng, n: usize| ((spawn_param(sh.mesh_spawn_mode, sh.mesh_spawn_spread, &sh.mesh_spawn_speed, e, rng) * n as f32) as usize).min(n.saturating_sub(1));
    let tri = |rng: &mut Rng| -> usize {
        let total = m.area_cdf.last().copied().unwrap_or(0.0);
        if total <= 0.0 {
            return (rng.f() * m.tris.len() as f32) as usize % m.tris.len().max(1);
        }
        let k = rng.f() * total;
        m.area_cdf.partition_point(|&c| c <= k).min(m.tris.len() - 1)
    };
    match sh.placement {
        // edge: a random point on a random triangle edge
        1 if !m.tris.is_empty() => {
            let t = m.tris[pick(rng, m.tris.len())];
            let k = (rng.f() * 3.0) as usize % 3;
            let (a, b) = (t[k] as usize, t[(k + 1) % 3] as usize);
            let f = rng.f();
            (m.positions[a].lerp(m.positions[b], f), m.normals[a].lerp(m.normals[b], f))
        }
        // triangle: area weighted, uniform within it
        2 if !m.tris.is_empty() => {
            let t = m.tris[tri(rng)];
            let (mut u, mut v) = (rng.f(), rng.f());
            if u + v > 1.0 {
                u = 1.0 - u;
                v = 1.0 - v;
            }
            let w = 1.0 - u - v;
            let [a, b, c] = t.map(|i| i as usize);
            (m.positions[a] * w + m.positions[b] * u + m.positions[c] * v, m.normals[a] * w + m.normals[b] * u + m.normals[c] * v)
        }
        // vertex
        _ => {
            let i = pick(rng, m.positions.len());
            (m.positions[i], m.normals[i])
        }
    }
}

fn spawn(s: &System, e: &Emit, rng: &mut Rng) -> Particle {
    let def = &s.def;
    let tn = e.tn;
    let mut lifetime = def.start_lifetime.eval(tn, rng.f()).max(1e-4);
    let speed = def.start_speed.eval(tn, rng.f());
    let sx = def.start_size.eval(tn, rng.f());
    let size = if def.size3d { Vec3::new(sx, def.start_size_y.eval(tn, rng.f()), def.start_size_z.eval(tn, rng.f())) } else { Vec3::splat(sx) };
    let rz = def.start_rotation.eval(tn, rng.f());
    let mut rot = if def.rotation3d { Vec3::new(def.start_rotation_x.eval(tn, rng.f()), def.start_rotation_y.eval(tn, rng.f()), rz) } else { Vec3::new(0.0, 0.0, rz) };
    // flipRotation: this proportion of particles turn the other way
    let rot_sign = if rng.f() < def.randomize_rotation { -1.0 } else { 1.0 };
    rot *= rot_sign;
    let color = def.start_color.eval(tn, rng.f());
    let gravity = def.gravity_modifier.eval(tn, rng.f());
    let (mut pos, dir) = emit_shape(s, e, rng);
    let mut vel = dir * speed;
    if def.simulation_space == 1 {
        pos = s.emitter.transform_point3(pos);
        vel = s.emitter.transform_vector3(vel);
    }
    let rand = std::array::from_fn(|_| rng.f());
    // InheritVelocity (world simulation space only): initial mode adds the emitter's velocity
    if let (Some(iv), 1) = (&def.inherit_velocity, def.simulation_space) {
        if iv.mode == 0 {
            vel += s.emitter_vel * iv.curve.eval(0.0, rand[4]);
        }
    }
    if let Some(ls) = &def.lifetime_by_emitter_speed {
        lifetime = (lifetime * ls.x.eval(speed_frac(s.emitter_vel.length(), ls.range), rand[5])).max(1e-4);
    }
    let (sheet_start, sheet_row, flip) = match &def.texture_sheet {
        Some(ts) => {
            let start = ts.start_frame.eval(tn, rng.f());
            let row = match ts.row_mode {
                1 => (rng.f() * ts.tiles_y.max(1) as f32) as u32,
                _ => ts.row_index,
            };
            (start, row, [rng.f() < ts.flip_u, rng.f() < ts.flip_v])
        }
        None => (0.0, 0, [false; 2]),
    };
    let trail = def.trail.as_ref().and_then(|t| {
        (rng.f() < t.ratio).then(|| TrailState { points: vec![(pos, 0.0)], vertex_life: t.lifetime.eval(tn, rng.f()) * lifetime })
    });
    Particle {
        pos,
        vel,
        total_vel: vel,
        age: 0.0,
        lifetime,
        start_size: size,
        start_color: color,
        rot,
        rot_sign,
        gravity,
        rand,
        sheet_start,
        sheet_row,
        flip,
        mesh_pick: rng.f(),
        noise_size: 1.0,
        trail,
        prev_world: s.sim_to_world.transform_point3(pos),
        kill: false,
    }
}

/// `v` as a fraction of a speed range (the x axis of the by-speed curves).
fn speed_frac(v: f32, range: [f32; 2]) -> f32 {
    if range[1] > range[0] { ((v - range[0]) / (range[1] - range[0])).clamp(0.0, 1.0) } else { 0.0 }
}

/// Per-update values shared by every particle of a system.
struct Ctx<'a> {
    def: &'a ParticleSystemDef,
    /// gravity in simulation space
    g: Vec3,
    /// emitter (local) directions -> simulation space
    local_to_sim: Quat,
    /// world directions -> simulation space
    world_to_sim: Quat,
    /// the emitter's origin in simulation space
    center: Vec3,
    /// emitter velocity (world), when InheritVelocity current applies
    inherit: Option<Vec3>,
    noise_scroll: f32,
    sim_to_world: Mat4,
    size_scale: f32,
    /// collision planes (world point, unit normal)
    planes: &'a [(Vec3, Vec3)],
    env: &'a Env<'a>,
    hits: std::cell::Cell<u64>,
}

fn step_particle(p: &mut Particle, c: &Ctx, dt: f32, rng: &mut Rng) -> bool {
    let def = c.def;
    p.age += dt;
    if p.age >= p.lifetime {
        return false;
    }
    let t = p.age / p.lifetime;
    let to_sim = |world: bool| if world { c.world_to_sim } else { c.local_to_sim };
    // forces on the particle's own velocity: gravity, ForceOverLifetime, drag / limit
    p.vel += c.g * p.gravity * dt;
    if let Some(f) = &def.force {
        let r = if f.randomize_per_frame { rng.f() } else { p.rand[4] };
        p.vel += to_sim(f.world_space) * Vec3::new(f.x.eval(t, r), f.y.eval(t, r), f.z.eval(t, r)) * dt;
    }
    if let Some(l) = &def.limit_velocity {
        let r = p.rand[5];
        // dampen is the fraction of the excess removed per (30 Hz) frame
        let keep = (1.0 - l.dampen.clamp(0.0, 1.0)).powf(dt * 30.0);
        if l.separate_axes {
            let q = to_sim(l.world_space);
            let mut v = q.inverse() * p.vel;
            let lim = [l.x.eval(t, r), l.y.eval(t, r), l.z.eval(t, r)];
            for a in 0..3 {
                let m = lim[a].abs();
                if v[a].abs() > m {
                    v[a] = v[a].signum() * (m + (v[a].abs() - m) * keep);
                }
            }
            p.vel = q * v;
        } else {
            let m = l.magnitude.eval(t, r).abs();
            let s = p.vel.length();
            if s > m {
                p.vel *= (m + (s - m) * keep) / s;
            }
        }
        let mut k = l.drag.eval(t, r);
        if k > 0.0 {
            if l.drag_by_size {
                k *= p.start_size.x * c.size_scale;
            }
            if l.drag_by_velocity {
                k *= p.vel.length();
            }
            p.vel *= (1.0 - k * dt).max(0.0);
        }
    }
    let mut v = p.vel;
    if let Some(vol) = &def.velocity_over_lifetime {
        let r = p.rand[2];
        v += to_sim(vol.world_space) * Vec3::new(vol.x.eval(t, r), vol.y.eval(t, r), vol.z.eval(t, r));
        v *= vol.speed_modifier.eval(t, r);
    }
    let old = p.pos;
    p.pos += v * dt;
    if let Some(vol) = &def.velocity_over_lifetime {
        // orbital (radians per second about the emitter's axes) and radial speed, around the
        // emitter's origin plus the offset
        let r = p.rand[2];
        let off = c.local_to_sim * Vec3::new(vol.orbital_offset[0].eval(t, r), vol.orbital_offset[1].eval(t, r), vol.orbital_offset[2].eval(t, r));
        let center = c.center + off;
        let w = Vec3::new(vol.orbital[0].eval(t, r), vol.orbital[1].eval(t, r), vol.orbital[2].eval(t, r));
        if w != Vec3::ZERO {
            let axis = c.local_to_sim * w;
            let rel = p.pos - center;
            p.pos = center + Quat::from_scaled_axis(axis * dt) * rel;
        }
        let rad = vol.radial.eval(t, r);
        if rad != 0.0 {
            p.pos += (p.pos - center).normalize_or_zero() * rad * dt;
        }
    }
    if let Some(e) = c.inherit {
        if let Some(iv) = &def.inherit_velocity {
            p.pos += e * iv.curve.eval(t, p.rand[4]) * dt;
        }
    }
    if let Some(nz) = &def.noise {
        let r = p.rand[6];
        let n = noise_sample(nz, p.pos, c.noise_scroll, t, r);
        p.pos += n * nz.position_amount.eval(t, r) * dt;
        p.rot.z += n.z * nz.rotation_amount.eval(t, r) * dt * p.rot_sign;
        p.noise_size = (1.0 + n.y * nz.size_amount.eval(t, r)).max(0.0);
    }
    p.total_vel = if dt > 0.0 { (p.pos - old) / dt } else { v };
    // rotation: over lifetime, by speed
    let mut w = Vec3::ZERO;
    if let Some(r) = &def.rotation_over_lifetime {
        w.z += r.eval(t, p.rand[3]);
        if let Some([x, y]) = &def.rotation_axes {
            w.x += x.eval(t, p.rand[3]);
            w.y += y.eval(t, p.rand[3]);
        }
    }
    if let Some(rs) = &def.rotation_by_speed {
        let f = speed_frac(p.total_vel.length(), rs.range);
        w.z += rs.z.eval(f, p.rand[3]);
        if rs.separate_axes {
            w.x += rs.x.eval(f, p.rand[3]);
            w.y += rs.y.eval(f, p.rand[3]);
        }
    }
    p.rot += w * p.rot_sign * dt;
    if let Some(col) = &def.collision {
        if collide(p, old, col, c) {
            return false;
        }
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

/// CollisionModule (3D): the particle's move this update against the world (colliders on
/// `collidesWith`) or the planes; it is pushed back out by its radius and bounces. Returns true
/// when the particle dies.
fn collide(p: &mut Particle, old: Vec3, col: &uk_assets::particles::Collision, c: &Ctx) -> bool {
    let m = c.sim_to_world;
    let a = m.transform_point3(old);
    let b = m.transform_point3(p.pos);
    let d = b - a;
    let len = d.length();
    if len < 1e-6 {
        return false;
    }
    let dir = d / len;
    let radius = p.start_size.x * c.size_scale * 0.5 * col.radius_scale;
    let hit = if col.kind == 1 {
        c.env.ray.and_then(|ray| ray(a, dir, len + radius, col.collides_with))
    } else {
        c.planes
            .iter()
            .filter_map(|&(q, n)| {
                let (da, db) = ((a - q).dot(n) - radius, (b - q).dot(n) - radius);
                (da >= 0.0 && db < 0.0).then(|| (len * da / (da - db) + radius, n))
            })
            .min_by(|x, y| x.0.total_cmp(&y.0))
    };
    let Some((dist, n)) = hit else { return false };
    c.hits.set(c.hits.get() + 1);
    let t = p.age / p.lifetime;
    let r = p.rand[7];
    let at = a + dir * (dist - radius).max(0.0);
    let inv = m.inverse();
    p.pos = inv.transform_point3(at);
    let vw = m.transform_vector3(p.vel);
    let vn = n * vw.dot(n);
    let vt = vw - vn;
    let vw = (vt - vn * col.bounce.eval(t, r)) * (1.0 - col.dampen.eval(t, r));
    p.vel = inv.transform_vector3(vw);
    p.age += col.lifetime_loss.eval(t, r) * p.lifetime;
    let speed = vw.length();
    p.age >= p.lifetime || speed < col.min_kill_speed || speed > col.max_kill_speed
}

// Gradient noise for the NoiseModule (Unity's own noise function is not public: this is Perlin
// noise with the module's frequency, octaves, scroll, damping and remap applied).
fn hash3(i: i32, j: i32, k: i32) -> u32 {
    let mut h = (i as u32).wrapping_mul(0x8da6_b343) ^ (j as u32).wrapping_mul(0xd816_3841) ^ (k as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

fn grad3(h: u32, x: f32, y: f32, z: f32) -> f32 {
    let h = h & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// Perlin noise, about [-1, 1].
pub fn perlin3(p: Vec3) -> f32 {
    let f = p.floor();
    let (i, j, k) = (f.x as i32, f.y as i32, f.z as i32);
    let r = p - f;
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (u, v, w) = (fade(r.x), fade(r.y), fade(r.z));
    let g = |di: i32, dj: i32, dk: i32| grad3(hash3(i + di, j + dj, k + dk), r.x - di as f32, r.y - dj as f32, r.z - dk as f32);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(g(0, 0, 0), g(1, 0, 0), u);
    let x10 = lerp(g(0, 1, 0), g(1, 1, 0), u);
    let x01 = lerp(g(0, 0, 1), g(1, 0, 1), u);
    let x11 = lerp(g(0, 1, 1), g(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// The noise field's value (a velocity) at `pos` (simulation space).
fn noise_sample(nz: &uk_assets::particles::Noise, pos: Vec3, scroll: f32, t: f32, r: f32) -> Vec3 {
    let mut sum = Vec3::ZERO;
    let mut amp = 1.0;
    let mut freq = nz.frequency;
    for _ in 0..nz.octaves.max(1) {
        let q = pos * freq + Vec3::new(0.0, 0.0, scroll);
        sum += Vec3::new(perlin3(q), perlin3(q + Vec3::new(31.416, 17.32, 5.91)), perlin3(q + Vec3::new(-9.13, 47.2, 23.37))) * amp;
        amp *= nz.octave_multiplier;
        freq *= nz.octave_scale;
    }
    let sum = sum.clamp(Vec3::splat(-1.0), Vec3::splat(1.0));
    let sum = match &nz.remap {
        Some(rm) => {
            let f = |a: usize| rm[if nz.separate_axes { a } else { 0 }].eval((sum[a] + 1.0) * 0.5, r);
            Vec3::new(f(0), f(1), f(2))
        }
        None => sum,
    };
    let st = if nz.separate_axes { Vec3::new(nz.strength[0].eval(t, r), nz.strength[1].eval(t, r), nz.strength[2].eval(t, r)) } else { Vec3::splat(nz.strength[0].eval(t, r)) };
    // damping: strength in proportion to the field's scale (1 / frequency)
    let st = if nz.damping { st / nz.frequency.max(1e-4) } else { st };
    sum * st
}

pub fn step_system(s: &mut System, dt: f32, rng: &mut Rng, env: &Env) {
    if std::mem::take(&mut s.prewarm_pending) {
        // Prewarm: the system starts as if it had already run one full cycle
        let n = (s.def.duration * 30.0).ceil().clamp(1.0, 300.0) as u32;
        let step = s.def.duration / s.def.simulation_speed.max(1e-4) / n as f32;
        for _ in 0..n {
            step_system(s, step, rng, env);
        }
    }
    let dt = dt * s.def.simulation_speed;
    // particles that hit the environment last update die now
    s.particles.retain(|p| !p.kill);
    let here = s.emitter.w_axis.truncate();
    let moved = s.prev_emitter.map_or(0.0, |q| q.distance(here));
    s.emitter_vel = match s.def.emitter_velocity_mode {
        2 => s.def.custom_emitter_velocity,
        _ => match s.prev_emitter {
            Some(q) if dt > 0.0 => (here - q) / dt,
            _ => Vec3::ZERO,
        },
    };
    s.prev_emitter = Some(here);
    let def = s.def.clone();
    let (_, sim_rot, _) = s.sim_to_world.to_scale_rotation_translation();
    let (_, em_rot, _) = s.emitter.to_scale_rotation_translation();
    let world_to_sim = sim_rot.inverse();
    if let Some(nz) = &def.noise {
        s.noise_scroll += nz.scroll_speed.eval(if def.duration > 0.0 { s.time / def.duration } else { 0.0 }, 0.5) * dt;
    }
    let planes = std::mem::take(&mut s.planes);
    let c = Ctx {
        def: &def,
        g: world_to_sim * Vec3::new(0.0, GRAVITY, 0.0),
        local_to_sim: world_to_sim * em_rot,
        world_to_sim,
        center: s.sim_to_world.inverse().transform_point3(here),
        inherit: def.inherit_velocity.as_ref().filter(|iv| iv.mode == 1 && def.simulation_space == 1).map(|_| s.emitter_vel),
        noise_scroll: s.noise_scroll,
        sim_to_world: s.sim_to_world,
        size_scale: s.size_scale,
        planes: &planes,
        env,
        hits: std::cell::Cell::new(0),
    };
    s.particles.retain_mut(|p| step_particle(p, &c, dt, rng));
    let born = if s.playing { emit(s, dt, moved, rng) } else { Vec::new() };
    for (age, mut p) in born {
        if s.particles.len() as u32 >= def.max_particles {
            break;
        }
        if step_particle(&mut p, &c, age, rng) {
            s.particles.push(p);
        }
    }
    s.collisions += c.hits.get();
    s.planes = planes;
}

/// Emission this update: (age at the end of the update, particle).
fn emit(s: &mut System, dt: f32, moved: f32, rng: &mut Rng) -> Vec<(f32, Particle)> {
    let def = s.def.clone();
    let mut born: Vec<(f32, Particle)> = Vec::new();
    let mut dt_sys = dt;
    if s.delay > 0.0 {
        let used = s.delay.min(dt_sys);
        s.delay -= used;
        dt_sys -= used;
        if dt_sys <= 0.0 {
            return born;
        }
    }
    if !s.emitting() {
        return born;
    }
    let mut done = 0.0f32;
    while done < dt_sys && s.emitting() {
        let t0 = s.time;
        let t1 = (t0 + dt_sys - done).min(def.duration);
        let tn = if def.duration > 0.0 { t0 / def.duration } else { 0.0 };
        let cycle_start = s.cycle as f32 * def.duration;
        // the end of this update in cycle time
        let frame_end = t0 + dt_sys - done;
        if def.emission_enabled {
            for (bi, b) in def.bursts.iter().enumerate() {
                loop {
                    let k = s.burst_next[bi];
                    // cycleCount 0 repeats for the whole duration
                    if (b.cycles != 0 && k >= b.cycles) || (k > 0 && b.interval <= 0.0) {
                        break;
                    }
                    let bt = b.time + k as f32 * b.interval;
                    if bt >= t1 || bt >= def.duration {
                        break;
                    }
                    s.burst_next[bi] += 1;
                    if rng.f() > b.probability {
                        continue;
                    }
                    let n = b.count.eval(tn, rng.f()).round().max(0.0) as u32;
                    for i in 0..n {
                        let e = Emit { tn, elapsed: cycle_start + bt, burst: Some((i, n)) };
                        born.push((frame_end - bt, spawn(s, &e, rng)));
                    }
                }
            }
            let rate = def.rate_over_time.eval(tn, rng.f());
            if rate > 0.0 && t1 > t0 {
                s.rate_acc += rate * (t1 - t0);
                let n = s.rate_acc.floor();
                s.rate_acc -= n;
                for k in 0..n as u32 {
                    let te = t0 + (t1 - t0) * (k as f32 + 0.5) / n;
                    let e = Emit { tn: te / def.duration.max(1e-6), elapsed: cycle_start + te, burst: None };
                    born.push((frame_end - te, spawn(s, &e, rng)));
                }
            }
        }
        done += t1 - t0;
        s.time = t1;
        if def.looping && s.time >= def.duration {
            // next cycle: bursts fire again
            s.time = 0.0;
            s.cycle += 1;
            s.burst_next.iter_mut().for_each(|b| *b = 0);
            if def.duration <= 0.0 {
                break;
            }
        } else if t1 <= t0 {
            break;
        }
    }
    // rateOverDistance: the emitter's movement this update
    if def.emission_enabled && moved > 0.0 {
        let tn = if def.duration > 0.0 { s.time / def.duration } else { 0.0 };
        let rate = match s.rate_over_distance_mul {
            Some(m) => uk_assets::particles::MinMaxCurve { scalar: m, ..def.rate_over_distance.clone() }.eval(tn, rng.f()),
            None => def.rate_over_distance.eval(tn, rng.f()),
        };
        if rate > 0.0 {
            s.dist_acc += rate * moved;
            let n = s.dist_acc.floor();
            s.dist_acc -= n;
            let elapsed = s.cycle as f32 * def.duration + s.time;
            for k in 0..n as u32 {
                let e = Emit { tn, elapsed, burst: None };
                born.push((dt_sys * (k as f32 + 0.5) / n, spawn(s, &e, rng)));
            }
        }
    }
    born
}
