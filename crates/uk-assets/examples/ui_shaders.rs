//! The shaders a level's UI materials use: passes, raw stencil state, keywords, and the first
//! SPIR-V variant's constant buffers.
//! cargo run --release -p uk-assets --example ui_shaders -- level0-1
use uk_assets::{db::AssetDb, scenedef, shader::ShaderAsset, ui};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let a = ui::load_ui_assets(&mut db, &def);
    let mut seen = std::collections::HashSet::new();
    for m in &a.materials {
        if !seen.insert(m.shader.clone()) { continue }
        let f = db.file(&m.shader.0).unwrap();
        let v = f.read_id(m.shader.1).unwrap();
        let sh = ShaderAsset::from_value(&v).unwrap();
        println!("== {} ({:?}) for {}", sh.name, m.shader, m.name);
        println!("keywords {:?}", sh.keyword_names);
        let pf = v.get("m_ParsedForm");
        for (i, p) in pf.get("m_SubShaders").array()[0].get("m_Passes").array().iter().enumerate() {
            let st = p.get("m_State");
            for k in ["stencilRef", "stencilReadMask", "stencilWriteMask", "stencilOp", "stencilOpFront", "stencilOpBack", "zTest", "zWrite", "culling", "rtBlend0", "alphaToMask", "offsetFactor"] {
                println!("  pass {i} {k}: {}", st.get(k).compact().chars().take(400).collect::<String>());
            }
            println!("  pass {i} name {:?} tags {:?}", sh.passes[i].name, sh.passes[i].tags);
            let vs: Vec<_> = sh.passes[i].vertex.iter().filter(|s| s.gpu_type == 25).collect();
            println!("  vertex variants {}: {:?}", vs.len(), vs.iter().map(|s| s.keywords.iter().map(|k| sh.keyword_names[*k as usize].as_str()).collect::<Vec<_>>()).collect::<Vec<_>>());
            if let Some(s) = sh.select(&sh.passes[i].vertex, &[]) {
                let prm = sh.params(s.params).unwrap();
                for cb in &prm.constant_buffers { println!("  CB {} size {}: {}", cb.name, cb.size, cb.params.iter().map(|p| format!("{}@{}", p.name, p.offset)).collect::<Vec<_>>().join(" ")); }
                for b in &prm.bindings { println!("  bind {} kind {} {}", b.name, b.kind, b.packed); }
            }
            if let Some(s) = sh.select(&sh.passes[i].fragment, &[]) {
                let prm = sh.params(s.params).unwrap();
                for cb in &prm.constant_buffers { println!("  FCB {} size {}: {}", cb.name, cb.size, cb.params.iter().map(|p| format!("{}@{}", p.name, p.offset)).collect::<Vec<_>>().join(" ")); }
                for b in &prm.bindings { println!("  fbind {} kind {} {}", b.name, b.kind, b.packed); }
            }
        }
        let mut fl: Vec<_> = m.props.floats.iter().collect(); fl.sort_by(|a, b| a.0.cmp(b.0));
        println!("  mat floats {:?} colors {:?}", fl, m.props.colors);
    }
}
