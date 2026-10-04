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
            let p = g.s.player.pos;
            let goal = bot.route[bot.idx].pos;
            let dir = (goal - p).with_y(0.0).normalize_or_zero();
            for h in [-1.4f32, -1.0, -0.5, 0.0, 0.8, 1.6] {
                println!("  ray +{h:4.1} toward wp: {:?}", g.world.raycast(p + bevy_math::Vec3::Y * h, dir, 12.0).map(|r| (r.distance, r.point, r.normal)));
            }
            for k in 0..18 { let x = p + dir * (0.2 + k as f32 * 0.8); let h = g.world.raycast(x.with_y(p.y + 2.5), bevy_math::Vec3::NEG_Y, 30.0); println!("  profile +{:4.1}: floor y {:?} n {:?}", 0.2 + k as f32 * 0.8, h.map(|r| r.point.y), h.map(|r| r.normal)); }
            println!("  floor below: {:?}", g.world.raycast(p, bevy_math::Vec3::NEG_Y, 5.0).map(|r| (r.distance, r.point, r.normal)));
            // who owns the geometry over the step (ClimbStep's forward capsule)
            let cap = uk_core::collide::Capsule { a: p + bevy_math::Vec3::new(0.5, 1.1, 0.0), b: p + bevy_math::Vec3::new(0.5, 3.6, 0.0), radius: 0.5 };
            for id in g.world.overlap_capsule(cap) {
                let o = g.world.owner(id);
                if o == uk_core::collide::ALWAYS { println!("  over step: static (always on)"); continue }
                let n = def.colliders[o as usize].node;
                let door = (0..def.nodes.len() as u32).rev().find(|&a| def.is_descendant(n, a) && def.scripts_on(a).any(|(_, s)| s.class == "Door"));
                let st = door.and_then(|d| def.scripts_on(d).find(|(_, s)| s.class == "Door").map(|(sc, _)| sc)).map(|sc| match &g.s.scripts[sc as usize] { uk_game::scripts::Script::Door(x) => format!("open={} locked={} pos={:?} open_pos={:?} closed_pos={:?}", x.open, x.locked, g.s.local_pos[def.scripts[sc as usize].node as usize], x.open_pos, x.closed_pos), _ => String::new() });
                println!("  over step: {} (mover {:?}) door {:?} {:?}", def.path(n), g.node_mover[n as usize], door.map(|d| def.path(d)), st);
            }
            if let Some(nav) = &g.nav {
                println!("  nav near feet {:?} near goal {:?}", nav.nearest(p - bevy_math::Vec3::Y * 1.5, bevy_math::Vec3::new(2.0, 4.0, 2.0)).map(|x| x.1), nav.nearest(goal, bevy_math::Vec3::new(2.0, 4.0, 2.0)).map(|x| x.1));
            }
            for e in g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)) { println!("  alive: {} {:?} at {:?} hp {} spawn_t {} grounded {}", def.path(e.node), e.kind, e.pos, e.health, e.spawn_t, e.grounded) }
            break;
        }
        prev = g.s.player.pos;
        g.events.clear();
    }
}
