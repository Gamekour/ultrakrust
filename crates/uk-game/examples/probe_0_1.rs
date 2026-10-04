//! Headless probe of 0-1's progression: cargo run --release -p uk-game --example probe_0_1
use bevy_math::{Vec2, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::Script;
use uk_game::Game;

fn step(g: &mut Game, input: Input, frames: usize, t: &mut f64) {
    for _ in 0..frames {
        g.fixed_update(&input);
        *t += FIXED_DT as f64;
        g.update(&input, FIXED_DT, *t);
        g.s.player.events.clear();
    }
}

fn node_pos(g: &Game, name_path_end: &str) -> Option<(u32, Vec3)> {
    (0..g.def.nodes.len() as u32)
        .find(|&n| g.def.path(n).ends_with(name_path_end))
        .map(|n| (n, g.def.nodes[n as usize].world0.w_axis.truncate()))
}

fn main() {
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let t0 = std::time::Instant::now();
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    println!("game built in {:?}: {} movers, {} triggers, {} enemies", t0.elapsed(), g.movers.len(), g.triggers.len(), g.s.enemies.len());
    let active_rooms: Vec<String> = (0..def.nodes.len() as u32)
        .filter(|&n| def.nodes[n as usize].parent.is_none() && g.active(n))
        .map(|n| def.nodes[n as usize].name.clone())
        .collect();
    println!("active roots at start: {active_rooms:?}");
    println!("player at {:?} activated={}", g.s.player.pos, g.s.player.activated);
    let mut t = 0.0;
    // 1. fall into the first room
    for i in 0..2000 {
        step(&mut g, Input::default(), 1, &mut t);
        if g.s.player.gc.on_ground {
            println!("landed after {i} ticks at {:?}; activated={}", g.s.player.pos, g.s.player.activated);
            break;
        }
    }
    step(&mut g, Input::default(), 30, &mut t);
    println!("after settling: activated={} hp={}", g.s.player.activated, g.s.hp);
    // 3. walk forward for 2 s
    let walk = Input { move_axis: Vec2::Y, ..Default::default() };
    let p0 = g.s.player.pos;
    step(&mut g, walk, 250, &mut t);
    println!("walked {:.1} units to {:?}", (g.s.player.pos - p0).length(), g.s.player.pos);
    // 2. punch the planks ahead
    let fwd = g.s.player.forward();
    for _ in 0..3 {
        let eye = g.s.player.pos + Vec3::Y * 1.4;
        g.punch(eye, fwd);
        g.punch(eye + Vec3::Y * 1.5, fwd);
        step(&mut g, Input::default(), 10, &mut t);
    }
    let planks: Vec<String> = (0..def.nodes.len() as u32)
        .filter(|&n| def.path(n).contains("Starting Room/Planks/Plank"))
        .map(|n| format!("{}:{}", def.nodes[n as usize].name, if g.s.destroyed[n as usize] { "broken" } else { "intact" }))
        .collect();
    println!("planks: {planks:?}");
    step(&mut g, walk, 250, &mut t);
    println!("after punching, walked on to {:?}", g.s.player.pos);
    // 4. teleport into the revolver pickup
    if let Some((_, pos)) = node_pos(&g, "3 - Gun Room/RevolverPickUp") {
        g.s.player.pos = pos + Vec3::Y * 0.5;
        g.s.player.prev_pos = g.s.player.pos;
        step(&mut g, Input::default(), 30, &mut t);
        // the title card runs ~8.9 s before the arena trigger switches on
        step(&mut g, Input::default(), 1200, &mut t);
        println!("after revolver pickup: has_revolver={} events={:?}", g.s.has_revolver, g.events.iter().rev().take(3).collect::<Vec<_>>());
        for sc in 0..def.scripts.len() {
            if def.path(def.scripts[sc].node).ends_with("Gun Room/TitleActivator") {
                if let Script::ObjectActivator(oa) = &g.s.scripts[sc] {
                    println!("  TitleActivator oa delay={} activated={} on={:?} off={:?}", oa.delay, oa.activated,
                        oa.events.to_activate.iter().map(|&n| def.path(n)).collect::<Vec<_>>(),
                        oa.events.to_deactivate.iter().map(|&n| def.path(n)).collect::<Vec<_>>());
                }
            }
            if def.path(def.scripts[sc].node).ends_with("Gun Room/RevolverPickUp") {
                if let Script::ObjectActivator(oa) = &g.s.scripts[sc] {
                    println!("  pickup activated={} -> {:?}", oa.activated, oa.events.to_activate.iter().map(|&n| (def.path(n), g.active(n))).collect::<Vec<_>>());
                }
            }
        }
    }
    // 5. gun room arena trigger
    let arena_trigger = def.scripts.iter().enumerate().find(|(_, s)| s.class == "ActivateArena" && def.path(s.node).contains("3 - Gun Room"));
    if let Some((sc, s)) = arena_trigger {
        println!("gun room arena trigger active={} script={}", g.active(s.node), sc);
        let c = def.colliders.iter().find(|c| c.node == s.node).map(|c| c.shape.clone());
        if let Some(uk_assets::scenedef::ShapeDef::Box { center, .. }) = c {
            g.s.player.pos = center;
            g.s.player.prev_pos = center;
            step(&mut g, Input::default(), 300, &mut t);
            let alive: Vec<_> = g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)).map(|e| def.path(e.node)).collect();
            println!("after entering arena: {} enemies active: {:?}", alive.len(), alive);
            if let Script::Arena(a) = &g.s.scripts[sc] {
                println!("arena activated={}", a.activated);
            }
        } else {
            println!("arena trigger has no box collider (active={})", g.active(s.node));
        }
    }
    // 6. kill everything active, repeatedly, and watch the waves
    for round in 0..6 {
        let ids: Vec<usize> = (0..g.s.enemies.len()).filter(|&i| g.s.enemies[i].alive && g.active(g.s.enemies[i].node)).collect();
        for i in &ids {
            uk_game::enemy::kill_enemy(&mut g, *i);
        }
        step(&mut g, Input::default(), 400, &mut t);
        let alive = g.s.enemies.iter().filter(|e| e.alive && g.active(e.node)).count();
        println!("round {round}: killed {}, now {} alive; kills={} hp={} dead={}", ids.len(), alive, g.s.kills, g.s.hp, g.s.dead);
        if ids.is_empty() && alive == 0 {
            break;
        }
    }
    let locked: Vec<String> = g
        .s
        .scripts
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match s {
            Script::Door(d) if d.locked => Some(def.path(def.scripts[i].node)),
            _ => None,
        })
        .collect();
    println!("doors still locked: {}", locked.len());
    for l in locked.iter().take(10) {
        println!("  {l}");
    }
    println!("unknown calls: {:?}", g.unknown_calls);
}
