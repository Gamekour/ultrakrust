//! Static collision world: oriented boxes and triangles (level MeshColliders),
//! a BVH over them, and the queries the movement code needs (raycast, sphere
//! cast, overlaps, capsule depenetration).

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

    fn aabb(&self) -> Aabb {
        let m = bevy_math::Mat3::from_quat(self.rot);
        let ext = Vec3::new(
            m.row(0).abs().dot(self.half),
            m.row(1).abs().dot(self.half),
            m.row(2).abs().dot(self.half),
        );
        Aabb { min: self.center - ext, max: self.center + ext }
    }
}

#[derive(Clone, Debug)]
pub struct Triangle {
    pub a: Vec3,
    pub b: Vec3,
    pub c: Vec3,
    pub n: Vec3,
}

impl Triangle {
    pub fn new(a: Vec3, b: Vec3, c: Vec3) -> Self {
        Self { a, b, c, n: (b - a).cross(c - a).normalize_or_zero() }
    }

    /// Ericson, Real-Time Collision Detection 5.1.5.
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let (a, b, c) = (self.a, self.b, self.c);
        let ab = b - a;
        let ac = c - a;
        let ap = p - a;
        let d1 = ab.dot(ap);
        let d2 = ac.dot(ap);
        if d1 <= 0.0 && d2 <= 0.0 {
            return a;
        }
        let bp = p - b;
        let d3 = ab.dot(bp);
        let d4 = ac.dot(bp);
        if d3 >= 0.0 && d4 <= d3 {
            return b;
        }
        let vc = d1 * d4 - d3 * d2;
        if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
            return a + ab * (d1 / (d1 - d3));
        }
        let cp = p - c;
        let d5 = ab.dot(cp);
        let d6 = ac.dot(cp);
        if d6 >= 0.0 && d5 <= d6 {
            return c;
        }
        let vb = d5 * d2 - d1 * d6;
        if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
            return a + ac * (d2 / (d2 - d6));
        }
        let va = d3 * d6 - d5 * d4;
        if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
            return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
        }
        let denom = 1.0 / (va + vb + vc);
        a + ab * (vb * denom) + ac * (vc * denom)
    }

    /// Möller–Trumbore, double-sided. Normal faces the ray origin.
    pub fn raycast(&self, o: Vec3, d: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let e1 = self.b - self.a;
        let e2 = self.c - self.a;
        let p = d.cross(e2);
        let det = e1.dot(p);
        if det.abs() < 1e-9 {
            return None;
        }
        let inv = 1.0 / det;
        let s = o - self.a;
        let u = s.dot(p) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = s.cross(e1);
        let v = d.dot(q) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = e2.dot(q) * inv;
        if t < 0.0 || t > max {
            return None;
        }
        let n = if self.n.dot(d) > 0.0 { -self.n } else { self.n };
        Some((t, n))
    }

    fn aabb(&self) -> Aabb {
        Aabb { min: self.a.min(self.b).min(self.c), max: self.a.max(self.b).max(self.c) }
    }
}

#[derive(Clone, Debug)]
pub enum Shape {
    Box(BoxCollider),
    Tri(Triangle),
}

