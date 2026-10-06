//! Mecanim runtime: every Animator in the scene runs its AnimatorController (state machine,
//! transitions with exit times / conditions / triggers, cross-fades, 1D/2D/direct blend trees,
//! layers), samples its clips onto the transforms below it and fires clip events.
//!
//! Everything is in Unity space. Only "chain" nodes (bound transforms and their ancestors up to the
//! Animator) are evaluated; every other node inherits its nearest chain ancestor's world delta
//! (`W_now * W_rest^-1`), which is exact for unanimated descendants. The renderer reads
//! `Anim::delta_of(node)`.
//!
//! Gaps (counted, see `Anim::stats`): avatar masks, additive layers, non-transform bindings
//! (material / active / script fields), root motion, write-defaults off holding last values.
use crate::game::{Game, GameEvent};
use bevy_math::{EulerRot, Mat4, Quat, Vec3, Vec4};
use std::sync::Arc;
use uk_assets::anim::{cond, param, path_hash, BlendNode, Clip, Condition, Controller, Target, Transition, SELECTOR_BASE};
use uk_assets::scene::{to_bevy_point, to_bevy_quat};
use uk_assets::scenedef::{ControllerDef, SceneDef};

const POS: u8 = 0;
const ROT: u8 = 1;
const SCALE: u8 = 2;

/// A controller's animated properties: unique (path hash, kind) slots, and per clip which curve
/// feeds which slot.
struct CtrlRt {
    slots: Vec<(u32, u8)>,
    /// per `Controller::clips` index: (first curve, slot, curve is euler degrees)
    curves: Vec<Vec<(usize, usize, bool)>>,
    other_bindings: usize,
}

impl CtrlRt {
    fn build(cd: &ControllerDef, clips: &[Arc<Clip>]) -> CtrlRt {
        let mut slots: Vec<(u32, u8)> = Vec::new();
        let mut slot_of = std::collections::HashMap::new();
        let mut other_bindings = 0;
        let curves = cd
            .clips
            .iter()
            .map(|c| {
                let Some(clip) = c.map(|c| &clips[c as usize]) else { return Vec::new() };
                let mut out = Vec::new();
                for b in clip.float_bindings() {
                    let (kind, euler) = match b.target {
                        Target::Position => (POS, false),
                        Target::Rotation => (ROT, false),
                        Target::Euler => (ROT, true),
                        Target::Scale => (SCALE, false),
                        Target::Other { .. } => {
                            other_bindings += 1;
                            continue;
                        }
                    };
                    let slot = *slot_of.entry((b.path, kind)).or_insert_with(|| {
                        slots.push((b.path, kind));
                        slots.len() - 1
                    });
                    out.push((b.curve, slot, euler));
                }
                out
            })
            .collect();
        CtrlRt { slots, curves, other_bindings }
    }
}

struct Rig {
    animator: u32,
    ctrl: u32,
    node: u32,
    /// chain nodes, parents before children: (node, parent's chain index; None = outside the rig)
    chain: Vec<(u32, Option<u32>)>,
    /// rest local TRS of each chain node (Unity space)
    rest: Vec<(Vec3, Quat, Vec3)>,
    /// rest world inverse of each chain node (Unity space)
    rest_inv: Vec<Mat4>,
    /// controller slot -> chain index
    slot_chain: Vec<Option<u32>>,
}

#[derive(Clone, Debug, Default)]
pub struct Params {
    pub floats: Vec<f32>,
    pub ints: Vec<i32>,
    pub bools: Vec<bool>,
}

#[derive(Clone, Debug)]
pub struct Fade {
    pub dest: u32,
    pub time: f32,
    pub prev: f32,
    pub elapsed: f32,
    pub duration: f32,
}

#[derive(Clone, Debug)]
pub struct LayerState {
    pub state: u32,
    /// normalized time (unclamped: 2.5 = half way through the third loop)
    pub time: f32,
    pub prev: f32,
    pub fade: Option<Fade>,
}

/// Per-Animator runtime state; part of `game::State` so checkpoints restore it.
#[derive(Clone, Debug)]
pub struct AnimatorState {
    pub params: Params,
    pub layers: Vec<LayerState>,
    pub started: bool,
}

