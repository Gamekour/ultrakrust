//! Custom maps from a glTF file (`--map`), with Godot's import name suffixes
//! (docs.godotengine.org: importing_3d_scenes/node_type_customization):
//!
//! - `-noimp`: the node and its children are not imported
//! - `-col` / `-convcol`: the mesh plus a trimesh / convex collider
//! - `-colonly` / `-convcolonly`: only the collider (an empty: a shape by its draw type, below)
//! - `-navmesh`: the enemies' walkable surface (invisible, no collision; several merge into one)
//! - `-occ`, `-occonly`, `-rigid`, `-vehicle`, `-wheel`: no equivalent yet (warned; `-occonly`
//!   still drops the mesh)
//!
//! Suffixes match case-insensitively as `-x` / `_x` at the end of the name (trailing digits,
//! dots and spaces ignored) or `$x` anywhere, and are stripped from the node name, as in Godot.
//! glTF does not store an empty's draw type: a `-colonly` empty reads it from its extras
//! (`empty_display_type` / `empty_draw_type`: CUBE -> box of size 2, IMAGE -> world boundary,
//! SINGLE_ARROW -> ray, which Unity has no collider for, anything else -> sphere of radius 1).
//!
//! Cameras are not imported. Lights (KHR_lights_punctual) become Unity lights by type. Meshes draw
//! with 0-2's exit elevator floor material (ULTRAKILL/Master); a glTF material overrides its base
//! colour (`_MainTex` x `_Color`) and emission (EMISSIVE: `_EmissiveTex` x `_EmissiveColor`,
//! `_EmissiveIntensity` from KHR_materials_emissive_strength). Smoothness and metallic are taken
//! as 0 (the shader has no such inputs outside REFLECTION). Every collider is a static
//! Environment (layer 8) collider, and the collision geometry is tagged as a metal surface.
//!
//! The rest of the scene comes from 0-1: its player rig (HUD included), GameController managers,
//! OnLevelStart, StatsManager and EventSystem, with the player moved to stand at the origin.
//!
//! ULTRAKILL suffixes (docs/MAP_FORMAT.md), on empties: `-filth`, `-stray` and `-maliciousface`
//! place a copy of one of 0-1's enemies at the empty, standing on its origin and facing its
//! forward (glTF -Z: Blender's +Y), in the level from the start (no spawn effect). Without a
//! `-navmesh`, Filth and Strays walk straight at the player (warned). `-room` collections and
//! `-door` meshes load and unload rooms as the campaign's doors do (see `rooms`). An `-env` empty
//! sets the skybox, fog and ambient light (see `env`).

use crate::db::AssetDb;
use crate::scene::{Batch, MaterialKey};
use crate::scenedef::{ColliderDef, LightDef, MaterialOverride, NodeDef, RenderDef, SceneDef, ShapeDef, SurfaceMat, SurfaceMeshDef};
use crate::texture::TextureData;
use std::sync::Arc;
use crate::{Error, Result};
use bevy_math::{Mat4, Quat, Vec3};
use std::collections::HashMap;
use std::path::Path;

mod env;
mod rooms;

/// TagManager "Environment".
pub const MAP_LAYER: u8 = 8;
/// SurfaceType.MetalHigh, used when the placeholder material carries no metal surface type.
pub const METAL: i32 = 16;
/// 0-2's exit elevator floor (ElevatorController/Elevator, the largest upward-facing submesh).
const PLACEHOLDER_FILE: &str = "CAB-56872c896f0e5b74fc3bff932eebbfda";
const PLACEHOLDER_NAME: &str = "Metal Pattern 2 15";
const PLACEHOLDER_ID: i64 = 4123375003429657669;

