//! Lists transparent materials (render queue > 2500) used by 0-1 renderers, with shader names.
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut seen: BTreeMap<String, (String, i64, usize, String)> = BTreeMap::new();
    let mut done = std::collections::HashSet::new();
    let mut shader_names: std::collections::HashMap<(String, i64), String> = Default::default();
    for r in &def.renderers {
        let Some(k) = &r.material else { continue };
        if !done.insert(k.clone()) {
            continue;
        }
        let Ok(f) = db.file(&k.file) else { continue };
        let Ok(v) = f.read_id(k.path_id) else { continue };
        let q = v.get("m_CustomRenderQueue").i64();
        let sp = v.get("m_Shader").pptr();
        let skey = match db.resolve(&f, sp) { Ok(Some((sf, id))) => (sf.name.clone(), id), _ => continue };
        let shader = match shader_names.get(&skey) {
            Some(n) => n.clone(),
            None => {
                let n = db.read_pptr(&f, sp).ok().flatten().map(|(_, _, s)| s.get("m_ParsedForm").get("m_Name").str().to_string()).unwrap_or_default();
                shader_names.insert(skey, n.clone());
                n
            }
        };
        if q > 2500 || shader.to_lowercase().contains("add") || shader.to_lowercase().contains("transp") {
            let kw = v.get("m_ValidKeywords").array().iter().map(|k| k.str().to_string()).collect::<Vec<_>>().join(",");
            let e = seen.entry(v.get("m_Name").str().to_string()).or_insert((shader, q, 0, kw));
            e.2 += 1;
        }
    }
    for (name, (shader, q, n, kw)) in seen { println!("{n:4}x queue {q} '{name}' shader '{shader}' keywords [{kw}]"); }
}
