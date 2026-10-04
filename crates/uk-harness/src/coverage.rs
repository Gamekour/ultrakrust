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
    let mut classes = BTreeSet::new();
    let mut native: BTreeMap<i32, usize> = BTreeMap::new();
    let mut load_ms = 0.0;
    for b in &bundles {
        let name = b.file_name().unwrap().to_string_lossy().to_string();
        let t = Instant::now();
        match scenedef::load_scene(&mut db, b) {
            Ok(def) => {
                loaded += 1;
                load_ms += t.elapsed().as_secs_f64() * 1e3;
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
