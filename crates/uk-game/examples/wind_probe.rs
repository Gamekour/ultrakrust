//! NewMovement.windStateParticle: its transform (camera default pos + velocity direction,
//! facing the camera) and the particles rateOverDistance emits while idle, after a dodge and
//! while falling and with windState forced to 0.5 during a dodge.
//! cargo run --release -p uk-game --example wind_probe
use bevy_math::{Vec2, Vec3};
use std::sync::Arc;
use uk_assets::{db::AssetDb, scenedef};
use uk_core::consts::FIXED_DT;
use uk_core::player::Input;
use uk_game::particles::to_unity;
use uk_game::Game;

fn report(g: &Game, label: &str) {
    let Some(k) = g.fx.wind else {
        println!("no wind particle");
        return;
    };
    let s = &g.fx.scene[k].sys;
    let p = &g.s.player;
    println!(
        "[{label}] t {:.2} wind_state {:.2} mul {:.1?} particles {} playing {} | player eye (unity) {:.2?} vel {:.1?}",
        g.s.time,
        p.wind_state,
        s.rate_over_distance_mul,
        s.particles.len(),
        s.playing,
        to_unity(p.pos + p.cam_default_pos),
        to_unity(p.vel)
    );
}

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let def = Arc::new(scenedef::load_scene(&mut db, &AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle")).unwrap());
    let mut g = Game::new(def);
    if let Some(k) = g.fx.wind {
        let d = &g.fx.scene[k].sys.def;
        println!("emission {} rate {:?} rate_over_distance {:?} space {} max {} lifetime {:?}", d.emission_enabled, d.rate_over_time, d.rate_over_distance, d.simulation_space, d.max_particles, d.start_lifetime);
    }
    g.s.player.activated = true;
    let mut t = 0.0f64;
    let mut step = |g: &mut Game, inp: &Input| {
        g.fixed_update(inp);
        t += FIXED_DT as f64;
        g.update(inp, FIXED_DT, t);
        g.s.player.events.clear();
    };
    let idle = Input::default();
    for i in 0..120 {
        step(&mut g, &idle);
        if i % 60 == 59 {
            report(&g, "idle");
        }
    }
    step(&mut g, &Input { dash_pressed: true, move_axis: Vec2::new(0.0, 1.0), ..Default::default() });
    for i in 0..50 {
        step(&mut g, &idle);
        if i % 10 == 9 {
            report(&g, "after dodge");
        }
    }
    g.s.player.pos += Vec3::Y * 60.0;
    g.s.player.prev_pos = g.s.player.pos;
    for i in 0..150 {
        step(&mut g, &idle);
        if i % 25 == 24 {
            report(&g, "falling");
        }
    }
    // windState = 0.5 (what an enemy-step slide jump, a dashed-from-ground slam or a wall SSJ
    // sets) while dashing forward
    for _ in 0..150 {
        step(&mut g, &idle);
    }
    step(&mut g, &Input { dash_pressed: true, move_axis: Vec2::new(0.0, 1.0), ..Default::default() });
    g.s.player.wind_state = 0.5;
    for i in 0..40 {
        step(&mut g, &idle);
        if i % 4 == 3 {
            report(&g, "windState 0.5 at dodge");
            let s = &g.fx.scene[g.fx.wind.unwrap()].sys;
            if let Some(p) = s.particles.first() {
                println!("    first particle at {:.2?}", p.pos);
            }
        }
    }
}
