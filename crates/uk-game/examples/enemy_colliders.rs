//! Every enemy's colliders (EnemyIdentifier subtrees): layer, trigger, shape and size, grouped by
//! enemy type, plus Rigidbody state on the enemy root. Which of them the player's capsule (layer 2,
//! 15 while dashing) and GroundCheck (20) can touch follows PhysicsManager's matrix.
//! cargo run --release -p uk-game --example enemy_colliders -- [bundle filter, default level0-1]
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let filter = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.starts_with("campaign_scenes_") && n.contains(&*filter)
        })
        .collect();
    bundles.sort();
    // enemy type -> description -> count
    let mut seen: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for b in &bundles {
        let Ok(def) = scenedef::load_scene(&mut db, b) else { continue };
        for s in def.scripts.iter().filter(|s| s.class == "EnemyIdentifier") {
            let root = s.node;
            let ty = format!("{} ({})", s.data.get("enemyType").i64(), def.nodes[root as usize].name.split(" (").next().unwrap_or(""));
            let mut sub = vec![root];
            let mut i = 0;
            while i < sub.len() {
                sub.extend(def.nodes[sub[i] as usize].children.iter().copied());
                i += 1;
            }
            let rb = def.scripts_on(root).map(|(_, s)| s.class.clone()).collect::<Vec<_>>();
            let entry = seen.entry(ty).or_default();
            *entry.entry(format!("root scripts {}", rb.iter().filter(|c| ["NavMeshAgent", "Rigidbody"].contains(&c.as_str())).cloned().collect::<Vec<_>>().join(","))).or_default() += 1;
            for c in def.colliders.iter().filter(|c| sub.contains(&c.node)) {
                let depth = {
                    let mut d = 0;
                    let mut n = c.node;
                    while n != root {
                        n = def.nodes[n as usize].parent.unwrap();
                        d += 1;
                    }
                    d
                };
                let shape = match &c.shape {
                    scenedef::ShapeDef::Box { half, .. } => format!("box {:.2}x{:.2}x{:.2}", half.x * 2.0, half.y * 2.0, half.z * 2.0),
                    scenedef::ShapeDef::Sphere { radius, .. } => format!("sphere r{radius:.2}"),
                    scenedef::ShapeDef::Capsule { a, b, radius } => format!("capsule r{radius:.2} h{:.2}", a.distance(*b) + 2.0 * radius),
                    scenedef::ShapeDef::Mesh(t) => format!("mesh {} tris", t.len()),
                };
                let name = if c.node == root { "<root>".to_string() } else { def.nodes[c.node as usize].name.clone() };
                *entry.entry(format!("depth {depth} {name}: layer {} {}{} {shape}", c.layer, if c.trigger { "trigger " } else { "" }, if c.enabled { "" } else { "disabled " })).or_default() += 1;
            }
        }
    }
    for (ty, cols) in &seen {
        println!("{ty}");
        for (d, n) in cols {
            println!("  x{n:<3} {d}");
        }
    }
}
