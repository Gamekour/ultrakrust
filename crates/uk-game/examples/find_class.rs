//! Every instance of the given MonoBehaviour classes across the install's scene bundles: scene, path, fields.
//! find_class Class1,Class2 [bundle filter]
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
        for s in def.scripts.iter().filter(|s| want.contains(&s.class)) {
            let fields: String = format!("{:?}", s.data).chars().take(300).collect();
            println!("{name}\t{}\t{}\tenabled {}\t{fields}", s.class, def.path(s.node), s.enabled);
        }
    }
}
