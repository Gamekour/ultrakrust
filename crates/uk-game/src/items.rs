//! Skulls and altars: ItemIdentifier, ItemPlaceZone, Skull, and Punch's item half (AltHit,
//! ForceHold, PlaceHeldObject). A picked-up item is reparented to Punch.holder at runtime
//! (`State::reparent`), so its activity, world transform and GetComponentInChildren lookups
//! follow the new parent.

use crate::game::Game;
use crate::particles::FLIP;
use crate::scripts::{nodes, parse_uevent, scripts, Script, UEvent};
use bevy_math::{Affine3A, EulerRot, Mat4, Quat, Vec3};
use uk_assets::scenedef::{SceneDef, ShapeDef};
use uk_core::collide::{BoxCollider, Capsule, Shape, Triangle};

/// ItemIdentifier
#[derive(Clone, Debug)]
pub struct Item {
    /// ItemType: None 0, SkullBlue 1, SkullRed 2, SkullGreen 3, Readable 4, Torch 5, Soap 6, ...
    pub item_type: i64,
    pub infinite_source: bool,
    pub reverse_transform: bool,
    /// putDownPosition / putDownRotation (Euler degrees) / putDownScale, Unity values
    pub put_down_pos: Vec3,
    pub put_down_rot: Vec3,
    pub put_down_scale: Vec3,
    pub no_holding_animation: bool,
    pub on_pick_up: UEvent,
    pub on_put_down: UEvent,
    pub picked_up: bool,
    pub been_picked_up: bool,
    /// the ItemPlaceZone that accepted it
    pub ipz: Option<u32>,
}

/// ItemPlaceZone
#[derive(Clone, Debug)]
pub struct PlaceZone {
    pub accepted: i64,
    pub activate_on_success: Vec<u32>,
    pub deactivate_on_success: Vec<u32>,
    pub activate_on_failure: Vec<u32>,
    /// Door scripts
    pub doors: Vec<u32>,
    pub reverse_doors: Vec<u32>,
    /// ArenaStatus scripts
    pub arena_statuses: Vec<u32>,
    pub reverse_arena_statuses: Vec<u32>,
    pub hide_effects: bool,
    /// the InstantiateObjects' nodes
    pub altar_elements: Vec<u32>,
    /// scene ParticleSystem index
    pub element_change_effect: Option<u32>,
    pub accepted_placed: bool,
    /// GetComponent<Collider>()
    pub col: Option<u32>,
}

/// Skull: its Light flares on PunchWith and goes out on an accepting altar.
#[derive(Clone, Debug, Default)]
pub struct Skull {
    /// `SceneDef::lights` index
    pub light: Option<u32>,
    pub orig_range: f32,
    pub lit_time: f32,
    /// the ModifyMaterial's node (GetComponentInChildren)
    pub modmat: Option<u32>,
}

pub fn parse(def: &SceneDef, idx: usize) -> Script {
    let s = &def.scripts[idx];
    let v = &s.data;
    let b = |k: &str| v.get(k).bool();
    let raw = |k: &str| Vec3::from(v.get(k).vec3());
    match s.class.as_str() {
        "ItemIdentifier" => Script::Item(Box::new(Item {
            item_type: v.get("itemType").i64(),
            infinite_source: b("infiniteSource"),
            reverse_transform: b("reverseTransformSettings"),
            put_down_pos: raw("putDownPosition"),
            put_down_rot: raw("putDownRotation"),
            put_down_scale: raw("putDownScale"),
            no_holding_animation: b("noHoldingAnimation"),
            on_pick_up: parse_uevent(def, v.get("onPickUp")),
            on_put_down: parse_uevent(def, v.get("onPutDown")),
            picked_up: b("pickedUp"),
            been_picked_up: b("beenPickedUp"),
            ipz: None,
        })),
        "ItemPlaceZone" => Script::PlaceZone(Box::new(PlaceZone {
            accepted: v.get("acceptedItemType").i64(),
            activate_on_success: nodes(def, v.get("activateOnSuccess")),
            deactivate_on_success: nodes(def, v.get("deactivateOnSuccess")),
            activate_on_failure: nodes(def, v.get("activateOnFailure")),
            doors: scripts(def, v.get("doors")),
            reverse_doors: scripts(def, v.get("reverseDoors")),
            arena_statuses: scripts(def, v.get("arenaStatuses")),
            reverse_arena_statuses: scripts(def, v.get("reverseArenaStatuses")),
            hide_effects: b("hideEffects"),
            altar_elements: scripts(def, v.get("altarElements")).into_iter().map(|i| def.scripts[i as usize].node).collect(),
            element_change_effect: {
                let (f, id) = v.get("elementChangeEffect").pptr();
                if f == 0 { def.comp_to_particle.get(&id).copied() } else { None }
            },
            accepted_placed: false,
            col: None,
        })),
        _ => Script::Skull(Skull::default()),
    }
}

