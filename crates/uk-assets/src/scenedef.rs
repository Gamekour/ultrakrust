//! Full scene graph for the runtime: every GameObject (hierarchy, transform,
//! active flag, layer, tag), every renderer baked per object, every collider,
//! and every MonoBehaviour's serialized fields. Unlike `scene::load_level`, nothing
//! is filtered by activity here — the game decides what is on at runtime.
//!
//! Coordinates are converted to Bevy's right-handed space (Unity z -> -z).

use crate::db::AssetDb;
use crate::mesh::{self, MeshData};
use crate::scene::{bake, to_bevy_point, to_bevy_quat, Batch, MaterialKey};
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
const CLASS_MONOBEHAVIOUR: i32 = 114;
const CLASS_RIGIDBODY: i32 = 54;
const CLASS_SKINNED_MESH_RENDERER: i32 = 137;
const CLASS_SPHERE_COLLIDER: i32 = 135;
const CLASS_CAPSULE_COLLIDER: i32 = 136;
const CLASS_RECT_TRANSFORM: i32 = 224;

/// Tag ids from TagManager (globalgamemanagers): custom tags start at 20000.
pub mod tags {
    pub const UNTAGGED: u32 = 0;
    pub const PLAYER: u32 = 6;
    pub const LIMB: u32 = 20009;
    pub const HEAD: u32 = 20010;
    pub const END_LIMB: u32 = 20011;
    pub const ENEMY: u32 = 20016;
    pub const BREAKABLE: u32 = 20022;
    pub const MOVING: u32 = 20024;
    pub const SLIPPERY: u32 = 20027;
    pub const ARMOR: u32 = 20028;
}

/// Layers that never render in the world (UI, HUD, the original player rig).
const HIDDEN_LAYERS: &[u8] = &[2, 5, 13, 19, 28, 30];

#[derive(Clone, Debug)]
pub struct NodeDef {
    pub name: String,
    pub parent: Option<u32>,
    pub children: Vec<u32>,
    pub active_self: bool,
    pub layer: u8,
    pub tag: u32,
    pub local_pos: Vec3,
    pub local_rot: Quat,
    pub local_scale: Vec3,
    /// World matrix at load time (Bevy space).
    pub world0: Mat4,
}

#[derive(Debug)]
pub struct RenderDef {
    pub node: u32,
    pub material: Option<MaterialKey>,
    /// Geometry baked in world space at load time (Bevy space).
    pub batch: Batch,
    pub enabled: bool,
}

#[derive(Clone, Debug)]
pub enum ShapeDef {
    /// World-space oriented box at load time.
    Box { center: Vec3, half: Vec3, rot: Quat },
    Sphere { center: Vec3, radius: f32 },
    Capsule { a: Vec3, b: Vec3, radius: f32 },
    Mesh(Vec<[Vec3; 3]>),
}

#[derive(Clone, Debug)]
pub struct ColliderDef {
    pub node: u32,
    pub shape: ShapeDef,
    pub trigger: bool,
    pub enabled: bool,
    pub layer: u8,
    pub path_id: i64,
}

#[derive(Debug)]
pub struct ScriptDef {
    pub node: u32,
    pub class: String,
    pub enabled: bool,
    pub path_id: i64,
    pub data: Value,
}

#[derive(Default)]
pub struct SceneDef {
    pub nodes: Vec<NodeDef>,
    pub renderers: Vec<RenderDef>,
    pub colliders: Vec<ColliderDef>,
    pub scripts: Vec<ScriptDef>,
    /// GameObject or component path id -> node.
    pub obj_to_node: HashMap<i64, u32>,
    /// MonoBehaviour path id -> script index.
    pub comp_to_script: HashMap<i64, u32>,
    /// Collider component path id -> collider index.
    pub comp_to_collider: HashMap<i64, u32>,
    /// Nodes that carry a Rigidbody.
    pub rigidbodies: std::collections::HashSet<u32>,
    pub warnings: Vec<String>,
}