/// What `append_gltf` added.
#[derive(Default, Debug)]
pub struct MapReport {
    pub nodes: usize,
    pub renderers: usize,
    pub colliders: usize,
    pub lights: usize,
    pub materials: usize,
    pub textures: usize,
    pub tris: usize,
    pub enemies: usize,
    /// navmesh polygons (from `-navmesh` triangles)
    pub nav_polys: usize,
    pub rooms: usize,
    pub doors: usize,
    /// the RenderSettings `-env` changed
    pub env: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum EnemyKind {
    Filth,
    Stray,
    MaliciousFace,
}

impl EnemyKind {
    const ALL: [(&'static str, EnemyKind); 3] = [("filth", EnemyKind::Filth), ("stray", EnemyKind::Stray), ("maliciousface", EnemyKind::MaliciousFace)];

    /// The 0-1 enemy each one is copied from (its EnemyIdentifier root, or the boss's own root).
    fn template(self) -> &'static str {
        match self {
            EnemyKind::Filth => "11 - Projectile Zombies Room/11 Content/Enemies/Wave 1/Filth/Zombie",
            EnemyKind::Stray => "10 - Combo Hallway/10 Content/Enemies/Wave 1/Projectile Zombie",
            EnemyKind::MaliciousFace => "13 - Malicious Face Arena/13 Content/Boss/Spider",
        }
    }
}

/// 0-1 pruned to the player, its managers and HUD, with the glTF scene added on top.
pub fn custom_scene(db: &mut AssetDb, mut def: SceneDef, path: &Path) -> Result<(SceneDef, MapReport)> {
    // 0-1's humanoid agent settings, before pruning drops its navmesh
    let agent = def.navmeshes.iter().filter(|m| m.agent_type == 0).max_by_key(|m| m.tiles.len()).cloned();
    let door_tpl = rooms::templates(&def);
    prune_base(&mut def)?;
    let (mut report, parsed) = append_gltf_with(db, &mut def, path)?;
    let Parsed { root, enemies, nav_tris, rooms, doors, envs } = parsed;
    env::apply(db, &mut def, &envs, &mut report);
    place_enemies(&mut def, &enemies, &mut report);
    // prune_base stands the player's feet on the origin
    rooms::setup(&mut def, root, &rooms, &doors, door_tpl.as_ref(), Vec3::Y, &mut report);
    let mut nav = None;
    if !nav_tris.is_empty() {
        let (mesh, dropped) = crate::navmesh::from_triangles("glTF navmesh", &nav_tris, agent.as_ref());
        if dropped > 0 {
            report.warnings.push(format!("-navmesh: {dropped} triangles skipped (vertical, degenerate, or past {} per map)", crate::navmesh::MAX_TILE_ITEMS));
        }
        report.nav_polys = mesh.tiles[0].polys.len();
        nav = Some(mesh);
    }
    if report.nav_polys == 0 && enemies.iter().any(|(k, _)| *k != EnemyKind::MaliciousFace) {
        report.warnings.push("no -navmesh: Filth and Strays walk straight at the player instead of pathing".into());
    }
    // the templates go, with the 0-1 rooms around them
    let chain = template_chain(&def);
    def.retain_nodes(|n| !chain[n as usize]);
    // (retain_nodes clears navmeshes)
    def.navmeshes = nav.into_iter().collect();
    def.warnings.extend(report.warnings.iter().cloned());
    Ok((def, report))
}

fn template_nodes(def: &SceneDef) -> Vec<(EnemyKind, Option<u32>)> {
    EnemyKind::ALL.iter().map(|&(_, k)| (k, (0..def.nodes.len() as u32).find(|&n| def.path(n) == k.template()))).collect()
}

/// The enemy templates, their ancestors and their subtrees.
fn template_chain(def: &SceneDef) -> Vec<bool> {
    let mut chain = vec![false; def.nodes.len()];
    for (_, t) in template_nodes(def) {
        let Some(t) = t else { continue };
        let mut p = def.nodes[t as usize].parent;
        while let Some(q) = p {
            chain[q as usize] = true;
            p = def.nodes[q as usize].parent;
        }
        for n in 0..def.nodes.len() as u32 {
            if def.is_descendant(n, t) {
                chain[n as usize] = true;
            }
        }
    }
    chain
}

/// A copy of the kind's template at each placement: on the empty's origin, turned to face its
/// forward, active, and without the spawn effect.
fn place_enemies(def: &mut SceneDef, enemies: &[(EnemyKind, u32)], report: &mut MapReport) {
    let templates = template_nodes(def);
    for &(kind, at) in enemies {
        let Some(t) = templates.iter().find(|(k, _)| *k == kind).and_then(|(_, t)| *t) else {
            report.warnings.push(format!("{}: 0-1 has no {} to copy ({})", def.nodes[at as usize].name, kind.template(), "enemy skipped"));
            continue;
        };
        let (ts, tr, _) = def.nodes[t as usize].world0.to_scale_rotation_translation();
        let (_, pr, pp) = def.nodes[at as usize].world0.to_scale_rotation_translation();
        let yaw = |r: Quat| {
            let f = r * Vec3::NEG_Z;
            f.x.atan2(-f.z)
        };
        // a turn about +Y by a lowers this yaw by a
        let rot = Quat::from_rotation_y(yaw(tr) - yaw(pr)) * tr;
        let root = def.clone_subtree(t, at, Mat4::from_scale_rotation_translation(ts, rot, pp));
        def.nodes[root as usize].active_self = true;
        // the copy's nodes are the last ones added
        let n = def.nodes.len() as u32;
        for s in def.scripts.iter_mut().filter(|s| s.class == "EnemyIdentifier" && s.node >= root && s.node < n) {
            set_field(&mut s.data, "spawnIn", false);
        }
        report.enemies += 1;
    }
}

/// Keeps FirstRoom (minus its Room geometry and the PlayerActivator Cube), StatsManager and
/// EventSystem, and moves FirstRoom so the player stands at the origin facing -Z (glTF forward).
fn prune_base(def: &mut SceneDef) -> Result<()> {
    let first = def.nodes.iter().position(|n| n.parent.is_none() && n.name == "FirstRoom").ok_or_else(|| Error("base scene has no FirstRoom".into()))? as u32;
    // the enemy templates stay (with their ancestors) until custom_scene has copied them
    let chain = template_chain(def);
    let keep: Vec<bool> = (0..def.nodes.len())
        .map(|i| {
            let n = &def.nodes[i];
            let mut root = i;
            while let Some(q) = def.nodes[root].parent {
                root = q as usize;
            }
            if chain[root] {
                return chain[i];
            }
            match n.parent {
                None => matches!(n.name.as_str(), "FirstRoom" | "StatsManager" | "EventSystem"),
                Some(p) if p == first => !matches!(n.name.as_str(), "Room" | "Cube"),
                Some(_) => true,
            }
        })
        .collect();
    def.retain_nodes(|n| keep[n as usize]);
    let first = def.find("FirstRoom").unwrap();
    let player = def.scripts.iter().find(|s| s.class == "NewMovement").map(|s| s.node).ok_or_else(|| Error("base scene has no NewMovement".into()))?;
    let (_, r, t) = def.nodes[player as usize].world0.to_scale_rotation_translation();
    // NewMovement's capsule reaches 1.5 below the transform: feet on the origin
    let d = Mat4::from_translation(Vec3::new(0.0, 1.5, 0.0)) * Mat4::from_quat(r.inverse()) * Mat4::from_translation(-t);
    def.move_root(first, d);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Col {
    None,
    Tri,
    Convex,
}

/// Godot's `_teststr`: trailing digits / dots / spaces ignored; `$x` anywhere, `-x` / `_x` at the end.
fn has_suffix(name: &str, s: &str) -> bool {
    let what = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c <= ' ').to_lowercase();
    what.contains(&format!("${s}")) || what.ends_with(&format!("-{s}")) || what.ends_with(&format!("_{s}"))
}

/// Godot's `_fixstr`: the name without the suffix.
fn strip_suffix(name: &str, s: &str) -> String {
    let what = name.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.' || c <= ' ');
    let low = what.to_lowercase();
    for pre in ['-', '_', '$'] {
        let suf = format!("{pre}{s}");
        if low.ends_with(&suf) {
            return what[..what.len() - suf.len()].trim_end().to_string();
        }
    }
    if let Some(i) = low.find(&format!("${s}")) {
        return format!("{}{}", &what[..i], &what[i + 1 + s.len()..]).trim().to_string();
    }
    name.to_string()
}

struct NodeKind {
    name: String,
    noimp: bool,
    col: Col,
    /// the mesh is not drawn
    only: bool,
    enemy: Option<EnemyKind>,
    /// the mesh is navmesh (not drawn, no collision)
    navmesh: bool,
    room: bool,
    door: bool,
    env: bool,
}

fn classify(raw: &str, warnings: &mut Vec<String>) -> NodeKind {
    let mut k = NodeKind { name: raw.to_string(), noimp: false, col: Col::None, only: false, enemy: None, navmesh: false, room: false, door: false, env: false };
    if has_suffix(raw, "noimp") {
        k.noimp = true;
        return k;
    }
    if let Some(&(s, e)) = EnemyKind::ALL.iter().find(|(s, _)| has_suffix(raw, s)) {
        k.name = strip_suffix(raw, s);
        k.enemy = Some(e);
        return k;
    }
    if has_suffix(raw, "env") {
        k.name = strip_suffix(raw, "env");
        k.env = true;
        return k;
    }
    if has_suffix(raw, "room") {
        k.name = strip_suffix(raw, "room");
        k.room = true;
        return k;
    }
    // a door collides as its convex hull
    if has_suffix(raw, "door") {
        k.name = strip_suffix(raw, "door");
        k.door = true;
        k.col = Col::Convex;
        return k;
    }
    let found = [("convcolonly", Col::Convex, true), ("colonly", Col::Tri, true), ("convcol", Col::Convex, false), ("col", Col::Tri, false)].into_iter().find(|(s, ..)| has_suffix(raw, s));
    if let Some((s, col, only)) = found {
        k.name = strip_suffix(raw, s);
        k.col = col;
        k.only = only;
        return k;
    }
    if has_suffix(raw, "navmesh") {
        k.name = strip_suffix(raw, "navmesh");
        k.navmesh = true;
        k.only = true;
        return k;
    }
    for (s, drop_mesh) in [("occonly", true), ("occ", false), ("vehicle", false), ("wheel", false), ("rigid", false)] {
        if has_suffix(raw, s) {
            warnings.push(format!("{raw}: -{s} has no equivalent yet, ignored"));
            k.name = strip_suffix(raw, s);
            k.only = drop_mesh;
            return k;
        }
    }
    k
}

/// Sets a serialized bool field (stored as a bool or an int) on a struct value.
fn set_field(v: &mut crate::serialized::Value, key: &str, on: bool) {
    use crate::serialized::Value;
    let Value::Struct(fields) = v else { return };
    if let Some((_, f)) = fields.iter_mut().find(|(k, _)| &**k == key) {
        *f = match f {
            Value::Bool(_) => Value::Bool(on),
            Value::UInt(_) => Value::UInt(on as u64),
            _ => Value::Int(on as i64),
        };
    }
}

/// Sets a serialized field on a struct value, adding it when missing.
fn set_value(v: &mut crate::serialized::Value, key: &str, val: crate::serialized::Value) {
    let crate::serialized::Value::Struct(fields) = v else { return };
    match fields.iter_mut().find(|(k, _)| &**k == key) {
        Some((_, f)) => *f = val,
        None => fields.push((key.into(), val)),
    }
}

fn extras_str(extras: &Option<Box<serde_json::value::RawValue>>, keys: &[&str]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(extras.as_ref()?.get()).ok()?;
    keys.iter().find_map(|k| v.get(*k)?.as_str().map(str::to_uppercase))
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

struct Ctx<'a> {
    def: &'a mut SceneDef,
    buffers: Vec<Vec<u8>>,
    material: Option<MaterialKey>,
    surface: SurfaceMat,
    report: MapReport,
    next_id: i64,
    dir: std::path::PathBuf,
    /// glTF material -> index into `material_overrides`
    mats: HashMap<usize, u32>,
    /// glTF texture -> decoded image (None: undecodable)
    images: HashMap<usize, Option<Arc<TextureData>>>,
    white: Option<Arc<TextureData>>,
    /// enemy placements: (kind, node)
    enemies: Vec<(EnemyKind, u32)>,
    /// `-navmesh` triangles, world space
    nav_tris: Vec<[Vec3; 3]>,
    /// `-room` nodes
    rooms: Vec<u32>,
    doors: Vec<rooms::DoorProps>,
    envs: Vec<(String, Option<String>)>,
}

/// Adds the glTF file's default scene under a new root node.
pub fn append_gltf(db: &mut AssetDb, def: &mut SceneDef, path: &Path) -> Result<MapReport> {
    let (report, ..) = append_gltf_with(db, def, path)?;
    def.warnings.extend(report.warnings.iter().cloned());
    Ok(report)
}

/// What `append_gltf_with` found for `custom_scene` to wire up.
struct Parsed {
    /// the glTF scene's root node
    root: u32,
    enemies: Vec<(EnemyKind, u32)>,
    /// `-navmesh` triangles, Bevy world space
    nav_tris: Vec<[Vec3; 3]>,
    rooms: Vec<u32>,
    doors: Vec<rooms::DoorProps>,
    /// `-env` empties: (name, extras JSON)
    envs: Vec<(String, Option<String>)>,
}

/// `append_gltf`, also returning what the ULTRAKILL suffixes placed; warnings are left to the
/// caller.
fn append_gltf_with(db: &mut AssetDb, def: &mut SceneDef, path: &Path) -> Result<(MapReport, Parsed)> {
    let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let g = gltf::Gltf::from_slice(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))?;
    let mut warnings = Vec::new();
    let buffers = g
        .buffers()
        .map(|b| match b.source() {
            gltf::buffer::Source::Bin => g.blob.clone().unwrap_or_default(),
            gltf::buffer::Source::Uri(uri) if !uri.starts_with("data:") => std::fs::read(path.parent().unwrap_or(Path::new(".")).join(uri)).unwrap_or_else(|e| {
                warnings.push(format!("buffer {uri}: {e}"));
                Vec::new()
            }),
            gltf::buffer::Source::Uri(_) => {
                warnings.push("embedded data: URI buffers are not supported (export as .glb or with a separate .bin)".into());
                Vec::new()
            }
        })
        .collect();
    let (material, surface) = placeholder(db);
    if material.is_none() {
        warnings.push(format!("placeholder material {PLACEHOLDER_NAME} not found: meshes draw with the fallback"));
    }
    let root = def.nodes.len() as u32;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    def.nodes.push(NodeDef {
        name: format!("glTF {stem}"),
        parent: None,
        children: Vec::new(),
        active_self: true,
        layer: MAP_LAYER,
        tag: 0,
        local_pos: Vec3::ZERO,
        local_rot: Quat::IDENTITY,
        local_scale: Vec3::ONE,
        world0: Mat4::IDENTITY,
        rect: None,
    });
    let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
    let mut cx = Ctx { def, buffers, material, surface, report: MapReport { nodes: 1, warnings, ..Default::default() }, next_id: -0x6c74_6700_0000, dir, mats: HashMap::new(), images: HashMap::new(), white: None, enemies: Vec::new(), nav_tris: Vec::new(), rooms: Vec::new(), doors: Vec::new(), envs: Vec::new() };
    let scene = g.default_scene().or_else(|| g.scenes().next()).ok_or_else(|| Error("glTF has no scene".into()))?;
    for n in scene.nodes() {
        visit(&mut cx, &n, root, Mat4::IDENTITY);
    }
    let parsed = Parsed { root, enemies: cx.enemies, nav_tris: cx.nav_tris, rooms: cx.rooms, doors: cx.doors, envs: cx.envs };
    let mut report = cx.report;
    // Blender's exporter leaves lights out unless asked to
    if report.lights == 0 {
        report.warnings.push("no lights in the file (Blender: export with Include > Data > Punctual Lights)".into());
    }
    report.warnings.dedup();
    Ok((report, parsed))
}

/// The placeholder material and the surface its collision geometry is tagged with (its own
/// `_SurfaceType` when that is a metal one, else MetalHigh).
fn placeholder(db: &mut AssetDb) -> (Option<MaterialKey>, SurfaceMat) {
    let mut surface = SurfaceMat { has_surface: true, surface: METAL, color: [1.0; 4], secondary: 0, secondary_color: [1.0; 4], vertex_blending: false, static_lighting: false };
    let Ok(f) = db.file(PLACEHOLDER_FILE) else { return (None, surface) };
    // the floor renderer's own material; by name should the file's ids ever change
    let by_id = f.read_id(PLACEHOLDER_ID).ok().filter(|v| v.get("m_Name").str() == PLACEHOLDER_NAME).map(|v| (PLACEHOLDER_ID, v));
    let found = by_id.or_else(|| {
        f.objects.iter().filter(|o| o.class_id == 21).find_map(|o| {
            let v = f.read(o).ok()?;
            (v.get("m_Name").str() == PLACEHOLDER_NAME).then_some((o.path_id, v))
        })
    });
    let Some((id, v)) = found else { return (None, surface) };
    let m = crate::shader::MaterialProps::from_value(&v);
    if let Some(&s) = m.floats.get("_SurfaceType") {
        let s = s.round() as i32;
        if matches!(s, 8 | 9 | 10 | 16 | 17) {
            surface.surface = s;
        }
    }
    if let Some(&c) = m.colors.get("_EnviroParticleColor") {
        surface.color = c;
    }
    (Some(MaterialKey { file: PLACEHOLDER_FILE.to_string(), path_id: id }), surface)
}

fn visit(cx: &mut Ctx, n: &gltf::Node, parent: u32, parent_world: Mat4) {
    let raw = n.name().map(str::to_string).unwrap_or_else(|| format!("node{}", n.index()));
    let kind = classify(&raw, &mut cx.report.warnings);
    if kind.noimp {
        return;
    }
    let (t, r, s) = n.transform().decomposed();
    let local = Mat4::from_cols_array_2d(&n.transform().matrix());
    let world = parent_world * local;
    let idx = cx.def.nodes.len() as u32;
    cx.def.nodes.push(NodeDef {
        name: kind.name.clone(),
        parent: Some(parent),
        children: Vec::new(),
        active_self: true,
        layer: MAP_LAYER,
        tag: 0,
        local_pos: Vec3::from(t),
        local_rot: Quat::from_array(r).normalize(),
        local_scale: Vec3::from(s),
        world0: world,
        rect: None,
    });
    cx.def.nodes[parent as usize].children.push(idx);
    cx.report.nodes += 1;
    if n.camera().is_some() {
        cx.report.warnings.push(format!("{raw}: camera not imported"));
    }
    if let Some(e) = kind.enemy {
        if n.mesh().is_some() {
            cx.report.warnings.push(format!("{raw}: an enemy goes on an empty; the mesh is not imported"));
        }
        cx.enemies.push((e, idx));
    }
    if kind.room {
        cx.rooms.push(idx);
    }
    if kind.env {
        match n.mesh() {
            Some(_) => cx.report.warnings.push(format!("{raw}: -env goes on an empty; its settings are ignored")),
            None => cx.envs.push((raw.clone(), n.extras().as_ref().map(|e| e.get().to_string()))),
        }
    }
    if kind.door {
        match n.mesh() {
            Some(_) => cx.doors.push(rooms::DoorProps::read(idx, n.extras())),
            None => cx.report.warnings.push(format!("{raw}: -door goes on a mesh; ignored")),
        }
    }
    match n.mesh() {
        Some(_) if kind.enemy.is_some() => {}
        Some(mesh) => add_mesh(cx, &mesh, idx, world, &kind),
        None if kind.door => {}
        None if kind.navmesh => cx.report.warnings.push(format!("{raw}: -navmesh goes on a mesh; ignored")),
        None if kind.col != Col::None => add_empty_shape(cx, n, idx, world, &raw),
        None => {}
    }
    if let Some(l) = n.light() {
        add_light(cx, &l, idx);
    }
    for c in n.children() {
        visit(cx, &c, idx, world);
    }
}

fn add_mesh(cx: &mut Ctx, mesh: &gltf::Mesh, node: u32, world: Mat4, kind: &NodeKind) {
    let flip = world.determinant() < 0.0;
    let nmat = world.inverse().transpose();
    let mut col_tris: Vec<[Vec3; 3]> = Vec::new();
    for prim in mesh.primitives() {
        if prim.mode() != gltf::mesh::Mode::Triangles {
            cx.report.warnings.push(format!("{}: primitive mode {:?} skipped (triangles only)", kind.name, prim.mode()));
            continue;
        }
        let rd = prim.reader(|b| cx.buffers.get(b.index()).map(Vec::as_slice));
        let Some(pos) = rd.read_positions() else { continue };
        let pos: Vec<Vec3> = pos.map(|p| world.transform_point3(Vec3::from(p))).collect();
        let idx: Vec<u32> = match rd.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..pos.len() as u32).collect(),
        };
        let tris: Vec<[u32; 3]> = idx.chunks_exact(3).filter(|t| t.iter().all(|&i| (i as usize) < pos.len())).map(|t| [t[0], t[1], t[2]]).collect();
        // front faces counter-clockwise (a mirroring node transform turns them around)
        col_tris.extend(tris.iter().map(|t| if flip { [pos[t[0] as usize], pos[t[2] as usize], pos[t[1] as usize]] } else { t.map(|i| pos[i as usize]) }));
        if kind.only {
            continue;
        }
        let normals: Vec<[f32; 3]> = match rd.read_normals() {
            Some(nr) => nr.map(|v| nmat.transform_vector3(Vec3::from(v)).normalize_or_zero().into()).collect(),
            None => {
                let mut acc = vec![Vec3::ZERO; pos.len()];
                for t in &tris {
                    let fnrm = (pos[t[1] as usize] - pos[t[0] as usize]).cross(pos[t[2] as usize] - pos[t[0] as usize]);
                    t.iter().for_each(|&i| acc[i as usize] += fnrm);
                }
                acc.iter().map(|v| v.normalize_or_zero().into()).collect()
            }
        };
        // Unity's v runs up, glTF's down
        let uvs: Vec<[f32; 2]> = match rd.read_tex_coords(0) {
            Some(uv) => uv.into_f32().map(|[u, v]| [u, 1.0 - v]).collect(),
            None => vec![[0.0, 0.0]; pos.len()],
        };
        let indices = tris.iter().flat_map(|t| if flip { [t[0], t[2], t[1]] } else { *t }).collect();
        let batch = Batch {
            positions: pos.iter().map(|p| (*p).into()).collect(),
            normals,
            uvs,
            colors: vec![[1.0; 4]; pos.len()],
            indices,
            src: (0..pos.len() as u32).collect(),
        };
        cx.report.tris += tris.len();
        cx.report.renderers += 1;
        if let (Some(mi), Some(_)) = (prim.material().index(), &cx.material) {
            if prim.material().pbr_metallic_roughness().base_color_texture().is_some_and(|t| t.tex_coord() != 0) {
                cx.report.warnings.push(format!("{}: only UV set 0 is imported", kind.name));
            }
            let ov = material_override(cx, &prim.material(), mi);
            cx.def.renderer_override.insert(cx.def.renderers.len() as u32, ov);
        }
        cx.def.renderers.push(RenderDef { node, material: cx.material.clone(), batch, enabled: true, skin: None });
    }
    if kind.navmesh {
        cx.nav_tris.extend(col_tris.iter().copied());
    }
    match kind.col {
        Col::None => {}
        Col::Tri => add_collider(cx, node, ShapeDef::Mesh(col_tris.clone()), col_tris),
        Col::Convex => {
            let hull = convex_hull(&col_tris).unwrap_or_else(|| {
                cx.report.warnings.push(format!("{}: degenerate convex hull, using the triangles", kind.name));
                col_tris
            });
            add_collider(cx, node, ShapeDef::Mesh(hull.clone()), hull);
        }
    }
}

