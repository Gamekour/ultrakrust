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
    for (ri, oi) in &def.renderer_override {
        let o = &def.material_overrides[*oi as usize];
        println!(
            "  renderer {ri} ({}) material {:?}: keywords {:?} floats {:?} colors {:.3?} textures {:?}",
            def.path(def.renderers[*ri as usize].node),
            o.name,
            o.keywords,
            o.floats,
            o.colors,
            o.textures.iter().map(|(n, t)| (n, t.width, t.height, t.filter, t.wrap, t.rgba[..4].to_vec(), t.rgba[t.rgba.len() - 4..].to_vec())).collect::<Vec<_>>()
        );
    }
    // winding: counter-clockwise faces vs their vertex normals, and vs the way out of the mesh
    for r in def.renderers.iter().filter(|r| def.is_descendant(r.node, root)) {
        let b = &r.batch;
        let p = |i: u32| Vec3::from(b.positions[i as usize]);
        let c = b.positions.iter().map(|&q| Vec3::from(q)).sum::<Vec3>() / b.positions.len() as f32;
        let (mut agree, mut out) = (0, 0);
        let n = b.indices.len() / 3;
        for t in b.indices.chunks_exact(3) {
            let g = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            agree += (g.dot(Vec3::from(b.normals[t[0] as usize])) > 0.0) as usize;
            out += (g.dot(p(t[0]) + p(t[1]) + p(t[2]) - 3.0 * c) > 0.0) as usize;
        }
        for t in b.indices.chunks_exact(3) {
            let (a, bb, cc) = (p(t[0]), p(t[1]), p(t[2]));
            let g = (bb - a).cross(cc - a).normalize_or_zero();
            if g.y.abs() > 0.9 && (a.y - bb.y).abs() < 1e-3 && (a.y - cc.y).abs() < 1e-3 {
                println!("    horizontal tri at y {:.2} ccw normal {:.2?} vertex normal {:.2?} x {:.2}..{:.2}", a.y, g, b.normals[t[0] as usize], a.x.min(bb.x).min(cc.x), a.x.max(bb.x).max(cc.x));
            }
        }
        println!("  {}: {n} tris, ccw face agrees with the vertex normal {agree}, faces away from the centre {out}", def.path(r.node));
    }
    println!("surface meshes on the map: {:?}", def.surface_meshes.iter().filter(|s| def.is_descendant(s.node, root)).map(|s| (s.tris.len(), s.mats[0].surface)).collect::<Vec<_>>());
    let mut g = Game::new(Arc::new(def));
    g.start_custom_map();
    for (i, l) in g.def.lights.iter().enumerate().filter(|(_, l)| g.def.is_descendant(l.node, root)) {
        let (on, range) = g.s.lights[i];
        println!("light {}: kind {} color {:.3?} intensity {:.3} range {range:.2} spot {:.1} enabled {on} active {}", g.def.path(l.node), l.kind, l.color, l.intensity, l.spot_angle, g.active(l.node));
    }
    let enemies = |g: &Game| {
        for e in &g.s.enemies {
            let spawn_fx = g.def.nodes[e.node as usize].children.iter().filter(|&&c| g.def.nodes[c as usize].name.starts_with("SpawnEffect") && g.active(c)).count();
            println!(
                "  enemy {:?} {}: active {} alive {} spawn_in {} hp {:.1} pos {:.2?} yaw {:.1} vel {:.2?} grounded {} spawn effects active {spawn_fx}",
                e.kind, g.def.path(e.node), g.active(e.node), e.alive, e.spawn_in, e.health, e.pos, e.yaw.to_degrees(), e.vel, e.grounded
            );
        }
    };
    println!("enemies at start:");
    enemies(&g);
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
    println!("enemies after 3 s:");
    enemies(&g);
    let p = g.s.player.pos;
    if let Some(h) = g.world.raycast(p, -Vec3::Y, 10.0) {
        let c = &g.def.colliders[g.world.owner(h.collider) as usize];
        println!("floor: {} at {:.3} (layer {})", g.def.path(c.node), h.point.y, c.layer);
    }
    println!("surface under the player: {:?}", g.surface_data(p, -Vec3::Y, 10.0));
}
