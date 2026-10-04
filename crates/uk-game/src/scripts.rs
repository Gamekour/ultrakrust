//! Runtime state for the MonoBehaviours 0-1 needs, parsed from their serialized
//! fields. Behaviour lives in `game.rs`; these are the data halves.

use bevy_math::Vec3;
use uk_assets::scenedef::SceneDef;
use uk_assets::serialized::Value;

/// Unity vector field -> Bevy space.
pub fn uvec(v: &Value) -> Vec3 {
    let a = v.vec3();
    Vec3::new(a[0], a[1], -a[2])
}

#[derive(Clone, Debug)]
pub enum Target {
    Node(u32),
    Script(u32),
    Collider(u32),
    None,
}

/// One persistent UnityEvent call.
#[derive(Clone, Debug)]
pub struct Call {
    pub target: Target,
    pub method: String,
    pub bool_arg: bool,
    pub float_arg: f32,
    pub int_arg: i64,
}

/// `UltrakillEvent`: object lists + UnityEvents.
#[derive(Clone, Debug, Default)]
pub struct UEvent {
    pub to_activate: Vec<u32>,
    pub to_deactivate: Vec<u32>,
    pub on_activate: Vec<Call>,
    pub on_deactivate: Vec<Call>,
}

pub fn parse_calls(def: &SceneDef, v: &Value) -> Vec<Call> {
    v.get("m_PersistentCalls")
        .get("m_Calls")
        .array()
        .iter()
        .map(|c| {
            let t = c.get("m_Target");
            let (f, id) = t.pptr();
            let target = if f != 0 || id == 0 {
                Target::None
            } else if let Some(&s) = def.comp_to_script.get(&id) {
                Target::Script(s)
            } else if let Some(&col) = def.comp_to_collider.get(&id) {
                Target::Collider(col)
            } else if let Some(n) = def.node_ref(t) {
                Target::Node(n)
            } else {
                Target::None
            };
            let a = c.get("m_Arguments");
            Call {
                target,
                method: c.get("m_MethodName").str().to_string(),
                bool_arg: a.get("m_BoolArgument").bool(),
                float_arg: a.get("m_FloatArgument").f32(),
                int_arg: a.get("m_IntArgument").i64(),
            }
        })
        .collect()
}

pub fn nodes(def: &SceneDef, v: &Value) -> Vec<u32> {
    v.array().iter().filter_map(|p| def.node_ref(p)).collect()
}

pub fn scripts(def: &SceneDef, v: &Value) -> Vec<u32> {
    v.array().iter().filter_map(|p| def.script_ref(p)).collect()
}

pub fn parse_uevent(def: &SceneDef, v: &Value) -> UEvent {
    UEvent {
        to_activate: nodes(def, v.get("toActivateObjects")),
        to_deactivate: nodes(def, v.get("toDisActivateObjects")),
        on_activate: parse_calls(def, v.get("onActivate")),
        on_deactivate: parse_calls(def, v.get("onDisActivate")),
    }
}

#[derive(Clone, Debug)]
pub struct ObjectActivator {
    pub one_time: bool,
    pub disable_on_exit: bool,
    pub dont_activate_on_enable: bool,
    pub reactivate_on_enable: bool,
    pub activate_on_disable: bool,
    pub for_enemies: bool,
    pub on_awake: bool,
    pub delay: f32,
    pub obac: Option<u32>,
    pub only_check_obac_once: bool,
    pub disable_if_obac_off: bool,
    pub events: UEvent,
    // runtime
    pub activated: bool,
    pub activating: bool,
    pub non_collider: bool,
    pub player_in: i32,
}

