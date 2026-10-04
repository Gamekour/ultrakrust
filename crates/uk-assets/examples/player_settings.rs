//! Project-wide render settings from globalgamemanagers: color space, quality, graphics APIs.
use uk_assets::db::AssetDb;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let f = db.file("globalgamemanagers").unwrap();
    println!("{} objects, classes {:?}", f.objects.len(), f.objects.iter().map(|o| o.class_id).collect::<std::collections::BTreeSet<_>>());
    for o in f.objects.clone() {
        let v = match f.read(&o) { Ok(v) => v, Err(e) => { if [129, 47, 30].contains(&o.class_id) { println!("class {}: {}", o.class_id, e) } continue } };
        match o.class_id {
            129 => println!("PlayerSettings: activeColorSpace {} (0 gamma, 1 linear); m_StereoRenderingPath {}", v.get("m_ActiveColorSpace").i64(), v.get("m_StereoRenderingPath").i64()),
            47 => { let c = v.compact(); println!("QualitySettings: {}", c.chars().take(600).collect::<String>()) }
            30 => { let c = v.compact(); println!("GraphicsSettings: {}", c.chars().take(300).collect::<String>()) }
            _ => {}
        }
    }
}
