//! Every instance of the given MonoBehaviour classes across the install's scene bundles: scene, path, fields.
//! find_class Class1,Class2 [bundle filter]  (FIND_SUBTREE=1 also lists every script under each match;
//! class `*` with FIND_TEXT=<s> matches every script whose fields contain s; FIND_FULL=1 prints all fields)
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let want: Vec<String> = std::env::args().nth(1).expect("classes").split(',').map(str::to_string).collect();
    let filter = std::env::args().nth(2).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
        .filter(|p| { let n = p.file_name().unwrap().to_string_lossy(); n.contains("_scenes_") && n.contains(&*filter) })
        .collect();
    bundles.sort();
    for b in &bundles {
        let name = b.file_name().unwrap().to_string_lossy().replace("campaign_scenes_", "").replace("specialscenes_scenes_", "");
        let name = name.split(".bundle").next().unwrap().split('_').next().unwrap().to_string();
        let Ok(def) = scenedef::load_scene(&mut db, b) else { continue };
        let text = std::env::var("FIND_TEXT").ok();
        for s in def.scripts.iter().filter(|s| want.contains(&s.class) || (want[0] == "*" && text.as_ref().is_some_and(|t| format!("{:?}", s.data).contains(t.as_str())))) {
            let fields: String = format!("{:?}", s.data).chars().take(if std::env::var_os("FIND_FULL").is_some() { usize::MAX } else { 300 }).collect();
            println!("{name}\t{}\t{}\tenabled {}\t{fields}", s.class, def.path(s.node), s.enabled);
            if std::env::var_os("FIND_SUBTREE").is_some() {
                // the match's own subtree, plus the subtree of every GameObject its fields reference
                let mut roots = vec![s.node];
                if let uk_assets::serialized::Value::Struct(fs) = &s.data {
                    roots.extend(fs.iter().filter(|(k, _)| &**k != "m_GameObject").filter_map(|(_, v)| def.node_ref(v)));
                }
                for r in roots {
                    println!("  subtree {}", def.path(r));
                    for (i, n) in def.nodes.iter().enumerate().filter(|(i, _)| def.is_descendant(*i as u32, r)) {
                        let classes: Vec<String> = def.scripts_on(i as u32).map(|(_, c)| format!("{}{}", c.class, if c.enabled { "" } else { "(off)" })).collect();
                        println!("    {}{} [{}]", def.path(i as u32), if n.active_self { "" } else { " (inactive)" }, classes.join(", "));
                    }
                }
            }
        }
    }
}
