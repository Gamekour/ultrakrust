//! Unity's baked NavMeshData: Detour-derived tiles (magic "DNAV", version 16).
//!
//! Tile layout (little-endian), verified against every tile in the install by the size formula:
//!   header 72 B: magic, version, x, y, layer, polyCount, vertCount, detailMeshCount, detailVertCount,
//!                detailTriCount, bvNodeCount (u32 each), bmin[3], bmax[3], bvQuantFactor (f32)
//!   verts        12 B each: f32 x, y, z (Unity space)
//!   polys        32 B each: u16 verts[6], u16 neis[6], u32 flags, u8 vertCount, u8 area, u16 (unused)
//!                neis: 0 = border, 0x8000 | side = edge continues in the neighbouring tile, else 1-based poly
//!   detail mesh  12 B each: u32 vertBase, u32 triBase, u16 vertCount, u16 triCount
//!   detail verts 12 B each: f32 x, y, z
//!   detail tris   8 B each: u16 v0, v1, v2, edge flags (index < poly vertCount = poly vertex)
//!   bv nodes     16 B each: u16 bmin[3], u16 bmax[3], i32 poly (or -escape)
use crate::serialized::Value;
use bevy_math::{Quat, Vec3};

pub const MAGIC: u32 = 0x444e4156;
pub const VERSION: u32 = 16;
/// Neighbour flag: the edge continues into the adjacent tile on side `nei & 7`.
pub const EXT_LINK: u16 = 0x8000;

#[derive(Clone, Debug)]
pub struct NavPoly {
    pub verts: Vec<u16>,
    pub neis: Vec<u16>,
    pub flags: u32,
    pub area: u8,
}

#[derive(Clone, Debug)]
pub struct NavTile {
    pub x: i32,
    pub y: i32,
    pub layer: i32,
    /// Bevy space.
    pub verts: Vec<Vec3>,
    pub polys: Vec<NavPoly>,
    /// Per poly: triangles of the height detail mesh, Bevy space.
    pub detail: Vec<Vec<[Vec3; 3]>>,
    pub bmin: Vec3,
    pub bmax: Vec3,
}

#[derive(Clone, Debug)]
pub struct OffMeshLink {
    pub start: Vec3,
    pub end: Vec3,
    pub radius: f32,
    pub bidirectional: bool,
    pub area: u8,
}

#[derive(Clone, Debug)]
pub struct NavMeshData {
    pub name: String,
    pub agent_type: i64,
    pub agent_radius: f32,
    pub agent_height: f32,
    pub agent_climb: f32,
    pub tile_world_size: f32,
    pub tiles: Vec<NavTile>,
    pub links: Vec<OffMeshLink>,
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes(b[i..i + 2].try_into().unwrap())
}
fn f32_at(b: &[u8], i: usize) -> f32 {
    f32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}

/// Unity point (after the NavMeshData transform) -> Bevy point.
fn to_bevy(p: Vec3) -> Vec3 {
    Vec3::new(p.x, p.y, -p.z)
}