/// The glTF material as overrides on the placeholder: base colour and emission. glTF factors are
/// linear, ULTRAKILL renders in gamma space; texture bytes are used as they are (sRGB both).
fn material_override(cx: &mut Ctx, m: &gltf::Material, mi: usize) -> u32 {
    if let Some(&i) = cx.mats.get(&mi) {
        return i;
    }
    let srgb = |c: [f32; 3], a: f32| [linear_to_srgb(c[0]), linear_to_srgb(c[1]), linear_to_srgb(c[2]), a];
    let pbr = m.pbr_metallic_roughness();
    let [r, g, b, a] = pbr.base_color_factor();
    let main = match pbr.base_color_texture() {
        Some(t) => image(cx, &t.texture()),
        None => None,
    };
    let main = main.unwrap_or_else(|| white(cx));
    let mut ov = MaterialOverride {
        name: m.name().unwrap_or("glTF material").to_string(),
        keywords: Vec::new(),
        floats: vec![("_Metallic".into(), 0.0), ("_Glossiness".into(), 0.0)],
        colors: vec![("_Color".into(), srgb([r, g, b], a))],
        textures: vec![("_MainTex".into(), main)],
    };
    let e = m.emissive_factor();
    let etex = m.emissive_texture().and_then(|t| image(cx, &t.texture()));
    if e.iter().any(|&c| c > 0.0) {
        ov.keywords.push("EMISSIVE".into());
        let etex = etex.unwrap_or_else(|| white(cx));
        ov.textures.push(("_EmissiveTex".into(), etex));
        ov.colors.push(("_EmissiveColor".into(), srgb(e, 1.0)));
        let strength = m.emissive_strength().unwrap_or(1.0);
        for (k, v) in [("EMISSIVE", 1.0), ("_EmissiveIntensity", strength), ("_UseAlbedoAsEmissive", 0.0), ("_EmissiveReplaces", 0.0), ("_EmissiveMask", 0.0), ("_EmissiveToVertexColors", 0.0)] {
            ov.floats.push((k.into(), v));
        }
    }
    cx.def.material_overrides.push(ov);
    cx.report.materials += 1;
    let i = cx.def.material_overrides.len() as u32 - 1;
    cx.mats.insert(mi, i);
    i
}

