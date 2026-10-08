//! Ports of ULTRAKILL's HUD MonoBehaviours driving the scene's own uGUI (ugui.rs):
//! PlayerActivatorRelay, HudOpenEffect, HealthBar, StaminaMeter, ColorBlindSettings/ColorBlindGet,
//! HudController, HUDPos, and NewMovement's HUD sway. They write `State::ui` (slider values,
//! Graphic colours, TMP text, RectTransform overrides) and node activity like the originals.

use crate::game::{Act, Game};
use crate::scripts::Script;
use crate::ugui::UiDef;
use bevy_math::{EulerRot, Mat4, Quat, Vec2, Vec3};
use std::sync::Arc;
use uk_assets::prefs::Prefs;
use uk_assets::scenedef::SceneDef;
use uk_assets::serialized::Value;
use uk_core::umath::{move_towards, vmove_towards};

pub type Color = [f32; 4];

pub const RED: Color = [1.0, 0.0, 0.0, 1.0];
/// Color.yellow
pub const YELLOW: Color = [1.0, 0.921_568_6, 0.015_686_275, 1.0];
pub const WHITE: Color = [1.0; 4];

pub fn color(v: &Value) -> Color {
    [v.get("r").f32(), v.get("g").f32(), v.get("b").f32(), v.get("a").f32()]
}

/// Color == Color (Vector4 equality: squared distance below 1e-5²)
pub fn color_eq(a: Color, b: Color) -> bool {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f32>() < 9.999_999_4e-11
}

/// HudColorType, in enum order
pub mod hct {
    pub const HEALTH: usize = 0;
    pub const HEALTH_AFTER_IMAGE: usize = 1;
    pub const ANTI_HP: usize = 2;
    pub const OVERHEAL: usize = 3;
    pub const HEALTH_TEXT: usize = 4;
    pub const STAMINA: usize = 5;
    pub const STAMINA_CHARGING: usize = 6;
    pub const STAMINA_EMPTY: usize = 7;
    pub const RAILCANNON_FULL: usize = 8;
    pub const RAILCANNON_CHARGING: usize = 9;
}

const HUD_COLOR_FIELDS: [&str; 10] = [
    "healthBarColor",
    "healthBarAfterImageColor",
    "antiHpColor",
    "overHealColor",
    "healthBarTextColor",
    "staminaColor",
    "staminaChargingColor",
    "staminaEmptyColor",
    "railcannonFullColor",
    "railcannonChargingColor",
];

/// ColorBlindSettings (the scene's singleton) with the player's `hudColor.*` prefs applied the way
/// ColorBlindActivator.Start -> ColorBlindSetter.Prepare does (enemy colours are not read here).
#[derive(Clone, Debug)]
pub struct ColorBlind {
    pub variation: Vec<Color>,
    pub hud: [Color; 10],
}

impl ColorBlind {
    pub fn build(def: &SceneDef, prefs: &Prefs) -> ColorBlind {
        let mut cb = ColorBlind { variation: Vec::new(), hud: [WHITE; 10] };
        let Some(s) = def.scripts.iter().find(|s| s.class == "ColorBlindSettings") else { return cb };
        cb.variation = s.data.get("variationColors").array().iter().map(color).collect();
        for (i, f) in HUD_COLOR_FIELDS.iter().enumerate() {
            cb.hud[i] = color(s.data.get(f));
        }
        // ColorBlindActivator.Start: GetComponentsInChildren<ColorBlindSetter>(includeInactive) -> Prepare
        for a in def.scripts.iter().filter(|s| s.class == "ColorBlindActivator" && s.enabled) {
            for st in def.scripts.iter().filter(|s| s.class == "ColorBlindSetter" && def.is_descendant(s.node, a.node)) {
                let d = &st.data;
                if d.get("enemyColor").bool() {
                    continue;
                }
                let variation = d.get("variationColor").bool();
                let slot = if variation { d.get("variationNumber").i64() as usize } else { d.get("hct").i64() as usize };
                let Some(orig) = (if variation { cb.variation.get(slot).copied() } else { cb.hud.get(slot).copied() }) else { continue };
                let name = &def.nodes[st.node as usize].name;
                let key = |c: &str, o: f32| prefs.float_or(&format!("hudColor.{name}.{c}"), o);
                let new = [key("r", orig[0]), key("g", orig[1]), key("b", orig[2]), orig[3]];
                if new != orig {
                    if variation {
                        cb.variation[slot] = new;
                    } else {
                        cb.hud[slot] = new;
                    }
                }
            }
        }
        cb
    }

    pub fn hud(&self, hct: usize) -> Color {
        self.hud.get(hct).copied().unwrap_or(WHITE)
    }
}

/// `float.ToString(format)` for the fixed formats the HUD uses ("F0", "0.00"): Mono formats a
/// float from its 7 significant digits, then rounds half away from zero.
/// LevelStats' time: `minutes + ":" + seconds.ToString("00.000")` after subtracting whole minutes
/// in f32 steps.
pub fn stats_time_text(total: f32) -> String {
    let mut seconds = total;
    let mut minutes = 0.0f32;
    while seconds >= 60.0 {
        seconds -= 60.0;
        minutes += 1.0;
    }
    let s = net_fixed(seconds, 3);
    let pad = if s.find('.').unwrap_or(s.len()) < 2 { "0" } else { "" };
    format!("{}:{pad}{s}", minutes as i64)
}

pub fn net_fixed(x: f32, decimals: usize) -> String {
    let s = format!("{:.6e}", x.abs());
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let digits: Vec<u8> = mant.bytes().filter(|b| b.is_ascii_digit()).map(|b| b - b'0').collect();
    // value = 0.d1d2..d7 * 10^(exp+1); keep the digits up to `decimals` places
    let int_len = exp + 1;
    let keep = int_len + decimals as i32;
    let mut out: Vec<u8> = if keep <= 0 { Vec::new() } else { digits.iter().copied().take(keep as usize).collect() };
    while (out.len() as i32) < keep {
        out.push(0);
    }
    let next = if keep < 0 { 0 } else { digits.get(keep as usize).copied().unwrap_or(0) };
    if next >= 5 {
        let mut i = out.len();
        loop {
            if i == 0 {
                out.insert(0, 1);
                break;
            }
            i -= 1;
            if out[i] == 9 {
                out[i] = 0;
            } else {
                out[i] += 1;
                break;
            }
        }
    }
    let total = (keep.max(0) as usize).max(out.len());
    let int_digits = total.saturating_sub(decimals);
    let mut r = String::new();
    if x < 0.0 && out.iter().any(|&d| d != 0) {
        r.push('-');
    }
    if int_digits == 0 {
        r.push('0');
    }
    let pad = decimals + int_digits - out.len().min(decimals + int_digits);
    let all: Vec<u8> = std::iter::repeat_n(0, pad).chain(out).collect();
    for (i, d) in all.iter().enumerate() {
        if i == int_digits && decimals > 0 {
            if int_digits == 0 {
                // "0" already pushed
            }
            r.push('.');
        }
        r.push((b'0' + d) as char);
    }
    if decimals > 0 && all.len() == int_digits {
        r.push('.');
        r.extend(std::iter::repeat_n('0', decimals));
    }
    r
}

#[derive(Clone, Debug)]
pub struct Relay {
    pub to_activate: Vec<Option<u32>>,
    pub gun_panel: Option<u32>,
    pub crosshair: Option<u32>,
    pub delay: f32,
    pub index: usize,
}

#[derive(Clone, Debug)]
pub struct OpenEffect {
    pub speed: f32,
    pub original_speed: f32,
    pub skip: bool,
    pub reverse: bool,
    pub y_first: bool,
    pub dont_use_scale: bool,
    pub got_values: bool,
    pub original: Vec2,
    pub target: Vec2,
    pub animating: bool,
}

#[derive(Clone, Debug)]
pub struct HealthBar {
    /// Slider scripts
    pub hp_sliders: Vec<u32>,
    pub after_image: Vec<u32>,
    pub anti_slider: Option<u32>,
    /// Image script
    pub anti_fill: Option<u32>,
    /// TMP_Text script
    pub text: Option<u32>,
    pub change_text_color: bool,
    pub normal_text_color: Color,
    pub yellow_color: bool,
    pub anti_hp_text: bool,
    pub hp: f32,
    pub anti_hp: f32,
    pub last_hp: f32,
    pub last_anti_hp: f32,
    pub difficulty: i32,
}

#[derive(Clone, Debug)]
pub struct Stamina {
    pub change_text_color: bool,
    pub normal_text_color: Color,
    pub red_empty: bool,
    pub always_update: bool,
    /// Slider script on this object (GetComponent<Slider>)
    pub slider: Option<u32>,
    /// TMP_Text graphic on this object
    pub text: Option<u32>,
    /// staminaBar / staminaFlash graphics (child 1/0 and its child 0)
    pub bar: Option<u32>,
    pub flash: Option<u32>,
    /// GetComponentInParent<Canvas> (canvas index)
    pub parent_canvas: Option<u32>,
    pub stamina: f32,
    pub full: bool,
    pub flash_color: Color,
    pub empty_color: Color,
    pub orig_color: Color,
    pub intro: bool,
    pub last_stamina: f32,
}

