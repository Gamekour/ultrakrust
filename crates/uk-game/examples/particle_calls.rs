//! Persistent UnityEvent calls whose target is a scene ParticleSystem (Play / Stop / Clear /
//! Emit ...), per level, with the calling script.
//! cargo run --release -p uk-game --example particle_calls -- [level filter]
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::serialized::Value;
use uk_assets::{db::AssetDb, scenedef};

fn walk(v: &Value, out: &mut Vec<(i64, String)>) {
    match v {
        Value::Struct(f) => {
            if let Some((_, calls)) = f.iter().find(|(k, _)| &**k == "m_PersistentCalls") {
                for c in calls.get("m_Calls").array() {
                    out.push((c.get("m_Target").pptr().1, c.get("m_MethodName").str().to_string()));
                }
            }
            for (_, x) in f {
                walk(x, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
        _ => {}
    }
}

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| {
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        n.starts_with("campaign_scenes_level") && n.contains(&filter)
    }).collect();
    bundles.sort();
    let mut total: BTreeMap<String, usize> = BTreeMap::new();
    for b in bundles {
        let Ok(def) = scenedef::load_scene(&mut db, &b) else { continue };
        let def = Arc::new(def);
        let ps: std::collections::HashMap<i64, u32> = def.particle_systems.iter().map(|p| (p.path_id, p.node)).collect();
        let mut found: BTreeMap<String, usize> = BTreeMap::new();
        for s in &def.scripts {
            let mut out = Vec::new();
            walk(&s.data, &mut out);
            for (id, m) in out {
                if ps.contains_key(&id) {
                    *found.entry(format!("{}: ParticleSystem.{m}", s.class)).or_default() += 1;
                }
            }
        }
        println!("{}: {found:?}", b.file_name().unwrap().to_string_lossy());
        for (k, v) in found {
            *total.entry(k).or_default() += v;
        }
    }
    println!("total {total:?}");
}
