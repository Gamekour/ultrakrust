//! EnemyIdentifier spawn effects: what is on a level's SpawnEffect objects (scripts, particle
//! systems, lights, scales), then the lifecycle of one after its wave activates: the bubble's
//! scale (-2/s), the light (enabled by SpawnEffect.Start, range -50/s), the particles, and
//! RemoveOnTime destroying the object.
//! spawn_fx [level] [wave path suffix]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let wave = std::env::args().nth(2).unwrap_or("3 - Gun Room/Enemies/Wave 2".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut fxs = Vec::new();
    for s in def.scripts.iter().filter(|s| s.class == "EnemyIdentifier") {
        if let Some(fx) = def.node_ref(s.data.get("spawnEffect")) {
            fxs.push((s.node, fx, s.data.get("spawnIn").bool()));
        }
    }
    println!("{} enemies with a spawnEffect ({} spawnIn)", fxs.len(), fxs.iter().filter(|f| f.2).count());
    let mut shown = std::collections::BTreeSet::new();
    for &(_, fx, _) in &fxs {
        if !shown.insert(def.nodes[fx as usize].name.clone()) {
            continue;
        }
        let mut stack = vec![(fx, 1)];
        while let Some((n, d)) = stack.pop() {
            let nd = &def.nodes[n as usize];
            let rs = def.renderers.iter().filter(|r| r.node == n).count();
            println!("{:w$}[{n}] {} active_self={} local_scale={} renderers={rs}", "", nd.name, nd.active_self, nd.local_scale, w = d * 2);
            for (_, sc) in def.scripts_on(n) {
                println!("{:w$}script {} {}", "", sc.class, sc.data.compact().chars().take(200).collect::<String>(), w = d * 2 + 2);
            }
            for p in def.particle_systems.iter().filter(|p| p.node == n) {
                println!("{:w$}particles play_on_awake={} duration={} looping={}", "", p.system.play_on_awake, p.system.duration, p.system.looping, w = d * 2 + 2);
            }
            for l in def.lights.iter().filter(|l| l.node == n) {
                println!("{:w$}light kind {} intensity {} range {} enabled {}", "", l.kind, l.intensity, l.range, l.enabled, w = d * 2 + 2);
            }
            stack.extend(nd.children.iter().rev().map(|&c| (c, d + 1)));
        }
    }

    let Some(wave_node) = (0..def.nodes.len() as u32).find(|&n| def.path(n).ends_with(&wave)) else {
        println!("no node {wave}");
        return;
    };
    let Some(&(enemy, fx, _)) = fxs.iter().find(|f| f.2 && def.is_descendant(f.0, wave_node)) else {
        println!("no spawnIn enemy under {wave}");
        return;
    };
    println!("activating {} -> {}", def.path(wave_node), def.path(enemy));
    let mut g = Game::new(def.clone());
    let idle = Input::default();
    let mut t = 0.0f64;
    let bubble = def.nodes[fx as usize].children[0];
    let light = def.lights.iter().position(|l| def.is_descendant(l.node, fx));
    let systems: Vec<usize> = (0..g.fx.scene.len()).filter(|&k| def.is_descendant(g.fx.scene[k].sys.node, fx)).collect();
    // what a wave does: the enemy (and any inactive ancestor) goes active
    let mut up = Some(enemy);
    while let Some(n) = up {
        g.set_active(n, true);
        up = def.nodes[n as usize].parent;
    }
    for k in 0..=320 {
        if k % 25 == 0 || k == 1 {
            let parts: usize = systems.iter().map(|&s| g.fx.scene[s].sys.particles.len()).sum();
            let playing = systems.iter().any(|&s| g.fx.scene[s].sys.playing);
            let world_x = (g.scale_delta(bubble) * g.anim.world_of(bubble)).x_axis.truncate().length();
            println!(
                "t={:.2}s fx active={} destroyed={} bubble local={:.3} world x={:.3} light={:?} particles={} playing={}",
                k as f32 * FIXED_DT,
                g.active(fx),
                g.s.destroyed[fx as usize],
                g.local_scale_of(bubble).x,
                world_x,
                light.map(|l| (g.s.lights[l].0, (g.s.lights[l].1 * 100.0).round() / 100.0)),
                parts,
                playing
            );
        }
        g.fixed_update(&idle);
        t += FIXED_DT as f64;
        g.update(&idle, FIXED_DT, t);
    }
    println!("script scale overrides left: {}", g.s.local_scale.len());
}
