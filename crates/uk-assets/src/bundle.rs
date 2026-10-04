//! UnityFS asset bundle container: header, block table, LZ4/LZMA blocks, file nodes.

use crate::reader::Reader;
use crate::{Error, Result};

#[derive(Debug)]
pub struct Node {
    pub offset: u64,
    pub size: u64,
    pub flags: u32,
    pub path: String,
}

/// A fully decompressed bundle: the concatenated block data plus its file table.
pub struct Bundle {
    pub unity_version: String,
    pub nodes: Vec<Node>,
    data: Vec<u8>,
}

impl Bundle {
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Self::parse(&bytes)
    }

    /// Reads only the header and block table: the file names inside, no data decompression.
    pub fn list_nodes(path: &std::path::Path) -> Result<Vec<String>> {
        use std::io::Read;
        let mut f = std::fs::File::open(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        let mut head = vec![0u8; 64 * 1024];
        let n = f.read(&mut head).map_err(|e| Error(e.to_string()))?;
        head.truncate(n);
        let mut r = Reader::new(&head, true);
        if r.cstr()? != "UnityFS" {
            return Err(Error("not UnityFS".into()));
        }
        r.u32()?;
        r.cstr()?;
        r.cstr()?;
        r.i64()?;
        let ci = r.u32()? as usize;
        let ui = r.u32()? as usize;
        let flags = r.u32()?;
        if flags & 0x80 != 0 {
            return Ok(Self::open(path)?.nodes.into_iter().map(|n| n.path).collect());
        }
        r.align(16);
        let info = decompress(r.take(ci)?, ui, flags & 0x3f)?;
        let mut ir = Reader::new(&info, true);
        ir.skip(16)?;
        let blocks = ir.i32()? as usize;
        ir.skip(blocks * 10)?;
        let nodes = ir.i32()?;
        let mut out = Vec::new();
        for _ in 0..nodes {
            ir.skip(20)?;
            out.push(ir.cstr()?);
        }
        Ok(out)
    }

    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes, true);
        let sig = r.cstr()?;
        if sig != "UnityFS" {
            return Err(Error(format!("not a UnityFS bundle ({sig})")));
        }
        let version = r.u32()?;
        let _player_version = r.cstr()?;
        let unity_version = r.cstr()?;
        let _size = r.i64()?;
        let ci_size = r.u32()? as usize;
        let ui_size = r.u32()? as usize;
        let flags = r.u32()?;
        if version >= 7 {
            r.align(16);
        }
        let info_raw = if flags & 0x80 != 0 {
            &bytes[bytes.len() - ci_size..]
        } else {
            r.take(ci_size)?
        };
        let info = decompress(info_raw, ui_size, flags & 0x3f)?;
        let mut ir = Reader::new(&info, true);
        ir.skip(16)?; // uncompressed data hash
        let block_count = ir.i32()?;
        let mut blocks = Vec::with_capacity(block_count as usize);
        for _ in 0..block_count {
            let usize_ = ir.u32()? as usize;
            let csize = ir.u32()? as usize;
            let bflags = ir.u16()?;
            blocks.push((usize_, csize, bflags));
        }
        let node_count = ir.i32()?;
        let mut nodes = Vec::with_capacity(node_count as usize);
        for _ in 0..node_count {
            nodes.push(Node { offset: ir.i64()? as u64, size: ir.i64()? as u64, flags: ir.u32()?, path: ir.cstr()? });
        }
        if flags & 0x200 != 0 {
            r.align(16);
        }
        let total: usize = blocks.iter().map(|b| b.0).sum();
        let mut data = Vec::with_capacity(total);
        for (usize_, csize, bflags) in blocks {
            let raw = r.take(csize)?;
            data.extend_from_slice(&decompress(raw, usize_, (bflags & 0x3f) as u32)?);
        }
        Ok(Self { unity_version, nodes, data })
    }

    pub fn file(&self, node: &Node) -> &[u8] {
        &self.data[node.offset as usize..(node.offset + node.size) as usize]
    }

    pub fn find(&self, name: &str) -> Option<&[u8]> {
        self.nodes.iter().find(|n| n.path == name).map(|n| self.file(n))
    }
}

fn decompress(raw: &[u8], out_size: usize, kind: u32) -> Result<Vec<u8>> {
    match kind {
        0 => Ok(raw.to_vec()),
        1 => {
            // Unity stores 5 property bytes then a raw LZMA stream; lzma-rs wants the
            // classic header (props + u64 size) in front.
            let mut hdr = Vec::with_capacity(raw.len() + 8);
            hdr.extend_from_slice(&raw[..5]);
            hdr.extend_from_slice(&(out_size as u64).to_le_bytes());
            hdr.extend_from_slice(&raw[5..]);
            let mut out = Vec::with_capacity(out_size);
            lzma_rs::lzma_decompress(&mut std::io::Cursor::new(hdr), &mut out).map_err(|e| Error(format!("lzma: {e}")))?;
            Ok(out)
        }
        2 | 3 => lz4_flex::block::decompress(raw, out_size).map_err(|e| Error(format!("lz4: {e}"))),
        k => Err(Error(format!("unsupported compression {k}"))),
    }
}
