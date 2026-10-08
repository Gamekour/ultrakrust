//! Script-spawned effects in the 0-1 Malicious Face fight: the bot fights it (with god-mode
//! health) and every effect prefab's spawn count and last spawn transform are printed, then the
//! face is killed and its corpse fall (impactParticle on the floor) is followed.
//! cargo run --release -p uk-game --example fx_boss_probe
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::particles::to_unity;
use uk_game::{
    bot::{Bot, Waypoint},
    Game,
};

fn report(g: &Game, label: &str) {
    println!("[{label}] t {:.2}", g.s.time);
    for (&p, &(n, pos, rot)) in &g.fx.spawns {
        println!("    {:24} x{n:3}  last at {:.2?} fwd {:.2?}", g.def.particle_prefabs[p as usize].name, pos, rot * Vec3::Z);
    }
}

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    let mut t = 0.0f64;
    for _ in 0..400 {
        g.fixed_update(&Input::default());
        t += FIXED_DT as f64;
        g.update(&Input::default(), FIXED_DT, t);
    }
    g.s.has_revolver = true;
    for r in ["13 - Malicious Face Arena", "12B - Pre-Boss Checkpoint"] {
        let n = (0..def.nodes.len() as u32).find(|&n| def.path(n) == r).unwrap();
        g.set_active(n, true);
    }
    g.s.player.pos = Vec3::new(202.0, 54.5, -425.0);
    g.s.player.prev_pos = g.s.player.pos;
    g.s.player.yaw_deg = 180.0;
    g.fx.spawns.clear();
    let mut bot = Bot::new(vec![Waypoint { pos: Vec3::new(202.0, 54.5, -416.0), label: "in", radius: 1.5, hold: 0.0 }]);
    for i in 0..(125 * 20) {
        g.s.hp = 100;
        let f = bot.think(&g, FIXED_DT);
        g.s.player.yaw_deg = f.yaw_deg;
        let fixed = Input { move_axis: f.input.move_axis, jump_held: f.input.jump_held, ..Default::default() };
        g.fixed_update(&fixed);
        t += FIXED_DT as f64;
        g.update(&f.input, FIXED_DT, t);
        if f.fire {
            let eye = g.s.player.pos + Vec3::Y * 1.4;
            let (y, p) = (f.yaw_deg.to_radians(), f.pitch_deg.to_radians());
            g.fire_revolver(eye, Vec3::new(y.sin() * p.cos(), p.sin(), -y.cos() * p.cos()), false);
        }
        g.events.clear();
        g.s.player.events.clear();
        if i % 625 == 624 {
            report(&g, "fight");
            if let Some(mf) = g.s.enemies.iter().find(|e| e.kind == uk_game::enemy::Kind::MaliciousFace) {
                println!("    face alive {} health {:.1} pos (unity) {:.2?} falling {} landed {}", mf.alive, mf.health, to_unity(mf.pos), mf.corpse_falling, mf.corpse_landed);
            }
        }
        let landed = g.s.enemies.iter().any(|e| e.kind == uk_game::enemy::Kind::MaliciousFace && e.corpse_landed);
        if landed {
            report(&g, "corpse landed");
            let mf = g.s.enemies.iter().find(|e| e.kind == uk_game::enemy::Kind::MaliciousFace).unwrap();
            println!("    face node world (unity) {:.2?}", g.node_world_now(mf.node).w_axis.truncate());
            return;
        }
    }
}
