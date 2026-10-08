//! Every component on the GameObjects whose path contains a substring, raw from the bundle.
//! raw_components "<path substring>" [level2-3]
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let q = std::env::args().nth(1).unwrap();
    let level = std::env::args().nth(2).unwrap_or("level2-3".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let files = db.load_bundle_files(&path).unwrap();
    for f in &files {
        for o in f.objects.iter().filter(|o| o.class_id == 1) {
            let Some(&n) = def.obj_to_node.get(&o.path_id) else { continue };
            if !def.path(n).contains(&q) {
                continue;
            }
            let g = f.read(o).unwrap();
            println!("{} (go {})", def.path(n), o.path_id);
            for c in g.get("m_Component").array() {
                let cid = c.get("component").pptr().1;
                if let (Some(co), Ok(cv)) = (f.object(cid), f.read_id(cid)) {
                    let s = cv.compact();
                    println!("  class {} id {cid}: {}", co.class_id, &s[..s.len().min(300)]);
                }
            }
        }
    }
}