#[derive(Clone, Debug)]
pub struct Controller {
    pub alt_hud: bool,
    pub colorless: bool,
    pub alt_hud_obj: Option<u32>,
    pub gun_canvas: Option<u32>,
    /// HUDPos script on the gun canvas
    pub hud_pos: Option<u32>,
    pub weapon_icon: Option<u32>,
    pub arm_icon: Option<u32>,
    pub style_meter: Option<u32>,
    pub style_info: Option<u32>,
    /// Speedometer script
    pub speedometer: Option<u32>,
    /// Image scripts
    pub backgrounds: Vec<u32>,
    pub text_elements: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct HudPos {
    pub active: bool,
    pub ready: bool,
    pub rect_transform: bool,
    pub reverse_pos: Vec3,
    pub reverse_rot: Vec3,
    pub anchors_max: [f32; 2],
    pub anchors_min: [f32; 2],
    pub pivot: [f32; 2],
    pub anchored_position: [f32; 2],
    /// defaults captured on the first CheckPos: rect (anchorMin, anchorMax, pivot, anchoredPosition)
    /// or Unity-space (localPosition, localRotation)
    pub default_rect: Option<([f32; 2], [f32; 2], [f32; 2], [f32; 2])>,
    pub default_tr: Option<(Vec3, Quat)>,
}

#[derive(Clone, Debug)]
pub enum HudScript {
    Relay(Box<Relay>),
    OpenEffect(Box<OpenEffect>),
    HealthBar(Box<HealthBar>),
    Stamina(Box<Stamina>),
    /// ColorBlindGet: (HudColorType, variation colour index)
    ColorGet { hct: usize, variation: Option<usize> },
    Controller(Box<Controller>),
    Pos(Box<HudPos>),
    LevelStatsEnabler(Box<LevelStatsEnabler>),
    LevelStats(Box<LevelStats>),
    /// StyleHUD: only the meter's visibility (no style points are scored, so comboActive stays false)
    Style { force_meter_on: bool },
    Railcannon(Box<RailcannonMeter>),
    Crosshair(Box<Crosshair>),
    FadeOutBars(Box<FadeOutBars>),
    SliderFill(Box<SliderFill>),
    /// Speedometer: only OnEnable (shown when the `speedometer` pref is above 0, its default 0
    /// hiding it); the readout itself (FixedUpdate) isn't ported
    Speedometer,
}

/// Crosshair (Image refs are script indices; circles are sprite indices, filled in at set_ui).
#[derive(Clone, Debug)]
pub struct Crosshair {
    pub altchs: Vec<Option<u32>>,
    pub chuds: Vec<Option<u32>>,
    pub circles: Vec<Option<u32>>,
}

/// FadeOutBars: the crosshair HUD bars fade out `fadeOutTime` seconds after their last change.
#[derive(Clone, Debug)]
pub struct FadeOutBars {
    pub fade_out: bool,
    pub fade_out_time: f32,
}

/// SliderToFillAmount: an Image's fillAmount (and colour) follows a Slider.
#[derive(Clone, Debug)]
pub struct SliderFill {
    /// Slider script
    pub target: Option<u32>,
    pub max_fill: f32,
    pub copy_color: bool,
    /// FadeOutBars script (FadeOutBars.Start assigns it to every SliderToFillAmount below it)
    pub mama: Option<u32>,
    pub is_invisible: bool,
    pub last_invisible: bool,
}

/// RailcannonMeter (Image refs are script indices). WeaponCharges.raicharge is 0 until the railcannon is ported.
#[derive(Clone, Debug)]
pub struct RailcannonMeter {
    pub background: Option<u32>,
    /// meters + colorlessMeter (trueMeters)
    pub true_meters: Vec<Option<u32>>,
    pub colorless: Option<u32>,
    pub alt_hud_panels: Vec<Option<u32>>,
    pub mini: Option<u32>,
    pub flash: f32,
    pub has_flashed: bool,
}

/// LevelStats: the replay stats panel (TMP_Text / Image refs are script indices).
#[derive(Clone, Debug)]
pub struct LevelStats {
    pub cyber_grind: bool,
    pub secret_level: bool,
    pub level_name: Option<u32>,
    pub time: Option<u32>,
    pub time_rank: Option<u32>,
    pub kills: Option<u32>,
    pub kills_rank: Option<u32>,
    pub style: Option<u32>,
    pub style_rank: Option<u32>,
    pub secrets: Vec<Option<u32>>,
    pub challenge: Option<u32>,
    pub major_assists: Option<u32>,
    pub ready: bool,
    pub check_secrets: bool,
}

#[derive(Clone, Debug)]
pub struct LevelStatsEnabler {
    pub secret_level: i64,
    pub can_always_enable: bool,
    pub level_stats: Option<u32>,
    pub keep_open: bool,
    pub double_tap: f32,
}

/// The frontend's UI input this frame (InputManager actions the HUD scripts read).
#[derive(Clone, Copy, Debug, Default)]
pub struct HudInput {
    /// Stats (Tab): WasPerformedThisFrame / WasCanceledThisFrame
    pub stats_performed: bool,
    pub stats_canceled: bool,
}

fn v2(v: &Value) -> [f32; 2] {
    [v.get("x").f32(), v.get("y").f32()]
}

fn v3u(v: &Value) -> Vec3 {
    Vec3::from_array(v.vec3())
}

pub fn parse(def: &SceneDef, idx: usize) -> Option<HudScript> {
    let s = &def.scripts[idx];
    let v = &s.data;
    let b = |k: &str| v.get(k).bool();
    let scripts = |k: &str| v.get(k).array().iter().filter_map(|p| def.script_ref(p)).collect::<Vec<u32>>();
    Some(match s.class.as_str() {
        "PlayerActivatorRelay" => HudScript::Relay(Box::new(Relay {
            to_activate: v.get("toActivate").array().iter().map(|p| def.node_ref(p)).collect(),
            gun_panel: def.node_ref(v.get("gunPanel")),
            crosshair: def.node_ref(v.get("crosshair")),
            delay: v.get("delay").f32(),
            index: 0,
        })),
        "HudOpenEffect" => HudScript::OpenEffect(Box::new(OpenEffect {
            speed: v.get("speed").f32(),
            original_speed: v.get("originalSpeed").f32(),
            skip: b("skip"),
            reverse: b("reverse"),
            y_first: b("YFirst"),
            dont_use_scale: b("dontUseScale"),
            got_values: b("gotValues"),
            original: Vec2::from_array(v2(v.get("originalDimensions"))),
            target: Vec2::from_array(v2(v.get("targetDimensions"))),
            animating: b("animating"),
        })),
        "HealthBar" => HudScript::HealthBar(Box::new(HealthBar {
            hp_sliders: scripts("hpSliders"),
            after_image: scripts("afterImageSliders"),
            anti_slider: def.script_ref(v.get("antiHpSlider")),
            anti_fill: def.script_ref(v.get("antiHpSliderFill")),
            text: def.script_ref(v.get("hpText")),
            change_text_color: b("changeTextColor"),
            normal_text_color: color(v.get("normalTextColor")),
            yellow_color: b("yellowColor"),
            anti_hp_text: b("antiHpText"),
            hp: 0.0,
            anti_hp: 0.0,
            last_hp: 0.0,
            last_anti_hp: 0.0,
            difficulty: 0,
        })),
        "StaminaMeter" => HudScript::Stamina(Box::new(Stamina {
            change_text_color: b("changeTextColor"),
            normal_text_color: color(v.get("normalTextColor")),
            red_empty: b("redEmpty"),
            always_update: b("alwaysUpdate"),
            slider: None,
            text: None,
            bar: None,
            flash: None,
            parent_canvas: None,
            stamina: 0.0,
            full: true,
            flash_color: [0.0; 4],
            empty_color: [0.0; 4],
            orig_color: [0.0; 4],
            intro: true,
            last_stamina: 0.0,
        })),
        "ColorBlindGet" => HudScript::ColorGet {
            hct: v.get("hct").i64() as usize,
            variation: b("variationColor").then(|| v.get("variationNumber").i64() as usize),
        },
        "HudController" => HudScript::Controller(Box::new(Controller {
            alt_hud: b("altHud"),
            colorless: b("colorless"),
            alt_hud_obj: None,
            gun_canvas: def.node_ref(v.get("gunCanvas")),
            hud_pos: None,
            weapon_icon: def.node_ref(v.get("weaponIcon")),
            arm_icon: def.node_ref(v.get("armIcon")),
            style_meter: def.node_ref(v.get("styleMeter")),
            style_info: def.node_ref(v.get("styleInfo")),
            speedometer: def.script_ref(v.get("speedometer")),
            backgrounds: scripts("hudBackgrounds"),
            text_elements: scripts("textElements"),
        })),
        "HUDPos" => HudScript::Pos(Box::new(HudPos {
            active: b("active"),
            ready: false,
            rect_transform: b("rectTransform"),
            reverse_pos: v3u(v.get("reversePos")),
            reverse_rot: v3u(v.get("reverseRot")),
            anchors_max: v2(v.get("anchorsMax")),
            anchors_min: v2(v.get("anchorsMin")),
            pivot: v2(v.get("pivot")),
            anchored_position: v2(v.get("anchoredPosition")),
            default_rect: None,
            default_tr: None,
        })),
        "LevelStats" => HudScript::LevelStats(Box::new(LevelStats {
            cyber_grind: b("cyberGrind"),
            secret_level: b("secretLevel"),
            level_name: def.script_ref(v.get("levelName")),
            time: def.script_ref(v.get("time")),
            time_rank: def.script_ref(v.get("timeRank")),
            kills: def.script_ref(v.get("kills")),
            kills_rank: def.script_ref(v.get("killsRank")),
            style: def.script_ref(v.get("style")),
            style_rank: def.script_ref(v.get("styleRank")),
            secrets: v.get("secrets").array().iter().map(|p| def.script_ref(p)).collect(),
            challenge: def.script_ref(v.get("challenge")),
            major_assists: def.script_ref(v.get("majorAssists")),
            ready: false,
            check_secrets: true,
        })),
        "LevelStatsEnabler" => HudScript::LevelStatsEnabler(Box::new(LevelStatsEnabler {
            secret_level: v.get("secretLevel").i64(),
            can_always_enable: b("canAlwaysEnable"),
            level_stats: None,
            keep_open: false,
            double_tap: 0.0,
        })),
        "RailcannonMeter" => HudScript::Railcannon(Box::new(RailcannonMeter {
            background: def.script_ref(v.get("meterBackground")),
            true_meters: v.get("meters").array().iter().map(|p| def.script_ref(p)).chain([def.script_ref(v.get("colorlessMeter"))]).collect(),
            colorless: def.script_ref(v.get("colorlessMeter")),
            alt_hud_panels: v.get("altHudPanels").array().iter().map(|p| def.node_ref(p)).collect(),
            mini: def.node_ref(v.get("miniVersion")),
            flash: 0.0,
            has_flashed: false,
        })),
        "StyleHUD" => HudScript::Style { force_meter_on: b("forceMeterOn") },
        "Crosshair" => HudScript::Crosshair(Box::new(Crosshair {
            altchs: v.get("altchs").array().iter().map(|p| def.script_ref(p)).collect(),
            chuds: v.get("chuds").array().iter().map(|p| def.script_ref(p)).collect(),
            circles: Vec::new(),
        })),
        "Speedometer" => HudScript::Speedometer,
        "FadeOutBars" => HudScript::FadeOutBars(Box::new(FadeOutBars { fade_out: false, fade_out_time: v.get("fadeOutTime").f32() })),
        "SliderToFillAmount" => HudScript::SliderFill(Box::new(SliderFill {
            target: def.script_ref(v.get("targetSlider")),
            max_fill: v.get("maxFill").f32(),
            copy_color: b("copyColor"),
            mama: def.script_ref(v.get("mama")),
            is_invisible: false,
            last_invisible: false,
        })),
        _ => return None,
    })
}

/// Unity-space localPosition / localRotation of a node at load.
fn rest_local(def: &SceneDef, n: u32) -> (Vec3, Quat) {
    let nd = &def.nodes[n as usize];
    let p = nd.local_pos;
    let q = nd.local_rot;
    (Vec3::new(p.x, p.y, -p.z), Quat::from_xyzw(-q.x, -q.y, q.z, q.w))
}

/// NewMovement's HUD sway targets: screenHud (the HUD root) and hudCam, with their Start positions.
#[derive(Clone, Copy, Debug)]
pub struct Sway {
    pub screen_hud: u32,
    pub hud_cam: u32,
    pub hud_original: Vec3,
    pub cam_original: Vec3,
}

impl Sway {
    pub fn from_def(def: &SceneDef) -> Option<Sway> {
        let nm = def.scripts.iter().find(|s| s.class == "NewMovement")?;
        let screen_hud = def.node_ref(nm.data.get("screenHud"))?;
        let hud_cam = def.node_ref(nm.data.get("hudCam"))?;
        Some(Sway { screen_hud, hud_cam, hud_original: rest_local(def, screen_hud).0, cam_original: rest_local(def, hud_cam).0 })
    }
}

impl Game {
    fn ui_def(&self) -> Option<Arc<UiDef>> {
        self.ui.clone()
    }

