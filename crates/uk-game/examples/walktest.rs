//! Walk from a point toward a heading: cargo run --release -p uk-game --example walktest -- x y z yaw_deg seconds
use bevy_math::{Vec2, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
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
    }
    g.s.player.activated = true;
    g.s.player.pos = Vec3::new(a[0], a[1], a[2]); g.s.player.prev_pos = g.s.player.pos; g.s.player.yaw_deg = a[3];
    let walk = Input { move_axis: Vec2::Y, ..Default::default() };
    let mut t = 0.0;
    for i in 0..(a[4] / FIXED_DT) as usize {
        g.fixed_update(&walk); t += FIXED_DT as f64; g.update(&walk, FIXED_DT, t); g.s.hp = 100;
        if i % 25 == 0 { let p = &g.s.player; println!("{:5.2}s pos {:?} vel {:?} ground {} slope {} wall {}", i as f32 * FIXED_DT, p.pos, p.vel, p.gc.on_ground, p.slope.on_ground, p.wall.on_wall); }
    }
}
