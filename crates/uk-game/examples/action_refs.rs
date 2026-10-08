//! HudMessage InputActionReferences: the referenced action and its InputActionAsset's bindings.
//! cargo run --release -p uk-game --example action_refs -- [bundle filter]
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let filter = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let b = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).find(|p| p.file_name().unwrap().to_string_lossy().contains(&*filter)).unwrap();
    let def = scenedef::load_scene(&mut db, &b).unwrap();
    let f = db.file(&def.scene_file).unwrap();
    let mut dumped = false;
    for s in def.scripts.iter().filter(|s| s.class == "HudMessage") {
        let r = s.data.get("actionReference").pptr();
        let Ok(Some((rf, _, rv))) = db.read_pptr(&f, r) else { continue };
        println!("{} -> {:?}", def.path(s.node), rv);
        if !dumped {
            dumped = true;
            if let Ok(Some((_, _, a))) = db.read_pptr(&rf, rv.get("m_Asset").pptr()) {
                for m in a.get("m_ActionMaps").array() {
                    println!("map {} {}", m.get("m_Name").str(), m.get("m_Id").str());
                    for x in m.get("m_Actions").array() {
                        println!("  action {:?} {}", x.get("m_Name").str(), x.get("m_Id").str());
                    }
                    for x in m.get("m_Bindings").array() {
                        println!("  bind {:?} path {:?} groups {:?} action {:?} flags {}", x.get("m_Name").str(), x.get("m_Path").str(), x.get("m_Groups").str(), x.get("m_Action").str(), x.get("m_Flags").i64());
                    }
                }
                for c in a.get("m_ControlSchemes").array() {
                    println!("scheme {:?}", c);
                }
            }
        }
    }
}
