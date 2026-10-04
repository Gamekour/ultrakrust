//! Print navgraph build stats (for cross-process determinism checks).
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let g = uk_game::Game::new(def);
    let n = g.nav.as_ref().unwrap();
    let links: usize = n.polys.iter().map(|p| p.links.len()).sum();
    println!("polys {} links {links} ext {} matched {} components {}", n.polys.len(), n.external_edges, n.external_matched, n.components());
}
