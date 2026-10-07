//! ULTRAKILL's preferences, read-only: `Preferences/Prefs.json` and `LocalPrefs.json` in the install
//! (PrefsManager.PrefsPath), falling back to PrefsManager.defaultValues for missing keys.

use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// PrefsManager.defaultValues (the entries the port reads; `disabledComputeShaders` is hardware-dependent and omitted).
const DEFAULTS: &[(&str, f64)] = &[
    ("difficulty", 2.0),
    ("mouseSensitivity", 50.0),
    ("gameSpeed", 1.0),
    ("damageTaken", 1.0),
    ("autoAimAmount", 0.2),
    ("outlineThickness", 1.0),
    ("fieldOfView", 105.0),
    ("musicVolume", 0.6),
    ("sfxVolume", 1.0),
    ("allVolume", 1.0),
    ("screenShake", 1.0),
    ("dithering", 0.2),
    ("colorCompression", 2.0),
    ("vertexWarping", 0.0),
    ("textureWarping", 0.0),
    ("pixelization", 0.0),
    ("gamma", 1.0),
    ("crossHair", 1.0),
    ("crossHairColor", 1.0),
    ("crossHairHud", 2.0),
    ("hudType", 1.0),
    ("hudBackgroundOpacity", 50.0),
    ("simplifyEnemies", 0.0),
];

#[derive(Default, Clone, Debug)]
pub struct Prefs {
    map: HashMap<String, Value>,
    local: HashMap<String, Value>,
}

impl Prefs {
    /// Missing or malformed files act as empty, like PrefsManager (which then uses the defaults).
    pub fn load(install: &Path) -> Prefs {
        let read = |name: &str| -> HashMap<String, Value> {
            std::fs::read_to_string(install.join("Preferences").join(name))
                .ok()
                .and_then(|s| serde_json::from_str::<HashMap<String, Value>>(&s).ok())
                .unwrap_or_default()
        };
        Prefs { map: read("Prefs.json"), local: read("LocalPrefs.json") }
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.map.get(key).or_else(|| self.local.get(key))
    }

    pub fn float(&self, key: &str) -> f32 {
        self.get(key).and_then(Value::as_f64).or_else(|| DEFAULTS.iter().find(|d| d.0 == key).map(|d| d.1)).unwrap_or(0.0) as f32
    }

    pub fn int(&self, key: &str) -> i32 {
        self.float(key) as i32
    }

    pub fn bool(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(Value::as_bool).unwrap_or(default)
    }

    /// Overrides a value for this run (never written back).
    pub fn set(&mut self, key: &str, v: f64) {
        self.map.insert(key.to_string(), Value::from(v));
    }
}

/// GraphicsSettings.GetPixelizationValue: the downscaled short side, 0 = native.
pub fn pixelization_value(option: i32) -> f32 {
    match option {
        1 => 720.0,
        2 => 480.0,
        3 => 360.0,
        4 => 240.0,
        5 => 144.0,
        6 => 36.0,
        _ => 0.0,
    }
}

/// GraphicsSettings.GetColorCompressionValue (`_ColorPrecision`).
pub fn color_compression_value(option: i32) -> f32 {
    match option {
        1 => 64.0,
        2 => 32.0,
        3 => 16.0,
        4 => 8.0,
        5 => 3.0,
        _ => 2048.0,
    }
}

/// GraphicsSettings.GetVertexWarpingValue (`_VertexWarping`; nonzero enables VERTEX_WARPING).
pub fn vertex_warping_value(option: i32) -> f32 {
    match option {
        1 => 400.0,
        2 => 160.0,
        3 => 80.0,
        4 => 40.0,
        5 => 16.0,
        _ => 0.0,
    }
}
