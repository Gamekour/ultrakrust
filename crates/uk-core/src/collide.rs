//! Static collision world: oriented boxes plus the handful of queries the
//! movement code needs (raycast, sphere cast, overlaps, capsule depenetration).

use bevy_math::{Quat, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColliderId(pub u32);

#[derive(Clone, Debug)]
pub struct BoxCollider {
    pub center: Vec3,
    pub half: Vec3,
    pub rot: Quat,
    /// Mirrors ULTRAKILL's "Slippery" tag: never counts as a wall for wall jumps.
    pub slippery: bool,
}

impl BoxCollider {
    pub fn new(center: Vec3, size: Vec3) -> Self {
        Self { center, half: size * 0.5, rot: Quat::IDENTITY, slippery: false }
    }

    pub fn rotated(mut self, rot: Quat) -> Self {
        self.rot = rot;
        self
    }

    fn to_local(&self, p: Vec3) -> Vec3 {
        self.rot.inverse() * (p - self.center)
    }

    fn to_world(&self, p: Vec3) -> Vec3 {
        self.rot * p + self.center
    }

    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let l = self.to_local(p).clamp(-self.half, self.half);
        self.to_world(l)
    }

    fn dist_sq(&self, p: Vec3) -> f32 {
        let l = self.to_local(p);
        (l - l.clamp(-self.half, self.half)).length_squared()
    }

    /// Slab test. Returns distance along `dir` (unit) and the world normal.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let o = self.to_local(origin);
        let d = self.rot.inverse() * dir;
        let mut tmin = 0.0f32;
        let mut tmax = max;
        let mut axis = 0usize;
        let mut sign = 0.0f32;
        for i in 0..3 {
            if d[i].abs() < 1e-8 {
                if o[i] < -self.half[i] || o[i] > self.half[i] {
                    return None;
                }
                continue;
            }
            let inv = 1.0 / d[i];
            let mut t0 = (-self.half[i] - o[i]) * inv;
            let mut t1 = (self.half[i] - o[i]) * inv;
            let mut s = -1.0;
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
                s = 1.0;
            }
            if t0 > tmin {
                tmin = t0;
                axis = i;
                sign = s;
            }
            tmax = tmax.min(t1);
            if tmin > tmax {
                return None;
            }
        }
        if sign == 0.0 {
            // Origin starts inside the box: Unity raycasts ignore such colliders.
            return None;
        }
        let mut n = Vec3::ZERO;
        n[axis] = sign;
        Some((tmin, self.rot * n))
    }

    /// Closest pair between segment `a..b` and the box: (point on segment, point on box).
    /// The distance to a convex set is convex along a segment, so a ternary search converges.
    pub fn closest_to_segment(&self, a: Vec3, b: Vec3) -> (Vec3, Vec3) {
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..28 {
            let m1 = lo + (hi - lo) / 3.0;
            let m2 = hi - (hi - lo) / 3.0;
            if self.dist_sq(a.lerp(b, m1)) <= self.dist_sq(a.lerp(b, m2)) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        let p = a.lerp(b, (lo + hi) * 0.5);
        (p, self.closest_point(p))
    }

    /// Exit direction and depth for a point inside the box (axis of least penetration).
    fn inside_push(&self, p: Vec3) -> (Vec3, f32) {
        let l = self.to_local(p);
        let mut best = (Vec3::ZERO, f32::MAX);
        for i in 0..3 {
            for s in [-1.0f32, 1.0] {
                let depth = self.half[i] - s * l[i];
                if depth < best.1 {
                    let mut n = Vec3::ZERO;
                    n[i] = s;
                    best = (self.rot * n, depth);
                }
            }
        }
        best
    }
}

/// A vertical capsule in world space, described by its segment and radius.
#[derive(Clone, Copy, Debug)]
pub struct Capsule {
    pub a: Vec3,
    pub b: Vec3,
    pub radius: f32,
}

impl Capsule {
    /// Unity-style capsule: `center` world point, total `height`, `radius`, Y axis.
    pub fn unity(center: Vec3, height: f32, radius: f32) -> Self {
        let h = (height * 0.5 - radius).max(0.0);
        Self { a: center - Vec3::Y * h, b: center + Vec3::Y * h, radius }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub collider: ColliderId,
}

#[derive(Default, Clone, Debug)]
pub struct World {
    pub boxes: Vec<BoxCollider>,
}

impl World {
    pub fn add(&mut self, b: BoxCollider) -> ColliderId {
        self.boxes.push(b);
        ColliderId(self.boxes.len() as u32 - 1)
    }

