//! Navigation on the level's own baked NavMesh (see `uk_assets::navmesh`): one polygon graph over all
//! tiles, nearest-point queries (Unity `NavMesh.SamplePosition`), A* over polygons with Unity's
//! partial-path behaviour (head for the reachable polygon closest to the goal), and funnel
//! string-pulling into corner points (what `NavMeshAgent.steeringTarget` walks through).
use bevy_math::{Vec2, Vec3};
use std::collections::{BinaryHeap, HashMap};
use uk_assets::navmesh::{NavMeshData, EXT_LINK};

#[derive(Clone, Debug)]
pub struct Link {
    pub to: u32,
    /// Portal as (left, right) seen when leaving this polygon. Both equal the take-off point for off-mesh links.
    pub a: Vec3,
    pub b: Vec3,
    /// Off-mesh link (e.g. a baked drop-down): where it lands.
    pub offmesh: Option<Vec3>,
}

#[derive(Clone, Debug)]
pub struct Poly {
    pub verts: Vec<Vec3>,
    pub detail: Vec<[Vec3; 3]>,
    pub center: Vec3,
    pub bmin: Vec3,
    pub bmax: Vec3,
    pub links: Vec<Link>,
    pub area: u8,
}

/// One point of a path. `offmesh`: the agent reaches it by an off-mesh link (a drop or jump), not by walking.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Corner {
    pub pos: Vec3,
    pub offmesh: bool,
}

pub struct NavGraph {
    pub polys: Vec<Poly>,
    pub agent_radius: f32,
    pub agent_height: f32,
    pub agent_climb: f32,
    grid: HashMap<(i32, i32), Vec<u32>>,
    pub external_edges: usize,
    pub external_matched: usize,
}

const CELL: f32 = 8.0;

fn xz(v: Vec3) -> Vec2 {
    Vec2::new(v.x, v.z)
}

fn tri_area2(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (c.x - a.x) * (b.z - a.z) - (b.x - a.x) * (c.z - a.z)
}

/// Height on triangle `t` at the xz of `p`, if `p` lies inside it (xz).
fn tri_height(t: &[Vec3; 3], p: Vec3) -> Option<f32> {
    let (a, b, c) = (xz(t[0]), xz(t[1]), xz(t[2]));
    let q = xz(p);
    let v0 = c - a;
    let v1 = b - a;
    let v2 = q - a;
    let den = v0.perp_dot(v1);
    if den.abs() < 1e-9 {
        return None;
    }
    let u = v2.perp_dot(v1) / den;
    let v = v0.perp_dot(v2) / den;
    const EPS: f32 = 1e-4;
    (u >= -EPS && v >= -EPS && u + v <= 1.0 + EPS).then(|| t[0].y + (t[2].y - t[0].y) * u + (t[1].y - t[0].y) * v)
}

fn closest_on_segment(p: Vec3, a: Vec3, b: Vec3) -> Vec3 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    a + ab * t
}