    fn hud(&mut self, sc: u32) -> Option<&mut HudScript> {
        match &mut self.s.scripts[sc as usize] {
            Script::Hud(h) => Some(h),
            _ => None,
        }
    }

    /// Current world matrix (Unity space) of a node, with `UiState::local` overrides on it or its
    /// ancestors applied on top of the animated pose.
    pub fn node_world(&self, n: u32) -> Mat4 {
        let local = &self.s.ui.local;
        if local.is_empty() {
            return self.anim.world_of(n);
        }
        let mut chain = Vec::new();
        let mut m = Some(n);
        while let Some(x) = m {
            if local.contains_key(&x) {
                chain.push(x);
            }
            m = self.def.nodes[x as usize].parent;
        }
        let mut c = Mat4::IDENTITY;
        for &x in chain.iter().rev() {
            let wp = match self.def.nodes[x as usize].parent {
                Some(p) => c * self.anim.world_of(p),
                None => Mat4::IDENTITY,
            };
            let (pos, rot) = local[&x];
            let wx = wp * Mat4::from_scale_rotation_translation(self.def.nodes[x as usize].local_scale, rot, pos);
            c = wx * self.anim.world_of(x).inverse();
        }
        c * self.anim.world_of(n)
    }

    /// `node_world` in Bevy space.
    pub fn node_world_bevy(&self, n: u32) -> Mat4 {
        let s = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
        s * self.node_world(n) * s
    }

    fn local_tr(&self, n: u32) -> (Vec3, Quat) {
        self.s.ui.local.get(&n).copied().unwrap_or_else(|| rest_local(&self.def, n))
    }

    fn set_local_pos(&mut self, n: u32, p: Vec3) {
        let (_, r) = self.local_tr(n);
        self.s.ui.local.insert(n, (p, r));
    }

    /// Transform.localPosition.z of a RectTransform inside a canvas, or of a node placed with `local`.
    fn set_local_z(&mut self, n: u32, z: f32) {
        if self.ui.as_ref().is_some_and(|u| u.node_canvas.get(&n).is_some_and(|&c| u.canvases[c as usize].node == n)) || self.def.nodes[n as usize].rect.is_none() {
            let (mut p, _) = self.local_tr(n);
            p.z = z;
            self.set_local_pos(n, p);
        } else {
            self.s.ui.z.insert(n, z);
        }
    }

    fn graphic_of_script(&self, ui: &UiDef, sc: u32) -> Option<u32> {
        ui.script_graphic.get(&sc).copied()
    }

    fn set_graphic_color(&mut self, g: u32, c: Color) {
        self.s.ui.color[g as usize] = c;
    }

    fn slider_value(&self, ui: &UiDef, sc: u32) -> Option<(u32, f32)> {
        let i = *ui.script_slider.get(&sc)?;
        Some((i, self.s.ui.slider[i as usize]))
    }

    /// Slider.value = v
    fn set_slider_value(&mut self, ui: &UiDef, i: u32, v: f32) {
        let def = self.def.clone();
        self.s.ui.set_slider(&def, ui, i, v);
    }

    fn rect_of(&self, n: u32) -> Option<uk_assets::scenedef::RectDef> {
        self.s.ui.rects.get(&n).copied().or(self.def.nodes[n as usize].rect)
    }

    fn local_scale(&self, n: u32) -> Vec3 {
        self.s.ui.scale.get(&n).copied().unwrap_or(self.def.nodes[n as usize].local_scale)
    }

    // ---------------------------------------------------------------- lifecycle

    pub(crate) fn hud_awake(&mut self, sc: u32) {
        let Some(ui) = self.ui_def() else { return };
        let node = self.def.scripts[sc as usize].node;
        let def = self.def.clone();
        match self.hud(sc) {
            Some(HudScript::OpenEffect(_)) => self.open_effect_init(sc),
            Some(HudScript::Controller(c)) => {
                if c.alt_hud && c.alt_hud_obj.is_none() {
                    c.alt_hud_obj = def.nodes[node as usize].children.first().copied();
                }
                if !c.alt_hud && c.hud_pos.is_none() {
                    c.hud_pos = c.gun_canvas.and_then(|g| def.scripts_on(g).find(|(_, s)| s.class == "HUDPos").map(|(i, _)| i));
                }
            }
            _ => {}
        }
        let _ = ui;
    }

