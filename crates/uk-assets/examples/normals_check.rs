//! Render-geometry sanity, no screenshots: for every baked triangle, does the stored vertex normal agree
//! with the face normal implied by Bevy's CCW front-face winding? Reports per level and the worst nodes.
//! normals_check [level filter]
use bevy_math::Vec3;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let filter = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut ps: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| { let n = p.file_name().unwrap().to_string_lossy(); n.contains("campaign_scenes_") && n.contains(&*filter) }).collect();
    ps.sort();
    for p in ps {
        let def = scenedef::load_scene(&mut db, &p).unwrap();
        let (mut agree, mut against, mut zero_n, mut degenerate) = (0usize, 0usize, 0usize, 0usize);
        let mut per_node: Vec<(usize, usize, u32)> = Vec::new();
        for r in &def.renderers {
            let b = &r.batch;
            let (mut a, mut x) = (0, 0);
            for t in b.indices.chunks_exact(3) {
                let v = |i: u32| Vec3::from(b.positions[i as usize]);
                let n = |i: u32| Vec3::from(b.normals[i as usize]);
                let face = (v(t[1]) - v(t[0])).cross(v(t[2]) - v(t[0]));
                if face.length_squared() < 1e-12 { degenerate += 1; continue }
                let vn = n(t[0]) + n(t[1]) + n(t[2]);
                if vn.length_squared() < 1e-6 { zero_n += 1; continue }
                if face.dot(vn) >= 0.0 { a += 1 } else { x += 1 }
            }
            agree += a; against += x;
            if x > 0 { per_node.push((x, a + x, r.node)); }
        }
        let name = p.file_name().unwrap().to_string_lossy().replace("campaign_scenes_", "");
        println!("{:14} tris agree {agree:7} AGAINST {against:7} ({:.1}%) zero-normal {zero_n} degenerate {degenerate}",
            name.split(".bundle").next().unwrap(), 100.0 * against as f64 / (agree + against).max(1) as f64);
        per_node.sort_by(|a, b| b.0.cmp(&a.0));
        for (x, tot, n) in per_node.iter().take(6) { println!("    {x:6}/{tot:6} against  {}", def.path(*n)); }
    }
}
