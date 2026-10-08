//! Per level: collider components in the scene file vs colliders extracted, and why the rest were dropped.
//! collider_census [level filter]; WARN=1 lists the scene's load warnings
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut ps: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).filter(|p| { let n = p.file_name().unwrap().to_string_lossy(); n.contains("campaign_scenes_") && n.contains(&*filter) }).collect();
    ps.sort();
    for p in ps {
        let files = db.load_bundle_files(&p).unwrap();
        let mut in_file = [0usize; 4]; // box, mesh, sphere, capsule
        let mut terrain = 0;
        for f in &files { for o in &f.objects { match o.class_id { 65 => in_file[0] += 1, 64 => in_file[1] += 1, 135 => in_file[2] += 1, 136 => in_file[3] += 1, 154 => terrain += 1, _ => {} } } }
        let def = scenedef::load_scene(&mut db, &p).unwrap();
        let mut got = [0usize; 4];
        for c in &def.colliders { match c.shape { scenedef::ShapeDef::Box { .. } => got[0] += 1, scenedef::ShapeDef::Mesh(_) => got[1] += 1, scenedef::ShapeDef::Sphere { .. } => got[2] += 1, scenedef::ShapeDef::Capsule { .. } => got[3] += 1 } }
        let n = p.file_name().unwrap().to_string_lossy().replace("campaign_scenes_", "");
        println!("{:16} box {:5}/{:5} mesh {:5}/{:5} sphere {:4}/{:4} capsule {:4}/{:4} terrain {terrain} warnings {}", n.split(".bundle").next().unwrap(), got[0], in_file[0], got[1], in_file[1], got[2], in_file[2], got[3], in_file[3], def.warnings.len());
        if std::env::var("WARN").is_ok() {
            for w in &def.warnings {
                println!("  {w}");
            }
        }
    }
}
