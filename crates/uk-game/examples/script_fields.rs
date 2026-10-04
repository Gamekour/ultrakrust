//! Serialized fields of every instance of a script class in a level (compact; PPtrs as @file:id).
//! script_fields <ClassName> [level]
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let class = std::env::args().nth(1).expect("class name");
    let level = std::env::args().nth(2).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    for s in def.scripts.iter().filter(|s| s.class == class) {
        println!("{} (enabled {}):\n  {}", def.path(s.node), s.enabled, s.data.compact().chars().take(3000).collect::<String>());
    }
}
