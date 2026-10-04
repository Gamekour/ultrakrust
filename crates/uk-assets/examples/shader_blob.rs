//! Decode the container of a Shader's compiled programs for one platform: decompress a chunk, print
//! its entry table and the start of the first entries, and look for SPIR-V / SMOL-V magics.
//! shader_blob [shader name] [platform index 0..2 (2 = Vulkan)] [chunk]
use uk_assets::db::AssetDb;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let want = args.get(1).cloned().unwrap_or("ULTRAKILL/Master".into());
    let plat: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(2);
    let chunk: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);
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
                let off = sv.get("offsets").array()[plat].array()[chunk].i64() as usize;
                let clen = sv.get("compressedLengths").array()[plat].array()[chunk].i64() as usize;
                let dlen = sv.get("decompressedLengths").array()[plat].array()[chunk].i64() as usize;
                let blob = sv.get("compressedBlob").bytes();
                let data = lz4_flex::block::decompress(&blob[off..off + clen], dlen).expect("lz4");
                let u = |i: usize| u32::from_le_bytes(data[i..i + 4].try_into().unwrap());
                println!("platform {} chunk {chunk}: {clen} -> {} bytes; chunks on this platform: {}", sv.get("platforms").array()[plat].i64(), data.len(), sv.get("offsets").array()[plat].array().len());
                let count = u(0) as usize;
                println!("first word (entry count?) {count}");
                for e in 0..count.min(6) {
                    let b = 4 + e * 12;
                    println!("  entry {e}: {} {} {}", u(b), u(b + 4), u(b + 8));
                }
                if let Ok(step) = std::env::var("SAMPLE") {
                    // translate every `step`-th program: SMOL-V -> SPIR-V -> naga validate (no output files)
                    let step: usize = step.parse().unwrap();
                    let mut segs: std::collections::HashMap<usize, Vec<u8>> = std::collections::HashMap::new();
                    let (mut progs, mut stages, mut ok) = (0, 0, 0);
                    let mut errs: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
                    for ei in (1..count).step_by(step) {
                        let (eo, el, seg) = (u(4 + ei * 12) as usize, u(8 + ei * 12) as usize, u(12 + ei * 12) as usize);
                        let s = segs.entry(seg).or_insert_with(|| {
                            let so = sv.get("offsets").array()[plat].array()[seg].i64() as usize;
                            let sc = sv.get("compressedLengths").array()[plat].array()[seg].i64() as usize;
                            let sd = sv.get("decompressedLengths").array()[plat].array()[seg].i64() as usize;
                            lz4_flex::block::decompress(&blob[so..so + sc], sd).unwrap()
                        });
                        let e = &s[eo..eo + el];
                        let w = |i: usize| u32::from_le_bytes(e[i..i + 4].try_into().unwrap());
                        if el < 32 || w(0) != 202012090 || w(4) != 25 { continue } // parameter blobs / other types
                        progs += 1;
                        // header: version, type, 4 stats, keyword count + padded strings, code length, code blob
                        let mut o = 24;
                        let nk = w(o) as usize;
                        o += 4;
                        for _ in 0..nk { let l = w(o) as usize; o += 4 + l.div_ceil(4) * 4; }
                        let clen = w(o) as usize;
                        let code = &e[o + 4..(o + 4 + clen).min(el)];
                        let cw = |i: usize| u32::from_le_bytes(code[i..i + 4].try_into().unwrap());
                        for st in 0..6 {
                            let (so, ss) = (cw(4 + st * 8) as usize, cw(8 + st * 8) as usize);
                            if ss == 0 { continue }
                            stages += 1;
                            let r = (|| -> Result<(), String> {
                                let words = uk_assets::smolv::decode(&code[so..so + ss]).ok_or("smolv")?;
                                let m = naga::front::spv::parse_u8_slice(bytemuck_cast(&words), &naga::front::spv::Options::default()).map_err(|e| format!("parse: {e}"))?;
                                naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).map_err(|e| format!("valid: {}", e.into_inner()))?;
                                Ok(())
                            })();
                            match r { Ok(()) => ok += 1, Err(m) => *errs.entry(m.chars().take(120).collect()).or_default() += 1 }
                        }
                    }
                    println!("sampled programs {progs}, stages {stages}, translate+validate OK {ok} ({:.1}%)", 100.0 * ok as f64 / stages.max(1) as f64);
                    for (m, n) in errs.iter().take(12) { println!("  {n:5}  {m}") }
                    return;
                }
                if std::env::var("STATS").is_ok() {
                    let mut lens: Vec<(usize, usize, usize)> = (0..count).map(|e| (u(8 + e * 12) as usize, e, u(12 + e * 12) as usize)).collect();
                    lens.sort();
                    let q = |p: f64| lens[((lens.len() - 1) as f64 * p) as usize].0;
                    println!("entry length p10 {} p50 {} p90 {} max {} (entry {} seg {})", q(0.1), q(0.5), q(0.9), lens.last().unwrap().0, lens.last().unwrap().1, lens.last().unwrap().2);
                    let segs: std::collections::BTreeSet<usize> = lens.iter().map(|x| x.2).collect();
                    println!("segments used: {} (min {:?} max {:?})", segs.len(), segs.first(), segs.last());
                    let big = lens.iter().filter(|x| x.0 > 4000).count();
                    println!("entries > 4000 bytes: {big}; first big: {:?}", lens.iter().find(|x| x.0 > 4000));
                    return;
                }
                if count < 1_000_000 && std::env::var("ENTRY").is_ok() {
                    let ei: usize = std::env::var("ENTRY").unwrap().parse().unwrap();
                    let (eo, el, seg) = (u(4 + ei * 12) as usize, u(8 + ei * 12) as usize, u(12 + ei * 12) as usize);
                    let so = sv.get("offsets").array()[plat].array()[seg].i64() as usize;
                    let sc = sv.get("compressedLengths").array()[plat].array()[seg].i64() as usize;
                    let sd = sv.get("decompressedLengths").array()[plat].array()[seg].i64() as usize;
                    let s = lz4_flex::block::decompress(&blob[so..so + sc], sd).unwrap();
                    let e = &s[eo..eo + el];
                    println!("entry {ei}: segment {seg} offset {eo} len {el}");
                    if std::env::var("TOKENS").is_ok() {
                        // strings (u32 length + bytes, padded to 4) vs plain u32s
                        let mut i = 0;
                        let mut toks = Vec::new();
                        while i + 4 <= el {
                            let w = u32::from_le_bytes(e[i..i + 4].try_into().unwrap()) as usize;
                            let s = (2..=64).contains(&w) && i + 4 + w <= el && e[i + 4..i + 4 + w].iter().all(|c| c.is_ascii_graphic() || *c == b' ');
                            if s { toks.push(format!("\"{}\"", String::from_utf8_lossy(&e[i + 4..i + 4 + w]))); i += 4 + w.div_ceil(4) * 4; }
                            else { toks.push(if w > 0xFFFF { format!("{w:#x}") } else { w.to_string() }); i += 4; }
                        }
                        println!("{}", toks.join(" "));
                        return;
                    }
                    for k in (std::env::var("FROM").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0) / 4)..(el / 4).min(std::env::var("TO").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(192) / 4) { let w = u32::from_le_bytes(e[k * 4..k * 4 + 4].try_into().unwrap()); println!("  +{:4} {w:08x} {w:>10} {:?}", k * 4, String::from_utf8_lossy(&e[k * 4..k * 4 + 4])); }
                    let mut hits = Vec::new();
                    for i in 0..e.len().saturating_sub(4) { let w = u32::from_le_bytes(e[i..i + 4].try_into().unwrap()); if w == 0x07230203 || w == 0x534D4F4C || &e[i..i + 4] == b"SMOL" { hits.push((i, w)); } }
                    println!("  magic hits (any alignment): {hits:x?}");
                    for (at, _) in &hits {
                        translate(&e[*at..], &format!("prog_{ei}_{at:x}"));
                    }
                    return;
                }
                // magics
                let mut smol = 0;
                let mut spirv = 0;
                for i in (0..data.len().saturating_sub(4)).step_by(4) {
                    match u(i) { 0x534D4F4C => smol += 1, 0x07230203 => spirv += 1, _ => {} }
                }
                println!("aligned magics: SMOL-V {smol}, SPIR-V {spirv}");
                // dump the start of the first entry
                let e0 = if count > 0 && count < 1_000_000 { u(4) as usize } else { 0 };
                println!("entry 0 words from {e0}:");
                for k in 0..24 {
                    let i = e0 + k * 4;
                    if i + 4 > data.len() { break }
                    let w = u(i);
                    println!("  +{:4} {w:08x} {w:>10} {:?}", k * 4, std::str::from_utf8(&data[i..i + 4]).unwrap_or(""));
                }
                // pass with programs and its first player subprogram
                let pf = sv.get("m_ParsedForm");
                for (pi, p) in pf.get("m_SubShaders").array()[0].get("m_Passes").array().iter().enumerate() {
                    let ps = p.get("progVertex").get("m_PlayerSubPrograms").array();
                    let n: usize = ps.iter().map(|x| x.array().len()).sum();
                    if n > 0 {
                        println!("pass {pi} vertex player subprograms {n}");
                        for (li, l) in ps.iter().enumerate() { println!("  list {li}: {} entries; first {}", l.array().len(), l.array().first().map(|x| x.compact()).unwrap_or_default().chars().take(200).collect::<String>()); }
                        let pb = p.get("progVertex").get("m_ParameterBlobIndices").array();
                        for (li, l) in pb.iter().enumerate() { println!("  param list {li}: {} entries; first {:?}", l.array().len(), l.array().first().map(|x| x.i64())); }
                        break;
                    }
                }
                return;
            }
        }
    }
}

