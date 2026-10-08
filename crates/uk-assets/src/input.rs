//! The Input System's InputActionAsset (ULTRAKILL's `InputActions`), the player's rebinds from
//! `Preferences/Binds.json` (JsonBindingMap.ApplyTo, read-only) and InputManager.GetBindingString,
//! the key names the HUD hints print.

use crate::serialized::Value;
use std::path::Path;

const FLAG_COMPOSITE: i64 = 4;
const FLAG_PART_OF_COMPOSITE: i64 = 8;
pub const KEYBOARD_MOUSE: &str = "Keyboard & Mouse";

#[derive(Clone, Debug, Default)]
pub struct Binding {
    pub name: String,
    pub path: String,
    pub groups: Vec<String>,
    pub composite: bool,
    pub part: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Action {
    pub map: String,
    pub name: String,
    pub id: String,
    pub bindings: Vec<Binding>,
}

#[derive(Clone, Debug, Default)]
pub struct InputActions {
    pub actions: Vec<Action>,
}

/// One entry of Binds.json's `modifiedActions`.
#[derive(Clone, Debug)]
pub struct JsonBinding {
    pub path: String,
    pub composite: bool,
    pub parts: Vec<(String, String)>,
}

/// JsonBindingMap: Binds.json (controlScheme + modifiedActions).
#[derive(Clone, Debug, Default)]
pub struct BindMap {
    pub control_scheme: String,
    pub actions: Vec<(String, Vec<JsonBinding>)>,
}

impl BindMap {
    /// A missing or malformed file is no overrides, like InputManager (which only loads it when it exists).
    pub fn load(install: &Path) -> BindMap {
        std::fs::read_to_string(install.join("Preferences").join("Binds.json")).ok().map(|s| BindMap::parse(&s)).unwrap_or_default()
    }

