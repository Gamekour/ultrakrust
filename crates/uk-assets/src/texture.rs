//! `Texture2D` and `Material` decoding: top mip level → RGBA8, plus the main
//! texture/color of a material.

use crate::db::AssetDb;
use crate::serialized::{SerializedFile, Value};
use crate::{Error, Result};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct TextureData {
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// RGBA8, rows in Unity order (bottom row first) — matches Unity UVs sampled with v=0 at row 0.
    pub rgba: Vec<u8>,
    /// Unity FilterMode: 0 point, 1 bilinear, 2 trilinear.
    pub filter: i64,
    /// TextureWrapMode: 0 repeat, 1 clamp.
    pub wrap: i64,
    /// Images stacked in `rgba`: 1, or 6 cube faces (+X -X +Y -Y +Z -Z) for a Cubemap.
    pub layers: u32,
}

#[derive(Clone, Debug)]
pub struct MaterialData {
    pub name: String,
    pub color: [f32; 4],
    pub main_tex: Option<(Arc<SerializedFile>, i64)>,
    pub tex_scale: [f32; 2],
    pub tex_offset: [f32; 2],
    /// Unity render queue > 2500 => transparent.
    pub transparent: bool,
}

pub fn decode_texture(db: &mut AssetDb, v: &Value) -> Result<TextureData> {
    let w = v.get("m_Width").i64() as usize;
    let h = v.get("m_Height").i64() as usize;
    let fmt = v.get("m_TextureFormat").i64();
    if w == 0 || h == 0 {
        return Err(Error(format!("empty texture {}", v.get("m_Name").str())));
    }
    let sd = v.get("m_StreamData");
    let inline = v.get("image data").bytes();
    let owned;
    let data: &[u8] = if inline.is_empty() && sd.get("size").i64() > 0 {
        let path = sd.get("path").str();
        let res = db
            .resource(path.rsplit('/').next().unwrap_or(path))
            .ok_or_else(|| Error(format!("missing stream {path}")))?;
        let off = sd.get("offset").i64() as usize;
        let size = sd.get("size").i64() as usize;
        owned = res[off..off + size].to_vec();
        &owned
    } else {
        inline
    };
    // a Cubemap stores its faces one after another, each with its full mip chain
    let layers = v.get("m_ImageCount").i64().max(1) as usize;
    let stride = data.len() / layers;
    let n = w * h;
    let mut rgba = vec![255u8; n * 4 * layers];
    for (l, out) in rgba.chunks_exact_mut(n * 4).enumerate() {
        decode_image(v, fmt, &data[l * stride..(l + 1) * stride], w, h, out)?;
    }
    let ts = v.get("m_TextureSettings");
    Ok(TextureData {
        name: v.get("m_Name").str().to_string(),
        width: w as u32,
        height: h as u32,
        rgba,
        filter: ts.get("m_FilterMode").i64(),
        wrap: ts.get("m_WrapU").i64(),
        layers: layers as u32,
    })
}