impl SceneDef {
    /// Node referenced by a PPtr to a GameObject or any component in the scene file.
    pub fn node_ref(&self, v: &Value) -> Option<u32> {
        let (f, id) = v.pptr();
        if f != 0 || id == 0 {
            return None;
        }
        self.obj_to_node.get(&id).copied()
    }

    pub fn script_ref(&self, v: &Value) -> Option<u32> {
        let (f, id) = v.pptr();
        if f != 0 || id == 0 {
            return None;
        }
        self.comp_to_script.get(&id).copied()
    }

    pub fn find(&self, name: &str) -> Option<u32> {
        self.nodes.iter().position(|n| n.name == name).map(|i| i as u32)
    }

    /// "Root/Child/Grandchild".
    pub fn path(&self, mut n: u32) -> String {
        let mut parts = vec![self.nodes[n as usize].name.clone()];
        while let Some(p) = self.nodes[n as usize].parent {
            parts.push(self.nodes[p as usize].name.clone());
            n = p;
        }
        parts.reverse();
        parts.join("/")
    }

    pub fn scripts_on(&self, node: u32) -> impl Iterator<Item = (u32, &ScriptDef)> {
        self.scripts.iter().enumerate().filter(move |(_, s)| s.node == node).map(|(i, s)| (i as u32, s))
    }

    pub fn is_descendant(&self, mut n: u32, ancestor: u32) -> bool {
        loop {
            if n == ancestor {
                return true;
            }
            match self.nodes[n as usize].parent {
                Some(p) => n = p,
                None => return false,
            }
        }
    }
}

fn unity_trs(v: &Value) -> (Vec3, Quat, Vec3) {
    let p = Vec3::from(v.get("m_LocalPosition").vec3());
    let q = v.get("m_LocalRotation").quat();
    let s = Vec3::from(v.get("m_LocalScale").vec3());
    (p, Quat::from_xyzw(q[0], q[1], q[2], q[3]).normalize(), s)
}

/// Unity-space world matrix -> Bevy-space world matrix (conjugate by the z mirror).
fn to_bevy_mat(m: Mat4) -> Mat4 {
    let f = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    f * m * f
}

struct Loader<'a> {
    db: &'a mut AssetDb,
    scene: Arc<SerializedFile>,
    meshes: HashMap<(String, i64), Option<Arc<MeshData>>>,
    class_names: HashMap<(i32, i64), String>,
}

impl Loader<'_> {
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

    fn class_name(&mut self, pptr: (i32, i64)) -> String {
        if let Some(n) = self.class_names.get(&pptr) {
            return n.clone();
        }
        let scene = self.scene.clone();
        let name = self
            .db
            .read_pptr(&scene, pptr)
            .ok()
            .flatten()
            .map(|(_, _, v)| v.get("m_ClassName").str().to_string())
            .unwrap_or_default();
        self.class_names.insert(pptr, name.clone());
        name
    }

    fn material_key(&mut self, p: (i32, i64)) -> Option<MaterialKey> {
        let scene = self.scene.clone();
        self.db.resolve(&scene, p).ok().flatten().map(|(f, id)| MaterialKey { file: f.name.clone(), path_id: id })
    }
}

