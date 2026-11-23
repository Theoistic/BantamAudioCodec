//! A FLAC reader after RFC 9639, so that existing recordings can be taken in without another tool: any native FLAC
//! stream (4 to 32 bits per sample, 1 to 8 channels) to its exact samples. Checks the CRC-8 of every frame header
//! and the CRC-16 of every frame. Not read: FLAC inside Ogg, and STREAMINFO's MD5.

use crate::error::{Error, Result};
use crate::pcm::Pcm;

fn bad(m: &str) -> Error {
    Error::Format(format!("flac: {m}"))
}

struct Info {
    rate: u32,
    channels: usize,
    bits: u32,
    /// samples per channel; 0 = not known
    total: u64,
}

pub fn read_flac(b: &[u8]) -> Result<Pcm> {
    let (info, mut pos) = header(b)?;
    let mut ch = vec![Vec::new(); info.channels];
    let mut block = vec![Vec::new(); info.channels];
    let mut done = 0u64;
    while pos < b.len() && (info.total == 0 || done < info.total) {
        let (n, next) = frame(b, pos, &info, &mut block)?;
        let take = if info.total == 0 { n } else { (info.total - done).min(n as u64) as usize };
        for (o, c) in ch.iter_mut().zip(&block) {
            o.extend(c[..take].iter().map(|&v| v as i32));
        }
        done += take as u64;
        pos = next;
    }
    if done < info.total {
        return Err(bad(&format!("the stream ends after {done} of {} samples", info.total)));
    }
    Ok(Pcm { rate: info.rate, bits: info.bits as u8, ch })
}

/// Skips ID3v2 tags, checks the marker, takes STREAMINFO and skips the other metadata blocks.
fn header(b: &[u8]) -> Result<(Info, usize)> {
    let mut o = 0usize;
    while let Some(h) = b.get(o..o + 10).filter(|h| &h[..3] == b"ID3") {
        let size = h[6..].iter().fold(0usize, |a, &v| a << 7 | (v & 0x7f) as usize);
        o += 10 + size + if h[5] & 0x10 != 0 { 10 } else { 0 };
    }
    if b.get(o..o + 4) != Some(b"fLaC".as_slice()) {
        return Err(bad("not a FLAC file"));
    }
    o += 4;
    let mut info = None;
    loop {
        let h = b.get(o..o + 4).ok_or_else(|| bad("the metadata ends early"))?;
        let (last, kind) = (h[0] & 0x80 != 0, h[0] & 0x7f);
        let len = (h[1] as usize) << 16 | (h[2] as usize) << 8 | h[3] as usize;
        let body = b.get(o + 4..o + 4 + len).ok_or_else(|| bad("a metadata block runs past the end"))?;
        o += 4 + len;
        if kind == 0 && info.is_none() {
            if len < 34 {
                return Err(bad("STREAMINFO is too short"));
            }
            // 20 bits sample rate, 3 bits channels - 1, 5 bits bits per sample - 1, 36 bits number of samples
            let v = body[10..18].iter().fold(0u64, |a, &x| a << 8 | x as u64);
            info = Some(Info { rate: (v >> 44) as u32, channels: (v >> 41 & 7) as usize + 1, bits: (v >> 36 & 31) as u32 + 1, total: v & ((1 << 36) - 1) });
        } else if info.is_none() {
            return Err(bad("the first metadata block is not STREAMINFO"));
        }
        if last {
            break;
        }
    }
    let info = info.ok_or_else(|| bad("no STREAMINFO"))?;
    if info.bits < 4 || info.rate == 0 {
        return Err(bad("no sample rate, or fewer than 4 bits per sample"));
    }
    Ok((info, o))
}

const SAMPLE_RATES: [u32; 12] = [0, 88_200, 176_400, 192_000, 8_000, 16_000, 22_050, 24_000, 32_000, 44_100, 48_000, 96_000];
const SAMPLE_BITS: [u32; 8] = [0, 8, 12, 0, 16, 20, 24, 32];

