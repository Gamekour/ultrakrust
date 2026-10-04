//! Floor heights along a line: cargo run --release -p uk-game --example slice -- x0 z0 x1 z1 y_from steps
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::Game;
fn main() {
    let a: Vec<f32> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    if let Ok(list) = std::env::var("ACTIVATE") {
        for p in list.split(';') { if let Some(n) = (0..def.nodes.len() as u32).find(|&n| def.path(n) == p) { g.set_active(n, true); } }
        g.fixed_update(&uk_core::player::Input::default());
    }
    let steps = a[5] as usize;
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = a[0] + (a[2] - a[0]) * t; let z = a[1] + (a[3] - a[1]) * t;
        let mut y = a[4];
        let mut layers = Vec::new();
        for _ in 0..4 {
            match g.world.raycast(Vec3::new(x, y, z), Vec3::NEG_Y, 80.0) {
                Some(h) => {
                    let o = g.world.owner(h.collider);
                    let name = if o == uk_core::collide::ALWAYS { "?".into() } else { def.nodes[def.colliders[o as usize].node as usize].name.clone() };
                    layers.push(format!("{:.1}:{}", h.point.y, name));
                    y = h.point.y - 0.3;
                }
                None => break,
            }
        }
        println!("({x:6.1},{z:7.1}) {}", layers.join("  |  "));
    }
}
