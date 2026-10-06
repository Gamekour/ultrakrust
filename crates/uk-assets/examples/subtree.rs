//! Nodes whose path contains NEEDLE: scripts and renderers (with material names) on each.
//! cargo run --release -p uk-assets --example subtree -- level0-1 "Virtual Camera"
use uk_assets::db::AssetDb;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (level, needle) = (a.get(1).cloned().unwrap_or("level0-1".into()), a.get(2).cloned().unwrap_or("Virtual Camera".into()));
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = uk_assets::scenedef::load_scene(&mut db, &path).unwrap();
    for (i, n) in def.nodes.iter().enumerate() {
        let p = def.path(i as u32);
        if !p.contains(&needle) { continue }
        let scripts: Vec<&str> = def.scripts_on(i as u32).map(|(_, s)| s.class.as_str()).collect();
        let rs: Vec<String> = def.renderers.iter().filter(|r| r.node == i as u32).map(|r| format!("{:?} en {}", r.material.as_ref().map(|m| m.path_id), r.enabled)).collect();
        println!("{p} active {} layer {} scripts {scripts:?} renderers {rs:?}", n.active_self, n.layer);
    }
}
