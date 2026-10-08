//! Script classes on the nodes under a path prefix (active flag split), for scoping uGUI work.
//! cargo run --release -p uk-assets --example ui_classes -- level0-1 [path-prefix]
//! UI_CLASS=A,B lists those classes' active paths (UI_ALL=1: inactive too).
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or_else(|| "level0-1".into());
    let prefix = std::env::args().nth(2).unwrap_or_else(|| "FirstRoom/Player".into());
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let active = |mut n: u32| loop {
        let nd = &def.nodes[n as usize];
        if !nd.active_self {
            return false;
        }
        match nd.parent {
            Some(p) => n = p,
            None => return true,
        }
    };
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for s in &def.scripts {
        if !def.path(s.node).starts_with(&prefix) || def.nodes[s.node as usize].rect.is_none() {
            continue;
        }
        if std::env::var("UI_CLASS").is_ok_and(|c| c.split(',').any(|c| c == s.class)) && (std::env::var("UI_ALL").is_ok() || active(s.node) && s.enabled) {
            println!("{} {} {}{}", s.class, def.path(s.node), if active(s.node) { "" } else { "(inactive)" }, if s.enabled { "" } else { "(disabled)" });
        }
        let e = counts.entry(s.class.clone()).or_default();
        if active(s.node) && s.enabled {
            e.0 += 1;
        } else {
            e.1 += 1;
        }
    }
    for (c, (a, i)) in counts {
        println!("{c:32} active {a:5} inactive {i:5}");
    }
}
