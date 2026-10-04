//! Behavioural checks on level 0-1 (ported from the `uk-game/examples/probe_*` probes) and sim performance.
use crate::Report;
use bevy_math::Vec3;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use uk_assets::{db::AssetDb, scenedef::{self, SceneDef, ShapeDef}};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::{bot::{route_0_1, Bot, BotFrame}, scripts::Script, Game, GameEvent};

/// Advance `n` ticks with no input. `god` keeps the player at full health.
fn idle(g: &mut Game, n: usize, t: &mut f64, god: bool) {
    for _ in 0..n {
        g.fixed_update(&Input::default());
        *t += FIXED_DT as f64;
        g.update(&Input::default(), FIXED_DT, *t);
        if god {
            g.s.hp = 100
        }
        g.s.player.events.clear();
    }
}

fn activate_chain(g: &mut Game, def: &SceneDef, node: u32) {
    let mut chain = vec![node];
    while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent {
        chain.push(p)
    }
    for n in chain.iter().rev() {
        g.set_active(*n, true)
    }
}

fn shape_center(def: &SceneDef, node: u32, trigger_only: bool) -> Option<Vec3> {
    def.colliders.iter().filter(|c| c.node == node && (!trigger_only || c.trigger)).find_map(|c| match c.shape {
        ShapeDef::Box { center, .. } | ShapeDef::Sphere { center, .. } => Some(center),
        _ => None,
    })
}

fn teleport(g: &mut Game, p: Vec3) {
    g.s.player.pos = p;
    g.s.player.prev_pos = p;
}

pub fn level_0_1(install: &Path, r: &mut Report) {
    let mut db = AssetDb::open(install).unwrap();
    let path = AssetDb::bundle_dir(install).join("campaign_scenes_level0-1.bundle");
    let t = Instant::now();
    let def = match scenedef::load_scene(&mut db, &path) {
        Ok(d) => Arc::new(d),
        Err(e) => {
            r.pass("0-1.load", false, e.to_string());
            return;
        }
    };
    r.lower("perf.0-1.scene_load_ms", t.elapsed().as_secs_f64() * 1e3, 0.35);
    let t = Instant::now();
    let _ = Game::new(def.clone());
    r.lower("perf.0-1.game_build_ms", t.elapsed().as_secs_f64() * 1e3, 0.35);
    r.pass("0-1.load", true, "");
    doors(&def, r);
    arenas(&def, r);
    boss_to_exit(&def, r);
    // A second, independent load: every HashMap gets a fresh random seed, so any load-order
    // dependence on hash iteration shows up as a divergence here.
    match scenedef::load_scene(&mut AssetDb::open(install).unwrap(), &path) {
        Ok(d2) => determinism(&def, &Arc::new(d2), r),
        Err(e) => r.pass("0-1.determinism", false, e.to_string()),
    }
    checkpoint(&def, r);
    bot_run(&def, r);
}

/// Hash of the simulation state that matters for outcomes (bit-exact floats).
pub fn state_hash(g: &Game) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut mix = |x: u64| {
        h ^= x;
        h = h.wrapping_mul(0x100000001b3)
    };
    let v3 = |v: Vec3| [v.x.to_bits() as u64, v.y.to_bits() as u64, v.z.to_bits() as u64];
    for x in v3(g.s.player.pos).into_iter().chain(v3(g.s.player.vel)) {
        mix(x)
    }
    mix(g.s.hp as u64);
    mix(g.s.kills as u64);
    for e in &g.s.enemies {
        for x in v3(e.pos) {
            mix(x)
        }
        mix(e.health.to_bits() as u64);
        mix(e.alive as u64);
    }
    for (i, a) in g.s.active.iter().enumerate() {
        if *a {
            mix(i as u64)
        }
    }
    h
}

/// Two bot runs from the same scene must be bit-identical tick for tick (replays, probes and
/// regression baselines all depend on it). Reports the first divergent tick.
fn determinism(def: &Arc<SceneDef>, def2: &Arc<SceneDef>, r: &mut Report) {
    let ticks = 125 * 90;
    let trace = |def: &Arc<SceneDef>| {
        let mut g = Game::new(def.clone());
        let mut bot = Bot::new(route_0_1());
        let mut t = 0.0;
        let mut out = Vec::with_capacity(ticks);
        for _ in 0..ticks {
            step_bot(&mut g, &mut bot, &mut t);
            g.events.clear();
            out.push(state_hash(&g));
        }
        out
    };
    let a = trace(def);
    let b = trace(def2);
    let first = a.iter().zip(&b).position(|(x, y)| x != y);
    r.pass("0-1.determinism", first.is_none(), format!("runs diverge at tick {:?}", first));
}

