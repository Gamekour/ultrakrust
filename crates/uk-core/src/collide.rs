//! Static collision world: oriented boxes and triangles (level MeshColliders),
//! a BVH over them, and the queries the movement code needs (raycast, sphere
//! cast, overlaps, capsule depenetration).

use bevy_math::{Affine3A, Quat, Vec3};

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

/// Closest point to `p` on segment `a..b`.
pub fn closest_on_segment(a: Vec3, b: Vec3, p: Vec3) -> Vec3 {
    let ab = b - a;
    let l2 = ab.length_squared();
    if l2 < 1e-12 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / l2).clamp(0.0, 1.0)
}

/// Closest points between segments `p1..q1` and `p2..q2` (Ericson, Real-Time Collision Detection 5.1.9).
pub fn closest_segments(p1: Vec3, q1: Vec3, p2: Vec3, q2: Vec3) -> (Vec3, Vec3) {
    let d1 = q1 - p1;
    let d2 = q2 - p2;
    let r = p1 - p2;
    let a = d1.length_squared();
    let e = d2.length_squared();
    let f = d2.dot(r);
    let (s, t);
    if a <= 1e-12 && e <= 1e-12 {
        return (p1, p2);
    }
    if a <= 1e-12 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-12 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let denom = a * e - b * b;
            let s0 = if denom > 1e-12 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
            let t0 = (b * s0 + f) / e;
            if t0 < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t0 > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            } else {
                t = t0;
                s = s0;
            }
        }
    }
    (p1 + d1 * s, p2 + d2 * t)
}

/// Unity CapsuleCollider / SphereCollider (a sphere is a capsule with `a == b`): a solid.
impl Capsule {
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        let q = closest_on_segment(self.a, self.b, p);
        let d = p - q;
        let l = d.length();
        if l <= self.radius {
            p
        } else {
            q + d * (self.radius / l)
        }
    }

    /// Ray against the solid. Like Unity, a ray starting inside misses.
    pub fn raycast(&self, o: Vec3, d: Vec3, max: f32) -> Option<(f32, Vec3)> {
        let r = self.radius;
        if closest_on_segment(self.a, self.b, o).distance_squared(o) <= r * r {
            return None;
        }
        let sphere = |c: Vec3| -> Option<f32> {
            let m = o - c;
            let b = m.dot(d);
            let cc = m.length_squared() - r * r;
            let disc = b * b - cc;
            (disc >= 0.0 && b < 0.0).then(|| -b - disc.sqrt()).filter(|t| *t >= 0.0)
        };
        let mut best = [sphere(self.a), sphere(self.b)].into_iter().flatten().fold(f32::MAX, f32::min);
        let axis = self.b - self.a;
        let len = axis.length();
        if len > 1e-6 {
            let u = axis / len;
            let m = o - self.a;
            let dd = d - u * d.dot(u);
            let mm = m - u * m.dot(u);
            let qa = dd.length_squared();
            if qa > 1e-12 {
                let qb = mm.dot(dd);
                let disc = qb * qb - qa * (mm.length_squared() - r * r);
                if disc >= 0.0 {
                    let t = (-qb - disc.sqrt()) / qa;
                    let h = (m + d * t).dot(u);
                    if t >= 0.0 && (0.0..=len).contains(&h) {
                        best = best.min(t);
                    }
                }
            }
        }
        if best > max {
            return None;
        }
        let h = o + d * best;
        Some((best, (h - closest_on_segment(self.a, self.b, h)).normalize_or(-d)))
    }

    /// Exit direction and depth (to the surface) for a point inside.
    fn inside_push(&self, p: Vec3, toward: Vec3) -> (Vec3, f32) {
        let q = closest_on_segment(self.a, self.b, p);
        let d = p - q;
        let l = d.length();
        if l > 1e-5 {
            return (d / l, self.radius - l);
        }
        // on the core: push sideways toward `toward` (the other capsule's centre)
        let axis = (self.b - self.a).normalize_or(Vec3::Y);
        let side = toward - q;
        let side = (side - axis * side.dot(axis)).normalize_or(axis.any_orthonormal_vector());
        (side, self.radius)
    }
}

#[derive(Clone, Debug)]
pub enum Shape {
    Box(BoxCollider),
    Tri(Triangle),
    Capsule(Capsule),
}

