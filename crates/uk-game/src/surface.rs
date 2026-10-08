//! SceneHelper's footstep physics scene: copies of the environment's renderable colliders
//! (`SceneDef::surface_meshes`) raycast single-sided (PhysicsManager queriesHitBackfaces is
//! off) to find the material, and from it the SurfaceType and particle color, under a point.

use bevy_math::Vec3;
use uk_assets::scenedef::SceneDef;
use uk_core::collide::{Shape, Triangle, World};

pub struct Surface {
    world: World,
    /// shape index -> (surface mesh, triangle)
    tris: Vec<(u32, u32)>,
}

/// SceneHelper.HitSurfaceData (Bevy space).
#[derive(Clone, Copy, Debug)]
pub struct SurfaceHit {
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
    pub surface: i32,
    pub color: [f32; 4],
    pub secondary_blend: bool,
    pub node: u32,
}

impl Surface {
    pub fn new(def: &SceneDef) -> Self {
        let mut world = World::default();
        let mut tris = Vec::new();
        for (mi, m) in def.surface_meshes.iter().enumerate() {
            for (ti, t) in m.tris.iter().enumerate() {
                world.add_shape(0, Shape::Tri(Triangle::new(t[0], t[1], t[2])), m.node);
                tris.push((mi as u32, ti as u32));
            }
        }
        world.build();
        Self { world, tris }
    }

    /// SceneHelper.TryGetSurfaceData(pos, direction, distance): the nearest front face of an
    /// active copy, its material (none: no data) and the surface type / color it carries.
    pub fn data(&self, def: &SceneDef, active: &[bool], pos: Vec3, dir: Vec3, dist: f32) -> Option<SurfaceHit> {
        let d = dir.normalize_or_zero();
        let h = self.world.raycast_filtered(pos, d, dist, |id| {
            let node = self.world.owner(id);
            // the Bevy-space winding is mirrored: Unity's front face normal is -n here
            let Shape::Tri(t) = self.world.get(id) else { return false };
            active.get(node as usize).copied().unwrap_or(false) && d.dot(t.n) > 0.0
        })?;
        let (mi, ti) = self.tris[h.collider.index as usize];
        let m = &def.surface_meshes[mi as usize];
        let mat = m.mats[m.tri_mat[ti as usize]? as usize];
        let mut secondary = false;
        if mat.vertex_blending && !m.tri_r.is_empty() {
            let [a, b, c] = m.tris[ti as usize];
            let w = barycentric(a, b, c, h.point);
            let r = m.tri_r[ti as usize];
            secondary = r[0] * w.x + r[1] * w.y + r[2] * w.z < 0.5;
        }
        let (surface, color) = match (mat.has_surface, secondary) {
            (false, _) => (0, [1.0; 4]),
            (true, false) => (mat.surface, mat.color),
            (true, true) => (mat.secondary, mat.secondary_color),
        };
        Some(SurfaceHit { point: h.point, normal: h.normal, distance: h.distance, surface, color, secondary_blend: secondary, node: m.node })
    }
}

fn barycentric(a: Vec3, b: Vec3, c: Vec3, p: Vec3) -> Vec3 {
    let (v0, v1, v2) = (b - a, c - a, p - a);
    let (d00, d01, d11, d20, d21) = (v0.dot(v0), v0.dot(v1), v1.dot(v1), v2.dot(v0), v2.dot(v1));
    let den = d00 * d11 - d01 * d01;
    if den.abs() < 1e-12 {
        return Vec3::new(1.0, 0.0, 0.0);
    }
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    Vec3::new(1.0 - v - w, v, w)
}
