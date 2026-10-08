//! Full scene graph for the runtime: every GameObject (hierarchy, transform,
//! active flag, layer, tag), every renderer baked per object, every collider,
//! and every MonoBehaviour's serialized fields. Unlike `scene::load_level`, nothing
//! is filtered by activity here — the game decides what is on at runtime.
//!
//! Coordinates are converted to Bevy's right-handed space (Unity z -> -z).

use crate::anim::{Clip, Controller};
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
const CLASS_LIGHT: i32 = 108;
const CLASS_RENDER_SETTINGS: i32 = 104;
const CLASS_CAMERA: i32 = 20;
const CLASS_ANIMATOR: i32 = 95;
const CLASS_CANVAS: i32 = 223;
const CLASS_NAV_MESH_AGENT: i32 = 195;
const CLASS_CANVAS_GROUP: i32 = 225;
const CLASS_ANIMATOR_CONTROLLER: i32 = 91;
const CLASS_ANIMATOR_OVERRIDE_CONTROLLER: i32 = 221;
const CLASS_PARTICLE_SYSTEM: i32 = 198;
const CLASS_PARTICLE_RENDERER: i32 = 199;

/// Tag ids from TagManager (globalgamemanagers): custom tags start at 20000.
pub mod tags {
    pub const UNTAGGED: u32 = 0;
    pub const PLAYER: u32 = 6;
    pub const BODY: u32 = 20001;
    pub const FLOOR: u32 = 20007;
    pub const LIMB: u32 = 20009;
    pub const HEAD: u32 = 20010;
    pub const END_LIMB: u32 = 20011;
    pub const ENEMY: u32 = 20016;
    pub const BREAKABLE: u32 = 20022;
    pub const MOVING: u32 = 20024;
    pub const SLIPPERY: u32 = 20027;
    pub const ARMOR: u32 = 20028;
}

/// Layers neither the player's Main Camera (culling mask 0x8fd2dfd7) nor its HUD Camera (0x2000)
/// draws: trigger volumes (16 Invisible), UI, PlayerOnly/EnemyWall blockers, ...
pub const HIDDEN_LAYERS: &[u8] = &[3, 5, 16, 18, 19, 21, 28, 29, 30];

/// The viewmodel layer ("AlwaysOnTop"): drawn only by the HUD Camera (fov 90, depth cleared).
pub const VIEWMODEL_LAYER: u8 = 13;

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
    /// RectTransform layout fields (Unity space), when the Transform is a RectTransform.
    pub rect: Option<RectDef>,
}

/// RectTransform: the rect inside the parent rect is
/// `lerp(parent, anchor_min..anchor_max) + anchored_pos`, grown by `size_delta` around `pivot`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectDef {
    pub anchor_min: [f32; 2],
    pub anchor_max: [f32; 2],
    pub anchored_pos: [f32; 2],
    pub size_delta: [f32; 2],
    pub pivot: [f32; 2],
}

/// NavMeshAgent (class 195).
#[derive(Clone, Debug)]
pub struct NavAgentDef {
    pub node: u32,
    pub enabled: bool,
    pub base_offset: f32,
    pub speed: f32,
    pub acceleration: f32,
    pub angular_speed: f32,
    pub stopping_distance: f32,
    pub radius: f32,
    pub height: f32,
    pub auto_braking: bool,
    pub path_id: i64,
}

/// A native (non-MonoBehaviour) UI component: Canvas (223) or CanvasGroup (225), raw serialized data.
#[derive(Clone, Debug)]
pub struct UiNativeDef {
    pub node: u32,
    pub class_id: i32,
    pub path_id: i64,
    pub data: Value,
}

#[derive(Clone, Debug)]
pub struct RenderDef {
    pub node: u32,
    pub material: Option<MaterialKey>,
    /// Geometry baked in world space at load time (Bevy space).
    pub batch: Batch,
    pub enabled: bool,
    /// SkinnedMeshRenderer: the bind-pose mesh and its bone nodes, for re-skinning when animated.
    pub skin: Option<Arc<SkinDef>>,
}

#[derive(Debug)]
pub struct SkinDef {
    pub mesh: Arc<MeshData>,
    /// bone index -> node (None: bone outside the scene, falls back to the renderer's node)
    pub bones: Vec<Option<u32>>,
}

#[derive(Clone, Debug)]
pub struct AnimatorDef {
    pub node: u32,
    /// index into `SceneDef::controllers`
    pub controller: Option<u32>,
    pub enabled: bool,
    /// 0 AlwaysAnimate, 1 CullUpdateTransforms, 2 CullCompletely
    pub culling: u8,
    /// 0 Normal, 1 AnimatePhysics, 2 UnscaledTime
    pub update_mode: u8,
    pub root_motion: bool,
    pub keep_state_on_disable: bool,
    pub path_id: i64,
}

#[derive(Debug)]
pub struct ControllerDef {
    pub ctrl: Arc<Controller>,
    /// `ctrl.clips[i]` -> index into `SceneDef::clips` (override clips already applied)
    pub clips: Vec<Option<u32>>,
}

#[derive(Clone, Debug)]
pub enum ShapeDef {
    /// World-space oriented box at load time.
    Box { center: Vec3, half: Vec3, rot: Quat },
    Sphere { center: Vec3, radius: f32 },
    Capsule { a: Vec3, b: Vec3, radius: f32 },
    Mesh(Vec<[Vec3; 3]>),
}

/// A Unity Light. kind: 0 spot, 1 directional, 2 point, 3 area. render_mode: 0 auto, 1 important, 2 not important.
#[derive(Clone, Debug)]
pub struct LightDef {
    pub node: u32,
    pub kind: u8,
    /// Linear-space floats as serialized (the project renders in gamma, so used as-is).
    pub color: [f32; 4],
    pub intensity: f32,
    pub range: f32,
    pub spot_angle: f32,
    pub enabled: bool,
    pub render_mode: u8,
    pub culling_mask: u32,
}

/// A ParticleSystem placed in the scene (or in a prefab instantiated into it) with the
/// ParticleSystemRenderer on the same GameObject.
#[derive(Clone, Debug)]
pub struct SceneParticleDef {
    pub node: u32,
    pub path_id: i64,
    pub system: Arc<crate::particles::ParticleSystemDef>,
    pub renderer: Option<Arc<crate::particles::ParticleRendererDef>>,
    /// mesh renderer shape: the node whose MeshRenderer the system emits from
    pub shape_node: Option<u32>,
    /// collision planes (CollisionModule.m_Planes) as nodes
    pub planes: Vec<u32>,
}

