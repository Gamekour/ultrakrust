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
    let first_load_ms = t.elapsed().as_secs_f64() * 1e3;
    let t = Instant::now();
    let mut g = Game::new(def.clone());
    r.lower("perf.0-1.game_build_ms", t.elapsed().as_secs_f64() * 1e3, 0.35);
    let mut rt = crate::anim::RtTotals::default();
    crate::anim::runtime(&mut g, 240, &mut rt);
    r.higher("animrt.0-1.rigs", rt.rigs as f64);
    r.higher("animrt.0-1.moved_nodes", rt.moved_nodes as f64);
    r.higher("animrt.0-1.slots_bound_pct", 100.0 * rt.slots_bound as f64 / rt.slots.max(1) as f64);
    crate::anim::enemy_attacks(&def, r);
    r.pass("animrt.0-1.finite", rt.nonfinite == 0, format!("{} non-finite", rt.nonfinite));
    r.pass("0-1.load", true, "");
    // A second, independent load: every HashMap gets a fresh random seed, so any load-order
    // dependence on hash iteration shows up as a divergence. Load time = the faster of the two,
    // so a cold disk cache on the first one doesn't read as a regression.
    let t = Instant::now();
    let def2 = scenedef::load_scene(&mut AssetDb::open(install).unwrap(), &path);
    r.lower("perf.0-1.scene_load_ms", first_load_ms.min(t.elapsed().as_secs_f64() * 1e3), 0.35);
    navmesh(&def, r);
    doors(&def, r);
    arenas(&def, r);
    boss_to_exit(&def, r);
    match def2 {
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

/// The baked navmesh: decoded polygons lie on the level's collision geometry, string-pulled paths
/// stay on the mesh and are no longer than the portal-midpoint chain, and the Fan Room is reachable.
pub fn nav_checks(prefix: &str, g: &Game, r: &mut Report) {
    let Some(nav) = &g.nav else {
        r.pass(&format!("{prefix}.navmesh"), false, "no navmesh");
        return;
    };
    r.info(&format!("nav.{prefix}.polys"), nav.polys.len() as f64);
    r.info(&format!("nav.{prefix}.components"), nav.components() as f64);
    r.info(&format!("nav.{prefix}.external_edges_joined_pct"), 100.0 * nav.external_matched as f64 / nav.external_edges.max(1) as f64);
    // 1. on geometry: from each polygon's surface point, the environment is right below. The navmesh is
    // baked over the whole level, so test against every collider, not just the rooms active at start.
    let mut world = g.world.clone();
    world.owner_enabled.iter_mut().for_each(|e| *e = true);
    let mut on = 0;
    for (i, p) in nav.polys.iter().enumerate() {
        let s = nav.closest_on_poly(i as u32, p.center);
        if world.raycast(s + Vec3::Y * 1.0, Vec3::NEG_Y, 2.0).is_some() {
            on += 1
        }
    }
    let on_pct = 100.0 * on as f64 / nav.polys.len().max(1) as f64;
    r.higher(&format!("nav.{prefix}.polys_on_geometry_pct"), on_pct);
    // 2. path quality on deterministic pseudo-random pairs
    let mut seed = 0x9e3779b9u32;
    let mut rnd = |n: usize| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as usize % n
    };
    let (mut tried, mut good, mut reached) = (0, 0, 0);
    for _ in 0..300 {
        let (a, b) = (rnd(nav.polys.len()), rnd(nav.polys.len()));
        let (pa, pb) = (nav.closest_on_poly(a as u32, nav.polys[a].center), nav.closest_on_poly(b as u32, nav.polys[b].center));
        let (Some((_, _, portals, ok)), Some((path, _))) = (nav.corridor(pa, pb), nav.find_path(pa, pb)) else { continue };
        tried += 1;
        reached += ok as usize;
        // The smoothed path must pass through every walking portal of its corridor (in xz). Polygons are
        // convex, so that keeps it inside the corridor. (Sampling heights along a 3D line is not a valid
        // test: on stairs the line floats above the steps and "nearest" picks a neighbour.)
        let segs: Vec<(Vec3, Vec3)> = path.windows(2).filter(|w| !w[1].offmesh).map(|w| (w[0].pos, w[1].pos)).collect();
        let through = portals.iter().filter(|l| l.offmesh.is_none()).all(|l| segs.iter().any(|&(p, q)| crosses_xz(p, q, l.a, l.b)));
        good += through as usize;
    }
    r.higher(&format!("nav.{prefix}.paths_through_corridor_pct"), 100.0 * good as f64 / tried.max(1) as f64);
    r.info(&format!("nav.{prefix}.random_pairs_reached_pct"), 100.0 * reached as f64 / tried.max(1) as f64);
    // Polygons-on-geometry is tracked per level against the baseline rather than gated on an absolute
    // threshold: some shipped navmeshes are stale bakes (6-2's lies where the level has no geometry at
    // all, in the original too), so a fixed cut-off would flag the data, not our reader.
    r.pass(&format!("{prefix}.navmesh"), good == tried, format!("paths through their corridor {good}/{tried} (polys on geometry {on_pct:.1}%)"));
}

