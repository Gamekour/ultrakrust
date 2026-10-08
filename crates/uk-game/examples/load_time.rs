//! Times loading a level's scene and building its Game. load_time [level1-1]
use std::{sync::Arc, time::Instant};
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level1-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let t = Instant::now();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    println!("scene: {:?}, {} colliders", t.elapsed(), def.colliders.len());
    let t = Instant::now();
    let _g = uk_game::Game::new(Arc::new(def));
    println!("Game::new: {:?}", t.elapsed());
}