fn bytemuck_cast(w: &[u32]) -> &[u8] {
    // SPIR-V words as little-endian bytes (x86/ARM hosts are little-endian)
    unsafe { std::slice::from_raw_parts(w.as_ptr() as *const u8, w.len() * 4) }
}

/// SMOL-V -> SPIR-V -> naga module -> validate -> WGSL file in $OUT.
fn translate(smol: &[u8], name: &str) {
    let Some(words) = uk_assets::smolv::decode(smol) else {
        println!("  {name}: SMOL-V decode FAILED (declared size {:?})", uk_assets::smolv::decoded_size(smol));
        return;
    };
    print!("  {name}: {} SPIR-V words; ", words.len());
    let module = match naga::front::spv::parse_u8_slice(bytemuck_cast(&words), &naga::front::spv::Options::default()) {
        Ok(m) => m,
        Err(err) => return println!("naga parse error: {err:?}"),
    };
    let info = match naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&module) {
        Ok(i) => i,
        Err(err) => return println!("validation error: {:?}", err.into_inner()),
    };
    match naga::back::wgsl::write_string(&module, &info, naga::back::wgsl::WriterFlags::empty()) {
        Ok(src) => {
            let p = format!("{}/{name}.wgsl", std::env::var("OUT").expect("set OUT to a directory outside the repo (translated game shaders must not be committed)"));
            std::fs::write(&p, &src).unwrap();
            println!("WGSL {} bytes -> {p}", src.len());
        }
        Err(err) => println!("wgsl error: {err:?}"),
    }
}