/// Every DoorController: stand in its trigger -> door opens (or is locked by an arena); leave -> it closes.
fn doors(def: &Arc<SceneDef>, r: &mut Report) {
    let (mut ok, mut bad) = (0, Vec::new());
    for sc in (0..def.scripts.len()).filter(|&i| def.scripts[i].class == "DoorController") {
        let node = def.scripts[sc].node;
        let mut g = Game::new(def.clone());
        let mut t = 0.0;
        idle(&mut g, 300, &mut t, true);
        activate_chain(&mut g, def, node);
        idle(&mut g, 5, &mut t, true);
        let Some(c) = def.colliders.iter().find(|c| c.node == node && c.trigger) else { continue };
        let ShapeDef::Box { center, .. } = c.shape else { continue };
        teleport(&mut g, center);
        g.s.player.activated = false;
        idle(&mut g, 200, &mut t, true);
        // controllers with no door are decorative in the original too
        let Some(d) = (match &g.s.scripts[sc] {
            Script::DoorController(d) => d.door,
            _ => None,
        }) else {
            continue;
        };
        let dn = def.scripts[d as usize].node;
        let (open, locked, at_open) = match &g.s.scripts[d as usize] {
            Script::Door(x) => (x.open, x.locked, g.s.local_pos[dn as usize] == x.open_pos),
            _ => (false, false, false),
        };
        teleport(&mut g, center + Vec3::Y * 500.0);
        idle(&mut g, 300, &mut t, true);
        let closed = matches!(&g.s.scripts[d as usize], Script::Door(x) if g.s.local_pos[dn as usize] == x.closed_pos);
        if ((open && at_open) || locked) && closed {
            ok += 1
        } else {
            bad.push(def.path(node))
        }
    }
    r.higher("play.0-1.doors_ok", ok as f64);
    r.pass("0-1.doors", bad.is_empty(), format!("{bad:?}"));
}

/// Every live ActivateArena: trigger it, kill every wave, the doors it locked must unlock.
fn arenas(def: &Arc<SceneDef>, r: &mut Report) {
    let (mut ok, mut bad) = (0, Vec::new());
    let mut unknown = std::collections::BTreeSet::new();
    for sc in (0..def.scripts.len()).filter(|&i| def.scripts[i].class == "ActivateArena") {
        let node = def.scripts[sc].node;
        let p = def.path(node);
        if p.contains("OLD") || p.contains("Alt") {
            continue;
        }
        let mut g = Game::new(def.clone());
        let mut t = 0.0;
        idle(&mut g, 300, &mut t, true);
        activate_chain(&mut g, def, node);
        let Some(center) = shape_center(def, node, false) else { continue };
        teleport(&mut g, center);
        idle(&mut g, 200, &mut t, true);
        let (activated, locked_doors) = match &g.s.scripts[sc] {
            Script::Arena(a) => (a.activated, a.doors.clone()),
            _ => (false, vec![]),
        };
        let mut killed = 0;
        for _ in 0..12 {
            idle(&mut g, 150, &mut t, true);
            let ids: Vec<usize> = (0..g.s.enemies.len())
                .filter(|&i| g.s.enemies[i].alive && g.active(g.s.enemies[i].node) && g.s.enemies[i].spawn_t <= 0.0)
                .collect();
            if ids.is_empty() {
                idle(&mut g, 300, &mut t, true);
                if g.s.enemies.iter().all(|e| !(e.alive && g.active(e.node))) {
                    break;
                }
            }
            killed += ids.len();
            for i in ids {
                uk_game::enemy::kill_enemy(&mut g, i)
            }
        }
        idle(&mut g, 400, &mut t, true);
        let still = locked_doors.iter().filter(|&&d| matches!(&g.s.scripts[d as usize], Script::Door(x) if x.locked)).count();
        unknown.extend(g.unknown_calls.iter().cloned());
        if activated && killed > 0 && still == 0 {
            ok += 1
        } else {
            bad.push(format!("{p} (activated={activated} killed={killed} still_locked={still})"))
        }
    }
    r.higher("play.0-1.arenas_ok", ok as f64);
    r.pass("0-1.arenas", bad.is_empty(), format!("{bad:?}"));
    r.lower("gap.0-1.unknown_event_calls", unknown.len() as f64, 0.0);
    if !unknown.is_empty() {
        r.note(format!("gap: unported UnityEvent targets hit in 0-1 arenas: {unknown:?}"))
    }
}

