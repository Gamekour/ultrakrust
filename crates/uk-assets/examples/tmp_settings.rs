//! TMP_Settings (resources.assets, no typetrees, read by hand) and the font assets it names:
//! default font, its fallback chain, and whether given characters are in their tables.
//! cargo run --release -p uk-assets --example tmp_settings -- [level bundle to load first]
use uk_assets::db::AssetDb;
use uk_assets::serialized::Value;

fn main() {
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    // load a level so TMP_FontAsset typetrees exist to borrow
    let level = std::env::args().nth(1).unwrap_or("level0-1".into());
    let b = std::fs::read_dir(AssetDb::bundle_dir(&install)).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().contains(&format!("campaign_scenes_{level}"))).unwrap();
    let _ = uk_assets::scenedef::load_scene(&mut db, &b).unwrap();
    let mut ui_fonts = 0;
    for f in ["sharedassets0.assets"] {
        let _ = f;
    }
    let f = db.file("resources.assets").unwrap();
    let o = f.objects.iter().find(|o| {
        let s = (f.data_offset + o.byte_start) as usize;
        o.class_id == 114 && f.data[s..s + o.byte_size as usize].windows(12).any(|w| w == b"TMP Settings")
    });
    let o = o.unwrap();
    let s = (f.data_offset + o.byte_start) as usize;
    let b = &f.data[s..s + o.byte_size as usize];
    let i32_at = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
    let i64_at = |o: usize| i64::from_le_bytes(b[o..o + 8].try_into().unwrap());
    // MonoBehaviour header (28) + name "TMP Settings" (4 + 12) = 0x2c; seven bools (4-aligned),
    // missingGlyphCharacter, warningsDisabled, defaultFontAsset at 0x50
    println!("missingGlyphCharacter {} warningsDisabled {}", i32_at(0x48), i32_at(0x4c));
    let default_font = (i32_at(0x50), i64_at(0x54));
    println!("defaultFontAsset {default_font:?} fallbackFontAssets count {} matchMaterialPreset {}", i32_at(0x98), i32_at(0x9c));
    let mut todo = vec![(f.clone(), default_font, 0)];
    let want: Vec<u32> = vec![0x2588, 0x25A1, 0x20];
    while let Some((from, p, depth)) = todo.pop() {
        match db.read_pptr(&from, p) {
            Ok(Some((ff, id, v))) => {
                ui_fonts += 1;
                let chars: Vec<u32> = v.get("m_CharacterTable").array().iter().map(|c| c.get("m_Unicode").i64() as u32).collect();
                let has: Vec<(String, bool)> = want.iter().map(|&u| (format!("{u:#x}"), chars.contains(&u))).collect();
                println!(
                    "{:indent$}font {:?} ({} {id}) mode {} chars {} has {has:?} source font {:?}",
                    "",
                    v.get("m_Name").str(),
                    ff.name,
                    v.get("m_AtlasPopulationMode").i64(),
                    chars.len(),
                    v.get("m_SourceFontFile").pptr(),
                    indent = depth * 2
                );
                for t in v.get("m_AtlasTextures").array() {
                    if let Ok(Some((_, tid, tv))) = db.read_pptr(&ff, t.pptr()) {
                        println!("{:indent$}  atlas {tid} {:?} {}x{} readable {} format {}", "", tv.get("m_Name").str(), tv.get("m_Width").i64(), tv.get("m_Height").i64(), tv.get("m_IsReadable").i64(), tv.get("m_TextureFormat").i64(), indent = depth * 2);
                    }
                }
                if let Ok(Some((_, sid, sv))) = db.read_pptr(&ff, v.get("m_SourceFontFile").pptr()) {
                    println!("{:indent$}  source font {sid} {:?} data {} bytes", "", sv.get("m_Name").str(), match sv.get("m_FontData") { uk_assets::serialized::Value::Bytes(b) => b.len(), x => x.array().len() }, indent = depth * 2);
                }
                println!("{:indent$}  padding {} size {}x{} render mode {} point size {}", "", v.get("m_AtlasPadding").i64(), v.get("m_AtlasWidth").i64(), v.get("m_AtlasHeight").i64(), v.get("m_AtlasRenderMode").i64(), v.get("m_FaceInfo").get("m_PointSize").f32(), indent = depth * 2);
                for fb in v.get("m_FallbackFontAssetTable").array() {
                    todo.push((ff.clone(), fb.pptr(), depth + 1));
                }
                let _: &Value = &v;
            }
            Ok(None) => println!("null font"),
            Err(e) => println!("font {p:?}: {e}"),
        }
    }
    println!("{ui_fonts} fonts read");
}
