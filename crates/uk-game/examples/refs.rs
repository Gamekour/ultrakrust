//! Scripts whose serialized fields reference a node (its GameObject or any of its components).
//! refs <exact node path> [level]
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let q = std::env::args().nth(1).expect("node path");
    let level = std::env::args().nth(2).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let node = (0..def.nodes.len() as u32).find(|&n| def.path(n) == q).expect("no such node");
    let ids: Vec<i64> = def.obj_to_node.iter().filter(|&(_, &n)| n == node).map(|(&id, _)| id).collect();
    println!("{q}: node {node} ids {ids:?} active_self {}", def.nodes[node as usize].active_self);
    for s in &def.scripts {
        let c = s.data.compact();
        let hit: Vec<_> = ids.iter().filter(|id| c.contains(&format!(":{id}}}")) || c.contains(&format!(":{id},")) || c.contains(&format!(":{id}]")) || c.contains(&format!(":{id} "))).collect();
        if !hit.is_empty() {
            let i = c.find(&format!("{}", hit[0])).unwrap_or(0);
            let lo = c[..i].char_indices().rev().nth(200).map_or(0, |x| x.0);
            println!("  {} on {} (file {:?}): ...{}...", s.class, def.path(s.node), s.file, &c[lo..(i + 40).min(c.len())]);
        }
    }
}
