//! Unity ParticleSystem (class 198) and ParticleSystemRenderer (class 199): the serialized
//! modules the game's effects use, with Unity's evaluation rules for curves and gradients, and
//! particle prefabs (a GameObject hierarchy whose nodes carry systems) loaded outside the scene
//! graph so the game can spawn them at runtime the way `Object.Instantiate` / pools do.
//!
//! Values stay in Unity space (left-handed, z forward); the simulation converts.

use crate::db::AssetDb;
use crate::scene::{to_bevy_point, to_bevy_quat, MaterialKey};
use crate::serialized::{SerializedFile, Value};
use crate::Result;
use bevy_math::{Quat, Vec3};
use std::sync::Arc;

const CLASS_TRANSFORM: i32 = 4;
const CLASS_MONOBEHAVIOUR: i32 = 114;
const CLASS_SPHERE_COLLIDER: i32 = 135;
const CLASS_PARTICLE_SYSTEM: i32 = 198;
const CLASS_PARTICLE_RENDERER: i32 = 199;

/// One AnimationCurve key (weights are not used by the game's particle curves).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Key {
    pub time: f32,
    pub value: f32,
    pub in_slope: f32,
    pub out_slope: f32,
}

/// AnimationCurve: cubic Hermite between keys, clamped outside them (particle curves span 0..1).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Curve(pub Vec<Key>);

impl Curve {
    fn read(v: &Value) -> Self {
        Curve(
            v.get("m_Curve")
                .array()
                .iter()
                .map(|k| Key { time: k.get("time").f32(), value: k.get("value").f32(), in_slope: k.get("inSlope").f32(), out_slope: k.get("outSlope").f32() })
                .collect(),
        )
    }

    pub fn eval(&self, t: f32) -> f32 {
        let k = &self.0;
        match k.len() {
            0 => return 0.0,
            1 => return k[0].value,
            _ => {}
        }
        if t <= k[0].time {
            return k[0].value;
        }
        let last = k[k.len() - 1];
        if t >= last.time {
            return last.value;
        }
        let i = k.windows(2).position(|w| t < w[1].time).unwrap_or(k.len() - 2);
        let (a, b) = (k[i], k[i + 1]);
        let dt = b.time - a.time;
        if dt <= 0.0 {
            return b.value;
        }
        // an infinite tangent is a step (constant until the next key)
        if !a.out_slope.is_finite() || !b.in_slope.is_finite() {
            return a.value;
        }
        let s = (t - a.time) / dt;
        let (s2, s3) = (s * s, s * s * s);
        let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
        let h10 = s3 - 2.0 * s2 + s;
        let h01 = -2.0 * s3 + 3.0 * s2;
        let h11 = s3 - s2;
        h00 * a.value + h10 * a.out_slope * dt + h01 * b.value + h11 * b.in_slope * dt
    }
}

/// ParticleSystem.MinMaxCurve. mode (minMaxState): 0 constant, 1 curve, 2 random between two
/// curves, 3 random between two constants. Curves are scaled by `scalar` (the curve multiplier).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MinMaxCurve {
    pub mode: u8,
    pub scalar: f32,
    pub min_scalar: f32,
    pub max: Curve,
    pub min: Curve,
}

impl MinMaxCurve {
    pub fn read(v: &Value) -> Self {
        Self {
            mode: v.get("minMaxState").i64() as u8,
            scalar: v.get("scalar").f32(),
            min_scalar: v.get("minScalar").f32(),
            max: Curve::read(v.get("maxCurve")),
            min: Curve::read(v.get("minCurve")),
        }
    }

    pub fn constant(c: f32) -> Self {
        Self { scalar: c, ..Self::default() }
    }

    /// Value at normalized time `t` with the random blend `r` in [0, 1).
    pub fn eval(&self, t: f32, r: f32) -> f32 {
        match self.mode {
            1 => self.max.eval(t) * self.scalar,
            2 => lerp(self.min.eval(t), self.max.eval(t), r) * self.scalar,
            3 => lerp(self.min_scalar, self.scalar, r),
            _ => self.scalar,
        }
    }