#[derive(Default, Clone, Debug)]
pub struct Stats {
    pub animators: usize,
    pub with_controller: usize,
    pub rigs: usize,
    pub slots: usize,
    pub slots_bound: usize,
    pub chain_nodes: usize,
    pub other_bindings: usize,
    pub additive_layers: usize,
    pub updated_last_frame: usize,
    pub transitions_fired: usize,
    pub events_fired: usize,
}

pub struct Anim {
    ctrls: Vec<Option<CtrlRt>>,
    rigs: Vec<Rig>,
    /// rest world matrix of every node (Unity space)
    w0u: Vec<Mat4>,
    /// per node: current world delta (valid for chain nodes)
    delta: Vec<Mat4>,
    /// per node: nearest chain ancestor-or-self (u32::MAX: not animated)
    src: Vec<u32>,
    pub stats: Stats,
    /// (animator, path hash) of bindings whose transform path matched nothing below the Animator
    pub unbound: Vec<(u32, u32)>,
    /// bumped whenever any delta changed (renderer skips uploads otherwise)
    pub version: u64,
    samples: Vec<f32>,
    acc: Vec<Vec4>,
    wsum: Vec<f32>,
    out: Vec<Vec4>,
    touched: Vec<bool>,
    local: Vec<(Vec3, Quat, Vec3)>,
    world: Vec<Mat4>,
}

fn unity(m: Mat4) -> Mat4 {
    let f = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    f * m * f
}

impl Anim {
    pub fn new(def: &SceneDef) -> (Anim, Vec<AnimatorState>) {
        let n = def.nodes.len();
        let w0u: Vec<Mat4> = def.nodes.iter().map(|nd| unity(nd.world0)).collect();
        let ctrls: Vec<Option<CtrlRt>> = def.controllers.iter().map(|c| Some(CtrlRt::build(c, &def.clips))).collect();
        let mut stats = Stats { animators: def.animators.len(), ..Default::default() };
        let mut rigs = Vec::new();
        let mut unbound = Vec::new();
        let mut states = Vec::new();
        let mut is_chain = vec![false; n];
        // outer Animators first so inner rigs see their parents' current worlds
        let depth = |mut x: u32| {
            let mut d = 0;
            while let Some(p) = def.nodes[x as usize].parent {
                d += 1;
                x = p;
            }
            d
        };
        let mut order: Vec<u32> = (0..def.animators.len() as u32).collect();
        order.sort_by_key(|&a| depth(def.animators[a as usize].node));
        for a in order {
            let ad = &def.animators[a as usize];
            let Some(ci) = ad.controller else { continue };
            stats.with_controller += 1;
            let ctrl = &def.controllers[ci as usize].ctrl;
            let rt = ctrls[ci as usize].as_ref().unwrap();
            stats.other_bindings += rt.other_bindings;
            stats.additive_layers += ctrl.layers.iter().filter(|l| l.blending == 1).count();
            // Transform paths are relative to the avatar root: the Animator itself, or (Animator
            // moved onto a parent of the imported model, like every enemy prefab) the descendant
            // the avatar's hierarchy hangs from. Pick the candidate binding the most slots.
            let mut best: (usize, std::collections::HashMap<u32, u32>) = (0, Default::default());
            let mut cands = vec![(ad.node, 0)];
            let mut ci_ = 0;
            while ci_ < cands.len() {
                let (root, d) = cands[ci_];
                ci_ += 1;
                let by_hash = paths(def, root);
                let hit = rt.slots.iter().filter(|(h, _)| by_hash.contains_key(h)).count();
                if hit > best.0 || ci_ == 1 {
                    best = (hit, by_hash);
                }
                if best.0 == rt.slots.len() {
                    break;
                }
                if d < 4 {
                    cands.extend(def.nodes[root as usize].children.iter().filter(|&&c| !def.nodes[c as usize].children.is_empty()).map(|&c| (c, d + 1)));
                }
            }
            let by_hash = best.1;
            let slot_node: Vec<Option<u32>> = rt.slots.iter().map(|(h, _)| by_hash.get(h).copied()).collect();
            for (i, s) in slot_node.iter().enumerate() {
                if s.is_none() {
                    unbound.push((a, rt.slots[i].0));
                }
            }
            stats.slots += slot_node.len();
            stats.slots_bound += slot_node.iter().flatten().count();
            // chain: bound nodes and their ancestors up to the Animator, top-down
            let mut members: Vec<u32> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for &sn in slot_node.iter().flatten() {
                let mut x = sn;
                loop {
                    if !seen.insert(x) {
                        break;
                    }
                    members.push(x);
                    if x == ad.node {
                        break;
                    }
                    match def.nodes[x as usize].parent {
                        Some(p) => x = p,
                        None => break,
                    }
                }
            }
            if members.is_empty() {
                continue;
            }
            members.sort_by_key(|&m| depth(m));
            let at: std::collections::HashMap<u32, u32> = members.iter().enumerate().map(|(i, &m)| (m, i as u32)).collect();
            let chain: Vec<(u32, Option<u32>)> = members.iter().map(|&m| (m, def.nodes[m as usize].parent.and_then(|p| at.get(&p).copied()))).collect();
            for &m in &members {
                is_chain[m as usize] = true;
            }
            let rest = members
                .iter()
                .map(|&m| {
                    let nd = &def.nodes[m as usize];
                    (to_bevy_point(nd.local_pos), to_bevy_quat(nd.local_rot), nd.local_scale)
                })
                .collect();
            let rest_inv = members.iter().map(|&m| w0u[m as usize].inverse()).collect();
            let slot_chain = slot_node.iter().map(|s| s.and_then(|s| at.get(&s).copied())).collect();
            stats.chain_nodes += members.len();
            states.push(initial(ctrl));
            rigs.push(Rig { animator: a, ctrl: ci, node: ad.node, chain, rest, rest_inv, slot_chain });
        }
        stats.rigs = rigs.len();
        // nearest chain ancestor-or-self of every node
        let mut src = vec![u32::MAX; n];
        let mut stack: Vec<(u32, u32)> = (0..n as u32).filter(|&i| def.nodes[i as usize].parent.is_none()).map(|r| (r, u32::MAX)).collect();
        while let Some((nd, up)) = stack.pop() {
            let s = if is_chain[nd as usize] { nd } else { up };
            src[nd as usize] = s;
            for &c in &def.nodes[nd as usize].children {
                stack.push((c, s));
            }
        }
        let anim = Anim {
            ctrls,
            rigs,
            w0u,
            delta: vec![Mat4::IDENTITY; n],
            src,
            stats,
            unbound,
            version: 0,
            samples: Vec::new(),
            acc: Vec::new(),
            wsum: Vec::new(),
            out: Vec::new(),
            touched: Vec::new(),
            local: Vec::new(),
            world: Vec::new(),
        };
        (anim, states)
    }

