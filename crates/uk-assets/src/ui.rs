//! The assets a level's uGUI graphics use: Sprites (Image), Texture2Ds (RawImage), TMP_FontAssets
//! (TextMeshProUGUI, with their fallback and weight chains) and the Materials graphics are drawn
//! with, including the default canvas material (`Canvas.GetDefaultCanvasMaterial`: a material on
//! the built-in UI/Default shader with the shader's default properties).
//!
//! Pixels are not decoded here: textures are named by (file, path id) for the renderer, with their
//! sizes for mesh generation. Every PPtr in a scene script that resolves to one of these assets is
//! recorded under (script, field path), so ported scripts can swap sprites / fonts by field.

use crate::db::AssetDb;
use crate::scenedef::SceneDef;
use crate::serialized::{SerializedFile, Value};
use crate::shader::MaterialProps;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

const CLASS_TEXTURE2D: i32 = 28;
const CLASS_MATERIAL: i32 = 21;
const CLASS_MONOBEHAVIOUR: i32 = 114;
const CLASS_FONT: i32 = 128;
const CLASS_SPRITE: i32 = 213;

/// unity_builtin_extra's UI/Default (`Canvas.GetDefaultCanvasMaterial`'s shader).
pub const UI_DEFAULT_SHADER: (&str, i64) = ("unity_builtin_extra", 10770);

#[derive(Clone, Debug)]
pub struct UiTexture {
    pub name: String,
    pub file: String,
    pub path_id: i64,
    pub width: u32,
    pub height: u32,
    /// `m_TextureSettings.m_WrapU` (TextureWrapMode: 0 Repeat, 1 Clamp, ...)
    pub wrap_u: i64,
    /// `m_TextureSettings.m_FilterMode` (0 Point, 1 Bilinear, 2 Trilinear)
    pub filter: i64,
    /// the atlas of a dynamic font asset: its pixels are written at runtime
    pub dynamic: Option<Arc<Mutex<DynAtlas>>>,
}

/// A Sprite with the UV / padding data `UnityEngine.Sprites.DataUtility` returns.
#[derive(Clone, Debug)]
pub struct Sprite {
    pub name: String,
    pub texture: Option<u32>,
    /// `rect` (x, y, w, h) in pixels
    pub rect: [f32; 4],
    /// `border` (left, bottom, right, top) in pixels
    pub border: [f32; 4],
    pub pixels_per_unit: f32,
    pub pivot: [f32; 2],
    /// DataUtility.GetOuterUV / GetInnerUV: (x0, y0, x1, y1)
    pub outer_uv: [f32; 4],
    pub inner_uv: [f32; 4],
    /// DataUtility.GetPadding: (left, bottom, right, top) pixels the packer trimmed
    pub padding: [f32; 4],
    /// `textureRect` (x, y, w, h)
    pub texture_rect: [f32; 4],
    /// `Sprite.packed` (m_RD.settingsRaw bit 0)
    pub packed: bool,
}

impl Sprite {
    /// `Sprite.bounds.size * pixelsPerUnit` == rect size
    pub fn size(&self) -> [f32; 2] {
        [self.rect[2], self.rect[3]]
    }
}

