//! Every AnimationClip in the game, decoded: curve counts must equal what the bindings consume,
//! sampled values must be finite, rotation curves must sample to unit quaternions (proves the
//! curve -> binding mapping), and streamed curves must be continuous across key frames (proves the
//! cubic segment decoding).
use crate::Report;
use std::path::Path;
use uk_assets::anim::{Clip, Controller, Target, SELECTOR_BASE};
use uk_assets::db::AssetDb;

pub fn all_clips(install: &Path, r: &mut Report) {
    let t0 = std::time::Instant::now();
    let mut db = AssetDb::open(install).unwrap();
    let mut bundles = Vec::new();
    let mut stack = vec![AssetDb::bundle_dir(install)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p)
            } else if p.extension().is_some_and(|x| x == "bundle") {
                bundles.push(p)
            }
        }
    }
    bundles.sort();
    let (mut clips, mut counts_ok, mut finite_ok, mut quat_ok, mut quats, mut events) = (0, 0, 0, 0, 0, 0);
    let (mut jumps, mut joints) = (0usize, 0usize);
    let (mut ctrls, mut ctrl_ok, mut states, mut named, mut transitions, mut blends) = (0, 0, 0, 0, 0, 0);
    let mut bad = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for b in &bundles {
        let Ok(files) = db.load_bundle_files(b) else { continue };
        for f in files {
            for o in f.objects.iter().filter(|o| o.class_id == 91) {
                if !seen.insert((f.name.clone(), o.path_id)) {
                    continue;
                }
                let Ok(v) = f.read(o) else { continue };
                let c = Controller::from_value(&v);
                ctrls += 1;
                // every internal reference must land: destinations, clips, parameters, names
                let mut why: Vec<String> = Vec::new();
                if c.layers.is_empty() || c.layers.iter().any(|l| l.machine as usize >= c.machines.len()) { why.push("layers".into()) }
                let pok = |id: u32| c.param(id).is_some_and(|p| match p.kind {
                    1 => (p.index as usize) < c.floats.len(),
                    3 => (p.index as usize) < c.ints.len(),
                    _ => (p.index as usize) < c.bools.len(),
                });
                for m in &c.machines {
                    let dest_ok = |d: u32| if d >= SELECTOR_BASE { ((d - SELECTOR_BASE) as usize) < m.selectors.len() } else { (d as usize) < m.states.len() };
                    if !(m.states.is_empty() || (m.default_state as usize) < m.states.len()) { why.push("default state".into()) }
                    for t in m.states.iter().flat_map(|s| &s.transitions).chain(&m.any_state) {
                        transitions += 1;
                        if !dest_ok(t.dest) { why.push(format!("dest {}", t.dest)) }
                        for k in t.conditions.iter().filter(|k| k.mode != 5 && !pok(k.param)) { why.push(format!("cond param {} mode {}", k.param, k.mode)) }
                    }
                    for s in &m.selectors {
                        if !s.transitions.iter().all(|(d, ks)| (*d == u32::MAX || dest_ok(*d)) && ks.iter().all(|k| pok(k.param))) { why.push("selector".into()) }
                    }
                    for s in &m.states {
                        states += 1;
                        named += !c.name(s.full_path).is_empty() as usize;
                        if !s.speed_param.is_none_or(pok) { why.push(format!("speed param {:?}", s.speed_param)) }
                        for n in s.trees.iter().flatten() {
                            if !n.clip.is_none_or(|i| (i as usize) < c.clips.len()) { why.push("clip".into()) }
                            if !n.children.iter().all(|&ch| s.trees.iter().any(|t| (ch as usize) < t.len())) { why.push("child".into()) }
                            if !n.children.is_empty() {
                                blends += 1;
                                if !match n.kind { 0 => pok(n.param), 4 => n.direct_params.iter().all(|&p| pok(p)), _ => pok(n.param) && pok(n.param_y) } { why.push(format!("blend kind {} param", n.kind)) }
                            }
                        }
                    }
                }
                if why.is_empty() {
                    ctrl_ok += 1;
                } else if bad.len() < 5 {
                    why.dedup();
                    bad.push(format!("controller {}: dangling {}", c.name, why.join(", ")));
                }
            }
            for o in f.objects.iter().filter(|o| o.class_id == 74) {
                if !seen.insert((f.name.clone(), o.path_id)) {
                    continue;
                }
                let Ok(v) = f.read(o) else { continue };
                if v.get("m_Legacy").bool() {
                    continue;
                }
                let c = Clip::from_value(&v);
                clips += 1;
                events += c.events.len();
                if c.curve_count() == c.bound_curves() {
                    counts_ok += 1;
                } else if bad.len() < 5 {
                    bad.push(format!("clip {} curves {} bound {}", c.name, c.curve_count(), c.bound_curves()));
                }
                let n = 8;
                let (mut fin, mut qok, mut qn) = (true, true, 0);
                for k in 0..=n {
                    let t = c.start + c.duration() * k as f32 / n as f32;
                    let s = c.sample(t);
                    fin &= s.iter().all(|x| x.is_finite());
                    for bd in c.bindings.iter().filter(|b| b.target == Target::Rotation) {
                        if let Some(q) = s.get(bd.curve..bd.curve + 4) {
                            qn += 1;
                            let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
                            // per-component cubics drift off unit mid-segment in fast swings (Unity
                            // normalizes after sampling); a wrong curve mapping is far off
                            qok &= (len - 1.0).abs() < 0.15;
                        }
                    }
                }
                // continuity across interior key frames: value just before vs just after
                for ft in c.frame_times().filter(|t| *t > c.start && *t < c.stop) {
                    let (a, b) = (c.sample(ft - 1e-4), c.sample(ft + 1e-4));
                    joints += 1;
                    jumps += a.iter().zip(&b).take(c.streamed_curves()).any(|(x, y)| (x - y).abs() > 0.05 * (1.0 + x.abs())) as usize;
                }
                finite_ok += fin as usize;
                if qn > 0 {
                    quats += 1;
                    quat_ok += qok as usize;
                }
            }
        }
    }
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    r.higher("anim.clips", clips as f64);
    r.higher("anim.curve_count_match_pct", pct(counts_ok, clips));
    r.higher("anim.finite_pct", pct(finite_ok, clips));
    r.higher("anim.unit_quat_pct", pct(quat_ok, quats));
    r.higher("anim.streamed_continuous_pct", pct(joints - jumps, joints));
    r.info("anim.events", events as f64);
    r.higher("anim.controllers", ctrls as f64);
    r.higher("anim.controller_refs_ok_pct", pct(ctrl_ok, ctrls));
    r.higher("anim.state_names_pct", pct(named, states));
    r.info("anim.states", states as f64);
    r.info("anim.transitions", transitions as f64);
    r.info("anim.blend_trees", blends as f64);
    r.lower("perf.anim.decode_all_s", t0.elapsed().as_secs_f64(), 0.5);
    for b in bad {
        r.note(format!("gap: anim {b}"));
    }
}

