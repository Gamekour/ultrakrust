//! TextMeshProUGUI meshing: literal ports of TMP_Text's text processing (PopulateTextProcessingArray,
//! TextMeshProUGUI.SetArraySizes, ValidateHtmlTag) and TextMeshProUGUI.GenerateTextMesh (layout,
//! word wrapping, auto sizing, truncation, alignment, SDF uv2 packing), with TMP_MaterialManager's
//! fallback materials and ShaderUtilities' scale ratios / padding.
//!
//! Output is in the text's RectTransform local space (Unity axes); `ugui` places and clips it.
//! Not ported (none of the scenes use them): sprites, Ellipsis/Page/Linked overflow, RTL, the
//! underline / strikethrough / highlight meshes, vertex gradients, non-Character uv mapping, tags
//! other than b i u s size color alpha noparse lowercase uppercase smallcaps /line-height /width.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use bevy_math::{Vec2, Vec3, Vec4};
use uk_assets::serialized::Value;
use uk_assets::shader::MaterialProps;
use uk_assets::ui::{TmpFont, UiAssets, SYNTH_GLYPH};

use crate::ugui::UiVertex;

pub const BOLD: u32 = 1;
pub const ITALIC: u32 = 2;
pub const UNDERLINE: u32 = 4;
pub const LOWER: u32 = 8;
pub const UPPER: u32 = 16;
pub const SMALLCAPS: u32 = 32;
pub const STRIKE: u32 = 64;

const LARGE_POS: f32 = 32767.0;
const LARGE_NEG: f32 = -32767.0;
const AUTOSIZE_MAX_ITERATIONS: i32 = 100;
const OVERFLOW: i64 = 0;
const TRUNCATE: i64 = 3;

/// The serialized TextMeshProUGUI fields the mesher reads.
#[derive(Clone, Debug)]
pub struct TmpDef {
    pub text: Arc<str>,
    pub font: Option<u32>,
    pub material: Option<u32>,
    pub font_size: f32,
    pub size_base: f32,
    pub size_min: f32,
    pub size_max: f32,
    pub autosize: bool,
    pub weight: i32,
    pub style: u32,
    pub h_align: i32,
    pub v_align: i32,
    pub char_spacing: f32,
    pub word_spacing: f32,
    pub line_spacing: f32,
    pub line_spacing_max: f32,
    pub paragraph_spacing: f32,
    pub cwa_max: f32,
    pub wrap: bool,
    pub wrap_ratios: f32,
    pub overflow: i64,
    pub kerning: bool,
    pub extra_padding: bool,
    pub rich: bool,
    pub parse_ctrl: bool,
    /// m_margin: (left, top, right, bottom)
    pub margin: [f32; 4],
    pub override_html: bool,
}

