//! Prints checksums of the biggest mesh in a level, for comparison with UnityPy.
use uk_assets::{db::AssetDb, mesh};
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    for f in db.load_bundle_files(&path).unwrap() {
        let big = f.objects.iter().filter(|o| o.class_id == 43).max_by_key(|o| o.byte_size);
        let Some(o) = big else { continue };
        let m = mesh::decode(&f.read(o).unwrap(), None).unwrap();
        let s: f64 = m.positions.iter().map(|p| (p[0] + p[1] + p[2]) as f64).sum();
        let n: f64 = m.normals.iter().map(|p| (p[0] + p[1] + p[2]) as f64).sum();
        let idx: usize = m.submeshes.iter().map(|s| s.indices.len()).sum();
        let isum: u64 = m.submeshes.iter().flat_map(|s| s.indices.iter()).map(|&i| i as u64).sum();
        let avg: f64 = m.normals.iter().map(|p| ((p[0]*p[0]+p[1]*p[1]+p[2]*p[2]) as f64).sqrt()).sum::<f64>() / m.normals.len().max(1) as f64;
        println!("avg normal len {avg:.4}");
        println!("{} verts={} possum={s:.3} nsum={n:.3} idx={idx} idxsum={isum} uv0={:?}", m.name, m.positions.len(), m.uv0.get(7));
    }
}
