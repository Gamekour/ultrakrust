//! Scripts on effect prefab nodes (root vs child), counted across the loaded prefabs of a level.
//! cargo run --release -p uk-game --example prefab_scripts -- [level0-1]
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let mut seen = std::collections::BTreeMap::<(String, bool), Vec<String>>::new();
    for pf in &def.particle_prefabs {
        for (i, n) in pf.nodes.iter().enumerate() {
            for s in &n.scripts {
                if i > 0 && ["RemoveOnTime", "RandomRotation", "ScaleTransform", "RandomForce"].contains(&s.class.as_str()) && std::env::args().any(|a| a == "-v") {
                    let sys = (0..pf.nodes.len()).filter(|&k| pf.nodes[k].system.is_some() && { let mut u = Some(k as u32); while u.is_some_and(|x| x != i as u32) { u = pf.nodes[u.unwrap() as usize].parent } u.is_some() }).count();
                    println!("{} node {i} '{}' {} systems under it {sys}: {:?}", pf.name, n.name, s.class, s.data);
                }
                seen.entry((s.class.clone(), i == 0)).or_default().push(pf.name.clone());
            }
        }
    }
    for ((c, root), v) in seen {
        println!("{c:32} {} {:3} {:?}", if root { "root " } else { "child" }, v.len(), &v[..v.len().min(4)]);
    }
}