    /// World delta (Unity space, `W_now * W_rest^-1`) to apply to `node`'s rest world matrix.
    pub fn delta_of(&self, node: u32) -> Mat4 {
        match self.src.get(node as usize) {
            Some(&s) if s != u32::MAX => self.delta[s as usize],
            _ => Mat4::IDENTITY,
        }
    }

    pub fn animated(&self, node: u32) -> bool {
        self.src.get(node as usize).is_some_and(|&s| s != u32::MAX)
    }

    /// Current world matrix (Unity space) of a node.
    pub fn world_of(&self, node: u32) -> Mat4 {
        self.delta_of(node) * self.w0u[node as usize]
    }

    /// Rig index of the Animator component at `animator` (index into `SceneDef::animators`).
    pub fn rig_of_animator(&self, animator: u32) -> Option<usize> {
        self.rigs.iter().position(|r| r.animator == animator)
    }

    /// (Animator index, controller index) of a rig.
    pub fn rig_info(&self, rig: usize) -> (u32, u32) {
        (self.rigs[rig].animator, self.rigs[rig].ctrl)
    }

    pub fn rig_count(&self) -> usize {
        self.rigs.len()
    }
}

/// Path hash -> node for every transform below `root` ("" = root itself); first match wins like
/// Unity's binding.
fn paths(def: &SceneDef, root: u32) -> std::collections::HashMap<u32, u32> {
    use uk_assets::anim::crc_feed;
    let mut by_hash = std::collections::HashMap::new();
    // (node, running crc of its path, path empty)
    let mut stack = vec![(root, !0u32, true)];
    while let Some((nd, crc, empty)) = stack.pop() {
        by_hash.entry(if empty { 0 } else { !crc }).or_insert(nd);
        for &c in def.nodes[nd as usize].children.iter().rev() {
            let name = def.nodes[c as usize].name.as_bytes();
            stack.push((c, crc_feed(if empty { crc } else { crc_feed(crc, b"/") }, name), false));
        }
    }
    by_hash
}

