//! PostProcessV2_Handler's outline shader (`outlinePx`) in a level: passes, render state (every
//! rtBlend), constant buffers, bindings, and each pass's stages as WGSL in $OUT (outside the repo).
//! Also the Master shader's per-target blend state, which decides what SV_Target1/2 receive.
//! OUT=dir outline_px [level]   (default 0-1)
use uk_assets::{db::AssetDb, shader::ShaderAsset};

fn blends(st: &uk_assets::serialized::Value) -> String {
    let mut s = format!("separate {} ", st.get("rtSeparateBlend").compact());
    for i in 0..3 {
        let b = st.get(&format!("rtBlend{i}"));
        s += &format!("rt{i} [src {} dst {} op {} mask {}] ", b.get("srcBlend").compact(), b.get("destBlend").compact(), b.get("blendOp").compact(), b.get("colMask").compact());
    }
    s
}

fn main() {
    let level = std::env::args().nth(1).unwrap_or("0-1".into());
    let out = std::env::var("OUT").expect("set OUT to a directory outside the repo");
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_level{level}.bundle"));
    let def = uk_assets::scenedef::load_scene(&mut db, &path).unwrap();
    let h = def.scripts.iter().find(|s| s.class == "PostProcessV2_Handler").expect("no handler");
    let f = db.file(&def.scene_file).unwrap();
    for field in ["outlinePx", "outlinePx_SimpleTest", "heatWaveMat", "screenNormal"] {
        println!("handler.{field} = {}", h.data.get(field).compact());
    }
    let (sf, sid) = db.resolve(&f, h.data.get("outlinePx").pptr()).unwrap().expect("outlinePx unresolved");
    let sv = sf.read_id(sid).unwrap();
    let sh = ShaderAsset::from_value(&sv).unwrap();
    println!("outlinePx = {:?}, {} passes, keywords {:?}", sh.name, sh.passes.len(), sh.keyword_names);
    let pf = sv.get("m_ParsedForm");
    let raw = pf.get("m_SubShaders").array();
    for (i, p) in sh.passes.iter().enumerate() {
        let st = raw[0].get("m_Passes").array()[i].get("m_State").clone();
        println!("pass {i} {:?} cull {:?} zwrite {:?} ztest {:?} {}", p.name, p.state.cull, p.state.zwrite, p.state.ztest, blends(&st));
        let Some(sub) = sh.select(&p.vertex, &[]).cloned() else { println!("  no vulkan variant"); continue };
        let prog = sh.program(sub.blob).unwrap();
        let params = sh.params(sub.params).unwrap();
        for cb in &params.constant_buffers {
            println!("  CB {} size {}: {}", cb.name, cb.size, cb.params.iter().map(|p| format!("{}@{}", p.name, p.offset)).collect::<Vec<_>>().join(" "));
        }
        for b in &params.bindings {
            println!("  binding {} kind {} packed {:#x}", b.name, b.kind, b.packed)
        }
        for (k, stg) in prog.stages.iter().enumerate().take(2) {
            let Some(w) = stg else { continue };
            let w = uk_assets::spirv::prepare(w);
            let bytes: Vec<u8> = w.iter().flat_map(|x| x.to_le_bytes()).collect();
            let m = naga::front::spv::parse_u8_slice(&bytes, &Default::default()).unwrap();
            let info = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap();
            let src = naga::back::wgsl::write_string(&m, &info, naga::back::wgsl::WriterFlags::empty()).unwrap();
            let fp = format!("{out}/outlinePx_p{i}_{}.wgsl", ["vs", "fs"][k]);
            std::fs::write(&fp, src).unwrap();
        }
    }
    // the Master shader's per-target blending, from the level's renderers
    let mut seen = std::collections::HashSet::new();
    for o in f.objects.iter().filter(|o| o.class_id == 23 || o.class_id == 137) {
        let Ok(v) = f.read(o) else { continue };
        for m in v.get("m_Materials").array() {
            let Ok(Some((mf, mid))) = db.resolve(&f, m.pptr()) else { continue };
            let Ok(mv) = mf.read_id(mid) else { continue };
            let Ok(Some((sf, sid))) = db.resolve(&mf, mv.get("m_Shader").pptr()) else { continue };
            if !seen.insert((sf.name.clone(), sid)) {
                continue;
            }
            let Ok(sv) = sf.read_id(sid) else { continue };
            let name = sv.get("m_ParsedForm").get("m_Name").str().to_string();
            for (i, p) in sv.get("m_ParsedForm").get("m_SubShaders").array().first().map(|s| s.get("m_Passes").array().to_vec()).unwrap_or_default().iter().enumerate() {
                let st = p.get("m_State");
                println!("{name} pass {i} {:?}: {}", st.get("m_Name").str(), blends(st));
            }
        }
    }
}
