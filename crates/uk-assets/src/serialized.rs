//! Unity SerializedFile (format 22, Unity 2022.3): types with embedded typetrees,
//! the object table and external file references. Objects are decoded generically
//! from their typetree into [`Value`].

use crate::common_strings::COMMON_STRINGS;
use crate::reader::Reader;
use crate::{Error, Result};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

#[derive(Debug, Clone)]
pub struct TypeNode {
    pub level: u8,
    pub type_flags: u8,
    pub ty: Arc<str>,
    pub name: Arc<str>,
    pub byte_size: i32,
    pub meta_flag: i32,
}

#[derive(Debug, Clone)]
pub struct SerType {
    pub class_id: i32,
    /// MonoBehaviour script type hash (the same script has the same hash in every file of a build)
    pub script_hash: Option<[u8; 16]>,
    pub nodes: Vec<TypeNode>,
}

#[derive(Debug, Clone)]
pub struct ObjectInfo {
    pub path_id: i64,
    pub byte_start: u64,
    pub byte_size: u32,
    pub type_index: usize,
    pub class_id: i32,
}

#[derive(Debug, Clone)]
pub struct External {
    pub path: String,
}

impl External {
    /// "archive:/CAB-xxx/CAB-xxx" -> "CAB-xxx"; "Library/unity default resources" -> "unity default resources".
    pub fn file_name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }
}

pub struct SerializedFile {
    pub name: String,
    pub data: Arc<[u8]>,
    pub big_endian: bool,
    pub data_offset: u64,
    pub types: Vec<SerType>,
    pub objects: Vec<ObjectInfo>,
    pub externals: Vec<External>,
    index: HashMap<i64, usize>,
    /// class id -> typetree, for files stored without typetrees (built-in resources).
    pub fallback: OnceLock<HashMap<i32, Vec<TypeNode>>>,
    /// script type hash -> MonoBehaviour typetree, for files stored without typetrees.
    pub fallback_scripts: OnceLock<HashMap<[u8; 16], Vec<TypeNode>>>,
}

/// Generic decoded object field.
#[derive(Debug, Clone)]
pub enum Value {
    Bool(bool),
    Int(i64),
    UInt(u64),
    Float(f64),
    Str(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Struct(Vec<(Arc<str>, Value)>),
}

static NULL: Value = Value::Int(0);

impl Value {
    pub fn get(&self, key: &str) -> &Value {
        match self {
            Value::Struct(f) => f.iter().find(|(k, _)| &**k == key).map(|(_, v)| v).unwrap_or(&NULL),
            _ => &NULL,
        }
    }

    pub fn has(&self, key: &str) -> bool {
        matches!(self, Value::Struct(f) if f.iter().any(|(k, _)| &**k == key))
    }

    pub fn i64(&self) -> i64 {
        match self {
            Value::Int(v) => *v,
            Value::UInt(v) => *v as i64,
            Value::Bool(b) => *b as i64,
            Value::Float(f) => *f as i64,
            _ => 0,
        }
    }

    pub fn f32(&self) -> f32 {
        match self {
            Value::Float(f) => *f as f32,
            Value::Int(v) => *v as f32,
            Value::UInt(v) => *v as f32,
            _ => 0.0,
        }
    }

    pub fn bool(&self) -> bool {
        self.i64() != 0
    }

    pub fn str(&self) -> &str {
        match self {
            Value::Str(s) => s,
            _ => "",
        }
    }

    pub fn bytes(&self) -> &[u8] {
        match self {
            Value::Bytes(b) => b,
            _ => &[],
        }
    }

    pub fn array(&self) -> &[Value] {
        match self {
            Value::Array(a) => a,
            _ => &[],
        }
    }

    pub fn vec3(&self) -> [f32; 3] {
        [self.get("x").f32(), self.get("y").f32(), self.get("z").f32()]
    }

    pub fn quat(&self) -> [f32; 4] {
        [self.get("x").f32(), self.get("y").f32(), self.get("z").f32(), self.get("w").f32()]
    }

    /// Compact JSON-like rendering (byte arrays summarized), for debugging.
    pub fn compact(&self) -> String {
        let mut out = String::new();
        self.write_compact(&mut out);
        out
    }

    fn write_compact(&self, o: &mut String) {
        use std::fmt::Write;
        match self {
            Value::Bool(b) => write!(o, "{b}").unwrap(),
            Value::Int(v) => write!(o, "{v}").unwrap(),
            Value::UInt(v) => write!(o, "{v}").unwrap(),
            Value::Float(v) => write!(o, "{v}").unwrap(),
            Value::Str(s) => write!(o, "{s:?}").unwrap(),
            Value::Bytes(b) => write!(o, "<{} bytes>", b.len()).unwrap(),
            Value::Array(a) => {
                o.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    v.write_compact(o);
                }
                o.push(']');
            }
            Value::Struct(f) => {
                if f.len() == 2 && &*f[0].0 == "m_FileID" {
                    write!(o, "@{}:{}", f[0].1.i64(), f[1].1.i64()).unwrap();
                    return;
                }
                o.push('{');
                for (i, (k, v)) in f.iter().enumerate() {
                    if i > 0 {
                        o.push(',');
                    }
                    write!(o, "{k}:").unwrap();
                    v.write_compact(o);
                }
                o.push('}');
            }
        }
    }

