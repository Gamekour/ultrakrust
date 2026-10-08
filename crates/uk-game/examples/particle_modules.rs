//! Every scene ParticleSystem of the matching levels, played standalone at its node for a few
//! seconds against the level's colliders: emitter shapes checked against their bounds (in shape
//! space) and per-module totals (systems, particles, collisions, non-finite values, cost).
//! cargo run --release -p uk-game --example particle_modules -- [level filter] [seconds]
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::particles::{self, Env, Rng, System};
use uk_game::Game;

#[derive(Default)]
struct Tally {
    systems: usize,
    particles: usize,
    collisions: u64,
    nonfinite: usize,
    us: f64,
}

/// Shape-space bound check for one sample; None when the shape has no closed-form bound.
fn shape_ok(sh: &uk_assets::particles::Shape, p: Vec3, d: Vec3) -> Option<bool> {
    let q = particles::unity_euler(sh.rotation);
    let inv_s = Vec3::new(1.0 / nz(sh.scale.x), 1.0 / nz(sh.scale.y), 1.0 / nz(sh.scale.z));
    let l = (q.inverse() * (p - sh.position)) * inv_s;
    let e = 1e-3;
    let free = sh.random_position > 0.0;
    let dir_free = sh.random_direction > 0.0 || sh.spherical_direction > 0.0;
    let ld = (q.inverse() * d).normalize_or_zero();
    let plus_z = dir_free || sh.scale.min_element() <= 0.0 || ld.z > 0.0;
    Some(
        free || match sh.kind {
            5 => l.abs().max_element() <= 0.5 + e && plus_z,
            15 => l.abs().max_element() <= 0.5 + e && l.abs().max_element() >= 0.5 - e && plus_z,
            16 => {
                let a = l.abs();
                a.max_element() <= 0.5 + e && [a.x, a.y, a.z].iter().filter(|v| **v >= 0.5 - e).count() >= 2 && plus_z
            }
            18 => l.z.abs() <= e && l.x.abs() <= 0.5 + e && l.y.abs() <= 0.5 + e && plus_z,
            12 => l.y.abs() <= e && l.z.abs() <= e && l.x.abs() <= sh.radius + e,
            17 => {
                let ring = Vec3::new(l.x, l.y, 0.0).normalize_or_zero() * sh.radius;
                l.distance(ring) <= sh.donut_radius + e
            }
            8 | 9 => {
                let r = sh.radius + l.z.max(0.0) * sh.angle.to_radians().tan();
                l.z >= -e && l.z <= sh.length + e && Vec3::new(l.x, l.y, 0.0).length() <= r + e
            }
            0 | 1 => l.length() <= sh.radius + e,
            2 | 3 => l.length() <= sh.radius + e && l.z >= -e,
            4 | 7 | 10 | 11 => l.z.abs() <= e && Vec3::new(l.x, l.y, 0.0).length() <= sh.radius + e,
            _ => return None,
        },
    )
}

fn nz(v: f32) -> f32 {
    if v.abs() < 1e-6 { 1e-6 } else { v }
}