/// Unity local TRS -> matrix (Unity space).
fn trs(pos: Vec3, euler: Vec3, scale: Vec3) -> Mat4 {
    // Quaternion.Euler: z, then x, then y
    let r = Quat::from_euler(EulerRot::YXZ, euler.y.to_radians(), euler.x.to_radians(), euler.z.to_radians());
    Mat4::from_scale_rotation_translation(scale, r, pos)
}

impl Game {
    // ---------------------------------------------------------------- runtime parenting

    /// transform.parent, with runtime SetParent.
    pub fn eff_parent(&self, n: u32) -> Option<u32> {
        match self.s.reparent.iter().find(|r| r.0 == n) {
            Some(r) => Some(r.1),
            None => self.def.nodes[n as usize].parent,
        }
    }

    /// The node's children, with runtime SetParent (moved-in children last, like SetParent).
    pub fn eff_children(&self, n: u32) -> Vec<u32> {
        let mut v: Vec<u32> = self.def.nodes[n as usize].children.iter().copied().filter(|&c| self.eff_parent(c) == Some(n)).collect();
        v.extend(self.s.reparent.iter().filter(|r| r.1 == n && self.def.nodes[r.0 as usize].parent != Some(n)).map(|r| r.0));
        v
    }

    /// Transform.SetParent(parent) with the given Unity-space local transform.
    fn set_parent(&mut self, n: u32, parent: u32, local: Mat4) {
        self.s.reparent.retain(|r| r.0 != n);
        self.s.reparent.push((n, parent, local));
        self.refresh_node(n);
    }

    /// Nodes below `n` (itself included), runtime parenting followed, depth first.
    fn eff_subtree(&self, n: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack = vec![n];
        while let Some(m) = stack.pop() {
            out.push(m);
            stack.extend(self.eff_children(m).into_iter().rev());
        }
        out
    }

    /// The held item's node (Punch.heldItem).
    pub fn held_item(&self) -> Option<u32> {
        self.s.held.map(|sc| self.def.scripts[sc as usize].node)
    }

    /// The node is the held item or below it.
    pub fn under_held(&self, mut n: u32) -> bool {
        let Some(h) = self.held_item() else { return false };
        loop {
            if n == h {
                return true;
            }
            match self.eff_parent(n) {
                Some(p) => n = p,
                None => return false,
            }
        }
    }

    /// Mover delta of an ItemIdentifier node: its runtime parent's current world times its
    /// local transform, against its load pose; before any SetParent it rides its parent's mover.
    pub(crate) fn item_mover_delta(&self, node: u32, world0: Mat4) -> Affine3A {
        match self.s.reparent.iter().find(|r| r.0 == node) {
            Some(&(_, p, local)) => {
                let w = self.node_world_now(p) * local;
                Affine3A::from_mat4(FLIP * w * (FLIP * world0 * FLIP).inverse() * FLIP)
            }
            None => match self.def.nodes[node as usize].parent.and_then(|p| self.node_mover[p as usize]) {
                Some(m) => self.mover_delta(m),
                None => Affine3A::IDENTITY,
            },
        }
    }

    /// A light's current world matrix (Bevy space). Lights on the held item sit in the
    /// viewmodel's load-time space; they are carried to the camera's current pose.
    pub fn light_world_bevy(&self, n: u32) -> Mat4 {
        let w = FLIP * self.node_world_now(n) * FLIP;
        match (self.under_held(n), self.sway) {
            (true, Some(sw)) => self.view_world * self.def.nodes[sw.hud_cam as usize].world0.inverse() * w,
            _ => w,
        }
    }

    /// MaterialPropertyBlock value on a node's renderers (`subtree`: every renderer below it).
    fn set_mat_prop(&mut self, node: u32, name: &'static str, v: [f32; 4], subtree: bool) {
        match self.s.mat_props.iter_mut().find(|p| p.0 == node && p.1 == name) {
            Some(p) => (p.2, p.3) = (v, subtree),
            None => self.s.mat_props.push((node, name, v, subtree)),
        }
    }

