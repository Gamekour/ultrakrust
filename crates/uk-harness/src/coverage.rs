//! Whole-install coverage: every scene must load; how many script instances and native components have a port.
use crate::Report;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Instant;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::parity::{native_status, status, Port};

pub fn all_scenes(install: &Path, r: &mut Report) {
    let mut db = AssetDb::open(install).unwrap();
    let dir = AssetDb::bundle_dir(install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().contains("_scenes_"))
        .collect();
    bundles.sort();
    let (mut loaded, mut total, mut ported, mut full) = (0, 0usize, 0usize, 0usize);
    let mut games_built = 0;
    let mut classes = BTreeSet::new();
    let mut native: BTreeMap<i32, usize> = BTreeMap::new();
    let mut load_ms = 0.0;
    let mut rt = crate::anim::RtTotals::default();
    for b in &bundles {
        let name = b.file_name().unwrap().to_string_lossy().to_string();
        let t = Instant::now();
        match scenedef::load_scene(&mut db, b) {
            Ok(def) => {
                loaded += 1;
                load_ms += t.elapsed().as_secs_f64() * 1e3;
                let short = name.split(".bundle").next().unwrap_or(&name).replace("campaign_scenes_", "").replace("specialscenes_scenes_", "");
                let short = short.split('_').next().unwrap_or(&short).to_string();
                // the level runtime must build for every scene, and every baked navmesh must check out
                let def = std::sync::Arc::new(def);
                let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| uk_game::Game::new(def.clone())));
                match built {
                    Ok(mut g) => {
                        games_built += 1;
                        if g.nav.is_some() {
                            crate::checks::nav_checks(&short, &g, r);
                        }
                        crate::anim::runtime(&mut g, 120, &mut rt);
                    }
                    Err(_) => r.note(format!("FAIL game build {short}: panicked")),
                }
                // Render geometry orientation: share of triangles whose stored normals point against the
                // front face Bevy's CCW culling sees. ~0 when winding is right; authored double-sided
                // cards (5-s tree leaves) keep some levels above zero.
                let (mut agree, mut against) = (0usize, 0usize);
                for rd in &def.renderers {
                    let b = &rd.batch;
                    for t in b.indices.chunks_exact(3) {
                        let v = |i: u32| bevy_math::Vec3::from(b.positions[i as usize]);
                        let n = |i: u32| bevy_math::Vec3::from(b.normals[i as usize]);
                        let face = (v(t[1]) - v(t[0])).cross(v(t[2]) - v(t[0]));
                        let vn = n(t[0]) + n(t[1]) + n(t[2]);
                        if face.length_squared() < 1e-12 || vn.length_squared() < 1e-6 {
                            continue;
                        }
                        if face.dot(vn) >= 0.0 { agree += 1 } else { against += 1 }
                    }
                }
                r.lower(&format!("render.{short}.tris_against_normals_pct"), 100.0 * against as f64 / (agree + against).max(1) as f64, 0.5);
                for s in def.scripts.iter().filter(|s| !s.class.is_empty()) {
                    total += 1;
                    match status(&s.class) {
                        Some(Port::Full) => {
                            ported += 1;
                            full += 1
                        }
                        Some(_) => ported += 1,
                        None => {}
                    }
                    classes.insert(s.class.clone());
                }
            }
            Err(e) => r.note(format!("FAIL load {name}: {e}")),
        }
        if let Ok(files) = db.load_bundle_files(b) {
            for f in files {
                for o in &f.objects {
                    *native.entry(o.class_id).or_default() += 1
                }
            }
        }
    }
    r.higher("cov.scenes_loading", loaded as f64);
    r.pass("all.games_build", games_built == loaded, format!("{games_built}/{loaded} scenes build a Game"));
    crate::anim::report_runtime(&rt, r);
    r.info("cov.scenes_total", bundles.len() as f64);
    r.pass("all.scenes_load", loaded == bundles.len(), format!("{loaded}/{} scenes load", bundles.len()));
    r.lower("perf.all.scene_load_ms_total", load_ms, 0.35);
    r.higher("cov.script_instances_ported_pct", 100.0 * ported as f64 / total.max(1) as f64);
    r.higher("cov.script_instances_full_pct", 100.0 * full as f64 / total.max(1) as f64);
    r.higher("cov.script_classes_ported", classes.iter().filter(|c| status(c).is_some()).count() as f64);
    r.info("cov.script_classes_total", classes.len() as f64);
    // Unweighted mean over the engine systems present in the install: weighting by instance count would
    // let 450k Transforms hide that audio, animation, particles and UI are at zero.
    let present: Vec<f64> = native.keys().filter_map(|id| native_status(*id)).collect();
    r.higher("cov.native_systems_mean_pct", 100.0 * present.iter().sum::<f64>() / present.len().max(1) as f64);
    r.info("cov.native_systems_tracked", present.len() as f64);
}
