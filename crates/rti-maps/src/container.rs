//! Generic GBX container: header chunks + (decompressed) body, for any class.

use crate::reader::{GbxError, GbxResult, Reader};

pub const SKIP: u32 = 0x534B_4950; // "PIKS"
pub const FACADE: u32 = 0xFACA_DE01;

pub struct Container {
    pub class_id: u32,
    pub version: u16,
    /// (chunk id, raw bytes)
    pub header_chunks: Vec<(u32, Vec<u8>)>,
    pub body: Vec<u8>,
}

pub fn read_container(data: &[u8]) -> anyhow::Result<Container> {
    let mut r = Reader::new(data);
    if r.bytes(3)? != b"GBX" {
        anyhow::bail!("not a GBX file");
    }
    let version = r.u16()?;
    anyhow::ensure!(version >= 3, "unsupported GBX version {version}");
    anyhow::ensure!(r.u8()? == b'B', "only binary GBX is supported");
    let _ref_compression = r.u8()?;
    let body_compression = r.u8()?;
    if version >= 4 {
        let _ = r.u8()?;
    }
    let class_id = r.u32()?;
    let mut header_chunks = vec![];
    if version >= 6 {
        let user_data_size = r.u32()? as usize;
        let start = r.pos;
        // embedded maps (inside replays) carry no header chunks at all
        let n = if user_data_size == 0 { 0 } else { r.u32()? };
        let mut entries = vec![];
        for _ in 0..n {
            let id = r.u32()?;
            let size = r.u32()? & 0x7FFF_FFFF;
            entries.push((id, size as usize));
        }
        for (id, size) in entries {
            header_chunks.push((id, r.bytes(size)?.to_vec()));
        }
        r.pos = start + user_data_size;
    }
    let _num_nodes = r.u32()?;
    let num_external = r.u32()?;
    if num_external > 0 {
        let _ancestor = r.u32()?;
        skip_folders(&mut r)?;
        for _ in 0..num_external {
            let flags = r.u32()?;
            if flags & 4 == 0 {
                let _ = r.string()?;
            } else {
                let _ = r.u32()?;
            }
            let _node_index = r.u32()?;
            if version >= 5 {
                let _use_file = r.u32()?;
            }
            if flags & 4 != 0 {
                let _folder = r.u32()?;
            }
        }
    }
    let body = if body_compression == b'C' {
        let uncompressed = r.u32()? as usize;
        let compressed = r.u32()? as usize;
        let src = r.bytes(compressed)?;
        let mut dst = vec![0u8; uncompressed];
        let n = lzokay::decompress::decompress(src, &mut dst)
            .map_err(|e| anyhow::anyhow!("LZO body decompression failed: {e:?}"))?;
        dst.truncate(n);
        dst
    } else {
        data[r.pos..].to_vec()
    };
    Ok(Container {
        class_id,
        version,
        header_chunks,
        body,
    })
}

fn skip_folders(r: &mut Reader) -> GbxResult<()> {
    let n = r.u32()?;
    for _ in 0..n {
        let _name = r.string()?;
        skip_folders(r)?;
    }
    Ok(())
}

/// Scan forward for the next `<class-prefixed chunk id><PIKS>` pair.
pub fn scan_next_skippable(body: &[u8], from: usize, class_prefix: u32) -> Option<usize> {
    let mut i = from;
    while i + 8 <= body.len() {
        let id = u32::from_le_bytes(body[i..i + 4].try_into().unwrap());
        if id & 0xFFFF_F000 == class_prefix {
            let next = u32::from_le_bytes(body[i + 4..i + 8].try_into().unwrap());
            if next == SKIP {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

pub fn zlib_inflate(src: &[u8], expected: usize) -> GbxResult<Vec<u8>> {
    use std::io::Read;
    let mut out = Vec::with_capacity(expected);
    flate2::read::ZlibDecoder::new(src)
        .read_to_end(&mut out)
        .map_err(|e| GbxError::Format(format!("zlib: {e}")))?;
    Ok(out)
}