/// RenderSettings (class 104). fog_mode: 1 linear, 2 exponential, 3 exp2. ambient_mode: 0 skybox,
/// 1 trilight, 3 flat, 4 custom.
#[derive(Clone, Debug, Default)]
pub struct RenderSettingsDef {
    pub fog: bool,
    pub fog_color: [f32; 4],
    pub fog_mode: i64,
    pub fog_density: f32,
    pub fog_start: f32,
    pub fog_end: f32,
    pub ambient_mode: i64,
    pub ambient_sky: [f32; 4],
    pub ambient_equator: [f32; 4],
    pub ambient_ground: [f32; 4],
    pub ambient_intensity: f32,
    /// `m_SkyboxMaterial` (None: the camera clears to its background color).
    pub skybox: Option<MaterialKey>,
    /// The player's Main Camera (culling mask 0x8fd2dfd7): clear flags (1 skybox, 2 solid color)
    /// and background color.
    pub camera_clear: Option<(i64, [f32; 4])>,
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

/// A material's SceneHelper surface data: `_SurfaceType` / `_EnviroParticleColor` and the
/// secondary pair VERTEX_BLENDING switches to.
#[derive(Debug, Clone, Copy)]
pub struct SurfaceMat {
    /// HasProperty(_SurfaceType) (approximated by the saved properties holding it)
    pub has_surface: bool,
    pub surface: i32,
    pub color: [f32; 4],
    pub secondary: i32,
    pub secondary_color: [f32; 4],
    pub vertex_blending: bool,
    /// STATIC_LIGHTING or STATIONARY_LIGHTING: the first material stands for every submesh
    pub static_lighting: bool,
}

/// An object SceneHelper duplicates into its footstep physics scene (IsValidForPhysicsScene: a
/// footstep layer, a non-trigger first Collider, a MeshRenderer, no non-kinematic Rigidbody),
/// with the mesh it gets (PreservedOriginalMesh, else the MeshCollider's, else the MeshFilter's)
/// at the object's load-time transform; the copy only follows the source's active state.
#[derive(Clone, Debug)]
pub struct SurfaceMeshDef {
    pub node: u32,
    pub layer: u8,
    /// world triangles (Bevy space), in the mesh's triangle order (submeshes concatenated)
    pub tris: Vec<[Vec3; 3]>,
    /// per triangle: index into `mats` ResolveHitSurfaceData picks (None: no material)
    pub tri_mat: Vec<Option<u16>>,
    /// per triangle: the vertex colors' red channel (empty when the mesh has no colors)
    pub tri_r: Vec<[f32; 3]>,
    pub mats: Vec<SurfaceMat>,
}

/// FootstepSet particle prefabs by SurfaceType (indices into `particle_prefabs`; None for a null
/// entry). Lookups fall back to Generic (0).
#[derive(Debug, Default, Clone)]
pub struct FootstepFx {
    pub enviro_gib_particles: HashMap<i32, Option<u32>>,
    pub slide_particles: HashMap<i32, Option<u32>>,
    pub wall_scrape_particles: HashMap<i32, Option<u32>>,
}

impl FootstepFx {
    fn get(m: &HashMap<i32, Option<u32>>, surface: i32) -> Option<u32> {
        m.get(&surface).or_else(|| m.get(&0)).copied().flatten()
    }
    pub fn enviro_gib_particle(&self, surface: i32) -> Option<u32> {
        Self::get(&self.enviro_gib_particles, surface)
    }
    pub fn slide_particle(&self, surface: i32) -> Option<u32> {
        Self::get(&self.slide_particles, surface)
    }
    pub fn wall_scrape_particle(&self, surface: i32) -> Option<u32> {
        Self::get(&self.wall_scrape_particles, surface)
    }
}

/// A material from outside the game (a custom map's glTF material): `RenderDef::material` (a
/// Unity material) supplies the shader and every property not overridden here.
#[derive(Debug, Clone)]
pub struct MaterialOverride {
    pub name: String,
    pub keywords: Vec<String>,
    pub floats: Vec<(String, f32)>,
    pub colors: Vec<(String, [f32; 4])>,
    /// texture property -> image (rows bottom first, as Unity's)
    pub textures: Vec<(String, Arc<crate::texture::TextureData>)>,
}

#[derive(Clone, Debug)]
pub struct ScriptDef {
    pub node: u32,
    pub class: String,
    pub enabled: bool,
    pub path_id: i64,
    pub data: Value,
    /// Serialized file the script's PPtrs resolve against when it came from a prefab (None: the scene file).
    pub file: Option<String>,
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
    /// Nodes whose Rigidbody is not kinematic.
    pub dynamic_rigidbodies: std::collections::HashSet<u32>,
    pub nav_agents: Vec<NavAgentDef>,
    pub warnings: Vec<String>,
    /// Baked navmeshes (one per agent type / surface).
    pub navmeshes: Vec<crate::navmesh::NavMeshData>,
    /// Light components (Unity space values; position/direction come from the node).
    pub lights: Vec<LightDef>,
    /// The scene's RenderSettings (fog, ambient).
    pub render_settings: RenderSettingsDef,
    /// Name of the scene's serialized file (PPtrs in `ScriptDef::data` resolve against it).
    pub scene_file: String,
    pub animators: Vec<AnimatorDef>,
    pub controllers: Vec<ControllerDef>,
    pub clips: Vec<Arc<Clip>>,
    /// Canvas / CanvasGroup components, in node order (kept apart from `scripts` so script order is unchanged).
    pub ui_natives: Vec<UiNativeDef>,
    /// The InputActionAsset the scene's InputActionReferences point at (ULTRAKILL's InputActions).
    pub input_actions: Option<std::sync::Arc<crate::input::InputActions>>,
    /// InputActionReference PPtrs (script file, file id, path id) -> m_ActionId.
    pub action_refs: HashMap<(Option<String>, i32, i64), String>,
    /// Effect prefabs scripts spawn at runtime (BloodsplatterManager's pools, MaliciousFace's
    /// dripBlood, ...), loaded outside the node graph.
    pub particle_prefabs: Vec<Arc<crate::particles::ParticlePrefab>>,
    /// (script, field) -> index into `particle_prefabs`
    pub script_prefabs: HashMap<(u32, String), u32>,
    /// (script, "field>Class") -> the data of the root `Class` MonoBehaviour of the prefab that
    /// field references (e.g. a ZombieProjectiles projectile's Projectile settings)
    pub script_nested: HashMap<(u32, String), Value>,
    /// SceneHelper's footstep physics scene
    pub surface_meshes: Vec<SurfaceMeshDef>,
    /// DefaultReferenceManager.footstepSet's particles
    pub footstep_fx: FootstepFx,
    /// ParticleSystems in the node graph, in node order.
    pub particle_systems: Vec<SceneParticleDef>,
    /// ParticleSystem component path id -> index into `particle_systems`.
    pub comp_to_particle: HashMap<i64, u32>,
    /// Materials made outside Unity (custom maps), drawn as their base material with overrides.
    pub material_overrides: Vec<MaterialOverride>,
    /// renderer index -> index into `material_overrides`
    pub renderer_override: HashMap<u32, u32>,
}

impl SceneDef {
    /// Drops every node `keep` rejects (and its subtree) with everything attached to it, remapping
    /// node / collider / script / particle-system indices. Baked navmeshes are cleared.
    pub fn retain_nodes(&mut self, keep: impl Fn(u32) -> bool) {
        let n = self.nodes.len();
        let mut kept = vec![false; n];
        // parents come before children in load order isn't guaranteed: walk ancestors
        for i in 0..n {
            let mut k = keep(i as u32);
            let mut p = self.nodes[i].parent;
            while k {
                let Some(q) = p else { break };
                k = keep(q);
                p = self.nodes[q as usize].parent;
            }
            kept[i] = k;
        }
        let mut map = vec![None; n];
        let mut c = 0u32;
        for i in 0..n {
            if kept[i] {
                map[i] = Some(c);
                c += 1;
            }
        }
        let m = |x: u32| map[x as usize];
        let nodes = std::mem::take(&mut self.nodes);
        self.nodes = nodes
            .into_iter()
            .enumerate()
            .filter(|(i, _)| kept[*i])
            .map(|(_, mut nd)| {
                nd.parent = nd.parent.and_then(m);
                nd.children = nd.children.iter().filter_map(|&c| m(c)).collect();
                nd
            })
            .collect();
        let mut rmap = vec![None; self.renderers.len()];
        let mut c = 0u32;
        for (i, r) in self.renderers.iter().enumerate() {
            if kept[r.node as usize] {
                rmap[i] = Some(c);
                c += 1;
            }
        }
        self.renderer_override = self.renderer_override.iter().filter_map(|(&k, &v)| Some((rmap[k as usize]?, v))).collect();
        self.renderers.retain(|r| kept[r.node as usize]);
        for r in &mut self.renderers {
            r.node = m(r.node).unwrap();
            if let Some(s) = &r.skin {
                r.skin = Some(Arc::new(SkinDef { mesh: s.mesh.clone(), bones: s.bones.iter().map(|b| b.and_then(m)).collect() }));
            }
        }
        let remap_list = |len: usize, keep: &dyn Fn(usize) -> bool| {
            let mut out = vec![None; len];
            let mut c = 0u32;
            for (i, o) in out.iter_mut().enumerate() {
                if keep(i) {
                    *o = Some(c);
                    c += 1;
                }
            }
            out
        };
        let cmap = remap_list(self.colliders.len(), &|i| kept[self.colliders[i].node as usize]);
        self.colliders.retain(|x| kept[x.node as usize]);
        for x in &mut self.colliders {
            x.node = m(x.node).unwrap();
        }
        self.comp_to_collider = self.comp_to_collider.iter().filter_map(|(&k, &v)| Some((k, cmap[v as usize]?))).collect();
        let smap = remap_list(self.scripts.len(), &|i| kept[self.scripts[i].node as usize]);
        self.scripts.retain(|x| kept[x.node as usize]);
        for x in &mut self.scripts {
            x.node = m(x.node).unwrap();
        }
        self.comp_to_script = self.comp_to_script.iter().filter_map(|(&k, &v)| Some((k, smap[v as usize]?))).collect();
        self.script_prefabs = std::mem::take(&mut self.script_prefabs).into_iter().filter_map(|((s, f), v)| Some(((smap[s as usize]?, f), v))).collect();
        self.script_nested = std::mem::take(&mut self.script_nested).into_iter().filter_map(|((s, f), v)| Some(((smap[s as usize]?, f), v))).collect();
        self.obj_to_node = self.obj_to_node.iter().filter_map(|(&k, &v)| Some((k, m(v)?))).collect();
        self.rigidbodies = self.rigidbodies.iter().filter_map(|&v| m(v)).collect();
        self.dynamic_rigidbodies = self.dynamic_rigidbodies.iter().filter_map(|&v| m(v)).collect();
        self.nav_agents.retain(|x| kept[x.node as usize]);
        for x in &mut self.nav_agents {
            x.node = m(x.node).unwrap();
        }
        self.navmeshes.clear();
        self.lights.retain(|x| kept[x.node as usize]);
        for x in &mut self.lights {
            x.node = m(x.node).unwrap();
        }
        self.animators.retain(|x| kept[x.node as usize]);
        for x in &mut self.animators {
            x.node = m(x.node).unwrap();
        }
        self.ui_natives.retain(|x| kept[x.node as usize]);
        for x in &mut self.ui_natives {
            x.node = m(x.node).unwrap();
        }
        self.surface_meshes.retain(|x| kept[x.node as usize]);
        for x in &mut self.surface_meshes {
            x.node = m(x.node).unwrap();
        }
        let pmap = remap_list(self.particle_systems.len(), &|i| kept[self.particle_systems[i].node as usize]);
        self.particle_systems.retain(|x| kept[x.node as usize]);
        for x in &mut self.particle_systems {
            x.node = m(x.node).unwrap();
            x.shape_node = x.shape_node.and_then(m);
            x.planes = x.planes.iter().filter_map(|&p| m(p)).collect();
        }
        self.comp_to_particle = self.comp_to_particle.iter().filter_map(|(&k, &v)| Some((k, pmap[v as usize]?))).collect();
    }

