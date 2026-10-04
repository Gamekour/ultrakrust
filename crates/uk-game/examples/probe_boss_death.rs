//! Reproduce the demo's boss entry without god mode and log damage.
use bevy_math::{Vec2, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::{bot::{Bot, Waypoint}, Game, GameEvent};
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    let mut t = 0.0f64;
    for _ in 0..400 { g.fixed_update(&Input::default()); t += FIXED_DT as f64; g.update(&Input::default(), FIXED_DT, t); }
    g.s.has_revolver = true;
    for r in ["13 - Malicious Face Arena", "12B - Pre-Boss Checkpoint"] {
        let n = (0..def.nodes.len() as u32).find(|&n| def.path(n) == r).unwrap(); g.set_active(n, true);
    }
    g.s.player.pos = Vec3::new(202.0, 54.5, -425.0); g.s.player.prev_pos = g.s.player.pos; g.s.player.yaw_deg = 180.0;
    let mut bot = Bot::new(vec![Waypoint { pos: Vec3::new(202.0, 54.5, -416.0), label: "in", radius: 1.5 }]);
    for i in 0..(125 * 20) {
        let f = bot.think(&g, FIXED_DT);
        g.s.player.yaw_deg = f.yaw_deg;
        let fixed = Input { move_axis: f.input.move_axis, jump_held: f.input.jump_held, ..Default::default() };
        g.fixed_update(&fixed); t += FIXED_DT as f64; g.update(&f.input, FIXED_DT, t);
        if f.fire {
            let eye = g.s.player.pos + Vec3::Y * 1.4; let (y, p) = (f.yaw_deg.to_radians(), f.pitch_deg.to_radians());
            g.fire_revolver(eye, Vec3::new(y.sin() * p.cos(), p.sin(), -y.cos() * p.cos()), false);
        }
        for e in g.events.drain(..) {
            match e {
                GameEvent::Hurt(d) => println!("{:6.2}s hurt {d} -> hp {} at {:?} (inv_layer {}) projectiles {}", i as f32 * FIXED_DT, g.s.hp, g.s.player.pos, g.s.player.invincible_layer, g.s.projectiles.len()),
                GameEvent::Died => { println!("{:6.2}s DIED at {:?}", i as f32 * FIXED_DT, g.s.player.pos); return; }
                _ => {}
            }
        }
        g.s.player.events.clear();
        if i % 250 == 0 { println!("{:6.2}s pos {:?} hp {} mf_alive {}", i as f32 * FIXED_DT, g.s.player.pos, g.s.hp, g.s.enemies.iter().any(|e| e.alive && g.active(e.node))); }
        let _ = Vec2::ZERO;
    }
}
