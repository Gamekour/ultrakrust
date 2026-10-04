//! Headless boss + ending probe for 0-1 (god mode).
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef::{self, ShapeDef}};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    let mut t = 0.0f64;
    let find = |p: &str| (0..def.nodes.len() as u32).find(|&n| def.path(n) == p).unwrap();
    let mut step = |g: &mut Game, n: usize, t: &mut f64| {
        for _ in 0..n {
            g.fixed_update(&Input::default());
            *t += FIXED_DT as f64;
            g.update(&Input::default(), FIXED_DT, *t);
            g.s.hp = 100; // god mode for the probe
            g.s.player.events.clear();
        }
    };
    step(&mut g, 400, &mut t); // land
    // the arena room is switched on by earlier progression; do it directly here
    for r in ["13 - Malicious Face Arena", "12B - Pre-Boss Checkpoint"] {
        g.set_active(find(r), true);
    }
    let trig = def.scripts.iter().position(|s| s.class == "ActivateArena" && def.path(s.node) == "13 - Malicious Face Arena/13 Content/Trigger").unwrap();
    let tnode = def.scripts[trig].node;
    println!("trigger active={}", g.active(tnode));
    let c = def.colliders.iter().find(|c| c.node == tnode).unwrap();
    let ShapeDef::Box { center, .. } = c.shape else { panic!() };
    g.s.player.pos = center;
    g.s.player.prev_pos = center;
    step(&mut g, 250, &mut t);
    let mf = g.s.enemies.iter().position(|e| e.kind == uk_game::enemy::Kind::MaliciousFace).unwrap();
    println!("MF active={} health={} at {:?}; player {:?}", g.active(g.s.enemies[mf].node), g.s.enemies[mf].health, g.s.enemies[mf].center(), g.s.player.pos);
    let mut shots = 0;
    let mut hurt = 0;
    while g.s.enemies[mf].alive && shots < 200 {
        let eye = g.s.player.pos + Vec3::Y * 1.4;
        let aim = (g.s.enemies[mf].center() - eye).normalize();
        g.fire_revolver(eye, aim, false);
        shots += 1;
        step(&mut g, 62, &mut t);
        hurt += g.events.iter().filter(|e| matches!(e, uk_game::GameEvent::Hurt(_))).count();
        g.events.clear();
    }
    println!("MF dead after {shots} shots ({} s); player got hit {hurt} times", shots as f32 * 0.5);
    step(&mut g, 600, &mut t);
    // final door / pit
    let fd = def.scripts.iter().position(|s| s.class == "FinalDoor" && def.path(s.node).contains("13 - Malicious")).unwrap();
    if let uk_game::scripts::Script::FinalDoor(f) = &g.s.scripts[fd] {
        println!("final door: about_to_open={} opened={} (node active {})", f.about_to_open, f.opened, g.active(def.scripts[fd].node));
    }
    let pit = def.scripts.iter().position(|s| s.class == "FinalPit").unwrap();
    let pnode = def.scripts[pit].node;
    println!("final pit active={} at {}", g.active(pnode), def.path(pnode));
    if let Some(pc) = def.colliders.iter().find(|c| c.node == pnode) {
        if let ShapeDef::Box { center, .. } = pc.shape {
            g.s.player.pos = center;
            g.s.player.prev_pos = center;
            step(&mut g, 20, &mut t);
        }
    }
    println!("level complete: {}", g.s.level_complete);
    println!("unknown calls: {:?}", g.unknown_calls);
}
