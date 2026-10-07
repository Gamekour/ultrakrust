//! Field layout of the TMP_FontAssets a level's texts use: every top-level key, arrays as len + first element.
//! cargo run --release -p uk-assets --example tmp_font_peek -- level0-1
use uk_assets::{db::AssetDb, scenedef, serialized::Value};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let scene = db.file(&def.scene_file).unwrap();
    let mut seen = std::collections::HashSet::new();
    for s in def.scripts.iter().filter(|s| s.class == "TextMeshProUGUI") {
        let file = match &s.file { Some(f) => db.file(f).unwrap(), None => scene.clone() };
        let Ok(Some((f, id, v))) = db.read_pptr(&file, s.data.get("m_fontAsset").pptr()) else { continue };
        if !seen.insert((f.name.clone(), id)) { continue }
        println!("== {} [{} {}]", v.get("m_Name").str(), f.name, id);
        if let Value::Struct(fields) = &v {
            for (k, x) in fields {
                match x {
                    Value::Array(a) => println!("  {k}: [{}] {}", a.len(), a.first().map(|e| { let c = e.compact(); c[..c.len().min(400)].to_string() }).unwrap_or_default()),
                    _ => { let c = x.compact(); println!("  {k}: {}", &c[..c.len().min(600)]) }
                }
            }
        }
    }
}
