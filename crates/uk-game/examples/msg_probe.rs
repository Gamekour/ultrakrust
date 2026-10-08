//! HudMessage -> HudMessageReceiver numerically: binding strings of the scene's hints, then a
//! hint trigger entered on 0-1 (the revolver's piercing-shot hint): ShowText's letter count per
//! frame, the MessageHud Image / Text enables, the TMP draw, and the hint's Done.
//! cargo run --release -p uk-game --example msg_probe -- [level0-1] [hint node path suffix]
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef, ui};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::scripts::Script;
use uk_game::ugui::{self, UiInput};
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

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let level = a.get(1).cloned().unwrap_or("level0-1".into());
    let hint = a.get(2).cloned().unwrap_or("5 - Glass Intro/5 Nonstuff/Cube (2)".into());
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let prefs = uk_assets::prefs::Prefs::load(&install);
    println!("Binds.json scheme {:?} modified {:?}", prefs.binds.control_scheme, prefs.binds.actions.iter().map(|(n, b)| (n, b.iter().map(|x| x.path.clone()).collect::<Vec<_>>())).collect::<Vec<_>>());
    let save = uk_assets::save::Save::load(&install, prefs.int("selectedSaveSlot"));
    let mut g = Game::with_prefs(def.clone(), prefs, save);
    g.set_ui(ui::load_ui_assets(&mut db, &def));
    let assets = g.ui_assets.clone().unwrap();
    for (sc, s) in g.s.scripts.iter().enumerate() {
        if let Script::HudMessage(h) = s {
            if let Some(act) = &h.action {
                let name = g.input_actions.as_ref().and_then(|a| a.find(act).map(|i| format!("{}/{}", a.actions[i].map, a.actions[i].name)));
                let b = g.input_actions.as_ref().map(|a| a.binding_string(act)).unwrap_or_default();
                println!("hint {} action {name:?} -> {b:?}", def.path(def.scripts[sc].node));
            }
        }
    }
    let m = &g.s.msg;
    println!("receiver img={:?} text={:?} hoe={:?}", m.img, m.text, m.hoe);
    let mut t = 0.0;
    step(&mut g, 120, &mut t);
    let node = def.scripts.iter().find(|s| s.class == "HudMessage" && def.path(s.node).ends_with(&hint)).expect("hint node").node;
    let col = def.colliders.iter().find(|c| c.node == node).expect("hint collider");
    let center = match &col.shape {
        uk_assets::scenedef::ShapeDef::Box { center, .. } => *center,
        uk_assets::scenedef::ShapeDef::Sphere { center, .. } => *center,
        _ => def.nodes[node as usize].world0.w_axis.truncate(),
    };
    // the hint's room may still be inactive: switch its ancestors on
    let mut n = Some(node);
    while let Some(x) = n {
        if !g.s.active_self[x as usize] {
            g.set_active(x, true);
        }
        n = def.nodes[x as usize].parent;
    }
    println!("visible before: {}", g.msg_visible());
    g.s.player.pos = center - Vec3::Y * 0.5;
    g.s.player.prev_pos = g.s.player.pos;
    g.s.player.vel = Vec3::ZERO;
    let ui = g.ui.clone().unwrap();
    let tmp_graphic = g.s.msg.text.and_then(|s| ui.script_graphic.get(&s).copied());
    let img_graphic = g.s.msg.img.and_then(|s| ui.script_graphic.get(&s).copied());
    for f in 0..40 {
        step(&mut g, 1, &mut t);
        let m = &g.s.msg;
        let shown = m.shown_text.chars().count();
        if f == 5 {
            let fr = ugui::build_frame(&UiInput { def: &g.def, ui: &ui, assets: &assets, state: &g.s.ui, active: &g.s.active, script_enabled: &g.s.script_enabled, screen: [1920.0, 1080.0], dpi: 96.0, world_of: Some(&|n| g.node_world(n)) });
            let quads: Vec<String> = fr.batches.iter().flat_map(|b| &b.draws).filter(|d| Some(d.graphic) == tmp_graphic).map(|d| format!("{} quads, last x [{:.1},{:.1}]", d.verts.len() / 4, d.verts[d.verts.len() - 4].pos.x, d.verts[d.verts.len() - 2].pos.x)).collect();
            println!("typing draw (text {:?}): {quads:?}", g.s.msg.shown_text);
        }
        if f < 12 || f % 5 == 0 {
            println!(
                "t={t:.4} frame {f}: visible={} chars={shown}/{} img_en={:?} ui_text={:?}",
                g.msg_visible(),
                m.full.chars().count(),
                m.img.map(|i| g.s.script_enabled[i as usize]),
                tmp_graphic.and_then(|gi| g.s.ui.text[gi as usize].as_ref().map(|s| s.chars().count()))
            );
        }
    }
    println!("full: {:?}", g.s.msg.full);
    println!("shown: {:?}", g.s.msg.shown_text);
    let f = ugui::build_frame(&UiInput { def: &g.def, ui: &ui, assets: &assets, state: &g.s.ui, active: &g.s.active, script_enabled: &g.s.script_enabled, screen: [1920.0, 1080.0], dpi: 96.0, world_of: Some(&|n| g.node_world(n)) });
    for d in f.batches.iter().flat_map(|b| &b.draws).filter(|d| Some(d.graphic) == tmp_graphic || Some(d.graphic) == img_graphic) {
        let (mn, mx) = d.verts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), v| (a.min(v.pos), b.max(v.pos)));
        println!("draw {} graphic {} v{} [{:.1},{:.1}]-[{:.1},{:.1}] col {:?}", def.path(d.node), d.graphic, d.verts.len(), mn.x, mn.y, mx.x, mx.y, d.verts.first().map(|v| v.color));
    }
    // leave the trigger; the hint is not timed and has no exit/disable rule: Begone after 1 s
    g.s.player.pos += Vec3::Y * 50.0;
    g.s.player.prev_pos = g.s.player.pos;
    step(&mut g, (1.2 / FIXED_DT) as usize, &mut t);
    let Script::HudMessage(h) = &g.s.scripts[def.scripts.iter().position(|s| s.node == node && s.class == "HudMessage").unwrap()] else { unreachable!() };
    println!("after 1.2 s: visible={} hint activated={} destroyed={}", g.msg_visible(), h.activated, h.destroyed);
    g.send_hud_message("timed <color=orange>TEST</color>");
    step(&mut g, 1, &mut t);
    println!("SendHudMessage (automatic timer): visible={} full={:?}", g.msg_visible(), g.s.msg.full);
    let sent = t - FIXED_DT as f64;
    while t - sent < 4.99 {
        step(&mut g, 1, &mut t);
    }
    println!("{:.3} s after sending: visible={}", t - sent, g.msg_visible());
    while t - sent < 5.02 {
        step(&mut g, 1, &mut t);
    }
    println!("{:.3} s after sending: visible={} shown={:?}", t - sent, g.msg_visible(), g.s.msg.shown_text);
}
