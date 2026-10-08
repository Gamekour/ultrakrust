//! `-room` collections and `-door` meshes: ULTRAKILL's room loading, done the way the campaign
//! does it. Each door gets a `Door` (Normal type) and a `DoorController` area, copied from 0-1's
//! `Door (Large) With Controllers (1)` and wrapped in their own node at the map root, as in 0-1.
//! Door positions and room lists are derived from the map:
//! - a door joins the rooms on its two sides (`Door.activatedRooms`, switched on when it opens);
//! - entering its area unloads the rooms beyond those two (`Door.deactivatedRooms`, by
//!   `DoorController` -> `Door.Optimize`);
//! - every room a door loads starts unloaded, except the one the player starts in.
//!
//! `door_rooms level0-1` shows the campaign's pattern this reproduces: the door between rooms
//! 4 and 5 activates 4 and 5 and deactivates 3 and 6.
use super::{set_value, MapReport};
use crate::scenedef::{ColliderDef, NodeDef, SceneDef, ScriptDef, ShapeDef};
use crate::serialized::Value;
use bevy_math::{Quat, Vec3};
use std::collections::BTreeSet;

/// 0-1's door the copies take their fields from.
const TEMPLATE: &str = "Door (Large) With Controllers (1)";
/// DoorController areas sit on layer 16, as in 0-1.
const CONTROLLER_LAYER: u8 = 16;
/// 0-1's door: the DoorController area reaches 4 m from the door's centre plane on each side.
const TRIGGER_DEPTH: f32 = 4.0;
/// 0-1's Door (Large) speed (m/s).
const DOOR_SPEED: f32 = 25.0;
/// How far from the door's centre plane its two sides are looked up.
const SIDE_PROBE: f32 = 2.0;

/// `Door` and `DoorController` data from 0-1, read before the base scene is pruned.
pub struct Templates {
    door: Value,
    controller: Value,
}

pub fn templates(def: &SceneDef) -> Option<Templates> {
    let script = |class: &str| {
        def.scripts.iter().find(|s| s.class == class && def.path(s.node).starts_with(TEMPLATE)).map(|s| s.data.clone())
    };
    Some(Templates { door: script("Door")?, controller: script("DoorController")? })
}

/// A `-door` mesh and its custom properties.
#[derive(Debug, Default)]
pub struct DoorProps {
    pub node: u32,
    /// how far it slides up when open, metres (default: its height)
    pub open: Option<f32>,
    pub speed: Option<f32>,
    pub start_open: bool,
    pub locked: bool,
    /// depth of the approach area on each side, from the door's centre plane
    pub trigger_size: Option<f32>,
    /// the two rooms it joins, by name (default: the rooms found on each side)
    pub rooms: Option<Vec<String>>,
}

impl DoorProps {
    pub fn read(node: u32, extras: &Option<Box<serde_json::value::RawValue>>) -> Self {
        let v: serde_json::Value = extras.as_ref().and_then(|e| serde_json::from_str(e.get()).ok()).unwrap_or_default();
        let num = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
        let flag = |k: &str| v.get(k).is_some_and(|x| x.as_bool().unwrap_or(x.as_f64().is_some_and(|f| f != 0.0)));
        DoorProps {
            node,
            open: num("open"),
            speed: num("speed"),
            start_open: flag("start_open"),
            locked: flag("locked"),
            trigger_size: num("trigger_size"),
            rooms: v.get("rooms").and_then(|x| x.as_str()).map(|s| s.split(',').map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).collect()),
        }
    }
}

#[derive(Clone, Copy)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::MAX), max: Vec3::splat(f32::MIN) };
    fn add(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }
    fn valid(&self) -> bool {
        self.min.cmple(self.max).all()
    }
    fn contains(&self, p: Vec3, pad: f32) -> bool {
        self.valid() && (p - self.min).cmpge(Vec3::splat(-pad)).all() && (self.max - p).cmpge(Vec3::splat(-pad)).all()
    }
    fn volume(&self) -> f32 {
        let s = self.max - self.min;
        s.x * s.y * s.z
    }
}

fn shape_points(s: &ShapeDef) -> Vec<Vec3> {
    match s {
        ShapeDef::Box { center, half, rot } => (0..8)
            .map(|i| *center + *rot * (*half * Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 })))
            .collect(),
        ShapeDef::Sphere { center, radius } => vec![*center - Vec3::splat(*radius), *center + Vec3::splat(*radius)],
        ShapeDef::Capsule { a, b, radius } => vec![a.min(*b) - Vec3::splat(*radius), a.max(*b) + Vec3::splat(*radius)],
        ShapeDef::Mesh(t) => t.iter().flatten().copied().collect(),
    }
}

/// A path id below every id the scene uses.
fn fresh_ids(def: &SceneDef) -> i64 {
    let used = [def.obj_to_node.keys(), def.comp_to_script.keys(), def.comp_to_collider.keys(), def.comp_to_particle.keys()];
    let lowest = used.into_iter().flatten().copied().chain(def.scripts.iter().map(|s| s.path_id)).chain(def.colliders.iter().map(|c| c.path_id)).min().unwrap_or(0);
    lowest.min(0) - 1
}

