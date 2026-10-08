//! TagManager (globalgamemanagers, stored without a typetree): custom tag ids (20000 + index) and
//! layer names, read from the raw object (vector<string> tags, then vector<string> layers).
use uk_assets::db::AssetDb;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let f = db.file("globalgamemanagers").unwrap();
    for o in f.objects.iter().filter(|o| o.class_id == 78) {
        let s = (f.data_offset + o.byte_start) as usize;
        let b = &f.data[s..s + o.byte_size as usize];
        let mut p = 0usize;
        let u32_at = |p: &mut usize| {
            let v = u32::from_le_bytes(b[*p..*p + 4].try_into().unwrap());
            *p += 4;
            v as usize
        };
        for (what, base) in [("tag", 20000), ("layer", 0)] {
            let n = u32_at(&mut p);
            for i in 0..n {
                let len = u32_at(&mut p);
                let t = String::from_utf8_lossy(&b[p..p + len]).to_string();
                p = (p + len + 3) & !3;
                if !t.is_empty() {
                    println!("{what} {} {t}", base + i);
                }
            }
        }
    }
}