    /// Copies `src`'s subtree (components, baked geometry, particle systems, lights, animators)
    /// under `parent`, with the copy's root at world matrix `world` (Bevy space; it must differ
    /// from `src`'s by a rigid transform). The copy's objects get new path ids, and PPtrs inside
    /// the subtree's scene-file components point at the copies, as Object.Instantiate does.
    /// Returns the copy's root node.
    pub fn clone_subtree(&mut self, src: u32, parent: u32, world: Mat4) -> u32 {
        let mut sub = Vec::new();
        let mut stack = vec![src];
        while let Some(n) = stack.pop() {
            sub.push(n);
            stack.extend(self.nodes[n as usize].children.iter().rev().copied());
        }
        let base = self.nodes.len() as u32;
        let nmap: HashMap<u32, u32> = sub.iter().enumerate().map(|(i, &n)| (n, base + i as u32)).collect();
        let m = |n: u32| nmap.get(&n).copied();
        let mn = |n: u32| m(n).unwrap_or(n);
        let d = world * self.nodes[src as usize].world0.inverse();
        let (_, dr, _) = d.to_scale_rotation_translation();
        let pt = |p: Vec3| d.transform_point3(p);
        // fresh path ids below every id in use
        let mut next = [self.obj_to_node.keys(), self.comp_to_script.keys(), self.comp_to_collider.keys(), self.comp_to_particle.keys()]
            .into_iter()
            .flatten()
            .copied()
            .min()
            .unwrap_or(0)
            .min(0)
            - 1;
        let mut ids: Vec<(i64, u32)> = self.obj_to_node.iter().filter(|(_, n)| nmap.contains_key(n)).map(|(&k, &n)| (k, n)).collect();
        ids.sort_unstable();
        let mut idmap = HashMap::new();
        for &(k, n) in &ids {
            idmap.insert(k, next);
            self.obj_to_node.insert(next, mn(n));
            next -= 1;
        }
        let id = |k: i64| idmap.get(&k).copied().unwrap_or(k);
        // nodes
        for &n in &sub {
            let mut nd = self.nodes[n as usize].clone();
            nd.parent = if n == src { Some(parent) } else { nd.parent.map(mn) };
            nd.children = nd.children.iter().map(|&c| mn(c)).collect();
            nd.world0 = d * nd.world0;
            self.nodes.push(nd);
        }
        let root = base;
        let (s, r, t) = (self.nodes[parent as usize].world0.inverse() * world).to_scale_rotation_translation();
        let rn = &mut self.nodes[root as usize];
        (rn.local_pos, rn.local_rot, rn.local_scale) = (t, r, s);
        self.nodes[parent as usize].children.push(root);
        // renderers
        for ri in 0..self.renderers.len() {
            let Some(node) = m(self.renderers[ri].node) else { continue };
            let mut r = self.renderers[ri].clone();
            r.node = node;
            r.batch.positions.iter_mut().for_each(|p| *p = pt(Vec3::from(*p)).into());
            r.batch.normals.iter_mut().for_each(|p| *p = (dr * Vec3::from(*p)).into());
            if let Some(sk) = &r.skin {
                r.skin = Some(Arc::new(SkinDef { mesh: sk.mesh.clone(), bones: sk.bones.iter().map(|b| b.map(mn)).collect() }));
            }
            if let Some(&o) = self.renderer_override.get(&(ri as u32)) {
                self.renderer_override.insert(self.renderers.len() as u32, o);
            }
            self.renderers.push(r);
        }
        // colliders
        let old: Vec<(i64, u32)> = self.comp_to_collider.iter().map(|(&k, &v)| (k, v)).collect();
        let mut cmap = HashMap::new();
        for ci in 0..self.colliders.len() {
            let Some(node) = m(self.colliders[ci].node) else { continue };
            let mut c = self.colliders[ci].clone();
            c.node = node;
            transform_shape(&mut c.shape, &pt, dr);
            cmap.insert(ci as u32, self.colliders.len() as u32);
            self.colliders.push(c);
        }
        for (k, v) in old {
            if let Some(&nv) = cmap.get(&v) {
                self.comp_to_collider.insert(id(k), nv);
            }
        }
        // scripts
        let old: Vec<(i64, u32)> = self.comp_to_script.iter().map(|(&k, &v)| (k, v)).collect();
        let mut smap = HashMap::new();
        for si in 0..self.scripts.len() {
            let Some(node) = m(self.scripts[si].node) else { continue };
            let mut s = self.scripts[si].clone();
            s.node = node;
            s.path_id = id(s.path_id);
            if s.file.is_none() {
                remap_pptrs(&mut s.data, &idmap);
            }
            smap.insert(si as u32, self.scripts.len() as u32);
            self.scripts.push(s);
        }
        for (k, v) in old {
            if let Some(&nv) = smap.get(&v) {
                self.comp_to_script.insert(id(k), nv);
            }
        }
        let sp: Vec<_> = self.script_prefabs.iter().filter_map(|((s, f), &v)| Some(((*smap.get(s)?, f.clone()), v))).collect();
        self.script_prefabs.extend(sp);
        let sn: Vec<_> = self.script_nested.iter().filter_map(|((s, f), v)| Some(((*smap.get(s)?, f.clone()), v.clone()))).collect();
        self.script_nested.extend(sn);
        // the rest of the per-node components
        let rbs: Vec<u32> = self.rigidbodies.iter().filter_map(|&n| m(n)).collect();
        self.rigidbodies.extend(rbs);
        let rbs: Vec<u32> = self.dynamic_rigidbodies.iter().filter_map(|&n| m(n)).collect();
        self.dynamic_rigidbodies.extend(rbs);
        let v: Vec<_> = self.nav_agents.iter().filter_map(|a| Some(NavAgentDef { node: m(a.node)?, path_id: id(a.path_id), ..a.clone() })).collect();
        self.nav_agents.extend(v);
        let v: Vec<_> = self.lights.iter().filter_map(|l| Some(LightDef { node: m(l.node)?, ..l.clone() })).collect();
        self.lights.extend(v);
        let v: Vec<_> = self.animators.iter().filter_map(|a| Some(AnimatorDef { node: m(a.node)?, path_id: id(a.path_id), ..a.clone() })).collect();
        self.animators.extend(v);
        let v: Vec<_> = self
            .ui_natives
            .iter()
            .filter_map(|u| {
                let mut u = UiNativeDef { node: m(u.node)?, path_id: id(u.path_id), ..u.clone() };
                remap_pptrs(&mut u.data, &idmap);
                Some(u)
            })
            .collect();
        self.ui_natives.extend(v);
        let v: Vec<_> = self
            .surface_meshes
            .iter()
            .filter_map(|s| {
                let mut s = SurfaceMeshDef { node: m(s.node)?, ..s.clone() };
                s.tris.iter_mut().flatten().for_each(|p| *p = pt(*p));
                Some(s)
            })
            .collect();
        self.surface_meshes.extend(v);
        let old: Vec<(i64, u32)> = self.comp_to_particle.iter().map(|(&k, &v)| (k, v)).collect();
        let mut pmap = HashMap::new();
        for pi in 0..self.particle_systems.len() {
            let p = &self.particle_systems[pi];
            let Some(node) = m(p.node) else { continue };
            let p = SceneParticleDef { node, path_id: id(p.path_id), shape_node: p.shape_node.map(mn), planes: p.planes.iter().map(|&q| mn(q)).collect(), ..p.clone() };
            pmap.insert(pi as u32, self.particle_systems.len() as u32);
            self.particle_systems.push(p);
        }
        for (k, v) in old {
            if let Some(&nv) = pmap.get(&v) {
                self.comp_to_particle.insert(id(k), nv);
            }
        }
        root
    }

