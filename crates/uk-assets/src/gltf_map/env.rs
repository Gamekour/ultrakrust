//! `-env` on an empty: the map's RenderSettings (skybox, fog, ambient light), in place of 0-1's.
//! Every property is optional; one left out keeps 0-1's value (fog linear 0-250 m in
//! (0.518, 0.235, 0.106), flat black ambient, and a camera that clears to black: 0-1's
//! Default-Skybox never shows). Settings are fixed for the level,
//! as no script that changes them at runtime (FogEnabler, SkyboxEnabler, ...) is ported.
use super::MapReport;
use crate::db::AssetDb;
use crate::scene::MaterialKey;
use crate::scenedef::SceneDef;
use serde_json::Value as Json;

/// The campaign's RenderSettings skyboxes. All but Default-Skybox (0-1's own, kept from the base
/// scene) live in this shared file; `skybox` (uk-assets example) lists them per level.
const SKY_FILE: &str = "CAB-56872c896f0e5b74fc3bff932eebbfda";
const SKIES: &[(&str, i64)] = &[
    ("BlackNoFog", -2861671488607520959),
    ("DawnSky 1", 404502996900694037),
    ("DaySky", 5882483977743092529),
    ("DaySky 2", -982138338955983059),
    ("EveningSky 1", -3123879007021396837),
    ("FraudCity_SkyMat_Night", 3685026820233865794),
    ("GreedSky", -4569516216838259398),
    ("GreedSky2", -8651844695234459503),
    ("GreedSky4", -8640136200613175004),
    ("LustSky", 8476240128902682097),
    ("LustSky 1", -1044569365471427301),
    ("LustSky 2", -3035990737521804638),
    ("OvercastSky", 4161589221845981409),
    ("OvercastSky 1", -8436378730830533170),
    ("RedSky", 2013778305358308649),
    ("RedSky2", -3538584794780690312),
    ("ViolenceSky", -6966677682961544536),
    ("ViolenceSky 1", -8378243418845928301),
    ("ViolenceSky3", -8135665001788955257),
];
const DEFAULT_SKY: &str = "Default-Skybox";
/// Fog distances past the camera's far plane (4000 m): fog off.
const NO_FOG: f32 = 100_000.0;

fn sky_names() -> String {
    std::iter::once(DEFAULT_SKY).chain(SKIES.iter().map(|s| s.0)).chain(std::iter::once("none")).collect::<Vec<_>>().join(", ")
}

/// The skybox material by name (case ignored); by its id, else by name in the file should the
/// ids ever change.
fn find_sky(db: &mut AssetDb, name: &str) -> Option<MaterialKey> {
    let &(canon, id) = SKIES.iter().find(|s| s.0.eq_ignore_ascii_case(name))?;
    let f = db.file(SKY_FILE).ok()?;
    if f.read_id(id).ok().is_some_and(|v| v.get("m_Name").str() == canon) {
        return Some(MaterialKey { file: SKY_FILE.to_string(), path_id: id });
    }
    f.objects.iter().filter(|o| o.class_id == 21).find(|o| f.read(o).ok().is_some_and(|v| v.get("m_Name").str() == canon)).map(|o| MaterialKey { file: SKY_FILE.to_string(), path_id: o.path_id })
}

/// A colour: `[r, g, b]` / `[r, g, b, a]` (0-1) or `"#RRGGBB"` / `"#RRGGBBAA"`, used as written
/// (Unity's colour picker values).
fn color(v: &Json) -> Option<[f32; 4]> {
    match v {
        Json::Array(a) if a.len() == 3 || a.len() == 4 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().map(|f| f as f32)).collect::<Option<_>>()?;
            Some([c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)])
        }
        Json::String(s) => {
            let h = s.trim().trim_start_matches('#');
            if !(h.len() == 6 || h.len() == 8) || !h.is_ascii() {
                return None;
            }
            let b = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok().map(|x| x as f32 / 255.0);
            Some([b(0)?, b(2)?, b(4)?, if h.len() == 8 { b(6)? } else { 1.0 }])
        }
        _ => None,
    }
}

fn number(v: &Json) -> Option<f32> {
    v.as_f64().map(|f| f as f32)
}

fn flag(v: &Json) -> Option<bool> {
    v.as_bool().or_else(|| v.as_f64().map(|f| f != 0.0))
}

