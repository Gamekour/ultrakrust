//! First-person camera, after `CameraController.LateUpdate`: look, dash FOV,
//! strafe tilt, slide eye-height smoothing and the small walking bob.

use crate::consts::*;
use crate::player::Player;
use crate::umath::*;
use bevy_math::{Quat, Vec2, Vec3};

#[derive(Clone, Debug)]
pub struct FpCamera {
    /// Pitch in degrees, positive = up (Unity rotationX).
    pub rotation_x: f32,
    /// Yaw in degrees, clockwise (Unity rotationY).
    pub rotation_y: f32,
    pub tilt_z: f32,
    tilt_vel: f32,
    pub tilt_enabled: bool,
    pub default_fov: f32,
    pub fov: f32,
    default_pos: Vec3,
    target_pos: Vec3,
    pub local_pos: Vec3,
    /// Degrees per mouse count.
    pub sensitivity: f32,
}

impl Default for FpCamera {
    fn default() -> Self {
        Self {
            rotation_x: 0.0,
            rotation_y: 0.0,
            tilt_z: 0.0,
            tilt_vel: 0.0,
            tilt_enabled: true,
            default_fov: DEFAULT_FOV,
            fov: DEFAULT_FOV,
            default_pos: CAMERA_POS,
            target_pos: CAMERA_POS - Vec3::Y * 0.1,
            local_pos: CAMERA_POS,
            sensitivity: 0.08,
        }
    }
}

impl FpCamera {
    pub fn look(&mut self, mouse_delta: Vec2) {
        let zoom = self.fov / self.default_fov;
        self.rotation_x -= mouse_delta.y * self.sensitivity * zoom;
        self.rotation_y += mouse_delta.x * self.sensitivity * zoom;
        let f = delta_angle(0.0, self.rotation_x);
        if f.abs() > 90.0 {
            self.rotation_x = 90.0 * f.signum();
        }
        self.rotation_y = self.rotation_y.rem_euclid(360.0);
    }

    pub fn late_update(&mut self, nm: &mut Player, strafe: f32, dt: f32) {
        nm.yaw_deg = self.rotation_y;
        if nm.cam_reset_requested {
            nm.cam_reset_requested = false;
            self.local_pos = self.default_pos;
            self.target_pos = self.default_pos - Vec3::Y * 0.1;
        }
        // ClimbStep keeps the camera's world position and lets defaultPos ease back
        if nm.eye_offset != 0.0 {
            self.default_pos.y -= nm.eye_offset;
            self.local_pos.y -= nm.eye_offset;
            nm.eye_offset = 0.0;
        }
        if dt > 0.0 {
            self.tilt_z = smooth_damp_angle(self.tilt_z, 0.0, &mut self.tilt_vel, 0.5, dt);
        }
        if nm.boost {
            match nm.cam_dodge_direction {
                0 => self.fov = self.default_fov - self.default_fov / 20.0,
                1 => self.fov = self.default_fov + self.default_fov / 10.0,
                _ => {}
            }
        } else {
            self.fov = move_towards(self.fov, self.default_fov, dt * 300.0);
        }
        let target = -strafe;
        let mut cur = self.tilt_z;
        if cur > 180.0 {
            cur -= 360.0;
        }
        self.tilt_z = if !self.tilt_enabled {
            move_towards(cur, 0.0, dt * 25.0 * (cur.abs() + 0.01))
        } else if nm.boost {
            move_towards(cur, target * 5.0, dt * 100.0 * ((cur - target * 5.0).abs() + 0.01))
        } else {
            move_towards(cur, target, dt * 25.0 * ((cur - target).abs() + 0.01))
        };
        let eye_target = nm.cam_default_target;
        if self.default_pos != eye_target {
            self.default_pos =
                vmove_towards(self.default_pos, eye_target, ((eye_target - self.default_pos).length() + 0.5) * dt * 10.0);
        }
        if nm.walking && nm.standing && self.default_pos == eye_target {
            let speed = nm.vel.length().min(15.0) / 15.0;
            let lp = self.local_pos;
            self.local_pos = Vec3::new(
                move_towards(lp.x, self.target_pos.x, dt * 0.5),
                move_towards(lp.y, self.target_pos.y, dt * 0.5 * speed),
                move_towards(lp.z, self.target_pos.z, dt * 0.5),
            );
            if self.local_pos == self.target_pos && self.target_pos != self.default_pos {
                self.target_pos = self.default_pos;
            } else if self.local_pos == self.target_pos {
                self.target_pos = self.default_pos - Vec3::Y * 0.1;
            }
        } else {
            self.local_pos = self.default_pos;
            self.target_pos = self.default_pos - Vec3::Y * 0.1;
        }
    }

    /// World rotation for a Bevy camera (-Z forward, right-handed).
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(-self.rotation_y.to_radians())
            * Quat::from_rotation_x(self.rotation_x.to_radians())
            * Quat::from_rotation_z(-self.tilt_z.to_radians())
    }
}