    /// Applies a rigid world-space transform to `root`'s subtree and everything baked under it,
    /// moving the root's local transform (roots only: the parent's frame is identity). Everything
    /// here is Bevy space, local_* included.
    pub fn move_root(&mut self, root: u32, d: Mat4) {
        let under: Vec<bool> = (0..self.nodes.len() as u32).map(|n| self.is_descendant(n, root)).collect();
        let (_, dr, _) = d.to_scale_rotation_translation();
        for (i, nd) in self.nodes.iter_mut().enumerate() {
            if under[i] {
                nd.world0 = d * nd.world0;
            }
        }
        let w = self.nodes[root as usize].world0;
        let (_, r, t) = w.to_scale_rotation_translation();
        self.nodes[root as usize].local_pos = t;
        self.nodes[root as usize].local_rot = r;
        let pt = |p: Vec3| d.transform_point3(p);
        for r in self.renderers.iter_mut().filter(|r| under[r.node as usize]) {
            for p in &mut r.batch.positions {
                *p = pt(Vec3::from(*p)).into();
            }
            for nm in &mut r.batch.normals {
                *nm = (dr * Vec3::from(*nm)).into();
            }
        }
        for c in self.colliders.iter_mut().filter(|c| under[c.node as usize]) {
            match &mut c.shape {
                ShapeDef::Box { center, rot, .. } => {
                    *center = pt(*center);
                    *rot = dr * *rot;
                }
                ShapeDef::Sphere { center, .. } => *center = pt(*center),
                ShapeDef::Capsule { a, b, .. } => {
                    *a = pt(*a);
                    *b = pt(*b);
                }
                ShapeDef::Mesh(t) => t.iter_mut().flatten().for_each(|p| *p = pt(*p)),
            }
        }
        for s in self.surface_meshes.iter_mut().filter(|s| under[s.node as usize]) {
            s.tris.iter_mut().flatten().for_each(|p| *p = pt(*p));
        }
    }
    /// Node referenced by a PPtr to a GameObject or any component in the scene file.
    pub fn node_ref(&self, v: &Value) -> Option<u32> {
        let (f, id) = v.pptr();
        if f != 0 || id == 0 {
            return None;
        }
        self.obj_to_node.get(&id).copied()
    }