fn white(cx: &mut Ctx) -> Arc<TextureData> {
    cx.white.get_or_insert_with(|| Arc::new(TextureData { name: "white".into(), width: 1, height: 1, rgba: vec![255; 4], filter: 0, wrap: 0, layers: 1 })).clone()
}

/// A glTF texture decoded (PNG / JPEG), rows flipped to Unity's bottom-first order, with its
/// sampler's filter (NEAREST -> point, else bilinear) and wrap (S axis: repeat / clamp / mirror).
fn image(cx: &mut Ctx, t: &gltf::Texture) -> Option<Arc<TextureData>> {
    let ti = t.index();
    if let Some(x) = cx.images.get(&ti) {
        return x.clone();
    }
    let img = t.source();
    let bytes = match img.source() {
        gltf::image::Source::View { view, .. } => cx.buffers.get(view.buffer().index()).and_then(|b| b.get(view.offset()..view.offset() + view.length())).map(<[u8]>::to_vec),
        gltf::image::Source::Uri { uri, .. } if !uri.starts_with("data:") => std::fs::read(cx.dir.join(uri)).ok(),
        gltf::image::Source::Uri { .. } => None,
    };
    let name = img.name().map(str::to_string).unwrap_or_else(|| format!("image{}", img.index()));
    let out = match bytes.map(|b| image::load_from_memory(&b)) {
        Some(Ok(d)) => {
            let d = image::imageops::flip_vertical(&d.to_rgba8());
            let s = t.sampler();
            use gltf::texture::{MagFilter, WrappingMode};
            let filter = if s.mag_filter() == Some(MagFilter::Nearest) { 0 } else { 1 };
            let wrap = match s.wrap_s() {
                WrappingMode::ClampToEdge => 1,
                WrappingMode::MirroredRepeat => 2,
                WrappingMode::Repeat => 0,
            };
            cx.report.textures += 1;
            Some(Arc::new(TextureData { name, width: d.width(), height: d.height(), rgba: d.into_raw(), filter, wrap, layers: 1 }))
        }
        Some(Err(e)) => {
            cx.report.warnings.push(format!("texture {name}: {e}"));
            None
        }
        None => {
            cx.report.warnings.push(format!("texture {name}: data not found (embedded data: URIs are not supported)"));
            None
        }
    };
    cx.images.insert(ti, out.clone());
    out
}

