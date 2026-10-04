//! Union of constant-buffer member listings across every parameter blob of a shader (Vulkan):
//! (cb name without hash suffix, offset) -> member names seen. Fixed-layout buffers like
//! ULTRAKILL's StandardProperties are only partially listed per variant.
use std::collections::{BTreeMap, BTreeSet};
use uk_assets::db::AssetDb;
use uk_assets::shader::ShaderAsset;
fn main() {
    let want = std::env::args().nth(1).unwrap_or("ULTRAKILL/Master".into());
    let cbf = std::env::args().nth(2).unwrap_or("StandardProperties".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let files = db.load_bundle_files(&path).unwrap();
    for f in &files { for o in f.objects.iter().filter(|o| o.class_id == 23) {
        let Ok(v) = f.read(o) else { continue };
        for m in v.get("m_Materials").array() {
            let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
            let Ok(mv) = mf.read_id(mid) else { continue };
            let Ok(Some((sf, sid))) = db.resolve(&mf, mv.get("m_Shader").pptr()) else { continue };
            let Ok(sv) = sf.read_id(sid) else { continue };
            if sv.get("m_ParsedForm").get("m_Name").str() != want { continue }
            let plat: i64 = std::env::var("PLATFORM").ok().and_then(|s| s.parse().ok()).unwrap_or(18);
            let sh = ShaderAsset::from_value_platform(&sv, plat).unwrap();
            let mut seen: BTreeMap<(u32, String), BTreeSet<String>> = BTreeMap::new();
            let mut sizes = BTreeSet::new();
            let mut n = 0;
            for e in 0..sh.entry_count() as u32 {
                let Ok(p) = sh.params(e) else { continue };
                n += 1;
                for cb in p.constant_buffers.iter().filter(|c| c.name.trim_end_matches(char::is_numeric) == cbf) {
                    sizes.insert(cb.size);
                    for q in &cb.params {
                        seen.entry((q.offset, format!("{}x{}{}", q.rows, q.cols, if q.array_size > 0 { format!("[{}]", q.array_size) } else { String::new() }))).or_default().insert(q.name.clone());
                    }
                }
            }
            println!("{want}: {n} parameter blobs; {cbf} sizes {sizes:?}");
            for ((off, ty), names) in &seen { println!("  @{off:4} {ty:8} {names:?}") }
            return;
        }
    }}
}
