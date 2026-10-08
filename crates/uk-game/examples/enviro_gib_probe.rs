//! SceneHelper.CreateEnviroGibs: finds floor points with surface data, then spawns gibs directly
//! (sizes 1 and 0.5), through a revolver shot at the floor, and through a hard landing, printing
//! per spawned effect the prefab, root scale, per-system node scale, startSpeed, first burst
//! count, startColor and live particles.
//! cargo run --release -p uk-game --example enviro_gib_probe -- [level0-1]
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::particles::to_unity;
use uk_game::Game;

fn dump(g: &Game, from: usize, label: &str) {
    println!("[{label}] {} new effects", g.fx.effects.len() - from);
    for e in &g.fx.effects[from..] {
        let pf = &g.def.particle_prefabs[e.prefab as usize];
        println!("  {} root scale {:.2?} pos {:.2?} fwd {:.2?} destroyed {}", pf.name, e.scale, e.pos, e.rot * Vec3::Z, e.destroyed);
        for s in &e.systems {
            println!(
                "    node {} '{}' node_scale {:.2} startSpeed {:.2}/{:.2} (mode {}) burst0 {:?} startColor {:.2?} particles {}",
                s.node,
                pf.nodes[s.node as usize].name,
                e.node_scale[s.node as usize],
                s.def.start_speed.scalar,
                s.def.start_speed.min_scalar,
                s.def.start_speed.mode,
                s.def.bursts.first().map(|b| (b.count.scalar, b.count.min_scalar, b.count.mode)),
                s.def.start_color.max_color,
                s.particles.len()
            );
        }
    }
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    println!("enviroGib particles by surface: {:?}", def.footstep_fx.enviro_gib_particles.iter().map(|(k, v)| (*k, v.map(|v| def.particle_prefabs[v as usize].name.clone()))).collect::<Vec<_>>());
    for v in def.footstep_fx.enviro_gib_particles.values().flatten().collect::<std::collections::BTreeSet<_>>() {
        let pf = &def.particle_prefabs[*v as usize];
        for (i, n) in pf.nodes.iter().enumerate() {
            for sc in n.scripts.iter().filter(|s| s.class == "EnviroGibModifier") {
                println!("{} node {i} '{}' active {} system {}: {:?}", pf.name, n.name, pf.is_active_in_hierarchy(i as u32), n.system.is_some(), sc.data);
            }
        }
    }
    let mut g = Game::new(def.clone());
    g.s.player.activated = true;
    let mut t = 0.0f64;
    let mut step = |g: &mut Game, inp: &Input| {
        g.fixed_update(inp);
        t += FIXED_DT as f64;
        g.update(inp, FIXED_DT, t);
    };
    let idle = Input::default();
    for _ in 0..60 {
        step(&mut g, &idle);
    }
    // floor points with surface data, preferring a non-white color
    let p0 = g.s.player.pos;
    let mut found = Vec::new();
    for k in -20..=20 {
        for side in -5..=5 {
            let o = p0 + Vec3::new(side as f32 * 3.0, 1.0, k as f32 * 4.0);
            if let Some(h) = g.surface_data(o, -Vec3::Y, 100.0) {
                if let Some(w) = g.world.raycast(o, -Vec3::Y, 100.0) {
                    if found.is_empty() && side == 0 {
                        println!("  surface d {:.2} world d {:.2} static {}", h.distance, w.distance, g.is_static_environment(w.collider));
                    }
                }
                if g.world.raycast(o, -Vec3::Y, 100.0).is_some_and(|w| (w.distance - h.distance).abs() < 0.01 && g.is_static_environment(w.collider)) {
                    found.push(h);
                }
            }
        }
    }
    println!("{} floor points with surface data", found.len());
    let Some(h) = found.iter().find(|h| h.color != [1.0; 4]).or(found.first()).copied() else { return };
    println!("using point (unity) {:.2?} surface {} color {:.2?} node {}", to_unity(h.point), h.surface, h.color, def.path(h.node));
    let up = h.point + Vec3::Y;
    for size in [1.0, 0.5] {
        g.fx.effects.clear();
        let n = 0;
        g.create_enviro_gibs(up, -Vec3::Y, 5.0, 3, size);
        dump(&g, n, &format!("direct size {size}"));
        step(&mut g, &idle);
        dump(&g, n, "one frame later");
    }
    // modifier surfaces with a tint: wood (14), glass (3), grass (4), tall (1), at sizes 1 and 0.5
    for surf in [14, 3, 4, 1] {
        for size in [1.0, 0.5] {
            g.fx.effects.clear();
            let mut hh = h;
            hh.surface = surf;
            hh.color = [1.0, 0.2, 0.1, 0.6];
            g.enviro_gibs_at(&hh, size);
            step(&mut g, &idle);
            dump(&g, 0, &format!("surface {surf} red tint size {size}, one frame later"));
        }
    }
    // revolver at the point from 3 m above, damage 1 (size 1)
    g.fx.effects.clear();
    let n = 0;
    g.fire_revolver(h.point + Vec3::new(0.0, 3.0, 0.5), Vec3::new(0.0, -3.0, -0.5).normalize(), false);
    dump(&g, n, "revolver at the floor");
    // landing from 60 m above the point
    g.s.player.pos = h.point + Vec3::Y * 60.0;
    g.s.player.prev_pos = g.s.player.pos;
    g.s.player.vel = Vec3::ZERO;
    g.fx.effects.clear();
    let n = 0;
    for _ in 0..400 {
        let was = g.s.player.gc.on_ground;
        let fall = g.s.player.fall_speed;
        step(&mut g, &idle);
        if !was && g.s.player.gc.on_ground {
            println!("landed at (unity) {:.2?} fall speed {fall:.1}", to_unity(g.s.player.pos));
            break;
        }
    }
    dump(&g, n, "hard landing");
}
