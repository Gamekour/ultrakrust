//! Print a fingerprint of each part of a loaded scene, to diff across processes.
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let h = |s: String| { let mut h: u64 = 1469598103934665603; for b in s.bytes() { h ^= b as u64; h = h.wrapping_mul(1099511628211) } h };
    println!("nodes     {:x}", h(format!("{:?}", def.nodes.iter().map(|n| (&n.name, n.parent, &n.children, n.world0.to_cols_array().map(f32::to_bits))).collect::<Vec<_>>())));
    println!("colliders {:x}", h(format!("{:?}", def.colliders)));
    println!("scripts   {:x}", h(format!("{:?}", def.scripts.iter().map(|s| (&s.class, s.node, s.enabled, s.data.compact())).collect::<Vec<_>>())));
    println!("renderers {:x}", h(format!("{}", def.renderers.len())));
    if std::env::var("DUMP").is_ok() { for (i, c) in def.colliders.iter().enumerate() { println!("C{i} {:?}", c) } for (i, s) in def.scripts.iter().enumerate() { println!("S{i} {} {} {}", s.class, s.node, s.data.compact()) } }
}
