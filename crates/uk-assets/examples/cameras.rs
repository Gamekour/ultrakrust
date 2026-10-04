//! Camera components in a level scene: clear flags, background, clip planes, FOV, culling mask, depth, target.
use uk_assets::db::AssetDb;
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = uk_assets::scenedef::load_scene(&mut db, &path).unwrap();
    for f in db.load_bundle_files(&path).unwrap() { for o in f.objects.iter().filter(|o| o.class_id == 20) {
        let Ok(v) = f.read(o) else { continue };
        let go = v.get("m_GameObject").pptr().1;
        let node = def.obj_to_node.get(&go).map(|&n| def.path(n)).unwrap_or_default();
        let bg = v.get("m_BackGroundColor");
        println!("{node}: clearFlags {} bg ({:.2},{:.2},{:.2}) near {} far {} fov {} depth {} cullingMask {:#x} enabled {} targetTexture {:?}",
            v.get("m_ClearFlags").i64(), bg.get("r").f32(), bg.get("g").f32(), bg.get("b").f32(), v.get("near clip plane").f32(), v.get("far clip plane").f32(),
            v.get("field of view").f32(), v.get("m_Depth").f32(), v.get("m_CullingMask").get("m_Bits").i64(), v.get("m_Enabled").bool(), v.get("m_TargetTexture").pptr());
    }}
}
