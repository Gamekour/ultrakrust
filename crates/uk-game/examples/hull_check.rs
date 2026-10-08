//! Every convex MeshCollider hull in a level: closed (every edge has its twin), convex (all of
//! the mesh's points on or behind every face), faces = 2V - 4. hull_check [level1-1]
use bevy_math::Vec3;
use std::collections::BTreeSet;
use uk_assets::{db::AssetDb, scenedef::{self, ShapeDef}};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level1-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let t = std::time::Instant::now();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    println!("scene loaded in {:?}", t.elapsed());
    let (mut n, mut bad, mut max_faces) = (0, 0, 0);
    for c in &def.colliders {
        let ShapeDef::Mesh(tris) = &c.shape else { continue };
        // hulls: re-run on the hull itself, it must come back with the same face count
        let Some(h) = uk_assets::gltf_map::convex_hull(tris) else { continue };
        if h.len() != tris.len() {
            continue; // not a hull (a concave mesh collider)
        }
        n += 1;
        max_faces = max_faces.max(h.len());
        let key = |p: Vec3| ((p.x * 1e4) as i64, (p.y * 1e4) as i64, (p.z * 1e4) as i64);
        let verts: BTreeSet<_> = tris.iter().flatten().map(|&p| key(p)).collect();
        let edges: BTreeSet<_> = tris.iter().flat_map(|t| [(key(t[0]), key(t[1])), (key(t[1]), key(t[2])), (key(t[2]), key(t[0]))]).collect();
        let open = edges.iter().filter(|(u, v)| !edges.contains(&(*v, *u))).count();
        let size = tris.iter().flatten().fold((Vec3::MAX, Vec3::MIN), |(l, h), p| (l.min(*p), h.max(*p)));
        let eps = (size.1 - size.0).max_element() * 1e-3;
        let worst = tris.iter().map(|f| {
            let nrm = (f[1] - f[0]).cross(f[2] - f[0]).normalize_or_zero();
            tris.iter().flatten().map(|&p| nrm.dot(p - f[0])).fold(f32::MIN, f32::max)
        }).fold(f32::MIN, f32::max);
        if open > 0 || worst > eps || tris.len() != 2 * verts.len() - 4 {
            bad += 1;
            if std::env::var("DUMP").is_ok_and(|d| def.path(c.node).contains(&d)) {
                for t in tris {
                    println!("tri {:?} {:?} {:?}", t[0].to_array(), t[1].to_array(), t[2].to_array());
                }
            }
            if bad <= 10 {
                println!("{}: {} faces, {} verts, open edges {open}, worst point outside {worst:.4} (eps {eps:.4})", def.path(c.node), tris.len(), verts.len());
            }
        }
    }
    println!("{n} hull-shaped mesh colliders, {bad} not closed and convex, largest {max_faces} faces");
}
