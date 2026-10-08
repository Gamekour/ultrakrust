//! Serialized Transform / RectTransform values of nodes whose path contains the given text.
//! cargo run --release -p uk-game --example rect_of -- <path text | #node index> [level0-1]
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let want = std::env::args().nth(1).expect("path text");
    let level = std::env::args().nth(2).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    for (i, n) in def.nodes.iter().enumerate() {
        let p = def.path(i as u32);
        if want.strip_prefix('#').map_or(p.contains(&want), |k| k.parse() == Ok(i)) {
            println!("{p}: local_pos {:?} rot {:?} scale {:?} rect {:?}", n.local_pos, n.local_rot, n.local_scale, n.rect);
        }
    }
}