    /// PPtr -> (file_id, path_id).
    pub fn pptr(&self) -> (i32, i64) {
        (self.get("m_FileID").i64() as i32, self.get("m_PathID").i64())
    }
}

impl std::fmt::Debug for SerializedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SerializedFile({})", self.name)
    }
}

impl SerializedFile {
    pub fn parse(name: &str, data: Arc<[u8]>) -> Result<Self> {
        let mut r = Reader::new(&data, true);
        let _meta = r.u32()?;
        let _fsize = r.u32()?;
        let version = r.u32()?;
        let mut data_offset = r.u32()? as u64;
        if !(17..=23).contains(&version) {
            return Err(Error(format!("{name}: unsupported SerializedFile version {version}")));
        }
        let big = r.u8()? != 0;
        r.skip(3)?;
        if version >= 22 {
            let _meta = r.u32()?;
            let _fsize = r.i64()?;
            data_offset = r.i64()? as u64;
            let _unknown = r.i64()?;
        }
        r.big = big;
        let _unity_version = r.cstr()?;
        let _platform = r.i32()?;
        let has_tree = r.bool()?;
        let type_count = r.i32()?;
        let mut types = Vec::with_capacity(type_count.max(0) as usize);
        for _ in 0..type_count {
            types.push(read_type(&mut r, version, false, has_tree)?);
        }
        let obj_count = r.i32()?;
        let mut objects = Vec::with_capacity(obj_count.max(0) as usize);
        for _ in 0..obj_count {
            r.align(4);
            let path_id = r.i64()?;
            let byte_start = if version >= 22 { r.i64()? as u64 } else { r.u32()? as u64 };
            let byte_size = r.u32()?;
            let type_index = r.i32()? as usize;
            let class_id = types.get(type_index).map(|t| t.class_id).unwrap_or(-1);
            objects.push(ObjectInfo { path_id, byte_start, byte_size, type_index, class_id });
        }
        let script_count = r.i32()?;
        for _ in 0..script_count {
            let _file = r.i32()?;
            r.align(4);
            let _id = r.i64()?;
        }
        let ext_count = r.i32()?;
        let mut externals = Vec::with_capacity(ext_count.max(0) as usize);
        for _ in 0..ext_count {
            let _empty = r.cstr()?;
            r.skip(16 + 4)?;
            externals.push(External { path: r.cstr()? });
        }
        let index = objects.iter().enumerate().map(|(i, o)| (o.path_id, i)).collect();
        Ok(Self {
            name: name.to_string(),
            data,
            big_endian: big,
            data_offset,
            types,
            objects,
            externals,
            index,
            fallback: OnceLock::new(),
            fallback_scripts: OnceLock::new(),
        })
    }

    pub fn has_typetrees(&self) -> bool {
        self.types.iter().any(|t| !t.nodes.is_empty())
    }

    pub fn object(&self, path_id: i64) -> Option<&ObjectInfo> {
        self.index.get(&path_id).map(|&i| &self.objects[i])
    }

    pub fn read(&self, obj: &ObjectInfo) -> Result<Value> {
        let start = (self.data_offset + obj.byte_start) as usize;
        let bytes = &self.data[start..start + obj.byte_size as usize];
        let mut r = Reader::new(bytes, self.big_endian);
        let own = &self.types[obj.type_index].nodes;
        let ty = &self.types[obj.type_index];
        let by_script = ty.script_hash.and_then(|h| self.fallback_scripts.get().and_then(|m| m.get(&h)));
        let nodes: &[TypeNode] = if !own.is_empty() {
            own
        } else if let Some(n) = by_script {
            n
        } else {
            self.fallback
                .get()
                .and_then(|m| m.get(&obj.class_id))
                .ok_or_else(|| Error(format!("{}: no typetree for class {}", self.name, obj.class_id)))?
        };
        let (v, _) = read_value(&mut r, nodes, 0)?;
        Ok(v)
    }

    pub fn read_id(&self, path_id: i64) -> Result<Value> {
        let o = self.object(path_id).ok_or_else(|| Error(format!("{}: no object {path_id}", self.name)))?;
        self.read(o)
    }
}

