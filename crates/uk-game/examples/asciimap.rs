//! Top-down walkability map from collision: cargo run --release -p uk-game --example asciimap -- x0 x1 z0 z1 y_top [step]
//! '#' wall at chest height, '_' low ceiling (crawl), '.' floor, ' ' no floor (pit), digits = floor height bands
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::Game;
fn main() {
    let a: Vec<f32> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let (x0, x1, z0, z1, ytop) = (a[0], a[1], a[2], a[3], a[4]);
    let step = *a.get(5).unwrap_or(&1.0);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    // show everything (inactive rooms too) so the full layout is visible
    if let Ok(list) = std::env::var("ACTIVATE") {
        for p in list.split(';') { if let Some(n) = (0..def.nodes.len() as u32).find(|&n| def.path(n) == p) { g.set_active(n, true); } }
        g.fixed_update(&uk_core::player::Input::default());
    }
    if std::env::var("ALLROOMS").is_ok() { for o in g.world.owner_enabled.iter_mut() { *o = true; } }
    let mut z = z1;
    print!("      ");
    let mut x = x0; let mut i = 0; while x <= x1 { if i % 10 == 0 { print!("|"); } else { print!(" "); } x += step; i += 1; }
    println!("  (x from {x0} step {step})");
    while z >= z0 {
        print!("{z:6.0}");
        let mut x = x0;
        while x <= x1 {
            let top = Vec3::new(x, ytop, z);
            let ch = match g.world.raycast(top, Vec3::NEG_Y, 80.0) {
                None => ' ',
                Some(h) => {
                    let fy = h.point.y;
                    // space above the floor: is there 3.5 of headroom?
                    let head = g.world.raycast(Vec3::new(x, fy + 0.1, z), Vec3::Y, 3.6).map(|h2| h2.distance);
                    let body_blocked = !g.world.overlap_sphere(Vec3::new(x, fy + 0.65, z), 0.45).is_empty();
                    if fy > ytop - 1.0 || body_blocked { '#' } else if head.is_some_and(|d| d < 1.3) { '#' } else if head.is_some_and(|d| d < 3.5) { '_' } else {
                        let base: f32 = std::env::var("BAND_BASE").ok().and_then(|v| v.parse().ok()).unwrap_or(-50.0);
                        let size: f32 = std::env::var("BAND").ok().and_then(|v| v.parse().ok()).unwrap_or(5.0);
                        let band = ((fy - base) / size).floor() as i32;
                        std::char::from_digit((band.rem_euclid(10)) as u32, 10).unwrap()
                    }
                }
            };
            print!("{ch}");
            x += step;
        }
        println!();
        z -= step;
    }
    let _ = &mut g;
}
