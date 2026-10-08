//! Particle inventory: per scene, the ParticleSystems placed in the scene and the effect prefabs
//! that ported scripts (and DefaultReferenceManager) reference, with what each needs from the
//! simulation: unsupported modules, renderer modes, and the other components in the prefab.
//! cargo run --release -p uk-game --example particle_inventory -- [scene filter] [--prefabs-only]
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::Arc;
use uk_assets::db::AssetDb;
use uk_assets::particles::ParticleSystemDef;
use uk_assets::serialized::{SerializedFile, Value};
use uk_assets::{addressables::Catalog, scenedef};

fn class_label(db: &mut AssetDb, f: &Arc<SerializedFile>, id: i64) -> String {
    let Some(o) = f.object(id) else { return "?".into() };
    match o.class_id {
        114 => {
            let Ok(v) = f.read_id(id) else { return "MonoBehaviour".into() };
            db.read_pptr(f, v.get("m_Script").pptr()).ok().flatten().map(|(_, _, s)| s.get("m_ClassName").str().to_string()).unwrap_or("MonoBehaviour".into())
        }
        4 => "Transform".into(),
        23 => "MeshRenderer".into(),
        33 => "MeshFilter".into(),
        54 => "Rigidbody".into(),
        65 => "BoxCollider".into(),
        82 => "AudioSource".into(),
        95 => "Animator".into(),
        96 => "TrailRenderer".into(),
        108 => "Light".into(),
        111 => "Animation".into(),
        120 => "LineRenderer".into(),
        135 => "SphereCollider".into(),
        136 => "CapsuleCollider".into(),
        137 => "SkinnedMeshRenderer".into(),
        198 => "ParticleSystem".into(),
        199 => "ParticleSystemRenderer".into(),
        212 => "SpriteRenderer".into(),
        224 => "RectTransform".into(),
        c => format!("class{c}"),
    }
}

#[derive(Default)]
struct PrefabInfo {
    name: String,
    systems: usize,
    unsupported: BTreeSet<String>,
    render_modes: BTreeSet<u8>,
    components: BTreeSet<String>,
    nodes: usize,
}

fn walk_prefab(db: &mut AssetDb, f: &Arc<SerializedFile>, go: i64) -> PrefabInfo {
    let mut info = PrefabInfo::default();
    let mut stack = vec![go];
    while let Some(g) = stack.pop() {
        let Ok(v) = f.read_id(g) else { continue };
        if info.nodes == 0 {
            info.name = v.get("m_Name").str().to_string();
        }
        info.nodes += 1;
        for c in v.get("m_Component").array() {
            let (_, cid) = c.get("component").pptr();
            let Some(o) = f.object(cid) else { continue };
            let class = o.class_id;
            let label = class_label(db, f, cid);
            let Ok(cv) = f.read_id(cid) else { continue };
            match class {
                4 | 224 => {
                    for k in cv.get("m_Children").array() {
                        if let Ok(t) = f.read_id(k.pptr().1) {
                            stack.push(t.get("m_GameObject").pptr().1);
                        }
                    }
                }
                198 => {
                    info.systems += 1;
                    info.unsupported.extend(ParticleSystemDef::read(&cv).unsupported);
                }
                199 => {
                    if cv.get("m_Enabled").bool() {
                        info.render_modes.insert(cv.get("m_RenderMode").i64() as u8);
                    }
                }
                _ => {}
            }
            if !matches!(class, 4 | 224 | 198 | 199) {
                info.components.insert(label);
            }
        }
    }
    info
}

/// Every PPtr / AssetReference in a script's fields: (field path, target).
enum Ref {
    Ptr((i32, i64)),
    Guid(String),
}

fn refs(v: &Value, path: &str, out: &mut Vec<(String, Ref)>, depth: usize) {
    if depth > 4 {
        return;
    }
    match v {
        Value::Struct(fields) => {
            if fields.iter().any(|(k, _)| &**k == "m_PathID") {
                let p = v.pptr();
                if p.1 != 0 {
                    out.push((path.to_string(), Ref::Ptr(p)));
                }
                return;
            }
            if let Some((_, g)) = fields.iter().find(|(k, _)| &**k == "m_AssetGUID") {
                if !g.str().is_empty() {
                    out.push((path.to_string(), Ref::Guid(g.str().to_string())));
                }
                return;
            }
            for (k, f) in fields {
                if path.is_empty() && (k.starts_with("m_") || &**k == "m_Script") {
                    continue;
                }
                refs(f, &if path.is_empty() { k.to_string() } else { format!("{path}.{k}") }, out, depth + 1);
            }
        }
        Value::Array(a) => {
            for x in a {
                refs(x, &format!("{path}[]"), out, depth + 1);
            }
        }
        _ => {}
    }
}

