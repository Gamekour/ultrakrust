//! The Addressables content catalog (`StreamingAssets/aa/catalog.json`): resolves the asset GUIDs
//! scripts hold in `AssetReference` fields (`m_AssetGUID`) to the bundle that contains the asset
//! and its key in that bundle's `AssetBundle.m_Container`, then to the root object.

use crate::db::AssetDb;
use crate::serialized::SerializedFile;
use crate::{Error, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const CLASS_ASSET_BUNDLE: i32 = 142;

pub struct Catalog {
    /// primary key (GUID or address) -> (bundle file name, internal id = m_Container key)
    entries: HashMap<String, (String, String)>,
}

/// A JSON string value of a top-level key (the catalog's fields are flat, so no full parser).
fn json_str<'a>(src: &'a str, key: &str) -> Option<&'a str> {
    let at = src.find(&format!("\"{key}\""))? + key.len() + 2;
    let rest = &src[at..];
    let open = rest.find('"')? + 1;
    let close = rest[open..].find('"')?;
    Some(&rest[open..open + close])
}

/// A JSON array of strings (with `\\` escapes, as in the catalog's paths).
fn json_str_array(src: &str, key: &str) -> Option<Vec<String>> {
    let at = src.find(&format!("\"{key}\""))?;
    let rest = &src[at + key.len() + 2..];
    let rest = &rest[rest.find('[')? + 1..];
    let mut out = Vec::new();
    let mut chars = rest.chars();
    loop {
        match chars.next()? {
            ']' => return Some(out),
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => s.push(chars.next()?),
                        c => s.push(c),
                    }
                }
                out.push(s);
            }
            _ => {}
        }
    }
}

fn base64(s: &str) -> Vec<u8> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut out, mut acc, mut bits) = (Vec::with_capacity(s.len() * 3 / 4), 0u32, 0);
    for v in s.bytes().filter_map(val) {
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

impl Catalog {
    pub fn path(install: &Path) -> PathBuf {
        AssetDb::data_dir(install).join("StreamingAssets/aa/catalog.json")
    }

    pub fn load(install: &Path) -> Result<Catalog> {
        let p = Self::path(install);
        let src = std::fs::read_to_string(&p).map_err(|e| Error(format!("{}: {e}", p.display())))?;
        let bad = || Error("catalog.json: unexpected layout".into());
        let ids = json_str_array(&src, "m_InternalIds").ok_or_else(bad)?;
        let kd = base64(json_str(&src, "m_KeyDataString").ok_or_else(bad)?);
        let bd = base64(json_str(&src, "m_BucketDataString").ok_or_else(bad)?);
        let ed = base64(json_str(&src, "m_EntryDataString").ok_or_else(bad)?);
        // key data: type byte, then 0 = ascii / 1 = utf-16 string with i32 length (others: not names)
        let key = |o: usize| -> Option<String> {
            let n = i32_at(&kd, o + 1) as usize;
            let b = kd.get(o + 5..o + 5 + n)?;
            match kd[o] {
                0 => Some(String::from_utf8_lossy(b).into_owned()),
                1 => Some(String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>())),
                _ => None,
            }
        };
        // buckets: (key offset, entries); entries: 7 i32 (internal id, provider, dependency key, dep hash, data, primary key, type)
        let mut buckets = Vec::new();
        let mut o = 4;
        for _ in 0..i32_at(&bd, 0) {
            let (ko, n) = (i32_at(&bd, o) as usize, i32_at(&bd, o + 4) as usize);
            let es: Vec<usize> = (0..n).map(|k| i32_at(&bd, o + 8 + 4 * k) as usize).collect();
            o += 8 + 4 * n;
            buckets.push((ko, es));
        }
        let entry = |e: usize| -> [i32; 7] { std::array::from_fn(|k| i32_at(&ed, 4 + 28 * e + 4 * k)) };
        let tail = |s: &str| s.rsplit(['\\', '/']).next().unwrap_or(s).to_string();
        let mut entries = HashMap::new();
        for (ko, es) in &buckets {
            let Some(k) = key(*ko) else { continue };
            for &e in es {
                let [iid, _, dep, ..] = entry(e);
                if dep < 0 {
                    continue;
                }
                // the asset's own bundle is the first of its dependency bucket
                let Some(&b0) = buckets.get(dep as usize).and_then(|b| b.1.first()) else { continue };
                let bundle = tail(&ids[entry(b0)[0] as usize]);
                if bundle.ends_with(".bundle") {
                    entries.entry(k.clone()).or_insert((bundle, ids[iid as usize].clone()));
                }
            }
        }
        Ok(Catalog { entries })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// (bundle file name, container key) of a GUID or address.
    pub fn locate(&self, key: &str) -> Option<&(String, String)> {
        self.entries.get(key)
    }

    /// The asset (usually a prefab's root GameObject) a GUID or address refers to.
    pub fn load_asset(&self, db: &mut AssetDb, key: &str) -> Result<(Arc<SerializedFile>, i64)> {
        let (bundle, inner) = self.locate(key).ok_or_else(|| Error(format!("{key}: not in catalog")))?;
        let path = AssetDb::bundle_dir(&db.install).join(bundle);
        let map = match db.containers.get(&path) {
            Some(m) => m.clone(),
            None => {
                let mut m = std::collections::HashMap::new();
                for f in db.load_bundle_files(&path)? {
                    for o in f.objects.iter().filter(|o| o.class_id == CLASS_ASSET_BUNDLE) {
                        for c in f.read(o)?.get("m_Container").array() {
                            m.entry(c.get("first").str().to_ascii_lowercase()).or_insert((f.clone(), c.get("second").get("asset").pptr()));
                        }
                    }
                }
                let m = std::sync::Arc::new(m);
                db.containers.insert(path, m.clone());
                m
            }
        };
        if let Some((f, ptr)) = map.get(&inner.to_ascii_lowercase()) {
            return match db.resolve(f, *ptr)? {
                Some(r) => Ok(r),
                None => Err(Error(format!("{key}: null asset"))),
            };
        }
        Err(Error(format!("{key}: not in {bundle}'s container")))
    }
}