/// A static Environment collider, its triangles (counter-clockwise fronts) tagged with the metal
/// surface. The surface scene keeps Unity's winding, mirrored in Bevy space: stored reversed.
fn add_collider(cx: &mut Ctx, node: u32, shape: ShapeDef, surface_tris: Vec<[Vec3; 3]>) {
    let surface_tris: Vec<[Vec3; 3]> = surface_tris.into_iter().map(|[a, b, c]| [a, c, b]).collect();
    cx.next_id -= 1;
    cx.def.colliders.push(ColliderDef { node, shape, trigger: false, enabled: true, layer: MAP_LAYER, path_id: cx.next_id });
    cx.report.colliders += 1;
    if !surface_tris.is_empty() {
        let n = surface_tris.len();
        cx.def.surface_meshes.push(SurfaceMeshDef { node, layer: MAP_LAYER, tris: surface_tris, tri_mat: vec![Some(0); n], tri_r: Vec::new(), mats: vec![cx.surface] });
    }
}

fn add_empty_shape(cx: &mut Ctx, n: &gltf::Node, node: u32, world: Mat4, raw: &str) {
    let draw = extras_str(n.extras(), &["empty_display_type", "empty_draw_type"]).unwrap_or_else(|| "SPHERE".into());
    let (s, r, t) = world.to_scale_rotation_translation();
    let s = s.abs();
    let box_shape = |half: Vec3, center: Vec3| ShapeDef::Box { center, half, rot: r };
    let shape = match draw.as_str() {
        "SINGLE_ARROW" => {
            cx.report.warnings.push(format!("{raw}: ray shapes have no Unity collider, skipped"));
            return;
        }
        "CUBE" => box_shape(s, t),
        // WorldBoundaryShape3D: the plane through the origin facing +Y, as a wide slab under it
        "IMAGE" => box_shape(Vec3::new(5000.0, 0.5, 5000.0), t - r * Vec3::Y * 0.5),
        _ => ShapeDef::Sphere { center: t, radius: s.max_element() },
    };
    let tris = shape_tris(&shape);
    add_collider(cx, node, shape, tris);
}

