//! A custom glTF map (`--map`): what was imported, where the player starts, whether they land
//! on the map's colliders and what surface the floor reports.
//! cargo run --release -p uk-game --example map_probe -- custom_maps/example.glb
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, gltf_map, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let map = std::env::args().nth(1).unwrap_or("custom_maps/example.glb".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let base = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle")).unwrap();
    let base_nodes = base.nodes.len();
    let (def, report) = gltf_map::custom_scene(&mut db, base, std::path::Path::new(&map)).unwrap();
    println!("{report:?}");
    println!("0-1 {base_nodes} nodes -> {} kept + map; roots {:?}", def.nodes.len(), def.nodes.iter().filter(|n| n.parent.is_none()).map(|n| &n.name).collect::<Vec<_>>());
    let root = def.nodes.iter().position(|n| n.name.starts_with("glTF ")).unwrap() as u32;
    for (i, n) in def.nodes.iter().enumerate().filter(|(i, _)| def.is_descendant(*i as u32, root)) {
        let rs: Vec<_> = def.renderers.iter().filter(|r| r.node == i as u32).map(|r| (r.batch.positions.len(), r.batch.indices.len() / 3, r.material.as_ref().map(|m| m.path_id))).collect();
        let cs: Vec<_> = def.colliders.iter().filter(|c| c.node == i as u32).map(|c| match &c.shape {
            scenedef::ShapeDef::Mesh(t) => format!("mesh {} tris", t.len()),
            s => format!("{s:?}"),
        }).collect();
        println!("  {} at {:.2?} renderers {rs:?} colliders {cs:?}", def.path(i as u32), n.world0.w_axis.truncate());
    }
    println!("surface meshes on the map: {:?}", def.surface_meshes.iter().filter(|s| def.is_descendant(s.node, root)).map(|s| (s.tris.len(), s.mats[0].surface)).collect::<Vec<_>>());
    let mut g = Game::new(Arc::new(def));
    g.start_custom_map();
    let p = &g.s.player;
    println!("start: pos {:.3?} yaw {:.1} activated {} timer {} level started {}", p.pos, g.spawn_yaw, p.activated, g.s.stats.timer, g.s.stats.level_started);
    let mut t = 0.0f64;
    for i in 0..180 {
        g.fixed_update(&Input::default());
        t += FIXED_DT as f64;
        g.update(&Input::default(), FIXED_DT, t);
        g.s.player.events.clear();
        if i % 60 == 59 {
            let p = &g.s.player;
            println!("t {:.2}: pos {:.3?} vel {:.2?} grounded {} hp {}", g.s.time, p.pos, p.vel, p.gc.on_ground, g.s.hp);
        }
    }
    let p = g.s.player.pos;
    if let Some(h) = g.world.raycast(p, -Vec3::Y, 10.0) {
        let c = &g.def.colliders[g.world.owner(h.collider) as usize];
        println!("floor: {} at {:.3} (layer {})", g.def.path(c.node), h.point.y, c.layer);
    }
    println!("surface under the player: {:?}", g.surface_data(p, -Vec3::Y, 10.0));
}
