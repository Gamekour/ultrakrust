//! Spot-check formats that decide architecture: AudioClip compression, Shader platforms, NavMeshData layout, VideoClip.
use uk_assets::db::AssetDb;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let want = |n: &str| n.contains("music_assets_layer0") || n.contains("level0-1") || n.contains("other_assets") || n.contains("shader_unity") || n.contains("intro");
    let mut seen = std::collections::HashMap::<i32, usize>::new();
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path(); let n = p.file_name().unwrap().to_string_lossy().to_string();
        if !want(&n) { continue }
        let Ok(files) = db.load_bundle_files(&p) else { continue };
        for f in files { for o in f.objects.clone() {
            if ![83, 48, 238, 329].contains(&o.class_id) { continue }
            let c = seen.entry(o.class_id).or_default(); if *c >= 2 { continue } *c += 1;
            let Ok(v) = f.read(&o) else { println!("{n}: class {} unreadable", o.class_id); continue };
            let s = v.compact(); println!("[{n}] class {}: {}", o.class_id, &s[..s.len().min(900)]);
        }}
    }
}
