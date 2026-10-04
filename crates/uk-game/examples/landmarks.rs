//! Prints doors, checkpoints, arenas and room bounds of 0-1 (Bevy coordinates).
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let pos = |n: u32| def.nodes[n as usize].world0.w_axis.truncate();
    for (i, s) in def.scripts.iter().enumerate() {
        if ["Door", "CheckPoint", "ActivateArena", "FinalPit", "PlayerActivator", "SpiderBody", "MaliciousFace", "FinalDoor"].contains(&s.class.as_str()) {
            let p = pos(s.node);
            println!("{:14} [{i:5}] ({:7.1},{:6.1},{:7.1}) {}", s.class, p.x, p.y, p.z, def.path(s.node));
        }
    }
}