impl Shape {
    pub fn closest_point(&self, p: Vec3) -> Vec3 {
        match self {
            Shape::Box(b) => b.closest_point(p),
            Shape::Tri(t) => t.closest_point(p),
            Shape::Capsule(c) => c.closest_point(p),
        }
    }

    fn dist_sq(&self, p: Vec3) -> f32 {
        self.closest_point(p).distance_squared(p)
    }

    pub fn raycast(&self, o: Vec3, d: Vec3, max: f32) -> Option<(f32, Vec3)> {
        match self {
            Shape::Box(b) => b.raycast(o, d, max),
            Shape::Tri(t) => t.raycast(o, d, max),
            Shape::Capsule(c) => c.raycast(o, d, max),
        }
    }

    pub fn slippery(&self) -> bool {
        matches!(self, Shape::Box(b) if b.slippery)
    }

    fn aabb(&self) -> Aabb {
        match self {
            Shape::Box(b) => b.aabb(),
            Shape::Tri(t) => t.aabb(),
            Shape::Capsule(c) => c.aabb(),
        }
    }

    /// Closest pair between segment `a..b` and the shape: (point on segment, point on shape).
    /// The distance to a convex set is convex along a segment, so a ternary search converges.
    pub fn closest_to_segment(&self, a: Vec3, b: Vec3) -> (Vec3, Vec3) {
        if let Shape::Capsule(c) = self {
            let (p, _) = closest_segments(a, b, c.a, c.b);
            return (p, c.closest_point(p));
        }
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
            Shape::Capsule(c) => {
                let (n, d) = c.inside_push(p, center);
                (n, d + radius)
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

    fn query(&self, q: &Aabb, f: &mut impl FnMut(u32)) {
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

    fn query_ray(&self, o: Vec3, d: Vec3, max: f32, f: &mut impl FnMut(u32)) {
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

/// Where a shape sits: its group (static level = 0, or a moving body) and index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColliderId {
    pub group: u32,
    pub index: u32,
}

/// Owner value for shapes that are always enabled.
pub const ALWAYS: u32 = u32::MAX;

/// A set of shapes sharing one rigid transform (local -> world). Group 0 is the
/// static level; doors and other moving bodies get their own group.
#[derive(Default, Clone, Debug)]
pub struct Group {
    pub shapes: Vec<Shape>,
    /// Per-shape owner (index into `World::owner_enabled`) or [`ALWAYS`].
    pub owners: Vec<u32>,
    bvh: Bvh,
    built_for: usize,
    pub xf: Affine3A,
    pub inv: Affine3A,
}

impl Group {
    fn candidates(&self, q: Aabb, f: &mut impl FnMut(u32)) {
        if self.built_for == self.shapes.len() && !self.shapes.is_empty() {
            self.bvh.query(&q, f);
        } else {
            (0..self.shapes.len() as u32).for_each(f);
        }
    }
}

#[derive(Clone, Debug)]
pub struct World {
    pub groups: Vec<Group>,
    /// Enabled flag per owner; the game toggles these as objects (de)activate.
    pub owner_enabled: Vec<bool>,
}

impl Default for World {
    fn default() -> Self {
        Self {
            groups: vec![Group { xf: Affine3A::IDENTITY, inv: Affine3A::IDENTITY, ..Default::default() }],
            owner_enabled: Vec::new(),
        }
    }
}

fn aabb_xf(q: Aabb, m: &Affine3A) -> Aabb {
    let c = m.transform_point3((q.min + q.max) * 0.5);
    let h = (q.max - q.min) * 0.5;
    let r = bevy_math::Mat3::from(m.matrix3);
    let e = Vec3::new(r.row(0).abs().dot(h), r.row(1).abs().dot(h), r.row(2).abs().dot(h));
    Aabb { min: c - e, max: c + e }
}

impl World {
    pub fn add(&mut self, b: BoxCollider) -> ColliderId {
        self.add_shape(0, Shape::Box(b), ALWAYS)
    }

    pub fn add_triangle(&mut self, a: Vec3, b: Vec3, c: Vec3) -> ColliderId {
        self.add_shape(0, Shape::Tri(Triangle::new(a, b, c)), ALWAYS)
    }

    pub fn add_shape(&mut self, group: usize, shape: Shape, owner: u32) -> ColliderId {
        let g = &mut self.groups[group];
        g.shapes.push(shape);
        g.owners.push(owner);
        if owner != ALWAYS && owner as usize >= self.owner_enabled.len() {
            self.owner_enabled.resize(owner as usize + 1, true);
        }
        ColliderId { group: group as u32, index: g.shapes.len() as u32 - 1 }
    }

    /// New moving group; its shapes are given in its local space (the pose at load time).
    pub fn new_group(&mut self) -> usize {
        self.groups.push(Group { xf: Affine3A::IDENTITY, inv: Affine3A::IDENTITY, ..Default::default() });
        self.groups.len() - 1
    }

    pub fn set_group_transform(&mut self, group: usize, xf: Affine3A) {
        let g = &mut self.groups[group];
        g.xf = xf;
        g.inv = xf.inverse();
    }

    /// Builds the BVHs. Queries before this (or after more adds) fall back to brute force.
    pub fn build(&mut self) {
        for g in &mut self.groups {
            g.bvh = Bvh::build(&g.shapes);
            g.built_for = g.shapes.len();
        }
    }

    pub fn get(&self, id: ColliderId) -> &Shape {
        &self.groups[id.group as usize].shapes[id.index as usize]
    }

    pub fn owner(&self, id: ColliderId) -> u32 {
        self.groups[id.group as usize].owners[id.index as usize]
    }

    /// Closest point on a shape, in world space.
    pub fn closest_point(&self, id: ColliderId, p: Vec3) -> Vec3 {
        let g = &self.groups[id.group as usize];
        g.xf.transform_point3(g.shapes[id.index as usize].closest_point(g.inv.transform_point3(p)))
    }

    fn enabled(&self, g: &Group, i: u32) -> bool {
        let o = g.owners[i as usize];
        o == ALWAYS || self.owner_enabled.get(o as usize).copied().unwrap_or(true)
    }

    /// Calls `f(group index, group, shape index)` for enabled shapes whose bounds touch the world-space box.
    fn each(&self, q: Aabb, mut f: impl FnMut(usize, &Group, u32)) {
        for (gi, g) in self.groups.iter().enumerate() {
            let lq = if gi == 0 { q } else { aabb_xf(q, &g.inv) };
            g.candidates(lq, &mut |i| {
                if self.enabled(g, i) {
                    f(gi, g, i)
                }
            });
        }
    }

    pub fn raycast(&self, origin: Vec3, dir: Vec3, max: f32) -> Option<RayHit> {
        self.raycast_filtered(origin, dir, max, |_| true)
    }

    /// Raycast that only considers shapes for which `keep(id)` is true.
    pub fn raycast_filtered(&self, origin: Vec3, dir: Vec3, max: f32, keep: impl Fn(ColliderId) -> bool) -> Option<RayHit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        let mut best: Option<RayHit> = None;
        for (gi, g) in self.groups.iter().enumerate() {
            let o = g.inv.transform_point3(origin);
            let d = g.inv.transform_vector3(dir);
            let mut test = |i: u32| {
                let id = ColliderId { group: gi as u32, index: i };
                if !self.enabled(g, i) || !keep(id) {
                    return;
                }
                if let Some((t, n)) = g.shapes[i as usize].raycast(o, d, max) {
                    if best.is_none_or(|h| t < h.distance) {
                        best = Some(RayHit { distance: t, point: origin + dir * t, normal: g.xf.transform_vector3(n), collider: id });
                    }
                }
            };
            if g.built_for == g.shapes.len() && !g.shapes.is_empty() {
                g.bvh.query_ray(o, d, max, &mut test);
            } else {
                (0..g.shapes.len() as u32).for_each(&mut test);
            }
        }
        best
    }

    pub fn overlap_sphere(&self, c: Vec3, r: f32) -> Vec<ColliderId> {
        let mut out = Vec::new();
        self.each(Aabb { min: c - Vec3::splat(r), max: c + Vec3::splat(r) }, |gi, g, i| {
            let lc = g.inv.transform_point3(c);
            if g.shapes[i as usize].dist_sq(lc) <= r * r {
                out.push(ColliderId { group: gi as u32, index: i });
            }
        });
        out
    }

    pub fn overlap_capsule(&self, cap: Capsule) -> Vec<ColliderId> {
        let mut out = Vec::new();
        self.each(cap.aabb(), |gi, g, i| {
            let (a, b) = (g.inv.transform_point3(cap.a), g.inv.transform_point3(cap.b));
            let (p, q) = g.shapes[i as usize].closest_to_segment(a, b);
            if p.distance_squared(q) <= cap.radius * cap.radius {
                out.push(ColliderId { group: gi as u32, index: i });
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
        self.each(q, |gi, g, i| {
            let s = &g.shapes[i as usize];
            let o = g.inv.transform_point3(origin);
            let d = g.inv.transform_vector3(dir);
            if s.dist_sq(o) <= r * r {
                return;
            }
            let mut t = 0.0f32;
            let mut hit_t = None;
            while t <= max {
                if s.dist_sq(o + d * t) <= r * r {
                    hit_t = Some(t);
                    break;
                }
                t += step;
            }
            let Some(mut hi) = hit_t else { return };
            let mut lo = (hi - step).max(0.0);
            for _ in 0..16 {
                let m = (lo + hi) * 0.5;
                if s.dist_sq(o + d * m) <= r * r {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            if best.is_none_or(|h| hi < h.distance) {
                let c = o + d * hi;
                let lp = s.closest_point(c);
                let ln = (c - lp).normalize_or(-d);
                best = Some(RayHit {
                    distance: hi,
                    point: g.xf.transform_point3(lp),
                    normal: g.xf.transform_vector3(ln),
                    collider: ColliderId { group: gi as u32, index: i },
                });
            }
        });
        best
    }

    /// Pushes a capsule out of every shape. Returns the contact normals so the caller can
    /// remove the velocity going into them (frictionless, zero-bounce contact, like
    /// the player NoFriction PhysicMaterial).
    pub fn depenetrate_capsule(&self, cap: &mut Capsule, normals: &mut Vec<Vec3>) {
        self.depenetrate_capsule_filtered(cap, normals, |_| true);
    }

    /// [`World::depenetrate_capsule`] against the shapes whose owner passes `keep`.
    pub fn depenetrate_capsule_filtered(&self, cap: &mut Capsule, normals: &mut Vec<Vec3>, keep: impl Fn(u32) -> bool) {
        let mut cands: Vec<(usize, u32)> = Vec::new();
        let margin = Vec3::splat(0.5);
        let q = cap.aabb();
        self.each(Aabb { min: q.min - margin, max: q.max + margin }, |gi, g, i| {
            if keep(g.owners[i as usize]) {
                cands.push((gi, i))
            }
        });
        for _ in 0..4 {
            let mut moved = false;
            for &(gi, i) in &cands {
                let g = &self.groups[gi];
                let s = &g.shapes[i as usize];
                let (a, b) = (g.inv.transform_point3(cap.a), g.inv.transform_point3(cap.b));
                let (p, q) = s.closest_to_segment(a, b);
                let d2 = p.distance_squared(q);
                if d2 >= cap.radius * cap.radius {
                    continue;
                }
                let (n, depth) = if d2 > 1e-10 {
                    let d = d2.sqrt();
                    ((p - q) / d, cap.radius - d)
                } else {
                    s.deep_push(p, (a + b) * 0.5, cap.radius)
                };
                let wn = g.xf.transform_vector3(n);
                let push = wn * (depth + 1e-4);
                cap.a += push;
                cap.b += push;
                normals.push(wn);
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
    fn capsule_shape_pushes_capsule_sideways() {
        let mut w = World::default();
        w.add_shape(0, Shape::Capsule(Capsule::unity(Vec3::new(0.0, 2.0, 0.0), 4.0, 0.7)), ALWAYS);
        let mut cap = Capsule::unity(Vec3::new(1.0, 1.75, 0.0), 3.5, 0.5);
        let mut n = vec![];
        w.depenetrate_capsule(&mut cap, &mut n);
        assert!((cap.a.x - 1.2).abs() < 1e-3 && cap.a.z.abs() < 1e-6, "{cap:?}");
        assert!(n[0].abs_diff_eq(Vec3::X, 1e-4));
        // sphere: ray from outside hits the surface, ray from inside misses
        let s = Capsule { a: Vec3::ZERO, b: Vec3::ZERO, radius: 2.0 };
        let (t, nn) = s.raycast(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, 10.0).unwrap();
        assert!((t - 3.0).abs() < 1e-4 && nn.abs_diff_eq(-Vec3::X, 1e-4));
        assert!(s.raycast(Vec3::ZERO, Vec3::X, 10.0).is_none());
        // capsule side
        let c = Capsule::unity(Vec3::new(0.0, 2.0, 0.0), 4.0, 0.7);
        let (t, _) = c.raycast(Vec3::new(-3.0, 2.5, 0.0), Vec3::X, 10.0).unwrap();
        assert!((t - 2.3).abs() < 1e-4);
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
