//! MainModule.stopAction / cullingMode per effect prefab and scene-placed system of a level
//! (0 None, 1 Disable, 2 Destroy, 3 Callback).
//! cargo run --release -p uk-game --example stop_actions -- [level0-1]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    for pf in &def.particle_prefabs {
        let acts: Vec<(String, u8)> = pf.nodes.iter().filter_map(|n| n.system.as_ref().map(|s| (n.name.clone(), s.stop_action))).filter(|(_, a)| *a != 0).collect();
        if !acts.is_empty() {
            let scripts: Vec<&str> = pf.nodes.iter().flat_map(|n| n.scripts.iter().map(|s| s.class.as_str())).collect();
            println!("prefab {}: {:?} scripts {:?}", pf.name, acts, scripts);
        }
    }
    let mut counts = [0usize; 4];
    for ps in &def.particle_systems {
        counts[ps.system.stop_action.min(3) as usize] += 1;
    }
    println!("scene systems by stopAction: {counts:?}");
}
