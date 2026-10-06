//! Serialized fields of every script of CLASS in a level (PPtrs shown as the node path they hit).
//! cargo run --release -p uk-assets --example script_fields -- level0-1 HookArm
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{}.bundle", a[1]))).unwrap();
    for s in def.scripts.iter().filter(|s| s.class == a[2]) {
        println!("{} on {} (file {:?}, enabled {})", s.class, def.path(s.node), s.file, s.enabled);
        let text = format!("{:?}", s.data);
        println!("  {}", &text[..text.len().min(3000)]);
    }
}
