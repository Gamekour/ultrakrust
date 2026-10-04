//! Unity Shader objects (class 48) from the user's install: passes, render state, keyword variants,
//! and the compiled Vulkan programs (SMOL-V -> SPIR-V) with their parameter layouts.
//!
//! Layout notes (Unity 2022.3, verified by exact-length parsing in the harness):
//! - `compressedBlob` holds per-platform LZ4 chunks (`offsets`/`compressedLengths`/`decompressedLengths`).
//!   Chunk 0 is an index: u32 count, then count × (offset, length, chunk) for every entry.
//! - Program entry: u32 version (202012090), u32 gpu program type (25 = SPIR-V), 4 × u32 stats,
//!   keyword strings (u32 count + length-prefixed, 4-aligned), u32 code length, code blob.
//!   Code blob: u32 flags, then 6 × (offset, size) stage slots (vertex, fragment, hull, domain,
//!   geometry, ...) relative to the code blob start, each a SMOL-V module.
//! - Parameter entry: u32 version, u32 constant buffer count (the first one is an unnamed empty
//!   global block), per buffer: name, size, param count, params × (name, type, rows, cols,
//!   is_matrix, array_size, offset), struct count; then u32 binding count, bindings × (name, kind,
//!   packed binding, then 2 more words for textures (kind 0: sampler, dimension) or 1 for others).
use crate::serialized::Value;
use crate::{Error, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub const PLATFORM_VULKAN: i64 = 18;
pub const GPU_PROGRAM_SPIRV: u32 = 25;
const ENTRY_VERSION: u32 = 202012090;

/// A render-state value: a constant, or driven by a material property (e.g. `_CullMode`).
#[derive(Clone, Debug, Default)]
pub struct StateValue {
    pub value: f32,
    pub property: Option<String>,
}

impl StateValue {
    fn from(v: &Value) -> Self {
        let p = v.get("name").str();
        StateValue { value: v.get("val").f32(), property: (!p.is_empty() && p != "<noninit>").then(|| p.to_string()) }
    }
    /// Resolved against a material's float properties.
    pub fn resolve(&self, floats: &HashMap<String, f32>) -> f32 {
        self.property.as_ref().and_then(|p| floats.get(p).copied()).unwrap_or(self.value)
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenderState {
    pub cull: StateValue,
    pub zwrite: StateValue,
    pub ztest: StateValue,
    pub src_blend: StateValue,
    pub dst_blend: StateValue,
    pub src_blend_alpha: StateValue,
    pub dst_blend_alpha: StateValue,
    pub blend_op: StateValue,
    pub color_mask: StateValue,
}

/// One compiled variant of a stage.
#[derive(Clone, Debug)]
pub struct SubProgram {
    pub blob: u32,
    pub params: u32,
    /// Indices into `ShaderAsset::keyword_names`.
    pub keywords: Vec<u16>,
}

#[derive(Clone, Debug)]
pub struct Pass {
    pub name: String,
    pub tags: Vec<(String, String)>,
    pub state: RenderState,
    pub vertex: Vec<SubProgram>,
    pub fragment: Vec<SubProgram>,
}

impl Pass {
    pub fn tag(&self, k: &str) -> Option<&str> {
        self.tags.iter().find(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, b)| b.as_str())
    }
}

#[derive(Clone, Debug)]
pub struct Param {
    pub name: String,
    pub ty: u32,
    pub rows: u32,
    pub cols: u32,
    pub is_matrix: bool,
    pub array_size: u32,
    pub offset: u32,
}

#[derive(Clone, Debug)]
pub struct ConstantBuffer {
    pub name: String,
    pub size: u32,
    pub params: Vec<Param>,
}

#[derive(Clone, Debug)]
pub struct Binding {
    pub name: String,
    /// 0 texture, 1 constant buffer, 4 sampler (others seen: kept raw).
    pub kind: u32,
    pub packed: u32,
    pub extra: Vec<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Params {
    pub constant_buffers: Vec<ConstantBuffer>,
    pub bindings: Vec<Binding>,
}

/// A program entry: keywords it was compiled with and SPIR-V per stage slot.
#[derive(Clone, Debug)]
pub struct Program {
    pub gpu_type: u32,
    pub keywords: Vec<String>,
    /// Stage slot -> SPIR-V words (slot 0 vertex, 1 fragment).
    pub stages: Vec<Option<Vec<u32>>>,
}

pub struct ShaderAsset {
    pub name: String,
    pub keyword_names: Vec<String>,
    pub passes: Vec<Pass>,
    blob: Arc<Vec<u8>>,
    /// Vulkan chunk table: (offset, compressed, decompressed).
    chunks: Vec<(usize, usize, usize)>,
    /// entry -> (offset, length, chunk)
    index: Vec<(u32, u32, u32)>,
    cache: Mutex<HashMap<usize, Arc<Vec<u8>>>>,
}

struct Rd<'a> {
    b: &'a [u8],
    i: usize,
}

impl Rd<'_> {
    fn u(&mut self) -> Result<u32> {
        let w = self.b.get(self.i..self.i + 4).ok_or_else(|| Error("parameter blob truncated".into()))?;
        self.i += 4;
        Ok(u32::from_le_bytes(w.try_into().unwrap()))
    }
    fn s(&mut self) -> Result<String> {
        let n = self.u()? as usize;
        let b = self.b.get(self.i..self.i + n).ok_or_else(|| Error("string past end".into()))?;
        self.i += n.div_ceil(4) * 4;
        Ok(String::from_utf8_lossy(b).into_owned())
    }
}

impl ShaderAsset {
    pub fn from_value(v: &Value) -> Result<Self> {
        let pf = v.get("m_ParsedForm");
        let keyword_names = pf.get("m_KeywordNames").array().iter().map(|k| k.str().to_string()).collect();
        let subs = |p: &Value, stage: &str| -> Vec<SubProgram> {
            let prog = p.get(stage);
            let lists = prog.get("m_PlayerSubPrograms").array();
            let params = prog.get("m_ParameterBlobIndices").array();
            // the one populated list (shared by all platforms; blob indices are per platform)
            let Some(li) = lists.iter().position(|l| !l.array().is_empty()) else { return Vec::new() };
            lists[li]
                .array()
                .iter()
                .enumerate()
                .map(|(k, s)| SubProgram {
                    blob: s.get("m_BlobIndex").i64() as u32,
                    params: params.get(li).and_then(|l| l.array().get(k)).map(|x| x.i64() as u32).unwrap_or(u32::MAX),
                    keywords: s.get("m_KeywordIndices").array().iter().map(|x| x.i64() as u16).collect(),
                })
                .collect()
        };
        let mut passes = Vec::new();
        if let Some(ss) = pf.get("m_SubShaders").array().first() {
            for p in ss.get("m_Passes").array() {
                let st = p.get("m_State");
                let b = st.get("rtBlend0");
                passes.push(Pass {
                    name: st.get("m_Name").str().to_string(),
                    tags: st.get("m_Tags").get("tags").array().iter().map(|t| (t.get("first").str().to_string(), t.get("second").str().to_string())).collect(),
                    state: RenderState {
                        cull: StateValue::from(st.get("culling")),
                        zwrite: StateValue::from(st.get("zWrite")),
                        ztest: StateValue::from(st.get("zTest")),
                        src_blend: StateValue::from(b.get("srcBlend")),
                        dst_blend: StateValue::from(b.get("destBlend")),
                        src_blend_alpha: StateValue::from(b.get("srcBlendAlpha")),
                        dst_blend_alpha: StateValue::from(b.get("destBlendAlpha")),
                        blend_op: StateValue::from(b.get("blendOp")),
                        color_mask: StateValue::from(b.get("colMask")),
                    },
                    vertex: subs(p, "progVertex"),
                    fragment: subs(p, "progFragment"),
                });
            }
        }
        let plat = v
            .get("platforms")
            .array()
            .iter()
            .position(|p| p.i64() == PLATFORM_VULKAN)
            .ok_or_else(|| Error("shader has no Vulkan programs".into()))?;
        let col = |k: &str| -> Vec<usize> { v.get(k).array().get(plat).map(|a| a.array().iter().map(|x| x.i64() as usize).collect()).unwrap_or_default() };
        let (o, c, d) = (col("offsets"), col("compressedLengths"), col("decompressedLengths"));
        let chunks: Vec<_> = (0..o.len()).map(|i| (o[i], c[i], d[i])).collect();
        let blob = Arc::new(v.get("compressedBlob").bytes().to_vec());
        let mut s = ShaderAsset {
            name: pf.get("m_Name").str().to_string(),
            keyword_names,
            passes,
            blob,
            chunks,
            index: Vec::new(),
            cache: Mutex::new(HashMap::new()),
        };
        let idx = s.chunk(0)?;
        let w = |i: usize| u32::from_le_bytes(idx[i..i + 4].try_into().unwrap());
        let n = w(0) as usize;
        if 4 + n * 12 > idx.len() {
            return Err(Error("shader index chunk too short".into()));
        }
        s.index = (0..n).map(|e| (w(4 + e * 12), w(8 + e * 12), w(12 + e * 12))).collect();
        Ok(s)
    }

    pub fn entry_count(&self) -> usize {
        self.index.len()
    }

    fn chunk(&self, i: usize) -> Result<Arc<Vec<u8>>> {
        if let Some(c) = self.cache.lock().unwrap().get(&i) {
            return Ok(c.clone());
        }
        let &(o, c, d) = self.chunks.get(i).ok_or_else(|| Error(format!("no chunk {i}")))?;
        let src = self.blob.get(o..o + c).ok_or_else(|| Error("chunk past blob end".into()))?;
        let data = Arc::new(lz4_flex::block::decompress(src, d).map_err(|e| Error(format!("lz4: {e}")))?);
        let mut cache = self.cache.lock().unwrap();
        // chunks are up to 16 MB; keep the working set small
        if cache.len() >= 8 {
            cache.clear();
        }
        cache.insert(i, data.clone());
        Ok(data)
    }

    /// Raw bytes of index entry `e`.
    pub fn entry(&self, e: u32) -> Result<Vec<u8>> {
        let &(o, l, c) = self.index.get(e as usize).ok_or_else(|| Error(format!("no entry {e}")))?;
        let ch = self.chunk(c as usize)?;
        ch.get(o as usize..(o + l) as usize).map(|b| b.to_vec()).ok_or_else(|| Error(format!("entry {e} past chunk end")))
    }

    pub fn program(&self, e: u32) -> Result<Program> {
        let b = self.entry(e)?;
        let mut r = Rd { b: &b, i: 0 };
        if r.u()? != ENTRY_VERSION {
            return Err(Error(format!("entry {e}: unexpected version")));
        }
        let gpu_type = r.u()?;
        r.i += 16; // stats
        let nk = r.u()? as usize;
        let keywords = (0..nk).map(|_| r.s()).collect::<Result<Vec<_>>>()?;
        let clen = r.u()? as usize;
        let code = b.get(r.i..r.i + clen).ok_or_else(|| Error(format!("entry {e}: code past end")))?;
        let cw = |i: usize| code.get(i..i + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()) as usize);
        let mut stages = Vec::new();
        for slot in 0..6 {
            let (Some(o), Some(n)) = (cw(4 + slot * 8), cw(8 + slot * 8)) else { break };
            if n == 0 {
                stages.push(None);
                continue;
            }
            let smol = code.get(o..o + n).ok_or_else(|| Error(format!("entry {e}: stage {slot} past end")))?;
            stages.push(Some(crate::smolv::decode(smol).ok_or_else(|| Error(format!("entry {e}: SMOL-V decode failed")))?));
        }
        Ok(Program { gpu_type, keywords, stages })
    }

    pub fn params(&self, e: u32) -> Result<Params> {
        let b = self.entry(e)?;
        let mut r = Rd { b: &b, i: 0 };
        if r.u()? != ENTRY_VERSION {
            return Err(Error(format!("params {e}: unexpected version")));
        }
        let mut p = Params::default();
        for _ in 0..r.u()? {
            let name = r.s()?;
            let size = r.u()?;
            let n = r.u()?;
            let mut params = Vec::with_capacity((n as usize).min(r.b.len() / 28));
            for _ in 0..n {
                params.push(Param {
                    name: r.s()?,
                    ty: r.u()?,
                    rows: r.u()?,
                    cols: r.u()?,
                    is_matrix: r.u()? != 0,
                    array_size: r.u()?,
                    offset: r.u()?,
                });
            }
            if r.u()? != 0 {
                return Err(Error(format!("params {e}: struct params not supported yet ({name})")));
            }
            p.constant_buffers.push(ConstantBuffer { name, size, params });
        }
        for _ in 0..r.u()? {
            let name = r.s()?;
            let kind = r.u()?;
            let packed = r.u()?;
            let extra = (0..if kind == 0 { 2 } else { 1 }).map(|_| r.u()).collect::<Result<Vec<_>>>()?;
            p.bindings.push(Binding { name, kind, packed, extra });
        }
        if r.i != b.len() {
            return Err(Error(format!("params {e}: {} trailing bytes", b.len() - r.i)));
        }
        Ok(p)
    }

    /// Unity's variant choice for one stage: the subprogram whose keyword set equals the enabled
    /// keywords restricted to the keywords this stage was compiled with. Falls back to the closest
    /// set (most shared, fewest extra) if none matches exactly.
    pub fn select<'a>(&self, subs: &'a [SubProgram], enabled: &[&str]) -> Option<&'a SubProgram> {
        let space: std::collections::BTreeSet<u16> = subs.iter().flat_map(|s| s.keywords.iter().copied()).collect();
        let want: std::collections::BTreeSet<u16> = space
            .iter()
            .copied()
            .filter(|&k| self.keyword_names.get(k as usize).is_some_and(|n| enabled.contains(&n.as_str())))
            .collect();
        let score = |s: &SubProgram| {
            let have: std::collections::BTreeSet<u16> = s.keywords.iter().copied().collect();
            let shared = have.intersection(&want).count() as i64;
            let wrong = (have.len() as i64 - shared) + (want.len() as i64 - shared);
            -wrong
        };
        subs.iter().max_by_key(|s| score(s))
    }
}