/// Mecanim runtime over one built level: binding resolution, how much moves, finiteness, cost.
#[derive(Default)]
pub struct RtTotals {
    pub animators: usize,
    pub with_controller: usize,
    pub rigs: usize,
    pub slots: usize,
    pub slots_bound: usize,
    pub chain_nodes: usize,
    pub moved_nodes: usize,
    pub nonfinite: usize,
    pub other_bindings: usize,
    pub additive_layers: usize,
    pub transitions: usize,
    pub events: usize,
    pub skinned: usize,
    pub skinned_animated: usize,
    pub frame_ms: f64,
    pub frames: usize,
}

pub fn runtime(g: &mut uk_game::Game, frames: usize, t: &mut RtTotals) {
    let t0 = std::time::Instant::now();
    for _ in 0..frames {
        uk_game::anim::update(g, 1.0 / 60.0);
    }
    t.frame_ms += t0.elapsed().as_secs_f64() * 1e3;
    t.frames += frames;
    let s = &g.anim.stats;
    t.animators += s.animators;
    t.with_controller += s.with_controller;
    t.rigs += s.rigs;
    t.slots += s.slots;
    t.slots_bound += s.slots_bound;
    t.chain_nodes += s.chain_nodes;
    t.other_bindings += s.other_bindings;
    t.additive_layers += s.additive_layers;
    t.transitions += s.transitions_fired;
    t.events += s.events_fired;
    for n in 0..g.def.nodes.len() as u32 {
        if !g.anim.animated(n) {
            continue;
        }
        let d = g.anim.delta_of(n);
        if !d.is_finite() {
            t.nonfinite += 1;
        } else if !d.abs_diff_eq(bevy_math::Mat4::IDENTITY, 1e-3) {
            t.moved_nodes += 1;
        }
    }
    for rd in &g.def.renderers {
        if let Some(sk) = &rd.skin {
            t.skinned += 1;
            t.skinned_animated += sk.bones.iter().flatten().any(|&b| g.anim.animated(b)) as usize;
        }
    }
}

