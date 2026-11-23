//! Plain samples in memory, and WAV files in and out.

use crate::error::{bad, Error, Result};

/// Samples as integers of `bits` bits, one buffer per channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub bits: u8,
    pub ch: Vec<Vec<i32>>,
}

impl Pcm {
    /// samples per channel
    pub fn len(&self) -> usize {
        self.ch.first().map_or(0, |c| c.len())
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn seconds(&self) -> f64 {
        self.len() as f64 / self.rate as f64
    }

    /// The channels averaged into one.
    pub fn mono(&self) -> Vec<i32> {
        if self.ch.len() == 1 {
            return self.ch[0].clone();
        }
        let n = self.ch.len() as i64;
        (0..self.len()).map(|i| (self.ch.iter().map(|c| c[i] as i64).sum::<i64>() / n) as i32).collect()
    }
}

/// A WAV file -> samples. Integer PCM of 8, 16, 24 or 32 bits and 32-bit float, also in the extensible form. 8-bit
/// samples are widened to 16 bits, 32-bit integers are cut to 24, floats are rounded to 16 bits.
pub fn read_wav(b: &[u8]) -> Result<Pcm> {
    if b.len() < 12 || &b[..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err(bad("not a WAV file"));
    }
    let mut o = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    while o + 8 <= b.len() {
        let id = &b[o..o + 4];
        let size = u32::from_le_bytes(b[o + 4..o + 8].try_into().unwrap()) as usize;
        let body = &b[o + 8..(o + 8).saturating_add(size).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            let mut tag = u16::from_le_bytes([body[0], body[1]]);
            if tag == 0xFFFE && body.len() >= 26 {
                tag = u16::from_le_bytes([body[24], body[25]]);
            }
            fmt = Some((tag, u16::from_le_bytes([body[2], body[3]]), u32::from_le_bytes(body[4..8].try_into().unwrap()), u16::from_le_bytes([body[14], body[15]])));
        } else if id == b"data" {
            let (tag, channels, rate, width) = fmt.ok_or_else(|| bad("WAV: data before fmt"))?;
            if channels == 0 || rate == 0 {
                return Err(bad("WAV: no channels or no sample rate"));
            }
            let bytes = (width / 8) as usize;
            let get: fn(&[u8]) -> i32 = match (tag, width) {
                (1, 8) => |s| (s[0] as i32 - 128) << 8,
                (1, 16) => |s| i16::from_le_bytes([s[0], s[1]]) as i32,
                (1, 24) => |s| s[0] as i32 | (s[1] as i32) << 8 | (s[2] as i8 as i32) << 16,
                (1, 32) => |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]) >> 8,
                (3, 32) => |s| (f32::from_le_bytes([s[0], s[1], s[2], s[3]]) * 32768.0).round().clamp(-32768.0, 32767.0) as i32,
                _ => return Err(Error::Unsupported(format!("WAV format {tag} with {width} bits"))),
            };
            let bits = match (tag, width) {
                (1, 24) | (1, 32) => 24,
                _ => 16,
            };
            let mut ch = vec![Vec::with_capacity(body.len() / bytes / channels as usize); channels as usize];
            for frame in body.chunks_exact(bytes * channels as usize) {
                for (c, s) in ch.iter_mut().zip(frame.chunks_exact(bytes)) {
                    c.push(get(s));
                }
            }
            return Ok(Pcm { rate, bits, ch });
        }
        o += 8 + size + (size & 1);
    }
    Err(bad("WAV: no data chunk"))
}

/// The 44 bytes before a WAV file's samples. `samples`: per channel; None for a stream of unknown length.
pub fn wav_header(rate: u32, channels: usize, bits: u8, samples: Option<u64>) -> Vec<u8> {
    let width = if bits <= 16 { 2u32 } else { 3 };
    let data = samples.map_or(u32::MAX as u64, |n| n * width as u64 * channels as u64).min(u32::MAX as u64 - 36) as u32;
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&(data + 36).to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&(channels as u16).to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * width * channels as u32).to_le_bytes());
    h.extend_from_slice(&((width * channels as u32) as u16).to_le_bytes());
    h.extend_from_slice(&((width * 8) as u16).to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&data.to_le_bytes());
    h
}

/// Samples of all channels as a WAV file's bytes: 16-bit up to 16 bits per sample, 24-bit above.
pub fn sample_bytes(ch: &[Vec<i32>], bits: u8) -> Vec<u8> {
    let n = ch.first().map_or(0, |c| c.len());
    let width = if bits <= 16 { 2 } else { 3 };
    let mut out = Vec::with_capacity(n * width * ch.len());
    for i in 0..n {
        for c in ch {
            out.extend_from_slice(&c[i].to_le_bytes()[..width]);
        }
    }
    out
}

pub fn write_wav(p: &Pcm) -> Vec<u8> {
    let mut out = wav_header(p.rate, p.ch.len(), p.bits, Some(p.len() as u64));
    out.extend_from_slice(&sample_bytes(&p.ch, p.bits));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_comes_back() {
        for bits in [16u8, 24] {
            let top = 1i32 << (bits - 1);
            let p = Pcm { rate: 22_050, bits, ch: vec![vec![0, 1, -1, top - 1, -top, 1234], vec![5, -5, 77, -77, 0, top / 3]] };
            assert_eq!(read_wav(&write_wav(&p)).unwrap(), p);
        }
        assert!(read_wav(b"RIFF\0\0\0\0WAVE").is_err());
    }
}
