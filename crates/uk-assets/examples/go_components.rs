//! Raw components of GameObjects named NAME in a level: class, and for renderers the materials + shader names,
//! for MeshFilter the mesh name.  cargo run --release -p uk-assets --example go_components -- level0-1 Quad
use uk_assets::db::AssetDb;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (level, name) = (a.get(1).cloned().unwrap_or("level0-1".into()), a.get(2).cloned().unwrap_or("Quad".into()));
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    for f in db.load_bundle_files(&path).unwrap() {
        for o in f.objects.iter().filter(|o| o.class_id == 1) {
            let Ok(go) = f.read(o) else { continue };
            if go.get("m_Name").str() != name { continue }
            println!("GO {} layer {} active {}", o.path_id, go.get("m_Layer").i64(), go.get("m_IsActive").bool());
            for c in go.get("m_Component").array() {
                let p = c.get("component").pptr();
                let Ok(Some((cf, id, v))) = db.read_pptr(&f, p) else { continue };
                let class = cf.objects.iter().find(|x| x.path_id == id).map(|x| x.class_id).unwrap_or(-1);
                let mut extra = String::new();
                if class == 23 {
                    extra += &format!(" enabled {}", v.get("m_Enabled").bool());
                    for m in v.get("m_Materials").array() {
                        if let Ok(Some((mf, _, mv))) = db.read_pptr(&cf, m.pptr()) {
                            let sh = db.read_pptr(&mf, mv.get("m_Shader").pptr()).ok().flatten().map(|(_, _, s)| s.get("m_ParsedForm").get("m_Name").str().to_string()).unwrap_or_default();
                            extra += &format!(" mat {:?} shader {sh:?} kw {:?}", mv.get("m_Name").str(), mv.get("m_ValidKeywords").compact());
                        }
                    }
                }
                if class == 33 {
                    if let Ok(Some((_, _, m))) = db.read_pptr(&cf, v.get("m_Mesh").pptr()) { extra += &format!(" mesh {:?}", m.get("m_Name").str()) }
                }
                if class == 114 {
                    if let Ok(Some((_, _, s))) = db.read_pptr(&cf, v.get("m_Script").pptr()) { extra += &format!(" script {:?}", s.get("m_ClassName").str()) }
                }
                println!("  class {class} id {id}{extra}");
            }
        }
    }
}
