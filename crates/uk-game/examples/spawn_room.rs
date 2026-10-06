//! Is the room around the spawn drawn at level start? Renderers within R of the player after the
//! start-up ticks: how many are active, and the inactive ancestors hiding the rest.
//! spawn_room [level, default 0-1] [radius, default 40] [ticks walking forward, default 0]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::Game;

fn main() {
    let q = std::env::args().nth(1).unwrap_or("0-1".into());
    let rad: f32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(40.0);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_level{q}.bundle"));
    let mut g = Game::new(Arc::new(scenedef::load_scene(&mut db, &path).unwrap()));
    let walk: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    for i in 0..250 + walk {
        let input = uk_core::player::Input { move_axis: bevy_math::Vec2::new(0.0, if i >= 250 { 1.0 } else { 0.0 }), ..Default::default() };
        g.fixed_update(&input);
        g.update(&input, 0.02, i as f64 * 0.02);
    }
    println!("player activated {} at {:?}", g.s.player.activated, g.s.player.pos);
    for s in g.def.scripts.iter().filter(|s| s.class == "OnLevelStart") {
        let ts = uk_game::scripts::nodes(&g.def, s.data.get("onStart").get("toActivateObjects"));
        let on = ts.iter().filter(|&&n| g.active(n)).count();
        println!("OnLevelStart targets active {on}/{}", ts.len());
    }
    let p = g.s.player.pos;
    let def = g.def.clone();
    let (mut near, mut on, mut best) = (0, 0, f32::MAX);
    let mut hidden_by: std::collections::BTreeMap<String, usize> = Default::default();
    for r in &def.renderers {
        if r.batch.positions.is_empty() {
            continue;
        }
        let c = r.batch.positions.iter().fold(bevy_math::Vec3::ZERO, |a, v| a + bevy_math::Vec3::from(*v)) / r.batch.positions.len() as f32;
        best = best.min(c.distance(p));
        if c.distance(p) > rad || def.nodes[r.node as usize].layer == scenedef::VIEWMODEL_LAYER {
            continue;
        }
        near += 1;
        if g.active(r.node) {
            on += 1;
            continue;
        }
        // topmost ancestor switched off
        let (mut x, mut top) = (Some(r.node), r.node);
        while let Some(n) = x {
            if !g.s.active_self[n as usize] {
                top = n;
            }
            x = def.nodes[n as usize].parent;
        }
        *hidden_by.entry(def.path(top)).or_default() += 1;
    }
    println!("{q}: player {p:.1}  renderers within {rad}: {near}, active {on}; nearest {best:.1} of {}", def.renderers.len());
    for (k, v) in &hidden_by {
        println!("  hidden {v:4} by {k}");
    }
}
