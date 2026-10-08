//! Scene-placed ParticleSystems: which are active, playing and emitting after N seconds of the level
//! running with the player idle, with particle counts, loop cycles and unsupported features.
//! cargo run --release -p uk-game --example scene_particles -- [level0-1] [seconds] [name filter]
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let secs: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(3.0);
    let filter = std::env::args().nth(3).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let mut g = Game::new(def.clone());
    let with_r = def.particle_systems.iter().filter(|p| p.renderer.is_some()).count();
    println!("{} scene systems ({with_r} with a renderer)", def.particle_systems.len());
    let mut t = 0.0f64;
    let steps = (secs / FIXED_DT).round() as usize;
    let t0 = std::time::Instant::now();
    for i in 0..steps {
        g.fixed_update(&Input::default());
        t += FIXED_DT as f64;
        g.update(&Input::default(), FIXED_DT, t);
        if (i + 1) % (steps / 4).max(1) == 0 {
            let on = g.fx.scene.iter().filter(|s| s.on).count();
            let playing = g.fx.scene.iter().filter(|s| s.on && s.sys.playing).count();
            let n: usize = g.fx.scene.iter().map(|s| s.sys.particles.len()).sum();
            println!("t {:.2}: {on} active, {playing} playing, {n} particles", t);
        }
    }
    println!("sim {:.1} ms per step", t0.elapsed().as_secs_f64() * 1000.0 / steps as f64);
    let mut by_unsupported: BTreeMap<String, usize> = BTreeMap::new();
    for s in g.fx.scene.iter().filter(|s| s.on) {
        let p = &def.particle_systems[s.index as usize];
        for u in &p.system.unsupported {
            *by_unsupported.entry(u.clone()).or_default() += 1;
        }
        let path = def.path(p.node);
        if !filter.is_empty() && !path.contains(&filter) {
            continue;
        }
        let d = &s.sys.def;
        let r = p.renderer.as_ref().map(|r| (r.enabled, r.render_mode));
        println!(
            "  {path}: playing {} t {:.2}/{:.2} cycle {} n {} (max {}) loop {} prewarm {} space {} renderer {r:?} unsupported {:?}",
            s.sys.playing,
            s.sys.time,
            d.duration,
            s.sys.cycle,
            s.sys.particles.len(),
            d.max_particles,
            d.looping,
            d.prewarm,
            d.simulation_space,
            p.system.unsupported
        );
    }
    println!("unsupported features on active systems: {by_unsupported:?}");
}
