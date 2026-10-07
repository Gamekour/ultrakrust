//! Compact dump of the assets a level's uGUI graphics reference: the first Sprite / Font / TMP_FontAsset /
//! Material named NAME. cargo run --release -p uk-assets --example ui_asset_peek -- level0-1 Image m_Sprite [maxlen]
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let level = a.get(1).cloned().unwrap_or("level0-1".into());
    let class = a.get(2).cloned().unwrap_or("Image".into());
    let field = a.get(3).cloned().unwrap_or("m_Sprite".into());
    let maxlen: usize = a.get(4).and_then(|s| s.parse().ok()).unwrap_or(3000);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let scene = db.file(&def.scene_file).unwrap();
    let mut seen = std::collections::HashSet::new();
    for s in def.scripts.iter().filter(|s| s.class == class) {
        let file = match &s.file { Some(f) => db.file(f).unwrap(), None => scene.clone() };
        let Ok(Some((f, id, v))) = db.read_pptr(&file, field.split('.').fold(&s.data, |v, k| v.get(k)).pptr()) else { continue };
        if !seen.insert((f.name.clone(), id)) { continue }
        let c = v.compact().chars().filter(|_| true).collect::<String>();
        let c = if let Some(k) = a.get(5) { c.split(',').filter(|p| !p.starts_with(k.as_str()) ).collect::<Vec<_>>().join(",") } else { c };
        println!("== {} [{} {}] {}", v.get("m_Name").str(), f.name, id, &c[..c.len().min(maxlen)]);
        if seen.len() >= 3 { break }
    }
}
