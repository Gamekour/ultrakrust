//! Which GPU platforms are the shaders compiled for? (Unity ShaderCompilerPlatform: 4=D3D11, 15=Metal? 18=Vulkan, ...)
use uk_assets::db::AssetDb;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut plats = std::collections::BTreeMap::<Vec<u8>, usize>::new();
    let mut names = Vec::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path(); if p.is_dir() { continue }
        let Ok(files) = db.load_bundle_files(&p) else { continue };
        for f in files { for o in f.objects.clone() { if o.class_id != 48 { continue }
            let Ok(v) = f.read(&o) else { continue };
            let pl = v.get("platforms").array().iter().map(|x| x.i64() as u8).collect::<Vec<_>>();
            let pl = if pl.is_empty() { v.get("m_ParsedForm").get("m_Platforms").bytes().to_vec() } else { pl };
            *plats.entry(pl).or_default() += 1;
            let nm = v.get("m_ParsedForm").get("m_Name").str().to_string();
            if !nm.is_empty() { names.push(nm) }
        }}
    }
    println!("platform sets: {plats:?}");
    names.sort(); names.dedup();
    println!("{} distinct shader names:", names.len());
    for n in names { println!("  {n}") }
}