fn initial(c: &Controller) -> AnimatorState {
    AnimatorState {
        params: Params { floats: c.floats.clone(), ints: c.ints.clone(), bools: c.bools.clone() },
        layers: c.layers.iter().map(|l| LayerState { state: c.machines.get(l.machine as usize).map_or(0, |m| m.default_state), time: 0.0, prev: 0.0, fade: None }).collect(),
        started: false,
    }
}

// ---- parameters ----------------------------------------------------------------------------

fn pf(c: &Controller, p: &Params, id: u32) -> f32 {
    match c.param(id) {
        Some(x) if x.kind == param::FLOAT => p.floats.get(x.index as usize).copied().unwrap_or(0.0),
        Some(x) if x.kind == param::INT => p.ints.get(x.index as usize).copied().unwrap_or(0) as f32,
        Some(x) => p.bools.get(x.index as usize).copied().unwrap_or(false) as i32 as f32,
        None => 0.0,
    }
}

fn pb(c: &Controller, p: &Params, id: u32) -> bool {
    pf(c, p, id) != 0.0
}

/// Setters used by scripts (`Animator.SetFloat/SetBool/SetTrigger/SetInteger` by name hash).
pub fn set_param(g: &mut Game, rig: usize, name: &str, value: f32) {
    let id = path_hash(name);
    let r = &g.anim.rigs[rig];
    let c = &g.def.controllers[r.ctrl as usize].ctrl;
    let Some(x) = c.param(id) else { return };
    let p = &mut g.s.anim[rig].params;
    match x.kind {
        param::FLOAT => {
            if let Some(v) = p.floats.get_mut(x.index as usize) {
                *v = value
            }
        }
        param::INT => {
            if let Some(v) = p.ints.get_mut(x.index as usize) {
                *v = value as i32
            }
        }
        _ => {
            if let Some(v) = p.bools.get_mut(x.index as usize) {
                *v = value != 0.0
            }
        }
    }
}

/// `Animator.Play(state)`: jump to a state of a layer by name or full path.
pub fn play(g: &mut Game, rig: usize, layer: usize, state: &str, time: f32) {
    let r = &g.anim.rigs[rig];
    let c = &g.def.controllers[r.ctrl as usize].ctrl;
    let h = path_hash(state);
    let Some(l) = c.layers.get(layer) else { return };
    let Some(m) = c.machines.get(l.machine as usize) else { return };
    if let Some(si) = m.states.iter().position(|s| s.name == h || s.full_path == h) {
        g.s.anim[rig].layers[layer] = LayerState { state: si as u32, time, prev: time, fade: None };
    }
}

// ---- state machine -------------------------------------------------------------------------

/// Leaf clips of a state with their blend weights: (controller clip index, weight).
fn leaves(c: &Controller, p: &Params, nodes: &[BlendNode], out: &mut Vec<(u32, f32)>) {
    fn walk(c: &Controller, p: &Params, nodes: &[BlendNode], i: usize, w: f32, out: &mut Vec<(u32, f32)>) {
        let Some(n) = nodes.get(i) else { return };
        if w <= 0.0 {
            return;
        }
        if n.children.is_empty() {
            if let Some(clip) = n.clip {
                out.push((clip, w));
            }
            return;
        }
        let k = n.children.len();
        let mut ws = vec![0.0f32; k];
        match n.kind {
            0 => {
                let x = pf(c, p, n.param);
                let t = &n.thresholds;
                if t.len() == k {
                    if x <= t[0] {
                        ws[0] = 1.0;
                    } else if x >= t[k - 1] {
                        ws[k - 1] = 1.0;
                    } else {
                        for j in 0..k - 1 {
                            if x >= t[j] && x <= t[j + 1] {
                                let f = (x - t[j]) / (t[j + 1] - t[j]).max(1e-6);
                                ws[j] = 1.0 - f;
                                ws[j + 1] = f;
                                break;
                            }
                        }
                    }
                } else {
                    ws[0] = 1.0;
                }
            }
            4 => {
                for (j, &id) in n.direct_params.iter().enumerate().take(k) {
                    ws[j] = pf(c, p, id).max(0.0);
                }
            }
            _ => {
                // gradient band interpolation (freeform cartesian; used for the directional kinds too)
                let x = [pf(c, p, n.param), pf(c, p, n.param_y)];
                let pos = &n.positions;
                if pos.len() == k {
                    for a in 0..k {
                        let mut h = 1.0f32;
                        for b in 0..k {
                            if a == b {
                                continue;
                            }
                            let ab = [pos[b][0] - pos[a][0], pos[b][1] - pos[a][1]];
                            let ax = [x[0] - pos[a][0], x[1] - pos[a][1]];
                            let l2 = ab[0] * ab[0] + ab[1] * ab[1];
                            if l2 > 1e-9 {
                                h = h.min(1.0 - (ax[0] * ab[0] + ax[1] * ab[1]) / l2);
                            }
                        }
                        ws[a] = h.max(0.0);
                    }
                    let s: f32 = ws.iter().sum();
                    if s > 0.0 {
                        ws.iter_mut().for_each(|w| *w /= s);
                    } else {
                        ws[0] = 1.0;
                    }
                } else {
                    ws[0] = 1.0;
                }
            }
        }
        for (j, &ch) in n.children.iter().enumerate() {
            walk(c, p, nodes, ch as usize, w * ws[j], out);
        }
    }
    walk(c, p, nodes, 0, 1.0, out);
}

