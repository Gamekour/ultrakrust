//! Mecanim AnimationClips (Unity 2022): streamed (Hermite-as-cubic keys), dense (sampled) and
//! constant curves, the generic bindings that say which transform/property each curve drives, and
//! clip events. `Clip::sample(t)` gives every curve's value at time t, in binding order.
use crate::serialized::Value;

/// What a run of curves drives. `path` is CRC32 of the transform path below the Animator.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    Position,
    Rotation,
    Scale,
    Euler,
    /// any other property (material, blend shape, script field, animator parameter, active, ...)
    Other { type_id: i32, attribute: u32, pptr: bool },
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub path: u32,
    pub target: Target,
    /// first curve index; Position/Scale/Euler use 3, Rotation 4 (x, y, z, w), Other 1
    pub curve: usize,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub time: f32,
    pub function: String,
    pub string: String,
    pub float: f32,
    pub int: i32,
    pub object: (i32, i64),
}

#[derive(Debug, Clone, Copy)]
struct Key {
    curve: u32,
    coeff: [f32; 4],
}

#[derive(Debug, Clone, Default)]
pub struct Clip {
    pub name: String,
    pub start: f32,
    pub stop: f32,
    pub looping: bool,
    pub sample_rate: f32,
    pub bindings: Vec<Binding>,
    pub events: Vec<Event>,
    /// PPtr curve targets (sprites, materials): the curve value indexes this list
    pub pptrs: Vec<(i32, i64)>,
    streamed_count: usize,
    /// streamed frames: time, keys (each key holds the cubic for the segment starting at `time`)
    frames: Vec<(f32, Vec<Key>)>,
    dense_count: usize,
    dense_frames: usize,
    dense_rate: f32,
    dense_begin: f32,
    dense: Vec<f32>,
    constant: Vec<f32>,
}

fn floats(v: &Value) -> Vec<f32> {
    v.array().iter().map(|x| x.f32()).collect()
}

impl Clip {
    pub fn curve_count(&self) -> usize {
        self.streamed_count + self.dense_count + self.constant.len()
    }

    /// Curves the bindings expect (pos/scale/euler 3, rotation 4, everything else 1), except
    /// trailing flipbook PPtr bindings that have no curve (see `pptr_at`).
    pub fn bound_curves(&self) -> usize {
        let all = self.bindings.last().map_or(0, |b| b.curve + width(b.target));
        let implicit = self.bindings.iter().rev().take_while(|b| matches!(b.target, Target::Other { pptr: true, .. })).count();
        if all > self.curve_count() && all - implicit == self.curve_count() { self.curve_count() } else { all }
    }

    pub fn streamed_curves(&self) -> usize {
        self.streamed_count
    }

