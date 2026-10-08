//! Numeric time series of the HUD scripts (hud.rs) through 0-1's start: player activation,
//! PlayerActivatorRelay, HudOpenEffect, HealthBar, StaminaMeter, damage, revolver pickup, death.
//! cargo run --release -p uk-game --example hud_probe -- [level0-1] [KEY=V,...prefs overrides]
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef, ui};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::hud::HudScript;
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

fn find(g: &Game, end: &str) -> Option<u32> {
    (0..g.def.nodes.len() as u32).find(|&n| g.def.path(n).ends_with(end))
}

fn report(g: &Game, t: f64) {
    let ui = g.ui.as_ref().unwrap();
    let mut line = format!("t={t:.3} hp={} boost={:.2} secs={:.3} timer={}", g.s.hp, g.s.player.boost_charge, g.s.stats.seconds, g.s.stats.timer);
    for (sc, s) in g.s.scripts.iter().enumerate() {
        let Script::Hud(h) = s else { continue };
        let node = g.def.scripts[sc].node;
        let live = g.active(node) && g.s.script_enabled[sc];
        let name = &g.def.nodes[node as usize].name;
        match &**h {
            HudScript::OpenEffect(o) if live => {
                let sc3 = g.s.ui.scale.get(&node).copied().unwrap_or(g.def.nodes[node as usize].local_scale);
                line += &format!(" | OE {name} [{:.4},{:.4}] anim={}", sc3.x, sc3.y, o.animating);
            }
            HudScript::HealthBar(hb) if live => {
                let sl: Vec<String> = hb.hp_sliders.iter().chain(&hb.after_image).filter_map(|s| ui.script_slider.get(s)).map(|&i| format!("{:.3}", g.s.ui.slider[i as usize])).collect();
                let txt = hb.text.and_then(|t| ui.script_graphic.get(&t)).map(|&gi| (g.s.ui.text[gi as usize].clone(), g.s.ui.color[gi as usize]));
                line += &format!(" | HB {name} hp={:.3} sliders={sl:?} text={txt:?}", hb.hp);
            }
            HudScript::Stamina(st) if live => {
                let v = st.slider.and_then(|s| ui.script_slider.get(&s)).map(|&i| g.s.ui.slider[i as usize]);
                let txt = st.text.map(|gi| (g.s.ui.text[gi as usize].clone(), g.s.ui.color[gi as usize]));
                let bar = st.bar.map(|b| g.s.ui.color[b as usize]);
                let fl = st.flash.map(|b| g.s.ui.color[b as usize][3]);
                line += &format!(" | ST {name} s={:.3} v={v:?} full={} bar={bar:?} flash={fl:?} text={txt:?}", st.stamina, st.full);
            }
            _ => {}
        }
    }
    println!("{line}");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let level = a.get(1).cloned().unwrap_or("level0-1".into());
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let mut prefs = uk_assets::prefs::Prefs::load(&install);
    for kv in a.get(2).map(String::as_str).unwrap_or("").split(',').filter(|s| !s.is_empty()) {
        if let Some((k, v)) = kv.split_once('=') {
            prefs.set(k, v.parse::<f64>().unwrap());
        }
    }
    let save = uk_assets::save::Save::load(&install, prefs.int("selectedSaveSlot"));
    let mut g = Game::with_prefs(def.clone(), prefs, save);
    g.set_ui(ui::load_ui_assets(&mut db, &def));
    let assets = g.ui_assets.clone().unwrap();
    println!("colors hud {:?}", g.colors.hud);
    println!("sway {:?}", g.sway);
    let relay = g.def.scripts.iter().find(|s| s.class == "PlayerActivatorRelay").map(|s| s.data.get("toActivate").array().iter().filter_map(|p| def.node_ref(p)).collect::<Vec<u32>>()).unwrap_or_default();
    let show_relay = |g: &Game| relay.iter().map(|&n| format!("{}={}", g.def.nodes[n as usize].name, g.active(n) as u8)).collect::<Vec<_>>().join(" ");
    println!("relay at load: {}", show_relay(&g));
    for n in ["Player/Canvas", "GunCanvas", "StyleCanvas"] {
        if let Some(x) = find(&g, n) {
            let c = g.ui.as_ref().unwrap().canvases.iter().position(|c| c.node == x);
            println!("{n}: active={} canvas_enabled={:?} local={:?} z={:?}", g.active(x), c.map(|c| g.s.ui.canvas_enabled[c]), g.s.ui.local.get(&x), g.s.ui.z.get(&x));
        }
    }
    let mut t = 0.0;
    let mut was = false;
    let mut since: Option<f64> = None;
    for s in &g.s.scripts {
        if let Script::OnLevelStart { on_start, .. } = s {
            println!("OnLevelStart activates {:?} calls {:?}", on_start.to_activate.iter().map(|&n| (g.def.path(n), g.active(n))).collect::<Vec<_>>(), on_start.on_activate.iter().map(|c| (format!("{:?}", c.target), c.method.clone())).collect::<Vec<_>>());
        }
    }
    let mut started = false;
    for _ in 0..5000 {
        step(&mut g, 1, &mut t);
        if g.s.stats.level_started && !started {
            started = true;
            println!("level started at t={t:.3}: timer={} prev_secrets={:?}", g.s.stats.timer, g.s.stats.prev_secrets);
        }
        if g.s.player.activated && !was {
            was = true;
            since = Some(t);
            println!("activated at t={t:.3}: {}", show_relay(&g));
        }
        if let Some(s0) = since {
            let k = ((t - s0) / FIXED_DT as f64).round() as i64;
            if std::env::var_os("HUD_TRACE").is_some() && k < 60 {
                let r = g.s.scripts.iter().find_map(|s| match s { Script::Hud(h) => match &**h { HudScript::Relay(r) => Some(r.index), _ => None }, _ => None });
                println!("  k={k} relay_index={r:?} {} invokes={:?}", show_relay(&g), g.s.invokes.iter().map(|i| (i.act, i.at)).collect::<Vec<_>>());
            }
            if k % 25 == 0 && k <= 500 {
                report(&g, t - s0);
            }
            if k % 25 == 0 && k <= 250 {
                println!("  relay {}", show_relay(&g));
            }
            if k > 500 {
                break;
            }
        }
    }
    // LevelStatsEnabler: rank data, PlayerPrefs, Tab hold / double tap
    println!("levelNumber={} rank={:?} LevStaOpe={} LevStaTut={}", g.level_number(), g.save.rank(g.level_number()).map(|r| r.get("levelNumber").i64()), g.save.pp_int("LevStaOpe", 0), g.save.pp_int("LevStaTut", 0));
    let lse: Vec<(u32, Option<u32>)> = g.s.scripts.iter().enumerate().filter_map(|(sc, s)| match s { Script::Hud(h) => match &**h { HudScript::LevelStatsEnabler(e) => Some((g.def.scripts[sc].node, e.level_stats)), _ => None }, _ => None }).collect();
    let show_ls = |g: &Game| lse.iter().map(|&(n, c)| format!("{}={} child={:?}", g.def.path(n), g.active(n) as u8, c.map(|c| g.active(c) as u8))).collect::<Vec<_>>().join(" ");
    println!("LSE: {}", show_ls(&g));
    let tab = |g: &mut Game, t: &mut f64, p: bool, c: bool, label: &str| {
        g.hud_input = uk_game::hud::HudInput { stats_performed: p, stats_canceled: c };
        step(g, 1, t);
        g.hud_input = Default::default();
        println!("  {label}: {} LevStaOpe={}", show_ls(g), g.save.pp_int("LevStaOpe", 0));
    };
    tab(&mut g, &mut t, true, false, "tab down");
    step(&mut g, 30, &mut t);
    tab(&mut g, &mut t, false, true, "tab up (0.5s)");
    step(&mut g, 40, &mut t);
    tab(&mut g, &mut t, true, false, "tab down (after doubleTap expiry)");
    tab(&mut g, &mut t, false, true, "tab up");
    tab(&mut g, &mut t, true, false, "tab down again (double tap)");
    tab(&mut g, &mut t, false, true, "tab up (kept open)");
    step(&mut g, 50, &mut t);
    {
        let ui = g.ui.as_ref().unwrap();
        for (sc, s) in g.s.scripts.iter().enumerate() {
            let Script::Hud(h) = s else { continue };
            let HudScript::LevelStats(l) = &**h else { continue };
            let txt = |r: Option<u32>| r.and_then(|r| ui.script_graphic.get(&r)).map(|&gi| g.s.ui.text[gi as usize].clone());
            println!("LevelStats {} live={} ready={} name={:?} time={:?} timeRank={:?} kills={:?} killsRank={:?} style={:?} styleRank={:?} challenge={:?} major={:?}", g.def.path(g.def.scripts[sc].node), g.active(g.def.scripts[sc].node), l.ready, txt(l.level_name), txt(l.time), txt(l.time_rank), txt(l.kills), txt(l.kills_rank), txt(l.style), txt(l.style_rank), txt(l.challenge), txt(l.major_assists));
            let sp: Vec<_> = l.secrets.iter().map(|r| r.and_then(|r| ui.script_graphic.get(&r)).map(|&gi| (g.s.ui.sprite[gi as usize], g.active(g.def.scripts[r.unwrap() as usize].node)))).collect();
            println!("  secrets (sprite, active) {sp:?} filled={:?} prev={:?} secs={:.3}", assets.sprite(sc as u32, "filledSecret"), g.s.stats.prev_secrets, g.s.stats.seconds);
        }
        let f = ugui::build_frame(&UiInput { def: &g.def, ui, assets: &assets, state: &g.s.ui, active: &g.s.active, script_enabled: &g.s.script_enabled, screen: [1920.0, 1080.0], dpi: 96.0, world_of: Some(&|n| g.node_world(n)) });
        for d in f.batches.iter().flat_map(|b| &b.draws).filter(|d| def.path(d.node).contains("Level Stats (1)")) {
            let (mn, mx) = d.verts.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(a, b), v| (a.min(v.pos), b.max(v.pos)));
            println!("  draw {} v{} [{:.1},{:.1}]-[{:.1},{:.1}]", def.path(d.node).rsplit_once("Level Stats (1)/").map_or("", |x| x.1), d.verts.len(), mn.x, mn.y, mx.x, mx.y);
        }
    }
    tab(&mut g, &mut t, true, false, "tab down (close)");
    tab(&mut g, &mut t, false, true, "tab up");
    // uGUI draws of the HUD
    let frame = |g: &Game| {
        ugui::build_frame(&UiInput { def: &g.def, ui: g.ui.as_ref().unwrap(), assets: &assets, state: &g.s.ui, active: &g.s.active, script_enabled: &g.s.script_enabled, screen: [1920.0, 1080.0], dpi: 96.0, world_of: Some(&|n| g.node_world(n)) })
    };
    let f = frame(&g);
    for b in &f.batches {
        let path = def.path(g.ui.as_ref().unwrap().canvases[b.canvas as usize].node);
        if !path.contains("HUD") && !path.contains("Player/Canvas") {
            continue;
        }
        println!("BATCH {path} draws {}", b.draws.len());
        for d in &b.draws {
            let gr = &g.ui.as_ref().unwrap().graphics[d.graphic as usize];
            let mut mn = Vec3::splat(f32::MAX);
            let mut mx = Vec3::splat(f32::MIN);
            for v in &d.verts {
                mn = mn.min(v.pos);
                mx = mx.max(v.pos);
            }
            println!("  {} {} v{} col{:?} [{:.3},{:.3}]-[{:.3},{:.3}]", def.path(d.node).rsplit_once("HUD/").map_or(def.path(d.node).as_str(), |x| x.1), def.scripts[gr.script as usize].class, d.verts.len(), d.verts.first().map(|v| v.color), mn.x, mn.y, mx.x, mx.y);
        }
    }
    // damage
    g.hurt_player(75, false);
    println!("hurt 75");
    for i in 0..=20 {
        if i % 5 == 0 {
            report(&g, t);
        }
        step(&mut g, 5, &mut t);
    }
    g.heal_player(60);
    println!("heal 60");
    for i in 0..=40 {
        if i % 5 == 0 {
            report(&g, t);
        }
        step(&mut g, 5, &mut t);
    }
    // dash: stamina drain and refill
    g.s.player.boost_charge -= 100.0;
    println!("boost -100");
    for i in 0..=60 {
        if i % 6 == 0 {
            report(&g, t);
        }
        step(&mut g, 5, &mut t);
    }
    // revolver
    if let Some(p) = find(&g, "3 - Gun Room/RevolverPickUp") {
        g.s.player.pos = g.def.nodes[p as usize].world0.w_axis.truncate() + Vec3::Y * 0.5;
        g.s.player.prev_pos = g.s.player.pos;
        step(&mut g, 30, &mut t);
        println!("revolver={} gunpanel: {}", g.s.has_revolver, ["GunPanel"].iter().filter_map(|n| find(&g, &format!("GunCanvas/{n}"))).map(|n| g.active(n)).map(|a| a.to_string()).collect::<Vec<_>>().join(","));
        // 0-1's level start: the title (TitleActivator / Title Sound) activates the Gun Room trigger's FinalDoorOpener
        let t0 = t;
        for _ in 0..1500 {
            step(&mut g, 1, &mut t);
            if g.s.stats.level_started {
                break;
            }
        }
        println!("level started {:.3}s after the pickup: timer={} secs={:.3}", t - t0, g.s.stats.timer, g.s.stats.seconds);
        step(&mut g, 100, &mut t);
        println!("secs after 100 more frames: {:.4}", g.s.stats.seconds);
    }
    // sway with velocity
    g.s.player.vel = Vec3::new(0.0, 0.0, 30.0);
    let sw = g.sway.unwrap();
    for _ in 0..3 {
        step(&mut g, 1, &mut t);
    }
    println!("sway after 3 frames at vel 30: hud {:?} cam {:?}", g.s.ui.local.get(&sw.screen_hud), g.s.ui.local.get(&sw.hud_cam));
    g.hurt_player(999, false);
    println!("dead: screenHud active={}", g.active(sw.screen_hud));
    step(&mut g, 10, &mut t);
    g.respawn();
    println!("respawn: screenHud active={}", g.active(sw.screen_hud));
    step(&mut g, 1, &mut t);
    report(&g, t);
}