    /// Property block overrides that apply to a renderer on `node`.
    pub fn mat_props_of(&self, node: u32) -> Vec<(&'static str, [f32; 4])> {
        self.s
            .mat_props
            .iter()
            .filter(|p| p.0 == node || (p.3 && self.def.is_descendant(node, p.0)))
            .map(|p| (p.1, p.2))
            .collect()
    }

    // ---------------------------------------------------------------- SendMessage

    fn item_message(&mut self, node: u32, msg: &str) {
        let scs: Vec<u32> = self.scripts_on_node(node).to_vec();
        for sc in scs {
            match (&self.s.scripts[sc as usize], msg) {
                // ItemIdentifier.PickUp / PutDown: onPickUp / onPutDown
                (Script::Item(it), "PickUp") => {
                    let ev = it.on_pick_up.clone();
                    self.run_uevent(&ev, false);
                }
                (Script::Item(it), "PutDown") => {
                    let ev = it.on_put_down.clone();
                    self.run_uevent(&ev, false);
                }
                (Script::Skull(_), "OnCorrectUse") => self.skull_correct_use(sc, true),
                (Script::Skull(_), "OffCorrectUse") => self.skull_correct_use(sc, false),
                (Script::Skull(_), "PunchWith") => self.skull_punch_with(sc),
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------- Skull

    /// Skull.Awake
    pub(crate) fn skull_awake(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let light = self.def.lights.iter().position(|l| l.node == node).map(|l| l as u32);
        let orig_range = light.map_or(0.0, |l| self.s.lights[l as usize].1);
        // GetComponentInChildren<ModifyMaterial>(): active GameObjects, depth first
        let modmat = self.eff_subtree(node).into_iter().find(|&n| self.s.active[n as usize] && self.def.scripts_on(n).any(|(_, s)| s.class == "ModifyMaterial"));
        if let Script::Skull(s) = &mut self.s.scripts[sc as usize] {
            *s = Skull { light, orig_range, lit_time: 0.0, modmat };
        }
    }

    /// Skull.Update
    pub(crate) fn skull_update(&mut self, sc: u32, dt: f32) {
        let Script::Skull(s) = &mut self.s.scripts[sc as usize] else { return };
        let Some(l) = s.light else { return };
        let r = &mut self.s.lights[l as usize].1;
        if s.lit_time > 0.0 {
            s.lit_time = (s.lit_time - dt).max(0.0);
        } else if *r > s.orig_range {
            *r = (*r - dt * 5.0).max(s.orig_range);
        }
    }

    /// Skull.PunchWith (the AudioSource is not ported)
    fn skull_punch_with(&mut self, sc: u32) {
        let Script::Skull(s) = &mut self.s.scripts[sc as usize] else { return };
        let Some(l) = s.light else { return };
        let r = &mut self.s.lights[l as usize].1;
        if *r == s.orig_range {
            s.lit_time = 1.0;
            *r = s.orig_range * 2.5;
        }
    }

    /// Skull.OnCorrectUse / OffCorrectUse: the light goes out and the material's emission comes up.
    fn skull_correct_use(&mut self, sc: u32, on: bool) {
        let Script::Skull(s) = &self.s.scripts[sc as usize] else { return };
        let (light, modmat) = (s.light, s.modmat);
        if let Some(l) = light {
            self.s.lights[l as usize].0 = !on;
        }
        // ModifyMaterial.ChangeEmissionIntensity
        if let Some(m) = modmat {
            self.set_mat_prop(m, "_EmissiveIntensity", [if on { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0], false);
        }
    }

    // ---------------------------------------------------------------- ItemPlaceZone

    /// ItemPlaceZone.Start (Awake's door bounds only feed the bounds-error check, not ported)
    pub(crate) fn place_zone_start(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let col = self.def.colliders.iter().position(|c| c.node == node).map(|c| c as u32);
        let Script::PlaceZone(z) = &mut self.s.scripts[sc as usize] else { return };
        z.col = col;
        let (doors, rev) = (z.doors.clone(), z.reverse_doors.clone());
        self.color_doors(sc, &doors);
        self.color_doors(sc, &rev);
        self.check_item(sc, true);
    }

    /// ItemPlaceZone.ColorDoors: the skull's colour on the doors' lights, then Door.AltarControlled.
    fn color_doors(&mut self, sc: u32, doors: &[u32]) {
        let Script::PlaceZone(z) = &self.s.scripts[sc as usize] else { return };
        let color = match z.accepted {
            1 => Some([0.0, 0.75, 1.0, 1.0]),
            2 => Some([1.0, 0.2, 0.2, 1.0]),
            3 => Some([0.25, 1.0, 0.25, 1.0]),
            _ => None,
        };
        for &d in doors {
            let dnode = self.def.scripts[d as usize].node;
            let Script::Door(door) = &self.s.scripts[d as usize] else { continue };
            let no_pass_on = door.no_pass.is_some_and(|n| self.s.active_self[n as usize]);
            // Door.ChangeColor(defaultLightsColor): `_EmissiveColor` on its MeshRenderers. Without a
            // skull type the default is the door's own material colour, which changes nothing.
            if let (Some(c), false) = (color, no_pass_on) {
                self.set_mat_prop(dnode, "_EmissiveColor", c, true);
            }
            self.door_altar_controlled(d);
        }
    }

    /// Door.AltarControlled: its DoorControllers go away and its own solid collider closes it.
    fn door_altar_controlled(&mut self, d: u32) {
        let dnode = self.def.scripts[d as usize].node;
        let Script::Door(door) = &self.s.scripts[d as usize] else { return };
        let docons = door.docons.clone();
        for dc in docons {
            let keep = self.def.scripts[dc as usize].data.get("dontDeactivateOnAltarControl").bool();
            if !keep {
                self.set_active(self.def.scripts[dc as usize].node, false);
            }
        }
        let col = self.def.colliders.iter().position(|c| c.node == dnode).map(|c| c as u32);
        if let Some(c) = col.filter(|&c| !self.def.colliders[c as usize].trigger) {
            let Script::Door(door) = &mut self.s.scripts[d as usize] else { return };
            door.doconless_col = Some(c);
            if !door.start_open {
                self.s.collider_enabled[c as usize] = true;
            }
        }
    }

    /// ItemPlaceZone.CheckItem
    pub(crate) fn check_item(&mut self, sc: u32, prelim: bool) {
        let node = self.def.scripts[sc as usize].node;
        // GetComponentInChildren<ItemIdentifier>(): active GameObjects only
        let item = self.eff_subtree(node).into_iter().filter(|&n| self.s.active[n as usize]).find_map(|n| {
            self.scripts_on_node(n).iter().copied().find(|&s| matches!(self.s.scripts[s as usize], Script::Item(_)))
        });
        let Script::PlaceZone(z) = &self.s.scripts[sc as usize] else { return };
        let z = z.clone();
        if let Some(it) = item {
            let Script::Item(i) = &self.s.scripts[it as usize] else { return };
            if i.item_type == z.accepted {
                if !prelim && !z.hide_effects && !z.accepted_placed {
                    if let Some(ps) = z.element_change_effect {
                        self.scene_particle_call(ps, "Play");
                    }
                }
                if let Script::PlaceZone(zm) = &mut self.s.scripts[sc as usize] {
                    zm.accepted_placed = true;
                }
                if let Script::Item(i) = &mut self.s.scripts[it as usize] {
                    i.ipz = Some(sc);
                }
                if !z.hide_effects {
                    self.item_message(self.def.scripts[it as usize].node, "OnCorrectUse");
                }
                for &n in &z.activate_on_success {
                    self.set_active(n, true);
                }
                for &n in &z.deactivate_on_success {
                    self.set_active(n, false);
                }
                for &d in &z.doors {
                    self.door_open(d, false, true);
                }
                for &d in &z.reverse_doors {
                    self.door_close(d, false);
                }
                if !z.hide_effects {
                    // InstantiateObject.Instantiate (the raised altar pieces) is not ported
                    for &n in &z.altar_elements {
                        self.set_active(n, true);
                    }
                }
                if !prelim {
                    self.add_arena_status(&z.arena_statuses, 1);
                    self.add_arena_status(&z.reverse_arena_statuses, -1);
                }
            } else {
                for &n in &z.activate_on_failure {
                    self.set_active(n, true);
                }
            }
            if let Some(c) = z.col {
                self.s.collider_enabled[c as usize] = false;
            }
            return;
        }
        if let Some(c) = z.col {
            self.s.collider_enabled[c as usize] = true;
        }
        if !prelim && !z.accepted_placed {
            return;
        }
        for &n in &z.activate_on_success {
            self.set_active(n, false);
        }
        for &n in &z.activate_on_failure {
            self.set_active(n, false);
        }
        for &n in &z.deactivate_on_success {
            self.set_active(n, true);
        }
        for &d in &z.doors {
            let local = self.s.local_pos[self.def.scripts[d as usize].node as usize];
            if matches!(&self.s.scripts[d as usize], Script::Door(dd) if dd.door_type != 0 || local != dd.closed_pos) {
                self.door_close(d, false);
            }
        }
        for &d in &z.reverse_doors {
            let local = self.s.local_pos[self.def.scripts[d as usize].node as usize];
            if matches!(&self.s.scripts[d as usize], Script::Door(dd) if dd.door_type != 0 || local != dd.open_pos) {
                self.door_open(d, false, true);
            }
        }
        if !prelim {
            if let Script::PlaceZone(zm) = &mut self.s.scripts[sc as usize] {
                zm.accepted_placed = false;
            }
            if !z.hide_effects {
                if let Some(ps) = z.element_change_effect {
                    self.scene_particle_call(ps, "Play");
                }
            }
            self.add_arena_status(&z.arena_statuses, -1);
            self.add_arena_status(&z.reverse_arena_statuses, 1);
        }
        for &n in &z.altar_elements {
            self.set_active(n, false);
        }
    }

    fn add_arena_status(&mut self, list: &[u32], by: i64) {
        for &a in list {
            if let Script::ArenaStatus { status } = &mut self.s.scripts[a as usize] {
                *status += by;
            }
        }
    }

    // ---------------------------------------------------------------- Punch

    /// Layer-22 colliders the punch's environment query can find: enabled, active, in their
    /// current pose.
    fn item_shapes(&self) -> Vec<(u32, Vec<Shape>)> {
        let mut out = Vec::new();
        for &ci in &self.alt_colliders {
            let c = &self.def.colliders[ci as usize];
            if !self.s.active[c.node as usize] || !self.s.collider_enabled[ci as usize] {
                continue;
            }
            let d = self.node_mover[c.node as usize].map_or(Affine3A::IDENTITY, |m| self.mover_delta(m));
            let (sc, rot, _) = d.to_scale_rotation_translation();
            let s = sc.x.abs().max(sc.y.abs()).max(sc.z.abs());
            let shapes = match &c.shape {
                ShapeDef::Box { center, half, rot: r } => {
                    vec![Shape::Box(BoxCollider { center: d.transform_point3(*center), half: *half * sc.abs(), rot: rot * *r, slippery: false })]
                }
                ShapeDef::Sphere { center, radius } => {
                    let p = d.transform_point3(*center);
                    vec![Shape::Capsule(Capsule { a: p, b: p, radius: radius * s })]
                }
                ShapeDef::Capsule { a, b, radius } => vec![Shape::Capsule(Capsule { a: d.transform_point3(*a), b: d.transform_point3(*b), radius: radius * s })],
                ShapeDef::Mesh(tris) => tris.iter().map(|t| Shape::Tri(Triangle::new(d.transform_point3(t[0]), d.transform_point3(t[1]), d.transform_point3(t[2])))).collect(),
            };
            out.push((ci, shapes));
        }
        out
    }

    /// Punch.ActiveFrame's environment query (CheckSphere / OverlapSphere r=0.01, else a 4 m
    /// Raycast on environmentMask, which with Physics.queriesHitTriggers also finds the layer-22
    /// item and altar colliders). Returns true when it ended on a layer-22 collider (no dust).
    pub(crate) fn punch_items(&mut self, eye: Vec3, dir: Vec3, env_hit: Option<f32>) -> bool {
        let has_held = self.s.held.is_some();
        let shapes = self.item_shapes();
        let inside: Vec<u32> = shapes.iter().filter(|(_, s)| s.iter().any(|s| s.closest_point(eye).distance(eye) <= 0.01)).map(|(c, _)| *c).collect();
        if !inside.is_empty() {
            for c in inside {
                self.alt_hit(c, has_held);
            }
            return true;
        }
        let mut best: Option<(f32, u32)> = None;
        for (ci, ss) in &shapes {
            for s in ss {
                if let Some((t, _)) = s.raycast(eye, dir, 4.0) {
                    if best.is_none_or(|b| t < b.0) {
                        best = Some((t, *ci));
                    }
                }
            }
        }
        match best {
            Some((t, ci)) if env_hit.is_none_or(|e| t < e) => {
                self.alt_hit(ci, has_held);
                true
            }
            _ => false,
        }
    }

    /// Punch.AltHit (Skull.HitWith's Flammable burn is not ported)
    fn alt_hit(&mut self, ci: u32, has_held: bool) {
        let target = self.def.colliders[ci as usize].node;
        let on = self.scripts_on_node(target).to_vec();
        let item = on.iter().copied().find(|&s| matches!(self.s.scripts[s as usize], Script::Item(_)));
        if item.is_some() && has_held {
            return;
        }
        let zones: Vec<u32> = on.iter().copied().filter(|&s| matches!(self.s.scripts[s as usize], Script::PlaceZone(_))).collect();
        // infiniteSource's CreateCopy (a fresh clone) is not ported: the source itself is taken
        if self.s.held.is_some() && !zones.is_empty() {
            self.place_held(&zones, target);
        } else if self.s.held.is_none() {
            if let Some(it) = item {
                self.force_hold(it);
            }
        }
    }

    /// Punch.ForceHold
    pub fn force_hold(&mut self, it: u32) {
        let Some(holder) = self.punch_holder else { return };
        let node = self.def.scripts[it as usize].node;
        let Script::Item(i) = &self.s.scripts[it as usize] else { return };
        let i = i.clone();
        if let Some(r) = self.vm_rigs.1 {
            if !i.no_holding_animation {
                crate::anim::set_param(self, r, "SemiHolding", 0.0);
                crate::anim::set_param(self, r, "Holding", 1.0);
            } else {
                crate::anim::set_param(self, r, "SemiHolding", 1.0);
            }
        }
        // GetComponentsInParent<ItemPlaceZone>(), before the item leaves its parent
        let mut zones = Vec::new();
        let mut n = Some(node);
        while let Some(m) = n {
            if self.s.active[m as usize] {
                zones.extend(self.scripts_on_node(m).iter().copied().filter(|&s| matches!(self.s.scripts[s as usize], Script::PlaceZone(_))));
            }
            n = self.eff_parent(m);
        }
        self.s.held = Some(it);
        // ResetHeldItemPosition
        let local = if i.reverse_transform { trs(i.put_down_pos, i.put_down_rot, i.put_down_scale) } else { Mat4::IDENTITY };
        if let Script::Item(im) = &mut self.s.scripts[it as usize] {
            im.ipz = None;
            im.picked_up = true;
            im.been_picked_up = true;
        }
        self.set_parent(node, holder, local);
        self.item_message(node, "OffCorrectUse");
        self.set_item_colliders(node, false);
        self.item_message(node, "PickUp");
        for z in zones {
            self.check_item(z, false);
        }
    }

    /// Punch.PlaceHeldObject
    fn place_held(&mut self, zones: &[u32], target: u32) {
        let Some(it) = self.s.held else { return };
        let node = self.def.scripts[it as usize].node;
        if let Some(r) = self.vm_rigs.1 {
            crate::anim::set_param(self, r, "Holding", 0.0);
            crate::anim::set_param(self, r, "SemiHolding", 0.0);
        }
        let Script::Item(i) = &mut self.s.scripts[it as usize] else { return };
        i.picked_up = false;
        let local = if i.reverse_transform { Mat4::IDENTITY } else { trs(i.put_down_pos, i.put_down_rot, i.put_down_scale) };
        self.set_parent(node, target, local);
        self.set_item_colliders(node, true);
        self.item_message(node, "PutDown");
        self.s.held = None;
        for &z in zones {
            self.check_item(z, false);
        }
    }

    fn set_item_colliders(&mut self, node: u32, on: bool) {
        for n in self.eff_subtree(node) {
            for (ci, c) in self.def.colliders.iter().enumerate() {
                if c.node == n {
                    self.s.collider_enabled[ci] = on;
                }
            }
        }
    }

    /// Punch.PunchStart: SendMessage("PunchWith") to the held item.
    pub(crate) fn punch_with_held(&mut self) {
        if let Some(n) = self.held_item() {
            self.item_message(n, "PunchWith");
        }
    }
}
