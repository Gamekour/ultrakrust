//! Every ActivateArena in 0-1, each in a fresh game: trigger it, kill all waves,
//! and check that the doors it locked end up unlocked again.
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef::{self, ShapeDef}};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::Script;
use uk_game::Game;

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let arenas: Vec<usize> = def
        .scripts
        .iter()
        .enumerate()
        .filter(|(_, s)| s.class == "ActivateArena")
        .filter(|(_, s)| {
            let p = def.path(s.node);
            !p.contains("OLD") && !p.contains("Alt")
        })
        .map(|(i, _)| i)
        .collect();
    let base = Game::new(def.clone());
    for sc in arenas {
        let node = def.scripts[sc].node;
        let mut g = Game::new(def.clone());
        let _ = &base;
        let mut t = 0.0f64;
        let step = |g: &mut Game, n: usize, t: &mut f64| {
            for _ in 0..n {
                g.fixed_update(&Input::default());
                *t += FIXED_DT as f64;
                g.update(&Input::default(), FIXED_DT, *t);
                g.s.hp = 100;
                g.s.player.events.clear();
                g.events.clear();
            }
        };
        step(&mut g, 300, &mut t);
        // simulate earlier progression: switch on every ancestor of the trigger
        let mut chain = vec![node];
        while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent {
            chain.push(p);
        }
        for n in chain.iter().rev() {
            g.set_active(*n, true);
        }
        let Some(c) = def.colliders.iter().find(|c| c.node == node) else {
            println!("{:60} no collider", def.path(node));
            continue;
        };
        let center = match &c.shape {
            ShapeDef::Box { center, .. } => *center,
            ShapeDef::Sphere { center, .. } => *center,
            _ => continue,
        };
        g.s.player.pos = center;
        g.s.player.prev_pos = center;
        step(&mut g, 200, &mut t);
        let locked_doors: Vec<u32> = match &g.s.scripts[sc] {
            Script::Arena(a) => a.doors.clone(),
            _ => vec![],
        };
        let activated = matches!(&g.s.scripts[sc], Script::Arena(a) if a.activated);
        let mut spawned = 0;
        for _round in 0..12 {
            step(&mut g, 150, &mut t);
            let ids: Vec<usize> = (0..g.s.enemies.len()).filter(|&i| g.s.enemies[i].alive && g.active(g.s.enemies[i].node) && g.s.enemies[i].spawn_t <= 0.0).collect();
            spawned += ids.len();
            if ids.is_empty() {
                step(&mut g, 300, &mut t);
                if (0..g.s.enemies.len()).all(|i| !(g.s.enemies[i].alive && g.active(g.s.enemies[i].node))) {
                    break;
                }
            }
            for i in ids {
                uk_game::enemy::kill_enemy(&mut g, i);
            }
        }
        step(&mut g, 400, &mut t);
        let still_locked: Vec<String> = locked_doors
            .iter()
            .filter(|&&d| matches!(&g.s.scripts[d as usize], Script::Door(dd) if dd.locked))
            .map(|&d| def.path(def.scripts[d as usize].node))
            .collect();
        println!(
            "{:62} activated={} killed={:2} doors locked by it={} still locked={:?}",
            def.path(node),
            activated,
            spawned,
            locked_doors.len(),
            still_locked
        );
        if !g.unknown_calls.is_empty() {
            println!("    unknown calls: {:?}", g.unknown_calls);
        }
    }
}