    pub(crate) fn hud_enable(&mut self, sc: u32) {
        if self.ui.is_none() {
            return;
        }
        match self.hud(sc) {
            Some(HudScript::OpenEffect(_)) => self.open_effect_reset(sc),
            Some(HudScript::Stamina(_)) => self.stamina_update_colors(sc),
            Some(HudScript::ColorGet { .. }) => self.color_get_update(sc),
            Some(HudScript::Pos(_)) => self.hud_pos_check(sc),
            Some(HudScript::Railcannon(_)) => self.railcannon_check_status(sc),
            Some(HudScript::SliderFill(f)) => f.last_invisible = !f.is_invisible,
            Some(HudScript::Speedometer) => {
                let on = self.prefs.int("speedometer") > 0;
                self.set_active(self.def.scripts[sc as usize].node, on);
            }
            _ => {}
        }
    }

    pub(crate) fn hud_start(&mut self, sc: u32) {
        let Some(ui) = self.ui_def() else { return };
        let node = self.def.scripts[sc as usize].node;
        let def = self.def.clone();
        let difficulty = self.prefs.int("difficulty");
        match self.hud(sc) {
            Some(HudScript::HealthBar(h)) => h.difficulty = difficulty,
            Some(HudScript::Stamina(_)) => {
                let slider = def.scripts_on(node).find(|(_, s)| s.class == "Slider").map(|(i, _)| i).filter(|i| ui.script_slider.contains_key(i));
                let text = ui.node_graphic.get(&node).copied().filter(|&g| matches!(ui.graphics[g as usize].kind, crate::ugui::GraphicKind::Tmp(_)));
                let (mut bar, mut flash) = (None, None);
                if slider.is_some() {
                    // transform.GetChild(1).GetChild(0).GetComponent<Image>(), and its GetChild(0)
                    let child = |n: u32, i: usize| def.nodes[n as usize].children.get(i).copied();
                    let img = |n: u32| ui.node_graphic.get(&n).copied().filter(|&g| matches!(ui.graphics[g as usize].kind, crate::ugui::GraphicKind::Image(_)));
                    let bn = child(node, 1).and_then(|c| child(c, 0));
                    bar = bn.and_then(img);
                    flash = bn.and_then(|b| child(b, 0)).and_then(img);
                }
                let mut canvas = None;
                let mut m = Some(node);
                while let Some(x) = m {
                    if let Some(c) = ui.canvases.iter().position(|c| c.node == x) {
                        canvas = Some(c as u32);
                        break;
                    }
                    m = def.nodes[x as usize].parent;
                }
                let flash_color = flash.map(|g| self.s.ui.color[g as usize]).unwrap_or([0.0; 4]);
                let orig = bar.map(|g| self.s.ui.color[g as usize]).unwrap_or([0.0; 4]);
                if let Some(HudScript::Stamina(st)) = self.hud(sc) {
                    st.slider = slider;
                    st.text = text;
                    st.bar = bar;
                    st.flash = flash;
                    st.flash_color = flash_color;
                    st.orig_color = orig;
                    st.last_stamina = st.stamina;
                    st.parent_canvas = canvas;
                }
                self.stamina_update_colors(sc);
            }
            Some(HudScript::ColorGet { .. }) => self.color_get_update(sc),
            Some(HudScript::Controller(_)) => self.controller_start(sc),
            Some(HudScript::Pos(_)) => self.hud_pos_check(sc),
            Some(HudScript::LevelStatsEnabler(_)) => self.level_stats_enabler_start(sc),
            Some(HudScript::LevelStats(_)) => self.level_stats_start(sc),
            Some(HudScript::Railcannon(_)) => self.railcannon_check_status(sc),
            Some(HudScript::Crosshair(_)) => self.crosshair_check(sc),
            Some(HudScript::FadeOutBars(_)) => self.fade_out_bars_start(sc),
            _ => {}
        }
    }

    /// The HUD's Update calls (script order).
    pub(crate) fn hud_update(&mut self, dt: f32) {
        if self.ui.is_none() {
            return;
        }
        for sc in 0..self.s.scripts.len() as u32 {
            if !matches!(self.s.scripts[sc as usize], Script::Hud(_)) || !self.script_live(sc) || !self.s.script_started[sc as usize] {
                continue;
            }
            match self.hud(sc) {
                Some(HudScript::OpenEffect(_)) => self.open_effect_update(sc, dt),
                Some(HudScript::HealthBar(_)) => self.health_bar_update(sc, dt),
                Some(HudScript::Stamina(_)) => self.stamina_update(sc, dt),
                Some(HudScript::LevelStatsEnabler(_)) => self.level_stats_enabler_update(sc, dt),
                Some(HudScript::LevelStats(l)) if l.ready => self.level_stats_check(sc),
                Some(HudScript::Railcannon(_)) => self.railcannon_update(sc, dt),
                Some(HudScript::FadeOutBars(f)) => {
                    // Time.unscaledDeltaTime (the port has no timescale changes on the HUD)
                    if f.fade_out {
                        f.fade_out_time = move_towards(f.fade_out_time, 0.0, dt);
                    }
                }
                Some(HudScript::SliderFill(_)) => self.slider_fill_update(sc),
                Some(&mut HudScript::Style { force_meter_on }) => {
                    // StyleHUD.UpdateMeter: styleHud (child 0) is shown while comboActive || forceMeterOn
                    let node = self.def.scripts[sc as usize].node;
                    if let Some(&hud) = self.def.nodes[node as usize].children.first() {
                        self.set_active(hud, force_meter_on);
                    }
                }
                _ => {}
            }
        }
    }

    /// set_ui: the HUD scripts that woke up before the UI existed get their Awake / OnEnable /
    /// Start effects now, in that order.
    pub(crate) fn hud_replay(&mut self) {
        for sc in 0..self.s.scripts.len() as u32 {
            if !matches!(self.s.scripts[sc as usize], Script::Hud(_)) {
                continue;
            }
            let i = sc as usize;
            if self.s.script_awake[i] {
                self.hud_awake(sc);
                if self.script_live(sc) {
                    self.hud_enable(sc);
                }
            }
        }
        for sc in 0..self.s.scripts.len() as u32 {
            if matches!(self.s.scripts[sc as usize], Script::Hud(_)) && self.s.script_started[sc as usize] {
                self.hud_start(sc);
            }
        }
    }

    // ---------------------------------------------------------------- PlayerActivatorRelay

    /// PlayerActivator.ActivateObjects: PlayerActivatorRelay.ResetIndex + Activate.
    pub(crate) fn relay_reset_activate(&mut self) {
        let Some(sc) = (0..self.s.scripts.len() as u32).find(|&i| self.s.scripts[i as usize].hud().is_some_and(|h| matches!(h, HudScript::Relay(_)))) else { return };
        if let Some(HudScript::Relay(r)) = self.hud(sc) {
            r.index = 0;
        }
        self.relay_activate(sc);
    }

    /// PlayerActivatorRelay.Activate (MapInfoBase.hideStockHUD is false in the campaign).
    pub(crate) fn relay_activate(&mut self, sc: u32) {
        let weapons = self.s.has_revolver;
        let icons = self.prefs.flag("weaponIcons");
        let Some(HudScript::Relay(r)) = self.hud(sc) else { return };
        if r.index >= r.to_activate.len() {
            return;
        }
        let t = r.to_activate[r.index];
        let (gun, cross) = (r.gun_panel, r.crosshair);
        r.index += 1;
        let again = (r.index < r.to_activate.len()).then_some(r.delay);
        if let Some(t) = t {
            if Some(t) == gun {
                if weapons && icons {
                    self.set_active(t, true);
                }
            } else if Some(t) == cross {
                self.set_active(t, true);
            } else {
                self.set_active(t, true);
            }
        }
        if let Some(d) = again {
            self.invoke(sc, Act::HudRelay, d);
        }
    }

    // ---------------------------------------------------------------- HudOpenEffect

    fn oe_dims(&self, n: u32, dont_use_scale: bool) -> Vec2 {
        if dont_use_scale {
            self.rect_of(n).map_or(Vec2::ZERO, |r| Vec2::from_array(r.size_delta))
        } else {
            self.local_scale(n).truncate()
        }
    }

    fn open_effect_init(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let Some(HudScript::OpenEffect(o)) = &self.s.scripts[sc as usize].hud() else { return };
        if o.got_values {
            return;
        }
        let d = self.oe_dims(node, o.dont_use_scale);
        let Some(HudScript::OpenEffect(o)) = self.hud(sc) else { return };
        o.original = d;
        o.target = d;
        o.original_speed = o.speed;
        o.got_values = true;
    }