impl Shape {
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        match self {
            Shape::Box(b) => b.closest_point(p),
            Shape::Tri(t) => t.closest_point(p),
        }
    }

    fn dist_sq(&self, p: Vec3) -> f32 {
        self.closest_point(p).distance_squared(p)
    }

    pub fn raycast(&self, o: Vec3, d: Vec3, max: f32) -> Option<(f32, Vec3)> {
        match self {
            Shape::Box(b) => b.raycast(o, d, max),
            Shape::Tri(t) => t.raycast(o, d, max),
        }
    }

    pub fn slippery(&self) -> bool {
        matches!(self, Shape::Box(b) if b.slippery)
    }

    fn aabb(&self) -> Aabb {
        match self {
            Shape::Box(b) => b.aabb(),
            Shape::Tri(t) => t.aabb(),
        }
    }

    /// Closest pair between segment `a..b` and the shape: (point on segment, point on shape).
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

    /// Push direction and depth when the capsule segment touches/crosses the shape.
    fn deep_push(&self, p: Vec3, center: Vec3, radius: f32) -> (Vec3, f32) {
        match self {
            Shape::Box(b) => {
                let (n, d) = b.inside_push(p);
                (n, d + radius)
            }
            Shape::Tri(t) => {
                let side = (center - t.a).dot(t.n);
                let n = if side >= 0.0 { t.n } else { -t.n };
                (n, radius - side.abs().min(radius) + 1e-3)
            }
        }
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

    fn aabb(&self) -> Aabb {
        Aabb { min: self.a.min(self.b) - Vec3::splat(self.radius), max: self.a.max(self.b) + Vec3::splat(self.radius) }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RayHit {
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
    pub collider: ColliderId,
}

#[derive(Clone, Copy, Debug)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    fn empty() -> Self {
        Self { min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN) }
    }
    fn union(self, o: Aabb) -> Aabb {
        Aabb { min: self.min.min(o.min), max: self.max.max(o.max) }
    }
    fn overlaps(&self, o: &Aabb) -> bool {
        self.min.cmple(o.max).all() && self.max.cmpge(o.min).all()
    }
    fn ray(&self, o: Vec3, inv: Vec3, max: f32) -> bool {
        let t0 = (self.min - o) * inv;
        let t1 = (self.max - o) * inv;
        let tmin = t0.min(t1).max_element().max(0.0);
        let tmax = t0.max(t1).min_element().min(max);
        tmin <= tmax
    }
}

#[derive(Clone, Debug)]
struct BvhNode {
    bounds: Aabb,
    /// leaf: start..start+count into `order`; inner: count == 0, children at `start`, `start+1`
    start: u32,
    count: u32,
}

#[derive(Default, Clone, Debug)]
struct Bvh {
    nodes: Vec<BvhNode>,
    order: Vec<u32>,
}

impl Bvh {
    fn build(shapes: &[Shape]) -> Self {
        let boxes: Vec<Aabb> = shapes.iter().map(|s| s.aabb()).collect();
        let mut order: Vec<u32> = (0..shapes.len() as u32).collect();
        let mut nodes = vec![BvhNode { bounds: Aabb::empty(), start: 0, count: 0 }];
        let mut stack = vec![(0usize, 0usize, order.len())];
        while let Some((ni, lo, hi)) = stack.pop() {
            let bounds = order[lo..hi].iter().fold(Aabb::empty(), |a, &i| a.union(boxes[i as usize]));
            nodes[ni].bounds = bounds;
            if hi - lo <= 4 {
                nodes[ni].start = lo as u32;
                nodes[ni].count = (hi - lo) as u32;
                continue;
            }
            let ext = bounds.max - bounds.min;
            let axis = if ext.x >= ext.y && ext.x >= ext.z { 0 } else if ext.y >= ext.z { 1 } else { 2 };
            let mid = (lo + hi) / 2;
            order[lo..hi].select_nth_unstable_by(mid - lo, |&a, &b| {
                let ca = boxes[a as usize].min[axis] + boxes[a as usize].max[axis];
                let cb = boxes[b as usize].min[axis] + boxes[b as usize].max[axis];
                ca.total_cmp(&cb)
            });
            let left = nodes.len();
            nodes.push(BvhNode { bounds: Aabb::empty(), start: 0, count: 0 });
            nodes.push(BvhNode { bounds: Aabb::empty(), start: 0, count: 0 });
            nodes[ni].start = left as u32;
            nodes[ni].count = 0;
            stack.push((left, lo, mid));
            stack.push((left + 1, mid, hi));
        }
        Self { nodes, order }
    }

