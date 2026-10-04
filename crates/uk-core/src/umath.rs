//! Unity math semantics the movement code relies on (clamped lerps, MoveTowards,
//! SmoothDamp, projections that return zero for degenerate normals, ...).

use bevy_math::Vec3;

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

pub fn vlerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

pub fn inverse_lerp(a: f32, b: f32, v: f32) -> f32 {
    if a != b { ((v - a) / (b - a)).clamp(0.0, 1.0) } else { 0.0 }
}

pub fn move_towards(cur: f32, target: f32, max_delta: f32) -> f32 {
    if (target - cur).abs() <= max_delta { target } else { cur + (target - cur).signum() * max_delta }
}

pub fn vmove_towards(cur: Vec3, target: Vec3, max_delta: f32) -> Vec3 {
    let d = target - cur;
    let len = d.length();
    if len <= max_delta || len == 0.0 { target } else { cur + d / len * max_delta }
}

/// `Vector3.normalized`: zero for tiny vectors.
pub fn normalized(v: Vec3) -> Vec3 {
    let m = v.length();
    if m > 1e-5 { v / m } else { Vec3::ZERO }
}

pub fn project(v: Vec3, on: Vec3) -> Vec3 {
    let sq = on.length_squared();
    if sq < f32::EPSILON { Vec3::ZERO } else { on * (v.dot(on) / sq) }
}

pub fn project_on_plane(v: Vec3, n: Vec3) -> Vec3 {
    let sq = n.length_squared();
    if sq < f32::EPSILON { v } else { v - n * (v.dot(n) / sq) }
}

/// `Vector3.ClampMagnitude`, including its behaviour for a negative max length.
pub fn clamp_magnitude(v: Vec3, max: f32) -> Vec3 {
    if v.length_squared() > max * max { normalized(v) * max } else { v }
}

pub fn reflect(dir: Vec3, normal: Vec3) -> Vec3 {
    dir - 2.0 * dir.dot(normal) * normal
}

/// `Vector3.Angle` in degrees.
pub fn angle_deg(a: Vec3, b: Vec3) -> f32 {
    let denom = (a.length_squared() * b.length_squared()).sqrt();
    if denom < 1e-15 {
        return 0.0;
    }
    (a.dot(b) / denom).clamp(-1.0, 1.0).acos().to_degrees()
}

pub fn delta_angle(cur: f32, target: f32) -> f32 {
    let mut d = (target - cur).rem_euclid(360.0);
    if d > 180.0 {
        d -= 360.0;
    }
    d
}

/// `Mathf.SmoothDamp` (critically damped spring).
pub fn smooth_damp(cur: f32, target: f32, vel: &mut f32, smooth_time: f32, dt: f32) -> f32 {
    let smooth_time = smooth_time.max(0.0001);
    let omega = 2.0 / smooth_time;
    let x = omega * dt;
    let exp = 1.0 / (1.0 + x + 0.48 * x * x + 0.235 * x * x * x);
    let change = cur - target;
    let temp = (*vel + omega * change) * dt;
    *vel = (*vel - omega * temp) * exp;
    let mut out = target + (change + temp) * exp;
    if (target - cur > 0.0) == (out > target) {
        out = target;
        *vel = (out - target) / dt;
    }
    out
}

pub fn smooth_damp_angle(cur: f32, target: f32, vel: &mut f32, smooth_time: f32, dt: f32) -> f32 {
    let target = cur + delta_angle(cur, target);
    smooth_damp(cur, target, vel, smooth_time, dt)
}
