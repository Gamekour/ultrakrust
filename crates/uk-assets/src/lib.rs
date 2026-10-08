//! Reads ULTRAKILL's Unity data from the user's own install at runtime:
//! UnityFS bundles, SerializedFiles (typetree-driven), meshes, and level scenes.
//! Nothing read here is bundled with ULTRAKRUST.

pub mod addressables;
pub mod anim;
pub mod bundle;
mod common_strings;
pub mod db;
pub mod gltf_map;
pub mod input;
pub mod mesh;
pub mod prefs;
pub mod navmesh;
pub mod nrbf;
pub mod particles;
pub mod reader;
pub mod save;
pub mod scene;
pub mod scenedef;
pub mod sdf;
pub mod serialized;
pub mod shader;
pub mod smolv;
pub mod spirv;
pub mod texture;
pub mod ui;

#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// The ULTRAKILL install: `ULTRAKILL_DIR` if set, else the default Steam folder, else any Steam
/// library listed in Steam's `libraryfolders.vdf`.
pub fn find_install() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    if let Ok(p) = std::env::var("ULTRAKILL_DIR") {
        return Some(p.into());
    }
    let mut steam_roots = vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"C:\Program Files\Steam")];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        steam_roots.extend([home.join(".steam/steam"), home.join(".local/share/Steam")]);
    }
    let mut libraries = steam_roots.clone();
    for root in &steam_roots {
        let Ok(vdf) = std::fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) else { continue };
        // lines like: "path"		"D:\\SteamLibrary"
        for line in vdf.lines() {
            let parts: Vec<&str> = line.split('"').collect();
            if parts.len() >= 4 && parts[1] == "path" {
                libraries.push(PathBuf::from(parts[3].replace("\\\\", "\\")));
            }
        }
    }
    libraries.into_iter().map(|l| l.join("steamapps").join("common").join("ULTRAKILL")).find(|p| p.join("ULTRAKILL_Data").is_dir())
}