use bevy_math::Vec3;

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let secs: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(3.0);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.starts_with("campaign_scenes_") && n.contains(&*filter)
        })
        .collect();
    bundles.sort();
    let mut by_feature: BTreeMap<String, Tally> = BTreeMap::new();
    // shape kind -> (samples, out of bounds, unchecked)
    let mut shapes: BTreeMap<u8, (usize, usize, usize)> = BTreeMap::new();
    let mut bad_examples: Vec<String> = Vec::new();
    // renderer settings in use: "mode/alignment", sort modes, custom vertex streams
    let mut renderers: BTreeMap<String, usize> = BTreeMap::new();
    let mut nonfinite: Vec<String> = Vec::new();
    let mut rng = Rng::default();
    let dt = 1.0 / 60.0;
    for b in &bundles {
        let name = b.file_name().unwrap().to_string_lossy().replace("campaign_scenes_", "").replace(".bundle", "");
        let Ok(def) = scenedef::load_scene(&mut db, b) else { continue };
        let def = Arc::new(def);
        let g = Game::new(def.clone());
        let ray = |o: Vec3, d: Vec3, len: f32, mask: u32| -> Option<(f32, Vec3)> {
            let h = g.world.raycast_filtered(particles::to_bevy(o), particles::to_bevy(d), len, |id| {
                let o = g.world.owner(id);
                o == uk_core::collide::ALWAYS || def.colliders.get(o as usize).is_some_and(|c| !c.trigger && mask & (1 << c.layer) != 0)
            })?;
            Some((h.distance, particles::to_unity(h.normal)))
        };
        let env = Env { ray: Some(&ray) };
        for p in &def.particle_systems {
            if let Some(r) = p.renderer.as_ref().filter(|r| r.enabled) {
                *renderers.entry(format!("mode {} align {}", r.render_mode, r.render_alignment)).or_default() += 1;
                *renderers.entry(format!("sort {}", r.sort_mode)).or_default() += 1;
                *renderers.entry(format!("streams {:?}", r.vertex_streams)).or_default() += 1;
                if r.pivot != Vec3::ZERO || r.flip != Vec3::ZERO {
                    *renderers.entry("pivot / flip".into()).or_default() += 1;
                }
                if r.render_mode == 1 && r.camera_velocity_scale != 0.0 {
                    *renderers.entry("camera velocity scale".into()).or_default() += 1;
                }
            }
            let mut s = System::new(p.node, p.system.clone());
            let w = g.node_world_now(p.node);
            s.set_transform(w, def.nodes[p.node as usize].local_scale);
            s.shape_xf = p.shape_node.map(|n| s.emitter.inverse() * g.node_world_now(n));
            if let Some(sh) = &s.def.shape {
                let ent = shapes.entry(sh.kind).or_default();
                for _ in 0..64 {
                    let (pp, dd) = particles::shape_sample(&s, &mut rng);
                    ent.0 += 1;
                    match shape_ok(sh, pp, dd) {
                        Some(true) => {}
                        Some(false) => {
                            ent.1 += 1;
                            if bad_examples.len() < 12 {
                                bad_examples.push(format!("{name} {} kind {} p {pp:?} d {dd:?}", def.path(p.node), sh.kind));
                            }
                        }
                        None => ent.2 += 1,
                    }
                    if !pp.is_finite() || !dd.is_finite() {
                        ent.1 += 1;
                    }
                }
            }
            s.planes = p
                .planes
                .iter()
                .map(|&n| {
                    let m = g.node_world_now(n);
                    (m.w_axis.truncate(), m.y_axis.truncate().normalize_or(Vec3::Y))
                })
                .collect();
            s.play(&mut rng);
            let t0 = std::time::Instant::now();
            let mut peak = 0;
            for _ in 0..(secs / dt) as usize {
                particles::step_system(&mut s, dt, &mut rng, &env);
                peak = peak.max(s.particles.len());
            }
            let us = t0.elapsed().as_secs_f64() * 1e6 / (secs / dt) as f64;
            let bad = s.particles.iter().filter(|q| !(q.pos.is_finite() && q.vel.is_finite() && q.rot.is_finite() && q.size3(&s.def).is_finite())).count()
                + s.particles.iter().filter(|q| q.sheet_uv(&s.def).is_some_and(|r| r.iter().any(|v| !v.is_finite()))).count();
            if bad > 0 && nonfinite.len() < 10 {
                let q = s.particles.iter().find(|q| !(q.pos.is_finite() && q.vel.is_finite() && q.rot.is_finite() && q.size3(&s.def).is_finite())).or(s.particles.first()).unwrap();
                nonfinite.push(format!("{name} {}: {bad} pos {:?} vel {:?} rot {:?} size {:?} uv {:?} emitter {:?}", def.path(p.node), q.pos, q.vel, q.rot, q.size3(&s.def), q.sheet_uv(&s.def), s.emitter));
            }
            let d = &s.def;
            let mut feats: Vec<String> = vec!["all".into()];
            let mut f = |on: bool, n: &str| {
                if on {
                    feats.push(n.into())
                }
            };
            f(d.noise.is_some(), "noise");
            f(d.collision.as_ref().is_some_and(|c| c.kind == 1), "collision world");
            f(d.collision.as_ref().is_some_and(|c| c.kind == 0), "collision planes");
            f(d.limit_velocity.is_some(), "limit velocity");
            f(d.force.is_some(), "force");
            f(d.inherit_velocity.is_some(), "inherit velocity");
            f(d.lifetime_by_emitter_speed.is_some(), "lifetime by emitter speed");
            f(d.rotation_by_speed.is_some(), "rotation by speed");
            f(d.texture_sheet.is_some(), "texture sheet");
            f(d.velocity_over_lifetime.as_ref().is_some_and(|v| v.orbital.iter().any(|c| !c.is_const(0.0)) || !v.radial.is_const(0.0)), "orbital / radial");
            f(d.size3d, "size3D");
            f(d.rotation3d, "rotation3D");
            f(d.size_axes.is_some(), "size separate axes");
            f(d.rotation_axes.is_some(), "rotation separate axes");
            for u in &d.unsupported {
                feats.push(format!("UNSUPPORTED {u}"));
            }
            for k in feats {
                let t = by_feature.entry(k).or_default();
                t.systems += 1;
                t.particles += peak;
                t.collisions += s.collisions;
                t.nonfinite += bad;
                t.us += us;
            }
        }
        println!("{name}: {} systems", def.particle_systems.len());
    }
    println!("\nshape kind: samples, out of bounds, unchecked");
    for (k, (n, bad, un)) in &shapes {
        println!("  {k:2}: {n:7} {bad:5} {un:7}");
    }
    for b in &bad_examples {
        println!("  bad: {b}");
    }
    for n in &nonfinite {
        println!("  nonfinite: {n}");
    }
    println!("\nrenderers:");
    for (k, n) in &renderers {
        println!("  {k}: {n}");
    }
    println!("\nfeature: systems, peak particles (sum), collisions, non-finite, mean us per step");
    for (k, t) in &by_feature {
        println!("  {k}: {} {} {} {} {:.1}", t.systems, t.particles, t.collisions, t.nonfinite, t.us / t.systems.max(1) as f64);
    }
}
