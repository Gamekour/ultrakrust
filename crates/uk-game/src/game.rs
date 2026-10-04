//! The level runtime: a small Unity-like object model (active-in-hierarchy,
//! Awake/OnEnable/Start/OnDisable, Invoke timers, trigger enter/exit) driving
//! ports of ULTRAKILL's progression scripts, plus player health, combat and
//! checkpoints. Engine-agnostic; the Bevy frontend reads its state.

use crate::enemy::{self, Enemy, Projectile};
use crate::scripts::{self, Call, Script, Target, UEvent};
use bevy_math::{Affine3A, Mat4, Quat, Vec3};
use std::sync::Arc;
use uk_assets::scenedef::{tags, SceneDef, ShapeDef};
use uk_core::collide::{BoxCollider, Capsule, Shape, Triangle, World};
use uk_core::player::{Input, Player};

/// Layers whose solid colliders block the player (LMD.Environment + Default).
const SOLID_LAYERS: &[u8] = &[0, 6, 8, 24];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    ObjActivate,
    ArenaSpawn,
    WaveSpawn,
    WaveEnd,
    FinalDoorOpenDoors,
    FinalDoorOpenerGoTime,
    HideMessage,
}

#[derive(Clone, Debug)]
pub struct Invoke {
    pub at: f64,
    pub script: u32,
    pub act: Act,
}

#[derive(Clone, Debug)]
pub struct Message {
    pub text: String,
    pub owner: Option<u32>,
    pub until: Option<f64>,
}

/// Something the frontend may want to show or play.
#[derive(Clone, Debug, PartialEq)]
pub enum GameEvent {
    Hurt(i32),
    Healed(i32),
    Died,
    Respawned,
    Checkpoint,
    EnemyHit { pos: Vec3, damage: f32, head: bool },
    EnemyKilled { pos: Vec3 },
    Broke { pos: Vec3 },
    DoorMoved,
    WeaponGot(&'static str),
    LevelComplete,
    Shot { from: Vec3, to: Vec3, pierce: bool },
    PunchHit,
}

#[derive(Clone)]
pub struct State {
    pub time: f64,
    pub active_self: Vec<bool>,
    pub active: Vec<bool>,
    pub destroyed: Vec<bool>,
    pub script_enabled: Vec<bool>,
    pub script_awake: Vec<bool>,
    pub script_started: Vec<bool>,
    pub collider_enabled: Vec<bool>,
    pub scripts: Vec<Script>,
    pub local_pos: Vec<Vec3>,
    pub invokes: Vec<Invoke>,
    /// Trigger colliders the player is currently inside.
    pub inside: Vec<u32>,
    pub player: Player,
    pub hp: i32,
    pub dead: bool,
    pub dead_timer: f32,
    pub has_revolver: bool,
    pub enemies: Vec<Enemy>,
    pub projectiles: Vec<Projectile>,
    pub messages: Vec<Message>,
    pub level_complete: bool,
    pub kills: u32,
    pub checkpoint_pos: Option<Vec3>,
    pub checkpoint_yaw: f32,
}

pub struct Mover {
    pub node: u32,
    pub group: usize,
    pub world0: Mat4,
}

pub struct Game {
    pub def: Arc<SceneDef>,
    pub s: State,
    pub world: World,
    /// The level's baked navmesh (humanoid agent type), if it has one.
    pub nav: Option<crate::nav::NavGraph>,
    /// node -> mover index (nearest mover ancestor, including itself)
    pub node_mover: Vec<Option<u32>>,
    pub movers: Vec<Mover>,
    pub triggers: Vec<u32>,
    pub spawn_yaw: f32,
    pub player_node: Option<u32>,
    checkpoint: Option<Box<State>>,
    start: Option<Box<State>>,
    /// Nodes whose active-in-hierarchy changed since the frontend last drained it.
    pub changed_nodes: Vec<u32>,
    /// Set when the whole state was replaced (respawn): frontend refreshes everything.
    pub full_refresh: bool,
    pub events: Vec<GameEvent>,
    pending_events: Vec<(u32, bool)>,
    scripts_by_node: Vec<Vec<u32>>,
    pub unknown_calls: std::collections::BTreeSet<String>,
}

fn layer_solid(l: u8) -> bool {
    SOLID_LAYERS.contains(&l)
}

impl Game {
    pub fn new(def: Arc<SceneDef>) -> Self {
        let n = def.nodes.len();
        let mut scripts_by_node = vec![Vec::new(); n];
        for (i, s) in def.scripts.iter().enumerate() {
            scripts_by_node[s.node as usize].push(i as u32);
        }
        // The original player rig: we drive our own V1, so its subtree is ignored.
        let player_node = def.scripts.iter().find(|s| s.class == "NewMovement").map(|s| s.node);
        let in_player = |node: u32| player_node.is_some_and(|p| def.is_descendant(node, p));

        // Enemies: roots carrying an EnemyIdentifier.
        let mut scripts_rt: Vec<Script> = (0..def.scripts.len()).map(|i| scripts::parse(&def, i)).collect();
        let mut enemies = Vec::new();
        for (i, s) in def.scripts.iter().enumerate() {
            if s.class == "EnemyIdentifier" {
                if let Some(e) = enemy::Enemy::from_def(&def, s.node, i as u32) {
                    scripts_rt[i] = Script::Enemy(enemies.len());
                    enemies.push(e);
                }
            }
        }
        let enemy_root: Vec<Option<u32>> = {
            let mut v = vec![None; n];
            for e in &enemies {
                let mut stack = vec![e.node];
                while let Some(m) = stack.pop() {
                    v[m as usize] = Some(e.node);
                    stack.extend(def.nodes[m as usize].children.iter().copied());
                }
            }
            v
        };

        // Movers: doors (scripts that translate their node) and enemy roots.
        let mut movers = Vec::new();
        let mut node_mover = vec![None; n];
        let mut world = World::default();
        let mark_mover = |node: u32, world: &mut World, movers: &mut Vec<Mover>, node_mover: &mut Vec<Option<u32>>| {
            if node_mover[node as usize].is_some_and(|m: u32| movers[m as usize].node == node) {
                return;
            }
            let group = world.new_group();
            let idx = movers.len() as u32;
            movers.push(Mover { node, group, world0: def.nodes[node as usize].world0 });
            let mut stack = vec![node];
            while let Some(m) = stack.pop() {
                node_mover[m as usize] = Some(idx);
                stack.extend(def.nodes[m as usize].children.iter().copied());
            }
        };
        for (i, s) in def.scripts.iter().enumerate() {
            if matches!(scripts_rt[i], Script::Door(_)) && !in_player(s.node) {
                mark_mover(s.node, &mut world, &mut movers, &mut node_mover);
            }
        }
        for e in &enemies {
            mark_mover(e.node, &mut world, &mut movers, &mut node_mover);
        }

        // Collision + triggers.
        let mut triggers = Vec::new();
        let mut skipped_shapes = 0;
        for (ci, c) in def.colliders.iter().enumerate() {
            if in_player(c.node) {
                continue;
            }
            if c.trigger {
                if enemy_root[c.node as usize].is_none() {
                    triggers.push(ci as u32);
                }
                continue;
            }
            if enemy_root[c.node as usize].is_some() || !layer_solid(c.layer) {
                continue;
            }
            let group = node_mover[c.node as usize].map(|m| movers[m as usize].group).unwrap_or(0);
            let slippery = def.nodes[c.node as usize].tag == tags::SLIPPERY;
            match &c.shape {
                ShapeDef::Box { center, half, rot } => {
                    let b = BoxCollider { center: *center, half: *half, rot: *rot, slippery };
                    world.add_shape(group, Shape::Box(b), ci as u32);
                }
                ShapeDef::Mesh(tris) => {
                    for t in tris {
                        world.add_shape(group, Shape::Tri(Triangle::new(t[0], t[1], t[2])), ci as u32);
                    }
                }
                ShapeDef::Sphere { center, radius } => {
                    // Rare in level geometry: approximate with a box.
                    let b = BoxCollider::new(*center, Vec3::splat(radius * 1.6));
                    world.add_shape(group, Shape::Box(b), ci as u32);
                    skipped_shapes += 1;
                }
                ShapeDef::Capsule { a, b, radius } => {
                    let center = (*a + *b) * 0.5;
                    let axis = *b - *a;
                    let rot = Quat::from_rotation_arc(Vec3::Y, axis.normalize_or(Vec3::Y));
                    let bc = BoxCollider { center, half: Vec3::new(*radius, axis.length() * 0.5 + radius, *radius), rot, slippery };
                    world.add_shape(group, Shape::Box(bc), ci as u32);
                    skipped_shapes += 1;
                }
            }
        }
        let _ = skipped_shapes;
        world.owner_enabled.resize(def.colliders.len(), true);
        world.build();

        // Player spawn from the original rig.
        let (spawn, spawn_yaw, activated) = match player_node {
            Some(p) => {
                let (_, rot, pos) = def.nodes[p as usize].world0.to_scale_rotation_translation();
                let fwd = rot * Vec3::NEG_Z;
                let nm = def.scripts.iter().find(|s| s.class == "NewMovement").map(|s| s.data.get("activated").bool()).unwrap_or(true);
                (pos, fwd.x.atan2(-fwd.z).to_degrees(), nm)
            }
            None => (Vec3::new(0.0, 10.0, 0.0), 0.0, true),
        };
        let mut player = Player::new(spawn);
        player.yaw_deg = spawn_yaw;
        player.activated = activated;

        let s = State {
            time: 0.0,
            active_self: def.nodes.iter().map(|n| n.active_self).collect(),
            active: vec![false; n],
            destroyed: vec![false; n],
            script_enabled: def.scripts.iter().map(|s| s.enabled).collect(),
            script_awake: vec![false; def.scripts.len()],
            script_started: vec![false; def.scripts.len()],
            collider_enabled: def.colliders.iter().map(|c| c.enabled).collect(),
            scripts: scripts_rt,
            local_pos: def.nodes.iter().map(|n| n.local_pos).collect(),
            invokes: Vec::new(),
            inside: Vec::new(),
            player,
            hp: 100,
            dead: false,
            dead_timer: 0.0,
            has_revolver: false,
            enemies,
            projectiles: Vec::new(),
            messages: Vec::new(),
            level_complete: false,
            kills: 0,
            checkpoint_pos: None,
            checkpoint_yaw: 0.0,
        };
        let mut g = Game {
            def: def.clone(),
            s,
            world,
            nav: crate::nav::NavGraph::build(&def.navmeshes),
            node_mover,
            movers,
            triggers,
            spawn_yaw,
            player_node,
            checkpoint: None,
            start: None,
            changed_nodes: Vec::new(),
            full_refresh: true,
            events: Vec::new(),
            pending_events: Vec::new(),
            scripts_by_node,
            unknown_calls: Default::default(),
        };
        // Scene load: activate roots (Awake/OnEnable for everything initially active).
        let roots: Vec<u32> = (0..n as u32).filter(|&i| def.nodes[i as usize].parent.is_none()).collect();
        for r in roots {
            g.refresh(r);
        }
        g.flush_events();
        g.run_starts();
        g.sync_world();
        g.start = Some(Box::new(g.s.clone()));
        g
    }

