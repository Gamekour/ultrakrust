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
