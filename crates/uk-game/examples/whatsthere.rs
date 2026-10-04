//! cargo run --release -p uk-game --example whatsthere -- x y z [dirx diry dirz]
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::Game;
fn main() {
    let a: Vec<f32> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let p = Vec3::new(a[0], a[1], a[2]);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    if let Ok(list) = std::env::var("ACTIVATE") {
        for path in list.split(';') {
            if let Some(n) = (0..def.nodes.len() as u32).find(|&n| def.path(n) == path) { g.set_active(n, true); }
        }
        g.fixed_update(&uk_core::player::Input::default());
    }
    let dirs = if a.len() >= 6 { vec![Vec3::new(a[3], a[4], a[5])] } else { vec![Vec3::NEG_Z, Vec3::Z, Vec3::X, Vec3::NEG_X, Vec3::NEG_Y, Vec3::Y] };
    for d in dirs {
        match g.world.raycast(p, d, 60.0) {
            Some(h) => {
                let owner = g.world.owner(h.collider);
                let name = if owner == uk_core::collide::ALWAYS { "?".into() } else { def.path(def.colliders[owner as usize].node) };
                println!("{d:?}: hit at {:.2} ({:?}) -> {name}", h.distance, h.point);
            }
            None => println!("{d:?}: nothing within 60"),
        }
    }
    // rooms (render bounds) near the point
    let mut rooms: std::collections::HashMap<u32, (Vec3, Vec3)> = Default::default();
    for r in &def.renderers {
        let mut root = r.node;
        while let Some(pp) = def.nodes[root as usize].parent { root = pp; }
        let e = rooms.entry(root).or_insert((Vec3::MAX, Vec3::MIN));
        for v in &r.batch.positions { e.0 = e.0.min(Vec3::from(*v)); e.1 = e.1.max(Vec3::from(*v)); }
    }
    let mut v: Vec<_> = rooms.into_iter().collect();
    v.sort_by_key(|(n, _)| def.nodes[*n as usize].name.clone());
    for (n, (lo, hi)) in v {
        println!("room {:40} active={} x[{:.0},{:.0}] y[{:.0},{:.0}] z[{:.0},{:.0}]", def.nodes[n as usize].name, g.active(n), lo.x, hi.x, lo.y, hi.y, lo.z, hi.z);
    }
}
