//! Level extraction: walks a scene's GameObject/Transform hierarchy and bakes
//! renderers and environment colliders into world space (converted to Bevy's
//! right-handed coordinates: Unity z → -z, triangle winding reversed).

use crate::db::AssetDb;
use crate::mesh::{self, MeshData};
use crate::serialized::{SerializedFile, Value};
use crate::Result;
use bevy_math::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;

const CLASS_GAMEOBJECT: i32 = 1;
const CLASS_TRANSFORM: i32 = 4;
const CLASS_MESH_RENDERER: i32 = 23;
const CLASS_MESH_FILTER: i32 = 33;
const CLASS_MESH_COLLIDER: i32 = 64;
const CLASS_BOX_COLLIDER: i32 = 65;
const CLASS_RECT_TRANSFORM: i32 = 224;

/// Layers whose renderers are level art.
const RENDER_LAYERS: &[i64] = &[0, 4, 6, 7, 8, 17, 24, 25];
/// LMD.Environment (6, 8, 24) plus Default.
const COLLIDE_LAYERS: &[i64] = &[0, 6, 8, 24];

#[derive(Clone, Debug)]
pub struct LevelOptions {
    /// ULTRAKILL enables rooms through triggers as you progress; force inactive root rooms on.
    pub force_root_rooms: bool,
    /// Root names containing any of these stay off (unused alternates, kill volumes).
    pub exclude_roots: Vec<String>,
}