struct Ctx<'a> {
    c: &'a Controller,
    cd: &'a ControllerDef,
    clips: &'a [Arc<Clip>],
    sync: usize,
}

impl Ctx<'_> {
    fn clip(&self, i: u32) -> Option<&Clip> {
        self.cd.clips.get(i as usize).copied().flatten().map(|c| &*self.clips[c as usize])
    }

    /// State length in seconds (blend-weighted leaf durations).
    fn length(&self, m: usize, s: u32, p: &Params, scratch: &mut Vec<(u32, f32)>) -> f32 {
        scratch.clear();
        let Some(st) = self.c.machines[m].states.get(s as usize) else { return 1.0 };
        if let Some(t) = st.tree(self.sync) {
            leaves(self.c, p, t, scratch);
        }
        let (mut len, mut w) = (0.0, 0.0);
        for &(ci, cw) in scratch.iter() {
            if let Some(cl) = self.clip(ci) {
                len += cl.duration() * cw;
                w += cw;
            }
        }
        if w > 0.0 && len > 1e-5 { len / w } else { 1.0 }
    }

    fn speed(&self, m: usize, s: u32, p: &Params) -> f32 {
        self.c.machines[m].states.get(s as usize).map_or(1.0, |st| st.speed * st.speed_param.map_or(1.0, |id| pf(self.c, p, id)))
    }
}

fn exit_reached(e: f32, prev: f32, now: f32) -> bool {
    if e >= 1.0 {
        now >= e
    } else {
        // checked every loop: some k + e in (prev, now]
        let k = (prev - e).floor() + 1.0;
        let x = k.max(0.0) + e;
        x > prev && x <= now
    }
}

fn conds_ok(c: &Controller, p: &Params, ks: &[Condition], prev: f32, now: f32) -> bool {
    ks.iter().all(|k| match k.mode {
        cond::IF => pb(c, p, k.param),
        cond::IF_NOT => !pb(c, p, k.param),
        cond::GREATER => pf(c, p, k.param) > k.threshold,
        cond::LESS => pf(c, p, k.param) < k.threshold,
        cond::EQUALS => pf(c, p, k.param) == k.threshold,
        cond::NOT_EQUAL => pf(c, p, k.param) != k.threshold,
        cond::EXIT_TIME => exit_reached(k.exit_time, prev, now),
        _ => false,
    })
}

fn transition_ok(c: &Controller, p: &Params, t: &Transition, prev: f32, now: f32) -> bool {
    if !t.has_exit_time && t.conditions.is_empty() {
        return false;
    }
    (!t.has_exit_time || exit_reached(t.exit_time, prev, now)) && conds_ok(c, p, &t.conditions, prev, now)
}

fn consume_triggers(c: &Controller, p: &mut Params, ks: &[Condition]) {
    for k in ks {
        if let Some(x) = c.param(k.param).filter(|x| x.kind == param::TRIGGER) {
            if let Some(b) = p.bools.get_mut(x.index as usize) {
                *b = false;
            }
        }
    }
}

