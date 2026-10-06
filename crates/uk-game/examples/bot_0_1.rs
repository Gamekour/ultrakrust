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
    let mut g = Game::new(def.clone());
    let mut bot = Bot::new(route_0_1());
    // optional section start: START_WP=<waypoint label> ACTIVATE="root1;root2" START=x,y,z
    if let Ok(list) = std::env::var("ACTIVATE") {
        for _ in 0..300 { g.fixed_update(&uk_core::player::Input::default()); g.update(&uk_core::player::Input::default(), FIXED_DT, 0.0); }
        for p in list.split(';') { if let Some(n) = (0..def.nodes.len() as u32).find(|&n| def.path(n) == p) { g.set_active(n, true); } }
        g.s.has_revolver = true;
    }
    if let Ok(start) = std::env::var("START") {
        let v: Vec<f32> = start.split(',').map(|x| x.parse().unwrap()).collect();
        g.s.player.pos = bevy_math::Vec3::new(v[0], v[1], v[2]);
        g.s.player.prev_pos = g.s.player.pos;
    }
    if let Ok(label) = std::env::var("START_WP") {
        bot.idx = bot.route.iter().position(|w| w.label == label).expect("waypoint label");
    }
    let mut t = 0.0f64;
    let mut deaths = 0;
    let mut last_report = 0.0;
    while (t as f32) < limit && !bot.done() && !g.s.level_complete {
        uk_game::bot::drive(&mut g, &mut bot, &mut t);
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
        if let Ok(w) = std::env::var("TRACE") {
            let v: Vec<f64> = w.split(',').map(|x| x.parse().unwrap()).collect();
            if t >= v[0] && t <= v[1] && (t * 2.0).fract() < 0.01 {
                let p = &g.s.player;
                let alive = g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)).count();
                let al: Vec<String> = g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)).map(|e| format!("{:?}@{:.0},{:.0},{:.0}{}", e.kind, e.pos.x, e.pos.y, e.pos.z, if e.attacking { "!" } else { "" })).collect();
                println!("  {t:6.1} pos {:.1},{:.1},{:.1} ground {} prog {:.1} wp {} alive {alive} {:?}", p.pos.x, p.pos.y, p.pos.z, p.gc.on_ground, bot.since_progress, bot.idx, al);
            }
        }
        if t - last_report > 30.0 {
            last_report = t;
            println!("{t:7.1}s  pos {:?} hp {} kills {} wp {}", g.s.player.pos, g.s.hp, g.s.kills, bot.idx);
        }
    }
    println!("end at {t:.1}s: waypoint {}/{}, kills {}, deaths {deaths}, complete {}", bot.idx, bot.route.len(), g.s.kills, g.s.level_complete);
    for e in g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)) {
        println!("  alive {:?} {} at {:?} vel {:?} grounded {} attacking {} hp {:.1} spawn_t {:.2}", e.kind, g.def.nodes[e.node as usize].name, e.pos, e.vel, e.grounded, e.attacking, e.health, e.spawn_t);
        if let Some(r) = e.rig {
            let (a, ci) = g.anim.rig_info(r);
            let c = &g.def.controllers[ci as usize].ctrl;
            let ls = &g.s.anim[r].layers[0];
            let m = &c.machines[c.layers[0].machine as usize];
            println!("    cd {:.2} at {:.2} hit_done {} anim_active {} state {} t {:.2} fade {}", e.cooldown, e.attack_t, e.hit_done, g.active(g.def.animators[a as usize].node), c.name(m.states[ls.state as usize].name), ls.time, ls.fade.is_some());
        }
    }
    println!("unknown calls: {:?}", g.unknown_calls);
}
