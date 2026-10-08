//! ActivateArena waitForStatus / ArenaStatus gating. Lists every arena that waits for a status;
//! then, for each ArenaStatus that has such arenas, in a fresh game with the room's chain active:
//! the player stands in each of its arena triggers in turn at the starting status (only the
//! waitForStatus 0 ones may activate), then AddToStatus(1) is called repeatedly while the player
//! stays in the last trigger, reporting which arenas activate at each status.
//! cargo run --release -p uk-game --example arena_status -- [level2-3]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef::{self, ShapeDef}};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::{Call, Script, Target};
use uk_game::Game;

fn step(g: &mut Game, n: usize, t: &mut f64) {
    for _ in 0..n {
        g.fixed_update(&Input::default());
        *t += FIXED_DT as f64;
        g.update(&Input::default(), FIXED_DT, *t);
        g.s.hp = 100;
        g.s.player.events.clear();
        g.events.clear();
    }
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level2-3".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let g0 = Game::new(def.clone());
    let wait = |g: &Game, sc: usize| match &g.s.scripts[sc] {
        Script::Arena(a) => a.wait_for_status,
        _ => 0,
    };
    let status = |g: &Game, sc: u32| match g.s.scripts[sc as usize] {
        Script::ArenaStatus { status } => status,
        _ => i64::MIN,
    };
    let arenas: Vec<usize> = (0..def.scripts.len()).filter(|&i| matches!(g0.s.scripts[i], Script::Arena(_))).collect();
    let stats: Vec<u32> = (0..def.scripts.len() as u32).filter(|&i| matches!(g0.s.scripts[i as usize], Script::ArenaStatus { .. })).collect();
    // GetComponentInParent from the arena's node
    let owner = |sc: usize| {
        let mut n = Some(def.scripts[sc].node);
        while let Some(m) = n {
            if let Some(&s) = stats.iter().find(|&&s| def.scripts[s as usize].node == m) {
                return Some(s);
            }
            n = def.nodes[m as usize].parent;
        }
        None
    };
    for &s in &stats {
        println!("ArenaStatus #{s} {} currentStatus {}", def.path(def.scripts[s as usize].node), status(&g0, s));
    }
    for &sc in &arenas {
        if wait(&g0, sc) > 0 {
            println!("  waits for {}: {} (status #{:?})", wait(&g0, sc), def.path(def.scripts[sc].node), owner(sc));
        }
    }
    for &st in &stats {
        let mine: Vec<usize> = arenas.iter().copied().filter(|&a| owner(a) == Some(st)).collect();
        if !mine.iter().any(|&a| wait(&g0, a) > 0) {
            continue;
        }
        println!("\n== {}", def.path(def.scripts[st as usize].node));
        let mut g = Game::new(def.clone());
        let mut t = 0.0;
        step(&mut g, 60, &mut t);
        g.s.player.activated = true;
        let mut chain = vec![def.scripts[st as usize].node];
        while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent {
            chain.push(p);
        }
        for n in chain.iter().rev() {
            g.set_active(*n, true);
        }
        let report = |g: &Game, label: &str| {
            let live = (0..g.s.enemies.len()).filter(|&i| g.s.enemies[i].alive && g.active(g.s.enemies[i].node)).count();
            let line: Vec<String> = mine
                .iter()
                .map(|&a| match &g.s.scripts[a] {
                    Script::Arena(x) => format!("{}[w{} {}{}]", def.nodes[def.scripts[a].node as usize].name, x.wait_for_status, if x.activated { "ON" } else { "off" }, if x.player_in { " in" } else { "" }),
                    _ => String::new(),
                })
                .collect();
            println!("  {label:28} status {} live enemies {live:3} | {}", status(g, st), line.join(" "));
        };
        report(&g, "start");
        let mut last = None;
        for &a in &mine {
            let node = def.scripts[a].node;
            if !g.active(node) {
                continue;
            }
            let Some(c) = def.colliders.iter().find(|c| c.node == node && c.trigger) else { continue };
            let center = match &c.shape {
                ShapeDef::Box { center, .. } | ShapeDef::Sphere { center, .. } => *center,
                _ => continue,
            };
            g.s.player.pos = center;
            g.s.player.prev_pos = center;
            g.s.player.vel = bevy_math::Vec3::ZERO;
            step(&mut g, 30, &mut t);
            report(&g, &format!("in {}", def.nodes[node as usize].name));
            last = Some(center);
        }
        // raise the status with the player held in the last trigger
        for k in 0..3 {
            g.run_call(&Call { target: Target::Script(st), method: "AddToStatus".into(), bool_arg: false, float_arg: 0.0, int_arg: 1, string_arg: String::new() });
            for _ in 0..30 {
                if let Some(c) = last {
                    g.s.player.pos = c;
                    g.s.player.prev_pos = c;
                }
                step(&mut g, 1, &mut t);
            }
            report(&g, &format!("AddToStatus(1) #{}", k + 1));
        }
        // re-enter every trigger now that the status is reached
        for &a in &mine {
            let node = def.scripts[a].node;
            let Some(c) = def.colliders.iter().find(|c| c.node == node && c.trigger) else { continue };
            let center = match &c.shape {
                ShapeDef::Box { center, .. } | ShapeDef::Sphere { center, .. } => *center,
                _ => continue,
            };
            g.s.player.pos = center;
            g.s.player.prev_pos = center;
            step(&mut g, 30, &mut t);
            report(&g, &format!("re-enter {}", def.nodes[node as usize].name));
        }
        for &a in &mine {
            if let Script::Arena(x) = &g.s.scripts[a] {
                let en: Vec<String> = x.enemies.iter().map(|&n| format!("{} active {}", def.path(n), g.active(n))).collect();
                println!("  {}: doors {:?} enemies {:?}", def.nodes[def.scripts[a].node as usize].name, x.doors, en);
            }
        }
    }
}
