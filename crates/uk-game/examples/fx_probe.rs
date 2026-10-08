//! Script-spawned effects: drives the player through a dodge, a slide, a slam and its landing
//! (NewMovement's dodge / slide / fall particles and impactDust) and prints, per phase, the live
//! effects with their root position and rotation, attachment, particle counts and the player's
//! transform for comparison.
//! cargo run --release -p uk-game --example fx_probe -- [level0-1]
use bevy_math::{Vec2, Vec3};
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::particles::to_unity;
use uk_game::Game;

fn report(g: &Game, label: &str) {
    let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for e in g.fx.effects.iter().filter(|e| !e.destroyed) {
        let pf = &g.def.particle_prefabs[e.prefab as usize];
        let n: usize = e.systems.iter().map(|s| s.particles.len()).sum();
        let tr: usize = e.systems.iter().flat_map(|s| &s.particles).filter_map(|p| p.trail.as_ref()).map(|t| t.points.len()).sum();
        let fwd = e.rot * Vec3::Z;
        by.entry(pf.name.clone()).or_default().push(format!(
            "pos {:.2?} fwd {:.2?} tag {} attach {} particles {n} trail pts {tr} playing {}",
            e.world[0].w_axis.truncate(),
            fwd,
            e.tag,
            e.attach.is_some(),
            e.systems.iter().any(|s| s.playing)
        ));
    }
    let p = &g.s.player;
    println!("[{label}] t {:.2} player (unity) {:.2?} vel {:.1?} ground {} sliding {} heavy {} dodge dir (unity) {:.2?}", g.s.time, to_unity(p.pos), to_unity(p.vel), p.gc.on_ground, p.sliding, p.gc.heavy_fall, to_unity(p.dodge_direction));
    for (k, v) in by {
        for s in v {
            println!("    {k}: {s}");
        }
    }
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    for ((sc, field), p) in &def.script_prefabs {
        let pf = &def.particle_prefabs[*p as usize];
        println!("{}.{field} -> {} ({} nodes, {} systems)", def.scripts[*sc as usize].class, pf.name, pf.nodes.len(), pf.nodes.iter().filter(|n| n.system.is_some()).count());
    }
    for w in &def.warnings {
        if w.contains('.') && !w.contains("missing") {
            println!("warning: {w}");
        }
    }
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
    report(&g, "settled");
    // dodge forward
    step(&mut g, &Input { dash_pressed: true, move_axis: Vec2::new(0.0, 1.0), ..Default::default() });
    report(&g, "dash frame");
    for i in 0..40 {
        step(&mut g, &idle);
        if i % 20 == 19 {
            report(&g, "after dash");
        }
    }
    // wait out the stamina, then slide right
    for _ in 0..150 {
        step(&mut g, &idle);
    }
    let slide = Input { slide_pressed: true, move_axis: Vec2::new(1.0, 0.0), ..Default::default() };
    step(&mut g, &slide);
    report(&g, "slide start");
    let hold = Input { move_axis: Vec2::new(1.0, 0.0), ..Default::default() };
    for i in 0..60 {
        step(&mut g, &hold);
        if i % 20 == 19 {
            report(&g, "sliding");
        }
    }
    step(&mut g, &Input { slide_released: true, ..Default::default() });
    step(&mut g, &idle);
    report(&g, "slide released");
    // slam from 40 m up
    g.s.player.pos += Vec3::Y * 40.0;
    g.s.player.prev_pos = g.s.player.pos;
    for _ in 0..80 {
        step(&mut g, &idle);
    }
    step(&mut g, &Input { slide_pressed: true, ..Default::default() });
    report(&g, "slam start");
    for i in 0..200 {
        let was = g.s.player.gc.heavy_fall;
        step(&mut g, &idle);
        if was && !g.s.player.gc.heavy_fall {
            report(&g, "landed");
        }
        if i % 50 == 49 {
            report(&g, "after slam");
        }
    }
    // weapons into the floor: hit particles must face up (forward = hit normal)
    let eye = g.s.player.pos + Vec3::Y;
    let down = Vec3::new(0.0, -1.0, -0.2).normalize();
    if let Some(h) = g.world.raycast(eye, down, 1000.0) {
        println!("floor ray: point (unity) {:.2?} normal (unity) {:.2?}", to_unity(h.point), to_unity(h.normal));
    }
    g.fire_revolver(eye, down, false);
    g.fire_revolver(eye, down, true);
    g.punch(eye, down);
    report(&g, "revolver, piercer, punch at the floor");
    for _ in 0..2 {
        step(&mut g, &idle);
    }
    report(&g, "two frames later");
}