/// Triangles of a box / sphere collider for the surface tagging, facing out.
fn shape_tris(shape: &ShapeDef) -> Vec<[Vec3; 3]> {
    let center = match *shape {
        ShapeDef::Box { center, .. } | ShapeDef::Sphere { center, .. } => center,
        _ => Vec3::ZERO,
    };
    let out = |[a, b, c]: [Vec3; 3]| if (b - a).cross(c - a).dot(a + b + c - 3.0 * center) < 0.0 { [a, c, b] } else { [a, b, c] };
    let tris: Vec<[Vec3; 3]> = match *shape {
        ShapeDef::Box { center, half, rot } => {
            let c = |x: f32, y: f32, z: f32| center + rot * (half * Vec3::new(x, y, z));
            let v: Vec<Vec3> = (0..8).map(|i| c(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 })).collect();
            let f = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
            f.iter().flat_map(|q| [[v[q[0]], v[q[1]], v[q[2]]], [v[q[0]], v[q[2]], v[q[3]]]]).collect()
        }
        ShapeDef::Sphere { center, radius } => {
            let (rings, segs) = (8, 12);
            let p = |i: usize, j: usize| {
                let th = std::f32::consts::PI * i as f32 / rings as f32;
                let ph = std::f32::consts::TAU * j as f32 / segs as f32;
                center + radius * Vec3::new(th.sin() * ph.cos(), th.cos(), th.sin() * ph.sin())
            };
            (0..rings).flat_map(|i| (0..segs).flat_map(move |j| [[p(i, j), p(i + 1, j), p(i + 1, j + 1)], [p(i, j), p(i + 1, j + 1), p(i, j + 1)]])).collect()
        }
        _ => Vec::new(),
    };
    // the sphere's pole triangles are degenerate: dropped
    tris.into_iter().filter(|[a, b, c]| (b - a).cross(c - a).length_squared() > 1e-12).map(out).collect()
}