/// Loads the complete scene graph of a level bundle.
pub fn load_scene(db: &mut AssetDb, bundle: &std::path::Path) -> Result<SceneDef> {
    let files = db.load_bundle_files(bundle)?;
    let scene = files
        .iter()
        .find(|f| !f.name.contains('.'))
        .cloned()
        .ok_or_else(|| crate::Error("no scene file in bundle".into()))?;
    let mut ld = Loader { db, scene: scene.clone(), meshes: HashMap::new(), class_names: HashMap::new() };
    let mut def = SceneDef::default();

    // --- GameObjects + Transforms -> nodes ---
    struct RawTr {
        go: i64,
        father: i64,
        trs: (Vec3, Quat, Vec3),
    }
    let mut raw_go: HashMap<i64, Value> = HashMap::new();
    let mut raw_tr: HashMap<i64, RawTr> = HashMap::new();
    for o in &scene.objects {
        match o.class_id {
            CLASS_GAMEOBJECT => {
                raw_go.insert(o.path_id, scene.read(o)?);
            }
            CLASS_TRANSFORM | CLASS_RECT_TRANSFORM => {
                let v = scene.read(o)?;
                raw_tr.insert(
                    o.path_id,
                    RawTr { go: v.get("m_GameObject").pptr().1, father: v.get("m_Father").pptr().1, trs: unity_trs(&v) },
                );
            }
            _ => {}
        }
    }
    // stable node order: by transform path id
    let mut tr_ids: Vec<i64> = raw_tr.keys().copied().collect();
    tr_ids.sort();
    let mut tr_to_node: HashMap<i64, u32> = HashMap::new();
    let mut node_tr: Vec<i64> = Vec::new();
    for &t in &tr_ids {
        let rt = &raw_tr[&t];
        let Some(g) = raw_go.get(&rt.go) else { continue };
        let idx = def.nodes.len() as u32;
        tr_to_node.insert(t, idx);
        node_tr.push(t);
        def.obj_to_node.insert(rt.go, idx);
        def.obj_to_node.insert(t, idx);
        let (p, q, s) = rt.trs;
        def.nodes.push(NodeDef {
            name: g.get("m_Name").str().to_string(),
            parent: None,
            children: Vec::new(),
            active_self: g.get("m_IsActive").bool(),
            layer: g.get("m_Layer").i64() as u8,
            tag: g.get("m_Tag").i64() as u32,
            local_pos: to_bevy_point(p),
            local_rot: to_bevy_quat(q),
            local_scale: s,
            world0: Mat4::IDENTITY,
        });
    }
    for &t in &tr_ids {
        let (Some(&n), Some(&pf)) = (tr_to_node.get(&t), tr_to_node.get(&raw_tr[&t].father)) else { continue };
        def.nodes[n as usize].parent = Some(pf);
        def.nodes[pf as usize].children.push(n);
    }
    // world matrices in Unity space, computed top-down
    let mut unity_world = vec![Mat4::IDENTITY; def.nodes.len()];
    let mut order: Vec<u32> = (0..def.nodes.len() as u32).filter(|&n| def.nodes[n as usize].parent.is_none()).collect();
    let mut i = 0;
    while i < order.len() {
        let n = order[i];
        let rt = &raw_tr[&node_tr[n as usize]];
        let local = Mat4::from_scale_rotation_translation(rt.trs.2, rt.trs.1, rt.trs.0);
        unity_world[n as usize] = match def.nodes[n as usize].parent {
            Some(p) => unity_world[p as usize] * local,
            None => local,
        };
        order.extend(def.nodes[n as usize].children.iter().copied());
        i += 1;
    }
    for (n, m) in unity_world.iter().enumerate() {
        def.nodes[n].world0 = to_bevy_mat(*m);
    }

    // --- components ---
    let go_ids: Vec<i64> = raw_go.keys().copied().collect();
    for gid in go_ids {
        let Some(&node) = def.obj_to_node.get(&gid) else { continue };
        let comps: Vec<(i32, i64)> = raw_go[&gid].get("m_Component").array().iter().map(|c| c.get("component").pptr()).collect();
        let layer = def.nodes[node as usize].layer;
        let world = unity_world[node as usize];
        let mut filter_mesh = None;
        let mut renderer: Option<Value> = None;
        for &(f, id) in &comps {
            if f != 0 {
                continue;
            }
            let Some(o) = scene.object(id) else { continue };
            match o.class_id {
                CLASS_MESH_FILTER => filter_mesh = scene.read(o).ok().map(|v| v.get("m_Mesh").pptr()),
                CLASS_MESH_RENDERER => renderer = scene.read(o).ok(),
                CLASS_SKINNED_MESH_RENDERER => {
                    let Ok(v) = scene.read(o) else { continue };
                    if HIDDEN_LAYERS.contains(&layer) {
                        continue;
                    }
                    def.obj_to_node.insert(id, node);
                    let Some(mesh) = ld.mesh(v.get("m_Mesh").pptr()) else { continue };
                    let bones: Vec<Mat4> = v
                        .get("m_Bones")
                        .array()
                        .iter()
                        .map(|b| tr_to_node.get(&b.pptr().1).map(|&bn| unity_world[bn as usize]).unwrap_or(world))
                        .collect();
                    let posed = skin(&mesh, &bones);
                    let mats: Vec<(i32, i64)> = v.get("m_Materials").array().iter().map(|m| m.pptr()).collect();
                    for (k, sm) in posed.submeshes.iter().enumerate() {
                        let material = mats.get(k).or(mats.last()).copied().and_then(|p| ld.material_key(p));
                        let mut batch = Batch::default();
                        bake(&mut batch, &posed, &sm.indices, Mat4::IDENTITY);
                        def.renderers.push(RenderDef { node, material, batch, enabled: v.get("m_Enabled").bool() });
                    }
                }
                CLASS_BOX_COLLIDER | CLASS_SPHERE_COLLIDER | CLASS_CAPSULE_COLLIDER | CLASS_MESH_COLLIDER => {
                    let Ok(v) = scene.read(o) else { continue };
                    let shape = match o.class_id {
                        CLASS_BOX_COLLIDER => {
                            let (scale, rot, _) = world.to_scale_rotation_translation();
                            let c = world.transform_point3(Vec3::from(v.get("m_Center").vec3()));
                            ShapeDef::Box {
                                center: to_bevy_point(c),
                                half: (Vec3::from(v.get("m_Size").vec3()) * scale).abs() * 0.5,
                                rot: to_bevy_quat(rot),
                            }
                        }
                        CLASS_SPHERE_COLLIDER => {
                            let (scale, _, _) = world.to_scale_rotation_translation();
                            let c = world.transform_point3(Vec3::from(v.get("m_Center").vec3()));
                            ShapeDef::Sphere { center: to_bevy_point(c), radius: v.get("m_Radius").f32() * scale.abs().max_element() }
                        }
                        CLASS_CAPSULE_COLLIDER => {
                            let (scale, _, _) = world.to_scale_rotation_translation();
                            let dir = v.get("m_Direction").i64() as usize;
                            let sc = scale.abs();
                            let radius = v.get("m_Radius").f32() * match dir {
                                0 => sc.y.max(sc.z),
                                1 => sc.x.max(sc.z),
                                _ => sc.x.max(sc.y),
                            };
                            let half = (v.get("m_Height").f32() * sc[dir.min(2)] * 0.5 - radius).max(0.0);
                            let mut axis = Vec3::ZERO;
                            axis[dir.min(2)] = 1.0;
                            let c = world.transform_point3(Vec3::from(v.get("m_Center").vec3()));
                            let ax = world.transform_vector3(axis).normalize_or_zero() * half;
                            ShapeDef::Capsule { a: to_bevy_point(c - ax), b: to_bevy_point(c + ax), radius }
                        }
                        _ => {
                            let Some(mesh) = ld.mesh(v.get("m_Mesh").pptr()) else { continue };
                            let mut tris = Vec::new();
                            for sm in &mesh.submeshes {
                                for t in sm.indices.chunks_exact(3) {
                                    let p = |i: u32| to_bevy_point(world.transform_point3(Vec3::from(mesh.positions[i as usize])));
                                    tris.push([p(t[0]), p(t[1]), p(t[2])]);
                                }
                            }
                            ShapeDef::Mesh(tris)
                        }
                    };
                    def.obj_to_node.insert(id, node);
                    def.comp_to_collider.insert(id, def.colliders.len() as u32);
                    def.colliders.push(ColliderDef {
                        node,
                        shape,
                        trigger: v.get("m_IsTrigger").bool(),
                        enabled: v.get("m_Enabled").bool(),
                        layer,
                        path_id: id,
                    });
                }
                CLASS_MONOBEHAVIOUR => {
                    let Ok(v) = scene.read(o) else { continue };
                    let class = ld.class_name(v.get("m_Script").pptr());
                    def.obj_to_node.insert(id, node);
                    def.comp_to_script.insert(id, def.scripts.len() as u32);
                    def.scripts.push(ScriptDef { node, class, enabled: v.get("m_Enabled").bool(), path_id: id, data: v });
                }
                CLASS_RIGIDBODY => {
                    def.obj_to_node.insert(id, node);
                    def.rigidbodies.insert(node);
                }
                _ => {
                    def.obj_to_node.insert(id, node);
                }
            }
        }
        if let (Some(r), Some(mp)) = (renderer, filter_mesh) {
            if HIDDEN_LAYERS.contains(&layer) {
                continue;
            }
            let Some(mesh) = ld.mesh(mp) else {
                def.warnings.push(format!("no mesh for {}", def.path(node)));
                continue;
            };
            let sb = r.get("m_StaticBatchInfo");
            let (first, count) = (sb.get("firstSubMesh").i64() as usize, sb.get("subMeshCount").i64() as usize);
            let (m, range) = if count > 0 {
                let root = r.get("m_StaticBatchRoot").pptr().1;
                let m = def.obj_to_node.get(&root).map(|&rn| unity_world[rn as usize]).unwrap_or(Mat4::IDENTITY);
                (m, first..first + count)
            } else {
                (world, 0..mesh.submeshes.len())
            };
            let mats: Vec<(i32, i64)> = r.get("m_Materials").array().iter().map(|m| m.pptr()).collect();
            for (k, si) in range.enumerate() {
                let Some(sm) = mesh.submeshes.get(si) else { continue };
                let material = mats.get(k).or(mats.last()).copied().and_then(|p| ld.material_key(p));
                let mut batch = Batch::default();
                bake(&mut batch, &mesh, &sm.indices, m);
                def.renderers.push(RenderDef { node, material, batch, enabled: r.get("m_Enabled").bool() });
            }
        }
    }
    Ok(def)
}

