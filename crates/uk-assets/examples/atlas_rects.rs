//! Glyph rects and serialized free / used rects of a font asset (sharedassets0.assets path id).
use uk_assets::db::AssetDb;
fn main() {
    let id: i64 = std::env::args().nth(1).map_or(88, |a| a.parse().unwrap());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let b = std::fs::read_dir(AssetDb::bundle_dir(&install)).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().contains("campaign_scenes_level0-1")).unwrap();
    let _ = uk_assets::scenedef::load_scene(&mut db, &b).unwrap();
    let f = db.file("sharedassets0.assets").unwrap();
    let v = f.read_id(id).unwrap();
    let r4 = |r: &uk_assets::serialized::Value| [r.get("m_X").i64(), r.get("m_Y").i64(), r.get("m_Width").i64(), r.get("m_Height").i64()];
    let mut g: Vec<[i64; 4]> = v.get("m_GlyphTable").array().iter().map(|g| r4(g.get("m_GlyphRect"))).collect();
    g.sort_by_key(|r| (r[1], r[0]));
    println!("multi atlas {} glyph rects (lowest) {:?}", v.get("m_IsMultiAtlasTexturesEnabled").i64(), &g[..g.len().min(6)]);
    println!("used {:?}", v.get("m_UsedGlyphRects").array().iter().take(6).map(r4).collect::<Vec<_>>());
    println!("free {:?}", v.get("m_FreeGlyphRects").array().iter().take(6).map(r4).collect::<Vec<_>>());
}
