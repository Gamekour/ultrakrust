//! Materials/shaders, Image types and sprite texture formats used by the uGUI graphics under a subtree.
//! cargo run --release -p uk-assets --example ui_survey -- level0-1 [path-prefix]
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or_else(|| "level0-1".into());
    let prefix = std::env::args().nth(2).unwrap_or_else(|| "FirstRoom/Player".into());
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let scene = db.file(&def.scene_file).unwrap();
    let mut mats: BTreeMap<String, usize> = BTreeMap::new();
    let mut types: BTreeMap<String, usize> = BTreeMap::new();
    let mut fmts: BTreeMap<String, usize> = BTreeMap::new();
    let mut fonts: BTreeMap<String, usize> = BTreeMap::new();
    for s in &def.scripts {
        if !def.path(s.node).starts_with(&prefix) { continue }
        let key = match s.class.as_str() { "Image" | "RawImage" | "Text" => "m_Material", "TextMeshProUGUI" => "m_sharedMaterial", _ => { *types.entry(format!("script {}", s.class)).or_default() += 1; continue } };
        let file = match &s.file { Some(f) => db.file(f).unwrap(), None => scene.clone() };
        let m = s.data.get(key).pptr();
        let mname = match db.read_pptr(&file, m) {
            Ok(Some((mf, _, mv))) => {
                let sh = db.read_pptr(&mf, mv.get("m_Shader").pptr()).ok().flatten().map(|(_, _, sv)| sv.get("m_ParsedForm").get("m_Name").str().to_string()).unwrap_or("?".into());
                format!("{} / {} [{}]", mv.get("m_Name").str(), sh, mf.name)
            }
            _ => "default".into(),
        };
        *mats.entry(format!("{} {}", s.class, mname)).or_default() += 1;
        if s.class == "Image" {
            *types.entry(format!("Image type {} fill {}", s.data.get("m_Type").i64(), s.data.get("m_FillMethod").i64())).or_default() += 1;
            if let Ok(Some((sf, _, sp))) = db.read_pptr(&file, s.data.get("m_Sprite").pptr()) {
                let rd = sp.get("m_RD");
                let packed = rd.get("settingsRaw").i64() & 1;
                if let Ok(Some((_, _, t))) = db.read_pptr(&sf, rd.get("texture").pptr()) {
                    *fmts.entry(format!("fmt {} packed {packed} atlas {}", t.get("m_TextureFormat").i64(), sp.get("m_SpriteAtlas").pptr().1 != 0)).or_default() += 1;
                }
            }
        }
        if s.class == "Text" {
            if let Ok(Some((_, _, f))) = db.read_pptr(&file, s.data.get("m_FontData").get("m_Font").pptr()) { *fonts.entry(format!("Text font {}", f.get("m_Name").str())).or_default() += 1; }
        }
        if s.class == "TextMeshProUGUI" {
            if let Ok(Some((_, _, f))) = db.read_pptr(&file, s.data.get("m_fontAsset").pptr()) { *fonts.entry(format!("TMP font {}", f.get("m_Name").str())).or_default() += 1; }
        }
    }
    for (k, v) in mats.iter().chain(&types).chain(&fmts).chain(&fonts) { println!("{v:5} {k}"); }
}
