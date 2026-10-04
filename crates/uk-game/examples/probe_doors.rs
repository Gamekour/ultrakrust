//! Every DoorController in 0-1: stand in its trigger, check its door opens; leave, check it closes.
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
    let ctrls: Vec<usize> = def.scripts.iter().enumerate().filter(|(_, s)| s.class == "DoorController").map(|(i, _)| i).collect();
    let (mut ok, mut bad) = (0, 0);
    for sc in ctrls {
        let node = def.scripts[sc].node;
        let mut g = Game::new(def.clone());
        let mut t = 0.0f64;
        let step = |g: &mut Game, n: usize, t: &mut f64| for _ in 0..n {
            g.fixed_update(&Input::default()); *t += FIXED_DT as f64; g.update(&Input::default(), FIXED_DT, *t);
            g.s.hp = 100; g.s.player.events.clear(); g.events.clear();
        };
        step(&mut g, 300, &mut t);
        let mut chain = vec![node];
        while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent { chain.push(p); }
        for n in chain.iter().rev() { g.set_active(*n, true); }
        step(&mut g, 5, &mut t);
        let Some(c) = def.colliders.iter().find(|c| c.node == node && c.trigger) else { continue };
        let ShapeDef::Box { center, .. } = c.shape else { continue };
        g.s.player.pos = center; g.s.player.prev_pos = center; g.s.player.activated = false; // stand still
        step(&mut g, 200, &mut t);
        let door = match &g.s.scripts[sc] { Script::DoorController(d) => d.door, _ => None };
        let Some(d) = door else { println!("{:60} NO DOOR FOUND", def.path(node)); bad += 1; continue };
        let dn = def.scripts[d as usize].node;
        let (open, locked, at_open) = match &g.s.scripts[d as usize] { Script::Door(x) => (x.open, x.locked, g.s.local_pos[dn as usize] == x.open_pos), _ => (false, false, false) };
        // leave
        g.s.player.pos = center + bevy_math::Vec3::Y * 500.0; g.s.player.prev_pos = g.s.player.pos;
        step(&mut g, 300, &mut t);
        let closed_again = match &g.s.scripts[d as usize] { Script::Door(x) => g.s.local_pos[dn as usize] == x.closed_pos, _ => false };
        let good = (open && at_open) || locked;
        if good { ok += 1 } else { bad += 1 }
        if !good || !closed_again {
            println!("{:60} open={open} reached_open={at_open} locked={locked} closed_after_leaving={closed_again}", def.path(node));
        }
    }
    println!("door controllers OK: {ok}, problems: {bad}");
}
