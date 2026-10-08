//! Winding of a level's navmesh polygons seen from above (+Y), in Bevy space, plus the agent
//! settings of each NavMeshData.
use uk_assets::{db::AssetDb, navmesh};
fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    for m in navmesh::load_scene_navmeshes(&mut db, &path).unwrap() {
        let (mut cw, mut ccw) = (0, 0);
        for t in &m.tiles {
            for p in &t.polys {
                let v: Vec<_> = p.verts.iter().map(|&i| t.verts[i as usize]).collect();
                // y component of the summed cross products: > 0 = counter-clockwise from above
                let n: f32 = (1..v.len().saturating_sub(1)).map(|k| (v[k] - v[0]).cross(v[k + 1] - v[0]).y).sum();
                if n > 0.0 { ccw += 1 } else { cw += 1 }
            }
        }
        println!("{} agent {} radius {} height {} climb {} tiles {} ccw {ccw} cw {cw}", m.name, m.agent_type, m.agent_radius, m.agent_height, m.agent_climb, m.tiles.len());
    }
}
