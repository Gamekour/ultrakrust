//! Navmesh diagnostics for one level: load timing, polygons off geometry, and one bad path in detail.
use std::sync::Arc;
use bevy_math::Vec3;
use uk_assets::{db::AssetDb, navmesh, scenedef};
use uk_game::Game;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let level = std::env::var("LEVEL").unwrap_or("level0-1".into());
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let t = std::time::Instant::now();
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    println!("scene load {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let n = navmesh::load_scene_navmeshes(&mut db, &path).unwrap();
    println!("navmesh reload {:?}: {} meshes", t.elapsed(), n.len());
    let g = Game::new(def.clone());
    let nav = g.nav.as_ref().unwrap();
    let mut w = g.world.clone();
    w.owner_enabled.iter_mut().for_each(|e| *e = true);
    let mut off = 0;
    for (i, p) in nav.polys.iter().enumerate() {
        let s = nav.closest_on_poly(i as u32, p.center);
        let hit = w.raycast(s + Vec3::Y * 1.0, Vec3::NEG_Y, 2.0);
        if hit.is_none() {
            off += 1;
            if off <= 8 {
                let below = w.raycast(s + Vec3::Y * 50.0, Vec3::NEG_Y, 200.0).map(|h| h.point.y);
                println!("off: poly {i} surface {s:?} nearest floor below/above at y {:?}", below);
            }
        }
    }
    println!("{off}/{} polys off geometry", nav.polys.len());
    let mut seed = 0x9e3779b9u32;
    let mut rnd = |n: usize| { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; seed as usize % n };
    let mut shown = 0;
    let (mut tried, mut badp) = (0, 0);
    for _ in 0..300 {
        let (a, b) = (rnd(nav.polys.len()), rnd(nav.polys.len()));
        let (pa, pb) = (nav.closest_on_poly(a as u32, nav.polys[a].center), nav.closest_on_poly(b as u32, nav.polys[b].center));
        let Some((path, ok)) = nav.find_path(pa, pb) else { continue };
        tried += 1;
        let mut this_bad = false;
        for wseg in path.windows(2) { for k in 1..4 {
            if wseg[1].offmesh { continue }
            let q = wseg[0].pos.lerp(wseg[1].pos, k as f32 / 4.0);
            let s = nav.nearest(q, Vec3::new(0.3, 12.0, 0.3)).map(|x| x.1);
            let bad = s.is_none_or(|c| (c - q).with_y(0.0).length() >= 0.25);
            this_bad |= bad;
            if bad && shown < 6 { shown += 1; println!("bad: reached {ok} corners {} seg {:?}->{:?} q {q:?} sample {s:?}", path.len(), wseg[0], wseg[1]); }
        }}
        badp += this_bad as usize;
    }
    println!("bad paths: {badp}/{tried}");
}

#[allow(dead_code)]
fn unused() {}
