//! cargo run --release -p uk-game --example inspect -- "path substring"
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let q = std::env::args().nth(1).expect("path substring");
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    for n in 0..def.nodes.len() as u32 {
        let p = def.path(n);
        if !p.contains(&q) { continue; }
        let nd = &def.nodes[n as usize];
        println!("[{n}] {p} active_self={} layer={} tag={} rb={}", nd.active_self, nd.layer, nd.tag, def.rigidbodies.contains(&n));
        for c in def.colliders.iter().filter(|c| c.node == n) {
            println!("    collider trigger={} enabled={} {:?}", c.trigger, c.enabled, match &c.shape { uk_assets::scenedef::ShapeDef::Mesh(t) => format!("Mesh({} tris)", t.len()), s => format!("{s:?}") });
        }
        for (_, s) in def.scripts_on(n) {
            if ["Breakable", "ObjectActivator", "ActivateArena", "Door", "PlayerActivator", "WeaponPickUp", "HudMessage", "FinalDoorOpener"].contains(&s.class.as_str()) {
                let c = s.data.compact();
                println!("    {} {}", s.class, &c[c.find("m_Name").unwrap_or(0)..c.len().min(c.find("m_Name").unwrap_or(0) + 700)]);
            } else {
                println!("    {}", s.class);
            }
        }
    }
}
