//! cargo run --release -p uk-game --example ctrl_dump -- [level bundle substring]
//! Every controller used in a level: animator node, params (kind/default) and states per layer.
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let q = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{q}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut done = std::collections::HashSet::new();
    for a in &def.animators {
        let Some(ci) = a.controller else { continue };
        if !done.insert(ci) { continue; }
        let c = &def.controllers[ci as usize].ctrl;
        println!("== {} (node '{}')", c.name, def.nodes[a.node as usize].name);
        let ps: Vec<String> = c.params.iter().map(|p| format!("{}:{}", c.name(p.id), match p.kind { 1 => "f", 3 => "i", 4 => "b", 9 => "T", _ => "?" })).collect();
        println!("   params {} defaults f{:?} b{:?}", ps.join(" "), c.floats, c.bools);
        for ci2 in def.controllers[ci as usize].clips.iter().flatten() {
            let cl = &def.clips[*ci2 as usize];
            let ev: Vec<String> = cl.events.iter().map(|e| format!("{:.2}:{}({}{})", (e.time - cl.start) / cl.duration().max(1e-6), e.function, e.string, if e.int != 0 { format!(" i{}", e.int) } else { String::new() })).collect();
            println!("   clip '{}' {:.2}s loop{} {}", cl.name, cl.duration(), cl.looping, ev.join(" "));
        }
        for (li, l) in c.layers.iter().enumerate() {
            let m = &c.machines[l.machine as usize];
            let ss: Vec<&str> = m.states.iter().map(|s| c.name(s.name)).collect();
            let _ = li;
            println!("   L{li} w{} blend{} def '{}' states {}", l.weight, l.blending, ss.get(m.default_state as usize).unwrap_or(&""), ss.join(","));
            if std::env::var("TRANS").is_ok() {
                let fmt = |t: &uk_assets::anim::Transition| format!("->{}{} [{}]", ss.get(t.dest as usize).unwrap_or(&"?"), if t.has_exit_time { format!(" exit{:.2}", t.exit_time) } else { String::new() }, t.conditions.iter().map(|k| format!("{}{}{}", c.name(k.param), ["", "", "!", ">", "<", "exit", "==", "!="].get(k.mode as usize).unwrap_or(&"?"), if k.mode >= 3 && k.mode != 5 { k.threshold.to_string() } else { String::new() })).collect::<Vec<_>>().join("&"));
                for (si, st) in m.states.iter().enumerate() {
                    let sp = st.speed_param.map(|p| c.name(p).to_string()).unwrap_or_default();
                    println!("     {} spd{}{} {}", ss[si], st.speed, if sp.is_empty() { String::new() } else { format!("*{sp}") }, st.transitions.iter().map(fmt).collect::<Vec<_>>().join(" "));
                }
                println!("     ANY {}", m.any_state.iter().map(fmt).collect::<Vec<_>>().join(" "));
            }
        }
    }
}
