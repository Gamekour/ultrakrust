//! When does each level start (OnLevelStart.StartLevel, StatsManager's timer)? Lands, then walks
//! towards the FirstRoom triggers and reports the tick the level starts and where the player is versus the FirstRoom
//! triggers that call FinalDoorOpener.GoTime.
//! cargo run --release -p uk-game --example level_start -- [level filter, e.g. 1-1]
use bevy_math::Vec2;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let filter = std::env::args().nth(1);
    let install = uk_assets::find_install().expect("install");
    let dir = AssetDb::bundle_dir(&install);
    let mut levels: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .filter_map(|e| e.file_name().to_string_lossy().strip_prefix("campaign_scenes_level").and_then(|s| s.strip_suffix(".bundle")).map(str::to_string))
        .filter(|l| filter.as_deref().is_none_or(|f| l.contains(f)))
        .collect();
    levels.sort();
    let mut db = AssetDb::open(&install).unwrap();
    for l in &levels {
        let Ok(def) = scenedef::load_scene(&mut db, &dir.join(format!("campaign_scenes_level{l}.bundle"))) else { continue };
        let def = Arc::new(def);
        let mut g = Game::new(def.clone());
        let cubes: Vec<(String, bool, bevy_math::Vec3)> = ["FirstRoom/Room/Cube", "FirstRoom/Room/Cube (1)"]
            .iter()
            .filter_map(|p| (0..def.nodes.len() as u32).find(|&n| def.path(n) == *p))
            .map(|n| (def.nodes[n as usize].name.clone(), g.active(n), def.nodes[n as usize].world0.w_axis.truncate()))
            .collect();
        let mut t = 0.0;
        let mut landed = None;
        let mut started = None;
        for i in 0..3000 {
            let walk = landed.is_some_and(|k| i > k + 30);
            // steer at the FirstRoom triggers (forward = (sin yaw, 0, -cos yaw))
            if let (true, Some((_, _, c))) = (walk, cubes.last()) {
                let d = *c - g.s.player.pos;
                g.s.player.yaw_deg = d.x.atan2(-d.z).to_degrees();
            }
            let input = Input { move_axis: Vec2::new(0.0, walk as i32 as f32), ..Default::default() };
            g.fixed_update(&input);
            t += FIXED_DT as f64;
            g.update(&input, FIXED_DT, t);
            g.s.player.events.clear();
            if landed.is_none() && g.s.player.activated {
                landed = Some(i);
            }
            if g.s.stats.level_started {
                started = Some(i);
                break;
            }
        }
        println!(
            "{l}: activated at {:?} started at {:?} timer={} player {:.1?} cubes {:.1?}",
            landed,
            started,
            g.s.stats.timer,
            g.s.player.pos,
            cubes
        );
    }
}
