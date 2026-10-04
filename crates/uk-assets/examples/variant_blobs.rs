//! For a Vulkan vertex blob index of ULTRAKILL/Master: its keywords, and the fragment subprograms
//! (Vulkan) with the same keywords, with their blob/params indices.
use uk_assets::db::AssetDb;
use uk_assets::shader::{ShaderAsset, GPU_PROGRAM_SPIRV};
fn main() {
    let want: u32 = std::env::args().nth(1).unwrap().parse().unwrap();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let files = db.load_bundle_files(&path).unwrap();
    for f in &files { for o in f.objects.iter().filter(|o| o.class_id == 23) {
        let Ok(v) = f.read(o) else { continue };
        for m in v.get("m_Materials").array() {
            let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
            let Ok(mv) = mf.read_id(mid) else { continue };
            let Ok(Some((sf, sid))) = db.resolve(&mf, mv.get("m_Shader").pptr()) else { continue };
            let Ok(sv) = sf.read_id(sid) else { continue };
            if sv.get("m_ParsedForm").get("m_Name").str() != "ULTRAKILL/Master" { continue }
            let sh = ShaderAsset::from_value(&sv).unwrap();
            let p = &sh.passes[0];
            let Some(vs) = p.vertex.iter().find(|s| s.gpu_type == GPU_PROGRAM_SPIRV && s.blob == want) else { continue };
            let names = |k: &[u16]| k.iter().map(|&i| sh.keyword_names[i as usize].clone()).collect::<Vec<_>>();
            println!("vertex blob {} params {} keywords {:?}", vs.blob, vs.params, names(&vs.keywords));
            let mut c = std::collections::BTreeMap::<u32, usize>::new();
            for s in &p.fragment { *c.entry(s.gpu_type).or_default() += 1 }
            println!("fragment gpu types {c:?}; first entries: {:?}", p.fragment.iter().take(3).map(|s| (s.blob, s.params, s.gpu_type, names(&s.keywords))).collect::<Vec<_>>());
            for fs in p.fragment.iter() {
                let fk = names(&fs.keywords);
                if fk.iter().all(|k| names(&vs.keywords).contains(k)) {
                    println!("  fragment blob {} params {} keywords {:?}", fs.blob, fs.params, fk);
                }
            }
            return;
        }
    }}
}
