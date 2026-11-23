//! The few primitives every unit payload is made of: variable-length integers (7 bits per byte, low bits first, the
//! top bit says "more"), strings and byte strings behind their length, and the CRC-32 of zip and PNG.

use crate::error::{bad, Result};

pub fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

pub fn put_bytes(out: &mut Vec<u8>, b: &[u8]) {
    put_varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

pub fn put_str(out: &mut Vec<u8>, s: &str) {
    put_bytes(out, s.as_bytes());
}

/// Reads a payload from front to back; running out of bytes is a format error.
pub struct Cursor<'a> {
    b: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(b: &'a [u8]) -> Self {
        Cursor { b, pos: 0 }
    }

    pub fn u8(&mut self) -> Result<u8> {
        let v = *self.b.get(self.pos).ok_or_else(|| bad("a unit ends early"))?;
        self.pos += 1;
        Ok(v)
    }

    pub fn varint(&mut self) -> Result<u64> {
        let mut v = 0u64;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            v |= ((b & 0x7F) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(bad("a number of more than 64 bits"))
    }

    /// A count of things that each take at least one byte: more than the bytes left is damage, not a reason to
    /// reserve memory.
    pub fn count(&mut self) -> Result<usize> {
        let n = self.varint()?;
        if n > (self.b.len() - self.pos) as u64 {
            return Err(bad("a count larger than its unit"));
        }
        Ok(n as usize)
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.b.get(self.pos..self.pos.checked_add(n).ok_or_else(|| bad("a unit ends early"))?).ok_or_else(|| bad("a unit ends early"))?;
        self.pos += n;
        Ok(s)
    }

    pub fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = self.count()?;
        self.take(n)
    }

    pub fn str(&mut self) -> Result<String> {
        String::from_utf8(self.bytes()?.to_vec()).map_err(|_| bad("text that is not UTF-8"))
    }

    pub fn rest(&mut self) -> &'a [u8] {
        let s = &self.b[self.pos..];
        self.pos = self.b.len();
        s
    }
}

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC: [u32; 256] = crc_table();

pub fn crc32(b: &[u8]) -> u32 {
    !b.iter().fold(!0u32, |c, &v| CRC[((c ^ v as u32) & 0xFF) as usize] ^ (c >> 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_of_the_standard_check_string() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn numbers_and_text_come_back() {
        let mut b = Vec::new();
        for v in [0u64, 1, 127, 128, 300, 1 << 35, u64::MAX] {
            put_varint(&mut b, v);
        }
        put_str(&mut b, "søk — 声");
        let mut c = Cursor::new(&b);
        for v in [0u64, 1, 127, 128, 300, 1 << 35, u64::MAX] {
            assert_eq!(c.varint().unwrap(), v);
        }
        assert_eq!(c.str().unwrap(), "søk — 声");
        assert!(c.u8().is_err());
    }
}
