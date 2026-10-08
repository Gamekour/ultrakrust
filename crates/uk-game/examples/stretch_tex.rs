//! The main textures of stretched-billboard (render mode 1) ParticleSystemRenderers: the axis
//! along which each texture's alpha is elongated (second moments), to fix which UV axis Unity
//! maps along the stretch.
//! cargo run --release -p uk-game --example stretch_tex -- [level filter]
use std::collections::BTreeSet;
use uk_assets::{db::AssetDb, scenedef, texture};

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.starts_with("campaign_scenes_") && n.contains(&*filter)
        })
        .collect();
    bundles.sort();
    let mut seen = BTreeSet::new();
    for b in &bundles {
        let Ok(def) = scenedef::load_scene(&mut db, b) else { continue };
        let rends = def.particle_systems.iter().filter_map(|p| p.renderer.clone().map(|r| (p.node, r)));
        let rends: Vec<_> = rends.chain(def.particle_prefabs.iter().flat_map(|pf| pf.nodes.iter().filter_map(|n| n.renderer.clone().map(|r| (u32::MAX, r)))).collect::<Vec<_>>()).collect();
        for (node, r) in rends {
            if r.render_mode != 1 || !r.enabled {
                continue;
            }
            let Some(Some(k)) = r.materials.first() else { continue };
            if !seen.insert((k.file.clone(), k.path_id)) {
                continue;
            }
            let Ok(f) = db.file(&k.file) else { continue };
            let Ok(mv) = f.read_id(k.path_id) else { continue };
            let Ok(m) = texture::decode_material(&mut db, &f, &mv) else { continue };
            let Some((tf, tid)) = m.main_tex else {
                println!("{}: no main texture", m.name);
                continue;
            };
            let Ok(tv) = tf.read_id(tid) else { continue };
            let Ok(t) = texture::decode_texture(&mut db, &tv) else { continue };
            // alpha (times luminance for additive textures) weighted second moments in uv units
            let (w, h) = (t.width as usize, t.height as usize);
            let (mut s, mut su, mut sv, mut suu, mut svv) = (0.0f64, 0.0, 0.0, 0.0, 0.0);
            for y in 0..h {
                for x in 0..w {
                    let px = &t.rgba[(y * w + x) * 4..(y * w + x) * 4 + 4];
                    let lum = (px[0] as f64 + px[1] as f64 + px[2] as f64) / (3.0 * 255.0);
                    let a = px[3] as f64 / 255.0 * lum;
                    let (u, v) = ((x as f64 + 0.5) / w as f64, (y as f64 + 0.5) / h as f64);
                    s += a;
                    su += a * u;
                    sv += a * v;
                    suu += a * u * u;
                    svv += a * v * v;
                }
            }
            let (mu, mv_) = (su / s.max(1e-9), sv / s.max(1e-9));
            let (vu, vv) = (suu / s.max(1e-9) - mu * mu, svv / s.max(1e-9) - mv_ * mv_);
            let at = if node == u32::MAX { "prefab".to_string() } else { def.path(node) };
            println!(
                "{} ({}x{} {}) at {at}: mean uv ({mu:.3}, {mv_:.3}) spread u {:.3} v {:.3} -> elongated along {}",
                m.name,
                w,
                h,
                t.name,
                vu.sqrt(),
                vv.sqrt(),
                if vu > vv * 1.2 { "u" } else if vv > vu * 1.2 { "v" } else { "neither" }
            );
        }
    }
}
