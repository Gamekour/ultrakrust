//! The BloodsplatterManager's gore prefabs (head, limb, body, small, smallest, splatter,
//! underwater, sand, ...): each prefab's hierarchy with its components, and for every
//! ParticleSystem / ParticleSystemRenderer the full serialized fields (GORE_FULL=1) or the
//! enabled modules.
//! cargo run --release -p uk-assets --example gore_dump -- [level0-1] [field filter] [script class]
//! (with a script class, the fields of its first instance are dumped instead, e.g.
//! `gore_dump level1-2 dripBlood,woundedParticle MaliciousFace`)
use std::sync::Arc;
use uk_assets::db::AssetDb;
use uk_assets::serialized::{SerializedFile, Value};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let only: Option<Vec<String>> = std::env::args().nth(2).map(|o| o.split(',').map(str::to_string).collect());
    let class = std::env::args().nth(3);
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let files = db.load_bundle_files(&AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"))).unwrap();
    let mut mgr = None;
    for f in &files {
        for o in &f.objects {
            if o.class_id != 114 {
                continue;
            }
            let Ok(v) = f.read(o) else { continue };
            let hit = match &class {
                Some(c) => mgr.is_none() && class_name(&mut db, f, o.path_id, &v) == *c,
                None => v.has("splatterClip") && v.has("chestExplosion"),
            };
            if hit {
                mgr = Some((f.clone(), v));
            }
        }
    }
    let (scene, mgr) = mgr.expect("BloodsplatterManager");
    let all = ["head", "limb", "body", "small", "smallest", "splatter", "underwater", "sand", "blessing", "chestExplosion"].map(str::to_string);
    let fields: Vec<String> = match (&class, &only) {
        (Some(_), Some(o)) => o.clone(),
        (_, o) => all.into_iter().filter(|n| o.as_ref().is_none_or(|o| o.contains(n))).collect(),
    };
    for name in &fields {
        let name = name.as_str();
        let Ok(Some((f, go))) = db.resolve(&scene, mgr.get(name).pptr()) else {
            println!("{name}: null");
            continue;
        };
        println!("=== {name} ({} #{go})", f.name);
        dump_go(&mut db, &f, go, 1);
    }
}

fn class_name(db: &mut AssetDb, f: &Arc<SerializedFile>, id: i64, v: &Value) -> String {
    let cid = f.object(id).map(|o| o.class_id).unwrap_or(-1);
    if cid == 114 {
        if let Ok(Some((_, _, s))) = db.read_pptr(f, v.get("m_Script").pptr()) {
            return s.get("m_ClassName").str().to_string();
        }
    }
    match cid {
        1 => "GameObject",
        4 => "Transform",
        135 => "SphereCollider",
        198 => "ParticleSystem",
        199 => "ParticleSystemRenderer",
        82 => "AudioSource",
        212 => "SpriteRenderer",
        23 => "MeshRenderer",
        33 => "MeshFilter",
        54 => "Rigidbody",
        108 => "Light",
        _ => return format!("class{cid}"),
    }
    .to_string()
}

fn dump_go(db: &mut AssetDb, f: &Arc<SerializedFile>, go: i64, depth: usize) {
    let v = f.read_id(go).unwrap();
    let ind = "  ".repeat(depth);
    println!("{ind}{} active {} layer {} tag {}", v.get("m_Name").str(), v.get("m_IsActive").bool(), v.get("m_Layer").i64(), v.get("m_Tag").i64());
    let full = std::env::var_os("GORE_FULL").is_some();
    let mut kids = Vec::new();
    for c in v.get("m_Component").array() {
        let (_, cid) = c.get("component").pptr();
        let cv = f.read_id(cid).unwrap();
        let cn = class_name(db, f, cid, &cv);
        match cn.as_str() {
            "Transform" => {
                println!("{ind}  Transform pos {:?} rot {:?} scale {:?}", cv.get("m_LocalPosition").vec3(), cv.get("m_LocalRotation").quat(), cv.get("m_LocalScale").vec3());
                for k in cv.get("m_Children").array() {
                    kids.push(f.read_id(k.pptr().1).unwrap().get("m_GameObject").pptr().1);
                }
            }
            "ParticleSystem" => {
                println!("{ind}  ParticleSystem");
                if full {
                    println!("{}", pretty(&cv, depth + 2));
                } else if let Value::Struct(fs) = &cv {
                    for (k, m) in fs {
                        if m.has("enabled") && !m.get("enabled").bool() {
                            continue;
                        }
                        println!("{ind}    {k}: {}", m.compact().chars().take(400).collect::<String>());
                    }
                }
            }
            "ParticleSystemRenderer" => {
                let mats: Vec<String> = cv
                    .get("m_Materials")
                    .array()
                    .iter()
                    .map(|m| match db.read_pptr(f, m.pptr()) {
                        Ok(Some((mf, _, mv))) => {
                            let sh = db.read_pptr(&mf, mv.get("m_Shader").pptr()).ok().flatten().map(|(_, _, s)| s.get("m_ParsedForm").get("m_Name").str().to_string()).unwrap_or_default();
                            format!("{} [{sh}]", mv.get("m_Name").str())
                        }
                        _ => "null".into(),
                    })
                    .collect();
                println!("{ind}  ParticleSystemRenderer enabled {} mats {mats:?}", cv.get("m_Enabled").bool());
                if full {
                    println!("{}", pretty(&cv, depth + 2));
                } else {
                    for k in ["m_RenderMode", "m_SortMode", "m_MinParticleSize", "m_MaxParticleSize", "m_CameraVelocityScale", "m_VelocityScale", "m_LengthScale", "m_SortingFudge", "m_NormalDirection", "m_RenderAlignment", "m_Pivot", "m_Flip", "m_UseCustomVertexStreams", "m_VertexStreams", "m_Mesh", "m_ShadowBias", "m_AllowRoll", "m_FreeformStretching", "m_RotateWithStretchDirection", "m_MaskInteraction", "m_SortingOrder", "m_SortingLayerID", "m_ApplyActiveColorSpace"] {
                        if cv.has(k) {
                            println!("{ind}    {k}: {}", cv.get(k).compact());
                        }
                    }
                }
            }
            _ => {
                let s = cv.compact();
                let s = s.find("m_Name").map_or(s.as_str(), |i| &s[i..]);
                println!("{ind}  {cn} {}", s.chars().take(500).collect::<String>());
            }
        }
    }
    for k in kids {
        dump_go(db, f, k, depth + 1);
    }
}

fn pretty(v: &Value, depth: usize) -> String {
    let ind = "  ".repeat(depth);
    match v {
        Value::Struct(fs) => fs.iter().map(|(k, m)| match m {
            Value::Struct(_) => format!("{ind}{k}:\n{}", pretty(m, depth + 1)),
            _ => format!("{ind}{k}: {}", m.compact()),
        }).collect::<Vec<_>>().join("\n"),
        _ => format!("{ind}{}", v.compact()),
    }
}
