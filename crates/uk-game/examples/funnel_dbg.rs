//! For random 0-1 paths, find the first portal the smoothed path does not pass through (xz).
use std::sync::Arc;
use bevy_math::{Vec2, Vec3};
use uk_assets::{db::AssetDb, scenedef};
use uk_game::Game;
fn seg_hit(p: Vec2, q: Vec2, a: Vec2, b: Vec2) -> bool {
    let d = q - p; let e = b - a; let den = d.perp_dot(e);
    if den.abs() < 1e-9 { return false }
    let t = (a - p).perp_dot(e) / den; let u = (a - p).perp_dot(d) / den;
    (-1e-3..=1.0 + 1e-3).contains(&t) && (-1e-3..=1.0 + 1e-3).contains(&u)
}
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let g = Game::new(def.clone());
    let nav = g.nav.as_ref().unwrap();
    let xz = |v: Vec3| Vec2::new(v.x, v.z);
    let mut seed = 0x9e3779b9u32;
    let mut rnd = |n: usize| { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed as usize % n };
    let (mut tried, mut bad, mut shown) = (0, 0, 0);
    for _ in 0..300 {
        let (a, b) = (rnd(nav.polys.len()), rnd(nav.polys.len()));
        let (pa, pb) = (nav.closest_on_poly(a as u32, nav.polys[a].center), nav.closest_on_poly(b as u32, nav.polys[b].center));
        let Some((sp, tgt, portals, _)) = nav.corridor(pa, pb) else { continue };
        let Some((corners, _)) = nav.find_path(pa, pb) else { continue };
        tried += 1;
        let _ = (sp, tgt);
        // every non-offmesh portal must be crossed by some walking segment
        let segs: Vec<(Vec2, Vec2)> = corners.windows(2).filter(|w| !w[1].offmesh).map(|w| (xz(w[0].pos), xz(w[1].pos))).collect();
        let miss = portals.iter().enumerate().find(|(_, l)| l.offmesh.is_none() && !segs.iter().any(|&(p, q)| seg_hit(p, q, xz(l.a), xz(l.b))));
        if let Some((i, l)) = miss {
            bad += 1;
            if shown < 4 {
                shown += 1;
                println!("path {a}->{b}: {} portals, {} corners; misses portal {i} a {:?} b {:?} len {:.2}", portals.len(), corners.len(), l.a, l.b, l.a.distance(l.b));
                for (k, p) in portals.iter().enumerate().take(i + 2).skip(i.saturating_sub(2)) { println!("   portal {k}: a {:?} b {:?} offmesh {:?} to {}", p.a, p.b, p.offmesh, p.to) }
                for c in &corners { println!("   corner {:?}", c) }
            }
        }
    }
    println!("paths missing a portal: {bad}/{tried}");
}
