//! Little-endian binary reader with GBX-specific primitives (lookback
//! strings, node references, pack descriptors).

use std::collections::HashMap;

#[derive(Debug, thiserror::Error)]
pub enum GbxError {
    #[error("unexpected end of data at {0} (need {1} bytes)")]
    Eof(usize, usize),
    #[error("{0}")]
    Format(String),
}

pub type GbxResult<T> = Result<T, GbxError>;

/// Lookback ("Id") string table state. One per header and one per body.
#[derive(Default, Clone, Debug)]
pub struct IdState {
    pub version: Option<u32>,
    pub strings: Vec<String>,
}

/// Nodes already read in this body (index → class id) so that references
/// to earlier nodes can be resolved without re-reading.
#[derive(Default, Debug)]
pub struct NodeState {
    pub seen: HashMap<u32, u32>,
}

pub struct Reader<'a> {
    pub data: &'a [u8],
    pub pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn need(&self, n: usize) -> GbxResult<()> {
        if self.pos + n > self.data.len() {
            Err(GbxError::Eof(self.pos, n))
        } else {
            Ok(())
        }
    }

    pub fn bytes(&mut self, n: usize) -> GbxResult<&'a [u8]> {
        self.need(n)?;
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn skip(&mut self, n: usize) -> GbxResult<()> {
        self.need(n)?;
        self.pos += n;
        Ok(())
    }

    pub fn u8(&mut self) -> GbxResult<u8> {
        Ok(self.bytes(1)?[0])
    }
    pub fn u16(&mut self) -> GbxResult<u16> {
        Ok(u16::from_le_bytes(self.bytes(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> GbxResult<u32> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> GbxResult<i32> {
        Ok(self.u32()? as i32)
    }
    pub fn u64(&mut self) -> GbxResult<u64> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> GbxResult<f32> {
        Ok(f32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }
    pub fn bool(&mut self) -> GbxResult<bool> {
        Ok(self.u32()? != 0)
    }
    pub fn vec2(&mut self) -> GbxResult<[f32; 2]> {
        Ok([self.f32()?, self.f32()?])
    }
    pub fn vec3(&mut self) -> GbxResult<[f32; 3]> {
        Ok([self.f32()?, self.f32()?, self.f32()?])
    }
    pub fn peek_u32(&self) -> GbxResult<u32> {
        self.need(4)?;
        Ok(u32::from_le_bytes(
            self.data[self.pos..self.pos + 4].try_into().unwrap(),
        ))
    }

    /// u32 length-prefixed string.
    pub fn string(&mut self) -> GbxResult<String> {
        let n = self.u32()? as usize;
        if n > 16 * 1024 * 1024 {
            return Err(GbxError::Format(format!(
                "absurd string length {n} at {}",
                self.pos
            )));
        }
        Ok(String::from_utf8_lossy(self.bytes(n)?).into_owned())
    }

    /// Lookback string ("Id").
    pub fn id(&mut self, st: &mut IdState) -> GbxResult<String> {
        if st.version.is_none() {
            let v = self.u32()?;
            st.version = Some(v);
        }
        let index = self.u32()?;
        if index == 0xFFFF_FFFF {
            return Ok(String::new());
        }
        let flags = index & 0xC000_0000;
        let low = index & 0x3FFF_FFFF;
        if flags == 0 {
            // numeric collection id
            return Ok(match low {
                26 => "Stadium".into(),
                n => format!("collection#{n}"),
            });
        }
        if low == 0 {
            let s = self.string()?;
            st.strings.push(s.clone());
            return Ok(s);
        }
        let i = low as usize - 1;
        st.strings.get(i).cloned().ok_or_else(|| {
            GbxError::Format(format!(
                "lookback index {i} out of range ({} strings) at {}",
                st.strings.len(),
                self.pos
            ))
        })
    }

    /// Ident = (id, collection, author) as three lookback strings.
    pub fn ident(&mut self, st: &mut IdState) -> GbxResult<(String, String, String)> {
        Ok((self.id(st)?, self.id(st)?, self.id(st)?))
    }

    /// File reference ("PackDesc").
    pub fn pack_desc(&mut self) -> GbxResult<String> {
        let version = self.u8()?;
        if version >= 3 {
            self.skip(32)?;
        }
        let path = self.string()?;
        if (!path.is_empty() && version >= 1) || version >= 3 {
            let _url = self.string()?;
        }
        Ok(path)
    }
}
