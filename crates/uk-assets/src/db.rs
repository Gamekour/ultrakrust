//! Asset database over an ULTRAKILL install: finds which bundle holds each
//! `CAB-*` serialized file, loads bundles lazily, and resolves PPtrs across files.

use crate::bundle::Bundle;
use crate::serialized::{SerializedFile, Value};
use crate::{Error, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct AssetDb {
    pub install: PathBuf,
    /// serialized file name → bundle path that contains it
    cab_to_bundle: HashMap<String, PathBuf>,
    bundles: HashMap<PathBuf, Arc<Bundle>>,
    files: HashMap<String, Arc<SerializedFile>>,
    /// resource name (e.g. "CAB-x.resS") → bytes
    resources: HashMap<String, Arc<[u8]>>,
}

/// A reference to an object in a specific serialized file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjRef<'a> {
    pub file: &'a str,
    pub path_id: i64,
}

impl AssetDb {
    pub fn data_dir(install: &Path) -> PathBuf {
        install.join("ULTRAKILL_Data")
    }

    pub fn bundle_dir(install: &Path) -> PathBuf {
        Self::data_dir(install).join("StreamingAssets/aa/StandaloneWindows64")
    }

    /// Indexes every bundle header under the Addressables folder (fast: no decompression of data).
    pub fn open(install: &Path) -> Result<Self> {
        let mut cab_to_bundle = HashMap::new();
        let mut stack = vec![Self::bundle_dir(install)];
        while let Some(dir) = stack.pop() {
            let rd = std::fs::read_dir(&dir).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "bundle") {
                    for name in Bundle::list_nodes(&p).unwrap_or_default() {
                        cab_to_bundle.insert(name, p.clone());
                    }
                }
            }
        }
        if cab_to_bundle.is_empty() {
            return Err(Error(format!("no bundles found under {}", install.display())));
        }
        Ok(Self {
            install: install.to_path_buf(),
            cab_to_bundle,
            bundles: HashMap::new(),
            files: HashMap::new(),
            resources: HashMap::new(),
        })
    }

    pub fn bundle_count(&self) -> usize {
        self.cab_to_bundle.values().collect::<std::collections::HashSet<_>>().len()
    }

    fn load_bundle(&mut self, path: &Path) -> Result<Arc<Bundle>> {
        if let Some(b) = self.bundles.get(path) {
            return Ok(b.clone());
        }
        let b = Arc::new(Bundle::open(path)?);
        for n in &b.nodes {
            let data: Arc<[u8]> = Arc::from(b.file(n));
            // SerializedFiles have no extension (or .sharedAssets); everything else is a resource.
            if n.path.ends_with(".resS") || n.path.ends_with(".resource") {
                self.resources.insert(n.path.clone(), data);
            } else if let Ok(sf) = SerializedFile::parse(&n.path, data.clone()) {
                self.files.insert(n.path.clone(), Arc::new(sf));
            } else {
                self.resources.insert(n.path.clone(), data);
            }
        }
        self.bundles.insert(path.to_path_buf(), b.clone());
        Ok(b)
    }

    /// Loads a whole bundle file (e.g. a level) and returns the serialized files inside it.
    pub fn load_bundle_files(&mut self, path: &Path) -> Result<Vec<Arc<SerializedFile>>> {
        let b = self.load_bundle(path)?;
        Ok(b.nodes.iter().filter_map(|n| self.files.get(&n.path).cloned()).collect())
    }

    /// Serialized file by name ("CAB-…", "CAB-….sharedAssets", "unity default resources").
    pub fn file(&mut self, name: &str) -> Result<Arc<SerializedFile>> {
        if let Some(f) = self.files.get(name) {
            return Ok(f.clone());
        }
        if let Some(bp) = self.cab_to_bundle.get(name).cloned() {
            self.load_bundle(&bp)?;
            return self.files.get(name).cloned().ok_or_else(|| Error(format!("{name} not in its bundle")));
        }
        // Loose files next to the player (built-in resources).
        for dir in [Self::data_dir(&self.install).join("Resources"), Self::data_dir(&self.install)] {
            let p = dir.join(name);
            if p.is_file() {
                let data: Arc<[u8]> = Arc::from(std::fs::read(&p).map_err(|e| Error(e.to_string()))?);
                let sf = SerializedFile::parse(name, data)?;
                if !sf.has_typetrees() {
                    // Built-in resources ship without typetrees; borrow them from files of
                    // the same Unity version that are already loaded.
                    let mut map = HashMap::new();
                    for f in self.files.values() {
                        for t in &f.types {
                            if !t.nodes.is_empty() && t.class_id != 114 {
                                map.entry(t.class_id).or_insert_with(|| t.nodes.clone());
                            }
                        }
                    }
                    let _ = sf.fallback.set(map);
                }
                let sf = Arc::new(sf);
                self.files.insert(name.to_string(), sf.clone());
                return Ok(sf);
            }
        }
        Err(Error(format!("serialized file {name} not found")))
    }

    pub fn resource(&mut self, name: &str) -> Option<Arc<[u8]>> {
        if let Some(r) = self.resources.get(name) {
            return Some(r.clone());
        }
        let bp = self.cab_to_bundle.get(name).cloned()?;
        self.load_bundle(&bp).ok()?;
        self.resources.get(name).cloned()
    }

    /// Resolves a PPtr found in `from` to (file, path_id). Returns None for null pointers.
    pub fn resolve(&mut self, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Result<Option<(Arc<SerializedFile>, i64)>> {
        let (file_id, path_id) = pptr;
        if path_id == 0 {
            return Ok(None);
        }
        if file_id == 0 {
            return Ok(Some((from.clone(), path_id)));
        }
        let ext = from
            .externals
            .get(file_id as usize - 1)
            .ok_or_else(|| Error(format!("{}: bad file id {file_id}", from.name)))?;
        let name = ext.file_name().to_string();
        Ok(Some((self.file(&name)?, path_id)))
    }

    pub fn read_pptr(&mut self, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Result<Option<(Arc<SerializedFile>, i64, Value)>> {
        match self.resolve(from, pptr)? {
            None => Ok(None),
            Some((f, id)) => {
                let v = f.read_id(id)?;
                Ok(Some((f, id, v)))
            }
        }
    }
}