impl NavGraph {
    pub fn build(meshes: &[NavMeshData]) -> Option<Self> {
        // the humanoid agent type (0) is the one Filth, Strays and most enemies use
        let m = meshes.iter().filter(|m| m.agent_type == 0).max_by_key(|m| m.tiles.len())?;
        let mut polys = Vec::new();
        // external edge: (poly, a, b, side)
        let mut ext: Vec<(u32, Vec3, Vec3, u16)> = Vec::new();
        for t in &m.tiles {
            let base = polys.len() as u32;
            for (pi, p) in t.polys.iter().enumerate() {
                let verts: Vec<Vec3> = p.verts.iter().map(|&v| t.verts[v as usize]).collect();
                let center = verts.iter().copied().sum::<Vec3>() / verts.len().max(1) as f32;
                let (mut bmin, mut bmax) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                for &v in &verts {
                    bmin = bmin.min(v);
                    bmax = bmax.max(v);
                }
                let mut links = Vec::new();
                let n = verts.len();
                for (k, &nei) in p.neis.iter().enumerate() {
                    let (a, b) = (verts[k], verts[(k + 1) % n]);
                    if nei & EXT_LINK != 0 {
                        ext.push((base + pi as u32, a, b, nei & 7));
                    } else if nei != 0 {
                        links.push(Link { to: base + nei as u32 - 1, a, b, offmesh: None });
                    }
                }
                let detail = t.detail.get(pi).cloned().unwrap_or_default();
                polys.push(Poly { verts, detail, center, bmin, bmax, links, area: p.area });
            }
        }
        // Join tile borders: edges on the same boundary line from opposite sides, overlapping in extent.
        let mut by_line: HashMap<(u8, i64), Vec<usize>> = HashMap::new();
        for (i, &(_, a, b, side)) in ext.iter().enumerate() {
            let along_x = side == 2 || side == 6; // boundary at constant z (Unity z = -Bevy z), edge runs along x
            let c = if along_x { a.z } else { a.x };
            if (if along_x { b.z } else { b.x } - c).abs() > 0.01 {
                continue;
            }
            by_line.entry((along_x as u8, (c * 64.0).round() as i64)).or_default().push(i);
        }
        let mut matched = 0;
        let mut keys: Vec<_> = by_line.keys().copied().collect();
        keys.sort_unstable();
        for key in keys {
            let ids = &by_line[&key];
            let along_x = key.0 == 1;
            for &i in ids {
                let (pi, a, b, side) = ext[i];
                let mut any = false;
                for &j in ids {
                    let (pj, c, d, side_j) = ext[j];
                    if pi == pj || (side + 4) % 8 != side_j {
                        continue;
                    }
                    let s = |v: Vec3| if along_x { v.x } else { v.z };
                    let (lo, hi) = (s(a).min(s(b)).max(s(c).min(s(d))), s(a).max(s(b)).min(s(c).max(s(d))));
                    if hi - lo < 0.01 {
                        continue;
                    }
                    let at = |p: Vec3, q: Vec3, v: f32| p + (q - p) * ((v - s(p)) / (s(q) - s(p)));
                    // heights must agree at both ends of the overlap
                    let climb = m.agent_climb.max(0.5);
                    if (at(a, b, lo).y - at(c, d, lo).y).abs() > climb || (at(a, b, hi).y - at(c, d, hi).y).abs() > climb {
                        continue;
                    }
                    // keep the polygon's own edge order (a -> b): the funnel relies on it for left/right
                    let (pa, pb) = if s(a) <= s(b) { (at(a, b, lo), at(a, b, hi)) } else { (at(a, b, hi), at(a, b, lo)) };
                    polys[pi as usize].links.push(Link { to: pj, a: pa, b: pb, offmesh: None });
                    any = true;
                }
                matched += any as usize;
            }
        }
        let mut g = NavGraph {
            polys,
            agent_radius: m.agent_radius,
            agent_height: m.agent_height,
            agent_climb: m.agent_climb,
            grid: HashMap::new(),
            external_edges: ext.len(),
            external_matched: matched,
        };
        for (i, p) in g.polys.iter().enumerate() {
            for cx in (p.bmin.x / CELL).floor() as i32..=(p.bmax.x / CELL).floor() as i32 {
                for cz in (p.bmin.z / CELL).floor() as i32..=(p.bmax.z / CELL).floor() as i32 {
                    g.grid.entry((cx, cz)).or_default().push(i as u32);
                }
            }
        }
        for l in &m.links {
            let ext = Vec3::new(l.radius.max(0.5), 2.0, l.radius.max(0.5));
            let (Some((s, sp)), Some((e, ep))) = (g.nearest(l.start, ext), g.nearest(l.end, ext)) else { continue };
            g.polys[s as usize].links.push(Link { to: e, a: sp, b: sp, offmesh: Some(ep) });
            if l.bidirectional {
                g.polys[e as usize].links.push(Link { to: s, a: ep, b: ep, offmesh: Some(sp) });
            }
        }
        Some(g)
    }