impl Default for LevelOptions {
    fn default() -> Self {
        Self {
            force_root_rooms: true,
            exclude_roots: ["Alt", "OLD", "OutOfBounds", "Directional Light", "Fog"].map(String::from).to_vec(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MaterialKey {
    pub file: String,
    pub path_id: i64,
}

#[derive(Default, Debug, Clone)]
pub struct Batch {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    /// Vertex colors (white when the mesh has none).
    pub colors: Vec<[f32; 4]>,
    pub indices: Vec<u32>,
    /// Source mesh vertex of each batch vertex (re-skinning writes through this).
    pub src: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct BoxDesc {
    pub center: Vec3,
    pub half: Vec3,
    pub rot: Quat,
}

#[derive(Default, Debug)]
pub struct LevelStats {
    pub game_objects: usize,
    pub active_renderers: usize,
    pub static_batched: usize,
    pub meshes_decoded: usize,
    pub mesh_colliders: usize,
    pub box_colliders: usize,
    pub skipped: Vec<String>,
}

#[derive(Default)]
pub struct Level {
    pub batches: HashMap<Option<MaterialKey>, Batch>,
    pub collision_tris: Vec<[Vec3; 3]>,
    pub boxes: Vec<BoxDesc>,
    pub spawn: Option<(Vec3, f32)>,
    /// Root objects ("rooms") with the bounds of their baked render geometry.
    pub rooms: Vec<Room>,
    pub stats: LevelStats,
}

#[derive(Clone, Debug)]
pub struct Room {
    pub name: String,
    pub min: Vec3,
    pub max: Vec3,
}

struct Go {
    name: String,
    layer: i64,
    active: bool,
    components: Vec<(i32, i64)>,
    transform: i64,
}

struct Tr {
    go: i64,
    pos: Vec3,
    rot: Quat,
    scale: Vec3,
    father: i64,
}

pub fn to_bevy_point(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, -v.z)
}

pub fn to_bevy_quat(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.y, q.z, q.w)
}

fn v3(v: &Value) -> Vec3 {
    Vec3::from(v.vec3())
}

fn quat(v: &Value) -> Quat {
    let q = v.quat();
    Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize()
}

struct Ctx<'a> {
    db: &'a mut AssetDb,
    scene: Arc<SerializedFile>,
    gos: HashMap<i64, Go>,
    trs: HashMap<i64, Tr>,
    active: HashMap<i64, bool>,
    world: HashMap<i64, Mat4>,
    meshes: HashMap<(String, i64), Option<Arc<MeshData>>>,
    opts: LevelOptions,
}

impl Ctx<'_> {
    fn active_in_hierarchy(&mut self, tr: i64) -> bool {
        if let Some(&a) = self.active.get(&tr) {
            return a;
        }
        let Some(t) = self.trs.get(&tr) else { return false };
        let (go, father) = (t.go, t.father);
        let g = &self.gos[&go];
        let mut self_active = g.active;
        if father == 0 && !self_active && self.opts.force_root_rooms {
            self_active = !self.opts.exclude_roots.iter().any(|x| g.name.contains(x.as_str()));
        }
        if father == 0 && self.opts.exclude_roots.iter().any(|x| g.name.contains(x.as_str())) {
            self_active = false;
        }
        let a = self_active && (father == 0 || self.active_in_hierarchy(father));
        self.active.insert(tr, a);
        a
    }

    fn root_name(&self, mut tr: i64) -> String {
        while let Some(t) = self.trs.get(&tr) {
            if t.father == 0 {
                return self.gos.get(&t.go).map(|g| g.name.clone()).unwrap_or_default();
            }
            tr = t.father;
        }
        String::new()
    }

    fn world_matrix(&mut self, tr: i64) -> Mat4 {
        if let Some(m) = self.world.get(&tr) {
            return *m;
        }
        let Some(t) = self.trs.get(&tr) else { return Mat4::IDENTITY };
        let local = Mat4::from_scale_rotation_translation(t.scale, t.rot, t.pos);
        let father = t.father;
        let m = if father == 0 { local } else { self.world_matrix(father) * local };
        self.world.insert(tr, m);
        m
    }

    fn mesh(&mut self, pptr: (i32, i64)) -> Option<Arc<MeshData>> {
        let scene = self.scene.clone();
        let (file, id) = self.db.resolve(&scene, pptr).ok()??;
        let key = (file.name.clone(), id);
        if let Some(m) = self.meshes.get(&key) {
            return m.clone();
        }
        let decoded = file.read_id(id).ok().and_then(|v| {
            let sd = v.get("m_StreamData");
            let path = sd.get("path").str().to_string();
            let stream = if !path.is_empty() && sd.get("size").i64() > 0 {
                let res = self.db.resource(path.rsplit('/').next().unwrap_or(&path))?;
                let off = sd.get("offset").i64() as usize;
                let size = sd.get("size").i64() as usize;
                Some(res[off..off + size].to_vec())
            } else {
                None
            };
            mesh::decode(&v, stream.as_deref()).ok().map(Arc::new)
        });
        self.meshes.insert(key, decoded.clone());
        decoded
    }

    fn component(&self, go: &Go, class: i32) -> Option<(i64, Value)> {
        for &(f, id) in &go.components {
            if f != 0 {
                continue;
            }
            let Some(o) = self.scene.object(id) else { continue };
            if o.class_id == class {
                return self.scene.read(o).ok().map(|v| (id, v));
            }
        }
        None
    }
}

/// Extracts render batches and collision from a level bundle (e.g. `campaign_scenes_level0-1.bundle`).
pub fn load_level(db: &mut AssetDb, bundle: &std::path::Path, opts: LevelOptions) -> Result<Level> {
    let files = db.load_bundle_files(bundle)?;
    let scene = files
        .iter()
        .find(|f| !f.name.contains('.'))
        .cloned()
        .ok_or_else(|| crate::Error("no scene file in bundle".into()))?;

    let mut gos = HashMap::new();
    let mut trs = HashMap::new();
    for o in &scene.objects {
        match o.class_id {
            CLASS_GAMEOBJECT => {
                let v = scene.read(o)?;
                let components: Vec<(i32, i64)> = v.get("m_Component").array().iter().map(|c| c.get("component").pptr()).collect();
                gos.insert(
                    o.path_id,
                    Go {
                        name: v.get("m_Name").str().to_string(),
                        layer: v.get("m_Layer").i64(),
                        active: v.get("m_IsActive").bool(),
                        components,
                        transform: 0,
                    },
                );
            }
            CLASS_TRANSFORM | CLASS_RECT_TRANSFORM => {
                let v = scene.read(o)?;
                trs.insert(
                    o.path_id,
                    Tr {
                        go: v.get("m_GameObject").pptr().1,
                        pos: v3(v.get("m_LocalPosition")),
                        rot: quat(v.get("m_LocalRotation")),
                        scale: v3(v.get("m_LocalScale")),
                        father: v.get("m_Father").pptr().1,
                    },
                );
            }
            _ => {}
        }
    }
    for (&tid, t) in &trs {
        if let Some(g) = gos.get_mut(&t.go) {
            g.transform = tid;
        }
    }

    let mut level = Level::default();
    level.stats.game_objects = gos.len();
    let mut ctx = Ctx { db, scene: scene.clone(), gos, trs, active: HashMap::new(), world: HashMap::new(), meshes: HashMap::new(), opts };

    let mut rooms: HashMap<String, (Vec3, Vec3)> = HashMap::new();
    let mut go_ids: Vec<i64> = ctx.gos.keys().copied().collect();
    go_ids.sort_unstable(); // deterministic order across processes
    for gid in go_ids {
        let tr = ctx.gos[&gid].transform;
        if tr == 0 || !ctx.active_in_hierarchy(tr) {
            continue;
        }
        let (layer, name) = (ctx.gos[&gid].layer, ctx.gos[&gid].name.clone());
        let world = ctx.world_matrix(tr);

        if name == "Player" && level.spawn.is_none() {
            let (_, rot, pos) = world.to_scale_rotation_translation();
            let fwd = rot * Vec3::Z;
            level.spawn = Some((to_bevy_point(pos), fwd.x.atan2(fwd.z).to_degrees()));
        }

        let go = &ctx.gos[&gid];
        // --- rendering ---
        if RENDER_LAYERS.contains(&layer) {
            if let Some((_, r)) = ctx.component(go, CLASS_MESH_RENDERER) {
                let filter = ctx.component(go, CLASS_MESH_FILTER);
                if r.get("m_Enabled").bool() && filter.is_some() {
                    let mesh_ptr = filter.unwrap().1.get("m_Mesh").pptr();
                    let sb = r.get("m_StaticBatchInfo");
                    let (first, count) = (sb.get("firstSubMesh").i64() as usize, sb.get("subMeshCount").i64() as usize);
                    let mats: Vec<(i32, i64)> = r.get("m_Materials").array().iter().map(|m| m.pptr()).collect();
                    let root = r.get("m_StaticBatchRoot").pptr();
                    if let Some(mesh) = ctx.mesh(mesh_ptr) {
                        level.stats.active_renderers += 1;
                        // Static-batched renderers draw a slice of a combined world-space mesh.
                        let (m, range) = if count > 0 {
                            level.stats.static_batched += 1;
                            let m = if root.1 != 0 {
                                let rt = ctx.gos.get(&root.1).map(|g| g.transform).unwrap_or(0);
                                if rt != 0 { ctx.world_matrix(rt) } else { Mat4::IDENTITY }
                            } else {
                                Mat4::IDENTITY
                            };
                            (m, first..first + count)
                        } else {
                            (world, 0..mesh.submeshes.len())
                        };
                        for (k, si) in range.enumerate() {
                            let Some(sm) = mesh.submeshes.get(si) else { continue };
                            let mat = mats.get(k).or(mats.last()).copied();
                            let key = mat.and_then(|p| {
                                ctx.db.resolve(&scene, p).ok().flatten().map(|(f, id)| MaterialKey { file: f.name.clone(), path_id: id })
                            });
                            let b = level.batches.entry(key).or_default();
                            let before = b.positions.len();
                            bake(b, &mesh, &sm.indices, m);
                            let room = rooms.entry(ctx.root_name(tr)).or_insert((Vec3::MAX, Vec3::MIN));
                            for p in &b.positions[before..] {
                                room.0 = room.0.min(Vec3::from(*p));
                                room.1 = room.1.max(Vec3::from(*p));
                            }
                        }
                    } else {
                        level.stats.skipped.push(format!("mesh for {name}"));
                    }
                }
            }
        }
        // --- collision ---
        if COLLIDE_LAYERS.contains(&layer) {
            let go = &ctx.gos[&gid];
            if let Some((_, c)) = ctx.component(go, CLASS_MESH_COLLIDER) {
                if c.get("m_Enabled").bool() && !c.get("m_IsTrigger").bool() {
                    if let Some(mesh) = ctx.mesh(c.get("m_Mesh").pptr()) {
                        level.stats.mesh_colliders += 1;
                        for sm in &mesh.submeshes {
                            for t in sm.indices.chunks_exact(3) {
                                let p = |i: u32| to_bevy_point(world.transform_point3(Vec3::from(mesh.positions[i as usize])));
                                // reversed winding after the handedness flip
                                level.collision_tris.push([p(t[0]), p(t[2]), p(t[1])]);
                            }
                        }
                    }
                }
            }
            let go = &ctx.gos[&gid];
            if let Some((_, c)) = ctx.component(go, CLASS_BOX_COLLIDER) {
                if c.get("m_Enabled").bool() && !c.get("m_IsTrigger").bool() {
                    level.stats.box_colliders += 1;
                    let (scale, rot, _) = world.to_scale_rotation_translation();
                    let center = world.transform_point3(v3(c.get("m_Center")));
                    level.boxes.push(BoxDesc {
                        center: to_bevy_point(center),
                        half: (v3(c.get("m_Size")) * scale).abs() * 0.5,
                        rot: to_bevy_quat(rot),
                    });
                }
            }
        }
    }
    level.stats.meshes_decoded = ctx.meshes.values().filter(|m| m.is_some()).count();
    let mut rooms: Vec<Room> = rooms.into_iter().filter(|(_, (lo, hi))| lo.x <= hi.x).map(|(name, (min, max))| Room { name, min, max }).collect();
    rooms.sort_by(|a, b| a.name.cmp(&b.name));
    level.rooms = rooms;
    Ok(level)
}

pub(crate) fn bake(b: &mut Batch, mesh: &MeshData, indices: &[u32], m: Mat4) {
    let flip = m.determinant() < 0.0;
    let nm = m.inverse().transpose();
    let mut remap: HashMap<u32, u32> = HashMap::new();
    let mut local = Vec::with_capacity(indices.len());
    for &i in indices {
        let next = b.positions.len() as u32;
        let idx = *remap.entry(i).or_insert_with(|| {
            let p = Vec3::from(mesh.positions[i as usize]);
            b.positions.push(to_bevy_point(m.transform_point3(p)).to_array());
            let n = mesh.normals.get(i as usize).map(|n| nm.transform_vector3(Vec3::from(*n)).normalize_or_zero()).unwrap_or(Vec3::Y);
            b.normals.push(to_bevy_point(n).to_array());
            b.uvs.push(mesh.uv0.get(i as usize).copied().unwrap_or([0.0, 0.0]));
            b.colors.push(mesh.colors.get(i as usize).copied().unwrap_or([1.0; 4]));
            b.src.push(i);
            next
        });
        local.push(idx);
    }
    for t in local.chunks_exact(3) {
        // Mirroring z (Unity -> Bevy) is a reflection: it reverses the geometric face normal
        // (v1-v0)x(v2-v0) but not the stored normals, so the winding must be reversed for faces to
        // stay front-facing under Bevy's CCW culling. An object whose own transform is mirrored
        // (det < 0) is reflected once more, which cancels it. Checked by `normals_check` and the
        // harness (`render.*.tris_against_normals_pct`).
        if flip {
            b.indices.extend_from_slice(&[t[0], t[1], t[2]]);
        } else {
            b.indices.extend_from_slice(&[t[0], t[2], t[1]]);
        }
    }
}