/// Blender's glTF light export (Lighting Mode: Standard) comes out 1000x brighter than the light
/// looks in Blender at ULTRAKILL's scale: every glTF intensity is scaled by this first.
const GLTF_LIGHT_SCALE: f32 = 1.0 / 1000.0;

/// KHR_lights_punctual -> a Unity light. glTF colors are linear and ULTRAKILL renders in gamma
/// space. Intensities are scaled by GLTF_LIGHT_SCALE. Directional: intensity = lux. Point / spot (candela, inverse square): Unity's falloff
/// tail is intensity * range^2 / (25 d^2), matched to I / d^2: without a glTF range, intensity 1
/// and range 5 sqrt(I); with one, that range and intensity 25 I / range^2.
fn add_light(cx: &mut Ctx, l: &gltf::khr_lights_punctual::Light, node: u32) {
    use gltf::khr_lights_punctual::Kind;
    let [r, g, b] = l.color();
    let color = [linear_to_srgb(r), linear_to_srgb(g), linear_to_srgb(b), 1.0];
    let i = l.intensity() * GLTF_LIGHT_SCALE;
    let (range, intensity) = match l.range() {
        Some(rg) if rg > 0.0 => (rg, 25.0 * i / (rg * rg)),
        _ => (5.0 * i.max(0.0).sqrt(), 1.0),
    };
    let (kind, intensity, range, spot_angle) = match l.kind() {
        Kind::Directional => (1u8, i, 10.0, 30.0),
        Kind::Point => (2, intensity, range, 30.0),
        Kind::Spot { outer_cone_angle, .. } => (0, intensity, range, (2.0 * outer_cone_angle).to_degrees()),
    };
    cx.def.lights.push(LightDef { node, kind, color, intensity, range, spot_angle, enabled: true, render_mode: 0, culling_mask: u32::MAX });
    cx.report.lights += 1;
}