    /// Whether the value is the constant `c` everywhere.
    pub fn is_const(&self, c: f32) -> bool {
        match self.mode {
            0 => self.scalar == c,
            3 => self.scalar == c && self.min_scalar == c,
            _ => false,
        }
    }
}

/// Gradient: up to 8 color keys and 8 alpha keys (times in 1/65535). mode: 0 blend, 1 fixed,
/// 2 perceptual blend (Oklab).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gradient {
    pub colors: Vec<(f32, [f32; 3])>,
    pub alphas: Vec<(f32, f32)>,
    pub mode: u8,
}

impl Gradient {
    fn read(v: &Value) -> Self {
        let nc = v.get("m_NumColorKeys").i64().clamp(0, 8) as usize;
        let na = v.get("m_NumAlphaKeys").i64().clamp(0, 8) as usize;
        let key = |i: usize| v.get(&format!("key{i}"));
        Self {
            colors: (0..nc)
                .map(|i| {
                    let k = key(i);
                    (v.get(&format!("ctime{i}")).f32() / 65535.0, [k.get("r").f32(), k.get("g").f32(), k.get("b").f32()])
                })
                .collect(),
            alphas: (0..na).map(|i| (v.get(&format!("atime{i}")).f32() / 65535.0, key(i).get("a").f32())).collect(),
            mode: v.get("m_Mode").i64() as u8,
        }
    }

    pub fn eval(&self, t: f32) -> [f32; 4] {
        let fixed = self.mode == 1;
        let c = eval_keys(&self.colors, t, fixed, [1.0; 3], |a, b, s| {
            if self.mode == 2 {
                oklab_lerp(a, b, s)
            } else {
                [lerp(a[0], b[0], s), lerp(a[1], b[1], s), lerp(a[2], b[2], s)]
            }
        });
        let a = eval_keys(&self.alphas, t, fixed, 1.0, lerp);
        [c[0], c[1], c[2], a]
    }
}

/// Keys are sorted by time; before the first / after the last the end keys hold. Fixed mode
/// returns the first key at or after `t`.
fn eval_keys<T: Copy>(keys: &[(f32, T)], t: f32, fixed: bool, empty: T, mix: impl Fn(T, T, f32) -> T) -> T {
    let Some(first) = keys.first() else { return empty };
    if t <= first.0 {
        return first.1;
    }
    for w in keys.windows(2) {
        let ((ta, a), (tb, b)) = (w[0], w[1]);
        if t <= tb {
            if fixed {
                return b;
            }
            let d = tb - ta;
            return if d <= 0.0 { b } else { mix(a, b, (t - ta) / d) };
        }
    }
    keys[keys.len() - 1].1
}

fn oklab_lerp(a: [f32; 3], b: [f32; 3], s: f32) -> [f32; 3] {
    fn to_lab(c: [f32; 3]) -> [f32; 3] {
        let l = (0.4122214708 * c[0] + 0.5363325363 * c[1] + 0.0514459929 * c[2]).cbrt();
        let m = (0.2119034982 * c[0] + 0.6806995451 * c[1] + 0.1073969566 * c[2]).cbrt();
        let s = (0.0883024619 * c[0] + 0.2817188376 * c[1] + 0.6299787005 * c[2]).cbrt();
        [
            0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
            1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
            0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
        ]
    }
    fn from_lab(c: [f32; 3]) -> [f32; 3] {
        let l = (c[0] + 0.3963377774 * c[1] + 0.2158037573 * c[2]).powi(3);
        let m = (c[0] - 0.1055613458 * c[1] - 0.0638541728 * c[2]).powi(3);
        let s = (c[0] - 0.0894841775 * c[1] - 1.2914855480 * c[2]).powi(3);
        [
            4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
            -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
            -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
        ]
    }
    let (la, lb) = (to_lab(a), to_lab(b));
    from_lab([lerp(la[0], lb[0], s), lerp(la[1], lb[1], s), lerp(la[2], lb[2], s)])
}

