//! Hierarchy of an Addressables prefab by GUID or address: per GameObject its active flag, layer
//! and components (scripts by class name). prefab <guid|address>
use uk_assets::{addressables::Catalog, db::AssetDb};
fn main() {
    let key = std::env::args().nth(1).expect("guid or address");
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let cat = Catalog::load(&install).unwrap();
    println!("catalog {} keys; {key} -> {:?}", cat.len(), cat.locate(&key));
    let (f, id) = cat.load_asset(&mut db, &key).unwrap();
    let go = f.read_id(id).unwrap();
    fn walk(db: &mut AssetDb, f: &std::sync::Arc<uk_assets::serialized::SerializedFile>, go: i64, depth: usize) {
        let v = f.read_id(go).unwrap();
        let mut comps = Vec::new();
        let mut kids = Vec::new();
        for c in v.get("m_Component").array() {
            let (_, cid) = c.get("component").pptr();
            let Some(o) = f.object(cid) else { continue };
            match o.class_id {
                4 | 224 => kids = f.read(o).unwrap().get("m_Children").array().iter().map(|k| k.pptr().1).collect(),
                114 => {
                    let s = f.read(o).unwrap();
                    let n = db.read_pptr(f, s.get("m_Script").pptr()).ok().flatten().map(|x| x.2.get("m_ClassName").str().to_string()).unwrap_or_default();
                    comps.push(n);
                }
                c => comps.push(format!("#{c}")),
            }
        }
        println!("{}{} [{}{}] {:?}", "  ".repeat(depth), v.get("m_Name").str(), if v.get("m_IsActive").bool() { "" } else { "off " }, v.get("m_Layer").i64(), comps);
        for k in kids {
            let t = f.read_id(k).unwrap();
            walk(db, f, t.get("m_GameObject").pptr().1, depth + 1);
        }
    }
    println!("root object class: {:?}", f.object(id).map(|o| o.class_id));
    let _ = go;
    walk(&mut db, &f, id, 0);
}