/// CPU skinning with the bone poses saved in the scene (Unity space in, Unity space out).
fn skin(mesh: &MeshData, bones: &[Mat4]) -> MeshData {
    let mut out = mesh.clone();
    if mesh.skin.is_empty() || bones.is_empty() {
        return out;
    }
    let mats: Vec<Mat4> = bones
        .iter()
        .enumerate()
        .map(|(i, b)| *b * mesh.bind_poses.get(i).copied().unwrap_or(Mat4::IDENTITY))
        .collect();
    for (vi, (idx, w)) in mesh.skin.iter().enumerate() {
        let p = Vec3::from(mesh.positions[vi]);
        let n = mesh.normals.get(vi).map(|n| Vec3::from(*n)).unwrap_or(Vec3::Y);
        let (mut pp, mut nn, mut tw) = (Vec3::ZERO, Vec3::ZERO, 0.0);
        for k in 0..4 {
            if w[k] <= 0.0 {
                continue;
            }
            let m = mats.get(idx[k] as usize).copied().unwrap_or(Mat4::IDENTITY);
            pp += m.transform_point3(p) * w[k];
            nn += m.transform_vector3(n) * w[k];
            tw += w[k];
        }
        if tw > 0.0 {
            out.positions[vi] = (pp / tw).to_array();
            if vi < out.normals.len() {
                out.normals[vi] = nn.normalize_or_zero().to_array();
            }
        }
    }
    out
}