/// Decodes the frame at `b[pos..]` into `ch`. Returns its block size and the offset of the next frame.
fn frame(b: &[u8], pos: usize, info: &Info, ch: &mut [Vec<i64>]) -> Result<(usize, usize)> {
    let short = || bad("a frame runs past the end");
    let f = &b[pos..];
    let h = f.get(..5).ok_or_else(short)?;
    if h[0] != 0xff || h[1] & 0xfe != 0xf8 {
        return Err(bad("lost frame sync"));
    }
    let variable = h[1] & 1 != 0;
    let (block_code, rate_code) = (h[2] >> 4, h[2] & 15);
    let (assignment, bits_code) = (h[3] >> 4, (h[3] >> 1 & 7) as usize);
    if h[3] & 1 != 0 || assignment > 10 || bits_code == 3 || block_code == 0 || rate_code == 15 {
        return Err(bad("reserved value in a frame header"));
    }
    // the frame or sample number, coded like UTF-8; only its length matters here
    let ones = h[4].leading_ones() as usize;
    let more = match ones {
        0 => 0,
        2..=6 => ones - 1,
        7 if variable => 6,
        _ => return Err(bad("invalid frame number")),
    };
    let mut o = 5 + more;
    let mut number = |bytes: usize| -> Result<usize> {
        let v = f.get(o..o + bytes).ok_or_else(short)?.iter().fold(0usize, |a, &x| a << 8 | x as usize);
        o += bytes;
        Ok(v)
    };
    let block = match block_code {
        1 => 192,
        2..=5 => 576 << (block_code - 2),
        6 => number(1)? + 1,
        7 => number(2)? + 1,
        _ => 256 << (block_code - 8),
    };
    let rate = match rate_code {
        0 => info.rate,
        12 => number(1)? as u32 * 1000,
        13 => number(2)? as u32,
        14 => number(2)? as u32 * 10,
        _ => SAMPLE_RATES[rate_code as usize],
    };
    let bits = if bits_code == 0 { info.bits } else { SAMPLE_BITS[bits_code] };
    let channels = if assignment < 8 { assignment as usize + 1 } else { 2 };
    if f.get(o).copied().ok_or_else(short)? != crc8(&f[..o]) {
        return Err(bad("frame header checksum mismatch"));
    }
    if (rate, bits, channels) != (info.rate, info.bits, info.channels) {
        return Err(bad("a frame that differs from the stream in rate, bits or channels"));
    }
    let head = o + 1;
    let mut r = BitReader::new(&f[head..]);
    for (i, c) in ch.iter_mut().enumerate() {
        c.clear();
        c.resize(block, 0);
        // a side channel (left - right) needs one more bit
        let side = matches!((assignment, i), (8, 1) | (9, 0) | (10, 1));
        subframe(&mut r, c, info.bits + side as u32)?;
    }
    if r.bits(r.n % 8)? != 0 {
        return Err(bad("nonzero padding at the end of a frame"));
    }
    let end = head + r.consumed_bytes();
    let crc = f.get(end..end + 2).ok_or_else(short)?;
    if crc16(&f[..end]) != u16::from_be_bytes([crc[0], crc[1]]) {
        return Err(bad("frame checksum mismatch"));
    }
    if let (8..=10, [c0, c1]) = (assignment, &mut *ch) {
        for (a, s) in c0.iter_mut().zip(c1.iter_mut()) {
            match assignment {
                8 => *s = a.wrapping_sub(*s),
                9 => *a = a.wrapping_add(*s),
                _ => {
                    let sum = a.wrapping_shl(1) | (*s & 1);
                    (*a, *s) = (sum.wrapping_add(*s) >> 1, sum.wrapping_sub(*s) >> 1);
                }
            }
        }
    }
    let half = 1i64 << (info.bits - 1);
    if ch.iter().any(|c| c.iter().any(|&v| v < -half || v >= half)) {
        return Err(bad("a sample does not fit the bits per sample"));
    }
    Ok((block, pos + end + 2))
}

const FIXED: [&[i64]; 5] = [&[], &[1], &[2, -1], &[3, -3, 1], &[4, -6, 4, -1]];

fn subframe(r: &mut BitReader, out: &mut [i64], bps: u32) -> Result<()> {
    let head = r.bits(8)?;
    if head & 0x80 != 0 {
        return Err(bad("invalid subframe header"));
    }
    let wasted = if head & 1 != 0 { r.unary()? + 1 } else { 0 };
    if wasted >= bps as u64 {
        return Err(bad("more wasted bits than bits per sample"));
    }
    let bps = bps - wasted as u32;
    match head >> 1 {
        0 => out.fill(r.signed(bps)?),
        1 => {
            for v in out.iter_mut() {
                *v = r.signed(bps)?;
            }
        }
        kind @ 8..=12 => {
            let order = kind as usize - 8;
            warm_up(r, out, order, bps)?;
            residual(r, out, order)?;
            predict(out, FIXED[order], 0);
        }
        kind @ 32..=63 => {
            let order = kind as usize - 31;
            warm_up(r, out, order, bps)?;
            let precision = r.bits(4)? as u32 + 1;
            let shift = r.signed(5)?;
            if precision == 16 || shift < 0 {
                return Err(bad("invalid LPC parameters"));
            }
            let mut coefs = [0i64; 32];
            for c in coefs[..order].iter_mut() {
                *c = r.signed(precision)?;
            }
            residual(r, out, order)?;
            predict(out, &coefs[..order], shift as u32);
        }
        _ => return Err(bad("reserved subframe type")),
    }
    if wasted > 0 {
        out.iter_mut().for_each(|v| *v = v.wrapping_shl(wasted as u32));
    }
    Ok(())
}

fn warm_up(r: &mut BitReader, out: &mut [i64], order: usize, bps: u32) -> Result<()> {
    for v in out.get_mut(..order).ok_or_else(|| bad("a predictor order above the block size"))? {
        *v = r.signed(bps)?;
    }
    Ok(())
}

