//! Floor heights on a grid (top-down raycasts) around a point of 0-1: route debugging.
//! cargo run --release -p uk-game --example probe_floor -- x z half step top_y   (half 0: surfaces under x z)
use std::sync::Arc;
use bevy_math::Vec3;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::Game;

fn main() {
    let a: Vec<f32> = std::env::args().skip(1).map(|s| s.parse().unwrap()).collect();
    let (cx, cz, half, step, top) = (a[0], a[1], a[2], a[3], a[4]);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let mut g = Game::new(Arc::new(scenedef::load_scene(&mut db, &path).unwrap()));
    // every room switched on (colliders of inactive rooms don't take raycasts)
    for n in 0..g.def.nodes.len() as u32 {
        g.set_active(n, true);
    }
    g.fixed_update(&uk_core::player::Input::default());
    // half = 0: every surface (top-down) under the single point x z instead of a grid
    if half == 0.0 {
        let (mut y, mut hits) = (top, Vec::new());
        while let Some(h) = g.world.raycast(Vec3::new(cx, y, cz), Vec3::NEG_Y, y + 50.0) {
            hits.push(format!("{:.1}", y - h.distance));
            y -= h.distance + 0.05;
        }
        println!("x{cx} z{cz}: {}", hits.join(" "));
        return;
    }
    let mut z = cz - half;
    while z <= cz + half {
        let mut row = format!("z{z:7.1}");
        let mut x = cx - half;
        while x <= cx + half {
            let h = g.world.raycast(Vec3::new(x, top, z), Vec3::NEG_Y, 60.0).map(|h| top - h.distance);
            row += &h.map(|y| format!("{y:5.0}")).unwrap_or("    .".into());
            x += step;
        }
        println!("{row}");
        z += step;
    }
}
