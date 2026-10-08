//! A material's saved properties and its shader's property names / keywords, and how often the
//! level's other materials on the same shader set each keyword.
//! cargo run --release -p uk-game --example material_props -- <CAB-...> <path_id> [level0-1]
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef, shader::{MaterialProps, ShaderAsset}};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let f = db.file(&a[1]).unwrap();
    let m = MaterialProps::from_value(&f.read_id(a[2].parse().unwrap()).unwrap());
    println!("{} keywords {:?} queue {}", m.name, m.keywords, m.render_queue);
    println!("floats {:?}", m.floats.iter().collect::<BTreeMap<_, _>>());
    println!("colors {:?}", m.colors.iter().collect::<BTreeMap<_, _>>());
    println!("textures {:?}", m.textures.iter().map(|(k, t)| (k, (t.texture, t.scale, t.offset))).collect::<BTreeMap<_, _>>());
    let (sf, sid) = db.resolve(&f, m.shader).unwrap().unwrap();
    let sh = ShaderAsset::from_value(&sf.read_id(sid).unwrap()).unwrap();
    println!("shader {} passes {}", sh.name, sh.passes.len());
    for extra in [&[][..], &["EMISSIVE"][..], &["REFLECTION"][..]] {
        let mut kws: Vec<&str> = m.keywords.iter().map(String::as_str).filter(|k| std::env::var_os("NO_VL").is_none() || *k != "VERTEX_LIGHTING").collect();
        kws.extend_from_slice(extra);
        let pass = &sh.passes[0];
        for (stage, subs) in [("vs", &pass.vertex), ("fs", &pass.fragment)] {
            let Some(sub) = sh.select(subs, &kws) else { continue };
            let p = sh.params(sub.params).unwrap();
            let names: Vec<String> = p.constant_buffers.iter().flat_map(|c| c.params.iter().map(|x| x.name.clone())).filter(|n| !n.starts_with("unity_") && !n.starts_with("glstate")).collect();
            let tex: Vec<&str> = p.bindings.iter().filter(|b| b.kind == 0).map(|b| b.name.as_str()).collect();
            println!("{kws:?} {stage}: cb {names:?} tex {tex:?}");
        }
    }
    let level = a.get(3).cloned().unwrap_or("level0-1".into());
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let mut kw: BTreeMap<String, usize> = BTreeMap::new();
    let mut fl: BTreeMap<String, (usize, f32, f32)> = BTreeMap::new();
    let mut seen = std::collections::HashSet::new();
    for k in def.renderers.iter().filter_map(|r| r.material.as_ref()) {
        if !seen.insert((k.file.clone(), k.path_id)) {
            continue;
        }
        let Ok(mf) = db.file(&k.file) else { continue };
        let Ok(v) = mf.read_id(k.path_id) else { continue };
        let p = MaterialProps::from_value(&v);
        let Ok(Some((s2, id2))) = db.resolve(&mf, p.shader) else { continue };
        if s2.name != sf.name || id2 != sid {
            continue;
        }
        for k in &p.keywords {
            *kw.entry(k.clone()).or_default() += 1;
        }
        if p.keywords.iter().any(|k| k == "REFLECTION") {
            println!("REFLECTION material {} {} {}: cube {:?} mask {:?} strength {:?} colour contribution {:?} lighting contribution {:?} cube mode {:?}", p.name, k.file, k.path_id, p.textures.get("_CubeTex").map(|t| t.texture), p.textures.get("_ReflectionMask").map(|t| t.texture), p.floats.get("_ReflectionStrength"), p.floats.get("_ColorContribution"), p.floats.get("_LightingContribution"), p.floats.get("_CubeMode"));
        }
        for (k, &v) in &p.floats {
            let e = fl.entry(k.clone()).or_insert((0, f32::MAX, f32::MIN));
            e.0 += 1;
            e.1 = e.1.min(v);
            e.2 = e.2.max(v);
        }
    }
    println!("{} materials on this shader in {level}; keywords {kw:?}", seen.len());
    println!("float ranges {fl:?}");
}