/// Boss arena: kill the Malicious Face with the revolver, final door opens, final pit completes the level.
fn boss_to_exit(def: &Arc<SceneDef>, r: &mut Report) {
    let find = |p: &str| (0..def.nodes.len() as u32).find(|&n| def.path(n) == p);
    let mut g = Game::new(def.clone());
    let mut t = 0.0;
    idle(&mut g, 400, &mut t, true);
    for room in ["13 - Malicious Face Arena", "12B - Pre-Boss Checkpoint"] {
        match find(room) {
            Some(n) => g.set_active(n, true),
            None => {
                r.pass("0-1.boss_to_exit", false, format!("no {room}"));
                return;
            }
        }
    }
    let Some(trig) = def
        .scripts
        .iter()
        .position(|s| s.class == "ActivateArena" && def.path(s.node) == "13 - Malicious Face Arena/13 Content/Trigger")
    else {
        r.pass("0-1.boss_to_exit", false, "no boss trigger");
        return;
    };
    teleport(&mut g, shape_center(def, def.scripts[trig].node, false).unwrap());
    idle(&mut g, 250, &mut t, true);
    let Some(mf) = g.s.enemies.iter().position(|e| e.kind == uk_game::enemy::Kind::MaliciousFace) else {
        r.pass("0-1.boss_to_exit", false, "no Malicious Face");
        return;
    };
    let mut shots = 0;
    while g.s.enemies[mf].alive && shots < 200 {
        let eye = g.s.player.pos + Vec3::Y * 1.4;
        let aim = (g.s.enemies[mf].center() - eye).normalize();
        g.fire_revolver(eye, aim, false);
        shots += 1;
        idle(&mut g, 62, &mut t, true);
        g.events.clear();
    }
    r.info("play.0-1.boss_revolver_shots", shots as f64);
    idle(&mut g, 600, &mut t, true);
    // 0-1 has an unused second pit ("Pit (2)") that stays inactive; use the live one.
    let Some(pit) = def.scripts.iter().position(|s| s.class == "FinalPit" && g.active(s.node)) else {
        r.pass("0-1.boss_to_exit", false, "no active FinalPit after the boss");
        return;
    };
    if let Some(c) = shape_center(def, def.scripts[pit].node, false) {
        teleport(&mut g, c);
        idle(&mut g, 20, &mut t, true)
    }
    let dead = !g.s.enemies[mf].alive;
    r.pass("0-1.boss_to_exit", dead && g.s.level_complete, format!("boss_dead={dead} level_complete={}", g.s.level_complete));
}

/// Bot to the first checkpoint, die: respawn at the checkpoint with the revolver and kills preserved.
fn checkpoint(def: &Arc<SceneDef>, r: &mut Report) {
    let mut g = Game::new(def.clone());
    let mut bot = Bot::new(route_0_1());
    let mut t = 0.0;
    let mut reached = false;
    for _ in 0..(125 * 120) {
        step_bot(&mut g, &mut bot, &mut t);
        if g.events.drain(..).any(|e| e == GameEvent::Checkpoint) {
            reached = true;
            break;
        }
    }
    if !reached {
        r.pass("0-1.checkpoint_respawn", false, "bot never reached a checkpoint");
        return;
    }
    let kills = g.s.kills;
    idle(&mut g, 250, &mut t, false);
    g.hurt_player(999, false);
    let died = g.s.dead;
    idle(&mut g, 250, &mut t, false);
    let ev: Vec<_> = g.events.drain(..).collect();
    let ok = died && !g.s.dead && g.s.hp == 100 && g.s.has_revolver && g.s.kills == kills && ev.contains(&GameEvent::Respawned);
    r.pass(
        "0-1.checkpoint_respawn",
        ok,
        format!("died={died} dead_after={} hp={} revolver={} kills {}->{}", g.s.dead, g.s.hp, g.s.has_revolver, kills, g.s.kills),
    );
}

fn step_bot(g: &mut Game, bot: &mut Bot, t: &mut f64) -> BotFrame {
    let f = uk_game::bot::drive(g, bot, t);
    bot.log.clear();
    f
}

/// Full autopilot run from the level start (no god mode): progress, deaths, and per-tick sim cost.
fn bot_run(def: &Arc<SceneDef>, r: &mut Report) {
    let mut g = Game::new(def.clone());
    let mut bot = Bot::new(route_0_1());
    let mut t = 0.0;
    let mut deaths = 0;
    let mut ticks_us = Vec::with_capacity(125 * 600);
    let wall = Instant::now();
    while t < 600.0 && !bot.done() && !g.s.level_complete {
        let s = Instant::now();
        step_bot(&mut g, &mut bot, &mut t);
        ticks_us.push(s.elapsed().as_secs_f64() * 1e6);
        deaths += g.events.drain(..).filter(|e| *e == GameEvent::Died).count();
    }
    ticks_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| ticks_us[((ticks_us.len() as f64 - 1.0) * p) as usize];
    r.higher("play.0-1.bot_waypoints", bot.idx as f64);
    r.info("play.0-1.bot_waypoints_total", bot.route.len() as f64);
    r.lower("play.0-1.bot_deaths", deaths as f64, 0.0);
    r.info("play.0-1.bot_sim_seconds", t);
    r.lower("perf.0-1.sim_tick_us_p50", pct(0.5), 0.30);
    r.lower("perf.0-1.sim_tick_us_p99", pct(0.99), 0.50);
    r.info("perf.0-1.sim_tick_us_max", pct(1.0));
    // 125 Hz gives 8000 us per tick; the sim must leave most of that for rendering.
    r.pass("0-1.sim_budget", pct(0.99) < 2000.0, format!("p99 tick {:.0} us exceeds 2000 us", pct(0.99)));
    r.info("perf.0-1.bot_wall_s", wall.elapsed().as_secs_f64());
    if !bot.done() {
        r.note(format!(
            "gap: autopilot stopped at waypoint {}/{} ({})",
            bot.idx,
            bot.route.len(),
            bot.route.get(bot.idx).map_or("", |w| w.label)
        ))
    }
}
