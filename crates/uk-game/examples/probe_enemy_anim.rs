//! Enemy Animators driven by game logic: each Filth/Stray in 0-1 in a fresh game, switched on
//! with the player standing nearby. Reports the rig's states visited, clip events fired, attacks
//! started/finished and damage dealt.
//! cargo run --release -p uk-game --example probe_enemy_anim
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::enemy::Kind;
use uk_game::game::GameEvent;
use uk_game::Game;

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let base = Game::new(def.clone());
    let ens = &base.s.enemies;
    let of = |k: Kind, n: usize| (0..ens.len()).filter(move |&i| ens[i].kind == k).take(n);
    let picks: Vec<usize> = of(Kind::Filth, 3).chain(of(Kind::Stray, 3)).collect();
    let bound = base.s.enemies.iter().filter(|e| e.rig.is_some()).count();
    println!("enemies {} with rig {}", base.s.enemies.len(), bound);
    for i in picks {
        let mut g = Game::new(def.clone());
        let node = g.s.enemies[i].node;
        let mut chain = vec![node];
        while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent {
            chain.push(p);
        }
        for n in chain.iter().rev() {
            g.set_active(*n, true);
        }
        let en = &g.s.enemies[i];
        let fwd = bevy_math::Vec3::new(en.yaw.sin(), 0.0, -en.yaw.cos());
        let d = if en.kind == Kind::Filth { 6.0 } else { 25.0 };
        g.s.player.pos = en.pos + fwd * d + bevy_math::Vec3::Y * 1.0;
        g.s.player.prev_pos = g.s.player.pos;
        let rig = en.rig;
        let anode = rig.map(|r| def.animators[g.anim.rig_info(r).0 as usize].node);
        let (mut t, mut dmg, mut attacks, mut ends, mut running) = (0.0f64, 0, 0, 0, 0);
        let mut evs: BTreeMap<String, usize> = BTreeMap::new();
        let mut states: BTreeMap<String, usize> = BTreeMap::new();
        let mut was = false;
        let mut last = String::new();
        for f in 0..900 {
            g.fixed_update(&Input::default());
            t += FIXED_DT as f64;
            g.update(&Input::default(), FIXED_DT, t);
            for e in g.events.drain(..) {
                match e {
                    GameEvent::AnimEvent { node, function, .. } if Some(node) == anode => *evs.entry(function).or_default() += 1,
                    GameEvent::Hurt(h) => dmg += h,
                    _ => {}
                }
            }
            g.s.hp = 100;
            g.s.player.events.clear();
            let en = &g.s.enemies[i];
            attacks += (en.attacking && !was) as usize;
            ends += (!en.attacking && was) as usize;
            was = en.attacking;
            running += (bevy_math::Vec3::new(en.vel.x, 0.0, en.vel.z).length() > 0.1) as usize;
            if let Some(r) = rig {
                let c = &def.controllers[g.anim.rig_info(r).1 as usize].ctrl;
                let ls = &g.s.anim[r].layers[0];
                let m = &c.machines[c.layers[0].machine as usize];
                let sn = c.name(m.states[ls.state as usize].name).to_string();
                if std::env::var("DBG").is_ok() {
                    let fd = ls.fade.as_ref().map(|f| format!("->{} {:.2}/{:.2}", c.name(m.states[f.dest as usize].name), f.elapsed, f.duration)).unwrap_or_default();
                    let line = format!("{sn} att={} act={} alive={} gr={} {fd}", en.attacking, g.active(node), en.alive, en.grounded);
                    if states.keys().last().is_none() || last != line { println!("  f{f} t={:.2} at={:.2} {line}", ls.time, en.attack_t); last = line; }
                }
                *states.entry(sn).or_default() += 1;
            }
        }
        println!(
            "{:40} {:?} rig={} alive={} attacks {attacks} ended {ends} dmg {dmg} moving_frames {running}\n    states {:?}\n    events {:?}",
            def.nodes[node as usize].name, g.s.enemies[i].kind, rig.is_some(), g.s.enemies[i].alive, states, evs
        );
    }
}