/// Follow selector (sub-state-machine entry/exit) states to a real state.
fn resolve(c: &Controller, m: usize, p: &mut Params, mut dest: u32) -> Option<u32> {
    let mach = &c.machines[m];
    for _ in 0..16 {
        if dest < SELECTOR_BASE {
            return ((dest as usize) < mach.states.len()).then_some(dest);
        }
        let sel = mach.selectors.get((dest - SELECTOR_BASE) as usize)?;
        let next = sel.transitions.iter().find(|(d, ks)| *d != u32::MAX && conds_ok(c, p, ks, 0.0, 0.0)).map(|(d, ks)| (*d, ks.clone()));
        match next {
            Some((d, ks)) => {
                consume_triggers(c, p, &ks);
                dest = d;
            }
            None => return Some(mach.default_state),
        }
    }
    None
}

fn step_layer(x: &Ctx, m: usize, p: &mut Params, ls: &mut LayerState, dt: f32, scratch: &mut Vec<(u32, f32)>, fired: &mut usize) {
    let len = x.length(m, ls.state, p, scratch);
    ls.prev = ls.time;
    ls.time += dt * x.speed(m, ls.state, p) / len;
    if let Some(f) = &mut ls.fade {
        let dl = x.length(m, f.dest, p, scratch);
        f.prev = f.time;
        f.time += dt * x.speed(m, f.dest, p) / dl;
        f.elapsed += dt;
        if f.elapsed >= f.duration {
            *ls = LayerState { state: f.dest, time: f.time, prev: f.prev, fade: None };
        }
    }
    let mach = &x.c.machines[m];
    // candidate transitions: any-state, then the current state's (or, mid-fade, per interruption source)
    let mut pick: Option<(Transition, bool)> = None;
    match &ls.fade {
        None => {
            for t in &mach.any_state {
                if (t.to_self || t.dest != ls.state) && transition_ok(x.c, p, t, ls.prev, ls.time) {
                    pick = Some((t.clone(), false));
                    break;
                }
            }
            if pick.is_none() {
                if let Some(st) = mach.states.get(ls.state as usize) {
                    pick = st.transitions.iter().find(|t| transition_ok(x.c, p, t, ls.prev, ls.time)).map(|t| (t.clone(), false));
                }
            }
        }
        Some(f) => {
            let cur = mach.states.get(ls.state as usize).and_then(|s| s.transitions.iter().find(|t| t.dest == f.dest || t.full_path != 0)).map(|t| t.interruption).unwrap_or(0);
            let src_ok = matches!(cur, 1 | 3 | 4);
            let dst_ok = matches!(cur, 2 | 3 | 4);
            for t in &mach.any_state {
                if t.dest != f.dest && transition_ok(x.c, p, t, f.prev, f.time) {
                    pick = Some((t.clone(), true));
                    break;
                }
            }
            if pick.is_none() && src_ok {
                if let Some(st) = mach.states.get(ls.state as usize) {
                    pick = st.transitions.iter().find(|t| t.dest != f.dest && transition_ok(x.c, p, t, ls.prev, ls.time)).map(|t| (t.clone(), false));
                }
            }
            if pick.is_none() && dst_ok {
                if let Some(st) = mach.states.get(f.dest as usize) {
                    pick = st.transitions.iter().find(|t| transition_ok(x.c, p, t, f.prev, f.time)).map(|t| (t.clone(), true));
                }
            }
        }
    }
    let Some((t, from_dest)) = pick else { return };
    consume_triggers(x.c, p, &t.conditions);
    let Some(dest) = resolve(x.c, m, p, t.dest) else { return };
    *fired += 1;
    if from_dest {
        // interrupted by the destination side: it becomes the source (pose snap; Unity freezes the blend)
        if let Some(f) = ls.fade.take() {
            *ls = LayerState { state: f.dest, time: f.time, prev: f.prev, fade: None };
        }
    }
    let src_len = x.length(m, ls.state, p, scratch);
    let duration = if t.fixed_duration { t.duration } else { t.duration * src_len };
    if duration <= 1e-5 {
        *ls = LayerState { state: dest, time: t.offset, prev: t.offset, fade: None };
    } else {
        ls.fade = Some(Fade { dest, time: t.offset, prev: t.offset, elapsed: 0.0, duration });
    }
}

// ---- sampling ------------------------------------------------------------------------------

fn clip_time(cl: &Clip, nt: f32) -> f32 {
    let f = if cl.looping { nt.rem_euclid(1.0) } else { nt.clamp(0.0, 1.0) };
    cl.start + f * cl.duration()
}