    /// Streamed key frame times (without the -inf / +inf sentinels).
    pub fn frame_times(&self) -> impl Iterator<Item = f32> + '_ {
        self.frames.iter().map(|f| f.0).filter(|t| t.is_finite() && t.abs() < f32::MAX)
    }

    pub fn duration(&self) -> f32 {
        (self.stop - self.start).max(0.0)
    }

    pub fn from_value(v: &Value) -> Clip {
        let mc = v.get("m_MuscleClip");
        let data = mc.get("m_Clip").get("data");
        let mut c = Clip {
            name: v.get("m_Name").str().to_string(),
            start: mc.get("m_StartTime").f32(),
            stop: mc.get("m_StopTime").f32(),
            looping: mc.get("m_LoopTime").bool(),
            sample_rate: v.get("m_SampleRate").f32(),
            ..Default::default()
        };
        // streamed: u32 words -> frames { time, n, n x { index, c0..c3 } }
        let s = data.get("m_StreamedClip");
        c.streamed_count = s.get("curveCount").i64().max(0) as usize;
        let words: Vec<u32> = s.get("data").array().iter().map(|w| w.i64() as u32).collect();
        let mut i = 0;
        while i + 2 <= words.len() {
            let time = f32::from_bits(words[i]);
            let n = words[i + 1] as usize;
            i += 2;
            let mut keys = Vec::with_capacity(n);
            for _ in 0..n {
                if i + 5 > words.len() {
                    break;
                }
                let f = |k: usize| f32::from_bits(words[i + k]);
                keys.push(Key { curve: words[i], coeff: [f(1), f(2), f(3), f(4)] });
                i += 5;
            }
            c.frames.push((time, keys));
        }
        let d = data.get("m_DenseClip");
        c.dense_count = d.get("m_CurveCount").i64().max(0) as usize;
        c.dense_frames = d.get("m_FrameCount").i64().max(0) as usize;
        c.dense_rate = d.get("m_SampleRate").f32();
        c.dense_begin = d.get("m_BeginTime").f32();
        c.dense = floats(d.get("m_SampleArray"));
        c.constant = floats(data.get("m_ConstantClip").get("data"));

        let bc = v.get("m_ClipBindingConstant");
        let mut curve = 0;
        for g in bc.get("genericBindings").array() {
            let type_id = g.get("typeID").i64() as i32;
            let attribute = g.get("attribute").i64() as u32;
            let target = match (type_id, attribute) {
                (4, 1) => Target::Position,
                (4, 2) => Target::Rotation,
                (4, 3) => Target::Scale,
                (4, 4) => Target::Euler,
                _ => Target::Other { type_id, attribute, pptr: g.get("isPPtrCurve").bool() },
            };
            c.bindings.push(Binding { path: g.get("path").i64() as u32, target, curve });
            curve += width(target);
        }
        c.pptrs = bc.get("pptrCurveMapping").array().iter().map(|p| p.pptr()).collect();
        c.events = v
            .get("m_Events")
            .array()
            .iter()
            .map(|e| Event {
                time: e.get("time").f32(),
                function: e.get("functionName").str().to_string(),
                string: e.get("data").str().to_string(),
                float: e.get("floatParameter").f32(),
                int: e.get("intParameter").i64() as i32,
                object: e.get("objectReferenceParameter").pptr(),
            })
            .collect();
        c
    }

    /// Every curve's value at clip-local time `t` (clamped to the clip; looping is the caller's).
    pub fn sample_into(&self, t: f32, out: &mut Vec<f32>) {
        out.clear();
        out.resize(self.curve_count(), 0.0);
        // streamed: the last key of each curve at or before t defines its segment
        for (ft, keys) in &self.frames {
            if *ft > t && *ft != f32::MIN {
                break;
            }
            for k in keys {
                let dt = if *ft == f32::MIN { 0.0 } else { t - ft };
                let [a, b, cc, d] = k.coeff;
                if let Some(o) = out.get_mut(k.curve as usize) {
                    *o = ((a * dt + b) * dt + cc) * dt + d;
                }
            }
        }
        if self.dense_count > 0 && self.dense_frames > 0 {
            let f = ((t - self.dense_begin) * self.dense_rate).max(0.0);
            let f0 = (f.floor() as usize).min(self.dense_frames - 1);
            let f1 = (f0 + 1).min(self.dense_frames - 1);
            let w = (f - f0 as f32).clamp(0.0, 1.0);
            for i in 0..self.dense_count {
                let a = self.dense.get(f0 * self.dense_count + i).copied().unwrap_or(0.0);
                let b = self.dense.get(f1 * self.dense_count + i).copied().unwrap_or(a);
                out[self.streamed_count + i] = a + (b - a) * w;
            }
        }
        let base = self.streamed_count + self.dense_count;
        out[base..].copy_from_slice(&self.constant);
    }

    /// PPtr curve value at `t`: an index into `pptrs`. The game's flipbook clips (sprite/material
    /// swaps) carry no float curve for it, only the mapping, whose entries are the keys evenly
    /// spaced over the clip (Fire*: 60 sprites over 5 s at 12 fps); those are sampled that way.
    pub fn pptr_at(&self, b: &Binding, t: f32, samples: &[f32]) -> Option<(i32, i64)> {
        let i = match samples.get(b.curve) {
            Some(v) if b.curve < self.curve_count() => *v as usize,
            _ => (((t - self.start) / self.duration().max(1e-6)) * self.pptrs.len() as f32).max(0.0) as usize,
        };
        self.pptrs.get(i.min(self.pptrs.len().saturating_sub(1))).copied()
    }

    /// Bindings with float curves present (implicit flipbook PPtr bindings excluded).
    pub fn float_bindings(&self) -> impl Iterator<Item = &Binding> {
        let n = self.curve_count();
        self.bindings.iter().filter(move |b| b.curve + width(b.target) <= n)
    }

    pub fn sample(&self, t: f32) -> Vec<f32> {
        let mut v = Vec::new();
        self.sample_into(t, &mut v);
        v
    }
}