    fn query(&self, q: &Aabb, mut f: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if !n.bounds.overlaps(q) {
                continue;
            }
            if n.count > 0 {
                for &s in &self.order[n.start as usize..(n.start + n.count) as usize] {
                    f(s);
                }
            } else {
                stack.push(n.start);
                stack.push(n.start + 1);
            }
        }
    }

    fn query_ray(&self, o: Vec3, d: Vec3, max: f32, mut f: impl FnMut(u32)) {
        if self.nodes.is_empty() {
            return;
        }
        let inv = Vec3::ONE / d;
        let mut stack = vec![0u32];
        while let Some(i) = stack.pop() {
            let n = &self.nodes[i as usize];
            if !n.bounds.ray(o, inv, max) {
                continue;
            }
            if n.count > 0 {
                for &s in &self.order[n.start as usize..(n.start + n.count) as usize] {
                    f(s);
                }
            } else {
                stack.push(n.start);
                stack.push(n.start + 1);
            }
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct World {
    pub shapes: Vec<Shape>,
    bvh: Bvh,
    built_for: usize,
}

impl World {
    pub fn add(&mut self, b: BoxCollider) -> ColliderId {
        self.shapes.push(Shape::Box(b));
        ColliderId(self.shapes.len() as u32 - 1)
    }

    pub fn add_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) -> ColliderId {
        self.shapes.push(Shape::Tri(Triangle::new(a, b, c)));
        ColliderId(self.shapes.len() as u32 - 1)
    }

    /// Builds the BVH. Queries before this (or after more adds) fall back to brute force.
    pub fn build(&mut self) {
        self.bvh = Bvh::build(&self.shapes);
        self.built_for = self.shapes.len();
    }

    pub fn get(&self, id: ColliderId) -> &Shape {
        &self.shapes[id.0 as usize]
    }

    fn candidates(&self, q: Aabb, mut f: impl FnMut(u32)) {
        if self.built_for == self.shapes.len() && !self.shapes.is_empty() {
            self.bvh.query(&q, f);
        } else {
            (0..self.shapes.len() as u32).for_each(&mut f);
        }
    }

    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let mut best: Option<RayHit> = None;
        let mut test = |i: u32| {
            if let Some((t, n)) = self.shapes[i as usize].raycast(origin, dir, max) {
                if best.is_none_or(|h| t < h.distance) {
                    best = Some(RayHit { distance: t, point: origin + dir * t, normal: n, collider: ColliderId(i) });
                }
            }
        };
        if self.built_for == self.shapes.len() && !self.shapes.is_empty() {
            self.bvh.query_ray(origin, dir, max, test);
        } else {
            (0..self.shapes.len() as u32).for_each(&mut test);
        }
        best
    }

    pub fn overlap_sphere(&self, c: Vec3, r: f32) -> Vec<ColliderId> {
        let mut out = Vec::new();
        self.candidates(Aabb { min: c - Vec3::splat(r), max: c + Vec3::splat(r) }, |i| {
            if self.shapes[i as usize].dist_sq(c) <= r * r {
                out.push(ColliderId(i));
            }
        });
        out
    }

    pub fn overlap_capsule(&self, cap: Capsule) -> Vec<ColliderId> {
        let mut out = Vec::new();
        self.candidates(cap.aabb(), |i| {
            let (p, q) = self.shapes[i as usize].closest_to_segment(cap.a, cap.b);
            if p.distance_squared(q) <= cap.radius * cap.radius {
                out.push(ColliderId(i));
            }
        });
        out
    }

    /// Sphere sweep. Like Unity, colliders the sphere already overlaps at the start are ignored.
    pub fn sphere_cast(&self, origin: Vec3, r: f32, dir: Vec3, max: f32) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        let end = origin + dir * max;
        let q = Aabb { min: origin.min(end) - Vec3::splat(r), max: origin.max(end) + Vec3::splat(r) };
        let mut best: Option<RayHit> = None;
        let step = (r * 0.25).max(0.02);
        self.candidates(q, |i| {
            let s = &self.shapes[i as usize];
            if s.dist_sq(origin) <= r * r {
                return;
            }
            let mut t = 0.0f32;
            let mut hit_t = None;
            while t <= max {
                if s.dist_sq(origin + dir * t) <= r * r {
                    hit_t = Some(t);
                    break;
                }
                t += step;
            }
            let Some(mut hi) = hit_t else { return };
            let mut lo = (hi - step).max(0.0);
            for _ in 0..16 {
                let m = (lo + hi) * 0.5;
                if s.dist_sq(origin + dir * m) <= r * r {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            if best.is_none_or(|h| hi < h.distance) {
                let c = origin + dir * hi;
                let point = s.closest_point(c);
                let normal = (c - point).normalize_or(-dir);
                best = Some(RayHit { distance: hi, point, normal, collider: ColliderId(i) });
            }
        });
        best
    }

    /// Pushes a capsule out of every shape. Returns the contact normals so the caller can
    /// remove the velocity going into them (frictionless, zero-bounce contact, like
    /// the player's "NoFriction" PhysicMaterial).
    pub fn depenetrate_capsule(&self, cap: &mut Capsule, normals: &mut Vec<Vec3>) {
        let mut cands = Vec::new();
        let margin = Vec3::splat(0.5);
        let q = cap.aabb();
        self.candidates(Aabb { min: q.min - margin, max: q.max + margin }, |i| cands.push(i));
        for _ in 0..4 {
            let mut moved = false;
            for &i in &cands {
                let s = &self.shapes[i as usize];
                let (p, q) = s.closest_to_segment(cap.a, cap.b);
                let d2 = p.distance_squared(q);
                if d2 >= cap.radius * cap.radius {
                    continue;
                }
                let (n, depth) = if d2 > 1e-10 {
                    let d = d2.sqrt();
                    ((p - q) / d, cap.radius - d)
                } else {
                    s.deep_push(p, (cap.a + cap.b) * 0.5, cap.radius)
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

    fn tri_floor() -> World {
        let mut w = World::default();
        let s = 50.0;
        w.add_triangle(Vec3::new(-s, 0.0, -s), Vec3::new(-s, 0.0, s), Vec3::new(s, 0.0, s));
        w.add_triangle(Vec3::new(-s, 0.0, -s), Vec3::new(s, 0.0, s), Vec3::new(s, 0.0, -s));
        w.build();
        w
    }

    #[test]
    fn raycast_hits_floor_top() {
        for w in [floor(), tri_floor()] {
            let h = w.raycast(Vec3::new(0.3, 5.0, 0.2), Vec3::NEG_Y, 10.0).unwrap();
            assert!((h.distance - 5.0).abs() < 1e-4);
            assert!(h.normal.abs_diff_eq(Vec3::Y, 1e-5));
        }
    }

    #[test]
    fn sphere_cast_contact_point() {
        for w in [floor(), tri_floor()] {
            let h = w.sphere_cast(Vec3::new(0.0, 3.0, 0.0), 0.35, Vec3::NEG_Y, 5.0).unwrap();
            assert!((h.distance - 2.65).abs() < 1e-3, "{h:?}");
            assert!(h.point.y.abs() < 1e-3);
        }
    }

    #[test]
    fn capsule_depenetrates_upwards() {
        for w in [floor(), tri_floor()] {
            let mut cap = Capsule::unity(Vec3::new(0.0, 1.6, 0.0), 3.5, 0.5);
            let mut n = vec![];
            w.depenetrate_capsule(&mut cap, &mut n);
            assert!((cap.a.y - 0.5).abs() < 1e-3, "{cap:?}");
            assert!(n[0].abs_diff_eq(Vec3::Y, 1e-4));
        }
    }

    #[test]
    fn bvh_matches_brute_force() {
        let mut w = World::default();
        for i in 0..200 {
            let x = (i % 20) as f32 * 3.0;
            let z = (i / 20) as f32 * 3.0;
            w.add_triangle(Vec3::new(x, 0.0, z), Vec3::new(x, 0.0, z + 2.0), Vec3::new(x + 2.0, 0.5, z));
        }
        let brute = w.clone();
        w.build();
        for k in 0..50 {
            let o = Vec3::new(k as f32 * 1.1, 4.0, k as f32 * 0.6);
            let a = w.raycast(o, Vec3::new(0.1, -1.0, 0.05), 20.0).map(|h| h.collider);
            let b = brute.raycast(o, Vec3::new(0.1, -1.0, 0.05), 20.0).map(|h| h.collider);
            assert_eq!(a, b);
            assert_eq!(w.overlap_sphere(o - Vec3::Y * 3.8, 1.0).len(), brute.overlap_sphere(o - Vec3::Y * 3.8, 1.0).len());
        }
    }
}
