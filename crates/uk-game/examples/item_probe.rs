//! Skulls and altars (ItemIdentifier / ItemPlaceZone / Skull / Punch.holder) of a level: the
//! state after Start, then a pickup / punch / placement / pickup time series.
//! cargo run --release -p uk-game --example item_probe -- [level0-2]
use bevy_math::{Mat4, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::Script;
use uk_game::Game;

fn step(g: &mut Game, frames: usize, t: &mut f64) {
    let input = Input::default();
    for _ in 0..frames {
        g.fixed_update(&input);
        *t += FIXED_DT as f64;
        g.update(&input, FIXED_DT, *t);
        g.s.player.events.clear();
    }
}

fn bevy_pos(g: &Game, n: u32) -> Vec3 {
    let w = g.node_world_now(n).w_axis;
    Vec3::new(w.x, w.y, -w.z)
}

fn report(g: &Game, tag: &str) {
    let def = &g.def;
    println!("-- {tag}: held={:?}", g.held_item().map(|n| def.path(n)));
    for (sc, s) in g.s.scripts.iter().enumerate() {
        let node = def.scripts[sc].node;
        match s {
            Script::Item(it) => {
                let cols: Vec<bool> = def.colliders.iter().enumerate().filter(|(_, c)| c.node == node).map(|(i, _)| g.s.collider_enabled[i]).collect();
                println!(
                    "  item {} type={} active={} parent={} pos={:.3?} picked={} ipz={:?} colliders={cols:?}",
                    def.path(node),
                    it.item_type,
                    g.active(node),
                    g.eff_parent(node).map_or("-".into(), |p| def.path(p)),
                    bevy_pos(g, node),
                    it.picked_up,
                    it.ipz.map(|z| def.path(def.scripts[z as usize].node)),
                )
            }
            Script::PlaceZone(z) => {
                let doors: Vec<String> = z
                    .doors
                    .iter()
                    .map(|&d| match &g.s.scripts[d as usize] {
                        Script::Door(dd) => format!("{} open={} requests={} local={:.2?} target={:.2?}", def.path(def.scripts[d as usize].node), dd.open, dd.requests, g.s.local_pos[def.scripts[d as usize].node as usize], dd.target_pos),
                        _ => "?".into(),
                    })
                    .collect();
                let elems: Vec<bool> = z.altar_elements.iter().map(|&n| g.active(n)).collect();
                println!(
                    "  zone {} accepts={} active={} placed={} col={:?} elements={elems:?} doors={doors:?}",
                    def.path(node),
                    z.accepted,
                    g.active(node),
                    z.accepted_placed,
                    z.col.map(|c| g.s.collider_enabled[c as usize]),
                )
            }
            Script::Skull(k) => {
                let l = k.light.map(|l| g.s.lights[l as usize]);
                let lw = k.light.map(|l| g.light_world_bevy(def.lights[l as usize].node).w_axis.truncate());
                println!(
                    "  skull {} light(enabled,range)={l:?} orig={} lit_time={:.2} light_pos={lw:.3?} modmat props={:?}",
                    def.path(node),
                    k.orig_range,
                    k.lit_time,
                    k.modmat.map(|m| g.mat_props_of(m))
                )
            }
            _ => {}
        }
    }
    if let Some(r) = g.vm_rigs.1 {
        println!("  punch anim Holding={:?} SemiHolding={:?}", uk_game::anim::get_param(g, r, "Holding"), uk_game::anim::get_param(g, r, "SemiHolding"));
    }
}

/// Punch from 2 m away along `dir` at the node's current world position.
fn punch_at(g: &mut Game, n: u32, dir: Vec3) {
    let p = bevy_pos(g, n);
    let eye = p - dir * 2.0;
    g.s.player.pos = eye - Vec3::Y * 1.0;
    g.view_world = Mat4::from_translation(eye);
    println!(">> punch at {} from {eye:.2?}", g.def.path(n));
    g.punch(eye, dir);
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-2".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    for s in def.scripts.iter().filter(|s| s.class == "Punch") {
        let h = def.node_ref(s.data.get("holder"));
        println!("Punch {} type={} holder={:?}", def.path(s.node), s.data.get("type").i64(), h.map(|h| (def.path(h), def.nodes[h as usize].layer, def.nodes[h as usize].world0.w_axis)));
    }
    if let Some(sw) = Game::new(def.clone()).sway {
        let mc = def.nodes[sw.hud_cam as usize].parent.map(|p| def.path(p));
        println!("HUD Camera {} parent {mc:?} local {:?}", def.path(sw.hud_cam), def.nodes[sw.hud_cam as usize].local_pos);
    }
    let mut g = Game::new(def.clone());
    let mut t = 0.0;
    // activate every zone's and item's ancestors, like the player walking in
    let mut want: Vec<u32> = g.s.scripts.iter().enumerate().filter(|(_, s)| matches!(s, Script::Item(_) | Script::PlaceZone(_))).map(|(i, _)| def.scripts[i].node).collect();
    want.sort();
    for &n in &want {
        let mut chain = vec![n];
        while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent {
            chain.push(p);
        }
        // the room root only: what is inside stays as the level leaves it
        let root = *chain.last().unwrap();
        if !g.active(root) {
            println!("activating {}", def.path(root));
            g.set_active(root, true);
        }
    }
    step(&mut g, 5, &mut t);
    report(&g, "after Start");
    let items: Vec<u32> = g.s.scripts.iter().enumerate().filter(|(_, s)| matches!(s, Script::Item(_))).map(|(i, _)| def.scripts[i].node).collect();
    let zones: Vec<(u32, i64)> = g.s.scripts.iter().enumerate().filter_map(|(i, s)| if let Script::PlaceZone(z) = s { Some((def.scripts[i].node, z.accepted)) } else { None }).collect();
    let Some(&skull) = items.first() else { return };
    let down = Vec3::new(0.0, -1.0, 0.3).normalize();
    punch_at(&mut g, skull, down);
    step(&mut g, 1, &mut t);
    report(&g, "after pickup punch");
    // punching while holding: PunchWith flares the light, which settles back at 5/s after 1 s
    g.s.player.pos += Vec3::Y * 50.0;
    let eye = g.s.player.pos + Vec3::Y;
    g.punch(eye, Vec3::Y);
    for k in 0..8 {
        let Some(Script::Skull(sk)) = g.s.scripts.iter().find(|s| matches!(s, Script::Skull(k) if k.light.is_some() && g.under_held(def.lights[k.light.unwrap() as usize].node))) else { break };
        println!("  t={:.2}s held skull light range={:.3} lit_time={:.2}", k as f32 * 0.25, g.s.lights[sk.light.unwrap() as usize].1, sk.lit_time);
        step(&mut g, (0.25 / FIXED_DT).round() as usize, &mut t);
    }
    // place it on the first empty altar that accepts it
    // (an active one whose own collider is on: no active item inside)
    let Some(&(zone, _)) = zones.iter().find(|&&(z, a)| a == 1 && g.active(z) && def.colliders.iter().enumerate().any(|(i, c)| c.node == z && g.s.collider_enabled[i])) else { return };
    punch_at(&mut g, zone, down);
    step(&mut g, 1, &mut t);
    report(&g, "after placing");
    step(&mut g, 300, &mut t);
    report(&g, "6 s later");
    punch_at(&mut g, skull, down);
    step(&mut g, 1, &mut t);
    report(&g, "after taking it back");
}