pub fn width(t: Target) -> usize {
    match t {
        Target::Position | Target::Scale | Target::Euler => 3,
        Target::Rotation => 4,
        Target::Other { .. } => 1,
    }
}

/// Unity's path hash for animation bindings (CRC32 / IEEE of the UTF-8 path, "a/b/c").
pub fn path_hash(path: &str) -> u32 {
    !crc_feed(!0, path.as_bytes())
}

/// Running (un-finalized) CRC32 of `path_hash`: hash a path's prefix once, extend per child.
pub fn crc_feed(mut crc: u32, bytes: &[u8]) -> u32 {
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    crc
}

// ---------------------------------------------------------------------------------------------
// AnimatorController (class 91): the compiled Mecanim runtime constant (layers -> state machines
// -> states -> transitions / blend trees) plus parameters, the hash -> name table and clip list.

/// Transition condition modes (AnimatorConditionMode).
pub mod cond {
    pub const IF: u32 = 1;
    pub const IF_NOT: u32 = 2;
    pub const GREATER: u32 = 3;
    pub const LESS: u32 = 4;
    pub const EXIT_TIME: u32 = 5;
    pub const EQUALS: u32 = 6;
    pub const NOT_EQUAL: u32 = 7;
}

/// Parameter types (AnimatorControllerParameterType).
pub mod param {
    pub const FLOAT: u32 = 1;
    pub const INT: u32 = 3;
    pub const BOOL: u32 = 4;
    pub const TRIGGER: u32 = 9;
}

/// Destination indices at or above this are selector (sub-state-machine entry/exit) states.
pub const SELECTOR_BASE: u32 = 30000;

#[derive(Debug, Clone)]
pub struct Condition {
    pub mode: u32,
    pub param: u32,
    pub threshold: f32,
    pub exit_time: f32,
}

#[derive(Debug, Clone)]
pub struct Transition {
    pub conditions: Vec<Condition>,
    pub dest: u32,
    pub full_path: u32,
    pub duration: f32,
    pub offset: f32,
    pub exit_time: f32,
    pub has_exit_time: bool,
    pub fixed_duration: bool,
    /// 0 none, 1 source, 2 destination, 3 source then destination, 4 destination then source
    pub interruption: u32,
    pub ordered_interruption: bool,
    pub to_self: bool,
}

#[derive(Debug, Clone)]
pub struct BlendNode {
    /// 0 1D, 1 SimpleDirectional2D, 2 FreeformDirectional2D, 3 FreeformCartesian2D, 4 Direct
    pub kind: u32,
    pub param: u32,
    pub param_y: u32,
    pub children: Vec<u32>,
    pub thresholds: Vec<f32>,
    pub positions: Vec<[f32; 2]>,
    pub direct_params: Vec<u32>,
    /// index into `Controller::clips`
    pub clip: Option<u32>,
    pub duration: f32,
    pub cycle_offset: f32,
    pub mirror: bool,
}

#[derive(Debug, Clone)]
pub struct State {
    pub name: u32,
    pub full_path: u32,
    pub speed: f32,
    /// parameter multiplying speed (None when unused)
    pub speed_param: Option<u32>,
    pub cycle_offset: f32,
    pub looping: bool,
    pub write_defaults: bool,
    pub transitions: Vec<Transition>,
    /// per synchronized layer: index into `trees` (None = empty state)
    pub tree_index: Vec<Option<u32>>,
    pub trees: Vec<Vec<BlendNode>>,
}