/// Applies the first `-env`'s properties (`envs`: (node name, extras JSON)) to the scene's
/// RenderSettings.
pub fn apply(db: &mut AssetDb, def: &mut SceneDef, envs: &[(String, Option<String>)], report: &mut MapReport) {
    let Some((name, extras)) = envs.first() else { return };
    if envs.len() > 1 {
        report.warnings.push(format!("{} -env objects: only {name}'s settings are used", envs.len()));
    }
    let props: serde_json::Map<String, Json> = extras.as_deref().and_then(|e| serde_json::from_str(e).ok()).unwrap_or_default();
    let bad = |k: &str, want: &str| format!("{name}: `{k}` should be {want}; ignored");
    let mut w = Vec::new();
    let rs = &mut def.render_settings;
    let mut set = Vec::new();
    // as SkyboxEnabler: a skybox makes the camera clear to it (flags 1), none to its colour (2)
    let clear_bg = rs.camera_clear.map_or([0.0, 0.0, 0.0, 1.0], |c| c.1);
    let mut sky_on = None;
    if let Some(v) = props.get("skybox") {
        match v.as_str() {
            Some(s) if s.eq_ignore_ascii_case("none") => {
                rs.skybox = None;
                sky_on = Some(false);
                set.push("skybox none".to_string());
            }
            // 0-1's own skybox, already in place
            Some(s) if s.eq_ignore_ascii_case(DEFAULT_SKY) => {
                sky_on = Some(rs.skybox.is_some());
                set.push(format!("skybox {DEFAULT_SKY}"));
            }
            Some(s) => match find_sky(db, s) {
                Some(k) => {
                    rs.skybox = Some(k);
                    sky_on = Some(true);
                    set.push(format!("skybox {s}"));
                }
                None => w.push(format!("{name}: unknown skybox \"{s}\" (known: {})", sky_names())),
            },
            None => w.push(bad("skybox", "a name")),
        }
    }
    let sky_color = match props.get("sky_color") {
        Some(v) => color(v).or_else(|| {
            w.push(bad("sky_color", "a colour"));
            None
        }),
        None => None,
    };
    let bg = sky_color.unwrap_or(clear_bg);
    if sky_color.is_some() {
        set.push(format!("sky_color {bg:.3?}"));
    }
    match sky_on {
        Some(true) => {
            rs.camera_clear = Some((1, bg));
            if sky_color.is_some() {
                w.push(format!("{name}: `sky_color` only shows without a skybox; ignored"));
            }
        }
        Some(false) => rs.camera_clear = Some((2, bg)),
        // no skybox named: 0-1's camera (which clears to a colour)
        None => rs.camera_clear = Some((rs.camera_clear.map_or(2, |c| c.0), bg)),
    }
    if let Some(v) = props.get("fog") {
        match flag(v) {
            Some(on) => {
                rs.fog = on;
                set.push(format!("fog {on}"));
            }
            None => w.push(bad("fog", "true or false")),
        }
    }
    for (k, field) in [("fog_min", 0), ("fog_max", 1)] {
        let Some(v) = props.get(k) else { continue };
        match number(v) {
            Some(x) => {
                *(if field == 0 { &mut rs.fog_start } else { &mut rs.fog_end }) = x;
                set.push(format!("{k} {x}"));
            }
            None => w.push(bad(k, "a distance in metres")),
        }
    }
    // the renderer fogs whatever RenderSettings.fog says (the campaign's fog switches, FogEnabler
    // and co., aren't ported, and 16 levels start with it off), so off moves the fog out of sight
    if !rs.fog {
        (rs.fog_start, rs.fog_end) = (NO_FOG, NO_FOG * 2.0);
    } else if rs.fog_start >= rs.fog_end {
        w.push(format!("{name}: fog_min ({}) is not below fog_max ({})", rs.fog_start, rs.fog_end));
    }
    if let Some(v) = props.get("fog_color") {
        match color(v) {
            Some(c) => {
                rs.fog_color = c;
                set.push(format!("fog_color {c:.3?}"));
            }
            None => w.push(bad("fog_color", "a colour")),
        }
    }
    // Flat ambient (0-1's mode): Unity has no intensity for it, so the strength scales the colour
    let ambient = match props.get("ambient_color") {
        Some(v) => color(v).or_else(|| {
            w.push(bad("ambient_color", "a colour"));
            None
        }),
        None => None,
    };
    let strength = match props.get("ambient_strength") {
        Some(v) => number(v).or_else(|| {
            w.push(bad("ambient_strength", "a number"));
            None
        }),
        None => None,
    };
    if ambient.is_some() || strength.is_some() {
        let c = ambient.unwrap_or(rs.ambient_sky);
        let s = strength.unwrap_or(1.0);
        rs.ambient_mode = 3;
        rs.ambient_sky = [c[0] * s, c[1] * s, c[2] * s, c[3]];
        set.push(format!("ambient {:.3?}", rs.ambient_sky));
    }
    for k in props.keys().filter(|k| !["skybox", "sky_color", "fog", "fog_min", "fog_max", "fog_color", "ambient_color", "ambient_strength"].contains(&k.as_str())) {
        w.push(format!("{name}: unknown -env property `{k}`; ignored"));
    }
    report.warnings.extend(w);
    report.env = set;
}
