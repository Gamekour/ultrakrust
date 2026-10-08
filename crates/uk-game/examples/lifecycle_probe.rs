//! Effect lifecycle scripts: RemoveOnTime on child nodes (PowerUpEnd's DustBig child, 2 s; the
//! RubbleFilth chunks, 1 ± 1 s) and DestroyOnCheckpointRestart (Explosion) on a checkpoint
//! restart vs a level-start respawn.
//! cargo run --release -p uk-game --example lifecycle_probe -- [level0-1]
use bevy_math::{Quat, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let mut g = Game::new(def.clone());
    let mut t = 0.0f64;
    let idle = Input::default();
    let mut step = |g: &mut Game| {
        g.fixed_update(&idle);
        t += FIXED_DT as f64;
        g.update(&idle, FIXED_DT, t);
    };
    let find = |name: &str| def.particle_prefabs.iter().position(|p| p.name == name).map(|i| i as u32);
    for name in ["PowerUpEnd", "RubbleFilth"] {
        let Some(pf) = find(name) else {
            println!("{name}: not in this scene");
            continue;
        };
        let i = g.fx_instantiate(pf, Vec3::new(0.0, 10.0, 0.0), Quat::IDENTITY);
        let p = &def.particle_prefabs[pf as usize];
        println!("{name}: root removal in {:?}", g.fx.effects[i].remove_at.map(|at| ((at - g.s.time) * 100.0).round() / 100.0));
        println!("{name}: child removals {:?}", g.fx.effects[i].child_remove.iter().map(|(n, at)| (p.nodes[*n as usize].name.clone(), ((at - g.s.time) * 100.0).round() / 100.0)).collect::<Vec<_>>());
        for k in 0..180 {
            step(&mut g);
            if k % 30 == 29 {
                let Some(e) = g.fx.effects.get(i).filter(|e| e.prefab == pf) else {
                    println!("  t+{:.1}s gone", (k + 1) as f32 * FIXED_DT);
                    break;
                };
                println!(
                    "  t+{:.1}s destroyed {} inactive nodes {:?} systems (node, playing, particles) {:?}",
                    (k + 1) as f32 * FIXED_DT,
                    e.destroyed,
                    (0..p.nodes.len()).filter(|&n| !e.node_active[n] && p.nodes[n].active_self).map(|n| p.nodes[n].name.clone()).collect::<Vec<_>>(),
                    e.systems.iter().map(|s| (s.node, s.playing, s.particles.len())).collect::<Vec<_>>()
                );
            }
        }
    }
    let Some(ex) = find("Explosion") else {
        println!("Explosion: not in this scene");
        return;
    };
    for checkpoint in [false, true] {
        if checkpoint {
            g.checkpoint = Some(Box::new(g.s.clone()));
        }
        let i = g.fx_instantiate(ex, Vec3::new(0.0, 10.0, 0.0), Quat::IDENTITY);
        step(&mut g);
        g.respawn();
        println!("Explosion, respawn with checkpoint {checkpoint}: destroyed {}", g.fx.effects[i].destroyed);
    }
}