impl State {
    /// Blend nodes (root first) of this state for a layer's synchronized index.
    pub fn tree(&self, sync: usize) -> Option<&[BlendNode]> {
        let i = (*self.tree_index.get(sync).or(self.tree_index.first())?)? as usize;
        self.trees.get(i).map(|t| t.as_slice())
    }
}

#[derive(Debug, Clone)]
pub struct Selector {
    pub full_path: u32,
    pub entry: bool,
    pub transitions: Vec<(u32, Vec<Condition>)>,
}

#[derive(Debug, Clone)]
pub struct Machine {
    pub states: Vec<State>,
    pub any_state: Vec<Transition>,
    pub selectors: Vec<Selector>,
    pub default_state: u32,
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub machine: u32,
    pub sync_index: u32,
    pub binding: u32,
    /// 0 override, 1 additive
    pub blending: u32,
    pub weight: f32,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub id: u32,
    pub kind: u32,
    pub index: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Controller {
    pub name: String,
    pub layers: Vec<Layer>,
    pub machines: Vec<Machine>,
    pub params: Vec<Param>,
    pub floats: Vec<f32>,
    pub ints: Vec<i32>,
    pub bools: Vec<bool>,
    pub names: std::collections::HashMap<u32, String>,
    /// PPtrs to AnimationClips; blend nodes index this
    pub clips: Vec<(i32, i64)>,
}

fn data(v: &Value) -> &Value {
    if v.has("data") { v.get("data") } else { v }
}

fn u32s(v: &Value) -> Vec<u32> {
    v.array().iter().map(|x| x.i64() as u32).collect()
}

fn conditions(v: &Value) -> Vec<Condition> {
    v.array()
        .iter()
        .map(|c| {
            let c = data(c);
            Condition { mode: c.get("m_ConditionMode").i64() as u32, param: c.get("m_EventID").i64() as u32, threshold: c.get("m_EventThreshold").f32(), exit_time: c.get("m_ExitTime").f32() }
        })
        .collect()
}

fn transition(t: &Value) -> Transition {
    let t = data(t);
    Transition {
        conditions: conditions(t.get("m_ConditionConstantArray")),
        dest: t.get("m_DestinationState").i64() as u32,
        full_path: t.get("m_FullPathID").i64() as u32,
        duration: t.get("m_TransitionDuration").f32(),
        offset: t.get("m_TransitionOffset").f32(),
        exit_time: t.get("m_ExitTime").f32(),
        has_exit_time: t.get("m_HasExitTime").bool(),
        fixed_duration: t.get("m_HasFixedDuration").bool(),
        interruption: t.get("m_InterruptionSource").i64() as u32,
        ordered_interruption: t.get("m_OrderedInterruption").bool(),
        to_self: t.get("m_CanTransitionToSelf").bool(),
    }
}

fn blend_node(n: &Value) -> BlendNode {
    let n = data(n);
    let clip = n.get("m_ClipID").i64() as u32;
    let b2 = data(n.get("m_Blend2dData"));
    BlendNode {
        kind: n.get("m_BlendType").i64() as u32,
        param: n.get("m_BlendEventID").i64() as u32,
        param_y: n.get("m_BlendEventYID").i64() as u32,
        children: u32s(n.get("m_ChildIndices")),
        thresholds: floats(data(n.get("m_Blend1dData")).get("m_ChildThresholdArray")),
        positions: b2.get("m_ChildPositionArray").array().iter().map(|p| [p.get("x").f32(), p.get("y").f32()]).collect(),
        direct_params: u32s(data(n.get("m_BlendDirectData")).get("m_ChildBlendEventIDArray")),
        clip: (clip != u32::MAX).then_some(clip),
        duration: n.get("m_Duration").f32(),
        cycle_offset: n.get("m_CycleOffset").f32(),
        mirror: n.get("m_Mirror").bool(),
    }
}

fn state(s: &Value) -> State {
    let s = data(s);
    let sp = s.get("m_SpeedParamID").i64() as u32;
    State {
        name: s.get("m_NameID").i64() as u32,
        full_path: s.get("m_FullPathID").i64() as u32,
        speed: s.get("m_Speed").f32(),
        speed_param: (sp != 0).then_some(sp),
        cycle_offset: s.get("m_CycleOffset").f32(),
        looping: s.get("m_Loop").bool(),
        write_defaults: s.get("m_WriteDefaultValues").bool(),
        transitions: s.get("m_TransitionConstantArray").array().iter().map(transition).collect(),
        tree_index: s.get("m_BlendTreeConstantIndexArray").array().iter().map(|i| (i.i64() >= 0).then_some(i.i64() as u32)).collect(),
        trees: s.get("m_BlendTreeConstantArray").array().iter().map(|t| data(t).get("m_NodeArray").array().iter().map(blend_node).collect()).collect(),
    }
}

fn machine(m: &Value) -> Machine {
    let m = data(m);
    let selectors = m
        .get("m_SelectorStateConstantArray")
        .array()
        .iter()
        .map(|s| {
            let s = data(s);
            let transitions = s
                .get("m_TransitionConstantArray")
                .array()
                .iter()
                .map(|t| {
                    let t = data(t);
                    (t.get("m_Destination").i64() as u32, conditions(t.get("m_ConditionConstantArray")))
                })
                .collect();
            Selector { full_path: s.get("m_FullPathID").i64() as u32, entry: s.get("m_IsEntry").bool(), transitions }
        })
        .collect();
    Machine {
        states: m.get("m_StateConstantArray").array().iter().map(state).collect(),
        any_state: m.get("m_AnyStateTransitionConstantArray").array().iter().map(transition).collect(),
        selectors,
        default_state: m.get("m_DefaultState").i64() as u32,
    }
}

impl Controller {
    pub fn from_value(v: &Value) -> Controller {
        let c = v.get("m_Controller");
        let layers = c
            .get("m_LayerArray")
            .array()
            .iter()
            .map(|l| {
                let l = data(l);
                Layer {
                    machine: l.get("m_StateMachineIndex").i64() as u32,
                    sync_index: l.get("m_StateMachineSynchronizedLayerIndex").i64() as u32,
                    binding: l.get("m_Binding").i64() as u32,
                    blending: l.get("(int&)m_LayerBlendingMode").i64() as u32,
                    weight: l.get("m_DefaultWeight").f32(),
                }
            })
            .collect();
        let params = data(c.get("m_Values"))
            .get("m_ValueArray")
            .array()
            .iter()
            .map(|p| Param { id: p.get("m_ID").i64() as u32, kind: p.get("m_Type").i64() as u32, index: p.get("m_Index").i64() as u32 })
            .collect();
        let dv = data(c.get("m_DefaultValues"));
        Controller {
            name: v.get("m_Name").str().to_string(),
            layers,
            machines: c.get("m_StateMachineArray").array().iter().map(machine).collect(),
            params,
            floats: floats(dv.get("m_FloatValues")),
            ints: dv.get("m_IntValues").array().iter().map(|x| x.i64() as i32).collect(),
            bools: dv.get("m_BoolValues").array().iter().map(|x| x.bool()).collect(),
            names: v.get("m_TOS").array().iter().map(|p| (p.get("first").i64() as u32, p.get("second").str().to_string())).collect(),
            clips: v.get("m_AnimationClips").array().iter().map(|p| p.pptr()).collect(),
        }
    }

    /// AnimatorOverrideController (class 221): the base controller with clips swapped.
    pub fn with_overrides(mut self, overrides: &[((i32, i64), (i32, i64))], same: impl Fn((i32, i64), (i32, i64)) -> bool) -> Controller {
        for c in &mut self.clips {
            if let Some((_, o)) = overrides.iter().find(|(orig, o)| same(*orig, *c) && o.1 != 0) {
                *c = *o;
            }
        }
        self
    }

    pub fn param(&self, id: u32) -> Option<&Param> {
        self.params.iter().find(|p| p.id == id)
    }

    pub fn name(&self, hash: u32) -> &str {
        self.names.get(&hash).map_or("", |s| s.as_str())
    }
}
