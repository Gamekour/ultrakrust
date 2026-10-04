//! Integration test on the user's own install: drop V1 into level 0-1's real
//! collision and check it lands, walks, and never falls through the floor.
//! Skipped when ULTRAKILL isn't installed.

use bevy_math::{Vec2, Vec3};
use uk_assets::{db::AssetDb, scene};
use uk_core::collide::{BoxCollider, World};
use uk_core::consts::FIXED_DT;
use uk_core::player::{Input, Player};

fn load() -> Option<(World, Vec3, f32)> {
    let install = uk_assets::find_install()?;
    let mut db = AssetDb::open(&install).ok()?;
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let lvl = scene::load_level(&mut db, &path, Default::default()).ok()?;
    let mut w = World::default();
    for t in &lvl.collision_tris {
        w.add_triangle(t[0], t[1], t[2]);
    }
    for b in &lvl.boxes {
        w.add(BoxCollider { center: b.center, half: b.half, rot: b.rot, slippery: false });
    }
    w.build();
    let (spawn, yaw) = lvl.spawn?;
    Some((w, spawn, yaw))
}

#[test]
fn v1_lands_and_walks_in_0_1() {
    let Some((world, spawn, yaw)) = load() else {
        eprintln!("ULTRAKILL not installed; skipping");
        return;
    };
    let mut p = Player::new(spawn);
    p.yaw_deg = yaw;
    let mut t = 0.0f64;
    let mut frame = |p: &mut Player, input: Input| {
        p.fixed_update(&world, &input);
        t += FIXED_DT as f64;
        p.update(&world, &input, FIXED_DT, t);
        p.events.clear();
    };
    // The level starts with V1 dropping into the first room.
    let start = std::time::Instant::now();
    let mut landed_at = None;
    for i in 0..2000 {
        frame(&mut p, Input::default());
        if p.gc.on_ground {
            landed_at = Some(i);
            break;
        }
    }
    let landed_at = landed_at.expect("V1 should land in the first room");
    let floor_y = p.pos.y;
    println!("landed after {landed_at} ticks at {:?} (fell {:.1})", p.pos, spawn.y - floor_y);
    // Walk forward for 3 seconds: should stay on the ground (no tunnelling).
    let walk = Input { move_axis: Vec2::Y, ..Default::default() };
    let mut min_y = f32::MAX;
    for _ in 0..375 {
        frame(&mut p, walk);
        min_y = min_y.min(p.pos.y);
    }
    let moved = (p.pos - Vec3::new(spawn.x, p.pos.y, spawn.z)).length();
    println!("walked to {:?}, horizontal distance {moved:.1}, min y {min_y:.2}; {:?} total", p.pos, start.elapsed());
    assert!(min_y > floor_y - 5.0, "fell through the floor");
    let ahead = world.raycast(p.pos, p.forward(), 50.0);
    println!("on_wall {}  wall ahead {:?}", p.wall.on_wall, ahead.map(|h| (h.distance, h.collider)));
    assert!(moved > 2.0, "should have walked somewhere");
    // Then turn around and walk the other way for 3 s: still grounded.
    p.yaw_deg += 180.0;
    for _ in 0..375 {
        frame(&mut p, walk);
        min_y = min_y.min(p.pos.y);
    }
    println!("back to {:?}", p.pos);
    assert!(min_y > floor_y - 5.0, "fell through the floor");
}
