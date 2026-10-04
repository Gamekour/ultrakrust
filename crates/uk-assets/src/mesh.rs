//! Unity `Mesh` decoding (uncompressed vertex data, Unity 2019+ channel layout).

use crate::serialized::Value;
use crate::{Error, Result};

#[derive(Clone, Debug, Default)]
pub struct SubMesh {
    /// Triangle indices (already offset by baseVertex), referencing the mesh vertex arrays.
    pub indices: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct MeshData {
    pub name: String,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uv0: Vec<[f32; 2]>,
    pub colors: Vec<[f32; 4]>,
    pub submeshes: Vec<SubMesh>,
}

const CH_POS: usize = 0;
const CH_NORMAL: usize = 1;
const CH_COLOR: usize = 3;
const CH_UV0: usize = 4;

fn format_size(f: i64) -> usize {
    match f {
        0 | 10 | 11 => 4,
        1 | 4 | 5 | 8 | 9 => 2,
        _ => 1,
    }
}

pub fn half_to_f32(h: u16) -> f32 {
    let s = ((h >> 15) as u32) << 31;
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as u32;
    let bits = if e == 0 {
        if m == 0 {
            s
        } else {
            // subnormal
            let mut e = -14;
            let mut m = m;
            while m & 0x400 == 0 {
                m <<= 1;
                e -= 1;
            }
            s | (((e + 127) as u32) << 23) | ((m & 0x3ff) << 13)
        }
    } else if e == 31 {
        s | 0x7f80_0000 | (m << 13)
    } else {
        s | (((e - 15 + 127) as u32) << 23) | (m << 13)
    };
    f32::from_bits(bits)
}

fn read_component(b: &[u8], fmt: i64) -> f32 {
    match fmt {
        0 => f32::from_le_bytes(b[..4].try_into().unwrap()),
        1 => half_to_f32(u16::from_le_bytes([b[0], b[1]])),
        2 => b[0] as f32 / 255.0,
        3 => (b[0] as i8 as f32 / 127.0).max(-1.0),
        4 => u16::from_le_bytes([b[0], b[1]]) as f32 / 65535.0,
        5 => (i16::from_le_bytes([b[0], b[1]]) as f32 / 32767.0).max(-1.0),
        6 => b[0] as f32,
        7 => b[0] as i8 as f32,
        8 => u16::from_le_bytes([b[0], b[1]]) as f32,
        9 => i16::from_le_bytes([b[0], b[1]]) as f32,
        10 => u32::from_le_bytes(b[..4].try_into().unwrap()) as f32,
        _ => i32::from_le_bytes(b[..4].try_into().unwrap()) as f32,
    }
}

/// Decodes a Mesh. `stream_data` supplies the bytes when the vertex data lives in a .resS file.
pub fn decode(v: &Value, stream_data: Option<&[u8]>) -> Result<MeshData> {
    if v.get("m_MeshCompression").i64() != 0 {
        return Err(Error(format!("{}: compressed meshes are not supported", v.get("m_Name").str())));
    }
    let vd = v.get("m_VertexData");
    let count = vd.get("m_VertexCount").i64() as usize;
    let data: &[u8] = match stream_data {
        Some(s) if vd.get("m_DataSize").bytes().is_empty() => s,
        _ => vd.get("m_DataSize").bytes(),
    };
    let channels: Vec<(usize, usize, i64, usize)> = vd
        .get("m_Channels")
        .array()
        .iter()
        .map(|c| {
            (
                c.get("stream").i64() as usize,
                c.get("offset").i64() as usize,
                c.get("format").i64(),
                (c.get("dimension").i64() & 0xf) as usize,
            )
        })
        .collect();
    // Stream layout: stride = channel bytes per stream; streams are 16-byte aligned.
    let stream_count = channels.iter().filter(|c| c.3 > 0).map(|c| c.0 + 1).max().unwrap_or(0);
    let mut stream_offset = vec![0usize; stream_count];
    let mut stride = vec![0usize; stream_count];
    let mut off = 0usize;
    for s in 0..stream_count {
        stride[s] = channels.iter().filter(|c| c.0 == s && c.3 > 0).map(|c| c.3 * format_size(c.2)).sum();
        stream_offset[s] = off;
        off += stride[s] * count;
        off = (off + 15) & !15;
    }
    let read_channel = |ch: usize| -> Option<Vec<Vec<f32>>> {
        let &(s, o, fmt, dim) = channels.get(ch)?;
        if dim == 0 {
            return None;
        }
        let fs = format_size(fmt);
        let mut out = Vec::with_capacity(count);
        for i in 0..count {
            let base = stream_offset[s] + i * stride[s] + o;
            let mut c = Vec::with_capacity(dim);
            for d in 0..dim {
                let p = base + d * fs;
                c.push(data.get(p..p + fs).map(|b| read_component(b, fmt)).unwrap_or(0.0));
            }
            out.push(c);
        }
        Some(out)
    };
    let positions: Vec<[f32; 3]> = read_channel(CH_POS)
        .ok_or_else(|| Error("mesh without positions".into()))?
        .into_iter()
        .map(|c| [c[0], c[1], c[2]])
        .collect();
    let normals = read_channel(CH_NORMAL).map(|n| n.into_iter().map(|c| [c[0], c[1], c[2]]).collect()).unwrap_or_default();
    let uv0 = read_channel(CH_UV0).map(|n| n.into_iter().map(|c| [c[0], c[1]]).collect()).unwrap_or_default();
    let colors = read_channel(CH_COLOR)
        .map(|n| n.into_iter().map(|c| [c[0], c[1], c[2], *c.get(3).unwrap_or(&1.0)]).collect())
        .unwrap_or_default();

    let ib = v.get("m_IndexBuffer").bytes();
    let wide = v.get("m_IndexFormat").i64() == 1;
    let index = |i: usize| -> u32 {
        if wide {
            u32::from_le_bytes(ib[i * 4..i * 4 + 4].try_into().unwrap())
        } else {
            u16::from_le_bytes([ib[i * 2], ib[i * 2 + 1]]) as u32
        }
    };
    let isz = if wide { 4 } else { 2 };
    let mut submeshes = Vec::new();
    for sm in v.get("m_SubMeshes").array() {
        let first = sm.get("firstByte").i64() as usize / isz;
        let n = sm.get("indexCount").i64() as usize;
        let base = sm.get("baseVertex").i64() as u32;
        let topo = sm.get("topology").i64();
        let mut indices = Vec::new();
        if topo == 0 && (first + n) * isz <= ib.len() {
            indices = (first..first + n).map(|i| index(i) + base).collect();
        }
        submeshes.push(SubMesh { indices });
    }
    Ok(MeshData { name: v.get("m_Name").str().to_string(), positions, normals, uv0, colors, submeshes })
}