impl TmpDef {
    pub fn from_script(d: &Value, assets: &UiAssets, si: u32, warn: &mut Vec<String>, path: &str) -> TmpDef {
        let m = d.get("m_margin");
        let overflow = d.get("m_overflowMode").i64();
        if overflow != OVERFLOW && overflow != TRUNCATE && overflow != 2 && overflow != 4 {
            warn.push(format!("{path}: TMP overflow mode {overflow} drawn as Overflow"));
        }
        if d.get("m_enableVertexGradient").bool() {
            warn.push(format!("{path}: TMP vertex gradient not ported"));
        }
        if d.get("m_horizontalMapping").i64() != 0 || d.get("m_verticalMapping").i64() != 0 {
            warn.push(format!("{path}: TMP uv mapping not ported"));
        }
        if d.get("m_isRightToLeft").bool() {
            warn.push(format!("{path}: TMP right-to-left not ported"));
        }
        TmpDef {
            text: Arc::from(d.get("m_text").str()),
            font: assets.font(si, "m_fontAsset"),
            material: assets.material(si, "m_sharedMaterial"),
            font_size: d.get("m_fontSize").f32(),
            size_base: d.get("m_fontSizeBase").f32(),
            size_min: d.get("m_fontSizeMin").f32(),
            size_max: d.get("m_fontSizeMax").f32(),
            autosize: d.get("m_enableAutoSizing").bool(),
            weight: d.get("m_fontWeight").i64() as i32,
            style: d.get("m_fontStyle").i64() as u32,
            h_align: d.get("m_HorizontalAlignment").i64() as i32,
            v_align: d.get("m_VerticalAlignment").i64() as i32,
            char_spacing: d.get("m_characterSpacing").f32(),
            word_spacing: d.get("m_wordSpacing").f32(),
            line_spacing: d.get("m_lineSpacing").f32(),
            line_spacing_max: d.get("m_lineSpacingMax").f32(),
            paragraph_spacing: d.get("m_paragraphSpacing").f32(),
            cwa_max: d.get("m_charWidthMaxAdj").f32(),
            wrap: d.get("m_enableWordWrapping").bool(),
            wrap_ratios: d.get("m_wordWrappingRatios").f32(),
            overflow: if overflow == TRUNCATE { TRUNCATE } else { OVERFLOW },
            kerning: d.get("m_enableKerning").bool(),
            extra_padding: d.get("m_enableExtraPadding").bool(),
            rich: d.get("m_isRichText").bool(),
            parse_ctrl: d.get("m_parseCtrlCharacters").bool(),
            margin: [m.get("x").f32(), m.get("y").f32(), m.get("z").f32(), m.get("w").f32()],
            override_html: d.get("m_overrideHtmlColors").bool(),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// runtime materials (TMP_MaterialManager fallback materials + ShaderUtilities.UpdateShaderRatios)

/// A material instance TMP draws with: an asset material (ratios updated) or a fallback copy.
#[derive(Clone, Debug)]
pub struct RtMat {
    /// the asset material whose shader / textures / render state this copy shares
    pub base: u32,
    pub props: Arc<MaterialProps>,
    /// `_MainTex` (index into `UiAssets::textures`)
    pub tex: Option<u32>,
}

#[derive(Default, Debug)]
pub struct RtMats {
    pub list: Vec<RtMat>,
    by_asset: HashMap<u32, u32>,
    /// TMP_MaterialManager.m_fallbackMaterials: (source material, target texture)
    fallback: HashMap<(u32, Option<u32>), u32>,
}

fn has(p: &MaterialProps, n: &str) -> bool {
    p.floats.contains_key(n) || p.colors.contains_key(n) || p.textures.contains_key(n)
}

fn getf(p: &MaterialProps, n: &str) -> f32 {
    p.floats.get(n).copied().unwrap_or(0.0)
}

/// ShaderUtilities.UpdateShaderRatios (m_clamp 1)
fn update_shader_ratios(p: &mut MaterialProps) {
    let flag = !p.keywords.iter().any(|k| k == "RATIOS_OFF");
    if has(p, "_GradientScale") && has(p, "_FaceDilate") {
        let gs = getf(p, "_GradientScale");
        let dilate = getf(p, "_FaceDilate");
        let ow = getf(p, "_OutlineWidth");
        let os = getf(p, "_OutlineSoftness");
        let num4 = getf(p, "_WeightNormal").max(getf(p, "_WeightBold")) / 4.0;
        let mut num5 = (num4 + dilate + ow + os).max(1.0);
        let a = if flag { (gs - 1.0) / (gs * num5) } else { 1.0 };
        p.floats.insert("_ScaleRatioA".into(), a);
        if has(p, "_GlowOffset") {
            let go = getf(p, "_GlowOffset");
            let gouter = getf(p, "_GlowOuter");
            let num6 = (num4 + dilate) * (gs - 1.0);
            num5 = (go + gouter).max(1.0);
            let b = if flag { (gs - 1.0 - num6).max(0.0) / (gs * num5) } else { 1.0 };
            p.floats.insert("_ScaleRatioB".into(), b);
        }
        if has(p, "_UnderlayOffsetX") {
            let ox = getf(p, "_UnderlayOffsetX");
            let oy = getf(p, "_UnderlayOffsetY");
            let ud = getf(p, "_UnderlayDilate");
            let us = getf(p, "_UnderlaySoftness");
            let num7 = (num4 + dilate) * (gs - 1.0);
            num5 = (ox.abs().max(oy.abs()) + ud + us).max(1.0);
            let c = if flag { (gs - 1.0 - num7).max(0.0) / (gs * num5) } else { 1.0 };
            p.floats.insert("_ScaleRatioC".into(), c);
        }
    }
}

/// ShaderUtilities.GetPadding (the ratios are already current)
fn get_padding(p: &MaterialProps, extra: bool) -> f32 {
    let num = if extra { 4.0 } else { 0.0 };
    if !has(p, "_GradientScale") {
        let pad = if has(p, "_Padding") { getf(p, "_Padding") as i32 as f32 } else { 0.0 };
        return num + pad + 1.0;
    }
    let kw = |k: &str| p.keywords.iter().any(|x| x == k);
    let num5 = if has(p, "_ScaleRatioA") { getf(p, "_ScaleRatioA") } else { 0.0 };
    let n2 = if has(p, "_FaceDilate") { getf(p, "_FaceDilate") * num5 } else { 0.0 };
    let n3 = if has(p, "_OutlineSoftness") { getf(p, "_OutlineSoftness") * num5 } else { 0.0 };
    let n4 = if has(p, "_OutlineWidth") { getf(p, "_OutlineWidth") * num5 } else { 0.0 };
    let mut n10 = n4 + n3 + n2;
    let (mut n8, mut n9) = (0.0, 0.0);
    if has(p, "_GlowOffset") && kw("GLOW_ON") {
        let n6 = if has(p, "_ScaleRatioB") { getf(p, "_ScaleRatioB") } else { 0.0 };
        n8 = getf(p, "_GlowOffset") * n6;
        n9 = getf(p, "_GlowOuter") * n6;
    }
    n10 = n10.max(n2 + n8 + n9);
    let mut z = Vec4::ZERO;
    if has(p, "_UnderlaySoftness") && kw("UNDERLAY_ON") {
        let n7 = if has(p, "_ScaleRatioC") { getf(p, "_ScaleRatioC") } else { 0.0 };
        let ox = getf(p, "_UnderlayOffsetX") * n7;
        let oy = getf(p, "_UnderlayOffsetY") * n7;
        let ud = getf(p, "_UnderlayDilate") * n7;
        let us = getf(p, "_UnderlaySoftness") * n7;
        z.x = z.x.max(n2 + ud + us - ox);
        z.y = z.y.max(n2 + ud + us - oy);
        z.z = z.z.max(n2 + ud + us + ox);
        z.w = z.w.max(n2 + ud + us + oy);
    }
    z = z.max(Vec4::splat(n10)) + Vec4::splat(num);
    z = z.min(Vec4::ONE) * getf(p, "_GradientScale");
    z.max_element() + 1.25
}

impl RtMats {
    /// the asset material itself (its ratios updated, as GetPadding does on the shared material)
    pub fn asset(&mut self, assets: &UiAssets, m: u32) -> u32 {
        if let Some(&r) = self.by_asset.get(&m) {
            return r;
        }
        let am = &assets.materials[m as usize];
        let mut props = (*am.props).clone();
        update_shader_ratios(&mut props);
        self.list.push(RtMat { base: m, props: Arc::new(props), tex: am.main_tex });
        let r = self.list.len() as u32 - 1;
        self.by_asset.insert(m, r);
        r
    }

    /// TMP_MaterialManager.GetFallbackMaterial(source, target)
    fn fallback(&mut self, assets: &UiAssets, src: u32, target: u32) -> u32 {
        let tex = assets.materials[target as usize].main_tex;
        if let Some(&r) = self.fallback.get(&(src, tex)) {
            return r;
        }
        let tp = &assets.materials[target as usize].props;
        let s = &self.list[src as usize];
        let (base, mut props) = if has(&s.props, "_GradientScale") && has(tp, "_GradientScale") {
            let mut p = (*s.props).clone();
            for n in ["_GradientScale", "_TextureWidth", "_TextureHeight", "_WeightNormal", "_WeightBold"] {
                p.floats.insert(n.into(), getf(tp, n));
            }
            (s.base, p)
        } else {
            (target, (**tp).clone())
        };
        update_shader_ratios(&mut props);
        self.list.push(RtMat { base, props: Arc::new(props), tex });
        let r = self.list.len() as u32 - 1;
        self.fallback.insert((src, tex), r);
        r
    }

    /// TMP_MaterialManager.GetFallbackMaterial(fontAsset, source, atlasIndex)
    fn atlas(&mut self, font: &TmpFont, src: u32, atlas_index: u32) -> u32 {
        let tex = font.atlas_textures.get(atlas_index as usize).copied().flatten();
        if let Some(&r) = self.fallback.get(&(src, tex)) {
            return r;
        }
        let s = self.list[src as usize].clone();
        self.list.push(RtMat { base: s.base, props: s.props, tex });
        let r = self.list.len() as u32 - 1;
        self.fallback.insert((src, tex), r);
        r
    }
}

// ---------------------------------------------------------------------------------------------
// output

#[derive(Clone, Debug)]
pub struct TmpSub {
    pub rt: u32,
    pub verts: Vec<UiVertex>,
    pub idx: Vec<u32>,
    /// mesh bounds (xy min, max), including the unused zero vertices at the origin
    pub min: Vec2,
    pub max: Vec2,
}

/// [0] the text's own mesh, [1..] its TMP_SubMeshUI meshes (empty when the text cleared its mesh)
#[derive(Clone, Debug, Default)]
pub struct TmpMesh {
    pub subs: Vec<TmpSub>,
    pub truncated: bool,
    pub font_size: f32,
}

impl TmpMesh {
    /// GetCompoundBounds (xy)
    pub fn compound_bounds(&self) -> Option<(Vec2, Vec2)> {
        let first = self.subs.first()?;
        let (mut mn, mut mx) = (first.min, first.max);
        for s in &self.subs[1..] {
            mn = mn.min(s.min);
            mx = mx.max(s.max);
        }
        Some((mn, mx))
    }
}

/// Per-call inputs that are not serialized fields.
pub struct TmpInput<'a> {
    pub def: &'a TmpDef,
    pub assets: &'a UiAssets,
    /// None: the serialized text (TextInputBox: escape sequences parsed)
    pub text: Option<&'a str>,
    /// Graphic.color (m_fontColor)
    pub color: [f32; 4],
    /// RectTransform.rect (x, y, w, h)
    pub rect: [f32; 4],
    /// the render-mode lossy factor GenerateTextMesh multiplies uv2.y by
    pub uv2_scale: f32,
}

// ---------------------------------------------------------------------------------------------
// character helpers (System.Char on UTF-16 code units)

fn is_whitespace(c: u32) -> bool {
    char::from_u32(c).is_some_and(|c| c.is_whitespace())
}

/// char.IsSeparator: Zs, Zl, Zp
fn is_separator(c: u32) -> bool {
    matches!(c, 0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000)
}

fn is_control(c: u32) -> bool {
    c < 0x20 || (0x7F..=0x9F).contains(&c)
}

fn is_lower(c: u32) -> bool {
    char::from_u32(c & 0xFFFF).is_some_and(|c| c.is_lowercase())
}

fn is_upper(c: u32) -> bool {
    char::from_u32(c & 0xFFFF).is_some_and(|c| c.is_uppercase())
}

fn to_upper(c: u32) -> u32 {
    char::from_u32(c & 0xFFFF).and_then(|c| c.to_uppercase().next()).map_or(c, |u| if (u as u32) <= 0xFFFF { u as u32 } else { c })
}

fn to_lower(c: u32) -> u32 {
    char::from_u32(c & 0xFFFF).and_then(|c| c.to_lowercase().next()).map_or(c, |u| if (u as u32) <= 0xFFFF { u as u32 } else { c })
}

fn hex_to_int(c: u32) -> u32 {
    match c {
        0x30..=0x39 => c - 0x30,
        0x41..=0x46 => c - 0x41 + 10,
        0x61..=0x66 => c - 0x61 + 10,
        _ => 15,
    }
}

const LOOKUP_U: &[u8] = b"-------------------------------- !-#$%&-()*+,-./0123456789:;<=>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ[-]^_`ABCDEFGHIJKLMNOPQRSTUVWXYZ{|}~-";

fn to_upper_ascii_fast(c: u32) -> u32 {
    if c as usize > LOOKUP_U.len() - 1 {
        c
    } else {
        LOOKUP_U[c as usize] as u32
    }
}

/// HexCharsToColor(char[], int length)
fn hex_color(t: &[u32], n: usize) -> [u8; 4] {
    let h = |i: usize| hex_to_int(t[i]);
    let b = |hi: usize, lo: usize| (h(hi) * 16 + h(lo)) as u8;
    match n {
        4 => [b(1, 1), b(2, 2), b(3, 3), 255],
        5 => [b(1, 1), b(2, 2), b(3, 3), b(4, 4)],
        7 => [b(1, 2), b(3, 4), b(5, 6), 255],
        9 => [b(1, 2), b(3, 4), b(5, 6), b(7, 8)],
        10 => [b(7, 7), b(8, 8), b(9, 9), 255],
        11 => [b(7, 7), b(8, 8), b(9, 9), b(10, 10)],
        13 => [b(7, 8), b(9, 10), b(11, 12), 255],
        15 => [b(7, 8), b(9, 10), b(11, 12), b(13, 14)],
        _ => [255; 4],
    }
}

/// HexCharsToColor(char[], int startIndex, int length)
fn hex_color_at(t: &[u32], s: usize, n: usize) -> [u8; 4] {
    let h = |i: usize| hex_to_int(t[s + i]);
    let b = |hi: usize, lo: usize| (h(hi) * 16 + h(lo)) as u8;
    match n {
        7 => [b(1, 2), b(3, 4), b(5, 6), 255],
        9 => [b(1, 2), b(3, 4), b(5, 6), b(7, 8)],
        _ => [255; 4],
    }
}

/// ConvertToFloat(chars, startIndex, length)
fn convert_to_float(t: &[u32], start: usize, len: usize) -> f32 {
    if start == 0 {
        return -32768.0;
    }
    let end = start + len;
    let mut i = start;
    let mut sign = 1.0;
    if t[i] == '+' as u32 {
        i += 1;
    } else if t[i] == '-' as u32 {
        sign = -1.0;
        i += 1;
    }
    let mut int_part = true;
    let mut frac = 0.0f32;
    let mut v = 0.0f32;
    while i < end {
        let c = t[i];
        i += 1;
        if !(0x30..=0x39).contains(&c) {
            match c {
                0x2E => {}
                0x2C => return if v > 32767.0 { -32768.0 } else { v },
                _ => continue,
            }
        }
        if c == 0x2E {
            int_part = false;
            frac = 0.1;
        } else if int_part {
            v = v * 10.0 + (c - 0x30) as f32 * sign;
        } else {
            v += (c - 0x30) as f32 * frac * sign;
            frac *= 0.1;
        }
    }
    if v > 32767.0 {
        -32768.0
    } else {
        v
    }
}

/// PopulateTextProcessingArray (no default style; `<style>` has no style sheet)
fn populate(text: &[u32], input_box: bool, parse_ctrl: bool, rich: bool) -> Vec<u32> {
    let count = text.len();
    let mut out = Vec::with_capacity(count + 1);
    let at = |i: usize| text.get(i).copied().unwrap_or(0);
    let mut i = 0;
    while i < count {
        let c = text[i];
        if c == 0 {
            break;
        }
        if input_box && c == 92 && i < count - 1 {
            match text[i + 1] {
                92 if parse_ctrl && count > i + 2 => {
                    out.push(text[i + 1]);
                    out.push(text[i + 2]);
                    i += 3;
                    continue;
                }
                110 | 114 | 116 | 118 if parse_ctrl => {
                    out.push(match text[i + 1] {
                        110 => 10,
                        114 => 13,
                        116 => 9,
                        _ => 11,
                    });
                    i += 2;
                    continue;
                }
                117 if count > i + 5 => {
                    out.push((0..4).fold(0, |a, k| a + (hex_to_int(at(i + 2 + k)) << (12 - 4 * k))));
                    i += 6;
                    continue;
                }
                85 if count > i + 9 => {
                    out.push((0..8).fold(0u32, |a, k| a.wrapping_add(hex_to_int(at(i + 2 + k)) << (28 - 4 * k))));
                    i += 10;
                    continue;
                }
                _ => {}
            }
        }
        if (0xD800..=0xDBFF).contains(&c) && count > i + 1 && (0xDC00..=0xDFFF).contains(&text[i + 1]) {
            out.push(((c - 0xD800) << 10) + (text[i + 1] - 0xDC00) + 0x10000);
            i += 2;
            continue;
        }
        if c == 60 && rich {
            let mut h: i32 = 0;
            let mut r = i + 1;
            while r < i + 17 && r < count {
                let ch = text[r];
                if ch == 62 || ch == 61 || ch == 32 {
                    break;
                }
                h = (h << 5).wrapping_add(h) ^ to_upper_ascii_fast(ch) as i32;
                r += 1;
            }
            match h {
                2256 => {
                    out.push(10);
                    i += 4;
                    continue;
                }
                2869039 => {
                    out.push(160);
                    i += 6;
                    continue;
                }
                3288238 => {
                    out.push(8203);
                    i += 6;
                    continue;
                }
                1927738392 => {
                    i += 8;
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
        i += 1;
    }
    out.push(0);
    out
}

// ---------------------------------------------------------------------------------------------
// generator state

#[derive(Clone)]
struct Stack<T: Copy + Default> {
    items: Vec<T>,
    index: usize,
}

impl<T: Copy + Default> Stack<T> {
    fn new(cap: usize) -> Self {
        Stack { items: vec![T::default(); cap], index: 0 }
    }
    fn set_default(&mut self, v: T) {
        self.items[0] = v;
        self.index = 1;
    }
    fn add(&mut self, v: T) {
        if self.index < self.items.len() {
            self.items[self.index] = v;
            self.index += 1;
        }
    }
    fn remove(&mut self) -> T {
        self.index = self.index.saturating_sub(1);
        if self.index == 0 {
            self.index = 1;
            return self.items[0];
        }
        self.items[self.index - 1]
    }
    fn peek(&self) -> T {
        self.items[self.index.saturating_sub(1)]
    }
}

/// TMP_FontStyleStack
#[derive(Clone, Copy, Default)]
struct StyleStack {
    bold: u8,
    italic: u8,
    underline: u8,
    upper: u8,
    lower: u8,
    strike: u8,
}

impl StyleStack {
    fn slot(&mut self, s: u32) -> Option<&mut u8> {
        match s {
            BOLD => Some(&mut self.bold),
            ITALIC => Some(&mut self.italic),
            UNDERLINE => Some(&mut self.underline),
            UPPER => Some(&mut self.upper),
            LOWER => Some(&mut self.lower),
            STRIKE => Some(&mut self.strike),
            _ => None,
        }
    }
    fn add(&mut self, s: u32) -> u8 {
        self.slot(s).map_or(0, |v| {
            *v = v.wrapping_add(1);
            *v
        })
    }
    fn remove(&mut self, s: u32) -> u8 {
        self.slot(s).map_or(0, |v| {
            *v = if *v > 1 { *v - 1 } else { 0 };
            *v
        })
    }
}

#[derive(Clone, Copy, Default)]
struct CharInfo {
    character: u32,
    has_elem: bool,
    font: u32,
    /// key into the font's glyphs
    glyph: u32,
    glyph_index: u32,
    elem_scale: f32,
    alt: bool,
    mat_idx: usize,
    mat: u32,
    point_size: f32,
    color: [u8; 4],
    style: u32,
    scale: f32,
    visible: bool,
    bl: Vec2,
    tl: Vec2,
    tr: Vec2,
    br: Vec2,
    origin: f32,
    xadv: f32,
    asc: f32,
    desc: f32,
    adj_asc: f32,
    adj_desc: f32,
    baseline: f32,
    aspect: f32,
    line: i32,
    /// vertex_BL, TL, TR, BR positions
    v: [Vec2; 4],
    uv: [Vec2; 4],
    vcolor: [u8; 4],
}

#[derive(Clone, Copy)]
struct LineInfo {
    char_count: i32,
    visible_count: i32,
    space_count: i32,
    control_count: i32,
    first: i32,
    first_visible: i32,
    last: i32,
    last_visible: i32,
    width: f32,
    asc: f32,
    desc: f32,
    baseline: f32,
    length: f32,
    line_height: f32,
    max_advance: f32,
    margin_left: f32,
    margin_right: f32,
    alignment: i32,
    ext_min: Vec2,
    ext_max: Vec2,
}

impl Default for LineInfo {
    fn default() -> Self {
        LineInfo {
            char_count: 0,
            visible_count: 0,
            space_count: 0,
            control_count: 0,
            first: 0,
            first_visible: 0,
            last: 0,
            last_visible: 0,
            width: 0.0,
            asc: LARGE_NEG,
            desc: LARGE_POS,
            baseline: 0.0,
            length: 0.0,
            line_height: 0.0,
            max_advance: 0.0,
            margin_left: 0.0,
            margin_right: 0.0,
            alignment: 0,
            ext_min: Vec2::splat(LARGE_POS),
            ext_max: Vec2::splat(LARGE_NEG),
        }
    }
}

#[derive(Clone, Copy)]
struct MatRef {
    rt: u32,
    ref_count: i32,
}

/// WordWrapState (stacks by index: their arrays are shared)
#[derive(Clone, Copy, Default)]
struct WrapState {
    font: u32,
    mat: u32,
    mat_idx: usize,
    prev_break: i32,
    total: i32,
    visible: i32,
    first: i32,
    first_vis: i32,
    last_vis: i32,
    style: u32,
    italic: i32,
    fsm: f32,
    cfs: f32,
    xadv: f32,
    max_cap: f32,
    max_asc: f32,
    elem_desc: f32,
    sola: f32,
    mla: f32,
    mld: f32,
    page_asc: f32,
    mesh_min: Vec2,
    mesh_max: Vec2,
    line_number: i32,
    line_offset: f32,
    baseline_offset: f32,
    driven: bool,
    glyph_adj: f32,
    cspace: f32,
    just: i32,
    ml: f32,
    mr: f32,
    html: [u8; 4],
    ucol: [u8; 4],
    scol: [u8; 4],
    noparse: bool,
    styles: StyleStack,
    /// italic, color, underline color, strike color, size, weight
    idx: [usize; 6],
    line_info: Option<LineInfo>,
}

struct Gen<'a> {
    def: &'a TmpDef,
    assets: &'a UiAssets,
    rt: &'a mut RtMats,
    chars: Vec<u32>,
    main_font: u32,
    shared_mat: u32,
    padding: f32,
    sub_padding: Vec<f32>,
    is_sdf: bool,
    // TMP_Text fields
    font_size: f32,
    max_font_size: f32,
    min_font_size: f32,
    line_spacing_delta: f32,
    cwa: f32,
    autosize_count: i32,
    truncated: bool,
    total: i32,
    using_bold: bool,
    font_color32: [u8; 4],
    current_font: u32,
    current_mat: u32,
    current_mat_idx: usize,
    style: u32,
    weight: i32,
    styles: StyleStack,
    weight_stack: Stack<i32>,
    size_stack: Stack<f32>,
    color_stack: Stack<[u8; 4]>,
    ucolor_stack: Stack<[u8; 4]>,
    scolor_stack: Stack<[u8; 4]>,
    italic_stack: Stack<i32>,
    html: [u8; 4],
    ucol: [u8; 4],
    scol: [u8; 4],
    italic: i32,
    cfs: f32,
    fsm: f32,
    line_offset: f32,
    line_height: f32,
    cspacing: f32,
    xadv: f32,
    cc: i32,
    first_char_of_line: i32,
    last_char_of_line: i32,
    first_vis_of_line: i32,
    last_vis_of_line: i32,
    mla: f32,
    mld: f32,
    line_number: i32,
    sola: f32,
    line_visible: i32,
    driven: bool,
    first_overflow: i32,
    max_cap: f32,
    max_text_asc: f32,
    elem_asc: f32,
    elem_desc: f32,
    page_asc: f32,
    mesh_min: Vec2,
    mesh_max: Vec2,
    new_page: bool,
    noparse: bool,
    just: i32,
    margin_left: f32,
    margin_right: f32,
    width: f32,
    baseline_offset: f32,
    glyph_adj: f32,
    // ValidateHtmlTag scratch (persists like m_htmlTag)
    html_tag: [u32; 128],
    ci: Vec<CharInfo>,
    lines: Vec<LineInfo>,
    refs: Vec<MatRef>,
    ref_lookup: HashMap<u32, usize>,
    s_wrap: WrapState,
    s_line: WrapState,
    s_valid: WrapState,
    s_soft: WrapState,
    space_count: i32,
}

#[derive(Clone, Copy, Default)]
struct Attr {
    name: i32,
    value: i32,
    ty: u8,
    unit: u8,
    start: usize,
    len: usize,
}

const TV_NONE: u8 = 0;
const TV_NUM: u8 = 1;
const TV_STR: u8 = 2;
const TV_COLOR: u8 = 3;
const TU_PIXELS: u8 = 0;
const TU_FONT: u8 = 1;
const TU_PERCENT: u8 = 2;

impl<'a> Gen<'a> {
    fn font(&self, f: u32) -> &'a TmpFont {
        &self.assets.fonts[f as usize]
    }

    fn line(&mut self, n: i32) -> &mut LineInfo {
        let n = n.max(0) as usize;
        if n >= self.lines.len() {
            self.lines.resize(n + 1, LineInfo::default());
        }
        &mut self.lines[n]
    }

    fn c(&mut self, i: i32) -> &mut CharInfo {
        let i = i.max(0) as usize;
        if i >= self.ci.len() {
            self.ci.resize(i + 1, CharInfo::default());
        }
        &mut self.ci[i]
    }

    fn cr(&self, i: i32) -> CharInfo {
        self.ci.get(i.max(0) as usize).copied().unwrap_or_default()
    }

    fn add_ref(&mut self, rt: u32) -> usize {
        if let Some(&i) = self.ref_lookup.get(&rt) {
            return i;
        }
        let i = self.refs.len();
        self.refs.push(MatRef { rt, ref_count: 0 });
        self.ref_lookup.insert(rt, i);
        i
    }

    // TMP_FontAssetUtilities.GetCharacterFromFontAsset_Internal: (font, character, alt)
    fn char_internal(&self, u: u32, f: u32, fallbacks: bool, visited: &mut HashSet<u32>) -> Option<(u32, u32, f32, bool)> {
        let fa = self.font(f);
        let italic = self.style & ITALIC != 0;
        if italic || self.weight != 400 {
            let num = match self.weight {
                100 => 1,
                200 => 2,
                300 => 3,
                500 => 5,
                600 => 6,
                700 => 7,
                800 => 8,
                900 => 9,
                _ => 4,
            };
            let face = fa.weights.get(num).and_then(|w| if italic { w.1 } else { w.0 });
            if let Some(face) = face {
                if let Some(c) = self.font(face).characters.get(&u) {
                    return Some((face, c.glyph, c.scale, true));
                }
            }
        }
        if let Some(c) = fa.characters.get(&u) {
            return Some((f, c.glyph, c.scale, false));
        }
        if fallbacks {
            for &fb in &fa.fallbacks {
                if visited.insert(fb) {
                    if let Some(r) = self.char_internal(u, fb, true, visited) {
                        return Some(r);
                    }
                }
            }
        }
        None
    }

    fn char_from_font(&self, u: u32, f: u32, fallbacks: bool) -> Option<(u32, u32, f32, bool)> {
        self.char_internal(u, f, fallbacks, &mut HashSet::new())
    }

    /// TMP_Text.GetTextElement
    fn text_element(&self, u: u32, f: u32) -> Option<(u32, u32, f32, bool)> {
        if let Some(r) = self.char_from_font(u, f, false) {
            return Some(r);
        }
        let mut visited = HashSet::new();
        for &fb in &self.font(f).fallbacks {
            if let Some(r) = self.char_internal(u, fb, true, &mut visited) {
                return Some(r);
            }
        }
        None
    }

    /// TextMeshProUGUI.SetArraySizes
    fn set_array_sizes(&mut self) {
        self.total = 0;
        self.using_bold = false;
        self.noparse = false;
        self.style = self.def.style;
        self.styles = StyleStack::default();
        self.weight = if self.style & BOLD != 0 { 700 } else { self.def.weight };
        self.weight_stack.set_default(self.weight);
        self.current_font = self.main_font;
        self.current_mat = self.shared_mat;
        self.current_mat_idx = 0;
        self.ref_lookup.clear();
        self.refs.clear();
        self.add_ref(self.shared_mat);
        let mut i: i32 = -1;
        loop {
            i += 1;
            if !(i < self.chars.len() as i32 && self.chars[i as usize] != 0) {
                break;
            }
            let mut u = self.chars[i as usize];
            if self.def.rich && u == 60 {
                if let Some(end) = self.validate_tag(i as usize + 1) {
                    i = end as i32;
                    if self.style & BOLD != 0 {
                        self.using_bold = true;
                    }
                    continue;
                }
            }
            let mut flag = false;
            let prev = (self.current_font, self.current_mat, self.current_mat_idx);
            if self.style & UPPER != 0 {
                if is_lower(u) {
                    u = to_upper(u);
                }
            } else if self.style & LOWER != 0 {
                if is_upper(u) {
                    u = to_lower(u);
                }
            } else if self.style & SMALLCAPS != 0 && is_lower(u) {
                u = to_upper(u);
            }
            let mut el = self.text_element(u, self.current_font);
            if el.is_none() {
                // TMP_Settings.missingGlyphCharacter 0 -> 9633; no TMP_Settings fallbacks / default font
                for sub in [9633u32, 32, 3] {
                    u = sub;
                    self.chars[i as usize] = sub;
                    el = self.char_from_font(sub, self.current_font, true);
                    if el.is_some() {
                        break;
                    }
                }
            }
            let Some((ef, glyph, escale, alt)) = el else {
                // Unity throws here; the character is dropped
                continue;
            };
            if ef != self.current_font {
                flag = true;
                self.current_font = ef;
            }
            let cc = self.total;
            {
                let c = self.c(cc);
                *c = CharInfo::default();
                c.has_elem = true;
                c.glyph = glyph;
                c.glyph_index = if glyph == SYNTH_GLYPH { 0 } else { glyph };
                c.elem_scale = escale;
                c.alt = alt;
                c.character = u & 0xFFFF;
                c.font = ef;
            }
            if flag && self.current_font != self.main_font {
                // TMP_Settings.matchMaterialPreset
                if let Some(fm) = self.font(self.current_font).material {
                    self.current_mat = self.rt.fallback(self.assets, self.current_mat, fm);
                }
                self.current_mat_idx = self.add_ref(self.current_mat);
            }
            let atlas_index = self.font(ef).glyphs.get(&glyph).map_or(0, |g| g.atlas_index);
            if atlas_index > 0 {
                self.current_mat = self.rt.atlas(self.font(self.current_font), self.current_mat, atlas_index);
                self.current_mat_idx = self.add_ref(self.current_mat);
                flag = true;
            }
            if !is_whitespace(u & 0xFFFF) && u != 8203 {
                self.refs[self.current_mat_idx].ref_count += 1;
            }
            let (m, mi) = (self.current_mat, self.current_mat_idx);
            let c = self.c(cc);
            c.mat = m;
            c.mat_idx = mi;
            if flag {
                (self.current_font, self.current_mat, self.current_mat_idx) = prev;
            }
            self.total += 1;
        }
    }

    /// TMP_Text.ValidateHtmlTag (the ported tags); Some(endIndex) when valid
    fn validate_tag(&mut self, start: usize) -> Option<usize> {
        let mut num = 0usize;
        let mut b = 0u8;
        let mut a = 0usize;
        let mut at = [Attr::default(); 8];
        let mut ty = TV_NONE;
        let mut unit = TU_PIXELS;
        let mut end = start;
        let mut flag = false;
        let mut ok = false;
        let next = |at: &mut [Attr; 8], a: &mut usize| {
            *a = (*a + 1).min(7);
            at[*a] = Attr::default();
        };
        let mut i = start;
        while i < self.chars.len() && self.chars[i] != 0 {
            if num >= self.html_tag.len() {
                break;
            }
            let u = self.chars[i];
            if u == 60 {
                break;
            }
            if u == 62 {
                ok = true;
                end = i;
                self.html_tag[num] = 0;
                break;
            }
            self.html_tag[num] = u & 0xFFFF;
            num += 1;
            if b == 1 {
                match ty {
                    TV_NONE => match u {
                        43 | 45 | 46 | 48..=57 => {
                            unit = TU_PIXELS;
                            ty = TV_NUM;
                            at[a].ty = TV_NUM;
                            at[a].start = num - 1;
                            at[a].len += 1;
                        }
                        35 => {
                            unit = TU_PIXELS;
                            ty = TV_COLOR;
                            at[a].ty = TV_COLOR;
                            at[a].start = num - 1;
                            at[a].len += 1;
                        }
                        34 => {
                            unit = TU_PIXELS;
                            ty = TV_STR;
                            at[a].ty = TV_STR;
                            at[a].start = num;
                        }
                        _ => {
                            unit = TU_PIXELS;
                            ty = TV_STR;
                            at[a].ty = TV_STR;
                            at[a].start = num - 1;
                            at[a].value = (at[a].value << 5).wrapping_add(at[a].value) ^ u as i32;
                            at[a].len += 1;
                        }
                    },
                    TV_NUM => {
                        if u == 112 || u == 101 || u == 37 || u == 32 {
                            b = 2;
                            ty = TV_NONE;
                            unit = match u {
                                101 => TU_FONT,
                                37 => TU_PERCENT,
                                _ => TU_PIXELS,
                            };
                            at[a].unit = unit;
                            next(&mut at, &mut a);
                        } else if b != 2 {
                            at[a].len += 1;
                        }
                    }
                    TV_COLOR => {
                        if u != 32 {
                            at[a].len += 1;
                        } else {
                            b = 2;
                            ty = TV_NONE;
                            unit = TU_PIXELS;
                            next(&mut at, &mut a);
                        }
                    }
                    _ => {
                        if u != 34 {
                            at[a].value = (at[a].value << 5).wrapping_add(at[a].value) ^ u as i32;
                            at[a].len += 1;
                        } else {
                            b = 2;
                            ty = TV_NONE;
                            unit = TU_PIXELS;
                            next(&mut at, &mut a);
                        }
                    }
                }
            }
            if u == 61 {
                b = 1;
            }
            if b == 0 && u == 32 {
                if flag {
                    return None;
                }
                flag = true;
                b = 2;
                ty = TV_NONE;
                unit = TU_PIXELS;
                next(&mut at, &mut a);
            }
            if b == 0 {
                at[a].name = (at[a].name << 3).wrapping_sub(at[a].name).wrapping_add(u as i32);
            }
            if b == 2 && u == 32 {
                b = 0;
            }
            i += 1;
        }
        let _ = ty;
        if !ok {
            return None;
        }
        let t = self.html_tag;
        let name = at[0].name;
        if self.noparse && name != 53822163 && name != 49429939 {
            return None;
        }
        if name == 53822163 || name == 49429939 {
            self.noparse = false;
            return Some(end);
        }
        if t[0] == '#' as u32 && matches!(num, 4 | 5 | 7 | 9) {
            self.html = hex_color(&t, num);
            self.color_stack.add(self.html);
            return Some(end);
        }
        let base = self.def.style;
        match name {
            66 | 98 => {
                self.style |= BOLD;
                self.styles.add(BOLD);
                self.weight = 700;
            }
            395 | 427 => {
                if base & BOLD == 0 && self.styles.remove(BOLD) == 0 {
                    self.style &= !BOLD;
                    self.weight = self.weight_stack.peek();
                }
            }
            73 | 105 => {
                self.style |= ITALIC;
                self.styles.add(ITALIC);
                if at[1].name == 276531 || at[1].name == 186899 {
                    self.italic = convert_to_float(&t, at[1].start, at[1].len) as i32;
                    if self.italic < -180 || self.italic > 180 {
                        return None;
                    }
                } else {
                    self.italic = self.font(self.current_font).italic_style as i32;
                }
                self.italic_stack.add(self.italic);
            }
            402 | 434 => {
                if base & ITALIC == 0 {
                    self.italic = self.italic_stack.remove();
                    if self.styles.remove(ITALIC) == 0 {
                        self.style &= !ITALIC;
                    }
                }
            }
            83 | 115 => {
                self.style |= STRIKE;
                self.styles.add(STRIKE);
                if at[1].name == 281955 || at[1].name == 192323 {
                    self.scol = hex_color_at(&t, at[1].start, at[1].len);
                    self.scol[3] = self.html[3].min(self.scol[3]);
                } else {
                    self.scol = self.html;
                }
                self.scolor_stack.add(self.scol);
            }
            412 | 444 => {
                if base & STRIKE == 0 && self.styles.remove(STRIKE) == 0 {
                    self.style &= !STRIKE;
                }
                self.scol = self.scolor_stack.remove();
            }
            85 | 117 => {
                self.style |= UNDERLINE;
                self.styles.add(UNDERLINE);
                if at[1].name == 281955 || at[1].name == 192323 {
                    self.ucol = hex_color_at(&t, at[1].start, at[1].len);
                    self.ucol[3] = self.html[3].min(self.ucol[3]);
                } else {
                    self.ucol = self.html;
                }
                self.ucolor_stack.add(self.ucol);
            }
            414 | 446 => {
                if base & UNDERLINE == 0 {
                    self.ucol = self.ucolor_stack.remove();
                    if self.styles.remove(UNDERLINE) == 0 {
                        self.style &= !UNDERLINE;
                    }
                }
                self.ucol = self.ucolor_stack.remove();
            }
            32745 | 45545 => {
                let v = convert_to_float(&t, at[0].start, at[0].len);
                if v == -32768.0 {
                    return None;
                }
                self.cfs = match unit {
                    TU_PIXELS => {
                        if t[5] == '+' as u32 || t[5] == '-' as u32 {
                            self.font_size + v
                        } else {
                            v
                        }
                    }
                    TU_FONT => self.font_size * v,
                    _ => self.font_size * v / 100.0,
                };
                self.size_stack.add(self.cfs);
            }
            145592 | 158392 => self.cfs = self.size_stack.remove(),
            192323 | 281955 => {
                if t[6] == '#' as u32 && matches!(num, 10 | 11 | 13 | 15) {
                    self.html = hex_color(&t, num);
                } else {
                    self.html = match at[0].value {
                        125395 => [255, 0, 0, 255],
                        -992792864 => [173, 216, 230, 255],
                        3573310 => [0, 0, 255, 255],
                        3680713 => [128, 128, 128, 255],
                        117905991 => [0, 0, 0, 255],
                        121463835 => [0, 255, 0, 255],
                        140357351 => [255, 255, 255, 255],
                        26556144 => [255, 128, 0, 255],
                        -36881330 => [160, 32, 240, 255],
                        554054276 => [255, 235, 4, 255],
                        _ => return None,
                    };
                }
                self.color_stack.add(self.html);
            }
            982252 | 1071884 => self.html = self.color_stack.remove(),
            186622 | 276254 => {
                if at[0].len != 3 {
                    return None;
                }
                self.html[3] = (hex_to_int(t[7]) * 16 + hex_to_int(t[8])) as u8;
            }
            1750458 => return None,
            426 => {}
            514803617 | 730022849 => {
                self.style |= LOWER;
                self.styles.add(LOWER);
            }
            -1883544150 | -1668324918 => {
                if base & LOWER == 0 && self.styles.remove(LOWER) == 0 {
                    self.style &= !LOWER;
                }
            }
            9133802 | 13526026 | 566686826 | 781906058 => {
                self.style |= UPPER;
                self.styles.add(UPPER);
            }
            -1831660941 | -1616441709 | 47840323 | 52232547 => {
                if base & UPPER == 0 && self.styles.remove(UPPER) == 0 {
                    self.style &= !UPPER;
                }
            }
            551025096 | 766244328 => {
                self.style |= SMALLCAPS;
                self.styles.add(SMALLCAPS);
            }
            -1847322671 | -1632103439 => {
                if base & SMALLCAPS == 0 && self.styles.remove(SMALLCAPS) == 0 {
                    self.style &= !SMALLCAPS;
                }
            }
            -445573839 | 1897350193 => self.line_height = LARGE_NEG,
            10723418 | 15115642 => self.noparse = true,
            1117479 => self.width = -1.0,
            _ => return None,
        }
        Some(end)
    }

    fn save(&self, index: i32, count: i32) -> WrapState {
        WrapState {
            font: self.current_font,
            mat: self.current_mat,
            mat_idx: self.current_mat_idx,
            prev_break: index,
            total: count,
            visible: self.line_visible,
            first: self.first_char_of_line,
            first_vis: self.first_vis_of_line,
            last_vis: self.last_vis_of_line,
            style: self.style,
            italic: self.italic,
            fsm: self.fsm,
            cfs: self.cfs,
            xadv: self.xadv,
            max_cap: self.max_cap,
            max_asc: self.max_text_asc,
            elem_desc: self.elem_desc,
            sola: self.sola,
            mla: self.mla,
            mld: self.mld,
            page_asc: self.page_asc,
            mesh_min: self.mesh_min,
            mesh_max: self.mesh_max,
            line_number: self.line_number,
            line_offset: self.line_offset,
            baseline_offset: self.baseline_offset,
            driven: self.driven,
            glyph_adj: self.glyph_adj,
            cspace: self.cspacing,
            just: self.just,
            ml: self.margin_left,
            mr: self.margin_right,
            html: self.html,
            ucol: self.ucol,
            scol: self.scol,
            noparse: self.noparse,
            styles: self.styles,
            idx: [
                self.italic_stack.index,
                self.color_stack.index,
                self.ucolor_stack.index,
                self.scolor_stack.index,
                self.size_stack.index,
                self.weight_stack.index,
            ],
            line_info: self.lines.get(self.line_number.max(0) as usize).copied().filter(|_| self.line_number >= 0),
        }
    }

    fn restore(&mut self, s: &WrapState) -> i32 {
        self.current_font = s.font;
        self.current_mat = s.mat;
        self.current_mat_idx = s.mat_idx;
        self.cc = s.total + 1;
        self.line_visible = s.visible;
        self.first_char_of_line = s.first;
        self.first_vis_of_line = s.first_vis;
        self.last_vis_of_line = s.last_vis;
        self.style = s.style;
        self.italic = s.italic;
        self.fsm = s.fsm;
        self.cfs = s.cfs;
        self.xadv = s.xadv;
        self.max_cap = s.max_cap;
        self.max_text_asc = s.max_asc;
        self.elem_desc = s.elem_desc;
        self.sola = s.sola;
        self.mla = s.mla;
        self.mld = s.mld;
        self.page_asc = s.page_asc;
        self.mesh_min = s.mesh_min;
        self.mesh_max = s.mesh_max;
        self.line_number = s.line_number;
        self.line_offset = s.line_offset;
        self.baseline_offset = s.baseline_offset;
        self.driven = s.driven;
        self.glyph_adj = s.glyph_adj;
        self.cspacing = s.cspace;
        self.just = s.just;
        self.margin_left = s.ml;
        self.margin_right = s.mr;
        self.html = s.html;
        self.ucol = s.ucol;
        self.scol = s.scol;
        self.noparse = s.noparse;
        self.styles = s.styles;
        self.italic_stack.index = s.idx[0];
        self.color_stack.index = s.idx[1];
        self.ucolor_stack.index = s.idx[2];
        self.scolor_stack.index = s.idx[3];
        self.size_stack.index = s.idx[4];
        self.weight_stack.index = s.idx[5];
        if self.line_number >= 0 && (self.line_number as usize) < self.lines.len() {
            if let Some(li) = s.line_info {
                self.lines[self.line_number as usize] = li;
            }
        }
        s.prev_break
    }

    fn adjust_line_offset(&mut self, start: i32, end: i32, off: f32) {
        for i in start..=end {
            let c = self.c(i);
            for p in [&mut c.bl, &mut c.tl, &mut c.tr, &mut c.br] {
                p.y -= off;
            }
            c.asc -= off;
            c.baseline -= off;
            c.desc -= off;
            if c.visible {
                for p in &mut c.v {
                    p.y -= off;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_new_line(&mut self, i: i32, base_scale: f32, elem_scale: f32, em_scale: f32, glyph_adj: f32, bold_adj: f32, cs_adj: f32, width: f32, line_gap: f32, max_vis_desc: &mut f32) {
        let num = self.mla - self.sola;
        if self.line_offset > 0.0 && num.abs() > 0.01 && !self.driven && !self.new_page {
            self.adjust_line_offset(self.first_char_of_line, self.cc, num);
            self.elem_desc -= num;
            self.line_offset += num;
        }
        let num2 = self.mla - self.line_offset;
        let num3 = self.mld - self.line_offset;
        self.elem_desc = self.elem_desc.min(num3);
        *max_vis_desc = self.elem_desc;
        self.first_vis_of_line = self.first_char_of_line.max(self.first_vis_of_line);
        let num4 = (self.cc - 1).max(0);
        self.last_char_of_line = num4;
        self.last_vis_of_line = self.last_vis_of_line.max(self.first_vis_of_line);
        let (fc, fv, lv, lvc) = (self.first_char_of_line, self.first_vis_of_line, self.last_vis_of_line, self.line_visible);
        let ext_min_x = self.cr(fv).bl.x;
        let ext_max_x = self.cr(lv).tr.x;
        let num5 = (glyph_adj * elem_scale + (self.font(self.current_font).normal_spacing_offset + cs_adj + bold_adj) * em_scale - self.cspacing) * (1.0 - self.cwa);
        let xadv = self.cr(lv).xadv - num5;
        let ln = self.line_number;
        let lo = self.line_offset;
        {
            let l = self.line(ln);
            l.first = fc;
            l.first_visible = fv;
            l.last = num4;
            l.last_visible = lv;
            l.char_count = num4 - fc + 1;
            l.visible_count = lvc;
            l.ext_min = Vec2::new(ext_min_x, num3);
            l.ext_max = Vec2::new(ext_max_x, num2);
            l.length = ext_max_x;
            l.width = width;
            l.max_advance = xadv;
            l.baseline = -lo;
            l.asc = num2;
            l.desc = num3;
            l.line_height = num2 - num3 + line_gap * base_scale;
        }
        self.c(num4).xadv = xadv;
        self.first_char_of_line = self.cc;
        self.line_visible = 0;
        self.s_line = self.save(i, self.cc - 1);
        self.line_number += 1;
        self.line(self.line_number);
        if self.line_height == LARGE_NEG {
            let adj = self.cr(self.cc).adj_asc;
            self.line_offset += -self.mld + adj + (line_gap + self.line_spacing_delta) * base_scale + self.def.line_spacing * em_scale;
            self.sola = adj;
        } else {
            self.line_offset += self.line_height + self.def.line_spacing * em_scale;
        }
        self.mla = LARGE_NEG;
        self.mld = LARGE_POS;
        self.xadv = 0.0;
    }

    /// SaveGlyphVertexInfo
    fn save_glyph(&mut self, padding: f32, style_padding: f32, mut color: [u8; 4]) {
        let sp = if self.is_sdf { style_padding } else { 0.0 };
        let c = self.cr(self.cc);
        let fa = self.font(c.font);
        let g = fa.glyphs.get(&c.glyph).copied().unwrap_or_default();
        color[3] = self.font_color32[3].min(color[3]);
        let (aw, ah) = (fa.atlas_width, fa.atlas_height);
        let uv = Vec2::new((g.rect[0] - padding - sp) / aw, (g.rect[1] - padding - sp) / ah);
        let uv2 = Vec2::new(uv.x, (g.rect[1] + padding + sp + g.rect[3]) / ah);
        let uv3 = Vec2::new((g.rect[0] + padding + sp + g.rect[2]) / aw, uv2.y);
        let uv4 = Vec2::new(uv3.x, uv.y);
        let c = self.c(self.cc);
        c.v = [c.bl, c.tl, c.tr, c.br];
        c.vcolor = color;
        c.uv = [uv, uv2, uv3, uv4];
    }

    /// GenerateTextMesh: one auto-size iteration; true when the point size is set
    fn generate(&mut self, inp: &TmpInput, margin_w: f32, margin_h: f32, max_vis_desc_out: &mut f32, last_num5: &mut u32) -> bool {
        let def = self.def;
        let mf = self.font(self.main_font);
        let total = self.total;
        self.current_font = self.main_font;
        self.current_mat = self.shared_mat;
        self.current_mat_idx = 0;
        let num = self.font_size / mf.face.point_size * mf.face.scale;
        let num3 = self.font_size * 0.01;
        self.fsm = 1.0;
        self.cfs = self.font_size;
        self.size_stack.set_default(self.cfs);
        let mut num5: u32 = 0;
        self.style = def.style;
        self.weight = if self.style & BOLD != 0 { 700 } else { def.weight };
        self.weight_stack.set_default(self.weight);
        self.styles = StyleStack::default();
        self.just = def.h_align;
        let (mut num6, mut num7, mut num8): (f32, f32, f32);
        self.baseline_offset = 0.0;
        self.font_color32 = crate::ugui::color32(inp.color);
        self.html = self.font_color32;
        self.ucol = self.html;
        self.scol = self.html;
        self.color_stack.set_default(self.html);
        self.ucolor_stack.set_default(self.html);
        self.scolor_stack.set_default(self.html);
        self.italic = mf.italic_style as i32;
        self.italic_stack.set_default(self.italic);
        self.line_offset = 0.0;
        self.line_height = LARGE_NEG;
        let num9 = mf.face.line_height - (mf.face.ascent_line - mf.face.descent_line);
        self.cspacing = 0.0;
        self.xadv = 0.0;
        self.noparse = false;
        self.cc = 0;
        self.first_char_of_line = 0;
        self.last_char_of_line = 0;
        self.first_vis_of_line = 0;
        self.last_vis_of_line = 0;
        self.mla = LARGE_NEG;
        self.mld = LARGE_POS;
        self.line_number = 0;
        self.sola = 0.0;
        self.line_visible = 0;
        let mut flag4 = true;
        self.driven = false;
        self.first_overflow = -1;
        let num11 = margin_w.max(0.0);
        let num12 = margin_h.max(0.0);
        self.margin_left = 0.0;
        self.margin_right = 0.0;
        self.width = -1.0;
        let mut num13 = num11 + 0.0001 - self.margin_left - self.margin_right;
        self.mesh_min = Vec2::splat(2.147_483_6e9);
        self.mesh_max = Vec2::splat(-2.147_483_6e9);
        for l in &mut self.lines {
            l.char_count = 0;
            l.space_count = 0;
            l.control_count = 0;
            l.width = 0.0;
            l.asc = LARGE_NEG;
            l.desc = LARGE_POS;
            l.margin_left = 0.0;
            l.margin_right = 0.0;
            l.ext_min = Vec2::splat(LARGE_POS);
            l.ext_max = Vec2::splat(LARGE_NEG);
            l.max_advance = 0.0;
        }
        self.space_count = 0;
        self.max_cap = 0.0;
        self.max_text_asc = 0.0;
        self.elem_desc = 0.0;
        self.page_asc = 0.0;
        let mut max_vis_desc = 0.0f32;
        self.new_page = false;
        let mut flag5 = true;
        let mut flag6 = false;
        let mut num14: i32 = 0;
        let mut subst: (i32, u32) = (-1, 0);
        let mut flag7 = false;
        self.s_wrap = self.save(-1, -1);
        self.s_line = self.save(-1, -1);
        self.s_valid = self.save(-1, -1);
        self.s_soft = self.save(-1, -1);
        let len = self.chars.len() as i32;
        let mut i: i32 = -1;
        loop {
            i += 1;
            if !(i < len && self.chars[i as usize] != 0) {
                break;
            }
            num5 = self.chars[i as usize];
            if def.rich && num5 == 60 {
                if let Some(end) = self.validate_tag(i as usize + 1) {
                    i = end as i32;
                    continue;
                }
            } else {
                let c = self.cr(self.cc);
                self.current_mat_idx = c.mat_idx;
                self.current_font = c.font;
            }
            let alt = self.cr(self.cc).alt;
            let mut flag8 = false;
            if subst.0 == self.cc {
                num5 = subst.1;
                flag8 = true;
                if num5 == 3 {
                    let f = self.current_font;
                    let e = self.font(f).characters.get(&3).copied().unwrap_or_default();
                    let c = self.c(self.cc);
                    c.glyph = e.glyph;
                    c.glyph_index = if e.glyph == SYNTH_GLYPH { 0 } else { e.glyph };
                    c.elem_scale = e.scale;
                    c.has_elem = true;
                    self.truncated = true;
                }
            }
            let mut num16 = 1.0f32;
            if self.style & UPPER != 0 {
                if is_lower(num5) {
                    num5 = to_upper(num5);
                }
            } else if self.style & LOWER != 0 {
                if is_upper(num5) {
                    num5 = to_lower(num5);
                }
            } else if self.style & SMALLCAPS != 0 && is_lower(num5) {
                num16 = 0.8;
                num5 = to_upper(num5);
            }
            let cur = self.cr(self.cc);
            if !cur.has_elem {
                continue;
            }
            self.current_font = cur.font;
            self.current_mat = cur.mat;
            self.current_mat_idx = cur.mat_idx;
            let cf = self.font(cur.font);
            let pt = cf.face.point_size;
            let num24 = if !flag8 || self.chars[i as usize] != 10 || self.cc == self.first_char_of_line {
                self.cfs * num16 / pt * cf.face.scale
            } else {
                self.cr(self.cc - 1).point_size * num16 / pt * cf.face.scale
            };
            let (num18, num19) = (cf.face.ascent_line, cf.face.descent_line);
            let glyph = cf.glyphs.get(&cur.glyph).copied().unwrap_or_default();
            let mut num2 = num24 * self.fsm * cur.elem_scale * glyph.scale;
            let num17 = cf.face.baseline * num24 * self.fsm * cf.face.scale;
            self.c(self.cc).scale = num2;
            num6 = if self.current_mat_idx == 0 { self.padding } else { self.sub_padding.get(self.current_mat_idx).copied().unwrap_or(self.padding) };
            let num25 = num2;
            if num5 == 173 || num5 == 3 {
                num2 = 0.0;
            }
            let (html, ucol, scol, style, cfs) = (self.html, self.ucol, self.scol, self.style, self.cfs);
            {
                let c = self.c(self.cc);
                c.character = num5 & 0xFFFF;
                c.point_size = cfs;
                c.color = html;
                c.style = style;
                let _ = (ucol, scol);
            }
            let flag9 = num5 <= 65535 && is_whitespace(num5);
            let mut rec = [0.0f32; 4];
            let num26 = def.char_spacing;
            self.glyph_adj = 0.0;
            if def.kerning {
                let gi = cur.glyph_index;
                if self.cc < total - 1 {
                    let key = (self.cr(self.cc + 1).glyph_index << 16) | gi;
                    if let Some(v) = cf.kerning.get(&key) {
                        rec = v.0;
                    }
                }
                if self.cc >= 1 {
                    let key2 = (gi << 16) | self.cr(self.cc - 1).glyph_index;
                    if let Some(v) = cf.kerning.get(&key2) {
                        for k in 0..4 {
                            rec[k] += v.1[k];
                        }
                    }
                }
                self.glyph_adj = rec[2];
            }
            let mp = self.rt.list[self.current_mat as usize].props.clone();
            if !alt && self.style & BOLD != 0 {
                if has(&mp, "_GradientScale") {
                    let gs = getf(&mp, "_GradientScale");
                    num7 = cf.bold_style / 4.0 * gs * getf(&mp, "_ScaleRatioA");
                    if num7 + num6 > gs {
                        num6 = gs - num7;
                    }
                } else {
                    num7 = 0.0;
                }
                num8 = cf.bold_spacing;
            } else {
                if has(&mp, "_GradientScale") && has(&mp, "_ScaleRatioA") {
                    let gs = getf(&mp, "_GradientScale");
                    num7 = cf.normal_style / 4.0 * gs * getf(&mp, "_ScaleRatioA");
                    if num7 + num6 > gs {
                        num6 = gs - num7;
                    }
                } else {
                    num7 = 0.0;
                }
                num8 = 0.0;
            }
            let cwa1 = 1.0 - self.cwa;
            let mut v2 = Vec2::new(
                self.xadv + (glyph.bearing_x - num6 - num7 + rec[0]) * num2 * cwa1,
                num17 + (glyph.bearing_y + num6 + rec[1]) * num2 - self.line_offset + self.baseline_offset,
            );
            let mut v3 = Vec2::new(v2.x, v2.y - (glyph.height + num6 * 2.0) * num2);
            let mut v4 = Vec2::new(v3.x + (glyph.width + num6 * 2.0 + num7 * 2.0) * num2 * cwa1, v2.y);
            let mut v5 = Vec2::new(v4.x, v3.y);
            if !alt && self.style & ITALIC != 0 {
                let num28 = self.italic as f32 * 0.01;
                let top = num28 * ((glyph.bearing_y + num6 + num7) * num2);
                let bottom = num28 * ((glyph.bearing_y - glyph.height - num6 - num7) * num2);
                let mid = (top - bottom) / 2.0;
                v2.x += top - mid;
                v3.x += bottom - mid;
                v4.x += top - mid;
                v5.x += bottom - mid;
            }
            let (xadv, lo, bo) = (self.xadv, self.line_offset, self.baseline_offset);
            {
                let c = self.c(self.cc);
                c.bl = v3;
                c.tl = v2;
                c.tr = v4;
                c.br = v5;
                c.origin = xadv;
                c.baseline = num17 - lo + bo;
                c.aspect = (v4.x - v3.x) / (v2.y - v3.y);
            }
            let num29 = num18 * num2 / num16 + bo;
            let num30 = num19 * num2 / num16 + bo;
            let mut num31 = num29;
            let mut num32 = num30;
            let flag10 = self.cc == self.first_char_of_line;
            if flag10 || !flag9 {
                if bo != 0.0 {
                    num31 = ((num29 - bo) / self.fsm).max(num31);
                    num32 = ((num30 - bo) / self.fsm).min(num32);
                }
                self.mla = num31.max(self.mla);
                self.mld = num32.min(self.mld);
            }
            let (mla, mld) = (self.mla, self.mld);
            if flag10 || !flag9 {
                let c = self.c(self.cc);
                c.adj_asc = num31;
                c.adj_desc = num32;
                c.asc = num29 - lo;
                c.desc = num30 - lo;
                self.elem_asc = num29 - lo;
                self.elem_desc = num30 - lo;
            } else {
                let c = self.c(self.cc);
                c.adj_asc = mla;
                c.adj_desc = mld;
                c.asc = mla - lo;
                c.desc = mld - lo;
                self.elem_asc = mla - lo;
                self.elem_desc = mld - lo;
            }
            if (self.line_number == 0 || self.new_page) && (flag10 || !flag9) {
                self.max_text_asc = self.mla;
                self.max_cap = self.max_cap.max(cf.face.cap_line * num2 / num16);
            }
            if self.line_offset == 0.0 && (flag10 || !flag9) {
                self.page_asc = if self.page_asc > num29 { self.page_asc } else { num29 };
            }
            self.c(self.cc).visible = false;
            let flag11 = self.just & 16 == 16 || self.just & 8 == 8;
            if num5 == 9 || (!flag9 && num5 != 8203 && num5 != 173 && num5 != 3) || (num5 == 173 && !flag7) {
                self.c(self.cc).visible = true;
                let (mut margin_left, mut margin_right) = (self.margin_left, self.margin_right);
                if flag8 {
                    let l = *self.line(self.line_number);
                    margin_left = l.margin_left;
                    margin_right = l.margin_right;
                }
                num13 = if self.width != -1.0 { (num11 + 0.0001 - margin_left - margin_right).min(self.width) } else { num11 + 0.0001 - margin_left - margin_right };
                let num33 = self.xadv.abs() + glyph.advance * (1.0 - self.cwa) * if num5 == 173 { num25 } else { num2 };
                let num34 = self.max_text_asc - (self.mld - self.line_offset) + if self.line_offset > 0.0 && !self.driven { self.mla - self.sola } else { 0.0 };
                let character_count = self.cc;
                if num34 > num12 + 0.0001 {
                    if self.first_overflow == -1 {
                        self.first_overflow = self.cc;
                    }
                    if def.autosize {
                        if self.line_spacing_delta > def.line_spacing_max && self.line_offset > 0.0 && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                            let num35 = (num12 - num34) / self.line_number as f32;
                            self.line_spacing_delta = (self.line_spacing_delta + num35 / num).max(def.line_spacing_max);
                            return false;
                        }
                        if self.font_size > def.size_min && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                            self.shrink();
                            return false;
                        }
                    }
                    if def.overflow == TRUNCATE {
                        let s = self.s_valid;
                        i = self.restore(&s);
                        subst = (character_count, 3);
                        continue;
                    }
                }
                if num33 > num13 * if flag11 { 1.05 } else { 1.0 } {
                    if def.wrap && self.cc != self.first_char_of_line {
                        let s = self.s_wrap;
                        i = self.restore(&s);
                        let num37 = if self.line_height == LARGE_NEG {
                            let adj = self.cr(self.cc).adj_asc;
                            (if self.line_offset > 0.0 && !self.driven { self.mla - self.sola } else { 0.0 }) - self.mld + adj + (num9 + self.line_spacing_delta) * num + def.line_spacing * num3
                        } else {
                            self.driven = true;
                            self.line_height + def.line_spacing * num3
                        };
                        let num38 = self.max_text_asc + num37 + self.line_offset - self.cr(self.cc).adj_desc;
                        if self.cr(self.cc - 1).character == 173 && !flag7 && (def.overflow == OVERFLOW || num38 < num12 + 0.0001) {
                            subst = (self.cc - 1, 45);
                            i -= 1;
                            self.cc -= 1;
                            continue;
                        }
                        flag7 = false;
                        if self.cr(self.cc).character == 173 {
                            flag7 = true;
                            continue;
                        }
                        if def.autosize && flag5 {
                            if self.cwa < def.cwa_max / 100.0 && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                                self.widen(num33, num13, flag11);
                                return false;
                            }
                            if self.font_size > def.size_min && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                                self.shrink();
                                return false;
                            }
                        }
                        let pwb = self.s_soft.prev_break;
                        if flag5 && pwb != -1 && pwb != num14 {
                            let s = self.s_soft;
                            i = self.restore(&s);
                            num14 = pwb;
                            if self.cr(self.cc - 1).character == 173 {
                                subst = (self.cc - 1, 45);
                                i -= 1;
                                self.cc -= 1;
                                continue;
                            }
                        }
                        if num38 <= num12 + 0.0001 {
                            self.insert_new_line(i, num, num2, num3, self.glyph_adj, num8, num26, num13, num9, &mut max_vis_desc);
                            flag4 = true;
                            flag5 = true;
                            continue;
                        }
                        if self.first_overflow == -1 {
                            self.first_overflow = self.cc;
                        }
                        if def.autosize {
                            if self.line_spacing_delta > def.line_spacing_max && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                                let num42 = (num12 - num38) / (self.line_number + 1) as f32;
                                self.line_spacing_delta = (self.line_spacing_delta + num42 / num).max(def.line_spacing_max);
                                return false;
                            }
                            if self.cwa < def.cwa_max / 100.0 && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                                self.widen(num33, num13, flag11);
                                return false;
                            }
                            if self.font_size > def.size_min && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                                self.shrink();
                                return false;
                            }
                        }
                        if def.overflow == TRUNCATE {
                            let s = self.s_valid;
                            i = self.restore(&s);
                            subst = (character_count, 3);
                            continue;
                        }
                        self.insert_new_line(i, num, num2, num3, self.glyph_adj, num8, num26, num13, num9, &mut max_vis_desc);
                        flag4 = true;
                        flag5 = true;
                        continue;
                    } else {
                        if def.autosize && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
                            if self.cwa < def.cwa_max / 100.0 {
                                self.widen(num33, num13, flag11);
                                return false;
                            }
                            if self.font_size > def.size_min {
                                self.shrink();
                                return false;
                            }
                        }
                        if def.overflow == TRUNCATE {
                            let s = self.s_wrap;
                            i = self.restore(&s);
                            subst = (character_count, 3);
                            continue;
                        }
                    }
                }
                match num5 {
                    9 => {
                        self.c(self.cc).visible = false;
                        self.last_vis_of_line = self.cc;
                        self.line(self.line_number).space_count += 1;
                        self.space_count += 1;
                    }
                    173 => self.c(self.cc).visible = false,
                    _ => {
                        let vc = if def.override_html { self.font_color32 } else { self.html };
                        self.save_glyph(num6, num7, vc);
                        if flag4 {
                            flag4 = false;
                            self.first_vis_of_line = self.cc;
                        }
                        self.line_visible += 1;
                        self.last_vis_of_line = self.cc;
                        let l = self.line(self.line_number);
                        l.margin_left = margin_left;
                        l.margin_right = margin_right;
                    }
                }
            } else {
                if (num5 == 10 || num5 == 11 || num5 == 160 || num5 == 8199 || num5 == 8232 || num5 == 8233 || is_separator(num5)) && num5 != 173 && num5 != 8203 && num5 != 8288 {
                    self.line(self.line_number).space_count += 1;
                    self.space_count += 1;
                }
                if num5 == 160 {
                    self.line(self.line_number).control_count += 1;
                }
            }
            let (lnum, just) = (self.line_number, self.just);
            self.c(self.cc).line = lnum;
            if (num5 != 10 && num5 != 11 && num5 != 13 && !flag8) || self.line(lnum).char_count == 1 {
                self.line(lnum).alignment = just;
            }
            if num5 == 9 {
                let num54 = cf.face.tab_width * (cf.tab_size as i32 as f32) * num2;
                let num55 = (self.xadv / num54).ceil() * num54;
                self.xadv = if num55 > self.xadv { num55 } else { self.xadv + num54 };
            } else {
                self.xadv += ((glyph.advance + rec[2]) * num2 + (cf.normal_spacing_offset + num26 + num8) * num3 + self.cspacing) * (1.0 - self.cwa);
                if flag9 || num5 == 8203 {
                    self.xadv += def.word_spacing * num3;
                }
            }
            let xa = self.xadv;
            self.c(self.cc).xadv = xa;
            if num5 == 13 {
                self.xadv = 0.0;
            }
            if num5 == 10 || num5 == 11 || num5 == 3 || num5 == 8232 || num5 == 8233 || (num5 == 45 && flag8) || self.cc == total - 1 {
                let num57 = self.mla - self.sola;
                if self.line_offset > 0.0 && num57.abs() > 0.01 && !self.driven && !self.new_page {
                    self.adjust_line_offset(self.first_char_of_line, self.cc, num57);
                    self.elem_desc -= num57;
                    self.line_offset += num57;
                }
                self.new_page = false;
                let num58 = self.mla - self.line_offset;
                let num59 = self.mld - self.line_offset;
                self.elem_desc = self.elem_desc.min(num59);
                max_vis_desc = self.elem_desc;
                self.first_vis_of_line = self.first_char_of_line.max(self.first_vis_of_line);
                self.last_char_of_line = self.cc;
                self.last_vis_of_line = self.last_vis_of_line.max(self.first_vis_of_line);
                let (fc, fv, lc, lv, lvc, cc) = (self.first_char_of_line, self.first_vis_of_line, self.last_char_of_line, self.last_vis_of_line, self.line_visible, self.cc);
                let ext_min_x = self.cr(fv).bl.x;
                let ext_max_x = self.cr(lv).tr.x;
                let num60 = ((cf.normal_spacing_offset + num26 + num8) * num3 - self.cspacing) * (1.0 - self.cwa);
                let max_adv = if self.cr(lv).visible { self.cr(lv).xadv - num60 } else { self.cr(lc).xadv - num60 };
                let lo = self.line_offset;
                {
                    let l = self.line(lnum);
                    l.first = fc;
                    l.first_visible = fv;
                    l.last = cc;
                    l.last_visible = lv;
                    l.char_count = cc - fc + 1;
                    l.visible_count = lvc;
                    l.ext_min = Vec2::new(ext_min_x, num59);
                    l.ext_max = Vec2::new(ext_max_x, num58);
                    l.length = ext_max_x - num6 * num2;
                    l.width = num13;
                    if l.char_count == 1 {
                        l.alignment = just;
                    }
                    l.max_advance = max_adv;
                    l.baseline = -lo;
                    l.asc = num58;
                    l.desc = num59;
                    l.line_height = num58 - num59 + num9 * num;
                }
                match num5 {
                    10 | 11 | 45 | 8232 | 8233 => {
                        self.s_line = self.save(i, self.cc);
                        self.line_number += 1;
                        flag4 = true;
                        flag6 = false;
                        flag5 = true;
                        self.first_char_of_line = self.cc + 1;
                        self.line_visible = 0;
                        self.line(self.line_number);
                        let adj2 = self.cr(self.cc).adj_asc;
                        let para = if num5 == 10 || num5 == 8233 { def.paragraph_spacing } else { 0.0 };
                        if self.line_height == LARGE_NEG {
                            self.line_offset += -self.mld + adj2 + (num9 + self.line_spacing_delta) * num + (def.line_spacing + para) * num3;
                            self.driven = false;
                        } else {
                            self.line_offset += self.line_height + (def.line_spacing + para) * num3;
                            self.driven = true;
                        }
                        self.mla = LARGE_NEG;
                        self.mld = LARGE_POS;
                        self.sola = adj2;
                        self.xadv = 0.0;
                        self.s_wrap = self.save(i, self.cc);
                        self.s_valid = self.save(i, self.cc);
                        self.cc += 1;
                        continue;
                    }
                    3 => i = len,
                    _ => {}
                }
            }
            let c = self.cr(self.cc);
            if c.visible {
                self.mesh_min = self.mesh_min.min(c.bl);
                self.mesh_max = self.mesh_max.max(c.tr);
            }
            if def.wrap || def.overflow == TRUNCATE {
                let cjk = ((num5 > 4352 && num5 < 4607) || (num5 > 43360 && num5 < 43391) || (num5 > 44032 && num5 < 55295))
                    || (num5 > 11904 && num5 < 40959)
                    || (num5 > 63744 && num5 < 64255)
                    || (num5 > 65072 && num5 < 65103)
                    || (num5 > 65280 && num5 < 65519);
                if (flag9 || num5 == 8203 || num5 == 45 || num5 == 173) && num5 != 160 && num5 != 8199 && num5 != 8209 && num5 != 8239 && num5 != 8288 {
                    self.s_wrap = self.save(i, self.cc);
                    flag5 = false;
                    self.s_soft.prev_break = -1;
                } else if cjk {
                    // no TMP_Settings line breaking rules: no leading / following characters
                    self.s_wrap = self.save(i, self.cc);
                    flag5 = false;
                } else if flag5 {
                    if flag9 || (num5 == 173 && !flag7) {
                        self.s_soft = self.save(i, self.cc);
                    }
                    self.s_wrap = self.save(i, self.cc);
                }
            }
            let _ = flag6;
            self.s_valid = self.save(i, self.cc);
            self.cc += 1;
        }
        let num4 = self.max_font_size - self.min_font_size;
        if def.autosize && num4 > 0.051 && self.font_size < def.size_max && self.autosize_count < AUTOSIZE_MAX_ITERATIONS {
            if self.cwa < def.cwa_max / 100.0 {
                self.cwa = 0.0;
            }
            self.min_font_size = self.font_size;
            let num63 = ((self.max_font_size - self.font_size) / 2.0).max(0.05);
            self.font_size += num63;
            self.font_size = (((self.font_size * 20.0 + 0.5) as i32) as f32 / 20.0).min(def.size_max);
            return false;
        }
        *max_vis_desc_out = max_vis_desc;
        *last_num5 = num5;
        true
    }

    fn shrink(&mut self) {
        self.max_font_size = self.font_size;
        let d = ((self.font_size - self.min_font_size) / 2.0).max(0.05);
        self.font_size -= d;
        self.font_size = (((self.font_size * 20.0 + 0.5) as i32) as f32 / 20.0).max(self.def.size_min);
    }

    fn widen(&mut self, num33: f32, num13: f32, flag11: bool) {
        let mut n = num33;
        if self.cwa > 0.0 {
            n /= 1.0 - self.cwa;
        }
        let over = num33 - (num13 - 0.0001) * if flag11 { 1.05 } else { 1.0 };
        self.cwa += over / n;
        self.cwa = self.cwa.min(self.def.cwa_max / 100.0);
    }

    /// The post-loop half of GenerateTextMesh: alignment, uv2, vertex buffers.
    fn build(&mut self, inp: &TmpInput, max_vis_desc: f32, last_num5: u32) -> TmpMesh {
        let def = self.def;
        let mut out = TmpMesh { subs: Vec::new(), truncated: self.truncated, font_size: self.font_size };
        if self.cc == 0 || (self.cc == 1 && last_num5 == 3) {
            return out;
        }
        let [rx, ry, _, rh] = inp.rect;
        let c0 = Vec2::new(rx, ry);
        let c1 = Vec2::new(rx, ry + rh);
        let m = def.margin;
        let mid = (c0 + c1) / 2.0;
        let v10 = match def.v_align {
            256 => c1 + Vec2::new(m[0], -self.max_text_asc - m[1]),
            512 => mid + Vec2::new(m[0], -(self.max_text_asc + m[1] + max_vis_desc - m[3]) / 2.0),
            1024 => c0 + Vec2::new(m[0], -max_vis_desc + m[3]),
            2048 => mid + Vec2::new(m[0], 0.0),
            4096 => mid + Vec2::new(m[0], -(self.mesh_max.y + m[1] + self.mesh_min.y - m[3]) / 2.0),
            8192 => mid + Vec2::new(m[0], -(self.max_cap - m[1] - m[3]) / 2.0),
            _ => Vec2::ZERO,
        };
        let mut v11 = Vec2::ZERO;
        let mut num65: i32 = 0;
        let mut flag13 = false;
        let mut subs: Vec<TmpSub> = self
            .refs
            .iter()
            .map(|r| TmpSub { rt: r.rt, verts: Vec::new(), idx: Vec::new(), min: Vec2::ZERO, max: Vec2::ZERO })
            .collect();
        let cc = self.cc;
        for j in 0..cc {
            let c = self.cr(j);
            let character = c.character;
            let ln = c.line;
            let li = *self.line(ln);
            match li.alignment {
                1 => v11 = Vec2::new(li.margin_left, 0.0),
                2 => v11 = Vec2::new(li.margin_left + li.width / 2.0 - li.max_advance / 2.0, 0.0),
                32 => v11 = Vec2::new(li.margin_left + li.width / 2.0 - (li.ext_min.x + li.ext_max.x) / 2.0, 0.0),
                4 => v11 = Vec2::new(li.margin_left + li.width - li.max_advance, 0.0),
                8 | 16 => {
                    if !(character == 10 || character == 173 || character == 8203 || character == 8288 || character == 3) {
                        let c2 = self.cr(li.last).character;
                        let flag16 = li.alignment & 16 == 16;
                        if (!is_control(c2) && ln < self.line_number) || flag16 || li.max_advance > li.width {
                            if ln != num65 || j == 0 {
                                v11 = Vec2::new(li.margin_left, 0.0);
                                flag13 = is_separator(character);
                            } else {
                                let num77 = li.width - li.max_advance;
                                let mut num78 = li.visible_count - 1 + li.control_count;
                                let mut num79 = (if self.cr(li.last).visible { li.space_count } else { li.space_count - 1 }) - li.control_count;
                                if flag13 {
                                    num79 -= 1;
                                    num78 += 1;
                                }
                                let num80 = if num79 > 0 { def.wrap_ratios } else { 1.0 };
                                if num79 < 1 {
                                    num79 = 1;
                                }
                                if character != 160 && (character == 9 || is_separator(character)) {
                                    v11.x += num77 * (1.0 - num80) / num79 as f32;
                                } else {
                                    v11.x += num77 * num80 / num78 as f32;
                                }
                            }
                        } else {
                            v11 = Vec2::new(li.margin_left, 0.0);
                        }
                    }
                }
                _ => {}
            }
            let zero3 = v10 + v11;
            if c.visible {
                let mut num68 = c.scale * (1.0 - self.cwa);
                if !c.alt && c.style & BOLD != 0 {
                    num68 *= -1.0;
                }
                num68 *= inp.uv2_scale;
                // Character mapping: x 0 0 1 1, y 0 1 1 0
                let pack = |x: f32, y: f32| (((x * 511.0) as i32) as f64 * 4096.0 + ((y * 511.0) as i32) as f64) as f32;
                let uv2 = [pack(0.0, 0.0), pack(0.0, 1.0), pack(1.0, 1.0), pack(1.0, 0.0)];
                let s = &mut subs[c.mat_idx];
                let b = s.verts.len() as u32;
                for k in 0..4 {
                    let p = c.v[k] + zero3;
                    s.verts.push(UiVertex {
                        pos: Vec3::new(p.x, p.y, 0.0),
                        color: c.vcolor,
                        uv0: Vec4::new(c.uv[k].x, c.uv[k].y, 0.0, 0.0),
                        uv1: Vec4::new(uv2[k], num68, 0.0, 0.0),
                    });
                }
                s.idx.extend([b, b + 1, b + 2, b + 2, b + 3, b]);
            }
            num65 = ln;
        }
        for s in &mut subs {
            // RecalculateBounds over the whole buffer: the unused (zeroed) quads put the origin in
            let (mut mn, mut mx) = (Vec2::ZERO, Vec2::ZERO);
            for v in &s.verts {
                mn = mn.min(v.pos.truncate());
                mx = mx.max(v.pos.truncate());
            }
            s.min = mn;
            s.max = mx;
        }
        out.subs = subs;
        out
    }
}

