//! `--tour <dir>`: flies the camera through fixed viewpoints (spawn, then an
//! elevated view of every room) and saves a screenshot at each, then exits.
//! A repeatable visual check that needs no input injection.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use std::path::PathBuf;

pub struct Shot {
    pub name: String,
    pub eye: Vec3,
    pub look: Vec3,
}

#[derive(Resource)]
pub struct Tour {
    pub dir: PathBuf,
    pub shots: Vec<Shot>,
    pub idx: usize,
    pub timer: f32,
    /// Let V1 land / the first frames settle before the first capture.
    pub warmup: f32,
}

impl Tour {
    pub fn new(dir: PathBuf, _spawn: Vec3, spawn_yaw: f32, rooms: &[uk_assets::scene::Room]) -> Self {
        let mut shots = vec![];
        let f = Vec3::new(spawn_yaw.to_radians().sin(), 0.0, -spawn_yaw.to_radians().cos());
        // index 0 is special: the live first-person view after V1 lands
        shots.push(Shot { name: "00_spawn_firstperson".into(), eye: Vec3::NAN, look: f });
        for (i, r) in rooms.iter().enumerate() {
            let size = r.max - r.min;
            if size.max_element() < 8.0 || size.max_element() > 2000.0 {
                continue;
            }
            let c = (r.min + r.max) * 0.5;
            let radius = size.length() * 0.5;
            // three-quarter view from above, like an editor camera
            let eye = c + Vec3::new(radius * 0.55, radius * 0.6, radius * 0.55);
            let safe: String = r.name.chars().map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' }).collect();
            shots.push(Shot { name: format!("{:02}_{}", i + 1, safe), eye, look: (c - eye).normalize() });
        }
        Self { dir, shots, idx: 0, timer: 0.0, warmup: 4.0 }
    }

    /// Camera override for the current shot (None = normal first-person view).
    pub fn camera(&self) -> Option<(Vec3, f32, f32)> {
        let s = self.shots.get(self.idx)?;
        if s.eye.is_nan() {
            return None;
        }
        let yaw = s.look.x.atan2(-s.look.z).to_degrees();
        let pitch = s.look.y.clamp(-1.0, 1.0).asin().to_degrees();
        Some((s.eye, yaw, pitch))
    }
}

pub fn run_tour(mut commands: Commands, time: Res<Time>, tour: Option<ResMut<Tour>>, mut exit: MessageWriter<AppExit>) {
    let Some(mut tour) = tour else { return };
    if tour.warmup > 0.0 {
        tour.warmup -= time.delta_secs();
        return;
    }
    tour.timer += time.delta_secs();
    if tour.timer < 0.6 {
        return;
    }
    tour.timer = 0.0;
    if tour.idx >= tour.shots.len() {
        exit.write(AppExit::Success);
        return;
    }
    let path = tour.dir.join(format!("{}.png", tour.shots[tour.idx].name));
    info!("tour shot {}", path.display());
    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    tour.idx += 1;
}
