//! cargo run --release -p uk-assets --example dump_level -- level0-1
use std::time::Instant;
use uk_assets::{db::AssetDb, scene};

fn main() {
    let level = std::env::args().nth(1).unwrap_or_else(|| "level0-1".into());
    let install = uk_assets::find_install().expect("ULTRAKILL install not found (set ULTRAKILL_DIR)");
    let t = Instant::now();
    let mut db = AssetDb::open(&install).expect("open db");
    println!("indexed {} bundles in {:?}", db.bundle_count(), t.elapsed());
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let t = Instant::now();
    let files = db.load_bundle_files(&path).expect("bundle");
    for f in &files {
        println!("  {} objects={} types={} externals={}", f.name, f.objects.len(), f.types.len(), f.externals.len());
    }
    let lvl = scene::load_level(&mut db, &path, Default::default()).expect("level");
    println!("extracted in {:?}", t.elapsed());
    let s = &lvl.stats;
    println!("game objects {}  active renderers {} (static batched {})  meshes decoded {}", s.game_objects, s.active_renderers, s.static_batched, s.meshes_decoded);
    println!("mesh colliders {}  box colliders {}  collision tris {}", s.mesh_colliders, s.box_colliders, lvl.collision_tris.len());
    let verts: usize = lvl.batches.values().map(|b| b.positions.len()).sum();
    let tris: usize = lvl.batches.values().map(|b| b.indices.len() / 3).sum();
    println!("render batches {}  verts {verts}  tris {tris}", lvl.batches.len());
    println!("spawn {:?}", lvl.spawn);
    let (mut lo, mut hi) = (bevy_math::Vec3::splat(f32::MAX), bevy_math::Vec3::splat(f32::MIN));
    for t in &lvl.collision_tris { for p in t { lo = lo.min(*p); hi = hi.max(*p); } }
    println!("collision bounds {lo:?} .. {hi:?}");
    println!("skipped {} (first: {:?})", s.skipped.len(), s.skipped.first());
}
