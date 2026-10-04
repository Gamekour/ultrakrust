//! Reads ULTRAKILL's Unity data from the user's own install at runtime:
//! UnityFS bundles, SerializedFiles (typetree-driven), meshes, and level scenes.
//! Nothing read here is bundled with ULTRAKRUST.

pub mod bundle;
mod common_strings;
pub mod db;
pub mod mesh;
pub mod reader;
pub mod scene;
pub mod serialized;
pub mod texture;

#[derive(Debug, Clone)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

/// Default Steam install location; override with the `ULTRAKILL_DIR` environment variable.
pub fn find_install() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("ULTRAKILL_DIR") {
        return Some(p.into());
    }
    let default = std::path::PathBuf::from(r"C:\Program Files (x86)\Steam\steamapps\common\ULTRAKILL");
    default.join("ULTRAKILL_Data").is_dir().then_some(default)
}
