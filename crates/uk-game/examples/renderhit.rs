//! What renderer (and material) is in front of the camera?
//! cargo run --release -p uk-game --example renderhit -- <scenario> x y z yaw_deg pitch_deg
//! scenario: start | gunroom (after the pickup, 12 s) | boss (arena rooms on, trigger entered)
use bevy_math::Vec3;
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef, texture};
use uk_core::collide::Triangle;
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::Game;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scen = args[0].clone();
    let n: Vec<f32> = args[1..].iter().map(|s| s.parse().unwrap()).collect();
    let (eye, yaw, pitch) = (Vec3::new(n[0], n[1], n[2]), n[3].to_radians(), n[4].to_radians());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let def = Arc::new(scenedef::load_scene(&mut db, &path).unwrap());
    let mut g = Game::new(def.clone());
    let mut t = 0.0;
    let step = |g: &mut Game, n: usize, t: &mut f64| for _ in 0..n {
        g.fixed_update(&Input::default()); *t += FIXED_DT as f64; g.update(&Input::default(), FIXED_DT, *t); g.s.hp = 100;
    };
    step(&mut g, 300, &mut t);
    let find = |p: &str| (0..def.nodes.len() as u32).find(|&n| def.path(n) == p);
    match scen.as_str() {
        "gunroom" => {
            g.s.player.pos = Vec3::new(40.0, 1.5, -393.0); g.s.player.prev_pos = g.s.player.pos;
            step(&mut g, 1500, &mut t);
        }
        "boss" => {
            for r in ["13 - Malicious Face Arena", "12B - Pre-Boss Checkpoint"] { if let Some(n) = find(r) { g.set_active(n, true); } }
            g.s.player.pos = Vec3::new(202.0, 54.5, -421.0); g.s.player.prev_pos = g.s.player.pos;
            step(&mut g, 400, &mut t);
        }
        _ => {}
    }
    let dir = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
    let mut hits: Vec<(f32, usize)> = Vec::new();
    for (ri, r) in def.renderers.iter().enumerate() {
        if !g.active(r.node) || !r.enabled { continue; }
        if g.player_node.is_some_and(|p| def.is_descendant(r.node, p)) { continue; }
        let b = &r.batch;
        let mut best = f32::MAX;
        for tri in b.indices.chunks_exact(3) {
            let p = |i: u32| Vec3::from(b.positions[i as usize]);
            if let Some((d, _)) = Triangle::new(p(tri[0]), p(tri[1]), p(tri[2])).raycast(eye, dir, 2000.0) { best = best.min(d); }
        }
        if best < f32::MAX { hits.push((best, ri)); }
    }
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, ri) in hits.into_iter().take(6) {
        let r = &def.renderers[ri];
        let mut desc = String::from("no material");
        if let Some(k) = &r.material {
            if let Ok(f) = db.file(&k.file) {
                if let Ok(v) = f.read_id(k.path_id) {
                    let props = v.get("m_SavedProperties");
                    let texs: Vec<String> = props.get("m_TexEnvs").array().iter()
                        .filter(|p| p.get("second").get("m_Texture").pptr().1 != 0)
                        .map(|p| p.get("first").str().to_string()).collect();
                    let shader = db.read_pptr(&f, v.get("m_Shader").pptr()).ok().flatten().map(|(_, _, s)| s.get("m_ParsedForm").get("m_Name").str().to_string()).unwrap_or_default();
                    let m = texture::decode_material(&mut db, &f, &v).ok();
                    desc = format!("mat '{}' shader '{}' color {:?} textures {:?} main_tex {}", v.get("m_Name").str(), shader, m.as_ref().map(|m| m.color), texs, m.as_ref().is_some_and(|m| m.main_tex.is_some()));
                }
            }
        }
        println!("{d:7.2}  {}\n         {desc}", def.path(r.node));
    }
}
