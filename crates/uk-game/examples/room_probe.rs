//! A custom map's `-room`s and `-door`s: each Door's room lists, then the player walking forward
//! (-Z) through the doors, printing every door's state and every room's activity as they change.
//! cargo run --release -p uk-game --example room_probe -- <map.gltf> [seconds]
use std::sync::Arc;
use uk_assets::{db::AssetDb, gltf_map, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::Script;
use uk_game::Game;

fn main() {
    let map = std::env::args().nth(1).expect("map path");
    let secs: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8.0);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let base = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle")).unwrap();
    let (def, report) = gltf_map::custom_scene(&mut db, base, std::path::Path::new(&map)).unwrap();
    println!("rooms {} doors {} warnings {:?}", report.rooms, report.doors, report.warnings);
    let mut g = Game::new(Arc::new(def));
    g.start_custom_map();
    let def = g.def.clone();
    let doors: Vec<usize> = (0..g.s.scripts.len()).filter(|&i| matches!(g.s.scripts[i], Script::Door(_))).filter(|&i| def.path(def.scripts[i].node).starts_with("glTF")).collect();
    let rooms: Vec<u32> = (0..def.nodes.len() as u32).filter(|&n| doors.iter().any(|&i| matches!(&g.s.scripts[i], Script::Door(d) if d.activated_rooms.contains(&n)))).collect();
    for &i in &doors {
        let Script::Door(d) = &g.s.scripts[i] else { unreachable!() };
        let names = |v: &[u32]| v.iter().map(|&n| def.nodes[n as usize].name.clone()).collect::<Vec<_>>();
        println!("{}: activates {:?} deactivates {:?} open_offset {:.2?} speed {} controllers {:?}", def.path(def.scripts[i].node), names(&d.activated_rooms), names(&d.deactivated_rooms), d.open_offset, d.speed,
            def.colliders.iter().filter(|c| c.trigger && def.nodes[c.node as usize].parent == def.nodes[def.scripts[i].node as usize].parent).map(|c| format!("{:?}", c.shape)).collect::<Vec<_>>());
    }
    let state = |g: &Game| {
        let ds: Vec<String> = doors.iter().map(|&i| {
            let Script::Door(d) = &g.s.scripts[i] else { unreachable!() };
            format!("{} open {}", def.nodes[def.scripts[i].node as usize].name, d.open)
        }).collect();
        let rs: Vec<String> = rooms.iter().map(|&r| format!("{} {}", def.nodes[r as usize].name, if g.active(r) { "on" } else { "off" })).collect();
        format!("{} | {}", ds.join(", "), rs.join(", "))
    };
    let mut last = String::new();
    let input = Input { move_axis: bevy_math::Vec2::Y, ..Default::default() };
    let mut t = 0.0f64;
    let steps = (secs / FIXED_DT) as usize;
    for i in 0..steps {
        let s = state(&g);
        if s != last {
            let ys: Vec<String> = doors.iter().map(|&i| format!("{:.2}", g.node_world_now(def.scripts[i].node).w_axis.y)).collect();
            println!("t {:.2} player z {:.2}: {s} (door y {})", g.s.time, g.s.player.pos.z, ys.join(", "));
            last = s;
        }
        let inp = if i < 30 { Input::default() } else { input };
        g.fixed_update(&inp);
        t += FIXED_DT as f64;
        g.update(&inp, FIXED_DT, t);
        g.s.player.events.clear();
    }
    println!("end t {:.2} player {:.2?}", g.s.time, g.s.player.pos);
}