/// The residual into out[order..]: 2^p partitions, each with its Rice parameter or an escape to plain values.
fn residual(r: &mut BitReader, out: &mut [i64], order: usize) -> Result<()> {
    let width = match r.bits(2)? {
        0 => 4,
        1 => 5,
        _ => return Err(bad("reserved residual coding method")),
    };
    let escape = (1u64 << width) - 1;
    let p = r.bits(4)? as u32;
    let per = out.len() >> p;
    if per << p != out.len() || per < order {
        return Err(bad("invalid residual partition order"));
    }
    let mut at = order;
    for i in 0..1usize << p {
        let len = if i == 0 { per - order } else { per };
        let part = &mut out[at..at + len];
        at += len;
        let k = r.bits(width)?;
        if k < escape {
            for v in part.iter_mut() {
                let u = r.unary()? << k | r.bits(k as u32)?;
                *v = (u >> 1) as i64 ^ -((u & 1) as i64);
            }
        } else {
            let bits = r.bits(5)? as u32;
            for v in part.iter_mut() {
                *v = if bits == 0 { 0 } else { r.signed(bits)? };
            }
        }
    }
    Ok(())
}

/// x holds warm-up samples and then the residual; each later sample becomes residual + prediction.
fn predict(x: &mut [i64], coefs: &[i64], shift: u32) {
    let order = coefs.len();
    for i in order..x.len() {
        let sum = coefs.iter().zip(x[..i].iter().rev()).fold(0i64, |s, (&c, &v)| s.wrapping_add(c.wrapping_mul(v)));
        x[i] = x[i].wrapping_add(sum >> shift);
    }
}

/// Reads bits, the most significant bit of a byte first.
struct BitReader<'a> {
    b: &'a [u8],
    pos: usize,
    /// loaded bits in the top `n` bits
    acc: u64,
    n: u32,
}

impl<'a> BitReader<'a> {
    fn new(b: &'a [u8]) -> Self {
        BitReader { b, pos: 0, acc: 0, n: 0 }
    }

    /// Loads whole bytes until at least 56 bits are there. Never all 64: `unary` shifts a found one out together with
    /// the zeros before it, and a shift by 64 is not a shift.
    fn refill(&mut self) {
        while self.n < 56 && self.pos < self.b.len() {
            self.acc |= (self.b[self.pos] as u64) << (56 - self.n);
            self.pos += 1;
            self.n += 8;
        }
    }

    /// The next `k` bits, k <= 56.
    fn bits(&mut self, k: u32) -> Result<u64> {
        if self.n < k {
            self.refill();
            if self.n < k {
                return Err(bad("unexpected end of data"));
            }
        }
        let v = self.acc >> 1 >> (63 - k);
        self.acc = if k == 0 { self.acc } else { self.acc << k };
        self.n -= k;
        Ok(v)
    }

    fn signed(&mut self, k: u32) -> Result<i64> {
        Ok((self.bits(k)? << (64 - k)) as i64 >> (64 - k))
    }

    /// The count of zero bits before the next one bit, which is consumed too.
    fn unary(&mut self) -> Result<u64> {
        let mut zeros = 0u64;
        loop {
            let lz = self.acc.leading_zeros();
            if lz < self.n {
                self.acc <<= lz + 1;
                self.n -= lz + 1;
                return Ok(zeros + lz as u64);
            }
            zeros += self.n as u64;
            (self.acc, self.n) = (0, 0);
            self.refill();
            if self.n == 0 {
                return Err(bad("unexpected end of data"));
            }
        }
    }

    /// Bytes read so far, a started byte counted whole.
    fn consumed_bytes(&self) -> usize {
        self.pos - (self.n / 8) as usize
    }
}

fn crc8(b: &[u8]) -> u8 {
    b.iter().fold(0u8, |mut c, &v| {
        c ^= v;
        for _ in 0..8 {
            c = if c & 0x80 != 0 { c << 1 ^ 0x07 } else { c << 1 };
        }
        c
    })
}

const fn crc16_table() -> [u16; 256] {
    let mut t = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u16) << 8;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000 != 0 { c << 1 ^ 0x8005 } else { c << 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

static CRC16: [u16; 256] = crc16_table();

fn crc16(b: &[u8]) -> u16 {
    b.iter().fold(0u16, |c, &v| c << 8 ^ CRC16[((c >> 8) as u8 ^ v) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_run_of_zeros_is_counted() {
        // 63 zero bits and a one, as a large residual's Rice code starts: once this filled the reader's 64 bits exactly
        let mut r = BitReader::new(&[0, 0, 0, 0, 0, 0, 0, 1, 0xA5, 0xFF]);
        assert_eq!(r.unary().unwrap(), 63);
        assert_eq!(r.bits(8).unwrap(), 0xA5);
        // and at every other offset
        for lead in 0..8u32 {
            let mut bytes = vec![0u8; 12];
            bytes[0] = (0xFFu16 << (8 - lead)) as u8;
            let one = lead as usize + 63;
            bytes[one / 8] |= 0x80 >> (one % 8);
            let mut r = BitReader::new(&bytes);
            assert_eq!(r.bits(lead).unwrap(), (1 << lead) - 1);
            assert_eq!(r.unary().unwrap(), 63, "after {lead} bits");
        }
        assert!(BitReader::new(&[0, 0]).unary().is_err());
    }
}
