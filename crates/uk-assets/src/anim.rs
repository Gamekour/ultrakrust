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
    let mut crc = !0u32;
    for &b in path.as_bytes() {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