/// Fire clip events crossed between normalized times `prev` and `now`.
fn events(cl: &Clip, prev: f32, now: f32, node: u32, out: &mut Vec<GameEvent>) -> usize {
    if cl.events.is_empty() || now <= prev {
        return 0;
    }
    let d = cl.duration().max(1e-6);
    let mut n = 0;
    for e in &cl.events {
        let fe = (e.time - cl.start) / d;
        let hit = if cl.looping {
            let k = (prev - fe).floor() + 1.0;
            let x = k + fe;
            x > prev && x <= now
        } else {
            fe > prev && fe <= now || (prev == 0.0 && fe == 0.0)
        };
        if hit {
            n += 1;
            out.push(GameEvent::AnimEvent { node, function: e.function.clone(), string: e.string.clone(), float: e.float, int: e.int });
        }
    }
    n
}

impl Anim {
    fn add_state(&mut self, x: &Ctx, rt: &CtrlRt, rig: usize, m: usize, s: u32, nt: f32, w: f32, p: &Params, scratch: &mut Vec<(u32, f32)>) {
        scratch.clear();
        let Some(st) = x.c.machines[m].states.get(s as usize) else { return };
        if let Some(t) = st.tree(x.sync) {
            leaves(x.c, p, t, scratch);
        }
        let nt = nt + st.cycle_offset;
        for &(ci, lw) in scratch.iter() {
            let Some(cl) = x.clip(ci) else { continue };
            cl.sample_into(clip_time(cl, nt), &mut self.samples);
            let ww = w * lw;
            for &(curve, slot, euler) in rt.curves.get(ci as usize).map_or(&[][..], |v| v.as_slice()) {
                if self.rigs[rig].slot_chain[slot].is_none() {
                    continue;
                }
                let sm = &self.samples;
                let v = match rt.slots[slot].1 {
                    ROT if euler => {
                        let r = |i: usize| sm.get(curve + i).copied().unwrap_or(0.0).to_radians();
                        let q = Quat::from_euler(EulerRot::YXZ, r(1), r(0), r(2));
                        Vec4::from(q.to_array())
                    }
                    ROT => {
                        let q = Vec4::new(sm[curve], sm[curve + 1], sm[curve + 2], sm[curve + 3]);
                        q.normalize_or(Vec4::W)
                    }
                    _ => Vec4::new(sm[curve], sm[curve + 1], sm[curve + 2], 0.0),
                };
                let v = if rt.slots[slot].1 == ROT && self.acc[slot].dot(v) < 0.0 { -v } else { v };
                self.acc[slot] += v * ww;
                self.wsum[slot] += ww;
            }
        }
    }
}