/// TMP FaceInfo (font units at `point_size`).
#[derive(Clone, Copy, Debug, Default)]
pub struct FaceInfo {
    pub point_size: f32,
    pub scale: f32,
    pub line_height: f32,
    pub ascent_line: f32,
    pub cap_line: f32,
    pub mean_line: f32,
    pub baseline: f32,
    pub descent_line: f32,
    pub superscript_offset: f32,
    pub superscript_size: f32,
    pub subscript_offset: f32,
    pub subscript_size: f32,
    pub underline_offset: f32,
    pub underline_thickness: f32,
    pub strikethrough_offset: f32,
    pub strikethrough_thickness: f32,
    pub tab_width: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Glyph {
    /// GlyphMetrics: width, height, horizontalBearingX, horizontalBearingY, horizontalAdvance
    pub width: f32,
    pub height: f32,
    pub bearing_x: f32,
    pub bearing_y: f32,
    pub advance: f32,
    /// GlyphRect: x, y, width, height (atlas pixels)
    pub rect: [f32; 4],
    pub scale: f32,
    pub atlas_index: u32,
}

/// `glyphs` key of the zero glyph synthesized for missing control characters (its glyph index is 0)
pub const SYNTH_GLYPH: u32 = u32::MAX;

#[derive(Clone, Copy, Debug, Default)]
pub struct TmpCharacter {
    pub glyph: u32,
    pub scale: f32,
}

/// GlyphValueRecord: xPlacement, yPlacement, xAdvance, yAdvance
pub type GlyphValue = [f32; 4];

#[derive(Clone, Debug, Default)]
pub struct TmpFont {
    pub name: String,
    pub file: String,
    pub path_id: i64,
    /// AtlasPopulationMode 1: glyphs missing from the tables are rasterized at runtime
    pub dynamic: bool,
    pub face: FaceInfo,
    pub glyphs: HashMap<u32, Glyph>,
    pub characters: HashMap<u32, TmpCharacter>,
    /// key `second << 16 | first` (glyph indices) -> (first record, second record)
    pub kerning: HashMap<u32, (GlyphValue, GlyphValue)>,
    pub fallbacks: Vec<u32>,
    /// `material` (index into `UiAssets::materials`)
    pub material: Option<u32>,
    pub atlas_textures: Vec<Option<u32>>,
    pub atlas_width: f32,
    pub atlas_height: f32,
    pub atlas_padding: f32,
    /// m_AtlasRenderMode (GlyphRenderMode; 4169 SDFAA, bitmap modes have bit 0x10/0x20 clear of 0x40...)
    pub atlas_render_mode: i64,
    /// m_FontWeightTable: (regular, italic) per weight 0..900 step 100
    pub weights: Vec<(Option<u32>, Option<u32>)>,
    pub normal_style: f32,
    pub normal_spacing_offset: f32,
    pub bold_style: f32,
    pub bold_spacing: f32,
    pub italic_style: f32,
    pub tab_size: f32,
    /// AtlasPopulationMode.Dynamic with a source font file: the runtime additions
    pub dynamic_atlas: Option<Arc<Mutex<DynAtlas>>>,
}

impl TmpFont {
    /// m_GlyphLookupDictionary (serialized glyphs, then the ones added at runtime)
    pub fn glyph(&self, g: u32) -> Option<Glyph> {
        self.glyphs.get(&g).copied().or_else(|| self.dynamic_atlas.as_ref().and_then(|d| d.lock().unwrap().glyphs.get(&g).copied()))
    }

