//! NewMovement.hurtScreen's Image color (the RGB of PostProcessV2's `_HurtScreenColor`).
//! cargo run --release -p uk-assets --example hurt_screen -- [level]   (default 0-1)
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_level{level}.bundle"))).unwrap();
    let s = def.scripts.iter().find(|s| s.class == "NewMovement").expect("no NewMovement");
    let f = db.file(s.file.as_deref().unwrap_or(&def.scene_file)).unwrap();
    let (imf, id) = db.resolve(&f, s.data.get("hurtScreen").pptr()).unwrap().expect("hurtScreen unresolved");
    let img = imf.read_id(id).unwrap();
    println!("hurtScreen in {} m_Color {}", imf.name, img.get("m_Color").compact());
}
