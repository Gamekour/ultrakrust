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
    let mut bundles = Vec::new();
    let mut stack = vec![AssetDb::bundle_dir(&install)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { stack.push(p) } else if p.extension().is_some_and(|x| x == "bundle") { bundles.push(p) }
        }
    }
    // AnimationClip census: binding (typeID, attribute) tallies, curve storage kinds, events
    let mut tally: std::collections::BTreeMap<(i64, i64, bool), usize> = Default::default();
    let (mut streamed, mut dense, mut constant, mut events, mut legacy) = (0, 0, 0, 0, 0);
    bundles.sort();
    let mut total = 0;
    let mut shown = false;
    for b in &bundles {
        let Ok(files) = db.load_bundle_files(b) else { continue };
        for f in files {
            for o in f.objects.iter().filter(|o| o.class_id == class) {
                total += 1;
                let Ok(v) = f.read(o) else { continue };
                if class == 74 {
                    let c = v.get("m_MuscleClip").get("m_Clip").get("data");
                    streamed += !c.get("m_StreamedClip").get("data").array().is_empty() as usize;
                    dense += !c.get("m_DenseClip").get("m_SampleArray").array().is_empty() as usize;
                    constant += !c.get("m_ConstantClip").get("data").array().is_empty() as usize;
                    events += v.get("m_Events").array().len();
                    legacy += v.get("m_Legacy").bool() as usize;
                    for g in v.get("m_ClipBindingConstant").get("genericBindings").array() {
                        *tally.entry((g.get("typeID").i64(), g.get("attribute").i64(), g.get("customType").i64() != 0)).or_default() += 1;
                    }
                }
                if shown { continue }
                if !v.get("m_Name").str().contains(&filter) { continue }
                let mut s = String::new();
                shape(&v, 0, &mut s);
                println!("{} in {} ({})\n{s}", v.get("m_Name").str(), b.file_name().unwrap().to_string_lossy(), f.name);
                shown = true;
            }
        }
    }
    println!("class {class}: {total} objects");
    if class == 74 {
        println!("streamed {streamed} dense {dense} constant {constant} events {events} legacy {legacy}");
        for ((t, a, c), n) in tally { println!("binding typeID {t} attribute {a} custom {c}: {n}") }
    }
}