    /// characterLookupTable, then TryAddCharacterInternal for a dynamic font
    pub fn character(&self, u: u32) -> Option<TmpCharacter> {
        if let Some(c) = self.characters.get(&u) {
            return Some(*c);
        }
        self.dynamic_atlas.as_ref().and_then(|d| d.lock().unwrap().try_add(u))
    }
}

/// A dynamic font asset's runtime state (TMP_FontAsset.TryAddCharacterInternal): the characters and
/// glyphs FontEngine added and the Alpha8 atlas it rendered them into.
#[derive(Debug, Default)]
pub struct DynAtlas {
    ttf: Arc<Vec<u8>>,
    point_size: f32,
    padding: u32,
    /// GlyphRenderMode 4169 SDFAA_HINTED
    hinted: bool,
    pub characters: HashMap<u32, TmpCharacter>,
    pub glyphs: HashMap<u32, Glyph>,
    /// m_MissingUnicodesFromFontFile
    missing: HashSet<u32>,
    free: Vec<crate::sdf::Rect>,
    used: Vec<crate::sdf::Rect>,
    pub width: u32,
    pub height: u32,
    /// Alpha8, bottom row first; empty while the texture is still the serialized 0x0
    pub pixels: Vec<u8>,
    /// bumped on every write to `pixels`
    pub version: u64,
}

impl DynAtlas {
    pub fn try_add(&mut self, unicode: u32) -> Option<TmpCharacter> {
        if let Some(c) = self.characters.get(&unicode) {
            return Some(*c);
        }
        if self.missing.contains(&unicode) {
            return None;
        }
        let mut gi = crate::sdf::glyph_index(&self.ttf, unicode);
        if gi == 0 {
            gi = match unicode {
                160 => crate::sdf::glyph_index(&self.ttf, 32),
                173 | 8209 => crate::sdf::glyph_index(&self.ttf, 45),
                _ => 0,
            };
            if gi == 0 {
                self.missing.insert(unicode);
                return None;
            }
        }
        let c = TmpCharacter { glyph: gi, scale: 1.0 };
        if self.glyphs.contains_key(&gi) {
            self.characters.insert(unicode, c);
            return Some(c);
        }
        let r = crate::sdf::render(&self.ttf, gi, self.point_size, self.padding, self.hinted)?;
        // Texture2D.Resize + FontEngine.ResetAtlasTexture on first use
        if self.pixels.is_empty() {
            self.pixels = vec![0; (self.width * self.height) as usize];
        }
        let (w, h, p) = (r.width as i32, r.height as i32, self.padding as i32);
        let mut rect = [0.0; 4];
        if w > 0 && h > 0 {
            // FontEngine packs (w + 2p + 1) x (h + 2p + 1) and places the glyph p + 1 in
            // (measured on the static atlases); no multi-atlas: a full atlas fails the add
            let n = crate::sdf::pack(&mut self.free, &mut self.used, w + 2 * p + 1, h + 2 * p + 1)?;
            let (ox, oy) = (n[0] + 1, n[1] + 1);
            let tw = w + 2 * p;
            for j in 0..h + 2 * p {
                for i in 0..tw {
                    let (x, y) = (ox + i, oy + j);
                    if x < self.width as i32 && y < self.height as i32 {
                        self.pixels[(y * self.width as i32 + x) as usize] = r.sdf[(j * tw + i) as usize];
                    }
                }
            }
            rect = [(n[0] + p + 1) as f32, (n[1] + p + 1) as f32, w as f32, h as f32];
            self.version += 1;
        }
        self.glyphs.insert(
            gi,
            Glyph { width: r.width, height: r.height, bearing_x: r.bearing_x, bearing_y: r.bearing_y, advance: r.advance, rect, scale: 1.0, atlas_index: 0 },
        );
        self.characters.insert(unicode, c);
        Some(c)
    }
}

/// A material a graphic is drawn with. `file` is what its texture PPtrs resolve against.
#[derive(Clone, Debug)]
pub struct UiMaterial {
    pub name: String,
    pub props: Arc<MaterialProps>,
    pub file: String,
    pub path_id: i64,
    /// the shader's (file, path id)
    pub shader: (String, i64),
    /// `_MainTex` (index into `UiAssets::textures`)
    pub main_tex: Option<u32>,
}

/// A legacy `UnityEngine.Font` (dynamic TTF).
#[derive(Clone, Debug)]
pub struct LegacyFont {
    pub name: String,
    pub ttf: Arc<Vec<u8>>,
    pub font_size: f32,
    pub line_spacing: f32,
    pub ascent: f32,
    pub descent: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiRef {
    Sprite(u32),
    Texture(u32),
    Font(u32),
    Material(u32),
    LegacyFont(u32),
}

#[derive(Default)]
pub struct UiAssets {
    pub textures: Vec<UiTexture>,
    pub sprites: Vec<Sprite>,
    pub fonts: Vec<TmpFont>,
    pub materials: Vec<UiMaterial>,
    pub legacy_fonts: Vec<LegacyFont>,
    /// Canvas.GetDefaultCanvasMaterial (index into `materials`)
    pub default_material: Option<u32>,
    /// TMP_Settings.missingGlyphCharacter (0 read as 9633)
    pub missing_glyph: u32,
    /// TMP_Settings.defaultFontAsset
    pub default_font: Option<u32>,
    /// TMP_Settings.fallbackFontAssets
    pub settings_fallbacks: Vec<u32>,
    /// (script index, field path such as "m_Sprite" or "rankImages/3") -> asset
    pub refs: HashMap<(u32, String), UiRef>,
    pub warnings: Vec<String>,
}

impl UiAssets {
    pub fn get(&self, script: u32, field: &str) -> Option<UiRef> {
        self.refs.get(&(script, field.to_string())).copied()
    }
    pub fn sprite(&self, script: u32, field: &str) -> Option<u32> {
        match self.get(script, field) {
            Some(UiRef::Sprite(s)) => Some(s),
            _ => None,
        }
    }
    pub fn material(&self, script: u32, field: &str) -> Option<u32> {
        match self.get(script, field) {
            Some(UiRef::Material(s)) => Some(s),
            _ => None,
        }
    }
    pub fn font(&self, script: u32, field: &str) -> Option<u32> {
        match self.get(script, field) {
            Some(UiRef::Font(s)) => Some(s),
            _ => None,
        }
    }
    pub fn texture(&self, script: u32, field: &str) -> Option<u32> {
        match self.get(script, field) {
            Some(UiRef::Texture(s)) => Some(s),
            _ => None,
        }
    }
}

/// Graphic classes whose material fields are loaded (other scripts' materials are not UI).
const GRAPHIC_CLASSES: &[&str] = &["Image", "RawImage", "Text", "TextMeshProUGUI"];

struct Loader<'a> {
    db: &'a mut AssetDb,
    out: UiAssets,
    textures: HashMap<(String, i64), Option<u32>>,
    sprites: HashMap<(String, i64), Option<u32>>,
    fonts: HashMap<(String, i64), Option<u32>>,
    materials: HashMap<(String, i64), Option<u32>>,
    legacy: HashMap<(String, i64), Option<u32>>,
}

fn rect4(v: &Value) -> [f32; 4] {
    [v.get("x").f32(), v.get("y").f32(), v.get("width").f32(), v.get("height").f32()]
}

fn is_pptr(v: &Value) -> bool {
    matches!(v, Value::Struct(f) if f.len() == 2 && &*f[0].0 == "m_FileID" && &*f[1].0 == "m_PathID")
}

impl Loader<'_> {
    fn texture(&mut self, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Option<u32> {
        let (f, id) = self.db.resolve(from, pptr).ok()??;
        let key = (f.name.clone(), id);
        if let Some(t) = self.textures.get(&key) {
            return *t;
        }
        let t = (|| {
            if f.object(id)?.class_id != CLASS_TEXTURE2D {
                return None;
            }
            let v = f.read_id(id).ok()?;
            self.out.textures.push(UiTexture {
                name: v.get("m_Name").str().to_string(),
                file: f.name.clone(),
                path_id: id,
                width: v.get("m_Width").i64() as u32,
                height: v.get("m_Height").i64() as u32,
                wrap_u: v.get("m_TextureSettings").get("m_WrapU").i64(),
                filter: v.get("m_TextureSettings").get("m_FilterMode").i64(),
                dynamic: None,
            });
            Some(self.out.textures.len() as u32 - 1)
        })();
        self.textures.insert(key, t);
        t
    }

    fn sprite(&mut self, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Option<u32> {
        let (f, id) = self.db.resolve(from, pptr).ok()??;
        let key = (f.name.clone(), id);
        if let Some(s) = self.sprites.get(&key) {
            return *s;
        }
        let s = (|| {
            if f.object(id)?.class_id != CLASS_SPRITE {
                return None;
            }
            let v = f.read_id(id).ok()?;
            let rd = v.get("m_RD");
            let texture = self.texture(&f, rd.get("texture").pptr());
            let rect = rect4(v.get("m_Rect"));
            let b = v.get("m_Border");
            let border = [b.get("x").f32(), b.get("y").f32(), b.get("z").f32(), b.get("w").f32()];
            let tr = rect4(rd.get("textureRect"));
            let tro = rd.get("textureRectOffset");
            let off = [tro.get("x").f32(), tro.get("y").f32()];
            let (mut outer, mut inner) = ([0.0; 4], [0.0; 4]);
            if let Some(t) = texture.map(|t| &self.out.textures[t as usize]) {
                let (tw, th) = (t.width as f32, t.height as f32);
                outer = [tr[0] / tw, tr[1] / th, (tr[0] + tr[2]) / tw, (tr[1] + tr[3]) / th];
                // the border is in sprite-rect pixels; the packer may have trimmed `off` of it
                let l = (border[0] - off[0]).max(0.0);
                let bt = (border[1] - off[1]).max(0.0);
                let r = (border[2] - (rect[2] - tr[2] - off[0])).max(0.0);
                let tp = (border[3] - (rect[3] - tr[3] - off[1])).max(0.0);
                inner = [(tr[0] + l) / tw, (tr[1] + bt) / th, (tr[0] + tr[2] - r) / tw, (tr[1] + tr[3] - tp) / th];
            }
            let p = v.get("m_Pivot");
            self.out.sprites.push(Sprite {
                name: v.get("m_Name").str().to_string(),
                texture,
                rect,
                border,
                pixels_per_unit: v.get("m_PixelsToUnits").f32(),
                pivot: [p.get("x").f32(), p.get("y").f32()],
                outer_uv: outer,
                inner_uv: inner,
                padding: [off[0], off[1], rect[2] - tr[2] - off[0], rect[3] - tr[3] - off[1]],
                texture_rect: tr,
                packed: rd.get("settingsRaw").i64() & 1 == 1,
            });
            Some(self.out.sprites.len() as u32 - 1)
        })();
        self.sprites.insert(key, s);
        s
    }

    fn material(&mut self, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Option<u32> {
        let (f, id) = self.db.resolve(from, pptr).ok()??;
        let key = (f.name.clone(), id);
        if let Some(m) = self.materials.get(&key) {
            return *m;
        }
        let m = (|| {
            if f.object(id)?.class_id != CLASS_MATERIAL {
                return None;
            }
            let props = MaterialProps::from_value(&f.read_id(id).ok()?);
            let (sf, sid) = self.db.resolve(&f, props.shader).ok()??;
            let main_tex = props.textures.get("_MainTex").and_then(|t| self.texture(&f, t.texture));
            self.out.materials.push(UiMaterial { name: props.name.clone(), props: Arc::new(props), file: f.name.clone(), path_id: id, shader: (sf.name.clone(), sid), main_tex });
            Some(self.out.materials.len() as u32 - 1)
        })();
        self.materials.insert(key, m);
        m
    }

    fn legacy_font(&mut self, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Option<u32> {
        let (f, id) = self.db.resolve(from, pptr).ok()??;
        let key = (f.name.clone(), id);
        if let Some(m) = self.legacy.get(&key) {
            return *m;
        }
        let m = (|| {
            if f.object(id)?.class_id != CLASS_FONT {
                return None;
            }
            let v = f.read_id(id).ok()?;
            let ttf: Vec<u8> = match v.get("m_FontData") {
                Value::Bytes(b) => b.clone(),
                Value::Array(a) => a.iter().map(|x| x.i64() as u8).collect(),
                _ => Vec::new(),
            };
            self.out.legacy_fonts.push(LegacyFont {
                name: v.get("m_Name").str().to_string(),
                ttf: Arc::new(ttf),
                font_size: v.get("m_FontSize").f32(),
                line_spacing: v.get("m_LineSpacing").f32(),
                ascent: v.get("m_Ascent").f32(),
                descent: v.get("m_Descent").f32(),
            });
            Some(self.out.legacy_fonts.len() as u32 - 1)
        })();
        self.legacy.insert(key, m);
        m
    }

    /// The runtime state of a dynamic font asset: its source font's TTF, the serialized free / used
    /// rects and atlas pixels. None without a source font (TryAddCharacterInternal fails).
    fn dynamic_atlas(&mut self, f: &Arc<SerializedFile>, v: &Value, atlas_textures: &[Option<u32>]) -> Option<Arc<Mutex<DynAtlas>>> {
        let (sf, sid) = self.db.resolve(f, v.get("m_SourceFontFile").pptr()).ok()??;
        if sf.object(sid)?.class_id != CLASS_FONT {
            return None;
        }
        let ttf: Vec<u8> = match sf.read_id(sid).ok()?.get("m_FontData") {
            Value::Bytes(b) => b.clone(),
            Value::Array(a) => a.iter().map(|x| x.i64() as u8).collect(),
            _ => return None,
        };
        if ttf.is_empty() {
            return None;
        }
        let r4 = |r: &Value| [r.get("m_X").i64() as i32, r.get("m_Y").i64() as i32, r.get("m_Width").i64() as i32, r.get("m_Height").i64() as i32];
        let (width, height) = (v.get("m_AtlasWidth").i64() as u32, v.get("m_AtlasHeight").i64() as u32);
        let render_mode = v.get("m_AtlasRenderMode").i64();
        let mut free: Vec<_> = v.get("m_FreeGlyphRects").array().iter().map(r4).collect();
        if free.is_empty() {
            let num = if render_mode & 16 != 16 { 1 } else { 0 };
            free.push([0, 0, width as i32 - num, height as i32 - num]);
        }
        // a populated serialized atlas starts from its pixels
        let tex = atlas_textures.first().copied().flatten()?;
        let t = &self.out.textures[tex as usize];
        let mut pixels = Vec::new();
        if t.width > 0 && t.height > 0 {
            let (file, id) = (t.file.clone(), t.path_id);
            if let Some(d) = self.db.file(&file).ok().and_then(|tf| tf.read_id(id).ok()).and_then(|tv| crate::texture::decode_texture(self.db, &tv).ok()) {
                if d.width == width && d.height == height {
                    pixels = d.rgba.chunks(4).map(|c| c[3]).collect();
                }
            }
        }
        let d = Arc::new(Mutex::new(DynAtlas {
            ttf: Arc::new(ttf),
            point_size: v.get("m_FaceInfo").get("m_PointSize").f32(),
            padding: v.get("m_AtlasPadding").i64() as u32,
            hinted: render_mode == 4169,
            free,
            used: v.get("m_UsedGlyphRects").array().iter().map(r4).collect(),
            width,
            height,
            pixels,
            ..Default::default()
        }));
        let t = &mut self.out.textures[tex as usize];
        t.dynamic = Some(d.clone());
        (t.width, t.height) = (width, height);
        Some(d)
    }

    /// TMP_Settings ("TMP Settings" in resources.assets). No file of the build carries its typetree,
    /// so the fields are read in their serialized order: MonoBehaviour header, m_Name, seven bools
    /// (4-aligned), missingGlyphCharacter, warningsDisabled, defaultFontAsset, defaultFontAssetPath,
    /// defaultFontSize, two autosize ratios, two default container sizes, two bools,
    /// fallbackFontAssets.
    fn tmp_settings(&mut self) {
        let Ok(f) = self.db.file("resources.assets") else {
            self.out.warnings.push("resources.assets: TMP_Settings unreadable".into());
            return;
        };
        let Some(o) = f.objects.iter().find(|o| {
            let s = (f.data_offset + o.byte_start) as usize;
            o.class_id == CLASS_MONOBEHAVIOUR && f.data.get(s + 28..s + 44).is_some_and(|b| b[..4] == 12u32.to_le_bytes() && &b[4..] == b"TMP Settings")
        }) else {
            self.out.warnings.push("TMP_Settings not found".into());
            return;
        };
        let s = (f.data_offset + o.byte_start) as usize;
        let b = &f.data[s..s + o.byte_size as usize];
        let i32_at = |o: usize| b.get(o..o + 4).map_or(0, |x| i32::from_le_bytes(x.try_into().unwrap()));
        let i64_at = |o: usize| b.get(o..o + 8).map_or(0, |x| i64::from_le_bytes(x.try_into().unwrap()));
        let m = i32_at(0x48) as u32;
        self.out.missing_glyph = if m == 0 { 9633 } else { m };
        let default = (i32_at(0x50), i64_at(0x54));
        let path_len = i32_at(0x5c) as usize;
        let mut at = (0x60 + path_len + 3) & !3;
        at += 4 + 8 + 16 + 8;
        let n = i32_at(at).max(0) as usize;
        let fallbacks: Vec<(i32, i64)> = (0..n).map(|k| (i32_at(at + 4 + 12 * k), i64_at(at + 8 + 12 * k))).collect();
        self.out.default_font = self.font(&f, default);
        self.out.settings_fallbacks = fallbacks.into_iter().filter_map(|p| self.font(&f, p)).collect();
    }

    fn font(&mut self, from: &Arc<SerializedFile>, pptr: (i32, i64)) -> Option<u32> {
        let (f, id) = self.db.resolve(from, pptr).ok()??;
        let key = (f.name.clone(), id);
        if let Some(m) = self.fonts.get(&key) {
            return *m;
        }
        if f.object(id)?.class_id != CLASS_MONOBEHAVIOUR {
            self.fonts.insert(key, None);
            return None;
        }
        let v = f.read_id(id).ok()?;
        if !v.has("m_GlyphTable") {
            self.fonts.insert(key, None);
            return None;
        }
        // reserve the slot first: fallback chains may loop back
        let idx = self.out.fonts.len() as u32;
        self.out.fonts.push(TmpFont::default());
        self.fonts.insert(key, Some(idx));
        let fi = v.get("m_FaceInfo");
        let mut face = FaceInfo {
            point_size: fi.get("m_PointSize").f32(),
            scale: fi.get("m_Scale").f32(),
            line_height: fi.get("m_LineHeight").f32(),
            ascent_line: fi.get("m_AscentLine").f32(),
            cap_line: fi.get("m_CapLine").f32(),
            mean_line: fi.get("m_MeanLine").f32(),
            baseline: fi.get("m_Baseline").f32(),
            descent_line: fi.get("m_DescentLine").f32(),
            superscript_offset: fi.get("m_SuperscriptOffset").f32(),
            superscript_size: fi.get("m_SuperscriptSize").f32(),
            subscript_offset: fi.get("m_SubscriptOffset").f32(),
            subscript_size: fi.get("m_SubscriptSize").f32(),
            underline_offset: fi.get("m_UnderlineOffset").f32(),
            underline_thickness: fi.get("m_UnderlineThickness").f32(),
            strikethrough_offset: fi.get("m_StrikethroughOffset").f32(),
            strikethrough_thickness: fi.get("m_StrikethroughThickness").f32(),
            tab_width: fi.get("m_TabWidth").f32(),
        };
        let mut glyphs = HashMap::new();
        for g in v.get("m_GlyphTable").array() {
            let m = g.get("m_Metrics");
            let r = g.get("m_GlyphRect");
            glyphs.insert(
                g.get("m_Index").i64() as u32,
                Glyph {
                    width: m.get("m_Width").f32(),
                    height: m.get("m_Height").f32(),
                    bearing_x: m.get("m_HorizontalBearingX").f32(),
                    bearing_y: m.get("m_HorizontalBearingY").f32(),
                    advance: m.get("m_HorizontalAdvance").f32(),
                    rect: [r.get("m_X").f32(), r.get("m_Y").f32(), r.get("m_Width").f32(), r.get("m_Height").f32()],
                    scale: g.get("m_Scale").f32(),
                    atlas_index: g.get("m_AtlasIndex").i64() as u32,
                },
            );
        }
        let mut characters = HashMap::new();
        for c in v.get("m_CharacterTable").array() {
            characters.insert(c.get("m_Unicode").i64() as u32, TmpCharacter { glyph: c.get("m_GlyphIndex").i64() as u32, scale: c.get("m_Scale").f32() });
        }
        // AddSynthesizedCharactersAndFaceMetrics: control characters missing from the table get a
        // zero glyph (no font face is loaded here, so dynamic fonts take this path too)
        for u in [3u32, 9, 10, 11, 13, 1564, 8203, 8206, 8207, 8232, 8233, 8288] {
            characters.entry(u).or_insert(TmpCharacter { glyph: SYNTH_GLYPH, scale: 1.0 });
        }
        glyphs.insert(SYNTH_GLYPH, Glyph { scale: 1.0, ..Default::default() });
        if face.cap_line == 0.0 {
            if let Some(g) = characters.get(&88).and_then(|c| glyphs.get(&c.glyph)) {
                face.cap_line = g.bearing_y;
            }
        }
        if face.mean_line == 0.0 {
            if let Some(g) = characters.get(&120).and_then(|c| glyphs.get(&c.glyph)) {
                face.mean_line = g.bearing_y;
            }
        }
        let gv = |r: &Value| {
            let g = r.get("m_GlyphValueRecord");
            [g.get("m_XPlacement").f32(), g.get("m_YPlacement").f32(), g.get("m_XAdvance").f32(), g.get("m_YAdvance").f32()]
        };
        let mut kerning = HashMap::new();
        for k in v.get("m_FontFeatureTable").get("m_GlyphPairAdjustmentRecords").array() {
            let (a, b) = (k.get("m_FirstAdjustmentRecord"), k.get("m_SecondAdjustmentRecord"));
            let key = (b.get("m_GlyphIndex").i64() as u32) << 16 | a.get("m_GlyphIndex").i64() as u32;
            kerning.insert(key, (gv(a), gv(b)));
        }
        let material = self.material(&f, v.get("material").pptr());
        let atlas_textures: Vec<Option<u32>> = v.get("m_AtlasTextures").array().iter().map(|t| self.texture(&f, t.pptr())).collect();
        let fallback_ptrs: Vec<(i32, i64)> = v.get("m_FallbackFontAssetTable").array().iter().map(|p| p.pptr()).collect();
        let fallbacks = fallback_ptrs.into_iter().filter_map(|p| self.font(&f, p)).collect();
        let weight_ptrs: Vec<((i32, i64), (i32, i64))> =
            v.get("m_FontWeightTable").array().iter().map(|w| (w.get("regularTypeface").pptr(), w.get("italicTypeface").pptr())).collect();
        let weights = weight_ptrs.into_iter().map(|(r, i)| (self.font(&f, r), self.font(&f, i))).collect();
        let dynamic_atlas = if v.get("m_AtlasPopulationMode").i64() == 1 { self.dynamic_atlas(&f, &v, &atlas_textures) } else { None };
        self.out.fonts[idx as usize] = TmpFont {
            name: v.get("m_Name").str().to_string(),
            file: f.name.clone(),
            path_id: id,
            dynamic: v.get("m_AtlasPopulationMode").i64() == 1,
            face,
            glyphs,
            characters,
            kerning,
            fallbacks,
            material,
            atlas_textures,
            atlas_width: v.get("m_AtlasWidth").f32(),
            atlas_height: v.get("m_AtlasHeight").f32(),
            atlas_padding: v.get("m_AtlasPadding").f32(),
            atlas_render_mode: v.get("m_AtlasRenderMode").i64(),
            weights,
            normal_style: v.get("normalStyle").f32(),
            normal_spacing_offset: v.get("normalSpacingOffset").f32(),
            bold_style: v.get("boldStyle").f32(),
            bold_spacing: v.get("boldSpacing").f32(),
            italic_style: v.get("italicStyle").f32(),
            tab_size: v.get("tabSize").f32(),
            dynamic_atlas,
        };
        Some(idx)
    }

    /// Every PPtr under `v`: (field path, pptr).
    fn collect(v: &Value, path: &mut String, out: &mut Vec<(String, (i32, i64))>) {
        if is_pptr(v) {
            let p = v.pptr();
            if p.1 != 0 {
                out.push((path.clone(), p));
            }
            return;
        }
        let len = path.len();
        match v {
            Value::Struct(fields) => {
                for (k, x) in fields {
                    if &**k == "m_GameObject" || &**k == "m_Script" {
                        continue;
                    }
                    if !path.is_empty() {
                        path.push('/');
                    }
                    path.push_str(k);
                    Self::collect(x, path, out);
                    path.truncate(len);
                }
            }
            Value::Array(a) => {
                for (i, x) in a.iter().enumerate() {
                    use std::fmt::Write;
                    let _ = write!(path, "/{i}");
                    Self::collect(x, path, out);
                    path.truncate(len);
                }
            }
            _ => {}
        }
    }
}

/// Shader property defaults (m_ParsedForm.m_PropInfo) as a material: what `new Material(shader)` holds.
pub fn shader_default_material(shader: &Value, name: &str) -> MaterialProps {
    let mut m = MaterialProps { name: name.to_string(), render_queue: -1, ..Default::default() };
    for p in shader.get("m_ParsedForm").get("m_PropInfo").get("m_Props").array() {
        let n = p.get("m_Name").str().to_string();
        let d = |i: usize| p.get(&format!("m_DefValue[{i}]")).f32();
        match p.get("m_Type").i64() {
            0 | 1 => {
                m.colors.insert(n, [d(0), d(1), d(2), d(3)]);
            }
            2 | 3 | 5 => {
                m.floats.insert(n, d(0));
            }
            4 => {
                m.textures.insert(n, crate::shader::TexEnv { texture: (0, 0), scale: [1.0, 1.0], offset: [0.0, 0.0] });
            }
            _ => {}
        }
    }
    m
}

pub fn load_ui_assets(db: &mut AssetDb, def: &SceneDef) -> UiAssets {
    let scene = match db.file(&def.scene_file) {
        Ok(f) => f,
        Err(e) => return UiAssets { warnings: vec![format!("scene file: {e}")], ..Default::default() },
    };
    let mut l = Loader {
        db,
        out: UiAssets::default(),
        textures: HashMap::new(),
        sprites: HashMap::new(),
        fonts: HashMap::new(),
        materials: HashMap::new(),
        legacy: HashMap::new(),
    };
    // Canvas.GetDefaultCanvasMaterial
    if let Ok(f) = l.db.file(UI_DEFAULT_SHADER.0) {
        if let Ok(v) = f.read_id(UI_DEFAULT_SHADER.1) {
            let props = shader_default_material(&v, "Default UI Material");
            l.out.materials.push(UiMaterial {
                name: props.name.clone(),
                props: Arc::new(props),
                file: f.name.clone(),
                path_id: 0,
                shader: (f.name.clone(), UI_DEFAULT_SHADER.1),
                main_tex: None,
            });
            l.out.default_material = Some(l.out.materials.len() as u32 - 1);
        }
    }
    if l.out.default_material.is_none() {
        l.out.warnings.push("UI/Default shader not found: default canvas material missing".into());
    }
    l.tmp_settings();
    let mut files: HashMap<String, Arc<SerializedFile>> = HashMap::new();
    for (si, s) in def.scripts.iter().enumerate() {
        let file = match &s.file {
            None => scene.clone(),
            Some(name) => match files.get(name) {
                Some(f) => f.clone(),
                None => match l.db.file(name) {
                    Ok(f) => {
                        files.insert(name.clone(), f.clone());
                        f
                    }
                    Err(_) => continue,
                },
            },
        };
        let graphic = GRAPHIC_CLASSES.contains(&s.class.as_str());
        let mut ptrs = Vec::new();
        Loader::collect(&s.data, &mut String::new(), &mut ptrs);
        for (path, p) in ptrs {
            // scene-local objects are GameObjects / components, never UI assets
            if p.0 == 0 && s.file.is_none() {
                continue;
            }
            let Ok(Some((f, id))) = l.db.resolve(&file, p) else { continue };
            let Some(class) = f.object(id).map(|o| o.class_id) else { continue };
            let r = match class {
                CLASS_SPRITE => l.sprite(&file, p).map(UiRef::Sprite),
                CLASS_TEXTURE2D => l.texture(&file, p).map(UiRef::Texture),
                CLASS_FONT => l.legacy_font(&file, p).map(UiRef::LegacyFont),
                CLASS_MATERIAL if graphic => l.material(&file, p).map(UiRef::Material),
                CLASS_MONOBEHAVIOUR if graphic || path.contains("font") || path.contains("Font") => l.font(&file, p).map(UiRef::Font),
                _ => None,
            };
            if let Some(r) = r {
                l.out.refs.insert((si as u32, path), r);
            }
        }
    }
    l.out
}
