//! Small binary reader/writer for the project's Redis payload formats.
//!
//! The formats are defined in `docs/WIRE-FORMATS.md`. This module deliberately
//! does not depend on a serialization framework so the bytes are auditable
//! against the document.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CodecError {
    #[error("unexpected end of input")]
    Eof,
    #[error("length prefix exceeds remaining input")]
    Length,
    #[error("trailing bytes after message")]
    Trailing,
    #[error("value out of range: {0}")]
    Range(&'static str),
    #[error("unknown discriminant: {0}")]
    Discriminant(u8),
}

pub type Result<T> = std::result::Result<T, CodecError>;

/// Append-only writer. All integers are little-endian unless stated otherwise.
#[derive(Default, Debug, Clone)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            buf: Vec::with_capacity(n),
        }
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    pub fn u16(&mut self, v: u16) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u32(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn u128(&mut self, v: u128) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn i64(&mut self, v: i64) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(b);
        self
    }

    /// `B0_nn`: one-byte length prefix (max 255).
    pub fn blob_u8(&mut self, b: &[u8]) -> Result<&mut Self> {
        let len = u8::try_from(b.len()).map_err(|_| CodecError::Range("blob_u8"))?;
        self.buf.push(len);
        self.buf.extend_from_slice(b);
        Ok(self)
    }

    /// `B0_64K`: two-byte length prefix (max 65535).
    pub fn blob_u16(&mut self, b: &[u8]) -> Result<&mut Self> {
        let len = u16::try_from(b.len()).map_err(|_| CodecError::Range("blob_u16"))?;
        self.u16(len);
        self.buf.extend_from_slice(b);
        Ok(self)
    }

    /// `B4`: four-byte length prefix (project extension for share blobs).
    pub fn blob_u32(&mut self, b: &[u8]) -> Result<&mut Self> {
        let len = u32::try_from(b.len()).map_err(|_| CodecError::Range("blob_u32"))?;
        self.u32(len);
        self.buf.extend_from_slice(b);
        Ok(self)
    }

    pub fn finish(self) -> Vec<u8> {
        self.buf
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
}

/// Bounds-checked reader.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(CodecError::Length)?;
        if end > self.buf.len() {
            return Err(CodecError::Eof);
        }
        let out = &self.buf[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(
            b.try_into().map_err(|_| CodecError::Length)?,
        ))
    }

    pub fn u128(&mut self) -> Result<u128> {
        let b = self.take(16)?;
        Ok(u128::from_le_bytes(
            b.try_into().map_err(|_| CodecError::Length)?,
        ))
    }

    pub fn i64(&mut self) -> Result<i64> {
        let b = self.take(8)?;
        Ok(i64::from_le_bytes(
            b.try_into().map_err(|_| CodecError::Length)?,
        ))
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        self.take(n)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let b = self.take(N)?;
        b.try_into().map_err(|_| CodecError::Length)
    }

    pub fn blob_u8(&mut self) -> Result<&'a [u8]> {
        let n = self.u8()? as usize;
        self.take(n)
    }

    pub fn blob_u16(&mut self) -> Result<&'a [u8]> {
        let n = self.u16()? as usize;
        self.take(n)
    }

    pub fn blob_u32(&mut self) -> Result<&'a [u8]> {
        let n = self.u32()? as usize;
        self.take(n)
    }

    pub fn finish(self) -> Result<()> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(CodecError::Trailing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_integers() {
        let mut w = Writer::new();
        w.u8(1)
            .u16(2)
            .u32(3)
            .u64(4)
            .u128(5)
            .blob_u8(b"abc")
            .unwrap();
        let bytes = w.finish();
        let mut r = Reader::new(&bytes);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.u16().unwrap(), 2);
        assert_eq!(r.u32().unwrap(), 3);
        assert_eq!(r.u64().unwrap(), 4);
        assert_eq!(r.u128().unwrap(), 5);
        assert_eq!(r.blob_u8().unwrap(), b"abc");
        r.finish().unwrap();
    }

    #[test]
    fn short_input_fails() {
        let mut r = Reader::new(&[1, 2]);
        assert_eq!(r.u32(), Err(CodecError::Eof));
    }

    #[test]
    fn trailing_bytes_fail() {
        let mut r = Reader::new(&[1, 2]);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.finish(), Err(CodecError::Trailing));
    }
}
