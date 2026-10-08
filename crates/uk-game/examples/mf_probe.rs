//! Malicious Face: its agent and colliders (head mesh AABB / open edges), then, once activated
//! (MANUAL=1 switches its chain on, else the player walks in), player contact scenarios (walk,
//! dash, jump; VA=1 / V=1 per tick), the floor tags under the arena, NavMeshAgent movement toward
//! the player, and the corpse: fall, landing on the Floor tag, then walking / dashing / dropping
//! into it.
//! cargo run --release -p uk-game --example mf_probe -- [level0-1]
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::enemy::Kind;
use uk_game::Game;

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let mut g = Game::new(def.clone());
    let faces: Vec<usize> = (0..g.s.enemies.len()).filter(|&e| g.s.enemies[e].kind == Kind::MaliciousFace).collect();
    for &e in &faces {
        let n = g.s.enemies[e].node;
        println!("MF #{e} {} pos0 {:?} radius {} center_y {} rig {:?}", def.path(n), g.s.enemies[e].pos0, g.s.enemies[e].radius, g.s.enemies[e].center_y, g.s.enemies[e].rig);
        for a in def.nav_agents.iter().filter(|a| a.node == n) {
            println!("  agent {a:?}");
        }
        println!("  scale {:?}", def.nodes[n as usize].world0.to_scale_rotation_translation().0);
        for (ci, c) in def.colliders.iter().enumerate() {
            if def.is_descendant(c.node, n) || c.node == n {
                println!("  collider {ci} {} layer {} trigger {} animated {}", def.nodes[c.node as usize].name, c.layer, c.trigger, g.anim.animated(c.node));
                if let scenedef::ShapeDef::Mesh(tris) = &c.shape {
                    let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
                    for t in tris {
                        for v in t {
                            lo = lo.min(*v);
                            hi = hi.max(*v);
                        }
                    }
                    println!("    mesh aabb {lo:.2?} .. {hi:.2?}");
                    // open edges (used by one triangle), by quantised endpoints
                    let q = |v: Vec3| ((v.x * 1000.0).round() as i64, (v.y * 1000.0).round() as i64, (v.z * 1000.0).round() as i64);
                    let mut edges: std::collections::HashMap<_, (u32, Vec3, Vec3)> = std::collections::HashMap::new();
                    for t in tris {
                        for k in 0..3 {
                            let (a, b) = (t[k], t[(k + 1) % 3]);
                            let key = if q(a) < q(b) { (q(a), q(b)) } else { (q(b), q(a)) };
                            edges.entry(key).or_insert((0, a, b)).0 += 1;
                        }
                    }
                    let open: Vec<_> = edges.values().filter(|e| e.0 == 1).collect();
                    let (mut olo, mut ohi) = (Vec3::MAX, Vec3::MIN);
                    for e in &open {
                        olo = olo.min(e.1.min(e.2));
                        ohi = ohi.max(e.1.max(e.2));
                    }
                    println!("    {} tris, {} edges, {} open edges spanning {olo:.2?} .. {ohi:.2?}", tris.len(), edges.len(), open.len());
                }
            }
        }
    }
    let Some(&e) = faces.first() else { return };
    let mut tagc = std::collections::BTreeMap::new();
    for c in &def.colliders {
        *tagc.entry((def.nodes[c.node as usize].tag, c.layer)).or_insert(0) += 1;
    }
    println!("  (tag, layer) counts {tagc:?}");
    // what is drawn under the Face's parent, and which mover carries it
    let spider = def.nodes[g.s.enemies[e].node as usize].parent.unwrap();
    for r in &def.renderers {
        if def.is_descendant(r.node, spider) || r.node == spider {
            let bones = r.skin.as_ref().map(|s| s.bones.iter().flatten().filter(|&&b| !def.is_descendant(b, g.s.enemies[e].node) && b != g.s.enemies[e].node).count());
            println!("  draw {} skin {} bones outside Body {:?} mover {:?} (Body mover {:?})", def.path(r.node), r.skin.is_some(), bones, g.node_mover[r.node as usize], g.node_mover[g.s.enemies[e].node as usize]);
            let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
            for p in &r.batch.positions {
                lo = lo.min(Vec3::from(*p));
                hi = hi.max(Vec3::from(*p));
            }
            println!("    enabled {} active {} aabb {lo:.2?} .. {hi:.2?}", r.enabled, g.active(r.node));
        }
    }
    for (ci, c) in def.colliders.iter().enumerate() {
        if def.is_descendant(c.node, spider) && !def.is_descendant(c.node, g.s.enemies[e].node) && c.node != g.s.enemies[e].node {
            println!("  sibling collider {ci} {} layer {} trigger {}", def.path(c.node), c.layer, c.trigger);
        }
    }
    // activate it the way the level does: the player walks into the arena
    if std::env::var("MANUAL").is_ok() {
        let mut chain = vec![g.s.enemies[e].node];
        while let Some(p) = def.nodes[*chain.last().unwrap() as usize].parent {
            chain.push(p);
        }
        for &m in chain.iter().rev() {
            g.set_active(m, true);
        }
        g.s.player.activated = true;
    } else {
        g.s.player.activated = true;
        let mut t = 0.0;
        for (i, z) in [-560.0f32, -500.0, -450.0, -430.0].into_iter().enumerate() {
            g.s.player.pos = Vec3::new(202.0, 54.5, z);
            g.s.player.prev_pos = g.s.player.pos;
            g.s.player.vel = Vec3::ZERO;
            for _ in 0..100 {
                g.s.hp = 100;
                g.fixed_update(&Input::default());
                t += FIXED_DT as f64;
                g.update(&Input::default(), FIXED_DT, t);
            }
            let en = &g.s.enemies[e];
            let b = &g.s.player.bodies;
            println!("  walk-in {i} z {z}: player {:.2?} MF active {} alive {} spawn_t {:.2} pos {:.2?} | 1370 on {} 1372 on {} 1572 on {} | group xf {:.2?}", g.s.player.pos, g.active(en.node), en.alive, en.spawn_t, en.pos, b.enabled(1370), b.enabled(1372), b.enabled(1572), b.world.groups[e + 1].xf.translation);
        }
    }
    for r in &def.renderers {
        if !g.active(r.node) || !r.enabled {
            continue;
        }
        let (mut lo, mut hi) = (Vec3::MAX, Vec3::MIN);
        for p in &r.batch.positions {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
        if lo.x < 204.0 && hi.x > 200.0 && lo.y < 65.0 && hi.y > 61.0 && lo.z < -407.0 && hi.z > -411.0 && (hi - lo).length() < 30.0 {
            println!("  drawn near the head: {} {lo:.2?}..{hi:.2?} mover {:?}", def.path(r.node), g.node_mover[r.node as usize]);
        }
    }
    // jump into it from the arena floor below, the Face moving freely
    let gap = |g: &Game| {
        use uk_core::player::{INVINCIBLE_LAYER_COLLIDES, PLAYER_LAYER_COLLIDES};
        let p = &g.s.player;
        let b = &p.bodies;
        let cap = p.capsule();
        let mask = if p.invincible_layer { INVINCIBLE_LAYER_COLLIDES } else { PLAYER_LAYER_COLLIDES };
        let grp = &b.world.groups[e + 1];
        let inv = grp.xf.inverse();
        let (a, c) = (inv.transform_point3(cap.a), inv.transform_point3(cap.b));
        let mut best = (f32::MAX, 0usize);
        for (i, sh) in grp.shapes.iter().enumerate() {
            let o = grp.owners[i] as usize;
            if b.trigger[o] || mask & (1 << b.layer[o]) == 0 || !b.enabled(o as u32) {
                continue;
            }
            let (x, y) = sh.closest_to_segment(a, c);
            let d = x.distance(y) - cap.radius;
            if d < best.0 {
                best = (d, o);
            }
        }
        (best, cap.b.y + cap.radius)
    };
    let mut t = 0.0;
    for (name, dash, off, walk) in [("jump straight up under it", false, 0.0, false), ("jump walking in 6", false, 6.0, true), ("jump walking in 8", false, 8.0, true), ("jump walking in 10", false, 10.0, true), ("jump walking in 12", false, 12.0, true), ("jump walking in 14", false, 14.0, true), ("jump+dash in 10", true, 10.0, true), ("jump+dash in 14", true, 14.0, true)] {
        let c = g.s.enemies[e].center();
        g.s.player.pos = Vec3::new(c.x, 54.5, c.z + off);
        g.s.player.prev_pos = g.s.player.pos;
        g.s.player.vel = Vec3::ZERO;
        g.s.player.yaw_deg = 0.0;
        let mut min = (f32::MAX, 0);
        let mut top = f32::MIN;
        for i in 0..300 {
            g.s.hp = 100;
            let fwd = bevy_math::Vec2::new(0.0, if walk { 1.0 } else { 0.0 });
            let inp = Input { jump_pressed: i == 30, jump_held: (30..60).contains(&i), dash_pressed: dash && i == 45, move_axis: fwd, ..Default::default() };
            g.fixed_update(&inp);
            t += FIXED_DT as f64;
            g.update(&inp, FIXED_DT, t);
            let ((d, o), tp) = gap(&g);
            if d < min.0 {
                min = (d, o);
            }
            top = top.max(tp);
            {
                let en = &g.s.enemies[e];
                let b = &g.s.player.bodies;
                let st = (g.active(en.node), b.enabled(1370), b.enabled(1572), g.active(def.colliders[1572].node), g.s.collider_enabled[1572], g.s.collider_enabled[1370]);
                if i == 0 || std::env::var("VA").is_ok() && i % 10 == 0 {
                    println!("   {name} {i}: MF active/1370 on/1572 on/head node active/1572 cenabled/1370 cenabled {st:?} top {tp:.2}");
                }
            }
            if std::env::var("V").is_ok() && (25..90).contains(&i) && i % 3 == 0 {
                println!("   {i} top {tp:.2} gap {d:.3} pos {:.2?} vel {:.2?}", g.s.player.pos, g.s.player.vel);
            }
        }
        println!("{name}: capsule top max {top:.2}, min gap {:.3} (collider {}), face center {:.2?}, player ends {:.2?}", min.0, min.1, g.s.enemies[e].center(), g.s.player.pos);
    }
    for (x, z) in [(202.0, -420.0), (190.0, -409.0), (202.0, -398.0), (215.0, -409.0)] {
        if let Some(h) = g.world.raycast(Vec3::new(x, 58.0, z), Vec3::NEG_Y, 100.0) {
            let o = g.world.owner(h.collider);
            let c = &def.colliders[o as usize];
            println!("  floor under ({x}, {z}): y {:.2} {} tag {} layer {}", h.point.y, def.path(c.node), def.nodes[c.node as usize].tag, c.layer);
        }
    }
    // NavMeshAgent movement: the player stands on the arena floor; the Face heads for them
    let mut t = 0.0;
    let mut step = |g: &mut Game, inp: &Input| {
        g.s.hp = 100;
        g.fixed_update(inp);
        t += FIXED_DT as f64;
        g.update(inp, FIXED_DT, t);
    };
    g.s.player.pos = Vec3::new(202.0, 54.5, -405.0);
    g.s.player.prev_pos = g.s.player.pos;
    g.s.player.vel = Vec3::ZERO;
    for i in 0..=600 {
        step(&mut g, &Input::default());
        if i % 100 == 0 {
            let en = &g.s.enemies[e];
            println!("move t+{:.1}s: face {:.2?} agent vel {:.2?} beam_charge {:.2} | flat distance to the player {:.2} | feet-to-navmesh {:.2}", i as f32 * FIXED_DT, en.pos, en.agent_vel, en.beam_charge, (en.pos - g.s.player.pos).with_y(0.0).length(), en.pos.y - 53.0);
        }
    }
    let (gap_now, top) = gap(&g);
    println!("move: player under the face: gap {:.3} (collider {}), capsule top {top:.2}", gap_now.0, gap_now.1);
    // death: the corpse falls onto the Floor, drops 1.5 and stays solid (head mesh, layer 11)
    uk_game::enemy::kill_enemy(&mut g, e);
    g.s.player.pos = g.s.enemies[e].pos + Vec3::new(0.0, 0.0, 12.0);
    g.s.player.pos.y = 54.5;
    g.s.player.prev_pos = g.s.player.pos;
    for i in 0..=150 {
        step(&mut g, &Input::default());
        if i % 10 == 0 || !g.s.enemies[e].corpse_falling && g.s.enemies[e].corpse_landed && i % 50 == 0 {
            let en = &g.s.enemies[e];
            let b = &g.s.player.bodies;
            println!("corpse t+{:.2}s: centre {:.2?} falling {} landed {} | sphere on {} layer {} | trigger on {} | head on {}", i as f32 * FIXED_DT, en.center(), en.corpse_falling, en.corpse_landed, b.enabled(1370), b.layer[1370], b.enabled(1372), b.enabled(1572));
        }
    }
    // walk into the corpse (dashing too), then drop onto it
    let cc = g.s.enemies[e].center();
    for (name, dash) in [("walk into corpse", false), ("dash into corpse", true)] {
        g.s.player.pos = Vec3::new(cc.x, 54.5, cc.z + 10.0);
        g.s.player.prev_pos = g.s.player.pos;
        g.s.player.vel = Vec3::ZERO;
        g.s.player.yaw_deg = 0.0;
        let mut min = (f32::MAX, 0);
        for i in 0..150 {
            step(&mut g, &Input { move_axis: bevy_math::Vec2::new(0.0, 1.0), dash_pressed: dash && i == 5, ..Default::default() });
            let ((d, o), _) = gap(&g);
            if d < min.0 {
                min = (d, o);
            }
        }
        println!("{name}: min gap {:.3} (collider {}), player ends {:.2?} (corpse centre z {:.2})", min.0, min.1, g.s.player.pos, cc.z);
    }
    g.s.player.pos = cc + Vec3::new(0.0, 6.0, 0.0);
    g.s.player.prev_pos = g.s.player.pos;
    g.s.player.vel = Vec3::ZERO;
    for _ in 0..150 {
        step(&mut g, &Input::default());
    }
    println!("drop onto corpse: player rests at {:.2?}, onGround {} (corpse mesh top {:.2})", g.s.player.pos, g.s.player.gc.on_ground, cc.y + 2.22);
}
