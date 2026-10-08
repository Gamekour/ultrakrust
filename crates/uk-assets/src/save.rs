//! The player's save state, read-only: GameProgressSaver's `Saves/Slot<n>/*.bepis` files (MS-NRBF,
//! see `nrbf.rs`) and Unity PlayerPrefs (on Windows: `HKCU\Software\Hakita\ULTRAKILL`, one value
//! per key named `<key>_h<hash>`).

use crate::serialized::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerPref {
    Int(i64),
    Float(f64),
    Str(String),
}

#[derive(Clone, Debug, Default)]
pub struct Save {
    /// GameProgressSaver.SavePath (`Saves/Slot<selectedSaveSlot + 1>`); None = no save data
    pub slot_dir: Option<PathBuf>,
    pub player_prefs: HashMap<String, PlayerPref>,
}

impl Save {
    /// `selected_slot` is PrefsManager's "selectedSaveSlot" (GameProgressSaver.currentSlot).
    pub fn load(install: &Path, selected_slot: i32) -> Save {
        let dir = install.join("Saves").join(format!("Slot{}", selected_slot + 1));
        Save { slot_dir: dir.is_dir().then_some(dir), player_prefs: read_player_prefs() }
    }

    /// GameProgressSaver.ReadFile: the decoded object, or None when missing / unreadable.
    pub fn read(&self, file: &str) -> Option<Value> {
        let b = std::fs::read(self.slot_dir.as_ref()?.join(file)).ok()?;
        crate::nrbf::decode(&b).ok()
    }

    /// GameProgressSaver.GetRank(returnNull: true, lvl): `lvl<n>progress.bepis` if it holds a RankData.
    pub fn rank(&self, level_number: i64) -> Option<Value> {
        self.read(&format!("lvl{level_number}progress.bepis")).filter(|v| v.get("$type").str() == "RankData")
    }

    /// GameProgressSaver's GameProgressMoneyAndGear (`generalprogress.bepis`).
    pub fn general(&self) -> Option<Value> {
        self.read("generalprogress.bepis").filter(|v| v.get("$type").str() == "GameProgressMoneyAndGear")
    }

    /// PlayerPrefs.GetInt(key, default)
    pub fn pp_int(&self, key: &str, default: i64) -> i64 {
        match self.player_prefs.get(key) {
            Some(PlayerPref::Int(v)) => *v,
            _ => default,
        }
    }

    /// PlayerPrefs.GetFloat(key, default)
    pub fn pp_float(&self, key: &str, default: f64) -> f64 {
        match self.player_prefs.get(key) {
            Some(PlayerPref::Float(v)) => *v,
            _ => default,
        }
    }
}

/// Unity's Windows PlayerPrefs: ints are REG_DWORD, floats REG_QWORD (double bits), strings
/// REG_BINARY UTF-8 with a trailing NUL. Read through `reg query`; empty elsewhere or on failure.
fn read_player_prefs() -> HashMap<String, PlayerPref> {
    let mut out = HashMap::new();
    if !cfg!(windows) {
        return out;
    }
    let Ok(o) = std::process::Command::new("reg").args(["query", r"HKCU\Software\Hakita\ULTRAKILL"]).output() else { return out };
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        let Some(line) = line.strip_prefix("    ") else { continue };
        let mut parts = line.splitn(3, "    ");
        let (Some(name), Some(kind), Some(data)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Some(key) = name.rsplit_once("_h").filter(|(_, h)| h.bytes().all(|b| b.is_ascii_digit())).map(|(k, _)| k) else { continue };
        let data = data.trim();
        let v = match kind {
            "REG_DWORD" => i64::from_str_radix(data.trim_start_matches("0x"), 16).ok().map(|v| PlayerPref::Int(v as u32 as i32 as i64)),
            "REG_QWORD" => u64::from_str_radix(data.trim_start_matches("0x"), 16).ok().map(|v| PlayerPref::Float(f64::from_bits(v))),
            "REG_BINARY" => {
                let bytes: Vec<u8> = (0..data.len() / 2).filter_map(|i| u8::from_str_radix(&data[2 * i..2 * i + 2], 16).ok()).collect();
                let s = bytes.split(|&b| b == 0).next().unwrap_or(&[]);
                Some(PlayerPref::Str(String::from_utf8_lossy(s).into_owned()))
            }
            _ => None,
        };
        if let Some(v) = v {
            out.insert(key.to_string(), v);
        }
    }
    out
}
