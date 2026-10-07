//! PostProcessV2_Handler's final composite material (`postProcessV2_VSRM`) in a level: its shader's
//! keywords, the material's enabled keywords, and the constant buffers and bindings of the
//! default variant and of each runtime keyword (DEAD, Vignette, WICKED, UNDERWATER, PALETTIZE).
//! post_vsrm [level]   (default 0-1)
use uk_assets::{db::AssetDb, shader::{MaterialProps, ShaderAsset}};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_level{level}.bundle"));
    let def = uk_assets::scenedef::load_scene(&mut db, &path).unwrap();
    let h = def.scripts.iter().find(|s| s.class == "PostProcessV2_Handler").expect("no handler");
    let f = db.file(&def.scene_file).unwrap();
    let (mf, mid) = db.resolve(&f, h.data.get("postProcessV2_VSRM").pptr()).unwrap().expect("material unresolved");
    let props = MaterialProps::from_value(&mf.read_id(mid).unwrap());
    let (sf, sid) = db.resolve(&mf, props.shader).unwrap().unwrap();
    let sh = ShaderAsset::from_value(&sf.read_id(sid).unwrap()).unwrap();
    println!("shader {:?}, {} passes, keywords {:?}", sh.name, sh.passes.len(), sh.keyword_names);
    // Unity: a material value wins only for properties the shader declares; the rest read globals
    let declared: Vec<String> = sf.read_id(sid).unwrap().get("m_ParsedForm").get("m_PropInfo").get("m_Props").array().iter().map(|p| p.get("m_Name").str().to_string()).collect();
    println!("declared properties {declared:?}");
    println!("material keywords {:?}", props.keywords);
    println!("material floats {:?}", props.floats);
    println!("material colors {:?}", props.colors);
    for (n, t) in &props.textures {
        let name = db.resolve(&mf, t.texture).ok().flatten().and_then(|(tf, id)| tf.read_id(id).ok()).map(|v| v.get("m_Name").str().to_string());
        println!("material texture {n}: {name:?}");
    }
    for field in ["vignetteTexture", "ditherTexture", "oilTex", "sandTex", "buffTex"] {
        let name = db.resolve(&f, h.data.get(field).pptr()).ok().flatten().and_then(|(tf, id)| tf.read_id(id).ok()).map(|v| v.get("m_Name").str().to_string());
        println!("handler {field}: {name:?}");
    }
    let base: Vec<&str> = props.keywords.iter().map(|s| s.as_str()).collect();
    for extra in [None, Some("DEAD"), Some("VIGNETTE"), Some("WICKED"), Some("UNDERWATER"), Some("PALETTIZE"), Some("DEAD UNDERWATER VIGNETTE WICKED")] {
        let mut kw = base.clone();
        kw.extend(extra.iter().flat_map(|e| e.split(' ')));
        let Some(sub) = sh.select(&sh.passes[0].vertex, &kw).cloned() else { println!("{extra:?}: no variant"); continue };
        let params = sh.params(sub.params).unwrap();
        println!("{extra:?}: keywords {:?}", sub.keywords.iter().map(|&k| sh.keyword_names.get(k as usize).cloned().unwrap_or_default()).collect::<Vec<_>>());
        for cb in &params.constant_buffers {
            println!("  CB {} size {}: {}", cb.name, cb.size, cb.params.iter().map(|p| format!("{}@{}", p.name, p.offset)).collect::<Vec<_>>().join(" "));
        }
        for b in &params.bindings {
            println!("  binding {} kind {}", b.name, b.kind)
        }
    }
}