fn decode_image(v: &Value, fmt: i64, data: &[u8], w: usize, h: usize, rgba: &mut [u8]) -> Result<()> {
    let n = w * h;
    match fmt {
        // Alpha8
        1 => {
            for i in 0..n.min(data.len()) {
                rgba[i * 4 + 3] = data[i];
            }
        }
        // RGB24
        3 => {
            for i in 0..n.min(data.len() / 3) {
                rgba[i * 4..i * 4 + 3].copy_from_slice(&data[i * 3..i * 3 + 3]);
            }
        }
        // RGBA32
        4 => {
            let len = (n * 4).min(data.len());
            rgba[..len].copy_from_slice(&data[..len]);
        }
        // ARGB32
        5 => {
            for i in 0..n.min(data.len() / 4) {
                let p = &data[i * 4..i * 4 + 4];
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[p[1], p[2], p[3], p[0]]);
            }
        }
        // BGRA32
        14 => {
            for i in 0..n.min(data.len() / 4) {
                let p = &data[i * 4..i * 4 + 4];
                rgba[i * 4..i * 4 + 4].copy_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        10 => decode_bc(data, w, h, rgba, false),
        12 => decode_bc(data, w, h, rgba, true),
        // BC7
        25 => {
            let mut px = vec![0u32; n];
            texture2ddecoder::decode_bc7(data, w, h, &mut px).map_err(|e| Error(format!("{}: bc7 {e}", v.get("m_Name").str())))?;
            for (o, p) in rgba.chunks_exact_mut(4).zip(px) {
                let [b, g, r, a] = p.to_le_bytes();
                o.copy_from_slice(&[r, g, b, a]);
            }
        }
        f => return Err(Error(format!("{}: texture format {f} not supported", v.get("m_Name").str()))),
    }
    Ok(())
}

fn rgb565(c: u16) -> [u8; 3] {
    let r = ((c >> 11) & 31) as u32;
    let g = ((c >> 5) & 63) as u32;
    let b = (c & 31) as u32;
    [(r * 255 / 31) as u8, (g * 255 / 63) as u8, (b * 255 / 31) as u8]
}

/// DXT1 (BC1) or DXT5 (BC3).
fn decode_bc(data: &[u8], w: usize, h: usize, out: &mut [u8], dxt5: bool) {
    let block = if dxt5 { 16 } else { 8 };
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * block;
            let Some(b) = data.get(off..off + block) else { return };
            let (alpha_blk, color_blk) = if dxt5 { (Some(&b[..8]), &b[8..]) } else { (None, b) };
            let c0 = u16::from_le_bytes([color_blk[0], color_blk[1]]);
            let c1 = u16::from_le_bytes([color_blk[2], color_blk[3]]);
            let (p0, p1) = (rgb565(c0), rgb565(c1));
            let mut pal = [[0u8; 4]; 4];
            pal[0] = [p0[0], p0[1], p0[2], 255];
            pal[1] = [p1[0], p1[1], p1[2], 255];
            let mix = |a: u8, b: u8, wa: u32, wb: u32| ((a as u32 * wa + b as u32 * wb) / (wa + wb)) as u8;
            if c0 > c1 || dxt5 {
                pal[2] = [mix(p0[0], p1[0], 2, 1), mix(p0[1], p1[1], 2, 1), mix(p0[2], p1[2], 2, 1), 255];
                pal[3] = [mix(p0[0], p1[0], 1, 2), mix(p0[1], p1[1], 1, 2), mix(p0[2], p1[2], 1, 2), 255];
            } else {
                pal[2] = [mix(p0[0], p1[0], 1, 1), mix(p0[1], p1[1], 1, 1), mix(p0[2], p1[2], 1, 1), 255];
                pal[3] = [0, 0, 0, 0];
            }
            let bits = u32::from_le_bytes([color_blk[4], color_blk[5], color_blk[6], color_blk[7]]);
            let mut alphas = [255u8; 16];
            if let Some(a) = alpha_blk {
                let (a0, a1) = (a[0] as u32, a[1] as u32);
                let mut ap = [0u8; 8];
                ap[0] = a0 as u8;
                ap[1] = a1 as u8;
                for i in 2..8u32 {
                    ap[i as usize] = if a0 > a1 {
                        (((8 - i) * a0 + (i - 1) * a1) / 7) as u8
                    } else if i < 6 {
                        (((6 - i) * a0 + (i - 1) * a1) / 5) as u8
                    } else if i == 6 {
                        0
                    } else {
                        255
                    };
                }
                let mut abits = 0u64;
                for (k, &byte) in a[2..8].iter().enumerate() {
                    abits |= (byte as u64) << (8 * k);
                }
                for (i, al) in alphas.iter_mut().enumerate() {
                    *al = ap[((abits >> (3 * i)) & 7) as usize];
                }
            }
            for py in 0..4 {
                for px in 0..4 {
                    let (x, y) = (bx * 4 + px, by * 4 + py);
                    if x >= w || y >= h {
                        continue;
                    }
                    let i = py * 4 + px;
                    let mut c = pal[((bits >> (2 * i)) & 3) as usize];
                    if dxt5 {
                        c[3] = alphas[i];
                    }
                    out[(y * w + x) * 4..(y * w + x) * 4 + 4].copy_from_slice(&c);
                }
            }
        }
    }
}

pub fn decode_material(db: &mut AssetDb, file: &Arc<SerializedFile>, v: &Value) -> Result<MaterialData> {
    let props = v.get("m_SavedProperties");
    let mut color = [1.0f32; 4];
    for pair in props.get("m_Colors").array() {
        if pair.get("first").str() == "_Color" {
            let c = pair.get("second");
            color = [c.get("r").f32(), c.get("g").f32(), c.get("b").f32(), c.get("a").f32()];
        }
    }
    let (mut main_tex, mut scale, mut offset) = (None, [1.0, 1.0], [0.0, 0.0]);
    for pair in props.get("m_TexEnvs").array() {
        if pair.get("first").str() == "_MainTex" {
            let t = pair.get("second");
            main_tex = db.resolve(file, t.get("m_Texture").pptr())?;
            let s = t.get("m_Scale");
            let o = t.get("m_Offset");
            scale = [s.get("x").f32(), s.get("y").f32()];
            offset = [o.get("x").f32(), o.get("y").f32()];
        }
    }
    let queue = v.get("m_CustomRenderQueue").i64();
    Ok(MaterialData {
        name: v.get("m_Name").str().to_string(),
        color,
        main_tex,
        tex_scale: scale,
        tex_offset: offset,
        transparent: queue > 2500,
    })
}