pub fn parse_tile(b: &[u8], rot: Quat, pos: Vec3) -> Option<NavTile> {
    if b.len() < 72 || u32_at(b, 0) != MAGIC || u32_at(b, 4) != VERSION {
        return None;
    }
    let h = |i: usize| u32_at(b, i * 4) as usize;
    let (poly_n, vert_n, dm_n, dv_n, dt_n, bv_n) = (h(5), h(6), h(7), h(8), h(9), h(10));
    if b.len() != 72 + vert_n * 12 + poly_n * 32 + dm_n * 12 + dv_n * 12 + dt_n * 8 + bv_n * 16 {
        return None;
    }
    let xf = |p: Vec3| to_bevy(rot * p + pos);
    let v3 = |o: usize| Vec3::new(f32_at(b, o), f32_at(b, o + 4), f32_at(b, o + 8));
    let mut o = 72;
    let verts: Vec<Vec3> = (0..vert_n).map(|i| xf(v3(o + i * 12))).collect();
    o += vert_n * 12;
    let polys: Vec<NavPoly> = (0..poly_n)
        .map(|i| {
            let p = o + i * 32;
            let n = b[p + 28] as usize;
            NavPoly {
                verts: (0..n).map(|k| u16_at(b, p + k * 2)).collect(),
                neis: (0..n).map(|k| u16_at(b, p + 12 + k * 2)).collect(),
                flags: u32_at(b, p + 24),
                area: b[p + 29],
            }
        })
        .collect();
    o += poly_n * 32;
    let dm = o;
    o += dm_n * 12;
    let dv: Vec<Vec3> = (0..dv_n).map(|i| xf(v3(o + i * 12))).collect();
    o += dv_n * 12;
    let dt = o;
    let detail = (0..poly_n)
        .map(|pi| {
            if pi >= dm_n {
                return Vec::new();
            }
            let m = dm + pi * 12;
            let (vb, tb, tc) = (u32_at(b, m) as usize, u32_at(b, m + 4) as usize, u16_at(b, m + 10) as usize);
            let poly = &polys[pi];
            let vtx = |k: u16| -> Vec3 {
                let k = k as usize;
                if k < poly.verts.len() { verts[poly.verts[k] as usize] } else { dv.get(vb + k - poly.verts.len()).copied().unwrap_or(Vec3::ZERO) }
            };
            (0..tc)
                .filter_map(|t| {
                    let q = dt + (tb + t) * 8;
                    (q + 8 <= b.len()).then(|| [vtx(u16_at(b, q)), vtx(u16_at(b, q + 2)), vtx(u16_at(b, q + 4))])
                })
                .collect()
        })
        .collect();
    let (bmin, bmax) = (xf(v3(44)), xf(v3(56)));
    Some(NavTile {
        x: u32_at(b, 8) as i32,
        y: u32_at(b, 12) as i32,
        layer: u32_at(b, 16) as i32,
        verts,
        polys,
        detail,
        bmin: bmin.min(bmax),
        bmax: bmin.max(bmax),
    })
}

/// Decodes a NavMeshData object (class 238).
pub fn parse(v: &Value) -> NavMeshData {
    let r = v.get("m_Rotation").quat();
    let rot = if r == [0.0; 4] { Quat::IDENTITY } else { Quat::from_xyzw(r[0], r[1], r[2], r[3]).normalize() };
    let pos = Vec3::from(v.get("m_Position").vec3());
    let s = v.get("m_NavMeshBuildSettings");
    let tile_size = s.get("tileSize").f32().max(1.0) * s.get("cellSize").f32();
    let tiles = v.get("m_NavMeshTiles").array().iter().filter_map(|t| parse_tile(t.get("m_MeshData").bytes(), rot, pos)).collect();
    let links = v
        .get("m_OffMeshLinks")
        .array()
        .iter()
        .map(|l| OffMeshLink {
            start: to_bevy(rot * Vec3::from(l.get("m_Start").vec3()) + pos),
            end: to_bevy(rot * Vec3::from(l.get("m_End").vec3()) + pos),
            radius: l.get("m_Radius").f32(),
            // Unity: m_LinkDirection bit 0 = one way? Detour stores DT_OFFMESH_CON_BIDIR; Unity serialises 1 = bidirectional.
            bidirectional: l.get("m_LinkDirection").i64() & 1 != 0,
            area: l.get("m_Area").i64() as u8,
        })
        .collect();
    NavMeshData {
        name: v.get("m_Name").str().to_string(),
        agent_type: s.get("agentTypeID").i64(),
        agent_radius: s.get("agentRadius").f32(),
        agent_height: s.get("agentHeight").f32(),
        agent_climb: s.get("agentClimb").f32(),
        tile_world_size: tile_size,
        tiles,
        links,
    }
}

/// Every NavMeshData object in a scene bundle.
pub fn load_scene_navmeshes(db: &mut crate::db::AssetDb, bundle: &std::path::Path) -> crate::Result<Vec<NavMeshData>> {
    let mut out = Vec::new();
    for f in db.load_bundle_files(bundle)? {
        for o in f.objects.iter().filter(|o| o.class_id == 238) {
            out.push(parse(&f.read(o)?));
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