    /// HudOpenEffect.ResetValues()
    fn open_effect_reset(&mut self, sc: u32) {
        self.open_effect_init(sc);
        let node = self.def.scripts[sc as usize].node;
        let Some(HudScript::OpenEffect(o)) = self.hud(sc) else { return };
        o.speed = o.original_speed;
        if o.skip {
            return;
        }
        if o.reverse {
            // Reverse(speed)
            o.target = Vec2::new(0.025, 0.0);
            o.animating = true;
        }
        let (dont, rev, orig) = (o.dont_use_scale, o.reverse, o.original);
        o.animating = true;
        if dont {
            if let Some(mut r) = self.rect_of(node) {
                r.size_delta = orig.to_array();
                self.s.ui.rects.insert(node, r);
            }
        } else {
            let z = self.local_scale(node).z;
            let s = if rev { 1.0 } else { 0.05 };
            self.s.ui.scale.insert(node, Vec3::new(s, s, z));
        }
    }

    /// HudOpenEffect.Reverse(newSpeed)
    pub fn open_effect_reverse(&mut self, sc: u32, speed: f32) {
        if let Some(HudScript::OpenEffect(o)) = self.hud(sc) {
            o.target = Vec2::new(0.025, 0.0);
            o.speed = speed;
            o.animating = true;
        }
    }

    fn open_effect_update(&mut self, sc: u32, dt: f32) {
        let node = self.def.scripts[sc as usize].node;
        let Some(HudScript::OpenEffect(o)) = self.s.scripts[sc as usize].hud() else { return };
        if !o.animating {
            return;
        }
        let o = o.clone();
        let (mut x, mut y) = if o.dont_use_scale {
            let sd = self.rect_of(node).map_or([0.0; 2], |r| r.size_delta);
            (sd[0] / o.original.x, sd[1] / o.original.y)
        } else {
            let s = self.local_scale(node);
            (s.x, s.y)
        };
        let t = o.target;
        if !o.skip {
            if o.y_first && y != t.y {
                y = move_towards(y, t.y, dt * (((t.y - y).abs() + 0.1) * o.speed));
            } else if x != t.x {
                x = move_towards(x, t.x, dt * (((t.x - x).abs() + 0.1) * o.speed));
            } else if y != t.y {
                y = move_towards(y, t.y, dt * (((t.y - y).abs() + 0.1) * o.speed));
            }
        } else {
            (x, y) = (t.x, t.y);
        }
        if o.dont_use_scale {
            if let Some(mut r) = self.rect_of(node) {
                r.size_delta = [x * o.original.x, y * o.original.y];
                self.s.ui.rects.insert(node, r);
            }
        } else {
            let z = self.local_scale(node).z;
            self.s.ui.scale.insert(node, Vec3::new(x, y, z));
        }
        if x == t.x && y == t.y {
            if let Some(HudScript::OpenEffect(o)) = self.hud(sc) {
                o.animating = false;
            }
            if x == 0.0 && y == 0.0 {
                self.set_active(node, false);
            }
        }
    }

    // ---------------------------------------------------------------- Crosshair

    /// Crosshair.CheckCrossHair (HideUI is never active; the invertMaterial of crossHairColor 0 is
    /// not swapped in: the Image keeps its serialized material).
    fn crosshair_check(&mut self, sc: u32) {
        let Some(ui) = self.ui_def() else { return };
        let Some(HudScript::Crosshair(c)) = self.hud(sc) else { return };
        let mut c = c.clone();
        if c.circles.is_empty() {
            if let Some(a) = self.ui_assets.clone() {
                let n = self.def.scripts[sc as usize].data.get("circles").array().len();
                c.circles = (0..n).map(|i| a.sprite(sc, &format!("circles/{i}"))).collect();
                if let Some(HudScript::Crosshair(cc)) = self.hud(sc) {
                    cc.circles.clone_from(&c.circles);
                }
            }
        }
        let node = self.def.scripts[sc as usize].node;
        let main = ui.node_graphic.get(&node).copied();
        let main_script = main.map(|g| ui.graphics[g as usize].script);
        let set = |g: &mut Game, s: Option<u32>, on: bool| {
            if let Some(s) = s {
                g.s.script_enabled[s as usize] = on;
            }
        };
        let (main_on, alt_on) = match self.prefs.int("crossHair") {
            0 => (Some(false), Some(false)),
            1 => (Some(true), Some(false)),
            2 => (Some(true), Some(true)),
            _ => (None, None),
        };
        if let Some(on) = main_on {
            set(self, main_script, on);
        }
        if let Some(on) = alt_on {
            for &a in &c.altchs {
                set(self, a, on);
            }
        }
        let color = match self.prefs.int("crossHairColor") {
            2 => [0.5, 0.5, 0.5, 1.0],
            3 => [0.0, 0.0, 0.0, 1.0],
            4 => [1.0, 0.0, 0.0, 1.0],
            5 => [0.0, 1.0, 0.0, 1.0],
            6 => [0.0, 0.0, 1.0, 1.0],
            7 => [0.0, 1.0, 1.0, 1.0],
            8 => [1.0, 0.92156863, 0.015686275, 1.0],
            9 => [1.0, 0.0, 1.0, 1.0],
            _ => WHITE,
        };
        if let Some(g) = main {
            self.set_graphic_color(g, color);
        }
        for &a in &c.altchs {
            if let Some(g) = a.and_then(|a| self.graphic_of_script(&ui, a)) {
                self.set_graphic_color(g, color);
            }
        }
        let hud = self.prefs.int("crossHairHud");
        for &ch in &c.chuds {
            set(self, ch, hud != 0);
            if hud != 0 {
                if let Some(g) = ch.and_then(|a| self.graphic_of_script(&ui, a)) {
                    self.s.ui.sprite[g as usize] = c.circles.get(hud as usize - 1).copied().flatten();
                }
            }
        }
    }

    /// FadeOutBars.Start: CheckState, then every SliderToFillAmount below gets it as `mama`.
    fn fade_out_bars_start(&mut self, sc: u32) {
        self.fade_out_bars_reset(sc, true);
        let node = self.def.scripts[sc as usize].node;
        // GetComponentsInChildren: active objects only
        let mut sub = vec![node];
        let mut i = 0;
        while i < sub.len() {
            let n = sub[i];
            sub.extend(self.def.nodes[n as usize].children.iter().copied().filter(|&c| self.s.active[c as usize]));
            i += 1;
        }
        for n in sub {
            let ids: Vec<u32> = self.def.scripts_on(n).map(|(i, _)| i).collect();
            for i in ids {
                if let Some(HudScript::SliderFill(f)) = self.hud(i) {
                    f.mama = Some(sc);
                }
            }
        }
    }

    /// FadeOutBars.CheckState (`check`) / ResetTimer (HideUI is never active).
    fn fade_out_bars_reset(&mut self, sc: u32, check: bool) {
        let fade = self.prefs.flag("crossHairHudFade");
        let off = self.prefs.int("crossHairHud") == 0;
        if let Some(HudScript::FadeOutBars(f)) = self.hud(sc) {
            if check {
                f.fade_out = fade;
            }
            f.fade_out_time = if off { 0.0 } else { 2.0 };
        }
    }

    /// SliderToFillAmount.Update
    fn slider_fill_update(&mut self, sc: u32) {
        let Some(ui) = self.ui_def() else { return };
        let node = self.def.scripts[sc as usize].node;
        let Some(&img) = ui.node_graphic.get(&node) else { return };
        let Some(HudScript::SliderFill(f)) = self.hud(sc) else { return };
        let f = f.clone();
        let a = self.s.ui.color[img as usize][3];
        // Mathf.Approximately(a, 0)
        let invisible = a.abs() < 8.0 * f32::MIN_POSITIVE;
        let mut last = f.last_invisible;
        if invisible != last {
            self.s.script_enabled[ui.graphics[img as usize].script as usize] = !invisible;
            last = invisible;
        }
        if let Some(HudScript::SliderFill(ff)) = self.hud(sc) {
            ff.is_invisible = invisible;
            ff.last_invisible = last;
        }
        let Some(si) = f.target.and_then(|t| ui.script_slider.get(&t).copied()) else { return };
        let sl = &ui.sliders[si as usize];
        let num = (self.s.ui.slider[si as usize] - sl.min) / (sl.max - sl.min) * f.max_fill;
        if num != self.s.ui.fill[img as usize] {
            // Image.fillAmount clamps to 0..1
            self.s.ui.fill[img as usize] = num.clamp(0.0, 1.0);
            if let Some(m) = f.mama {
                self.fade_out_bars_reset(m, false);
            }
        }
        if f.copy_color {
            // targetSlider.targetGraphic.color
            let target = ui.selectables.iter().find(|s| s.script == sl.script).and_then(|s| s.target);
            if let Some(t) = target {
                let c = self.s.ui.color[t as usize];
                if c != self.s.ui.color[img as usize] {
                    self.s.ui.color[img as usize] = c;
                }
            }
        }
        if let Some(m) = f.mama {
            if let Some(HudScript::FadeOutBars(fo)) = self.hud(m) {
                let num2 = fo.fade_out_time.min(1.0);
                let c = &mut self.s.ui.color[img as usize];
                if num2 != c[3] {
                    c[3] = num2;
                }
            }
        }
    }

    // ---------------------------------------------------------------- HealthBar

