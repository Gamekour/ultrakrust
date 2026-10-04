//! List UnityEvent persistent calls whose target is not a ported script/collider: target class, method, where.
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::scripts::{parse_calls, Target};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let files = db.load_bundle_files(&path).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut seen = std::collections::BTreeMap::<String, usize>::new();
    for s in &def.scripts {
        // every UnityEvent-shaped field anywhere in the script
        fn walk(v: &uk_assets::serialized::Value, out: &mut Vec<uk_assets::serialized::Value>) {
            if v.has("m_PersistentCalls") { out.push(v.clone()); return }
            if let uk_assets::serialized::Value::Struct(m) = v { for (_, x) in m { walk(x, out) } }
            if let uk_assets::serialized::Value::Array(a) = v { for x in a { walk(x, out) } }
        }
        let mut evs = Vec::new(); walk(&s.data, &mut evs);
        for e in evs {
            for (c, raw) in parse_calls(&def, &e).into_iter().zip(e.get("m_PersistentCalls").get("m_Calls").array()) {
                let (_, id) = raw.get("m_Target").pptr();
                let cls = files.iter().find_map(|f| f.object(id).map(|o| o.class_id)).unwrap_or(-1);
                let k = match c.target { Target::Script(t) => format!("{}.{}", def.scripts[t as usize].class, c.method), Target::Collider(_) => format!("Collider.{}", c.method), Target::Node(_) => format!("native#{cls}.{}", c.method), Target::None => format!("none.{}", c.method) };
                *seen.entry(k).or_default() += 1;
            }
        }
    }
    for (k, n) in seen { println!("{n:5} {k}") }
}
