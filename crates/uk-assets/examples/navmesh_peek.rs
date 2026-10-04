//! Peek at Unity's baked NavMeshData tiles: sizes and header (is it Detour's dtMeshHeader?).
use uk_assets::db::AssetDb;
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    for f in db.load_bundle_files(&path).unwrap() { for o in f.objects.clone() { if o.class_id != 238 { continue }
        let v = f.read(&o).unwrap();
        let tiles = v.get("m_NavMeshTiles").array();
        let sizes: Vec<usize> = tiles.iter().map(|t| t.get("m_MeshData").bytes().len()).collect();
        let big = tiles.iter().map(|t| t.get("m_MeshData").bytes()).max_by_key(|b| b.len()).unwrap();
        println!("{} tiles, total {} bytes, max {}; params {}", tiles.len(), sizes.iter().sum::<usize>(), big.len(),
            { let c = v.compact(); let i = c.find("m_NavMeshBuildSettings").unwrap_or(0); c[i..c.len().min(i + 500)].to_string() });
        let w: Vec<String> = big[..64.min(big.len())].chunks(4).map(|c| format!("{:08x}", u32::from_le_bytes(c.try_into().unwrap()))).collect();
        println!("header words: {}", w.join(" "));
    }}
}
