//! Shape of an AnimationClip / AnimatorController / Avatar / Animator typetree (field names, scalar
//! values, array lengths + first element), and where they live.
//!   cargo run --release -p uk-assets --example clip_peek -- [class_id=74] [name filter]
use uk_assets::db::AssetDb;
use uk_assets::serialized::Value;

fn shape(v: &Value, depth: usize, out: &mut String) {
    let pad = "  ".repeat(depth);
    match v {
        Value::Struct(fields) => {
            for (k, f) in fields {
                match f {
                    Value::Struct(_) => {
                        *out += &format!("{pad}{k}:\n");
                        shape(f, depth + 1, out);
                    }
                    Value::Array(a) => {
                        *out += &format!("{pad}{k}: [{}]\n", a.len());
                        if let Some(e) = a.first() {
                            if matches!(e, Value::Struct(_)) { shape(e, depth + 1, out) } else { *out += &format!("{pad}  {}\n", f.compact().chars().take(120).collect::<String>()) }
                        }
                    }
                    Value::Bytes(b) => *out += &format!("{pad}{k}: bytes[{}]\n", b.len()),
                    _ => *out += &format!("{pad}{k}: {}\n", f.compact().chars().take(80).collect::<String>()),
                }
            }
        }
        _ => *out += &format!("{pad}{}\n", v.compact()),
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let class: i32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(74);
    let filter = a.get(2).cloned().unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let mut bundles: Vec<_> = std::fs::read_dir(AssetDb::bundle_dir(&install)).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "bundle")).collect();
    bundles.sort();
    let mut total = 0;
    let mut shown = false;
    for b in &bundles {
        let Ok(files) = db.load_bundle_files(b) else { continue };
        for f in files {
            for o in f.objects.iter().filter(|o| o.class_id == class) {
                total += 1;
                if shown { continue }
                let Ok(v) = f.read(o) else { continue };
                if !v.get("m_Name").str().contains(&filter) { continue }
                let mut s = String::new();
                shape(&v, 0, &mut s);
                println!("{} in {} ({})\n{s}", v.get("m_Name").str(), b.file_name().unwrap().to_string_lossy(), f.file_name());
                shown = true;
            }
        }
    }
    println!("class {class}: {total} objects");
}
