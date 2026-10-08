//! The 0-1 autopilot's per-tick sim cost with the scene particle systems running and with them stopped and cleared every tick, and the
//! live particle / collision counts (what the harness sim_budget sees).
//! cargo run --release -p uk-game --example fx_cost
use std::sync::Arc;
use std::time::Instant;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::bot::{route_0_1, Bot};
use uk_game::Game;

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle")).unwrap());
    for scene in [true, false] {
        let mut g = Game::new(def.clone());
        let mut bot = Bot::new(route_0_1());
        let mut t = 0.0;
        let mut ticks = Vec::new();
        let (mut live, mut coll) = (0usize, 0usize);
        while t < 600.0 && !bot.done() && !g.s.level_complete {
            let s = Instant::now();
            if !scene {
                for ss in &mut g.fx.scene {
                    ss.sys.stop();
                    ss.sys.clear();
                }
            }
            uk_game::bot::drive(&mut g, &mut bot, &mut t);
            bot.log.clear();
            ticks.push(s.elapsed().as_secs_f64() * 1e6);
            let n: usize = g.fx.scene.iter().map(|s| s.sys.particles.len()).sum();
            let c: usize = g.fx.scene.iter().filter(|s| s.sys.def.collision.is_some()).map(|s| s.sys.particles.len()).sum();
            live = live.max(n);
            coll = coll.max(c);
        }
        ticks.sort_by(|a, b| a.total_cmp(b));
        let p = |q: f64| ticks[((ticks.len() - 1) as f64 * q) as usize];
        println!("scene systems {scene}: {} ticks p50 {:.0} p99 {:.0} us; max live scene particles {live}, with collision {coll}", ticks.len(), p(0.5), p(0.99));
    }
}
