//! Endian-aware byte cursor.

use crate::{Error, Result};

#[derive(Clone)]
pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
    pub big: bool,
}

macro_rules! num {
    ($name:ident, $t:ty) => {
        pub fn $name(&mut self) -> Result<$t> {
            const N: usize = std::mem::size_of::<$t>();
            let b: [u8; N] = self.take(N)?.try_into().unwrap();
            Ok(if self.big { <$t>::from_be_bytes(b) } else { <$t>::from_le_bytes(b) })
        }
    };
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8], big: bool) -> Self {
        Self { data, pos: 0, big }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error(format!("read past end ({} + {n} > {})", self.pos, self.data.len())));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    pub fn align(&mut self, a: usize) {
        self.pos = self.pos.div_ceil(a) * a;
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn bool(&mut self) -> Result<bool> {
        Ok(self.u8()? != 0)
    }

    num!(u16, u16);
    num!(i16, i16);
    num!(u32, u32);
    num!(i32, i32);
    num!(u64, u64);
    num!(i64, i64);
    num!(f32, f32);
    num!(f64, f64);

    pub fn cstr(&mut self) -> Result<String> {
        let rest = &self.data[self.pos..];
        let end = rest.iter().position(|&b| b == 0).ok_or_else(|| Error("unterminated string".into()))?;
        let s = String::from_utf8_lossy(&rest[..end]).into_owned();
        self.pos += end + 1;
        Ok(s)
    }
}
