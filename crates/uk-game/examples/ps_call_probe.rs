//! UnityEvent calls on scene ParticleSystems (Play / Stop / Clear): runs every such call of a
//! level through Game::run_call with the target GameObject active and prints the playing flag,
//! emission state and live particle count of the targeted system and its descendants after each.
//! cargo run --release -p uk-game --example ps_call_probe -- [level0-E]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::{parse_calls, Target};
use uk_game::Game;

fn walk(v: &uk_assets::serialized::Value, out: &mut Vec<uk_assets::serialized::Value>) {
    if v.has("m_PersistentCalls") {
        out.push(v.clone());
        return;
    }
    if let uk_assets::serialized::Value::Struct(m) = v {
        for (_, x) in m {
            walk(x, out)
        }
    }
    if let uk_assets::serialized::Value::Array(a) = v {
        for x in a {
            walk(x, out)
        }
    }
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-E".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let mut calls = Vec::new();
    for s in &def.scripts {
        let mut evs = Vec::new();
        walk(&s.data, &mut evs);
        for e in evs {
            for c in parse_calls(&def, &e) {
                if let Target::Particle(ps) = c.target {
                    calls.push((s.class.clone(), def.path(s.node), ps, c));
                }
            }
        }
    }
    let mut g = Game::new(def.clone());
    let mut t = 0.0f64;
    let idle = Input::default();
    for (class, from, ps, c) in calls {
        let root = def.particle_systems[ps as usize].node;
        let mut n = Some(root);
        while let Some(u) = n {
            g.s.active[u as usize] = true;
            n = def.nodes[u as usize].parent;
        }
        let show = |g: &Game, label: &str| {
            let v: Vec<_> = g
                .fx
                .scene
                .iter()
                .filter(|ss| {
                    let mut u = Some(ss.sys.node);
                    while u.is_some_and(|x| x != root) {
                        u = def.nodes[u.unwrap() as usize].parent;
                    }
                    u.is_some()
                })
                .map(|ss| (ss.sys.node, ss.on, ss.sys.playing, ss.sys.emitting(), ss.sys.particles.len()))
                .collect();
            println!("    {label}: (node, on, playing, emitting, particles) {v:?}");
        };
        println!("{class} on {from} -> {}.{} ({} systems)", def.path(root), c.method, def.particle_systems.iter().filter(|p| p.node == root).count());
        for _ in 0..30 {
            g.fixed_update(&idle);
            t += FIXED_DT as f64;
            g.update(&idle, FIXED_DT, t);
        }
        show(&g, "before");
        g.run_call(&c);
        show(&g, "after the call");
        for _ in 0..30 {
            g.fixed_update(&idle);
            t += FIXED_DT as f64;
            g.update(&idle, FIXED_DT, t);
        }
        show(&g, "half a second later");
    }
}
