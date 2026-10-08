//! Root GameObjects of a scene with their subtree sizes, renderers, colliders and script classes;
//! with a second argument, the same for the children of the root of that name.
//! cargo run --release -p uk-game --example scene_roots -- [level0-1] [FirstRoom]
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let level = a.get(1).cloned().unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let list: Vec<u32> = match a.get(2) {
        Some(n) => def.nodes[def.find(n).unwrap() as usize].children.clone(),
        None => (0..def.nodes.len() as u32).filter(|&i| def.nodes[i as usize].parent.is_none()).collect(),
    };
    for r in list {
        let under = |n: u32| def.is_descendant(n, r);
        let nodes = (0..def.nodes.len() as u32).filter(|&n| under(n)).count();
        let rend = def.renderers.iter().filter(|x| under(x.node)).count();
        let col = def.colliders.iter().filter(|x| under(x.node)).count();
        let lights = def.lights.iter().filter(|x| under(x.node)).count();
        let mut cls = std::collections::BTreeMap::<&str, usize>::new();
        for s in def.scripts.iter().filter(|s| under(s.node)) {
            *cls.entry(&s.class).or_default() += 1;
        }
        let mut cls: Vec<_> = cls.into_iter().collect();
        cls.sort_by(|a, b| b.1.cmp(&a.1));
        println!("{} active {} layer {}: {nodes} nodes, {rend} renderers, {col} colliders, {lights} lights; {:?}", def.nodes[r as usize].name, def.nodes[r as usize].active_self, def.nodes[r as usize].layer, &cls[..cls.len().min(14)]);
    }
}