pub fn report_runtime(t: &RtTotals, r: &mut Report) {
    let pct = |a: usize, b: usize| 100.0 * a as f64 / b.max(1) as f64;
    r.info("animrt.animators", t.animators as f64);
    r.higher("animrt.with_controller", t.with_controller as f64);
    r.higher("animrt.rigs", t.rigs as f64);
    r.higher("animrt.slots_bound_pct", pct(t.slots_bound, t.slots));
    r.info("animrt.chain_nodes", t.chain_nodes as f64);
    r.higher("animrt.moved_nodes", t.moved_nodes as f64);
    r.pass("animrt.finite", t.nonfinite == 0, format!("{} animated nodes with non-finite deltas", t.nonfinite));
    r.info("animrt.unhandled_bindings", t.other_bindings as f64);
    r.info("animrt.additive_layers_skipped", t.additive_layers as f64);
    r.info("animrt.transitions_fired", t.transitions as f64);
    r.info("animrt.events_fired", t.events as f64);
    r.higher("animrt.skinned_animated_pct", pct(t.skinned_animated, t.skinned));
    r.lower("perf.animrt.frame_ms", t.frame_ms / t.frames.max(1) as f64, 0.5);
}

/// Enemy attacks timed by their animation: one Filth and one Stray of 0-1, switched on with the
/// player in reach, must play Attack, fire the clip events that drive the attack (DamageStart /
/// SwingEnd, ThrowProjectile) and finish it from SwingEnd.
pub fn enemy_attacks(def: &std::sync::Arc<uk_assets::scenedef::SceneDef>, r: &mut Report) {
    use uk_game::enemy::Kind;
    use uk_game::game::GameEvent;
    let base = uk_game::Game::new(def.clone());
    r.higher("animrt.0-1.enemy_rig_pct", 100.0 * base.s.enemies.iter().filter(|e| e.rig.is_some()).count() as f64 / base.s.enemies.len().max(1) as f64);
    for (kind, reach, key, want) in [(Kind::Filth, 6.0, "filth", "DamageStart"), (Kind::Stray, 25.0, "stray", "ThrowProjectile")] {
        let Some(i) = base.s.enemies.iter().position(|e| e.kind == kind && e.rig.is_some()) else {
            r.pass(&format!("animrt.0-1.{key}_attack"), false, "no rigged enemy");
            continue;
        };
        let mut g = uk_game::Game::new(def.clone());
        let mut n = Some(g.s.enemies[i].node);
        while let Some(x) = n {
            g.set_active(x, true);
            n = def.nodes[x as usize].parent;
        }
        let en = &g.s.enemies[i];
        let fwd = bevy_math::Vec3::new(en.yaw.sin(), 0.0, -en.yaw.cos());
        g.s.player.pos = en.pos + fwd * reach + bevy_math::Vec3::Y;
        g.s.player.prev_pos = g.s.player.pos;
        let anode = def.animators[g.anim.rig_info(en.rig.unwrap()).0 as usize].node;
        let (mut t, mut fired, mut ends, mut was) = (0.0f64, 0, 0, false);
        for _ in 0..900 {
            let inp = uk_core::player::Input::default();
            g.fixed_update(&inp);
            t += uk_core::consts::FIXED_DT as f64;
            g.update(&inp, uk_core::consts::FIXED_DT, t);
            for e in g.events.drain(..) {
                if let GameEvent::AnimEvent { node, function, .. } = e {
                    fired += (node == anode && function == want) as usize;
                }
            }
            g.s.hp = 100;
            g.s.player.events.clear();
            let a = g.s.enemies[i].attacking;
            ends += (was && !a) as usize;
            was = a;
        }
        r.pass(&format!("animrt.0-1.{key}_attack"), fired >= 2 && ends >= 2, format!("{fired} {want} events, {ends} attacks finished"));
    }
}