    fn health_bar_update(&mut self, sc: u32, dt: f32) {
        let Some(ui) = self.ui_def() else { return };
        let nhp = self.s.hp as f32;
        // NewMovement.antiHp (hard damage) is not ported: always 0
        let n_anti = 0.0f32;
        let Some(HudScript::HealthBar(h)) = self.hud(sc) else { return };
        if h.hp < nhp {
            h.hp = move_towards(h.hp, nhp, dt * ((nhp - h.hp) * 5.0 + 5.0));
        } else if h.hp > nhp {
            h.hp = nhp;
        }
        let h = h.clone();
        let hp = h.hp;
        for &s in &h.hp_sliders {
            if let Some((i, v)) = self.slider_value(&ui, s) {
                if v != hp {
                    self.set_slider_value(&ui, i, hp);
                }
            }
        }
        for &s in &h.after_image {
            if let Some((i, v)) = self.slider_value(&ui, s) {
                if v < hp {
                    self.set_slider_value(&ui, i, hp);
                } else if v > hp {
                    self.set_slider_value(&ui, i, move_towards(v, hp, dt * ((v - hp) * 5.0 + 5.0)));
                }
            }
        }
        if let Some((i, v)) = h.anti_slider.and_then(|s| self.slider_value(&ui, s)) {
            if v != n_anti {
                self.set_slider_value(&ui, i, move_towards(v, n_anti, dt * ((v - n_anti).abs() * 5.0 + 5.0)));
            }
            if let Some(f) = h.anti_fill {
                let on = self.s.ui.slider[i as usize] > 0.0;
                self.s.script_enabled[f as usize] = on;
            }
        }
        let Some(g) = h.text.and_then(|t| self.graphic_of_script(&ui, t)) else { return };
        if !h.anti_hp_text {
            if h.last_hp != hp {
                self.s.ui.text[g as usize] = Some(net_fixed(hp, 0).into());
                if let Some(HudScript::HealthBar(hb)) = self.hud(sc) {
                    hb.last_hp = hp;
                }
            }
            if h.change_text_color {
                let c = if hp <= 30.0 {
                    RED
                } else if hp <= 50.0 && h.yellow_color {
                    YELLOW
                } else {
                    h.normal_text_color
                };
                self.set_graphic_color(g, c);
            } else if color_eq(h.normal_text_color, WHITE) {
                let c = if hp <= 30.0 { RED } else { self.colors.hud(hct::HEALTH_TEXT) };
                self.set_graphic_color(g, c);
            }
        } else if h.difficulty == 0 {
            self.s.ui.text[g as usize] = Some("/200".into());
        } else {
            let anti = move_towards(h.anti_hp, n_anti, dt * ((h.anti_hp - n_anti).abs() * 5.0 + 5.0));
            let num = 100.0 - anti;
            let changed = h.last_anti_hp != num;
            if let Some(HudScript::HealthBar(hb)) = self.hud(sc) {
                hb.anti_hp = anti;
                if changed {
                    hb.last_anti_hp = num;
                }
            }
            if changed {
                self.s.ui.text[g as usize] = Some(format!("/{}", net_fixed(num, 0)).into());
            }
        }
    }

    // ---------------------------------------------------------------- StaminaMeter

    /// StaminaMeter.UpdateColors
    fn stamina_update_colors(&mut self, sc: u32) {
        let orig = self.colors.hud(hct::STAMINA);
        let (charging, empty) = (self.colors.hud(hct::STAMINA_CHARGING), self.colors.hud(hct::STAMINA_EMPTY));
        let Some(HudScript::Stamina(st)) = self.hud(sc) else { return };
        st.orig_color = orig;
        st.empty_color = if st.red_empty { empty } else { charging };
        if let Some(b) = st.bar {
            let c = if st.full { st.orig_color } else { st.empty_color };
            self.set_graphic_color(b, c);
        }
    }

    fn stamina_update(&mut self, sc: u32, dt: f32) {
        let Some(ui) = self.ui_def() else { return };
        let charge = self.s.player.boost_charge;
        let Some(HudScript::Stamina(st)) = self.hud(sc) else { return };
        if st.intro {
            st.stamina = move_towards(st.stamina, charge, dt * ((charge - st.stamina) * 5.0 + 10.0));
            if st.stamina >= charge {
                st.intro = false;
            }
        } else if st.stamina < charge {
            st.stamina = move_towards(st.stamina, charge, dt * ((charge - st.stamina) * 25.0 + 25.0));
        } else if st.stamina > charge {
            st.stamina = move_towards(st.stamina, charge, dt * ((st.stamina - charge) * 25.0 + 25.0));
        }
        let st = (**st).clone();
        let canvas_on = st.parent_canvas.is_some_and(|c| self.s.ui.canvas_enabled[c as usize]);
        if !st.always_update && !canvas_on {
            return;
        }
        let mut full = st.full;
        let mut flash_color = st.flash_color;
        if let Some((i, _)) = st.slider.and_then(|s| self.slider_value(&ui, s)) {
            self.set_slider_value(&ui, i, st.stamina);
            let max = ui.sliders[i as usize].max;
            let v = self.s.ui.slider[i as usize];
            if v >= max && !full {
                full = true;
                if let Some(b) = st.bar {
                    self.set_graphic_color(b, st.orig_color);
                }
                // Flash(): the AudioSource plays (no audio yet); flashColor = white
                flash_color = WHITE;
                if let Some(f) = st.flash {
                    self.set_graphic_color(f, flash_color);
                }
            }
            if flash_color[3] > 0.0 {
                if flash_color[3] - dt > 0.0 {
                    flash_color[3] -= dt;
                } else {
                    flash_color[3] = 0.0;
                }
                if let Some(f) = st.flash {
                    self.set_graphic_color(f, flash_color);
                }
            }
            if v < max {
                full = false;
                if let Some(b) = st.bar {
                    self.set_graphic_color(b, st.empty_color);
                }
            }
        }
        if let Some(HudScript::Stamina(s)) = self.hud(sc) {
            s.full = full;
            s.flash_color = flash_color;
        }
        let Some(g) = st.text else { return };
        if st.last_stamina != st.stamina {
            self.s.ui.text[g as usize] = Some(net_fixed(st.stamina / 100.0, 2).into());
        }
        if let Some(HudScript::Stamina(s)) = self.hud(sc) {
            s.last_stamina = s.stamina;
        }
        if st.change_text_color {
            let c = if st.stamina < 100.0 { RED } else { self.colors.hud(hct::STAMINA) };
            self.set_graphic_color(g, c);
        } else if color_eq(st.normal_text_color, WHITE) {
            let c = if st.stamina < 100.0 { RED } else { self.colors.hud(hct::HEALTH_TEXT) };
            self.set_graphic_color(g, c);
        }
    }

    // ---------------------------------------------------------------- ColorBlindGet

    fn color_get_update(&mut self, sc: u32) {
        let Some(ui) = self.ui_def() else { return };
        let Some(HudScript::ColorGet { hct, variation }) = self.hud(sc).cloned() else { return };
        let c = match variation {
            Some(v) => self.colors.variation.get(v).copied().unwrap_or(WHITE),
            None => self.colors.hud(hct),
        };
        let node = self.def.scripts[sc as usize].node;
        // img / txt / txt2: every Graphic on the object
        if let Some(&g) = ui.node_graphic.get(&node) {
            self.set_graphic_color(g, c);
        }
    }

    // ---------------------------------------------------------------- RailcannonMeter

    /// RailcannonMeter.RailcannonStatus: a railcannon variant owned (CheckGear) and enabled, and weapons
    fn railcannon_status(&self) -> bool {
        let general = self.save.general();
        let no_weapons = !self.s.has_revolver;
        (0..4).any(|i| {
            let gear = general.as_ref().map_or(0, |g| g.get(&format!("rai{i}")).i64());
            gear == 1 && self.prefs.int_or(&format!("weapon.rai{i}"), 1) == 1 && !no_weapons
        })
    }

    /// RailcannonMeter.CheckStatus (Start / OnEnable)
    fn railcannon_check_status(&mut self, sc: u32) {
        let Some(HudScript::Railcannon(r)) = self.hud(sc).cloned() else { return };
        let me = self.def.scripts_on(self.def.scripts[sc as usize].node).find(|(_, s)| s.class == "Image").map(|(i, _)| i);
        let on = self.prefs.flag("railcannonMeter") && self.railcannon_status();
        let icons = self.prefs.flag("weaponIcons");
        if let Some(me) = me {
            self.s.script_enabled[me as usize] = on && icons;
        }
        if let Some(m) = r.mini {
            self.set_active(m, on && !icons);
        }
        for p in r.alt_hud_panels.iter().flatten() {
            self.set_active(*p, on);
        }
    }