    // ---------------------------------------------------------------- object model

    pub fn active(&self, n: u32) -> bool {
        self.s.active[n as usize]
    }

    pub fn set_active(&mut self, n: u32, on: bool) {
        let i = n as usize;
        if self.s.destroyed[i] || self.s.active_self[i] == on {
            return;
        }
        self.s.active_self[i] = on;
        self.refresh(n);
        self.flush_events();
    }

    /// Unity `Destroy(gameObject)`: gone for good.
    pub fn destroy(&mut self, n: u32) {
        if self.s.destroyed[n as usize] {
            return;
        }
        self.s.destroyed[n as usize] = true;
        self.refresh(n);
        self.flush_events();
    }

    /// Recomputes active-in-hierarchy below `n` and queues enable/disable callbacks.
    fn refresh(&mut self, n: u32) {
        let parent_active = self.def.nodes[n as usize].parent.map(|p| self.s.active[p as usize]).unwrap_or(true);
        let mut stack = vec![(n, parent_active)];
        while let Some((m, pa)) = stack.pop() {
            let i = m as usize;
            let now = pa && self.s.active_self[i] && !self.s.destroyed[i];
            if now == self.s.active[i] {
                continue;
            }
            self.s.active[i] = now;
            self.changed_nodes.push(m);
            for &sc in &self.scripts_by_node[i] {
                self.pending_events.push((sc, now));
            }
            for &c in self.def.nodes[i].children.iter().rev() {
                stack.push((c, now));
            }
        }
    }

    fn flush_events(&mut self) {
        while !self.pending_events.is_empty() {
            let batch: Vec<(u32, bool)> = std::mem::take(&mut self.pending_events);
            for (sc, on) in batch {
                let i = sc as usize;
                if on {
                    if !self.s.script_awake[i] {
                        self.s.script_awake[i] = true;
                        self.awake(sc);
                    }
                    if self.s.script_enabled[i] {
                        self.on_enable(sc);
                    }
                } else if self.s.script_awake[i] && self.s.script_enabled[i] {
                    self.on_disable(sc);
                }
            }
        }
    }

    fn set_script_enabled(&mut self, sc: u32, on: bool) {
        let i = sc as usize;
        if self.s.script_enabled[i] == on {
            return;
        }
        self.s.script_enabled[i] = on;
        if self.s.active[self.def.scripts[i].node as usize] {
            self.pending_events.push((sc, on));
            if on && !self.s.script_awake[i] {
                // awake handled in flush
            }
            self.flush_events();
        }
    }

    fn script_live(&self, sc: u32) -> bool {
        let i = sc as usize;
        self.s.active[self.def.scripts[i].node as usize] && self.s.script_enabled[i]
    }

    fn invoke(&mut self, sc: u32, act: Act, delay: f32) {
        self.s.invokes.push(Invoke { at: self.s.time + delay as f64, script: sc, act });
    }

    fn cancel_invoke(&mut self, sc: u32, act: Act) {
        self.s.invokes.retain(|i| !(i.script == sc && i.act == act));
    }

    fn node_has_collider(&self, n: u32) -> bool {
        self.def.colliders.iter().any(|c| c.node == n)
    }

    fn node_colliders(&self, n: u32) -> Vec<u32> {
        self.def.colliders.iter().enumerate().filter(|(_, c)| c.node == n).map(|(i, _)| i as u32).collect()
    }

    // ---------------------------------------------------------------- lifecycle dispatch

