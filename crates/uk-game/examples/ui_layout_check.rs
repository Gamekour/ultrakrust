//! uGUI placement against the editor: every RectTransform's serialized m_LocalPosition (and a root
//! canvas's sizeDelta / localScale) is what Unity's own layout computed when the scene was saved.
//! Each overlay root is laid out at the screen size its serialized rect implies (sizeDelta *
//! scale), with every node and script on, and every RectTransform's local position is compared.
//! cargo run --release -p uk-game --example ui_layout_check -- [level0-1 ...]
use bevy_math::{Vec2, Vec3};
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef, ui};
use uk_game::ugui::{self, RenderMode, UiDef, UiInput, UiState};

fn main() {
    let levels: Vec<String> = std::env::args().skip(1).collect();
    let levels = if levels.is_empty() { vec!["level0-1".to_string()] } else { levels };
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let (mut total, mut bad_total) = (0usize, 0usize);
    for level in levels {
        let def = scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
        let assets = ui::load_ui_assets(&mut db, &def);
        let uidef = UiDef::build(&def, &assets);
        let state = UiState::new(&uidef);
        let active = vec![true; def.nodes.len()];
        let enabled = vec![true; def.scripts.len()];
        // overlay roots grouped by the editor screen their rect implies
        let mut screens: BTreeMap<(i32, i32), Vec<u32>> = BTreeMap::new();
        let mut world = Vec::new();
        for &c in &uidef.roots {
            let cd = &uidef.canvases[c as usize];
            let n = &def.nodes[cd.node as usize];
            let Some(r) = &n.rect else { continue };
            if cd.mode == RenderMode::World {
                world.push(c);
                continue;
            }
            let s = n.local_scale.x;
            let key = ((r.size_delta[0] * s).round() as i32, (r.size_delta[1] * s).round() as i32);
            if key.0 <= 0 || key.1 <= 0 {
                println!("{level}: root {} has an empty serialized rect {:?} scale {s}", def.path(cd.node), r.size_delta);
                continue;
            }
            screens.entry(key).or_default().push(c);
        }
        let mut runs: Vec<((i32, i32), Vec<u32>)> = screens.into_iter().collect();
        runs.push(((1920, 1080), world));
        let (mut checked, mut bad) = (0usize, Vec::new());
        let mut root_bad = 0;
        let mut zeroed = 0usize;
        for ((w, h), roots) in runs {
            let inp = UiInput { def: &def, ui: &uidef, assets: &assets, state: &state, active: &active, script_enabled: &enabled, screen: [w as f32, h as f32], dpi: 96.0, world_of: None };
            let f = ugui::build_frame(&inp);
            for &c in &roots {
                let root = uidef.canvases[c as usize].node;
                // the scaler's result against the serialized root
                if let Some(b) = f.batches.iter().find(|b| b.root_node == root && b.canvas == c) {
                    let n = &def.nodes[root as usize];
                    let r = n.rect.as_ref().unwrap();
                    if b.mode != RenderMode::World && ((b.scale_factor - n.local_scale.x).abs() > 1e-3 || (b.root_rect.w - r.size_delta[0]).abs() > 0.05 || (b.root_rect.h - r.size_delta[1]).abs() > 0.05) {
                        root_bad += 1;
                        println!(
                            "  ROOT {} screen {w}x{h}: scale {:.4} rect {:.2}x{:.2} vs serialized scale {:.4} {:.2}x{:.2}",
                            def.path(root),
                            b.scale_factor,
                            b.root_rect.w,
                            b.root_rect.h,
                            n.local_scale.x,
                            r.size_delta[0],
                            r.size_delta[1]
                        );
                    }
                }
                let mut stack = def.nodes[root as usize].children.clone();
                while let Some(n) = stack.pop() {
                    let nd = &def.nodes[n as usize];
                    stack.extend(nd.children.iter().copied());
                    // nested canvases are laid out like any child (their own roots are only the top ones)
                    let (Some(m), Some(pm)) = (f.node_to_root.get(&n), nd.parent.and_then(|p| f.node_to_root.get(&p))) else { continue };
                    if nd.rect.is_none() {
                        continue;
                    }
                    let local = pm.inverse() * *m;
                    let ours = local.w_axis.truncate();
                    // serialized (Unity space)
                    let ser = Vec3::new(nd.local_pos.x, nd.local_pos.y, -nd.local_pos.z);
                    // a player build keeps no layout result for these: nothing to compare
                    if ser.x == 0.0 && ser.y == 0.0 {
                        zeroed += 1;
                        continue;
                    }
                    checked += 1;
                    let d = (Vec2::new(ours.x, ours.y) - Vec2::new(ser.x, ser.y)).abs().max_element();
                    if d > 0.05 {
                        bad.push((d, n, ours, ser));
                    }
                }
            }
        }
        bad.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("{level}: {checked} RectTransforms with a serialized position ({zeroed} at 0,0 skipped), {} off by > 0.05, roots with a scaler mismatch {root_bad}", bad.len());
        for (d, n, ours, ser) in bad.iter().take(25) {
            let r = def.nodes[*n as usize].rect.as_ref().unwrap();
            println!("  {d:9.2} {} ours ({:.2},{:.2}) serialized ({:.2},{:.2}) anchors {:?}-{:?} pivot {:?}", def.path(*n), ours.x, ours.y, ser.x, ser.y, r.anchor_min, r.anchor_max, r.pivot);
        }
        // classes on the mismatching nodes (layout drivers)
        let mut classes: BTreeMap<String, usize> = BTreeMap::new();
        for (_, n, _, _) in &bad {
            for s in def.scripts.iter().filter(|s| s.node == *n || def.nodes[*n as usize].parent == Some(s.node)) {
                *classes.entry(s.class.clone()).or_default() += 1;
            }
        }
        println!("  scripts on mismatching nodes or their parents: {classes:?}");
        total += checked;
        bad_total += bad.len();
    }
    println!("TOTAL {total} checked, {bad_total} mismatching");
}
