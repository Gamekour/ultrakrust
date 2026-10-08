//! NewMovement's slide and wall scrapes: slides along the floor (the scrape is created at the
//! slide start, follows the player while grounded, detaches with RemoveOnTime 10 at the stop),
//! then falls along a wall pushing into it (Cling: the wall scrape at the contact point + up,
//! facing the wall normal, detaching on landing).
//! cargo run --release -p uk-game --example scrape_probe -- [level0-1]
use bevy_math::{Vec2, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::particles::{to_unity, TAG_SLIDE_SCRAPE, TAG_WALL_SCRAPE};
use uk_game::Game;

fn report(g: &Game, label: &str) {
    let p = &g.s.player;
    println!(
        "[{label}] t {:.2} player (unity) {:.2?} vel {:.1?} ground {} on wall {} sliding {} slide surface {} scrape surface {}",
        g.s.time,
        to_unity(p.pos),
        to_unity(p.vel),
        p.gc.on_ground,
        p.wall.on_wall,
        p.sliding,
        g.fx.slide_surface,
        g.fx.scrape_surface
    );
    for e in g.fx.effects.iter().filter(|e| !e.destroyed) {
        let pf = &g.def.particle_prefabs[e.prefab as usize];
        if !pf.name.contains("Scrape") && !pf.name.contains("Slide") && e.tag != TAG_SLIDE_SCRAPE && e.tag != TAG_WALL_SCRAPE {
            continue;
        }
        let n: usize = e.systems.iter().map(|s| s.particles.len()).sum();
        println!(
            "    {} tag {} pos {:.2?} fwd {:.2?} playing {} particles {n} remove in {:?} colors {:?}",
            pf.name,
            e.tag,
            e.pos,
            e.rot * Vec3::Z,
            e.systems.iter().any(|s| s.playing),
            e.remove_at.map(|t| ((t - g.s.time) * 100.0).round() / 100.0),
            e.systems.iter().map(|s| s.def.start_color.max_color).collect::<Vec<_>>()
        );
    }
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let ff = &def.footstep_fx;
    let name = |v: &Option<u32>| v.map(|v| def.particle_prefabs[v as usize].name.clone());
    println!("slide particles: {:?}", ff.slide_particles.iter().map(|(k, v)| (*k, name(v))).collect::<Vec<_>>());
    println!("wall scrape particles: {:?}", ff.wall_scrape_particles.iter().map(|(k, v)| (*k, name(v))).collect::<Vec<_>>());
    let mut g = Game::new(def.clone());
    g.s.player.activated = true;
    let mut t = 0.0f64;
    let mut step = |g: &mut Game, inp: &Input| {
        g.fixed_update(inp);
        t += FIXED_DT as f64;
        g.update(inp, FIXED_DT, t);
        g.s.player.events.clear();
    };
    let idle = Input::default();
    for _ in 0..60 {
        step(&mut g, &idle);
    }
    if level != "level0-1" {
        frictionless(&mut g, &def, &mut step);
        return;
    }
    // move to the hallway floor (it has surface data; the start room's floor has none)
    g.s.player.pos = Vec3::new(0.0, 5.6, -270.0);
    g.s.player.prev_pos = g.s.player.pos;
    for _ in 0..60 {
        step(&mut g, &idle);
    }
    report(&g, "settled");
    let fwd = Input { slide_pressed: true, move_axis: Vec2::new(0.0, 1.0), ..Default::default() };
    step(&mut g, &fwd);
    report(&g, "slide start");
    let hold = Input { move_axis: Vec2::new(0.0, 1.0), ..Default::default() };
    for i in 0..30 {
        step(&mut g, &hold);
        if i % 10 == 9 {
            report(&g, "sliding");
        }
    }
    step(&mut g, &Input { slide_released: true, ..Default::default() });
    report(&g, "slide released");
    for _ in 0..60 {
        step(&mut g, &idle);
    }
    report(&g, "one second later");
    // a wall: the nearest hit to the side
    let p = g.s.player.pos;
    let Some((dir, h)) = [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z].iter().filter_map(|&d| g.world.raycast(p, d, 30.0).map(|h| (d, h))).min_by(|a, b| a.1.distance.total_cmp(&b.1.distance)) else { return };
    println!("wall (unity) {:.2?} normal {:.2?} at {:.2}", to_unity(h.point), to_unity(h.normal), h.distance);
    // up against it, 15 m up, facing it
    g.s.player.pos = h.point - dir * 0.6 + Vec3::Y * 15.0;
    g.s.player.prev_pos = g.s.player.pos;
    g.s.player.vel = Vec3::ZERO;
    let yaw = (-dir.x).atan2(-dir.z).to_degrees();
    g.s.player.yaw_deg = -yaw;
    let push = Input { move_axis: Vec2::new(0.0, 1.0), ..Default::default() };
    for i in 0..200 {
        step(&mut g, &push);
        if i % 15 == 0 || g.s.player.gc.on_ground {
            report(&g, "falling along the wall");
        }
        if g.s.player.gc.on_ground {
            break;
        }
    }
    step(&mut g, &idle);
    report(&g, "landed");
    frictionless(&mut g, &def, &mut step);
}

fn frictionless(g: &mut Game, def: &scenedef::SceneDef, step: &mut impl FnMut(&mut Game, &Input)) {
    let idle = Input::default();
    // frictionless: stand on top of a Slippery-tagged or layer-0 solid collider
    let tops: Vec<(u32, Vec3)> = def
        .colliders
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.trigger && g.s.active[c.node as usize] && (def.nodes[c.node as usize].tag == scenedef::tags::SLIPPERY || def.nodes[c.node as usize].layer == 0))
        .filter_map(|(ci, c)| {
            let at = match &c.shape {
                scenedef::ShapeDef::Box { center, .. } => *center,
                scenedef::ShapeDef::Mesh(t) => (t.first()?[0] + t.first()?[1] + t.first()?[2]) / 3.0,
                _ => return None,
            };
            let h = g.world.raycast_filtered(at + Vec3::Y * 50.0, -Vec3::Y, 100.0, |id| g.world.owner(id) == ci as u32)?;
            let same = |o: Vec3| g.world.raycast(o + Vec3::Y * 50.0, -Vec3::Y, 100.0).is_some_and(|w| g.world.owner(w.collider) == ci as u32 && (w.point.y - h.point.y).abs() < 0.2);
            let roomy = [Vec3::ZERO, Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z].iter().all(|&d| same(h.point + d * 1.5));
            (h.normal.y > 0.3 && roomy).then_some((ci as u32, h.point))
        })
        .collect();
    println!("{} slippery / layer-0 colliders with a walkable top", tops.len());
    for &(ci, top) in tops.iter().take(3) {
        let c = &def.colliders[ci as usize];
        println!("collider {} layer {} tag {} top (unity) {:.2?}", def.path(c.node), def.nodes[c.node as usize].layer, def.nodes[c.node as usize].tag, to_unity(top));
        g.s.player.pos = top + Vec3::Y * 1.6;
        g.s.player.prev_pos = g.s.player.pos;
        g.s.player.vel = Vec3::ZERO;
        for i in 0..20 {
            step(g, &idle);
            if i % 5 == 4 {
                report(g, "on it");
                let p = &g.s.player;
                println!("    ray at the top now {:?} | from player pos {:?}", g.world.raycast(top + Vec3::Y * 5.0, -Vec3::Y, 10.0).map(|h| h.distance), g.world.raycast(p.pos, -Vec3::Y, 10.0).map(|h| h.distance));
                println!("    slope on ground {} gc (unity) {:.2?} ray {:?}", p.slope.on_ground, to_unity(p.gc_pos()), [0.5f32, 5.0, 50.0].map(|d| g.world.raycast(p.gc_pos() + Vec3::Y * 0.1, -Vec3::Y, d).map(|h| (h.distance, g.world.owner(h.collider)))));
                for e in g.fx.effects.iter().filter(|e| e.tag == uk_game::particles::TAG_FRIC_SLIDE && !e.destroyed) {
                    println!("    frictionless {} scale {:.3?} fwd {:.2?}", def.particle_prefabs[e.prefab as usize].name, e.scale, e.rot * Vec3::Z);
                }
            }
        }
    }
}
