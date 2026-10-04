//! Distribution of m_GpuProgramType in a shader's player subprogram lists.
use uk_assets::db::AssetDb;
use uk_assets::shader::ShaderAsset;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let files = db.load_bundle_files(&path).unwrap();
    let mut done = std::collections::HashSet::new();
    for f in &files { for o in f.objects.iter().filter(|o| o.class_id == 23) {
        let Ok(v) = f.read(o) else { continue };
        for m in v.get("m_Materials").array() {
            let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
            let Ok(mv) = mf.read_id(mid) else { continue };
            let Ok(Some((sf, sid))) = db.resolve(&mf, mv.get("m_Shader").pptr()) else { continue };
            if !done.insert((sf.name.clone(), sid)) { continue }
            let Ok(sv) = sf.read_id(sid) else { continue };
            let Ok(sh) = ShaderAsset::from_value(&sv) else { continue };
            let Some(p) = sh.passes.first() else { continue };
            let mut c = std::collections::BTreeMap::<u32, usize>::new();
            for s in &p.vertex { *c.entry(s.gpu_type).or_default() += 1 }
            println!("{:40} vertex gpu types {c:?}", sh.name);
        }
    }}
}