    /// RailcannonMeter.Update
    fn railcannon_update(&mut self, sc: u32, dt: f32) {
        let Some(ui) = self.ui_def() else { return };
        let Some(HudScript::Railcannon(mut r)) = self.hud(sc).cloned() else { return };
        let me = self.def.scripts_on(self.def.scripts[sc as usize].node).find(|(_, s)| s.class == "Image").map(|(i, _)| i);
        let self_on = me.is_some_and(|i| self.s.script_enabled[i as usize]);
        let mini_on = r.mini.is_some_and(|m| self.s.active_self[m as usize]);
        // WeaponCharges.raicharge (railcannon not ported)
        let raicharge = 0.0f32;
        let enable = |g: &mut Self, s: Option<u32>, on: bool| {
            if let Some(s) = s {
                g.s.script_enabled[s as usize] = on;
            }
        };
        if self_on || mini_on {
            for (i, &m) in r.true_meters.iter().enumerate() {
                enable(self, m, self_on || i != 0);
            }
            if raicharge > 4.0 {
                // Time.timeScale is always 1 here
                if !r.has_flashed {
                    r.flash = 1.0;
                }
                r.has_flashed = true;
                let mut c = self.colors.hud(hct::RAILCANNON_FULL);
                if r.flash > 0.0 {
                    c = std::array::from_fn(|i| c[i] + (WHITE[i] - c[i]) * r.flash);
                    r.flash = (r.flash - dt).max(0.0);
                }
                for &m in r.true_meters.iter().flatten() {
                    if let Some(g) = self.graphic_of_script(&ui, m) {
                        self.s.ui.fill[g as usize] = 1.0;
                        self.set_graphic_color(g, if Some(m) != r.colorless { c } else { WHITE });
                    }
                }
            } else {
                r.flash = 0.0;
                r.has_flashed = false;
                let c = self.colors.hud(hct::RAILCANNON_CHARGING);
                for &m in r.true_meters.iter().flatten() {
                    if let Some(g) = self.graphic_of_script(&ui, m) {
                        self.set_graphic_color(g, c);
                        self.s.ui.fill[g as usize] = (raicharge / 4.0).clamp(0.0, 1.0);
                    }
                }
            }
            enable(self, r.background, !(raicharge > 4.0 || !self_on));
        } else {
            r.flash = 0.0;
            r.has_flashed = false;
            enable(self, r.background, false);
            for &m in &r.true_meters {
                enable(self, m, false);
            }
        }
        if let Some(HudScript::Railcannon(x)) = self.hud(sc) {
            **x = *r;
        }
    }

    // ---------------------------------------------------------------- HudController / HUDPos

    fn controller_start(&mut self, sc: u32) {
        self.controller_check_situation(sc);
        let Some(HudScript::Controller(c)) = self.hud(sc).cloned() else { return };
        if !self.prefs.flag("weaponIcons") {
            if !c.alt_hud {
                if let Some(sp) = c.speedometer {
                    let n = self.def.scripts[sp as usize].node;
                    if let Some(mut r) = self.rect_of(n) {
                        r.anchored_pos = [-79.0, 190.0];
                        self.s.ui.rects.insert(n, r);
                    }
                }
                if let Some(w) = c.weapon_icon {
                    self.set_local_z(w, 45.0);
                }
            } else if let Some(w) = c.weapon_icon {
                self.set_active(w, false);
            }
        }
        if !self.prefs.flag("armIcons") {
            if !c.alt_hud {
                if let Some(a) = c.arm_icon {
                    self.set_local_z(a, 0.0);
                }
            } else if let Some(a) = c.arm_icon {
                self.set_active(a, false);
            }
        }
        if !c.alt_hud {
            if !self.prefs.flag("styleMeter") {
                if let Some(m) = c.style_meter {
                    self.set_local_z(m, -9999.0);
                }
            }
            if !self.prefs.flag("styleInfo") {
                if let Some(m) = c.style_info {
                    self.set_local_z(m, -9999.0);
                }
            }
        }
        let opacity = self.prefs.float("hudBackgroundOpacity");
        if opacity != 50.0 {
            // SetOpacity
            if let Some(ui) = self.ui_def() {
                for &b in &c.backgrounds {
                    if let Some(g) = self.graphic_of_script(&ui, b) {
                        self.s.ui.color[g as usize][3] = opacity / 100.0;
                    }
                }
            }
        }
        // SetAlwaysOnTop: textElements' fontSharedMaterial = overlay / normal text material
        if self.prefs.flag("hudAlwaysOnTop") && !c.text_elements.is_empty() {
            self.unknown_calls.insert("HudController.SetAlwaysOnTop(true): TMP material swap not ported".into());
        }
    }

    /// HudController.CheckSituation (HideUI is off)
    fn controller_check_situation(&mut self, sc: u32) {
        let Some(HudScript::Controller(c)) = self.hud(sc).cloned() else { return };
        let hud_type = self.prefs.int("hudType");
        if c.alt_hud {
            if let Some(o) = c.alt_hud_obj {
                let on = (hud_type == 2 && !c.colorless) || (hud_type == 3 && c.colorless);
                self.set_active(o, on);
            }
            return;
        }
        let Some(gc) = c.gun_canvas else { return };
        let canvas = self.ui.as_ref().and_then(|u| u.canvases.iter().position(|cd| cd.node == gc));
        let on = hud_type == 1;
        self.set_local_z(gc, if on { 1.0 } else { -100.0 });
        if let Some(ci) = canvas {
            self.s.ui.canvas_enabled[ci] = on;
        }
        if let Some(hp) = c.hud_pos {
            if let Some(HudScript::Pos(p)) = self.hud(hp) {
                p.active = on;
            }
            if on {
                self.hud_pos_check(hp);
            }
        }
    }

    /// HUDPos.CheckPos
    fn hud_pos_check(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let reverse = self.prefs.int("weaponHoldPosition") == 2;
        let cur_rect = self.rect_of(node);
        let cur_tr = self.local_tr(node);
        let Some(HudScript::Pos(p)) = self.hud(sc) else { return };
        if !p.active {
            return;
        }
        if !p.ready {
            p.ready = true;
            if p.rect_transform {
                p.default_rect = cur_rect.map(|r| (r.anchor_min, r.anchor_max, r.pivot, r.anchored_pos));
            } else {
                p.default_tr = Some(cur_tr);
            }
        }
        let p = (**p).clone();
        if p.rect_transform {
            let Some(mut r) = cur_rect else { return };
            let (amin, amax, piv, pos) = if reverse { (p.anchors_min, p.anchors_max, p.pivot, p.anchored_position) } else { p.default_rect.unwrap_or((r.anchor_min, r.anchor_max, r.pivot, r.anchored_pos)) };
            (r.anchor_min, r.anchor_max, r.pivot, r.anchored_pos) = (amin, amax, piv, pos);
            if Some(r) != self.def.nodes[node as usize].rect || self.s.ui.rects.contains_key(&node) {
                self.s.ui.rects.insert(node, r);
            }
        } else {
            let tr = if reverse {
                let e = p.reverse_rot;
                (p.reverse_pos, Quat::from_euler(EulerRot::YXZ, e.y.to_radians(), e.x.to_radians(), e.z.to_radians()))
            } else {
                p.default_tr.unwrap_or(cur_tr)
            };
            if tr != rest_local(&self.def, node) || self.s.ui.local.contains_key(&node) {
                self.s.ui.local.insert(node, tr);
            }
        }
    }

    // ---------------------------------------------------------------- LevelStatsEnabler

    /// StatsManager.levelNumber
    pub fn level_number(&self) -> i64 {
        self.def.scripts.iter().find(|s| s.class == "StatsManager").map_or(0, |s| s.data.get("levelNumber").i64())
    }

    /// StatsManager.levelNumber != 0 and GetRank(returnNull) holds a RankData for it (release build)
    fn has_level_rank(&self) -> bool {
        let lvl = self.level_number();
        lvl != 0 && self.save.rank(lvl).is_some_and(|r| r.get("levelNumber").i64() == lvl)
    }

    /// StockMapInfo.Instance.assets.LargeText
    fn stock_large_text(&self) -> Option<String> {
        let s = self.def.scripts.iter().find(|s| s.class == "StockMapInfo")?;
        Some(s.data.get("assets").get("LargeText").str().to_string())
    }

    pub(crate) fn set_tmp_text(&mut self, script: Option<u32>, text: &str) {
        let Some(ui) = self.ui_def() else { return };
        let Some(g) = script.and_then(|s| self.graphic_of_script(&ui, s)) else { return };
        if self.s.ui.text[g as usize].as_deref() != Some(text) {
            self.s.ui.text[g as usize] = Some(text.into());
        }
    }

    /// StatsManager.GetRanks (TMP rich-text letter)
    pub fn rank_text(&self, field: &str, value: f32, reverse: bool) -> &'static str {
        let sm = self.def.scripts.iter().find(|s| s.class == "StatsManager");
        let ranks: Vec<i64> = sm.map(|s| s.data.get(field).array().iter().map(|v| v.i64()).collect()).unwrap_or_default();
        let n = ranks.iter().take_while(|&&t| if reverse { value <= t as f32 } else { value >= t as f32 }).count();
        match n {
            _ if n >= ranks.len() => "<color=#FF0000>S</color>",
            0 => "<color=#0094FF>D</color>",
            1 => "<color=#4CFF00>C</color>",
            2 => "<color=#FFD800>B</color>",
            _ => "<color=#FF6A00>A</color>",
        }
    }