/// Segment p-q meets portal a-b in xz (touching or running along it counts).
fn crosses_xz(p: Vec3, q: Vec3, a: Vec3, b: Vec3) -> bool {
    use bevy_math::Vec2;
    let (p, q, a, b) = (Vec2::new(p.x, p.z), Vec2::new(q.x, q.z), Vec2::new(a.x, a.z), Vec2::new(b.x, b.z));
    let on = |x: Vec2| {
        let e = b - a;
        let t = if e.length_squared() > 0.0 { ((x - a).dot(e) / e.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
        (a + e * t).distance(x) < 1e-2
    };
    if on(p) || on(q) {
        return true;
    }
    let (d, e) = (q - p, b - a);
    let den = d.perp_dot(e);
    if den.abs() < 1e-9 {
        return false;
    }
    let t = (a - p).perp_dot(e) / den;
    let u = (a - p).perp_dot(d) / den;
    (-1e-3..=1.0 + 1e-3).contains(&t) && (-1e-3..=1.0 + 1e-3).contains(&u)
}

fn navmesh(def: &Arc<SceneDef>, r: &mut Report) {
    let g = Game::new(def.clone());
    nav_checks("0-1", &g, r);
    let Some(nav) = &g.nav else { return };
    // The Fan Room Filth that used to stall the autopilot must be able to come down to the walkway.
    let filth = Vec3::new(40.7, 3.26, -571.4);
    let walkway = Vec3::new(40.7, -10.4, -567.2);
    let res = nav.find_path(filth, walkway);
    r.pass("0-1.nav_fan_room", res.as_ref().is_some_and(|(_, ok)| *ok),
        format!("Fan Room ledge -> walkway: {:?}", res.map(|(p, ok)| (p.len(), ok))));
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
    // the first pit (the one the hallway drops into); TeleportFinalPit moves the fall into "Pit (2)"
    let first = |i: usize| matches!(&g.s.scripts[i], Script::FinalPit(p) if !p.second_pit);
    let Some(pit) = (0..def.scripts.len()).find(|&i| def.scripts[i].class == "FinalPit" && g.active(def.scripts[i].node) && first(i)) else {
        r.pass("0-1.boss_to_exit", false, "no active first FinalPit after the boss");
        return;
    };
    // enter off-axis, looking away, so the centering and the view turn have work to do
    let Some(c) = shape_center(def, def.scripts[pit].node, false) else {
        r.pass("0-1.boss_to_exit", false, "FinalPit has no collider");
        return;
    };
    // just inside the top of the shaft, as if dropped from the room above
    teleport(&mut g, c + Vec3::new(0.6, 55.0, -0.4));
    g.s.player.yaw_deg += 120.0;
    g.s.view_pitch = -30.0;
    idle(&mut g, 20, &mut t, true);
    let dead = !g.s.enemies[mf].alive;
    r.pass("0-1.boss_to_exit", dead && g.s.level_complete, format!("boss_dead={dead} level_complete={}", g.s.level_complete));
    final_pit(def, &mut g, &mut t, r);
}

/// FinalPit -> FinalRank: R does nothing in the elevator, the player is pulled to the pit's axis and
/// turned to its rotation, the results go up, and the fall reaches the second pit, which arms the
/// continue to the next level (`targetLevelName`).
fn final_pit(def: &Arc<SceneDef>, g: &mut Game, t: &mut f64, r: &mut Report) {
    let before = (g.s.player.pos, g.s.restarts, g.s.level_complete);
    g.respawn();
    let r_ignored = g.s.player.pos == before.0 && g.s.restarts == before.1 && g.s.level_complete;
    r.pass("0-1.exit_ignores_restart", r_ignored, format!("pos {:?} -> {:?}, restarts {} -> {}", before.0, g.s.player.pos, before.1, g.s.restarts));
    // distance to the axis of whichever shaft the player is in (the second sits 20 along the first's forward)
    let axes: Vec<Vec3> = def.scripts.iter().filter(|s| s.class == "FinalPit").map(|s| def.nodes[s.node as usize].world0.to_scale_rotation_translation().2).collect();
    let axis_dist = |g: &Game| axes.iter().map(|a| Vec3::new(g.s.player.pos.x - a.x, 0.0, g.s.player.pos.z - a.z).length()).fold(f32::MAX, f32::min);
    r.info("play.0-1.pit_axis_dist_enter", axis_dist(g) as f64);
    let (mut results_at, mut second_at, mut min_axis) = (None, None, f32::MAX);
    let mut view_err = f32::NAN;
    let target_view = {
        let n = def.scripts.iter().position(|s| s.class == "FinalPit" && g.active(s.node)).map(|i| def.scripts[i].node).unwrap();
        let (_, rot, _) = def.nodes[n as usize].world0.to_scale_rotation_translation();
        let f = rot * Vec3::NEG_Z;
        (f.x.atan2(-f.z).to_degrees(), f.y.clamp(-1.0, 1.0).asin().to_degrees())
    };
    for i in 0..(125 * 20) {
        idle(g, 1, t, true);
        min_axis = min_axis.min(axis_dist(g));
        if let Some((y, p)) = g.s.forced_view {
            let dy = ((y - target_view.0 + 540.0).rem_euclid(360.0) - 180.0).abs();
            view_err = dy.max((p - target_view.1).abs());
        }
        if results_at.is_none() && g.s.results_shown {
            results_at = Some(i as f32 * FIXED_DT);
        }
        if second_at.is_none() && g.s.reached_second_pit {
            second_at = Some(i as f32 * FIXED_DT);
            break;
        }
    }
    r.info("play.0-1.pit_axis_dist_min", min_axis as f64);
    r.info("play.0-1.pit_view_err_deg", view_err as f64);
    r.info("play.0-1.results_at_s", results_at.unwrap_or(-1.0) as f64);
    r.info("play.0-1.second_pit_at_s", second_at.unwrap_or(-1.0) as f64);
    let next = g.s.next_level.clone().unwrap_or_default();
    r.pass(
        "0-1.exit_to_next_level",
        results_at.is_some() && second_at.is_some() && next == "Level 0-2" && view_err < 0.5,
        format!("results {results_at:?} second_pit {second_at:?} next {next:?} view_err {view_err:.3} axis_min {min_axis:.3}"),
    );
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

/// OnLevelStart: on every campaign level, walking out of the spawn starts the level and its
/// `onStart` brings in the first rooms (no void past the FirstRoom door).
pub fn first_rooms(install: &Path, filter: Option<&str>, r: &mut Report) {
    let dir = AssetDb::bundle_dir(install);
    let mut levels: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.strip_prefix("campaign_scenes_level").and_then(|s| s.strip_suffix(".bundle")).map(str::to_string)
        })
        .filter(|l| filter.is_none_or(|f| l.contains(f)))
        .collect();
    levels.sort();
    let mut db = AssetDb::open(install).unwrap();
    let (mut ok, mut with) = (0, 0);
    for l in &levels {
        let Ok(def) = scenedef::load_scene(&mut db, &dir.join(format!("campaign_scenes_level{l}.bundle"))) else { continue };
        let def = Arc::new(def);
        let targets: Vec<u32> = def
            .scripts
            .iter()
            .filter(|s| s.class == "OnLevelStart")
            .flat_map(|s| uk_game::scripts::nodes(&def, s.data.get("onStart").get("toActivateObjects")))
            .collect();
        if targets.is_empty() {
            continue;
        }
        with += 1;
        let mut g = Game::new(def.clone());
        let mut t = 0.0;
        for i in 0..400 {
            let input = Input { move_axis: bevy_math::Vec2::new(0.0, (i >= 250) as i32 as f32), ..Default::default() };
            g.fixed_update(&input);
            t += FIXED_DT as f64;
            g.update(&input, FIXED_DT, t);
        }
        let on = targets.iter().filter(|&&n| g.active(n)).count();
        if g.s.player.activated && on == targets.len() {
            ok += 1;
        } else {
            r.note(format!("FAIL first_rooms {l}: player activated {}, OnLevelStart targets active {on}/{}", g.s.player.activated, targets.len()));
        }
    }
    r.higher("first_rooms.levels_ok", ok as f64);
    r.pass("first_rooms.all", ok == with, format!("{ok}/{with} levels bring in their first rooms at level start"));
}