/// Incremental 3D convex hull, outward-facing triangles. None when the points are (nearly) flat.
pub fn convex_hull(tris: &[[Vec3; 3]]) -> Option<Vec<[Vec3; 3]>> {
    let mut seen = HashMap::new();
    let pts: Vec<Vec3> = tris.iter().flatten().filter(|p| seen.insert(((p.x * 1e4) as i64, (p.y * 1e4) as i64, (p.z * 1e4) as i64), ()).is_none()).copied().collect();
    if pts.len() < 4 {
        return None;
    }
    let (lo, hi) = pts.iter().fold((Vec3::MAX, Vec3::MIN), |(l, h), p| (l.min(*p), h.max(*p)));
    // work around the centre: far from the origin, f32 spacing exceeds the tolerance and faces fold
    let centre = (lo + hi) * 0.5;
    let pts: Vec<Vec3> = pts.iter().map(|p| *p - centre).collect();
    let eps = (hi - lo).max_element() * 1e-4;
    let far = |f: &dyn Fn(Vec3) -> f32| (0..pts.len()).max_by(|&a, &b| f(pts[a]).total_cmp(&f(pts[b]))).unwrap();
    let a = 0;
    let b = far(&|p| p.distance(pts[a]));
    let ab = (pts[b] - pts[a]).normalize_or_zero();
    let c = far(&|p| (p - pts[a]).cross(ab).length());
    let nrm = (pts[b] - pts[a]).cross(pts[c] - pts[a]).normalize_or_zero();
    let d = far(&|p| (p - pts[a]).dot(nrm).abs());
    if (pts[d] - pts[a]).dot(nrm).abs() <= eps || nrm == Vec3::ZERO {
        return None;
    }
    let inside = (pts[a] + pts[b] + pts[c] + pts[d]) / 4.0;
    let orient = |f: [usize; 3]| {
        let n = (pts[f[1]] - pts[f[0]]).cross(pts[f[2]] - pts[f[0]]);
        if n.dot(pts[f[0]] - inside) < 0.0 {
            [f[0], f[2], f[1]]
        } else {
            f
        }
    };
    let mut faces: Vec<[usize; 3]> = [[a, b, c], [a, b, d], [a, c, d], [b, c, d]].into_iter().map(orient).collect();
    let above = |f: &[usize; 3], p: Vec3| {
        let n = (pts[f[1]] - pts[f[0]]).cross(pts[f[2]] - pts[f[0]]).normalize_or_zero();
        n.dot(p - pts[f[0]])
    };
    // quickhull order: always add the point farthest outside, and drop points once they're inside
    let mut outside: Vec<usize> = (0..pts.len()).filter(|p| ![a, b, c, d].contains(p)).collect();
    loop {
        let mut best: Option<(f32, usize)> = None;
        outside.retain(|&p| {
            let h = faces.iter().map(|f| above(f, pts[p])).fold(f32::MIN, f32::max);
            if h > eps && best.is_none_or(|(bh, _)| h > bh) {
                best = Some((h, p));
            }
            h > eps
        });
        let Some((_, p)) = best else { break };
        let (visible, rest): (Vec<[usize; 3]>, Vec<[usize; 3]>) = faces.iter().partition(|f| above(f, pts[p]) > eps);
        // ordered set: the face order must not change between loads
        let edges: std::collections::BTreeSet<(usize, usize)> = visible.iter().flat_map(|f| [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])]).collect();
        faces = rest;
        for &(u, v) in &edges {
            if !edges.contains(&(v, u)) {
                faces.push([u, v, p]);
            }
        }
    }
    Some(faces.iter().map(|f| f.map(|i| pts[i] + centre)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suffixes() {
        let mut w = Vec::new();
        let k = classify("Plane -col", &mut w);
        assert_eq!((k.name.as_str(), k.col, k.only), ("Plane", Col::Tri, false));
        let k = classify("Wall_ConvColOnly.001", &mut w);
        assert_eq!((k.name.as_str(), k.col, k.only), ("Wall", Col::Convex, true));
        let k = classify("Box$colonly", &mut w);
        assert_eq!((k.name.as_str(), k.col, k.only), ("Box", Col::Tri, true));
        assert!(classify("Junk-noimp", &mut w).noimp);
        assert_eq!(classify("Sphere -conv", &mut w).col, Col::None);
    }

    #[test]
    fn cube_hull() {
        let tris = shape_tris(&ShapeDef::Box { center: Vec3::ZERO, half: Vec3::ONE, rot: Quat::IDENTITY });
        let mut pts = tris.clone();
        pts.push([Vec3::ZERO, Vec3::splat(0.5), Vec3::splat(-0.5)]);
        let h = convex_hull(&pts).unwrap();
        assert_eq!(h.len(), 12);
        for t in &h {
            let n = (t[1] - t[0]).cross(t[2] - t[0]);
            assert!(n.dot(t[0]) > 0.0);
        }
    }
}