fn pptr(id: i64) -> Value {
    Value::Struct(vec![("m_FileID".into(), Value::Int(0)), ("m_PathID".into(), Value::Int(id))])
}

fn uvec3(v: Vec3) -> Value {
    // Unity space: z flipped
    Value::Struct(vec![("x".into(), Value::Float(v.x as f64)), ("y".into(), Value::Float(v.y as f64)), ("z".into(), Value::Float(-v.z as f64))])
}

/// The nearest `-room` ancestor of `n` (itself included).
fn room_of(def: &SceneDef, is_room: &[bool], mut n: u32) -> Option<u32> {
    loop {
        if is_room[n as usize] {
            return Some(n);
        }
        n = def.nodes[n as usize].parent?;
    }
}

/// Wires the map's rooms and doors. `root` is the glTF scene's root node, `start` the player's
/// start position.
pub fn setup(def: &mut SceneDef, root: u32, rooms: &[u32], doors: &[DoorProps], tpl: Option<&Templates>, start: Vec3, report: &mut MapReport) {
    if doors.is_empty() {
        if !rooms.is_empty() {
            report.warnings.push("-room without any -door: rooms only load through doors, so they stay loaded".into());
        }
        return;
    }
    let Some(tpl) = tpl else {
        report.warnings.push(format!("0-1 has no {TEMPLATE} to copy: -door skipped"));
        return;
    };
    let mut is_room = vec![false; def.nodes.len()];
    rooms.iter().for_each(|&r| is_room[r as usize] = true);
    let own_room: Vec<Option<u32>> = doors.iter().map(|d| def.nodes[d.node as usize].parent.and_then(|p| room_of(def, &is_room, p))).collect();
    let mut next = fresh_ids(def);
    let mut id = || {
        next -= 1;
        next + 1
    };
    let mut room_id = std::collections::HashMap::new();
    for &r in rooms {
        let i = id();
        def.obj_to_node.insert(i, r);
        room_id.insert(r, i);
    }
    // each door: its own wrapper at the map root (rooms unload; doors stay, as in 0-1)
    struct Placed {
        node: u32,
        center: Vec3,
        normal: Vec3,
        sides: Vec<u32>,
        named: Option<Vec<String>>,
    }
    let mut placed = Vec::new();
    for (di, d) in doors.iter().enumerate() {
        let n = d.node;
        let world = def.nodes[n as usize].world0;
        let inv = world.inverse();
        let mut local = Aabb::EMPTY;
        let mut wbox = Aabb::EMPTY;
        for c in def.colliders.iter().filter(|c| c.node == n) {
            for p in shape_points(&c.shape) {
                local.add(inv.transform_point3(p));
                wbox.add(p);
            }
        }
        if !local.valid() {
            report.warnings.push(format!("{}: -door has no mesh; skipped", def.nodes[n as usize].name));
            continue;
        }
        let (scale, rot, _) = world.to_scale_rotation_translation();
        let size = (local.max - local.min) * scale.abs();
        // the door's thin horizontal axis is the way through it
        let thick_x = size.x < size.z;
        let axis = if thick_x { Vec3::X } else { Vec3::Z };
        let normal = (rot * axis).normalize();
        let center = world.transform_point3((local.min + local.max) * 0.5);
        let depth = d.trigger_size.unwrap_or(TRIGGER_DEPTH);
        let height = wbox.max.y - wbox.min.y;
        // re-parent: map root -> wrapper -> (door, controller)
        let old = def.nodes[n as usize].parent;
        if let Some(p) = old {
            def.nodes[p as usize].children.retain(|&c| c != n);
        }
        let (ws, wr, wt) = (def.nodes[root as usize].world0.inverse() * world).to_scale_rotation_translation();
        let base = def.nodes[n as usize].name.clone();
        let wrap = def.nodes.len() as u32;
        let node = |name: String, parent: u32, layer: u8| NodeDef {
            name,
            parent: Some(parent),
            children: Vec::new(),
            active_self: true,
            layer,
            tag: 0,
            local_pos: Vec3::ZERO,
            local_rot: Quat::IDENTITY,
            local_scale: Vec3::ONE,
            world0: world,
            rect: None,
        };
        def.nodes.push(NodeDef { local_pos: wt, local_rot: wr, local_scale: ws, ..node(format!("{base} With Controllers"), root, super::MAP_LAYER) });
        def.nodes[root as usize].children.push(wrap);
        let ctrl = def.nodes.len() as u32;
        def.nodes.push(node("DoorController".into(), wrap, CONTROLLER_LAYER));
        def.nodes[wrap as usize].children = vec![n, ctrl];
        let dn = &mut def.nodes[n as usize];
        dn.parent = Some(wrap);
        (dn.local_pos, dn.local_rot, dn.local_scale) = (Vec3::ZERO, Quat::IDENTITY, Vec3::ONE);
        for nd in [n, wrap, ctrl] {
            let i = id();
            def.obj_to_node.insert(i, nd);
        }
        // the DoorController area: the door's width and height, `depth` each side
        let mut half = (local.max - local.min) * 0.5 * scale.abs();
        if thick_x {
            half.x = depth;
        } else {
            half.z = depth;
        }
        let cid = id();
        def.comp_to_collider.insert(cid, def.colliders.len() as u32);
        def.colliders.push(ColliderDef { node: ctrl, shape: ShapeDef::Box { center, half, rot }, trigger: true, enabled: true, layer: CONTROLLER_LAYER, path_id: cid });
        // Door: slides up by `open` (its height), in the wrapper's space
        let up = inv.transform_vector3(Vec3::Y * d.open.unwrap_or(height));
        let mut data = tpl.door.clone();
        set_value(&mut data, "openPos", uvec3(up));
        set_value(&mut data, "speed", Value::Float(d.speed.unwrap_or(DOOR_SPEED) as f64));
        set_value(&mut data, "startOpen", Value::UInt(d.start_open as u64));
        set_value(&mut data, "locked", Value::UInt(d.locked as u64));
        set_value(&mut data, "noPass", pptr(0));
        set_value(&mut data, "openLight", pptr(0));
        let sid = id();
        def.comp_to_script.insert(sid, def.scripts.len() as u32);
        def.scripts.push(ScriptDef { node: n, class: "Door".into(), enabled: true, path_id: sid, data, file: None });
        let sid = id();
        def.comp_to_script.insert(sid, def.scripts.len() as u32);
        def.scripts.push(ScriptDef { node: ctrl, class: "DoorController".into(), enabled: true, path_id: sid, data: tpl.controller.clone(), file: None });
        is_room.resize(def.nodes.len(), false);
        placed.push(Placed { node: n, center, normal, sides: own_room[di].into_iter().collect(), named: d.rooms.clone() });
    }
    // room bounds: the geometry under each room (doors have left)
    let mut bounds = vec![Aabb::EMPTY; rooms.len()];
    let ri = |r: u32| rooms.iter().position(|&x| x == r);
    for c in &def.colliders {
        if let Some(i) = room_of(def, &is_room, c.node).and_then(ri) {
            shape_points(&c.shape).into_iter().for_each(|p| bounds[i].add(p));
        }
    }
    for r in &def.renderers {
        if let Some(i) = room_of(def, &is_room, r.node).and_then(ri) {
            r.batch.positions.iter().for_each(|&p| bounds[i].add(Vec3::from(p)));
        }
    }
    let room_at = |p: Vec3| {
        (0..rooms.len()).filter(|&i| bounds[i].contains(p, 0.5)).min_by(|&a, &b| bounds[a].volume().total_cmp(&bounds[b].volume())).map(|i| rooms[i])
    };
    for pd in &mut placed {
        let name = def.nodes[pd.node as usize].name.clone();
        if let Some(names) = pd.named.take() {
            pd.sides.clear();
            for nm in names {
                match rooms.iter().find(|&&r| def.nodes[r as usize].name.eq_ignore_ascii_case(&nm)) {
                    Some(&r) => pd.sides.push(r),
                    None => report.warnings.push(format!("{name}: rooms names \"{nm}\", which is no -room")),
                }
            }
        } else {
            let mid = pd.center;
            for s in [1.0, -1.0] {
                if let Some(r) = room_at(mid + pd.normal * SIDE_PROBE * s) {
                    pd.sides.push(r);
                }
            }
        }
        pd.sides.sort_unstable();
        pd.sides.dedup();
        if pd.sides.len() < 2 {
            report.warnings.push(format!("{name}: found {} room(s) beside it, not 2 (give it a `rooms` property)", pd.sides.len()));
        }
    }
    // neighbours through doors
    let mut adj: std::collections::BTreeMap<u32, BTreeSet<u32>> = Default::default();
    for p in &placed {
        for &a in &p.sides {
            for &b in &p.sides {
                if a != b {
                    adj.entry(a).or_default().insert(b);
                }
            }
        }
    }
    let list = |v: &[u32]| Value::Array(v.iter().map(|r| pptr(room_id[r])).collect());
    for p in &placed {
        let far: Vec<u32> = p.sides.iter().flat_map(|s| adj.get(s).into_iter().flatten().copied()).filter(|r| !p.sides.contains(r)).collect::<BTreeSet<_>>().into_iter().collect();
        let sc = def.scripts.iter().rposition(|s| s.node == p.node && s.class == "Door").unwrap();
        set_value(&mut def.scripts[sc].data, "activatedRooms", list(&p.sides));
        set_value(&mut def.scripts[sc].data, "deactivatedRooms", list(&far));
    }
    report.rooms = rooms.len();
    report.doors = placed.len();
    // rooms a door loads start unloaded, except where the player starts
    let start_rooms: Vec<u32> = (0..rooms.len()).filter(|&i| bounds[i].contains(start, 0.5)).map(|i| rooms[i]).collect();
    if start_rooms.is_empty() {
        report.warnings.push("the player start is in no -room: every room starts loaded".into());
        return;
    }
    for &r in adj.keys() {
        if !start_rooms.contains(&r) {
            def.nodes[r as usize].active_self = false;
        }
    }
}