#[derive(Clone, Debug)]
pub struct Door {
    pub start_open: bool,
    pub open: bool,
    pub locked: bool,
    pub speed: f32,
    pub ease_in: bool,
    pub open_offset: Vec3,
    pub open_on_unlock: bool,
    pub dont_close_when_another_opens: bool,
    pub dont_close_others: bool,
    pub no_pass: Option<u32>,
    pub activated_rooms: Vec<u32>,
    pub deactivated_rooms: Vec<u32>,
    pub on_fully_opened: Vec<Call>,
    pub on_fully_closed: Vec<Call>,
    // runtime
    pub got_pos: bool,
    pub got_values: bool,
    pub closed_pos: Vec3,
    pub open_pos: Vec3,
    pub target_pos: Vec3,
    pub in_pos: bool,
    pub requests: i32,
    pub docons: Vec<u32>,
    pub doconless_col: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct DoorController {
    pub kind: i64,
    pub door: Option<u32>,
    pub open: bool,
    pub player_in: bool,
    pub destroyed: bool,
}

#[derive(Clone, Debug)]
pub struct Arena {
    pub only_wave: bool,
    pub doors: Vec<u32>,
    pub enemies: Vec<u32>,
    pub activate_on_enable: bool,
    pub for_enemy: bool,
    pub activated: bool,
    pub current: usize,
    pub destroyed: bool,
}

#[derive(Clone, Debug)]
pub struct Wave {
    pub last_wave: bool,
    pub enemy_count: i32,
    pub dead: i32,
    pub next_enemies: Vec<u32>,
    pub doors: Vec<u32>,
    pub to_activate: Vec<u32>,
    pub door_forward: Option<u32>,
    pub no_activation_delay: bool,
    pub activated: bool,
    pub current_enemy: usize,
    pub current_door: usize,
    pub objects_activated: bool,
    pub destroyed: bool,
}

#[derive(Clone, Debug)]
pub struct Breakable {
    pub durability: f32,
    pub unbreakable: bool,
    pub weak: bool,
    pub precision_only: bool,
    pub activate_on_break: Vec<u32>,
    pub destroy_on_break: Vec<u32>,
    pub destroy_event: UEvent,
    pub broken: bool,
}

#[derive(Clone, Debug)]
pub struct Glass {
    pub broken: bool,
    pub on_shatter: UEvent,
}

#[derive(Clone, Debug)]
pub struct CheckPoint {
    pub to_activate: Option<u32>,
    pub graphic: Option<u32>,
    pub doors_to_unlock: Vec<u32>,
    pub activated: bool,
}

#[derive(Clone, Debug)]
pub struct DeathZone {
    pub not_instakill: bool,
    pub damage: i32,
    pub player_affected: bool,
    pub on_hit_player: UEvent,
    pub disabled: bool,
}

#[derive(Clone, Debug)]
pub struct Teleport {
    pub affect_position: bool,
    pub not_relative: bool,
    pub relative: Vec3,
    pub objective: Vec3,
    pub reset_speed: bool,
    pub on_teleport: UEvent,
}

#[derive(Clone, Debug)]
pub struct FinalDoor {
    pub doors: Vec<u32>,
    pub door_light: Option<u32>,
    pub start_open: bool,
    pub closing_blocker: Option<u32>,
    pub opened: bool,
    pub about_to_open: bool,
}

#[derive(Clone, Debug)]
pub struct HudMessage {
    pub message: String,
    pub deactivating: bool,
    pub not_one_time: bool,
    pub timed: bool,
    pub timer: f32,
    pub dont_on_trigger: bool,
    pub deactivate_on_exit: bool,
    pub shown: bool,
}

#[derive(Clone, Debug)]
pub enum Script {
    ObjectActivator(Box<ObjectActivator>),
    ObjectActivationCheck { ready: bool },
    Door(Box<Door>),
    DoorController(DoorController),
    DoorOpener { door: Option<u32>, one_time: bool, done: bool },
    Arena(Arena),
    Wave(Wave),
    Breakable(Box<Breakable>),
    Glass(Box<Glass>),
    CheckPoint(CheckPoint),
    DeathZone(Box<DeathZone>),
    Teleport(Box<Teleport>),
    PlayerActivator { activated: bool, only_player: bool },
    FinalDoor(FinalDoor),
    FinalDoorOpener { opened: bool, opening: bool, closed: bool },
    FinalPit,
    HudMessage(Box<HudMessage>),
    /// Index into `State::enemies`.
    Enemy(usize),
    WeaponPickUp,
    Other,
}

pub fn parse(def: &SceneDef, idx: usize) -> Script {
    let s = &def.scripts[idx];
    let v = &s.data;
    let b = |k: &str| v.get(k).bool();
    let f = |k: &str| v.get(k).f32();
    match s.class.as_str() {
        "ObjectActivator" => Script::ObjectActivator(Box::new(ObjectActivator {
            one_time: b("oneTime"),
            disable_on_exit: b("disableOnExit"),
            dont_activate_on_enable: b("dontActivateOnEnable"),
            reactivate_on_enable: b("reactivateOnEnable"),
            activate_on_disable: b("activateOnDisable"),
            for_enemies: b("forEnemies"),
            on_awake: b("onAwake"),
            delay: f("delay"),
            obac: def.script_ref(v.get("obac")),
            only_check_obac_once: b("onlyCheckObacOnce"),
            disable_if_obac_off: b("disableIfObacOff"),
            events: parse_uevent(def, v.get("events")),
            activated: false,
            activating: false,
            non_collider: false,
            player_in: 0,
        })),
        "ObjectActivationCheck" => Script::ObjectActivationCheck { ready: b("readyToActivate") },
        "Door" => Script::Door(Box::new(Door {
            start_open: b("startOpen"),
            open: b("open"),
            locked: b("locked"),
            speed: f("speed"),
            ease_in: b("easeIn"),
            open_offset: uvec(v.get("openPos")),
            open_on_unlock: b("openOnUnlock"),
            dont_close_when_another_opens: b("dontCloseWhenAnotherDoorOpens"),
            dont_close_others: b("dontCloseOtherDoorsWhenOpening"),
            no_pass: def.node_ref(v.get("noPass")),
            activated_rooms: nodes(def, v.get("activatedRooms")),
            deactivated_rooms: nodes(def, v.get("deactivatedRooms")),
            on_fully_opened: parse_calls(def, v.get("onFullyOpened")),
            on_fully_closed: parse_calls(def, v.get("onFullyClosed")),
            got_pos: false,
            got_values: false,
            closed_pos: Vec3::ZERO,
            open_pos: Vec3::ZERO,
            target_pos: Vec3::ZERO,
            in_pos: true,
            requests: v.get("requests").i64() as i32,
            docons: Vec::new(),
            doconless_col: None,
        })),
        "DoorController" => {
            Script::DoorController(DoorController { kind: v.get("type").i64(), door: None, open: false, player_in: false, destroyed: false })
        }
        "DoorOpener" => Script::DoorOpener { door: def.script_ref(v.get("door")), one_time: b("oneTime"), done: false },
        "ActivateArena" => Script::Arena(Arena {
            only_wave: b("onlyWave"),
            doors: scripts(def, v.get("doors")),
            enemies: nodes(def, v.get("enemies")),
            activate_on_enable: b("activateOnEnable"),
            for_enemy: b("forEnemy"),
            activated: false,
            current: 0,
            destroyed: false,
        }),
        "ActivateNextWave" => Script::Wave(Wave {
            last_wave: b("lastWave"),
            enemy_count: v.get("enemyCount").i64() as i32,
            dead: v.get("deadEnemies").i64() as i32,
            next_enemies: nodes(def, v.get("nextEnemies")),
            doors: scripts(def, v.get("doors")),
            to_activate: nodes(def, v.get("toActivate")),
            door_forward: def.script_ref(v.get("doorForward")),
            no_activation_delay: b("noActivationDelay"),
            activated: false,
            current_enemy: 0,
            current_door: 0,
            objects_activated: false,
            destroyed: false,
        }),
        "Breakable" => Script::Breakable(Box::new(Breakable {
            durability: f("durability"),
            unbreakable: b("unbreakable"),
            weak: b("weak"),
            precision_only: b("precisionOnly"),
            activate_on_break: nodes(def, v.get("activateOnBreak")),
            destroy_on_break: nodes(def, v.get("destroyOnBreak")),
            destroy_event: parse_uevent(def, v.get("destroyEvent")),
            broken: false,
        })),
        "Glass" => Script::Glass(Box::new(Glass { broken: false, on_shatter: parse_uevent(def, v.get("onShatter")) })),
        "CheckPoint" => Script::CheckPoint(CheckPoint {
            to_activate: def.node_ref(v.get("toActivate")),
            graphic: def.node_ref(v.get("graphic")),
            doors_to_unlock: scripts(def, v.get("doorsToUnlock")),
            activated: false,
        }),
        "DeathZone" => Script::DeathZone(Box::new(DeathZone {
            not_instakill: b("notInstakill"),
            damage: v.get("damage").i64() as i32,
            // AffectedSubjects: 0 All, 1 EnemiesOnly, 2 PlayerOnly
            player_affected: v.get("affected").i64() != 1,
            on_hit_player: parse_uevent(def, v.get("onHitPlayer")),
            disabled: false,
        })),
        "TeleportPlayer" => Script::Teleport(Box::new(Teleport {
            affect_position: b("affectPosition"),
            not_relative: b("notRelative"),
            relative: uvec(v.get("relativePosition")),
            objective: uvec(v.get("objectivePosition")),
            reset_speed: b("resetPlayerSpeed"),
            on_teleport: parse_uevent(def, v.get("onTeleportPlayer")),
        })),
        "PlayerActivator" => Script::PlayerActivator { activated: false, only_player: b("onlyActivatePlayer") },
        "FinalDoor" => Script::FinalDoor(FinalDoor {
            doors: scripts(def, v.get("doors")),
            door_light: def.node_ref(v.get("doorLight")),
            start_open: b("startOpen"),
            closing_blocker: def.node_ref(v.get("closingBlocker")),
            opened: false,
            about_to_open: false,
        }),
        "FinalDoorOpener" => Script::FinalDoorOpener { opened: false, opening: false, closed: false },
        "FinalPit" => Script::FinalPit,
        "HudMessage" => Script::HudMessage(Box::new(HudMessage {
            message: clean_rich_text(v.get("message").str()),
            deactivating: b("deactivating"),
            not_one_time: b("notOneTime"),
            timed: b("timed"),
            timer: f("timerTime"),
            dont_on_trigger: b("dontActivateOnTriggerEnter"),
            deactivate_on_exit: b("deactiveOnTriggerExit"),
            shown: false,
        })),
        "WeaponPickUp" => Script::WeaponPickUp,
        _ => Script::Other,
    }
}

/// Strips TextMeshPro tags; `$` is ULTRAKILL's line break in hints.
pub fn clean_rich_text(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            '$' if !in_tag => out.push('\n'),
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}