    /// LevelStats.Start (release build, not a custom level)
    fn level_stats_start(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let Some(HudScript::LevelStats(l)) = self.hud(sc).cloned() else { return };
        if l.secret_level || l.cyber_grind {
            self.set_tmp_text(l.level_name, if l.cyber_grind { "THE CYBER GRIND" } else { "SECRET MISSION" });
            if let Some(HudScript::LevelStats(l)) = self.hud(sc) {
                l.ready = true;
            }
            self.level_stats_check(sc);
            return;
        }
        if self.has_level_rank() {
            let name = self.stock_large_text().unwrap_or_else(|| "???".into());
            self.set_tmp_text(l.level_name, &name);
            if let Some(HudScript::LevelStats(l)) = self.hud(sc) {
                l.ready = true;
            }
            self.level_stats_check(sc);
        } else {
            self.set_active(node, false);
        }
        let n = self.secret_objects().len();
        for s in l.secrets.iter().skip(n).rev() {
            if let Some(s) = s {
                let sn = self.def.scripts[*s as usize].node;
                self.set_active(sn, false);
            }
        }
    }

    /// LevelStats.CheckStats (no ChallengeManager / major assists: "NO")
    fn level_stats_check(&mut self, sc: u32) {
        let Some(HudScript::LevelStats(l)) = self.hud(sc).cloned() else { return };
        let st = self.s.stats.clone();
        if l.time.is_some() {
            self.set_tmp_text(l.time, &stats_time_text(st.seconds));
        }
        if l.time_rank.is_some() {
            let r = self.rank_text("timeRanks", st.seconds, true);
            self.set_tmp_text(l.time_rank, r);
        }
        if l.cyber_grind {
            return;
        }
        if l.kills.is_some() {
            self.set_tmp_text(l.kills, &self.s.kills.to_string());
        }
        if l.kills_rank.is_some() {
            let r = self.rank_text("killRanks", self.s.kills as f32, false);
            self.set_tmp_text(l.kills_rank, r);
        }
        if l.style.is_some() {
            self.set_tmp_text(l.style, &st.style_points.to_string());
        }
        if l.style_rank.is_some() {
            let r = self.rank_text("styleRanks", st.style_points as f32, false);
            self.set_tmp_text(l.style_rank, r);
        }
        if l.check_secrets && !l.secrets.is_empty() {
            let mut all = true;
            let n = self.secret_objects().len();
            let filled = self.ui_assets.as_ref().and_then(|a| a.sprite(sc, "filledSecret"));
            let ui = self.ui_def();
            for (num, num2) in (0..n).rev().enumerate() {
                if st.prev_secrets.contains(&num) || st.new_secrets.contains(&num) {
                    let g = l.secrets.get(num2).copied().flatten().and_then(|s| ui.as_ref().and_then(|u| self.graphic_of_script(u, s)));
                    if let Some(g) = g {
                        self.s.ui.sprite[g as usize] = filled;
                    }
                } else {
                    all = false;
                }
            }
            if all {
                if let Some(HudScript::LevelStats(l)) = self.hud(sc) {
                    l.check_secrets = false;
                }
            }
        }
        if l.challenge.is_some() {
            self.set_tmp_text(l.challenge, "NO");
        }
        if l.major_assists.is_some() {
            let t = if st.major_used { "<color=#4C99E6>YES</color>" } else { "NO" };
            self.set_tmp_text(l.major_assists, t);
        }
    }

    /// LevelStatsEnabler.Start (a release build, not a custom level)
    fn level_stats_enabler_start(&mut self, sc: u32) {
        let node = self.def.scripts[sc as usize].node;
        let Some(HudScript::LevelStatsEnabler(e)) = self.hud(sc).cloned() else { return };
        if !e.can_always_enable {
            let hide = if e.secret_level < 0 {
                !self.has_level_rank()
            } else {
                let m = self.save.general().map_or(0, |g| g.get("secretMissions").array().get(e.secret_level as usize).map_or(0, |v| v.i64()));
                m < 2
            };
            if hide {
                self.save.player_prefs.insert("LevStaOpe".into(), uk_assets::save::PlayerPref::Int(0));
                self.set_active(node, false);
            } else if e.secret_level < 0 && self.save.pp_int("LevStaTut", 0) == 0 {
                self.invoke(sc, Act::LevelStatsTutorial, 1.5);
            }
        }
        let child = self.def.nodes[node as usize].children.first().copied();
        let open = self.save.pp_int("LevStaOpe", 0) != 0;
        if let Some(HudScript::LevelStatsEnabler(e)) = self.hud(sc) {
            e.level_stats = child;
            e.keep_open = open;
        }
        if let (Some(c), false) = (child, open) {
            self.set_active(c, false);
        }
    }

    /// LevelStatsEnabler.LevelStatsTutorial
    pub(crate) fn level_stats_tutorial(&mut self) {
        self.save.player_prefs.insert("LevStaTut".into(), uk_assets::save::PlayerPref::Int(1));
        self.send_hud_message("Hold <color=orange>TAB</color> to see current stats when <color=orange>REPLAYING</color> a level.
<color=orange>DOUBLE TAP</color> to keep open.");
    }

    fn level_stats_enabler_update(&mut self, sc: u32, dt: f32) {
        let inp = self.hud_input;
        let Some(HudScript::LevelStatsEnabler(e)) = self.hud(sc) else { return };
        let mut set = None;
        let mut pref = None;
        if !e.keep_open {
            if inp.stats_performed {
                if e.double_tap > 0.0 {
                    pref = Some(1);
                    e.keep_open = true;
                } else {
                    e.double_tap = 0.5;
                }
                set = Some(true);
            } else if inp.stats_canceled {
                set = Some(false);
            }
        } else if inp.stats_performed {
            e.keep_open = false;
            pref = Some(0);
            set = Some(false);
        }
        if e.double_tap > 0.0 {
            e.double_tap = move_towards(e.double_tap, 0.0, dt);
        }
        let ls = e.level_stats;
        if let Some(p) = pref {
            self.save.player_prefs.insert("LevStaOpe".into(), uk_assets::save::PlayerPref::Int(p));
        }
        if let (Some(on), Some(n)) = (set, ls) {
            self.set_active(n, on);
        }
    }

    // ---------------------------------------------------------------- NewMovement HUD sway

    /// NewMovement.Update (unless reduceHudMotion): screenHud trails the camera-local velocity,
    /// hudCam leads it.
    pub(crate) fn hud_sway(&mut self, dt: f32) {
        let Some(sw) = self.sway else { return };
        if self.prefs.flag("reduceHudMotion") {
            return;
        }
        let q = crate::game::view_quat(self.s.player.yaw_deg, self.s.view_pitch);
        let lb = q.inverse() * self.s.player.vel;
        let lu = Vec3::new(lb.x, lb.y, -lb.z);
        let target = sw.hud_original - lu / 1000.0;
        let cur = self.local_tr(sw.screen_hud).0;
        let d = target.distance(cur);
        self.set_local_pos(sw.screen_hud, vmove_towards(cur, target, dt * 15.0 * d));
        let target = (sw.cam_original - lu / 350.0 * -1.0).clamp_length_max(0.2);
        let cur = self.local_tr(sw.hud_cam).0;
        let d = target.distance(cur);
        self.set_local_pos(sw.hud_cam, vmove_towards(cur, target, dt * 25.0 * d));
    }
}

impl Script {
    pub fn hud(&self) -> Option<&HudScript> {
        match self {
            Script::Hud(h) => Some(h),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{net_fixed, stats_time_text};

    #[test]
    fn stats_time() {
        assert_eq!(stats_time_text(0.0), "0:00.000");
        assert_eq!(stats_time_text(5.25), "0:05.250");
        assert_eq!(stats_time_text(75.5), "1:15.500");
        assert_eq!(stats_time_text(122.357574), "2:02.358");
        // the minutes are taken before rounding, like the game
        assert_eq!(stats_time_text(659.9996), "10:60.000");
    }

    #[test]
    fn fixed_formats() {
        assert_eq!(net_fixed(100.0, 0), "100");
        assert_eq!(net_fixed(99.5, 0), "100");
        assert_eq!(net_fixed(0.4, 0), "0");
        assert_eq!(net_fixed(0.0, 0), "0");
        assert_eq!(net_fixed(3.0, 2), "3.00");
        assert_eq!(net_fixed(2.675, 2), "2.68");
        assert_eq!(net_fixed(0.005, 2), "0.01");
        assert_eq!(net_fixed(0.0, 2), "0.00");
        assert_eq!(net_fixed(1.999, 2), "2.00");
        assert_eq!(net_fixed(0.123, 2), "0.12");
        assert_eq!(net_fixed(12.0, 0), "12");
    }
}
