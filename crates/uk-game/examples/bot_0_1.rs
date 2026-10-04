//! Headless autopilot run through 0-1: cargo run --release -p uk-game --example bot_0_1 [seconds]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_game::bot::{route_0_1, Bot};
use uk_game::Game;

fn main() {
    let limit: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(600.0);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def);
    let mut bot = Bot::new(route_0_1());
    let mut t = 0.0f64;
    let mut deaths = 0;
    let mut last_report = 0.0;
    while (t as f32) < limit && !bot.done() && !g.s.level_complete {
        let f = bot.think(&g, FIXED_DT);
        g.s.player.yaw_deg = f.yaw_deg;
        let eye = g.s.player.pos + bevy_math::Vec3::Y * 1.4;
        let pitch = f.pitch_deg.to_radians();
        let yaw = f.yaw_deg.to_radians();
        let aim = bevy_math::Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
        let fixed = uk_core::player::Input { move_axis: f.input.move_axis, jump_held: f.input.jump_held, ..Default::default() };
        g.fixed_update(&fixed);
        t += FIXED_DT as f64;
        g.update(&f.input, FIXED_DT, t);
        if f.fire && g.s.has_revolver {
            g.fire_revolver(eye, aim, false);
        }
        if f.punch {
            g.punch(eye, aim);
        }
        for e in g.events.drain(..) {
            match e {
                uk_game::GameEvent::Died => {
                    deaths += 1;
                    println!("{t:7.1}s DIED at {:?} (wp {})", g.s.player.pos, bot.idx);
                }
                uk_game::GameEvent::Checkpoint => println!("{t:7.1}s checkpoint"),
                uk_game::GameEvent::WeaponGot(w) => println!("{t:7.1}s got {w}"),
                uk_game::GameEvent::LevelComplete => println!("{t:7.1}s LEVEL COMPLETE"),
                _ => {}
            }
        }
        g.s.player.events.clear();
        for l in bot.log.drain(..) {
            println!("{l}");
        }
        if t - last_report > 30.0 {
            last_report = t;
            println!("{t:7.1}s  pos {:?} hp {} kills {} wp {}", g.s.player.pos, g.s.hp, g.s.kills, bot.idx);
        }
    }
    println!("end at {t:.1}s: waypoint {}/{}, kills {}, deaths {deaths}, complete {}", bot.idx, bot.route.len(), g.s.kills, g.s.level_complete);
    println!("unknown calls: {:?}", g.unknown_calls);
}
