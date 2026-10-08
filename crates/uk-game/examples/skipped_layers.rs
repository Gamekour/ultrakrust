//! Non-trigger scene colliders the world leaves out because of their layer, per layer, with a few paths.
//! skipped_layers [level2-3] [root substring]
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level2-3".into());
    let q = std::env::args().nth(2).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let mut by = std::collections::BTreeMap::<u8, Vec<String>>::new();
    for c in def.colliders.iter().filter(|c| !c.trigger && def.path(c.node).contains(&q)) {
        by.entry(c.layer).or_default().push(def.path(c.node));
    }
    for (l, v) in by {
        println!("layer {l:2}: {} colliders, e.g. {:?}", v.len(), &v[..v.len().min(4)]);
    }
}
