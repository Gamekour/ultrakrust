//! Every Door in a level (default level0-1) with the rooms it activates on Open and deactivates on
//! Optimize (when the player enters one of its DoorControllers), and whether each room starts active.
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_game::scripts::Script;
use uk_game::Game;
fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let g = Game::new(def.clone());
    let (mut doors, mut with_rooms) = (0, 0);
    for (i, s) in g.s.scripts.iter().enumerate() {
        let Script::Door(d) = s else { continue };
        doors += 1;
        if d.activated_rooms.is_empty() && d.deactivated_rooms.is_empty() {
            continue;
        }
        with_rooms += 1;
        let show = |v: &[u32]| v.iter().map(|&n| format!("{} ({})", def.path(n), if g.active(n) { "on" } else { "off" })).collect::<Vec<_>>().join(", ");
        println!("{}", def.path(def.scripts[i].node));
        println!("    activates   {}", show(&d.activated_rooms));
        println!("    deactivates {}", show(&d.deactivated_rooms));
    }
    println!("doors {doors}, with room lists {with_rooms}");
}
