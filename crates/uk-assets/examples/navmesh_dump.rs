//! Dump one NavMesh tile as words (u32 + f32) to reverse-engineer Unity's Detour v16 layout.
use uk_assets::db::AssetDb;
fn main() {
    let idx: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    for f in db.load_bundle_files(&path).unwrap() { for o in f.objects.clone() { if o.class_id != 238 { continue }
        let v = f.read(&o).unwrap();
        let tiles = v.get("m_NavMeshTiles").array();
        let mut by: Vec<(usize, usize)> = tiles.iter().enumerate().map(|(i, t)| (t.get("m_MeshData").bytes().len(), i)).collect();
        by.sort();
        println!("sizes: {:?}", by.iter().map(|x| x.0).collect::<Vec<_>>());
        let b = tiles[by[idx].1].get("m_MeshData").bytes();
        for (i, c) in b.chunks(4).enumerate().take(400) {
            let u = u32::from_le_bytes(c.try_into().unwrap()); let fl = f32::from_bits(u);
            let h = (u16::from_le_bytes([c[0], c[1]]), u16::from_le_bytes([c[2], c[3]]));
            println!("{:4} {:5} {u:08x} {u:>10} {fl:>14.4} u16 {:?}", i, i * 4, h.0.min(65535), );
            let _ = h;
        }
    }}
}
