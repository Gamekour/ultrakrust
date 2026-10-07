//! Bot to the first checkpoint, then die: respawn must be at the checkpoint with its saved state.
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::{bot::{route_0_1, Bot}, Game, GameEvent};
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    let mut bot = Bot::new(route_0_1());
    let mut t = 0.0f64;
    let mut got_cp = false;
    for _ in 0..(125 * 120) {
        let f = bot.think(&g, FIXED_DT);
        g.s.player.yaw_deg = f.yaw_deg;
        let fixed = Input { move_axis: f.input.move_axis, jump_held: f.input.jump_held, ..Default::default() };
        g.fixed_update(&fixed); t += FIXED_DT as f64; g.update(&f.input, FIXED_DT, t);
        let eye = g.s.player.pos + bevy_math::Vec3::Y * 1.4; let (y, p) = (f.yaw_deg.to_radians(), f.pitch_deg.to_radians());
        let aim = bevy_math::Vec3::new(y.sin() * p.cos(), p.sin(), -y.cos() * p.cos());
        if f.fire && g.s.has_revolver { g.fire_revolver(eye, aim, false); }
        if f.punch { g.punch(eye, aim); }
        if g.events.drain(..).any(|e| e == GameEvent::Checkpoint) { got_cp = true; println!("checkpoint at t={t:.1} pos {:?} kills {}", g.s.player.pos, g.s.kills); break; }
        g.s.player.events.clear();
    }
    if !got_cp { println!("no checkpoint reached"); return; }
    let kills_at_cp = g.s.kills;
    // walk on a bit, then die
    for _ in 0..250 { g.fixed_update(&Input::default()); t += FIXED_DT as f64; g.update(&Input::default(), FIXED_DT, t); }
    g.hurt_player(999, false);
    println!("died: dead={} hp={}", g.s.dead, g.s.hp);
    let u = &g.death_ui;
    println!("death ui: {} lines (delay {}), first {:?}, last {:?}, black {:?}, you_died {:?} {:?}, text_over_black {}", u.lines.len(), u.line_delay, u.lines.first(), u.lines.last(), u.black, u.you_died, u.you_died_color, u.text_over_black);
    for t in [0.0, 0.049, 0.05, 1.0, 1.85, 2.0] { print!("lines@{t}={} ", u.lines_shown(t)); }
    println!();
    for _ in 0..260 { g.fixed_update(&Input::default()); t += FIXED_DT as f64; g.update(&Input::default(), FIXED_DT, t); }
    println!("death screen: dead={} black_screen={} t={:.2}", g.s.dead, g.s.death_screen, g.s.dead_timer);
    g.death_restart();
    println!("after respawn: dead={} hp={} pos {:?} kills {} (at checkpoint {}) has_revolver {}", g.s.dead, g.s.hp, g.s.player.pos, g.s.kills, kills_at_cp, g.s.has_revolver);
    let ev: Vec<_> = g.events.drain(..).collect();
    println!("events: {:?}", ev.iter().filter(|e| matches!(e, GameEvent::Respawned | GameEvent::Died)).collect::<Vec<_>>());
}
