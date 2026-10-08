//! Raw serialized fields of scene ParticleSystems / renderers: the first system whose GameObject
//! name contains the filter, or the first with a given module enabled.
//! cargo run --release -p uk-assets --example ps_raw -- <scene filter> <go name filter | module=NameModule | shape=N> [module to print]
use uk_assets::db::AssetDb;
use uk_assets::serialized::Value;

fn show(v: &Value, ind: usize, depth: usize) {
    if let Value::Struct(fields) = v {
        if depth > 3 {
            println!("{:ind$}{{..}}", "");
            return;
        }
        for (k, f) in fields {
            match f {
                Value::Struct(_) => {
                    println!("{:ind$}{k}:", "");
                    show(f, ind + 2, depth + 1);
                }
                Value::Array(a) if a.len() > 6 => println!("{:ind$}{k}: [{} items]", "", a.len()),
                _ => println!("{:ind$}{k}: {f:?}", ""),
            }
        }
    }
}

fn main() {
    let scene = std::env::args().nth(1).unwrap_or("0-1".into());
    let filter = std::env::args().nth(2).unwrap_or_default();
    let module = std::env::args().nth(3);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().contains(&format!("campaign_scenes_level{scene}"))).collect();
    bundles.sort();
    for b in bundles {
        let files = db.load_bundle_files(&b).unwrap();
        for f in &files {
            for o in f.objects.iter().filter(|o| o.class_id == 198) {
                let Ok(v) = f.read(o) else { continue };
                let go = f.read_id(v.get("m_GameObject").pptr().1).map(|g| g.get("m_Name").str().to_string()).unwrap_or_default();
                let hit = if let Some(m) = filter.strip_prefix("module=") {
                    v.get(m).get("enabled").bool()
                } else if let Some(n) = filter.strip_prefix("shape=") {
                    v.get("ShapeModule").get("enabled").bool() && v.get("ShapeModule").get("type").i64().to_string() == n
                } else {
                    go.contains(&filter)
                };
                if !hit {
                    continue;
                }
                println!("== {} #{} {go}", b.file_name().unwrap().to_string_lossy(), o.path_id);
                match &module {
                    Some(m) if m == "renderer" => {
                        let g = f.read_id(v.get("m_GameObject").pptr().1).unwrap();
                        for c in g.get("m_Component").array() {
                            let id = c.get("component").pptr().1;
                            if f.object(id).is_some_and(|o| o.class_id == 199) {
                                show(&f.read_id(id).unwrap(), 2, 0);
                            }
                        }
                    }
                    Some(m) => show(v.get(m), 2, 0),
                    None => show(&v, 2, 0),
                }
                return;
            }
        }
    }
}
