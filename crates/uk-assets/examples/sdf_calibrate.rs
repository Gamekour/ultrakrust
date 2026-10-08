//! sdf.rs against FontEngine: glyphs of the static "LiberationSans SDF" atlas (baked by the editor
//! from the same face, point size 86, padding 9, SDFAA) re-rendered from the dynamic fallback's
//! source font; metrics and distance-field error.
//! cargo run --release -p uk-assets --example sdf_calibrate -- [chars]
use uk_assets::db::AssetDb;
use uk_assets::serialized::Value;

fn main() {
    let chars: Vec<u32> = std::env::args().nth(1).unwrap_or("AOg□i.W%".into()).chars().map(|c| c as u32).collect();
    let install = uk_assets::find_install().unwrap();
    let mut db = AssetDb::open(&install).unwrap();
    let b = std::fs::read_dir(AssetDb::bundle_dir(&install)).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().contains("campaign_scenes_level0-1")).unwrap();
    let _ = uk_assets::scenedef::load_scene(&mut db, &b).unwrap();
    let f = db.file("sharedassets0.assets").unwrap();
    let font = f.read_id(88).unwrap();
    let dynf = f.read_id(87).unwrap();
    let (_, _, src) = db.read_pptr(&f, dynf.get("m_SourceFontFile").pptr()).unwrap().unwrap();
    let ttf: Vec<u8> = match src.get("m_FontData") {
        Value::Bytes(b) => b.clone(),
        _ => panic!("no font data"),
    };
    if let Ok(out) = std::env::var("DUMP_TTF") {
        std::fs::write(out, &ttf).unwrap();
    }
    let (_, _, atlas) = db.read_pptr(&f, font.get("m_AtlasTextures").array()[0].pptr()).unwrap().unwrap();
    let tex = uk_assets::texture::decode_texture(&mut db, &atlas).unwrap();
    let aw = tex.width as i32;
    let pt = font.get("m_FaceInfo").get("m_PointSize").f32();
    let pad = font.get("m_AtlasPadding").i64() as u32;
    println!("point size {pt} padding {pad} atlas {}x{}", tex.width, tex.height);
    println!("FontEngine GetGlyphIndex(0x2588) = {}", uk_assets::sdf::glyph_index(&ttf, 0x2588));
    let glyphs = font.get("m_GlyphTable").array();
    let mut hist = [0usize; 8];
    for u in chars {
        let Some(c) = font.get("m_CharacterTable").array().iter().find(|c| c.get("m_Unicode").i64() as u32 == u) else {
            println!("{u:#x} not in the static atlas");
            continue;
        };
        let gi = c.get("m_GlyphIndex").i64();
        let g = glyphs.iter().find(|g| g.get("m_Index").i64() == gi).unwrap();
        let m = g.get("m_Metrics");
        let r = g.get("m_GlyphRect");
        let (rx, ry, rw, rh) = (r.get("m_X").i64() as i32, r.get("m_Y").i64() as i32, r.get("m_Width").i64() as i32, r.get("m_Height").i64() as i32);
        let mine = uk_assets::sdf::render(&ttf, uk_assets::sdf::glyph_index(&ttf, u), pt, pad, std::env::var("UNHINTED").is_err()).unwrap();
        println!(
            "{:?} idx {gi} (mine {}): static w {} h {} bx {} by {} adv {} rect {:?} | mine w {} h {} bx {} by {} adv {}",
            char::from_u32(u).unwrap(),
            uk_assets::sdf::glyph_index(&ttf, u),
            m.get("m_Width").f32(),
            m.get("m_Height").f32(),
            m.get("m_HorizontalBearingX").f32(),
            m.get("m_HorizontalBearingY").f32(),
            m.get("m_HorizontalAdvance").f32(),
            (rx, ry, rw, rh),
            mine.width,
            mine.height,
            mine.bearing_x,
            mine.bearing_y,
            mine.advance
        );
        if mine.width as i32 != rw || mine.height as i32 != rh {
            continue;
        }
        // compare the padded rects texel by texel
        let p = pad as i32;
        let tw = rw + 2 * p;
        let (mut sum, mut max, mut n, mut bias) = (0.0f64, 0i32, 0, 0i64);
        let mut mid_s = Vec::new();
        let mut mid_m = Vec::new();
        for j in 0..rh + 2 * p {
            for i in 0..tw {
                let (ax, ay) = (rx - p + i, ry - p + j);
                if ax < 0 || ay < 0 || ax >= aw || ay >= tex.height as i32 {
                    continue;
                }
                let sv = tex.rgba[((ay * aw + ax) * 4 + 3) as usize] as i32;
                let mv = mine.sdf[(j * tw + i) as usize] as i32;
                let e = mv - sv;
                sum += (e * e) as f64;
                max = max.max(e.abs());
                bias += e as i64;
                n += 1;
                hist[(e.unsigned_abs() as usize).min(7)] += 1;
                if j == (rh + 2 * p) / 2 {
                    mid_s.push(sv);
                    mid_m.push(mv);
                }
            }
        }
        println!("  rms {:.2} max {max} bias {:.2} over {n} texels", (sum / n as f64).sqrt(), bias as f64 / n as f64);
        println!("  mid row static {mid_s:?}");
        println!("  mid row mine   {mid_m:?}");
    }
    println!("|error| histogram 0..7+: {hist:?}");
}