    pub fn get(&self, id: ColliderId) -> &BoxCollider {
        &self.boxes[id.0 as usize]
    }

    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let mut best: Option<RayHit> = None;
        for (i, b) in self.boxes.iter().enumerate() {
            if let Some((t, n)) = b.raycast(origin, dir, max) {
                if best.is_none_or(|h| t < h.distance) {
                    best = Some(RayHit { distance: t, point: origin + dir * t, normal: n, collider: ColliderId(i as u32) });
                }
            }
        }
        best
    }

    pub fn overlap_sphere(&self, c: Vec3, r: f32) -> impl Iterator<Item = ColliderId> + '_ {
        self.boxes
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.dist_sq(c) <= r * r)
            .map(|(i, _)| ColliderId(i as u32))
    }

    pub fn overlap_capsule(&self, cap: Capsule) -> impl Iterator<Item = ColliderId> + '_ {
        self.boxes
            .iter()
            .enumerate()
            .filter(move |(_, b)| {
                let (p, q) = b.closest_to_segment(cap.a, cap.b);
                p.distance_squared(q) <= cap.radius * cap.radius
            })
            .map(|(i, _)| ColliderId(i as u32))
    }

    /// Sphere sweep. Like Unity, colliders the sphere already overlaps at the start are ignored.
    pub fn sphere_cast(&self, origin: Vec3, r: f32, dir: Vec3, max: f32) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        let mut best: Option<RayHit> = None;
        let step = (r * 0.25).max(0.02);
        for (i, b) in self.boxes.iter().enumerate() {
            if b.dist_sq(origin) <= r * r {
                continue;
            }
            let mut t = 0.0f32;
            let mut hit_t = None;
            while t <= max {
                if b.dist_sq(origin + dir * t) <= r * r {
                    hit_t = Some(t);
                    break;
                }
                t += step;
            }
            let Some(mut hi) = hit_t else { continue };
            let mut lo = (hi - step).max(0.0);
            for _ in 0..16 {
                let m = (lo + hi) * 0.5;
                if b.dist_sq(origin + dir * m) <= r * r {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            if best.is_none_or(|h| hi < h.distance) {
                let c = origin + dir * hi;
                let point = b.closest_point(c);
                let normal = (c - point).normalize_or(-dir);
                best = Some(RayHit { distance: hi, point, normal, collider: ColliderId(i as u32) });
            }
        }
        best
    }

    /// Pushes a capsule out of every box. Returns the contact normals so the caller can
    /// remove the velocity going into them (frictionless, zero-bounce contact, like
    /// the player's "NoFriction" PhysicMaterial).
    pub fn depenetrate_capsule(&self, cap: &mut Capsule, normals: &mut Vec<Vec3>) {
        for _ in 0..4 {
            let mut moved = false;
            for b in &self.boxes {
                let (p, q) = b.closest_to_segment(cap.a, cap.b);
                let d2 = p.distance_squared(q);
                if d2 >= cap.radius * cap.radius {
                    continue;
                }
                let (n, depth) = if d2 > 1e-10 {
                    let d = d2.sqrt();
                    ((p - q) / d, cap.radius - d)
                } else {
                    let (n, depth) = b.inside_push(p);
                    (n, depth + cap.radius)
                };
                let push = n * (depth + 1e-4);
                cap.a += push;
                cap.b += push;
                normals.push(n);
                moved = true;
            }
            if !moved {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor() -> World {
        let mut w = World::default();
        w.add(BoxCollider::new(Vec3::new(0.0, -0.5, 0.0), Vec3::new(100.0, 1.0, 100.0)));
        w
    }

    #[test]
    fn raycast_hits_floor_top() {
        let h = floor().raycast(Vec3::new(0.0, 5.0, 0.0), Vec3::NEG_Y, 10.0).unwrap();
        assert!((h.distance - 5.0).abs() < 1e-4);
        assert!(h.normal.abs_diff_eq(Vec3::Y, 1e-5));
    }

    #[test]
    fn sphere_cast_contact_point() {
        let h = floor().sphere_cast(Vec3::new(0.0, 3.0, 0.0), 0.35, Vec3::NEG_Y, 5.0).unwrap();
        assert!((h.distance - 2.65).abs() < 1e-3, "{h:?}");
        assert!(h.point.y.abs() < 1e-3);
    }

    #[test]
    fn capsule_depenetrates_upwards() {
        let w = floor();
        let mut cap = Capsule::unity(Vec3::new(0.0, 1.6, 0.0), 3.5, 0.5);
        let mut n = vec![];
        w.depenetrate_capsule(&mut cap, &mut n);
        assert!((cap.a.y - 0.5 - 0.0).abs() < 1e-3, "{cap:?}");
        assert!(n[0].abs_diff_eq(Vec3::Y, 1e-4));
    }
}
