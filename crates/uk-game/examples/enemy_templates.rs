//! Enemies in a level (default level0-1): path, class mix, spawnIn, active, root height above the capsule bottom.
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    for s in def.scripts.iter().filter(|s| s.class == "EnemyIdentifier") {
        let n = s.node;
        let classes: Vec<&str> = def.scripts_on(n).map(|(_, s)| s.class.as_str()).filter(|c| matches!(*c, "ZombieMelee" | "ZombieProjectiles" | "SpiderBody" | "MaliciousFace")).collect();
        let nodes = (0..def.nodes.len() as u32).filter(|&m| def.is_descendant(m, n)).count();
        let agent = def.nav_agents.iter().find(|a| a.node == n).map(|a| a.base_offset);
        println!("{} {:?} spawnIn={} active_self={} nodes={nodes} agent_offset={agent:?} scale={:.2?}", def.path(n), classes, s.data.get("spawnIn").bool(), def.nodes[n as usize].active_self, def.nodes[n as usize].world0.to_scale_rotation_translation().0);
    }
}
