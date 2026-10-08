//! Read-only MS-NRBF (.NET BinaryFormatter) decoder for ULTRAKILL's save files (`Saves/Slot*/*.bepis`,
//! written by GameProgressSaver). Objects become `Value::Struct`s with a leading `$type` field;
//! arrays become `Value::Array`; references are resolved from the root.

use crate::serialized::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Debug)]
enum MemberType {
    Primitive(u8),
    String,
    Object,
    SystemClass,
    Class,
    ObjectArray,
    StringArray,
    PrimitiveArray,
}

#[derive(Clone, Debug)]
struct ClassMeta {
    name: String,
    members: Vec<String>,
    types: Option<Vec<MemberType>>,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
    meta: HashMap<i32, Arc<ClassMeta>>,
    objects: HashMap<i32, Value>,
}

const REF: &str = "$ref";

fn reference(id: i32) -> Value {
    Value::Struct(vec![(REF.into(), Value::Int(id as i64))])
}

/// Marker for ObjectNullMultiple: how many nulls to emit.
enum Item {
    One(Value),
    Nulls(usize),
    End,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let s = self.b.get(self.p..self.p + n).ok_or_else(|| format!("nrbf: eof at {}", self.p))?;
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn i32(&mut self) -> Result<i32, String> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String, String> {
        let mut len = 0usize;
        for i in 0..5 {
            let b = self.u8()?;
            len |= ((b & 0x7f) as usize) << (7 * i);
            if b & 0x80 == 0 {
                break;
            }
        }
        Ok(String::from_utf8_lossy(self.take(len)?).into_owned())
    }

    fn primitive(&mut self, t: u8) -> Result<Value, String> {
        let mut le = |n: usize| -> Result<[u8; 8], String> {
            let mut a = [0u8; 8];
            a[..n].copy_from_slice(self.take(n)?);
            Ok(a)
        };
        Ok(match t {
            1 => Value::Bool(le(1)?[0] != 0),
            2 => Value::UInt(le(1)?[0] as u64),
            3 => {
                // Char: one UTF-8 code point
                let b0 = self.u8()?;
                let n = if b0 < 0x80 { 0 } else if b0 >= 0xf0 { 3 } else if b0 >= 0xe0 { 2 } else { 1 };
                let mut v = vec![b0];
                v.extend_from_slice(self.take(n)?);
                Value::Str(String::from_utf8_lossy(&v).into_owned())
            }
            5 | 18 => Value::Str(self.string()?),
            6 => Value::Float(f64::from_le_bytes(le(8)?)),
            7 => Value::Int(i16::from_le_bytes(le(2)?[..2].try_into().unwrap()) as i64),
            8 => Value::Int(i32::from_le_bytes(le(4)?[..4].try_into().unwrap()) as i64),
            9 | 12 => Value::Int(i64::from_le_bytes(le(8)?)),
            13 => Value::UInt(u64::from_le_bytes(le(8)?)),
            10 => Value::Int(le(1)?[0] as i8 as i64),
            11 => Value::Float(f32::from_le_bytes(le(4)?[..4].try_into().unwrap()) as f64),
            14 => Value::UInt(u16::from_le_bytes(le(2)?[..2].try_into().unwrap()) as u64),
            15 => Value::UInt(u32::from_le_bytes(le(4)?[..4].try_into().unwrap()) as u64),
            16 => Value::UInt(u64::from_le_bytes(le(8)?)),
            17 => Value::Int(0),
            _ => return Err(format!("nrbf: primitive type {t}")),
        })
    }

    fn class_info(&mut self) -> Result<(i32, String, Vec<String>), String> {
        let id = self.i32()?;
        let name = self.string()?;
        let n = self.i32()?;
        let members = (0..n).map(|_| self.string()).collect::<Result<_, _>>()?;
        Ok((id, name, members))
    }

    fn member_types(&mut self, n: usize) -> Result<Vec<MemberType>, String> {
        let kinds = self.take(n)?.to_vec();
        kinds
            .into_iter()
            .map(|k| {
                Ok(match k {
                    0 => MemberType::Primitive(self.u8()?),
                    1 => MemberType::String,
                    2 => MemberType::Object,
                    3 => {
                        self.string()?;
                        MemberType::SystemClass
                    }
                    4 => {
                        self.string()?;
                        self.i32()?;
                        MemberType::Class
                    }
                    5 => MemberType::ObjectArray,
                    6 => MemberType::StringArray,
                    7 => {
                        self.u8()?;
                        MemberType::PrimitiveArray
                    }
                    _ => return Err(format!("nrbf: binary type {k}")),
                })
            })
            .collect()
    }

    fn members(&mut self, id: i32, meta: Arc<ClassMeta>) -> Result<Value, String> {
        let mut fields: Vec<(Arc<str>, Value)> = vec![("$type".into(), Value::Str(meta.name.clone()))];
        let mut pending_nulls = 0usize;
        for (i, m) in meta.members.iter().enumerate() {
            let v = match meta.types.as_ref().map(|t| &t[i]) {
                Some(MemberType::Primitive(t)) => self.primitive(*t)?,
                _ if pending_nulls > 0 => {
                    pending_nulls -= 1;
                    Value::Int(0)
                }
                _ => match self.record()? {
                    Item::One(v) => v,
                    Item::Nulls(n) => {
                        pending_nulls = n - 1;
                        Value::Int(0)
                    }
                    Item::End => return Err("nrbf: MessageEnd inside object".into()),
                },
            };
            fields.push((m.as_str().into(), v));
        }
        let v = Value::Struct(fields);
        self.objects.insert(id, v.clone());
        Ok(reference(id))
    }

