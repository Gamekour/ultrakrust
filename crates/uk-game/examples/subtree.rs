//! A node's subtree with active flags and script classes. subtree <path substring> [level]
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let q = std::env::args().nth(1).unwrap();
    let level = std::env::args().nth(2).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let Some(root) = (0..def.nodes.len() as u32).find(|&n| def.path(n).ends_with(&q)) else { return println!("not found") };
    let depth = |n: u32| (0..).scan(n, |m, _| { let p = def.nodes[*m as usize].parent?; *m = p; Some(()) }).count();
    let d0 = depth(root);
    let mut stack = vec![root];
    while let Some(n) = stack.pop() {
        let nd = &def.nodes[n as usize];
        let cls: Vec<&str> = def.scripts_on(n).map(|(_, s)| s.class.as_str()).collect();
        let cols = def.colliders.iter().filter(|c| c.node == n).count();
        println!("{}{} [{}] active={} layer={} cols={cols} pos={:.2?}", "  ".repeat(depth(n) - d0), nd.name, cls.join(","), nd.active_self, nd.layer, nd.world0.w_axis.truncate());
        if std::env::var("DEEP").is_ok() || n == root {
            stack.extend(nd.children.iter().rev().copied());
        }
    }
}
