//! RenderSettings.m_SkyboxMaterial per level: shader name, texture/color/float properties; plus
//! scene cameras' clear flags. skybox [level filter]
use uk_assets::db::AssetDb;
fn main() {
    let filt = std::env::args().nth(1).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let mut levels: Vec<_> = std::fs::read_dir(AssetDb::bundle_dir(&install)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("campaign_scenes_") && n.contains(&filt)).collect();
    levels.sort();
    for l in levels {
        let path = AssetDb::bundle_dir(&install).join(&l);
        let files = db.load_bundle_files(&path).unwrap();
        for f in &files {
            for o in f.objects.iter().filter(|o| o.class_id == 104) {
                let v = f.read(o).unwrap();
                let p = v.get("m_SkyboxMaterial").pptr();
                let mut out = format!("{l}: fog {} skybox pptr {p:?}", v.get("m_Fog").bool());
                if let Ok(Some((mf, _, m))) = db.read_pptr(f, p) {
                    let sh = m.get("m_Shader").pptr();
                    let shname = db.read_pptr(&mf, sh).ok().flatten().map(|(_, _, s)| s.get("m_ParsedForm").get("m_Name").str().to_string()).unwrap_or_default();
                    let props = uk_assets::shader::MaterialProps::from_value(&m);
                    for (tn, env) in props.textures.iter().filter(|t| t.1.texture.1 != 0) {
                        if let Ok(Some((_, _, t))) = db.read_pptr(&mf, env.texture) {
                            let dec = uk_assets::texture::decode_texture(&mut db, &t).map(|d| { let n = (d.rgba.len() / 4).max(1) as f64; let m = |c: usize| d.rgba.chunks(4).map(|p| p[c] as f64).sum::<f64>() / n; let distinct = d.rgba.chunks(4).map(|p| (p[0], p[1], p[2])).collect::<std::collections::HashSet<_>>().len(); (d.layers, format!("mean rgb ({:.1},{:.1},{:.1}) distinct {distinct}", m(0), m(1), m(2))) }).map_err(|e| e.0);
                            out += &format!("
   {tn} {:?} fmt {} {}x{} images {} decode {dec:?}", t.get("m_Name").str(), t.get("m_TextureFormat").i64(), t.get("m_Width").i64(), t.get("m_Height").i64(), t.get("m_ImageCount").i64());
                        }
                    }
                    out += &format!(" file {} id {}", mf.name, p.1);
                    out += &format!(" mat {:?} shader {shname:?} kw {:?}\n   tex {:?}\n   col {:?}\n   flt {:?}", m.get("m_Name").str(), props.keywords, props.textures.iter().filter(|t| t.1.texture.1 != 0).map(|t| t.0).collect::<Vec<_>>(), props.colors, props.floats);
                }
                println!("{out}");
            }
            for o in f.objects.iter().filter(|o| o.class_id == 20) {
                let v = f.read(o).unwrap();
                let mask = v.get("m_CullingMask").get("m_Bits").i64();
                if v.get("m_ClearFlags").i64() != 2 && v.get("m_Enabled").bool() { let bg = v.get("m_BackGroundColor"); println!("   camera clearFlags {} mask {mask:#x} bg ({:.2},{:.2},{:.2})", v.get("m_ClearFlags").i64(), bg.get("r").f32(), bg.get("g").f32(), bg.get("b").f32()); }
            }
        }
    }
}