fn main() {
    let filter = std::env::args().nth(1).unwrap_or_default();
    let prefabs_only = std::env::args().any(|a| a == "--prefabs-only");
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let catalog = Catalog::load(&install).ok();
    let dir = AssetDb::bundle_dir(&install);
    let mut bundles: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap().to_string_lossy();
            n.starts_with("campaign_scenes_") && n.contains(&*filter)
        })
        .collect();
    bundles.sort();
    let mut extra: HashSet<&str> = HashSet::new();
    extra.extend(["DefaultReferenceManager", "BloodsplatterManager", "RevolverBeam", "Explosion"]);
    // (class.field) -> prefab key -> info
    let mut fields: BTreeMap<String, BTreeMap<(String, i64), String>> = BTreeMap::new();
    let mut prefabs: BTreeMap<(String, i64), PrefabInfo> = BTreeMap::new();
    let mut scene_unsupported: BTreeMap<String, usize> = BTreeMap::new();
    let mut scene_modes: BTreeMap<u8, usize> = BTreeMap::new();
    let mut scene_total = 0usize;
    for b in &bundles {
        let name = b.file_name().unwrap().to_string_lossy().replace("campaign_scenes_", "").replace(".bundle", "");
        let def = match scenedef::load_scene(&mut db, b) {
            Ok(d) => d,
            Err(e) => {
                println!("{name}: load failed {e}");
                continue;
            }
        };
        // scene-placed systems
        let files = db.load_bundle_files(b).unwrap();
        let (mut n, mut un) = (0, 0);
        if !prefabs_only {
            for f in &files {
                for o in f.objects.iter().filter(|o| o.class_id == 198 || o.class_id == 199) {
                    let Ok(v) = f.read(o) else { continue };
                    if o.class_id == 199 {
                        if v.get("m_Enabled").bool() {
                            *scene_modes.entry(v.get("m_RenderMode").i64() as u8).or_default() += 1;
                        }
                        continue;
                    }
                    n += 1;
                    let d = ParticleSystemDef::read(&v);
                    if !d.unsupported.is_empty() {
                        un += 1;
                    }
                    for u in d.unsupported {
                        *scene_unsupported.entry(u).or_default() += 1;
                    }
                }
            }
            scene_total += n;
        }
        // prefabs referenced by ported scripts
        let mut found = 0;
        for s in &def.scripts {
            if uk_game::parity::status(&s.class).is_none() && !extra.contains(s.class.as_str()) {
                continue;
            }
            let from = match &s.file {
                Some(f) => db.file(f).ok(),
                None => files.iter().find(|f| f.objects.iter().any(|o| o.class_id == 1)).cloned(),
            };
            let Some(from) = from else { continue };
            let mut rs = Vec::new();
            refs(&s.data, "", &mut rs, 0);
            for (field, r) in rs {
                let target = match r {
                    Ref::Ptr(p) => db.resolve(&from, p).ok().flatten(),
                    Ref::Guid(g) => catalog.as_ref().and_then(|c| c.load_asset(&mut db, &g).ok()),
                };
                let Some((f, id)) = target else { continue };
                // a component -> its GameObject
                let Some(o) = f.object(id) else { continue };
                let go = match o.class_id {
                    1 => id,
                    198 | 199 | 4 | 114 => match f.read_id(id) {
                        Ok(v) => v.get("m_GameObject").pptr().1,
                        Err(_) => continue,
                    },
                    _ => continue,
                };
                // scene objects: listed separately (they run in place)
                if files.iter().any(|x| Arc::ptr_eq(x, &f)) {
                    continue;
                }
                let key = (f.name.clone(), go);
                if !prefabs.contains_key(&key) {
                    let info = walk_prefab(&mut db, &f, go);
                    prefabs.insert(key.clone(), info);
                }
                if prefabs[&key].systems == 0 {
                    continue;
                }
                found += 1;
                fields.entry(format!("{}.{field}", s.class)).or_default().insert(key, name.clone());
            }
        }
        println!("{name}: {n} scene systems ({un} with unsupported features), {found} prefab refs from ported scripts");
    }
    if !prefabs_only {
        println!("\nscene systems {scene_total}; unsupported features (systems): {scene_unsupported:#?}\nrenderer modes (enabled renderers): {scene_modes:?}");
    }
    println!("\n=== effect prefabs referenced by ported scripts");
    for (field, ps) in &fields {
        for (key, scene) in ps {
            let p = &prefabs[key];
            println!("{field} -> {} ({} #{}, first in {scene}): {} nodes, {} systems, modes {:?}, unsupported {:?}, other {:?}", p.name, key.0, key.1, p.nodes, p.systems, p.render_modes, p.unsupported, p.components);
        }
    }
}
