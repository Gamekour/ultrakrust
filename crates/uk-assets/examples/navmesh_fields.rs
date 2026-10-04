//! Non-tile NavMeshData fields per level (agent types, transforms, off-mesh links), and the size-formula check.
use uk_assets::db::AssetDb;
fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut ps: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| { let n = p.file_name().unwrap().to_string_lossy(); n.contains("_scenes_") && n.contains(&*filter) }).collect();
    ps.sort();
    let (mut ok, mut bad) = (0, 0);
    for p in ps {
        let n = p.file_name().unwrap().to_string_lossy().to_string();
        for f in db.load_bundle_files(&p).unwrap() { for o in f.objects.clone() { if o.class_id != 238 { continue }
            let v = f.read(&o).unwrap();
            for t in v.get("m_NavMeshTiles").array() {
                let b = t.get("m_MeshData").bytes();
                let w = |i: usize| u32::from_le_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()) as usize;
                let want = 72 + w(6) * 12 + w(5) * 32 + w(7) * 12 + w(8) * 12 + w(9) * 8 + w(10) * 16;
                if b.len() == want && w(0) == 0x444e4156 && w(1) == 16 { ok += 1 } else { bad += 1; println!("{n}: tile size {} != {want}", b.len()) }
            }
            let s = v.get("m_NavMeshBuildSettings");
            let mut c = v.compact();
            if let Some(i) = c.find("m_NavMeshTiles") { let j = c[i..].find("m_NavMeshBuildSettings").map(|j| i + j).unwrap_or(c.len()); c.replace_range(i..j, "TILES,"); }
            let c: String = c.chars().take(700).collect();
            println!("{n:50} {} agent {} r {} h {} | {}", v.get("m_Name").str(), s.get("agentTypeID").i64(), s.get("agentRadius").f32(), s.get("agentHeight").f32(), { let full = v.compact(); let i = full.rfind("m_OffMeshLinks").unwrap_or(0); let k = full[i..].find("m_SourceBounds").map(|k| i + k).unwrap_or(i); full[k..].chars().take(400).collect::<String>() });
        }}
    }
    println!("tile size formula: {ok} ok, {bad} mismatched");
}