/// OnPreRenderCanvas: ParseInputText, then GenerateTextMesh until the auto-size point size is set.
pub fn generate(inp: &TmpInput, rt: &mut RtMats) -> TmpMesh {
    let def = inp.def;
    let (Some(font), Some(mat)) = (def.font, def.material) else {
        return TmpMesh::default();
    };
    if inp.assets.fonts.get(font as usize).is_none() || inp.assets.materials.get(mat as usize).is_none() {
        return TmpMesh::default();
    }
    let text: Vec<u32> = inp.text.unwrap_or(&def.text).encode_utf16().map(u32::from).collect();
    let chars = populate(&text, inp.text.is_none(), def.parse_ctrl, def.rich);
    if chars[0] == 0 {
        return TmpMesh::default();
    }
    let shared = rt.asset(inp.assets, mat);
    let shared_props = rt.list[shared as usize].props.clone();
    let mut g = Gen {
        def,
        assets: inp.assets,
        rt,
        chars,
        main_font: font,
        shared_mat: shared,
        // UpdateMeshPadding (Awake: m_isUsingBold false)
        padding: get_padding(&shared_props, def.extra_padding),
        sub_padding: Vec::new(),
        is_sdf: has(&shared_props, "_WeightNormal"),
        font_size: def.font_size,
        max_font_size: 0.0,
        min_font_size: 0.0,
        line_spacing_delta: 0.0,
        cwa: 0.0,
        autosize_count: 0,
        truncated: false,
        total: 0,
        using_bold: false,
        font_color32: [255; 4],
        current_font: font,
        current_mat: shared,
        current_mat_idx: 0,
        style: def.style,
        weight: def.weight,
        styles: StyleStack::default(),
        weight_stack: Stack::new(8),
        size_stack: Stack::new(16),
        color_stack: Stack::new(16),
        ucolor_stack: Stack::new(16),
        scolor_stack: Stack::new(16),
        italic_stack: Stack::new(16),
        html: [255; 4],
        ucol: [255; 4],
        scol: [255; 4],
        italic: 0,
        cfs: def.font_size,
        fsm: 1.0,
        line_offset: 0.0,
        line_height: LARGE_NEG,
        cspacing: 0.0,
        xadv: 0.0,
        cc: 0,
        first_char_of_line: 0,
        last_char_of_line: 0,
        first_vis_of_line: 0,
        last_vis_of_line: 0,
        mla: LARGE_NEG,
        mld: LARGE_POS,
        line_number: 0,
        sola: 0.0,
        line_visible: 0,
        driven: false,
        first_overflow: -1,
        max_cap: 0.0,
        max_text_asc: 0.0,
        elem_asc: 0.0,
        elem_desc: 0.0,
        page_asc: 0.0,
        mesh_min: Vec2::ZERO,
        mesh_max: Vec2::ZERO,
        new_page: false,
        noparse: false,
        just: def.h_align,
        margin_left: 0.0,
        margin_right: 0.0,
        width: -1.0,
        baseline_offset: 0.0,
        glyph_adj: 0.0,
        html_tag: [0; 128],
        ci: Vec::new(),
        lines: vec![LineInfo::default(); 2],
        refs: Vec::new(),
        ref_lookup: HashMap::new(),
        s_wrap: WrapState::default(),
        s_line: WrapState::default(),
        s_valid: WrapState::default(),
        s_soft: WrapState::default(),
        space_count: 0,
    };
    g.set_array_sizes();
    g.ci.resize(g.total.max(0) as usize + 2, CharInfo::default());
    // TMP_SubMeshUI.padding: GetPadding(sub material, extraPadding, isUsingBold)
    g.sub_padding = g.refs.iter().map(|r| get_padding(&g.rt.list[r.rt as usize].props, def.extra_padding)).collect();
    if def.autosize {
        g.font_size = def.size_base.clamp(def.size_min, def.size_max);
    }
    g.max_font_size = def.size_max;
    g.min_font_size = def.size_min;
    let margin_w = inp.rect[2] - def.margin[0] - def.margin[2];
    let margin_h = inp.rect[3] - def.margin[1] - def.margin[3];
    let mut max_vis_desc = 0.0;
    let mut last = 0;
    loop {
        let done = g.generate(inp, margin_w, margin_h, &mut max_vis_desc, &mut last);
        g.autosize_count += 1;
        if done {
            break;
        }
    }
    g.build(inp, max_vis_desc, last)
}