    /// Closest point on polygon `i` to `p`.
    pub fn closest_on_poly(&self, i: u32, p: Vec3) -> Vec3 {
        let poly = &self.polys[i as usize];
        for t in &poly.detail {
            if let Some(h) = tri_height(t, p) {
                return Vec3::new(p.x, h, p.z);
            }
        }
        let n = poly.verts.len();
        let mut best = poly.center;
        let mut bd = f32::MAX;
        for k in 0..n {
            let c = closest_on_segment(p, poly.verts[k], poly.verts[(k + 1) % n]);
            let d = c.distance_squared(p);
            if d < bd {
                bd = d;
                best = c;
            }
        }
        best
    }

    /// Nearest polygon within the box `p ± half_ext` (Detour `findNearestPoly`).
    pub fn nearest(&self, p: Vec3, half_ext: Vec3) -> Option<(u32, Vec3)> {
        let (lo, hi) = (p - half_ext, p + half_ext);
        let mut best: Option<(f32, u32, Vec3)> = None;
        for cx in (lo.x / CELL).floor() as i32..=(hi.x / CELL).floor() as i32 {
            for cz in (lo.z / CELL).floor() as i32..=(hi.z / CELL).floor() as i32 {
                let Some(ids) = self.grid.get(&(cx, cz)) else { continue };
                for &i in ids {
                    let poly = &self.polys[i as usize];
                    if poly.bmax.x < lo.x || poly.bmin.x > hi.x || poly.bmax.z < lo.z || poly.bmin.z > hi.z
                        || poly.bmax.y < lo.y - 0.5 || poly.bmin.y > hi.y + 0.5 {
                        continue;
                    }
                    let c = self.closest_on_poly(i, p);
                    let d = c.distance_squared(p);
                    if best.is_none_or(|b| d < b.0 || (d == b.0 && i < b.1)) {
                        best = Some((d, i, c));
                    }
                }
            }
        }
        best.map(|(_, i, c)| (i, c))
    }

    /// Unity `NavMesh.SamplePosition(pos, out hit, max_distance, ...)`.
    pub fn sample_position(&self, p: Vec3, max_distance: f32) -> Option<Vec3> {
        self.nearest(p, Vec3::splat(max_distance)).map(|(_, c)| c).filter(|c| c.distance(p) <= max_distance)
    }

    /// Path corners from `start` to `end`. `None` if `start` is not on the navmesh. When `end` is
    /// unreachable the path ends at the closest reachable point, as Unity's partial paths do.
    pub fn find_path(&self, start: Vec3, end: Vec3) -> Option<(Vec<Corner>, bool)> {
        let (sp, target, portals, reached) = self.corridor(start, end)?;
        self.pull(sp, target, &portals, reached)
    }

