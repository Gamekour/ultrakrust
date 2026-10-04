//! What lighting/shading does a level actually use? Shaders by renderer count (with keywords and the
//! properties they set), lightmap use, Light components, and RenderSettings. Text only.
//! render_survey [level]   (default level0-1)
use std::collections::BTreeMap;
use uk_assets::db::AssetDb;

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let files = db.load_bundle_files(&path).unwrap();
    // shader -> (renderers, materials, keywords, props)
    let mut shaders: BTreeMap<String, (usize, std::collections::BTreeSet<String>, std::collections::BTreeSet<String>, std::collections::BTreeSet<String>)> = BTreeMap::new();
    let (mut lm_used, mut lm_none, mut renderers) = (0, 0, 0);
    let mut lights: BTreeMap<String, usize> = BTreeMap::new();
    let mut light_examples = Vec::new();
    let mut mat_cache: std::collections::HashMap<(String, i64), Option<(String, String, Vec<String>, Vec<String>)>> = std::collections::HashMap::new();
    let mut shader_cache: std::collections::HashMap<(String, i64), String> = std::collections::HashMap::new();
    for f in &files {
        for o in f.objects.clone() {
            match o.class_id {
                23 | 137 => {
                    let Ok(v) = f.read(&o) else { continue };
                    renderers += 1;
                    let li = v.get("m_LightmapIndex").i64();
                    if (0..0xfffe).contains(&li) { lm_used += 1 } else { lm_none += 1 }
                    for m in v.get("m_Materials").array() {
                        let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
                        let info = mat_cache.entry((mf.name.clone(), mid)).or_insert_with(|| {
                            let mv = mf.read_id(mid).ok()?;
                            let sp = mv.get("m_Shader").pptr();
                            let (sf, sid) = db.resolve(&mf, sp).ok().flatten()?;
                            let shader = shader_cache.entry((sf.name.clone(), sid)).or_insert_with(|| {
                                // only the name is needed; Shader objects are large, so read once
                                let n = sf.read_id(sid).map(|sv| sv.get("m_ParsedForm").get("m_Name").str().to_string()).unwrap_or_default();
                                if n.is_empty() { format!("<unnamed {}:{sid}>", sf.name) } else { n }
                            }).clone();
                            let kw = mv.get("m_ValidKeywords").array().iter().chain(mv.get("m_InvalidKeywords").array()).map(|k| k.str().to_string()).collect();
                            let ps = mv.get("m_SavedProperties");
                            let mut props = Vec::new();
                            for (kind, field) in [("tex", "m_TexEnvs"), ("f", "m_Floats"), ("c", "m_Colors")] {
                                for p in ps.get(field).array() {
                                    props.push(format!("{kind}:{}", p.get("first").str()));
                                }
                            }
                            Some((shader, mv.get("m_Name").str().to_string(), kw, props))
                        });
                        let Some((shader, name, kw, props)) = info.clone() else { continue };
                        let e = shaders.entry(shader).or_default();
                        e.0 += 1;
                        e.1.insert(name);
                        e.2.extend(kw);
                        e.3.extend(props);
                    }
                }
                108 => {
                    let Ok(v) = f.read(&o) else { continue };
                    let ty = ["Spot", "Directional", "Point", "Area", "Disc"].get(v.get("m_Type").i64() as usize).copied().unwrap_or("?");
                    let mode = ["Realtime?", "Mixed?", "Baked?"].get(0).copied().unwrap_or("");
                    let _ = mode;
                    let key = format!("{ty} lightmapBake={} shadows={} render={}", v.get("m_Lightmapping").i64(), v.get("m_Shadows").get("m_Type").i64(), v.get("m_RenderMode").i64());
                    *lights.entry(key).or_default() += 1;
                    if light_examples.len() < 4 {
                        let c = v.compact();
                        light_examples.push(c.chars().take(500).collect::<String>());
                    }
                }
                104 | 157 => {
                    let Ok(v) = f.read(&o) else { continue };
                    let c = v.compact();
                    println!("[{}] {}", if o.class_id == 104 { "RenderSettings" } else { "LightmapSettings" }, c.chars().take(900).collect::<String>());
                }
                _ => {}
            }
        }
    }
    println!("\nrenderers {renderers}: lightmapped {lm_used}, not lightmapped {lm_none}");
    println!("\nlights:");
    for (k, n) in &lights { println!("  {n:5}  {k}") }
    for e in &light_examples { println!("  e.g. {e}") }
    println!("\nshaders by renderer-material slots:");
    let mut v: Vec<_> = shaders.into_iter().collect();
    v.sort_by(|a, b| b.1 .0.cmp(&a.1 .0));
    for (s, (n, mats, kw, props)) in v.iter().take(12) {
        println!("  {n:6}  {s}  ({} materials)\n          keywords: {:?}\n          props: {:?}", mats.len(), kw, props);
    }
}
