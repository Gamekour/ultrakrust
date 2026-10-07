//! Directional lights per level (RenderSettings.m_Sun, active at load, world direction towards the light)
//! for the procedural skybox's sun. suns <level filter>
use uk_assets::db::AssetDb;
fn main() {
    let filt = std::env::args().nth(1).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut levels: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("campaign_scenes_") && n.contains(&filt)).collect();
    levels.sort();
    for l in levels {
        let def = uk_assets::scenedef::load_scene(&mut db, &dir.join(&l)).unwrap();
        let active = |mut n: u32| loop {
            let nd = &def.nodes[n as usize];
            if !nd.active_self { break false }
            match nd.parent { Some(p) => n = p, None => break true }
        };
        let mut sun = String::new();
        for f in db.load_bundle_files(&dir.join(&l)).unwrap() {
            for o in f.objects.iter().filter(|o| o.class_id == 104) {
                let p = f.read(o).unwrap().get("m_Sun").pptr();
                sun = format!("{p:?}");
                if let Ok(Some((lf, _, lv))) = db.read_pptr(&f, p) {
                    let go = lv.get("m_GameObject").pptr();
                    if let Ok(Some((_, _, g))) = db.read_pptr(&lf, go) { sun += &format!(" {:?}", g.get("m_Name").str()); }
                }
            }
        }
        println!("{l}: m_Sun {sun}");
        for li in def.lights.iter().filter(|li| li.kind == 1) {
            let n = &def.nodes[li.node as usize];
            let to_light = -n.world0.transform_vector3(bevy_math::Vec3::NEG_Z).normalize();
            println!("   {:?} enabled {} active {} col {:?} i {} to_light ({:.2},{:.2},{:.2})", n.name, li.enabled, active(li.node), li.color, li.intensity, to_light.x, to_light.y, to_light.z);
        }
    }
}