pub fn update(g: &mut Game, dt: f32) {
    let def = g.def.clone();
    let mut scratch = Vec::new();
    let mut evs = Vec::new();
    let (mut updated, mut fired, mut evn) = (0, 0, 0);
    let mut changed = false;
    for ri in 0..g.anim.rigs.len() {
        let (a, ci, node) = (g.anim.rigs[ri].animator, g.anim.rigs[ri].ctrl, g.anim.rigs[ri].node);
        let ad = &def.animators[a as usize];
        let cd = &def.controllers[ci as usize];
        let c = &*cd.ctrl;
        if !(ad.enabled && g.active(node)) {
            if g.s.anim[ri].started && !ad.keep_state_on_disable {
                g.s.anim[ri] = initial(c);
            }
            continue;
        }
        updated += 1;
        changed = true;
        let rt = g.anim.ctrls[ci as usize].take().unwrap();
        let st = &mut g.s.anim[ri];
        let first = !st.started;
        st.started = true;
        let nslots = rt.slots.len();
        let an = &mut g.anim;
        an.out.clear();
        an.out.resize(nslots, Vec4::ZERO);
        an.touched.clear();
        an.touched.resize(nslots, false);
        for (li, layer) in c.layers.iter().enumerate() {
            let lw = if li == 0 { 1.0 } else { layer.weight };
            if layer.machine as usize >= c.machines.len() {
                continue;
            }
            let x = Ctx { c, cd, clips: &def.clips, sync: layer.sync_index as usize };
            let m = layer.machine as usize;
            let mut ls = st.layers[li].clone();
            if !first {
                step_layer(&x, m, &mut st.params, &mut ls, dt, &mut scratch, &mut fired);
            }
            // zero-weight and additive layers still run their state machines
            if layer.blending == 1 || lw <= 0.0 {
                st.layers[li] = ls;
                continue;
            }
            // pose of this layer
            an.acc.clear();
            an.acc.resize(nslots, Vec4::ZERO);
            an.wsum.clear();
            an.wsum.resize(nslots, 0.0);
            let fw = ls.fade.as_ref().map_or(0.0, |f| (f.elapsed / f.duration.max(1e-6)).clamp(0.0, 1.0));
            an.add_state(&x, &rt, ri, m, ls.state, ls.time, 1.0 - fw, &st.params, &mut scratch);
            for &(cl, w) in scratch.clone().iter() {
                if let (Some(clip), true) = (x.clip(cl), w * (1.0 - fw) > 0.0) {
                    evn += events(clip, ls.prev, ls.time, node, &mut evs);
                }
            }
            if let Some(f) = ls.fade.clone() {
                an.add_state(&x, &rt, ri, m, f.dest, f.time, fw, &st.params, &mut scratch);
                for &(cl, w) in scratch.clone().iter() {
                    if let (Some(clip), true) = (x.clip(cl), w * fw > 0.0) {
                        evn += events(clip, f.prev, f.time, node, &mut evs);
                    }
                }
            }
            st.layers[li] = ls;
            let rig = &an.rigs[ri];
            for s in 0..nslots {
                let Some(chi) = rig.slot_chain[s] else { continue };
                if an.wsum[s] <= 0.0 {
                    continue;
                }
                // write defaults: the uncovered share of the weight is the rest value
                let (rp, rr, rs) = rig.rest[chi as usize];
                let kind = rt.slots[s].1;
                let rest = match kind {
                    POS => rp.extend(0.0),
                    ROT => Vec4::from(rr.to_array()),
                    _ => rs.extend(0.0),
                };
                let mut v = an.acc[s];
                if an.wsum[s] < 1.0 {
                    let r = if kind == ROT && v.dot(rest) < 0.0 { -rest } else { rest };
                    v += r * (1.0 - an.wsum[s]);
                } else {
                    v /= an.wsum[s];
                }
                if kind == ROT {
                    v = v.normalize_or(Vec4::W);
                }
                if !an.touched[s] {
                    an.out[s] = if lw >= 1.0 { v } else { rest.lerp(v, lw) };
                    an.touched[s] = true;
                } else {
                    let base = an.out[s];
                    let v = if kind == ROT && base.dot(v) < 0.0 { -v } else { v };
                    an.out[s] = base.lerp(v, lw);
                }
                if kind == ROT {
                    an.out[s] = an.out[s].normalize_or(Vec4::W);
                }
            }
        }
        // chain locals -> worlds -> deltas
        let rig = &an.rigs[ri];
        an.local.clear();
        an.local.extend(rig.rest.iter().copied());
        for s in 0..nslots {
            let (Some(chi), true) = (rig.slot_chain[s], an.touched[s]) else { continue };
            let v = an.out[s];
            let l = &mut an.local[chi as usize];
            match rt.slots[s].1 {
                POS => l.0 = v.truncate(),
                ROT => l.1 = Quat::from_vec4(v),
                _ => l.2 = v.truncate(),
            }
        }
        an.world.clear();
        for (k, &(nd, parent)) in rig.chain.iter().enumerate() {
            let pw = match parent {
                Some(pi) => an.world[pi as usize],
                None => match def.nodes[nd as usize].parent {
                    Some(p) => {
                        let s = an.src[p as usize];
                        let d = if s != u32::MAX { an.delta[s as usize] } else { Mat4::IDENTITY };
                        d * an.w0u[p as usize]
                    }
                    None => Mat4::IDENTITY,
                },
            };
            let (t, r, sc) = an.local[k];
            an.world.push(pw * Mat4::from_scale_rotation_translation(sc, r, t));
        }
        for (k, &(nd, _)) in rig.chain.iter().enumerate() {
            an.delta[nd as usize] = an.world[k] * rig.rest_inv[k];
        }
        an.ctrls[ci as usize] = Some(rt);
    }
    g.anim.stats.updated_last_frame = updated;
    g.anim.stats.transitions_fired += fired;
    g.anim.stats.events_fired += evn;
    if changed {
        g.anim.version += 1;
    }
    g.events.extend(evs);
}
