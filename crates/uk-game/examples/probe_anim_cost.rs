//! Mecanim runtime cost per animated rig: 0-1 at start, then with every node switched on.
//! cargo run --release -p uk-game --example probe_anim_cost [level0-1]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::Game;

fn main() {
    let q = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{q}.bundle"))).unwrap());
    let t = std::time::Instant::now();
    let mut g = Game::new(def.clone());
    println!("build {:.1} ms, rigs {}", t.elapsed().as_secs_f64() * 1e3, g.anim.rig_count());
    for pass in 0..2 {
        if pass == 1 {
            for n in 0..def.nodes.len() as u32 {
                g.set_active(n, true);
            }
        }
        uk_game::anim::update(&mut g, 0.008);
        let mut ts: Vec<f64> = (0..300).map(|_| { let t = std::time::Instant::now(); uk_game::anim::update(&mut g, 0.008); t.elapsed().as_secs_f64() * 1e6 }).collect();
        ts.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let n = g.anim.stats.updated_last_frame;
        println!("{} updated {n}: p50 {:.0} us p99 {:.0} us, {:.1} us/rig", ["start", "all on"][pass], ts[150], ts[296], ts[150] / n.max(1) as f64);
    }
}
