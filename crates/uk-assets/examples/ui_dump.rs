//! Raw components (compact values) of GameObjects named NAME in a level, for the uGUI port.
//! cargo run --release -p uk-assets --example ui_dump -- level0-1 GunCanvas [maxlen]
use uk_assets::db::AssetDb;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (level, name) = (a.get(1).cloned().unwrap_or("level0-1".into()), a.get(2).cloned().unwrap_or("Canvas".into()));
    let maxlen: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(600);
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
                let mut script = String::new();
                if class == 114 {
                    if let Ok(Some((_, _, s))) = db.read_pptr(&cf, v.get("m_Script").pptr()) { script = s.get("m_ClassName").str().to_string() }
                }
                let s = format!("{:?}", v.compact());
                println!("  class {class} {script} id {id}: {}", &s[..s.len().min(maxlen)]);
            }
        }
    }
}
