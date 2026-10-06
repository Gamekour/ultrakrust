//! The variant a material would use (shader name + enabled keywords): its constant buffers, bindings,
//! bind channels, and both stages as WGSL written to $OUT (must be outside the repo).
//! OUT=dir variant_wgsl "ULTRAKILL/PostProcessV2" [KW,KW..] [pass]
use uk_assets::{db::AssetDb, shader::ShaderAsset};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let want = a.get(1).cloned().unwrap_or("ULTRAKILL/PostProcessV2".into());
    let kws: Vec<String> = a.get(2).map(|s| s.split(',').filter(|s| !s.is_empty()).map(String::from).collect()).unwrap_or_default();
    let pass_i: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
    let out = std::env::var("OUT").expect("set OUT to a directory outside the repo");
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let mut seen = std::collections::HashSet::new();
    for f in &db.load_bundle_files(&path).unwrap() {
        for o in f.objects.iter().filter(|o| o.class_id == 23) {
            let Ok(v) = f.read(o) else { continue };
            for m in v.get("m_Materials").array() {
                let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
                let Ok(mv) = mf.read_id(mid) else { continue };
                let Ok(Some((sf, sid))) = db.resolve(&mf, mv.get("m_Shader").pptr()) else { continue };
                if !seen.insert((sf.name.clone(), sid)) { continue }
                let Ok(sv) = sf.read_id(sid) else { continue };
                if sv.get("m_ParsedForm").get("m_Name").str() != want { continue }
                let sh = ShaderAsset::from_value(&sv).unwrap();
                println!("{} passes; keyword names {:?}", sh.passes.len(), sh.keyword_names);
                let p = &sh.passes[pass_i];
                println!("pass {pass_i} {:?} tags {:?}", p.name, p.tags);
                let k: Vec<&str> = kws.iter().map(|s| s.as_str()).collect();
                let sub = sh.select(&p.vertex, &k).expect("no variant").clone();
                let prog = sh.program(sub.blob).unwrap();
                let params = sh.params(sub.params).unwrap();
                println!("blob {} keywords {:?} channels {:?}", sub.blob, prog.keywords, prog.channels);
                for cb in &params.constant_buffers {
                    println!("CB {} size {}: {}", cb.name, cb.size, cb.params.iter().map(|p| format!("{}@{}", p.name, p.offset)).collect::<Vec<_>>().join(" "));
                }
                for b in &params.bindings { println!("binding {} kind {} packed {:#x}", b.name, b.kind, b.packed) }
                for (i, st) in prog.stages.iter().enumerate().take(2) {
                    let Some(w) = st else { continue };
                    let w = uk_assets::spirv::prepare(w);
                    let bytes: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
                    let m = naga::front::spv::parse_u8_slice(&bytes, &Default::default()).unwrap();
                    let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap();
                    let src = naga::back::wgsl::write_string(&m, &info, naga::back::wgsl::WriterFlags::empty()).unwrap();
                    let fp = format!("{out}/{}_{}.wgsl", want.replace('/', "_"), ["vs", "fs"][i]);
                    std::fs::write(&fp, src).unwrap();
                    println!("wrote {fp}");
                }
                return;
            }
        }
    }
    println!("shader {want} not found among 0-1 renderers");
}
