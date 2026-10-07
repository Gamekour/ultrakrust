//! Objects of CLASS in a loose built-in file whose m_Name contains NAME.
//! cargo run --release -p uk-assets --example builtin_find -- "unity_builtin_extra" 21 UI
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    // load a scene first so typetrees exist for the fallback
    let _ = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle")).unwrap();
    let f = db.file(&a[1]).unwrap();
    let class: i32 = a[2].parse().unwrap();
    let mut counts = std::collections::BTreeMap::new();
    for o in &f.objects { *counts.entry(o.class_id).or_insert(0) += 1; }
    eprintln!("{} objects {:?}", f.objects.len(), counts);
    for o in f.objects.iter().filter(|o| o.class_id == class) {
        let Ok(v) = f.read(o) else { println!("{} unreadable", o.path_id); continue };
        let name = v.get("m_Name").str().to_string();
        if name.contains(a.get(3).map(|s| s.as_str()).unwrap_or("")) {
            let c = v.compact();
            let pn = v.get("m_ParsedForm").get("m_Name").str().to_string(); println!("{} {} [{}] {}", o.path_id, name, pn, &c[..c.len().min(1500)]);
        }
    }
}