    fn awake(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        match &mut self.s.scripts[sc as usize] {
            Script::ObjectActivator(oa) => {
                if oa.on_awake {
                    self.start_check(sc);
                }
            }
            Script::Door(_) => self.door_awake(sc),
            Script::FinalDoorOpener { opened, .. } => {
                if !*opened {
                    if let Some(fd) = self.parent_script(node, |s| matches!(s, Script::FinalDoor(_))) {
                        self.final_door_open(fd);
                        if let Script::FinalDoorOpener { opening, .. } = &mut self.s.scripts[sc as usize] {
                            *opening = true;
                        }
                        self.invoke(sc, Act::FinalDoorOpenerGoTime, 1.0);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_enable(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let has_col = self.node_has_collider(node) || self.def.rigidbodies.contains(&node);
        match &mut self.s.scripts[sc as usize] {
            Script::ObjectActivator(oa) => {
                if (!oa.activated || oa.reactivate_on_enable) && oa.non_collider {
                    let ready = oa.obac.map(|o| self.obac_ready(o)).unwrap_or(true);
                    if let Script::ObjectActivator(oa) = &mut self.s.scripts[sc as usize] {
                        if ready {
                            oa.activating = true;
                            let d = oa.delay;
                            self.invoke(sc, Act::ObjActivate, d);
                        }
                    }
                }
            }
            Script::Arena(a) => {
                if !a.activated && a.activate_on_enable {
                    self.arena_activate(sc);
                }
            }
            Script::DoorOpener { door, one_time, .. } => {
                if !has_col {
                    let (d, ot) = (*door, *one_time);
                    if let Some(d) = d {
                        self.door_open(d, false, true);
                    }
                    if ot {
                        self.set_script_enabled(sc, false);
                    }
                }
            }
            Script::FinalDoorOpener { closed, .. } => {
                if *closed {
                    if let Some(fd) = self.parent_script(node, |s| matches!(s, Script::FinalDoor(_))) {
                        self.final_door_open(fd);
                        self.invoke(sc, Act::FinalDoorOpenerGoTime, 1.0);
                    }
                }
            }
            Script::Enemy(e) => {
                let e = *e;
                enemy::on_enable(self, e);
            }
            _ => {}
        }
    }

    fn on_disable(&mut self, sc: u32) {
        match &mut self.s.scripts[sc as usize] {
            Script::ObjectActivator(oa) => {
                oa.activating = false;
                oa.player_in = 0;
                let (act_on_disable, activated, one_time, non_col, dis_exit) =
                    (oa.activate_on_disable, oa.activated, oa.one_time, oa.non_collider, oa.disable_on_exit);
                self.cancel_invoke(sc, Act::ObjActivate);
                if (!activated || !one_time) && act_on_disable {
                    self.obj_activate(sc, true);
                } else if activated && non_col && dis_exit {
                    self.obj_deactivate(sc);
                }
            }
            Script::DoorController(dc) => {
                if dc.player_in && dc.open && dc.kind == 0 {
                    if let Some(d) = dc.door {
                        if !self.door_locked(d) {
                            if let Script::DoorController(dc) = &mut self.s.scripts[sc as usize] {
                                dc.open = false;
                            }
                            self.door_close(d, false);
                        }
                    }
                }
            }
            Script::HudMessage(_) => {}
            _ => {}
        }
    }

    fn run_starts(&mut self) {
        for sc in 0..self.def.scripts.len() as u32 {
            let i = sc as usize;
            if self.s.script_started[i] || !self.script_live(sc) {
                continue;
            }
            self.s.script_started[i] = true;
            let node = self.def.scripts[i].node;
            match &self.s.scripts[i] {
                Script::ObjectActivator(oa) => {
                    if !oa.on_awake {
                        self.start_check(sc);
                    }
                }
                Script::DoorController(_) => {
                    let door = self.parent_script(node, |s| matches!(s, Script::Door(_))).or_else(|| {
                        let p = self.def.nodes[node as usize].parent?;
                        self.child_script(p, |s| matches!(s, Script::Door(_)))
                    });
                    if let Script::DoorController(dc) = &mut self.s.scripts[i] {
                        dc.door = door;
                    }
                }
                Script::FinalDoor(fd) => {
                    let (so, about, opened, light) = (fd.start_open, fd.about_to_open, fd.opened, fd.door_light);
                    if !about {
                        if let Some(l) = light {
                            self.set_active(l, false);
                        }
                    }
                    if so || (about && !opened) {
                        self.final_door_open(sc);
                    }
                }
                _ => {}
            }
        }
    }

    /// GetComponentInParent (self first, then ancestors).
    fn parent_script(&self, mut node: u32, pred: impl Fn(&Script) -> bool) -> Option<u32> {
        loop {
            for &sc in &self.scripts_by_node[node as usize] {
                if pred(&self.s.scripts[sc as usize]) {
                    return Some(sc);
                }
            }
            node = self.def.nodes[node as usize].parent?;
        }
    }

    /// GetComponentInChildren (self first, depth-first, active objects only).
    fn child_script(&self, node: u32, pred: impl Fn(&Script) -> bool) -> Option<u32> {
        let mut stack = vec![node];
        while let Some(m) = stack.pop() {
            if !self.s.active[m as usize] && m != node {
                continue;
            }
            for &sc in &self.scripts_by_node[m as usize] {
                if pred(&self.s.scripts[sc as usize]) {
                    return Some(sc);
                }
            }
            stack.extend(self.def.nodes[m as usize].children.iter().rev().copied());
        }
        None
    }

    fn children_scripts(&self, node: u32, pred: impl Fn(&Script) -> bool) -> Vec<u32> {
        let mut out = Vec::new();
        let mut stack = vec![node];
        while let Some(m) = stack.pop() {
            if !self.s.active[m as usize] {
                continue;
            }
            for &sc in &self.scripts_by_node[m as usize] {
                if pred(&self.s.scripts[sc as usize]) {
                    out.push(sc);
                }
            }
            stack.extend(self.def.nodes[m as usize].children.iter().rev().copied());
        }
        out
    }

    // ---------------------------------------------------------------- events

    pub fn run_uevent(&mut self, ev: &UEvent, revert: bool) {
        if !revert {
            for &n in &ev.to_deactivate {
                self.set_active(n, false);
            }
            for &n in &ev.to_activate {
                self.set_active(n, true);
            }
            for c in &ev.on_activate {
                self.run_call(c);
            }
        } else {
            for &n in &ev.to_deactivate {
                self.set_active(n, true);
            }
            for &n in &ev.to_activate {
                self.set_active(n, false);
            }
            for c in &ev.on_deactivate {
                self.run_call(c);
            }
        }
    }

    pub fn run_call(&mut self, c: &Call) {
        match (&c.target, c.method.as_str()) {
            (Target::Node(n), "SetActive") => self.set_active(*n, c.bool_arg),
            (Target::Script(s), "SetActive") => {
                let n = self.def.scripts[*s as usize].node;
                self.set_active(n, c.bool_arg)
            }
            (Target::Script(s), "set_enabled") => self.set_script_enabled(*s, c.bool_arg),
            (Target::Collider(col), "set_enabled") => self.s.collider_enabled[*col as usize] = c.bool_arg,
            (Target::Script(s), m) => {
                let s = *s;
                match (&self.s.scripts[s as usize], m) {
                    (Script::ObjectActivator(_), "Activate") => self.obj_activate(s, false),
                    (Script::ObjectActivator(_), "Deactivate") => self.obj_deactivate(s),
                    (Script::ObjectActivator(_), "ActivateDelayed") => self.invoke(s, Act::ObjActivate, c.float_arg),
                    (Script::ObjectActivationCheck { .. }, "StateChange") => {
                        self.s.scripts[s as usize] = Script::ObjectActivationCheck { ready: c.bool_arg }
                    }
                    (Script::PlayerActivator { .. }, "Activate") => self.player_activator(s),
                    (Script::Arena(_), "Activate") => self.arena_activate(s),
                    (Script::Breakable(_), "Break") => self.breakable_break(s, 99999.0),
                    (Script::Glass(_), "Shatter") => self.glass_shatter(s),
                    (Script::Door(_), "Open") => self.door_open(s, false, false),
                    (Script::Door(_), "SimpleOpenOverride") => self.door_open(s, false, true),
                    (Script::Door(_), "Close") => self.door_close(s, false),
                    (Script::Door(_), "Lock") => self.door_lock(s),
                    (Script::Door(_), "Unlock") => self.door_unlock(s),
                    (Script::FinalDoor(_), "Open") => self.final_door_open(s),
                    (Script::FinalDoor(_), "OpenDoors") => self.final_door_open_doors(s),
                    (_, "AbruptChangeLevel") => self.complete_level(),
                    _ => {
                        self.unknown_calls.insert(format!("{}.{}", self.def.scripts[s as usize].class, m));
                    }
                }
            }
            (_, "AbruptChangeLevel") => self.complete_level(),
            (t, m) => {
                self.unknown_calls.insert(format!("{t:?}.{m}"));
            }
        }
    }

    fn complete_level(&mut self) {
        if !self.s.level_complete {
            self.s.level_complete = true;
            self.events.push(GameEvent::LevelComplete);
        }
    }

    // ---------------------------------------------------------------- ObjectActivator

    fn obac_ready(&self, o: u32) -> bool {
        match &self.s.scripts[o as usize] {
            Script::ObjectActivationCheck { ready } => *ready,
            _ => true,
        }
    }

    fn start_check(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let has_col = self.node_has_collider(node) || self.def.rigidbodies.contains(&node);
        let Script::ObjectActivator(oa) = &mut self.s.scripts[sc as usize] else { return };
        if oa.dont_activate_on_enable || has_col {
            return;
        }
        oa.non_collider = true;
        let ready = oa.obac.map(|o| self.obac_ready(o)).unwrap_or(true);
        let Script::ObjectActivator(oa) = &mut self.s.scripts[sc as usize] else { return };
        if ready && (!oa.one_time || (!oa.activating && !oa.activated)) {
            if oa.delay == 0.0 {
                self.obj_activate(sc, false);
            } else {
                let d = oa.delay;
                self.invoke(sc, Act::ObjActivate, d);
            }
        }
    }

    fn obj_activate(&mut self, sc: u32, ignore_disabled: bool) {
        let node = self.def.scripts[sc as usize].node;
        let active_self = self.s.active_self[node as usize];
        let Script::ObjectActivator(oa) = &self.s.scripts[sc as usize] else { return };
        let ready = oa.obac.map(|o| self.obac_ready(o)).unwrap_or(true);
        let Script::ObjectActivator(oa) = &mut self.s.scripts[sc as usize] else { return };
        if (active_self || ignore_disabled) && (!oa.activated || !oa.one_time) && ready {
            oa.activating = false;
            oa.activated = true;
            let ev = oa.events.clone();
            self.run_uevent(&ev, false);
        }
    }

    fn obj_deactivate(&mut self, sc: u32) {
        let Script::ObjectActivator(oa) = &mut self.s.scripts[sc as usize] else { return };
        if !oa.one_time {
            oa.activated = false;
            oa.activating = false;
        }
        let ev = oa.events.clone();
        self.run_uevent(&ev, true);
        self.cancel_invoke(sc, Act::ObjActivate);
    }

    // ---------------------------------------------------------------- Door

    fn door_awake(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let parent = self.def.nodes[node as usize].parent;
        let docons = match parent {
            Some(p) => self.children_scripts(p, |s| matches!(s, Script::DoorController(_))),
            None => Vec::new(),
        };
        let layer = self.def.nodes[node as usize].layer;
        let local = self.s.local_pos[node as usize];
        let solid_col = self.node_colliders(node).into_iter().find(|&c| !self.def.colliders[c as usize].trigger);
        let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
        if d.requests == 1 {
            d.requests = 0;
            d.start_open = true;
        }
        if !d.got_pos {
            d.got_pos = true;
            d.closed_pos = local;
            d.open_pos = local + d.open_offset;
            if d.start_open {
                self.s.local_pos[node as usize] = d.open_pos;
            }
        }
        d.docons = docons;
        if d.docons.is_empty() && ![6u8, 8, 24].contains(&layer) {
            if let Some(c) = solid_col {
                d.doconless_col = Some(c);
                self.s.collider_enabled[c as usize] = !(d.start_open || d.open);
            }
        }
        d.got_values = true;
        if d.start_open || d.open {
            d.open = true;
        }
        let (locked, no_pass) = (d.locked, d.no_pass);
        if locked {
            if let Some(np) = no_pass {
                self.set_active(np, true);
            }
        }
    }

    fn door_locked(&self, sc: u32) -> bool {
        matches!(&self.s.scripts[sc as usize], Script::Door(d) if d.locked)
    }

    pub fn door_open(&mut self, sc: u32, enemy_open: bool, skull: bool) {
        let node = self.def.scripts[sc as usize].node;
        if !self.s.script_awake[sc as usize] {
            // Door not awake yet (inactive): remember the request like Unity's gotValues path.
            if let Script::Door(d) = &mut self.s.scripts[sc as usize] {
                d.start_open = true;
            }
            return;
        }
        let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
        if !skull || d.docons.is_empty() {
            d.requests += 1;
        } else {
            let any_active = d.docons.iter().any(|&dc| self.s.active[self.def.scripts[dc as usize].node as usize]);
            let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
            if !any_active {
                d.requests += 1;
            }
        }
        let Script::Door(d) = &self.s.scripts[sc as usize] else { return };
        let rooms = d.activated_rooms.clone();
        for r in rooms {
            self.set_active(r, true);
        }
        let local = self.s.local_pos[node as usize];
        let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
        if (d.open && !skull) || local == d.open_pos {
            return;
        }
        if !d.got_values {
            d.start_open = true;
            return;
        }
        d.open = true;
        let close_others = !enemy_open && !d.dont_close_others && !d.docons.is_empty();
        d.target_pos = d.open_pos;
        d.in_pos = false;
        if let Some(c) = d.doconless_col {
            self.s.collider_enabled[c as usize] = false;
        }
        self.events.push(GameEvent::DoorMoved);
        if close_others {
            for other in 0..self.s.scripts.len() as u32 {
                if other == sc {
                    continue;
                }
                let Script::Door(od) = &self.s.scripts[other as usize] else { continue };
                if !(od.open && !od.start_open && !od.dont_close_when_another_opens) {
                    continue;
                }
                let onode = self.def.scripts[other as usize].node;
                let ctl = self.def.nodes[onode as usize]
                    .parent
                    .and_then(|p| self.child_script(p, |s| matches!(s, Script::DoorController(_))));
                if let Some(ctl) = ctl {
                    if let Script::DoorController(dc) = &self.s.scripts[ctl as usize] {
                        if dc.kind == 0 {
                            self.door_close(other, false);
                        }
                    }
                }
            }
        }
    }

    pub fn door_close(&mut self, sc: u32, force: bool) {
        let node = self.def.scripts[sc as usize].node;
        let local = self.s.local_pos[node as usize];
        let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
        if !d.got_pos {
            return;
        }
        if d.requests > 1 && !force {
            d.requests -= 1;
            return;
        }
        if local == d.closed_pos {
            return;
        }
        d.open = false;
        if d.requests > 0 && !force {
            d.requests -= 1;
        } else if force {
            d.requests = 0;
        }
        d.start_open = false;
        d.target_pos = d.closed_pos;
        d.in_pos = false;
        if let Some(c) = d.doconless_col {
            self.s.collider_enabled[c as usize] = true;
        }
        self.events.push(GameEvent::DoorMoved);
    }

    pub fn door_lock(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let local = self.s.local_pos[node as usize];
        let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
        if !d.got_values {
            d.locked = true;
            return;
        }
        if d.locked {
            return;
        }
        d.locked = true;
        let (np, closed) = (d.no_pass, d.closed_pos);
        if let Some(np) = np {
            self.set_active(np, true);
        }
        if local != closed {
            self.door_close(sc, true);
        }
    }

    pub fn door_unlock(&mut self, sc: u32) {
        let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
        if !d.got_values {
            d.locked = false;
            return;
        }
        d.locked = false;
        let (np, open_on_unlock, open) = (d.no_pass, d.open_on_unlock, d.open);
        if let Some(np) = np {
            self.set_active(np, false);
        }
        if open_on_unlock && !open {
            self.door_open(sc, false, false);
        }
    }

    fn door_optimize(&mut self, sc: u32) {
        let Script::Door(d) = &self.s.scripts[sc as usize] else { return };
        let rooms = d.deactivated_rooms.clone();
        for r in rooms {
            self.set_active(r, false);
        }
    }

    fn door_update(&mut self, sc: u32, dt: f32) {
        let node = self.def.scripts[sc as usize].node;
        let local = self.s.local_pos[node as usize];
        let Script::Door(d) = &mut self.s.scripts[sc as usize] else { return };
        if d.in_pos {
            return;
        }
        let span = d.closed_pos.distance(d.open_pos).min(100.0).max(1e-4);
        let num = local.distance(d.target_pos) / span;
        let speed = if d.ease_in { d.speed.min(num * 2.0 * d.speed + d.speed / 50.0) } else { d.speed };
        let new = uk_core::umath::vmove_towards(local, d.target_pos, dt * speed);
        self.s.local_pos[node as usize] = new;
        if new.distance(d.target_pos) < 0.1f32.min(d.closed_pos.distance(d.open_pos) / 1000.0).max(1e-5) {
            self.s.local_pos[node as usize] = d.target_pos;
            d.in_pos = true;
            let calls = if d.target_pos == d.open_pos { d.on_fully_opened.clone() } else { d.on_fully_closed.clone() };
            for c in calls {
                self.run_call(&c);
            }
        }
    }

    fn door_controller_update(&mut self, sc: u32) {
        let Script::DoorController(dc) = &self.s.scripts[sc as usize] else { return };
        if dc.destroyed {
            return;
        }
        let Some(door) = dc.door else { return };
        let locked = self.door_locked(door);
        let (player_in, open, kind) = (dc.player_in, dc.open, dc.kind);
        if player_in && !open && !locked {
            if let Script::DoorController(dc) = &mut self.s.scripts[sc as usize] {
                dc.open = true;
            }
            self.door_optimize(door);
            match kind {
                0 => self.door_open(door, false, false),
                1 => {
                    self.door_open(door, false, false);
                    if let Script::DoorController(dc) = &mut self.s.scripts[sc as usize] {
                        dc.destroyed = true;
                    }
                }
                2 => {
                    self.door_close(door, false);
                    if let Script::DoorController(dc) = &mut self.s.scripts[sc as usize] {
                        dc.destroyed = true;
                    }
                }
                _ => {}
            }
        } else if open && !locked && !player_in {
            if let Script::DoorController(dc) = &mut self.s.scripts[sc as usize] {
                dc.open = false;
            }
            self.door_close(door, false);
        }
        let dnode = self.def.scripts[door as usize].node;
        let at_closed = matches!(&self.s.scripts[door as usize], Script::Door(d) if self.s.local_pos[dnode as usize] == d.closed_pos);
        if let Script::DoorController(dc) = &mut self.s.scripts[sc as usize] {
            if !dc.player_in && at_closed {
                dc.open = false;
            }
        }
    }

    // ---------------------------------------------------------------- arenas & waves

    fn arena_activate(&mut self, sc: u32) {
        let Script::Arena(a) = &mut self.s.scripts[sc as usize] else { return };
        if a.activated || a.destroyed {
            return;
        }
        a.activated = true;
        let (doors, has_enemies) = (a.doors.clone(), !a.enemies.is_empty());
        if !doors.is_empty() {
            for d in doors {
                let dn = self.def.scripts[d as usize].node;
                if !self.s.active_self[dn as usize] {
                    self.set_active(dn, true);
                }
                self.door_lock(d);
            }
            if has_enemies {
                self.invoke(sc, Act::ArenaSpawn, 1.0);
            } else if let Script::Arena(a) = &mut self.s.scripts[sc as usize] {
                a.destroyed = true;
            }
        } else if has_enemies {
            self.arena_spawn(sc);
        } else if let Script::Arena(a) = &mut self.s.scripts[sc as usize] {
            a.destroyed = true;
        }
    }

    fn arena_spawn(&mut self, sc: u32) {
        let Script::Arena(a) = &mut self.s.scripts[sc as usize] else { return };
        if a.current >= a.enemies.len() {
            a.destroyed = true;
            return;
        }
        let e = a.enemies[a.current];
        a.current += 1;
        let more = a.current < a.enemies.len();
        let was_on = self.s.active_self[e as usize];
        if !was_on {
            self.set_active(e, true);
        }
        if more {
            self.invoke(sc, Act::ArenaSpawn, if was_on { 0.0 } else { 0.1 });
        } else if let Script::Arena(a) = &mut self.s.scripts[sc as usize] {
            a.destroyed = true;
        }
    }

    fn wave_fixed(&mut self, sc: u32) {
        let Script::Wave(w) = &mut self.s.scripts[sc as usize] else { return };
        if w.destroyed {
            return;
        }
        if w.dead < 0 {
            w.dead = 0;
        }
        if w.activated || w.dead < w.enemy_count {
            return;
        }
        w.activated = true;
        let delay = if w.no_activation_delay { 0.0 } else { 1.0 };
        if !w.last_wave {
            let (ta, doors) = (w.to_activate.clone(), w.doors.clone());
            for n in ta {
                self.set_active(n, true);
            }
            for d in doors {
                self.door_unlock(d);
            }
            self.invoke(sc, Act::WaveSpawn, delay);
        } else {
            self.invoke(sc, Act::WaveEnd, delay);
        }
    }

    fn wave_spawn(&mut self, sc: u32) {
        let Script::Wave(w) = &mut self.s.scripts[sc as usize] else { return };
        if !w.next_enemies.is_empty() {
            let e = w.next_enemies.get(w.current_enemy).copied();
            w.current_enemy += 1;
            if let Some(e) = e {
                self.set_active(e, true);
            }
        }
        let Script::Wave(w) = &mut self.s.scripts[sc as usize] else { return };
        if w.current_enemy < w.next_enemies.len() {
            self.invoke(sc, Act::WaveSpawn, 0.1);
        } else {
            w.destroyed = true;
        }
    }

    fn wave_end(&mut self, sc: u32) {
        let Script::Wave(w) = &mut self.s.scripts[sc as usize] else { return };
        if !w.to_activate.is_empty() && !w.objects_activated {
            w.objects_activated = true;
            let ta = w.to_activate.clone();
            for n in ta {
                self.set_active(n, true);
            }
            self.wave_end(sc);
        } else if w.current_door < w.doors.len() {
            let d = w.doors[w.current_door];
            let fwd = w.door_forward == Some(d);
            w.current_door += 1;
            self.door_unlock(d);
            if fwd {
                self.door_open(d, false, true);
            }
            self.invoke(sc, Act::WaveEnd, 0.1);
        } else {
            w.destroyed = true;
        }
    }

    /// `ActivateNextWave.AddDeadEnemy` on the nearest wave above a dead enemy (and its linked waves).
    pub fn add_dead_enemy(&mut self, enemy_node: u32) {
        let Some(sc) = self.parent_script(enemy_node, |s| matches!(s, Script::Wave(_))) else { return };
        let node = self.def.scripts[sc as usize].node;
        for &other in &self.scripts_by_node[node as usize].clone() {
            if let Script::Wave(w) = &mut self.s.scripts[other as usize] {
                w.dead += 1;
            }
        }
    }

    // ---------------------------------------------------------------- breakables, glass

    pub fn breakable_break(&mut self, sc: u32, damage: f32) {
        let node = self.def.scripts[sc as usize].node;
        let Script::Breakable(b) = &mut self.s.scripts[sc as usize] else { return };
        if b.unbreakable || b.broken {
            return;
        }
        if b.durability > damage {
            b.durability -= damage;
            return;
        }
        b.broken = true;
        let (aob, dob, ev) = (b.activate_on_break.clone(), b.destroy_on_break.clone(), b.destroy_event.clone());
        for n in aob {
            self.set_active(n, true);
        }
        for n in dob {
            self.destroy(n);
        }
        self.run_uevent(&ev, false);
        let pos = self.def.nodes[node as usize].world0.w_axis.truncate();
        self.events.push(GameEvent::Broke { pos });
        self.destroy(node);
    }

    pub fn glass_shatter(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        if std::env::var("DEBUG_BREAK").is_ok() && matches!(&self.s.scripts[sc as usize], Script::Glass(g) if !g.broken) {
            eprintln!("glass shatter {} at t={:.2} player {:?} wind {:.2}
{}", self.def.path(node), self.s.time, self.s.player.pos, self.s.player.wind_state,
                std::backtrace::Backtrace::force_capture().to_string().lines().filter(|l| l.contains("uk_game::")).take(4).collect::<Vec<_>>().join("
"));
        }
        let Script::Glass(g) = &mut self.s.scripts[sc as usize] else { return };
        if g.broken {
            return;
        }
        g.broken = true;
        let ev = g.on_shatter.clone();
        self.run_uevent(&ev, false);
        for c in self.def.nodes[node as usize].children.clone() {
            self.destroy(c);
        }
        for c in self.node_colliders(node) {
            if !self.def.colliders[c as usize].trigger {
                self.s.collider_enabled[c as usize] = false;
            }
        }
        let pos = self.def.nodes[node as usize].world0.w_axis.truncate();
        self.events.push(GameEvent::Broke { pos });
    }

    // ---------------------------------------------------------------- player-facing scripts

    fn player_activator(&mut self, sc: u32) {
        let Script::PlayerActivator { activated, .. } = &mut self.s.scripts[sc as usize] else { return };
        if *activated {
            return;
        }
        *activated = true;
        self.s.player.activated = true;
    }

    fn final_door_open(&mut self, sc: u32) {
        let Script::FinalDoor(fd) = &mut self.s.scripts[sc as usize] else { return };
        fd.about_to_open = true;
        let light = fd.door_light;
        self.invoke(sc, Act::FinalDoorOpenDoors, 1.0);
        if let Some(l) = light {
            self.set_active(l, true);
        }
    }

    fn final_door_open_doors(&mut self, sc: u32) {
        let Script::FinalDoor(fd) = &mut self.s.scripts[sc as usize] else { return };
        if fd.opened {
            return;
        }
        fd.opened = true;
        fd.about_to_open = false;
        let (doors, blocker) = (fd.doors.clone(), fd.closing_blocker);
        for d in doors {
            self.door_open(d, false, true);
        }
        if let Some(b) = blocker {
            self.set_active(b, false);
        }
    }

    fn checkpoint_activate(&mut self, sc: u32) {
        let Script::CheckPoint(cp) = &mut self.s.scripts[sc as usize] else { return };
        if cp.activated {
            return;
        }
        cp.activated = true;
        let (ta, graphic, doors) = (cp.to_activate, cp.graphic, cp.doors_to_unlock.clone());
        if let Some(t) = ta {
            self.set_active(t, true);
        }
        if let Some(g) = graphic {
            self.set_active(g, false);
        }
        for d in doors {
            self.door_unlock(d);
        }
        // CheckPoint.OnRespawn: player at checkpoint position + up * 1.25, facing its forward
        let node = self.def.scripts[sc as usize].node;
        let (_, rot, pos) = self.def.nodes[node as usize].world0.to_scale_rotation_translation();
        self.s.checkpoint_pos = Some(pos + rot * Vec3::Y * 1.25);
        let fwd = rot * Vec3::NEG_Z;
        self.s.checkpoint_yaw = fwd.x.atan2(-fwd.z).to_degrees();
        self.events.push(GameEvent::Checkpoint);
        self.checkpoint = Some(Box::new(self.s.clone()));
    }

    fn death_zone(&mut self, sc: u32) {
        let Script::DeathZone(dz) = &mut self.s.scripts[sc as usize] else { return };
        if dz.disabled || !dz.player_affected {
            return;
        }
        let ev = dz.on_hit_player.clone();
        let (insta, damage, target) = (!dz.not_instakill, dz.damage, dz.respawn_target);
        if insta {
            dz.disabled = true;
        }
        self.run_uevent(&ev, false);
        if insta {
            self.hurt_player(999_999, false);
        } else if self.s.hp > 0 {
            if damage == 0 || self.s.hp == 1 {
            } else if self.s.hp > damage {
                self.hurt_player(damage, true);
            } else if self.s.hp > 1 {
                let d = self.s.hp - 1;
                self.hurt_player(d, true);
            }
            // send the player back to safety
            let node = self.def.scripts[sc as usize].node;
            let up = self.def.nodes[node as usize].world0.transform_vector3(Vec3::Y).normalize_or(Vec3::Y);
            let dest = match (target, self.s.checkpoint_pos) {
                (Some(t), _) => t + up * 1.25,
                (None, Some(c)) => c + Vec3::Y * 1.25,
                (None, None) => self.start.as_ref().map(|s| s.player.pos).unwrap_or(self.s.player.pos),
            };
            self.s.player.vel = Vec3::ZERO;
            self.s.player.pos = dest;
            self.s.player.prev_pos = dest;
        }
    }

    /// OutOfBoundsTargetSetter.Activate: point death zones' respawn target here.
    fn oob_target_setter(&mut self, sc: u32) {
        let Script::OobTargetSetter { death_zones } = &self.s.scripts[sc as usize] else { return };
        let explicit = !death_zones.is_empty();
        let list: Vec<u32> = if explicit {
            death_zones.clone()
        } else {
            (0..self.s.scripts.len() as u32).filter(|&i| matches!(self.s.scripts[i as usize], Script::DeathZone(_))).collect()
        };
        let node = self.def.scripts[sc as usize].node;
        let pos = self.def.nodes[node as usize].world0.w_axis.truncate();
        for d in list {
            if let Script::DeathZone(dz) = &mut self.s.scripts[d as usize] {
                if !dz.dont_change_respawn_target || explicit {
                    dz.respawn_target = Some(pos);
                }
            }
        }
    }

    fn teleport(&mut self, sc: u32) {
        let Script::Teleport(t) = &self.s.scripts[sc as usize] else { return };
        let t = (**t).clone();
        if t.affect_position {
            let new = if t.not_relative { t.objective } else { self.s.player.pos + t.relative };
            let d = new - self.s.player.pos;
            self.s.player.pos += d;
            self.s.player.prev_pos = self.s.player.pos;
        }
        if t.reset_speed {
            self.s.player.vel = Vec3::ZERO;
        }
        self.run_uevent(&t.on_teleport, false);
    }

    fn hud_message(&mut self, sc: u32, enter: bool) {
        let Script::HudMessage(h) = &mut self.s.scripts[sc as usize] else { return };
        if enter {
            if h.dont_on_trigger {
                return;
            }
            if h.deactivating {
                self.s.messages.clear();
                return;
            }
            if h.shown && !h.not_one_time {
                return;
            }
            h.shown = true;
            let until = if h.timed { Some(self.s.time + h.timer as f64) } else { None };
            let text = h.message.clone();
            self.s.messages.retain(|m| m.owner != Some(sc));
            self.s.messages.push(Message { text, owner: Some(sc), until });
        } else if h.deactivate_on_exit {
            self.s.messages.retain(|m| m.owner != Some(sc));
        }
    }

    // ---------------------------------------------------------------- triggers

    fn trigger_contains(&self, ci: u32, cap: &Capsule) -> bool {
        let c = &self.def.colliders[ci as usize];
        let (a, b) = match self.node_mover[c.node as usize] {
            Some(m) => {
                let inv = self.mover_delta(m).inverse();
                (inv.transform_point3(cap.a), inv.transform_point3(cap.b))
            }
            None => (cap.a, cap.b),
        };
        let r = cap.radius;
        match &c.shape {
            ShapeDef::Box { center, half, rot } => {
                let s = Shape::Box(BoxCollider { center: *center, half: *half, rot: *rot, slippery: false });
                let (p, q) = s.closest_to_segment(a, b);
                p.distance_squared(q) <= r * r
            }
            ShapeDef::Sphere { center, radius } => seg_point_dist(a, b, *center) <= r + radius,
            ShapeDef::Capsule { a: ca, b: cb, radius } => seg_seg_dist(a, b, *ca, *cb) <= r + radius,
            ShapeDef::Mesh(tris) => tris.iter().any(|t| {
                let s = Shape::Tri(Triangle::new(t[0], t[1], t[2]));
                let (p, q) = s.closest_to_segment(a, b);
                p.distance_squared(q) <= r * r
            }),
        }
    }

    /// Is a world point inside a trigger collider (at its current pose)?
    pub fn point_in_trigger(&self, ci: u32, p: Vec3) -> bool {
        let cap = Capsule { a: p, b: p, radius: 0.0 };
        self.trigger_contains(ci, &cap)
    }

    fn update_triggers(&mut self) {
        let cap = self.s.player.capsule();
        let mut now = Vec::new();
        for &ci in &self.triggers {
            let c = &self.def.colliders[ci as usize];
            if !self.s.active[c.node as usize] || !self.s.collider_enabled[ci as usize] {
                continue;
            }
            if self.trigger_contains(ci, &cap) {
                now.push(ci);
            }
        }
        let prev = std::mem::take(&mut self.s.inside);
        // Unity sends no exit for colliders that were deactivated.
        for &ci in &prev {
            let c = &self.def.colliders[ci as usize];
            if !now.contains(&ci) && self.s.active[c.node as usize] && self.s.collider_enabled[ci as usize] {
                self.trigger_event(ci, false);
            }
        }
        self.s.inside = now.clone();
        for &ci in &now {
            if !prev.contains(&ci) {
                self.trigger_event(ci, true);
            }
        }
    }

    /// OnCollisionEnter for solid colliders the player touches (DeathZone's
    /// `OnCollisionEnter -> GotHit`, e.g. the fan blades).
    fn update_contacts(&mut self) {
        let mut cap = self.s.player.capsule();
        cap.radius += 0.05;
        let mut touched = Vec::new();
        for id in self.world.overlap_capsule(cap) {
            let o = self.world.owner(id);
            if o == uk_core::collide::ALWAYS {
                continue;
            }
            let node = self.def.colliders[o as usize].node;
            if !touched.contains(&node) {
                touched.push(node);
            }
        }
        for node in touched {
            let scs: Vec<u32> = self.scripts_by_node[node as usize].clone();
            for sc in scs {
                if matches!(self.s.scripts[sc as usize], Script::DeathZone(_)) {
                    self.death_zone(sc);
                }
            }
        }
    }

    /// The GameObject of a collider's attached Rigidbody (itself or nearest ancestor with one).
    fn attached_rigidbody(&self, node: u32) -> Option<u32> {
        let mut n = node;
        loop {
            if self.def.rigidbodies.contains(&n) {
                return Some(n);
            }
            n = self.def.nodes[n as usize].parent?;
        }
    }

    fn trigger_event(&mut self, ci: u32, enter: bool) {
        let node = self.def.colliders[ci as usize].node;
        // Unity sends trigger messages to the collider's GameObject and to its attached
        // Rigidbody's GameObject (compound colliders, e.g. CheckPoint + child "Hitbox").
        let mut targets = self.scripts_by_node[node as usize].clone();
        if let Some(rb) = self.attached_rigidbody(node) {
            if rb != node {
                targets.extend(self.scripts_by_node[rb as usize].iter().copied());
            }
        }
        for sc in targets {
            // Trigger messages reach disabled MonoBehaviours too.
            if !self.s.active[node as usize] && enter {
                continue;
            }
            match &mut self.s.scripts[sc as usize] {
                Script::ObjectActivator(oa) => {
                    if oa.for_enemies {
                        continue;
                    }
                    if enter {
                        oa.player_in += 1;
                        let ready = oa.obac.map(|o| self.obac_ready(o)).unwrap_or(true);
                        let Script::ObjectActivator(oa) = &mut self.s.scripts[sc as usize] else { continue };
                        if (!oa.one_time || (!oa.activating && !oa.activated)) && oa.player_in == 1 && ready {
                            if oa.one_time {
                                oa.activating = true;
                            }
                            let d = oa.delay;
                            self.invoke(sc, Act::ObjActivate, d);
                        }
                    } else {
                        oa.player_in -= 1;
                        if oa.disable_on_exit && (oa.activating || oa.activated) && oa.player_in == 0 {
                            self.obj_deactivate(sc);
                        }
                    }
                }
                Script::DoorController(dc) => dc.player_in = enter,
                Script::DoorOpener { door, one_time, done } => {
                    if enter && !*done && self.s.script_enabled[sc as usize] {
                        if *one_time {
                            *done = true;
                        }
                        if let Some(d) = *door {
                            self.door_open(d, false, true);
                        }
                    }
                }
                Script::Arena(a) => {
                    if enter && !a.for_enemy && !a.activated {
                        self.arena_activate(sc);
                    }
                }
                Script::CheckPoint(_) if enter => self.checkpoint_activate(sc),
                Script::DeathZone(_) if enter => self.death_zone(sc),
                Script::Teleport(_) if enter => self.teleport(sc),
                Script::PlayerActivator { .. } if enter => self.player_activator(sc),
                Script::FinalPit if enter => self.complete_level(),
                Script::OobTargetSetter { .. } if enter => self.oob_target_setter(sc),
                Script::HudMessage(_) => self.hud_message(sc, enter),
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------- movers / world sync

    /// Current world transform delta of a mover relative to its load-time pose.
    pub fn mover_delta(&self, m: u32) -> Affine3A {
        let mv = &self.movers[m as usize];
        let node = mv.node as usize;
        if let Some(e) = self.s.enemies.iter().find(|e| e.node == mv.node) {
            return e.delta();
        }
        let d = self.s.local_pos[node] - self.def.nodes[node].local_pos;
        if d == Vec3::ZERO {
            return Affine3A::IDENTITY;
        }
        let parent_lin = self.def.nodes[node].parent.map(|p| self.def.nodes[p as usize].world0).unwrap_or(Mat4::IDENTITY);
        Affine3A::from_translation(parent_lin.transform_vector3(d))
    }

    fn sync_world(&mut self) {
        for (ci, c) in self.def.colliders.iter().enumerate() {
            let on = self.s.active[c.node as usize] && self.s.collider_enabled[ci];
            if let Some(e) = self.world.owner_enabled.get_mut(ci) {
                *e = on;
            }
        }
        for m in 0..self.movers.len() {
            let d = self.mover_delta(m as u32);
            let g = self.movers[m].group;
            self.world.set_group_transform(g, d);
        }
    }

    // ---------------------------------------------------------------- player

    pub fn hurt_player(&mut self, damage: i32, invincible: bool) {
        if self.s.dead || self.s.level_complete || damage <= 0 {
            return;
        }
        if invincible && self.s.player.invincible_layer {
            return;
        }
        if invincible {
            self.s.player.hurt_invincibility = if damage >= 50 { 0.8 } else { 0.5 };
            self.s.player.invincible_layer = true;
        }
        self.s.hp = (self.s.hp - damage).max(0);
        self.events.push(GameEvent::Hurt(damage));
        if self.s.hp == 0 {
            self.s.dead = true;
            self.s.dead_timer = 0.0;
            self.events.push(GameEvent::Died);
        }
    }

    pub fn hurt_player_ignoring_invincibility(&mut self, damage: i32) {
        let saved = self.s.player.invincible_layer;
        self.s.player.invincible_layer = false;
        self.hurt_player(damage, false);
        self.s.player.invincible_layer = saved;
    }

    pub fn heal_player(&mut self, amount: i32) {
        if self.s.dead || self.s.hp >= 100 {
            return;
        }
        let before = self.s.hp;
        self.s.hp = (self.s.hp + amount).min(100);
        self.events.push(GameEvent::Healed(self.s.hp - before));
    }

    pub fn respawn(&mut self) {
        let snap = self.checkpoint.clone().or_else(|| self.start.clone()).expect("start snapshot");
        self.s = *snap;
        if let Some(p) = self.s.checkpoint_pos {
            self.s.player = Player::new(p);
            self.s.player.activated = true;
            self.s.player.yaw_deg = self.s.checkpoint_yaw;
        }
        self.s.hp = 100;
        self.s.dead = false;
        self.full_refresh = true;
        self.events.push(GameEvent::Respawned);
        self.sync_world();
    }

    /// Re-takes the level-start snapshot (after the caller customised the initial state).
    pub fn rebase_start(&mut self) {
        self.checkpoint = None;
        self.start = Some(Box::new(self.s.clone()));
    }

    pub fn restart_level(&mut self) {
        self.checkpoint = None;
        self.respawn();
    }

    // ---------------------------------------------------------------- frame stepping

    /// NewMovement.FixedUpdate: while windState > 0, whatever the player is about to
    /// sweep into this step breaks (weak Breakables, Glass on layers 8/24).
    fn wind_sweep(&mut self) {
        let p = &self.s.player;
        if p.wind_state <= 0.0 {
            return;
        }
        let dist = p.vel.length() * uk_core::consts::FIXED_DT;
        if dist <= 0.0 {
            return;
        }
        let cap = p.capsule();
        let dir = p.vel / p.vel.length();
        let mut hit_nodes = Vec::new();
        // Unity's SweepTest ignores colliders already in contact (contact offset 0.01),
        // so the floor under the player's feet must not count.
        for c in [cap.a, (cap.a + cap.b) * 0.5, cap.b] {
            if let Some(h) = self.world.sphere_cast(c, cap.radius - 0.03, dir, dist + 0.01) {
                let owner = self.world.owner(h.collider);
                if owner != uk_core::collide::ALWAYS {
                    let col = &self.def.colliders[owner as usize];
                    if col.layer == 8 || col.layer == 24 {
                        hit_nodes.push(col.node);
                    }
                }
            }
        }
        for n in hit_nodes {
            let scs: Vec<u32> = self.def.scripts_on(n).map(|(i, _)| i).collect();
            for sc in scs {
                match &self.s.scripts[sc as usize] {
                    Script::Breakable(b) if !b.precision_only => self.breakable_break(sc, 99999.0),
                    Script::Glass(_) => self.glass_shatter(sc),
                    _ => {}
                }
            }
        }
    }

    /// Unity FixedUpdate + physics at 125 Hz.
    pub fn fixed_update(&mut self, input: &Input) {
        if self.s.dead || self.s.level_complete {
            return;
        }
        self.sync_world();
        self.wind_sweep();
        let world = std::mem::take(&mut self.world);
        self.s.player.fixed_update(&world, input);
        self.world = world;
        self.update_triggers();
        self.update_contacts();
        for sc in 0..self.s.scripts.len() as u32 {
            if matches!(self.s.scripts[sc as usize], Script::Wave(_)) && self.script_live(sc) {
                self.wave_fixed(sc);
            }
        }
        enemy::fixed_update(self);
    }

    /// Unity Update (once per rendered frame).
    pub fn update(&mut self, input: &Input, dt: f32, now: f64) {
        self.s.time += dt as f64;
        if self.s.dead {
            self.s.dead_timer += dt;
            if self.s.dead_timer > 1.5 {
                self.respawn();
            }
            return;
        }
        self.run_starts();
        // timers
        let t = self.s.time;
        let mut due: Vec<Invoke> = Vec::new();
        self.s.invokes.retain(|i| {
            if i.at <= t {
                due.push(i.clone());
                false
            } else {
                true
            }
        });
        due.sort_by(|a, b| a.at.total_cmp(&b.at));
        for inv in due {
            match inv.act {
                Act::ObjActivate => self.obj_activate(inv.script, false),
                Act::ArenaSpawn => self.arena_spawn(inv.script),
                Act::WaveSpawn => self.wave_spawn(inv.script),
                Act::WaveEnd => self.wave_end(inv.script),
                Act::FinalDoorOpenDoors => self.final_door_open_doors(inv.script),
                Act::FinalDoorOpenerGoTime => {
                    if let Script::FinalDoorOpener { opened, opening, .. } = &mut self.s.scripts[inv.script as usize] {
                        *opening = false;
                        *opened = true;
                    }
                }
                Act::HideMessage => {}
            }
        }
        // per-script Update
        for sc in 0..self.s.scripts.len() as u32 {
            if !self.script_live(sc) {
                continue;
            }
            match &self.s.scripts[sc as usize] {
                Script::Door(_) => self.door_update(sc, dt),
                Script::DoorController(_) => self.door_controller_update(sc),
                Script::ObjectActivator(oa) => {
                    if let Some(o) = oa.obac {
                        let ready = self.obac_ready(o);
                        let Script::ObjectActivator(oa) = &mut self.s.scripts[sc as usize] else { continue };
                        if (oa.non_collider || oa.player_in > 0) && !oa.activating && !oa.activated && ready && !oa.only_check_obac_once {
                            oa.activating = true;
                            let d = oa.delay;
                            self.invoke(sc, Act::ObjActivate, d);
                        }
                        let Script::ObjectActivator(oa) = &self.s.scripts[sc as usize] else { continue };
                        if oa.disable_if_obac_off && oa.activated && !ready {
                            self.obj_deactivate(sc);
                        }
                    }
                }
                _ => {}
            }
        }
        self.s.messages.retain(|m| m.until.is_none_or(|u| u > t));
        self.sync_world();
        let world = std::mem::take(&mut self.world);
        self.s.player.update(&world, input, dt, now);
        self.world = world;
        enemy::update(self, dt);
        // Falling out of the world: treat like an out-of-bounds death.
        if self.s.player.pos.y < -1000.0 {
            self.hurt_player(999_999, false);
        }
        self.check_weapons();
    }

    /// Weapons: the real game unlocks them from save progress (GunSetter); here picking up
    /// a weapon pickup (the pickup object switching itself off) grants it.
    fn check_weapons(&mut self) {
        if self.s.has_revolver {
            return;
        }
        for &n in &self.changed_nodes {
            let name = &self.def.nodes[n as usize].name;
            if !self.s.active[n as usize] && name.starts_with("RevolverPickUp") {
                self.s.has_revolver = true;
                self.events.push(GameEvent::WeaponGot("REVOLVER"));
                break;
            }
        }
    }

    /// Frontend: take the list of nodes whose visibility changed.
    pub fn drain_changed(&mut self) -> (bool, Vec<u32>) {
        let full = std::mem::replace(&mut self.full_refresh, false);
        (full, std::mem::take(&mut self.changed_nodes))
    }
}

fn seg_point_dist(a: Vec3, b: Vec3, p: Vec3) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 { ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t).distance(p)
}

fn seg_seg_dist(a: Vec3, b: Vec3, c: Vec3, d: Vec3) -> f32 {
    // sample-based is plenty for triggers
    (0..=16).map(|i| seg_point_dist(c, d, a.lerp(b, i as f32 / 16.0))).fold(f32::MAX, f32::min)
}
