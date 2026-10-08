//! MeshCollider components in a level's scene file that did not become colliders: the node, the
//! component's enabled / convex flags, and where its m_Mesh points (file, path id, whether it loads).
//! dropped_colliders [level2-3]
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or("level2-3".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let files = db.load_bundle_files(&path).unwrap();
    let mut by_mesh = std::collections::BTreeMap::<String, usize>::new();
    for f in &files {
        for o in f.objects.iter().filter(|o| o.class_id == 64) {
            if def.comp_to_collider.contains_key(&o.path_id) {
                continue;
            }
            let Ok(v) = f.read(o) else {
                println!("unreadable MeshCollider {}", o.path_id);
                continue;
            };
            let go = v.get("m_GameObject").pptr().1;
            let node = def.obj_to_node.get(&go).copied();
            let (fi, mid) = v.get("m_Mesh").pptr();
            let target = if fi == 0 { f.name.clone() } else { f.externals.get(fi as usize - 1).map_or(format!("external #{fi}?"), |e| e.file_name().to_string()) };
            let loads = (mid != 0).then(|| {
                db.file(&target).ok().and_then(|tf| tf.object(mid).map(|x| (x.class_id, tf.read(x).ok().map(|m| m.get("m_Name").str().to_string()))))
            });
            println!(
                "{} enabled={} convex={} mesh=({target}, {mid}) -> {:?}",
                node.map_or(format!("go {go} (no node)"), |n| def.path(n)),
                v.get("m_Enabled").i64(),
                v.get("m_Convex").i64(),
                loads
            );
            if std::env::var("RAW").is_ok() {
                println!("  collider: {}", v.compact());
                if let Ok(g) = f.read_id(go) {
                    for c in g.get("m_Component").array() {
                        let cid = c.get("component").pptr().1;
                        if let (Some(co), Ok(cv)) = (f.object(cid), f.read_id(cid)) {
                            if co.class_id != 4 {
                                let s = cv.compact();
                                println!("  class {}: {}", co.class_id, &s[..s.len().min(400)]);
                            }
                        }
                    }
                }
                for c in g_meshes(&f, go) {
                    match f.object(c) {
                        Some(mo) => { let mv = f.read_id(c); println!("  filter mesh {c}: class {} read {:?}", mo.class_id, mv.as_ref().map(|m| (m.get("m_Name").str().to_string(), m.get("m_StreamData").compact(), m.get("m_VertexData").get("m_VertexCount").i64())).map_err(|e| e.to_string())); }
                        None => println!("  filter mesh {c}: not in {}", f.name),
                    }
                }
                println!("  externals: {:?}", f.externals.iter().map(|e| e.file_name().to_string()).collect::<Vec<_>>());
            }
            *by_mesh.entry(format!("{target}:{mid}")).or_default() += 1;
        }
    }
    println!("dropped by mesh: {by_mesh:?}");
}

fn g_meshes(f: &uk_assets::serialized::SerializedFile, go: i64) -> Vec<i64> {
    let Ok(g) = f.read_id(go) else { return vec![] };
    g.get("m_Component").array().iter().filter_map(|c| {
        let cid = c.get("component").pptr().1;
        (f.object(cid)?.class_id == 33).then(|| f.read_id(cid).ok()).flatten().map(|v| v.get("m_Mesh").pptr().1)
    }).collect()
}
