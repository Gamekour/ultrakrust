//! The game's own shaders, per level: for every material its renderers use, pick the variant Unity
//! would (material keywords), decode both stages (SMOL-V -> SPIR-V), validate them with naga, and
//! parse the parameter layout exactly. Constant-buffer sizes from the parameter blob must equal the
//! uniform struct sizes naga derives from the SPIR-V.
use crate::Report;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;
use uk_assets::db::AssetDb;
use uk_assets::shader::ShaderAsset;

fn spirv_bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// Uniform buffer sizes (group, binding) -> bytes, as naga lays them out.
fn uniform_sizes(m: &naga::Module) -> BTreeMap<(u32, u32), u32> {
    let mut layouter = naga::proc::Layouter::default();
    let _ = layouter.update(m.to_ctx());
    m.global_variables
        .iter()
        .filter(|(_, g)| g.space == naga::AddressSpace::Uniform)
        .filter_map(|(_, g)| g.binding.as_ref().map(|b| ((b.group, b.binding), layouter[g.ty].size)))
        .collect()
}

fn validate(words: &[u32]) -> Result<naga::Module, String> {
    let m = naga::front::spv::parse_u8_slice(&spirv_bytes(words), &naga::front::spv::Options::default()).map_err(|e| format!("parse: {e}"))?;
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&m)
        .map_err(|e| format!("validate: {}", e.into_inner()))?;
    Ok(m)
}

pub fn level(install: &Path, level: &str, r: &mut Report) {
    let mut db = AssetDb::open(install).unwrap();
    let path = AssetDb::bundle_dir(install).join(format!("campaign_scenes_{level}.bundle"));
    let Ok(files) = db.load_bundle_files(&path) else { return };
    let t = std::time::Instant::now();
    // distinct materials used by renderers -> (shader key, keywords, name)
    let mut mats: BTreeMap<(String, i64), Option<((String, i64), Vec<String>)>> = BTreeMap::new();
    for f in &files {
        for o in f.objects.iter().filter(|o| o.class_id == 23 || o.class_id == 137) {
            let Ok(v) = f.read(o) else { continue };
            for m in v.get("m_Materials").array() {
                let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
                if mats.contains_key(&(mf.name.clone(), mid)) {
                    continue;
                }
                let info = mf.read_id(mid).ok().and_then(|mv| {
                    let (sf, sid) = db.resolve(&mf, mv.get("m_Shader").pptr()).ok().flatten()?;
                    let kw = mv.get("m_ValidKeywords").array().iter().map(|k| k.str().to_string()).collect();
                    Some(((sf.name.clone(), sid), kw))
                });
                mats.insert((mf.name.clone(), mid), info);
            }
        }
    }
    let mut shaders: HashMap<(String, i64), Option<Arc<ShaderAsset>>> = HashMap::new();
    let (mut total, mut ok) = (0usize, 0usize);
    let mut fails: BTreeMap<String, usize> = BTreeMap::new();
    for info in mats.values() {
        let Some((skey, kw)) = info else { continue };
        let sh = shaders
            .entry(skey.clone())
            .or_insert_with(|| {
                let f = db.file(&skey.0).ok()?;
                let v = f.read_id(skey.1).ok()?;
                ShaderAsset::from_value(&v).ok().map(Arc::new)
            })
            .clone();
        total += 1;
        let Some(sh) = sh else {
            *fails.entry("shader has no Vulkan programs / unreadable".into()).or_default() += 1;
            continue;
        };
        let enabled: Vec<&str> = kw.iter().map(|s| s.as_str()).collect();
        let res = (|| -> Result<(), String> {
            let pass = sh.passes.first().ok_or("no pass")?;
            let vs = sh.select(&pass.vertex, &enabled).ok_or("no vertex variant")?;
            let fs = sh.select(&pass.fragment, &enabled).ok_or("no fragment variant")?;
            // On Vulkan the vertex variant's entry carries both stages (linked as a pair) and its parameter
            // blob covers both; the fragment list's blob indices point at parameter blobs.
            let _ = fs;
            for (sub, slot) in [(vs, 0usize), (vs, 1usize)] {
                let prog = sh.program(sub.blob).map_err(|e| format!("slot {slot} program blob {}: {}", sub.blob, e.0))?;
                let words = prog.stages.get(slot).cloned().flatten().ok_or(format!("{} stage missing", if slot == 0 { "vertex" } else { "fragment" }))?;
                let module = validate(&words).map_err(|e| { if let Ok(p) = std::env::var("DUMP_SPV") { std::fs::write(format!("{p}/{}_{}_{slot}.spv", sh.name.replace("/", "_").replace(" ", "_"), sub.blob), spirv_bytes(&words)).ok(); } format!("{} slot {slot}: {e}", sh.name) })?;
                let params = sh.params(sub.params).map_err(|e| format!("slot {slot} params {}: {}", sub.params, e.0))?;
                // Packed binding: stage mask in the top byte (0x04 vertex, 0x08 fragment), descriptor set
                // in the next byte, binding in the low 16 bits. Every constant buffer bound to this stage
                // must sit at exactly that (set, binding) in the SPIR-V with the same size (SPIR-V pads to 16).
                let us = uniform_sizes(&module);
                let stage_bit = if slot == 0 { 0x04 } else { 0x08 };
                for b in params.bindings.iter().filter(|b| b.kind == 1 && (b.packed >> 24) & stage_bit != 0) {
                    let key = ((b.packed >> 16) & 0xff, b.packed & 0xffff);
                    let cb = params.constant_buffers.iter().find(|c| c.name == b.name).ok_or(format!("binding {} has no buffer", b.name))?;
                    let got = us.get(&key).copied().ok_or(format!("{}: {} not at {key:?} in SPIR-V {us:?}", sh.name, b.name))?;
                    if got != cb.size && got != cb.size.next_multiple_of(16) {
                        return Err(format!("{}: {} size {} vs SPIR-V {got}", sh.name, b.name, cb.size));
                    }
                }
                let sizes: Vec<u32> = uniform_sizes(&module).values().copied().collect();
                if std::env::var_os("SHADER_DEBUG").is_some() {
                    eprintln!("{} slot {slot} blob {} params {} prog keywords {:?}", sh.name, sub.blob, sub.params, prog.keywords);
                    eprintln!("   spirv uniforms {:?}", uniform_sizes(&module));
                    for cb in &params.constant_buffers { eprintln!("   param cb {:?} size {} ({} params)", cb.name, cb.size, cb.params.len()) }
                    for b in &params.bindings { eprintln!("   binding {:?} kind {} packed {:#x} {:?}", b.name, b.kind, b.packed, b.extra) }
                }
                let _ = sizes;
            }
            Ok(())
        })();
        match res {
            Ok(()) => ok += 1,
            Err(e) => { if std::env::var_os("SHADER_DEBUG").is_some() { eprintln!("FAIL {e}") } *fails.entry(e.split(" blob ").next().unwrap_or(&e).split(" params ").next().unwrap_or(&e).chars().take(100).collect()).or_default() += 1 }
        }
    }
    r.info(&format!("shader.{level}.materials"), total as f64);
    r.higher(&format!("shader.{level}.materials_translated_pct"), 100.0 * ok as f64 / total.max(1) as f64);
    r.info(&format!("perf.{level}.shader_variants_ms"), t.elapsed().as_secs_f64() * 1e3);
    let mut fv: Vec<_> = fails.into_iter().collect();
    fv.sort_by(|a, b| b.1.cmp(&a.1));
    for (e, n) in fv.iter().take(8) {
        r.note(format!("gap: {level} shaders: {n} materials: {e}"));
    }
}