    pub fn parse(s: &str) -> BindMap {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(s) else { return BindMap::default() };
        let control_scheme = v.get("controlScheme").and_then(|x| x.as_str()).unwrap_or_default().to_string();
        let mut actions = Vec::new();
        if let Some(m) = v.get("modifiedActions").and_then(|x| x.as_object()) {
            for (name, list) in m {
                let binds = list
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .map(|b| JsonBinding {
                                path: b.get("path").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
                                composite: b.get("isComposite").and_then(|x| x.as_bool()).unwrap_or(false),
                                parts: b
                                    .get("parts")
                                    .and_then(|x| x.as_object())
                                    .map(|p| p.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect())
                                    .unwrap_or_default(),
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                actions.push((name.clone(), binds));
            }
        }
        BindMap { control_scheme, actions }
    }
}

/// JsonBindingMap.bindAliases
fn bind_alias(name: &str) -> &str {
    match name {
        "Slot 1" => "Revolver",
        "Slot 2" => "Shotgun",
        "Slot 3" => "Nailgun",
        "Slot 4" => "Railcannon",
        "Slot 5" => "Rocket Launcher",
        "Change Variation" => "Next Variation",
        "Last Weapon" => "Last Used Weapon",
        n => n,
    }
}

impl InputActions {
    /// From the InputActionAsset object.
    pub fn read(asset: &Value) -> InputActions {
        let mut actions = Vec::new();
        for m in asset.get("m_ActionMaps").array() {
            let map = m.get("m_Name").str();
            let first = actions.len();
            for a in m.get("m_Actions").array() {
                actions.push(Action { map: map.to_string(), name: a.get("m_Name").str().to_string(), id: a.get("m_Id").str().to_string(), bindings: Vec::new() });
            }
            for b in m.get("m_Bindings").array() {
                let action = b.get("m_Action").str();
                let flags = b.get("m_Flags").i64();
                let binding = Binding {
                    name: b.get("m_Name").str().to_string(),
                    path: b.get("m_Path").str().to_string(),
                    groups: b.get("m_Groups").str().split(';').filter(|g| !g.is_empty()).map(str::to_string).collect(),
                    composite: flags & FLAG_COMPOSITE != 0,
                    part: flags & FLAG_PART_OF_COMPOSITE != 0,
                };
                // m_Action is the action's name (or id) within this map
                if let Some(a) = actions[first..].iter_mut().find(|a| a.name.eq_ignore_ascii_case(action) || a.id == action) {
                    a.bindings.push(binding);
                }
            }
        }
        InputActions { actions }
    }

    /// InputActionAsset.FindAction(nameOrId): an id, "Map/Action" or an action name.
    pub fn find(&self, name_or_id: &str) -> Option<usize> {
        let id = name_or_id.trim_matches(|c| c == '{' || c == '}');
        if let Some(i) = self.actions.iter().position(|a| a.id.eq_ignore_ascii_case(id)) {
            return Some(i);
        }
        if let Some((m, n)) = name_or_id.split_once('/') {
            return self.actions.iter().position(|a| a.map.eq_ignore_ascii_case(m) && a.name.eq_ignore_ascii_case(n));
        }
        self.actions.iter().position(|a| a.name.eq_ignore_ascii_case(name_or_id))
    }

    /// JsonBindingMap.ApplyTo: each modified action loses its bindings in the map's control scheme
    /// (WipeAction) and gets the saved ones appended in that group.
    pub fn apply(&mut self, map: &BindMap) {
        let scheme = map.control_scheme.as_str();
        for (name, binds) in &map.actions {
            let Some(i) = self.find(bind_alias(name)) else {
                // "Action ... does not exist", and ApplyTo breaks out of the loop
                break;
            };
            let a = &mut self.actions[i];
            // WipeAction: ChangeBindingWithGroup(scheme).Erase() until none is left (erasing a
            // composite erases its parts)
            let mut k = 0;
            while k < a.bindings.len() {
                if a.bindings[k].groups.iter().any(|g| g == scheme) {
                    let mut end = k + 1;
                    if a.bindings[k].composite {
                        while end < a.bindings.len() && a.bindings[end].part {
                            end += 1;
                        }
                    }
                    a.bindings.drain(k..end);
                } else {
                    k += 1;
                }
            }
            for b in binds {
                if b.composite {
                    if b.parts.is_empty() {
                        continue;
                    }
                    a.bindings.push(Binding { name: b.path.clone(), path: b.path.clone(), groups: vec![scheme.to_string()], composite: true, part: false });
                    for (n, p) in &b.parts {
                        a.bindings.push(Binding { name: n.clone(), path: p.clone(), groups: vec![scheme.to_string()], composite: false, part: true });
                    }
                } else {
                    a.bindings.push(Binding { name: String::new(), path: b.path.clone(), groups: vec![scheme.to_string()], composite: false, part: false });
                }
            }
        }
    }

    /// InputManager.GetBindingString with the keyboard and mouse as the only devices and the last
    /// button device a keyboard / mouse (the Keyboard & Mouse scheme).
    pub fn binding_string(&self, name_or_id: &str) -> String {
        let Some(i) = self.find(name_or_id) else { return String::new() };
        let b = &self.actions[i].bindings;
        let mut composite = 0;
        for (i, x) in b.iter().enumerate() {
            if x.composite {
                composite = i;
                continue;
            }
            // InputSystem.FindControl(path) == null: no such device connected
            let Some(device) = device_of(&x.path) else { continue };
            if x.part {
                let mut text = String::new();
                for j in composite + 1..b.len() {
                    if !b[j].part {
                        break;
                    }
                    if j > composite + 1 {
                        text += " + ";
                    }
                    text += &display_string(&b[j].path);
                }
                return text;
            }
            if device == "Keyboard" || device == "Mouse" {
                return display_string(&x.path);
            }
        }
        String::new()
    }
}

/// The connected device a control path names (keyboard and mouse only).
fn device_of(path: &str) -> Option<&'static str> {
    let dev = path.strip_prefix('<')?.split('>').next()?;
    match dev {
        "Keyboard" => Some("Keyboard"),
        "Mouse" | "Pointer" => Some("Mouse"),
        _ => None,
    }
}

/// InputBinding.ToDisplayString(): InputControlPath.ToHumanReadableString with OmitDevice and
/// UseShortNames, from the device layouts' displayName / shortDisplayName (no control instance).
pub fn display_string(path: &str) -> String {
    let Some(rest) = path.strip_prefix('<') else { return path.to_string() };
    let Some((dev, controls)) = rest.split_once(">/") else { return path.to_string() };
    controls.split('/').enumerate().map(|(i, c)| control_name(dev, c, i)).collect::<Vec<_>>().join("/")
}

fn control_name(dev: &str, c: &str, depth: usize) -> String {
    let fixed = match (dev, depth, c) {
        ("Mouse" | "Pointer", 0, "leftButton") => "LMB",
        ("Mouse" | "Pointer", 0, "rightButton") => "RMB",
        ("Mouse" | "Pointer", 0, "middleButton") => "MMB",
        ("Mouse", 0, "forwardButton") => "Forward",
        ("Mouse", 0, "backButton") => "Back",
        ("Mouse" | "Pointer", 0, "scroll") => "Scroll",
        ("Mouse" | "Pointer", 0, "delta") => "Delta",
        ("Mouse" | "Pointer", 0, "position") => "Position",
        ("Mouse" | "Pointer", 1, "up") => "Up",
        ("Mouse" | "Pointer", 1, "down") => "Down",
        ("Mouse" | "Pointer", 1, "left") => "Left",
        ("Mouse" | "Pointer", 1, "right") => "Right",
        ("Keyboard", 0, k) => match k {
            "space" => "Space",
            "enter" => "Enter",
            "tab" => "Tab",
            "backquote" => "`",
            "quote" => "'",
            "semicolon" => ";",
            "comma" => ",",
            "period" => ".",
            "slash" => "/",
            "backslash" => "\\",
            "leftBracket" => "[",
            "rightBracket" => "]",
            "minus" => "-",
            "equals" => "=",
            "leftShift" => "LShift",
            "rightShift" => "RShift",
            "leftAlt" => "LAlt",
            "rightAlt" => "RAlt",
            "leftCtrl" => "LCtrl",
            "rightCtrl" => "RCtrl",
            "leftMeta" => "LSys",
            "rightMeta" => "RSys",
            "contextMenu" => "Context Menu",
            "escape" => "Esc",
            "leftArrow" => "Left Arrow",
            "rightArrow" => "Right Arrow",
            "upArrow" => "Up Arrow",
            "downArrow" => "Down Arrow",
            "backspace" => "Backspace",
            "pageDown" => "Page Down",
            "pageUp" => "Page Up",
            "home" => "Home",
            "end" => "End",
            "insert" => "Insert",
            "delete" => "Del",
            "capsLock" => "Caps Lock",
            "numLock" => "Num Lock",
            "printScreen" => "Print Screen",
            "scrollLock" => "Scroll Lock",
            "pause" => "Pause/Break",
            "numpadEnter" => "Numpad Enter",
            "numpadDivide" => "Numpad /",
            "numpadMultiply" => "Numpad *",
            "numpadPlus" => "Numpad +",
            "numpadMinus" => "Numpad -",
            "numpadPeriod" => "Numpad .",
            "numpadEquals" => "Numpad =",
            k if k.len() == 1 => return k.to_uppercase(),
            k if k.starts_with('f') && k[1..].parse::<u32>().is_ok() => return k.to_uppercase(),
            k if k.starts_with("numpad") && k[6..].parse::<u32>().is_ok() => return format!("Numpad {}", &k[6..]),
            k if k.starts_with("digit") => return k[5..].to_string(),
            k => k,
        },
        _ => c,
    };
    fixed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset() -> InputActions {
        let b = |path: &str, groups: &[&str], composite: bool, part: bool| Binding {
            name: String::new(),
            path: path.into(),
            groups: groups.iter().map(|g| g.to_string()).collect(),
            composite,
            part,
        };
        InputActions {
            actions: vec![
                Action { map: "Weapon".into(), name: "Secondary Fire".into(), id: "15bebcb7".into(), bindings: vec![b("<Gamepad>/leftTrigger", &["Gamepad"], false, false), b("<Mouse>/rightButton", &[KEYBOARD_MOUSE], false, false)] },
                Action { map: "Movement".into(), name: "Slide".into(), id: "624c1b28".into(), bindings: vec![b("<Keyboard>/leftCtrl", &[KEYBOARD_MOUSE], false, false)] },
                Action {
                    map: "Movement".into(),
                    name: "Move".into(),
                    id: "cb0ce271".into(),
                    bindings: vec![
                        b("2DVector", &[], true, false),
                        b("<Keyboard>/w", &[KEYBOARD_MOUSE], false, true),
                        b("<Keyboard>/s", &[KEYBOARD_MOUSE], false, true),
                    ],
                },
                Action { map: "Fist".into(), name: "Punch".into(), id: "869113fb".into(), bindings: vec![b("<Keyboard>/f", &[KEYBOARD_MOUSE], false, false)] },
            ],
        }
    }

    #[test]
    fn binding_strings() {
        let mut a = asset();
        assert_eq!(a.binding_string("15bebcb7"), "RMB");
        assert_eq!(a.binding_string("Movement/Slide"), "LCtrl");
        assert_eq!(a.binding_string("Move"), "W + S");
        let map = BindMap::parse(r#"{"controlScheme":"Keyboard & Mouse","modifiedActions":{"Punch":[],"Slide":[{"path":"<Mouse>/backButton"}]}}"#);
        a.apply(&map);
        assert_eq!(a.binding_string("Punch"), "");
        assert_eq!(a.binding_string("Slide"), "Back");
    }
}
