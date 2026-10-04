//! cargo run --release -p uk-assets --example dump_scene -- level0-1 [ClassName] [count]
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or_else(|| "level0-1".into());
    let show = std::env::args().nth(2);
    let n: usize = std::env::args().nth(3).and_then(|x| x.parse().ok()).unwrap_or(5);
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let t = std::time::Instant::now();
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    println!(
        "loaded in {:?}: {} nodes, {} renderers, {} colliders ({} triggers), {} scripts, {} warnings",
        t.elapsed(),
        def.nodes.len(),
        def.renderers.len(),
        def.colliders.len(),
        def.colliders.iter().filter(|c| c.trigger).count(),
        def.scripts.len(),
        def.warnings.len()
    );
    let mut classes: BTreeMap<&str, usize> = BTreeMap::new();
    for s in &def.scripts {
        *classes.entry(&s.class).or_default() += 1;
    }
    let mut v: Vec<_> = classes.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    if std::env::var("ALL").is_ok() { for (k, n) in &v { println!("{n} {k}"); } } else { println!("{:?}", &v[..v.len().min(40)]); }
    if let Some(c) = show {
        for s in def.scripts.iter().filter(|s| s.class == c).take(n) {
            println!("--- {} [{}] {}", c, s.node, def.path(s.node));
            println!("{}", s.data.compact());
        }
    }
}
