//! The viewmodel after load: prefabs spawned under GunSetter, layer-13 renderers / skins /
//! animators, and the HUD Camera. viewmodel [level, default 0-1]
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let q = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{q}.bundle"))).unwrap();
    for w in def.warnings.iter().filter(|w| w.contains("viewmodel")) {
        println!("warning: {w}");
    }
    let vm = |n: u32| def.nodes[n as usize].layer == scenedef::VIEWMODEL_LAYER;
    let rs: Vec<_> = def.renderers.iter().filter(|r| vm(r.node)).collect();
    println!("layer-13 renderers {} (skinned {}), nodes {}", rs.len(), rs.iter().filter(|r| r.skin.is_some()).count(), (0..def.nodes.len() as u32).filter(|&n| vm(n)).count());
    for r in &rs {
        let unbound = r.skin.as_ref().map_or(0, |s| s.bones.iter().filter(|b| b.is_none()).count());
        let b = &r.batch;
        let against = b.indices.chunks_exact(3).filter(|t| {
            let v = |i: u32| bevy_math::Vec3::from(b.positions[i as usize]);
            let n = |i: u32| bevy_math::Vec3::from(b.normals[i as usize]);
            (v(t[1]) - v(t[0])).cross(v(t[2]) - v(t[0])).dot(n(t[0]) + n(t[1]) + n(t[2])) < 0.0
        }).count();
        let det = def.nodes[r.node as usize].world0.determinant();
        println!("  #{} {} active_self {} tris {} against {against} det {det:.3} mat {} unbound bones {unbound} enabled {}", r.node, def.path(r.node), def.nodes[r.node as usize].active_self, r.batch.indices.len() / 3, r.material.is_some(), r.enabled);
    }
    for a in def.animators.iter().filter(|a| vm(a.node)) {
        let c = a.controller.map(|c| (def.controllers[c as usize].ctrl.clips.len(), def.controllers[c as usize].clips.iter().flatten().count()));
        println!("  animator {} controller (clips, decoded) {c:?}", def.path(a.node));
        let Some(c) = a.controller else { continue };
        let c = &def.controllers[c as usize].ctrl;
        let ps: Vec<_> = c.params.iter().map(|p| format!("{}:{}", c.name(p.id), p.kind)).collect();
        println!("    params {ps:?}");
        for (li, l) in c.layers.iter().enumerate() {
            let m = &c.machines[l.machine as usize];
            println!("    layer {li} weight {} default {}", l.weight, m.default_state);
            let cond = |ks: &[uk_assets::anim::Condition]| ks.iter().map(|k| format!("{} m{} {}", c.name(k.param), k.mode, k.threshold)).collect::<Vec<_>>();
            for (si, st) in m.states.iter().enumerate() {
                let tr: Vec<_> = st.transitions.iter().map(|t| format!("->{} {:?} exit {}", t.dest, cond(&t.conditions), if t.has_exit_time { t.exit_time } else { -1.0 })).collect();
                println!("      [{si}] {} {tr:?}", c.name(st.name));
            }
            for t in &m.any_state {
                println!("      any ->{} {:?}", t.dest, cond(&t.conditions));
            }
        }
    }
    for s in def.scripts.iter().filter(|s| s.file.is_some()) {
        println!("  prefab script {} on {}", s.class, def.path(s.node));
    }
    if let Some(h) = def.find("HUD Camera") {
        let (_, r, t) = def.nodes[h as usize].world0.to_scale_rotation_translation();
        println!("HUD Camera {} pos {t:?} rot {r:?}", def.path(h));
    }
}
