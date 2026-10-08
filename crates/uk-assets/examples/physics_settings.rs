//! PhysicsManager (globalgamemanagers, stored without a typetree): the raw object as u32 / f32
//! words, to read query settings such as m_QueriesHitBackfaces by layout.
use uk_assets::db::AssetDb;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let f = db.file("globalgamemanagers").unwrap();
    for o in f.objects.iter().filter(|o| o.class_id == 55) {
        let s = (f.data_offset + o.byte_start) as usize;
        let b = &f.data[s..s + o.byte_size as usize];
        for (i, w) in b.chunks(4).enumerate() {
            let mut a = [0u8; 4];
            a[..w.len()].copy_from_slice(w);
            println!("{:3}: {:02x?} u32 {} f32 {}", i * 4, w, u32::from_le_bytes(a), f32::from_le_bytes(a));
        }
    }
}