    /// The action id an InputActionReference field of script `sc` points at.
    pub fn action_ref(&self, sc: u32, v: &Value) -> Option<&str> {
        let (f, id) = v.pptr();
        self.action_refs.get(&(self.scripts[sc as usize].file.clone(), f, id)).map(String::as_str)
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
    controllers: HashMap<(String, i64), Option<u32>>,
    clips: HashMap<(String, i64), Option<u32>>,
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

    /// AnimationClip by PPtr from `from`, decoded once per scene.
    fn clip(&mut self, def: &mut SceneDef, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Option<u32> {
        let (file, id) = self.db.resolve(from, pptr).ok()??;
        let key = (file.name.clone(), id);
        if let Some(c) = self.clips.get(&key) {
            return *c;
        }
        let c = file.read_id(id).ok().filter(|v| !v.get("m_Legacy").bool()).map(|v| {
            def.clips.push(Arc::new(Clip::from_value(&v)));
            def.clips.len() as u32 - 1
        });
        self.clips.insert(key, c);
        c
    }

    /// AnimatorController (or override controller) by PPtr from the scene, decoded once.
    fn controller(&mut self, def: &mut SceneDef, pptr: (i32, i64)) -> Option<u32> {
        let scene = self.scene.clone();
        let (file, id) = self.db.resolve(&scene, pptr).ok()??;
        let key = (file.name.clone(), id);
        if let Some(c) = self.controllers.get(&key) {
            return *c;
        }
        let class = file.objects.iter().find(|o| o.path_id == id).map(|o| o.class_id);
        let v = file.read_id(id).ok()?;
        let built = match class {
            Some(CLASS_ANIMATOR_CONTROLLER) => {
                let ctrl = Controller::from_value(&v);
                let clips = ctrl.clips.iter().map(|&p| self.clip(def, &file, p)).collect();
                Some(ControllerDef { ctrl: Arc::new(ctrl), clips })
            }
            Some(CLASS_ANIMATOR_OVERRIDE_CONTROLLER) => self.override_controller(def, &file, &v),
            _ => None,
        };
        let idx = built.map(|c| {
            def.controllers.push(c);
            def.controllers.len() as u32 - 1
        });
        self.controllers.insert(key, idx);
        idx
    }

    /// AnimatorOverrideController (221): the base controller with its clips swapped. Originals
    /// resolve against the override's file, the base's own clip list against the base's file.
    fn override_controller(&mut self, def: &mut SceneDef, file: &Arc<SerializedFile>, v: &Value) -> Option<ControllerDef> {
        let (bf, bid) = self.db.resolve(file, v.get("m_Controller").pptr()).ok()??;
        let base = Controller::from_value(&bf.read_id(bid).ok()?);
        let mut over: HashMap<(String, i64), (i32, i64)> = HashMap::new();
        for c in v.get("m_Clips").array() {
            let o = c.get("m_OverrideClip").pptr();
            if o.1 == 0 {
                continue;
            }
            if let Ok(Some((f, i))) = self.db.resolve(file, c.get("m_OriginalClip").pptr()) {
                over.insert((f.name.clone(), i), o);
            }
        }
        let mut clips = Vec::new();
        for &p in &base.clips {
            let orig = self.db.resolve(&bf, p).ok().flatten().map(|(f, i)| (f.name.clone(), i));
            clips.push(match orig.and_then(|k| over.get(&k).copied()) {
                Some(o) => self.clip(def, file, o),
                None => self.clip(def, &bf, p),
            });
        }
        Some(ControllerDef { ctrl: Arc::new(base), clips })
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

/// The effect fields of effect-spawning scripts (GameObjects or AssetReferences they
/// Instantiate). `a>B.c` follows field `a` to a prefab and reads field `c` of the `B` script on
/// its root (RevolverBeam.hitParticle through Revolver.revolverBeam). BloodsplatterManager.InitPools
/// pools its fields (the bloodstain / gib fields aside).
const EFFECT_PREFABS: &[(&str, &[&str])] = &[
    ("BloodsplatterManager", &["head", "limb", "body", "small", "smallest", "splatter", "underwater", "sand"]),
    ("MaliciousFace", &["dripBlood", "woundedParticle", "beamExplosion", "breakParticle", "enrageEffect", "impactParticle", "proj>Projectile.explosionEffect"]),
    ("NewMovement", &["dodgeParticle", "slideParticle", "fallParticle", "impactDust"]),
    ("Breakable", &["breakParticle", "breakParticleFallback", "durabilityHurtParticle"]),
    ("Glass", &["shatterParticle"]),
    ("CheckPoint", &["activateEffect"]),
    ("TeleportPlayer", &["teleportEffect"]),
    ("DeathZone", &["sawSound"]),
    ("ZombieMelee", &["hitGroundParticle", "pullOutParticle"]),
    ("ZombieProjectiles", &["projectileBeam", "projectile>Projectile.explosionEffect"]),
    ("PowerUpMeter", &["endEffect"]),
    ("DualWieldPickup", &["pickUpEffect"]),
    ("Punch", &["dustParticle"]),
    ("Revolver", &["revolverBeam>RevolverBeam.hitParticle", "revolverBeamSuper>RevolverBeam.hitParticle"]),
];

/// The GameObject a PPtr (to a GameObject or any of its components) or an AssetReference points at.
fn effect_target(ld: &mut Loader, from: &Arc<SerializedFile>, v: &Value) -> Option<(Arc<SerializedFile>, i64)> {
    let guid = v.get("m_AssetGUID").str().to_string();
    let (f, id) = if !guid.is_empty() {
        if ld.db.catalog.is_none() {
            ld.db.catalog = crate::addressables::Catalog::load(&ld.db.install.clone()).ok().map(Arc::new);
        }
        let cat = ld.db.catalog.clone()?;
        cat.load_asset(ld.db, &guid).ok()?
    } else {
        ld.db.resolve(from, v.pptr()).ok().flatten()?
    };
    let class = f.object(id)?.class_id;
    let go = if class == 1 { id } else { f.read_id(id).ok()?.get("m_GameObject").pptr().1 };
    (go != 0).then_some((f, go))
}

fn load_effect_prefabs(ld: &mut Loader, def: &mut SceneDef) {
    let mut loaded: HashMap<(String, i64), u32> = HashMap::new();
    for si in 0..def.scripts.len() {
        let Some((_, fields)) = EFFECT_PREFABS.iter().find(|(c, _)| *c == def.scripts[si].class) else { continue };
        let from = match &def.scripts[si].file {
            Some(f) => ld.db.file(f).ok(),
            None => Some(ld.scene.clone()),
        };
        let Some(from) = from else { continue };
        for &field in *fields {
            let target = match field.split_once('>') {
                None => effect_target(ld, &from, def.scripts[si].data.get(field)),
                Some((outer, inner)) => {
                    let (class, inner) = inner.split_once('.').unwrap();
                    effect_target(ld, &from, def.scripts[si].data.get(outer)).and_then(|(f, go)| {
                        let g = f.read_id(go).ok()?;
                        let sv = g.get("m_Component").array().iter().find_map(|c| {
                            let cv = f.read_id(c.get("component").pptr().1).ok()?;
                            let name = ld.db.read_pptr(&f, cv.get("m_Script").pptr()).ok().flatten()?.2.get("m_ClassName").str().to_string();
                            (name == class).then_some(cv)
                        })?;
                        def.script_nested.insert((si as u32, format!("{outer}>{class}")), sv.clone());
                        effect_target(ld, &f, sv.get(inner))
                    })
                }
            };
            let Some((f, go)) = target else { continue };
            let key = (f.name.clone(), go);
            let idx = match loaded.get(&key) {
                Some(&i) => i,
                None => match crate::particles::load_prefab(ld.db, &f, go) {
                    Ok(pf) => {
                        def.particle_prefabs.push(Arc::new(pf));
                        let i = def.particle_prefabs.len() as u32 - 1;
                        loaded.insert(key, i);
                        i
                    }
                    Err(e) => {
                        def.warnings.push(format!("{}.{field}: {e}", def.scripts[si].class));
                        continue;
                    }
                },
            };
            def.script_prefabs.insert((si as u32, field.to_string()), idx);
        }
    }
}

/// TagManager layers in SceneHelper's footstepLayerMask: Environment, Outdoors, EnvironmentBaked,
/// OutdoorsBaked.
pub const FOOTSTEP_LAYERS: [u8; 4] = [8, 24, 7, 6];

fn surface_mat(ld: &mut Loader, from: &Arc<SerializedFile>, p: (i32, i64)) -> Option<SurfaceMat> {
    let (_, _, v) = ld.db.read_pptr(from, p).ok()??;
    let m = crate::shader::MaterialProps::from_value(&v);
    let kw = |k: &str| m.keywords.iter().any(|x| x == k);
    let col = |k: &str| m.colors.get(k).copied().unwrap_or([1.0; 4]);
    let flt = |k: &str| m.floats.get(k).copied().unwrap_or(0.0).round() as i32;
    Some(SurfaceMat {
        has_surface: m.floats.contains_key("_SurfaceType"),
        surface: flt("_SurfaceType"),
        color: col("_EnviroParticleColor"),
        secondary: flt("_SecondarySurfaceType"),
        secondary_color: col("_SecondaryEnviroParticleColor"),
        vertex_blending: kw("VERTEX_BLENDING"),
        static_lighting: kw("STATIC_LIGHTING") || kw("STATIONARY_LIGHTING"),
    })
}

/// A footstep physics scene object; per triangle the material SceneHelper.ResolveHitSurfaceData
/// resolves (`reusableMaterials[submesh - subMeshStartIndex]`, or the first material when the
/// mesh has one submesh or that material is statically lit).
#[allow(clippy::too_many_arguments)]
fn surface_mesh(ld: &mut Loader, file: &Arc<SerializedFile>, node: u32, layer: u8, world: Mat4, mesh: &MeshData, mats: &[(i32, i64)], start: usize) -> SurfaceMeshDef {
    let smats: Vec<Option<SurfaceMat>> = mats.iter().map(|&p| surface_mat(ld, file, p)).collect();
    let mut out = SurfaceMeshDef { node, layer, tris: Vec::new(), tri_mat: Vec::new(), tri_r: Vec::new(), mats: Vec::new() };
    let idx: Vec<Option<u16>> = smats
        .iter()
        .map(|m| {
            m.map(|m| {
                out.mats.push(m);
                out.mats.len() as u16 - 1
            })
        })
        .collect();
    let first = idx.first().copied().flatten();
    let whole = mesh.submeshes.len() <= 1 || smats.first().copied().flatten().is_some_and(|m| m.static_lighting);
    let r = |i: u32| mesh.colors.get(i as usize).map_or(1.0, |c| c[0]);
    let mut t = 0usize;
    for sm in &mesh.submeshes {
        for tri in sm.indices.chunks_exact(3) {
            let p = |i: u32| to_bevy_point(world.transform_point3(Vec3::from(mesh.positions[i as usize])));
            out.tris.push([p(tri[0]), p(tri[1]), p(tri[2])]);
            // the submesh walk starts at subMeshStartIndex with its index count from 0
            let num = t * 3;
            let mut found = None;
            if !whole {
                let mut cum = 0;
                for i in start..mesh.submeshes.len() {
                    let next = cum + mesh.submeshes[i].indices.len();
                    if num < next {
                        found = Some((i, cum));
                        break;
                    }
                    cum = next;
                }
            }
            out.tri_mat.push(if whole { first } else { found.and_then(|(i, _)| idx.get(i - start).copied().flatten()) });
            if !mesh.colors.is_empty() {
                // GetTriangles(submesh)[num - num2..]: the found submesh's own triangle list
                let (si, base) = found.unwrap_or((0, 0));
                let ix = &mesh.submeshes[si].indices;
                let k = num - base;
                out.tri_r.push(if k + 2 < ix.len() { [r(ix[k]), r(ix[k + 1]), r(ix[k + 2])] } else { [1.0; 3] });
            }
            t += 1;
        }
    }
    out
}

/// DefaultReferenceManager.footstepSet: the FootstepSet's particle tables, last entry per
/// SurfaceType winning as in its Initialize.
fn load_footstep_set(ld: &mut Loader, def: &mut SceneDef) {
    let Some(sc) = def.scripts.iter().find(|s| s.class == "DefaultReferenceManager") else { return };
    let from = match &sc.file {
        Some(f) => ld.db.file(f).ok(),
        None => Some(ld.scene.clone()),
    };
    let Some(from) = from else { return };
    let Ok(Some((sf, _, set))) = ld.db.read_pptr(&from, sc.data.get("footstepSet").pptr()) else {
        def.warnings.push("DefaultReferenceManager.footstepSet: missing".into());
        return;
    };
    let mut loaded: HashMap<(String, i64), u32> = HashMap::new();
    let mut fx = FootstepFx::default();
    for (field, map) in [("enviroGibParticles", &mut fx.enviro_gib_particles), ("slideParticles", &mut fx.slide_particles), ("wallScrapeParticles", &mut fx.wall_scrape_particles)] {
        for e in set.get(field).array() {
            let surface = e.get("<SurfaceType>k__BackingField").i64() as i32;
            let prefab = effect_target(ld, &sf, e.get("<particle>k__BackingField")).and_then(|(f, go)| {
                let key = (f.name.clone(), go);
                if let Some(&i) = loaded.get(&key) {
                    return Some(i);
                }
                match crate::particles::load_prefab(ld.db, &f, go) {
                    Ok(pf) => {
                        def.particle_prefabs.push(Arc::new(pf));
                        let i = def.particle_prefabs.len() as u32 - 1;
                        loaded.insert(key, i);
                        Some(i)
                    }
                    Err(e) => {
                        def.warnings.push(format!("FootstepSet.{field}: {e}"));
                        None
                    }
                }
            });
            map.insert(surface, prefab);
        }
    }
    def.footstep_fx = fx;
}

/// Resolves the scripts' InputActionReference fields (top-level fields named `action*`, single or
/// arrays) and reads the InputActionAsset they belong to.
fn load_action_refs(ld: &mut Loader, def: &mut SceneDef) {
    for s in &def.scripts {
        let Value::Struct(fields) = &s.data else { continue };
        let from = match &s.file {
            Some(f) => ld.db.file(f).ok(),
            None => Some(ld.scene.clone()),
        };
        let Some(from) = from else { continue };
        for (k, v) in fields {
            if !k.to_ascii_lowercase().starts_with("action") {
                continue;
            }
            let ptrs: Vec<&Value> = match v {
                Value::Array(a) => a.iter().collect(),
                v => vec![v],
            };
            for p in ptrs {
                let (f, id) = p.pptr();
                if id == 0 || def.action_refs.contains_key(&(s.file.clone(), f, id)) {
                    continue;
                }
                let Ok(Some((rf, _, rv))) = ld.db.read_pptr(&from, (f, id)) else { continue };
                if !rv.has("m_ActionId") {
                    continue;
                }
                def.action_refs.insert((s.file.clone(), f, id), rv.get("m_ActionId").str().to_string());
                if def.input_actions.is_none() {
                    if let Ok(Some((_, _, a))) = ld.db.read_pptr(&rf, rv.get("m_Asset").pptr()) {
                        def.input_actions = Some(std::sync::Arc::new(crate::input::InputActions::read(&a)));
                    }
                }
            }
        }
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
    let mut ld = Loader { db, scene: scene.clone(), meshes: HashMap::new(), class_names: HashMap::new(), controllers: HashMap::new(), clips: HashMap::new() };
    let mut def = SceneDef { scene_file: scene.name.clone(), ..SceneDef::default() };

    // --- GameObjects + Transforms -> nodes ---
    let mut raw_go: HashMap<i64, Value> = HashMap::new();
    let mut raw_tr: HashMap<i64, RawTr> = HashMap::new();
    for o in &scene.objects {
        match o.class_id {
            CLASS_GAMEOBJECT => {
                raw_go.insert(o.path_id, scene.read(o)?);
            }
            CLASS_TRANSFORM | CLASS_RECT_TRANSFORM => {
                let v = scene.read(o)?;
                raw_tr.insert(o.path_id, self::raw_tr(&v));
            }
            CLASS_RENDER_SETTINGS => {
                let v = scene.read(o)?;
                let col = |k: &str| {
                    let c = v.get(k);
                    [c.get("r").f32(), c.get("g").f32(), c.get("b").f32(), c.get("a").f32()]
                };
                def.render_settings = RenderSettingsDef {
                    fog: v.get("m_Fog").bool(),
                    fog_color: col("m_FogColor"),
                    fog_mode: v.get("m_FogMode").i64(),
                    fog_density: v.get("m_FogDensity").f32(),
                    fog_start: v.get("m_LinearFogStart").f32(),
                    fog_end: v.get("m_LinearFogEnd").f32(),
                    ambient_mode: v.get("m_AmbientMode").i64(),
                    ambient_sky: col("m_AmbientSkyColor"),
                    ambient_equator: col("m_AmbientEquatorColor"),
                    ambient_ground: col("m_AmbientGroundColor"),
                    ambient_intensity: v.get("m_AmbientIntensity").f32(),
                    skybox: ld.material_key(v.get("m_SkyboxMaterial").pptr()),
                    camera_clear: def.render_settings.camera_clear,
                };
            }
            CLASS_CAMERA => {
                let v = scene.read(o)?;
                if v.get("m_Enabled").bool() && v.get("m_CullingMask").get("m_Bits").i64() as u32 == 0x8fd2dfd7 {
                    let c = v.get("m_BackGroundColor");
                    def.render_settings.camera_clear = Some((v.get("m_ClearFlags").i64(), [c.get("r").f32(), c.get("g").f32(), c.get("b").f32(), c.get("a").f32()]));
                }
            }
            _ => {}
        }
    }
    add_objects(&mut ld, &mut def, &raw_go, &raw_tr, None)?;
    spawn_viewmodel(&mut ld, &mut def);
    load_action_refs(&mut ld, &mut def);
    load_effect_prefabs(&mut ld, &mut def);
    load_footstep_set(&mut ld, &mut def);
    def.navmeshes = crate::navmesh::load_scene_navmeshes(ld.db, bundle).unwrap_or_default();
    Ok(def)
}

struct RawTr {
    go: i64,
    father: i64,
    trs: (Vec3, Quat, Vec3),
    /// m_Children: the sibling order (GetChild / uGUI draw order).
    children: Vec<i64>,
    rect: Option<RectDef>,
}

fn raw_tr(v: &Value) -> RawTr {
    let v2 = |k: &str| {
        let x = v.get(k);
        [x.get("x").f32(), x.get("y").f32()]
    };
    let rect = v.has("m_AnchorMin").then(|| RectDef {
        anchor_min: v2("m_AnchorMin"),
        anchor_max: v2("m_AnchorMax"),
        anchored_pos: v2("m_AnchoredPosition"),
        size_delta: v2("m_SizeDelta"),
        pivot: v2("m_Pivot"),
    });
    RawTr {
        go: v.get("m_GameObject").pptr().1,
        father: v.get("m_Father").pptr().1,
        trs: unity_trs(v),
        children: v.get("m_Children").array().iter().map(|c| c.pptr().1).collect(),
        rect,
    }
}

/// Appends the GameObjects / Transforms of `ld.scene` (`raw_go`, `raw_tr`) as nodes with their
/// components. Roots attach under `parent` (local TRS kept, as `Instantiate(prefab, parent)`).
/// Returns the first new root node.
fn add_objects(ld: &mut Loader, def: &mut SceneDef, raw_go: &HashMap<i64, Value>, raw_tr: &HashMap<i64, RawTr>, parent: Option<u32>) -> Result<Option<u32>> {
    let file = ld.scene.clone();
    let prefab = parent.is_some().then(|| file.name.clone());
    let base = def.nodes.len();
    // stable node order: by transform path id
    let mut tr_ids: Vec<i64> = raw_tr.keys().copied().collect();
    tr_ids.sort();
    let mut tr_to_node: HashMap<i64, u32> = HashMap::new();
    let mut node_tr: Vec<i64> = Vec::new();
    // A player build serializes every RectTransform's m_LocalPosition as 0. For one whose parent is not a
    // RectTransform, Unity's localPosition.xy is its anchoredPosition (z is kept as serialized).
    let parent_has_rect = parent.is_some_and(|p| def.nodes[p as usize].rect.is_some());
    let unity_pos = |rt: &RawTr| -> Vec3 {
        let p = rt.trs.0;
        match &rt.rect {
            Some(r) if raw_tr.get(&rt.father).map_or(!parent_has_rect, |f| f.rect.is_none()) => Vec3::new(r.anchored_pos[0], r.anchored_pos[1], p.z),
            _ => p,
        }
    };
    for &t in &tr_ids {
        let rt = &raw_tr[&t];
        let Some(g) = raw_go.get(&rt.go) else { continue };
        if def.obj_to_node.contains_key(&rt.go) {
            return Err(crate::Error(format!("{}: object {} already in the scene (instantiated twice?)", file.name, rt.go)));
        }
        let idx = def.nodes.len() as u32;
        tr_to_node.insert(t, idx);
        node_tr.push(t);
        def.obj_to_node.insert(rt.go, idx);
        def.obj_to_node.insert(t, idx);
        let (_, q, s) = rt.trs;
        let p = unity_pos(rt);
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
            rect: rt.rect,
        });
    }
    let mut roots = Vec::new();
    for &t in &tr_ids {
        let Some(&n) = tr_to_node.get(&t) else { continue };
        let pf = tr_to_node.get(&raw_tr[&t].father).copied();
        if pf.is_none() {
            roots.push(n);
        }
        if let Some(pf) = pf.or(parent) {
            def.nodes[n as usize].parent = Some(pf);
            def.nodes[pf as usize].children.push(n);
        }
    }
    // sibling order is m_Children's, not path-id order
    for &t in &tr_ids {
        let Some(&n) = tr_to_node.get(&t) else { continue };
        let order: Vec<u32> = raw_tr[&t].children.iter().filter_map(|c| tr_to_node.get(c).copied()).collect();
        let ch = &mut def.nodes[n as usize].children;
        if order.len() == ch.len() {
            *ch = order;
        }
    }
    // world matrices in Unity space, computed top-down (existing nodes keep theirs)
    let mut unity_world: Vec<Mat4> = def.nodes.iter().map(|n| to_bevy_mat(n.world0)).collect();
    let mut order = roots.clone();
    let mut i = 0;
    while i < order.len() {
        let n = order[i];
        let rt = &raw_tr[&node_tr[n as usize - base]];
        let local = Mat4::from_scale_rotation_translation(rt.trs.2, rt.trs.1, unity_pos(rt));
        unity_world[n as usize] = match def.nodes[n as usize].parent {
            Some(p) => unity_world[p as usize] * local,
            None => local,
        };
        order.extend(def.nodes[n as usize].children.iter().copied());
        i += 1;
    }
    for n in base..def.nodes.len() {
        def.nodes[n].world0 = to_bevy_mat(unity_world[n]);
    }

    // --- components ---
    // Node order, not HashMap order: script/collider indices drive Awake/Start/Update order, so this must
    // be the same in every process for runs to be reproducible.
    let go_ids: Vec<i64> = node_tr.iter().map(|t| raw_tr[t].go).collect();
    for gid in go_ids {
        let Some(&node) = def.obj_to_node.get(&gid) else { continue };
        let comps: Vec<(i32, i64)> = raw_go[&gid].get("m_Component").array().iter().map(|c| c.get("component").pptr()).collect();
        let layer = def.nodes[node as usize].layer;
        let world = unity_world[node as usize];
        let mut filter_mesh = None;
        let mut renderer: Option<Value> = None;
        // SceneHelper.IsValidForPhysicsScene inputs: the first Collider's isTrigger, the
        // MeshCollider's mesh, a non-kinematic Rigidbody, a PreservedOriginalMesh
        let mut first_collider_trigger: Option<bool> = None;
        let mut collider_mesh = None;
        let mut dynamic_rb = false;
        let mut preserved_mesh = None;
        let (mut ps_system, mut ps_renderer) = (None, None);
        for &(f, id) in &comps {
            if f != 0 {
                continue;
            }
            let Some(o) = file.object(id) else { continue };
            match o.class_id {
                CLASS_MESH_FILTER => filter_mesh = file.read(o).ok().map(|v| v.get("m_Mesh").pptr()),
                CLASS_MESH_RENDERER => renderer = file.read(o).ok(),
                CLASS_SKINNED_MESH_RENDERER => {
                    let Ok(v) = file.read(o) else { continue };
                    if HIDDEN_LAYERS.contains(&layer) {
                        continue;
                    }
                    def.obj_to_node.insert(id, node);
                    let Some(mesh) = ld.mesh(v.get("m_Mesh").pptr()) else { continue };
                    let bone_nodes: Vec<Option<u32>> = v.get("m_Bones").array().iter().map(|b| tr_to_node.get(&b.pptr().1).copied()).collect();
                    let bones: Vec<Mat4> = bone_nodes.iter().map(|b| b.map(|bn| unity_world[bn as usize]).unwrap_or(world)).collect();
                    let posed = skin(&mesh, &bones);
                    let skin_def = (!mesh.skin.is_empty() && !bones.is_empty()).then(|| Arc::new(SkinDef { mesh: mesh.clone(), bones: bone_nodes }));
                    let mats: Vec<(i32, i64)> = v.get("m_Materials").array().iter().map(|m| m.pptr()).collect();
                    for (k, sm) in posed.submeshes.iter().enumerate() {
                        let material = mats.get(k).or(mats.last()).copied().and_then(|p| ld.material_key(p));
                        let mut batch = Batch::default();
                        bake(&mut batch, &posed, &sm.indices, Mat4::IDENTITY);
                        def.renderers.push(RenderDef { node, material, batch, enabled: v.get("m_Enabled").bool(), skin: skin_def.clone() });
                    }
                }
                CLASS_BOX_COLLIDER | CLASS_SPHERE_COLLIDER | CLASS_CAPSULE_COLLIDER | CLASS_MESH_COLLIDER => {
                    let Ok(v) = file.read(o) else { continue };
                    first_collider_trigger.get_or_insert(v.get("m_IsTrigger").bool());
                    if o.class_id == CLASS_MESH_COLLIDER {
                        collider_mesh = Some(v.get("m_Mesh").pptr());
                    }
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
                            // PhysX collides a convex MeshCollider as the hull of its vertices
                            // (flat meshes keep their triangles)
                            if v.get("m_Convex").bool() {
                                if let Some(hull) = crate::gltf_map::convex_hull(&tris) {
                                    tris = hull;
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
                    let Ok(v) = file.read(o) else { continue };
                    let class = ld.class_name(v.get("m_Script").pptr());
                    if class == "PreservedOriginalMesh" {
                        preserved_mesh = Some(v.get("mesh").pptr()).filter(|p| p.1 != 0);
                    }
                    def.obj_to_node.insert(id, node);
                    def.comp_to_script.insert(id, def.scripts.len() as u32);
                    def.scripts.push(ScriptDef { node, class, enabled: v.get("m_Enabled").bool(), path_id: id, data: v, file: prefab.clone() });
                }
                CLASS_ANIMATOR => {
                    let Ok(v) = file.read(o) else { continue };
                    def.obj_to_node.insert(id, node);
                    let controller = ld.controller(def, v.get("m_Controller").pptr());
                    def.animators.push(AnimatorDef {
                        node,
                        controller,
                        enabled: v.get("m_Enabled").bool(),
                        culling: v.get("m_CullingMode").i64() as u8,
                        update_mode: v.get("m_UpdateMode").i64() as u8,
                        root_motion: v.get("m_ApplyRootMotion").bool(),
                        keep_state_on_disable: v.get("m_KeepAnimatorStateOnDisable").bool(),
                        path_id: id,
                    });
                }
                CLASS_CANVAS | CLASS_CANVAS_GROUP => {
                    def.obj_to_node.insert(id, node);
                    let Ok(v) = file.read(o) else { continue };
                    def.ui_natives.push(UiNativeDef { node, class_id: o.class_id, path_id: id, data: v });
                }
                CLASS_RIGIDBODY => {
                    def.obj_to_node.insert(id, node);
                    def.rigidbodies.insert(node);
                    dynamic_rb |= file.read(o).is_ok_and(|v| !v.get("m_IsKinematic").bool());
                    if dynamic_rb {
                        def.dynamic_rigidbodies.insert(node);
                    }
                }
                CLASS_NAV_MESH_AGENT => {
                    def.obj_to_node.insert(id, node);
                    let Ok(v) = file.read(o) else { continue };
                    def.nav_agents.push(NavAgentDef {
                        node,
                        enabled: v.get("m_Enabled").bool(),
                        base_offset: v.get("m_BaseOffset").f32(),
                        speed: v.get("m_Speed").f32(),
                        acceleration: v.get("m_Acceleration").f32(),
                        angular_speed: v.get("m_AngularSpeed").f32(),
                        stopping_distance: v.get("m_StoppingDistance").f32(),
                        radius: v.get("m_Radius").f32(),
                        height: v.get("m_Height").f32(),
                        auto_braking: v.get("m_AutoBraking").bool(),
                        path_id: id,
                    });
                }
                CLASS_LIGHT => {
                    def.obj_to_node.insert(id, node);
                    let Ok(v) = file.read(o) else { continue };
                    let c = v.get("m_Color");
                    def.lights.push(LightDef {
                        node,
                        kind: v.get("m_Type").i64() as u8,
                        color: [c.get("r").f32(), c.get("g").f32(), c.get("b").f32(), c.get("a").f32()],
                        intensity: v.get("m_Intensity").f32(),
                        range: v.get("m_Range").f32(),
                        spot_angle: v.get("m_SpotAngle").f32(),
                        enabled: v.get("m_Enabled").bool(),
                        render_mode: v.get("m_RenderMode").i64() as u8,
                        culling_mask: v.get("m_CullingMask").get("m_Bits").i64() as u32,
                    });
                }
                CLASS_PARTICLE_SYSTEM => {
                    def.obj_to_node.insert(id, node);
                    let Ok(v) = file.read(o) else { continue };
                    ps_system = Some((id, crate::particles::ParticleSystemDef::read(&v)));
                }
                CLASS_PARTICLE_RENDERER => {
                    def.obj_to_node.insert(id, node);
                    let Ok(v) = file.read(o) else { continue };
                    let mats = v.get("m_Materials").array().iter().map(|m| ld.material_key(m.pptr())).collect();
                    ps_renderer = Some(crate::particles::ParticleRendererDef::read(&v, mats));
                }
                _ => {
                    def.obj_to_node.insert(id, node);
                }
            }
        }
        if let Some((id, mut sys)) = ps_system {
            let scene = ld.scene.clone();
            let shape_go = crate::particles::load_system_meshes(ld.db, &scene, &mut sys, ps_renderer.as_mut());
            def.comp_to_particle.insert(id, def.particle_systems.len() as u32);
            let planes = sys.collision.as_ref().map(|c| c.planes.iter().filter(|p| p.0 == 0).filter_map(|p| def.obj_to_node.get(&p.1).copied()).collect()).unwrap_or_default();
            def.particle_systems.push(SceneParticleDef {
                node,
                path_id: id,
                system: Arc::new(sys),
                renderer: ps_renderer.map(Arc::new),
                shape_node: shape_go.and_then(|g| def.obj_to_node.get(&g).copied()),
                planes,
            });
        }
        if let (Some(r), Some(false), false, true) = (&renderer, first_collider_trigger, dynamic_rb, FOOTSTEP_LAYERS.contains(&layer)) {
            if let Some(mesh) = preserved_mesh.or(collider_mesh).or(filter_mesh).filter(|p| p.1 != 0).and_then(|p| ld.mesh(p)) {
                let mats: Vec<(i32, i64)> = r.get("m_Materials").array().iter().map(|m| m.pptr()).collect();
                let start = r.get("m_StaticBatchInfo").get("firstSubMesh").i64() as usize;
                let sm = surface_mesh(ld, &file, node, layer, world, &mesh, &mats, start);
                def.surface_meshes.push(sm);
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
                def.renderers.push(RenderDef { node, material, batch, enabled: r.get("m_Enabled").bool(), skin: None });
            }
        }
    }
    Ok(roots.first().copied())
}

/// `Object.Instantiate(prefab, parent)`: the prefab whose root GameObject is `go` in `file`,
/// with its whole hierarchy and components, under `parent`. Returns the instance's root node.
fn instantiate(ld: &mut Loader, def: &mut SceneDef, file: Arc<SerializedFile>, go: i64, parent: u32) -> Result<u32> {
    let mut raw_go = HashMap::new();
    let mut raw_trs = HashMap::new();
    let mut stack = vec![go];
    while let Some(g) = stack.pop() {
        let v = file.read_id(g)?;
        for c in v.get("m_Component").array() {
            let (_, cid) = c.get("component").pptr();
            if !matches!(file.object(cid).map(|o| o.class_id), Some(CLASS_TRANSFORM | CLASS_RECT_TRANSFORM)) {
                continue;
            }
            let t = file.read_id(cid)?;
            for k in t.get("m_Children").array() {
                stack.push(file.read_id(k.pptr().1)?.get("m_GameObject").pptr().1);
            }
            let mut rt = raw_tr(&t);
            if g == go {
                rt.father = 0;
            }
            raw_trs.insert(cid, rt);
        }
        raw_go.insert(g, v);
    }
    let prev = std::mem::replace(&mut ld.scene, file);
    // class names are cached per PPtr, which is relative to the file
    let names = std::mem::take(&mut ld.class_names);
    let r = add_objects(ld, def, &raw_go, &raw_trs, Some(parent));
    ld.scene = prev;
    ld.class_names = names;
    r?.ok_or_else(|| crate::Error("prefab without a root transform".into()))
}

/// The viewmodel prefabs spawned at Awake from Addressables references:
/// GunSetter.ResetWeapons instantiates the equipped weapons under GunControl (in 0-1: the blue
/// Piercer revolver, the one weapon the player starts with); FistControl.ResetFists the blue
/// Feedbacker arm under Punch.
fn spawn_viewmodel(ld: &mut Loader, def: &mut SceneDef) {
    if !def.scripts.iter().any(|s| s.class == "GunSetter" || s.class == "FistControl") {
        return;
    }
    let cat = match ld.db.catalog.clone().map(Ok).unwrap_or_else(|| crate::addressables::Catalog::load(&ld.db.install.clone()).map(std::sync::Arc::new)) {
        Ok(c) => {
            ld.db.catalog = Some(c.clone());
            c
        }
        Err(e) => {
            def.warnings.push(format!("viewmodel: {e}"));
            return;
        }
    };
    // (spawning script, AssetReference field; arrays are [blue, green] variants)
    for (class, field) in [("GunSetter", "revolverPierce"), ("FistControl", "blueArm")] {
        let Some(s) = def.scripts.iter().position(|s| s.class == class) else { continue };
        let r = def.scripts[s].data.get(field);
        let r = r.array().first().unwrap_or(r);
        let (node, guid) = (def.scripts[s].node, r.get("m_AssetGUID").str().to_string());
        if guid.is_empty() {
            continue;
        }
        match cat.load_asset(ld.db, &guid).and_then(|(f, id)| instantiate(ld, def, f, id, node)) {
            // GunControl.SwitchWeapon / FistControl.ArmChange activate the equipped one
            Ok(root) => def.nodes[root as usize].active_self = true,
            Err(e) => def.warnings.push(format!("viewmodel {class}.{field}: {e}")),
        }
    }
}

/// CPU skinning with the bone poses saved in the scene (Unity space in, Unity space out).
/// Linear-blend skinning of every vertex: `bones` are bone world matrices (bind poses applied
/// here). Writes posed positions / normals (same space as `bones`) and whether each vertex's
/// dominant matrix mirrors; vertices without weights keep their bind-pose values.
pub fn skin_vertices(mesh: &MeshData, bones: &[Mat4], pos: &mut Vec<Vec3>, nrm: &mut Vec<Vec3>, mirrored: &mut Vec<bool>) {
    let mats: Vec<Mat4> = bones.iter().enumerate().map(|(i, b)| *b * mesh.bind_poses.get(i).copied().unwrap_or(Mat4::IDENTITY)).collect();
    let normal_mats: Vec<Mat4> = mats.iter().map(|m| m.inverse().transpose()).collect();
    pos.clear();
    nrm.clear();
    mirrored.clear();
    pos.extend(mesh.positions.iter().map(|p| Vec3::from(*p)));
    nrm.extend((0..mesh.positions.len()).map(|i| mesh.normals.get(i).map(|n| Vec3::from(*n)).unwrap_or(Vec3::Y)));
    mirrored.resize(mesh.positions.len(), false);
    for (vi, (idx, w)) in mesh.skin.iter().enumerate().take(pos.len()) {
        let (p, n) = (pos[vi], nrm[vi]);
        let (mut pp, mut nn, mut tw) = (Vec3::ZERO, Vec3::ZERO, 0.0);
        let mut dominant = (0.0f32, false);
        for k in 0..4 {
            if w[k] <= 0.0 {
                continue;
            }
            let i = idx[k] as usize;
            let m = mats.get(i).copied().unwrap_or(Mat4::IDENTITY);
            pp += m.transform_point3(p) * w[k];
            nn += normal_mats.get(i).copied().unwrap_or(Mat4::IDENTITY).transform_vector3(n) * w[k];
            tw += w[k];
            if w[k] > dominant.0 {
                dominant = (w[k], m.determinant() < 0.0);
            }
        }
        if tw > 0.0 {
            pos[vi] = pp / tw;
            nrm[vi] = nn.normalize_or_zero();
            mirrored[vi] = dominant.1;
        }
    }
}

fn skin(mesh: &MeshData, bones: &[Mat4]) -> MeshData {
    let mut out = mesh.clone();
    if mesh.skin.is_empty() || bones.is_empty() {
        return out;
    }
    let (mut pos, mut nrm, mut mirrored) = (Vec::new(), Vec::new(), Vec::new());
    skin_vertices(mesh, bones, &mut pos, &mut nrm, &mut mirrored);
    for (vi, p) in pos.iter().enumerate() {
        out.positions[vi] = p.to_array();
        if vi < out.normals.len() {
            out.normals[vi] = nrm[vi].to_array();
        }
    }
    // A reflecting skin matrix reverses a triangle's orientation; reverse its winding back so the
    // posed mesh (baked with an identity transform) keeps Unity's front faces.
    for sm in &mut out.submeshes {
        for t in sm.indices.chunks_exact_mut(3) {
            let m = t.iter().filter(|&&i| mirrored.get(i as usize).copied().unwrap_or(false)).count();
            if m >= 2 {
                t.swap(1, 2);
            }
        }
    }
    out
}

/// A shape moved by a rigid transform (`pt` maps points, `dr` is its rotation).
fn transform_shape(shape: &mut ShapeDef, pt: &dyn Fn(Vec3) -> Vec3, dr: Quat) {
    match shape {
        ShapeDef::Box { center, rot, .. } => {
            *center = pt(*center);
            *rot = dr * *rot;
        }
        ShapeDef::Sphere { center, .. } => *center = pt(*center),
        ShapeDef::Capsule { a, b, .. } => {
            *a = pt(*a);
            *b = pt(*b);
        }
        ShapeDef::Mesh(t) => t.iter_mut().flatten().for_each(|p| *p = pt(*p)),
    }
}

/// Points scene-file PPtrs (m_FileID 0) whose path id is in `map` at the mapped id.
fn remap_pptrs(v: &mut Value, map: &HashMap<i64, i64>) {
    match v {
        Value::Struct(fields) => {
            let is_ptr = fields.iter().any(|(k, v)| &**k == "m_FileID" && v.i64() == 0) && fields.iter().any(|(k, _)| &**k == "m_PathID");
            for (k, f) in fields.iter_mut() {
                if is_ptr && &**k == "m_PathID" {
                    if let Some(&n) = map.get(&f.i64()) {
                        *f = Value::Int(n);
                    }
                } else {
                    remap_pptrs(f, map);
                }
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|x| remap_pptrs(x, map)),
        _ => {}
    }
}
