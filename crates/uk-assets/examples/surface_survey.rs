//! SceneHelper surface data of a level: the FootstepSet particle tables, the footstep physics
//! scene objects and the surface types / colors their materials carry.
//! cargo run --release -p uk-assets --example surface_survey -- [level0-1]
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let fx = &def.footstep_fx;
    for (name, m) in [("enviroGibParticles", &fx.enviro_gib_particles), ("slideParticles", &fx.slide_particles), ("wallScrapeParticles", &fx.wall_scrape_particles)] {
        let mut v: Vec<_> = m.iter().collect();
        v.sort();
        println!("{name}:");
        for (k, p) in v {
            println!("    {k:3} -> {}", p.map_or("null".into(), |p| def.particle_prefabs[p as usize].name.clone()));
        }
    }
    for w in def.warnings.iter().filter(|w| w.contains("Footstep")) {
        println!("warning: {w}");
    }
    let tris: usize = def.surface_meshes.iter().map(|s| s.tris.len()).sum();
    println!("footstep scene: {} objects, {tris} triangles", def.surface_meshes.len());
    let mut by: BTreeMap<(bool, i32, i32, bool, bool, String), usize> = BTreeMap::new();
    for s in &def.surface_meshes {
        for m in &s.mats {
            let c = format!("{:.2?} / {:.2?}", m.color, m.secondary_color);
            *by.entry((m.has_surface, m.surface, m.secondary, m.vertex_blending, m.static_lighting, c)).or_default() += 1;
        }
    }
    println!("(has _SurfaceType, surface, secondary, VERTEX_BLENDING, static lit, colors): material slots");
    for (k, n) in by {
        println!("    {k:?}: {n}");
    }
    let none: usize = def.surface_meshes.iter().map(|s| s.tri_mat.iter().filter(|m| m.is_none()).count()).sum();
    println!("triangles without a material: {none}");
}