    /// The A* polygon corridor: (start point, end point, portals crossed in order, goal reached).
    pub fn corridor(&self, start: Vec3, end: Vec3) -> Option<(Vec3, Vec3, Vec<Link>, bool)> {
        let ext = Vec3::new(2.0, 4.0, 2.0);
        let (s, sp) = self.nearest(start, ext)?;
        let goal = self.nearest(end, ext);
        let (e, ep) = goal.unwrap_or((u32::MAX, end));
        // A* over polygons, node position = entry point (portal midpoint)
        #[derive(PartialEq)]
        struct N(f32, u32);
        impl Eq for N {}
        impl PartialOrd for N {
            fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
                Some(self.cmp(o))
            }
        }
        impl Ord for N {
            fn cmp(&self, o: &Self) -> std::cmp::Ordering {
                o.0.total_cmp(&self.0).then(o.1.cmp(&self.1))
            }
        }
        let mut cost: HashMap<u32, (f32, Vec3, u32, usize)> = HashMap::new(); // poly -> (g, pos, parent, link idx)
        cost.insert(s, (0.0, sp, u32::MAX, 0));
        let mut open = BinaryHeap::new();
        open.push(N(sp.distance(ep), s));
        let mut closest = (sp.distance(ep), s);
        let mut reached = false;
        let mut expanded = 0;
        while let Some(N(_, cur)) = open.pop() {
            expanded += 1;
            if cur == e {
                reached = true;
                break;
            }
            if expanded > 20000 {
                break;
            }
            let (g, pos, _, _) = cost[&cur];
            for (li, l) in self.polys[cur as usize].links.iter().enumerate() {
                let np = (l.a + l.b) * 0.5;
                let ng = g + pos.distance(np);
                if cost.get(&l.to).is_none_or(|c| ng < c.0) {
                    cost.insert(l.to, (ng, np, cur, li));
                    let h = np.distance(ep);
                    if h < closest.0 {
                        closest = (h, l.to);
                    }
                    open.push(N(ng + h, l.to));
                }
            }
        }
        let last = if reached { e } else { closest.1 };
        let target = if reached { ep } else { self.closest_on_poly(last, end) };
        // walk back the chain of portals
        let mut portals: Vec<Link> = Vec::new();
        let mut n = last;
        while n != s {
            let (_, _, parent, li) = cost[&n];
            portals.push(self.polys[parent as usize].links[li].clone());
            n = parent;
        }
        portals.reverse();
        Some((sp, target, portals, reached))
    }

    fn pull(&self, sp: Vec3, target: Vec3, portals: &[Link], reached: bool) -> Option<(Vec<Corner>, bool)> {
        // funnel each stretch between off-mesh links; a link is taken as a straight hop
        let mut corners = vec![Corner { pos: sp, offmesh: false }];
        let mut from = sp;
        let mut run: Vec<(Vec3, Vec3)> = Vec::new();
        for l in portals {
            match l.offmesh {
                Some(land) => {
                    corners.extend(self.string_pull(from, l.a, &run).into_iter().skip(1).map(|pos| Corner { pos, offmesh: false }));
                    corners.push(Corner { pos: land, offmesh: true });
                    from = land;
                    run.clear();
                }
                // Detour gives (left, right) = (edge start, edge end) in Unity space; mirroring z swaps them.
                None => run.push((l.b, l.a)),
            }
        }
        corners.extend(self.string_pull(from, target, &run).into_iter().skip(1).map(|pos| Corner { pos, offmesh: false }));
        corners.dedup_by(|a, b| a.pos.distance_squared(b.pos) < 1e-6 && !a.offmesh);
        Some((corners, reached))
    }

    /// Simple stupid funnel over (left, right) portals.
    fn string_pull(&self, start: Vec3, end: Vec3, portals: &[(Vec3, Vec3)]) -> Vec<Vec3> {
        let mut pts: Vec<(Vec3, Vec3)> = Vec::with_capacity(portals.len() + 2);
        pts.push((start, start));
        pts.extend_from_slice(portals);
        pts.push((end, end));
        let mut path = vec![start];
        let (mut apex, mut left, mut right) = (start, start, start);
        let (mut li, mut ri) = (0usize, 0usize);
        let mut ai;
        let mut i = 1;
        while i < pts.len() {
            let (pl, pr) = pts[i];
            // right side
            if tri_area2(apex, right, pr) <= 0.0 {
                if apex == right || tri_area2(apex, left, pr) > 0.0 {
                    right = pr;
                    ri = i;
                } else {
                    path.push(left);
                    apex = left;
                    ai = li;
                    (left, right) = (apex, apex);
                    (li, ri) = (ai, ai);
                    i = ai + 1;
                    continue;
                }
            }
            // left side
            if tri_area2(apex, left, pl) >= 0.0 {
                if apex == left || tri_area2(apex, right, pl) < 0.0 {
                    left = pl;
                    li = i;
                } else {
                    path.push(right);
                    apex = right;
                    ai = ri;
                    (left, right) = (apex, apex);
                    (li, ri) = (ai, ai);
                    i = ai + 1;
                    continue;
                }
            }
            i += 1;
        }
        if path.last() != Some(&end) {
            path.push(end);
        }
        path.dedup_by(|a, b| a.distance_squared(*b) < 1e-6);
        path
    }

    /// Number of connected components (diagnostics).
    pub fn components(&self) -> usize {
        let mut seen = vec![false; self.polys.len()];
        let mut n = 0;
        for s in 0..self.polys.len() {
            if seen[s] {
                continue;
            }
            n += 1;
            let mut stack = vec![s];
            seen[s] = true;
            while let Some(c) = stack.pop() {
                for l in &self.polys[c].links {
                    if !seen[l.to as usize] {
                        seen[l.to as usize] = true;
                        stack.push(l.to as usize);
                    }
                }
            }
        }
        n
    }
}
