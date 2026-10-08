//! Visible geometry with no collision behind it: for each enabled renderer under a root, rays
//! from just in front of each big triangle back into it; renderers whose surface mostly misses
//! every collider are listed (with what the misses would have been inside of, if anything).
//! solid_gaps <level> <root name substring> [activate: 1 = set the root's subtree chain active]
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let level = a.first().cloned().unwrap_or("level2-3".into());
    let root_q = a.get(1).cloned().unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    let mut t = 0.0;
    for _ in 0..10 {
        g.fixed_update(&Input::default());
        t += FIXED_DT as f64;
        g.update(&Input::default(), FIXED_DT, t);
    }
    let roots: Vec<u32> = (0..def.nodes.len() as u32).filter(|&n| def.nodes[n as usize].parent.is_none() && def.nodes[n as usize].name.contains(&root_q)).collect();
    if a.get(2).is_some_and(|s| s == "1") {
        for &r in &roots {
            g.set_active(r, true);
        }
        for _ in 0..10 {
            g.fixed_update(&Input::default());
            t += FIXED_DT as f64;
            g.update(&Input::default(), FIXED_DT, t);
        }
    }
    // RAY=x,y,z: what a 4 m downward ray from 2 m above the point hits
    if let Ok(r) = std::env::var("RAY") {
        let v: Vec<f32> = r.split(',').map(|x| x.parse().unwrap()).collect();
        let p = Vec3::new(v[0], v[1] + 2.0, v[2]);
        match g.world.raycast(p, -Vec3::Y, 4.0) {
            Some(h) => {
                let c = &def.colliders[g.world.owner(h.collider) as usize];
                println!("ray at {p:?}: hit y {:.3} {} layer {}", h.point.y, def.path(c.node), c.layer);
            }
            None => println!("ray at {p:?}: nothing"),
        }
        return;
    }
    let (mut total, mut gaps) = (0, 0);
    for r in def.renderers.iter() {
        if !r.enabled || !roots.iter().any(|&q| def.is_descendant(r.node, q)) {
            continue;
        }
        let b = &r.batch;
        let p = |i: u32| Vec3::from(b.positions[i as usize]);
        let (mut area, mut miss_area, mut n_up_miss) = (0.0f32, 0.0f32, 0);
        let mut sample = None;
        for tri in b.indices.chunks_exact(3) {
            let (a0, a1, a2) = (p(tri[0]), p(tri[1]), p(tri[2]));
            let cr = (a1 - a0).cross(a2 - a0);
            let ar = cr.length() * 0.5;
            if ar < 0.25 {
                continue;
            }
            let n = cr / (2.0 * ar);
            let c = (a0 + a1 + a2) / 3.0;
            area += ar;
            let hit = g.world.raycast(c + n * 1.5, -n, 3.0).is_some() || g.world.raycast(c - n * 1.5, n, 3.0).is_some();
            if !hit {
                miss_area += ar;
                if n.y > 0.7 {
                    n_up_miss += 1;
                }
                sample.get_or_insert(c);
            }
        }
        if area < 1.0 {
            continue;
        }
        total += 1;
        if miss_area > 0.3 * area && g.active(r.node) {
            gaps += 1;
            let cols: Vec<_> = def.colliders.iter().filter(|c| c.node == r.node).map(|c| (c.layer, c.trigger, c.enabled, match &c.shape { scenedef::ShapeDef::Mesh(t) => format!("mesh{}", t.len()), s => format!("{s:?}").chars().take(30).collect() })).collect();
            println!(
                "{:.0}/{:.0} m2 missed (up-facing tris {n_up_miss}) active {} layer {} {} at {:.1?} own colliders {:?}",
                miss_area, area, g.active(r.node), def.nodes[r.node as usize].layer, def.path(r.node), sample.unwrap(), cols
            );
        }
    }
    println!("{gaps}/{total} renderers with >30% of their area off any collider");
}
