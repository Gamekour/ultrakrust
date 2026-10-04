//! Structure of one Shader object (by name, found through level 0-1's materials): top-level fields,
//! per-platform blob layout, subshader/pass/program counts. Big byte arrays are summarised.
//! shader_info [shader name]   (default ULTRAKILL/Master)
use uk_assets::db::AssetDb;
use uk_assets::serialized::Value;

fn shape(v: &Value, depth: usize, out: &mut String) {
    match v {
        Value::Struct(f) => {
            out.push('{');
            for (k, x) in f {
                out.push_str(k);
                out.push(':');
                if depth > 0 { shape(x, depth - 1, out) } else { out.push('…') }
                out.push(',');
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push_str(&format!("[{}]", a.len()));
            if let Some(x) = a.first() { if depth > 0 { shape(x, depth - 1, out) } }
        }
        Value::Bytes(b) => out.push_str(&format!("<{} bytes>", b.len())),
        x => out.push_str(&format!("{x:?}").chars().take(40).collect::<String>()),
    }
}

fn main() {
    let want = std::env::args().nth(1).unwrap_or("ULTRAKILL/Master".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let files = db.load_bundle_files(&path).unwrap();
    let mut seen = std::collections::HashSet::new();
    for f in &files {
        for o in f.objects.iter().filter(|o| o.class_id == 23) {
            let Ok(v) = f.read(o) else { continue };
            for m in v.get("m_Materials").array() {
                let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
                let Ok(mv) = mf.read_id(mid) else { continue };
                let Ok(Some((sf, sid))) = db.resolve(&mf, mv.get("m_Shader").pptr()) else { continue };
                if !seen.insert((sf.name.clone(), sid)) { continue }
                let Ok(sv) = sf.read_id(sid) else { continue };
                if sv.get("m_ParsedForm").get("m_Name").str() != want { continue }
                let mut s = String::new();
                shape(&sv, 3, &mut s);
                println!("shader {want} in {} id {sid}\n{s}\n", sf.name);
                for k in ["platforms", "offsets", "compressedLengths", "decompressedLengths"] {
                    let a = sv.get(k);
                    let mut t = String::new();
                    shape(a, 4, &mut t);
                    println!("{k}: {t}  values {}", a.compact().chars().take(300).collect::<String>());
                }
                println!("compressedBlob: {} bytes", sv.get("compressedBlob").bytes().len());
                let pf = sv.get("m_ParsedForm");
                let subs = pf.get("m_SubShaders").array();
                for (i, ss) in subs.iter().enumerate() {
                    for (j, p) in ss.get("m_Passes").array().iter().enumerate() {
                        let n = |k: &str| p.get(k).get("m_SubPrograms").array().len();
                        println!("subshader {i} pass {j} name {:?} type {} vp {} fp {} (progs: vertex/fragment subprograms)",
                            p.get("m_State").get("m_Name").str(), p.get("m_Type").i64(), n("progVertex"), n("progFragment"));
                    }
                }
                let kw = pf.get("m_KeywordNames").array();
                println!("keyword names ({}): {:?}", kw.len(), kw.iter().map(|k| k.str()).collect::<Vec<_>>());
                for (j, p) in subs[0].get("m_Passes").array().iter().enumerate() {
                    let tags: Vec<String> = p.get("m_State").get("m_Tags").get("tags").array().iter().map(|t| format!("{}={}", t.get("first").str(), t.get("second").str())).collect();
                    let st = p.get("m_State");
                    println!("pass {j}: name {:?} tags {tags:?} cull {} zwrite {} ztest {} blend src {} dst {} | vtx progs {} frag progs {}", st.get("m_Name").str(), st.get("culling").get("val").f32(), st.get("zWrite").get("val").f32(), st.get("zTest").get("val").f32(), st.get("rtBlend0").get("srcBlend").get("val").f32(), st.get("rtBlend0").get("destBlend").get("val").f32(), p.get("progVertex").get("m_PlayerSubPrograms").array().iter().map(|l| l.array().len()).sum::<usize>(), p.get("progFragment").get("m_PlayerSubPrograms").array().iter().map(|l| l.array().len()).sum::<usize>());
                    println!("   culling raw: {}", st.get("culling").compact());
                }
                let mut t = String::new();
                let first_prog = subs.first().and_then(|s| s.get("m_Passes").array().first().cloned());
                if let Some(p) = first_prog { shape(&p.get("progVertex"), 5, &mut t); }
                println!("progVertex shape: {t}");
                return;
            }
        }
    }
    println!("shader {want} not found through 0-1 materials");
}
