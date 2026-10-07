//! Numeric summary of `ugui::build_frame` for a level's initial hierarchy.
//! cargo run --release -p uk-game --example ugui_frame -- level0-1 1920 1080 [filter]
use uk_assets::{db::AssetDb, scenedef, ui};
use uk_game::ugui::{self, UiDef, UiInput, UiState};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let level = a.get(1).cloned().unwrap_or("level0-1".into());
    let w: f32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(1920.0);
    let h: f32 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1080.0);
    let filter = a.get(4).cloned();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let assets = ui::load_ui_assets(&mut db, &def);
    let uidef = UiDef::build(&def, &assets);
    for w in &uidef.warnings {
        println!("WARN {w}");
    }
    println!(
        "canvases {} roots {} groups {} scalers {} graphics {} masks {} rect_masks {} effects {}",
        uidef.canvases.len(),
        uidef.roots.len(),
        uidef.groups.len(),
        uidef.scalers.len(),
        uidef.graphics.len(),
        uidef.masks.len(),
        uidef.rect_masks.len(),
        uidef.effects.len()
    );
    // activeInHierarchy from activeSelf
    let mut active = vec![false; def.nodes.len()];
    fn walk(def: &scenedef::SceneDef, n: u32, up: bool, out: &mut Vec<bool>) {
        let a = up && def.nodes[n as usize].active_self;
        out[n as usize] = a;
        for &c in &def.nodes[n as usize].children {
            walk(def, c, a, out);
        }
    }
    for (i, n) in def.nodes.iter().enumerate() {
        if n.parent.is_none() {
            walk(&def, i as u32, true, &mut active);
        }
    }
    let enabled: Vec<bool> = def.scripts.iter().map(|s| s.enabled).collect();
    let state = UiState::new(&uidef);
    let inp = UiInput { def: &def, ui: &uidef, assets: &assets, state: &state, active: &active, script_enabled: &enabled, screen: [w, h], dpi: 96.0 };
    let t = std::time::Instant::now();
    let f = ugui::build_frame(&inp);
    println!("frame in {:?}", t.elapsed());
    for b in &f.batches {
        let path = def.path(uidef.canvases[b.canvas as usize].node);
        if filter.as_ref().is_some_and(|x| !path.contains(x.as_str())) {
            continue;
        }
        let nv: usize = b.draws.iter().map(|d| d.verts.len()).sum();
        println!(
            "BATCH {} mode {:?} order {} layer {} scale {:.4} refppu {} rect {:?} draws {} verts {}",
            path,
            b.mode,
            b.sorting_order,
            b.layer,
            b.scale_factor,
            b.reference_ppu,
            [b.root_rect.x, b.root_rect.y, b.root_rect.w, b.root_rect.h],
            b.draws.len(),
            nv
        );
        for d in &b.draws {
            if d.verts.is_empty() {
                println!("  {} (no mesh) {}", def.path(d.node), def.scripts[uidef.graphics[d.graphic as usize].script as usize].class);
                continue;
            }
            let mut mn = d.verts[0].pos;
            let mut mx = mn;
            for v in &d.verts {
                mn = mn.min(v.pos);
                mx = mx.max(v.pos);
            }
            let (smn, smx) = if b.mode != ugui::RenderMode::World { (b.to_screen.transform_point3(mn), b.to_screen.transform_point3(mx)) } else { (mn, mx) };
            let g = &uidef.graphics[d.graphic as usize];
            println!(
                "  {} {} v{} col{:?} tex {:?} mat {:?} st {}/{}/{} cm{} clip {:?}{} px [{:.1},{:.1}]-[{:.1},{:.1}]",
                def.path(d.node),
                def.scripts[g.script as usize].class,
                d.verts.len(),
                d.verts[0].color,
                d.texture.map(|t| assets.textures[t as usize].name.as_str()),
                d.material.map(|m| assets.materials[m as usize].name.as_str()),
                d.stencil.id,
                d.stencil.op,
                d.stencil.comp,
                d.stencil.color_mask,
                d.clip.map(|c| c.rect),
                if d.pop { " POP" } else { "" },
                smn.x,
                smn.y,
                smx.x,
                smx.y
            );
        }
    }
}
