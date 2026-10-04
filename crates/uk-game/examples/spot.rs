//! Inspect a spot in 0-1: navmesh path between two points, and what blocks a straight walk.
//! spot x y z  x2 y2 z2   (Bevy coordinates; first point = player transform)
use std::sync::Arc;
use bevy_math::Vec3;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let a: Vec<f32> = std::env::args().skip(1).map(|s| s.parse().unwrap()).collect();
    let (p, q) = (Vec3::new(a[0], a[1], a[2]), Vec3::new(a[3], a[4], a[5]));
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let g = uk_game::Game::new(def.clone());
    let nav = g.nav.as_ref().unwrap();
    let feet = p - Vec3::Y * 1.5;
    println!("nearest to feet: {:?}\nnearest to goal: {:?}", nav.nearest(feet, Vec3::new(2.0, 4.0, 2.0)), nav.nearest(q, Vec3::new(2.0, 4.0, 2.0)));
    println!("path: {:?}", nav.find_path(feet, q));
    let mut w = g.world.clone();
    w.owner_enabled.iter_mut().for_each(|e| *e = true);
    let dir = (q - p).with_y(0.0).normalize();
    for h in [-1.4f32, -1.0, -0.5, 0.0, 0.8, 1.6] {
        let o = p + Vec3::Y * h;
        let hit = g.world.raycast(o, dir, 12.0);
        println!("ray at +{h:4.1}: {:?}", hit.map(|r| (r.distance, r.point, r.collider)));
    }
}
