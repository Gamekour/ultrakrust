//! Animator slots whose transform path finds no node, per Animator (layer-13 viewmodel first).
//! unbound [level, default level0-1]
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let q = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{q}.bundle"))).unwrap());
    let g = uk_game::Game::new(def.clone());
    // any path suffix of any node: tells which hierarchy the slot expected
    let mut suffix: std::collections::HashMap<u32, String> = Default::default();
    for n in 0..def.nodes.len() as u32 {
        let p = def.path(n);
        let parts: Vec<&str> = p.split('/').collect();
        for k in 0..parts.len() {
            let s = parts[k..].join("/");
            suffix.entry(uk_assets::anim::path_hash(&s)).or_insert(format!("{p} [{s}]"));
        }
    }
    if std::env::var_os("UK_TREE").is_some() {
        for n in 0..def.nodes.len() as u32 {
            let p = def.path(n);
            if p.contains("Punch/Arm Blue") {
                println!("TREE {p}");
            }
        }
    }
    let mut by: std::collections::BTreeMap<u32, Vec<u32>> = Default::default();
    for &(a, h) in &g.anim.unbound {
        by.entry(a).or_default().push(h);
    }
    for (a, hs) in by {
        let ad = &def.animators[a as usize];
        let vm = def.nodes[ad.node as usize].layer == scenedef::VIEWMODEL_LAYER;
        let c = &def.controllers[ad.controller.unwrap() as usize].ctrl;
        let names: Vec<_> = hs.iter().take(8).map(|h| suffix.get(h).cloned().unwrap_or_else(|| c.name(*h).to_string())).collect();
        println!("{}{} unbound {}: {names:?}", if vm { "VM " } else { "" }, def.path(ad.node), hs.len());
    }
}
