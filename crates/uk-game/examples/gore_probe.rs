//! Hit effects: the BloodsplatterManager pools, then revolver shots / punches into a zombie and the
//! Malicious Face with per-tick counts of live effects (by prefab), particles, trails, heals,
//! stains and pool sizes.
//! cargo run --release -p uk-game --example gore_probe -- [level0-1] [ticks per hit]
use bevy_math::Vec3;
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::enemy::Kind;
use uk_game::Game;

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let ticks: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(150);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let mut g = Game::new(def.clone());
    println!("bsm {:?} gore_on {}", g.fx.bsm, g.fx.gore_on);
    for (bs, p) in g.fx.pool_prefab.iter().enumerate() {
        let Some(p) = p else { continue };
        let pf = &def.particle_prefabs[*p as usize];
        let systems = pf.nodes.iter().filter(|n| n.system.is_some()).count();
        println!("  pool {} {:?}: {} queued at {:.2?}, {} nodes, {systems} systems", uk_game::particles::POOLS[bs], pf.name, g.fx.pools[bs].len(), g.fx.pools[bs].front(), pf.nodes.len());
    }
    let mut t = 0.0;
    let targets: Vec<usize> = [Kind::Filth, Kind::MaliciousFace].iter().filter_map(|k| (0..g.s.enemies.len()).find(|&e| g.s.enemies[e].kind == *k)).collect();
    for e in targets {
        // switch the enemy's chain on
        let mut chain = vec![g.s.enemies[e].node];
        while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent {
            chain.push(p);
        }
        for &m in chain.iter().rev() {
            g.set_active(m, true);
        }
        g.s.player.activated = true;
        g.s.enemies[e].spawn_t = 0.0;
        let en = &g.s.enemies[e];
        println!("\n=== {:?} #{e} {} health {} thick_limbs {}", en.kind, def.path(en.node), en.health, en.thick_limbs);
        let shots = if en.kind == Kind::MaliciousFace { 30 } else { 4 };
        for shot in 0..shots {
            let en = &g.s.enemies[e];
            if !en.hittable() {
                break;
            }
            // stand 4 m in front of it, a bit hurt so heals show
            let c = en.center();
            g.s.player.pos = c + Vec3::new(0.0, -0.5, 4.0);
            g.s.player.prev_pos = g.s.player.pos;
            g.s.player.vel = Vec3::ZERO;
            g.s.hp = 50;
            let eye = g.s.player.pos + Vec3::Y * 1.0;
            let n0 = g.fx.effects.len();
            let (h0, s0) = (g.fx.heals.len(), g.fx.stains);
            if shot % 4 == 3 {
                g.punch(eye, (c - eye).normalize());
            } else {
                g.fire_revolver(eye, (c - eye).normalize(), shot % 4 == 2);
            }
            let en = &g.s.enemies[e];
            let spawned: Vec<String> = g.fx.effects[n0..].iter().map(|f| format!("{} at {:.2?} x{:.2} hp {:?} ready {:?}", def.particle_prefabs[f.prefab as usize].name, f.pos, f.scale.x, f.splatter.as_ref().map(|s| s.hp), f.splatter.as_ref().map(|s| s.ready))).collect();
            println!("shot {shot} ({}) -> health {:.2} alive {} | spawned {spawned:#?}", if shot % 4 == 3 { "punch" } else if shot % 4 == 2 { "pierce" } else { "revolver" }, en.health, en.alive);
            for i in 0..ticks {
                g.fixed_update(&Input::default());
                t += FIXED_DT as f64;
                g.update(&Input::default(), FIXED_DT, t);
                if (i % 10 == 0 && std::env::var_os("QUIET").is_none()) || i == ticks - 1 {
                    let mut by: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
                    for f in g.fx.effects.iter().filter(|f| !f.destroyed) {
                        let r = by.entry(def.particle_prefabs[f.prefab as usize].name.as_str()).or_default();
                        r.0 += 1;
                        r.1 += f.systems.iter().map(|s| s.particles.len()).sum::<usize>();
                        r.2 += f.systems.iter().flat_map(|s| &s.particles).filter_map(|p| p.trail.as_ref()).map(|tr| tr.points.len()).sum::<usize>();
                    }
                    let pools: Vec<usize> = g.fx.pools.iter().map(|q| q.len()).collect();
                    println!("  t+{:.2}s live (effects, particles, trail pts) {by:?} heals {:?} stains +{} hp {} pools {pools:?}", (i + 1) as f32 * FIXED_DT, &g.fx.heals[h0..], g.fx.stains - s0, g.s.hp);
                }
            }
        }
    }
    // what is still alive at the end
    println!("
live at the end (t {t:.2}):");
    for f in g.fx.effects.iter().filter(|f| !f.destroyed) {
        let sys: Vec<String> = f.systems.iter().map(|s| format!("[{} playing {} t {:.2} n {} max age {:.2} y {:.1?}]", f.world.len(), s.playing, s.time, s.particles.len(), s.particles.iter().map(|p| p.age).fold(0.0, f32::max), s.particles.iter().map(|p| p.pos.y).fold(f32::MAX, f32::min))).collect();
        println!("  {} at {:.2?} active {} remove_at {:?} disable_at {:?} col {:?} {}", def.particle_prefabs[f.prefab as usize].name, f.pos, f.active, f.remove_at, f.splatter.as_ref().map(|s| s.disable_at), f.splatter.as_ref().map(|s| (s.col_enabled, s.can_collide)), sys.join(" "));
    }
}