fn read_type(r: &mut Reader, version: u32, is_ref: bool, has_tree: bool) -> Result<SerType> {
    let class_id = r.i32()?;
    let _stripped = r.u8()?;
    let script_index = r.i16()?;
    let mut script_hash = None;
    if (is_ref && script_index >= 0) || class_id == 114 {
        script_hash = Some(<[u8; 16]>::try_from(r.take(16)?).unwrap());
    }
    r.skip(16)?;
    if !has_tree {
        return Ok(SerType { class_id, script_hash, nodes: Vec::new() });
    }
    let node_count = r.i32()? as usize;
    let str_size = r.i32()? as usize;
    let mut raw = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let _v = r.u16()?;
        let level = r.u8()?;
        let type_flags = r.u8()?;
        let t_off = r.u32()?;
        let n_off = r.u32()?;
        let byte_size = r.i32()?;
        let _index = r.i32()?;
        let meta_flag = r.i32()?;
        if version >= 19 {
            r.skip(8)?;
        }
        raw.push((level, type_flags, t_off, n_off, byte_size, meta_flag));
    }
    let strbuf = r.take(str_size)?;
    let s = |off: u32| -> Arc<str> {
        let (buf, o) = if off & 0x8000_0000 != 0 {
            (COMMON_STRINGS.as_bytes(), (off & 0x7fff_ffff) as usize)
        } else {
            (strbuf, off as usize)
        };
        let end = buf[o..].iter().position(|&b| b == 0).map(|e| o + e).unwrap_or(buf.len());
        Arc::from(String::from_utf8_lossy(&buf[o..end]).as_ref())
    };
    let nodes = raw
        .into_iter()
        .map(|(level, type_flags, t, n, byte_size, meta_flag)| TypeNode { level, type_flags, ty: s(t), name: s(n), byte_size, meta_flag })
        .collect();
    if version >= 21 {
        if is_ref {
            r.cstr()?;
            r.cstr()?;
            r.cstr()?;
        } else {
            let deps = r.i32()?;
            r.skip(deps as usize * 4)?;
        }
    }
    Ok(SerType { class_id, script_hash, nodes })
}

/// Index just past the subtree rooted at `i`.
fn subtree_end(nodes: &[TypeNode], i: usize) -> usize {
    let lvl = nodes[i].level;
    let mut j = i + 1;
    while j < nodes.len() && nodes[j].level > lvl {
        j += 1;
    }
    j
}

fn children(nodes: &[TypeNode], i: usize) -> Vec<usize> {
    let end = subtree_end(nodes, i);
    let lvl = nodes[i].level + 1;
    (i + 1..end).filter(|&j| nodes[j].level == lvl).collect()
}

const ALIGN: i32 = 0x4000;

fn read_value(r: &mut Reader, nodes: &[TypeNode], i: usize) -> Result<(Value, usize)> {
    let n = &nodes[i];
    let end = subtree_end(nodes, i);
    let v = match &*n.ty {
        "SInt8" => Value::Int(r.u8()? as i8 as i64),
        "UInt8" | "char" => Value::UInt(r.u8()? as u64),
        "bool" => Value::Bool(r.bool()?),
        "SInt16" | "short" => Value::Int(r.i16()? as i64),
        "UInt16" | "unsigned short" => Value::UInt(r.u16()? as u64),
        "SInt32" | "int" | "Type*" => Value::Int(r.i32()? as i64),
        "UInt32" | "unsigned int" => Value::UInt(r.u32()? as u64),
        "SInt64" | "long long" => Value::Int(r.i64()?),
        "UInt64" | "unsigned long long" | "FileSize" => Value::UInt(r.u64()?),
        "float" => Value::Float(r.f32()? as f64),
        "double" => Value::Float(r.f64()?),
        "string" => {
            let len = r.i32()? as usize;
            let s = String::from_utf8_lossy(r.take(len)?).into_owned();
            let ch = children(nodes, i);
            if ch.first().is_some_and(|&c| nodes[c].meta_flag & ALIGN != 0) {
                r.align(4);
            }
            Value::Str(s)
        }
        "TypelessData" => {
            let len = r.i32()? as usize;
            Value::Bytes(r.take(len)?.to_vec())
        }
        _ => {
            let ch = children(nodes, i);
            if ch.len() == 1 && (nodes[ch[0]].type_flags & 1 != 0 || &*nodes[ch[0]].ty == "Array") {
                let arr = ch[0];
                let arr_ch = children(nodes, arr);
                let count = r.i32()? as usize;
                let data = arr_ch[1];
                let v = match &*nodes[data].ty {
                    "UInt8" | "char" | "SInt8" => Value::Bytes(r.take(count)?.to_vec()),
                    _ => {
                        let mut items = Vec::with_capacity(count.min(1 << 20));
                        for _ in 0..count {
                            items.push(read_value(r, nodes, data)?.0);
                        }
                        Value::Array(items)
                    }
                };
                if nodes[arr].meta_flag & ALIGN != 0 {
                    r.align(4);
                }
                v
            } else {
                let mut fields = Vec::with_capacity(ch.len());
                for c in ch {
                    let (v, _) = read_value(r, nodes, c)?;
                    fields.push((nodes[c].name.clone(), v));
                }
                Value::Struct(fields)
            }
        }
    };
    if n.meta_flag & ALIGN != 0 {
        r.align(4);
    }
    Ok((v, end))
}
