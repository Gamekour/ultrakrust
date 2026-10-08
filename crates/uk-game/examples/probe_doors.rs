//! Every DoorController in a level (default level0-1): stand in its trigger, check its door opens; leave, check it closes.
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef::{self, ShapeDef}};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::Script;
use uk_game::Game;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
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
        // BigDoorController doors: every leaf at its target rotation (Unity space, to_bevy_quat is an involution)
        let leaves = |g: &Game, open: bool| match &g.s.scripts[d as usize] {
            Script::Door(x) => x.bdoors.iter().all(|&b| match &g.s.scripts[b as usize] {
                Script::BigDoor(bd) => {
                    let cur = uk_assets::scene::to_bevy_quat(g.local_rot_of(def.scripts[b as usize].node));
                    cur.dot(if open { bd.target_open } else { bd.orig_rot }).abs() > 0.99999
                }
                _ => false,
            }),
            _ => false,
        };
        let (open, locked, at_open, big) = match &g.s.scripts[d as usize] {
            Script::Door(x) if x.door_type == 1 => (x.open, x.locked, leaves(&g, true), x.bdoors.len()),
            Script::Door(x) => (x.open, x.locked, g.s.local_pos[dn as usize] == x.open_pos, 0),
            _ => (false, false, false, 0),
        };
        if big > 0 {
            let moved = (0..g.movers.len() as u32).filter(|&m| g.mover_delta(m) != bevy_math::Affine3A::IDENTITY).count();
            println!("{:60} BigDoorController: {big} leaves, open={open} at target={at_open}, movers displaced {moved}", def.path(node));
        }
        // leave
        g.s.player.pos = center + bevy_math::Vec3::Y * 500.0; g.s.player.prev_pos = g.s.player.pos;
        step(&mut g, 300, &mut t);
        let closed_again = match &g.s.scripts[d as usize] {
            Script::Door(x) if x.door_type == 1 => leaves(&g, false),
            Script::Door(x) => g.s.local_pos[dn as usize] == x.closed_pos,
            _ => false,
        };
        let good = (open && at_open) || locked;
        if good { ok += 1 } else { bad += 1 }
        if !good || !closed_again {
            println!("{:60} open={open} reached_open={at_open} locked={locked} closed_after_leaving={closed_again}", def.path(node));
        }
    }
    println!("door controllers OK: {ok}, problems: {bad}");
}
