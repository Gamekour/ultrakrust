//! Materials of the renderers whose scene path contains a substring: path, material name, key.
//! With `--names <file>`: every Material in that serialized file whose name contains the substring, with
//! its keywords.
//! cargo run --release -p uk-game --example find_material -- <level0-1> <substring> [--names CAB-...]
use uk_assets::{db::AssetDb, scenedef, texture};
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{}.bundle", a[1]))).unwrap();
    if let Some(i) = a.iter().position(|x| x == "--names") {
        let f = db.file(&a[i + 1]).unwrap();
        for o in f.objects.iter().filter(|o| o.class_id == 21) {
            let Ok(v) = f.read(o) else { continue };
            let n = v.get("m_Name").str().to_string();
            if n.to_lowercase().contains(&a[2].to_lowercase()) {
                let m = uk_assets::shader::MaterialProps::from_value(&v);
                println!("{n} | {} {} | keywords {:?} _Fog {:?}", a[i + 1], o.path_id, m.keywords, m.floats.get("_Fog"));
            }
        }
        return;
    }
    let mut seen = std::collections::BTreeSet::new();
    for r in &def.renderers {
        let path = def.path(r.node);
        if !path.to_lowercase().contains(&a[2].to_lowercase()) {
            continue;
        }
        let Some(k) = &r.material else { continue };
        let name = db.file(&k.file).ok().and_then(|f| f.read_id(k.path_id).ok().and_then(|v| texture::decode_material(&mut db, &f, &v).ok())).map(|m| m.name).unwrap_or_default();
        if seen.insert((name.clone(), path.clone())) {
            let b = &r.batch;
            let up = b.normals.iter().filter(|n| n[1] > 0.9).count();
            let ys = b.positions.iter().map(|p| p[1]);
            let (lo, hi) = ys.fold((f32::MAX, f32::MIN), |(l, h), y| (l.min(y), h.max(y)));
            println!("{path} | {name} | {} {} | {} verts, {up} facing up, y {lo:.2}..{hi:.2}", k.file, k.path_id, b.positions.len());
        }
    }
}
