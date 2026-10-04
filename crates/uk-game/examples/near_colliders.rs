//! Colliders closest to a point (by shape centre / mesh vertex), with layer, trigger, owner path.
//! LEVEL=level6-2 near_colliders x y z
use std::sync::Arc;
use bevy_math::Vec3;
use uk_assets::{db::AssetDb, scenedef::{self, ShapeDef}};
fn main() {
    let a: Vec<f32> = std::env::args().skip(1).map(|s| s.parse().unwrap()).collect();
    let p = Vec3::new(a[0], a[1], a[2]);
    let level = std::env::var("LEVEL").unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut v: Vec<(f32, usize)> = def.colliders.iter().enumerate().map(|(i, c)| {
        let d = match &c.shape {
            ShapeDef::Box { center, .. } | ShapeDef::Sphere { center, .. } => center.distance(p),
            ShapeDef::Capsule { a, b, .. } => ((*a + *b) * 0.5).distance(p),
            ShapeDef::Mesh(t) => t.iter().flat_map(|t| t.iter()).map(|q| q.distance(p)).fold(f32::MAX, f32::min),
        };
        (d, i)
    }).collect();
    v.sort_by(|x, y| x.0.total_cmp(&y.0));
    println!("{} colliders; nearest to {p:?}:", def.colliders.len());
    for &(d, i) in v.iter().take(8) {
        let c = &def.colliders[i];
        println!("  {d:8.2}  layer {:2} trig {} en {} {} {}", c.layer, c.trigger, c.enabled, def.path(c.node), match &c.shape { ShapeDef::Mesh(t) => format!("mesh {} tris", t.len()), s => format!("{s:?}").chars().take(80).collect() });
    }
    // roots and how many colliders under each
    let mut roots = std::collections::BTreeMap::<String, usize>::new();
    for c in &def.colliders { let mut n = c.node; while let Some(q) = def.nodes[n as usize].parent { n = q } *roots.entry(def.nodes[n as usize].name.clone()).or_default() += 1; }
    println!("roots with colliders: {roots:?}");
    let mut r: Vec<(f32, u32)> = def.renderers.iter().map(|r| (def.nodes[r.node as usize].world0.w_axis.truncate().distance(p), r.node)).collect();
    r.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, n) in r.iter().take(4) { println!("  renderer {d:8.2} {}", def.path(*n)); }
    let mut ys: Vec<f32> = def.nodes.iter().map(|n| n.world0.w_axis.y).collect();
    ys.sort_by(|a, b| a.total_cmp(b));
    println!("node y range {:.1}..{:.1}, median {:.1}", ys[0], ys[ys.len() - 1], ys[ys.len() / 2]);
}