    fn elements(&mut self, n: usize, prim: Option<u8>) -> Result<Vec<Value>, String> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            if let Some(t) = prim {
                out.push(self.primitive(t)?);
                continue;
            }
            match self.record()? {
                Item::One(v) => out.push(v),
                Item::Nulls(k) => out.extend(std::iter::repeat_n(Value::Int(0), k)),
                Item::End => return Err("nrbf: MessageEnd inside array".into()),
            }
        }
        out.truncate(n);
        Ok(out)
    }

    fn record(&mut self) -> Result<Item, String> {
        let kind = self.u8()?;
        Ok(match kind {
            0 => {
                self.take(16)?;
                self.record()?
            }
            1 => {
                // ClassWithId
                let id = self.i32()?;
                let mid = self.i32()?;
                let meta = self.meta.get(&mid).cloned().ok_or("nrbf: unknown metadata id")?;
                self.meta.insert(id, meta.clone());
                Item::One(self.members(id, meta)?)
            }
            2 | 3 => {
                // (System)ClassWithMembers: untyped members follow as records
                let (id, name, members) = self.class_info()?;
                if kind == 3 {
                    self.i32()?;
                }
                let meta = Arc::new(ClassMeta { name, members, types: None });
                self.meta.insert(id, meta.clone());
                Item::One(self.members(id, meta)?)
            }
            4 | 5 => {
                let (id, name, members) = self.class_info()?;
                let types = self.member_types(members.len())?;
                if kind == 5 {
                    self.i32()?;
                }
                let meta = Arc::new(ClassMeta { name, members, types: Some(types) });
                self.meta.insert(id, meta.clone());
                Item::One(self.members(id, meta)?)
            }
            6 => {
                let id = self.i32()?;
                let s = Value::Str(self.string()?);
                self.objects.insert(id, s.clone());
                Item::One(s)
            }
            7 => {
                // BinaryArray
                let id = self.i32()?;
                let at = self.u8()?;
                let rank = self.i32()? as usize;
                let lens: Vec<i32> = (0..rank).map(|_| self.i32()).collect::<Result<_, _>>()?;
                if at >= 3 {
                    for _ in 0..rank {
                        self.i32()?;
                    }
                }
                let t = self.member_types(1)?.remove(0);
                let n = lens.iter().map(|&l| l.max(0) as usize).product();
                let prim = if let MemberType::Primitive(p) = t { Some(p) } else { None };
                let v = Value::Array(self.elements(n, prim)?);
                self.objects.insert(id, v);
                Item::One(reference(id))
            }
            8 => {
                let t = self.u8()?;
                Item::One(self.primitive(t)?)
            }
            9 => Item::One(reference(self.i32()?)),
            10 => Item::Nulls(1),
            11 => Item::End,
            12 => {
                self.i32()?;
                self.string()?;
                self.record()?
            }
            13 => Item::Nulls(self.u8()? as usize),
            14 => Item::Nulls(self.i32()? as usize),
            15 => {
                let id = self.i32()?;
                let n = self.i32()? as usize;
                let t = self.u8()?;
                let v = Value::Array(self.elements(n, Some(t))?);
                self.objects.insert(id, v);
                Item::One(reference(id))
            }
            16 | 17 => {
                let id = self.i32()?;
                let n = self.i32()? as usize;
                let v = Value::Array(self.elements(n, None)?);
                self.objects.insert(id, v);
                Item::One(reference(id))
            }
            _ => return Err(format!("nrbf: record type {kind} at {}", self.p - 1)),
        })
    }

    fn resolve(&self, v: &Value, depth: usize) -> Value {
        if depth > 64 {
            return Value::Int(0);
        }
        match v {
            Value::Struct(f) if f.len() == 1 && &*f[0].0 == REF => match self.objects.get(&(f[0].1.i64() as i32)) {
                Some(o) => self.resolve(o, depth + 1),
                None => Value::Int(0),
            },
            Value::Struct(f) => Value::Struct(f.iter().map(|(k, x)| (k.clone(), self.resolve(x, depth + 1))).collect()),
            Value::Array(a) => Value::Array(a.iter().map(|x| self.resolve(x, depth + 1)).collect()),
            _ => v.clone(),
        }
    }
}

/// Decodes a BinaryFormatter stream into its root object.
pub fn decode(bytes: &[u8]) -> Result<Value, String> {
    let mut r = Reader { b: bytes, p: 0, meta: HashMap::new(), objects: HashMap::new() };
    if r.u8()? != 0 {
        return Err("nrbf: no serialization header".into());
    }
    let root = r.i32()?;
    r.take(12)?;
    loop {
        match r.record()? {
            Item::End => break,
            Item::One(_) | Item::Nulls(_) => {}
        }
        if r.p >= r.b.len() {
            break;
        }
    }
    Ok(r.resolve(&reference(root), 0))
}