/// ParticleSystem.MinMaxGradient. mode: 0 color, 1 gradient, 2 random between two colors,
/// 3 random between two gradients, 4 random color (a random point of the gradient).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MinMaxGradient {
    pub mode: u8,
    pub min_color: [f32; 4],
    pub max_color: [f32; 4],
    pub max: Gradient,
    pub min: Gradient,
}

impl MinMaxGradient {
    pub fn read(v: &Value) -> Self {
        let col = |c: &Value| [c.get("r").f32(), c.get("g").f32(), c.get("b").f32(), c.get("a").f32()];
        Self {
            mode: v.get("minMaxState").i64() as u8,
            min_color: col(v.get("minColor")),
            max_color: col(v.get("maxColor")),
            max: Gradient::read(v.get("maxGradient")),
            min: Gradient::read(v.get("minGradient")),
        }
    }

    pub fn eval(&self, t: f32, r: f32) -> [f32; 4] {
        let mix = |a: [f32; 4], b: [f32; 4]| [lerp(a[0], b[0], r), lerp(a[1], b[1], r), lerp(a[2], b[2], r), lerp(a[3], b[3], r)];
        match self.mode {
            1 => self.max.eval(t),
            2 => mix(self.min_color, self.max_color),
            3 => mix(self.min.eval(t), self.max.eval(t)),
            4 => self.max.eval(r),
            _ => self.max_color,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Burst {
    pub time: f32,
    pub count: MinMaxCurve,
    /// 0: repeats forever
    pub cycles: u32,
    pub interval: f32,
    pub probability: f32,
}

/// ShapeModule. kind: 0 sphere, 1 sphere shell (legacy), 2 hemisphere, 4 cone (from the base),
/// 5 box, 7 cone volume, 10 circle, 12 single-sided edge, 17 donut, 18 rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub kind: u8,
    pub angle: f32,
    pub length: f32,
    pub radius: f32,
    pub radius_thickness: f32,
    pub arc: f32,
    /// arc mode: 0 random, 1 loop, 2 ping-pong, 3 burst spread
    pub arc_mode: u8,
    pub box_thickness: Vec3,
    pub donut_radius: f32,
    pub position: Vec3,
    /// Euler degrees (Unity order: z, then x, then y)
    pub rotation: Vec3,
    pub scale: Vec3,
    pub align_to_direction: bool,
    pub random_direction: f32,
    pub spherical_direction: f32,
    pub random_position: f32,
}

/// VelocityOverLifetimeModule: extra velocity (not integrated into the particle's own) plus
/// radial speed, with the particle's velocity scaled by `speed_modifier`.
#[derive(Clone, Debug, PartialEq)]
pub struct VelocityOverLifetime {
    pub x: MinMaxCurve,
    pub y: MinMaxCurve,
    pub z: MinMaxCurve,
    pub orbital: [MinMaxCurve; 3],
    pub orbital_offset: [MinMaxCurve; 3],
    pub radial: MinMaxCurve,
    pub speed_modifier: MinMaxCurve,
    pub world_space: bool,
}

/// TrailModule (mode 0: one trail per particle; 1: ribbons through the particles).
#[derive(Clone, Debug, PartialEq)]
pub struct Trail {
    pub mode: u8,
    pub ratio: f32,
    /// vertex lifetime as a fraction of the particle's lifetime
    pub lifetime: MinMaxCurve,
    pub min_vertex_distance: f32,
    /// 0 stretch, 1 tile, 2 distribute per segment, 3 repeat per segment
    pub texture_mode: u8,
    pub world_space: bool,
    pub die_with_particles: bool,
    pub size_affects_width: bool,
    pub size_affects_lifetime: bool,
    pub inherit_particle_color: bool,
    pub color_over_lifetime: MinMaxGradient,
    pub width_over_trail: MinMaxCurve,
    pub color_over_trail: MinMaxGradient,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleSystemDef {
    /// lengthInSec
    pub duration: f32,
    pub looping: bool,
    pub prewarm: bool,
    pub play_on_awake: bool,
    pub start_delay: MinMaxCurve,
    pub simulation_speed: f32,
    /// moveWithTransform: 0 local, 1 world, 2 custom
    pub simulation_space: u8,
    /// 0 hierarchy, 1 local, 2 shape
    pub scaling_mode: u8,
    /// 0 automatic, 1 pause and catch up, 2 pause, 3 always simulate
    pub culling_mode: u8,
    /// 0 none, 1 disable, 2 destroy, 3 callback (Bloodsplatter.Awake switches its own to callback)
    pub stop_action: u8,
    pub start_lifetime: MinMaxCurve,
    pub start_speed: MinMaxCurve,
    pub start_color: MinMaxGradient,
    pub start_size: MinMaxCurve,
    pub start_rotation: MinMaxCurve,
    pub randomize_rotation: f32,
    pub max_particles: u32,
    pub gravity_modifier: MinMaxCurve,
    pub shape: Option<Shape>,
    pub emission_enabled: bool,
    pub rate_over_time: MinMaxCurve,
    pub rate_over_distance: MinMaxCurve,
    pub bursts: Vec<Burst>,
    pub color_over_lifetime: Option<MinMaxGradient>,
    pub size_over_lifetime: Option<MinMaxCurve>,
    /// RotationModule z (radians per second)
    pub rotation_over_lifetime: Option<MinMaxCurve>,
    pub velocity_over_lifetime: Option<VelocityOverLifetime>,
    pub trail: Option<Trail>,
    /// Enabled modules (or module settings) the simulation does not implement.
    pub unsupported: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParticleRendererDef {
    pub enabled: bool,
    /// 0 billboard, 1 stretched, 2 horizontal, 3 vertical, 4 mesh, 5 none
    pub render_mode: u8,
    /// 0 none, 1 by distance, 2 oldest first, 3 youngest first
    pub sort_mode: u8,
    pub sorting_fudge: f32,
    /// fractions of the viewport height
    pub min_particle_size: f32,
    pub max_particle_size: f32,
    /// 0 view, 1 world, 2 local, 3 facing, 4 velocity
    pub render_alignment: u8,
    pub pivot: Vec3,
    pub flip: Vec3,
    /// ParticleSystemVertexStream ids fed to the shader (custom streams, else the default set)
    pub vertex_streams: Vec<u8>,
    /// [particle material, trail material]
    pub materials: Vec<Option<MaterialKey>>,
    pub length_scale: f32,
    pub velocity_scale: f32,
}

impl ParticleSystemDef {
    pub fn read(v: &Value) -> Self {
        let init = v.get("InitialModule");
        let mut unsupported = Vec::new();
        let on = |m: &str| v.get(m).get("enabled").bool();
        let shape = on("ShapeModule").then(|| {
            let s = v.get("ShapeModule");
            let kind = s.get("type").i64() as u8;
            if !matches!(kind, 0 | 1 | 2 | 4 | 7 | 10) {
                unsupported.push(format!("ShapeModule.type {kind}"));
            }
            Shape {
                kind,
                angle: s.get("angle").f32(),
                length: s.get("length").f32(),
                radius: s.get("radius").get("value").f32(),
                radius_thickness: s.get("radiusThickness").f32(),
                arc: s.get("arc").get("value").f32(),
                arc_mode: s.get("arc").get("mode").i64() as u8,
                box_thickness: Vec3::from(s.get("boxThickness").vec3()),
                donut_radius: s.get("donutRadius").f32(),
                position: Vec3::from(s.get("m_Position").vec3()),
                rotation: Vec3::from(s.get("m_Rotation").vec3()),
                scale: Vec3::from(s.get("m_Scale").vec3()),
                align_to_direction: s.get("alignToDirection").bool(),
                random_direction: s.get("randomDirectionAmount").f32(),
                spherical_direction: s.get("sphericalDirectionAmount").f32(),
                random_position: s.get("randomPositionAmount").f32(),
            }
        });
        let em = v.get("EmissionModule");
        let n_bursts = em.get("m_BurstCount").i64().max(0) as usize;
        let bursts = em
            .get("m_Bursts")
            .array()
            .iter()
            .take(n_bursts)
            .map(|b| Burst {
                time: b.get("time").f32(),
                count: MinMaxCurve::read(b.get("countCurve")),
                cycles: b.get("cycleCount").i64().max(0) as u32,
                interval: b.get("repeatInterval").f32(),
                probability: b.get("probability").f32(),
            })
            .collect();
        let vel = on("VelocityModule").then(|| {
            let m = v.get("VelocityModule");
            let c = |k: &str| MinMaxCurve::read(m.get(k));
            VelocityOverLifetime {
                x: c("x"),
                y: c("y"),
                z: c("z"),
                orbital: [c("orbitalX"), c("orbitalY"), c("orbitalZ")],
                orbital_offset: [c("orbitalOffsetX"), c("orbitalOffsetY"), c("orbitalOffsetZ")],
                radial: c("radial"),
                speed_modifier: c("speedModifier"),
                world_space: m.get("inWorldSpace").bool(),
            }
        });
        if let Some(vl) = &vel {
            if vl.orbital.iter().chain(vl.orbital_offset.iter()).any(|c| !c.is_const(0.0)) {
                unsupported.push("VelocityModule.orbital".into());
            }
        }
        let trail = on("TrailModule").then(|| {
            let t = v.get("TrailModule");
            let mode = t.get("mode").i64() as u8;
            if mode != 0 {
                unsupported.push("TrailModule.mode ribbon".into());
            }
            Trail {
                mode,
                ratio: t.get("ratio").f32(),
                lifetime: MinMaxCurve::read(t.get("lifetime")),
                min_vertex_distance: t.get("minVertexDistance").f32(),
                texture_mode: t.get("textureMode").i64() as u8,
                world_space: t.get("worldSpace").bool(),
                die_with_particles: t.get("dieWithParticles").bool(),
                size_affects_width: t.get("sizeAffectsWidth").bool(),
                size_affects_lifetime: t.get("sizeAffectsLifetime").bool(),
                inherit_particle_color: t.get("inheritParticleColor").bool(),
                color_over_lifetime: MinMaxGradient::read(t.get("colorOverLifetime")),
                width_over_trail: MinMaxCurve::read(t.get("widthOverTrail")),
                color_over_trail: MinMaxGradient::read(t.get("colorOverTrail")),
            }
        });
        if init.get("size3D").bool() {
            unsupported.push("InitialModule.size3D".into());
        }
        if init.get("rotation3D").bool() {
            unsupported.push("InitialModule.rotation3D".into());
        }
        if v.get("SizeModule").get("separateAxes").bool() && on("SizeModule") {
            unsupported.push("SizeModule.separateAxes".into());
        }
        if v.get("RotationModule").get("separateAxes").bool() && on("RotationModule") {
            unsupported.push("RotationModule.separateAxes".into());
        }
        if let Value::Struct(fields) = v {
            for (k, m) in fields {
                let handled = ["InitialModule", "ShapeModule", "EmissionModule", "ColorModule", "SizeModule", "RotationModule", "VelocityModule", "TrailModule"];
                if k.ends_with("Module") && !handled.contains(&&**k) && m.get("enabled").bool() {
                    unsupported.push(k.to_string());
                }
            }
        }
        if v.get("moveWithTransform").i64() == 2 {
            unsupported.push("simulationSpace custom".into());
        }
        Self {
            duration: v.get("lengthInSec").f32(),
            looping: v.get("looping").bool(),
            prewarm: v.get("prewarm").bool(),
            play_on_awake: v.get("playOnAwake").bool(),
            start_delay: MinMaxCurve::read(v.get("startDelay")),
            simulation_speed: v.get("simulationSpeed").f32(),
            simulation_space: v.get("moveWithTransform").i64() as u8,
            scaling_mode: v.get("scalingMode").i64() as u8,
            culling_mode: v.get("cullingMode").i64() as u8,
            stop_action: v.get("stopAction").i64() as u8,
            start_lifetime: MinMaxCurve::read(init.get("startLifetime")),
            start_speed: MinMaxCurve::read(init.get("startSpeed")),
            start_color: MinMaxGradient::read(init.get("startColor")),
            start_size: MinMaxCurve::read(init.get("startSize")),
            start_rotation: MinMaxCurve::read(init.get("startRotation")),
            randomize_rotation: init.get("randomizeRotationDirection").f32(),
            max_particles: init.get("maxNumParticles").i64().max(0) as u32,
            gravity_modifier: MinMaxCurve::read(init.get("gravityModifier")),
            shape,
            emission_enabled: on("EmissionModule"),
            rate_over_time: MinMaxCurve::read(em.get("rateOverTime")),
            rate_over_distance: MinMaxCurve::read(em.get("rateOverDistance")),
            bursts,
            color_over_lifetime: on("ColorModule").then(|| MinMaxGradient::read(v.get("ColorModule").get("gradient"))),
            size_over_lifetime: on("SizeModule").then(|| MinMaxCurve::read(v.get("SizeModule").get("curve"))),
            rotation_over_lifetime: on("RotationModule").then(|| MinMaxCurve::read(v.get("RotationModule").get("curve"))),
            velocity_over_lifetime: vel,
            trail,
            unsupported,
        }
    }
}

impl ParticleRendererDef {
    pub fn read(v: &Value, materials: Vec<Option<MaterialKey>>) -> Self {
        let streams = if v.get("m_UseCustomVertexStreams").bool() {
            v.get("m_VertexStreams").bytes().to_vec()
        } else {
            // Unity's default set: position, normal, color, uv
            vec![0, 1, 3, 4]
        };
        Self {
            enabled: v.get("m_Enabled").bool(),
            render_mode: v.get("m_RenderMode").i64() as u8,
            sort_mode: v.get("m_SortMode").i64() as u8,
            sorting_fudge: v.get("m_SortingFudge").f32(),
            min_particle_size: v.get("m_MinParticleSize").f32(),
            max_particle_size: v.get("m_MaxParticleSize").f32(),
            render_alignment: v.get("m_RenderAlignment").i64() as u8,
            pivot: Vec3::from(v.get("m_Pivot").vec3()),
            flip: Vec3::from(v.get("m_Flip").vec3()),
            vertex_streams: streams,
            materials,
            length_scale: v.get("m_LengthScale").f32(),
            velocity_scale: v.get("m_VelocityScale").f32(),
        }
    }
}

/// A MonoBehaviour on a prefab node: class name and serialized fields.
#[derive(Debug, Clone)]
pub struct PrefabScript {
    pub class: String,
    pub enabled: bool,
    pub data: Value,
}

/// One GameObject of a particle prefab. Transforms are local to the parent (Bevy space).
#[derive(Debug, Clone)]
pub struct PrefabNode {
    pub name: String,
    pub parent: Option<u32>,
    pub active_self: bool,
    pub layer: u8,
    pub local_pos: Vec3,
    pub local_rot: Quat,
    pub local_scale: Vec3,
    pub system: Option<Arc<ParticleSystemDef>>,
    pub renderer: Option<Arc<ParticleRendererDef>>,
    /// SphereCollider (center in Bevy space, radius, trigger, enabled)
    pub sphere: Option<(Vec3, f32, bool, bool)>,
    pub scripts: Vec<PrefabScript>,
}

/// A prefab hierarchy (node 0 is the root, parents before children) as `Object.Instantiate`
/// would copy it.
#[derive(Debug, Clone)]
pub struct ParticlePrefab {
    pub name: String,
    pub nodes: Vec<PrefabNode>,
}

impl ParticlePrefab {
    pub fn script(&self, node: u32, class: &str) -> Option<&PrefabScript> {
        self.nodes[node as usize].scripts.iter().find(|s| s.class == class)
    }

    /// GameObject.activeInHierarchy within the prefab.
    pub fn is_active_in_hierarchy(&self, mut n: u32) -> bool {
        loop {
            let node = &self.nodes[n as usize];
            if !node.active_self {
                return false;
            }
            match node.parent {
                Some(p) => n = p,
                None => return true,
            }
        }
    }
}

/// Loads the prefab whose root GameObject is `go` in `file`.
pub fn load_prefab(db: &mut AssetDb, file: &Arc<SerializedFile>, go: i64) -> Result<ParticlePrefab> {
    let mut nodes: Vec<PrefabNode> = Vec::new();
    let mut stack = vec![(go, None::<u32>)];
    while let Some((g, parent)) = stack.pop() {
        let v = file.read_id(g)?;
        let idx = nodes.len() as u32;
        let mut node = PrefabNode {
            name: v.get("m_Name").str().to_string(),
            parent,
            active_self: v.get("m_IsActive").bool(),
            layer: v.get("m_Layer").i64() as u8,
            local_pos: Vec3::ZERO,
            local_rot: Quat::IDENTITY,
            local_scale: Vec3::ONE,
            system: None,
            renderer: None,
            sphere: None,
            scripts: Vec::new(),
        };
        let mut kids = Vec::new();
        for c in v.get("m_Component").array() {
            let (_, cid) = c.get("component").pptr();
            let Some(o) = file.object(cid) else { continue };
            let class = o.class_id;
            let cv = file.read_id(cid)?;
            match class {
                CLASS_TRANSFORM => {
                    let p = Vec3::from(cv.get("m_LocalPosition").vec3());
                    let q = cv.get("m_LocalRotation").quat();
                    node.local_pos = to_bevy_point(p);
                    node.local_rot = to_bevy_quat(Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize());
                    node.local_scale = Vec3::from(cv.get("m_LocalScale").vec3());
                    for k in cv.get("m_Children").array() {
                        kids.push(file.read_id(k.pptr().1)?.get("m_GameObject").pptr().1);
                    }
                }
                CLASS_PARTICLE_SYSTEM => node.system = Some(Arc::new(ParticleSystemDef::read(&cv))),
                CLASS_PARTICLE_RENDERER => {
                    let mats = cv
                        .get("m_Materials")
                        .array()
                        .iter()
                        .map(|m| db.resolve(file, m.pptr()).ok().flatten().map(|(f, id)| MaterialKey { file: f.name.clone(), path_id: id }))
                        .collect();
                    node.renderer = Some(Arc::new(ParticleRendererDef::read(&cv, mats)));
                }
                CLASS_SPHERE_COLLIDER => {
                    node.sphere = Some((to_bevy_point(Vec3::from(cv.get("m_Center").vec3())), cv.get("m_Radius").f32(), cv.get("m_IsTrigger").bool(), cv.get("m_Enabled").bool()));
                }
                CLASS_MONOBEHAVIOUR => {
                    let class = db.read_pptr(file, cv.get("m_Script").pptr()).ok().flatten().map(|(_, _, s)| s.get("m_ClassName").str().to_string()).unwrap_or_default();
                    node.scripts.push(PrefabScript { class, enabled: cv.get("m_Enabled").bool(), data: cv });
                }
                _ => {}
            }
        }
        nodes.push(node);
        // children in Transform order: pushed reversed so the stack pops them first-to-last
        for k in kids.into_iter().rev() {
            stack.push((k, Some(idx)));
        }
    }
    let name = nodes.first().map(|n| n.name.clone()).unwrap_or_default();
    Ok(ParticlePrefab { name, nodes })
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hermite_matches_linear_tangents() {
        // a straight line's Hermite with matching slopes is the line itself
        let c = Curve(vec![Key { time: 0.0, value: 1.0, in_slope: -1.0, out_slope: -1.0 }, Key { time: 1.0, value: 0.0, in_slope: -1.0, out_slope: -1.0 }]);
        for i in 0..=10 {
            let t = i as f32 / 10.0;
            assert!((c.eval(t) - (1.0 - t)).abs() < 1e-5);
        }
        assert_eq!(c.eval(-1.0), 1.0);
        assert_eq!(c.eval(2.0), 0.0);
    }

    #[test]
    fn gradient_blend_and_fixed() {
        let g = Gradient { colors: vec![(0.0, [1.0, 0.0, 0.0]), (1.0, [0.0, 0.0, 1.0])], alphas: vec![(0.0, 1.0), (1.0, 0.0)], mode: 0 };
        assert_eq!(g.eval(0.5), [0.5, 0.0, 0.5, 0.5]);
        let f = Gradient { mode: 1, ..g };
        assert_eq!(f.eval(0.5), [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(f.eval(0.0), [1.0, 0.0, 0.0, 1.0]);
    }
}
