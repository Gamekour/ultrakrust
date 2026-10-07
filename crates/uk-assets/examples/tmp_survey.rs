//! Histogram of TextMeshProUGUI settings in a level (what the TMP port must cover).
//! cargo run --release -p uk-assets --example tmp_survey -- level0-1
use std::collections::BTreeMap;
use uk_assets::{db::AssetDb, scenedef};

fn main() {
    let level = std::env::args().nth(1).unwrap_or_else(|| "level0-1".into());
    let install = uk_assets::find_install().expect("install");
    let mut db = AssetDb::open(&install).unwrap();
    let path = AssetDb::bundle_dir(&install).join(format!("campaign_scenes_{level}.bundle"));
    let def = scenedef::load_scene(&mut db, &path).unwrap();
    let mut h: BTreeMap<String, usize> = BTreeMap::new();
    let mut tags: BTreeMap<String, usize> = BTreeMap::new();
    for s in def.scripts.iter().filter(|s| s.class == "TextMeshProUGUI") {
        let d = &s.data;
        for k in ["m_enableAutoSizing", "m_overflowMode", "m_enableWordWrapping", "m_HorizontalAlignment", "m_VerticalAlignment", "m_isRichText", "m_fontStyle", "m_characterSpacing", "m_lineSpacing", "m_wordSpacing", "m_paragraphSpacing", "m_enableKerning", "m_isOrthographic", "m_parseCtrlCharacters", "m_overrideHtmlColors", "m_enableVertexGradient", "m_spriteAsset", "m_fontWeight", "m_margin", "m_isRightToLeft", "m_VertexBufferAutoSizeReduction", "m_charWidthMaxAdj", "m_lineSpacingMax", "m_maxVisibleCharacters", "m_maxVisibleLines", "m_firstVisibleCharacter", "m_textWrappingMode", "m_fontColorGradientPreset", "m_StyleSheet", "m_TextStyleHashCode", "m_isVolumetricText", "m_useMaxVisibleDescender", "m_geometrySortingOrder", "m_IsTextObjectScaleStatic", "m_isUsingLegacyAnimationComponent"] {
            if !d.has(k) { *h.entry(format!("{k} MISSING")).or_default() += 1; continue }
            *h.entry(format!("{k} = {:?}", d.get(k).compact())).or_default() += 1;
        }
        let t = d.get("m_text").str();
        let mut rest = t;
        while let Some(i) = rest.find('<') {
            let e = rest[i..].find('>').map(|e| i + e).unwrap_or(rest.len() - 1);
            let tag = &rest[i + 1..e];
            let name: String = tag.chars().take_while(|c| *c != '=' && *c != ' ').collect();
            *tags.entry(name).or_default() += 1;
            rest = &rest[e + 1..];
        }
        for c in t.chars().filter(|c| (*c as u32) < 32 || *c as u32 > 126) { *tags.entry(format!("char U+{:04X}", c as u32)).or_default() += 1; }
    }
    for (k, v) in &h { println!("{v:5} {k}") }
    println!("-- tags");
    for (k, v) in &tags { println!("{v:5} {k}") }
}
