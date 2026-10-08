//! SceneHelper.TryGetSurfaceData against the footstep physics scene: rays down from the start
//! and along a line through the level, and up from below the floor (single-sided: no hit), with
//! the world collider hit for comparison.
//! cargo run --release -p uk-game --example surface_probe -- [level0-1]
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::particles::to_unity;
use uk_game::Game;

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let mut g = Game::new(def.clone());
    let mut t = 0.0f64;
    for _ in 0..200 {
        g.fixed_update(&Input::default());
        t += FIXED_DT as f64;
        g.update(&Input::default(), FIXED_DT, t);
    }
    let p0 = g.s.player.pos;
    println!("player (unity) {:.2?}", to_unity(p0));
    let mut hits = 0;
    for k in -6..=6 {
        for side in [-4.0f32, 0.0, 4.0] {
            let o = p0 + Vec3::new(side, 1.0, k as f32 * 6.0);
            let s = g.surface_data(o, -Vec3::Y, 100.0);
            let w = g.world.raycast(o, -Vec3::Y, 100.0);
            hits += s.is_some() as u32;
            println!(
                "down from (unity) {:.1?}: surface {} | world {}",
                to_unity(o),
                s.map_or("none".into(), |h| format!("{} color {:.2?} blend2 {} at y {:.2} n {:.2?} node {}", h.surface, h.color, h.secondary_blend, h.point.y, h.normal, def.path(h.node))),
                w.map_or("none".into(), |h| format!("y {:.2}", h.point.y))
            );
            if let (None, Some(w)) = (s, w) {
                let o = g.world.owner(w.collider);
                if o != uk_core::collide::ALWAYS {
                    let c = &def.colliders[o as usize];
                    let sm = def.surface_meshes.iter().find(|m| m.node == c.node);
                    println!(
                        "    world hit {} layer {} trigger {} | in footstep scene {} tris {} without material {}",
                        def.path(c.node),
                        c.layer,
                        c.trigger,
                        sm.is_some(),
                        sm.map_or(0, |m| m.tris.len()),
                        sm.map_or(0, |m| m.tri_mat.iter().filter(|x| x.is_none()).count())
                    );
                    if let Some(m) = sm {
                        println!("    materials {:?}", m.mats);
                        for r in def.renderers.iter().filter(|r| r.node == c.node) {
                            println!("    renderer material {:?} tris {}", r.material, r.batch.indices.len() / 3);
                        }
                    }
                }
            }
            if let Some(h) = s {
                let up = g.surface_data(h.point - Vec3::Y * 0.5, Vec3::Y, 0.4);
                if up.is_some() {
                    println!("    BACKFACE HIT from below");
                }
            }
        }
    }
    println!("{hits} of 39 rays found surface data");
}
