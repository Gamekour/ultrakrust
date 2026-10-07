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
    println!("material keywords {:?}", props.keywords);
    let base: Vec<&str> = props.keywords.iter().map(|s| s.as_str()).collect();
    for extra in [None, Some("DEAD"), Some("VIGNETTE"), Some("WICKED"), Some("UNDERWATER"), Some("PALETTIZE")] {
        let mut kw = base.clone();
        kw.extend(extra);
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
