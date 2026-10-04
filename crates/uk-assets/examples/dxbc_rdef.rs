//! Full constant-buffer layouts from D3D11 bytecode: the DXBC RDEF chunk lists every variable of
//! every cbuffer (name, offset, size, used flag), which Unity's parameter lists omit.
//! dxbc_rdef [shader] [cbuffer name prefix]
use std::collections::BTreeMap;
use uk_assets::db::AssetDb;
use uk_assets::shader::ShaderAsset;

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn cstr(b: &[u8], i: usize) -> String {
    b[i..].iter().take_while(|&&c| c != 0).map(|&c| c as char).collect()
}

/// (cbuffer, offset) -> (variable, size, used) from one DXBC container.
fn rdef(dxbc: &[u8], out: &mut BTreeMap<(String, u32), (String, u32, bool)>) {
    let chunks = u32_at(dxbc, 28) as usize;
    for c in 0..chunks {
        let off = u32_at(dxbc, 32 + c * 4) as usize;
        if &dxbc[off..off + 4] != b"RDEF" {
            continue;
        }
        let d = &dxbc[off + 8..];
        let (cb_count, cb_off) = (u32_at(d, 0) as usize, u32_at(d, 4) as usize);
        let sm5 = d.len() > 32 && &d[28..32] == b"RD11";
        let var_stride = if sm5 { 40 } else { 24 };
        for k in 0..cb_count {
            let cb = cb_off + k * 24;
            let name = cstr(d, u32_at(d, cb) as usize);
            let (n, vo) = (u32_at(d, cb + 4) as usize, u32_at(d, cb + 8) as usize);
            for j in 0..n {
                let v = vo + j * var_stride;
                let vname = cstr(d, u32_at(d, v) as usize);
                let (start, size, flags) = (u32_at(d, v + 4), u32_at(d, v + 8), u32_at(d, v + 12));
                out.insert((name.clone(), start), (vname, size, flags & 2 != 0));
            }
        }
    }
}

fn main() {
    let want = std::env::args().nth(1).unwrap_or("ULTRAKILL/Master".into());
    let prefix = std::env::args().nth(2).unwrap_or("StandardProperties".into());
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join("campaign_scenes_level0-1.bundle");
    let files = db.load_bundle_files(&path).unwrap();
    for f in &files {
        for o in f.objects.iter().filter(|o| o.class_id == 23) {
            let Ok(v) = f.read(o) else { continue };
            for m in v.get("m_Materials").array() {
                let Ok(Some((mf, mid))) = db.resolve(f, m.pptr()) else { continue };
                let Ok(mv) = mf.read_id(mid) else { continue };
                let Ok(Some((sf, sid))) = db.resolve(&mf, mv.get("m_Shader").pptr()) else { continue };
                let Ok(sv) = sf.read_id(sid) else { continue };
                if sv.get("m_ParsedForm").get("m_Name").str() != want {
                    continue;
                }
                let sh = ShaderAsset::from_value_platform(&sv, 4).unwrap();
                let mut layout = BTreeMap::new();
                let mut programs = 0;
                for e in 0..sh.entry_count() as u32 {
                    let Ok(b) = sh.entry(e) else { continue };
                    // DXBC containers anywhere in the entry
                    let mut i = 0;
                    while let Some(p) = b[i..].windows(4).position(|w| w == b"DXBC") {
                        let at = i + p;
                        let total = if at + 28 <= b.len() { u32_at(&b, at + 24) as usize } else { 0 };
                        if total > 32 && at + total <= b.len() {
                            rdef(&b[at..at + total], &mut layout);
                            programs += 1;
                        }
                        i = at + 4;
                    }
                    if programs > 400 {
                        break;
                    }
                }
                println!("{want}: {programs} DXBC programs scanned");
                let cbs: std::collections::BTreeSet<&String> = layout.keys().map(|k| &k.0).collect();
                println!("cbuffers seen: {cbs:?}");
                for ((cb, off), (name, size, used)) in layout.iter().filter(|((cb, _), _)| cb.starts_with(&prefix)) {
                    println!("  {cb:24} @{off:4} {name:28} {size:3} bytes {}", if *used { "used" } else { "" });
                }
                return;
            }
        }
    }
}
