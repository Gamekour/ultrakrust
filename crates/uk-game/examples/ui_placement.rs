//! Screen placement of every visible gameplay UI draw a few seconds after the level starts, at several
//! resolutions: overlay / camera canvases through their CanvasScaler, the HUD Camera's world-space
//! canvases (layer 13) through the HUD Camera (fov 90). Each draw's visible-vertex bounds are printed
//! normalized to the screen (0..1, bottom-left origin); draws reaching outside the screen and layout
//! inputs the port does not compute are listed.
//! cargo run --release -p uk-game --example ui_placement -- [level0-1] [WxH,...] [seconds after start] [idle]
//! (`idle`: no input; stops the given seconds after load instead of after the level start)
use bevy_math::{Vec2, Vec3};
use std::collections::BTreeMap;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef, ui};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::ugui::{self, RenderMode, UiInput};
use uk_game::Game;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let level = a.get(1).cloned().unwrap_or("level0-1".into());
    let screens: Vec<[f32; 2]> = a
        .get(2)
        .cloned()
        .unwrap_or("1920x1080,1280x720,2560x1080,1280x1024,3840x2160".into())
        .split(',')
        .map(|s| {
            let (w, h) = s.split_once('x').unwrap();
            [w.parse().unwrap(), h.parse().unwrap()]
        })
        .collect();
    let after: f64 = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(3.0);
    let idle = a.get(4).is_some_and(|s| s == "idle");
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap());
    let prefs = uk_assets::prefs::Prefs::load(&install);
    let save = uk_assets::save::Save::load(&install, prefs.int("selectedSaveSlot"));
    let mut g = Game::with_prefs(def.clone(), prefs, save);
    g.set_ui(ui::load_ui_assets(&mut db, &def));
    let assets = g.ui_assets.clone().unwrap();
    let mut t = 0.0f64;
    let mut started_at = None;
    let mut landed = None;
    // land, then walk at the FirstRoom triggers until the level starts (level_start.rs); 0-1 starts after
    // the revolver pickup (its title activates the Gun Room's FinalDoorOpener): step onto it
    let pickup = (0..def.nodes.len() as u32).find(|&n| def.path(n).ends_with("/RevolverPickUp")).map(|n| def.nodes[n as usize].world0.w_axis.truncate());
    let target = (0..def.nodes.len() as u32).find(|&n| def.path(n) == "FirstRoom/Room/Cube (1)").map(|n| def.nodes[n as usize].world0.w_axis.truncate());
    for i in 0..6000 {
        if idle && t >= after {
            break;
        }
        let walk = !idle && started_at.is_none() && landed.is_some_and(|k| i > k + 30);
        if let (true, Some(c)) = (walk, target) {
            let d = c - g.s.player.pos;
            g.s.player.yaw_deg = d.x.atan2(-d.z).to_degrees();
        }
        if let (Some(k), Some(p), None, false) = (landed, pickup, started_at, idle) {
            if i == k + 60 {
                g.s.player.pos = p + Vec3::Y * 0.5;
                g.s.player.prev_pos = g.s.player.pos;
            }
        }
        if landed.is_none() && g.s.player.activated {
            landed = Some(i);
        }
        let input = Input { move_axis: Vec2::new(0.0, walk as i32 as f32), ..Default::default() };
        g.fixed_update(&input);
        t += FIXED_DT as f64;
        g.update(&input, FIXED_DT, t);
        g.s.player.events.clear();
        if g.s.stats.level_started && started_at.is_none() {
            started_at = Some(t);
        }
        if started_at.is_some_and(|s| t - s >= after) {
            break;
        }
    }
    println!("{level}: t={t:.3} landed {landed:?} level started at {started_at:?} player {:?} target {target:?}", g.s.player.pos);
    let ui = g.ui.as_ref().unwrap();
    let hud_cam = def.find("HUD Camera").map(|h| g.node_world(h));
    // element -> per-screen normalized bounds
    let mut table: BTreeMap<String, Vec<Option<[f32; 4]>>> = BTreeMap::new();
    let mut outside: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut unported: BTreeMap<(String, &str), usize> = BTreeMap::new();
    for (si, &[w, h]) in screens.iter().enumerate() {
        let f = ugui::build_frame(&UiInput { def: &g.def, ui, assets: &assets, state: &g.s.ui, active: &g.s.active, script_enabled: &g.s.script_enabled, screen: [w, h], dpi: 96.0, world_of: Some(&|n| g.node_world(n)) });
        for (n, what) in &f.layout_unported {
            *unported.entry((def.path(*n), what)).or_default() += 1;
        }
        for b in &f.batches {
            // root-local -> normalized screen
            let to_norm: Box<dyn Fn(Vec3) -> Option<Vec2>> = match b.mode {
                RenderMode::Overlay | RenderMode::Camera => Box::new(move |p: Vec3| {
                    let s = b.to_screen.transform_point3(p);
                    Some(Vec2::new(s.x / w, s.y / h))
                }),
                RenderMode::World => {
                    // the HUD Camera's canvases only (layer 13)
                    let Some(cam) = hud_cam.filter(|_| b.layer == 13) else { continue };
                    let root_w = g.node_world(b.root_node);
                    let view = cam.inverse() * root_w;
                    let tan = (45f32).to_radians().tan();
                    let aspect = w / h;
                    Box::new(move |p: Vec3| {
                        // Unity space: camera looks along +z
                        let c = view.transform_point3(p);
                        (c.z > 0.0).then(|| Vec2::new(0.5 + 0.5 * c.x / (c.z * tan * aspect), 0.5 + 0.5 * c.y / (c.z * tan)))
                    })
                }
            };
            for d in &b.draws {
                if d.pop {
                    continue;
                }
                let vis: Vec<Vec2> = d.verts.iter().filter(|v| v.color[3] > 0).filter_map(|v| to_norm(v.pos)).collect();
                if vis.is_empty() {
                    continue;
                }
                let (mn, mx) = vis.iter().fold((Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)), |(a, b), v| (a.min(*v), b.max(*v)));
                let path = def.path(d.node);
                let key = format!("{path} #{}", d.graphic);
                let row = table.entry(key.clone()).or_insert_with(|| vec![None; screens.len()]);
                row[si] = Some([mn.x, mn.y, mx.x, mx.y]);
                if mn.x < -0.001 || mn.y < -0.001 || mx.x > 1.001 || mx.y > 1.001 {
                    outside.entry(key).or_default().push(format!("{w}x{h} [{:.3},{:.3}]-[{:.3},{:.3}]", mn.x, mn.y, mx.x, mx.y));
                }
            }
        }
    }
    let res: Vec<String> = screens.iter().map(|s| format!("{}x{}", s[0], s[1])).collect();
    println!("normalized bounds (x0,y0)-(x1,y1) per screen: {}", res.join(" | "));
    for (k, row) in &table {
        let cells: Vec<String> = row.iter().map(|c| c.map_or("-".into(), |c| format!("{:.3},{:.3}-{:.3},{:.3}", c[0], c[1], c[2], c[3]))).collect();
        println!("  {k}: {}", cells.join(" | "));
    }
    println!("reaching outside the screen: {}", outside.len());
    for (k, v) in &outside {
        println!("  {k}: {}", v.join("; "));
    }
    println!("unported layout inputs: {}", unported.len());
    for ((p, w), c) in &unported {
        println!("  {p}: {w} x{c}");
    }
}
