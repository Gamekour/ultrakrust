//! Find where the autopilot stops making progress: log state around jumps and the stall.
//! cargo run --release -p uk-game --example freeze -- [from_seconds]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::bot::{drive, route_0_1, Bot};
use uk_game::Game;
fn main() {
    let from: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    let mut bot = Bot::new(route_0_1());
    let mut t = 0.0f64;
    let mut prev = g.s.player.pos;
    let (mut last_wp, mut since) = (0, 0.0);
    while t < 600.0 {
        let f = drive(&mut g, &mut bot, &mut t);
        let jump = (g.s.player.pos - prev).length();
        for l in bot.log.drain(..) { if t > from { println!("{l}") } }
        if t > from && (jump > 2.0 || !g.events.is_empty()) {
            println!("{t:7.2} pos {:?} jump {jump:.2} vel {:?} ev {:?}", g.s.player.pos, g.s.player.vel, g.events);
        }
        if bot.idx != last_wp { last_wp = bot.idx; since = 0.0 } else { since += 0.008 }
        if since > 20.0 {
            println!("{t:7.2} STALLED 20 s at wp {} ({}) pos {:?} vel {:?} input {:?} ground {} activated {}",
                bot.idx, bot.route[bot.idx].label, g.s.player.pos, g.s.player.vel, f.input.move_axis, g.s.player.gc.on_ground, g.s.player.activated);
            for e in g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)) { println!("  alive: {} {:?} at {:?} hp {} spawn_t {} grounded {}", def.path(e.node), e.kind, e.pos, e.health, e.spawn_t, e.grounded) }
            break;
        }
        prev = g.s.player.pos;
        g.events.clear();
    }
}
